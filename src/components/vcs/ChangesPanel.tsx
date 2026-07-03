import { useEffect, useState } from "react";
import { CircleNotch, Minus, Plus } from "@phosphor-icons/react";
import { invoke } from "@tauri-apps/api/core";
import type { WorkingChange } from "../../types";
import { FileStatusChip } from "./FileStatusChip";
import { useRepository } from "../../lib/repository";
import { inTauri } from "../../lib/tauri";

function Section({
  title,
  items,
  action,
  onToggle,
  onToggleAll,
  disabled,
  focusedFile,
  onFocusFile,
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
  focusedFile,
  onFocusFile,
}: {
  focusedFile: string | null;
  onFocusFile: (path: string) => void;
}) {
  const { current, refreshNonce, refresh, saving, setSaving, setScanning } = useRepository();
  const [items, setItems] = useState<WorkingChange[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("");

  const path = current?.path ?? null;

  useEffect(() => {
    // No backend in a plain browser (`npm run dev`), nothing to scan without a repo.
    if (!inTauri() || !path) {
      setItems([]);
      return;
    }
    let cancelled = false;
    setScanning(true);
    invoke<WorkingChange[]>("scan_repository", { path })
      .then((changes) => {
        if (!cancelled) {
          setItems(changes);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setItems([]);
          setError(String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setScanning(false);
      });
    return () => {
      cancelled = true;
    };
  }, [path, refreshNonce, setScanning]);

  const toggle = (path: string) =>
    setItems((prev) => prev.map((c) => (c.change.path === path ? { ...c, staged: !c.staged } : c)));
  const setAll = (staged: boolean) => setItems((prev) => prev.map((c) => ({ ...c, staged })));

  const commit = async () => {
    if (!message.trim() || saving || !path) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("commit_snapshot", {
        path,
        message: message.trim(),
        author: "You",
      });
      setMessage("");
      refresh(); // refetch changes (now clean) + history
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
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
      <Section
        title="Staged"
        items={staged}
        action="unstage"
        onToggle={toggle}
        onToggleAll={() => setAll(false)}
        disabled={saving}
        focusedFile={focusedFile}
        onFocusFile={onFocusFile}
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
          {error && <p className="text-[11px] text-danger">{error}</p>}
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
    </div>
  );
}
