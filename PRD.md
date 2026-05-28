# Yggdrasil — Product Requirements Document
**Version:** 2.6  
**Updated:** May 2026

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

## Current State (v2.6)

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
| Mimir Library — Resource gap finder | ✅ Done | Surfaces resources most relevant to unmastered checkpoints (agentic suggestions) |
| Mimir Library — Study Map | ✅ Done | Ranks resources by checkpoint coverage; groups matched checkpoints by section for reading-order guidance; frontier toggle; per-project filter |
| Mimir Chat — Hybrid RAG | ✅ Done | RRF over pgvector cosine + Postgres FTS; Groq LLaMA 3.3-70b synthesis, section+page citations |
| Mimir Chat — Reranking | ✅ Done | llama-3.1-8b-instant reranker, top-3 selection |
| Mimir Chat — Session memory | ✅ Done | Persisted per (tree_id, node_id); last 10 messages injected into context |
| Mimir Chat — Pre-matched chunk loading | ✅ Done | Node context bypasses cold retrieval; pre-loads chunks from mimir_node_links |
| Mimir Chat — Tree context awareness | ✅ Done | Phase breakdown (overall %, per-phase checkpoints done/total, library coverage) in system prompt; node_is_active branching for tutor vs. tree-only mode |
| Mimir Chat — GraphRAG traversal | ✅ Done | Concept graph walk before cosine search; prerequisite/successor context injected into synthesis |
| Mimir — Native Rust | ✅ Done | No Node.js sidecar — split across mimir_ingest / mimir_retrieval / mimir_tags / mimir_manage |
| Mimir — Retrieval logging | ✅ Done | Every RAG call writes full stats to mimir_retrieval_logs |
| Embeddings | ✅ Done | pplx-embed-v1-0.6b (1024-dim) via OpenRouter |
| Tree Generation — PRD | ✅ Done | Gemini Flash, concept graph, topo sort, PRD profile stage |
| Tree Generation — GitHub repo | ✅ Done | GitHub API, source file analysis, concept graph, repo profile stage |
| Tree Generation — Paper context | ✅ Done | Optional paper URL or PDF (arXiv supported); fed into Phase 1 concept graph + Phase 2 repo profile + per-skill expansion |
| Tree Generation — Two-stage pipeline | ✅ Done | Phase 1 concept graph → Phase 2 tree gen with graph-ordered context; stable with graceful fallback |
| Tree Generation — Regenerate with KG bridge | ✅ Done | regenerate_tree reads mastered concepts from universal_skills; passes them as already-known context; diff-based carry-over preserves notes + completion state by concept_id |
| Tree Rendering | ✅ Done | Custom Canvas (YggdrasilTree.tsx), tapered filled branches, polar coordinate layout (boughs/limbs/twigs), atmospheric roots, checkpoint completion |
| Tree Rendering — Leaf states | ✅ Done | Bud / sprout / leaf / bloomed states with breathing pulse, progress ring, orbiting sparkles |
| Tree Versioning | ✅ Done | tree_versions table (migration 034); concept_id + concept_slug stable identity; diff-based state carry-over on regenerate |
| Node panel — Source citations | ✅ Done | Shows section title + page range for every matched chunk from Mimir resources |
| Auto-matching checkpoints to resources | ✅ Done | pgvector cosine match on checkpoint title + description; persists matched chunk + section + page range |
| Resource matching — async + reranking | ✅ Done | MatchResourceToNodes job variant in orchestrator; in-memory reranking with same-tree boost + lexical overlap boost; top-5 at threshold < 0.55 |
| HNSW index on tree_nodes.title_embedding | ✅ Done | migration 036; ANN search for resource→node matching; vector(1024) column on tree_nodes |
| Background job queue | ✅ Done | orchestrator.rs JobQueue (tokio mpsc); RematchAllNodes, ReembedResources, InferSkillDeps, AutoTagResources, MatchResourceToNodes; emits ygg-* events |
| Event-driven frontend | ✅ Done | UI subscribes to ygg-* Tauri events; Rust owns long-running state machines |
| Read-model helpers | ✅ Done | 5 purpose-built Tauri commands in read_models.rs; get_node_neighborhood added |
| Node panel — Graph Neighborhood | ✅ Done | Collapsible prerequisites/dependents/siblings section in NodePanel; clickable rows select node on canvas; progress bar, lock icon, skill level badge, resource pills |
| Prompt & model version logging | ✅ Done | prompt_logs table; all call_llm sites instrumented; get_prompt_stats command; dev-only Model logs tab |
| Shared reqwest::Client | ✅ Done | Single client managed as Tauri state; injected into brain, mimir, orchestrator commands |
| God module splits | ✅ Done | brain.rs → llm_client + github + prompt_builders + tree_persistence; mimir.rs → mimir_ingest + mimir_retrieval + mimir_tags + mimir_manage |
| Universal Skill Tree — Canvas V2 | ✅ Done | Canvas-rendered radial tree with domain classification, replacing D3 force galaxy |
| Universal Skills — Domain-agnostic schema | ✅ Done | skill_domains table seeded; kind widened to 6 values; review_needed + status columns; skill_evidence first-class rows |
| Universal Skills — Provenance columns | ✅ Done | origin + state columns live (migration 037); origin ∈ (resume|ontology|job_gap|resource|tree_quest|work); state ∈ (seed|adjacent); backfilled from evidence JSONB |
| Universal Skills — Canonicalization | ✅ Done | skill_aliases table + merge UI; alias lookups case-insensitive |
| Skills Graph — Lane layout redesign | ✅ Done | Domain-grouped lane layout, hover highlighting (focused subgraph), zoom-adaptive labels, legend, double-click to zoom, fit-all control |
| Phase 2 Skill Graph — Prereq paths | ✅ Done | `get_prereq_path` BFS walk from gap skill backward through prerequisite edges to nearest seed; powers SkillPanel prerequisite display |
| Phase 3 Skill Graph — Growth recommendations + Learning Path | ✅ Done | `get_growth_recommendations` (job-demand-weighted gap × demand); `compute_learning_path` Steiner-tree-style ordered acquisition path feeding Daily Matrix |
| Jobs Page | ✅ Done | Kanban + skill gap detection + dynamic season selection (term + year picker) |
| Jobs Page — Tailored Projects | ✅ Done | TailoredProjectsPanel ranks resume projects by skill overlap against JD; `get_tailored_projects` + `save_tailored_projects` persist selection to `job_applications.tailored_projects` JSONB (migration 041) |
| Resume Page | ✅ Done | Auto-parse, skill extraction; parsed projects persisted to `resume_projects` table (handles NULL tech_stack defensively) |
| Work Page | ✅ Done | Co-op tracker, skill extraction |
| Daily Matrix | ✅ Done | Eisenhower 2x2 triage for quests + free-form tasks |
| Resource Reading Progress | ✅ Done | `resource_reading_progress` table (migration 040); per-section completion by `(resource_id, section_title)` with optional page range; `mark_section_read` / `mark_sections_read_up_to` / `get_reading_progress` commands |

