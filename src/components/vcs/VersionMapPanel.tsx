import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  MiniMap,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  BaseEdge,
  useReactFlow,
  useStore,
  useStoreApi,
  type BuiltInEdge,
  type Edge,
  type EdgeProps,
  type Node,
  type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  ArrowLeft,
  ArrowUDownLeft,
  ArrowsClockwise,
  ArrowsOut,
  CaretDown,
  GitBranch,
  GitMerge,
  GpsFix,
  MapTrifold,
  Plus,
  SidebarSimple,
  Trash,
} from "@phosphor-icons/react";
import { MainPanel } from "../MainPanel";
import { PanelHeader } from "../shell/DockerPanel";
import { Inspector } from "../shell/Inspector";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { Switch } from "../ui/Switch";
import { Menu } from "../ui/Menu";
import { Tooltip } from "../ui/Tooltip";
import { useBranchActions, type BranchActions } from "./useBranchActions";
import {
  NODE_H,
  NODE_W,
  PreviewNode,
  SPINE_TOP,
  VersionNode,
  type PreviewNodeData,
  type VersionNodeData,
} from "./VersionNode";
import { useCommitDiff, useCommits } from "../../lib/repoData";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";
import { useTour } from "../../lib/tour";
import { assetName, versionLabel } from "../../lib/friendly";
import { ICON } from "../../lib/iconSize";
import {
  bendFraction,
  buildVersionMap,
  wrapVersionMap,
  type WrapCell,
  type WrapEdge,
  type WrappedMap,
} from "../../lib/versionMap";
import type { Branch, Commit } from "../../types";
import { shiftLayout } from "../../lib/layoutShift";

/** Horizontal pitch between node columns (node width + gutter). */
const NODE_PITCH = NODE_W + 72;
/** Vertical pitch between branch lanes — a node's nominal height plus a gutter. */
const LANE_PITCH = NODE_H + 60;
/** How far a connector runs straight off its handle before it may bend. Deliberately small (and
 *  *not* the gutter half-width): `getSmoothStepPath` drops a gap point only when it lands exactly
 *  on the bend x, and float error in the measured handle positions meant it sometimes didn't —
 *  leaving a stray point a hair from the corner, which collapses that corner's radius to ~0. One
 *  bend rounded and the other square. Keeping the gap points well clear of the bend makes both
 *  corners get the full radius; `stepPosition` below still puts the bend in the gutter. */
const STEP_GAP = 20;
const FIT_PADDING = 0.15;
/** Minimap box, and how far React Flow pads its viewBox past the content (`offsetScale * viewScale`
 *  on every side). Our history is much wider than tall, so viewScale is width-driven and the
 *  default 5 becomes a visible gap in the short axis. MinimapViewport re-derives the same
 *  geometry, so these have to be the values the MiniMap itself is given. */
const MINIMAP_W = 180;
const MINIMAP_H = 96;
const MINIMAP_OFFSET_SCALE = 1.5;
/** Whether the map draws every branch or just the one you're on. Map-local, so it's a plain
 *  localStorage read rather than another app-wide context (cf. artistMode/legacyHistory). */
const SHOW_ALL_KEY = "krita-vc:map-show-all";
/** Whether a long line wraps into stacked bands (see `wrapVersionMap`). Map-local, like the above. */
const WRAP_KEY = "krita-vc:map-wrap";
/** Extra space between two wrapped bands, on top of the normal lane gutter — room for the return
 *  line, and enough that a band reads as a new row rather than another branch lane. */
const BAND_GAP = 80;
/** Most bands the auto-wrap will try. Past this the nodes are big enough anyway. */
const MAX_BANDS = 12;

// Version Map's own lane palette — separate from graph.ts's LANE_COLORS (whose positional
// lane-0-is-accent is a fixed convention for the legacy History graph). Reuses theme-tuned status
// tokens, so no new CSS tokens are needed. Lane 0 is the trunk and divergent branches take 1.. in
// order of first appearance (see lib/versionMap.ts) — so lane index no longer says where you're
// standing, and the accent is spent saying it instead: whichever lane you're on gets it, the rest
// cycle this palette. Same idea as graph.ts's `branchColorMap`.
const BRANCH_LANE_COLORS = [
  "var(--color-info-fg)",
  "var(--color-success-fg)",
  "var(--color-warning-fg)",
];
function laneColor(lane: number, currentLane: number): string {
  return lane === currentLane
    ? "var(--color-accent)"
    : BRANCH_LANE_COLORS[lane % BRANCH_LANE_COLORS.length];
}
/** A lane's color at line weight: dimmer than its dots, but **opaque**. Mixing toward
 *  `transparent` instead let two crossing lines composite into a brighter, two-tone band that
 *  read as a misaligned double line — mixing toward the background looks the same and can't. */
function dimLane(color: string): string {
  return `color-mix(in srgb, ${color} 55%, var(--color-bg))`;
}
/** Top edge of a row, in flow coordinates. With wrapping off there's one band and the row is the
 *  lane's index among the lanes drawn. */
function rowTop(row: number, band: number): number {
  return row * LANE_PITCH + band * BAND_GAP;
}
/** A lane-crossing connector's gradient: which two lanes' colors, between which two spine y's.
 *  Keyed by the y's as well as the lanes because the same lane pair sits at different heights in
 *  different bands. */
interface LaneLink {
  id: string;
  from: number;
  to: number;
  y1: number;
  y2: number;
}
function laneLink(from: number, to: number, y1: number, y2: number): LaneLink {
  return { id: `kvc-lane-${from}-${to}-${Math.round(y1)}-${Math.round(y2)}`, from, to, y1, y2 };
}

// Registered once at module scope — a fresh object here would remount every node each render.
const NODE_TYPES = { version: VersionNode, preview: PreviewNode };

/** A wrap connector's route: which gutter track it runs along between the two bands. */
interface WrapEdgeData extends Record<string, unknown> {
  /** y of the horizontal run in the gap below the source's band. */
  trackY: number;
  /** Which track in that gap (0 = first). Later tracks also step their vertical runs outward,
   *  so two lines wrapping through one gap cross once instead of running on top of each other. */
  track: number;
}

