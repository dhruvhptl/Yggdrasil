// src/pages/SkillsPage.tsx
// Universal Skill Tree: D3 force-directed galaxy visualization.
// Left panel: skill list, sync controls, gap analysis. Right panel: D3 galaxy.

import { useEffect, useRef, useState, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import * as d3 from 'd3';
import {
  RefreshCw, Loader2, Download, AlertTriangle, ChevronDown, ChevronRight,
  Sparkles, FileText, TreePine, Briefcase, X, Wand2, GitMerge, Check, Search, Eye,
} from 'lucide-react';
import type { UniversalSkill, SkillGap, SkillDependency, SkillEvidence, SkillAlias, SkillGraphSnapshot } from '../types';
import { validateOrLog, SkillSchema } from '../lib/validators';
import { z } from 'zod';

// ─── D3 types ────────────────────────────────────────────────────────────────

interface D3Node extends d3.SimulationNodeDatum {
  id: string;
  kind: 'domain' | 'skill' | 'gap';
  label: string;
  color: string;
  radius: number;
  level: number;
  evidence?: SkillEvidence[];
  demandScore?: number;
  skillCount?: number;
}

interface D3Link extends d3.SimulationLinkDatum<D3Node> {
  edgeColor: string;
  isDep?: boolean;
}

// ─── Constants ───────────────────────────────────────────────────────────────

const DOMAIN_COLORS = ['#10b981', '#6366f1', '#f59e0b', '#ec4899', '#14b8a6', '#3b82f6', '#8b5cf6', '#f97316'];
const GAP_COLOR = '#f59e0b';

const LEVEL_LABELS = ['', 'Aware', 'Familiar', 'Proficient', 'Advanced', 'Expert'];

// ─── Graph builder ───────────────────────────────────────────────────────────

function buildSkillGalaxy(
  skills: UniversalSkill[],
  gaps: SkillGap[],
  deps: SkillDependency[],
): { nodes: D3Node[]; links: D3Link[] } {
  const nodes: D3Node[] = [];
  const links: D3Link[] = [];

  // Group skills by domain
  const domainMap = new Map<string, UniversalSkill[]>();
  for (const skill of skills) {
    const domain = skill.domain || 'General';
    const arr = domainMap.get(domain) || [];
    arr.push(skill);
    domainMap.set(domain, arr);
  }

  // Create domain sun nodes
  const domainColorMap = new Map<string, string>();
  let colorIdx = 0;
  for (const [domain, domainSkills] of domainMap) {
    const color = DOMAIN_COLORS[colorIdx % DOMAIN_COLORS.length];
    domainColorMap.set(domain, color);
    colorIdx++;

    nodes.push({
      id: `domain-${domain}`,
      kind: 'domain',
      label: domain,
      color,
      radius: 24 + Math.min(domainSkills.length * 2, 14),
      level: 0,
      skillCount: domainSkills.length,
    });
  }

  // Create skill star nodes
  for (const skill of skills) {
    const domain = skill.domain || 'General';
    const color = domainColorMap.get(domain) || '#64748b';
    const radius = 8 + skill.level * 4;

    nodes.push({
      id: skill.id,
      kind: 'skill',
      label: skill.name,
      color,
      radius,
      level: skill.level,
      evidence: skill.evidence,
    });

    links.push({
      source: `domain-${domain}`,
      target: skill.id,
      edgeColor: color,
    });
  }

  // Create gap nodes (demanded but not in universal_skills or level <= 1)
  const existingNames = new Set(skills.map(s => s.name.toLowerCase()));
  for (const gap of gaps) {
    if (existingNames.has(gap.skillName.toLowerCase()) && gap.currentLevel > 0) continue;
    nodes.push({
      id: `gap-${gap.skillName}`,
      kind: 'gap',
      label: gap.skillName,
      color: GAP_COLOR,
      radius: 10,
      level: 0,
      demandScore: gap.demandScore,
    });
  }

  // Skill dependency edges
  const nodeIds = new Set(nodes.map(n => n.id));
  for (const dep of deps) {
    if (nodeIds.has(dep.sourceSkillId) && nodeIds.has(dep.targetSkillId)) {
      links.push({
        source: dep.sourceSkillId,
        target: dep.targetSkillId,
        edgeColor: '#475569',
        isDep: true,
      });
    }
  }

  return { nodes, links };
}

// ─── Stable hash for animation offsets ───────────────────────────────────────

function stableHash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) & 0xffff;
  return h;
}

// ─── Galaxy Canvas ───────────────────────────────────────────────────────────

