# Yggdrasil — Claude Code Development Guide

## What This App Is

A personal learning OS. Build a project, paste the repo, Yggdrasil generates a skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

Read `PRD.md` for the full vision. This file is your technical bible.

---

## Current State (v2.6)

**All core features shipped (Phases 0-6 complete):**
- Project creation, listing, edit, delete
- AI tree generation from PRD text and GitHub repo URL (two-phase: concept graph → tree)
- `analyze_repo` accepts optional `paper_url: Option<String>` and `paper_pdf: Option<String>`; paper context is injected into Phase 1 concept graph extraction as well as the Phase 2 repo profile + per-skill expansion calls
- Custom canvas-based organic tree renderer — polar coordinate layout via `layoutTree()` (replaces the prior L-system); `NodePlacement[]` shape is unchanged so downstream draw code is untouched
- Checkpoint completion, notes, unlock mechanics, progress cascade
- Mimir: URL/PDF/text ingestion, TOC-aware chunking, 1024-dim embeddings, pgvector storage — fully in Rust, no sidecar
- Mimir chat: hybrid retrieval (RRF over cosine + FTS), Groq LLaMA 3.3-70b synthesis, reranking, source citations (section title + page range)
- Mimir chat: persistent session memory per (tree_id, node_id) — `mimir_chat_sessions` + `mimir_chat_messages` tables; last 10 messages injected into context on each query
- Mimir chat: pre-matched chunks loaded eagerly from `mimir_node_links` (bypasses cold retrieval when node context is set)
- Mimir chat: full tree context awareness — `mimir_chat` receives `project_name` + `tree_name`; section 5 builds a phase-breakdown block (overall %, per-phase checkpoints done/total, library coverage) via inline SQL; section 6 branches on `node_is_active` for checkpoint-tutor vs. tree-only mode
- Mimir chat: GraphRAG traversal — concept-level graph walk before cosine search; prerequisite/successor context injected into synthesis prompt
- Retrieval logging: every `mimir_chat` retrieval writes a row to `mimir_retrieval_logs` with full stats (k, threshold, candidate counts, hybrid metrics, sources JSONB)
- Auto-matching resources to checkpoint nodes on tree generation — `mimir_node_links` now persists `matched_chunk_id`, `matched_section_title`, `matched_page_start`, `matched_page_end` so node panels show exact citations
- Resource library: search, tag filters, type/sort dropdowns, auto-tagging, completion tracking, per-video playlist completion
- Resource gap finder: surfaces Mimir resources most relevant to unmastered checkpoints (agentic suggestions)
- Study Map: `get_resource_study_map` read-model command — ranks resources by coverage of unlocked checkpoints, groups matched checkpoints by section for reading-order guidance, optional frontier toggle, per-project filter
- Work page: co-op tracker + D3 force galaxy visualization + AI skill extraction
- Jobs page: kanban board + JD analysis + skill demand analytics + follow-up tracker; dynamic season selection (term + year picker); tailored projects panel — `get_tailored_projects` ranks resume projects by skill overlap (HashSet match) against required + nice-to-have JD skills; `save_tailored_projects` persists user selection to `job_applications.tailored_projects` JSONB (migration 041)
- Resume page: paste/upload resume, AI parsing, skill pre-population; parsed projects persisted to `resume_projects` table (id, name, description, tech_stack JSONB, role) — handles NULL tech_stack defensively
- Ideas page: scratchpad with tags, pin, promote to project
- Universal Skill Tree V2: canvas-rendered radial tree, domain classification, skill-to-concept edges, gap analysis; domain-agnostic schema (concept/technical/soft/practical/domain/unclassified kinds); skill canonicalization via `skill_aliases` table + merge UI; human-curation columns (`review_needed`, `status`); first-class `skill_evidence` rows; `origin` + `state` columns (migration 037) — `origin` ∈ (resume|ontology|job_gap|resource|tree_quest|work), `state` ∈ (seed|adjacent)
- Daily Eisenhower Matrix: 2x2 quadrant triage for quests + free-form tasks, day navigation
- Tree export as ZIP
- Tree versioning: `tree_versions` table (migration 034); `concept_id`/`concept_slug` stable identity columns; `regenerate_tree` carries mastered concepts forward via KG→tree bridge and runs diff-based state carry-over
- Background async job queue (`orchestrator.rs`) — `JobQueue` struct with tokio mpsc channel; worker processes RematchAllNodes, ReembedResources, InferSkillDeps, AutoTagResources off the UI thread; emits `ygg-*` Tauri events for progress
- Read-model helpers (`read_models.rs`) — `get_active_tree_for_project`, `get_node_chat_context`, `get_project_tree_summary`, `get_skill_graph_snapshot`, `get_node_neighborhood`, `get_resource_study_map`, `get_tailored_projects`; purpose-built query functions replacing ad-hoc frontend joins
- Resource reading progress: `resource_reading_progress` table (migration 040) — per-section completion tracked by `(resource_id, section_title)` with optional `page_start`/`page_end`; commands `mark_section_read`, `mark_sections_read_up_to`, `get_reading_progress`
- Skills graph redesign: canvas lane layout (domain-grouped columns), hover highlighting (focused subgraph), zoom-adaptive labels, legend, double-click to zoom, fit-all control
- Prompt/model version logging — `prompt_logs` table; `log_prompt_call` fire-and-forget helper in `brain.rs`; all `call_llm` sites instrumented (concept_graph, repo_profile, prd_profile, tree_outline, skill_expansion, mimir_chat, mimir_rerank, auto_tag, skill_deps); `get_prompt_stats` Tauri command + dev-only "Model logs" tab in MimirChat.tsx
- Shared `reqwest::Client` managed as Tauri state — injected into all commands that call external APIs (brain, mimir modules, orchestrator)
- God module splits: `brain.rs` → `llm_client.rs` + `github.rs` + `prompt_builders.rs` + `tree_persistence.rs`; `mimir.rs` → `mimir_ingest.rs` + `mimir_retrieval.rs` + `mimir_tags.rs` + `mimir_manage.rs`
- Postgres on Neon with pgvector
- All migrations (001-041) run automatically on startup
- HNSW index on `tree_nodes.title_embedding vector(1024)` (migration 036) — used for async resource→node ANN matching
- Background async job queue now includes `MatchResourceToNodes { resource_id: String }` — enqueued by `on_resource_ingested_async` after auto-tag completes; worker calls `run_match_resource_to_nodes` with in-memory reranking (same-tree boost -0.05, lexical overlap boost -0.03; top-5, threshold < 0.55)
- `get_node_neighborhood` command in `read_models.rs` — returns `NodeNeighborhood { prerequisites, dependents, siblings }` each as `Vec<NeighborNode>`; traverses `concept_slug → universal_skills → skill_dependencies → tree_nodes`; top-3 resources per neighbor via `mimir_node_links`
- YouTube transcript metadata: `transcript_source` / `transcript_mode` / `transcript_chars` columns on `mimir_resources` (migration 038); async `transcript_jobs` retry queue (migration 039)

