# Data integrity: what the engine does

An artist's `.kra` is often the only copy of weeks of work, and this VCS is local-only: there's no
remote to fetch from if the store goes bad. So the engine's stance is "never be the reason a file is
lost", built from a handful of cheap, boring rules rather than one big transaction system.

This page lists the measures that are in the code today, and where each one lives.

## 1. Concurrency: one writer per store

| Measure | Where |
| --- | --- |
| **An unreachable store is never mistaken for an untracked document.** `Repo::locate` reports three states, not two: it opens, `NotARepo` ("never versioned"), and `StoreUnreachable` ("the history is in a folder that isn't available right now", such as a custom store root on a drive that isn't plugged in). The UI answers `NotARepo` by offering to start tracking, which for the third case would create an empty store and orphan every version the artist saved. `locate_failure` is a pure function so that this rule can be tested without changing the process-global store root. | `repo.rs`: `Repo::locate`, `locate_failure`; `tests/engine.rs`: `unreachable_store_is_not_reported_as_untracked` |
| **Stopping tracking never touches the artwork.** `Repo::delete` removes the store, preferring the Recycle Bin. Under the folder model the same action deleted the project folder, art included. The artwork now looks perfectly fine afterwards while its history is gone for good, so the confirm dialog asks the artist to type the artwork's name instead of relying on the loss being noticed. | `repo.rs`: `Repo::delete`; `TopBar.tsx`: `RemoveRepoModal` |
| **Stores share nothing, so a sweep can't cross artworks.** Each document's store owns its `objects/`, `chains/` and `cache/`. Two paintings in one folder share only the hidden container, so GC over one can't reach the other's blobs. That's the hazard a shared object store would have introduced, and the reason the per-document design rejected sharing. | `repo.rs`: `store_dir_for`; `tests/engine.rs`: `sibling_documents_have_independent_stores` |
| **An OS-level exclusive lock** on each store (`<store>/kvc.lock`, through `File::try_lock`, which is `LockFileEx` or `flock`). Every entry point that writes takes it, in the Tauri commands and in the `kvc` CLI, so a Krita-plugin commit can't interleave with a desktop commit, switch or GC into a torn write. | `repo.rs`: `RepoLock::acquire` |
| **No stale-lock state.** The OS releases the lock when the holding process's handle closes, whether it exits cleanly, unwinds from a panic or is killed. There's deliberately no `impl Drop` and no marker file to clean up. | `repo.rs`: `RepoLock` |
| **The lock says who holds it.** A `kvc.lock.info` file records a present-participle label ("committing", "switching branches"), and its mtime gives the age, so a blocked caller's `Locked` error says what is holding the store and for how long. It's a separate file that's never locked, because Windows enforces a locked byte range against ordinary reads. | `repo.rs`: `write_lock_info`, `lock_holder_description` |
| **Reads take no lock,** by design (`status`, `branches`, `stash-list`), so the Krita plugin's 1.5-second poll never waits on a write or blocks one. | `bin/kvc.rs` |
| **Reads the artist looks at check for a stale snapshot.** `branches.json` carries a `generation` counter bumped on every write (`Repo::save`, `Repo::save_branches`). `list_commits`, `commit_diff`, `working_diff` and `list_branches` re-read just that counter before and after, and retry a bounded number of times if a write landed in between. That's a re-read of `branches.json`, not a second full read. It's deliberately not applied to the CLI's poll commands, which must stay as cheap as they were: the race is narrow and harmless (a stale but consistent snapshot, never corruption, because `write_atomic`'s rename is already the atomic boundary), so it's worth closing on the paths the artist looks at but not worth taxing the poll for. | `repo.rs`: `Branches::generation`; `commands.rs`: `read_consistent` |
| **At most two heavy operations at once** (`cpu::heavy_permit`, a tokio semaphore that `commands::run_heavy` takes). Cancelling a diff in the UI doesn't cancel the backend, and clicking quickly through history used to stack up unbounded 64 MB decode buffers. | `cpu.rs`, `commands.rs` |
| **A full-screen busy overlay** during every write (commit, switch, merge, branch create and delete, rollback, undo, cleanup), so a stray click can't race a file rewrite. | `src/components/shell/BusyOverlay.tsx` |

