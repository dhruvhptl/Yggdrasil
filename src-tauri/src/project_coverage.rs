// src-tauri/src/project_coverage.rs
//
// Tree-vs-repo coverage pass: for each skill checkpoint in a tree, ANN-search
// the project's scanned concept graph for candidate evidence symbols, then one
// batched LLM call classifies each checkpoint as covered/partial/gap. Results
// are upserted into `project_coverage` (idempotent on re-run).
//
// Run INLINE at the end of a successful project scan (project_scanner.rs) —
// NOT via an orchestrator job. The orchestrator worker holds no JobQueue sender
// (see orchestrator.rs's sequential consolidate -> extract-facts chain for the
// same reason), so a follow-up job can't be enqueued from inside the worker.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSummary {
    pub covered: u32,
    pub partial: u32,
    pub gap: u32,
    pub total: u32,
}

/// One LLM verdict for a single checkpoint. Groq's `json_object` mode requires
/// the model to return an OBJECT, not a bare array — so this is parsed via the
/// `CoverageResponse` wrapper below, never deserialized standalone from the
/// top-level LLM response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LlmVerdict {
    checkpoint_id: String,
    status: String,
    evidence_node_id: Option<String>,
    reason: String,
}

/// Wrapper matching the `{"verdicts":[...]}` shape the LLM is prompted to
/// return (Groq json_mode rejects a bare top-level array).
#[derive(Debug, Deserialize)]
struct CoverageResponse {
    verdicts: Vec<LlmVerdict>,
}

