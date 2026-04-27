// src-tauri/src/mimir_manage.rs
// Resource management, playlist, link discovery, and completion commands for Mimir.

use serde_json::json;
use sqlx::Row;
use tauri::State;
use crate::constants::SCRAPER_URL;
use crate::database::Database;
use crate::mimir::{
    MimirResource, DiscoveredLink, PlaylistInfo, LinkedNodeTitle,
};

#[tauri::command]
pub async fn get_mimir_resources(
    database: State<'_, Database>,
) -> Result<Vec<MimirResource>, String> {
    let rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.user_notes, \
                mr.created_at::text, mr.parent_id, mr.tags, mr.is_completed, \
                COALESCE(nc.node_count, 0)::int AS node_count \
         FROM mimir_resources mr \
         LEFT JOIN ( \
           SELECT resource_id, COUNT(DISTINCT node_id)::int AS node_count \
           FROM mimir_node_links \
           GROUP BY resource_id \
         ) nc ON nc.resource_id = mr.id \
         ORDER BY mr.created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut resources = Vec::new();
    for row in &rows {
        resources.push(MimirResource {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            url: row.try_get("url").ok(),
            resource_type: row.try_get("type").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            user_notes: row.try_get("user_notes").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
            parent_id: row.try_get("parent_id").ok(),
            tags: row.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
            is_completed: row.try_get("is_completed").unwrap_or(false),
            node_count: row.try_get("node_count").unwrap_or(0),
            relevance_score: None,
            matched_section_title: None,
            matched_page_start: None,
            matched_page_end: None,
        });
    }

    Ok(resources)
}

#[tauri::command]
pub async fn get_node_resources(
    node_id: String,
    database: State<'_, Database>,
) -> Result<Vec<MimirResource>, String> {
    let rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.user_notes, \
                mr.created_at::text, mr.parent_id, mr.tags, mr.is_completed, \
                mnl.relevance_score, mnl.matched_section_title, \
                mnl.matched_page_start, mnl.matched_page_end \
         FROM mimir_resources mr \
         JOIN mimir_node_links mnl ON mnl.resource_id = mr.id \
         WHERE mnl.node_id = $1 \
         ORDER BY mnl.relevance_score ASC NULLS LAST"
    )
    .bind(&node_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut resources = Vec::new();
    for row in &rows {
        resources.push(MimirResource {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            url: row.try_get("url").ok(),
            resource_type: row.try_get("type").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            user_notes: row.try_get("user_notes").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
            parent_id: row.try_get("parent_id").ok(),
            tags: row.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
            is_completed: row.try_get("is_completed").unwrap_or(false),
            node_count: 0,
            relevance_score: row.try_get("relevance_score").ok(),
            matched_section_title: row.try_get("matched_section_title").ok().flatten(),
            matched_page_start: row.try_get("matched_page_start").ok().flatten(),
            matched_page_end: row.try_get("matched_page_end").ok().flatten(),
        });
    }

    Ok(resources)
}

