// src/pages/ProjectTreePage.tsx
import { useParams, useNavigate } from 'react-router-dom';
import YggdrasilTree from '../components/YggdrasilTree';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Tree, Project } from '../types';
import { listen } from '@tauri-apps/api/event';
import { exportTreeAsZip } from '../utils/exportTree';

type InputMode = 'prd' | 'github';

interface TreeRegenerationRecord {
  id: string;
  oldTreeId: string | null;
  newTreeId: string | null;
  regeneratedAt: string;
  nodesCarried: number;
  nodesDropped: number;
}

type GapType = 'libraryGap' | 'knowledgeGap' | 'partialKgGap';

interface CheckpointGap {
  nodeId: string;
  title: string;
  description: string;
  phaseName: string;
  skillName: string;
  progress: number;
  isLocked: boolean;
  hasWeakMatches: boolean;
  conceptSlug: string | null;
  searchTerms: string[];
  masteryCriteria: string | null;
  gapType: GapType;
  prerequisiteConcepts: string[];
}

interface TreeResourceGaps {
  treeId: string;
  totalCheckpoints: number;
  greenMatchedCheckpoints: number;
  unmatchedCheckpoints: number;
  coveragePercent: number;
  libraryGapCount: number;
  knowledgeGapCount: number;
  partialKgGapCount: number;
  gaps: CheckpointGap[];
}