---

## Project Structure

```
├── src/
│   ├── components/
│   │   ├── MimirChat.tsx              # RAG chat sidebar (hybrid retrieval, session memory, model logs tab)
│   │   ├── ProjectInput.tsx           # Project creation form
│   │   └── YggdrasilTree.tsx          # Canvas tree renderer + NodePanel
│   ├── contexts/
│   │   └── MimirContext.tsx            # Shared treeId/nodeTitle for Mimir chat
│   ├── pages/
│   │   ├── Homepage.tsx               # Project list + creation
│   │   ├── ProjectTreePage.tsx        # Tree canvas + PRD/GitHub generation
│   │   ├── TreesPage.tsx              # All trees list
│   │   ├── DailyPage.tsx              # Eisenhower Matrix (replaces QuestsPage)
│   │   ├── ResourcesPage.tsx          # Mimir resource library
│   │   ├── WorkPage.tsx               # Co-op galaxy
│   │   ├── JobsPage.tsx               # Job tracker
│   │   ├── IdeasPage.tsx              # Ideas scratchpad
│   │   ├── ResumePage.tsx             # Resume parser
│   │   └── SkillsPage.tsx             # Universal Skill Tree galaxy
│   ├── layouts/
│   │   └── MainLayout.tsx             # Sidebar navigation + Mimir chat toggle
│   └── types.ts
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                    # Command registration — register ALL new commands here
│   │   ├── commands.rs                # Project CRUD
│   │   ├── tree_commands.rs           # Tree/node CRUD + quest completion + unlock mechanics + tree versioning
│   │   ├── brain.rs                   # AI generation orchestration + log_prompt_call helper (thin coordinator)
│   │   ├── llm_client.rs              # Shared HTTP client + call_llm helper (split from brain.rs)
│   │   ├── github.rs                  # GitHub REST API fetching (split from brain.rs)
│   │   ├── prompt_builders.rs         # Prompt template construction for all LLM calls (split from brain.rs)
│   │   ├── tree_persistence.rs        # Tree/node DB writes after generation (split from brain.rs)
│   │   ├── mimir_ingest.rs            # URL/PDF/text ingest pipeline, chunking, embeddings (split from mimir.rs)
│   │   ├── mimir_retrieval.rs         # Hybrid RAG, GraphRAG traversal, session memory, tree context (split from mimir.rs)
│   │   ├── mimir_tags.rs              # Auto-tagging, tag filter helpers (split from mimir.rs)
│   │   ├── mimir_manage.rs            # Rescrape, re-embed, resource CRUD, completion, reading progress (mark_section_read / mark_sections_read_up_to / get_reading_progress, migration 040) (split from mimir.rs)
│   │   ├── orchestrator.rs            # Background job queue (JobQueue + start_worker) + cascade handlers + ygg-* events; MatchResourceToNodes variant
│   │   ├── read_models.rs             # Purpose-built read-model Tauri commands (11 helpers: active tree, node chat context, tree summary, skill graph snapshot, node neighborhood, resource gaps, prereq path, growth recommendations, learning path, study map, tailored projects)
│   │   ├── work_commands.rs           # Co-op/topic/resource/skill commands
│   │   ├── job_commands.rs            # Job application commands + save_tailored_projects (tailored_projects JSONB on job_applications, migration 041)
│   │   ├── idea_commands.rs           # Ideas CRUD + promote to project
│   │   ├── resume_commands.rs         # Resume parsing + profile management
│   │   ├── skill_commands.rs          # Universal skill sync + dependencies + gaps + aliases + merge
│   │   ├── daily_commands.rs          # Daily Eisenhower Matrix commands
│   │   ├── export_commands.rs         # Tree ZIP export
│   │   └── database.rs               # PgPool connection + migrations
│   ├── migrations/                    # Auto-run on startup, sequential (001-041)
│   └── capabilities/
│       └── default.json               # Tauri 2 capability grants (includes core:event:allow-listen)
├── scraper/                           # Python FastAPI scraper (port 3002)
│   ├── main.py                        # /fetch, /fetch-pdf, /fetch-playlist, /discover, /health
│   └── requirements.txt               # includes pymupdf
└── dev.sh                             # Bitwarden-based env loader + launch
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Main backend | Rust + sqlx |
| Database | Postgres on Neon (pgvector enabled) |
| AI generation | OpenRouter (Gemini Flash for both tree gen + concept graphs) + Groq (LLaMA for chat/skills) |
| Tree renderer | Custom HTML Canvas (tapered filled branches, polar layout — trunk/boughs/limbs/twigs + atmospheric roots) |
| Work/Skills galaxy | D3 force simulation |
| Mimir | Native Rust split across mimir_ingest / mimir_retrieval / mimir_tags / mimir_manage — no sidecar |
| Scraper | Python FastAPI (port 3002) — URL scraping, PDF extraction (pymupdf), playlist |
| Embeddings | Perplexity pplx-embed-v1-0.6b (1024 dimensions) via OpenRouter |
| Vector search | pgvector on Neon — vector(1024) column |
| Hybrid retrieval | RRF over pgvector cosine + Postgres FTS (tsvector GIN index on mimir_chunks) |
| Background jobs | tokio mpsc channel in orchestrator.rs — JobQueue managed Tauri state |
| GitHub | REST API via reqwest |

---

## Database Schema

### Core tables
```sql
projects          -- id, name, description, discipline_ids JSONB, skill_ids JSONB, status, progress,
                  -- active_tree_id TEXT FK trees(id), created_at
