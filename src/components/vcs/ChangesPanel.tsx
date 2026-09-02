import { useEffect, useMemo, useState } from "react";
import { ArrowCounterClockwise, CircleNotch } from "@phosphor-icons/react";
import { invoke } from "@tauri-apps/api/core";
import type { ArtLayer, Branch, DiffEntry, FileStatus, WorkingChange } from "../../types";
import { BranchBadge } from "./BranchBadge";
import { FileStatusChip } from "./FileStatusChip";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Modal } from "../ui/Modal";
import { Tooltip } from "../ui/Tooltip";
import { useRepository } from "../../lib/repository";
import { useWorkingDiff } from "../../lib/repoData";
import { resolvedAuthor, useAuthorName } from "../../lib/authorName";
import { ICON } from "../../lib/iconSize";
import { layerTypeIcon } from "../../lib/friendly";
import { inTauri } from "../../lib/tauri";
import { timed } from "../../lib/perf";

/**
 * Unsaved work on the tracked artwork, described as **which layers changed** rather than which
 * files are staged — and which of them go into the next version.
 *
 * A store tracks exactly one `.kra`, so the file list this panel used to show would always be
 * one row. What the artist actually wants to choose between is what moved in the painting, which
 * the diff already computes: `working_diff` returns per-layer `change` on every art entry, so the
 * rows need no backend call of their own.
 *
 * Ticking one sends its id in `commit_snapshot`'s `layers`, and the backend synthesizes a `.kra`
 * holding the ticked layers plus the committed form of every other one (`stage::stage_kra`). Three
 * consequences worth keeping in mind here:
 *
 * - **Top-level only.** A group is saved whole, so a change inside one rolls up to the group's row
 *   via `ArtLayer.topLevelId` — the row's id is what actually gets committed.
 * - **Everything is ticked by default**, so doing nothing saves the whole artwork exactly as
 *   before. The state tracked below is therefore what's been *un*ticked.
 * - **Changes outside the layer stack always ride along** (canvas size, animation, document
 *   settings) — there's no row to untick them with, and the copy says so once a selection is
 *   partial.
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
 * The backend enumerates layers with `.descendants()`, so a group's children arrive as siblings of
 * the group — but each one carries `topLevelId`, the id of the top-level layer it sits under. That
 * makes the rollup exact, which it has to be now that a row is a checkbox: a row's id is the layer
 * the backend will actually commit, and an approximate grouping would save something other than
 * what the artist ticked. It also catches a changed layer inside an *otherwise unchanged* group,
 * which previously surfaced as its own top-level row that nothing could act on.
 */
function layerRows(entries: DiffEntry[]): LayerRow[] {
  const byId = new Map<string, LayerRow>();
  const order: string[] = [];
  for (const entry of entries) {
    if (entry.kind !== "art") continue;
    for (const layer of entry.layers) {
      if (layer.change === "unchanged") continue;
      const top = layer.topLevelId ?? layer.id;
      let row = byId.get(top);
      if (!row) {
        // Label the row with the top-level layer itself when it's in the list; a nested change
        // inside an unchanged group is the only evidence of that group we'd otherwise have.
        const owner = entry.layers.find((l) => l.id === top) ?? layer;
        row = {
          id: top,
          name: owner.name,
          // A group whose own record is unchanged still changed, if something inside it did.
          change: owner.change === "unchanged" ? "modified" : owner.change,
          layerType: owner.layerType,
          nested: 0,
        };
        byId.set(top, row);
        order.push(top);
      }
      if (layer.id !== top) row.nested += 1;
    }
  }
  // Backend order is bottom-to-top (raw `.kra` stacking order); the diff navigator
  // (`LayerStackPanel`) reverses it to show top-first, so match that here too.
  return order.reverse().map((id) => byId.get(id) as LayerRow);
}

