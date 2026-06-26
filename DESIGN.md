# Design Spec — Krita VCS

> Frontend design reference for the Tauri desktop application.
> Aesthetic direction: **dark studio tool** — like Krita's own UI, but more structured. Ink, canvas, and precision.

---

## Design Configuration

```
/* Hallmark · genre: atmospheric · tone: dark-studio-tool
 * DESIGN_VARIANCE: 4 · MOTION_INTENSITY: 3 · VISUAL_DENSITY: 7
 */
```

| Setting            | Value | Rationale                                                    |
|--------------------|-------|--------------------------------------------------------------|
| Genre              | Atmospheric | Dark creative tool — Krita/Blender/VS Code Dark+ family |
| `DESIGN_VARIANCE`  | 4     | Structured and precise; not expressive                       |
| `MOTION_INTENSITY` | 3     | VCS operations are frequent — over-animating adds friction   |
| `VISUAL_DENSITY`   | 7     | File trees, diffs, commit logs demand compact, readable layout |

---

## Krita Design Influence

Krita VCS is built for Krita users. These design patterns are adopted directly from Krita's UI to reduce cognitive friction for the primary audience.

| Krita Pattern | How We Apply It |
|---|---|
| **Flat icon buttons** | Toolbar/docker buttons are borderless with no background by default; `--state-hover` overlay appears only on hover |
| **1px panel dividers** | All panel-to-panel borders are exactly 1px `--border` — no elevation or shadows between docked panels |
| **Near-square corners** | Border radii reduced (3px buttons, 4px panels) to match Krita's sharper, tool-focused aesthetic |
| **Orange accent** | `--accent #E07B39` directly matches Krita's branded "Dark Orange" official theme variant |
| **Docker metaphor** | App shell uses the docker/panel paradigm: compact 24px title bars, tab strips for grouped panels |
| **Canvas as distinct zone** | The main working area uses `--bg` (darkest), no border, clearly distinct from surrounding panels |
| **Icon-first interactions** | Primary tool actions are icon buttons; text labels are secondary and optional |
| **Breeze icon compatibility** | Phosphor Icons `regular` weight is a thin-outline SVG style that matches Krita/KDE Breeze icons |
| **Dense information layout** | Small type (11-13px), compact spacing, high density — matches Krita's information-rich panels |

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

Krita uses near-square corners — sharper than typical web apps. These values match that aesthetic.

| Context            | Value  |
|--------------------|--------|
| Buttons, inputs    | `3px`  |
| Cards, panels      | `4px`  |
| Badges, tags       | `2px`  |
| Modals             | `6px`  |

---

## Shadow & Elevation

Krita separates panels with 1px `--border` lines, not shadows. **Shadows are reserved for floating/detached windows only** — never between docked panels.

| Token             | Value                            | Usage                                     |
|-------------------|----------------------------------|-------------------------------------------|
| `--shadow-float`  | `0 4px 16px rgba(0,0,0,0.5)`    | Dropdowns, popovers, context menus        |
| `--shadow-modal`  | `0 8px 32px rgba(0,0,0,0.7)`    | Dialogs, drawers, full overlays           |

> Panel-to-panel separation always uses a 1px `--border` rule — not shadow. This matches Krita's flat panel aesthetic.

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
| `default`    | Base token values                                                        |
| `hover`      | `--state-hover` overlay (5% white) on background                        |
| `focus`      | 2px `--accent` ring at 50% opacity, 2px offset (see focus ring spec)    |
| `active`     | `--state-active` overlay (8% white) + `scale(0.97)` on `--dur-instant`  |
| `disabled`   | 40% opacity, `cursor: not-allowed`, no hover/active response             |
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

Desktop-only application. No mobile breakpoints.

