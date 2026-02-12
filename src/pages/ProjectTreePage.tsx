// src/pages/ProjectTreePage.tsx
import { useParams } from 'react-router-dom';
import EditableSkillTree from '../components/EditableSkillTree';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export default function ProjectTreePage() {
  const { projectId } = useParams<{ projectId: string }>();
  const [projectName, setProjectName] = useState<string>('');
  const [prdText, setPrdText] = useState<string>('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [generatedTree, setGeneratedTree] = useState<any>(null);
  
  useEffect(() => {
    if (projectId) {
      invoke<any[]>('get_projects').then(projects => {
        const project = projects.find(p => p.id === projectId);
        if (project) {
          setProjectName(project.name);
        }
      });
    }
  }, [projectId]);

  const handleGenerateTree = async () => {
    if (!projectId || !prdText.trim()) return;
    
    setIsGenerating(true);
    try {
      const treeJson = await invoke<string>('generate_skill_tree', {
        projectId,
        prdText
      });
      
      console.log('Raw response from Rust:', treeJson);
      const tree = JSON.parse(treeJson);
      console.log('Parsed tree:', tree);
      
      setGeneratedTree(tree);
      alert('Tree generated successfully!');
    } catch (error) {
      console.error('Failed to generate tree:', error);
      alert(`Failed to generate tree: ${error}`);
    } finally {
      setIsGenerating(false);
    }
  };
  
  if (!projectId) {
    return <div className="p-6 text-slate-300">Project not found</div>;
  }

  return (
    <div className="h-full flex">
      {/* Left panel: PRD input */}
      <aside className="w-96 border-r border-slate-800 bg-slate-900/40 p-4 flex flex-col gap-3">
        <div>
          <h2 className="text-lg font-semibold text-slate-100">
            {projectName || 'Loading...'}
          </h2>
          <p className="text-xs text-slate-500 mt-1">
            Project ID: {projectId}
          </p>
        </div>
        
        <div className="flex-1 flex flex-col">
          <label className="text-sm font-medium text-slate-300 mb-2">
            Project PRD
          </label>
          <textarea
            value={prdText}
            onChange={(e) => setPrdText(e.target.value)}
            placeholder="Paste your PRD here...

Example:
# Project: Learn Quantum Computing
## Goal
Build foundational understanding of quantum mechanics and quantum algorithms.

## Timeline
3 months

## Core Topics
- Linear algebra review
- Quantum mechanics basics
- Quantum circuits
- Key algorithms (Shor's, Grover's)
"
            className="flex-1 bg-slate-950 border border-slate-700 rounded-md p-3 text-sm text-slate-200 font-mono resize-none focus:outline-none focus:border-emerald-500 focus:ring-1 focus:ring-emerald-500"
          />
        </div>
        
        <button
          onClick={handleGenerateTree}
          disabled={isGenerating || !prdText.trim()}
          className="px-4 py-2.5 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md font-medium text-sm transition-colors shadow-sm"
        >
          {isGenerating ? (
            <span className="flex items-center justify-center gap-2">
              <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none"/>
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
              </svg>
              Generating...
            </span>
          ) : (
            'Generate Skill Tree'
          )}
        </button>

        <div className="text-xs text-slate-500 space-y-1">
          <p>💡 Tip: Include goals, timeline, and key topics</p>
          <p>⚡ Tree generation takes ~5-10 seconds</p>
        </div>
      </aside>

      {/* Main area: Tree visualization */}
      <main className="flex-1 overflow-auto bg-slate-950">
        {generatedTree ? (
          <div className="p-6">
            <div className="mb-6 flex items-center justify-between">
              <div>
                <h3 className="text-xl font-bold text-slate-100">Generated Skill Tree</h3>
                <p className="text-sm text-slate-400 mt-1">
                  {generatedTree.phases?.length || 0} phases • 
                  {' '}{generatedTree.phases?.reduce((sum: number, p: any) => sum + (p.skills?.length || 0), 0) || 0} skills •
                  {' '}{generatedTree.phases?.reduce((sum: number, p: any) => sum + p.skills?.reduce((s2: number, sk: any) => s2 + (sk.quests?.length || 0), 0), 0) || 0} quests
                </p>
              </div>
              <button
                onClick={() => setGeneratedTree(null)}
                className="px-3 py-1.5 text-sm text-slate-400 hover:text-slate-200 border border-slate-700 hover:border-slate-600 rounded transition-colors"
              >
                Clear Tree
              </button>
            </div>

            <div className="space-y-8">
              {generatedTree.phases?.map((phase: any, phaseIdx: number) => (
                <div key={phase.id} className="border border-slate-700 rounded-lg overflow-hidden">
                  {/* Phase header */}
                  <div className="bg-slate-800/60 p-4 border-b border-slate-700">
                    <div className="flex items-center gap-3">
                      <div className="w-8 h-8 rounded-full bg-emerald-600 flex items-center justify-center text-white font-bold text-sm">
                        {phaseIdx + 1}
                      </div>
                      <div className="flex-1">
                        <h4 className="text-lg font-semibold text-slate-100">{phase.name}</h4>
                        <p className="text-sm text-slate-400 mt-0.5">{phase.description}</p>
                      </div>
                    </div>
                  </div>

                  {/* Skills in this phase */}
                  <div className="p-4 space-y-4">
                    {phase.skills?.map((skill: any) => (
                      <div key={skill.id} className="bg-slate-900/60 border border-slate-700 rounded-lg p-4">
                        <div className="mb-3">
                          <h5 className="text-base font-semibold text-emerald-300">{skill.name}</h5>
                          <p className="text-sm text-slate-400 mt-1">{skill.description}</p>
                        </div>

                        {/* Quests for this skill */}
                        <div className="space-y-2">
                          {skill.quests?.map((quest: any) => (
                            <div 
                              key={quest.id} 
                              className="flex items-start gap-3 p-3 bg-slate-950 border border-slate-700/50 rounded hover:border-slate-600 transition-colors"
                            >
                              <div className="w-5 h-5 mt-0.5 rounded border-2 border-slate-600 flex-shrink-0" />
                              <div className="flex-1 min-w-0">
                                <h6 className="text-sm font-medium text-slate-200">{quest.title}</h6>
                                <p className="text-xs text-slate-500 mt-1">{quest.description}</p>
                                <div className="flex items-center gap-3 mt-2 text-xs">
                                  <span className={`px-2 py-0.5 rounded ${
                                    quest.difficulty === 'easy' ? 'bg-green-900/40 text-green-300' :
                                    quest.difficulty === 'medium' ? 'bg-yellow-900/40 text-yellow-300' :
                                    'bg-red-900/40 text-red-300'
                                  }`}>
                                    {quest.difficulty}
                                  </span>
                                  <span className="text-slate-500">~{quest.estimated_hours}h</span>
                                </div>
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        ) : (
          <EditableSkillTree projectId={projectId} />
        )}
      </main>
    </div>
  );
}
