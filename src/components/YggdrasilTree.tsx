// src/components/YggdrasilTree.tsx
// Procedural L-system skill tree rendered on HTML Canvas.
// Two rendering passes: (1) organic tree branches, (2) node ornaments at tips.
// Dynamic coordinates — tree fills canvas naturally. No pan/zoom transform.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { MimirResource, NeighborNode, NodeNeighborhood, PathNode, Project, Tree } from '../types';
import { useMimirContext } from '../contexts/MimirContext';
import { validateOrLog, TreeNodeSchema } from '../lib/validators';
import { z } from 'zod';

// ─── Types ───────────────────────────────────────────────────────────────────

type TasksPayload = {
  mastery_criteria: string;
  exercises: string[];
  notes: string;
  completed: boolean;
} | null;

interface TreeNode {
  id: string;
  tree_id: string;
  parent_id: string | null;
  type: 'trunk' | 'branch' | 'leaf';
  title: string;
  description: string;
  progress: number; // 0–100
  tasks: TasksPayload;
  resources: unknown[] | null;
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

interface YggdrasilTreeProps {
  projectId: string;
  externalSelectedNodeId?: string | null;
}

// ─── Branch type (tapered filled segments) ───────────────────────────────────

interface Branch {
  x1: number; y1: number;
  x2: number; y2: number;
  w1: number; w2: number;      // start/end widths for tapered fill
  color: string;               // hex fill color
  depth: number;               // 0=trunk, 1=bough, 2=limb, 3=twig, 4=atmospheric
  curveBias: number;           // lateral offset fraction for quadratic CP
  domainColor: string | null;  // domain accent for bough/limb/twig
}

interface Tip {
  x: number;
  y: number;
  angle: number;
  depth: number;
}

interface NodePlacement {
  node: TreeNode;
  x: number;
  y: number;
  angle: number;
  branchAngle: number;
  phaseIndex: number;
  nodeType: 'checkpoint' | 'skill' | 'phase';
}


// ─── Constants ───────────────────────────────────────────────────────────────

const PHASE_COLORS = [
  '#c8a94a',  // amber
  '#4a9eff',  // blue
  '#a855f7',  // purple
  '#ef4444',  // red
  '#10b981',  // emerald
];

// ─── Tree layout ─────────────────────────────────────────────────────────────
// Polar coordinate layout matching the playground design.
// Trunk → domain boughs → skill limbs → checkpoint twigs (all tapered filled).


interface LayoutResult {
  branches: Branch[];
  placements: NodePlacement[];
  unusedTips: Tip[];
}

function layoutTree(nodes: TreeNode[], w: number, h: number): LayoutResult {
  const branches: Branch[] = [];
  const placements: NodePlacement[] = [];

  // ── Playground polar coordinate layout ─────────────────────────────────────
  // Origin: trunk top at cx = W*0.43, trunkTopY = H*0.58.
  // Domain spines at fixed angles from vertical (positive = right).
  // Skills fanned ±20° around each spine at 3 radii (160/280/390 px scaled).
  // Nodes fanned ±16° around each skill arm at cluster + offset.
  // All radii scaled by Math.min(w, h) / 900.

  const scale  = Math.min(w, h) / 900;
  const cx     = w * 0.43;          // trunk center-X (slightly left of center)
  const baseY  = h * 0.97;          // trunk base near bottom
  const trunkTopY = h * 0.58;       // trunk top / domain junction

  // Tapered trunk segments (3 segments for natural taper)
  branches.push({ x1: cx,   y1: baseY,                            x2: cx+8, y2: baseY*0.85+trunkTopY*0.15, w1: 28*scale, w2: 22*scale, color: '#3d2510', depth: 0, curveBias: 0,    domainColor: null });
  branches.push({ x1: cx+8, y1: baseY*0.85+trunkTopY*0.15,       x2: cx+4, y2: baseY*0.55+trunkTopY*0.45, w1: 22*scale, w2: 17*scale, color: '#3a2210', depth: 0, curveBias: 0,    domainColor: null });
  branches.push({ x1: cx+4, y1: baseY*0.55+trunkTopY*0.45,       x2: cx,   y2: trunkTopY,                 w1: 17*scale, w2: 12*scale, color: '#362010', depth: 0, curveBias: 0,    domainColor: null });

  // Trunk side twigs for organic feel
  for (let i = 0; i < 5; i++) {
    const frac = 0.2 + i * 0.15;
    const ty   = trunkTopY + (baseY - trunkTopY) * frac;
    const side = i % 2 === 0 ? 1 : -1;
    const len  = (18 + i * 5) * scale;
    branches.push({
      x1: cx, y1: ty,
      x2: cx + side * len * 0.9, y2: ty - len * 0.45,
      w1: 2.5*scale, w2: 0.7*scale,
      color: '#3d2510', depth: 4, curveBias: side * 0.12, domainColor: null,
    });
  }

  // Collect & group checkpoints
  const checkpoints = collectCheckpoints(nodes);
  if (checkpoints.length === 0) return { branches, placements, unusedTips: [] };

  // Assign a phase color per phase index
  const byPhase = new Map<number, typeof checkpoints>();
  checkpoints.forEach(cp => {
    if (!byPhase.has(cp.phaseIndex)) byPhase.set(cp.phaseIndex, []);
    byPhase.get(cp.phaseIndex)!.push(cp);
  });
  const phaseIndices = [...byPhase.keys()].sort((a, b) => a - b);

  // Up to 5 domain spine angles (degrees from vertical, +right)
  const SPINE_ANGLES_DEG = [-45, 0, 45, -22, 22];
  // Skill cluster radii (base px, scaled): near / mid / far
  const SKILL_RADII = [160, 280, 390].map(r => r * scale);
  // Skill fan: ±20° around spine
  const SKILL_FAN_DEG  = [-20, 0, 20];
  // Node fan: ±16° around each skill arm
  const NODE_FAN_DEG   = [-16, 0, 16];
  const NODE_OFFSET_PX = 52 * scale;   // radial offset from skill cluster to node

  phaseIndices.forEach((phaseIdx, pOrdinal) => {
    const cpList       = byPhase.get(phaseIdx)!;
    const color        = PHASE_COLORS[phaseIdx % PHASE_COLORS.length];
    const spineAngleDeg = SPINE_ANGLES_DEG[pOrdinal] ?? (pOrdinal - 2) * 45;
    const spineAngle   = (spineAngleDeg * Math.PI) / 180;

    // Polar → Cartesian helpers (up = −cos, right = +sin)
    const polar = (r: number, angle: number) => ({
      x: cx + r * Math.sin(angle),
      y: trunkTopY - r * Math.cos(angle),
    });

    // Bough end (short reach from trunk junction)
    const boughR   = 130 * scale;
    const boughEnd = polar(boughR, spineAngle);
    const boughCurveBias = spineAngleDeg < 0 ? -0.1 : spineAngleDeg > 0 ? 0.1 : 0.0;

    branches.push({
      x1: cx, y1: trunkTopY,
      x2: boughEnd.x, y2: boughEnd.y,
      w1: 11*scale, w2: 8*scale,
      color, depth: 1, curveBias: boughCurveBias, domainColor: color,
    });

    // Atmospheric twigs along bough
    for (let t = 0; t < 3; t++) {
      const tFrac = 0.3 + t * 0.25;
      const tx    = cx    + boughR * tFrac * Math.sin(spineAngle);
      const ty    = trunkTopY - boughR * tFrac * Math.cos(spineAngle);
      const tAng  = spineAngle + (t - 1) * 0.25;
      const tLen  = (25 + t * 8) * scale;
      branches.push({
        x1: tx, y1: ty,
        x2: tx + tLen * Math.sin(tAng + 0.3),
        y2: ty - tLen * Math.cos(tAng + 0.3),
        w1: 2*scale, w2: 0.6*scale,
        color: '#5c3820', depth: 4, curveBias: 0.1, domainColor: null,
      });
    }

    // Group checkpoints by skill
    const bySkill = new Map<number, typeof cpList>();
    cpList.forEach(cp => {
      if (!bySkill.has(cp.skillIndex)) bySkill.set(cp.skillIndex, []);
      bySkill.get(cp.skillIndex)!.push(cp);
    });
    const skillIndices = [...bySkill.keys()].sort((a, b) => a - b);

    skillIndices.forEach((skillIdx, sOrdinal) => {
      const cpSubList  = bySkill.get(skillIdx)!;
      const fanDeg     = SKILL_FAN_DEG[sOrdinal] ?? (sOrdinal - 1) * 20;
      const fanRad     = (fanDeg * Math.PI) / 180;
      const skillAngle = spineAngle + fanRad;
      const r          = SKILL_RADII[sOrdinal] ?? SKILL_RADII[SKILL_RADII.length - 1];
      const skillEnd   = polar(r, skillAngle);
      const limbCurveBias = fanDeg < 0 ? -0.12 : fanDeg > 0 ? 0.12 : 0.0;

      // Limb: bough end → skill cluster
      branches.push({
        x1: boughEnd.x, y1: boughEnd.y,
        x2: skillEnd.x, y2: skillEnd.y,
        w1: 7*scale, w2: 4*scale,
        color, depth: 2, curveBias: limbCurveBias, domainColor: color,
      });

      cpSubList.forEach((cp, cpOrdinal) => {
        const nodeFanDeg   = NODE_FAN_DEG[cpOrdinal] ?? (cpOrdinal - 1) * 16;
        const nodeFanRad   = (nodeFanDeg * Math.PI) / 180;
        const nodeAngle    = skillAngle + nodeFanRad;
        const nodeR        = r + NODE_OFFSET_PX + cpOrdinal * 8 * scale;
        const nodePos      = polar(nodeR, nodeAngle);

        // Twig: skill cluster → node
        branches.push({
          x1: skillEnd.x, y1: skillEnd.y,
          x2: nodePos.x,  y2: nodePos.y,
          w1: 3.5*scale, w2: 1.5*scale,
          color, depth: 3, curveBias: (cpOrdinal - 1) * 0.08, domainColor: color,
        });

        // Decorative branch split off twig midpoint
        const midX      = (skillEnd.x + nodePos.x) * 0.5;
        const midY      = (skillEnd.y + nodePos.y) * 0.5;
        branches.push({
          x1: midX, y1: midY,
          x2: midX + 22 * scale * Math.sin(nodeAngle + 0.4),
          y2: midY - 22 * scale * Math.cos(nodeAngle + 0.4),
          w1: 2*scale, w2: 0.8*scale,
          color, depth: 4, curveBias: 0.08, domainColor: null,
        });

        // Canvas rotation angle for drawLeaf (spine direction → canvas convention)
        const leafCanvasAngle = nodeAngle - Math.PI / 2;

        placements.push({
          node: cp.checkpoint,
          x: nodePos.x,
          y: nodePos.y,
          angle: leafCanvasAngle,
          branchAngle: leafCanvasAngle,
          phaseIndex: phaseIdx,
          nodeType: 'checkpoint',
        });
      });
    });
  });

  return { branches, placements, unusedTips: [] };
}


// ─── Checkpoint collection — phase → skill → checkpoint order ────────────────
// Handles any tree depth: trunk → phases → skills → checkpoints
// Also handles 2-level trees where phases have leaves directly

function collectCheckpoints(nodes: TreeNode[]): {
  checkpoint: TreeNode;
  skillId: string | null;
  skillIndex: number;
  phaseIndex: number;
}[] {
  const childrenOf = new Map<string | null, TreeNode[]>();
  nodes.forEach(n => {
    const key = n.parent_id ?? null;
    if (!childrenOf.has(key)) childrenOf.set(key, []);
    childrenOf.get(key)!.push(n);
  });
  childrenOf.forEach(arr => arr.sort((a, b) => a.order_index - b.order_index));

  const roots = childrenOf.get(null) ?? [];
  if (roots.length === 0) return [];

  // DB schema (brain.rs): phases are type="trunk" with parent_id=NULL (multiple roots).
  // Manual trees: single trunk root whose children are phases.
  const phases = roots.length > 1 ? roots : (childrenOf.get(roots[0].id) ?? []);
  const result: { checkpoint: TreeNode; skillId: string | null; skillIndex: number; phaseIndex: number }[] = [];

  // Recursively collect all leaf nodes under a given parent
  function collectLeaves(parentId: string): TreeNode[] {
    const children = childrenOf.get(parentId) ?? [];
    if (children.length === 0) {
      const node = nodes.find(n => n.id === parentId);
      return node && node.type !== 'trunk' ? [node] : [];
    }
    const leaves: TreeNode[] = [];
    children.forEach(child => {
      const childLeaves = collectLeaves(child.id);
      if (childLeaves.length > 0) {
        leaves.push(...childLeaves);
      }
    });
    return leaves;
  }

  phases.forEach((phase, phaseIdx) => {
    const phaseChildren = childrenOf.get(phase.id) ?? [];

    // Check if this phase has intermediate skill nodes or direct leaf children
    const hasSkillLevel = phaseChildren.some(c => {
      const grandchildren = childrenOf.get(c.id) ?? [];
      return grandchildren.length > 0;
    });

    if (hasSkillLevel) {
      // 3-level: phase → skill → checkpoints
      phaseChildren.forEach((skill, skillIdx) => {
        const leaves = collectLeaves(skill.id);
        leaves.forEach(cp => {
          result.push({ checkpoint: cp, skillId: skill.id, skillIndex: skillIdx, phaseIndex: phaseIdx });
        });
      });
    } else {
      // 2-level: phase → checkpoints directly
      phaseChildren.forEach((cp, cpIdx) => {
        result.push({ checkpoint: cp, skillId: null, skillIndex: cpIdx, phaseIndex: phaseIdx });
      });
    }
  });

  return result;
}


// ─── Canvas rendering ───────────────────────────────────────────────────────

// ─── Background + atmosphere ──────────────────────────────────────────────────

function drawBackground(ctx: CanvasRenderingContext2D, w: number, h: number) {
  // Deep night sky
  const skyGrad = ctx.createLinearGradient(0, 0, 0, h);
  skyGrad.addColorStop(0, '#04050a');
  skyGrad.addColorStop(0.4, '#07090f');
  skyGrad.addColorStop(0.7, '#0a0c14');
  skyGrad.addColorStop(1, '#0d1008');
  ctx.fillStyle = skyGrad;
  ctx.fillRect(0, 0, w, h);

  // Canopy glow — faint green radial behind the canopy
  const cx = w * 0.43;
  const canopyGlow = ctx.createRadialGradient(cx, h * 0.35, 0, cx, h * 0.35, w * 0.45);
  canopyGlow.addColorStop(0, 'rgba(74,140,92,0.045)');
  canopyGlow.addColorStop(0.35, 'rgba(45,107,66,0.025)');
  canopyGlow.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = canopyGlow;
  ctx.fillRect(0, 0, w, h);
}

function drawGroundRoots(ctx: CanvasRenderingContext2D, w: number, h: number) {
  const scale = Math.min(w, h) / 900;
  const cx    = w * 0.43;
  const baseY = h * 0.97;

  // Ground fog ellipse
  const fogGrad = ctx.createRadialGradient(cx, baseY, 0, cx, baseY, 120 * scale);
  fogGrad.addColorStop(0, 'rgba(40,25,10,0.25)');
  fogGrad.addColorStop(0.5, 'rgba(20,12,5,0.1)');
  fogGrad.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.beginPath();
  ctx.ellipse(cx, baseY, 120 * scale, 40 * scale, 0, 0, Math.PI * 2);
  ctx.fillStyle = fogGrad;
  ctx.fill();

  // Surface roots fanning from base
  const rootConfigs = [
    { angle: -0.6, len: 80, wobble:  0.15 },
    { angle: -0.25,len: 65, wobble: -0.1  },
    { angle:  0.1, len: 55, wobble:  0.08 },
    { angle:  0.35,len: 70, wobble: -0.12 },
    { angle:  0.65,len: 60, wobble:  0.1  },
  ];
  rootConfigs.forEach(rc => {
    const ex = cx + rc.len * scale * Math.sin(rc.angle);
    const ey = baseY + rc.len * scale * Math.cos(rc.angle) * 0.35;
    taperedBranch(ctx, cx, baseY - 4 * scale, ex, ey, 10 * scale, 2 * scale, rc.wobble);
    const rootGrad = ctx.createLinearGradient(cx, baseY, ex, ey);
    rootGrad.addColorStop(0, 'rgba(61,37,16,0.7)');
    rootGrad.addColorStop(1, 'rgba(30,18,8,0.1)');
    ctx.fillStyle = rootGrad;
    ctx.fill();
  });
}

function drawRoot(ctx: CanvasRenderingContext2D, w: number, h: number, animTime: number) {
  const scale     = Math.min(w, h) / 900;
  const cx        = w * 0.43;
  const trunkTopY = h * 0.58;

  ctx.save();
  ctx.translate(cx, trunkTopY);

  const pulseFrac = 0.5 + 0.5 * Math.sin(animTime * 0.8);
  const outerR    = 18 * scale * (1 + pulseFrac * 0.08);

  // Outer glow ring
  const ringGrad = ctx.createRadialGradient(0, 0, outerR * 0.6, 0, 0, outerR * 1.8);
  ringGrad.addColorStop(0, 'rgba(200,130,26,0.15)');
  ringGrad.addColorStop(1, 'rgba(200,130,26,0)');
  ctx.beginPath();
  ctx.arc(0, 0, outerR * 1.8, 0, Math.PI * 2);
  ctx.fillStyle = ringGrad;
  ctx.fill();

  // Domain junction ring
  ctx.beginPath();
  ctx.arc(0, 0, outerR, 0, Math.PI * 2);
  ctx.strokeStyle = `rgba(200,130,26,${0.25 + pulseFrac * 0.15})`;
  ctx.lineWidth = 1.5 * scale;
  ctx.stroke();

  // Center amber dot
  ctx.beginPath();
  ctx.arc(0, 0, 5 * scale, 0, Math.PI * 2);
  const dg = ctx.createRadialGradient(0, 0, 0, 0, 0, 5 * scale);
  dg.addColorStop(0, '#e8a832');
  dg.addColorStop(1, '#8b5e1a');
  ctx.fillStyle = dg;
  ctx.fill();

  ctx.restore();
}

function drawAtmosphere(ctx: CanvasRenderingContext2D, w: number, h: number, animTime: number) {
  const scale = Math.min(w, h) / 900;
  ctx.save();
  const moteCount = 18;
  for (let i = 0; i < moteCount; i++) {
    const seedX  = ((i * 137.5 + 50) % (w * 0.7)) + w * 0.1;
    const seedY  = ((i * 97.3  + 80) % (h * 0.6)) + h * 0.05;
    const floatY = seedY + Math.sin(animTime * 0.3 + i * 1.7) * 12 * scale;
    const floatX = seedX + Math.cos(animTime * 0.22 + i * 1.3) * 8 * scale;
    const alpha  = 0.04 + 0.06 * Math.sin(animTime * 0.5 + i * 2.3);
    ctx.beginPath();
    ctx.arc(floatX, floatY, 1.2 * scale, 0, Math.PI * 2);
    if      (i % 3 === 0) ctx.fillStyle = `rgba(200,130,26,${alpha})`;
    else if (i % 3 === 1) ctx.fillStyle = `rgba(58,122,184,${alpha})`;
    else                   ctx.fillStyle = `rgba(124,77,189,${alpha})`;
    ctx.fill();
  }
  ctx.restore();
}

// ─── Hex → RGB helper ────────────────────────────────────────────────────────

function hexToRgb(hex: string): { r: number; g: number; b: number } {
  if (!hex || hex.length < 7) return { r: 100, g: 100, b: 100 };
  return {
    r: parseInt(hex.slice(1, 3), 16),
    g: parseInt(hex.slice(3, 5), 16),
    b: parseInt(hex.slice(5, 7), 16),
  };
}

// ─── Tapered filled branch path ───────────────────────────────────────────────

function taperedBranch(
  ctx: CanvasRenderingContext2D,
  x1: number, y1: number, x2: number, y2: number,
  w1: number, w2: number,
  curveBias = 0,
) {
  const dx = x2 - x1, dy = y2 - y1;
  const len = Math.hypot(dx, dy);
  if (len < 0.5) return;
  const nx = -dy / len, ny = dx / len;            // perpendicular unit vector
  const perpDir = Math.atan2(dy, dx) + Math.PI / 2;
  const cpOff   = curveBias * len;
  const cpX = (x1 + x2) / 2 + cpOff * Math.cos(perpDir);
  const cpY = (y1 + y2) / 2 + cpOff * Math.sin(perpDir);
  const hw1 = w1 / 2, hw2 = w2 / 2;

  ctx.beginPath();
  ctx.moveTo(x1 + hw1 * nx, y1 + hw1 * ny);
  ctx.quadraticCurveTo(cpX + hw1 * nx * 0.5, cpY + hw1 * ny * 0.5, x2 + hw2 * nx, y2 + hw2 * ny);
  ctx.lineTo(x2 - hw2 * nx, y2 - hw2 * ny);
  ctx.quadraticCurveTo(cpX - hw1 * nx * 0.5, cpY - hw1 * ny * 0.5, x1 - hw1 * nx, y1 - hw1 * ny);
  ctx.closePath();
}

// ─── Draw all branches sorted back-to-front by depth ─────────────────────────

function drawTree(ctx: CanvasRenderingContext2D, branches: Branch[]) {
  const sorted = [...branches].sort((a, b) => a.depth - b.depth);

  sorted.forEach(b => {
    taperedBranch(ctx, b.x1, b.y1, b.x2, b.y2, b.w1, b.w2, b.curveBias);

    const baseColor = b.color || '#3d2510';
    const grad = ctx.createLinearGradient(b.x1, b.y1, b.x2, b.y2);

    if (b.depth === 0) {
      // Trunk — warm bark gradient
      grad.addColorStop(0, '#4a2e14');
      grad.addColorStop(0.4, '#3d2510');
      grad.addColorStop(1, '#2e1c0c');
    } else if (b.depth === 1) {
      // Domain boughs — domain color at low opacity
      const c = hexToRgb(baseColor);
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.5)`);
      grad.addColorStop(0.5, `rgba(${c.r},${c.g},${c.b},0.35)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.2)`);
    } else {
      // Limbs, twigs, atmospheric — fading domain color
      const c = hexToRgb(baseColor);
      grad.addColorStop(0, `rgba(${c.r},${c.g},${c.b},0.3)`);
      grad.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0.12)`);
    }

