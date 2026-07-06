import { useEffect, useState } from "react";
import { Broom, CaretDown, FolderOpen, FolderPlus, Plus, Trash, X } from "@phosphor-icons/react";
import { open } from "@tauri-apps/plugin-dialog";
import { Menu, type MenuItem } from "../ui/Menu";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { useRepository, type CleanupReport } from "../../lib/repository";
import { inTauri } from "../../lib/tauri";
import type { Repository } from "../../types";

/**
 * Slim top bar spanning the window. Hosts the repository switcher (a folder the
 * user has designated). Local-only VCS — no remote/fetch/push affordances.
 * (DESIGN.md → Layout & App Shell → Top bar)
 */
export function TopBar() {
  const { repositories, current, currentId, setCurrent, browseRepository } = useRepository();
  const [modal, setModal] = useState<
    { kind: "create" } | { kind: "remove"; repo: Repository } | { kind: "cleanup" } | null
  >(null);

  const items: MenuItem[] = repositories.map((repo) => ({
    id: repo.id,
    label: repo.name,
    detail: repo.path,
    selected: repo.id === currentId,
    icon: <FolderOpen size={15} weight="regular" />,
    onSelect: () => setCurrent(repo.id),
    action: (
      <button
        type="button"
        title="Remove repository"
        aria-label={`Remove ${repo.name}`}
        onClick={(e) => {
          e.stopPropagation();
          setModal({ kind: "remove", repo });
        }}
        className="grid h-5 w-5 place-items-center rounded-button text-text-muted hover:bg-white/5 hover:text-danger disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-text-muted"
      >
        <X size={13} />
      </button>
    ),
  }));

  const footer: MenuItem[] = [
    {
      id: "create-repository",
      label: "Create repository",
      icon: <FolderPlus size={15} weight="regular" />,
      onSelect: () => setModal({ kind: "create" }),
    },
    {
      id: "browse-repository",
      label: "Browse existing repository…",
      icon: <Plus size={15} weight="regular" />,
      onSelect: browseRepository,
    },
    ...(current
      ? [
          {
            id: "cleanup-repository",
            label: "Clean up storage…",
            icon: <Broom size={15} weight="regular" />,
            onSelect: () => setModal({ kind: "cleanup" }),
          } satisfies MenuItem,
        ]
      : []),
  ];

  return (
    <header className="flex h-9 shrink-0 items-center border-b border-border bg-surface px-2">
      <Menu
        trigger={() => <RepoTrigger name={current?.name ?? "Open a repository…"} />}
        items={items}
        footer={footer}
        minWidth={240}
      />

      {modal?.kind === "create" && <CreateRepoModal onClose={() => setModal(null)} />}
      {modal?.kind === "remove" && (
        <RemoveRepoModal repo={modal.repo} onClose={() => setModal(null)} />
      )}
      {modal?.kind === "cleanup" && <CleanupModal onClose={() => setModal(null)} />}
    </header>
  );
}

function RepoTrigger({ name }: { name: string }) {
  return (
    <span
      title="Switch repository"
      className="flex items-center gap-1.5 rounded-button px-2 py-1 text-[13px] text-text transition-colors hover:bg-white/5"
    >
      <FolderOpen size={15} weight="regular" className="text-text-muted" />
      <span className="max-w-55 truncate font-medium">{name}</span>
      <CaretDown size={12} className="text-text-muted" />
    </span>
  );
}