#[tauri::command]
pub async fn delete_mimir_resource(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    // CASCADE handles chunks + embeddings + node_links
    let result = sqlx::query("DELETE FROM mimir_resources WHERE id = $1")
        .bind(&resource_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err("Resource not found".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn link_resource_to_node(
    resource_id: String,
    node_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    let link_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mimir_node_links (id, resource_id, node_id) VALUES ($1, $2, $3) \
         ON CONFLICT (resource_id, node_id) DO NOTHING",
    )
    .bind(&link_id)
    .bind(&resource_id)
    .bind(&node_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_linked_node_titles(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<Vec<LinkedNodeTitle>, String> {
    let rows = sqlx::query(
        "SELECT tn.title FROM mimir_node_links mnl JOIN tree_nodes tn ON tn.id = mnl.node_id WHERE mnl.resource_id = $1 ORDER BY tn.title"
    )
    .bind(&resource_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let titles: Vec<LinkedNodeTitle> = rows
        .iter()
        .map(|r| LinkedNodeTitle {
            title: r.try_get::<String, _>("title").unwrap_or_default(),
        })
        .collect();
    Ok(titles)
}

#[tauri::command]
pub async fn toggle_resource_completion(
    resource_id: String,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<bool, String> {
    let row = sqlx::query(
        "UPDATE mimir_resources SET is_completed = NOT is_completed WHERE id = $1 RETURNING is_completed"
    )
    .bind(&resource_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let new_val: bool = row.try_get("is_completed").map_err(|e| e.to_string())?;

    // Only cascade on mark-complete, not on un-complete
    if new_val {
        crate::orchestrator::on_resource_completed(&database.pool, &app, &resource_id).await;
    }

    Ok(new_val)
}

#[tauri::command]
pub async fn on_resource_completed(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    let linked = sqlx::query(
        "SELECT DISTINCT mnl.node_id, tn.tree_id
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         WHERE mnl.resource_id = $1"
    )
    .bind(&resource_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    if linked.is_empty() {
        return Ok(());
    }

    let mut affected_trees: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in &linked {
        let node_id: String = row.try_get("node_id").map_err(|e| e.to_string())?;
        let tree_id: String = row.try_get("tree_id").map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE tree_nodes SET progress = LEAST(COALESCE(progress, 0) + 20, 100) WHERE id = $1"
        )
        .bind(&node_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        affected_trees.insert(tree_id);
    }

    for tree_id in &affected_trees {
        let rows = sqlx::query(
            "SELECT id, parent_id, progress FROM tree_nodes WHERE tree_id = $1"
        )
        .bind(tree_id)
        .fetch_all(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        let mut children_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        let mut progress_map: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
        let mut roots: Vec<String> = Vec::new();

        for row in &rows {
            let id: String = row.try_get("id").map_err(|e| e.to_string())?;
            let parent_id: Option<String> = row.try_get("parent_id").map_err(|e| e.to_string())?;
            let progress: Option<i32> = row.try_get("progress").map_err(|e| e.to_string())?;

            progress_map.insert(id.clone(), progress.unwrap_or(0));
            match parent_id {
                Some(pid) => children_map.entry(pid).or_default().push(id),
                None => roots.push(id),
            }
        }

        let mut order: Vec<String> = Vec::new();
        let mut stack = roots;
        while let Some(id) = stack.pop() {
            order.push(id.clone());
            if let Some(kids) = children_map.get(&id) {
                stack.extend(kids.iter().cloned());
            }
        }
        order.reverse();

        for id in &order {
            if let Some(kids) = children_map.get(id) {
                if !kids.is_empty() {
                    let avg: i32 = kids.iter()
                        .map(|kid| *progress_map.get(kid).unwrap_or(&0))
                        .sum::<i32>() / kids.len() as i32;
                    progress_map.insert(id.clone(), avg);
                    sqlx::query("UPDATE tree_nodes SET progress = $1 WHERE id = $2")
                        .bind(avg)
                        .bind(id)
                        .execute(&database.pool)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        println!("🔄 Recalculated progress for tree {}", tree_id);
    }

    Ok(())
}

#[tauri::command]
pub async fn get_chunk_counts(
    database: State<'_, Database>,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let rows = sqlx::query(
        "SELECT resource_id, COUNT(*) AS count FROM mimir_chunks GROUP BY resource_id",
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut map = std::collections::HashMap::new();
    for row in rows {
        let resource_id: String = row.try_get("resource_id").map_err(|e| e.to_string())?;
        let count: i64 = row.try_get("count").map_err(|e| e.to_string())?;
        map.insert(resource_id, count);
    }
    Ok(map)
}

#[tauri::command]
pub async fn fetch_playlist(url: String, client: tauri::State<'_, reqwest::Client>) -> Result<PlaylistInfo, String> {
    let response = client
        .post(format!("{}/fetch-playlist", SCRAPER_URL))
        .json(&json!({ "url": url }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| {
            if e.to_string().contains("connect") || e.to_string().contains("refused") {
                "Scraper service not running".to_string()
            } else {
                e.to_string()
            }
        })?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("Playlist fetch failed").to_string());
    }

    let raw = response.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str::<PlaylistInfo>(&raw).map_err(|e| {
        eprintln!("[fetch_playlist] decode error: {e}\nraw body: {raw}");
        format!("Failed to parse playlist response: {e}")
    })
}

#[tauri::command]
pub async fn discover_links(url: String, client: tauri::State<'_, reqwest::Client>) -> Result<Vec<DiscoveredLink>, String> {
    let response = client
        .post(format!("{}/discover", SCRAPER_URL))
        .json(&json!({ "url": url }))
        .timeout(std::time::Duration::from_secs(90))
        .send()
        .await
        .map_err(|e| {
            if e.to_string().contains("connect") || e.to_string().contains("refused") {
                "Scraper service not running".to_string()
            } else {
                e.to_string()
            }
        })?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("Discovery failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let links = result["links"]
        .as_array()
        .ok_or_else(|| "Invalid response format".to_string())?;

    Ok(links
        .iter()
        .map(|l| DiscoveredLink {
            url: l["url"].as_str().unwrap_or("").to_string(),
            text: l["text"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}
