# File Tracking & Version Control

The Rust backend (`src-tauri/src/`) is a **custom local VCS** purpose-built for Krita art
files — `git2` was evaluated and dropped. It stores history in a `.kvc/` folder inside each
repository, decomposing `.kra` archives down to individual 64×64 tiles so an edit only stores
the tiles that actually changed. Local-only: no remotes, no network.

The frontend calls into this through Tauri commands (`@tauri-apps/api/core` `invoke`); see
[Frontend integration](#frontend-integration) for how the UI consumes it (in a plain browser,
with no backend, the UI renders empty states).

## `.kvc/` store layout

`init_repository` creates this inside the repo root ([`repo.rs`](../src-tauri/src/repo.rs)):

```text
.kvc/
  config.json    engine config: delta-chain threshold (default 20), tile size (64),
                 raster cache byte budget (cacheMaxBytes, default 256 MB; a v1→v2 config
                 migration lowers old 512 MB defaults), the opt-in tilePixelDeltas flag, and
                 the opt-in lowMemoryDiff flag (on-demand working-diff entry decode).
                 cacheMaxBytes + tilePixelDeltas + lowMemoryDiff are user-editable in the
                 Settings modal (get_repo_config/set_repo_config)
  kvc.lock       advisory create-new lock file present only while a mutating operation runs
                 (see Concurrency & locking); removed on completion
  index.json     committed head per tracked file — drives the scanner
  chains/        per-tracked-file shards, each every stored version of that file's delta
                 streams (KVCC2-tagged zstd bincode, <blake3(relpath)[..16]>.bin, faulted in
                 on first touch). Pre-KVCC2 shards (which stored each version's object name
                 redundantly) decode transparently and upgrade when next dirtied; a legacy
                 monolithic chains.bin (or older chains.json) is read transparently and
                 split into shards on the next commit's save
  commits.log    the commit log, JSON-lines (oldest-first, append-order = topological
                 order): a commit appends one line — O(1) instead of rewriting the whole
                 history; undo/GC rewrite it. Legacy commits.json migrates on first save
  branches.json  branch name → tip commit id, plus the current branch (written after the
                 log, so a torn log append is never a dangling tip)
  stashes.json   the set-aside shelf: work parked off to the side of history (see Stashes
                 below). Written last, for the same reason branches.json is — a stash record
                 must never outlive the chain content it points at. Absent = empty shelf,
                 which is the whole migration for repos that predate stashing
  objects/       content-addressed blobs: <hash>.full (zstd) or <hash>.patch (bsdiff),
                 sharded 256-way (objects/<hash[..2]>/, flat legacy paths still read); a
                 commit with ≥32 new objects writes them as one objects/pack/<hash>.pack
                 instead of one loose file each
  cache/         content-addressed capped PNG rasters for the diff viewer, served from disk
                 (see Raster delivery below); size-budgeted with LRU pruning, pruned by
                 "Clean up storage" too (a .filter-version marker triggers a full wipe when
                 the downscale filter changes)
```

A repo from before branching existed (no `branches.json`) migrates on open: everything is
treated as `main` with its tip at the newest commit, persisted by the next save.

