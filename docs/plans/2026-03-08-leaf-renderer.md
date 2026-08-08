# Organic Leaf Renderer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace circle-based checkpoint rendering in YggdrasilTree.tsx with organic botanical leaf shapes, with inner-to-outer progression placement, arc-constrained repulsion, and hover/click interaction.

**Architecture:** The change is entirely within `src/components/YggdrasilTree.tsx`. We replace three systems: (1) `placeNodes()` — new placement with inner-branch relocation + arc-constrained repulsion, (2) `drawNodes()` — replaced with `drawLeaves()` using botanical bezier leaf shapes, (3) click/hover hit-testing — updated for leaf geometry. The L-system tree generation (`generateTree`, `growTree`) stays unchanged except for config values.

**Tech Stack:** React 19 + TypeScript, HTML Canvas 2D, existing L-system code.

**User's chosen config from playground:**
- seed: 113, branchLength: 142
- leafSize: 14, leafWidth: 0.45, leafPoint: 0.3, leafCurve: 0.15, leafAngleVar: 30°, showVein: true
- innerDensity: 0.5 (default), minSpacing: 10, repulsionIters: 20, tetherStrength: 0, depthSpread: 0.3
- glowRadius: 7, lockedOpacity: 0.2
- labelMode: hover, colors: ['#10b981','#c8a94a','#4a9eff','#a855f7','#ef4444']

---

## File Map

All changes in one file: `src/components/YggdrasilTree.tsx`

**Functions to ADD:**
- `bezierPoint()` — sample point on cubic bezier
- `bezierAngle()` — tangent angle of cubic bezier
- `drawLeaf()` — render one botanical leaf shape
- `drawLeaves()` — render all leaf placements (replaces `drawNodes`)

**Functions to MODIFY:**
- `TREE_CONFIG` — update seed to 113, branchLengthBase to 142
- `placeNodes()` — completely rewrite with inner-branch relocation + arc repulsion
- `NodePlacement` interface — add `branchAngle` field
- `drawNodes()` — remove entirely (replaced by `drawLeaves`)
- Main component render effect — call `drawLeaves` instead of `drawNodes`, pass `branches`
- `handleCanvasClick` — update hit radius for leaf geometry
- Main component `useMemo` for placements — pass `branches` to new `placeNodes`

**Functions UNCHANGED:** `generateTree`, `growTree`, `SeededRandom`, `sortTips`, `collectCheckpoints`, `drawBackground`, `drawTree`, `NodePanel`, all CRUD handlers.

---

### Task 1: Add bezier utility functions

**Files:**
- Modify: `src/components/YggdrasilTree.tsx` — insert after the `sortTips` function (around line 233)

**Step 1: Add `bezierPoint` and `bezierAngle` functions**

Insert after the `sortTips` function:

```typescript
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
```

**Step 2: Verify no TypeScript errors**

