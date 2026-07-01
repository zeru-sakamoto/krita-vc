import { useMemo, useState } from "react";
import {
  ArrowsLeftRight,
  BoundingBox,
  Columns,
  Eye,
  EyeSlash,
  Sparkle,
} from "@phosphor-icons/react";
import type { ArtDiff, DiffState, PaletteDiff } from "../../types";
import { IconButton } from "../ui/IconButton";
import { FileStatusChip } from "./FileStatusChip";
import { useArtistMode } from "../../lib/artistMode";
import { assetName } from "../../lib/friendly";
import { ArtCanvas, type HighlightMode } from "./ArtCanvas";
import { CompareSlider } from "./CompareSlider";
import { LayerStackPanel, COMPOSITE_ID, PALETTE_ID } from "./LayerStackPanel";
import { PaletteDiffView } from "./PaletteDiffView";

type ViewMode = "split" | "slider";

function Pane({
  label,
  diff,
  layers,
  state,
  overlay,
  highlightMode,
}: {
  label: string;
  diff: ArtDiff;
  layers: ArtDiff["layers"];
  state: DiffState;
  overlay?: boolean;
  highlightMode?: HighlightMode;
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex h-6 shrink-0 items-center border-b border-border bg-surface px-2 text-[11px] font-medium uppercase text-text-muted">
        {label}
      </div>
      <div className="min-h-0 flex-1">
        <ArtCanvas
          diff={diff}
          layers={layers}
          state={state}
          overlay={overlay}
          highlightMode={highlightMode}
        />
      </div>
    </div>
  );
}

/**
 * Visual diff for one art (.kra) file: a layer stack panel beside a before/after
 * canvas. Toolbar toggles side-by-side vs swipe slider, and the change-highlight
 * overlay (on/off, translucent box vs precise mask).
 *
 * When a `palette` prop is supplied, a "Color Palette" section appears below the
 * layers in the navigator. Selecting it swaps the right pane to the palette grid.
 */
export function ArtDiffView({ diff, palette }: { diff: ArtDiff; palette?: PaletteDiff }) {
  const { artistMode } = useArtistMode();
  const [selectedId, setSelectedId] = useState<string>(COMPOSITE_ID);
  const [viewMode, setViewMode] = useState<ViewMode>("split");
  const [highlightOn, setHighlightOn] = useState(true);
  const [highlightMode, setHighlightMode] = useState<HighlightMode>("box");

  const layers = useMemo(() => {
    if (selectedId === COMPOSITE_ID) {
      // Prefer the backend's real composite (mergedimage.png) over stacking layers, so the
      // composite is correct even if some layers can't be rastered. Mock data omits these.
      if (diff.beforeImage !== undefined || diff.afterImage !== undefined) {
        const composite: ArtDiff["layers"][number] = {
          id: COMPOSITE_ID,
          name: "Composite",
          opacity: 100,
          blendMode: "normal",
          change: "modified",
          before: diff.beforeImage ?? null,
          after: diff.afterImage ?? null,
        };
        return [composite];
      }
      return diff.layers;
    }
    const found = diff.layers.find((l) => l.id === selectedId);
    return found ? [found] : diff.layers;
  }, [diff, selectedId]);

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
      </div>

      <div className="flex flex-1 min-h-0">
        <LayerStackPanel
          diff={diff}
          palette={palette}
          selectedId={selectedId}
          onSelect={setSelectedId}
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
                  onClick={() => setViewMode("split")}
                />
                <IconButton
                  icon={ArrowsLeftRight}
                  label="Swipe slider"
                  size={18}
                  active={viewMode === "slider"}
                  onClick={() => setViewMode("slider")}
                />
                <span className="mx-1 h-4 w-px bg-border" />
                <IconButton
                  icon={highlightOn ? Eye : EyeSlash}
                  label={highlightOn ? "Hide change highlight" : "Show change highlight"}
                  size={18}
                  active={highlightOn}
                  onClick={() => setHighlightOn((v) => !v)}
                />
                <IconButton
                  icon={BoundingBox}
                  label="Highlight: region boxes"
                  size={18}
                  active={highlightOn && highlightMode === "box"}
                  disabled={!highlightOn}
                  onClick={() => setHighlightMode("box")}
                />
                <IconButton
                  icon={Sparkle}
                  label="Highlight: precise mask"
                  size={18}
                  active={highlightOn && highlightMode === "mask"}
                  disabled={!highlightOn}
                  onClick={() => setHighlightMode("mask")}
                />
              </div>

              {/* Canvas */}
              <div className="min-h-0 flex-1 overflow-auto">
                {viewMode === "split" ? (
                  <div className="flex h-full">
                    <Pane label="Before" diff={diff} layers={layers} state="before" />
                    <div className="w-px shrink-0 bg-border" />
                    <Pane
                      label="After"
                      diff={diff}
                      layers={layers}
                      state="after"
                      overlay={highlightOn}
                      highlightMode={highlightMode}
                    />
                  </div>
                ) : (
                  <CompareSlider
                    diff={diff}
                    layers={layers}
                    overlay={highlightOn}
                    highlightMode={highlightMode}
                  />
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
