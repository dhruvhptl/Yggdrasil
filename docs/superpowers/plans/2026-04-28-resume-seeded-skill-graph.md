# Resume-Seeded Skill Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `origin` and `state` provenance columns to skills, seed the graph from resume skills, and surface a Growth Plan panel recommending job-demanded skills reachable from the user's seed skills.

**Architecture:** Pure DB columns (`origin` sticky, `state` recomputed on sync) with in-memory Rust BFS for growth recommendations. `gap` and `target` are derived display states — never stored. All new Tauri commands follow existing patterns in `skill_commands.rs` and `read_models.rs`.

**Tech Stack:** Rust + sqlx (Postgres $N placeholders), Tauri 2, React 19 + TypeScript, HTML Canvas

---

## File Map

| File | Change |
|------|--------|
| `src-tauri/migrations/037_skill_provenance.sql` | CREATE — new migration |
| `src-tauri/src/skill_commands.rs` | MODIFY — `upsert_skill` signature, sync call sites, `expand_seed_neighbors`, `expand_skill_graph` command, `get_universal_skills` SELECT |
| `src-tauri/src/read_models.rs` | MODIFY — `GrowthTarget` struct, `get_growth_recommendations` command, `get_skill_graph_snapshot` SELECT |
| `src-tauri/src/orchestrator.rs` | MODIFY — `write_mimir_resource_evidence` call to `upsert_skill` |
| `src-tauri/src/main.rs` | MODIFY — register 2 new commands |
| `src/types.ts` | MODIFY — `UniversalSkill` gets `origin` + `state` |
| `src/lib/validators.ts` | MODIFY — `SkillSchema` gets `origin` + `state` |
| `src/pages/SkillsPage.tsx` | MODIFY — state filter chips, canvas seed/target visuals, Growth Plan panel |
| `src/pages/ResumePage.tsx` | MODIFY — "Baseline Skills (seeds)" label + seed dot on each chip |

---

## Task 1: Migration 037

**Files:**
- Create: `src-tauri/migrations/037_skill_provenance.sql`

- [ ] **Step 1: Write the migration**

```sql
-- src-tauri/migrations/037_skill_provenance.sql
ALTER TABLE universal_skills
  ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'tree_quest'
    CHECK(origin IN ('resume','ontology','job_gap','resource','tree_quest','work')),
  ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'adjacent'
    CHECK(state IN ('seed','adjacent'));

-- Backfill: resume evidence → seed
UPDATE universal_skills SET origin = 'resume', state = 'seed'
WHERE evidence::text LIKE '%"type":"resume"%';

-- Backfill: work evidence (no resume) → work/adjacent
UPDATE universal_skills SET origin = 'work', state = 'adjacent'
WHERE evidence::text LIKE '%"type":"work_resource"%'
  AND NOT evidence::text LIKE '%"type":"resume"%';
```

- [ ] **Step 2: Verify migration runs**

```bash
bash dev.sh
```

Expected: app starts, no migration errors in console. Check logs for `Applied migration 037_skill_provenance`.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/migrations/037_skill_provenance.sql
git commit -m "feat: migration 037 — origin + state provenance columns on universal_skills"
```

---

## Task 2: Update `upsert_skill` signature and all call sites

**Files:**
- Modify: `src-tauri/src/skill_commands.rs`
- Modify: `src-tauri/src/orchestrator.rs`

- [ ] **Step 1: Update `upsert_skill` in `skill_commands.rs`**

Replace the function signature and body at line 152. The INSERT gains `origin` and `state`; DO UPDATE sets `state` but not `origin`:

```rust
pub(crate) async fn upsert_skill(
    pool: &PgPool,
    name: &str,
    domain: Option<&str>,
    evidence_entry: serde_json::Value,
    origin: &str,
    state: &str,
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Empty skill name".to_string());
    }
    let normalized = normalize_skill_name(trimmed);
    let canonical_name = if normalized.is_empty() { trimmed } else { &normalized };
    let slug = canonical_name.to_lowercase().replace(' ', "-");
    let id = Uuid::new_v4().to_string();
    let evidence_arr = serde_json::json!([evidence_entry]);

    let no_domain = domain.is_none();
    let row = sqlx::query(
        "INSERT INTO universal_skills
             (id, name, concept_slug, domain, level, evidence, last_updated, status, review_needed, origin, state)
         VALUES ($1, $2, $3, $4, 1, $5::jsonb, NOW(),
                 CASE WHEN $4 IS NULL THEN 'unclassified' ELSE 'active' END,
                 $6, $7, $8)
         ON CONFLICT (LOWER(name)) DO UPDATE SET
             evidence = universal_skills.evidence || $5::jsonb,
             concept_slug = COALESCE(universal_skills.concept_slug, EXCLUDED.concept_slug),
             domain = COALESCE($4, universal_skills.domain),
             state = EXCLUDED.state,
             last_updated = NOW()
         RETURNING id"
    )
    .bind(&id)
    .bind(canonical_name)
    .bind(&slug)
    .bind(domain)
    .bind(&evidence_arr)
    .bind(no_domain)
    .bind(origin)
    .bind(state)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("upsert_skill failed: {}", e))?;

    row.try_get::<String, _>("id").map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Update `sync_resume_inner` call site**

Find the `upsert_skill(pool, skill_name, None, evidence)` call inside `sync_resume_inner` (around line 223) and add the two new args:

```rust
upsert_skill(pool, skill_name, None, evidence, "resume", "seed").await?;
```

- [ ] **Step 3: Update `sync_trees_inner` call site**

