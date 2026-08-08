# Reference: deepagents, Hound, and Graphify — What Yggdrasil Takes From Each

**Date:** 2026-07-23
**Purpose:** Architectural reference. These three projects are inspiration sources for
Yggdrasil's v3 evolution — they inform design decisions but we never fork or depend on them.

---

## 1. deepagents (LangChain)

- **Repo:** github.com/langchain-ai/deepagents · **Language:** Python (LangGraph) · **License:** MIT
- **Concept:** "Batteries-included agent harness" — a production agent loop with sub-agents,
  filesystem access, context management, human-in-the-loop, and skills.

**Layering:** LangGraph (runtime/streaming/persistence) → LangChain `create_agent` (minimal harness)
→ deepagents (opinionated harness: sub-agents, middleware, filesystem, skills). The agent loop is a
`StateGraph` with `agent` (LLM call), `tools` (dispatch), and `summarize` (context management) nodes.

### Patterns Yggdrasil borrows

- **Middleware stack** — composable `pre_run` / `post_run` hooks around the model call
  (human-in-the-loop, sub-agent routing, summarize). Rust equivalent: a `Middleware` trait +
  `MiddlewareChain`.
- **Sub-agents with isolated context** — the key insight: sub-agents do **not** share context
  windows. Each gets a fresh, filtered message list + scoped tool set. Lets Mimir split into modes
  (Tree Tutor, Researcher, Skill Advisor, Project Auditor), each with only the tools/context it needs.
- **Tool trait** — tools are a simple interface (`name`, `description`, `parameters` JSON schema,
  `execute`). Rust: an `AgentTool` trait + registry. MCP servers can also be tool sources.
- **Human-in-the-loop** — interrupt tool calls for approval on destructive ops. Yggdrasil already
  has this shape in `graph_audit.rs` (LLM proposes, human reviews); the harness extends it to any
  destructive tool (delete fact, merge skills, delete resource).
- **Filesystem with permissions** — pluggable backends + per-agent path allowlists. Maps to Tauri
  v2 capabilities + the `fs` plugin scoped to user-added project directories.
- **Context management** — auto-summarize long conversations, offload tool output to disk.
  Yggdrasil: after N messages, enqueue a background consolidation job → `mimir_memory_shortterm`,
  truncate oldest active messages.

**Planned Mimir tools** (agent loop): `search_mimir`, `read_tree`, `set_fact`, `get_facts`,
`smart_fetch` (Hound), `smart_search` (Hound), `read_file`, `list_project_files`,
`complete_checkpoint`, `suggest_next`.

**What deepagents is NOT for Yggdrasil:** no LangChain/LangGraph (Python; Yggdrasil is Rust/Tauri),
no dcode CLI, no full LangSmith tracing (the `prompt_logs` table covers the basics).

---

## 2. Hound (master-fetch)

- **Repo:** github.com/dondai1234/master-fetch · **PyPI:** `hound-mcp` · **License:** MIT
- **Concept:** One MCP server that gives any agent the web — fetch, crawl, search, PDF extraction,
  all local and free.

**Architecture:** a single MCP stdio server with an HTTP fetcher (primp), a stealth browser
(Patchright + system Chrome, Cloudflare Turnstile solver, fingerprint rotation), a search engine
(10 keyless backends in parallel + ONNX cross-encoder reranker + consensus scoring), a PDF extractor
(pdfplumber + pypdfium2 OCR), a best-first crawler, and a size-capped WAL SQLite cache.

**The 6 tools:** `smart_fetch` (auto-escalate to browser, BM25 `focus`, page actions),
`smart_crawl` (best-first same-domain), `smart_search` (keyless, 10 backends, `find_similar`),
`screenshot`, `cache_clear`, `version`.

**Integration decision — Option C: optional sidecar over HTTP.** If Hound is installed, Mimir's
agent uses its tools for web search + smart fetch; if not, fall back to the existing scraper
(port 3002). New `src-tauri/src/hound_client.rs` with `smart_search`, `smart_fetch`, `health`;
detected on startup and `manage()`d as Tauri state so the agent degrades gracefully.

**What Hound adds:** keyless reranked web search, anti-bot/stealth fetch (Cloudflare bypass),
PDF OCR for scanned docs, best-first crawl with budgets, `find_similar`, TTL cache — all of which
current Yggdrasil lacks.

---

## 3. Graphify

- **Repo:** github.com/Graphify-Labs/graphify · **PyPI:** `graphifyy` · **License:** Apache-2.0 / MIT
- **Concept:** Turn any codebase into a queryable knowledge graph. Local AST parsing (tree-sitter,
  40+ languages), every edge explained, no vector store.

**Architecture:** tree-sitter AST (calls/imports/inherits) + a semantic LLM pass
(concepts/relationships/rationale) → graph construction (`graph.json`, D3 `graph.html`,
`GRAPH_REPORT.md`) → graph verbs (`query`, `path`, `explain`) → Leiden community detection.

