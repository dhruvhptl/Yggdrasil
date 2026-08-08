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

/// One reconciled row, ready to upsert into `project_coverage`.
#[derive(Debug, Clone, PartialEq)]
struct CoverageRow {
    checkpoint_node_id: String,
    checkpoint_title: String,
    status: &'static str,
    evidence_node_id: Option<String>,
    reason: String,
}

fn normalize_status(s: &str) -> &'static str {
    match s {
        "covered" => "covered",
        "partial" => "partial",
        _ => "gap",
    }
}

/// Pure reconciliation: `checkpoints` (id, title) is the source of truth, never
/// `verdicts`. Verdicts whose `checkpoint_id` doesn't match a real checkpoint
/// (hallucinated) are dropped. Checkpoints with no matching verdict (the LLM
/// omitted them) get a synthesized default `gap` row. Guarantees exactly one
/// row per checkpoint, `total == checkpoints.len()`, and
/// `covered + partial + gap == total` — no silently shrinking counts, no
/// garbage rows from invented ids.
fn reconcile(checkpoints: &[(String, String)], verdicts: Vec<LlmVerdict>) -> (Vec<CoverageRow>, CoverageSummary) {
    let known_ids: std::collections::HashSet<&str> = checkpoints.iter().map(|(id, _)| id.as_str()).collect();
    let mut verdict_by_id: std::collections::HashMap<String, LlmVerdict> = verdicts
        .into_iter()
        .filter(|v| known_ids.contains(v.checkpoint_id.as_str()))
        .map(|v| (v.checkpoint_id.clone(), v))
        .collect();

    let mut sum = CoverageSummary { covered: 0, partial: 0, gap: 0, total: checkpoints.len() as u32 };
    let mut rows = Vec::with_capacity(checkpoints.len());
    for (id, title) in checkpoints {
        let (status, evidence_node_id, reason) = match verdict_by_id.remove(id) {
            Some(v) => (normalize_status(&v.status), v.evidence_node_id, v.reason),
            None => ("gap", None, "not classified this run".to_string()),
        };
        match status {
            "covered" => sum.covered += 1,
            "partial" => sum.partial += 1,
            _ => sum.gap += 1,
        }
        rows.push(CoverageRow {
            checkpoint_node_id: id.clone(),
            checkpoint_title: title.clone(),
            status,
            evidence_node_id,
            reason,
        });
    }
    (rows, sum)
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

    // Source of truth for reconciliation (Step 4) — built once, up front, so a
    // checkpoint the LLM drops or hallucinates can never silently skew the count.
    let checkpoint_pairs: Vec<(String, String)> = checkpoints
        .iter()
        .map(|c| {
            (
                c.try_get::<String, _>("id").unwrap_or_default(),
                c.try_get::<String, _>("title").unwrap_or_default(),
            )
        })
        .collect();

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

    // 4. Reconcile against checkpoints (source of truth) — drops hallucinated
    //    ids, synthesizes a `gap` row for any checkpoint the LLM omitted.
    //    Guarantees one row per checkpoint and covered+partial+gap == total.
    let (rows, sum) = reconcile(&checkpoint_pairs, verdicts);

    for row in &rows {
        sqlx::query(
            "INSERT INTO project_coverage (id, project_root_id, tree_id, checkpoint_node_id, checkpoint_title, status, evidence_node_id, reason) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (project_root_id, checkpoint_node_id) DO UPDATE SET \
               status = EXCLUDED.status, evidence_node_id = EXCLUDED.evidence_node_id, reason = EXCLUDED.reason, created_at = now()",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(project_root_id)
        .bind(tree_id)
        .bind(&row.checkpoint_node_id)
        .bind(&row.checkpoint_title)
        .bind(row.status)
        .bind(&row.evidence_node_id)
        .bind(&row.reason)
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

    fn verdict(checkpoint_id: &str, status: &str, evidence_node_id: Option<&str>, reason: &str) -> LlmVerdict {
        LlmVerdict {
            checkpoint_id: checkpoint_id.to_string(),
            status: status.to_string(),
            evidence_node_id: evidence_node_id.map(|s| s.to_string()),
            reason: reason.to_string(),
        }
    }

    /// checkpoints (never verdicts) are the source of truth: a checkpoint the
    /// LLM drops still gets a row (synthesized gap, "not classified this run");
    /// a hallucinated checkpoint_id the LLM invents is dropped entirely; the
    /// tally always reconciles to exactly one row per real checkpoint.
    #[test]
    fn reconcile_uses_checkpoints_as_source_of_truth() {
        let checkpoints = vec![
            ("c1".to_string(), "Skill One".to_string()),
            ("c2".to_string(), "Skill Two".to_string()),
            ("c3".to_string(), "Skill Three".to_string()),
        ];
        let verdicts = vec![
            verdict("c1", "covered", Some("n1"), "found it"),
            // c2 intentionally missing — the LLM dropped it from its response
            verdict("c3", "partial", None, "half done"),
            verdict("ghost", "covered", Some("nX"), "hallucinated checkpoint id"),
        ];

        let (rows, summary) = reconcile(&checkpoints, verdicts);

        // One row per real checkpoint — the hallucinated "ghost" id never appears.
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.checkpoint_node_id != "ghost"));

        // Missing checkpoint (c2) synthesizes a default gap row.
        let c2 = rows.iter().find(|r| r.checkpoint_node_id == "c2").expect("c2 row present");
        assert_eq!(c2.status, "gap");
        assert_eq!(c2.reason, "not classified this run");
        assert_eq!(c2.evidence_node_id, None);
        assert_eq!(c2.checkpoint_title, "Skill Two");

        // Known verdicts still map through correctly.
        let c1 = rows.iter().find(|r| r.checkpoint_node_id == "c1").expect("c1 row present");
        assert_eq!(c1.status, "covered");
        assert_eq!(c1.evidence_node_id.as_deref(), Some("n1"));

        // Counts always reconcile: total == checkpoints.len(), and the three
        // buckets sum back to total (no silent shrink, no orphaned tallies).
        assert_eq!(summary.total, 3);
        assert_eq!(summary.covered + summary.partial + summary.gap, summary.total);
        assert_eq!(summary.covered, 1);
        assert_eq!(summary.partial, 1);
        assert_eq!(summary.gap, 1);
    }
}