Find the `upsert_skill(pool, &title, None, evidence)` call inside `sync_trees_inner` (around line 266) and update:

```rust
upsert_skill(pool, &title, None, evidence, "tree_quest", "adjacent").await?;
```

- [ ] **Step 4: Update `sync_work_inner` call site**

Find the `upsert_skill(pool, &skill_name, None, evidence)` call inside `sync_work_inner` (around line 300) and update:

```rust
upsert_skill(pool, &skill_name, None, evidence, "work", "adjacent").await?;
```

- [ ] **Step 5: Update `write_mimir_resource_evidence` call site in `orchestrator.rs`**

Find the `upsert_skill(pool, &node_title, None, evidence.clone())` call at line 683 and update:

```rust
let skill_id = match crate::skill_commands::upsert_skill(pool, &node_title, None, evidence.clone(), "resource", "adjacent").await {
```

- [ ] **Step 6: Build to verify compilation**

```bash
cd src-tauri && cargo build 2>&1 | head -50
```

Expected: compiles with no errors (warnings about unused fields are fine).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/skill_commands.rs src-tauri/src/orchestrator.rs
git commit -m "feat: add origin/state params to upsert_skill, update all call sites"
```

---

## Task 3: `expand_seed_neighbors` + `expand_skill_graph` command

**Files:**
- Modify: `src-tauri/src/skill_commands.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Add `expand_seed_neighbors` function to `skill_commands.rs`**

Add after `sync_concept_slugs_inner` at the bottom of the file:

```rust
/// Full recompute of seed/adjacent states.
/// Seeds = skills with origin='resume'. Everything else = adjacent.
/// Returns count of rows updated.
pub(crate) async fn expand_seed_neighbors(pool: &PgPool) -> Result<usize, String> {
    let seed_result = sqlx::query(
        "UPDATE universal_skills SET state = 'seed' WHERE origin = 'resume'"
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let adjacent_result = sqlx::query(
        "UPDATE universal_skills SET state = 'adjacent' WHERE origin != 'resume'"
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = (seed_result.rows_affected() + adjacent_result.rows_affected()) as usize;
    println!("🌱 expand_seed_neighbors: {} seeds, {} adjacent",
        seed_result.rows_affected(), adjacent_result.rows_affected());
    Ok(total)
}

#[tauri::command]
pub async fn expand_skill_graph(
    database: State<'_, Database>,
) -> Result<usize, String> {
    expand_seed_neighbors(&database.pool).await
}
```

- [ ] **Step 2: Update `sync_all_skills` to call `expand_seed_neighbors`**

Find the end of `sync_all_skills` in `skill_commands.rs` — after `recalculate_levels_inner` is called (around line 441), add:

```rust
    expand_seed_neighbors(&database.pool).await?;

    println!("🌳 Synced all skills: resume={}, trees={}, work={}", r1.upserted, r2.upserted, r3.upserted);
    Ok(vec![r1, r2, r3])
```

(Replace the existing `println!` + `Ok(vec![...])` block with this.)

- [ ] **Step 3: Register `expand_skill_graph` in `main.rs`**

In `main.rs`, add to the `invoke_handler` list after `skill_commands::reset_skill_domains`:

```rust
            skill_commands::expand_skill_graph,
```

- [ ] **Step 4: Build to verify**

```bash
cd src-tauri && cargo build 2>&1 | head -50
```

Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/skill_commands.rs src-tauri/src/main.rs
git commit -m "feat: expand_seed_neighbors full recompute + expand_skill_graph command"
```

---

## Task 4: `get_growth_recommendations` command

**Files:**
- Modify: `src-tauri/src/read_models.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Add `GrowthTarget` struct to `read_models.rs`**

Add after the `SkillGraphSnapshot` struct (around line 924):

```rust
// ─── GrowthTarget ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTarget {
    pub skill_id: String,
    pub skill_name: String,
    pub rationale: String,
    pub job_relevance_score: f32,
    pub prereq_distance: i32,
    pub nearest_seed: String,
    pub prereq_path: Vec<String>,
    pub has_resources: bool,
    pub job_count: i64,
    pub final_score: f32,
    pub is_reachable: bool,
}
```

- [ ] **Step 2: Add `get_growth_recommendations` function to `read_models.rs`**

Add after the `GrowthTarget` struct:

