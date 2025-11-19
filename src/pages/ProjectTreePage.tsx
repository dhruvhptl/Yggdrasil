// src/pages/ProjectTreePage.tsx

import { useParams } from 'react-router-dom';
import EditableSkillTree from '../components/EditableSkillTree';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export default function ProjectTreePage() {
  const { projectId } = useParams<{ projectId: string }>();
  const [projectName, setProjectName] = useState<string>('');
  
  console.log('===== ProjectTreePage Debug =====');
  console.log('projectId from params:', projectId);
  console.log('projectId type:', typeof projectId);
  console.log('projectId truthy?', !!projectId);

  useEffect(() => {
    if (projectId) {
      // Load project name for display
      invoke<any[]>('get_projects').then(projects => {
        const project = projects.find(p => p.id === projectId);
        if (project) {
          setProjectName(project.name);
        }
      });
    }
  }, [projectId]);
  
  if (!projectId) {
    console.log('❌ No projectId - showing error');
    return <div>Project not found</div>;
  }
  
  console.log('✅ Rendering tree with projectId:', projectId);


  return (
    <div>
      <h1 style={{ padding: '20px', margin: 0 }}>
        Skill Tree: {projectName || 'Loading...'}
      </h1>
      <EditableSkillTree projectId={projectId} />
    </div>
  );
}
