// src-tauri/src/graph_audit.rs
//
// Mimir brain — graph health audit + proposal system.
// The LLM proposes changes; nothing is auto-applied. All proposals sit in
// mimir_proposals until a human approves or rejects them via the review UI.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tauri::State;
use crate::database::Database;

// ─── Public types (returned to frontend) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    #[serde(rename = "type")]
    pub proposal_type: String,
    pub payload: serde_json::Value,
    pub llm_reasoning: String,
    pub status: String,
    pub created_at: String,
    pub reviewed_at: Option<String>,
}

// ─── run_graph_audit ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_graph_audit(
    database: State<'_, Database>,
    client: State<'_, reqwest::Client>,
) -> Result<usize, String> {
    let pool = &database.pool;
    let client = &*client;

    // ── 1. Duplicate candidates by embedding cosine distance ─────────────────
    let dup_rows = sqlx::query(
        "SELECT a.id AS id_a, a.name AS name_a,
                b.id AS id_b, b.name AS name_b,
                (a.embedding <=> b.embedding)::float8 AS distance
         FROM universal_skills a
         JOIN universal_skills b ON a.id < b.id
         WHERE a.embedding IS NOT NULL
           AND b.embedding IS NOT NULL
           AND a.embedding <=> b.embedding < 0.12
         ORDER BY distance ASC
         LIMIT 60"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("duplicate query failed: {}", e))?;

    // ── 2. Vague root nodes (too many dependents in skill_dependencies) ───────
    // "source_skill_id" is the prerequisite; count skills that depend on it.
    let vague_rows = sqlx::query(
        "SELECT sd.source_skill_id AS skill_id, us.name, COUNT(*) AS dependent_count
         FROM skill_dependencies sd
         JOIN universal_skills us ON us.id = sd.source_skill_id
         GROUP BY sd.source_skill_id, us.name
         HAVING COUNT(*) > 20
         ORDER BY dependent_count DESC
         LIMIT 20"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("vague roots query failed: {}", e))?;

    if dup_rows.is_empty() && vague_rows.is_empty() {
        println!("🔍 [audit] No issues found — skipping LLM call");
        return Ok(0);
    }

    // ── 3. Build LLM prompt ───────────────────────────────────────────────────
    let mut dup_lines = String::new();
    // Build a lookup of id → name for validation later
    let mut id_to_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for row in &dup_rows {
        let id_a: String = row.try_get("id_a").unwrap_or_default();
        let name_a: String = row.try_get("name_a").unwrap_or_default();
        let id_b: String = row.try_get("id_b").unwrap_or_default();
        let name_b: String = row.try_get("name_b").unwrap_or_default();
        let dist: f64 = row.try_get("distance").unwrap_or(1.0);
        dup_lines.push_str(&format!(
            "  - '{}' (id:{}) and '{}' (id:{}) — distance: {:.4}\n",
            name_a, id_a, name_b, id_b, dist
        ));
        id_to_name.insert(id_a, name_a);
        id_to_name.insert(id_b, name_b);
    }

    let mut vague_lines = String::new();
    for row in &vague_rows {
        let sid: String = row.try_get("skill_id").unwrap_or_default();
        let name: String = row.try_get("name").unwrap_or_default();
        let count: i64 = row.try_get("dependent_count").unwrap_or(0);
        vague_lines.push_str(&format!("  - '{}' (id:{}) has {} dependents\n", name, sid, count));
        id_to_name.insert(sid, name);
    }

    let system_prompt = "You are a knowledge graph curator. Review potential issues in a skill graph \
        and propose specific repair actions. Be conservative — only propose merges for clearly \
        identical concepts (e.g. 'ML' and 'Machine Learning'). For vague root nodes, propose a \
        rename to something more specific, or dismiss if appropriate. \
        Return a JSON array of proposal objects. Each object must have: \
        { \"type\": \"merge_skills\"|\"delete_skill\"|\"rename_skill\"|\"dismiss\", \
          \"payload\": { ... }, \
          \"reasoning\": \"one sentence\" } \
        For merge_skills payload: { \"keep_id\": \"...\", \"merge_id\": \"...\" } \
        For delete_skill payload: { \"skill_id\": \"...\" } \
        For rename_skill payload: { \"skill_id\": \"...\", \"new_name\": \"...\" } \
        For dismiss payload: { \"skill_id\": \"...\" } or { \"id_a\": \"...\", \"id_b\": \"...\" } \
        Return ONLY the JSON array, no other text.";

    let user_prompt = format!(
        "Potential duplicate skill pairs (by embedding similarity):\n{}\n\
         Overly broad root nodes (skills with 20+ dependents):\n{}\n\
         Propose repairs for each issue. Use the exact IDs shown.",
        if dup_lines.is_empty() { "  (none)\n".to_string() } else { dup_lines },
        if vague_lines.is_empty() { "  (none)\n".to_string() } else { vague_lines },
    );

    // ── 4. LLM call ───────────────────────────────────────────────────────────
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not set".to_string())?;
    let base_url = "https://openrouter.ai/api/v1/chat/completions";
    let model = "google/gemini-3.1-flash-lite";

    let t_start = std::time::Instant::now();
    let (content, latency_ms) = crate::llm_client::call_llm(
        client, base_url, &api_key, model,
        system_prompt, &user_prompt,
        2000, false,
    ).await.unwrap_or_else(|e| {
        println!("⚠️  [audit] LLM call failed: {}", e);
        (String::new(), t_start.elapsed().as_millis() as i64)
    });

    crate::brain::log_prompt_call(
        pool.clone(), "run_graph_audit", model,
        "graph_audit_v1", latency_ms, !content.is_empty(), None, None,
    );

    if content.is_empty() { return Ok(0); }

    // ── 5. Parse + validate proposals ─────────────────────────────────────────
    let clean = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let raw_proposals: Vec<serde_json::Value> = serde_json::from_str(clean)
        .unwrap_or_default();

    if raw_proposals.is_empty() {
        println!("⚠️  [audit] LLM returned no proposals");
        return Ok(0);
    }

    // Validate: all skill IDs referenced in payload must exist
    let valid_ids: std::collections::HashSet<&str> = id_to_name.keys()
        .map(|s| s.as_str())
        .collect();

    let mut written = 0usize;
    for raw in &raw_proposals {
        let proposal_type = match raw["type"].as_str() {
            Some("merge_skills") => "merge_skills",
            Some("delete_skill") => "delete_skill",
            Some("rename_skill") => "rename_skill",
            Some("dismiss") => continue, // dismiss = no proposal needed
            _ => {
                println!("⚠️  [audit] Unknown proposal type: {}", raw["type"]);
                continue;
            }
        };

        let payload = raw["payload"].clone();
        let reasoning = raw["reasoning"].as_str().unwrap_or("").to_string();
        if reasoning.is_empty() { continue; }

        // Validate IDs in payload and enrich with names
        let enriched_payload: serde_json::Value = match proposal_type {
            "merge_skills" => {
                let a = payload["keep_id"].as_str().unwrap_or("");
                let b = payload["merge_id"].as_str().unwrap_or("");
                if a.is_empty() || b.is_empty() || !valid_ids.contains(a) || !valid_ids.contains(b) {
                    println!("⚠️  [audit] Skipping merge proposal with invalid IDs: {:?}", payload);
                    continue;
                }
                serde_json::json!({
                    "keep_id": a,
                    "keep_name": id_to_name.get(a).map(|s| s.as_str()).unwrap_or(""),
                    "merge_id": b,
                    "merge_name": id_to_name.get(b).map(|s| s.as_str()).unwrap_or(""),
                })
            }
            "delete_skill" => {
                let sid = payload["skill_id"].as_str().unwrap_or("");
                if sid.is_empty() || !valid_ids.contains(sid) {
                    println!("⚠️  [audit] Skipping delete proposal with invalid ID: {:?}", payload);
                    continue;
                }
                serde_json::json!({
                    "skill_id": sid,
                    "skill_name": id_to_name.get(sid).map(|s| s.as_str()).unwrap_or(""),
                })
            }
            "rename_skill" => {
                let sid = payload["skill_id"].as_str().unwrap_or("");
                let new_name = payload["new_name"].as_str().unwrap_or("");
                if sid.is_empty() || !valid_ids.contains(sid) || new_name.is_empty() {
                    println!("⚠️  [audit] Skipping rename proposal with invalid IDs: {:?}", payload);
                    continue;
                }
                serde_json::json!({
                    "skill_id": sid,
                    "skill_name": id_to_name.get(sid).map(|s| s.as_str()).unwrap_or(""),
                    "new_name": new_name,
                })
            }
            _ => continue,
        };

        let proposal_id = uuid::Uuid::new_v4().to_string();
        let res = sqlx::query(
            "INSERT INTO mimir_proposals (id, type, payload, llm_reasoning, status) \
             VALUES ($1, $2, $3::jsonb, $4, 'pending')"
        )
        .bind(&proposal_id)
        .bind(proposal_type)
        .bind(enriched_payload.to_string())
        .bind(&reasoning)
        .execute(pool)
        .await;

        match res {
            Ok(_) => {
                written += 1;
                println!("📋 [audit] Proposal: {} — {}", proposal_type, reasoning);
            }
            Err(e) => println!("⚠️  [audit] Insert failed: {}", e),
        }
    }

    println!("✅ [audit] {} proposals created", written);
    Ok(written)
}

// ─── get_pending_proposals ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_pending_proposals(
    database: State<'_, Database>,
) -> Result<Vec<Proposal>, String> {
    let rows = sqlx::query(
        "SELECT id, type, payload::TEXT AS payload, llm_reasoning, status, \
                created_at::TEXT AS created_at, \
                reviewed_at::TEXT AS reviewed_at \
         FROM mimir_proposals \
         WHERE status = 'pending' \
         ORDER BY created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in &rows {
        let id: String = match r.try_get("id") {
            Ok(v) => v,
            Err(e) => { println!("⚠️  [audit] row missing id: {}", e); continue; }
        };
        let proposal_type: String = match r.try_get("type") {
            Ok(v) => v,
            Err(e) => { println!("⚠️  [audit] row {} missing type: {}", id, e); continue; }
        };
        let payload_str: String = match r.try_get("payload") {
            Ok(v) => v,
            Err(e) => { println!("⚠️  [audit] row {} missing payload: {}", id, e); continue; }
        };
        let payload: serde_json::Value = match serde_json::from_str(&payload_str) {
            Ok(v) => v,
            Err(e) => { println!("⚠️  [audit] row {} bad payload JSON: {}", id, e); continue; }
        };
        out.push(Proposal {
            id,
            proposal_type,
            payload,
            llm_reasoning: r.try_get("llm_reasoning").unwrap_or_default(),
            status: r.try_get("status").unwrap_or_default(),
            created_at: r.try_get("created_at").unwrap_or_default(),
            reviewed_at: r.try_get("reviewed_at").ok().flatten(),
        });
    }
    Ok(out)
}

// ─── approve_proposal ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn approve_proposal(
    proposal_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    let pool = &database.pool;

    // Fetch the proposal
    let row = sqlx::query(
        "SELECT type, payload::TEXT AS payload FROM mimir_proposals WHERE id = $1 AND status = 'pending'"
    )
    .bind(&proposal_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Proposal not found or already reviewed".to_string())?;

    let proposal_type: String = row.try_get("type").map_err(|e| e.to_string())?;
    let payload_str: String = row.try_get("payload").map_err(|e| e.to_string())?;
    let payload: serde_json::Value = serde_json::from_str(&payload_str)
        .map_err(|e| format!("invalid payload JSON: {}", e))?;

    match proposal_type.as_str() {
        "merge_skills" => {
            let keep_id = payload["keep_id"].as_str()
                .ok_or("payload missing keep_id")?;
            let merge_id = payload["merge_id"].as_str()
                .ok_or("payload missing merge_id")?;

            apply_merge(pool, keep_id, merge_id).await?;
        }
        "delete_skill" => {
            let skill_id = payload["skill_id"].as_str()
                .ok_or("payload missing skill_id")?;
            sqlx::query("DELETE FROM universal_skills WHERE id = $1")
                .bind(skill_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            println!("🗑️  [audit/approve] deleted skill {}", skill_id);
        }
        "rename_skill" => {
            let skill_id = payload["skill_id"].as_str()
                .ok_or("payload missing skill_id")?;
            let new_name = payload["new_name"].as_str()
                .ok_or("payload missing new_name")?;
            sqlx::query(
                "UPDATE universal_skills SET name = $1 WHERE id = $2"
            )
            .bind(new_name)
            .bind(skill_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            println!("✏️  [audit/approve] renamed skill {} → '{}'", skill_id, new_name);
        }
        other => return Err(format!("Unknown proposal type: {}", other)),
    }

    // Mark approved
    sqlx::query(
        "UPDATE mimir_proposals SET status = 'approved', reviewed_at = NOW() WHERE id = $1"
    )
    .bind(&proposal_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ─── reject_proposal ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn reject_proposal(
    proposal_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE mimir_proposals SET status = 'rejected', reviewed_at = NOW() WHERE id = $1"
    )
    .bind(&proposal_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── clear_all_proposals ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn clear_all_proposals(
    database: State<'_, Database>,
) -> Result<u64, String> {
    let result = sqlx::query(
        "DELETE FROM mimir_proposals WHERE status = 'pending'"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

// ─── apply_merge (internal) ───────────────────────────────────────────────────

async fn apply_merge(pool: &PgPool, keep_id: &str, merge_id: &str) -> Result<(), String> {
    // Look up concept_slug for the merge target before deleting it
    let merge_slug: Option<String> = sqlx::query(
        "SELECT concept_slug FROM universal_skills WHERE id = $1"
    )
    .bind(merge_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .and_then(|r| r.try_get("concept_slug").ok());

    let keep_slug: Option<String> = sqlx::query(
        "SELECT concept_slug FROM universal_skills WHERE id = $1"
    )
    .bind(keep_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .and_then(|r| r.try_get("concept_slug").ok());

    // Repoint mimir_skill_links — delete rows that would collide with keep's existing links,
    // then repoint the rest. (PK is (skill_id, resource_id))
    sqlx::query(
        "DELETE FROM mimir_skill_links \
         WHERE skill_id = $1 \
           AND resource_id IN (SELECT resource_id FROM mimir_skill_links WHERE skill_id = $2)"
    )
    .bind(merge_id)
    .bind(keep_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE mimir_skill_links SET skill_id = $1 WHERE skill_id = $2"
    )
    .bind(keep_id)
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Repoint skill_dependencies edges (both directions), skip collisions
    sqlx::query(
        "UPDATE skill_dependencies SET source_skill_id = $1 \
         WHERE source_skill_id = $2 \
           AND NOT EXISTS (SELECT 1 FROM skill_dependencies sd2 \
                           WHERE sd2.source_skill_id = $1 AND sd2.target_skill_id = skill_dependencies.target_skill_id)"
    )
    .bind(keep_id)
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM skill_dependencies WHERE source_skill_id = $1"
    )
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE skill_dependencies SET target_skill_id = $1 \
         WHERE target_skill_id = $2 \
           AND NOT EXISTS (SELECT 1 FROM skill_dependencies sd2 \
                           WHERE sd2.source_skill_id = skill_dependencies.source_skill_id AND sd2.target_skill_id = $1)"
    )
    .bind(keep_id)
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM skill_dependencies WHERE target_skill_id = $1"
    )
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Remove any self-loops
    sqlx::query(
        "DELETE FROM skill_dependencies WHERE source_skill_id = target_skill_id"
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Repoint tree_nodes.concept_slug if merge target had a slug
    if let (Some(ref ms), Some(ref ks)) = (&merge_slug, &keep_slug) {
        sqlx::query(
            "UPDATE tree_nodes SET concept_slug = $1 WHERE concept_slug = $2"
        )
        .bind(ks)
        .bind(ms)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Delete the merged skill (FK cascades handle skill_evidence, skill_aliases etc.)
    sqlx::query("DELETE FROM universal_skills WHERE id = $1")
        .bind(merge_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    println!("🔀 [audit/approve] merged {} → {}", merge_id, keep_id);
    Ok(())
}
