// src/pages/ProjectTreePage.tsx
import { useParams, useNavigate } from 'react-router-dom';
import YggdrasilTree from '../components/YggdrasilTree';
import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { exportTreeAsZip } from '../utils/exportTree';

type InputMode = 'prd' | 'github';

export default function ProjectTreePage() {
  const { projectId } = useParams<{ projectId: string }>();
  const navigate = useNavigate();
  const [projectName, setProjectName] = useState<string>('');
  const [inputMode, setInputMode] = useState<InputMode>('prd');
  const [prdText, setPrdText] = useState<string>('');
  const [githubUrl, setGithubUrl] = useState<string>('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [treeKey, setTreeKey] = useState(0);
  const [prdOpen, setPrdOpen] = useState(true);
  const [hasTree, setHasTree] = useState(false);
  const [isExporting, setIsExporting] = useState(false);

  useEffect(() => {
    if (projectId) {
      invoke<any[]>('get_projects').then(projects => {
        const project = projects.find(p => p.id === projectId);
        if (project) setProjectName(project.name);
      });
      invoke<any[]>('get_trees', { projectId }).then(trees => {
        setHasTree(trees.length > 0);
      });
    }
  }, [projectId, treeKey]);

  const canSubmit = inputMode === 'prd' ? prdText.trim().length > 0 : githubUrl.trim().length > 0;

  const handleExport = async () => {
    if (!projectId || isExporting) return;
    setIsExporting(true);
    try {
      await exportTreeAsZip(projectId);
      alert('Export downloaded!');
    } catch (err) {
      alert(`Export failed: ${err}`);
    } finally {
      setIsExporting(false);
    }
  };

  const handleGenerate = async () => {
    if (!projectId || !canSubmit) return;

    setIsGenerating(true);
    try {
      let treeJson: string;

      if (inputMode === 'prd') {
        treeJson = await invoke<string>('generate_skill_tree', { projectId, prdText });
      } else {
        treeJson = await invoke<string>('analyze_repo', { projectId, githubUrl });
      }

      // Tree is saved to DB by the Rust command — remount YggdrasilTree to reload it
      JSON.parse(treeJson); // validate JSON (throws if malformed)
      setTreeKey(k => k + 1);
      setPrdOpen(false);
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

  const floatBtn: React.CSSProperties = {
    display: 'flex', alignItems: 'center', gap: 5,
    padding: '6px 11px',
    background: 'rgba(15, 23, 42, 0.82)',
    backdropFilter: 'blur(6px)',
    border: '1px solid #1e293b',
    borderRadius: 7,
    color: '#64748b',
    fontSize: 12,
    cursor: 'pointer',
    fontFamily: 'inherit',
    transition: 'color 0.15s, border-color 0.15s',
    whiteSpace: 'nowrap' as const,
  };

  const tabStyle = (active: boolean): React.CSSProperties => ({
    flex: 1, padding: '5px 0', fontSize: 11, fontWeight: 500,
    cursor: 'pointer', borderRadius: 5, border: 'none',
    background: active ? '#1e3a2e' : 'transparent',
    color: active ? '#6ee7b7' : '#475569',
    transition: 'background 0.15s, color 0.15s',
  });

  return (
    <div style={{ height: '100%', display: 'flex' }}>
      {/* Left panel */}
      <aside style={{
        width: prdOpen ? 288 : 0,
        flexShrink: 0,
        overflow: 'hidden',
        transition: 'width 0.2s ease',
        borderRight: prdOpen ? '1px solid #1e293b' : 'none',
        background: 'rgba(15, 23, 42, 0.35)',
      }}>
        <div style={{ width: 288, height: '100%', display: 'flex', flexDirection: 'column', gap: 12, padding: 16 }}>
          {/* Header */}
          <div className="flex items-center justify-between gap-2">
            <h2 className="text-sm font-semibold text-slate-100 truncate">
              {projectName || 'Loading…'}
            </h2>
            <button
              onClick={() => setPrdOpen(false)}
              title="Collapse panel"
              style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', fontSize: 16, lineHeight: 1, padding: '0 2px', flexShrink: 0 }}
              onMouseEnter={e => (e.currentTarget.style.color = '#94a3b8')}
              onMouseLeave={e => (e.currentTarget.style.color = '#475569')}
            >×</button>
          </div>

          {/* Mode toggle: PRD / GitHub */}
          <div style={{ display: 'flex', gap: 4, background: '#0f172a', borderRadius: 7, padding: 3 }}>
            <button style={tabStyle(inputMode === 'prd')} onClick={() => setInputMode('prd')}>
              From PRD
            </button>
            <button style={tabStyle(inputMode === 'github')} onClick={() => setInputMode('github')}>
              From GitHub
            </button>
          </div>

          {/* Input area */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
            {inputMode === 'prd' ? (
              <>
                <label className="text-xs font-medium text-slate-400 mb-1.5 uppercase tracking-wider">
                  Project PRD
                </label>
                <textarea
                  value={prdText}
                  onChange={e => setPrdText(e.target.value)}
                  placeholder={`Paste your PRD here…\n\nExample:\n# Project: Learn Quantum Computing\n## Goal\nBuild foundational understanding…\n\n## Core Topics\n- Linear algebra\n- Quantum circuits\n- Key algorithms`}
                  className="flex-1 bg-slate-950 border border-slate-700 rounded-md p-3 text-sm text-slate-200 font-mono resize-none focus:outline-none focus:border-emerald-600"
                  style={{ minHeight: 0 }}
                />
              </>
            ) : (
              <>
                <label className="text-xs font-medium text-slate-400 mb-1.5 uppercase tracking-wider">
                  GitHub Repository URL
                </label>
                <input
                  type="url"
                  value={githubUrl}
                  onChange={e => setGithubUrl(e.target.value)}
                  placeholder="https://github.com/owner/repo"
                  className="bg-slate-950 border border-slate-700 rounded-md p-3 text-sm text-slate-200 focus:outline-none focus:border-emerald-600"
                />
                <p className="text-xs text-slate-600 mt-2">
                  Fetches README + dependency files to generate a learning tree of concepts you used.
                  Add <code className="text-slate-500">GITHUB_TOKEN</code> to <code className="text-slate-500">.env</code> for private repos.
                </p>
              </>
            )}
          </div>

          <button
            onClick={handleGenerate}
            disabled={isGenerating || !canSubmit}
            className="px-4 py-2.5 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md font-medium text-sm transition-colors"
          >
            {isGenerating ? (
              <span className="flex items-center justify-center gap-2">
                <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none"/>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
                </svg>
                {inputMode === 'github' ? 'Analyzing repo…' : 'Generating…'}
              </span>
            ) : (
              inputMode === 'github' ? 'Analyze Repo' : 'Generate Skill Tree'
            )}
          </button>

          <p className="text-xs text-slate-600">
            {inputMode === 'prd'
              ? 'Include goals, timeline, and key topics for best results.'
              : 'Uses the README and dependency files to map learning gaps.'}
          </p>
        </div>
      </aside>

      {/* Main area */}
      <main className="relative" style={{ flex: 1, minWidth: 0, height: '100%', overflow: 'hidden', background: '#010208' }}>
        {/* Floating top-left buttons */}
        <div style={{ position: 'absolute', top: 12, left: 12, zIndex: 30, display: 'flex', gap: 6 }}>
          <button
            onClick={() => navigate('/')}
            style={floatBtn}
            onMouseEnter={e => (e.currentTarget.style.color = '#94a3b8')}
            onMouseLeave={e => (e.currentTarget.style.color = '#64748b')}
          >← Projects</button>
          {!prdOpen && (
            <button
              onClick={() => setPrdOpen(true)}
              style={floatBtn}
              onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8'; e.currentTarget.style.borderColor = '#334155'; }}
              onMouseLeave={e => { e.currentTarget.style.color = '#64748b'; e.currentTarget.style.borderColor = '#1e293b'; }}
            >Edit PRD ↓</button>
          )}
          {hasTree && (
            <button
              onClick={handleExport}
              disabled={isExporting}
              style={{ ...floatBtn, opacity: isExporting ? 0.5 : 1 }}
              onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8'; e.currentTarget.style.borderColor = '#334155'; }}
              onMouseLeave={e => { e.currentTarget.style.color = '#64748b'; e.currentTarget.style.borderColor = '#1e293b'; }}
            >{isExporting ? 'Exporting…' : 'Export ↓'}</button>
          )}
        </div>

        <YggdrasilTree key={treeKey} projectId={projectId} />
      </main>
    </div>
  );
}
