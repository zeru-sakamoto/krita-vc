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

## Screen checklist

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

| Status | Screen | File | Notes |
|---|---|---|---|
| Not started | Button | `Button.tsx` | Default / primary / destructive |
| Not started | Icon button | `IconButton.tsx` | Flat icon button |
| Not started | Menu | `Menu.tsx` | Dropdown menu |
| Not started | Modal | `Modal.tsx` | Themed dialog shell |

Status values: `Not started` · `In progress` · `Done`.