trees             -- id, project_id FK, name, version INT, parent_tree_id FK, archived_at, created_at
tree_nodes        -- id, tree_id FK, parent_id FK, type (trunk/branch/leaf),
                  -- title, description, progress, tasks JSONB, resources JSONB,
                  -- x, y, order_index, is_locked,
                  -- concept_id TEXT,              ← stable identity across tree versions
                  -- concept_slug TEXT,            ← URL-safe slug for same
                  -- title_embedding vector(1024)  ← HNSW index (migration 036) for ANN resource matching
                  --
                  -- Node types:
                  --   trunk  = phase (top-level grouping, never locked)
                  --   branch = skill  (can be is_locked=true; first skill per phase starts unlocked,
                  --                    subsequent skills locked until predecessor reaches progress=100)
                  --   leaf   = checkpoint (always is_locked=false; visibility gated by parent branch lock)
                  --
                  -- progress on branch/trunk nodes is the average of their children's progress,
                  -- cascaded upward by recalculate_tree_progress_inner on every checkpoint completion.
                  --
                  -- Unlock mechanic (recalculate_unlocks_inner in orchestrator.rs):
                  --   1. First branch under each trunk is always unlocked.
                  --   2. When a branch reaches progress=100, the next sibling branch is unlocked.
                  --   This fires eagerly on checkpoint completion — there is no steady-state where
                  --   a branch is locked with a completed predecessor (frontier windows collapse immediately).
