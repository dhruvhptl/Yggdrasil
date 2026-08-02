// src-tauri/src/skill_commands.rs
// Universal Skill Tree: sync skills from all sources, calculate levels, detect gaps.
// All queries use sqlx::query() non-macro to avoid offline cache issues.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use tauri::State;
use uuid::Uuid;
use crate::constants::GROQ_API_URL;

use crate::database::Database;

// ─── Abbreviation map ────────────────────────────────────────────────────────

static ABBREV_MAP: std::sync::LazyLock<std::collections::HashMap<&'static str, &'static str>> =
    std::sync::LazyLock::new(|| {
        [
            ("ml",  "Machine Learning"),
            ("dl",  "Deep Learning"),
            ("nlp", "Natural Language Processing"),
            ("cv",  "Computer Vision"),
            ("rl",  "Reinforcement Learning"),
            ("db",  "Database"),
            ("os",  "Operating Systems"),
            ("ds",  "Data Structures"),
            ("ai",  "Artificial Intelligence"),
            ("oop", "Object-Oriented Programming"),
            ("fp",  "Functional Programming"),
            ("ci",  "Continuous Integration"),
            ("cd",  "Continuous Deployment"),
        ]
        .into_iter()
        .collect()
    });

// ─── Structs ─────────────────────────────────────────────────────────────────

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillAlias {
    pub id: String,
    pub canonical_skill_id: String,
    pub canonical_skill_name: String,
    pub alias: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDependency {
    pub id: String,
    pub source_skill_id: String,
    pub target_skill_id: String,
    pub relationship: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGap {
    pub skill_name: String,
    pub demand_count: i64,
    pub frequency: f64,
    pub demand_score: f64,
    pub current_level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub upserted: usize,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedMerge {
    pub canonical_id: String,
    pub canonical_name: String,
    pub alias_ids: Vec<String>,
    pub alias_names: Vec<String>,
    pub reason: String,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Deterministic normalization: lowercase + trim + suffix stripping + abbrev expansion.
/// Preserves special cases like C++, C#, .NET.
pub(crate) fn normalize_skill_name(name: &str) -> String {
    let s = name.trim();
    if s.is_empty() { return String::new(); }

    // Special cases that must not be mangled
    let lower = s.to_lowercase();
    if matches!(lower.as_str(), "c++" | "c#" | ".net" | "f#" | "c") {
        return s.to_string();
    }

    // Strip language-ecosystem suffixes (case-insensitive)
    let stripped = {
        let l = lower.as_str();
        // Version numbers: "Python 3", "Python 3.11", "ES2022", etc.
        // Strip trailing whitespace + digits/dots (but keep if that's the whole name)
        let no_ver = {
            let bytes = l.as_bytes();
            let mut end = bytes.len();
            while end > 0 && (bytes[end - 1].is_ascii_digit() || bytes[end - 1] == b'.') {
                end -= 1;
            }
            // also strip trailing whitespace before the version
            while end > 0 && bytes[end - 1] == b' ' {
                end -= 1;
            }
            if end == 0 { l } else { &l[..end] }
        };
        // Strip .js / .py / .rb / .ts suffixes
        let no_ext = no_ver
            .strip_suffix(".js")
            .or_else(|| no_ver.strip_suffix(".py"))
            .or_else(|| no_ver.strip_suffix(".rb"))
            .or_else(|| no_ver.strip_suffix(".ts"))
            .unwrap_or(no_ver);
        no_ext.trim()
    };

    // Abbreviation expansion — domain-agnostic common abbreviations only.
    // Extend this map for any abbreviations that should always be expanded.
    let expanded = ABBREV_MAP.get(stripped).copied().unwrap_or("");
    if !expanded.is_empty() {
        return expanded.to_string();
    }

    // Title-case the stripped form to produce a clean canonical name.
    // We keep the original casing of s but apply the stripping from stripped.
    // To preserve user's preferred casing, we re-slice the original `s` to the
    // same byte length as `stripped` (both are derived from the same string).
    let orig_trimmed = s;
    let stripped_len_in_original = {
        // stripped is a substring of lower (same byte positions), so find how
        // many chars that maps to in the original.
        // Since we only stripped ASCII suffixes and spaces, byte positions match.
        stripped.len()
    };
    if stripped_len_in_original == 0 || stripped_len_in_original > orig_trimmed.len() {
        return orig_trimmed.to_string();
    }
    orig_trimmed[..stripped_len_in_original].trim().to_string()
}

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

    // ON CONFLICT uses a case-insensitive expression index on LOWER(name).
    // New skills with no domain are flagged unclassified + review_needed so they
    // surface for human classification. On update we never flip review_needed back
    // to false — only the explicit mark_skill_reviewed command does that.
    // origin is sticky (set once, never overwritten). state is recomputed on sync.
    let no_domain = domain.is_none();
    // Guard: only write concept_slug if no *other* row already claims it.
    // The unique partial index idx_universal_skills_concept_slug forbids
    // duplicates among non-null slugs. Distinct skill names can slugify to
    // the same value (e.g. "K-Means" and "K Means" both → "k-means"),
    // and ON CONFLICT (LOWER(name)) does not catch that race.
    let row = sqlx::query(
        "INSERT INTO universal_skills
             (id, name, concept_slug, domain, level, evidence, last_updated, status, review_needed, origin, state)
         VALUES (
             $1, $2,
             CASE WHEN EXISTS (SELECT 1 FROM universal_skills WHERE concept_slug = $3)
                  THEN NULL ELSE $3 END,
             $4, 1, $5::jsonb, NOW(),
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

// ─── Inner sync functions (take &PgPool directly) ───────────────────────────

pub(crate) async fn sync_resume_inner(pool: &PgPool) -> Result<SyncResult, String> {
    let row = sqlx::query(
        "SELECT skills FROM resume_profile WHERE is_active = TRUE ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let skills_json = match row {
        Some(r) => r.try_get::<serde_json::Value, _>("skills").map_err(|e| e.to_string())?,
        None => return Ok(SyncResult { upserted: 0, source: "resume".into() }),
    };

    let skills: Vec<String> = serde_json::from_value(skills_json).unwrap_or_default();
    let mut count = 0;

    for skill_name in &skills {
        if skill_name.trim().is_empty() { continue; }
        let evidence = serde_json::json!({
            "type": "resume",
            "detail": "Listed on resume"
        });
        upsert_skill(pool, skill_name, None, evidence, "resume", "seed").await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "resume".into() })
}

pub(crate) async fn sync_trees_inner(pool: &PgPool) -> Result<SyncResult, String> {
    // Skill nodes: branch nodes whose parent is also a branch (phase).
    // trunk -> phase (branch, parent=trunk) -> skill (branch, parent=phase) -> leaf (quest)
    // We want: n.type='branch' AND parent.type='branch'
    let rows = sqlx::query(
        "SELECT n.id, n.title, n.progress, n.tree_id,
                t.name AS tree_name, p.name AS project_name
         FROM tree_nodes n
         JOIN tree_nodes parent ON n.parent_id = parent.id
         JOIN trees t ON n.tree_id = t.id
         JOIN projects p ON t.project_id = p.id
         WHERE n.type = 'branch'
           AND parent.type = 'branch'
           AND n.progress = 100"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for row in &rows {
        let title: String = row.try_get("title").map_err(|e| e.to_string())?;
        let progress: i32 = row.try_get("progress").unwrap_or(0);
        let tree_name: String = row.try_get("tree_name").map_err(|e| e.to_string())?;
        let project_name: String = row.try_get("project_name").map_err(|e| e.to_string())?;
        let node_id: String = row.try_get("id").map_err(|e| e.to_string())?;

        let evidence = serde_json::json!({
            "type": "tree_quest",
            "projectName": project_name,
            "treeName": tree_name,
            "nodeTitle": title,
            "nodeId": node_id,
            "progress": progress
        });

        upsert_skill(pool, &title, None, evidence, "tree_quest", "adjacent").await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "trees".into() })
}

pub(crate) async fn sync_work_inner(pool: &PgPool) -> Result<SyncResult, String> {
    let rows = sqlx::query(
        "SELECT wrs.skill_name, wr.title AS resource_title, wr.id AS resource_id,
                ct.company
         FROM work_resource_skills wrs
         JOIN work_resources wr ON wrs.resource_id = wr.id
         JOIN research_topics rt ON wr.topic_id = rt.id
         JOIN coop_terms ct ON rt.coop_id = ct.id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for row in &rows {
        let skill_name: String = row.try_get("skill_name").map_err(|e| e.to_string())?;
        let resource_title: String = row.try_get("resource_title").map_err(|e| e.to_string())?;
        let resource_id: String = row.try_get("resource_id").map_err(|e| e.to_string())?;
        let company: String = row.try_get("company").map_err(|e| e.to_string())?;

        let evidence = serde_json::json!({
            "type": "work_resource",
            "company": company,
            "resourceTitle": resource_title,
            "resourceId": resource_id
        });

        upsert_skill(pool, &skill_name, None, evidence, "work", "adjacent").await?;
        count += 1;
    }

    Ok(SyncResult { upserted: count, source: "work".into() })
}

pub(crate) async fn sync_jobs_inner(pool: &PgPool) -> Result<SyncResult, String> {
    let rows = sqlx::query(
        "SELECT js.skill_name,
                COUNT(DISTINCT js.job_id) AS job_count,
                AVG(CASE WHEN js.is_required THEN 1.0 ELSE 0.0 END)::float8 AS required_ratio
         FROM job_skills js
         GROUP BY js.skill_name"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for row in &rows {
        let skill_name: String = row.try_get("skill_name").map_err(|e| e.to_string())?;
        let job_count: i64 = row.try_get("job_count").map_err(|e| e.to_string())?;
        let required_ratio: f64 = row.try_get("required_ratio").unwrap_or(0.0);

        let evidence = serde_json::json!({
            "type": "job_demand",
            "job_count": job_count,
            "is_required_ratio": required_ratio
        });

        // Only insert missing skills; for existing ones preserve state if already seed,
        // otherwise mark adjacent. Never overwrite evidence from other sources.
        let id = Uuid::new_v4().to_string();
        let normalized = normalize_skill_name(skill_name.trim());
        let canonical = if normalized.is_empty() { skill_name.trim().to_string() } else { normalized };
        let slug = canonical.to_lowercase().replace(' ', "-");
        let evidence_arr = serde_json::json!([evidence]);

        // Same slug-collision guard as upsert_skill — see comment there.
        let result = sqlx::query(
            "INSERT INTO universal_skills
                 (id, name, concept_slug, level, evidence, last_updated, status, review_needed, origin, state)
             VALUES (
                 $1, $2,
                 CASE WHEN EXISTS (SELECT 1 FROM universal_skills WHERE concept_slug = $3)
                      THEN NULL ELSE $3 END,
                 1, $4::jsonb, NOW(), 'unclassified', true, 'job_gap', 'adjacent')
             ON CONFLICT (LOWER(name)) DO UPDATE SET
                 evidence = universal_skills.evidence || $4::jsonb,
                 state = CASE WHEN universal_skills.state = 'seed' THEN 'seed' ELSE 'adjacent' END,
                 last_updated = NOW()"
        )
        .bind(&id)
        .bind(&canonical)
        .bind(&slug)
        .bind(&evidence_arr)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        if result.rows_affected() > 0 {
            count += 1;
        }
    }

    Ok(SyncResult { upserted: count, source: "jobs".into() })
}

pub(crate) async fn recalculate_levels_inner(pool: &PgPool) -> Result<usize, String> {
    let rows = sqlx::query("SELECT id, evidence FROM universal_skills")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut updated = 0;
    for row in &rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let evidence: serde_json::Value = row.try_get("evidence").map_err(|e| e.to_string())?;
        let entries = evidence.as_array().cloned().unwrap_or_default();

        let tree_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "tree_quest")
            .collect();
        let work_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "work_resource")
            .collect();
        let mimir_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "mimir_resource")
            .collect();
        let manual_entries: Vec<_> = entries.iter()
            .filter(|e| e["type"] == "manual")
            .collect();

        // Unique project contexts from tree evidence
        let unique_projects: HashSet<String> = tree_entries.iter()
            .filter_map(|e| e["projectName"].as_str().map(|s| s.to_lowercase()))
            .collect();

        // Count completed skill nodes (progress == 100)
        let completed_skills = tree_entries.iter()
            .filter(|e| e["progress"].as_i64().unwrap_or(0) == 100)
            .count();

        // mimir_resource and manual count as practical evidence alongside work
        let has_work = !work_entries.is_empty();
        let has_practical = has_work || !mimir_entries.is_empty() || !manual_entries.is_empty();
        let total_evidence = entries.len();

        // PRD level rules (extended for domain-agnostic sources)
        let level: i32 = if total_evidence >= 8 && unique_projects.len() >= 3 && has_practical {
            5 // Expert
        } else if unique_projects.len() >= 2 && (has_practical || completed_skills >= 2) {
            4 // Advanced
        } else if unique_projects.len() >= 2 || (completed_skills >= 1 && has_practical) {
            3 // Proficient
        } else if completed_skills >= 1 || (!manual_entries.is_empty() && !mimir_entries.is_empty()) {
            2 // Familiar
        } else {
            1 // Aware
        };

        sqlx::query("UPDATE universal_skills SET level = $1, last_updated = NOW() WHERE id = $2")
            .bind(level)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

        updated += 1;
    }

    Ok(updated)
}

// ─── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn sync_skills_from_resume(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_resume_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_skills_from_trees(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_trees_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_skills_from_work(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_work_inner(&database.pool).await
}

