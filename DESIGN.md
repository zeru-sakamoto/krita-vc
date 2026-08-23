# Design Spec — Krita VCS

> Frontend design reference for the Tauri desktop application.
> Aesthetic direction: **tactile bento studio** — a dark creative tool rebuilt as soft, raised
> panels in a framed grid. Ink, canvas, and precision, with weight you can feel under the cursor.

---

## Redesign in progress

A screen-by-screen UI/UX redesign is underway, tracked in [`REDESIGN.md`](REDESIGN.md).

**The foundation pass has landed.** The app-wide style — framed bento layout, tactile elevation,
and purposeful glass — is described throughout this document and is now the locked system. Every
screen redesign must *share* it rather than vary from it; a screen that drifts from this file is
the bug, not the exception.

- **Locked / out of scope:** the **Color Palette** section below and the theme system (see its
  note) — colors do not change.
- **Settled by the foundation pass:** Border Radius, Shadow & Elevation, Glass, Interaction
  States, Layout & App Shell, and the button / slider / status-bar patterns. Amend them here first
  if a screen genuinely needs something different — never override locally.
- **Still open per screen:** the interiors of the individual panels (changes list, history graph,
  diff viewer, layer stack, palette diff, performance tab, dialog bodies), plus Typography,
  Spacing, Motion and Icon System if a screen turns up a real need.

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
| `VISUAL_DENSITY`   | 7     | File trees, diffs, commit logs demand compact, readable layout — the bento gutters are 8px for exactly this reason |

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
`charcoal-light` and `studio-light` (light — flip `color-scheme` and also override the status/diff
colors below). None of the 8 themes change as part of this redesign.

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

| Level        | Size     | Weight  | Font       | Color          | Line Height | Usage                             |
|--------------|----------|---------|------------|----------------|-------------|-----------------------------------|
| `title`      | `20px`   | `600`   | UI         | `--text`       | `1.3`       | Window/page title                 |
| `heading`    | `15px`   | `600`   | UI         | `--text`       | `1.3`       | Section headers, panel titles     |
| `subheading` | `13px`   | `500`   | UI         | `--text`       | `1.4`       | Subsection labels                 |
| `body`       | `13px`   | `400`   | UI         | `--text`       | `1.5`       | Default readable content          |
| `caption`    | `11px`   | `400`   | UI         | `--text-muted` | `1.5`       | Timestamps, metadata, hints       |
| `label`      | `11px`   | `500`   | UI         | `--text-muted` | `1.4`       | Form labels, input prefixes       |
| `mono`       | `12px`   | `400`   | Monospace  | `--text`       | `1.6`       | Paths, hashes, diffs, layer names |
| `mono-muted` | `12px`   | `400`   | Monospace  | `--text-muted` | `1.6`       | Unchanged diff lines, context     |

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

Applied through five utility classes in `global.css` — `.raised`, `.tactile`, `.inset-well`,
`.glass`, `.row-selected` — rather than repeated class strings. **These are unlayered CSS, so they
beat Tailwind's utilities layer**: never put a `shadow-*` utility on an element that already
carries one of them, and never try to cancel one with `disabled:shadow-none` (handled inside
`.tactile` instead).

Only `--edge-light`, `--shadow-tint` and `--scrim` are mode-dependent; the two light themes and
True Black override them, everything else derives.

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
--ease-in-out:  cubic-bezier(0.77, 0, 0.175, 1);   /* on-screen element movement */
--ease-drawer:  cubic-bezier(0.32, 0.72, 0, 1);    /* panel slide-in */
```

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
| Button `:active`  | `--dur-instant`| `--ease-out` | `scale(0.97)`                  |
| Tooltip open      | `--dur-fast`   | `--ease-out` | `scale(0.97)` + `opacity: 0→1` |
| Dropdown / menu   | `--dur-normal` | `--ease-out` | `scale(0.95)` + `opacity: 0→1` |
| Panel slide-in    | `280ms`        | `--ease-drawer` | `translateX(-100%)→0`       |
| Modal open        | `--dur-slow`   | `--ease-out` | `scale(0.97)` + `opacity: 0→1` |
| Diff row expand   | `--dur-normal` | `--ease-in-out` | height + `opacity: 0→1`    |

### Principles

- Animate `transform` and `opacity` only — never `width`, `height`, `top`, or `padding`
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
```

Never use arbitrary z-index values. Reference these tokens for every layered element.

---

