# Library Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Library tab to the Chrome extension so users can save any webpage to Mimir with one click, browse recent resources, and search them.

**Architecture:** Three new Axum routes on port 1421 handle ingest (POST), listing/search (GET), and note updates (PATCH). The POST handler inserts synchronously and fires the full ingest pipeline (scrape → chunk → embed → auto-tag → match nodes → extract skills) in a `tokio::spawn` background task. The extension gains a second tab that manages its own browse/saving/search state without touching the existing job flow.

**Tech Stack:** Rust/Axum (ext_server.rs), sqlx/Postgres, reqwest, tauri AppHandle, TypeScript (popup.ts, background.ts), Chrome Extension MV3

## Global Constraints

- Rust: `$N` placeholders only (never `?N`); `Result<T, String>` returns; no `?` propagation across async spawn boundaries
- sqlx: `sqlx::query()` non-macro form only; `.try_get("col")` for column access
- Tags column is `TEXT[]` — use `row.try_get::<Vec<String>, _>("tags")`; unwrap with `.unwrap_or_default()`
- `created_at` is `TIMESTAMPTZ` — use `chrono::DateTime<chrono::Utc>` and `.to_rfc3339()`
- Extension: no new fonts or colors beyond existing palette (`#4f98a3` teal, `#171614` bg, `#cdccca` text, `#2a2825` borders)
- Auth on all new routes: `X-Ygg-Key` header via `check_auth()` (existing helper)
- `PATCH /api/library/:id` requires `Method::PATCH` in CORS allow_methods — already fixed in prior session
- All extension types in camelCase for API bodies; serde uses `rename_all = "camelCase"` on Rust structs
- No new migrations — `mimir_resources.user_notes TEXT` exists since migration 003

---

### Task 1: Extend ExtState with AppHandle and JobQueue

**Files:**
- Modify: `src-tauri/src/ext_server.rs` (top ~30 lines — struct and start fn)
- Modify: `src-tauri/src/main.rs:71-77`

**Interfaces:**
- Produces: `ExtState` with fields `pool: PgPool`, `client: reqwest::Client`, `api_key: String`, `app: tauri::AppHandle`, `queue: crate::orchestrator::JobQueue`
- Produces: `ext_server::start(pool, client, api_key, app, queue)` — new signature consumed by Tasks 2-3

- [ ] **Step 1: Add imports to ext_server.rs**

At the top of `src-tauri/src/ext_server.rs`, add to the existing use block:

```rust
use tauri::AppHandle;
use crate::orchestrator::JobQueue;
```

- [ ] **Step 2: Extend ExtState struct**

Replace the existing `ExtState` struct in `ext_server.rs`:

```rust
#[derive(Clone)]
pub struct ExtState {
    pool: PgPool,
    client: reqwest::Client,
    api_key: String,
    app: AppHandle,
    queue: JobQueue,
}
```

- [ ] **Step 3: Update start() signature and body**

Replace the existing `pub async fn start(...)` signature and state construction in `ext_server.rs`:

```rust
pub async fn start(pool: PgPool, client: reqwest::Client, api_key: String, app: AppHandle, queue: JobQueue) {
    let state = ExtState { pool, client, api_key, app, queue };
    // ... rest of function unchanged
```

- [ ] **Step 4: Update main.rs call site**

In `src-tauri/src/main.rs`, replace line 77 (the `tokio::spawn(ext_server::start(...))` call) with:

```rust
let ext_queue = queue.clone();
let ext_app = app_handle.clone();
tokio::spawn(ext_server::start(pool, http_client, ext_key, ext_app, ext_queue));
```

This must appear after `let queue = orchestrator::start_worker(...)` and before `app_handle.manage(queue)` — or after, since `queue.clone()` works either way as long as `queue` hasn't been moved. Place it immediately before `app_handle.manage(queue)` to be safe:

```rust
let queue = orchestrator::start_worker(pool.clone(), app_handle.clone(), http_client.clone());
app_handle.manage(http_client.clone());

let ext_key = std::env::var("YGG_EXT_KEY")
    .unwrap_or_else(|_| "ygg-local-dev".to_string());
let ext_queue = queue.clone();
let ext_app = app_handle.clone();
tokio::spawn(ext_server::start(pool, http_client, ext_key, ext_app, ext_queue));

app_handle.manage(queue);
```

- [ ] **Step 5: Verify cargo check passes**

```
cd src-tauri && cargo check 2>&1
```

