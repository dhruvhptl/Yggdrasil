// src-tauri/src/mimir_agent.rs
//
// Model-agnostic tool-calling agent loop for Mimir.
// Speaks the OpenAI chat-completions function-calling format, so any
// provider exposing that API works (Groq, OpenRouter → Gemini/Claude/GPT...).
// Default: Groq LLaMA 3.3-70b. Swap via MIMIR_AGENT_{MODEL,BASE_URL,API_KEY}.

use serde_json::json;
use sqlx::Row;

// ─── Provider config ─────────────────────────────────────────────────────────

pub(crate) struct AgentModelConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl AgentModelConfig {
    /// Resolve from env with Groq defaults. Model-agnostic: point
    /// MIMIR_AGENT_BASE_URL at any OpenAI-compatible endpoint.
    pub(crate) fn from_env() -> Result<Self, String> {
        let model = std::env::var("MIMIR_AGENT_MODEL")
            .unwrap_or_else(|_| "llama-3.3-70b-versatile".to_string());
        let base_url = std::env::var("MIMIR_AGENT_BASE_URL")
            .unwrap_or_else(|_| crate::constants::GROQ_API_URL.to_string());
        let api_key = match std::env::var("MIMIR_AGENT_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => crate::mimir::groq_api_key()?,
        };
        Ok(Self { base_url, api_key, model })
    }
}

// ─── Tool schemas (OpenAI function-calling format) ───────────────────────────

pub(crate) fn tool_schemas(hound_available: bool) -> Vec<serde_json::Value> {
    let mut schemas = vec![
        json!({
            "type": "function",
            "function": {
                "name": "search_mimir",
                "description": "Search the user's personal resource library using hybrid vector + keyword retrieval with reranking. Returns relevant passages with source citations. Call this before answering any substantive knowledge question.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query — the user's question or a sharper reformulation of it."
                        }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_facts",
                "description": "Recall long-term memory about this user: what they have mastered, studied, their preferences and goals. Optionally filter by fact key.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "fact_key": {
                            "type": "string",
                            "description": "Optional: only return facts with this key (e.g. 'mastered_concept')."
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "set_fact",
                "description": "Record a durable fact about the user in long-term memory — a stated preference, goal, background, or misconception. Use snake_case keys like 'prefers_format' or 'career_goal'. Do NOT record trivia about the subject matter.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "fact_key": { "type": "string", "description": "snake_case key, e.g. 'career_goal'" },
                        "fact_value": { "type": "string", "description": "Short value, e.g. 'transition into ML engineering'" },
                        "entity_id": { "type": "string", "description": "Optional: id of the node/resource this fact is about, when it concerns a specific one." },
                        "confidence": { "type": ["number", "string"], "description": "0.0-1.0, how certain the fact is. Default 0.7." }
                    },
                    "required": ["fact_key", "fact_value"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_tree",
                "description": "Read the user's current learning tree: phases, skills, checkpoints, progress and locked state. Use to ground advice about what to learn next.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "query_graph",
                "description": "Search the concept graph for concepts matching a topic. Returns matching concepts and the edges between them. Use to see how ideas in this project relate.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Topic to search for, e.g. 'sharding', 'RRF', 'authentication'" }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "path_between",
                "description": "Find the shortest prerequisite path from one concept to another. Returns the ordered chain of concepts to learn from source to target.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "description": "Starting concept title (more foundational)" },
                        "target": { "type": "string", "description": "Target concept title (more advanced)" }
                    },
                    "required": ["source", "target"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "explain_node",
                "description": "Get full detail on one concept: its prerequisites, the concepts that depend on it, references, and linked learning resources from the user's library.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Concept title to explain" }
                    },
                    "required": ["title"]
                }
            }
        }),
    ];
    if hound_available {
        schemas.push(json!({
            "type": "function",
            "function": {
                "name": "smart_search",
                "description": "Search the live web. Returns relevant results with snippets and source citations. Call this BEFORE smart_fetch to find URLs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query — be specific." },
                        "count": { "type": ["integer", "string"], "description": "Number of results (max 10, default 6)." }
                    },
                    "required": ["query"]
                }
            }
        }));
        schemas.push(json!({
            "type": "function",
            "function": {
                "name": "smart_fetch",
                "description": "Fetch a URL and return its content as readable text. Handles articles, documentation, PDFs, and most websites. Use URLs returned by smart_search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The full URL to fetch." },
                        "focus": { "type": "string", "description": "Optional: a short phrase describing what part of the page matters." }
                    },
                    "required": ["url"]
                }
            }
        }));
    }
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "delete_fact",
            "description": "Delete a stored memory fact. REQUIRES user approval before it executes — the user will be shown a confirmation.",
            "parameters": {
                "type": "object",
                "properties": {
                    "fact_key": { "type": "string", "description": "The fact key to delete, e.g. 'career_goal'." },
                    "entity_id": { "type": "string", "description": "Optional: the entity id if the fact is entity-scoped (e.g. a node/resource id)." }
                },
                "required": ["fact_key"]
            }
        }
    }));
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "delete_resource",
            "description": "Delete a resource from the user's library. REQUIRES user approval before it executes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "The title of the resource to delete." }
                },
                "required": ["title"]
            }
        }
    }));
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "merge_skills",
            "description": "Merge one skill into another (the source is absorbed into the target). REQUIRES user approval before it executes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "The skill to absorb (by name)." },
                    "target": { "type": "string", "description": "The skill to keep (by name)." }
                },
                "required": ["source", "target"]
            }
        }
    }));
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "scan_project",
            "description": "Parse a local project directory and extract concept nodes and edges from source code into the concept graph (confidence='extracted'). Supports Python, JavaScript, TypeScript, Rust, Go.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute or relative path to the project directory." }
                },
                "required": ["path"]
            }
        }
    }));
    schemas
}

