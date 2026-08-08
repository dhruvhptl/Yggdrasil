// src-tauri/src/job_commands.rs
// Job tracker: applications → JD → AI skill extraction → analytics.
// All queries use sqlx::query() non-macro to avoid offline cache issues.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;
use uuid::Uuid;

use crate::constants::GROQ_API_URL;
use crate::database::Database;

// ─── Structs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobApplication {
    pub id: String,
    pub company: String,
    pub position: String,
    pub location: Option<String>,
    pub source: Option<String>,
    pub status: String,
    pub date_applied: Option<String>,
    pub date_follow_up: Option<String>,
    pub job_description: Option<String>,
    pub link: Option<String>,
    pub notes: Option<String>,
    pub rating_overall: Option<i32>,
    pub rating_location: Option<i32>,
    pub rating_alignment: Option<i32>,
    pub rating_salary: Option<i32>,
    pub rating_role: Option<i32>,
    pub season: String,
    pub created_at: String,
    pub follow_up_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSkill {
    pub id: String,
    pub job_id: String,
    pub skill_name: String,
    pub is_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDemand {
    pub skill_name: String,
    pub count: i64,
    pub total_jobs: i64,
    pub frequency: f64,
    pub avg_rating: f64,
    pub demand_score: f64,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn load_api_key() -> Result<String, String> {
    std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY environment variable not set".to_string())
}

fn row_to_job(r: &sqlx::postgres::PgRow) -> Result<JobApplication, String> {
    let date_applied = r
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("date_applied")
        .map_err(|e| e.to_string())?
        .map(|d| d.to_rfc3339());
    let date_follow_up = r
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("date_follow_up")
        .map_err(|e| e.to_string())?
        .map(|d| d.to_rfc3339());
    let created_at = r
        .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
        .map(|d| d.to_rfc3339())
        .map_err(|e| e.to_string())?;

    Ok(JobApplication {
        id: r.try_get("id").map_err(|e| e.to_string())?,
        company: r.try_get("company").map_err(|e| e.to_string())?,
        position: r.try_get("position").map_err(|e| e.to_string())?,
        location: r.try_get("location").map_err(|e| e.to_string())?,
        source: r.try_get("source").map_err(|e| e.to_string())?,
        status: r.try_get("status").map_err(|e| e.to_string())?,
        date_applied,
        date_follow_up,
        job_description: r.try_get("job_description").map_err(|e| e.to_string())?,
        link: r.try_get("link").map_err(|e| e.to_string())?,
        notes: r.try_get("notes").map_err(|e| e.to_string())?,
        rating_overall: r.try_get("rating_overall").map_err(|e| e.to_string())?,
        rating_location: r.try_get("rating_location").map_err(|e| e.to_string())?,
        rating_alignment: r.try_get("rating_alignment").map_err(|e| e.to_string())?,
        rating_salary: r.try_get("rating_salary").map_err(|e| e.to_string())?,
        rating_role: r.try_get("rating_role").map_err(|e| e.to_string())?,
        season: r.try_get("season").map_err(|e| e.to_string())?,
        created_at,
        follow_up_done: r.try_get("follow_up_done").unwrap_or(false),
    })
}

// Internal: call Groq and parse skill extraction response.
// Returns (required_skills, nicetohave_skills).
pub(crate) async fn do_extract_skills(
    client: &reqwest::Client,
    job_id: &str,
    jd_text: &str,
    pool: &sqlx::PgPool,
) -> Result<Vec<JobSkill>, String> {
    let api_key = load_api_key()?;

    let system_prompt = "You are a skill extractor. Given a job description, extract skills and \
        return ONLY a JSON object in this format: \
        {\"required\": [\"skill1\", \"skill2\"], \"nice_to_have\": [\"skill3\", \"skill4\"], \
        \"domain\": \"data science\", \"seniority\": \"intern\"} \
        Max 10 required skills, max 5 nice-to-have. Return ONLY the JSON, nothing else.";

    let user_prompt = format!("Job Description: {}", jd_text);

    let response = client
        .post(GROQ_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.3-70b-versatile",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_prompt }
            ],
            "temperature": 0.2,
            "max_tokens": 400,
            "response_format": { "type": "json_object" }
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Groq request failed: {}", e))?;

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Groq parse error: {}", e))?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");

    #[derive(Deserialize, Default)]
    struct SkillResponse {
        #[serde(default)]
        required: Vec<String>,
        #[serde(default)]
        nice_to_have: Vec<String>,
    }

    let parsed: SkillResponse = serde_json::from_str(content).unwrap_or_default();

    // Idempotent: clear existing skills for this job
    sqlx::query("DELETE FROM job_skills WHERE job_id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();

    for skill_name in &parsed.required {
        if skill_name.trim().is_empty() { continue; }
        let skill_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO job_skills (id, job_id, skill_name, is_required) VALUES ($1, $2, $3, true)"
        )
        .bind(&skill_id).bind(job_id).bind(skill_name.trim())
        .execute(pool).await.map_err(|e| e.to_string())?;
        result.push(JobSkill { id: skill_id, job_id: job_id.to_string(), skill_name: skill_name.trim().to_string(), is_required: true });
    }

    for skill_name in &parsed.nice_to_have {
        if skill_name.trim().is_empty() { continue; }
        let skill_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO job_skills (id, job_id, skill_name, is_required) VALUES ($1, $2, $3, false)"
        )
        .bind(&skill_id).bind(job_id).bind(skill_name.trim())
        .execute(pool).await.map_err(|e| e.to_string())?;
        result.push(JobSkill { id: skill_id, job_id: job_id.to_string(), skill_name: skill_name.trim().to_string(), is_required: false });
    }

    println!("🏷️  Extracted {} skills for job {}", result.len(), job_id);
    Ok(result)
}