## Icon System

| Setting       | Value                     |
|---------------|---------------------------|
| Library       | `@phosphor-icons/react`   |
| Size — dense  | `16px`                    |
| Size — default | `20px`                   |
| Size — toolbar | `24px`                   |
| Weight        | `regular` (default); `bold` for warning/error semantic icons only |
| Color         | Always inherit from surrounding text token — never hardcoded      |

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

Frame thickness varies by side — 36px top, 48px left, 24px bottom, 8px right — because three sides
are functional and one is not. What stays uniform is the **well's 8px inset**: every card sits 8px
from the well's edge on all four sides, matching the 8px card-to-card gutter.

```
┌───────────────────────────────────────────────────────────────┐
│  TopBar (36px) — repository switcher                          │  ← frame
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
| Top bar         | frame  | full width, 36px | Repository switcher (folder the user designated) |
| Activity bar    | frame  | 48px fixed     | Icon-only vertical strip, leftmost          |
| Status bar      | frame  | full width, 24px | Active file, branch, commit count         |
| Sidebar         | card   | 240–320px, resizable | File tree, branch list, history        |
| Main panel      | card   | `flex: 1`      | Primary workspace — diff view, commit canvas |
| Inspector panel | card   | 280px, toggleable | Commit details, layer metadata, blame    |

Cards carry `--radius-panel` + `.raised` + `overflow-hidden` (so a card header's fill follows the
top corners) and never a border. **Nothing draws a line between two cards** — the 8px gutter
already says it.

Desktop-only application. No mobile breakpoints. **Minimum window size:** 900 × 600px.

**Repository switcher (top bar):** a flat button — folder icon + repo name + caret — opening a
dropdown menu of local repositories plus an "Add repository…" action. Local-only: no fetch/push/sync
affordances. Menu surface: `--surface-2`, 1px `--border`, `panel` radius, `--shadow-float`; the
active repo row shows a check in `--accent`. Closes on outside-click or Escape.

### Docker / Panel System

Krita users navigate a docker-based UI. Panels in Krita VCS follow the same conventions.

**Docker title bar** (40px height) — the card header:
- Background: `--surface-2` (one lightness step above the card body)
- Label: 11px, weight 500, `--text-muted`, uppercase
- Right side: 16px action icons (collapse, close, options)
- Bottom border: 1px `--border` — the one border that survives, because it divides *inside* a
  single card rather than between two of them

**Docker tab strip** (28px height, when panels are grouped):
- Inactive tab: `--surface` background, `--text-muted` label
- Active tab: `--surface-2` background, `--text` label, 2px `--accent` bottom border
- Tab padding: `sm` horizontal (8px)

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
| Shape       | Pill, `border-radius: 4px`                     |
| Background  | `--surface-3`                                  |
| Text        | Mono font, 11px                                |
| Colors      | `--text` local, `--accent` current HEAD |

### File Status Chip (change indicator in file tree)

Single-letter indicator, right-aligned in the tree row, mono 11px.

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
- Icon: 20px centered
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
| disabled | 40% opacity              | none (lies flat)   | `--text-muted` |

Borderless — the elevation defines the edge.

**Destructive button** (Delete / Reset / Discard):
- Default: `--surface-3` background, raised
- Hover: `--danger` at 15% opacity background, `--danger` text

### Slider / Range Control

Used for numeric properties — opacity, threshold, offset. Common in creative tool panels.

| Element | Value |
|---|---|
| Track height | 4px |
| Track background | `--surface-3` + `--shadow-well` (carved in) |
| Fill color | `--accent` |
| Thumb size | 12px circle |
| Thumb fill | `--accent`, `--shadow-raised` (sits proud of the track) |
| Thumb border | 2px `--bg` (for contrast against fill) |
| Label position | Left: property name (`label` scale, 11px); right: current value (mono 11px) |
| Keyboard behavior | Arrow keys: ±1 unit; Shift+Arrow: ±10 units |

### Status Bar

Single bar fixed at the bottom of the app shell.

| Property | Value |
|---|---|
| Height | 24px |
| Background | `--surface` — the frame's bottom edge, continuous with the activity bar |
| Top border | None (it is the frame, not a panel against one) |
| Font | `caption` scale (11px, `--text-muted`) |
| Left zone | Active file name, modification indicator (`·` prefix for unsaved) |
| Right zone | Branch name, commit count |
| Separator | `--border` vertical 1px between zones |

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