tree_edges        -- id, tree_id FK, source_node_id FK, target_node_id FK
disciplines       -- id, name, description, color
```

### Mimir tables
```sql
mimir_resources   -- id, title, url, type, status, user_notes, content_hash, parent_id,
                  -- tags TEXT[], is_completed, created_at, updated_at,
                  -- raw_text TEXT,               ← full extracted text (PDFs only)
                  -- sections_json JSONB          ← TOC-aware section structure (PDFs only)
                  -- transcript_source TEXT       ← youtube_transcript_api | youtubetranscript_dev | metadata_only (migration 038)
                  -- transcript_mode TEXT         ← captions | asr | none (migration 038)
                  -- transcript_chars INT         ← length of fetched transcript (migration 038)
mimir_chunks      -- id, resource_id FK, content, chunk_index,
                  -- section_title TEXT,     ← heading this chunk falls under
                  -- page_start INT,         ← first page of chunk content
                  -- page_end INT,           ← last page of chunk content
                  -- fts_vector TSVECTOR GENERATED ← auto-maintained FTS column (GIN indexed)
mimir_embeddings  -- id, chunk_id FK, embedding vector(1024)
mimir_node_links  -- id, resource_id FK, node_id FK, relevance_score,
                  -- matched_chunk_id FK,       ← best-matching chunk from the cosine search
                  -- matched_section_title,     ← heading that chunk falls under
                  -- matched_page_start,        ← first page of the matched chunk
                  -- matched_page_end           ← last page of the matched chunk
mimir_chat_sessions    -- id, tree_id FK, node_id FK, created_at, updated_at; UNIQUE(tree_id, node_id)
mimir_chat_messages    -- id, session_id FK, role (user|assistant), content, sources JSONB, created_at
mimir_retrieval_logs   -- id, query, node_id, tree_id, top_k, threshold,
                       -- candidates_before_rerank, candidates_after_rerank,
                       -- prematch_chunks_used, rerank_fallback_used,
                       -- lexical_candidates, hybrid_candidates_merged,
                       -- sources JSONB, created_at
transcript_jobs        -- id, resource_id FK, status (pending|processing|done|failed|skipped),
                       -- attempts INT, last_error TEXT, next_retry_at TIMESTAMPTZ,
                       -- created_at, updated_at
                       -- (migration 039) async retry queue for YouTube transcript fetches
resource_reading_progress -- id, resource_id FK, section_title, page_start, page_end,
                          -- completed_at, UNIQUE(resource_id, section_title)
                          -- (migration 040) per-section read tracking for PDFs / structured resources
```

### Work tables
```sql
coop_terms           -- id, company, role, start_date, end_date, color
research_topics      -- id, coop_id FK, name
work_resources       -- id, topic_id FK, title, url, notes, completed
work_resource_skills -- id, resource_id FK, skill_name, tree_id FK (nullable)
```

### Jobs tables
```sql
job_applications  -- id, company, position, location, source, status,
                  -- date_applied, date_follow_up, job_description, link, notes,
                  -- rating_location, rating_alignment, rating_salary, rating_role,
                  -- season, created_at,
                  -- tailored_projects JSONB         ← saved resume-project recommendation (migration 041)
job_skills        -- id, job_id FK, skill_name, is_required
```

### Ideas
```sql
ideas             -- id, content, tag, pinned, created_at
```

### Resume
```sql
resume_profile    -- id, full_text, parsed JSONB, created_at
resume_projects   -- id, name, description, tech_stack JSONB (nullable), role, created_at
                  -- (migration 008) — populated from resume parsing; queried by get_tailored_projects
```

### Universal Skills (domain-agnostic v2)
```sql
skill_domains        -- id, name UNIQUE, description, parent_domain_id FK, color, created_at
                     -- seeded with: Engineering, Mathematics, Science, Computing,
                     --              Communication, Design, Business, Research

universal_skills     -- id, name, domain TEXT (legacy), domain_id FK skill_domains,
                     -- kind TEXT CHECK(concept|technical|soft|practical|domain|unclassified),
                     -- level (1-5), evidence JSONB (legacy), last_updated,
                     -- concept_slug TEXT UNIQUE, parent_skill_id FK,
                     -- review_needed BOOLEAN DEFAULT false,
                     -- status TEXT CHECK(active|unclassified|archived),
                     -- origin TEXT NOT NULL DEFAULT 'tree_quest'
                     --   CHECK(origin IN ('resume','ontology','job_gap','resource','tree_quest','work'))
                     -- state TEXT NOT NULL DEFAULT 'adjacent'
                     --   CHECK(state IN ('seed','adjacent'))
                     -- (migration 037 — backfilled from evidence JSONB)

