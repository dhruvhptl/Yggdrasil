# Yggdrasil — Product Requirements Document
**Version:** 2.1  
**Updated:** April 2026

---

## What It Is

Yggdrasil is a personal learning OS that advocates **build first, learn after.**

You build a project. Yggdrasil analyzes it, generates a skill tree of every concept behind what you built, connects your existing resources to each quest automatically, and guides you through learning it. Co-op experience, job targets, and learning progress all feed into one place — culminating in a Universal Skill Tree that is your living proof of expertise.

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

Built last — when all data sources are mature and feeding it properly. Every feature before it exists to make it meaningful.

---

## The Problem

Learning tools assume you start with a topic. Builders start with a project. The gap between building something and truly understanding it is enormous — and no tool bridges it.

Separately: resources accumulate but go unused. Career progress is tracked across five different tools. Co-op experience lives in Notion. Job applications live in a spreadsheet. Skills are nowhere.

Yggdrasil puts it all in one place.

---

## The Core Loop

```
1. Build a project (vibe-code it, ship it)
2. Paste the GitHub repo → Yggdrasil analyzes the codebase
3. Skill tree generated: all concepts behind what you built, as checkpoints
4. Mimir attaches your saved resources to each checkpoint automatically
5. Climb the tree: reach each checkpoint → resources + exercises guide you there
6. Mark a checkpoint complete when you genuinely understand the concept
7. Skills feed the Universal Skill Tree
8. Job tracker reads Universal Tree → "you need these skills for your target roles"
9. Generate a tree for the gap → climb it → Universal Tree grows
10. Repeat
```

---

## Current State (v2.1)

### What's Built & Working

| Module | Status | Notes |
|---|---|---|
| Mimir Library — URL ingestion | ✅ Done | 3-tier scraper, YouTube transcripts, PDF via pymupdf |
| Mimir Library — PDF ingestion | ✅ Done | TOC-aware chunking, section titles + page ranges on chunks |
| Mimir Library — YouTube playlists | ✅ Done | Parent-child grouping, per-video completion tracking |
| Mimir Library — Page type detection | ✅ Done | Auto-detects resource lists, triggers child ingest modal |
| Mimir Library — External links modal | ✅ Done | Links from resource-list pages ingested as children |
| Mimir Library — Rescrape All | ✅ Done | Bulk rescrape with real-time progress events; YouTube videos excluded |
| Mimir Library — Re-embed PDFs | ✅ Done | Re-embeds from sections_json or raw_text without re-uploading |
| Mimir Chat — RAG | ✅ Done | pgvector cosine search, Groq LLaMA 3.3-70b synthesis, section+page citations |
| Mimir Chat — Reranking | ✅ Done | llama-3.1-8b-instant reranker, top-3 selection |
| Mimir — Native Rust | ✅ Done | No Node.js sidecar — all ingest/RAG/rescrape in mimir.rs |
| Embeddings | ✅ Done | pplx-embed-v1-0.6b (1024-dim) via OpenRouter |
| Tree Generation — PRD | ✅ Done | Kimi-k2, concept graph, topo sort |
| Tree Generation — GitHub repo | ✅ Done | GitHub API, source file analysis, concept graph |
| Tree Rendering | ✅ Done | Custom Canvas (YggdrasilTree.tsx), trunk/branch/leaf organic layout, node panel, checkpoint completion |
| Auto-matching checkpoints to resources | ✅ Done | pgvector cosine match on checkpoint title + description |
| Universal Skill Tree — Galaxy | ✅ Done | Skills, dependencies, gap analysis |
| Jobs Page | ✅ Done | Kanban + skill gap detection |
| Resume Page | ✅ Done | Auto-parse, skill extraction |
| Work Page | ✅ Done | Co-op tracker, skill extraction |
| Daily Matrix | ✅ Done | Eisenhower 2x2 triage for quests + free-form tasks |

---

## Technical Architecture

### Stack

| Layer | Technology | Status |
|---|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind | ✅ |
| Desktop | Tauri 2.0 | ✅ |
| Main backend | Rust + sqlx | ✅ |
| Database | Postgres on Neon (pgvector enabled) | ✅ |
| AI generation | Kimi K2 + Gemini Flash via OpenRouter | ✅ |
| AI chat / extraction | Groq — LLaMA 3.3-70b-versatile + 3.1-8b-instant | ✅ |
| Tree visualization | Custom HTML Canvas (L-system) | ✅ |
| Work/Skills galaxy | D3 force simulation | ✅ |
| Mimir | Native Rust in mimir.rs — no sidecar | ✅ |
| PDF extraction | Python scraper /fetch-pdf with pymupdf (TOC-aware) | ✅ |
| Embeddings | Perplexity pplx-embed-v1-0.6b (1024-dim) via OpenRouter | ✅ |
| Vector search | pgvector on Neon — vector(1024) | ✅ |
| GitHub integration | REST API via reqwest | ✅ |