Run: `npx tsc --noEmit`
Expected: No new errors from these additions (they're pure functions, not yet called).

**Step 3: Commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "feat(tree): add bezier sampling utility functions"
```

---

### Task 2: Update TREE_CONFIG and NodePlacement

**Files:**
- Modify: `src/components/YggdrasilTree.tsx` lines 76-85 (TREE_CONFIG) and line 57-64 (NodePlacement)

**Step 1: Update TREE_CONFIG**

Change seed and branchLengthBase:

```typescript
const TREE_CONFIG = {
  trunkBaseX: 600,
  trunkBaseY: 880,
  trunkTopY: 560,
  initialThickness: 24,
  thicknessScale: 0.64,
  branchLengthBase: 142,   // was 145
  lengthScale: 0.72,
  seed: 113,                // was 42
};
```

**Step 2: Add `branchAngle` to NodePlacement**

```typescript
interface NodePlacement {
  node: TreeNode;
  x: number;
  y: number;
  angle: number;
  branchAngle: number;      // ← ADD: angle of the branch the leaf sits on
  phaseIndex: number;
  nodeType: 'checkpoint' | 'skill' | 'phase';
}
```

**Step 3: Commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "feat(tree): update tree config (seed 113, branch length 142) and NodePlacement type"
```

---

### Task 3: Rewrite `placeNodes()` with inner-branch relocation and arc repulsion

**Files:**
- Modify: `src/components/YggdrasilTree.tsx` — replace the entire `placeNodes` function (lines 307-403)

**Step 1: Replace `placeNodes` with the new implementation**

Replace the entire function body:

```typescript
// ─── Leaf placement constants ────────────────────────────────────────────────

const LEAF_CONFIG = {
  innerDensity: 0.5,
  minSpacing: 10,
  repulsionIters: 20,
  tetherStrength: 0,
  depthSpread: 0.3,
  leafAngleVar: 30,
};

// ─── Node placement — inner-to-outer with arc-constrained repulsion ─────────

function placeNodes(nodes: TreeNode[], tips: Tip[], branches: Branch[]): NodePlacement[] {
  const sortedTips = sortTips(tips);
  const checkpoints = collectCheckpoints(nodes);
  if (checkpoints.length === 0 || sortedTips.length === 0) return [];

  const cx = TREE_CONFIG.trunkBaseX;
  const cy = TREE_CONFIG.trunkTopY;
  const rng = new SeededRandom(TREE_CONFIG.seed + 777);

  // ── Relocate some tips to inner branches ──
  const relocRng = new SeededRandom(TREE_CONFIG.seed + 888);
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

  // ── Sort by progression: inner/shallow = foundational, outer/deep = advanced ──
  const byDist = allPoints.map((pt, idx) => {
    const dx = pt.x - cx, dy = pt.y - cy;
    return { pt, idx, dist: Math.sqrt(dx * dx + dy * dy), depth: pt.depth };
  });
  const ds = LEAF_CONFIG.depthSpread;
  const maxDist = Math.max(...byDist.map(t => t.dist)) || 1;
  const maxDepth = Math.max(...byDist.map(t => t.depth)) || 1;
  byDist.forEach(t => {
    t.progressScore = (1 - ds) * (t.depth / maxDepth) + ds * (t.dist / maxDist);
  });
  byDist.sort((a, b) => a.progressScore - b.progressScore);

  // ── Map sorted tips to checkpoints (stride evenly) ──
  const totalCPs = checkpoints.length;
  const totalPts = byDist.length;
  const stride = totalPts / totalCPs;

  const placements: NodePlacement[] = [];
  const positionById = new Map<string, { x: number; y: number; angle: number }>();

  // Mutable positions for repulsion
  const leafPositions: { x: number; y: number; origX: number; origY: number; angle: number }[] = [];

  checkpoints.forEach(({ checkpoint, phaseIndex }, i) => {
    const ptIdx = Math.min(totalPts - 1, Math.round(i * stride));
    const pt = byDist[ptIdx].pt;
    const angleVar = LEAF_CONFIG.leafAngleVar * Math.PI / 180;
    const leafAngle = pt.angle + rng.range(-angleVar, angleVar);
    leafPositions.push({ x: pt.x, y: pt.y, origX: pt.x, origY: pt.y, angle: leafAngle });
  });

  // ── Arc-constrained repulsion ──
  for (let iter = 0; iter < LEAF_CONFIG.repulsionIters; iter++) {
    for (let i = 0; i < leafPositions.length; i++) {
      for (let j = i + 1; j < leafPositions.length; j++) {
        const dx = leafPositions[j].x - leafPositions[i].x;
        const dy = leafPositions[j].y - leafPositions[i].y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
        if (dist < LEAF_CONFIG.minSpacing) {
          const force = (LEAF_CONFIG.minSpacing - dist) / 2 * 0.4;
          // Push along canopy arc (tangent to radial from trunk top)
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
      // Tether
      const tether = LEAF_CONFIG.tetherStrength;
      if (tether > 0) {
        leafPositions[i].x += (leafPositions[i].origX - leafPositions[i].x) * tether;
        leafPositions[i].y += (leafPositions[i].origY - leafPositions[i].y) * tether;
      }
    }
  }

  // ── Build final placements ──
  checkpoints.forEach(({ checkpoint, phaseIndex }, i) => {
    const lp = leafPositions[i];
    placements.push({
      node: checkpoint,
      x: lp.x,
      y: lp.y,
      angle: lp.angle,
      branchAngle: lp.angle,
      phaseIndex,
      nodeType: 'checkpoint',
    });
    positionById.set(checkpoint.id, { x: lp.x, y: lp.y, angle: lp.angle });
  });

  // ── Place skills at centroid of their checkpoints ──
  const childrenOf = new Map<string | null, TreeNode[]>();
  nodes.forEach(n => {
    const key = n.parent_id ?? null;
    if (!childrenOf.has(key)) childrenOf.set(key, []);
    childrenOf.get(key)!.push(n);
  });

  const roots = childrenOf.get(null) ?? [];
  const root = roots[0];
  if (!root) return placements;
  const phases = childrenOf.get(root.id) ?? [];

  phases.forEach((phase, phaseIdx) => {
    const phaseChildren = childrenOf.get(phase.id) ?? [];
    const hasSkillLevel = phaseChildren.some(c => (childrenOf.get(c.id) ?? []).length > 0);
    const skillPositions: { x: number; y: number }[] = [];

    if (hasSkillLevel) {
      phaseChildren.forEach(skill => {
        const descendantPositions: { x: number; y: number }[] = [];
        function collectDescendantPositions(nodeId: string) {
          const pos = positionById.get(nodeId);
          if (pos) descendantPositions.push(pos);
          const children = childrenOf.get(nodeId) ?? [];
          children.forEach(c => collectDescendantPositions(c.id));
        }
        collectDescendantPositions(skill.id);
        if (descendantPositions.length === 0) return;
        const skx = descendantPositions.reduce((s, p) => s + p.x, 0) / descendantPositions.length;
        const sky = descendantPositions.reduce((s, p) => s + p.y, 0) / descendantPositions.length;
        placements.push({
          node: skill, x: skx, y: sky,
          angle: -Math.PI / 2, branchAngle: -Math.PI / 2,
          phaseIndex: phaseIdx, nodeType: 'skill',
        });
        positionById.set(skill.id, { x: skx, y: sky, angle: -Math.PI / 2 });
        skillPositions.push({ x: skx, y: sky });
      });
    }

    const phasePositions = skillPositions.length > 0
      ? skillPositions
      : placements.filter(p => p.phaseIndex === phaseIdx && p.nodeType === 'checkpoint').map(p => ({ x: p.x, y: p.y }));
    if (phasePositions.length === 0) return;
    const px = phasePositions.reduce((s, p) => s + p.x, 0) / phasePositions.length;
    const py = phasePositions.reduce((s, p) => s + p.y, 0) / phasePositions.length;
    placements.push({
      node: phase, x: px, y: py,
      angle: -Math.PI / 2, branchAngle: -Math.PI / 2,
      phaseIndex: phaseIdx, nodeType: 'phase',
    });
  });

  return placements;
}
```

**Step 2: Update the `useMemo` call in the main component**

The `useMemo` for placements (around line 1040) needs to pass `branches`:

```typescript
// was: () => placeNodes(nodes, tips),
const placements = useMemo(
  () => placeNodes(nodes, tips, branches),
  [nodes, tips, branches],
);
```

**Step 3: Commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "feat(tree): rewrite placeNodes with inner-branch relocation and arc repulsion"
```

---

### Task 4: Replace `drawNodes` with `drawLeaves`

**Files:**
- Modify: `src/components/YggdrasilTree.tsx` — replace the `drawNodes` function (lines 451-561)

**Step 1: Remove `drawNodes` and replace with `drawLeaf` + `drawLeaves`**

```typescript
// ─── Leaf shape constants ────────────────────────────────────────────────────

const LEAF_SHAPE = {
  size: 14,
  width: 0.45,
  pointiness: 0.3,
  curve: 0.15,
  glowRadius: 7,
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

  // State-based styling
  let fillColor: string, strokeColor: string, alpha: number;
  if (state === 'bloomed') {
    fillColor = color;
    strokeColor = '#ffffffaa';
    alpha = 1;
  } else if (state === 'dormant') {
    fillColor = '#111';
    strokeColor = '#333';
    alpha = LEAF_SHAPE.lockedOpacity;
  } else if (state === 'growing') {
    fillColor = color + '99';
    strokeColor = color + 'cc';
    alpha = 0.85;
  } else {
    // budding
    fillColor = color + '55';
    strokeColor = color + '88';
    alpha = 0.7;
  }

  ctx.globalAlpha = alpha;

  // Glow for completed
  if (state === 'bloomed' && LEAF_SHAPE.glowRadius > 0) {
    const glow = ctx.createRadialGradient(len * 0.4, 0, 2, len * 0.4, 0, LEAF_SHAPE.glowRadius + size);
    glow.addColorStop(0, color + '44');
    glow.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, LEAF_SHAPE.glowRadius + size, 0, Math.PI * 2);
    ctx.fillStyle = glow;
    ctx.fill();
  }

  // Hover glow
  if (isHovered) {
    const hg = ctx.createRadialGradient(len * 0.4, 0, 2, len * 0.4, 0, size * 2);
    hg.addColorStop(0, color + '55');
    hg.addColorStop(1, color + '00');
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 2, 0, Math.PI * 2);
    ctx.fillStyle = hg;
    ctx.fill();
  }

  // Selection ring
  if (isSelected) {
    ctx.beginPath();
    ctx.arc(len * 0.4, 0, size * 1.3, 0, Math.PI * 2);
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.globalAlpha = 0.8;
    ctx.stroke();
    ctx.globalAlpha = alpha;
  }

  // Leaf body — two cubic beziers
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.bezierCurveTo(
    pointOff * 0.3, -(w * 0.5 + curve),
    pointOff, -(w + curve * 0.5),
    len, 0,
  );
  ctx.bezierCurveTo(
    pointOff, (w + curve * 0.5),
    pointOff * 0.3, (w * 0.5 + curve),
    0, 0,
  );
  ctx.closePath();
  ctx.fillStyle = fillColor;
  ctx.fill();
  ctx.strokeStyle = strokeColor;
  ctx.lineWidth = isHovered ? 1.8 : 1;
  ctx.stroke();

  // Midrib vein
  ctx.beginPath();
  ctx.moveTo(1, 0);
  ctx.lineTo(len * 0.85, 0);
  ctx.strokeStyle = state === 'dormant' ? '#222' : (state === 'bloomed' ? '#ffffff44' : color + '55');
  ctx.lineWidth = 0.6;
  ctx.stroke();

  // Side veins
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

  // Stem connecting back to branch
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(-size * 0.4, 0);
  ctx.strokeStyle = '#1a1008';
  ctx.lineWidth = 1.2;
  ctx.stroke();

  ctx.globalAlpha = 1;
  ctx.restore();
}

