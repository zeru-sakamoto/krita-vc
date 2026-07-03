import { CircleNotchIcon } from "@phosphor-icons/react";
import type { ArtDiff, ArtLayer, FileStatus, PaletteDiff } from "../../types";
import { compositeSvg } from "../../lib/svgArt";
import { assetName } from "../../lib/friendly";
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
 */
export function LayerStackPanel({
  diff,
  palette,
  selectedId,
  onSelect,
  pendingIds,
}: LayerStackPanelProps) {
  // Derive an overall palette change status: prefer M > A > D over unchanged swatches.
  const paletteStatus: FileStatus | null = palette ? (palette.status ?? null) : null;

  return (
    <div className="flex w-50 shrink-0 flex-col border-r border-border bg-surface">
      <div className="min-h-0 flex-1 overflow-auto">
        {/* ── Layers ── */}
        <SectionLabel>Layers</SectionLabel>

        {/* Composite */}
        <Row selected={selectedId === COMPOSITE_ID} onClick={() => onSelect(COMPOSITE_ID)}>
          <Thumb svg={compositeSvg(diff.layers, "after", diff.width, diff.height)} />
          <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-text">
            Composite
          </span>
        </Row>

        {/* Layers, top-first */}
        {[...diff.layers].reverse().map((l) => {
          const state = l.after != null ? "after" : "before";
          const status = CHANGE_STATUS[l.change];
          return (
            <Row key={l.id} selected={selectedId === l.id} onClick={() => onSelect(l.id)}>
              {pendingIds?.has(l.id) ? (
                <div className="grid h-7 w-9 shrink-0 place-items-center rounded-badge border border-border bg-surface-2">
                  <CircleNotchIcon size={12} className="animate-spin text-text-muted" />
                </div>
              ) : (
                <Thumb svg={compositeSvg([l], state, diff.width, diff.height)} />
              )}
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-[12px] text-text">{l.name}</span>
                <span className="truncate font-mono text-[10px] text-text-muted">
                  {l.opacity}% · {l.blendMode}
                </span>
              </span>
              {status && <FileStatusChip status={status} />}
            </Row>
          );
        })}

        {/* ── Color Palette (optional) ── */}
        {palette && (
          <>
            <SectionLabel>Color Palette</SectionLabel>
            <Row selected={selectedId === PALETTE_ID} onClick={() => onSelect(PALETTE_ID)}>
              <PaletteThumb swatches={palette.swatches} />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-[12px] text-text">{assetName(palette.path)}</span>
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
}
