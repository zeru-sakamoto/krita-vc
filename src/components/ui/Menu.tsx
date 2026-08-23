import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CaretDown } from "@phosphor-icons/react";

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
  /** Greys out the row and blocks selection. */
  disabled?: boolean;
  /** Draws a divider above this row, starting a new group. Matches the `footer` rule. */
  separator?: boolean;
  /** Tour spotlight target — sets `data-tour-id` on this row's button. */
  tourId?: string;
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
  /** Which edge of the trigger the popover aligns to. Default "left". */
  align?: "left" | "right";
  /** Holds the popover open regardless of click state — used by the product tour to
   *  spotlight individual rows. ORed with the normal click-toggled state, so it never
   *  needs to fight outside-click/Escape handling. */
  forceOpen?: boolean;
  /** Stretches the trigger to the width of its container — for select-style usage. */
  fullWidth?: boolean;
  disabled?: boolean;
}

/**
 * Minimal dropdown menu: a trigger button + an absolutely-positioned list.
 * Closes on outside click or Escape. Themed per DESIGN.md (surface-2 popover,
 * hairline border, panel radius, float shadow).
 */
export function Menu({
  trigger,
  items,
  footer,
  minWidth = 200,
  align = "left",
  forceOpen,
  fullWidth,
  disabled,
}: MenuProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const open = !disabled && (forceOpen || internalOpen);
  const rootRef = useRef<HTMLDivElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  // Popover renders in a body portal (below) so it's positioned by viewport coords,
  // not clipped by / adding scroll height to a scrollable ancestor (e.g. a Modal body).
  const [pos, setPos] = useState<{
    top: number;
    left?: number;
    right?: number;
    width: number;
  } | null>(null);

  useLayoutEffect(() => {
    if (!open || !rootRef.current) return;
    const rect = rootRef.current.getBoundingClientRect();
    setPos(
      align === "right"
        ? { top: rect.bottom + 4, right: window.innerWidth - rect.right, width: rect.width }
        : { top: rect.bottom + 4, left: rect.left, width: rect.width }
    );
  }, [open, align]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (rootRef.current?.contains(target)) return;
      if (popoverRef.current?.contains(target)) return;
      setInternalOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setInternalOpen(false);
    };
    const onScroll = () => setInternalOpen(false);
    // Capture phase: a modal's backdrop stops pointerdown from bubbling past its panel
    // (so panel clicks don't also close the modal), which would otherwise swallow this
    // listener before it ever saw the click.
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    // Capture phase catches scrolling any scrollable ancestor (e.g. a Modal body),
    // which would otherwise leave the portal-positioned popover pinned to a stale spot.
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  const renderItem = (item: MenuItem) => (
    <div
      key={item.id}
      className={[
        "group relative flex items-center",
        item.separator ? "mt-1 border-t border-border pt-1" : "",
      ].join(" ")}
    >
      <button
        type="button"
        role="menuitem"
        disabled={item.disabled}
        data-tour-id={item.tourId}
        onClick={() => {
          item.onSelect();
          setInternalOpen(false);
        }}
        className={[
          "flex min-w-0 flex-1 items-center gap-2 px-2.5 py-1.5 text-left text-[13px]",
          "transition-colors hover:bg-state-hover",
          "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent",
          item.selected ? "text-accent" : "text-text",
        ].join(" ")}
      >
        {align !== "right" && (
          <span aria-hidden className="w-3 shrink-0 text-accent">
            {item.selected && "✓"}
          </span>
        )}
        {item.icon && <span className="shrink-0 text-text-muted">{item.icon}</span>}
        <span
          className={["flex min-w-0 flex-1 flex-col", align === "right" && "text-right"]
            .filter(Boolean)
            .join(" ")}
        >
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
    <div ref={rootRef} className={["relative", fullWidth ? "block" : "inline-block"].join(" ")}>
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        onClick={() => setInternalOpen((v) => !v)}
        className={fullWidth ? "block w-full disabled:cursor-not-allowed" : undefined}
      >
        {trigger(open)}
      </button>

      {open &&
        pos &&
        createPortal(
          <div
            ref={popoverRef}
            id={menuId}
            role="menu"
            style={{
              minWidth: fullWidth ? pos.width : minWidth,
              top: pos.top,
              left: pos.left,
              right: pos.right,
            }}
            className="glass fixed z-(--z-modal) overflow-hidden rounded-panel shadow-(--shadow-float)"
          >
            <div className="max-h-72 overflow-auto py-1">{items.map(renderItem)}</div>
            {footer && footer.length > 0 && (
              <div className="border-t border-border py-1">{footer.map(renderItem)}</div>
            )}
          </div>,
          document.body
        )}
    </div>
  );
}

/**
 * A `<select>` replacement built on `Menu`, so it gets the same themed popover
 * (and portal positioning — no OS-native dropdown, no scrollbar side effects
 * inside a Modal) instead of the browser's default select UI.
 */
export function Select<T extends string | number>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  const cur = options.find((o) => o.value === value);
  return (
    <Menu
      fullWidth
      disabled={disabled}
      items={options.map((o) => ({
        id: String(o.value),
        label: o.label,
        selected: o.value === value,
        onSelect: () => onChange(o.value),
      }))}
      trigger={(open) => (
        <span
          className={[
            "inset-well flex w-full items-center justify-between gap-2 rounded-button border bg-bg px-2 py-1.5 text-left text-[13px] text-text",
            disabled ? "opacity-50" : "",
            open ? "border-accent" : "border-border",
          ].join(" ")}
        >
          <span className="min-w-0 flex-1 truncate">{cur?.label ?? ""}</span>
          <CaretDown size={12} weight="bold" className="shrink-0 text-text-muted" />
        </span>
      )}
    />
  );
}
