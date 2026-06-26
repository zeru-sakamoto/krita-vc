import { useState } from "react";
import type { PaletteDiff, PaletteSwatch, SwatchChange } from "../../types";

/** Small diagonal-split square: top-left = before, bottom-right = after. */
function SplitSwatch({ before, after }: { before: string; after: string }) {
  return (
    <svg viewBox="0 0 40 40" className="h-full w-full" aria-hidden>
      {/* Bottom-right triangle = "after" */}
      <polygon points="0,40 40,0 40,40" fill={after} />
      {/* Top-left triangle = "before" */}
      <polygon points="0,0 40,0 0,40" fill={before} />
      {/* Divider line */}
      <line x1="0" y1="40" x2="40" y2="0" stroke="rgba(0,0,0,0.25)" strokeWidth="1" />
    </svg>
  );
}

const CHANGE_RING: Record<SwatchChange, string> = {
  added: "ring-2 ring-diff-add-fg",
  removed: "ring-2 ring-diff-del-fg opacity-50",
  modified: "ring-2 ring-accent",
  unchanged: "",
};

const CHANGE_BADGE: Record<SwatchChange, string | null> = {
  added: "Added",
  removed: "Removed",
  modified: null,
  unchanged: null,
};

const HEX_BASE =
  "cursor-pointer font-mono text-[11px] leading-none transition-colors duration-200 [text-shadow:0_0_4px_rgba(0,0,0,1),0_0_2px_rgba(0,0,0,1)]";

function SwatchCell({ swatch }: { swatch: PaletteSwatch }) {
  const { name, before, after, change } = swatch;
  const displayColor = after ?? before ?? "#888888";
  const badge = CHANGE_BADGE[change];
  const hexLabel = change === "modified" && before && after ? `${before} → ${after}` : displayColor;

  const [copied, setCopied] = useState<string | null>(null);
  function handleCopy(text: string) {
    void navigator.clipboard.writeText(text);
    setCopied(text);
    setTimeout(() => setCopied(null), 1500);
  }

  return (
    <div className="group relative flex flex-col gap-0.5" title={`${name}: ${hexLabel}`}>
      {/* Color square */}
      <div
        className={[
          "relative h-10 w-full overflow-hidden rounded-badge border border-border",
          CHANGE_RING[change],
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {change === "modified" && before && after ? (
          <>
            <SplitSwatch before={before} after={after} />
            {/* Old hex — left half (top-left triangle = before color) */}
            <span
              className={[
                "absolute left-1 top-1/2 -translate-y-1/2",
                HEX_BASE,
                copied === before ? "text-diff-add-fg" : "text-white",
              ].join(" ")}
              onClick={(e) => {
                e.stopPropagation();
                handleCopy(before);
              }}
            >
              {copied === before ? "✓" : before}
            </span>
            {/* New hex — right half (bottom-right triangle = after color) */}
            <span
              className={[
                "absolute right-1 top-1/2 -translate-y-1/2",
                HEX_BASE,
                copied === after ? "text-diff-add-fg" : "text-white",
              ].join(" ")}
              onClick={(e) => {
                e.stopPropagation();
                handleCopy(after);
              }}
            >
              {copied === after ? "✓" : after}
            </span>
          </>
        ) : (
          <>
            <div className="h-full w-full" style={{ backgroundColor: displayColor }} />
            {/* Hex code centered inside the color block */}
            <span
              className={[
                "absolute inset-0 flex items-center justify-center",
                HEX_BASE,
                copied === displayColor ? "text-diff-add-fg" : "text-white",
              ].join(" ")}
              onClick={(e) => {
                e.stopPropagation();
                handleCopy(displayColor);
              }}
            >
              {copied === displayColor ? "✓" : displayColor}
            </span>
          </>
        )}

        {/* Change badge overlay (added / removed only) */}
        {badge && (
          <span
            className={[
              "pointer-events-none absolute bottom-0.5 right-0.5 rounded-badge px-1 py-px text-[9px] font-semibold uppercase leading-none",
              change === "added" ? "bg-diff-add text-diff-add-fg" : "bg-diff-del text-diff-del-fg",
            ].join(" ")}
          >
            {badge}
          </span>
        )}
      </div>

      {/* Name */}
      <span
        className={[
          "truncate text-center font-mono text-[10px] leading-tight",
          change === "removed" ? "text-text-muted line-through" : "text-text-muted",
        ].join(" ")}
      >
        {name}
      </span>
    </div>
  );
}

/** Legend entry */
function LegendItem({ color, label }: { color: string; label: string }) {
  return (
    <span className="flex items-center gap-1 text-[11px] text-text-muted">
      <span className={["h-2.5 w-2.5 rounded-sm border", color].join(" ")} />
      {label}
    </span>
  );
}

/**
 * Visual diff for a color palette (.gpl) file: a grid of color squares that
 * highlights which swatches were added, removed, or changed.
 * (Mirrors ArtCanvas in purpose — the right-pane content when the palette is
 * selected in LayerStackPanel's Color Palette section.)
 */
export function PaletteDiffView({ diff }: { diff: PaletteDiff }) {
  const cols = diff.columns || 4;

  return (
    <div className="flex h-full flex-col overflow-auto bg-bg">
      {/* Legend bar */}
      <div className="flex h-8 shrink-0 items-center gap-4 border-b border-border bg-surface-2 px-3">
        <LegendItem color="border-diff-add-fg bg-diff-add" label="Added" />
        <LegendItem color="border-diff-del-fg bg-diff-del" label="Removed" />
        <LegendItem color="border-accent bg-accent/20" label="Changed" />
        <LegendItem color="border-border bg-surface" label="Unchanged" />
      </div>

      {/* Swatch grid */}
      <div
        className="p-4"
        style={{
          display: "grid",
          gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
          gap: "0.5rem",
        }}
      >
        {diff.swatches.map((sw) => (
          <SwatchCell key={sw.name} swatch={sw} />
        ))}
      </div>
    </div>
  );
}
