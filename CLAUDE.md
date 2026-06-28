# Yggdrasil — Claude Code Development Guide

## What This App Is

A personal learning OS. Build a project, paste the repo, Yggdrasil generates a skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

Read `PRD.md` for the full vision. This file is your technical bible.

---

## Current State (v2.6)

All core features shipped (Phases 0-6 complete):

**Core:** Project CRUD, AI tree generation (two-phase: concept graph → tree), canvas-based organic tree renderer (polar layout), checkpoint completion + unlock mechanics + progress cascade, tree versioning + regeneration, tree export as ZIP.

**Mimir (RAG):** URL/PDF/text ingestion, TOC-aware chunking, 1024-dim embeddings, pgvector storage. Hybrid retrieval (RRF cosine + FTS), GraphRAG traversal, Groq LLaMA 3.3-70b synthesis, reranking, source citations. Persistent chat session memory per `(tree_id, node_id)`. Full tree context awareness (phase-breakdown, tutor vs. tree-only mode). Pre-matched chunks via `mimir_node_links`. Reading progress tracking per section.

**Skill pipeline:** LLM-based resource→skill extraction (`ExtractSkillsFromResource` — samples 30 chunks, ANN pre-filters 50 candidates, gemini-3.1-flash-lite verifies, writes to `mimir_skill_links` with `source='llm_extraction'`). ANN links (`source='ann'`) are legacy. Auto-tag on ingest, then `MatchResourceToNodes` enqueued → ANN top-5 node matching with in-memory reranking.

**Graph audit** (`graph_audit.rs`, migration 047 `mimir_proposals`): LLM proposes graph repairs, human reviews. `run_graph_audit` flags cosine < 0.12 duplicate pairs + nodes with >20 dependents → inserts pending proposals. `approve_proposal` executes merge/delete/rename cascade. `clear_all_proposals` wipes pending before a fresh run.

**Pages:** Homepage, ProjectTreePage, TreesPage, DailyPage (Eisenhower Matrix), ResourcesPage, WorkPage (co-op galaxy), JobsPage (kanban + JD analysis + tailored projects panel), ResumePage, IdeasPage, SkillsPage (canvas lane layout, hover subgraph, zoom-adaptive labels).

**Infrastructure:** Background async job queue (orchestrator.rs, tokio mpsc, cap 64). 11 read-model commands (`read_models.rs`). Shared `reqwest::Client` as Tauri state. Prompt/model version logging (`prompt_logs`). Retrieval logging (`mimir_retrieval_logs`). All migrations (001–047) auto-run on startup.

### Graph Audit Workflow
```
1. Clear All pending proposals  (wipe stale runs before starting fresh)
2. Run Audit  (gemini-3.1-flash-lite; flags cosine < 0.12 pairs + nodes with >20 dependents)
3. Review proposals:
   - APPROVE: punctuation variants ("Back-end"/"Backend"), plural/singular ("LLM"/"LLMs")
   - REJECT: broad synonym merges, vague root renames
4. Re-run after approvals — distances shift once merged skills are removed
```

---

## Project Structure

