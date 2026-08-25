# Data integrity — what the engine already does

An artist's `.kra` is often the only copy of weeks of work, and this VCS is **local-only**: there
is no remote to re-pull from if the store goes bad. So the engine's integrity posture is
"never be the reason a file is lost", and it is built out of a handful of cheap, boring
invariants rather than one big transaction system.

This document lists the measures that are **in the code today**, with where they live.

---

## 1. Concurrency: one writer per repository

| Measure | Where |
| --- | --- |
| **Real OS-level exclusive lock** over each `.kvc/` store (`.kvc/kvc.lock`, `File::try_lock` → `LockFileEx`/`flock`). Every mutating entry point takes it — Tauri commands *and* the `kvc` CLI — so a Krita-plugin commit can't interleave with a desktop commit/switch/GC into a torn write. | `repo.rs` — `RepoLock::acquire` |
| **No stale-lock state.** The lock is released by the OS when the holding process's handle closes — cleanly, on a panic unwind, or on a force-kill. There is deliberately no `impl Drop` and no marker file to clean up. | `repo.rs` — `RepoLock` |
| **Lock attribution.** A `kvc.lock.info` sidecar records a present-participle label (`"committing"`, `"switching branches"`) and its mtime gives the age, so a blocked caller's `Locked` error says *what* is holding the repo and *for how long*. The sidecar is a separate, never-locked file because Windows enforces a locked byte range against ordinary reads. | `repo.rs` — `write_lock_info`, `lock_holder_description` |
| **Reads take no lock**, by design — `status`, `branches`, `stash-list` — so the Krita plugin's 1.5 s poll never contends with, or blocks, a write. | `bin/kvc.rs` |
| **User-facing reads re-check for a stale snapshot.** `branches.json` carries a `generation` counter bumped on every write (`Repo::save`/`Repo::save_branches`); `list_commits`, `commit_diff`, `working_diff` and `list_branches` re-read just that counter before and after, retrying (bounded) if a write landed mid-read — a cheap `branches.json`-sized re-read, not a doubled full read. Deliberately **not** applied to the CLI's poll trio above, which must stay exactly as cheap as before: the race this closes is narrow and benign (a stale-but-consistent snapshot, never corruption — `write_atomic`'s rename is already the atomicity boundary), so it's worth closing on paths the artist actually looks at but not worth taxing the poll for. | `repo.rs` — `Branches::generation`; `commands.rs` — `read_consistent` |
| **Heavy operations capped at two concurrently** (`cpu::heavy_permit`, a tokio semaphore taken by `commands::run_heavy`). Cancelling a diff in the UI does *not* cancel the backend, and rapid history clicking otherwise stacked unbounded 64 MB decode buffers. | `cpu.rs`, `commands.rs` |
| **Full-screen busy overlay** during every write op (commit, switch, merge, create/delete branch, rollback, undo, cleanup) so a stray click can't race a file rewrite. | `src/components/shell/BusyOverlay.tsx` |

## 2. Crash safety of the store