Nothing on the hot path ever deletes stored data — undo and branch-delete just drop a
reference, leaving orphaned commits/chain versions/objects behind (harmless, since objects are
content-addressed and dedup on any re-commit). The user-facing **"Clean up storage"** action
(`cleanup_repository`, mark-and-sweep in [`gc.rs`](../src-tauri/src/gc.rs)) reclaims everything
unreachable from any branch tip **or stash** (a stash is referenced by nothing in the commit log,
so it has to be rooted explicitly or a cleanup would destroy set-aside work): unreachable commits
leave the log, dead chain versions leave
their shards, dead loose objects are deleted, and packs are dropped (fully dead) or rewritten
with survivors only when >25% dead (below that, rewriting costs more IO than it frees). A live
patch's whole chain back to its full snapshot counts as reachable. GC also prunes the raster
cache to its budget (reported separately as `cacheBytesReclaimed` — regenerable previews),
sweeps stale `*.tmp` crash leftovers, and consolidates ≥8 sub-4 MB live packs into one.
State files are rewritten before any object is deleted, so a crash mid-sweep only leaves
re-collectable orphans. A `dry_run` mode reports what a real pass would free without touching
anything — the frontend runs it on modal open, then confirms before the real pass. See
[performance.md](performance.md#storage-reclamation-gcrs) for the full mark-and-sweep writeup.

State is loaded into a `Repo` struct (`Repo::open`), mutated in memory, then flushed with
`Repo::save`. State writes are **atomic** (write to a `*.tmp` sibling, then `rename` over the
target).
Hashing is **blake3** throughout (`hash_bytes`); timestamps are ISO-8601 UTC computed without a
date crate (`now_iso` / `epoch_to_iso`).

### Concurrency & locking

The engine has no internal locking, so every **mutating** entry point takes an advisory,
create-new file lock (`RepoLock`, `repo.rs` → `.kvc/kvc.lock`, released on drop) before touching
the store: the desktop app's mutating Tauri commands (commit, branch create/switch/merge/delete,
rollback, undo, restore, real cleanup, config write, delete) **and** the `kvc` CLI the Krita
plugin shells out to share the same lock, so a plugin commit can't interleave with a desktop
commit/switch/GC into a torn write. A second writer gets `KvcError::Locked` ("repository is
busy") rather than corrupting state. Read-only commands (scan, history, diffs, dry-run cleanup)
don't lock. A crash leaves a stale lock, which GC's `*.tmp`-style cleanup can sweep. It's an
advisory create-new lock; upgrade to an OS flock only if a stale lock ever bites.

### Path safety

Committed file paths live in `commits.log` (plain JSON that travels with a shared `.kvc/` store)
and `file` arguments arrive from the frontend, so both are **untrusted input** whenever a repo the
user didn't create is opened. Every working-tree write/delete/read joins the relative path through
`repo::safe_join`, which rejects absolute paths, drive/UNC prefixes, root, and `..` components
(`KvcError::BadPath`) — `Path::join` with an absolute path silently replaces the root and `..`
walks out of it, so this closes an arbitrary-file-write/delete hole on materialize, rollback,
restore, and the working-diff read.

## File tracking — the scanner

[`scan.rs`](../src-tauri/src/scan.rs) walks the working tree (`walkdir`) and classifies each
file against `index.json`:

| Status | Meaning |
|--------|---------|
| `U` | untracked — not in the index |
| `M` | modified — blake3 differs from the index head |
| `D` | deleted — in the index but absent on disk |

Unchanged files produce nothing. The `.kvc/` directory and Krita's backup file (`*.kra~`) are
skipped by the walk. A **tracking guardrail** (`scan::is_supported`) further limits what is *newly*
tracked to the file types Krita VCS actually understands — `.kra` documents and the palette
formats (`.gpl`/`.kpl`/`.aco`/`.ase`); any other file in the project folder is ignored outright
(never staged, hashed, or committed). It also rejects Krita's **autosave** artifact
(`foo.kra-autosave.kra`, dot-prefixed on Linux/macOS): `is_supported` is a suffix match, not an
extension parse, so that scratch file ends in `.kra` and would otherwise be tracked as a document
and get its own chain shard. The guard runs only for files **not already in the index**
(a cheap short-circuit on the steady-state scan), so already-tracked files stay tracked and a repo
that predates either rule is never silently pruned — and an unsupported file is now rejected by
suffix instead of being read and blake3-hashed like before. A file whose **size + mtime still
match the index** (`TrackedFile.size`/`mtime`,
nanosecond resolution) **and whose mtime is strictly older than the index file's own on-disk
mtime** is assumed unchanged and skipped without being read or hashed — the win for big `.kra`
files. Everything else is hashed and compared against the committed blake3, so a size-preserving
edit or an mtime touch is still classified correctly. The mtime comparison is git's **"racy
clean"** rule: a quick re-save right after a commit can land in the *same* filesystem mtime tick as
the index write and, if the byte size is unchanged too (`"v1"` → `"v2"`), size+mtime alone can't
tell it apart from "untouched". So any working file whose mtime is `>=` the index file's mtime
(`.kvc/index.json`, statted once per scan) is treated as racy and re-hashed rather than trusted;
files committed in an earlier tick keep the fast path. Staging is a **selection over the same
scan**, not a separate index: `commit_selected`'s optional `paths` filters the scanned changes
down to a subset before committing, leaving everything else dirty.

## Committing — `commit_snapshot` / `commit_selected`

[`commit.rs`](../src-tauri/src/commit.rs) scans, optionally filters to `paths` (`commit_selected`;
`commit_snapshot` is just `commit_selected(.., None)`), then routes each change:

- **deletion** → drop from the index, record a `D` file entry (no content).
- **`.kra`** → `kra::commit_kra` decomposes the archive (see below) and returns its manifest hash.
- **anything else** (a palette file — the guardrail means nothing else reaches a commit) →
  `Repo::store_stream("file:<path>", bytes)` returns the blob's content hash. Palettes are small,
  but they ride the same delta-chain store as everything else, so successive versions bsdiff
  against each other for free (below the 64 KB patch floor they simply snapshot).

Each non-deleted file's blake3 (plus its size + mtime, for the scanner's fast path) is written
back into the index (the scan hands its already-read bytes to the commit too, so a big `.kra`
is read once per commit), and a `Commit` is recorded with `parents` set to the **current branch
tip** (first parent = mainline; a merge commit has two parents), the branch name stamped on it
(cosmetic — the frontend uses it for labels/colors), and each file's on-disk blake3 as
`fileHash` (lets `undo` rewind the index without reconstructing files just to hash them; old
records without it fall back). The branch tip then advances to the new commit. Returns
`KvcError::Nothing` if the tree is clean. The commit id/hash is the first 12 hex chars of a
blake3 over the timestamp + message + parents + per-file content hashes. State is flushed with
`Repo::save`: `index.json`/`branches.json` as **compact** JSON, the commit as **one appended
line** of `commits.log` (O(1) — never a rewrite that grows with history), and only the per-file
chain shards a commit actually dirtied (`ChainStore` per-shard dirty tracking) as KVCC2-tagged
zstd bincode — a commit's chain-write cost scales with the files it touched, not total repo
history. `save` skips shards entirely when no new stream version was committed there, so
switch/merge/undo never rewrite chains at all. A batch of ≥32 distinct new objects is written
as one pack file instead of one loose file each (see
[performance.md](performance.md#state-file-writes) — per-file creates dominated large commits
on Windows).

### The first-parent-delta invariant

`Commit.files` holds only the *changed* files, and is by invariant **exactly the diff of the
commit's tree against its first parent's tree** (merge commits are constructed to record the
merged result's full diff vs their first parent). The effective tree at any commit is therefore
a fold along the **first-parent chain** only (`tree_at_commit`), root → commit; second parents
exist purely for graph drawing and reachability. This keeps tree computation correct and cheap
(O(first-parent depth)) even though the commit log interleaves branches.

## Delta-chain storage

[`delta.rs`](../src-tauri/src/delta.rs) — a **stream** is any versioned byte sequence (a generic
file, a `.kra` manifest, a layer entry, or a single tile), keyed by a string. `store_stream`:

1. **Dedup** — if the content hash already exists in the stream's chain, return it; store nothing.
2. **Patch** — if the content is at least 64 KB, not already compressed (PNG/zip/zstd magic), and
   the chain head is under `delta_chain_max` (20), store a `bsdiff` patch against the head
   (`<hash>.patch`). Patching only pays for large diff-friendly data (the `.kra` manifests):
   small streams (tiles) cost a chain-walk reconstruct + suffix-sort `bsdiff` to save a couple of
   KB, and compressed payloads yield patches near full size.
3. **Snapshot** — otherwise (first version, small/compressed content, or threshold reached) store
   a fresh `zstd` full snapshot (`<hash>.full`), resetting the chain length.

`reconstruct(key, hash)` walks the patch chain back to its full snapshot, applies patches, and
**verifies** the rebuilt bytes hash to the requested hash (integrity guard). Objects are
content-addressed, so writing an object that already exists is a no-op (cross-file dedup).

`store_stream` is split into `prepare_stream` (`&self`, read-only: dedup check, reconstruct base,
`bsdiff` + verify or `zstd` — the CPU cost) and `commit_prepared` (`&mut self`: write the object,
push the version). This lets many independent streams be prepared in parallel and then folded in
serially — used by the `.kra` tile engine below.

## `.kra` tile engine

A `.kra` is a ZIP. [`kra.rs`](../src-tauri/src/kra.rs) + [`tiles.rs`](../src-tauri/src/tiles.rs)
decompose it into streams so small edits stay small:

- **Tiled layer-data entries** (binary blocks under `<doc>/layers/`, detected by a `VERSION `
  header) are parsed into individual tiles; **each tile becomes its own stream**
  (`kra:<path>:tile:<entry>:<x>,<y>`). Unchanged tiles dedup automatically — a one-corner edit
  only stores those tiles. Tiles are `prepare_stream`'d **in parallel** (`rayon`) — the diff/zstd
  work fans across cores — then `commit_prepared` serially (each tile is a distinct key, so no
  race), which is the bulk of a commit's cost.
- **Every other archive entry** becomes one stream (`kra:<path>:entry:<name>`) — including a
  tiled layer's sibling `<entry>.defaultpixel` file, if Krita wrote one: the fill color for any
  pixel the layer's tiles don't cover (Krita only tiles a layer's painted-on regions, so a
  uniformly-filled layer — a solid "Background" layer, most commonly — is otherwise mostly
  untiled). Raster reconstruction for the diff viewer reads this back to seed the canvas before
  blitting real tiles on top, instead of defaulting to transparent (see
  [visual-diff-viewer.md](visual-diff-viewer.md)).
- A **JSON manifest** (`kra:<path>:manifest`) records entry order, per-entry blob hashes, per-tile
  refs, and each entry's zip **crc32 + uncompressed size** — enough to reassemble a logically
  identical archive (`mimetype` stays first and stored; tiles re-emitted uncompressed in their
  original block format).
- **Commit-time entry skip** — `commit_kra` takes the previous commit's manifest for that path
  (`commit_snapshot` looks it up via the current tip's tree) and, for each zip entry whose crc32 +
  size (read from the central directory, no decompression needed) match the previous manifest's,
  reuses that manifest entry verbatim instead of inflating/re-storing it. Commit cost becomes
  proportional to the entries that actually changed. Uses crc32+size as the change detector
  (~2⁻³² false-match chance per changed entry); upgrade path is hashing the compressed bytes.
- **The composite is block-tiled** (`KraEntry::CompositePng`) — `mergedimage.png` changes on
  nearly every commit and, as an opaque PNG, used to add its full multi-MB self per commit
  (the store's dominant cost). Eligible composites (8-bit RGB/RGBA, no ICC profile; an sRGB
  chunk is recorded and re-emitted) are decoded once and stored as **256 px raw-pixel blocks**
  in the tile keyspace: unchanged regions dedup across commits, changed blocks bsdiff at the
  same position. Restore reassembles + re-encodes a valid PNG — **pixels exact, bytes not
  Krita's original encoding**. Ineligible composites stay byte-exact `Raw`; `preview.png`
  stays `Raw` deliberately (tens of KB). Old manifests reconstruct unchanged.
- **Reconstruction is parallel and memory-bounded** — `reconstruct_kra` resolves entries'
  bytes (patch-chain replay per tile) with `rayon`'s `par_iter` in 64 MB chunks, writing each
  chunk serially in manifest order before the next builds (peak RAM = output + one chunk, not
  the whole decompressed document). Rebuilt tile blocks and other non-compressed entries are
  written **deflate-fast** (Krita deflates them too — Stored left restored files several×
  larger); entries that already look compressed (`delta::looks_compressed` — PNG/zip/zstd
  magic) stay stored, since recompressing buys nothing.
- **Commit is memory-bounded too** — `commit_kra` reads zip entries and prepares them in the
  same `RESTORE_CHUNK_BUDGET` (64 MB uncompressed) chunks: a chunk accumulates inflated entries,
  runs the parallel prepare + serial fold (`prepare_entry_work`/`flush_entry_chunk`), then drops
  the buffers before the next chunk reads (verbatim reuses carry no buffer). Peak RAM is ~one
  chunk instead of the whole decompressed document — previously a first commit or a big edit held
  every decompressed entry at once.
- **Untrusted-input guards** — dimensions and counts read from a `.kra` drive allocations, so the
  parsers cap them: `parse_image_meta` rejects a canvas over `MAX_CANVAS_DIM` (32 768 px, far
  above any real Krita document) before it can size a `width*height*4` raster, and the tile parser
  clamps its `DATA <n>` preallocation to the block's byte length so a crafted count can't force a
  giant up-front `Vec`.

Tiles are diffed as opaque LZF-compressed blobs by default. The opt-in **`tilePixelDeltas`**
config flag stores decoded planar pixels instead — they bsdiff across versions (2-10× smaller
for heavily-revised layers), restore re-encodes LZF (`raster::lzf_compress`), and the per-ref
`raw` flag in the manifest means mixed histories work and turning the flag off never breaks
existing commits. Off by default: the LZF decode/encode cost lands on the commit/restore paths
of low-end devices.

A second opt-in flag, **`lowMemoryDiff`** (off by default, Settings modal), only affects the
**working-tree diff view**, never stored data. Normally `parse_working` decodes a working `.kra`
fully into memory (`WorkingKra`) so layer rasters decode straight from RAM. With the flag on, it
keeps only the compressed archive plus per-entry metadata (headers, tile coords + content hashes)
and re-inflates each entry on demand when its raster is requested — peak RAM becomes the
compressed document plus one decoded entry instead of the whole decompressed document, trading a
little CPU for bounded memory on low-end machines. Change detection is identical either way (the
hashes are always retained).

`maindoc.xml` is also parsed (`parse_maindoc`, via `roxmltree` with DTD allowed) so layer
metadata changes — added / removed / opacity / blend / rename, matched by uuid then name — can be
reported between two commits (`diff_maindoc`). `parse_image_meta` additionally reads the image's
**DPI** (`x-res`), **color model** (`colorspacename`) and **ICC profile**, plus each layer's
**visibility** (`visible`) and **nodetype** (`kind`, e.g. `paintlayer`/`grouplayer`); a layer's
tile-granular **painted-area bounding box** comes from `kra::layer_bounds` (union of its tile
coords, no pixel decode). These ride out on `ArtDiffDto`/`LayerDto` and surface in the frontend
Inspector's "Selected" section.

## Restoring, rollback & undo

`commit::file_at_commit` rebuilds the exact bytes of a file as of any commit: for `.kra` it
reconstructs from the manifest (`reconstruct_kra`), otherwise from the blob stream. The
`restore_file` command writes those bytes back into the working tree.

Two higher-level history operations build on this ([`commit.rs`](../src-tauri/src/commit.rs)):

- **Rollback** (`rollback_to_commit`) — if `commit_id` is a **historical** (non-tip) commit:
  computes its tree via `tree_at_commit` (the first-parent fold), materializes it into the
  working tree (skipping files whose committed content already matches the current tree, writing
  the rest, deleting ones that didn't exist at the target), then records it as a **new** commit on
  the current branch — synthesized directly from the tree diff (every restored file's hash is
  already known, so this skips a full `commit_snapshot` rescan) — non-destructive and reversible.
  The new commit's `restored_from` is set to `commit_id`, so the history graph can draw a link back to it
  (`CommitGraph`'s revert-link overlay in the frontend). If `commit_id` **is** the current branch
  tip, there's nothing new to record, so it delegates to `discard_to_tip` instead: this scans the
  **actual on-disk** working tree (`scan::scan_detailed`, not `current_tree` — which is derived
  from committed history and would trivially already match the tip) and rewrites/removes exactly
  the dirty files back to the tip's committed content, in place — no new commit. Either path
  returns `Nothing` if there's nothing to do (tree already matches).
- **Discard working changes** (`discard_working_changes`, the `discard_changes` command) — the
  general form of the same in-place rewrite `discard_to_tip` uses, exposed directly to the
  frontend so it can discard less than everything: an optional `paths` filter limits it to those
  relative paths (e.g. the Changes panel's per-file "discard" action, or the sidebar's "Discard
  current changes" restricted to unstaged files), `None` discards every dirty file. `discard_to_tip`
  is now a thin wrapper calling this with `None`. Errors `Nothing` if nothing in scope is dirty.
- **Undo last commit** (`undo_last_commit`) — a *soft* reset of the **current branch tip** (which
  may sit mid-vec after a switch): removes that commit by id, rewinds the branch tip to its first
  parent, and rewinds only the index entries for the paths it touched (from the new tip's tree).
  Refused (`CannotUndo`) if a later commit builds on the tip or another branch points at it. The
  working tree is left untouched, so the undone edits resurface as uncommitted changes. Orphaned
  objects/chain versions are left in place (they're content-addressed and dedup on any re-commit).

## Branches — create, switch, merge

[`branch.rs`](../src-tauri/src/branch.rs). Branches are named tips over the shared commit DAG
(`branches.json`); delta streams are keyed by file path, not branch, so identical content
deduplicates across branches for free.

- **Create** (`create_branch`) — validate the name (1–60 chars, no Windows-hostile punctuation).
  With no `base` (or `base` == the current branch) it records the new name at the current tip and
  switches to it; the tree is identical, so this is **O(1)** — no file I/O beyond `branches.json`
  (`save_branches`, which never touches the chains file). With a *different* `base` branch, it
  refuses on a dirty tree and materializes that branch's tree first (same rewrite-only-differing-
  files path as `switch_branch`), then records the new branch at `base`'s tip — this needs the
  full repo (`Repo::open`, not `open_light`) since it walks `tree_at_commit`.
- **Switch** (`switch_branch`) — refused on a dirty tree (`DirtyTree`; a clean scan also proves
  no untracked files can be clobbered). Computes both branch trees and calls `materialize_tree`,
  which rewrites **only files whose committed content hash differs** — unchanged files are never
  read, reconstructed, or rewritten (their index entries carry over, keeping the scanner's
  size/mtime fast path warm). A differing `.kra` is rebuilt **incrementally**
  (`kra::materialize_kra`): entries identical between the two manifests are raw-copied out of the
  on-disk working file (verified per entry against the manifest's recorded crc32+size), a changed
  tiled entry lifts its unchanged tiles from the working copy in memory, and only tiles whose
  content actually differs replay from the object store — so switch cost tracks what changed
  between the branches, not document size (full `reconstruct_kra` remains the fallback). Index
  and working tree land exactly on the target branch, and the chains file is not rewritten
  (nothing new was stored).
- **Merge** (`merge_branch`, source → current) — **fast-forwards** when the current tip is an
  ancestor of the source tip (tip moves, no new commit). Otherwise a per-file **three-way**
  against the merge base (first common ancestor): changed only in source → taken; changed only
  in current → kept (no entry — the first parent already has it); changed in **both** → the
  source version wins and the entry is flagged `"C"` (art files can't be content-merged; the UI
  surfaces the flag as "Needs review"). When one side **deleted** a file and the other **edited**
  it, the edit is kept rather than the deletion winning — a conflict never destroys data — still
  flagged `"C"` either way. The merge commit has `parents: [current_tip, source_tip]`
  and its `files` is the merged result's diff vs the first parent — preserving the fold
  invariant. `NothingToMerge` when the source is already part of the current branch.
- **Delete** (`delete_branch`) — removes the label only (refused for the current branch and for
  `main`, `DeleteMain`); its commits stay in `commits.log` as harmless unreachable data. The
  Branches panel also hides the delete affordance on `main`.

## Stashes — setting work aside

[`stash.rs`](../src-tauri/src/stash.rs) parks the working tree's changes off to the side of
history, reverts the files on disk, and brings them back later. In the UI this is **"Set aside"**
(Artist Mode) / **"Stash"**, from the Changes panel's ⋮ menu, the save-first prompt, and Settings.

A stash is **not a commit** and never enters `commits.log`. That's deliberate: as a commit it
would show up as a spurious version row in the storage report (`compute_storage_stats` walks
every commit unfiltered) and, being parented on the tip, would silently block "Undo the last
version". Instead a `Stash` record — id, label, author, timestamp, origin branch, and
`files: Vec<CommittedFile>` — lives in `.kvc/stashes.json`.

Storage is entirely borrowed from the commit path. `commit::store_change` (shared with
`commit_selected`) stores each changed file through the **same relpath-keyed streams** a commit
uses (`kra:{rel}:*` / `file:{rel}`), so a stashed `.kra`'s unchanged tiles dedup against
committed history — setting aside a lightly-edited document costs almost nothing. It also means
`gc.rs` marks a stash's content with the same walk it uses for commits, and `commit::bytes_of`
restores it with no new code.

Three things in here are load-bearing, and each has a test that fails without it:

- **A stash must not touch `index.json`.** The index is the *committed* head; a stash commits
  nothing. `store_change` returns the index entry rather than applying it, and `stash::create`
  drops it. Recording the stashed hash would make the revert below scan the file as clean and
  skip it — the set-aside would silently fail to clear the tree.
- **`create` saves before it reverts.** `discard_working_changes` erases the work from disk
  *before* its own save, so the record must already be durable when it runs; otherwise a crash
  mid-revert leaves the files reverted with no record of what was in them. Saving first inverts
  the failure mode to a harmless one (stash on the shelf, files still dirty).
- **`pop` writes the files before dropping the record**, for the mirror-image reason. A crash
  between the two just means the next pop reports a conflict — recoverable.

The operations:

- **Create** (`stash::create`) — scans, filters to `only` (the frontend's staged selection;
  staging has no backend concept of its own), stores the content, records the stash,
  then reverts via `commit::discard_working_changes`. `Nothing` if nothing in scope is dirty;
  `NoCommit` on a repo with no commits — there's no committed state to revert to, and the UI
  gates the menu items on `commits.length` the way undo does.
- **Pop** (`stash::pop`) — writes each file (`"D"` records delete instead), leaves the index alone
  — which is exactly what makes the restored work scan as changed again — and drops the record.
  A **conflict** is a stashed path that's been edited since it was set aside:
  - A conflicting **`.kra`** is **merged**, not refused: the layers the set-aside version actually
    **added or modified** are folded into the working file (clashing top-level layer names suffixed
    ` [2]`, each incoming layer's data files + uuid remapped to fresh ids) via
    [`merge::merge_layers`](../src-tauri/src/merge.rs), so the artist reconciles them by hand in
    Krita — see [Merging set-aside `.kra` work](#merging-set-aside-kra-work-on-a-pop-conflict).
  - **Any other conflict** still hard-refuses the *whole* pop with `StashConflict` before a byte is
    written — a non-`.kra` file (no layers to merge) or a stashed *deletion* landing on edited work;
    overwriting either would destroy the current work with no way back.
  - Bytes for every file are computed **before** the first disk write, so a merge that can't be done
    cleanly (`MergeFailed`) leaves the working tree and the stash untouched.

  Popping onto a different branch is allowed: `branch` is recorded for display only and nothing is
  ever looked up by it, so a stash outlives its origin branch.
- **Drop / drop-all** (`stash::drop_one` / `drop_all`) — take a stash off the shelf without
  restoring it. These run on an `open_light` repo and so must use `save_stashes()`, never
  `save()` (which would rewrite index/commits from partial state — the same hazard documented on
  `save_branches`). The content lingers until the next "Clean up storage" reclaims it.

### Merging set-aside `.kra` work on a pop conflict

When a pop finds the working `.kra` has been edited since it was set aside, both versions are the
same document taken two ways, so [`merge::merge_layers`](../src-tauri/src/merge.rs) folds the
set-aside version's **added and modified** layers into the working file rather than making the artist
choose one version to lose. The result opens in Krita with the working stack **plus** just those
set-aside changes, ready to reconcile by hand — combining *whole* layers is automatic; reconciling
pixels *within* a layer is the manual part.

Only the *changed* layers cross over, not the whole stack: `merge_layers` takes the committed
**ancestor** both sides diverged from (the file's version at the branch tip, passed by `stash::pop`)
and skips any incoming top-level layer unchanged since — matched by uuid, then compared on the
layer's **content**, not any byte-for-byte form. Each `layers/layerN…` data file is canonicalized to
its **tiles sorted by position** (`canon_entry`): Krita's tile *order* within a layer's data file
isn't stable across saves, so two saves that wrote the same tiles in a different order reconstruct to
different bytes but must still compare equal — this is the case that made an untouched layer fold in
as a duplicate. Non-tiled entries (`.defaultpixel`, `.icc`, shape-layer SVG) compare verbatim, and
the per-file blobs are collected filename-independently (Krita may renumber `layerN`). On top of the
tiles, a small set of **meaningful metadata** attributes (`name`/`opacity`/`compositeop`/`visible`/
`x`/`y`) must match. It deliberately does **not** compare the raw `<layer>` XML: Krita rewrites
volatile per-save attributes — `selected` on the active layer, `collapsed`, timeline flags — every
save, so an untouched layer's XML differs between two saves and a text compare would fold *every*
layer in (one of the bugs this path had). Anything whose tiles or meaningful metadata differ, or a
layer with no ancestor match (a genuinely new layer), folds in — so a real change is never dropped;
an obscure metadata attribute left off the list is at worst *not* folded, never spuriously
duplicated. With no ancestor (`None`) every incoming layer folds, the pre-fix behaviour. If *every*
incoming layer matches the ancestor (the set-aside change was only outside the layer stack, e.g.
canvas size), there's nothing to fold and it surfaces `MergeFailed`, untouched.

Mechanics (all on the raw `.kra`, no engine internals):
- The working file is the **base**; the set-aside version's added/modified top-level layers are
  spliced in as the first children of base's `<layers>`, so they land **on top** of the stack.
- `roxmltree` (read-only) locates each incoming `<layer>` subtree by byte range (`Node::range`) and
  base's `<layers>` insertion point; the id/name edits are whole-token string replaces
  (`filename="…"`, `uuid="…"`) — safe because uuids (`{hex}`) and filenames (`layerN`) never contain
  XML-special characters, so no XML-writer dependency is needed.
- Every incoming layer's data files and uuid are **remapped to fresh, collision-free ids** (new
  `layerN` above base's highest, checked against the actual archive too so an orphan file can't
  clash; uuids synthesized from `blake3`, no `uuid` crate) and its archive entries copied into
  base's `layers/` dir. A top-level layer whose **name** clashes with a base layer is suffixed
  ` [2]`, ` [3]`, … — opening-tag only, so nested layer/mask names are left alone.
- It **refuses** (`MergeFailed`, nothing written) rather than emit a file Krita can't open when the
  two versions use **different color spaces**; a canvas-*size* difference is allowed through (Krita
  opens it — the incoming layers may just sit at an offset).

No frontend or CLI change was needed: a merged pop returns normally through `pop_stash` / the `kvc`
CLI, so the existing "brought back" success path applies, and the artist sees the ` [2]` layers
directly in Krita. `StashConflict` still surfaces for the non-`.kra` refuse case.

## Raster delivery (`kvcimg` URI scheme)

In the desktop shell, cached diff rasters ship as `kvcimg://` URLs (`raster::raster_url`)
instead of base64 data-URLs: the webview fetches the PNG straight from `.kvc/cache/` via a
registered URI scheme handler (`commands::serve_raster`, wired in [`lib.rs`](../src-tauri/src/lib.rs))
— no base64 inflation, no multi-MB IPC payload, and content-addressed keys let the response
carry `Cache-Control: immutable` so repeat views are browser-cache hits. The handler only ever
serves `<registered repo root>/.kvc/cache/<hex key>.png` for roots a diff command has already
registered (`register_served_repo`, called only **after** the repo's `Repo::open` succeeds, so a
failed open never adds a root to the allowlist); it can't be steered at an arbitrary path.
Outside the shell, or if a cache write fails, rasters fall back to base64 data URLs, which always
work. See
[performance.md](performance.md#raster-delivery-kvcimg-uri-scheme) for the rationale.

## Tauri commands

Registered in [`lib.rs`](../src-tauri/src/lib.rs); thin wrappers in
[`commands.rs`](../src-tauri/src/commands.rs) run the heavy I/O on the blocking pool
(`spawn_blocking`) so the webview stays responsive, and flatten engine errors to strings. DTOs
use serde `camelCase` to match [`src/types.ts`](../src/types.ts).

| Command | Purpose |
|---------|---------|
| `init_repository(path)` | Create a fresh `.kvc/` store. |
| `is_repository(path)` | Does `path` already have a `.kvc/` store? |
| `open_repository(path)` | Validate + load (success = it opened). |
| `delete_repository(path)` | Delete the folder (guarded by `is_repo`), preferring the OS Recycle Bin. Returns `true` if the Recycle Bin was used, `false` if it fell back to a permanent delete. |
| `scan_repository(path)` | Working-tree changes as `WorkingChange[]` (`staged: false`). |
| `commit_snapshot(path, message, author, paths?)` | Commit working-tree changes; `paths` restricts the commit to those relative paths (the frontend's "staged" set), omitted/`null` commits everything. Returns the `Commit`. |
| `discard_changes(path, paths)` | Discard uncommitted changes, restoring them to the branch tip's committed content — no new commit. Empty `paths` discards everything dirty; otherwise only those relative paths. |
| `list_commits(path)` | Commits **reachable from the current branch tip** (oldest-first topological; the frontend reverses for newest-first). Merged branches' commits appear; other branches' don't. |
| `list_branches(path)` | All local branches as `{ name, tip, current }`. |
| `create_branch(path, name, base?)` | Create + switch to a branch. No `base` (or `base` = current): instant, at the current tip. Different `base`: materializes that branch's tree first (refused on unsaved changes). Returns the branch list. |
| `switch_branch(path, name)` | Switch the working tree to a branch (rewrites only differing files). Returns the branch list. |
| `merge_branch(path, source, author)` | Merge `source` into the current branch; returns the tip/merge `Commit`. |
| `delete_branch(path, name)` | Remove a branch label (not the current one, and never `main`). Returns the branch list. |
| `list_stashes(path)` | The set-aside shelf as `StashDto[]` (`{ id, label, author, timestamp, branch, changes }`), newest-first for the UI. Stream hashes stay backend-side. |
| `create_stash(path, label, author, paths?)` | Set aside working-tree changes and revert those files; `paths` restricts it to those relative paths (the frontend passes its staged selection), omitted/`null` sets aside everything dirty. Returns the shelf. Needs `Repo::open` — storing content writes streams, which a light repo forbids. |
| `pop_stash(path, id)` | Bring a stash back into the working tree and drop it from the shelf. Errors `"stash conflict: …"` if anything it holds is dirty (the frontend matches that prefix). Returns the shelf. |
| `drop_stash(path, id)` / `drop_all_stashes(path)` | Remove stashes without restoring them; storage is reclaimed by the next `cleanup_repository`. |
| `cleanup_repository(path, dryRun)` | Mark-and-sweep GC of everything unreachable from any branch tip **or stash**. `dryRun` reports what would be freed without deleting anything. |
| `get_repo_config(path)` | The user-editable `.kvc/config.json` knobs (`cacheMaxBytes`, `tilePixelDeltas`, `lowMemoryDiff`) for the Settings modal. Uses `Repo::open_light`. |
| `set_repo_config(path, cacheMaxBytes, tilePixelDeltas, lowMemoryDiff)` | Persist those knobs via `Repo::save_config` (config-only write — no index/chain/commit flush). |
| `layer_diff(path, file, oldCommit, newCommit)` | Per-layer metadata changes for a `.kra`. |
| `restore_file(path, file, commitId)` | Reconstruct a file at a commit and write it back. |
| `rollback_to_commit(path, commitId, author)` | Restore the whole tree to a commit; records a new commit, unless `commitId` is the current tip — then it discards uncommitted changes in place instead. |
| `undo_last_commit(path)` | Drop the last commit (keep working-tree changes); returns the new head or null. |
| `commit_diff(path, commitId)` | The commit's visual diff: `.kra` files as art diffs (**composite + layer metadata + change regions**, no per-layer rasters — those load lazily), palette files as **swatch diffs** (`palette` entries — see [Palette diffs](#palette-diffs)), others as minimal text entries. |
| `commit_layers(path, commitId, file)` | The per-layer before/after PNG rasters for one `.kra` in a commit — the heavy part, fetched on demand after `commit_diff`. |
| `working_diff(path, file)` | Working-tree file vs its last commit, same shape as `commit_diff` (composite + metadata, rasters lazy). |
| `working_layers(path, file)` | The lazy per-layer rasters for a working-tree `.kra`; the working-diff counterpart to `commit_layers`. |
| `export_repository_zip(path, dest)` | Zip the whole repository folder (art files + `.kvc/`) to `dest`. Manual backup — see [Backup & recovery](#backup--recovery). |
| `export_repositories_zip(paths, destDir)` | Zip every repo in `paths` into `destDir`, one `<folder-name>-<date>.zip` each. Returns the paths that failed rather than aborting the batch on one bad repo. |

## Backup & recovery

There is no automated backup, sync, or cloud storage — this is a local-only app and any safety
net has to be either OS-level or something the user triggers themselves. Two mechanisms cover
the two ways a repository can be lost:

- **Deleting a repository through the app** (`delete_repository`, `Repo::delete` in
  [`repo.rs`](../src-tauri/src/repo.rs)) moves the folder to the OS Recycle Bin (the `trash`
  crate) instead of removing it outright, so an accidental delete from inside the app is
  recoverable the same way an accidental Explorer/Finder delete is — restore it from the Recycle
  Bin and re-add it via Browse. `Repo::delete` falls back to a permanent `remove_dir_all` only if
  the trash move itself fails, and reports which happened (`Ok(true)` = trashed, `Ok(false)` =
  permanent) so the UI can warn rather than close silently.
- **Total loss outside the app's control** — the whole project folder deleted via `rm -rf` or
  Shift+Delete, disk failure, an external tool corrupting `.kvc/` — isn't something the app can
  intervene in after the fact. The only protection is a backup made *before* it happens: **"Back
  up this repository…"** in the Settings modal (`export_repository_zip`) zips the whole folder to
  a path the user picks via a native Save dialog — the same `.kra` files plus full version
  history, ready to move to an external drive or the user's own cloud storage. **"Back up all
  repositories…"** in the repository switcher menu (`export_repositories_zip`) does the same for
  every known repo into one chosen destination folder. Recovery is just extraction: since
  `Repo::is_repo` only checks that `.kvc/config.json` exists, unzipping a backup anywhere and
  pointing Browse at it reopens it as a fully working repository — no dedicated "restore" command.

## Frontend integration

The frontend uses [`inTauri()`](../src/lib/tauri.ts) to detect the desktop shell:

- **In Tauri** — [`useWorkingChanges`](../src/lib/repoData.ts) (shared by `ChangesPanel` and
  `Sidebar`, so both see the same staged/unstaged split off one scan) calls `scan_repository`;
  `ChangesPanel` calls `commit_snapshot` with the staged paths (or `null`/the full set, behind a
  confirm modal, when nothing or only some files are staged); [`useCommits`](../src/lib/repoData.ts)
  calls `list_commits` and maps `BackendCommit` → the frontend `Commit` shape;
  [`useBranches`](../src/lib/repoData.ts) calls `list_branches`;
  [`repository.tsx`](../src/lib/repository.tsx) drives `init`/`is`/`delete`, backup
  (`export_repository_zip`/`export_repositories_zip`), the native folder/save picker
  (`tauri-plugin-dialog`), and the mutating actions (rollback/undo, `discardChanges`,
  branch create/switch/merge/delete).
- **In a plain browser** (`npm run dev`, no backend) — the hooks return empty results and
  repository/branch actions are no-ops; the status bar shows a "Browser preview" badge. There is
  no mock data.

`list_commits` / `scan_repository` / `commit_diff` are re-fetched whenever the repository context
bumps `refreshNonce` (e.g. after a commit, rollback, or undo). Per-commit diffs are **real** for
`.kra` files, and load in **two stages** so the panel appears immediately:
[`useCommitDiff`](../src/lib/repoData.ts) calls `commit_diff` for the `mergedimage.png` composite,
layer metadata, and tile-derived change regions (fast); then
[`useArtLayers`](../src/lib/repoData.ts) lazily calls `commit_layers` (or `working_layers`) for
that file's per-layer PNG rasters (as SVG `<image>` markup, so the SVG-compositing viewer renders
them unchanged), which [`ArtDiffView`](../src/components/vcs/ArtDiffView.tsx) merges in when they
arrive. Both hooks expose a `loading` flag: `MainPanel` shows an "Analyzing changes…" spinner for
the initial diff, and `ArtDiffView` a "Loading layers…" indicator while the rasters stream in.
Layer rasters are downscaled to a longest side of `raster::MAX_RASTER_DIM` (2048px) before
encoding — a diff preview never needs full document resolution, and full-res PNG encode was the
diff's dominant cost. Palette files get real **swatch diffs** (below); other non-`.kra` files
still get minimal text entries (real line diffs are deferred).

## Palette diffs

The four palette formats ([`palette.rs`](../src-tauri/src/palette.rs)) get a real color-by-color
swatch diff, computed entirely in the backend (the frontend's `PaletteDiffView` renders the
result — see [visual-diff-viewer.md](visual-diff-viewer.md#palette-diffs)). Each format is parsed
to a flat list of named sRGB swatches:

- **`.gpl`** (GIMP) — text; `R G B  Name` lines, `Columns:` header for the grid width.
- **`.kpl`** (Krita) — a ZIP; `colorset.xml` parsed with `roxmltree` (reusing the `.kra` path's
  zip + XML deps). `RGB`/`sRGB` entries are exact; `Gray`/`CMYK` are converted.
- **`.aco`** (Adobe Color) — binary, big-endian. The v1 section gives colors (RGB exact, grayscale/
  CMYK converted); the optional v2 section supplies UTF-16 names (best-effort — a misparse keeps
  the v1 colors with hex names).
- **`.ase`** (Adobe Swatch Exchange) — binary, big-endian; color blocks carry a UTF-16 name + a
  4-char color model (`RGB `/`CMYK`/`Gray`/`LAB `, converted to sRGB), group blocks are skipped.

`palette::diff` then matches swatches **by name** (first-unconsumed, since names can repeat), so a
recolor reads as `modified` (before→after) rather than remove+add; name-only-in-new is `added`,
name-only-in-old is `removed`. `commands::palette_dto` reconstructs each side's bytes
(`file:<path>` blob stream for the committed side, disk read for the working side), runs the diff,
and serializes it as the `Palette` `DiffEntryDto` variant (`kind: "palette"`). A malformed palette
degrades to a plain text entry, so one bad file can't blank the panel. The cost is negligible
(palettes are KB-sized, parse is O(swatches)), so unlike `.kra` rasters there is no streaming or
caching — the diff is computed inline on the blocking pool.