// ─── Draw all leaves + phase/skill labels ────────────────────────────────────

function drawLeaves(
  ctx: CanvasRenderingContext2D,
  placements: NodePlacement[],
  selectedId: string | null,
  hoveredId: string | null,
) {
  // Draw checkpoints (leaves) first, then skill labels, then phase labels
  const order: Record<string, number> = { checkpoint: 0, skill: 1, phase: 2 };
  const sorted = [...placements].sort((a, b) => order[a.nodeType] - order[b.nodeType]);

  sorted.forEach(({ node, x, y, angle, phaseIndex, nodeType }) => {
    const color = PHASE_COLORS[phaseIndex % PHASE_COLORS.length];
    const isLocked = node.is_locked ?? false;
    const progress = node.progress ?? 0;
    const isSelected = node.id === selectedId;
    const isHovered = node.id === hoveredId;

    if (nodeType === 'checkpoint') {
      const state: 'dormant' | 'budding' | 'growing' | 'bloomed' =
        isLocked ? 'dormant'
        : progress === 0 ? 'budding'
        : progress < 100 ? 'growing'
        : 'bloomed';

      drawLeaf(ctx, x, y, angle, LEAF_SHAPE.size, color, state, isHovered, isSelected);

      // Label on hover
      if (isHovered || isSelected) {
        const label = node.title.length > 22 ? node.title.slice(0, 22) + '…' : node.title;
        ctx.font = '400 10px Inter, sans-serif';
        ctx.fillStyle = '#ddd';
        ctx.textBaseline = 'middle';
        const labelOffset = LEAF_SHAPE.size + 10;
        if (x > TREE_CONFIG.trunkBaseX) {
          ctx.textAlign = 'left';
          ctx.fillText(label, x + labelOffset, y);
        } else {
          ctx.textAlign = 'right';
          ctx.fillText(label, x - labelOffset, y);
        }
        ctx.textAlign = 'left';
      }
    } else {
      // Skill and Phase nodes — render as text badges (no circles)
      const fontSize = nodeType === 'phase' ? 12 : 10;
      const fontWeight = nodeType === 'phase' ? '600' : '400';
      ctx.font = `${fontWeight} ${fontSize}px Inter, sans-serif`;
      ctx.fillStyle = color + (nodeType === 'phase' ? 'cc' : '88');
      ctx.textBaseline = 'middle';

      const label = node.title.length > 20 ? node.title.slice(0, 20) + '…' : node.title;
      if (x > TREE_CONFIG.trunkBaseX) {
        ctx.textAlign = 'left';
        ctx.fillText(label, x + (nodeType === 'phase' ? 18 : 14), y);
      } else {
        ctx.textAlign = 'right';
        ctx.fillText(label, x - (nodeType === 'phase' ? 18 : 14), y);
      }

      // Subtle underline for phase labels
      if (nodeType === 'phase') {
        const textW = ctx.measureText(label).width;
        const startX = x > TREE_CONFIG.trunkBaseX ? x + 18 : x - 18 - textW;
        ctx.beginPath();
        ctx.moveTo(startX, y + fontSize / 2 + 2);
        ctx.lineTo(startX + textW, y + fontSize / 2 + 2);
        ctx.strokeStyle = color + '44';
        ctx.lineWidth = 1;
        ctx.stroke();
      }
      ctx.textAlign = 'left';
    }
  });
}
```

**Step 2: Commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "feat(tree): replace circle nodes with botanical leaf shapes"
```