Expected: same 5 pre-existing warnings, zero errors. If `JobQueue` doesn't implement `Clone`, check `src-tauri/src/orchestrator.rs` for the struct definition and add `#[derive(Clone)]` there if it wraps a channel sender (which is always Clone).

- [ ] **Step 6: Commit**

```
git add src-tauri/src/ext_server.rs src-tauri/src/main.rs
git commit -m "feat(ext): wire AppHandle and JobQueue into ExtState"
```

---

### Task 2: Add POST /api/library endpoint

**Files:**
- Modify: `src-tauri/src/ext_server.rs`

**Interfaces:**
- Consumes: `ExtState` from Task 1 (with `app` and `queue`)
- Consumes: `crate::mimir_ingest::fetch_url_content_pub`, `crate::mimir_ingest::store_chunks_and_embeddings`, `crate::orchestrator::on_resource_ingested_async` (all already pub)
- Produces: `POST /api/library` → `{ id, title, type, alreadyExisted }`

- [ ] **Step 1: Add request/response types**

Add these structs to `ext_server.rs` (after the existing `UpdateJobBody` struct):

```rust
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
```

- [ ] **Step 2: Add helper functions**

Add these two helpers to `ext_server.rs` (before `create_job_handler`):

```rust
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
```

Note: `scraper` is already a dependency (used in `mimir_ingest.rs`). Add `use scraper::{Html, Selector};` inside `fetch_page_title` as shown.

- [ ] **Step 3: Add create_library_handler**

Add this handler function to `ext_server.rs` (after `meta_handler`):

```rust
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
        let resource_type: String = row.try_get("type").unwrap_or_else(|_| "webpage".to_string());
        return (StatusCode::OK, Json(serde_json::json!({
            "id": id,
            "title": title,
            "type": resource_type,
            "alreadyExisted": true,
        }))).into_response();
    }

    let resource_type = infer_resource_type(&url);
    let id = Uuid::new_v4().to_string();

    // Resolve title: provided > fetched > hostname
    let title = match body.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => fetch_page_title(&state.client, &url).await,
    };

    // Insert resource row
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
```

- [ ] **Step 4: Register the route**

In the `let router = Router::new()` block in `ext_server.rs`, add the new route:

```rust
let router = Router::new()
    .route("/api/jobs", post(create_job_handler))
    .route("/api/jobs/meta", get(meta_handler))
    .route("/api/jobs/:id", patch(update_job_handler))
    .route("/api/health", get(health_handler))
    .route("/api/library", post(create_library_handler))  // ← add this
    .layer(cors)
    .with_state(state);
```

- [ ] **Step 5: Verify cargo check**

```
cd src-tauri && cargo check 2>&1
```

Expected: same pre-existing warnings, zero errors. If `type` column name causes a conflict with sqlx's `try_get`, use `try_get::<String, _>("type")` explicitly.

- [ ] **Step 6: Commit**

```
git add src-tauri/src/ext_server.rs
git commit -m "feat(ext): add POST /api/library endpoint with background ingest pipeline"
```

---

### Task 3: Add GET /api/library and PATCH /api/library/:id endpoints

**Files:**
- Modify: `src-tauri/src/ext_server.rs`

**Interfaces:**
- Consumes: `ExtState` from Task 1
- Produces: `GET /api/library` → `{ resources: LibraryEntry[] }`
- Produces: `PATCH /api/library/:id` → `{ ok: true }` or 404

- [ ] **Step 1: Add types**

Add to `ext_server.rs` after the `CreateLibraryResponse` struct:

```rust
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
```

- [ ] **Step 2: Add list_library_handler**

Add after `create_library_handler`:

```rust
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
            let resource_type: String = row.try_get("type").unwrap_or_else(|_| "webpage".to_string());
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
```

- [ ] **Step 3: Add update_library_handler**

Add after `list_library_handler`:

```rust
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
```

- [ ] **Step 4: Register the two routes**

Update the router in `ext_server.rs`:

```rust
let router = Router::new()
    .route("/api/jobs", post(create_job_handler))
    .route("/api/jobs/meta", get(meta_handler))
    .route("/api/jobs/:id", patch(update_job_handler))
    .route("/api/health", get(health_handler))
    .route("/api/library", post(create_library_handler))
    .route("/api/library", get(list_library_handler))       // ← add
    .route("/api/library/:id", patch(update_library_handler)) // ← add
    .layer(cors)
    .with_state(state);
```

- [ ] **Step 5: Add chrono import to ext_server.rs**

At the top of `ext_server.rs`, add:

```rust
use chrono;
```

- [ ] **Step 6: Verify cargo check**

