import { useState } from "react";
import { Minus, PaintBrush, Square, Trash, X } from "@phosphor-icons/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as pickArchive } from "@tauri-apps/plugin-dialog";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Radio } from "../ui/Radio";
import { RestoreModal } from "./RestoreModal";
import { SwitchArtworkModal } from "./SwitchArtworkModal";
import { Tooltip } from "../ui/Tooltip";
import { ICON } from "../../lib/iconSize";
import { useRepository } from "../../lib/repository";
import { useWindowChrome } from "../../lib/windowChrome";
import { inTauri } from "../../lib/tauri";
import type { Repository } from "../../types";

/**
 * Slim top bar spanning the window. Hosts the artwork switcher — one `.kra` the user has
 * chosen to track. Local-only VCS — no remote/fetch/push affordances.
 * (DESIGN.md → Layout & App Shell → Top bar)
 */
export function TopBar() {
  const { repositories, current, currentId, setCurrent, browseRepository } = useRepository();
  const { customTitleBar } = useWindowChrome();
  const [modal, setModal] = useState<
    { kind: "remove"; repo: Repository } | { kind: "restore"; archive: string } | null
  >(null);
  // Separate from `modal`: the remove-confirm must be able to stack on top of this still-open
  // list (Modal has no portal, so two mounted Modals already stack correctly as DOM siblings —
  // same as RestoreModal -> RestoreCompareModal), which a single discriminated union can't do.
  const [switchOpen, setSwitchOpen] = useState(false);

  // Restoring is how someone gets their artworks back after a reinstall or a lost drive, so it
  // sits next to "Track an artwork…" rather than buried in Settings.
  const restore = async () => {
    if (!inTauri()) return;
    const picked = await pickArchive({
      title: "Choose a backup to restore",
      filters: [{ name: "Backup archive", extensions: ["zip"] }],
    });
    if (typeof picked === "string") setModal({ kind: "restore", archive: picked });
  };
  const showWindowControls = customTitleBar && inTauri();

  return (
    <header
      className="flex h-11 shrink-0 items-center bg-surface pr-2"
      {...(showWindowControls ? { "data-tauri-drag-region": true } : {})}
    >
      {/* w-12 matches ActivityBar's width so the logo centers above the nav icons below it,
          and the switcher after it starts flush with the well's left inset (Sidebar content). */}
      <div className="grid w-12 shrink-0 place-items-center">
        <img src="/logo.svg" alt="" className="h-5 w-5" />
      </div>
      <Tooltip label="Switch artwork">
        <button
          type="button"
          data-tour-id="repo-switcher"
          onClick={() => setSwitchOpen(true)}
          // ml-2 matches the well's own p-2 inset (AppShell), so this chip's left edge lines
          // up with the panel cards below it instead of sitting flush with the logo column.
          className="tactile ml-2 flex items-center gap-1.5 rounded-button bg-surface-2 px-2.5 py-1.5 text-body text-text transition-[translate,background-color,box-shadow,color] duration-(--dur-instant) ease-(--ease-out) hover:bg-surface-3 active:translate-y-px"
        >
          <PaintBrush size={ICON.dense} weight="regular" className="text-text-muted" />
          <span className="max-w-55 truncate font-medium">
            {current?.name ?? "Track an artwork…"}
          </span>
        </button>
      </Tooltip>

      {showWindowControls && <WindowControls />}

      {switchOpen && (
        <SwitchArtworkModal
          repositories={repositories}
          currentId={currentId}
          onSelect={(id) => {
            setCurrent(id);
            setSwitchOpen(false);
          }}
          onRemove={(repo) => setModal({ kind: "remove", repo })}
          onBrowse={() => {
            setSwitchOpen(false);
            browseRepository();
          }}
          onRestore={() => {
            setSwitchOpen(false);
            restore();
          }}
          onClose={() => setSwitchOpen(false)}
        />
      )}
      {modal?.kind === "remove" && (
        <RemoveRepoModal repo={modal.repo} onClose={() => setModal(null)} />
      )}
      {modal?.kind === "restore" && (
        <RestoreModal archive={modal.archive} onClose={() => setModal(null)} />
      )}
    </header>
  );
}

