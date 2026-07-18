import { memo, useMemo } from "react";
import type { ArtDiff, ArtLayer, ChangeRegion, DiffState } from "../../types";
import { layersBody, wrapSvg } from "../../lib/svgArt";

// Theme-reactive: reads the active theme's `--color-accent` (global.css), not a fixed hex.
const ACCENT = "var(--color-accent)";

// Region labels are backend-supplied text spliced into an SVG string that's injected via
// dangerouslySetInnerHTML downstream — escape the markup-significant characters first.
const escapeXml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export type HighlightMode = "box" | "pixels";

interface ArtCanvasProps {
  diff: ArtDiff;
  /** Layers to composite (full stack for "composite", or a single focused layer). */
  layers: ArtLayer[];
  state: DiffState;
  /** Draw the change-highlight overlay on top of this canvas. */
  overlay?: boolean;
  highlightMode?: HighlightMode;
  /**
   * The change-highlight source, chosen by the caller from the current selection: the composite's
   * `diffImage`/`diffOutline`/`regions` for the Composite view, or the selected layer's own. When
   * these are absent (e.g. an unchanged/added/removed layer), the overlay simply doesn't draw —
   * the composite's highlight is never reused for a single layer.
   */
  diffImage?: string | null;
  diffOutline?: string | null;
  regions?: ChangeRegion[];
  /**
   * Shared zoom/pan CSS transform (e.g. "translate(10px,4px) scale(2)"). Applied to the
   * SVG-wrapping div only, so the memoized SVG string and its DOM stay untouched during
   * interaction — the compositor handles the transform off the main thread.
   */
  transform?: string;
  className?: string;
}

/**
 * Change-region markers: a subtle filled rect plus **bold corner brackets**. All strokes use
 * `vector-effect="non-scaling-stroke"` so their width stays constant in *screen* pixels — on a
 * large canvas shown fit-to-pane the plain document-space dashes go sub-pixel and vanish, but the
 * brackets stay legible at any zoom-out. Arm length is a fraction of the region (capped vs the
 * canvas) so brackets read as corners on both tiny and near-full-canvas regions.
 */
function boxOverlay(diff: ArtDiff, regions: ChangeRegion[]): string {
  const { width: W, height: H } = diff;
  return regions
    .map((r) => {
      const x = r.x * W;
      const y = r.y * H;
      const w = r.w * W;
      const h = r.h * H;
      const arm = Math.min(Math.min(w, h) * 0.3, Math.min(W, H) * 0.1);
      const corners = [
        `M${x} ${y + arm} L${x} ${y} L${x + arm} ${y}`, // top-left
        `M${x + w - arm} ${y} L${x + w} ${y} L${x + w} ${y + arm}`, // top-right
        `M${x + w} ${y + h - arm} L${x + w} ${y + h} L${x + w - arm} ${y + h}`, // bottom-right
        `M${x + arm} ${y + h} L${x} ${y + h} L${x} ${y + h - arm}`, // bottom-left
      ].join(" ");
      const label = r.label
        ? `<text x="${x + 4}" y="${y + 14}" font-family="sans-serif" font-size="11" fill="${ACCENT}">${escapeXml(r.label)}</text>`
        : "";
      return (
        `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${ACCENT}" fill-opacity="0.12" ` +
        `stroke="${ACCENT}" stroke-width="1.5" stroke-dasharray="6 4" rx="3" vector-effect="non-scaling-stroke"/>` +
        `<path d="${corners}" fill="none" stroke="${ACCENT}" stroke-width="3" stroke-linecap="round" ` +
        `stroke-linejoin="round" vector-effect="non-scaling-stroke"/>${label}`
      );
    })
    .join("");
}

/**
 * Changed-pixel overlay: the backend mask (`diff.diffImage`) supplies the *shape* (its alpha
 * channel only — its baked-in RGB is never shown), recolored with the active theme's accent and
 * painted three ways so the change reads on top of busy artwork without an expensive filter —
 *  1. a flat accent tint of the changed pixels (area sense),
 *  2. a diagonal **hatch pattern** masked to the same pixels — the alternating stripes give
 *     high-frequency contrast that survives against any underlying color (a flat tint blends),
 *  3. a **dashed outline** that hugs the changed pixels' silhouette (`diff.diffOutline`, a vector
 *     path traced by the backend — not a bounding box), `non-scaling-stroke` so its width and dash
 *     length stay constant on screen at any zoom.
 * All of this is plain fills/patterns/masks/paths (GPU-composited) and only rebuilds when the
 * memoized SVG string changes — never on zoom/pan (that's a CSS transform on the wrapper). The
 * hatch tile is sized relative to the canvas so it doesn't go sub-pixel on a large canvas at fit view.
 */