/// Gather the tree's skill checkpoints, ANN-match each against the project's
/// scanned code graph for evidence candidates, then classify all of them in one
/// batched LLM call. Upserts `project_coverage` rows and returns tallied counts.
pub(crate) async fn coverage_for_project(
    pool: &PgPool,
    client: &reqwest::Client,
    project_root_id: &str,
    tree_id: &str,
) -> Result<CoverageSummary, String> {
    // 1. Checkpoints = skill branch nodes (branch whose parent is also a branch/phase) —
    //    same definition skill_commands::sync_trees_inner uses.
    let checkpoints = sqlx::query(
        "SELECT n.id, n.title FROM tree_nodes n \
         JOIN tree_nodes parent ON n.parent_id = parent.id \
         WHERE n.tree_id = $1 AND n.type = 'branch' AND parent.type = 'branch'",
    )
    .bind(tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if checkpoints.is_empty() {
        return Ok(CoverageSummary { covered: 0, partial: 0, gap: 0, total: 0 });
    }

    let Some(graph_id) = crate::concept_graph::graph_id_for_project(pool, project_root_id).await?
    else {
        return Ok(CoverageSummary {
            covered: 0,
            partial: 0,
            gap: checkpoints.len() as u32,
            total: checkpoints.len() as u32,
        });
    };

    // 2. Per checkpoint, ANN top-3 evidence symbols from the scanned project graph.
    let mut prompt_items = String::new();
    for c in &checkpoints {
        let cid: String = c.try_get("id").unwrap_or_default();
        let title: String = c.try_get("title").unwrap_or_default();
        let qvec = crate::mimir_ingest::get_embeddings_batch(client, &[title.clone()])
            .await?
            .into_iter()
            .next()
            .ok_or("no query embedding")?;
        let ev = sqlx::query(
            "SELECT id, title, file_path FROM concept_graph_nodes \
             WHERE graph_id = $1 AND embedding IS NOT NULL \
             ORDER BY embedding <=> $2::vector LIMIT 3",
        )
        .bind(&graph_id)
        .bind(crate::mimir_ingest::vector_str(&qvec))
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        let ev_str = ev
            .iter()
            .map(|r| {
                format!(
                    "[{}] {} ({})",
                    r.try_get::<String, _>("id").unwrap_or_default(),
                    r.try_get::<String, _>("title").unwrap_or_default(),
                    r.try_get::<Option<String>, _>("file_path").ok().flatten().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        prompt_items.push_str(&format!(
            "- checkpoint_id={} title=\"{}\" candidates: {}\n",
            cid, title, ev_str
        ));
    }

    // 3. One batched LLM classification. Groq json_mode requires a JSON OBJECT
    //    at the top level, so the model is asked for {"verdicts": [...]}.
    let prompt = format!(
        "For each learning checkpoint, decide if the codebase implements it based on the candidate code symbols.\n\
         Return ONLY a JSON object of this exact shape:\n\
         {{\"verdicts\": [{{\"checkpointId\": \"...\", \"status\": \"covered|partial|gap\", \"evidenceNodeId\": \"... or null\", \"reason\": \"<=12 words\"}}]}}\n\
         covered = clearly implemented; partial = related code exists but incomplete; gap = no evidence.\n\n{}",
        prompt_items
    );

    let api_key = crate::mimir::groq_api_key()?;
    let (raw, latency_ms) = crate::llm_client::call_llm(
        client,
        crate::constants::GROQ_API_URL,
        &api_key,
        "llama-3.3-70b-versatile",
        "You classify whether learning checkpoints are implemented in a codebase, from candidate code symbols. Return ONLY JSON.",
        &prompt,
        2048,
        true, // json_mode
        0.2,
    )
    .await?;
    crate::brain::log_prompt_call(
        pool.clone(),
        "project_coverage",
        "llama-3.3-70b-versatile",
        "coverage_v1",
        latency_ms,
        true,
        None,
        None,
    );

    let json = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let verdicts: Vec<LlmVerdict> = serde_json::from_str::<CoverageResponse>(json)
        .map_err(|e| format!("coverage parse: {} raw: {}", e, crate::text_util::truncate_chars(json, 200)))?
        .verdicts;

    // 4. Upsert + tally.
    let mut sum = CoverageSummary { covered: 0, partial: 0, gap: 0, total: verdicts.len() as u32 };
    let title_by_id: std::collections::HashMap<String, String> = checkpoints
        .iter()
        .map(|c| {
            (
                c.try_get::<String, _>("id").unwrap_or_default(),
                c.try_get::<String, _>("title").unwrap_or_default(),
            )
        })
        .collect();
    for v in &verdicts {
        let status = if v.status == "covered" || v.status == "partial" { v.status.as_str() } else { "gap" };
        match status {
            "covered" => sum.covered += 1,
            "partial" => sum.partial += 1,
            _ => sum.gap += 1,
        }
        sqlx::query(
            "INSERT INTO project_coverage (id, project_root_id, tree_id, checkpoint_node_id, checkpoint_title, status, evidence_node_id, reason) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (project_root_id, checkpoint_node_id) DO UPDATE SET \
               status = EXCLUDED.status, evidence_node_id = EXCLUDED.evidence_node_id, reason = EXCLUDED.reason, created_at = now()",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(project_root_id)
        .bind(tree_id)
        .bind(&v.checkpoint_id)
        .bind(title_by_id.get(&v.checkpoint_id).cloned().unwrap_or_default())
        .bind(status)
        .bind(&v.evidence_node_id)
        .bind(&v.reason)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(sum)
}

/// Manual re-run entry point (frontend button / debug). The automatic path is
/// the inline call at the end of `scan_project_with_events`.
#[tauri::command]
pub async fn run_project_coverage_cmd(
    project_root_id: String,
    tree_id: String,
    client: tauri::State<'_, reqwest::Client>,
    database: tauri::State<'_, crate::database::Database>,
) -> Result<CoverageSummary, String> {
    coverage_for_project(&database.pool, &client, &project_root_id, &tree_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Groq's json_mode requires a top-level JSON OBJECT (Correction B) — the
    /// LLM is prompted for {"verdicts": [...]} with camelCase field names
    /// (checkpointId / evidenceNodeId). This is a pure parse test: no DB, no
    /// network, just proving CoverageResponse/LlmVerdict deserialize that shape.
    #[test]
    fn parses_coverage_response_wrapper_with_camel_case_fields() {
        let json = r#"{"verdicts":[
            {"checkpointId":"c1","status":"covered","evidenceNodeId":"n1","reason":"matches UNet class"},
            {"checkpointId":"c2","status":"gap","evidenceNodeId":null,"reason":"no symbols found"}
        ]}"#;
        let parsed: CoverageResponse = serde_json::from_str(json).expect("should parse camelCase wrapper");
        assert_eq!(parsed.verdicts.len(), 2);
        assert_eq!(parsed.verdicts[0].checkpoint_id, "c1");
        assert_eq!(parsed.verdicts[0].status, "covered");
        assert_eq!(parsed.verdicts[0].evidence_node_id.as_deref(), Some("n1"));
        assert_eq!(parsed.verdicts[0].reason, "matches UNet class");
        assert_eq!(parsed.verdicts[1].checkpoint_id, "c2");
        assert_eq!(parsed.verdicts[1].evidence_node_id, None);
    }
}