// ─── Response parsing ────────────────────────────────────────────────────────

pub(crate) struct ToolCallReq {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

pub(crate) enum AgentStep {
    Answer(String),
    ToolCalls(Vec<ToolCallReq>),
}

/// Parse one chat-completions response body into either a final answer or a
/// list of requested tool calls. Also returns the raw assistant message (to
/// append back into the conversation before tool results).
pub(crate) fn parse_agent_response(
    body: &serde_json::Value,
) -> Result<(AgentStep, serde_json::Value), String> {
    let msg = &body["choices"][0]["message"];
    if msg.is_null() {
        return Err(format!(
            "no choices in agent response: {}",
            serde_json::to_string(body).unwrap_or_default().chars().take(300).collect::<String>()
        ));
    }
    let tool_calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
    if !tool_calls.is_empty() {
        let mut reqs = Vec::new();
        for tc in &tool_calls {
            let id = tc["id"].as_str().unwrap_or_default().to_string();
            let name = tc["function"]["name"].as_str().unwrap_or_default().to_string();
            let raw_args = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let arguments: serde_json::Value =
                serde_json::from_str(raw_args).unwrap_or_else(|_| json!({}));
            if !name.is_empty() {
                reqs.push(ToolCallReq { id, name, arguments });
            }
        }
        return Ok((AgentStep::ToolCalls(reqs), msg.clone()));
    }
    let content = msg["content"].as_str().unwrap_or("").to_string();
    Ok((AgentStep::Answer(content), msg.clone()))
}

// ─── LLM call with tools ─────────────────────────────────────────────────────

pub(crate) async fn call_agent_llm(
    client: &reqwest::Client,
    cfg: &AgentModelConfig,
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    force_answer: bool,
) -> Result<(AgentStep, serde_json::Value), String> {
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.4,
        "max_tokens": 2048,
    });
    if !force_answer {
        // When forcing a final answer we omit BOTH "tools" and "tool_choice":
        // a plain chat request is universally accepted, whereas tool_choice
        // "none" without tools (or with tools) errors on some providers.
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }

    let resp = client
        .post(&cfg.base_url)
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("agent LLM request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("agent LLM read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("agent LLM returned {}: {}", status, text.chars().take(400).collect::<String>()));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("agent LLM parse failed: {}", e))?;
    parse_agent_response(&parsed)
}

