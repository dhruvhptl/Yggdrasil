# SkillsPage Graph Redesign — Implementation Spec

**Goal:** Make the skill knowledge graph readable, navigable, and visually coherent at 100+ nodes — replacing the current broken horizontal smear with a domain-lane + depth-band layout, connection-aware hover highlighting, zoom-adaptive labels, and a small canvas legend.

**Architecture:** All changes are isolated to `src/pages/SkillsPage.tsx`. No backend changes. The layout function, canvas draw pipeline, and interaction handlers are the three units being replaced or extended.

**Tech Stack:** React 19, HTML Canvas 2D, existing Tauri invoke/state plumbing.

---

## 1. Layout — `layoutSkillGraph`

### Domain lanes

- Compute the ordered list of unique domain names from `skills`.
- Allocate canvas width proportionally: each domain gets `(domainSkillCount / totalSkills) * usableWidth` pixels, minimum 80px per lane.
- Lane left/right boundaries are stored per domain for use in label clamping and lane-header drawing.
- A faint vertical separator line is drawn at each lane boundary (rgba white, opacity 0.04).

### Depth bands (Y axis)

- BFS from skills with no prerequisite edges → `depth` map (0 = foundation, max 6).
- Skills unreachable by BFS (cycles, disconnected) default to depth 0.
- Minimum band height: `Math.max(80, usableHeight / (maxDepth + 1))` px.
- `depthToY(d)` = `canvasHeight - padBottom - d * bandHeight` — depth 0 at bottom, depth 6 near top.
- Total canvas logical height = `padTop + (maxDepth + 1) * bandHeight + padBottom`. When this exceeds the container height the user pans vertically.

### Node placement within a cell (domain × depth)

- Each cell gets all skills at that (domain, depth) pair.
- Primary X: evenly spaced across 80% of the lane width, centred in the lane.
- Y: `depthToY(d)` ± up to `bandHeight * 0.35` of random vertical jitter (seeded, stable).
- X jitter: ±12px random per node (seeded).
- **One-pass repulsion:** after initial placement, iterate once over all node pairs within the same cell. If two nodes are closer than `minDist = 28px`, push them apart along their difference vector by half the overlap. Cap at 3 iterations. This prevents node pileup without a full force simulation.

### Lane headers

- Each domain gets a label drawn at the **top** of its lane (above the topmost node in that lane, or at `padTop - 20` if no nodes).
- Font: `600 11px DM Sans`, domain color at 0.65 opacity.
- No per-depth-band domain labels (those were the `"Domain (L2)"` strings that cluttered the previous version).

### Gap nodes

- Controlled by the existing `showGaps` toggle in the sidebar.
- When visible, gap nodes are placed at depth 0, spread across the full canvas width (not per-domain), with amber color. They share the depth-0 band with seed skills.

---

## 2. Canvas Draw Pipeline

Draw order (back to front):

1. Background (existing `drawBackground` — starfield + nebula, unchanged)
2. Faint lane separator lines
3. Depth-band guide labels (left edge of canvas, fixed in world space): "Foundations" at d=0, "Intermediate" at d=ceil(maxDepth/2), "Advanced" at d=maxDepth — only when maxDepth ≥ 2
4. Dependency edges (only for hovered or selected node — see §3)
5. Skill nodes (all, with per-node opacity — see §3)
6. Skill name labels (zoom-adaptive — see §3)
7. Lane header labels
8. Hover tooltip (existing `drawHoverLabel`, unchanged)
9. Canvas legend (fixed screen-space, bottom-left — see §4)

---

## 3. Labels, Hover & Dimming

### Zoom-adaptive labels

Three zoom tiers based on `transform.scale`:

| Zoom | Labels shown |
|------|-------------|
| < 1.2× | Seeds (state='seed') + L3/L4/L5 skills only |
| 1.2× – 1.5× | All L2+ skills |
| ≥ 1.5× | All skills |

