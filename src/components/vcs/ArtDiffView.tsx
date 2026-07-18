import { memo, useEffect, useMemo, useState } from "react";
import {
  ArrowsIn,
  ArrowsLeftRight,
  BoundingBox,
  CircleNotchIcon,
  Columns,
  Eye,
  EyeSlash,
  Sparkle,
} from "@phosphor-icons/react";
import type { ArtDiff, ChangeRegion, DiffState, PaletteDiff } from "../../types";
import { IconButton } from "../ui/IconButton";
import { FileStatusChip } from "./FileStatusChip";
import { useArtistMode } from "../../lib/artistMode";
import { useArtLayers } from "../../lib/repoData";
import { useZoomPan } from "../../lib/useZoomPan";
import { assetName } from "../../lib/friendly";
import { ArtCanvas, type HighlightMode } from "./ArtCanvas";
import { CompareSlider } from "./CompareSlider";
import { LayerStackPanel, COMPOSITE_ID, PALETTE_ID } from "./LayerStackPanel";
import { PaletteDiffView } from "./PaletteDiffView";

type ViewMode = "split" | "slider";

const Pane = memo(function Pane({
  label,
  diff,
  layers,
  state,
  overlay,
  highlightMode,
  diffImage,
  diffOutline,
  regions,
  transform,
  onWheel,
}: {
  label: string;
  diff: ArtDiff;
  layers: ArtDiff["layers"];
  state: DiffState;
  overlay?: boolean;
  highlightMode?: HighlightMode;
  diffImage?: string | null;
  diffOutline?: string | null;
  regions?: ChangeRegion[];
  transform?: string;
  onWheel?: (e: React.WheelEvent) => void;
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex h-6 shrink-0 items-center border-b border-border bg-surface px-2 text-[11px] font-medium uppercase text-text-muted">
        {label}
      </div>
      {/* Wheel handler lives on this pane's own element (not the shared split-view container)
          so the zoom pivot's cursor offset is measured against this pane's own rect — the
          container spans both panes, which would double the effective width and bias the
          pivot toward whichever pane is on the right. */}
      <div className="min-h-0 flex-1" onWheel={onWheel}>
        <ArtCanvas
          diff={diff}
          layers={layers}
          state={state}
          overlay={overlay}
          highlightMode={highlightMode}
          diffImage={diffImage}
          diffOutline={diffOutline}
          regions={regions}
          transform={transform}
        />
      </div>
    </div>
  );
});

/**
 * Visual diff for one art (.kra) file: a layer stack panel beside a before/after
 * canvas. Toolbar toggles side-by-side vs swipe slider, and the change-highlight
 * overlay (on/off, translucent box vs precise mask).
 *
 * When a `palette` prop is supplied, a "Color Palette" section appears below the
 * layers in the navigator. Selecting it swaps the right pane to the palette grid.
 */
interface ArtDiffViewProps {
  diff: ArtDiff;
  palette?: PaletteDiff;
  /** Navigator id to seed the initial selection with, e.g. jump straight to the palette pane. */
  initialFocusId?: string;
  /** Diff source for lazily fetching this file's per-layer rasters. Absent in the browser. */
  repoPath?: string;
  commitId?: string | null;
  working?: boolean;
  nonce?: number;
  /** Reports the navigator selection up to the Inspector (which file + which layer/composite). */
  onFocus?: (f: { path: string; id: string }) => void;
}