---

## Technical Architecture

### Stack

| Layer | Technology | Status |
|---|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind | ✅ |
| Desktop | Tauri 2.0 | ✅ |
| Main backend | Rust + sqlx | ✅ |
| Database | Postgres on Neon (pgvector enabled) | ✅ |
| AI generation | Gemini Flash via OpenRouter (tree gen + concept graphs) | ✅ |
| AI chat / extraction | Groq — LLaMA 3.3-70b-versatile + 3.1-8b-instant | ✅ |
| Tree visualization | Custom HTML Canvas (tapered filled branches, polar layout) | ✅ |
| Work/Skills galaxy | Custom HTML Canvas (radial tree, same renderer as YggdrasilTree.tsx) | ✅ |
| HNSW index | pgvector HNSW on tree_nodes.title_embedding vector(1024) (migration 036) | ✅ |
| Transcript jobs | async transcript_jobs retry queue (migrations 038-039); transcript_source/mode/chars on mimir_resources | ✅ |
| Mimir | Native Rust — mimir_ingest / mimir_retrieval / mimir_tags / mimir_manage | ✅ |
| Hybrid retrieval | RRF over pgvector cosine + Postgres FTS (GIN tsvector) | ✅ |
| Chat session memory | mimir_chat_sessions + mimir_chat_messages tables | ✅ |
| Background jobs | tokio mpsc JobQueue in orchestrator.rs; ygg-* Tauri events | ✅ |
| Observability | prompt_logs + mimir_retrieval_logs tables; dev UI in MimirChat | ✅ |
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