/** Minimize/maximize/close buttons for the custom title bar (Settings → "Custom title bar"). */
function WindowControls() {
  const win = getCurrentWindow();
  return (
    <div className="ml-auto flex items-center gap-0.5">
      <Tooltip label="Minimize">
        <button
          type="button"
          aria-label="Minimize"
          onClick={() => win.minimize()}
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-state-hover hover:text-text"
        >
          <Minus size={ICON.inline} />
        </button>
      </Tooltip>
      <Tooltip label="Maximize">
        <button
          type="button"
          aria-label="Maximize"
          onClick={() => win.toggleMaximize()}
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-state-hover hover:text-text"
        >
          <Square size={ICON.inline} />
        </button>
      </Tooltip>
      <Tooltip label="Close">
        <button
          type="button"
          aria-label="Close"
          onClick={() => win.close()}
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-danger hover:text-bg"
        >
          <X size={ICON.inline} />
        </button>
      </Tooltip>
    </div>
  );
}

function RemoveRepoModal({ repo, onClose }: { repo: Repository; onClose: () => void }) {
  const { removeRepository } = useRepository();
  const [deleteHistory, setDeleteHistory] = useState(false);
  const [confirmName, setConfirmName] = useState("");
  const [busy, setBusy] = useState(false);
  // Set only if the delete fell back to a permanent one (Recycle Bin unavailable) — then the
  // modal stays open to surface that instead of closing silently.
  const [fallbackWarning, setFallbackWarning] = useState(false);

  // Typing the artwork's name is the confirmation. Deleting history is unrecoverable and the
  // artwork keeps looking perfectly fine afterwards, so there's nothing to notice if it was a
  // misclick — unlike the folder delete this replaced, which took the art with it and so at
  // least announced itself.
  const canConfirm = !busy && (!deleteHistory || confirmName.trim() === repo.name);

  const remove = async () => {
    if (!canConfirm) return;
    setBusy(true);
    try {
      const usedTrash = await removeRepository(repo.id, deleteHistory);
      if (deleteHistory && !usedTrash) {
        setFallbackWarning(true);
      } else {
        onClose();
      }
    } finally {
      setBusy(false);
    }
  };

  if (fallbackWarning) {
    return (
      <Modal
        title={`“${repo.name}” removed`}
        onClose={onClose}
        footer={(close) => <Button onClick={close}>Close</Button>}
      >
        <p className="text-body text-text">
          The Recycle Bin wasn’t available, so the version history was deleted permanently instead
          of moved there. It can’t be recovered from Explorer. Your artwork file is untouched.
        </p>
      </Modal>
    );
  }

  return (
    <Modal
      title={`Stop tracking “${repo.name}”?`}
      onClose={onClose}
      footer={(close) => (
        <>
          <Button onClick={close}>Cancel</Button>
          <Button
            variant={deleteHistory ? "destructive" : "default"}
            disabled={!canConfirm}
            onClick={remove}
          >
            {deleteHistory ? <Trash size={ICON.dense} /> : null}
            {deleteHistory ? "Delete history" : "Remove from list"}
          </Button>
        </>
      )}
    >
      <fieldset className="flex flex-col gap-2">
        <label className="flex items-start gap-2 text-body text-text">
          <Radio
            name="remove-mode"
            checked={!deleteHistory}
            onChange={() => setDeleteHistory(false)}
            className="mt-0.5"
          />
          <span>
            Remove from list only
            <span className="block text-caption text-text-muted">
              Forgets this artwork here. The file and its version history stay on disk, and you can
              add it back at any time.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-2 text-body text-text">
          <Radio
            name="remove-mode"
            checked={deleteHistory}
            onChange={() => setDeleteHistory(true)}
            tone="danger"
            className="mt-0.5"
          />
          <span>
            Delete version history
            <span className="block text-caption text-text-muted">
              Moves every saved version of this artwork to the Recycle Bin. The artwork file itself
              is never touched — only its history.
            </span>
          </span>
        </label>
      </fieldset>

      {deleteHistory && (
        <div className="mt-3">
          <label className="mb-1 block text-dense text-text-muted">
            Type <span className="font-mono text-text">{repo.name}</span> to confirm:
          </label>
          <input
            autoFocus
            value={confirmName}
            onChange={(e) => setConfirmName(e.target.value)}
            placeholder={repo.name}
            // `!outline-none` stays: global.css's `:focus-visible` outline is unlayered CSS and beats
            // Tailwind's utilities layer, so plain `outline-none` would lose to it. The accent/danger
            // border is the sanctioned .inset-well substitute (DESIGN.md § Focus Ring Spec), paired
            // with a background step so the state survives being seen in grayscale.
            className="w-full inset-well rounded-button border border-border bg-bg px-2 py-1.5 font-mono text-dense text-text placeholder:text-text-muted transition-[color,background-color,border-color] duration-(--dur-fast) ease-(--ease-out) focus-visible:border-danger focus-visible:bg-surface !outline-none"
          />
        </div>
      )}
    </Modal>
  );
}
