# Design Spec — Krita VCS

> Frontend design reference for the Tauri desktop application.
> Aesthetic direction: **tactile bento studio** — a dark creative tool rebuilt as soft, raised
> panels in a framed grid. Ink, canvas, and precision, with weight you can feel under the cursor.

---

## Status

This document is the single source of truth for the app's design system. The app-wide style —
framed bento layout, tactile elevation, and purposeful glass — is described throughout this
document and is the locked system. Any UI change must *share* it rather than vary from it; a
screen that drifts from this file is the bug, not the exception.

- **Locked / out of scope:** the **Color Palette** section below and the theme system (see its
  note) — colors do not change.
- **Settled:** Border Radius, Shadow & Elevation, Glass, Interaction States, Layout & App Shell,
  the button / slider / status-bar patterns, and the Type Scale and Icon System — closed sets
  backed by tokens (`--text-*` in `@theme`, `ICON` in [`src/lib/iconSize.ts`](src/lib/iconSize.ts)).
  Amend this file first if a screen genuinely needs something different — never override locally.
  A value outside a closed set is a bug, not a judgment call.
- **Still open:** the interiors of the individual panels (changed-layer list, version map nodes,
  history graph, diff viewer, layer stack, palette diff, performance tab, dialog bodies), plus
  Spacing and Motion if a screen turns up a real need.

---

## Design Configuration

```
/* Hallmark · genre: atmospheric · tone: tactile-bento-studio
 * macrostructure: Bento Grid · design-system: DESIGN.md · designed-as-app
 * DESIGN_VARIANCE: 5 · MOTION_INTENSITY: 3 · VISUAL_DENSITY: 7
 */
```

| Setting            | Value | Rationale                                                    |
|--------------------|-------|--------------------------------------------------------------|
| Genre              | Atmospheric | Dark creative tool — Krita/Blender/VS Code Dark+ family |
| `DESIGN_VARIANCE`  | 5     | Bento + tactile depth is more expressive than flat panels, still structured |
| `MOTION_INTENSITY` | 3     | VCS operations are frequent — over-animating adds friction   |
| `VISUAL_DENSITY`   | 7     | Layer lists, diffs, version histories demand compact, readable layout — the bento gutters are 8px for exactly this reason |

---

## Krita Design Influence

Krita VCS is built for Krita users. These design patterns are adopted directly from Krita's UI to reduce cognitive friction for the primary audience.

| Krita Pattern | How We Apply It |
|---|---|
| **Orange accent** | `--accent #E07B39` directly matches Krita's branded "Dark Orange" official theme variant |
| **Docker metaphor** | App shell uses the docker/panel paradigm: compact title bars, one panel per zone |
| **Canvas as distinct zone** | The main working area uses `--bg` (darkest) inside its card, clearly distinct from the panel chrome around it |
| **Icon-first interactions** | Primary tool actions are icon buttons; text labels are secondary and optional |
| **Breeze icon compatibility** | Phosphor Icons `regular` weight is a thin-outline SVG style that matches Krita/KDE Breeze icons |
| **Dense information layout** | Small type (11-13px), compact spacing, high density — matches Krita's information-rich panels |

### Deliberately reversed

Three Krita patterns this spec used to inherit were dropped in the bento redesign. They are listed
here rather than deleted, because "why doesn't this look like Krita" is a fair question to ask of
this file later.

| Was | Now | Why |
|---|---|---|
| **Flat icon buttons** — borderless, no background until hover | Raised icon chips that sink on press; toggled-on *is* the pressed look | The tactile direction is the point of the redesign. Depth also carries toggle state without spending the accent color on it. |
| **1px panel dividers** — every panel seam a 1px `--border`, never shadow | 8px gutters and card elevation; no borders between zones | Spacing already communicates separation. Drawing a line *and* leaving a gap says the same thing twice. |
| **Near-square corners** — 3px buttons, 4px panels | 8px controls, 14px panels | Bento reads as a grid of distinct tiles, which needs real corner radius. |

**Identity retained:** Warm dark base palette (`#131210` family with brown-gray undertones) is our own identity, distinct from Krita's neutral grays. The orange accent is shared; the warmth is ours.

---

## Color Palette

### Base Tokens

| Token          | Hex       | OKLCH (approx)         | Role                                   |
|----------------|-----------|------------------------|----------------------------------------|
| `--bg`         | `#131210` | `oklch(9% 0.004 60)`   | App background                         |
| `--surface`    | `#1E1C1A` | `oklch(14% 0.004 60)`  | Panels, sidebars, cards                |
| `--surface-2`  | `#252320` | `oklch(17% 0.004 60)`  | Inputs, raised cards                   |
| `--surface-3`  | `#2C2A27` | `oklch(20% 0.004 60)`  | Dropdowns, context menus, popovers     |
| `--border`     | `#2D2B28` | `oklch(21% 0.004 60)`  | Dividers, input outlines               |
| `--accent`     | `#E07B39` | `oklch(63% 0.16 47)`   | Primary actions, active states, links  |
| `--text`       | `#F0EDE8` | `oklch(94% 0.006 60)`  | Primary readable text                  |
| `--text-muted` | `#7A7570` | `oklch(52% 0.006 60)`  | Labels, placeholders, secondary info   |
| `--danger`     | `#C84B31` | `oklch(50% 0.18 28)`   | Destructive actions, error states      |

> `--accent` is derived from Krita's orange branding. Use sparingly — one dominant interactive element per view.

