// Artist-friendly label helpers. Pure, no side effects. Used when Artist Mode is
// on to turn technical strings (paths, hashes, status codes, palette diffs) into
// plain-language labels. See src/lib/artistMode.tsx.

import {
  BezierCurve,
  CircleHalf,
  Copy,
  File,
  FileImage,
  FolderSimple,
  Funnel,
  GearSix,
  Image,
  PaintBrush,
  Palette,
  Stack,
  type Icon,
} from "@phosphor-icons/react";
import type { ArtLayer, Commit, FileStatus } from "../types";

/** Title-case a slug/word: "skin-tones" → "Skin Tones", "hero" → "Hero". */
function titleCase(input: string): string {
  return input
    .replace(/[-_]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

/** The file's actual name as the artist typed/saved it — just the path stripped down to its
 *  basename, extension and all. No de-slugging or extension-stripping: that guessed at a
 *  "nicer" name and drifted from what's actually on disk. */
export function assetName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/**
 * Friendly label for a palette `DiffEntry`'s path. Handles both standalone palette files and
 * embedded ones (keyed `<kra>::<palette-file>` — takes the palette's own segment). Krita's raw
 * representative filename is `<name>.<NNNN>.<ext>` (e.g. `sun-set.0006.kpl`) — a real extension
 * *and* an internal resource-version segment — so both get stripped, not just the extension.
 */
export function paletteName(path: string): string {
  const own = path.includes("::") ? (path.split("::").pop() ?? path) : path;
  const base = own.split(/[\\/]/).pop() ?? own;
  const stem = base.replace(/\.[^.]+$/, "").replace(/\.\d+$/, "");
  const deslugged = stem.replace(/[-_]+/g, " ").trim();
  return deslugged || base;
}

export interface AssetKind {
  label: string;
  icon: Icon;
}

/** Maps a file extension to a friendly asset category + icon. */
export function assetKind(path: string): AssetKind {
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  switch (ext) {
    case "kra":
      return { label: "Artwork", icon: FileImage };
    case "gpl":
      return { label: "Palette", icon: Palette };
    case "png":
    case "jpg":
    case "jpeg":
    case "webp":
      return { label: "Reference", icon: Image };
    default:
      return { label: "Settings", icon: GearSix };
  }
}

/** Plain-language verb for a file status. */
export function statusVerb(status: FileStatus): string {
  switch (status) {
    case "M":
      return "Updated";
    case "A":
      return "Added";
    case "D":
      return "Deleted";
    case "R":
      return "Renamed";
    case "C":
      return "Needs review";
    case "U":
      return "New";
  }
}

/** Krita layer `nodetype` → a short friendly label. */
export function layerTypeLabel(kind: string): string {
  switch (kind) {
    case "paintlayer":
      return "Paint";
    case "grouplayer":
      return "Group";
    case "filterlayer":
    case "adjustmentlayer":
      return "Filter";
    case "clonelayer":
      return "Clone";
    case "shapelayer":
    case "vectorlayer":
      return "Vector";
    case "filelayer":
      return "File";
    case "transparencymask":
    case "transformmask":
    case "selectionmask":
    case "filtermask":
      return "Mask";
    default:
      return kind ? titleCase(kind.replace(/layer$/, "")) : "Layer";
  }
}

/** Krita layer `nodetype` → an icon. Same cases as `layerTypeLabel`, kept beside it. */
export function layerTypeIcon(kind: string): Icon {
  switch (kind) {
    case "paintlayer":
      return PaintBrush;
    case "grouplayer":
      return FolderSimple;
    case "filterlayer":
    case "adjustmentlayer":
      return Funnel;
    case "clonelayer":
      return Copy;
    case "shapelayer":
    case "vectorlayer":
      return BezierCurve;
    case "filelayer":
      return File;
    case "transparencymask":
    case "transformmask":
    case "selectionmask":
    case "filtermask":
      return CircleHalf;
    default:
      return Stack;
  }
}

/** Plain-language label for a layer's change state. */
export function layerChangeLabel(change: ArtLayer["change"]): string {
  switch (change) {
    case "added":
      return "Added";
    case "removed":
      return "Removed";
    case "modified":
      return "Modified";
    case "unchanged":
      return "Unchanged";
  }
}

/**
 * Krita `compositeop` → a readable name: "svg:multiply" → "Multiply", "color_dodge" →
 * "Color dodge". Krita has dozens of ops; spelling out its own id beats a table that drifts.
 */
export function blendLabel(op: string | undefined): string {
  const bare = (op || "normal").replace(/^svg:/, "").replace(/[_-]+/g, " ");
  return bare.charAt(0).toUpperCase() + bare.slice(1);
}

/** One change on a layer, as a short label for tight spots and a full before → after line. */
export interface LayerChangeText {
  /** "Hidden", "Repainted", "Opacity 40%". Fits a layer row. */
  short: string;
  /** "Shown → Hidden", "Opacity 100% → 40%". For the inspector and tooltips. */
  full: string;
}

/**
 * Label every change on a layer, most important first (the order `changes` arrives in). `layers`
 * is the diff's layer list, used to name the group a layer moved into.
 */
export function layerChangeTexts(layer: ArtLayer, layers: ArtLayer[] = []): LayerChangeText[] {
  const p = layer.prev;
  return (layer.changes ?? []).map((kind): LayerChangeText => {
    switch (kind) {
      case "pixels":
        return { short: "Repainted", full: "Repainted" };
      case "visibility": {
        const now = layer.visible === false ? "Hidden" : "Shown";
        return { short: now, full: `${p?.visible === false ? "Hidden" : "Shown"} → ${now}` };
      }
      case "moved": {
        if (p && p.parentId !== layer.parentId) {
          if (layer.parentId) {
            const to = layers.find((l) => l.id === layer.parentId)?.name;
            return { short: "Moved", full: to ? `Moved into ${to}` : "Moved into a group" };
          }
          return {
            short: "Moved",
            full: p.parentName ? `Moved out of ${p.parentName}` : "Moved out of its group",
          };
        }
        return { short: "Reordered", full: "Moved up or down the stack" };
      }
      case "opacity":
        return {
          short: `Opacity ${layer.opacity}%`,
          full: `Opacity ${p?.opacity ?? "?"}% → ${layer.opacity}%`,
        };
      case "blend":
        return {
          short: blendLabel(layer.blendOp),
          full: `Blend ${blendLabel(p?.blendOp)} → ${blendLabel(layer.blendOp)}`,
        };
      case "renamed":
        return { short: "Renamed", full: `Renamed from ${p?.name ?? "?"}` };
    }
  });
}

/**
 * The Inspector's Change row: what kinds of change a layer has ("Blend", "Repainted"), with no
 * values, since the property rows above it already show those as was → now. Renamed and moved
 * have no property row of their own, so they keep their detail ("Renamed from Sketch").
 */
export function layerChangeKinds(layer: ArtLayer, layers: ArtLayer[] = []): string[] {
  const texts = layerChangeTexts(layer, layers);
  return (layer.changes ?? []).map((kind, i) => {
    switch (kind) {
      case "pixels":
        return "Repainted";
      case "visibility":
        return "Visibility";
      case "opacity":
        return "Opacity";
      case "blend":
        return "Blend";
      case "renamed":
      case "moved":
        return texts[i].full;
    }
  });
}

/**
 * A layer row's change summary: the most important change, and a count for the rest
 * ("Repainted · +1"). Two short ones fit side by side and show in full ("Hidden · Renamed").
 * `title` lists every change in full, for a tooltip. Empty `text` when nothing is listed.
 */
export function layerChangeSummary(
  layer: ArtLayer,
  layers: ArtLayer[] = []
): { text: string; title: string } {
  const texts = layerChangeTexts(layer, layers);
  // One line: the Tooltip doesn't wrap on newlines.
  const title = texts.map((t) => t.full).join(", ");
  if (texts.length === 0) return { text: "", title };
  if (texts.length === 2 && texts[0].short.length + texts[1].short.length <= 18)
    return { text: `${texts[0].short} · ${texts[1].short}`, title };
  const rest = texts.length - 1;
  return { text: rest > 0 ? `${texts[0].short} · +${rest}` : texts[0].short, title };
}

/** Map of commit id → version number, newest commit = highest number. */
export function versionNumbers(commits: Commit[]): Map<string, number> {
  const total = commits.length;
  const map = new Map<string, number>();
  commits.forEach((c, i) => map.set(c.id, total - i));
  return map;
}

/** "Version 6" */
export function versionLabel(n: number): string {
  return `Version ${n}`;
}
