import type { ArtDiff, DiffEntry, DiffLine, PaletteDiff, TextDiff } from "../../types";
import { FileStatusChip } from "./FileStatusChip";
import { ArtDiffView } from "./ArtDiffView";
import { PaletteDiffView } from "./PaletteDiffView";
import { LayerStackPanel, PALETTE_ID } from "./LayerStackPanel";
import { useArtistMode } from "../../lib/artistMode";
import { assetKind, assetName, statusVerb } from "../../lib/friendly";
import { useState } from "react";

/** Per-line background/foreground per DESIGN.md → Diff Colors. */
function lineClasses(kind: DiffLine["kind"]): string {
  switch (kind) {
    case "add":
      return "bg-diff-add text-diff-add-fg";
    case "del":
      return "bg-diff-del text-diff-del-fg";
    case "hunk":
      return "bg-surface-3 text-text-muted";
    default:
      return "bg-bg text-text-muted";
  }
}

function gutter(n?: number) {
  return (
    <span className="w-10 shrink-0 select-none pr-2 text-right text-[11px] text-text-muted/70">
      {n ?? ""}
    </span>
  );
}

function DiffFileBlock({ file }: { file: TextDiff }) {
  return (
    <div>
      {/* File path header */}
      <div className="sticky top-0 z-(--z-sticky) flex items-center gap-2 border-y border-border bg-surface px-3 py-1.5">
        <FileStatusChip status={file.status} />
        <span className="selectable font-mono text-[12px] text-text">{file.path}</span>
      </div>

      {/* Diff lines */}
      <div className="font-mono text-[12px] leading-[1.6]">
        {file.lines.map((line, i) => {
          const isHunk = line.kind === "hunk";
          return (
            <div key={i} className={["flex", lineClasses(line.kind)].join(" ")}>
              {!isHunk && gutter(line.oldLine)}
              {!isHunk && gutter(line.newLine)}
              <span className="w-4 shrink-0 select-none text-center text-text-muted/60">
                {line.kind === "add" ? "+" : line.kind === "del" ? "−" : ""}
              </span>
              <span className="selectable whitespace-pre pr-3">
                {isHunk ? `      ${line.text}` : line.text}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function GenericSummary({ file }: { file: TextDiff }) {
  const added = file.lines.filter((l) => l.kind === "add").length;
  const removed = file.lines.filter((l) => l.kind === "del").length;
  const kind = assetKind(file.path).label.toLowerCase();
  const parts: string[] = [];
  if (added > 0) parts.push(`${added} ${added === 1 ? "entry" : "entries"} added`);
  if (removed > 0) parts.push(`${removed} ${removed === 1 ? "entry" : "entries"} removed`);
  const detail = parts.length > 0 ? ` — ${parts.join(", ")}` : "";
  const verb = file.status === "A" ? "created" : file.status === "D" ? "removed" : "updated";
  return (
    <p className="px-3 py-2 text-[13px] text-text-muted">
      {`${kind.charAt(0).toUpperCase()}${kind.slice(1)} ${verb}${detail}.`}
    </p>
  );
}

/** Artist-friendly view of a non-art, non-palette file: no code, no hunks, no line numbers. */
function FriendlyFileDiff({ file }: { file: TextDiff }) {
  const kind = assetKind(file.path);
  const Icon = kind.icon;
  return (
    <div className="border-b border-border">
      {/* Friendly header */}
      <div className="sticky top-0 z-(--z-sticky) flex items-center gap-2 border-y border-border bg-surface px-3 py-2">
        <Icon size={16} className="shrink-0 text-text-muted" />
        <span className="text-[13px] font-medium text-text">{assetName(file.path)}</span>
        <span className="text-[12px] text-text-muted">{kind.label}</span>
        <span className="ml-auto rounded-badge bg-surface-3 px-1.5 py-0.5 text-[11px] text-text-muted">
          {statusVerb(file.status)}
        </span>
      </div>
      <GenericSummary file={file} />
    </div>
  );
}

/**
 * Standalone palette view: used when there are palette diffs but no art diff to
 * attach them to. Mirrors the ArtDiffView layout (left navigator + right grid).
 */
function StandalonePaletteDiff({ palette }: { palette: PaletteDiff }) {
  const [selectedId, setSelectedId] = useState<string>(PALETTE_ID);
  // Build a minimal ArtDiff shell so LayerStackPanel can render a palette-only navigator.
  // We pass a zero-layer ArtDiff so the Layers section is empty; only Color Palette shows.
  const emptyArtDiff: ArtDiff = {
    kind: "art",
    path: "",
    status: "M",
    width: 1,
    height: 1,
    layers: [],
    regions: [],
  };
  return (
    <div className="flex flex-col border-b border-border">
      {/* Header */}
      <div className="sticky top-0 z-(--z-sticky) flex items-center gap-2 border-y border-border bg-surface px-3 py-1.5">
        <FileStatusChip status={palette.status} />
        <span className="selectable text-[12px] font-medium text-text">
          {assetName(palette.path)}
        </span>
      </div>
      <div className="flex" style={{ minHeight: 300 }}>
        <LayerStackPanel
          diff={emptyArtDiff}
          palette={palette}
          selectedId={selectedId}
          onSelect={setSelectedId}
        />
        <div className="min-w-0 flex-1">
          <PaletteDiffView diff={palette} />
        </div>
      </div>
    </div>
  );
}

interface DiffViewProps {
  entries: DiffEntry[];
  /** Diff source, forwarded to art views for lazy per-layer raster loading. Absent in the browser. */
  repoPath?: string;
  commitId?: string | null;
  working?: boolean;
  nonce?: number;
}

export function DiffView({ entries, repoPath, commitId, working, nonce }: DiffViewProps) {
  const { artistMode } = useArtistMode();

  // Partition entries by kind so we can attach the first palette to the first art diff's navigator.
  const artDiffs: ArtDiff[] = entries.filter((e): e is ArtDiff => e.kind === "art");
  const paletteDiffs: PaletteDiff[] = entries.filter((e): e is PaletteDiff => e.kind === "palette");
  const textDiffs: TextDiff[] = entries.filter((e): e is TextDiff => e.kind === "text");

  // The first palette (if any) is embedded in the first art diff's LayerStackPanel.
  // Any additional palettes (rare) render standalone.
  const attachedPalette = paletteDiffs[0];
  const extraPalettes = paletteDiffs.slice(1);

  return (
    <div className="h-full flex flex-col overflow-auto bg-bg">
      {/* Art diffs — first one gets the palette embedded in its navigator */}
      {artDiffs.map((diff, i) => (
        <ArtDiffView
          key={diff.path}
          diff={diff}
          palette={i === 0 ? attachedPalette : undefined}
          repoPath={repoPath}
          commitId={commitId}
          working={working}
          nonce={nonce}
        />
      ))}

      {/* Palette-only: if there are palettes but no art diff, show standalone */}
      {artDiffs.length === 0 && attachedPalette && (
        <StandalonePaletteDiff key={attachedPalette.path} palette={attachedPalette} />
      )}

      {/* Extra palettes (not attached to any art diff) */}
      {extraPalettes.map((p) => (
        <StandalonePaletteDiff key={p.path} palette={p} />
      ))}

      {/* Text (config, etc.) — friendly summary in artist mode, raw diff otherwise */}
      {textDiffs.map((file) =>
        artistMode ? (
          <FriendlyFileDiff key={file.path} file={file} />
        ) : (
          <DiffFileBlock key={file.path} file={file} />
        )
      )}
    </div>
  );
}
