import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * Discrete slider over a fixed option list — the drag-to-pick sibling of Select.
 * Pointer-capture drag mirrors CompareSlider's divider handle, including the rAF coalescing:
 * raw pointermove fires >100x/s, and committing a state update on every one of them repaints
 * this and the modal's glass backdrop far faster than the screen refreshes, which this WebView
 * build visibly can't keep up with (a momentary dark flash as the backdrop-filter falls behind).
 * We track the latest pointer X in a ref and flush to state at most once per frame instead.
 */
export function Slider<T extends string | number>({
  value,
  options,
  onChange,
  disabled,
  ariaLabel,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  ariaLabel?: string;
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const latestXRef = useRef(0);
  const rafRef = useRef(0);
  const [dragRatio, setDragRatio] = useState<number | null>(null);
  const index = Math.max(
    0,
    options.findIndex((o) => o.value === value)
  );
  const steps = Math.max(1, options.length - 1);

  const setIndex = useCallback(
    (i: number) => {
      const clamped = Math.min(options.length - 1, Math.max(0, i));
      const next = options[clamped];
      if (next && next.value !== value) onChange(next.value);
    },
    [onChange, options, value]
  );

  const flush = useCallback(() => {
    rafRef.current = 0;
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return;
    const ratio = Math.min(1, Math.max(0, (latestXRef.current - rect.left) / rect.width));
    setDragRatio(ratio);
    setIndex(Math.round(ratio * steps));
  }, [setIndex, steps]);

  const moveTo = useCallback(
    (clientX: number) => {
      latestXRef.current = clientX;
      if (rafRef.current === 0) rafRef.current = requestAnimationFrame(flush);
    },
    [flush]
  );

  // Cancel any pending frame on unmount so flush never runs after teardown.
  useEffect(
    () => () => {
      if (rafRef.current !== 0) cancelAnimationFrame(rafRef.current);
    },
    []
  );

  // The thumb is placed with translate3d, which needs pixels — a translateX percentage would
  // resolve against the thumb's own 16px box, not the track. Measured before paint so the
  // thumb never renders at 0 and then visibly slides into place when the modal opens.
  const [trackWidth, setTrackWidth] = useState(0);
  useLayoutEffect(() => {
    const el = trackRef.current;
    if (!el) return;
    const measure = () => setTrackWidth(el.getBoundingClientRect().width);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const ratio = dragRatio ?? index / steps;

  return (
    <div>
      <div
        ref={trackRef}
        role="slider"
        aria-label={ariaLabel}
        aria-valuemin={0}
        aria-valuemax={options.length - 1}
        aria-valuenow={index}
        aria-valuetext={options[index]?.label}
        aria-disabled={disabled}
        tabIndex={disabled ? -1 : 0}
        className={[
          "inset-well relative h-1.5 w-full touch-none rounded-full bg-surface-3",
          disabled ? "opacity-50" : "cursor-pointer",
        ].join(" ")}
        onPointerDown={(e) => {
          if (disabled) return;
          draggingRef.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          moveTo(e.clientX);
        }}
        onPointerMove={(e) => {
          if (draggingRef.current) moveTo(e.clientX);
        }}
        onPointerUp={(e) => {
          draggingRef.current = false;
          if (rafRef.current !== 0) {
            cancelAnimationFrame(rafRef.current);
            rafRef.current = 0;
          }
          setDragRatio(null);
          e.currentTarget.releasePointerCapture(e.pointerId);
        }}
        onPointerCancel={() => {
          draggingRef.current = false;
          setDragRatio(null);
        }}
        onKeyDown={(e) => {
          if (disabled) return;
          if (e.key === "ArrowRight" || e.key === "ArrowUp") {
            e.preventDefault();
            setIndex(index + 1);
          } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
            e.preventDefault();
            setIndex(index - 1);
          } else if (e.key === "Home") {
            e.preventDefault();
            setIndex(0);
          } else if (e.key === "End") {
            e.preventDefault();
            setIndex(options.length - 1);
          }
        }}
      >
        {/* Positioned with `transform`, never `left`. `left` is a layout property, and this
            thumb lives inside the modal's `.glass` panel — a backdrop-filter over the
            `.scrim`'s own backdrop-filter. Invalidating layout in there each drag frame makes
            this WebView re-rasterize that nested backdrop chain, which shows up as the whole
            screen dimming while you drag. `transform` + `will-change` keeps the thumb on its
            own compositor layer, so moving it never dirties the backdrop behind it. */}
        <span
          aria-hidden
          className="raised absolute left-0 top-1/2 size-4 rounded-full bg-accent"
          style={{
            transform: `translate3d(${ratio * trackWidth}px, -50%, 0) translateX(-50%)`,
            transition: dragRatio === null ? "transform 150ms ease-out" : "none",
            willChange: "transform",
          }}
        />
      </div>
      {/* Equal-width columns, not justify-between: with unequal label widths (e.g. "Gentle" vs
          "Full speed"), space-between centers the middle label between the *edges* of its
          neighbors rather than on the container's true midpoint, so it drifts off the dot's
          50% position. A flex-1 column per label keeps each one's own center aligned with its
          tick (0%, 50%, ..., 100%), matching how the thumb is positioned above. */}
      <div className="mt-1 flex text-[11px]">
        {options.map((o, i) => (
          <button
            key={String(o.value)}
            type="button"
            disabled={disabled}
            onClick={() => setIndex(i)}
            className={[
              "flex-1 rounded px-1 disabled:cursor-not-allowed",
              i === 0
                ? "-ml-1 text-left"
                : i === options.length - 1
                  ? "-mr-1 text-right"
                  : "text-center",
              o.value === value ? "text-text" : "text-text-muted hover:text-text",
            ].join(" ")}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
