# Yggdrasil — Claude Code Development Guide

## What This App Is

A personal learning OS. Build a project, paste the repo, Yggdrasil generates a skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

Read `PRD.md` for the full vision. This file is your technical bible.

---

## Current State (v2.0)

**All core features shipped (Phases 0-5 complete):**
- Project creation, listing, edit, delete
- AI tree generation from PRD text and GitHub repo URL (two-phase: concept graph → tree)
- Custom canvas-based organic tree renderer (L-system branches, node ornaments at tips)
- Quest tracking with checkpoint completion, notes, unlock mechanics, progress cascade
- Mimir sidecar: URL/PDF/text ingestion, chunking, embeddings, pgvector storage
- Mimir chat: RAG assistant with tree-aware system prompt, reranking, source citations
- Auto-matching resources to quest nodes on tree generation
- Resource library: search, tag filters, type/sort dropdowns, auto-tagging, completion tracking
- Work page: co-op tracker + D3 force galaxy visualization + AI skill extraction
- Jobs page: kanban board + JD analysis + skill demand analytics + follow-up tracker
- Resume page: paste/upload resume, AI parsing, skill pre-population
- Ideas page: scratchpad with tags, pin, promote to project
- Universal Skill Tree: D3 force galaxy, skill dependencies, gap analysis, multi-source sync
- Daily Eisenhower Matrix: 2x2 quadrant triage for quests + free-form tasks, day navigation
- Tree export as ZIP
- Postgres on Neon with pgvector
- All migrations (001-016) run automatically on startup

---

## Project Structure

```
├── src/
│   ├── components/
│   │   ├── MimirChat.tsx              # RAG chat sidebar
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
│   │   ├── tree_commands.rs           # Tree/node CRUD + quest completion + unlock mechanics
│   │   ├── brain.rs                   # Groq AI generation + GitHub repo analysis
│   │   ├── mimir.rs                   # Mimir resource CRUD, tags, completion, chat proxy
│   │   ├── work_commands.rs           # Co-op/topic/resource/skill commands
│   │   ├── job_commands.rs            # Job application commands
│   │   ├── idea_commands.rs           # Ideas CRUD + promote to project
│   │   ├── resume_commands.rs         # Resume parsing + profile management
│   │   ├── skill_commands.rs          # Universal skill sync + dependencies + gaps
│   │   ├── daily_commands.rs          # Daily Eisenhower Matrix commands
│   │   ├── export_commands.rs         # Tree ZIP export
│   │   └── database.rs               # PgPool connection + migrations
│   ├── migrations/                    # Auto-run on startup, sequential (001-016)
│   └── .env                           # DATABASE_URL + API keys (never commit)
├── sidecar/                           # Node.js Mimir service
│   ├── src/
│   │   └── routes/
│   │       ├── ingest.ts              # URL/PDF/text ingestion + embeddings
│   │       ├── chat.ts                # RAG chat with tree-aware context
│   │       ├── match.ts               # Resource-to-node matching
│   │       ├── resources.ts           # Resource listing
│   │       ├── discover.ts            # Link discovery
│   │       ├── playlist.ts            # YouTube playlist ingestion
│   │       └── rescrape.ts            # Re-scrape stale resources
│   └── package.json
├── scraper/                           # Python FastAPI scraper (port 3002)
│   ├── main.py
│   └── requirements.txt
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
| AI generation | OpenRouter (Kimi K2 for trees, Gemini Flash for concept graphs) + Groq (LLaMA for chat/skills) |
| Tree renderer | Custom HTML Canvas (L-system procedural branches) |
| Work/Skills galaxy | D3 force simulation |
| Mimir sidecar | Node.js + TypeScript (port 3001) |
| Scraper | Python FastAPI (port 3002) |
| Embeddings | Transformers.js all-MiniLM-L6-v2 (384 dimensions) |
| Vector search | pgvector on Neon |
| GitHub | REST API via reqwest |

---

## Database Schema

### Core tables
```sql
projects          -- id, name, description, discipline_ids JSONB, skill_ids JSONB, status, progress, created_at
trees             -- id, project_id FK, name, created_at
tree_nodes        -- id, tree_id FK, parent_id FK, type (trunk/branch/leaf),
                  -- title, description, progress, tasks JSONB, resources JSONB,
                  -- x, y, order_index, is_locked
tree_edges        -- id, tree_id FK, source_node_id FK, target_node_id FK
disciplines       -- id, name, description, color
```

### Mimir tables
```sql
mimir_resources   -- id, title, url, type, status, user_notes, content_hash, parent_id,
                  -- tags TEXT[], is_completed, created_at, updated_at
mimir_chunks      -- id, resource_id FK, content, chunk_index
mimir_embeddings  -- id, chunk_id FK, embedding vector(384)
mimir_node_links  -- id, resource_id FK, node_id FK, relevance_score
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
                  -- season, created_at
