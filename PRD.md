# Yggdrasil — Product Requirements Document
**Version:** 2.2  
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

Generate and climb trees for active projects. Take notes in checkpoint panels. Use Mimir chat while climbing. Document every pain point — these feed the V2 roadmap.

### 3. Add Remaining Jobs

Expand the job tracker. Target: 50+ jobs before reviewing skill gap analysis.

### 4. V2 Build Sequence (after climbing)

In order — each depends on the previous being stable:

1. **Smarter retrieval** — usage_weight column, feedback signals, dynamic context injection
2. **Chat history persistence** — mimir_chat_sessions + mimir_chat_messages tables
3. **Knowledge graph schema** — typed edges, prerequisite traversal, resource attachment to skill nodes
4. **Universal Skill Tree visual rebuild** — canvas renderer, organic tree layout, glowing nodes
5. **Graph-aware Mimir retrieval** — graph traversal before cosine search

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

## V2 Features

### 2.1 Scraper Upgrade

The current scraper works for most sites but has limitations:

- Better `extract_text()` for div-heavy sites (currently misses content in non-semantic HTML)
- Audio transcription via Whisper for YouTube videos with no captions
- Video description + chapters as fallback when transcript unavailable
- Proper encoding handling — UTF-8 forced on Windows (`sys.stdout.reconfigure`)

### 2.2 GitHub Repo Reader Upgrade

Current `analyze_repo` in `brain.rs` does shallow file fetching. V2 upgrade:

- Tree-sitter Rust crate — parse source files into structured symbols (functions, classes, imports)
- Call graph analysis — which functions call which
- Import map — what each file imports from where
- Richer LLM context — structured symbol map instead of raw file dumps
- Result: checkpoints reference specific patterns the author actually used, not just library names

### 2.3 Universal Skill Tree — Visual Rebuild

The current D3 galaxy view becomes a proper knowledge graph — the visual centrepiece of the app:

- **Visual**: dark organic canvas, warm amber/green gradient background, twisted branches, glowing circular nodes — inspired by mythological Yggdrasil
- **Structure**: Domain → Concept → Technical Skill hierarchy
- **Typed edges**: `requires`, `builds_on`, `applies`, `referenced_by`
- Single root node (you), domain branches emerge dynamically from skill data
- Completed quests from project trees automatically light up dependent skill nodes via prerequisite traversal
- Job demand highlights — branches required by target JDs glow differently
- Resources attach directly to skill nodes, not just quest nodes
- Radial tree layout that expands outward as skills are acquired

Data feeds:
- Project trees — completed checkpoints unlock and illuminate skill nodes
- Work page — extracted skills from co-op resources
- Job tracker — demand shapes which branches to grow toward
- Resume — pre-populates existing skills

---

## Mimir Intelligence Roadmap

### Current State

Mimir is a RAG system — it indexes resources and retrieves relevant chunks via cosine similarity. Quality improves as the library grows, but there is no feedback loop and no memory. Every query starts from scratch.

```
Query → embed → cosine search → rerank → synthesize → response
                     ↑
               no feedback, no weighting, no history
```

### V2 — Smarter Retrieval

**Feedback signals on chat responses**

After each Mimir chat response, the user can rate it (helpful / not helpful). This signal is stored and used to adjust future retrieval weighting for the sources that contributed to that response.

- New table: `mimir_feedback (id, chunk_id, response_id, signal, created_at)`
- Helpful responses boost `usage_weight` on contributing chunks
- Not-helpful responses suppress those chunks for similar queries

**Usage-weighted retrieval**

Resources the user reads and completes get boosted in similarity search. A resource sitting unread at 0% contributes equally to one with 20 highlights and a complete badge — this fixes that.

- New column: `usage_weight FLOAT NOT NULL DEFAULT 1.0` on `mimir_chunks`
- Boosted by: chunk retrieved → user engages with source → `usage_weight += 0.1`
- Boosted by: resource marked complete → all its chunks `usage_weight += 0.5`
- Applied in cosine search: `similarity_score * usage_weight` as effective retrieval score

**Dynamic context injection**

Mimir knows which quest you're currently viewing and biases retrieval toward resources already linked to that node. The selected node's title + description are prepended to the query embedding.

### V2 — Knowledge Graph (Skills Page Rebuild)

Skills, concepts, resources, and quests become nodes in a unified knowledge graph. This replaces the current D3 force galaxy.

**Graph schema**

```
Nodes: Domain | Concept | TechnicalSkill | Resource | Quest
Edges (typed):
  requires      — Concept A must be understood before Concept B
  builds_on     — TechnicalSkill extends a Concept
  applies       — Quest demonstrates a TechnicalSkill
  referenced_by — Resource teaches a Concept or TechnicalSkill
```

