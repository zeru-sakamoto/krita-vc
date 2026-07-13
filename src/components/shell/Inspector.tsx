import { useMemo, useState } from "react";
import { ArrowCounterClockwise, Palette as PaletteIcon, X } from "@phosphor-icons/react";
import { IconButton } from "../ui/IconButton";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { FileStatusChip } from "../vcs/FileStatusChip";
import { COMPOSITE_ID, PALETTE_ID } from "../vcs/LayerStackPanel";
import type { ArtDiff, ArtLayer, Commit, DiffEntry, FileChange, PaletteDiff } from "../../types";
import { fullTimestamp } from "../../lib/format";
import {
  assetName,
  layerChangeLabel,
  layerTypeLabel,
  paletteName,
  statusVerb,
  versionLabel,
} from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";
import { useRepository } from "../../lib/repository";

interface InspectorProps {
  commit: Commit | null;
  /** Version number for the selected commit (used in Artist Mode). */
  version: number;
  /** Current diff entries — the Inspector resolves `focus` against these. */
  entries: DiffEntry[];
  /** The diff navigator's selection (which art file + which layer/composite), or null. */
  focus: { path: string; id: string } | null;
  /** True when showing uncommitted working-tree changes rather than a selected commit. */
  working: boolean;
  /** The focused file's path, when `working` is true. */
  focusedFile: string | null;
  /** True when `commit` is the current branch tip — restoring it discards in place. */
  isTip: boolean;
  onClose: () => void;
  /** Which changed file (among possibly several) is currently shown in the main panel. */
  selectedFile: string | null;
  /** Selects a file (and optionally a navigator id within it) to show in the main panel. */
  onSelectFile: (path: string, focusId?: string) => void;
}

function MetaRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[64px_1fr] items-baseline gap-2 px-3 py-1.5">
      <span className="text-[11px] font-medium uppercase text-text-muted">{label}</span>
      <span className="selectable text-[13px] text-text">{children}</span>
    </div>
  );
}

/** The "Selected" section: details for the layer or composite picked in the diff navigator. */
function SelectedDetails({ art, layer }: { art: ArtDiff; layer: ArtLayer | null }) {
  if (layer) {
    return (
      <div className="border-t border-border px-3 py-2">
        <h3 className="mb-1.5 text-[11px] font-medium uppercase text-text-muted">Selected layer</h3>
        <div className="flex flex-col">
          <MetaRow label="Name">{layer.name}</MetaRow>
          {layer.layerType && <MetaRow label="Type">{layerTypeLabel(layer.layerType)}</MetaRow>}
          <MetaRow label="Visible">{layer.visible === false ? "Hidden" : "Visible"}</MetaRow>
          <MetaRow label="Opacity">{layer.opacity}%</MetaRow>
          <MetaRow label="Blend">{layer.blendMode}</MetaRow>
          <MetaRow label="Change">{layerChangeLabel(layer.change)}</MetaRow>
          {layer.bounds && (
            <MetaRow label="Bounds">
              {layer.bounds.w} × {layer.bounds.h} at ({layer.bounds.x}, {layer.bounds.y})
            </MetaRow>
          )}
        </div>
      </div>
    );
  }
  const changed = art.layers.filter((l) => l.change !== "unchanged").length;
  return (
    <div className="border-t border-border px-3 py-2">
      <h3 className="mb-1.5 text-[11px] font-medium uppercase text-text-muted">Composite</h3>
      <div className="flex flex-col">
        <MetaRow label="Size">
          {art.width} × {art.height}
        </MetaRow>
        {art.dpi != null && art.dpi > 0 && <MetaRow label="Res">{Math.round(art.dpi)} DPI</MetaRow>}
        {art.colorModel && (
          <MetaRow label="Color">
            {art.colorModel}
            {art.colorProfile ? ` · ${art.colorProfile}` : ""}
          </MetaRow>
        )}
        <MetaRow label="Layers">{art.layers.length}</MetaRow>
        <MetaRow label="Changed">{changed}</MetaRow>
        <MetaRow label="Status">{statusVerb(art.status)}</MetaRow>
      </div>
    </div>
  );
}

/**
 * 280px toggleable inspector showing the selected commit's metadata, plus a "Selected" section
 * mirroring the diff navigator's layer/composite selection.
 * (DESIGN.md → Layout & App Shell → Inspector panel)
 */
