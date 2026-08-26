// Dev-only fixture so the UI can be worked on in a plain browser (`npm run dev`), where there
// is no Tauri backend at all. Deliberately OFF by default and stripped from production builds:
// the project's rule is that browser mode shows real empty states, not fake history. Turn it on
// for a session by loading http://localhost:1420/?mock — nothing else reaches it.
//
// ponytail: hand-written fixture, not a generator. It exists to look at layout, so it only needs
// to be plausible; if a case needs real data, run the desktop shell.

import type { ArtDiff, ArtLayer, Branch, Commit, FileChange, FileStatus } from "../types";

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

/** Where the side line forks off `main`, and where it merges back — indices into MESSAGES. */
const FORK_AT = 4;
const MERGE_AT = 9;
const SIDE = ["Rough in the alt hair", "Recolour the alt hair"];
const SIDE_BRANCH = "hair-experiment";
const MAIN_TIP = `mock-${MESSAGES.length - 1}`;
const SIDE_TIP = `side-${SIDE.length - 1}`;

function mockChanges(i: number): FileChange[] {
  return [
    { path: MOCK_PATH, status: (i === 0 ? "A" : "M") as FileStatus },
    ...(i % 4 === 3 ? [{ path: "palettes/skin-tones.gpl", status: "M" as FileStatus }] : []),
  ];
}

/**
 * A 12-version main line plus a two-version side branch that forks at `FORK_AT` and merges back
 * at `MERGE_AT` — the fixture the Version Map's lane layout is developed against, since the
 * frontend has no test runner. Load `http://localhost:1420/?mock` and turn "show all lines" on.
 */
export function mockCommits(allBranches = false): Commit[] {
  const now = Date.now();
  const at = (i: number) => new Date(now - (MESSAGES.length - i) * 5 * HOUR).toISOString();
  // Oldest first while building (so parents are easy), reversed to newest-first at the end —
  // the shape `useCommits` returns.
  const out: Commit[] = MESSAGES.map((message, i) => ({
    id: `mock-${i}`,
    hash: `c0ffee${i.toString(16).padStart(2, "0")}`,
    message,
    author: AUTHOR,
    timestamp: at(i),
    parents:
      i === 0
        ? []
        : // The merge commit's second parent is the side line's tip.
          i === MERGE_AT && allBranches
          ? [`mock-${i - 1}`, SIDE_TIP]
          : [`mock-${i - 1}`],
    branch: "main",
    changes: mockChanges(i),
  }));
  if (!allBranches) return out.reverse();

  const side: Commit[] = SIDE.map((message, k) => ({
    id: `side-${k}`,
    hash: `5-de-0${k}`,
    message,
    author: AUTHOR,
    timestamp: at(FORK_AT + k + 1),
    parents: [k === 0 ? `mock-${FORK_AT}` : `side-${k - 1}`],
    branch: SIDE_BRANCH,
    changes: mockChanges(FORK_AT + k),
  }));
  // Insert in log order (oldest first) so the reverse below still yields newest-first.
  out.splice(FORK_AT + 1, 0, ...side);
  return out.reverse();
}

export function mockBranches(): Branch[] {
  return [
    { name: "main", kind: "current", tip: MAIN_TIP },
    { name: SIDE_BRANCH, kind: "local", tip: SIDE_TIP },
  ];
}

/** The `commit_diff` payload for one mock commit: one art entry, composite + changed layers. */
export function mockDiff(commitId: string): ArtDiff[] {
  // Side-branch ids (`side-N`) are offset so their art doesn't clone the main line's.
  const n = Number(commitId.replace(/^(mock|side)-/, "")) || 0;
  const i = commitId.startsWith("side-") ? n + MESSAGES.length : n;
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