```rust
#[tauri::command]
pub async fn get_growth_recommendations(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<GrowthTarget>, String> {
    let pool = &database.pool;

    // 1. Load job skill demand (filtered by season)
    let job_rows = if let Some(ref s) = season {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS job_count,
                    (SELECT COUNT(*) FROM job_applications WHERE season = $1) AS total_jobs,
                    SUM(CASE WHEN js.is_required THEN 1 ELSE 0 END)::float /
                        NULLIF(COUNT(DISTINCT js.job_id), 0) AS is_required_ratio
             FROM job_skills js
             JOIN job_applications ja ON js.job_id = ja.id
             WHERE ja.season = $1
             GROUP BY js.skill_name"
        )
        .bind(s)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS job_count,
                    (SELECT COUNT(*) FROM job_applications) AS total_jobs,
                    SUM(CASE WHEN js.is_required THEN 1 ELSE 0 END)::float /
                        NULLIF(COUNT(DISTINCT js.job_id), 0) AS is_required_ratio
             FROM job_skills js
             GROUP BY js.skill_name"
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    if job_rows.is_empty() {
        return Ok(vec![]);
    }

    let total_jobs: i64 = job_rows.first()
        .and_then(|r| r.try_get("total_jobs").ok())
        .unwrap_or(1);
    if total_jobs == 0 {
        return Ok(vec![]);
    }

    // 2. Load all universal_skills (id, name, state, level) for gap detection
    let skill_rows = sqlx::query(
        "SELECT id, name, state, level FROM universal_skills"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // name (lowercase) → (id, state, level)
    let skill_by_name: std::collections::HashMap<String, (String, String, i32)> = skill_rows.iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let id: String = r.try_get("id").ok()?;
            let state: String = r.try_get("state").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((name.to_lowercase(), (id, state, level)))
        })
        .collect();

    // 3. Load skill_dependencies into adjacency map: skill_id → Vec<target_skill_id>
    let dep_rows = sqlx::query(
        "SELECT source_skill_id, target_skill_id FROM skill_dependencies"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for r in &dep_rows {
        let src: String = r.try_get("source_skill_id").unwrap_or_default();
        let tgt: String = r.try_get("target_skill_id").unwrap_or_default();
        if !src.is_empty() && !tgt.is_empty() {
            adj.entry(src).or_default().push(tgt);
        }
    }

    // 4. Seed IDs and names
    let seeds: Vec<(String, String)> = skill_rows.iter()
        .filter_map(|r| {
            let state: String = r.try_get("state").ok()?;
            if state != "seed" { return None; }
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            Some((id, name))
        })
        .collect();

    // 5. Check resources: skill_name → has_resources (title ILIKE match)
    // Batch: load all resource titles once
    let resource_titles: Vec<String> = sqlx::query("SELECT title FROM mimir_resources")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.try_get::<String, _>("title").ok())
        .map(|t| t.to_lowercase())
        .collect();

    // 6. BFS from all seeds simultaneously (multi-source BFS), max depth 3
    // Track: skill_id → (distance, path_of_names, nearest_seed_name)
    use std::collections::VecDeque;
    struct BfsEntry {
        skill_id: String,
        distance: i32,
        path: Vec<String>, // skill names from seed to current node (inclusive)
        seed_name: String,
    }

    // id → name map for path reconstruction
    let id_to_name: std::collections::HashMap<String, String> = skill_rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            Some((id, name))
        })
        .collect();

    let mut visited: std::collections::HashMap<String, (i32, Vec<String>, String)> = std::collections::HashMap::new();
    let mut queue: VecDeque<BfsEntry> = VecDeque::new();

    for (seed_id, seed_name) in &seeds {
        if !visited.contains_key(seed_id) {
            visited.insert(seed_id.clone(), (0, vec![seed_name.clone()], seed_name.clone()));
            queue.push_back(BfsEntry {
                skill_id: seed_id.clone(),
                distance: 0,
                path: vec![seed_name.clone()],
                seed_name: seed_name.clone(),
            });
        }
    }

    while let Some(entry) = queue.pop_front() {
        if entry.distance >= 3 { continue; }
        if let Some(neighbors) = adj.get(&entry.skill_id) {
            for neighbor_id in neighbors {
                if visited.contains_key(neighbor_id) { continue; }
                let neighbor_name = id_to_name.get(neighbor_id).cloned().unwrap_or_default();
                let mut new_path = entry.path.clone();
                new_path.push(neighbor_name.clone());
                visited.insert(neighbor_id.clone(), (
                    entry.distance + 1,
                    new_path.clone(),
                    entry.seed_name.clone(),
                ));
                queue.push_back(BfsEntry {
                    skill_id: neighbor_id.clone(),
                    distance: entry.distance + 1,
                    path: new_path,
                    seed_name: entry.seed_name.clone(),
                });
            }
        }
    }

    // 7. Build growth targets from job demand
    let mut reachable: Vec<GrowthTarget> = Vec::new();
    let mut disconnected: Vec<GrowthTarget> = Vec::new();

    for row in &job_rows {
        let skill_name: String = match row.try_get("skill_name") { Ok(v) => v, Err(_) => continue };
        let job_count: i64 = row.try_get("job_count").unwrap_or(0);
        let is_required_ratio: f64 = row.try_get("is_required_ratio").unwrap_or(0.0);

        // Only surface as a growth target if level <= 1 (gap or absent)
        let level = skill_by_name.get(&skill_name.to_lowercase())
            .map(|(_, _, lvl)| *lvl)
            .unwrap_or(0);
        if level > 1 { continue; }

        let job_frequency = job_count as f32 / total_jobs as f32;

        // Check has_resources via title match
        let name_lower = skill_name.to_lowercase();
        let has_resources = resource_titles.iter().any(|t| t.contains(&name_lower));

        // Check reachability from BFS results
        // Gap skill may not be in universal_skills at all — match by name
        let skill_entry = skill_by_name.get(&name_lower);
        let bfs_result = skill_entry.and_then(|(id, _, _)| visited.get(id));

        let (is_reachable, prereq_distance, nearest_seed, prereq_path) = match bfs_result {
            Some((dist, path, seed_name)) if *dist > 0 => {
                // dist > 0 means it was reached from a seed (seeds themselves have dist=0)
                (true, *dist, seed_name.clone(), path.clone())
            }
            _ => (false, -1i32, String::new(), vec![]),
        };

        let final_score = if is_reachable {
            (job_frequency * 0.4)
                + (is_required_ratio as f32 * 0.2)
                + (1.0 / (prereq_distance + 1) as f32 * 0.3)
                + (if has_resources { 0.1 } else { 0.0 })
        } else {
            job_frequency
        };

        let rationale = if is_reachable {
            format!("Required by {} job{} · {} step{} from {}",
                job_count, if job_count == 1 { "" } else { "s" },
                prereq_distance, if prereq_distance == 1 { "" } else { "s" },
                nearest_seed)
        } else {
            format!("Required by {} job{} · no path from your seeds",
                job_count, if job_count == 1 { "" } else { "s" })
        };

        let skill_id = skill_entry.map(|(id, _, _)| id.clone()).unwrap_or_default();

        let target = GrowthTarget {
            skill_id,
            skill_name,
            rationale,
            job_relevance_score: job_frequency,
            prereq_distance,
            nearest_seed,
            prereq_path,
            has_resources,
            job_count,
            final_score,
            is_reachable,
        };

        if is_reachable {
            reachable.push(target);
        } else {
            disconnected.push(target);
        }
    }

    reachable.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap_or(std::cmp::Ordering::Equal));
    disconnected.sort_by(|a, b| b.job_count.cmp(&a.job_count));

    let mut result: Vec<GrowthTarget> = reachable.into_iter().take(3).collect();
    result.extend(disconnected.into_iter().take(2));

    Ok(result)
}
```