---

### Task 5: Add hover state and update render effect + hit testing

**Files:**
- Modify: `src/components/YggdrasilTree.tsx` — main component section

**Step 1: Add hoveredNode state**

In the main component state declarations (around line 1014):

```typescript
const [hoveredNode, setHoveredNode] = useState<string | null>(null);
```

**Step 2: Update the rAF render effect**

Replace the `drawNodes` call (around line 1107):

```typescript
// was: drawNodes(ctx, placements, selectedNode?.id ?? null);
drawLeaves(ctx, placements, selectedNode?.id ?? null, hoveredNode);
```

Add `hoveredNode` to the dependency array:

```typescript
}, [branches, placements, selectedNode, hoveredNode, size, renderTick]);
```

**Step 3: Update `handleCanvasClick` hit radius**

In `handleCanvasClick` (around line 1252), update the hit radius:

```typescript
// was: const hitRadius = p.nodeType === 'phase' ? 20 : p.nodeType === 'skill' ? 16 : 14;
const hitRadius = p.nodeType === 'phase' ? 20 : p.nodeType === 'skill' ? 16 : LEAF_SHAPE.size * 1.5;
```

**Step 4: Add hover detection to `handleMouseMove`**

In `handleMouseMove`, after the existing drag logic (around line 1283), add hover detection for non-dragging moves:

