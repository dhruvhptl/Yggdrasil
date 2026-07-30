// src/pages/ResourcesPage.tsx
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Trash2, Link, FileText, Globe, BookOpen, X, RefreshCw, CheckCircle2, Minus, XCircle, Filter, Search, ChevronDown, ChevronRight, ChevronUp, Play, Tag, Wand2, Check, Map as MapIcon, BookMarked, Loader2 } from 'lucide-react';
import { MimirResource, TranscriptJobStats, ResourceStudyMap, StudyMapEntry, StudyMapSection } from '../types';
import { validateOrLog, MimirResourceSchema } from '../lib/validators';
import { log } from '../lib/logger';
import { z } from 'zod';

type AddMode = 'url' | 'text' | 'pdf';

const TYPE_ICON: Record<string, React.ReactElement> = {
  webpage: <Globe className="w-3.5 h-3.5" />,
  text: <FileText className="w-3.5 h-3.5" />,
  pdf: <BookOpen className="w-3.5 h-3.5" />,
};

const MODE_LABEL: Record<AddMode, string> = {
  url: 'URL',
  text: 'Paste Text',
  pdf: 'Upload PDF',
};

interface RescrapeResult {
  success: boolean;
  changed: boolean;
  oldChunks: number;
  newChunks: number;
  message: string;
}

interface BulkProgress {
  resourceId: string;
  title: string;
  status: 'improved' | 'unchanged' | 'failed';
  oldChunks: number;
  newChunks: number;
}

interface PlaylistVideoItem {
  videoId: string;
  title: string;
  url: string;
  position: number;
  checked: boolean;
}

interface PlaylistModalData {
  playlistTitle: string;
  playlistId: string;
  playlistUrl: string;
  videos: PlaylistVideoItem[];
}

interface IngestResult {
  id: string;
  title: string;
  pageType: string;
  externalLinks: { url: string; text: string; type: string }[];
}

interface ExternalLinkItem {
  url: string;
  text: string;
  type: string;
  checked: boolean;
}

type CardState = 'idle' | 'loading' | 'done' | 'unchanged' | 'error';

function getChunkBadgeProps(n: number): { label: string; className: string; title?: string } {
  if (n === 0) return {
    label: 'No content',
    className: 'bg-red-950/60 text-red-400 border border-red-800/50',
  };
  if (n <= 3) return {
    label: `${n} chunk${n !== 1 ? 's' : ''}`,
    className: 'bg-amber-950/60 text-amber-400 border border-amber-800/50',
    title: 'May be incomplete — consider re-scraping or finding a deeper URL',
  };
  if (n <= 9) return {
    label: `${n} chunks`,
    className: 'bg-slate-800 text-slate-500 border border-slate-700/50',
  };
  return {
    label: `${n} chunks`,
    className: 'bg-emerald-950/40 text-emerald-500 border border-emerald-800/40',
  };
}

function getYouTubeBadgeProps(n: number): { label: string; className: string } {
  if (n === 0) return { label: 'No transcript', className: 'bg-red-950/60 text-red-400 border border-red-800/50' };
  if (n < 4) return { label: `${n} chunk${n !== 1 ? 's' : ''}`, className: 'bg-amber-950/60 text-amber-400 border border-amber-800/50' };
  return { label: `${n} chunks`, className: 'bg-emerald-950/40 text-emerald-500 border border-emerald-800/40' };
}

function RelevanceDot({ tier }: { tier: 'green' | 'amber' | 'grey' }) {
  const colors = { green: 'bg-emerald-400', amber: 'bg-amber-400', grey: 'bg-slate-500' };
  return <span className={`inline-block w-2 h-2 rounded-full flex-shrink-0 ${colors[tier]}`} />;
}

