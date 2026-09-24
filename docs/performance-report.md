# Performance report

The Performance tab (in the activity bar, below the Version Map, or below Branches when Legacy
version history is on) shows two things about a painting's history: how long its operations take,
and how much disk the delta store saves compared with keeping a full copy of the painting at every
version. It only reports; it changes nothing about how the VCS works. For why the diff path is fast,
see [performance.md](performance.md). This page is about the metrics UI.

Two independent streams of data feed the tab, each measured in the one place that can see it
honestly.

## Operation timing: client-side, in `localStorage`

Durations (commit, branch switch, merge, diff) are measured on the frontend as the `invoke` round
trip, not in Rust. That's deliberate: it captures the whole wait the user sits through (IPC, the
engine's work and, for diffs, the lazily streamed layer rasters), which no single backend timer sees,
because the diff path is split across `commit_diff` (the fast first paint) and the out-of-order
`commit_layers` stream.

- [`src/lib/perf.ts`](../src/lib/perf.ts): `timed(repoPath, op, promise, meta?)` wraps a promise,
  records a `{ op, ms, ts, commitId? }` sample when it succeeds (a failure rethrows without
  recording, so a fast error can't skew the averages), and returns the resolved value. `meta(value)`
  pulls extra fields, such as the resulting commit id, off the resolved value into the sample.
  `readTimings` and `summarizeTimings` read the samples back, and `timingByCommit(samples)` collapses
  them to `{ saveMs, compareMs }` per commit id (the latest wins) for the per-version cards.
- Samples are kept in `localStorage["krita-vc:perf:<repoPath>"]`, capped at the last 100 per
  painting. Timing belongs to the machine, so it lives with the browser, not in the store.
- Three call sites are wrapped, with no signature changes elsewhere:
  - **Commit**: `ChangesPanel.doCommit`, around `invoke("commit_snapshot", …)`. `commit_snapshot`
    returns the new `Commit`, so `meta: (c) => ({ commitId: c.id })` ties the sample to its version.
    That's the Save time on each card.
  - **Switch and merge** (and create and delete): `repository.tsx`'s `branchMutation`, with the
    operation label derived from the command name through `BRANCH_OP`.
  - **Diff**: `useCommitDiff` and `useWorkingDiff` in [`repoData.ts`](../src/lib/repoData.ts). Only
    uncached calls are timed (a cache hit returns before the `timed` wrapper), so the numbers reflect
    the real cost of a backend diff. `useCommitDiff` tags its sample with the `commitId` (the card's
    Compare time); `useWorkingDiff` stays untagged, since unsaved changes aren't a version.

Merge and rollback create versions too, so they're tagged with their new commit id as well: merge
through `branchMutation`'s `meta` (operation `merge`), and rollback through a
`timed(…, "rollback", …)` wrapper in `rollbackToCommit`. `timingByCommit` treats `commit`, `merge`
and `rollback` as sources of save time. A plain `commit` is authoritative, and merge and rollback
only fill the save slot when there's no commit sample, so a fast-forward merge can't overwrite a
version's real commit time.

Diffs don't bump `refreshNonce`, so new diff samples appear the next time the panel mounts or after
any write, not live. A version's Save or Compare time reads "—" until you've done that operation in
the app on this machine (a version you've never opened has no Compare time). This refresh on mount
only applies to the samples, which are a cheap `localStorage` read that `PerformancePanel`
recomputes every time it mounts. The storage figures below don't recompute on mount at all.

## Storage savings: backend, forward-only

The headline number compares the hypothetical cost of one full copy of the painting per version with
the delta store's real size on disk.

- **Original size per version.** `CommittedFile` (`src-tauri/src/repo.rs`) carries an
  `original_size: u64`, the uncompressed size of the working file when the commit recorded it. It's
  captured for free at commit time (the scanner already read the file) and set everywhere a
  `CommittedFile` is built (commit, rollback, merge). It's `#[serde(default)]`, so older
  `commits.log` lines still deserialize.
- **The whole-store figure.** `commands::compute_storage_stats(&Repo)` (pure and testable) folds each
  commit's full tree with `commit::tree_at_commit` and sums `original_size` over its files, giving
  one `VersionRow` per commit. `naiveBytes` is the sum of those rows, `actualBytes` is the size of
  the store's `objects/` and `chains/` folders, and `savedBytes = naive − actual` (never below zero).
  It's exposed as the `repo_storage_stats` command (`useStorageStats` in `repoData.ts`), called once
  in `RepoShell` (`AppShell.tsx`) and passed to `PerformancePanel` as props. It isn't called inside
  the panel, which mounts and unmounts on every switch to and from the Performance view and would
  otherwise recompute the figures on every visit. It refetches only on a real `refreshNonce` bump (a
  write, or the refresh when the window regains focus; see
  [frontend-architecture.md](frontend-architecture.md#app-shell)). The per-version tree re-fold is
  quadratic (commits × files), which is fine for histories made by hand.
- **Stored bytes per version (`VersionRow.storedBytes`).** Each version also reports what it added to
  the store, by first-reference attribution. That reuses the GC mark, so it needed no change to the
  commit path and works on existing history. `object_size_map` builds an `objectName → bytes` map
  once from a walk of the loose objects plus `delta::read_pack_header` (mirroring `gc.rs`), and
  `stored_bytes_by_commit` walks the commits oldest first with a `seen` set. It maps each commit's
  files to object names (for a `.kra`, `kra::manifest_stream_key` plus the manifest's
  `kra::referenced_streams`, resolved through `repo.chains.chain(key)` → `Version::object_name()`;
  for any other file, `("file:{path}", content)`) and credits each object's bytes to the first commit
  that refers to it. So a version that changed a few tiles of a large painting shows a big saving:
  `originalBytes` is the whole painting (a full copy) and `storedBytes` is just the new delta. It
  counts objects only, so the stored bytes of all versions add up to at most `actualBytes` (it leaves
  out the chain shards, pack index overhead and objects orphaned by undo). The summary keeps the
  whole-store total, and the per-version cards use `storedBytes`.
- **Forward-only.** `original_size` has been recorded since the field was added. Versions committed
  before that count their files as 0 bytes, so on an older store `naive` can be smaller than `actual`
  and the saving reads 0. The panel detects this (`hasSavings = naive > actual`) and, instead of a
  misleading "5 MB stored against 12 KB of copies", shows the stored size with a note that savings
  will appear once a few new versions are recorded. As new commits come in, the figure climbs: a
  newly committed `.kra` counts its full size for each version, while the store keeps only deltas.

## The panel

[`src/components/vcs/PerformancePanel.tsx`](../src/components/vcs/PerformancePanel.tsx) is
self-contained (it reads `useRepository()` and `useArtistMode()` itself, like `BranchesPanel`) and
renders:

- **A summary card**: the average commit, switch, merge and compare times, and the total storage
  saved as a percentage ("Storage saved" in Artist Mode, "Storage saved vs full copies" otherwise).
- **A per-version card list**, newest first: one card per version titled `Version N` with its note,
  showing `storedBytes` against `originalBytes` ("full copy") with a "% saved" badge, plus a row of
  Save time and Compare time from `timingByCommit`. The badge uses `savedPercent(stored, fullCopy)`,
  which rounds to the nearest percent but is clamped so it never shows a misleading 100% while bytes
  were actually stored (1.2 MB of 349.6 MB shows 99%, not 100%), and never 0% while anything was
  saved. A true 100% only appears when a version stored nothing new.
- **A recent-operations log**: the five most recent timed operations, with relative timestamps.

The panel manages its own height (the Sidebar passes `scroll={false}` to `DockerPanel` for this
view): only the version cards scroll, so the summary stays at the top and the recent-operations log
stays pinned to the bottom however many versions there are. A card with no recorded original size
(from the forward-only history above) shows "Size not measured" instead of misleading zeros. Labels
follow Artist Mode (`Save`, `Compare` and `Version N` when it's on). In browser preview, with no
backend, the storage report isn't available and the timing lists show empty-state hints.

When Legacy version history is off, the Version Map fills the space beside the panel (see
[version-map.md](version-map.md#legacy-version-history)).

## Wiring a new tab (reference)

Adding this tab touched the usual four places: the `ActivityView` union and `ITEMS` array
(`ActivityBar.tsx`), the `PANEL_TITLE` record and content switch (`Sidebar.tsx`), and the new panel.
The new backend command is registered in the `generate_handler!` list in `src-tauri/src/lib.rs`.

## Files

| Concern | Location |
| --- | --- |
| Timing helper and `localStorage` | `src/lib/perf.ts` (`timed`, `readTimings`, `summarizeTimings`, `timingByCommit`) |
| Timed call sites | `src/components/vcs/ChangesPanel.tsx`, `src/lib/repository.tsx`, `src/lib/repoData.ts` |
| Storage stats hook and types | `src/lib/repoData.ts` (`useStorageStats`, `StorageStats`, `VersionRow`) |
| Storage figures and per-version attribution | `src-tauri/src/commands.rs` (`compute_storage_stats`, `object_size_map`, `stored_bytes_by_commit`, `repo_storage_stats`) |
| The `original_size` field | `src-tauri/src/repo.rs`, set in `commit.rs` and `branch.rs` |
| Panel UI | `src/components/vcs/PerformancePanel.tsx` |
| Tab wiring | `src/components/shell/ActivityBar.tsx`, `Sidebar.tsx` |
| Tests | `src-tauri/tests/engine.rs` (`commit_records_original_size_*`, `storage_stats_sums_per_version_*`) |
