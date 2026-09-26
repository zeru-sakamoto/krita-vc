# Backend architecture

How the Rust engine (`src-tauri/`) is put together: the crate layout, the module map, how a request
gets from the frontend into the store and back, the concurrency model, the two binaries that share
the crate, and a reference for every Tauri command and CLI subcommand. For what the version control
system does (commits, branches, stashes, the `.kra` tile engine), see
[version-control.md](version-control.md). For why the hot paths are fast, see
[performance.md](performance.md). This page is the structural map between the two.

## Crate shape

`src-tauri` is one Cargo package, `krita_vc_lib`, built three ways:

```text
[lib]      krita_vc_lib          staticlib + cdylib + rlib  (the engine, with no Tauri-specific state)
[[bin]]    krita-vc (default)    src/main.rs → krita_vc_lib::run()   the Tauri desktop app
[[bin]]    kvc                   src/bin/kvc.rs                      the headless CLI, same engine
```

The `rlib` crate type is what lets `kvc` and the test suite link the engine directly with no Tauri
dependency. `Cargo.toml`'s `[dependencies]` pull in `tauri`, but nothing in `src/*.rs` below
`commands.rs` and `lib.rs` imports it. `default-run = "krita-vc"` settles which binary a bare
`cargo run` means, since there are two `[[bin]]` targets.

Every installer bundles both binaries: `kvc.exe` sits next to `krita-vc.exe` on Windows, the
`.deb` and `.rpm` install `/usr/bin/kvc`, and the macOS app carries it in `Contents/MacOS/kvc`. That's
how the Krita plugin finds the CLI without a separate download.

## Module map

```text
src/
  repo.rs      the store layout and the Repo struct (open, change, save; `root` is the folder
               holding the tracked .kra, `store` its history), store_dir_for and the custom store
               root, RepoLock, safe_join, archive reads capped against decompression bombs, and
               backup export and import
  scan.rs      the tracked document versus index.json → U/M/D, with the size+mtime fast path and
               the racy-clean guard; is_supported gates what can be tracked
  commit.rs    commit_snapshot and commit_selected, restore, rollback and undo,
               discard_working_changes
  stage.rs     layer-subset staging: synthesizes the .kra a partial commit stores (stage_kra,
               committed_subset, plan_pieces)
  delta.rs     store_stream and reconstruct, the generic dedup, patch and snapshot chain engine;
               loose objects and pack files
  kra.rs       .kra decomposition into tile-level streams, rebuilding, incremental materialize,
               manifests and working-file parsing
  tiles.rs     Krita's tile-block binary format (parse and write individual 64×64 tiles)
  branch.rs    create, create at a commit, switch, merge and delete, over branches.json
  stash.rs     set aside and bring back: stashes.json, sharing commit.rs's storage path
  merge.rs     the layer-level .kra merge for bringing set-aside work back onto an edited painting
               (roxmltree plus string-token surgery)
  palette.rs   .gpl, .kpl, .aco and .ase parsing and the named-swatch diff
  raster.rs    PNG encoding and downscaling, the change-highlight overlay, the stack compositor,
               kvcimg:// URLs and the raster cache
  gc.rs        mark-and-sweep storage reclamation (cleanup_repository), trash quarantine
  check.rs     the read-only integrity check over stored history (check_repository, kvc check)
  cpu.rs       the budgeted rayon pool and the heavy-operation semaphore (see cpu-headroom.md)
  diskspace.rs the free-space preflight before big writes (Windows-only; skipped elsewhere)
  ops_log.rs   the append-only audit log for undo, discard, cleanup and branch delete
  error.rs     KvcError (thiserror), the one error type every engine function returns
  commands.rs  the #[tauri::command] wrappers: DTOs, spawn_blocking, errors turned into strings
  lib.rs       tauri::Builder wiring, invoke_handler registration, the kvcimg:// scheme
  bin/kvc.rs   the CLI entry point (flag parsing, JSON on stdout or stderr, its own lock and
               priority calls)
```

Below `commands.rs`, every module speaks the engine's own vocabulary (`Repo`, `Commit`,
`KvcError`); nothing past `repo.rs` knows a Tauri command or a CLI flag exists. `commands.rs` is the
only place engine types become serde DTOs, and `bin/kvc.rs` is the only other place they become
CLI-facing JSON. Both are translation layers around one core, not separate implementations of it.

## Request flow

A Tauri command and a `kvc` invocation end up in the same three engine steps through different
plumbing:

```text
Frontend (invoke)              Krita plugin (subprocess)
      │                                  │
      ▼                                  ▼
commands.rs #[tauri::command]      bin/kvc.rs main()
      │  builds DTOs, calls run()        │  parses --flags, calls the engine directly,
      │  or run_heavy() (spawn_blocking  │  prints one JSON object to stdout or stderr
      │  plus cpu::install)              │
      └──────────────┬───────────────────┘
                     ▼
       RepoLock::acquire(op)    (writes only; both share <store>/kvc.lock)
                     ▼
       Repo::open / open_light  →  engine function (commit.rs, branch.rs, stash.rs, …)
                     ▼
       Repo::save (atomic *.tmp plus rename)  →  RepoLock dropped (file handle closed)
```

Every fallible engine function returns `error::Result<T>` (`Result<T, KvcError>`, a `thiserror`
enum in [`error.rs`](../src-tauri/src/error.rs)). `commands.rs` is the only place that error is
flattened to a `String` for the frontend (its `Display` comes from the `#[error(...)]` messages).
Tauri commands can only return strings or serde-able errors, so instead of a typed error crossing the
IPC boundary, a few variants carry a stable prefix the frontend matches on: `"unsaved changes"` for
`DirtyTree`, `"stash conflict"` for `StashConflict`, and `"history isn't reachable"` for
`StoreUnreachable`. `bin/kvc.rs` also catches panics (`catch_unwind` plus a silenced panic hook in
`main`) and reports them in the same `{"error": "..."}` shape, since the plugin parses stdout and
stderr as JSON and a bare Rust backtrace would break it.

`commands.rs`'s `run` and `run_heavy` are the single funnel every Tauri command goes through:

- `run(f)` is `tauri::async_runtime::spawn_blocking(move || cpu::install(f))`. It moves the blocking
  I/O and CPU work off the async runtime, so the webview stays responsive, and runs it inside the
  budgeted rayon pool in the same step. Every nested `par_iter` under `f` inherits that pool, which
  is why the CPU budget covers the whole engine from one call site.
- `run_heavy(f)` is `run(f)` plus a permit from `cpu::heavy_permit()`, held for the call. Writes and
  full-document decodes (diffs, layer streams) use it. Cheap reads (`list_commits`, `status`) stay on
  plain `run`, so they never queue behind a diff.

## Concurrency model

Two independent mechanisms solve different problems.

- **`RepoLock`** ([`repo.rs`](../src-tauri/src/repo.rs)) is an OS-level advisory lock
  (`File::try_lock`: `LockFileEx` or `flock`) on `<store>/kvc.lock`, taken by every entry point that
  writes, in both the desktop app and the `kvc` CLI. A plugin commit can't interleave with a desktop
  commit, switch or GC into a torn write. The engine itself has no internal locking, so this is the
  only point of serialization. The OS releases it when the holder's file handle closes, even on a
  crash, so there's no stale-lock state. On acquire, a present-participle label ("committing",
  "switching branches") goes into a `kvc.lock.info` file so a blocked caller's error names what's
  holding the lock. Read-only commands take no lock, except that the four whose staleness would be
  visible (`list_commits`, `commit_diff`, `working_diff`, `list_branches`) re-check a `generation`
  counter in `branches.json` before and after and retry, a bounded number of times, if a write landed
  in between (`read_consistent` in `commands.rs`). The CLI's poll commands (`status`, `branches`,
  `stash-list`) are deliberately left out and stay lock-free and recheck-free.
- **`cpu.rs`'s budgeted pool and `heavy_permit` semaphore** aren't about correctness but about
  headroom. They cap how much of the machine, and how much memory at once, the engine takes, so a
  commit or diff doesn't starve Krita or stack unbounded 64 MB decode buffers when the UI asks for
  diffs faster than they finish. See [cpu-headroom.md](cpu-headroom.md).

The two compose in a fixed order: `heavy_permit` is always taken outside `RepoLock`, never the
reverse, so there's no lock-ordering hazard between them.

## The two binaries

| | `krita-vc` (desktop app) | `kvc` (CLI) |
|---|---|---|
| Entry point | `main.rs` → `lib.rs::run()` | `bin/kvc.rs::main()` |
| Transport | Tauri IPC (`invoke`), DTOs in `commands.rs` | Command-line flags in, one JSON object out on stdout or stderr |
| Process priority | Unchanged (the UI thread has to stay responsive) | Lowered below normal (`cpu::lower_process_priority`): it runs inside Krita's process tree while Krita paints, so unlike the app it has no idle moment |
| Locking | The same `RepoLock`, per Tauri command | The same `RepoLock`, per subcommand |
| Panics | Tauri's own handling | Caught (`catch_unwind`) and reported as `{"error": ...}` so the plugin's JSON parser never sees a bare backtrace |
| Consumer | The React frontend | The Krita plugin ([`krita-plugin/kritavc/kvc_client.py`](../krita-plugin/kritavc/kvc_client.py)), run once per action and per poll tick |

