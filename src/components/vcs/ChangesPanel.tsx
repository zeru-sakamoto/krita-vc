import { useState, type Dispatch, type SetStateAction } from "react";
import { ArrowCounterClockwise, CircleNotch, Minus, Plus } from "@phosphor-icons/react";
import { invoke } from "@tauri-apps/api/core";
import type { Branch, WorkingChange } from "../../types";
import { BranchBadge } from "./BranchBadge";
import { FileStatusChip } from "./FileStatusChip";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";
import { resolvedAuthor, useAuthorName } from "../../lib/authorName";
import { assetName } from "../../lib/friendly";
import { inTauri } from "../../lib/tauri";
import { timed } from "../../lib/perf";

function Section({
  title,
  items,
  action,
  onToggle,
  onToggleAll,
  disabled,
  focusedFile,
  onFocusFile,
  onDiscardFile,
}: {
  title: string;
  items: WorkingChange[];
  action: "stage" | "unstage";
  onToggle: (path: string) => void;
  /** Stage all / unstage all. */
  onToggleAll: () => void;
  /** True while a commit is in flight — staging is locked. */
  disabled: boolean;
  /** File whose working-tree diff is currently shown in the main panel. */
  focusedFile: string | null;
  onFocusFile: (path: string) => void;
  /** Discard this one file's uncommitted changes. */
  onDiscardFile: (path: string) => void;
}) {
  return (
    <div>
      <h3 className="flex h-8 shrink-0 items-center justify-between gap-2 px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
        <span className="flex items-center gap-2">
          {title}
          <span className="text-text-muted/70">{items.length}</span>
        </span>
        {items.length > 0 && (
          <button
            type="button"
            onClick={onToggleAll}
            disabled={disabled}
            className="rounded-button px-1 py-0.5 text-[10px] normal-case tracking-normal text-text-muted hover:bg-white/5 hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
          >
            {action === "stage" ? "Stage all" : "Unstage all"}
          </button>
        )}
      </h3>
      <ul className="flex flex-col">
        {items.map(({ change }) => (
          <li
            key={change.path}
            className={[
              "group flex items-center gap-2 px-3 py-1",
              change.path === focusedFile ? "bg-accent/12" : "hover:bg-white/5",
            ].join(" ")}
          >
            <FileStatusChip status={change.status} />
            <button
              type="button"
              onClick={() => onFocusFile(change.path)}
              title="Show this file's changes"
              className="min-w-0 flex-1 truncate text-left font-mono text-[12px] text-text hover:text-accent"
            >
              {change.path}
            </button>
            <button
              type="button"
              title="Discard changes to this file"
              aria-label="Discard changes to this file"
              onClick={() => onDiscardFile(change.path)}
              disabled={disabled}
              className="grid h-5 w-5 shrink-0 place-items-center rounded-button text-text-muted opacity-0 transition-opacity hover:bg-white/5 hover:text-danger group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <ArrowCounterClockwise size={12} />
            </button>
            <button
              type="button"
              title={action === "stage" ? "Stage" : "Unstage"}
              aria-label={action === "stage" ? "Stage file" : "Unstage file"}
              onClick={() => onToggle(change.path)}
              disabled={disabled}
              className="grid h-5 w-5 shrink-0 place-items-center rounded-button text-text-muted opacity-0 transition-opacity hover:bg-white/5 hover:text-text group-hover:opacity-100 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {action === "stage" ? <Plus size={13} /> : <Minus size={13} />}
            </button>
          </li>
        ))}
        {items.length === 0 && (
          <li className="px-3 py-2 text-[12px] text-text-muted">Nothing here.</li>
        )}
      </ul>
    </div>
  );
}

/**
 * Working-tree changes from the real scanner (`scan_repository`) for the selected
 * repository. Empty in a plain browser (no backend). Staging is cosmetic local
 * state — this VCS commits the whole working tree, not a staging area.
 */
