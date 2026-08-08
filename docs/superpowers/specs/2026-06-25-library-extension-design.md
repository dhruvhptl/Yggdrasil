# Library Extension Design

**Date:** 2026-06-25
**Status:** Approved

## Goal

Add a Library tab to the Chrome extension so users can save any webpage to Mimir with one click, browse saved resources, and search them — replacing Chrome bookmarks for learning resources.

---

## Backend: ext_server.rs

### ExtState change

`ext_server::start` gains two new parameters:

```rust
pub async fn start(
    pool: PgPool,
    client: reqwest::Client,
    api_key: String,
    app: AppHandle,
    queue: JobQueue,
)
```

`main.rs` passes them:

```rust
let queue_clone = app_handle.state::<JobQueue>().inner().clone();
tokio::spawn(ext_server::start(pool, http_client, ext_key, app_handle.clone(), queue_clone));
```

`ExtState` struct gains `app: AppHandle` and `queue: JobQueue`.

### New routes

```
POST  /api/library       → create_library_handler
GET   /api/library       → list_library_handler
PATCH /api/library/:id   → update_library_handler
```

### POST /api/library

Request body (camelCase JSON):
```json
{ "url": "string", "title": "string (optional)", "note": "string (optional)" }
```

Flow:
1. Auth: `X-Ygg-Key` header
2. Dedup: `SELECT id, title FROM mimir_resources WHERE url = $1` — if found, return `200 { id, title, type, already_existed: true }`
3. Infer type from URL:
   - `youtube.com` / `youtu.be` → `"video"`
   - `arxiv.org` / ends in `.pdf` → `"paper"`
   - `github.com` → `"repo"`
   - else → `"webpage"`
4. Resolve title: use provided title if non-empty; else `GET url` + parse `<title>` tag (30s timeout, fallback to hostname)
5. `INSERT INTO mimir_resources (id, title, url, type, status='read', user_notes)`
6. Return `200 { id, title, type, already_existed: false }` immediately
7. `tokio::spawn`: POST `{SCRAPER_URL}/fetch` → Rust reqwest fallback if scraper unreachable → `store_chunks_and_embeddings` → `on_resource_ingested_async` (auto-tag + MatchResourceToNodes + ExtractSkillsFromResource + ygg-resource-ingested emit)

Error paths: 400 if URL empty, 500 on DB errors.

### GET /api/library

Query params: `q` (optional search term), `limit` (optional, default 50).

With `q`:
```sql
SELECT id, title, url, type, tags, user_notes, created_at
FROM mimir_resources
WHERE url IS NOT NULL
  AND (title ILIKE '%q%' OR url ILIKE '%q%' OR $q = ANY(tags))
ORDER BY created_at DESC
LIMIT $limit
```

Without `q`: same columns, no WHERE filter on content, ORDER BY created_at DESC LIMIT $limit.

Response: `{ resources: [{ id, title, url, type, tags, user_notes, created_at }] }`

### PATCH /api/library/:id

Request body: `{ "note": "string" }`

```sql
UPDATE mimir_resources SET user_notes = $1, updated_at = NOW() WHERE id = $2
```

Returns `404` if `rows_affected == 0`, `200 { ok: true }` on success.

---

## Extension: popup.ts / popup.html

### Tab bar

Two pill tabs at the top of the popup: **Add Job** (existing) and **Library** (new). Active tab has a teal bottom border. Tab selection is an in-memory JS variable — defaults to "Add Job" on open. Switching tabs shows/hides the corresponding view div; job form draft state is unaffected.

### Library tab state machine

Three sub-states controlled by a JS variable (`libraryState: 'browse' | 'saving' | 'search'`):

#### BROWSE (default on tab open)

- Loads on tab switch: `GET /api/library?limit=20`
- Top row: search input (full width) + "Save Page" button
- Scrollable list of resource rows:
  - Title (bold, 1-line truncation) + domain extracted from URL (muted)
  - Tags: up to 3 teal pills, "+N more" if >3
  - Click row → `chrome.tabs.create({ url })`
  - Hover → note icon appears; click opens inline note editor
- Empty state: "No saved resources yet."
- Offline: show items from `ygg_library_queue` with a "queued" indicator; show "Yggdrasil is not running" banner

#### SAVING (triggered by "Save Page" button)

- Inline mini-form replaces list (does not use the full-page views):
  - Title field: pre-filled from content script `GET_PAGE_CONTEXT` → fallback to `tab.title` → fallback to `tab.url`
  - Note field: optional textarea
  - "Save" button + "Cancel" link
- On Save (online): `POST /api/library { url: tab.url, title, note }` → flash "✓ Saved" → refresh list → return to BROWSE
- On Save (offline): push `{ url, title, note, queued_at }` to `ygg_library_queue` → show "Queued" → return to BROWSE

#### SEARCH (triggered when user types in search input)

- Debounced 300ms: `GET /api/library?q=term`
- Results replace browse list using the same row format
- ✕ button on search input clears the field and returns to BROWSE

#### Inline note editor

- Appears within the resource row on note icon click
- `<textarea>` pre-filled with `user_notes`
- "Save" → `PATCH /api/library/:id { note }` → collapse editor, update displayed note in DOM
- "Cancel" → collapse without saving

### Offline library queue

- `chrome.storage.local` key: `ygg_library_queue`
- Entry shape: `{ url: string, title: string, note: string, queued_at: string }`
- Background alarm (`flush-queue`, every 5 min) flushes both `ygg_job_queue` and `ygg_library_queue`
- Badge count = `ygg_job_queue.length + ygg_library_queue.length`

### Styling (within existing palette)

- Tab bar: `display: flex; gap: 4px; margin-bottom: 14px`. Each tab: pill button, `background: transparent`, active state: `color: #4f98a3; border-bottom: 2px solid #4f98a3`
- Resource rows: `border-bottom: 1px solid #2a2825; padding: 10px 0; min-height: 52px; cursor: pointer`
- Domain text: `font-size: 11px; color: #555`
- Tag pills: same chip style as job form but `font-size: 10px; padding: 1px 6px`
- Note icon: `color: #444`, `color: #4f98a3` on hover
- No new fonts or colors

---

## Files changed

| File | Change |
|---|---|
| `src-tauri/src/ext_server.rs` | Add `AppHandle` + `JobQueue` to `ExtState`; 3 new routes + handlers; title-fetch helper |
| `src-tauri/src/main.rs` | Pass `app_handle` + `queue` to `ext_server::start` |
| `extension/popup.ts` | Tab switching, Library state machine, all Library handlers |
| `extension/popup.html` | Tab bar markup + library tab HTML |
| `extension/background.ts` | Fix `QueuedJob` interface (add `source`); flush `ygg_library_queue` in alarm |
| `CLAUDE.md` | Add 3 library endpoints to Browser Extension section |

No new migrations. `mimir_resources.user_notes TEXT` exists since migration 003. `store_chunks_and_embeddings` and `on_resource_ingested_async` are already `pub(crate)` / `pub`.

---

## Verification checklist

- [ ] Cargo check passes with no new errors
- [ ] `tsc --noEmit` passes in both root and `extension/`
- [ ] On any webpage: Library tab → Save Page → title pre-filled → Save → appears in list
- [ ] Search "neural" → filters to matching titles/tags
- [ ] Duplicate URL → returns `already_existed: true`, no duplicate in list
- [ ] Save while app offline → queued, badge increments → open app → alarm flushes → badge clears
- [ ] Note on a resource → PATCH fires → note persists on next open
- [ ] Switching between Add Job and Library tabs preserves job form draft