export function Inspector({
  commit,
  version,
  entries,
  focus,
  working,
  focusedFile,
  isTip,
  onClose,
  selectedFile,
  onSelectFile,
}: InspectorProps) {
  const { artistMode } = useArtistMode();
  const { rollbackToCommit, saving } = useRepository();
  const [confirmRestore, setConfirmRestore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Split the commit's changed files into selectable rows: regular files (each carrying its
  // embedded palette, if any, per `<kra>::<palette-file>`-keyed entries) and standalone palettes.
  const { fileChanges, paletteChanges } = useMemo(() => {
    const files: { change: FileChange; embeddedPalette: PaletteDiff | undefined }[] = [];
    const palettes: FileChange[] = [];
    for (const c of commit?.changes ?? []) {
      const entry = entries.find((e) => e.path === c.path);
      if (entry?.kind === "palette") {
        palettes.push(c);
      } else {
        const embeddedPalette = entries.find(
          (e): e is PaletteDiff => e.kind === "palette" && e.path.startsWith(`${c.path}::`)
        );
        files.push({ change: c, embeddedPalette });
      }
    }
    return { fileChanges: files, paletteChanges: palettes };
  }, [commit, entries]);

  // Resolve the navigator selection against the current diff. A stale focus (e.g. the next
  // commit is text-only, or a palette is selected) resolves to nothing and the section hides —
  // no reset logic needed.
  const focusedArt =
    focus != null
      ? (entries.find((e): e is ArtDiff => e.kind === "art" && e.path === focus.path) ?? null)
      : null;
  const focusedLayer =
    focusedArt && focus && focus.id !== COMPOSITE_ID && focus.id !== PALETTE_ID
      ? (focusedArt.layers.find((l) => l.id === focus.id) ?? null)
      : null;
  const showSelected = focusedArt != null && focus?.id !== PALETTE_ID;

  const restoreLabel = artistMode ? `Version ${version}` : (commit?.hash ?? "");

  const onRestore = async () => {
    setError(null);
    try {
      await rollbackToCommit(commit!.id);
      setConfirmRestore(false);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="flex h-full w-70 shrink-0 flex-col border-l border-border">
      {/* Single header row — py-1.5, aligns with the "Modified Hero" file header */}
      <div className="flex shrink-0 items-center border-b border-border bg-surface-2 px-3 py-2 h-8">
        <span className="flex-1 text-[11px] font-medium uppercase tracking-wide text-text-muted">
          {working ? "Changes" : artistMode ? "Version" : "Commit"}
        </span>
        <IconButton icon={X} label="Close inspector" size={16} onClick={onClose} />
      </div>
      {/* Scrollable content */}
      <div className="min-h-0 flex-1 overflow-auto bg-surface">
        {working ? (
          focusedFile ? (
            <div className="flex flex-col">
              <div className="border-b border-border py-1">
                <MetaRow label="Status">
                  <span className="rounded-badge bg-surface-3 px-1.5 py-0.5 text-[11px] text-text-muted">
                    Unsaved changes
                  </span>
                </MetaRow>
                <MetaRow label="File">
                  <span className={["selectable", artistMode ? "" : "font-mono"].join(" ")}>
                    {artistMode ? assetName(focusedFile) : focusedFile}
                  </span>
                </MetaRow>
              </div>

              {showSelected && focusedArt && (
                <SelectedDetails art={focusedArt} layer={focusedLayer} />
              )}
            </div>
          ) : (
            <div className="grid h-full place-items-center px-6 text-center text-[12px] text-text-muted">
              No changes to show.
            </div>
          )
        ) : !commit ? (
          <div className="grid h-full place-items-center px-6 text-center text-[12px] text-text-muted">
            Select a commit to inspect its details.
          </div>
        ) : (
          <div className="flex flex-col">
            <div className="border-b border-border py-1">
              {artistMode ? (
                <MetaRow label="Version">{versionLabel(version)}</MetaRow>
              ) : (
                <MetaRow label="Hash">
                  <span className="font-mono text-[12px]">{commit.hash}</span>
                </MetaRow>
              )}
              <MetaRow label="Author">{commit.author}</MetaRow>
              <MetaRow label="Date">
                <span className="text-[12px] text-text-muted">
                  {fullTimestamp(commit.timestamp)}
                </span>
              </MetaRow>
            </div>

            <div className="border-b border-border px-3 py-2.5">
              <p className="selectable text-[13px] leading-relaxed text-text">{commit.message}</p>
            </div>

            <div className="px-3 py-2">
              <h3 className="mb-1.5 text-[11px] font-medium uppercase text-text-muted">
                Changed files ({commit.changes.length})
              </h3>
              <ul className="flex flex-col">
                {fileChanges.map(({ change: c, embeddedPalette }) => (
                  <li key={c.path} className="flex flex-col">
                    <button
                      type="button"
                      onClick={() => onSelectFile(c.path, undefined)}
                      className={[
                        "flex w-full items-center gap-2 rounded-button border-l-2 px-1 py-1 text-left transition-colors",
                        selectedFile === c.path
                          ? "border-accent bg-accent/12"
                          : "border-transparent hover:bg-white/5",
                      ].join(" ")}
                    >
                      <FileStatusChip status={c.status} />
                      <span
                        className={[
                          "selectable truncate text-[12px] text-text",
                          artistMode ? "" : "font-mono",
                        ].join(" ")}
                      >
                        {artistMode ? assetName(c.path) : c.path}
                      </span>
                    </button>
                    {embeddedPalette && (
                      <button
                        type="button"
                        onClick={() => onSelectFile(c.path, PALETTE_ID)}
                        className={[
                          "ml-4 flex items-center gap-2 rounded-button border-l-2 px-1 py-1 text-left transition-colors",
                          focus?.path === c.path && focus?.id === PALETTE_ID
                            ? "border-accent bg-accent/12"
                            : "border-transparent hover:bg-white/5",
                        ].join(" ")}
                      >
                        <PaletteIcon size={12} className="shrink-0 text-text-muted" />
                        <span className="selectable truncate text-[11px] text-text-muted">
                          {artistMode ? paletteName(embeddedPalette.path) : embeddedPalette.path}
                        </span>
                      </button>
                    )}
                  </li>
                ))}
              </ul>

              {paletteChanges.length > 0 && (
                <>
                  <h3 className="mb-1.5 mt-2 text-[11px] font-medium uppercase text-text-muted">
                    Palettes ({paletteChanges.length})
                  </h3>
                  <ul className="flex flex-col">
                    {paletteChanges.map((c) => (
                      <li key={c.path}>
                        <button
                          type="button"
                          onClick={() => onSelectFile(c.path, undefined)}
                          className={[
                            "flex w-full items-center gap-2 rounded-button border-l-2 px-1 py-1 text-left transition-colors",
                            selectedFile === c.path
                              ? "border-accent bg-accent/12"
                              : "border-transparent hover:bg-white/5",
                          ].join(" ")}
                        >
                          <FileStatusChip status={c.status} />
                          <PaletteIcon size={13} className="shrink-0 text-text-muted" />
                          <span
                            className={[
                              "selectable truncate text-[12px] text-text",
                              artistMode ? "" : "font-mono",
                            ].join(" ")}
                          >
                            {artistMode ? paletteName(c.path) : c.path}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>

            {showSelected && focusedArt && (
              <SelectedDetails art={focusedArt} layer={focusedLayer} />
            )}
          </div>
        )}
      </div>

      {/* Restore (rollback) action */}
      {!working && commit && (
        <div className="shrink-0 border-t border-border bg-surface-2 px-3 py-2">
          <Button
            variant="default"
            className="w-full"
            disabled={saving}
            onClick={() => {
              setError(null);
              setConfirmRestore(true);
            }}
          >
            <ArrowCounterClockwise size={14} />
            {artistMode ? "Restore this version" : "Roll back to this commit"}
          </Button>
        </div>
      )}

      {confirmRestore && commit && (
        <Modal
          title={artistMode ? `Restore ${restoreLabel}?` : `Roll back to ${restoreLabel}?`}
          onClose={() => (saving ? undefined : setConfirmRestore(false))}
          footer={
            <>
              <Button onClick={() => setConfirmRestore(false)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="primary" onClick={onRestore} disabled={saving}>
                {saving ? "Restoring…" : artistMode ? "Restore this version" : "Roll back"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            {isTip
              ? "This discards any unsaved changes and restores your last saved version. No new version is recorded."
              : "This copies that version's files into your working folder and saves the result as a new version. Nothing in your history is lost — you can always come back."}
          </p>
          {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
        </Modal>
      )}
    </div>
  );
}