## 2. Crash safety of the store

| Measure | Where |
| --- | --- |
| **Atomic state writes.** Every state file (`index.json`, `branches.json`, `stashes.json`, `config.json`, the chain shards, and the commit log when it's rewritten) is written to a sibling `*.tmp` and renamed over the target, which replaces it atomically on Windows and POSIX alike. | `repo.rs`: `write_atomic`, `write_json`, `write_chains_file` |
| **Atomic working-tree writes.** Every restored file (branch switch, rollback, discard, bringing set-aside work back, single-file restore) goes through the same temp-then-rename, so a crash, a power cut or a full disk in the middle of a write can never hand Krita a truncated `.kra`. The temp suffix is appended (`foo.kra.kvctmp`), not substituted, because `with_extension` would collapse `a.kra` and `a.gpl` onto one temp path, and `.kvctmp` can never pass `scan::is_supported`, so a leftover is invisible to the scanner. | `repo.rs`: `write_file_atomic`; `commit.rs`, `stash.rs`, `commands.rs` |
| **Atomic loose-object writes.** `write_loose` dedups by existence, so a torn object would be trusted forever: every later commit storing that content would skip the write. Temp-then-rename makes an interrupted write invisible instead, and GC sweeps the leftover, since it deletes anything in `objects/` it can't name. | `delta.rs`: `write_loose` |
| **Durable, not just atomic.** The temp file is `fsync`ed before the rename (plus the parent directory on POSIX; on Windows the rename is journaled). Without that, the rename can land while the contents are still in the page cache: atomic against a process crash, but a power cut can still leave a zero-length `branches.json`. The `commits.log` append is fsynced too, because `save()`'s "tips go last" rule is only an ordering if the line is on disk before `branches.json` names it. | `repo.rs`: `sync_write`, `sync_parent_dir`, `flush_commits` |
| **Deliberately not fsynced: loose-object and pack payloads.** That's the commit hot path, thousands of tiny objects, and the save ordering already makes a lost object harmless: it's an unreachable orphan, and the atomic write means it can never be trusted half-written. | `delta.rs` |
| **Pack files are written temp-then-rename too,** and named by the blake3 of their index, so a pack's name vouches for its contents. | `delta.rs`: the pack writer |
| **The write order matters.** `save()` writes the index, then the chains, then the commit log, then `branches.json`, then `stashes.json`. Tips go last, so a torn commit-log append is always an unreachable orphan record, never a dangling branch tip. `stashes.json` follows for the same reason: a stash record must never outlive the chain content it points at. | `repo.rs`: `Repo::save` |
| **An append-only commit log.** A commit appends one JSON line; the log is only rewritten when history is really truncated (undo, GC), flagged through `note_commits_truncated`. There's no rewrite of the whole history on the hot path, so each commit's crash window is one line. | `repo.rs`: `flush_commits` |
| **Torn lines are tolerated on load.** A partial last line in `commits.log` (a crash in the middle of an append) is dropped on read and flags a rewrite, so the fragment is cleaned up instead of appended to. | `repo.rs`: `read_commits` |
| **Narrow flushes for narrow edits.** `save_config`, `save_branches` and `save_stashes` exist so an `open_light` repo (partial state in memory) never rewrites the index or commits from state it didn't fully load. | `repo.rs` |
| **One previous copy of the small state files.** `index.json`, `branches.json` and `stashes.json` each get a sibling `.bak`, the copy from before every write. If the main file won't decode on open, the store falls back to it instead of refusing to open at all, and the main file repairs itself on the next save. `branches.json` is the single point of failure for a whole store (lose it and every commit is unreachable, still on disk with nothing pointing at it), so this is cheap insurance for files a few kilobytes in size. | `repo.rs`: `write_json_with_backup`, `read_json_with_backup` |
| **A sweep for stale `*.tmp` files.** Leftovers from interrupted atomic writes are found and removed by the cleanup pass; nothing else ever removes them. | `gc.rs`: `stale_tmp_files` |
| **Legacy formats are retired only after the new one lands.** The chains monolith is deleted only once every shard is written, and `commits.json` only once `commits.log` is safely in place, so there's never a moment without valid state. | `repo.rs`: `ChainStore::flush`, `flush_commits` |

## 3. Content integrity of stored data

| Measure | Where |
| --- | --- |
| **Content addressing throughout.** Objects, tiles, composite pixel blocks, cache entries and index entries are all keyed by the blake3 of their bytes. Identical content dedups instead of being stored again, and an object's name is a claim about its contents. | `repo.rs`: `hash_bytes`; `delta.rs`; `kra.rs` |
| **Every bsdiff patch is verified when it's written.** Right after computing a patch, the engine applies it back against the base and compares byte for byte; a mismatch falls back to a full zstd snapshot. This is what guarantees every stored version rebuilds, so a corrupt chain can never reach a commit and break it. | `delta.rs`: `prepare_stream_opts` |
| **A chain head that won't rebuild degrades instead of failing.** If the current head can't be rebuilt, the new version is stored as a full snapshot instead of a patch on broken data, so a damaged chain heals going forward. | `delta.rs`: `prepare_stream_opts` |
| **Patches are named by result and base.** A patch is only valid against its base, so two streams that reach identical content from different bases can never collide on one object name. | `delta.rs` |
| **An explicit format tag and config version.** Chain shards start with a `KVCC2` tag (bincode isn't self-describing, so a field change needed a version marker), and untagged shards decode through a legacy struct. `Config.version` exists, and every setting added later is `#[serde(default)]`, so old configs keep deserializing. | `repo.rs`: `CHAINS_MAGIC`, `decode_chains`, `Config` |
| **An unreadable shard is an empty shard, not a crash.** A missing or corrupt chain shard returns `None` and the store still opens, so the rest of the history stays reachable. | `repo.rs`: `read_chains_file` |
| **Verified reads where a bad byte would become the artist's file.** Write-time verification covers engine bugs, but not bit rot, a failing disk, or something outside the app editing the store. So the operations that write rebuilt bytes into the working tree (switch, rollback, discard, bringing set-aside work back, single-file restore) set `Repo::verify_reads`, which re-hashes every object `reconstruct` rebuilds and refuses with `Corrupt` on a mismatch. `reconstruct` recurses along the patch chain, so the whole chain is verified link by link. | `repo.rs`: `verify_reads`; `delta.rs`: `reconstruct` |
| **Off for diffs and previews, deliberately.** That's the hottest loop in the app, and a wrong pixel in a preview isn't data loss. A test pins both halves: the restore refuses, and the diff still returns. | `tests/engine.rs` |
| **Files lifted from disk during a restore are checked against the manifest.** The incremental `.kra` path copies unchanged zip entries and tiles straight out of the working file, but only when the entry's crc32 and uncompressed size match what the manifest recorded at commit time. An old manifest without them (`(0, 0)`) is never trusted, and any mismatch falls back to a full rebuild from the store. | `kra.rs`: `materialize_kra` |
| **Pixel-exact, not byte-exact, on restore, and said so.** The composite (`mergedimage.png`) is re-encoded from content-addressed pixel blocks, and tile entries are rewritten with fast deflate, so a restored `.kra` is deliberately not byte-identical to what Krita saved, and a restore can't be verified by comparing file hashes. What the store guarantees is pixel equality, which is pinned by a regression test rather than left implicit, because anyone relying on byte-exact round trips (external tools, signatures) would otherwise be surprised. | `kra.rs`: `materialize_kra`; `tests/engine.rs`: `composite_tiles_dedup_and_pixel_roundtrip` |

## 4. Garbage collection that can't eat live data

Nothing in the engine deletes stored data on its own. `undo` orphans a commit's objects and
`delete_branch` strands whole histories, both on purpose, because content-addressed orphans are
harmless. Reclaiming them is an explicit, user-triggered "Clean up storage" (see
[performance.md](performance.md#storage-reclamation-gcrs) for how it works).

| Measure | Where |
| --- | --- |
| **Mark and sweep from every branch tip,** not only the current one. | `gc.rs` |
| **Stashes are GC roots.** Nothing in `commits.log` refers to stash content, so without this rule the shelf would be collected out from under the artist. | `gc.rs` |
| **Patch bases are closed over.** A patch is useless without its chain back to a full snapshot, so reachability follows `Version.base`. | `gc.rs` |
| **State files are rewritten before any object is removed.** A crash in the middle of a sweep leaves only orphans the next cleanup collects, never a live reference to missing data. | `gc.rs` |
| **A dry run first.** The confirm dialog in Settings is filled in by a real dry run, so the artist approves the actual numbers. | `gc.rs`, `SettingsModal` |
| **Pack rewrites only above 25% dead** (with a unit test on the threshold), and dead bytes left in a kept pack are excluded from the report, so the report states what the run actually frees. | `gc.rs`: `worth_rewriting` |
| **Cache reclaim is reported separately** (`cacheBytesReclaimed`), because the raster cache regenerates and losing it isn't losing data. | `gc.rs` |
| **Swept objects are quarantined, not deleted.** Dead loose objects and dead or rewritten packs move to `<store>/trash/<timestamp>/` (a rename on the same volume, the same cost as the delete it replaces) instead of being unlinked. If the reachability logic is ever wrong, or a cleanup runs right after a branch delete (an ordinary sequence), the data can still be recovered by hand, which matters with no remote to fetch it from. | `gc.rs`: `quarantine` |
| **The trash empties itself.** Quarantine folders older than 14 days are deleted for good on the next real cleanup (never on a dry run) and reported separately as `trashBytesPruned`: bounded retention, not unbounded growth. | `gc.rs`: `prune_trash` |

## 5. Checking that history is intact

"Is my history intact?" used to be unanswerable, for the artist and for a bug report. A read-only
check now answers it. It shares GC's reachability walk (`gc::mark_live`), and one `tolerant` flag is
the whole difference between the two callers. GC must fail hard on a manifest it can't load, because
sweeping on a partial mark would delete live data, while the check exists to report exactly that and
keep walking. It takes no lock and writes nothing.

| Measure | Where |
| --- | --- |
| **Finds the five ways history becomes unreachable:** a missing object, a broken chain, a branch tip naming a version that isn't in the log, a commit-log line that won't decode, and a pack that won't parse. | `check.rs`: `check_repository` |
| **Damage in the middle of the log is reported, not swallowed.** `read_commits` stops at the first bad line and drops everything after it, which is right for a torn last line and wrong for corruption in the middle, where it would quietly shorten history. The check scans the whole file. | `check.rs` |
| **A corrupt pack is named directly.** Everywhere else, the engine skips a pack it can't parse, which turns real corruption into a confusing `MissingObject` for every object inside it. | `check.rs`; `delta.rs`: `read_pack_header` |
| **Findings are a successful run.** The Tauri command and `kvc check` both report problems in the normal result; `{"error": …}` means the check itself failed, and the Krita plugin couldn't tell the two apart otherwise. | `commands.rs`: `check_repository`; `bin/kvc.rs`: `run_check` |
| **It sits next to "Clean up storage"** in Settings → Storage, as "Check for problems…". It can run over the open artwork, every tracked artwork, or only the ones never checked before (`lib/checkedRepos.ts`, a `localStorage` set stamped after each run). A run over several artworks can be cancelled between artworks; nothing can stop a check midway, so the one in progress always finishes. It's read-only, so it raises no busy overlay. | `SettingsModal.tsx`: `CheckModal` |
| **An opt-in bit-rot scrub.** `check_repository` and `kvc check` take a `scrub` flag (off by default and never run automatically, because it reads the whole store) that also re-hashes every live version's content, through the same `Repo::reconstruct_cached` and `Repo::verify_reads` machinery the restore path uses: a walk over the objects, not new verification logic. One bad version doesn't stop the walk (`corruptContent` problems accumulate). It's "Also read back every version (slower)" in the check dialog, or `kvc check --scrub true`. | `check.rs`: `check_repository(repo, scrub)` |
| **Deliberately not included: any `--repair` mode.** Detection is the part this covers. | none |

## 6. Working-tree safety

| Measure | Where |
| --- | --- |
| **The dirty-tree guard.** Switching and merging refuse while the working tree has uncommitted changes (`DirtyTree`) instead of overwriting them. | `branch.rs`: `ensure_clean` |
| **Stable error prefixes as a contract.** `"unsaved changes: …"` and `"stash conflict: …"` are matched by the frontend (and the plugin) to raise the right recovery dialog (save, set aside, or go to Changes). They're deliberately different strings. | `error.rs`, `BranchDialogs.tsx`, `StashDialogs.tsx` |
| **Rollback is non-destructive.** Restoring an old version records a new commit instead of rewinding history, so it can itself be undone. | `commit.rs`: `rollback_to_commit` |
| **`delete_branch` deletes only the label.** The commits stay; only an explicit cleanup can reclaim them. | `branch.rs` |
| **A switch rewrites only what differs.** Files whose committed content hash matches are never read, rebuilt or rewritten, and less I/O means a smaller window in which a crash can damage anything. | `commit.rs`: `materialize_tree` |
| **Picking layers defaults to all of them, and can't save nothing.** Every changed layer starts ticked, so the default is the whole artwork and the panel tracks only what has been unticked. Ticks reset whenever the artwork, branch or scan underneath them changes. Unticking everything disables the button instead of recording an empty version. | `ChangesPanel.tsx` |
| **A partial commit leaves the artwork dirty, on purpose.** What it stores is a synthesized document, not the bytes on disk, so its index entry is flagged `TrackedFile.partial`, and `scan_detailed` reports `"M"` on a size and mtime match instead of skipping the file as unchanged. Without the flag, the next scan would call the artwork clean, and the layers the artist held back would vanish from the Changes panel with nothing to announce it. The recorded hash is the synthesized document's, so a scan that falls through to a full read still comes out modified. | `commit.rs`: `store_change`; `scan::ScanChange::partial`; `repo.rs`: `TrackedFile::partial`; `tests/staging.rs` |
| **Synthesizing a version refuses rather than write a broken `.kra`.** A color-space change between the two versions, a malformed `maindoc.xml`, or a first version with no committed side to revert to all fail with `StageFailed` and write nothing. Staging stays at the top-level layer grain, so a group is always taken whole, which makes it impossible to emit XML that references a data file that wasn't copied. | `stage.rs`: `stage_kra` |
| **Discard is behind a confirm.** With the plugin saving automatically, discard is the only thing between the artist and losing saved but uncommitted work (and the reopen takes Krita's undo history with it). | `krita-plugin/`, `ChangesPanel.tsx` |
| **Verified backups, any number of artworks, one archive.** The export writes each chosen `.kra` with its store, skips the regenerable `cache/` and transient files, and reopens the archive to check its entry count and manifest before reporting success, so a truncated backup is never reported as good. An archive that ended up empty is deleted rather than left looking like a backup. Settings → Storage shows when the last backup was made. | `repo.rs`: `Repo::export_zip_multi`, `verify_zip`, `skip_in_backup`; `BackupModal.tsx`, `SettingsModal.tsx`. See [backup-and-restore.md](backup-and-restore.md). |
| **Restore works out where history goes on this machine.** `import_zip` treats the archive's `.kvc/<slug>/` path as payload: it extracts the `.kra`, then asks this machine where that document's history belongs through `store_dir_for`, so a custom store root is honored and the artwork doesn't read as untracked. Entry names are joined through `safe_join` (zip-slip) and inflated through `read_entry_capped` (decompression bombs), Replace sends the old store to the Recycle Bin, and every restored store must pass a full `check_repository` before the artwork goes back in the list. | `repo.rs`: `Repo::import_zip`, `import_one`, `Repo::plan_restore`; `check.rs`; `RestoreModal.tsx` |
| **A clash defaults to Skip, and Replace can be checked first.** A destination that already holds a file starts unticked, because Replace overwrites the artwork and deletes its history. When what's there is a tracked artwork, "Compare versions" lists the archive's versions beside the ones on disk, each scoped to its own branch tip, so the artist can tell whether the backup is ahead or behind before choosing. | `repo.rs`: `Repo::backup_versions`, `parse_commit_log`; `commands.rs`: `compare_restore_versions`; `RestoreCompareModal.tsx` |

## 7. Stash ordering rules

Three orderings matter, and each has a test:

1. A stash must not write `repo.index`, or the revert scans clean and silently leaves the tree dirty.
2. `create` saves the stash before reverting the files, so a crash between the two can't erase work
   with no record of it.
3. `pop` writes the files before dropping the record, so a crash leaves the stash on the shelf with
   the work already restored (recoverable as a conflict), never the other way round.

On top of that, `pop` computes everything before it writes anything: every file's final bytes,
including any `.kra` layer merge, are produced before the first byte reaches the disk, and conflicts
that can't be merged refuse the whole pop up front. A failed merge leaves both the tree and the shelf
untouched, and the merge refuses outright (`MergeFailed`) rather than write a `.kra` Krita can't
open. See [stashes.md](stashes.md) (`stash.rs`, `merge.rs`).

## 8. Input validation at the trust boundaries

| Measure | Where |
| --- | --- |
| **Path-traversal defense.** Every on-disk path is built through `safe_join`, which accepts only `Component::Normal` segments and rejects `..`, empty, absolute, Windows drive-relative (`C:\…`) and UNC (`\\server\share`) paths. Covered by three unit tests. | `repo.rs`: `safe_join` |
| **A decompression-bomb cap.** `.kra` archive entries are read through `read_capped`, which fails with `CorruptZip` past `MAX_ARCHIVE_ENTRY_BYTES` instead of allocating whatever the zip header claims. Unit tested. | `repo.rs`: `read_capped` |
| **Size caps.** `MAX_CANVAS_DIM` (32,768 px) on the parsed `maindoc`, `MAX_TILE_DIM` (1024) on tile blocks, and `MAX_RASTER_DIM` (2048) on anything rasterized, so a malformed header can't turn into a multi-gigabyte allocation. | `kra.rs`, `raster.rs` |
| **Only real documents can be tracked.** `is_supported` accepts `.kra` alone, as a suffix check on the whole path, and rejects Krita's autosave artifact (`*.kra-autosave.kra`) explicitly even though it ends in `.kra`. It gates `Repo::init`, the only place tracking starts, and a store only ever scans its one document, so Krita's backup (`*.kra~`) and autosave files never enter a history. | `scan.rs`: `is_supported`, `scan_detailed` |
| **Malformed input degrades instead of crashing.** A palette that won't parse falls back to a plain text diff entry, and a corrupt `.kra` raises `CorruptZip` or `BadTiles` instead of panicking. | `palette.rs`, `kra.rs` |
| **Panics are contained in the CLI.** `kvc`'s `main` wraps the dispatch in `catch_unwind` with a silenced hook and reports `{"error": …}` JSON, because the Krita plugin parses stdout and stderr as JSON and a bare Rust backtrace would break it. `cpu::install` sits inside the `catch_unwind`, so a panic on a rayon worker is caught too. | `bin/kvc.rs` |
| **List flags are JSON arrays.** `--paths` and `--layers` take a JSON array, not a repeated or comma-joined flag, because the parser is a map (a repeat would overwrite) and real paths contain commas. | `bin/kvc.rs` |

## 9. A typed error boundary

Every fallible engine call returns `Result<_, KvcError>`, a closed enum of named failures
(`CorruptZip`, `BadTiles`, `MissingObject`, `Corrupt`, `BadIndex`, `BadPath`, `Locked`,
`DirtyTree`, `StashConflict`, `MergeFailed`, `StageFailed`, `StoreUnreachable`,
`InsufficientDiskSpace` and more). `Corrupt` is separate from `MissingObject` on purpose: the data is
there but isn't what it claims to be, which is a different problem with a different fix. `io_at`
attaches the offending path to every I/O error and turns permission failures into a clearer variant.
Tauri commands convert to `String` only at the very edge, and the frontend matches on stable message
prefixes. See `error.rs`.

## 10. Krita's side: memory and disk

The engine only ever sees the disk, and Krita's canvas lives only in memory. The plugin moves data in
both directions, and both directions are integrity measures:

- **Memory to disk** (`_save_tracked`). The tracked `.kra` is saved, if Krita reports it modified,
  before a commit and whenever focus enters the docker. Only `.kra`, because saving a `.png` can make
  Krita raise an export dialog and hang the UI thread it's saving on. `busy` is set during the save,
  because `doc.save()` spins the event loop, which would otherwise let the 1.5-second poll run
  `kvc status` on a half-written `.kra`. Commit re-runs `refresh()` between the save and
  `_selected_paths()`, or it would skip the very work it just wrote.
- **Disk to memory** (`_rebuild_docs`, around switch, discard, set aside and bring back). It refuses
  while an open document has unsaved changes, then closes and reopens each document whose file
  changed on disk. Without the reopen, Krita keeps serving the old copy and the next Ctrl+S silently
  reverts the operation; without the refusal, the reopen would destroy real work the engine's
  dirty-tree guard can never see.

See [`krita-plugin/README.md`](../krita-plugin/README.md).

## 11. Audit trail, pack self-check and disk-space preflight

| Measure | Where |
| --- | --- |
| **`<store>/ops.log`.** Undo, discard, cleanup and branch delete each append one JSON-lines record (the branch, the tip before and after, a short detail), the "my work disappeared" trail those four operations didn't have. Same append-then-`sync_all` shape as `commits.log`, capped at 2 MB with the oldest entries dropped first (checked on write, not on a schedule, so a normal append never pays for it). For support and recovery only; nothing in the app reads it. | `ops_log.rs` |
| **Packs check their own length.** New packs (`KVCP2`) carry an explicit body-length field next to the index, and `read_pack_header` rejects a pack whose declared length doesn't match its real file size, the same way it rejects a header it can't parse. A truncated pack shows up as a `badPack` finding in the check instead of a `MissingObject` or garbage bytes once something reads from it. Old `KVCP1` packs are read exactly as before, and nothing is rewritten. | `delta.rs`: `read_pack_header`, `Packs::write_pack` |
| **A disk-space preflight.** `commit_selected`, `rollback_to_commit` and the shared `materialize_tree` (switch, create-branch, merge) add up the bytes they're about to write and refuse up front with `InsufficientDiskSpace` if the volume has less than double that free (covering `write_file_atomic`'s brief overlap of the old file and the new one). Windows-only (`GetDiskFreeSpaceExW`, in the same platform-gated style as `cpu.rs`). Elsewhere, or if the call fails, the check is skipped instead of blocking a write it can't evaluate. | `diskspace.rs` |
| **The plugin checks a document after reopening it.** `_rebuild_docs` records the file's mtime and size right after the operation runs, and `_reopen` compares them against the file after `openDocument` succeeds. If something changed the file again during the reopen (another process, a stray autosave), the docker shows a clear error in its status line instead of quietly handing back a stale document. | `krita-plugin/kritavc/vc_docker.py`: `_rebuild_docs`, `_reopen` |

## 12. Tests that pin these rules

The Rust tests live in `src-tauri/tests/` (`engine.rs`, `staging.rs`, `kvc_cli.rs`,
`backup_store_root.rs`, and the ignored-by-default `bench.rs`), plus unit tests in the modules. The
integrity-related ones: the three stash orderings, `safe_join`'s escapes (POSIX and Windows),
`read_capped`, the GC rewrite threshold, the rayon pool's inheritance through `commands::run`, and the
`kvc` CLI contract tests, which spawn the real binary and assert the JSON shapes the Krita plugin
parses. For the measures above:

- `working_tree_writes_are_atomic`: a failed write leaves the target byte-for-byte intact and no
  `.kvctmp` behind. A test can't kill a write halfway through in-process, so this pins the observable
  contract instead.
- `corrupt_object_is_refused_on_restore_but_not_on_the_diff_path`: a valid zstd frame holding the
  wrong content (so only a hash check can catch it) makes the restore refuse with `Corrupt` and leave
  the working file alone, while the diff path still returns. The second half is what keeps the check
  off the hot loop.
- `check_reports_missing_objects_and_dangling_tips`: silent on a healthy store, then names a deleted
  object, a dangling tip and a damaged commit-log line.
- `check_reports_findings_without_failing` (`kvc_cli.rs`): problems come back as a successful run,
  not as `{"error": …}`.
- `branches_json_corruption_recovers_from_backup`, `index_json_corruption_recovers_from_backup`,
  `stashes_json_corruption_recovers_from_backup` and
  `open_fails_cleanly_when_primary_and_backup_both_corrupt`: the `.bak` fallback recovers from a
  corrupt main file, and still fails cleanly (not a panic) when both copies are bad.
- `cleanup_moves_dead_objects_to_trash_instead_of_deleting`, `cleanup_moves_dead_packs_to_trash`,
  `dry_run_cleanup_never_touches_trash`, `prune_trash_removes_dirs_older_than_cutoff` and
  `prune_trash_leaves_dirs_within_cutoff`: swept objects land in `<store>/trash/`, a dry run never
  touches it, and aging out is tested by moving the cutoff rather than faking a folder's mtime.
- `check_scrub_detects_corrupted_loose_object`, `check_scrub_detects_corrupted_pack_entry`,
  `check_scrub_off_by_default_skips_content_hash` and
  `check_scrub_reports_multiple_corruptions_without_aborting`: the same tampered-but-valid-zstd trick,
  proving that `scrub` catches what the presence-only check can't, stays off by default, and doesn't
  stop at the first bad version.
- `export_multi_round_trips_two_documents`, `import_without_a_custom_root_lands_beside_the_artwork`,
  `import_replaces_an_existing_artwork_in_place`, `backup_skips_the_raster_cache` and
  `import_rejects_zip_slip`: the archive round-trips two artworks with their history, the disposable
  cache never ships, Replace restores the backup's history over newer work, and an archive whose entry
  names escape the destination writes nothing outside it.
- `import_follows_this_machines_store_root_in_both_directions` (`backup_store_root.rs`): the
  regression the restore path exists to close. It's a test binary of its own because the custom store
  root is process-global state, so a test that sets it would move every concurrently running test's
  store, and it points the app-data folder at a temporary one so the suite never touches the
  developer's real setting.
- `verify_zip_rejects_entry_count_mismatch` and `verify_zip_rejects_missing_manifest` unit-test the
  reopen-and-check step against hand-built bad archives.
- `generation_bumps_on_every_branches_write` checks that every `branches.json` write bumps the
  counter, and `read_consistent_retries_when_a_write_lands_mid_read` and
  `read_consistent_gives_up_after_max_attempts_and_returns_last_result` simulate a write landing
  mid-read by changing real state on disk from inside the wrapped closure.
- `composite_tiles_dedup_and_pixel_roundtrip`: composites round-trip pixel-exact, not byte-exact.
- The staging tests in `tests/staging.rs` pin the partial flag and the dropped preview renders (see
  [layer-staging.md](layer-staging.md)).
