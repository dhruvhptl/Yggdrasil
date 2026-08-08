# Settings Overhaul (Model Selector / Folder Picker / Project Label) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Three Settings additions: (1) an agent-model dropdown (validated OpenRouter list) persisted to a `settings` table so the user swaps models without touching env vars; (2) a native "Browse…" folder picker beside the manual path input; (3) the folder-label field becomes a `<select>` of existing project names.

**Architecture:** A generic key-value `settings` table + `settings.rs` (get/set + 2 commands). `AgentModelConfig::from_env` becomes `async fn from_env(pool)` resolving **DB setting > env var > hardcoded default**. A static, live-validated model list lives in `src/lib/openrouter-models.ts`; the id→name display resolution happens in the **frontend** (not Rust). `tauri-plugin-dialog` provides the folder picker.

**Tech Stack:** Rust (Tauri 2, sqlx), React 19 + TS, Postgres.

## Global Constraints (corrections to the design baked in)

- **Migration number is `055`** (highest on disk is `054_project_roots.sql`) — NOT 056.
- **id→name resolution is frontend-only** — `get_agent_config_cmd` returns just the stored model id (`""` when unset); the frontend maps id→display name via `openrouter-models.ts`. Do NOT duplicate the model list into Rust.
- Rust: `$N` placeholders; non-macro `sqlx::query()`; commands `#[serde(rename_all="camelCase")]` where they return structs; `database: State<'_, Database>` last.
- Do NOT touch `RetrievalConfig::from_env()` (mimir_retrieval.rs:28/408/979) — only `AgentModelConfig::from_env()` (mimir_agent.rs:23, single call site at mimir_retrieval.rs:1190) changes.
- base_url / api_key stay from env (OpenRouter, via dev.sh) — never stored in DB.
- Work on branch `fix-compilation-errors`; commit locally, do NOT push.
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`

## File Structure

| File | Change | Task |
|---|---|---|
| `src-tauri/migrations/055_settings.sql` | Create key-value `settings` table | 1 |
| `src-tauri/src/settings.rs` | Create — get/set + 2 commands | 1 |
| `src-tauri/src/main.rs` | `mod settings;` + 2 commands + `.plugin(dialog)` | 1, 3 |
| `src-tauri/src/mimir_agent.rs` | `from_env` → `async fn from_env(pool)` + DB check | 2 |
| `src-tauri/src/mimir_retrieval.rs` | `.await` the new `from_env(&database.pool)` | 2 |
| `src-tauri/Cargo.toml` | `+ tauri-plugin-dialog = "2"` | 3 |
| `package.json` | `+ @tauri-apps/plugin-dialog` | 3 |
| `src-tauri/capabilities/default.json` | `+ "dialog:allow-open"` | 3 |
| `src/lib/openrouter-models.ts` | Create — 25 validated models | 4 |
| `src/pages/SettingsPage.tsx` | Full rewrite (3 sections) | 4 |

---

### Task 1: Settings store (table + module + commands)

**Files:**
- Create: `src-tauri/migrations/055_settings.sql`, `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/main.rs` (`mod settings;` ~:38; 2 commands in `generate_handler!`)

**Interfaces:**
- Produces: `settings::get_setting(pool, key) -> Option<String>`; `set_setting(pool, key, value) -> Result<(),String>`; commands `get_agent_config_cmd() -> String` (stored model id or `""`); `set_agent_config_cmd(model: String)` (empty ⇒ delete row).

- [ ] **Step 1: Migration 055**

`src-tauri/migrations/055_settings.sql`:
```sql
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

- [ ] **Step 2: `settings.rs`**

```rust
use sqlx::{PgPool, Row};

pub(crate) async fn get_setting(pool: &PgPool, key: &str) -> Option<String> {
    sqlx::query("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<String, _>("value").ok())
}

pub(crate) async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = $2")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Returns the stored agent model id, or "" if unset (env/default takes over).
/// The frontend maps the id to a display name via openrouter-models.ts.
#[tauri::command]
pub async fn get_agent_config_cmd(
    database: tauri::State<'_, crate::database::Database>,
) -> Result<String, String> {
    Ok(get_setting(&database.pool, "agent_model").await.unwrap_or_default())
}

/// Upsert the agent model; an empty string deletes the row (→ env var wins).
#[tauri::command]
pub async fn set_agent_config_cmd(
    model: String,
    database: tauri::State<'_, crate::database::Database>,
) -> Result<(), String> {
    let m = model.trim();
    if m.is_empty() {
        sqlx::query("DELETE FROM settings WHERE key = 'agent_model'")
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        set_setting(&database.pool, "agent_model", m).await?;
    }
    Ok(())
}
```

