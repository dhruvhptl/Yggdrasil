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
  const [modelError, setModelError] = useState<string | null>(null);
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
    const prev = model;
    setModel(id);
    setModelSaved(false);
    setModelError(null);
    try {
      await invoke("set_agent_config_cmd", { model: id });
      setModelSaved(true);
      setTimeout(() => setModelSaved(false), 2500);
    } catch (e) { setModel(prev); setModelError(String(e)); }
  };

  const browse = async () => {
    try {
      const folder = await open({ directory: true, title: "Select project folder" });
      if (typeof folder === "string") setPath(folder);
    } catch (e) { setError(String(e)); }
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
        {modelError && <p className="mt-2 text-sm text-red-400">{modelError}</p>}
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
