import { useState } from "react";
import {
  ArrowCounterClockwise,
  CaretDown,
  Minus,
  PaintBrush,
  Plus,
  Square,
  Trash,
  X,
} from "@phosphor-icons/react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as pickArchive } from "@tauri-apps/plugin-dialog";
import { Menu, type MenuItem } from "../ui/Menu";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Radio } from "../ui/Radio";
import { RestoreModal } from "./RestoreModal";
import { Tooltip } from "../ui/Tooltip";
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

  const items: MenuItem[] = repositories.map((repo) => ({
    id: repo.id,
    label: repo.name,
    detail: repo.path,
    selected: repo.id === currentId,
    icon: <PaintBrush size={15} weight="regular" />,
    onSelect: () => setCurrent(repo.id),
    action: (
      <Tooltip label="Stop tracking this artwork">
        <button
          type="button"
          aria-label={`Stop tracking ${repo.name}`}
          onClick={(e) => {
            e.stopPropagation();
            setModal({ kind: "remove", repo });
          }}
          className="grid h-5 w-5 place-items-center rounded-button text-text-muted hover:bg-state-hover hover:text-danger disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-text-muted"
        >
          <X size={13} />
        </button>
      </Tooltip>
    ),
  }));

  const footer: MenuItem[] = [
    {
      id: "browse-repository",
      label: "Track an artwork…",
      icon: <Plus size={15} weight="regular" />,
      onSelect: browseRepository,
    },
    {
      id: "restore-backup",
      label: "Restore from a backup…",
      icon: <ArrowCounterClockwise size={15} weight="regular" />,
      separator: true,
      onSelect: restore,
    },
  ];

  return (
    <header
      className="flex h-9 shrink-0 items-center gap-1.5 bg-surface px-2"
      {...(showWindowControls ? { "data-tauri-drag-region": true } : {})}
    >
      <img src="/logo.svg" alt="" className="h-5 w-5 shrink-0" />
      <div data-tour-id="repo-switcher" className="flex items-center">
        <Menu
          trigger={() => <RepoTrigger name={current?.name ?? "Track an artwork…"} />}
          items={items}
          footer={footer}
          minWidth={240}
        />
      </div>

      {showWindowControls && <WindowControls />}

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
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted hover:bg-state-hover hover:text-text"
        >
          <Minus size={13} />
        </button>
      </Tooltip>
      <Tooltip label="Maximize">
        <button
          type="button"
          aria-label="Maximize"
          onClick={() => win.toggleMaximize()}
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted hover:bg-state-hover hover:text-text"
        >
          <Square size={11} />
        </button>
      </Tooltip>
      <Tooltip label="Close">
        <button
          type="button"
          aria-label="Close"
          onClick={() => win.close()}
          className="grid h-7 w-8 place-items-center rounded-button text-text-muted hover:bg-danger hover:text-bg"
        >
          <X size={13} />
        </button>
      </Tooltip>
    </div>
  );
}

function RepoTrigger({ name }: { name: string }) {
  return (
    <Tooltip label="Switch artwork">
      <span className="flex items-center gap-1.5 rounded-button px-2 py-1 text-[13px] text-text transition-colors hover:bg-state-hover">
        <PaintBrush size={15} weight="regular" className="text-text-muted" />
        <span className="max-w-55 truncate font-medium">{name}</span>
        <CaretDown size={12} className="text-text-muted" />
      </span>
    </Tooltip>
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
        footer={<Button onClick={onClose}>Close</Button>}
      >
        <p className="text-[13px] text-text">
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
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant={deleteHistory ? "destructive" : "default"}
            disabled={!canConfirm}
            onClick={remove}
          >
            {deleteHistory ? <Trash size={14} /> : null}
            {deleteHistory ? "Delete history" : "Remove from list"}
          </Button>
        </>
      }
    >
      <fieldset className="flex flex-col gap-2">
        <label className="flex items-start gap-2 text-[13px] text-text">
          <Radio
            name="remove-mode"
            checked={!deleteHistory}
            onChange={() => setDeleteHistory(false)}
            className="mt-0.5"
          />
          <span>
            Remove from list only
            <span className="block text-[11px] text-text-muted">
              Forgets this artwork here. The file and its version history stay on disk, and you can
              add it back at any time.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-2 text-[13px] text-text">
          <Radio
            name="remove-mode"
            checked={deleteHistory}
            onChange={() => setDeleteHistory(true)}
            tone="danger"
            className="mt-0.5"
          />
          <span>
            Delete version history
            <span className="block text-[11px] text-text-muted">
              Moves every saved version of this artwork to the Recycle Bin. The artwork file itself
              is never touched — only its history.
            </span>
          </span>
        </label>
      </fieldset>

      {deleteHistory && (
        <div className="mt-3">
          <label className="mb-1 block text-[12px] text-text-muted">
            Type <span className="font-mono text-text">{repo.name}</span> to confirm:
          </label>
          <input
            autoFocus
            value={confirmName}
            onChange={(e) => setConfirmName(e.target.value)}
            placeholder={repo.name}
            className="w-full inset-well rounded-button border border-border bg-bg px-2 py-1.5 font-mono text-[12px] text-text placeholder:text-text-muted focus:border-danger !outline-none"
          />
        </div>
      )}
    </Modal>
  );
}
