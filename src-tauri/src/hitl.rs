// src-tauri/src/hitl.rs
//
// Human-in-the-loop gate for destructive agent tools. requires_approval() is a
// pure classifier: destructive tool call → a proposal the user must approve
// before execute_destructive_action_cmd() performs it. The agent never executes
// these directly; run_agent_turn halts and surfaces the proposal.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use tauri::State;

use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionProposal {
    pub action_type: String,
    pub summary: String,
    pub params: serde_json::Value,
}

/// Pure. Some(proposal) iff `name` is a destructive tool. Summary is built from
/// args alone (no DB). Unknown/read-only tools → None.
pub(crate) fn requires_approval(name: &str, args: &serde_json::Value) -> Option<ActionProposal> {
    match name {
        "delete_fact" => {
            let key = args["fact_key"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "delete_fact".into(),
                summary: format!("delete the fact '{}'", key),
                params: args.clone(),
            })
        }
        "delete_resource" => {
            let title = args["title"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "delete_resource".into(),
                summary: format!("delete the resource '{}'", title),
                params: args.clone(),
            })
        }
        "merge_skills" => {
            let source = args["source"].as_str().unwrap_or("(unspecified)");
            let target = args["target"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "merge_skills".into(),
                summary: format!("merge skill '{}' into '{}'", source, target),
                params: args.clone(),
            })
        }
        _ => None,
    }
}

// ─── Approved execution ──────────────────────────────────────────────────────

/// Execute a previously-approved destructive action. Resolves human-friendly
/// args (keys/titles/names) to rows and performs the action. Idempotent-ish:
/// a target that no longer exists returns a friendly "nothing matched".
#[tauri::command]
pub async fn execute_destructive_action_cmd(
    action_type: String,
    params: serde_json::Value,
    tree_id: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String> {
    match action_type.as_str() {
        "delete_fact" => {
            let fact_key = params["fact_key"].as_str().unwrap_or("").trim().to_string();
            if fact_key.is_empty() {
                return Err("delete_fact requires fact_key".into());
            }
            let entity_id = params["entity_id"].as_str().filter(|s| !s.is_empty());
            // Scope: tree if tree_id present, else user. entity_id optional filter.
            let result = sqlx::query(
                "DELETE FROM mimir_memory_facts \
                 WHERE fact_key = $1 \
                   AND ( (scope = 'tree' AND tree_id = $2) OR (scope = 'user' AND $2 IS NULL) ) \
                   AND ($3::text IS NULL OR entity_id = $3)"
            )
            .bind(&fact_key)
            .bind(&tree_id)
            .bind(entity_id)
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;
            let n = result.rows_affected();
            if n == 0 {
                Ok(format!("No fact matched '{}' — nothing deleted.", fact_key))
            } else {
                Ok(format!("Deleted {} fact(s) for '{}'.", n, fact_key))
            }
        }
        "delete_resource" => {
            let title = params["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() {
                return Err("delete_resource requires title".into());
            }
            let row = sqlx::query("SELECT id FROM mimir_resources WHERE title ILIKE $1 LIMIT 1")
                .bind(&title)
                .fetch_optional(&database.pool)
                .await
                .map_err(|e| e.to_string())?;
            let Some(row) = row else {
                return Ok(format!("No resource titled '{}' — nothing deleted.", title));
            };
            let id: String = row.try_get("id").map_err(|e| e.to_string())?;
            // CASCADE removes chunks/embeddings/node_links (see 003_mimir.sql).
            sqlx::query("DELETE FROM mimir_resources WHERE id = $1")
                .bind(&id)
                .execute(&database.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Deleted resource '{}'.", title))
        }
        "merge_skills" => {
            let source = params["source"].as_str().unwrap_or("").trim().to_string();
            let target = params["target"].as_str().unwrap_or("").trim().to_string();
            if source.is_empty() || target.is_empty() {
                return Err("merge_skills requires source and target".into());
            }
            let source_id = resolve_skill_id(&database.pool, &source).await?;
            let target_id = resolve_skill_id(&database.pool, &target).await?;
            let (Some(source_id), Some(target_id)) = (source_id, target_id) else {
                return Ok(format!(
                    "Could not resolve both skills ('{}' → '{}') — nothing merged.",
                    source, target
                ));
            };
            if source_id == target_id {
                return Ok("Source and target are the same skill — nothing merged.".into());
            }
            crate::skill_commands::merge_skills_inner(&target_id, &[source_id], &database.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Merged skill '{}' into '{}'.", source, target))
        }
        other => Err(format!("unknown destructive action '{}'", other)),
    }
}

/// Resolve a skill name (or alias) to a universal_skills id, case-insensitive.
async fn resolve_skill_id(pool: &sqlx::PgPool, name: &str) -> Result<Option<String>, String> {
    if let Some(row) = sqlx::query("SELECT id FROM universal_skills WHERE LOWER(name) = LOWER($1) LIMIT 1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(row.try_get("id").ok());
    }
    let row = sqlx::query(
        "SELECT canonical_skill_id AS id FROM skill_aliases WHERE LOWER(alias) = LOWER($1) LIMIT 1"
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.try_get::<String, _>("id").ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_tools_yield_proposals() {
        let p = requires_approval("delete_fact", &json!({ "fact_key": "career_goal" })).unwrap();
        assert_eq!(p.action_type, "delete_fact");
        assert_eq!(p.summary, "delete the fact 'career_goal'");

        let r = requires_approval("delete_resource", &json!({ "title": "RRF paper" })).unwrap();
        assert_eq!(r.action_type, "delete_resource");
        assert!(r.summary.contains("RRF paper"));

        let m = requires_approval("merge_skills", &json!({ "source": "SQL", "target": "Databases" })).unwrap();
        assert_eq!(m.action_type, "merge_skills");
        assert_eq!(m.summary, "merge skill 'SQL' into 'Databases'");
    }

    #[test]
    fn readonly_tools_yield_none() {
        for n in ["search_mimir", "get_facts", "set_fact", "read_tree", "query_graph",
                  "path_between", "explain_node", "smart_search", "smart_fetch"] {
            assert!(requires_approval(n, &json!({})).is_none(), "{} should not require approval", n);
        }
    }

    #[test]
    fn missing_args_still_proposes_safely() {
        let p = requires_approval("delete_fact", &json!({})).unwrap();
        assert!(p.summary.contains("(unspecified)"));
    }
}