```
src/
  components/   MimirChat.tsx, ProjectInput.tsx, YggdrasilTree.tsx
  contexts/     MimirContext.tsx
  pages/        Homepage, ProjectTreePage, TreesPage, DailyPage, ResourcesPage,
                WorkPage, JobsPage, IdeasPage, ResumePage, SkillsPage
  layouts/      MainLayout.tsx
  types.ts
src-tauri/src/
  main.rs               # Command registration — register ALL new commands here
  commands.rs           # Project CRUD
  tree_commands.rs      # Tree/node CRUD, checkpoint, unlock, versioning
  brain.rs              # AI orchestration (thin coordinator)
  llm_client.rs         # call_llm helper + shared HTTP client
  github.rs             # GitHub REST API
  prompt_builders.rs    # All LLM prompt templates
  tree_persistence.rs   # Tree/node DB writes post-generation
  mimir_ingest.rs       # Ingest pipeline, chunking, embeddings
  mimir_retrieval.rs    # Hybrid RAG, GraphRAG, session memory, tree context
  mimir_tags.rs         # Auto-tagging
  mimir_manage.rs       # Rescrape, re-embed, resource CRUD, reading progress
  orchestrator.rs       # Background job queue + ygg-* events
  read_models.rs        # 11 purpose-built read-model commands
  graph_audit.rs        # Proposal-based graph repair (LLM proposes, human reviews)
  skill_commands.rs     # Universal skill sync, deps, gaps, aliases, merge
  job_commands.rs       # Job applications + save_tailored_projects
  ext_server.rs         # Axum HTTP bridge for Chrome extension (port 1421)
  work_commands.rs      # Co-op/topic/resource/skill
  daily_commands.rs     # Eisenhower Matrix
  resume_commands.rs    # Resume parsing
  idea_commands.rs      # Ideas CRUD
  export_commands.rs    # Tree ZIP export
  database.rs           # PgPool + migrations
src-tauri/migrations/   # 001–048, auto-run on startup
scraper/main.py         # Python FastAPI (port 3002): /fetch /fetch-pdf /fetch-playlist /discover /health
dev.sh                  # Bitwarden env loader + launch
extension/              # Chrome extension (MV3)
  manifest.json         # Permissions: storage, activeTab, scripting, alarms
  popup.html / popup.ts # Sidebar popup UI — form → success/error/offline states
  content.ts            # Extracts company/position from page title + meta tags
  background.ts         # Service worker — 5-min alarm to flush offline queue
  tsconfig.json / package.json
  dist/                 # tsc output (popup.js, content.js, background.js)
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Backend | Rust + sqlx |
| Database | Postgres on Neon (pgvector) |
| AI | OpenRouter (Gemini Flash — tree gen, concept graphs, audit) + Groq (LLaMA — chat/skills) |
| Tree renderer | Custom HTML Canvas — polar layout, tapered branches |
| Work/Skills | D3 force simulation |
| Embeddings | `perplexity/pplx-embed-v1-0.6b` via OpenRouter — 1024-dim, L2-normalized |
| Vector search | pgvector HNSW indexes on `title_embedding` (nodes) + `embedding` (skills) |
| Hybrid retrieval | RRF over pgvector cosine + Postgres FTS (tsvector GIN) |
| Background jobs | tokio mpsc channel — `JobQueue` managed Tauri state |
| Scraper | Python FastAPI (port 3002) — pymupdf, YouTube transcripts |

---

## Database Schema

### Core
```sql
projects        -- id, name, description, discipline_ids JSONB, skill_ids JSONB, status, progress, active_tree_id FK, created_at
trees           -- id, project_id FK, name, version INT, parent_tree_id FK, archived_at, created_at
tree_nodes      -- id, tree_id FK, parent_id FK, type (trunk/branch/leaf), title, description,
                -- progress, tasks JSONB, resources JSONB, x, y, order_index, is_locked,
                -- concept_id TEXT, concept_slug TEXT, title_embedding vector(1024)
                -- trunk=phase(never locked), branch=skill(locked until predecessor=100), leaf=checkpoint
tree_edges      -- id, tree_id FK, source_node_id FK, target_node_id FK
disciplines     -- id, name, description, color
```

### Mimir
```sql
mimir_resources      -- id, title, url, type, status, tags TEXT[], is_completed, raw_text, sections_json JSONB,
                     -- transcript_source, transcript_mode, transcript_chars
mimir_chunks         -- id, resource_id FK, content, chunk_index, section_title, page_start, page_end, fts_vector TSVECTOR
mimir_embeddings     -- id, chunk_id FK, embedding vector(1024)
mimir_node_links     -- id, resource_id FK, node_id FK, relevance_score, matched_chunk_id FK,
                     -- matched_section_title, matched_page_start, matched_page_end
mimir_chat_sessions  -- id, tree_id FK, node_id FK; UNIQUE(tree_id, node_id)
mimir_chat_messages  -- id, session_id FK, role, content, sources JSONB
mimir_retrieval_logs -- id, query, node_id, tree_id, top_k, threshold, candidates_*, sources JSONB
mimir_skill_links    -- id, skill_id FK, resource_id FK, relevance_score, matched_chunk_id FK,
                     -- matched_section_title, matched_page_start, matched_page_end,
                     -- source CHECK('manual'|'ann'|'tree_bridge'|'llm_extraction'), confidence FLOAT
                     -- source semantics: manual=user (never overwrite), ann=legacy, llm_extraction=primary path
mimir_proposals      -- id, proposal_type, target_skill_id FK, merge_into_skill_id FK,
                     -- proposed_name, reason, status CHECK('pending'|'approved'|'rejected'), created_at
