# Visual Diff Viewer

For an art VCS, a code-style text patch is the wrong mental model — artists need to see the **actual
layer imagery** and a **visual comparison** of what changed. Art (`.kra`) files render as a **layer
stack + before/after canvas**. Color palettes (`.gpl`) have their own `kind: "palette"` type and
render as **color-swatch grids** (`PaletteDiffView`) regardless of Artist Mode — the first palette
is embedded in the art diff's layer navigator; standalone palettes get their own panel. Generic
text files (config, settings) render as a friendly one-line summary (`FriendlyFileDiff`) in Artist
Mode, or a code-style line diff (`DiffFileBlock`) with it off. See
[frontend-architecture.md → Diff viewer](frontend-architecture.md#diff-viewer).

> All imagery arrives as **inline SVG markup strings** and is composited in the webview — real
> `.kra` layer rasters come from the backend as SVG `<image>` elements wrapping base64 PNGs, so
> the viewer needs no raster pipeline of its own.

## Data model (`src/types.ts`)

```ts
type DiffEntry = ArtDiff | TextDiff | PaletteDiff;  // discriminated by `kind`

interface ArtDiff {
  kind: "art";
  path: string;            // "characters/hero.kra"
  status: FileStatus;      // M | A | D | U | R | C
  width: number;           // artwork px → SVG viewBox
  height: number;
  layers: ArtLayer[];      // ordered bottom→top
  regions: ChangeRegion[]; // changed-region rects (normalized 0..1) for the box overlay
}

interface ArtLayer {
  id: string;
  name: string;
  opacity: number;         // 0..100
  blendMode: BlendMode;    // normal | multiply | screen | overlay | add
  change: LayerChange;     // added | removed | modified | unchanged
  before: string | null;   // inner SVG markup; null when the layer didn't exist (added)
  after: string | null;    // null when the layer was removed
}
```

A layer's pixels at each state are just **SVG markup strings**. `before`/`after` differ for a
modified layer; one side is `null` for added/removed layers.

## SVG compositing helpers (`src/lib/svgArt.ts`)

| Helper | Purpose |
|--------|---------|
| `layersBody(layers, state)` | Inner markup for the layers at one state; each layer wrapped in a `<g>` with `opacity` + `mix-blend-mode` (`blendCss` maps `BlendMode` → CSS). Skips `null` markup. |
| `wrapSvg(body, w, h)` | Wraps markup in a scalable, self-contained `<svg>` (`viewBox`, `preserveAspectRatio`). |
| `compositeSvg(layers, state, w, h)` | `wrapSvg(layersBody(...))` — used for thumbnails and the slider. |

## Components

### `ArtDiffView` — orchestrator (`src/components/vcs/ArtDiffView.tsx`)

Owns the per-file UI state and lays out the **layer panel + toolbar + canvas**:

| State | Default | Control |
|-------|---------|---------|
| `selectedId` | `"composite"` | Layer panel row click. |
| `viewMode` | `"split"` | Toolbar: Side-by-side ↔ Swipe slider. |
| `highlightOn` | `true` | Toolbar: eye toggle. |
| `highlightMode` | `"pixels"` | Toolbar: Sparkle (changed pixels) ↔ BoundingBox (region boxes). |

`selectedId` selects the layers to render: the whole stack (`composite`) or a single focused layer.

**Zoom/pan** is shared across both view modes via `useZoomPan` (`src/lib/useZoomPan.ts`), called
once in `ArtDiffView`. It owns `{scale, tx, ty}` and returns a CSS `transform` passed to **every**
`ArtCanvas` (both split panes and the slider's two stacked layers), so before/after and the slider
divider stay pixel-registered under zoom+pan. Wheel zooms toward the cursor; middle-mouse or
space-held left-drag pans (plain left-drag stays reserved for the slider divider). The transform
rides on the SVG-wrapping `<div>`, never re-serialized into the SVG string, so interaction stays on
the compositor and the memoized SVG DOM is untouched. A toolbar "Reset zoom" button + a live % read
out expose the state; switching view mode calls `reset()` (the panes and slider frame differ in width).

### `LayerStackPanel` (`src/components/vcs/LayerStackPanel.tsx`)

Krita-style layer list, shown **top-first** (layers are stored bottom→top). Each row: a small SVG
**thumbnail** (`compositeSvg` of that one layer), name, `opacity% · blendMode`, and a change marker
reusing `FileStatusChip` (added→A, removed→D, modified→M, unchanged→none). A **Composite** row at the
top selects the full stack. Selected row uses the accent left-border + tint.

### `ArtCanvas` (`src/components/vcs/ArtCanvas.tsx`)

Renders one state's composited SVG over a **checkerboard matte** (so layer transparency reads true).
The SVG is built inline (`dangerouslySetInnerHTML`) rather than via `<img>` so blend modes and
filters composite correctly. `ArtCanvas` is `React.memo`'d so the slider divider drag / zoom-pan
re-renders of its parent don't re-enter it when props are unchanged. When `overlay` is set, it
appends a change-highlight overlay **in the same viewBox** (so it aligns with the art):

- **pixels mode** (default) — the backend's changed-pixel mask (`ArtDiff.diffImage`, an `<image>`
  sized to the viewBox): transparent except where the before/after composites differ. Rendered three
  ways for legibility on busy artwork (`pixelOverlay`): a flat accent tint of the changed pixels, a
  diagonal **hatch pattern** masked to those same pixels (the alternating stripes give contrast a
  flat tint can't against arbitrary underlying color), and a **dashed outline** that hugs the changed
  pixels' silhouette. The outline is a vector path (`ArtDiff.diffOutline`, normalized 0..1) traced in
  Rust (`raster::outline_from_grid`, marching the changed/unchanged cell boundary of a downsampled
  grid into closed loops) — *not* a bounding box; the frontend scales it to the viewBox and strokes
  it dashed with `non-scaling-stroke` (constant on-screen dash size at any zoom). All plain
  fills/patterns/masks/paths — GPU-composited, no filters, rebuilt only when the memoized SVG changes
  (never on zoom/pan). Because both mask and outline ride on the first `commit_diff` (computed off the
  composite), they appear immediately — no wait for the per-layer stream. On a cache hit the outline
  is re-traced from the cached mask PNG (`raster::outline_from_mask_png`), so no sibling cache file.
- **box mode** — a subtle filled rect + **bold corner brackets** per `ArtDiff.regions` entry (+
  optional labels), a coarse tile-bbox fallback. Strokes use `vector-effect="non-scaling-stroke"`
  so they stay legible in screen pixels even when a large canvas is shown fit-to-pane (plain
  document-space dashes would otherwise go sub-pixel and disappear).

### `CompareSlider` (`src/components/vcs/CompareSlider.tsx`)

Swipe comparison: **after** fills the frame; **before** is clipped to the left of a draggable divider
(`clip-path: inset(...)`). The divider uses the same pointer-capture drag pattern as the Sidebar
resize handle, plus arrow-key nudging, and its `setPos` is **rAF-throttled** (pointermove fires
>100×/s) with the component `React.memo`'d — together these stop each drag frame from re-rendering
both stacked canvases. The shared zoom/pan `transform` is applied identically to both canvases while
the `clip-path` stays on the untransformed before-wrapper (frame screen space), so the reveal line
tracks the image under any zoom+pan. The highlight overlay (when on) is drawn on the after side.

## How the modes combine

```
ArtDiffView (owns shared useZoomPan → transform)
├─ viewMode "split"  → ArtCanvas(before, transform) | ArtCanvas(after, transform, overlay=highlightOn)
└─ viewMode "slider" → CompareSlider(transform, overlay=highlightOn)  // before clipped over after
                        highlightMode ∈ { pixels, box } controls the overlay style
```

## Real backend data

`ArtDiff`/`ArtLayer` are the swap point, and for `.kra` files the backend fills it in **two
stages** so the panel appears immediately instead of blocking on every layer's raster:

1. **`commit_diff`** (see [version-control.md](version-control.md)) returns the cheap parts up
   front — layer *metadata* (`ArtLayer` with `before`/`after` = `null`), the composite, and change
   regions:
   - **Composite** — `mergedimage.png` at each state in `ArtDiff.beforeImage`/`afterImage`,
     re-encoded down to ≤`MAX_RASTER_DIM` (`raster::cap_png`; full-resolution composites of big
     canvases dominated the IPC payload). The "Composite" navigator row prefers this over
     stacking layers; `ArtDiffView` swaps in a single composite "layer" when these are present, so
     the default view is correct the instant the diff loads.
   - **Changed-pixel mask + outline** — `ArtDiff.diffImage` and `ArtDiff.diffOutline`: the
     before/after composites diffed pixel-for-pixel in Rust (`raster::diff_overlay`, threshold
     ~16/channel). The mask is a transparent-except-changed accent PNG, capped + cached
     (`kra::diff_cache_key`) + served over `kvcimg://`; the outline is a vector path tracing the
     changed pixels' silhouette. Together they drive the default "pixels" highlight; keyed off the
     composite so they need no layer stream.
   - **Change regions** — one normalized bounding box over the tiles that differ between the two
     commits (no pixel decode; just tile-hash comparison), feeding the coarse box-highlight overlay.
2. **`commit_layers`** (or `working_layers`) is then fetched lazily by
   [`useArtLayers`](../src/lib/repoData.ts) and **streamed**: the command takes a Tauri
   `Channel<LayerDto>` and sends each layer the moment its rasters finish (rayon-parallel, so
   out of order; the frontend merges by layer id over the metadata from stage 1). Each layer's
   pixels are reconstructed from stored tiles (LZF-decoded, planar BGRA → RGBA, downscaled to
   ≤`MAX_RASTER_DIM` via an **area-average box filter** — `raster::cap_rgba`/`box_downscale`, in
   premultiplied-alpha space so transparent edges don't bleed dark; sharper than the old
   nearest-neighbour under zoom), PNG-encoded) and arrive as **SVG `<image href="data:image/png;base64,…">`
   markup** in `ArtLayer.before`/`after`, so `layersBody`/`wrapSvg`/`ArtCanvas`/`CompareSlider`
   composite them with **zero rendering changes** (blend modes, checkerboard, overlays all still
   apply). Layers pop in one by one; not-yet-arrived layers show a spinner thumb in the navigator
   (and a canvas spinner if selected), with the "Loading layers…" header indicator until the
   whole set lands.

Rasters use `preserveAspectRatio="xMidYMid meet"` (never `none`), so a before-side from a version
with different canvas dimensions letterboxes instead of stretching.

**Caching:** every capped PNG (composite and per-layer) is written to **`.kvc/cache/`**, keyed by
a hash of the content that produced it (composite: the entry's content hash; layer: tile
positions + hashes + dims + cap). Keys are content-derived, so entries never invalidate, unchanged
layers share one entry across commits and across the committed/working paths, and a repeat view —
including after an app restart — skips reconstruct/decode/encode entirely. In-session, the
frontend also memoizes `commit_diff` results and streamed layer sets (small LRU maps in
`repoData.ts`).

Deferred (ponytail): non-RGBA-8 colorspaces (those layers fall back to the composite), per-layer
change regions with labels, and `.kvc/cache` eviction (capped PNGs are small).
