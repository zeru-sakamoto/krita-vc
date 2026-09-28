# Visual diff viewer

For an art VCS, a code-style text patch is the wrong mental model. Artists need to see the actual
layer imagery and a visual comparison of what changed, so art (`.kra`) files render as a layer stack
next to a before and after canvas.

Palettes have their own `kind: "palette"` and always render as color-swatch grids
(`PaletteDiffView`), whether or not Artist Mode is on. Today they come from the palettes embedded in
a `.kra`, which appear inside that artwork's layer navigator; standalone palette files are no longer
tracked. The swatch diff (parse each format into named sRGB swatches, match them by name, and
classify each as added, removed, modified or unchanged) is computed in the backend (`palette.rs` and
`commands::palette_dto_from`, see [version-control.md](version-control.md#palette-diffs)), and the
frontend just renders the `swatches[]` it receives; a palette that won't parse on either side is left
out. The only text entries left are a deleted `.kra` and one that couldn't be rasterized, and they
render as a one-line summary (`FriendlyFileDiff`) in both modes. See
[frontend-architecture.md](frontend-architecture.md#diff-viewer).

All imagery is composited in the webview from inline SVG markup strings. Real `.kra` layer rasters
come from the backend as SVG `<image>` elements. In the desktop shell their `href` is a `kvcimg://`
URL served straight from the store's raster cache; outside the shell, or if a cache write fails, it's
a base64 data URL. Either way the viewer needs no raster pipeline of its own.

## Data model (`src/types.ts`)

Abridged; see `src/types.ts` for every field.

```ts
type DiffEntry = ArtDiff | TextDiff | PaletteDiff;  // told apart by `kind`

interface ArtDiff {
  kind: "art";
  path: string;               // "hero.kra"
  status: FileStatus;         // M | A | D | U | R | C
  width: number;              // artwork pixels, used as the SVG viewBox
  height: number;
  dpi?: number;               // plus colorModel and colorProfile from maindoc.xml
  layers: ArtLayer[];         // ordered bottom to top
  regions: ChangeRegion[];    // changed-region boxes, normalized 0..1, for the box overlay
  beforeImage?: string | null;  // the composite at each state (mergedimage.png)
  afterImage?: string | null;
  diffImage?: string | null;    // the changed-pixel mask for the composite
  diffOutline?: string | null;  // the outline of the changed pixels, normalized 0..1
}

interface ArtLayer {
  id: string;
  topLevelId?: string;        // the top-level layer this one sits under (for the Changes panel)
  name: string;
  opacity: number;            // 0..100
  blendMode: BlendMode;       // normal | multiply | screen | overlay | add
  change: LayerChange;        // added | removed | modified | unchanged
  visible?: boolean;
  layerType?: string;         // Krita node type, e.g. "paintlayer", "grouplayer"
  bounds?: { x: number; y: number; w: number; h: number };  // painted area, tile-granular
  before: string | null;      // inner SVG markup; null when the layer didn't exist (added)
  after: string | null;       // null when the layer was removed
  beforeThumb?: string | null; // the same markup pointing at a 128 px thumbnail, for the list
  afterThumb?: string | null;
  diffImage?: string | null;  // this layer's own highlight, only for modified layers
  diffOutline?: string | null;
  regions?: ChangeRegion[];
}
```

A layer's pixels at each state are SVG markup strings. `before` and `after` differ for a modified
layer, and one side is `null` for an added or removed layer.

## SVG compositing helpers (`src/lib/svgArt.ts`)

| Helper | What it does |
|--------|---------|
| `layersBody(layers, state)` | The inner markup for the layers at one state. Each layer is wrapped in a `<g>` with `opacity` and `mix-blend-mode` (`blendCss` maps `BlendMode` to CSS), and `null` markup is skipped. |
| `wrapSvg(body, w, h)` | Wraps markup in a scalable, self-contained `<svg>` (`viewBox`, `preserveAspectRatio`). |
| `compositeSvg(layers, state, w, h)` | `wrapSvg(layersBody(...))`, used for thumbnails and the slider. |

## Components

### ArtDiffView

The orchestrator ([`src/components/vcs/ArtDiffView.tsx`](../src/components/vcs/ArtDiffView.tsx)).
It owns the per-file UI state and lays out the layer panel, the toolbar and the canvas.

| State | Default | Control |
|-------|---------|---------|
| `selectedId` | `"composite"` | Clicking a row in the layer panel. |
| `viewMode` | `"split"` | Toolbar: side by side or swipe slider. |
| `highlightOn` | `true` | Toolbar: the eye toggle. |
| `highlightMode` | `"pixels"` | Toolbar: changed pixels (Sparkle) or region boxes (BoundingBox). |

`selectedId` decides which layers render: the whole stack (`composite`) or a single focused layer.

Zoom and pan are shared across both view modes through `useZoomPan`
([`src/lib/useZoomPan.ts`](../src/lib/useZoomPan.ts)), called once in `ArtDiffView`. It owns
`{scale, tx, ty}` and returns a CSS `transform` that is passed to every `ArtCanvas` (both split
panes and the slider's two stacked layers), so before and after, and the slider's divider, stay
registered pixel for pixel under zoom and pan. The wheel zooms toward the cursor, and a plain left
drag or a middle drag pans; the slider's divider stops the event so dragging it doesn't pan. The
transform sits on the `<div>` around the SVG and is never serialized into the SVG string, so
interaction stays on the compositor and the memoized SVG DOM isn't touched. A "Reset zoom" button
and a live percentage in the toolbar show the state, and switching the view mode calls `reset()`,
because the panes and the slider frame have different widths.

### LayerStackPanel

A Krita-style layer list ([`src/components/vcs/LayerStackPanel.tsx`](../src/components/vcs/LayerStackPanel.tsx)),
shown top first (layers are stored bottom to top). Each row has a small SVG thumbnail
(`compositeSvg` of that one layer, pointed at `beforeThumb`/`afterThumb`, the backend's 128 px
thumbnail, and at the full raster only where there's none: a 36 × 28 px row used to make the webview
decode up to 2048 × 2048 per layer), the name, `opacity% · blendMode`, and a change marker that reuses
`FileStatusChip` (added is A, removed is D, modified is M, unchanged shows nothing). A Composite row
at the top selects the full stack, and a "Color Palette" section below the layers shows an embedded
palette that changed. It shows one palette only: `DiffView` passes the first matching `<kra>::`
entry (`entries.find`), so when several embedded palettes changed in one version, the rest are
computed by the backend but not shown. The selected row uses the accent left border and tint.

### ArtCanvas

Renders one state's composited SVG over a checkerboard, so layer transparency reads correctly
([`src/components/vcs/ArtCanvas.tsx`](../src/components/vcs/ArtCanvas.tsx)). The SVG is built inline
(`dangerouslySetInnerHTML`) rather than through `<img>`, so blend modes and filters composite
correctly. `ArtCanvas` is wrapped in `React.memo`, so dragging the slider divider or zooming and
panning its parent doesn't re-render it when its props haven't changed. When `overlay` is set, it
appends a change-highlight overlay in the same viewBox, so the highlight lines up with the art.

The overlay data (`diffImage`, `diffOutline`, `regions`) arrives as explicit props, not read from
`diff`. `ArtDiffView` picks the source from the current selection: the whole-file highlight for the
Composite view, or the selected layer's own highlight when a single layer is focused (see
[Per-layer highlights](#per-layer-highlights)). A layer without a highlight of its own (unchanged,
added, removed, or still streaming) gets empty props, and the overlay simply doesn't draw. Only
`diff.width` and `diff.height` (the viewBox) still come from `diff`.

- **Pixels mode** (the default) uses the changed-pixel mask (`diffImage`, an `<image>` sized to the
  viewBox), which is transparent except where before and after differ. The backend bakes a
  placeholder color into that raster, but the frontend (`pixelOverlay`) only ever reads its alpha
  channel (`mask-type: alpha`) and repaints with `var(--color-accent)`, so the highlight always
  matches the active theme and switching themes never needs the cached raster regenerated. It
  renders three ways so it stays readable on busy artwork: a flat accent tint over the changed
  pixels, a diagonal hatch pattern masked to the same pixels (the stripes give contrast a flat tint
  can't, against any underlying color), and a dashed outline that follows the changed pixels' shape,
  also stroked with `var(--color-accent)`. The outline is a vector path (`diffOutline`, normalized
  0..1) traced in Rust (`raster::outline_from_grid`, which follows the boundary between changed and
  unchanged cells of a downsampled grid into closed loops), not a bounding box. The frontend scales it
  to the viewBox and strokes it dashed with `non-scaling-stroke`, so the dashes stay the same size on
  screen at any zoom. All of it is plain fills, patterns, masks and paths, composited on the GPU with
  no filters, and rebuilt only when the memoized SVG changes (never on zoom or pan). The outline and
  the normalized bounding box ride in the cached mask PNG's own `tEXt` chunks (`kvc-outline`,
  `kvc-bbox`, written ahead of the pixels), so a cache hit reads both from its header
  (`raster::mask_meta`) without decoding the mask, and there's no separate cache file for them. A
  mask cached before the chunks existed is decoded and traced again (`raster::outline_from_mask_png`).
- **Box mode** draws a faint filled rectangle with bold corner brackets for each `regions` entry
  (plus optional labels), a coarse bounding-box fallback. Region coordinates are normalized 0..1 of
  the viewBox (for both the composite's tile bounding box and a layer's own changed-pixel bounding
  box), and `boxOverlay` scales them by width and height, so a region must never be pre-scaled to
  pixels, or it overflows past the bottom right of the canvas. Strokes use
  `vector-effect="non-scaling-stroke"` so they stay readable in screen pixels even when a large
  canvas is shown fit to the pane; plain document-space dashes would shrink below a pixel and vanish.

### Per-layer highlights

The composite's highlight (`ArtDiff.diffImage`, `diffOutline`, `regions`) ships with the first
`commit_diff` and drives the Composite view. Each modified layer also carries its own `diffImage`,
`diffOutline` and `regions` on `ArtLayer`, diffed in Rust from that layer's before and after rasters
(`commands::layer_diff_overlay` → `raster::diff_overlay_full`: one changed-pixel grid gives the mask,
the outline and the normalized bounding box). So selecting a layer shows only its changed pixels,
not the whole file's silhouette painted on every layer. These are computed during the per-layer
stream from the capped PNGs the raster path just encoded (read back from the raster cache when the
rasters were cache hits), and the mask PNG is cached content-addressed by both layer raster keys,
so a repeat view reads neither raster. Added, removed and unchanged layers carry none.

### CompareSlider

The swipe comparison ([`src/components/vcs/CompareSlider.tsx`](../src/components/vcs/CompareSlider.tsx)).
The after state fills the frame, and the before state is clipped to the left of a draggable divider
(`clip-path: inset(...)`). The divider uses the same pointer-capture drag pattern as the Sidebar's
resize handle, plus nudging with the arrow keys. Its `setPos` is throttled to one update per
animation frame (pointer moves fire more than 100 times a second), and the component is wrapped in
`React.memo`, so a drag frame doesn't re-render both stacked canvases. The shared zoom and pan
`transform` applies to both canvases the same way, while the `clip-path` stays on the untransformed
wrapper around the before canvas (in the frame's screen space), so the reveal line tracks the image
under any zoom and pan. When the highlight is on, it's drawn on the after side.

## How the modes combine

```
ArtDiffView (owns the shared useZoomPan → transform)
├─ viewMode "split"  → ArtCanvas(before, transform) | ArtCanvas(after, transform, overlay=highlightOn)
└─ viewMode "slider" → CompareSlider(transform, overlay=highlightOn)  // before clipped over after
                        highlightMode (pixels or box) picks the overlay style
```

## Where the data comes from

For `.kra` files the backend fills in `ArtDiff` and `ArtLayer` in two stages, so the panel appears
immediately instead of waiting for every layer's raster. A version is always compared with its first
parent; the working tree is compared with its last commit.

1. **`commit_diff`** (or `working_diff`, see [version-control.md](version-control.md)) returns the
   cheap parts first: the layer metadata (`ArtLayer` with `before` and `after` set to `null`), the
   composite, and the change regions.
   - **The composite.** `mergedimage.png` at each state goes in `ArtDiff.beforeImage` and
     `afterImage`, re-encoded down to at most `MAX_RASTER_DIM` (`raster::cap_png`), because
     full-resolution composites of large canvases dominated the IPC payload. The navigator's
     Composite row prefers this over stacking layers: `ArtDiffView` swaps in a single composite
     "layer" when these are present, so the default view is right the moment the diff loads.

     A version saved from a layer subset has no `mergedimage.png`. Krita rendered the whole stack and
     the engine can't redo that render, so `stage::stage_kra` drops it rather than ship a preview
     showing layers the version doesn't contain (see [layer-staging.md](layer-staging.md)). For those
     versions, `commands::stacked_composite_url` composites the stack itself
     (`raster::composite_stack`) and caches the result under `kra::stack_cache_key` like any other
     raster, so the Composite pane, and the Version Map node, which draws `afterImage` and fetches no
     per-layer rasters, isn't blank. Both the before and after sides fall back this way. It models
     source-over blending, per-layer opacity, `visible`, one level of group opacity and the five
     blend modes `svgArt.ts::blendCss` maps; masks and filter, clone and vector layers render as plain
     paint. That's the same ceiling the frontend's SVG stacker has always had, and it's acceptable
     because this is a cached preview, never anything written into the artwork. Its inputs are the
     ordinary per-layer rasters, so the diff viewer and this share cache entries both ways. Krita
     rewrites the real composite on the next save.
   - **The changed-pixel mask and outline.** `ArtDiff.diffImage` and `ArtDiff.diffOutline` come from
     comparing the before and after composites pixel by pixel in Rust (`raster::diff_overlay_full`, with a
     threshold of about 16 per channel). Each side is capped to `MAX_RASTER_DIM` right after decoding,
     so the comparison never holds two full-resolution composites at once. The mask is a PNG that's
     transparent except where pixels changed; its RGB is a fixed placeholder, since only the alpha
     channel matters and the frontend repaints it with the active theme's `--color-accent`. It's
     capped, cached (`kra::diff_cache_key`) and served over `kvcimg://`. The outline is a vector path
     tracing the changed pixels' shape. Together they drive the default "pixels" highlight, and since
     they're computed from the composite, they don't need the layer stream.
   - **Change regions.** One normalized bounding box over the tiles that differ between the two
     commits (no pixel decoding, just comparing tile hashes), for the coarse box highlight.
   - **Each layer's `change`** (`added`, `removed`, `modified` or `unchanged`). A layer is
     `modified` when its curated metadata moved (`opacity`, `compositeop`, `name`) or its tiles
     differ. Both are decided per matched layer, never per archive path. Layers are paired by
     `commands::layer_id` (the uuid, or the name when there is none), and the tile comparison looks
     up the old side under that matched layer's own `<image name>/layers/<filename>`. That's why
     `kra::diff_tile_indexes` takes a `pair` map from each new-side entry path to its old-side entry
     path (`""` for a layer with no counterpart, since no entry path is empty, so an added layer's
     tiles all count as new and it still contributes to the change region).

     This pairing matters. Krita renumbers its `layerN` data files whenever the stack changes, so
     inserting one layer shifts every layer above it. Pairing by path made an untouched layer compare
     against the wrong file and report `modified`: add a layer, and the whole stack above it lit up
     as changed, in the diff navigator and in the Changes panel, whose rows roll up this same
     `change`. `merge.rs` guards against the same class of bug by collecting data files independently
     of their names, and the raster path in the same loop already got it right (`before_r` reads the
     matched old layer's `filename` and its own side's image name). Any replacement must keep identity
     and content pairing on the same key.
2. **`commit_layers`** (or `working_layers`) is then fetched lazily by
   [`useArtLayers`](../src/lib/repoData.ts) and streamed. The command takes a Tauri
   `Channel<LayerDto>` and sends each layer as soon as its rasters are ready (in parallel with rayon,
   so out of order; the frontend merges each one by layer id over the metadata from stage 1). Each
   layer's pixels are rebuilt from the stored tiles (LZF-decoded, planar BGRA to RGBA) over that
   entry's `.defaultpixel` sibling. Krita only stores tiles for the painted parts of a layer, so a
   uniformly filled layer (a solid "Background", for example) is mostly or entirely untiled, and
   without the fill those areas would decode as transparent instead of their real color. The result
   is at most `MAX_RASTER_DIM` on its longest side, area-averaged with a box filter in premultiplied
   alpha so transparent edges don't darken (sharper than the old nearest-neighbor when zoomed).
   `raster::rasterize_tiles` accumulates each decoded tile straight into that capped size, with the
   exact integer arithmetic of `box_downscale` over a full canvas, so no full-resolution canvas is
   ever allocated (it was 278 MB per layer at 600 dpi A3); see
   [performance.md](performance.md#parallelism-rayon). The raster is encoded as PNG, with a 128 px
   thumbnail beside it for the layer list, and delivered as SVG `<image>` markup in
   `ArtLayer.before` and `after` (and `beforeThumb`/`afterThumb`). A modified layer also
   carries its own `diffImage`, `diffOutline` and `regions` in the same `LayerDto` (see
   [Per-layer highlights](#per-layer-highlights)). `layersBody`, `wrapSvg`, `ArtCanvas` and
   `CompareSlider` composite all of it with no rendering changes (blend modes, the checkerboard and
   the overlays still apply). Layers appear one by one; layers that haven't arrived show a spinner
   thumbnail in the navigator (and a canvas spinner if selected), with a "Loading layers…" indicator
   until the whole set has landed.

Rasters use `preserveAspectRatio="xMidYMid meet"`, never `none`, so a before side from a version with
different canvas dimensions letterboxes instead of stretching.

## Caching

Every capped PNG, composite or per-layer, is written to the store's `cache/` folder, keyed by a hash
of the content that produced it. For a composite that's the entry's content hash. For a layer it's
the tile positions and hashes, the dimensions, the cap and the resolved `.defaultpixel` fill, so a
change to the fill alone, with no tile touched, still invalidates the entry. Because keys come from
content, entries never need invalidating, unchanged layers share one entry across commits and across
the committed and working paths, and a repeat view, even after an app restart, skips rebuilding,
decoding and encoding entirely. The cache has a size budget (`Config.cacheMaxBytes`, 256 MB by
default, "Preview cache size" in Settings → Storage) with oldest-first pruning, and "Clean up storage"
prunes it too. Within a session the frontend also memoizes `commit_diff` results and streamed layer
sets in small LRU maps in `repoData.ts`. See [performance.md](performance.md#caching-across-requests).

## Not built yet

- Color spaces other than 8-bit RGBA: those layers fall back to the composite.
- Labels on the per-layer change regions.
- Comparing two arbitrary versions. Every diff compares a version with its first parent (or the
  working tree with its last commit). `layer_diff` can report metadata changes between any two
  commits, but nothing in the UI calls it.