Both link `krita_vc_lib` as an ordinary dependency. `kvc` isn't a stripped-down reimplementation;
it calls the same `commit`, `branch`, `scan` and `stash` functions the desktop app does.

## Tauri command reference

Registered in [`lib.rs`](../src-tauri/src/lib.rs) (39 commands); thin wrappers in
[`commands.rs`](../src-tauri/src/commands.rs) run the heavy I/O on the blocking pool
(`spawn_blocking`) so the webview stays responsive, and flatten engine errors to strings. DTOs use
serde `camelCase` to match [`src/types.ts`](../src/types.ts). Every command's `path` is the `.kra`
document, not a folder, and the engine resolves its store from it (`repo::store_dir_for`), so no
signature changed when tracking went per-document.

### Tracking

| Command | What it does |
|---------|--------------|
| `init_repository(path)` | Start tracking one `.kra`, creating its store where `store_dir_for` says (the `.kvc/` container beside it by default). Refuses a file that isn't a `.kra` (`Unsupported`), one that isn't there, and one already tracked (`AlreadyRepo`). |
| `is_repository(path)` | Is this `.kra` already tracked? |
| `open_repository(path)` | Validate and load. Tells `NotARepo` (never versioned) apart from `StoreUnreachable` (the history is on a drive that isn't mounted); treating the second like the first would offer to start tracking and orphan every saved version. |
| `delete_repository(path)` | Delete the document's store, preferring the Recycle Bin, and remove the container if it's now empty. Never touches the artwork. Returns `true` if the Recycle Bin was used, `false` for a permanent delete. |
| `get_store_root()`, `set_store_root(path?)` | Where new stores are created; `null` means the default, beside each document. App-global (the `kvc` CLI reads the same file), and it deliberately doesn't move existing stores. |

### Working tree and versions

| Command | What it does |
|---------|--------------|
| `scan_repository(path)` | The working-tree changes as `WorkingChange[]`, which has at most one entry, the tracked document. |
| `commit_snapshot(path, message, author, paths?, layers?)` | Commit the working-tree changes. `paths` limits the commit to those relative paths; omitted or `null` commits everything. `layers` limits it, within the tracked `.kra`, to those top-level layer ids (`LayerDto.id`), and the unticked layers stay uncommitted (see [layer-staging.md](layer-staging.md)). Returns the `Commit`. |
| `discard_changes(path, paths)` | Put uncommitted changes back to the branch tip's content, with no new commit. Empty `paths` discards everything dirty; otherwise only those relative paths. |
| `list_commits(path, allBranches?)` | Commits reachable from the current branch tip, oldest first in topological order (the frontend reverses them). Merged branches' commits appear; other branches' don't. `allBranches: true` (default false) takes the union over every branch tip instead, for the Version Map's "show all lines". |
| `rollback_to_commit(path, commitId, author)` | Restore the whole tree to a commit and record a new commit, unless `commitId` is the current tip, in which case it discards uncommitted changes in place. |
| `undo_last_commit(path)` | Drop the last commit and keep its changes in the working tree. Returns the new head, or null. |
| `restore_file(path, file, commitId)` | Rebuild one file as of a commit and write it back. |

### Branches

| Command | What it does |
|---------|--------------|
| `list_branches(path)` | Every local branch as `{ name, tip, current }`. |
| `create_branch(path, name, base?, commit?)` | Create a branch and switch to it. With no `base` or `commit` (or `base` equal to the current branch), it's instant, at the current tip. A different `base` writes that branch's tree first (refused on unsaved changes). `commit` does the same starting at any commit id (`create_branch_at`); it can't be combined with `base` and wins if both are given. Returns the branch list. |
| `switch_branch(path, name)` | Switch the working tree to a branch, rewriting only files that differ. Returns the branch list. |
| `merge_branch(path, source, author)` | Merge `source` into the current branch. Returns the tip or merge `Commit`. |
| `delete_branch(path, name)` | Remove a branch label (never the current branch, never `main`). Returns the branch list. |

### Setting work aside

| Command | What it does |
|---------|--------------|
| `list_stashes(path)` | The shelf as `StashDto[]`, newest first. |
| `create_stash(path, label, author, paths?)` | Set aside changes and revert those files. Returns the shelf. |
| `pop_stash(path, id)` | Bring a stash back and drop it from the shelf; a `"stash conflict: …"` error when it can't. Returns the shelf. |
| `drop_stash(path, id)`, `drop_all_stashes(path)` | Remove stashes without restoring them. |

See [stashes.md](stashes.md#commands) for the details.

### Diffs

| Command | What it does |
|---------|--------------|
| `commit_diff(path, commitId)` | A version's visual diff against its first parent: the `.kra` as an art diff (composite, layer metadata and change regions, but no per-layer rasters, which load lazily), plus a palette entry for each embedded palette that changed. |
| `commit_layers(path, commitId, file)` | The per-layer before and after rasters for one `.kra` in a commit, streamed over a Tauri `Channel`. |
| `working_diff(path, file)` | The working file against its last commit, in the same shape as `commit_diff`. |
| `working_layers(path, file)` | The per-layer rasters for the working `.kra`, the working counterpart of `commit_layers`. |
| `layer_diff(path, file, oldCommit, newCommit)` | Per-layer metadata changes between any two commits. Registered, but not called by the frontend today. |

### Storage, integrity and settings

| Command | What it does |
|---------|--------------|
| `cleanup_repository(path, dryRun)` | Mark-and-sweep GC of everything in this document's store unreachable from any of its branch tips or stashes. Stores share nothing, so the sweep can never reach another painting's history. Victims go to `<store>/trash/` (pruned after 14 days, along with histories a restore replaced and old commit-log copies). `dryRun` reports what would be freed without touching anything. Both refuse with `DamagedHistory` when the walk from a tip reaches a version the log doesn't have, since everything behind that gap would otherwise be swept. |
| `check_repository(path, scrub)` | The read-only integrity check: missing objects, broken chains, dangling branch tips, versions whose parent is missing, commit-log lines that won't decode, chain shards that won't decode, packs that won't read. Takes no lock and writes nothing; findings come back in the report, not as an error. `scrub` (off by default) also re-hashes every live version's content. |
| `repo_storage_stats(path)` | The Performance tab's storage figures: stored bytes against a full copy per version (see [performance-report.md](performance-report.md)). |
| `get_repo_config(path)` | The editable `<store>/config.json` settings (`cacheMaxBytes`, `tilePixelDeltas`, `lowMemoryDiff`), through `Repo::open_light`. |
| `set_repo_config(path, cacheMaxBytes, tilePixelDeltas, lowMemoryDiff)` | Save those settings through `Repo::save_config`, a config-only write. |
| `set_cpu_budget(percent)` | Rebuild the engine's worker pool at a new share of the cores (see [cpu-headroom.md](cpu-headroom.md)). |

### Backup and restore

`export_repositories_zip`, `read_backup_manifest`, `plan_restore`, `compare_restore_versions` and
`import_repository_zip` are described in
[backup-and-restore.md](backup-and-restore.md#commands).

## `kvc` CLI reference

`kvc <subcommand> --repo <path to the .kra> [flags]`. The flag is still called `--repo` from the
folder era; the plugin passes the document path and the engine resolves the store from it. Every
subcommand prints one JSON object to stdout, or `{"error": "..."}` to stderr with a non-zero exit.
List-valued flags (`--paths`, `--layers`) take a JSON array, because the hand-written flag parser is a
map (a repeated flag would overwrite) and paths can contain commas. Leaving a list flag out means
"everything".

| Subcommand | Flags | Lock label |
|---|---|---|
| `status` | `--repo` | none (read) |
| `commit` | `--repo --message --author [--paths] [--layers]` | "committing" |
| `branches` | `--repo` | none (read) |
| `switch` | `--repo --branch` | "switching branches" |
| `create-branch` | `--repo --name [--base]` | "creating a branch" |
| `discard` | `--repo [--paths]` | "discarding changes" |
| `stash` | `--repo --author [--label] [--paths]` | "setting work aside" |
| `stash-pop` | `--repo --id` | "bringing back set-aside work" |
| `stash-list` | `--repo` | none (read) |
| `check` | `--repo [--scrub true]` | none (read) |

`status` returns the changes, the current `branch`, the full `branches` list (so the plugin's poll
needs one process per tick, not two), a `stashes` count and the tracked `document`. `stash-list`
reuses `commands::stash_dtos` for its newest-first order, which the plugin's "Bring back latest"
relies on. `check` reports problems as a successful run; `{"error": …}` means the check itself
failed. `create_branch_at` is deliberately not exposed, because the plugin has no version picker.

Run with no arguments, `kvc` prints a usage line starting with `usage: kvc`. That prefix matters:
the plugin's "Locate kvc…" picker identifies the binary by it, so the list of commands can change,
but the prefix can't. (The usage line currently omits `check`.) The contract tests in
[`src-tauri/tests/kvc_cli.rs`](../src-tauri/tests/kvc_cli.rs) spawn the real binary and check the
JSON shapes the plugin parses.

## Third-party Rust crates

The direct dependencies declared in [`Cargo.toml`](../src-tauri/Cargo.toml), at the versions
resolved in `Cargo.lock` (`cargo metadata` is authoritative if this drifts). All are permissively
licensed; there's no GPL or other copyleft dependency in the tree.

| Crate | Version | License | Used for |
|---|---|---|---|
| [tauri](https://github.com/tauri-apps/tauri) | 2.11.3 | MIT OR Apache-2.0 | The app shell, IPC (`invoke`), window management |
| [tauri-build](https://github.com/tauri-apps/tauri) | 2.6.3 | MIT OR Apache-2.0 | Build-time code generation for the Tauri app (build dependency) |
| [tauri-plugin-opener](https://github.com/tauri-apps/plugins-workspace) | 2.5.4 | MIT OR Apache-2.0 | Opening files and URLs with the OS default handler |
| [tauri-plugin-dialog](https://github.com/tauri-apps/plugins-workspace) | 2.7.1 | MIT OR Apache-2.0 | Native file, folder and save pickers (choosing a `.kra` to track, the store root, backup and restore) |
| [serde](https://github.com/serde-rs/serde) | 1.0.228 | MIT OR Apache-2.0 | Serialization for every on-disk and DTO type |
| [serde_json](https://github.com/serde-rs/json) | 1.0.150 | MIT OR Apache-2.0 | JSON state files (`index.json`, `branches.json` and the rest) and command DTOs |
| [zip](https://github.com/zip-rs/zip2) | 2.4.2 | MIT | Reading and writing `.kra` and `.kpl` archives and backups |
| [qbsdiff](https://github.com/hucsmn/qbsdiff) | 1.4.4 | MIT | bsdiff and bspatch delta compression for the chain store |
| [blake3](https://github.com/BLAKE3-team/BLAKE3) | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Content hashing throughout (index, objects, commit ids) |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | 0.20.0 | MIT OR Apache-2.0 | Read-only XML parsing (`maindoc.xml`, `.kpl` color sets, layer merging and staging) |
| [walkdir](https://github.com/BurntSushi/walkdir) | 2.5.0 | Unlicense OR MIT | Walking a store's files when writing a backup |
| [rayon](https://github.com/rayon-rs/rayon) | 1.12.0 | MIT OR Apache-2.0 | Data parallelism (tile and layer fan-out inside the budgeted pool) |
| [zstd](https://github.com/gyscos/zstd-rs) | 0.13.3 | MIT | Full-snapshot compression in the chain store |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.18 | MIT OR Apache-2.0 | The `KvcError` derive |
| [png](https://github.com/image-rs/image-png) | 0.17.16 | MIT OR Apache-2.0 | Encoding and decoding the diff viewer's rasters and composite blocks |
| [bincode](https://github.com/servo/bincode) | 1.3.3 | MIT | Binary encoding for chain shards |
| [trash](https://github.com/ArturKovacs/trash) | 5.2.6 | MIT | Sending a deleted history to the Recycle Bin or Trash |
| [tokio](https://github.com/tokio-rs/tokio) | 1.52.3 | MIT | The `sync` feature only, for the heavy-operation `Semaphore` (the async runtime is Tauri's) |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.59.0 | MIT OR Apache-2.0 | Windows only: thread and process priority, the free-space check, and hiding the `.kvc` container |

Dev-only (tests and benchmarks, not shipped): `tempfile` (3.27.0, MIT OR Apache-2.0), plus `zip`,
`serde_json`, `bincode` and `zstd` again, for building test fixtures such as legacy chain files.

### Frontend-side Tauri packages

The npm side of the IPC boundary ([`package.json`](../package.json)); see
[frontend-architecture.md](frontend-architecture.md) for the rest of the frontend's dependencies.

| Package | License | Used for |
|---|---|---|
| [@tauri-apps/api](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 | `invoke`, window controls (the custom title bar) |
| [@tauri-apps/plugin-dialog](https://github.com/tauri-apps/plugins-workspace) | MIT OR Apache-2.0 | JavaScript bindings for the native pickers |
| [@tauri-apps/plugin-opener](https://github.com/tauri-apps/plugins-workspace) | MIT OR Apache-2.0 | JavaScript bindings for opening files and URLs |
| [@tauri-apps/cli](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 | `npm run tauri` dev and build tooling (dev dependency) |
