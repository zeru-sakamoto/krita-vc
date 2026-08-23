# UI/UX Redesign Tracker

Screens are redesigned **one at a time**, driven by direct prompts — not one-shot. This doc is the
shared state across sessions: what's in scope, and which screens are done.

## Workflow

1. User names a screen (or picks the next `Not started` row below).
2. Implement the redesign directly in the real app (no separate mockup/approval step).
3. Review live in the dev server / Tauri shell, iterate on feedback.
4. Once approved: flip its row to `Done`, and update the relevant section(s) of
   [`DESIGN.md`](DESIGN.md) in place so the spec keeps matching the shipped UI.

## Scope

- **Locked — do not change:** the color palette and the 8-theme system (`DESIGN.md`'s Color
  Palette section, `src/styles/global.css`, `src/lib/theme.tsx`).
- **Open:** everything else — layout, typography, spacing, radii, shadows, motion, iconography,
  and component patterns.

## Foundation — done

The app-wide style pass landed before any screen work: **framed bento** (one continuous chrome
frame, closed on all four sides; Sidebar / Main / Inspector as raised cards in a recessed well),
**tactile depth** (everything raised, sinks on press), and **glass only on surfaces that float
over content**. Density: 8px gutters · 14px panel radius · 8px control radius · 22px well radius
(concentric with the panel radius + gutter, so the frame's corners curve as one continuous piece
instead of meeting at right angles).

What it changed, and what every screen below now inherits:

| Area | Change |
|---|---|
| Tokens | New radii incl. `--radius-well`; `--shadow-raised`/`-pressed`/`-well`; glass + scrim tokens; `--color-state-*` overlays. Five utility classes: `.raised` `.tactile` `.inset-well` `.glass` `.row-selected` |
| Shell | Frame + rounded well + three cards, closed ring on all four sides; all inter-panel borders removed; resize handle is now the gutter; activity-bar icons spaced to match the well's rhythm |
| Primitives | `Button` and `IconButton` are tactile (icon chips replace the old flat/borderless pattern); `Menu`, `Modal`, toast are glass |
| Overlays | One `--scrim` token replaces three hardcoded blacks (also fixes light themes) |
| Cleanup | Side-stripe selection idiom removed at all 7 sites; `bg-white/5` tokenized app-wide |

Three follow-up passes on top of the initial foundation drop (frame corner fillets, icon spacing,
closing the right edge of the ring) are folded into the table above — `DESIGN.md` reflects the
current shipped state, not the history of how it got there.

**Read `DESIGN.md` before starting any row below.** Screens must share this system, not vary from
it — if a screen needs something the system doesn't have, amend `DESIGN.md` first.

**Status note:** every row below is still `Not started`. The foundation pass touched these files'
outer surfaces (elevation, radii, spacing) but not their interiors — that's the work still to
come, one screen at a time.

## Screen checklist

Rows below are about each screen's **interior**. Their surfaces, radii, elevation, selection and
hover states already come from the foundation.

### Shell (`src/components/shell/`)

| Status | Screen | File | Notes |
|---|---|---|---|
| Not started | Welcome / empty state | `AppShell.tsx` (`WelcomeShell`) | Shown when no repository is selected |
| Not started | Repo shell layout | `AppShell.tsx` (`RepoShell`) | Wires the 4 zones + StatusBar |
| Not started | Top bar | `TopBar.tsx` | Repository switcher; doubles as custom title bar |
| Not started | Activity bar | `ActivityBar.tsx` | Changes / History / Branches / Performance + Settings gear |
| Not started | Sidebar | `Sidebar.tsx` | Resizable; content swaps per active view |
| Not started | Inspector | `Inspector.tsx` | Commit metadata / unsaved-changes detail |
| Not started | Status bar | `StatusBar.tsx` | Active file, branch, commit count, save progress |
| Not started | Settings modal | `SettingsModal.tsx` | Appearance / Set-Aside / Storage tabs |
| Not started | Docker panel (shared shell) | `DockerPanel.tsx` | Reusable title-bar panel wrapper |
| Not started | Busy overlay | `BusyOverlay.tsx` | Full-screen block during writes |
| Not started | Tour overlay | `TourOverlay.tsx` | First-launch spotlight walkthrough |

### Main canvas

| Status | Screen | File | Notes |
|---|---|---|---|
| Not started | Main panel | `src/components/MainPanel.tsx` | Hosts `DiffView`; loading/error/empty states |

### VCS panels & components (`src/components/vcs/`)

| Status | Screen | File | Notes |
|---|---|---|---|
| Not started | Changes panel | `ChangesPanel.tsx` | Working-tree changes, stage/unstage, discard |
| Not started | Branches panel | `BranchesPanel.tsx` | Switch/merge/delete, new-branch modal |
| Not started | Performance panel | `PerformancePanel.tsx` | Operation timing + storage-saved metrics |
| Not started | Commit graph | `CommitGraph.tsx` / `CommitGraphRail.tsx` | History lineage graph + lane rail |
| Not started | Commit card | `CommitCard.tsx` | Single commit row |
| Not started | Branch badge | `BranchBadge.tsx` | Pill badge for branch name |
| Not started | File status chip | `FileStatusChip.tsx` | M/A/D/U/R/C indicator |
| Not started | Diff view (dispatch) | `DiffView.tsx` | Routes per-file diff rendering by kind |
| Not started | Art diff view | `ArtDiffView.tsx` | `.kra` diff — split/slider modes, highlight toggle |
| Not started | Art canvas | `ArtCanvas.tsx` | SVG-composited layer/canvas renderer |
| Not started | Compare slider | `CompareSlider.tsx` | Before/after swipe compare |
| Not started | Layer stack panel | `LayerStackPanel.tsx` | Layer list, thumbnails, composite + palette rows |
| Not started | Palette diff view | `PaletteDiffView.tsx` | Color-palette diff swatches |
| Not started | Branch dialogs | `BranchDialogs.tsx` | Create-branch, save-first (dirty-tree guard) |
| Not started | Stash dialogs | `StashDialogs.tsx` | Set-aside, pick-stash, conflict modal |

### UI primitives (`src/components/ui/`)

Unlike the screens above, these four got a real redesign as part of the foundation pass (not just
inherited tokens) — they're the actual chokepoints the tactile/glass direction was built through.
Reopen a row only if a later screen surfaces something these don't already cover.

| Status | Screen | File | Notes |
|---|---|---|---|
| Done | Button | `Button.tsx` | Tactile — raised, sinks on `:active`. Default / primary / destructive |
| Done | Icon button | `IconButton.tsx` | Raised icon chip, was flat/borderless; toggled-on = pressed look |
| Done | Menu | `Menu.tsx` | Glass dropdown |
| Done | Modal | `Modal.tsx` | Glass dialog + `.scrim` backdrop, now uses `--radius-modal`/`--shadow-modal` |

Status values: `Not started` · `In progress` · `Done`.
