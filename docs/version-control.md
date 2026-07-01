# File Tracking & Version Control

The Rust backend (`src-tauri/src/`) is a **custom local VCS** purpose-built for Krita art
files — `git2` was evaluated and dropped. It stores history in a `.kvc/` folder inside each
repository, decomposing `.kra` archives down to individual 64×64 tiles so an edit only stores
the tiles that actually changed. Local-only: no remotes, no network.

The frontend calls into this through Tauri commands (`@tauri-apps/api/core` `invoke`); see
[Swapping in the backend](#frontend-integration) for how the UI consumes it (and falls back to
mock data in a plain browser).

## `.kvc/` store layout

`init_repository` creates this inside the repo root ([`repo.rs`](../src-tauri/src/repo.rs)):

```text
.kvc/
  config.json    engine config: delta-chain threshold (default 20), tile size (64)
  index.json     committed head per tracked file — drives the scanner
  chains.json    every stored version of every delta stream — drives storage/restore
  commits.json   the commit log (oldest-first)
  objects/       content-addressed blobs: <hash>.full (zstd) or <hash>.patch (bsdiff)
```

State is loaded into a `Repo` struct (`Repo::open`), mutated in memory, then flushed with
`Repo::save`. JSON writes are **atomic** (write to `*.json.tmp`, then `rename` over the target).
Hashing is **blake3** throughout (`hash_bytes`); timestamps are ISO-8601 UTC computed without a
date crate (`now_iso` / `epoch_to_iso`).

## File tracking — the scanner

[`scan.rs`](../src-tauri/src/scan.rs) walks the working tree (`walkdir`) and classifies each
file against `index.json`:

| Status | Meaning |
|--------|---------|
| `U` | untracked — not in the index |
| `M` | modified — blake3 differs from the index head |
| `D` | deleted — in the index but absent on disk |