transcript_jobs           -- id, resource_id FK, status, attempts, last_error, next_retry_at
resource_reading_progress -- id, resource_id FK, section_title, page_start, page_end, completed_at
```

### Skills (domain-agnostic v2)
```sql
skill_domains      -- id, name UNIQUE, description, parent_domain_id FK, color
                   -- seeded: Engineering, Mathematics, Science, Computing, Communication, Design, Business, Research
universal_skills   -- id, name, domain_id FK, kind CHECK(concept|technical|soft|practical|domain|unclassified),
                   -- level(1-5), concept_slug TEXT UNIQUE, parent_skill_id FK, embedding vector(1024),
                   -- review_needed BOOL, status CHECK(active|unclassified|archived),
                   -- origin CHECK(resume|ontology|job_gap|resource|tree_quest|work),
                   -- state CHECK(seed|adjacent)
skill_aliases      -- id, canonical_skill_id FK, alias TEXT UNIQUE (case-insensitive index)
skill_dependencies -- id, source_skill_id FK, target_skill_id FK,
                   -- relationship CHECK(prerequisite|related|part_of|specialization|co_occurs), is_manual BOOL
skill_evidence     -- id, skill_id FK, source_type, source_id, payload JSONB; UNIQUE(skill_id, source_type, source_id)
skill_profiles     -- id, skill_id FK UNIQUE, status CHECK(active|archived|learning|mastered|untouched)
skill_trees        -- id, skill_id FK, tree_id FK
skill_project_links-- id, skill_id FK, project_id FK
```

### Other tables
```sql
coop_terms, research_topics, work_resources, work_resource_skills  -- Work page
job_applications   -- includes tailored_projects JSONB (migration 041)
job_skills         -- id, job_id FK, skill_name, is_required
ideas              -- id, content, tag, pinned, created_at
resume_profile, resume_projects  -- resume_projects columns: id, resume_id FK, name, description, tech_stack JSONB, github_url, linked_project_id FK, created_at (no `role` column)
daily_logs, daily_quest_links    -- Eisenhower Matrix
prompt_logs        -- id, command, model, prompt_version, input/output tokens, latency_ms, success
```

---

## Coding Conventions

### Rust
```rust
// Always $N placeholders — never ?N
// Prefer non-macro sqlx::query() (no offline cache needed)
let row = sqlx::query("SELECT * FROM table WHERE id = $1")
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
let value: String = row.try_get("column").map_err(|e| e.to_string())?;

