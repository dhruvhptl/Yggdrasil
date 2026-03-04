// src/components/YggdrasilTree.tsx
// Organic SVG skill tree: trunk at bottom, branches grow upward, leaves at tips.

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MimirResource } from '../types';

// ─── Types ───────────────────────────────────────────────────────────────────

interface TreeNode {
  id: string;
  tree_id: string;
  parent_id: string | null;
  type: 'trunk' | 'branch' | 'leaf';
  title: string;
  description: string;
  progress: number; // 0–100
  tasks: any[];
  resources: any[] | null;
  x: number | null;
  y: number | null;
  order_index: number;
  is_locked: boolean;
}

interface TreeEdge {
  id: string;
  tree_id: string;
  source_node_id: string;
  target_node_id: string;
}

interface NodePos {
  x: number;
  y: number;
  angle: number;
}

interface YggdrasilTreeProps {
  projectId: string;
}

// ─── Layout helpers ───────────────────────────────────────────────────────────

const DEG = Math.PI / 180;

function computeLayout(nodes: TreeNode[]): Map<string, NodePos> {
  const positions = new Map<string, NodePos>();
  const childrenOf = new Map<string | null, TreeNode[]>();

  nodes.forEach(n => {
    const key = n.parent_id ?? null;
    if (!childrenOf.has(key)) childrenOf.set(key, []);
    childrenOf.get(key)!.push(n);
  });

  childrenOf.forEach(arr => arr.sort((a, b) => a.order_index - b.order_index));

  const roots = childrenOf.get(null) ?? [];

  function branchLength(type: string): number {
    if (type === 'trunk') return 160;
    if (type === 'branch') return 130;
    return 90;
  }

  function place(node: TreeNode, x: number, y: number, angle: number) {
    positions.set(node.id, { x, y, angle });
    const children = childrenOf.get(node.id) ?? [];
    if (children.length === 0) return;

    const len = branchLength(node.type);
    const maxSpread = node.type === 'trunk' ? 80 : 60;
    const spread = Math.min(maxSpread, children.length * 25);
    const start = angle - spread / 2;
    const step = children.length > 1 ? spread / (children.length - 1) : 0;

    children.forEach((child, i) => {
      const childAngle = start + i * step;
      const rad = childAngle * DEG;
      place(child, x + len * Math.cos(rad), y + len * Math.sin(rad), childAngle);
    });
  }

  if (roots.length === 1) {
    place(roots[0], 500, 680, -90);
  } else if (roots.length > 1) {
    const spread = Math.min(100, roots.length * 28);
    const startAngle = -90 - spread / 2;
    const step = roots.length > 1 ? spread / (roots.length - 1) : 0;

    roots.forEach((root, i) => {
      const angle = startAngle + i * step;
      const rad = angle * DEG;
      const x = 500 + 160 * Math.cos(rad);
      const y = 760 + 160 * Math.sin(rad);
      place(root, x, y, angle);
    });
  }

  return positions;
}

function branchPath(x1: number, y1: number, x2: number, y2: number): string {
  const midY = (y1 + y2) / 2;
  const dx = x2 - x1;
  const cp1x = x1 + dx * 0.15;
  const cp2x = x2 - dx * 0.15;
  return `M ${x1} ${y1} C ${cp1x} ${midY} ${cp2x} ${midY} ${x2} ${y2}`;
}

function nodeColor(node: TreeNode): string {
  if (node.progress === 100) return '#10b981';
  if (node.progress > 0) return '#059669';
  if (node.type === 'leaf') return '#374151';
  return '#1d4027';
}

function nodeRadius(type: string): number {
  if (type === 'trunk') return 16;
  if (type === 'branch') return 12;
  return 8;
}

function branchStroke(parentType: string): number {
  return parentType === 'trunk' ? 5 : 3;
}

function truncate(str: string, max: number): string {
  return str.length > max ? str.slice(0, max) + '…' : str;
}

// ─── Node Panel ───────────────────────────────────────────────────────────────

interface NodePanelProps {
  node: TreeNode;
  skillLocked: boolean;
  onSave: (updates: Partial<TreeNode>) => void;
  onClose: () => void;
  onAddChild: () => void;
  onDelete: () => void;
}

