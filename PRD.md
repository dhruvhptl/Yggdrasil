# Yggdrasil — Product Requirements Document
**Version:** 3.0  
**Updated:** February 2026

---

## What It Is

Yggdrasil is a personal learning OS that advocates **build first, learn after.**

You build a project. Yggdrasil analyzes it, generates a skill tree of everything behind what you built, connects your existing resources to each quest automatically, and guides you through learning it. Your co-op experience, job targets, and learning progress all feed into one place — culminating in a Universal Skill Tree that is your living proof of expertise.

---

## The North Star

The **Universal Skill Tree** is the hub that connects everything. Every other feature feeds it:

```
GitHub repos    →  Project trees  →  Completed quests  ──┐
Resume                                                     ├──→  Universal Skill Tree
Co-op work page →  Extracted skills  ────────────────────┘            ↓
Job tracker     →  Skill demand analysis  ─────────────────→  "Learn these to get hired"
                                                                       ↓
                                                           Generate tree → climb it
                                                                       ↓
                                                           Universal Tree grows
```

It is built last — when all data sources are mature and feeding it properly. Every feature before it exists to make it meaningful.

---

## The Problem

Learning tools assume you start with a topic. Builders start with a project. The gap between building something and truly understanding it is enormous — and no tool bridges it.

Separately: resources accumulate but go unused. Career progress is tracked across five different tools. Co-op experience lives in Notion. Job applications are a spreadsheet. Skills are nowhere.

Yggdrasil puts it all in one place.

---

## The Core Loop

```
1. Build a project (vibe-code it, ship it)
2. Paste the GitHub repo → Yggdrasil analyzes the codebase
3. Skill tree generated: all concepts behind what you built
4. Mimir attaches your saved resources to each quest automatically
5. Climb the tree: read, watch, practice → quests complete → skills unlock
6. Skills feed the Universal Skill Tree
7. Job tracker reads Universal Tree → "you need these skills for your target roles"
8. Generate a tree for the gap → climb it → Universal Tree grows
9. Repeat
```

---

## What's Built (v1.0)

### ✅ Trees
A literal organic tree — trunk at bottom, branches growing upward, leaves at tips. Generated from a PRD or a GitHub repo URL via Groq (LLaMA 3.3-70b, ~$0.10/tree).

- Trunk = project, branches = phases, leaves = quests
- Node states: locked / unlocked / in-progress / complete / mastered
- Quest completion cascades: quest % → skill % → phase % → tree %
- Three viewing modes: Overview, Climb, Study
- Study panel: resources, notes, completion, estimated time

### ✅ Quests
Learning actions only — never implementation tasks. Each quest has title, description, difficulty, estimated hours, attached resources, completion state, and notes. Aggregated across all projects in the Quests page.

### ✅ Mimir (Librarian V1)
A Node.js sidecar that ingests your personal resource library and auto-matches content to quests via semantic search.

- Ingests: URLs (scraped), PDFs (text-based), plain text
- Embeddings: Transformers.js `all-MiniLM-L6-v2` (384 dimensions, free, local, ~60MB)
- Vector search: pgvector on Neon
- Auto-matches resources to quests on tree generation
- Resource library view: browse, filter, manage

### ✅ Work Page
Co-op experience tracker with a D3 force galaxy visualization.

- Co-ops → research topics → resources hierarchy in left panel
- AI skill extraction via Groq (max 6 tags: specific method + broader domain)
- Galaxy: skill nodes orbit co-op sun nodes, brightness scales with resource count
- D3 force simulation with star field background — feels alive
- Skills shared across co-ops shown with connections between clusters
- Clicking a skill node shows backing resources

### ✅ Jobs Page
Job application tracker with JD analysis and skill demand analytics.

- Kanban board: Saved / Applied / Interviewing / Offer / Rejected
- Per-job ratings: location, alignment, salary, role (overall auto-calculated as average)
- Full JD storage with AI skill extraction (required vs nice-to-have badges)
- Analytics: skill demand chart weighted by job ratings, skills to prioritize
- Follow-up tracker: overdue/due-soon indicators, one-click mark followed up
- Season grouping for recruiting cycle comparison

---

## What's Next

### Phase 1 — Resume Page (`/resume`)
Paste resume → AI extracts projects, skills, experience → pre-populates Universal Skill Tree starting point.

- Resume text input or PDF upload
- AI extracts: skills, projects (with tech stack), work experience, education
- Skills pre-loaded into `universal_skills` table at appropriate starting levels
- Projects shown as tree candidates: "Generate a tree for this project?"
- Stored as structured profile in DB