```
cd src-tauri && cargo check 2>&1
```

Expected: zero errors.

- [ ] **Step 7: Commit**

```
git add src-tauri/src/ext_server.rs
git commit -m "feat(ext): add GET /api/library and PATCH /api/library/:id endpoints"
```

---

### Task 4: Add tab bar to popup — HTML structure and tab switching

**Files:**
- Modify: `extension/popup.html`
- Modify: `extension/popup.ts`

**Interfaces:**
- Produces: `switchTab(tab: 'job' | 'library'): void` — consumed by Tasks 5-7
- Produces: `#view-job` wrapper div containing all existing job views
- Produces: `#view-library` div (empty shell, populated by Tasks 5-7)

- [ ] **Step 1: Add tab bar CSS to popup.html**

Inside the `<style>` block in `popup.html`, add after the existing `.offline-box` rule:

```css
/* ── Tab bar ── */
#tab-bar {
  display: flex;
  gap: 2px;
  margin-bottom: 14px;
  border-bottom: 1px solid #2a2825;
  padding-bottom: 0;
}
.tab-btn {
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: #666;
  cursor: pointer;
  font-family: inherit;
  font-size: 13px;
  font-weight: 600;
  padding: 4px 12px 8px;
  transition: color 0.12s, border-color 0.12s;
  margin-bottom: -1px;
}
.tab-btn:hover { color: #cdccca; }
.tab-active { color: #4f98a3; border-bottom-color: #4f98a3; }
```

- [ ] **Step 2: Restructure popup.html body**

In `popup.html`, replace the `<body>` content so that:
- The `<header>` stays at the top (unchanged)
- A new `<div id="tab-bar">` comes next
- All existing job views (`#view-idle`, `#view-submitting`, `#view-success`, `#view-error`, `#view-offline`) are wrapped in `<div id="view-job">`
- A new `<div id="view-library">` follows as a sibling

The structure after `<header>...</header>` should be:

```html
  <!-- ── TAB BAR ── -->
  <div id="tab-bar">
    <button id="tab-job" class="tab-btn tab-active">Add Job</button>
    <button id="tab-library" class="tab-btn">Library</button>
  </div>

  <!-- ── JOB TAB ── -->
  <div id="view-job">

    <!-- ── IDLE ── -->
    <div id="view-idle">
      <!-- UNCHANGED — full existing content here -->
    </div>

    <!-- ── SUBMITTING ── -->
    <div id="view-submitting">
      <!-- UNCHANGED -->
    </div>

    <!-- ── SUCCESS ── -->
    <div id="view-success" style="display:none">
      <!-- UNCHANGED -->
    </div>

    <!-- ── ERROR ── -->
    <div id="view-error">
      <!-- UNCHANGED -->
    </div>

    <!-- ── OFFLINE ── -->
    <div id="view-offline">
      <!-- UNCHANGED -->
    </div>

  </div><!-- /#view-job -->

  <!-- ── LIBRARY TAB ── -->
  <div id="view-library" style="display:none">
    <!-- populated in Task 5 -->
  </div>
```

Important: the contents of each inner view div are **exactly unchanged** from the current file. Only add the outer wrapper and the sibling `#view-library`.

- [ ] **Step 3: Add library row and note editor CSS**

Add to the `<style>` block in `popup.html`:

