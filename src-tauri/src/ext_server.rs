// ext_server.rs — Axum HTTP bridge for the Yggdrasil Chrome extension.
// Binds to 127.0.0.1:1421. Auth via X-Ygg-Key header (YGG_EXT_KEY env var).

use axum::{
    extract::{Path, State as AxState},
    http::{header, HeaderMap, HeaderName, Method, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::collections::HashSet;
use std::net::SocketAddr;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

use tauri::AppHandle;
use crate::job_commands::do_extract_skills;
use crate::orchestrator::JobQueue;

#[derive(Clone)]
pub struct ExtState {
    pool: PgPool,
    client: reqwest::Client,
    api_key: String,
    app: AppHandle,
    queue: JobQueue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateJobBody {
    company: String,
    position: String,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    link: Option<String>,
    season: String,
    #[serde(default)]
    job_description: Option<String>,
}

#[derive(Serialize)]
struct ExtProject {
    name: String,
    description: Option<String>,
    github_url: Option<String>,
    score: f32,
}

fn normalize_skill(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_auth(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("x-ygg-key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

async fn tailored_for_job(pool: &PgPool, job_id: &str) -> Vec<ExtProject> {
    let skills_rows = match sqlx::query(
        "SELECT skill_name FROM job_skills WHERE job_id = $1 AND is_required = true",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return vec![],
    };

    let required_skills: Vec<String> = skills_rows
        .iter()
        .filter_map(|r| r.try_get::<String, _>("skill_name").ok().map(|s| normalize_skill(&s)))
        .collect();

    if required_skills.is_empty() {
        return vec![];
    }

    let proj_rows = match sqlx::query(
        "SELECT rp.name, rp.description, rp.tech_stack, rp.github_url
         FROM resume_projects rp
         JOIN resume_profile rprofile ON rp.resume_id = rprofile.id
         ORDER BY rprofile.created_at DESC, rp.created_at
         LIMIT 100",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return vec![],
    };

    let mut projects: Vec<ExtProject> = proj_rows
        .iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let description: Option<String> = r.try_get("description").ok().flatten();
            let github_url: Option<String> = r.try_get::<Option<String>, _>("github_url").ok().flatten();
            let tech_json: Option<Value> = r.try_get("tech_stack").ok();

            let tech_set: HashSet<String> = tech_json
                .and_then(|v| {
                    v.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| normalize_skill(s)))
                            .collect()
                    })
                })
                .unwrap_or_default();

            let matched = required_skills.iter().filter(|s| tech_set.contains(*s)).count();
            let score = matched as f32 / required_skills.len() as f32;

            Some(ExtProject { name, description, github_url, score })
        })
        .collect();

    projects.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    projects.truncate(4);
    projects
}

async fn create_job_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
    Json(body): Json<CreateJobBody>,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let company = body.company.trim().to_string();
    let position = body.position.trim().to_string();

    if company.is_empty() || position.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "company and position are required" })),
        )
            .into_response();
    }

    // Dedup: only when a non-empty link is provided
    if let Some(ref link) = body.link {
        if !link.trim().is_empty() {
            let existing = sqlx::query(
                "SELECT id FROM job_applications WHERE company = $1 AND position = $2 AND link = $3",
            )
            .bind(&company)
            .bind(&position)
            .bind(link.trim())
            .fetch_optional(&state.pool)
            .await;

            if let Ok(Some(row)) = existing {
                if let Ok(id) = row.try_get::<String, _>("id") {
                    let projects = tailored_for_job(&state.pool, &id).await;
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({ "id": id, "tailored_projects": projects })),
                    )
                        .into_response();
                }
            }
        }
    }

    let id = Uuid::new_v4().to_string();
    let link = body.link.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());

    let source_val = body
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("extension")
        .to_string();

    if let Err(e) = sqlx::query(
        "INSERT INTO job_applications (id, company, position, location, source, link, season) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&id)
    .bind(&company)
    .bind(&position)
    .bind(&body.location)
    .bind(Some(source_val.as_str()))
    .bind(link)
    .bind(body.season.trim())
    .execute(&state.pool)
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    if let Some(ref jd) = body.job_description {
        if !jd.trim().is_empty() {
            let _ = sqlx::query("UPDATE job_applications SET job_description = $1 WHERE id = $2")
                .bind(jd)
                .bind(&id)
                .execute(&state.pool)
                .await;

            let _ = do_extract_skills(&state.client, &id, jd, &state.pool).await;
        }
    }

    let projects = tailored_for_job(&state.pool, &id).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "id": id, "tailored_projects": projects })),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateJobBody {
    status: String,
    #[serde(default)]
    location_rating: Option<i32>,
    #[serde(default)]
    alignment_rating: Option<i32>,
    #[serde(default)]
    salary_rating: Option<i32>,
    #[serde(default)]
    role_rating: Option<i32>,
}

async fn update_job_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateJobBody>,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    let valid_statuses = ["saved", "applied", "interviewing", "offer", "rejected"];
    if !valid_statuses.contains(&body.status.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid status" })),
        )
            .into_response();
    }

    let result = sqlx::query(
        "UPDATE job_applications \
         SET status = $1, \
             rating_location  = COALESCE($2, rating_location), \
             rating_alignment = COALESCE($3, rating_alignment), \
             rating_salary    = COALESCE($4, rating_salary), \
             rating_role      = COALESCE($5, rating_role) \
         WHERE id = $6",
    )
    .bind(&body.status)
    .bind(body.location_rating)
    .bind(body.alignment_rating)
    .bind(body.salary_rating)
    .bind(body.role_rating)
    .bind(&id)
    .execute(&state.pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "job not found" })),
        )
            .into_response(),
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn meta_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )
            .into_response();
    }

    // Collect distinct raw values, split on commas, deduplicate, sort
    async fn distinct_values(pool: &PgPool, col: &str) -> Vec<String> {
        let query = format!(
            "SELECT DISTINCT {col} FROM job_applications WHERE {col} IS NOT NULL AND {col} != ''"
        );
        let rows = sqlx::query(&query).fetch_all(pool).await.unwrap_or_default();
        let mut set = HashSet::new();
        for row in &rows {
            if let Ok(v) = row.try_get::<String, &str>(col) {
                for part in v.split(',') {
                    let part = part.trim().to_string();
                    if !part.is_empty() {
                        set.insert(part);
                    }
                }
            }
        }
        let mut out: Vec<String> = set.into_iter().collect();
        out.sort();
        out
    }

    let sources = distinct_values(&state.pool, "source").await;
    let locations = distinct_values(&state.pool, "location").await;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "sources": sources, "locations": locations })),
    )
        .into_response()
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

pub async fn start(pool: PgPool, client: reqwest::Client, api_key: String, app: AppHandle, queue: JobQueue) {
    let state = ExtState { pool, client, api_key, app, queue };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            let bytes = origin.as_bytes();
            bytes.starts_with(b"chrome-extension://")
                || bytes.starts_with(b"http://localhost")
                || bytes.starts_with(b"http://127.0.0.1")
        }))
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("x-ygg-key"),
        ]);

    let router = Router::new()
        .route("/api/jobs", post(create_job_handler))
        .route("/api/jobs/meta", get(meta_handler))
        .route("/api/jobs/:id", patch(update_job_handler))
        .route("/api/health", get(health_handler))
        .layer(cors)
        .with_state(state);

    let addr: SocketAddr = "127.0.0.1:1421".parse().unwrap();
    println!("Extension server listening on {}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Extension server failed to bind to {}: {}", addr, e);
            return;
        }
    };

    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("Extension server error: {}", e);
    }
}