**Running cost: ~$0/month.** Only real cost is tree generation at ~$0.10/tree.

### Architecture

```
Tauri App (React frontend)
       ↓
Rust backend (Tauri commands)  ←→  Neon Postgres (pgvector enabled)
       ↓
Python scraper (port 3002)     ←→  OpenRouter (embeddings + tree gen)
                                ←→  Groq (chat synthesis + reranking)
```

### Database Rules
- Always read `DATABASE_URL` from environment — never hardcode
- Standard Postgres + pgvector only — no Neon-specific features
- All JSON stored as JSONB
- All query placeholders use `$N` format

### Pages

| Route | Page | Status |
|---|---|---|
| `/` | Home — project list + creation | ✅ |
| `/trees` | All trees across projects | ✅ |
| `/project/:id` | Tree canvas + PRD/GitHub input | ✅ |
| `/quests` | All quests across projects | ✅ |
| `/resources` | Mimir resource library | ✅ |
| `/work` | Co-op skill galaxy | ✅ |
| `/jobs` | Job application tracker | ✅ |
| `/resume` | Resume parser + profile | ✅ |
| `/skills` | Universal Skill Tree | ✅ (V2: radial tree layout rebuild) |
| `/ideas` | Scratchpad | ✅ |

---

## Immediate Priorities (Now)

### 1. Ingest the Mimir Library

- Upload key PDFs (textbooks, papers) — section-aware chunking now preserves TOC structure
- Ingest YouTube playlists for active learning tracks
- Run Re-embed PDFs after any embedding model change

### 2. Climb Trees

Generate and climb trees for active projects. Take notes in checkpoint panels. Use Mimir chat while climbing. Document every pain point.

### 3. Add Remaining Jobs

Expand the job tracker. Target: 50+ jobs before reviewing skill gap analysis.

---

## The Checkpoint Model

Leaf nodes in Yggdrasil are **concept checkpoints**, not tasks. The mental model shift:

```
Before:  leaf node = quest = "do this thing"
After:   leaf node = checkpoint = "understand this concept"
```

A checkpoint represents a specific concept you need to genuinely understand. You reach it — you don't complete it like a chore.

### What a checkpoint contains

- **Concept name** — the specific thing to understand (e.g. "Velocity-Verlet Symplectic Integration")
- **Mastery criteria** — AI-drafted description of what understanding this concept looks like. You refine it as you learn.
- **Resources** — auto-matched from Mimir, the material that gets you there
- **Exercises** — specific things to work through to confirm understanding
- **Your notes** — written as you climb

### What "complete" means

A checkpoint is complete when you've satisfied the mastery criteria — a combination of working through the resources, completing the exercises, and your own judgment that you genuinely get it. You mark it done manually; there's no automatic completion.

### Tree structure (unchanged)

```
Trunk (project)
  └── Branch (phase/domain)
        └── Leaf (concept checkpoint)
```

Checkpoints have no difficulty rating or estimated time. They're concepts — some take an afternoon, some take a week. You'll know when you're there.

### Migration

All existing leaf nodes (quests) are migrated to the checkpoint model. The `tasks` JSONB field on leaf nodes is repurposed: instead of a checklist of tasks, it holds `{ mastery_criteria, exercises, notes }`.

---



### 2.1 Scraper Upgrade

The current 5-tier scraper works for most sites but has limitations. V2 goals:

- Better `extract_text()` for div-heavy sites (currently misses content in non-semantic HTML)
- Audio transcription via Whisper for YouTube videos with no captions
- Video description + chapters as fallback when transcript unavailable
- Fix the 11 permanently broken resources — replace with better URLs
- Proper encoding handling — UTF-8 forced on Windows (`sys.stdout.reconfigure`)

### 2.2 GitHub Repo Reader Upgrade

Current `analyze_repo` in `brain.rs` does shallow fetching. V2 upgrade:

- Tree-sitter Rust crate — parse source files into structured symbols (functions, classes, imports)
- Call graph analysis — which functions call which
- Import map — what each file imports from where
- Richer LLM context — structured symbol map instead of raw file dumps
- Result: checkpoints reference specific patterns the author actually used, not just library names

### 2.3 System Prompt Rewrite — Checkpoint Model

Update `build_system_prompt()` and `build_repo_system_prompt()` in `brain.rs` to generate checkpoints instead of quests:

- Each leaf node is a **concept**, not a task — title is a noun phrase, not an imperative
- Output includes `mastery_criteria` (what understanding looks like), `exercises` (specific things to work through)
- No `difficulty` or `estimated_hours` fields
- Banned patterns updated: no "Learn X", "Read X", "Watch X" — titles must be concept names
- Example good checkpoint: `"Velocity-Verlet Symplectic Integration"` not `"Learn how integrators work"`

### 2.3 Universal Skill Tree — Visual Rebuild

The current galaxy view needs to become a proper ever-growing tree:

- Single root node (you), main branches emerge dynamically from skill data
- Fully dynamic domain clustering — LLM classifies each skill into domains, creates new domains as needed
- Completed quests from project trees automatically light up nodes
- Job demand highlights — branches required by target JDs glow differently
- Radial tree layout that expands outward as nodes are added
- Eventually spans to represent your entire learning journey

Data feeds into universal tree:
- Project trees — completed quests unlock skills
- Work page — job skills extracted from JDs
- Future: courses, certifications, reading completions

### 2.4 RAG Improvements

- Dynamic context — Mimir chat knows which quest you're on, biases search toward matched resources
- Chat history persistence — currently React state only, lost on refresh
- Gap analysis — `GET /gaps/:treeId` — identify what's missing from library for a given tree
- Better distance threshold tuning — currently 0.85, may need per-domain calibration

---

## V3 Features

### 3.1 Mimir as Unified Agent

Everything AI-powered consolidates under one agent (Mimir) with tool-calling:

| Tool | Model | Purpose |
|---|---|---|
| `search_library(query)` | Claude Sonnet | RAG across all chunks |
| `generate_tree(context)` | Kimi-k2 | Tree JSON generation |
| `analyze_repo(url)` | Kimi-k2 | GitHub analysis + concept graph |
| `scrape_url(url)` | Haiku / Gemini Flash | Scraper summarization |
| `match_resources(node_id)` | MiniLM embeddings | Auto-match quests to library |
| `rerank(chunks, query)` | llama-3.1-8b-instant | Fast relevance filtering |

Mimir orchestrates everything. One conversation drives the full workflow: analyze repo → generate tree → match resources → answer questions while climbing.

### 3.2 RAG + Library Before Quest Generation

Before generating quests, RAG across Mimir library to find relevant resources the user already has. Quests reference specific resources: "Watch this video in your library" or "Read this chapter you already ingested." Requires library to be rich enough first.

### 3.3 MCP Integration & Deployment

Deploy scraper to Railway or Oracle Cloud. Expose Mimir tools via MCP server so external tools (Claude Code, other agents) can query your knowledge library. Build `/batch` endpoint for bulk operations.

---

## Ideas Backlog

Not prioritized. Review after climbing the 4 trees.

### Scraper
- Auto-detect YouTube links on resource-list pages and offer playlist-style ingest
- Better page type detection — wiki, forum, course platform-specific extractors
- Re-scrape stale resources automatically when content changes

### Tree
- Tree spreading — more horizontal spacing, longer labels before truncation
- Quest difficulty visual indicators on tree nodes
- Estimated time remaining shown on branch/phase nodes
- Tree comparison — how does your pluto tree compare to someone else's

### Learning
- Spaced repetition — resurface completed quests for review after N days
- Quest notes export — compile all notes from a tree into a study document
- Resource recommendations — "you're missing content on X, here are 3 sources"
- Progress sharing — shareable skill tree snapshots

### Universal Tree
- Skill decay — skills fade if not practiced/reviewed
- Skill prerequisites visualization — show what unlocks what
- Learning velocity — how fast are you acquiring skills over time
- Domain comparison vs job market — which domains are you over/under-indexed in

---

## Immediate Action Items

In order:

1. Ingest PDFs + playlists into Mimir library — section-aware chunking now works
2. Generate trees for active projects
3. Climb trees — take notes, use Mimir chat, document pain points
4. Add 50+ jobs to jobs page
5. Review pain points → prioritize V2 features

---

*Build the tree. Climb it. Understand everything.*