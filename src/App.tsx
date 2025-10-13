// src/App.tsx - Main app component

import { useState, useEffect } from "react";
import ProjectInput from "./components/ProjectInput";
import { Project } from "./types";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);

  // Load projects on component mount
  useEffect(() => {
    loadProjects();
  }, []);

  const loadProjects = async () => {
    try {
      const projectList = await invoke<Project[]>('get_projects');
      setProjects(projectList);
    } catch (error) {
      console.error('Failed to load projects:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleProjectCreated = (newProject: Project) => {
    setProjects(prev => [...prev, newProject]);
  };

  return (
    <div className="container mx-auto p-4">
      <h1 className="text-3xl font-bold text-center mb-8">Universal Skill Tree</h1>
      
      {/* Project Input Form */}
      <div className="mb-8">
        <ProjectInput onProjectCreated={handleProjectCreated} />
      </div>

      {/* Projects List */}
      <div className="max-w-4xl mx-auto">
        <h2 className="text-2xl font-bold mb-4">Your Projects</h2>
        
        {loading ? (
          <p>Loading projects...</p>
        ) : projects.length === 0 ? (
          <p className="text-gray-600">No projects yet. Create your first project above!</p>
        ) : (
          <div className="grid gap-4">
            {projects.map(project => {
              console.log("project.created_at value:", project.createdAt);
              return (
                <div key={project.id} className="border rounded-lg p-4 bg-white shadow">
                  <h3 className="text-xl font-semibold">{project.name}</h3>
                  <p className="text-gray-600 mt-1">{project.description}</p>
                  <div className="mt-2 flex items-center justify-between">
                    <span>
                      Created: {project.createdAt && typeof project.createdAt === "string" &&
                        !isNaN(Date.parse(project.createdAt))
                          ? new Date(project.createdAt).toLocaleDateString()
                          : "N/A"}
                    </span>

                    <span className="text-sm bg-blue-100 text-blue-800 px-2 py-1 rounded">
                      {project.status}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