function SkillGalaxy({
  skills,
  gaps,
  deps,
  showGaps,
  onSkillClick,
}: {
  skills: UniversalSkill[];
  gaps: SkillGap[];
  deps: SkillDependency[];
  showGaps: boolean;
  onSkillClick: (skill: UniversalSkill | null, gap: SkillGap | null) => void;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const onClickRef = useRef(onSkillClick);
  onClickRef.current = onSkillClick;

  useEffect(() => {
    const svgEl = svgRef.current!;
    const svg = d3.select<SVGSVGElement, unknown>(svgEl);
    svg.selectAll('*').remove();

    const { width, height } = svgEl.getBoundingClientRect();
    const filteredGaps = showGaps ? gaps : [];
    const { nodes, links } = buildSkillGalaxy(skills, filteredGaps, deps);
    if (nodes.length === 0) return;

    const san = (s: string) => s.replace(/[^a-zA-Z0-9]/g, '_');

    // ── Defs ──────────────────────────────────────────────────────────────────
    const defs = svg.append('defs');

    const bgGrad = defs.append('radialGradient')
      .attr('id', 'sk-bg')
      .attr('cx', '50%').attr('cy', '50%').attr('r', '62%');
    bgGrad.append('stop').attr('offset', '0%').attr('stop-color', '#0e1b2e');
    bgGrad.append('stop').attr('offset', '100%').attr('stop-color', '#020817');

    // Bloom filters
    const makeBloom = (id: string, dev: number, ext: number) => {
      const f = defs.append('filter')
        .attr('id', id)
        .attr('x', `-${ext}%`).attr('y', `-${ext}%`)
        .attr('width', `${200 + ext * 2}%`).attr('height', `${200 + ext * 2}%`);
      f.append('feGaussianBlur').attr('in', 'SourceGraphic').attr('stdDeviation', dev).attr('result', 'b');
      const m = f.append('feMerge');
      m.append('feMergeNode').attr('in', 'b');
      m.append('feMergeNode').attr('in', 'SourceGraphic');
    };
    makeBloom('bloom-skill', 5, 60);
    makeBloom('bloom-domain', 16, 120);
    makeBloom('bloom-gap', 4, 50);

    // Per-node gradients
    for (const node of nodes) {
      const id = san(node.id);

      const cg = defs.append('radialGradient')
        .attr('id', `core_${id}`)
        .attr('cx', '32%').attr('cy', '32%').attr('r', '68%');

      if (node.kind === 'domain') {
        cg.append('stop').attr('offset', '0%').attr('stop-color', '#ffffff').attr('stop-opacity', '1.0');
        cg.append('stop').attr('offset', '40%').attr('stop-color', node.color).attr('stop-opacity', '0.95');
        cg.append('stop').attr('offset', '100%').attr('stop-color', node.color).attr('stop-opacity', '0.65');
      } else if (node.kind === 'gap') {
        cg.append('stop').attr('offset', '0%').attr('stop-color', '#fef3c7').attr('stop-opacity', '0.9');
        cg.append('stop').attr('offset', '100%').attr('stop-color', GAP_COLOR).attr('stop-opacity', '0.5');
      } else {
        const levelOpacity = 0.3 + (node.level / 5) * 0.7;
        cg.append('stop').attr('offset', '0%').attr('stop-color', '#dde4ff').attr('stop-opacity', String(levelOpacity));
        cg.append('stop').attr('offset', '38%').attr('stop-color', '#2e2777').attr('stop-opacity', '0.85');
        cg.append('stop').attr('offset', '100%').attr('stop-color', node.color).attr('stop-opacity', String(levelOpacity * 0.8));
      }

      const hg = defs.append('radialGradient')
        .attr('id', `halo_${id}`)
        .attr('cx', '50%').attr('cy', '50%').attr('r', '50%');
      hg.append('stop').attr('offset', '0%').attr('stop-color', node.color).attr('stop-opacity', '1');
      hg.append('stop').attr('offset', '100%').attr('stop-color', node.color).attr('stop-opacity', '0');
    }

    // ── Background ──────────────────────────────────────────────────────────
    svg.append('rect')
      .attr('width', '100%').attr('height', '100%')
      .attr('fill', 'url(#sk-bg)')
      .attr('pointer-events', 'none');

    // ── Starfield ───────────────────────────────────────────────────────────
    const stars = svg.append('g').attr('class', 'starfield').attr('pointer-events', 'none');
    for (let i = 0; i < 200; i++) {
      const sx = Math.random() * width;
      const sy = Math.random() * height;
      const sr = i < 170 ? 0.4 + Math.random() * 0.4 : 0.8 + Math.random() * 0.6;
      const op = sr < 0.8 ? 0.04 + Math.random() * 0.10 : 0.10 + Math.random() * 0.12;
      stars.append('circle').attr('cx', sx).attr('cy', sy).attr('r', sr).attr('fill', '#fff').attr('opacity', op);
    }

    // ── Zoom / pan ──────────────────────────────────────────────────────────
    const g = svg.append('g').attr('class', 'zoom-layer');
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.04, 6])
      .on('zoom', ev => g.attr('transform', ev.transform));
    svg.call(zoom).on('dblclick.zoom', null);

    // ── Initial scatter ─────────────────────────────────────────────────────
    nodes.forEach(n => {
      n.x = width / 2 + (Math.random() - 0.5) * width * 0.55;
      n.y = height / 2 + (Math.random() - 0.5) * height * 0.55;
    });

    // ── Force simulation ────────────────────────────────────────────────────
    const sim = d3.forceSimulation<D3Node>(nodes)
      .force('link', d3.forceLink<D3Node, D3Link>(links)
        .id(d => d.id)
        .distance(d => 100 + (d.target as D3Node).radius * 4)
        .strength(d => (d as D3Link).isDep ? 0.1 : 0.35)
      )
      .force('charge', d3.forceManyBody<D3Node>()
        .strength(d => d.kind === 'domain' ? -2000 : d.kind === 'gap' ? -200 : -400)
        .distanceMin(20).distanceMax(900)
      )
      .force('center', d3.forceCenter(width / 2, height / 2).strength(0.03))
      .force('collide', d3.forceCollide<D3Node>()
        .radius(d => d.radius + 24).strength(0.8)
      )
      .alphaDecay(0.012)
      .velocityDecay(0.22);

    // ── Edges ───────────────────────────────────────────────────────────────
    const edgeSel = g.append('g').attr('class', 'edges')
      .selectAll<SVGLineElement, D3Link>('line')
      .data(links)
      .join('line')
      .attr('stroke', d => d.isDep ? d.edgeColor + '20' : d.edgeColor + '0d')
      .attr('stroke-width', d => d.isDep ? 1 : 0.5)
      .attr('stroke-dasharray', d => d.isDep ? '4 4' : 'none');

    // ── Drag ────────────────────────────────────────────────────────────────
    const drag = d3.drag<SVGGElement, D3Node>()
      .on('start', (ev, d) => { if (!ev.active) sim.alphaTarget(0.25).restart(); d.fx = d.x; d.fy = d.y; })
      .on('drag', (ev, d) => { d.fx = ev.x; d.fy = ev.y; })
      .on('end', (ev, d) => { if (!ev.active) sim.alphaTarget(0); d.fx = null; d.fy = null; });

    // ── Node groups ─────────────────────────────────────────────────────────
    const nodeSel = g.append('g').attr('class', 'nodes')
      .selectAll<SVGGElement, D3Node>('g')
      .data(nodes)
      .join('g')
      .attr('cursor', d => d.kind === 'domain' ? 'grab' : 'pointer')
      .call(drag)
      .on('click', (ev, d) => {
        ev.stopPropagation();
        if (d.kind === 'skill') {
          const skill = skills.find(s => s.id === d.id);
          if (skill) onClickRef.current(skill, null);
        } else if (d.kind === 'gap') {
          const gapName = d.id.replace('gap-', '');
          const gap = gaps.find(g => g.skillName === gapName);
          if (gap) onClickRef.current(null, gap);
        }
      });

    // Build visual layers per node
    nodeSel.each(function (d) {
      const grp = d3.select<SVGGElement, D3Node>(this);
      const id = san(d.id);
      const h = stableHash(d.id);
      const isDomain = d.kind === 'domain';
      const isGap = d.kind === 'gap';

      const intensity = isDomain ? 1.0 : isGap ? 0.7 : Math.min(0.25 + d.level * 0.15, 1.0);

      const outerR = isDomain ? d.radius * 5.5 : isGap ? d.radius * 3.0 : d.radius * (2.8 + Math.min(d.level * 0.3, 1.8));
      const midR = isDomain ? d.radius * 2.8 : isGap ? d.radius * 1.8 : d.radius * 1.85;

      const outerMax = (isDomain ? 0.22 : isGap ? 0.12 : 0.04 + d.level * 0.025) * intensity;
      const midMax = (isDomain ? 0.38 : isGap ? 0.18 : 0.10 + d.level * 0.04) * intensity;

      const outerDur = isGap ? '1.2' : (2.4 + (h % 30) * 0.09).toFixed(2);
      const midDur = isGap ? '0.8' : (1.6 + ((h >> 4) % 25) * 0.09).toFixed(2);
      const outerDelay = `-${((h % 24) * 0.13).toFixed(2)}s`;
      const midDelay = `-${(((h >> 3) % 20) * 0.17).toFixed(2)}s`;

      // Outer halo
      grp.append('circle')
        .attr('r', outerR)
        .attr('fill', `url(#halo_${id})`)
        .attr('pointer-events', 'none')
        .style('--min-op', (outerMax * 0.30).toFixed(4))
        .style('--max-op', outerMax.toFixed(4))
        .style('animation', `gx-pulse ${outerDur}s ${outerDelay} ease-in-out infinite alternate`);

      // Mid glow
      grp.append('circle')
        .attr('r', midR)
        .attr('fill', d.color)
        .attr('pointer-events', 'none')
        .style('--min-op', (midMax * 0.35).toFixed(4))
        .style('--max-op', midMax.toFixed(4))
        .style('animation', `gx-pulse ${midDur}s ${midDelay} ease-in-out infinite alternate`);

      // Core circle
      grp.append('circle')
        .attr('r', d.radius)
        .attr('fill', `url(#core_${id})`)
        .attr('stroke', d.color)
        .attr('stroke-width', isDomain ? 2.5 : isGap ? 1.5 : 1.5)
        .attr('stroke-opacity', intensity)
        .attr('stroke-dasharray', isGap ? '3 2' : 'none')
        .attr('filter', isDomain ? 'url(#bloom-domain)' : isGap ? 'url(#bloom-gap)' : 'url(#bloom-skill)')
        .attr('pointer-events', 'none');

      // Hit target
      grp.append('circle')
        .attr('r', d.radius + 6)
        .attr('fill', 'transparent');

      // Label
      grp.append('text')
        .text(d.label)
        .attr('text-anchor', 'middle')
        .attr('dy', d.radius + 14)
        .attr('fill', isDomain ? '#f8fafc' : isGap ? GAP_COLOR : `rgba(241,245,249,${0.45 + intensity * 0.55})`)
        .attr('font-size', isDomain ? 12 : 10)
        .attr('font-weight', isDomain ? '700' : isGap ? '600' : '400')
        .attr('pointer-events', 'none')
        .attr('font-family', 'system-ui, -apple-system, sans-serif')
        .attr('paint-order', 'stroke')
        .attr('stroke', '#020817')
        .attr('stroke-width', '4')
        .attr('stroke-linejoin', 'round');
    });

    // ── Tick ────────────────────────────────────────────────────────────────
    sim.on('tick', () => {
      edgeSel
        .attr('x1', d => (d.source as D3Node).x!)
        .attr('y1', d => (d.source as D3Node).y!)
        .attr('x2', d => (d.target as D3Node).x!)
        .attr('y2', d => (d.target as D3Node).y!);
      nodeSel.attr('transform', d => `translate(${d.x ?? 0},${d.y ?? 0})`);
    });

    return () => { sim.stop(); };
  }, [skills, gaps, deps, showGaps]);

  return <svg ref={svgRef} style={{ width: '100%', height: '100%', display: 'block' }} />;
}

