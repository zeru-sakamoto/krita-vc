import type { ArtDiff, DiffEntry, PaletteDiff, TextDiff } from "../../types";
import { ArtDiffView } from "./ArtDiffView";
import { assetKind, assetName, statusVerb } from "../../lib/friendly";
import { ICON } from "../../lib/iconSize";

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
    <p className="px-3 py-2 text-body text-text-muted">
      {`${kind.charAt(0).toUpperCase()}${kind.slice(1)} ${verb}${detail}.`}
    </p>
  );
}

/** Artist-friendly view of a file with no visual diff: no code, no hunks, no line numbers. */
function FriendlyFileDiff({ file }: { file: TextDiff }) {
  const kind = assetKind(file.path);
  const Icon = kind.icon;
  return (
    <div className="border-b border-border">
      {/* Friendly header */}
      <div className="sticky top-0 z-(--z-sticky) flex items-center gap-2 border-y border-border bg-surface px-3 py-2">
        <Icon size={ICON.default} className="shrink-0 text-text-muted" />
        <span className="text-body font-medium text-text">{assetName(file.path)}</span>
        <span className="text-dense text-text-muted">{kind.label}</span>
        <span className="ml-auto rounded-badge bg-surface-3 px-1.5 py-0.5 text-caption text-text-muted">
          {statusVerb(file.status)}
        </span>
      </div>
      <GenericSummary file={file} />
    </div>
  );
}

interface DiffViewProps {
  entries: DiffEntry[];
  /** Which top-level entry (by path) to show. Defaults to the first entry when absent/stale. */
  selectedPath?: string | null;
  /** Navigator id to seed the selected art file's view with (e.g. jump to its palette pane). */
  focusId?: string;
  /** Diff source, forwarded to art views for lazy per-layer raster loading. Absent in the browser. */
  repoPath?: string;
  commitId?: string | null;
  working?: boolean;
  nonce?: number;
  /** Forwarded to art views so the navigator selection reaches the Inspector. */
  onFocus?: (f: { path: string; id: string }) => void;
  /** Forwarded to art views so a layer row's "View layer details" can reveal the Inspector. */
  onOpenInspector?: () => void;
}

export function DiffView({
  entries,
  selectedPath,
  focusId,
  repoPath,
  commitId,
  working,
  nonce,
  onFocus,
  onOpenInspector,
}: DiffViewProps) {
  // Every palette entry is one embedded in a .kra (`<kra>::<palette-file>`, since standalone
  // palettes aren't tracked), and isn't independently selectable — it's reached via its parent
  // .kra's own view instead.
  const topLevel = entries.filter((e): e is ArtDiff | TextDiff => e.kind !== "palette");
  const selected = topLevel.find((e) => e.path === selectedPath) ?? topLevel[0];

  if (!selected) {
    return <div className="h-full flex flex-col overflow-auto bg-bg" />;
  }

  if (selected.kind === "art") {
    const embeddedPalette = entries.find(
      (e): e is PaletteDiff => e.kind === "palette" && e.path.startsWith(`${selected.path}::`)
    );
    return (
      <div className="h-full flex flex-col overflow-auto bg-bg">
        <ArtDiffView
          key={`${selected.path}:${focusId ?? "auto"}`}
          diff={selected}
          palette={embeddedPalette}
          initialFocusId={focusId}
          repoPath={repoPath}
          commitId={commitId}
          working={working}
          nonce={nonce}
          onFocus={onFocus}
          onOpenInspector={onOpenInspector}
        />
      </div>
    );
  }

  // The one text entry left is a `.kra` that couldn't be rasterized (or a deleted one); the
  // backend sends no lines for it, so there's no raw line diff to offer outside Artist Mode.
  return (
    <div className="h-full flex flex-col overflow-auto bg-bg">
      <FriendlyFileDiff key={selected.path} file={selected} />
    </div>
  );
}
