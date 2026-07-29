import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderPlus, Trash2 } from "lucide-react";

interface ProjectRoot { id: string; label: string; path: string; }

export default function SettingsPage() {
  const [roots, setRoots] = useState<ProjectRoot[]>([]);
  const [label, setLabel] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = () => invoke<ProjectRoot[]>("get_project_roots_cmd").then(setRoots).catch(() => {});
  useEffect(() => { load(); }, []);

  const add = async () => {
    setError(null);
    try {
      await invoke<ProjectRoot>("add_project_root_cmd", { label, path });
      setLabel(""); setPath(""); load();
    } catch (e) { setError(String(e)); }
  };
  const remove = async (id: string) => { await invoke("remove_project_root_cmd", { id }); load(); };

  return (
    <div className="p-8 max-w-2xl">
      <h1 className="text-2xl font-semibold mb-6">Settings</h1>
      <section>
        <h2 className="text-lg font-medium mb-1">Project Folders</h2>
        <p className="text-sm opacity-60 mb-4">
          Folders the Mimir agent may read from (read-only). Only these are accessible; secret files (.env, keys, .ssh, .git) are always blocked.
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
        <div className="flex gap-2 items-start">
          <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Label (optional)"
            className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm w-40" />
          <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="C:\Users\you\projects\repo"
            className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm flex-1" />
          <button onClick={add} disabled={!path.trim()}
            className="rounded-md bg-white/10 hover:bg-white/20 px-3 py-2 text-sm flex items-center gap-1 disabled:opacity-40">
            <FolderPlus className="w-4 h-4" /> Add
          </button>
        </div>
        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
      </section>
    </div>
  );
}