- [ ] **Step 3: Register command in `main.rs`**

Add after `read_models::get_node_neighborhood`:

```rust
            read_models::get_growth_recommendations,
```

- [ ] **Step 4: Build to verify**

```bash
cd src-tauri && cargo build 2>&1 | head -60
```

Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/read_models.rs src-tauri/src/main.rs
git commit -m "feat: GrowthTarget struct + get_growth_recommendations command (BFS reachability scoring)"
```

---

## Task 5: Update `get_universal_skills` and `get_skill_graph_snapshot` SELECTs

**Files:**
- Modify: `src-tauri/src/skill_commands.rs`
- Modify: `src-tauri/src/read_models.rs`

- [ ] **Step 1: Update `UniversalSkill` struct in `skill_commands.rs`**

Find the `UniversalSkill` struct (around line 39) and add two fields:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalSkill {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub level: i32,
    pub evidence: serde_json::Value,
    pub last_updated: String,
    pub review_needed: bool,
    pub status: String,
    pub origin: String,
    pub state: String,
}
```

- [ ] **Step 2: Update `get_universal_skills` query and mapping**

Find `get_universal_skills` (around line 448). Update the SELECT:

```rust
    let rows = sqlx::query(
        "SELECT id, name, domain, level, evidence, last_updated, review_needed, status, origin, state
         FROM universal_skills ORDER BY review_needed DESC, level DESC, name ASC"
    )
```

And add the two new fields to the struct construction in the `.map()`:

```rust
            origin: r.try_get("origin").unwrap_or_else(|_| "tree_quest".to_string()),
            state: r.try_get("state").unwrap_or_else(|_| "adjacent".to_string()),
```

- [ ] **Step 3: Update `get_skill_graph_snapshot` query and mapping in `read_models.rs`**

Find the skills query inside `get_skill_graph_snapshot` (around line 932). Update the SELECT to include `origin` and `state`:

```rust
        sqlx::query(
            "SELECT us.id, us.name,
                    COALESCE(sd.name, us.domain) AS domain,
                    us.level, us.evidence, us.last_updated, us.review_needed, us.status,
                    us.origin, us.state
             FROM universal_skills us
             LEFT JOIN skill_domains sd ON sd.id = us.domain_id
             ORDER BY us.review_needed DESC, us.level DESC, us.name ASC"
        ).fetch_all(&database.pool),
```

And update the `UniversalSkill` construction in the skills mapping (around line 969):

```rust
    let skills: Vec<UniversalSkill> = skills_rows.iter().map(|r| {
        Ok(UniversalSkill {
            id: r.try_get("id").map_err(|e: sqlx::Error| e.to_string())?,
            name: r.try_get("name").map_err(|e: sqlx::Error| e.to_string())?,
            domain: r.try_get("domain").map_err(|e: sqlx::Error| e.to_string())?,
            level: r.try_get("level").map_err(|e: sqlx::Error| e.to_string())?,
            evidence: r.try_get("evidence").map_err(|e: sqlx::Error| e.to_string())?,
            last_updated: r.try_get::<chrono::DateTime<chrono::Utc>, _>("last_updated")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e: sqlx::Error| e.to_string())?,
            review_needed: r.try_get("review_needed").unwrap_or(false),
            status: r.try_get("status").unwrap_or_else(|_| "active".to_string()),
            origin: r.try_get("origin").unwrap_or_else(|_| "tree_quest".to_string()),
            state: r.try_get("state").unwrap_or_else(|_| "adjacent".to_string()),
        })
    }).collect::<Result<Vec<_>, String>>()?;
```

- [ ] **Step 4: Build to verify**

