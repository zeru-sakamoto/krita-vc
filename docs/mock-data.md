# Mock Data Model

The whole UI is currently fed by mock modules in `src/data/`. **Nothing talks to git or the
filesystem** — these exist so the frontend can be built and reviewed against [`DESIGN.md`](../DESIGN.md)
ahead of the Rust/Tauri backend.

## Domain types (`src/types.ts`)

| Type | Used for |
|------|----------|
| `FileStatus` | `M`odified, `A`dded, `D`eleted, `U`ntracked, `R`enamed, `C`onflicted. |
| `FileChange` | A `{ path, status }` pair. |
| `Branch` / `BranchKind` | Branch name + `local` \| `current` (local-only — no remotes). |
| `Repository` | `{ id, name, path }` — a local folder the user has designated (local-only, no remotes). |
| `Commit` | `id`, `hash`, `message`, `author`, ISO `timestamp`, `changes[]`, `parents[]` (lineage for the graph; first parent = mainline, length > 1 = merge), optional `branch`. |
| `DiffEntry` = `ArtDiff \| TextDiff \| PaletteDiff` | A single file's diff, discriminated by `kind`. |
| `TextDiff` | `kind:"text"` — line diff (`DiffLine[]`); shown as a one-line friendly summary in Artist Mode, or a code-style diff with it off. For generic config/settings files. |
| `PaletteDiff` / `PaletteSwatch` / `SwatchChange` | `kind:"palette"` — structured diff for `.gpl` color palettes. `PaletteSwatch` holds `name`, `before`/`after` hex strings, and `SwatchChange` (`"added" \| "removed" \| "modified" \| "unchanged"`). Always renders as color swatches — not Artist Mode gated. |
| `ArtDiff` / `ArtLayer` / `ChangeRegion` | `kind:"art"` — visual layer diff (see [visual-diff-viewer.md](visual-diff-viewer.md)). |
| `WorkingChange` | `{ change: FileChange, staged: boolean }` for the Changes tab. |

## Mock modules (`src/data/`)

### `mockData.ts`

| Export | Shape | Consumed by |
|--------|-------|-------------|
| `MOCK_REPOSITORIES` | `Repository[]` | `RepositoryProvider` (seeds the repo list; selection persists). |
| `MOCK_BRANCHES` | `Branch[]` | `AppShell` (current branch), `BranchesPanel`. |
| `MOCK_COMMITS` | `Commit[]` | `Sidebar`/`CommitGraph`, `Inspector`, `StatusBar`. A small DAG: `main` with a `character-redesign` branch that diverges off the color-flats commit and merges back. |
| `MOCK_DIFF_BY_COMMIT` | `Record<commitId, DiffEntry[]>` | `MainPanel`/`DiffView`. `.kra` entries reference `ART_DIFFS`; others are inline `TextDiff`. |
| `MOCK_WORKING_CHANGES` | `WorkingChange[]` | `ChangesPanel`. |

`MOCK_REPOSITORIES` paths are illustrative — there is no native folder picker yet (no Tauri dialog
plugin), so every repo currently shows the same `MOCK_COMMITS`/`MOCK_BRANCHES`. Per-repo data arrives
with the backend.

### `mockArt.ts`

Generated inline-SVG artwork + compositing helpers (`layersBody`, `wrapSvg`, `compositeSvg`,
`blendCss`) and the pre-built `ART_DIFFS` map. Detailed in
[visual-diff-viewer.md](visual-diff-viewer.md).

### `mockPalette.ts`

| Export | Shape | Consumed by |
|--------|-------|-------------|
| `PALETTE_DIFFS` | `Record<string, PaletteDiff>` | `mockData.ts` — keyed by a `"<palette>_<commitId>"` slug; the relevant entry is attached to `MOCK_DIFF_BY_COMMIT`. |

Contains structured `PaletteDiff` entries with full `PaletteSwatch` arrays (unchanged, modified,
added, and removed swatches) so `PaletteDiffView` can render the color grid. Replace when the
real backend supplies parsed `.gpl` diffs.

## Swapping in the real backend

The data exports above are the seam. When the Rust side is ready:

1. Add `#[tauri::command]`s in `src-tauri/src/lib.rs` (e.g. `list_commits`, `diff_commit`,
   `working_changes`, `branches`, plus a folder picker via `tauri-plugin-dialog` for "Add
   repository") and register them in `generate_handler!`.
2. Replace the mock reads in `AppShell`/`RepositoryProvider` (and the sidebar panels) with
   `invoke(...)` calls returning the same domain types — `Repository[]`, `Commit[]`, `Branch[]`,
   `DiffEntry[]`, `WorkingChange[]` — keyed by the selected repository path.
3. For art diffs, return per-layer rasters + region geometry instead of SVG strings; delete
   `mockArt.ts`. Component props (`DiffEntry` etc.) do not change.

Keeping the component/prop boundaries identical means the swap is confined to `src/data/` and the
fetch points in `AppShell`.
