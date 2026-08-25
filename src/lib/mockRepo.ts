// Dev-only fixture so the UI can be worked on in a plain browser (`npm run dev`), where there
// is no Tauri backend at all. Deliberately OFF by default and stripped from production builds:
// the project's rule is that browser mode shows real empty states, not fake history. Turn it on
// for a session by loading http://localhost:1420/?mock — nothing else reaches it.
//
// ponytail: hand-written fixture, not a generator. It exists to look at layout, so it only needs
// to be plausible; if a case needs real data, run the desktop shell.

import type { ArtDiff, ArtLayer, Branch, Commit, FileStatus } from "../types";

export function mockEnabled(): boolean {
  if (!import.meta.env.DEV || typeof location === "undefined") return false;
  return new URLSearchParams(location.search).has("mock");
}

const AUTHOR = "Zeru Sakamoto";
const HOUR = 3600_000;

/** A flat-color composite so a node has something recognizable in it. */
function composite(bg: string, ink: string, seed: number): string {
  const cx = 200 + (seed % 5) * 30;
  const cy = 240 + ((seed * 7) % 5) * 24;
  return [
    `<rect width="640" height="800" fill="${bg}"/>`,
    `<circle cx="${cx}" cy="${cy}" r="${120 + (seed % 4) * 18}" fill="${ink}" opacity="0.85"/>`,
    `<rect x="${90 + seed * 11}" y="470" width="${300 + (seed % 3) * 60}" height="210" rx="24" fill="${ink}" opacity="0.5"/>`,
    `<path d="M60 ${700 - seed * 9} Q 320 ${590 - seed * 12} 580 ${690 - seed * 6}" stroke="${ink}" stroke-width="14" fill="none" opacity="0.7"/>`,
  ].join("");
}

const PALETTES: [string, string][] = [
  ["#2b2118", "#e8a35c"],
  ["#1c2430", "#7fb4e0"],
  ["#241c2b", "#c08ce0"],
  ["#1e2b22", "#7fd6a0"],
  ["#2b1c1c", "#e08080"],
];

const LAYER_POOL: { name: string; layerType: string }[] = [
  { name: "Lineart", layerType: "paintlayer" },
  { name: "Flats", layerType: "paintlayer" },
  { name: "Shadows", layerType: "paintlayer" },
  { name: "Highlights", layerType: "paintlayer" },
  { name: "Background", layerType: "grouplayer" },
  { name: "Color balance", layerType: "filterlayer" },
  { name: "Speech bubbles", layerType: "vectorlayer" },
  { name: "Hair mask", layerType: "transparencymask" },
  { name: "Rim light", layerType: "paintlayer" },
];

const CHANGES: ArtLayer["change"][] = ["modified", "added", "modified", "removed", "modified"];

const MESSAGES = [
  "First rough sketch",
  "Blocked in the silhouette",
  "Cleaned up the lineart",
  "Flat colours down",
  "First pass at shadows",
  "Warmed up the palette",
  "Reworked the hair",
  "Added rim light",
  "Background group + gradient",
  "Fixed the hand, again",
  "Speech bubbles for panel 2",
  "Final colour balance",
];

export const MOCK_PATH = "characters/hero.kra";

export function mockCommits(): Commit[] {
  const now = Date.now();
  // Oldest first while building (so parents are easy), reversed to newest-first at the end —
  // the shape `useCommits` returns.
  const out: Commit[] = MESSAGES.map((message, i) => ({
    id: `mock-${i}`,
    hash: `c0ffee${i.toString(16).padStart(2, "0")}`,
    message,
    author: AUTHOR,
    timestamp: new Date(now - (MESSAGES.length - i) * 5 * HOUR).toISOString(),
    parents: i === 0 ? [] : [`mock-${i - 1}`],
    branch: "main",
    changes: [
      { path: MOCK_PATH, status: (i === 0 ? "A" : "M") as FileStatus },
      ...(i % 4 === 3 ? [{ path: "palettes/skin-tones.gpl", status: "M" as FileStatus }] : []),
    ],
  }));
  return out.reverse();
}

export function mockBranches(): Branch[] {
  return [{ name: "main", kind: "current", tip: "mock-" + (MESSAGES.length - 1) }];
}

/** The `commit_diff` payload for one mock commit: one art entry, composite + changed layers. */
export function mockDiff(commitId: string): ArtDiff[] {
  const i = Number(commitId.replace("mock-", "")) || 0;
  const [bg, ink] = PALETTES[i % PALETTES.length];
  const count = 2 + (i % 4);
  const layers: ArtLayer[] = Array.from({ length: count }, (_, k) => {
    const spec = LAYER_POOL[(i * 3 + k) % LAYER_POOL.length];
    return {
      id: `mock-${i}-l${k}`,
      name: spec.name,
      layerType: spec.layerType,
      opacity: 100,
      blendMode: "normal",
      change: i === 0 ? "added" : CHANGES[(i + k) % CHANGES.length],
      visible: true,
      before: null,
      after: null,
    };
  });
  return [
    {
      kind: "art",
      path: MOCK_PATH,
      status: i === 0 ? "A" : "M",
      width: 640,
      height: 800,
      layers,
      regions: [],
      afterImage: composite(bg, ink, i),
      beforeImage: null,
    },
  ];
}
