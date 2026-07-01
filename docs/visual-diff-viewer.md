# Visual Diff Viewer

For an art VCS, a code-style text patch is the wrong mental model — artists need to see the **actual
layer imagery** and a **visual comparison** of what changed. Art (`.kra`) files render as a **layer
stack + before/after canvas**. Color palettes (`.gpl`) have their own `kind: "palette"` type and
render as **color-swatch grids** (`PaletteDiffView`) regardless of Artist Mode — the first palette
is embedded in the art diff's layer navigator; standalone palettes get their own panel. Generic
text files (config, settings) render as a friendly one-line summary (`FriendlyFileDiff`) in Artist
Mode, or a code-style line diff (`DiffFileBlock`) with it off. See
[frontend-architecture.md → Diff viewer](frontend-architecture.md#diff-viewer).

> All imagery is **generated inline SVG** — no external assets, no dependencies, composited offline
> in the webview. This is mock data; the real backend will supply actual `.kra` layer rasters.

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

## Generated mock art (`src/data/mockArt.ts`)

Scenes (hero character, forest background, sword prop, villain) are assembled from `ArtLayer`s built
with the terse `layer(id, name, before, after?, opts)` helper. Compositing helpers:

| Helper | Purpose |
|--------|---------|
| `layersBody(layers, state)` | Inner markup for the layers at one state; each layer wrapped in a `<g>` with `opacity` + `mix-blend-mode` (`blendCss` maps `BlendMode` → CSS). Skips `null` markup. |
| `wrapSvg(body, w, h)` | Wraps markup in a scalable, self-contained `<svg>` (`viewBox`, `preserveAspectRatio`). |
| `compositeSvg(layers, state, w, h)` | `wrapSvg(layersBody(...))` — used for thumbnails and the slider. |

Pre-built `ArtDiff`s are exported as `ART_DIFFS` and referenced from `MOCK_DIFF_BY_COMMIT` in
`src/data/mockData.ts`.

## Components

### `ArtDiffView` — orchestrator (`src/components/vcs/ArtDiffView.tsx`)

Owns the per-file UI state and lays out the **layer panel + toolbar + canvas**:

| State | Default | Control |
|-------|---------|---------|
| `selectedId` | `"composite"` | Layer panel row click. |
| `viewMode` | `"split"` | Toolbar: Side-by-side ↔ Swipe slider. |
| `highlightOn` | `true` | Toolbar: eye toggle. |
| `highlightMode` | `"box"` | Toolbar: BoundingBox (box) ↔ Sparkle (mask). |

`selectedId` selects the layers to render: the whole stack (`composite`) or a single focused layer.

### `LayerStackPanel` (`src/components/vcs/LayerStackPanel.tsx`)

Krita-style layer list, shown **top-first** (layers are stored bottom→top). Each row: a small SVG
**thumbnail** (`compositeSvg` of that one layer), name, `opacity% · blendMode`, and a change marker
reusing `FileStatusChip` (added→A, removed→D, modified→M, unchanged→none). A **Composite** row at the
top selects the full stack. Selected row uses the accent left-border + tint.

### `ArtCanvas` (`src/components/vcs/ArtCanvas.tsx`)

Renders one state's composited SVG over a **checkerboard matte** (so layer transparency reads true).
The SVG is built inline (`dangerouslySetInnerHTML`) rather than via `<img>` so blend modes and
filters composite correctly. When `overlay` is set, it appends a change-highlight overlay **in the
same viewBox** (so it aligns with the art):

- **box mode** — translucent dashed accent rectangles from `ArtDiff.regions` (+ optional labels).
- **mask mode** — the changed layers' silhouettes recolored to accent with a glow, via an SVG filter
  (`feFlood` + `feComposite operator="in"` against `SourceAlpha` + `feGaussianBlur`/`feMerge`).

### `CompareSlider` (`src/components/vcs/CompareSlider.tsx`)

Swipe comparison: **after** fills the frame; **before** is clipped to the left of a draggable divider
(`clip-path: inset(...)`). The divider uses the same pointer-capture drag pattern as the Sidebar
resize handle, plus arrow-key nudging. The highlight overlay (when on) is drawn on the after side.

## How the modes combine

```
ArtDiffView
├─ viewMode "split"  → ArtCanvas(before)  | ArtCanvas(after, overlay=highlightOn)
└─ viewMode "slider" → CompareSlider(overlay=highlightOn)   // before clipped over after
                        highlightMode ∈ { box, mask } controls the overlay style
```

## Real backend data

`ArtDiff`/`ArtLayer` are the swap point, and for `.kra` files the backend now fills it. The
`commit_diff` command (see [version-control.md](version-control.md)) reconstructs each paint
layer's pixels from its stored tiles (LZF-decoded, planar BGRA → RGBA, PNG-encoded) and returns
them as **SVG `<image href="data:image/png;base64,…">` markup** in `ArtLayer.before`/`after` — so
`layersBody`/`wrapSvg`/`ArtCanvas`/`CompareSlider` composite them with **zero rendering changes**
(blend modes, checkerboard, overlays all still apply). It also supplies:

- **Composite** — `mergedimage.png` at each state in `ArtDiff.beforeImage`/`afterImage`. The
  "Composite" navigator row prefers this (a reliable whole-image render) over stacking layers;
  `ArtDiffView` swaps in a single composite "layer" when these are present.
- **Change regions** — one normalized bounding box over the tiles that differ between the two
  commits (no pixel decode; just tile-hash comparison), feeding the box-highlight overlay.

`mockArt.ts` stays for the browser fallback (`useCommitDiff` uses it when not in the Tauri shell).
Deferred (ponytail): non-RGBA-8 colorspaces (those layers fall back to the composite), and
per-layer change regions with labels.