Unchanged files produce nothing. The `.kvc/` directory and Krita lock/autosave files (`*.kra~`)
are skipped. ponytail: every file is re-hashed each scan — fine for art repos; cache size+mtime
in the index if it ever bites. There is **no staging area** — the scanner reports the whole
working-tree delta and a commit captures all of it (the frontend's stage toggles are cosmetic).

## Committing — `commit_snapshot`

[`commit.rs`](../src-tauri/src/commit.rs) scans, then routes each change:

- **deletion** → drop from the index, record a `D` file entry (no content).
- **`.kra`** → `kra::commit_kra` decomposes the archive (see below) and returns its manifest hash.
- **anything else** → `Repo::store_stream("file:<path>", bytes)` returns the blob's content hash.

Each non-deleted file's blake3 is written back into the index, and a `Commit` is recorded with
`parents` set to the previous head (linear history; first parent = mainline). Returns
`KvcError::Nothing` if the tree is clean. The commit id/hash is the first 12 hex chars of a
blake3 over the timestamp + message + per-file content hashes.

## Delta-chain storage

[`delta.rs`](../src-tauri/src/delta.rs) — a **stream** is any versioned byte sequence (a generic
file, a `.kra` manifest, a layer entry, or a single tile), keyed by a string. `store_stream`:

1. **Dedup** — if the content hash already exists in the stream's chain, return it; store nothing.
2. **Patch** — if the chain head is under `delta_chain_max` (20), store a `bsdiff` patch against
   the head (`<hash>.patch`).
3. **Snapshot** — otherwise (first version, or threshold reached) store a fresh `zstd` full
   snapshot (`<hash>.full`), resetting the chain length.

`reconstruct(key, hash)` walks the patch chain back to its full snapshot, applies patches, and
**verifies** the rebuilt bytes hash to the requested hash (integrity guard). Objects are
content-addressed, so writing an object that already exists is a no-op (cross-file dedup).

## `.kra` tile engine

A `.kra` is a ZIP. [`kra.rs`](../src-tauri/src/kra.rs) + [`tiles.rs`](../src-tauri/src/tiles.rs)
decompose it into streams so small edits stay small:

- **Tiled layer-data entries** (binary blocks under `<doc>/layers/`, detected by a `VERSION `
  header) are parsed into individual tiles; **each tile becomes its own stream**
  (`kra:<path>:tile:<entry>:<x>,<y>`). Unchanged tiles dedup automatically — a one-corner edit
  only stores those tiles.
- **Every other archive entry** becomes one stream (`kra:<path>:entry:<name>`).
- A **JSON manifest** (`kra:<path>:manifest`) records entry order, per-entry blob hashes, and
  per-tile refs — enough to reassemble a logically identical archive (`mimetype` stays first and
  stored; tiles re-emitted uncompressed in their original block format).

ponytail: tiles are diffed as opaque LZF-compressed blobs — Krita's LZF is never decoded, so
delta quality on already-compressed bytes is limited. Upgrade path: decode LZF and delta raw
pixels if footprint ever demands it.

`maindoc.xml` is also parsed (`parse_maindoc`, via `roxmltree` with DTD allowed) so layer
metadata changes — added / removed / opacity / blend / rename, matched by uuid then name — can be
reported between two commits (`diff_maindoc`).

## Restoring, rollback & undo

`commit::file_at_commit` rebuilds the exact bytes of a file as of any commit: for `.kra` it
reconstructs from the manifest (`reconstruct_kra`), otherwise from the blob stream. The
`restore_file` command writes those bytes back into the working tree.

Two higher-level history operations build on this ([`commit.rs`](../src-tauri/src/commit.rs)):

- **Rollback** (`rollback_to_commit`) — since commits store only their *changed* files, the full
  tree state at commit N is the fold of every commit up to N (`tree_at`, last-writer-wins per
  path, dropping deletions). Rollback materializes that state into the working tree (writing each
  file, deleting ones that didn't exist at N) then records it as a **new** commit via
  `commit_snapshot` — non-destructive; history stays linear and reversible. Returns `Nothing` if
  the tree already matches.
- **Undo last commit** (`undo_last_commit`) — a *soft* reset: pops the last commit and rewinds
  only the index entries for the paths it touched (recomputing each file's whole-file blake3 from
  its most-recent surviving version). The working tree is left untouched, so the undone edits
  resurface as uncommitted changes. Orphaned objects/chain versions are left in place (they're
  content-addressed and dedup on any re-commit).

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
| `delete_repository(path)` | Permanently delete the folder (guarded by `is_repo`). |
| `scan_repository(path)` | Working-tree changes as `WorkingChange[]` (`staged: false`). |
| `commit_snapshot(path, message, author)` | Commit the whole working tree; returns the `Commit`. |
| `list_commits(path)` | The commit log (oldest-first; the frontend reverses for newest-first). |
| `layer_diff(path, file, oldCommit, newCommit)` | Per-layer metadata changes for a `.kra`. |
| `restore_file(path, file, commitId)` | Reconstruct a file at a commit and write it back. |
| `rollback_to_commit(path, commitId, author)` | Restore the whole tree to a commit; record a new commit. |
| `undo_last_commit(path)` | Drop the last commit (keep working-tree changes); returns the new head or null. |
| `commit_diff(path, commitId)` | The commit's visual diff: `.kra` files as art diffs (per-layer PNG rasters + composite + change regions), others as minimal text entries. |

## Frontend integration

The frontend uses [`inTauri()`](../src/lib/tauri.ts) to detect the desktop shell:

- **In Tauri** — [`ChangesPanel`](../src/components/vcs/ChangesPanel.tsx) calls `scan_repository`
  / `commit_snapshot`; [`useCommits`](../src/lib/repoData.ts) calls `list_commits` and maps
  `BackendCommit` → the frontend `Commit` shape; [`repository.tsx`](../src/lib/repository.tsx)
  drives `init`/`is`/`delete` and the native folder picker (`tauri-plugin-dialog`).
- **In a plain browser** (`npm run dev`, no backend) — these fall back to the mock modules in
  `src/data/` so the UI can still be built and reviewed.

`list_commits` / `scan_repository` / `commit_diff` are re-fetched whenever the repository context
bumps `refreshNonce` (e.g. after a commit, rollback, or undo). Per-commit diffs are now **real**
for `.kra` files: [`useCommitDiff`](../src/lib/repoData.ts) calls `commit_diff`, which supplies
per-layer PNG rasters (as SVG `<image>` markup, so the existing SVG-compositing viewer renders
them unchanged) plus a `mergedimage.png` composite and tile-derived change regions. Non-`.kra`
files still get minimal text entries (real line/palette diffs are deferred). In a plain browser
the hook falls back to the `src/data/` mock diffs.
