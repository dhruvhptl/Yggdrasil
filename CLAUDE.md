# Yggdrasil — Claude Code Development Guide

## What This App Is

A personal learning OS. Build a project, paste the repo, Yggdrasil generates a skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

Read `PRD.md` for the full vision. This file is your technical bible.

---

## Current State (v1.0)

**Fully working:**
- Project creation, listing, edit, delete
- AI tree generation from PRD text (brain.rs → Groq)
- AI tree generation from GitHub repo URL (brain.rs → GitHub API → Groq)
- Custom SVG organic tree renderer (trunk at bottom, branches up, leaves at tips)
- Quest tracking with completion, notes, progress cascade
- Mimir sidecar: URL scraping, PDF ingestion, text chunking, embeddings, pgvector storage
- Auto-matching resources to quest nodes on tree generation
- Resource library page
- Work page: co-op tracker + D3 force galaxy visualization + AI skill extraction
- Jobs page: kanban board + JD analysis + skill demand analytics + follow-up tracker
- Postgres on Neon with pgvector
- All migrations run automatically on startup

**Not yet built:**
- Resume page (`/resume`) — Phase 1
- Ideas page (`/ideas`) — Phase 2
- Mimir chat/RAG — Phase 3
- GitHub MCP integration — Phase 4
- Universal Skill Tree (`/skills`) — Phase 5

---

## Project Structure

```
├── src/
│   ├── components/
│   │   ├── EditableSkillTree.tsx   # Legacy tree editor — may still be referenced
│   │   └── ProjectInput.tsx        # Project creation form
│   ├── pages/
│   │   ├── Homepage.tsx            # Project list + creation
│   │   ├── ProjectTreePage.tsx     # Tree canvas + PRD/GitHub generation
│   │   ├── TreesPage.tsx           # All trees list
│   │   ├── QuestsPage.tsx          # All quests across projects
│   │   ├── LibraryPage.tsx         # Mimir resource library
│   │   ├── WorkPage.tsx            # Co-op galaxy
│   │   └── JobsPage.tsx            # Job tracker
│   ├── layouts/
│   │   └── MainLayout.tsx          # Sidebar navigation
│   └── types.ts
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                 # Command registration — register ALL new commands here
│   │   ├── commands.rs             # Project CRUD
│   │   ├── tree_commands.rs        # Tree/node CRUD + quest completion
│   │   ├── brain.rs                # Groq AI generation + GitHub repo analysis
│   │   ├── work_commands.rs        # Co-op/topic/resource/skill commands
│   │   ├── job_commands.rs         # Job application commands
│   │   └── database.rs             # PgPool connection + migrations
│   ├── migrations/                 # Auto-run on startup, sequential
│   └── .env                        # DATABASE_URL + GROQ_API_KEY (never commit)
├── sidecar/                        # Node.js Mimir service
│   ├── src/
│   │   └── routes/
│   │       └── ingest.ts           # URL/PDF/text ingestion + embeddings
│   └── package.json
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Main backend | Rust + Axum |
| Database | Postgres on Neon (pgvector enabled) |
| AI generation | Groq API — LLaMA 3.3-70b-versatile |
| Tree renderer | Custom SVG (organic tree shape) |
| Work/Skills galaxy | D3 force simulation |
| Mimir sidecar | Node.js + TypeScript (port 3001) |
| Embeddings | Transformers.js all-MiniLM-L6-v2 (384 dimensions) |
| Vector search | pgvector on Neon |
| GitHub | REST API via reqwest (upgrading to MCP in Phase 4) |

---

## Database Schema

### Core tables
```sql
projects          -- id, name, description, discipline_ids JSONB, status, progress, created_at
trees             -- id, project_id FK, name, created_at
tree_nodes        -- id, tree_id FK, parent_id FK, type (trunk/branch/leaf),
                  -- title, description, progress, tasks JSONB, resources JSONB,
                  -- x, y, order_index
