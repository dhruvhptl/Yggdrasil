# Mimir Memory + Minimal Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Mimir persistent memory (durable facts + rolling session summaries) and turn its chat into a model-agnostic tool-calling agent with four tools (`search_mimir`, `get_facts`, `set_fact`, `read_tree`), with a fallback to the existing one-shot synthesis.

**Architecture:** A new `mimir_memory.rs` module owns the memory substrate (migration 049). A new `mimir_agent.rs` module owns a provider-agnostic think→act→observe loop speaking the OpenAI function-calling format (default: Groq LLaMA 3.3-70b, swappable via env). The existing retrieval pipeline in `mimir_retrieval.rs` is extracted — unchanged — into a callable `run_retrieval`, which becomes the `search_mimir` tool. `mimir_chat` routes through the agent by default and falls back to the classic path on any failure. Background consolidation jobs distill chat into memory; orchestrator events auto-write facts.

**Tech Stack:** Rust (Tauri 2.0, sqlx non-macro, tokio), Postgres/Neon, Groq + OpenRouter (OpenAI-compatible chat completions), serde_json.

**Spec:** `docs/superpowers/specs/2026-07-23-mimir-memory-agent-design.md`

## Global Constraints

- SQL placeholders are ALWAYS `$N`, never `?N`. Non-macro `sqlx::query()` + `try_get`, never the `query!` macros.
- Tauri commands return `Result<T, String>`; `database: State<'_, Database>` is always the LAST parameter.
- All new response structs use `#[serde(rename_all = "camelCase")]`.
- IDs are app-generated: `uuid::Uuid::new_v4().to_string()` (TEXT PKs — no DB-side `gen_random_uuid()`).
- Never create a new `reqwest::Client` — use the shared client passed in / managed as Tauri state.
- Never use dotenvy or read `.env` in Rust — env vars come from `dev.sh`.
- JSON columns are JSONB, never TEXT.
- New migration number is **049** (048 already exists). Never modify an existing migration.
- Register every new `#[tauri::command]` in `main.rs`'s `generate_handler![]`.
- All shell commands below run from `C:/Users/dhruv/projects/Yggdrasil/src-tauri` unless stated otherwise. Work on the current branch (`fix-compilation-errors`).
- This crate has NO existing tests. New pure-logic functions get `#[cfg(test)] mod tests` unit tests (run with `cargo test`); DB/LLM-bound code is verified by `cargo build` + the runtime checklist in Task 9.
- `cargo build` may take several minutes on first run — that is normal, do not kill it.

---

### Task 1: Migration 049 — memory tables

**Files:**
- Create: `src-tauri/migrations/049_mimir_memory.sql`

**Interfaces:**
- Produces: tables `mimir_memory_facts` (unique expression index `idx_memory_facts_unique`) and `mimir_memory_shortterm` (UNIQUE `session_id`) that Tasks 2 and 7 write to.

- [ ] **Step 1: Write the migration file**

Create `src-tauri/migrations/049_mimir_memory.sql` with exactly:

```sql
-- Migration 049: Mimir memory system — durable facts + rolling session summaries

-- Long-term memory: one unified facts table, scoped user/tree/project.
-- entity_id makes per-entity facts (mastered_concept per node) accumulate
-- instead of overwriting; NULL entity_id = singular fact that upserts in place.
CREATE TABLE IF NOT EXISTS mimir_memory_facts (
    id          TEXT PRIMARY KEY,
    scope       TEXT NOT NULL CHECK(scope IN ('user','tree','project')),
    tree_id     TEXT REFERENCES trees(id) ON DELETE CASCADE,
    project_id  TEXT REFERENCES projects(id) ON DELETE CASCADE,
    fact_key    TEXT NOT NULL,
    entity_id   TEXT,
    fact_value  JSONB NOT NULL,
    confidence  FLOAT NOT NULL DEFAULT 0.7 CHECK(confidence >= 0.0 AND confidence <= 1.0),
    source      TEXT NOT NULL CHECK(source IN (
                    'agent_extraction','checkpoint_complete','resource_complete',
                    'user_said','manual','session_consolidation')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Collision-safe uniqueness: same (scope+owner+key+entity) upserts in place;
-- different entities coexist. COALESCE handles NULL owners/entities.
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_facts_unique ON mimir_memory_facts (
    scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')
);
CREATE INDEX IF NOT EXISTS idx_memory_facts_tree  ON mimir_memory_facts(tree_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_scope ON mimir_memory_facts(scope);

-- Short-term memory: ONE rolling summary row per chat session, updated
-- incrementally. covered_through = created_at of the last message summarized.
CREATE TABLE IF NOT EXISTS mimir_memory_shortterm (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
    summary         TEXT NOT NULL,
    key_topics      TEXT[] NOT NULL DEFAULT '{}',
    covered_through TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(session_id)
);
CREATE INDEX IF NOT EXISTS idx_memory_shortterm_topics ON mimir_memory_shortterm USING GIN(key_topics);
```

- [ ] **Step 2: Sanity-check numbering**

Run: `ls migrations | tail -3`
Expected: `047_mimir_proposals.sql`, `048_jobs_dedup_constraint.sql`, `049_mimir_memory.sql`

- [ ] **Step 3: Commit**

```bash
git add migrations/049_mimir_memory.sql
git commit -m "feat(memory): migration 049 — mimir_memory_facts + mimir_memory_shortterm"
```

---

### Task 2: `mimir_memory.rs` — fact CRUD, memory block, Tauri commands

**Files:**
- Create: `src-tauri/src/mimir_memory.rs`
- Modify: `src-tauri/src/main.rs` (add `mod mimir_memory;` after line 30 `mod ext_server;`; register 3 commands after the line `mimir_retrieval::mimir_chat,` in `generate_handler![]` — main.rs:113)

**Interfaces:**
- Consumes: migration 049 tables; `crate::database::Database`.
- Produces (used by Tasks 5–8):
  - `pub(crate) struct MemoryFact` / `pub(crate) struct MemoryContext` (camelCase serde)
  - `pub(crate) async fn set_memory_fact(pool: &PgPool, scope: &str, tree_id: Option<&str>, project_id: Option<&str>, fact_key: &str, entity_id: Option<&str>, fact_value: serde_json::Value, confidence: f64, source: &str) -> Result<String, String>`
  - `pub(crate) async fn get_memory_context(pool: &PgPool, tree_id: Option<&str>) -> Result<MemoryContext, String>`
  - `pub(crate) async fn delete_memory_fact(pool: &PgPool, fact_id: &str) -> Result<(), String>`
  - `pub(crate) fn format_memory_block(ctx: &MemoryContext, max_facts: usize) -> String`
  - Tauri commands: `get_memory_context_cmd`, `set_memory_fact_cmd`, `delete_memory_fact_cmd`

- [ ] **Step 1: Create the module with structs and a failing unit test for `format_memory_block`**

Create `src-tauri/src/mimir_memory.rs`:

