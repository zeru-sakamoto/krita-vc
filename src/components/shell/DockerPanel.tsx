import type { ReactNode } from "react";

/**
 * Horizontal inset of the header strip.
 *
 * `"default"` (`pl-3 pr-1`) is the docker title bar: a text label needs the
 * 12px inset, right-aligned icon buttons bring their own. `"tight"` (`px-1`)
 * is for a header whose *first* element is itself a button — a back arrow or a
 * toolbar — since the button's own padding already supplies the inset and
 * `pl-3` on top of it reads as a stray indent.
 */
export type PanelHeaderPad = "default" | "tight";

export interface PanelHeaderProps {
  /**
   * The card's uppercase caption label.
   *
   * A `string`, deliberately — this slot is a *name for the region*, not
   * content. Anything richer belongs in `leading` or `meta`, which is what
   * keeps the label step (see below) from being spent on a commit message.
   */
  title?: string;
  /** Rendered before the title: a back button, or a whole toolbar. */
  leading?: ReactNode;
  /**
   * Free-form content after the title, in the flexible middle of the bar:
   * a trailing count ("12 versions"), a commit hash + message, a placeholder.
   * This slot is what makes the bar hold *content* rather than only a label,
   * and it is the one that expands — so `actions` stays flush right whether it
   * is filled or not.
   */
  meta?: ReactNode;
  /** Right-aligned action icons in the title bar (IconButtons at size 16) */
  actions?: ReactNode;
  /** Horizontal inset; see {@link PanelHeaderPad}. Default `"default"`. */
  pad?: PanelHeaderPad;
}

interface DockerPanelProps extends PanelHeaderProps {
  /** When false, the content area won't scroll (parent controls layout) */
  scroll?: boolean;
  children: ReactNode;
  /** Applied to the card, not the header. */
  className?: string;
}

const PAD: Record<PanelHeaderPad, string> = {
  default: "pl-3 pr-1",
  tight: "px-1",
};

/**
 * The 40px docker title bar — the card header, standalone.
 *
 * Exported because several cards own their own container (the main panel, the
 * inspector, the version map) or have no card at all (the diff toolbar), so
 * they need the header chrome without `DockerPanel`'s section wrapper. It was
 * hand-retyped at five such sites and had already diverged on horizontal
 * padding; this is the one definition. `DockerPanel` composes it.
 *
 * Layout: `leading` · `title` · `meta` (flexible — the spacer, present even
 * when empty, which is what keeps `actions` flush right) · `actions`. Each
 * slot is a `ReactNode`; a caller whose slot needs internal spacing other than
 * the bar's own supplies its own flex container rather than a prop here.
 *
 * **The uppercase-caption `title` treatment is reserved for this level — the
 * card header.** In-card section headings ("STORAGE SAVED", "LAYERS") and
 * sub-labels inside a section ("BEFORE" / "AFTER") must *not* re-use it: they
 * take the `subheading` step (`text-body font-medium`, per DESIGN.md → Type
 * Scale). Three nesting levels sharing one style is why those screens have no
 * typographic signal for which label owns which region.
 *
 * (DESIGN.md → Layout & App Shell → Docker / Panel System)
 */
export function PanelHeader({ title, leading, meta, actions, pad = "default" }: PanelHeaderProps) {
  return (
    <header
      className={[
        "flex h-10 shrink-0 items-center gap-2 border-b border-border bg-surface-2",
        PAD[pad],
      ].join(" ")}
    >
      {leading}
      {title && (
        <span className="shrink-0 text-caption font-medium uppercase tracking-wide text-text-muted">
          {title}
        </span>
      )}
      <div className="flex min-w-0 flex-1 items-center gap-2">{meta}</div>
      {actions && <div className="flex shrink-0 items-center gap-1">{actions}</div>}
    </header>
  );
}

/**
 * Reusable docker panel: 40px title bar + content area.
 * Also the bento card — rounded, raised off the well, clipping its own
 * content so the header's fill follows the top corners.
 *
 * Header slots are forwarded to {@link PanelHeader}; see it for the rule on
 * which labels may wear the uppercase caption.
 * (DESIGN.md → Layout & App Shell → Docker / Panel System)
 */
export function DockerPanel({
  title,
  leading,
  meta,
  actions,
  pad,
  scroll = true,
  children,
  className = "",
}: DockerPanelProps) {
  return (
    <section
      className={[
        "raised flex min-h-0 min-w-0 flex-col overflow-hidden rounded-panel bg-surface",
        className,
      ].join(" ")}
    >
      <PanelHeader title={title} leading={leading} meta={meta} actions={actions} pad={pad} />
      <div className={["min-h-0 flex-1", scroll ? "overflow-auto" : ""].join(" ")}>{children}</div>
    </section>
  );
}
