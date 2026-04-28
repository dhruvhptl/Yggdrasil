# Resume-Seeded Skill Graph — Phase 1 Foundation + Growth Recommendations

**Date:** 2026-04-28  
**Status:** Approved  
**Approach:** Option A — Pure DB columns + Rust in-memory BFS

---

## Overview

Adds provenance (`origin`) and graph-position (`state`) to every skill, seeds the graph from resume skills, and surfaces a Growth Plan — a ranked list of job-demanded skills reachable from your resume seeds via the existing dependency graph.

---

## 1. Database & Migration

**File:** `src-tauri/migrations/037_skill_provenance.sql`

Two new columns on `universal_skills`:

```sql
ALTER TABLE universal_skills
  ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'tree_quest'
    CHECK(origin IN ('resume','ontology','job_gap','resource','tree_quest','work')),
  ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'adjacent'
    CHECK(state IN ('seed','adjacent'));
```

**Note:** `gap` and `target` are **derived display states** — computed at read time from `SkillGap[]` and `GrowthTarget[]` respectively. They are never stored in the DB.

**Backfill in migration:**
```sql
UPDATE universal_skills SET origin = 'resume', state = 'seed'
WHERE evidence::text LIKE '%"type":"resume"%';

UPDATE universal_skills SET origin = 'work', state = 'adjacent'
WHERE evidence::text LIKE '%"type":"work_resource"%'
  AND NOT evidence::text LIKE '%"type":"resume"%';
```

Everything else stays at defaults (`origin='tree_quest', state='adjacent'`).

---

## 2. Rust Backend

### 2a. `upsert_skill` changes (`skill_commands.rs`)

Add two new parameters: `origin: &str`, `state: &str`.

- `origin` is included in INSERT and **omitted from DO UPDATE** (sticky provenance — set once, never overwritten)
- `state` is included in both INSERT and DO UPDATE (recomputed on every sync)

### 2b. Sync function updates

All five call sites pass explicit values:

| Call site | origin | state |
|-----------|--------|-------|
| `sync_resume_inner` | `'resume'` | `'seed'` |
| `sync_trees_inner` | `'tree_quest'` | `'adjacent'` |
| `sync_work_inner` | `'work'` | `'adjacent'` |
| `write_mimir_resource_evidence` (orchestrator.rs) | `'resource'` | `'adjacent'` |

### 2c. `expand_seed_neighbors` (`skill_commands.rs`)

Full recomputation pass — not a one-way promotion. Called at the end of `sync_all_skills`.

Algorithm:
1. `UPDATE universal_skills SET state='seed' WHERE origin='resume'`
2. `UPDATE universal_skills SET state='adjacent' WHERE origin != 'resume'`
3. Return count of rows where state changed (for toast feedback)

The BFS walk over `skill_dependencies` is reserved for a future ontology expansion pass when external concept nodes are added. For now, `origin='resume'` fully determines seed membership.

### 2d. New Tauri command: `expand_skill_graph`

```rust
#[tauri::command]
pub async fn expand_skill_graph(
    database: State<'_, Database>,
) -> Result<usize, String>
```

Calls `expand_seed_neighbors`. Registered in `main.rs`.

### 2e. `get_growth_recommendations` (`read_models.rs`)

```rust
pub struct GrowthTarget {
    pub skill_id: String,
    pub skill_name: String,
    pub rationale: String,
    pub job_relevance_score: f32,
    pub prereq_distance: i32,       // hops from nearest seed; -1 = disconnected
    pub nearest_seed: String,       // which seed skill is closest; empty if disconnected
    pub prereq_path: Vec<String>,   // path from seed to this skill; empty if disconnected
    pub has_resources: bool,
    pub job_count: i64,
    pub final_score: f32,
    pub is_reachable: bool,         // false = no path from any seed
}
```

**Query logic:**

1. Load job skills with frequency counts (filtered by `season` if provided)
2. Load all `universal_skills` (id, name, state, level)
3. Load all `skill_dependencies` into in-memory adjacency map (`HashMap<String, Vec<String>>`)
4. Find gap skills: demanded by jobs, absent from `universal_skills` OR level ≤ 1
5. Find seeds: all skills with `state='seed'`
6. For each gap skill, BFS **forward from seeds** along `source_skill_id → target_skill_id` edges (max depth 3) to find shortest hop count from any seed to the gap skill — track the full path and which seed was nearest
7. Split into two buckets:
   - **Reachable**: path found from at least one seed
   - **Disconnected**: no path from any seed (`prereq_distance = -1`, `is_reachable = false`)
8. Score reachable gaps:
   ```
   final_score = (job_frequency / total_jobs * 0.4)
               + (is_required_ratio * 0.2)
               + (1.0 / (prereq_distance + 1) * 0.3)
               + (has_resources ? 0.1 : 0.0)
   ```
   Where:
   - `job_frequency` = number of jobs listing this skill / total jobs
   - `is_required_ratio` = (jobs where `job_skills.is_required = true` for this skill) / (total jobs listing this skill)
   - `has_resources` = `EXISTS(SELECT 1 FROM mimir_resources WHERE title ILIKE '%{skill_name}%')` — simple title match, no tree node join needed
   
   `rationale = "Required by {job_count} jobs · {prereq_distance} steps from {nearest_seed}"`
