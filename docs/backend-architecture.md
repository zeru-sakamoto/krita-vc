# Backend Architecture

How the Rust engine (`src-tauri/`) is put together: crate layout, the module map, how a request
gets from the frontend into the store and back, the concurrency model, and the two binaries that
share this one crate. For the *feature*-level behavior of the VCS itself (commits, branches,
stashes, the `.kra` tile engine), see [version-control.md](version-control.md); for why the hot
paths are fast, see [performance.md](performance.md). This page is the structural map between
those two.

## Crate shape

`src-tauri` is one Cargo package, `krita_vc_lib`, built three ways:

```text
[lib]      krita_vc_lib          staticlib + cdylib + rlib  ← the engine, no Tauri-specific state
[[bin]]    krita-vc (default)    src/main.rs → krita_vc_lib::run()   — the Tauri desktop app
[[bin]]    kvc                   src/bin/kvc.rs                      — headless CLI, same engine
```

The `rlib` crate type is what lets `kvc` (and the test suite) link the engine directly with zero
Tauri dependency — `Cargo.toml`'s `[dependencies]` pulls in `tauri`, but nothing in `src/*.rs`
below `commands.rs`/`lib.rs` imports it. `default-run = "krita-vc"` disambiguates bare `cargo run`
since there are two `[[bin]]` targets.

## Module map

```text
src/
  repo.rs      store layout, Repo struct (load/mutate/save; `root` = the folder holding the
               tracked .kra, `store` = its history dir), store_dir_for, RepoLock, safe_join,
               decompression-bomb-capped archive reads
  scan.rs      working-tree walk (walkdir) vs index.json → U/M/D classification, is_supported
               tracking guardrail
  commit.rs    commit_snapshot/commit_selected, restore/rollback/undo, discard_working_changes
  delta.rs     store_stream/reconstruct — the generic dedup/patch/snapshot chain engine
  kra.rs       .kra archive decomposition into tile-granular streams, reconstruction, materialize
  tiles.rs     Krita tile-block binary format (parse/serialize individual 64×64 tiles)
  branch.rs    create/switch/merge/delete over branches.json
  stash.rs     set-aside/bring-back — stashes.json, shares commit.rs's storage path
  merge.rs     .kra layer-level merge for a stash-pop conflict (roxmltree + string-token surgery)
  palette.rs   .gpl/.kpl/.aco/.ase parsing + named-swatch diff
  raster.rs    PNG encode/downscale, change-highlight overlay, kvcimg:// scheme, raster cache
  gc.rs        mark-and-sweep storage reclamation (cleanup_repository)
  check.rs     read-only integrity check over stored history (check_repository, kvc check)
  cpu.rs       the budgeted rayon pool + heavy-op semaphore (see Concurrency below)
  error.rs     KvcError (thiserror) — the one error type every engine fn returns
  commands.rs  #[tauri::command] wrappers: DTOs, spawn_blocking, error-to-string
  lib.rs       tauri::Builder wiring, invoke_handler registration, kvcimg:// scheme registration
  bin/kvc.rs   headless CLI entry point (flag parsing, JSON stdout/stderr, its own lock/priority calls)
```

Below `commands.rs`, every module speaks the engine's own vocabulary (`Repo`, `Commit`,
`KvcError`) — nothing downstream of `repo.rs` knows a Tauri command or a CLI flag exists.
`commands.rs` is the only place engine types get turned into serde DTOs, and `bin/kvc.rs` is the
only other place they get turned into CLI-facing JSON — both are translation layers around the
same core, not separate implementations of it.

## Request flow

A Tauri command call and a `kvc` CLI invocation both bottom out in the same three engine steps,
just through different plumbing:

```text
Frontend (invoke)              Krita plugin (subprocess)
      │                                  │
      ▼                                  ▼
commands.rs #[tauri::command]      bin/kvc.rs main()
      │  builds DTOs, calls run()/       │  parses --flags, calls the engine fns directly,
      │  run_heavy() (spawn_blocking     │  prints one JSON object to stdout/stderr
      │  + cpu::install)                 │
      └──────────────┬───────────────────┘
                      ▼
        RepoLock::acquire(op)   (mutating calls only; both callers share <store>/kvc.lock)
                      ▼
        Repo::open / open_light  →  engine fn (commit.rs / branch.rs / stash.rs / …)
                      ▼
        Repo::save (atomic *.tmp + rename)  →  RepoLock dropped (file handle close)
```

