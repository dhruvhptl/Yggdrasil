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
import { z } from 'zod';

// ─── Constants ────────────────────────────────────────────────────────────────

const PHASE_COLORS = [
  '#10b981', // emerald
  '#4a9eff', // blue
  '#a855f7', // purple
  '#ef4444', // red
  '#f59e0b', // amber
  '#14b8a6', // teal
  '#ec4899', // pink
  '#84cc16', // lime
];

const LEVEL_LABELS = ['', 'Aware', 'Familiar', 'Proficient', 'Advanced', 'Expert'];
const GAP_COLOR = '#f59e0b';

const LEAF = {
  size: 13,
  width: 0.45,
  pointiness: 0.3,
  curve: 0.15,
  glowRadius: 11,
  lockedOpacity: 0.30,
  buddingPulseSpeed: 2.0,
  buddingPulseMin: 0.50,
  buddingPulseMax: 0.85,
  progressRingRadius: 17,
  progressRingWidth: 1.6,
  sparkleCount: 5,
  sparkleRadius: 19,
  sparkleDotSize: 1.2,
};

// ─── Types ────────────────────────────────────────────────────────────────────

interface SkillPlacement {
  skill: UniversalSkill | null;
  gap: SkillGap | null;
  x: number;
  y: number;
  angle: number;
  color: string;
  domainIndex: number;
}

interface BranchSegment {
  x1: number; y1: number;
  x2: number; y2: number;
  cpx: number; cpy: number;   // quadratic control point for Bezier curve
  w1: number; w2: number;
  color: string;
  depth: number;
}

interface LayoutResult {
  placements: SkillPlacement[];
  branches: BranchSegment[];
}

// ─── Seeded pseudo-random ─────────────────────────────────────────────────────

function seededRand(seed: number): () => number {
  let s = seed;
  return () => {
    s = (s * 1664525 + 1013904223) & 0xffffffff;
    return (s >>> 0) / 0xffffffff;
  };
}

// ─── Layout — true organic tree ───────────────────────────────────────────────