function StudyMapPanel({
  studyMap,
  loading,
  frontier,
  onFrontierChange,
  treeFilter,
  onTreeFilterChange,
  expandedNodes,
  onToggleNodes,
}: {
  studyMap: ResourceStudyMap | null;
  loading: boolean;
  frontier: boolean;
  onFrontierChange: (v: boolean) => void;
  treeFilter: string[];
  onTreeFilterChange: (ids: string[]) => void;
  expandedNodes: Set<string>;
  onToggleNodes: (id: string) => void;
}) {
  // readProgress maps resourceId -> Set of completed section titles
  const [readProgress, setReadProgress] = useState<Map<string, Set<string>>>(new Map());
  const [markingSection, setMarkingSection] = useState<string | null>(null); // "resourceId|sectionTitle"

  const allProjects = useMemo(() => {
    if (!studyMap) return [];
    const seen = new Map<string, string>();
    for (const entry of studyMap.entries) {
      for (const tb of entry.treeBreakdown) {
        if (!seen.has(tb.projectId)) seen.set(tb.projectId, tb.projectName);
      }
    }
    return [...seen.entries()].map(([projectId, projectName]) => ({ projectId, projectName }));
  }, [studyMap]);

  // Load reading progress when a resource is expanded
  async function loadReadProgress(resourceId: string) {
    try {
      const sections = await invoke<string[]>('get_reading_progress', { resourceId });
      setReadProgress(prev => {
        const next = new Map(prev);
        next.set(resourceId, new Set(sections));
        return next;
      });
    } catch (e) {
      console.error('get_reading_progress failed', e);
    }
  }

  function handleToggleNodes(resourceId: string) {
    onToggleNodes(resourceId);
    if (!expandedNodes.has(resourceId)) {
      loadReadProgress(resourceId);
    }
  }

  async function handleMarkRead(resourceId: string, section: StudyMapSection) {
    const key = `${resourceId}|${section.sectionTitle}`;
    setMarkingSection(key);
    try {
      await invoke('mark_section_read', {
        resourceId,
        sectionTitle: section.sectionTitle,
        pageStart: section.pageStart ?? null,
        pageEnd: section.pageEnd ?? null,
      });
      await loadReadProgress(resourceId);
    } catch (e) {
      console.error('mark_section_read failed', e);
    } finally {
      setMarkingSection(null);
    }
  }

  async function handleMarkUpTo(resourceId: string, section: StudyMapSection) {
    const key = `${resourceId}|${section.sectionTitle}|up_to`;
    setMarkingSection(key);
    try {
      await invoke('mark_sections_read_up_to', {
        resourceId,
        sectionTitle: section.sectionTitle,
        pageStart: section.pageStart ?? null,
      });
      await loadReadProgress(resourceId);
    } catch (e) {
      console.error('mark_sections_read_up_to failed', e);
    } finally {
      setMarkingSection(null);
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-16 text-slate-500">
        <Loader2 className="w-5 h-5 animate-spin mr-2" />
        Building study map…
      </div>
    );
  }

  if (!studyMap) return null;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3 flex-wrap">
        {allProjects.length > 1 && (
          <div className="flex items-center gap-1.5 flex-wrap">
            {allProjects.map(({ projectId, projectName }) => {
              const active = treeFilter.includes(projectId);
              return (
                <button
                  key={projectId}
                  onClick={() => onTreeFilterChange(
                    active
                      ? treeFilter.filter(id => id !== projectId)
                      : [...treeFilter, projectId]
                  )}
                  className={`px-2 py-0.5 rounded-full text-xs font-medium transition-colors ${
                    active
                      ? 'bg-blue-900/60 text-blue-300 border border-blue-700/60'
                      : 'bg-slate-800 text-slate-400 border border-slate-700 hover:text-slate-200'
                  }`}
                >
                  {projectName}
                </button>
              );
            })}
          </div>
        )}
        <label className="flex items-center gap-1.5 cursor-pointer ml-auto">
          <input
            type="checkbox"
            checked={frontier}
            onChange={e => onFrontierChange(e.target.checked)}
            className="w-3.5 h-3.5 rounded accent-blue-500"
          />
          <span className="text-xs text-slate-400">Include frontier</span>
        </label>
      </div>

      <p className="text-xs text-slate-500">
        Top {studyMap.entries.length} resources covering{' '}
        <span className="text-slate-400">{studyMap.totalUnlockedNodes} unlocked</span>
        {frontier
          ? <> + <span className={studyMap.totalFrontierNodes > 0 ? 'text-amber-400' : 'text-slate-600'}>{studyMap.totalFrontierNodes} frontier</span></>
          : null
        }
        {' '}nodes
        {studyMap.totalResourcesWithLinks > 0 && ` · ${studyMap.totalResourcesWithLinks} total linked`}
      </p>

      <div className="flex items-center gap-1.5 flex-wrap">
        {(['webpage', 'pdf', 'text'] as const).map(type => {
          const count = studyMap.entries.filter((e: StudyMapEntry) => e.resourceType === type).length;
          if (count === 0) return null;
          const labels: Record<string, string> = { webpage: 'URL', pdf: 'PDF', text: 'Text' };
          return (
            <span key={type} className="px-2 py-0.5 rounded-full text-xs bg-slate-800 text-slate-400 border border-slate-700">
              {labels[type]} · {count}
            </span>
          );
        })}
      </div>

      {studyMap.entries.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 gap-3 text-slate-500">
          <BookMarked className="w-8 h-8 opacity-30" />
          <p className="text-sm">No matched resources yet.</p>
          <p className="text-xs text-slate-600">Ingest resources and run Re-match to populate the Study Map.</p>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {studyMap.entries.map((entry: StudyMapEntry, idx: number) => {
            const nodesExpanded = expandedNodes.has(entry.resourceId);
            const doneSet = readProgress.get(entry.resourceId) ?? new Set<string>();
            return (
              <div
                key={entry.resourceId}
                className="bg-slate-900 border border-slate-800 rounded-lg p-4 flex flex-col gap-2"
              >
                <div className="flex items-start gap-2">
                  <span className="text-xs text-slate-600 font-mono w-5 flex-shrink-0 mt-0.5">{idx + 1}</span>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <RelevanceDot tier={entry.relevanceTier} />
                      {entry.url ? (
                        <a
                          href={entry.url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-sm font-medium text-slate-100 hover:text-blue-300 truncate"
                        >
                          {entry.title}
                        </a>
                      ) : (
                        <span className="text-sm font-medium text-slate-100 truncate">{entry.title}</span>
                      )}
                      <span className="text-xs text-slate-500 bg-slate-800 px-1.5 py-0.5 rounded capitalize flex-shrink-0">
                        {entry.resourceType === 'webpage' ? 'URL' : entry.resourceType.toUpperCase()}
                      </span>
                    </div>
                  </div>
                  <span className="flex-shrink-0 text-xs font-semibold text-emerald-400 bg-emerald-950/50 border border-emerald-800/50 rounded px-2 py-0.5">
                    {entry.coverageCount} node{entry.coverageCount !== 1 ? 's' : ''}
                  </span>
                </div>

                {entry.treeBreakdown.length > 0 && (
                  <div className="flex items-center gap-1.5 flex-wrap pl-7">
                    {entry.treeBreakdown.map(tb => (
                      <span
                        key={tb.projectId}
                        className="text-xs text-blue-300 bg-blue-950/40 border border-blue-800/40 rounded-full px-2 py-0.5"
                      >
                        {tb.projectName} · {tb.nodeCount}
                      </span>
                    ))}
                  </div>
                )}

                {entry.sections.length > 0 && (
                  <div className="pl-7">
                    <button
                      onClick={() => handleToggleNodes(entry.resourceId)}
                      className="flex items-center gap-1 text-xs text-slate-500 hover:text-slate-300 transition-colors mb-1"
                    >
                      {nodesExpanded
                        ? <ChevronUp className="w-3 h-3" />
                        : <ChevronDown className="w-3 h-3" />}
                      {nodesExpanded ? 'Hide' : 'Show'} {entry.coverageCount} checkpoint{entry.coverageCount !== 1 ? 's' : ''}
                    </button>
                    {nodesExpanded && (
                      <div className="flex flex-col gap-2 mt-1">
                        {entry.sections.map((section: StudyMapSection) => {
                          const isRead = doneSet.has(section.sectionTitle);
                          const markingRead = markingSection === `${entry.resourceId}|${section.sectionTitle}`;
                          const markingUpTo = markingSection === `${entry.resourceId}|${section.sectionTitle}|up_to`;
                          return (
                            <div key={section.sectionTitle} className="flex flex-col gap-1 group">
                              <div className="flex items-center gap-2 text-xs font-medium">
                                {/* Read indicator */}
                                {isRead
                                  ? <span className="text-emerald-500 flex-shrink-0" title="Read">✓</span>
                                  : <span className="w-3.5 flex-shrink-0" />}
                                <span className={`truncate ${isRead ? 'text-slate-600 line-through' : 'text-slate-400'}`}>
                                  {section.sectionTitle}
                                </span>
                                {section.pageStart != null && (
                                  <span className="text-slate-600 flex-shrink-0">
                                    pp. {section.pageStart}{section.pageEnd != null && section.pageEnd !== section.pageStart ? `–${section.pageEnd}` : ''}
                                  </span>
                                )}
                                <span className="text-slate-600 flex-shrink-0">· {section.nodeCount} checkpoint{section.nodeCount !== 1 ? 's' : ''}</span>
                                {/* Action buttons — visible on hover */}
                                {!isRead && (
                                  <div className="ml-auto flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                                    <button
                                      onClick={() => handleMarkRead(entry.resourceId, section)}
                                      disabled={!!markingSection}
                                      className="text-xs text-emerald-600 hover:text-emerald-400 disabled:opacity-40 px-1.5 py-0.5 rounded border border-emerald-900/50 hover:border-emerald-700/60 transition-colors"
                                      title="Mark this section as read"
                                    >
                                      {markingRead ? '…' : '✓ Read'}
                                    </button>
                                    {section.pageStart != null && (
                                      <button
                                        onClick={() => handleMarkUpTo(entry.resourceId, section)}
                                        disabled={!!markingSection}
                                        className="text-xs text-slate-500 hover:text-slate-300 disabled:opacity-40 px-1.5 py-0.5 rounded border border-slate-700/50 hover:border-slate-600 transition-colors"
                                        title="Mark all sections up to this page as read"
                                      >
                                        {markingUpTo ? '…' : '✓ Up to here'}
                                      </button>
                                    )}
                                  </div>
                                )}
                              </div>
                              <div className="flex flex-col gap-0.5 pl-5">
                                {section.nodes.map(node => (
                                  <div key={node.nodeId} className="flex items-center gap-2 text-xs text-slate-500">
                                    <span className="w-1 h-1 rounded-full bg-slate-700 flex-shrink-0" />
                                    <span className="truncate">{node.title}</span>
                                  </div>
                                ))}
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export default function ResourcesPage() {
  const [resources, setResources] = useState<MimirResource[]>([]);
  const [loading, setLoading] = useState(true);

  // Chunk counts
  const [chunkCounts, setChunkCounts] = useState<Record<string, number>>({});
  const [chunksLoaded, setChunksLoaded] = useState(false);

  // Filter + search + sort
  const [searchParams] = useSearchParams();
  const [showThin, setShowThin] = useState(false);
  const [searchQuery, setSearchQuery] = useState(() => searchParams.get('q') ?? '');
  const [typeFilter, setTypeFilter] = useState<string>('all');
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [sortBy, setSortBy] = useState<'newest' | 'oldest' | 'most_linked' | 'alpha'>('newest');

  // Tags
  const [allTags, setAllTags] = useState<string[]>([]);
  const [editingTagsFor, setEditingTagsFor] = useState<string | null>(null);
  const [tagInput, setTagInput] = useState('');
  const [autoTagging, setAutoTagging] = useState(false);
  const [autoTagResult, setAutoTagResult] = useState<string | null>(null);
  const [rematching, setRematching] = useState(false);
  const [rematchResult, setRematchResult] = useState<string | null>(null);
  const [reembedding, setReembedding] = useState(false);
  const [reembedResult, setReembedResult] = useState<string | null>(null);
  const [skillBackfilling, setSkillBackfilling] = useState(false);
  const [skillBackfillResult, setSkillBackfillResult] = useState<string | null>(null);
  const [llmExtracting, setLlmExtracting] = useState(false);
  const [llmExtractResult, setLlmExtractResult] = useState<string | null>(null);
  const [completionFilter, setCompletionFilter] = useState<'all' | 'completed' | 'incomplete'>('all');
  const [nodePopoverFor, setNodePopoverFor] = useState<string | null>(null);
  const [nodePopoverTitles, setNodePopoverTitles] = useState<string[]>([]);

  // Add form state
  const [addMode, setAddMode] = useState<AddMode>('url');
  const [urlInput, setUrlInput] = useState('');
  const [titleInput, setTitleInput] = useState('');
  const [textInput, setTextInput] = useState('');
  const [pdfFile, setPdfFile] = useState<File | null>(null);
  const [forceDynamic, setForceDynamic] = useState(false);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState('');
  const [dupWarning, setDupWarning] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Per-card rescrape state
  const [cardState, setCardState] = useState<Record<string, CardState>>({});
  const [cardMsg, setCardMsg] = useState<Record<string, string>>({});

  // Bulk rescrape state
  const [bulkOpen, setBulkOpen] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<BulkProgress[]>([]);
  const [bulkDone, setBulkDone] = useState(false);
  const [bulkSummary, setBulkSummary] = useState<{ improved: number; unchanged: number; failed: number } | null>(null);
  const [bulkRunning, setBulkRunning] = useState(false);
  const [bulkCurrent, setBulkCurrent] = useState<{ index: number; total: number; title: string } | null>(null);

  // Playlist state
  const [playlistDetected, setPlaylistDetected] = useState(false);
  const [playlistData, setPlaylistData] = useState<PlaylistModalData | null>(null);
  const [playlistFetching, setPlaylistFetching] = useState(false);
  const [playlistProgress, setPlaylistProgress] = useState<{ current: number; total: number; currentTitle: string; skipping: boolean } | null>(null);
  const [playlistDone, setPlaylistDone] = useState(false);
  const [playlistToast, setPlaylistToast] = useState<string | null>(null);

  // Discover state
  const [discoverFor, setDiscoverFor] = useState<{ id: string; url: string; title: string } | null>(null);
  const [discoverLinks, setDiscoverLinks] = useState<{ url: string; text: string; checked: boolean }[]>([]);
  const [discoverLoading, setDiscoverLoading] = useState(false);
  const [discoverError, setDiscoverError] = useState('');
  const [discoverProgress, setDiscoverProgress] = useState<{ current: number; total: number } | null>(null);
  const [discoverDone, setDiscoverDone] = useState(false);
  const [scraperDown, setScraperDown] = useState(false);
  const [transcriptStats, setTranscriptStats] = useState<TranscriptJobStats | null>(null);
  const [showTranscriptPopover, setShowTranscriptPopover] = useState(false);

  // Study Map state
  const [showStudyMap, setShowStudyMap] = useState(false);
  const [studyMap, setStudyMap] = useState<ResourceStudyMap | null>(null);
  const [studyMapLoading, setStudyMapLoading] = useState(false);
  const [studyMapFrontier, setStudyMapFrontier] = useState(false);
  const [studyMapTreeFilter, setStudyMapTreeFilter] = useState<string[]>([]);
  const [studyMapExpandedNodes, setStudyMapExpandedNodes] = useState<Set<string>>(new Set());

  // External links modal state (after URL ingest of resource_list pages)
  const [extLinksData, setExtLinksData] = useState<{ parentId: string; parentTitle: string; links: ExternalLinkItem[] } | null>(null);
  const [extLinksProgress, setExtLinksProgress] = useState<{ current: number; total: number; currentTitle: string; skipping: boolean } | null>(null);
  const [extLinksDone, setExtLinksDone] = useState(false);

  // Parent-child grouping state
  const [expandedParents, setExpandedParents] = useState<Set<string>>(new Set());
  const initialExpandDone = useRef(false);

  // Librarian note state
  const [librarianNote, setLibrarianNote] = useState<string | null>(null);

  async function loadResources() {
    try {
      const raw = await invoke('get_mimir_resources');
      const list = validateOrLog(z.array(MimirResourceSchema), raw, 'get_mimir_resources') as MimirResource[];
      setResources(list);
    } catch (err) {
      console.error('Failed to load resources:', err);
    } finally {
      setLoading(false);
    }
  }

  async function loadStudyMap() {
    setStudyMapLoading(true);
    try {
      const result = await invoke<ResourceStudyMap>('get_resource_study_map', {
        projectIds: studyMapTreeFilter.length > 0 ? studyMapTreeFilter : null,
        includeFrontier: studyMapFrontier,
      });
      setStudyMap(result);
    } catch (err) {
      console.error('Study Map load failed:', err);
    } finally {
      setStudyMapLoading(false);
    }
  }

  async function loadChunkCounts() {
    try {
      const counts = await invoke<Record<string, number>>('get_chunk_counts');
      setChunkCounts(counts);
      setChunksLoaded(true);
    } catch {
      // chunk counts are supplementary — don't block UI
    }
  }

  async function loadTags() {
    try {
      const tags = await invoke<string[]>('get_distinct_tags');
      setAllTags(tags);
    } catch {
      // tags are supplementary
    }
  }

  async function handleAutoTag() {
    if (autoTagging) return;
    setAutoTagging(true);
    setAutoTagResult(null);
    try {
      const result = await invoke<string>('auto_tag_existing_resources');
      setAutoTagResult(result);
      await loadResources();
      await loadTags();
      setTimeout(() => setAutoTagResult(null), 5000);
    } catch (err) {
      setAutoTagResult(`Failed: ${err}`);
      setTimeout(() => setAutoTagResult(null), 5000);
    } finally {
      setAutoTagging(false);
    }
  }

  async function handleRematchAll() {
    if (rematching) return;
    setRematching(true);
    setRematchResult('Re-matching in background…');
    invoke<string>('rematch_all_nodes')
      .then(result => {
        setRematchResult(result);
        loadResources();
        setTimeout(() => setRematchResult(null), 5000);
      })
      .catch(err => {
        setRematchResult(`Failed: ${err}`);
        setTimeout(() => setRematchResult(null), 5000);
      })
      .finally(() => setRematching(false));
  }

  async function handleReembedPdfs() {
    if (reembedding) return;
    setReembedding(true);
    setReembedResult(null);
    try {
      const result = await invoke<string>('reembed_pdfs');
      setReembedResult(result);
      setTimeout(() => setReembedResult(null), 5000);
    } catch (err) {
      setReembedResult(`Failed: ${err}`);
      setTimeout(() => setReembedResult(null), 5000);
    } finally {
      setReembedding(false);
    }
  }

  async function handleLlmSkillExtraction() {
    if (llmExtracting) return;
    setLlmExtracting(true);
    setLlmExtractResult(null);
    try {
      const result = await invoke<string>('extract_skills_backfill');
      setLlmExtractResult(result);
      setTimeout(() => setLlmExtractResult(null), 10000);
    } catch (err) {
      setLlmExtractResult(`Failed: ${err}`);
      setTimeout(() => setLlmExtractResult(null), 8000);
    } finally {
      setLlmExtracting(false);
    }
  }

  async function handleSkillResourceBackfill() {
    if (skillBackfilling) return;
    setSkillBackfilling(true);
    setSkillBackfillResult(null);
    try {
      const result = await invoke<string>('run_skill_resource_backfill');
      setSkillBackfillResult(result);
      setTimeout(() => setSkillBackfillResult(null), 8000);
    } catch (err) {
      setSkillBackfillResult(`Failed: ${err}`);
      setTimeout(() => setSkillBackfillResult(null), 8000);
    } finally {
      setSkillBackfilling(false);
    }
  }

  async function handleToggleCompletion(resourceId: string) {
    try {
      const newVal = await invoke<boolean>('toggle_resource_completion', { resourceId });
      // Orchestrator handles tree progress + skills cascade server-side on mark-complete.
      // Optimistically update completion badge in the list.
      setResources(prev => prev.map(r => r.id === resourceId ? { ...r, isCompleted: newVal } : r));
    } catch (err) {
      console.error('Failed to toggle completion:', err);
    }
  }

  async function handleShowLinkedNodes(resourceId: string) {
    if (nodePopoverFor === resourceId) {
      setNodePopoverFor(null);
      return;
    }
    try {
      const rows = await invoke<{ title: string }[]>('get_linked_node_titles', { resourceId });
      setNodePopoverTitles(rows.map(r => r.title));
      setNodePopoverFor(resourceId);
    } catch {
      setNodePopoverTitles([]);
      setNodePopoverFor(resourceId);
    }
  }

  async function handleUpdateTags(resourceId: string, tags: string[]) {
    try {
      await invoke('update_resource_tags', { resourceId, tags });
      setResources(prev => prev.map(r => r.id === resourceId ? { ...r, tags } : r));
      await loadTags();
    } catch (err) {
      console.error('Failed to update tags:', err);
    }
  }

  function handleRemoveTag(resourceId: string, tag: string) {
    const resource = resources.find(r => r.id === resourceId);
    if (!resource) return;
    handleUpdateTags(resourceId, resource.tags.filter(t => t !== tag));
  }

  function handleAddTag(resourceId: string, tag: string) {
    const trimmed = tag.trim().toLowerCase();
    if (!trimmed) return;
    const resource = resources.find(r => r.id === resourceId);
    if (!resource || resource.tags.includes(trimmed)) return;
    handleUpdateTags(resourceId, [...resource.tags, trimmed]);
    setTagInput('');
  }

  const childrenByParent = useMemo(() =>
    resources.reduce((acc, r) => {
      if (r.parentId) acc[r.parentId] = [...(acc[r.parentId] || []), r];
      return acc;
    }, {} as Record<string, MimirResource[]>),
  [resources]);

  function getTotalChunks(resourceId: string): number {
    const own = chunkCounts[resourceId] || 0;
    const childChunks = (childrenByParent[resourceId] || []).reduce((sum, c) => sum + (chunkCounts[c.id] || 0), 0);
    return own + childChunks;
  }

  function getResourceIcon(resource: MimirResource): React.ReactElement {
    if (resource.url?.includes('playlist?list=')) return <span className="text-sm">🎬</span>;
    if (resource.url?.includes('youtube.com/watch')) return <Play className="w-3 h-3 fill-current" />;
    return TYPE_ICON[resource.resourceType] ?? <Link className="w-3.5 h-3.5" />;
  }

  useEffect(() => {
    loadResources();
    loadChunkCounts();
    loadTags();

    // Load transcript job stats on mount, then poll every 30s when jobs are pending
    async function fetchTranscriptStats() {
      try {
        const stats = await invoke<TranscriptJobStats>('get_transcript_job_status');
        setTranscriptStats(stats);
      } catch {
        // ignore — command may not be available in older builds
      }
    }
    fetchTranscriptStats();
    const intervalId = setInterval(async () => {
      const stats = await invoke<TranscriptJobStats>('get_transcript_job_status').catch(() => null);
      if (stats) setTranscriptStats(stats);
    }, 30_000);
    return () => clearInterval(intervalId);
  }, []);

  // Reload list when orchestrator finishes ingesting or completing a resource
  useEffect(() => {
    const unlistenIngested = listen('ygg-resource-ingested', () => { loadResources(); loadTags(); });
    const unlistenCompleted = listen('ygg-resource-completed', () => { loadResources(); });
    const unlistenRematch = listen('ygg-rematch-complete', () => { if (showStudyMap) { void loadStudyMap(); } });
    return () => {
      unlistenIngested.then(fn => fn());
      unlistenCompleted.then(fn => fn());
      unlistenRematch.then(fn => fn());
    };
  }, [showStudyMap]);

  useEffect(() => {
    const t = setTimeout(() => {
      const url = urlInput.trim();
      const isPlaylist = url.includes('youtube.com') && url.includes('list=') && url.includes('/playlist');
      setPlaylistDetected(isPlaylist);
    }, 500);
    return () => clearTimeout(t);
  }, [urlInput]);

  useEffect(() => {
    if (resources.length > 0 && !initialExpandDone.current) {
      initialExpandDone.current = true;
      const playlistIds = resources.filter(r => r.url?.includes('playlist?list=')).map(r => r.id);
      if (playlistIds.length > 0) setExpandedParents(new Set(playlistIds));
    }
  }, [resources]);

  useEffect(() => {
    if (showStudyMap) loadStudyMap();
  }, [showStudyMap, studyMapFrontier, studyMapTreeFilter]);

  useEffect(() => {
    let unsub: (() => void) | undefined;
    (async () => {
      unsub = await listen<{ untagged: number; unmatched: number; queued: number }>(
        "ygg-library-suggestion",
        ({ payload }) => {
          setLibrarianNote(
            `🧹 Auto-librarian queued ${payload.queued} resource(s) — ${payload.untagged} to tag, ${payload.unmatched} to match.`
          );
          setTimeout(() => setLibrarianNote(null), 12000);
        }
      );
    })();
    return () => { unsub?.(); };
  }, []);

  async function handleAdd() {
    if (adding) return;
    if (addMode === 'url' && !urlInput.trim()) return;
    if (addMode === 'text' && !textInput.trim()) return;
    if (addMode === 'pdf' && !pdfFile) return;

    // Intercept YouTube playlist URLs
    if (addMode === 'url' && playlistDetected) {
      await handleFetchPlaylist();
      return;
    }

    setAdding(true);
    setAddError('');
    try {
      if (addMode === 'url') {
        const result = await invoke<IngestResult>('ingest_mimir_url', {
          url: urlInput.trim(),
          title: titleInput.trim() || undefined,
          forceDynamic: forceDynamic || undefined,
        });
        setUrlInput('');
        setTitleInput('');
        // If the page was a resource list with external links, offer to ingest them
        if (result.externalLinks && result.externalLinks.length > 0) {
          setExtLinksData({
            parentId: result.id,
            parentTitle: result.title,
            links: result.externalLinks.map(l => ({ ...l, checked: true })),
          });
          setExtLinksDone(false);
          setExtLinksProgress(null);
        }
      } else if (addMode === 'text') {
        if (!titleInput.trim()) {
          setAddError('Title is required for text ingestion');
          setAdding(false);
          return;
        }
        await invoke('ingest_mimir_text', {
          text: textInput.trim(),
          title: titleInput.trim(),
        });
        setTextInput('');
        setTitleInput('');
      } else {
        const file = pdfFile!;
        const arrayBuffer = await file.arrayBuffer();
        const bytes = new Uint8Array(arrayBuffer);
        let binary = '';
        bytes.forEach(b => { binary += String.fromCharCode(b); });
        const pdf_base64 = btoa(binary);
        await invoke('ingest_mimir_pdf', {
          filename: file.name,
          pdfBase64: pdf_base64,
        });
        setPdfFile(null);
        if (fileInputRef.current) fileInputRef.current.value = '';
      }
      await loadResources();
      await loadChunkCounts();
    } catch (err) {
      const msg = String(err);
      if (msg.startsWith('DUPLICATE:')) {
        const title = msg.slice('DUPLICATE:'.length);
        setDupWarning(title);
        setTimeout(() => setDupWarning(null), 4000);
      } else {
        setAddError(msg);
      }
    } finally {
      setAdding(false);
    }
  }

  async function handleDelete(id: string) {
    try {
      await invoke('delete_mimir_resource', { resourceId: id });
      setResources(prev => prev.filter(r => r.id !== id));
      setChunkCounts(prev => { const next = { ...prev }; delete next[id]; return next; });
    } catch (err) {
      alert(`Delete failed: ${err}`);
    }
  }

  async function handleRescrapeOne(id: string) {
    setCardState(prev => ({ ...prev, [id]: 'loading' }));
    setCardMsg(prev => ({ ...prev, [id]: '' }));
    try {
      const result = await invoke<RescrapeResult>('rescrape_resource', { resourceId: id });
      if (result.changed) {
        const delta = result.newChunks - result.oldChunks;
        setCardState(prev => ({ ...prev, [id]: 'done' }));
        setCardMsg(prev => ({ ...prev, [id]: `+${delta} chunk${delta !== 1 ? 's' : ''}` }));
        setChunkCounts(prev => ({ ...prev, [id]: result.newChunks }));
        setTimeout(() => setCardState(prev => ({ ...prev, [id]: 'idle' })), 3000);
      } else {
        setCardState(prev => ({ ...prev, [id]: 'unchanged' }));
        setCardMsg(prev => ({ ...prev, [id]: 'Already complete' }));
        setTimeout(() => setCardState(prev => ({ ...prev, [id]: 'idle' })), 2000);
      }
    } catch (err) {
      setCardState(prev => ({ ...prev, [id]: 'error' }));
      setCardMsg(prev => ({ ...prev, [id]: 'Failed — try pasting text instead' }));
      setTimeout(() => setCardState(prev => ({ ...prev, [id]: 'idle' })), 4000);
    }
  }

  async function handleRescrapeAll() {
    if (bulkRunning) return;
    setBulkOpen(true);
    setBulkProgress([]);
    setBulkDone(false);
    setBulkSummary(null);
    setBulkCurrent(null);
    setBulkRunning(true);

    const unlistenProgress = await listen<{
      status: 'starting' | 'improved' | 'unchanged' | 'failed';
      current: number; total: number; resourceId: string; title: string;
      oldChunks?: number; newChunks?: number;
    }>('rescrape-progress', ({ payload }) => {
      if (payload.status === 'starting') {
        setBulkCurrent({ index: payload.current, total: payload.total, title: payload.title });
        return;
      }
      setBulkCurrent(null);
      const finalStatus = payload.status as 'improved' | 'unchanged' | 'failed';
      setBulkProgress(prev => [...prev, {
        resourceId: payload.resourceId,
        title: payload.title,
        status: finalStatus,
        oldChunks: payload.oldChunks ?? 0,
        newChunks: payload.newChunks ?? 0,
      }]);
      if (payload.status === 'improved' && payload.newChunks != null) {
        setChunkCounts(prev => ({ ...prev, [payload.resourceId]: payload.newChunks! }));
      }
    });

    const unlistenComplete = await listen<{
      improved: number; unchanged: number; failed: number; total: number;
    }>('rescrape-complete', ({ payload }) => {
      setBulkCurrent(null);
      setBulkDone(true);
      setBulkSummary({
        improved: payload.improved,
        unchanged: payload.unchanged,
        failed: payload.failed,
      });
      unlistenProgress();
      unlistenComplete();
      setBulkRunning(false);
      loadChunkCounts();
    });

    try {
      await invoke('rescrape_all');
    } catch (err) {
      setBulkCurrent(null);
      setBulkDone(true);
      unlistenProgress();
      unlistenComplete();
      setBulkRunning(false);
      loadChunkCounts();
      console.error('Rescrape all failed:', err);
    }
  }

  async function handleFetchPlaylist() {
    if (playlistFetching) return;
    setPlaylistFetching(true);
    setAddError('');
    try {
      const data = await invoke<{ playlistTitle: string; playlistId: string; videos: { videoId: string; title: string; url: string; position: number }[] }>(
        'fetch_playlist', { url: urlInput.trim() }
      );
      setPlaylistData({
        playlistTitle: data.playlistTitle,
        playlistId: data.playlistId,
        playlistUrl: urlInput.trim(),
        videos: data.videos.map(v => ({ ...v, checked: true })),
      });
    } catch (err) {
      const msg = String(err);
      if (msg.includes('not running') || msg.includes('ECONNREFUSED')) {
        setScraperDown(true);
      }
      setAddError(msg);
    } finally {
      setPlaylistFetching(false);
    }
  }

  async function handleIngestPlaylist() {
    if (!playlistData) return;
    const selected = playlistData.videos.filter(v => v.checked);
    if (selected.length === 0) return;

    setPlaylistProgress({ current: 0, total: selected.length, currentTitle: '', skipping: false });

    // Ingest the playlist URL itself as the parent resource
    let parentId: string | undefined;
    
    try {
      // Create playlist parent as a text resource — don't try to scrape YouTube playlist URL
      parentId = await invoke<string>('ingest_mimir_text', {
        text: `YouTube Playlist: ${playlistData.playlistTitle}\n\nURL: ${playlistData.playlistUrl}`,
        title: playlistData.playlistTitle,
      });
      log('[playlist] parent created:', parentId);
    } catch (err) {
      const msg = String(err);
      log('[playlist] parent create error:', msg);
      if (msg.startsWith('DUPLICATE:')) {
        const existing = resources.find(r => r.title === playlistData.playlistTitle);
        parentId = existing?.id;
        log('[playlist] found existing parent:', parentId);
      }
    }

    log('[playlist] parentId captured:', parentId);

    for (let i = 0; i < selected.length; i++) {
      const video = selected[i];
      log(`[playlist] ingesting video ${i+1} with parent_id:`, parentId);
      setPlaylistProgress({ current: i + 1, total: selected.length, currentTitle: video.title, skipping: false });
      try {
        await invoke('ingest_mimir_url', {
          url: video.url,
          title: video.title,
          parentId: parentId, // for backward compatibility with older sidecar versions that expect this key
        });
      } catch (err) {
        const msg = String(err);
        if (msg.startsWith('DUPLICATE:')) {
          setPlaylistProgress(prev => prev ? { ...prev, skipping: true } : null);
        }
        // ignore other individual failures — best-effort
      }
      if (i < selected.length - 1) {
        await new Promise(r => setTimeout(r, 2000));
      }
    }

    setPlaylistDone(true);
    setPlaylistProgress(null);
    const count = selected.length;
    setPlaylistToast(`✓ Playlist ingested — ${count} video${count !== 1 ? 's' : ''} added`);
    setTimeout(() => setPlaylistToast(null), 4000);
    setPlaylistData(null);
    setPlaylistDone(false);
    setUrlInput('');
    setPlaylistDetected(false);
    await loadResources();
    await loadChunkCounts();
  }

  function closePlaylistModal() {
    setPlaylistData(null);
    setPlaylistProgress(null);
    setPlaylistDone(false);
  }

  async function handleIngestExternalLinks() {
    if (!extLinksData) return;
    const selected = extLinksData.links.filter(l => l.checked);
    if (selected.length === 0) return;

    setExtLinksProgress({ current: 0, total: selected.length, currentTitle: '', skipping: false });

    for (let i = 0; i < selected.length; i++) {
      const link = selected[i];
      setExtLinksProgress({ current: i + 1, total: selected.length, currentTitle: link.text || link.url, skipping: false });
      try {
        await invoke<IngestResult>('ingest_mimir_url', {
          url: link.url,
          title: link.text || undefined,
          parentId: extLinksData.parentId,
        });
      } catch (err) {
        const msg = String(err);
        if (msg.startsWith('DUPLICATE:')) {
          setExtLinksProgress(prev => prev ? { ...prev, skipping: true } : null);
        }
      }
      if (i < selected.length - 1) {
        await new Promise(r => setTimeout(r, 2000));
      }
    }

    setExtLinksDone(true);
    setExtLinksProgress(null);
    await loadResources();
    await loadChunkCounts();
  }

  function closeExtLinksModal() {
    setExtLinksData(null);
    setExtLinksProgress(null);
    setExtLinksDone(false);
  }

  async function openDiscover(resource: MimirResource) {
    setDiscoverFor({ id: resource.id, url: resource.url!, title: resource.title });
    setDiscoverLinks([]);
    setDiscoverLoading(true);
    setDiscoverError('');
    setDiscoverProgress(null);
    setDiscoverDone(false);

    try {
      const links = await invoke<{ url: string; text: string }[]>('discover_links', { url: resource.url! });
      setDiscoverLinks(links.map(l => ({ ...l, checked: true })));
    } catch (err) {
      const msg = String(err);
      if (msg.includes('not running') || msg.includes('ECONNREFUSED') || msg.includes('offline')) {
        setScraperDown(true);
      }
      setDiscoverError(msg);
    } finally {
      setDiscoverLoading(false);
    }
  }

  function closeDiscover() {
    setDiscoverFor(null);
    setDiscoverLinks([]);
    setDiscoverError('');
    setDiscoverLoading(false);
    setDiscoverProgress(null);
    setDiscoverDone(false);
  }

  async function handleIngestSelected() {
    const toIngest = discoverLinks.filter(l => l.checked);
    if (toIngest.length === 0) return;

    setDiscoverProgress({ current: 0, total: toIngest.length });
    setDiscoverDone(false);

    for (let i = 0; i < toIngest.length; i++) {
      setDiscoverProgress({ current: i + 1, total: toIngest.length });
      try {
        await invoke('ingest_mimir_url', { url: toIngest[i].url, parentId: discoverFor?.id });
      } catch {
        // ignore duplicates and individual errors — best-effort batch
      }
      if (i < toIngest.length - 1) {
        await new Promise(r => setTimeout(r, 1000));
      }
    }

    setDiscoverDone(true);
    setDiscoverProgress(null);
    await loadResources();
    await loadChunkCounts();
  }

  function closeBulkModal() {
    setBulkOpen(false);
    setBulkProgress([]);
    setBulkSummary(null);
    setBulkDone(false);
    setBulkCurrent(null);
  }

  const urlResources = resources.filter(r => r.resourceType === 'webpage');
  const thinCount = chunksLoaded ? resources.filter(r => (chunkCounts[r.id] ?? 0) <= 3).length : 0;

  const filteredResources = useMemo(() => {
    let list = resources;

    // Thin filter
    if (showThin) {
      list = list.filter(r => (chunkCounts[r.id] ?? 0) <= 3);
    }

    // Search
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      list = list.filter(r =>
        r.title.toLowerCase().includes(q) ||
        (r.url && r.url.toLowerCase().includes(q))
      );
    }

    // Type filter
    if (typeFilter !== 'all') {
      list = list.filter(r => r.resourceType === typeFilter);
    }

    // Tag filter (any match)
    if (selectedTags.length > 0) {
      list = list.filter(r =>
        r.tags && r.tags.some(t => selectedTags.includes(t))
      );
    }

    // Completion filter
    if (completionFilter === 'completed') {
      list = list.filter(r => r.isCompleted);
    } else if (completionFilter === 'incomplete') {
      list = list.filter(r => !r.isCompleted);
    }

    // Sort
    list = [...list].sort((a, b) => {
      switch (sortBy) {
        case 'oldest':
          return (a.createdAt || '').localeCompare(b.createdAt || '');
        case 'most_linked':
          return (b.nodeCount || 0) - (a.nodeCount || 0);
        case 'alpha':
          return a.title.localeCompare(b.title);
        case 'newest':
        default:
          return (b.createdAt || '').localeCompare(a.createdAt || '');
      }
    });

    return list;
  }, [resources, showThin, chunkCounts, searchQuery, typeFilter, selectedTags, completionFilter, sortBy]);

  const parentIdSet = new Set(resources.map(r => r.id));
  const topLevelFiltered = filteredResources.filter(r => !r.parentId || !parentIdSet.has(r.parentId));

  const tagSuggestions = useMemo(() => {
    if (!tagInput.trim() || !editingTagsFor) return allTags;
    const q = tagInput.toLowerCase();
    return allTags.filter(t => t.includes(q));
  }, [tagInput, allTags, editingTagsFor]);

  const canAdd = addMode === 'url' ? !!urlInput.trim()
    : addMode === 'text' ? !!textInput.trim()
    : !!pdfFile;

  return (
    <div className="p-6 max-w-3xl mx-auto flex flex-col gap-6">
      {librarianNote && (
        <div className="mb-3 rounded-md border border-white/10 bg-white/5 px-3 py-2 text-sm opacity-80">
          {librarianNote}
        </div>
      )}
      {/* Header */}
      <div>
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-semibold text-slate-100">Mimir Library</h1>
          {transcriptStats && (transcriptStats.pending + transcriptStats.processing) > 0 && (
            <div className="relative">
              <button
                onClick={() => setShowTranscriptPopover(v => !v)}
                className="flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-amber-900/50 text-amber-300 border border-amber-700/60 hover:bg-amber-900/70 transition-colors"
              >
                <span className="animate-pulse w-1.5 h-1.5 rounded-full bg-amber-400 inline-block" />
                {transcriptStats.pending + transcriptStats.processing} transcript{transcriptStats.pending + transcriptStats.processing !== 1 ? 's' : ''} pending
              </button>
              {showTranscriptPopover && (
                <div className="absolute left-0 top-full mt-1 z-20 bg-slate-900 border border-slate-700 rounded-lg p-3 shadow-xl min-w-[180px]">
                  <p className="text-xs font-semibold text-slate-300 mb-2">Transcript Jobs</p>
                  {([['pending', transcriptStats.pending], ['processing', transcriptStats.processing], ['done', transcriptStats.done], ['failed', transcriptStats.failed], ['skipped', transcriptStats.skipped]] as [string, number][]).map(([label, count]) => count > 0 && (
                    <div key={label} className="flex items-center justify-between text-xs text-slate-400 py-0.5">
                      <span className="capitalize">{label}</span>
                      <span className="text-slate-200 font-medium">{count}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
          <button
            onClick={() => setShowStudyMap(v => !v)}
            className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs font-medium transition-colors ${
              showStudyMap
                ? 'bg-blue-900/60 text-blue-300 border border-blue-700/60'
                : 'bg-slate-800 text-slate-400 border border-slate-700 hover:text-slate-200'
            }`}
          >
            <MapIcon className="w-3.5 h-3.5" />
            Study Map
          </button>
        </div>
        <p className="text-sm text-slate-400 mt-1">
          Ingest URLs, text, or PDFs — Mimir embeds them and auto-links them to matching quest nodes.
        </p>
      </div>

      {/* Add resource panel */}
      <div className="bg-slate-900 border border-slate-800 rounded-lg p-4 flex flex-col gap-3">
        <p className="text-xs font-medium text-slate-500 uppercase tracking-wider">Add Resource</p>

          {/* Mode toggle */}
          <div className="flex gap-1 bg-slate-950 rounded-md p-1 w-fit">
            {(['url', 'text', 'pdf'] as AddMode[]).map(m => (
              <button
                key={m}
                onClick={() => { setAddMode(m); setAddError(''); }}
                className={`px-3 py-1 rounded text-xs font-medium transition-colors ${
                  addMode === m
                    ? 'bg-emerald-900/60 text-emerald-300 border border-emerald-800/60'
                    : 'text-slate-500 hover:text-slate-300'
                }`}
              >
                {MODE_LABEL[m]}
              </button>
            ))}
          </div>

          {/* Title input (URL + text modes only) */}
          {addMode !== 'pdf' && (
            <input
              type="text"
              value={titleInput}
              onChange={e => setTitleInput(e.target.value)}
              placeholder={addMode === 'url' ? 'Title (optional — auto-detected from page)' : 'Title *'}
              className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-emerald-600"
            />
          )}

          {addMode === 'url' ? (
            <>
              <input
                type="url"
                value={urlInput}
                onChange={e => setUrlInput(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleAdd()}
                placeholder="https://example.com/article"
                className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-emerald-600"
              />
              {playlistDetected && (
                <div className="flex items-center gap-2 text-xs text-blue-300 bg-blue-950/30 border border-blue-800/50 rounded-md px-3 py-2">
                  <span>📋</span>
                  <span>YouTube playlist detected — click Ingest to preview videos</span>
                </div>
              )}
              <label className="flex items-center gap-2 cursor-pointer w-fit">
                <input
                  type="checkbox"
                  checked={forceDynamic}
                  onChange={e => setForceDynamic(e.target.checked)}
                  className="w-3.5 h-3.5 rounded accent-emerald-500"
                />
                <span className="text-xs text-slate-400">
                  Force full browser render
                  <span className="ml-1 text-slate-600">(NeetCode, LeetCode, JS-heavy sites)</span>
                </span>
              </label>
            </>
          ) : addMode === 'text' ? (
            <textarea
              value={textInput}
              onChange={e => setTextInput(e.target.value)}
              placeholder="Paste notes, documentation, or any text content…"
              rows={6}
              className="bg-slate-950 border border-slate-700 rounded-md px-3 py-2 text-sm text-slate-200 font-mono resize-none focus:outline-none focus:border-emerald-600"
            />
          ) : (
            <div className="flex flex-col gap-2">
              <label
                className="flex items-center gap-3 bg-slate-950 border border-slate-700 border-dashed rounded-md px-3 py-4 cursor-pointer hover:border-emerald-700 transition-colors"
                onClick={() => fileInputRef.current?.click()}
              >
                <BookOpen className="w-5 h-5 text-slate-500 flex-shrink-0" />
                <span className="text-sm text-slate-400">
                  {pdfFile ? pdfFile.name : 'Click to select a PDF file…'}
                </span>
              </label>
              <input
                ref={fileInputRef}
                type="file"
                accept=".pdf,application/pdf"
                className="hidden"
                onChange={e => setPdfFile(e.target.files?.[0] ?? null)}
              />
            </div>
          )}

          {addError && (
            <p className="text-xs text-red-400">{addError}</p>
          )}

          <button
            onClick={handleAdd}
            disabled={adding || playlistFetching || !canAdd}
            className="self-start px-4 py-2 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md text-sm font-medium transition-colors flex items-center gap-2"
          >
            {(adding || playlistFetching) ? (
              <>
                <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none"/>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
                </svg>
                {playlistFetching ? 'Fetching playlist…' : addMode === 'url' ? 'Fetching & embedding…' : addMode === 'pdf' ? 'Parsing & embedding…' : 'Embedding…'}
              </>
            ) : (
              addMode === 'url' && playlistDetected ? 'Preview Playlist' :
              addMode === 'url' ? 'Ingest URL' :
              addMode === 'pdf' ? 'Ingest PDF' : 'Ingest Text'
            )}
          </button>
          {adding && (
            <p className="text-xs text-slate-500">
              First run downloads the ~90 MB embedding model — this can take 1–2 minutes.
            </p>
          )}
        </div>

      {/* Study Map panel */}
      {showStudyMap && (
        <StudyMapPanel
          studyMap={studyMap}
          loading={studyMapLoading}
          frontier={studyMapFrontier}
          onFrontierChange={v => setStudyMapFrontier(v)}
          treeFilter={studyMapTreeFilter}
          onTreeFilterChange={ids => setStudyMapTreeFilter(ids)}
          expandedNodes={studyMapExpandedNodes}
          onToggleNodes={id => setStudyMapExpandedNodes(prev => {
            const next = new Set(prev);
            next.has(id) ? next.delete(id) : next.add(id);
            return next;
          })}
        />
      )}

      {/* Resource list */}
      {!showStudyMap && (loading ? (
        <p className="text-sm text-slate-500">Loading…</p>
      ) : resources.length === 0 ? (
        <p className="text-sm text-slate-500">
          No resources yet. Ingest a URL, paste some text, or upload a PDF to get started.
        </p>
      ) : (
        <>
          {resources.length > 0 && (
            <div className="flex flex-col gap-3 -mb-2">
              {/* Search bar */}
              <div className="relative">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder="Search by title or URL…"
                  className="w-full bg-slate-900 border border-slate-800 rounded-lg pl-9 pr-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-emerald-600 placeholder:text-slate-600"
                />
                {searchQuery && (
                  <button onClick={() => setSearchQuery('')} className="absolute right-3 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300">
                    <X className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>

              {/* Filter row */}
              <div className="flex items-center gap-2 flex-wrap">
                {/* Type filter */}
                <select
                  value={typeFilter}
                  onChange={e => setTypeFilter(e.target.value)}
                  className="bg-slate-900 border border-slate-800 rounded-md px-2 py-1 text-xs text-slate-300 focus:outline-none focus:border-emerald-600 cursor-pointer"
                >
                  <option value="all">All types</option>
                  <option value="webpage">URL</option>
                  <option value="pdf">PDF</option>
                  <option value="text">Text</option>
                </select>

                {/* Sort */}
                <select
                  value={sortBy}
                  onChange={e => setSortBy(e.target.value as typeof sortBy)}
                  className="bg-slate-900 border border-slate-800 rounded-md px-2 py-1 text-xs text-slate-300 focus:outline-none focus:border-emerald-600 cursor-pointer"
                >
                  <option value="newest">Newest</option>
                  <option value="oldest">Oldest</option>
                  <option value="most_linked">Most linked</option>
                  <option value="alpha">A → Z</option>
                </select>

                {/* Thin filter */}
                {chunksLoaded && thinCount > 0 && (
                  <button
                    onClick={() => setShowThin(v => !v)}
                    className={`flex items-center gap-1 text-xs px-2 py-1 rounded transition-colors ${
                      showThin
                        ? 'bg-amber-900/50 text-amber-300 border border-amber-700/60'
                        : 'text-amber-500 hover:text-amber-300 border border-slate-800 hover:border-amber-800/40'
                    }`}
                    title="Show only resources with ≤ 3 chunks — likely incomplete"
                  >
                    <Filter className="w-3 h-3" />
                    {showThin ? 'Show all' : `${thinCount} thin`}
                  </button>
                )}

                {/* Spacer */}
                <div className="flex-1" />

                {/* Auto-tag button */}
                <button
                  onClick={handleAutoTag}
                  disabled={autoTagging}
                  className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-emerald-300 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="Use AI to auto-tag all untagged resources"
                >
                  <Wand2 className={`w-3.5 h-3.5 ${autoTagging ? 'animate-spin' : ''}`} />
                  {autoTagging ? 'Tagging…' : 'Auto-tag'}
                </button>

                {/* Re-match all nodes */}
                <button
                  onClick={handleRematchAll}
                  disabled={rematching}
                  className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-blue-300 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="Re-match all leaf nodes to resources via embeddings"
                >
                  <Link className={`w-3.5 h-3.5 ${rematching ? 'animate-spin' : ''}`} />
                  {rematching ? 'Matching…' : 'Re-match'}
                </button>

                {/* Re-embed PDFs */}
                <button
                  onClick={handleReembedPdfs}
                  disabled={reembedding}
                  className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-amber-300 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="Re-embed all PDF resources from stored chunk text (use after embedding model change)"
                >
                  <BookOpen className={`w-3.5 h-3.5 ${reembedding ? 'animate-spin' : ''}`} />
                  {reembedding ? 'Re-embedding…' : 'Re-embed PDFs'}
                </button>

                {/* Backfill skill → chapter links */}
                <button
                  onClick={handleSkillResourceBackfill}
                  disabled={skillBackfilling}
                  className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-indigo-300 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="Match every embedded skill to top resource chapters via direct ANN. Slow but worth running once after embedding all skills."
                >
                  <Link className={`w-3.5 h-3.5 ${skillBackfilling ? 'animate-spin' : ''}`} />
                  {skillBackfilling ? 'Linking skills…' : 'Link skills → chapters'}
                </button>

                {/* LLM skill extraction backfill */}
                <button
                  onClick={handleLlmSkillExtraction}
                  disabled={llmExtracting}
                  className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-violet-300 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                  title="Enqueue all resources for LLM-based skill extraction (Gemini). Runs in background — fast to enqueue, slow to process."
                >
                  <Wand2 className={`w-3.5 h-3.5 ${llmExtracting ? 'animate-spin' : ''}`} />
                  {llmExtracting ? 'Enqueueing…' : 'Extract skills (LLM)'}
                </button>

                {/* Re-scrape all */}
                {urlResources.length > 0 && (
                  <button
                    onClick={handleRescrapeAll}
                    disabled={bulkRunning}
                    className="flex items-center gap-1.5 text-xs text-slate-400 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                    title={`Re-scrape ${urlResources.length} URL resource${urlResources.length !== 1 ? 's' : ''} with Scrapling`}
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${bulkRunning ? 'animate-spin' : ''}`} />
                    Re-scrape All
                  </button>
                )}
              </div>

              {/* Tag filter chips */}
              {allTags.length > 0 && (
                <div className="flex items-center gap-1.5 flex-wrap">
                  <Tag className="w-3 h-3 text-slate-600 flex-shrink-0" />
                  {allTags.map(tag => (
                    <button
                      key={tag}
                      onClick={() => setSelectedTags(prev =>
                        prev.includes(tag) ? prev.filter(t => t !== tag) : [...prev, tag]
                      )}
                      className={`text-xs px-2 py-0.5 rounded-full transition-colors ${
                        selectedTags.includes(tag)
                          ? 'bg-emerald-900/60 text-emerald-300 border border-emerald-700/60'
                          : 'bg-slate-800/60 text-slate-500 border border-slate-700/40 hover:text-slate-300 hover:border-slate-600'
                      }`}
                    >
                      {tag}
                    </button>
                  ))}
                  {selectedTags.length > 0 && (
                    <button
                      onClick={() => setSelectedTags([])}
                      className="text-xs text-slate-600 hover:text-slate-300 ml-1"
                    >
                      clear
                    </button>
                  )}
                </div>
              )}

              {/* Completion filter */}
              <div className="flex items-center gap-1.5">
                <Check className="w-3 h-3 text-slate-600 flex-shrink-0" />
                {(['all', 'completed', 'incomplete'] as const).map(v => (
                  <button
                    key={v}
                    onClick={() => setCompletionFilter(v)}
                    className={`text-xs px-2 py-0.5 rounded-full transition-colors ${
                      completionFilter === v
                        ? v === 'completed' ? 'bg-emerald-900/60 text-emerald-300 border border-emerald-700/60'
                          : v === 'incomplete' ? 'bg-amber-900/50 text-amber-300 border border-amber-700/60'
                          : 'bg-slate-700 text-slate-300 border border-slate-600'
                        : 'bg-slate-800/60 text-slate-500 border border-slate-700/40 hover:text-slate-300 hover:border-slate-600'
                    }`}
                  >
                    {v === 'all' ? 'All' : v === 'completed' ? 'Completed' : 'Incomplete'}
                  </button>
                ))}
              </div>

              {/* Result toasts */}
              {autoTagResult && (
                <div className={`text-xs px-3 py-1.5 rounded-md ${autoTagResult.startsWith('Failed') ? 'bg-red-950/40 text-red-400 border border-red-800/50' : 'bg-emerald-950/40 text-emerald-400 border border-emerald-800/50'}`}>
                  {autoTagResult}
                </div>
              )}
              {rematchResult && (
                <div className={`text-xs px-3 py-1.5 rounded-md ${rematchResult.startsWith('Failed') ? 'bg-red-950/40 text-red-400 border border-red-800/50' : 'bg-blue-950/40 text-blue-400 border border-blue-800/50'}`}>
                  {rematchResult}
                </div>
              )}
              {reembedResult && (
                <div className={`text-xs px-3 py-1.5 rounded-md ${reembedResult.startsWith('Failed') ? 'bg-red-950/40 text-red-400 border border-red-800/50' : 'bg-amber-950/40 text-amber-400 border border-amber-800/50'}`}>
                  {reembedResult}
                </div>
              )}
              {skillBackfillResult && (
                <div className={`text-xs px-3 py-1.5 rounded-md ${skillBackfillResult.startsWith('Failed') ? 'bg-red-950/40 text-red-400 border border-red-800/50' : 'bg-indigo-950/40 text-indigo-400 border border-indigo-800/50'}`}>
                  {skillBackfillResult}
                </div>
              )}
              {llmExtractResult && (
                <div className={`text-xs px-3 py-1.5 rounded-md ${llmExtractResult.startsWith('Failed') ? 'bg-red-950/40 text-red-400 border border-red-800/50' : 'bg-violet-950/40 text-violet-400 border border-violet-800/50'}`}>
                  {llmExtractResult}
                </div>
              )}

              {/* Count */}
              <p className="text-xs font-medium text-slate-500 uppercase tracking-wider">
                {(() => {
                  const shown = showThin ? filteredResources.length : topLevelFiltered.length;
                  return shown === resources.length
                    ? `${resources.length} ${resources.length === 1 ? 'resource' : 'resources'}`
                    : `${shown} of ${resources.length} resources`;
                })()}
              </p>
            </div>
          )}
          <div className="flex flex-col gap-2">
            {showThin ? (
              // Thin filter: flat list
              <>
                {filteredResources.map(resource => {
                  const state = cardState[resource.id] ?? 'idle';
                  const msg = cardMsg[resource.id] ?? '';
                  const isWebpage = resource.resourceType === 'webpage';
                  const chunkCount = chunkCounts[resource.id] ?? 0;
                  const badge = chunksLoaded ? getChunkBadgeProps(chunkCount) : null;
                  return (
                    <div key={resource.id} className={`group bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-lg p-4 flex items-start gap-3 transition-colors ${resource.isCompleted ? 'opacity-60' : ''}`}>
                      <button
                        onClick={() => handleToggleCompletion(resource.id)}
                        className={`w-8 h-8 rounded-lg border flex items-center justify-center flex-shrink-0 mt-0.5 transition-colors ${
                          resource.isCompleted
                            ? 'bg-emerald-900/40 border-emerald-700/60 text-emerald-400'
                            : 'bg-slate-800 border-slate-700/60 text-slate-400 hover:border-emerald-700 hover:text-emerald-400'
                        }`}
                        title={resource.isCompleted ? 'Mark incomplete' : 'Mark completed'}
                      >
                        {resource.isCompleted ? <Check className="w-4 h-4" /> : getResourceIcon(resource)}
                      </button>
                      <div className="flex-1 min-w-0">
                        <p className={`text-sm font-medium truncate ${resource.isCompleted ? 'text-slate-400 line-through' : 'text-slate-100'}`}>{resource.title}</p>
                        {resource.url && (
                          <a href={resource.url} target="_blank" rel="noopener noreferrer" className="text-xs text-blue-400 hover:text-blue-300 truncate block mt-0.5">{resource.url}</a>
                        )}
                        <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                          <span className="text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-500 border border-slate-700/50">{resource.resourceType}</span>
                          {badge && <span className={`text-xs px-1.5 py-0.5 rounded font-mono ${badge.className}`} title={badge.title}>{badge.label}</span>}
                          {resource.nodeCount > 0 && (
                            <span className="relative">
                              <button
                                onClick={() => handleShowLinkedNodes(resource.id)}
                                className="text-xs px-1.5 py-0.5 rounded bg-blue-950/40 text-blue-400 border border-blue-800/40 hover:bg-blue-900/40 hover:text-blue-300 transition-colors cursor-pointer"
                              >
                                {resource.nodeCount} node{resource.nodeCount !== 1 ? 's' : ''}
                              </button>
                              {nodePopoverFor === resource.id && (
                                <div className="absolute top-full left-0 mt-1 bg-slate-900 border border-slate-700 rounded-md shadow-xl z-20 min-w-[180px] max-w-[280px] max-h-40 overflow-y-auto p-1.5">
                                  {nodePopoverTitles.length > 0 ? nodePopoverTitles.map((t, i) => (
                                    <p key={i} className="text-xs text-slate-300 px-2 py-1 rounded hover:bg-slate-800 truncate">{t}</p>
                                  )) : (
                                    <p className="text-xs text-slate-500 px-2 py-1">No linked nodes</p>
                                  )}
                                </div>
                              )}
                            </span>
                          )}
                          <span className="text-xs text-slate-600">{resource.createdAt && !isNaN(Date.parse(resource.createdAt)) ? new Date(resource.createdAt).toLocaleDateString() : ''}</span>
                          {state !== 'idle' && <span className={`text-xs font-medium ${state === 'loading' ? 'text-slate-500' : state === 'done' ? 'text-emerald-400' : state === 'unchanged' ? 'text-slate-500' : 'text-red-400'}`}>{state === 'loading' ? 'Re-scraping…' : msg}</span>}
                        </div>
                        {/* Tags */}
                        <div className="flex items-center gap-1 mt-1.5 flex-wrap">
                          {resource.tags?.map(tag => (
                            <span key={tag} className="inline-flex items-center gap-0.5 text-xs px-1.5 py-0.5 rounded-full bg-slate-800 text-slate-400 border border-slate-700/50">
                              {tag}
                              <button onClick={(e) => { e.stopPropagation(); handleRemoveTag(resource.id, tag); }} className="text-slate-600 hover:text-red-400 ml-0.5"><X className="w-2.5 h-2.5" /></button>
                            </span>
                          ))}
                          {editingTagsFor === resource.id ? (
                            <div className="relative">
                              <input
                                type="text"
                                value={tagInput}
                                onChange={e => setTagInput(e.target.value)}
                                onKeyDown={e => { if (e.key === 'Enter' && tagInput.trim()) { handleAddTag(resource.id, tagInput); } else if (e.key === 'Escape') { setEditingTagsFor(null); setTagInput(''); } }}
                                onBlur={() => setTimeout(() => { setEditingTagsFor(null); setTagInput(''); }, 150)}
                                placeholder="add tag…"
                                autoFocus
                                className="w-20 bg-slate-950 border border-slate-700 rounded px-1.5 py-0.5 text-xs text-slate-200 focus:outline-none focus:border-emerald-600"
                              />
                              {tagInput && tagSuggestions.length > 0 && (
                                <div className="absolute top-full left-0 mt-1 bg-slate-900 border border-slate-700 rounded-md shadow-xl z-10 max-h-32 overflow-y-auto min-w-[100px]">
                                  {tagSuggestions.filter(t => !resource.tags?.includes(t)).slice(0, 8).map(t => (
                                    <button key={t} onMouseDown={() => handleAddTag(resource.id, t)} className="block w-full text-left px-2 py-1 text-xs text-slate-300 hover:bg-slate-800 hover:text-white">{t}</button>
                                  ))}
                                </div>
                              )}
                            </div>
                          ) : (
                            <button onClick={() => { setEditingTagsFor(resource.id); setTagInput(''); }} className="text-xs text-slate-600 hover:text-emerald-400 px-1" title="Add tag">+</button>
                          )}
                        </div>
                      </div>
                      <div className="flex items-center gap-1 flex-shrink-0">
                        {isWebpage && resource.url && (
                          <button onClick={() => openDiscover(resource)} disabled={discoverLoading || scraperDown} title={scraperDown ? 'Scraper offline' : 'Discover same-domain links'} className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-950/40 disabled:cursor-not-allowed disabled:opacity-40 transition-all">
                            <Search className="w-3.5 h-3.5" />
                          </button>
                        )}
                        {isWebpage && (
                          <button onClick={() => handleRescrapeOne(resource.id)} disabled={state === 'loading'} title="Re-scrape" className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-950/40 disabled:cursor-not-allowed transition-all">
                            <RefreshCw className={`w-3.5 h-3.5 ${state === 'loading' ? 'animate-spin' : ''}`} />
                          </button>
                        )}
                        <button onClick={() => handleDelete(resource.id)} title="Delete" className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-red-400 hover:bg-red-950/40 transition-all">
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>
                  );
                })}
                {filteredResources.length === 0 && (
                  <p className="text-sm text-slate-500 text-center py-4">No matching resources found.</p>
                )}
              </>
            ) : (
              // Grouped view: parent cards + collapsed/expanded children
              topLevelFiltered.map(resource => {
                const state = cardState[resource.id] ?? 'idle';
                const msg = cardMsg[resource.id] ?? '';
                const isWebpage = resource.resourceType === 'webpage';
                const totalChunks = getTotalChunks(resource.id);
                const badge = chunksLoaded ? getChunkBadgeProps(totalChunks) : null;
                const children = childrenByParent[resource.id] || [];
                const hasChildren = children.length > 0;
                const isExpanded = expandedParents.has(resource.id);
                const completedChildren = children.filter(c => c.isCompleted).length;

                return (
                  <div key={resource.id}>
                    {/* Parent card */}
                    <div className={`group bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-lg p-4 flex items-start gap-3 transition-colors ${resource.isCompleted ? 'opacity-60' : ''}`}>
                      <button
                        onClick={() => handleToggleCompletion(resource.id)}
                        className={`w-8 h-8 rounded-lg border flex items-center justify-center flex-shrink-0 mt-0.5 transition-colors ${
                          resource.isCompleted
                            ? 'bg-emerald-900/40 border-emerald-700/60 text-emerald-400'
                            : 'bg-slate-800 border-slate-700/60 text-slate-400 hover:border-emerald-700 hover:text-emerald-400'
                        }`}
                        title={resource.isCompleted ? 'Mark incomplete' : 'Mark completed'}
                      >
                        {resource.isCompleted ? <Check className="w-4 h-4" /> : getResourceIcon(resource)}
                      </button>
                      <div className="flex-1 min-w-0">
                        <p className={`text-sm font-medium truncate ${resource.isCompleted ? 'text-slate-400 line-through' : 'text-slate-100'}`}>{resource.title}</p>
                        {resource.url && (
                          <a href={resource.url} target="_blank" rel="noopener noreferrer" className="text-xs text-blue-400 hover:text-blue-300 truncate block mt-0.5">{resource.url}</a>
                        )}
                        <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                          <span className="text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-500 border border-slate-700/50">{resource.resourceType}</span>
                          {badge && <span className={`text-xs px-1.5 py-0.5 rounded font-mono ${badge.className}`} title={badge.title}>{badge.label}</span>}
                          {resource.nodeCount > 0 && (
                            <span className="relative">
                              <button
                                onClick={() => handleShowLinkedNodes(resource.id)}
                                className="text-xs px-1.5 py-0.5 rounded bg-blue-950/40 text-blue-400 border border-blue-800/40 hover:bg-blue-900/40 hover:text-blue-300 transition-colors cursor-pointer"
                              >
                                {resource.nodeCount} node{resource.nodeCount !== 1 ? 's' : ''}
                              </button>
                              {nodePopoverFor === resource.id && (
                                <div className="absolute top-full left-0 mt-1 bg-slate-900 border border-slate-700 rounded-md shadow-xl z-20 min-w-[180px] max-w-[280px] max-h-40 overflow-y-auto p-1.5">
                                  {nodePopoverTitles.length > 0 ? nodePopoverTitles.map((t, i) => (
                                    <p key={i} className="text-xs text-slate-300 px-2 py-1 rounded hover:bg-slate-800 truncate">{t}</p>
                                  )) : (
                                    <p className="text-xs text-slate-500 px-2 py-1">No linked nodes</p>
                                  )}
                                </div>
                              )}
                            </span>
                          )}
                          {hasChildren && (
                            <button
                              onClick={() => setExpandedParents(prev => { const next = new Set(prev); next.has(resource.id) ? next.delete(resource.id) : next.add(resource.id); return next; })}
                              className="flex items-center gap-1 text-xs px-1.5 py-0.5 rounded bg-slate-800 text-slate-400 border border-slate-700/50 hover:text-slate-200 hover:border-slate-600 transition-colors"
                            >
                              {isExpanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                              {completedChildren > 0
                                ? <><span className="text-emerald-400">{completedChildren}</span>/{children.length} videos</>
                                : <>{children.length} video{children.length !== 1 ? 's' : ''}</>
                              }
                            </button>
                          )}
                          <span className="text-xs text-slate-600">{resource.createdAt && !isNaN(Date.parse(resource.createdAt)) ? new Date(resource.createdAt).toLocaleDateString() : ''}</span>
                          {state !== 'idle' && <span className={`text-xs font-medium ${state === 'loading' ? 'text-slate-500' : state === 'done' ? 'text-emerald-400' : state === 'unchanged' ? 'text-slate-500' : 'text-red-400'}`}>{state === 'loading' ? 'Re-scraping…' : msg}</span>}
                        </div>
                        {/* Tags */}
                        <div className="flex items-center gap-1 mt-1.5 flex-wrap">
                          {resource.tags?.map(tag => (
                            <span key={tag} className="inline-flex items-center gap-0.5 text-xs px-1.5 py-0.5 rounded-full bg-slate-800 text-slate-400 border border-slate-700/50">
                              {tag}
                              <button onClick={(e) => { e.stopPropagation(); handleRemoveTag(resource.id, tag); }} className="text-slate-600 hover:text-red-400 ml-0.5"><X className="w-2.5 h-2.5" /></button>
                            </span>
                          ))}
                          {editingTagsFor === resource.id ? (
                            <div className="relative">
                              <input
                                type="text"
                                value={tagInput}
                                onChange={e => setTagInput(e.target.value)}
                                onKeyDown={e => { if (e.key === 'Enter' && tagInput.trim()) { handleAddTag(resource.id, tagInput); } else if (e.key === 'Escape') { setEditingTagsFor(null); setTagInput(''); } }}
                                onBlur={() => setTimeout(() => { setEditingTagsFor(null); setTagInput(''); }, 150)}
                                placeholder="add tag…"
                                autoFocus
                                className="w-20 bg-slate-950 border border-slate-700 rounded px-1.5 py-0.5 text-xs text-slate-200 focus:outline-none focus:border-emerald-600"
                              />
                              {tagInput && tagSuggestions.length > 0 && (
                                <div className="absolute top-full left-0 mt-1 bg-slate-900 border border-slate-700 rounded-md shadow-xl z-10 max-h-32 overflow-y-auto min-w-[100px]">
                                  {tagSuggestions.filter(t => !resource.tags?.includes(t)).slice(0, 8).map(t => (
                                    <button key={t} onMouseDown={() => handleAddTag(resource.id, t)} className="block w-full text-left px-2 py-1 text-xs text-slate-300 hover:bg-slate-800 hover:text-white">{t}</button>
                                  ))}
                                </div>
                              )}
                            </div>
                          ) : (
                            <button onClick={() => { setEditingTagsFor(resource.id); setTagInput(''); }} className="text-xs text-slate-600 hover:text-emerald-400 px-1" title="Add tag">+</button>
                          )}
                        </div>
                      </div>
                      <div className="flex items-center gap-1 flex-shrink-0">
                        {isWebpage && resource.url && (
                          <button onClick={() => openDiscover(resource)} disabled={discoverLoading || scraperDown} title={scraperDown ? 'Scraper offline' : 'Discover same-domain links'} className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-950/40 disabled:cursor-not-allowed disabled:opacity-40 transition-all">
                            <Search className="w-3.5 h-3.5" />
                          </button>
                        )}
                        {isWebpage && (
                          <button onClick={() => handleRescrapeOne(resource.id)} disabled={state === 'loading'} title="Re-scrape" className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-950/40 disabled:cursor-not-allowed transition-all">
                            <RefreshCw className={`w-3.5 h-3.5 ${state === 'loading' ? 'animate-spin' : ''}`} />
                          </button>
                        )}
                        <button onClick={() => handleDelete(resource.id)} title="Delete" className="opacity-0 group-hover:opacity-100 p-1.5 rounded text-slate-600 hover:text-red-400 hover:bg-red-950/40 transition-all">
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>

                    {/* Children (when expanded) */}
                    {isExpanded && hasChildren && (
                      <div className="ml-8 pl-4 border-l border-slate-700/40 flex flex-col gap-1.5 mt-1.5">
                        {children.map(child => {
                          const isYouTube = !!child.url?.includes('youtube.com/watch');
                          const childChunks = chunkCounts[child.id] ?? 0;
                          const childBadge = chunksLoaded
                            ? (isYouTube ? getYouTubeBadgeProps(childChunks) : getChunkBadgeProps(childChunks))
                            : null;
                          return (
                            <div key={child.id} className={`group bg-slate-900/60 border border-slate-800/60 hover:border-slate-700 rounded-lg px-3 py-2.5 flex items-center gap-2.5 transition-colors ${child.isCompleted ? 'opacity-60' : ''}`}>
                              <button
                                onClick={() => handleToggleCompletion(child.id)}
                                className={`w-5 h-5 rounded border flex items-center justify-center flex-shrink-0 transition-colors ${
                                  child.isCompleted
                                    ? 'bg-emerald-900/40 border-emerald-700/60 text-emerald-400'
                                    : 'bg-slate-800 border-slate-700/40 text-slate-600 hover:border-emerald-700 hover:text-emerald-400'
                                }`}
                                title={child.isCompleted ? 'Mark incomplete' : 'Mark complete'}
                              >
                                {child.isCompleted ? <Check className="w-3 h-3" /> : getResourceIcon(child)}
                              </button>
                              <div className="flex-1 min-w-0">
                                <p className={`text-xs font-medium truncate ${child.isCompleted ? 'text-slate-500 line-through' : 'text-slate-300'}`}>{child.title}</p>
                                {child.url && (
                                  <a href={child.url} target="_blank" rel="noopener noreferrer" className="text-xs text-blue-500 hover:text-blue-400 truncate block">{child.url}</a>
                                )}
                                {childBadge && (
                                  <span className={`text-xs px-1.5 py-0.5 rounded font-mono mt-0.5 inline-block ${childBadge.className}`}>{childBadge.label}</span>
                                )}
                              </div>
                              <button onClick={() => handleDelete(child.id)} title="Delete" className="opacity-0 group-hover:opacity-100 p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-950/40 transition-all flex-shrink-0">
                                <Trash2 className="w-3 h-3" />
                              </button>
                            </div>
                          );
                        })}
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </>
      ))}

      {/* Playlist modal */}
      {playlistData && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-lg shadow-2xl flex flex-col gap-4">
            {/* Header */}
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-white font-semibold text-sm">{playlistData.playlistTitle}</h3>
                <p className="text-xs text-slate-500 mt-0.5">{playlistData.videos.length} videos</p>
              </div>
              {!playlistProgress && (
                <button onClick={closePlaylistModal} className="text-slate-500 hover:text-white transition-colors">
                  <X className="w-4 h-4" />
                </button>
              )}
            </div>

            {/* Video checklist */}
            {!playlistDone && (
              <>
                <div className="max-h-64 overflow-y-auto flex flex-col gap-0.5 pr-1">
                  {playlistData.videos.map((video, i) => (
                    <label
                      key={video.videoId}
                      className="flex items-start gap-2.5 cursor-pointer hover:bg-slate-800/50 rounded px-2 py-1.5 transition-colors"
                    >
                      <input
                        type="checkbox"
                        checked={video.checked}
                        onChange={() => setPlaylistData(prev => prev ? {
                          ...prev,
                          videos: prev.videos.map((v, j) => j === i ? { ...v, checked: !v.checked } : v),
                        } : null)}
                        className="mt-0.5 w-3.5 h-3.5 rounded accent-emerald-500 flex-shrink-0"
                      />
                      <div className="flex-1 min-w-0">
                        <p className="text-xs text-slate-200 truncate">{video.title}</p>
                        <p className="text-xs text-slate-600 font-mono">#{video.position + 1}</p>
                      </div>
                    </label>
                  ))}
                </div>
                <div className="pt-2 border-t border-slate-700/60 flex flex-col gap-2">
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex gap-3">
                      <button
                        onClick={() => setPlaylistData(prev => prev ? { ...prev, videos: prev.videos.map(v => ({ ...v, checked: true })) } : null)}
                        className="text-xs text-slate-400 hover:text-white transition-colors"
                      >
                        Select All
                      </button>
                      <button
                        onClick={() => setPlaylistData(prev => prev ? { ...prev, videos: prev.videos.map(v => ({ ...v, checked: false })) } : null)}
                        className="text-xs text-slate-400 hover:text-white transition-colors"
                      >
                        Deselect All
                      </button>
                    </div>
                    <button
                      onClick={handleIngestPlaylist}
                      disabled={playlistData.videos.filter(v => v.checked).length === 0 || !!playlistProgress}
                      className="px-3 py-1.5 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md text-xs font-medium transition-colors min-w-[160px] text-center"
                    >
                      {playlistProgress
                        ? `${playlistProgress.skipping ? 'Skipping' : 'Ingesting'} ${playlistProgress.current}/${playlistProgress.total}…`
                        : (() => {
                            const n = playlistData.videos.filter(v => v.checked).length;
                            return `Ingest ${n} Video${n !== 1 ? 's' : ''}`;
                          })()
                      }
                    </button>
                  </div>
                  {playlistProgress && playlistProgress.currentTitle && (
                    <p className={`text-xs truncate ${playlistProgress.skipping ? 'text-amber-400' : 'text-slate-500'}`}>
                      {playlistProgress.skipping ? '↷ Already ingested: ' : '→ '}{playlistProgress.currentTitle}
                    </p>
                  )}
                </div>
              </>
            )}

            {/* Done state */}
            {playlistDone && (
              <div className="flex flex-col gap-3 items-center py-2">
                <div className="flex items-center gap-2 text-emerald-400">
                  <CheckCircle2 className="w-4 h-4" />
                  <span className="text-sm font-medium">Playlist ingested</span>
                </div>
                <button
                  onClick={closePlaylistModal}
                  className="w-full py-2 bg-slate-700 hover:bg-slate-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Close
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {/* External links modal (resource_list pages) */}
      {extLinksData && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-lg shadow-2xl flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-white font-semibold text-sm">Linked Resources Found</h3>
                <p className="text-xs text-slate-500 mt-0.5 truncate">From: {extLinksData.parentTitle}</p>
              </div>
              {!extLinksProgress && (
                <button onClick={closeExtLinksModal} className="text-slate-500 hover:text-white transition-colors">
                  <X className="w-4 h-4" />
                </button>
              )}
            </div>

            {!extLinksDone && (
              <>
                <p className="text-xs text-slate-500 -mb-2">
                  {extLinksData.links.length} external link{extLinksData.links.length !== 1 ? 's' : ''} detected
                  {' — '}{extLinksData.links.filter(l => l.checked).length} selected
                </p>
                <div className="max-h-64 overflow-y-auto flex flex-col gap-0.5 pr-1">
                  {extLinksData.links.map((link, i) => (
                    <label
                      key={i}
                      className="flex items-start gap-2.5 cursor-pointer hover:bg-slate-800/50 rounded px-2 py-1.5 transition-colors"
                    >
                      <input
                        type="checkbox"
                        checked={link.checked}
                        disabled={!!extLinksProgress}
                        onChange={() => setExtLinksData(prev => prev ? {
                          ...prev,
                          links: prev.links.map((l, j) => j === i ? { ...l, checked: !l.checked } : l),
                        } : null)}
                        className="mt-0.5 w-3.5 h-3.5 rounded accent-emerald-500 flex-shrink-0"
                      />
                      <span className="mt-0.5 flex-shrink-0 text-slate-500">
                        {link.type === 'youtube' ? <Play className="w-3 h-3 fill-current" /> : <Globe className="w-3 h-3" />}
                      </span>
                      <div className="flex-1 min-w-0">
                        <p className="text-xs text-slate-200 truncate">{link.text || link.url}</p>
                        <p className="text-xs text-slate-500 truncate font-mono">{link.url}</p>
                      </div>
                    </label>
                  ))}
                </div>
                <div className="pt-2 border-t border-slate-700/60 flex flex-col gap-2">
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex gap-3">
                      <button
                        onClick={() => setExtLinksData(prev => prev ? { ...prev, links: prev.links.map(l => ({ ...l, checked: true })) } : null)}
                        disabled={!!extLinksProgress}
                        className="text-xs text-slate-400 hover:text-white transition-colors disabled:opacity-40"
                      >
                        Select All
                      </button>
                      <button
                        onClick={() => setExtLinksData(prev => prev ? { ...prev, links: prev.links.map(l => ({ ...l, checked: false })) } : null)}
                        disabled={!!extLinksProgress}
                        className="text-xs text-slate-400 hover:text-white transition-colors disabled:opacity-40"
                      >
                        Deselect All
                      </button>
                    </div>
                    <button
                      onClick={handleIngestExternalLinks}
                      disabled={extLinksData.links.filter(l => l.checked).length === 0 || !!extLinksProgress}
                      className="px-3 py-1.5 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md text-xs font-medium transition-colors min-w-[160px] text-center"
                    >
                      {extLinksProgress
                        ? `${extLinksProgress.skipping ? 'Skipping' : 'Ingesting'} ${extLinksProgress.current}/${extLinksProgress.total}…`
                        : (() => {
                            const n = extLinksData.links.filter(l => l.checked).length;
                            return `Ingest ${n} Link${n !== 1 ? 's' : ''} as Children`;
                          })()
                      }
                    </button>
                  </div>
                  {extLinksProgress && extLinksProgress.currentTitle && (
                    <p className={`text-xs truncate ${extLinksProgress.skipping ? 'text-amber-400' : 'text-slate-500'}`}>
                      {extLinksProgress.skipping ? '↷ Already ingested: ' : '→ '}{extLinksProgress.currentTitle}
                    </p>
                  )}
                </div>
              </>
            )}

            {extLinksDone && (
              <div className="flex flex-col gap-3 items-center py-2">
                <div className="flex items-center gap-2 text-emerald-400">
                  <CheckCircle2 className="w-4 h-4" />
                  <span className="text-sm font-medium">Links ingested as children</span>
                </div>
                <button
                  onClick={closeExtLinksModal}
                  className="w-full py-2 bg-slate-700 hover:bg-slate-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Close
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Discover links modal */}
      {discoverFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-lg shadow-2xl flex flex-col gap-4">
            {/* Header */}
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Search className="w-4 h-4 text-emerald-400" />
                <h3 className="text-white font-semibold text-sm">Discover Links</h3>
              </div>
              {!discoverProgress && (
                <button onClick={closeDiscover} className="text-slate-500 hover:text-white transition-colors">
                  <X className="w-4 h-4" />
                </button>
              )}
            </div>
            <p className="text-xs text-slate-500 truncate -mt-2">{discoverFor.url}</p>

            {/* Loading */}
            {discoverLoading && (
              <div className="flex items-center justify-center gap-2 text-sm text-slate-400 py-6">
                <svg className="animate-spin h-4 w-4 flex-shrink-0" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none"/>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"/>
                </svg>
                <span>Fetching page links…</span>
              </div>
            )}

            {/* Error */}
            {discoverError && !discoverLoading && (
              <div className="text-sm text-red-400 bg-red-950/40 border border-red-800/50 rounded-lg px-3 py-2">
                {discoverError}
              </div>
            )}

            {/* Empty */}
            {!discoverLoading && !discoverError && discoverLinks.length === 0 && (
              <p className="text-sm text-slate-500 text-center py-4">No same-domain links found on this page.</p>
            )}

            {/* Links checklist */}
            {discoverLinks.length > 0 && !discoverDone && (
              <>
                <p className="text-xs text-slate-500 -mb-2">
                  Found {discoverLinks.length} same-domain link{discoverLinks.length !== 1 ? 's' : ''}
                  {' — '}{discoverLinks.filter(l => l.checked).length} selected
                </p>
                <div className="max-h-64 overflow-y-auto flex flex-col gap-0.5 pr-1">
                  {discoverLinks.map((link, i) => (
                    <label
                      key={i}
                      className="flex items-start gap-2.5 cursor-pointer hover:bg-slate-800/50 rounded px-2 py-1.5 transition-colors"
                    >
                      <input
                        type="checkbox"
                        checked={link.checked}
                        onChange={() => setDiscoverLinks(prev => prev.map((l, j) => j === i ? { ...l, checked: !l.checked } : l))}
                        className="mt-0.5 w-3.5 h-3.5 rounded accent-emerald-500 flex-shrink-0"
                      />
                      <div className="flex-1 min-w-0">
                        <p className="text-xs text-slate-200 truncate">{link.text || link.url}</p>
                        <p className="text-xs text-slate-500 truncate font-mono">{link.url}</p>
                      </div>
                    </label>
                  ))}
                </div>
                <div className="pt-2 border-t border-slate-700/60 flex items-center justify-between gap-3">
                  <div className="flex gap-3">
                    <button
                      onClick={() => setDiscoverLinks(prev => prev.map(l => ({ ...l, checked: true })))}
                      className="text-xs text-slate-400 hover:text-white transition-colors"
                    >
                      Select All
                    </button>
                    <button
                      onClick={() => setDiscoverLinks(prev => prev.map(l => ({ ...l, checked: false })))}
                      className="text-xs text-slate-400 hover:text-white transition-colors"
                    >
                      Deselect All
                    </button>
                  </div>
                  <button
                    onClick={handleIngestSelected}
                    disabled={discoverLinks.filter(l => l.checked).length === 0 || !!discoverProgress}
                    className="px-3 py-1.5 bg-emerald-600 hover:bg-emerald-700 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed text-white rounded-md text-xs font-medium transition-colors min-w-[140px] text-center"
                  >
                    {discoverProgress
                      ? `Ingesting ${discoverProgress.current}/${discoverProgress.total}…`
                      : `Ingest ${discoverLinks.filter(l => l.checked).length} Selected`
                    }
                  </button>
                </div>
              </>
            )}

            {/* Done */}
            {discoverDone && (
              <div className="flex flex-col gap-3 items-center py-2">
                <div className="flex items-center gap-2 text-emerald-400">
                  <CheckCircle2 className="w-4 h-4" />
                  <span className="text-sm font-medium">Done — URLs queued for ingestion</span>
                </div>
                <button
                  onClick={closeDiscover}
                  className="w-full py-2 bg-slate-700 hover:bg-slate-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Close
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Bulk rescrape modal */}
      {bulkOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="bg-slate-900 border border-slate-700 rounded-xl p-6 w-full max-w-lg shadow-2xl flex flex-col gap-4">
            {/* Modal header */}
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <RefreshCw className={`w-4 h-4 text-blue-400 ${bulkRunning ? 'animate-spin' : ''}`} />
                <h3 className="text-white font-semibold text-sm">
                  {bulkDone
                    ? 'Re-scrape Complete'
                    : bulkCurrent
                      ? `${bulkCurrent.index}/${bulkCurrent.total} URLs…`
                      : `Re-scraping ${urlResources.length} URL${urlResources.length !== 1 ? 's' : ''}…`}
                </h3>
              </div>
              {bulkDone && (
                <button onClick={closeBulkModal} className="text-slate-500 hover:text-white transition-colors">
                  <X className="w-4 h-4" />
                </button>
              )}
            </div>

            {/* Progress list */}
            <div className="max-h-72 overflow-y-auto flex flex-col gap-1.5 pr-1">
              {bulkProgress.map((p, i) => (
                <div key={i} className="flex items-center gap-2.5 text-sm">
                  {p.status === 'improved' ? (
                    <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0" />
                  ) : p.status === 'unchanged' ? (
                    <Minus className="w-4 h-4 text-slate-500 flex-shrink-0" />
                  ) : (
                    <XCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
                  )}
                  <span className="flex-1 truncate text-slate-300 text-xs">{p.title}</span>
                  {p.status === 'improved' && (
                    <span className="text-emerald-400 text-xs font-mono flex-shrink-0">{p.oldChunks}→{p.newChunks}</span>
                  )}
                  {p.status === 'unchanged' && (
                    <span className="text-slate-600 text-xs flex-shrink-0">{p.oldChunks} chunks</span>
                  )}
                </div>
              ))}
              {bulkCurrent && (
                <div className="flex items-center gap-2.5 text-xs text-slate-400">
                  <RefreshCw className="w-3.5 h-3.5 animate-spin flex-shrink-0 text-blue-400" />
                  <span className="flex-1 truncate">{bulkCurrent.title}</span>
                  <span className="text-slate-600 flex-shrink-0">{bulkCurrent.index}/{bulkCurrent.total}</span>
                </div>
              )}
              {!bulkDone && !bulkCurrent && bulkProgress.length === 0 && (
                <div className="flex items-center gap-2.5 text-xs text-slate-500">
                  <RefreshCw className="w-3.5 h-3.5 animate-spin flex-shrink-0" />
                  <span>Starting…</span>
                </div>
              )}
            </div>

            {/* Summary */}
            {bulkSummary && (
              <div className="pt-3 border-t border-slate-700/60 flex gap-4 text-sm">
                <span className="text-emerald-400">{bulkSummary.improved} improved</span>
                <span className="text-slate-500">{bulkSummary.unchanged} unchanged</span>
                {bulkSummary.failed > 0 && (
                  <span className="text-red-400">{bulkSummary.failed} failed</span>
                )}
              </div>
            )}

            {bulkDone && (
              <button
                onClick={closeBulkModal}
                className="w-full py-2 bg-slate-700 hover:bg-slate-600 text-white rounded-lg text-sm font-medium transition-colors"
              >
                Close
              </button>
            )}
          </div>
        </div>
      )}

      {/* Playlist success toast */}
      {playlistToast && (
        <div className="fixed bottom-6 right-6 z-50 flex items-start gap-3 bg-emerald-950/90 border border-emerald-700/60 rounded-lg px-4 py-3 shadow-xl max-w-sm">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium text-emerald-300">{playlistToast}</p>
          </div>
          <button
            onClick={() => setPlaylistToast(null)}
            className="flex-shrink-0 text-emerald-500 hover:text-emerald-300 transition-colors mt-0.5"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Duplicate warning toast */}
      {dupWarning && (
        <div className="fixed bottom-6 right-6 z-50 flex items-start gap-3 bg-amber-950/90 border border-amber-700/60 rounded-lg px-4 py-3 shadow-xl max-w-sm">
          <div className="flex-1 min-w-0">
            <p className="text-sm font-medium text-amber-300">Already in your library</p>
            <p className="text-xs text-amber-400/80 mt-0.5 truncate">{dupWarning}</p>
          </div>
          <button
            onClick={() => setDupWarning(null)}
            className="flex-shrink-0 text-amber-500 hover:text-amber-300 transition-colors mt-0.5"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      )}
    </div>
  );
}
