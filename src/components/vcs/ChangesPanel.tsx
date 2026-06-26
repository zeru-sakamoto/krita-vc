import { useState } from "react";
import { Minus, Plus } from "@phosphor-icons/react";
import type { WorkingChange } from "../../types";
import { FileStatusChip } from "./FileStatusChip";

function Section({
  title,
  items,
  action,
  onToggle,
}: {
  title: string;
  items: WorkingChange[];
  action: "stage" | "unstage";
  onToggle: (path: string) => void;
}) {
  return (
    <div>
      <h3 className="flex h-8 shrink-0 items-center justify-between px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
        {title}
        <span>{items.length}</span>
      </h3>
      <ul className="flex flex-col">
        {items.map(({ change }) => (
          <li
            key={change.path}
            className="group flex items-center gap-2 px-3 py-1 hover:bg-white/5"
          >
            <FileStatusChip status={change.status} />
            <span className="selectable min-w-0 flex-1 truncate font-mono text-[12px] text-text">
              {change.path}
            </span>
            <button
              type="button"
              title={action === "stage" ? "Stage" : "Unstage"}
              aria-label={action === "stage" ? "Stage file" : "Unstage file"}
              onClick={() => onToggle(change.path)}
              className="grid h-5 w-5 shrink-0 place-items-center rounded-button text-text-muted opacity-0 transition-opacity hover:bg-white/5 hover:text-text group-hover:opacity-100"
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
 * Working-tree changes, grouped into Staged / Unstaged. Staging is mock-only
 * (toggles local state for feel) — no backend git involvement.
 */
export function ChangesPanel({ changes }: { changes: WorkingChange[] }) {
  const [items, setItems] = useState(changes);

  const toggle = (path: string) =>
    setItems((prev) => prev.map((c) => (c.change.path === path ? { ...c, staged: !c.staged } : c)));

  const staged = items.filter((c) => c.staged);
  const unstaged = items.filter((c) => !c.staged);

  return (
    <div className="flex flex-col">
      <Section title="Staged" items={staged} action="unstage" onToggle={toggle} />
      <div className="my-1 h-px bg-border" />
      <Section title="Changes" items={unstaged} action="stage" onToggle={toggle} />
    </div>
  );
}