| Measure | Where |
| --- | --- |
| **Atomic state writes.** Every `.kvc/` state file (`index.json`, `branches.json`, `stashes.json`, `config.json`, chain shards, the rewritten commit log) is written to a sibling `*.tmp` and `rename`d over the target — replacing atomically on both Windows and POSIX. | `repo.rs` — `write_atomic`, `write_json`, `write_chains_file` |
| **Atomic working-tree writes.** Every restored file — branch switch, rollback, discard, stash pop, single-file restore — goes through the same temp-then-rename, so a crash, power cut or full disk mid-write can never hand Krita a truncated `.kra`. The temp suffix is *appended* (`foo.kra.kvctmp`), not substituted: `with_extension` would collapse `a.kra` and `a.gpl` onto one temp path, and `.kvctmp` can never match `scan::is_supported`, so a leftover is invisible to the scanner. | `repo.rs` — `write_file_atomic`; `commit.rs`, `stash.rs`, `commands.rs` |
| **Atomic loose-object writes.** `write_loose` dedups by *existence*, so a torn object would be trusted forever — every later commit storing that content would skip the write. Temp-then-rename makes an interrupted write invisible instead; GC sweeps the leftover, since it deletes anything in `objects/` it can't name. | `delta.rs` — `write_loose` |
| **Durability, not just atomicity.** The temp file is `fsync`ed *before* the rename (plus a parent-directory fsync on POSIX; on Windows the rename is journaled). Without it the rename can land while the contents are still in the page cache — atomic against a process crash, but a power cut still yields a zero-length `branches.json`. The `commits.log` append is fsynced too: `save()`'s "tips go last" ordering is only an ordering if the line is on the platter before `branches.json` names it. | `repo.rs` — `sync_write`, `sync_parent_dir`, `flush_commits` |
| **Not fsynced, deliberately:** loose-object and pack payloads. That's the commit hot path — thousands of tiny objects — and the save ordering already makes an unsynced object harmless: a lost one is an unreachable orphan, and the atomic write above means it can never be *partially* trusted. | `delta.rs` |
| **Pack files are temp-then-rename too**, and are named by the blake3 of their index — so a pack's name attests to its contents. | `delta.rs` — pack writer |
| **Write ordering is load-bearing.** `save()` writes index → chains → commit log → `branches.json` → `stashes.json`. Tips go **last**, so a torn commit-log append is always an *unreachable orphan record*, never a dangling branch tip. `stashes.json` follows for the same reason: a stash record must never outlive the chain content it points at. | `repo.rs` — `Repo::save` |
| **Append-only commit log.** A commit appends one JSON line; the log is only rewritten when history was genuinely truncated (undo, GC), flagged via `note_commits_truncated`. No O(history) rewrite on the hot path, so the crash window per commit is one line. | `repo.rs` — `flush_commits` |
| **Torn-line tolerance on load.** A partial trailing line in `commits.log` (crash mid-append) is dropped on read and flags a rewrite so the fragment is scrubbed rather than appended onto. | `repo.rs` — `read_commits` |
| **Narrow flushes for narrow edits.** `save_config`, `save_branches`, `save_stashes` exist so an `open_light` repo (partial in-memory state) never rewrites `index`/`commits` from state it didn't fully load. | `repo.rs` |
| **One previous generation kept for small state files.** `index.json`, `branches.json` and `stashes.json` each get a sibling `.bak` — the pre-write copy, taken before every write — and a decode failure on open falls back to it instead of refusing to open the repo at all; the primary self-heals on the next save. `branches.json` is the single point of failure for the whole repository (lose it and every commit is unreachable, still on disk but nothing points at it), so this is cheap insurance for kilobyte-scale files. | `repo.rs` — `write_json_with_backup`, `read_json_with_backup` |
| **Stale `*.tmp` sweep.** Crash leftovers from interrupted atomic writes are found and reclaimed by the cleanup pass — nothing else ever removes them. | `gc.rs` — `stale_tmp_files` |
| **Legacy formats retire only after the new one lands.** The chains monolith is deleted only once every shard is written; `commits.json` only once `commits.log` is safely in place. There is no window without valid state. | `repo.rs` — `ChainStore::flush`, `flush_commits` |

## 3. Content integrity of stored data

