# Yggdrasil — Product Requirements Document
**Version:** 2.0  
**Updated:** February 2026

---

## What It Is

Yggdrasil is a personal learning OS built around one insight: **build first, then understand.**

You build a project. Yggdrasil analyzes it, generates a skill tree of everything behind what you built, connects your existing resources to each quest automatically, and guides you through learning it — visualized as a literal growing tree that comes alive as you learn.

Over time the tree becomes your living resume.

---

## The Problem

Learning tools assume you start with a topic ("learn Python"). Builders start with a project ("build a Flask API"). The gap between building something and truly understanding it is enormous — and no tool bridges it.

Separately: resources accumulate but go unused. PDFs saved, videos bookmarked, articles starred — all sitting untouched because nothing connects them to the moment you actually need them.

---

## The Core Loop

```
1. Have a project idea
2. Brainstorm with Mimir → generate a PRD
3. Vibe-code the project (Claude Code, Cursor, etc.)
4. Paste the repo into Yggdrasil → AI analyzes the codebase
5. Yggdrasil generates a skill tree: all concepts behind what you built
6. Mimir searches your resource library → attaches relevant materials to each quest
7. Climb the tree: read, watch, practice → quests unlock skills → skills unlock phases
8. Completed skills feed your Universal Skill Tree (your living resume)
9. Repeat — the tree grows
```

---

## The Four Pillars

### 1. The Tree

A **literal organic tree** — not a node graph, not a diagram. Trunk at the bottom, branches growing upward, leaves at the tips.

- **Trunk** = the project (the why)
- **Major branches** = Phases of learning (3-5 per tree)
- **Minor branches** = Skills within each phase (2-4 per phase)
- **Leaves** = Individual quests (3-6 per skill)

**Node states:**

| State | Visual |
|---|---|
| Locked | Bare branch, grey, no leaves |
| Unlocked | Branch with dark leaves |
| In Progress | Leaves fill with color proportionally |
| Complete | Fully colored, glowing leaves |
| Mastered | Golden shimmer |

**Unlock mechanics:**
- Completing ALL quests in a skill → unlocks the next skill on that branch
- First skill of each branch unlocks when the tree is generated
- Progress auto-cascades: quest % → skill % → phase % → tree %
- Cross-branch dependencies shown visually, non-blocking in V1

**Three viewing modes:**
- **Overview** — zoomed out, full tree visible, understand scope
- **Climb** — focused on current position, next quest highlighted
- **Study** — inside a quest leaf, full panel with resources and completion

### 2. Quests

Individual learning actions. Never implementation tasks.

- "Read Chapter X from [Resource]" ✓
- "Watch lecture series on [Topic]" ✓  
- "Work through practice problems" ✓
- "Implement X feature" ✗
- "Build Y component" ✗

Each quest has: title, description, difficulty (easy/medium/hard), estimated hours, attached resources, completion state.

Quest completion is the atomic unit of progress. Everything else derives from it.

### 3. Mimir (The Librarian)

A built-in AI librarian. Mimir's job: you have a large library of resources that aren't being used. Mimir reads everything, understands what it covers, and surfaces the right resource at the right moment — automatically.

**Mimir does NOT search the web.** It works exclusively with resources you provide. Every suggestion is grounded in material you've already chosen to save.

**What Mimir ingests:**
- URLs (articles, documentation, blog posts — scraped server-side)
- PDFs (uploaded directly, text extracted)
- YouTube links (transcript extracted)
- Plain text / notes

**How it works:**
- Content is parsed and chunked (~500 tokens per chunk)
- Chunks are embedded using Transformers.js `all-MiniLM-L6-v2` (local, free, ~60MB, no API key)
- Embeddings stored in pgvector (same Postgres DB, no extra service)
- On tree generation: Mimir auto-searches library and attaches matching resources to each quest
- On resource add: Mimir finds which existing quests it matches

**Mimir chat panel** — sidebar accessible from any view:
- "What do I have on async Python?" → surfaces relevant chunks
- "What's missing for this tree?" → identifies library gaps
- "I'm stuck on decorators" → answers from your library

### 4. Universal Skill Tree

A persistent, growing graph tracking everything learned across all projects. Unlike project trees (temporary), the Universal Tree is permanent — it represents your actual accumulated knowledge.

Completing a skill in any project tree → automatically adds to or levels up the corresponding Universal Skill, with evidence attached.

**Skill levels:**
- Level 1 — Aware: completed one quest mentioning this skill
- Level 2 — Familiar: completed a full skill node
- Level 3 — Proficient: completed in 2+ different project contexts
- Level 4 — Advanced: hard quests + multiple projects
- Level 5 — Expert: extensive cross-project evidence

**Visualization:** constellation/galaxy map — skill clusters with connections between related skills. Density and brightness reflect level and recency. Exportable as PDF/image for use as a visual resume.

---

## Technical Architecture

### Stack

| Layer | Technology | Notes |
|---|---|---|
| Frontend | React + TypeScript + Tailwind | Existing |
| Desktop | Tauri 2.0 | Existing |
| Main backend | Rust + Axum | Existing |
| Database | Postgres on Neon | Migrating from SQLite |
| AI generation | Groq — LLaMA 3.3-70b | Existing, ~$0.10/tree |
| Mimir sidecar | Node.js + TypeScript | New — Phase 3 |
| Embeddings | Transformers.js all-MiniLM-L6-v2 | Free, local, ~60MB |
| Vector search | pgvector on Neon | No extra infrastructure |