### 1. Smarter retrieval signals

`usage_weight` column on `mimir_chunks`, feedback signals (`mimir_feedback` table), dynamic context injection in `mimir_chat`. The agentic suggestions / gap finder / study map all rely on cold matching today — feedback closes the loop.

### 2. Browser extension

One-click ingest of the current page into Mimir, auto-tag by domain, optional link-to-checkpoint picker. Lowers ingest friction for the long tail of resources discovered while browsing.

### 3. V2 Build Sequence

In order — each depends on the previous being stable:

1. ~~**Chat history persistence**~~ ✅ Done — mimir_chat_sessions + mimir_chat_messages; session memory in Groq context
2. ~~**Hybrid retrieval**~~ ✅ Done — RRF over cosine + FTS; retrieval logging
3. ~~**Graph-aware Mimir retrieval**~~ ✅ Done — GraphRAG concept walk before cosine search; prerequisite/successor context in synthesis
4. ~~**Mimir chat tree context awareness**~~ ✅ Done — project/tree name, phase breakdown, node_is_active branching for tutor vs. tree-only mode
5. ~~**Mimir agentic suggestions / resource gap finder**~~ ✅ Done — surfaces resources most relevant to unmastered checkpoints
6. ~~**Universal Skill Tree visual rebuild**~~ ✅ Done — canvas-rendered radial tree with domain classification
7. ~~**Tree versioning + diff-based updates**~~ ✅ Done — tree_versions table, concept_id stable identity, regenerate_tree KG bridge + diff carry-over
8. ~~**Async resource→node matching with reranking**~~ ✅ Done — MatchResourceToNodes job variant; HNSW index on title_embedding; in-memory same-tree + lexical reranking
9. ~~**Graph neighborhood panel**~~ ✅ Done — get_node_neighborhood command; NodePanel prerequisites/dependents/siblings section with clickable rows
10. ~~**Recursive prerequisite paths**~~ ✅ Done — `get_prereq_path` BFS walk from gap to seed; ordered learning path per gap
11. ~~**Graph-aware gap planner**~~ ✅ Done — `get_growth_recommendations` + `compute_learning_path` aggregate growth targets, dedup shared prereqs, produce ranked acquisition sequence for Daily Matrix
12. ~~**Skills graph visual redesign**~~ ✅ Done — domain-grouped lane layout, hover highlighting, zoom-adaptive labels, legend, double-click zoom, fit-all
13. ~~**Tailored projects per job**~~ ✅ Done — `get_tailored_projects` ranks resume projects by skill overlap; `save_tailored_projects` persists to `job_applications.tailored_projects` (migration 041)
14. ~~**Resource reading progress**~~ ✅ Done — `resource_reading_progress` table (migration 040); per-section completion tracking
15. **Smarter retrieval** — usage_weight column on mimir_chunks, feedback signals (mimir_feedback table), dynamic context injection
16. **Browser extension** — one-click ingest of the current page into Mimir, auto-tag by domain, optional link-to-checkpoint picker

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