**Prerequisite traversal**

Completing a quest lights up the skill nodes it `applies`. Reaching 100% on a skill node unlocks successor nodes connected by `requires` edges — the same unlock mechanic already in project trees, generalised across the universal graph.

**Resource attachment**

Resources attach to skill and concept nodes directly (via `referenced_by` edges), not just to quest nodes. Mimir auto-match runs against the full graph, not just leaf nodes.

**Job demand integration**

The job tracker's skill demand scores flow into the graph as edge weights. Branches leading to high-demand skills glow; the gap between current level and demand score is visualised as branch thickness or color.

**Visual design**

Dark organic canvas tree — the same aesthetic as the project tree renderer, scaled up:
- Warm amber/deep green gradient background
- Twisted procedural branches from a single root node (you)
- Glowing circular nodes: dim for locked, softly lit for in-progress, bright for mastered
- Domain nodes are larger, brighter, higher in the canopy
- Skill nodes cluster around their domain branch
- Resource nodes appear as small satellites orbiting skill nodes

### V2 — Personal Knowledge Graph as Librarian

Mimir stops being a document search engine and becomes a navigator of your personal knowledge graph.

**Graph-aware retrieval**

When you ask Mimir a question, it doesn't just cosine-search chunks — it first locates the relevant concept nodes in the knowledge graph, then retrieves resources attached to those nodes and their prerequisites.

```
Query: "explain backpropagation"
→ Find concept node: Backpropagation
→ Traverse: requires → Chain Rule, Matrix Calculus
→ Retrieve: resources on all three concepts (ordered by prerequisites)
→ Synthesise: "You're working on Backpropagation. You'll need Chain Rule first.
               You have these resources in your library..."
```

**Chat history persistence**

Chat history currently lives only in React state and is lost on refresh. V2 persists it:

- New table: `mimir_chat_sessions (id, created_at)` and `mimir_chat_messages (id, session_id, role, content, created_at)`
- Sessions are resumed automatically on the same tree/node context
- History window: last 10 messages injected into context for continuity

**Dynamic context**

Mimir always knows:
- Which quest node is selected in the tree canvas → biases retrieval and tone
- Which resources are already linked to that node → avoids redundant suggestions
- Your current skill level on the relevant skill nodes → calibrates explanation depth

The system prompt is rebuilt on each query incorporating this dynamic context, not just a static template.

---

## V3 Features

### 3.1 Fine-Tuning on Personal Learning History

Fine-tune a small local model on everything Yggdrasil knows about how you learn:

**Training data sources**
- Quest checkpoint notes — what you wrote while climbing trees
- Mimir chat history — questions you asked and responses you rated helpful
- Completed checkpoints — which concepts you mastered and in what order
- Resource highlights (future) — passages you flagged while reading

**What it enables**

True personalisation — the model knows how you learn, what analogies click for you, which prerequisites you actually have vs nominally have, and where you typically get stuck.

- Generates checkpoint mastery criteria calibrated to your actual level
- Writes exercises that match your learning style
- Mimir chat responses reference your own notes: "You wrote about this when you learned X"

**Infrastructure**

Runs locally via the Python sidecar (port 3002) — a fine-tuned 1-3B parameter model (Phi-3 Mini, Qwen-2, or similar) loaded via llama.cpp or MLX on Apple Silicon. No cloud inference cost.

### 3.2 MCP Integration & Deployment

Expose Mimir tools via MCP server so external tools (Claude Code, other agents) can query your personal knowledge library:

```
mimir.search(query)      → ranked chunks from your library
mimir.graph(concept)     → prerequisite graph for a concept
mimir.status(skill)      → your current level + evidence
```

Deploy Python scraper to Railway or Oracle Cloud for always-on ingest without the desktop app running.

---

## Ideas Backlog

Not prioritized. Review after climbing 4+ trees and the V2 knowledge graph is live.

### Scraper
- Auto-detect YouTube links on resource-list pages and offer playlist-style ingest
- Better page type detection — wiki, forum, course platform-specific extractors
- Re-scrape stale resources automatically when content changes

### Tree
- Tree spreading — more horizontal spacing, longer labels before truncation
- Tree comparison — how does your tree compare to someone else's (shareable snapshots)

### Learning
- Spaced repetition — resurface completed checkpoints for review after N days
- Quest notes export — compile all checkpoint notes from a tree into a study document
- Progress sharing — shareable skill tree snapshots

### Universal Tree / Knowledge Graph
- Skill decay — skills fade if not reinforced after N weeks
- Learning velocity — how fast are you acquiring skills over time
- Domain comparison vs job market — which domains are you over/under-indexed in

---

*Build the tree. Climb it. Understand everything.*