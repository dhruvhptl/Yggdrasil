// src/pages/SkillsPage.tsx
// Universal Skill Tree — cosmic organic tree, branches spread like a real tree spanning the universe.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  RefreshCw, Loader2, AlertTriangle, Sparkles, FileText, TreePine,
  Briefcase, X, Wand2, GitMerge, Check, Search, Eye, Download, ChevronDown, ChevronRight,
  PanelLeftClose, PanelLeftOpen, RotateCcw,
} from 'lucide-react';
import type { UniversalSkill, SkillGap, SkillDependency, SkillAlias, SkillGraphSnapshot } from '../types';
import { validateOrLog, SkillSchema } from '../lib/validators';
import { log } from '../lib/logger';
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

// Radii for skill placement tiers along each bough
const SKILL_RADII = [160, 240, 320];
// Branch widths
const W_TRUNK = 8;
const W_BOUGH = 4;
const W_TWIG  = 1.5;

// ─── Types ────────────────────────────────────────────────────────────────────

interface SkillPlacement {
  skill: UniversalSkill | null;
  gap: SkillGap | null;
  x: number;
  y: number;
  color: string;
  domainIndex: number;
  /** outward angle from center — used for hit-test and label positioning */
  outAngle: number;
}

interface Branch {
  x1: number; y1: number;
  x2: number; y2: number;
  cpx: number; cpy: number;
  w1: number; w2: number;
  color: string;
  /** 0=trunk/root, 1=bough, 2=twig, 3=decorative */
  depth: number;
  /** which domain this branch belongs to (-1 = trunk/root) */
  domainIndex: number;
}

interface DomainLabel {
  name: string;
  color: string;
  x: number;
  y: number;
}