function CreateRepoModal({ onClose }: { onClose: () => void }) {
  const { createRepository } = useRepository();
  const [name, setName] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pickParent = async () => {
    // No native picker outside the desktop shell.
    if (!inTauri()) return;
    const picked = await open({ directory: true, title: "Choose where to create the repository" });
    if (typeof picked === "string") setParent(picked);
  };

  const create = async () => {
    if (!name.trim() || !parent || busy) return;
    setBusy(true);
    try {
      await createRepository(parent, name);
      onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title="Create repository"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!name.trim() || !parent || busy} onClick={create}>
            {busy ? "Creating…" : "Create"}
          </Button>
        </>
      }
    >
      <label className="mb-1 block text-[12px] text-text-muted">Repository name</label>
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="my-illustration"
        className="mb-3 w-full rounded-button border border-border bg-surface-2 px-2 py-1.5 text-[13px] text-text placeholder:text-text-muted focus:border-accent focus:outline-none"
      />
      <label className="mb-1 block text-[12px] text-text-muted">Location</label>
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate rounded-button border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-text">
          {parent ? `${parent}` : <span className="text-text-muted">No folder chosen</span>}
        </span>
        <Button onClick={pickParent}>Choose…</Button>
      </div>
      {parent && name.trim() && (
        <p className="mt-2 truncate font-mono text-[11px] text-text-muted">
          Creates: {parent}/{name.trim()}
        </p>
      )}
    </Modal>
  );
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${Math.round(n / 1024)} KB`;
  return `${n} B`;
}

/**
 * "Clean up storage": a dry run on open shows what a real pass would free (space held by
 * versions no branch can reach — leftovers of undo and deleted branches), then one confirm
 * runs it for real. Cleaning never touches current artwork or any version still in history.
 */
function CleanupModal({ onClose }: { onClose: () => void }) {
  const { cleanupRepository } = useRepository();
  const [preview, setPreview] = useState<CleanupReport | null>(null);
  const [result, setResult] = useState<CleanupReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    cleanupRepository(true)
      .then((r) => {
        if (!cancelled) setPreview(r);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [cleanupRepository]);

  const clean = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await cleanupRepository(false));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const totalOf = (r: CleanupReport) => r.bytesReclaimed + r.cacheBytesReclaimed;
  const nothing = preview != null && totalOf(preview) === 0;

  return (
    <Modal
      title="Clean up storage"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{result ? "Done" : "Cancel"}</Button>
          {!result && (
            <Button variant="primary" disabled={busy || preview == null || nothing} onClick={clean}>
              {busy ? "Cleaning…" : "Clean up"}
            </Button>
          )}
        </>
      }
    >
      <p className="mb-2 text-[13px] text-text">
        Frees space held by versions no longer part of any history — leftovers from undone saves and
        deleted branches. Your current artwork and every version you can still see are never
        touched.
      </p>
      {error && <p className="text-[12px] text-danger">{error}</p>}
      {!error && result ? (
        <p className="text-[13px] text-text">
          Freed <span className="font-medium">{formatBytes(totalOf(result))}</span>
          {result.cacheBytesReclaimed > 0 && (
            <span className="text-text-muted">
              {" "}
              (including {formatBytes(result.cacheBytesReclaimed)} of preview images that can be
              regenerated)
            </span>
          )}
          .
        </p>
      ) : !error && preview == null ? (
        <p className="text-[12px] text-text-muted">Checking what can be cleaned…</p>
      ) : !error && nothing ? (
        <p className="text-[12px] text-text-muted">Nothing to clean up — storage is tidy.</p>
      ) : (
        !error && (
          <p className="text-[13px] text-text">
            About <span className="font-medium">{formatBytes(totalOf(preview!))}</span> can be freed
            {preview!.cacheBytesReclaimed > 0 && (
              <span className="text-text-muted">
                , including preview images that can be regenerated
              </span>
            )}
            .
          </p>
        )
      )}
    </Modal>
  );
}

function RemoveRepoModal({ repo, onClose }: { repo: Repository; onClose: () => void }) {
  const { removeRepository } = useRepository();
  const [deleteFolder, setDeleteFolder] = useState(false);
  const [confirmPath, setConfirmPath] = useState("");
  const [busy, setBusy] = useState(false);

  // ponytail: last two path segments (parent\repo) — enough to confirm intent without typing the full path
  const shortPath = repo.path.split(/[\\/]/).filter(Boolean).slice(-2).join("\\");
  const canConfirm = !busy && (!deleteFolder || confirmPath.replace(/\//g, "\\") === shortPath);

  const remove = async () => {
    if (!canConfirm) return;
    setBusy(true);
    try {
      await removeRepository(repo.id, deleteFolder);
      onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={`Remove “${repo.name}”?`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant={deleteFolder ? "destructive" : "default"}
            disabled={!canConfirm}
            onClick={remove}
          >
            {deleteFolder ? <Trash size={14} /> : null}
            {deleteFolder ? "Delete permanently" : "Remove from list"}
          </Button>
        </>
      }
    >
      <fieldset className="flex flex-col gap-2">
        <label className="flex items-start gap-2 text-[13px] text-text">
          <input
            type="radio"
            name="remove-mode"
            checked={!deleteFolder}
            onChange={() => setDeleteFolder(false)}
            className="mt-1 accent-accent"
          />
          <span>
            Remove from list only
            <span className="block text-[11px] text-text-muted">
              Forgets this repository here. Files and version history stay on disk.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-2 text-[13px] text-text">
          <input
            type="radio"
            name="remove-mode"
            checked={deleteFolder}
            onChange={() => setDeleteFolder(true)}
            className="mt-1 accent-danger"
          />
          <span>
            Delete folder permanently
            <span className="block text-[11px] text-text-muted">
              Deletes the entire folder and all its contents. This cannot be undone.
            </span>
          </span>
        </label>
      </fieldset>

      {deleteFolder && (
        <div className="mt-3">
          <label className="mb-1 block text-[12px] text-text-muted">
            Type <span className="font-mono text-text">{shortPath}</span> to confirm:
          </label>
          <input
            autoFocus
            value={confirmPath}
            onChange={(e) => setConfirmPath(e.target.value)}
            placeholder={shortPath}
            className="w-full rounded-button border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-text placeholder:text-text-muted focus:border-danger focus:outline-none"
          />
        </div>
      )}
    </Modal>
  );
}