Label format: skill name truncated to 20 chars. Font `500 9px DM Sans`. Positioned 10px right of node centre. X-clamped to not overflow the right edge of the node's domain lane. Selected skill label is always shown at full opacity regardless of zoom.

### Hover logic

When `hoveredId` is set:

- **Hovered node:** full opacity (1.0), existing tooltip drawn.
- **Direct neighbors** (any edge direction, any relationship): opacity 0.90.
- **All other nodes:** opacity **0.25** (not 8% — readable but clearly de-emphasised).
- **Edges:** draw all edges connected to hovered node. Prereq/part_of edges: solid indigo `#818cf8` at 0.55 opacity, lineWidth 1.4. Related/co_occurs edges: dashed `#64748b` at 0.30 opacity, lineWidth 0.8. Specialization edges: solid teal `#14b8a6` at 0.40 opacity.
- Labels: neighbors show their label regardless of zoom tier. Non-neighbors hide labels.

### Select logic (click)

Same as hover but persistent. Clicking the same node deselects. Clicking elsewhere deselects. `SkillPanel` opens for selected skill.

### Double-click — zoom to neighborhood

On double-click of a node:
1. Collect the node + all direct neighbors into a bounding box (min/max x/y of their positions).
2. Add 80px padding on all sides.
3. Compute scale = `min(containerW / bbW, containerH / bbH)`, clamped to [0.5, 3.0].
4. Compute translate so the bounding box centre maps to canvas centre.
5. Animate transform over 350ms using `easeOutCubic`. Use `requestAnimationFrame` — store start transform + target transform in refs, lerp each frame.

### Fit-all button

Small circular button, bottom-right of the canvas area (not canvas world space — fixed in the container div). Icon: `Maximize2` from lucide-react. On click: compute bounding box of all placements, fit with 40px padding, animate same as double-click zoom (350ms easeOutCubic).

---

## 4. Canvas Legend

Drawn in screen space (not affected by pan/zoom). Fixed position: 16px from left, 16px from bottom of canvas.

Contents (top to bottom):
- **Row 1 — Node size = level:** five circles (radii matching NODE_R[1]–NODE_R[5]), labelled "L1" to "L5" below each. Spacing 18px.
- **Row 2 — State markers:** gold ring = seed, green pulse ring = growth target. Small dot + text label inline.
- **Row 3 — Edge types:** 24px solid indigo line + "prereq", 24px dashed gray line + "related".

Background pill: `rgba(4,8,20,0.72)`, border `rgba(255,255,255,0.07)`, border-radius 8px. Total size ~180×88px. Font `400 9px DM Sans`, color `#64748b` for labels, node colors for dots.

---

## 5. What Stays Unchanged

- `SkillPanel` component (right-side detail panel)
- `GrowthPlanPanel`, `LearningPathPanel`, `AutoMergeModal`, `MergeModal`
- All sidebar controls (sync buttons, filters, search, domain list)
- `drawSkillNode` function (node rendering with glow, level ring, seed/target markers)
- `drawGapNode` function
- `drawHoverLabel` tooltip
- `drawBackground` (starfield + nebula)
- Wheel zoom, drag-to-pan handlers
- All Tauri invoke calls and state management
- `SkillPlacement` shape (`outAngle` field repurposed to store depth int, already done)

---

## 6. Files Modified

- `src/pages/SkillsPage.tsx` — only file changed:
  - Replace `layoutSkillGraph` body (lane allocation, repulsion pass, lane headers)
  - Add `drawLaneSeparators(ctx, lanes)` helper
  - Add `drawDepthGuides(ctx, maxDepth, bandHeight, padBottom, w)` helper
  - Update `drawDepEdges` to handle all relationship types with correct styles
  - Update canvas render loop: new draw order, zoom-adaptive label logic, dimming at 0.25
  - Add double-click zoom-to-neighborhood handler + animation refs
  - Add `drawLegend(ctx, w, h)` helper (screen-space, called outside `ctx.save/restore`)
  - Add Fit-all button DOM element in canvas container div
