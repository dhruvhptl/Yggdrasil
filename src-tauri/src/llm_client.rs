// src-tauri/src/llm_client.rs

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

/// Token budget for each per-skill checkpoint expansion call (Stage 2).
/// Every call to expand_skill_checkpoints uses this constant — never a local
/// literal — so there is one place to change and no risk of per-skill drift.
pub(crate) const CHECKPOINT_EXPANSION_MAX_TOKENS: u32 = 1500;

/// Accept both `"id": "abc"` and `"id": 9` from LLM output
pub(crate) fn string_or_int<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt { Str(String), Int(i64), Float(f64) }
    match StringOrInt::deserialize(deserializer)? {
        StringOrInt::Str(s) => Ok(s),
        StringOrInt::Int(n) => Ok(n.to_string()),
        StringOrInt::Float(f) => Ok(f.to_string()),
    }
}

pub(crate) fn string_or_int_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    string_or_int(deserializer).or(Ok(String::new()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillTree {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub project_id: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Phase {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    pub skills: Vec<Skill>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    /// Accept both "checkpoints" and legacy "quests" from LLM output
    #[serde(alias = "quests", default)]
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Checkpoint {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    #[serde(alias = "name", default)]
    pub title: String,
    #[serde(default)]
    pub mastery_criteria: String,
    #[serde(default)]
    pub exercises: Vec<String>,
    #[serde(default)]
    pub order: i32,
    /// Legacy fields — tolerated during deserialization but not used
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub estimated_hours: Option<f32>,
    #[serde(default)]
    pub difficulty: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqMessage,
}

#[derive(Debug, Deserialize)]
struct GroqMessage {
    content: String,
}

// ─── JSON cleanup helper ─────────────────────────────────────────────────────

/// Strip markdown fences and leading/trailing whitespace from an LLM response
/// before handing it to serde_json. Also tries to extract the first top-level
/// JSON object if the model still emitted surrounding prose.
pub(crate) fn clean_llm_json(raw: &str) -> String {
    let s = raw.trim();

    // Strip ```json ... ``` or ``` ... ``` fences
    let s = if let Some(inner) = s.strip_prefix("```json") {
        inner.strip_suffix("```").unwrap_or(inner).trim()
    } else if let Some(inner) = s.strip_prefix("```") {
        inner.strip_suffix("```").unwrap_or(inner).trim()
    } else {
        s
    };

    // If there's still surrounding prose, try to extract the first top-level
    // JSON object (scan for the outermost { ... } pair).
    if !s.starts_with('{') {
        if let Some(start) = s.find('{') {
            let candidate = &s[start..];
            // Walk forward to find the matching closing brace.
            // Must be string-aware: { and } inside string literals must not
            // affect depth. Uses the same idiom as repair_truncated_tree_json.
            let mut depth = 0i32;
            let mut end = None;
            let mut in_string = false;
            let mut escape_next = false;
            for (i, ch) in candidate.char_indices() {
                if escape_next { escape_next = false; continue; }
                if ch == '\\' && in_string { escape_next = true; continue; }
                if ch == '"' { in_string = !in_string; continue; }
                if in_string { continue; }
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(e) = end {
                return candidate[..e].trim().to_string();
            }
        }
    }

    s.to_string()
}

// ─── Outline structure repair ────────────────────────────────────────────────

/// Fix the LLM habit of doubling the opening brace after an array bracket:
///   `"phases": [ { { "id": ...`  →  `"phases": [ { "id": ...`
///   `"skills": [ { { "id": ...`  →  `"skills": [ { "id": ...`
///
/// The model emits `[ {` (correct) then immediately emits another `{` before
/// the first key. This removes the spurious second `{` (and a matching extra
/// closing `}`) so the resulting JSON is structurally valid.
///
/// Strategy: scan for the byte sequence `[ { {` (with any whitespace) and
/// collapse the two consecutive `{` tokens into one. We also remove the
/// matching surplus closing `}` by decrementing the brace count on the way
/// back — but that's handled naturally by the `repair_truncated_tree_json`
/// helper if needed; here we only fix the OPEN side, which is what prevents
/// serde_json from parsing at all.
pub(crate) fn repair_duplicate_open_braces(s: &str) -> (String, bool) {
    // We use a simple state machine rather than regex to avoid a dependency.
    // Look for the pattern: '[' → optional-whitespace → '{' → optional-whitespace → '{'
    // and collapse the two '{' into one.
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut repaired = false;

    while i < bytes.len() {
        // Detect '[' followed (through whitespace) by '{' followed (through whitespace) by '{'
        if bytes[i] == b'[' {
            out.push(bytes[i]);
            i += 1;
            // Skip whitespace
            let ws_start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'{' {
                // Emit everything up to and including this first '{'
                out.extend_from_slice(&bytes[ws_start..i]);
                out.push(b'{');
                i += 1;
                // Skip whitespace after first '{'
                let ws2_start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'{' {
                    // This is the spurious duplicate — skip it (and any trailing whitespace)
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    // Also drop one surplus closing '}' from the end to keep balance.
                    // We do this by marking that we need to remove the last bare '}'.
                    repaired = true;
                    // Don't emit ws2 — restart loop with i pointing at first key
                    continue;
                } else {
                    // No duplicate — emit the whitespace we consumed and continue
                    out.extend_from_slice(&bytes[ws2_start..i]);
                    continue;
                }
            } else {
                // Not a '{ {' pattern — emit the whitespace and continue
                out.extend_from_slice(&bytes[ws_start..i]);
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    if !repaired {
        return (s.to_string(), false);
    }

    // We removed N opening braces without removing matching closing braces,
    // so the JSON may now be unbalanced (extra '}'). Remove the last bare '}'
    // for each repair we made. A single scan from the end is sufficient for
    // the common case (1–3 duplicate pairs).
    //
    // Count how many extra '}' we need to drop: we removed exactly one '{' per
    // repair event, so we need to drop one '}' from the end.
    // We already set repaired=true once per '[{ {' occurrence; but the loop
    // above only sets it once (the flag, not a counter). Count properly:
    let removals_needed = {
        // Re-count occurrences in original string
        let orig = s.as_bytes();
        let mut count = 0usize;
        let mut j = 0usize;
        while j < orig.len() {
            if orig[j] == b'[' {
                let mut k = j + 1;
                while k < orig.len() && orig[k].is_ascii_whitespace() { k += 1; }
                if k < orig.len() && orig[k] == b'{' {
                    k += 1;
                    while k < orig.len() && orig[k].is_ascii_whitespace() { k += 1; }
                    if k < orig.len() && orig[k] == b'{' {
                        count += 1;
                    }
                }
            }
            j += 1;
        }
        count
    };

    // Remove `removals_needed` closing '}' from the end of `out`
    let mut removed = 0usize;
    let mut end = out.len();
    while removed < removals_needed && end > 0 {
        end -= 1;
        while end > 0 && out[end].is_ascii_whitespace() { end -= 1; }
        if out[end] == b'}' {
            out.remove(end);
            removed += 1;
            end = out.len();
        } else {
            break; // nothing left to safely remove
        }
    }

    match String::from_utf8(out) {
        Ok(fixed) => (fixed, true),
        Err(_) => (s.to_string(), false), // shouldn't happen — input was valid UTF-8
    }
}

// ─── Tree JSON repair helper ─────────────────────────────────────────────────

/// Attempt minimal repair of a truncated tree JSON string.
/// Handles the common case where the model output is cut off near the end and
/// is only missing the final closing brackets.
/// Returns (repaired_string, was_repaired).
pub(crate) fn repair_truncated_tree_json(s: &str) -> (String, bool) {
    if s.ends_with('}') {
        return (s.to_string(), false);
    }

    // Count brace/bracket imbalance
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    for ch in s.chars() {
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ => {}
        }
    }

    if brace_depth <= 0 && bracket_depth <= 0 {
        // Already balanced — no repair needed
        return (s.to_string(), false);
    }

    // Build closing suffix: close any open string, then close arrays and objects
    let mut suffix = String::new();
    // If we're mid-string (odd number of unescaped quotes), close it
    if in_string {
        suffix.push('"');
    }
    // Close open arrays
    for _ in 0..bracket_depth.max(0) {
        suffix.push(']');
    }
    // Close open objects
    for _ in 0..brace_depth.max(0) {
        suffix.push('}');
    }

    let repaired = format!("{}{}", s.trim_end_matches(',').trim_end(), suffix);
    (repaired, true)
}

// ─── Prompt logging ──────────────────────────────────────────────────────────

/// Fire-and-forget insert into prompt_logs. Spawns its own task so it never
/// blocks the caller. All errors are silently swallowed (debug tool only).
pub(crate) fn log_prompt_call(
    pool: sqlx::PgPool,
    command: &str,
    model: &str,
    prompt_version: &str,
    latency_ms: i64,
    success: bool,
    error: Option<String>,
    metadata: Option<serde_json::Value>,
) {
    let command = command.to_string();
    let model = model.to_string();
    let prompt_version = prompt_version.to_string();
    tokio::spawn(async move {
        let id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO prompt_logs \
               (id, command, model, prompt_version, latency_ms, success, error, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        )
        .bind(&id)
        .bind(&command)
        .bind(&model)
        .bind(&prompt_version)
        .bind(latency_ms as i32)
        .bind(success)
        .bind(error.as_deref())
        .bind(&metadata)
        .execute(&pool)
        .await;
    });
}

// ─── LLM call helper (OpenAI-compatible API) ────────────────────────────────

/// Returns (content, latency_ms).
pub(crate) async fn call_llm(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    json_mode: bool,
    temperature: f64,
) -> Result<(String, i64), String> {
    println!("📡 call_llm: model={} max_tokens={} json_mode={} temp={} url={}", model, max_tokens, json_mode, temperature, base_url);
    let mut request_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": temperature,
        "max_tokens": max_tokens,
    });
    if json_mode {
        request_body["response_format"] = json!({ "type": "json_object" });
    }

    let t0 = std::time::Instant::now();
    let mut attempts = 0u32;
    let max_retries = 3u32;
    let (status, response_text) = loop {
        attempts += 1;
        let resp = client
            .post(base_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let st = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let retryable = (st == 429 || st.is_server_error()) && attempts < max_retries;
        if retryable {
            let wait = attempts * 10;
            println!(
                "⏳ API returned {} — retrying in {}s (attempt {}/{})…",
                st, wait, attempts, max_retries
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait as u64)).await;
            continue;
        }
        break (st, text);
    };
    let latency_ms = t0.elapsed().as_millis() as i64;

    if !status.is_success() {
        return Err(format!("LLM API returned {}: {}", status, response_text));
    }

    let parsed: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse LLM response: {}\nRaw: {}", e, crate::text_util::truncate_chars(&response_text, 500)))?;

    Ok((parsed.choices[0].message.content.clone(), latency_ms))
}