function layoutSkillTree(
  skills: UniversalSkill[],
  gaps: SkillGap[],
  showGaps: boolean,
  w: number,
  h: number,
): LayoutResult {
  const branches: BranchSegment[] = [];
  const placements: SkillPlacement[] = [];

  const scale = Math.min(w, h) / 900;
  const cx = w * 0.50;
  const cy = h * 0.55;   // root slightly below center so crown fills upward

  // Group skills by resolved domain name (from JOIN with skill_domains in Rust).
  // s.domain is now the joined domain name, not the raw UUID domain_id field.
  const domainMap = new Map<string, UniversalSkill[]>();
  for (const s of skills) {
    const d = s.domain || 'General';
    if (!domainMap.has(d)) domainMap.set(d, []);
    domainMap.get(d)!.push(s);
  }

  const domainNames = [...domainMap.keys()];
  const domainCount = domainNames.length || 1;

  console.log('[SkillsPage] domain groups:', domainNames.map(d => `${d}(${domainMap.get(d)!.length})`).join(', '));

  // ── Trunk ────────────────────────────────────────────────────────────────
  const trunkHeight = 80 * scale;
  const trunkTop = { x: cx, y: cy - trunkHeight };
  branches.push({
    x1: cx, y1: cy,
    x2: trunkTop.x, y2: trunkTop.y,
    cpx: cx, cpy: cy - trunkHeight * 0.5,
    w1: 22 * scale, w2: 14 * scale,
    color: '#4a2e14', depth: 0,
  });

  // ── Domain boughs ─────────────────────────────────────────────────────────
  // Distribute main boughs in a tree-like spread:
  // Up to 2 boughs go upward (±20° from vertical), rest fan left/right at steeper angles.
  // This avoids the "circle" look and creates an organic canopy shape.

  // Angles from straight-up (-π/2): small angles = near vertical (crown top), large = horizontal (wide)
  // For N domains, spread from -150° to +150° (top arc), avoiding straight down
  const arcTotal = Math.min(300, 50 + domainCount * 40); // degrees
  const arcStart = -90 - arcTotal / 2; // degrees from horizontal

  const rand = seededRand(42);

  domainNames.forEach((domainName, dIdx) => {
    const domainSkills = domainMap.get(domainName)!;
    const color = PHASE_COLORS[dIdx % PHASE_COLORS.length];

    // Bough spine angle
    const angleFrac = domainCount === 1 ? 0.5 : dIdx / (domainCount - 1);
    const spineAngleDeg = arcStart + angleFrac * arcTotal;
    const spineAngle = (spineAngleDeg * Math.PI) / 180;

    // Bough length: longer in middle, shorter at extremes
    const centeredness = 1 - Math.abs(angleFrac - 0.5) * 2; // 0 at edge, 1 at center
    const boughLen = (100 + centeredness * 60) * scale;

    // Bough end — from trunk top
    const boughEnd = {
      x: trunkTop.x + boughLen * Math.cos(spineAngle),
      y: trunkTop.y + boughLen * Math.sin(spineAngle),
    };

    // Control point: slight S-curve (boughs curve away from trunk center)
    const cpBias = (dIdx < domainCount / 2 ? -1 : 1) * 0.25;
    const perpAngle = spineAngle + Math.PI / 2;
    const boughCpx = (trunkTop.x + boughEnd.x) / 2 + cpBias * boughLen * Math.cos(perpAngle);
    const boughCpy = (trunkTop.y + boughEnd.y) / 2 + cpBias * boughLen * Math.sin(perpAngle);

    branches.push({
      x1: trunkTop.x, y1: trunkTop.y,
      x2: boughEnd.x, y2: boughEnd.y,
      cpx: boughCpx, cpy: boughCpy,
      w1: 12 * scale, w2: 7 * scale,
      color, depth: 1,
    });

    // ── Limb & twig branches for each skill ─────────────────────────────
    const sorted = [...domainSkills].sort((a, b) => a.level - b.level);
    const skillCount = sorted.length;

    // Fan from bough: skills branch perpendicular to spine
    const fanMaxDeg = Math.min(70, 15 + skillCount * 8);
    const perLimb = Math.max(2, Math.ceil(skillCount / 4));

    sorted.forEach((skill, sIdx) => {
      const limb = Math.floor(sIdx / perLimb);
      const pos = sIdx % perLimb;
      const limbCount = Math.ceil(skillCount / perLimb);

      // Limb anchor along bough
      const limbT = limbCount <= 1 ? 0.5 : 0.25 + (limb / (limbCount - 1)) * 0.65;
      const limbAnchorX = trunkTop.x + (boughEnd.x - trunkTop.x) * limbT;
      const limbAnchorY = trunkTop.y + (boughEnd.y - trunkTop.y) * limbT;

      // Limb angle: fan perpendicular to bough with slight jitter
      const perCount = Math.min(perLimb, skillCount - limb * perLimb);
      const fanFrac = perCount <= 1 ? 0.5 : pos / (perCount - 1);
      const fanOffDeg = (fanFrac - 0.5) * fanMaxDeg * (1 + 0.2 * (rand() - 0.5));
      const limbAngle = spineAngle + ((fanOffDeg + (rand() - 0.5) * 5) * Math.PI / 180);

      // Limb length: outer limbs shorter
      const limbLen = (55 + centeredness * 30 - Math.abs(fanFrac - 0.5) * 25 + skill.level * 6) * scale;

      const leafPos = {
        x: limbAnchorX + limbLen * Math.cos(limbAngle),
        y: limbAnchorY + limbLen * Math.sin(limbAngle),
      };

      // Twig control point (natural curve away from bough direction)
      const twigCpx = (limbAnchorX + leafPos.x) / 2 + (rand() - 0.5) * 20 * scale * Math.cos(perpAngle);
      const twigCpy = (limbAnchorY + leafPos.y) / 2 + (rand() - 0.5) * 20 * scale * Math.sin(perpAngle);

      branches.push({
        x1: limbAnchorX, y1: limbAnchorY,
        x2: leafPos.x, y2: leafPos.y,
        cpx: twigCpx, cpy: twigCpy,
        w1: 4 * scale, w2: 1.2 * scale,
        color, depth: 2,
      });

      // Leaf angle: pointing outward from center
      const leafAngle = Math.atan2(leafPos.y - cy, leafPos.x - cx);

      placements.push({
        skill,
        gap: null,
        x: leafPos.x,
        y: leafPos.y,
        angle: leafAngle - Math.PI / 2,
        color,
        domainIndex: dIdx,
      });
    });

    // Small decorative twigs on bough
    for (let t = 0; t < 4; t++) {
      const tT = 0.15 + t * 0.22;
      const tAx = trunkTop.x + (boughEnd.x - trunkTop.x) * tT;
      const tAy = trunkTop.y + (boughEnd.y - trunkTop.y) * tT;
      const tAng = spineAngle + ((t % 2 === 0 ? 1 : -1) * (25 + rand() * 20) * Math.PI / 180);
      const tLen = (18 + rand() * 14) * scale;
      branches.push({
        x1: tAx, y1: tAy,
        x2: tAx + tLen * Math.cos(tAng),
        y2: tAy + tLen * Math.sin(tAng),
        cpx: tAx + tLen * 0.5 * Math.cos(tAng + 0.2),
        cpy: tAy + tLen * 0.5 * Math.sin(tAng + 0.2),
        w1: 2 * scale, w2: 0.4 * scale,
        color: '#5c3820', depth: 3,
      });
    }
  });

  // Atmospheric roots
  for (let r = 0; r < 5; r++) {
    const rootAng = (Math.PI / 6) + (r / 4) * (Math.PI * 2 / 3) - Math.PI / 3 + Math.PI / 2;
    const rootLen = (40 + r * 12) * scale;
    branches.push({
      x1: cx, y1: cy,
      x2: cx + rootLen * Math.cos(rootAng),
      y2: cy + rootLen * Math.sin(rootAng),
      cpx: cx + rootLen * 0.5 * Math.cos(rootAng + 0.3),
      cpy: cy + rootLen * 0.5 * Math.sin(rootAng + 0.3),
      w1: 8 * scale, w2: 1 * scale,
      color: '#2e1c0c', depth: 0,
    });
  }

  // Gap nodes — outer ring
  if (showGaps && gaps.length > 0) {
    const existingNames = new Set(skills.map(s => s.name.toLowerCase()));
    const visibleGaps = gaps.filter(g => !(existingNames.has(g.skillName.toLowerCase()) && g.currentLevel > 0));
    const gapR = 380 * scale;
    visibleGaps.forEach((gap, i) => {
      const angle = ((i / visibleGaps.length) * 360 - 90) * Math.PI / 180;
      const pos = { x: cx + gapR * Math.cos(angle), y: cy + gapR * Math.sin(angle) };
      branches.push({
        x1: cx, y1: cy, x2: pos.x, y2: pos.y,
        cpx: (cx + pos.x) / 2, cpy: (cy + pos.y) / 2,
        w1: 1 * scale, w2: 0.3 * scale,
        color: GAP_COLOR + '30', depth: 4,
      });
      placements.push({
        skill: null, gap, x: pos.x, y: pos.y,
        angle: angle - Math.PI / 2,
        color: GAP_COLOR, domainIndex: -1,
      });
    });
  }

  return { placements, branches };
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

// Draw a quadratic-bezier tapered branch (organic S-curve shape)
function taperedBezierBranch(
  ctx: CanvasRenderingContext2D,
  x1: number, y1: number,
  cpx: number, cpy: number,
  x2: number, y2: number,
  w1: number, w2: number,
) {
  if (w1 < 0.2 && w2 < 0.2) return;
  // Approximate offset normals at start, control, and end
  const dxS = cpx - x1, dyS = cpy - y1;
  const lenS = Math.hypot(dxS, dyS) || 1;
  const nxS = -dyS / lenS, nyS = dxS / lenS;
  const dxE = x2 - cpx, dyE = y2 - cpy;
  const lenE = Math.hypot(dxE, dyE) || 1;
  const nxE = -dyE / lenE, nyE = dxE / lenE;

  const hw1 = w1 / 2, hw2 = w2 / 2;
  const hwM = (hw1 + hw2) / 2;

  ctx.beginPath();
  ctx.moveTo(x1 + hw1 * nxS, y1 + hw1 * nyS);
  ctx.quadraticCurveTo(cpx + hwM * nxS, cpy + hwM * nyS, x2 + hw2 * nxE, y2 + hw2 * nyE);
  ctx.lineTo(x2 - hw2 * nxE, y2 - hw2 * nyE);
  ctx.quadraticCurveTo(cpx - hwM * nxS, cpy - hwM * nyS, x1 - hw1 * nxS, y1 - hw1 * nyS);
  ctx.closePath();
}

function drawBranches(ctx: CanvasRenderingContext2D, branches: BranchSegment[]) {
  const sorted = [...branches].sort((a, b) => a.depth - b.depth);
  sorted.forEach(b => {
    taperedBezierBranch(ctx, b.x1, b.y1, b.cpx, b.cpy, b.x2, b.y2, b.w1, b.w2);
    const c = hexToRgb(b.color);
    const grad = ctx.createLinearGradient(b.x1, b.y1, b.x2, b.y2);
    if (b.depth === 0) {
      // trunk & roots: rich bark gradient
      grad.addColorStop(0, '#5a3620');
      grad.addColorStop(0.5, '#3d2410');
      grad.addColorStop(1, '#1e0f05');
    } else if (b.depth === 1) {
      // main boughs: domain-colored, semi-transparent
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.65)`);
      grad.addColorStop(0.6, `rgba(${c.r},${c.g},${c.b},0.35)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.18)`);
    } else if (b.depth === 2) {
      // limbs: thinner, more transparent
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.40)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.10)`);
    } else {
      // decorative twigs
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.20)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.04)`);
    }
    ctx.fillStyle = grad;
    ctx.fill();
  });
}

// Seeded starfield (stable across renders)
let _starCache: { x: number; y: number; r: number; a: number; twinklePhase: number }[] | null = null;
function getStars(w: number, h: number, count = 280) {
  if (_starCache && _starCache.length === count) return _starCache;
  const rng = seededRand(7331);
  _starCache = Array.from({ length: count }, () => ({
    x: rng() * w, y: rng() * h,
    r: 0.4 + rng() * 1.4,
    a: 0.3 + rng() * 0.7,
    twinklePhase: rng() * Math.PI * 2,
  }));
  return _starCache;
}

function drawBackground(ctx: CanvasRenderingContext2D, w: number, h: number, animTime: number, domainColors: string[]) {
  // Deep space
  const skyGrad = ctx.createLinearGradient(0, 0, 0, h);
  skyGrad.addColorStop(0, '#01020a');
  skyGrad.addColorStop(0.35, '#020408');
  skyGrad.addColorStop(1, '#030608');
  ctx.fillStyle = skyGrad;
  ctx.fillRect(0, 0, w, h);

  // Nebula clouds per domain
  const cx = w * 0.5, cy = h * 0.55;
  domainColors.forEach((color, i) => {
    const c = hexToRgb(color);
    const nebAngle = ((i / (domainColors.length || 1)) * Math.PI * 2) - Math.PI / 2;
    const nebR = Math.min(w, h) * 0.28;
    const nbx = cx + nebR * Math.cos(nebAngle);
    const nby = cy + nebR * Math.sin(nebAngle);
    const nebGlow = ctx.createRadialGradient(nbx, nby, 0, nbx, nby, Math.min(w, h) * 0.22);
    nebGlow.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.055)`);
    nebGlow.addColorStop(0.4, `rgba(${c.r},${c.g},${c.b},0.022)`);
    nebGlow.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0)`);
    ctx.fillStyle = nebGlow;
    ctx.fillRect(0, 0, w, h);
  });

  // Central origin glow
  const originGlow = ctx.createRadialGradient(cx, cy, 0, cx, cy, Math.min(w, h) * 0.15);
  originGlow.addColorStop(0, 'rgba(16,185,129,0.06)');
  originGlow.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = originGlow;
  ctx.fillRect(0, 0, w, h);

  // Stars
  const stars = getStars(w, h, 320);
  stars.forEach(star => {
    const twinkle = star.a * (0.6 + 0.4 * Math.sin(animTime * 0.8 + star.twinklePhase));
    ctx.beginPath();
    ctx.arc(star.x * (w / w), star.y * (h / h), star.r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(220,235,255,${twinkle})`;
    ctx.fill();
  });

  // Milky way band — faint diagonal haze
  const mwGrad = ctx.createLinearGradient(0, h * 0.1, w, h * 0.9);
  mwGrad.addColorStop(0, 'rgba(100,120,180,0)');
  mwGrad.addColorStop(0.3, 'rgba(80,100,160,0.018)');
  mwGrad.addColorStop(0.5, 'rgba(100,120,200,0.035)');
  mwGrad.addColorStop(0.7, 'rgba(80,100,160,0.018)');
  mwGrad.addColorStop(1, 'rgba(100,120,180,0)');
  ctx.fillStyle = mwGrad;
  ctx.fillRect(0, 0, w, h);
}