```typescript
function handleMouseMove(e: React.MouseEvent) {
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
  const canvas = canvasRef.current;
  if (!canvas) return;
  const rect = canvas.getBoundingClientRect();
  const t = transform.current;
  const mx = (e.clientX - rect.left - t.x) / t.scale;
  const my = (e.clientY - rect.top - t.y) / t.scale;

  let closest: string | null = null;
  let closestDist = Infinity;
  placements.forEach(p => {
    if (p.nodeType !== 'checkpoint') return;
    const dist = Math.hypot(p.x - mx, p.y - my);
    if (dist < LEAF_SHAPE.size * 1.5 && dist < closestDist) {
      closest = p.node.id;
      closestDist = dist;
    }
  });
  if (closest !== hoveredNode) {
    setHoveredNode(closest);
  }
}
```

**Step 5: Clear hover on mouse leave**

Update the `onMouseLeave` handler on the canvas (around line 1340):

```typescript
onMouseLeave={() => { isDragging.current = false; dragMoved.current = false; setHoveredNode(null); }}
```

**Step 6: Update cursor style**

```typescript
cursor: isDragging.current ? 'grabbing' : hoveredNode ? 'pointer' : 'grab',
```

**Step 7: Commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "feat(tree): add hover state, leaf hit testing, and cursor feedback"
```

---

### Task 6: Verify and clean up

**Step 1: Run TypeScript check**

Run: `npx tsc --noEmit`
Expected: No errors

**Step 2: Run dev server to visually verify**

Run: `npm run tauri dev`
Expected: Tree renders with leaf shapes instead of circles. Hover highlights leaves. Click opens panel. Inner leaves sit on shallow branches. Phase/skill labels are text-only.

**Step 3: Remove dead code**

If the old `drawNodes` function still exists, delete it entirely. Search for any references to `drawNodes` and remove them.

**Step 4: Final commit**

```bash
git add src/components/YggdrasilTree.tsx
git commit -m "refactor(tree): remove old circle-based drawNodes"
```

---

## Summary of changes

| What | Before | After |
|---|---|---|
| Checkpoint rendering | Colored circles with rings | Botanical leaf shapes with veins |
| Placement | Linear stride through angular-sorted tips | Inner-branch relocation + arc-constrained repulsion |
| Labels | Always visible for all nodes | Hover-only for checkpoints, text badges for phases/skills |
| Hover | None | Glow + label on hover, pointer cursor |
| Hit testing | Circle radius | Leaf-sized radius |
| Tree config | seed=42, branchLength=145 | seed=113, branchLength=142 |
| Node separation | None (overlapping allowed) | Arc-constrained repulsion, minSpacing=10 |