```css
/* ── Library tab ── */
.lib-search-row {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 10px;
}
.lib-search-wrap { flex: 1; position: relative; }
.lib-search-input {
  width: 100%;
  background: #211f1d;
  border: 1px solid #2e2c2a;
  border-radius: 5px;
  color: #cdccca;
  font-size: 13px;
  padding: 7px 28px 7px 10px;
  outline: none;
  font-family: inherit;
  transition: border-color 0.12s;
}
.lib-search-input:focus { border-color: #4f98a3; }
.lib-search-clear {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  background: none;
  border: none;
  color: #555;
  cursor: pointer;
  font-size: 13px;
  padding: 0;
  display: none;
}
.lib-search-clear:hover { color: #cdccca; }
.lib-save-btn { flex-shrink: 0; width: auto; padding: 7px 12px; }
.lib-row {
  border-bottom: 1px solid #2a2825;
  padding: 10px 0;
  min-height: 52px;
  display: flex;
  align-items: flex-start;
  gap: 8px;
}
.lib-row-main { flex: 1; cursor: pointer; min-width: 0; }
.lib-row-title {
  font-weight: 600;
  color: #e8e6e3;
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.lib-row-main:hover .lib-row-title { color: #4f98a3; }
.lib-row-domain { font-size: 11px; color: #555; margin-top: 2px; }
.lib-row-tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
.lib-tag {
  background: #1a3438;
  color: #4f98a3;
  border-radius: 4px;
  font-size: 10px;
  padding: 1px 6px;
}
.lib-tag-more { color: #555; font-size: 10px; padding: 1px 4px; }
.lib-note-btn {
  background: none;
  border: none;
  color: #444;
  cursor: pointer;
  font-size: 15px;
  padding: 2px 4px;
  opacity: 0;
  transition: opacity 0.1s, color 0.1s;
  flex-shrink: 0;
  line-height: 1;
}
.lib-row:hover .lib-note-btn { opacity: 1; }
.lib-note-btn:hover { color: #4f98a3; }
.lib-note-editor { padding: 6px 0 2px; width: 100%; }
.lib-note-textarea {
  width: 100%;
  background: #211f1d;
  border: 1px solid #2e2c2a;
  border-radius: 5px;
  color: #cdccca;
  font-size: 12px;
  font-family: inherit;
  padding: 6px 8px;
  outline: none;
  resize: vertical;
  min-height: 48px;
}
.lib-note-textarea:focus { border-color: #4f98a3; }
.lib-offline-banner {
  background: #172428;
  border: 1px solid #274448;
  border-radius: 5px;
  padding: 8px 10px;
  color: #4f98a3;
  font-size: 11px;
  margin-bottom: 8px;
  line-height: 1.4;
  display: none;
}
.lib-status {
  font-size: 12px;
  font-weight: 600;
  margin-top: 6px;
  display: none;
}
```

- [ ] **Step 4: Add tab switching to popup.ts**

At the top of `popup.ts`, add after the existing `let currentJobId` and `ratings` declarations:

```typescript
let activeTab: 'job' | 'library' = 'job';

function switchTab(tab: 'job' | 'library'): void {
  activeTab = tab;
  el('tab-job').classList.toggle('tab-active', tab === 'job');
  el('tab-library').classList.toggle('tab-active', tab === 'library');
  el('view-job').style.display = tab === 'job' ? 'block' : 'none';
  el('view-library').style.display = tab === 'library' ? 'block' : 'none';
  if (tab === 'library') void loadLibraryBrowse();
}
```

- [ ] **Step 5: Wire tab buttons in the init IIFE**

Inside the existing `(async () => { ... })()` at the bottom of `popup.ts`, add at the end (before the closing `})();`):

```typescript
  el('tab-job').addEventListener('click', () => switchTab('job'));
  el('tab-library').addEventListener('click', () => switchTab('library'));
```

- [ ] **Step 6: Add loadLibraryBrowse stub**

Add a temporary stub so TypeScript compiles (will be replaced in Task 5):

```typescript
async function loadLibraryBrowse(): Promise<void> {
  // implemented in next task
}
```

- [ ] **Step 7: tsc check**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 8: Commit**

```
git add extension/popup.html extension/popup.ts
git commit -m "feat(ext): add tab bar and Library tab scaffold"
```

---

### Task 5: Library BROWSE state — list rendering and inline note editor

**Files:**
- Modify: `extension/popup.html` (add library HTML inside `#view-library`)
- Modify: `extension/popup.ts` (replace stub, add all BROWSE functions)

**Interfaces:**
- Consumes: `GET /api/library` from Task 3
- Consumes: `PATCH /api/library/:id` from Task 3
- Consumes: `checkHealth()`, `escHtml()`, `el()` — all existing in popup.ts
- Produces: `loadLibraryBrowse(): Promise<void>` — replaces stub from Task 4
- Produces: `fetchAndRenderLibrary(q?: string): Promise<void>`

- [ ] **Step 1: Add library HTML inside #view-library**

Replace the `<!-- populated in Task 5 -->` comment inside `#view-library` in `popup.html` with:

```html
    <!-- Search + Save row -->
    <div class="lib-search-row">
      <div class="lib-search-wrap">
        <input id="lib-search" type="text" class="lib-search-input" placeholder="Search library...">
        <button id="lib-search-clear" class="lib-search-clear">✕</button>
      </div>
      <button id="lib-save-page" class="btn btn-primary lib-save-btn">Save Page</button>
    </div>

    <!-- Inline save form (hidden by default) -->
    <div id="lib-save-form" style="display:none">
      <div class="field">
        <label for="lib-save-title">Title</label>
        <input id="lib-save-title" type="text" placeholder="Page title" autocomplete="off">
      </div>
      <div class="field">
        <label for="lib-save-note">Note (optional)</label>
        <textarea id="lib-save-note" style="min-height:56px" placeholder="Add a note..."></textarea>
      </div>
      <div style="display:flex; gap:8px; align-items:center; margin-top:8px">
        <button id="lib-save-submit" class="btn btn-primary" style="flex:1">Save</button>
        <a href="#" id="lib-save-cancel" class="text-link">Cancel</a>
      </div>
      <div id="lib-save-status" class="lib-status"></div>
    </div>

    <!-- Offline banner -->
    <div id="lib-offline-banner" class="lib-offline-banner">
      Yggdrasil is not running. Saved pages will sync when the app opens.
    </div>

    <!-- Resource list -->
    <div id="lib-list"></div>
    <div id="lib-empty" style="display:none; color:#555; font-size:12px; text-align:center; padding:24px 0">
      No saved resources yet. Browse the web and click Save Page.
    </div>
```

- [ ] **Step 2: Add types and helpers to popup.ts**

Add after the existing `interface FormDraft` block:

```typescript
interface LibraryEntry {
  id: string;
  title: string;
  url: string;
  type: string;
  tags: string[];
  user_notes: string | null;
  created_at: string;
}

interface LibraryQueuedItem {
  url: string;
  title: string;
  note: string;
  queued_at: string;
}

const LIBRARY_QUEUE_KEY = 'ygg_library_queue';

function extractDomain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

async function getLibraryQueue(): Promise<LibraryQueuedItem[]> {
  const data = await chrome.storage.local.get(LIBRARY_QUEUE_KEY);
  return (data[LIBRARY_QUEUE_KEY] as LibraryQueuedItem[]) ?? [];
}

async function saveLibraryQueue(items: LibraryQueuedItem[]): Promise<void> {
  await chrome.storage.local.set({ [LIBRARY_QUEUE_KEY]: items });
  const jobQ = await getQueue();
  const total = items.length + jobQ.length;
  await chrome.action.setBadgeText({ text: total > 0 ? String(total) : '' });
  if (total > 0) await chrome.action.setBadgeBackgroundColor({ color: '#4f98a3' });
}

async function patchLibraryNote(id: string, note: string): Promise<void> {
  await fetch(`${API_BASE}/api/library/${id}`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
    body: JSON.stringify({ note }),
  });
}
```

- [ ] **Step 3: Add renderLibraryRow and renderLibraryList**

Add after the helpers above:

```typescript
function renderLibraryRow(r: LibraryEntry): HTMLElement {
  const row = document.createElement('div');
  row.className = 'lib-row';

  const domain = extractDomain(r.url);
  const displayTags = r.tags.slice(0, 3);
  const extraTags = r.tags.length > 3 ? r.tags.length - 3 : 0;

  const main = document.createElement('div');
  main.className = 'lib-row-main';
  main.innerHTML = `
    <div class="lib-row-title">${escHtml(r.title)}</div>
    <div class="lib-row-domain">${escHtml(domain)}</div>
    ${r.tags.length > 0 ? `<div class="lib-row-tags">
      ${displayTags.map((t) => `<span class="lib-tag">${escHtml(t)}</span>`).join('')}
      ${extraTags > 0 ? `<span class="lib-tag-more">+${extraTags} more</span>` : ''}
    </div>` : ''}
  `;
  main.addEventListener('click', () => { void chrome.tabs.create({ url: r.url }); });

  const noteBtn = document.createElement('button');
  noteBtn.className = 'lib-note-btn';
  noteBtn.title = 'Edit note';
  noteBtn.textContent = '✎';

  const noteEditor = document.createElement('div');
  noteEditor.style.cssText = 'display:none; flex-direction:column; width:100%; padding:6px 0 2px';

  const noteTA = document.createElement('textarea');
  noteTA.className = 'lib-note-textarea';
  noteTA.value = r.user_notes ?? '';

  const noteActions = document.createElement('div');
  noteActions.style.cssText = 'display:flex; gap:6px; margin-top:4px; align-items:center';

  const noteSave = document.createElement('button');
  noteSave.className = 'btn btn-primary';
  noteSave.style.cssText = 'font-size:11px; padding:4px 10px; width:auto';
  noteSave.textContent = 'Save';

  const noteCancel = document.createElement('a');
  noteCancel.href = '#';
  noteCancel.className = 'text-link';
  noteCancel.textContent = 'Cancel';

  noteActions.appendChild(noteSave);
  noteActions.appendChild(noteCancel);
  noteEditor.appendChild(noteTA);
  noteEditor.appendChild(noteActions);

  noteBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const open = noteEditor.style.display !== 'none';
    noteEditor.style.display = open ? 'none' : 'flex';
    if (!open) noteTA.focus();
  });

  noteSave.addEventListener('click', () => {
    void patchLibraryNote(r.id, noteTA.value);
    noteEditor.style.display = 'none';
  });

  noteCancel.addEventListener('click', (e) => {
    e.preventDefault();
    noteEditor.style.display = 'none';
  });

  // Outer wrapper so note editor spans full width below the row
  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'display:flex; flex-direction:column; flex:1; min-width:0';
  wrapper.appendChild(main);
  wrapper.appendChild(noteEditor);

  row.appendChild(wrapper);
  // Only show note editing for persisted resources (not locally-queued offline items)
  if (r.id) row.appendChild(noteBtn);
  return row;
}

function renderLibraryList(resources: LibraryEntry[]): void {
  const listEl = el('lib-list');
  listEl.innerHTML = '';
  el('lib-empty').style.display = resources.length === 0 ? 'block' : 'none';
  resources.forEach((r) => listEl.appendChild(renderLibraryRow(r)));
}
```

