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
import { ArrowLeft, ArrowsOut, CaretDown, CaretLineRight, MapTrifold } from "@phosphor-icons/react";
import { MainPanel } from "../MainPanel";
import { IconButton } from "../ui/IconButton";
import { Menu } from "../ui/Menu";
import { NODE_H, NODE_W, SPINE_TOP, VersionNode, type VersionNodeData } from "./VersionNode";
import { useCommitDiff } from "../../lib/repoData";
import { useArtistMode } from "../../lib/artistMode";
import { assetName, versionLabel, versionNumbers } from "../../lib/friendly";
import type { Branch, Commit } from "../../types";

/** Horizontal pitch between node columns (node width + gutter). */
const NODE_PITCH = NODE_W + 72;
/** Vertical pitch between branch lanes. Only lane 0 is used today; branches come later. */
const LANE_PITCH = 340;
const FIT_PADDING = 0.15;

// Version Map's own lane palette — separate from graph.ts's LANE_COLORS (mainline=accent there
// is deliberate, for the legacy History graph). Reuses theme-tuned status tokens, so no new CSS
// tokens are needed. Index 0 is the current/main branch; later indices are for divergent
// branches once multi-branch lanes land here (LANE_PITCH is already reserved for that).
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
  commits: Commit[];
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
 * Only the current branch is drawn, which is free: `list_commits` is already scoped to commits
 * reachable from the current branch tip. Divergent branches are deliberately not laid out yet —
 * `LANE_PITCH` and the parent-derived edges are the seams they'll arrive through.
 */
export function VersionMapPanel(props: VersionMapPanelProps) {
  // The provider has to sit above anything calling useReactFlow/useStore, including the header.
  return (
    <ReactFlowProvider>
      <VersionMap {...props} />
    </ReactFlowProvider>
  );
}

function VersionMap({ repoPath, commits, currentBranch, nonce = 0 }: VersionMapPanelProps) {
  const { artistMode } = useArtistMode();
  const [openId, setOpenId] = useState<string | null>(null);
  const versions = useMemo(() => versionNumbers(commits), [commits]);
  const newestId = commits[0]?.id ?? null;
  const { fitView, setCenter, getViewport } = useReactFlow();

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

  const nodes = useMemo<Node<VersionNodeData>[]>(() => {
    // `useCommits` hands back newest-first; the map reads oldest→newest, left to right.
    const ordered = [...commits].reverse();
    return ordered.map((commit, i) => ({
      id: commit.id,
      type: "version",
      position: { x: i * NODE_PITCH, y: 0 * LANE_PITCH },
      // Both dimensions are required: the MiniMap reads them off this object (see NODE_H).
      width: NODE_W,
      height: NODE_H,
      selected: commit.id === openId,
      data: {
        commit,
        version: versions.get(commit.id) ?? 0,
        isTip: commit.id === currentBranch.tip,
        isOldest: i === 0,
        isNewest: i === ordered.length - 1,
        repoPath,
        onOpen,
        laneColor: laneColor(0),
      },
    }));
  }, [commits, versions, currentBranch.tip, repoPath, openId, onOpen]);

  // Edges come from `parents`, not from adjacency — so a merge commit's second parent already
  // draws its own line the day branches land, with no change here.
  const edges = useMemo<Edge[]>(() => {
    const known = new Set(commits.map((c) => c.id));
    const out: Edge[] = [];
    for (const c of commits) {
      for (const p of c.parents) {
        if (known.has(p)) {
          out.push({
            id: `${p}->${c.id}`,
            source: p,
            target: c.id,
            type: "smoothstep",
            // Dimmer than the nodes it connects — same hue as the branch's lane color, mixed
            // toward transparent so the dark well shows through.
            style: {
              stroke: `color-mix(in srgb, ${laneColor(0)} 55%, transparent)`,
              strokeWidth: 1.5,
            },
          });
        }
      }
    }
    return out;
  }, [commits]);

  const jumpToLatest = useCallback(() => {
    if (!newestId) return;
    const i = commits.length - 1; // newest is last left-to-right
    setCenter(i * NODE_PITCH + NODE_W / 2, SPINE_TOP, { zoom: 1, duration: 350 });
  }, [newestId, commits.length, setCenter]);

  // Land on the newest version, and follow it when a new one is committed.
  useEffect(() => {
    if (!newestId) return;
    const t = setTimeout(jumpToLatest, 0); // after React Flow has measured the nodes
    return () => clearTimeout(t);
  }, [newestId, jumpToLatest]);

  const openCommit = openId ? (commits.find((c) => c.id === openId) ?? null) : null;

  return (
    <section className="raised flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-panel bg-surface">
      {openCommit ? (
        <CommitDrilldown
          repoPath={repoPath}
          commit={openCommit}
          version={versions.get(openCommit.id) ?? 0}
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
              {commits.length} {artistMode ? "versions" : "commits"}
            </span>
            {commits.length > 0 && (
              <>
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

          {commits.length === 0 ? (
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