export default function ProjectTreePage() {
  const { projectId } = useParams<{ projectId: string }>();
  const navigate = useNavigate();
  const [projectName, setProjectName] = useState<string>('');
  const [inputMode, setInputMode] = useState<InputMode>('prd');
  const [prdText, setPrdText] = useState<string>('');
  const [pdfFileName, setPdfFileName] = useState<string | null>(null);
  const [isPdfExtracting, setIsPdfExtracting] = useState(false);
  const [githubUrl, setGithubUrl] = useState<string>('');
  const [paperUrl, setPaperUrl] = useState<string>('');
  const [paperPdfBase64, setPaperPdfBase64] = useState<string | null>(null);
  const [paperFileName, setPaperFileName] = useState<string | null>(null);
  const [isPaperExtracting, setIsPaperExtracting] = useState(false);
  const [isGenerating, setIsGenerating] = useState(false);
  const [treeKey, setTreeKey] = useState(0);
  const [prdOpen, setPrdOpen] = useState(true);
  const [hasTree, setHasTree] = useState(false);
  const [isExporting, setIsExporting] = useState(false);
  const [treeVersion, setTreeVersion] = useState<number | null>(null);
  const [activeTreeId, setActiveTreeId] = useState<string | null>(null);
  const [isRegenerating, setIsRegenerating] = useState(false);
  const [showRegenConfirm, setShowRegenConfirm] = useState(false);
  const [regenToast, setRegenToast] = useState<{ carried: number; dropped: number } | null>(null);
  const [showVersionHistory, setShowVersionHistory] = useState(false);
  const [versionHistory, setVersionHistory] = useState<TreeRegenerationRecord[]>([]);
  const versionBadgeRef = useRef<HTMLButtonElement>(null);
  const [showCoveragePanel, setShowCoveragePanel] = useState(false);
  const [coverageGaps, setCoverageGaps] = useState<TreeResourceGaps | null>(null);
  const [isCoverageLoading, setIsCoverageLoading] = useState(false);
  const [matchingNodeId, setMatchingNodeId] = useState<string | null>(null);
  const [expandedGapId, setExpandedGapId] = useState<string | null>(null);
  const [isInferringDeps, setIsInferringDeps] = useState(false);
  const [isBackfillingEmbeddings, setIsBackfillingEmbeddings] = useState(false);
  const [backfillToast, setBackfillToast] = useState<string | null>(null);
  const [externalSelectedNodeId, setExternalSelectedNodeId] = useState<string | null>(null);
  const pdfInputRef = React.useRef<HTMLInputElement>(null);
  const paperInputRef = React.useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (projectId) {
      invoke<{ treeId: string; treeCount: number } | null>(
        'get_active_tree_for_project',
        { projectId }
      ).then(async summary => {
        setHasTree(summary !== null);
        if (summary?.treeId) {
          setActiveTreeId(summary.treeId);
          // Load version from trees table
          try {
            const trees = await invoke<Tree[]>('get_trees', { projectId });
            const active = trees.find(t => t.id === summary.treeId);
            if (active) setTreeVersion(active.version ?? 1);
          } catch { /* non-fatal */ }
        }
      }).catch(() => {
        invoke<Tree[]>('get_trees', { projectId }).then(trees => setHasTree(trees.length > 0));
      });

      invoke<Project[]>('get_projects').then(projects => {
        const project = projects.find(p => p.id === projectId);
        if (project) setProjectName(project.name);
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

      const data = await invoke<{ text: string; pages: number; chars: number }>('extract_pdf_text', { pdfBase64: base64 });
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

  const handlePaperPdfUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    setIsPaperExtracting(true);
    setPaperFileName(file.name);
    setPaperUrl('');

    try {
      const arrayBuffer = await file.arrayBuffer();
      const bytes = new Uint8Array(arrayBuffer);
      let binary = '';
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
      setPaperPdfBase64(btoa(binary));
    } catch (err) {
      alert(`Paper upload failed: ${err}`);
      setPaperFileName(null);
      setPaperPdfBase64(null);
    } finally {
      setIsPaperExtracting(false);
      if (paperInputRef.current) paperInputRef.current.value = '';
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
        treeJson = await invoke<string>('analyze_repo', {
          projectId,
          githubUrl,
          paperUrl: paperUrl.trim() || null,
          paperPdf: paperPdfBase64,
        });
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

  const handleRegenerate = async () => {
    if (!projectId || isRegenerating) return;
    setShowRegenConfirm(false);
    setIsRegenerating(true);
    try {
      const result = await invoke<{ treeId: string; version: number; nodesCarried: number; nodesDropped: number }>(
        'regenerate_tree',
        {
          projectId,
          useGithub: inputMode === 'github',
          githubUrl: inputMode === 'github' ? githubUrl : null,
          prdText: inputMode === 'prd' ? prdText : null,
          paperUrl: paperUrl.trim() || null,
          paperPdf: paperPdfBase64,
        }
      );
      setTreeKey(k => k + 1);
      setPrdOpen(false);
      setRegenToast({ carried: result.nodesCarried, dropped: result.nodesDropped });
      setTimeout(() => setRegenToast(null), 6000);
    } catch (error) {
      alert(`Regeneration failed: ${error}`);
    } finally {
      setIsRegenerating(false);
    }
  };

  const handleShowVersionHistory = async () => {
    if (!activeTreeId) return;
    setShowVersionHistory(v => !v);
    if (!showVersionHistory && activeTreeId) {
      try {
        const records = await invoke<TreeRegenerationRecord[]>('get_tree_regenerations', { treeId: activeTreeId });
        setVersionHistory(records);
      } catch { /* non-fatal */ }
    }
  };

  // Listen for ygg-open-coverage from Mimir suggestion chips
  useEffect(() => {
    if (!activeTreeId) return;
    let unlisten: (() => void) | undefined;
    listen<{ treeId: string | null }>('ygg-open-coverage', (event) => {
      const tid = event.payload?.treeId ?? activeTreeId;
      if (tid !== activeTreeId) return;
      setShowCoveragePanel(true);
      loadCoverageGaps(activeTreeId);
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [activeTreeId]); // loadCoverageGaps excluded intentionally — stable ref

  const loadCoverageGaps = useCallback(async (treeId: string) => {
    setIsCoverageLoading(true);
    try {
      const gaps = await invoke<TreeResourceGaps>('get_tree_resource_gaps', { treeId });
      setCoverageGaps(gaps);
    } catch { /* non-fatal */ }
    finally { setIsCoverageLoading(false); }
  }, []);

  const handleOpenCoverage = useCallback(async () => {
    if (!activeTreeId) return;
    setShowCoveragePanel(true);
    await loadCoverageGaps(activeTreeId);
  }, [activeTreeId, loadCoverageGaps]);

  const handleFindMatches = useCallback(async (nodeId: string) => {
    if (!activeTreeId) return;
    setMatchingNodeId(nodeId);
    try {
      await invoke('match_node_to_resources', { nodeId, treeId: activeTreeId });
      await loadCoverageGaps(activeTreeId);
    } catch { /* non-fatal */ }
    finally { setMatchingNodeId(null); }
  }, [activeTreeId, loadCoverageGaps]);

  const handleInferDeps = useCallback(async () => {
    setIsInferringDeps(true);
    try {
      await invoke('enqueue_infer_deps');
      // Reload gaps after a short delay to pick up any newly inferred edges
      setTimeout(() => {
        if (activeTreeId) loadCoverageGaps(activeTreeId);
        setIsInferringDeps(false);
      }, 2500);
    } catch {
      setIsInferringDeps(false);
    }
  }, [activeTreeId, loadCoverageGaps]);

  const handleBackfillEmbeddings = useCallback(async () => {
    setIsBackfillingEmbeddings(true);
    try {
      const count = await invoke<number>('backfill_node_embeddings');
      setBackfillToast(count === 0 ? 'All nodes already embedded' : `Embedded ${count} node${count !== 1 ? 's' : ''}`);
      setTimeout(() => setBackfillToast(null), 3000);
    } catch (e) {
      setBackfillToast('Backfill failed');
      setTimeout(() => setBackfillToast(null), 3000);
    } finally {
      setIsBackfillingEmbeddings(false);
    }
  }, []);

  const handleFindAllMatches = useCallback(async () => {
    if (!activeTreeId) return;
    setMatchingNodeId('all');
    try {
      await invoke('enqueue_rematch', { treeId: activeTreeId });
      const unlisten = await listen('ygg-rematch-complete', async () => {
        unlisten();
        setMatchingNodeId(null);
        await loadCoverageGaps(activeTreeId);
      });
    } catch {
      setMatchingNodeId(null);
    }
  }, [activeTreeId, loadCoverageGaps]);

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

                <div className="flex items-center justify-between mt-4 mb-1.5">
                  <label className="text-xs font-medium text-slate-400 uppercase tracking-wider">
                    Paper <span className="text-slate-600 normal-case tracking-normal">(optional)</span>
                  </label>
                  <div>
                    <input
                      ref={paperInputRef}
                      type="file"
                      accept=".pdf"
                      style={{ display: 'none' }}
                      onChange={handlePaperPdfUpload}
                    />
                    <button
                      type="button"
                      onClick={() => paperInputRef.current?.click()}
                      disabled={isPaperExtracting}
                      style={{
                        display: 'flex', alignItems: 'center', gap: 4,
                        padding: '3px 8px', borderRadius: 5, fontSize: 11,
                        background: 'rgba(16,185,129,0.08)',
                        border: '1px solid rgba(16,185,129,0.25)',
                        color: isPaperExtracting ? '#475569' : '#6ee7b7',
                        cursor: isPaperExtracting ? 'not-allowed' : 'pointer',
                        fontFamily: 'inherit', whiteSpace: 'nowrap',
                        transition: 'background 0.15s, border-color 0.15s',
                      }}
                      onMouseEnter={e => { if (!isPaperExtracting) (e.currentTarget.style.background = 'rgba(16,185,129,0.15)'); }}
                      onMouseLeave={e => { (e.currentTarget.style.background = 'rgba(16,185,129,0.08)'); }}
                    >
                      <svg style={{ width: 11, height: 11 }} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                        <polyline points="14 2 14 8 20 8"/>
                      </svg>
                      Upload PDF
                    </button>
                  </div>
                </div>
                <input
                  type="url"
                  value={paperUrl}
                  onChange={e => {
                    setPaperUrl(e.target.value);
                    if (paperFileName) { setPaperFileName(null); setPaperPdfBase64(null); }
                  }}
                  placeholder="https://arxiv.org/abs/… or paper URL"
                  disabled={!!paperFileName}
                  className="bg-slate-950 border border-slate-700 rounded-md p-3 text-sm text-slate-200 focus:outline-none focus:border-emerald-600 disabled:opacity-40"
                />
                {paperFileName && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 6 }}>
                    <svg style={{ width: 11, height: 11, color: '#6ee7b7', flexShrink: 0 }} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                      <polyline points="20 6 9 17 4 12"/>
                    </svg>
                    <span style={{ fontSize: 11, color: '#94a3b8', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {paperFileName}
                    </span>
                    <button
                      type="button"
                      onClick={() => { setPaperFileName(null); setPaperPdfBase64(null); }}
                      style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', fontSize: 13, lineHeight: 1, padding: '0 2px', flexShrink: 0 }}
                      title="Clear"
                    >×</button>
                  </div>
                )}

                <p className="text-xs text-slate-600 mt-2">
                  Fetches README + dependency files to generate a learning tree of concepts you used.
                  Add <code className="text-slate-500">GITHUB_TOKEN</code> to <code className="text-slate-500">.env</code> for private repos.
                  Adding a paper grounds the tree in its theoretical context.
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
          {hasTree && (
            <button
              onClick={showCoveragePanel ? () => setShowCoveragePanel(false) : handleOpenCoverage}
              style={{
                ...floatBtn,
                color: showCoveragePanel ? '#6ee7b7' : '#64748b',
                borderColor: showCoveragePanel ? '#1d4e3a' : '#1e293b',
              }}
              onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8'; e.currentTarget.style.borderColor = '#334155'; }}
              onMouseLeave={e => {
                e.currentTarget.style.color = showCoveragePanel ? '#6ee7b7' : '#64748b';
                e.currentTarget.style.borderColor = showCoveragePanel ? '#1d4e3a' : '#1e293b';
              }}
              title="Show resource coverage for this tree"
            >Coverage</button>
          )}
          {hasTree && (
            <button
              onClick={() => setShowRegenConfirm(true)}
              disabled={isRegenerating || !canSubmit}
              style={{ ...floatBtn, opacity: (isRegenerating || !canSubmit) ? 0.45 : 1, color: isRegenerating ? '#64748b' : '#64748b' }}
              onMouseEnter={e => { if (!isRegenerating && canSubmit) { e.currentTarget.style.color = '#94a3b8'; e.currentTarget.style.borderColor = '#334155'; } }}
              onMouseLeave={e => { e.currentTarget.style.color = '#64748b'; e.currentTarget.style.borderColor = '#1e293b'; }}
              title="Regenerate tree, carrying over your notes and progress"
            >
              {isRegenerating ? (
                <>
                  <svg style={{ width: 11, height: 11, animation: 'spin 1s linear infinite' }} viewBox="0 0 24 24" fill="none">
                    <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" strokeOpacity="0.25"/>
                    <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round"/>
                  </svg>
                  Regenerating…
                </>
              ) : '↺ Regenerate'}
            </button>
          )}
          {hasTree && treeVersion != null && (
            <div style={{ position: 'relative' }}>
              <button
                ref={versionBadgeRef}
                onClick={handleShowVersionHistory}
                style={{ ...floatBtn, color: '#475569', fontSize: 11 }}
                onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8'; e.currentTarget.style.borderColor = '#334155'; }}
                onMouseLeave={e => { e.currentTarget.style.color = '#475569'; e.currentTarget.style.borderColor = '#1e293b'; }}
                title="View regeneration history"
              >v{treeVersion}</button>
              {showVersionHistory && (
                <div style={{
                  position: 'absolute', top: '100%', left: 0, marginTop: 6, zIndex: 50,
                  background: 'rgba(15, 23, 42, 0.97)', border: '1px solid #1e293b',
                  borderRadius: 8, padding: 12, minWidth: 260, boxShadow: '0 8px 24px rgba(0,0,0,0.6)',
                }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: '#64748b', marginBottom: 8, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                    Regeneration History
                  </div>
                  {versionHistory.length === 0 ? (
                    <div style={{ fontSize: 12, color: '#475569' }}>No regenerations yet.</div>
                  ) : versionHistory.map(r => (
                    <div key={r.id} style={{ marginBottom: 8, paddingBottom: 8, borderBottom: '1px solid #1e293b' }}>
                      <div style={{ fontSize: 12, color: '#94a3b8' }}>
                        {new Date(r.regeneratedAt).toLocaleString()}
                      </div>
                      <div style={{ fontSize: 11, color: '#6ee7b7', marginTop: 2 }}>
                        {r.nodesCarried} carried
                        {r.nodesDropped > 0 && (
                          <span style={{ color: '#f59e0b', marginLeft: 6 }}>{r.nodesDropped} lost</span>
                        )}
                      </div>
                    </div>
                  ))}
                  <button
                    onClick={() => setShowVersionHistory(false)}
                    style={{ marginTop: 4, fontSize: 11, color: '#475569', background: 'none', border: 'none', cursor: 'pointer', padding: 0 }}
                  >Close</button>
                </div>
              )}
            </div>
          )}
        </div>

        <YggdrasilTree key={treeKey} projectId={projectId} externalSelectedNodeId={externalSelectedNodeId} />

        {/* Coverage side panel */}
        {showCoveragePanel && (
          <div style={{
            position: 'absolute', top: 0, right: 0, bottom: 0, width: 300,
            background: 'rgba(10, 15, 30, 0.97)', borderLeft: '1px solid #1e293b',
            zIndex: 40, display: 'flex', flexDirection: 'column', overflow: 'hidden',
          }}>
            {/* Panel header */}
            <div style={{ padding: '12px 14px 10px', borderBottom: '1px solid #1e293b', flexShrink: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: '#94a3b8', textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                  Resource Coverage
                </span>
                <button
                  onClick={() => setShowCoveragePanel(false)}
                  style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', fontSize: 16, lineHeight: 1, padding: '0 2px' }}
                >×</button>
              </div>

              {isCoverageLoading && !coverageGaps ? (
                <div style={{ fontSize: 11, color: '#475569' }}>Loading…</div>
              ) : coverageGaps ? (() => {
                const pct = coverageGaps.coveragePercent;
                const barColor = pct >= 70 ? '#10b981' : pct >= 40 ? '#f59e0b' : '#ef4444';
                const textColor = pct >= 70 ? '#6ee7b7' : pct >= 40 ? '#fcd34d' : '#fca5a5';
                return (
                  <>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
                      <span style={{ fontSize: 11, color: '#64748b' }}>
                        {coverageGaps.greenMatchedCheckpoints}/{coverageGaps.totalCheckpoints} covered
                      </span>
                      <span style={{ fontSize: 12, fontWeight: 600, color: textColor }}>
                        {Math.round(pct)}%
                      </span>
                    </div>
                    <div style={{ height: 5, borderRadius: 3, background: '#1e293b', overflow: 'hidden' }}>
                      <div style={{ width: `${pct}%`, height: '100%', background: barColor, borderRadius: 3, transition: 'width 0.4s ease' }} />
                    </div>
                    {/* Gap type breakdown */}
                    {coverageGaps.unmatchedCheckpoints > 0 && (
                      <div style={{ display: 'flex', gap: 8, marginTop: 7, flexWrap: 'wrap' }}>
                        {coverageGaps.libraryGapCount > 0 && (
                          <span style={{ fontSize: 9, color: '#60a5fa', display: 'flex', alignItems: 'center', gap: 3 }}>
                            <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#3b82f6', display: 'inline-block' }} />
                            {coverageGaps.libraryGapCount} missing resource
                          </span>
                        )}
                        {coverageGaps.partialKgGapCount > 0 && (
                          <span style={{ fontSize: 9, color: '#fcd34d', display: 'flex', alignItems: 'center', gap: 3 }}>
                            <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#f59e0b', display: 'inline-block' }} />
                            {coverageGaps.partialKgGapCount} weak coverage
                          </span>
                        )}
                        {coverageGaps.knowledgeGapCount > 0 && (
                          <span style={{ fontSize: 9, color: '#fca5a5', display: 'flex', alignItems: 'center', gap: 3 }}>
                            <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#ef4444', display: 'inline-block' }} />
                            {coverageGaps.knowledgeGapCount} unknown territory
                          </span>
                        )}
                      </div>
                    )}
                    <div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
                      <button
                        onClick={handleFindAllMatches}
                        disabled={matchingNodeId === 'all'}
                        style={{
                          flex: 1, padding: '5px 0', borderRadius: 5, fontSize: 11,
                          background: matchingNodeId === 'all' ? 'rgba(16,185,129,0.05)' : 'rgba(16,185,129,0.1)',
                          border: '1px solid rgba(16,185,129,0.25)',
                          color: matchingNodeId === 'all' ? '#475569' : '#6ee7b7',
                          cursor: matchingNodeId === 'all' ? 'not-allowed' : 'pointer',
                          fontFamily: 'inherit', transition: 'background 0.15s',
                        }}
                      >
                        {matchingNodeId === 'all' ? 'Matching…' : 'Find all matches'}
                      </button>
                      <button
                        onClick={handleBackfillEmbeddings}
                        disabled={isBackfillingEmbeddings}
                        title="Cache node title embeddings for fast async matching"
                        style={{
                          padding: '5px 9px', borderRadius: 5, fontSize: 11,
                          background: 'rgba(99,102,241,0.08)', border: '1px solid rgba(99,102,241,0.2)',
                          color: isBackfillingEmbeddings ? '#475569' : '#a5b4fc',
                          cursor: isBackfillingEmbeddings ? 'not-allowed' : 'pointer',
                          fontFamily: 'inherit',
                        }}
                      >{isBackfillingEmbeddings ? '…' : 'Backfill'}</button>
                      <button
                        onClick={() => activeTreeId && loadCoverageGaps(activeTreeId)}
                        disabled={isCoverageLoading}
                        style={{
                          padding: '5px 9px', borderRadius: 5, fontSize: 11,
                          background: 'rgba(30,41,59,0.5)', border: '1px solid #1e293b',
                          color: isCoverageLoading ? '#475569' : '#64748b',
                          cursor: isCoverageLoading ? 'not-allowed' : 'pointer',
                          fontFamily: 'inherit',
                        }}
                        title="Refresh"
                      >↻</button>
                    </div>
                  </>
                );
              })() : null}
            </div>

            {/* Gap list */}
            <div style={{ flex: 1, overflowY: 'auto', padding: '8px 0' }}>
              {!coverageGaps || coverageGaps.gaps.length === 0 ? (
                <div style={{ padding: '24px 14px', textAlign: 'center', fontSize: 12, color: '#475569' }}>
                  {coverageGaps ? '🎉 All checkpoints have matched resources.' : ''}
                </div>
              ) : (() => {
                // Group by phase
                const byPhase = new Map<string, CheckpointGap[]>();
                for (const gap of coverageGaps.gaps) {
                  if (!byPhase.has(gap.phaseName)) byPhase.set(gap.phaseName, []);
                  byPhase.get(gap.phaseName)!.push(gap);
                }
                return Array.from(byPhase.entries()).map(([phase, gaps]) => (
                  <div key={phase}>
                    <div style={{
                      padding: '6px 14px 4px', fontSize: 10, fontWeight: 600,
                      color: '#475569', textTransform: 'uppercase', letterSpacing: '0.06em',
                    }}>{phase}</div>
                    {gaps.map(gap => {
                      const isExpanded = expandedGapId === gap.nodeId;
                      return (
                        <div
                          key={gap.nodeId}
                          style={{
                            padding: '8px 14px',
                            borderBottom: '1px solid rgba(30,41,59,0.5)',
                            cursor: 'pointer',
                            background: externalSelectedNodeId === gap.nodeId ? 'rgba(16,185,129,0.07)' : 'transparent',
                            transition: 'background 0.12s',
                          }}
                          onClick={() => setExternalSelectedNodeId(gap.nodeId)}
                          onMouseEnter={e => { if (externalSelectedNodeId !== gap.nodeId) (e.currentTarget as HTMLDivElement).style.background = 'rgba(30,41,59,0.4)'; }}
                          onMouseLeave={e => { (e.currentTarget as HTMLDivElement).style.background = externalSelectedNodeId === gap.nodeId ? 'rgba(16,185,129,0.07)' : 'transparent'; }}
                        >
                          {/* Per-type indicator color and label */}
                          {(() => {
                            const typeStyle = gap.gapType === 'libraryGap'
                              ? { dot: '#3b82f6', label: 'Missing resource', labelColor: '#93c5fd' }
                              : gap.gapType === 'knowledgeGap'
                              ? { dot: '#ef4444', label: 'Unknown territory', labelColor: '#fca5a5' }
                              : { dot: '#f59e0b', label: 'Weak coverage', labelColor: '#fcd34d' };

                            return (
                              <>
                                {/* Row 1: title + action buttons */}
                                <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 6 }}>
                                  <div style={{ flex: 1, minWidth: 0 }}>
                                    <div style={{ fontSize: 12, color: gap.isLocked ? '#475569' : '#e2e8f0', fontWeight: 500, lineHeight: 1.3, marginBottom: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                      {gap.isLocked && <span style={{ marginRight: 4, fontSize: 10 }}>🔒</span>}
                                      {gap.title}
                                    </div>
                                    <div style={{ fontSize: 10, color: '#475569', display: 'flex', alignItems: 'center', gap: 5 }}>
                                      <span>{gap.skillName}</span>
                                      {gap.progress > 0 && <span style={{ color: '#64748b' }}>{gap.progress}%</span>}
                                      <span style={{ width: 5, height: 5, borderRadius: '50%', background: typeStyle.dot, display: 'inline-block', flexShrink: 0 }} />
                                      <span style={{ color: typeStyle.labelColor, fontSize: 9 }}>{typeStyle.label}</span>
                                    </div>
                                  </div>
                                  {/* Action buttons */}
                                  <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
                                    {(gap.gapType === 'libraryGap' || gap.gapType === 'partialKgGap') && (
                                      <button
                                        onClick={e => { e.stopPropagation(); handleFindMatches(gap.nodeId); }}
                                        disabled={matchingNodeId === gap.nodeId}
                                        style={{
                                          padding: '3px 7px', borderRadius: 4, fontSize: 10,
                                          background: 'rgba(16,185,129,0.08)', border: '1px solid rgba(16,185,129,0.2)',
                                          color: matchingNodeId === gap.nodeId ? '#475569' : '#6ee7b7',
                                          cursor: matchingNodeId === gap.nodeId ? 'not-allowed' : 'pointer',
                                          fontFamily: 'inherit',
                                        }}
                                      >
                                        {matchingNodeId === gap.nodeId ? '…' : 'Find'}
                                      </button>
                                    )}
                                    {(gap.gapType === 'knowledgeGap' || gap.gapType === 'partialKgGap') && (
                                      <button
                                        onClick={e => { e.stopPropagation(); handleInferDeps(); }}
                                        disabled={isInferringDeps}
                                        title="Infer skill dependencies from your knowledge graph"
                                        style={{
                                          padding: '3px 7px', borderRadius: 4, fontSize: 10,
                                          background: 'rgba(245,158,11,0.08)', border: '1px solid rgba(245,158,11,0.2)',
                                          color: isInferringDeps ? '#475569' : '#fcd34d',
                                          cursor: isInferringDeps ? 'not-allowed' : 'pointer',
                                          fontFamily: 'inherit', whiteSpace: 'nowrap',
                                        }}
                                      >
                                        {isInferringDeps ? '…' : 'Infer deps'}
                                      </button>
                                    )}
                                  </div>
                                </div>

                                {/* Guidance message */}
                                <div style={{ fontSize: 9, color: '#475569', marginTop: 3, lineHeight: 1.4 }}>
                                  {gap.gapType === 'libraryGap' && 'Your library is missing coverage for this concept. Search for:'}
                                  {gap.gapType === 'knowledgeGap' && 'No knowledge graph entry yet. Run "Infer deps" to map prerequisites, then search for:'}
                                  {gap.gapType === 'partialKgGap' && 'Concept is in your knowledge graph but has no mapped prerequisites yet.'}
                                </div>

                                {/* Prerequisite concepts (LibraryGap only) */}
                                {gap.gapType === 'libraryGap' && gap.prerequisiteConcepts.length > 0 && (
                                  <div style={{ fontSize: 9, color: '#64748b', marginTop: 2 }}>
                                    Requires: {gap.prerequisiteConcepts.join(', ')}
                                  </div>
                                )}

                                {/* Search term chips */}
                                {gap.searchTerms.length > 0 && (
                                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginTop: 5 }}>
                                    {gap.searchTerms.map(term => (
                                      <button
                                        key={term}
                                        onClick={e => { e.stopPropagation(); navigate(`/resources?q=${encodeURIComponent(term)}`); }}
                                        style={{
                                          padding: '2px 7px', borderRadius: 10, fontSize: 9,
                                          background: gap.gapType === 'libraryGap'
                                            ? 'rgba(59,130,246,0.1)' : gap.gapType === 'knowledgeGap'
                                            ? 'rgba(239,68,68,0.1)' : 'rgba(245,158,11,0.1)',
                                          border: gap.gapType === 'libraryGap'
                                            ? '1px solid rgba(59,130,246,0.25)' : gap.gapType === 'knowledgeGap'
                                            ? '1px solid rgba(239,68,68,0.25)' : '1px solid rgba(245,158,11,0.25)',
                                          color: typeStyle.labelColor,
                                          cursor: 'pointer', fontFamily: 'inherit', whiteSpace: 'nowrap',
                                        }}
                                      >
                                        {term}
                                      </button>
                                    ))}
                                  </div>
                                )}

                                {/* Mastery criteria collapsible */}
                                {gap.masteryCriteria && (
                                  <div style={{ marginTop: 5 }}>
                                    <button
                                      onClick={e => { e.stopPropagation(); setExpandedGapId(isExpanded ? null : gap.nodeId); }}
                                      style={{
                                        background: 'none', border: 'none', padding: 0, cursor: 'pointer',
                                        fontSize: 9, color: '#475569', fontFamily: 'inherit',
                                        display: 'flex', alignItems: 'center', gap: 3,
                                      }}
                                    >
                                      <span style={{ transform: isExpanded ? 'rotate(90deg)' : 'rotate(0deg)', display: 'inline-block', transition: 'transform 0.15s' }}>▶</span>
                                      Mastery criteria
                                    </button>
                                    {isExpanded && (
                                      <div style={{
                                        marginTop: 4, padding: '5px 8px',
                                        background: 'rgba(15,23,42,0.6)', borderRadius: 4,
                                        border: '1px solid rgba(30,41,59,0.8)',
                                        fontSize: 10, color: '#94a3b8', lineHeight: 1.5,
                                      }}>
                                        {gap.masteryCriteria}
                                      </div>
                                    )}
                                  </div>
                                )}
                              </>
                            );
                          })()}
                        </div>
                      );
                    })}
                  </div>
                ));
              })()}
            </div>
          </div>
        )}

        {/* Regenerate confirmation dialog */}
        {showRegenConfirm && (
          <div style={{
            position: 'absolute', inset: 0, zIndex: 100,
            background: 'rgba(1,2,8,0.75)', backdropFilter: 'blur(4px)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <div style={{
              background: '#0f172a', border: '1px solid #1e293b', borderRadius: 10,
              padding: 24, maxWidth: 360, width: '90%',
              boxShadow: '0 16px 48px rgba(0,0,0,0.8)',
            }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: '#e2e8f0', marginBottom: 8 }}>
                Regenerate tree?
              </div>
              <div style={{ fontSize: 12, color: '#94a3b8', lineHeight: 1.6, marginBottom: 20 }}>
                A new tree will be generated from your current {inputMode === 'github' ? 'GitHub repo' : 'PRD'}.
                Your notes and checkpoint progress will be carried over where possible.
                The old tree will be archived.
              </div>
              <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                <button
                  onClick={() => setShowRegenConfirm(false)}
                  style={{
                    padding: '6px 14px', borderRadius: 6, fontSize: 12,
                    background: 'transparent', border: '1px solid #334155',
                    color: '#64748b', cursor: 'pointer', fontFamily: 'inherit',
                  }}
                >Cancel</button>
                <button
                  onClick={handleRegenerate}
                  style={{
                    padding: '6px 14px', borderRadius: 6, fontSize: 12,
                    background: '#059669', border: '1px solid #10b981',
                    color: '#fff', cursor: 'pointer', fontFamily: 'inherit', fontWeight: 500,
                  }}
                >Regenerate</button>
              </div>
            </div>
          </div>
        )}

        {/* Post-regeneration toast */}
        {regenToast && (
          <div style={{
            position: 'absolute', bottom: 20, left: '50%', transform: 'translateX(-50%)',
            zIndex: 80, background: 'rgba(15,23,42,0.96)', border: '1px solid #1e293b',
            borderRadius: 8, padding: '10px 18px', display: 'flex', alignItems: 'center', gap: 10,
            boxShadow: '0 8px 24px rgba(0,0,0,0.6)', fontSize: 12, color: '#e2e8f0',
            whiteSpace: 'nowrap',
          }}>
            <svg style={{ width: 14, height: 14, color: '#6ee7b7', flexShrink: 0 }} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="20 6 9 17 4 12"/>
            </svg>
            Tree updated —{' '}
            <span style={{ color: '#6ee7b7' }}>{regenToast.carried} checkpoints carried over</span>
            {regenToast.dropped > 0 && (
              <span style={{ color: '#f59e0b' }}>, {regenToast.dropped} lost</span>
            )}
            <button
              onClick={() => setRegenToast(null)}
              style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', fontSize: 14, lineHeight: 1, padding: '0 2px', marginLeft: 4 }}
            >×</button>
          </div>
        )}

        {/* Backfill embeddings toast */}
        {backfillToast && (
          <div style={{
            position: 'absolute', bottom: 20, right: showCoveragePanel ? 316 : 16,
            zIndex: 80, background: 'rgba(15,23,42,0.96)', border: '1px solid #312e81',
            borderRadius: 8, padding: '8px 14px', fontSize: 12, color: '#a5b4fc',
            display: 'flex', alignItems: 'center', gap: 8,
          }}>
            {backfillToast}
          </div>
        )}
      </main>
    </div>
  );
}