export function ChangesPanel({
  currentBranch,
  focusedFile,
  onFocusFile,
  items,
  setItems,
  error,
}: {
  currentBranch: Branch;
  focusedFile: string | null;
  onFocusFile: (path: string) => void;
  /** Working-tree changes + their cosmetic staged flag — lifted to `Sidebar` so the
   *  "discard current changes" action can see the same staged/unstaged split. */
  items: WorkingChange[];
  setItems: Dispatch<SetStateAction<WorkingChange[]>>;
  error: string | null;
}) {
  const { current, saving, setSaving, setBusyMessage, refresh, discardChanges } = useRepository();
  const { artistMode } = useArtistMode();
  const { authorName } = useAuthorName();
  const [message, setMessage] = useState("");
  const [commitError, setCommitError] = useState<string | null>(null);
  const [confirmDiscardPath, setConfirmDiscardPath] = useState<string | null>(null);
  const [discardError, setDiscardError] = useState<string | null>(null);
  const [confirmCommit, setConfirmCommit] = useState<"none" | "partial" | null>(null);

  const path = current?.path ?? null;

  const toggle = (path: string) =>
    setItems((prev) => prev.map((c) => (c.change.path === path ? { ...c, staged: !c.staged } : c)));
  const setAll = (staged: boolean) => setItems((prev) => prev.map((c) => ({ ...c, staged })));

  const discardOne = async () => {
    if (!confirmDiscardPath) return;
    setDiscardError(null);
    try {
      await discardChanges([confirmDiscardPath]);
      setConfirmDiscardPath(null);
    } catch (e) {
      setDiscardError(String(e));
    }
  };

  const doCommit = async (paths: string[] | null) => {
    if (!message.trim() || saving || !path) return;
    setSaving(true);
    setBusyMessage("Committing changes — please wait…");
    setCommitError(null);
    try {
      await timed(
        path,
        "commit",
        invoke<{ id: string }>("commit_snapshot", {
          path,
          message: message.trim(),
          author: resolvedAuthor(authorName),
          paths,
        }),
        (c) => ({ commitId: c.id })
      );
      setMessage("");
      setConfirmCommit(null);
      refresh(); // refetch changes (now clean) + history
    } catch (e) {
      setCommitError(String(e));
    } finally {
      setSaving(false);
      setBusyMessage(null);
    }
  };

  // Nothing staged -> confirm committing everything. Some staged, some not -> confirm
  // committing only the staged files (the rest stay dirty). All staged -> commit right away.
  const commit = () => {
    if (!message.trim() || saving || !path) return;
    if (staged.length === 0) setConfirmCommit("none");
    else if (unstaged.length > 0) setConfirmCommit("partial");
    else doCommit(staged.map((c) => c.change.path));
  };

  if (error && items.length === 0) {
    return (
      <p className="px-3 py-2 text-[12px] text-text-muted">Couldn’t scan this folder: {error}</p>
    );
  }

  const staged = items.filter((c) => c.staged);
  const unstaged = items.filter((c) => !c.staged);

  return (
    <div className="flex flex-col">
      <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-border px-3 text-[11px] text-text-muted">
        Saving to
        <BranchBadge branch={currentBranch} />
      </div>
      <Section
        title="Staged"
        items={staged}
        action="unstage"
        onToggle={toggle}
        onToggleAll={() => setAll(false)}
        disabled={saving}
        focusedFile={focusedFile}
        onFocusFile={onFocusFile}
        onDiscardFile={setConfirmDiscardPath}
      />
      <div className="my-1 h-px bg-border" />
      <Section
        title="Changes"
        items={unstaged}
        action="stage"
        onToggle={toggle}
        onToggleAll={() => setAll(true)}
        disabled={saving}
        focusedFile={focusedFile}
        onFocusFile={onFocusFile}
        onDiscardFile={setConfirmDiscardPath}
      />

      {inTauri() && (
        <div className="mt-1 flex flex-col gap-2 border-t border-border p-3">
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Describe this version…"
            rows={2}
            className="resize-none rounded-button border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-text placeholder:text-text-muted focus:border-accent focus:outline-none"
          />
          {commitError && <p className="text-[11px] text-danger">{commitError}</p>}
          <button
            type="button"
            onClick={commit}
            disabled={!message.trim() || items.length === 0 || saving}
            className="flex items-center justify-center gap-1.5 rounded-button bg-accent/15 px-2 py-1.5 text-[12px] font-medium text-accent transition-colors hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {saving && <CircleNotch size={13} className="animate-spin" />}
            {saving ? "Saving version…" : "Commit version"}
          </button>
        </div>
      )}

      {confirmDiscardPath && (
        <Modal
          title="Discard changes to this file?"
          onClose={() => (saving ? undefined : setConfirmDiscardPath(null))}
          footer={
            <>
              <Button onClick={() => setConfirmDiscardPath(null)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={discardOne} disabled={saving}>
                {saving ? "Discarding…" : "Discard"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            This permanently reverts{" "}
            <span className="font-medium text-text">
              {artistMode ? assetName(confirmDiscardPath) : confirmDiscardPath}
            </span>{" "}
            to its last saved version. Any unsaved edits to it are lost.
          </p>
          {discardError && <p className="mt-3 text-[12px] text-danger">{discardError}</p>}
        </Modal>
      )}

      {confirmCommit && (
        <Modal
          title={confirmCommit === "none" ? "Nothing staged" : "Some changes aren't staged"}
          onClose={() => (saving ? undefined : setConfirmCommit(null))}
          footer={
            <>
              <Button onClick={() => setConfirmCommit(null)} disabled={saving}>
                Cancel
              </Button>
              <Button
                onClick={() =>
                  doCommit(confirmCommit === "none" ? null : staged.map((c) => c.change.path))
                }
                disabled={saving}
              >
                {saving ? "Saving version…" : "Commit anyway"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            {confirmCommit === "none"
              ? "You haven't staged any changes. Commit everything in Changes anyway?"
              : `${unstaged.length} unstaged file${unstaged.length === 1 ? "" : "s"} won't be included. Commit only the ${staged.length} staged file${staged.length === 1 ? "" : "s"}?`}
          </p>
          {commitError && <p className="mt-3 text-[12px] text-danger">{commitError}</p>}
        </Modal>
      )}
    </div>
  );
}
