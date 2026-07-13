import { memo, useMemo } from "react";
import { CircleNotchIcon } from "@phosphor-icons/react";
import type { ArtDiff, ArtLayer, FileStatus, PaletteDiff } from "../../types";
import { compositeSvg } from "../../lib/svgArt";
import { paletteName } from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";
import { FileStatusChip } from "./FileStatusChip";

export const COMPOSITE_ID = "composite";
export const PALETTE_ID = "palette";

const CHANGE_STATUS: Record<ArtLayer["change"], FileStatus | null> = {
  added: "A",
  removed: "D",
  modified: "M",
  unchanged: null,
};

function Thumb({ svg }: { svg: string }) {
  return (
    <div
      className="h-7 w-9 shrink-0 overflow-hidden rounded-badge border border-border bg-[repeating-conic-gradient(#1a1916_0%_25%,#222019_0%_50%)] bg-size-[8px_8px] [&>svg]:h-full [&>svg]:w-full"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

/** Small multi-swatch thumbnail for a palette: first N after/before hex colors side by side. */
function PaletteThumb({ swatches }: { swatches: PaletteDiff["swatches"] }) {
  // Show up to 4 swatches as color strips inside the same thumb shell.
  const shown = swatches.slice(0, 4);
  return (
    <div className="flex h-7 w-9 shrink-0 overflow-hidden rounded-badge border border-border">
      {shown.map((sw) => (
        <div
          key={sw.name}
          className="flex-1"
          style={{ backgroundColor: sw.after ?? sw.before ?? "#888" }}
        />
      ))}
    </div>
  );
}

interface RowProps {
  selected: boolean;
  onClick: () => void;
  children: React.ReactNode;
}

function Row({ selected, onClick, children }: RowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={[
        "flex w-full items-center gap-2 border-l-2 px-2 py-1.5 text-left transition-colors",
        selected ? "border-accent bg-accent/12" : "border-transparent hover:bg-white/5",
      ].join(" ")}
    >
      {children}
    </button>
  );
}

/** Thin divider heading between sections inside the panel body. */
function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-6 shrink-0 items-center border-b border-border bg-surface-2 px-2">
      <span className="text-[11px] font-medium uppercase tracking-wide text-text-muted">
        {children}
      </span>
    </div>
  );
}

/**
 * One layer row. Memoized because rebuilding the thumb's SVG re-serializes the layer's
 * raster markup (multi-MB on the base64 fallback) — with N layers and one parent render per
 * zoom/pan frame or streamed-layer message, unmemoized rows turn into O(N²) string churn.
 */
const LayerRow = memo(function LayerRow({
  layer,
  width,
  height,
  selected,
  pending,
  onSelect,
}: {
  layer: ArtLayer;
  width: number;
  height: number;
  selected: boolean;
  pending: boolean;
  onSelect: (id: string) => void;
}) {
  const state = layer.after != null ? "after" : "before";
  const status = CHANGE_STATUS[layer.change];
  const svg = useMemo(
    () => compositeSvg([layer], state, width, height),
    [layer, state, width, height]
  );
  return (
    <Row selected={selected} onClick={() => onSelect(layer.id)}>
      {pending ? (
        <div className="grid h-7 w-9 shrink-0 place-items-center rounded-badge border border-border bg-surface-2">
          <CircleNotchIcon size={12} className="animate-spin text-text-muted" />
        </div>
      ) : (
        <Thumb svg={svg} />
      )}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-[12px] text-text">{layer.name}</span>
        <span className="truncate font-mono text-[10px] text-text-muted">
          {layer.opacity}% · {layer.blendMode}
        </span>
      </span>
      {status && <FileStatusChip status={status} />}
    </Row>
  );
});

interface LayerStackPanelProps {
  diff: ArtDiff;
  /** Optional palette diff — renders a "Color Palette" section below the layer list. */
  palette?: PaletteDiff;
  selectedId: string;
  onSelect: (id: string) => void;
  /** Layers whose rasters are still streaming in — their thumbs show a spinner. */
  pendingIds?: Set<string>;
}

/**
 * Krita-style navigator list for an ArtDiff. Rendered in two labeled sections:
 *   1. Layers — Composite row + individual layers (bottom→top reversed to top-first).
 *   2. Color Palette — single selectable row per palette (when palette prop is provided).
 * (DESIGN.md → Layout & App Shell → Docker / Panel System)
 *
 * Memoized: the parent re-renders on every zoom/pan frame, but the navigator's props only
 * change when layers stream in or the selection moves.
 */
export const LayerStackPanel = memo(function LayerStackPanel({
  diff,
  palette,
  selectedId,
  onSelect,
  pendingIds,
}: LayerStackPanelProps) {
  const { artistMode } = useArtistMode();
  // Derive an overall palette change status: prefer M > A > D over unchanged swatches.
  const paletteStatus: FileStatus | null = palette ? (palette.status ?? null) : null;

  // Prefer the backend's real composite (mergedimage.png) for the thumb — one small image
  // instead of concatenating every layer's raster markup (mirrors the canvas's preference).
  const compositeThumb = useMemo(() => {
    const img = diff.afterImage ?? diff.beforeImage;
    if (img != null) {
      const composite: ArtLayer = {
        id: COMPOSITE_ID,
        name: "Composite",
        opacity: 100,
        blendMode: "normal",
        change: "modified",
        before: null,
        after: img,
      };
      return compositeSvg([composite], "after", diff.width, diff.height);
    }
    return compositeSvg(diff.layers, "after", diff.width, diff.height);
  }, [diff.afterImage, diff.beforeImage, diff.layers, diff.width, diff.height]);

  return (
    <div className="flex w-50 shrink-0 flex-col border-r border-border bg-surface">
      <div className="min-h-0 flex-1 overflow-auto">
        {/* ── Layers ── */}
        <SectionLabel>Layers</SectionLabel>

        {/* Composite */}
        <Row selected={selectedId === COMPOSITE_ID} onClick={() => onSelect(COMPOSITE_ID)}>
          <Thumb svg={compositeThumb} />
          <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-text">
            Composite
          </span>
        </Row>

        {/* Layers, top-first */}
        {[...diff.layers].reverse().map((l) => (
          <LayerRow
            key={l.id}
            layer={l}
            width={diff.width}
            height={diff.height}
            selected={selectedId === l.id}
            pending={pendingIds?.has(l.id) ?? false}
            onSelect={onSelect}
          />
        ))}

        {/* ── Color Palette (optional) ── */}
        {palette && (
          <>
            <SectionLabel>Color Palette</SectionLabel>
            <Row selected={selectedId === PALETTE_ID} onClick={() => onSelect(PALETTE_ID)}>
              <PaletteThumb swatches={palette.swatches} />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-[12px] text-text">
                  {artistMode ? paletteName(palette.path) : palette.path}
                </span>
                <span className="truncate font-mono text-[10px] text-text-muted">
                  {palette.swatches.length} swatches
                </span>
              </span>
              {paletteStatus && <FileStatusChip status={paletteStatus} />}
            </Row>
          </>
        )}
      </div>
    </div>
  );
});
