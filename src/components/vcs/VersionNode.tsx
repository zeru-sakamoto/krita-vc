import { memo, useMemo } from "react";
import { Handle, Position, useStore } from "@xyflow/react";
import { PencilSimple, Plus, Trash, type Icon } from "@phosphor-icons/react";
import type { ArtDiff, ArtLayer, Commit } from "../../types";
import { useCommitDiff } from "../../lib/repoData";
import { useArtistMode } from "../../lib/artistMode";
import { relativeTime } from "../../lib/format";
import {
  assetKind,
  layerChangeLabel,
  layerTypeIcon,
  layerTypeLabel,
  versionLabel,
} from "../../lib/friendly";
import { wrapSvg } from "../../lib/svgArt";

/** Node column width — React Flow lays out with it, so the panel imports it too. */
export const NODE_W = 208;
/** Nominal node height. Real height varies with the chip count and the LOD cutoff, but React
 *  Flow's MiniMap sizes from the *user* node object (`getNodeDimensions` reads it, not the
 *  measured box), so without an explicit height it plots nothing at all. Visuals are driven by
 *  the inner content either way. */
export const NODE_H = 280;
const THUMB_H = 150;
/** Y of the connector dot, measured from the node's top — where the edge handles attach. */
export const SPINE_TOP = THUMB_H + 8 + 5;
const MAX_CHIPS = 6;
/** Below this zoom the caption and chips are dropped — see `detailed` below. */
const LOD_ZOOM = 0.55;

// The checkerboard behind a composite with transparency — same swatch as the layer navigator's
// thumbs (LayerStackPanel), so a transparent .kra reads identically in both places.
const CHECKER = "bg-[repeating-conic-gradient(#1a1916_0%_25%,#222019_0%_50%)] bg-size-[8px_8px]";

const CHANGE_GLYPH: Record<
  Exclude<ArtLayer["change"], "unchanged">,
  { icon: Icon; color: string }
> = {
  added: { icon: Plus, color: "text-success-fg" },
  modified: { icon: PencilSimple, color: "text-warning-fg" },
  removed: { icon: Trash, color: "text-danger" },
};

/** What the panel packs into each React Flow node. */
export interface VersionNodeData {
  commit: Commit;
  /** Version number (newest = highest), from `versionNumbers`. */
  version: number;
  /** This commit is the current branch's tip. */
  isTip: boolean;
  /** Ends of the line: the spine is drawn half-width on these so it stops at the end dots. */
  isOldest: boolean;
  isNewest: boolean;
  /** This branch's lane color (see VersionMapPanel's BRANCH_LANE_COLORS) — shared by every node
   *  on the line, so the connector dots read as one continuous branch identity. */
  laneColor: string;
  repoPath: string;
  onOpen: (id: string) => void;
  [key: string]: unknown;
}

// Handles carry the edges; they're never dragged from, so they're invisible and sit exactly on
// the dot's row so a connector meets the dot instead of the card.
function SpineHandle({ type, position }: { type: "source" | "target"; position: Position }) {
  return (
    <Handle
      type={type}
      position={position}
      isConnectable={false}
      className="!h-0 !w-0 !min-h-0 !min-w-0 !border-0 !bg-transparent"
      style={{ top: SPINE_TOP }}
    />
  );
}

/**
 * One version on the map: the after-composite on top, the connector dot the spine runs through,
 * then the chips for the layers this version changed. A React Flow node type — position comes
 * from the panel's computed layout, never from dragging.
 *
 * The composite and the changed-layer list both come from `commit_diff` — the *same* call the diff
 * viewer makes, so opening this node is a `diffCache` hit rather than a second backend round trip.
 * ponytail: `commit_diff` is `run_heavy` (2 concurrent) and also builds a changed-pixel mask this
 * node throws away, so a cold cache over a long history is a lot of heavy calls. Bounded by React
 * Flow's `onlyRenderVisibleElements` — an off-viewport node isn't mounted, so it never fetches. If
 * that stops being enough, the upgrade is a dedicated `commit_thumbnails(path, ids)` returning just
 * the composite URL + layer changes.
 */