**Locked for the redesign.** These are the default (`charcoal`) tokens. The app also ships 8
selectable color themes, each overriding the base tokens via `html[data-theme="…"]` in
`src/styles/global.css` (stamped by `src/lib/theme.tsx`): `charcoal` (default, shown above),
`krita-blue`, `electric-cyan`, `sunset-coral`, `tokyo-night`, `true-black` (all dark), plus
`gallery` and `overcast` (light — flip `color-scheme` and also override the status/diff colors
below; standalone palettes, not the base tokens inverted — see global.css's notes on each).

### Interaction Overlays

Composited on top of any surface using `background` + overlay stacking:

| Token              | Value                        | Role                                  |
|--------------------|------------------------------|---------------------------------------|
| `--state-hover`    | `rgba(255,255,255,0.05)`     | Hover tint for any interactive element |
| `--state-active`   | `rgba(255,255,255,0.08)`     | Press/active tint                     |
| `--state-selected` | `rgba(224,123,57,0.12)`      | Selected row, active tree item        |

### Status Colors

| Token           | Hex       | Foreground  | Role                         |
|-----------------|-----------|-------------|------------------------------|
| `--success`     | `#3A7D44` | `#6FCF97`   | Successful operations, added |
| `--warning`     | `#C49A28` | `#F2C94C`   | Warnings, modified state     |
| `--info`        | `#2D6EA8` | `#56B4E9`   | Informational, neutral state |

---

## Typography

### Typefaces

| Role       | Family            | Notes                              |
|------------|-------------------|------------------------------------|
| UI / Body  | `Inter`           | System fallback: `system-ui`       |
| Monospace  | `JetBrains Mono`  | Diffs, file paths, commit hashes   |

```css
--font-ui:   'Inter', system-ui, sans-serif;
--font-mono: 'JetBrains Mono', 'Fira Code', monospace;
```

---

### Type Scale

Six steps, defined as `--text-*` entries in `global.css`'s `@theme` and therefore available as
Tailwind utilities. **Use the utility, never `text-[Npx]`.** An arbitrary size is by definition
off-scale — there is a token for every size the app is allowed to use, and if none of the six fits,
the answer is a different step, not a seventh.

| Level     | Utility        | Size   | Weight | Line Height | Color          | Usage                                  |
|-----------|----------------|--------|--------|-------------|----------------|----------------------------------------|
| `title`   | `text-title`   | `20px` | `600`  | `1.3`       | `--text`       | Window/page title, welcome headline    |
| `heading` | `text-heading` | `15px` | `600`  | `1.3`       | `--text`       | Section headers, panel + modal titles  |
| `body`    | `text-body`    | `13px` | `400`  | `1.5`       | `--text`       | Default readable content, button labels |
| `dense`   | `text-dense`   | `12px` | `400`  | `1.5`       | `--text`       | Dense rows, mono content, chips, tables |
| `caption` | `text-caption` | `11px` | `400`  | `1.4`       | `--text-muted` | Timestamps, metadata, hints, card labels |
| `micro`   | `text-micro`   | `10px` | `400`  | `1.4`       | `--text-muted` | Counts, tick labels, overlay annotations |

**`dense` (12px) is the load-bearing addition.** It is the app's single most-used size, and this
table previously assigned 12px only to *mono* — so every one of the ~95 non-mono 12px sites had no
step to name itself with. That omission is the reason the scale drifted to nine sizes; a scale with
a hole in the middle of its most-used range does not get followed, it gets worked around.

**Weight and family are separate axes, and the tokens deliberately do not carry them.** `heading`
and `subheading` share a size in most systems and differ only in weight; a font-size utility that
silently set weight could not express that. So:

| Role         | Composition                                  | Usage                             |
|--------------|----------------------------------------------|-----------------------------------|
| `subheading` | `text-body font-medium`                      | In-card section labels            |
| `label`      | `text-caption font-medium uppercase`         | Card headers, form labels         |
| `mono`       | `text-dense font-mono`                       | Paths, hashes, diffs, layer names |
| `mono-muted` | `text-dense font-mono text-text-muted`       | Unchanged diff lines, context     |

There is **no `--text-mono` token**: mono content is 12px and `font-mono` already selects it, so a
size token of its own would be a second way to say the same thing — and two ways to say it is how
they drift apart.

---

## Spacing

Base unit: `4px`

| Token   | Value  |
|---------|--------|
| `xs`    | `4px`  |
| `sm`    | `8px`  |
| `md`    | `12px` |
| `lg`    | `16px` |
| `xl`    | `24px` |
| `2xl`   | `32px` |

---

## Border Radius

Bento tiles need real corners. Calibrated at the "balanced" density step — enough rounding to read
as distinct cards, not so much that the 900×600 minimum window loses a list row to it.

| Context            | Token             | Value  |
|--------------------|-------------------|--------|
| Buttons, inputs    | `--radius-button` | `8px`  |
| Cards, panels      | `--radius-panel`  | `14px` |
| Badges, tags       | `--radius-badge`  | `6px`  |
| Modals             | `--radius-modal`  | `16px` |
| The recessed well  | `--radius-well`   | `22px` |

Bento gutter: **8px** (`gap-2` / `p-2`). Every zone gap and the well's inset use this one value.

`--radius-well` is **derived, not chosen**: `14px (--radius-panel) + 8px (gutter) = 22px`. Nesting
radii concentrically means the frame's inner curve runs exactly parallel to the card corners
sitting 8px inside it. If either the panel radius or the gutter changes, this must change with
them — a well radius that isn't `panel + gutter` makes the curves converge or diverge at the
corners, which is visible immediately.

---

## Shadow & Elevation

Depth is **tactile, not glowing**. Three things stack to make a surface read as raised on a dark
palette, in this order of importance:

1. **A lightness step on the existing palette ladder.** `--bg` (recessed well) → `--surface`
   (card) → `--surface-2` (card header, raised chip) → `--surface-3` (raised control). This does
   most of the work; the shadows only sharpen it.
2. **A hairline top highlight** (`--edge-light`, ~6% white) — the lit edge of a physical object.
3. **A tight, dark, close-in shadow** (`--shadow-tint`).

> **Never a soft colored halo around a card.** A wide glowing box-shadow on a dark surface is the
> most recognizable generated-UI tell, and it smears against the diff canvas. Shadows here are
> tight and neutral; if a surface isn't reading as raised, take another lightness step before
> reaching for more blur radius.

| Token              | Role                                                        |
|--------------------|-------------------------------------------------------------|
| `--shadow-raised`  | Bento cards, raised controls at rest                        |
| `--shadow-pressed` | `:active`, and the toggled-on state of any switch-like control |
| `--shadow-well`    | Carved-in surfaces: inputs, toggle tracks, slider tracks    |
| `--shadow-float`   | Dropdowns, popovers, toasts                                 |
| `--shadow-modal`   | Dialogs and full overlays                                   |

Applied through six utility classes in `global.css` — `.raised`, `.tactile`, `.inset-well`,
`.row-selected`, and (§ Glass) `.glass` and `.scrim` — rather than repeated class strings. **These
are unlayered CSS, so they beat Tailwind's utilities layer**: never put a `shadow-*` utility on an
element carrying one of the four that set `box-shadow` (`.raised`, `.tactile`, `.inset-well`,
`.row-selected`), and never try to cancel one with `disabled:shadow-none` (handled inside
`.tactile` instead). `.glass` and `.scrim` set only background and border, so a floating surface
correctly pairs `.glass` with `shadow-(--shadow-float)`.

Only `--edge-light`, `--shadow-tint` and `--scrim` are mode-dependent; the two light themes and
True Black override them, everything else derives.

**On light themes the model drops to two cues, not three.** The hairline top highlight is a
dark-mode device — a white line on a `#ffffff` card is physically meaningless — and it cannot simply
be inverted, because `--edge-light` is the *top* inset on `--shadow-raised`: a dark hairline there
draws what reads as a broken top border, not as depth. So the light themes set
`--edge-light: transparent` and carry elevation on the lightness step plus a **substantially
stronger** `--shadow-tint`, roughly 2.5× the alpha the dark themes need. This inverts the
dark-theme instinct and is correct: on a light ground an object is legible by the shadow it casts,
not by the edge it catches. Under-tinting here is not a subtle miss — it collapses the three bento
cards and the well into one flat sheet, which is the entire direction gone.

---

## Glass

Frosted translucency is allowed **only on surfaces that float over content**, where the blur is
what communicates depth:

| Surface | Glass? |
|---|---|
| Menus, dropdowns, popovers | Yes |
| Modals and their scrim | Yes |
| Toasts | Yes |
| Blocking overlay (`BusyOverlay`) | Yes |
| Tour callout + skip chip | Yes |
| **Bento cards** (Sidebar / Main / Inspector) | **No** |
| **Chrome frame** (TopBar / ActivityBar / StatusBar) | **No** |
| **Tour dim bands** | **No** — four tiled rectangles; per-band backdrop sampling seams at the joins |

The cards sit on a flat well with nothing behind them, so glass there would blur a solid color —
decoration, not depth. It would also put a blur pass over the streaming `.kra` layer canvas, which
is the app's hottest paint path.

`.glass` is **opaque by default** and upgrades to `backdrop-filter` inside
`@supports (backdrop-filter: blur(1px))`. This build has silently dropped compositing features
before (see `TourOverlay.tsx`), so the fallback is the guaranteed path, not the exception.
`.scrim` is its sibling for the layer *behind* a floating surface — a flat `--scrim` fill,
upgrading to `blur(--scrim-blur)` under the same `@supports`. It is a separate class because the
scrim tints and the surface frosts; one class doing both would put a border on the backdrop.

---

## Diff Colors

Specific to the commit/layer diff view:

| Token            | Hex         | Usage                     |
|------------------|-------------|---------------------------|
| `--diff-add`     | `#1E3A2F`   | Added lines background    |
| `--diff-add-fg`  | `#6FCF97`   | Added lines text          |
| `--diff-del`     | `#3A1E1E`   | Removed lines background  |
| `--diff-del-fg`  | `#EB5757`   | Removed lines text        |

---

## Motion System

> **MOTION_INTENSITY: 3** — This is a tool used constantly. VCS operations (commit, stage, checkout, reset) are instant — never animated. Reserve motion for spatial feedback on UI elements the user doesn't trigger repetitively.

### Easing Curves

```css
--ease-out:     cubic-bezier(0.23, 1, 0.32, 1);    /* UI interactions, enter animations */
--ease-in-out:  cubic-bezier(0.77, 0, 0.175, 1);   /* on-screen element movement — reserved, see ‡ */
--ease-drawer:  cubic-bezier(0.32, 0.72, 0, 1);    /* panel slide-in — reserved, see ‡ */
```

`--ease-out` is the working curve; the other two are reserved for interactions the app does not yet
have (marked ‡ in the timing table below).

Never use `ease-in` on UI elements — it starts slow and makes the interface feel unresponsive.

### Duration Scale

```css
--dur-instant:  100ms;   /* button press feedback */
--dur-fast:     160ms;   /* tooltips, small popovers */
--dur-normal:   220ms;   /* dropdowns, menus */
--dur-slow:     320ms;   /* modals, drawers, panels */
```

### Per-Interaction Timing

| Element           | Duration       | Easing       | Transform                      |
|-------------------|----------------|--------------|--------------------------------|
| Button `:active`  | `--dur-instant`| `--ease-out` | `translateY(1px)` — it sinks, it doesn't shrink |
| Tooltip open      | `--dur-fast`   | `--ease-out` | `scale(0.97)` + `opacity: 0→1` |
| Dropdown / menu   | `--dur-normal` | `--ease-out` | `scale(0.95)` + `opacity: 0→1` |
| Modal open        | `--dur-slow`   | `--ease-out` | `scale(0.97)` + `opacity: 0→1` |
| **Any of the three closing** | `--dur-fast` | `--ease-out` | reverse of its open |
| *Panel slide-in* ‡ | `280ms`       | `--ease-drawer` | `translateX(-100%)→0`       |
| *Diff row expand* ‡ | `--dur-normal` | `--ease-in-out` | height + `opacity: 0→1`   |

Button `:active` is `translateY`, not `scale`, and the two tables agree on this. A tactile control
sinks into its housing; scaling it down instead reads as the button retreating from the cursor,
which is the opposite of contact. See § Interaction States.

**Floating surfaces animate away, not just in**, and every dismissal path gets it — Escape, a
backdrop or outside click, and a footer's Cancel/Close. One exit duration (`--dur-fast`) for all
three surfaces regardless of how long they took to arrive: an element appearing is information and
can afford the time, an element leaving is cleanup, and a slow exit reads as lag on a tool used this
often. The easing stays `--ease-out` — `--ease-in` remains banned even on the way out, per
§ Easing Curves.

Mechanically this needs the surface to outlive its own open state. `Menu` and `Tooltip` own that
state and use `useExitTransition` (`src/lib/useExitTransition.ts`), which keeps them mounted for the
exit and then drops them. `Modal` cannot: it is mounted by its parent (`{show && <Modal/>}`), so it
inverts the problem and holds `onClose` back until the transition has run. That is why `Modal`'s
`footer` takes a render prop — `footer={(close) => …}` — and why a dismiss button must call that
`close` rather than the parent's own `onClose`, which would unmount instantly and skip the exit. An
action that *completes* (Delete, Merge, Restore) still unmounts immediately and is meant to: the
dialog is finished, not dismissed.

While exiting, a surface is still in the DOM. It must be `pointer-events-none` so it cannot swallow
a click aimed at what is behind it.

‡ **Aspirational — no shipped surface uses these two rows**, and `--ease-drawer` / `--ease-in-out`
are consequently defined in `global.css` and referenced nowhere. They are kept because both
interactions are plausible additions and the curves should be decided once rather than improvised at
the moment one is needed. Treat them as reserved, not as describing current behaviour: don't cite
either row as precedent, and if a panel slide-in or a row expand actually lands, that is the moment
to confirm the curve is right rather than inherit it unexamined.

### Principles

- Animate `transform` and `opacity` only — never `width`, `height`, `top`, or `padding`. This
  extends to anything that *moves per frame* under a pointer, animated or not: a slider thumb is
  positioned with `translate3d` and its fill with `scaleX`, never `left`/`width`. Inside a modal
  those live under `.glass` over `.scrim` — a nested backdrop chain — and dirtying layout there
  each drag frame makes this WebView re-rasterize the whole stack, which reads as the screen
  dimming while you drag. `will-change: transform` keeps them on their own layer.
- Start from `scale(0.97)` not `scale(0)` — nothing appears from nothing
- Popovers scale from their trigger origin, not from the element's center
- Keyboard-triggered VCS operations are instant — no animation whatsoever
- `prefers-reduced-motion: reduce` → collapse all motion to ≤150ms opacity-only crossfade

---

## Interaction States

Full 8-state system required for every interactive element.

| State        | Visual Treatment                                                         |
|--------------|--------------------------------------------------------------------------|
| `default`    | Base token values + `--shadow-raised` on anything interactive            |
| `hover`      | One lightness step up (`--state-hover` overlay, or surface-2 → surface-3) |
| `focus`      | 2px `--accent` ring at 50% opacity, 2px offset (see focus ring spec)    |
| `active`     | `--shadow-pressed` + `translateY(1px)` on `--dur-instant` — it sinks, it doesn't shrink |
| `pressed`    | Toggled-on controls hold `--shadow-pressed` (`data-pressed="true"`) + accent icon |
| `disabled`   | 40% opacity, shadow removed (lies flat), `cursor: not-allowed`, no hover/active response |
| `loading`    | Spinner replaces content label, pointer-events none                      |
| `error`      | `--danger` border + text, `--danger` at 15% opacity background          |
| `success`    | `--success` fg + border, transient — returns to default after 1.5s      |

### Focus Ring Spec

```css
:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 50%, transparent);
  outline-offset: 2px;
}
```

> Never animate the focus ring appearance — it must appear instantly on focus.

**The one sanctioned exception: `.inset-well` text fields.** A carved-in field already casts its
own inset shadow at its edge; a 2px ring sitting 2px *outside* that edge lands in the gap and reads
as a halo bolted onto the field rather than as focus on it. These fields therefore suppress the
global outline and take an `--accent` border instead — the border sits exactly where the field's own
edge is, which is where focus belongs.

Two requirements, both non-negotiable, because this exception gives up the system's default
guarantees and has to buy them back:

1. **`focus-visible:`, never `focus:`.** `focus:` also fires on mouse click, which is precisely what
   the focus-visible convention exists to prevent — a field that lights up every time it is clicked
   trains the user to ignore the signal, and by the time keyboard focus genuinely needs to be found
   the indicator means nothing.
2. **A non-color co-indicator.** § Accessibility: *"Never use color as the sole indicator of
   state."* A border that changes only in hue fails for the same reason a red/green status dot does.
   Pair it with a width, weight, or background step so the state survives being seen in grayscale.

This applies **only** to text inputs inside `.inset-well`. Every other interactive element — buttons,
icon chips, toggles, menu items, sliders — takes the global ring unmodified. `!outline-none` on
anything else is a bug.

---

## Z-index Scale

```css
--z-base:     0;
--z-sticky:   10;    /* sticky headers, pinned columns */
--z-overlay:  20;    /* backdrop scrims */
--z-panel:    30;    /* floating panels, sidebars */
--z-modal:    40;    /* dialogs, drawers */
--z-toast:    50;    /* notifications */
--z-tooltip:  60;    /* tooltips */
--z-blocking: 70;    /* BusyOverlay — the non-dismissible write-in-progress block */
--z-tour:     80;    /* the tour's dim bands and callout */
```

Never use arbitrary z-index values. Reference these tokens for every layered element.

The top two are deliberately **above the tooltip**, which every other scale in this file would
call backwards. Both are modal-in-the-strong-sense: `BusyOverlay` exists to stop a stray click
racing a file rewrite, and the tour's job is to be the only thing the user can reach. A tooltip
surfacing through either one would be a hole in exactly the guarantee it is there to make.

---

## Icon System

| Setting       | Value                     |
|---------------|---------------------------|
| Library       | `@phosphor-icons/react`   |
| Sizes         | `ICON` in [`src/lib/iconSize.ts`](src/lib/iconSize.ts) — the five values below |
| Weight        | `regular` (default); `bold` for warning/error semantic icons only |
| Color         | Always inherit from surrounding text token — never hardcoded      |

| Step             | Size   | Usage                                              |
|------------------|--------|----------------------------------------------------|
| `ICON.inline`    | `12px` | Inside a chip, badge, or a line of text            |
| `ICON.dense`     | `14px` | Dense rows: layer lists, status bar, menu items    |
| `ICON.default`   | `16px` | Standard control icon — `IconButton`'s default     |
| `ICON.toolbar`   | `24px` | Toolbar / activity bar                             |
| `ICON.display`   | `32px` | Empty-state and error art only, **never a control** |

**The scale lives in TypeScript, not in `@theme`, because Phosphor takes its size as a React prop.**
A number passed to a component is invisible to the stylesheet, so this is the only token in the
system that cannot be a CSS variable — and importing `ICON` is what makes it enforceable. A literal
`size={n}` at a call site is off-scale by definition.

This replaces an earlier `16 / 20 / 24` claim that the interface had already rejected: the app had
drifted to nine distinct sizes, and `20` appeared at no explicit call site at all — it survived only
as a default that 16 of 20 `IconButton` uses overrode. `14` and `13` were the two most common. The
spec was corrected to the density the UI actually needs rather than 90 icons being edited up to a
size that had never been used deliberately.

Use one icon family per project. Do not mix Phosphor with Lucide or any other set.

---

## Layout & App Shell

**Framed bento.** The window is one continuous `--surface` **chrome frame**. TopBar, ActivityBar
and StatusBar are seamless *zones within that frame* — no borders between them, none at the window
edge. Everything else is a recessed `--bg` **well** with `--radius-well` corners, and the three
content zones float inside it as raised bento cards, 8px in from the well's edge.

**The well's rounded corners are what make the frame read as one piece.** Where two bars meet, the
frame material curves through the corner instead of forming a right angle, so the top bar, left
rail and status bar look like one border wrapping the app rather than three strips that happen to
touch. It's a single `rounded-well` on the well container — the frame simply shows through behind
it.

The frame is a **closed ring**. TopBar, ActivityBar and StatusBar supply three of the four sides;
the fourth is a plain 8px strip of `--surface` down the right edge (an `mr-2` on the well — no
component lives there, it's just frame material). Without it the chrome reads as a C open to the
right and the two right-hand corners curve into nothing.

Frame thickness varies by side — 44px top, 48px left, 24px bottom, 8px right — because three sides
are functional and one is not. What stays uniform is the **well's 8px inset**: every card sits 8px
from the well's edge on all four sides, matching the 8px card-to-card gutter.

The top edge is 44px rather than the 36px this spec originally set, because it carries a *tactile
chip* (the artwork switcher) and the window controls, not a flat label. A raised control needs
vertical room on both sides of it or the bar reads as clamped around it.

```
┌───────────────────────────────────────────────────────────────┐
│  TopBar (44px) — artwork switcher · window controls           │  ← frame
├────┬────────────────────────────────────────────────────────┬─┤
│ A  ╭────────────────────────────────────────────────────────╮ │
│ c  │ ╭──────────╮ ╭──────────────────╮ ╭─────────────────╮  │ │
│ t  │ │ Sidebar  │ │ Main Panel       │ │ Inspector       │  │ │
│ 48 │ ╰──────────╯ ╰──────────────────╯ ╰─────────────────╯  │ │
│    ╰────────────────────────────────────────────────────────╯ │
├────┴────────────────────────────────────────────────────────┴─┤
│  StatusBar (24px)                                             │  ← frame
└───────────────────────────────────────────────────────────────┘
     └ frame ┘└──── well (--bg), 22px corners, 8px inset ────┘└8┘
```

`WelcomeShell` carries the same ring with `mx-2 mb-2`, since it has no activity bar or status bar
to supply the left and bottom edges.

| Zone            | Layer  | Width          | Content                                     |
|-----------------|--------|----------------|---------------------------------------------|
| Top bar         | frame  | full width, 44px | Logo, artwork switcher, window controls — it is also the drag region |
| Activity bar    | frame  | 48px fixed     | Icon-only vertical strip, leftmost          |
| Status bar      | frame  | full width, 24px | Active file, branch, version count        |
| Sidebar         | card   | 240–320px, resizable | Changed layers, branch list, history, performance stats |
| Main panel      | card   | `flex: 1`      | Primary workspace — version map, diff view  |
| Inspector panel | card   | 280px, toggleable | Version metadata, layer details          |

**There is no file tree.** A store versions one `.kra`, so the changes sidebar lists the *layers*
that moved in the painting, not files — see CLAUDE.md → `ChangesPanel`. Any pattern in this file
that says "tree row" means that list.

Cards carry `--radius-panel` + `.raised` + `overflow-hidden` (so a card header's fill follows the
top corners) and never a border. **Nothing draws a line between two cards** — the 8px gutter
already says it.

Desktop-only application. No mobile breakpoints. **Layout is calibrated down to 900 × 600px** —
that is the size the radii and the 8px gutters are checked against, and nothing may require more.
It is a design target, not a constraint the shell enforces: `tauri.conf.json`'s window sets a
1440 × 900 default with no `minWidth`/`minHeight`, so the app can currently be dragged below it.

**Artwork switcher (top bar):** a tactile chip — brush icon + artwork name — on `--surface-2`,
`button` radius, sinking on press like any other. It opens a **searchable modal**, not a dropdown:
the tracked list is unbounded, and a `Menu` that grows past a handful of rows is a scroll trap with
no way to filter. The modal also hosts "Track an artwork…" and "Restore from a backup…", so every
way in or out of an artwork sits in one place. Local-only: no fetch/push/sync affordances.

The chip's left edge lines up with the well's 8px inset (i.e. with the sidebar card below it), and
the logo column to its left is 48px wide to match the activity bar — the frame's two left-hand
zones share one column so the bar doesn't read as two grids stacked.

**Settings dialog (activity-bar gear):** a modal with **four** left-hand category tabs — Appearance,
Performance, Storage, Set-Aside. The tab list is **static**: it renders the same four regardless of
whether an artwork is selected, and a tab whose settings need one shows a fallback message rather
than disappearing. A tab set that resizes as you switch repositories moves the other tabs under the
cursor, which is worse than a tab that is briefly empty.

Every tab shares one row anatomy — a section label, then its control — so the dialog reads as one
surface rather than four. Field width is set by the field's own content, not by the tab it sits in.

### Docker / Panel System

Krita users navigate a docker-based UI. Panels in Krita VCS follow the same conventions.

**Docker title bar** (40px height) — the card header. One definition,
`PanelHeader` in [`src/components/shell/DockerPanel.tsx`](src/components/shell/DockerPanel.tsx),
exported separately from `DockerPanel` because several cards own their own container (main panel,
inspector, version map) or have no card at all (the diff toolbar):
- Background: `--surface-2` (one lightness step above the card body)
- Label: `label` role — `text-caption font-medium uppercase`, `--text-muted`
- Slots, in order: `leading` (a back button or a whole toolbar) · `title` · `meta` (the flexible
  middle — a count, a commit message; present even when empty, which is what keeps `actions` flush
  right) · `actions` (`ICON.default` icon buttons)
- Horizontal inset: `pl-3 pr-1`, or `px-1` when the first element is *itself* a button — its own
  padding already supplies the inset and 12px on top of it reads as a stray indent
- Bottom border: 1px `--border` — the one border that survives, because it divides *inside* a
  single card rather than between two of them

**The uppercase-caption `title` treatment is reserved for this level.** In-card section headings
("STORAGE SAVED", "LAYERS") and sub-labels inside a section ("BEFORE" / "AFTER") take the
`subheading` role instead (`text-body font-medium`, § Type Scale). Three nesting levels sharing
one style is why a screen ends up with no typographic signal for which label owns which region.

**Panel dividers:** none. Cards are separated by the 8px gutter and their own elevation. A border
between two cards double-states the separation and reintroduces the flat-panel look the bento
replaced.

**Resize handle — the gutter is the handle:**
- Fills the full 8px gutter between two cards (`cursor-col-resize`)
- Invisible at rest; a short 2px `--accent` pill fades in centered on hover
- **Vertical edge** (column resize): sidebar width. **Horizontal edge** (row resize,
  `cursor-row-resize`): the art-diff canvas height — clamped, with inner content scrolling when
  shrunk so it never overflows. Both use the shared `useResize` hook and persist to `localStorage`.

### Canvas Area

The main working area — where diffs, commit graphs, and file previews are displayed — has a distinct treatment from surrounding panels.

| Property | Value |
|---|---|
| Background | `--bg` (`#131210`) |
| Border | None |
| Shadow | None |
| Fill behavior | Fills the main panel zone completely; no padding at edges |

---

## VCS Component Patterns

### Commit Card (history / timeline list)

| Property       | Value                                                  |
|----------------|--------------------------------------------------------|
| Background     | `--surface` default / `--state-hover` overlay on hover |
| Selected state | `.row-selected` — `--state-selected` background + `--shadow-pressed`, so the row sits *into* the panel. The graph rail carries the lineage; never a left border |
| Hash           | `mono` scale, `--text-muted`, 12px                    |
| Message        | `body` scale, `--text`, 13px                          |
| Timestamp      | `caption` scale, `--text-muted`, 11px                 |
| Padding        | `lg` vertical, paired with the graph rail on its left  |

### History Graph (commit lineage rail)

The history list is a git-style graph: each commit card is paired with a left **rail** that draws
the commit's node and the lane lines connecting it to its neighbors, so branch divergence and merges
read at a glance.

| Property | Value |
|---|---|
| Lane width | 16px; node centered vertically in its row |
| Node | filled circle, lane color, 1px `--bg` ring; merge node is larger with an inner `--bg` dot |
| Lines | 1.5px stroke (non-scaling), straight for through-lanes, smooth cubic for branch/merge diagonals |
| Selected node | accent halo (`box-shadow` ring in `--accent`) |

**Lane colors** are a deliberate **functional exception** to the single-accent rule (they encode
distinct branches for readability): lane 0 (mainline) = `--accent`, then `--info-fg`, `--success-fg`,
`--warning-fg`, cycled. Layout is computed in `src/lib/graph.ts`.

### Diff View

| Element          | Style                                                     |
|------------------|-----------------------------------------------------------|
| Line numbers     | `mono-muted` scale, 11px, right-aligned                   |
| Added lines      | `--diff-add` background, `--diff-add-fg` text             |
| Deleted lines    | `--diff-del` background, `--diff-del-fg` text             |
| Unchanged lines  | `--bg` background, `--text-muted` text (`mono-muted`)     |
| Hunk header      | `--surface-3` background, `--text-muted` text, mono 11px  |
| Word-level diff  | Brighter fg color on darker bg within the changed line    |

### Branch Badge

| Property    | Value                                          |
|-------------|------------------------------------------------|
| Shape       | Pill — `--radius-panel`, which at this badge's ~15px height rounds it fully |
| Background  | `--surface-3`                                  |
| Leading icon | `ICON.inline` `GitBranch`, inheriting the text color |
| Text        | `mono` role (`text-dense font-mono`), truncating |
| Colors      | `--text` local, `--accent` current branch      |

`--radius-badge` (6px) is for rectangular chips — the status badge, the file-status chip. A pill
needs half its own height, and at 15px tall `--radius-panel` is the step that supplies it; a
literal `border-radius: 4px` (what this row used to say) would have made it a rounded rectangle.

### File Status Chip (change indicator on a changed-layer row)

Two renderings of one vocabulary, chosen by Artist Mode. **On** (the default): the icon plus the
word, `caption` scale. **Off**: the bare letter, `mono` + `caption`. The color is the same either
way, and the icon is what keeps it legible in grayscale — § Accessibility, *"Never use color as
the sole indicator of state."*

| Symbol | Status     | Color           |
|--------|------------|-----------------|
| `M`    | Modified   | `--warning` fg (`#F2C94C`) |
| `A`    | Added      | `--success` fg (`#6FCF97`) |
| `D`    | Deleted    | `--danger` fg              |
| `U`    | Untracked  | `--text-muted`             |
| `R`    | Renamed    | `--info` fg (`#56B4E9`)    |
| `C`    | Conflicted | `--accent`                 |

### Tool Button (tactile icon chip)

Two distinct button types. Both are tactile; they differ in whether they carry a text label.

**Icon chip** (toolbar / docker actions):
| State    | Background      | Elevation           | Icon            |
|----------|-----------------|---------------------|-----------------|
| default  | `--surface-2`   | `--shadow-raised`   | `--text-muted`  |
| hover    | `--surface-3`   | `--shadow-raised`   | `--text`        |
| active (held) | `--surface-3` | `--shadow-pressed` + `translateY(1px)` | `--text` |
| checked  | `--surface-2`   | `--shadow-pressed` (held down) | `--accent` |
| focus    | —               | `--accent` focus ring (2px) | —       |
| disabled | `--surface-2`   | none (lies flat), 40% opacity | —     |

- Hit target: 32×32px minimum
- Icon: `ICON.default` (16px) centered — `IconButton`'s default. Denser rows pass `ICON.dense`; the
  activity rail passes `ICON.toolbar`. See § Icon System.
- No text label by default (tooltip on hover provides context)
- Checked state is carried by **depth first, color second** — the chip stays held down and the icon
  goes accent. Never a filled accent background; that spends the accent on chrome.
- **Spacing:** chips sit 8px apart — the same value as the bento gutter, so the activity rail keeps
  the shell's rhythm. The rail's vertical padding is 8px too, which lands the first chip's top edge
  and the last chip's bottom edge exactly on the card edges across the gutter. Chips need this room;
  at the 2px they inherited from the old borderless buttons they read as one jammed stack of tiles.

**Text action button** (OK / Cancel / confirm dialogs):
| State    | Background               | Elevation          | Text       |
|----------|--------------------------|--------------------|------------|
| default  | `--surface-3`            | `--shadow-raised`  | `--text`   |
| hover    | `--state-hover` overlay  | `--shadow-raised`  | `--text`   |
| active   | —                        | `--shadow-pressed` + `translateY(1px)` | `--text` |
| primary  | `--accent`               | `--shadow-raised`  | `--bg`     |
| ghost    | none until hover, then `--state-hover` | none | `--text-muted` → `--text` on hover |
| disabled | 40% opacity              | none (lies flat)   | `--text-muted` |

Borderless — the elevation defines the edge. Two sizes: `md` (28px tall, `body` text) and `sm`
(`dense` text) for dense rows. `ghost` is the one variant that is *not* raised — it is for a
tertiary action inside an already-raised surface, where a second chip would compete with the
primary one; it is not a licence to drop elevation from an ordinary button.

**Destructive button** (Delete / Reset / Discard):
- Default: `--surface-3` background, raised
- Hover: `--danger` at 15% opacity background, `--danger` text

### Toggle / Switch

| Element | Off | On |
|---|---|---|
| Track | `--surface-3`, `--shadow-well` (carved in) | **unchanged — still `--surface-3`, still recessed** |
| Thumb | `--text`, `--shadow-raised`, resting at the left | `--accent`, `--shadow-raised`, slid to the right |
| Travel | — | `--dur-normal` / `--ease-out`, `transform` only |

Two sizes, both in `Switch.tsx`: `md` (20×36px track, 16px thumb) is the default; `sm` (16×28px,
12px thumb) is for a switch sitting inside a dense row. Nothing else.

**Never fill the track with the accent.** The track is the *housing*; the thumb is the part that
moves and therefore the part that carries state. Filling the housing spends the accent on chrome —
§ Notes, *"Depth before color"* — and it does so at the worst possible scale: a settings list is a
column of toggles, so an accent-filled track turns every preference the user happens to have enabled
into a saturated pill competing with the one action on the screen that should be dominant. In the
diff viewer it made a "Show Diff" switch the loudest element in a view whose subject is the artwork.

The track stays recessed in both states because the housing does not move. Depth here says *"this is
a slot"*, not *"this is on"* — that is the thumb's job, and the thumb's position states it without
any color at all, which is what keeps the control legible in grayscale (§ Accessibility, *"Never use
color as the sole indicator of state"*).

One `Switch` primitive, one set of measurements. Three hand-rolled copies at three different sizes
is three thumb animations to keep in sync and three chances for them to disagree.

### Slider / Range Control

Two kinds, and they are not interchangeable. What separates them is whether the value is a *number
the user is dialling in* or *one of a short list of named options* — a continuous slider with three
stops is a radio group wearing a costume, and a discrete slider showing a raw number tells the user
nothing about what the number means.

**Continuous slider** ‡ — numeric properties: opacity, threshold, offset. Common in creative tool
panels, and **not currently built**: `src/components/ui/Slider.tsx` is the discrete one only (it
takes an `options` array, so there is no continuous mode to reach). Reserved, per the same
convention as § Motion's ‡ rows — decide the measurements once here rather than improvising them
the day a numeric property needs one.

| Element | Value |
|---|---|
| Track height | 4px |
| Track background | `--surface-3` + `--shadow-well` (carved in) |
| Fill color | `--accent` on the travelled portion |
| Thumb size | 12px circle |
| Thumb fill | `--accent`, `--shadow-raised` (sits proud of the track) |
| Thumb border | 2px `--bg` (for contrast against fill) |
| Label position | Left: property name (`label` role, 11px); right: current value (mono 11px) |
| Keyboard behavior | Arrow keys: ±1 unit; Shift+Arrow: ±10 units |

**Discrete slider** — a drag-to-pick option list with labelled stops (Settings → Performance's
"Background CPU use": Gentle / Balanced / Full speed). The drag-target sibling of `Select`, chosen
over one when the options are *ordered* and the ordering is the point.

| Element | Value |
|---|---|
| Track height | 6px |
| Track background | `--surface-3` + `--shadow-well` (carved in) |
| Fill color | **`--accent` on the travelled portion — required** |
| Thumb size | 16px circle |
| Thumb fill | `--accent`, `--shadow-raised` |
| Labels | One per stop, `caption` scale, equal-width columns so each label centres on its own tick; current option in `--text`, the rest `--text-muted` |
| Keyboard behavior | Arrow keys: ±1 stop; Home/End: first/last |

The larger 6px track and 16px thumb are deliberate and are the one place this control departs from
the continuous spec above: it is dragged directly rather than nudged, and its stops are coarse, so
it needs a real hit target. **The accent fill is not optional.** Without it the thumb's position is
the only encoding of the value, which forces the user to measure a dot against a bare track — a
filled track states "how far along" pre-attentively, which is the entire reason a slider is used
here instead of a dropdown.

### Status Bar

Single bar fixed at the bottom of the app shell.

| Property | Value |
|---|---|
| Height | 24px |
| Background | `--surface` — the frame's bottom edge, continuous with the activity bar |
| Top border | None (it is the frame, not a panel against one) |
| Font | `caption` scale (11px, `--text-muted`) |
| Left zone | Active file name, modification indicator (`·` prefix in `--warning-fg` for unsaved) |
| Right zone | Browser-preview badge (outside the Tauri shell only), branch name, version count |
| Separator | `--border` vertical 1px between zones |
| Save progress | While a version is being written: a 2px indeterminate `--accent` bar along the bar's **top** edge, full width. `top-0`, not `-top-px` — the status bar is a seamless edge of the frame, so there is no border left to straddle |

The browser-preview badge is outlined, not filled: on the two light themes `--warning` is already a
pale cream, so a 20% wash of it over a light page is invisible. `--warning-fg` is the one warning
token defined to contrast with its own theme's background, so the border holds in all eight.

### Toast

One slot, bottom-right (`bottom-4 right-4`), `--z-toast`, `.glass` + `--shadow-float`, `panel`
radius, `role="status"`. A new toast **replaces** the current one
rather than stacking — this app raises them for low-frequency manual actions (a finished backup),
so what is wanted is a brief confirmation, not a notification feed that needs managing.

| Property | Value |
|---|---|
| Variants | `success` (`--success-fg` + `CheckCircle`) / `error` (`--danger` + `WarningCircle`) — icon, not color alone |
| Dismissal | Auto after 5s, or the `X` button |
| Text | `body` scale |

---

## Accessibility

| Requirement     | Rule                                                                          |
|-----------------|-------------------------------------------------------------------------------|
| Contrast — body | WCAG AA minimum: 4.5:1 for text ≤17px                                        |
| Contrast — large | WCAG AA minimum: 3:1 for text ≥18px or ≥14px bold                           |
| Focus           | Every interactive element must have a visible `:focus-visible` ring (see spec) |
| Keyboard nav    | Tab order follows visual left-to-right, top-to-bottom; no focus traps outside modals |
| Motion          | All animations respect `prefers-reduced-motion` (see Motion System)           |
| Color alone     | Never use color as the sole indicator of state — pair with icon, label, or shape |

---

## Notes

- All interactive elements must have a visible focus ring using `--accent` at 50% opacity.
- Avoid pure black (`#000`) or pure white (`#fff`) — use the palette tokens.
- Prefer `--text-muted` for anything the user doesn't need to read immediately.
- The monospace font carries significant visual weight in this app — treat it as a design element.
- Animate `transform` and `opacity` only. Never animate layout properties.
- VCS operations (commit, stage, checkout) are instant — no animation.
- Use `--state-hover` / `--state-active` overlays rather than separate hover color tokens; this keeps surfaces composable across all elevation levels.
- Icon color always inherits from surrounding text — never set icon color independently.
- **Elevated over flat** — cards are separated by 8px gutters and their own depth, never by a border. A border between two cards states the separation twice.
- **Depth before color** — when something needs to read as selected, active, or held, reach for `--shadow-pressed` and a lightness step first. Spend the accent on the one dominant action per view, not on chrome.
- **No side stripes** — a selected row is `.row-selected` (tinted + pressed), never a thick accent border on one edge. That pattern is a well-known generated-UI tell and it fights the history graph's rail.
- **Two button types, never mixed** — icon chips for tool actions; text buttons for dialog confirmations. Never put text labels on an icon chip or icon-only content on a text action button.
- **Never mix `.raised` / `.tactile` / `.inset-well` with a `shadow-*` utility** on the same element — they are unlayered CSS and will win silently.