// Commands return Result<T, String>; database: State<'_, Database> always last
#[tauri::command]
pub async fn my_command(param: String, database: State<'_, Database>) -> Result<MyType, String> { }

// Background fire-and-forget: tokio::spawn
// Long jobs: enqueue via JobQueue
queue.send(OrchestratorJob::RematchAllNodes { tree_id }).await?;
```

### TypeScript / React
```typescript
const result = await invoke<MyType>('command_name', { params });

// JSONB fields may come as object or string
const tasks = Array.isArray(node.tasks) ? node.tasks : JSON.parse(node.tasks ?? '[]');

// Tauri events
const unlisten = await listen<Payload>('event-name', ({ payload }) => { });
// Call unlisten() in complete handler — not in finally

// Dev-only UI
{import.meta.env.DEV && <DebugPanel />}
```

### Never Do
- Never hardcode `DATABASE_URL` or commit `.env`
- Never use dotenvy/load_env in Rust — env vars loaded by dev.sh
- Never use `?N` placeholders (Postgres uses `$N`)
- Never store JSON as TEXT (always JSONB)
- Never generate quests that are implementation tasks ("Build X", "Implement Y")
- Never add Neon-specific SQL features

---

## Common Patterns

### Add a New Tauri Command
1. Write in `*_commands.rs` with `#[tauri::command]`
2. Register in `main.rs` invoke_handler
3. Call from frontend: `invoke('command_name', { params })`

### Add a Migration
`src-tauri/migrations/NNN_description.sql` — auto-runs on startup. Never modify existing. Current highest: **047**.

### Add a New Page
1. `src/pages/NewPage.tsx`
2. Route in `App.tsx`
3. Nav link + lucide-react icon in `MainLayout.tsx`

### Enqueue a Background Job
```rust
queue.send(OrchestratorJob::RematchAllNodes { tree_id: id.clone() }).await
    .map_err(|e| e.to_string())?;
```

### Subscribe to ygg-* Events (frontend)
```typescript
const unlisten = await listen<{ treeId: string; progress: number }>('ygg-rematch-progress', ({ payload }) => {
    setProgress(payload.progress);
});
```

### Run the App
```bash
bash dev.sh
```
Unlocks Bitwarden, exports env vars, runs `npm run tauri dev`. No `.env` file needed.

---

## Environment Variables

```bash
DATABASE_URL, GROQ_API_KEY, GITHUB_TOKEN, OPENROUTER_API_KEY, YOUTUBE_API_KEY  # from Bitwarden
TREE_GEN_BASE_URL / TREE_GEN_API_KEY / TREE_GEN_MODEL=google/gemini-2.5-flash  # derived
CONCEPT_GRAPH_MODEL=google/gemini-2.5-flash, CONCEPT_GRAPH_BASE_URL / _API_KEY  # derived
```

---

## AI Generation Rules

### Tree Generation — Two-Phase (brain.rs + prompt_builders.rs + tree_persistence.rs)
- **Phase 1:** Gemini Flash extracts 8-20 concepts with prerequisites → Kahn's topological sort. Paper context (`paper_url`/`paper_pdf`) prepended (20k cap). Falls back to single-phase on failure.
- **Phase 2:** Gemini Flash generates tree using sorted concept order. Paper context appended (20k cap); per-skill expansion gets 4k cap.
- **Prompt logging:** every `call_llm` → `prompt_logs` via `log_prompt_call` (fire-and-forget). Versions: `concept_graph_v1`, `repo_profile_v1`, `prd_profile_v1`, `tree_outline_v1`, `skill_expansion_v1`. Increment on template changes.
- **Quest rules:** learning actions ONLY. ALLOWED: "Read X", "Watch Y", "Work through Z". FORBIDDEN: "Implement", "Build", "Create", "Code". Structure: 3-5 phases → 2-4 skills → 3 quests.
- **`regenerate_tree`:** reads mastered concepts from `universal_skills`, passes as already-known; diff carry-over preserves notes + completion by `concept_id`.

### Skill Extraction
- Work page: max 6 tags; include specific method AND broader domain; JSON array only
- JDs: required (max 10) + nice-to-have (max 5); separate arrays

---

## Mimir

### Embedding
- `perplexity/pplx-embed-v1-0.6b` via OpenRouter — 1024-dim, L2-normalized
- Do NOT add `"dimensions"` to request body
- Shared `reqwest::Client` via Tauri state — never create a new client per call

### Hybrid Retrieval
RRF over: (1) pgvector cosine on `mimir_embeddings`, (2) Postgres FTS on `mimir_chunks.fts_vector`. Score: `Σ 1/(60+rank)`. Top candidates → LLaMA 3.1-8b reranker → top-3 to synthesis. Pre-matched chunks from `mimir_node_links` bypass cold retrieval when `node_id` set. Every retrieval writes to `mimir_retrieval_logs`.

### GraphRAG
Before cosine search: look up concept nodes by query topic → walk `tree_edges` for prerequisites/successors → inject adjacent concept titles into Groq synthesis prompt.

### PDF Ingestion
1. Frontend sends base64 → `ingest_mimir_pdf`
2. Rust POSTs to Python scraper `/fetch-pdf` (pymupdf: TOC-aware sections, font-size fallback)
3. Stores `raw_text` + `sections_json` JSONB
4. Chunking: 400-word sliding window, 50-word overlap, block-level page provenance per chunk

### Rescraping
- `rescrape_all` / `rescrape_one`: skip YouTube URLs. Skip playlist video children.

### Background Job Queue (orchestrator.rs)
```rust
pub(crate) enum OrchestratorJob {
    RematchAllNodes { tree_id: String },
    ReembedResources { resource_ids: Vec<String> },
    InferSkillDeps,
    AutoTagResources { resource_ids: Vec<String> },
    MatchResourceToNodes { resource_id: String },   // enqueued after auto-tag
    ExtractSkillsFromResource { resource_id: String }, // enqueued after MatchResourceToNodes
    FetchTranscript { resource_id: String },
}
```
`start_worker` called in `main.rs`; `JobQueue` managed as Tauri state. Channel cap: 64.

`MatchResourceToNodes` flow: embed first chunk → HNSW ANN top-10 → in-memory rerank (same-tree -0.05, lexical -0.03) → top-5 threshold < 0.55 → upsert `mimir_node_links`.

`ExtractSkillsFromResource` flow: sample 30 diverse chunks → ANN pre-filter 50 candidate skills → gemini-3.1-flash-lite verify → write `mimir_skill_links` with `source='llm_extraction', confidence=0.9`. Never overwrites `source='manual'`.

Workers emit `ygg-*` events via `app.emit()`. Frontend `listen()` must call `unlisten()` on completion — not in `finally`.

---

## Read Models (read_models.rs)

| Command | Returns | Purpose |
|---|---|---|
| `get_active_tree_for_project` | `Option<TreeSummary>` | Active tree + completion stats |
| `get_node_chat_context` | `NodeChatContext` | Node + linked resources for Mimir |
| `get_project_tree_summary` | `ProjectTreeSummary` | Project + tree + phase stats |
| `get_skill_graph_snapshot` | `SkillGraphSnapshot` | Skills + deps + gaps |
| `get_node_neighborhood` | `NodeNeighborhood` | Prerequisites, dependents, siblings + top-3 resources each |
| `get_tree_resource_gaps` | `TreeResourceGaps` | Leaf nodes by resource match quality |
| `get_prereq_path` | `PrereqPath` | BFS from gap skill back to seed nodes |
| `get_growth_recommendations` | `Vec<GrowthTarget>` | Job-demand-weighted skill targets |
| `compute_learning_path` | `LearningPath` | Steiner-tree ordered acquisition path |
| `get_resource_study_map` | `ResourceStudyMap` | Resources ranked by checkpoint coverage |
| `get_tailored_projects` | `TailoredProjects` | Resume projects ranked by JD skill overlap |

All structs use `#[serde(rename_all = "camelCase")]`.

---

## Python Scraper (port 3002)

- `POST /fetch` — 3-tier scraper (Async → Stealthy → Dynamic), YouTube transcript detection
- `POST /fetch-pdf` — pymupdf TOC-aware extraction → `{text, pages, sections}`
- `POST /fetch-playlist` — YouTube Data API v3
- `POST /discover` — extract internal links (max 50)
- `GET /health`

No dotenv in main.py — env vars loaded by dev.sh before start. Requires `pymupdf`.

---

## Browser Extension (extension/)

Axum HTTP bridge on `127.0.0.1:1421` exposes routes accessible from the extension:

| Route | Purpose |
|---|---|
| `GET /api/health` | Ping — returns `{"status":"ok"}` |
| `POST /api/jobs` | Create job + extract skills + return tailored projects |
| `GET /api/jobs/meta` | Return distinct sources and locations for tag presets |
| `PATCH /api/jobs/:id` | Update job status and ratings |
| `POST /api/library` | Create resource + fire ingest pipeline in background |
| `GET /api/library` | List/search resources (`?q=`, `?limit=`) |
| `PATCH /api/library/:id` | Update user_notes on a resource |

**Auth:** `X-Ygg-Key` header must match `YGG_EXT_KEY` env var (default `"ygg-local-dev"`). Set in dev.sh or Bitwarden.

**POST /api/jobs body** (JSON, camelCase):
```json
{ "company": "Acme", "position": "SWE", "location": "Remote", "link": "https://...", "season": "Fall 2026", "jobDescription": "..." }
```

**Dedup:** if `link` is non-empty and `(company, position, link)` already exists, returns the existing record + tailored projects without a new insert. Migration 048 adds a partial unique index for non-null links.

**Tailored projects:** after Groq LLM extracts skills to `job_skills`, the server runs the same HashSet overlap logic as `get_tailored_projects` (read_models.rs), returns top 4 resume projects by match score.

**CORS:** allows `chrome-extension://`, `http://localhost`, `http://127.0.0.1` origins. The `X-Ygg-Key` header is explicitly listed in `allow_headers`.

**Extension build:**
```bash
cd extension && npm install && npm run build   # outputs dist/
```
Load as unpacked extension from the `extension/` directory. Background service worker flushes the offline queue every 5 minutes via a Chrome alarm.

---

## Build Order

1. DB migration (if new tables)
2. Rust commands
3. Register in `main.rs`
4. Frontend

Never start frontend before backend commands exist.