tree_edges        -- id, tree_id FK, source_node_id FK, target_node_id FK
disciplines       -- id, name, description, color
```

### Mimir tables
```sql
mimir_resources   -- id, title, url, type, status, user_notes, content_hash, created_at
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

### Universal skills (ready, not yet used)
```sql
universal_skills  -- id, name, domain, level (1-5), evidence JSONB, last_updated
```

---

## Coding Conventions

### Rust
```rust
// Always $N placeholders — never ?N
sqlx::query!("SELECT * FROM projects WHERE id = $1", id)

// Commands return Result<T, String>
pub async fn my_command(...) -> Result<MyType, String> {
    op().await.map_err(|e| e.to_string())
}

// Always propagate errors with ?
let result = some_async_op().await?;
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
Create `src-tauri/migrations/00N_description.sql` — runs automatically on startup. Never modify existing migrations.

### Add a New Page
1. Create `src/pages/NewPage.tsx`
2. Add route in `App.tsx`
3. Add nav link with lucide-react icon in `MainLayout.tsx`

### Run the App
```bash
npm run tauri dev
```

---

## Environment Variables

```bash
# src-tauri/.env
DATABASE_URL=postgresql://...neon.tech/neondb?sslmode=require
GROQ_API_KEY=gsk_...
GITHUB_TOKEN=ghp_... (optional — increases rate limit, required for private repos)

# sidecar reads same DATABASE_URL
MIMIR_PORT=3001
```

---

## AI Generation Rules

### Tree generation (brain.rs) — Two-Phase Approach
Tree generation uses a two-phase pipeline:

**Phase 1: Concept Graph Extraction**
- First AI call extracts 8-20 concepts with prerequisite relationships from the project context
- Concepts are topologically sorted using Kahn's algorithm (foundational first, advanced last)
- If extraction fails, falls back to single-phase generation gracefully

**Phase 2: Tree Generation with Graph Context**
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

Runs on port 3001. Tauri manages lifecycle. Reads `DATABASE_URL` from environment.

### Endpoints
- `POST /ingest/url` — scrape URL, chunk, embed, store
- `POST /ingest/pdf` — parse PDF (text-based only), chunk, embed, store
- `POST /ingest/text` — chunk text directly, embed, store
- `POST /match/:nodeId` — find top 5 matching resources for a quest node
- `GET /resources` — list all ingested resources
- `DELETE /resources/:id` — remove resource + chunks + embeddings

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

---

## Next Features (in order)

### Phase 1: Resume Page
- New migration: `resume_profile` table (skills, projects, experience, education as JSONB)
- New `resume_commands.rs`: `parse_resume(text) → ResumeProfile`, `get_resume() → ResumeProfile`
- AI extracts structured data from resume text via Groq
- Pre-populates `universal_skills` table with starting levels
- Frontend: `/resume` page, paste input or PDF upload, parsed profile display, "Generate tree" buttons on projects

### Phase 2: Ideas Page
- New migration: `ideas` table (id, content, tag, pinned, created_at)
- New commands: `create_idea`, `get_ideas`, `update_idea`, `delete_idea`, `idea_to_project`
- Frontend: `/ideas` page, simple card list, tag filter, pin, "Turn into project" action

### Phase 3: Mimir Chat
- New sidecar endpoint: `POST /chat` — embed query, pgvector search, build context, Groq RAG
- Frontend: collapsible chat sidebar in MainLayout, context passes current page/tree
- Gap analysis endpoint: `GET /gaps/:treeId`

### Phase 4: GitHub MCP
- Replace GitHub REST calls in `brain.rs` with MCP client
- Read-only only — no writes ever
- Deeper analysis: actual source files, issues, PR history

### Phase 5: Universal Skill Tree
- New migration: `skill_dependencies` table
- New `skill_commands.rs`: sync from trees, sync from work page, calculate levels
- Sync triggers: on quest completion, on work resource skill extraction
- Frontend: `/skills` page, D3 force galaxy (reuse Work page galaxy component)
- Job tracker overlay: highlight skill gaps in amber
- Export: PDF/image of full galaxy