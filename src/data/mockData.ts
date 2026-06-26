// =============================================================================
// === MOCK DATA — replace when the Rust/Tauri backend is wired up.          ===
// === Nothing here talks to git or the filesystem; it exists purely so the  ===
// === UI can be built and reviewed against DESIGN.md.                       ===
// =============================================================================

import type { Branch, Commit, DiffEntry, Repository, WorkingChange } from "../types";
import { ART_DIFFS } from "./mockArt";
import { PALETTE_DIFFS } from "./mockPalette";

// Local repositories the user has designated. There is no native folder picker
// yet (no Tauri dialog plugin), so these paths are illustrative; the selected
// repo persists, and per-repo data (commits/branches/diffs below) will be fetched
// per repository once the backend lands. For now every repo shows the same data.
export const MOCK_REPOSITORIES: Repository[] = [
  { id: "repo-hero", name: "hero-illustration", path: "C:/Art/hero-illustration" },
  { id: "repo-forest", name: "forest-scene-study", path: "C:/Art/forest-scene-study" },
  { id: "repo-props", name: "prop-asset-pack", path: "D:/Commissions/prop-asset-pack" },
];

export const MOCK_BRANCHES: Branch[] = [
  { name: "main", kind: "current" },
  { name: "character-redesign", kind: "local" },
  { name: "color-experiments", kind: "local" },
];

// History is a small DAG (newest first): `main` with a `character-redesign`
// branch that diverges off the color-flats commit and merges back in.
// `parents` carries the lineage the history graph draws; `branch` colors lanes.
export const MOCK_COMMITS: Commit[] = [
  {
    id: "c6",
    hash: "a1b2c3d",
    message: "Repaint hair highlights on character layer group",
    author: "Zeru Sakamoto",
    timestamp: "2026-06-26T14:32:00",
    branch: "main",
    parents: ["c5"],
    changes: [
      { path: "characters/hero.kra", status: "M" },
      { path: "palettes/skin-tones.gpl", status: "M" },
    ],
  },
  {
    id: "c5",
    hash: "9f8e7d6",
    message: "Merge character-redesign into main",
    author: "Zeru Sakamoto",
    timestamp: "2026-06-25T19:05:00",
    branch: "main",
    parents: ["c4", "cr2"],
    changes: [
      { path: "characters/hero.kra", status: "M" },
      { path: "characters/villain.kra", status: "C" },
    ],
  },
  {
    id: "c4",
    hash: "3c4b5a6",
    message: "Add background gradient map and atmospheric haze layer",
    author: "Zeru Sakamoto",
    timestamp: "2026-06-24T18:02:00",
    branch: "main",
    parents: ["c3"],
    changes: [
      { path: "scenes/forest-bg.kra", status: "M" },
      { path: "references/mood-board.png", status: "A" },
    ],
  },
  {
    id: "cr2",
    hash: "b7c6d5e",
    message: "Repaint face shading on the redesigned hero",
    author: "Mika Tanaka",
    timestamp: "2026-06-24T11:48:00",
    branch: "character-redesign",
    parents: ["cr1"],
    changes: [{ path: "characters/hero.kra", status: "M" }],
  },
  {
    id: "c3",
    hash: "7e8f9a0",
    message: "Block in line art for sword and shield props",
    author: "Mika Tanaka",
    timestamp: "2026-06-23T16:20:00",
    branch: "main",
    parents: ["c2"],
    changes: [
      { path: "props/sword.kra", status: "A" },
      { path: "props/shield.kra", status: "A" },
      { path: "props/old-dagger.kra", status: "D" },
    ],
  },
  {
    id: "cr1",
    hash: "4f5e6d7",
    message: "Explore alternate hairstyle silhouette",
    author: "Mika Tanaka",
    timestamp: "2026-06-22T15:30:00",
    branch: "character-redesign",
    parents: ["c2"],
    changes: [{ path: "characters/hero.kra", status: "M" }],
  },
  {
    id: "c2",
    hash: "1d2e3f4",
    message: "Initial color flats pass over base sketch",
    author: "Zeru Sakamoto",
    timestamp: "2026-06-22T09:14:00",
    branch: "main",
    parents: ["c1"],
    changes: [{ path: "characters/hero.kra", status: "M" }],
  },
  {
    id: "c1",
    hash: "0a9b8c7",
    message: "Initial commit: project structure and base sketch",
    author: "Zeru Sakamoto",
    timestamp: "2026-06-21T20:41:00",
    branch: "main",
    parents: [],
    changes: [
      { path: "characters/hero.kra", status: "A" },
      { path: ".kritavc/config.toml", status: "A" },
    ],
  },
];

// Per-commit diffs. Art (.kra) files render as visual layer diffs (see mockArt.ts);
// non-art files (palettes, config) keep a code-style text diff.
export const MOCK_DIFF_BY_COMMIT: Record<string, DiffEntry[]> = {
  c6: [ART_DIFFS.hero_c6, PALETTE_DIFFS.skin_tones_c6],
  c5: [ART_DIFFS.villain_c3],
  c4: [ART_DIFFS.forest_c5],
  cr2: [ART_DIFFS.hero_c6],
  c3: [ART_DIFFS.sword_c4],
  cr1: [ART_DIFFS.hero_c2],
  c2: [ART_DIFFS.hero_c2],
  c1: [
    ART_DIFFS.hero_c1,
    {
      kind: "text",
      path: ".kritavc/config.toml",
      status: "A",
      lines: [
        { kind: "hunk", text: "@@ new file @@" },
        { kind: "add", newLine: 1, text: "[repository]" },
        { kind: "add", newLine: 2, text: 'name = "krita-vc"' },
        { kind: "add", newLine: 3, text: "lfs = true" },
      ],
    },
  ],
};

// Working-tree changes for the Changes tab (no real git — purely illustrative).
export const MOCK_WORKING_CHANGES: WorkingChange[] = [
  { change: { path: "characters/hero.kra", status: "M" }, staged: true },
  { change: { path: "palettes/skin-tones.gpl", status: "M" }, staged: true },
  { change: { path: "scenes/forest-bg.kra", status: "M" }, staged: false },
  { change: { path: "props/shield.kra", status: "M" }, staged: false },
  { change: { path: "references/pose-study.png", status: "U" }, staged: false },
];
