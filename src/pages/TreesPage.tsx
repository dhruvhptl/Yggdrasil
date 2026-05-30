// src/pages/TreesPage.tsx
import { useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { TreePine, Trash2 } from "lucide-react";
import { Project } from "../types";

export default function TreesPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();

  async function handleDeleteProject(e: React.MouseEvent, projectId: string) {
    e.stopPropagation();
    if (!confirm("Delete this project? This will remove the tree and all progress.")) return;
    try {
      await invoke("delete_project", { projectId });
      setProjects((prev) => prev.filter((p) => p.id !== projectId));
    } catch (err) {
      console.error("Failed to delete project:", err);
    }
  }

  useEffect(() => {
    invoke<Project[]>("get_projects")
      .then(setProjects)
      .catch((err) => console.error("Failed to load projects:", err))
      .finally(() => setLoading(false));
  }, []);

  // If we arrived via `/trees?selected=<treeId>` (e.g. from Growth Plan
  // "Open tree →"), resolve the tree to its project and route to the
  // existing canvas — which is how every other tree open works on this page.
  useEffect(() => {
    const selectedTreeId = searchParams.get("selected");
    if (!selectedTreeId) return;
    invoke<{ projectId: string } | null>("get_project_id_for_tree", { treeId: selectedTreeId })
      .then((row) => {
        if (row && row.projectId) {
          navigate(`/project/${row.projectId}`, { replace: true });
        }
      })
      .catch((err) => console.error("Failed to resolve selected tree:", err));
  }, [searchParams, navigate]);

  return (
    <div className="p-6 max-w-2xl mx-auto flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-semibold text-slate-100">Trees</h1>
        <p className="text-sm text-slate-400 mt-1">
          Navigate to your project skill trees.
        </p>
      </div>

      {loading ? (
        <p className="text-sm text-slate-500">Loading...</p>
      ) : projects.length === 0 ? (
        <p className="text-sm text-slate-500">
          No trees yet. Create a project from the Home page.
        </p>
      ) : (
        <>
          <p className="text-xs font-medium text-slate-500 uppercase tracking-wider -mb-4">
            {projects.length} {projects.length === 1 ? "project" : "projects"}
          </p>
          <div className="flex flex-col gap-2">
            {projects.map((project) => (
              <div
                key={project.id}
                onClick={() => navigate(`/project/${project.id}`)}
                className="group bg-slate-900 border border-slate-800 hover:border-emerald-800/60 rounded-lg p-4 cursor-pointer transition-all flex items-center gap-4"
              >
                {/* Tree icon */}
                <div className="w-10 h-10 rounded-lg bg-emerald-950 border border-emerald-900/60 flex items-center justify-center flex-shrink-0">
                  <TreePine className="w-5 h-5 text-emerald-400" />
                </div>

                {/* Info */}
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-semibold text-slate-100 group-hover:text-emerald-300 transition-colors">
                    {project.name}
                  </p>
                  {project.description && (
                    <p className="text-xs text-slate-500 mt-0.5 truncate">
                      {project.description}
                    </p>
                  )}
                  <div className="flex items-center gap-2 mt-1.5">
                    <span className="text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-500 border border-slate-700/50">
                      {project.status}
                    </span>
                    <span className="text-xs text-slate-600">
                      {project.createdAt && !isNaN(Date.parse(project.createdAt))
                        ? new Date(project.createdAt).toLocaleDateString()
                        : ""}
                    </span>
                  </div>
                </div>

                {/* Progress */}
                {project.progress > 0 && (
                  <div className="flex items-center gap-2 flex-shrink-0">
                    <div className="w-16 h-1 bg-slate-800 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-emerald-500 rounded-full transition-all"
                        style={{ width: `${project.progress}%` }}
                      />
                    </div>
                    <span className="text-xs text-slate-600">
                      {project.progress}%
                    </span>
                  </div>
                )}

                <button
                  onClick={(e) => handleDeleteProject(e, project.id)}
                  className="opacity-0 group-hover:opacity-100 transition-opacity p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-950/40 flex-shrink-0"
                  title="Delete project"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
                <span className="text-slate-700 group-hover:text-emerald-600 transition-colors text-sm flex-shrink-0">
                  →
                </span>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
