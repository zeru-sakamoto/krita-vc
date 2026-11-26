/**
 * Glides a container's children to their new spot when the layout around them changes —
 * a button appearing beside them, a sibling's label getting longer, a chip disappearing —
 * instead of letting them jump there in one frame.
 *
 * Use it as a ref on the container whose *direct* children should glide:
 *
 *   <div ref={shiftLayout} className="flex items-center gap-1">…</div>
 *
 * It is a plain ref callback, not a hook: React 19 runs the function it returns as the ref's
 * cleanup, and a module-level function has a stable identity, so React attaches it once per
 * mounted element.
 *
 * Mechanically it's FLIP: remember where every child was laid out, and when something changes,
 * measure again and play a `transform` from the old spot back to zero. Only `transform` and
 * `opacity` ever animate (DESIGN.md → Motion System → Principles), so layout is never dirtied
 * per frame. Movement uses `--ease-in-out` at `--dur-normal`; a newly appearing child gets the
 * enter treatment instead (`--ease-out`, `--dur-fast`, `scale(0.97)` + fade). A child that is
 * removed just vanishes — React owns the DOM and has already dropped it — but its neighbours
 * glide into the gap.
 *
 * **Nested containers are one system, not a stack of independent ones.** Positions are measured
 * relative to the *outermost* tracked container (the root), never to each child's own parent,
 * and a child that contains a tracked container is left to that container rather than glided as
 * a rigid block. Composing per-level glides instead looked equivalent and wasn't: when a box
 * resizes, the things inside it move by different amounts, so one rigid glide was right for some
 * of them and wrong for the rest — the map header's readout widening slid its whole right-aligned
 * group back as a block, which pushed the last button out into the header's padding.
 *
 * **A child that resized moves by its steadiest edge**: the smallest of its left-, right- and
 * centre-deltas. A flush-right box that grows leftwards kept its right edge, so it doesn't move;
 * the boxes beside it, which kept their size, glide by the full amount.
 *
 * What triggers a check:
 * - a DOM mutation anywhere inside a tracked container (a child added/removed, a label's text or
 *   a class changed). These always animate.
 * - a tracked container or one of its children resizing. These animate unless the root itself
 *   changed size: when it's resized from outside (the sidebar being dragged, the window
 *   resizing), children following it should track the drag 1:1, not trail 220 ms behind.
 * Either way the whole root is re-measured, so a change in one tracked row also glides the rows
 * nested beside or inside it.
 *
 * Not for large or virtualized lists (every check measures each child), and not inside React
 * Flow nodes — those are re-rendered on viewport changes and would measure constantly.
 */

// Duplicated from `--dur-normal` / `--dur-fast` and `--ease-in-out` / `--ease-out` in
// `global.css`: the Web Animations API takes literal values, and reading custom properties per
// animation would cost a style recalc. Keep them in sync.
const MOVE_MS = 220;
const ENTER_MS = 160;
const EASE_IN_OUT = "cubic-bezier(0.77, 0, 0.175, 1)";
const EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface Glide {
  anim: Animation;
  dx: number;
  dy: number;
}

interface Tracker {
  /** Laid-out (untransformed) box of each direct child, relative to the root. */
  last: Map<Element, Box>;
  /**
   * Whether `last` describes a visible layout. False on mount and after the root was hidden, so
   * the next measurement only records positions — a panel coming back from `display: none` must
   * not animate every child in from nowhere.
   */
  primed: boolean;
  /** The container's own size, consulted only while it is a root. */
  size: { w: number; h: number };
  ro: ResizeObserver;
}

const trackers = new Map<HTMLElement, Tracker>();
const glides = new WeakMap<Element, Glide>();

const reducedMotion =
  typeof window !== "undefined" ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;

/**
 * Page position from the `offsetLeft`/`offsetTop` chain. These are layout coordinates: no
 * transform touches them — not a glide in flight, not an enter's `scale(0.97)` (measured as a
 * bounding rect, that read as a sub-pixel "move") — so they are exactly what FLIP needs.
 */