- [ ] **Step 4: Replace the loadLibraryBrowse stub**

Replace the stub from Task 4 with:

```typescript
async function loadLibraryBrowse(): Promise<void> {
  el('lib-offline-banner').style.display = 'none';
  await fetchAndRenderLibrary();
}

async function fetchAndRenderLibrary(q?: string): Promise<void> {
  const params = new URLSearchParams({ limit: '20' });
  if (q) params.set('q', q);

  try {
    const res = await fetch(`${API_BASE}/api/library?${params.toString()}`, {
      headers: { 'X-Ygg-Key': API_KEY },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = (await res.json()) as { resources: LibraryEntry[] };
    renderLibraryList(data.resources);
  } catch {
    // offline — show queued items
    if (!q) {
      const queued = await getLibraryQueue();
      const fakeEntries: LibraryEntry[] = queued.map((item) => ({
        id: '',
        title: item.title || item.url,
        url: item.url,
        type: 'webpage',
        tags: [],
        user_notes: item.note || null,
        created_at: item.queued_at,
      }));
      renderLibraryList(fakeEntries);
      el('lib-offline-banner').style.display = 'block';
    }
  }
}
```

- [ ] **Step 5: tsc check**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 6: Commit**

```
git add extension/popup.html extension/popup.ts
git commit -m "feat(ext): Library BROWSE state with resource list and inline note editor"
```

---

### Task 6: Library SAVING state

**Files:**
- Modify: `extension/popup.ts`

**Interfaces:**
- Consumes: `POST /api/library` from Task 2
- Consumes: `checkHealth()`, `getLibraryQueue()`, `saveLibraryQueue()` — defined in Task 5
- Consumes: `LibraryQueuedItem` — defined in Task 5
- Consumes: `fetchAndRenderLibrary()` — defined in Task 5

- [ ] **Step 1: Add save form state helpers**

Add to `popup.ts` after `fetchAndRenderLibrary`:

```typescript
function showSaveForm(visible: boolean): void {
  el('lib-save-form').style.display = visible ? 'block' : 'none';
  el('lib-save-page').style.display = visible ? 'none' : 'inline-block';
  if (!visible) {
    (el('lib-save-title') as HTMLInputElement).value = '';
    (el('lib-save-note') as HTMLTextAreaElement).value = '';
    el('lib-save-status').style.display = 'none';
    el('lib-save-status').textContent = '';
    (el<HTMLButtonElement>('lib-save-submit')).disabled = false;
    (el<HTMLButtonElement>('lib-save-submit')).textContent = 'Save';
  }
}

function showSaveStatus(msg: string, color: string): void {
  const s = el('lib-save-status');
  s.textContent = msg;
  s.style.color = color;
  s.style.display = 'block';
}
```

- [ ] **Step 2: Add handleSavePage**

```typescript
async function handleSavePage(): Promise<void> {
  showSaveForm(true);
  // Pre-fill from tab.title — GET_PAGE_CONTEXT is job-oriented, not useful here
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.url && !tab.url.startsWith('chrome://')) {
      (el('lib-save-title') as HTMLInputElement).value = tab.title ?? tab.url;
    }
  } catch { /* activeTab unavailable */ }
}
```

- [ ] **Step 3: Add handleLibrarySave**