function drawCenterNode(ctx: CanvasRenderingContext2D, cx: number, cy: number, animTime: number, scale: number) {
  const pulse = 0.5 + 0.5 * Math.sin(animTime * 1.1);
  const outerR = (24 + pulse * 5) * scale;

  // Outer halo
  const halo = ctx.createRadialGradient(cx, cy, 0, cx, cy, outerR * 2.6);
  halo.addColorStop(0, `rgba(16,185,129,${0.14 + pulse * 0.07})`);
  halo.addColorStop(0.5, `rgba(16,185,129,0.03)`);
  halo.addColorStop(1, 'rgba(16,185,129,0)');
  ctx.beginPath();
  ctx.arc(cx, cy, outerR * 2.6, 0, Math.PI * 2);
  ctx.fillStyle = halo;
  ctx.fill();

  // Pulsing ring
  ctx.beginPath();
  ctx.arc(cx, cy, outerR, 0, Math.PI * 2);
  ctx.strokeStyle = `rgba(16,185,129,${0.28 + pulse * 0.20})`;
  ctx.lineWidth = 1.5 * scale;
  ctx.stroke();

  // Root glow where trunk meets ground
  const rootGlow = ctx.createRadialGradient(cx, cy, 0, cx, cy, 36 * scale);
  rootGlow.addColorStop(0, 'rgba(90,54,32,0.55)');
  rootGlow.addColorStop(0.5, 'rgba(60,36,20,0.25)');
  rootGlow.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.beginPath();
  ctx.arc(cx, cy, 36 * scale, 0, Math.PI * 2);
  ctx.fillStyle = rootGlow;
  ctx.fill();

  // Core gem
  const core = ctx.createRadialGradient(cx, cy - 1 * scale, 0, cx, cy, 7 * scale);
  core.addColorStop(0, '#e0fff6');
  core.addColorStop(0.35, '#10b981');
  core.addColorStop(0.75, '#064e3b');
  core.addColorStop(1, '#021a12');
  ctx.beginPath();
  ctx.arc(cx, cy, 7 * scale, 0, Math.PI * 2);
  ctx.fillStyle = core;
  ctx.fill();
}

