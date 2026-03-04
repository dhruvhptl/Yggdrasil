// src/pages/WorkPage.tsx
// Work graph: D3 force-directed galaxy visualization.
// Left panel: accordion CRUD. Right panel: D3 force simulation on SVG.

import React, { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import * as d3 from 'd3';
import { ChevronDown, ChevronRight, Plus, ExternalLink, X, Loader2 } from 'lucide-react';

// ─── Domain types ─────────────────────────────────────────────────────────────

interface WorkResourceSkill { id: string; resourceId: string; skillName: string; treeId?: string; }
interface ResourceWithSkills {
  id: string; topicId: string; title: string;
  url?: string; notes?: string; completed: boolean; createdAt: string;
  skills: WorkResourceSkill[];
}
interface TopicWithResources { id: string; coopId: string; name: string; createdAt: string; resources: ResourceWithSkills[]; }
interface CoopWithTopics {
  id: string; company: string; role: string;
  startDate: string; endDate: string; color: string; createdAt: string;
  topics: TopicWithResources[];
}
interface WorkGraph { coops: CoopWithTopics[]; }
interface Project { id: string; name: string; }

interface SkillNodeData {
  skillName: string;
  colors: string[];
  coopIds: string[];
  coopCompanies: string[];
  resources: { title: string; completed: boolean; coopId: string; coopColor: string }[];
  matchedProject?: Project;
}

// ─── D3 simulation node / link types ─────────────────────────────────────────

interface D3Node extends d3.SimulationNodeDatum {
  id: string;
  kind: 'coop' | 'skill';
  label: string;
  color: string;
  colors: string[];
  radius: number;
  resourceCount: number;
  skillData?: SkillNodeData;
}

interface D3Link extends d3.SimulationLinkDatum<D3Node> {
  edgeColor: string;
}

// ─── Constants ────────────────────────────────────────────────────────────────

const ACCENT_COLORS = ['#10b981', '#6366f1', '#f59e0b', '#ec4899', '#14b8a6'];

// ─── Graph builder ────────────────────────────────────────────────────────────

function buildD3Graph(graph: WorkGraph, projects: Project[]): { nodes: D3Node[]; links: D3Link[] } {
  const nodes: D3Node[] = [];
  const links: D3Link[] = [];

  const skillMap = new Map<string, {
    coopIds: string[];
    coopCompanies: string[];
    colors: string[];
    resources: { title: string; completed: boolean; coopId: string; coopColor: string }[];
  }>();

  for (const coop of graph.coops) {
    nodes.push({
      id: coop.id,
      kind: 'coop',
      label: coop.company,
      color: coop.color,
      colors: [coop.color],
      radius: 32,
      resourceCount: 0,
    });

    for (const topic of coop.topics) {
      for (const resource of topic.resources) {
        for (const skill of resource.skills) {
          const entry = skillMap.get(skill.skillName) ?? {
            coopIds: [], coopCompanies: [], colors: [], resources: [],
          };
          if (!entry.coopIds.includes(coop.id)) {
            entry.coopIds.push(coop.id);
            entry.coopCompanies.push(coop.company);
            entry.colors.push(coop.color);
          }
          entry.resources.push({
            title: resource.title, completed: resource.completed,
            coopId: coop.id, coopColor: coop.color,
          });
          skillMap.set(skill.skillName, entry);
        }
      }
    }
  }

  for (const [skillName, info] of skillMap.entries()) {
    const nodeId = `skill-${skillName}`;
    const matchedProject = projects.find(p =>
      p.name.toLowerCase().includes(skillName.toLowerCase()) ||
      skillName.toLowerCase().includes(p.name.toLowerCase())
    );
    // Scale radius with resource backing: base 9, +2 per resource, max 22
    const resourceCount = info.resources.length;
    const radius = Math.min(9 + resourceCount * 2, 22);

    nodes.push({
      id: nodeId,
      kind: 'skill',
      label: skillName + (matchedProject ? ' 🌲' : ''),
      color: info.colors[0],
      colors: info.colors,
      radius,
      resourceCount,
      skillData: {
        skillName,
        colors: info.colors,
        coopIds: info.coopIds,
        coopCompanies: info.coopCompanies,
        resources: info.resources,
        matchedProject,
      },
    });

    for (let i = 0; i < info.coopIds.length; i++) {
      links.push({ source: info.coopIds[i], target: nodeId, edgeColor: info.colors[i] });
    }
  }

  return { nodes, links };
}

// ─── Galaxy Canvas ────────────────────────────────────────────────────────────

// Deterministic hash so animation params are stable across re-renders
function stableHash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) & 0xffff;
  return h;
}