```typescript
async function handleLibrarySave(): Promise<void> {
  const btn = el<HTMLButtonElement>('lib-save-submit');
  btn.disabled = true;
  btn.textContent = '…';

  let tab: chrome.tabs.Tab | undefined;
  try {
    [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  } catch { /* noop */ }

  const url = tab?.url ?? '';
  if (!url || url.startsWith('chrome://')) {
    btn.disabled = false;
    btn.textContent = 'Save';
    showSaveStatus('Cannot save this page.', '#e05252');
    return;
  }

  const title = (el('lib-save-title') as HTMLInputElement).value.trim();
  const note = (el('lib-save-note') as HTMLTextAreaElement).value.trim();

  const online = await checkHealth();
  if (!online) {
    const queue = await getLibraryQueue();
    queue.push({ url, title, note, queued_at: new Date().toISOString() });
    await saveLibraryQueue(queue);
    showSaveStatus('Queued — syncs when Yggdrasil opens', '#4f98a3');
    setTimeout(() => {
      showSaveForm(false);
      void fetchAndRenderLibrary();
      el('lib-offline-banner').style.display = 'block';
    }, 1000);
    return;
  }

  try {
    const res = await fetch(`${API_BASE}/api/library`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
      body: JSON.stringify({ url, title, note }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
      throw new Error((data as { error?: string }).error ?? `HTTP ${res.status}`);
    }
    showSaveStatus('✓ Saved', '#4caf7d');
    setTimeout(() => {
      showSaveForm(false);
      void fetchAndRenderLibrary();
    }, 700);
  } catch (err) {
    btn.disabled = false;
    btn.textContent = 'Save';
    showSaveStatus(err instanceof Error ? err.message : 'Save failed', '#e05252');
  }
}
```

- [ ] **Step 4: Wire save form event listeners**

Inside the init IIFE, add after the tab button listeners (from Task 4):

```typescript
  el('lib-save-page').addEventListener('click', () => void handleSavePage());
  el('lib-save-submit').addEventListener('click', () => void handleLibrarySave());
  el('lib-save-cancel').addEventListener('click', (e) => {
    e.preventDefault();
    showSaveForm(false);
  });
```

- [ ] **Step 5: tsc check**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 6: Commit**

```
git add extension/popup.ts
git commit -m "feat(ext): Library SAVING state with offline queue support"
```

---

### Task 7: Library SEARCH state

**Files:**
- Modify: `extension/popup.ts`

**Interfaces:**
- Consumes: `fetchAndRenderLibrary(q)` from Task 5
- Consumes: `loadLibraryBrowse()` from Task 5

- [ ] **Step 1: Add search handler**

Add to `popup.ts` after `handleLibrarySave`:

```typescript
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

function handleLibrarySearch(q: string): void {
  el('lib-search-clear').style.display = q ? 'inline-block' : 'none';
  if (searchDebounceTimer !== null) clearTimeout(searchDebounceTimer);
  if (!q.trim()) {
    void loadLibraryBrowse();
    return;
  }
  searchDebounceTimer = setTimeout(() => void fetchAndRenderLibrary(q.trim()), 300);
}
```

- [ ] **Step 2: Wire search input and clear button**

Inside the init IIFE, after the save form listeners:

```typescript
  el<HTMLInputElement>('lib-search').addEventListener('input', (e) => {
    handleLibrarySearch((e.target as HTMLInputElement).value);
  });
  el('lib-search-clear').addEventListener('click', () => {
    (el('lib-search') as HTMLInputElement).value = '';
    handleLibrarySearch('');
    (el('lib-search') as HTMLInputElement).focus();
  });
```

- [ ] **Step 3: tsc check**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 4: Commit**

```
git add extension/popup.ts
git commit -m "feat(ext): Library SEARCH state with debounced query and clear button"
```

---

### Task 8: Fix background.ts — QueuedJob type, library queue flush, combined badge

**Files:**
- Modify: `extension/background.ts`

**Interfaces:**
- Consumes: `POST /api/library` from Task 2
- Produces: updated `QueuedJob` with `source` field
- Produces: `flushLibraryQueue()` — flushes `ygg_library_queue` items on alarm

- [ ] **Step 1: Fix QueuedJob and add LibraryQueuedItem**

In `background.ts`, replace the existing `QueuedJob` interface and add `LibraryQueuedItem`:

```typescript
interface QueuedJob {
  company: string;
  position: string;
  location: string;
  source: string;          // ← was missing
  link: string;
  season: string;
  jobDescription: string;
  queuedAt: string;
}

interface LibraryQueuedItem {
  url: string;
  title: string;
  note: string;
  queued_at: string;
}

const LIBRARY_QUEUE_KEY = 'ygg_library_queue';
```