Every fallible engine function returns `error::Result<T>` (`Result<T, KvcError>`, a `thiserror`
enum in [`error.rs`](../src-tauri/src/error.rs)). `commands.rs` is the only place that error gets
flattened to a `String` for the frontend (`Display` via `#[error(...)]` messages) — a few variants
carry a **stable prefix** the frontend pattern-matches on (`"unsaved changes"` →
`DirtyTree`, `"stash conflict"` → `StashConflict`) instead of a typed error crossing the IPC
boundary, since Tauri commands can only return `String`/serde-able errors. `bin/kvc.rs` catches
panics too (`catch_unwind` + a silenced panic hook in `main`) and reports them as the same
`{"error": "..."}` shape, since the plugin parses stdout/stderr as JSON and a bare Rust backtrace
would break that contract.

`commands.rs`'s `run`/`run_heavy` are the single funnel every Tauri command goes through:

- `run(f)` — `tauri::async_runtime::spawn_blocking(move || cpu::install(f))`. Moves the blocking
  I/O/CPU work off the async runtime (so the webview stays responsive) and wraps it in the
  budgeted rayon pool in the same step — every nested `par_iter` anywhere under `f` inherits that
  pool for free, which is why `cpu.rs`'s throttling covers the whole engine from one call site.
- `run_heavy(f)` — `run(f)`, plus a permit from `cpu::heavy_permit()` held for the call. Used by
  writes and full-document decodes (diffs, layer streams); cheap reads (`list_commits`, `status`)
  stay on plain `run` so they never queue behind a diff.

## Concurrency model

Two independent mechanisms, solving different problems:

- **`RepoLock`** ([`repo.rs`](../src-tauri/src/repo.rs)) — a real OS-level advisory lock
  (`File::try_lock`, `LockFileEx`/`flock`) over `<store>/kvc.lock`, taken by every **mutating** entry
  point in both the desktop app and the `kvc` CLI, so a plugin commit can't interleave with a
  desktop commit/switch/GC into a torn write. The engine itself has no internal locking — this is
  the only serialization point. Released automatically when the holding process's file handle
  closes, even on a crash, so there's no stale-lock state. A present-participle label
  (`"committing"`, `"switching branches"`) is written to a `kvc.lock.info` sidecar on acquire so a
  blocked caller's error names what's holding it. Read-only commands take no lock — except the
  four whose staleness is user-visible (`list_commits`, `commit_diff`, `working_diff`,
  `list_branches`), which re-check a `generation` counter on `branches.json` before/after and
  retry (bounded) if a write landed mid-read (`commands.rs` — `read_consistent`); the `kvc` CLI's
  poll trio (`status`/`branches`/`stash-list`) is deliberately excluded and stays lock-free and
  recheck-free.
- **`cpu.rs`**'s budgeted pool + `heavy_permit` semaphore — not correctness, but headroom: caps
  how much of the machine and how much concurrent memory the engine takes, so a commit or diff
  doesn't starve Krita (which triggered it) or stack unbounded 64 MB decode buffers when the UI
  fires diffs faster than they finish. See the CPU headroom section of
  [performance.md](performance.md) for the full rationale; `cpu.rs` above covers the mechanism.

These compose in a fixed order — `heavy_permit` is always acquired *outside* `RepoLock`, never the
reverse, so there's no lock-ordering hazard between the two.

## The two binaries

| | `krita-vc` (desktop app) | `kvc` (CLI) |
|---|---|---|
| Entry point | `main.rs` → `lib.rs::run()` | `bin/kvc.rs::main()` |
| Transport | Tauri IPC (`invoke`), DTOs in `commands.rs` | stdin flags → stdout/stderr JSON |
| Process priority | Unchanged (UI thread must stay responsive) | Dropped below-normal (`cpu::lower_process_priority`) — it runs *inside* Krita's process tree mid-paint, so unlike the app it has no idle moment |
| Locking | Same `RepoLock`, per Tauri command | Same `RepoLock`, per subcommand |
| Panics | Tauri's own handling | Caught (`catch_unwind`) and reported as `{"error": ...}` so the plugin's JSON parser never sees a bare backtrace |
| Consumer | The React frontend | The Krita plugin (`krita-plugin/kvc_client.py`), shelling out once per action/poll tick |

Both link `krita_vc_lib` as a normal dependency — `kvc` is not a stripped-down reimplementation,
it calls the exact same `commit`/`branch`/`scan`/`stash` functions the desktop app does.

## Third-party Rust crates