export function ArtDiffView({
  diff,
  palette,
  initialFocusId,
  repoPath,
  commitId,
  working,
  nonce,
  onFocus,
}: ArtDiffViewProps) {
  const { artistMode } = useArtistMode();
  const [selectedId, setSelectedId] = useState<string>(initialFocusId ?? COMPOSITE_ID);

  // Mirror the navigator selection into the Inspector. Fires on mount (default composite) and on
  // every selection change; `onFocus` (a stable setState) makes this a no-churn dependency.
  useEffect(() => {
    onFocus?.({ path: diff.path, id: selectedId });
  }, [onFocus, diff.path, selectedId]);
  const [viewMode, setViewMode] = useState<ViewMode>("split");
  const [highlightOn, setHighlightOn] = useState(true);
  const [highlightMode, setHighlightMode] = useState<HighlightMode>("pixels");

  // Shared zoom/pan applied identically to both panes and the slider's stacked canvases.
  const zoom = useZoomPan();
  const switchView = (mode: ViewMode) => {
    setViewMode(mode);
    zoom.reset(); // split panes vs the slider frame differ in width — refit on switch
  };

  // The per-commit diff ships layer *metadata* but not the heavy per-layer rasters; those stream
  // in one at a time (keyed by layer id) and merge over the metadata as they land. Until a
  // layer arrives its metadata row still renders, and the composite (mergedimage.png) is
  // available from the start.
  const { layers: fetchedLayers, loading: layersLoading } = useArtLayers(
    repoPath ?? "",
    diff.path,
    {
      commitId: commitId ?? null,
      working: working ?? false,
      nonce,
    }
  );
  const effectiveDiff = useMemo<ArtDiff>(() => {
    if (!fetchedLayers || fetchedLayers.size === 0) return diff;
    return { ...diff, layers: diff.layers.map((l) => fetchedLayers.get(l.id) ?? l) };
  }, [diff, fetchedLayers]);
  // Layers whose rasters haven't streamed in yet — spinners in the navigator and canvas.
  const pendingIds = useMemo(() => {
    if (!layersLoading) return new Set<string>();
    return new Set(diff.layers.filter((l) => !fetchedLayers?.has(l.id)).map((l) => l.id));
  }, [diff, fetchedLayers, layersLoading]);

  const layers = useMemo(() => {
    if (selectedId === COMPOSITE_ID) {
      // Prefer the backend's real composite (mergedimage.png) over stacking layers, so the
      // composite is correct even if some layers can't be rastered.
      if (effectiveDiff.beforeImage !== undefined || effectiveDiff.afterImage !== undefined) {
        const composite: ArtDiff["layers"][number] = {
          id: COMPOSITE_ID,
          name: "Composite",
          opacity: 100,
          blendMode: "normal",
          change: "modified",
          before: effectiveDiff.beforeImage ?? null,
          after: effectiveDiff.afterImage ?? null,
        };
        return [composite];
      }
      return effectiveDiff.layers;
    }
    const found = effectiveDiff.layers.find((l) => l.id === selectedId);
    return found ? [found] : effectiveDiff.layers;
  }, [effectiveDiff, selectedId]);

  // The change-highlight source follows the selection: the whole-file composite highlight for the
  // Composite view, or the selected layer's *own* highlight (so its outline hugs only that layer's
  // changed pixels, not the composite's). A layer with no highlight (unchanged/added/removed, or
  // still streaming) yields undefined fields → the overlay simply doesn't draw.
  const overlay = useMemo(() => {
    if (selectedId !== COMPOSITE_ID) {
      const found = effectiveDiff.layers.find((l) => l.id === selectedId);
      if (found) {
        return {
          diffImage: found.diffImage,
          diffOutline: found.diffOutline,
          regions: found.regions,
        };
      }
    }
    return {
      diffImage: effectiveDiff.diffImage,
      diffOutline: effectiveDiff.diffOutline,
      regions: effectiveDiff.regions,
    };
  }, [effectiveDiff, selectedId]);

  const showPalette = selectedId === PALETTE_ID && palette != null;

  return (
    <div className="flex flex-col border-b border-border flex-1 min-h-0">
      {/* File header */}
      <div className="sticky top-0 z-(--z-sticky) flex items-center gap-2 border-b border-border bg-surface px-3 py-1.5 h-8">
        <FileStatusChip status={diff.status} />
        <span
          className={[
            "selectable text-[12px] text-text",
            artistMode ? "font-medium" : "font-mono",
          ].join(" ")}
        >
          {artistMode ? assetName(diff.path) : diff.path}
        </span>
        {layersLoading && (
          <span className="ml-auto flex items-center gap-1 text-[11px] text-text-muted">
            <CircleNotchIcon size={12} className="animate-spin" />
            Loading layers…
          </span>
        )}
      </div>

      <div className="flex flex-1 min-h-0">
        <LayerStackPanel
          diff={effectiveDiff}
          palette={palette}
          selectedId={selectedId}
          onSelect={setSelectedId}
          pendingIds={pendingIds}
        />

        <div className="flex min-w-0 flex-1 flex-col bg-bg">
          {showPalette ? (
            /* ── Palette grid pane ── */
            <PaletteDiffView diff={palette} />
          ) : (
            /* ── Art canvas pane ── */
            <>
              {/* Diff toolbar */}
              <div className="flex h-8 shrink-0 items-center gap-1 border-b border-border bg-surface-2 px-1">
                <IconButton
                  icon={Columns}
                  label="Side-by-side"
                  size={18}
                  active={viewMode === "split"}
                  onClick={() => switchView("split")}
                />
                <IconButton
                  icon={ArrowsLeftRight}
                  label="Swipe slider"
                  size={18}
                  active={viewMode === "slider"}
                  onClick={() => switchView("slider")}
                />
                <span className="mx-1 h-4 w-px bg-border" />
                <IconButton
                  icon={ArrowsIn}
                  label="Reset zoom"
                  size={18}
                  disabled={zoom.scale === 1}
                  onClick={zoom.reset}
                />
                <span className="text-[11px] tabular-nums text-text-muted w-9 text-center">
                  {Math.round(zoom.scale * 100)}%
                </span>
                <span className="mx-1 h-4 w-px bg-border" />
                <IconButton
                  icon={highlightOn ? Eye : EyeSlash}
                  label={highlightOn ? "Hide change highlight" : "Show change highlight"}
                  size={18}
                  active={highlightOn}
                  onClick={() => setHighlightOn((v) => !v)}
                />
                <IconButton
                  icon={Sparkle}
                  label="Highlight: changed pixels"
                  size={18}
                  active={highlightOn && highlightMode === "pixels"}
                  disabled={!highlightOn}
                  onClick={() => setHighlightMode("pixels")}
                />
                <IconButton
                  icon={BoundingBox}
                  label="Highlight: region boxes"
                  size={18}
                  active={highlightOn && highlightMode === "box"}
                  disabled={!highlightOn}
                  onClick={() => setHighlightMode("box")}
                />
              </div>

              {/* Canvas — overflow-hidden, never auto: the SVG scales to fit its pane, so
                  scrolling could only ever crop the artwork. Wheel zooms toward the cursor;
                  middle-mouse or plain left-drag pans (the slider divider stops propagation on
                  its own pointerdown so dragging it doesn't also pan). Wheel is NOT bound here
                  in split view — see the per-Pane comment. */}
              <div
                className={`relative min-h-0 flex-1 overflow-hidden ${zoom.panCursor}`}
                onWheel={viewMode === "slider" ? zoom.onWheel : undefined}
                onPointerDown={zoom.onPointerDown}
                onPointerMove={zoom.onPointerMove}
                onPointerUp={zoom.onPointerUp}
              >
                {viewMode === "split" ? (
                  <div className="flex h-full">
                    <Pane
                      label="Before"
                      diff={effectiveDiff}
                      layers={layers}
                      state="before"
                      transform={zoom.transform}
                      onWheel={zoom.onWheel}
                    />
                    <div className="w-px shrink-0 bg-border" />
                    <Pane
                      label="After"
                      diff={effectiveDiff}
                      layers={layers}
                      state="after"
                      overlay={highlightOn}
                      highlightMode={highlightMode}
                      diffImage={overlay.diffImage}
                      diffOutline={overlay.diffOutline}
                      regions={overlay.regions}
                      transform={zoom.transform}
                      onWheel={zoom.onWheel}
                    />
                  </div>
                ) : (
                  <CompareSlider
                    diff={effectiveDiff}
                    layers={layers}
                    overlay={highlightOn}
                    highlightMode={highlightMode}
                    diffImage={overlay.diffImage}
                    diffOutline={overlay.diffOutline}
                    regions={overlay.regions}
                    transform={zoom.transform}
                  />
                )}
                {/* The selected layer's raster is still streaming in. */}
                {pendingIds.has(selectedId) && (
                  <div className="absolute inset-0 grid place-items-center bg-bg/40">
                    <CircleNotchIcon size={24} className="animate-spin text-accent" />
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