```bash
cd src-tauri && cargo build 2>&1 | head -60
```

Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/skill_commands.rs src-tauri/src/read_models.rs
git commit -m "feat: add origin + state to UniversalSkill struct and all SELECT queries"
```

---

## Task 6: TypeScript types + validators

**Files:**
- Modify: `src/types.ts`
- Modify: `src/lib/validators.ts`

- [ ] **Step 1: Update `UniversalSkill` in `types.ts`**

Find the `UniversalSkill` interface (around line 111) and add two fields:

```typescript
export interface UniversalSkill {
  id: string;
  name: string;
  domain: string | null;
  level: number;
  evidence: SkillEvidence[];
  lastUpdated: string;
  reviewNeeded: boolean;
  status: string;
  origin: string;
  state: string;
}
```

- [ ] **Step 2: Update `SkillSchema` in `validators.ts`**

Find the `SkillSchema` object (around line 101) and add two fields:

```typescript
export const SkillSchema = z.object({
  id: z.string(),
  name: z.string(),
  domain: z.string().nullable(),
  level: z.number(),
  evidence: z.array(SkillEvidenceSchema),
  lastUpdated: z.string(),
  reviewNeeded: z.boolean().optional().default(false),
  status: z.string().optional().default('active'),
  origin: z.string().optional().default('tree_quest'),
  state: z.string().optional().default('adjacent'),
});
```

- [ ] **Step 3: Start app and verify no TypeScript errors**

```bash
bash dev.sh
```

Expected: app compiles and loads skills page without console errors.

- [ ] **Step 4: Commit**

```bash
git add src/types.ts src/lib/validators.ts
git commit -m "feat: add origin + state to UniversalSkill TS type and SkillSchema validator"
```

---

## Task 7: SkillsPage — state filter chips

**Files:**
- Modify: `src/pages/SkillsPage.tsx`

- [ ] **Step 1: Add `GrowthTarget` type import and state variables**

At the top of `SkillsPage.tsx`, add the import for `GrowthTarget` (we'll add it to `types.ts` inline here):

In `src/types.ts`, add after `SkillAlias`:

```typescript
export interface GrowthTarget {
  skillId: string;
  skillName: string;
  rationale: string;
  jobRelevanceScore: number;
  prereqDistance: number;
  nearestSeed: string;
  prereqPath: string[];
  hasResources: boolean;
  jobCount: number;
  finalScore: number;
  isReachable: boolean;
}
```

Then in `SkillsPage.tsx`, add to the imports from `../types`:

```typescript
import type { UniversalSkill, SkillGap, SkillDependency, SkillAlias, SkillGraphSnapshot, GrowthTarget } from '../types';
```

- [ ] **Step 2: Add new state variables to `SkillsPage`**

In the `SkillsPage` component function, after the existing state declarations, add:

```typescript
  const [stateFilter, setStateFilter] = useState<'all' | 'seed' | 'adjacent' | 'gap' | 'target'>('all');
  const [growthTargets, setGrowthTargets] = useState<GrowthTarget[]>([]);
  const [showGrowthPlan, setShowGrowthPlan] = useState(false);
  const [loadingGrowthPlan, setLoadingGrowthPlan] = useState(false);
  const [growthPlanToast, setGrowthPlanToast] = useState<string | null>(null);
```

- [ ] **Step 3: Add `loadGrowthPlan` handler**

After the `handleResetDomains` function:

```typescript
  async function handleOpenGrowthPlan() {
    setShowGrowthPlan(true);
    if (growthTargets.length === 0) {
      setLoadingGrowthPlan(true);
      try {
        const targets = await invoke<GrowthTarget[]>('get_growth_recommendations', {});
        setGrowthTargets(targets);
      } catch (e) {
        console.error('get_growth_recommendations failed:', e);
      } finally {
        setLoadingGrowthPlan(false);
      }
    }
  }

  async function handleExpandSkillGraph() {
    try {
      const count = await invoke<number>('expand_skill_graph');
      setGrowthPlanToast(`Updated ${count} skill states`);
      await loadAll();
      setGrowthTargets([]);
      setTimeout(() => setGrowthPlanToast(null), 3000);
    } catch (e) {
      console.error('expand_skill_graph failed:', e);
    }
  }
```

- [ ] **Step 4: Update `filteredDomainGroups` memo to respect `stateFilter`**

Find the `filteredDomainGroups` useMemo (around line 1018). Replace it with:

```typescript
  const gapNameSet = useMemo(() =>
    new Set(gaps.map(g => g.skillName.toLowerCase())),
  [gaps]);

  const targetIdSet = useMemo(() =>
    new Set(growthTargets.map(t => t.skillId)),
  [growthTargets]);

  const filteredDomainGroups = useMemo(() => {
    let base = showReviewOnly ? skills.filter(s => s.reviewNeeded) : skills;

    if (stateFilter === 'seed') {
      base = base.filter(s => s.state === 'seed');
    } else if (stateFilter === 'adjacent') {
      base = base.filter(s => s.state === 'adjacent');
    } else if (stateFilter === 'gap') {
      base = base.filter(s => gapNameSet.has(s.name.toLowerCase()));
    } else if (stateFilter === 'target') {
      base = base.filter(s => targetIdSet.has(s.id));
    }

    const filtered = searchQuery.trim()
      ? base.filter(s => s.name.toLowerCase().includes(searchQuery.toLowerCase()))
      : base;

    const map = new Map<string, UniversalSkill[]>();
    for (const s of filtered) {
      const d = s.domain || 'General';
      if (!map.has(d)) map.set(d, []);
      map.get(d)!.push(s);
    }
    return map;
  }, [skills, showReviewOnly, searchQuery, stateFilter, gapNameSet, targetIdSet]);
```

- [ ] **Step 5: Add state filter chips to the sidebar UI**

Find the review toggle block in the sidebar (around line 1431, the `{!sidebarCollapsed && gaps.length > 0 && ...}` section). After the review toggle section, add the state filter chips row:

```tsx
        {!sidebarCollapsed && (
          <div style={{ padding: '6px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', gap: 4, flexWrap: 'wrap' }}>
            {(['all', 'seed', 'adjacent', 'gap', 'target'] as const).map(f => {
              const colors: Record<string, string> = {
                all: '#475569', seed: '#f59e0b', adjacent: '#10b981', gap: '#f59e0b', target: '#34d399',
              };
              const labels: Record<string, string> = {
                all: 'All', seed: 'Seeds', adjacent: 'Adjacent', gap: 'Gaps', target: 'Targets',
              };
              const active = stateFilter === f;
              return (
                <button
                  key={f}
                  onClick={() => setStateFilter(f)}
                  className="text-[9px] px-2 py-0.5 rounded transition-colors"
                  style={{
                    background: active ? `${colors[f]}22` : 'rgba(255,255,255,0.03)',
                    color: active ? colors[f] : '#334155',
                    border: `1px solid ${active ? colors[f] + '44' : 'rgba(255,255,255,0.05)'}`,
                  }}
                >
                  {labels[f]}
                </button>
              );
            })}
          </div>
        )}