// ─── Level dots ──────────────────────────────────────────────────────────────

function LevelDots({ level }: { level: number }) {
  return (
    <div className="flex gap-0.5">
      {[1, 2, 3, 4, 5].map(i => (
        <div
          key={i}
          className={`w-1.5 h-1.5 rounded-full ${
            i <= level ? 'bg-emerald-400' : 'bg-slate-700'
          }`}
        />
      ))}
    </div>
  );
}

// ─── Evidence icon ───────────────────────────────────────────────────────────

function EvidenceIcon({ type }: { type: string }) {
  switch (type) {
    case 'resume': return <FileText className="w-3 h-3 text-blue-400 flex-shrink-0" />;
    case 'tree_quest': return <TreePine className="w-3 h-3 text-emerald-400 flex-shrink-0" />;
    case 'work_resource': return <Briefcase className="w-3 h-3 text-violet-400 flex-shrink-0" />;
    default: return <Sparkles className="w-3 h-3 text-slate-400 flex-shrink-0" />;
  }
}

// ─── Skill Popover ───────────────────────────────────────────────────────────

function SkillPopover({
  skill,
  gap,
  onClose,
}: {
  skill: UniversalSkill | null;
  gap: SkillGap | null;
  onClose: () => void;
}) {
  if (gap && !skill) {
    return (
      <div
        onClick={e => e.stopPropagation()}
        className="absolute top-4 right-4 z-50 w-72 bg-slate-950 border border-amber-700/50 rounded-xl p-4 shadow-2xl"
      >
        <div className="flex items-start justify-between mb-3">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-4 h-4 text-amber-400" />
            <h3 className="text-sm font-semibold text-amber-300">{gap.skillName}</h3>
          </div>
          <button onClick={onClose} className="text-slate-500 hover:text-slate-300">
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
        <p className="text-xs text-slate-400 mb-2">Skill Gap — demanded by jobs but not yet in your skill tree.</p>
        <div className="space-y-1 text-xs text-slate-400">
          <div>Demanded by <span className="text-amber-300 font-medium">{gap.demandCount}</span> job(s)</div>
          <div>Frequency: <span className="text-amber-300 font-medium">{(gap.frequency * 100).toFixed(0)}%</span></div>
          <div>Demand score: <span className="text-amber-300 font-medium">{gap.demandScore.toFixed(1)}</span></div>
        </div>
      </div>
    );
  }

  if (!skill) return null;

  return (
    <div
      onClick={e => e.stopPropagation()}
      className="absolute top-4 right-4 z-50 w-80 bg-slate-950 border border-slate-700 rounded-xl p-4 shadow-2xl max-h-[80vh] overflow-auto"
    >
      <div className="flex items-start justify-between mb-3">
        <div>
          <h3 className="text-sm font-semibold text-slate-100 capitalize">{skill.name}</h3>
          {skill.domain && (
            <span className="text-[10px] text-slate-500">{skill.domain}</span>
          )}
        </div>
        <button onClick={onClose} className="text-slate-500 hover:text-slate-300">
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="flex items-center gap-2 mb-4">
        <LevelDots level={skill.level} />
        <span className="text-xs text-slate-400">
          Level {skill.level} — {LEVEL_LABELS[skill.level] || ''}
        </span>
      </div>

      {skill.evidence.length > 0 && (
        <div>
          <div className="text-[10px] font-semibold text-slate-500 uppercase tracking-wider mb-2">Evidence</div>
          <div className="space-y-2">
            {skill.evidence.map((e, i) => (
              <div key={i} className="flex items-start gap-2 text-xs text-slate-400">
                <EvidenceIcon type={e.type} />
                <div>
                  {e.type === 'resume' && <span>{e.detail || 'Listed on resume'}</span>}
                  {e.type === 'tree_quest' && (
                    <span>
                      <span className="text-emerald-400">{e.projectName}</span>
                      {' → '}{e.nodeTitle}
                      {e.progress !== undefined && (
                        <span className="text-slate-500 ml-1">({e.progress}%)</span>
                      )}
                    </span>
                  )}
                  {e.type === 'work_resource' && (
                    <span>
                      <span className="text-violet-400">{e.company}</span>
                      {' — '}{e.resourceTitle}
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ─── Merge Modal ─────────────────────────────────────────────────────────────

function MergeModal({
  skills,
  onClose,
  onMerged,
}: {
  skills: UniversalSkill[];
  onClose: () => void;
  onMerged: () => void;
}) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [canonicalId, setCanonicalId] = useState<string | null>(null);
  const [merging, setMerging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (!query.trim()) return skills;
    const q = query.toLowerCase();
    return skills.filter(s => s.name.toLowerCase().includes(q));
  }, [skills, query]);

  function toggleSkill(id: string) {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
        if (canonicalId === id) setCanonicalId(null);
      } else {
        next.add(id);
        if (!canonicalId) setCanonicalId(id);
      }
      return next;
    });
  }

  function setAsCanonical(id: string) {
    setSelected(prev => new Set([...prev, id]));
    setCanonicalId(id);
  }

  async function handleMerge() {
    if (!canonicalId || selected.size < 2) return;
    const aliasIds = [...selected].filter(id => id !== canonicalId);
    setMerging(true);
    setError(null);
    try {
      await invoke('merge_skills', { canonicalId, aliasIds });
      onMerged();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setMerging(false);
    }
  }

  const selectedSkills = skills.filter(s => selected.has(s.id));
  const canonical = skills.find(s => s.id === canonicalId);

  return (
    <div
      style={{
        position: 'fixed', inset: 0, zIndex: 100,
        background: 'rgba(0,0,0,0.7)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
      onClick={onClose}
    >
      <div
        style={{
          width: 520, maxHeight: '80vh', background: '#0f172a',
          border: '1px solid #1e293b', borderRadius: 16, display: 'flex',
          flexDirection: 'column', overflow: 'hidden',
        }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ padding: '16px 20px', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <GitMerge size={16} style={{ color: '#10b981' }} />
            <span style={{ fontWeight: 600, fontSize: 14, color: '#f1f5f9' }}>Merge Skills</span>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#64748b', cursor: 'pointer' }}>
            <X size={16} />
          </button>
        </div>

        {/* Search */}
        <div style={{ padding: '10px 20px', borderBottom: '1px solid #1e293b' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, background: '#1e293b', borderRadius: 8, padding: '6px 10px' }}>
            <Search size={13} style={{ color: '#475569', flexShrink: 0 }} />
            <input
              autoFocus
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder="Filter skills…"
              style={{ flex: 1, background: 'none', border: 'none', outline: 'none', color: '#f1f5f9', fontSize: 13 }}
            />
          </div>
        </div>

        {/* Skill list */}
        <div style={{ flex: 1, overflowY: 'auto', padding: '8px 20px' }}>
          {filtered.map(skill => {
            const isSelected = selected.has(skill.id);
            const isCanonical = skill.id === canonicalId;
            return (
              <div
                key={skill.id}
                style={{
                  display: 'flex', alignItems: 'center', gap: 10,
                  padding: '6px 8px', borderRadius: 8, marginBottom: 2,
                  background: isSelected ? 'rgba(16,185,129,0.08)' : 'transparent',
                  border: isCanonical ? '1px solid rgba(16,185,129,0.4)' : '1px solid transparent',
                  cursor: 'pointer',
                }}
                onClick={() => toggleSkill(skill.id)}
              >
                <div style={{
                  width: 16, height: 16, borderRadius: 4, flexShrink: 0,
                  background: isSelected ? '#10b981' : '#1e293b',
                  border: '1px solid ' + (isSelected ? '#10b981' : '#334155'),
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  {isSelected && <Check size={10} color="#fff" />}
                </div>
                <span style={{ flex: 1, fontSize: 13, color: isSelected ? '#d1fae5' : '#94a3b8' }}>{skill.name}</span>
                {isSelected && !isCanonical && (
                  <button
                    onClick={e => { e.stopPropagation(); setAsCanonical(skill.id); }}
                    style={{
                      fontSize: 10, padding: '2px 6px', borderRadius: 4,
                      background: 'rgba(99,102,241,0.15)', border: '1px solid rgba(99,102,241,0.3)',
                      color: '#818cf8', cursor: 'pointer',
                    }}
                  >
                    Set canonical
                  </button>
                )}
                {isCanonical && (
                  <span style={{ fontSize: 10, padding: '2px 6px', borderRadius: 4, background: 'rgba(16,185,129,0.15)', color: '#34d399' }}>
                    canonical
                  </span>
                )}
                <span style={{ fontSize: 10, color: '#475569' }}>L{skill.level}</span>
              </div>
            );
          })}
        </div>

        {/* Summary + action */}
        <div style={{ padding: '12px 20px', borderTop: '1px solid #1e293b' }}>
          {selected.size >= 2 && canonical && (
            <div style={{ fontSize: 12, color: '#64748b', marginBottom: 10 }}>
              Merge {selectedSkills.filter(s => s.id !== canonicalId).map(s => `"${s.name}"`).join(', ')} →{' '}
              <span style={{ color: '#10b981' }}>"{canonical.name}"</span>
            </div>
          )}
          {error && <div style={{ fontSize: 11, color: '#f87171', marginBottom: 8 }}>{error}</div>}
          <button
            onClick={handleMerge}
            disabled={selected.size < 2 || !canonicalId || merging}
            style={{
              width: '100%', padding: '8px', borderRadius: 8, border: 'none',
              background: (selected.size >= 2 && canonicalId && !merging) ? '#059669' : '#1e293b',
              color: (selected.size >= 2 && canonicalId && !merging) ? '#fff' : '#475569',
              fontSize: 13, fontWeight: 500, cursor: (selected.size >= 2 && canonicalId && !merging) ? 'pointer' : 'default',
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
            }}
          >
            {merging ? <><Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} /> Merging…</> : <><GitMerge size={13} /> Merge {selected.size >= 2 ? `${selected.size} skills` : 'skills'}</>}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Main Page ───────────────────────────────────────────────────────────────

export default function SkillsPage() {
  const [skills, setSkills] = useState<UniversalSkill[]>([]);
  const [gaps, setGaps] = useState<SkillGap[]>([]);
  const [deps, setDeps] = useState<SkillDependency[]>([]);
  const [aliases, setAliases] = useState<SkillAlias[]>([]);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [inferring, setInferring] = useState(false);
  const [showGaps, setShowGaps] = useState(true);
  const [showReviewOnly, setShowReviewOnly] = useState(false);
  const [showMerge, setShowMerge] = useState(false);
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(new Set());
  const [selectedSkill, setSelectedSkill] = useState<UniversalSkill | null>(null);
  const [selectedGap, setSelectedGap] = useState<SkillGap | null>(null);
  const svgContainerRef = useRef<HTMLDivElement>(null);

  // Map from canonical skill ID → alias count, for indicators in the list
  const aliasCountMap = useMemo(() => {
    const m = new Map<string, number>();
    for (const a of aliases) {
      m.set(a.canonicalSkillId, (m.get(a.canonicalSkillId) ?? 0) + 1);
    }
    return m;
  }, [aliases]);

  async function loadAll() {
    try {
      const snapshot = await invoke<SkillGraphSnapshot>('get_skill_graph_snapshot');
      const s = validateOrLog(z.array(SkillSchema), snapshot.skills, 'get_skill_graph_snapshot') as UniversalSkill[];
      setSkills(s);
      setGaps(snapshot.gaps);
      setDeps(snapshot.dependencies);
      setAliases(snapshot.aliases);

      // Auto-expand all domains
      const domains = new Set(s.map(sk => sk.domain || 'General'));
      setExpandedDomains(domains);
    } catch (e) {
      console.error('Failed to load skills:', e);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { loadAll(); }, []);

  // Reload skill graph whenever the orchestrator finishes a cascade
  useEffect(() => {
    const unlisten = listen('ygg-skills-updated', () => { loadAll(); });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  async function handleSync() {
    setSyncing(true);
    try {
      await invoke('sync_all_skills');
      await loadAll();
    } catch (e) {
      console.error('Sync failed:', e);
    } finally {
      setSyncing(false);
    }
  }

  async function handleInferDeps() {
    setInferring(true);
    try {
      await invoke('infer_skill_dependencies');
      await loadAll();
    } catch (e) {
      console.error('Infer deps failed:', e);
    } finally {
      setInferring(false);
    }
  }

  async function handleMarkReviewed(skillId: string, e: React.MouseEvent) {
    e.stopPropagation();
    try {
      await invoke('mark_skill_reviewed', { skillId });
      setSkills(prev => prev.map(s => s.id === skillId ? { ...s, reviewNeeded: false, status: 'active' } : s));
    } catch (err) {
      console.error('mark_skill_reviewed failed:', err);
    }
  }

  function handleExport() {
    const svgEl = svgContainerRef.current?.querySelector('svg');
    if (!svgEl) return;
    const serializer = new XMLSerializer();
    const svgStr = serializer.serializeToString(svgEl);
    const blob = new Blob([svgStr], { type: 'image/svg+xml' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'skill-galaxy.svg';
    a.click();
    URL.revokeObjectURL(url);
  }

  function handleSkillClick(skill: UniversalSkill | null, gap: SkillGap | null) {
    setSelectedSkill(skill);
    setSelectedGap(gap);
  }

  const avgLevel = skills.length > 0
    ? (skills.reduce((sum, s) => sum + s.level, 0) / skills.length).toFixed(1)
    : '0';

  const levelCounts = [0, 0, 0, 0, 0, 0]; // index 0 unused
  for (const s of skills) levelCounts[s.level]++;

  const reviewCount = skills.filter(s => s.reviewNeeded).length;

  // Apply review filter to domain groups
  const filteredSkills = showReviewOnly ? skills.filter(s => s.reviewNeeded) : skills;
  const filteredDomainGroups = new Map<string, UniversalSkill[]>();
  for (const skill of filteredSkills) {
    const domain = skill.domain || 'General';
    const arr = filteredDomainGroups.get(domain) || [];
    arr.push(skill);
    filteredDomainGroups.set(domain, arr);
  }

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-6 h-6 text-emerald-400 animate-spin" />
      </div>
    );
  }

  return (
    <div className="flex h-full bg-slate-950 text-white">
      {/* ── Left Panel ─────────────────────────────────────────────────────── */}
      <div className="w-80 flex-shrink-0 border-r border-slate-800 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="p-4 border-b border-slate-800">
          <div className="flex items-center justify-between mb-3">
            <h1 className="text-lg font-semibold flex items-center gap-2">
              <Sparkles className="w-5 h-5 text-emerald-400" />
              Skills
            </h1>
            <div className="flex items-center gap-1">
              <button
                onClick={handleExport}
                disabled={skills.length === 0}
                className="p-1.5 text-slate-500 hover:text-slate-300 disabled:opacity-30 transition-colors"
                title="Export galaxy as SVG"
              >
                <Download className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          {/* Sync button */}
          <button
            onClick={handleSync}
            disabled={syncing}
            className="w-full py-2 bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
          >
            {syncing ? (
              <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Syncing...</>
            ) : (
              <><RefreshCw className="w-3.5 h-3.5" /> Sync All Skills</>
            )}
          </button>

          {/* Infer dependencies button */}
          {skills.length >= 2 && (
            <button
              onClick={handleInferDeps}
              disabled={inferring}
              className="w-full mt-2 py-1.5 bg-slate-800 hover:bg-slate-700 disabled:bg-slate-800 disabled:text-slate-600 text-slate-300 text-xs font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
            >
              {inferring ? (
                <><Loader2 className="w-3 h-3 animate-spin" /> Inferring...</>
              ) : (
                <><Wand2 className="w-3 h-3" /> Infer Dependencies</>
              )}
            </button>
          )}

          {/* Merge skills button */}
          {skills.length >= 2 && (
            <button
              onClick={() => setShowMerge(true)}
              className="w-full mt-2 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
            >
              <GitMerge className="w-3 h-3" /> Merge Skills
            </button>
          )}
        </div>

        {/* Stats */}
        {skills.length > 0 && (
          <div className="px-4 py-3 border-b border-slate-800">
            <div className="flex items-center justify-between text-xs text-slate-400 mb-2">
              <span>{skills.length} skills</span>
              <span>avg level {avgLevel}</span>
            </div>
            <div className="flex gap-1">
              {[1, 2, 3, 4, 5].map(lvl => (
                <div key={lvl} className="flex-1 text-center">
                  <div className="text-[10px] text-slate-500 mb-0.5">L{lvl}</div>
                  <div className={`text-xs font-medium ${levelCounts[lvl] > 0 ? 'text-emerald-400' : 'text-slate-600'}`}>
                    {levelCounts[lvl]}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Gap toggle */}
        {gaps.length > 0 && (
          <div className="px-4 py-2 border-b border-slate-800 flex items-center justify-between">
            <span className="text-xs text-amber-400 flex items-center gap-1.5">
              <AlertTriangle className="w-3 h-3" />
              {gaps.length} skill gap{gaps.length !== 1 ? 's' : ''}
            </span>
            <button
              onClick={() => setShowGaps(!showGaps)}
              className={`text-[10px] px-2 py-0.5 rounded ${showGaps ? 'bg-amber-600/20 text-amber-300' : 'bg-slate-800 text-slate-500'}`}
            >
              {showGaps ? 'Showing' : 'Hidden'}
            </button>
          </div>
        )}

        {/* Review filter */}
        {reviewCount > 0 && (
          <div className="px-4 py-2 border-b border-slate-800 flex items-center justify-between">
            <span className="text-xs text-orange-400 flex items-center gap-1.5">
              <span className="w-2 h-2 rounded-full bg-orange-400 flex-shrink-0" />
              {reviewCount} need{reviewCount === 1 ? 's' : ''} review
            </span>
            <button
              onClick={() => setShowReviewOnly(!showReviewOnly)}
              className={`text-[10px] px-2 py-0.5 rounded ${showReviewOnly ? 'bg-orange-600/20 text-orange-300' : 'bg-slate-800 text-slate-500'}`}
            >
              {showReviewOnly ? 'Filtering' : 'Show'}
            </button>
          </div>
        )}

        {/* Skill list */}
        <div className="flex-1 overflow-auto px-4 py-3">
          {skills.length === 0 ? (
            <div className="text-center py-12">
              <Sparkles className="w-8 h-8 text-slate-700 mx-auto mb-3" />
              <p className="text-sm text-slate-500 mb-1">No skills yet</p>
              <p className="text-xs text-slate-600">Click "Sync All Skills" to pull from your resume, trees, and work page.</p>
            </div>
          ) : (
            <div className="space-y-1">
              {Array.from(filteredDomainGroups.entries()).map(([domain, domainSkills]) => {
                const expanded = expandedDomains.has(domain);
                return (
                  <div key={domain}>
                    <button
                      onClick={() => {
                        setExpandedDomains(prev => {
                          const next = new Set(prev);
                          next.has(domain) ? next.delete(domain) : next.add(domain);
                          return next;
                        });
                      }}
                      className="w-full flex items-center gap-1.5 py-1.5 text-xs text-slate-400 hover:text-slate-200 transition-colors"
                    >
                      {expanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                      <span className="font-medium capitalize">{domain}</span>
                      <span className="text-slate-600 ml-auto">{domainSkills.length}</span>
                    </button>
                    {expanded && (
                      <div className="ml-4 space-y-0.5">
                        {domainSkills
                          .sort((a, b) => b.level - a.level)
                          .map(skill => (
                            <div
                              key={skill.id}
                              className={`group w-full flex items-center gap-2 py-1 px-2 rounded text-left transition-colors cursor-pointer ${
                                selectedSkill?.id === skill.id
                                  ? 'bg-slate-800 text-slate-100'
                                  : 'text-slate-400 hover:bg-slate-800/50 hover:text-slate-200'
                              }`}
                              onClick={() => handleSkillClick(skill, null)}
                            >
                              {skill.reviewNeeded && (
                                <span
                                  title="Needs classification review"
                                  className="w-1.5 h-1.5 rounded-full bg-orange-400 flex-shrink-0"
                                />
                              )}
                              <span className="text-xs truncate flex-1 capitalize">{skill.name}</span>
                              {skill.reviewNeeded && (
                                <button
                                  onClick={e => handleMarkReviewed(skill.id, e)}
                                  title="Mark as reviewed"
                                  className="hidden group-hover:flex items-center gap-1 text-[9px] px-1.5 py-0.5 rounded bg-orange-900/30 text-orange-300 hover:bg-orange-800/50 flex-shrink-0"
                                >
                                  <Eye className="w-2.5 h-2.5" />
                                </button>
                              )}
                              {aliasCountMap.has(skill.id) && (
                                <span title={`${aliasCountMap.get(skill.id)} alias(es) merged`} style={{ fontSize: 9, padding: '1px 4px', borderRadius: 3, background: 'rgba(99,102,241,0.15)', color: '#818cf8', flexShrink: 0 }}>
                                  {aliasCountMap.get(skill.id)}↗
                                </span>
                              )}
                              <LevelDots level={skill.level} />
                            </div>
                          ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {/* Gap list */}
          {showGaps && gaps.length > 0 && (
            <div className="mt-4 pt-4 border-t border-slate-800">
              <div className="text-[10px] font-semibold text-amber-500/80 uppercase tracking-wider mb-2">
                Skill Gaps (from jobs)
              </div>
              <div className="space-y-0.5">
                {gaps.slice(0, 15).map(gap => (
                  <button
                    key={gap.skillName}
                    onClick={() => handleSkillClick(null, gap)}
                    className="w-full flex items-center gap-2 py-1 px-2 rounded text-left text-xs text-amber-400/70 hover:bg-amber-900/20 hover:text-amber-300 transition-colors"
                  >
                    <AlertTriangle className="w-2.5 h-2.5 flex-shrink-0" />
                    <span className="truncate flex-1">{gap.skillName}</span>
                    <span className="text-[10px] text-slate-600">{gap.demandCount}</span>
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* ── Right Panel: Galaxy ────────────────────────────────────────────── */}
      <div className="flex-1 relative" ref={svgContainerRef}>
        <SkillGalaxy
          skills={skills}
          gaps={gaps}
          deps={deps}
          showGaps={showGaps}
          onSkillClick={handleSkillClick}
        />

        {/* Popover */}
        {(selectedSkill || selectedGap) && (
          <SkillPopover
            skill={selectedSkill}
            gap={selectedGap}
            onClose={() => { setSelectedSkill(null); setSelectedGap(null); }}
          />
        )}
      </div>

      <style>{`
        @keyframes gx-pulse {
          from { opacity: var(--min-op, 0.02); }
          to   { opacity: var(--max-op, 0.12); }
        }
      `}</style>

      {showMerge && (
        <MergeModal
          skills={skills}
          onClose={() => setShowMerge(false)}
          onMerged={loadAll}
        />
      )}
    </div>
  );
}