function LayerRowItem({
  row,
  checked,
  disabled,
  onToggle,
}: {
  row: LayerRow;
  checked: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  const Icon = layerTypeIcon(row.layerType ?? "");
  return (
    <li>
      <label className="flex cursor-pointer items-center gap-2 px-3 py-1 hover:bg-state-hover">
        <Checkbox
          checked={checked}
          disabled={disabled}
          onChange={onToggle}
          aria-label={`Include ${row.name} in this version`}
        />
        <FileStatusChip status={CHANGE_STATUS[row.change] ?? "M"} />
        <Icon size={ICON.dense} weight="regular" className="shrink-0 text-text-muted" />
        <Tooltip label={row.name}>
          <span className="min-w-0 flex-1 truncate text-dense text-text">{row.name}</span>
        </Tooltip>
        {row.nested > 0 && (
          <span className="shrink-0 text-micro text-text-muted">+{row.nested} inside</span>
        )}
      </label>
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
  // What the artist has *un*ticked, not what they've ticked — so a layer that shows up in a later
  // scan is included by default and "do nothing" keeps saving the whole artwork.
  const [excluded, setExcluded] = useState<Set<string>>(() => new Set());

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

  // Ticks are only meaningful against the diff they were made on: a new scan, a different artwork
  // or a branch switch all replace the rows underneath them.
  useEffect(() => setExcluded(new Set()), [docPath, currentBranch.name, refreshNonce]);

  const included = useMemo(() => rows.filter((r) => !excluded.has(r.id)), [rows, excluded]);
  // A subset only exists once something is unticked *and* there are rows to untick. With no rows
  // (a change outside the layer stack) there's nothing to choose, so it's a whole-artwork save.
  const partial = rows.length > 0 && included.length < rows.length;
  const nothingPicked = rows.length > 0 && included.length === 0;

  const toggle = (id: string) =>
    setExcluded((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const doCommit = async () => {
    if (!message.trim() || saving || !path || nothingPicked) return;
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
          // null is the whole artwork — the same call this made before layer picking existed.
          layers: partial ? included.map((r) => r.id) : null,
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
      <p className="px-3 py-2 text-dense text-text-muted">Couldn’t check this artwork: {error}</p>
    );
  }

  return (
    <div className="flex flex-col">
      <div
        data-tour-id="changes-branch"
        className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border px-3 text-caption text-text-muted"
      >
        Saving to
        <BranchBadge branch={currentBranch} />
      </div>

      <div data-tour-id="changes-unstaged">
        <h3 className="flex h-8 shrink-0 items-center justify-between gap-2 px-3 text-caption font-medium uppercase tracking-wide text-text-muted">
          <span className="flex items-center gap-2">
            Since your last version
            {rows.length > 0 && (
              <span className="text-text-muted/70">
                {partial ? `${included.length}/${rows.length}` : rows.length}
              </span>
            )}
          </span>
          {changed && (
            <Tooltip
              label={checking ? "Checking for changes…" : "Undo everything since the last version"}
            >
              <button
                type="button"
                onClick={() => setConfirmDiscard(true)}
                disabled={saving || checking}
                className="flex items-center gap-1 rounded-button px-1 py-0.5 text-micro normal-case tracking-normal text-text-muted hover:bg-state-hover hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
              >
                <ArrowCounterClockwise size={ICON.inline} />
                Undo all
              </button>
            </Tooltip>
          )}
        </h3>

        {!changed ? (
          <p className="px-3 pb-2 text-dense text-text-muted">
            No changes yet — this artwork matches its latest version.
          </p>
        ) : changed.status === "D" ? (
          <p className="px-3 pb-2 text-dense text-text-muted">
            This artwork is missing from its folder. Undo to bring the latest version back.
          </p>
        ) : loading ? (
          <p className="flex items-center gap-1.5 px-3 pb-2 text-dense text-text-muted">
            <CircleNotch size={ICON.inline} className="animate-spin" />
            Looking at what changed…
          </p>
        ) : rows.length > 0 ? (
          <>
            {rows.length > 1 && (
              <div className="flex justify-end px-3 pb-1">
                <button
                  type="button"
                  onClick={() => setExcluded(partial ? new Set() : new Set(rows.map((r) => r.id)))}
                  disabled={saving || checking}
                  className="rounded-button px-1 py-0.5 text-micro text-text-muted hover:bg-state-hover hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
                >
                  {partial ? "Choose all" : "Choose none"}
                </button>
              </div>
            )}
            <ul className="flex flex-col pb-1">
              {rows.map((row) => (
                <LayerRowItem
                  key={row.id}
                  row={row}
                  checked={!excluded.has(row.id)}
                  disabled={saving || checking}
                  onToggle={() => toggle(row.id)}
                />
              ))}
            </ul>
            {partial && (
              <p className="px-3 pb-2 text-micro leading-relaxed text-text-muted">
                The layers you leave out stay unsaved, ready for a later version. Canvas size and
                document settings are always included.
              </p>
            )}
          </>
        ) : (
          <p className="px-3 pb-2 text-dense text-text-muted">
            {changed.status === "U"
              ? "First version — the whole artwork will be saved."
              : "Changed outside the layer stack (canvas size, animation, or document settings)."}
          </p>
        )}
      </div>

      {inTauri() && (
        <div className="mt-1 flex flex-col gap-2 border-t border-border p-3">
          {/* Focus: the sanctioned `.inset-well` exception (DESIGN.md § Focus Ring Spec) — the
              global ring's 2px offset lands in the gap outside a carved-in field and reads as a
              halo, so the accent moves onto the field's own edge. `!outline-none` is genuinely
              needed: the global `:focus-visible` rule is unlayered and beats Tailwind's utilities
              layer. `focus-visible:` (not `focus:`) so it doesn't fire on mouse click, and the
              surface step is the non-color co-indicator the exception is required to carry. */}
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Describe this version…"
            rows={2}
            data-tour-id="commit-message"
            className="resize-none inset-well rounded-button border border-border bg-bg px-2 py-1.5 text-dense text-text placeholder:text-text-muted focus-visible:border-accent focus-visible:bg-surface-2 !outline-none"
          />
          {commitError && <p className="text-caption text-danger">{commitError}</p>}
          <Button
            variant="primary"
            onClick={doCommit}
            disabled={!message.trim() || !changed || saving || nothingPicked}
            data-tour-id="commit-button"
          >
            {saving && <CircleNotch size={ICON.inline} className="animate-spin" />}
            {saving
              ? "Saving version…"
              : nothingPicked
                ? "Choose at least one layer"
                : partial
                  ? `Save ${included.length} of ${rows.length} layers`
                  : "Save this version"}
          </Button>
        </div>
      )}

      {confirmDiscard && (
        <Modal
          title="Undo everything since the last version?"
          onClose={() => (saving ? undefined : setConfirmDiscard(false))}
          footer={(close) => (
            <>
              <Button onClick={close} disabled={saving}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={discardAll} disabled={saving}>
                {saving ? "Undoing…" : "Undo changes"}
              </Button>
            </>
          )}
        >
          <p className="text-body leading-relaxed text-text-muted">
            This permanently reverts{" "}
            <span className="font-medium text-text">{current?.name ?? "this artwork"}</span> to its
            latest saved version. Everything painted since is lost — including Krita’s undo history
            for it.
          </p>
          {discardError && <p className="mt-3 text-dense text-danger">{discardError}</p>}
        </Modal>
      )}
    </div>
  );
}