skill_aliases        -- id, canonical_skill_id FK, alias TEXT UNIQUE, created_at
                     -- alias lookups are case-insensitive (idx on LOWER(alias))

skill_dependencies   -- id, source_skill_id FK, target_skill_id FK,
                     -- relationship CHECK(prerequisite|related|part_of|specialization|co_occurs),
                     -- is_manual BOOLEAN DEFAULT false

skill_evidence       -- id, skill_id FK, source_type CHECK(resume|tree_quest|work_resource|
                     --   manual|mimir_resource|job_demand|external),
                     -- source_id TEXT, payload JSONB, recorded_at;
                     -- UNIQUE(skill_id, source_type, COALESCE(source_id,''))
```

### Daily Matrix
```sql
daily_logs           -- id, date (UNIQUE), notes, created_at
daily_quest_links    -- id, date, node_id FK tree_nodes(id) ON DELETE CASCADE,
                     -- free_text, quadrant (do/schedule/delegate/eliminate),
                     -- sort_order, added_at, UNIQUE(date, node_id)
```

### Observability tables
```sql
prompt_logs      -- id, command, model, prompt_version, input_tokens, output_tokens,
                 -- latency_ms, success, error, metadata JSONB, created_at
                 -- indexed on (command, created_at DESC) and (model, created_at DESC)
```

---

## Coding Conventions

### Rust
```rust
// Always $N placeholders — never ?N
sqlx::query("SELECT * FROM projects WHERE id = $1")

// Prefer non-macro sqlx::query() to avoid offline cache issues
// Use .bind() chains, Row trait for extraction
let row = sqlx::query("SELECT * FROM table WHERE id = $1")
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
let value: String = row.try_get("column").map_err(|e| e.to_string())?;

// Commands return Result<T, String>
#[tauri::command]
pub async fn my_command(
    param: String,
    database: State<'_, Database>,  // always last param
) -> Result<MyType, String> {
    // ...
}

// Background work: fire-and-forget with tokio::spawn
tokio::spawn(async move {
    // best-effort; errors logged, never propagated
});

// Long-running jobs: enqueue via JobQueue (managed Tauri state)
queue.send(OrchestratorJob::RematchAllNodes { tree_id }).await?;
```

### TypeScript / React
```typescript
// Always type invoke returns
const result = await invoke<MyType>('command_name', { params });

// JSONB fields come back as objects — handle both cases
const tasks = Array.isArray(node.tasks)
  ? node.tasks
  : JSON.parse(node.tasks ?? '[]');

// Loading states on all async operations
const [loading, setLoading] = useState(false);

// Use listen() from @tauri-apps/api/event for Tauri events
import { listen } from '@tauri-apps/api/event';
const unlisten = await listen<PayloadType>('event-name', ({ payload }) => { ... });
// Always call unlisten() when done — ideally inside the complete handler, not finally

// Dev-only UI: guard with import.meta.env.DEV
{import.meta.env.DEV && <DebugPanel />}
```

### Never Do
- Never hardcode `DATABASE_URL`
- Never commit `.env` files
- Never use dotenvy/load_env in Rust — env vars are loaded by dev.sh before the process starts
- Never use `?N` placeholders (Postgres uses `$N`)
- Never store JSON as TEXT (always JSONB)
- Never generate quests that are implementation tasks ("Build X", "Implement Y")
- Never add Neon-specific SQL features (keep it portable)

---

## Common Patterns

### Call Rust from React
```typescript
import { invoke } from '@tauri-apps/api/core';
const result = await invoke<ReturnType>('command_name', { paramName: value });
```

### Add a New Tauri Command
1. Write function in appropriate `*_commands.rs` with `#[tauri::command]`
2. Register in `main.rs` invoke_handler
3. Call from frontend with `invoke('command_name', { params })`

### Add a Migration
Create `src-tauri/migrations/NNN_description.sql` — runs automatically on startup. Never modify existing migrations. Current highest: **041**.

### Add a New Page
1. Create `src/pages/NewPage.tsx`
2. Add route in `App.tsx`
3. Add nav link with lucide-react icon in `MainLayout.tsx`

### Enqueue a Background Job
```rust
// From a Tauri command handler:
let queue = queue_state.inner();
queue.send(OrchestratorJob::RematchAllNodes { tree_id: id.clone() }).await
    .map_err(|e| e.to_string())?;
```
The worker in `orchestrator.rs` picks it up, emits `ygg-*` events, and never crashes on error.

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
This unlocks Bitwarden, exports all env vars into the process, and runs `npm run tauri dev`.
The Rust backend and Python scraper both read env vars from the process environment — no `.env` file is required at runtime.