Direct dependencies declared in [`Cargo.toml`](../src-tauri/Cargo.toml) (versions as resolved in
`Cargo.lock`; `cargo metadata` is authoritative if this drifts). All are permissively licensed —
no GPL/copyleft dependency in the tree.

| Crate | Version | License | Used for |
|---|---|---|---|
| [tauri](https://github.com/tauri-apps/tauri) | 2.11.3 | MIT OR Apache-2.0 | App shell, IPC (`invoke`), window management |
| [tauri-build](https://github.com/tauri-apps/tauri) | 2.6.3 | MIT OR Apache-2.0 | Build-time codegen for the Tauri app (build-dependency) |
| [tauri-plugin-opener](https://github.com/tauri-apps/plugins-workspace) | 2.5.4 | MIT OR Apache-2.0 | Opening files/URLs with the OS default handler |
| [tauri-plugin-dialog](https://github.com/tauri-apps/plugins-workspace) | 2.7.1 | MIT OR Apache-2.0 | Native file/folder/save pickers (choosing a `.kra` to track, the store root, backup export) |
| [serde](https://github.com/serde-rs/serde) | 1.0.228 | MIT OR Apache-2.0 | Serialization framework for every on-disk/DTO type |
| [serde_json](https://github.com/serde-rs/json) | 1.0.150 | MIT OR Apache-2.0 | JSON state files (`index.json`, `branches.json`, …) and command DTOs |
| [zip](https://github.com/zip-rs/zip2) | 2.4.2 | MIT | Reading/writing `.kra`/`.kpl` archives |
| [qbsdiff](https://github.com/hucsmn/qbsdiff) | 1.4.4 | MIT | `bsdiff`/`bspatch` delta compression for the chain store |
| [blake3](https://github.com/BLAKE3-team/BLAKE3) | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Content hashing throughout (index, objects, commit ids) |
| [roxmltree](https://github.com/RazrFalcon/roxmltree) | 0.20.0 | MIT OR Apache-2.0 | Read-only XML parsing (`maindoc.xml`, `.kpl` colorset, layer-merge surgery) |
| [walkdir](https://github.com/BurntSushi/walkdir) | 2.5.0 | Unlicense OR MIT | Working-tree directory walk (the scanner) |
| [rayon](https://github.com/rayon-rs/rayon) | 1.12.0 | MIT OR Apache-2.0 | Data parallelism (tile/layer fan-out under the budgeted pool) |
| [zstd](https://github.com/gyscos/zstd-rs) | 0.13.3 | MIT | Full-snapshot compression in the delta-chain store |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.18 | MIT OR Apache-2.0 | `KvcError` derive |
| [png](https://github.com/image-rs/image-png) | 0.17.16 | MIT OR Apache-2.0 | Encoding diff-viewer raster PNGs |
| [bincode](https://github.com/servo/bincode) | 1.3.3 | MIT | Binary encoding for chain shards |
| [trash](https://github.com/ArturKovacs/trash) | 5.2.6 | MIT | Recycle-Bin-first repository deletion |
| [tokio](https://github.com/tokio-rs/tokio) | 1.52.3 | MIT | `sync` feature only — the heavy-op `Semaphore` (not the async runtime, which is Tauri's) |
| [windows-sys](https://github.com/microsoft/windows-rs) | 0.59.0 | MIT OR Apache-2.0 | Windows-only: `SetThreadPriority`/`SetPriorityClass` for CPU headroom |

Dev-only (test/bench code, not shipped): `tempfile` (3.27.0, MIT OR Apache-2.0), plus `zip`,
`serde_json`, `bincode`, `zstd` again at their dev-profile versions for constructing test fixtures
(legacy chain files, etc.).

### Frontend-side Tauri packages

The npm side of the IPC boundary ([`package.json`](../package.json)) — see
[frontend-architecture.md](frontend-architecture.md) for the rest of the frontend's dependencies:

| Package | License | Used for |
|---|---|---|
| [@tauri-apps/api](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 | `invoke`, window controls (custom title bar) |
| [@tauri-apps/plugin-dialog](https://github.com/tauri-apps/plugins-workspace) | MIT OR Apache-2.0 | JS bindings for the native file/folder/save picker |
| [@tauri-apps/plugin-opener](https://github.com/tauri-apps/plugins-workspace) | MIT OR Apache-2.0 | JS bindings for opening files/URLs |
| [@tauri-apps/cli](https://github.com/tauri-apps/tauri) | MIT OR Apache-2.0 | `npm run tauri` dev/build tooling (dev-dependency) |
