import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  useStore,
  type Edge,
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
  MapTrifold,
} from "@phosphor-icons/react";
import { MainPanel } from "../MainPanel";
import { IconButton } from "../ui/IconButton";
import { Menu } from "../ui/Menu";
import { NODE_H, NODE_W, SPINE_TOP, VersionNode, type VersionNodeData } from "./VersionNode";
import { useCommitDiff, useCommits } from "../../lib/repoData";
import { useArtistMode } from "../../lib/artistMode";
import { assetName, versionLabel } from "../../lib/friendly";
import { buildVersionMap } from "../../lib/versionMap";
import type { Branch, Commit } from "../../types";

/** Horizontal pitch between node columns (node width + gutter). */
const NODE_PITCH = NODE_W + 72;
/** Vertical pitch between branch lanes — a node's nominal height plus a gutter. */
const LANE_PITCH = NODE_H + 60;
const FIT_PADDING = 0.15;
/** Whether the map draws every branch or just the one you're on. Map-local, so it's a plain
 *  localStorage read rather than another app-wide context (cf. artistMode/legacyHistory). */
const SHOW_ALL_KEY = "krita-vc:map-show-all";

// Version Map's own lane palette — separate from graph.ts's LANE_COLORS (mainline=accent there
// is deliberate, for the legacy History graph). Reuses theme-tuned status tokens, so no new CSS
// tokens are needed. Index 0 is always the branch you're standing on; divergent branches take
// 1.. in order of first appearance (see lib/versionMap.ts).
const BRANCH_LANE_COLORS = [
  "var(--color-info-fg)",
  "var(--color-success-fg)",
  "var(--color-warning-fg)",
  "var(--color-accent)",
];
function laneColor(lane: number): string {
  return BRANCH_LANE_COLORS[lane % BRANCH_LANE_COLORS.length];
}

// Registered once at module scope — a fresh object here would remount every node each render.
const NODE_TYPES = { version: VersionNode };

interface VersionMapPanelProps {
  repoPath: string;
  /** The current branch's commits (newest first) — what the map draws with "show all" off. */
  commits: Commit[];
  branches: Branch[];
  currentBranch: Branch;
  /** Bumped by any mutating repo op — forwarded to the drilldown diff. */
  nonce?: number;
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
}: VersionMapPanelProps) {
  const { artistMode } = useArtistMode();
  const [openId, setOpenId] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(() => localStorage.getItem(SHOW_ALL_KEY) === "1");
  const { fitView, setCenter, getViewport } = useReactFlow();

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

  const nodes = useMemo<Node<VersionNodeData>[]>(
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
            hasIncoming: at?.hasIncoming ?? false,
            hasOutgoing: at?.hasOutgoing ?? false,
            repoPath,
            onOpen,
            laneColor: laneColor(lane),
          },
        };
      }),
    [drawn, layout, repoPath, openId, onOpen]
  );

  // Edges come from `parents`, not from adjacency — so a merge commit's second parent draws its
  // own line into whichever lane it came from.
  const edges = useMemo<Edge[]>(() => {
    const out: Edge[] = [];
    for (const c of drawn) {
      const at = layout.placed.get(c.id);
      for (const p of c.parents) {
        const from = layout.placed.get(p);
        if (!from) continue;
        // A lane-crossing edge belongs to the *side* lane at either end, so the fork out of the
        // mainline and the merge back into it both read in the side line's color rather than
        // changing hue halfway. Dimmer than the nodes, so the dark well shows through.
        const color = laneColor(Math.max(from.lane, at?.lane ?? 0));
        out.push({
          id: `${p}->${c.id}`,
          source: p,
          target: c.id,
          type: "smoothstep",
          style: {
            stroke: `color-mix(in srgb, ${color} 55%, transparent)`,
            strokeWidth: 1.5,
          },
        });
      }
    }
    return out;
  }, [drawn, layout]);

  // The current branch's tip, which with lanes on is no longer just the rightmost node.
  const tipAt = currentBranch.tip ? (layout.placed.get(currentBranch.tip) ?? null) : null;
  const tipCol = tipAt?.col ?? null;
  const tipLane = tipAt?.lane ?? 0;
  const jumpToLatest = useCallback(() => {
    if (tipCol == null) return;
    setCenter(tipCol * NODE_PITCH + NODE_W / 2, tipLane * LANE_PITCH + SPINE_TOP, {
      zoom: 1,
      duration: 350,
    });
  }, [tipCol, tipLane, setCenter]);

  // Land on the newest version, and follow it when a new one is committed.
  useEffect(() => {
    if (tipCol == null) return;
    const t = setTimeout(jumpToLatest, 0); // after React Flow has measured the nodes
    return () => clearTimeout(t);
  }, [tipCol, jumpToLatest]);

  const openCommit = openId ? (drawn.find((c) => c.id === openId) ?? null) : null;

  return (
    <section className="raised flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-panel bg-surface">
      {openCommit ? (
        <CommitDrilldown
          repoPath={repoPath}
          commit={openCommit}
          version={layout.placed.get(openCommit.id)?.version ?? 0}
          nonce={nonce}
          onBack={() => setOpenId(null)}
        />
      ) : (
        <>
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
                <ZoomReadout />
                {otherBranches > 0 && (
                  <IconButton
                    icon={GitBranch}
                    active={showAll}
                    label={
                      artistMode
                        ? showAll
                          ? "Show only the line you're on"
                          : `Show all version lines (${otherBranches} other)`
                        : showAll
                          ? "Show only the current branch"
                          : `Show all branches (${otherBranches} other)`
                    }
                    size={16}
                    onClick={toggleShowAll}
                  />
                )}
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
              </div>
            </div>
          ) : (
            <div className="min-h-0 flex-1 bg-bg">
              <ReactFlow
                nodes={nodes}
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
                  maskColor="color-mix(in srgb, var(--color-bg) 72%, transparent)"
                  maskStrokeColor="var(--color-accent)"
                  maskStrokeWidth={3}
                  nodeColor={(n) => (n.data as VersionNodeData).laneColor}
                  nodeStrokeWidth={0}
                  style={{ width: 180, height: 96 }}
                  className="!raised !rounded-panel !border !border-border !bottom-3 !right-3"
                />
              </ReactFlow>
            </div>
          )}
        </>
      )}
    </section>
  );
}

