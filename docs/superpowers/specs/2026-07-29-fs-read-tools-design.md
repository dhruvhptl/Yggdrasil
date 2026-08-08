# Filesystem Read Tools + Path Allowlist (Agent Step 3) — Design

**Date:** 2026-07-29
**Status:** Design approved — ready for implementation plan
**Depends on:** the Mimir tool-calling agent (mimir_agent.rs), `tool_schemas` gating, and (for the scan retro-gate) Agent Step 2's background `scan_project`.

---

## Goal

Give the Mimir agent **bounded read access** to source files so it can actually understand a project (read a README, config, or a source file) instead of only seeing tree-sitter-extracted names. Access is confined to folders the user explicitly registers (a **global allowlist**), with a security model that defends against the real threat: a prompt injection (from a fetched web page or a scanned file) telling the agent to read `.env` and exfiltrate it via `smart_fetch`.

Two new agent tools — `read_file` and `list_project_files` — plus a global `project_roots` registry managed from a new Settings page. Every file access (including the existing `scan_project`) flows through one `resolve_safe_path` gate.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| Scope | **Global allowlist** (`project_roots`, no `tree_id`) | Single-user local app; register once, read from any chat; matches how `scan_project` already takes a path. |
| Registration UI | **New dedicated Settings page** (gear nav item), first section "Project Folders" | Gives future settings a home; barely more work than a panel. |
| Listing depth | **One level** (name + size), recursive/glob deferred | Simpler + safer for v1; agent navigates by listing subdirs. |
| `scan_project` | **Retro-gated** through `resolve_safe_path` | The agent must not be talkable into scanning arbitrary paths. |
| Extension scan path | **Left ungated** | User-initiated from the browser (not an injection surface); already behind `X-Ygg-Key`. |

**Behavior change to communicate:** after this step, the agent can only `read_file` / `list_project_files` / `scan_project` under a *registered* root. An arbitrary `scan my project at C:\...\foo` is rejected until `foo` is added in Settings. This is the intended posture.

---

## Grounding (verified against current code 2026-07-29 — AUTHORITATIVE)

1. **No `project_roots` table exists** — `local_path` is only transient in the ext bridge. This step creates the registry. **Highest migration is 053** (Step 1) → this adds **054**.
2. **`tool_schemas(hound_available: bool)`** (mimir_agent.rs:38) is called once at :721 as `tool_schemas(ctx.hound_base_url.is_some())` and unit-tested at :837-849. → add a 2nd param `fs_tools_available: bool`; update the call site and the two test calls (`tool_schemas(false)`→`tool_schemas(false, false)`, etc.).
3. **`ToolCtx`** is built in `mimir_chat` (mimir_retrieval.rs ~:1180) — gains `project_roots: Vec<ProjectRoot>`, loaded from the DB just before the build. `fs_tools_available = !ctx.project_roots.is_empty()`.
4. **The agent system prompt** is assembled in `mimir_chat` (~:1150, `agent_system_prompt`) before the run — inject the registered-folders line there, only when roots exist.
5. **`scan_project` arm** now enqueues a background job (Step 2, mimir_agent.rs ~:663) — the retro-gate adds a `resolve_safe_path` check before the `queue.send`.
6. **Frontend:** routes are `<Route path="/x" element={<XPage/>}/>` (App.tsx:22-31); nav is `<SidebarLink to="/x" icon={<Icon/>} label="…"/>` with lucide icons (MainLayout.tsx:37-45). Adding a Settings page is one route + one nav link + the page component.
7. **`truncate_chars`** (text_util.rs) is the mandated char-safe truncation for the `read_file` cap.

---

## The security core: `resolve_safe_path`

```rust
pub(crate) struct ProjectRoot { pub id: String, pub label: String, pub path: String }

/// Resolve a requested path to a canonical PathBuf ONLY if it is a real path,
/// not secret-adjacent, and inside a registered root. Any failure → Err (never
/// returns an unsafe path).
pub(crate) fn resolve_safe_path(requested: &str, roots: &[ProjectRoot]) -> Result<std::path::PathBuf, String>;
```
Algorithm (order is load-bearing):
1. **`std::fs::canonicalize(requested)`** — normalizes `..` **and resolves symlinks to their real target**. Fails if the path doesn't exist → `Err("path not found")`.
2. **Denylist** on the canonical path: reject if any path component is `.git` / `.ssh` / `.aws` / `.gnupg`, or the file name matches (case-insensitive) `.env` / `.env.*` / `*.pem` / `*.key` / `id_*` / `credentials*` / `.git-credentials` / `.npmrc`.
3. **Allowlist**: canonicalize each root's `path`; require the canonical requested path to `starts_with` at least one canonical root. Else `Err("outside allowed project folders")`.

Canonicalize-before-allowlist is what closes the symlink-escape hole: a symlink inside an allowed root pointing at `~/.ssh` canonicalizes to the real `~/.ssh`, which is not under any root → rejected.

