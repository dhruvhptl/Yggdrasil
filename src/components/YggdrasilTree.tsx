// src/components/YggdrasilTree.tsx
// Procedural L-system skill tree rendered on HTML Canvas.
// Two rendering passes: (1) organic tree branches, (2) node ornaments at tips.
// Dynamic coordinates — tree fills canvas naturally. No pan/zoom transform.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MimirResource } from '../types';
import { useMimirContext } from '../contexts/MimirContext';

// ─── Types ───────────────────────────────────────────────────────────────────

interface TreeNode {
  id: string;
  tree_id: string;
  parent_id: string | null;
  type: 'trunk' | 'branch' | 'leaf';
  title: string;
  description: string;
  progress: number; // 0–100
  tasks: any;
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

interface YggdrasilTreeProps {
  projectId: string;
}

// ─── L-System types ──────────────────────────────────────────────────────────

interface Branch {
  x1: number; y1: number;
  x2: number; y2: number;
  cp1x: number; cp1y: number;
  cp2x: number; cp2y: number;
  thickness: number;
  depth: number;
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

interface PlaceNodesResult {
  placements: NodePlacement[];
  unusedTips: Tip[];
}

// ─── Constants ───────────────────────────────────────────────────────────────

const PHASE_COLORS = [
  '#c8a94a',  // amber
  '#4a9eff',  // blue
  '#a855f7',  // purple
  '#ef4444',  // red
  '#10b981',  // emerald
];

// Dynamic tree config — computed from canvas size so tree fills viewport naturally.
// No pan/zoom transform needed: tree coordinates ARE screen coordinates.
interface TreeConfig {
  baseX: number;
  baseY: number;
  topY: number;
  initialThickness: number;
  thicknessScale: number;
  branchLen: number;
  lengthScale: number;
  seed: number;
}

function makeTreeConfig(w: number, h: number): TreeConfig {
  // Scale tree to fill the viewport — trunk takes ~35% of height,
  // branches scale proportionally so the canopy fills the rest.
  const trunkHeight = h * 0.35;
  const scale = Math.min(w, h) / 900; // reference size 900px
  return {
    baseX: w / 2,
    baseY: h * 0.95,
    topY: h * 0.95 - trunkHeight,
    initialThickness: 24 * scale,
    thicknessScale: 0.64,
    branchLen: 145 * scale,
    lengthScale: 0.72,
    seed: 113,
  };
}

// ─── Seeded Random ───────────────────────────────────────────────────────────

class SeededRandom {
  private s: number;
  constructor(seed: number) {
    this.s = seed % 2147483647;
    if (this.s <= 0) this.s += 2147483646;
  }
  next(): number {
    this.s = (this.s * 16807) % 2147483647;
    return (this.s - 1) / 2147483646;
  }
  range(min: number, max: number): number {
    return min + this.next() * (max - min);
  }
}

// ─── Tree generation — recursive L-system branching ─────────────────────────

function generateTree(targetTips: number, seed: number, cfg: TreeConfig): { branches: Branch[]; tips: Tip[] } {
  // Binary-search minThickness so tip count ≥ targetTips (never fewer)
  let lo = 0.3, hi = 4.0;
  let bestResult: { branches: Branch[]; tips: Tip[] } = { branches: [], tips: [] };
  let bestDiff = Infinity;

  for (let iter = 0; iter < 16; iter++) {
    const mid = (lo + hi) / 2;
    const result = growTree(mid, seed, cfg);
    const tipCount = result.tips.length;

    // Only accept results with enough tips; prefer smallest surplus
    if (tipCount >= targetTips) {
      const surplus = tipCount - targetTips;
      if (surplus < bestDiff) {
        bestDiff = surplus;
        bestResult = result;
      }
      if (surplus <= 8) break; // close enough above target
      lo = mid; // raise threshold → fewer tips
    } else {
      hi = mid; // lower threshold → more tips
    }
  }

  // Fallback: if binary search never found enough, use last best
  if (bestResult.tips.length === 0) {
    bestResult = growTree(lo, seed, cfg);
  }

  return bestResult;
}

function growTree(minThickness: number, seed: number, cfg: TreeConfig): { branches: Branch[]; tips: Tip[] } {
  const rng = new SeededRandom(seed);
  const branches: Branch[] = [];
  const tips: Tip[] = [];

  function grow(
    x: number, y: number,
    angle: number,
    thickness: number,
    length: number,
    depth: number,
  ) {
    if (thickness < minThickness) {
      tips.push({ x, y, angle, depth });
      return;
    }

    const numChildren = depth === 0
      ? Math.round(rng.range(3, 5))
      : Math.round(rng.range(2, 3));

    const spreadDeg = depth === 0 ? 110
      : depth === 1 ? 70
      : depth === 2 ? 50
      : 35;
    const spreadRad = spreadDeg * (Math.PI / 180);

    for (let i = 0; i < numChildren; i++) {
      const t = numChildren === 1 ? 0.5 : i / (numChildren - 1);
      const baseAngle = angle - spreadRad / 2 + t * spreadRad;
      const wanderRad = rng.range(-0.12, 0.12);
      let childAngle = baseAngle + wanderRad;

      // Keep branches growing upward
      childAngle = Math.max(-Math.PI * 0.97, Math.min(-Math.PI * 0.03, childAngle));

      const childLength = length * cfg.lengthScale * rng.range(0.88, 1.12);
      const endX = x + childLength * Math.cos(childAngle);
      const endY = y + childLength * Math.sin(childAngle);

      // Bezier control points with perpendicular wander
      const perp = childAngle + Math.PI / 2;
      const w1 = rng.range(-18, 18);
      const w2 = rng.range(-12, 12);
      const cp1x = x + childLength * 0.35 * Math.cos(childAngle) + w1 * Math.cos(perp);
      const cp1y = y + childLength * 0.35 * Math.sin(childAngle) + w1 * Math.sin(perp);
      const cp2x = endX - childLength * 0.25 * Math.cos(childAngle) + w2 * Math.cos(perp);
      const cp2y = endY - childLength * 0.25 * Math.sin(childAngle) + w2 * Math.sin(perp);

      const childThickness = thickness * cfg.thicknessScale;

      branches.push({
        x1: x, y1: y, x2: endX, y2: endY,
        cp1x, cp1y, cp2x, cp2y,
        thickness: childThickness,
        depth,
      });

      grow(endX, endY, childAngle, childThickness, childLength, depth + 1);
    }
  }

  // Trunk bezier
  const trunkMidX = cfg.baseX + rng.range(-6, 6);
  branches.push({
    x1: cfg.baseX, y1: cfg.baseY,
    x2: cfg.baseX, y2: cfg.topY,
    cp1x: trunkMidX + 8, cp1y: cfg.baseY - 100,
    cp2x: trunkMidX - 6, cp2y: cfg.topY + 90,
    thickness: cfg.initialThickness,
    depth: -1,
  });

  // Grow from trunk top
  grow(
    cfg.baseX,
    cfg.topY,
    -Math.PI / 2,
    cfg.initialThickness * cfg.thicknessScale,
    cfg.branchLen,
    0,
  );

  // Sort tips: by depth then left-to-right
  tips.sort((a, b) => a.depth - b.depth || a.x - b.x);

  return { branches, tips };
}

// ─── Tip sorting — angular sweep from trunk top ─────────────────────────────

function sortTips(tips: Tip[], cfg: TreeConfig): Tip[] {
  const cx = cfg.baseX;
  const cy = cfg.topY;
  return [...tips].sort((a, b) => {
    const angleA = Math.atan2(a.y - cy, a.x - cx);
    const angleB = Math.atan2(b.y - cy, b.x - cx);
    return angleA - angleB;
  });
}

// ─── Bezier utilities ────────────────────────────────────────────────────────

function bezierPoint(
  x1: number, y1: number, cp1x: number, cp1y: number,
  cp2x: number, cp2y: number, x2: number, y2: number, t: number,
): { x: number; y: number } {
  const u = 1 - t;
  return {
    x: u*u*u*x1 + 3*u*u*t*cp1x + 3*u*t*t*cp2x + t*t*t*x2,
    y: u*u*u*y1 + 3*u*u*t*cp1y + 3*u*t*t*cp2y + t*t*t*y2,
  };
}

function bezierAngle(
  x1: number, y1: number, cp1x: number, cp1y: number,
  cp2x: number, cp2y: number, x2: number, y2: number, t: number,
): number {
  const u = 1 - t;
  const dx = 3*u*u*(cp1x - x1) + 6*u*t*(cp2x - cp1x) + 3*t*t*(x2 - cp2x);
  const dy = 3*u*u*(cp1y - y1) + 6*u*t*(cp2y - cp1y) + 3*t*t*(y2 - cp2y);
  return Math.atan2(dy, dx);
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

// ─── Node placement — leaf distribution across canopy ────────────────────────

const LEAF_CONFIG = {
  innerDensity: 0.5,
  minSpacing: 35,
  repulsionIters: 15,
  tetherStrength: 0.3,
  depthSpread: 0.6,
  leafAngleVar: 30,
};

function placeNodes(nodes: TreeNode[], tips: Tip[], branches: Branch[], cfg: TreeConfig): PlaceNodesResult {
  const sortedTips = sortTips(tips, cfg);
  const checkpoints = collectCheckpoints(nodes);
  if (checkpoints.length === 0 || sortedTips.length === 0) return { placements: [], unusedTips: [] };

  const cx = cfg.baseX;
  const cy = cfg.topY;
  const rng = new SeededRandom(cfg.seed + 777);

  const relocRng = new SeededRandom(cfg.seed + 888);
  const innerBranches = branches.filter(b => b.depth >= 0 && b.depth <= 2 && b.thickness > 1.5);

  const allPoints = sortedTips.map(tip => {
    if (LEAF_CONFIG.innerDensity <= 0 || innerBranches.length === 0 || relocRng.next() > LEAF_CONFIG.innerDensity) {
      return { x: tip.x, y: tip.y, angle: tip.angle, depth: tip.depth };
    }
    const b = innerBranches[Math.floor(relocRng.range(0, innerBranches.length))];
    const t = relocRng.range(0.25, 0.85);
    const pt = bezierPoint(b.x1, b.y1, b.cp1x, b.cp1y, b.cp2x, b.cp2y, b.x2, b.y2, t);
    const angle = bezierAngle(b.x1, b.y1, b.cp1x, b.cp1y, b.cp2x, b.cp2y, b.x2, b.y2, t);
    const perpAngle = angle + (relocRng.next() > 0.5 ? Math.PI / 2 : -Math.PI / 2);
    const offset = relocRng.range(4, 12);
    return {
      x: pt.x + Math.cos(perpAngle) * offset,
      y: pt.y + Math.sin(perpAngle) * offset,
      angle,
      depth: b.depth,
    };
  });

  const byDist = allPoints.map((pt, idx) => {
    const dx = pt.x - cx, dy = pt.y - cy;
    return { pt, idx, dist: Math.sqrt(dx * dx + dy * dy), depth: pt.depth, progressScore: 0 };
  });
  const ds = LEAF_CONFIG.depthSpread;
  const maxDist = Math.max(...byDist.map(t => t.dist)) || 1;
  const maxDepth = Math.max(...byDist.map(t => t.depth)) || 1;
  byDist.forEach(t => {
    t.progressScore = (1 - ds) * (t.depth / maxDepth) + ds * (t.dist / maxDist);
  });
  byDist.sort((a, b) => a.progressScore - b.progressScore);

  const totalCPs = checkpoints.length;
  const totalPts = byDist.length;
  const stride = totalPts / totalCPs;

  const placements: NodePlacement[] = [];
  const leafPositions: { x: number; y: number; origX: number; origY: number; angle: number }[] = [];
  const usedIndices = new Set<number>();

  checkpoints.forEach((_cp, i) => {
    const ptIdx = Math.min(totalPts - 1, Math.round(i * stride));
    usedIndices.add(ptIdx);
    const pt = byDist[ptIdx].pt;
    const angleVar = LEAF_CONFIG.leafAngleVar * Math.PI / 180;
    const leafAngle = pt.angle + rng.range(-angleVar, angleVar);
    leafPositions.push({ x: pt.x, y: pt.y, origX: pt.x, origY: pt.y, angle: leafAngle });
  });

  for (let iter = 0; iter < LEAF_CONFIG.repulsionIters; iter++) {
    for (let i = 0; i < leafPositions.length; i++) {
      for (let j = i + 1; j < leafPositions.length; j++) {
        const dx = leafPositions[j].x - leafPositions[i].x;
        const dy = leafPositions[j].y - leafPositions[i].y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
        if (dist < LEAF_CONFIG.minSpacing) {
          const force = (LEAF_CONFIG.minSpacing - dist) / 2 * 0.4;
          for (const [leaf, sign] of [[leafPositions[i], -1], [leafPositions[j], 1]] as const) {
            const rdx = leaf.x - cx, rdy = leaf.y - cy;
            const rLen = Math.sqrt(rdx * rdx + rdy * rdy) || 1;
            const tangentX = -rdy / rLen;
            const tangentY = rdx / rLen;
            const repDx = sign * dx / dist;
            const repDy = sign * dy / dist;
            const dot = repDx * tangentX + repDy * tangentY;
            const dir = dot >= 0 ? 1 : -1;
            leaf.x += dir * tangentX * force;
            leaf.y += dir * tangentY * force;
          }
        }
      }
      if (LEAF_CONFIG.tetherStrength > 0) {
        leafPositions[i].x += (leafPositions[i].origX - leafPositions[i].x) * LEAF_CONFIG.tetherStrength;
        leafPositions[i].y += (leafPositions[i].origY - leafPositions[i].y) * LEAF_CONFIG.tetherStrength;
      }
    }
  }

  checkpoints.forEach(({ checkpoint, phaseIndex }, i) => {
    const lp = leafPositions[i];
    placements.push({
      node: checkpoint, x: lp.x, y: lp.y,
      angle: lp.angle, branchAngle: lp.angle,
      phaseIndex, nodeType: 'checkpoint',
    });
  });

  const unusedTips: Tip[] = byDist
    .filter((_, idx) => !usedIndices.has(idx))
    .map(entry => ({ x: entry.pt.x, y: entry.pt.y, angle: entry.pt.angle, depth: entry.pt.depth }));

  return { placements, unusedTips };
}

// ─── Canvas rendering ───────────────────────────────────────────────────────

function drawBackground(ctx: CanvasRenderingContext2D, w: number, h: number) {
  // Deep night sky gradient — vertical, darkest at top
  const skyGrad = ctx.createLinearGradient(0, 0, 0, h);
  skyGrad.addColorStop(0, '#010208');
  skyGrad.addColorStop(0.35, '#020406');
  skyGrad.addColorStop(0.65, '#04080a');
  skyGrad.addColorStop(1, '#060d08');
  ctx.fillStyle = skyGrad;
  ctx.fillRect(0, 0, w, h);

  // Subtle radial glow behind the canopy — warm green, very faint
  const canopyGlow = ctx.createRadialGradient(w * 0.5, h * 0.4, 0, w * 0.5, h * 0.4, w * 0.55);
  canopyGlow.addColorStop(0, 'rgba(14, 30, 14, 0.35)');
  canopyGlow.addColorStop(0.6, 'rgba(8, 16, 8, 0.15)');
  canopyGlow.addColorStop(1, 'rgba(0, 0, 0, 0)');
  ctx.fillStyle = canopyGlow;
  ctx.fillRect(0, 0, w, h);

  // Ground plane — soft gradient at the base
  const groundY = h * 0.88;
  const groundGrad = ctx.createLinearGradient(0, groundY, 0, h);
  groundGrad.addColorStop(0, 'rgba(0, 0, 0, 0)');
  groundGrad.addColorStop(0.3, 'rgba(8, 12, 6, 0.2)');
  groundGrad.addColorStop(1, 'rgba(10, 16, 8, 0.35)');
  ctx.fillStyle = groundGrad;
  ctx.fillRect(0, groundY, w, h - groundY);

  // Ground line — very subtle mossy horizon
  ctx.beginPath();
  ctx.moveTo(0, h * 0.93);
  // Slightly wavy ground line
  for (let x = 0; x <= w; x += 40) {
    const yOff = Math.sin(x * 0.008) * 3 + Math.sin(x * 0.023) * 1.5;
    ctx.lineTo(x, h * 0.93 + yOff);
  }
  ctx.lineTo(w, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  ctx.fillStyle = 'rgba(6, 14, 6, 0.25)';
  ctx.fill();

  // Scattered ambient particles — fireflies / spores
  const particleRng = new SeededRandom(42);
  const particleCount = Math.floor(w * h / 18000);
  for (let i = 0; i < particleCount; i++) {
    const px = particleRng.range(0, w);
    const py = particleRng.range(h * 0.05, h * 0.85);
    const pr = particleRng.range(0.3, 1.2);
    const alpha = particleRng.range(0.08, 0.25);
    // Warm tones for fireflies, cool tones for distant stars
    const isWarm = py > h * 0.4;
    const r = isWarm ? Math.round(particleRng.range(140, 200)) : Math.round(particleRng.range(120, 180));
    const g = isWarm ? Math.round(particleRng.range(160, 220)) : Math.round(particleRng.range(140, 190));
    const b = isWarm ? Math.round(particleRng.range(80, 120)) : Math.round(particleRng.range(170, 230));
    ctx.beginPath();
    ctx.arc(px, py, pr, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(${r},${g},${b},${alpha})`;
    ctx.fill();
  }
}

function drawTree(ctx: CanvasRenderingContext2D, branches: Branch[]) {
  const sorted = [...branches].sort((a, b) => b.thickness - a.thickness);

  sorted.forEach(branch => {
    const t = branch.thickness;
    const darkness = Math.max(10, 35 - branch.depth * 3);
    const r = darkness;
    const g = Math.round(darkness * 0.65);
    const bl = Math.round(darkness * 0.25);

    ctx.beginPath();
    ctx.moveTo(branch.x1, branch.y1);
    ctx.bezierCurveTo(branch.cp1x, branch.cp1y, branch.cp2x, branch.cp2y, branch.x2, branch.y2);
    ctx.strokeStyle = `rgb(${r},${g},${bl})`;
    ctx.lineWidth = Math.max(0.5, t);
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.stroke();

    // Bark texture for thick branches
    if (t > 4) {
      ctx.beginPath();
      ctx.moveTo(branch.x1 + 1, branch.y1);
      ctx.bezierCurveTo(
        branch.cp1x + 2, branch.cp1y,
        branch.cp2x - 1, branch.cp2y,
        branch.x2 + 1, branch.y2,
      );
      ctx.strokeStyle = `rgba(${r + 8},${g + 5},${bl + 2},0.35)`;
      ctx.lineWidth = Math.max(1, t * 0.4);
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
  lockedOpacity: 0.2,
};

// ─── Draw a single botanical leaf ────────────────────────────────────────────

function drawLeaf(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, angle: number,
  size: number, color: string,
  state: 'dormant' | 'budding' | 'growing' | 'bloomed',
  isHovered: boolean, isSelected: boolean,
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
    fillColor = color; strokeColor = '#ffffffaa'; alpha = 1;
  } else if (state === 'dormant') {
    fillColor = '#111'; strokeColor = '#333'; alpha = LEAF_SHAPE.lockedOpacity;
  } else if (state === 'growing') {
    fillColor = color + '99'; strokeColor = color + 'cc'; alpha = 0.85;
  } else {
    fillColor = color + '55'; strokeColor = color + '88'; alpha = 0.7;
  }

  ctx.globalAlpha = alpha;

  if (state === 'bloomed' && LEAF_SHAPE.glowRadius > 0) {
    const glow = ctx.createRadialGradient(len * 0.4, 0, 2, len * 0.4, 0, LEAF_SHAPE.glowRadius + size);
    glow.addColorStop(0, color + '44');
    glow.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, LEAF_SHAPE.glowRadius + size, 0, Math.PI * 2);
    ctx.fillStyle = glow;
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

  ctx.beginPath();
  ctx.moveTo(1, 0);
  ctx.lineTo(len * 0.85, 0);
  ctx.strokeStyle = state === 'dormant' ? '#222' : (state === 'bloomed' ? '#ffffff44' : color + '55');
  ctx.lineWidth = 0.6;
  ctx.stroke();

  const veinCount = Math.max(2, Math.floor(size / 5));
  for (let v = 1; v <= veinCount; v++) {
    const t = v / (veinCount + 1);
    const vx = len * t * 0.85;
    const vw = w * (1 - t * 0.6) * 0.6;
    const veinStroke = state === 'dormant' ? '#1a1a1a' : (state === 'bloomed' ? '#ffffff22' : color + '33');
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

    drawLeaf(ctx, x, y, angle, LEAF_SHAPE.size, color, state, isHovered, isSelected);

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
  const checkpoint = node.type === 'leaf'
    ? (Array.isArray(node.tasks)
        ? { mastery_criteria: node.tasks[0]?.description ?? '', exercises: [] as string[], notes: node.tasks[0]?.notes ?? '', completed: node.tasks[0]?.completed ?? false }
        : (node.tasks as { mastery_criteria?: string; exercises?: string[]; notes?: string; completed?: boolean } | null))
    : null;
  const [notesText, setNotesText] = useState(checkpoint?.notes ?? '');
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
    invoke<MimirResource[]>('get_node_resources', { nodeId: node.id })
      .then(setMimiResources)
      .catch(() => {});
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

  function handleSave() {
    onSave({ title, description, resources: resources.length > 0 ? resources : null });
  }

  function handleToggleComplete() {
    if (!checkpoint) return;
    const newCompleted = !checkpoint.completed;
    onSave({
      progress: newCompleted ? 100 : 0,
      tasks: { ...checkpoint, completed: newCompleted },
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

  // Dynamic tree config — recomputed when canvas size changes
  const treeConfig = useMemo(() => makeTreeConfig(size.w, size.h), [size.w, size.h]);

  // Count leaf nodes for tree generation (terminal nodes = no children)
  const leafCount = useMemo(() => {
    const hasChildren = new Set<string>();
    nodes.forEach(n => { if (n.parent_id) hasChildren.add(n.parent_id); });
    return nodes.filter(n => !hasChildren.has(n.id) && n.type !== 'trunk').length;
  }, [nodes]);

  // Generate procedural tree — deterministic from seed + leaf count + canvas size
  const { branches, tips } = useMemo(
    () => generateTree(Math.max(leafCount, 8), treeConfig.seed + leafCount, treeConfig),
    [leafCount, treeConfig],
  );

  // Place data nodes onto branch tips
  const { placements, unusedTips } = useMemo(
    () => placeNodes(nodes, tips, branches, treeConfig),
    [nodes, tips, branches, treeConfig],
  );

  // Sync selected node to Mimir context
  useEffect(() => {
    setMimirContext({
      treeId: selectedNode ? treeId : null,
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

    // Background fills the full canvas (not transformed)
    drawBackground(ctx, size.w, size.h);

    // Tree + leaves drawn in world space via pan/zoom transform
    const t = transform.current;
    ctx.save();
    ctx.translate(t.x, t.y);
    ctx.scale(t.scale, t.scale);
    drawTree(ctx, branches);
    drawLeaves(ctx, placements, unusedTips, selectedNode?.id ?? null, hoveredNode, treeConfig.baseX);
    ctx.restore();

  }, [branches, placements, unusedTips, selectedNode, hoveredNode, size, treeConfig, renderTick]);

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

      if (updates.progress !== undefined) {
        await invoke('recalculate_tree_progress', { treeId });
        await invoke('recalculate_unlocks', { treeId });
        await invoke('update_project_progress', { projectId });
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
        />
      )}
    </div>
  );
}
