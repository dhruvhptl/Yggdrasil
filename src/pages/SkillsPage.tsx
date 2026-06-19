// src/pages/SkillsPage.tsx
// Universal Skill Graph — depth-stratified grid: foundations at bottom, advanced skills at top.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  RefreshCw, Loader2, AlertTriangle, Sparkles,
  X, Wand2, GitMerge, Check, Search, Eye, Download, ChevronDown, ChevronRight,
  PanelLeftClose, PanelLeftOpen, RotateCcw, TrendingUp, Zap, Route, BookOpen,
  Maximize2, MoreHorizontal, ShieldCheck, CheckCircle, XCircle, Trash2,
} from 'lucide-react';
import type { UniversalSkill, SkillGap, SkillDependency, SkillAlias, SkillGraphSnapshot, GrowthTarget, PathNode, PrereqPath, LearningStep, LearningPath, Proposal } from '../types';
import { validateOrLog, SkillSchema } from '../lib/validators';
import { z } from 'zod';

// ─── Constants ────────────────────────────────────────────────────────────────

const DOMAIN_COLORS = [
  '#10b981', // emerald
  '#4a9eff', // blue
  '#a855f7', // purple
  '#ef4444', // red
  '#f59e0b', // amber
  '#14b8a6', // teal
  '#ec4899', // pink
  '#84cc16', // lime
  '#f97316', // orange
  '#06b6d4', // cyan
  '#8b5cf6', // violet
  '#22d3ee', // sky
];

const LEVEL_LABELS = ['', 'Aware', 'Familiar', 'Proficient', 'Advanced', 'Expert'];
// Node radius by level (px, pre-scale)
const NODE_R = [0, 6, 8, 10, 12, 14];
const GAP_COLOR = '#f59e0b';
const GAP_NODE_R = 4;
const MAX_DEPTH = 6;

// ─── Types ────────────────────────────────────────────────────────────────────

interface SuggestedMerge {
  canonicalId: string;
  canonicalName: string;
  aliasIds: string[];
  aliasNames: string[];
  reason: string;
}

interface SkillPlacement {
  skill: UniversalSkill | null;
  gap: SkillGap | null;
  x: number;
  y: number;
  color: string;
  domainIndex: number;
  /** prerequisite depth (0 = foundation) */
  outAngle: number;
}

interface DomainLabel {
  name: string;
  color: string;
  x: number;
  y: number;
}

interface Lane {
  domainName: string;
  domainIndex: number;
  color: string;
  left: number;
  right: number;
  headerY: number;
}

interface LayoutResult {
  placements: SkillPlacement[];
  domainLabels: DomainLabel[];
  lanes: Lane[];
  bandHeight: number;
  maxDepth: number;
}

// ─── Seeded pseudo-random ─────────────────────────────────────────────────────

function seededRand(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) & 0xffffffff;
    return (s >>> 0) / 0xffffffff;
  };
}

// ─── Layout ───────────────────────────────────────────────────────────────────

function layoutSkillGraph(
  skills: UniversalSkill[],
  gaps: SkillGap[],
  showGaps: boolean,
  deps: SkillDependency[],
  w: number,
  h: number,
): LayoutResult {
  const placements: SkillPlacement[] = [];
  const domainLabels: DomainLabel[] = []; // always empty — lane headers replace these
  const lanes: Lane[] = [];

  if (skills.length === 0) return { placements, domainLabels, lanes, bandHeight: 80, maxDepth: 0 };

  const rand = seededRand(42);

  // ── Step 1: BFS depth per skill ───────────────────────────────────────────
  const prereqEdges = deps.filter(d => d.relationship === 'prerequisite' || d.relationship === 'part_of');
  const prereqCount = new Map<string, number>();
  const successors  = new Map<string, string[]>();
  for (const s of skills) { prereqCount.set(s.id, 0); successors.set(s.id, []); }
  for (const e of prereqEdges) {
    if (!prereqCount.has(e.sourceSkillId) || !prereqCount.has(e.targetSkillId)) continue;
    prereqCount.set(e.targetSkillId, (prereqCount.get(e.targetSkillId) ?? 0) + 1);
    successors.get(e.sourceSkillId)!.push(e.targetSkillId);
  }

  const depth = new Map<string, number>();
  const visited = new Set<string>();
  const queue: string[] = [];
  for (const s of skills) {
    if ((prereqCount.get(s.id) ?? 0) === 0) {
      depth.set(s.id, 0);
      queue.push(s.id);
      visited.add(s.id);
    }
  }
  let qi = 0;
  while (qi < queue.length) {
    const id = queue[qi++];
    const d = depth.get(id) ?? 0;
    for (const nextId of (successors.get(id) ?? [])) {
      if (!visited.has(nextId)) {
        visited.add(nextId);
        depth.set(nextId, d + 1);
        queue.push(nextId);
      }
    }
  }
  for (const s of skills) { if (!depth.has(s.id)) depth.set(s.id, 0); }

  const maxDepth = Math.min(MAX_DEPTH, Math.max(...[...depth.values()]));

  // ── Step 2: Group by domain ───────────────────────────────────────────────
  const domainNames = [...new Set(skills.map(s => s.domain || 'General'))];
  const domainIndexMap = new Map(domainNames.map((d, i) => [d, i]));
  const totalSkills = skills.length;

  // ── Step 3: Lane allocation (proportional width) ──────────────────────────
  const padLR = 40;
  const usableW = w - 2 * padLR;
  const padTop = 60, padBottom = 60;
  const usableH = h - padTop - padBottom;
  const bandHeight = Math.max(80, usableH / (maxDepth + 1));

  function depthToY(d: number): number {
    return h - padBottom - d * bandHeight;
  }

  // Count skills per domain for proportional widths
  const domainSkillCount = new Map<number, number>();
  for (const s of skills) {
    const di = domainIndexMap.get(s.domain || 'General') ?? 0;
    domainSkillCount.set(di, (domainSkillCount.get(di) ?? 0) + 1);
  }

  // Compute raw widths and normalize if needed
  const rawWidths = domainNames.map((_, di) => {
    const count = domainSkillCount.get(di) ?? 0;
    return Math.max(80, (count / Math.max(totalSkills, 1)) * usableW);
  });
  const rawTotal = rawWidths.reduce((a, b) => a + b, 0);
  const scale = rawTotal > usableW ? usableW / rawTotal : 1;
  const laneWidths = rawWidths.map(rw => rw * scale);

  // Build lane structs (headerY computed after placements)
  let laneX = padLR;
  for (let di = 0; di < domainNames.length; di++) {
    const lw = laneWidths[di];
    lanes.push({
      domainName: domainNames[di],
      domainIndex: di,
      color: DOMAIN_COLORS[di % DOMAIN_COLORS.length],
      left: laneX,
      right: laneX + lw,
      headerY: padTop - 10, // default; will be updated below
    });
    laneX += lw;
  }

  // ── Step 4: Group skills by (domainIdx, depth) cell ──────────────────────
  type Cell = { skills: UniversalSkill[] };
  const grid = new Map<string, Cell>();
  const cellKey = (di: number, d: number) => `${di}:${d}`;

  for (const s of skills) {
    const di = domainIndexMap.get(s.domain || 'General') ?? 0;
    const d  = Math.min(depth.get(s.id) ?? 0, MAX_DEPTH);
    const key = cellKey(di, d);
    if (!grid.has(key)) grid.set(key, { skills: [] });
    grid.get(key)!.skills.push(s);
  }

  // ── Step 5: Place nodes in cells ─────────────────────────────────────────
  for (const [key, cell] of grid.entries()) {
    const [diStr, dStr] = key.split(':');
    const di    = parseInt(diStr);
    const d     = parseInt(dStr);
    const lane  = lanes[di];
    const count = cell.skills.length;
    const color = DOMAIN_COLORS[di % DOMAIN_COLORS.length];
    const laneW = lane.right - lane.left;

    cell.skills.forEach((skill, i) => {
      const frac = count <= 1 ? 0.5 : i / (count - 1);
      const innerW = laneW * 0.80;
      const innerLeft = lane.left + laneW * 0.10;
      const baseX = innerLeft + frac * innerW;
      const baseY = depthToY(d);
      const x = baseX + (rand() - 0.5) * 24;
      const y = baseY + (rand() - 0.5) * bandHeight * 0.35;
      placements.push({
        skill, gap: null, x, y,
        color, domainIndex: di,
        outAngle: d,
      });
    });
  }

  // ── Step 6: Repulsion within each cell (3 iterations) ────────────────────
  for (const [key, cell] of grid.entries()) {
    const cellPlacements = cell.skills.map(s => placements.find(p => p.skill?.id === s.id)!).filter(Boolean);
    for (let iter = 0; iter < 3; iter++) {
      for (let a = 0; a < cellPlacements.length; a++) {
        for (let b = a + 1; b < cellPlacements.length; b++) {
          const pa = cellPlacements[a], pb = cellPlacements[b];
          const dx = pb.x - pa.x, dy = pb.y - pa.y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 0.001;
          if (dist < 28) {
            const push = (28 - dist) / 2;
            const nx = dx / dist, ny = dy / dist;
            pa.x -= nx * push; pa.y -= ny * push;
            pb.x += nx * push; pb.y += ny * push;
          }
        }
      }
    }
    void key; // suppress unused warning
  }

  // ── Step 7: Gap nodes ─────────────────────────────────────────────────────
  if (showGaps && gaps.length > 0) {
    const existingNames = new Set(skills.map(s => s.name.toLowerCase()));
    const visibleGaps = gaps.filter(g => !existingNames.has(g.skillName.toLowerCase()));
    if (visibleGaps.length > 0) {
      const gapY = depthToY(0);
      const gapBandW = usableW / Math.max(1, visibleGaps.length);
      visibleGaps.forEach((gap, i) => {
        const x = padLR + (i + 0.5) * gapBandW + (rand() - 0.5) * 18;
        placements.push({
          skill: null, gap, x, y: gapY + (rand() - 0.5) * 20,
          color: GAP_COLOR, domainIndex: -1, outAngle: 0,
        });
      });
    }
  }

  // ── Step 8: Compute lane headerY from topmost node in each lane ───────────
  for (let di = 0; di < lanes.length; di++) {
    const laneNodes = placements.filter(p => p.domainIndex === di);
    if (laneNodes.length > 0) {
      const minY = Math.min(...laneNodes.map(p => p.y));
      lanes[di].headerY = Math.max(padTop - 10, minY - 24);
    }
  }

  return { placements, domainLabels, lanes, bandHeight, maxDepth };
}

// ─── Canvas helpers ───────────────────────────────────────────────────────────

function hexToRgb(hex: string) {
  if (!hex || hex.length < 7) return { r: 100, g: 100, b: 100 };
  return {
    r: parseInt(hex.slice(1, 3), 16),
    g: parseInt(hex.slice(3, 5), 16),
    b: parseInt(hex.slice(5, 7), 16),
  };
}

// Seeded starfield (stable, only rebuilt when dimensions change)
let _starCache: { x: number; y: number; r: number; a: number; phase: number }[] | null = null;
let _starDims = { w: 0, h: 0 };
function getStars(w: number, h: number) {
  if (_starCache && _starDims.w === w && _starDims.h === h) return _starCache;
  const rng = seededRand(7331);
  _starCache = Array.from({ length: 320 }, () => ({
    x: rng() * w, y: rng() * h,
    r: 0.4 + rng() * 1.3,
    a: 0.3 + rng() * 0.65,
    phase: rng() * Math.PI * 2,
  }));
  _starDims = { w, h };
  return _starCache;
}