/** Live zoom percentage; click to snap back to 100%. Mirrors the diff viewer's zoom affordance. */
function ZoomReadout() {
  const zoom = useStore((s) => s.transform[2]);
  const { zoomTo } = useReactFlow();
  return (
    <button
      type="button"
      title="Reset zoom to 100%"
      onClick={() => void zoomTo(1, { duration: 200 })}
      className="rounded-button px-1.5 py-0.5 font-mono text-[11px] text-text-muted transition-colors hover:bg-state-hover hover:text-text"
    >
      {Math.round(zoom * 100)}%
    </button>
  );
}

/** The full diff viewer for one version, in place of the map. No inspector — the file picker in
 *  the header is what replaces its file list for a multi-file version. */
function CommitDrilldown({
  repoPath,
  commit,
  version,
  nonce,
  onBack,
}: {
  repoPath: string;
  commit: Commit;
  version: number;
  nonce: number;
  onBack: () => void;
}) {
  const { artistMode } = useArtistMode();
  // Same call the node already made, so this is a `diffCache` hit and opens instantly.
  const { entries, error, loading } = useCommitDiff(repoPath, commit.id);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  // Top-level files only — embedded palettes (`<kra>::<palette>`) are reached inside their .kra.
  const files = useMemo(
    () =>
      entries.filter((e) => !(e.kind === "palette" && e.path.includes("::"))).map((e) => e.path),
    [entries]
  );
  const active = selectedFile && files.includes(selectedFile) ? selectedFile : (files[0] ?? null);

  return (
    <>
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-surface-2 pl-1 pr-3">
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
              <span
                className={[
                  "flex items-center gap-1.5 rounded-button px-2 py-1 text-[12px] text-text-muted",
                  "hover:bg-state-hover hover:text-text",
                  open ? "bg-state-active text-text" : "",
                ].join(" ")}
                title="Choose which file to view"
              >
                {active ? (artistMode ? assetName(active) : active) : "Files"}
                <CaretDown size={12} />
              </span>
            )}
            items={files.map((f) => ({
              id: f,
              label: artistMode ? assetName(f) : f,
              selected: f === active,
              onSelect: () => setSelectedFile(f),
            }))}
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
        selectedFile={active}
      />
    </>
  );
}
