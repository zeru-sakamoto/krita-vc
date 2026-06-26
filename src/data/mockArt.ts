// =============================================================================
// === MOCK ART — generated inline SVG "artwork" for the visual diff viewer.  ===
// === No external assets, no deps: each layer is a string of SVG markup so   ===
// === the canvas can composite layers and swap before/after states offline.  ===
// === Replace with real .kra layer rasters when the Rust/Tauri backend lands.===
// =============================================================================

import type { ArtDiff, ArtLayer, BlendMode, DiffState, FileStatus } from "../types";

// --- helpers ----------------------------------------------------------------

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

interface LayerOpts {
  opacity?: number;
  blendMode?: BlendMode;
  change?: ArtLayer["change"];
}

/** Terse builder so scenes stay readable. `after` defaults to `before` (unchanged). */
function layer(
  id: string,
  name: string,
  before: string | null,
  after: string | null = before,
  { opacity = 100, blendMode = "normal", change = "unchanged" }: LayerOpts = {}
): ArtLayer {
  return { id, name, opacity, blendMode, change, before, after };
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

// --- scene: hero character (320×400) ----------------------------------------

const HERO_W = 320;
const HERO_H = 400;

const heroSkin = `
  <ellipse cx="160" cy="150" rx="78" ry="92" fill="#e6c2a0"/>
  <path d="M96 226 Q160 270 224 226 L240 360 Q160 392 80 360 Z" fill="#e6c2a0"/>`;

const heroShadow = `
  <path d="M160 70 Q120 90 110 170 Q120 235 160 250 L160 70 Z" fill="#5b4636" opacity="0.55"/>
  <ellipse cx="200" cy="170" rx="26" ry="40" fill="#5b4636" opacity="0.30"/>`;

// Hair highlights — modified in c6: subdued (before) → brighter + rim light (after).
const heroHairBefore = `
  <path d="M88 150 Q92 56 160 50 Q228 56 232 150 Q210 100 160 96 Q110 100 88 150 Z" fill="#3a2a1e"/>
  <path d="M120 92 Q150 70 180 92" stroke="#7a5a3a" stroke-width="6" fill="none" stroke-linecap="round"/>`;
const heroHairAfter = `
  <path d="M88 150 Q92 56 160 50 Q228 56 232 150 Q210 100 160 96 Q110 100 88 150 Z" fill="#3a2a1e"/>
  <path d="M118 90 Q150 64 184 90" stroke="#d8a86a" stroke-width="8" fill="none" stroke-linecap="round"/>
  <path d="M104 130 Q120 96 150 92" stroke="#f0d29a" stroke-width="5" fill="none" stroke-linecap="round"/>
  <path d="M214 132 Q200 96 176 92" stroke="#f0d29a" stroke-width="5" fill="none" stroke-linecap="round"/>`;

const heroLineArt = `
  <ellipse cx="160" cy="150" rx="78" ry="92" fill="none" stroke="#23150c" stroke-width="3"/>
  <circle cx="134" cy="150" r="6" fill="#23150c"/>
  <circle cx="186" cy="150" r="6" fill="#23150c"/>
  <path d="M140 196 Q160 208 180 196" stroke="#23150c" stroke-width="3" fill="none" stroke-linecap="round"/>`;

const heroSketch = `
  <ellipse cx="160" cy="150" rx="78" ry="92" fill="none" stroke="#8a8580" stroke-width="2" stroke-dasharray="5 4"/>
  <path d="M96 226 Q160 270 224 226" stroke="#8a8580" stroke-width="2" fill="none" stroke-dasharray="5 4"/>`;

const heroFlat = `
  <ellipse cx="160" cy="150" rx="78" ry="92" fill="#e6c2a0"/>
  <path d="M96 226 Q160 270 224 226 L240 360 Q160 392 80 360 Z" fill="#c98f63"/>`;

// --- scene: forest background (400×300) -------------------------------------

const FOREST_W = 400;
const FOREST_H = 300;

const forestSky = `
  <defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1c3a4a"/><stop offset="1" stop-color="#dba36b"/>
  </linearGradient></defs>
  <rect width="400" height="300" fill="url(#sky)"/>`;

const forestGradientMap = `
  <defs><linearGradient id="gmap" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#7b3fa0"/><stop offset="1" stop-color="#e0763a"/>
  </linearGradient></defs>
  <rect width="400" height="300" fill="url(#gmap)"/>`;

const forestHaze = `
  <ellipse cx="200" cy="220" rx="260" ry="60" fill="#e8e0d0"/>
  <ellipse cx="120" cy="200" rx="120" ry="34" fill="#e8e0d0"/>`;

const forestTrees = `
  <path d="M40 300 L40 150 L24 150 L60 90 L48 90 L80 40 L112 90 L100 90 L136 150 L120 150 L120 300 Z" fill="#0f1a14"/>
  <path d="M260 300 L260 170 L246 170 L286 100 L274 100 L312 50 L350 100 L338 100 L378 170 L364 170 L364 300 Z" fill="#0f1a14"/>
  <rect x="160" y="210" width="60" height="90" fill="#13211a"/>`;

// --- scene: sword prop (200×360, all-new file) ------------------------------

const SWORD_W = 200;
const SWORD_H = 360;

const swordFlat = `
  <polygon points="100,20 116,140 84,140" fill="#c8ccd2"/>
  <rect x="60" y="146" width="80" height="14" fill="#8a6a3a"/>
  <rect x="92" y="160" width="16" height="60" fill="#5a432a"/>
  <circle cx="100" cy="232" r="14" fill="#caa14a"/>`;
const swordBladeLine = `
  <polygon points="100,20 116,140 84,140" fill="none" stroke="#1a1c20" stroke-width="3"/>
  <line x1="100" y1="34" x2="100" y2="138" stroke="#1a1c20" stroke-width="2"/>`;
const swordHiltLine = `
  <rect x="60" y="146" width="80" height="14" fill="none" stroke="#1a1c20" stroke-width="3"/>
  <rect x="92" y="160" width="16" height="60" fill="none" stroke="#1a1c20" stroke-width="3"/>
  <circle cx="100" cy="232" r="14" fill="none" stroke="#1a1c20" stroke-width="3"/>`;

// --- scene: villain (320×400, merge conflict on Cape) -----------------------

const villainBody = `
  <ellipse cx="160" cy="150" rx="72" ry="86" fill="#cdbfc0"/>
  <path d="M100 224 Q160 264 220 224 L236 360 Q160 392 84 360 Z" fill="#b7a8aa"/>`;
// Cape — conflicted: opaque red normal (HEAD) vs darker multiply (incoming).
const villainCapeBefore = `
  <path d="M96 210 Q60 320 100 392 L160 360 L220 392 Q260 320 224 210 Q160 250 96 210 Z" fill="#9b2230"/>`;
const villainCapeAfter = `
  <path d="M96 210 Q60 320 100 392 L160 360 L220 392 Q260 320 224 210 Q160 250 96 210 Z" fill="#6e1f2a"/>`;
const villainLine = `
  <ellipse cx="160" cy="150" rx="72" ry="86" fill="none" stroke="#1a1014" stroke-width="3"/>
  <path d="M132 150 L148 150 M172 150 L188 150" stroke="#1a1014" stroke-width="4" stroke-linecap="round"/>`;

// --- assembled scenes -------------------------------------------------------

interface Scene {
  width: number;
  height: number;
  layers: ArtLayer[];
  regions: ArtDiff["regions"];
}

/** Build an ArtDiff from a scene + the file's path/status. */
function art(path: string, status: FileStatus, scene: Scene): ArtDiff {
  return { kind: "art", path, status, ...scene };
}

// c6: hero.kra — Hair Highlights modified, Hair Rim Light added.
const heroC6: Scene = {
  width: HERO_W,
  height: HERO_H,
  layers: [
    layer("skin", "Skin Base", heroSkin, heroSkin, { opacity: 100 }),
    layer("shadows", "Shadows", heroShadow, heroShadow, { opacity: 72, blendMode: "multiply" }),
    layer("hair-hl", "Hair Highlights", heroHairBefore, heroHairAfter, {
      opacity: 68,
      blendMode: "screen",
      change: "modified",
    }),
    layer("rim", "Hair Rim Light", null, heroHairAfter, {
      opacity: 40,
      blendMode: "add",
      change: "added",
    }),
    layer("line", "Line Art", heroLineArt, heroLineArt, { opacity: 100 }),
  ],
  regions: [{ x: 0.25, y: 0.1, w: 0.5, h: 0.28, label: "Hair Highlights" }],
};

// c5: forest-bg.kra — Gradient Map + Atmospheric Haze added.
const forestC5: Scene = {
  width: FOREST_W,
  height: FOREST_H,
  layers: [
    layer("sky", "Sky Gradient", forestSky, forestSky, { opacity: 100 }),
    layer("gmap", "Gradient Map", null, forestGradientMap, {
      opacity: 85,
      blendMode: "overlay",
      change: "added",
    }),
    layer("haze", "Atmospheric Haze", null, forestHaze, {
      opacity: 30,
      blendMode: "screen",
      change: "added",
    }),
    layer("trees", "Tree Silhouettes", forestTrees, forestTrees, { opacity: 100 }),
  ],
  regions: [
    { x: 0, y: 0, w: 1, h: 1, label: "Gradient Map" },
    { x: 0.0, y: 0.5, w: 1, h: 0.4, label: "Haze" },
  ],
};

// c4: sword.kra — brand new file, all layers added.
const swordC4: Scene = {
  width: SWORD_W,
  height: SWORD_H,
  layers: [
    layer("flat", "Flat Color", null, swordFlat, { opacity: 100, change: "added" }),
    layer("blade", "Blade Line Art", null, swordBladeLine, { opacity: 100, change: "added" }),
    layer("hilt", "Hilt Line Art", null, swordHiltLine, { opacity: 100, change: "added" }),
  ],
  regions: [{ x: 0.3, y: 0.02, w: 0.4, h: 0.96, label: "New artwork" }],
};

// c3: villain.kra — Cape layer in merge conflict.
const villainC3: Scene = {
  width: HERO_W,
  height: HERO_H,
  layers: [
    layer("body", "Body", villainBody, villainBody, { opacity: 100 }),
    layer("cape", "Cape", villainCapeBefore, villainCapeAfter, {
      opacity: 90,
      blendMode: "multiply",
      change: "modified",
    }),
    layer("vline", "Line Art", villainLine, villainLine, { opacity: 100 }),
  ],
  regions: [{ x: 0.18, y: 0.5, w: 0.64, h: 0.48, label: "Cape (conflict)" }],
};

// c2: hero.kra — Flat Color added over the base Sketch.
const heroC2: Scene = {
  width: HERO_W,
  height: HERO_H,
  layers: [
    layer("sketch", "Sketch", heroSketch, heroSketch, { opacity: 100 }),
    layer("flat", "Flat Color", null, heroFlat, { opacity: 100, change: "added" }),
  ],
  regions: [{ x: 0.2, y: 0.08, w: 0.6, h: 0.86, label: "Flat Color" }],
};

// c1: hero.kra — initial Sketch (new file).
const heroC1: Scene = {
  width: HERO_W,
  height: HERO_H,
  layers: [layer("sketch", "Sketch", null, heroSketch, { opacity: 100, change: "added" })],
  regions: [{ x: 0.2, y: 0.08, w: 0.6, h: 0.86, label: "Sketch" }],
};

/** Pre-built ArtDiff entries keyed by the commit they belong to. */
export const ART_DIFFS = {
  hero_c6: art("characters/hero.kra", "M", heroC6),
  forest_c5: art("scenes/forest-bg.kra", "M", forestC5),
  sword_c4: art("props/sword.kra", "A", swordC4),
  villain_c3: art("characters/villain.kra", "C", villainC3),
  hero_c2: art("characters/hero.kra", "M", heroC2),
  hero_c1: art("characters/hero.kra", "A", heroC1),
};