### Tree structure

```
Trunk (phase)
  └── Branch (skill — can be locked)
        └── Leaf (concept checkpoint — never locked directly)
```

- **Trunk** = a learning phase (e.g. "Foundations", "Core Algorithms"). Never locked.
- **Branch** = a skill within that phase. The first skill per phase starts unlocked; subsequent skills start `is_locked=true`. A branch unlocks the moment its predecessor branch reaches 100% progress.
- **Leaf** = a concept checkpoint. Leaves are always accessible when their parent branch is unlocked — `is_locked` is only ever set on branches. Progress on branches and trunks is the rolling average of their children, cascaded upward on every checkpoint completion.

**Frontier:** A locked branch whose immediately-preceding sibling just reached 100% is the "frontier" — the next skill to unlock. In practice, the unlock cascade fires eagerly on checkpoint completion, so frontier windows collapse immediately and `frontier_count` is typically 0 at any given steady state.

Checkpoints have no difficulty rating or estimated time. They're concepts — some take an afternoon, some take a week. You'll know when you're there.

### Migration

All existing leaf nodes (quests) are migrated to the checkpoint model. The `tasks` JSONB field on leaf nodes is repurposed: instead of a checklist of tasks, it holds `{ mastery_criteria, exercises, notes }`.

---

## Resume-Seeded Skill Graph

### Vision

The resume is not proof of mastery — it is a starting point. When you upload a resume, Yggdrasil seeds the Universal Skill Tree with baseline nodes representing what you claim to know. The graph then grows outward from those seeds via ontology-derived dependency edges: if you have "Backpropagation" on your resume, the graph automatically surfaces "Chain Rule" and "Matrix Calculus" as adjacent concepts that you may or may not actually understand.

Job descriptions feed in from the opposite direction as directional signal: required skills become `gap` nodes that pull the graph toward them. The planner bridges the two — here's where you are, here's where the job needs you, here's the path.

### Node States

| State | Meaning | Primary Source | Status |
|---|---|---|---|
| `seed` | Claimed on resume — baseline, not verified mastery | resume_profile parsed JSON | ✅ Live (migration 037) |
| `adjacent` | Default for all other skills — ontology neighbor, co-op extract, tree quest | skill_dependencies / tree gen / work | ✅ Live (migration 037) |
| `gap` | Required by target JD(s) but absent or low-level in your graph | job_skills demand analysis | Planned |
| `growth_target` | User-designated or planner-recommended next node to climb | user action / gap planner | Planned |
| `mastered` | Checkpoint completed at level ≥ 4 | tree quest completion | Planned |

### Edge Semantics

`skill_dependencies.relationship` values:

| Edge type | Direction | Meaning |
|---|---|---|
| `prerequisite` | A → B | Must understand A before B makes sense |
| `part_of` | A → B | A is a sub-concept of B |
| `specialization` | A → B | A is a specific instance of B |
| `related` | A ↔ B | Conceptually adjacent, no strict ordering |
| `co_occurs` | A ↔ B | Frequently appear together in practice |

### Skill Origin Values

`universal_skills.origin` tracks how a skill entered the graph:

| Origin | Meaning |
|---|---|
| `resume` | Extracted from uploaded resume |
| `tree_quest` | Created when climbing a project tree |
| `work` | Extracted from co-op work resources |
| `resource` | Inferred from a Mimir resource |
| `ontology` | Inferred via skill dependency inference |
| `job_gap` | Required by a job description |

### Build Roadmap

**Phase 1 — Local neighborhoods (✅ Done)**
`get_node_neighborhood` command returns prerequisites, dependents, and siblings for any checkpoint. Shown in the NodePanel `GRAPH NEIGHBORHOOD` section. Traversal uses `concept_slug` as the bridge between `tree_nodes` and `universal_skills → skill_dependencies`. Resource pills surface matched Mimir content per neighbor.