/**
 * The line carrying history from the end of one band to the start of the next: out of the source
 * dot to the middle of the gutter on its right, down into the gap below its band, back left along
 * the gap to the gutter before the target, down to the target's spine and in. Like a carriage
 * return, so time keeps reading left→right on every band.
 */
function WrapConnector({
  sourceX,
  sourceY,
  targetX,
  targetY,
  data,
  style,
}: EdgeProps<Edge<WrapEdgeData>>) {
  const step = NODE_PITCH / 2 + (data?.track ?? 0) * 10;
  const trackY = data?.trackY ?? (sourceY + targetY) / 2;
  const path = roundedPath(
    [
      [sourceX, sourceY],
      [sourceX + step, sourceY],
      [sourceX + step, trackY],
      [targetX - step, trackY],
      [targetX - step, targetY],
      [targetX, targetY],
    ],
    16
  );
  return <BaseEdge path={path} style={style} />;
}

const EDGE_TYPES = { wrap: WrapConnector };
/** Ordinary connectors are React Flow's smoothstep; only a line wrapping between bands is ours. */
type MapEdge = BuiltInEdge | Edge<WrapEdgeData, "wrap">;

/** An orthogonal polyline as an SVG path with every corner rounded to `r` (less where a run is
 *  too short to fit it). Same quadratic-corner shape `getSmoothStepPath` draws, so wrap lines and
 *  ordinary connectors bend alike. */
function roundedPath(pts: [number, number][], r: number): string {
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const [px, py] = pts[i - 1];
    const [cx, cy] = pts[i];
    const [nx, ny] = pts[i + 1];
    const inLen = Math.hypot(cx - px, cy - py);
    const outLen = Math.hypot(nx - cx, ny - cy);
    const k = Math.min(r, inLen / 2, outLen / 2);
    if (k <= 0) {
      d += ` L${cx},${cy}`;
      continue;
    }
    const ax = cx - ((cx - px) / inLen) * k;
    const ay = cy - ((cy - py) / inLen) * k;
    const bx = cx + ((nx - cx) / outLen) * k;
    const by = cy + ((ny - cy) / outLen) * k;
    d += ` L${ax},${ay} Q${cx},${cy} ${bx},${by}`;
  }
  const [lx, ly] = pts[pts.length - 1];
  return `${d} L${lx},${ly}`;
}

/**
 * How many columns a band should be for this canvas: tries 1..MAX_BANDS bands and keeps the one
 * whose fit-to-screen zoom is largest — i.e. the biggest nodes once the whole history is on
 * screen. Prefers fewer bands when the gain is marginal, and never wraps a history that already
 * fits at 100%. Returns 0 for "don't wrap".
 */
function chooseWrapCols(
  cells: WrapCell[],
  edges: WrapEdge[],
  view: { w: number; h: number }
): number {
  if (cells.length === 0 || view.w <= 0 || view.h <= 0) return 0;
  const zoomFor = (m: WrappedMap) => {
    const w = m.widest * NODE_PITCH - (NODE_PITCH - NODE_W);
    const h = m.rows * LANE_PITCH - (LANE_PITCH - NODE_H) + (m.bands.length - 1) * BAND_GAP;
    return Math.min(1, view.w / w, view.h / h);
  };
  let minCol = Infinity;
  let maxCol = -Infinity;
  for (const c of cells) {
    minCol = Math.min(minCol, c.col);
    maxCol = Math.max(maxCol, c.col);
  }
  const total = maxCol - minCol + 1;
  const tries: { cols: number; zoom: number }[] = [
    { cols: 0, zoom: zoomFor(wrapVersionMap(cells, edges, 0)) },
  ];
  for (let k = 2; k <= Math.min(MAX_BANDS, total); k++) {
    const cols = Math.ceil(total / k);
    tries.push({ cols, zoom: zoomFor(wrapVersionMap(cells, edges, cols)) });
  }
  const best = Math.max(...tries.map((t) => t.zoom));
  // First (fewest bands) within 5% of the best: an extra band that barely helps isn't worth it.
  return tries.find((t) => t.zoom >= best * 0.95)?.cols ?? 0;
}

/** Node id of the pending-changes preview. Can never collide with a commit id. */
const PREVIEW_ID = "__preview__";

interface VersionMapPanelProps {
  repoPath: string;
  /** The current branch's commits (newest first) — what the map draws with "show all" off. */
  commits: Commit[];
  branches: Branch[];
  currentBranch: Branch;
  /** Bumped by any mutating repo op — forwarded to the drilldown diff. */
  nonce?: number;
  /** The working tree has uncommitted changes — draws the pending-version preview node at the
   *  end of the current branch. From the shell's one `scan_repository`. */
  dirty?: boolean;
  /** Switches the shell to the Changes tab — wired to the empty-state's call to action and to
   *  the preview node. */
  onShowChanges: () => void;
}

/**
 * The Version Map: this branch's line of versions on a pannable, zoomable canvas, laid out
 * left→right oldest first, each node carrying its own artwork instead of a hash and a message.
 * The default view and the visual replacement for the History graph; there's no inspector, so
 * the node itself is the metadata and clicking one opens the full diff viewer in place.
 *
 * Built on React Flow rather than a hand-rolled transform: the branch phase needs edge routing,
 * a minimap and fit-view over a real graph, which is most of what React Flow is. Node positions
 * are always **computed** from the commit graph (`nodesDraggable={false}`) — history is not a
 * mood board, so there's nothing to persist and a new commit can never leave the layout stale.
 *
 * By default only the current branch is drawn, which is free: `list_commits` is already scoped to
 * commits reachable from the current branch tip. "Show all lines" opts into the wider
 * `allBranches` scope and lays divergent branches out on their own lanes — see
 * `lib/versionMap.ts` for the lane/column rules.
 */
export function VersionMapPanel(props: VersionMapPanelProps) {
  // The provider has to sit above anything calling useReactFlow/useStore, including the header.
  return (
    <ReactFlowProvider>
      <VersionMap {...props} />
    </ReactFlowProvider>
  );
}