---

## Environment Variables

All vars are exported by `dev.sh` from Bitwarden before the process starts. No `.env` file is needed at runtime.

```bash
# Fetched from Bitwarden by dev.sh
DATABASE_URL=postgresql://...neon.tech/neondb?sslmode=require
GROQ_API_KEY=gsk_...
GITHUB_TOKEN=ghp_...
OPENROUTER_API_KEY=sk-or-...
YOUTUBE_API_KEY=...

# Derived in dev.sh
TREE_GEN_BASE_URL=https://openrouter.ai/api/v1/chat/completions
TREE_GEN_API_KEY=$OPENROUTER_API_KEY
TREE_GEN_MODEL=google/gemini-2.5-flash
CONCEPT_GRAPH_MODEL=google/gemini-2.5-flash
CONCEPT_GRAPH_BASE_URL=https://openrouter.ai/api/v1/chat/completions
CONCEPT_GRAPH_API_KEY=$OPENROUTER_API_KEY
MIMIR_PORT=3001   # unused — Mimir is now native Rust, no port
```

---

## AI Generation Rules

### Tree generation (brain.rs + prompt_builders.rs + tree_persistence.rs) — Two-Phase Approach
Tree generation uses a two-phase pipeline via OpenRouter:

**Phase 1: Concept Graph Extraction**
- Uses Gemini Flash via OpenRouter
- Extracts 8-20 concepts with prerequisite relationships from the project context
- Concepts are topologically sorted using Kahn's algorithm (foundational first, advanced last)
- If `paper_url` or `paper_pdf` was supplied to `analyze_repo`, the paper text is prepended to the user prompt as a `## PAPER / THEORY CONTEXT` section (20k char cap) so the LLM prioritizes concepts that appear in both the paper and the codebase
- If extraction fails, falls back to single-phase generation gracefully

**Phase 2: Tree Generation with Graph Context**
- Uses Gemini Flash via OpenRouter
- The sorted concept dependency order is prepended to the user prompt
- The tree generation LLM uses this to determine phase ordering, skill sequencing, and quest progression
- Every quest should connect back to a concept in the dependency graph
- Paper context (when present) is appended to the Phase 2 context (20k cap) and to each skill's `expansion_context` (4k cap per call to bound fanout cost)

**Prompt version logging**
Every `call_llm` invocation records a row to `prompt_logs` via `log_prompt_call` (fire-and-forget `tokio::spawn`). Prompt versions: `concept_graph_v1`, `repo_profile_v1`, `prd_profile_v1`, `tree_outline_v1`, `skill_expansion_v1`. Increment the version constant when the prompt template changes.

**Rules:**
- Quests must be learning actions ONLY
- ALLOWED: "Read Chapter X", "Watch lecture on Y", "Work through exercises Z"
- FORBIDDEN: "Implement X", "Build Y", "Create Z", "Code W"
- Structure: 3-5 phases → 2-4 skills each → 3 quests each
- GitHub repo analysis: fetch README + dependency files + directory structure + key source files (via `github.rs`)
- Paper input: arXiv URLs are auto-rewritten `/abs/` → `/pdf/`; other URLs are scraped via the Python scraper `/fetch`; base64 PDFs go directly to `/fetch-pdf`; all paper handling is non-fatal (failure logs a warning and proceeds without paper context)
- `regenerate_tree`: reads mastered concepts from `universal_skills` (via KG→tree bridge), passes them as already-known context to Phase 1 so regenerated trees skip mastered prerequisites; diff-based carry-over preserves notes + completion state by matching `concept_id`

### Skill extraction (work_commands.rs + job_commands.rs)
- Max 6 tags per resource (Work page)
- Include specific method (e.g. "LDA") AND broader domain (e.g. "Topic Modelling")
- Return ONLY JSON array — no preamble
- For JDs: separate required vs nice-to-have, max 10 required + 5 nice-to-have

---

## Mimir (Native Rust)

Mimir is fully implemented across `mimir_ingest.rs`, `mimir_retrieval.rs`, `mimir_tags.rs`, and `mimir_manage.rs` (split from the former monolithic `mimir.rs`). There is no Node.js sidecar.

### Embedding Pipeline
- Model: `perplexity/pplx-embed-v1-0.6b` via OpenRouter (`https://openrouter.ai/api/v1/embeddings`)
- Output: 1024-dim float32 vector, L2-normalized
- Storage: pgvector `vector(1024)` column (migration 017 widened from 384)
- Shared `reqwest::Client` is managed as Tauri state (injected into all commands that call external APIs); `mimir_ingest` receives it via Tauri state injection — do NOT create a new client per call
- Do NOT add `"dimensions"` to the request body — pplx-embed does not support MRL truncation via API

