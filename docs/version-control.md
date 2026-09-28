# File tracking and version control

The Rust backend (`src-tauri/src/`) is a custom local version control system built for Krita art
files; `git2` was evaluated and dropped (see [history/01](history/01-origins-and-the-git2-prototype.md)).
It takes `.kra` archives apart down to individual 64×64 tiles, so an edit only stores the tiles that
changed. It is local-only: no remotes, no network.

One `.kra` document is one history. The unit the app versions is a single painting, not a folder of
them. [per-document-tracking.md](per-document-tracking.md) explains why, and describes the container
folder that keeps a folder of tracked paintings from growing a hidden folder per painting.

This page covers the engine's core: the store, the scanner, commits, the delta chains, the `.kra`
tile engine, restoring, and branches. Other features have their own pages:
[layer-subset staging](layer-staging.md), [setting work aside](stashes.md),
[backup and restore](backup-and-restore.md), and the [data-integrity](data-integrity.md) measures.
The Tauri commands and the `kvc` CLI are listed in
[backend-architecture.md](backend-architecture.md#tauri-command-reference).

## Store layout

Each tracked document gets its own self-contained store. The stores sit side by side in one hidden
`.kvc/` container beside the artwork and share nothing: not `objects/`, not `chains/`, not a GC.

```text
artfolder/
  painting.kra
  study.kra
  .kvc/                    hidden; README.txt plus one store per tracked document
    painting-a3f9c1/       <- Repo::store, laid out as below
    study-7b02e4/
```

A custom store root (Settings → Storage → "Where version history is kept") puts new stores somewhere
else instead, and `repo::store_dir_for` is the single place that decides. So `Repo` carries two
paths: `root`, the folder holding the document (what working-tree writes are `safe_join`ed onto),
and `store`, the history laid out below.

`init_repository` creates this inside the store ([`repo.rs`](../src-tauri/src/repo.rs)):

```text
<store>/
  doc.json       which document this store tracks: { relpath, displayName, createdAt }. Kept
                 apart from index.json, which only knows files already committed, so a fresh
                 store has an empty index but a defined document
  config.json    engine settings: the delta-chain threshold (default 20; manifests are capped at
                 5 whatever it says), the tile size (64), the raster cache budget (cacheMaxBytes,
                 default 256 MB), the opt-in tilePixelDeltas flag, and the opt-in lowMemoryDiff
                 flag. The last three can be edited in Settings (get_repo_config and
                 set_repo_config)
  kvc.lock       an OS-level advisory lock (File::try_lock: LockFileEx or flock), held only while
                 a write runs. Its contents don't matter and it is never deleted; only the OS
                 lock state does (see Concurrency and locking)
  kvc.lock.info  a best-effort note naming the current holder's operation ("committing",
                 "switching branches"), rewritten on every acquire and never itself locked
  index.json     the committed head of each tracked file; drives the scanner
  worktree.json  a cache: the size, mtime and hash of the saved-but-unversioned document as the
                 last scan read it, so the next scans needn't read it again (see performance.md).
                 Best-effort, never backed up; missing just means the next scan reads the file
  chains/        every stored version of every delta stream (KVCC2-tagged zstd bincode, loaded on
                 first touch): one shard per tiled entry (a layer's tiles, or the composite's
                 blocks), and one for the document's manifest and small entries, each named
                 <blake3(shard name)[..16]>.bin. A store from before per-entry shards keeps
                 everything in the document shard, which is split on the next save
  commits.log    the commit log as JSON lines, oldest first (append order is topological order).
                 A commit appends one line; only undo and GC rewrite it. Each line records the
                 bytes of new objects that version wrote (storedBytes), for the storage report
  branches.json  branch name → tip commit id, the current branch, and a `generation` counter
                 bumped on every write. Written after the log, so a torn append is never a
                 dangling tip
  stashes.json   the set-aside shelf (see stashes.md). Written last, for the same reason as
                 branches.json: a stash record must never outlive the content it points at.
                 Missing means an empty shelf
  index.json.bak, branches.json.bak, stashes.json.bak
                 one previous copy of each of these small files, taken before every write; if
                 the main copy won't decode on open, the store falls back to the .bak
  ops.log        an append-only record of undo, discard, cleanup and branch delete, kept for
                 support and recovery (see data-integrity.md)
  trash/<timestamp>/
                 what "Clean up storage" swept, moved here instead of deleted and pruned after 14
                 days on the next real cleanup
  objects/       content-addressed blobs: <hash>.full (zstd) or <hash>.patch (bsdiff), sharded
                 256 ways (objects/<hash[..2]>/). A commit with 32 or more new objects writes
                 them as one objects/pack/<hash>.pack instead, and reads ask the packs first
  cache/         content-addressed PNG rasters for the diff viewer, served straight from disk
                 (see performance.md); size-budgeted with LRU pruning, and wiped whole when the
                 downscale filter's .filter-version marker changes
```

None of the formats from before per-document stores (the `chains.bin` and `chains.json` monoliths,
untagged chain shards, `commits.json`, `KVCP1` packs, flat loose objects, a v1 `config.json`, a
store with no `branches.json`) is read any more: v2.0.0 shipped with no migration from v1, so no
store it can open was ever written in them.

Nothing on the hot path ever deletes stored data. Undo and branch delete only drop a reference and
leave orphaned commits, chain versions and objects behind, which is harmless because objects are
content-addressed and dedup on any re-commit. Reclaiming them is the user-facing "Clean up storage"
(`cleanup_repository`, a mark-and-sweep in [`gc.rs`](../src-tauri/src/gc.rs)). It reclaims
everything unreachable from any branch tip or any stash, moves swept objects and packs to `trash/`
instead of deleting them, and runs as a dry run first so the confirm dialog shows real numbers. The
full write-up is in [performance.md](performance.md#storage-reclamation-gcrs), and the safety rules
are in [data-integrity.md](data-integrity.md#4-garbage-collection-that-cant-eat-live-data).

State is loaded into a `Repo` (`Repo::open`), changed in memory, then flushed with `Repo::save`.
State writes are atomic: they go to a `*.tmp` sibling that is then renamed over the target. Hashing
is blake3 throughout (`hash_bytes`), and timestamps are ISO-8601 UTC, computed without a date crate
(`now_iso`, `epoch_to_iso`).

### Concurrency and locking

The engine has no internal locking, so every entry point that writes takes an OS-level advisory
lock (`RepoLock` in `repo.rs`, on `<store>/kvc.lock`, held through `std::fs::File::try_lock`:
`LockFileEx` on Windows, `flock` on Unix) before touching the store. The desktop app's writing
commands (commit, branch create, switch, merge and delete, rollback, undo, restore, a real cleanup,
config writes, delete) and the `kvc` CLI the Krita plugin runs share the same lock, so a plugin
commit can't interleave with a desktop commit, switch or GC into a torn write. Because it's an OS
lock rather than "the file exists", the OS releases it the moment the holding process's file handle
closes, whether cleanly, on a panic unwind, or on a crash or force-kill. There is no stale-lock state
to clean up; the next `try_lock()` on an orphaned `kvc.lock` simply succeeds. (An earlier scheme that
created a marker file could get stuck forever if the holder never exited cleanly.)

`RepoLock::acquire` takes a short present-participle label ("committing", "switching branches",
and so on) and writes it into `kvc.lock.info`, a small file next to the lock. It can't go in
`kvc.lock` itself, because Windows enforces a locked byte range against ordinary reads too (unlike
POSIX `flock`, which is purely advisory), so a blocked caller reading `kvc.lock` would fail with
`ERROR_LOCK_VIOLATION`. The info file is never locked, so it's always readable. A second writer gets
`KvcError::Locked` with a `"<repo> — <op> for <age>"` description built by
`lock_holder_description` (the painting's name from the path, the holder's operation read back from
`kvc.lock.info` or "writing" if that's empty, and an age from the file's mtime, shown in seconds,
minutes or hours). That lets a blocked caller tell a slow operation from a stuck one. Every call site
names its own operation (see `commands.rs` and `bin/kvc.rs`). Read-only commands (scan, history,
diffs, a dry-run cleanup) don't lock.

Since reads take no lock, a read could in principle land in the middle of a write and see a partly
updated snapshot. `branches.json` carries a `generation` counter bumped on every write, and the four
read commands where staleness would be visible (`list_commits`, `commit_diff`, `working_diff`,
`list_branches`) re-read just that counter before and after (`read_consistent` in `commands.rs`),
retrying a bounded number of times if it moved. That costs a re-read of `branches.json`, not a second
full read. The `kvc` CLI's poll commands (`status`, `branches`, `stash-list`) deliberately skip this
and stay exactly as cheap as before: the race is narrow and harmless (a stale but consistent
snapshot, never corruption), so it isn't worth taxing a 1.5-second poll for.

### Path safety

Committed file paths live in `commits.log`, plain JSON that travels with a store, and `file`
arguments arrive from the frontend, so both are untrusted input whenever a store the user didn't
create is opened. Every working-tree write, delete and read joins the relative path through
`repo::safe_join`, which rejects absolute paths, drive and UNC prefixes, the root and `..`
components (`KvcError::BadPath`). `Path::join` with an absolute path silently replaces the root, and
`..` walks out of it, so this closes an arbitrary file write and delete hole in materialize,
rollback, restore and the working-diff read.

## File tracking: the scanner

[`scan.rs`](../src-tauri/src/scan.rs) compares the tracked document against `index.json`:

| Status | Meaning |
|--------|---------|
| `U` | untracked: not in the index yet (the document before its first commit) |
| `M` | modified: its blake3 differs from the index head |
| `D` | deleted: in the index but missing on disk |

An unchanged document produces nothing. There is no directory walk. A store tracks exactly one
document, chosen at `init`, so there's nothing to discover, and `scan_detailed` stats that one path.
That's why scanning an art folder that holds fifty 400 MB `.kra` files costs one `stat`.
`scan::is_supported` no longer gates a walk either; it gates `Repo::init`, the only place tracking
can begin, and accepts `.kra` alone. Standalone palette files (`.gpl`, `.kpl`, `.aco`, `.ase`) are
not tracked; a `.kra`'s embedded document palettes are still parsed and diffed from the `.kra`
itself (see [Palette diffs](#palette-diffs)). `is_supported` also rejects Krita's autosave artifact
(`foo.kra-autosave.kra`, dot-prefixed on Linux and macOS). It is a suffix match on the whole path,
not an extension parse, because that scratch file ends in `.kra` and would otherwise be trackable.

A document whose size and mtime still match the index (`TrackedFile.size` and `mtime`, at
nanosecond resolution), and whose mtime is strictly older than the index file's own mtime, is assumed
unchanged and skipped without being read or hashed. That's the big win for large `.kra` files.
Anything else is hashed and compared with the committed blake3, so a size-preserving edit or a
touched mtime is still classified correctly. The mtime comparison is git's "racy clean" rule. A quick
re-save right after a commit can land in the same filesystem mtime tick as the index write, and if
the size didn't change either (`"v1"` to `"v2"`), size and mtime alone can't tell it apart from
"untouched". So a working file whose mtime is at or after the index file's (`<store>/index.json`,
statted once per scan) is treated as racy and re-hashed; a document committed in an earlier tick
keeps the fast path. `commit_selected`'s optional `paths` filter survives, because the CLI still
passes it, but with one tracked document the UI has no file subset to choose.

A layer-subset commit has to get past that fast path without paying for it: it stores content that
isn't what sits on disk, yet the file's size and mtime match the index. The index flags such an
entry `TrackedFile.partial`, and the scanner reports it as `"M"` straight from the `stat`. See
[layer-staging.md](layer-staging.md#keeping-the-painting-dirty-afterwards).

## Committing: `commit_snapshot` and `commit_selected`

[`commit.rs`](../src-tauri/src/commit.rs) scans, optionally filters to `paths` (`commit_selected`;
`commit_snapshot` is `commit_selected(.., None)`), then routes each change:

- A deletion is dropped from the index and recorded as a `D` file entry with no content.
- A change to the document goes to `kra::commit_kra`, which decomposes the archive (see below) and
  returns its manifest hash. With a `layers` argument it first goes through layer-subset staging
  ([layer-staging.md](layer-staging.md)). A store tracks one `.kra` and `Repo::init` refuses
  anything else, so there is no generic path for other files; the one that stored whole blobs under
  `file:<path>` streams is gone.

Each stored file's blake3, size and mtime are written back into the index (the scan hands the bytes
it already read to the commit, so a big `.kra` is read once per commit). A `Commit` is recorded with
`parents` set to the current branch tip (the first parent is the mainline; a merge commit has two),
the branch name stamped on it (cosmetic; the frontend uses it for labels and colors), and each
file's on-disk blake3 as `fileHash`, which lets `undo` rewind the index without reconstructing files
just to hash them. Older records without it fall back to reconstructing. It also records
`storedBytes`, the bytes of the new objects the commit wrote (counted as `commit_prepared_batch` and
`commit_prepared` write them), which is what the storage report sums. The branch tip then moves to
the new commit. A clean tree returns `KvcError::Nothing`. The commit id is the first 12 hex
characters of a blake3 over the timestamp, message, parents and each file's content hash
(`commit::record_id`, which a stash's id shares).

`Repo::save` flushes the state: `index.json` and `branches.json` as compact JSON, the commit as one
appended line of `commits.log` (constant time, never a rewrite that grows with history), and only
the chain shards the commit actually changed (`ChainStore` tracks dirty shards), as KVCC2-tagged
zstd bincode. Tiles are sharded per layer entry, so a commit's chain-write cost scales with the
layers it touched, not with the total history, and `save` skips shards entirely when no new stream
version was stored, so switch, merge and undo never rewrite chains. A batch of 32 or more new objects is written as one pack file instead
of one loose file each, because creating files one by one dominated large commits on Windows (see
[performance.md](performance.md#state-file-writes)).

### The first-parent-delta invariant

`Commit.files` holds only the changed files, and by invariant it is exactly the diff between the
commit's tree and its first parent's tree. Merge commits are built to record the merged result's
full diff against their first parent. So the tree at any commit is a fold along the first-parent
chain only (`tree_at_commit`), from the root to the commit, and second parents exist only for
drawing the graph and for reachability. That keeps tree computation correct and cheap (linear in the
first-parent depth) even though the commit log interleaves branches.

## Delta-chain storage

In [`delta.rs`](../src-tauri/src/delta.rs), a stream is any versioned byte sequence (a `.kra`
manifest, an archive entry or a single tile) with a string key. `store_stream` does one of three
things:

1. **Dedup.** If the content hash already exists in the stream's chain, it returns that hash and
   stores nothing.
2. **Patch.** If the content is at least 64 KB, isn't already compressed (PNG, zip or zstd magic),
   and the chain is shorter than `delta_chain_max` (20), it stores a bsdiff patch against the chain
   head (`<hash>.patch`). Patching only pays off for large, diff-friendly data, which in practice
   means the `.kra` manifests. Those are capped at five patches whatever the config says
   (`delta::MANIFEST_CHAIN_MAX`): every diff, restore and commit loads a manifest by replaying its
   chain, and at 20 that was up to 250 ms a load on a long history. For small streams such as tiles,
   the chain-walk reconstruct plus a bsdiff suffix sort costs more than the couple of KB it saves, and
   patches against compressed payloads come out close to full size.
3. **Snapshot.** Otherwise (a first version, small or compressed content, or the chain limit
   reached) it stores a fresh zstd snapshot (`<hash>.full`) and resets the chain length.

Every patch is verified when it's written: the engine applies it back against the base and compares
the result byte for byte, and falls back to a full snapshot on a mismatch, so every stored version is
guaranteed to rebuild. `reconstruct(key, hash)` walks the patch chain back to its full snapshot and
applies the patches. It doesn't re-hash what it rebuilt on the normal read path; restore paths turn
that on with `Repo::verify_reads` (see
[data-integrity.md](data-integrity.md#3-content-integrity-of-stored-data)). Objects are
content-addressed, so writing one that already exists does nothing, which is dedup across files for
free.

`store_stream` is split into `prepare_stream` (`&self`, read-only: the dedup check, reconstructing
the base, bsdiff and verify or zstd, which is where the CPU goes) and `commit_prepared` (`&mut self`:
write the object, push the version). That lets many independent streams be prepared in parallel and
then folded in one at a time, which is how the `.kra` tile engine below uses it.

## The `.kra` tile engine

A `.kra` is a zip archive. [`kra.rs`](../src-tauri/src/kra.rs) and
[`tiles.rs`](../src-tauri/src/tiles.rs) split it into streams so small edits stay small.

- **Tiled layer data.** Binary entries under `<doc>/layers/` that start with a `VERSION ` header are
  parsed into individual tiles, and each tile becomes its own stream
  (`kra:<path>:tile:<entry>:<x>,<y>`). Unchanged tiles dedup automatically, so an edit in one
  corner only stores those tiles. Tiles are prepared in parallel with rayon, which spreads the diff
  and zstd work over the cores, then committed one at a time (each tile is a distinct key, so there's
  no race). This is the bulk of a commit's cost.
- **Every other entry** becomes one stream (`kra:<path>:entry:<name>`). That includes a tiled layer's
  `<entry>.defaultpixel` file when Krita writes one: the fill color for any pixel the layer's tiles
  don't cover. Krita only stores tiles for the painted parts of a layer, so a uniformly filled layer
  (most often a solid "Background") is mostly untiled. The diff viewer's raster code reads this back
  to fill the canvas before drawing the real tiles on top, instead of defaulting to transparent (see
  [visual-diff-viewer.md](visual-diff-viewer.md)).
- **A JSON manifest** (`kra:<path>:manifest`) records the entry order, each entry's blob hash, the
  per-tile references, and each entry's zip crc32 and uncompressed size, which is enough to put back
  together a logically identical archive (`mimetype` stays first and stored, and tiles are re-emitted
  in their original block format).
- **Unchanged entries are skipped at commit time.** `commit_kra` takes the previous commit's
  manifest for that path (`commit_snapshot` looks it up through the current tip's tree), and for
  each zip entry whose crc32 and size, read from the central directory without decompressing, match
  the previous manifest, it reuses that manifest entry as-is instead of inflating and re-storing it.
  Commit cost follows the entries that actually changed. crc32 plus size has about a 2⁻³² chance of a
  false match per changed entry; the upgrade would be hashing the compressed bytes.
- **The composite is stored in blocks** (`KraEntry::CompositePng`). `mergedimage.png` changes on
  almost every commit, and as an opaque PNG it used to add its whole multi-megabyte self each time,
  which was the store's largest cost. Composites that qualify (8-bit RGB or RGBA with no ICC profile;
  an sRGB chunk is recorded and written back) are decoded once and stored as 256 px raw-pixel blocks
  in the tile keyspace, so unchanged regions dedup across commits and changed blocks bsdiff at the
  same position. A restore reassembles the blocks and re-encodes a valid PNG: the pixels are exact,
  the bytes aren't Krita's original encoding. Composites that don't qualify stay byte-exact `Raw`,
  `preview.png` stays `Raw` on purpose (it's tens of KB), and old manifests still rebuild unchanged.
- **Rebuilding is parallel and bounded in memory.** `kra::write_kra_from` resolves entries' bytes
  (replaying each tile's patch chain) with rayon's `par_iter` in 64 MB chunks, writing each chunk to
  the zip in manifest order before building the next. The restore paths point it at the temp file
  beside the artwork (`kra::write_kra`), so peak memory is one chunk, not the whole decompressed
  document; `reconstruct_kra`, for the callers that want the bytes in memory, writes into a `Vec`.
  Rebuilt tile blocks and other uncompressed entries are written
  with fast deflate (Krita deflates them too; storing them uncompressed left restored files several
  times larger), and entries that already look compressed (`delta::looks_compressed`: PNG, zip or
  zstd magic) are stored as they are, since compressing them again buys nothing.
- **Committing is bounded in memory too.** `commit_kra` reads and prepares zip entries in the same
  64 MB chunks (`RESTORE_CHUNK_BUDGET`, uncompressed): a chunk collects inflated entries, runs the
  parallel prepare and the serial fold (`prepare_entry_work`, `flush_entry_chunk`), then drops its
  buffers before the next chunk is read (entries reused as-is carry no buffer). Peak memory is about
  one chunk, where a first commit or a big edit used to hold every decompressed entry at once.
- **Untrusted input is capped.** Sizes and counts read from a `.kra` drive allocations, so the
  parsers cap them: `parse_image_meta` rejects a canvas larger than `MAX_CANVAS_DIM` (32,768 px, far
  beyond any real Krita document) before it can size a `width*height*4` raster, and the tile parser
  clamps its `DATA <n>` preallocation to the block's byte length, so a crafted count can't force a
  huge up-front allocation.

Tiles are diffed as opaque LZF-compressed blobs by default. The opt-in `tilePixelDeltas` flag
("Compact storage for heavily-revised art" in Settings → Storage) stores the decoded planar pixels
instead. Those bsdiff well across versions (2 to 10 times smaller for heavily reworked layers), a
restore re-encodes them as LZF (`raster::lzf_compress`), and a `raw` flag on each tile reference in
the manifest means mixed histories work and turning the flag off never breaks existing commits. It's
off by default because the LZF decode and encode cost lands on the commit and restore paths of
low-end machines.

A second opt-in flag, `lowMemoryDiff` ("Low-memory diffs" in Settings → Performance), only affects
the working-tree diff view, never stored data. Normally `parse_working` decodes a working `.kra`
fully into memory (`WorkingKra`), so layer rasters decode straight from RAM. With the flag on, it
keeps only the compressed archive plus per-entry metadata (headers, tile coordinates and content
hashes) and re-inflates each entry when its raster is requested. Peak memory becomes the compressed
document plus one decoded entry, instead of the whole decompressed document, for a little extra CPU.
Change detection is the same either way, because the hashes are always kept.

`maindoc.xml` is parsed too (`parse_maindoc`, with `roxmltree` and DTDs allowed), so layer metadata
changes between two commits can be reported: added, removed, opacity, blend mode and renames,
matched by uuid and then by name (`diff_maindoc`). `parse_image_meta` also reads the image's DPI
(`x-res`), color model (`colorspacename`) and ICC profile, and each layer's visibility (`visible`)
and node type (`kind`, for example `paintlayer` or `grouplayer`). A layer's painted-area bounding box
comes from `kra::layer_bounds`, the union of its tile coordinates, with no pixel decoding. All of
this goes out on `ArtDiffDto` and `LayerDto` and appears in the Inspector's Selected section.

## Restoring, rollback and undo

`commit::file_at_commit` rebuilds the document's bytes as of any commit from its manifest
(`reconstruct_kra`). The `restore_file` command and the other restores write straight into the temp
file beside the artwork instead (`commit::write_committed`, `kra::write_kra`), then rename it into
place, so the rebuilt document is never held whole in memory. Two higher-level operations build on
this in [`commit.rs`](../src-tauri/src/commit.rs).

- **Rollback** (`rollback_to_commit`, "Restore this version" in the UI). For a historical commit
  (not the tip), it computes that commit's tree through `tree_at_commit`, writes it into the working
  tree (skipping files whose committed content already matches, writing the rest, and deleting files
  that didn't exist then), and records the result as a new commit on the current branch. That commit
  is built directly from the tree diff, since every restored file's hash is already known, so no full
  rescan is needed. It's non-destructive and can itself be undone. The new commit's `restored_from`
  is set to the source commit, which is how the legacy history graph draws a link back to it. If the
  commit is the current tip, there's nothing new to record, so it calls `discard_to_tip` instead,
  which scans the actual working tree (`scan::scan_detailed`, not `current_tree`, which comes from
  history and would trivially match the tip) and rewrites or removes exactly the dirty files back to
  the tip's content in place, with no new commit. Either way it returns `Nothing` when there's
  nothing to do.
- **Discard working changes** (`discard_working_changes`, the `discard_changes` command). The
  general form of that in-place rewrite. An optional `paths` filter limits it to those relative
  paths; `None` discards every dirty file. The Changes panel's "Undo all" and the panel menu's
  "Discard current changes" both discard everything. `discard_to_tip` is a thin wrapper that calls
  it with `None`. It returns `Nothing` if nothing in scope is dirty.
- **Undo the last commit** (`undo_last_commit`, "Undo the last version"). A soft reset of the
  current branch tip, which can sit in the middle of the commit list after a switch. It removes that
  commit by id, moves the branch tip back to its first parent, and rewinds only the index entries for
  the paths it touched (from the new tip's tree). It's refused (`CannotUndo`) when a later commit
  builds on the tip or another branch points at it. The working tree is left alone, so the undone
  edits come back as uncommitted changes. Orphaned objects and chain versions stay in place until a
  cleanup.

## Branches: create, switch, merge

[`branch.rs`](../src-tauri/src/branch.rs). Branches are named tips over the shared commit graph
(`branches.json`). Delta streams are keyed by file path, not by branch, so identical content dedups
across branches for free.

- **Create** (`create_branch`). Validates the name (1 to 60 characters, none of the punctuation
  Windows rejects). With no `base`, or with `base` equal to the current branch, it records the new
  name at the current tip and switches to it. The tree is identical, so this costs nothing beyond
  writing `branches.json` (`save_branches`, which never touches the chains). With a different
  `base`, it refuses on a dirty tree, writes that branch's tree into the working tree first (the same
  "rewrite only what differs" path as `switch_branch`), then records the new branch at `base`'s tip.
  That needs the full repo (`Repo::open`, not `open_light`), because it walks `tree_at_commit`.
- **Create at a commit** (`create_branch_at`). The same thing from any commit rather than another
  branch's tip: "go back to version 5 and try a different direction". Same rules (clean tree,
  materialize, `Repo::open`), reusing `tree_at_commit` and `materialize_tree`, and the commits between
  it and the old tip stay reachable from the branch they were made on. It's reached through
  `create_branch`'s `commit:` argument, which can't be combined with `base:`, and from the Version
  Map's pick-a-version mode. It is deliberately not in the `kvc` CLI, because the Krita plugin has no
  version picker to call it from.
- **Switch** (`switch_branch`). Refused on a dirty tree (`DirtyTree`; a clean scan also proves no
  untracked file can be clobbered). It computes both branch trees and calls `materialize_tree`, which
  rewrites only files whose committed content hash differs. Unchanged files are never read,
  reconstructed or rewritten, and their index entries carry over, so the scanner's fast path stays
  warm. A differing `.kra` is rebuilt incrementally (`kra::materialize_kra_into`): entries
  identical in the two manifests are raw-copied out of the working file on disk (each checked against
  the manifest's recorded crc32 and size), a changed tiled entry lifts its unchanged tiles from the
  working copy, and only tiles whose content differs are replayed from the object store. It reads
  the working file through a handle and writes into the temp file beside it, so neither document is
  held whole in memory. Switch cost follows what differs between the branches, not the size of the
  document, and a full rebuild from the store remains the fallback. The index and working tree end up exactly on the target
  branch, and the chains aren't rewritten, because nothing new was stored.
- **Merge** (`merge_branch`, source into current). Fast-forwards when the current tip is an ancestor
  of the source tip (the tip moves, with no new commit). Otherwise it does a per-file three-way merge
  against the merge base (the first common ancestor). A file changed only in the source is taken; a
  file changed only in the current branch is kept (no entry, since the first parent already has it);
  a file changed in both takes the source version and is flagged `"C"`, because art files can't be
  merged by content, and the UI shows the flag as "Needs review". When one side deleted a file and the
  other edited it, the edit is kept rather than the deletion winning, so a conflict never destroys
  data, and it's flagged `"C"` either way. The merge commit has `parents: [current_tip, source_tip]`,
  and its `files` is the merged result's diff against the first parent, which keeps the fold
  invariant. `NothingToMerge` when the source is already part of the current branch.
- **Delete** (`delete_branch`). Removes the label only, and is refused for the current branch and
  for `main` (`DeleteMain`). Its commits stay in `commits.log` as harmless unreachable data until a
  cleanup. The UI never offers delete on `main`.

## Frontend integration

The frontend uses [`inTauri()`](../src/lib/tauri.ts) to detect the desktop shell.

- **In Tauri.** [`useWorkingChanges`](../src/lib/repoData.ts) calls `scan_repository` once, in
  `RepoShell`, and the result is shared by the Changes panel and the Version Map. `ChangesPanel`
  calls `commit_snapshot` and describes the pending work as the layers that changed, read from
  `useWorkingDiff`'s per-layer `change` rather than from any new command.
  [`useCommits`](../src/lib/repoData.ts) calls `list_commits` and maps `BackendCommit` to the
  frontend's `Commit`, and [`useBranches`](../src/lib/repoData.ts) calls `list_branches`.
  [`repository.tsx`](../src/lib/repository.tsx) drives tracking and deletion
  (`is_repository`, `init_repository`, `delete_repository`), backup and restore, the native file and
  save pickers (`tauri-plugin-dialog`, filtered to `.kra`), and every write action (rollback, undo,
  `discardChanges`, stashes, and branch create, switch, merge and delete).
- **In a plain browser** (`npm run dev`, no backend), the hooks return empty results and the actions
  do nothing, and the status bar shows a "Browser preview" badge. The one exception is the opt-in
  `?mock` fixture described in [frontend-architecture.md](frontend-architecture.md).

`list_commits`, `scan_repository` and the working diff are fetched again whenever the repository
context bumps `refreshNonce` (after a commit, rollback or undo, and when the window regains focus).
Visual diffs of `.kra` files load in two stages so the panel appears immediately.
[`useCommitDiff`](../src/lib/repoData.ts) calls `commit_diff` for the composite, the layer metadata
and the tile-derived change regions, which is fast. Then [`useArtLayers`](../src/lib/repoData.ts)
calls `commit_layers` (or `working_layers`) for that file's per-layer rasters, delivered as SVG
`<image>` markup so the SVG-compositing viewer renders them unchanged, and
[`ArtDiffView`](../src/components/vcs/ArtDiffView.tsx) merges them in as they arrive. Both hooks
expose a `loading` flag: `MainPanel` shows "Analyzing changes…" for the first stage, and
`ArtDiffView` shows "Loading layers…" while the rasters stream in. Layer rasters are downscaled to a
longest side of `raster::MAX_RASTER_DIM` (2048 px) before encoding, because a diff preview never
needs full document resolution and full-resolution encoding was the diff's largest cost. See
[visual-diff-viewer.md](visual-diff-viewer.md) and [performance.md](performance.md).

## Palette diffs

Palettes get a real color-by-color swatch diff, computed entirely in the backend
([`palette.rs`](../src-tauri/src/palette.rs)); the frontend's `PaletteDiffView` renders the result
(see [visual-diff-viewer.md](visual-diff-viewer.md)).

Only palettes embedded in a `.kra` reach the diff today, since standalone palette files aren't
tracked. Krita stores a document's palettes inside the archive, often as several files per palette:
the native `.kpl`, `.gpl` exports, and copies with a version segment (`sun-set.0006.kpl`).
`commands::kra_palette_dtos` finds them through `KraSource::palette_entry_names` (entries under
`palettes/` or with a palette extension) and collapses each palette to one representative
(`palette_logical_key` strips the extension and any `.NNNN` segment; `logical_palette_reps` prefers
`.kpl`, then the highest version). A palette whose entry hash didn't change is skipped. Each side is
parsed with its own entry name, so the right parser runs even when the representative changes format
between versions. The result is a `Palette` `DiffEntryDto` keyed `<kra>::<palette-file>`, so one
`.kra` diff yields its `Art` entry plus zero or more `Palette` entries.

Each format parses to a flat list of named sRGB swatches:

- **`.gpl`** (GIMP): text, `R G B  Name` lines, with a `Columns:` header for the grid width.
- **`.kpl`** (Krita): a zip whose `colorset.xml` is parsed with `roxmltree`, reusing the `.kra` code's
  zip and XML dependencies. `RGB` and `sRGB` entries are exact; `Gray` and `CMYK` are converted.
- **`.aco`** (Adobe Color): big-endian binary. The v1 section gives the colors (RGB exact, grayscale
  and CMYK converted); the optional v2 section adds UTF-16 names, best effort, so a misparse keeps
  the v1 colors with hex names.
- **`.ase`** (Adobe Swatch Exchange): big-endian binary. Color blocks carry a UTF-16 name and a
  four-character color model (`RGB `, `CMYK`, `Gray` or `LAB `, converted to sRGB); group blocks are
  skipped.

`palette::diff` then matches swatches by name (the first unused match, since names can repeat), so a
recolor reads as `modified` (before and after) rather than a removal plus an addition. A name only in
the new side is `added`, and a name only in the old side is `removed`. `commands::palette_dto` and
`palette_dto_from` run the diff and serialize it as the `Palette` variant (`kind: "palette"`). A
palette that won't parse degrades to a plain text entry, so one bad file can't blank the panel. The
cost is negligible (palettes are kilobytes and parsing is linear in the swatches), so unlike `.kra`
rasters there's no streaming and no caching: the diff is computed inline, on the blocking pool.
