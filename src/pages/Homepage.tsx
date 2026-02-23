// src/pages/Homepage.tsx
import { useState, useEffect, useMemo } from "react";
import ProjectInput from "../components/ProjectInput";
import { Project } from "../types";
import { invoke } from "@tauri-apps/api/core";
import { useNavigate } from "react-router-dom";

interface Discipline {
  id: string;
  name: string;
}

export default function HomePage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [disciplines, setDisciplines] = useState<Discipline[]>([]);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();

  useEffect(() => {
    loadProjects();
    invoke<Discipline[]>("get_disciplines")
      .then(setDisciplines)
      .catch(() => {});
  }, []);

  async function loadProjects() {
    try {
      const list = await invoke<Project[]>("get_projects");
      setProjects(list);
    } catch (err) {
      console.error("Failed to load projects:", err);
    } finally {
      setLoading(false);
    }
  }

  const disciplineMap = useMemo(() => {
    const map = new Map<string, string>();
    disciplines.forEach((d) => map.set(d.id, d.name));
    return map;
  }, [disciplines]);

  function handleProjectCreated(project: Project) {
    setProjects((prev) => [project, ...prev]);
  }

  return (
    <div className="p-6 max-w-2xl mx-auto flex flex-col gap-6">
      {/* Header */}
      <div>
        <h1 className="text-xl font-semibold text-slate-100">Projects</h1>
        <p className="text-sm text-slate-400 mt-1">
          Your skill trees and learning journeys.
        </p>
      </div>

      {/* Create project */}
      <ProjectInput onProjectCreated={handleProjectCreated} />

      {/* Project list */}
      <div className="flex flex-col gap-3">
        <h2 className="text-xs font-medium text-slate-500 uppercase tracking-wider">
          {loading
            ? "Loading…"
            : projects.length === 0
            ? "No projects yet"
            : `${projects.length} project${projects.length === 1 ? "" : "s"}`}
        </h2>

        {!loading && projects.length === 0 && (
          <p className="text-sm text-slate-500">
            Create your first project above to get started.
          </p>
        )}

        <div className="flex flex-col gap-2">
          {projects.map((project) => {
            const disciplineName =
              project.disciplineIds?.[0]
                ? disciplineMap.get(project.disciplineIds[0])
                : null;

            return (
              <div
                key={project.id}
                onClick={() => navigate(`/project/${project.id}`)}
                className="group bg-slate-900 border border-slate-800 hover:border-emerald-800/50 rounded-lg p-4 cursor-pointer transition-all"
              >
                <div className="flex items-start justify-between gap-4">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <h3 className="text-sm font-semibold text-slate-100 group-hover:text-emerald-300 transition-colors">
                        {project.name}
                      </h3>
                      {disciplineName && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-emerald-950 text-emerald-400 border border-emerald-900/60">
                          {disciplineName}
                        </span>
                      )}
                      <span className="text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-500 border border-slate-700/60">
                        {project.status}
                      </span>
                    </div>
                    {project.description && (
                      <p className="text-xs text-slate-400 mt-1 line-clamp-1">
                        {project.description}
                      </p>
                    )}
                  </div>
                  <div className="flex items-center gap-3 flex-shrink-0">
                    <span className="text-xs text-slate-600 mt-0.5">
                      {project.createdAt && !isNaN(Date.parse(project.createdAt))
                        ? new Date(project.createdAt).toLocaleDateString()
                        : ""}
                    </span>
                    <span className="text-xs text-slate-600 group-hover:text-emerald-600 transition-colors mt-0.5">
                      →
                    </span>
                  </div>
                </div>

                {project.progress > 0 && (
                  <div className="mt-3 flex items-center gap-2">
                    <div className="flex-1 h-1 bg-slate-800 rounded-full overflow-hidden">
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
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
