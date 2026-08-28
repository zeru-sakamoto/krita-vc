import { useEffect, useMemo, useState } from "react";
import { ArrowCounterClockwise, CircleNotch } from "@phosphor-icons/react";
import { invoke } from "@tauri-apps/api/core";
import type { ArtLayer, Branch, DiffEntry, FileStatus, WorkingChange } from "../../types";
import { BranchBadge } from "./BranchBadge";
import { FileStatusChip } from "./FileStatusChip";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { Tooltip } from "../ui/Tooltip";
import { useRepository } from "../../lib/repository";
import { useWorkingDiff } from "../../lib/repoData";
import { resolvedAuthor, useAuthorName } from "../../lib/authorName";
import { layerTypeIcon } from "../../lib/friendly";
import { inTauri } from "../../lib/tauri";
import { timed } from "../../lib/perf";

/**
 * Unsaved work on the tracked artwork, described as **which layers changed** rather than which
 * files are staged.
 *
 * A store tracks exactly one `.kra`, so the file list this panel used to show would always be
 * one row — and staging a subset of a one-file working tree means nothing. What the artist
 * actually wants to know is what moved in the painting since the last version, which the diff
 * already computes: `working_diff` returns per-layer `change` on every art entry, so this needs
 * no new backend call.
 *
 * The rows are **read-only**. Choosing which layers go into a version is a real feature and a
 * separate one — it needs a write path that synthesizes a `.kra` holding only the ticked layers
 * (`merge::merge_layers` folds selected top-level layers onto a base, which is most of it). Until
 * that exists, a version captures the whole artwork, and pretending otherwise with checkboxes
 * that don't bind would be worse than showing none.
 */

/** A changed top-level layer, with the nested changes inside a group rolled up into it. */
interface LayerRow {
  id: string;
  name: string;
  change: ArtLayer["change"];
  layerType?: string;
  /** Changed descendants folded into this row (groups only). */
  nested: number;
}

const CHANGE_STATUS: Record<string, FileStatus> = {
  added: "A",
  removed: "D",
  modified: "M",
};

/**
 * Roll the diff's flat layer list up to top-level rows.
 *
 * The backend enumerates layers with `.descendants()`, so a group's children arrive as siblings
 * of the group with no parent link. Without one, the honest rollup is: show every changed layer,
 * and where a changed group is present, count the changed layers that follow it as nested. That
 * over-counts in a document with several groups — which is why the row says "and N more inside"
 * rather than claiming an exact tree.
 */
function layerRows(entries: DiffEntry[]): LayerRow[] {
  const rows: LayerRow[] = [];
  for (const entry of entries) {
    if (entry.kind !== "art") continue;
    let group: LayerRow | null = null;
    for (const layer of entry.layers) {
      const isGroup = layer.layerType === "grouplayer";
      if (layer.change === "unchanged") {
        if (isGroup) group = null;
        continue;
      }
      if (isGroup) {
        group = {
          id: layer.id,
          name: layer.name,
          change: layer.change,
          layerType: layer.layerType,
          nested: 0,
        };
        rows.push(group);
      } else if (group) {
        group.nested += 1;
      } else {
        rows.push({
          id: layer.id,
          name: layer.name,
          change: layer.change,
          layerType: layer.layerType,
          nested: 0,
        });
      }
    }
  }
  return rows;
}

function LayerRowItem({ row }: { row: LayerRow }) {
  const Icon = layerTypeIcon(row.layerType ?? "");
  return (
    <li className="flex items-center gap-2 px-3 py-1">
      <FileStatusChip status={CHANGE_STATUS[row.change] ?? "M"} />
      <Icon size={14} weight="regular" className="shrink-0 text-text-muted" />
      <Tooltip label={row.name}>
        <span className="min-w-0 flex-1 truncate text-[12px] text-text">{row.name}</span>
      </Tooltip>
      {row.nested > 0 && (
        <span className="shrink-0 text-[10px] text-text-muted">+{row.nested} inside</span>
      )}
    </li>
  );
}