| Measure | Where |
| --- | --- |
| **Content addressing throughout.** Objects, tiles, composite pixel blocks, cache entries and index entries are all keyed by **blake3** of their bytes. Identical content deduplicates instead of being re-stored, and an object's name is a claim about its contents. | `repo.rs` — `hash_bytes`; `delta.rs`; `kra.rs` |
| **Every bsdiff patch is round-trip verified at write time.** After computing the patch, the engine immediately `bspatch`es it back against the base and compares byte-for-byte; a mismatch falls back to a full zstd snapshot. This is what guarantees every stored version rebuilds — a corrupt chain can never reach a commit and brick it. | `delta.rs` — `prepare_stream_opts` |
| **Unreconstructable heads degrade instead of failing.** If the current chain head can't be rebuilt, the new version stores as a full snapshot rather than patching onto broken data — a damaged chain heals forward. | `delta.rs` — `prepare_stream_opts` |
| **Patches are named by (result, base).** A patch is only valid against its base, so two streams reaching identical content from different bases can never collide on one object name. | `delta.rs` |
| **Explicit format tag + config version.** Chain shards carry a `KVCC2` magic prefix (bincode is not self-describing, so a field change needed a version marker); pre-tag shards decode through a legacy struct. `Config.version` exists, and every later-added knob is `#[serde(default)]` so old configs keep deserializing. | `repo.rs` — `CHAINS_MAGIC`, `decode_chains`, `Config` |
| **Unreadable shard = empty shard, not a crash.** A missing or corrupt chain shard returns `None` and the repo still opens, keeping the rest of history reachable. | `repo.rs` — `read_chains_file` |
| **Verified reads where a bad byte becomes the artist's file.** Write-time verification covers engine bugs but not bit rot, a failing disk, or something outside the app editing `.kvc/`. So the operations that write reconstructed bytes into the working tree — switch, rollback, discard, stash pop, restore-file — set `Repo::verify_reads` and re-hash every object `reconstruct` rebuilds, refusing with `Corrupt` on a mismatch. `reconstruct` recurses along the patch chain, so the whole chain is verified link by link. | `repo.rs` — `verify_reads`; `delta.rs` — `reconstruct` |
| **Off for diffs and previews, deliberately.** That's the hottest loop in the app, and a wrong pixel in a preview is not data loss. A test pins both halves — the restore refuses, the diff path still returns. | `tests/engine.rs` |
| **Restores from disk are checked against the manifest.** The incremental `.kra` path lifts unchanged zip entries and tiles straight out of the working file, but only when the entry's **crc32 + uncompressed size** match what the manifest recorded at commit time; a pre-crc `(0,0)` manifest is never trusted, and any mismatch falls back to the full store rebuild. | `kra.rs` — `materialize_kra` |
| **Pixel-exact, not byte-exact, on restore — and that's documented, not just implied.** The composite (`mergedimage.png`) is re-encoded from content-addressed pixel blocks and tile entries are rewritten deflate-fast, so a restored `.kra` is deliberately not byte-identical to what Krita originally saved — a restore cannot be verified by comparing file hashes. What the store guarantees is **pixel** equality, which is why it's pinned by a regression test rather than left implicit: an artist depending on byte-exact round-trips (external tooling, signatures) would otherwise be surprised. | `kra.rs` — `materialize_kra`; `tests/engine.rs` — `composite_tiles_dedup_and_pixel_roundtrip` |

## 4. Garbage collection that can't eat live data

Nothing in the engine deletes stored data on its own — `undo` orphans a commit's objects,
`delete_branch` strands whole histories, both deliberately, because content-addressed orphans are
harmless. Reclaiming is an explicit, user-triggered "Clean up storage".

