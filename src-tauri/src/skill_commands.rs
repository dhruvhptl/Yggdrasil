// src-tauri/src/skill_commands.rs
// Universal Skill Tree: sync skills from all sources, calculate levels, detect gaps.
// All queries use sqlx::query() non-macro to avoid offline cache issues.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use tauri::State;
use uuid::Uuid;

use crate::database::Database;

// ─── Structs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalSkill {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub level: i32,
    pub evidence: serde_json::Value,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDependency {
    pub id: String,
    pub source_skill_id: String,
    pub target_skill_id: String,
    pub relationship: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGap {
    pub skill_name: String,
    pub demand_count: i64,
    pub frequency: f64,
    pub demand_score: f64,
    pub current_level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub upserted: usize,
    pub source: String,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

async fn upsert_skill(
    pool: &PgPool,
    name: &str,
    domain: Option<&str>,
    evidence_entry: serde_json::Value,
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Empty skill name".to_string());
    }
    let id = Uuid::new_v4().to_string();
    let evidence_arr = serde_json::json!([evidence_entry]);

    // ON CONFLICT uses a case-insensitive expression index on LOWER(name).
    // If an existing row has a different casing, we keep the existing name
    // (DO UPDATE does not update `name`) so the first-inserted casing wins.
    let row = sqlx::query(
        "INSERT INTO universal_skills (id, name, domain, level, evidence, last_updated)
         VALUES ($1, $2, $3, 1, $4::jsonb, NOW())
         ON CONFLICT (LOWER(name)) DO UPDATE SET
             evidence = universal_skills.evidence || $4::jsonb,
             domain = COALESCE($3, universal_skills.domain),
             last_updated = NOW()
         RETURNING id"
    )
    .bind(&id)
    .bind(trimmed)
    .bind(domain)
    .bind(&evidence_arr)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("upsert_skill failed: {}", e))?;

    row.try_get::<String, _>("id").map_err(|e| e.to_string())
}

// ─── Inner sync functions (take &PgPool directly) ───────────────────────────

async fn sync_resume_inner(pool: &PgPool) -> Result<SyncResult, String> {
    let row = sqlx::query(
        "SELECT skills FROM resume_profile ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let skills_json = match row {
        Some(r) => r.try_get::<serde_json::Value, _>("skills").map_err(|e| e.to_string())?,
        None => return Ok(SyncResult { upserted: 0, source: "resume".into() }),
    };

    let skills: Vec<String> = serde_json::from_value(skills_json).unwrap_or_default();
    let mut count = 0;

    for skill_name in &skills {
        if skill_name.trim().is_empty() { continue; }
        let evidence = serde_json::json!({
            "type": "resume",
            "detail": "Listed on resume"
        });
        upsert_skill(pool, skill_name, None, evidence).await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "resume".into() })
}

async fn sync_trees_inner(pool: &PgPool) -> Result<SyncResult, String> {
    // Skill nodes: branch nodes whose parent is also a branch (phase).
    // trunk -> phase (branch, parent=trunk) -> skill (branch, parent=phase) -> leaf (quest)
    // We want: n.type='branch' AND parent.type='branch'
    let rows = sqlx::query(
        "SELECT n.id, n.title, n.progress, n.tree_id,
                t.name AS tree_name, p.name AS project_name
         FROM tree_nodes n
         JOIN tree_nodes parent ON n.parent_id = parent.id
         JOIN trees t ON n.tree_id = t.id
         JOIN projects p ON t.project_id = p.id
         WHERE n.type = 'branch'
           AND parent.type = 'branch'
           AND n.progress = 100"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for row in &rows {
        let title: String = row.try_get("title").map_err(|e| e.to_string())?;
        let progress: i32 = row.try_get("progress").unwrap_or(0);
        let tree_name: String = row.try_get("tree_name").map_err(|e| e.to_string())?;
        let project_name: String = row.try_get("project_name").map_err(|e| e.to_string())?;
        let node_id: String = row.try_get("id").map_err(|e| e.to_string())?;

        let evidence = serde_json::json!({
            "type": "tree_quest",
            "projectName": project_name,
            "treeName": tree_name,
            "nodeTitle": title,
            "nodeId": node_id,
            "progress": progress
        });

        upsert_skill(pool, &title, None, evidence).await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "trees".into() })
}