// ─── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_job(
    company: String,
    position: String,
    location: Option<String>,
    source: Option<String>,
    link: Option<String>,
    season: String,
    database: State<'_, Database>,
) -> Result<JobApplication, String> {
    if company.trim().is_empty() || position.trim().is_empty() {
        return Err("Company and position are required".into());
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO job_applications (id, company, position, location, source, link, season) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(&id)
    .bind(company.trim())
    .bind(position.trim())
    .bind(&location)
    .bind(&source)
    .bind(&link)
    .bind(season.trim())
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT * FROM job_applications WHERE id = $1"
    )
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    row_to_job(&row)
}

#[tauri::command]
pub async fn get_jobs(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<JobApplication>, String> {
    let rows = if let Some(s) = &season {
        sqlx::query("SELECT * FROM job_applications WHERE season = $1 ORDER BY created_at DESC")
            .bind(s)
            .fetch_all(&database.pool)
            .await
    } else {
        sqlx::query("SELECT * FROM job_applications ORDER BY created_at DESC")
            .fetch_all(&database.pool)
            .await
    }
    .map_err(|e| e.to_string())?;

    rows.iter().map(row_to_job).collect()
}

#[tauri::command]
pub async fn update_job(
    id: String,
    status: Option<String>,
    notes: Option<String>,
    date_applied: Option<String>,
    date_follow_up: Option<String>,
    rating_overall: Option<i32>,
    rating_location: Option<i32>,
    rating_alignment: Option<i32>,
    rating_salary: Option<i32>,
    rating_role: Option<i32>,
    database: State<'_, Database>,
) -> Result<JobApplication, String> {
    // Parse date strings to TIMESTAMPTZ; null strings → NULL
    let parse_date = |s: Option<String>| -> Option<String> {
        s.filter(|d| !d.trim().is_empty())
    };
    let da = parse_date(date_applied);
    let dfu = parse_date(date_follow_up);

    sqlx::query(
        "UPDATE job_applications SET \
         status = COALESCE($2, status), \
         notes = $3, \
         date_applied = $4::DATE::TIMESTAMPTZ, \
         date_follow_up = $5::DATE::TIMESTAMPTZ, \
         rating_overall = $6, \
         rating_location = $7, \
         rating_alignment = $8, \
         rating_salary = $9, \
         rating_role = $10 \
         WHERE id = $1"
    )
    .bind(&id)
    .bind(&status)
    .bind(&notes)
    .bind(&da)
    .bind(&dfu)
    .bind(rating_overall)
    .bind(rating_location)
    .bind(rating_alignment)
    .bind(rating_salary)
    .bind(rating_role)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query("SELECT * FROM job_applications WHERE id = $1")
        .bind(&id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    row_to_job(&row)
}

#[tauri::command]
pub async fn delete_job(
    id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM job_applications WHERE id = $1")
        .bind(&id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn save_job_description(
    id: String,
    job_description: String,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<Vec<JobSkill>, String> {
    sqlx::query("UPDATE job_applications SET job_description = $1 WHERE id = $2")
        .bind(&job_description)
        .bind(&id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    do_extract_skills(&*client, &id, &job_description, &database.pool).await
}

#[tauri::command]
pub async fn extract_job_skills(
    id: String,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<Vec<JobSkill>, String> {
    let row = sqlx::query("SELECT job_description FROM job_applications WHERE id = $1")
        .bind(&id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| format!("Job not found: {}", e))?;

    let jd: Option<String> = row.try_get("job_description").map_err(|e| e.to_string())?;
    let jd_text = jd.ok_or_else(|| "No job description saved yet".to_string())?;

    do_extract_skills(&*client, &id, &jd_text, &database.pool).await
}

#[tauri::command]
pub async fn get_job_skills(
    job_id: String,
    database: State<'_, Database>,
) -> Result<Vec<JobSkill>, String> {
    let rows = sqlx::query(
        "SELECT id, job_id, skill_name, is_required FROM job_skills WHERE job_id = $1 ORDER BY is_required DESC, skill_name ASC"
    )
    .bind(&job_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter()
        .map(|r| -> Result<JobSkill, String> {
            Ok(JobSkill {
                id: r.try_get("id").map_err(|e| e.to_string())?,
                job_id: r.try_get("job_id").map_err(|e| e.to_string())?,
                skill_name: r.try_get("skill_name").map_err(|e| e.to_string())?,
                is_required: r.try_get("is_required").map_err(|e| e.to_string())?,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn get_skill_demand(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<SkillDemand>, String> {
    // Total job count (for frequency denominator)
    let total_row = if let Some(s) = &season {
        sqlx::query("SELECT COUNT(*) as cnt FROM job_applications WHERE season = $1")
            .bind(s)
            .fetch_one(&database.pool)
            .await
    } else {
        sqlx::query("SELECT COUNT(*) as cnt FROM job_applications")
            .fetch_one(&database.pool)
            .await
    }
    .map_err(|e| e.to_string())?;

    let total_jobs: i64 = total_row.try_get("cnt").map_err(|e| e.to_string())?;
    println!("📊 get_skill_demand: total_jobs={}", total_jobs);
    if total_jobs == 0 {
        return Ok(vec![]);
    }

    // Skill aggregates — avg_rating only counts jobs where at least one rating is set.
    // NULL ratings are excluded from the average so unrated jobs don't dilute the score.
    let agg_sql_base =
        "SELECT js.skill_name, \
         COUNT(DISTINCT js.job_id) as count, \
         AVG(NULLIF(\
           (COALESCE(ja.rating_location, 0) + COALESCE(ja.rating_alignment, 0) + \
            COALESCE(ja.rating_salary, 0) + COALESCE(ja.rating_role, 0))::float / \
           NULLIF((CASE WHEN ja.rating_location IS NOT NULL THEN 1 ELSE 0 END + \
                   CASE WHEN ja.rating_alignment IS NOT NULL THEN 1 ELSE 0 END + \
                   CASE WHEN ja.rating_salary IS NOT NULL THEN 1 ELSE 0 END + \
                   CASE WHEN ja.rating_role IS NOT NULL THEN 1 ELSE 0 END), 0)\
         , 0)) as avg_rating \
         FROM job_skills js \
         JOIN job_applications ja ON js.job_id = ja.id";

    let rows = if let Some(s) = &season {
        sqlx::query(&format!("{} WHERE ja.season = $1 GROUP BY js.skill_name", agg_sql_base))
            .bind(s)
            .fetch_all(&database.pool)
            .await
    } else {
        sqlx::query(&format!("{} GROUP BY js.skill_name", agg_sql_base))
            .fetch_all(&database.pool)
            .await
    }
    .map_err(|e| e.to_string())?;

    let mut demands: Vec<SkillDemand> = rows
        .iter()
        .map(|r| -> Result<SkillDemand, String> {
            let skill_name: String = r.try_get("skill_name").map_err(|e| e.to_string())?;
            let count: i64 = r.try_get("count").map_err(|e| e.to_string())?;
            let avg_rating: Option<f64> = r.try_get("avg_rating").map_err(|e| e.to_string())?;
            let frequency = count as f64 / total_jobs as f64;
            // demand_score: frequency * 100 when no ratings, or frequency * (rating/5) * 100 when rated.
            // This avoids the COALESCE-3 hack that made every unrated skill score 60% of frequency.
            let demand_score = match avg_rating {
                Some(r) if r > 0.0 => frequency * (r / 5.0) * 100.0,
                _ => frequency * 100.0,
            };
            let avg_rating = avg_rating.unwrap_or(0.0);
            println!(
                "  📊 Skill '{}': count={}/{} (freq={:.0}%), avg_rating={:.2}, demand_score={:.1}",
                skill_name, count, total_jobs,
                frequency * 100.0,
                avg_rating,
                demand_score
            );
            Ok(SkillDemand {
                skill_name,
                count,
                total_jobs,
                frequency,
                avg_rating,
                demand_score,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    demands.sort_by(|a, b| b.demand_score.partial_cmp(&a.demand_score).unwrap_or(std::cmp::Ordering::Equal));
    demands.truncate(20);
    Ok(demands)
}

#[tauri::command]
pub async fn mark_followed_up(
    id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE job_applications SET date_follow_up = NOW(), follow_up_done = TRUE WHERE id = $1"
    )
    .bind(&id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReextractResult {
    pub processed: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn reextract_all_skills(
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<ReextractResult, String> {
    let rows = sqlx::query(
        "SELECT id, company, position, job_description FROM job_applications \
         WHERE job_description IS NOT NULL AND job_description != ''",
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut processed = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let job_id: String = match row.try_get("id") {
            Ok(v) => v,
            Err(e) => {
                failed += 1;
                errors.push(format!("Row read error: {}", e));
                continue;
            }
        };
        let company: String = row.try_get("company").unwrap_or_default();
        let position: String = row.try_get("position").unwrap_or_default();
        let jd: String = match row.try_get("job_description") {
            Ok(v) => v,
            Err(e) => {
                failed += 1;
                errors.push(format!("{} @ {} — {}", position, company, e));
                continue;
            }
        };

        if jd.trim().is_empty() {
            continue;
        }

        match do_extract_skills(&*client, &job_id, &jd, &database.pool).await {
            Ok(_) => processed += 1,
            Err(e) => {
                failed += 1;
                errors.push(format!("{} @ {} — {}", position, company, e));
            }
        }
    }

    println!("🏷️  Reextract all: {} processed, {} failed", processed, failed);
    Ok(ReextractResult { processed, failed, errors })
}

#[tauri::command]
pub async fn save_tailored_projects(
    job_id: String,
    tailored_projects_json: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE job_applications SET tailored_projects = $2 WHERE id = $1"
    )
    .bind(&job_id)
    .bind(&tailored_projects_json)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