| Measure | Where |
| --- | --- |
| **Mark and sweep from every branch tip**, not just the current one. | `gc.rs` |
| **Stashes are GC roots.** Nothing in `commits.log` references stash content, so without this the shelf would be collected out from under the artist. | `gc.rs` |
| **Patch bases are closed over.** A patch is useless without the chain back to its full snapshot, so reachability follows `Version.base`. | `gc.rs` |
| **State files are rewritten *before* any object is deleted.** A crash mid-sweep leaves only re-collectable orphans — never a live reference to missing data. | `gc.rs` |
| **Dry-run first.** The Settings confirm modal is powered by a real dry run, so the user approves the actual numbers. | `gc.rs`, `SettingsModal` |
| **Pack rewrites gated at >25% dead** (with a unit test on the gate), and kept dead bytes are excluded from the report — the report states what the run actually frees. | `gc.rs` — `worth_rewriting` |
| **Cache reclaim is reported separately** (`cacheBytesReclaimed`) because the raster cache is regenerable and its loss is not a data loss. | `gc.rs` |
| **Sweep victims are quarantined, not deleted.** Dead loose objects and dead/rewritten packs move to `.kvc/trash/<timestamp>/` (a same-volume `rename` — the same cost class as the delete it replaces) instead of being unlinked outright. If the reachability logic is ever wrong, or cleanup runs right after a branch delete (an ordinary sequence), the data stays recoverable by hand instead of gone with no remote to re-fetch from. | `gc.rs` — `quarantine` |
| **Quarantined trash ages out on its own.** Trash run-directories older than 14 days are permanently pruned on the next *real* cleanup (never during a dry run), reported separately as `trashBytesPruned` — bounded, self-pruning retention rather than unbounded growth. | `gc.rs` — `prune_trash` |

## 4b. Checking that history is intact

"Is my history intact?" used to be unanswerable — for the artist and for a bug report. A
read-only **check** pass now answers it. It shares GC's reachability walk (`gc::mark_live`, whose
one `tolerant` flag is the whole difference between the callers: GC must fail hard on a manifest
it can't load, because sweeping on a partial mark would delete live data, while check exists
precisely to report it and keep walking). It takes no lock and writes nothing.

| Measure | Where |
| --- | --- |
| **Detects the five ways history goes unreachable**: a missing object, a broken chain, a branch tip naming a version that isn't in the log, an undecodable commit-log line, and an unparseable pack. | `check.rs` — `check_repository` |
| **Log damage in the middle is reported, not swallowed.** `read_commits` stops at the first bad line and drops everything after — right for a torn tail, wrong for corruption mid-file, which would quietly shorten history. The check scans the whole file. | `check.rs` |
| **A corrupt pack is named directly.** Everywhere else in the engine an unparseable pack is silently skipped, turning real corruption into a confusing `MissingObject` per contained object. | `check.rs`, `delta.rs` — `read_pack_header` |
| **Findings are a successful run.** Both the Tauri command and `kvc check` report problems in the normal result; `{"error": …}` means the check itself failed, and the Krita plugin can't tell those apart otherwise. | `commands.rs` — `check_repository`; `bin/kvc.rs` — `run_check` |
| **Surfaced next to "Clean up storage"** in Settings → Storage, as "Check for problems…". Read-only, so it raises no busy overlay. | `SettingsModal.tsx` — `CheckModal` |
| **Opt-in bit-rot scrub.** `check_repository`/`kvc check` take a `scrub` flag (off by default, never run automatically — it's IO over the whole store) that additionally re-hashes every live version's content, via the same `Repo::reconstruct_cached` + `Repo::verify_reads` machinery the restore path already uses — a walk over the objects, not new verification logic. One bad version doesn't stop the walk (`corruptContent` problems accumulate). Reachable from Settings → Storage → "Check for problems…" → "Also read back every version (slower)", or `kvc check --scrub true`. | `check.rs` — `check_repository(repo, scrub)` |
| **Still deliberately not included:** any `--repair` mode. Detection is the part this covers. | — |

## 5. Working-tree safety

| Measure | Where |
| --- | --- |
| **Dirty-tree guard.** Branch switch and merge refuse while the working tree has uncommitted changes (`DirtyTree`), rather than overwriting them. | `branch.rs` — `ensure_clean` |
| **Stable error prefixes as a contract.** `"unsaved changes: …"` and `"stash conflict: …"` are matched by the frontend to raise the right recovery dialog (save / set aside / go to Changes). They're deliberately distinct strings. | `error.rs`, `BranchDialogs.tsx`, `StashDialogs.tsx` |
| **Rollback is non-destructive.** Restoring an old version records a **new** commit rather than rewinding history, so it is itself undoable. | `commit.rs` — `rollback_to_commit` |
| **`delete_branch` deletes only the label.** The commits stay; only an explicit cleanup can reclaim them. | `branch.rs` |
| **Switch rewrites only what differs.** Files whose committed content hash matches are never read, reconstructed, or rewritten — less IO means a smaller window in which a crash can damage anything. | `commit.rs` — `materialize_tree` |
| **Staging is explicit, and partial staging is confirmed.** Committing with nothing staged, or with a partial selection, raises a confirm modal before anything is captured. | `ChangesPanel.tsx` |
| **Discard is behind a confirm.** With the plugin's auto-save, discard is the only thing between the artist and losing saved-but-uncommitted work (the reopen takes the undo history with it). | `krita-plugin/`, `ChangesPanel.tsx` |
| **One-click, verified backup.** `export_repository_zip` zips the whole project (working tree + `.kvc/`) from the activity bar, with a toast reporting where it landed. The finished archive carries a `MANIFEST.json` (repo name, branch, tip commit, timestamp, app version) and is reopened and checked — entry count, manifest readability — before success is reported, so a truncated or otherwise bad backup is never reported as good. Settings → Storage shows a "last backed up N days ago" hint so a stale backup doesn't go unnoticed. | `repo.rs` — `Repo::export_zip`, `verify_zip`; `commands.rs`, `ActivityBar.tsx`, `SettingsModal.tsx` |

## 6. Stash ordering invariants

Three orderings are load-bearing, and **each has a test**:

1. A stash must **not** write `repo.index` — otherwise the revert scans clean and silently keeps the tree dirty.
2. `create` saves the stash **before** reverting the files — a crash between the two must not erase work with no record of it.
3. `pop` writes files **before** dropping the record — a crash leaves the stash on the shelf with the work already restored (recoverable as a conflict), never the reverse.

On top of that, `pop` is **compute-everything-then-write**: every file's final bytes (including any
`.kra` layer merge) are produced before the first byte hits the disk, and non-mergeable conflicts
refuse the whole pop up front. A failed merge leaves the tree *and* the shelf untouched, and the
merge refuses outright (`MergeFailed`) rather than write a `.kra` Krita can't open.

