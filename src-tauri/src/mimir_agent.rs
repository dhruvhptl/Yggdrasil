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

pub(crate) fn tool_schemas() -> Vec<serde_json::Value> {
    vec![
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
                        "confidence": { "type": "number", "description": "0.0-1.0, how certain the fact is. Default 0.7." }
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
    ]
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
            let confidence = args["confidence"].as_f64().unwrap_or(0.7);
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
        other => Err(format!("unknown tool '{}'", other)),
    }
}

// ─── The agent loop ──────────────────────────────────────────────────────────

pub(crate) struct AgentTurnResult {
    pub answer: String,
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
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

    let tools = tool_schemas();
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
                });
            }
            AgentStep::ToolCalls(calls) => {
                messages.push(assistant_msg);
                for call in calls {
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
    fn tool_schemas_declares_all_four_tools() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["function"]["name"].as_str())
            .collect();
        assert_eq!(names, vec!["search_mimir", "get_facts", "set_fact", "read_tree"]);
        for s in &schemas {
            assert_eq!(s["type"], "function");
            assert!(s["function"]["description"].as_str().unwrap_or("").len() > 20);
            assert!(s["function"]["parameters"]["type"] == "object");
        }
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
