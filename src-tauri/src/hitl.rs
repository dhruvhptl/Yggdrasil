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