function layoutPos(el: HTMLElement): { x: number; y: number } {
  let x = 0;
  let y = 0;
  for (let e: HTMLElement | null = el; e; e = e.offsetParent as HTMLElement | null) {
    x += e.offsetLeft;
    y += e.offsetTop;
  }
  return { x, y };
}

/** How far a child's running glide currently has it displaced from its laid-out spot. */
function offsetOf(el: Element): { x: number; y: number } {
  const g = glides.get(el);
  if (!g || g.anim.playState !== "running") return { x: 0, y: 0 };
  // `progress` is the eased progress, because the easing is on the effect timing (not on a
  // keyframe), so the remaining displacement is a straight lerp of it.
  const p = g.anim.effect?.getComputedTiming().progress ?? 1;
  return { x: g.dx * (1 - p), y: g.dy * (1 - p) };
}

/** The edge delta (old − new) of smallest magnitude: how far a box moved, net of resizing. */
function steadiest(start: number, end: number, center: number): number {
  let d = start;
  if (Math.abs(end) < Math.abs(d)) d = end;
  if (Math.abs(center) < Math.abs(d)) d = center;
  return d;
}

interface Wall {
  min: number;
  max: number;
}

/**
 * Limits a glide's starting offset `d` so the box `[pos, pos + size]` starts inside `wall`.
 * A box already outside the wall at its resting spot (it overflows anyway) is left alone, since
 * there is no in-bounds start to clamp to.
 */
function clampToWall(d: number, pos: number, size: number, wall: Wall | null): number {
  if (!wall || pos < wall.min - 0.5 || pos + size > wall.max + 0.5) return d;
  return Math.min(Math.max(d, wall.min - pos), wall.max - (pos + size));
}

/** Whether `el` is, or contains, a tracked container — whose own tracker then owns its motion. */
function holdsTracked(el: Element): boolean {
  for (const c of trackers.keys()) if (el === c || el.contains(c)) return true;
  return false;
}

function rootOf(el: HTMLElement): HTMLElement {
  let root = el;
  for (let p = el.parentElement; p; p = p.parentElement) if (trackers.has(p)) root = p;
  return root;
}

