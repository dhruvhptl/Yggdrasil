// src-tauri/src/mimir_agent.rs
//
// Model-agnostic tool-calling agent loop for Mimir.
// Speaks the OpenAI chat-completions function-calling format, so any
// provider exposing that API works (Groq, OpenRouter → Gemini/Claude/GPT...).
// Default: Groq LLaMA 3.3-70b. Swap via MIMIR_AGENT_{MODEL,BASE_URL,API_KEY}.

use serde_json::json;

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
}