### Phase 2 — Ideas Page (`/ideas`)
Simple scratchpad. One afternoon of work.

- Text entries with timestamps
- Tags: project idea / resource / random / learning
- Pin important entries
- "Turn into project" button → creates project from idea

### Phase 3 — Mimir Chat
RAG over your personal library.

- Chat sidebar accessible from any view
- Pipeline: embed query → pgvector search → retrieve chunks → Groq synthesizes
- Context-aware: knows which tree you're viewing
- Gap analysis: "what's missing from my library for this tree?"
- Source attribution on every answer

### Phase 4 — GitHub MCP (Read-Only)
Replace current GitHub REST API calls with GitHub MCP for deeper code analysis.

Current: fetches README, dependency files, directory listing.

With MCP (read-only):
- Read actual source files — understand how libraries are used, not just listed
- Read issues/PRs — understand what problems were being solved
- Commit history — understand how the project evolved
- Result: trees that reflect what you actually built, not just what you imported

Every tree generated after this will be significantly richer. Better trees → better Universal Skill Tree data.

### Phase 5 — Universal Skill Tree (`/skills`)
Built last. When all data sources are mature. Has to be perfect.

**What feeds it:**
- Resume page → starting skill levels
- Completed project tree quests → level up with evidence
- Work page skills → professional context
- Job tracker demand analysis → gap highlighting

**Skill levels:**
- Level 1 — Aware: one quest mentioning this skill
- Level 2 — Familiar: completed a full skill node
- Level 3 — Proficient: completed in 2+ project contexts
- Level 4 — Advanced: hard quests + multiple projects + work evidence
- Level 5 — Expert: extensive cross-project evidence

**Visualization:** D3 force galaxy — same visual language as Work page. Skill clusters group by domain. Brightness and size reflect level and recency. Skill dependency edges from tree branch structure show learning progression. Exportable as PDF/image (visual resume).

**Job tracker overlay:** skills you need but don't have highlighted in amber directly on the galaxy.

**Work page integration:** once built, Work page galaxy inherits skill dependency edges automatically — no manual input needed.

---

## Technical Architecture

### Stack

| Layer | Technology | Status |
|---|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind | ✅ |
| Desktop | Tauri 2.0 | ✅ |
| Main backend | Rust + Axum | ✅ |
| Database | Postgres on Neon (pgvector enabled) | ✅ |
| AI generation | Groq — LLaMA 3.3-70b-versatile | ✅ |
| Tree visualization | Custom SVG organic renderer | ✅ |
| Work/Skills galaxy | D3 force simulation | ✅ |
| Mimir sidecar | Node.js + TypeScript (port 3001) | ✅ |
| Embeddings | Transformers.js all-MiniLM-L6-v2 (384d) | ✅ |
| Vector search | pgvector on Neon | ✅ |
| GitHub integration | REST API via reqwest | ✅ |
| GitHub MCP | Read-only MCP server | 🔲 Phase 4 |

**Running cost: ~$0/month.** Only real cost is Groq at ~$0.10/tree.

### Architecture
```
Tauri App (React frontend)
       ↓
Rust/Axum main API (port 3000)  ←→  Neon Postgres (pgvector enabled)
       ↓                                     ↑
Node.js Mimir sidecar (port 3001)  ──────────┘
```

### Database Rules
- Always read `DATABASE_URL` from environment — never hardcode
- Standard Postgres + pgvector only — no Neon-specific features
- All JSON stored as JSONB
- All query placeholders use `$N` format

---

## Pages

| Route | Page | Status |
|---|---|---|
| `/` | Home — project list + creation | ✅ |
| `/trees` | All trees across projects | ✅ |
| `/project/:id` | Tree canvas + PRD/GitHub input | ✅ |
| `/quests` | All quests across projects | ✅ |
| `/library` | Mimir resource library | ✅ |
| `/work` | Co-op skill galaxy | ✅ |
| `/jobs` | Job application tracker | ✅ |
| `/resume` | Resume parser + profile | 🔲 Phase 1 |
| `/ideas` | Scratchpad | 🔲 Phase 2 |
| `/skills` | Universal Skill Tree | 🔲 Phase 5 |

---

## Future: Monetization

Currently personal use only. Path to multi-user:
- Auth: Clerk or Supabase Auth
- Cloud deployment: Rust backend on Railway, frontend on Vercel
- Per-user data isolation in Postgres

The core loop (build → tree → learn → Universal Tree) is the product. The job tracker + skill gap analysis is the hook. GitHub MCP is the differentiator.

---

*Build the tree. Climb it. Understand everything.*