### Hybrid Retrieval (RAG)
Mimir chat (`mimir_retrieval.rs`) uses Reciprocal Rank Fusion (RRF) over two retrieval lanes:
1. **Cosine (vector)** — pgvector similarity search on `mimir_embeddings`
2. **FTS (lexical)** — Postgres `tsvector` full-text search on `mimir_chunks.fts_vector` (GIN index, migration 030)

RRF score: `Σ 1 / (60 + rank_i)` across lanes. Top candidates go to the LLaMA 3.1-8b reranker; top-3 are passed to synthesis.

Pre-matched chunks (from `mimir_node_links`) are injected first when a `node_id` is provided, bypassing cold retrieval for the current checkpoint context.

Every retrieval call writes a row to `mimir_retrieval_logs` (migration 028 + 030 columns).

### GraphRAG Traversal
Before cosine search, `mimir_chat` looks up concept nodes in `tree_nodes` matching the query topic, walks prerequisite and successor edges in `tree_edges`, and injects adjacent concept titles + descriptions as graph context into the Groq synthesis prompt. This produces responses that reference where the concept sits in the learning graph ("You'll need Chain Rule first…").

### Tree Context Awareness
`mimir_chat` receives `project_name: Option<String>` and `tree_name: Option<String>` from the frontend. Section 5 of the system prompt is a phase-breakdown block built from three inline SQL queries (overall progress %, per-trunk-node checkpoint counts, matched resource count). Section 6 branches on `node_is_active`:
- **Node active** → checkpoint-tutor mode: condensed progress line + full tutor block + tree block
- **No node** → tree-only mode: "viewing your learning tree for '…'" guidance + tree block

### Chat Session Memory
Sessions are persisted in `mimir_chat_sessions` (unique on `(tree_id, node_id)`) and `mimir_chat_messages`. The last 10 messages of the current session are injected into the Groq context on every query. `get_chat_session` and `clear_chat_session` Tauri commands manage lifecycle.

### PDF Ingestion Pipeline
1. Frontend sends base64 PDF to `ingest_mimir_pdf` Tauri command
2. Rust computes SHA256 for dedup, then POSTs base64 to Python scraper `POST /fetch-pdf`
3. Python scraper (pymupdf / fitz):
   - Tries `doc.get_toc()` first — if ≥ 2 entries, uses TOC as authoritative section structure
   - Falls back to font-size heuristic heading detection if TOC is absent
   - Returns `{ text, pages, sections: [{title, heading_level, page_start, page_end, blocks}] }`
4. Rust stores: `raw_text` (full text), `sections_json` (JSONB structure) on the resource row
5. If `section_count > 1`: `store_sections_and_embeddings()` — 400-word sliding window with 50-word overlap, block-level page provenance per chunk, `section_title` set on every chunk
6. If flat fallback: `store_chunks_and_embeddings()` — standard chunking, no section metadata

### Re-embedding PDFs
`reembed_pdfs` command re-embeds all PDF resources from stored data without re-uploading:
- Prefers `sections_json` → re-chunks with section structure preserved
- Falls back to `raw_text` → flat chunking (warns that section structure is lost)
- Skips if neither is present (pre-migration PDFs)

### Rescraping
- `rescrape_all` excludes YouTube URLs (`youtube.com/watch`, `youtu.be/`) at the SQL level
- `rescrape_one` also checks and skips YouTube videos with a log message
- Playlist video children (type=webpage with parent type=text) are excluded from bulk rescrape

### RAG Chat Sources
`MimirChatSource` includes:
- `sectionTitle: string | null` — heading the matched chunk falls under
- `pageStart: number | null` — first page of the chunk
- `pageEnd: number | null` — last page of the chunk

### Tauri Capabilities
`src-tauri/capabilities/default.json` grants:
- `core:default` — all standard Tauri core APIs
- `core:event:default`, `core:event:allow-listen`, `core:event:allow-unlisten`, `core:event:allow-emit` — required for `listen()` from frontend (used by rescrape-progress and `ygg-*` events)
- `opener:default`

### Known Limitations
- Scanned/image-based PDFs fail (no OCR) — user must paste text instead
- YouTube video children cannot be re-scraped (transcript was fetched at ingest time)

---

## Orchestrator & Background Jobs

`src-tauri/src/orchestrator.rs` owns all long-running async work.

