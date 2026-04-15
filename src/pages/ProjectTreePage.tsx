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
  const [pdfFileName, setPdfFileName] = useState<string | null>(null);
  const [isPdfExtracting, setIsPdfExtracting] = useState(false);
  const [githubUrl, setGithubUrl] = useState<string>('');
  const [isGenerating, setIsGenerating] = useState(false);
  const [treeKey, setTreeKey] = useState(0);
  const [prdOpen, setPrdOpen] = useState(true);
  const [hasTree, setHasTree] = useState(false);
  const [isExporting, setIsExporting] = useState(false);
  const pdfInputRef = React.useRef<HTMLInputElement>(null);

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

  const handlePdfUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    setIsPdfExtracting(true);
    setPdfFileName(file.name);
    setPrdText('');

    try {
      const arrayBuffer = await file.arrayBuffer();
      const bytes = new Uint8Array(arrayBuffer);
      let binary = '';
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
      const base64 = btoa(binary);

      const response = await fetch('http://localhost:3002/fetch-pdf', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ pdf_base64: base64, filename: file.name }),
      });

      if (!response.ok) {
        const err = await response.text();
        throw new Error(`Scraper error: ${err}`);
      }

      const data = await response.json();
      const text: string = data.text ?? '';
      if (!text.trim()) throw new Error('PDF extracted no text — it may be scanned/image-based.');

      setPrdText(text);
    } catch (err) {
      alert(`PDF extraction failed: ${err}`);
      setPdfFileName(null);
    } finally {
      setIsPdfExtracting(false);
      // Reset input so the same file can be re-selected
      if (pdfInputRef.current) pdfInputRef.current.value = '';
    }
  };

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
                <div className="flex items-center justify-between mb-1.5">
                  <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                    Project PRD
                  </label>
                  <div>
                    <input
                      ref={pdfInputRef}
                      type="file"
                      accept=".pdf"
                      style={{ display: 'none' }}
                      onChange={handlePdfUpload}
                    />
                    <button
                      type="button"
                      onClick={() => pdfInputRef.current?.click()}
                      disabled={isPdfExtracting}
                      style={{
                        display: 'flex', alignItems: 'center', gap: 4,
                        padding: '3px 8px', borderRadius: 5, fontSize: 11,
                        background: 'rgba(16,185,129,0.08)',
                        border: '1px solid rgba(16,185,129,0.25)',
                        color: isPdfExtracting ? '#475569' : '#6ee7b7',
                        cursor: isPdfExtracting ? 'not-allowed' : 'pointer',
                        fontFamily: 'inherit', whiteSpace: 'nowrap',
                        transition: 'background 0.15s, border-color 0.15s',
                      }}
                      onMouseEnter={e => { if (!isPdfExtracting) (e.currentTarget.style.background = 'rgba(16,185,129,0.15)'); }}
                      onMouseLeave={e => { (e.currentTarget.style.background = 'rgba(16,185,129,0.08)'); }}
                    >
                      {isPdfExtracting ? (
                        <>
                          <svg style={{ width: 11, height: 11, animation: 'spin 1s linear infinite' }} viewBox="0 0 24 24" fill="none">
                            <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" strokeOpacity="0.25"/>
                            <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round"/>
                          </svg>
                          Extracting…
                        </>
                      ) : (
                        <>
                          <svg style={{ width: 11, height: 11 }} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                            <polyline points="14 2 14 8 20 8"/>
                            <line x1="12" y1="18" x2="12" y2="12"/>
                            <line x1="9" y1="15" x2="15" y2="15"/>
                          </svg>
                          Upload PDF
                        </>
                      )}
                    </button>
                  </div>
                </div>
                {pdfFileName && !isPdfExtracting && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6 }}>
                    <svg style={{ width: 11, height: 11, color: '#6ee7b7', flexShrink: 0 }} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                      <polyline points="20 6 9 17 4 12"/>
                    </svg>
                    <span style={{ fontSize: 11, color: '#94a3b8', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {pdfFileName}
                    </span>
                    <button
                      type="button"
                      onClick={() => { setPdfFileName(null); setPrdText(''); }}
                      style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', fontSize: 13, lineHeight: 1, padding: '0 2px', flexShrink: 0 }}
                      title="Clear"
                    >×</button>
                  </div>
                )}
                <textarea
                  value={prdText}
                  onChange={e => { setPrdText(e.target.value); if (pdfFileName) setPdfFileName(null); }}
                  placeholder={isPdfExtracting ? 'Extracting text from PDF…' : `Paste your PRD here…\n\nExample:\n# Project: Learn Quantum Computing\n## Goal\nBuild foundational understanding…\n\n## Core Topics\n- Linear algebra\n- Quantum circuits\n- Key algorithms`}
                  disabled={isPdfExtracting}
                  className="flex-1 bg-slate-950 border border-slate-700 rounded-md p-3 text-sm text-slate-200 font-mono resize-none focus:outline-none focus:border-emerald-600 disabled:opacity-40"
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
            disabled={isGenerating || isPdfExtracting || !canSubmit}
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