function GalaxyCanvas({
  graph,
  projects,
  onSkillClick,
}: {
  graph: WorkGraph;
  projects: Project[];
  onSkillClick: (data: SkillNodeData) => void;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const onSkillClickRef = useRef(onSkillClick);
  onSkillClickRef.current = onSkillClick;

  useEffect(() => {
    const svgEl = svgRef.current!;
    const svg = d3.select<SVGSVGElement, unknown>(svgEl);
    svg.selectAll('*').remove();

    const { width, height } = svgEl.getBoundingClientRect();
    const { nodes, links } = buildD3Graph(graph, projects);
    if (nodes.length === 0) return;

    const san = (s: string) => s.replace(/[^a-zA-Z0-9]/g, '_');

    // ── Defs ──────────────────────────────────────────────────────────────────
    const defs = svg.append('defs');

    // Subtle galaxy-center background gradient (fixed, outside zoom layer)
    const bgGrad = defs.append('radialGradient')
      .attr('id', 'gx-bg')
      .attr('cx', '50%').attr('cy', '50%').attr('r', '62%');
    bgGrad.append('stop').attr('offset', '0%').attr('stop-color', '#0e1b2e');
    bgGrad.append('stop').attr('offset', '100%').attr('stop-color', '#020817');

    // Bloom filters — skill (moderate) and coop (strong sun-like)
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
    makeBloom('bloom-coop', 16, 120);

    // Per-node: core radial gradient + halo feathered radial gradient
    for (const node of nodes) {
      const id = san(node.id);
      const isCoop = node.kind === 'coop';

      // Core: bright center → accent at outer edge (inner star/sun)
      const cg = defs.append('radialGradient')
        .attr('id', `core_${id}`)
        .attr('cx', '32%').attr('cy', '32%').attr('r', '68%');
      if (isCoop) {
        cg.append('stop').attr('offset', '0%').attr('stop-color', '#ffffff').attr('stop-opacity', '1.0');
        cg.append('stop').attr('offset', '40%').attr('stop-color', node.color).attr('stop-opacity', '0.95');
        cg.append('stop').attr('offset', '100%').attr('stop-color', node.color).attr('stop-opacity', '0.65');
      } else {
        // Deep indigo core → accent at rim; multi-coop blends both colors
        cg.append('stop').attr('offset', '0%').attr('stop-color', '#dde4ff').attr('stop-opacity', '0.80');
        cg.append('stop').attr('offset', '38%').attr('stop-color', '#2e2777').attr('stop-opacity', '0.85');
        if (node.colors.length > 1) {
          cg.append('stop').attr('offset', '72%').attr('stop-color', node.colors[0]).attr('stop-opacity', '0.70');
          cg.append('stop').attr('offset', '100%').attr('stop-color', node.colors[node.colors.length - 1]).attr('stop-opacity', '0.55');
        } else {
          cg.append('stop').attr('offset', '100%').attr('stop-color', node.colors[0]).attr('stop-opacity', '0.65');
        }
      }

      // Halo: accent color fading to transparent (used by glow circles)
      const hg = defs.append('radialGradient')
        .attr('id', `halo_${id}`)
        .attr('cx', '50%').attr('cy', '50%').attr('r', '50%');
      hg.append('stop').attr('offset', '0%').attr('stop-color', node.color).attr('stop-opacity', '1');
      hg.append('stop').attr('offset', '100%').attr('stop-color', node.color).attr('stop-opacity', '0');
    }

    // ── Fixed background rect ─────────────────────────────────────────────────
    svg.append('rect')
      .attr('width', '100%').attr('height', '100%')
      .attr('fill', 'url(#gx-bg)')
      .attr('pointer-events', 'none');

    // ── Static starfield — tiny white dots, fixed to viewport ─────────────────
    const stars = svg.append('g').attr('class', 'starfield').attr('pointer-events', 'none');
    const starCount = 200;
    for (let i = 0; i < starCount; i++) {
      const sx = Math.random() * width;
      const sy = Math.random() * height;
      // Vary size: most are tiny (0.4–0.8), a few slightly larger (up to 1.4)
      const sr = i < 170 ? 0.4 + Math.random() * 0.4 : 0.8 + Math.random() * 0.6;
      // Vary opacity: 4–18%, brighter for the larger ones
      const op = sr < 0.8 ? 0.04 + Math.random() * 0.10 : 0.10 + Math.random() * 0.12;
      stars.append('circle')
        .attr('cx', sx).attr('cy', sy)
        .attr('r', sr)
        .attr('fill', '#ffffff')
        .attr('opacity', op);
    }

    // ── Zoom / pan ────────────────────────────────────────────────────────────
    const g = svg.append('g').attr('class', 'zoom-layer');
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.04, 6])
      .on('zoom', ev => g.attr('transform', ev.transform));
    svg.call(zoom).on('dblclick.zoom', null);

    // ── Initial scatter ───────────────────────────────────────────────────────
    nodes.forEach(n => {
      n.x = width / 2 + (Math.random() - 0.5) * width * 0.55;
      n.y = height / 2 + (Math.random() - 0.5) * height * 0.55;
    });

    // ── Force simulation ──────────────────────────────────────────────────────
    const sim = d3.forceSimulation<D3Node>(nodes)
      .force('link', d3.forceLink<D3Node, D3Link>(links)
        .id(d => d.id)
        .distance(d => 130 + (d.target as D3Node).radius * 5)
        .strength(0.38)
      )
      .force('charge', d3.forceManyBody<D3Node>()
        .strength(d => d.kind === 'coop' ? -1700 : -380)
        .distanceMin(20).distanceMax(900)
      )
      .force('center', d3.forceCenter(width / 2, height / 2).strength(0.03))
      .force('collide', d3.forceCollide<D3Node>()
        .radius(d => d.radius + 28).strength(0.8)
      )
      .alphaDecay(0.010)
      .velocityDecay(0.22);

    // ── Edges — barely-there light trails ────────────────────────────────────
    const edgeSel = g.append('g').attr('class', 'edges')
      .selectAll<SVGLineElement, D3Link>('line')
      .data(links)
      .join('line')
      .attr('stroke', d => d.edgeColor + '0d')   // ~5% opacity
      .attr('stroke-width', 0.5);

    // ── Drag ─────────────────────────────────────────────────────────────────
    const drag = d3.drag<SVGGElement, D3Node>()
      .on('start', (ev, d) => { if (!ev.active) sim.alphaTarget(0.25).restart(); d.fx = d.x; d.fy = d.y; })
      .on('drag',  (ev, d) => { d.fx = ev.x; d.fy = ev.y; })
      .on('end',   (ev, d) => { if (!ev.active) sim.alphaTarget(0); d.fx = null; d.fy = null; });

    // ── Node groups ───────────────────────────────────────────────────────────
    const nodeSel = g.append('g').attr('class', 'nodes')
      .selectAll<SVGGElement, D3Node>('g')
      .data(nodes)
      .join('g')
      .attr('cursor', d => d.kind === 'skill' ? 'pointer' : 'grab')
      .call(drag)
      .on('click', (ev, d) => {
        if (d.kind === 'skill' && d.skillData) {
          ev.stopPropagation();
          onSkillClickRef.current(d.skillData);
        }
      });

    // Build layers inside each node group
    nodeSel.each(function(d) {
      const grp  = d3.select<SVGGElement, D3Node>(this);
      const id   = san(d.id);
      const h    = stableHash(d.id);
      const coop = d.kind === 'coop';

      // Intensity: co-op is always 1.0; skills scale with resource count (dimmer when fewer)
      const intensity = coop ? 1.0 : Math.min(0.28 + d.resourceCount * 0.08, 1.0);

      // Outer halo size grows with resources for skills
      const outerR = coop
        ? d.radius * 5.5
        : d.radius * (2.8 + Math.min(d.resourceCount * 0.22, 1.8));
      const midR = coop ? d.radius * 2.8 : d.radius * 1.85;

      // Max opacity for each glow ring (intensity-scaled)
      const outerMax = (coop ? 0.22 : 0.04 + d.resourceCount * 0.018) * intensity;
      const midMax   = (coop ? 0.38 : 0.10 + d.resourceCount * 0.030) * intensity;

      // Pulse durations / offsets — deterministic, vary per node so they don't sync
      const outerDur   = (2.4 + (h % 30) * 0.09).toFixed(2);
      const midDur     = (1.6 + ((h >> 4) % 25) * 0.09).toFixed(2);
      const outerDelay = `-${((h % 24) * 0.13).toFixed(2)}s`;
      const midDelay   = `-${(((h >> 3) % 20) * 0.17).toFixed(2)}s`;

      // ① Outer halo — feathered radial gradient, slow pulse
      grp.append('circle')
        .attr('r', outerR)
        .attr('fill', `url(#halo_${id})`)
        .attr('pointer-events', 'none')
        .style('--min-op', (outerMax * 0.30).toFixed(4))
        .style('--max-op', outerMax.toFixed(4))
        .style('animation', `gx-pulse ${outerDur}s ${outerDelay} ease-in-out infinite alternate`);

      // ② Mid glow — solid accent color, faster pulse
      grp.append('circle')
        .attr('r', midR)
        .attr('fill', d.color)
        .attr('pointer-events', 'none')
        .style('--min-op', (midMax * 0.35).toFixed(4))
        .style('--max-op', midMax.toFixed(4))
        .style('animation', `gx-pulse ${midDur}s ${midDelay} ease-in-out infinite alternate`);

      // ③ Core circle — radial gradient fill + bloom filter
      grp.append('circle')
        .attr('r', d.radius)
        .attr('fill', `url(#core_${id})`)
        .attr('stroke', d.color)
        .attr('stroke-width', coop ? 2.5 : 1.5)
        .attr('stroke-opacity', intensity)
        .attr('filter', coop ? 'url(#bloom-coop)' : 'url(#bloom-skill)')
        .attr('pointer-events', 'none');

      // ④ Transparent hit target (slightly larger than core for easier clicking)
      grp.append('circle')
        .attr('r', d.radius + 6)
        .attr('fill', 'transparent');

      // ⑤ Label
      grp.append('text')
        .text(d.label)
        .attr('text-anchor', 'middle')
        .attr('dy', d.radius + 14)
        .attr('fill', coop ? '#f8fafc' : `rgba(241,245,249,${0.55 + intensity * 0.45})`)
        .attr('font-size', coop ? 12 : 10)
        .attr('font-weight', coop ? '700' : '400')
        .attr('pointer-events', 'none')
        .attr('font-family', 'system-ui, -apple-system, sans-serif')
        .attr('paint-order', 'stroke')
        .attr('stroke', '#020817')
        .attr('stroke-width', '4')
        .attr('stroke-linejoin', 'round');
    });

    // ── Tick ──────────────────────────────────────────────────────────────────
    sim.on('tick', () => {
      edgeSel
        .attr('x1', d => (d.source as D3Node).x!)
        .attr('y1', d => (d.source as D3Node).y!)
        .attr('x2', d => (d.target as D3Node).x!)
        .attr('y2', d => (d.target as D3Node).y!);
      nodeSel.attr('transform', d => `translate(${d.x ?? 0},${d.y ?? 0})`);
    });

    return () => { sim.stop(); };
  }, [graph, projects]);

  return <svg ref={svgRef} style={{ width: '100%', height: '100%', display: 'block' }} />;
}