`stash.rs`, `merge.rs`

## 7. Input validation at the trust boundaries

| Measure | Where |
| --- | --- |
| **Path-traversal defense.** Every on-disk path is built through `safe_join`, which accepts only `Component::Normal` segments — rejecting `..`, empty, absolute, Windows drive-relative (`C:\…`) and UNC (`\\server\share`) paths. Covered by three unit tests. | `repo.rs` — `safe_join` |
| **Decompression-bomb cap.** `.kra` archive entries are read through `read_capped`, which fails with `CorruptZip` past `MAX_ARCHIVE_ENTRY_BYTES` instead of allocating whatever the zip header claims. Unit tested. | `repo.rs` — `read_capped` |
| **Dimension caps.** `MAX_CANVAS_DIM` (32 768 px) on the parsed `maindoc`, `MAX_TILE_DIM` (1024) on tile blocks, `MAX_RASTER_DIM` (2048) on anything rasterized — a malformed header can't turn into a multi-GB allocation. | `kra.rs`, `raster.rs` |
| **Tracking guardrail.** The scanner newly tracks only *supported* types (`.kra` + the palette formats). Krita's autosave (`*.kra-autosave.kra`) is rejected explicitly and its backup (`*.kra~`) is skipped in the walk — so transient artifacts never enter history as documents. The rejection lives in `is_supported` (gating *new* tracking only), so a pre-guardrail repo that already committed one isn't pruned. | `scan.rs` — `is_supported`, `scan_detailed` |
| **Malformed input degrades, not crashes.** An unparseable palette falls back to a plain text diff entry; a corrupt `.kra` raises `CorruptZip`/`BadTiles` rather than panicking. | `palette.rs`, `kra.rs` |
| **CLI panic containment.** `kvc`'s `main` wraps dispatch in `catch_unwind` with a silenced hook and reports `{"error": …}` JSON — the Krita plugin parses stdout/stderr as JSON, and a bare Rust backtrace would break it. `cpu::install` sits *inside* the `catch_unwind` so a rayon-worker panic is caught too. | `bin/kvc.rs` |
| **The `--paths` subset flag is a JSON array**, not a repeated or comma-joined flag — the parser is a map (a repeat would overwrite) and real paths contain commas. | `bin/kvc.rs` |