// ─── Leaf drawing ─────────────────────────────────────────────────────────────

function drawSkillLeaf(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, angle: number,
  size: number, color: string,
  state: 'dormant' | 'budding' | 'growing' | 'bloomed',
  isHovered: boolean, isSelected: boolean,
  animTime: number,
  progress: number,
  isGap: boolean,
) {
  const w = size * LEAF.width;
  const len = size;
  const pointOff = len * LEAF.pointiness;
  const curve = LEAF.curve * size;

  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(angle);

  let fillColor: string, strokeColor: string, alpha: number;

  if (isGap) {
    fillColor = '#1a1a1a';
    strokeColor = GAP_COLOR + '88';
    alpha = 0.55 + 0.2 * Math.sin(animTime * 1.4);
  } else if (state === 'bloomed') {
    fillColor = color;
    strokeColor = '#ffffffcc';
    alpha = 1;
  } else if (state === 'dormant') {
    fillColor = '#1c1c2a';
    strokeColor = '#2e2e44';
    alpha = LEAF.lockedOpacity;
  } else if (state === 'growing') {
    fillColor = color;
    strokeColor = color + 'ff';
    alpha = 1.0;
  } else {
    const pulse = LEAF.buddingPulseMin +
      (LEAF.buddingPulseMax - LEAF.buddingPulseMin) *
      (0.5 + 0.5 * Math.sin(animTime * LEAF.buddingPulseSpeed));
    fillColor = color + 'bb';
    strokeColor = color + 'ff';
    alpha = pulse;
  }

  ctx.globalAlpha = alpha;

  if (state === 'bloomed' && !isGap && LEAF.glowRadius > 0) {
    const halo = ctx.createRadialGradient(len * 0.4, 0, 4, len * 0.4, 0, LEAF.glowRadius + size * 1.4);
    halo.addColorStop(0, color + '55');
    halo.addColorStop(0.5, color + '22');
    halo.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, LEAF.glowRadius + size * 1.4, 0, Math.PI * 2);
    ctx.fillStyle = halo;
    ctx.fill();
    const core = ctx.createRadialGradient(len * 0.4, 0, 1, len * 0.4, 0, size * 0.9);
    core.addColorStop(0, color + 'aa');
    core.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 0.9, 0, Math.PI * 2);
    ctx.fillStyle = core;
    ctx.fill();
  }

  if (state === 'budding' && !isGap) {
    const ringAlpha = Math.round(
      (0.2 + 0.3 * (0.5 + 0.5 * Math.sin(animTime * LEAF.buddingPulseSpeed + Math.PI))) * 255
    ).toString(16).padStart(2, '0');
    const buddingGlow = ctx.createRadialGradient(len * 0.4, 0, size * 0.8, len * 0.4, 0, size * 1.8);
    buddingGlow.addColorStop(0, color + '00');
    buddingGlow.addColorStop(0.5, color + ringAlpha);
    buddingGlow.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 1.8, 0, Math.PI * 2);
    ctx.fillStyle = buddingGlow;
    ctx.fill();
  }

  if (isGap) {
    const ghostGlow = ctx.createRadialGradient(len * 0.4, 0, size * 0.5, len * 0.4, 0, size * 2.0);
    ghostGlow.addColorStop(0, GAP_COLOR + '33');
    ghostGlow.addColorStop(1, GAP_COLOR + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 2.0, 0, Math.PI * 2);
    ctx.fillStyle = ghostGlow;
    ctx.fill();
  }

  if (isHovered) {
    const hg = ctx.createRadialGradient(len * 0.4, 0, 2, len * 0.4, 0, size * 2);
    hg.addColorStop(0, color + '55');
    hg.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 2, 0, Math.PI * 2);
    ctx.fillStyle = hg;
    ctx.fill();
  }

  if (isSelected) {
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 1.3, 0, Math.PI * 2);
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.globalAlpha = 0.8;
    ctx.stroke();
    ctx.globalAlpha = alpha;
  }

  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.bezierCurveTo(pointOff * 0.3, -(w * 0.5 + curve), pointOff, -(w + curve * 0.5), len, 0);
  ctx.bezierCurveTo(pointOff, (w + curve * 0.5), pointOff * 0.3, (w * 0.5 + curve), 0, 0);
  ctx.closePath();
  ctx.fillStyle = fillColor;
  ctx.fill();

  if (isGap) {
    ctx.setLineDash([3, 2]);
    ctx.strokeStyle = GAP_COLOR + 'aa';
    ctx.lineWidth = 0.8;
    ctx.stroke();
    ctx.setLineDash([]);
  } else {
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth = isHovered ? 1.8 : 0.8;
    ctx.stroke();
  }

  if (state === 'growing' && progress > 0 && !isGap) {
    const rcx = len * 0.4;
    const r = LEAF.progressRingRadius;
    const startAngle = -Math.PI / 2;
    const endAngle = startAngle + (progress / 100) * Math.PI * 2;
    ctx.beginPath();
    ctx.arc(rcx, 0, r, 0, Math.PI * 2);
    ctx.strokeStyle = color + '33';
    ctx.lineWidth = LEAF.progressRingWidth;
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rcx, 0, r, startAngle, endAngle);
    ctx.strokeStyle = color + 'ee';
    ctx.lineWidth = LEAF.progressRingWidth;
    ctx.lineCap = 'round';
    ctx.stroke();
    ctx.lineCap = 'butt';
  }

  // Veins
  ctx.beginPath();
  ctx.moveTo(1, 0);
  ctx.lineTo(len * 0.85, 0);
  ctx.strokeStyle = isGap ? GAP_COLOR + '22'
    : state === 'dormant' ? '#2a2a3a'
    : state === 'bloomed' ? '#ffffff44'
    : color + '55';
  ctx.lineWidth = 0.6;
  ctx.stroke();

  const veinCount = Math.max(2, Math.floor(size / 5));
  for (let v = 1; v <= veinCount; v++) {
    const t = v / (veinCount + 1);
    const vx = len * t * 0.85;
    const vw = w * (1 - t * 0.6) * 0.6;
    const veinStroke = isGap ? GAP_COLOR + '18'
      : state === 'dormant' ? '#22223a'
      : state === 'bloomed' ? '#ffffff22'
      : color + '33';
    ctx.beginPath();
    ctx.moveTo(vx, 0);
    ctx.quadraticCurveTo(vx + len * 0.06, -vw * 0.6, vx + len * 0.12, -vw);
    ctx.strokeStyle = veinStroke;
    ctx.lineWidth = 0.4;
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(vx, 0);
    ctx.quadraticCurveTo(vx + len * 0.06, vw * 0.6, vx + len * 0.12, vw);
    ctx.stroke();
  }

  if (state === 'bloomed' && !isGap) {
    const scx = len * 0.4;
    const baseAngle = animTime * 0.6;
    const savedAlpha = ctx.globalAlpha;
    for (let i = 0; i < LEAF.sparkleCount; i++) {
      const theta = baseAngle + (i / LEAF.sparkleCount) * Math.PI * 2;
      const dist = LEAF.sparkleRadius * (0.7 + 0.3 * (Math.sin(animTime * 1.3 + i * 2.1) * 0.5 + 0.5));
      const sx = scx + Math.cos(theta) * dist;
      const sy = Math.sin(theta) * dist * 0.6;
      const sa = 0.4 + 0.4 * (Math.sin(animTime * 2.7 + i * 1.8) * 0.5 + 0.5);
      ctx.beginPath();
      ctx.arc(sx, sy, LEAF.sparkleDotSize, 0, Math.PI * 2);
      ctx.fillStyle = '#ffffff';
      ctx.globalAlpha = sa;
      ctx.fill();
    }
    ctx.globalAlpha = savedAlpha;
  }

  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(-size * 0.35, 0);
  ctx.strokeStyle = '#1a1008';
  ctx.lineWidth = 1.0;
  ctx.stroke();

  ctx.globalAlpha = 1;
  ctx.restore();
}