// ─── Skill Popover ────────────────────────────────────────────────────────────

function SkillPopover({
  data,
  onClose,
}: {
  data: SkillNodeData;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  return (
    <div
      onClick={e => e.stopPropagation()}
      style={{
        position: 'absolute', top: 16, right: 16, zIndex: 200,
        width: 280, background: '#0f172a', border: '1px solid #1e293b',
        borderRadius: 10, boxShadow: '0 8px 32px rgba(0,0,0,0.6)',
        padding: 16,
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 12 }}>
        <div>
          <div style={{ fontWeight: 700, fontSize: 14, color: '#f1f5f9' }}>{data.skillName}</div>
          <div style={{ display: 'flex', gap: 4, marginTop: 4, flexWrap: 'wrap' }}>
            {data.colors.map((c, i) => (
              <span key={i} style={{
                fontSize: 9, padding: '1px 6px', borderRadius: 10,
                background: c + '33', border: `1px solid ${c}`, color: c,
              }}>
                {data.coopCompanies[i] ?? data.coopIds[i]?.slice(0, 8)}
              </span>
            ))}
          </div>
        </div>
        <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#64748b', cursor: 'pointer', fontSize: 18, lineHeight: 1, padding: 0 }}>
          <X className="w-4 h-4" />
        </button>
      </div>

      <div style={{ fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: '0.07em', marginBottom: 6 }}>
        Backed by
      </div>
      <div style={{ maxHeight: 180, overflowY: 'auto' }}>
        {data.resources.map((r, i) => (
          <div key={i} style={{
            display: 'flex', alignItems: 'center', gap: 6, padding: '4px 0',
            borderBottom: i < data.resources.length - 1 ? '1px solid #1e293b' : 'none',
          }}>
            <span style={{ fontSize: 13, color: r.completed ? '#10b981' : '#475569', flexShrink: 0 }}>
              {r.completed ? '✓' : '○'}
            </span>
            <span style={{ fontSize: 11, color: r.completed ? '#94a3b8' : '#f1f5f9', flex: 1 }}>
              {r.title}
            </span>
            <span style={{ width: 8, height: 8, borderRadius: '50%', background: r.coopColor, flexShrink: 0 }} />
          </div>
        ))}
        {data.resources.length === 0 && (
          <p style={{ fontSize: 11, color: '#475569', fontStyle: 'italic' }}>No resources yet</p>
        )}
      </div>

      {data.matchedProject && (
        <button
          onClick={() => navigate(`/project/${data.matchedProject!.id}`)}
          style={{
            marginTop: 12, width: '100%', padding: '6px 0',
            background: '#065f46', border: '1px solid #059669',
            borderRadius: 6, color: '#6ee7b7', fontSize: 11,
            cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 4,
          }}
        >
          🌲 View in {data.matchedProject.name}
        </button>
      )}
    </div>
  );
}

// ─── Left Panel ───────────────────────────────────────────────────────────────

interface CoopFormState { company: string; role: string; startDate: string; endDate: string; color: string; }
type SourceType = 'url' | 'pdf';
interface ResourceFormState { title: string; url: string; notes: string; completed: boolean; sourceType: SourceType; }

function LeftPanel({
  graph,
  onRefresh,
}: {
  graph: WorkGraph;
  onRefresh: () => Promise<void>;
}) {
  const [showCoopForm, setShowCoopForm] = useState(false);
  const [coopForm, setCoopForm] = useState<CoopFormState>({ company: '', role: '', startDate: '', endDate: '', color: '#10b981' });
  const [savingCoop, setSavingCoop] = useState(false);

  const [addingTopicFor, setAddingTopicFor] = useState<string | null>(null);
  const [topicName, setTopicName] = useState('');
  const [savingTopic, setSavingTopic] = useState(false);

  const [addingResourceFor, setAddingResourceFor] = useState<string | null>(null);
  const [resourceForm, setResourceForm] = useState<ResourceFormState>({ title: '', url: '', notes: '', completed: false, sourceType: 'url' as SourceType});
  const [savingResource, setSavingResource] = useState(false);
  const [pdfFile, setPdfFile] = useState<File | null>(null);
  const [uploadingPdf, setUploadingPdf] = useState(false);
  const pdfInputRef = useRef<HTMLInputElement>(null);
  const [extractingFor, setExtractingFor] = useState<Set<string>>(new Set());
  const [togglingFor, setTogglingFor] = useState<Set<string>>(new Set());

  const [expandedCoops, setExpandedCoops] = useState<Set<string>>(new Set());
  const [expandedTopics, setExpandedTopics] = useState<Set<string>>(new Set());

  function toggleCoop(id: string) {
    setExpandedCoops(prev => { const s = new Set(prev); s.has(id) ? s.delete(id) : s.add(id); return s; });
  }
  function toggleTopic(id: string) {
    setExpandedTopics(prev => { const s = new Set(prev); s.has(id) ? s.delete(id) : s.add(id); return s; });
  }

  async function handleAddCoop() {
    if (!coopForm.company.trim() || !coopForm.role.trim()) return;
    setSavingCoop(true);
    try {
      await invoke('create_coop', {
        company: coopForm.company, role: coopForm.role,
        startDate: coopForm.startDate, endDate: coopForm.endDate, color: coopForm.color,
      });
      setCoopForm({ company: '', role: '', startDate: '', endDate: '', color: '#10b981' });
      setShowCoopForm(false);
      await onRefresh();
    } finally { setSavingCoop(false); }
  }

  async function handleAddTopic(coopId: string) {
    if (!topicName.trim()) return;
    setSavingTopic(true);
    try {
      await invoke('create_topic', { coopId, name: topicName });
      setTopicName('');
      setAddingTopicFor(null);
      setExpandedCoops(prev => new Set([...prev, coopId]));
      await onRefresh();
    } finally { setSavingTopic(false); }
  }

  function handlePdfSelect(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    setPdfFile(file);
    setResourceForm(p => ({ ...p, title: file.name, url: '', sourceType: 'pdf' as SourceType }));
  }

  function resetPdfState() {
    setPdfFile(null);
    if (pdfInputRef.current) pdfInputRef.current.value = '';
  }

  async function handleAddResource(topicId: string, _coopId: string) {
    if (!resourceForm.title.trim()) return;
    setSavingResource(true);
    try {
      let notesForResource = resourceForm.notes || null;

      // If PDF selected, upload to Mimir sidecar via Tauri command
      if (pdfFile) {
        setUploadingPdf(true);
        try {
          const base64 = await new Promise<string>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => {
              const result = reader.result as string;
              resolve(result.split(',')[1]);
            };
            reader.onerror = reject;
            reader.readAsDataURL(pdfFile);
          });

          const pdfResult = await invoke<{ id: string; title: string; textPreview?: string }>(
            'ingest_mimir_pdf',
            { filename: pdfFile.name, pdfBase64: base64 },
          );
          if (pdfResult.textPreview) {
            notesForResource = pdfResult.textPreview;
          }
        } finally {
          setUploadingPdf(false);
        }
      }

      const resource = await invoke<{ id: string }>('add_resource', {
        topicId,
        title: resourceForm.title,
        url: resourceForm.url || null,
        notes: notesForResource,
        completed: resourceForm.completed,
      });
      setResourceForm({ title: '', url: '', notes: '', completed: false, sourceType: 'url' as SourceType});
      resetPdfState();
      setAddingResourceFor(null);
      await onRefresh();

      // Auto-extract skills
      setExtractingFor(prev => new Set([...prev, resource.id]));
      try {
        await invoke('extract_skills', { resourceId: resource.id });
        await onRefresh();
        // Sync work skills to universal skills (fire-and-forget)
        invoke('sync_skills_from_work').then(() => invoke('recalculate_skill_levels')).catch(console.warn);
      } catch (e) {
        console.warn('Skill extraction failed:', e);
      } finally {
        setExtractingFor(prev => { const s = new Set(prev); s.delete(resource.id); return s; });
      }
    } finally { setSavingResource(false); }
  }

  async function handleToggleCompleted(resourceId: string) {
    setTogglingFor(prev => new Set([...prev, resourceId]));
    try {
      await invoke('toggle_resource_completed', { resourceId });
      await onRefresh();
    } catch (e) {
      console.warn('Toggle failed:', e);
    } finally {
      setTogglingFor(prev => { const s = new Set(prev); s.delete(resourceId); return s; });
    }
  }

  const inputStyle: React.CSSProperties = {
    width: '100%', padding: '5px 8px',
    background: '#1e293b', border: '1px solid #334155',
    borderRadius: 5, color: '#f1f5f9', fontSize: 11,
    boxSizing: 'border-box',
  };
  const btnStyle = (color = '#047857'): React.CSSProperties => ({
    padding: '4px 10px', background: color, border: 'none',
    borderRadius: 5, color: '#f1f5f9', fontSize: 11, cursor: 'pointer',
  });

  return (
    <div style={{ width: 320, flexShrink: 0, borderRight: '1px solid #1e293b', display: 'flex', flexDirection: 'column', background: '#0a0f1a', height: '100%' }}>
      {/* Header */}
      <div style={{ padding: '12px 14px', borderBottom: '1px solid #1e293b', display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0 }}>
        <span style={{ fontWeight: 600, fontSize: 13, color: '#f1f5f9' }}>Co-op Work</span>
        <button
          onClick={() => setShowCoopForm(v => !v)}
          style={{ ...btnStyle(), display: 'flex', alignItems: 'center', gap: 4, fontSize: 11 }}
        >
          <Plus className="w-3 h-3" /> Add Co-op
        </button>
      </div>

      {/* Add co-op form */}
      {showCoopForm && (
        <div style={{ padding: '10px 14px', borderBottom: '1px solid #1e293b', background: '#0f172a', flexShrink: 0 }}>
          <input placeholder="Company" value={coopForm.company} onChange={e => setCoopForm(p => ({ ...p, company: e.target.value }))} style={{ ...inputStyle, marginBottom: 5 }} />
          <input placeholder="Role" value={coopForm.role} onChange={e => setCoopForm(p => ({ ...p, role: e.target.value }))} style={{ ...inputStyle, marginBottom: 5 }} />
          <div style={{ display: 'flex', gap: 5, marginBottom: 5 }}>
            <input type="date" value={coopForm.startDate} onChange={e => setCoopForm(p => ({ ...p, startDate: e.target.value }))} style={{ ...inputStyle, flex: 1 }} />
            <input type="date" value={coopForm.endDate} onChange={e => setCoopForm(p => ({ ...p, endDate: e.target.value }))} style={{ ...inputStyle, flex: 1 }} />
          </div>
          <div style={{ display: 'flex', gap: 6, marginBottom: 8 }}>
            {ACCENT_COLORS.map(c => (
              <button key={c} onClick={() => setCoopForm(p => ({ ...p, color: c }))}
                style={{ width: 20, height: 20, borderRadius: '50%', background: c, border: coopForm.color === c ? '2px solid white' : '2px solid transparent', cursor: 'pointer', padding: 0 }}
              />
            ))}
          </div>
          <div style={{ display: 'flex', gap: 5 }}>
            <button onClick={handleAddCoop} disabled={savingCoop} style={btnStyle()}>
              {savingCoop ? 'Saving…' : 'Save'}
            </button>
            <button onClick={() => setShowCoopForm(false)} style={btnStyle('#1e293b')}>Cancel</button>
          </div>
        </div>
      )}

      {/* Co-op list */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '6px 0' }}>
        {graph.coops.length === 0 && (
          <div style={{ padding: '24px 16px', textAlign: 'center', color: '#475569', fontSize: 12 }}>
            No co-ops yet. Add your first one above.
          </div>
        )}

        {graph.coops.map(coop => (
          <div key={coop.id} style={{ borderLeft: `3px solid ${coop.color}`, marginBottom: 2 }}>
            <button
              onClick={() => toggleCoop(coop.id)}
              style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 6, padding: '8px 12px', background: 'none', border: 'none', cursor: 'pointer', textAlign: 'left' }}
            >
              {expandedCoops.has(coop.id)
                ? <ChevronDown className="w-3 h-3" style={{ color: '#475569', flexShrink: 0 }} />
                : <ChevronRight className="w-3 h-3" style={{ color: '#475569', flexShrink: 0 }} />}
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: '#f1f5f9', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{coop.company}</div>
                <div style={{ fontSize: 10, color: '#64748b' }}>{coop.role} · {coop.startDate} – {coop.endDate}</div>
              </div>
            </button>

            {expandedCoops.has(coop.id) && (
              <div style={{ paddingLeft: 18 }}>
                {coop.topics.map(topic => (
                  <div key={topic.id} style={{ marginBottom: 2 }}>
                    <button
                      onClick={() => toggleTopic(topic.id)}
                      style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 5, padding: '5px 8px', background: 'none', border: 'none', cursor: 'pointer', textAlign: 'left' }}
                    >
                      {expandedTopics.has(topic.id)
                        ? <ChevronDown className="w-3 h-3" style={{ color: '#475569' }} />
                        : <ChevronRight className="w-3 h-3" style={{ color: '#475569' }} />}
                      <span style={{ fontSize: 11, color: '#cbd5e1' }}>{topic.name}</span>
                    </button>

                    {expandedTopics.has(topic.id) && (
                      <div style={{ paddingLeft: 16 }}>
                        {topic.resources.map(res => (
                          <div key={res.id} style={{ padding: '4px 0 6px 0', borderBottom: '1px solid #1e293b11' }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                              <button
                                onClick={() => handleToggleCompleted(res.id)}
                                disabled={togglingFor.has(res.id)}
                                title={res.completed ? 'Mark incomplete' : 'Mark complete'}
                                style={{
                                  fontSize: 13, lineHeight: 1, padding: '1px 2px',
                                  background: 'none', border: 'none', cursor: 'pointer',
                                  color: res.completed ? '#10b981' : '#475569',
                                  opacity: togglingFor.has(res.id) ? 0.4 : 1,
                                  flexShrink: 0,
                                }}
                              >
                                {res.completed ? '✓' : '○'}
                              </button>
                              <span style={{ fontSize: 11, color: res.completed ? '#475569' : '#cbd5e1', textDecoration: res.completed ? 'line-through' : 'none', flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{res.title}</span>
                              {res.url ? (
                                <a href={res.url} target="_blank" rel="noopener noreferrer" style={{ color: '#475569', flexShrink: 0 }}>
                                  <ExternalLink className="w-3 h-3" />
                                </a>
                              ) : (
                                <span style={{ color: '#475569', flexShrink: 0, fontSize: 11 }}>📄</span>
                              )}
                              {extractingFor.has(res.id) && <Loader2 className="w-3 h-3" style={{ color: '#475569', animation: 'spin 1s linear infinite', flexShrink: 0 }} />}
                            </div>
                            {res.skills.length > 0 && (
                              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 3, marginTop: 3 }}>
                                {res.skills.map(s => (
                                  <span key={s.id} style={{
                                    fontSize: 9, padding: '1px 6px', borderRadius: 8,
                                    background: coop.color + '22', border: `1px solid ${coop.color}55`, color: coop.color,
                                  }}>
                                    {s.skillName}
                                  </span>
                                ))}
                              </div>
                            )}
                          </div>
                        ))}

                        {addingResourceFor === topic.id ? (
                          <div style={{ padding: '6px 0' }}>
                            {/* Source type toggle */}
                            <div style={{ display: 'flex', marginBottom: 5, borderRadius: 5, overflow: 'hidden', border: '1px solid #334155' }}>
                              {([['url', 'URL'], ['pdf', 'PDF']] as const).map(([key, label]) => (
                                <button
                                  key={key}
                                  onClick={() => {
                                    setResourceForm(p => ({ ...p, sourceType: key, url: '', title: key === 'url' ? p.title : p.title }));
                                    if (key === 'url') resetPdfState();
                                  }}
                                  style={{
                                    flex: 1, padding: '4px 0', fontSize: 10, fontWeight: 600,
                                    background: resourceForm.sourceType === key ? '#1e293b' : 'transparent',
                                    color: resourceForm.sourceType === key ? '#f1f5f9' : '#64748b',
                                    border: 'none', cursor: 'pointer',
                                  }}
                                >
                                  {key === 'pdf' ? '📄 ' : '🔗 '}{label}
                                </button>
                              ))}
                            </div>

                            <input placeholder="Title *" value={resourceForm.title} onChange={e => setResourceForm(p => ({ ...p, title: e.target.value }))} style={{ ...inputStyle, marginBottom: 4 }} />

                            {/* URL mode */}
                            {resourceForm.sourceType === 'url' && (
                              <input placeholder="URL (optional)" value={resourceForm.url} onChange={e => setResourceForm(p => ({ ...p, url: e.target.value }))} style={{ ...inputStyle, marginBottom: 4 }} />
                            )}

                            {/* PDF mode */}
                            {resourceForm.sourceType === 'pdf' && (
                              <>
                                <input ref={pdfInputRef} type="file" accept=".pdf" onChange={handlePdfSelect} style={{ display: 'none' }} />
                                {pdfFile ? (
                                  <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 10, color: '#94a3b8', padding: '5px 8px', background: '#1e293b', borderRadius: 5, marginBottom: 4 }}>
                                    <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>📄 {pdfFile.name}</span>
                                    <button onClick={() => { resetPdfState(); setResourceForm(p => ({ ...p, title: '' })); }} style={{ background: 'none', border: 'none', color: '#ef4444', cursor: 'pointer', padding: 0, fontSize: 11, flexShrink: 0 }}>✕</button>
                                  </div>
                                ) : (
                                  <button type="button" onClick={() => pdfInputRef.current?.click()} style={{ ...inputStyle, textAlign: 'center', cursor: 'pointer', color: '#64748b', marginBottom: 4, borderStyle: 'dashed' }}>
                                    Choose PDF file…
                                  </button>
                                )}
                              </>
                            )}

                            <textarea placeholder="Notes (optional)" value={resourceForm.notes} onChange={e => setResourceForm(p => ({ ...p, notes: e.target.value }))} rows={2} style={{ ...inputStyle, resize: 'vertical', marginBottom: 4, fontFamily: 'inherit' }} />
                            <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                              <button onClick={() => handleAddResource(topic.id, coop.id)} disabled={savingResource || uploadingPdf} style={btnStyle()}>
                                {uploadingPdf ? 'Uploading…' : savingResource ? 'Saving…' : 'Add'}
                              </button>
                              <button onClick={() => { setAddingResourceFor(null); resetPdfState(); }} style={btnStyle('#1e293b')}>Cancel</button>
                              <label style={{ display: 'flex', alignItems: 'center', gap: 3, fontSize: 10, color: '#64748b', cursor: 'pointer', marginLeft: 'auto' }}>
                                <input type="checkbox" checked={resourceForm.completed} onChange={e => setResourceForm(p => ({ ...p, completed: e.target.checked }))} style={{ width: 12, height: 12 }} />
                                Done
                              </label>
                            </div>
                          </div>
                        ) : (
                          <button
                            onClick={() => { setAddingResourceFor(topic.id); setResourceForm({ title: '', url: '', notes: '', completed: false, sourceType: 'url' as SourceType}); resetPdfState(); }}
                            style={{ fontSize: 10, color: '#475569', background: 'none', border: 'none', cursor: 'pointer', padding: '3px 0', display: 'flex', alignItems: 'center', gap: 3 }}
                          >
                            <Plus className="w-3 h-3" /> Add Resource
                          </button>
                        )}
                      </div>
                    )}
                  </div>
                ))}

                {addingTopicFor === coop.id ? (
                  <div style={{ padding: '6px 8px' }}>
                    <input placeholder="Topic name" value={topicName} onChange={e => setTopicName(e.target.value)} style={{ ...inputStyle, marginBottom: 4 }} autoFocus />
                    <div style={{ display: 'flex', gap: 4 }}>
                      <button onClick={() => handleAddTopic(coop.id)} disabled={savingTopic} style={btnStyle()}>
                        {savingTopic ? '…' : 'Add'}
                      </button>
                      <button onClick={() => setAddingTopicFor(null)} style={btnStyle('#1e293b')}>Cancel</button>
                    </div>
                  </div>
                ) : (
                  <button
                    onClick={() => { setAddingTopicFor(coop.id); setTopicName(''); }}
                    style={{ fontSize: 10, color: '#475569', background: 'none', border: 'none', cursor: 'pointer', padding: '4px 8px', display: 'flex', alignItems: 'center', gap: 3 }}
                  >
                    <Plus className="w-3 h-3" /> Add Topic
                  </button>
                )}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

