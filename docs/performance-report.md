# Performance report

The **Performance** tab (activity bar, below Branches) surfaces two things about a repository:
how long its operations take, and how much disk the delta store saves versus naively keeping a
full copy of every tracked file at every version. It is a reporting surface only — it changes
nothing about how the VCS works. For *why* the diff path is fast, see
[performance.md](performance.md); this doc is about the metrics UI.

Two independent data streams feed the tab, measured in the two places that can see them honestly.

## Operation timing — client-side, localStorage

Durations (commit / branch switch / merge / diff) are measured as the **invoke round-trip** on the
frontend, not in Rust. That's deliberate: it captures the whole latency the user actually waits
through — IPC, engine work, and (for diffs) the lazily-streamed layer rasters — which no single
backend timer sees, because the diff path is split across `commit_diff` (fast first paint) and the
out-of-order `commit_layers` stream.

- [`src/lib/perf.ts`](../src/lib/perf.ts) — `timed(repoPath, op, promise, meta?)` wraps a promise,
  records a `{ op, ms, ts, commitId? }` sample on success (failures rethrow unrecorded so a fast
  error can't skew the averages), and returns the resolved value. `meta(value)` pulls extra fields
  (the resulting commit id) off the resolved value into the sample. `readTimings` /
  `summarizeTimings` read them back; `timingByCommit(samples)` collapses them to
  `{ saveMs, compareMs }` per commit id (latest wins) for the per-version cards.
- Samples persist to `localStorage["krita-vc:perf:<repoPath>"]`, capped at the last **100** per
  repo. Timing is inherently per-machine, so it lives with the browser, not the `.kvc/` store.
- Three call sites are wrapped, with no signature changes elsewhere:
  - **commit** — `ChangesPanel.doCommit` around `invoke("commit_snapshot", …)`; `commit_snapshot`
    returns the created `Commit`, so `meta: (c) => ({ commitId: c.id })` ties the sample to its
    version (that's the **Save time** shown on each card).
  - **switch / merge** (and create / delete) — `repository.tsx`'s `branchMutation`; the op label is
    derived from the command name via `BRANCH_OP`.
  - **diff** — `useCommitDiff` / `useWorkingDiff` in [`repoData.ts`](../src/lib/repoData.ts). Only
    *uncached* invokes are timed (a cache hit returns before the `timed` wrapper), so the numbers
    reflect real backend diff cost. `useCommitDiff` tags the sample with its `commitId` (the card's
    **Compare time**); `useWorkingDiff` stays untagged — unsaved changes aren't a version.

Merge and rollback create versions too, so they're tagged with their new commit id as well: merge
via `branchMutation`'s `meta` (op `merge`), rollback via a `timed(…, "rollback", …)` wrap in
`rollbackToCommit`. `timingByCommit` treats `commit`/`merge`/`rollback` as save-time sources — a
plain `commit` is authoritative, and merge/rollback only fill the save slot where no commit sample
exists (so a fast-forward merge can't clobber a pre-existing version's real commit time).

Because diffs don't bump `refreshNonce`, new diff samples surface the next time the panel mounts or
after any mutation, rather than live. A version's Save/Compare time reads "—" until you've done that
op in-app on this machine (e.g. a version you've never opened has no Compare time).

## Storage savings — backend, forward-only

The headline number compares the hypothetical "one full copy of every file per version" cost
against the delta store's real footprint.

- **Original size per version.** `CommittedFile` (`src-tauri/src/repo.rs`) carries an
  `original_size: u64` — the uncompressed byte size of the working file when the commit recorded it.
  It's captured for free at commit time (the scanner already read it) and set at every
  `CommittedFile` construction site (commit, rollback, merge). It is `#[serde(default)]`, so legacy
  `commits.log` lines still deserialize.
- **The whole-store math.** `commands::compute_storage_stats(&Repo)` (pure, testable) folds each
  commit's full tree with `commit::tree_at_commit` and sums `original_size` over every file — one
  `VersionRow` per commit. `naiveBytes` is the sum of those rows; `actualBytes` is a recursive
  byte-size walk of `.kvc/objects/` + `.kvc/chains/`; `savedBytes = naive − actual` (saturating).
  Exposed as the `repo_storage_stats` Tauri command (`useStorageStats` in `repoData.ts`).
  `// ponytail:` the per-version tree re-fold is O(commits × files) — fine for hand-scale histories.
- **Per-version stored bytes (`VersionRow.storedBytes`).** Each version also reports what it
  *added* to the store, by **first-reference attribution** (reuses the GC mark, so no commit-path
  change and it works retroactively): `object_size_map` builds `objectName → bytes` once from a
  loose-object walk + `delta::read_pack_header` (mirrors `gc.rs`); `stored_bytes_by_commit` then
  walks commits oldest-first with a `seen` set, mapping each commit's files to object names
  (`("file:{path}", content)` for generic files; `kra::manifest_stream_key` + the manifest's
  `kra::referenced_streams` for `.kra`, resolved through `repo.chains.chain(key)` →
  `Version::object_name()`) and crediting each object's bytes to the **first** commit that
  references it. So a version that changed one small file in a big tree shows a huge saving:
  `originalBytes` = the whole tree (a full copy), `storedBytes` = just the new delta. Objects-only,
  so `Σ storedBytes ≤ actualBytes` (excludes `.kvc/chains/` shard bytes, pack index overhead, and
  undo orphans) — the summary keeps the whole-store total; the per-version cards use `storedBytes`.
- **Forward-only caveat.** `original_size` is recorded from the moment the field existed. Versions
  committed *before* that count their files as 0 bytes until re-committed, so on an existing repo
  `naive` can trail `actual` and the saving reads 0. The panel detects this
  (`hasSavings = naive > actual`) and, instead of a misleading "5 MB stored vs 12 KB copies," shows
  the stored size plus a note that savings appear once a few new versions are recorded. Make fresh
  commits and the figure climbs (a re-committed `.kra` counts its full size per version, while the
  store keeps only deltas).

## The panel

[`src/components/vcs/PerformancePanel.tsx`](../src/components/vcs/PerformancePanel.tsx) is
self-contained (pulls its own `useRepository()` + `useArtistMode()`, like `BranchesPanel`). It
renders:

- a **summary card** — average commit/switch/merge/compare times, and total storage saved with a
  percentage;
- a **per-version card list** (newest-first) — one card per version titled `Version N` + its commit
  message, showing `storedBytes` vs `originalBytes` ("full copy") with a **% saved** badge, plus a
  **Save time** / **Compare time** row from `timingByCommit`. The badge uses `savedPercent(stored,
  fullCopy)` — nearest-rounded but clamped so it never reads a misleading **100%** while bytes were
  actually stored (1.2 MB of 349.6 MB → 99%, not 100%), nor **0%** while any bytes were saved; a
  true 100% only shows when a version stored nothing new;
- a **recent-operations log** — the 5 most recent timed operations with relative timestamps.

The panel owns its own height (the Sidebar passes `scroll={false}` to `DockerPanel` for this view):
only the version-card list scrolls, so the summary stays on top and **Recent operations stays pinned
to the bottom** regardless of version count. A card with no recorded original sizes (forward-only
history) shows "Size not measured" instead of misleading 0s. Labels respect Artist Mode
(`Save`/`Compare`/`Version N` when on). In browser preview (no backend) the storage report is
unavailable and the timing lists show empty-state hints.

## Wiring a new tab (reference)

Adding the tab touched the standard four points: the `ActivityView` union + `ITEMS` array
(`ActivityBar.tsx`), the `PANEL_TITLE` record + content switch (`Sidebar.tsx`), and the new panel.
The new backend command is registered in the `generate_handler!` list in `src-tauri/src/lib.rs`.

## Files

| Concern | Location |
| --- | --- |
| Timing helper + localStorage | `src/lib/perf.ts` (`timed`, `readTimings`, `summarizeTimings`, `timingByCommit`) |
| Timed invoke sites | `src/components/vcs/ChangesPanel.tsx`, `src/lib/repository.tsx`, `src/lib/repoData.ts` |
| Storage stats hook + types | `src/lib/repoData.ts` (`useStorageStats`, `StorageStats`, `VersionRow`) |
| Storage + per-version attribution | `src-tauri/src/commands.rs` (`compute_storage_stats`, `object_size_map`, `stored_bytes_by_commit`, `repo_storage_stats`) |
| `original_size` field | `src-tauri/src/repo.rs`, set in `commit.rs` / `branch.rs` |
| Panel UI | `src/components/vcs/PerformancePanel.tsx` |
| Tab wiring | `src/components/shell/ActivityBar.tsx`, `Sidebar.tsx` |
| Tests | `src-tauri/tests/engine.rs` (`commit_records_original_size_*`, `storage_stats_sums_per_version_*`) |