    ctx.fillStyle = grad;
    ctx.fill();

    // Subtle bark highlight on trunk
    if (b.depth === 0) {
      taperedBranch(ctx, b.x1, b.y1, b.x2, b.y2, b.w1, b.w2, b.curveBias);
      ctx.strokeStyle = 'rgba(255,200,120,0.04)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  });
}

// ─── Leaf shape constants ────────────────────────────────────────────────────

const LEAF_SHAPE = {
  size: 14,
  width: 0.45,
  pointiness: 0.3,
  curve: 0.15,
  glowRadius: 12,
  lockedOpacity: 0.35,        // stone-dark but legible silhouette
  buddingPulseSpeed: 2.2,     // sin oscillation rad/s
  buddingPulseMin: 0.55,
  buddingPulseMax: 0.85,
  progressRingRadius: 18,     // px, arc centered at leaf visual center
  progressRingWidth: 1.8,
  sparkleCount: 5,
  sparkleRadius: 20,
  sparkleDotSize: 1.2,
};

// ─── Draw a single botanical leaf ────────────────────────────────────────────

function drawLeaf(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, angle: number,
  size: number, color: string,
  state: 'dormant' | 'budding' | 'growing' | 'bloomed',
  isHovered: boolean, isSelected: boolean,
  animTime: number,
  progress: number,
) {
  const w = size * LEAF_SHAPE.width;
  const len = size;
  const pointOff = len * LEAF_SHAPE.pointiness;
  const curve = LEAF_SHAPE.curve * size;

  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(angle);

  let fillColor: string, strokeColor: string, alpha: number;
  if (state === 'bloomed') {
    fillColor = color;
    strokeColor = '#ffffffcc';
    alpha = 1;
  } else if (state === 'dormant') {
    fillColor = '#1e1e1e';     // stone-grey silhouette
    strokeColor = '#3a3a3a';   // visible edge
    alpha = LEAF_SHAPE.lockedOpacity;
  } else if (state === 'growing') {
    fillColor = color;         // full color — vibrant
    strokeColor = color + 'ff';
    alpha = 1.0;
  } else {
    // budding — breathing pulse
    const pulse = LEAF_SHAPE.buddingPulseMin +
      (LEAF_SHAPE.buddingPulseMax - LEAF_SHAPE.buddingPulseMin) *
      (0.5 + 0.5 * Math.sin(animTime * LEAF_SHAPE.buddingPulseSpeed));
    fillColor = color + 'bb';  // moderate saturation
    strokeColor = color + 'ff'; // strong outline signals "ready"
    alpha = pulse;
  }

  ctx.globalAlpha = alpha;

  if (state === 'bloomed' && LEAF_SHAPE.glowRadius > 0) {
    // Outer soft halo
    const halo = ctx.createRadialGradient(len * 0.4, 0, 4, len * 0.4, 0, LEAF_SHAPE.glowRadius + size * 1.4);
    halo.addColorStop(0, color + '55');
    halo.addColorStop(0.5, color + '22');
    halo.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, LEAF_SHAPE.glowRadius + size * 1.4, 0, Math.PI * 2);
    ctx.fillStyle = halo;
    ctx.fill();
    // Tight bright core glow
    const core = ctx.createRadialGradient(len * 0.4, 0, 1, len * 0.4, 0, size * 0.9);
    core.addColorStop(0, color + 'aa');
    core.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 0.9, 0, Math.PI * 2);
    ctx.fillStyle = core;
    ctx.fill();
  }

  // Budding: pulsing annular ring to signal "ready to start"
  if (state === 'budding') {
    const ringAlpha = Math.round(
      (0.2 + 0.3 * (0.5 + 0.5 * Math.sin(animTime * LEAF_SHAPE.buddingPulseSpeed + Math.PI)))
      * 255
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
  ctx.strokeStyle = strokeColor;
  ctx.lineWidth = isHovered ? 1.8 : 1;
  ctx.stroke();

  // Growing: progress arc ring around leaf center
  if (state === 'growing' && progress > 0) {
    const rcx = len * 0.4;
    const r = LEAF_SHAPE.progressRingRadius;
    const startAngle = -Math.PI / 2;
    const endAngle = startAngle + (progress / 100) * Math.PI * 2;
    // Background track
    ctx.beginPath();
    ctx.arc(rcx, 0, r, 0, Math.PI * 2);
    ctx.strokeStyle = color + '33';
    ctx.lineWidth = LEAF_SHAPE.progressRingWidth;
    ctx.stroke();
    // Filled arc showing completion %
    ctx.beginPath();
    ctx.arc(rcx, 0, r, startAngle, endAngle);
    ctx.strokeStyle = color + 'ee';
    ctx.lineWidth = LEAF_SHAPE.progressRingWidth;
    ctx.lineCap = 'round';
    ctx.stroke();
    ctx.lineCap = 'butt';
  }

  ctx.beginPath();
  ctx.moveTo(1, 0);
  ctx.lineTo(len * 0.85, 0);
  ctx.strokeStyle = state === 'dormant' ? '#2e2e2e' : (state === 'bloomed' ? '#ffffff44' : color + '55');
  ctx.lineWidth = 0.6;
  ctx.stroke();

  const veinCount = Math.max(2, Math.floor(size / 5));
  for (let v = 1; v <= veinCount; v++) {
    const t = v / (veinCount + 1);
    const vx = len * t * 0.85;
    const vw = w * (1 - t * 0.6) * 0.6;
    const veinStroke = state === 'dormant' ? '#262626' : (state === 'bloomed' ? '#ffffff22' : color + '33');
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

  // Bloomed: slowly orbiting sparkle dots
  if (state === 'bloomed') {
    const scx = len * 0.4;
    const baseAngle = animTime * 0.6;
    const savedAlpha = ctx.globalAlpha;
    for (let i = 0; i < LEAF_SHAPE.sparkleCount; i++) {
      const theta = baseAngle + (i / LEAF_SHAPE.sparkleCount) * Math.PI * 2;
      const dist = LEAF_SHAPE.sparkleRadius * (0.7 + 0.3 * (Math.sin(animTime * 1.3 + i * 2.1) * 0.5 + 0.5));
      const sx = scx + Math.cos(theta) * dist;
      const sy = Math.sin(theta) * dist * 0.6;
      const sa = 0.4 + 0.4 * (Math.sin(animTime * 2.7 + i * 1.8) * 0.5 + 0.5);
      ctx.beginPath();
      ctx.arc(sx, sy, LEAF_SHAPE.sparkleDotSize, 0, Math.PI * 2);
      ctx.fillStyle = '#ffffff';
      ctx.globalAlpha = sa;
      ctx.fill();
    }
    ctx.globalAlpha = savedAlpha;
  }

  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(-size * 0.4, 0);
  ctx.strokeStyle = '#1a1008';
  ctx.lineWidth = 1.2;
  ctx.stroke();

  ctx.globalAlpha = 1;
  ctx.restore();
}

// ─── Draw a decorative (non-interactive) leaf at unused tips ─────────────────

function drawDecorativeLeaf(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, angle: number,
  size: number,
) {
  const scale = 0.6;
  const len = size * scale;
  const w = len * LEAF_SHAPE.width;
  const pointOff = len * LEAF_SHAPE.pointiness;
  const curve = LEAF_SHAPE.curve * len;

  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(angle);
  ctx.globalAlpha = 0.35;

  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.bezierCurveTo(pointOff * 0.3, -(w * 0.5 + curve), pointOff, -(w + curve * 0.5), len, 0);
  ctx.bezierCurveTo(pointOff, (w + curve * 0.5), pointOff * 0.3, (w * 0.5 + curve), 0, 0);
  ctx.closePath();
  ctx.fillStyle = '#1a1208';
  ctx.fill();
  ctx.strokeStyle = '#2a1e10';
  ctx.lineWidth = 0.6;
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(1, 0);
  ctx.lineTo(len * 0.8, 0);
  ctx.strokeStyle = '#221a0c';
  ctx.lineWidth = 0.4;
  ctx.stroke();

  ctx.globalAlpha = 1;
  ctx.restore();
}

// ─── Draw all leaves + phase/skill labels ────────────────────────────────────

function drawLeaves(
  ctx: CanvasRenderingContext2D,
  placements: NodePlacement[],
  unusedTips: Tip[],
  selectedId: string | null,
  hoveredId: string | null,
  centerX: number,
  animTime: number,
) {
  // Draw decorative leaves at unused tips first (behind interactive leaves)
  unusedTips.forEach(tip => {
    drawDecorativeLeaf(ctx, tip.x, tip.y, tip.angle, LEAF_SHAPE.size);
  });

  // Draw interactive checkpoint leaves
  placements.forEach(({ node, x, y, angle, phaseIndex }) => {
    const color = PHASE_COLORS[phaseIndex % PHASE_COLORS.length];
    const isLocked = node.is_locked ?? false;
    const progress = node.progress ?? 0;
    const isSelected = node.id === selectedId;
    const isHovered = node.id === hoveredId;

    const state: 'dormant' | 'budding' | 'growing' | 'bloomed' =
      isLocked ? 'dormant'
      : progress === 0 ? 'budding'
      : progress < 100 ? 'growing'
      : 'bloomed';

    drawLeaf(ctx, x, y, angle, LEAF_SHAPE.size, color, state, isHovered, isSelected, animTime, progress);

    if (isHovered || isSelected) {
      const label = node.title.length > 22 ? node.title.slice(0, 22) + '\u2026' : node.title;
      ctx.font = '400 10px Inter, sans-serif';
      ctx.fillStyle = '#ddd';
      ctx.textBaseline = 'middle';
      const labelOffset = LEAF_SHAPE.size + 10;
      if (x > centerX) {
        ctx.textAlign = 'left';
        ctx.fillText(label, x + labelOffset, y);
      } else {
        ctx.textAlign = 'right';
        ctx.fillText(label, x - labelOffset, y);
      }
      ctx.textAlign = 'left';
    }
  });
}

// ─── Neighborhood Section ─────────────────────────────────────────────────────

function NeighborRow({ n, onSelectNode }: { n: NeighborNode; onSelectNode: (id: string) => void }) {
  const hasResources = n.resources.length > 0;
  return (
    <div style={{ marginBottom: 4 }}>
      <button
        onClick={() => onSelectNode(n.nodeId)}
        style={{
          width: '100%', textAlign: 'left', background: 'none',
          border: '1px solid #1e293b', borderRadius: 5,
          padding: '5px 8px', cursor: 'pointer',
          display: 'flex', flexDirection: 'column', gap: 3,
        }}
        onMouseEnter={e => (e.currentTarget.style.borderColor = '#334155')}
        onMouseLeave={e => (e.currentTarget.style.borderColor = '#1e293b')}
      >
        {/* Title row */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
          {n.isLocked ? (
            <span style={{ fontSize: 9, color: '#475569', flexShrink: 0 }}>🔒</span>
          ) : (
            <div style={{
              flexShrink: 0, width: 28, height: 3, borderRadius: 2,
              background: '#1e293b', overflow: 'hidden',
            }}>
              <div style={{
                width: `${n.progress}%`, height: '100%',
                background: n.progress >= 100 ? '#10b981' : n.progress > 0 ? '#059669' : '#334155',
              }} />
            </div>
          )}
          <span style={{
            flex: 1, fontSize: 11, color: '#94a3b8', minWidth: 0,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {n.title}
          </span>
          {n.skillLevel != null && (
            <span style={{
              flexShrink: 0, fontSize: 9, fontWeight: 600,
              padding: '1px 4px', borderRadius: 3,
              background: 'rgba(99,102,241,0.15)', color: '#818cf8',
            }}>
              L{n.skillLevel}
            </span>
          )}
        </div>
        {/* Resource pills */}
        {hasResources && (
          <div style={{ display: 'flex', gap: 3, flexWrap: 'wrap', paddingLeft: 2 }}>
            {n.resources.slice(0, 3).map(r => (
              <span key={r.resourceId} style={{
                fontSize: 9, color: '#64748b',
                padding: '1px 5px', borderRadius: 3,
                background: '#0f172a', border: '1px solid #1e293b',
                maxWidth: 100, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>
                {r.matchedSectionTitle ?? r.title}
              </span>
            ))}
          </div>
        )}
      </button>
    </div>
  );
}

function NeighborhoodSubsection({
  label, nodes, onSelectNode, accent,
}: {
  label: string;
  nodes: NeighborNode[];
  onSelectNode: (id: string) => void;
  accent: string;
}) {
  const [open, setOpen] = useState(true);
  if (nodes.length === 0) return null;
  return (
    <div style={{ marginBottom: 8 }}>
      <button
        onClick={() => setOpen(v => !v)}
        style={{
          display: 'flex', alignItems: 'center', gap: 5, width: '100%',
          background: 'none', border: 'none', cursor: 'pointer', padding: '2px 0', marginBottom: 4,
        }}
      >
        <span style={{ fontSize: 9, color: accent, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
          {label}
        </span>
        <span style={{ fontSize: 9, color: '#334155', marginLeft: 2 }}>
          ({nodes.length})
        </span>
        <span style={{ fontSize: 9, color: '#475569', marginLeft: 'auto' }}>
          {open ? '▴' : '▾'}
        </span>
      </button>
      {open && nodes.map(n => (
        <NeighborRow key={n.nodeId} n={n} onSelectNode={onSelectNode} />
      ))}
    </div>
  );
}

function PrereqPathInline({ path }: { path: PathNode[] }) {
  if (path.length === 0) return null;
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 3, marginBottom: 8 }}>
      {path.map((node, i) => {
        const isLast = i === path.length - 1;
        const color = node.state === 'seed' ? '#f59e0b' : isLast ? '#34d399' : '#64748b';
        return (
          <span key={node.skillId} style={{ display: 'inline-flex', alignItems: 'center', gap: 3 }}>
            <span style={{
              fontSize: 9, padding: '1px 5px', borderRadius: 4,
              background: node.state === 'seed' ? 'rgba(245,158,11,0.15)' : isLast ? 'rgba(52,211,153,0.12)' : 'rgba(255,255,255,0.05)',
              color,
              border: '1px solid ' + (node.state === 'seed' ? 'rgba(245,158,11,0.3)' : isLast ? 'rgba(52,211,153,0.25)' : 'rgba(255,255,255,0.08)'),
              fontWeight: node.state === 'seed' || isLast ? 600 : 400,
            }}>
              {node.state === 'seed' && <span style={{ display: 'inline-block', width: 4, height: 4, borderRadius: '50%', background: '#f59e0b', marginRight: 3, verticalAlign: 'middle' }} />}
              {node.skillName}
            </span>
            {!isLast && <span style={{ color: '#334155', fontSize: 9 }}>→</span>}
          </span>
        );
      })}
    </div>
  );
}

function NeighborhoodSection({ neighborhood, onSelectNode }: {
  neighborhood: NodeNeighborhood;
  onSelectNode: (id: string) => void;
}) {
  const prereqPath = neighborhood.prereqPath;
  return (
    <div style={{ marginTop: 12, paddingTop: 10, borderTop: '1px solid #1e293b' }}>
      <label style={{ display: 'block', fontSize: 10, color: '#64748b', marginBottom: 8, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
        Graph Neighborhood
      </label>
      {prereqPath && prereqPath.isReachable && prereqPath.path.length > 0 && (
        <div style={{ marginBottom: 8 }}>
          <div style={{ fontSize: 9, color: '#475569', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.07em' }}>Path from your skills</div>
          <PrereqPathInline path={prereqPath.path} />
        </div>
      )}
      {prereqPath && !prereqPath.isReachable && (
        <div style={{ fontSize: 9, color: '#334155', fontStyle: 'italic', marginBottom: 8 }}>
          No path from seed skills — infer dependencies first
        </div>
      )}
      <NeighborhoodSubsection label="Prerequisites" nodes={neighborhood.prerequisites} onSelectNode={onSelectNode} accent="#f59e0b" />
      <NeighborhoodSubsection label="Dependents" nodes={neighborhood.dependents} onSelectNode={onSelectNode} accent="#60a5fa" />
      <NeighborhoodSubsection label="Siblings" nodes={neighborhood.siblings} onSelectNode={onSelectNode} accent="#a78bfa" />
    </div>
  );
}

// ─── Node Panel ───────────────────────────────────────────────────────────────

interface NodePanelProps {
  node: TreeNode;
  skillLocked: boolean;
  onSave: (updates: Partial<TreeNode>) => void;
  onClose: () => void;
  onAddChild: () => void;
  onDelete: () => void;
  onSelectNode: (nodeId: string) => void;
}

function NodePanel({ node, skillLocked, onSave, onClose, onAddChild, onDelete, onSelectNode }: NodePanelProps) {
  const [title, setTitle] = useState(node.title);
  const [description, setDescription] = useState(node.description);
  const [resources, setResources] = useState<string[]>(
    Array.isArray(node.resources) ? node.resources.filter(r => typeof r === 'string') : []
  );
  const [mimiResources, setMimiResources] = useState<MimirResource[]>([]);
  const [matching, setMatching] = useState(false);
  const [neighborhood, setNeighborhood] = useState<NodeNeighborhood | null>(null);
  const { setMimirContext, openMimir } = useMimirContext();
  const checkpoint = node.type === 'leaf'
    ? (Array.isArray(node.tasks)
        ? { mastery_criteria: node.tasks[0]?.description ?? '', exercises: [] as string[], notes: node.tasks[0]?.notes ?? '', completed: node.tasks[0]?.completed ?? false }
        : (node.tasks as { mastery_criteria?: string; exercises?: string[]; notes?: string; completed?: boolean } | null))
    : null;
  const [notesText, setNotesText] = useState(checkpoint?.notes ?? '');
  const [showDayPicker, setShowDayPicker] = useState(false);
  const [dayPickerLoading, setDayPickerLoading] = useState(false);
  const notesTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const cp = node.type === 'leaf'
      ? (Array.isArray(node.tasks)
          ? { mastery_criteria: node.tasks[0]?.description ?? '', exercises: [] as string[], notes: node.tasks[0]?.notes ?? '', completed: node.tasks[0]?.completed ?? false }
          : (node.tasks as { mastery_criteria?: string; exercises?: string[]; notes?: string; completed?: boolean } | null))
      : null;
    setTitle(node.title);
    setDescription(node.description);
    setResources(
      Array.isArray(node.resources) ? node.resources.filter(r => typeof r === 'string') : []
    );
    setNotesText(cp?.notes ?? '');
    if (notesTimerRef.current) clearTimeout(notesTimerRef.current);
    setMimiResources([]);
    setNeighborhood(null);
    invoke<MimirResource[]>('get_node_resources', { nodeId: node.id })
      .then(setMimiResources)
      .catch(() => {});
    if (node.type === 'leaf') {
      invoke<NodeNeighborhood>('get_node_neighborhood', { nodeId: node.id })
        .then(setNeighborhood)
        .catch(() => {});
    }
  }, [node.id]);

  useEffect(() => {
    return () => { if (notesTimerRef.current) clearTimeout(notesTimerRef.current); };
  }, []);

  function handleNotesChange(value: string) {
    setNotesText(value);
    if (notesTimerRef.current) clearTimeout(notesTimerRef.current);
    notesTimerRef.current = setTimeout(async () => {
      if (!checkpoint) return;
      try {
        await invoke('update_tree_node', {
          nodeId: node.id,
          title: null, description: null, progress: null,
          tasks: { ...checkpoint, notes: value },
          resources: null, position: null,
        });
      } catch { /* silently ignore auto-save errors */ }
    }, 1000);
  }

  async function handleFindMatches() {
    setMatching(true);
    try {
      await invoke('match_node_to_resources', { nodeId: node.id });
      const resources = await invoke<MimirResource[]>('get_node_resources', { nodeId: node.id });
      setMimiResources(resources);
    } catch {
    } finally {
      setMatching(false);
    }
  }

  async function handleResourceClick(r: MimirResource) {
    if (r.url) {
      await openUrl(r.url);
    } else {
      // PDF — open Mimir chat pre-seeded with section context
      const contextHint = r.matchedSectionTitle
        ? `${r.title} — ${r.matchedSectionTitle}${r.matchedPageStart != null ? ` (pp. ${r.matchedPageStart}–${r.matchedPageEnd ?? r.matchedPageStart})` : ''}`
        : r.title;
      setMimirContext({ nodeTitle: contextHint });
      openMimir();
    }
  }

  async function handleToggleCompletion(r: MimirResource) {
    try {
      const newVal = await invoke<boolean>('toggle_resource_completion', { resourceId: r.id });
      setMimiResources(prev =>
        prev.map(res => res.id === r.id ? { ...res, isCompleted: newVal } : res)
      );
    } catch { /* silently ignore */ }
  }

  function handleSave() {
    onSave({ title, description, resources: resources.length > 0 ? resources : null });
  }

  function handleToggleComplete() {
    if (!checkpoint) return;
    const newCompleted = !checkpoint.completed;
    onSave({
      progress: newCompleted ? 100 : 0,
      tasks: {
        mastery_criteria: checkpoint.mastery_criteria ?? '',
        exercises: checkpoint.exercises ?? [],
        notes: checkpoint.notes ?? '',
        completed: newCompleted,
      },
    });
  }

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
            Reach all checkpoints in the previous skill to unlock this one.
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

        {/* Checkpoint details — leaf nodes only */}
        {node.type === 'leaf' && checkpoint && (
          <div style={{
            marginBottom: 12, padding: 10,
            background: '#1e293b', borderRadius: 8, border: '1px solid #334155',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
              <span style={{ fontSize: 10, color: '#64748b', textTransform: 'uppercase', letterSpacing: '0.07em' }}>Checkpoint</span>
            </div>
            {checkpoint.mastery_criteria && (
              <>
                <label style={{ display: 'block', fontSize: 9, color: '#475569', marginBottom: 3, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
                  Mastery Criteria
                </label>
                <p style={{ fontSize: 12, color: '#94a3b8', lineHeight: 1.5, margin: '0 0 10px 0' }}>
                  {checkpoint.mastery_criteria}
                </p>
              </>
            )}
            {checkpoint.exercises && checkpoint.exercises.length > 0 && (
              <>
                <label style={{ display: 'block', fontSize: 9, color: '#475569', marginBottom: 3, textTransform: 'uppercase', letterSpacing: '0.07em' }}>
                  Exercises
                </label>
                <ul style={{ margin: '0 0 10px 0', paddingLeft: 16 }}>
                  {checkpoint.exercises.map((ex: string, i: number) => (
                    <li key={i} style={{ fontSize: 11, color: '#94a3b8', lineHeight: 1.6, marginBottom: 2 }}>
                      {ex}
                    </li>
                  ))}
                </ul>
              </>
            )}
            <button
              onClick={handleToggleComplete}
              style={{ display: 'flex', alignItems: 'center', gap: 8, background: 'none', border: 'none', cursor: 'pointer', padding: 0 }}
            >
              <div style={{
                width: 16, height: 16, borderRadius: 4,
                border: `2px solid ${checkpoint.completed ? '#10b981' : '#475569'}`,
                background: checkpoint.completed ? '#10b981' : 'transparent',
                display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
              }}>
                {checkpoint.completed && (
                  <svg width="9" height="9" viewBox="0 0 10 10" fill="none">
                    <path d="M1.5 5L4 7.5L8.5 2.5" stroke="white" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                )}
              </div>
              <span style={{ fontSize: 12, color: checkpoint.completed ? '#10b981' : '#94a3b8' }}>
                {checkpoint.completed ? 'Reached' : 'Mark as reached'}
              </span>
            </button>
            {/* Add to today's matrix */}
            <div style={{ marginTop: 8, position: 'relative' }}>
              <button
                onClick={() => setShowDayPicker((v) => !v)}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  background: 'none', border: '1px solid #334155',
                  borderRadius: 6, padding: '4px 10px', cursor: 'pointer',
                  color: '#94a3b8', fontSize: 11,
                }}
              >
                <span style={{ fontSize: 13 }}>+</span> Add to today
              </button>
              {showDayPicker && (
                <div style={{
                  position: 'absolute', top: '100%', left: 0, marginTop: 4,
                  background: '#1e293b', border: '1px solid #334155',
                  borderRadius: 8, padding: 6, display: 'flex', flexDirection: 'column', gap: 4,
                  zIndex: 10, minWidth: 140,
                }}>
                  {[
                    { key: 'do', label: 'Do', color: '#dc2626' },
                    { key: 'schedule', label: 'Schedule', color: '#2563eb' },
                    { key: 'delegate', label: 'Delegate', color: '#d97706' },
                    { key: 'eliminate', label: 'Eliminate', color: '#64748b' },
                  ].map((q) => (
                    <button
                      key={q.key}
                      disabled={dayPickerLoading}
                      onClick={async () => {
                        setDayPickerLoading(true);
                        try {
                          const today = new Date().toISOString().slice(0, 10);
                          await invoke('add_quest_to_day', {
                            date: today,
                            nodeId: node.id,
                            quadrant: q.key,
                          });
                          setShowDayPicker(false);
                        } catch (e) {
                          console.error('Failed to add to today:', e);
                        } finally {
                          setDayPickerLoading(false);
                        }
                      }}
                      style={{
                        background: 'none', border: 'none', cursor: 'pointer',
                        color: q.color, fontSize: 12, textAlign: 'left',
                        padding: '4px 8px', borderRadius: 4,
                      }}
                      onMouseEnter={(e) => { (e.target as HTMLElement).style.background = '#0f172a'; }}
                      onMouseLeave={(e) => { (e.target as HTMLElement).style.background = 'none'; }}
                    >
                      {q.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}

        {/* Checkpoint notes — leaf nodes only */}
        {node.type === 'leaf' && checkpoint && (
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
        <div style={{ marginBottom: neighborhood && (neighborhood.prerequisites.length > 0 || neighborhood.dependents.length > 0 || neighborhood.siblings.length > 0) ? 0 : 12 }}>
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
            mimiResources.map(r => {
              const isPdf = r.resourceType === 'pdf';
              const score = r.relevanceScore ?? 1;
              const dotColor = score < 0.3 ? '#10b981' : score < 0.45 ? '#f59e0b' : '#475569';
              const hasSection = isPdf && r.matchedSectionTitle;
              const pageRange = r.matchedPageStart != null
                ? r.matchedPageStart === r.matchedPageEnd
                  ? `p. ${r.matchedPageStart}`
                  : `pp. ${r.matchedPageStart}–${r.matchedPageEnd}`
                : null;

              return (
                <div key={r.id} style={{
                  display: 'flex', alignItems: 'flex-start', gap: 4, marginBottom: 4,
                  opacity: r.isCompleted ? 0.6 : 1,
                }}>
                  {/* Clickable resource card */}
                  <button
                    onClick={() => handleResourceClick(r)}
                    title={r.url ?? (isPdf ? 'Open in Mimir chat' : r.title)}
                    style={{
                      flex: 1, minWidth: 0, textAlign: 'left',
                      padding: '5px 8px', cursor: 'pointer',
                      background: '#0c1a2e', border: '1px solid #1e3a5f',
                      borderRadius: 5, display: 'flex', flexDirection: 'column', gap: 2,
                    }}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', gap: 5, minWidth: 0 }}>
                      {/* Type badge */}
                      <span style={{
                        flexShrink: 0,
                        fontSize: 9, fontWeight: 600, letterSpacing: '0.04em',
                        padding: '1px 4px', borderRadius: 3,
                        background: isPdf ? 'rgba(239,68,68,0.12)' : 'rgba(96,165,250,0.1)',
                        color: isPdf ? '#f87171' : '#60a5fa',
                      }}>
                        {isPdf ? 'PDF' : 'URL'}
                      </span>
                      {/* Title */}
                      <span style={{
                        flex: 1, fontSize: 10, minWidth: 0,
                        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        color: r.isCompleted ? '#475569' : isPdf ? '#94a3b8' : '#93c5fd',
                        textDecoration: r.isCompleted ? 'line-through' : 'none',
                      }}>
                        {r.title}
                      </span>
                      {/* Relevance dot */}
                      <span style={{
                        flexShrink: 0, width: 6, height: 6, borderRadius: '50%',
                        background: dotColor, display: 'inline-block',
                      }} title={`Relevance: ${score.toFixed(2)}`} />
                    </div>
                    {/* Section + page hint */}
                    {hasSection && (
                      <div style={{
                        fontSize: 9, color: '#64748b', paddingLeft: 2,
                        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      }}>
                        {r.matchedSectionTitle}{pageRange ? ` · ${pageRange}` : ''}
                      </div>
                    )}
                  </button>

                  {/* Completion toggle */}
                  <button
                    onClick={() => handleToggleCompletion(r)}
                    title={r.isCompleted ? 'Mark incomplete' : 'Mark complete'}
                    style={{
                      flexShrink: 0, width: 20, height: 20,
                      borderRadius: 4, border: `1px solid ${r.isCompleted ? '#059669' : '#334155'}`,
                      background: r.isCompleted ? 'rgba(5,150,105,0.15)' : 'transparent',
                      color: r.isCompleted ? '#10b981' : '#475569',
                      cursor: 'pointer', fontSize: 11, lineHeight: 1,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      marginTop: 2,
                    }}
                  >
                    {r.isCompleted ? '✓' : ''}
                  </button>
                </div>
              );
            })
          )}
        </div>

        {/* Graph Neighborhood — leaf nodes only, shown when any section has content */}
        {node.type === 'leaf' && neighborhood && (
          neighborhood.prerequisites.length > 0 || neighborhood.dependents.length > 0 || neighborhood.siblings.length > 0
        ) && (
          <NeighborhoodSection neighborhood={neighborhood} onSelectNode={onSelectNode} />
        )}
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
            disabled={node.type === 'leaf'}
            style={{
              flex: 1, padding: '6px 0',
              background: node.type === 'leaf' ? '#1e293b' : '#1e3a2e',
              color: node.type === 'leaf' ? '#475569' : '#6ee7b7',
              border: `1px solid ${node.type === 'leaf' ? '#334155' : '#166534'}`,
              borderRadius: 6, cursor: node.type === 'leaf' ? 'not-allowed' : 'pointer', fontSize: 11,
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

export default function YggdrasilTree({ projectId, externalSelectedNodeId }: YggdrasilTreeProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const { setMimirContext } = useMimirContext();

  // Data state
  const [treeId, setTreeId] = useState<string | null>(null);
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [, setEdges] = useState<TreeEdge[]>([]);
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(null);
  const [hoveredNode, setHoveredNode] = useState<string | null>(null);
  const [size, setSize] = useState({ w: 1200, h: 900 });

  // Pan/zoom via refs (no re-render on transform change — renderTick triggers redraw)
  const transform = useRef({ x: 0, y: 0, scale: 1 });
  const isDragging = useRef(false);
  const lastMouse = useRef({ x: 0, y: 0 });
  const dragMoved = useRef(false);
  const [renderTick, setRenderTick] = useState(0);
  const triggerRender = useCallback(() => setRenderTick(t => t + 1), []);

  // Deterministic layout — branches and leaf placements computed from node hierarchy
  const { branches, placements, unusedTips } = useMemo(
    () => layoutTree(nodes, size.w, size.h),
    [nodes, size.w, size.h],
  );

  // Sync selected node + tree to Mimir context — projectName/treeName set by loadTree and persist
  useEffect(() => {
    setMimirContext({
      treeId,  // always keep treeId once loaded so Mimir has tree context even without a node
      nodeId: selectedNode?.id ?? null,
      nodeTitle: selectedNode?.title ?? null,
    });
  }, [selectedNode, treeId]);

  // Resize observer — re-run when nodes load (container mounts conditionally)
  const hasNodes = nodes.length > 0;
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    // Grab initial size synchronously so the first frame is correct
    const { clientWidth, clientHeight } = el;
    if (clientWidth > 0 && clientHeight > 0) {
      setSize({ w: clientWidth, h: clientHeight });
    }
    const ro = new ResizeObserver(entries => {
      const { width, height } = entries[0].contentRect;
      if (width > 0 && height > 0) {
        setSize({ w: Math.round(width), h: Math.round(height) });
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [hasNodes]);

  // Load tree when project changes
  useEffect(() => {
    setNodes([]);
    setEdges([]);
    setSelectedNode(null);
    loadTree();
  }, [projectId]);

  // Sync external node selection (e.g. from gap panel)
  useEffect(() => {
    if (!externalSelectedNodeId) return;
    const node = nodes.find(n => n.id === externalSelectedNodeId) ?? null;
    if (node) setSelectedNode(node);
  }, [externalSelectedNodeId, nodes]);

  // ── Canvas render — with pan/zoom transform ─────────────────────────────────
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

    // Background + atmosphere fill the full canvas (not pan/zoom transformed)
    drawBackground(ctx, size.w, size.h);
    drawAtmosphere(ctx, size.w, size.h, animTime);
    drawGroundRoots(ctx, size.w, size.h);

    // Tree + leaves drawn in world space via pan/zoom transform
    const t = transform.current;
    ctx.save();
    ctx.translate(t.x, t.y);
    ctx.scale(t.scale, t.scale);
    drawTree(ctx, branches);
    drawRoot(ctx, size.w, size.h, animTime);
    drawLeaves(ctx, placements, unusedTips, selectedNode?.id ?? null, hoveredNode, size.w * 0.43, animTime);
    ctx.restore();

  }, [branches, placements, unusedTips, selectedNode, hoveredNode, size, renderTick]);

  // ── Animation loop — drives budding pulse, growing ring, bloomed sparkles ───
  const hasAnimatedNodes = useMemo(
    () => placements.some(p => !(p.node.is_locked ?? false)),
    [placements],
  );

  useEffect(() => {
    if (!hasAnimatedNodes) return;
    let rafId: number;
    const tick = () => {
      triggerRender();
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [hasAnimatedNodes, triggerRender]);

  // ── Wheel zoom (native listener for preventDefault) ────────────────────────
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
      const newScale = Math.max(0.2, Math.min(5, t.scale * delta));
      // Zoom toward cursor
      t.x = mx - (mx - t.x) * (newScale / t.scale);
      t.y = my - (my - t.y) * (newScale / t.scale);
      t.scale = newScale;
      triggerRender();
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => canvas.removeEventListener('wheel', onWheel);
  }, [triggerRender]);

  // ── Data loading ──────────────────────────────────────────────────────────

  async function loadTree() {
    try {
      const [trees, projects] = await Promise.all([
        invoke<Tree[]>('get_trees', { projectId }),
        invoke<Project[]>('get_projects'),
      ]);
      const project = projects.find(p => p.id === projectId);
      const pName = project?.name ?? null;

      let id: string;
      let tName: string;

      if (trees.length === 0) {
        const newTree = await invoke<Tree>('create_tree', { projectId, name: 'My Skill Tree' });
        id = newTree.id;
        tName = newTree.name;
        await invoke('create_tree_node', {
          treeId: id, parentId: null, type: 'trunk',
          title: 'Start Here', description: 'Root of your skill tree',
        });
      } else {
        id = trees[0].id;
        tName = trees[0].name;
      }

      setTreeId(id);
      setMimirContext({ treeId: id, projectName: pName, treeName: tName });
      await loadTreeContents(id);
    } catch (err) {
      console.error('Failed to load tree:', err);
    }
  }

  async function loadTreeContents(id: string) {
    try {
      const [, rawNodes, edgeList] = await invoke<[any, unknown[], TreeEdge[]]>(
        'get_tree_with_contents', { treeId: id }
      );
      const nodeList = validateOrLog(z.array(TreeNodeSchema), rawNodes, 'get_tree_with_contents') as TreeNode[];
      setNodes(nodeList);
      setEdges(edgeList);
    } catch (err) {
      console.error('Failed to load tree contents:', err);
    }
  }

  // Reload tree when the orchestrator emits ygg-checkpoint-completed for our tree
  useEffect(() => {
    if (!treeId) return;
    const unlisten = listen<{ treeId: string }>('ygg-checkpoint-completed', ({ payload }) => {
      if (payload.treeId === treeId) {
        loadTreeContents(treeId);
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, [treeId]);

  // ── CRUD handlers ─────────────────────────────────────────────────────────

  async function handleSave(updates: Partial<TreeNode>) {
    if (!selectedNode || !treeId) return;
    try {
      await invoke('update_tree_node', {
        nodeId: selectedNode.id,
        title: updates.title ?? null,
        description: updates.description ?? null,
        progress: updates.progress ?? null,
        // Send undefined (not null) when tasks isn't changing — Tauri maps undefined
        // to None in Rust, which falls back to the existing DB value. Sending null
        // would be deserialized as Some(Value::Null) and wipe the tasks JSONB.
        tasks: 'tasks' in updates ? (updates.tasks ?? null) : undefined,
        resources: updates.resources !== undefined ? (updates.resources ?? null) : null,
        position: null,
      });

      // recalculate_tree_progress, recalculate_unlocks, and skill sync are now
      // handled server-side by the orchestrator. Reload to reflect server state.
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

  // ── Canvas interaction — inverse transform hit detection + pan ──────────────

  /** Convert screen (CSS) coords to world coords via inverse transform */
  function screenToWorld(sx: number, sy: number) {
    const t = transform.current;
    return { x: (sx - t.x) / t.scale, y: (sy - t.y) / t.scale };
  }

  /** Find the closest leaf node to a world-space point */
  function findNode(wx: number, wy: number): TreeNode | null {
    const hitRadius = LEAF_SHAPE.size * 1.5;
    let closest: TreeNode | null = null;
    let closestDist = Infinity;
    placements.forEach(p => {
      const dist = Math.hypot(p.x - wx, p.y - wy);
      if (dist < hitRadius && dist < closestDist) {
        closest = p.node;
        closestDist = dist;
      }
    });
    return closest;
  }

  function handleCanvasClick(e: React.MouseEvent) {
    // If we just finished a drag, don't treat it as a click
    if (dragMoved.current) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    setSelectedNode(findNode(world.x, world.y)); // null deselects
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

    // Hover detection
    const rect = canvas.getBoundingClientRect();
    const world = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    const node = findNode(world.x, world.y);
    const newHovered = node?.id ?? null;
    if (newHovered !== hoveredNode) {
      setHoveredNode(newHovered);
    }
  }

  function handleMouseUp() {
    isDragging.current = false;
  }

  // ── Render ────────────────────────────────────────────────────────────────

  if (nodes.length === 0) {
    return (
      <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', color: '#64748b', fontSize: 13, background: '#010208' }}>
        Loading tree…
      </div>
    );
  }

  const nodeById = new Map(nodes.map(n => [n.id, n]));

  return (
    <div ref={containerRef} style={{ position: 'absolute', inset: 0, overflow: 'hidden', background: '#010208' }}>
      <canvas
        ref={canvasRef}
        style={{
          display: 'block', width: size.w + 'px', height: size.h + 'px',
          cursor: isDragging.current ? 'grabbing' : hoveredNode ? 'pointer' : 'grab',
        }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onClick={handleCanvasClick}
        onMouseLeave={() => { isDragging.current = false; dragMoved.current = false; setHoveredNode(null); }}
      />

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
          onSelectNode={(id) => {
            const n = nodeById.get(id);
            if (n) setSelectedNode(n);
          }}
        />
      )}
    </div>
  );
}