```

- [ ] **Step 6: Add "Growth Plan" button to the sidebar header**

Find the collapsed sidebar header buttons section and the expanded header row (around line 1343). In the expanded header's button row (alongside Export and Collapse), add before the collapse button:

```tsx
                  <button onClick={handleOpenGrowthPlan} className="p-1.5 text-slate-700 hover:text-emerald-400 transition-colors" title="Growth Plan">
                    <Sparkles className="w-3 h-3" />
                  </button>
```

Also add to the collapsed sidebar icon column (after the Export button):

```tsx
              <button onClick={handleOpenGrowthPlan} className="p-1.5 text-slate-700 hover:text-emerald-400 transition-colors" title="Growth Plan">
                <Sparkles className="w-3 h-3" />
              </button>
```

- [ ] **Step 7: Commit**

```bash
git add src/types.ts src/pages/SkillsPage.tsx
git commit -m "feat: state filter chips (All/Seeds/Adjacent/Gaps/Targets) in SkillsPage sidebar"
```

---

## Task 8: SkillsPage — canvas seed/target node visuals

**Files:**
- Modify: `src/pages/SkillsPage.tsx`

- [ ] **Step 1: Update `drawSkillNode` to accept `isSeed` and `isTarget` flags**

Find `drawSkillNode` (around line 467). Add two parameters:

```typescript
function drawSkillNode(
  ctx: CanvasRenderingContext2D,
  x: number, y: number,
  color: string,
  level: number,
  isHovered: boolean,
  isSelected: boolean,
  animTime: number,
  domainAlpha: number,
  growFrac: number,
  isSeed: boolean,
  isTarget: boolean,
) {
```

- [ ] **Step 2: Add seed gold ring inside `drawSkillNode`**

After the opening `if (growFrac < 0.01) return;` guard and before the outer glow block, add:

```typescript
  // Seed ring: warm gold outer ring
  if (isSeed) {
    const seedR = r + 5;
    ctx.beginPath();
    ctx.arc(x, y, seedR, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(245,158,11,${(0.70 * domainAlpha).toFixed(3)})`;
    ctx.lineWidth = 1.5;
    ctx.globalAlpha = domainAlpha;
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
```

- [ ] **Step 3: Add target glow pulse inside `drawSkillNode`**

After the seed ring block, add:

```typescript
  // Target glow: double intensity green pulse
  if (isTarget) {
    const pulse = 0.5 + 0.5 * Math.sin(animTime * 1.8);
    const targetR = r * (2.0 + 0.6 * pulse);
    const tg = ctx.createRadialGradient(x, y, 0, x, y, targetR);
    tg.addColorStop(0, `rgba(52,211,153,${(0.28 * pulse * domainAlpha).toFixed(3)})`);
    tg.addColorStop(1, `rgba(52,211,153,0)`);
    ctx.beginPath();
    ctx.arc(x, y, targetR, 0, Math.PI * 2);
    ctx.fillStyle = tg;
    ctx.fill();
  }
```

- [ ] **Step 4: Update the canvas draw loop to pass `isSeed` and `isTarget`**

Find the `placements.forEach` loop in the canvas useEffect (around line 1118). Update the `drawSkillNode` call:

```typescript
      if (p.skill) {
        const isSeed = p.skill.state === 'seed';
        const isTarget = targetIdSet.has(p.skill.id);
        drawSkillNode(ctx, p.x, p.y, p.color, p.skill.level, isHovered, isSelected, animTime, domAlpha, growFrac, isSeed, isTarget);
      } else {
        drawGapNode(ctx, p.x, p.y, animTime, domAlpha);
      }
```

Also add `targetIdSet` to the `useEffect` dependency array for the canvas render.

- [ ] **Step 5: Build and visually verify**

```bash
bash dev.sh
```

Expected: seed skills show a warm gold ring; target skills (if any in growth plan) show a green pulse. No console errors.

- [ ] **Step 6: Commit**

```bash
git add src/pages/SkillsPage.tsx
git commit -m "feat: seed gold ring + target green pulse on canvas skill nodes"
```

---

## Task 9: SkillsPage — Growth Plan panel

**Files:**
- Modify: `src/pages/SkillsPage.tsx`

- [ ] **Step 1: Add `GrowthPlanPanel` component**

Add before the `SkillPanel` component (around line 739):

```tsx
// ─── Growth Plan Panel ────────────────────────────────────────────────────────

function GrowthPlanPanel({
  targets,
  loading,
  sidebarWidth,
  onClose,
  onExpand,
  onExpanding,
  toast,
}: {
  targets: GrowthTarget[];
  loading: boolean;
  sidebarWidth: number;
  onClose: () => void;
  onExpand: () => void;
  onExpanding: boolean;
  toast: string | null;
}) {
  const navigate = useNavigate();
  const reachable = targets.filter(t => t.isReachable);
  const disconnected = targets.filter(t => !t.isReachable);

  return (
    <div
      style={{
        position: 'absolute', top: 16, left: sidebarWidth + 16, zIndex: 50,
        width: 300, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: '1px solid rgba(52,211,153,0.18)',
        borderRadius: 16,
        overflow: 'auto',
        boxShadow: '0 12px 48px rgba(0,0,0,0.85), 0 0 0 1px rgba(52,211,153,0.08)',
      }}
      onClick={e => e.stopPropagation()}
    >
      {/* Header */}
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
          <Sparkles size={13} style={{ color: '#34d399' }} />
          <span style={{ fontSize: 13, fontWeight: 600, color: '#f1f5f9' }}>Growth Plan</span>
        </div>
        <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', padding: 2 }}>
          <X size={14} />
        </button>
      </div>

      <div style={{ padding: '10px 16px' }}>
        {loading && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: '#475569', fontSize: 12, padding: '12px 0' }}>
            <Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} />
            Computing growth recommendations…
          </div>
        )}

        {!loading && targets.length === 0 && (
          <div style={{ fontSize: 12, color: '#334155', padding: '12px 0' }}>
            No growth recommendations yet. Add jobs with skill requirements to generate recommendations.
          </div>
        )}

        {!loading && reachable.length > 0 && (
          <div style={{ marginBottom: 12 }}>
            <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>Recommended</div>
            {reachable.map((t, i) => (
              <div key={i} style={{ marginBottom: 12, paddingBottom: 12, borderBottom: i < reachable.length - 1 ? '1px solid rgba(255,255,255,0.04)' : 'none' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 3 }}>
                  <span style={{ fontSize: 12, fontWeight: 600, color: '#e2e8f0' }}>{t.skillName}</span>
                  {t.hasResources && <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#10b981', flexShrink: 0 }} title="Resources available" />}
                </div>
                <div style={{ fontSize: 10, color: '#64748b', marginBottom: 5 }}>{t.rationale}</div>
                {t.prereqPath.length > 1 && (
                  <div style={{ fontSize: 10, color: '#475569', marginBottom: 5 }}>
                    {t.prereqPath.join(' → ')}
                  </div>
                )}
                {/* Score bar */}
                <div style={{ height: 2, background: 'rgba(255,255,255,0.06)', borderRadius: 1, marginBottom: 7, overflow: 'hidden' }}>
                  <div style={{ height: '100%', width: `${Math.min(100, t.finalScore * 100)}%`, background: '#34d399', borderRadius: 1 }} />
                </div>
                <button
                  onClick={() => navigate(`/?prefill=${encodeURIComponent(t.skillName)}`)}
                  style={{ fontSize: 10, padding: '3px 8px', borderRadius: 5, background: 'rgba(52,211,153,0.10)', border: '1px solid rgba(52,211,153,0.22)', color: '#34d399', cursor: 'pointer' }}
                >
                  Generate tree
                </button>
              </div>
            ))}
          </div>
        )}

        {!loading && disconnected.length > 0 && (
          <div style={{ marginTop: 4 }}>
            <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6, paddingTop: 8, borderTop: '1px solid rgba(255,255,255,0.04)' }}>
              No clear path from your seeds
            </div>
            {disconnected.map((t, i) => (
              <div key={i} style={{ marginBottom: 10 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                  <span style={{ fontSize: 11, fontWeight: 600, color: '#94a3b8' }}>{t.skillName}</span>
                  {t.hasResources && <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#10b981', flexShrink: 0 }} />}
                </div>
                <div style={{ fontSize: 10, color: '#475569', marginBottom: 4 }}>{t.rationale}</div>
                <button
                  onClick={() => navigate(`/?prefill=${encodeURIComponent(t.skillName)}`)}
                  style={{ fontSize: 10, padding: '3px 8px', borderRadius: 5, background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)', color: '#475569', cursor: 'pointer' }}
                >
                  Generate tree
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Expand skill graph button */}
        <div style={{ marginTop: 12, paddingTop: 10, borderTop: '1px solid rgba(255,255,255,0.05)' }}>
          {toast && <div style={{ fontSize: 10, color: '#34d399', marginBottom: 6 }}>{toast}</div>}
          <button
            onClick={onExpand}
            disabled={onExpanding}
            style={{
              width: '100%', padding: '6px', borderRadius: 7, border: '1px solid rgba(52,211,153,0.15)',
              background: 'rgba(52,211,153,0.06)', color: '#34d399', fontSize: 11, cursor: 'pointer',
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
            }}
          >
            {onExpanding ? <><Loader2 size={11} style={{ animation: 'spin 1s linear infinite' }} />Expanding…</> : <><RefreshCw size={11} />Expand skill graph</>}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Add `useNavigate` import to `SkillsPage.tsx`**

At the top of the file, add to the React Router import:

```typescript
import { useNavigate } from 'react-router-dom';
```

And in the `SkillsPage` component body, add:

```typescript
  const navigate = useNavigate();
```

- [ ] **Step 3: Wire up `GrowthPlanPanel` in the canvas area JSX**

In the canvas area div (the `div.flex-1.relative.overflow-hidden`), after the `SkillPanel` block (around line 1613), add:

```tsx
        {showGrowthPlan && (
          <GrowthPlanPanel
            targets={growthTargets}
            loading={loadingGrowthPlan}
            sidebarWidth={sidebarCollapsed ? 44 : 276}
            onClose={() => setShowGrowthPlan(false)}
            onExpand={handleExpandSkillGraph}
            onExpanding={false}
            toast={growthPlanToast}
          />
        )}
```

- [ ] **Step 4: Build and verify panel opens**

```bash
bash dev.sh
```

Expected: clicking the Growth Plan sparkle button opens the panel. Panel shows loading state then recommendations (or empty state if no jobs). "Generate tree" navigates to home with prefill.

- [ ] **Step 5: Commit**

```bash
git add src/pages/SkillsPage.tsx
git commit -m "feat: Growth Plan panel with reachable + disconnected targets, generate tree CTA"
```

---

## Task 10: ResumePage seed indicators

**Files:**
- Modify: `src/pages/ResumePage.tsx`

- [ ] **Step 1: Change section heading to "Baseline Skills (seeds)"**

In `LoadedState`, find the skills section (around line 354). The skills are rendered as chips in the profile header `div`. Find:

```tsx
            {resume.skills.length > 0 && (
              <div className="flex flex-wrap gap-2 mt-3">
```

Add a label above the chip row:

```tsx
            {resume.skills.length > 0 && (
              <div className="mt-3">
                <div className="text-xs font-semibold text-slate-500 mb-2">Baseline Skills (seeds)</div>
                <div className="flex flex-wrap gap-2">
```

And close the extra `div` after the chips:

```tsx
                </div>
              </div>
```

- [ ] **Step 2: Add seed dot to each skill chip**

Find each skill chip (the `span` with `emerald-500/15` background inside the skills map). Replace:

```tsx
                  <span
                    key={skill}
                    className="px-2 py-0.5 text-xs font-medium rounded-full bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                  >
                    {skill}
                  </span>
```

With:

```tsx
                  <span
                    key={skill}
                    className="flex items-center gap-1.5 px-2 py-0.5 text-xs font-medium rounded-full bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                    title="This skill seeds your knowledge graph — it defines your starting point, not your ceiling"
                  >
                    <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#f59e0b', flexShrink: 0, display: 'inline-block' }} />
                    {skill}
                  </span>
```

- [ ] **Step 3: Build and visually verify**

```bash
bash dev.sh
```

Expected: resume skills section shows "Baseline Skills (seeds)" label, each skill chip has a small warm gold dot to the left of the text, and hovering shows the tooltip.

- [ ] **Step 4: Commit**

```bash
git add src/pages/ResumePage.tsx
git commit -m "feat: resume page seed indicators — 'Baseline Skills (seeds)' label + gold dot per chip"
```

---

## Self-Review

### Spec coverage check

| Spec requirement | Task |
|-----------------|------|
| Migration 037 with origin + state columns + backfill | Task 1 |
| `upsert_skill` origin sticky, state recomputed | Task 2 step 1 |
| `sync_resume_inner` origin='resume', state='seed' | Task 2 step 2 |
| `sync_trees_inner` origin='tree_quest', state='adjacent' | Task 2 step 3 |
| `sync_work_inner` origin='work', state='adjacent' | Task 2 step 4 |
| `write_mimir_resource_evidence` origin='resource', state='adjacent' | Task 2 step 5 |
| `expand_seed_neighbors` full recompute (seed/adjacent) | Task 3 step 1 |
| `expand_skill_graph` Tauri command registered | Task 3 steps 1+3 |
| `sync_all_skills` calls `expand_seed_neighbors` | Task 3 step 2 |
| `GrowthTarget` struct with all fields incl. `is_reachable` | Task 4 step 1 |
| `get_growth_recommendations` BFS + scoring + split buckets | Task 4 step 2 |
| Disconnected targets shown separately, ranked by frequency | Task 4 step 2 |
| `get_universal_skills` + `get_skill_graph_snapshot` include origin/state | Task 5 |
| `UniversalSkill` TS type gains origin + state | Task 6 step 1 |
| `SkillSchema` validator gains origin + state | Task 6 step 2 |
| `GrowthTarget` TS type | Task 7 step 1 |
| State filter chips (All/Seeds/Adjacent/Gaps/Targets) | Task 7 steps 4-5 |
| Filter chips sit alongside review toggle | Task 7 step 5 |
| filteredDomainGroups respects stateFilter | Task 7 step 4 |
| Seed nodes: warm gold ring on canvas | Task 8 steps 1-3 |
| Target nodes: green pulse on canvas | Task 8 steps 1-3 |
| Growth Plan panel with reachable + disconnected rows | Task 9 step 1 |
| Growth Plan panel prereq breadcrumb | Task 9 step 1 |
| Growth Plan panel score bar | Task 9 step 1 |
| Growth Plan panel "Generate tree" → navigate with prefill | Task 9 step 1 |
| Growth Plan panel "Expand skill graph" button + toast | Task 9 step 1 |
| Growth Plan button in sidebar header | Task 7 step 6 |
| ResumePage "Baseline Skills (seeds)" label | Task 10 step 1 |
| ResumePage seed dot + tooltip on each chip | Task 10 step 2 |

All spec requirements covered. ✓

### Type consistency check

- `GrowthTarget` defined in `types.ts` (Task 7 step 1) — used in `SkillsPage.tsx` and returned by `get_growth_recommendations`. Field names match: `skillId`, `skillName`, `prereqPath`, `isReachable`, `finalScore`, `hasResources`, `jobCount`, `prereqDistance`, `nearestSeed`, `rationale`, `jobRelevanceScore`. ✓
- `expand_seed_neighbors` returns `usize` → `expand_skill_graph` returns `Result<usize, String>` → frontend `invoke<number>`. ✓
- `drawSkillNode` now takes 12 args — all call sites updated in Task 8 step 4. ✓
- `GrowthPlanPanel` `onExpanding` prop is `boolean`, wired as `false` (no separate loading state for expand since it's fast). If you want a loading state, add `expandingGraph` state and pass it — acceptable simplification for v1. ✓