### Patterns Yggdrasil borrows

- **Concept graphs as first-class entities** — a knowledge graph is a *data structure*, not a
  visualization. Yggdrasil currently stores `concept_graph` as an opaque JSONB blob (migration 033);
  promote it to real tables: `concept_graphs`, `concept_graph_nodes` (with `file_path`,
  `line_start/end`, optional `embedding`), `concept_graph_edges` (typed relationship + `confidence`
  tag). `trees.active_concept_graph_id` FK replaces the blob.
- **Graph verbs** — `query "<q>"` → scoped subgraph; `path A B` → shortest prerequisite path;
  `explain "X"` → node + all edges + linked resources. Partially exist in `read_models.rs`
  (`get_prereq_path`, `compute_learning_path`, `get_node_neighborhood`) but those traverse the rigid
  trunk/branch/leaf hierarchy; concept-graph traversal is a flexible DAG.
- **Edge confidence tags** — every edge tagged `extracted` / `inferred` / `ambiguous` for trust.
- **Community detection** — Leiden clustering to color subsystems: natural skill clusters,
  tightly-coupled nodes to learn together, bridge resources.
- **Codebase-to-graph** — scan a project dir → concepts → files → functions. Use the **tree-sitter
  Rust crate** directly (not the Python package) — same 40+ grammars, native Rust.

**What Graphify is NOT for Yggdrasil:** no graphify sidecar (heavy PyPI deps — use tree-sitter from
Rust), no skill registration (`/graphify` is for coding agents), no doc/image semantic extraction at
this stage (concept graph is LLM-generated from repo analysis; AST extraction is a later phase).

---

## 4. Phase roadmap — what Yggdrasil takes and when

| Pattern | Source | Phase |
|---|---|---|
| Three-tier memory (working/short/long) | deepagents | **0** |
| Agent loop (think → act → observe) | deepagents | **0-lite / 3** |
| Tool trait + registry | deepagents | 0-lite / 3 |
| Model-agnostic tool-calling | deepagents | **0** |
| Sub-agents with isolated context | deepagents | 4 |
| Human-in-the-loop for destructive ops | deepagents + own `graph_audit` | 4 |
| Context-window management (summarize) | deepagents | 0 / 4 |
| Concept graphs as first-class tables | Graphify | 1 |
| Graph verbs (query/path/explain) | Graphify | 1 |
| Edge confidence tags | Graphify | 1 |
| Community detection | Graphify | 2 |
| D3 force-directed graph viz | Graphify | 2 |
| Web search (10 keyless backends) | Hound | 3 |
| Anti-bot fetch + stealth browser | Hound | 3 |
| PDF OCR for scanned docs | Hound | 3 |
| Optional sidecar pattern | Hound | 3 |
| tree-sitter AST from Rust | Graphify | 5 |

### Visual roadmap
```
Phase 0  Memory + minimal agent   ← deepagents   (migration 049, mimir_memory.rs, mimir_agent.rs)
Phase 1  Concept graphs           ← Graphify     (concept_graph tables, graph verbs)
Phase 2  Graph visualization      ← Graphify     (D3 force layout, community colors, dual view)
Phase 3  Full agent harness       ← deepagents   (tool registry, Hound integration)
Phase 4  Sub-agents + HITL        ← deepagents   (AgentRegistry, isolated context, review middleware)
Phase 5  Project scanning         ← Graphify     (tree-sitter Rust, file→concept mapping)
```

> **Note (2026-07-23):** Phase 0 as actually designed pulls a *minimal* slice of Phase 3 forward —
> the agent loop + a 4-tool registry — so the memory tools have a live consumer. See
> `docs/superpowers/specs/2026-07-23-mimir-memory-agent-design.md`.

---

## 5. Quick comparison

| Dimension | deepagents | Hound | Graphify | Yggdrasil takes |
|---|---|---|---|---|
| Language | Python | Python | Python | **Rust** |
| Runtime | LangGraph state machine | MCP stdio server | CLI + MCP server | Tauri app + tokio |
| Install | `pip install deepagents` | `pip install hound-mcp` | `uv tool install graphifyy` | Single Tauri binary |
| Model support | Any tool-calling LLM | None (no LLM in loop) | Gemini/Claude/OpenAI/etc | OpenRouter + Groq (agnostic) |
| Filesystem | Pluggable backends | None | Local only | Tauri fs plugin |
| Web access | Tool-based | Built-in (fetch+search) | PDFs only | Hound sidecar |
| Persistence | LangGraph checkpoints | SQLite cache | graph.json | Postgres (Neon) |
| Graph model | None | None | AST knowledge graph | Tree + concept graph + skill graph |
| Sub-agents | Built-in | None | None | Will build (deepagents pattern) |
