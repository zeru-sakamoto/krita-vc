// SVG compositing helpers for the visual diff viewer. Each layer's raster arrives as a
// string of inner SVG markup (an `<image>` for real .kra rasters), so the canvas can
// composite layers and swap before/after states without a raster pipeline of its own.

import type { ArtLayer, BlendMode, DiffState } from "../types";

/** CSS mix-blend-mode for a Krita-ish blend mode. */
export function blendCss(mode: BlendMode): string {
  switch (mode) {
    case "multiply":
      return "multiply";
    case "screen":
      return "screen";
    case "overlay":
      return "overlay";
    case "add":
      return "plus-lighter";
    default:
      return "normal";
  }
}

/** Inner markup (no `<svg>` wrapper) for the given layers at one state, bottom→top. */
export function layersBody(layers: ArtLayer[], state: DiffState): string {
  return layers
    .map((l) => {
      const markup = state === "before" ? l.before : l.after;
      if (markup == null) return "";
      const style = `opacity:${l.opacity / 100};mix-blend-mode:${blendCss(l.blendMode)}`;
      return `<g style="${style}">${markup}</g>`;
    })
    .join("");
}

/** Wrap inner markup in a self-contained, scalable `<svg>` string. */
export function wrapSvg(body: string, width: number, height: number): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" preserveAspectRatio="xMidYMid meet">${body}</svg>`;
}

/**
 * Composite the given layers into a single `<svg>` string at one state.
 * Layers are expected bottom→top; null markup at this state is skipped.
 */
export function compositeSvg(
  layers: ArtLayer[],
  state: DiffState,
  width: number,
  height: number
): string {
  return wrapSvg(layersBody(layers, state), width, height);
}
