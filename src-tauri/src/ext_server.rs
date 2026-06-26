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
use chrono;

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
struct CreateLibraryBody {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Serialize)]
struct CreateLibraryResponse {
    id: String,
    title: String,
    #[serde(rename = "type")]
    resource_type: String,
    already_existed: bool,
}

#[derive(Deserialize)]
struct LibraryQuery {
    q: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct LibraryEntry {
    id: String,
    title: String,
    url: String,
    #[serde(rename = "type")]
    resource_type: String,
    tags: Vec<String>,
    user_notes: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct UpdateLibraryBody {
    note: Option<String>,
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

fn infer_resource_type(url: &str) -> &'static str {
    if url.contains("youtube.com") || url.contains("youtu.be") {
        "video"
    } else if url.contains("arxiv.org") || url.to_lowercase().ends_with(".pdf") {
        "paper"
    } else if url.contains("github.com") {
        "repo"
    } else {
        "webpage"
    }
}

fn extract_hostname(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .trim_start_matches("www.")
        .to_string()
}

async fn fetch_page_title(client: &reqwest::Client, url: &str) -> String {
    use scraper::{Html, Selector};
    match client
        .get(url)
        .header("User-Agent", "Yggdrasil/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(html) = resp.text().await {
                let doc = Html::parse_document(&html);
                if let Some(el) = Selector::parse("title")
                    .ok()
                    .and_then(|s| doc.select(&s).next())
                {
                    let t = el.text().collect::<String>().trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            extract_hostname(url)
        }
        _ => extract_hostname(url),
    }
}

async fn create_library_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
    Json(body): Json<CreateLibraryBody>,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "url is required" }))).into_response();
    }

    // Dedup check
    let existing = sqlx::query("SELECT id, title, type FROM mimir_resources WHERE url = $1 LIMIT 1")
        .bind(&url)
        .fetch_optional(&state.pool)
        .await;

    if let Ok(Some(row)) = existing {
        let id: String = row.try_get("id").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_default();
        let resource_type: String = row.try_get::<String, _>("type").unwrap_or_else(|_| "webpage".to_string());
        return (StatusCode::OK, Json(serde_json::json!({
            "id": id,
            "title": title,
            "type": resource_type,
            "alreadyExisted": true,
        }))).into_response();
    }

    let resource_type = infer_resource_type(&url);
    let id = Uuid::new_v4().to_string();

    let title = match body.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => fetch_page_title(&state.client, &url).await,
    };

    let user_notes = body.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.to_string());

    if let Err(e) = sqlx::query(
        "INSERT INTO mimir_resources (id, title, url, type, status, user_notes) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(&title)
    .bind(&url)
    .bind(resource_type)
    .bind("read")
    .bind(&user_notes)
    .execute(&state.pool)
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response();
    }

    // Fire background pipeline — response returns immediately
    let pool_bg = state.pool.clone();
    let client_bg = state.client.clone();
    let app_bg = state.app.clone();
    let queue_bg = state.queue.clone();
    let id_bg = id.clone();
    let url_bg = url.clone();

    tokio::spawn(async move {
        use crate::mimir_ingest::{fetch_url_content_pub, store_chunks_and_embeddings};
        match fetch_url_content_pub(&client_bg, &url_bg, false).await {
            Ok(fetch_result) if fetch_result.text.len() >= 50 => {
                match pool_bg.begin().await {
                    Ok(mut tx) => {
                        match store_chunks_and_embeddings(&mut tx, &client_bg, &id_bg, &fetch_result.text).await {
                            Ok(_) => {
                                if let Err(e) = tx.commit().await {
                                    eprintln!("[library/ingest] commit failed for {}: {}", id_bg, e);
                                    return;
                                }
                            }
                            Err(e) => eprintln!("[library/ingest] chunk/embed failed for {}: {}", id_bg, e),
                        }
                    }
                    Err(e) => eprintln!("[library/ingest] begin tx failed for {}: {}", id_bg, e),
                }
            }
            Ok(_) => eprintln!("[library/ingest] text too short, skipping chunks for {}", id_bg),
            Err(e) => eprintln!("[library/ingest] fetch failed for {}: {}", id_bg, e),
        }
        crate::orchestrator::on_resource_ingested_async(&pool_bg, &app_bg, &client_bg, &id_bg, &queue_bg).await;
    });

    (StatusCode::OK, Json(serde_json::json!({
        "id": id,
        "title": title,
        "type": resource_type,
        "alreadyExisted": false,
    }))).into_response()
}

async fn list_library_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<LibraryQuery>,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let limit = params.limit.unwrap_or(50).min(200);

    let rows = match &params.q {
        Some(q) if !q.trim().is_empty() => {
            let pattern = format!("%{}%", q.trim());
            sqlx::query(
                "SELECT id, title, url, type, tags, user_notes, created_at \
                 FROM mimir_resources \
                 WHERE url IS NOT NULL \
                   AND (title ILIKE $1 OR url ILIKE $1 OR $2 = ANY(tags)) \
                 ORDER BY created_at DESC \
                 LIMIT $3",
            )
            .bind(&pattern)
            .bind(q.trim())
            .bind(limit)
            .fetch_all(&state.pool)
            .await
        }
        _ => {
            sqlx::query(
                "SELECT id, title, url, type, tags, user_notes, created_at \
                 FROM mimir_resources \
                 WHERE url IS NOT NULL \
                 ORDER BY created_at DESC \
                 LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&state.pool)
            .await
        }
    };

    let rows = match rows {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    };

    let resources: Vec<LibraryEntry> = rows
        .iter()
        .filter_map(|row| {
            let id: String = row.try_get("id").ok()?;
            let title: String = row.try_get("title").ok()?;
            let url: String = row.try_get("url").ok()?;
            let resource_type: String = row.try_get::<String, _>("type").unwrap_or_else(|_| "webpage".to_string());
            let tags: Vec<String> = row.try_get::<Vec<String>, _>("tags").unwrap_or_default();
            let user_notes: Option<String> = row.try_get("user_notes").ok().flatten();
            let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").ok()?;
            Some(LibraryEntry {
                id,
                title,
                url,
                resource_type,
                tags,
                user_notes,
                created_at: created_at.to_rfc3339(),
            })
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!({ "resources": resources }))).into_response()
}

async fn update_library_handler(
    AxState(state): AxState<ExtState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateLibraryBody>,
) -> impl IntoResponse {
    if !check_auth(&headers, &state.api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))).into_response();
    }

    let result = sqlx::query(
        "UPDATE mimir_resources SET user_notes = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(body.note.as_deref())
    .bind(&id)
    .execute(&state.pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "resource not found" })),
        ).into_response(),
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ).into_response(),
    }
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
        .route("/api/library", post(create_library_handler))
        .route("/api/library", get(list_library_handler))
        .route("/api/library/:id", patch(update_library_handler))
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
