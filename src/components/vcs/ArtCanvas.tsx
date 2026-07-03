import { useMemo } from "react";
import type { ArtDiff, ArtLayer, DiffState } from "../../types";
import { layersBody, wrapSvg } from "../../lib/svgArt";

const ACCENT = "#e07b39";

export type HighlightMode = "box" | "mask";

interface ArtCanvasProps {
  diff: ArtDiff;
  /** Layers to composite (full stack for "composite", or a single focused layer). */
  layers: ArtLayer[];
  state: DiffState;
  /** Draw the change-highlight overlay on top of this canvas. */
  overlay?: boolean;
  highlightMode?: HighlightMode;
  className?: string;
}

/** Translucent accent rectangles over each changed region. */
function boxOverlay(diff: ArtDiff): string {
  const { width: W, height: H } = diff;
  return diff.regions
    .map((r) => {
      const x = r.x * W;
      const y = r.y * H;
      const w = r.w * W;
      const h = r.h * H;
      const label = r.label
        ? `<text x="${x + 4}" y="${y + 14}" font-family="sans-serif" font-size="11" fill="${ACCENT}">${r.label}</text>`
        : "";
      return `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${ACCENT}" fill-opacity="0.16" stroke="${ACCENT}" stroke-width="2" stroke-dasharray="6 4" rx="3"/>${label}`;
    })
    .join("");
}

/** Recolor the changed layers' silhouettes to accent + glow (precise mask). */
function maskOverlay(diff: ArtDiff): string {
  const changed = diff.layers.filter((l) => l.change !== "unchanged");
  const body = changed
    .map((l) => l.after ?? l.before ?? "")
    .filter(Boolean)
    .join("");
  if (!body) return "";
  const filter = `<filter id="tint" x="-20%" y="-20%" width="140%" height="140%">
    <feFlood flood-color="${ACCENT}" result="flood"/>
    <feComposite in="flood" in2="SourceAlpha" operator="in" result="tinted"/>
    <feGaussianBlur in="tinted" stdDeviation="3" result="glow"/>
    <feMerge><feMergeNode in="glow"/><feMergeNode in="tinted"/></feMerge>
  </filter>`;
  return `${filter}<g filter="url(#tint)" opacity="0.75">${body}</g>`;
}

/**
 * Renders one artwork state as a self-contained SVG over a checkerboard matte
 * (so layer transparency reads true), with an optional change-highlight overlay.
 */
export function ArtCanvas({
  diff,
  layers,
  state,
  overlay = false,
  highlightMode = "box",
  className = "",
}: ArtCanvasProps) {
  const svg = useMemo(() => {
    let body = layersBody(layers, state);
    if (overlay) {
      body += highlightMode === "box" ? boxOverlay(diff) : maskOverlay(diff);
    }
    return wrapSvg(body, diff.width, diff.height);
  }, [layers, state, overlay, highlightMode, diff]);

  return (
    <div
      className={[
        "grid h-full w-full place-items-center overflow-hidden",
        // Checkerboard matte — transparency reads true, Krita-style.
        "bg-[repeating-conic-gradient(#1a1916_0%_25%,#222019_0%_50%)] bg-size-[16px_16px]",
        className,
      ].join(" ")}
    >
      <div
        className="h-full w-full [&>svg]:h-full [&>svg]:w-full [&>svg]:object-contain"
        // Inline SVG (not <img>) so blend modes + filters composite against the matte.
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    </div>
  );
}
