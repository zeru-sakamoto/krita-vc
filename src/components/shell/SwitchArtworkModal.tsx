import { useState } from "react";
import { ArrowCounterClockwise, PaintBrush, Plus, X } from "@phosphor-icons/react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Tooltip } from "../ui/Tooltip";
import { ICON } from "../../lib/iconSize";
import type { Repository } from "../../types";

interface SwitchArtworkModalProps {
  repositories: Repository[];
  currentId: string;
  onSelect: (id: string) => void;
  onRemove: (repo: Repository) => void;
  onBrowse: () => void;
  onRestore: () => void;
  onClose: () => void;
}

/**
 * Replaces the old Menu-dropdown artwork switcher for a searchable, scrollable list —
 * a Modal has real headroom, unlike a popover capped to the trigger's neighborhood.
 */
export function SwitchArtworkModal({
  repositories,
  currentId,
  onSelect,
  onRemove,
  onBrowse,
  onRestore,
  onClose,
}: SwitchArtworkModalProps) {
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const filtered = q
    ? repositories.filter(
        (r) => r.name.toLowerCase().includes(q) || r.path.toLowerCase().includes(q)
      )
    : repositories;

  return (
    <Modal
      title="Switch artwork"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onBrowse}>
            <Plus size={ICON.dense} />
            Track an artwork…
          </Button>
          <Button onClick={onRestore}>
            <ArrowCounterClockwise size={ICON.dense} />
            Restore from a backup…
          </Button>
        </>
      }
    >
      <input
        type="text"
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search artworks…"
        // `!outline-none` stays: global.css's `:focus-visible` outline is unlayered CSS and
        // beats Tailwind's utilities layer, so plain `outline-none` would lose to it.
        className="w-full inset-well rounded-button border border-border bg-bg px-2 py-1.5 text-dense text-text placeholder:text-text-muted transition-[color,background-color,border-color] duration-(--dur-fast) ease-(--ease-out) focus-visible:border-accent focus-visible:bg-surface !outline-none"
      />

      {repositories.length === 0 ? (
        <p className="mt-3 text-dense text-text-muted">You aren’t tracking any artworks yet.</p>
      ) : filtered.length === 0 ? (
        <p className="mt-3 text-dense text-text-muted">No artworks match “{query}”.</p>
      ) : (
        <ul className="-mx-1 mt-2 max-h-72 overflow-auto">
          {filtered.map((repo) => {
            const selected = repo.id === currentId;
            return (
              <li key={repo.id} className="group relative flex items-center">
                <button
                  type="button"
                  onClick={() => onSelect(repo.id)}
                  className={[
                    "flex min-w-0 flex-1 items-center gap-2 rounded-button px-2.5 py-1.5 text-left text-body",
                    "transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-state-hover",
                    selected ? "text-accent" : "text-text",
                  ].join(" ")}
                >
                  <span aria-hidden className="w-3 shrink-0 text-accent">
                    {selected && "✓"}
                  </span>
                  <PaintBrush
                    size={ICON.dense}
                    weight="regular"
                    className="shrink-0 text-text-muted"
                  />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate">{repo.name}</span>
                    <span className="truncate font-mono text-micro text-text-muted">
                      {repo.path}
                    </span>
                  </span>
                </button>
                <span className="absolute right-1.5 opacity-0 transition-opacity duration-(--dur-fast) ease-(--ease-out) group-hover:opacity-100">
                  <Tooltip label="Stop tracking this artwork">
                    <button
                      type="button"
                      aria-label={`Stop tracking ${repo.name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onRemove(repo);
                      }}
                      className="grid h-5 w-5 place-items-center rounded-button text-text-muted transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-state-hover hover:text-danger"
                    >
                      <X size={ICON.inline} />
                    </button>
                  </Tooltip>
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </Modal>
  );
}
