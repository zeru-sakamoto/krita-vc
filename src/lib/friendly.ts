// Artist-friendly label helpers. Pure, no side effects. Used when Artist Mode is
// on to turn technical strings (paths, hashes, status codes, palette diffs) into
// plain-language labels. See src/lib/artistMode.tsx.

import { FileImage, Image, Palette, GearSix, type Icon } from "@phosphor-icons/react";
import type { ArtLayer, Commit, DiffLine, FileStatus } from "../types";

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

/** Basename without extension, de-slugged + title-cased. */
export function assetName(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? path;
  const stem = base.replace(/\.[^.]+$/, "");
  return titleCase(stem) || base;
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

export interface PaletteColor {
  name: string;
  color: string;
}
export interface PaletteChange {
  name: string;
  before: string;
  after: string;
}
export interface PaletteDiff {
  changed: PaletteChange[];
  added: PaletteColor[];
  removed: PaletteColor[];
}

const COLOR_LINE = /^(\d{1,3})\s+(\d{1,3})\s+(\d{1,3})\s+(.+)$/;

/** "#RRGGBB" (uppercase), clamped to 0..255 per channel. */
export function rgbToHex(r: number, g: number, b: number): string {
  const hex = (n: number) =>
    Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0").toUpperCase();
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

function parseColorLine(text: string): PaletteColor | null {
  const m = COLOR_LINE.exec(text.trim());
  if (!m) return null;
  const [, r, g, b, name] = m;
  return { name: name.trim(), color: rgbToHex(Number(r), Number(g), Number(b)) };
}

/**
 * Turns a palette (.gpl) text diff into grouped color changes. A removed +
 * added color with the same name is treated as a single "changed" entry.
 */
export function parsePaletteDiff(lines: DiffLine[]): PaletteDiff {
  const added: PaletteColor[] = [];
  const removed: PaletteColor[] = [];
  for (const line of lines) {
    if (line.kind !== "add" && line.kind !== "del") continue;
    const color = parseColorLine(line.text);
    if (!color) continue;
    (line.kind === "add" ? added : removed).push(color);
  }

  const changed: PaletteChange[] = [];
  for (let i = removed.length - 1; i >= 0; i--) {
    const match = added.findIndex((a) => a.name === removed[i].name);
    if (match !== -1) {
      changed.unshift({
        name: removed[i].name,
        before: removed[i].color,
        after: added[match].color,
      });
      added.splice(match, 1);
      removed.splice(i, 1);
    }
  }

  return { changed, added, removed };
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
