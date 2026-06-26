import type { ReactNode } from "react";

interface DockerPanelProps {
  title: string;
  /** Right-aligned action icons in the title bar (IconButtons at size 16) */
  actions?: ReactNode;
  /** When false, the content area won't scroll (parent controls layout) */
  scroll?: boolean;
  children: ReactNode;
  className?: string;
}

/**
 * Reusable docker panel: 24px title bar + content area.
 * (DESIGN.md → Layout & App Shell → Docker / Panel System)
 */
export function DockerPanel({
  title,
  actions,
  scroll = true,
  children,
  className = "",
}: DockerPanelProps) {
  return (
    <section className={["flex min-h-0 min-w-0 flex-col bg-surface", className].join(" ")}>
      <header className="flex h-9 shrink-0 items-center justify-between border-b border-border bg-surface-2 pl-2 pr-1">
        <span className="text-[11px] font-medium uppercase tracking-wide text-text-muted">
          {title}
        </span>
        {actions && <div className="flex items-center">{actions}</div>}
      </header>
      <div className={["min-h-0 flex-1", scroll ? "overflow-auto" : ""].join(" ")}>{children}</div>
    </section>
  );
}