- [ ] **Step 2: Add flushLibraryQueue**

Add after the existing `flushQueue` function in `background.ts`:

```typescript
async function flushLibraryQueue(): Promise<void> {
  const data = await chrome.storage.local.get(LIBRARY_QUEUE_KEY);
  const queue: LibraryQueuedItem[] = (data[LIBRARY_QUEUE_KEY] as LibraryQueuedItem[]) ?? [];
  if (queue.length === 0) return;

  const remaining: LibraryQueuedItem[] = [];
  for (const item of queue) {
    try {
      const res = await fetch(`${API_BASE}/api/library`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
        body: JSON.stringify(item),
      });
      if (!res.ok) remaining.push(item);
    } catch {
      remaining.push(item);
    }
  }
  await chrome.storage.local.set({ [LIBRARY_QUEUE_KEY]: remaining });
}

async function updateBadge(): Promise<void> {
  const [jobData, libData] = await Promise.all([
    chrome.storage.local.get(QUEUE_KEY),
    chrome.storage.local.get(LIBRARY_QUEUE_KEY),
  ]);
  const total =
    ((jobData[QUEUE_KEY] as unknown[]) ?? []).length +
    ((libData[LIBRARY_QUEUE_KEY] as unknown[]) ?? []).length;
  await chrome.action.setBadgeText({ text: total > 0 ? String(total) : '' });
  if (total > 0) await chrome.action.setBadgeBackgroundColor({ color: '#4f98a3' });
}
```

- [ ] **Step 3: Update the alarm handler**

Replace the existing `chrome.alarms.onAlarm.addListener` block in `background.ts` with:

```typescript
chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name !== 'flush-queue') return;

  try {
    const res = await fetch(`${API_BASE}/api/health`, {
      signal: AbortSignal.timeout(3000),
    });
    if (!res.ok) return;
  } catch {
    return;
  }

  await Promise.all([flushQueue(), flushLibraryQueue()]);
  await updateBadge();
});
```

- [ ] **Step 4: Update flushQueue to use updateBadge**

In `background.ts`, at the end of the existing `flushQueue` function, replace the badge-update lines with a call to `updateBadge()`:

```typescript
  await chrome.storage.local.set({ [QUEUE_KEY]: remaining });
  await updateBadge();
```

(Remove the inline `setBadgeText`/`setBadgeBackgroundColor` calls that were at the bottom of `flushQueue`.)

- [ ] **Step 5: tsc check**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 6: Commit**

```
git add extension/background.ts
git commit -m "fix(ext): add source to QueuedJob, add library queue flush and combined badge"
```

---

### Task 9: Update CLAUDE.md and verify full build

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add library endpoints to CLAUDE.md**

In `CLAUDE.md`, find the Browser Extension section's route table under `ext_server.rs`. Add the three new routes:

```markdown
| `POST /api/library` | Create resource + fire ingest pipeline in background |
| `GET /api/library` | List/search resources (`?q=`, `?limit=`) |
| `PATCH /api/library/:id` | Update user_notes on a resource |
```

- [ ] **Step 2: cargo check**

```
cd src-tauri && cargo check 2>&1
```

Expected: same 5 pre-existing dead-code warnings, zero errors.

- [ ] **Step 3: tsc --noEmit (root)**

```
npx tsc --noEmit 2>&1
```

Expected: zero output (clean).

- [ ] **Step 4: tsc --noEmit (extension)**

```
cd extension && npx tsc --noEmit 2>&1
```

Expected: zero errors.

- [ ] **Step 5: Build extension**

```
cd extension && npm run build 2>&1
```

Expected: `tsc` completes with no output. Verify `dist/popup.js`, `dist/content.js`, `dist/background.js` all have updated timestamps.

- [ ] **Step 6: Manual smoke test checklist**

With the app running (`bash dev.sh`):
- Load extension in Chrome (`chrome://extensions` → Load unpacked → `extension/` dir)
- Open any webpage → click Library tab → click "Save Page" → title pre-fills → click Save → "✓ Saved" flashes → resource appears in list
- Search for a word from a saved resource's title → list filters
- Clear search → full list returns
- Hover a resource row → note icon appears → click → textarea opens → type a note → Save → editor closes
- Visit same URL again → Save Page → should see immediate return without duplicate
- Stop the Tauri app → open extension → Library tab → Save Page → "Queued — syncs when Yggdrasil opens" → restart app → within 5 min alarm fires → resource appears in list, badge clears

- [ ] **Step 7: Commit**

```
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with library extension endpoints"
```