### JobQueue
```rust
pub(crate) enum OrchestratorJob {
    RematchAllNodes { tree_id: String },
    ReembedResources { resource_ids: Vec<String> },
    InferSkillDeps,
    RescrapeResources { resource_ids: Vec<String> },   // currently unused
    AutoTagResources { resource_ids: Vec<String> },
    MatchResourceToNodes { resource_id: String },      // enqueued after ingest auto-tag completes
    FetchTranscript { resource_id: String },           // enqueued by periodic poller for YouTube transcript retry jobs
}
```
`start_worker(pool, app_handle)` is called in `main.rs` setup; the returned `JobQueue` is managed as Tauri state. Channel capacity: 64.

`MatchResourceToNodes` flow: `on_resource_ingested_async` awaits `auto_tag_single` → enqueues job → worker calls `run_match_resource_to_nodes` → embeds first chunk → ANN top-10 via HNSW on `title_embedding` → in-memory rerank (same-tree boost -0.05, lexical overlap -0.03) → sort ascending → top-5, threshold < 0.55 → upsert `mimir_node_links`.

### ygg-* Events emitted
Workers emit progress/completion events via `app.emit()`. Frontend components subscribe with `listen()` and must call `unlisten()` on completion — not in `finally`.

---

## Read Models

`src-tauri/src/read_models.rs` exposes eleven purpose-built Tauri commands:

| Command | Returns | Purpose |
|---|---|---|
| `get_active_tree_for_project` | `Option<TreeSummary>` | Active tree + completion stats for a project |
| `get_node_chat_context` | `NodeChatContext` | Node title + description + linked resources for Mimir context |
| `get_project_tree_summary` | `ProjectTreeSummary` | Full project + tree + phase completion stats |
| `get_skill_graph_snapshot` | `SkillGraphSnapshot` | Skills + dependencies + gaps in one round trip |
| `get_node_neighborhood` | `NodeNeighborhood` | Prerequisites, dependents, siblings for a leaf node — each with top-3 resources and skill level |
| `get_tree_resource_gaps` | `TreeResourceGaps` | Leaf nodes grouped by resource match quality (green/weak/none) for the gap finder UI |
| `get_prereq_path` | `PrereqPath` | BFS walk from a gap skill backward through prerequisite edges to current seed nodes |
| `get_growth_recommendations` | `Vec<GrowthTarget>` | Job-demand-weighted skill targets; optional season filter; ranks by gap × demand |
| `compute_learning_path` | `LearningPath` | Steiner-tree-style ordered acquisition path across all growth targets; feeds Daily Matrix |
| `get_resource_study_map` | `ResourceStudyMap` | Resources ranked by checkpoint coverage; grouped matched checkpoints by section; optional frontier toggle + per-project filter |
| `get_tailored_projects` | `TailoredProjects` | Ranks `resume_projects` by skill overlap (HashSet match against `job_skills` required + nice-to-have); handles NULL tech_stack; used by TailoredProjectsPanel in JobsPage |

All structs use `#[serde(rename_all = "camelCase")]`.

### get_node_neighborhood
Query logic:
- Siblings: `WHERE parent_id = $1 AND id != $2 AND type = 'leaf' ORDER BY order_index`
- Resolve `concept_slug` → `universal_skills.id` for the focal node
- Prerequisites: `skill_dependencies WHERE source_skill_id = focal AND relationship IN ('prerequisite','part_of')` → join `universal_skills → tree_nodes` via `concept_slug`
- Dependents: `skill_dependencies WHERE target_skill_id = focal` → same join direction reversed
- Batch skill levels: collect all `concept_slug`s from all three sets → single `ANY($1)` query
- Resources per neighbor: `mimir_node_links JOIN mimir_resources LIMIT 3` per node

---

## Python Scraper (port 3002)

### Endpoints
- `POST /fetch` — scrape URL with 3-tier fetcher (AsyncFetcher → StealthyFetcher → DynamicFetcher), YouTube transcript detection
- `POST /fetch-pdf` — pymupdf PDF extraction with TOC-aware section detection, returns `{text, pages, sections}`
- `POST /fetch-playlist` — YouTube Data API v3 playlist metadata (requires `YOUTUBE_API_KEY`)
- `POST /discover` — extract internal links from a page (max 50)
- `GET /health` — liveness check

### Notes
- env vars are loaded by dev.sh before the scraper process starts — no dotenv call inside main.py
- `pymupdf` (`fitz`) is required — run `pip install pymupdf` or install from requirements.txt

---

## Build Order for New Features

Always:
1. DB migration first (if new tables needed)
2. Rust commands second
3. Register in main.rs
4. Frontend last

Never start frontend before backend commands exist.