**Running cost: ~$0/month.** The only cost is Groq at ~$0.10 per tree generated.

### Architecture

```
Tauri App (React frontend)
       ↓
Rust/Axum main API (port 3000)  ←→  Neon Postgres (pgvector enabled)
       ↓                                     ↑
Node.js Mimir sidecar (port 3001)  ──────────┘
  - URL scraping, PDF parsing
  - YouTube transcripts
  - Text chunking
  - Transformers.js embeddings
  - pgvector read/write
```

Tauri manages the sidecar lifecycle — starts and stops it automatically. One app from the user's perspective.

### Database Flexibility

Always read `DATABASE_URL` from environment. Never hardcoded. Switching Postgres hosts (Neon → Railway → self-hosted → anything) = changing one env variable. Use standard Postgres + pgvector only — no host-specific features.

---

## Build Phases

### Phase 0 — Foundation (Do First)
Migrate SQLite → Postgres on Neon. Everything else depends on this.

- Set up Neon account, get connection string
- Update Cargo.toml: sqlx sqlite → postgres
- Update database.rs: SqlitePool → PgPool, read DATABASE_URL from env
- Rewrite migrations: TEXT → JSONB, `?N` → `$N` throughout
- Add Mimir schema tables (create now, use later)
- Confirm app builds and connects to Neon
- Merge `fix-compilation-errors` → `main`, delete branch
- `git rm --cached src-tauri/.env`
- Delete `SkillTreeView.tsx`
- Implement `update_project` and `delete_project` stubs

### Phase 1 — Core Tree
Custom literal tree renderer. This is the visual heart of the product.

- Custom SVG organic tree (trunk at bottom, branches up, leaves at tips)
- Node state machine: locked / unlocked / in-progress / complete / mastered
- Color progression: leaves fill proportionally with quest completion
- Quest completion wiring: checkbox → JSONB update → progress cascade
- Unlock mechanics: all quests done → next skill on branch unlocks
- Three viewing modes: Overview, Climb, Study
- Study Mode panel: resources, notes, completion, estimated time

### Phase 2 — AI Generation
Improve what already exists.

- Better PRD→tree prompt (stricter learning focus, validate structure)
- Repo analyzer: GitHub URL → fetch files → extract stack → generate tree of concepts
- Quest quality validator: reject implementation tasks at generation time
- Tree regeneration without losing manual edits

### Phase 3 — Mimir V1
The librarian. Resource ingestion and auto-matching.

- Node.js sidecar setup with Tauri sidecar bundling
- URL scraping (cheerio + node-fetch)
- PDF parsing (pdf-parse)
- YouTube transcripts (youtube-transcript)
- Text chunking pipeline
- Transformers.js embeddings → pgvector storage
- Auto-matching on tree generation
- Resource library view (browse, filter, manage)

### Phase 4 — Mimir Chat
RAG over your personal library.

- Chat panel sidebar component
- RAG pipeline: query → embed → vector search → synthesize answer
- Context awareness: Mimir knows which tree you're looking at
- Gap analysis: identify what's missing from your library for a given tree

### Phase 5 — Universal Skill Tree
Your living resume.

- Sync completed tree skills → universal_skills table
- Evidence-based level calculation
- Constellation visualization (not a tree — a galaxy map)
- Export as PDF/image

### Phase 6 — Polish
- Tree growth/unlock animations
- Onboarding for first-time use
- Error boundaries and loading states throughout
- Performance: lazy load branches, paginate lists

### Phase 7 — Cloud
- Abstract Tauri invoke() behind service layer (swap localhost → production URL)
- Deploy Rust backend to Railway or Fly.io
- Deploy Mimir sidecar alongside backend
- Deploy frontend to Vercel
- Add auth (Clerk or Supabase) when multi-device access needed

---

## V1 Definition of Done

- [ ] Tree generates from PRD in under 15 seconds
- [ ] Tree generates from a GitHub repo URL
- [ ] Tree renders as a literal organic tree (not a node graph)
- [ ] Completing all quests in a skill visually unlocks the next skill
- [ ] Leaves change color proportionally as quests complete
- [ ] PDF or URL can be added to Mimir and indexed
- [ ] Mimir auto-suggests resources when a tree is generated
- [ ] Universal Skill Tree updates when project skills complete
- [ ] Data persists across restarts
- [ ] No crashes during normal use flow

---

## Future: Moltbot Integration

Moltbot is a separate career tracking app (job applications, JD analysis, skill gap analysis) that will share the same Postgres database and integrate with Yggdrasil via the Universal Skill Tree.

Design for it now by:
- Keeping `universal_skills` schema clean and queryable by external apps
- Ensuring the Rust API is accessible independently (not Tauri-only — it already is)
- Tagging skills with a `domain` field for JD matching

Moltbot gets its own PRD when Yggdrasil V1 is complete.

---

*Build the tree. Climb it. Understand everything.*