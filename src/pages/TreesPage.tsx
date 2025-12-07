// src/pages/TreesPage.tsx
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";

interface Project {
  id: string;
  name: string;
  created_at: string;
  status: string;
}

export default function TreesPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const navigate = useNavigate();

  useEffect(() => {
    invoke<Project[]>("get_projects").then(setProjects);
  }, []);

  const handleOpenProject = (id: string) => {
    navigate(`/project/${id}`);
  };

  return (
    <div style={{ padding: "20px" }}>
      <h1>Your Trees</h1>

      {projects.length === 0 && (
        <p>No trees yet. Create one from the Home page.</p>
      )}

      <div style={{ marginTop: "16px", display: "flex", gap: "12px", flexWrap: "wrap" }}>
        {projects.map((project) => (
          <div
            key={project.id}
            onClick={() => handleOpenProject(project.id)}
            style={{
              padding: "12px 16px",
              borderRadius: "8px",
              background: "white",
              cursor: "pointer",
              boxShadow: "0 1px 3px rgba(0,0,0,0.1)",
            }}
          >
            <div style={{ fontWeight: 600 }}>{project.name}</div>
            <div style={{ fontSize: "12px", color: "#666" }}>
              Created: {project.created_at}
            </div>
            <div style={{ fontSize: "12px", marginTop: "4px" }}>
              Status: {project.status}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