async fn sync_work_inner(pool: &PgPool) -> Result<SyncResult, String> {
    let rows = sqlx::query(
        "SELECT wrs.skill_name, wr.title AS resource_title, wr.id AS resource_id,
                ct.company
         FROM work_resource_skills wrs
         JOIN work_resources wr ON wrs.resource_id = wr.id
         JOIN research_topics rt ON wr.topic_id = rt.id
         JOIN coop_terms ct ON rt.coop_id = ct.id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for row in &rows {
        let skill_name: String = row.try_get("skill_name").map_err(|e| e.to_string())?;
        let resource_title: String = row.try_get("resource_title").map_err(|e| e.to_string())?;
        let resource_id: String = row.try_get("resource_id").map_err(|e| e.to_string())?;
        let company: String = row.try_get("company").map_err(|e| e.to_string())?;

        let evidence = serde_json::json!({
            "type": "work_resource",
            "company": company,
            "resourceTitle": resource_title,
            "resourceId": resource_id
        });

        upsert_skill(pool, &skill_name, None, evidence).await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "work".into() })
}

async fn recalculate_levels_inner(pool: &PgPool) -> Result<usize, String> {
    let rows = sqlx::query("SELECT id, evidence FROM universal_skills")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut updated = 0;
    for row in &rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let evidence: serde_json::Value = row.try_get("evidence").map_err(|e| e.to_string())?;
        let entries = evidence.as_array().cloned().unwrap_or_default();

        let tree_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "tree_quest")
            .collect();
        let work_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "work_resource")
            .collect();

        // Unique project contexts from tree evidence
        let unique_projects: HashSet<String> = tree_entries.iter()
            .filter_map(|e| e["projectName"].as_str().map(|s| s.to_lowercase()))
            .collect();

        // Count completed skill nodes (progress == 100)
        let completed_skills = tree_entries.iter()
            .filter(|e| e["progress"].as_i64().unwrap_or(0) == 100)
            .count();

        let has_work = !work_entries.is_empty();
        let total_evidence = entries.len();

        // PRD level rules
        let level: i32 = if total_evidence >= 8 && unique_projects.len() >= 3 && has_work {
            5 // Expert
        } else if unique_projects.len() >= 2 && (has_work || completed_skills >= 2) {
            4 // Advanced
        } else if unique_projects.len() >= 2 || (completed_skills >= 1 && has_work) {
            3 // Proficient
        } else if completed_skills >= 1 {
            2 // Familiar
        } else {
            1 // Aware
        };

        sqlx::query("UPDATE universal_skills SET level = $1, last_updated = NOW() WHERE id = $2")
            .bind(level)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

        updated += 1;
    }

    Ok(updated)
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn sync_skills_from_resume(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_resume_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_skills_from_trees(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_trees_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_skills_from_work(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_work_inner(&database.pool).await
}

#[tauri::command]
pub async fn recalculate_skill_levels(
    database: State<'_, Database>,
) -> Result<usize, String> {
    recalculate_levels_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_all_skills(
    database: State<'_, Database>,
) -> Result<Vec<SyncResult>, String> {
    // Clear all evidence for a clean full sync
    sqlx::query("UPDATE universal_skills SET evidence = '[]'::jsonb")
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let r1 = sync_resume_inner(&database.pool).await?;
    let r2 = sync_trees_inner(&database.pool).await?;
    let r3 = sync_work_inner(&database.pool).await?;

    // Prune skills that still have empty evidence after the full sync —
    // these are stale rows whose sources (resources, tree nodes) no longer exist.
    let pruned = sqlx::query(
        "DELETE FROM skill_dependencies
         WHERE source_skill_id IN (SELECT id FROM universal_skills WHERE evidence = '[]'::jsonb)
            OR target_skill_id IN (SELECT id FROM universal_skills WHERE evidence = '[]'::jsonb)"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let deleted = sqlx::query(
        "DELETE FROM universal_skills WHERE evidence = '[]'::jsonb"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    if deleted > 0 {
        println!("🌳 Pruned {} stale skills ({} dep rows)", deleted, pruned);
    }

    recalculate_levels_inner(&database.pool).await?;

    println!("🌳 Synced all skills: resume={}, trees={}, work={}", r1.upserted, r2.upserted, r3.upserted);
    Ok(vec![r1, r2, r3])
}

#[tauri::command]
pub async fn get_universal_skills(
    database: State<'_, Database>,
) -> Result<Vec<UniversalSkill>, String> {
    let rows = sqlx::query(
        "SELECT id, name, domain, level, evidence, last_updated
         FROM universal_skills ORDER BY level DESC, name ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter().map(|r| {
        Ok(UniversalSkill {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            name: r.try_get("name").map_err(|e| e.to_string())?,
            domain: r.try_get("domain").map_err(|e| e.to_string())?,
            level: r.try_get("level").map_err(|e| e.to_string())?,
            evidence: r.try_get("evidence").map_err(|e| e.to_string())?,
            last_updated: r.try_get::<chrono::DateTime<chrono::Utc>, _>("last_updated")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e| e.to_string())?,
        })
    }).collect()
}

#[tauri::command]
pub async fn get_skill_dependencies(
    database: State<'_, Database>,
) -> Result<Vec<SkillDependency>, String> {
    let rows = sqlx::query(
        "SELECT id, source_skill_id, target_skill_id, relationship
         FROM skill_dependencies"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter().map(|r| {
        Ok(SkillDependency {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            source_skill_id: r.try_get("source_skill_id").map_err(|e| e.to_string())?,
            target_skill_id: r.try_get("target_skill_id").map_err(|e| e.to_string())?,
            relationship: r.try_get("relationship").map_err(|e| e.to_string())?,
        })
    }).collect()
}

#[tauri::command]
pub async fn get_skill_gaps(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<SkillGap>, String> {
    // Single query: join job_skills with universal_skills in one pass.
    // LEFT JOIN on LOWER(name) so we get current_level = NULL when skill is absent.
    let demand_rows = if let Some(ref s) = season {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS count,
                    (SELECT COUNT(*) FROM job_applications WHERE season = $1) AS total,
                    MAX(us.level) AS current_level
             FROM job_skills js
             JOIN job_applications ja ON js.job_id = ja.id
             LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
             WHERE ja.season = $1
             GROUP BY js.skill_name"
        )
        .bind(s)
        .fetch_all(&database.pool)
        .await
    } else {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS count,
                    (SELECT COUNT(*) FROM job_applications) AS total,
                    MAX(us.level) AS current_level
             FROM job_skills js
             LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
             GROUP BY js.skill_name"
        )
        .fetch_all(&database.pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    let mut gaps = Vec::new();
    for row in &demand_rows {
        let skill_name: String = row.try_get("skill_name").map_err(|e| e.to_string())?;
        let count: i64 = row.try_get("count").map_err(|e| e.to_string())?;
        let total: i64 = row.try_get("total").map_err(|e| e.to_string())?;
        if total == 0 { continue; }

        let frequency = count as f64 / total as f64;
        let demand_score = frequency * 100.0;

        // NULL level means skill is absent from universal_skills → treat as 0
        let current_level: i32 = row.try_get::<Option<i32>, _>("current_level")
            .unwrap_or(None)
            .unwrap_or(0);

        // Gap = demanded but level 0 (missing) or level 1 (only aware)
        if current_level <= 1 {
            gaps.push(SkillGap {
                skill_name,
                demand_count: count,
                frequency,
                demand_score,
                current_level,
            });
        }
    }

    gaps.sort_by(|a, b| b.demand_score.partial_cmp(&a.demand_score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(gaps)
}

#[tauri::command]
pub async fn infer_skill_dependencies(
    database: State<'_, Database>,
) -> Result<usize, String> {
    let rows = sqlx::query("SELECT id, name FROM universal_skills ORDER BY name")
        .fetch_all(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if rows.len() < 2 { return Ok(0); }

    let skill_names: Vec<String> = rows.iter()
        .map(|r| r.try_get::<String, _>("name").unwrap_or_default())
        .collect();

    // Build name -> id map
    let name_to_id: std::collections::HashMap<String, String> = rows.iter()
        .map(|r| {
            let name: String = r.try_get("name").unwrap_or_default();
            let id: String = r.try_get("id").unwrap_or_default();
            (name, id)
        })
        .collect();

    // Load API key
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    let system_prompt = "Given a list of technical skills, identify prerequisite relationships. \
        Return ONLY a JSON array of objects: [{\"from\": \"skill_a\", \"to\": \"skill_b\"}] \
        where skill_a is a prerequisite for skill_b. Only include strong, clear prerequisites. \
        Use the exact skill names provided. Max 30 relationships. Return ONLY the JSON array, no markdown.";

    #[derive(Deserialize)]
    struct DepPair {
        from: String,
        to: String,
    }

    // Build a lowercase name → id lookup to match LLM output case-insensitively
    let lower_to_id: std::collections::HashMap<String, String> = name_to_id.iter()
        .map(|(name, id)| (name.to_lowercase(), id.clone()))
        .collect();

    let client = reqwest::Client::new();

    // Batch into groups of 30 to stay well within token limits
    const BATCH_SIZE: usize = 30;
    let batches: Vec<&[String]> = skill_names.chunks(BATCH_SIZE).collect();
    let mut all_pairs: Vec<DepPair> = Vec::new();

    for (i, batch) in batches.iter().enumerate() {
        if i > 0 {
            // Small delay between batches to avoid rate limiting
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        let user_prompt = format!("Skills: {}", batch.join(", "));

        let response = client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "llama-3.3-70b-versatile",
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.3,
                "max_tokens": 1000
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Groq request failed (batch {}): {}", i, e))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Groq parse error (batch {}): {}", i, e))?;

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("[]");

        // Strip markdown fences before parsing
        let clean = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let batch_pairs: Vec<DepPair> = serde_json::from_str(clean).unwrap_or_default();
        println!("  🔗 Batch {}: {} pairs from {} skills", i + 1, batch_pairs.len(), batch.len());
        all_pairs.extend(batch_pairs);
    }

    // Clear existing inferred dependencies
    sqlx::query("DELETE FROM skill_dependencies WHERE relationship = 'prerequisite'")
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut count = 0;
    for pair in &all_pairs {
        let from_id = lower_to_id.get(&pair.from.to_lowercase());
        let to_id = lower_to_id.get(&pair.to.to_lowercase());

        if let (Some(from_id), Some(to_id)) = (from_id, to_id) {
            if from_id == to_id { continue; }
            let dep_id = Uuid::new_v4().to_string();
            let result = sqlx::query(
                "INSERT INTO skill_dependencies (id, source_skill_id, target_skill_id, relationship)
                 VALUES ($1, $2, $3, 'prerequisite')
                 ON CONFLICT (source_skill_id, target_skill_id) DO NOTHING"
            )
            .bind(&dep_id)
            .bind(from_id)
            .bind(to_id)
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;

            if result.rows_affected() > 0 {
                count += 1;
            }
        }
    }

    println!("🔗 Inferred {} skill dependencies from {} total pairs ({} batches)", count, all_pairs.len(), batches.len());
    Ok(count)
}