function drawBackground(ctx: CanvasRenderingContext2D, w: number, h: number, animTime: number, domainColors: string[]) {
  const skyGrad = ctx.createLinearGradient(0, 0, 0, h);
  skyGrad.addColorStop(0, '#01020a');
  skyGrad.addColorStop(0.4, '#020408');
  skyGrad.addColorStop(1, '#030608');
  ctx.fillStyle = skyGrad;
  ctx.fillRect(0, 0, w, h);

  // Per-domain nebula blobs
  const ncx = w * 0.5, ncy = h * 0.55;
  domainColors.forEach((color, i) => {
    const c = hexToRgb(color);
    const ang = ((i / (domainColors.length || 1)) * Math.PI * 2) - Math.PI / 2;
    const nr = Math.min(w, h) * 0.30;
    const nbx = ncx + nr * Math.cos(ang), nby = ncy + nr * Math.sin(ang);
    const g = ctx.createRadialGradient(nbx, nby, 0, nbx, nby, Math.min(w, h) * 0.22);
    g.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.05)`);
    g.addColorStop(0.5, `rgba(${c.r},${c.g},${c.b},0.018)`);
    g.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0)`);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
  });

  // Milky way haze
  const mw = ctx.createLinearGradient(0, h * 0.1, w, h * 0.9);
  mw.addColorStop(0,   'rgba(100,120,180,0)');
  mw.addColorStop(0.3, 'rgba(80,100,160,0.016)');
  mw.addColorStop(0.5, 'rgba(100,120,200,0.030)');
  mw.addColorStop(0.7, 'rgba(80,100,160,0.016)');
  mw.addColorStop(1,   'rgba(100,120,180,0)');
  ctx.fillStyle = mw;
  ctx.fillRect(0, 0, w, h);

  // Stars
  const stars = getStars(w, h);
  stars.forEach(s => {
    const twinkle = s.a * (0.55 + 0.45 * Math.sin(animTime * 0.75 + s.phase));
    ctx.beginPath();
    ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(220,235,255,${twinkle.toFixed(3)})`;
    ctx.fill();
  });
}


// ─── Lane / depth draw helpers ────────────────────────────────────────────────

function drawLaneSeparators(ctx: CanvasRenderingContext2D, lanes: Lane[], canvasH: number) {
  ctx.save();
  ctx.strokeStyle = 'rgba(255,255,255,0.04)';
  ctx.lineWidth = 1;
  // Draw vertical lines at each lane boundary (between lanes)
  for (let i = 1; i < lanes.length; i++) {
    const x = lanes[i].left;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, canvasH);
    ctx.stroke();
  }
  ctx.restore();
}

function drawDepthGuides(
  ctx: CanvasRenderingContext2D,
  maxDepth: number,
  bandHeight: number,
  padBottom: number,
  _w: number,
  h: number,
) {
  if (maxDepth < 2) return;
  const depthToY = (d: number) => h - padBottom - d * bandHeight;

  ctx.save();
  ctx.font = '400 9px DM Sans, system-ui';
  ctx.fillStyle = 'rgba(255,255,255,0.18)';
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';

  const labels: { d: number; label: string }[] = [
    { d: 0,                         label: 'Foundations' },
    { d: Math.ceil(maxDepth / 2),   label: 'Intermediate' },
    { d: maxDepth,                  label: 'Advanced' },
  ];

  for (const { d, label } of labels) {
    ctx.fillText(label, 8, depthToY(d));
  }
  ctx.restore();
}

function drawLaneHeaders(ctx: CanvasRenderingContext2D, lanes: Lane[], growFrac: number) {
  if (growFrac <= 0.5) return;
  const alpha = Math.min(1, (growFrac - 0.5) / 0.4);
  ctx.save();
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.font = '600 11px DM Sans, system-ui';

  for (const lane of lanes) {
    const laneW = lane.right - lane.left;
    const cx = lane.left + laneW / 2;
    const c = hexToRgb(lane.color);
    ctx.fillStyle = `rgba(${c.r},${c.g},${c.b},${(0.65 * alpha).toFixed(3)})`;
    ctx.fillText(lane.domainName, cx, lane.headerY);
  }
  ctx.restore();
}

function drawLegend(ctx: CanvasRenderingContext2D, _w: number, h: number) {
  const legendW = 190, legendH = 90;
  const lx = 16, ly = h - 16 - legendH;

  // Background
  ctx.save();
  ctx.fillStyle = 'rgba(4,8,20,0.72)';
  ctx.strokeStyle = 'rgba(255,255,255,0.07)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(lx, ly, legendW, legendH, 8);
  ctx.fill();
  ctx.stroke();

  ctx.font = '400 9px DM Sans, system-ui';
  ctx.textBaseline = 'middle';

  // Row 1: Level label + circles
  ctx.fillStyle = '#64748b';
  ctx.textAlign = 'left';
  ctx.fillText('Level', lx + 8, ly + 14);

  for (let i = 0; i < 5; i++) {
    const cx = lx + 38 + i * 20;
    const cy = ly + 14;
    const r = NODE_R[i + 1];
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fillStyle = '#94a3b8';
    ctx.fill();
    ctx.font = '400 8px DM Sans, system-ui';
    ctx.fillStyle = '#64748b';
    ctx.textAlign = 'center';
    ctx.fillText(`L${i + 1}`, cx, cy + r + 6);
    ctx.font = '400 9px DM Sans, system-ui';
  }

  // Row 2: seed / target
  ctx.textAlign = 'left';
  let rx = lx + 8;
  const row2y = ly + 44;

  // seed circle
  ctx.beginPath();
  ctx.arc(rx + 5, row2y, 5, 0, Math.PI * 2);
  ctx.fillStyle = '#f59e0b';
  ctx.fill();
  ctx.fillStyle = '#64748b';
  ctx.fillText(' seed', rx + 5 + 5, row2y);

  rx += 5 + 5 + ctx.measureText(' seed').width + 12;

  // target circle
  ctx.beginPath();
  ctx.arc(rx + 5, row2y, 5, 0, Math.PI * 2);
  ctx.fillStyle = '#34d399';
  ctx.fill();
  ctx.fillStyle = '#64748b';
  ctx.fillText(' target', rx + 5 + 5, row2y);

  // Row 3: prereq line / related line
  const row3y = ly + 68;
  rx = lx + 8;

  // prereq — solid indigo line
  ctx.beginPath();
  ctx.moveTo(rx, row3y);
  ctx.lineTo(rx + 24, row3y);
  ctx.strokeStyle = '#818cf8';
  ctx.lineWidth = 1.2;
  ctx.setLineDash([]);
  ctx.stroke();
  ctx.fillStyle = '#64748b';
  ctx.textAlign = 'left';
  ctx.fillText(' prereq', rx + 24, row3y);

  rx += 24 + ctx.measureText(' prereq').width + 12;

  // related — dashed gray line
  ctx.beginPath();
  ctx.moveTo(rx, row3y);
  ctx.lineTo(rx + 24, row3y);
  ctx.strokeStyle = '#64748b';
  ctx.lineWidth = 1.2;
  ctx.setLineDash([3, 5]);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = '#64748b';
  ctx.fillText(' related', rx + 24, row3y);

  ctx.restore();
}

// ─── Skill Node Drawing ───────────────────────────────────────────────────────
// Circles with level-scaled radius, glow layers, progress arc for growing nodes.

function drawSkillNode(
  ctx: CanvasRenderingContext2D,
  x: number, y: number,
  color: string,
  level: number,
  isHovered: boolean,
  isSelected: boolean,
  animTime: number,
  domainAlpha: number,       // 1 = full, 0.10 = dimmed
  growFrac: number,
  isSeed: boolean = false,
  isTarget: boolean = false,
) {
  if (growFrac < 0.01) return;
  const r = NODE_R[Math.max(1, Math.min(5, level))] * Math.min(growFrac * 2, 1);
  if (r < 0.5) return;
  const c = hexToRgb(color);
  ctx.globalAlpha = domainAlpha;

  // Seed ring: warm gold outer ring
  if (isSeed) {
    const seedPulse = 0.6 + 0.4 * Math.sin(animTime * 1.8);
    const seedR = r + 5;
    ctx.beginPath();
    ctx.arc(x, y, seedR, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(245,158,11,${(0.70 * seedPulse * domainAlpha).toFixed(3)})`;
    ctx.lineWidth = 1.8;
    ctx.globalAlpha = 1;
    ctx.stroke();
    ctx.globalAlpha = domainAlpha;
  }

  // Target ring: double-intensity green pulse
  if (isTarget) {
    const tPulse = 0.5 + 0.5 * Math.sin(animTime * 2.4);
    const tR1 = r + 6;
    const tR2 = r + 10;
    const tg = ctx.createRadialGradient(x, y, tR1, x, y, tR2);
    tg.addColorStop(0, `rgba(52,211,153,${(0.50 * tPulse).toFixed(3)})`);
    tg.addColorStop(1, `rgba(52,211,153,0)`);
    ctx.beginPath();
    ctx.arc(x, y, tR2, 0, Math.PI * 2);
    ctx.fillStyle = tg;
    ctx.globalAlpha = domainAlpha;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(x, y, tR1, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(52,211,153,${(0.80 * tPulse * domainAlpha).toFixed(3)})`;
    ctx.lineWidth = 1.5;
    ctx.globalAlpha = 1;
    ctx.stroke();
    ctx.globalAlpha = domainAlpha;
  }

  // Outer glow
  const glowR = r * (2.8 + (isHovered ? 1.2 : 0));
  const glow = ctx.createRadialGradient(x, y, 0, x, y, glowR);
  glow.addColorStop(0, `rgba(${c.r},${c.g},${c.b},${level >= 4 ? 0.35 : 0.18})`);
  glow.addColorStop(0.5, `rgba(${c.r},${c.g},${c.b},0.07)`);
  glow.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0)`);
  ctx.beginPath();
  ctx.arc(x, y, glowR, 0, Math.PI * 2);
  ctx.fillStyle = glow;
  ctx.fill();

  // Progress ring for level 2-3 (growing)
  if (level >= 2 && level <= 3) {
    const progress = level === 2 ? 0.40 : 0.75;
    const ringR = r + 4;
    ctx.beginPath();
    ctx.arc(x, y, ringR, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(${c.r},${c.g},${c.b},0.18)`;
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(x, y, ringR, -Math.PI / 2, -Math.PI / 2 + progress * Math.PI * 2);
    ctx.strokeStyle = `rgba(${c.r},${c.g},${c.b},0.85)`;
    ctx.lineWidth = 1.5;
    ctx.lineCap = 'round';
    ctx.stroke();
    ctx.lineCap = 'butt';
  }

  // Bloomed sparkles (level 5)
  if (level === 5) {
    const baseAng = animTime * 0.55;
    for (let i = 0; i < 5; i++) {
      const theta = baseAng + (i / 5) * Math.PI * 2;
      const dist  = (r + 7) * (0.75 + 0.25 * Math.sin(animTime * 1.3 + i * 2.1));
      const sa    = (0.35 + 0.40 * (Math.sin(animTime * 2.6 + i * 1.8) * 0.5 + 0.5)) * domainAlpha;
      ctx.beginPath();
      ctx.arc(x + Math.cos(theta) * dist, y + Math.sin(theta) * dist, 1.2, 0, Math.PI * 2);
      ctx.fillStyle = '#ffffff';
      ctx.globalAlpha = sa;
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  // Budding pulse (level 1)
  if (level === 1) {
    const pulse = 0.5 + 0.5 * Math.sin(animTime * 2.0);
    const pulseR = r * (1.4 + 0.4 * pulse);
    const pg = ctx.createRadialGradient(x, y, r, x, y, pulseR);
    pg.addColorStop(0, `rgba(${c.r},${c.g},${c.b},${(0.28 * pulse).toFixed(3)})`);
    pg.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0)`);
    ctx.beginPath();
    ctx.arc(x, y, pulseR, 0, Math.PI * 2);
    ctx.fillStyle = pg;
    ctx.fill();
  }

  // Selection ring
  if (isSelected) {
    ctx.beginPath();
    ctx.arc(x, y, r + 5, 0, Math.PI * 2);
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.8;
    ctx.globalAlpha = 0.9 * domainAlpha;
    ctx.stroke();
    ctx.globalAlpha = domainAlpha;
  }

  // Core fill
  const fill = ctx.createRadialGradient(x - r * 0.25, y - r * 0.25, 0, x, y, r);
  if (level === 0) {
    fill.addColorStop(0, '#2a2a3a');
    fill.addColorStop(1, '#16161f');
  } else {
    fill.addColorStop(0, '#ffffff');
    fill.addColorStop(0.3, color);
    fill.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.4)`);
  }
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = fill;
  ctx.fill();

  // Rim
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.strokeStyle = level === 0 ? '#2e2e3e' : (isHovered ? '#ffffff88' : color + 'aa');
  ctx.lineWidth = isHovered ? 1.8 : 0.8;
  ctx.stroke();

  ctx.globalAlpha = 1;
}

function drawGapNode(
  ctx: CanvasRenderingContext2D,
  x: number, y: number,
  _animTime: number,
  domainAlpha: number,
) {
  ctx.globalAlpha = 0.45 * domainAlpha;
  ctx.beginPath();
  ctx.arc(x, y, GAP_NODE_R, 0, Math.PI * 2);
  ctx.fillStyle = '#2a2000';
  ctx.fill();
  ctx.strokeStyle = GAP_COLOR + '66';
  ctx.lineWidth = 0.8;
  ctx.stroke();
  ctx.globalAlpha = 1;
}

// Tooltip-style hover label
function drawHoverLabel(
  ctx: CanvasRenderingContext2D,
  p: SkillPlacement,
  centerX: number,
) {
  const skill = p.skill;
  const gap   = p.gap;
  const label = skill
    ? (skill.name.length > 32 ? skill.name.slice(0, 32) + '…' : skill.name)
    : (gap?.skillName.slice(0, 32) ?? '');
  const sub = skill
    ? `L${skill.level} · ${skill.evidence.length} evidence${skill.level > 0 ? ' · ' + LEVEL_LABELS[skill.level] : ''}`
    : `Gap · ${gap?.demandCount ?? 0} job(s)`;

  ctx.font = '600 11px "DM Sans", system-ui, sans-serif';
  const labelW = ctx.measureText(label).width;
  ctx.font = '400 9.5px "DM Sans", system-ui, sans-serif';
  const subW = ctx.measureText(sub).width;
  const boxW = Math.max(labelW, subW) + 20;
  const boxH = 34;

  // Position: right of node if left-half, left if right-half
  const lx = p.x > centerX ? p.x + 16 : p.x - 16 - boxW;
  const ly = p.y - boxH / 2;

  // Pill background
  ctx.fillStyle = 'rgba(3,7,22,0.90)';
  ctx.beginPath();
  ctx.roundRect(lx, ly, boxW, boxH, 6);
  ctx.fill();
  ctx.strokeStyle = p.color + '44';
  ctx.lineWidth = 0.8;
  ctx.stroke();

  ctx.font = '600 11px "DM Sans", system-ui, sans-serif';
  ctx.fillStyle = skill ? '#f1f5f9' : GAP_COLOR;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  ctx.fillText(label, lx + 10, ly + 11);

  ctx.font = '400 9.5px "DM Sans", system-ui, sans-serif';
  ctx.fillStyle = '#64748b';
  ctx.fillText(sub, lx + 10, ly + 24);
  ctx.textAlign = 'left';
}


// Draw dep edges for selected or hovered skill — all relationship types styled distinctly
function drawDepEdges(
  ctx: CanvasRenderingContext2D,
  placements: SkillPlacement[],
  selectedId: string | null,
  hoveredId: string | null,
  deps: SkillDependency[],
) {
  const focusId = selectedId ?? hoveredId;
  if (!focusId) return;
  const bySkillId = new Map(placements.filter(p => p.skill).map(p => [p.skill!.id, p]));
  const focusPlacement = bySkillId.get(focusId);
  if (!focusPlacement) return;

  const relevant = deps.filter(d => d.sourceSkillId === focusId || d.targetSkillId === focusId);

  relevant.forEach(d => {
    const otherId = d.sourceSkillId === focusId ? d.targetSkillId : d.sourceSkillId;
    const other = bySkillId.get(otherId);
    if (!other) return;

    const rel = d.relationship;
    let color: string;
    let opacity: number;
    let lineWidth: number;
    let dashed = false;

    if (rel === 'prerequisite' || rel === 'part_of') {
      color = '#818cf8'; opacity = 0.55; lineWidth = 1.4;
    } else if (rel === 'specialization') {
      color = '#14b8a6'; opacity = 0.40; lineWidth = 1.2;
    } else {
      // related, co_occurs, or anything else
      color = '#64748b'; opacity = 0.30; lineWidth = 0.8; dashed = true;
    }

    const c = hexToRgb(color);
    ctx.beginPath();
    ctx.moveTo(focusPlacement.x, focusPlacement.y);
    ctx.lineTo(other.x, other.y);
    ctx.strokeStyle = `rgba(${c.r},${c.g},${c.b},${opacity})`;
    ctx.lineWidth = lineWidth;
    if (dashed) ctx.setLineDash([3, 5]);
    ctx.stroke();
    ctx.setLineDash([]);
  });
}

// ─── Level Dots ───────────────────────────────────────────────────────────────

function LevelDots({ level }: { level: number }) {
  return (
    <div className="flex gap-0.5">
      {[1, 2, 3, 4, 5].map(i => (
        <div key={i} className={`w-1.5 h-1.5 rounded-full ${i <= level ? 'bg-emerald-400' : 'bg-slate-700'}`} />
      ))}
    </div>
  );
}

// ─── Skill Detail panel (right-side drawer) ──────────────────────────────────

interface SkillDetail {
  skillId: string;
  name: string;
  displayName: string | null;
  domainName: string | null;
  origin: string;        // resume | work | job_gap | tree_quest
  state: string;         // seed | adjacent
  status: string;        // untouched | in_progress | practiced | mastered
  jobDemandCount: number;
  prereqs: Array<{ skillId: string; name: string }>;
  unlocks: Array<{ skillId: string; name: string; jobDemandCount: number }>;
  related: Array<{ skillId: string; name: string }>;
  trees: Array<{ treeId: string; treeTitle: string }>;
  projects: Array<{ projectId: string; name: string }>;
  resources: Array<{
    resourceId: string;
    title: string;
    url: string;
    relevanceScore: number | null;
    sectionTitle: string | null;
    pageStart: number | null;
    pageEnd: number | null;
  }>;
  notes: string | null;
}

const STATUS_CYCLE: Array<SkillDetail['status']> = ['untouched', 'in_progress', 'practiced', 'mastered'];
const STATUS_LABEL: Record<string, string> = {
  untouched: 'Untouched',
  in_progress: 'In progress',
  practiced: 'Practiced',
  mastered: 'Mastered',
};
const STATUS_STYLE: Record<string, { bg: string; color: string; border: string }> = {
  untouched:   { bg: 'rgba(148,163,184,0.10)', color: '#94a3b8', border: 'rgba(148,163,184,0.22)' },
  in_progress: { bg: 'rgba(99,102,241,0.12)',  color: '#a5b4fc', border: 'rgba(99,102,241,0.28)' },
  practiced:   { bg: 'rgba(245,158,11,0.12)',  color: '#fcd34d', border: 'rgba(245,158,11,0.28)' },
  mastered:    { bg: 'rgba(16,185,129,0.14)',  color: '#6ee7b7', border: 'rgba(16,185,129,0.30)' },
};

function isGapMode(origin: string): boolean {
  return origin === 'job_gap' || origin === 'tree_quest';
}

function SkillDetailPanel({
  skillId, onClose, onSelectSkill, onOpenGrowthPlan,
}: {
  skillId: string;
  onClose: () => void;
  onSelectSkill?: (name: string) => void;
  onOpenGrowthPlan?: () => void;
}) {
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [statusSaving, setStatusSaving] = useState(false);
  const [notesDraft, setNotesDraft] = useState<string>('');
  const [notesSaving, setNotesSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  async function handleOpenTree() {
    if (!detail || opening) return;
    setOpening(true);
    setOpenError(null);
    const result = await openSkillTree(detail.skillId);
    if (!result.ok) setOpenError(result.error);
    setOpening(false);
  }
  const [prereqPath, setPrereqPath] = useState<PrereqPath | null>(null);
  const [similar, setSimilar] = useState<Array<{ skillId: string; name: string; distance: number }> | null>(null);
  const [similarLoading, setSimilarLoading] = useState(false);

  // Fetch on skill change
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setDetail(null);
    setPrereqPath(null);
    invoke<SkillDetail>('get_skill_detail', { skillId })
      .then(d => {
        if (cancelled) return;
        setDetail(d);
        setNotesDraft(d.notes ?? '');
      })
      .catch(e => {
        if (cancelled) return;
        setError(typeof e === 'string' ? e : 'Failed to load skill detail');
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [skillId]);

  // Gap mode also needs prereq path
  useEffect(() => {
    if (!detail || !isGapMode(detail.origin) || detail.state === 'seed') {
      setPrereqPath(null);
      return;
    }
    let cancelled = false;
    invoke<PrereqPath>('get_prereq_path', { skillId: detail.skillId })
      .then(p => { if (!cancelled) setPrereqPath(p); })
      .catch(() => { if (!cancelled) setPrereqPath(null); });
    return () => { cancelled = true; };
  }, [detail?.skillId, detail?.origin, detail?.state]);

  // Escape to close
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Similar skills — ANN over embeddings
  useEffect(() => {
    let cancelled = false;
    setSimilarLoading(true);
    setSimilar(null);
    invoke<Array<{ skillId: string; name: string; distance: number }>>(
      'get_similar_skills',
      { skillId, limit: 8 },
    )
      .then(rows => { if (!cancelled) setSimilar(rows); })
      .catch(e => { if (!cancelled) { console.error('get_similar_skills failed:', e); setSimilar([]); } })
      .finally(() => { if (!cancelled) setSimilarLoading(false); });
    return () => { cancelled = true; };
  }, [skillId]);

  async function handleStatusClick() {
    if (!detail || statusSaving) return;
    const next = STATUS_CYCLE[(STATUS_CYCLE.indexOf(detail.status) + 1) % STATUS_CYCLE.length];
    setStatusSaving(true);
    try {
      await invoke('update_skill_status', { skillId: detail.skillId, status: next });
      setDetail({ ...detail, status: next });
    } catch (e) {
      console.error('update_skill_status failed:', e);
    } finally {
      setStatusSaving(false);
    }
  }

  async function handleNotesBlur() {
    if (!detail || notesSaving) return;
    if ((detail.notes ?? '') === notesDraft) return;
    setNotesSaving(true);
    try {
      await invoke('update_skill_notes', { skillId: detail.skillId, notes: notesDraft });
      setDetail({ ...detail, notes: notesDraft });
    } catch (e) {
      console.error('update_skill_notes failed:', e);
    } finally {
      setNotesSaving(false);
    }
  }

  // Note: outside-click is handled by stopPropagation in the panel + a click
  // handler on the canvas (existing pattern). Escape works via the listener above.

  const baseColor = detail && isGapMode(detail.origin) ? GAP_COLOR : '#10b981';
  const titleColor = detail && isGapMode(detail.origin) ? GAP_COLOR : '#f1f5f9';

  return (
    <div
      style={{
        position: 'absolute', top: 16, right: 16, zIndex: 50,
        width: 320, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: `1px solid ${baseColor}22`,
        borderRadius: 16,
        overflow: 'auto',
        boxShadow: `0 12px 48px rgba(0,0,0,0.85), 0 0 0 1px ${baseColor}11`,
      }}
      onClick={e => e.stopPropagation()}
    >
      {/* Header */}
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 8 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 15, fontWeight: 600, color: titleColor, marginBottom: 4, lineHeight: 1.3 }}>
              {detail?.displayName || detail?.name || (loading ? 'Loading…' : 'Skill')}
            </div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
              {detail?.domainName && (
                <span style={{ fontSize: 9, padding: '1px 6px', borderRadius: 3, background: 'rgba(99,102,241,0.10)', color: '#818cf8', border: '1px solid rgba(99,102,241,0.18)' }}>
                  {detail.domainName}
                </span>
              )}
              {detail && !isGapMode(detail.origin) && (
                <button
                  onClick={handleStatusClick}
                  disabled={statusSaving}
                  title="Click to cycle status"
                  style={{
                    fontSize: 9, padding: '2px 7px', borderRadius: 4,
                    background: STATUS_STYLE[detail.status]?.bg ?? 'rgba(148,163,184,0.10)',
                    color: STATUS_STYLE[detail.status]?.color ?? '#94a3b8',
                    border: `1px solid ${STATUS_STYLE[detail.status]?.border ?? 'rgba(148,163,184,0.22)'}`,
                    cursor: statusSaving ? 'default' : 'pointer',
                    display: 'inline-flex', alignItems: 'center', gap: 4,
                  }}
                >
                  {statusSaving && <Loader2 size={9} className="animate-spin" />}
                  {STATUS_LABEL[detail.status] ?? detail.status}
                </button>
              )}
              {detail && isGapMode(detail.origin) && detail.jobDemandCount > 0 && (
                <span style={{ fontSize: 10, color: '#78350f' }}>
                  {detail.jobDemandCount} job{detail.jobDemandCount !== 1 ? 's' : ''} require this
                </span>
              )}
            </div>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', flexShrink: 0, padding: 2 }}>
            <X size={14} />
          </button>
        </div>
      </div>

      {/* Body */}
      <div style={{ padding: '12px 16px' }}>
        {loading && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: '#475569' }}>
            <Loader2 size={11} className="animate-spin" />Loading…
          </div>
        )}
        {error && !loading && (
          <div style={{ fontSize: 11, color: '#fca5a5' }}>{error}</div>
        )}

        {detail && !loading && (
          <>
            {/* Gap mode — Distance section */}
            {isGapMode(detail.origin) && detail.state !== 'seed' && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Distance</div>
                {prereqPath && prereqPath.isReachable ? (
                  <>
                    <div style={{ fontSize: 11, color: '#94a3b8', marginBottom: 6 }}>
                      <span style={{ color: '#fcd34d', fontWeight: 600 }}>{prereqPath.path.length}</span> step{prereqPath.path.length !== 1 ? 's' : ''} from your seeds
                    </div>
                    <PrereqPathBreadcrumb path={prereqPath.path} onSelectSkill={onSelectSkill} />
                  </>
                ) : prereqPath && !prereqPath.isReachable ? (
                  <div style={{ fontSize: 10, color: '#334155', fontStyle: 'italic' }}>
                    No path from your seeds — run Infer Dependencies
                  </div>
                ) : (
                  <div style={{ fontSize: 10, color: '#334155', fontStyle: 'italic' }}>Finding path…</div>
                )}
              </div>
            )}

            {/* Graph — prereqs + unlocks (both modes) */}
            {(detail.prereqs.length > 0 || detail.unlocks.length > 0) && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Graph</div>
                {detail.prereqs.length > 0 && (
                  <div style={{ marginBottom: 6 }}>
                    <div style={{ fontSize: 9, color: '#475569', marginBottom: 2 }}>Prereqs</div>
                    {detail.prereqs.map(p => (
                      <div
                        key={p.skillId}
                        onClick={() => onSelectSkill?.(p.name)}
                        style={{ fontSize: 11, color: '#94a3b8', padding: '2px 0', cursor: onSelectSkill ? 'pointer' : 'default', display: 'flex', alignItems: 'center', gap: 5 }}
                      >
                        <span style={{ color: '#475569' }}>←</span>
                        {p.name}
                      </div>
                    ))}
                  </div>
                )}
                {detail.unlocks.length > 0 && (
                  <div>
                    <div style={{ fontSize: 9, color: '#475569', marginBottom: 2 }}>Unlocks</div>
                    {detail.unlocks.map(u => (
                      <div
                        key={u.skillId}
                        onClick={() => onSelectSkill?.(u.name)}
                        style={{ fontSize: 11, color: '#94a3b8', padding: '2px 0', cursor: onSelectSkill ? 'pointer' : 'default', display: 'flex', alignItems: 'center', gap: 5 }}
                      >
                        <span style={{ color: '#475569' }}>→</span>
                        <span style={{ flex: 1 }}>{u.name}</span>
                        {u.jobDemandCount > 0 && (
                          <span style={{ fontSize: 8, padding: '0 4px', borderRadius: 3, background: 'rgba(99,102,241,0.10)', color: '#818cf8', border: '1px solid rgba(99,102,241,0.18)' }}>
                            {u.jobDemandCount}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Related */}
            {detail.related.length > 0 && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Related</div>
                <div style={{ fontSize: 11, color: '#94a3b8' }}>
                  {detail.related.map(r => r.name).join(', ')}
                </div>
              </div>
            )}

            {/* Owned mode — Trees */}
            {!isGapMode(detail.origin) && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Trees</div>
                {detail.trees.length === 0 ? (
                  <>
                    <button
                      onClick={e => { e.stopPropagation(); handleOpenTree(); }}
                      disabled={opening}
                      title="Open existing tree or generate a graph-seeded tree"
                      style={{
                        display: 'inline-flex', alignItems: 'center', gap: 4,
                        fontSize: 10, padding: '3px 8px', borderRadius: 4,
                        background: opening ? 'rgba(255,255,255,0.05)' : 'rgba(16,185,129,0.10)',
                        color: opening ? '#475569' : '#34d399',
                        border: '1px solid ' + (opening ? 'rgba(255,255,255,0.08)' : 'rgba(16,185,129,0.2)'),
                        cursor: opening ? 'default' : 'pointer',
                      }}
                    >
                      {opening
                        ? <><Loader2 size={9} className="animate-spin" />Opening…</>
                        : <>Open tree →</>}
                    </button>
                    {openError && (
                      <div style={{ fontSize: 9, color: '#fca5a5', marginTop: 4 }}>{openError}</div>
                    )}
                  </>
                ) : (
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                    {detail.trees.map(t => (
                      <a
                        key={t.treeId}
                        href={`/trees?selected=${encodeURIComponent(t.treeId)}`}
                        onClick={e => e.stopPropagation()}
                        style={{ fontSize: 11, color: '#6ee7b7', textDecoration: 'none', padding: '3px 6px', borderRadius: 4, background: 'rgba(16,185,129,0.06)', border: '1px solid rgba(16,185,129,0.14)' }}
                      >
                        🌳 {t.treeTitle}
                      </a>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Owned mode — Projects */}
            {!isGapMode(detail.origin) && detail.projects.length > 0 && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Projects</div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                  {detail.projects.map(p => (
                    <a
                      key={p.projectId}
                      href={`/project/${p.projectId}`}
                      onClick={e => e.stopPropagation()}
                      style={{ fontSize: 11, color: '#a5b4fc', textDecoration: 'none', padding: '3px 6px', borderRadius: 4, background: 'rgba(99,102,241,0.06)', border: '1px solid rgba(99,102,241,0.14)' }}
                    >
                      📁 {p.name}
                    </a>
                  ))}
                </div>
              </div>
            )}

            {/* Resources (both modes) — chapter-granular when section data exists */}
            <div style={{ marginBottom: 14 }}>
              <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Resources</div>
              {detail.resources.length === 0 ? (
                <div style={{ fontSize: 10, color: '#334155', fontStyle: 'italic' }}>No resources linked yet</div>
              ) : (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                  {detail.resources.map((r, idx) => {
                    // Deep-link to PDF page when we have a page number; supported
                    // by most browser PDF viewers via the #page= fragment.
                    const href = r.pageStart && r.url ? `${r.url}#page=${r.pageStart}` : r.url;
                    const pageLabel = r.pageStart
                      ? (r.pageEnd && r.pageEnd !== r.pageStart
                          ? `p.${r.pageStart}–${r.pageEnd}`
                          : `p.${r.pageStart}`)
                      : null;
                    return (
                      <a
                        key={`${r.resourceId}:${r.sectionTitle ?? ''}:${idx}`}
                        href={href}
                        target="_blank"
                        rel="noreferrer"
                        onClick={e => e.stopPropagation()}
                        title={r.url}
                        style={{
                          fontSize: 11, color: '#6ee7b7', textDecoration: 'none',
                          padding: '3px 6px', borderRadius: 4,
                          background: 'rgba(16,185,129,0.06)',
                          border: '1px solid rgba(16,185,129,0.14)',
                          display: 'flex', flexDirection: 'column', gap: 1,
                          overflow: 'hidden',
                        }}
                      >
                        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          📖 {r.title || r.url}
                        </span>
                        {(r.sectionTitle || pageLabel) && (
                          <span style={{
                            fontSize: 9, color: '#475569', paddingLeft: 16,
                            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                          }}>
                            {r.sectionTitle && pageLabel
                              ? `${r.sectionTitle} · ${pageLabel}`
                              : r.sectionTitle ?? pageLabel}
                          </span>
                        )}
                      </a>
                    );
                  })}
                </div>
              )}
            </div>

            {/* Owned mode — Notes */}
            {!isGapMode(detail.origin) && (
              <div style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6, display: 'flex', alignItems: 'center', gap: 6 }}>
                  Notes
                  {notesSaving && <Loader2 size={9} className="animate-spin" />}
                </div>
                <textarea
                  value={notesDraft}
                  onChange={e => setNotesDraft(e.target.value)}
                  onBlur={handleNotesBlur}
                  placeholder="Add notes…"
                  style={{
                    width: '100%', minHeight: 60, resize: 'vertical',
                    fontSize: 11, lineHeight: 1.5,
                    background: 'rgba(255,255,255,0.03)',
                    border: '1px solid rgba(255,255,255,0.06)',
                    borderRadius: 6, padding: '6px 8px',
                    color: '#cbd5e1', outline: 'none',
                    fontFamily: 'inherit',
                  }}
                />
              </div>
            )}

            {/* Similar skills — ANN cross-links (both modes) */}
            {(similarLoading || (similar && similar.length > 0)) && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: '#cbd5e1', marginBottom: 6 }}>
                  Similar skills
                </div>
                {similarLoading ? (
                  <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                    {[0, 1, 2].map(i => (
                      <span
                        key={i}
                        style={{
                          display: 'inline-block',
                          width: 60 + i * 14, height: 18,
                          borderRadius: 4,
                          background: 'rgba(255,255,255,0.04)',
                          border: '1px solid rgba(255,255,255,0.06)',
                        }}
                      />
                    ))}
                  </div>
                ) : (
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                    {similar!.map(s => (
                      <button
                        key={s.skillId}
                        onClick={() => onSelectSkill?.(s.name)}
                        title={`Distance ${s.distance.toFixed(2)} (cosine, 0=identical)`}
                        style={{
                          display: 'inline-flex', alignItems: 'center', gap: 5,
                          fontSize: 10, padding: '2px 7px',
                          borderRadius: 4,
                          background: 'rgba(99,102,241,0.08)',
                          color: '#a5b4fc',
                          border: '1px solid rgba(99,102,241,0.18)',
                          cursor: onSelectSkill ? 'pointer' : 'default',
                        }}
                      >
                        <span>{s.name}</span>
                        <span style={{ color: '#64748b', fontSize: 9 }}>
                          {s.distance.toFixed(2)}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Gap mode — Start learning */}
            {isGapMode(detail.origin) && onOpenGrowthPlan && (
              <div style={{ marginTop: 10, paddingTop: 10, borderTop: '1px solid rgba(255,255,255,0.05)' }}>
                <button
                  onClick={onOpenGrowthPlan}
                  style={{ width: '100%', padding: '6px 10px', borderRadius: 6, background: 'rgba(245,158,11,0.12)', color: '#fcd34d', border: '1px solid rgba(245,158,11,0.28)', cursor: 'pointer', fontSize: 11, fontWeight: 500 }}
                >
                  Start learning →
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

// Gap-pseudo-node panel (for canvas `gap-<name>` orphans that have no
// universal_skills row). Distinct from gap-mode of SkillDetailPanel which is
// for real universal_skills rows with origin='job_gap'|'tree_quest'.
function GapPseudoPanel({ gap, onClose }: { gap: SkillGap; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div
      style={{
        position: 'absolute', top: 16, right: 16, zIndex: 50,
        width: 300, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: `1px solid ${GAP_COLOR}22`,
        borderRadius: 16,
        overflow: 'auto',
        boxShadow: `0 12px 48px rgba(0,0,0,0.85), 0 0 0 1px ${GAP_COLOR}11`,
      }}
      onClick={e => e.stopPropagation()}
    >
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 8 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: GAP_COLOR, marginBottom: 2 }}>{gap.skillName}</div>
            <div style={{ fontSize: 10, color: '#78350f' }}>Skill Gap — not yet mastered</div>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', flexShrink: 0, padding: 2 }}>
            <X size={14} />
          </button>
        </div>
      </div>
      <div style={{ padding: '10px 16px', display: 'flex', flexDirection: 'column', gap: 6, fontSize: 11, color: '#92400e' }}>
        <div>Demanded by <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{gap.demandCount}</span> job(s)</div>
        <div>Frequency: <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{(gap.frequency * 100).toFixed(0)}%</span></div>
        <div>Demand score: <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{gap.demandScore.toFixed(1)}</span></div>
      </div>
    </div>
  );
}

// ─── Auto-Merge Preview Modal ─────────────────────────────────────────────────

function AutoMergeModal({ suggestions, onClose, onConfirm, merging }: {
  suggestions: SuggestedMerge[];
  onClose: () => void;
  onConfirm: () => void;
  merging: boolean;
}) {
  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 100, background: 'rgba(0,0,0,0.80)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} onClick={onClose}>
      <div style={{ width: 520, maxHeight: '78vh', background: 'rgba(4,8,20,0.96)', border: '1px solid #1e293b', borderRadius: 16, display: 'flex', flexDirection: 'column', overflow: 'hidden', backdropFilter: 'blur(16px)' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid rgba(255,255,255,0.06)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <GitMerge size={15} style={{ color: '#10b981' }} />
            <span style={{ fontWeight: 600, fontSize: 13, color: '#f1f5f9' }}>Auto-merge duplicates</span>
            <span style={{ fontSize: 11, color: '#475569', marginLeft: 4 }}>{suggestions.length} suggestion{suggestions.length !== 1 ? 's' : ''}</span>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer' }}><X size={14} /></button>
        </div>

        {suggestions.length === 0 ? (
          <div style={{ padding: '32px 18px', textAlign: 'center', color: '#475569', fontSize: 13 }}>
            No duplicate skills detected.
          </div>
        ) : (
          <div style={{ flex: 1, overflowY: 'auto', padding: '8px 18px' }}>
            {suggestions.map((s, i) => (
              <div key={i} style={{ padding: '7px 0', borderBottom: '1px solid rgba(255,255,255,0.04)' }}>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 6, fontSize: 12 }}>
                  <span style={{ color: '#34d399', fontWeight: 500 }}>{s.canonicalName}</span>
                  <span style={{ color: '#334155' }}>←</span>
                  <span style={{ color: '#94a3b8' }}>{s.aliasNames.join(', ')}</span>
                </div>
                <div style={{ fontSize: 10, color: '#475569', marginTop: 2 }}>{s.reason}</div>
              </div>
            ))}
          </div>
        )}

        <div style={{ padding: '10px 18px', borderTop: '1px solid rgba(255,255,255,0.06)', display: 'flex', gap: 8 }}>
          <button onClick={onClose} style={{ flex: 1, padding: '6px 10px', borderRadius: 6, background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)', color: '#64748b', cursor: 'pointer', fontSize: 11 }}>
            Cancel
          </button>
          <button
            disabled={merging || suggestions.length === 0}
            onClick={onConfirm}
            style={{ flex: 2, padding: '6px 10px', borderRadius: 6, background: merging || suggestions.length === 0 ? 'rgba(16,185,129,0.06)' : 'rgba(16,185,129,0.18)', border: '1px solid rgba(16,185,129,0.25)', color: merging || suggestions.length === 0 ? '#334155' : '#34d399', cursor: merging || suggestions.length === 0 ? 'default' : 'pointer', fontSize: 11, fontWeight: 500, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5 }}
          >
            {merging ? <><Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} />Merging…</> : <><GitMerge size={12} />Merge {suggestions.length} group{suggestions.length !== 1 ? 's' : ''}</>}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Merge Modal ──────────────────────────────────────────────────────────────

function MergeModal({ skills, onClose, onMerged }: { skills: UniversalSkill[]; onClose: () => void; onMerged: () => void }) {
  const [query, setQuery]         = useState('');
  const [selected, setSelected]   = useState<Set<string>>(new Set());
  const [canonicalId, setCanonicalId] = useState<string | null>(null);
  const [merging, setMerging]     = useState(false);
  const [error, setError]         = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = query.toLowerCase();
    return q ? skills.filter(s => s.name.toLowerCase().includes(q)) : skills;
  }, [skills, query]);

  function toggleSkill(id: string) {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(id)) { next.delete(id); if (canonicalId === id) setCanonicalId(null); }
      else { next.add(id); if (!canonicalId) setCanonicalId(id); }
      return next;
    });
  }

  async function handleMerge() {
    if (!canonicalId || selected.size < 2) return;
    const aliasIds = [...selected].filter(id => id !== canonicalId);
    setMerging(true); setError(null);
    try { await invoke('merge_skills', { canonicalId, aliasIds }); onMerged(); onClose(); }
    catch (e) { setError(String(e)); }
    finally { setMerging(false); }
  }

  const canonical      = skills.find(s => s.id === canonicalId);
  const selectedSkills = skills.filter(s => selected.has(s.id));

  return (
    <div style={{ position: 'fixed', inset: 0, zIndex: 100, background: 'rgba(0,0,0,0.80)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} onClick={onClose}>
      <div style={{ width: 500, maxHeight: '78vh', background: 'rgba(4,8,20,0.96)', border: '1px solid #1e293b', borderRadius: 16, display: 'flex', flexDirection: 'column', overflow: 'hidden', backdropFilter: 'blur(16px)' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid rgba(255,255,255,0.06)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}><GitMerge size={15} style={{ color: '#10b981' }} /><span style={{ fontWeight: 600, fontSize: 13, color: '#f1f5f9' }}>Merge Skills</span></div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer' }}><X size={14} /></button>
        </div>
        <div style={{ padding: '8px 18px', borderBottom: '1px solid rgba(255,255,255,0.06)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, background: 'rgba(255,255,255,0.04)', borderRadius: 7, padding: '5px 10px' }}>
            <Search size={12} style={{ color: '#475569' }} />
            <input autoFocus value={query} onChange={e => setQuery(e.target.value)} placeholder="Filter skills…" style={{ flex: 1, background: 'none', border: 'none', outline: 'none', color: '#f1f5f9', fontSize: 12 }} />
          </div>
        </div>
        <div style={{ flex: 1, overflowY: 'auto', padding: '6px 18px' }}>
          {filtered.map(skill => {
            const isSel   = selected.has(skill.id);
            const isCanon = skill.id === canonicalId;
            return (
              <div key={skill.id} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 7px', borderRadius: 7, marginBottom: 2, background: isSel ? 'rgba(16,185,129,0.07)' : 'transparent', border: '1px solid ' + (isCanon ? 'rgba(16,185,129,0.35)' : 'transparent'), cursor: 'pointer' }} onClick={() => toggleSkill(skill.id)}>
                <div style={{ width: 14, height: 14, borderRadius: 3, flexShrink: 0, background: isSel ? '#10b981' : 'rgba(255,255,255,0.04)', border: '1px solid ' + (isSel ? '#10b981' : '#334155'), display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  {isSel && <Check size={9} color="#fff" />}
                </div>
                <span style={{ flex: 1, fontSize: 12, color: isSel ? '#d1fae5' : '#94a3b8' }}>{skill.name}</span>
                {isSel && !isCanon && <button onClick={e => { e.stopPropagation(); setSelected(prev => new Set([...prev, skill.id])); setCanonicalId(skill.id); }} style={{ fontSize: 9, padding: '1px 5px', borderRadius: 3, background: 'rgba(99,102,241,0.13)', border: '1px solid rgba(99,102,241,0.25)', color: '#818cf8', cursor: 'pointer' }}>canonical</button>}
                {isCanon && <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 3, background: 'rgba(16,185,129,0.13)', color: '#34d399' }}>✓ canonical</span>}
                <span style={{ fontSize: 10, color: '#334155' }}>L{skill.level}</span>
              </div>
            );
          })}
        </div>
        <div style={{ padding: '10px 18px', borderTop: '1px solid rgba(255,255,255,0.06)' }}>
          {selected.size >= 2 && canonical && (
            <div style={{ fontSize: 11, color: '#475569', marginBottom: 8 }}>
              Merge {selectedSkills.filter(s => s.id !== canonicalId).map(s => `"${s.name}"`).join(', ')} → <span style={{ color: '#10b981' }}>"{canonical.name}"</span>
            </div>
          )}
          {error && <div style={{ fontSize: 10, color: '#f87171', marginBottom: 6 }}>{error}</div>}
          <button onClick={handleMerge} disabled={selected.size < 2 || !canonicalId || merging} style={{ width: '100%', padding: '7px', borderRadius: 7, border: 'none', background: (selected.size >= 2 && canonicalId && !merging) ? '#059669' : 'rgba(255,255,255,0.04)', color: (selected.size >= 2 && canonicalId && !merging) ? '#fff' : '#334155', fontSize: 12, fontWeight: 500, cursor: (selected.size >= 2 && canonicalId && !merging) ? 'pointer' : 'default', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5 }}>
            {merging ? <><Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> Merging…</> : <><GitMerge size={12} /> Merge {selected.size >= 2 ? `${selected.size} skills` : 'skills'}</>}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── Growth Plan Panel ────────────────────────────────────────────────────────

function GrowthPlanPanel({
  targets, loading, expandingGraph, onClose, onExpandGraph, onSelectSkill,
}: {
  targets: GrowthTarget[];
  loading: boolean;
  expandingGraph: boolean;
  onClose: () => void;
  onExpandGraph: () => void;
  onSelectSkill?: (name: string) => void;
}) {
  const reachable     = targets.filter(t => t.isReachable).slice(0, 3);
  const disconnected  = targets.filter(t => !t.isReachable).slice(0, 2);
  const maxScore      = Math.max(...targets.map(t => t.finalScore), 1);

  return (
    <div
      style={{
        position: 'absolute', top: 16, left: 16, zIndex: 50,
        width: 300, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: '1px solid rgba(245,158,11,0.2)',
        borderRadius: 16,
        overflow: 'auto',
        boxShadow: '0 12px 48px rgba(0,0,0,0.85)',
      }}
      onClick={e => e.stopPropagation()}
    >
      {/* Header */}
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
          <TrendingUp size={13} style={{ color: '#f59e0b' }} />
          <span style={{ fontSize: 13, fontWeight: 600, color: '#fef3c7' }}>Growth Plan</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
          <button
            onClick={onExpandGraph}
            disabled={expandingGraph}
            title="Recompute seed adjacency"
            style={{ background: 'none', border: 'none', color: expandingGraph ? '#334155' : '#475569', cursor: expandingGraph ? 'default' : 'pointer', padding: 2 }}
          >
            {expandingGraph ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Zap size={12} />}
          </button>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', padding: 2 }}>
            <X size={14} />
          </button>
        </div>
      </div>

      <div style={{ padding: '10px 16px' }}>
        {loading && (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '24px 0', gap: 8, color: '#475569', fontSize: 12 }}>
            <Loader2 size={14} style={{ animation: 'spin 1s linear infinite' }} />Loading recommendations…
          </div>
        )}

        {!loading && targets.length === 0 && (
          <div style={{ fontSize: 11, color: '#475569', textAlign: 'center', padding: '20px 0', lineHeight: 1.6 }}>
            No growth targets found.<br />
            <span style={{ color: '#334155' }}>Parse a resume and sync skills to seed the graph, then infer dependencies.</span>
          </div>
        )}

        {!loading && reachable.length > 0 && (
          <div style={{ marginBottom: 14 }}>
            <div style={{ fontSize: 9, fontWeight: 600, color: '#78350f', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
              Reachable from your seeds
            </div>
            {reachable.map(t => (
              <GrowthTargetRow key={t.skillId} target={t} maxScore={maxScore} onSelectSkill={onSelectSkill} />
            ))}
          </div>
        )}

        {!loading && disconnected.length > 0 && (
          <div>
            <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
              High-demand (no path yet)
            </div>
            {disconnected.map(t => (
              <GrowthTargetRow key={t.skillId} target={t} maxScore={maxScore} onSelectSkill={onSelectSkill} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Learning Path Panel ──────────────────────────────────────────────────────

const SEASONS = ['Fall 2024', 'Winter 2025', 'Spring 2025', 'Summer 2025', 'Fall 2025', 'Winter 2026', 'Spring 2026', 'Summer 2026'];

interface SkillInlineContext {
  prereqsDone: Array<{ skillId: string; name: string }>;
  unlocks: Array<{ skillId: string; name: string; jobDemandCount: number }>;
  related: Array<{ skillId: string; name: string }>;
  resources: Array<{ title: string; url: string }>;
}

function LearningStepRow({
  step, onSelectSkill, getInline,
}: {
  step: LearningStep;
  onSelectSkill?: (name: string) => void;
  getInline: (skillId: string) => Promise<SkillInlineContext>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [inline, setInline] = useState<SkillInlineContext | null>(null);
  const [inlineLoading, setInlineLoading] = useState(false);
  const [inlineError, setInlineError] = useState<string | null>(null);
  const topJobs = step.jobsNeedingThis.slice(0, 3);
  const extraJobs = step.jobsNeedingThis.length - topJobs.length;

  async function handleToggle() {
    if (expanded) { setExpanded(false); return; }
    setExpanded(true);
    if (inline !== null) return;
    setInlineLoading(true);
    setInlineError(null);
    try {
      const ctx = await getInline(step.skillId);
      setInline(ctx);
    } catch (e) {
      setInlineError(typeof e === 'string' ? e : 'Failed to load context');
    } finally {
      setInlineLoading(false);
    }
  }

  const stateColor = step.skillState === 'seed' ? '#f59e0b'
    : step.skillState === 'gap' ? '#ef4444'
    : '#64748b';

  return (
    <div style={{
      marginBottom: 8,
      borderRadius: 8,
      background: 'rgba(255,255,255,0.025)',
      border: '1px solid rgba(255,255,255,0.06)',
      overflow: 'hidden',
    }}>
      {/* Header row */}
      <div
        style={{ padding: '8px 10px', cursor: 'pointer', display: 'flex', alignItems: 'flex-start', gap: 8 }}
        onClick={handleToggle}
      >
        {/* Step number */}
        <div style={{
          width: 22, height: 22, borderRadius: '50%', flexShrink: 0,
          background: 'rgba(99,102,241,0.15)', border: '1px solid rgba(99,102,241,0.25)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: 9, fontWeight: 700, color: '#818cf8',
        }}>
          {step.step}
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
            <span
              style={{ fontSize: 12, fontWeight: 600, color: '#f1f5f9', cursor: onSelectSkill ? 'pointer' : 'default' }}
              onClick={e => { if (onSelectSkill) { e.stopPropagation(); onSelectSkill(step.skillName); } }}
            >
              {step.skillName}
            </span>
            <span style={{
              fontSize: 8, padding: '1px 4px', borderRadius: 3,
              background: step.skillState === 'gap' ? 'rgba(239,68,68,0.12)' : 'rgba(255,255,255,0.05)',
              color: stateColor, border: `1px solid ${stateColor}33`,
            }}>
              {step.skillState}
            </span>
            {step.hasResources && (
              <BookOpen size={10} style={{ color: '#34d399', flexShrink: 0 }} />
            )}
          </div>

          {/* Job badges */}
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 3, marginBottom: 4 }}>
            {topJobs.map((j, i) => (
              <span key={i} style={{
                fontSize: 8, padding: '1px 5px', borderRadius: 3,
                background: 'rgba(99,102,241,0.08)', color: '#818cf8',
                border: '1px solid rgba(99,102,241,0.15)',
                whiteSpace: 'nowrap', maxWidth: 160, overflow: 'hidden', textOverflow: 'ellipsis',
              }} title={j}>
                {j}
              </span>
            ))}
            {extraJobs > 0 && (
              <span style={{ fontSize: 8, color: '#334155' }}>+{extraJobs} more</span>
            )}
          </div>

          {/* Prereq path breadcrumb */}
          {step.prereqPath.length > 0 && (
            <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 2, marginBottom: 4 }}>
              {step.prereqPath.map((node, i) => (
                <PathNodeChip key={node.skillId || i} node={node} isLast={i === step.prereqPath.length - 1} onSelect={onSelectSkill} />
              ))}
            </div>
          )}
        </div>

        {/* Resource indicator */}
        <div style={{
          width: 6, height: 6, borderRadius: '50%', flexShrink: 0, marginTop: 4,
          background: step.hasResources ? '#34d399' : '#1e3a2a',
          border: `1px solid ${step.hasResources ? '#34d399' : '#1e293b'}`,
        }} title={step.hasResources ? 'Resources available' : 'No resources yet'} />

        {/* Expand chevron */}
        <div style={{ flexShrink: 0, marginTop: 2, color: '#475569' }}>
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </div>
      </div>

      {/* Expanded — rationale + inline context */}
      {expanded && (
        <div style={{ padding: '0 10px 10px 40px' }}>
          {step.rationale && (
            <div style={{ fontSize: 9, color: '#475569', lineHeight: 1.5, marginBottom: 8 }}>{step.rationale}</div>
          )}

          {inlineLoading && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 10, color: '#475569' }}>
              <Loader2 size={10} className="animate-spin" />Loading context…
            </div>
          )}

          {inlineError && (
            <div style={{ fontSize: 10, color: '#fca5a5' }}>{inlineError}</div>
          )}

          {!inlineLoading && !inlineError && inline && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              <div style={{ fontSize: 10, lineHeight: 1.5 }}>
                <span style={{ color: '#34d399', fontWeight: 600 }}>✓ Prereqs done</span>{' '}
                <span style={{ color: '#94a3b8' }}>
                  {inline.prereqsDone.length === 0
                    ? <span style={{ color: '#475569', fontStyle: 'italic' }}>none yet</span>
                    : inline.prereqsDone.map(p => p.name).join(', ')}
                </span>
              </div>

              <div style={{ fontSize: 10, lineHeight: 1.5 }}>
                <span style={{ color: '#818cf8', fontWeight: 600 }}>→ Unlocks</span>{' '}
                {inline.unlocks.length === 0 ? (
                  <span style={{ color: '#475569', fontStyle: 'italic' }}>none yet</span>
                ) : (
                  <span style={{ color: '#94a3b8' }}>
                    {inline.unlocks.map((u, i) => (
                      <span key={u.skillId}>
                        {i > 0 && ', '}
                        {u.name}
                        {u.jobDemandCount > 0 && (
                          <span style={{
                            marginLeft: 4, fontSize: 8, padding: '0 4px', borderRadius: 3,
                            background: 'rgba(99,102,241,0.08)', color: '#6366f1',
                            border: '1px solid rgba(99,102,241,0.14)',
                          }}>
                            {u.jobDemandCount}
                          </span>
                        )}
                      </span>
                    ))}
                  </span>
                )}
              </div>

              <div style={{ fontSize: 10, lineHeight: 1.5 }}>
                <span style={{ color: '#fbbf24', fontWeight: 600 }}>~ Related</span>{' '}
                <span style={{ color: '#94a3b8' }}>
                  {inline.related.length === 0
                    ? <span style={{ color: '#475569', fontStyle: 'italic' }}>none found</span>
                    : inline.related.map(r => r.name).join(', ')}
                </span>
              </div>

              <div style={{ fontSize: 10, lineHeight: 1.5 }}>
                <span style={{ color: '#34d399', fontWeight: 600 }}>📖 Resources</span>{' '}
                {inline.resources.length === 0 ? (
                  <span style={{ color: '#475569', fontStyle: 'italic' }}>No resources linked yet</span>
                ) : (
                  <span style={{ display: 'inline-flex', flexWrap: 'wrap', gap: 6 }}>
                    {inline.resources.map((res, i) => (
                      <a
                        key={i}
                        href={res.url}
                        target="_blank"
                        rel="noreferrer"
                        onClick={e => e.stopPropagation()}
                        style={{
                          color: '#6ee7b7', textDecoration: 'none',
                          padding: '1px 6px', borderRadius: 4,
                          background: 'rgba(16,185,129,0.08)',
                          border: '1px solid rgba(16,185,129,0.18)',
                          maxWidth: 220, overflow: 'hidden',
                          textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                          display: 'inline-block',
                        }}
                        title={res.url}
                      >
                        {res.title || res.url}
                      </a>
                    ))}
                  </span>
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function LearningPathPanel({
  learningPath, loading, onClose, onSelectSkill, season, onSeasonChange,
}: {
  learningPath: LearningPath | null;
  loading: boolean;
  onClose: () => void;
  onSelectSkill?: (name: string) => void;
  season: string | null;
  onSeasonChange: (s: string | null) => void;
}) {
  // Inline-context cache shared across rows — keyed by skillId
  const inlineCache = useRef<Map<string, SkillInlineContext>>(new Map());
  const getInline = useCallback(async (skillId: string): Promise<SkillInlineContext> => {
    const cached = inlineCache.current.get(skillId);
    if (cached) return cached;
    const ctx = await invoke<SkillInlineContext>('get_skill_inline_context', { skillId });
    inlineCache.current.set(skillId, ctx);
    return ctx;
  }, []);

  return (
    <div
      style={{
        position: 'absolute', top: 16, left: 16, zIndex: 50,
        width: 340, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: '1px solid rgba(99,102,241,0.2)',
        borderRadius: 16,
        overflow: 'hidden',
        display: 'flex', flexDirection: 'column',
        boxShadow: '0 12px 48px rgba(0,0,0,0.85)',
      }}
      onClick={e => e.stopPropagation()}
    >
      {/* Header */}
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
            <Route size={13} style={{ color: '#818cf8' }} />
            <span style={{ fontSize: 13, fontWeight: 600, color: '#e0e7ff' }}>Learning Path</span>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', padding: 2 }}>
            <X size={14} />
          </button>
        </div>

        {/* Season filter */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span style={{ fontSize: 9, color: '#475569', flexShrink: 0 }}>Season:</span>
          <select
            value={season ?? ''}
            onChange={e => onSeasonChange(e.target.value || null)}
            style={{
              flex: 1, fontSize: 10, background: 'rgba(255,255,255,0.05)',
              border: '1px solid rgba(255,255,255,0.08)', borderRadius: 5,
              color: '#94a3b8', padding: '2px 6px', outline: 'none',
            }}
          >
            <option value="">All time</option>
            {SEASONS.map(s => <option key={s} value={s}>{s}</option>)}
          </select>
        </div>

        {/* Summary stats */}
        {learningPath && !loading && (
          <div style={{ display: 'flex', gap: 10, marginTop: 8 }}>
            <div style={{ fontSize: 9, color: '#475569' }}>
              <span style={{ color: '#818cf8', fontWeight: 600 }}>{learningPath.steps.length}</span> skills to learn
            </div>
            <div style={{ fontSize: 9, color: '#475569' }}>
              <span style={{ color: '#f59e0b', fontWeight: 600 }}>{learningPath.targetJobsCount}</span> jobs targeted
            </div>
            <div style={{ fontSize: 9, color: '#475569' }}>
              <span style={{ color: '#34d399', fontWeight: 600 }}>{learningPath.seededSkillsCount}</span> already seeded
            </div>
          </div>
        )}
      </div>

      {/* Body */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '10px 12px' }}>
        {loading && (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '24px 0', gap: 8, color: '#475569', fontSize: 12 }}>
            <Loader2 size={14} style={{ animation: 'spin 1s linear infinite' }} />Computing…
          </div>
        )}

        {!loading && (!learningPath || learningPath.steps.length === 0) && (
          <div style={{ fontSize: 11, color: '#475569', textAlign: 'center', padding: '20px 8px', lineHeight: 1.7 }}>
            No learning path yet.<br />
            <span style={{ fontSize: 10, color: '#334155' }}>
              Add job applications and run Sync Skills to generate your path.
            </span>
          </div>
        )}

        {!loading && learningPath && learningPath.steps.length > 0 && (
          <>
            <div style={{ fontSize: 9, color: '#334155', marginBottom: 8, fontStyle: 'italic' }}>
              Prerequisites always appear before dependents · sorted by job demand weight
            </div>
            {learningPath.steps.map(step => (
              <LearningStepRow key={step.step} step={step} onSelectSkill={onSelectSkill} getInline={getInline} />
            ))}
          </>
        )}
      </div>
    </div>
  );
}

function PathNodeChip({ node, isLast, onSelect }: { node: PathNode; isLast: boolean; onSelect?: (name: string) => void }) {
  const nodeColor = node.state === 'seed' ? '#f59e0b' : isLast ? '#34d399' : '#64748b';
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 3 }}>
      <span
        onClick={() => onSelect?.(node.skillName)}
        style={{
          fontSize: 9, padding: '1px 5px', borderRadius: 4, cursor: onSelect ? 'pointer' : 'default',
          background: node.state === 'seed' ? 'rgba(245,158,11,0.15)' : isLast ? 'rgba(52,211,153,0.12)' : 'rgba(255,255,255,0.05)',
          color: nodeColor,
          border: '1px solid ' + (node.state === 'seed' ? 'rgba(245,158,11,0.3)' : isLast ? 'rgba(52,211,153,0.25)' : 'rgba(255,255,255,0.08)'),
          fontWeight: node.state === 'seed' || isLast ? 600 : 400,
        }}
        title={node.hasResources ? 'Has learning resources' : undefined}
      >
        {node.state === 'seed' && <span style={{ display: 'inline-block', width: 4, height: 4, borderRadius: '50%', background: '#f59e0b', marginRight: 3, verticalAlign: 'middle' }} />}
        {node.skillName}
        {node.hasResources && <span style={{ marginLeft: 2, opacity: 0.7 }}>·</span>}
      </span>
      {!isLast && <span style={{ color: '#334155', fontSize: 9 }}>→</span>}
    </span>
  );
}

function PrereqPathBreadcrumb({ path, onSelectSkill }: { path: PathNode[]; onSelectSkill?: (name: string) => void }) {
  if (path.length === 0) return null;
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 2, marginBottom: 6 }}>
      {path.map((node, i) => (
        <PathNodeChip key={node.skillId} node={node} isLast={i === path.length - 1} onSelect={onSelectSkill} />
      ))}
    </div>
  );
}

// ─── GapPath types (Phase 2 skill graph) ──────────────────────────────────────

interface GapSkillStep {
  skillId: string;
  skillName: string;
  level: number;
  state: string;       // "seed" | "adjacent" | "gap"
  origin: string;
  hasTree: boolean;
  treeNodeId: string | null;
  treeProjectId: string | null;
}

interface GapPath {
  gapSkillId: string;
  gapSkillName: string;
  path: GapSkillStep[];
  estimatedDepth: number;
}

function stateChipStyle(state: string): React.CSSProperties {
  switch (state) {
    case 'seed':
      return { background: 'rgba(16,185,129,0.14)', color: '#6ee7b7', border: '1px solid rgba(16,185,129,0.28)' };
    case 'gap':
      return { background: 'rgba(245,158,11,0.14)', color: '#fcd34d', border: '1px solid rgba(245,158,11,0.28)' };
    case 'adjacent':
    default:
      return { background: 'rgba(148,163,184,0.10)', color: '#94a3b8', border: '1px solid rgba(148,163,184,0.20)' };
  }
}

function GapPathSteps({ steps }: { steps: GapSkillStep[] }) {
  if (steps.length === 0) {
    return (
      <div style={{ fontSize: 10, color: '#64748b', fontStyle: 'italic', padding: '6px 0' }}>
        No prerequisite path found — you may already have the foundations
      </div>
    );
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 6 }}>
      {steps.map((s, idx) => (
        <div
          key={s.skillId}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '4px 6px',
            borderRadius: 5,
            background: idx === steps.length - 1 ? 'rgba(245,158,11,0.06)' : 'rgba(255,255,255,0.025)',
            border: '1px solid ' + (idx === steps.length - 1 ? 'rgba(245,158,11,0.15)' : 'rgba(255,255,255,0.04)'),
          }}
        >
          <span style={{ fontSize: 9, color: '#475569', minWidth: 14, textAlign: 'center' }}>{idx + 1}</span>
          <span style={{ fontSize: 11, color: '#e2e8f0', flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {s.skillName}
          </span>
          <span style={{ fontSize: 8, padding: '1px 5px', borderRadius: 3, background: 'rgba(255,255,255,0.05)', color: '#94a3b8', border: '1px solid rgba(255,255,255,0.08)', flexShrink: 0 }}>
            L{s.level}
          </span>
          <span style={{ fontSize: 8, padding: '1px 5px', borderRadius: 3, ...stateChipStyle(s.state), flexShrink: 0 }}>
            {s.state}
          </span>
          {s.hasTree && s.treeNodeId && (
            <button
              onClick={() => {
                if (s.treeProjectId) {
                  window.location.href = `/project/${s.treeProjectId}?node=${s.treeNodeId}`;
                }
              }}
              title="Open tree node"
              style={{ background: 'none', border: 'none', cursor: 'pointer', fontSize: 11, padding: 0, lineHeight: 1, flexShrink: 0 }}
            >
              🌿
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

function GrowthTargetRow({ target, maxScore, onSelectSkill }: { target: GrowthTarget; maxScore: number; onSelectSkill?: (name: string) => void }) {
  const scorePct = Math.round((target.finalScore / maxScore) * 100);
  const [expanded, setExpanded] = useState(false);
  const [pathLoading, setPathLoading] = useState(false);
  const [gapPath, setGapPath] = useState<GapPath | null>(null);
  const [pathError, setPathError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  async function handleOpenTree() {
    if (opening) return;
    setOpening(true);
    setOpenError(null);
    const result = await openSkillTree(target.skillId);
    if (!result.ok) setOpenError(result.error);
    setOpening(false);
  }

  async function handleTogglePath() {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (gapPath !== null) return;
    setPathLoading(true);
    setPathError(null);
    try {
      const result = await invoke<GapPath>('get_gap_path', { skillId: target.skillId });
      setGapPath(result);
    } catch (e) {
      setPathError(typeof e === 'string' ? e : 'Failed to load path');
    } finally {
      setPathLoading(false);
    }
  }

  return (
    <div style={{ marginBottom: 10, padding: '8px 10px', borderRadius: 8, background: target.isReachable ? 'rgba(245,158,11,0.06)' : 'rgba(255,255,255,0.03)', border: '1px solid ' + (target.isReachable ? 'rgba(245,158,11,0.15)' : 'rgba(255,255,255,0.06)') }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 8, marginBottom: 4 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: target.isReachable ? '#fef3c7' : '#94a3b8', flex: 1 }}>
          {target.skillName}
        </div>
        <button
          onClick={e => { e.stopPropagation(); handleOpenTree(); }}
          disabled={opening}
          style={{
            fontSize: 9, padding: '2px 7px', borderRadius: 4,
            background: opening ? 'rgba(255,255,255,0.05)' : 'rgba(16,185,129,0.12)',
            color: opening ? '#475569' : '#34d399',
            border: '1px solid ' + (opening ? 'rgba(255,255,255,0.08)' : 'rgba(16,185,129,0.2)'),
            cursor: opening ? 'default' : 'pointer',
            flexShrink: 0, whiteSpace: 'nowrap',
            display: 'inline-flex', alignItems: 'center', gap: 4,
          }}
          title="Open existing tree or generate a new graph-seeded tree"
        >
          {opening
            ? <><Loader2 size={9} style={{ animation: 'spin 1s linear infinite' }} />Opening…</>
            : <>Open tree →</>}
        </button>
      </div>

      {openError && (
        <div style={{ fontSize: 9, color: '#fca5a5', marginBottom: 4 }}>{openError}</div>
      )}

      {target.prereqPath.length > 0 && (
        <PrereqPathBreadcrumb path={target.prereqPath} onSelectSkill={onSelectSkill} />
      )}

      {!target.isReachable && (
        <div style={{ fontSize: 9, color: '#475569', marginBottom: 5, fontStyle: 'italic' }}>
          No path from your seeds — run Infer Dependencies first
        </div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <div style={{ flex: 1, height: 3, borderRadius: 2, background: 'rgba(255,255,255,0.06)', overflow: 'hidden' }}>
          <div style={{ width: `${scorePct}%`, height: '100%', borderRadius: 2, background: target.isReachable ? '#f59e0b' : '#475569', transition: 'width 0.4s ease' }} />
        </div>
        <span style={{ fontSize: 9, color: '#334155', flexShrink: 0 }}>{target.jobCount} job{target.jobCount !== 1 ? 's' : ''}</span>
      </div>

      {target.rationale && (
        <div style={{ fontSize: 9, color: '#334155', marginTop: 4 }}>{target.rationale}</div>
      )}

      <div style={{ marginTop: 6 }}>
        <button
          onClick={handleTogglePath}
          disabled={pathLoading}
          style={{
            fontSize: 9, padding: '3px 7px', borderRadius: 4,
            background: 'rgba(99,102,241,0.10)', color: '#a5b4fc',
            border: '1px solid rgba(99,102,241,0.22)',
            cursor: pathLoading ? 'default' : 'pointer',
            display: 'inline-flex', alignItems: 'center', gap: 4,
          }}
        >
          {pathLoading ? (
            <><Loader2 size={9} style={{ animation: 'spin 1s linear infinite' }} />Loading…</>
          ) : (
            <>{expanded ? 'Hide path' : 'Show path →'}</>
          )}
        </button>
      </div>

      {expanded && !pathLoading && pathError && (
        <div style={{ fontSize: 10, color: '#fca5a5', marginTop: 6 }}>{pathError}</div>
      )}

      {expanded && !pathLoading && !pathError && gapPath && (
        <GapPathSteps steps={gapPath.path} />
      )}
    </div>
  );
}

// ─── Easing ───────────────────────────────────────────────────────────────────

// ─── Open-tree-for-skill (shared by GrowthPlanPanel + SkillDetailPanel) ──────
// Returns { ok: true } after triggering navigation. Returns { ok: false, error }
// if anything failed before navigation could start.
const SKILLS_PROJECT_ID = '00000000-0000-0000-0000-000000000001';

async function openSkillTree(skillId: string): Promise<{ ok: true } | { ok: false; error: string }> {
  try {
    const existing = await invoke<{ treeId: string; treeTitle: string } | null>(
      'get_tree_for_skill',
      { skillId },
    );
    if (existing) {
      window.location.href = `/trees?selected=${encodeURIComponent(existing.treeId)}`;
      return { ok: true };
    }
    const skillContext = await invoke<string>('get_skill_graph_context', { skillId });
    const treeJson = await invoke<string>('generate_skill_tree', {
      projectId: SKILLS_PROJECT_ID,
      prdText: '',
      skillContext,
      skillId,
    });
    let newTreeId: string | null = null;
    try {
      const parsed = JSON.parse(treeJson);
      if (parsed && typeof parsed.tree_id === 'string') newTreeId = parsed.tree_id;
    } catch { /* fall through to error below */ }
    if (!newTreeId) {
      return { ok: false, error: 'Tree generated but no id returned' };
    }
    window.location.href = `/trees?selected=${encodeURIComponent(newTreeId)}`;
    return { ok: true };
  } catch (e) {
    return { ok: false, error: typeof e === 'string' ? e : 'Failed to open tree' };
  }
}

function easeOutCubic(t: number) {
  return 1 - Math.pow(1 - t, 3);
}

// ─── GraphAuditPanel ──────────────────────────────────────────────────────────

const PROPOSAL_TYPE_LABELS: Record<string, string> = {
  merge_skills: 'Merge',
  delete_skill: 'Delete',
  rename_skill: 'Rename',
};
const PROPOSAL_TYPE_COLORS: Record<string, string> = {
  merge_skills: '#818cf8',
  delete_skill: '#f87171',
  rename_skill: '#f59e0b',
};

function ProposalRow({
  proposal,
  onApprove,
  onReject,
  busy,
}: {
  proposal: Proposal;
  onApprove: () => void;
  onReject: () => void;
  busy: boolean;
}) {
  const color = PROPOSAL_TYPE_COLORS[proposal.type] ?? '#94a3b8';
  const label = PROPOSAL_TYPE_LABELS[proposal.type] ?? proposal.type;

  // Human-readable description of the action
  let description = '';
  if (proposal.type === 'merge_skills') {
    const from = proposal.payload.merge_name || proposal.payload.merge_id;
    const to = proposal.payload.keep_name || proposal.payload.keep_id;
    description = `"${from}" → "${to}"`;
  } else if (proposal.type === 'delete_skill') {
    const name = proposal.payload.skill_name || proposal.payload.skill_id;
    description = `"${name}"`;
  } else if (proposal.type === 'rename_skill') {
    const name = proposal.payload.skill_name || proposal.payload.skill_id;
    description = `"${name}" → "${proposal.payload.new_name}"`;
  }

  return (
    <div style={{
      padding: '10px 14px',
      borderBottom: '1px solid rgba(255,255,255,0.04)',
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <span style={{
          fontSize: 9, fontWeight: 700, padding: '2px 6px', borderRadius: 4,
          background: color + '1a', color, border: `1px solid ${color}33`,
          textTransform: 'uppercase', letterSpacing: '0.06em', flexShrink: 0,
        }}>
          {label}
        </span>
        <span style={{ fontSize: 10, color: '#94a3b8', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {description}
        </span>
      </div>
      <p style={{ fontSize: 10, color: '#475569', margin: 0, lineHeight: 1.5 }}>
        {proposal.llmReasoning}
      </p>
      <div style={{ display: 'flex', gap: 6 }}>
        <button
          onClick={onApprove}
          disabled={busy}
          style={{
            display: 'flex', alignItems: 'center', gap: 4, padding: '3px 10px',
            borderRadius: 6, border: '1px solid rgba(52,211,153,0.3)',
            background: busy ? 'rgba(255,255,255,0.04)' : 'rgba(52,211,153,0.12)',
            color: busy ? '#334155' : '#34d399', fontSize: 10, cursor: busy ? 'default' : 'pointer',
          }}
        >
          {busy ? <Loader2 style={{ width: 10, height: 10 }} className="animate-spin" /> : <CheckCircle style={{ width: 10, height: 10 }} />}
          Approve
        </button>
        <button
          onClick={onReject}
          disabled={busy}
          style={{
            display: 'flex', alignItems: 'center', gap: 4, padding: '3px 10px',
            borderRadius: 6, border: '1px solid rgba(248,113,113,0.2)',
            background: 'none', color: busy ? '#334155' : '#f87171',
            fontSize: 10, cursor: busy ? 'default' : 'pointer',
          }}
        >
          <XCircle style={{ width: 10, height: 10 }} />
          Reject
        </button>
      </div>
    </div>
  );
}

function GraphAuditPanel({
  onApprove,
  onReject,
  onClose,
}: {
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  onClose: () => void;
}) {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(false);
  const [running, setRunning] = useState(false);
  const [applyingId, setApplyingId] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  async function fetchProposals() {
    setLoading(true);
    try {
      const p = await invoke<Proposal[]>('get_pending_proposals');
      setProposals(p);
    } catch (e) { console.error('get_pending_proposals failed:', e); }
    finally { setLoading(false); }
  }

  useEffect(() => { fetchProposals(); }, []);

  async function handleRunAudit() {
    setRunning(true);
    setToast('Running graph audit…');
    try {
      const count = await invoke<number>('run_graph_audit');
      setToast(`${count} proposal${count !== 1 ? 's' : ''} generated`);
      setTimeout(() => setToast(null), 5000);
      const p = await invoke<Proposal[]>('get_pending_proposals');
      setProposals(p);
    } catch (e) {
      setToast(`Audit failed: ${e}`);
      setTimeout(() => setToast(null), 6000);
    } finally {
      setRunning(false);
    }
  }

  async function handleApprove(id: string) {
    setApplyingId(id);
    try {
      await invoke('approve_proposal', { proposalId: id });
      setProposals(prev => prev.filter(p => p.id !== id));
      onApprove(id);
    } catch (e) { console.error('approve_proposal failed:', e); }
    finally { setApplyingId(null); }
  }

  async function handleReject(id: string) {
    setApplyingId(id);
    try {
      await invoke('reject_proposal', { proposalId: id });
      setProposals(prev => prev.filter(p => p.id !== id));
      onReject(id);
    } catch (e) { console.error('reject_proposal failed:', e); }
    finally { setApplyingId(null); }
  }

  // Group by type
  const grouped = proposals.reduce<Record<string, Proposal[]>>((acc, p) => {
    (acc[p.type] ??= []).push(p);
    return acc;
  }, {});

  return (
    <div style={{
      position: 'absolute', top: 0, right: 0, bottom: 0,
      width: 380, zIndex: 50,
      background: 'rgba(2,4,16,0.97)',
      borderLeft: '1px solid rgba(139,92,246,0.2)',
      display: 'flex', flexDirection: 'column',
      backdropFilter: 'blur(16px)',
    }}>
      {/* Header */}
      <div style={{ padding: '14px 16px', borderBottom: '1px solid rgba(255,255,255,0.06)', display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <ShieldCheck style={{ width: 14, height: 14, color: '#a78bfa' }} />
          <span style={{ fontSize: 12, fontWeight: 600, color: '#e2e8f0' }}>Graph Audit</span>
          {proposals.length > 0 && (
            <span style={{ fontSize: 9, padding: '1px 6px', borderRadius: 10, background: 'rgba(139,92,246,0.15)', color: '#a78bfa', border: '1px solid rgba(139,92,246,0.25)' }}>
              {proposals.length} pending
            </span>
          )}
        </div>
        <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', padding: 2 }}>
          <X style={{ width: 14, height: 14 }} />
        </button>
      </div>

      {/* Run Audit + Clear All */}
      <div style={{ padding: '10px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)', flexShrink: 0 }}>
        <div style={{ display: 'flex', gap: 6 }}>
          <button
            onClick={handleRunAudit}
            disabled={running}
            style={{
              flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
              padding: '7px 12px', borderRadius: 8,
              border: '1px solid rgba(139,92,246,0.3)',
              background: running ? 'rgba(255,255,255,0.04)' : 'rgba(139,92,246,0.14)',
              color: running ? '#334155' : '#a78bfa',
              fontSize: 11, fontWeight: 600, cursor: running ? 'default' : 'pointer',
            }}
          >
            {running ? <Loader2 style={{ width: 12, height: 12 }} className="animate-spin" /> : <ShieldCheck style={{ width: 12, height: 12 }} />}
            {running ? 'Auditing…' : 'Run Audit'}
          </button>
          {proposals.length > 0 && (
            <button
              onClick={() => {
                if (window.confirm(`Clear all ${proposals.length} pending proposal${proposals.length !== 1 ? 's' : ''}? This cannot be undone.`)) {
                  invoke('clear_all_proposals').then(() => setProposals([]));
                }
              }}
              style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '7px 10px', borderRadius: 8,
                border: '1px solid rgba(248,113,113,0.2)',
                background: 'none', color: '#f87171',
                fontSize: 11, fontWeight: 600, cursor: 'pointer', flexShrink: 0,
              }}
            >
              <Trash2 style={{ width: 11, height: 11 }} />
              Clear
            </button>
          )}
        </div>
        {toast && (
          <p style={{ fontSize: 10, color: '#a78bfa', textAlign: 'center', marginTop: 6, marginBottom: 0 }}>{toast}</p>
        )}
      </div>

      {/* Proposals list */}
      <div style={{ flex: 1, overflowY: 'auto' }}>
        {loading ? (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 32 }}>
            <Loader2 style={{ width: 18, height: 18, color: '#a78bfa' }} className="animate-spin" />
          </div>
        ) : proposals.length === 0 ? (
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: 40, gap: 10 }}>
            <ShieldCheck style={{ width: 28, height: 28, color: '#1e293b' }} />
            <p style={{ fontSize: 11, color: '#334155', textAlign: 'center', margin: 0 }}>No pending proposals</p>
            <p style={{ fontSize: 10, color: '#1e293b', textAlign: 'center', margin: 0 }}>Run an audit to detect issues</p>
          </div>
        ) : (
          <>
            {Object.entries(grouped).map(([type, typeProposals]) => (
              <div key={type}>
                <div style={{ padding: '8px 14px 4px', fontSize: 9, fontWeight: 700, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                  {PROPOSAL_TYPE_LABELS[type] ?? type} · {typeProposals.length}
                </div>
                {typeProposals.map(p => (
                  <ProposalRow
                    key={p.id}
                    proposal={p}
                    onApprove={() => handleApprove(p.id)}
                    onReject={() => handleReject(p.id)}
                    busy={applyingId === p.id}
                  />
                ))}
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

export default function SkillsPage() {
  const canvasRef    = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const [skills, setSkills]     = useState<UniversalSkill[]>([]);
  const [gaps, setGaps]         = useState<SkillGap[]>([]);
  const [deps, setDeps]         = useState<SkillDependency[]>([]);
  const [aliases, setAliases]   = useState<SkillAlias[]>([]);
  const [loading, setLoading]   = useState(true);
  const [syncing, setSyncing]   = useState(false);
  const [stateFilter, setStateFilter] = useState<'all' | 'seed' | 'adjacent'>('all');
  const [growthTargets, setGrowthTargets] = useState<GrowthTarget[]>([]);
  const [showGrowthPlan, setShowGrowthPlan] = useState(false);
  const [growthLoading, setGrowthLoading] = useState(false);
  const [expandingGraph, setExpandingGraph] = useState(false);
  const [showLearningPath, setShowLearningPath] = useState(false);
  const [learningPath, setLearningPath] = useState<LearningPath | null>(null);
  const [learningPathLoading, setLearningPathLoading] = useState(false);
  const [learningPathSeason, setLearningPathSeason] = useState<string | null>(null);
  const [syncToast, setSyncToast] = useState<string | null>(null);
  const [showOverflowMenu, setShowOverflowMenu] = useState(false);
  const [showAuditPanel, setShowAuditPanel] = useState(false);
  const [inferring, setInferring] = useState(false);
  const [classifying, setClassifying] = useState(false);
  const [resetting, setResetting]     = useState(false);
  const [classifyToast, setClassifyToast] = useState<string | null>(null);
  const [backfillToast, setBackfillToast] = useState<string | null>(null);
  const [syncingJobs, setSyncingJobs] = useState(false);
  const [autoMergeSuggestions, setAutoMergeSuggestions] = useState<SuggestedMerge[] | null>(null);
  const [autoMerging, setAutoMerging] = useState(false);
  const [showGaps, setShowGaps]       = useState(false);   // default hidden
  const [showReviewOnly, setShowReviewOnly] = useState(false);
  const [showMerge, setShowMerge]     = useState(false);
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedId, setSelectedId]   = useState<string | null>(null);
  const [hoveredId, setHoveredId]     = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [size, setSize]               = useState({ w: 1200, h: 900 });

  // Branch-growth animation
  const growStartRef  = useRef<number | null>(null);
  const growFracRef   = useRef(0);
  const GROW_DURATION = 600; // ms

  // Double-click zoom animation
  const animStartRef   = useRef<number | null>(null);
  const animFromRef    = useRef({ x: 0, y: 0, scale: 1 });
  const animToRef      = useRef({ x: 0, y: 0, scale: 1 });
  const animatingRef   = useRef(false);

  // Zoom percentage label — updated from rAF loop to avoid per-frame React renders
  const zoomLabelRef   = useRef<HTMLSpanElement>(null);
  const ZOOM_MIN = 0.15;
  const ZOOM_MAX = 6.0;

  const transform    = useRef({ x: 0, y: 0, scale: 1 });
  const isDragging   = useRef(false);
  const lastMouse    = useRef({ x: 0, y: 0 });
  const dragMoved    = useRef(false);
  const [renderTick, setRenderTick] = useState(0);
  const triggerRender = useCallback(() => setRenderTick(t => t + 1), []);

  const { placements, lanes, bandHeight, maxDepth } = useMemo(
    () => layoutSkillGraph(skills, gaps, showGaps, deps, size.w, size.h),
    [skills, gaps, showGaps, deps, size.w, size.h],
  );

  // Reset grow animation whenever layout changes
  useEffect(() => {
    growStartRef.current = null;
    growFracRef.current  = 0;
  }, [placements]);

  const domainColors = useMemo(() => {
    const names = [...new Set(skills.map(s => s.domain || 'General'))];
    return names.map((_, i) => DOMAIN_COLORS[i % DOMAIN_COLORS.length]);
  }, [skills]);

  const seedIds = useMemo(() => new Set(skills.filter(s => s.state === 'seed').map(s => s.id)), [skills]);
  const targetSkillNames = useMemo(() => new Set(growthTargets.map(t => t.skillName.toLowerCase())), [growthTargets]);
  const targetIds = useMemo(() => new Set(skills.filter(s => targetSkillNames.has(s.name.toLowerCase())).map(s => s.id)), [skills, targetSkillNames]);

  const aliasCountMap = useMemo(() => {
    const m = new Map<string, number>();
    for (const a of aliases) m.set(a.canonicalSkillId, (m.get(a.canonicalSkillId) ?? 0) + 1);
    return m;
  }, [aliases]);

  const selectedSkill = useMemo(
    () => selectedId ? skills.find(s => s.id === selectedId) ?? null : null,
    [selectedId, skills],
  );
  const selectedGap = useMemo(() => {
    if (!selectedId) return null;
    const gapName = selectedId.startsWith('gap-') ? selectedId.slice(4) : null;
    return gapName ? gaps.find(g => g.skillName === gapName) ?? null : null;
  }, [selectedId, gaps]);

  const filteredDomainGroups = useMemo(() => {
    let base = showReviewOnly ? skills.filter(s => s.reviewNeeded) : skills;
    if (stateFilter !== 'all') base = base.filter(s => s.state === stateFilter);
    const filtered = searchQuery.trim()
      ? base.filter(s => s.name.toLowerCase().includes(searchQuery.toLowerCase()))
      : base;
    const map = new Map<string, UniversalSkill[]>();
    for (const s of filtered) {
      const d = s.domain || 'General';
      if (!map.has(d)) map.set(d, []);
      map.get(d)!.push(s);
    }
    return map;
  }, [skills, showReviewOnly, stateFilter, searchQuery]);

  const avgLevel     = skills.length > 0
    ? (skills.reduce((sum, s) => sum + s.level, 0) / skills.length).toFixed(1) : '0';
  const levelCounts  = [0, 0, 0, 0, 0, 0];
  for (const s of skills) levelCounts[s.level]++;
  const reviewCount  = skills.filter(s => s.reviewNeeded).length;

  async function loadAll() {
    try {
      const snapshot = await invoke<SkillGraphSnapshot>('get_skill_graph_snapshot');
      const s = validateOrLog(z.array(SkillSchema), snapshot.skills, 'get_skill_graph_snapshot') as UniversalSkill[];
      setSkills(s);
      setGaps(snapshot.gaps);
      setDeps(snapshot.dependencies);
      setAliases(snapshot.aliases);
      setExpandedDomains(new Set(s.map(sk => sk.domain || 'General')));
    } catch (e) {
      console.error('Failed to load skills:', e);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { loadAll(); }, []);
  useEffect(() => {
    const unsub = listen('ygg-skills-updated', () => { loadAll(); });
    return () => { unsub.then(fn => fn()); };
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const { clientWidth, clientHeight } = el;
    if (clientWidth > 0 && clientHeight > 0) setSize({ w: clientWidth, h: clientHeight });
    const ro = new ResizeObserver(entries => {
      const { width, height } = entries[0].contentRect;
      if (width > 0 && height > 0) setSize({ w: Math.round(width), h: Math.round(height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ── Canvas render ─────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width  = size.w * dpr;
    canvas.height = size.h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const now      = performance.now();
    const animTime = now / 1000;

    // Advance grow animation
    if (placements.length > 0) {
      if (growStartRef.current === null) growStartRef.current = now;
      const elapsed = now - growStartRef.current;
      growFracRef.current = Math.min(1, easeOutCubic(elapsed / GROW_DURATION));
    }
    const growFrac = growFracRef.current;

    drawBackground(ctx, size.w, size.h, animTime, domainColors);

    const t = transform.current;
    ctx.save();
    ctx.translate(t.x, t.y);
    ctx.scale(t.scale, t.scale);

    // Compute which skill IDs are "neighbors" of selected/hovered (for dimming)
    const focusId = selectedId ?? hoveredId;
    const neighborIds = new Set<string>();
    if (focusId) {
      neighborIds.add(focusId);
      for (const d of deps) {
        if (d.sourceSkillId === focusId) neighborIds.add(d.targetSkillId);
        if (d.targetSkillId === focusId) neighborIds.add(d.sourceSkillId);
      }
    }

    // Lane separators and depth guides (world-space)
    drawLaneSeparators(ctx, lanes, size.h);
    drawDepthGuides(ctx, maxDepth, bandHeight, 60, size.w, size.h);

    // Dep edges behind nodes (visible only for selected or hovered skill)
    drawDepEdges(ctx, placements, selectedId, hoveredId, deps);

    // Draw nodes
    const hoveredPlacement = hoveredId ? placements.find(p =>
      (p.skill && p.skill.id === hoveredId) ||
      (p.gap   && 'gap-' + p.gap.skillName === hoveredId),
    ) ?? null : null;

    placements.forEach(p => {
      const pid        = p.skill ? p.skill.id : 'gap-' + (p.gap?.skillName ?? '');
      const isSelected = pid === selectedId;
      const isHovered  = pid === hoveredId;

      let nodeAlpha = 1.0;
      if (focusId) {
        if (pid === focusId) nodeAlpha = 1.0;
        else if (p.skill && neighborIds.has(p.skill.id)) nodeAlpha = 0.90;
        else nodeAlpha = 0.25;
      }

      if (p.skill) {
        const isSeed   = seedIds.has(p.skill.id);
        const isTarget = targetIds.has(p.skill.id);
        drawSkillNode(ctx, p.x, p.y, p.color, p.skill.level, isHovered, isSelected, animTime, nodeAlpha, growFrac, isSeed, isTarget);
      } else {
        drawGapNode(ctx, p.x, p.y, animTime, nodeAlpha);
      }
    });

    // Zoom-adaptive labels
    const scale = t.scale;
    placements.forEach(p => {
      if (!p.skill) return;
      const pid = p.skill.id;
      if (pid === hoveredId) return; // tooltip handles it

      const isFocused   = pid === selectedId;
      const isNeighbor  = focusId ? neighborIds.has(pid) : false;
      const level       = p.skill.level;
      const isSeed      = seedIds.has(pid);

      let showLabel = false;
      if (isFocused) showLabel = true;
      else if (isNeighbor) showLabel = true;
      else if (scale >= 1.5) showLabel = true;
      else if (scale >= 1.2 && level >= 2) showLabel = true;
      else if (scale < 1.2 && (isSeed || level >= 3)) showLabel = true;

      if (!showLabel) return;

      let labelAlpha = 1.0;
      if (focusId && !isFocused && !isNeighbor) labelAlpha = 0.0;
      if (!focusId && scale < 1.2 && !isSeed && level < 3) labelAlpha = 0.0;

      if (labelAlpha < 0.01) return;

      const label = p.skill.name.length > 20 ? p.skill.name.slice(0, 20) + '…' : p.skill.name;
      ctx.font = isFocused ? '600 10px "DM Sans", system-ui' : '500 9px "DM Sans", system-ui';
      const lx = p.x + 10;
      ctx.globalAlpha = labelAlpha * (isFocused ? 1.0 : focusId ? 0.85 : 0.70);
      ctx.fillStyle = isFocused ? '#f1f5f9' : '#94a3b8';
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      ctx.fillText(label, lx, p.y);
      ctx.globalAlpha = 1;
    });

    // Lane headers (world-space, above nodes)
    drawLaneHeaders(ctx, lanes, growFrac);

    // Hover tooltip (on top of everything, inside transform)
    if (hoveredPlacement) {
      drawHoverLabel(ctx, hoveredPlacement, size.w / 2);
    }

    ctx.restore();

    // Screen-space legend — OUTSIDE save/restore so transform doesn't affect it
    drawLegend(ctx, size.w, size.h);

  }, [placements, lanes, bandHeight, maxDepth, selectedId, hoveredId,
      size, renderTick, domainColors, skills, deps, seedIds, targetIds]);

  // ── RAF loop ──────────────────────────────────────────────────────────────
  useEffect(() => {
    if (skills.length === 0 && gaps.length === 0) return;
    let rafId: number;
    const tick = () => {
      if (animatingRef.current) {
        const now2 = performance.now();
        if (animStartRef.current === null) animStartRef.current = now2;
        const t2 = Math.min(1, easeOutCubic((now2 - animStartRef.current) / 350));
        const from = animFromRef.current, to = animToRef.current;
        transform.current.x     = from.x + (to.x - from.x) * t2;
        transform.current.y     = from.y + (to.y - from.y) * t2;
        transform.current.scale = from.scale + (to.scale - from.scale) * t2;
        if (t2 >= 1) animatingRef.current = false;
      }
      triggerRender();
      if (zoomLabelRef.current) {
        zoomLabelRef.current.textContent = Math.round(transform.current.scale * 100) + '%';
      }
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [skills.length, gaps.length, triggerRender]);

  // ── Wheel zoom ────────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const t = transform.current;
      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left, my = e.clientY - rect.top;
      const delta    = e.deltaY > 0 ? 0.92 : 1.08;
      const newScale = Math.max(0.15, Math.min(6, t.scale * delta));
      t.x = mx - (mx - t.x) * (newScale / t.scale);
      t.y = my - (my - t.y) * (newScale / t.scale);
      t.scale = newScale;
      triggerRender();
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => canvas.removeEventListener('wheel', onWheel);
  }, [triggerRender]);

  function screenToWorld(sx: number, sy: number) {
    const t = transform.current;
    return { x: (sx - t.x) / t.scale, y: (sy - t.y) / t.scale };
  }

  function findPlacement(wx: number, wy: number): SkillPlacement | null {
    const baseHit = 16;
    let closest: SkillPlacement | null = null;
    let closestDist = Infinity;
    for (const p of placements) {
      const r   = p.skill ? NODE_R[Math.max(1, Math.min(5, p.skill.level))] + baseHit : GAP_NODE_R + 8;
      const dist = Math.hypot(p.x - wx, p.y - wy);
      if (dist < r && dist < closestDist) { closest = p; closestDist = dist; }
    }
    return closest;
  }

  function getPlacementId(p: SkillPlacement): string {
    return p.skill ? p.skill.id : 'gap-' + (p.gap?.skillName ?? '');
  }

  function handleCanvasClick(e: React.MouseEvent) {
    if (dragMoved.current) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect  = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const p     = findPlacement(world.x, world.y);
    setSelectedId(p ? getPlacementId(p) : null);
  }

  function handleMouseDown(e: React.MouseEvent) {
    isDragging.current  = true;
    dragMoved.current   = false;
    lastMouse.current   = { x: e.clientX, y: e.clientY };
  }

  function handleMouseMove(e: React.MouseEvent) {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (isDragging.current) {
      const dx = e.clientX - lastMouse.current.x, dy = e.clientY - lastMouse.current.y;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) dragMoved.current = true;
      transform.current.x += dx;
      transform.current.y += dy;
      lastMouse.current = { x: e.clientX, y: e.clientY };
      triggerRender();
      return;
    }
    const rect  = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const p     = findPlacement(world.x, world.y);
    const newId = p ? getPlacementId(p) : null;
    if (newId !== hoveredId) setHoveredId(newId);
  }

  function handleMouseUp() { isDragging.current = false; }

  function handleCanvasDblClick(e: React.MouseEvent) {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect  = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const p     = findPlacement(world.x, world.y);
    if (!p) return;

    const focusSkillId = p.skill?.id ?? null;
    const neighborPlacements: SkillPlacement[] = [p];
    if (focusSkillId) {
      for (const d of deps) {
        if (d.sourceSkillId !== focusSkillId && d.targetSkillId !== focusSkillId) continue;
        const otherId = d.sourceSkillId === focusSkillId ? d.targetSkillId : d.sourceSkillId;
        const op = placements.find(pl => pl.skill?.id === otherId);
        if (op) neighborPlacements.push(op);
      }
    }

    const xs = neighborPlacements.map(n => n.x);
    const ys = neighborPlacements.map(n => n.y);
    const minX = Math.min(...xs) - 80, maxX = Math.max(...xs) + 80;
    const minY = Math.min(...ys) - 80, maxY = Math.max(...ys) + 80;
    const bbW = maxX - minX, bbH = maxY - minY;

    const newScale0 = Math.min(Math.min(size.w / bbW, size.h / bbH) * 0.80, 3.0);
    const newScale  = Math.max(newScale0, 0.5);
    const bbCx = (minX + maxX) / 2, bbCy = (minY + maxY) / 2;
    const newX = size.w / 2 - bbCx * newScale;
    const newY = size.h / 2 - bbCy * newScale;

    animFromRef.current  = { ...transform.current };
    animToRef.current    = { x: newX, y: newY, scale: newScale };
    animStartRef.current = null;
    animatingRef.current = true;
  }

  function zoomByFactor(factor: number) {
    const t = transform.current;
    const newScale = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, t.scale * factor));
    // Zoom about canvas center so the user's framing stays put.
    const cx = size.w / 2, cy = size.h / 2;
    t.x = cx - (cx - t.x) * (newScale / t.scale);
    t.y = cy - (cy - t.y) * (newScale / t.scale);
    t.scale = newScale;
    triggerRender();
  }

  function handleFitAll() {
    if (placements.length === 0) return;
    const xs = placements.map(p => p.x), ys = placements.map(p => p.y);
    const minX = Math.min(...xs) - 40, maxX = Math.max(...xs) + 40;
    const minY = Math.min(...ys) - 40, maxY = Math.max(...ys) + 40;
    const bbW = maxX - minX, bbH = maxY - minY;
    const newScale = Math.min(size.w / bbW, size.h / bbH, 2.0);
    const bbCx = (minX + maxX) / 2, bbCy = (minY + maxY) / 2;
    animFromRef.current  = { ...transform.current };
    animToRef.current    = { x: size.w / 2 - bbCx * newScale, y: size.h / 2 - bbCy * newScale, scale: newScale };
    animStartRef.current = null;
    animatingRef.current = true;
    triggerRender();
  }

  function handleExport() {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const url = canvas.toDataURL('image/png');
    const a   = document.createElement('a');
    a.href = url; a.download = 'skill-tree.png'; a.click();
  }

  async function handleSync() {
    setSyncing(true);
    setSyncToast('Syncing skills…');
    try {
      await invoke('sync_all_skills');
      await loadAll();
    } catch (e) {
      console.error('Sync failed:', e);
      setSyncToast(`Sync failed: ${String(e)}`);
      setTimeout(() => setSyncToast(null), 5000);
      setSyncing(false);
      return;
    } finally {
      setSyncing(false);
    }

    // Chain embed → infer → classify → reload as a background pipeline.
    // Embed must run BEFORE infer: the ANN inferencer (infer_skill_deps_ann)
    // needs `universal_skills.embedding` populated to compute neighborhoods.
    // Classify can produce new domain names that influence embed text, so it
    // tail-calls `backfill_skill_embeddings` to re-embed any newly classified
    // skills automatically — no extra step needed here.
    //
    // Errors at each stage are surfaced in the toast but don't abort downstream.
    (async () => {
      setSyncToast('Embedding skills…');
      try {
        await invoke('backfill_skill_embeddings_cmd');
      } catch (e) {
        console.error('Embed failed:', e);
      }

      setSyncToast('Inferring dependencies…');
      try {
        await invoke('infer_skill_dependencies');
      } catch (e) {
        console.error('Infer deps failed:', e);
      }

      setSyncToast('Classifying domains…');
      try {
        const result = await invoke<{ total: number; classified: number; failed: number }>('classify_skill_domains');
        setSyncToast(`Done — classified ${result.classified}/${result.total}`);
      } catch (e) {
        console.error('Classify domains failed:', e);
        setSyncToast(`Done (classify error: ${String(e)})`);
      }

      await loadAll();
      setTimeout(() => setSyncToast(null), 4000);
    })();
  }

  async function handleAutoMergePreview() {
    try {
      const suggestions = await invoke<SuggestedMerge[]>('suggest_skill_merges');
      setAutoMergeSuggestions(suggestions);
    } catch (e) {
      console.error('suggest_skill_merges failed:', e);
    }
  }

  async function handleAutoMergeConfirm() {
    setAutoMerging(true);
    try {
      const count = await invoke<number>('auto_merge_suggested');
      setAutoMergeSuggestions(null);
      // Refresh slugs and reload after merge
      await invoke('backfill_concept_slugs');
      await loadAll();
      setBackfillToast(`Auto-merged ${count} duplicate group${count !== 1 ? 's' : ''}`);
      setTimeout(() => setBackfillToast(null), 4000);
    } catch (e) {
      console.error('auto_merge_suggested failed:', e);
    } finally {
      setAutoMerging(false);
    }
  }

  async function handleSyncJobs() {
    setSyncingJobs(true);
    setSyncToast('Syncing job skills…');
    try {
      await invoke('sync_skills_from_jobs');
      await loadAll();
    } catch (e) {
      console.error('Job skill sync failed:', e);
      setSyncToast(`Job sync failed: ${String(e)}`);
      setTimeout(() => setSyncToast(null), 5000);
      setSyncingJobs(false);
      return;
    } finally {
      setSyncingJobs(false);
    }

    // Mirror handleSync's pipeline so job-derived skills get the same
    // embed → infer → classify treatment. Without the Embed step the ANN
    // inferencer cannot see any of the newly-imported job_gap skills.
    (async () => {
      setSyncToast('Embedding skills…');
      try {
        await invoke('backfill_skill_embeddings_cmd');
      } catch (e) {
        console.error('Embed failed:', e);
      }

      setSyncToast('Inferring dependencies…');
      try {
        await invoke('infer_skill_dependencies');
      } catch (e) {
        console.error('Infer deps failed:', e);
      }

      setSyncToast('Classifying domains…');
      try {
        const result = await invoke<{ total: number; classified: number; failed: number }>('classify_skill_domains');
        setSyncToast(`Done — classified ${result.classified}/${result.total}`);
      } catch (e) {
        console.error('Classify domains failed:', e);
        setSyncToast(`Done (classify error: ${String(e)})`);
      }

      await loadAll();
      setTimeout(() => setSyncToast(null), 4000);
    })();
  }

  async function handleInferDeps() {
    setInferring(true);
    try { await invoke('infer_skill_dependencies'); await loadAll(); }
    catch (e) { console.error('Infer deps failed:', e); }
    finally { setInferring(false); }
  }

  async function handleClassifyDomains() {
    setClassifying(true); setClassifyToast(null);
    try {
      const result = await invoke<{ total: number; classified: number; failed: number }>('classify_skill_domains');
      setClassifyToast(`Classified ${result.classified}/${result.total} skills${result.failed > 0 ? ` (${result.failed} failed)` : ''}`);
      await loadAll();
      setTimeout(() => setClassifyToast(null), 4000);
    } catch (e) {
      setClassifyToast(`Error: ${String(e)}`);
      setTimeout(() => setClassifyToast(null), 5000);
    } finally { setClassifying(false); }
  }

  async function handleResetDomains() {
    setResetting(true); setClassifyToast(null);
    try {
      const count = await invoke<number>('reset_skill_domains');
      setClassifyToast(`Reset ${count} skills to unclassified`);
      await loadAll();
      setTimeout(() => setClassifyToast(null), 3000);
    } catch (e) {
      setClassifyToast(`Error: ${String(e)}`);
      setTimeout(() => setClassifyToast(null), 5000);
    } finally { setResetting(false); }
  }

  async function handleMarkReviewed(skillId: string, e: React.MouseEvent) {
    e.stopPropagation();
    try {
      await invoke('mark_skill_reviewed', { skillId });
      setSkills(prev => prev.map(s => s.id === skillId ? { ...s, reviewNeeded: false, status: 'active' } : s));
    } catch (err) { console.error('mark_skill_reviewed failed:', err); }
  }

  async function handleOpenGrowthPlan() {
    setShowGrowthPlan(true);
    if (growthTargets.length > 0) return;
    setGrowthLoading(true);
    try {
      const targets = await invoke<GrowthTarget[]>('get_growth_recommendations', { season: null });
      setGrowthTargets(targets);
    } catch (e) { console.error('get_growth_recommendations failed:', e); }
    finally { setGrowthLoading(false); }
  }

  async function handleOpenLearningPath(season: string | null = learningPathSeason) {
    setShowLearningPath(true);
    setShowGrowthPlan(false);
    setLearningPathLoading(true);
    try {
      const path = await invoke<LearningPath>('compute_learning_path', { season });
      setLearningPath(path);
    } catch (e) { console.error('compute_learning_path failed:', e); }
    finally { setLearningPathLoading(false); }
  }

  async function handleLearningPathSeasonChange(season: string | null) {
    setLearningPathSeason(season);
    handleOpenLearningPath(season);
  }

  async function handleExpandSkillGraph() {
    setExpandingGraph(true);
    try {
      await invoke('expand_skill_graph');
      await loadAll();
    } catch (e) { console.error('expand_skill_graph failed:', e); }
    finally { setExpandingGraph(false); }
  }


  if (loading) {
    return (
      <div className="h-full flex items-center justify-center" style={{ background: '#01020a' }}>
        <Loader2 className="w-6 h-6 text-emerald-400 animate-spin" />
      </div>
    );
  }

  return (
    <div className="flex h-full text-white" style={{ background: '#01020a' }}>
      {/* ── Left Panel ─────────────────────────────────────────────────────────── */}
      <div
        className="flex-shrink-0 flex flex-col overflow-hidden transition-all duration-200"
        style={{
          width: sidebarCollapsed ? 44 : 276,
          background: 'rgba(2,4,16,0.96)',
          borderRight: '1px solid rgba(255,255,255,0.06)',
          backdropFilter: 'blur(12px)',
        }}
      >
        {/* Header */}
        <div style={{ padding: sidebarCollapsed ? '10px 6px' : '14px 14px 10px', borderBottom: '1px solid rgba(255,255,255,0.06)' }}>
          {sidebarCollapsed ? (
            <div className="flex flex-col items-center gap-3">
              <button onClick={() => setSidebarCollapsed(false)} className="p-1.5 text-slate-600 hover:text-slate-300 transition-colors" title="Expand sidebar">
                <PanelLeftOpen className="w-4 h-4" />
              </button>
              <button onClick={handleSync} disabled={syncing} className="p-1.5 text-emerald-700 hover:text-emerald-400 disabled:opacity-30 transition-colors" title="Sync All Skills">
                {syncing ? <Loader2 className="w-4 h-4 animate-spin" /> : <RefreshCw className="w-4 h-4" />}
              </button>
              <button onClick={handleExport} disabled={skills.length === 0} className="p-1.5 text-slate-700 hover:text-slate-400 disabled:opacity-30 transition-colors" title="Export as PNG">
                <Download className="w-4 h-4" />
              </button>
            </div>
          ) : (
            <>
              {/* Header row */}
              <div className="flex items-center justify-between mb-3">
                <h1 className="text-sm font-semibold flex items-center gap-2 text-slate-200">
                  <Sparkles className="w-3.5 h-3.5 text-emerald-400" />
                  Skills
                </h1>
                <div className="flex items-center gap-1">
                  <button onClick={handleExport} disabled={skills.length === 0} className="p-1.5 text-slate-700 hover:text-slate-400 disabled:opacity-30 transition-colors" title="Export as PNG">
                    <Download className="w-3 h-3" />
                  </button>
                  {/* ⋯ Overflow menu */}
                  <div style={{ position: 'relative' }}>
                    <button
                      onClick={() => setShowOverflowMenu(v => !v)}
                      className="p-1.5 text-slate-700 hover:text-slate-400 transition-colors"
                      title="Maintenance tools"
                    >
                      <MoreHorizontal className="w-3 h-3" />
                    </button>
                    {showOverflowMenu && (
                      <>
                        {/* Click-away backdrop */}
                        <div style={{ position: 'fixed', inset: 0, zIndex: 98 }} onClick={() => setShowOverflowMenu(false)} />
                        <div style={{
                          position: 'absolute', top: '100%', right: 0, zIndex: 99,
                          width: 224, marginTop: 4,
                          background: 'rgba(4,8,20,0.97)',
                          border: '1px solid rgba(255,255,255,0.09)',
                          borderRadius: 10,
                          boxShadow: '0 8px 32px rgba(0,0,0,0.7)',
                          backdropFilter: 'blur(12px)',
                          padding: '6px 0',
                        }}>
                          <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', padding: '4px 12px 6px' }}>Sync</div>

                          {/* Sync Job Skills (jobs only) */}
                          <button
                            onClick={() => { setShowOverflowMenu(false); handleSyncJobs(); }}
                            disabled={syncingJobs || syncing}
                            style={{ width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '6px 12px', background: 'none', border: 'none', cursor: (syncingJobs || syncing) ? 'default' : 'pointer', color: (syncingJobs || syncing) ? '#334155' : '#94a3b8', fontSize: 11 }}
                          >
                            <span style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                              {syncingJobs ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw style={{ width: 12, height: 12, color: '#fbbf24' }} />}
                              {syncingJobs ? 'Syncing Jobs…' : 'Sync Job Skills'}
                            </span>
                            <span style={{ fontSize: 8, color: '#475569', whiteSpace: 'nowrap' }}>JD only</span>
                          </button>

                          <div style={{ margin: '4px 12px', borderTop: '1px solid rgba(255,255,255,0.06)' }} />

                          <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', padding: '4px 12px 6px', opacity: 0.7 }}>Force re-run</div>

                          {/* Infer Dependencies — auto-runs on Sync; manual escape hatch */}
                          {skills.length >= 2 && (
                            <button
                              onClick={() => { setShowOverflowMenu(false); handleInferDeps(); }}
                              disabled={inferring}
                              style={{ width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '6px 12px', background: 'none', border: 'none', cursor: inferring ? 'default' : 'pointer', color: inferring ? '#334155' : '#64748b', fontSize: 11 }}
                            >
                              <span style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                                {inferring ? <Loader2 className="w-3 h-3 animate-spin" /> : <Wand2 style={{ width: 12, height: 12 }} />}
                                {inferring ? 'Inferring…' : 'Infer Dependencies'}
                              </span>
                              <span style={{ fontSize: 8, padding: '1px 5px', borderRadius: 3, background: 'rgba(99,102,241,0.08)', color: '#4f46e5', border: '1px solid rgba(99,102,241,0.14)', whiteSpace: 'nowrap' }}>llama-3.3-70b · Groq</span>
                            </button>
                          )}

                          {/* Classify Domains — auto-runs on Sync; manual escape hatch */}
                          <button
                            onClick={() => { setShowOverflowMenu(false); handleClassifyDomains(); }}
                            disabled={classifying || skills.length === 0}
                            style={{ width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '6px 12px', background: 'none', border: 'none', cursor: (classifying || skills.length === 0) ? 'default' : 'pointer', color: classifying ? '#334155' : '#64748b', fontSize: 11 }}
                          >
                            <span style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                              {classifying ? <Loader2 className="w-3 h-3 animate-spin" /> : <Sparkles style={{ width: 12, height: 12 }} />}
                              {classifying ? 'Classifying…' : 'Classify Domains'}
                            </span>
                            <span style={{ fontSize: 8, padding: '1px 5px', borderRadius: 3, background: 'rgba(99,102,241,0.08)', color: '#4f46e5', border: '1px solid rgba(99,102,241,0.14)', whiteSpace: 'nowrap' }}>scout-17b · Groq</span>
                          </button>

                          <div style={{ margin: '4px 12px', borderTop: '1px solid rgba(255,255,255,0.06)' }} />

                          <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', padding: '4px 12px 6px' }}>Maintenance</div>

                          {/* Reset Domain Assignments */}
                          <button
                            onClick={() => {
                              setShowOverflowMenu(false);
                              if (window.confirm('Reset all domain assignments? This cannot be undone.')) handleResetDomains();
                            }}
                            disabled={resetting || classifying || skills.length === 0}
                            style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 7, padding: '6px 12px', background: 'none', border: 'none', cursor: (resetting || classifying || skills.length === 0) ? 'default' : 'pointer', color: (resetting || classifying || skills.length === 0) ? '#334155' : '#f87171', fontSize: 11 }}
                          >
                            {resetting ? <Loader2 className="w-3 h-3 animate-spin" /> : <RotateCcw style={{ width: 12, height: 12 }} />}
                            {resetting ? 'Resetting…' : 'Reset Domain Assignments'}
                          </button>

                          {/* Merge Skills */}
                          {skills.length >= 2 && (
                            <button
                              onClick={() => { setShowOverflowMenu(false); setShowMerge(true); }}
                              style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 7, padding: '6px 12px', background: 'none', border: 'none', cursor: 'pointer', color: '#94a3b8', fontSize: 11 }}
                            >
                              <GitMerge style={{ width: 12, height: 12 }} />Merge Skills…
                            </button>
                          )}

                          {/* Auto-merge Duplicates */}
                          {skills.length >= 2 && (
                            <button
                              onClick={() => { setShowOverflowMenu(false); handleAutoMergePreview(); }}
                              style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 7, padding: '6px 12px', background: 'none', border: 'none', cursor: 'pointer', color: '#94a3b8', fontSize: 11 }}
                            >
                              <GitMerge style={{ width: 12, height: 12 }} />Auto-merge Duplicates
                            </button>
                          )}

                          <div style={{ margin: '4px 12px', borderTop: '1px solid rgba(255,255,255,0.06)' }} />
                          <div style={{ fontSize: 9, fontWeight: 600, color: '#334155', textTransform: 'uppercase', letterSpacing: '0.08em', padding: '4px 12px 6px' }}>Graph Health</div>

                          {/* Graph Audit */}
                          <button
                            onClick={() => { setShowOverflowMenu(false); setShowAuditPanel(true); }}
                            style={{ width: '100%', display: 'flex', alignItems: 'center', gap: 7, padding: '6px 12px', background: 'none', border: 'none', cursor: 'pointer', color: '#94a3b8', fontSize: 11 }}
                          >
                            <ShieldCheck style={{ width: 12, height: 12 }} />Graph Audit
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                  <button onClick={() => setSidebarCollapsed(true)} className="p-1.5 text-slate-700 hover:text-slate-400 transition-colors" title="Collapse sidebar">
                    <PanelLeftClose className="w-3 h-3" />
                  </button>
                </div>
              </div>

              {/* Sync — single button. Chains infer + classify in the background. */}
              <button
                onClick={handleSync}
                disabled={syncing || syncingJobs}
                className="w-full py-2 text-xs font-medium transition-colors flex items-center justify-center gap-2"
                style={{
                  borderRadius: 8,
                  border: '1px solid rgba(16,185,129,0.25)',
                  background: (syncing || syncingJobs) ? 'rgba(255,255,255,0.04)' : 'rgba(16,185,129,0.18)',
                  color: (syncing || syncingJobs) ? '#334155' : '#34d399',
                }}
                title="Sync, infer dependencies, classify domains"
              >
                {syncing ? <><Loader2 className="w-3 h-3 animate-spin" />Syncing…</> : <><RefreshCw className="w-3 h-3" />Sync</>}
              </button>

              {/* Growth Plan + Learning Path */}
              <div className="flex gap-1.5 mt-1.5">
                <button
                  onClick={handleOpenGrowthPlan}
                  className="flex-1 py-1.5 text-xs rounded-lg transition-colors flex items-center justify-center gap-2"
                  style={{ background: 'rgba(245,158,11,0.12)', color: '#f59e0b', border: '1px solid rgba(245,158,11,0.25)' }}
                >
                  <TrendingUp className="w-3 h-3" />Growth Plan
                </button>
                <button
                  onClick={() => handleOpenLearningPath(learningPathSeason)}
                  className="flex-1 py-1.5 text-xs rounded-lg transition-colors flex items-center justify-center gap-2"
                  style={{ background: 'rgba(99,102,241,0.12)', color: '#818cf8', border: '1px solid rgba(99,102,241,0.25)' }}
                >
                  <Route className="w-3 h-3" />Learning Path
                </button>
              </div>
            </>
          )}
        </div>

        {/* Stats */}
        {!sidebarCollapsed && skills.length > 0 && (
          <div style={{ padding: '10px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
            <div className="flex items-center justify-between text-[10px] text-slate-600 mb-1.5">
              <span>{skills.length} skills</span>
              <span>avg L{avgLevel}</span>
            </div>
            <div className="flex gap-1">
              {[1, 2, 3, 4, 5].map(lvl => (
                <div key={lvl} className="flex-1 text-center">
                  <div className="text-[9px] text-slate-700 mb-0.5">L{lvl}</div>
                  <div className={`text-[10px] font-medium ${levelCounts[lvl] > 0 ? 'text-emerald-600' : 'text-slate-800'}`}>{levelCounts[lvl]}</div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Toggles */}
        {!sidebarCollapsed && gaps.length > 0 && (
          <div style={{ padding: '6px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <span className="text-[10px] text-amber-600/80 flex items-center gap-1.5">
              <AlertTriangle className="w-2.5 h-2.5" />{gaps.length} gap{gaps.length !== 1 ? 's' : ''}
            </span>
            <button
              onClick={() => setShowGaps(!showGaps)}
              className="text-[9px] px-2 py-0.5 rounded transition-colors"
              style={{ background: showGaps ? 'rgba(251,191,36,0.12)' : 'rgba(255,255,255,0.04)', color: showGaps ? '#f59e0b' : '#334155' }}
            >
              {showGaps ? 'Visible' : 'Hidden'}
            </button>
          </div>
        )}
        {!sidebarCollapsed && (
          <div style={{ padding: '6px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
            {/* State filter chips */}
            {(['all', 'seed', 'adjacent'] as const).map(f => (
              <button
                key={f}
                onClick={() => setStateFilter(f)}
                className="text-[9px] px-2 py-0.5 rounded transition-colors capitalize"
                style={{
                  background: stateFilter === f
                    ? f === 'seed' ? 'rgba(245,158,11,0.15)' : f === 'adjacent' ? 'rgba(16,185,129,0.12)' : 'rgba(255,255,255,0.07)'
                    : 'rgba(255,255,255,0.03)',
                  color: stateFilter === f
                    ? f === 'seed' ? '#f59e0b' : f === 'adjacent' ? '#34d399' : '#94a3b8'
                    : '#334155',
                  border: '1px solid ' + (stateFilter === f
                    ? f === 'seed' ? 'rgba(245,158,11,0.3)' : f === 'adjacent' ? 'rgba(16,185,129,0.25)' : 'rgba(255,255,255,0.12)'
                    : 'rgba(255,255,255,0.04)'),
                }}
              >
                {f === 'seed' && <span style={{ display: 'inline-block', width: 5, height: 5, borderRadius: '50%', background: '#f59e0b', marginRight: 3, verticalAlign: 'middle' }} />}
                {f}
              </button>
            ))}
            {/* Review filter */}
            {reviewCount > 0 && (
              <button
                onClick={() => setShowReviewOnly(!showReviewOnly)}
                className="text-[9px] px-2 py-0.5 rounded transition-colors ml-auto"
                style={{ background: showReviewOnly ? 'rgba(249,115,22,0.12)' : 'rgba(255,255,255,0.04)', color: showReviewOnly ? '#f97316' : '#334155', border: '1px solid ' + (showReviewOnly ? 'rgba(249,115,22,0.25)' : 'rgba(255,255,255,0.04)') }}
                title={`${reviewCount} skills to review`}
              >
                <Eye className="w-2.5 h-2.5 inline mr-1" style={{ verticalAlign: 'middle' }} />
                {reviewCount}
              </button>
            )}
          </div>
        )}

        {/* Search + skill list */}
        {!sidebarCollapsed && (
          <>
            <div style={{ padding: '10px 14px 8px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
              <div className="flex items-center gap-2 rounded-lg px-2.5 py-1.5" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.06)' }}>
                <Search className="w-3 h-3 text-slate-700 flex-shrink-0" />
                <input
                  value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder="Search skills…"
                  className="flex-1 bg-transparent text-xs text-slate-300 placeholder-slate-700 outline-none"
                />
              </div>
            </div>

            <div className="flex-1 overflow-auto" style={{ padding: '6px 14px' }}>
              {skills.length === 0 ? (
                <div className="text-center py-10">
                  <Sparkles className="w-7 h-7 mx-auto mb-2" style={{ color: '#1e293b' }} />
                  <p className="text-xs text-slate-700 mb-1">No skills yet</p>
                  <p className="text-[10px] text-slate-800">Sync to import from resume, trees &amp; work.</p>
                </div>
              ) : (
                <div className="space-y-0.5">
                  {[...filteredDomainGroups.entries()].map(([domain, domainSkills]) => {
                    const expanded = expandedDomains.has(domain);
                    const domainColor = DOMAIN_COLORS[
                      [...(new Map(skills.map(s => [s.domain || 'General', true]))).keys()].indexOf(domain) % DOMAIN_COLORS.length
                    ];
                    return (
                      <div key={domain}>
                        <button
                          onClick={() => setExpandedDomains(prev => { const n = new Set(prev); n.has(domain) ? n.delete(domain) : n.add(domain); return n; })}
                          className="w-full flex items-center gap-1.5 py-1.5 text-[10px] transition-colors"
                          style={{ color: '#475569' }}
                        >
                          {expanded ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
                          <span className="font-medium capitalize" style={{ color: domainColor + 'cc' }}>{domain}</span>
                          <span className="ml-auto" style={{ color: '#2d3748' }}>{domainSkills.length}</span>
                        </button>
                        {expanded && (
                          <div className="ml-3 space-y-0.5">
                            {domainSkills.sort((a, b) => b.level - a.level).map(skill => (
                              <div
                                key={skill.id}
                                className="group w-full flex items-center gap-2 py-1 px-2 rounded cursor-pointer transition-colors"
                                style={{
                                  background: selectedId === skill.id ? 'rgba(255,255,255,0.06)' : 'transparent',
                                  color: selectedId === skill.id ? '#e2e8f0' : '#475569',
                                }}
                                onClick={() => setSelectedId(selectedId === skill.id ? null : skill.id)}
                              >
                                {skill.reviewNeeded && <span className="w-1.5 h-1.5 rounded-full bg-orange-500 flex-shrink-0" />}
                                <span className="text-[10px] truncate flex-1 capitalize">{skill.name}</span>
                                {skill.reviewNeeded && (
                                  <button onClick={e => handleMarkReviewed(skill.id, e)} className="hidden group-hover:flex items-center gap-1 text-[9px] px-1.5 py-0.5 rounded flex-shrink-0" style={{ background: 'rgba(249,115,22,0.12)', color: '#f97316' }}>
                                    <Eye className="w-2 h-2" />
                                  </button>
                                )}
                                {aliasCountMap.has(skill.id) && (
                                  <span style={{ fontSize: 9, padding: '1px 3px', borderRadius: 3, background: 'rgba(99,102,241,0.13)', color: '#818cf8', flexShrink: 0 }}>{aliasCountMap.get(skill.id)}↗</span>
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

              {showGaps && gaps.length > 0 && (
                <div className="mt-3 pt-3" style={{ borderTop: '1px solid rgba(255,255,255,0.05)' }}>
                  <div className="text-[9px] font-semibold text-amber-700/70 uppercase tracking-wider mb-1.5">Skill Gaps</div>
                  {gaps.slice(0, 15).map(gap => (
                    <button
                      key={gap.skillName}
                      onClick={() => setSelectedId(selectedId === 'gap-' + gap.skillName ? null : 'gap-' + gap.skillName)}
                      className="w-full flex items-center gap-2 py-1 px-2 rounded text-left text-[10px] transition-colors"
                      style={{ color: '#78350f' }}
                    >
                      <AlertTriangle className="w-2.5 h-2.5 flex-shrink-0 text-amber-700" />
                      <span className="truncate flex-1">{gap.skillName}</span>
                      <span style={{ fontSize: 9, color: '#334155' }}>{gap.demandCount}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </>
        )}
      </div>

      {/* ── Canvas ─────────────────────────────────────────────────────────────── */}
      <div className="flex-1 relative overflow-hidden" ref={containerRef}>
        {syncToast && (
          <div style={{
            position: 'absolute', top: 16, left: '50%', transform: 'translateX(-50%)',
            background: 'rgba(15,15,25,0.92)', border: '1px solid rgba(16,185,129,0.35)',
            borderRadius: 8, padding: '8px 16px', fontSize: 12, color: '#6ee7b7',
            zIndex: 51, backdropFilter: 'blur(6px)', whiteSpace: 'nowrap',
            boxShadow: '0 4px 24px rgba(0,0,0,0.5)',
            display: 'flex', alignItems: 'center', gap: 6,
          }}>
            {syncing || syncToast.startsWith('Syncing') || syncToast.startsWith('Inferring') || syncToast.startsWith('Classifying')
              ? <Loader2 size={12} className="animate-spin" />
              : <RefreshCw size={12} />}
            {syncToast}
          </div>
        )}
        {classifyToast && (
          <div style={{
            position: 'absolute', top: 16, left: '50%', transform: 'translateX(-50%)',
            background: 'rgba(15,15,25,0.92)', border: '1px solid rgba(99,102,241,0.35)',
            borderRadius: 8, padding: '8px 16px', fontSize: 12, color: '#c4b5fd',
            zIndex: 50, backdropFilter: 'blur(6px)', whiteSpace: 'nowrap',
            boxShadow: '0 4px 24px rgba(0,0,0,0.5)',
          }}>
            <Sparkles size={12} style={{ display: 'inline', marginRight: 6, verticalAlign: 'middle' }} />
            {classifyToast}
          </div>
        )}
        {backfillToast && (
          <div style={{
            position: 'absolute', top: 16, left: '50%', transform: 'translateX(-50%)',
            background: 'rgba(15,15,25,0.92)', border: '1px solid rgba(16,185,129,0.35)',
            borderRadius: 8, padding: '8px 16px', fontSize: 12, color: '#6ee7b7',
            zIndex: 50, backdropFilter: 'blur(6px)', whiteSpace: 'nowrap',
            boxShadow: '0 4px 24px rgba(0,0,0,0.5)',
          }}>
            {backfillToast}
          </div>
        )}
        <canvas
          ref={canvasRef}
          style={{
            display: 'block',
            width: size.w + 'px',
            height: size.h + 'px',
            cursor: isDragging.current ? 'grabbing' : hoveredId ? 'pointer' : 'grab',
          }}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onClick={handleCanvasClick}
          onDoubleClick={handleCanvasDblClick}
          onMouseLeave={() => { isDragging.current = false; dragMoved.current = false; setHoveredId(null); }}
        />

        {/* Zoom controls */}
        {skills.length > 0 && (
          <div
            style={{
              position: 'absolute', bottom: 16, right: 16, zIndex: 10,
              display: 'flex', flexDirection: 'column', gap: 6,
            }}
          >
            <div
              style={{
                display: 'flex', flexDirection: 'column',
                borderRadius: 8,
                background: 'rgba(255,255,255,0.06)',
                border: '1px solid rgba(255,255,255,0.10)',
                overflow: 'hidden',
              }}
            >
              <button
                onClick={() => zoomByFactor(1.25)}
                title="Zoom in"
                style={{
                  width: 36, height: 28,
                  background: 'transparent', border: 'none',
                  color: '#94a3b8', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 14, lineHeight: 1,
                }}
              >+</button>
              <div
                style={{
                  width: 36, height: 22,
                  borderTop: '1px solid rgba(255,255,255,0.06)',
                  borderBottom: '1px solid rgba(255,255,255,0.06)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 9, color: '#64748b',
                }}
              >
                <span ref={zoomLabelRef}>100%</span>
              </div>
              <button
                onClick={() => zoomByFactor(0.8)}
                title="Zoom out"
                style={{
                  width: 36, height: 28,
                  background: 'transparent', border: 'none',
                  color: '#94a3b8', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 14, lineHeight: 1,
                }}
              >−</button>
            </div>
            <button
              onClick={handleFitAll}
              title="Fit all skills"
              style={{
                width: 36, height: 28, borderRadius: 8,
                background: 'rgba(255,255,255,0.06)',
                border: '1px solid rgba(255,255,255,0.10)',
                color: '#94a3b8', cursor: 'pointer',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}
            >
              <Maximize2 size={12} />
            </button>
          </div>
        )}

        {/* Empty state */}
        {skills.length === 0 && !loading && (
          <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', pointerEvents: 'none' }}>
            <Sparkles size={28} style={{ marginBottom: 12, color: '#1e293b' }} />
            <div style={{ fontSize: 13, color: '#1e293b' }}>Sync your skills to grow the tree</div>
          </div>
        )}

        {/* Skill detail panel — real universal_skills row */}
        {selectedSkill && (
          <SkillDetailPanel
            skillId={selectedSkill.id}
            onClose={() => setSelectedId(null)}
            onSelectSkill={name => {
              const s = skills.find(sk => sk.name.toLowerCase() === name.toLowerCase());
              if (s) setSelectedId(s.id);
            }}
            onOpenGrowthPlan={handleOpenGrowthPlan}
          />
        )}

        {/* Gap-pseudo panel for canvas `gap-<name>` orphans */}
        {!selectedSkill && selectedGap && (
          <GapPseudoPanel gap={selectedGap} onClose={() => setSelectedId(null)} />
        )}

        {/* Growth Plan panel */}
        {showGrowthPlan && (
          <GrowthPlanPanel
            targets={growthTargets}
            loading={growthLoading}
            expandingGraph={expandingGraph}
            onClose={() => setShowGrowthPlan(false)}
            onExpandGraph={handleExpandSkillGraph}
            onSelectSkill={name => {
              const skill = skills.find(s => s.name.toLowerCase() === name.toLowerCase());
              if (skill) setSelectedId(skill.id);
            }}
          />
        )}

        {/* Learning Path panel */}
        {showLearningPath && (
          <LearningPathPanel
            learningPath={learningPath}
            loading={learningPathLoading}
            onClose={() => setShowLearningPath(false)}
            season={learningPathSeason}
            onSeasonChange={handleLearningPathSeasonChange}
            onSelectSkill={name => {
              const skill = skills.find(s => s.name.toLowerCase() === name.toLowerCase());
              if (skill) setSelectedId(skill.id);
            }}
          />
        )}

        {/* Graph Audit panel */}
        {showAuditPanel && (
          <GraphAuditPanel
            onApprove={loadAll}
            onReject={() => {}}
            onClose={() => setShowAuditPanel(false)}
          />
        )}
      </div>

      {showMerge && (
        <MergeModal skills={skills} onClose={() => setShowMerge(false)} onMerged={loadAll} />
      )}
      {autoMergeSuggestions !== null && (
        <AutoMergeModal
          suggestions={autoMergeSuggestions}
          onClose={() => setAutoMergeSuggestions(null)}
          onConfirm={handleAutoMergeConfirm}
          merging={autoMerging}
        />
      )}
    </div>
  );
}
