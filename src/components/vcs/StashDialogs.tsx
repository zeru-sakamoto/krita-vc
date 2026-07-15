import { useState } from "react";
import { Archive, ArrowLineUp } from "@phosphor-icons/react";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";
import { relativeTime } from "../../lib/format";
import { assetName } from "../../lib/friendly";
import { errorText } from "./BranchDialogs";
import type { Stash } from "../../types";

/**
 * Shared set-aside ("stash") dialogs, used by the Changes panel menu, the Branches panel's
 * save-first prompt, and Settings. Copy follows Artist Mode: plain language on, VCS terms off.
 */

/** The backend's stash-conflict guard — matched on its stable message prefix, like the
 *  dirty-tree one. Deliberately distinct so the two prompts never cross-fire. */
export function isStashConflictError(e: unknown): boolean {
  return String(e).startsWith("stash conflict");
}

/** What a set-aside action should capture: just the staged files, or every working change. */
export type StashScope = "staged" | "all";

/** One-line summary of a stash's contents: "3 files · on main · 2h ago". */
export function stashSummary(s: Stash): string {
  const n = s.changes.length;
  return `${n} ${n === 1 ? "file" : "files"} · on ${s.branch} · ${relativeTime(s.timestamp)}`;
}

/** A stash's display name — its label, or the files it holds when it was never labelled. */
export function stashTitle(s: Stash): string {
  if (s.label.trim()) return s.label;
  const names = s.changes.map((c) => assetName(c.path));
  if (names.length === 0) return "Empty";
  if (names.length <= 2) return names.join(", ");
  return `${names.slice(0, 2).join(", ")} +${names.length - 2}`;
}

/**
 * Label-and-confirm dialog for setting work aside. `paths` is null for "everything", else the
 * exact relative paths to capture. On success the files are reverted on disk — the copy says so,
 * because that's the part a user can't undo by closing a dialog.
 */
export function SetAsideModal({
  scope,
  paths,
  onClose,
  onDone,
}: {
  scope: StashScope;
  /** Relative paths to set aside; null means every dirty file. */
  paths: string[] | null;
  onClose: () => void;
  /** Fired after a successful set-aside — lets a caller retry what it was blocked on. */
  onDone?: () => void;
}) {
  const { createStash, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const [label, setLabel] = useState("");
  const [error, setError] = useState<string | null>(null);

  const count = paths?.length;
  const submit = async () => {
    setError(null);
    try {
      await createStash(label.trim(), paths);
      onClose();
      onDone?.();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const scopeText = artistMode
    ? { staged: "the files you've chosen", all: "everything you've changed" }
    : { staged: "staged changes", all: "all working-tree changes" };

  return (
    <Modal
      title={artistMode ? "Set your work aside" : "Stash changes"}
      onClose={() => (saving ? undefined : onClose())}
      footer={
        <>
          <Button onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={saving}>
            {saving ? "Setting aside…" : artistMode ? "Set aside" : "Stash"}
          </Button>
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode ? (
          <>
            This tucks away {scopeText[scope]} and puts those files back the way they were at your
            last version. Nothing is lost — bring it back whenever you like.
          </>
        ) : (
          <>
            Stashes {scopeText[scope]} and reverts those files to the current tip. Restore them
            later from the panel menu or Settings.
          </>
        )}
      </p>
      <input
        type="text"
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !saving) void submit();
        }}
        placeholder={artistMode ? "What's this? (optional)" : "message (optional)"}
        autoFocus
        className="mt-3 w-full rounded-button border border-border bg-surface-2 px-2.5 py-1.5 text-[13px] text-text placeholder:text-text-muted focus:border-accent focus:outline-none"
      />
      {count != null && (
        <p className="mt-2 text-[11px] text-text-muted">
          {count} {count === 1 ? "file" : "files"}
        </p>
      )}
      {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
    </Modal>
  );
}

/** Pick which stash to bring back. */
export function PickStashModal({
  stashes,
  onClose,
  onPick,
}: {
  stashes: Stash[];
  onClose: () => void;
  onPick: (id: string) => void;
}) {
  const { saving } = useRepository();
  const { artistMode } = useArtistMode();
  return (
    <Modal
      title={artistMode ? "Bring back work you set aside" : "Pop a stash"}
      onClose={() => (saving ? undefined : onClose())}
      footer={
        <Button onClick={onClose} disabled={saving}>
          Cancel
        </Button>
      }
    >
      <p className="mb-3 text-[13px] leading-relaxed text-text-muted">
        {artistMode
          ? "Choose what to bring back. Its files return as unsaved work, and it leaves the shelf."
          : "Restores the stash's files into the working tree and drops it from the list."}
      </p>
      <ul className="-mx-1 max-h-64 overflow-auto">
        {stashes.map((s) => (
          <li key={s.id}>
            <button
              type="button"
              disabled={saving}
              onClick={() => onPick(s.id)}
              className="flex w-full flex-col items-start gap-0.5 rounded-button px-2 py-1.5 text-left transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <span className="truncate text-[13px] text-text">{stashTitle(s)}</span>
              <span className="truncate text-[11px] text-text-muted">{stashSummary(s)}</span>
            </button>
          </li>
        ))}
      </ul>
    </Modal>
  );
}

/**
 * Shown when bringing a stash back would clobber newer work. The stash is untouched, so the way
 * out is to deal with the working file first — hence the jump to Changes.
 */
export function StashConflictModal({
  message,
  onClose,
  onShowChanges,
}: {
  message: string;
  onClose: () => void;
  onShowChanges?: () => void;
}) {
  const { artistMode } = useArtistMode();
  // The backend names the files after its stable prefix — show just that part.
  const files = message.replace(/^stash conflict:\s*/, "").replace(/\s*changed since.*$/, "");
  return (
    <Modal
      title={artistMode ? "Can't bring this back yet" : "Stash conflict"}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          {onShowChanges && (
            <Button
              variant="primary"
              onClick={() => {
                onClose();
                onShowChanges();
              }}
            >
              Go to Changes
            </Button>
          )}
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode ? (
          <>
            You've changed <span className="text-text">{files}</span> since setting this aside.
            Bringing it back would paint over that newer work, so save or discard it first — your
            set-aside work is still safe on the shelf.
          </>
        ) : (
          <>
            <span className="text-text">{files}</span> has uncommitted changes. Commit or discard
            them before popping this stash; the stash is left intact.
          </>
        )}
      </p>
    </Modal>
  );
}

/** Icons re-exported so menu call sites don't each pick their own. Bringing work back must not
 *  reuse undo's ⤺ — they sit in the same menu and are entirely different actions. */
export const StashIcon = Archive;
export const UnstashIcon = ArrowLineUp;
