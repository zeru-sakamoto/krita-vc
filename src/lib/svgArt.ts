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

/**
 * A layer's opacity, blend mode, visibility and group at one state. The layer's own fields
 * describe the new version; `prev` holds them as they were whenever any changed, so the "before"
 * side reads from it when present.
 */
export function layerPropsAt(l: ArtLayer, state: DiffState) {
  const p = state === "before" ? l.prev : undefined;
  return {
    opacity: p ? p.opacity : l.opacity,
    blendMode: p ? p.blendMode : l.blendMode,
    visible: (p ? p.visible : l.visible) !== false,
    parentId: p ? p.parentId : l.parentId,
  };
}

/**
 * Inner markup (no `<svg>` wrapper) for the given layers at one state, bottom→top. Visibility is
 * deliberately not applied: a layer selected on its own is drawn even when hidden, so you can see
 * what was hidden. Callers stacking several layers filter on `layerPropsAt(..).visible`.
 */
export function layersBody(layers: ArtLayer[], state: DiffState): string {
  return layers
    .map((l) => {
      const markup = state === "before" ? l.before : l.after;
      if (markup == null) return "";
      const { opacity, blendMode } = layerPropsAt(l, state);
      const style = `opacity:${opacity / 100};mix-blend-mode:${blendCss(blendMode)}`;
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
 * The layers directly inside `group` that are visible at `state`, bottom→top in `layers`' order,
 * for drawing the group as the composite of its contents (a group has no raster of its own, so
 * drawn alone it is blank).
 *
 * Membership and visibility are read per state: a layer that moved into the group, or was shown,
 * belongs to its "after" side only. Nested groups are included, and `groupAsLayer` draws them
 * from their own children in turn. Data without any `parentId` (the browser build's mock repo)
 * falls back to `topLevelId`, which only knows a layer's top-level ancestor, so nested groups
 * there stay flat.
 */
export function groupChildren(
  layers: ArtLayer[],
  group: ArtLayer,
  state: DiffState = "after"
): ArtLayer[] {
  const hasTree = layers.some((l) => l.parentId !== undefined);
  return layers.filter((l) => {
    if (l.id === group.id) return false;
    const props = layerPropsAt(l, state);
    if (!props.visible) return false;
    return hasTree ? props.parentId === group.id : l.topLevelId === group.id && !isGroupLayer(l);
  });
}

/**
 * A group drawn as the composite of its contents: the group's own metadata, with `before`/`after`
 * markup stacked from its children's. Fed back through `layersBody`, the group's opacity and
 * blend mode then apply to the stacked whole, the way Krita composites a (non-pass-through)
 * group, and a nested group is drawn the same way so its own opacity applies too. `thumbs` uses
 * the children's small thumbnails, for the navigator's row.
 */
export function groupAsLayer(layers: ArtLayer[], group: ArtLayer, thumbs = false): ArtLayer {
  const side = (state: DiffState) => {
    const children = groupChildren(layers, group, state).map((l) => {
      if (isGroupLayer(l)) return groupAsLayer(layers, l, thumbs);
      return thumbs
        ? { ...l, before: l.beforeThumb ?? l.before, after: l.afterThumb ?? l.after }
        : l;
    });
    const body = layersBody(children, state);
    return body === "" ? null : body;
  };
  return { ...group, before: side("before"), after: side("after") };
}
