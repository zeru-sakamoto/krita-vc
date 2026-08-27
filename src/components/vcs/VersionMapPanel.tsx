import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  MiniMap,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  useStore,
  type BuiltInEdge,
  type Node,
  type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  ArrowLeft,
  ArrowsOut,
  CaretDown,
  CaretLineRight,
  GitBranch,
  GitMerge,
  MapTrifold,
  Plus,
  SidebarSimple,
  Trash,
} from "@phosphor-icons/react";
import { MainPanel } from "../MainPanel";
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
import { useArtistMode } from "../../lib/artistMode";
import { assetName, versionLabel } from "../../lib/friendly";
import { bendFraction, buildVersionMap } from "../../lib/versionMap";
import type { Branch, Commit } from "../../types";

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
/** The y a lane's spine runs at, in flow coordinates — shared by the gradient defs below. */
function laneY(lane: number): number {
  return lane * LANE_PITCH + SPINE_TOP;
}
/** Gradient id for a connector between two lanes. */
function laneGradientId(from: number, to: number): string {
  return `kvc-lane-${from}-${to}`;
}

// Registered once at module scope — a fresh object here would remount every node each render.
const NODE_TYPES = { version: VersionNode, preview: PreviewNode };

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
  const [openId, setOpenId] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(() => localStorage.getItem(SHOW_ALL_KEY) === "1");
  // Pick-a-version mode: the action bar asked for a starting point, so the next node click
  // forks a branch there instead of opening the diff viewer.
  const [picking, setPicking] = useState(false);
  const { fitView, setCenter, getViewport } = useReactFlow();
  const actions = useBranchActions({ currentBranch: currentBranch.name, onShowChanges });

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

  const nodes = useMemo<Node<VersionNodeData | PreviewNodeData>[]>(
    () =>
      drawn.map((commit) => {
        const at = layout.placed.get(commit.id);
        const lane = at?.lane ?? 0;
        return {
          id: commit.id,
          type: "version",
          position: { x: (at?.col ?? 1) * NODE_PITCH, y: lane * LANE_PITCH },
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
            repoPath,
            onOpen: onNodeClick,
            laneColor: laneColor(lane, layout.currentLane),
          },
        };
      }),
    [drawn, layout, repoPath, openId, onNodeClick]
  );

  const allNodes = useMemo<Node<VersionNodeData | PreviewNodeData>[]>(() => {
    if (!preview) return nodes;
    return [
      ...nodes,
      {
        id: PREVIEW_ID,
        type: "preview",
        position: { x: preview.col * NODE_PITCH, y: layout.currentLane * LANE_PITCH },
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
  }, [nodes, preview, layout.currentLane, currentBranch.name, picking, onShowChanges]);

  // Edges come from `parents`, not from adjacency — so a merge commit's second parent draws its
  // own line into whichever lane it came from. Since both handles sit on the connector dot, one
  // edge is the *entire* line between two versions: through the source node, the gutter, and the
  // target node, with nothing else drawing any part of it.
  const { edges, laneLinks } = useMemo(() => {
    const out: BuiltInEdge[] = [];
    const links = new Set<string>(); // "from-to" lane pairs needing a gradient
    for (const c of drawn) {
      const at = layout.placed.get(c.id);
      for (const p of c.parents) {
        const from = layout.placed.get(p);
        if (!from) continue;
        const toLane = at?.lane ?? 0;
        const crosses = from.lane !== toLane;
        if (crosses) links.add(`${from.lane}-${toLane}`);
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
            stroke: crosses
              ? `url(#${laneGradientId(from.lane, toLane)})`
              : dimLane(laneColor(toLane, layout.currentLane)),
            strokeWidth: 1.5,
          },
        });
      }
    }
    // The dashed run out to the pending-changes preview. Same lane as its parent, so it needs no
    // bend and no lane gradient — just the spine's color, dashed.
    if (preview && currentBranch.tip && layout.placed.has(currentBranch.tip)) {
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
    return { edges: out, laneLinks: [...links] };
  }, [drawn, layout, preview, currentBranch.tip]);

  // The current branch's tip, which with lanes on is no longer just the rightmost node.
  const tipAt = currentBranch.tip ? (layout.placed.get(currentBranch.tip) ?? null) : null;
  const tipCol = tipAt?.col ?? null;
  const tipLane = tipAt?.lane ?? 0;
  // With unsaved changes the preview node *is* the newest thing on the line, so that's what
  // "latest" means.
  const latestCol = preview?.col ?? tipCol;
  const latestLane = preview ? layout.currentLane : tipLane;
  const jumpToLatest = useCallback(() => {
    if (latestCol == null) return;
    setCenter(latestCol * NODE_PITCH + NODE_W / 2, latestLane * LANE_PITCH + SPINE_TOP, {
      zoom: 1,
      duration: 350,
    });
  }, [latestCol, latestLane, setCenter]);

  // Land on the newest version, and follow it when a new one is committed.
  useEffect(() => {
    if (latestCol == null) return;
    const t = setTimeout(jumpToLatest, 0); // after React Flow has measured the nodes
    return () => clearTimeout(t);
  }, [latestCol, jumpToLatest]);

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
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-surface-2 pl-3 pr-1">
        <span className="text-[11px] font-medium uppercase tracking-wide text-text-muted">
          {artistMode ? "Version map" : `Version map · ${currentBranch.name}`}
        </span>
        <span className="flex-1 text-[11px] text-text-muted">
          {drawn.length} {artistMode ? "versions" : "commits"}
          {showAll && layout.laneCount > 1 && (
            <>
              {" · "}
              {layout.laneCount} {artistMode ? "lines" : "branches"}
            </>
          )}
        </span>
        {drawn.length > 0 && (
          <>
            {otherBranches > 0 && (
              <>
                <Switch
                  active={showAll}
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
                <span className="mx-1 h-4 w-px bg-text-muted/40" />
              </>
            )}
            <ZoomReadout />
            <IconButton
              icon={ArrowsOut}
              label="Fit all versions"
              size={16}
              onClick={() => void fitView({ padding: FIT_PADDING, duration: 350 })}
            />
            <IconButton
              icon={CaretLineRight}
              label={artistMode ? "Jump to the newest version" : "Jump to the tip"}
              size={16}
              onClick={jumpToLatest}
            />
          </>
        )}
      </header>

      {drawn.length === 0 ? (
        <div className="grid flex-1 place-items-center">
          <div className="flex max-w-xs flex-col items-center gap-2 px-4 text-center text-text-muted">
            <MapTrifold size={32} />
            <p className="text-[13px]">
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

  return (
    // `nopan` for the same reason VersionNode's thumbnail has it — without it a drag started on
    // the bar pans the canvas underneath. Panel itself already re-enables pointer events, which
    // is why MinimapViewport has to opt back *out*.
    <Panel position="top-left" className="nopan !top-3 !left-3">
      <div className="raised flex items-center gap-1 rounded-panel border border-border bg-surface-2 p-1">
        {picking ? (
          <>
            <span className="px-2 text-[12px] text-text">
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
              trigger={(open) => (
                <Tooltip
                  label={artistMode ? "Choose which version line you're on" : "Switch branch"}
                >
                  <span
                    className={[
                      "flex items-center gap-1.5 rounded-button px-2 py-1 text-[12px] text-text",
                      "hover:bg-state-hover",
                      open ? "bg-state-active" : "",
                    ].join(" ")}
                  >
                    <GitBranch size={13} className="text-text-muted" />
                    <span className="max-w-40 truncate">{currentBranch.name}</span>
                    <CaretDown size={12} className="text-text-muted" />
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
                        size={14}
                        disabled={saving || !b.tip}
                        onClick={() => actions.askMerge(b.name)}
                      />
                      {/* The backend refuses to delete main too (DeleteMain). */}
                      {b.name !== "main" && (
                        <IconButton
                          icon={Trash}
                          label={artistMode ? "Remove this version line" : "Delete branch"}
                          size={14}
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
                  icon: <Plus size={13} />,
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
                className="flex items-center gap-1.5 rounded-button px-2 py-1 text-[12px] text-text-muted transition-colors hover:bg-state-hover hover:text-text disabled:opacity-40"
              >
                <GitBranch size={13} />
                {artistMode ? "Start a line here…" : "Branch from a commit…"}
              </button>
            </Tooltip>
          </>
        )}
      </div>
      {actions.error && <p className="mt-1 max-w-xs text-[12px] text-danger">{actions.error}</p>}
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
  let x1 = view.x;
  let y1 = view.y;
  let x2 = view.x + view.width;
  let y2 = view.y + view.height;
  for (const n of nodes) {
    x1 = Math.min(x1, n.position.x);
    y1 = Math.min(y1, n.position.y);
    x2 = Math.max(x2, n.position.x + (n.width ?? 0));
    y2 = Math.max(y2, n.position.y + (n.height ?? 0));
  }
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
function LaneGradients({ links, currentLane }: { links: string[]; currentLane: number }) {
  if (links.length === 0) return null;
  return (
    <svg aria-hidden className="pointer-events-none absolute h-0 w-0">
      <defs>
        {links.map((key) => {
          const [from, to] = key.split("-").map(Number);
          return (
            <linearGradient
              key={key}
              id={laneGradientId(from, to)}
              gradientUnits="userSpaceOnUse"
              x1="0"
              y1={laneY(from)}
              x2="0"
              y2={laneY(to)}
            >
              <stop offset="0" style={{ stopColor: dimLane(laneColor(from, currentLane)) }} />
              <stop offset="1" style={{ stopColor: dimLane(laneColor(to, currentLane)) }} />
            </linearGradient>
          );
        })}
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
        className="rounded-button px-1.5 py-0.5 font-mono text-[11px] text-text-muted transition-colors hover:bg-state-hover hover:text-text"
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
        <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-surface-2 pl-1 pr-1">
          <IconButton icon={ArrowLeft} label="Back to the version map" size={16} onClick={onBack} />
          <span
            className={[
              "shrink-0 text-[12px] text-text-muted",
              artistMode ? "font-medium" : "font-mono",
            ].join(" ")}
          >
            {artistMode ? versionLabel(version) : commit.hash}
          </span>
          <span className="min-w-0 flex-1 truncate text-[13px] text-text">{commit.message}</span>
          {files.length > 1 && (
            <Menu
              align="right"
              trigger={(open) => (
                <Tooltip label="Choose which file to view">
                  <span
                    className={[
                      "flex items-center gap-1.5 rounded-button px-2 py-1 text-[12px] text-text-muted",
                      "hover:bg-state-hover hover:text-text",
                      open ? "bg-state-active text-text" : "",
                    ].join(" ")}
                  >
                    {active ? (artistMode ? assetName(active) : active) : "Files"}
                    <CaretDown size={12} />
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
              size={18}
              onClick={() => setInspectorOpen(true)}
              iconClassName="-scale-x-100"
            />
          )}
        </header>
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