## 8. Typed error boundary

Every fallible engine call returns `Result<_, KvcError>` — a closed enum of named failure modes
(`CorruptZip`, `BadTiles`, `MissingObject`, `Corrupt`, `BadIndex`, `BadPath`, `Locked`,
`DirtyTree`, `StashConflict`, `MergeFailed`, …). `Corrupt` is distinct from `MissingObject` on
purpose: the data is there, it just isn't what it claims to be, which is a different problem with
a different fix. `io_at` attaches the offending path to every IO error and
promotes permission failures to a clearer variant. Tauri commands convert to `String` only at
the very edge, and the frontend matches on stable message prefixes.

`error.rs`

## 9. Krita-side integrity (memory ↔ disk)

The engine only ever sees the disk; Krita's canvas lives only in memory. Both directions of the
plugin are integrity measures:

- **memory → disk** (`_save_tracked`): all *modified* `.kra` under the repo root are saved before
  a commit. Only `.kra` — Krita can raise an export dialog on a `.png` and hang the UI thread it's
  saving on. `busy` is set during the save because `doc.save()` spins the event loop, which would
  otherwise let the 1.5 s poll `kvc status` a half-written `.kra`. Commit **re-runs `refresh()`
  between the save and `_selected_paths()`**, or it skips the very work just written.
- **disk → memory** (`_rebuild_docs`, wrapping switch/discard/stash/pop): **refuses while any open
  document is unsaved**, then closes and reopens each document whose file changed on disk. Without
  the reopen, Krita keeps serving the pre-op copy and the next Ctrl+S silently reverts the
  operation; without the refusal, that reopen eats real work the engine's dirty-tree guard can
  never see.

`krita-plugin/`

## 10. Tests that pin the invariants

Rust tests live in `src-tauri/tests/` (`engine.rs`, `kvc_cli.rs`, `bench.rs`) plus unit tests in
the modules. The integrity-relevant ones: the three stash orderings, `safe_join` escapes
(POSIX + Windows), `read_capped`, the GC rewrite gate, the rayon-pool inheritance through
`commands::run`, and the `kvc` CLI contract tests that spawn the real binary and assert the JSON
shape the Krita plugin parses. Plus, for the measures above:

- `working_tree_writes_are_atomic` — a failed write leaves the target byte-intact and no
  `.kvctmp` behind (in-process we can't kill a write mid-flight, so this pins the observable
  contract instead).
- `corrupt_object_is_refused_on_restore_but_not_on_the_diff_path` — a valid zstd frame holding
  the *wrong* content (so only a hash check can catch it) makes the restore refuse with `Corrupt`
  and leave the working file alone, while the diff path still returns. The second half is what
  keeps the check off the hot loop.
- `check_reports_missing_objects_and_dangling_tips` — silent on a healthy repo, then names a
  deleted object, a dangling tip, and a damaged commit-log line.
- `check_reports_findings_without_failing` (`kvc_cli.rs`) — problems come back as a successful
  run, not as `{"error": …}`.
- `branches_json_corruption_recovers_from_backup`, `index_json_corruption_recovers_from_backup`,
  `stashes_json_corruption_recovers_from_backup`,
  `open_fails_cleanly_when_primary_and_backup_both_corrupt` — the `.bak` fallback recovers from a
  corrupt primary, and still fails cleanly (not a panic) when both copies are bad.