interface LayoutResult {
  placements: SkillPlacement[];
  branches: Branch[];
  domainLabels: DomainLabel[];
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

function layoutSkillTree(
  skills: UniversalSkill[],
  gaps: SkillGap[],
  showGaps: boolean,
  w: number,
  h: number,
): LayoutResult {
  const branches: Branch[] = [];
  const placements: SkillPlacement[] = [];
  const domainLabels: DomainLabel[] = [];

  const sc = Math.min(w, h) / 900;
  const cx = w * 0.50;
  const cy = h * 0.55;

  // ── Group skills by domain ───────────────────────────────────────────────
  const domainMap = new Map<string, UniversalSkill[]>();
  for (const s of skills) {
    const d = s.domain || 'General';
    if (!domainMap.has(d)) domainMap.set(d, []);
    domainMap.get(d)!.push(s);
  }
  const domainNames = [...domainMap.keys()];
  const domainCount = domainNames.length || 1;

  log('[SkillsPage] domain groups:', domainNames.map(d => `${d}(${domainMap.get(d)!.length})`).join(', '));

  // ── Trunk ────────────────────────────────────────────────────────────────
  const trunkLen = 80 * sc;
  const trunkOrigin = { x: cx, y: cy };         // where boughs split from
  const trunkTip    = { x: cx, y: cy - trunkLen };

  branches.push({
    x1: cx, y1: cy,
    x2: trunkTip.x, y2: trunkTip.y,
    cpx: cx, cpy: cy - trunkLen * 0.5,
    w1: W_TRUNK * sc, w2: (W_TRUNK * 0.6) * sc,
    color: '#4a2e14', depth: 0, domainIndex: -1,
  });

  // ── Atmospheric roots ────────────────────────────────────────────────────
  const rand = seededRand(42);
  for (let r = 0; r < 5; r++) {
    const rootAng = (Math.PI / 6) + (r / 4) * (Math.PI * 2 / 3) - Math.PI / 3 + Math.PI / 2;
    const rootLen = (35 + r * 10) * sc;
    branches.push({
      x1: cx, y1: cy,
      x2: cx + rootLen * Math.cos(rootAng),
      y2: cy + rootLen * Math.sin(rootAng),
      cpx: cx + rootLen * 0.5 * Math.cos(rootAng + 0.3),
      cpy: cy + rootLen * 0.5 * Math.sin(rootAng + 0.3),
      w1: 6 * sc, w2: 0.8 * sc,
      color: '#2e1c0c', depth: 0, domainIndex: -1,
    });
  }

  // ── Compute proportional angle allocation ────────────────────────────────
  // Domains with more skills get a larger slice of the arc.
  const totalSkills = Math.max(1, skills.length);
  const arcSpanDeg  = Math.min(300, 50 + domainCount * 35); // total spread
  const arcStartDeg = -90 - arcSpanDeg / 2;                 // top-centred
  const minGapDeg   = 20;                                    // minimum between boughs

  // Raw weight per domain
  const rawWeights = domainNames.map(d => Math.max(1, domainMap.get(d)!.length / totalSkills));
  const weightSum  = rawWeights.reduce((a, b) => a + b, 0);
  // Normalised fractions → convert to degrees, respecting minGap
  const allocDeg = rawWeights.map(w => (w / weightSum) * arcSpanDeg);
  // Running cumulative mid-angle for each domain
  const midAngles: number[] = [];
  let cursor = arcStartDeg;
  for (let i = 0; i < domainCount; i++) {
    midAngles.push(cursor + allocDeg[i] / 2);
    cursor += Math.max(allocDeg[i], minGapDeg);
  }

  // ── Per-domain boughs + skill placement ─────────────────────────────────
  domainNames.forEach((domainName, dIdx) => {
    const domainSkills = domainMap.get(domainName)!;
    const color = DOMAIN_COLORS[dIdx % DOMAIN_COLORS.length];
    const spineAngle = (midAngles[dIdx] * Math.PI) / 180;

    // Bough: from trunk tip outward
    // Length is generous — label goes at the tip, not midpoint
    const boughLen = 130 * sc;
    const boughTip = {
      x: trunkTip.x + boughLen * Math.cos(spineAngle),
      y: trunkTip.y + boughLen * Math.sin(spineAngle),
    };
    // S-curve control point: alternating bias gives organic spread
    const cpBias   = (dIdx % 2 === 0 ? -1 : 1) * 0.20;
    const perpAng  = spineAngle + Math.PI / 2;
    const boughCpx = (trunkTip.x + boughTip.x) / 2 + cpBias * boughLen * Math.cos(perpAng);
    const boughCpy = (trunkTip.y + boughTip.y) / 2 + cpBias * boughLen * Math.sin(perpAng);

    branches.push({
      x1: trunkTip.x, y1: trunkTip.y,
      x2: boughTip.x, y2: boughTip.y,
      cpx: boughCpx, cpy: boughCpy,
      w1: W_BOUGH * sc, w2: (W_BOUGH * 0.4) * sc,
      color, depth: 1, domainIndex: dIdx,
    });

    // Domain label at bough tip
    domainLabels.push({ name: domainName, color, x: boughTip.x, y: boughTip.y });

    // ── Skills — distribute across radii ──────────────────────────────────
    const sorted = [...domainSkills].sort((a, b) => a.level - b.level);
    const skillCount = sorted.length;
    if (skillCount === 0) return;

    // Fan half-angle: ±15° max (tight so neighbours don't collide)
    const halfFanDeg = Math.min(15, 4 + skillCount * 1.5);

    // Evenly distribute across three radii tiers
    const tierSize = Math.ceil(skillCount / SKILL_RADII.length);

    sorted.forEach((skill, sIdx) => {
      const tier = Math.min(Math.floor(sIdx / tierSize), SKILL_RADII.length - 1);
      const posInTier = sIdx - tier * tierSize;
      const tierCount = Math.min(tierSize, skillCount - tier * tierSize);

      const fanFrac = tierCount <= 1 ? 0.5 : posInTier / (tierCount - 1);
      const fanOffDeg = (fanFrac - 0.5) * 2 * halfFanDeg + (rand() - 0.5) * 3;
      const twigAngle = spineAngle + (fanOffDeg * Math.PI / 180);

      const r = SKILL_RADII[tier] * sc;
      const leafPos = {
        x: trunkOrigin.x + r * Math.cos(twigAngle),
        y: trunkOrigin.y + r * Math.sin(twigAngle),
      };

      // Twig anchor: interpolate along bough toward bough tip
      const twigT = 0.3 + tier * 0.25;
      const twigAnchorX = trunkTip.x + (boughTip.x - trunkTip.x) * twigT;
      const twigAnchorY = trunkTip.y + (boughTip.y - trunkTip.y) * twigT;

      // Twig control: slight outward curve
      const twigCpx = (twigAnchorX + leafPos.x) / 2 + (rand() - 0.5) * 14 * sc * Math.cos(perpAng);
      const twigCpy = (twigAnchorY + leafPos.y) / 2 + (rand() - 0.5) * 14 * sc * Math.sin(perpAng);

      branches.push({
        x1: twigAnchorX, y1: twigAnchorY,
        x2: leafPos.x, y2: leafPos.y,
        cpx: twigCpx, cpy: twigCpy,
        w1: W_TWIG * sc, w2: 0.6 * sc,
        color, depth: 2, domainIndex: dIdx,
      });

      placements.push({
        skill,
        gap: null,
        x: leafPos.x,
        y: leafPos.y,
        color,
        domainIndex: dIdx,
        outAngle: Math.atan2(leafPos.y - cy, leafPos.x - cx),
      });
    });
  });

  // ── Gap ring (optional, no connector lines, static) ──────────────────────
  if (showGaps && gaps.length > 0) {
    const existingNames = new Set(skills.map(s => s.name.toLowerCase()));
    const visibleGaps = gaps.filter(g => !(existingNames.has(g.skillName.toLowerCase()) && g.currentLevel > 0));
    const gapR = 380 * sc;
    visibleGaps.forEach((gap, i) => {
      const angle = ((i / visibleGaps.length) * 360 - 90) * Math.PI / 180;
      placements.push({
        skill: null, gap,
        x: cx + gapR * Math.cos(angle),
        y: cy + gapR * Math.sin(angle),
        color: GAP_COLOR,
        domainIndex: -1,
        outAngle: angle,
      });
    });
  }

  return { placements, branches, domainLabels };
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

// Quadratic-bezier tapered branch
function taperedBezierBranch(
  ctx: CanvasRenderingContext2D,
  x1: number, y1: number,
  cpx: number, cpy: number,
  x2: number, y2: number,
  w1: number, w2: number,
) {
  if (w1 < 0.15 && w2 < 0.15) return;
  const dxS = cpx - x1, dyS = cpy - y1;
  const lenS = Math.hypot(dxS, dyS) || 1;
  const nxS = -dyS / lenS, nyS = dxS / lenS;
  const dxE = x2 - cpx, dyE = y2 - cpy;
  const lenE = Math.hypot(dxE, dyE) || 1;
  const nxE = -dyE / lenE, nyE = dxE / lenE;
  const hw1 = w1 / 2, hw2 = w2 / 2, hwM = (hw1 + hw2) / 2;

  ctx.beginPath();
  ctx.moveTo(x1 + hw1 * nxS, y1 + hw1 * nyS);
  ctx.quadraticCurveTo(cpx + hwM * nxS, cpy + hwM * nyS, x2 + hw2 * nxE, y2 + hw2 * nyE);
  ctx.lineTo(x2 - hw2 * nxE, y2 - hw2 * nyE);
  ctx.quadraticCurveTo(cpx - hwM * nxS, cpy - hwM * nyS, x1 - hw1 * nxS, y1 - hw1 * nyS);
  ctx.closePath();
}

function drawBranches(
  ctx: CanvasRenderingContext2D,
  branches: Branch[],
  selectedDomainIndex: number | null,
  growFrac: number,          // 0→1 animation progress
) {
  const sorted = [...branches].sort((a, b) => a.depth - b.depth);
  sorted.forEach(b => {
    // Selection dimming: dim branches not on selected domain
    let domainAlpha = 1;
    if (selectedDomainIndex !== null && b.domainIndex !== -1 && b.domainIndex !== selectedDomainIndex) {
      domainAlpha = 0.10;
    }

    // Branch-growth animation: interpolate endpoint toward x1/y1
    let x2 = b.x2, y2 = b.y2, cpx = b.cpx, cpy = b.cpy;
    if (growFrac < 1) {
      const t = growFrac;
      x2   = b.x1 + (b.x2  - b.x1)  * t;
      y2   = b.y1 + (b.y2  - b.y1)  * t;
      cpx  = b.x1 + (b.cpx - b.x1)  * t;
      cpy  = b.y1 + (b.cpy - b.y1)  * t;
    }

    taperedBezierBranch(ctx, b.x1, b.y1, cpx, cpy, x2, y2, b.w1, b.w2);

    const c = hexToRgb(b.color);
    const grad = ctx.createLinearGradient(b.x1, b.y1, x2, y2);
    ctx.globalAlpha = domainAlpha;
    if (b.depth === 0) {
      grad.addColorStop(0, '#5a3620');
      grad.addColorStop(0.5, '#3d2410');
      grad.addColorStop(1, '#1e0f05');
    } else if (b.depth === 1) {
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.70)`);
      grad.addColorStop(0.6, `rgba(${c.r},${c.g},${c.b},0.38)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.18)`);
    } else if (b.depth === 2) {
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.45)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.10)`);
    } else {
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.18)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.03)`);
    }
    ctx.fillStyle = grad;
    ctx.fill();
    ctx.globalAlpha = 1;
  });
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

function drawCenterNode(ctx: CanvasRenderingContext2D, cx: number, cy: number, animTime: number, sc: number) {
  const pulse = 0.5 + 0.5 * Math.sin(animTime * 1.1);
  const outerR = (22 + pulse * 4) * sc;

  const halo = ctx.createRadialGradient(cx, cy, 0, cx, cy, outerR * 2.4);
  halo.addColorStop(0, `rgba(16,185,129,${(0.12 + pulse * 0.06).toFixed(3)})`);
  halo.addColorStop(0.5, 'rgba(16,185,129,0.025)');
  halo.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.beginPath();
  ctx.arc(cx, cy, outerR * 2.4, 0, Math.PI * 2);
  ctx.fillStyle = halo;
  ctx.fill();

  ctx.beginPath();
  ctx.arc(cx, cy, outerR, 0, Math.PI * 2);
  ctx.strokeStyle = `rgba(16,185,129,${(0.25 + pulse * 0.18).toFixed(3)})`;
  ctx.lineWidth = 1.4 * sc;
  ctx.stroke();

  const rootGlow = ctx.createRadialGradient(cx, cy, 0, cx, cy, 32 * sc);
  rootGlow.addColorStop(0, 'rgba(90,54,32,0.50)');
  rootGlow.addColorStop(0.5, 'rgba(60,36,20,0.20)');
  rootGlow.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.beginPath();
  ctx.arc(cx, cy, 32 * sc, 0, Math.PI * 2);
  ctx.fillStyle = rootGlow;
  ctx.fill();

  const core = ctx.createRadialGradient(cx, cy - sc, 0, cx, cy, 6.5 * sc);
  core.addColorStop(0, '#e0fff6');
  core.addColorStop(0.35, '#10b981');
  core.addColorStop(0.75, '#064e3b');
  core.addColorStop(1, '#021a12');
  ctx.beginPath();
  ctx.arc(cx, cy, 6.5 * sc, 0, Math.PI * 2);
  ctx.fillStyle = core;
  ctx.fill();
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
) {
  if (growFrac < 0.01) return;
  const r = NODE_R[Math.max(1, Math.min(5, level))] * Math.min(growFrac * 2, 1);
  if (r < 0.5) return;
  const c = hexToRgb(color);
  ctx.globalAlpha = domainAlpha;

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

function drawDomainLabels(
  ctx: CanvasRenderingContext2D,
  labels: DomainLabel[],
  selectedDomainIndex: number | null,
  _placements: SkillPlacement[],
  _centerX: number,
  growFrac: number,
) {
  if (growFrac < 0.6) return;
  const alpha = Math.min(1, (growFrac - 0.6) / 0.3);

  labels.forEach((lbl, dIdx) => {
    const c = hexToRgb(lbl.color);
    const dimmed = selectedDomainIndex !== null && selectedDomainIndex !== dIdx;
    const a = (dimmed ? 0.20 : 0.75) * alpha;

    ctx.font = '700 13px "DM Sans", system-ui, sans-serif';
    ctx.letterSpacing = '0.06em';
    const tw = ctx.measureText(lbl.name).width;

    // Pill
    ctx.fillStyle = `rgba(${c.r},${c.g},${c.b},${(a * 0.15).toFixed(3)})`;
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.roundRect(lbl.x - tw / 2 - 7, lbl.y - 10, tw + 14, 18, 5);
    ctx.fill();

    ctx.fillStyle = `rgba(${c.r},${c.g},${c.b},${a.toFixed(3)})`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(lbl.name, lbl.x, lbl.y);
    ctx.textAlign = 'left';
    ctx.letterSpacing = '0';
  });
}

// Draw dep edges for selected skill only
function drawDepEdges(
  ctx: CanvasRenderingContext2D,
  placements: SkillPlacement[],
  selectedId: string | null,
  deps: SkillDependency[],
) {
  if (!selectedId) return;
  const bySkillId = new Map(placements.filter(p => p.skill).map(p => [p.skill!.id, p]));
  const selPlacement = bySkillId.get(selectedId);
  if (!selPlacement) return;

  const relevant = deps.filter(
    d => d.sourceSkillId === selectedId || d.targetSkillId === selectedId,
  );

  relevant.forEach(d => {
    const otherId = d.sourceSkillId === selectedId ? d.targetSkillId : d.sourceSkillId;
    const other   = bySkillId.get(otherId);
    if (!other) return;

    const isPrereq  = d.targetSkillId === selectedId;
    const edgeColor = isPrereq ? '#818cf8' : '#34d399';
    const cx        = hexToRgb(edgeColor);

    ctx.beginPath();
    ctx.moveTo(selPlacement.x, selPlacement.y);
    ctx.lineTo(other.x, other.y);
    ctx.strokeStyle = `rgba(${cx.r},${cx.g},${cx.b},0.55)`;
    ctx.lineWidth = 1.2;
    ctx.setLineDash([4, 4]);
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

// ─── Evidence Icon ────────────────────────────────────────────────────────────

function EvidenceIcon({ type }: { type: string }) {
  switch (type) {
    case 'resume':        return <FileText  className="w-3 h-3 text-blue-400 flex-shrink-0" />;
    case 'tree_quest':    return <TreePine  className="w-3 h-3 text-emerald-400 flex-shrink-0" />;
    case 'work_resource': return <Briefcase className="w-3 h-3 text-violet-400 flex-shrink-0" />;
    default:              return <Sparkles  className="w-3 h-3 text-slate-400 flex-shrink-0" />;
  }
}

// ─── Skill Detail Panel ───────────────────────────────────────────────────────

function SkillPanel({
  skill, gap, aliases, deps, skills, onClose,
}: {
  skill: UniversalSkill | null;
  gap: SkillGap | null;
  aliases: SkillAlias[];
  deps: SkillDependency[];
  skills: UniversalSkill[];
  onClose: () => void;
}) {
  if (!skill && !gap) return null;

  const skillAliases = skill ? aliases.filter(a => a.canonicalSkillId === skill.id) : [];
  const skillById    = new Map(skills.map(s => [s.id, s]));
  const prereqs = skill
    ? deps.filter(d => d.targetSkillId === skill.id).map(d => skillById.get(d.sourceSkillId)).filter(Boolean) as UniversalSkill[]
    : [];
  const unlocks = skill
    ? deps.filter(d => d.sourceSkillId === skill.id).map(d => skillById.get(d.targetSkillId)).filter(Boolean) as UniversalSkill[]
    : [];

  const panelColor = gap && !skill ? GAP_COLOR : '#10b981';

  return (
    <div
      style={{
        position: 'absolute', top: 16, right: 16, zIndex: 50,
        width: 300, maxHeight: 'calc(100% - 32px)',
        background: 'rgba(4,8,20,0.94)',
        backdropFilter: 'blur(16px)',
        border: `1px solid ${panelColor}22`,
        borderRadius: 16,
        overflow: 'auto',
        boxShadow: `0 12px 48px rgba(0,0,0,0.85), 0 0 0 1px ${panelColor}11`,
      }}
      onClick={e => e.stopPropagation()}
    >
      <div style={{ padding: '14px 16px 10px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 8 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: gap && !skill ? GAP_COLOR : '#f1f5f9', marginBottom: 2 }}>
              {skill ? skill.name : gap!.skillName}
            </div>
            {skill?.domain && <div style={{ fontSize: 10, color: '#475569' }}>{skill.domain}</div>}
            {gap && !skill && <div style={{ fontSize: 10, color: '#78350f' }}>Skill Gap — not yet mastered</div>}
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: '#475569', cursor: 'pointer', flexShrink: 0, padding: 2 }}>
            <X size={14} />
          </button>
        </div>
      </div>

      <div style={{ padding: '10px 16px' }}>
        {skill && (
          <>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
              <LevelDots level={skill.level} />
              <span style={{ fontSize: 11, color: '#94a3b8' }}>Level {skill.level} — {LEVEL_LABELS[skill.level] || ''}</span>
            </div>
            {skill.evidence.length > 0 && (
              <div style={{ marginBottom: 12 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 6 }}>Evidence</div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {skill.evidence.map((e, i) => (
                    <div key={i} style={{ display: 'flex', alignItems: 'flex-start', gap: 6, fontSize: 11, color: '#94a3b8' }}>
                      <EvidenceIcon type={e.type} />
                      <span>
                        {e.type === 'resume' && (e.detail || 'Listed on resume')}
                        {e.type === 'tree_quest' && (<><span style={{ color: '#34d399' }}>{e.projectName}</span>{' → '}{e.nodeTitle}{e.progress !== undefined && <span style={{ color: '#475569' }}> ({e.progress}%)</span>}</>)}
                        {e.type === 'work_resource' && (<><span style={{ color: '#a78bfa' }}>{e.company}</span>{' — '}{e.resourceTitle}</>)}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}
            {skillAliases.length > 0 && (
              <div style={{ marginBottom: 12 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 4 }}>Also known as</div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                  {skillAliases.map(a => (
                    <span key={a.id} style={{ fontSize: 10, padding: '2px 6px', borderRadius: 4, background: 'rgba(99,102,241,0.12)', color: '#818cf8', border: '1px solid rgba(99,102,241,0.2)' }}>{a.alias}</span>
                  ))}
                </div>
              </div>
            )}
            {prereqs.length > 0 && (
              <div style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 4 }}>Prerequisites</div>
                {prereqs.map(s => <div key={s.id} style={{ fontSize: 11, color: '#64748b', padding: '2px 0' }}>← {s.name}</div>)}
              </div>
            )}
            {unlocks.length > 0 && (
              <div>
                <div style={{ fontSize: 9, fontWeight: 600, color: '#475569', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 4 }}>Unlocks</div>
                {unlocks.map(s => <div key={s.id} style={{ fontSize: 11, color: '#64748b', padding: '2px 0' }}>→ {s.name}</div>)}
              </div>
            )}
          </>
        )}
        {gap && !skill && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, fontSize: 11, color: '#92400e' }}>
            <div>Demanded by <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{gap.demandCount}</span> job(s)</div>
            <div>Frequency: <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{(gap.frequency * 100).toFixed(0)}%</span></div>
            <div>Demand score: <span style={{ color: GAP_COLOR, fontWeight: 500 }}>{gap.demandScore.toFixed(1)}</span></div>
          </div>
        )}
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

// ─── Easing ───────────────────────────────────────────────────────────────────

function easeOutCubic(t: number) {
  return 1 - Math.pow(1 - t, 3);
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
  const [inferring, setInferring] = useState(false);
  const [classifying, setClassifying] = useState(false);
  const [resetting, setResetting]     = useState(false);
  const [classifyToast, setClassifyToast] = useState<string | null>(null);
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

  const transform    = useRef({ x: 0, y: 0, scale: 1 });
  const isDragging   = useRef(false);
  const lastMouse    = useRef({ x: 0, y: 0 });
  const dragMoved    = useRef(false);
  const [renderTick, setRenderTick] = useState(0);
  const triggerRender = useCallback(() => setRenderTick(t => t + 1), []);

  const { placements, branches, domainLabels } = useMemo(
    () => layoutSkillTree(skills, gaps, showGaps, size.w, size.h),
    [skills, gaps, showGaps, size.w, size.h],
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

  // Which domain is selected (for dimming everything else)
  const selectedDomainIndex = useMemo(() => {
    if (!selectedId) return null;
    const p = placements.find(p =>
      (p.skill && p.skill.id === selectedId) ||
      (p.gap   && 'gap-' + p.gap.skillName === selectedId),
    );
    return p ? p.domainIndex : null;
  }, [selectedId, placements]);

  const filteredDomainGroups = useMemo(() => {
    const base = showReviewOnly ? skills.filter(s => s.reviewNeeded) : skills;
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
  }, [skills, showReviewOnly, searchQuery]);

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
    const sc       = Math.min(size.w, size.h) / 900;
    const cx       = size.w * 0.50;
    const cy       = size.h * 0.55;

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

    drawBranches(ctx, branches, selectedDomainIndex, growFrac);
    drawCenterNode(ctx, cx, cy, animTime, sc);

    // Dep edges behind nodes
    drawDepEdges(ctx, placements, selectedId, deps);

    // Draw nodes
    const hoveredPlacement = hoveredId ? placements.find(p =>
      (p.skill && p.skill.id === hoveredId) ||
      (p.gap   && 'gap-' + p.gap.skillName === hoveredId),
    ) ?? null : null;

    placements.forEach(p => {
      const pid        = p.skill ? p.skill.id : 'gap-' + (p.gap?.skillName ?? '');
      const isSelected = pid === selectedId;
      const isHovered  = pid === hoveredId;
      const domAlpha   = selectedDomainIndex !== null && p.domainIndex !== selectedDomainIndex ? 0.10 : 1;

      if (p.skill) {
        drawSkillNode(ctx, p.x, p.y, p.color, p.skill.level, isHovered, isSelected, animTime, domAlpha, growFrac);
      } else {
        drawGapNode(ctx, p.x, p.y, animTime, domAlpha);
      }
    });

    // Domain labels (fade in after grow)
    if (skills.length > 0) {
      drawDomainLabels(ctx, domainLabels, selectedDomainIndex, placements, cx, growFrac);
    }

    // Hover tooltip (on top of everything)
    if (hoveredPlacement) {
      drawHoverLabel(ctx, hoveredPlacement, cx);
    }

    // Permanent skill labels only when zoomed in past 1.8x
    if (t.scale > 1.8) {
      placements.forEach(p => {
        if (!p.skill) return;
        const domAlpha = selectedDomainIndex !== null && p.domainIndex !== selectedDomainIndex ? 0.10 : 1;
        const label = p.skill.name.length > 20 ? p.skill.name.slice(0, 20) + '…' : p.skill.name;
        const pid = p.skill.id;
        if (pid === hoveredId) return; // already shown by hover tooltip
        ctx.font = '500 9px "DM Sans", system-ui, sans-serif';
        const tw = ctx.measureText(label).width;
        const lx = p.x > cx ? p.x + 10 : p.x - 10 - tw;
        ctx.globalAlpha = 0.60 * domAlpha;
        ctx.fillStyle = '#c8d8e8';
        ctx.textBaseline = 'middle';
        ctx.textAlign = 'left';
        ctx.fillText(label, lx, p.y);
        ctx.globalAlpha = 1;
      });
    }

    ctx.restore();
  }, [branches, placements, domainLabels, selectedId, selectedDomainIndex, hoveredId,
      size, renderTick, domainColors, skills, deps]);

  // ── RAF loop ──────────────────────────────────────────────────────────────
  useEffect(() => {
    if (skills.length === 0 && gaps.length === 0) return;
    let rafId: number;
    const tick = () => { triggerRender(); rafId = requestAnimationFrame(tick); };
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

  function handleExport() {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const url = canvas.toDataURL('image/png');
    const a   = document.createElement('a');
    a.href = url; a.download = 'skill-tree.png'; a.click();
  }

  async function handleSync() {
    setSyncing(true);
    try { await invoke('sync_all_skills'); await loadAll(); }
    catch (e) { console.error('Sync failed:', e); }
    finally { setSyncing(false); }
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
              <div className="flex items-center justify-between mb-3">
                <h1 className="text-sm font-semibold flex items-center gap-2 text-slate-200">
                  <Sparkles className="w-3.5 h-3.5 text-emerald-400" />
                  Skills
                </h1>
                <div className="flex items-center gap-1">
                  <button onClick={handleExport} disabled={skills.length === 0} className="p-1.5 text-slate-700 hover:text-slate-400 disabled:opacity-30 transition-colors" title="Export as PNG">
                    <Download className="w-3 h-3" />
                  </button>
                  <button onClick={() => setSidebarCollapsed(true)} className="p-1.5 text-slate-700 hover:text-slate-400 transition-colors" title="Collapse sidebar">
                    <PanelLeftClose className="w-3 h-3" />
                  </button>
                </div>
              </div>

              <button
                onClick={handleSync}
                disabled={syncing}
                className="w-full py-2 text-xs font-medium rounded-lg transition-colors flex items-center justify-center gap-2"
                style={{ background: syncing ? 'rgba(255,255,255,0.04)' : 'rgba(16,185,129,0.18)', color: syncing ? '#334155' : '#34d399', border: '1px solid rgba(16,185,129,0.25)' }}
              >
                {syncing ? <><Loader2 className="w-3 h-3 animate-spin" />Syncing…</> : <><RefreshCw className="w-3 h-3" />Sync All Skills</>}
              </button>

              {skills.length >= 2 && (
                <button
                  onClick={handleInferDeps}
                  disabled={inferring}
                  className="w-full mt-1.5 py-1.5 text-xs rounded-lg transition-colors flex items-center justify-center gap-2"
                  style={{ background: 'rgba(255,255,255,0.03)', color: '#475569', border: '1px solid rgba(255,255,255,0.05)' }}
                >
                  {inferring ? <><Loader2 className="w-3 h-3 animate-spin" />Inferring…</> : <><Wand2 className="w-3 h-3" />Infer Dependencies</>}
                </button>
              )}

              {skills.length >= 2 && (
                <button
                  onClick={() => setShowMerge(true)}
                  className="w-full mt-1.5 py-1.5 text-xs rounded-lg transition-colors flex items-center justify-center gap-2"
                  style={{ background: 'rgba(255,255,255,0.03)', color: '#475569', border: '1px solid rgba(255,255,255,0.05)' }}
                >
                  <GitMerge className="w-3 h-3" />Merge Skills
                </button>
              )}

              <div className="flex gap-1.5 mt-1.5">
                <button
                  onClick={handleClassifyDomains}
                  disabled={classifying || resetting || skills.length === 0}
                  className="flex-1 py-1.5 text-xs rounded-lg transition-colors flex items-center justify-center gap-1.5"
                  style={{ background: 'rgba(255,255,255,0.03)', color: classifying ? '#334155' : '#818cf8', border: '1px solid rgba(99,102,241,0.15)' }}
                >
                  {classifying ? <><Loader2 className="w-3 h-3 animate-spin" />Classifying…</> : <><Sparkles className="w-3 h-3" />Classify</>}
                </button>
                <button
                  onClick={handleResetDomains}
                  disabled={resetting || classifying || skills.length === 0}
                  className="py-1.5 px-2.5 text-xs rounded-lg transition-colors flex items-center justify-center"
                  style={{ background: 'rgba(255,255,255,0.03)', color: resetting ? '#334155' : '#475569', border: '1px solid rgba(255,255,255,0.06)' }}
                  title="Reset all domain assignments"
                >
                  {resetting ? <Loader2 className="w-3 h-3 animate-spin" /> : <RotateCcw className="w-3 h-3" />}
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
        {!sidebarCollapsed && reviewCount > 0 && (
          <div style={{ padding: '6px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <span className="text-[10px] text-orange-500/80 flex items-center gap-1.5">
              <span className="w-1.5 h-1.5 rounded-full bg-orange-500 flex-shrink-0" />{reviewCount} to review
            </span>
            <button
              onClick={() => setShowReviewOnly(!showReviewOnly)}
              className="text-[9px] px-2 py-0.5 rounded transition-colors"
              style={{ background: showReviewOnly ? 'rgba(249,115,22,0.12)' : 'rgba(255,255,255,0.04)', color: showReviewOnly ? '#f97316' : '#334155' }}
            >
              {showReviewOnly ? 'Filtering' : 'Show'}
            </button>
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
          onMouseLeave={() => { isDragging.current = false; dragMoved.current = false; setHoveredId(null); }}
        />

        {/* 'YOU' label */}
        {skills.length > 0 && (
          <div
            style={{
              position: 'absolute',
              left: `${size.w * 0.50 + transform.current.x}px`,
              top: `${size.h * 0.55 + transform.current.y - 28 * (Math.min(size.w, size.h) / 900) * transform.current.scale}px`,
              transform: 'translateX(-50%)',
              fontSize: 8,
              fontWeight: 700,
              color: '#34d399',
              letterSpacing: '0.18em',
              textTransform: 'uppercase',
              pointerEvents: 'none',
              opacity: Math.min(1, transform.current.scale * 1.4),
            }}
          >
            you
          </div>
        )}

        {/* Empty state */}
        {skills.length === 0 && !loading && (
          <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', pointerEvents: 'none' }}>
            <Sparkles size={28} style={{ marginBottom: 12, color: '#1e293b' }} />
            <div style={{ fontSize: 13, color: '#1e293b' }}>Sync your skills to grow the tree</div>
          </div>
        )}

        {/* Skill detail panel */}
        {(selectedSkill || selectedGap) && (
          <SkillPanel
            skill={selectedSkill}
            gap={selectedGap}
            aliases={aliases}
            deps={deps}
            skills={skills}
            onClose={() => setSelectedId(null)}
          />
        )}
      </div>

      {showMerge && (
        <MergeModal skills={skills} onClose={() => setShowMerge(false)} onMerged={loadAll} />
      )}
    </div>
  );
}
