// src-tauri/src/mimir_tags.rs
// Tag management commands for Mimir resources.

use serde_json::json;
use sqlx::Row;
use tauri::State;
use crate::constants::GROQ_API_URL;
use crate::database::Database;

#[tauri::command]
pub async fn get_distinct_tags(
    database: State<'_, Database>,
) -> Result<Vec<String>, String> {
    let rows = sqlx::query(
        "SELECT DISTINCT unnest(tags) AS tag FROM mimir_resources ORDER BY tag"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let tags: Vec<String> = rows
        .iter()
        .map(|r| r.try_get::<String, _>("tag").unwrap_or_default())
        .collect();
    Ok(tags)
}

#[tauri::command]
pub async fn update_resource_tags(
    resource_id: String,
    tags: Vec<String>,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE mimir_resources SET tags = $1 WHERE id = $2"
    )
    .bind(&tags)
    .bind(&resource_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn auto_tag_existing_resources(
    client: tauri::State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<String, String> {
    let model = "llama-3.3-70b-versatile";
    let base_url = GROQ_API_URL;
    let api_key = crate::mimir::groq_api_key()?;

    let rows = sqlx::query(
        "SELECT id, title FROM mimir_resources WHERE tags = '{}' OR tags IS NULL"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = rows.len();
    println!("🏷️  Auto-tagging {} untagged resources with model {}", total, model);

    let mut tagged = 0u32;

    for row in &rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let title: String = row.try_get("title").map_err(|e| e.to_string())?;

        let user_prompt = format!(
            "Given this resource title: '{}', suggest 2-4 tags from these categories:\n\
             Technology: rust, typescript, python, react, sql, julia, node, postgres, tauri, d3, numpy\n\
             Type: tutorial, docs, video, article, reference\n\
             Level: beginner, intermediate, advanced\n\
             Return ONLY a JSON array of strings, e.g. [\"rust\", \"docs\"]",
            title
        );

        let autotag_t0 = std::time::Instant::now();
        let response = client
            .post(base_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": "You are a tag classifier. Return ONLY a JSON array of tag strings." },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.2,
                "max_tokens": 100
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;

        let resp = match response {
            Ok(r) => r,
            Err(e) => {
                crate::brain::log_prompt_call(
                    database.pool.clone(), "auto_tag", model, "auto_tag_v1",
                    autotag_t0.elapsed().as_millis() as i64, false, Some(e.to_string()), None,
                );
                println!("  ⚠️  Failed to tag '{}': {}", title, e);
                continue;
            }
        };

        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                println!("  ⚠️  Failed to parse response for '{}': {}", title, e);
                continue;
            }
        };

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("[]");

        let clean = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let tags: Vec<String> = match serde_json::from_str(clean) {
            Ok(t) => t,
            Err(_) => {
                println!("  ⚠️  Bad JSON for '{}': {}", title, clean);
                continue;
            }
        };

        if let Err(e) = sqlx::query("UPDATE mimir_resources SET tags = $1 WHERE id = $2")
            .bind(&tags)
            .bind(&id)
            .execute(&database.pool)
            .await
        {
            println!("  ⚠️  Failed to save tags for '{}': {}", title, e);
            continue;
        }

        crate::brain::log_prompt_call(
            database.pool.clone(), "auto_tag", model, "auto_tag_v1",
            autotag_t0.elapsed().as_millis() as i64, true, None, None,
        );
        tagged += 1;
        println!("  ✅ [{}] {} → {:?}", tagged, title, tags);
    }

    let summary = format!("Tagged {} of {} resources", tagged, total);
    println!("🏷️  {}", summary);
    Ok(summary)
}
