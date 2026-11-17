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

/** True for a Krita group layer, which carries no pixels of its own. */
export function isGroupLayer(layer: ArtLayer): boolean {
  return layer.layerType === "grouplayer";
}

/**
 * The visible layers inside a group, bottom→top in `layers`' order, for drawing the group as
 * the composite of its contents (a group has no raster of its own, so drawn alone it is blank).
 *
 * Matched on `topLevelId`, which records only a layer's *top-level* ancestor. A top-level group
 * therefore gets everything nested under it, but a group nested inside another one finds no
 * children and stays blank, and the nested group's own opacity isn't applied. That's acceptable
 * for a thumbnail and a preview. Switch to a real parent chain once `LayerDto` carries one.
 */
export function groupChildren(layers: ArtLayer[], group: ArtLayer): ArtLayer[] {
  return layers.filter(
    (l) => l.id !== group.id && l.topLevelId === group.id && l.visible !== false && !isGroupLayer(l)
  );
}

/**
 * A group drawn as the composite of its contents: the group's own metadata, with `before`/`after`
 * markup stacked from its children's. Fed back through `layersBody`, the group's opacity and
 * blend mode then apply to the stacked whole, the way Krita composites a (non-pass-through)
 * group. `thumbs` uses the children's small thumbnails, for the navigator's row.
 */
export function groupAsLayer(layers: ArtLayer[], group: ArtLayer, thumbs = false): ArtLayer {
  const children = groupChildren(layers, group).map((l) =>
    thumbs ? { ...l, before: l.beforeThumb ?? l.before, after: l.afterThumb ?? l.after } : l
  );
  const side = (state: DiffState) => {
    const body = layersBody(children, state);
    return body === "" ? null : body;
  };
  return { ...group, before: side("before"), after: side("after") };
}