job_skills        -- id, job_id FK, skill_name, is_required
```

### Ideas
```sql
ideas             -- id, content, tag, pinned, created_at
```

### Resume
```sql
resume_profile    -- id, full_text, parsed JSONB, created_at
```

### Universal Skills
```sql
universal_skills     -- id, name, domain, level (1-5), evidence JSONB, last_updated
skill_dependencies   -- id, source_skill_id FK, target_skill_id FK, relationship
```

### Daily Matrix
```sql
daily_logs           -- id, date (UNIQUE), notes, created_at
daily_quest_links    -- id, date, node_id, free_text, quadrant (do/schedule/delegate/eliminate),
                     -- sort_order, added_at, UNIQUE(date, node_id)
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
```

### Never Do
- Never hardcode `DATABASE_URL`
- Never commit `.env` files
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
Create `src-tauri/migrations/NNN_description.sql` — runs automatically on startup. Never modify existing migrations. Current highest: 016.

### Add a New Page
1. Create `src/pages/NewPage.tsx`
2. Add route in `App.tsx`
3. Add nav link with lucide-react icon in `MainLayout.tsx`

### Run the App
```bash
bash dev.sh
```
This unlocks Bitwarden, exports all env vars, and runs `npm run tauri dev`.

---

## Environment Variables

```bash
# src-tauri/.env (for direct runs without dev.sh)
DATABASE_URL=postgresql://...neon.tech/neondb?sslmode=require
GROQ_API_KEY=gsk_...
GITHUB_TOKEN=ghp_...

# Set by dev.sh (OpenRouter for tree generation)
OPENROUTER_API_KEY=sk-or-...
TREE_GEN_BASE_URL=https://openrouter.ai/api/v1/chat/completions
TREE_GEN_API_KEY=$OPENROUTER_API_KEY
TREE_GEN_MODEL=moonshotai/kimi-k2
CONCEPT_GRAPH_MODEL=google/gemini-2.5-flash
CONCEPT_GRAPH_BASE_URL=https://openrouter.ai/api/v1/chat/completions
CONCEPT_GRAPH_API_KEY=$OPENROUTER_API_KEY
YOUTUBE_API_KEY=...

# Sidecar reads DATABASE_URL from src-tauri/.env
MIMIR_PORT=3001
```

---

## AI Generation Rules

### Tree generation (brain.rs) — Two-Phase Approach
Tree generation uses a two-phase pipeline via OpenRouter:

**Phase 1: Concept Graph Extraction**
- Uses Gemini Flash via OpenRouter
- Extracts 8-20 concepts with prerequisite relationships from the project context
- Concepts are topologically sorted using Kahn's algorithm (foundational first, advanced last)
- If extraction fails, falls back to single-phase generation gracefully

**Phase 2: Tree Generation with Graph Context**
- Uses Kimi K2 via OpenRouter
- The sorted concept dependency order is prepended to the user prompt
- The tree generation LLM uses this to determine phase ordering, skill sequencing, and quest progression
- Every quest should connect back to a concept in the dependency graph

**Rules:**
- Quests must be learning actions ONLY
- ALLOWED: "Read Chapter X", "Watch lecture on Y", "Work through exercises Z"
- FORBIDDEN: "Implement X", "Build Y", "Create Z", "Code W"
- Structure: 3-5 phases → 2-4 skills each → 3 quests each
- GitHub repo analysis: fetch README + dependency files + directory structure + key source files

### Skill extraction (work_commands.rs + job_commands.rs)
- Max 6 tags per resource (Work page)
- Include specific method (e.g. "LDA") AND broader domain (e.g. "Topic Modelling")
- Return ONLY JSON array — no preamble
- For JDs: separate required vs nice-to-have, max 10 required + 5 nice-to-have

---

## Mimir Sidecar

Runs on port 3001. Reads `DATABASE_URL` from `src-tauri/.env`.

### Endpoints
- `POST /ingest/url` — scrape URL, chunk, embed, store
- `POST /ingest/pdf` — parse PDF (text-based only), chunk, embed, store
- `POST /ingest/text` — chunk text directly, embed, store
- `POST /match/:nodeId` — find top 5 matching resources for a quest node
- `POST /chat` — RAG chat: embed query, pgvector search, rerank, inject tree structure if treeId present, Groq synthesis
- `GET /resources` — list all resources with tags, completion status, node counts
- `DELETE /resources/:id` — remove resource + chunks + embeddings
- `POST /discover` — discover links from resource content
- `POST /playlist` — ingest YouTube playlist
- `POST /rescrape/:id` — re-scrape a resource
- `POST /rescrape/all` — re-scrape all stale resources

### Known limitations
- Scanned/image-based PDFs fail (no OCR) — user must paste text instead
- Body size limit: 50MB (configured in Express setup)
- Null bytes stripped from PDF text before storage

---

## Build Order for New Features

Always:
1. DB migration first (if new tables needed)
2. Rust commands second
3. Register in main.rs
4. Frontend last

Never start frontend before backend commands exist.