**Phase 2 — Recursive prerequisite paths (next)**
Given a `gap` node (e.g. a JD-required skill at level 0), walk the prerequisite chain recursively to find the full learning path from your current `seed` nodes to the gap. Surface as an ordered list of checkpoints, each linked to a tree or flagged for tree generation.
- Query: breadth-first walk from gap node backward through `prerequisite` edges, stopping at any node already at level ≥ 2
- Output: `GapPath { gap_skill, path: Vec<SkillStep>, estimated_depth: u32 }`
- New Tauri command: `get_gap_path(skill_id: String) -> Result<GapPath, String>`

**Phase 3 — Graph-aware gap planning (later)**
The planner aggregates all `growth_target` nodes, computes shared prerequisite subtrees (dedup), and produces a ranked sequence of skills to acquire that maximally covers the most job gaps with the least total work.
- Uses Steiner-tree approximation or greedy shared-subtree heuristic over the dependency graph
- Output feeds the Daily Matrix as "schedule" quadrant suggestions

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

### 2.4 Browser Extension

A lightweight Chrome/Firefox extension that turns any page into a Mimir resource in one click:

- Toolbar button on every tab — click to send the current URL to the desktop app
- Desktop app exposes a localhost ingest endpoint (or a registered deep link) that accepts URLs from the extension
- Optional: pick a tree + checkpoint on click, so the resource is pre-linked on ingest
- Auto-tag by domain (arxiv.org → `paper`, youtube.com → `video`, github.com → `repo`)
- Paste-and-send text selection — for short passages that don't warrant a full page scrape
- Works offline — queues URLs if the desktop app isn't running, flushes on reconnect

### 2.5 Tree Versioning & Diff-Based Updates

Trees today are one-shot — regenerating discards everything. V2 treats each generation as a revision:

- `tree_versions (id, tree_id, generation, source_fingerprint, created_at)` — one row per generation
- Every node has a stable `concept_id` independent of its surrogate row id; regeneration reconciles by concept, not by position
- **Diff view** when a new version is generated: added concepts, removed concepts, renamed/rescoped concepts, reordered prerequisites
- User accepts changes per-concept — accepted additions land as new leaves, accepted removals are archived (not deleted), renames carry forward the user's notes and completion state
- Source fingerprint = hash of (repo commit SHA, paper URL, PRD text) so re-runs against unchanged input are no-ops
- Enables: update a tree when the repo gains new features, swap the paper, revise the PRD — without losing your climb

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

Dark organic canvas tree — the project tree renderer (`YggdrasilTree.tsx`) already ships this aesthetic:
- Deep night-sky gradient background, canopy glow, floating pollen motes
- Tapered filled branches (trunk → domain boughs → skill limbs → checkpoint twigs) with bark gradients
- Surface roots fanning from trunk base with ground fog
- Pulsing amber glow at the domain junction
- Leaf nodes: botanical bezier shape, 4 states (dormant/budding/growing/bloomed), orbiting sparkles on mastered
- For the Universal Skill Tree: same renderer scaled up, circular nodes replacing leaf shapes, domain clusters replacing phase spines

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

**Chat history persistence** ✅ Done

Chat history is persisted in `mimir_chat_sessions` (unique on `(tree_id, node_id)`) and `mimir_chat_messages`. Sessions resume automatically; last 10 messages are injected into context. `get_chat_session` and `clear_chat_session` Tauri commands manage lifecycle.

**Dynamic context**

Mimir always knows:
- Which quest node is selected in the tree canvas → biases retrieval and tone
- Which resources are already linked to that node → avoids redundant suggestions
- Your current skill level on the relevant skill nodes → calibrates explanation depth

The system prompt is rebuilt on each query incorporating this dynamic context, not just a static template.

### V2 — Mimir Chat Improvements

A cluster of upgrades that turn Mimir chat from a stateless Q&A surface into a learning companion that knows where you are in the tree.