**Minimum window size:** 900 × 600px

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Top bar (36px) — repository switcher                                      │
├──────────┬──────────────────────┬──────────────────────┬───────────────┤
│ Activity │ Sidebar              │ Main Panel           │ Inspector     │
│  48px    │  240–320px resizable │  flex: 1             │  280px toggle │
│  fixed   │  file tree, branches │  diff, commit canvas │  commit meta  │
└──────────┴──────────────────────┴──────────────────────┴───────────────┘
```

| Zone            | Width          | Content                                     |
|-----------------|----------------|---------------------------------------------|
| Top bar         | full width, 36px | Repository switcher (folder the user designated) |
| Activity bar    | 48px fixed     | Icon-only vertical strip, leftmost          |
| Sidebar         | 240–320px, resizable | File tree, branch list, history        |
| Main panel      | `flex: 1`      | Primary workspace — diff view, commit canvas |
| Inspector panel | 280px, toggleable | Commit details, layer metadata, blame    |

Panel borders use `--border`. Active/focused panel has no special indicator beyond content context.

**Repository switcher (top bar):** a flat button — folder icon + repo name + caret — opening a
dropdown menu of local repositories plus an "Add repository…" action. Local-only: no fetch/push/sync
affordances. Menu surface: `--surface-2`, 1px `--border`, `panel` radius, `--shadow-float`; the
active repo row shows a check in `--accent`. Closes on outside-click or Escape.

### Docker / Panel System

Krita users navigate a docker-based UI. Panels in Krita VCS follow the same conventions.

**Docker title bar** (24px height):
- Background: `--surface-2`
- Label: 11px, weight 500, `--text-muted`, uppercase
- Right side: 16px action icons (collapse, close, options)
- Bottom border: 1px `--border`

**Docker tab strip** (28px height, when panels are grouped):
- Inactive tab: `--surface` background, `--text-muted` label
- Active tab: `--surface-2` background, `--text` label, 2px `--accent` bottom border
- Tab padding: `sm` horizontal (8px)

**Panel dividers:**
- Always 1px `--border` between any two panels
- No shadow, no elevation — flat separators only

**Resize handle:**
- 4px draggable strip at resizable panel edges
- Default: `--border` color
- Hover/drag: `--accent` color
- **Vertical edge** (column resize, `cursor-col-resize`): sidebar width. **Horizontal edge** (row
  resize, `cursor-row-resize`): the art-diff canvas height — drag up/down; the height is clamped and
  the inner content scrolls when shrunk so it never overflows. Both use the shared `useResize` hook
  and persist to `localStorage`.

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
| Selected state | `--state-selected` background (the graph rail carries the lineage; no left border) |
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

### Tool Button (Krita-style flat icon button)

Two distinct button types — a critical Krita design pattern:

**Flat icon button** (toolbar / docker actions):
| State    | Background                       | Border | Notes                         |
|----------|----------------------------------|--------|-------------------------------|
| default  | transparent                      | none   | No visual chrome until hover  |
| hover    | `--state-hover` overlay          | none   |                               |
| active/checked | `--state-selected` overlay | none   | Toggled tool state            |
| focus    | `--accent` focus ring (2px)      | none   |                               |
| disabled | transparent, 40% opacity         | none   |                               |

- Hit target: 32×32px minimum
- Icon: 20px centered
- No text label by default (tooltip on hover provides context)

**Text action button** (OK / Cancel / confirm dialogs):
| State    | Background               | Border      | Text       |
|----------|--------------------------|-------------|------------|
| default  | `--surface-3`            | `--border` 1px | `--text` |
| hover    | `--state-hover` overlay  | `--border` 1px | `--text` |
| active   | `--state-active` overlay | `--border` 1px | `--text` |
| primary  | `--accent`               | none        | `--bg`     |
| disabled | 40% opacity              | `--border` 1px | `--text-muted` |

**Destructive button** (Delete / Reset / Discard):
- Default: `--surface-3` background, `--border` border
- Hover: `--danger` at 15% opacity background, `--danger` border, `--danger` text

### Slider / Range Control

Used for numeric properties — opacity, threshold, offset. Common in creative tool panels.

| Element | Value |
|---|---|
| Track height | 4px |
| Track background | `--surface-3` |
| Fill color | `--accent` |
| Thumb size | 12px circle |
| Thumb fill | `--accent` |
| Thumb border | 2px `--bg` (for contrast against fill) |
| Label position | Left: property name (`label` scale, 11px); right: current value (mono 11px) |
| Keyboard behavior | Arrow keys: ±1 unit; Shift+Arrow: ±10 units |

### Status Bar

Single bar fixed at the bottom of the app shell.

| Property | Value |
|---|---|
| Height | 24px |
| Background | `--surface` |
| Top border | 1px `--border` |
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
- **Flat over elevated** — panels are separated by 1px `--border` lines, never by shadows. Shadows only appear on floating elements (dropdowns, modals).
- **Borderless icon buttons** in toolbars and docker headers — no border, no background until hover. This is Krita's primary interactive pattern and what users expect in a creative tool.
- **Two button types, never mixed** — flat icon buttons for tool actions; bordered text buttons for dialog confirmations. Never put text labels on flat icon buttons or icon-only content on text action buttons.