- [ ] **Step 3: Register in `main.rs`**

Add `mod settings;` near the other mods (~:38). Add to the `generate_handler![ … ]` list:
```rust
            settings::get_agent_config_cmd,
            settings::set_agent_config_cmd,
```

- [ ] **Step 4: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -8`
Expected: build clean; existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/migrations/055_settings.sql src-tauri/src/settings.rs src-tauri/src/main.rs
git commit -m "feat(settings): key-value settings table + agent_model get/set commands"
```

---

### Task 2: DB-aware agent model resolution

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (`AgentModelConfig::from_env` :23-33)
- Modify: `src-tauri/src/mimir_retrieval.rs` (call site :1190)

**Interfaces:**
- Consumes: `crate::settings::get_setting` (Task 1).
- Produces: `AgentModelConfig::from_env(pool: &sqlx::PgPool) -> Result<Self, String>` (async).

- [ ] **Step 1: Make `from_env` async + DB-aware**

Replace the existing `AgentModelConfig::from_env` (mimir_agent.rs:23-33) with:
```rust
    pub(crate) async fn from_env(pool: &sqlx::PgPool) -> Result<Self, String> {
        let model = crate::settings::get_setting(pool, "agent_model")
            .await
            .filter(|m| !m.is_empty())
            .or_else(|| std::env::var("MIMIR_AGENT_MODEL").ok())
            .unwrap_or_else(|| "llama-3.3-70b-versatile".to_string());
        let base_url = std::env::var("MIMIR_AGENT_BASE_URL")
            .unwrap_or_else(|_| crate::constants::GROQ_API_URL.to_string());
        let api_key = match std::env::var("MIMIR_AGENT_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => crate::mimir::groq_api_key()?,
        };
        Ok(Self { base_url, api_key, model })
    }
```

- [ ] **Step 2: Update the single call site**

In `mimir_retrieval.rs:1190`, change `AgentModelConfig::from_env()` to `AgentModelConfig::from_env(&database.pool).await`:
```rust
        match crate::mimir_agent::AgentModelConfig::from_env(&database.pool).await {
```
(This is inside `mimir_chat`, already async, `database` in scope. Confirm no OTHER caller of `AgentModelConfig::from_env` exists — grep; `RetrievalConfig::from_env` is a different type, leave it.)

- [ ] **Step 3: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -8`
Expected: build clean; tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs
git commit -m "feat(settings): agent model resolves DB setting > env > default"
```

---

### Task 3: tauri-plugin-dialog (folder picker infra)

**Files:**
- Modify: `src-tauri/Cargo.toml`, `package.json`, `src-tauri/src/main.rs`, `src-tauri/capabilities/default.json`

**Interfaces:**
- Produces: the `@tauri-apps/plugin-dialog` `open()` API available to the frontend; `dialog:allow-open` permission.

- [ ] **Step 1: Rust dep + plugin init**

In `src-tauri/Cargo.toml` `[dependencies]`, add: `tauri-plugin-dialog = "2"`.
In `src-tauri/src/main.rs`, register the plugin on the builder — change `tauri::Builder::default()` to:
```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
```
(Insert `.plugin(...)` immediately after `tauri::Builder::default()`, before `.setup(...)`.)

- [ ] **Step 2: Capability permission**

In `src-tauri/capabilities/default.json`, add `"dialog:allow-open"` to the `permissions` array (after `"opener:default"`).

- [ ] **Step 3: npm dep**

Run: `npm install @tauri-apps/plugin-dialog@^2` (from repo root). Confirm it lands in `package.json` dependencies.

- [ ] **Step 4: Build (Rust + TS)**