export function ChangesPanel({
  currentBranch,
  focusedFile,
  onFocusFile,
  items,
  error,
}: {
  currentBranch: Branch;
  focusedFile: string | null;
  onFocusFile: (path: string | null) => void;
  /** Working-tree changes — at most one, the tracked artwork. Owned by `Sidebar` so the
   *  panel-options "discard" and "set aside" actions see the same scan. */
  items: WorkingChange[];
  error: string | null;
}) {
  const {
    current,
    saving,
    scanning,
    setSaving,
    setBusyMessage,
    refresh,
    discardChanges,
    refreshNonce,
  } = useRepository();
  const { authorName } = useAuthorName();
  const [message, setMessage] = useState("");
  const [commitError, setCommitError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [discardError, setDiscardError] = useState<string | null>(null);

  const path = current?.path ?? null;
  const changed = items[0]?.change ?? null;
  const docPath = changed?.path ?? null;

  // Focus the artwork so the main panel shows its working diff — there's only one thing to look
  // at, so making the artist click it first would be a step for nothing. Clears back to null once
  // the change disappears (e.g. right after a commit) — otherwise the main panel and Inspector
  // keep showing a stale "unsaved changes" diff for a file that's actually clean.
  useEffect(() => {
    if (focusedFile !== docPath) onFocusFile(docPath);
  }, [docPath, focusedFile, onFocusFile]);

  // A commit error is only meaningful for the artwork/branch it happened on — otherwise it
  // lingers (this component stays mounted across switches) and misleadingly reads as live.
  useEffect(() => setCommitError(null), [path, currentBranch.name]);

  // `refreshNonce` matters: without it this panel keeps painting the pre-commit diff while
  // `AppShell`'s copy refetches. Both callers land on one backend call anyway — `useWorkingDiff`
  // dedupes concurrent requests for the same path|file|nonce.
  const { entries, loading } = useWorkingDiff(path ?? "", docPath, refreshNonce);
  const rows = useMemo(() => layerRows(entries), [entries]);
  // Either kind of "checking" leaves the diff this button reverts in flux.
  const checking = scanning || loading;

  const doCommit = async () => {
    if (!message.trim() || saving || !path) return;
    setSaving(true);
    setBusyMessage("Saving this version — please wait…");
    setCommitError(null);
    try {
      await timed(
        path,
        "commit",
        invoke<{ id: string }>("commit_snapshot", {
          path,
          message: message.trim(),
          author: resolvedAuthor(authorName),
          paths: null,
        }),
        (c) => ({ commitId: c.id })
      );
      setMessage("");
      refresh(); // refetch changes (now clean) + history
    } catch (e) {
      setCommitError(String(e));
    } finally {
      setSaving(false);
      setBusyMessage(null);
    }
  };

  const discardAll = async () => {
    setDiscardError(null);
    try {
      await discardChanges(docPath ? [docPath] : []);
      setConfirmDiscard(false);
    } catch (e) {
      setDiscardError(String(e));
    }
  };

  if (error && items.length === 0) {
    return (
      <p className="px-3 py-2 text-[12px] text-text-muted">Couldn’t check this artwork: {error}</p>
    );
  }

  return (
    <div className="flex flex-col">
      <div
        data-tour-id="changes-branch"
        className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border px-3 text-[11px] text-text-muted"
      >
        Saving to
        <BranchBadge branch={currentBranch} />
      </div>

      <div data-tour-id="changes-unstaged">
        <h3 className="flex h-8 shrink-0 items-center justify-between gap-2 px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
          <span className="flex items-center gap-2">
            Since your last version
            {rows.length > 0 && <span className="text-text-muted/70">{rows.length}</span>}
          </span>
          {changed && (
            <Tooltip
              label={checking ? "Checking for changes…" : "Undo everything since the last version"}
            >
              <button
                type="button"
                onClick={() => setConfirmDiscard(true)}
                disabled={saving || checking}
                className="flex items-center gap-1 rounded-button px-1 py-0.5 text-[10px] normal-case tracking-normal text-text-muted hover:bg-state-hover hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
              >
                <ArrowCounterClockwise size={11} />
                Undo all
              </button>
            </Tooltip>
          )}
        </h3>

        {!changed ? (
          <p className="px-3 pb-2 text-[12px] text-text-muted">
            No changes yet — this artwork matches its latest version.
          </p>
        ) : changed.status === "D" ? (
          <p className="px-3 pb-2 text-[12px] text-text-muted">
            This artwork is missing from its folder. Undo to bring the latest version back.
          </p>
        ) : loading ? (
          <p className="flex items-center gap-1.5 px-3 pb-2 text-[12px] text-text-muted">
            <CircleNotch size={12} className="animate-spin" />
            Looking at what changed…
          </p>
        ) : rows.length > 0 ? (
          <ul className="flex flex-col pb-1">
            {rows.map((row) => (
              <LayerRowItem key={row.id} row={row} />
            ))}
          </ul>
        ) : (
          <p className="px-3 pb-2 text-[12px] text-text-muted">
            {changed.status === "U"
              ? "First version — the whole artwork will be saved."
              : "Changed outside the layer stack (canvas size, animation, or document settings)."}
          </p>
        )}
      </div>

      {inTauri() && (
        <div className="mt-1 flex flex-col gap-2 border-t border-border p-3">
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Describe this version…"
            rows={2}
            data-tour-id="commit-message"
            className="resize-none inset-well rounded-button border border-border bg-bg px-2 py-1.5 text-[12px] text-text placeholder:text-text-muted focus:border-accent !outline-none"
          />
          {commitError && <p className="text-[11px] text-danger">{commitError}</p>}
          <button
            type="button"
            onClick={doCommit}
            disabled={!message.trim() || !changed || saving}
            data-tour-id="commit-button"
            className="flex items-center justify-center gap-1.5 rounded-button bg-accent/15 px-2 py-1.5 text-[12px] font-medium text-accent transition-colors hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving && <CircleNotch size={13} className="animate-spin" />}
            {saving ? "Saving version…" : "Save this version"}
          </button>
        </div>
      )}

      {confirmDiscard && (
        <Modal
          title="Undo everything since the last version?"
          onClose={() => (saving ? undefined : setConfirmDiscard(false))}
          footer={
            <>
              <Button onClick={() => setConfirmDiscard(false)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={discardAll} disabled={saving}>
                {saving ? "Undoing…" : "Undo changes"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            This permanently reverts{" "}
            <span className="font-medium text-text">{current?.name ?? "this artwork"}</span> to its
            latest saved version. Everything painted since is lost — including Krita’s undo history
            for it.
          </p>
          {discardError && <p className="mt-3 text-[12px] text-danger">{discardError}</p>}
        </Modal>
      )}
    </div>
  );
}
