# Yggdrasil — Claude Code Development Guide

## What This App Is

A personal learning OS. You build a project, paste the repo or PRD, and Yggdrasil generates a **literal growing tree** of everything you need to learn. Quests unlock skills, skills unlock phases, the tree comes alive as you learn. Over time it becomes your living resume.

Read `PRD.md` for the full vision. This file is your technical bible.

---

## Current State

**Working:**
- Project creation and listing
- AI tree generation via Groq API (brain.rs)
- Manual tree editor (EditableSkillTree.tsx, react-d3-tree)
- SQLite persistence with SQLx
- Node CRUD (create, update, delete, resources)

**Broken / Stubbed:**
- `update_project` and `delete_project` commands are stubs — not implemented
- Quest completion checkboxes exist in UI but don't write to database
- `QuestsPage.tsx` is an empty placeholder
- Progress on project cards doesn't update

**To Delete:**
- `src/components/SkillTreeView.tsx` — unused ReactFlow component, never rendered

**Security:**
- `src-tauri/.env` was accidentally committed — run `git rm --cached src-tauri/.env`

---

## Project Structure

```
├── src/
│   ├── components/
│   │   ├── EditableSkillTree.tsx   # Main tree editor (react-d3-tree) — KEEP
│   │   ├── ProjectInput.tsx        # Project creation form
│   │   └── SkillTreeView.tsx       # DELETE — unused ReactFlow component
│   ├── pages/
│   │   ├── Homepage.tsx            # Project list
│   │   ├── ProjectTreePage.tsx     # Tree view + AI generation
│   │   ├── TreesPage.tsx           # All trees list
│   │   └── QuestsPage.tsx          # EMPTY PLACEHOLDER — needs building
│   ├── layouts/
│   │   └── MainLayout.tsx          # Sidebar navigation
│   └── types.ts
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                 # Command registration
│   │   ├── commands.rs             # Project CRUD (update/delete are stubs)
│   │   ├── tree_commands.rs        # Tree/node CRUD — fully working
│   │   ├── brain.rs                # Groq AI generation — fully working
│   │   └── database.rs             # DB connection + init
│   ├── migrations/
│   │   ├── 001_initial.sql
│   │   └── 002_tree_schema.sql
│   └── .env                        # GROQ_API_KEY (do not commit)
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Main backend | Rust + Axum |
| Database | **Postgres on Neon** (migrating from SQLite — see Phase 0) |
| AI generation | Groq API — LLaMA 3.3-70b-versatile |
| Tree visualization | Custom SVG renderer (replacing react-d3-tree) |
| Mimir sidecar | Node.js + TypeScript (future — Phase 3) |
| Embeddings | Transformers.js all-MiniLM-L6-v2 (future — Phase 3) |
| Vector search | pgvector on Neon (future — Phase 3) |

---

## Phase 0: SQLite → Postgres Migration (DO THIS FIRST)

Everything else depends on this. Do not start Phase 1 until the app is running on Neon.

### Steps

**1. Cargo.toml** — swap sqlx feature:
```toml
# Remove:
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-native-tls"] }
# Add:
sqlx = { version = "0.7", features = ["postgres", "runtime-tokio-native-tls"] }
```

**2. database.rs** — swap pool type:
```rust
// Remove: SqlitePool, SqlitePoolOptions
// Add: PgPool, PgPoolOptions
// Read DATABASE_URL from env (not hardcoded)
let pool = PgPoolOptions::new()
    .connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
    .await?;
```

**3. .env** — add Neon connection string:
```
DATABASE_URL=postgresql://user:pass@host.neon.tech/neondb?sslmode=require
GROQ_API_KEY=your_key_here
```

**4. All migrations** — two changes throughout:
- `?1, ?2, ?3` → `$1, $2, $3`
- `TEXT NOT NULL DEFAULT '[]'` → `JSONB NOT NULL DEFAULT '[]'`
- `TEXT DEFAULT NULL` → `JSONB DEFAULT NULL`
- `BOOLEAN` stays as-is (Postgres supports it natively)
- `TEXT NOT NULL` for timestamps → `TIMESTAMPTZ NOT NULL DEFAULT NOW()`

**5. All queries in .rs files** — same placeholder change: `?1` → `$1` etc.

**6. New migration** — add Mimir tables (create these now so schema is ready):
```sql
-- 003_postgres_and_mimir.sql