**Two implementation details that are security-relevant (not optional):**
- The allowlist check MUST use `std::path::Path::starts_with` (component-wise), **not** a string prefix. String prefix would let `C:\proj-evil` match a root `C:\proj`; `Path::starts_with` compares whole path components and doesn't.
- On Windows, `std::fs::canonicalize` returns the extended-length `\\?\C:\…` (verbatim) prefix. Because **both** the requested path and the roots are canonicalized, the `starts_with` comparison stays consistent (prefix on both sides). `add_project_root` stores the canonical path, so it may carry the `\\?\` prefix — the Settings UI may strip it for display, but the stored/compared value must remain the canonical one.

---

## Components

### 1. Migration 054 (`migrations/054_project_roots.sql`)
```sql
CREATE TABLE IF NOT EXISTS project_roots (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    path       TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 2. `project_roots.rs` (new module)
- `resolve_safe_path` (above) + `ProjectRoot`.
- `get_project_roots(pool) -> Vec<ProjectRoot>`; `add_project_root(pool, label, path)` (canonicalizes, confirms it exists + is a directory + not denylisted, stores the **canonical** path); `remove_project_root(pool, id)`.
- Tauri commands `get_project_roots_cmd` / `add_project_root_cmd` / `remove_project_root_cmd` (all `#[serde(rename_all="camelCase")]`, `database` last). Register in `main.rs`.

### 3. Agent tools (`mimir_agent.rs`)
- `tool_schemas(hound_available, fs_tools_available)` — when `fs_tools_available`, append:
  - `read_file { path }` — reads a text file, **100 KB cap** via `truncate_chars`; path via `resolve_safe_path`.
  - `list_project_files { path }` — one-level directory listing (name + size), cap ~200 entries, skip `node_modules`/`target`/`.venv`/`.git`; path via `resolve_safe_path`.
- `execute_tool` arms for both, each calling `resolve_safe_path(path, &ctx.project_roots)` first and returning the `Err` string as the observation on rejection.
- `scan_project` arm: run the path through `resolve_safe_path` before `queue.send` (retro-gate).
- Update the tool-count unit test for the new arity/tools.

### 4. Agent context (`mimir_retrieval.rs`)
- Load `project_roots` before the `ToolCtx` build; set `ToolCtx.project_roots`.
- When non-empty, append to `agent_system_prompt`: `"You may read files in these registered project folders (use absolute paths under them): <label → path>, …. Only these folders are readable."`

### 5. Settings page (frontend)
- `src/pages/SettingsPage.tsx` — a "Project Folders" section: list roots (label + path), add (label + path inputs → `add_project_root_cmd`, surfacing its validation error), remove (`remove_project_root_cmd`). `invoke<ProjectRoot[]>("get_project_roots_cmd")` on load.
- Route `/settings` in `App.tsx`; `<SidebarLink to="/settings" icon={<Settings/>} label="Settings"/>` in `MainLayout.tsx` (lucide `Settings` icon).

---

## What this defends against
Threat: a prompt injection in fetched/scanned content instructing the agent to read a secret and exfiltrate it. Defenses: reads are bounded to user-chosen folders (allowlist); secrets are blocked even inside them (denylist); symlink escapes are neutralized (canonicalize-first); there is no code path where the agent reads outside a registered root. **Residual (accepted):** within an allowed root, `read_file` + `smart_fetch` in one turn is still a channel — but the blast radius is only registered project dirs, never system/secret paths.

---

## Error handling
Path not found / outside roots / denylisted → the tool returns the `Err` string as its observation; the agent relays a refusal, no crash. `read_file` on a directory → error. Non-UTF-8/binary file → lossy read, capped (v1). `add_project_root` on a non-existent or non-directory path → command returns an error the Settings UI shows. Empty registry → FS tools simply aren't offered (`fs_tools_available=false`).

---

## Testing
- **Rust (TDD):** `resolve_safe_path` against real temp dirs — allow a file inside a root; reject a path outside all roots; reject `.env` / a `.ssh` component inside a root; reject a non-existent path. (Symlink-escape assertion where the platform permits symlink creation in tests; otherwise documented as runtime-verified.) The denylist matcher as a pure helper.
- **Rust:** `tool_schemas(_, true)` includes `read_file`/`list_project_files`; `tool_schemas(_, false)` omits them. Update the existing gating test to the new arity.
- **Runtime checklist (`bash dev.sh`):** register a root in Settings; agent `list_project_files` + `read_file` under it work; a path outside / a `.env` request is refused; `scan_project` on an unregistered path is refused, on a registered one runs; removing the root turns the FS tools off.

---

## Files
**Create:** `migrations/054_project_roots.sql`; `src-tauri/src/project_roots.rs`; `src/pages/SettingsPage.tsx`.
**Modify:** `src-tauri/src/main.rs` (`mod project_roots;` + 3 commands); `src-tauri/src/mimir_agent.rs` (`tool_schemas` arity + 2 tools + 2 arms + scan retro-gate + test); `src-tauri/src/mimir_retrieval.rs` (load roots → `ToolCtx.project_roots` + system-prompt line); `src/App.tsx` (route); `src/layouts/MainLayout.tsx` (nav link).

---

## Build order (for the plan)
1. Migration 054 + `project_roots.rs` (`resolve_safe_path` TDD + CRUD) + 3 commands + `main.rs`.
2. Agent integration: `tool_schemas` arity + `read_file`/`list_project_files` arms + `ToolCtx.project_roots` load + system-prompt line + `scan_project` retro-gate + tests.
3. Settings page + route + nav.
4. Live verification.

---

## Explicitly out of scope
Recursive/glob listing; a `grep` tool; binary-file handling beyond lossy read; gating the extension's scan path; per-tree scoping; editing/writing files (read-only only).