// ─── Tool execution ──────────────────────────────────────────────────────────

pub(crate) struct ToolCtx<'a> {
    pub pool: &'a sqlx::PgPool,
    pub client: &'a reqwest::Client,
    pub groq_api_key: &'a str,
    pub tree_id: Option<String>,
    pub node_id: Option<String>,
    pub node_title: Option<String>,
    pub node_description: Option<String>,
    pub message: String,
    pub hound_base_url: Option<String>,
}

#[derive(Default)]
pub(crate) struct AgentTurnState {
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
}

/// Drop duplicate sources across multiple search_mimir calls in one turn.
/// Key: (title, section_title, page_start). First occurrence wins (pre-matched
/// chunks arrive first with score 1.0).
pub(crate) fn dedup_sources(
    sources: Vec<crate::mimir::MimirChatSource>,
) -> Vec<crate::mimir::MimirChatSource> {
    let mut seen: std::collections::HashSet<(String, String, i32)> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(sources.len());
    for s in sources {
        let key = (
            s.title.clone(),
            s.section_title.clone().unwrap_or_default(),
            s.page_start.unwrap_or(-1),
        );
        if seen.insert(key) {
            out.push(s);
        }
    }
    out
}

/// Execute one tool call. Errors are returned as Err — the loop converts them
/// into observations so the model can recover.
pub(crate) async fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolCtx<'_>,
    state: &mut AgentTurnState,
) -> Result<String, String> {
    match name {
        "search_mimir" => {
            let query = args["query"].as_str().unwrap_or(&ctx.message).to_string();
            let r = crate::mimir_retrieval::run_retrieval(
                ctx.pool,
                ctx.client,
                ctx.groq_api_key,
                &query,
                ctx.node_id.as_deref(),
                ctx.node_title.as_deref(),
                ctx.node_description.as_deref(),
            )
            .await?;
            state.stats.merge(&r.stats);
            state.sources.extend(r.sources);
            if r.context_blocks.trim().is_empty() {
                Ok("No relevant passages found in the library for this query. Answer from your own knowledge.".to_string())
            } else {
                // Cap the observation so one search can't blow the context window.
                Ok(r.context_blocks.chars().take(8000).collect())
            }
        }
        "get_facts" => {
            let mem = crate::mimir_memory::get_memory_context(ctx.pool, ctx.tree_id.as_deref()).await?;
            let filter = args["fact_key"].as_str();
            let mut items: Vec<serde_json::Value> = Vec::new();
            for f in mem.facts.iter().chain(mem.user_facts.iter()) {
                if let Some(k) = filter {
                    if f.fact_key != k { continue; }
                }
                items.push(json!({
                    "factKey": f.fact_key,
                    "value": f.fact_value,
                    "confidence": f.confidence,
                    "source": f.source,
                    "entityId": f.entity_id,
                }));
                if items.len() >= 30 { break; }
            }
            if items.is_empty() {
                Ok("No stored facts yet.".to_string())
            } else {
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
        }
        "set_fact" => {
            let raw_key = args["fact_key"].as_str().unwrap_or("").trim().to_lowercase();
            let fact_key = raw_key.replace(' ', "_");
            if fact_key.is_empty() {
                return Err("set_fact requires a non-empty fact_key".to_string());
            }
            let fact_value = if args["fact_value"].is_null() {
                return Err("set_fact requires fact_value".to_string());
            } else {
                args["fact_value"].clone()
            };
            let fact_value = match fact_value {
                serde_json::Value::String(s) if s.chars().count() > 500 => {
                    serde_json::Value::String(s.chars().take(500).collect())
                }
                v => v,
            };
            // Groq LLaMA sometimes sends a stringified number ("0.0"); accept either.
            let confidence = args["confidence"]
                .as_f64()
                .or_else(|| args["confidence"].as_str().and_then(|s| s.trim().parse::<f64>().ok()))
                .unwrap_or(0.7);
            let entity_id = args["entity_id"].as_str().filter(|s| !s.is_empty());
            let user_key = crate::mimir_memory::is_user_scope_key(&fact_key);
            let scope = if user_key { "user" } else if ctx.tree_id.is_some() { "tree" } else { "user" };
            let scoped_tree_id = if user_key { None } else { ctx.tree_id.as_deref() };
            let id = crate::mimir_memory::set_memory_fact(
                ctx.pool,
                scope,
                scoped_tree_id,
                None,
                &fact_key,
                entity_id,
                fact_value,
                confidence,
                "agent_extraction",
            )
            .await?;
            Ok(format!("Saved fact '{}' (id {}).", fact_key, id))
        }
        "read_tree" => {
            let Some(tid) = ctx.tree_id.as_deref() else {
                return Ok("No learning tree is active in this conversation.".to_string());
            };
            let rows = sqlx::query(
                "SELECT ph.title AS phase, br.title AS skill, lf.title AS checkpoint, \
                        COALESCE(lf.progress, 0)::int AS progress, COALESCE(lf.is_locked, false) AS is_locked \
                 FROM tree_nodes ph \
                 LEFT JOIN tree_nodes br ON br.parent_id = ph.id AND br.tree_id = ph.tree_id AND br.type = 'branch' \
                 LEFT JOIN tree_nodes lf ON lf.parent_id = br.id AND lf.tree_id = ph.tree_id AND lf.type = 'leaf' \
                 WHERE ph.tree_id = $1 AND ph.type = 'trunk' \
                 ORDER BY ph.order_index ASC, br.order_index ASC, lf.order_index ASC"
            )
            .bind(tid)
            .fetch_all(ctx.pool)
            .await
            .map_err(|e| e.to_string())?;

            if rows.is_empty() {
                return Ok("The tree has no phases yet.".to_string());
            }
            let mut out = String::new();
            let mut last_phase = String::new();
            let mut last_skill = String::new();
            for row in &rows {
                let phase: String = row.try_get("phase").unwrap_or_default();
                let skill: Option<String> = row.try_get("skill").ok().flatten();
                let checkpoint: Option<String> = row.try_get("checkpoint").ok().flatten();
                let progress: i32 = row.try_get("progress").unwrap_or(0);
                let is_locked: bool = row.try_get("is_locked").unwrap_or(false);
                if phase != last_phase {
                    out.push_str(&format!("Phase: {}\n", phase));
                    last_phase = phase;
                    last_skill.clear();
                }
                if let Some(s) = skill {
                    if s != last_skill {
                        out.push_str(&format!("  Skill: {}\n", s));
                        last_skill = s;
                    }
                }
                if let Some(c) = checkpoint {
                    let status = if progress >= 100 { "done" } else if is_locked { "locked" } else { "open" };
                    out.push_str(&format!("    [{}] {} ({}%)\n", status, c, progress));
                }
                if out.len() > 6000 {
                    out.push_str("... (truncated)\n");
                    break;
                }
            }
            Ok(out)
        }
        "query_graph" => {
            let Some(tid) = ctx.tree_id.as_deref() else {
                return Ok("No learning tree is active — the concept graph is per-tree.".to_string());
            };
            let query = args["query"].as_str().unwrap_or(&ctx.message).to_string();
            let sub = crate::concept_graph::query_graph(ctx.pool, tid, &query).await?;
            if sub.nodes.is_empty() {
                return Ok(format!("No concepts in the graph match '{}'.", query));
            }
            let mut out = String::from("Concepts:\n");
            for n in &sub.nodes {
                out.push_str(&format!("- {}: {}\n", n.title, n.description));
            }
            if !sub.edges.is_empty() {
                // Build id->title for readable edges.
                let title_of: std::collections::HashMap<&str, &str> =
                    sub.nodes.iter().map(|n| (n.id.as_str(), n.title.as_str())).collect();
                out.push_str("Relationships:\n");
                for e in &sub.edges {
                    let s = title_of.get(e.source_node_id.as_str()).copied().unwrap_or("?");
                    let t = title_of.get(e.target_node_id.as_str()).copied().unwrap_or("?");
                    out.push_str(&format!("- {} --{}--> {}\n", s, e.relationship, t));
                }
            }
            Ok(out.chars().take(6000).collect())
        }
        "path_between" => {
            let Some(tid) = ctx.tree_id.as_deref() else {
                return Ok("No learning tree is active — the concept graph is per-tree.".to_string());
            };
            let source = args["source"].as_str().unwrap_or("").to_string();
            let target = args["target"].as_str().unwrap_or("").to_string();
            if source.is_empty() || target.is_empty() {
                return Err("path_between requires 'source' and 'target'".to_string());
            }
            let path = crate::concept_graph::path_between(ctx.pool, tid, &source, &target).await?;
            if path.is_empty() {
                return Ok(format!("No prerequisite path found from '{}' to '{}'.", source, target));
            }
            let chain: Vec<String> = path.iter().map(|p| p.title.clone()).collect();
            Ok(format!("Prerequisite path: {}", chain.join(" → ")))
        }
        "explain_node" => {
            let Some(tid) = ctx.tree_id.as_deref() else {
                return Ok("No learning tree is active — the concept graph is per-tree.".to_string());
            };
            let title = args["title"].as_str().unwrap_or("").to_string();
            if title.is_empty() {
                return Err("explain_node requires 'title'".to_string());
            }
            let Some(node_id) = crate::concept_graph::resolve_node_by_title(ctx.pool, tid, &title).await? else {
                return Ok(format!("No concept titled '{}' in this tree's graph.", title));
            };
            let d = crate::concept_graph::explain_node(ctx.pool, &node_id).await?;
            let mut out = format!("{}: {}\n", d.node.title, d.node.description);
            if !d.prerequisites.is_empty() {
                let names: Vec<String> = d.prerequisites.iter().map(|e| e.title.clone()).collect();
                out.push_str(&format!("Prerequisites: {}\n", names.join(", ")));
            }
            if !d.dependents.is_empty() {
                let names: Vec<String> = d.dependents.iter().map(|e| e.title.clone()).collect();
                out.push_str(&format!("Leads to: {}\n", names.join(", ")));
            }
            if !d.resources.is_empty() {
                let names: Vec<String> = d.resources.iter().map(|r| r.title.clone()).collect();
                out.push_str(&format!("Your resources: {}\n", names.join(", ")));
            }
            Ok(out.chars().take(6000).collect())
        }
        "smart_search" => {
            let Some(base_url) = ctx.hound_base_url.as_deref() else {
                return Ok("Web search is not available (Hound is not running).".to_string());
            };
            let query = args["query"].as_str().unwrap_or(&ctx.message).to_string();
            let count = args["count"]
                .as_u64()
                .or_else(|| args["count"].as_str().and_then(|s| s.trim().parse::<u64>().ok()))
                .map(|n| n as u32);
            let results = crate::hound_client::smart_search(ctx.client, base_url, &query, count).await?;
            if results.is_empty() {
                return Ok(format!("No web results found for '{}'.", query));
            }
            let mut out = String::from("Web results:\n");
            for r in results.iter().take(10) {
                out.push_str(&format!("- {} — {}\n  {}\n  (source: {})\n", r.title, r.url, r.snippet, r.source));
            }
            Ok(out.chars().take(6000).collect())
        }
        "smart_fetch" => {
            let Some(base_url) = ctx.hound_base_url.as_deref() else {
                return Ok("Web fetch is not available (Hound is not running).".to_string());
            };
            let Some(url) = args["url"].as_str() else {
                return Err("smart_fetch requires 'url'".to_string());
            };
            let focus = args["focus"].as_str();
            let fetched = crate::hound_client::smart_fetch(ctx.client, base_url, url, focus).await?;
            if !fetched.content_ok {
                return Ok(format!("Couldn't access that page ({}). {}", url, fetched.content.chars().take(200).collect::<String>()));
            }
            let header = format!("{} ({})\n", fetched.title, fetched.url);
            let body: String = fetched.content.chars().take(8000).collect();
            Ok(format!("{}{}", header, body))
        }
        "delete_fact" | "delete_resource" | "merge_skills" => {
            Err(format!("{} requires user approval and cannot execute directly", name))
        }
        "scan_project" => {
            let path = args["path"].as_str().ok_or("scan_project requires a 'path' argument")?;
            let Some(tree_id) = ctx.tree_id.as_deref() else {
                return Ok("No active tree to scan into — open a tree first.".to_string());
            };
            let r = crate::project_scanner::scan_project(ctx.pool, path, tree_id).await?;
            let mut msg = format!(
                "Scanned {} files ({} skipped). Added {} new concepts, enriched {} existing, added {} edges.",
                r.files_scanned, r.files_skipped, r.nodes_added, r.nodes_enriched, r.edges_added
            );
            if !r.errors.is_empty() {
                msg.push_str(&format!(" {} file(s) had errors and were skipped.", r.errors.len()));
            }
            Ok(msg)
        }
        other => Err(format!("unknown tool '{}'", other)),
    }
}

// ─── The agent loop ──────────────────────────────────────────────────────────

pub(crate) struct AgentTurnResult {
    pub answer: String,
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
    pub pending_approval: Option<crate::hitl::ActionProposal>,
}

const MAX_TOOL_CALLS: u32 = 6;
const MAX_ITERATIONS: u32 = 8;

/// Think → act → observe loop. Tool errors become observations; hard failures
/// return Err so the caller can fall back to classic synthesis.
pub(crate) async fn run_agent_turn(
    cfg: &AgentModelConfig,
    ctx: &ToolCtx<'_>,
    system_prompt: &str,
    history: &[serde_json::Value],
    user_message: &str,
    pool_for_log: sqlx::PgPool,
) -> Result<AgentTurnResult, String> {
    let mut messages: Vec<serde_json::Value> =
        vec![json!({ "role": "system", "content": system_prompt })];
    messages.extend_from_slice(history);
    messages.push(json!({ "role": "user", "content": user_message }));

    let tools = tool_schemas(ctx.hound_base_url.is_some());
    let mut state = AgentTurnState::default();
    let t0 = std::time::Instant::now();

    for _iteration in 0..MAX_ITERATIONS {
        let force_answer = state.tool_calls_made >= MAX_TOOL_CALLS;
        let (step, assistant_msg) =
            call_agent_llm(ctx.client, cfg, &messages, &tools, force_answer).await?;

        match step {
            AgentStep::Answer(content) => {
                if content.trim().is_empty() {
                    return Err("agent returned an empty answer".to_string());
                }
                crate::brain::log_prompt_call(
                    pool_for_log,
                    "mimir_agent_turn",
                    &cfg.model,
                    "mimir_agent_v1",
                    t0.elapsed().as_millis() as i64,
                    true,
                    None,
                    Some(json!({ "tool_calls_made": state.tool_calls_made })),
                );
                return Ok(AgentTurnResult {
                    answer: content,
                    sources: dedup_sources(state.sources),
                    stats: state.stats,
                    tool_calls_made: state.tool_calls_made,
                    pending_approval: None,
                });
            }
            AgentStep::ToolCalls(calls) => {
                messages.push(assistant_msg);
                for call in calls {
                    if let Some(proposal) = crate::hitl::requires_approval(&call.name, &call.arguments) {
                        crate::brain::log_prompt_call(
                            pool_for_log.clone(), "mimir_hitl_proposal", &cfg.model, "mimir_hitl_v1",
                            t0.elapsed().as_millis() as i64, true, None,
                            Some(json!({ "action_type": proposal.action_type })),
                        );
                        return Ok(AgentTurnResult {
                            answer: format!("I'd like to {}. Approve?", proposal.summary),
                            sources: dedup_sources(state.sources),
                            stats: state.stats,
                            tool_calls_made: state.tool_calls_made,
                            pending_approval: Some(proposal),
                        });
                    }
                    if state.tool_calls_made >= MAX_TOOL_CALLS {
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": "Tool budget exhausted — answer now with what you already have."
                        }));
                        continue;
                    }
                    state.tool_calls_made += 1;
                    println!("🛠  [agent] tool call {}/{}: {}", state.tool_calls_made, MAX_TOOL_CALLS, call.name);
                    let observation = match execute_tool(&call.name, &call.arguments, ctx, &mut state).await {
                        Ok(o) => o,
                        Err(e) => format!("Tool error: {}", e),
                    };
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": observation
                    }));
                }
            }
        }
    }
    crate::brain::log_prompt_call(
        pool_for_log,
        "mimir_agent_turn",
        &cfg.model,
        "mimir_agent_v1",
        t0.elapsed().as_millis() as i64,
        false,
        Some("max iterations without answer".to_string()),
        None,
    );
    Err("agent loop exceeded max iterations without an answer".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_registers_web_tools_only_when_available() {
        let base: Vec<String> = tool_schemas(false)
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert_eq!(
            base,
            vec!["search_mimir", "get_facts", "set_fact", "read_tree",
                 "query_graph", "path_between", "explain_node",
                 "delete_fact", "delete_resource", "merge_skills", "scan_project"]
        );

        let with_web: Vec<String> = tool_schemas(true)
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert_eq!(with_web.len(), 13);
        assert_eq!(with_web[7], "smart_search");
        assert_eq!(with_web[8], "smart_fetch");
        assert_eq!(with_web[9], "delete_fact");
        assert_eq!(with_web[10], "delete_resource");
        assert_eq!(with_web[11], "merge_skills");
        assert_eq!(with_web[12], "scan_project");
    }

    #[test]
    fn parse_agent_response_extracts_tool_calls() {
        let body = json!({
            "choices": [{ "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "search_mimir", "arguments": "{\"query\": \"what is RRF\"}" }
                }]
            }}]
        });
        let (step, raw_msg) = parse_agent_response(&body).unwrap();
        match step {
            AgentStep::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "search_mimir");
                assert_eq!(calls[0].arguments["query"], "what is RRF");
            }
            _ => panic!("expected ToolCalls"),
        }
        assert!(raw_msg["tool_calls"].is_array());
    }

    #[test]
    fn parse_agent_response_extracts_final_answer() {
        let body = json!({
            "choices": [{ "message": { "role": "assistant", "content": "RRF merges ranked lists." } }]
        });
        let (step, _) = parse_agent_response(&body).unwrap();
        match step {
            AgentStep::Answer(a) => assert_eq!(a, "RRF merges ranked lists."),
            _ => panic!("expected Answer"),
        }
    }

    #[test]
    fn parse_agent_response_errors_on_empty_body() {
        assert!(parse_agent_response(&json!({})).is_err());
    }

    #[test]
    fn malformed_tool_arguments_fall_back_to_empty_object() {
        let body = json!({
            "choices": [{ "message": {
                "tool_calls": [{
                    "id": "c2", "type": "function",
                    "function": { "name": "read_tree", "arguments": "not json {" }
                }]
            }}]
        });
        let (step, _) = parse_agent_response(&body).unwrap();
        match step {
            AgentStep::ToolCalls(calls) => assert_eq!(calls[0].arguments, json!({})),
            _ => panic!("expected ToolCalls"),
        }
    }

    fn src(title: &str, section: Option<&str>, page: Option<i32>, score: f32) -> crate::mimir::MimirChatSource {
        crate::mimir::MimirChatSource {
            title: title.to_string(),
            url: None,
            chunk: String::new(),
            score,
            section_title: section.map(|s| s.to_string()),
            page_start: page,
            page_end: page,
        }
    }

    #[test]
    fn dedup_sources_keeps_first_occurrence() {
        let sources = vec![
            src("Paper A", Some("Intro"), Some(1), 1.0),
            src("Paper A", Some("Intro"), Some(1), 0.6),  // duplicate — dropped
            src("Paper A", Some("Methods"), Some(4), 0.8), // different section — kept
            src("Blog B", None, None, 0.7),
        ];
        let out = dedup_sources(sources);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].score, 1.0); // first occurrence wins
    }
}