9. Return top 3 reachable (by `final_score DESC`) + top 2 disconnected (by `job_frequency DESC`)

**Tauri command:**
```rust
#[tauri::command]
pub async fn get_growth_recommendations(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<GrowthTarget>, String>
```

Registered in `main.rs`.

### 2f. `sync_all_skills` update

After the existing sync + prune + recalculate_levels flow, call `expand_seed_neighbors` to recompute seed/adjacent states cleanly.

### 2g. `get_universal_skills` and `get_skill_graph_snapshot` updates

Add `origin` and `state` to the SELECT in both queries.

---

## 3. TypeScript Types

**`src/types.ts` — `UniversalSkill`:**
```typescript
export interface UniversalSkill {
  // ... existing fields ...
  origin: string;
  state: string;
}
```

**`src/lib/validators.ts` — `SkillSchema`:**
```typescript
export const SkillSchema = z.object({
  // ... existing fields ...
  origin: z.string().optional().default('tree_quest'),
  state: z.string().optional().default('adjacent'),
});
```

---

## 4. Frontend — SkillsPage.tsx

### 4a. State filter chips

New state variable: `stateFilter: 'all' | 'seed' | 'adjacent' | 'gap' | 'target'`

Chips sit in the same horizontal row as the existing review toggle, between the review toggle and the search box. Chips: `All · Seeds · Adjacent · Gaps · Targets`

Color map:
- Seeds → `#f59e0b` (warm gold)
- Adjacent → `#10b981` (emerald, current default)
- Gaps → `#f59e0b` (amber, existing GAP_COLOR)
- Targets → `#34d399` (green)

`filteredDomainGroups` memo gains a filter step before domain grouping:
- `seed` → `s.state === 'seed'`
- `adjacent` → `s.state === 'adjacent'`
- `gap` → skill appears in `gaps` array (by name, case-insensitive)
- `target` → skill appears in `growthTargets` array (by `skill_id`)

### 4b. Canvas node visuals (derived states)

**Seed nodes** (`skill.state === 'seed'`): warm gold outer ring drawn before the standard glow layer; the standard white highlight dot replaced with a 4-point star shape at the node center.

**Target nodes** (skill id appears in `growthTargets`): glow intensity doubled; soft animated outer pulse ring in `#34d399`.

**Adjacent/gap nodes**: unchanged.

### 4c. Growth Plan panel

New state: `showGrowthPlan: boolean`, `growthTargets: GrowthTarget[]`, `loadingGrowthPlan: boolean`

"Growth Plan" button lives in the left panel header row alongside Export and Collapse buttons. Calls `get_growth_recommendations` on open (lazy load).

Panel: overlaid on the canvas, left-anchored (`top: 16, left: sidebarWidth + 16`), width 300, same glass-morphism style as `SkillPanel`.

Layout per reachable target row:
- Skill name (bold) + domain badge
- Rationale: `"Required by 8 jobs · 2 steps from Python"`
- Prereq breadcrumb: `Python → NumPy → SciPy` (slate-600 text, `→` separator)
- Score bar (thin, `#34d399`, width = `finalScore * 100%`)
- Has-resources dot (emerald dot if true, slate if false)
- "Generate tree" button → `navigate('/?prefill=' + encodeURIComponent(skillName))`

Disconnected targets shown below a faint divider labeled "No clear path from your seeds" — same row layout minus the prereq breadcrumb.

"Expand skill graph" button at panel bottom → calls `expand_skill_graph`, shows count toast.

---

## 5. Frontend — ResumePage.tsx

In `LoadedState`, the skills section heading changes from `"Skills"` to `"Baseline Skills (seeds)"`.

Each skill chip:
- Gets a small `#f59e0b` filled circle (5px, `inline-block`, `rounded-full`) to the left of the skill name text
- Gets `title="This skill seeds your knowledge graph — it defines your starting point, not your ceiling"` on the chip wrapper

---

## 6. Build Order

1. Migration `037_skill_provenance.sql`
2. `upsert_skill` signature update + all call sites
3. `expand_seed_neighbors` + `expand_skill_graph` command
4. `get_growth_recommendations` + Tauri command
5. `get_universal_skills` + `get_skill_graph_snapshot` SELECT updates
6. `sync_all_skills` call to `expand_seed_neighbors`
7. Register new commands in `main.rs`
8. `types.ts` + `validators.ts` updates
9. `SkillsPage.tsx` — state filter chips + canvas visuals + Growth Plan panel
10. `ResumePage.tsx` — seed indicators

---

## 7. Key Decisions

- **`origin` is sticky**: set on INSERT, never touched in DO UPDATE. Provenance doesn't change.
- **`state` is recomputed**: set in both INSERT and DO UPDATE. Full recompute on every `sync_all_skills`.
- **`gap` and `target` are display-only**: never stored in DB. Derived from `SkillGap[]` and `GrowthTarget[]` at render time.
- **`expand_seed_neighbors` is a full recompute**: `state='seed'` for `origin='resume'` skills, `state='adjacent'` for everything else. No stale labels.
- **BFS for prereq paths**: in-memory over loaded `skill_dependencies`. Disconnected gap skills returned separately with `is_reachable=false`, ranked by job frequency only.
- **Disconnected targets**: shown in Growth Plan under a separate divider, not folded into distance-based ranking with a fake max.