-- Update existing columns to JSONB (handled in migration)
ALTER TABLE projects ALTER COLUMN discipline_ids TYPE JSONB USING discipline_ids::jsonb;
ALTER TABLE projects ALTER COLUMN skill_ids TYPE JSONB USING skill_ids::jsonb;
ALTER TABLE tree_nodes ALTER COLUMN tasks TYPE JSONB USING COALESCE(tasks, '[]')::jsonb;
ALTER TABLE tree_nodes ALTER COLUMN resources TYPE JSONB USING COALESCE(resources, '[]')::jsonb;

-- Mimir tables
CREATE TABLE IF NOT EXISTS mimir_resources (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    url TEXT,
    type TEXT NOT NULL DEFAULT 'other',
    status TEXT NOT NULL DEFAULT 'unread',
    user_notes TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS mimir_chunks (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    chunk_index INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mimir_embeddings (
    id TEXT PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES mimir_chunks(id) ON DELETE CASCADE,
    embedding vector(384)
);

CREATE TABLE IF NOT EXISTS mimir_node_links (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES tree_nodes(id) ON DELETE CASCADE,
    relevance_score REAL,
    UNIQUE(resource_id, node_id)
);

CREATE TABLE IF NOT EXISTS universal_skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT,
    level INTEGER NOT NULL DEFAULT 1 CHECK(level BETWEEN 1 AND 5),
    evidence JSONB NOT NULL DEFAULT '[]',
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Database Rules (Always Follow)
- Always read `DATABASE_URL` from environment — never hardcode
- Use standard Postgres + pgvector only — no Neon-specific features
- All JSON → JSONB
- All query placeholders → `$N` format

---

## Phase 1: Core Tree (After Postgres Migration)

The literal organic tree renderer. This replaces react-d3-tree entirely.

### Visual Requirements
- Trunk at the **bottom**, branches grow **upward**
- Organic, slightly irregular angles — not symmetric
- Leaves at branch tips (quests)
- Canvas is infinite — pan and zoom
- Node states drive appearance:

| State | Visual |
|---|---|
| Locked | Bare branch, grey, no leaves |
| Unlocked | Branch with dark/muted leaves |
| In Progress | Leaves fill with color proportionally |
| Complete | Fully colored, subtle glow |
| Mastered | Golden shimmer |

### Unlock Logic
- Completing ALL quests in a skill → unlocks next skill on that branch
- First skill of each branch unlocks when tree is generated
- Progress cascades: quest completion → skill % → phase % → tree %
- Cross-branch dependencies shown visually but non-blocking

### Quest Completion Wiring (currently broken)
```typescript
// When checkbox ticked:
// 1. Update tasks JSONB in tree_nodes
// 2. Recalculate node progress (completed / total * 100)
// 3. Check if all quests done → unlock next skill
// 4. Propagate progress up the tree
await invoke('complete_quest', { nodeId, questId });
```

### Three Viewing Modes
- **Overview** — zoomed out, full tree, understand scope
- **Climb** — focused on current position, next quest highlighted
- **Study** — inside a quest, full panel (resources, notes, completion)

### Implement Stub Commands
```rust
// commands.rs — these are stubs, implement them:
pub async fn update_project(project_id: String, name: Option<String>, description: Option<String>, ...)
pub async fn delete_project(project_id: String, ...) // cascade: project → trees → nodes → edges
pub async fn update_project_progress(project_id: String, ...) // aggregate from tree nodes
```

---

## Phase 2: AI Generation Improvements

### What Already Works
- `brain.rs` → Groq API → JSON → tree_nodes in DB
- PRD text → tree generation (~$0.10, 5-10s)

### What to Add
- **Repo analyzer**: accept GitHub URL → fetch key files → extract tech stack → generate tree of "concepts behind what you built"
- **Quest quality validator**: system prompt must reject implementation tasks, enforce learning actions only
- **Tree regeneration**: re-run AI on same project without destroying manual edits

### Quest Quality Rules (enforce in system prompt)
```
ALLOWED: "Read Chapter X", "Watch lecture on Y", "Work through exercises Z"
FORBIDDEN: "Implement X", "Build Y", "Create Z", "Code W"
```

---

## Phase 3: Mimir Sidecar

A separate Node.js + TypeScript service. Tauri bundles and manages it.

### Sidecar Architecture
```
Rust (port 3000) ←→ Node sidecar (port 3001) ←→ Neon Postgres
```

Rust proxies Mimir API calls to the sidecar. Both services share the same `DATABASE_URL`.

### Sidecar Responsibilities
- URL scraping (cheerio + node-fetch)
- PDF parsing (pdf-parse)
- YouTube transcripts (youtube-transcript)
- Text chunking (~500 tokens)
- Embeddings (Transformers.js — `all-MiniLM-L6-v2`, 384 dimensions, ~60MB download on first use)
- pgvector read/write

### Auto-Matching Flow
On tree generation:
1. For each new quest node, embed the title + description
2. Search `mimir_embeddings` for similar chunks
3. Write matches to `mimir_node_links` with relevance score
4. Return linked resources to frontend with tree

### Mimir Chat (RAG)
1. User sends message
2. Embed the query
3. Search pgvector for top-k similar chunks
4. Build context from chunk content
5. Send to Groq: system prompt + context + user message
6. Return answer

---

## Coding Conventions

### Rust
```rust
// Queries use $N placeholders (Postgres)
sqlx::query!("SELECT * FROM projects WHERE id = $1", id)

// Always propagate errors with ?
let result = some_async_op().await?;

// Commands return Result<T, String>
pub async fn my_command(...) -> Result<MyType, String> {
    op().await.map_err(|e| e.to_string())
}
```

### TypeScript / React
```typescript
// Always type invoke returns
const result = await invoke<MyType>('command_name', { params });

// Parse JSONB fields safely
const tasks = Array.isArray(node.tasks) ? node.tasks : JSON.parse(node.tasks ?? '[]');

// Loading states on all async ops
const [loading, setLoading] = useState(false);
```

### Never Do
- Never hardcode `DATABASE_URL`
- Never commit `.env` files
- Never use `?N` query placeholders (Postgres uses `$N`)
- Never store JSON as TEXT (use JSONB)
- Never generate quests that are implementation tasks
- Never use SkillTreeView.tsx (delete it)

---

## Common Patterns

### Call Rust from React
```typescript
import { invoke } from '@tauri-apps/api/core';
const tree = await invoke<Tree>('get_tree_with_contents', { treeId });
```

### Add a New Tauri Command
1. Write function in `commands.rs` or `tree_commands.rs` with `#[tauri::command]`
2. Register in `main.rs` invoke_handler
3. Call from frontend with `invoke('command_name', { params })`

### Add a Migration
Create `src-tauri/migrations/00N_description.sql` — runs automatically on startup.

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
```

When Mimir sidecar is added:
```bash
MIMIR_PORT=3001
MIMIR_URL=http://localhost:3001
```

---

## Build Order (Do Not Skip Steps)

1. **Phase 0** — Postgres migration. App must build and connect to Neon before anything else.
2. **Phase 1** — Literal tree renderer + quest completion wiring + stub commands.
3. **Phase 2** — Improve AI generation (repo analysis, quest quality).
4. **Phase 3** — Mimir sidecar (ingestion + embeddings + auto-matching).
5. **Phase 4** — Mimir chat panel (RAG).
6. **Phase 5** — Universal Skill Tree.

Each phase must be fully working before starting the next.