function NodePanel({ node, skillLocked, onSave, onClose, onAddChild, onDelete }: NodePanelProps) {
  const [title, setTitle] = useState(node.title);
  const [description, setDescription] = useState(node.description);
  const [resources, setResources] = useState<string[]>(
    Array.isArray(node.resources) ? node.resources.filter(r => typeof r === 'string') : []
  );
  const [mimiResources, setMimiResources] = useState<MimirResource[]>([]);
  const [matching, setMatching] = useState(false);
  const task = node.type === 'leaf' && Array.isArray(node.tasks) ? node.tasks[0] : null;
  const [notesText, setNotesText] = useState(task?.notes ?? '');
  const notesTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const currentTask = node.type === 'leaf' && Array.isArray(node.tasks) ? node.tasks[0] : null;
    setTitle(node.title);
    setDescription(node.description);
    setResources(
      Array.isArray(node.resources) ? node.resources.filter(r => typeof r === 'string') : []
    );
    setNotesText(currentTask?.notes ?? '');
    if (notesTimerRef.current) clearTimeout(notesTimerRef.current);
    // Load Mimir-linked resources for this node (best-effort)
    setMimiResources([]);
    invoke<MimirResource[]>('get_node_resources', { nodeId: node.id })
      .then(setMimiResources)
      .catch(() => { /* sidecar not running — silently ignore */ });
  }, [node.id]);

  // Cleanup debounce on unmount
  useEffect(() => {
    return () => { if (notesTimerRef.current) clearTimeout(notesTimerRef.current); };
  }, []);

  function handleNotesChange(value: string) {
    setNotesText(value);
    if (notesTimerRef.current) clearTimeout(notesTimerRef.current);
    notesTimerRef.current = setTimeout(async () => {
      if (!task) return;
      try {
        await invoke('update_tree_node', {
          nodeId: node.id,
          title: null, description: null, progress: null,
          tasks: [{ ...task, notes: value }],
          resources: null, position: null,
        });
      } catch { /* silently ignore auto-save errors */ }
    }, 1000);
  }

  async function handleFindMatches() {
    setMatching(true);
    try {
      const matched = await invoke<MimirResource[]>('match_node_to_resources', { nodeId: node.id });
      if (matched.length > 0) setMimiResources(matched);
    } catch {
      // sidecar not running — silently ignore
    } finally {
      setMatching(false);
    }
  }

  function handleSave() {
    onSave({ title, description, resources: resources.length > 0 ? resources : null });
  }

  function handleToggleComplete() {
    if (!task) return;
    const newCompleted = !task.completed;
    onSave({
      progress: newCompleted ? 100 : 0,
      tasks: [{ ...task, completed: newCompleted }],
    });
  }

  const DIFF_COLOR: Record<string, string> = {
    easy: '#6ee7b7',
    medium: '#fde68a',
    hard: '#fca5a5',
  };

  // Locked panel — all hooks already called above, safe to return early here
  if (skillLocked) {
    return (
      <div style={{
        position: 'absolute', top: 0, right: 0,
        width: 320, height: '100%',
        background: '#0f172a', borderLeft: '1px solid #1e293b',
        display: 'flex', flexDirection: 'column', zIndex: 100,
      }}>
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '10px 14px', borderBottom: '1px solid #1e293b', flexShrink: 0,
        }}>
          <span style={{ fontSize: 11, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.07em' }}>
            Locked
          </span>
          <button
            onClick={onClose}
            style={{ background: 'none', border: 'none', color: '#64748b', cursor: 'pointer', fontSize: 20, lineHeight: 1, padding: 0 }}
          >
            ×
          </button>
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: '24px', textAlign: 'center', gap: 12 }}>
          <span style={{ fontSize: 32 }}>🔒</span>
          <p style={{ fontSize: 13, color: '#475569', lineHeight: 1.6, margin: 0 }}>
            Complete all quests in the previous skill to unlock this one.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div style={{
      position: 'absolute', top: 0, right: 0,
      width: 320, height: '100%',
      background: '#0f172a',
      borderLeft: '1px solid #1e293b',
      display: 'flex', flexDirection: 'column',
      zIndex: 100,
    }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        padding: '10px 14px',
        borderBottom: '1px solid #1e293b',
        flexShrink: 0,
      }}>
        <span style={{ fontSize: 11, color: '#64748b', textTransform: 'uppercase', letterSpacing: '0.07em' }}>
          {node.type}
        </span>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{
            fontSize: 12, fontWeight: 600,
            color: node.progress === 100 ? '#10b981' : node.progress > 0 ? '#059669' : '#475569',
          }}>
            {node.progress}%
          </span>
          <button
            onClick={onClose}
            style={{ background: 'none', border: 'none', color: '#64748b', cursor: 'pointer', fontSize: 20, lineHeight: 1, padding: 0 }}
          >
            ×
          </button>
        </div>
      </div>

      {/* Scrollable body */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '14px' }}>

        <div style={{ marginBottom: 12 }}>
          <label style={{ display: 'block', fontSize: 10, color: '#64748b', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
            Title
          </label>
          <input
            value={title}
            onChange={e => setTitle(e.target.value)}
            style={{
              width: '100%', padding: '6px 10px',
              background: '#1e293b', border: '1px solid #334155',
              borderRadius: 6, color: '#f1f5f9', fontSize: 13,
              boxSizing: 'border-box',
            }}
          />
        </div>

        <div style={{ marginBottom: 12 }}>
          <label style={{ display: 'block', fontSize: 10, color: '#64748b', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
            Notes
          </label>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            rows={3}
            style={{
              width: '100%', padding: '6px 10px',
              background: '#1e293b', border: '1px solid #334155',
              borderRadius: 6, color: '#f1f5f9', fontSize: 12,
              resize: 'vertical', boxSizing: 'border-box',
            }}
          />
        </div>

        {/* Quest details — leaf nodes only */}
        {node.type === 'leaf' && task && (
          <div style={{
            marginBottom: 12, padding: 10,
            background: '#1e293b', borderRadius: 8, border: '1px solid #334155',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
              <span style={{ fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: '0.07em' }}>Quest</span>
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                {task.difficulty && (
                  <span style={{
                    fontSize: 10, padding: '2px 6px', borderRadius: 4,
                    background: (DIFF_COLOR[task.difficulty] || '#94a3b8') + '22',
                    color: DIFF_COLOR[task.difficulty] || '#94a3b8',
                    border: `1px solid ${(DIFF_COLOR[task.difficulty] || '#94a3b8')}44`,
                  }}>
                    {task.difficulty}
                  </span>
                )}
                {task.estimated_hours != null && (
                  <span style={{ fontSize: 10, color: '#64748b' }}>~{task.estimated_hours}h</span>
                )}
              </div>
            </div>
            {task.description && (
              <p style={{ fontSize: 12, color: '#94a3b8', lineHeight: 1.5, margin: '0 0 8px 0' }}>
                {task.description}
              </p>
            )}
            <button
              onClick={handleToggleComplete}
              style={{ display: 'flex', alignItems: 'center', gap: 8, background: 'none', border: 'none', cursor: 'pointer', padding: 0 }}
            >
              <div style={{
                width: 16, height: 16, borderRadius: 4,
                border: `2px solid ${task.completed ? '#10b981' : '#475569'}`,
                background: task.completed ? '#10b981' : 'transparent',
                display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
              }}>
                {task.completed && (
                  <svg width="9" height="9" viewBox="0 0 10 10" fill="none">
                    <path d="M1.5 5L4 7.5L8.5 2.5" stroke="white" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                )}
              </div>
              <span style={{ fontSize: 12, color: task.completed ? '#10b981' : '#94a3b8' }}>
                {task.completed ? 'Completed' : 'Mark complete'}
              </span>
            </button>
          </div>
        )}

        {/* Quest notes — leaf nodes only */}
        {node.type === 'leaf' && task && (
          <div style={{ marginBottom: 12 }}>
            <label style={{ display: 'block', fontSize: 10, color: '#64748b', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
              My notes
            </label>
            <textarea
              value={notesText}
              onChange={e => handleNotesChange(e.target.value)}
              rows={4}
              placeholder="What did you learn? What confused you? Key takeaways…"
              style={{
                width: '100%', padding: '6px 10px',
                background: '#1e293b', border: '1px solid #334155',
                borderRadius: 6, color: '#f1f5f9', fontSize: 12,
                resize: 'vertical', boxSizing: 'border-box', fontFamily: 'inherit',
              }}
            />
            <p style={{ fontSize: 9, color: '#475569', marginTop: 3 }}>Auto-saves as you type</p>
          </div>
        )}

        {/* Resources */}
        <div style={{ marginBottom: 12 }}>
          <label style={{ display: 'block', fontSize: 10, color: '#64748b', marginBottom: 6, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
            Resources
          </label>
          {resources.map((url, i) => (
            <div key={i} style={{ display: 'flex', gap: 5, marginBottom: 5 }}>
              <input
                value={url}
                onChange={e => {
                  const next = [...resources];
                  next[i] = e.target.value;
                  setResources(next);
                }}
                placeholder="https://..."
                style={{
                  flex: 1, padding: '5px 8px',
                  background: '#1e293b', border: '1px solid #334155',
                  borderRadius: 5, color: '#f1f5f9', fontSize: 11,
                }}
              />
              <button
                onClick={() => setResources(resources.filter((_, j) => j !== i))}
                style={{
                  padding: '4px 7px', background: '#450a0a', color: '#fca5a5',
                  border: 'none', borderRadius: 5, cursor: 'pointer', fontSize: 12,
                }}
              >
                ×
              </button>
            </div>
          ))}
          <button
            onClick={() => setResources([...resources, ''])}
            style={{
              width: '100%', padding: '5px 0',
              background: '#1e293b', border: '1px dashed #334155',
              borderRadius: 5, color: '#64748b', cursor: 'pointer', fontSize: 11,
            }}
          >
            + Add resource
          </button>
          {resources.filter(u => u.trim()).length > 0 && (
            <div style={{ marginTop: 8 }}>
              {resources.filter(u => u.trim()).map((url, i) => (
                <a
                  key={i} href={url} target="_blank" rel="noopener noreferrer"
                  style={{
                    display: 'block', padding: '3px 8px', marginBottom: 3,
                    background: '#172554', color: '#60a5fa',
                    textDecoration: 'none', borderRadius: 4, fontSize: 10,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}
                >
                  🔗 {url}
                </a>
              ))}
            </div>
          )}
        </div>

        {/* Mimir Library matches */}
        <div style={{ marginBottom: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
            <label style={{ display: 'block', fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: '0.07em' }}>
              From Library
            </label>
            <button
              onClick={handleFindMatches}
              disabled={matching}
              style={{
                background: 'none', border: 'none', cursor: matching ? 'not-allowed' : 'pointer',
                fontSize: 10, color: matching ? '#475569' : '#059669', padding: 0,
              }}
            >
              {matching ? 'Matching…' : '↺ Find matches'}
            </button>
          </div>
          {mimiResources.length === 0 ? (
            <p style={{ fontSize: 10, color: '#475569', fontStyle: 'italic' }}>
              No library resources linked. Run "Find matches" or ingest content in the Library page.
            </p>
          ) : (
            mimiResources.map(r => (
              <div key={r.id} style={{ marginBottom: 4 }}>
                {r.url ? (
                  <a
                    href={r.url} target="_blank" rel="noopener noreferrer"
                    style={{
                      display: 'block', padding: '4px 8px',
                      background: '#0c1a2e', border: '1px solid #1e3a5f',
                      borderRadius: 5, color: '#60a5fa',
                      textDecoration: 'none', fontSize: 10,
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    }}
                    title={r.url}
                  >
                    🔗 {r.title}
                  </a>
                ) : (
                  <div style={{
                    padding: '4px 8px',
                    background: '#0c1a2e', border: '1px solid #1e3a5f',
                    borderRadius: 5, color: '#94a3b8', fontSize: 10,
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}>
                    📄 {r.title}
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      </div>

      {/* Footer buttons */}
      <div style={{ padding: '10px 14px', borderTop: '1px solid #1e293b', display: 'flex', flexDirection: 'column', gap: 6, flexShrink: 0 }}>
        <div style={{ display: 'flex', gap: 6 }}>
          <button
            onClick={handleSave}
            style={{
              flex: 1, padding: '7px 0', background: '#047857', color: '#ecfdf5',
              border: 'none', borderRadius: 6, cursor: 'pointer', fontSize: 12, fontWeight: 600,
            }}
          >
            Save
          </button>
          <button
            onClick={onClose}
            style={{
              flex: 1, padding: '7px 0', background: '#1e293b', color: '#94a3b8',
              border: '1px solid #334155', borderRadius: 6, cursor: 'pointer', fontSize: 12,
            }}
          >
            Cancel
          </button>
        </div>
        <div style={{ display: 'flex', gap: 6 }}>
          <button
            onClick={onAddChild}
            style={{
              flex: 1, padding: '6px 0', background: '#1e3a2e', color: '#6ee7b7',
              border: '1px solid #166534', borderRadius: 6, cursor: 'pointer', fontSize: 11,
            }}
          >
            + Add Child
          </button>
          <button
            onClick={onDelete}
            disabled={node.parent_id === null}
            style={{
              flex: 1, padding: '6px 0',
              background: node.parent_id === null ? '#1e293b' : '#450a0a',
              color: node.parent_id === null ? '#475569' : '#fca5a5',
              border: `1px solid ${node.parent_id === null ? '#334155' : '#7f1d1d'}`,
              borderRadius: 6,
              cursor: node.parent_id === null ? 'not-allowed' : 'pointer',
              fontSize: 11,
            }}
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Main component ────────────────────────────────────────────────────────────

export default function YggdrasilTree({ projectId }: YggdrasilTreeProps) {
  // DOM refs
  const svgRef = useRef<SVGSVGElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Data state
  const [treeId, setTreeId] = useState<string | null>(null);
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [edges, setEdges] = useState<TreeEdge[]>([]);
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(null);

  // Pan / zoom state
  const [isPanning, setIsPanning] = useState(false);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [scale, setScale] = useState(1);
  const panRef = useRef<{ active: boolean; startMouse: { x: number; y: number }; startPan: { x: number; y: number } }>({
    active: false, startMouse: { x: 0, y: 0 }, startPan: { x: 0, y: 0 },
  });
  const initialFitDone = useRef(false);

  // Layout computed from nodes
  const positions = useMemo(() => computeLayout(nodes), [nodes]);

  // ── Native wheel listener (React passive listeners can't preventDefault) ──
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setScale(s => Math.min(3, Math.max(0.25, s - e.deltaY * 0.001)));
    };
    svg.addEventListener('wheel', onWheel, { passive: false });
    return () => svg.removeEventListener('wheel', onWheel);
  }, []);

  // ── Load tree when project changes ───────────────────────────────────────
  useEffect(() => {
    initialFitDone.current = false;
    setNodes([]);
    setEdges([]);
    setSelectedNode(null);
    setPan({ x: 0, y: 0 });
    setScale(1);
    loadTree();
  }, [projectId]);

  // ── Fit to view after first node load ────────────────────────────────────
  useEffect(() => {
    if (nodes.length > 0 && !initialFitDone.current) {
      initialFitDone.current = true;
      // Positions are computed synchronously in useMemo, so they're ready here.
      // Small timeout to let the DOM paint first so clientWidth/Height are correct.
      setTimeout(() => fitView(), 50);
    }
  }, [nodes]);

  // ── fitView: zoom/pan so all nodes are visible and centered ──────────────
  function fitView() {
    if (positions.size === 0 || !containerRef.current) return;

    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    positions.forEach(({ x, y }) => {
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    });

    const pad = 80;
    const treeW = maxX - minX + pad * 2;
    const treeH = maxY - minY + pad * 2;
    const { clientWidth: vw, clientHeight: vh } = containerRef.current;

    const s = Math.min(1.5, Math.max(0.25, Math.min(vw / treeW, vh / treeH)));
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;

    setScale(s);
    setPan({ x: vw / 2 - cx * s, y: vh / 2 - cy * s });
  }

  // ── Data loading ──────────────────────────────────────────────────────────

  async function loadTree() {
    try {
      const trees = await invoke<any[]>('get_trees', { projectId });
      let id: string;

      if (trees.length === 0) {
        const newTree = await invoke<any>('create_tree', { projectId, name: 'My Skill Tree' });
        id = newTree.id;
        await invoke('create_tree_node', {
          treeId: id, parentId: null, type: 'trunk',
          title: 'Start Here', description: 'Root of your skill tree',
        });
      } else {
        id = trees[0].id;
      }

      setTreeId(id);
      await loadTreeContents(id);
    } catch (err) {
      console.error('Failed to load tree:', err);
    }
  }

  async function loadTreeContents(id: string) {
    try {
      const [, nodeList, edgeList] = await invoke<[any, TreeNode[], TreeEdge[]]>(
        'get_tree_with_contents', { treeId: id }
      );
      setNodes(nodeList);
      setEdges(edgeList);
    } catch (err) {
      console.error('Failed to load tree contents:', err);
    }
  }

  // ── CRUD handlers ─────────────────────────────────────────────────────────

  async function handleSave(updates: Partial<TreeNode>) {
    if (!selectedNode || !treeId) return;
    try {
      await invoke('update_tree_node', {
        nodeId: selectedNode.id,
        title: updates.title ?? null,
        description: updates.description ?? null,
        progress: updates.progress ?? null,
        tasks: updates.tasks ?? null,
        resources: updates.resources !== undefined ? (updates.resources ?? null) : null,
        position: null,
      });

      // When quest completion changes, propagate progress up the tree and to the project
      if (updates.progress !== undefined) {
        await invoke('recalculate_tree_progress', { treeId });
        await invoke('recalculate_unlocks', { treeId });
        await invoke('update_project_progress', { projectId });
        // Sync tree skills to universal skills (fire-and-forget)
        invoke('sync_skills_from_trees').then(() => invoke('recalculate_skill_levels')).catch(console.warn);
      }

      await loadTreeContents(treeId);
      setSelectedNode(prev => prev ? { ...prev, ...updates } : null);
    } catch (err) {
      console.error('Failed to save node:', err);
    }
  }

  async function handleAddChild() {
    if (!selectedNode || !treeId) return;
    const childType = selectedNode.type === 'trunk' ? 'branch' : 'leaf';
    try {
      await invoke('create_tree_node', {
        treeId, parentId: selectedNode.id, type: childType,
        title: childType === 'branch' ? 'New Skill' : 'New Quest',
        description: '',
      });
      await loadTreeContents(treeId);
    } catch (err) {
      console.error('Failed to add child:', err);
    }
  }

  async function handleDelete() {
    if (!selectedNode || !treeId || selectedNode.parent_id === null) return;
    if (!window.confirm(`Delete "${selectedNode.title}" and all its children?`)) return;
    try {
      await invoke('delete_tree_node', { nodeId: selectedNode.id });
      setSelectedNode(null);
      await loadTreeContents(treeId);
    } catch (err) {
      console.error('Failed to delete node:', err);
      alert('Delete failed: ' + err);
    }
  }

  // ── Pan handlers ──────────────────────────────────────────────────────────

  function handleBgMouseDown(e: React.MouseEvent) {
    if ((e.target as SVGElement).getAttribute('data-node')) return;
    setIsPanning(true);
    panRef.current = { active: true, startMouse: { x: e.clientX, y: e.clientY }, startPan: { ...pan } };
  }

  function handleMouseMove(e: React.MouseEvent) {
    if (!panRef.current.active) return;
    setPan({
      x: panRef.current.startPan.x + (e.clientX - panRef.current.startMouse.x),
      y: panRef.current.startPan.y + (e.clientY - panRef.current.startMouse.y),
    });
  }

  function stopPan() {
    panRef.current.active = false;
    setIsPanning(false);
  }

  // ── Node click ────────────────────────────────────────────────────────────

  function handleNodeClick(e: React.MouseEvent, node: TreeNode) {
    e.stopPropagation();
    setSelectedNode(node);
  }

  // ── Render ────────────────────────────────────────────────────────────────

  if (nodes.length === 0) {
    return (
      <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', color: '#64748b', fontSize: 13, background: '#020817' }}>
        Loading tree…
      </div>
    );
  }

  const nodeById = new Map(nodes.map(n => [n.id, n]));

  return (
    // position: absolute + inset: 0 guarantees we fill the parent exactly,
    // regardless of the parent's overflow or height chain.
    <div ref={containerRef} style={{ position: 'absolute', inset: 0, background: '#020817', overflow: 'hidden' }}>
      <svg
        ref={svgRef}
        width="100%"
        height="100%"
        style={{ display: 'block', cursor: isPanning ? 'grabbing' : 'grab' }}
        onMouseDown={handleBgMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={stopPan}
        onMouseLeave={stopPan}
      >
        {/* Transparent hit-area rect — pointer-events: all ensures it captures
            mouse events even with no fill */}
        <rect width="100%" height="100%" fill="none" style={{ pointerEvents: 'all' }} />

        <g transform={`translate(${pan.x},${pan.y}) scale(${scale})`}>

          {/* Thick trunk base lines for multi-root trees (AI-generated phases) */}
          {(() => {
            const roots = nodes.filter(n => n.parent_id === null);
            if (roots.length <= 1) return null;
            return roots.map(root => {
              const pos = positions.get(root.id);
              if (!pos) return null;
              return (
                <path
                  key={`trunk-base-${root.id}`}
                  d={branchPath(500, 800, pos.x, pos.y)}
                  fill="none"
                  stroke="#1d4027"
                  strokeWidth={7}
                  strokeLinecap="round"
                />
              );
            });
          })()}

          {/* Branch paths (parent → child) */}
          {edges.map(edge => {
            const src = positions.get(edge.source_node_id);
            const tgt = positions.get(edge.target_node_id);
            const parentNode = nodeById.get(edge.source_node_id);
            if (!src || !tgt || !parentNode) return null;
            return (
              <path
                key={edge.id}
                d={branchPath(src.x, src.y, tgt.x, tgt.y)}
                fill="none"
                stroke="#1d4027"
                strokeWidth={branchStroke(parentNode.type)}
                strokeLinecap="round"
              />
            );
          })}

          {/* Node circles */}
          {nodes.map(node => {
            const pos = positions.get(node.id);
            if (!pos) return null;
            const r = nodeRadius(node.type);
            const isSelected = selectedNode?.id === node.id;
            const parentNode = node.parent_id ? nodeById.get(node.parent_id) : undefined;
            const effectiveLocked = node.is_locked || (parentNode?.is_locked ?? false);
            return (
              <circle
                key={node.id}
                cx={pos.x}
                cy={pos.y}
                r={r}
                fill={effectiveLocked ? '#0f172a' : nodeColor(node)}
                stroke={isSelected ? '#f0fdf4' : effectiveLocked ? '#334155' : '#0f172a'}
                strokeWidth={isSelected ? 3 : 2}
                strokeDasharray={effectiveLocked && !isSelected ? '4 3' : undefined}
                opacity={effectiveLocked ? 0.55 : 1}
                style={{ cursor: 'pointer', transition: 'fill 0.25s' }}
                data-node="true"
                onClick={e => handleNodeClick(e, node)}
              />
            );
          })}

          {/* Labels */}
          {nodes.map(node => {
            const pos = positions.get(node.id);
            if (!pos) return null;
            const r = nodeRadius(node.type);
            const fontSize = node.type === 'leaf' ? 10 : 12;
            const parentNode = node.parent_id ? nodeById.get(node.parent_id) : undefined;
            const effectiveLocked = node.is_locked || (parentNode?.is_locked ?? false);
            return (
              <text
                key={`lbl-${node.id}`}
                x={pos.x}
                y={pos.y + r + fontSize + 2}
                textAnchor="middle"
                fontSize={fontSize}
                fill={effectiveLocked ? '#334155' : node.type === 'leaf' ? '#9ca3af' : '#d1d5db'}
                style={{ pointerEvents: 'none', userSelect: 'none' }}
              >
                {truncate(node.title, 16)}
              </text>
            );
          })}
        </g>
      </svg>

      {/* Zoom / fit controls */}
      <div style={{ position: 'absolute', bottom: 16, left: 16, display: 'flex', flexDirection: 'column', gap: 4, zIndex: 10 }}>
        {[
          { label: '+', action: () => setScale(s => Math.min(3, s + 0.15)) },
          { label: '−', action: () => setScale(s => Math.max(0.25, s - 0.15)) },
          { label: '⊡', action: fitView },
        ].map(({ label, action }) => (
          <button
            key={label}
            onClick={action}
            style={{
              width: 30, height: 30, background: '#1e293b', border: '1px solid #334155',
              borderRadius: 6, color: '#94a3b8', fontSize: label === '⊡' ? 14 : 18,
              cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Study / edit panel */}
      {selectedNode && (
        <NodePanel
          node={selectedNode}
          skillLocked={(() => {
            if (selectedNode.type === 'branch') return selectedNode.is_locked;
            if (selectedNode.type === 'leaf') {
              const parent = selectedNode.parent_id ? nodeById.get(selectedNode.parent_id) : undefined;
              return parent?.is_locked ?? false;
            }
            return false;
          })()}
          onSave={handleSave}
          onClose={() => setSelectedNode(null)}
          onAddChild={handleAddChild}
          onDelete={handleDelete}
        />
      )}
    </div>
  );
}