function drawAllLeaves(
  ctx: CanvasRenderingContext2D,
  placements: SkillPlacement[],
  selectedId: string | null,
  hoveredId: string | null,
  centerX: number,
  animTime: number,
) {
  placements.forEach(p => {
    const isGap = p.skill === null;
    const id = isGap ? (p.gap?.skillName ?? '') : p.skill!.id;
    const isSelected = id === selectedId;
    const isHovered = id === hoveredId;

    let state: 'dormant' | 'budding' | 'growing' | 'bloomed' = 'dormant';
    let progress = 0;

    if (!isGap && p.skill) {
      const lvl = p.skill.level;
      if (lvl === 0) state = 'dormant';
      else if (lvl === 1) state = 'budding';
      else if (lvl >= 2 && lvl < 4) { state = 'growing'; progress = (lvl / 4) * 100; }
      else state = 'bloomed';
    }

    drawSkillLeaf(ctx, p.x, p.y, p.angle, LEAF.size, p.color, state, isHovered, isSelected, animTime, progress, isGap);

    if (isHovered || isSelected) {
      const label = isGap
        ? (p.gap?.skillName ?? '').slice(0, 28)
        : (p.skill!.name.length > 28 ? p.skill!.name.slice(0, 28) + '…' : p.skill!.name);
      ctx.font = '500 10px "DM Sans", system-ui, sans-serif';
      const tw = ctx.measureText(label).width;
      const lx = p.x > centerX ? p.x + LEAF.size + 8 : p.x - LEAF.size - 8 - tw;
      const ly = p.y;
      // Pill background
      ctx.fillStyle = 'rgba(2,6,18,0.82)';
      ctx.beginPath();
      ctx.roundRect(lx - 4, ly - 7, tw + 8, 14, 4);
      ctx.fill();
      ctx.fillStyle = isGap ? GAP_COLOR : '#e2e8f0';
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      ctx.fillText(label, lx, ly);
      ctx.textAlign = 'left';
    }
  });
}