**Knowledge graph augmentation**

Every chat query first consults the personal knowledge graph (from V2 Skills Page Rebuild) and enriches the retrieved chunks with adjacent concept context — prerequisites, successors, and sibling skills. Responses reference the graph position explicitly: "This concept sits between X (which you've mastered) and Y (next up)."

**Session memory per checkpoint**

Chat history is scoped to `(tree_id, node_id)` — opening the same checkpoint resumes the prior conversation, not a blank slate.

- New table: `mimir_chat_sessions (id, tree_id, node_id, created_at, last_active_at)` — unique on (tree_id, node_id)
- `mimir_chat_messages (id, session_id, role, content, sources JSONB, created_at)`
- Last 10 messages of the checkpoint's session are injected into context on every query for continuity
- Switching checkpoints switches conversations; the previous one is preserved and resumable

**Checkpoint-aware context pre-loading**

When a checkpoint panel opens, Mimir pre-warms retrieval in the background — embedding the title + description, fetching the top-k chunks, and caching them. First question is answered without a cold retrieval round-trip.

- Pre-loaded chunks surfaced as "Starter references" inside the panel before any question is asked
- Cache invalidates when the linked resource set changes

**Agentic suggestions**

Mimir proactively surfaces next actions inside the chat panel, not just answers to typed questions:

- "You've read 3 of 5 sources on this checkpoint. Want me to quiz you on what's left?"
- "This concept's prerequisite (Chain Rule) hasn't been touched in 4 weeks — review first?"
- "You asked about backpropagation three times today. Want me to draft study notes from your prior conversations?"

Suggestions come from a lightweight trigger set — time since last visit, unread linked resources, repeat question detection, upcoming prerequisite decay.

---

## V3 Features

### 3.1 Fine-Tuning on User Notes

Fine-tune a small local model on everything Yggdrasil knows about how you learn — with checkpoint notes as the primary signal:

**Training data sources**
- **Checkpoint notes** (primary) — what you wrote while climbing trees, paired with the concept title, mastery criteria, and linked resources
- Mimir chat history — questions you asked and responses you rated helpful
- Completed checkpoints — which concepts you mastered and in what order
- Resource highlights (future) — passages you flagged while reading

**What it enables**

True personalisation — the model knows your vocabulary, which analogies click, which prerequisites you actually have vs nominally have, and where you typically get stuck.

- Generates checkpoint mastery criteria calibrated to your actual level
- Writes exercises that match your learning style
- Mimir chat responses reference your own notes: "You wrote about this when you learned X"

**Infrastructure**

Runs locally — a fine-tuned 1-3B parameter model (Phi-3 Mini, Qwen-2, or similar) loaded via llama.cpp or MLX on Apple Silicon. No cloud inference cost.

### 3.2 Anki / Spaced Repetition Export

Every checkpoint becomes a spaced-repetition candidate:

- Export a tree (or a set of completed checkpoints) as an `.apkg` Anki deck
- Card front = concept name + mastery criteria; card back = your notes + linked resources
- Cloze-deletion cards auto-generated from checkpoint notes where you marked key phrases
- Re-export round-trips review history back into `daily_quest_links` as "schedule" quadrant items when a card's interval lapses
- Decouples long-term retention from keeping the desktop app open

### 3.3 Collaborative Trees

Trees stop being strictly personal:

- Share a tree snapshot via a stable URL — read-only by default, with notes/progress hidden
- "Fork" a shared tree — clone it into your own account with a fresh climb state
- Compare mode — side-by-side view of your tree vs another user's, with a diff over concept coverage and completion
- Tree-level comments — attach questions or corrections to specific checkpoints; the author can merge them back
- Use cases: study groups, mentor-authored curricula, course TAs handing out scaffolded trees, public portfolios of mastered domains

### 3.4 Mobile Companion

Read-only mobile app for the two activities that don't need a desktop:

- **Review** — pull up any checkpoint, read notes, read linked resources (browser hand-off for PDFs)
- **Capture** — voice or text notes that sync back into the desktop app's inbox, later triaged into checkpoints
- Daily Matrix widget — today's "do" quadrant on the lock screen / home screen
- Offline-first — queue mutations, sync on reconnect
- Not a tree editor — generation and canvas interaction stay on desktop

### 3.5 MCP Integration & Deployment

Expose Mimir tools via MCP server so external tools (Claude Code, other agents) can query your personal knowledge library:

```
mimir.search(query)      → ranked chunks from your library
mimir.graph(concept)     → prerequisite graph for a concept
mimir.status(skill)      → your current level + evidence
```

Deploy Python scraper to Railway or Oracle Cloud for always-on ingest without the desktop app running.

---

## Technical Roadmap

Infrastructure and architecture work that isn't a user-facing feature but unblocks or amplifies one. Organized by when it makes sense to do the work, not by what it is.

### Do Soon

Foundation work that every V2 feature benefits from. Worth doing before the next major build pass so later features don't pile more weight onto shaky ground.

- ~~**Rust-owned orchestrator + event-driven UI**~~ ✅ Done — `orchestrator.rs` JobQueue + `ygg-*` events; frontend is event-driven.
- ~~**Alias/canonicalization layer for skills**~~ ✅ Done — `skill_aliases` table + merge UI + case-insensitive lookups.
- ~~**Boundary validation**~~ ✅ Done — Zod schemas at Tauri invoke call-sites.
- ~~**HNSW index + async resource matching**~~ ✅ Done — migration 036; `MatchResourceToNodes` orchestrator job; in-memory reranking.
- ~~**Graph neighborhood in node panel**~~ ✅ Done — `get_node_neighborhood` command; prerequisites/dependents/siblings with resource pills.
- **Recursive prerequisite paths** — `get_gap_path` command; breadth-first walk from gap skill back to seeds; powers Phase 2 of resume-seeded skill graph.
- **Local cache for hot desktop state** — a small SQLite or sled cache in `%APPDATA%` for frequently-read data (project list, active tree summary, skill totals). Cold-start feels instant; Postgres reads only on explicit invalidation.
- **pgvector profiling and threshold tuning** — measure cosine score distributions across real resources, tune the auto-match threshold per-domain (PDFs vs videos vs web pages have different score floors).

### Do As You Build Next Features

Work that doesn't need to happen upfront but should land alongside the next major feature that would benefit. Think of these as "when you're already in this area."

- ~~**Hybrid retrieval**~~ ✅ Done — RRF over pgvector cosine + Postgres FTS (migration 030); retrieval logging in `mimir_retrieval_logs`.
- ~~**Read-model helpers in Rust**~~ ✅ Done — `read_models.rs` with 4 purpose-built commands.
- ~~**Background async jobs for scraper/rematch/inference**~~ ✅ Done — `orchestrator.rs` JobQueue; `ygg-*` progress events.
- ~~**Prompt/model version logging**~~ ✅ Done — `prompt_logs` table (migration 031); all LLM call sites instrumented; `get_prompt_stats` command + dev UI.

### Later

Foundational upgrades that are only worth doing once the core product is battle-tested and the user base or usage pattern demands it.

- **Fuller local-first sync model** — CRDT or log-based sync between local cache and Postgres so the app works fully offline, not just on cache hits. Only matters if mobile companion (V3) or multi-device use materialises.
- **Richer eval dashboard** — an internal page that tracks retrieval precision@k, tree generation quality scores, skill extraction accuracy on a held-out set. Needed once there's enough usage history for the numbers to be meaningful.
- **More advanced graph analytics** — centrality, clustering, shortest-path queries over the knowledge graph. Powers features like "which skill unlocks the most downstream nodes" or "what's your critical path to a target role." Valuable once the graph is dense enough for the analytics to be non-trivial.

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