Run: `cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5` (with `CARGO_TARGET_DIR`) — expect the plugin crate compiles clean.
Run: `npx tsc --noEmit` — expect exit 0 (the plugin's types resolve).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock package.json package-lock.json src-tauri/src/main.rs src-tauri/capabilities/default.json
git commit -m "chore(settings): add tauri-plugin-dialog for native folder picker"
```

---

### Task 4: Model list + Settings page rewrite

**Files:**
- Create: `src/lib/openrouter-models.ts`
- Modify: `src/pages/SettingsPage.tsx` (full rewrite — 3 sections)

**Interfaces:**
- Consumes: `get_agent_config_cmd`/`set_agent_config_cmd` (Task 1); `get_project_roots_cmd`/`add_project_root_cmd`/`remove_project_root_cmd` (Step 3); `get_projects` (returns `Project[]` with `name`); `open` from `@tauri-apps/plugin-dialog` (Task 3).

- [ ] **Step 1: `openrouter-models.ts` (validated list)**

```ts
export interface OpenRouterModel { id: string; name: string; context: number; provider: string; }

// Validated against OpenRouter's live catalog on 2026-07-30.
export const OPENROUTER_MODELS: OpenRouterModel[] = [
  { id: "deepseek/deepseek-v4-flash", name: "DeepSeek V4 Flash", context: 1048576, provider: "DeepSeek" },
  { id: "deepseek/deepseek-v4-pro", name: "DeepSeek V4 Pro", context: 1048576, provider: "DeepSeek" },
  { id: "deepseek/deepseek-v3.2", name: "DeepSeek V3.2", context: 163840, provider: "DeepSeek" },
  { id: "google/gemini-3.6-flash", name: "Gemini 3.6 Flash", context: 1048576, provider: "Google" },
  { id: "google/gemini-3.5-flash", name: "Gemini 3.5 Flash", context: 1048576, provider: "Google" },
  { id: "google/gemini-3.5-flash-lite", name: "Gemini 3.5 Flash Lite", context: 1048576, provider: "Google" },
  { id: "anthropic/claude-sonnet-5", name: "Claude Sonnet 5", context: 1000000, provider: "Anthropic" },
  { id: "anthropic/claude-opus-5", name: "Claude Opus 5", context: 1000000, provider: "Anthropic" },
  { id: "anthropic/claude-opus-4.8", name: "Claude Opus 4.8", context: 1000000, provider: "Anthropic" },
  { id: "meta-llama/llama-4-maverick", name: "Llama 4 Maverick", context: 1048576, provider: "Meta" },
  { id: "meta-llama/llama-4-scout", name: "Llama 4 Scout", context: 1310720, provider: "Meta" },
  { id: "meta-llama/llama-3.3-70b-instruct", name: "Llama 3.3 70B Instruct", context: 131072, provider: "Meta" },
  { id: "openai/gpt-5.5-pro", name: "GPT-5.5 Pro", context: 1050000, provider: "OpenAI" },
  { id: "openai/gpt-5-mini", name: "GPT-5 Mini", context: 400000, provider: "OpenAI" },
  { id: "openai/o4-mini", name: "o4 Mini", context: 200000, provider: "OpenAI" },
  { id: "mistralai/mistral-medium-3-5", name: "Mistral Medium 3.5", context: 262144, provider: "Mistral" },
  { id: "mistralai/mistral-large-2512", name: "Mistral Large 3", context: 262144, provider: "Mistral" },
  { id: "mistralai/codestral-2508", name: "Codestral 2508", context: 256000, provider: "Mistral" },
  { id: "qwen/qwen3.7-flash", name: "Qwen3.7 Flash", context: 1000000, provider: "Qwen" },
  { id: "qwen/qwen3.7-max", name: "Qwen3.7 Max", context: 1000000, provider: "Qwen" },
  { id: "cohere/command-a", name: "Command A", context: 256000, provider: "Cohere" },
  { id: "x-ai/grok-4.3", name: "Grok 4.3", context: 1000000, provider: "xAI" },
  { id: "x-ai/grok-4.5", name: "Grok 4.5", context: 500000, provider: "xAI" },
  { id: "amazon/nova-2-lite-v1", name: "Nova 2 Lite", context: 1000000, provider: "Amazon" },
  { id: "amazon/nova-pro-v1", name: "Nova Pro", context: 300000, provider: "Amazon" },
];
```

- [ ] **Step 2: Rewrite `SettingsPage.tsx` (3 sections)**

Replace the whole file with:
```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Trash2, FolderOpen, Check } from "lucide-react";
import { OPENROUTER_MODELS } from "../lib/openrouter-models";