function VersionMap({
  repoPath,
  commits,
  branches,
  currentBranch,
  nonce = 0,
  dirty = false,
  onShowChanges,
}: VersionMapPanelProps) {
  const { artistMode } = useArtistMode();
  const { refresh, scanning } = useRepository();
  const [openId, setOpenId] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(() => localStorage.getItem(SHOW_ALL_KEY) === "1");
  // Pick-a-version mode: the action bar asked for a starting point, so the next node click
  // forks a branch there instead of opening the diff viewer.
  const [picking, setPicking] = useState(false);
  const [wrap, setWrap] = useState(() => localStorage.getItem(WRAP_KEY) !== "0");
  // The canvas size the wrap was chosen for. Deliberately a snapshot, not live: re-choosing on
  // every resize would reflow the whole map under the cursor while you drag a window edge. It's
  // taken once the canvas is first measured and again whenever "Fit all versions" is pressed.
  const [wrapView, setWrapView] = useState<{ w: number; h: number } | null>(null);
  const [fitNonce, setFitNonce] = useState(0);
  const storeApi = useStoreApi();
  const measured = useStore((s) => s.width > 0 && s.height > 0);
  const { fitView, setCenter, getViewport } = useReactFlow();
  const actions = useBranchActions({ currentBranch: currentBranch.name, onShowChanges });
  const { active: tourActive } = useTour();

  // A drilldown replaces the canvas outright, taking every map spotlight target with it. The tour
  // auto-fires on a fresh install where nothing is open, but "Replay tour" from Settings can fire
  // with a version open — so hand the canvas back before the map steps run.
  useEffect(() => {
    if (tourActive) setOpenId(null);
  }, [tourActive]);

  // Only fetched while the toggle is on: an empty path makes `useCommits` a no-op, so the off
  // state costs nothing and stays byte-identical to the commits the shell already loaded.
  const allCommits = useCommits(showAll ? repoPath : "", nonce, true);
  // Falling back to `commits` while the wider fetch is in flight, rather than to `[]` — the
  // all-branches set is a superset, so this just holds the current line on screen for a frame
  // instead of flashing the "no versions yet" empty state.
  const drawn = showAll && allCommits.length > 0 ? allCommits : commits;

  const layout = useMemo(
    () => buildVersionMap(drawn, branches, currentBranch.name),
    [drawn, branches, currentBranch.name]
  );
  const otherBranches = branches.length - 1;

  const toggleShowAll = useCallback(() => {
    setShowAll((v) => {
      localStorage.setItem(SHOW_ALL_KEY, v ? "0" : "1");
      return !v;
    });
  }, []);

  // Opening a version unmounts the canvas, which would otherwise come back at the origin — i.e.
  // scrolled to the oldest version, nowhere near where you were. Stash the viewport on the way
  // out and hand it back as `defaultViewport` on the way in.
  const savedViewport = useRef<Viewport | null>(null);
  const onOpen = useCallback(
    (id: string) => {
      savedViewport.current = getViewport();
      setOpenId(id);
    },
    [getViewport]
  );

  const onPick = useCallback(
    (id: string) => {
      setPicking(false);
      actions.askCreate(id, layout.placed.get(id)?.version);
    },
    [actions, layout]
  );
  // The node calls whatever it was handed as `onOpen` — swapping the callback is the entire
  // pick mode, so VersionNode needs no notion of it.
  const onNodeClick = picking ? onPick : onOpen;

  useEffect(() => {
    if (!picking) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPicking(false);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [picking]);

  // Where an unsaved version would land: one column past the end of the current lane, on that
  // lane. ponytail: takes the lane's *last* column rather than the tip's, because
  // `buildVersionMap`'s collision guard can shift a node right of its generation depth — so the
  // tip isn't reliably the rightmost thing on its own lane. Off by a column at worst, never
  // overlapping. Null when clean, when there's nothing to hang it off, or before history loads.
  const preview = useMemo(() => {
    if (!dirty || drawn.length === 0 || !currentBranch.tip) return null;
    const tip = layout.placed.get(currentBranch.tip);
    if (!tip) return null;
    let col = 0;
    for (const p of layout.placed.values()) {
      if (p.lane === layout.currentLane) col = Math.max(col, p.col);
    }
    return { col: col + 1, version: tip.version + 1 };
  }, [dirty, drawn.length, currentBranch.tip, layout]);

  // --- Wrapping: the (col, lane) grid → bands stacked top to bottom (lib/versionMap.ts). ------
  const wrapInput = useMemo(() => {
    const cells: WrapCell[] = drawn.map((c) => {
      const at = layout.placed.get(c.id);
      return { id: c.id, col: at?.col ?? 1, lane: at?.lane ?? 0, time: Date.parse(c.timestamp) };
    });
    const edges: WrapEdge[] = [];
    for (const c of drawn) {
      const toCol = layout.placed.get(c.id)?.col ?? 1;
      for (const p of c.parents) {
        const from = layout.placed.get(p);
        if (from) edges.push({ fromCol: from.col, toCol });
      }
    }
    if (preview) {
      cells.push({ id: PREVIEW_ID, col: preview.col, lane: layout.currentLane });
      const tip = currentBranch.tip ? layout.placed.get(currentBranch.tip) : undefined;
      if (tip) edges.push({ fromCol: tip.col, toCol: preview.col });
    }
    return { cells, edges };
  }, [drawn, layout, preview, currentBranch.tip]);

  useEffect(() => {
    if (!measured || wrapView) return;
    const { width, height } = storeApi.getState();
    setWrapView({ w: width, h: height });
  }, [measured, wrapView, storeApi]);

  const wrapCols = useMemo(
    () => (wrap && wrapView ? chooseWrapCols(wrapInput.cells, wrapInput.edges, wrapView) : 0),
    [wrap, wrapView, wrapInput]
  );
  const wrapped = useMemo(
    () => wrapVersionMap(wrapInput.cells, wrapInput.edges, wrapCols),
    [wrapInput, wrapCols]
  );
  const posOf = useCallback(
    (id: string) => {
      const w = wrapped.at.get(id);
      return w ? { x: w.x * NODE_PITCH, y: rowTop(w.row, w.band) } : { x: 0, y: 0 };
    },
    [wrapped]
  );

  // Re-choose the wrap for the canvas as it is *now*, then fit once the new layout is in. The
  // timeout lets React Flow take the new node positions into its store before fitView reads them.
  const fitAll = useCallback(() => {
    const { width, height } = storeApi.getState();
    if (width > 0 && height > 0) setWrapView({ w: width, h: height });
    setFitNonce((n) => n + 1);
  }, [storeApi]);
  useEffect(() => {
    if (fitNonce === 0) return;
    const t = setTimeout(() => void fitView({ padding: FIT_PADDING, duration: 350 }), 30);
    return () => clearTimeout(t);
  }, [fitNonce, fitView]);

  const toggleWrap = useCallback(() => {
    setWrap((v) => {
      localStorage.setItem(WRAP_KEY, v ? "0" : "1");
      return !v;
    });
    fitAll();
  }, [fitAll]);

  const nodes = useMemo<Node<VersionNodeData | PreviewNodeData>[]>(
    () =>
      drawn.map((commit) => {
        const at = layout.placed.get(commit.id);
        const lane = at?.lane ?? 0;
        return {
          id: commit.id,
          type: "version",
          position: posOf(commit.id),
          // Both dimensions are required: the MiniMap reads them off this object (see NODE_H).
          width: NODE_W,
          height: NODE_H,
          selected: commit.id === openId,
          data: {
            commit,
            version: at?.version ?? 0,
            isTip: layout.tips.has(commit.id),
            tipOf: layout.tips.get(commit.id) ?? [],
            branch: at?.branch ?? null,
            tourTarget: commit.id === currentBranch.tip,
            repoPath,
            onOpen: onNodeClick,
            laneColor: laneColor(lane, layout.currentLane),
          },
        };
      }),
    [drawn, layout, repoPath, openId, onNodeClick, currentBranch.tip, posOf]
  );

  const allNodes = useMemo<Node<VersionNodeData | PreviewNodeData>[]>(() => {
    if (!preview) return nodes;
    return [
      ...nodes,
      {
        id: PREVIEW_ID,
        type: "preview",
        position: posOf(PREVIEW_ID),
        width: NODE_W,
        height: NODE_H,
        data: {
          version: preview.version,
          branch: currentBranch.name,
          laneColor: laneColor(layout.currentLane, layout.currentLane),
          picking,
          onOpen: onShowChanges,
        },
      },
    ];
  }, [nodes, preview, layout.currentLane, currentBranch.name, picking, onShowChanges, posOf]);

  // Edges come from `parents`, not from adjacency — so a merge commit's second parent draws its
  // own line into whichever lane it came from. Since both handles sit on the connector dot, one
  // edge is the *entire* line between two versions: through the source node, the gutter, and the
  // target node, with nothing else drawing any part of it.
  const { edges, laneLinks } = useMemo(() => {
    const out: MapEdge[] = [];
    const links = new Map<string, LaneLink>(); // lane-crossing connectors needing a gradient
    const spineY = (id: string) => {
      const w = wrapped.at.get(id);
      return w ? rowTop(w.row, w.band) + SPINE_TOP : 0;
    };
    // Each band's gap gets its own tracks, one per line wrapping through it, so two wrap lines
    // through the same gap never draw on top of each other. The trunk's is usually the only one.
    // Tracks go out in source-row order — the trunk (top row) innermost — which is what keeps
    // the nesting to a single clean crossing.
    const trackOf = new Map<string, number>();
    {
      const crossing: { id: string; band: number; row: number }[] = [];
      const consider = (source: string, target: string) => {
        const s = wrapped.at.get(source);
        const t = wrapped.at.get(target);
        if (s && t && s.band !== t.band)
          crossing.push({ id: `${source}->${target}`, band: s.band, row: s.row });
      };
      for (const c of drawn) for (const p of c.parents) consider(p, c.id);
      if (preview && currentBranch.tip) consider(currentBranch.tip, PREVIEW_ID);
      crossing.sort((x, y) => x.row - y.row);
      const used = new Map<number, number>();
      for (const e of crossing) {
        const n = used.get(e.band) ?? 0;
        used.set(e.band, n + 1);
        trackOf.set(e.id, n);
      }
    }
    const nextTrack = (band: number, id: string) => {
      const b = wrapped.bands[band];
      const lastRow = b.rowStart + Math.max(1, b.lanes.length) - 1;
      const track = trackOf.get(id) ?? 0;
      const trackY =
        rowTop(lastRow, band) + NODE_H + (LANE_PITCH - NODE_H + BAND_GAP) / 2 + track * 10;
      return { trackY, track };
    };
    /** A line from one band into a later one, drawn as a carriage return through the gap. */
    const wrapEdge = (
      id: string,
      source: string,
      target: string,
      fromLane: number,
      toLane: number,
      dashed: boolean
    ): MapEdge | null => {
      const s = wrapped.at.get(source);
      const t = wrapped.at.get(target);
      if (!s || !t || s.band === t.band) return null;
      let stroke = dimLane(laneColor(toLane, layout.currentLane));
      if (fromLane !== toLane) {
        const link = laneLink(fromLane, toLane, spineY(source), spineY(target));
        links.set(link.id, link);
        stroke = `url(#${link.id})`;
      }
      return {
        id,
        source,
        target,
        type: "wrap",
        data: nextTrack(s.band, id),
        style: { stroke, strokeWidth: 1.5, ...(dashed ? { strokeDasharray: "5 4" } : {}) },
      };
    };
    for (const c of drawn) {
      const at = layout.placed.get(c.id);
      for (const p of c.parents) {
        const from = layout.placed.get(p);
        if (!from) continue;
        const toLane = at?.lane ?? 0;
        const across = wrapEdge(`${p}->${c.id}`, p, c.id, from.lane, toLane, false);
        if (across) {
          out.push(across);
          continue;
        }
        const crosses = from.lane !== toLane;
        let gradient = "";
        if (crosses) {
          const link = laneLink(from.lane, toLane, spineY(p), spineY(c.id));
          links.set(link.id, link);
          gradient = `url(#${link.id})`;
        }
        out.push({
          id: `${p}->${c.id}`,
          source: p,
          target: c.id,
          type: "smoothstep",
          // Where the step bends. `stepPosition` is a fraction of the run between the two gap
          // points, so solve it for the descent x we actually want: the middle of the gutter
          // *next to the end on the shallower lane* — a branch drops out of the spine at the
          // version it started from and climbs back in at the version it merges into, clear of
          // the node's caption and chips, and never runs alongside the spine (which doubles it).
          pathOptions: {
            offset: STEP_GAP,
            stepPosition: bendFraction(
              ((at?.col ?? 0) - from.col) * NODE_PITCH,
              toLane > from.lane,
              NODE_PITCH,
              STEP_GAP
            ),
            borderRadius: 16,
          },
          style: {
            stroke: crosses ? gradient : dimLane(laneColor(toLane, layout.currentLane)),
            strokeWidth: 1.5,
          },
        });
      }
    }
    // The dashed run out to the pending-changes preview. Same lane as its parent, so it needs no
    // bend and no lane gradient — just the spine's color, dashed.
    if (preview && currentBranch.tip && layout.placed.has(currentBranch.tip)) {
      const lane = layout.currentLane;
      const tipLane = layout.placed.get(currentBranch.tip)?.lane ?? lane;
      const across = wrapEdge(
        `${currentBranch.tip}->${PREVIEW_ID}`,
        currentBranch.tip,
        PREVIEW_ID,
        tipLane,
        lane,
        true
      );
      if (across) out.push(across);
      else
        out.push({
          id: `${currentBranch.tip}->${PREVIEW_ID}`,
          source: currentBranch.tip,
          target: PREVIEW_ID,
          type: "smoothstep",
          style: {
            stroke: dimLane(laneColor(layout.currentLane, layout.currentLane)),
            strokeWidth: 1.5,
            strokeDasharray: "5 4",
          },
        });
    }
    return { edges: out, laneLinks: [...links.values()] };
  }, [drawn, layout, preview, currentBranch.tip, wrapped]);

  // The current branch's tip, which with lanes on is no longer just the rightmost node. With
  // unsaved changes the preview node *is* the newest thing on the line, so that's what "latest"
  // means.
  const latestId = preview
    ? PREVIEW_ID
    : currentBranch.tip && layout.placed.has(currentBranch.tip)
      ? currentBranch.tip
      : null;
  const latestPos = latestId ? posOf(latestId) : null;
  const latestX = latestPos?.x ?? null;
  const latestY = latestPos?.y ?? null;
  const jumpToLatest = useCallback(() => {
    if (latestX == null || latestY == null) return;
    setCenter(latestX + NODE_W / 2, latestY + SPINE_TOP, { zoom: 1, duration: 350 });
  }, [latestX, latestY, setCenter]);

  // Land on the newest version, and follow it when a new one is committed (or the wrap moves it).
  useEffect(() => {
    if (latestX == null) return;
    const t = setTimeout(jumpToLatest, 0); // after React Flow has measured the nodes
    return () => clearTimeout(t);
  }, [latestX, latestY, jumpToLatest]);

  const openCommit = openId ? (drawn.find((c) => c.id === openId) ?? null) : null;

  if (openCommit) {
    return (
      <CommitDrilldown
        repoPath={repoPath}
        commit={openCommit}
        version={layout.placed.get(openCommit.id)?.version ?? 0}
        nonce={nonce}
        isTip={openCommit.id === currentBranch.tip}
        onBack={() => setOpenId(null)}
      />
    );
  }

  return (
    <section className="raised flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-panel bg-surface">
      <PanelHeader
        title={artistMode ? "Version map" : `Version map · ${currentBranch.name}`}
        meta={
          <span className="text-caption text-text-muted">
            {drawn.length} {artistMode ? "versions" : "commits"}
            {showAll && layout.laneCount > 1 && (
              <>
                {" · "}
                {layout.laneCount} {artistMode ? "lines" : "branches"}
              </>
            )}
          </span>
        }
        actions={
          drawn.length > 0 ? (
            // Its own `gap-2` container: PanelHeader's `actions` slot spaces at `gap-1`, and this
            // row's switch + divider + readout + chips need the wider rhythm the header gave them.
            <div ref={shiftLayout} className="flex items-center gap-2">
              {otherBranches > 0 && (
                // Wrapper rather than a `tourId` prop on Switch: the tour only needs a rect, and
                // every other map target is a plain `data-tour-id` too.
                <span data-tour-id="map-all-lines" className="flex items-center">
                  {/* `sm`: this is a toolbar row beside 32x32 icon chips, which is exactly what
                      the small track was sized for — `md` is the settings-list metric. */}
                  <Switch
                    active={showAll}
                    size="sm"
                    icon={GitBranch}
                    text={artistMode ? "All lines" : "All branches"}
                    label={
                      artistMode
                        ? showAll
                          ? "Show only the line you're on"
                          : `Show all version lines (${otherBranches} other)`
                        : showAll
                          ? "Show only the current branch"
                          : `Show all branches (${otherBranches} other)`
                    }
                    onClick={toggleShowAll}
                  />
                </span>
              )}
              {/* Prototype toggle for the wrapped layout, so it can be compared against the
                  single line. */}
              <Switch
                active={wrap}
                size="sm"
                icon={ArrowUDownLeft}
                text="Wrap"
                label={
                  wrap
                    ? "Lay every version out on one long line"
                    : "Wrap a long line into rows to fill the screen"
                }
                onClick={toggleWrap}
              />
              <IconButton
                icon={ArrowsClockwise}
                label="Rescan for changes"
                spinning={scanning}
                disabled={scanning}
                onClick={refresh}
              />
              {otherBranches > 0 && <span className="mx-1 h-4 w-px bg-text-muted/40" />}
              {/* One spotlight for all three: consecutive holes over adjacent 16px buttons read as
                  padding, not as teaching. `gap-2` reproduces what the header was giving them. */}
              <span data-tour-id="map-view-controls" className="flex items-center gap-2">
                <ZoomReadout />
                <IconButton icon={ArrowsOut} label="Fit all versions" onClick={fitAll} />
                <IconButton
                  icon={GpsFix}
                  label={artistMode ? "Jump to the newest version" : "Jump to the tip"}
                  onClick={jumpToLatest}
                />
              </span>
            </div>
          ) : undefined
        }
      />

      {drawn.length === 0 ? (
        <div className="grid flex-1 place-items-center">
          <div
            data-tour-id="map-empty"
            className="flex max-w-xs flex-col items-center gap-2 px-4 text-center text-text-muted"
          >
            <MapTrifold size={ICON.display} />
            <p className="text-body">
              {artistMode
                ? "No versions yet — save your first version from the Changes tab."
                : "No commits yet — make one from the Changes tab."}
            </p>
            <Button variant="primary" onClick={onShowChanges}>
              Go to Changes
            </Button>
          </div>
        </div>
      ) : (
        <div className="relative min-h-0 flex-1 bg-bg">
          <LaneGradients links={laneLinks} currentLane={layout.currentLane} />
          <ReactFlow
            nodes={allNodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            // Positions are computed; the canvas pans, the nodes never move.
            nodesDraggable={false}
            nodesConnectable={false}
            edgesFocusable={false}
            elementsSelectable={false}
            // Wheel zooms toward the cursor and drag pans — deliberately the same gesture
            // pair as the diff viewer's `useZoomPan`, so the two canvases in this app don't
            // disagree about what the wheel does.
            panOnDrag
            panOnScroll={false}
            zoomOnScroll
            zoomOnDoubleClick={false}
            minZoom={0.2}
            maxZoom={2.5}
            defaultViewport={savedViewport.current ?? undefined}
            // Off-viewport nodes aren't mounted, which is also what stops them fetching
            // their composite (see VersionNode's ponytail note).
            onlyRenderVisibleElements
            proOptions={{ hideAttribution: true }}
            className="[&_.react-flow\\_\\_pane]:cursor-grab [&_.react-flow\\_\\_pane.dragging]:cursor-grabbing"
          >
            <Background
              variant={BackgroundVariant.Lines}
              gap={24}
              size={1}
              color="var(--color-grid)"
            />
            <MiniMap
              pannable
              zoomable
              ariaLabel="Version map overview"
              bgColor="var(--color-surface-2)"
              // The mask + viewport frame are drawn by MinimapViewport below instead: React
              // Flow paints both as ONE evenodd path, so `maskStrokeColor` also strokes the
              // outer rectangle — half of that stroke lands inside the viewBox and reads as a
              // stray accent line down the minimap's edge — and a path can't round the hole's
              // corners the way an `rx` rect can.
              maskColor="transparent"
              maskStrokeWidth={0}
              nodeColor={(n) => (n.data as VersionNodeData).laneColor}
              nodeStrokeWidth={0}
              offsetScale={MINIMAP_OFFSET_SCALE}
              style={{ width: MINIMAP_W, height: MINIMAP_H }}
              // `overflow-hidden` is load-bearing: React Flow's own MiniMap container is
              // `rounded-panel` but `overflow: visible`, so the plain rectangular <svg>
              // inside it (sharp corners, sized in HTML attrs rather than clipped to the
              // parent) shows through past the rounded border — a straight edge doubling
              // the CSS border on the sides and squaring off the viewport-mask box's
              // corners instead of following the panel's rounding.
              className="!raised !overflow-hidden !rounded-panel !border !border-border !bottom-3 !right-3"
            />
            <MinimapViewport nodes={allNodes} />
            <MapActionBar
              branches={branches}
              currentBranch={currentBranch}
              actions={actions}
              picking={picking}
              onStartPicking={() => setPicking(true)}
              onCancelPicking={() => setPicking(false)}
            />
          </ReactFlow>
        </div>
      )}

      {/* Sibling of the canvas, not a child — `Modal` has no portal (cf. CleanupModal). */}
      {actions.dialogs}
    </section>
  );
}

/**
 * The map's branch controls, floating over the top-left of the canvas (clear of the minimap,
 * which sits bottom-right). Switch, merge, delete and create all live in one `Menu` — its `selected`,
 * `detail`, hover-revealed `action` and `footer` slots are exactly the legacy Branches panel's
 * row model, so this needs no new primitive.
 *
 * "Start a line here" is the one action the map alone can offer: `create_branch_at` has been in
 * the engine, tested and unreachable, because nothing else in the app is a version picker.
 */
function MapActionBar({
  branches,
  currentBranch,
  actions,
  picking,
  onStartPicking,
  onCancelPicking,
}: {
  branches: Branch[];
  currentBranch: Branch;
  actions: BranchActions;
  picking: boolean;
  onStartPicking: () => void;
  onCancelPicking: () => void;
}) {
  const { artistMode } = useArtistMode();
  const { saving } = actions;
  // The tour's "Start a new line" step spotlights a row inside this menu, and the overlay blocks
  // the real click that would open it — same pattern as Sidebar's panel-options steps.
  const { active: tourActive, step: tourStep } = useTour();
  const forceBranchMenuOpen = tourActive && tourStep.tourId === "map-branch-new";

  return (
    // `nopan` for the same reason VersionNode's thumbnail has it — without it a drag started on
    // the bar pans the canvas underneath. Panel itself already re-enables pointer events, which
    // is why MinimapViewport has to opt back *out*.
    <Panel position="top-left" className="nopan !top-3 !left-3">
      <div
        ref={shiftLayout}
        className="raised flex items-center gap-1 rounded-panel border border-border bg-surface-2 p-1"
      >
        {picking ? (
          <>
            <span className="px-2 text-dense text-text">
              {artistMode
                ? "Click the version to start the new line from"
                : "Click the commit to branch from"}
            </span>
            <Button onClick={onCancelPicking}>Cancel</Button>
          </>
        ) : (
          <>
            <Menu
              minWidth={220}
              forceOpen={forceBranchMenuOpen}
              trigger={(open) => (
                <Tooltip
                  label={artistMode ? "Choose which version line you're on" : "Switch branch"}
                >
                  <span
                    data-tour-id="map-branch"
                    className={[
                      "flex items-center gap-1.5 rounded-button px-2 py-1 text-dense text-text",
                      "hover:bg-state-hover",
                      open ? "bg-state-active" : "",
                    ].join(" ")}
                  >
                    <GitBranch size={ICON.inline} className="text-text-muted" />
                    <span className="max-w-40 truncate">{currentBranch.name}</span>
                    <CaretDown size={ICON.inline} className="text-text-muted" />
                  </span>
                </Tooltip>
              )}
              items={branches.map((b) => ({
                id: b.name,
                label: b.name,
                selected: b.kind === "current",
                detail: b.tip ? b.tip.slice(0, 7) : undefined,
                disabled: saving,
                onSelect: () => actions.switchTo(b.name),
                action:
                  b.kind === "current" ? undefined : (
                    <span className="flex items-center gap-0.5">
                      <IconButton
                        icon={GitMerge}
                        label={
                          artistMode
                            ? `Bring ${b.name} into ${currentBranch.name}`
                            : `Merge into ${currentBranch.name}`
                        }
                        size={ICON.dense}
                        disabled={saving || !b.tip}
                        onClick={() => actions.askMerge(b.name)}
                      />
                      {/* The backend refuses to delete main too (DeleteMain). */}
                      {b.name !== "main" && (
                        <IconButton
                          icon={Trash}
                          label={artistMode ? "Remove this version line" : "Delete branch"}
                          size={ICON.dense}
                          disabled={saving}
                          onClick={() => actions.askDelete(b.name)}
                        />
                      )}
                    </span>
                  ),
              }))}
              footer={[
                {
                  id: "new-branch",
                  label: artistMode ? "New version line…" : "New branch…",
                  icon: <Plus size={ICON.inline} />,
                  tourId: "map-branch-new",
                  disabled: saving,
                  onSelect: () => actions.askCreate(),
                },
              ]}
            />
            <span className="mx-0.5 h-4 w-px bg-text-muted/40" />
            <Tooltip
              label={
                artistMode
                  ? "Go back to an earlier version and take a different direction from there"
                  : "Create a branch starting at any commit"
              }
            >
              <button
                type="button"
                onClick={onStartPicking}
                disabled={saving}
                data-tour-id="map-pick"
                className="flex items-center gap-1.5 rounded-button px-2 py-1 text-dense text-text-muted transition-colors duration-(--dur-instant) ease-(--ease-out) hover:bg-state-hover hover:text-text disabled:opacity-40"
              >
                <GitBranch size={ICON.inline} />
                {artistMode ? "Start a line here…" : "Branch from a commit…"}
              </button>
            </Tooltip>
          </>
        )}
      </div>
      {actions.error && <p className="mt-1 max-w-xs text-dense text-danger">{actions.error}</p>}
    </Panel>
  );
}

/**
 * The minimap's dim-outside-the-viewport mask and the rounded frame around the viewport itself,
 * drawn as a separate overlay because React Flow paints them as one un-roundable, fully-stroked
 * path (see the MiniMap props). Geometry is React Flow's, verbatim, so the two SVGs line up.
 */
function MinimapViewport({ nodes }: { nodes: Node[] }) {
  const transform = useStore((s) => s.transform);
  const flowWidth = useStore((s) => s.width);
  const flowHeight = useStore((s) => s.height);

  const view = {
    x: -transform[0] / transform[2],
    y: -transform[1] / transform[2],
    width: flowWidth / transform[2],
    height: flowHeight / transform[2],
  };
  // The node extent doesn't move when the viewport does, but this component re-renders on every
  // frame of every pan and zoom (it has to — it draws the viewport rect). Memoizing on `nodes`
  // keeps that per-frame cost O(1) instead of a full sweep of the map on each one.
  const nodeBounds = useMemo(() => {
    let x1 = Infinity;
    let y1 = Infinity;
    let x2 = -Infinity;
    let y2 = -Infinity;
    for (const n of nodes) {
      x1 = Math.min(x1, n.position.x);
      y1 = Math.min(y1, n.position.y);
      x2 = Math.max(x2, n.position.x + (n.width ?? 0));
      y2 = Math.max(y2, n.position.y + (n.height ?? 0));
    }
    return { x1, y1, x2, y2 };
  }, [nodes]);
  // Seeded from the viewport, so an empty map (bounds still ±Infinity) collapses to exactly the
  // viewport, which is what the pre-memo loop did.
  const x1 = Math.min(view.x, nodeBounds.x1);
  const y1 = Math.min(view.y, nodeBounds.y1);
  const x2 = Math.max(view.x + view.width, nodeBounds.x2);
  const y2 = Math.max(view.y + view.height, nodeBounds.y2);
  const viewScale = Math.max((x2 - x1) / MINIMAP_W, (y2 - y1) / MINIMAP_H);
  const offset = MINIMAP_OFFSET_SCALE * viewScale;
  const boxW = viewScale * MINIMAP_W + offset * 2;
  const boxH = viewScale * MINIMAP_H + offset * 2;
  const boxX = x1 - (viewScale * MINIMAP_W - (x2 - x1)) / 2 - offset;
  const boxY = y1 - (viewScale * MINIMAP_H - (y2 - y1)) / 2 - offset;
  // The hole is rounded to match the frame, else the mask's square corners leave undimmed
  // slivers just inside the stroke's curve.
  const r = Math.min(6 * viewScale, view.width / 2, view.height / 2);
  const hole =
    `M${view.x + r},${view.y}h${view.width - 2 * r}a${r},${r} 0 0 1 ${r},${r}` +
    `v${view.height - 2 * r}a${r},${r} 0 0 1 ${-r},${r}` +
    `h${-(view.width - 2 * r)}a${r},${r} 0 0 1 ${-r},${-r}` +
    `v${-(view.height - 2 * r)}a${r},${r} 0 0 1 ${r},${-r}z`;

  return (
    // A Panel, not a plain div, so it inherits the same margin/stacking as the MiniMap's own
    // panel and lands exactly on top of it; the transparent border stands in for the MiniMap's.
    <Panel
      position="bottom-right"
      // Also the tour's minimap target: this sits pixel-exactly on the MiniMap with the same
      // geometry, so it saves reaching into React Flow's own component for a rect.
      data-tour-id="map-minimap"
      className="pointer-events-none !overflow-hidden !rounded-panel !border !border-transparent !bottom-3 !right-3"
      style={{ width: MINIMAP_W, height: MINIMAP_H }}
    >
      <svg
        width={MINIMAP_W}
        height={MINIMAP_H}
        viewBox={`${boxX} ${boxY} ${boxW} ${boxH}`}
        aria-hidden
      >
        <path
          d={`M${boxX},${boxY}h${boxW}v${boxH}h${-boxW}z ${hole}`}
          fillRule="evenodd"
          fill="color-mix(in srgb, var(--color-bg) 72%, transparent)"
        />
        <rect
          x={view.x}
          y={view.y}
          width={view.width}
          height={view.height}
          rx={r}
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth={3 * viewScale}
        />
      </svg>
    </Panel>
  );
}

/**
 * `<defs>` for the branch connectors: one gradient per lane pair that actually has a connector,
 * fading a fork out of its parent branch's color and a merge back into the color it joins.
 *
 * Two load-bearing details. It lives in its own zero-size `<svg>` rather than React Flow's —
 * a `url(#…)` paint reference resolves document-wide, and there's no hook to inject defs into
 * React Flow's `<svg>` without a custom edge component. And the gradient is
 * `userSpaceOnUse` running **purely vertically** between the two lanes' spine y values (user
 * space here is React Flow's flow coordinates, since that's the referencing element's space).
 * That confines the whole colour transition to the descent, so the connector's horizontal run at
 * either end is exactly that lane's own colour — which is also what makes the short stretch it
 * shares with the spine before the bend invisible. An `objectBoundingBox` gradient would smear
 * the transition across the entire path and tint that shared stretch.
 */
function LaneGradients({ links, currentLane }: { links: LaneLink[]; currentLane: number }) {
  if (links.length === 0) return null;
  return (
    <svg aria-hidden className="pointer-events-none absolute h-0 w-0">
      <defs>
        {links.map(({ id, from, to, y1, y2 }) => (
          <linearGradient
            key={id}
            id={id}
            gradientUnits="userSpaceOnUse"
            x1="0"
            y1={y1}
            x2="0"
            y2={y2}
          >
            <stop offset="0" style={{ stopColor: dimLane(laneColor(from, currentLane)) }} />
            <stop offset="1" style={{ stopColor: dimLane(laneColor(to, currentLane)) }} />
          </linearGradient>
        ))}
      </defs>
    </svg>
  );
}

/** Live zoom percentage; click to snap back to 100%. Mirrors the diff viewer's zoom affordance. */
function ZoomReadout() {
  const zoom = useStore((s) => s.transform[2]);
  const { zoomTo } = useReactFlow();
  return (
    <Tooltip label="Reset zoom to 100%">
      <button
        type="button"
        onClick={() => void zoomTo(1, { duration: 200 })}
        className="rounded-button px-1.5 py-0.5 font-mono text-caption text-text-muted transition-colors duration-(--dur-instant) ease-(--ease-out) hover:bg-state-hover hover:text-text"
      >
        {Math.round(zoom * 100)}%
      </button>
    </Tooltip>
  );
}

/** The full diff viewer for one version, in place of the map. The file picker in the header
 *  replaces the Inspector's file list for a multi-file version; the Inspector itself is
 *  optional here (default open, same as the legacy view) for its per-layer "Selected" details
 *  and the restore action — toggled with the same icon-button pattern `AppShell` uses. */
function CommitDrilldown({
  repoPath,
  commit,
  version,
  nonce,
  isTip,
  onBack,
}: {
  repoPath: string;
  commit: Commit;
  version: number;
  nonce: number;
  isTip: boolean;
  onBack: () => void;
}) {
  const { artistMode } = useArtistMode();
  // Same call the node already made, so this is a `diffCache` hit and opens instantly.
  const { entries, error, loading } = useCommitDiff(repoPath, commit.id);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [selectedFocusId, setSelectedFocusId] = useState<string | undefined>(undefined);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  // Which layer/composite the diff navigator has selected — mirrored into the Inspector.
  const [focus, setFocus] = useState<{ path: string; id: string } | null>(null);

  // Top-level files only — embedded palettes (`<kra>::<palette>`) are reached inside their .kra.
  const files = useMemo(
    () =>
      entries.filter((e) => !(e.kind === "palette" && e.path.includes("::"))).map((e) => e.path),
    [entries]
  );
  const active = selectedFile && files.includes(selectedFile) ? selectedFile : (files[0] ?? null);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 gap-2">
      <section className="raised flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-panel bg-surface">
        <PanelHeader
          pad="tight"
          leading={<IconButton icon={ArrowLeft} label="Back to the version map" onClick={onBack} />}
          meta={
            <>
              <span
                className={[
                  "shrink-0 text-dense text-text-muted",
                  artistMode ? "font-medium" : "font-mono",
                ].join(" ")}
              >
                {artistMode ? versionLabel(version) : commit.hash}
              </span>
              <span className="min-w-0 flex-1 truncate text-body text-text">{commit.message}</span>
            </>
          }
          actions={
            <>
              {files.length > 1 && (
                <Menu
                  align="right"
                  trigger={(open) => (
                    <Tooltip label="Choose which file to view">
                      <span
                        className={[
                          "flex items-center gap-1.5 rounded-button px-2 py-1 text-dense text-text-muted",
                          "hover:bg-state-hover hover:text-text",
                          open ? "bg-state-active text-text" : "",
                        ].join(" ")}
                      >
                        {active ? (artistMode ? assetName(active) : active) : "Files"}
                        <CaretDown size={ICON.inline} />
                      </span>
                    </Tooltip>
                  )}
                  items={files.map((f) => ({
                    id: f,
                    label: artistMode ? assetName(f) : f,
                    selected: f === active,
                    onSelect: () => setSelectedFile(f),
                  }))}
                />
              )}
              {!inspectorOpen && (
                <IconButton
                  icon={SidebarSimple}
                  label="Show inspector"
                  onClick={() => setInspectorOpen(true)}
                  iconClassName="-scale-x-100"
                />
              )}
            </>
          }
        />
        <MainPanel
          diff={entries}
          error={error}
          loading={loading}
          repoPath={repoPath}
          commitId={commit.id}
          working={false}
          nonce={nonce}
          onFocus={setFocus}
          selectedFile={active}
          focusId={selectedFocusId}
        />
      </section>

      {inspectorOpen && (
        <Inspector
          commit={commit}
          version={version}
          entries={entries}
          focus={focus}
          working={false}
          focusedFile={null}
          isTip={isTip}
          onHide={() => setInspectorOpen(false)}
          selectedFile={active}
          onSelectFile={(path, focusId) => {
            setSelectedFile(path);
            setSelectedFocusId(focusId);
          }}
        />
      )}
    </div>
  );
}