function run(trigger: HTMLElement, fromMutation: boolean) {
  const root = rootOf(trigger);
  const rt = trackers.get(root);
  if (!rt) return;
  const inRoot = [...trackers].filter(([c]) => c === root || root.contains(c));

  if (root.offsetWidth === 0 && root.offsetHeight === 0) {
    // Hidden: forget everything, start over when it shows again.
    for (const [, t] of inRoot) {
      t.last = new Map();
      t.primed = false;
    }
    return;
  }
  const resized =
    Math.abs(root.offsetWidth - rt.size.w) > 0.5 || Math.abs(root.offsetHeight - rt.size.h) > 0.5;
  rt.size = { w: root.offsetWidth, h: root.offsetHeight };
  const origin = layoutPos(root);
  const reduce = reducedMotion?.matches ?? false;

  // The root's content box in root-relative layout coordinates, computed lazily (it needs a
  // `getComputedStyle`) and only once per run. An axis whose content overflows (a scroller, a
  // row too narrow for its items) gets no wall: positions there legitimately sit beyond it.
  let wallCache: { x: Wall | null; y: Wall | null } | undefined;
  const walls = () => {
    if (wallCache) return wallCache;
    const cs = getComputedStyle(root);
    const pl = parseFloat(cs.paddingLeft) || 0;
    const pr = parseFloat(cs.paddingRight) || 0;
    const pt = parseFloat(cs.paddingTop) || 0;
    const pb = parseFloat(cs.paddingBottom) || 0;
    // Positions come from `layoutPos` differences, whose origin is the root's *padding* edge when
    // the root is its children's offsetParent (it's positioned) and its *border* edge otherwise.
    const bl = cs.position === "static" ? root.clientLeft : 0;
    const bt = cs.position === "static" ? root.clientTop : 0;
    wallCache = {
      x:
        root.scrollWidth > root.clientWidth + 1
          ? null
          : { min: bl + pl, max: bl + root.clientWidth - pr },
      y:
        root.scrollHeight > root.clientHeight + 1
          ? null
          : { min: bt + pt, max: bt + root.clientHeight - pb },
    };
    return wallCache;
  };

  for (const [container, t] of inRoot) {
    const animate = t.primed && (fromMutation || !resized);
    const next = new Map<Element, Box>();
    for (const el of Array.from(container.children)) {
      // `offsetParent` is null for a hidden or fixed child; neither takes part in the row.
      if (!(el instanceof HTMLElement) || el.offsetParent === null) continue;
      if (el.offsetWidth === 0 && el.offsetHeight === 0) continue;
      if (holdsTracked(el)) continue;
      t.ro.observe(el);
      const p = layoutPos(el);
      const box = { x: p.x - origin.x, y: p.y - origin.y, w: el.offsetWidth, h: el.offsetHeight };
      next.set(el, box);
      if (!animate) continue;

      const prev = t.last.get(el);
      if (!prev) {
        el.animate(
          reduce
            ? [{ opacity: 0 }, { opacity: 1 }]
            : [
                { opacity: 0, transform: "scale(0.97)" },
                { opacity: 1, transform: "scale(1)" },
              ],
          { duration: ENTER_MS, easing: EASE_OUT, composite: "add" }
        );
        continue;
      }
      const mx = steadiest(
        prev.x - box.x,
        prev.x + prev.w - (box.x + box.w),
        prev.x + prev.w / 2 - (box.x + box.w / 2)
      );
      const my = steadiest(
        prev.y - box.y,
        prev.y + prev.h - (box.y + box.h),
        prev.y + prev.h / 2 - (box.y + box.h / 2)
      );
      if (Math.abs(mx) < 0.5 && Math.abs(my) < 0.5) continue;

      // Start from where it is drawn now: its old spot plus whatever glide is still in flight —
      // but never from outside the root's content box. That box is the wall: an item may glide
      // up to it, not into the padding beyond (or past a border). The steadiest-edge pick can
      // otherwise land a start point there (an item that both grew and was pushed moves by its
      // centre), and so can a glide interrupted mid-flight, or a root that shrank.
      const cur = offsetOf(el);
      const dx = clampToWall(mx + cur.x, box.x, box.w, walls().x);
      const dy = clampToWall(my + cur.y, box.y, box.h, walls().y);
      glides.get(el)?.anim.cancel();
      glides.delete(el);
      // Reduced motion: no movement at all (DESIGN.md: opacity-only) — it just lands.
      if (reduce) continue;
      const anim = el.animate(
        [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "translate(0, 0)" }],
        // `composite: "add"` so a child's own transform (if any) keeps applying underneath.
        { duration: MOVE_MS, easing: EASE_IN_OUT, composite: "add" }
      );
      glides.set(el, { anim, dx, dy });
    }
    t.last = next;
    t.primed = true;
  }
}

export function shiftLayout(container: HTMLElement | null): (() => void) | undefined {
  if (!container || typeof ResizeObserver === "undefined") return;

  // ResizeObserver fires after layout and before paint, and a MutationObserver's microtask runs
  // before the frame paints too, so the inverted transform is in place before the jump is ever
  // seen. Our own animations touch only transform/opacity, so they never re-trigger either.
  // Several observers often fire for one change (every tracked level sees a subtree mutation);
  // the first run records the new layout, so the rest find nothing moved.
  const ro = new ResizeObserver(() => run(container, false));
  const mo = new MutationObserver(() => run(container, true));
  trackers.set(container, { last: new Map(), primed: false, size: { w: 0, h: 0 }, ro });
  ro.observe(container);
  mo.observe(container, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
    attributeFilter: ["class", "style", "hidden"],
  });
  // Re-prime the whole root: a newly tracked container changes which children its ancestors
  // own (they now skip the child that holds it), and those must not read as "removed".
  const root = rootOf(container);
  for (const [c, t] of trackers) if (c === root || root.contains(c)) t.primed = false;
  run(container, false);

  return () => {
    ro.disconnect();
    mo.disconnect();
    trackers.delete(container);
  };
}
