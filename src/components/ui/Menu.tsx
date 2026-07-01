import { useEffect, useId, useRef, useState } from "react";

export interface MenuItem {
  id: string;
  label: string;
  /** Optional leading icon node. */
  icon?: React.ReactNode;
  /** Optional secondary text shown muted under the label. */
  detail?: string;
  /** Marks the currently active item (shows a check + accent text). */
  selected?: boolean;
  /** Optional trailing action (e.g. a remove button), revealed on row hover. */
  action?: React.ReactNode;
  onSelect: () => void;
}

interface MenuProps {
  /** Render-prop for the trigger; receives whether the menu is open. */
  trigger: (open: boolean) => React.ReactNode;
  items: MenuItem[];
  /** Optional sticky action rows pinned to the bottom (e.g. "Create"/"Browse"). */
  footer?: MenuItem[];
  /** Min width of the popover. */
  minWidth?: number;
}

/**
 * Minimal dropdown menu: a trigger button + an absolutely-positioned list.
 * Closes on outside click or Escape. Themed per DESIGN.md (surface-2 popover,
 * hairline border, panel radius, float shadow).
 */
export function Menu({ trigger, items, footer, minWidth = 200 }: MenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const renderItem = (item: MenuItem) => (
    <div key={item.id} className="group relative flex items-center">
      <button
        type="button"
        role="menuitem"
        onClick={() => {
          item.onSelect();
          setOpen(false);
        }}
        className={[
          "flex min-w-0 flex-1 items-center gap-2 px-2.5 py-1.5 text-left text-[13px]",
          "transition-colors hover:bg-white/5",
          item.selected ? "text-accent" : "text-text",
        ].join(" ")}
      >
        <span aria-hidden className="w-3 shrink-0 text-accent">
          {item.selected && "✓"}
        </span>
        {item.icon && <span className="shrink-0 text-text-muted">{item.icon}</span>}
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate">{item.label}</span>
          {item.detail && (
            <span className="truncate font-mono text-[10px] text-text-muted">{item.detail}</span>
          )}
        </span>
      </button>
      {item.action && (
        <span className="absolute right-1.5 opacity-0 transition-opacity group-hover:opacity-100">
          {item.action}
        </span>
      )}
    </div>
  );

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        {trigger(open)}
      </button>

      {open && (
        <div
          id={menuId}
          role="menu"
          style={{ minWidth }}
          className="absolute left-0 top-[calc(100%+4px)] z-(--z-modal) overflow-hidden rounded-panel border border-border bg-surface-2 shadow-(--shadow-float)"
        >
          <div className="max-h-72 overflow-auto py-1">{items.map(renderItem)}</div>
          {footer && footer.length > 0 && (
            <div className="border-t border-border py-1">{footer.map(renderItem)}</div>
          )}
        </div>
      )}
    </div>
  );
}