interface ProjectRoot { id: string; label: string; path: string; }
interface Project { id: string; name: string; }

const PROVIDERS = Array.from(new Set(OPENROUTER_MODELS.map((m) => m.provider)));

export default function SettingsPage() {
  // ── Agent model ──
  const [model, setModel] = useState<string>("");
  const [modelSaved, setModelSaved] = useState(false);
  // ── Project folders ──
  const [roots, setRoots] = useState<ProjectRoot[]>([]);
  const [label, setLabel] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  // ── Label-from-projects ──
  const [projects, setProjects] = useState<Project[]>([]);
  const [labelCustom, setLabelCustom] = useState(false);

  const loadRoots = () => invoke<ProjectRoot[]>("get_project_roots_cmd").then(setRoots).catch(() => {});
  useEffect(() => {
    invoke<string>("get_agent_config_cmd").then((m) => setModel(m ?? "")).catch(() => {});
    loadRoots();
    invoke<Project[]>("get_projects").then(setProjects).catch(() => setProjects([]));
  }, []);

  const changeModel = async (id: string) => {
    setModel(id);
    setModelSaved(false);
    try {
      await invoke("set_agent_config_cmd", { model: id });
      setModelSaved(true);
      setTimeout(() => setModelSaved(false), 2500);
    } catch (e) { setError(String(e)); }
  };

  const browse = async () => {
    const folder = await open({ directory: true, title: "Select project folder" });
    if (typeof folder === "string") setPath(folder);
  };

  const add = async () => {
    setError(null);
    try {
      await invoke<ProjectRoot>("add_project_root_cmd", { label, path });
      setLabel(""); setPath(""); setLabelCustom(false); loadRoots();
    } catch (e) { setError(String(e)); }
  };
  const remove = async (id: string) => { await invoke("remove_project_root_cmd", { id }); loadRoots(); };

  // If the stored model isn't in the static list (e.g. a custom env model), still show it.
  const modelInList = OPENROUTER_MODELS.some((m) => m.id === model);

  return (
    <div className="p-8 max-w-2xl space-y-10">
      <h1 className="text-2xl font-semibold">Settings</h1>

      {/* Section 1 — Agent Model */}
      <section>
        <h2 className="text-lg font-medium mb-1">Agent Model</h2>
        <p className="text-sm opacity-60 mb-3">
          Which model Mimir uses to think and call tools. Takes effect on your next message. "Env Default" uses whatever <code>MIMIR_AGENT_MODEL</code> is set to.
        </p>
        <div className="flex items-center gap-2">
          <select value={model} onChange={(e) => changeModel(e.target.value)}
            className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm min-w-[22rem]">
            <option value="">Env Default</option>
            {!modelInList && model !== "" && <option value={model}>{model} (current)</option>}
            {PROVIDERS.map((p) => (
              <optgroup key={p} label={p}>
                {OPENROUTER_MODELS.filter((m) => m.provider === p).map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name} — {(m.context / 1000).toFixed(0)}k ctx
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
          {modelSaved && <span className="text-xs text-green-400 flex items-center gap-1"><Check className="w-3.5 h-3.5" /> saved</span>}
        </div>
      </section>

      {/* Section 2 — Project Folders */}
      <section>
        <h2 className="text-lg font-medium mb-1">Project Folders</h2>
        <p className="text-sm opacity-60 mb-4">
          Folders the Mimir agent may read from (read-only). Only these are accessible; secret files (.env, keys, .ssh, .git, cloud creds) are always blocked.
        </p>
        <div className="space-y-2 mb-4">
          {roots.length === 0 && <p className="text-sm opacity-50">No folders registered yet.</p>}
          {roots.map((r) => (
            <div key={r.id} className="flex items-center gap-3 rounded-md border border-white/10 bg-white/5 px-3 py-2">
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium">{r.label}</div>
                <div className="text-xs opacity-60 truncate">{r.path}</div>
              </div>
              <button onClick={() => remove(r.id)} className="opacity-60 hover:opacity-100" title="Remove">
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
          ))}
        </div>
        <div className="space-y-2">
          <div className="flex gap-2">
            <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="C:\Users\you\projects\repo"
              className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm flex-1" />
            <button onClick={browse}
              className="rounded-md bg-white/10 hover:bg-white/20 px-3 py-2 text-sm flex items-center gap-1">
              <FolderOpen className="w-4 h-4" /> Browse
            </button>
          </div>
          <div className="flex gap-2 items-center">
            {labelCustom || projects.length === 0 ? (
              <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Label (optional)"
                className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm w-56" />
            ) : (
              <select value={label}
                onChange={(e) => { if (e.target.value === "__custom__") { setLabelCustom(true); setLabel(""); } else setLabel(e.target.value); }}
                className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm w-56">
                <option value="">Label (optional)</option>
                {projects.map((p) => <option key={p.id} value={p.name}>{p.name}</option>)}
                <option value="__custom__">Custom…</option>
              </select>
            )}
            <button onClick={add} disabled={!path.trim()}
              className="rounded-md bg-white/10 hover:bg-white/20 px-3 py-2 text-sm flex items-center gap-1 disabled:opacity-40">
              <FolderPlus className="w-4 h-4" /> Add
            </button>
          </div>
        </div>
        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
      </section>
    </div>
  );
}
```

- [ ] **Step 3: Verify the frontend compiles**

Run: `npx tsc --noEmit` (repo root). Expected: exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/lib/openrouter-models.ts src/pages/SettingsPage.tsx
git commit -m "feat(settings): model dropdown, folder Browse picker, project-label select"
```

---

### Task 5: Live verification (runtime checklist)

**Files:** none.

- [ ] **Step 1:** `bash dev.sh` (applies migration 055). Open Settings.
- [ ] **Step 2:** Agent Model dropdown pre-selects the current value ("Env Default" if unset, i.e. the `deepseek/deepseek-v4-flash` env default). Pick a different model → "saved" indicator → ask Mimir something → console shows the new model on the turn. Pick "Env Default" → next turn uses the env var again.
- [ ] **Step 3:** In Project Folders, click **Browse** → native OS folder picker opens → selecting a folder fills the path input. Cancel → no-op.
- [ ] **Step 4:** The label control is a dropdown of project names + "Custom…"; selecting a name fills the label; "Custom…" reveals a text input. Add a folder → appears in the list. Remove works.
- [ ] **Step 5:** Confirm `MIMIR_AGENT_MODEL` env var still works when no DB row exists (i.e. after picking "Env Default").

---

## Self-Review

**Design coverage (+ 3 corrections):** model selector persisted to `settings` (Task 1) resolved DB>env>default (Task 2); frontend id→name resolution (Task 4, `openrouter-models.ts` + `modelInList`); migration **055** not 056 (Task 1); folder picker via tauri-plugin-dialog (Task 3 + Task 4 `browse`); project-label `<select>` from `get_projects` with "Custom…" (Task 4). Validated 25-model list baked (Task 4 Step 1). ✓

**Placeholder scan:** No TBD/TODO; full code for new files + the SettingsPage rewrite. Task 1 Step 3 / Task 3 are locate-and-insert into `main.rs`/config with exact snippets.

**Type consistency:** `get_agent_config_cmd` returns `String` (id or ""), consumed as `string` in the frontend; `set_agent_config_cmd({ model })` matches the invoke. `open()` from `@tauri-apps/plugin-dialog` returns `string | string[] | null` → guarded with `typeof folder === "string"`. Command names match registration.

**Edge cases handled:** stored model not in the static list → shown as a "(current)" option, not lost; `get_projects` failure → empty projects → label falls back to a text input (`projects.length === 0`); folder picker cancelled → `null` → no-op; empty model → row deleted → env/default.

**Note on TDD:** DB/command/Tauri-plugin/frontend code with no isolated pure logic — verified by `cargo test` (existing) + `tsc` + the Task 5 runtime checklist, consistent with the codebase's approach.

## Explicitly out of scope
Live OpenRouter fetch; storing base_url/api_key in DB; per-tree model overrides; pricing display; multi-provider base_url toggle; custom model text entry (beyond showing an existing custom env value); folder picker theming.