export const VersionNode = memo(function VersionNode({
  data,
  selected,
}: {
  data: VersionNodeData;
  selected?: boolean;
}) {
  const { commit, version, isTip, isOldest, isNewest, laneColor, repoPath, onOpen } = data;
  // Dimmer than the dots — same hue as the lane, mixed toward transparent.
  const dimLaneColor = `color-mix(in srgb, ${laneColor} 55%, transparent)`;
  const { artistMode } = useArtistMode();
  // A boolean selector, so a node re-renders only when the threshold is actually crossed —
  // not on every frame of a zoom gesture.
  const detailed = useStore((s) => s.transform[2] >= LOD_ZOOM);

  const { entries, loading } = useCommitDiff(repoPath, commit.id);
  const art = useMemo(() => entries.find((e): e is ArtDiff => e.kind === "art") ?? null, [entries]);
  const thumb = useMemo(
    () => (art?.afterImage ? wrapSvg(art.afterImage, art.width, art.height) : null),
    [art]
  );
  const changed = useMemo(() => (art?.layers ?? []).filter((l) => l.change !== "unchanged"), [art]);
  const rel = useMemo(() => relativeTime(commit.timestamp), [commit.timestamp]);
  const extraFiles = commit.changes.length - (art ? 1 : 0);
  const shown = changed.slice(0, MAX_CHIPS);
  const overflow = changed.length - shown.length;
  const { icon: KindIcon } = assetKind(art?.path ?? commit.changes[0]?.path ?? "");

  return (
    <div className="flex flex-col items-center" style={{ width: NODE_W }}>
      <SpineHandle type="target" position={Position.Left} />
      <SpineHandle type="source" position={Position.Right} />

      <button
        type="button"
        onClick={() => onOpen(commit.id)}
        aria-pressed={selected}
        title={commit.message}
        className={[
          "raised block w-full overflow-hidden rounded-panel bg-surface-2",
          "transition-[box-shadow,transform] duration-100 ease-out",
          "hover:-translate-y-0.5 active:translate-y-0",
          selected ? "ring-1 ring-accent" : "",
        ].join(" ")}
        style={
          isTip
            ? { height: THUMB_H, outline: `2px solid ${laneColor}`, outlineOffset: 4 }
            : { height: THUMB_H }
        }
      >
        {thumb ? (
          <div
            className={`h-full w-full ${CHECKER} [&>svg]:h-full [&>svg]:w-full`}
            dangerouslySetInnerHTML={{ __html: thumb }}
          />
        ) : loading ? (
          <div className="h-full w-full animate-pulse bg-surface-3" />
        ) : (
          <div className="grid h-full w-full place-items-center bg-surface-2">
            <KindIcon size={28} className="text-text-muted" />
          </div>
        )}
      </button>

      {/* Connector dot, with the spine continued across the node behind it. React Flow's edges
          only span the gutter between node boxes, so without this rule the line reads as broken
          at every version. Matches the edge stroke in `VersionMapPanel`. */}
      <div className="relative my-2 flex w-full shrink-0 justify-center">
        {!isOldest && (
          <span
            aria-hidden
            className="absolute left-0 right-1/2 top-1/2 h-[1.5px] -translate-y-1/2"
            style={{ background: dimLaneColor }}
          />
        )}
        {!isNewest && (
          <span
            aria-hidden
            className="absolute left-1/2 right-0 top-1/2 h-[1.5px] -translate-y-1/2"
            style={{ background: dimLaneColor }}
          />
        )}
        <span
          aria-hidden
          className="relative h-2.5 w-2.5 shrink-0 rounded-full"
          style={{ background: laneColor }}
        />
      </div>

      {detailed && (
        <>
          <div className="w-full text-center">
            <p
              className={[
                "text-[11px] text-text-muted",
                artistMode ? "font-medium" : "font-mono",
              ].join(" ")}
            >
              {artistMode ? versionLabel(version) : commit.hash}
              <span className="mx-1" aria-hidden>
                ·
              </span>
              {rel}
            </p>
            <p className="mt-0.5 line-clamp-2 text-[12px] leading-snug text-text">
              {commit.message}
            </p>
          </div>

          {(shown.length > 0 || extraFiles > 0) && (
            <div className="mt-2 grid w-full grid-cols-2 gap-1.5">
              {shown.map((l) => {
                const TypeIcon = layerTypeIcon(l.layerType ?? "");
                const glyph =
                  CHANGE_GLYPH[l.change as keyof typeof CHANGE_GLYPH] ?? CHANGE_GLYPH.modified;
                const GlyphIcon = glyph.icon;
                return (
                  <span
                    key={l.id}
                    title={`${l.name} — ${layerTypeLabel(l.layerType ?? "")}, ${layerChangeLabel(l.change)}`}
                    className="raised flex min-w-0 items-center gap-1 rounded-badge bg-surface-2 px-1.5 py-1 text-[11px] leading-none"
                  >
                    <TypeIcon size={12} className="shrink-0 text-text-muted" />
                    <span className="min-w-0 flex-1 truncate text-text">{l.name}</span>
                    <GlyphIcon size={11} weight="bold" className={`shrink-0 ${glyph.color}`} />
                  </span>
                );
              })}
              {overflow > 0 && (
                <span className="flex items-center justify-center rounded-badge px-1.5 py-1 text-[11px] leading-none text-text-muted">
                  +{overflow} more
                </span>
              )}
              {extraFiles > 0 && (
                <span className="flex items-center justify-center rounded-badge px-1.5 py-1 text-[11px] leading-none text-text-muted">
                  +{extraFiles} file{extraFiles === 1 ? "" : "s"}
                </span>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
});