```rust
// src-tauri/src/mimir_memory.rs
//
// Mimir's persistent memory.
//   Tier 3 (long-term): mimir_memory_facts — durable, scoped facts.
//   Tier 2 (short-term): mimir_memory_shortterm — one rolling summary per session.
// Facts are written by orchestrator events, by the agent's set_fact tool, and
// by session consolidation. Read via get_memory_context / the recall seed.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::Row;
use tauri::State;

use crate::database::Database;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryFact {
    pub id: String,
    pub scope: String,
    pub tree_id: Option<String>,
    pub project_id: Option<String>,
    pub fact_key: String,
    pub entity_id: Option<String>,
    pub fact_value: serde_json::Value,
    pub confidence: f64,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryContext {
    pub facts: Vec<MemoryFact>,
    pub user_facts: Vec<MemoryFact>,
    pub recent_topics: Vec<String>,
}

// ─── Recall seed formatting ──────────────────────────────────────────────────

/// Render a JSON fact_value compactly: bare strings stay bare, objects with a
/// "title" key collapse to the title, everything else serializes.
fn compact_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => o
            .get("title")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| v.to_string()),
        _ => v.to_string(),
    }
}

/// Build the "[Known facts...]" block injected into the chat system prompt.
/// Returns an empty string when there is nothing to say.
pub(crate) fn format_memory_block(ctx: &MemoryContext, max_facts: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for f in ctx.facts.iter().take(max_facts) {
        lines.push(format!(
            "- {}: {} (confidence {:.1})",
            f.fact_key,
            compact_value(&f.fact_value),
            f.confidence
        ));
    }
    let remaining = max_facts.saturating_sub(lines.len());
    for f in ctx.user_facts.iter().take(remaining) {
        lines.push(format!(
            "- [user] {}: {} (confidence {:.1})",
            f.fact_key,
            compact_value(&f.fact_value),
            f.confidence
        ));
    }
    if !ctx.recent_topics.is_empty() {
        lines.push(format!(
            "- Recent topics discussed: {}",
            ctx.recent_topics.join(", ")
        ));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\n\n[Known facts about the user and their progress]\n{}\n",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(key: &str, value: serde_json::Value, conf: f64) -> MemoryFact {
        MemoryFact {
            id: "f1".into(),
            scope: "tree".into(),
            tree_id: Some("t1".into()),
            project_id: None,
            fact_key: key.into(),
            entity_id: None,
            fact_value: value,
            confidence: conf,
            source: "manual".into(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn empty_context_renders_empty_string() {
        let ctx = MemoryContext::default();
        assert_eq!(format_memory_block(&ctx, 12), "");
    }

    #[test]
    fn facts_render_with_title_collapse_and_cap() {
        let mut ctx = MemoryContext::default();
        ctx.facts.push(fact(
            "mastered_concept",
            json!({ "title": "RRF Fusion", "node_id": "n1" }),
            0.9,
        ));
        ctx.facts.push(fact("prefers_format", json!("video"), 0.7));
        ctx.recent_topics = vec!["pgvector".into(), "hybrid search".into()];
        let block = format_memory_block(&ctx, 12);
        assert!(block.contains("[Known facts about the user and their progress]"));
        assert!(block.contains("- mastered_concept: RRF Fusion (confidence 0.9)"));
        assert!(block.contains("- prefers_format: video (confidence 0.7)"));
        assert!(block.contains("- Recent topics discussed: pgvector, hybrid search"));

        // Cap: max_facts=1 keeps only the first fact line (+ topics line)
        let capped = format_memory_block(&ctx, 1);
        assert!(capped.contains("mastered_concept"));
        assert!(!capped.contains("prefers_format"));
    }
}
```

- [ ] **Step 2: Wire the module and run the test to verify it compiles and passes**

In `src-tauri/src/main.rs`, after `mod ext_server;` (line 30) add:

```rust
mod mimir_memory;
```

Run: `cargo test mimir_memory 2>&1 | tail -15`
Expected: `test mimir_memory::tests::empty_context_renders_empty_string ... ok` and `...facts_render_with_title_collapse_and_cap ... ok` (2 passed). Warnings about unused structs are fine at this stage.

- [ ] **Step 3: Add the fact CRUD functions**

Append to `src-tauri/src/mimir_memory.rs` (above the `#[cfg(test)]` module):

```rust
// ─── Tier 3: fact CRUD ───────────────────────────────────────────────────────

const ALLOWED_SOURCES: [&str; 6] = [
    "agent_extraction", "checkpoint_complete", "resource_complete",
    "user_said", "manual", "session_consolidation",
];

/// Upsert a fact. Same (scope, owner, key, entity) updates in place;
/// distinct entity_ids accumulate as separate rows. Returns the fact id.
pub(crate) async fn set_memory_fact(
    pool: &PgPool,
    scope: &str,
    tree_id: Option<&str>,
    project_id: Option<&str>,
    fact_key: &str,
    entity_id: Option<&str>,
    fact_value: serde_json::Value,
    confidence: f64,
    source: &str,
) -> Result<String, String> {
    if fact_key.trim().is_empty() {
        return Err("fact_key must not be empty".into());
    }
    if !ALLOWED_SOURCES.contains(&source) {
        return Err(format!("invalid source '{}'", source));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let row = sqlx::query(
        "INSERT INTO mimir_memory_facts \
           (id, scope, tree_id, project_id, fact_key, entity_id, fact_value, confidence, source) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')) \
         DO UPDATE SET fact_value = EXCLUDED.fact_value, \
                       confidence = EXCLUDED.confidence, \
                       source     = EXCLUDED.source, \
                       updated_at = NOW() \
         RETURNING id"
    )
    .bind(&id)
    .bind(scope)
    .bind(tree_id)
    .bind(project_id)
    .bind(fact_key)
    .bind(entity_id)
    .bind(&fact_value)
    .bind(confidence.clamp(0.0, 1.0))
    .bind(source)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    row.try_get("id").map_err(|e| e.to_string())
}

fn row_to_fact(row: &sqlx::postgres::PgRow) -> Option<MemoryFact> {
    Some(MemoryFact {
        id: row.try_get("id").ok()?,
        scope: row.try_get("scope").ok()?,
        tree_id: row.try_get("tree_id").ok().flatten(),
        project_id: row.try_get("project_id").ok().flatten(),
        fact_key: row.try_get("fact_key").ok()?,
        entity_id: row.try_get("entity_id").ok().flatten(),
        fact_value: row.try_get("fact_value").ok()?,
        confidence: row.try_get("confidence").ok()?,
        source: row.try_get("source").ok()?,
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    })
}

const FACT_COLUMNS: &str =
    "id, scope, tree_id, project_id, fact_key, entity_id, fact_value, confidence, source, \
     created_at::text AS created_at, updated_at::text AS updated_at";

/// Read memory for a chat context: tree-scoped facts (when tree_id given),
/// user-level facts, and recent topics from short-term summaries.
pub(crate) async fn get_memory_context(
    pool: &PgPool,
    tree_id: Option<&str>,
) -> Result<MemoryContext, String> {
    let mut ctx = MemoryContext::default();

    if let Some(tid) = tree_id {
        let rows = sqlx::query(&format!(
            "SELECT {} FROM mimir_memory_facts \
             WHERE scope = 'tree' AND tree_id = $1 \
             ORDER BY confidence DESC, updated_at DESC LIMIT 50",
            FACT_COLUMNS
        ))
        .bind(tid)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        ctx.facts = rows.iter().filter_map(row_to_fact).collect();

        let topic_rows = sqlx::query(
            "SELECT ms.key_topics FROM mimir_memory_shortterm ms \
             JOIN mimir_chat_sessions cs ON cs.id = ms.session_id \
             WHERE cs.tree_id = $1 \
             ORDER BY ms.updated_at DESC LIMIT 5"
        )
        .bind(tid)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for row in &topic_rows {
            let topics: Vec<String> = row.try_get("key_topics").unwrap_or_default();
            for t in topics {
                if !ctx.recent_topics.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                    ctx.recent_topics.push(t);
                }
            }
        }
        ctx.recent_topics.truncate(10);
    }

    let user_rows = sqlx::query(&format!(
        "SELECT {} FROM mimir_memory_facts \
         WHERE scope = 'user' \
         ORDER BY confidence DESC, updated_at DESC LIMIT 25",
        FACT_COLUMNS
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    ctx.user_facts = user_rows.iter().filter_map(row_to_fact).collect();

    Ok(ctx)
}

pub(crate) async fn delete_memory_fact(pool: &PgPool, fact_id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM mimir_memory_facts WHERE id = $1")
        .bind(fact_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_memory_context_cmd(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<MemoryContext, String> {
    get_memory_context(&database.pool, Some(&tree_id)).await
}

#[tauri::command]
pub async fn set_memory_fact_cmd(
    tree_id: Option<String>,
    fact_key: String,
    fact_value: serde_json::Value,
    entity_id: Option<String>,
    confidence: Option<f64>,
    source: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String> {
    let scope = if tree_id.is_some() { "tree" } else { "user" };
    let src = source.unwrap_or_else(|| "manual".to_string());
    let src = if ALLOWED_SOURCES.contains(&src.as_str()) { src } else { "manual".to_string() };
    set_memory_fact(
        &database.pool,
        scope,
        tree_id.as_deref(),
        None,
        &fact_key,
        entity_id.as_deref(),
        fact_value,
        confidence.unwrap_or(0.7),
        &src,
    )
    .await
}

#[tauri::command]
pub async fn delete_memory_fact_cmd(
    fact_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    delete_memory_fact(&database.pool, &fact_id).await
}
```