// Draw floating domain labels on the canvas
function drawDomainLabels(
  ctx: CanvasRenderingContext2D,
  _skills: UniversalSkill[],
  placements: SkillPlacement[],
  animTime: number,
) {
  const domainMap = new Map<number, { color: string; name: string; xs: number[]; ys: number[] }>();
  placements.forEach(p => {
    if (p.skill === null || p.domainIndex < 0) return;
    if (!domainMap.has(p.domainIndex)) {
      domainMap.set(p.domainIndex, { color: p.color, name: p.skill.domain || 'General', xs: [], ys: [] });
    }
    domainMap.get(p.domainIndex)!.xs.push(p.x);
    domainMap.get(p.domainIndex)!.ys.push(p.y);
  });

  domainMap.forEach(({ color, name, xs, ys }) => {
    if (xs.length === 0) return;
    const cx = xs.reduce((a, b) => a + b, 0) / xs.length;
    const cy = ys.reduce((a, b) => a + b, 0) / ys.length;
    const floatY = cy + Math.sin(animTime * 0.4 + name.length) * 3;

    const c = hexToRgb(color);
    ctx.font = '600 9px "DM Sans", system-ui, sans-serif';
    ctx.letterSpacing = '0.1em';
    const tw = ctx.measureText(name.toUpperCase()).width;

    ctx.fillStyle = `rgba(${c.r},${c.g},${c.b},0.12)`;
    ctx.beginPath();
    ctx.roundRect(cx - tw / 2 - 5, floatY - 8, tw + 10, 14, 3);
    ctx.fill();

    ctx.fillStyle = `rgba(${c.r},${c.g},${c.b},0.65)`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(name.toUpperCase(), cx, floatY);
    ctx.textAlign = 'left';
    ctx.letterSpacing = '0';
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
    case 'resume': return <FileText className="w-3 h-3 text-blue-400 flex-shrink-0" />;
    case 'tree_quest': return <TreePine className="w-3 h-3 text-emerald-400 flex-shrink-0" />;
    case 'work_resource': return <Briefcase className="w-3 h-3 text-violet-400 flex-shrink-0" />;
    default: return <Sparkles className="w-3 h-3 text-slate-400 flex-shrink-0" />;
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
  const skillById = new Map(skills.map(s => [s.id, s]));
  const prereqs = skill
    ? deps.filter(d => d.targetSkillId === skill.id).map(d => skillById.get(d.sourceSkillId)).filter(Boolean) as UniversalSkill[]
    : [];
  const unlocks = skill
    ? deps.filter(d => d.sourceSkillId === skill.id).map(d => skillById.get(d.targetSkillId)).filter(Boolean) as UniversalSkill[]
    : [];

  const panelColor = gap && !skill ? GAP_COLOR : (skill ? (PHASE_COLORS[0]) : '#475569');

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
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [canonicalId, setCanonicalId] = useState<string | null>(null);
  const [merging, setMerging] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const canonical = skills.find(s => s.id === canonicalId);
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
            const isSel = selected.has(skill.id);
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

// ─── Main Page ────────────────────────────────────────────────────────────────

export default function SkillsPage() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const [skills, setSkills] = useState<UniversalSkill[]>([]);
  const [gaps, setGaps] = useState<SkillGap[]>([]);
  const [deps, setDeps] = useState<SkillDependency[]>([]);
  const [aliases, setAliases] = useState<SkillAlias[]>([]);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [inferring, setInferring] = useState(false);
  const [classifying, setClassifying] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [classifyToast, setClassifyToast] = useState<string | null>(null);
  const [showGaps, setShowGaps] = useState(true);
  const [showReviewOnly, setShowReviewOnly] = useState(false);
  const [showMerge, setShowMerge] = useState(false);
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [size, setSize] = useState({ w: 1200, h: 900 });

  const transform = useRef({ x: 0, y: 0, scale: 1 });
  const isDragging = useRef(false);
  const lastMouse = useRef({ x: 0, y: 0 });
  const dragMoved = useRef(false);
  const [renderTick, setRenderTick] = useState(0);
  const triggerRender = useCallback(() => setRenderTick(t => t + 1), []);

  const { placements, branches } = useMemo(
    () => layoutSkillTree(skills, gaps, showGaps, size.w, size.h),
    [skills, gaps, showGaps, size.w, size.h],
  );

  const hasAnimatedNodes = useMemo(() => skills.length > 0 || (showGaps && gaps.length > 0), [skills, gaps, showGaps]);

  const domainColors = useMemo(() => {
    const names = [...new Set(skills.map(s => s.domain || 'General'))];
    return names.map((_, i) => PHASE_COLORS[i % PHASE_COLORS.length]);
  }, [skills]);

  const aliasCountMap = useMemo(() => {
    const m = new Map<string, number>();
    for (const a of aliases) m.set(a.canonicalSkillId, (m.get(a.canonicalSkillId) ?? 0) + 1);
    return m;
  }, [aliases]);

  const selectedSkill = useMemo(() => selectedId ? skills.find(s => s.id === selectedId) ?? null : null, [selectedId, skills]);
  const selectedGap = useMemo(() => {
    if (!selectedId) return null;
    const gapName = selectedId.startsWith('gap-') ? selectedId.slice(4) : null;
    return gapName ? gaps.find(g => g.skillName === gapName) ?? null : null;
  }, [selectedId, gaps]);

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

  const avgLevel = skills.length > 0
    ? (skills.reduce((sum, s) => sum + s.level, 0) / skills.length).toFixed(1) : '0';
  const levelCounts = [0, 0, 0, 0, 0, 0];
  for (const s of skills) levelCounts[s.level]++;
  const reviewCount = skills.filter(s => s.reviewNeeded).length;

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

  // Canvas render
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.w * dpr;
    canvas.height = size.h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const animTime = performance.now() / 1000;
    const scale = Math.min(size.w, size.h) / 900;
    const cx = size.w * 0.50;
    const cy = size.h * 0.55;

    drawBackground(ctx, size.w, size.h, animTime, domainColors);

    const t = transform.current;
    ctx.save();
    ctx.translate(t.x, t.y);
    ctx.scale(t.scale, t.scale);

    drawBranches(ctx, branches);
    drawCenterNode(ctx, cx, cy, animTime, scale);
    drawAllLeaves(ctx, placements, selectedId, hoveredId, cx, animTime);
    if (skills.length > 0) drawDomainLabels(ctx, skills, placements, animTime);

    ctx.restore();
  }, [branches, placements, selectedId, hoveredId, size, renderTick, domainColors, skills]);

  useEffect(() => {
    if (!hasAnimatedNodes) return;
    let rafId: number;
    const tick = () => { triggerRender(); rafId = requestAnimationFrame(tick); };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [hasAnimatedNodes, triggerRender]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const t = transform.current;
      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      const delta = e.deltaY > 0 ? 0.92 : 1.08;
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
    const hitRadius = LEAF.size * 2.2;
    let closest: SkillPlacement | null = null;
    let closestDist = Infinity;
    for (const p of placements) {
      const dist = Math.hypot(p.x - wx, p.y - wy);
      if (dist < hitRadius && dist < closestDist) { closest = p; closestDist = dist; }
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
    const rect = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const p = findPlacement(world.x, world.y);
    setSelectedId(p ? getPlacementId(p) : null);
  }

  function handleMouseDown(e: React.MouseEvent) {
    isDragging.current = true;
    dragMoved.current = false;
    lastMouse.current = { x: e.clientX, y: e.clientY };
  }

  function handleMouseMove(e: React.MouseEvent) {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (isDragging.current) {
      const dx = e.clientX - lastMouse.current.x;
      const dy = e.clientY - lastMouse.current.y;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) dragMoved.current = true;
      transform.current.x += dx;
      transform.current.y += dy;
      lastMouse.current = { x: e.clientX, y: e.clientY };
      triggerRender();
      return;
    }
    const rect = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const p = findPlacement(world.x, world.y);
    const newHovered = p ? getPlacementId(p) : null;
    if (newHovered !== hoveredId) setHoveredId(newHovered);
  }

  function handleMouseUp() { isDragging.current = false; }

  function handleExport() {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const url = canvas.toDataURL('image/png');
    const a = document.createElement('a');
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
    setClassifying(true);
    setClassifyToast(null);
    try {
      const result = await invoke<{ total: number; classified: number; failed: number }>('classify_skill_domains');
      setClassifyToast(`Classified ${result.classified}/${result.total} skills${result.failed > 0 ? ` (${result.failed} failed)` : ''}`);
      await loadAll();
      setTimeout(() => setClassifyToast(null), 4000);
    } catch (e) {
      setClassifyToast(`Error: ${String(e)}`);
      setTimeout(() => setClassifyToast(null), 5000);
    } finally {
      setClassifying(false);
    }
  }

  async function handleResetDomains() {
    setResetting(true);
    setClassifyToast(null);
    try {
      const count = await invoke<number>('reset_skill_domains');
      setClassifyToast(`Reset ${count} skills to unclassified`);
      await loadAll();
      setTimeout(() => setClassifyToast(null), 3000);
    } catch (e) {
      setClassifyToast(`Error: ${String(e)}`);
      setTimeout(() => setClassifyToast(null), 5000);
    } finally {
      setResetting(false);
    }
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
                  <p className="text-[10px] text-slate-800">Sync to import from resume, trees & work.</p>
                </div>
              ) : (
                <div className="space-y-0.5">
                  {[...filteredDomainGroups.entries()].map(([domain, domainSkills]) => {
                    const expanded = expandedDomains.has(domain);
                    const domainColor = PHASE_COLORS[
                      [...(new Map(skills.map(s => [s.domain || 'General', true]))).keys()].indexOf(domain) % PHASE_COLORS.length
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
              top: `${size.h * 0.55 + transform.current.y - 32 * (Math.min(size.w, size.h) / 900) * transform.current.scale}px`,
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