- `cleanup_moves_dead_objects_to_trash_instead_of_deleting`,
  `cleanup_moves_dead_packs_to_trash`, `dry_run_cleanup_never_touches_trash`,
  `prune_trash_removes_dirs_older_than_cutoff`, `prune_trash_leaves_dirs_within_cutoff` — sweep
  victims land in `.kvc/trash/`, a dry run never touches it, and aging-out is exercised by moving
  the cutoff rather than faking a directory's mtime.
- `check_scrub_detects_corrupted_loose_object`, `check_scrub_detects_corrupted_pack_entry`,
  `check_scrub_off_by_default_skips_content_hash`,
  `check_scrub_reports_multiple_corruptions_without_aborting` — the same tampered-but-valid-zstd
  trick as `corrupt_object_is_refused_on_restore_but_not_on_the_diff_path`, this time proving
  `scrub` catches what presence-only `check` can't, stays off by default, and doesn't abort on the
  first bad version.
- `export_zip_round_trips_into_a_reopenable_repo` — extended to assert `MANIFEST.json`'s fields;
  `verify_zip_rejects_entry_count_mismatch` / `verify_zip_rejects_missing_manifest` unit-test the
  reopen-and-check step directly against a hand-built bad archive.
- `generation_bumps_on_every_branches_write` — every `branches.json` write path bumps it;
  `read_consistent_retries_when_a_write_lands_mid_read` /
  `read_consistent_gives_up_after_max_attempts_and_returns_last_result` simulate a write landing
  mid-read by mutating real on-disk state from inside the wrapped closure.
- `composite_tiles_dedup_and_pixel_roundtrip` — the pre-existing test that already pinned gap #9's
  guarantee (pixel-exact, not byte-exact, composite round-trips).

## 11. Audit trail, pack self-check, and a disk-space preflight

The last of the former gap list (P2 — the old gap list's #11–#14, since folded into this doc).

| Measure | Where |
| --- | --- |
| **`.kvc/ops.log`.** Undo, discard, cleanup and branch-delete each append one JSON-lines record (branch, tip before/after, a short detail string) — the "my work disappeared" trail those four had none of before. Same append-then-`sync_all` shape as `commits.log`, size-capped at 2 MB with truncate-oldest rotation (checked, not scheduled, so the common append path never pays for it). Support/recovery only — nothing in the app reads it back. | `ops_log.rs` |
| **Pack self-check.** New packs (`KVCP2`) carry an explicit body-length field alongside the existing index; `read_pack_header` rejects a pack whose declared length doesn't match its real file size, the same way it already rejects an unparseable header — so a truncated pack surfaces as the existing `badPack` check finding instead of `MissingObject`/garbage bytes once something tries to read out of it. Old `KVCP1` packs are still read exactly as before; nothing is rewritten. | `delta.rs` — `read_pack_header`, `Packs::write_pack` |
| **Disk-space preflight.** `commit_selected`, `rollback_to_commit`, and the shared `materialize_tree` (switch/create-branch/merge) sum the bytes they're about to write and refuse up front — `InsufficientDiskSpace` — if the volume has less than double that free (covers `write_file_atomic`'s brief old-file/new-file overlap). Windows-only (`GetDiskFreeSpaceExW`, mirroring `cpu.rs`'s platform-gated style); elsewhere, or if the syscall fails, the check is skipped rather than blocking a write it can't evaluate. | `diskspace.rs` |
| **Plugin reopen re-check.** `_rebuild_docs` takes a stat snapshot right after the op runs, and `_reopen` re-checks it against the file *after* `openDocument` succeeds — if something changed the file again during the reopen window (another process, a stray autosave), the docker surfaces a loud status-label error instead of silently handing back a stale document. | `krita-plugin/kritavc/vc_docker.py` — `_rebuild_docs`, `_reopen` |