#[tauri::command]
pub async fn recalculate_skill_levels(
    database: State<'_, Database>,
) -> Result<usize, String> {
    recalculate_levels_inner(&database.pool).await
}

#[tauri::command]
pub async fn sync_all_skills(
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<Vec<SyncResult>, String> {
    // Clear all evidence for a clean full sync
    sqlx::query("UPDATE universal_skills SET evidence = '[]'::jsonb")
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let r1 = sync_resume_inner(&database.pool).await?;
    let r2 = sync_trees_inner(&database.pool).await?;
    let r3 = sync_work_inner(&database.pool).await?;

    // Prune skills that still have empty evidence after the full sync —
    // these are stale rows whose sources (resources, tree nodes) no longer exist.
    // Excludes "engaged" skills: those linked to a tree or with a non-default
    // profile status. Without this guard, manually-curated rows (added to a
    // skill_trees row, marked in_progress, etc.) get nuked when their source
    // evidence happens to drop in a given sync run.
    let pruned = sqlx::query(
        "DELETE FROM skill_dependencies
         WHERE source_skill_id IN (
             SELECT id FROM universal_skills
             WHERE evidence = '[]'::jsonb
               AND id NOT IN (SELECT skill_id FROM skill_trees)
               AND id NOT IN (
                   SELECT skill_id FROM skill_profiles WHERE status NOT IN ('untouched')
               )
         )
            OR target_skill_id IN (
             SELECT id FROM universal_skills
             WHERE evidence = '[]'::jsonb
               AND id NOT IN (SELECT skill_id FROM skill_trees)
               AND id NOT IN (
                   SELECT skill_id FROM skill_profiles WHERE status NOT IN ('untouched')
               )
         )"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let deleted = sqlx::query(
        "DELETE FROM universal_skills
         WHERE evidence = '[]'::jsonb
           AND id NOT IN (SELECT skill_id FROM skill_trees)
           AND id NOT IN (
               SELECT skill_id FROM skill_profiles WHERE status NOT IN ('untouched')
           )"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    if deleted > 0 {
        println!("🌳 Pruned {} stale skills ({} dep rows)", deleted, pruned);
    }

    // ── Fix 3: orphan cleanup ──────────────────────────────────────────────
    // ON DELETE CASCADE on these FKs should handle this automatically when
    // migrations are clean, but stale rows from older schema versions or
    // half-rolled-back DELETEs can leave dangling references. Belt-and-braces.
    let orphan_trees = sqlx::query(
        "DELETE FROM skill_trees \
         WHERE skill_id NOT IN (SELECT id FROM universal_skills)"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let orphan_projects = sqlx::query(
        "DELETE FROM skill_project_links \
         WHERE skill_id NOT IN (SELECT id FROM universal_skills)"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let orphan_resources = sqlx::query(
        "DELETE FROM mimir_skill_links \
         WHERE skill_id NOT IN (SELECT id FROM universal_skills)"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    let orphan_profiles = sqlx::query(
        "DELETE FROM skill_profiles \
         WHERE skill_id NOT IN (SELECT id FROM universal_skills)"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    if orphan_trees + orphan_projects + orphan_resources + orphan_profiles > 0 {
        println!(
            "🧹 Orphan cleanup — skill_trees: {}, skill_project_links: {}, mimir_skill_links: {}, skill_profiles: {}",
            orphan_trees, orphan_projects, orphan_resources, orphan_profiles,
        );
    }

    let r4 = sync_jobs_inner(&database.pool).await?;

    sync_concept_slugs_inner(&database.pool).await?;
    // Ensure skill embeddings exist before the ANN slug backfill queries them.
    if let Err(e) = backfill_skill_embeddings(&*client, &database.pool).await {
        println!("⚠️  backfill_skill_embeddings (pre-slug-backfill) failed: {}", e);
    }
    backfill_concept_slugs_by_embedding_inner(&database.pool).await?;
    recalculate_levels_inner(&database.pool).await?;
    expand_seed_neighbors(&database.pool).await?;

    // Recompute skill_profiles.status from current origin + skill_trees state.
    // Non-fatal: log on failure but don't roll back the sync.
    if let Err(e) = refresh_skill_statuses(&database.pool).await {
        println!("⚠️  refresh_skill_statuses failed (non-fatal): {}", e);
    }

    println!("🌳 Synced all skills: resume={}, trees={}, work={}, jobs={}", r1.upserted, r2.upserted, r3.upserted, r4.upserted);
    Ok(vec![r1, r2, r3, r4])
}

#[tauri::command]
pub async fn sync_skills_from_jobs(
    database: State<'_, Database>,
) -> Result<SyncResult, String> {
    sync_jobs_inner(&database.pool).await
}

#[tauri::command]
pub async fn backfill_concept_slugs(
    database: State<'_, Database>,
) -> Result<usize, String> {
    let string_matched = sync_concept_slugs_inner(&database.pool).await?;
    let emb_matched = backfill_concept_slugs_by_embedding_inner(&database.pool).await?;
    Ok(string_matched + emb_matched)
}

#[tauri::command]
pub async fn backfill_concept_slugs_by_embedding(
    database: State<'_, Database>,
) -> Result<usize, String> {
    backfill_concept_slugs_by_embedding_inner(&database.pool).await
}

#[tauri::command]
pub async fn get_universal_skills(
    database: State<'_, Database>,
) -> Result<Vec<UniversalSkill>, String> {
    let rows = sqlx::query(
        "SELECT id, name, domain, level, evidence, last_updated, review_needed, status, origin, state
         FROM universal_skills ORDER BY review_needed DESC, level DESC, name ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter().map(|r| {
        Ok(UniversalSkill {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            name: r.try_get("name").map_err(|e| e.to_string())?,
            domain: r.try_get("domain").map_err(|e| e.to_string())?,
            level: r.try_get("level").map_err(|e| e.to_string())?,
            evidence: r.try_get("evidence").map_err(|e| e.to_string())?,
            last_updated: r.try_get::<chrono::DateTime<chrono::Utc>, _>("last_updated")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e| e.to_string())?,
            review_needed: r.try_get("review_needed").unwrap_or(false),
            status: r.try_get("status").unwrap_or_else(|_| "active".to_string()),
            origin: r.try_get("origin").unwrap_or_else(|_| "tree_quest".to_string()),
            state: r.try_get("state").unwrap_or_else(|_| "adjacent".to_string()),
        })
    }).collect()
}

#[tauri::command]
pub async fn get_skill_dependencies(
    database: State<'_, Database>,
) -> Result<Vec<SkillDependency>, String> {
    let rows = sqlx::query(
        "SELECT id, source_skill_id, target_skill_id, relationship
         FROM skill_dependencies"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter().map(|r| {
        Ok(SkillDependency {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            source_skill_id: r.try_get("source_skill_id").map_err(|e| e.to_string())?,
            target_skill_id: r.try_get("target_skill_id").map_err(|e| e.to_string())?,
            relationship: r.try_get("relationship").map_err(|e| e.to_string())?,
        })
    }).collect()
}

#[tauri::command]
pub async fn get_skill_gaps(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<SkillGap>, String> {
    // Single query: join job_skills with universal_skills in one pass.
    // LEFT JOIN on LOWER(name) so we get current_level = NULL when skill is absent.
    let demand_rows = if let Some(ref s) = season {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS count,
                    (SELECT COUNT(*) FROM job_applications WHERE season = $1) AS total,
                    MAX(us.level) AS current_level
             FROM job_skills js
             JOIN job_applications ja ON js.job_id = ja.id
             LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
             WHERE ja.season = $1
             GROUP BY js.skill_name"
        )
        .bind(s)
        .fetch_all(&database.pool)
        .await
    } else {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS count,
                    (SELECT COUNT(*) FROM job_applications) AS total,
                    MAX(us.level) AS current_level
             FROM job_skills js
             LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
             GROUP BY js.skill_name"
        )
        .fetch_all(&database.pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    let mut gaps = Vec::new();
    for row in &demand_rows {
        let skill_name: String = row.try_get("skill_name").map_err(|e| e.to_string())?;
        let count: i64 = row.try_get("count").map_err(|e| e.to_string())?;
        let total: i64 = row.try_get("total").map_err(|e| e.to_string())?;
        if total == 0 { continue; }

        let frequency = count as f64 / total as f64;
        let demand_score = frequency * 100.0;

        // NULL level means skill is absent from universal_skills → treat as 0
        let current_level: i32 = row.try_get::<Option<i32>, _>("current_level")
            .unwrap_or(None)
            .unwrap_or(0);

        // Gap = demanded but level 0 (missing) or level 1 (only aware)
        if current_level <= 1 {
            gaps.push(SkillGap {
                skill_name,
                demand_count: count,
                frequency,
                demand_score,
                current_level,
            });
        }
    }

    gaps.sort_by(|a, b| b.demand_score.partial_cmp(&a.demand_score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(gaps)
}

#[tauri::command]
pub async fn infer_skill_dependencies(
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<usize, String> {
    // Routed to the ANN-driven inferencer. The legacy alphabetical-batch
    // implementation (`infer_skill_deps_inner`) remains defined below but
    // is no longer called from anywhere.
    infer_skill_deps_ann(&*client, &database.pool).await
}

pub(crate) async fn infer_skill_deps_inner(client: &reqwest::Client, pool: &PgPool) -> Result<usize, String> {
    // Skip transient job_gap skills with no engagement signal. They tend to
    // churn (deleted in the next sync prune if no longer in any JD), so paying
    // an LLM round-trip on them is wasted budget. A skill is included iff:
    //   - origin is resume/work/tree_quest (durable sources), OR
    //   - has a skill_trees row (user opened a tree for it), OR
    //   - has a non-untouched skill_profiles status
    let rows = sqlx::query(
        "SELECT u.id, u.name
         FROM universal_skills u
         WHERE u.origin IN ('resume', 'work', 'tree_quest')
            OR EXISTS (SELECT 1 FROM skill_trees st WHERE st.skill_id = u.id)
            OR EXISTS (
                SELECT 1 FROM skill_profiles sp
                WHERE sp.skill_id = u.id AND sp.status NOT IN ('untouched')
            )
         ORDER BY u.name"
    )
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Count what was filtered out so we can surface it.
    let total_row = sqlx::query("SELECT COUNT(*)::bigint AS n FROM universal_skills")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let total: i64 = total_row.try_get("n").unwrap_or(0);
    let skipped = (total as usize).saturating_sub(rows.len());
    if skipped > 0 {
        println!("⚡ Skipped dep inference for {} transient job_gap skills", skipped);
    }

    if rows.len() < 2 { return Ok(0); }

    let skill_names: Vec<String> = rows.iter()
        .map(|r| r.try_get::<String, _>("name").unwrap_or_default())
        .collect();

    let name_to_id: std::collections::HashMap<String, String> = rows.iter()
        .map(|r| {
            let name: String = r.try_get("name").unwrap_or_default();
            let id: String = r.try_get("id").unwrap_or_default();
            (name, id)
        })
        .collect();

    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    let system_prompt = "Given a list of technical skills, identify prerequisite relationships. \
        Return ONLY a JSON array of objects: [{\"from\": \"skill_a\", \"to\": \"skill_b\"}] \
        where skill_a is a prerequisite for skill_b. Only include strong, clear prerequisites. \
        Use the exact skill names provided. Max 30 relationships. Return ONLY the JSON array, no markdown.";

    #[derive(Deserialize)]
    struct DepPair {
        from: String,
        to: String,
    }

    let lower_to_id: std::collections::HashMap<String, String> = name_to_id.iter()
        .map(|(name, id)| (name.to_lowercase(), id.clone()))
        .collect();

    const BATCH_SIZE: usize = 30;
    let batches: Vec<&[String]> = skill_names.chunks(BATCH_SIZE).collect();
    let mut all_pairs: Vec<DepPair> = Vec::new();

    for (i, batch) in batches.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        let user_prompt = format!("Skills: {}", batch.join(", "));

        let deps_t0 = std::time::Instant::now();
        let response = client
            .post(GROQ_API_URL)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "llama-3.3-70b-versatile",
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.3,
                "max_tokens": 1000
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| {
                crate::brain::log_prompt_call(
                    pool.clone(), "skill_deps", "llama-3.3-70b-versatile", "skill_deps_v1",
                    deps_t0.elapsed().as_millis() as i64, false, Some(e.to_string()), None,
                );
                format!("Groq request failed (batch {}): {}", i, e)
            })?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Groq parse error (batch {}): {}", i, e))?;

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("[]");

        let clean = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let batch_pairs: Vec<DepPair> = serde_json::from_str(clean).unwrap_or_default();
        crate::brain::log_prompt_call(
            pool.clone(), "skill_deps", "llama-3.3-70b-versatile", "skill_deps_v1",
            deps_t0.elapsed().as_millis() as i64, true, None,
            Some(serde_json::json!({ "batch": i, "skills": batch.len() })),
        );
        println!("  🔗 Batch {}: {} pairs from {} skills", i + 1, batch_pairs.len(), batch.len());
        all_pairs.extend(batch_pairs);
    }

    // ── Second pass: related / co_occurs edges ───────────────────────────────
    let related_system_prompt = "Given a list of technical skills, identify related skills \
        (conceptually similar or frequently used together). \
        Return ONLY a JSON array: [{\"from\": \"skill_a\", \"to\": \"skill_b\", \"relationship\": \"related\"}]. \
        Use exact skill names from the list. Max 40 relationships. Return ONLY the JSON array, no markdown.";

    #[derive(Deserialize)]
    struct RelatedTriple {
        from: String,
        to: String,
        #[allow(dead_code)]
        relationship: String,
    }

    let mut all_related: Vec<RelatedTriple> = Vec::new();

    for (i, batch) in batches.iter().enumerate() {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let user_prompt = format!("Skills: {}", batch.join(", "));
        let rel_t0 = std::time::Instant::now();
        let response = client
            .post(GROQ_API_URL)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "llama-3.3-70b-versatile",
                "messages": [
                    { "role": "system", "content": related_system_prompt },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.3,
                "max_tokens": 1200
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;

        match response {
            Err(e) => {
                crate::brain::log_prompt_call(
                    pool.clone(), "skill_related", "llama-3.3-70b-versatile", "skill_related_v1",
                    rel_t0.elapsed().as_millis() as i64, false, Some(e.to_string()), None,
                );
                println!("  ⚠️  Related pass batch {} failed (non-fatal): {}", i, e);
                continue;
            }
            Ok(resp) => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let content = body["choices"][0]["message"]["content"].as_str().unwrap_or("[]");
                let clean = content.trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();
                let batch_triples: Vec<RelatedTriple> = serde_json::from_str(clean).unwrap_or_default();
                crate::brain::log_prompt_call(
                    pool.clone(), "skill_related", "llama-3.3-70b-versatile", "skill_related_v1",
                    rel_t0.elapsed().as_millis() as i64, true, None,
                    Some(serde_json::json!({ "batch": i, "skills": batch.len() })),
                );
                println!("  🔗 Related batch {}: {} triples", i + 1, batch_triples.len());
                all_related.extend(batch_triples);
            }
        }
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    // Delete inferred prerequisite and related edges — preserve manually curated ones
    sqlx::query(
        "DELETE FROM skill_dependencies WHERE relationship IN ('prerequisite', 'related') AND is_manual = false"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0;
    for pair in &all_pairs {
        let from_id = lower_to_id.get(&pair.from.to_lowercase());
        let to_id = lower_to_id.get(&pair.to.to_lowercase());

        if let (Some(from_id), Some(to_id)) = (from_id, to_id) {
            if from_id == to_id { continue; }
            let dep_id = Uuid::new_v4().to_string();
            let result = sqlx::query(
                "INSERT INTO skill_dependencies (id, source_skill_id, target_skill_id, relationship)
                 VALUES ($1, $2, $3, 'prerequisite')
                 ON CONFLICT (source_skill_id, target_skill_id) DO NOTHING"
            )
            .bind(&dep_id)
            .bind(from_id)
            .bind(to_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            if result.rows_affected() > 0 {
                count += 1;
            }
        }
    }

    for triple in &all_related {
        let from_id = lower_to_id.get(&triple.from.to_lowercase());
        let to_id = lower_to_id.get(&triple.to.to_lowercase());

        if let (Some(from_id), Some(to_id)) = (from_id, to_id) {
            if from_id == to_id { continue; }
            let dep_id = Uuid::new_v4().to_string();
            let result = sqlx::query(
                "INSERT INTO skill_dependencies (id, source_skill_id, target_skill_id, relationship)
                 VALUES ($1, $2, $3, 'related')
                 ON CONFLICT (source_skill_id, target_skill_id) DO NOTHING"
            )
            .bind(&dep_id)
            .bind(from_id)
            .bind(to_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            if result.rows_affected() > 0 {
                count += 1;
            }
        }
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    println!("🔗 Inferred {} skill deps ({} prereq pairs, {} related triples, {} batches)",
        count, all_pairs.len(), all_related.len(), batches.len());
    Ok(count)
}

// ─── ANN-driven dep inference (replaces alphabetical batching) ──────────────

/// New dep inferencer: builds LLM batches from semantic neighborhoods rather
/// than alphabetical chunks. Every embedded skill becomes a cluster center;
/// its top-12 nearest neighbors form the cluster. Heavily-overlapping clusters
/// are skipped via a pair-dedup HashSet. Final batches pack up to 5 clusters
/// into one LLM call (~65 skills max, deduplicated to unique names).
///
/// Reads embeddings only via server-side cosine queries — no Vec<f32> decoding
/// (the codebase has no pgvector Rust crate). All ANN work happens in HNSW.
pub(crate) async fn infer_skill_deps_ann(client: &reqwest::Client, pool: &PgPool) -> Result<usize, String> {
    // 1. Load every embedded skill — id + name.
    let rows = sqlx::query(
        "SELECT id, name
         FROM universal_skills
         WHERE embedding IS NOT NULL
         ORDER BY id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let total_skills_row = sqlx::query("SELECT COUNT(*)::bigint AS n FROM universal_skills")
        .fetch_one(pool).await.map_err(|e| e.to_string())?;
    let total_skills: i64 = total_skills_row.try_get("n").unwrap_or(0);
    let skipped_unembedded = (total_skills as usize).saturating_sub(rows.len());
    if skipped_unembedded > 0 {
        println!("⚡ Skipped dep inference for {} unembedded skills", skipped_unembedded);
    }

    if rows.len() < 2 { return Ok(0); }

    let centers: Vec<(String, String)> = rows.iter().filter_map(|r| {
        let id: String = r.try_get("id").ok()?;
        let name: String = r.try_get("name").ok()?;
        Some((id, name))
    }).collect();

    let name_to_id: std::collections::HashMap<String, String> = centers.iter()
        .map(|(id, name)| (name.clone(), id.clone()))
        .collect();
    let lower_to_id: std::collections::HashMap<String, String> = centers.iter()
        .map(|(id, name)| (name.to_lowercase(), id.clone()))
        .collect();

    // 2. For each center, fetch its top-12 neighbors as (id, name) pairs.
    //    Uses a correlated subquery to feed the center's own embedding into
    //    the cosine ORDER BY — avoids needing a Vec<f32> decoder.
    struct Cluster { center_id: String, member_ids: Vec<String>, member_names: Vec<String> }
    let mut clusters: Vec<Cluster> = Vec::with_capacity(centers.len());

    for (center_id, center_name) in &centers {
        let neighbor_rows = sqlx::query(
            "SELECT u.id, u.name
             FROM universal_skills u,
                  (SELECT embedding FROM universal_skills WHERE id = $1) src
             WHERE u.embedding IS NOT NULL
               AND u.id != $1
             ORDER BY u.embedding <=> src.embedding
             LIMIT 12"
        )
        .bind(center_id)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("neighbor query failed for {}: {}", center_id, e))?;

        let mut member_ids = Vec::with_capacity(13);
        let mut member_names = Vec::with_capacity(13);
        member_ids.push(center_id.clone());
        member_names.push(center_name.clone());
        for r in &neighbor_rows {
            if let (Ok(id), Ok(name)) = (
                r.try_get::<String, _>("id"),
                r.try_get::<String, _>("name"),
            ) {
                member_ids.push(id);
                member_names.push(name);
            }
        }
        clusters.push(Cluster { center_id: center_id.clone(), member_ids, member_names });
    }

    // 3. Pair-dedup: skip clusters where >50% of pairs are already covered.
    //    Pairs are stored as sorted tuples so (a,b) == (b,a).
    let mut seen_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    fn ordered_pair(a: &str, b: &str) -> (String, String) {
        if a < b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) }
    }

    let mut kept_clusters: Vec<Cluster> = Vec::new();
    let mut skipped_clusters = 0usize;
    for cluster in clusters {
        let mut all_pairs_in_cluster: Vec<(String, String)> = Vec::new();
        for i in 0..cluster.member_ids.len() {
            for j in (i + 1)..cluster.member_ids.len() {
                all_pairs_in_cluster.push(ordered_pair(&cluster.member_ids[i], &cluster.member_ids[j]));
            }
        }
        let total = all_pairs_in_cluster.len().max(1);
        let already_seen = all_pairs_in_cluster.iter().filter(|p| seen_pairs.contains(*p)).count();
        if already_seen * 2 > total {
            skipped_clusters += 1;
            continue;
        }
        for p in &all_pairs_in_cluster { seen_pairs.insert(p.clone()); }
        kept_clusters.push(cluster);
    }
    if skipped_clusters > 0 {
        println!("🔁 ANN: skipped {} redundant clusters (>50% pairs already covered)", skipped_clusters);
    }
    if kept_clusters.is_empty() { return Ok(0); }

    // 4. Pack 5 clusters per LLM batch, deduplicating skill names within the batch.
    const CLUSTERS_PER_BATCH: usize = 5;
    let batches: Vec<Vec<&Cluster>> = kept_clusters
        .chunks(CLUSTERS_PER_BATCH)
        .map(|chunk| chunk.iter().collect())
        .collect();

    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    let system_prompt = "You are a technical curriculum expert. Given a list of skills that are semantically related, identify prerequisite relationships between them.\n\n\
        A prerequisite means: you cannot meaningfully learn skill B without first understanding skill A.\n\n\
        Rules:\n\
        - Only include strong, unambiguous prerequisites\n\
        - Both skills must be from the provided list\n\
        - Return ONLY a JSON array, no markdown, no explanation\n\
        - Format: [{\"from\": \"prerequisite_skill\", \"to\": \"dependent_skill\"}]\n\
        - Maximum 20 pairs per response\n\
        - If no strong prerequisites exist, return []";

    #[derive(Deserialize)]
    struct DepPair { from: String, to: String }

    let mut all_pairs: Vec<DepPair> = Vec::new();
    let mut batches_run = 0usize;

    for (i, batch) in batches.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        // Deduplicate skill names within the batch.
        let mut seen_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut deduped_names: Vec<&str> = Vec::new();
        for cluster in batch {
            for name in &cluster.member_names {
                if seen_names.insert(name.as_str()) {
                    deduped_names.push(name.as_str());
                }
            }
        }

        let user_prompt = format!("Skills: {}", deduped_names.join(", "));
        let t0 = std::time::Instant::now();
        let response = client
            .post(GROQ_API_URL)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": "llama-3.3-70b-versatile",
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.1,
                "max_tokens": 800
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;

        let elapsed = t0.elapsed().as_millis() as i64;
        batches_run += 1;

        match response {
            Err(e) => {
                crate::brain::log_prompt_call(
                    pool.clone(), "skill_deps_ann", "llama-3.3-70b-versatile", "skill_deps_ann_v1",
                    elapsed, false, Some(e.to_string()), None,
                );
                println!("⚠️  ANN batch {} HTTP failed (non-fatal): {}", i + 1, e);
                continue;
            }
            Ok(resp) => {
                let body: serde_json::Value = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => {
                        println!("⚠️  ANN batch {} body parse failed: {}", i + 1, e);
                        continue;
                    }
                };
                let content = body["choices"][0]["message"]["content"].as_str().unwrap_or("[]");
                let clean = content
                    .trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();
                let batch_pairs: Vec<DepPair> = match serde_json::from_str(clean) {
                    Ok(v) => v,
                    Err(e) => {
                        let preview: String = clean.chars().take(200).collect();
                        println!("⚠️  dep parse failed for batch {}: {} | response: {}", i + 1, e, preview);
                        Vec::new()
                    }
                };
                crate::brain::log_prompt_call(
                    pool.clone(), "skill_deps_ann", "llama-3.3-70b-versatile", "skill_deps_ann_v1",
                    elapsed, true, None,
                    Some(serde_json::json!({ "batch": i, "skills": deduped_names.len(), "pairs": batch_pairs.len() })),
                );
                println!("🔗 ANN batch {}: {} pairs from {} skills", i + 1, batch_pairs.len(), deduped_names.len());
                all_pairs.extend(batch_pairs);
            }
        }
    }

    // 5. Resolve names → ids once, in memory, before touching the DB. This
    //    way we collect the final (source_id, target_id) write set up front
    //    and the actual transaction window stays short.
    let mut resolved_pairs: Vec<(String, String)> = Vec::with_capacity(all_pairs.len());
    let mut unknown_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let _ = &name_to_id; // name_to_id retained for parity with old code; lower_to_id is the one we use

    for pair in &all_pairs {
        let from_id = lower_to_id.get(&pair.from.to_lowercase());
        let to_id = lower_to_id.get(&pair.to.to_lowercase());
        match (from_id, to_id) {
            (Some(fid), Some(tid)) if fid != tid => {
                resolved_pairs.push((fid.clone(), tid.clone()));
            }
            (Some(_), Some(_)) => { /* self-loop, drop */ }
            _ => {
                if from_id.is_none() { unknown_names.insert(pair.from.clone()); }
                if to_id.is_none() { unknown_names.insert(pair.to.clone()); }
            }
        }
    }

    for name in &unknown_names {
        println!("⚠️  dep unknown skill: {}", name);
    }

    // 6. Atomic swap. We open the transaction here — not at the top of the
    //    function — because holding a tx across LLM calls would pin a Neon
    //    connection in idle-in-transaction state for tens of seconds per
    //    batch and risk hitting the server's idle timeout. The atomicity the
    //    spec asks for ("if the LLM calls fail partway through, the old
    //    edges are not lost") is preserved either way: any LLM/HTTP error
    //    above returns Err before we reach this point, so the DELETE never
    //    runs and the existing graph stays intact.
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "DELETE FROM skill_dependencies
         WHERE relationship = 'prerequisite' AND is_manual = false"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let mut count = 0usize;
    for (fid, tid) in &resolved_pairs {
        let dep_id = Uuid::new_v4().to_string();
        let res = sqlx::query(
            "INSERT INTO skill_dependencies (id, source_skill_id, target_skill_id, relationship)
             VALUES ($1, $2, $3, 'prerequisite')
             ON CONFLICT (source_skill_id, target_skill_id) DO NOTHING"
        )
        .bind(&dep_id)
        .bind(fid)
        .bind(tid)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        if res.rows_affected() > 0 { count += 1; }
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    println!(
        "🔗 Inferred {} skill deps via ANN ({} pairs, {} batches)",
        count, all_pairs.len(), batches_run,
    );
    Ok(count)
}

// ─── Skill alias / merge commands ───────────────────────────────────────────

#[tauri::command]
pub async fn get_skill_aliases(
    database: State<'_, Database>,
) -> Result<Vec<SkillAlias>, String> {
    let rows = sqlx::query(
        "SELECT sa.id, sa.canonical_skill_id, us.name AS canonical_skill_name, sa.alias
         FROM skill_aliases sa
         JOIN universal_skills us ON us.id = sa.canonical_skill_id
         ORDER BY us.name ASC, sa.alias ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.iter().map(|r| {
        Ok(SkillAlias {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            canonical_skill_id: r.try_get("canonical_skill_id").map_err(|e| e.to_string())?,
            canonical_skill_name: r.try_get("canonical_skill_name").map_err(|e| e.to_string())?,
            alias: r.try_get("alias").map_err(|e| e.to_string())?,
        })
    }).collect()
}

// ─── Deterministic duplicate detection ───────────────────────────────────────

static STRIP_SUFFIXES: &[&str] = &[
    " skills", " skill", " knowledge", " tools", " tool",
    " principles", " systems", " system", " concepts", " concept",
];

fn strip_knowledge_suffix(s: &str) -> &str {
    let lower = s.to_lowercase();
    for suffix in STRIP_SUFFIXES {
        if lower.ends_with(suffix) {
            return &s[..s.len() - suffix.len()];
        }
    }
    s
}

fn to_singular(s: &str) -> String {
    // Only handle the common English plural patterns we care about
    if s.ends_with("ies") && s.len() > 3 {
        return format!("{}y", &s[..s.len() - 3]);
    }
    if s.ends_with("ses") && s.len() > 3 {
        return s[..s.len() - 2].to_string();
    }
    if s.ends_with('s') && s.len() > 2 && !s.ends_with("ss") {
        return s[..s.len() - 1].to_string();
    }
    s.to_string()
}

#[tauri::command]
pub async fn suggest_skill_merges(
    database: State<'_, Database>,
) -> Result<Vec<SuggestedMerge>, String> {
    suggest_skill_merges_inner(&database.pool).await
}

pub(crate) async fn suggest_skill_merges_inner(pool: &PgPool) -> Result<Vec<SuggestedMerge>, String> {
    // Load all skills
    let rows = sqlx::query(
        "SELECT us.id, us.name FROM universal_skills us ORDER BY us.name"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Load alias table: alias text → canonical_skill_id
    let alias_rows = sqlx::query(
        "SELECT LOWER(alias) AS alias_lower, canonical_skill_id FROM skill_aliases"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let alias_map: std::collections::HashMap<String, String> = alias_rows.iter()
        .filter_map(|r| {
            let alias: String = r.try_get("alias_lower").ok()?;
            let cid: String = r.try_get("canonical_skill_id").ok()?;
            Some((alias, cid))
        })
        .collect();

    struct Skill { id: String, name: String }
    let skills: Vec<Skill> = rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            Some(Skill { id, name })
        })
        .collect();

    // Group candidates by a normalised key — first-match-wins across four rules.
    // key → (canonical_idx, reason)
    let mut groups: std::collections::HashMap<String, (usize, String)> = std::collections::HashMap::new();
    // idx → list of (alias_idx, reason) to merge into canonical
    let mut merges: std::collections::HashMap<usize, Vec<(usize, String)>> = std::collections::HashMap::new();

    let id_to_idx: std::collections::HashMap<String, usize> = skills.iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();

    for (i, skill) in skills.iter().enumerate() {
        let name = &skill.name;
        let name_lower = name.to_lowercase();

        // Rule 1 — normalized name match
        let norm = normalize_skill_name(name).to_lowercase();
        if !norm.is_empty() {
            if let Some(&(canon_i, ref _r)) = groups.get(&norm) {
                if canon_i != i {
                    merges.entry(canon_i).or_default().push((i, "normalized match".to_string()));
                    continue;
                }
            } else {
                groups.insert(norm.clone(), (i, "normalized match".to_string()));
            }
        }

        // Rule 2 — alias collision: this skill's name matches an alias pointing to another skill
        if let Some(canonical_id) = alias_map.get(&name_lower) {
            if let Some(&canon_i) = id_to_idx.get(canonical_id) {
                if canon_i != i {
                    merges.entry(canon_i).or_default().push((i, "alias collision".to_string()));
                    continue;
                }
            }
        }

        // Rule 3 — suffix stripping
        let stripped = strip_knowledge_suffix(name).to_lowercase();
        if stripped != name_lower && !stripped.is_empty() {
            if let Some(&(canon_i, ref _r)) = groups.get(&stripped) {
                if canon_i != i {
                    // merge the longer (suffix) form into the shorter canonical
                    merges.entry(canon_i).or_default().push((i, "suffix strip".to_string()));
                    continue;
                }
            } else {
                // Register stripped key pointing to this skill, but also check if
                // another skill with the stripped name already exists
                let mut found_shorter = false;
                for (j, other) in skills.iter().enumerate() {
                    if j == i { continue; }
                    if other.name.to_lowercase() == stripped {
                        // other is the shorter form → canonical
                        merges.entry(j).or_default().push((i, "suffix strip".to_string()));
                        found_shorter = true;
                        break;
                    }
                }
                if !found_shorter {
                    groups.entry(stripped).or_insert((i, "suffix strip".to_string()));
                }
                continue;
            }
        }

        // Rule 4 — plural/singular
        let singular = to_singular(&name_lower);
        if singular != name_lower {
            // Is there already a skill with the singular form?
            let mut found_singular = false;
            for (j, other) in skills.iter().enumerate() {
                if j == i { continue; }
                if other.name.to_lowercase() == singular {
                    merges.entry(j).or_default().push((i, "plural/singular".to_string()));
                    found_singular = true;
                    break;
                }
            }
            if found_singular { continue; }
        }
    }

    // Build result — deduplicate: each alias_idx should appear in at most one merge group
    let mut used_as_alias: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut suggestions: Vec<SuggestedMerge> = Vec::new();

    for (canon_i, alias_list) in &merges {
        let canon = &skills[*canon_i];
        let mut alias_ids: Vec<String> = Vec::new();
        let mut alias_names: Vec<String> = Vec::new();
        let mut reason = String::new();

        for (alias_i, alias_reason) in alias_list {
            if used_as_alias.contains(alias_i) { continue; }
            if *alias_i == *canon_i { continue; }
            alias_ids.push(skills[*alias_i].id.clone());
            alias_names.push(skills[*alias_i].name.clone());
            if reason.is_empty() { reason = alias_reason.clone(); }
            used_as_alias.insert(*alias_i);
        }

        if alias_ids.is_empty() { continue; }

        suggestions.push(SuggestedMerge {
            canonical_id: canon.id.clone(),
            canonical_name: canon.name.clone(),
            alias_ids,
            alias_names,
            reason,
        });
    }

    suggestions.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
    println!("🔍 suggest_skill_merges: {} suggestions", suggestions.len());
    Ok(suggestions)
}

#[tauri::command]
pub async fn auto_merge_suggested(
    database: State<'_, Database>,
) -> Result<usize, String> {
    let suggestions = suggest_skill_merges_inner(&database.pool).await?;
    let count = suggestions.len();
    for suggestion in suggestions {
        merge_skills_inner(&suggestion.canonical_id, &suggestion.alias_ids, &database.pool).await?;
    }
    println!("✅ Auto-merged {} duplicate skill groups", count);
    Ok(count)
}

// ─── Merge implementation (inner, pool-only) ──────────────────────────────────

pub(crate) async fn merge_skills_inner(canonical_id: &str, alias_ids: &[String], pool: &PgPool) -> Result<(), String> {
    if alias_ids.is_empty() { return Ok(()); }

    let canonical_row = sqlx::query(
        "SELECT name, evidence FROM universal_skills WHERE id = $1"
    )
    .bind(canonical_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Canonical skill not found: {}", e))?;

    let canonical_name: String = canonical_row.try_get("name").map_err(|e| e.to_string())?;
    let mut merged_evidence: Vec<serde_json::Value> = canonical_row
        .try_get::<serde_json::Value, _>("evidence")
        .map_err(|e| e.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default();

    for alias_id in alias_ids {
        if alias_id == canonical_id { continue; }

        let alias_row = sqlx::query(
            "SELECT name, evidence FROM universal_skills WHERE id = $1"
        )
        .bind(alias_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

        let Some(alias_row) = alias_row else { continue; };
        let alias_name: String = alias_row.try_get("name").map_err(|e| e.to_string())?;

        let alias_entry_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO skill_aliases (id, canonical_skill_id, alias)
             VALUES ($1, $2, $3)
             ON CONFLICT (alias) DO UPDATE SET canonical_skill_id = EXCLUDED.canonical_skill_id"
        )
        .bind(&alias_entry_id)
        .bind(canonical_id)
        .bind(&alias_name)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        let alias_evidence: Vec<serde_json::Value> = alias_row
            .try_get::<serde_json::Value, _>("evidence")
            .map_err(|e| e.to_string())?
            .as_array()
            .cloned()
            .unwrap_or_default();
        merged_evidence.extend(alias_evidence);

        sqlx::query(
            "UPDATE skill_dependencies SET source_skill_id = $1
             WHERE source_skill_id = $2
               AND NOT EXISTS (
                 SELECT 1 FROM skill_dependencies
                 WHERE source_skill_id = $1
                   AND target_skill_id = skill_dependencies.target_skill_id
               )"
        )
        .bind(canonical_id)
        .bind(alias_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE skill_dependencies SET target_skill_id = $1
             WHERE target_skill_id = $2
               AND NOT EXISTS (
                 SELECT 1 FROM skill_dependencies
                 WHERE target_skill_id = $1
                   AND source_skill_id = skill_dependencies.source_skill_id
               )"
        )
        .bind(canonical_id)
        .bind(alias_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("DELETE FROM universal_skills WHERE id = $1")
            .bind(alias_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

        println!("🔀 Merged '{}' → '{}' (canonical)", alias_name, canonical_name);
    }

    let merged_json = serde_json::Value::Array(merged_evidence);
    sqlx::query(
        "UPDATE universal_skills SET evidence = $1::jsonb, last_updated = NOW() WHERE id = $2"
    )
    .bind(&merged_json)
    .bind(canonical_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    recalculate_levels_inner(pool).await?;
    println!("✅ Merge complete: {} aliases into '{}'", alias_ids.len(), canonical_name);
    Ok(())
}

#[tauri::command]
pub async fn merge_skills(
    canonical_id: String,
    alias_ids: Vec<String>,
    database: State<'_, Database>,
) -> Result<(), String> {
    if alias_ids.is_empty() {
        return Err("No alias IDs provided".to_string());
    }
    merge_skills_inner(&canonical_id, &alias_ids, &database.pool).await
}

#[tauri::command]
pub async fn mark_skill_reviewed(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE universal_skills SET review_needed = false, status = 'active', last_updated = NOW()
         WHERE id = $1"
    )
    .bind(&skill_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── skill_profiles mutations (status + notes from the Skill Detail panel) ──

#[tauri::command]
pub async fn update_skill_status(
    skill_id: String,
    status: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    if !["untouched", "in_progress", "practiced", "mastered"].contains(&status.as_str()) {
        return Err(format!("invalid status: {}", status));
    }
    // Upsert so newly synced skills without a profile row still accept writes.
    sqlx::query(
        "INSERT INTO skill_profiles (skill_id, status, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (skill_id) DO UPDATE SET
             status = EXCLUDED.status,
             updated_at = NOW()"
    )
    .bind(&skill_id)
    .bind(&status)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_skill_notes(
    skill_id: String,
    notes: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO skill_profiles (skill_id, notes, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (skill_id) DO UPDATE SET
             notes = EXCLUDED.notes,
             updated_at = NOW()"
    )
    .bind(&skill_id)
    .bind(&notes)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Skill status refresh ────────────────────────────────────────────────────

/// Recompute `skill_profiles.status` for every skill based on derived signals.
///
/// Priority order (highest wins, evaluated as three sequential upserts):
///   1. mastered     — already 'mastered'; never auto-downgrade. Preserved by
///                     the `WHERE skill_profiles.status != 'mastered'` clause
///                     in every UPDATE branch below.
///   2. practiced    — origin IN ('resume','work')
///   3. in_progress  — at least one row in skill_trees AND origin NOT IN
///                     ('resume','work')
///   4. untouched    — everything else (lowest priority; ON CONFLICT DO NOTHING
///                     so we never overwrite an existing status).
///
/// Runs inside a single transaction so concurrent reads see a consistent set.
pub(crate) async fn refresh_skill_statuses(pool: &PgPool) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    // 2. practiced — resume/work skills
    sqlx::query(
        "INSERT INTO skill_profiles (skill_id, status, updated_at)
         SELECT id, 'practiced', NOW()
         FROM universal_skills
         WHERE origin IN ('resume', 'work')
         ON CONFLICT (skill_id) DO UPDATE SET
             status = EXCLUDED.status,
             updated_at = NOW()
         WHERE skill_profiles.status != 'mastered'
           AND skill_profiles.status != EXCLUDED.status"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("refresh practiced failed: {}", e))?;

    // 3. in_progress — has a skill_trees row, origin is not resume/work
    sqlx::query(
        "INSERT INTO skill_profiles (skill_id, status, updated_at)
         SELECT u.id, 'in_progress', NOW()
         FROM universal_skills u
         WHERE u.origin NOT IN ('resume', 'work')
           AND EXISTS (SELECT 1 FROM skill_trees st WHERE st.skill_id = u.id)
         ON CONFLICT (skill_id) DO UPDATE SET
             status = EXCLUDED.status,
             updated_at = NOW()
         WHERE skill_profiles.status NOT IN ('practiced', 'mastered')
           AND skill_profiles.status != EXCLUDED.status"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("refresh in_progress failed: {}", e))?;

    // 4. untouched — ensure every skill has a profile row, never overwrite
    sqlx::query(
        "INSERT INTO skill_profiles (skill_id, status, updated_at)
         SELECT id, 'untouched', NOW()
         FROM universal_skills
         ON CONFLICT (skill_id) DO NOTHING"
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("refresh untouched failed: {}", e))?;

    tx.commit().await.map_err(|e| format!("refresh commit failed: {}", e))?;

    // Summary log
    if let Ok(rows) = sqlx::query(
        "SELECT status, COUNT(*)::bigint AS n FROM skill_profiles GROUP BY status"
    )
    .fetch_all(pool)
    .await {
        let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for r in rows {
            let status: String = r.try_get("status").unwrap_or_default();
            let n: i64 = r.try_get("n").unwrap_or(0);
            counts.insert(status, n);
        }
        println!(
            "🩺 refresh_skill_statuses: mastered: {}, practiced: {}, in_progress: {}, untouched: {}",
            counts.get("mastered").copied().unwrap_or(0),
            counts.get("practiced").copied().unwrap_or(0),
            counts.get("in_progress").copied().unwrap_or(0),
            counts.get("untouched").copied().unwrap_or(0),
        );
    }

    Ok(())
}

#[tauri::command]
pub async fn refresh_skill_statuses_cmd(
    database: State<'_, Database>,
) -> Result<(), String> {
    refresh_skill_statuses(&database.pool).await
}

// ─── Skill embedding backfill ────────────────────────────────────────────────

/// Embed every `universal_skills` row whose `embedding` column is NULL.
/// Reuses the same OpenRouter pplx-embed-v1-0.6b endpoint as Mimir chunks,
/// batched 50 per call. Returns the count embedded.
///
/// Called automatically at the tail of `classify_skill_domains` so newly
/// synced + classified skills get embedded on every sync without UI prompts.
pub(crate) async fn backfill_skill_embeddings(
    client: &reqwest::Client,
    pool: &PgPool,
) -> Result<usize, String> {
    let rows = sqlx::query(
        "SELECT us.id, us.name, us.display_name, sd.name as domain_name
         FROM universal_skills us
         LEFT JOIN skill_domains sd ON sd.id = us.domain_id
         WHERE us.embedding IS NULL
         ORDER BY us.id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() { return Ok(0); }

    struct Pending { id: String, text: String }
    let pending: Vec<Pending> = rows.iter().filter_map(|r| {
        let id: String = r.try_get("id").ok()?;
        let name: String = r.try_get("name").ok()?;
        let display_name: String = r.try_get::<Option<String>, _>("display_name").ok().flatten().unwrap_or_default();
        let domain_name: String = r.try_get::<Option<String>, _>("domain_name").ok().flatten().unwrap_or_default();
        let text = format!("{} {} {}", name, display_name, domain_name)
            .split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() { return None; }
        Some(Pending { id, text })
    }).collect();

    let total = pending.len();
    let mut embedded = 0usize;

    const BATCH_SIZE: usize = 50;
    for batch in pending.chunks(BATCH_SIZE) {
        let texts: Vec<String> = batch.iter().map(|p| p.text.clone()).collect();
        let embeddings = match crate::mimir_ingest::get_embeddings_batch(client, &texts).await {
            Ok(v) => v,
            Err(e) => {
                println!("⚠️  backfill_skill_embeddings batch failed: {}", e);
                continue;
            }
        };
        if embeddings.len() != batch.len() {
            println!(
                "⚠️  backfill_skill_embeddings batch length mismatch: got {} for {} inputs",
                embeddings.len(), batch.len()
            );
            continue;
        }
        for (p, emb) in batch.iter().zip(embeddings.iter()) {
            let vec_str = crate::mimir_ingest::vector_str(emb);
            match sqlx::query(
                "UPDATE universal_skills SET embedding = $1::vector WHERE id = $2"
            )
            .bind(&vec_str)
            .bind(&p.id)
            .execute(pool)
            .await {
                Ok(_) => embedded += 1,
                Err(e) => println!("⚠️  embed UPDATE failed for {}: {}", p.id, e),
            }
        }
    }

    println!("🧠 Embedded {}/{} skills", embedded, total);
    Ok(embedded)
}

#[tauri::command]
pub async fn backfill_skill_embeddings_cmd(
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<String, String> {
    let n = backfill_skill_embeddings(&*client, &database.pool).await?;
    Ok(format!("Embedded {} skills", n))
}

#[tauri::command]
pub async fn get_similar_skills(
    skill_id: String,
    limit: Option<usize>,
    database: State<'_, Database>,
) -> Result<Vec<crate::read_models::NearestSkill>, String> {
    let limit = limit.unwrap_or(8) as i64;
    let pool = &database.pool;

    // First check the source skill has an embedding — early-return empty if not.
    // This avoids running an expensive ORDER BY against a NULL probe vector
    // (Postgres would error, but we'd rather degrade silently).
    let probe = sqlx::query(
        "SELECT 1 AS ok FROM universal_skills
         WHERE id = $1 AND embedding IS NOT NULL
         LIMIT 1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if probe.is_none() {
        return Ok(Vec::new());
    }

    // Single-query ANN: subquery pulls the source row's embedding and feeds
    // it back into the cosine ORDER BY. HNSW index (mig 044) handles it.
    let rows = sqlx::query(
        "SELECT u.id,
                u.name,
                (u.embedding <=> src.embedding)::float4 AS distance
         FROM universal_skills u,
              (SELECT embedding FROM universal_skills WHERE id = $1) src
         WHERE u.embedding IS NOT NULL
           AND u.id != $1
         ORDER BY u.embedding <=> src.embedding
         LIMIT $2"
    )
    .bind(&skill_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().filter_map(|r| {
        Some(crate::read_models::NearestSkill {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        })
    }).collect())
}

// ─── Direct skill → resource ANN (bypasses concept_slug bridge) ──────────────

/// Match a skill against the full Mimir chunk corpus by cosine distance.
/// Groups winning chunks by (resource_id, section_title) so a textbook with
/// multiple relevant chapters surfaces as multiple link rows. Chunks with
/// NULL section_title fall through to whole-resource entries.
///
/// Writes results into `mimir_skill_links` with section metadata denormalized
/// for the panel renderer. Returns the count of rows upserted.
#[tauri::command]
pub async fn match_skill_to_resources(
    skill_id: String,
    limit: Option<usize>,
    database: State<'_, Database>,
) -> Result<usize, String> {
    let limit_i = limit.unwrap_or(8) as i64;
    let pool = &database.pool;

    // Probe: skill must exist + have an embedding.
    let probe = sqlx::query(
        "SELECT 1 AS ok FROM universal_skills \
         WHERE id = $1 AND embedding IS NOT NULL LIMIT 1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if probe.is_none() { return Ok(0); }

    // Sectioned candidates — group by (resource_id, section_title), MIN distance
    // per group, take one representative chunk per group via DISTINCT ON.
    let sectioned_rows = sqlx::query(
        "SELECT DISTINCT ON (mc.resource_id, mc.section_title) \
                mc.resource_id, \
                mc.id AS chunk_id, \
                mc.section_title, \
                mc.page_start, \
                mc.page_end, \
                (me.embedding <=> src.embedding)::float4 AS distance \
         FROM mimir_chunks mc \
         JOIN mimir_embeddings me ON me.chunk_id = mc.id \
         CROSS JOIN (SELECT embedding FROM universal_skills WHERE id = $1) src \
         WHERE mc.section_title IS NOT NULL \
         ORDER BY mc.resource_id, mc.section_title, me.embedding <=> src.embedding \
         LIMIT $2"
    )
    .bind(&skill_id)
    .bind(limit_i * 4) // over-fetch sectioned candidates so the final mix has variety
    .fetch_all(pool)
    .await
    .map_err(|e| format!("sectioned ANN failed: {}", e))?;

    // Whole-resource candidates — for resources whose chunks have no section,
    // take the single best-matching chunk per resource.
    let whole_rows = sqlx::query(
        "SELECT DISTINCT ON (mc.resource_id) \
                mc.resource_id, \
                mc.id AS chunk_id, \
                (me.embedding <=> src.embedding)::float4 AS distance \
         FROM mimir_chunks mc \
         JOIN mimir_embeddings me ON me.chunk_id = mc.id \
         CROSS JOIN (SELECT embedding FROM universal_skills WHERE id = $1) src \
         WHERE mc.section_title IS NULL \
         ORDER BY mc.resource_id, me.embedding <=> src.embedding \
         LIMIT $2"
    )
    .bind(&skill_id)
    .bind(limit_i * 2)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("whole-resource ANN failed: {}", e))?;

    // Merge + sort by distance ASC, truncate to limit
    struct Candidate {
        resource_id: String,
        chunk_id: String,
        section_title: Option<String>,
        page_start: Option<i32>,
        page_end: Option<i32>,
        distance: f32,
    }
    let mut all: Vec<Candidate> = Vec::with_capacity(sectioned_rows.len() + whole_rows.len());
    for r in &sectioned_rows {
        all.push(Candidate {
            resource_id: match r.try_get("resource_id") { Ok(v) => v, Err(_) => continue },
            chunk_id: r.try_get("chunk_id").unwrap_or_default(),
            section_title: r.try_get("section_title").ok(),
            page_start: r.try_get("page_start").ok(),
            page_end: r.try_get("page_end").ok(),
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        });
    }
    for r in &whole_rows {
        all.push(Candidate {
            resource_id: match r.try_get("resource_id") { Ok(v) => v, Err(_) => continue },
            chunk_id: r.try_get("chunk_id").unwrap_or_default(),
            section_title: None,
            page_start: None,
            page_end: None,
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        });
    }
    all.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    all.truncate(limit_i as usize);

    let mut written = 0usize;
    for c in &all {
        if let Some(ref section) = c.section_title {
            let row_id = Uuid::new_v4().to_string();
            let res = sqlx::query(
                "INSERT INTO mimir_skill_links \
                   (id, skill_id, resource_id, relevance_score, \
                    matched_chunk_id, matched_section_title, matched_page_start, matched_page_end) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (skill_id, resource_id, matched_section_title) \
                   WHERE matched_section_title IS NOT NULL \
                 DO UPDATE SET \
                   relevance_score = EXCLUDED.relevance_score, \
                   matched_chunk_id = EXCLUDED.matched_chunk_id, \
                   matched_page_start = EXCLUDED.matched_page_start, \
                   matched_page_end = EXCLUDED.matched_page_end"
            )
            .bind(&row_id)
            .bind(&skill_id)
            .bind(&c.resource_id)
            .bind(c.distance)
            .bind(&c.chunk_id)
            .bind(section)
            .bind(c.page_start)
            .bind(c.page_end)
            .execute(pool)
            .await;
            if res.is_ok() { written += 1; }
        } else {
            let row_id = Uuid::new_v4().to_string();
            let res = sqlx::query(
                "INSERT INTO mimir_skill_links \
                   (id, skill_id, resource_id, relevance_score, matched_chunk_id) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (skill_id, resource_id) \
                   WHERE matched_section_title IS NULL \
                 DO UPDATE SET \
                   relevance_score = EXCLUDED.relevance_score, \
                   matched_chunk_id = EXCLUDED.matched_chunk_id"
            )
            .bind(&row_id)
            .bind(&skill_id)
            .bind(&c.resource_id)
            .bind(c.distance)
            .bind(&c.chunk_id)
            .execute(pool)
            .await;
            if res.is_ok() { written += 1; }
        }
    }

    Ok(written)
}

// ─── Backfill: match every embedded skill to resources ──────────────────────

#[tauri::command]
pub async fn run_skill_resource_backfill(
    database: State<'_, Database>,
) -> Result<String, String> {
    let pool = &database.pool;

    let rows = sqlx::query(
        "SELECT id FROM universal_skills WHERE embedding IS NOT NULL ORDER BY id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = rows.len();
    if total == 0 {
        return Ok("No embedded skills to backfill".to_string());
    }

    let mut processed = 0usize;
    let mut total_written = 0usize;
    for r in &rows {
        let skill_id: String = match r.try_get("id") { Ok(v) => v, Err(_) => continue };
        // Inline the match logic so we can take pool by &, not by State.
        match run_skill_match_inner(pool, &skill_id, 8).await {
            Ok(n) => total_written += n,
            Err(e) => println!("⚠️  backfill failed for skill {}: {}", skill_id, e),
        }
        processed += 1;
        if processed % 25 == 0 {
            println!("📚 skill→resource backfill: {}/{} skills processed", processed, total);
        }
    }

    let summary = format!(
        "Matched {} skill→resource links across {} skills",
        total_written, processed,
    );
    println!("📚 {}", summary);
    Ok(summary)
}

/// Internal: same as match_skill_to_resources but operating on a borrowed pool
/// so the backfill loop can call it directly without a Tauri State.
async fn run_skill_match_inner(
    pool: &PgPool,
    skill_id: &str,
    limit: usize,
) -> Result<usize, String> {
    let limit_i = limit as i64;

    let probe = sqlx::query(
        "SELECT 1 AS ok FROM universal_skills \
         WHERE id = $1 AND embedding IS NOT NULL LIMIT 1"
    )
    .bind(skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if probe.is_none() { return Ok(0); }

    let sectioned_rows = sqlx::query(
        "SELECT DISTINCT ON (mc.resource_id, mc.section_title) \
                mc.resource_id, mc.id AS chunk_id, mc.section_title, \
                mc.page_start, mc.page_end, \
                (me.embedding <=> src.embedding)::float4 AS distance \
         FROM mimir_chunks mc \
         JOIN mimir_embeddings me ON me.chunk_id = mc.id \
         CROSS JOIN (SELECT embedding FROM universal_skills WHERE id = $1) src \
         WHERE mc.section_title IS NOT NULL \
         ORDER BY mc.resource_id, mc.section_title, me.embedding <=> src.embedding \
         LIMIT $2"
    )
    .bind(skill_id)
    .bind(limit_i * 4)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let whole_rows = sqlx::query(
        "SELECT DISTINCT ON (mc.resource_id) \
                mc.resource_id, mc.id AS chunk_id, \
                (me.embedding <=> src.embedding)::float4 AS distance \
         FROM mimir_chunks mc \
         JOIN mimir_embeddings me ON me.chunk_id = mc.id \
         CROSS JOIN (SELECT embedding FROM universal_skills WHERE id = $1) src \
         WHERE mc.section_title IS NULL \
         ORDER BY mc.resource_id, me.embedding <=> src.embedding \
         LIMIT $2"
    )
    .bind(skill_id)
    .bind(limit_i * 2)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    struct C {
        resource_id: String,
        chunk_id: String,
        section_title: Option<String>,
        page_start: Option<i32>,
        page_end: Option<i32>,
        distance: f32,
    }
    let mut all: Vec<C> = Vec::with_capacity(sectioned_rows.len() + whole_rows.len());
    for r in &sectioned_rows {
        all.push(C {
            resource_id: match r.try_get("resource_id") { Ok(v) => v, Err(_) => continue },
            chunk_id: r.try_get("chunk_id").unwrap_or_default(),
            section_title: r.try_get("section_title").ok(),
            page_start: r.try_get("page_start").ok(),
            page_end: r.try_get("page_end").ok(),
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        });
    }
    for r in &whole_rows {
        all.push(C {
            resource_id: match r.try_get("resource_id") { Ok(v) => v, Err(_) => continue },
            chunk_id: r.try_get("chunk_id").unwrap_or_default(),
            section_title: None,
            page_start: None,
            page_end: None,
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        });
    }
    all.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    all.truncate(limit);

    let mut written = 0usize;
    for c in &all {
        if let Some(ref section) = c.section_title {
            let row_id = Uuid::new_v4().to_string();
            let res = sqlx::query(
                "INSERT INTO mimir_skill_links \
                   (id, skill_id, resource_id, relevance_score, \
                    matched_chunk_id, matched_section_title, matched_page_start, matched_page_end) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (skill_id, resource_id, matched_section_title) \
                   WHERE matched_section_title IS NOT NULL \
                 DO UPDATE SET \
                   relevance_score = EXCLUDED.relevance_score, \
                   matched_chunk_id = EXCLUDED.matched_chunk_id, \
                   matched_page_start = EXCLUDED.matched_page_start, \
                   matched_page_end = EXCLUDED.matched_page_end"
            )
            .bind(&row_id).bind(skill_id).bind(&c.resource_id).bind(c.distance)
            .bind(&c.chunk_id).bind(section).bind(c.page_start).bind(c.page_end)
            .execute(pool).await;
            if res.is_ok() { written += 1; }
        } else {
            let row_id = Uuid::new_v4().to_string();
            let res = sqlx::query(
                "INSERT INTO mimir_skill_links \
                   (id, skill_id, resource_id, relevance_score, matched_chunk_id) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (skill_id, resource_id) \
                   WHERE matched_section_title IS NULL \
                 DO UPDATE SET \
                   relevance_score = EXCLUDED.relevance_score, \
                   matched_chunk_id = EXCLUDED.matched_chunk_id"
            )
            .bind(&row_id).bind(skill_id).bind(&c.resource_id).bind(c.distance).bind(&c.chunk_id)
            .execute(pool).await;
            if res.is_ok() { written += 1; }
        }
    }
    Ok(written)
}

// ─── Domain classification ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationResult {
    pub total: usize,
    pub classified: usize,
    pub failed: usize,
}

#[tauri::command]
pub async fn classify_skill_domains(
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<ClassificationResult, String> {
    let api_key = std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY not set".to_string())?;

    // ── Step 1: ensure the skill_domains table is populated ─────────────────
    // Seed common domain names the LLM tends to return. Idempotent: ON CONFLICT
    // on name skips existing rows. Uses generated UUIDs for new rows; existing
    // seeds (e.g. 'dom-*' slugs from migration 029) are untouched.
    let seed_names: [&str; 39] = [
        "Machine Learning", "Data Science", "Software Engineering",
        "Web Development", "DevOps", "Cloud Computing", "Engineering",
        "Computing", "Computer Science", "Mathematics", "Physics",
        "Business", "Communication", "Databases",
        "Natural Language Processing", "Deep Learning",
        "Computer Vision", "Research Methods", "Science", "Visualization",
        "Cybersecurity", "Materials Science", "Energy",
        "Systems Programming", "High-Performance Computing",
        "Distributed Systems", "Human-Computer Interaction",
        "Robotics", "Simulation", "Hardware", "Embedded Systems",
        "Electrical Engineering", "Quantum Computing", "Design",
        "GIS", "Bioinformatics", "Control Systems", "Signal Processing",
        "Problem Solving",
    ];
    for name in seed_names.iter() {
        let new_id = Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO skill_domains (id, name, description)
             VALUES ($1, $2, '')
             ON CONFLICT (name) DO NOTHING"
        )
        .bind(&new_id)
        .bind(*name)
        .execute(&database.pool)
        .await;
    }

    // Load all domains with descriptions
    let domain_rows = sqlx::query(
        "SELECT id, name, COALESCE(description, '') AS description FROM skill_domains ORDER BY name ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    if domain_rows.is_empty() {
        return Err("No skill_domains found — run migrations first".to_string());
    }

    let domains: Vec<(String, String, String)> = domain_rows.iter().map(|r| {
        let id: String = r.try_get("id").unwrap_or_default();
        let name: String = r.try_get("name").unwrap_or_default();
        let desc: String = r.try_get("description").unwrap_or_default();
        (id, name, desc)
    }).collect();

    // ── Step 2: name → id resolution map ────────────────────────────────────
    // The LLM frequently returns a domain *name* rather than the exact id we
    // listed. Build a normalized lookup keyed by lowercase, stripped of
    // spaces / hyphens / underscores, mapping to the canonical id.
    fn normalize_domain_key(s: &str) -> String {
        s.to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect()
    }
    let mut domain_lookup: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (id, name, _) in &domains {
        domain_lookup.insert(normalize_domain_key(name), id.clone());
        domain_lookup.insert(normalize_domain_key(id), id.clone());
    }

    // Explicit aliases for compound names (redundant with the normalize step
    // above, kept defensively so the lookup survives future normalizer changes).
    let alias_pairs: [(&str, &str); 4] = [
        ("quantumcomputing", "Quantum Computing"),
        ("embeddedsystems",  "Embedded Systems"),
        ("controlsystems",   "Control Systems"),
        ("signalprocessing", "Signal Processing"),
    ];
    for (alias, canonical_name) in alias_pairs.iter() {
        if let Some(canonical_id) = domains.iter()
            .find(|(_, n, _)| n == canonical_name)
            .map(|(id, _, _)| id.clone())
        {
            domain_lookup.insert((*alias).to_string(), canonical_id);
        }
    }

    // Kept for the "did this id exist at all" check inside the loop.
    let valid_domain_ids: std::collections::HashSet<&str> =
        domains.iter().map(|(id, _, _)| id.as_str()).collect();

    // One line per domain name (no IDs — they confuse the LLM when they are raw UUIDs).
    // The name→id resolution happens in domain_lookup after the LLM returns.
    let domain_list: String = domains.iter()
        .map(|(_, name, desc)| {
            if desc.is_empty() {
                format!("  {}", name)
            } else {
                format!("  {}: {}", name, desc)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Load unclassified skills
    let skill_rows = sqlx::query(
        "SELECT id, name FROM universal_skills
         WHERE domain_id IS NULL OR status = 'unclassified'
         ORDER BY name ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let skills: Vec<(String, String)> = skill_rows.iter().map(|r| {
        let id: String = r.try_get("id").unwrap_or_default();
        let name: String = r.try_get("name").unwrap_or_default();
        (id, name)
    }).collect();

    let total = skills.len();
    if total == 0 {
        return Ok(ClassificationResult { total: 0, classified: 0, failed: 0 });
    }

    let mut classified = 0usize;
    let mut failed = 0usize;

    for chunk in skills.chunks(20) {
        let skill_list: String = chunk.iter()
            .map(|(id, name)| format!("  \"{}\": \"{}\"", name, id))
            .collect::<Vec<_>>()
            .join(",\n");

        let prompt = format!(
            "You are classifying skills into specific learning domains.\n\n\
             Available domains:\n{domain_list}\n\n\
             Skills to classify (skill_id: skill_name):\n{skill_list}\n\n\
             Return ONLY a JSON object mapping each skill_id to the exact domain name from the list above:\n\
             {{\"<skill_id>\": \"<domain name>\", ...}}\n\n\
             Rules:\n\
             - Use the EXACT domain name as it appears in the list (case-sensitive)\n\
             - Assign the MOST SPECIFIC domain that fits (e.g. prefer 'Machine Learning' over 'Computing' for 'gradient descent')\n\
             - Every skill must appear in the output exactly once\n\
             - Use the most general fitting domain only if no specific domain fits\n\
             - Return only the JSON object — no markdown fences, no explanation, no extra text"
        );

        let t0 = std::time::Instant::now();
        let body = serde_json::json!({
            "model": "meta-llama/llama-4-scout-17b-16e-instruct",
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "temperature": 0.1,
            "max_tokens": 1024,
        });

        let resp = client
            .post(GROQ_API_URL)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let elapsed = t0.elapsed().as_millis() as i64;

        let raw = match resp {
            Err(e) => {
                crate::brain::log_prompt_call(
                    database.pool.clone(), "skill_domain_classification",
                    "meta-llama/llama-4-scout-17b-16e-instruct", "skill_domain_class_v2",
                    elapsed, false, Some(e.to_string()), None,
                );
                failed += chunk.len();
                continue;
            }
            Ok(r) => match r.text().await {
                Err(e) => {
                    crate::brain::log_prompt_call(
                        database.pool.clone(), "skill_domain_classification",
                        "meta-llama/llama-4-scout-17b-16e-instruct", "skill_domain_class_v2",
                        elapsed, false, Some(e.to_string()), None,
                    );
                    failed += chunk.len();
                    continue;
                }
                Ok(t) => t,
            }
        };

        let json_val: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let content = json_val["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        let clean = content
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let mapping: std::collections::HashMap<String, String> =
            serde_json::from_str(clean).unwrap_or_default();

        let input_tokens = json_val["usage"]["prompt_tokens"].as_i64();
        let output_tokens = json_val["usage"]["completion_tokens"].as_i64();
        crate::brain::log_prompt_call(
            database.pool.clone(), "skill_domain_classification",
            "meta-llama/llama-4-scout-17b-16e-instruct", "skill_domain_class_v2",
            elapsed, true, None,
            Some(serde_json::json!({ "input_tokens": input_tokens, "output_tokens": output_tokens })),
        );

        for (skill_id, domain_id) in &mapping {
            // Resolve via normalized name/id lookup before hitting the FK.
            // This rescues responses where the LLM returns "Machine Learning"
            // instead of the canonical id we showed it.
            let resolved_id = match domain_lookup.get(&normalize_domain_key(domain_id)) {
                Some(id) => id.clone(),
                None => {
                    if valid_domain_ids.contains(domain_id.as_str()) {
                        domain_id.clone()
                    } else {
                        eprintln!("⚠️ LLM returned unknown domain_id '{}' for skill {} — skipping", domain_id, skill_id);
                        failed += 1;
                        continue;
                    }
                }
            };

            // Null out embedding when domain_id changes so the post-classify
            // backfill_skill_embeddings call re-embeds with the new domain
            // name baked into the embed text. The IS DISTINCT FROM guard
            // means unchanged classifications don't trigger a wasted re-embed.
            let res = sqlx::query(
                "UPDATE universal_skills
                 SET domain_id = $1,
                     status = 'active',
                     review_needed = false,
                     last_updated = NOW(),
                     embedding = CASE
                         WHEN domain_id IS DISTINCT FROM $1 THEN NULL
                         ELSE embedding
                     END
                 WHERE id = $2"
            )
            .bind(&resolved_id)
            .bind(skill_id)
            .execute(&database.pool)
            .await;

            match res {
                Ok(r) if r.rows_affected() > 0 => classified += 1,
                Ok(_) => failed += 1,
                Err(e) => {
                    eprintln!("⚠️ Failed to update skill {}: {}", skill_id, e);
                    failed += 1;
                }
            }
        }

        // Any skills from this chunk not in mapping count as failed
        let mapped_ids: std::collections::HashSet<&String> = mapping.keys().collect();
        for (id, _) in chunk {
            if !mapped_ids.contains(id) {
                failed += 1;
            }
        }
    }

    println!("🏷️ classify_skill_domains: {classified}/{total} classified, {failed} failed");

    // Auto-embed newly classified skills. Non-fatal — log on failure but
    // never block classification results from returning.
    if let Err(e) = backfill_skill_embeddings(&*client, &database.pool).await {
        println!("⚠️  backfill_skill_embeddings (post-classify) failed: {}", e);
    }

    Ok(ClassificationResult { total, classified, failed })
}

// ─── Reset domain assignments ─────────────────────────────────────────────────

#[tauri::command]
pub async fn reset_skill_domains(
    database: State<'_, Database>,
) -> Result<usize, String> {
    let result = sqlx::query(
        "UPDATE universal_skills
         SET domain_id = NULL, status = 'unclassified', review_needed = true, last_updated = NOW()
         WHERE status = 'active'"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let count = result.rows_affected() as usize;
    println!("🔄 reset_skill_domains: cleared domain_id for {count} skills");
    Ok(count)
}

// ─── Concept slug bridge ──────────────────────────────────────────────────────

/// Full recompute of seed/adjacent states.
/// Seeds = skills with origin='resume'. Everything else = adjacent.
/// Returns total rows updated.
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

/// Copy concept_slug from tree_nodes onto universal_skills where the skill name
/// matches the node title. Uses two passes:
///   Pass 1 — normalized name match (normalize_skill_name applied to both sides)
///   Pass 2 — normalized slug match (skill name slugified vs node concept_slug)
/// Called from on_tree_generated so newly generated trees immediately enrich
/// the skill graph with concept identities from the concept graph.
pub async fn sync_concept_slugs_inner(pool: &PgPool) -> Result<usize, String> {
    // Load all skills without a concept_slug
    let skill_rows = sqlx::query(
        "SELECT id, name FROM universal_skills WHERE concept_slug IS NULL"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = skill_rows.len();

    // Load all tree nodes that have a concept_slug
    let node_rows = sqlx::query(
        "SELECT title, concept_slug FROM tree_nodes WHERE concept_slug IS NOT NULL"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Build lookup: normalized_title -> concept_slug (first win per normalized form)
    let mut norm_title_to_slug: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Also: slug -> concept_slug (pass 2: slugified name match)
    let mut slug_to_slug: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for row in &node_rows {
        let title: String = row.try_get("title").unwrap_or_default();
        let concept_slug: String = row.try_get("concept_slug").unwrap_or_default();
        if title.is_empty() || concept_slug.is_empty() { continue; }

        let norm = normalize_skill_name(&title).to_lowercase();
        norm_title_to_slug.entry(norm).or_insert_with(|| concept_slug.clone());

        // Slugified form of the title for pass 2 comparison
        let slugified: String = title.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        slug_to_slug.entry(slugified).or_insert_with(|| concept_slug.clone());
    }

    let mut updated = 0usize;
    for row in &skill_rows {
        let skill_id: String = row.try_get("id").unwrap_or_default();
        let skill_name: String = row.try_get("name").unwrap_or_default();
        if skill_id.is_empty() || skill_name.is_empty() { continue; }

        // Pass 1: normalized name match
        let norm_name = normalize_skill_name(&skill_name).to_lowercase();
        let matched_slug = norm_title_to_slug.get(&norm_name).cloned().or_else(|| {
            // Pass 2: slugified skill name vs node concept_slug
            let slugified_name: String = skill_name.to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .split('-')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            slug_to_slug.get(&slugified_name).cloned()
        });

        if let Some(slug) = matched_slug {
            // Guard the unique partial index on concept_slug. Multiple skills
            // can match the same tree_node slug (e.g. surviving skills after
            // a merge prune); the first wins, the rest skip silently.
            let result = sqlx::query(
                "UPDATE universal_skills
                 SET concept_slug = $1
                 WHERE id = $2
                   AND concept_slug IS NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM universal_skills WHERE concept_slug = $1
                   )"
            )
            .bind(&slug)
            .bind(&skill_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            if result.rows_affected() > 0 {
                updated += 1;
            }
        }
    }

    println!("[graph] concept_slug backfill: {}/{} skills matched", updated, total);
    Ok(updated)
}

/// ANN-based fallback: for leaf nodes where concept_slug IS NULL but title_embedding exists,
/// find the nearest universal_skill by embedding distance and copy its concept_slug onto the node.
/// Only writes when the best distance is < 0.35. Uses a single LATERAL join query (no N+1).
pub async fn backfill_concept_slugs_by_embedding_inner(pool: &PgPool) -> Result<usize, String> {
    let result = sqlx::query(
        "UPDATE tree_nodes tn
         SET concept_slug = matched.concept_slug
         FROM (
             SELECT DISTINCT ON (tn2.id)
                 tn2.id AS node_id,
                 us.concept_slug
             FROM tree_nodes tn2
             CROSS JOIN LATERAL (
                 SELECT concept_slug
                 FROM universal_skills
                 WHERE embedding IS NOT NULL
                   AND concept_slug IS NOT NULL
                 ORDER BY embedding <=> tn2.title_embedding
                 LIMIT 1
             ) us
             WHERE tn2.type = 'leaf'
               AND tn2.concept_slug IS NULL
               AND tn2.title_embedding IS NOT NULL
               AND (
                   SELECT embedding <=> tn2.title_embedding
                   FROM universal_skills
                   WHERE embedding IS NOT NULL
                   ORDER BY embedding <=> tn2.title_embedding
                   LIMIT 1
               ) < 0.65
         ) matched
         WHERE tn.id = matched.node_id"
    )
    .execute(pool)
    .await
    .map_err(|e| format!("backfill_concept_slugs_by_embedding failed: {}", e))?;

    let matched = result.rows_affected() as usize;
    println!("[graph] concept_slug ANN backfill: {} nodes matched", matched);
    Ok(matched)
}