- [ ] **Step 4: Register commands in `main.rs`**

In `generate_handler![]`, directly after the line `mimir_retrieval::mimir_chat,` (main.rs:113), add:

```rust
            mimir_memory::get_memory_context_cmd,
            mimir_memory::set_memory_fact_cmd,
            mimir_memory::delete_memory_fact_cmd,
```

- [ ] **Step 5: Build and test**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors (warnings acceptable).
Run: `cargo test mimir_memory 2>&1 | tail -5`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add src/mimir_memory.rs src/main.rs
git commit -m "feat(memory): mimir_memory module — fact CRUD, recall block, Tauri commands"
```

---

### Task 3: Pure extraction — `run_retrieval` + `build_system_prompt` (⚠ highest-risk task)

This is a **pure move refactor** of `mimir_chat` (mimir_retrieval.rs:367-1283). Behavior must be byte-for-byte identical. No logic changes, no reordering of side effects. The goal: retrieval and prompt-building become callable so Task 5 can use retrieval as a tool and Task 6 can rebuild the classic prompt in the fallback path.

**Files:**
- Modify: `src-tauri/src/mimir_retrieval.rs`

**Interfaces:**
- Consumes: existing private items in the same file (`RetrievalConfig`, `Candidate`, `rrf_merge`) and `crate::mimir_ingest::{get_embedding, vector_str}`, `crate::mimir::MimirChatSource`, `crate::constants::GROQ_API_URL`.
- Produces (used by Tasks 5–6):
  - `pub(crate) struct RetrievalStats` (Default + Clone) with fields `prematch_chunks_used: i32, graph_chunks_used: i32, candidates_before_rerank: i32, candidates_after_rerank: i32, rerank_fallback_used: bool, lexical_candidates: i32, hybrid_merged: i32` and method `pub(crate) fn merge(&mut self, other: &RetrievalStats)`
  - `pub(crate) struct RetrievalResult { pub sources: Vec<crate::mimir::MimirChatSource>, pub context_blocks: String, pub prereq_skill_names: Vec<String>, pub stats: RetrievalStats }`
  - `pub(crate) async fn run_retrieval(pool: &sqlx::PgPool, client: &reqwest::Client, api_key: &str, message: &str, node_id: Option<&str>, node_title: Option<&str>, node_description: Option<&str>) -> Result<RetrievalResult, String>`
  - `pub(crate) struct PromptInputs<'a>` + `pub(crate) fn build_system_prompt(inp: &PromptInputs) -> String`

- [ ] **Step 1: Add the structs and empty shells**

In `mimir_retrieval.rs`, directly above the `// ─── Chat (RAG) ───` comment (line 364), add:

```rust
// ─── Extracted retrieval pipeline ────────────────────────────────────────────
// Pure extraction of mimir_chat's retrieval sections so the agent loop can
// call retrieval as a tool and the fallback path can rebuild the classic
// prompt. NO behavior changes vs. the inline versions.

#[derive(Debug, Default, Clone)]
pub(crate) struct RetrievalStats {
    pub prematch_chunks_used: i32,
    pub graph_chunks_used: i32,
    pub candidates_before_rerank: i32,
    pub candidates_after_rerank: i32,
    pub rerank_fallback_used: bool,
    pub lexical_candidates: i32,
    pub hybrid_merged: i32,
}

impl RetrievalStats {
    pub(crate) fn merge(&mut self, other: &RetrievalStats) {
        self.prematch_chunks_used += other.prematch_chunks_used;
        self.graph_chunks_used += other.graph_chunks_used;
        self.candidates_before_rerank += other.candidates_before_rerank;
        self.candidates_after_rerank += other.candidates_after_rerank;
        self.rerank_fallback_used = self.rerank_fallback_used || other.rerank_fallback_used;
        self.lexical_candidates += other.lexical_candidates;
        self.hybrid_merged += other.hybrid_merged;
    }
}

pub(crate) struct RetrievalResult {
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub context_blocks: String,
    pub prereq_skill_names: Vec<String>,
    pub stats: RetrievalStats,
}
```

- [ ] **Step 2: Move the retrieval body into `run_retrieval`**

Create this function below the structs. Its body is **moved verbatim** from `mimir_chat` — sections 1 through 4 (currently lines 405-831: query embed, embeddings count, 3a pre-matched chunks, 3b GraphRAG, cosine + lexical + RRF + rerank + "Build sources and context" loop). Mechanical renames only:

- `&database.pool` → `pool`
- `&*client` → `client`
- `message` → `message` (now a `&str` param — change `message.clone()` in section 1's `query_text` construction to `message.to_string()`, and the `.bind(&message)` in the lexical query to `.bind(message)`)
- `node_id` / `node_title` / `node_description` → the `Option<&str>` params (`if let Some(ref nid) = node_id` becomes `if let Some(nid) = node_id`; `node_title.as_deref()` becomes just `node_title`)
- The stat counter locals (`prematch_chunks_used`, `graph_chunks_used`, `candidates_before_rerank`, `candidates_after_rerank`, `rerank_fallback_used`, `lexical_candidates_count`, `hybrid_merged_count`) become locals that populate the returned `RetrievalStats`
- The rerank prompt-log call `crate::brain::log_prompt_call(database.pool.clone(), ...)` becomes `crate::brain::log_prompt_call(pool.clone(), ...)`
- `cfg` comes from `RetrievalConfig::from_env()` at the top of the function (moved from mimir_chat)

Function skeleton (head and tail exact; `// [MOVED: ...]` markers show what moves in between):

```rust
pub(crate) async fn run_retrieval(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    api_key: &str,
    message: &str,
    node_id: Option<&str>,
    node_title: Option<&str>,
    node_description: Option<&str>,
) -> Result<RetrievalResult, String> {
    let cfg = RetrievalConfig::from_env();

    // [MOVED: section 1 — query_text + get_embedding + vector_str, lines 405-411]
    // [MOVED: section 2 — SELECT COUNT(*) FROM mimir_embeddings, lines 413-418]

    let mut sources: Vec<crate::mimir::MimirChatSource> = Vec::new();
    let mut context_blocks = String::new();
    let mut prematch_chunks_used: i32 = 0;
    let mut graph_chunks_used: i32 = 0;
    let mut prereq_skill_names: Vec<String> = Vec::new();

    // [MOVED: section 3a — pre-matched chunks block, lines 427-486]
    // [MOVED: section 3b — GraphRAG traversal block, lines 488-621]
    // [MOVED: stat locals + `if emb_count > 0` hybrid retrieval + rerank + build
    //         sources/context loop, lines 623-831]

    Ok(RetrievalResult {
        sources,
        context_blocks,
        prereq_skill_names,
        stats: RetrievalStats {
            prematch_chunks_used,
            graph_chunks_used,
            candidates_before_rerank,
            candidates_after_rerank,
            rerank_fallback_used,
            lexical_candidates: lexical_candidates_count,
            hybrid_merged: hybrid_merged_count,
        },
    })
}
```

- [ ] **Step 3: Move prompt building into `build_system_prompt`**

Below `run_retrieval`, add. The body is **moved verbatim** from `mimir_chat` sections 6 (lines 916-1003: base personality string, tutor block, tree-only intro, tree_block append, context append):

```rust
pub(crate) struct PromptInputs<'a> {
    pub node_title: Option<&'a str>,
    pub node_description: Option<&'a str>,
    pub skill_name: Option<&'a str>,
    pub phase_name: Option<&'a str>,
    pub prereq_skill_names: &'a [String],
    pub tree_block: &'a str,
    pub context_blocks: &'a str,
    pub project_name: Option<&'a str>,
    pub tree_id_present: bool,
}

pub(crate) fn build_system_prompt(inp: &PromptInputs) -> String {
    // [MOVED verbatim from mimir_chat lines 916-1003 with these renames:]
    //   node_title.as_deref()        → inp.node_title
    //   node_description.as_deref()  → inp.node_description
    //   skill_name.as_deref()        → inp.skill_name
    //   phase_name.as_deref()        → inp.phase_name
    //   prereq_skill_names           → inp.prereq_skill_names
    //   tree_block                   → inp.tree_block   (&str: `!inp.tree_block.is_empty()`)
    //   context_blocks               → inp.context_blocks
    //   project_name.as_deref()      → inp.project_name
    //   tree_id.is_some()            → inp.tree_id_present
    // Returns the final `system_prompt` String instead of mutating a local.
}
```

The moved body ends with:

```rust
    if !inp.tree_block.is_empty() {
        system_prompt.push_str(inp.tree_block);
    }
    if !inp.context_blocks.is_empty() {
        system_prompt.push_str("\n\nContext:\n");
        system_prompt.push_str(inp.context_blocks);
    }
    system_prompt
```

- [ ] **Step 4: Rewire `mimir_chat` to call both — identical behavior**

Inside `mimir_chat`, replace the moved regions with:

After section 0 (node ctx fetch, keep lines 382-403 as-is) — replace lines 405-831 with:

```rust
    // 1-4. Retrieval pipeline (extracted — see run_retrieval)
    let retrieval = run_retrieval(
        &database.pool,
        &*client,
        &api_key,
        &message,
        node_id.as_deref(),
        node_title.as_deref(),
        node_description.as_deref(),
    )
    .await?;
    let sources = retrieval.sources;
    let context_blocks = retrieval.context_blocks;
    let prereq_skill_names = retrieval.prereq_skill_names;
    let prematch_chunks_used = retrieval.stats.prematch_chunks_used;
    let graph_chunks_used = retrieval.stats.graph_chunks_used;
    let candidates_before_rerank = retrieval.stats.candidates_before_rerank;
    let candidates_after_rerank = retrieval.stats.candidates_after_rerank;
    let rerank_fallback_used = retrieval.stats.rerank_fallback_used;
    let lexical_candidates_count = retrieval.stats.lexical_candidates;
    let hybrid_merged_count = retrieval.stats.hybrid_merged;
```

(The `let cfg = RetrievalConfig::from_env();` at mimir_chat's top STAYS — the retrieval-log block at the end still reads `cfg.top_k` / `cfg.threshold`. `run_retrieval` builds its own cfg internally; `from_env` is cheap.)

Keep section 5 (tree_block, lines 833-913) as-is. Replace section 6 (lines 915-1003) with:

```rust
    // 6. Build system prompt (extracted — see build_system_prompt)
    let system_prompt = build_system_prompt(&PromptInputs {
        node_title: node_title.as_deref(),
        node_description: node_description.as_deref(),
        skill_name: skill_name.as_deref(),
        phase_name: phase_name.as_deref(),
        prereq_skill_names: &prereq_skill_names,
        tree_block: &tree_block,
        context_blocks: &context_blocks,
        project_name: project_name.as_deref(),
        tree_id_present: tree_id.is_some(),
    });
```

Everything after (sections 7-10 + retrieval log) stays untouched.

- [ ] **Step 5: Verify — build clean, no behavior drift**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`, zero errors.

Self-check the diff before committing: `git diff --stat` should show ONLY `src/mimir_retrieval.rs`. Read the diff and confirm: no SQL string changed, no prompt text changed, no log line changed, all `println!` markers intact.

- [ ] **Step 6: Commit**

```bash
git add src/mimir_retrieval.rs
git commit -m "refactor(mimir): extract run_retrieval + build_system_prompt from mimir_chat (pure move)"
```

---

### Task 4: `mimir_agent.rs` — provider config, tool schemas, response parsing

**Files:**
- Create: `src-tauri/src/mimir_agent.rs`
- Modify: `src-tauri/src/main.rs` (add `mod mimir_agent;` after `mod mimir_memory;`)

**Interfaces:**
- Consumes: `crate::mimir::groq_api_key`, `crate::constants::GROQ_API_URL`.
- Produces (used by Tasks 5–6):
  - `pub(crate) struct AgentModelConfig { pub base_url: String, pub api_key: String, pub model: String }` with `pub(crate) fn from_env() -> Result<Self, String>`
  - `pub(crate) fn tool_schemas() -> Vec<serde_json::Value>` — 4 OpenAI function defs
  - `pub(crate) struct ToolCallReq { pub id: String, pub name: String, pub arguments: serde_json::Value }`
  - `pub(crate) enum AgentStep { Answer(String), ToolCalls(Vec<ToolCallReq>) }`
  - `pub(crate) fn parse_agent_response(body: &serde_json::Value) -> Result<(AgentStep, serde_json::Value), String>`
  - `pub(crate) async fn call_agent_llm(client: &reqwest::Client, cfg: &AgentModelConfig, messages: &[serde_json::Value], tools: &[serde_json::Value], force_answer: bool) -> Result<(AgentStep, serde_json::Value), String>`

- [ ] **Step 1: Write failing unit tests**

Create `src-tauri/src/mimir_agent.rs` with the test module first (the referenced items don't exist yet — this will not compile, which is the failing state):

```rust
// src-tauri/src/mimir_agent.rs
//
// Model-agnostic tool-calling agent loop for Mimir.
// Speaks the OpenAI chat-completions function-calling format, so any
// provider exposing that API works (Groq, OpenRouter → Gemini/Claude/GPT...).
// Default: Groq LLaMA 3.3-70b. Swap via MIMIR_AGENT_{MODEL,BASE_URL,API_KEY}.

use serde_json::json;

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
```

Add `mod mimir_agent;` to `main.rs` after `mod mimir_memory;`.

Run: `cargo test mimir_agent 2>&1 | tail -10`
Expected: **compile error** — `tool_schemas`, `parse_agent_response`, `AgentStep` not found.

- [ ] **Step 2: Implement config, schemas, parsing, and the HTTP call**

Add above the test module:

```rust
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
```

- [ ] **Step 3: Run the tests**

Run: `cargo test mimir_agent 2>&1 | tail -8`
Expected: 5 passed (`tool_schemas_declares_all_four_tools`, both parse tests, empty-body error, malformed-args fallback).

- [ ] **Step 4: Commit**

```bash
git add src/mimir_agent.rs src/main.rs
git commit -m "feat(agent): model-agnostic provider config, tool schemas, response parsing"
```

---

### Task 5: Tool executors — `execute_tool` + source dedup

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs`

**Interfaces:**
- Consumes: `crate::mimir_retrieval::{run_retrieval, RetrievalStats}` (Task 3), `crate::mimir_memory::{get_memory_context, set_memory_fact}` (Task 2), `crate::mimir::MimirChatSource`.
- Produces (used by Task 6):
  - `pub(crate) struct ToolCtx<'a> { pub pool: &'a sqlx::PgPool, pub client: &'a reqwest::Client, pub groq_api_key: &'a str, pub tree_id: Option<String>, pub node_id: Option<String>, pub node_title: Option<String>, pub node_description: Option<String>, pub message: String }`
  - `#[derive(Default)] pub(crate) struct AgentTurnState { pub sources: Vec<MimirChatSource>, pub stats: RetrievalStats, pub tool_calls_made: u32 }`
  - `pub(crate) async fn execute_tool(name: &str, args: &serde_json::Value, ctx: &ToolCtx<'_>, state: &mut AgentTurnState) -> Result<String, String>`
  - `pub(crate) fn dedup_sources(sources: Vec<MimirChatSource>) -> Vec<MimirChatSource>`

- [ ] **Step 1: Write the failing dedup test**

Add to the `tests` module in `mimir_agent.rs`:

```rust
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
```

NOTE: this test constructs `MimirChatSource` directly. Check `src-tauri/src/mimir.rs:92` — if any field is not `pub`, make the struct's fields `pub` (they are consumed cross-module by `mimir_retrieval.rs` already, so they almost certainly are).

Run: `cargo test mimir_agent 2>&1 | tail -5`
Expected: compile error — `dedup_sources` not found.

- [ ] **Step 2: Implement `dedup_sources`, `ToolCtx`, `AgentTurnState`, `execute_tool`**

Add above the tests:

```rust
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
            let confidence = args["confidence"].as_f64().unwrap_or(0.7);
            let entity_id = args["entity_id"].as_str();
            let scope = if ctx.tree_id.is_some() { "tree" } else { "user" };
            let id = crate::mimir_memory::set_memory_fact(
                ctx.pool,
                scope,
                ctx.tree_id.as_deref(),
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
            use sqlx::Row;
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
```

Also check the `use sqlx::Row;` inside the `read_tree` arm: if the file already imports it at the top, remove the inner one — one import only, at the top of the file: add `use sqlx::Row;` to the top imports and drop the inline one.

- [ ] **Step 3: Build + test**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test mimir_agent 2>&1 | tail -5` → 6 passed (5 prior + dedup).

- [ ] **Step 4: Commit**

```bash
git add src/mimir_agent.rs src/mimir.rs
git commit -m "feat(agent): tool executors — search_mimir, get_facts, set_fact, read_tree"
```

---

### Task 6: Agent loop + wire into `mimir_chat` with fallback

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (add `run_agent_turn`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (`mimir_chat` becomes agent-first)

**Interfaces:**
- Consumes: everything from Tasks 3–5; `crate::mimir_memory::{get_memory_context, format_memory_block}`.
- Produces:
  - `pub(crate) struct AgentTurnResult { pub answer: String, pub sources: Vec<MimirChatSource>, pub stats: RetrievalStats, pub tool_calls_made: u32 }`
  - `pub(crate) async fn run_agent_turn(cfg: &AgentModelConfig, ctx: &ToolCtx<'_>, system_prompt: &str, history: &[serde_json::Value], user_message: &str, pool_for_log: sqlx::PgPool) -> Result<AgentTurnResult, String>`
  - Env switch: `MIMIR_AGENT_ENABLED` (default on; `"false"` restores classic path).

- [ ] **Step 1: Implement `run_agent_turn`**

Add to `mimir_agent.rs`:

```rust
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
```

- [ ] **Step 2: Rewire `mimir_chat` — agent-first with classic fallback**

In `mimir_retrieval.rs` `mimir_chat`, restructure as follows.

**(a)** Move the session-create + history-load block (section 7, currently after prompt building — the `session_id_opt` and `history_messages` code) UP so it runs right after section 0 (node ctx fetch) and before retrieval. It has no dependency on retrieval. Keep its code identical.

**(b)** Delete the eager `run_retrieval` call added in Task 3, and the `build_system_prompt` call. Replace with:

```rust
    // 5. Rich tree context block (unchanged — needed by both paths)
    // ... (tree_block code stays exactly where it is)

    // 6a. Agent-path system prompt: classic prompt minus retrieval context,
    //     plus memory recall seed and tool guidance.
    let memory_block = match tree_id.as_deref() {
        Some(tid) => crate::mimir_memory::get_memory_context(&database.pool, Some(tid))
            .await
            .map(|ctx| crate::mimir_memory::format_memory_block(&ctx, 12))
            .unwrap_or_default(),
        None => String::new(),
    };

    let mut agent_system_prompt = build_system_prompt(&PromptInputs {
        node_title: node_title.as_deref(),
        node_description: node_description.as_deref(),
        skill_name: skill_name.as_deref(),
        phase_name: phase_name.as_deref(),
        prereq_skill_names: &[],
        tree_block: &tree_block,
        context_blocks: "",
        project_name: project_name.as_deref(),
        tree_id_present: tree_id.is_some(),
    });
    agent_system_prompt.push_str(&memory_block);
    agent_system_prompt.push_str(
        "\n\nYou have tools: search_mimir (the user's personal library — call it before answering any \
         substantive knowledge question), get_facts / set_fact (long-term memory about the user — use \
         set_fact when the user states a durable preference, goal, or background), and read_tree (their \
         learning tree). Cite sources returned by search_mimir the same way as before."
    );

    // 6b. Run the agent; fall back to classic one-shot synthesis on any failure.
    let agent_enabled = std::env::var("MIMIR_AGENT_ENABLED")
        .map(|v| v != "false")
        .unwrap_or(true);

    let history_slice: Vec<serde_json::Value> = {
        let start = history_messages.len().saturating_sub(20);
        history_messages[start..].to_vec()
    };

    let mut agent_outcome: Option<crate::mimir_agent::AgentTurnResult> = None;
    if agent_enabled {
        match crate::mimir_agent::AgentModelConfig::from_env() {
            Ok(agent_cfg) => {
                let tool_ctx = crate::mimir_agent::ToolCtx {
                    pool: &database.pool,
                    client: &*client,
                    groq_api_key: &api_key,
                    tree_id: tree_id.clone(),
                    node_id: node_id.clone(),
                    node_title: node_title.clone(),
                    node_description: node_description.clone(),
                    message: message.clone(),
                };
                match crate::mimir_agent::run_agent_turn(
                    &agent_cfg,
                    &tool_ctx,
                    &agent_system_prompt,
                    &history_slice,
                    &message,
                    database.pool.clone(),
                )
                .await
                {
                    Ok(r) => agent_outcome = Some(r),
                    Err(e) => println!("⚠️  [agent] loop failed — falling back to classic synthesis: {}", e),
                }
            }
            Err(e) => println!("⚠️  [agent] config unavailable — classic path: {}", e),
        }
    }

    // 6c. Resolve answer + sources + stats from whichever path ran.
    let (answer, sources, stats) = match agent_outcome {
        Some(r) => (r.answer, r.sources, r.stats),
        None => {
            // Classic path: eager retrieval → full prompt → one-shot synthesis.
            let retrieval = run_retrieval(
                &database.pool,
                &*client,
                &api_key,
                &message,
                node_id.as_deref(),
                node_title.as_deref(),
                node_description.as_deref(),
            )
            .await?;
            let classic_prompt = build_system_prompt(&PromptInputs {
                node_title: node_title.as_deref(),
                node_description: node_description.as_deref(),
                skill_name: skill_name.as_deref(),
                phase_name: phase_name.as_deref(),
                prereq_skill_names: &retrieval.prereq_skill_names,
                tree_block: &tree_block,
                context_blocks: &retrieval.context_blocks,
                project_name: project_name.as_deref(),
                tree_id_present: tree_id.is_some(),
            });

            // [KEEP: the existing section-8 Groq synthesis call verbatim, with
            //  exactly two mechanical renames:
            //    `system_prompt` → `classic_prompt` in groq_messages construction
            //    `candidates_after_rerank` in the success log_prompt_call metadata
            //        → `retrieval.stats.candidates_after_rerank`
            //  Its log_prompt_call command/model/version strings stay exactly as
            //  they are ("mimir_chat", "mimir_chat_v2").]

            (answer, retrieval.sources, retrieval.stats)
        }
    };
```

**(c)** Sections 9 (persist messages), 10 (suggestions) and the retrieval-log block stay, with these mechanical adjustments:
- They now read `answer` and `sources` from the tuple above.
- The retrieval-log block's SEVEN stat variables now come from `stats`:
  `let log_prematch = stats.prematch_chunks_used;`, `let log_before = stats.candidates_before_rerank;`,
  `let log_after = stats.candidates_after_rerank;`, `let log_fallback = stats.rerank_fallback_used;`,
  `let log_lexical = stats.lexical_candidates;`, `let log_hybrid = stats.hybrid_merged;`,
  `let log_graph = stats.graph_chunks_used;`
- `groq_messages` construction moves inside the classic branch (agent path builds its own messages).
- The section-8 synthesis code lives ONLY inside the `None =>` fallback branch now.

**(d)** Delete now-unused locals from the old wiring (the Task-3 destructured stat variables). `cfg` at the top stays (retrieval-log block still uses `cfg.top_k` / `cfg.threshold`).

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished`, no errors. Fix any move/borrow issues by cloning the small strings involved (all context values are small).

- [ ] **Step 4: Run existing unit tests (regression gate)**

Run: `cargo test 2>&1 | tail -5`
Expected: all tests still pass (mimir_memory 2, mimir_agent 6).

- [ ] **Step 5: Commit**

```bash
git add src/mimir_agent.rs src/mimir_retrieval.rs
git commit -m "feat(agent): run_agent_turn loop; mimir_chat agent-first with classic fallback"
```

---

### Task 7: Distillation — consolidation + fact extraction + job + trigger

**Files:**
- Modify: `src-tauri/src/mimir_memory.rs` (add consolidation/extraction + parse helpers + `consolidate_session_cmd`)
- Modify: `src-tauri/src/orchestrator.rs` (new `ConsolidateSession` job variant + handler)
- Modify: `src-tauri/src/mimir_retrieval.rs` (`mimir_chat` gains `queue` state param + enqueue trigger)
- Modify: `src-tauri/src/main.rs` (register `consolidate_session_cmd`)

**Interfaces:**
- Consumes: `crate::llm_client::{call_llm, clean_llm_json}`, `crate::brain::log_prompt_call`, `crate::constants::GROQ_API_URL`, `crate::mimir::groq_api_key`, `set_memory_fact` (Task 2).
- Produces:
  - `pub(crate) async fn consolidate_session(pool: &PgPool, client: &reqwest::Client, session_id: &str) -> Result<bool, String>` (Ok(false) = too few new messages, skipped)
  - `pub(crate) async fn extract_longterm_facts(pool: &PgPool, client: &reqwest::Client, session_id: &str) -> Result<usize, String>`
  - `OrchestratorJob::ConsolidateSession { session_id: String }`
  - Tauri command `consolidate_session_cmd`

- [ ] **Step 1: Write failing parse tests**

Add to the `tests` module in `mimir_memory.rs`:

```rust
    #[test]
    fn parse_consolidation_handles_fenced_json() {
        let raw = "```json\n{\"summary\": \"Covered RRF and pgvector.\", \"key_topics\": [\"rrf\", \"pgvector\"]}\n```";
        let out = parse_consolidation(raw).unwrap();
        assert_eq!(out.summary, "Covered RRF and pgvector.");
        assert_eq!(out.key_topics, vec!["rrf", "pgvector"]);
    }

    #[test]
    fn parse_extracted_facts_tolerates_garbage() {
        let good = "{\"facts\": [{\"fact_key\": \"career goal\", \"fact_value\": \"ML engineering\", \"confidence\": 0.8}]}";
        let out = parse_extracted_facts(good);
        assert_eq!(out.facts.len(), 1);
        assert_eq!(out.facts[0].fact_key, "career goal");

        let bad = "no json here at all";
        assert_eq!(parse_extracted_facts(bad).facts.len(), 0);
    }
```

Run: `cargo test mimir_memory 2>&1 | tail -5`
Expected: compile error — `parse_consolidation` / `parse_extracted_facts` not found.

- [ ] **Step 2: Implement consolidation + extraction**

Add to `mimir_memory.rs` (above the tests):

```rust
// ─── Tier 2: session consolidation ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ConsolidationOut {
    pub summary: String,
    #[serde(default)]
    pub key_topics: Vec<String>,
}

pub(crate) fn parse_consolidation(raw: &str) -> Result<ConsolidationOut, String> {
    serde_json::from_str(&crate::llm_client::clean_llm_json(raw))
        .map_err(|e| format!("consolidation parse failed: {}", e))
}

fn default_extract_confidence() -> f64 { 0.6 }

#[derive(Debug, Deserialize)]
pub(crate) struct ExtractedFact {
    pub fact_key: String,
    pub fact_value: serde_json::Value,
    #[serde(default = "default_extract_confidence")]
    pub confidence: f64,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ExtractedFacts {
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
}

pub(crate) fn parse_extracted_facts(raw: &str) -> ExtractedFacts {
    serde_json::from_str(&crate::llm_client::clean_llm_json(raw)).unwrap_or_default()
}

/// Incrementally update the session's ONE rolling summary row.
/// Only messages newer than covered_through are read; the previous summary is
/// passed as context. Returns Ok(false) when there was too little new material.
pub(crate) async fn consolidate_session(
    pool: &PgPool,
    client: &reqwest::Client,
    session_id: &str,
) -> Result<bool, String> {
    let api_key = crate::mimir::groq_api_key()?;

    let existing = sqlx::query(
        "SELECT summary, key_topics, covered_through::text AS covered \
         FROM mimir_memory_shortterm WHERE session_id = $1"
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let (prev_summary, prev_topics, covered): (String, Vec<String>, Option<String>) = match &existing {
        Some(r) => (
            r.try_get("summary").unwrap_or_default(),
            r.try_get("key_topics").unwrap_or_default(),
            r.try_get::<Option<String>, _>("covered").unwrap_or(None),
        ),
        None => (String::new(), Vec::new(), None),
    };

    let rows = sqlx::query(
        "SELECT role, content, created_at::text AS created_at \
         FROM mimir_chat_messages \
         WHERE session_id = $1 \
           AND ($2::timestamptz IS NULL OR created_at > $2::timestamptz) \
         ORDER BY created_at ASC LIMIT 40"
    )
    .bind(session_id)
    .bind(&covered)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.len() < 4 {
        return Ok(false);
    }

    let mut transcript = String::new();
    let mut last_ts = String::new();
    for row in &rows {
        let role: String = row.try_get("role").unwrap_or_default();
        let content: String = row.try_get("content").unwrap_or_default();
        last_ts = row.try_get("created_at").unwrap_or_default();
        // User messages in full-ish; assistant answers trimmed — the gist is
        // what a summary needs, not the retrieved passages.
        let cap = if role == "assistant" { 600 } else { 1200 };
        let trimmed: String = content.chars().take(cap).collect();
        transcript.push_str(&format!("{}: {}\n", role, trimmed));
    }

    let system = "You maintain a rolling summary of a tutoring chat session. Merge the previous summary \
                  with the new messages into one updated summary. Return ONLY JSON: \
                  {\"summary\": \"<= 200 words\", \"key_topics\": [\"lowercase topic\", ...]} \
                  with at most 12 topics.";
    let prev_for_prompt = if prev_summary.is_empty() { "(none)".to_string() } else { prev_summary };
    let user = format!("Previous summary:\n{}\n\nNew messages:\n{}", prev_for_prompt, transcript);

    let (raw, latency) = crate::llm_client::call_llm(
        client,
        crate::constants::GROQ_API_URL,
        &api_key,
        "llama-3.3-70b-versatile",
        system,
        &user,
        400,
        true,
        0.2,
    )
    .await?;
    crate::brain::log_prompt_call(
        pool.clone(), "mimir_consolidate", "llama-3.3-70b-versatile", "mimir_consolidate_v1",
        latency, true, None, None,
    );

    let parsed = parse_consolidation(&raw)?;

    // Union topics (case-insensitive), cap 12.
    let mut topics = prev_topics;
    for t in parsed.key_topics {
        let tl = t.trim().to_lowercase();
        if !tl.is_empty() && !topics.iter().any(|x| x.eq_ignore_ascii_case(&tl)) {
            topics.push(tl);
        }
    }
    topics.truncate(12);

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mimir_memory_shortterm (id, session_id, summary, key_topics, covered_through) \
         VALUES ($1, $2, $3, $4, $5::timestamptz) \
         ON CONFLICT (session_id) DO UPDATE SET \
           summary = EXCLUDED.summary, \
           key_topics = EXCLUDED.key_topics, \
           covered_through = EXCLUDED.covered_through, \
           updated_at = NOW()"
    )
    .bind(&id)
    .bind(session_id)
    .bind(&parsed.summary)
    .bind(&topics)
    .bind(&last_ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    println!("🧠 [memory] consolidated session {} ({} new msgs, {} topics)", session_id, rows.len(), topics.len());
    Ok(true)
}

// ─── Tier 2→3: fact extraction from summaries ────────────────────────────────

/// Distill durable facts from recent short-term summaries for the session's
/// tree. Works off summaries, not raw messages. Returns number written.
pub(crate) async fn extract_longterm_facts(
    pool: &PgPool,
    client: &reqwest::Client,
    session_id: &str,
) -> Result<usize, String> {
    let api_key = crate::mimir::groq_api_key()?;

    let tree_row = sqlx::query("SELECT tree_id FROM mimir_chat_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let tree_id: Option<String> = tree_row
        .and_then(|r| r.try_get::<Option<String>, _>("tree_id").ok())
        .flatten();
    let Some(tree_id) = tree_id else { return Ok(0) };

    let rows = sqlx::query(
        "SELECT ms.summary, ms.key_topics \
         FROM mimir_memory_shortterm ms \
         JOIN mimir_chat_sessions cs ON cs.id = ms.session_id \
         WHERE cs.tree_id = $1 AND ms.updated_at > NOW() - INTERVAL '24 hours' \
         ORDER BY ms.updated_at DESC LIMIT 5"
    )
    .bind(&tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut digest = String::new();
    for row in &rows {
        let summary: String = row.try_get("summary").unwrap_or_default();
        let topics: Vec<String> = row.try_get("key_topics").unwrap_or_default();
        digest.push_str(&format!("Session summary: {}\nTopics: {}\n\n", summary, topics.join(", ")));
    }

    let system = "Extract durable facts about the LEARNER from these tutoring-session summaries. \
                  Only facts about the user themself: their knowledge level, misconceptions, \
                  preferences, goals, background. NOT facts about the subject matter. \
                  Return ONLY JSON: {\"facts\": [{\"fact_key\": \"snake_case_key\", \
                  \"fact_value\": \"short string\", \"confidence\": 0.0}]} \
                  — at most 8 facts, empty array if none.";

    let (raw, latency) = crate::llm_client::call_llm(
        client,
        crate::constants::GROQ_API_URL,
        &api_key,
        "llama-3.3-70b-versatile",
        system,
        &digest,
        500,
        true,
        0.2,
    )
    .await?;
    crate::brain::log_prompt_call(
        pool.clone(), "mimir_extract_facts", "llama-3.3-70b-versatile", "mimir_extract_facts_v1",
        latency, true, None, None,
    );

    let parsed = parse_extracted_facts(&raw);
    let mut written = 0usize;
    for f in parsed.facts.into_iter().take(8) {
        let key = f.fact_key.trim().to_lowercase().replace(' ', "_");
        if key.is_empty() { continue; }
        if set_memory_fact(
            pool, "tree", Some(&tree_id), None, &key, None,
            f.fact_value, f.confidence.clamp(0.0, 1.0), "session_consolidation",
        )
        .await
        .is_ok()
        {
            written += 1;
        }
    }
    println!("🧠 [memory] extracted {} long-term facts for tree {}", written, tree_id);
    Ok(written)
}

#[tauri::command]
pub async fn consolidate_session_cmd(
    session_id: String,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<(), String> {
    consolidate_session(&database.pool, &*client, &session_id).await?;
    let _ = extract_longterm_facts(&database.pool, &*client, &session_id).await;
    Ok(())
}
```

- [ ] **Step 3: Run the parse tests**

Run: `cargo test mimir_memory 2>&1 | tail -5`
Expected: 4 passed (2 prior + 2 new).

- [ ] **Step 4: Add the orchestrator job**

In `orchestrator.rs`:

To the `OrchestratorJob` enum (after `ExtractSkillsFromResource { resource_id: String },` line 27) add:

```rust
    ConsolidateSession { session_id: String },
```

To the worker `match` (after the `ExtractSkillsFromResource` arm, line 106-108) add:

```rust
                OrchestratorJob::ConsolidateSession { session_id } => {
                    match crate::mimir_memory::consolidate_session(&pool, &client, &session_id).await {
                        Ok(true) => {
                            // Chain fact extraction sequentially (worker holds no sender).
                            match crate::mimir_memory::extract_longterm_facts(&pool, &client, &session_id).await {
                                Ok(n) if n > 0 => {
                                    let _ = app.emit("ygg-memory-updated", serde_json::json!({ "facts": n }));
                                }
                                Ok(_) => {}
                                Err(e) => println!("⚠️  [orch] extract_longterm_facts failed: {}", e),
                            }
                        }
                        Ok(false) => {}
                        Err(e) => println!("⚠️  [orch] consolidate_session failed: {}", e),
                    }
                }
```

- [ ] **Step 5: Add the trigger in `mimir_chat`**

In `mimir_retrieval.rs`, add the queue param to `mimir_chat`'s signature, BEFORE `database` (which stays last per convention):

```rust
    queue: State<'_, crate::orchestrator::JobQueue>,
    database: State<'_, Database>,
```

(Tauri state params are injected — the frontend `invoke` call needs no change.)

At the END of section 9 (after both message INSERTs, inside the `if let Some(ref sid) = session_id_opt` block), add:

```rust
        // Every ~20 messages, consolidate this session into short-term memory.
        let cnt_row = sqlx::query(
            "SELECT COUNT(*) AS n FROM mimir_chat_messages WHERE session_id = $1"
        )
        .bind(sid)
        .fetch_one(&database.pool)
        .await;
        if let Ok(row) = cnt_row {
            let n: i64 = row.try_get("n").unwrap_or(0);
            if n > 0 && n % 20 == 0 {
                let _ = queue
                    .send(crate::orchestrator::OrchestratorJob::ConsolidateSession {
                        session_id: sid.clone(),
                    })
                    .await;
            }
        }
```

- [ ] **Step 6: Register the command**

In `main.rs` `generate_handler![]`, after `mimir_memory::delete_memory_fact_cmd,` add:

```rust
            mimir_memory::consolidate_session_cmd,
```

- [ ] **Step 7: Build + full test suite**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -5` → 10 passed (4 memory + 6 agent).

- [ ] **Step 8: Commit**

```bash
git add src/mimir_memory.rs src/orchestrator.rs src/mimir_retrieval.rs src/main.rs
git commit -m "feat(memory): session consolidation + fact extraction, ConsolidateSession job, ~20-message trigger"
```

---

### Task 8: Event fact writes + dev.sh env vars

**Files:**
- Modify: `src-tauri/src/orchestrator.rs` (`on_checkpoint_completed`, `on_resource_completed`)
- Modify: `dev.sh` (repo root)

**Interfaces:**
- Consumes: `crate::mimir_memory::set_memory_fact` (Task 2).
- Produces: `mastered_concept` / `resource_studied` facts written automatically; `MIMIR_AGENT_*` env vars exported by dev.sh.

- [ ] **Step 1: `on_checkpoint_completed` — write mastered_concept fact**

In `orchestrator.rs`, inside `on_checkpoint_completed` (starts line 456), directly BEFORE the final `app.emit("ygg-checkpoint-completed", ...)` (line 498), add:

```rust
    // Memory: record mastered concept — entity-keyed so each checkpoint
    // accumulates its own fact instead of overwriting the last one.
    let title_row = sqlx::query("SELECT title FROM tree_nodes WHERE id = $1")
        .bind(node_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if let Some(row) = title_row {
        let title: String = row.try_get("title").unwrap_or_default();
        if !title.is_empty() {
            if let Err(e) = crate::mimir_memory::set_memory_fact(
                pool,
                "tree",
                Some(tree_id),
                None,
                "mastered_concept",
                Some(node_id),
                serde_json::json!({ "title": title, "node_id": node_id }),
                0.9,
                "checkpoint_complete",
            )
            .await
            {
                println!("⚠️  [orch] memory fact (mastered_concept) failed: {}", e);
            }
        }
    }
```

- [ ] **Step 2: `on_resource_completed` — write resource_studied facts**

In `on_resource_completed`, directly AFTER the `for tree_id in &affected_trees { ... }` recalc loop (ends line 432) and before the `sync_trees_inner` call (line 434), add:

```rust
    // Memory: record studied resource for every affected tree (entity-keyed).
    let title_row = sqlx::query("SELECT title FROM mimir_resources WHERE id = $1")
        .bind(resource_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let resource_title: String = title_row
        .and_then(|r| r.try_get("title").ok())
        .unwrap_or_default();
    if !resource_title.is_empty() {
        for tree_id in &affected_trees {
            if let Err(e) = crate::mimir_memory::set_memory_fact(
                pool,
                "tree",
                Some(tree_id),
                None,
                "resource_studied",
                Some(resource_id),
                serde_json::json!({ "title": resource_title, "resource_id": resource_id }),
                0.8,
                "resource_complete",
            )
            .await
            {
                println!("⚠️  [orch] memory fact (resource_studied) failed: {}", e);
            }
        }
    }
```

- [ ] **Step 3: dev.sh env vars**

In `dev.sh` (repo root, NOT src-tauri), after the `export CONCEPT_GRAPH_API_KEY="$OPENROUTER_API_KEY"` line (line 36), add:

```bash
export MIMIR_AGENT_MODEL="llama-3.3-70b-versatile"
export MIMIR_AGENT_BASE_URL="https://api.groq.com/openai/v1/chat/completions"
export MIMIR_AGENT_API_KEY="$GROQ_API_KEY"
# export MIMIR_AGENT_ENABLED="false"   # uncomment to force the classic one-shot chat path
```

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.

- [ ] **Step 5: Commit**

```bash
git add src/orchestrator.rs ../dev.sh
git commit -m "feat(memory): auto-write mastered_concept/resource_studied facts; MIMIR_AGENT env vars"
```

---

### Task 9: Final verification

**Files:** none created — verification only.

- [ ] **Step 1: Full build + tests**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -8` → 10 passed, 0 failed.

- [ ] **Step 2: Diff review**

Run: `git log --oneline -8` — expect the 8 task commits.
Run: `git diff <base>..HEAD --stat` (base = commit before Task 1) — expect ONLY: `migrations/049_mimir_memory.sql`, `src/mimir_memory.rs`, `src/mimir_agent.rs`, `src/mimir_retrieval.rs`, `src/orchestrator.rs`, `src/main.rs`, `dev.sh`.

- [ ] **Step 3: Runtime checklist (requires `bash dev.sh` — user-driven; do NOT fake these)**

These need the live app + DB and are for the human/final review session, not CI:

- [ ] App starts; migration 049 applies (check startup logs).
- [ ] Ask Mimir a knowledge question → console shows `🛠  [agent] tool call 1/6: search_mimir`; answer has citations.
- [ ] Tell Mimir "I'm switching into ML engineering" → a `set_fact` tool call fires; row appears: `SELECT * FROM mimir_memory_facts WHERE source='agent_extraction';`
- [ ] Complete two different checkpoints → TWO `mastered_concept` rows (collision fix proof): `SELECT fact_key, entity_id FROM mimir_memory_facts WHERE fact_key='mastered_concept';`
- [ ] Complete a resource → `resource_studied` row(s) appear.
- [ ] New chat turn on the same tree → system prompt includes the `[Known facts...]` block (recall seed; check via answer behavior or a temporary println).
- [ ] Reach 20 messages in one session → `mimir_memory_shortterm` has ONE row for that session with summary + topics + `covered_through` set.
- [ ] Manually invoke `consolidate_session_cmd` from devtools → same row updates, `session_consolidation` facts may appear.
- [ ] Set `MIMIR_AGENT_ENABLED=false` → classic one-shot path runs (no `[agent]` logs), chat still works.
- [ ] Swap `MIMIR_AGENT_MODEL`/`MIMIR_AGENT_BASE_URL` to an OpenRouter model (e.g. `google/gemini-2.5-flash` + `https://openrouter.ai/api/v1/chat/completions` + `MIMIR_AGENT_API_KEY=$OPENROUTER_API_KEY`) → agent loop still functions (model-agnostic proof).

- [ ] **Step 4: Hand off**

Use superpowers:finishing-a-development-branch to decide merge/PR/cleanup with the user.