function pixelOverlay(
  diff: ArtDiff,
  diffImage?: string | null,
  diffOutline?: string | null
): string {
  const img = diffImage;
  if (!img) return "";
  const { width: W, height: H } = diff;
  // Unique ids per file so two inline SVGs on screen don't cross-reference each other's defs.
  const uid = diff.path.replace(/[^a-zA-Z0-9]/g, "-");
  const tile = Math.max(6, Math.min(W, H) * 0.014);
  const defs =
    `<defs>` +
    `<pattern id="kvc-hatch-${uid}" width="${tile}" height="${tile}" ` +
    `patternUnits="userSpaceOnUse" patternTransform="rotate(45)">` +
    `<rect width="${tile / 2}" height="${tile}" fill="${ACCENT}" fill-opacity="0.6"/>` +
    `</pattern>` +
    // mask-type:alpha → use only the raster's alpha (its baked-in RGB is ignored so the overlay
    // stays theme-reactive instead of the backend's fixed mask color); changed pixels become the
    // mask, everything else is cut away.
    `<mask id="kvc-diffmask-${uid}" style="mask-type:alpha">${img}</mask>` +
    `</defs>`;
  // Outline path is normalized 0..1 → scale it to the viewBox. non-scaling-stroke keeps the dashes
  // a constant on-screen size despite the scale (and any zoom transform on the wrapper).
  const outline = diffOutline
    ? `<g transform="scale(${W} ${H})"><path d="${diffOutline}" fill="none" ` +
      `stroke="${ACCENT}" stroke-width="1.5" stroke-dasharray="5 4" stroke-linejoin="round" ` +
      `vector-effect="non-scaling-stroke"/></g>`
    : "";
  return (
    defs +
    // flat tint (area) — same mask as the hatch, so ACCENT (not the backend's baked color) shows.
    `<rect x="0" y="0" width="${W}" height="${H}" fill="${ACCENT}" mask="url(#kvc-diffmask-${uid})"/>` +
    `<rect x="0" y="0" width="${W}" height="${H}" fill="url(#kvc-hatch-${uid})" ` +
    `mask="url(#kvc-diffmask-${uid})"/>` +
    outline
  );
}

/**
 * Renders one artwork state as a self-contained SVG over a checkerboard matte
 * (so layer transparency reads true), with an optional change-highlight overlay.
 * Memoized: the parent (slider divider drag / zoom-pan) re-renders often but props are
 * stable, so memo keeps the two stacked canvases from re-entering their render bodies.
 */
export const ArtCanvas = memo(function ArtCanvas({
  diff,
  layers,
  state,
  overlay = false,
  highlightMode = "pixels",
  diffImage,
  diffOutline,
  regions,
  transform,
  className = "",
}: ArtCanvasProps) {
  const svg = useMemo(() => {
    let body = layersBody(layers, state);
    if (overlay) {
      // "pixels": the changed-pixel mask, tinted + hatched + framed for legibility on busy
      // artwork. "box": coarse region rectangles with corner brackets. Both draw from the
      // caller-supplied overlay data (composite or the selected layer's own), so a single layer
      // never shows the whole-file composite highlight.
      body +=
        highlightMode === "pixels"
          ? pixelOverlay(diff, diffImage, diffOutline)
          : boxOverlay(diff, regions ?? []);
    }
    return wrapSvg(body, diff.width, diff.height);
  }, [layers, state, overlay, highlightMode, diff, diffImage, diffOutline, regions]);

  return (
    <div
      className={[
        // Plain block, not `grid place-items-center`: the child below is unconditionally
        // forced h-full/w-full anyway, and place-items-center's align-items:center stops it
        // from stretching — its percentage height then resolves against its own aspect-ratio-
        // driven size instead of this container's, so a portrait canvas silently overflows
        // downward past the pane (visible whenever the width-driven height exceeds the
        // actual available height — full-width swipe-slider panes hit this far more than
        // split view's half-width panes, but a wide-enough split pane hits it too).
        "h-full w-full overflow-hidden",
        // Checkerboard matte — transparency reads true, Krita-style.
        "bg-[repeating-conic-gradient(#1a1916_0%_25%,#222019_0%_50%)] bg-size-[16px_16px]",
        className,
      ].join(" ")}
    >
      <div
        className="h-full w-full [&>svg]:h-full [&>svg]:w-full [&>svg]:object-contain"
        // willChange promotes the heavy inline-SVG subtree to its own compositor layer, so
        // zoom/pan only moves a cached texture instead of repainting the SVG per frame.
        style={
          transform ? { transform, transformOrigin: "0 0", willChange: "transform" } : undefined
        }
        // Inline SVG (not <img>) so blend modes + filters composite against the matte.
        dangerouslySetInnerHTML={{ __html: svg }}
      />
    </div>
  );
});