export default function WorkPage() {
  const [graph, setGraph] = useState<WorkGraph>({ coops: [] });
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedSkill, setSelectedSkill] = useState<SkillNodeData | null>(null);

  async function loadAll() {
    try {
      const [g, p] = await Promise.all([
        invoke<WorkGraph>('get_full_work_graph'),
        invoke<Project[]>('get_projects'),
      ]);
      setGraph(g);
      setProjects(p);
    } catch (e) {
      console.error('Failed to load work graph:', e);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { loadAll(); }, []);

  const hasSkills = graph.coops.some(c =>
    c.topics.some(t => t.resources.some(r => r.skills.length > 0))
  );

  return (
    <div style={{ display: 'flex', height: '100%', overflow: 'hidden' }}>
      <LeftPanel graph={graph} onRefresh={loadAll} />

      {/* D3 galaxy canvas */}
      <div
        style={{ flex: 1, position: 'relative', background: '#020817' }}
        onClick={() => setSelectedSkill(null)}
      >
        {loading ? (
          <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', color: '#64748b', fontSize: 13 }}>
            Loading…
          </div>
        ) : !hasSkills ? (
          <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', color: '#475569', fontSize: 13, gap: 8, textAlign: 'center', padding: 32 }}>
            <span style={{ fontSize: 32 }}>🌌</span>
            <p>Your skill galaxy will appear here once you add a co-op, topics, and resources.</p>
            <p style={{ fontSize: 11 }}>Skills are extracted automatically when you save a resource.</p>
          </div>
        ) : (
          <GalaxyCanvas
            graph={graph}
            projects={projects}
            onSkillClick={setSelectedSkill}
          />
        )}

        {selectedSkill && (
          <SkillPopover data={selectedSkill} onClose={() => setSelectedSkill(null)} />
        )}
      </div>

      <style>{`
        @keyframes spin { to { transform: rotate(360deg) } }
        @keyframes gx-pulse {
          from { opacity: var(--min-op, 0.02); }
          to   { opacity: var(--max-op, 0.12); }
        }
      `}</style>
    </div>
  );
}
