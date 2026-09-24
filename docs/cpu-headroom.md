# CPU headroom

[performance.md](performance.md) is about throughput: finishing each operation as fast as
possible. That's the wrong default on the machines this app is for. Rayon's global pool is sized to
`num_cpus`, and with 17 `par_iter` sites nested three deep on both hot paths (a diff goes layers →
tiles → downscale rows; a commit goes entries → tiles → bsdiff, zstd and blake3), a commit or a diff
used to pin every logical core at normal priority. On a laptop with two to four cores that starves
Krita, which is often the very thing that started the commit, through the plugin, along with the
browser and the rest of the desktop. Being 20% faster is worth nothing if the artist can't draw
while it runs.

So the engine runs on its own pool ([`cpu.rs`](../src-tauri/src/cpu.rs)), not on rayon's global one.

## Its own pool

- **Worker threads below normal priority.** `ThreadPoolBuilder::start_handler` drops each worker to
  `THREAD_PRIORITY_BELOW_NORMAL` once, when it starts. This is Windows-only (through `windows-sys`)
  and a no-op elsewhere, with `libc::nice` noted as the upgrade path. It does most of the real work:
  the OS scheduler always prefers Krita's paint thread over these workers, so even at a 100% budget
  the desktop stays responsive. It costs nothing when the machine is otherwise idle, which is the
  point of a priority hint rather than a hard cap.
- **A thread budget**, by default 75% of logical cores (8 → 6, 4 → 3, 2 → 1), with a minimum of 1.
  This is the belt to the priority's braces, and the knob an artist can actually reason about. It is
  "Background CPU use" in Settings → Performance, a slider with three stops: Gentle (50), Balanced
  (75, recommended) and Full speed (100). The setting is app-global rather than per painting, since
  the pool is process-wide and two open paintings would otherwise fight over it, so the row renders
  outside that tab's "open a painting first" gate and says it applies everywhere. It's stored in
  `localStorage` ([`src/lib/cpuBudget.tsx`](../src/lib/cpuBudget.tsx), the same shape as
  `windowChrome.tsx`) and pushed to the backend with `set_cpu_budget`. Changing it swaps in a fresh
  pool. Work already running finishes on the old pool, which is dropped with its last `Arc`, so no
  restart is needed.

The engine builds its own pool instead of calling `build_global()` because the global pool can only
be initialized once, and the budget is a live setting.

The integration is one line. Every Tauri command goes through `commands::run`, and nested
`par_iter`s inherit the pool that installed them, so wrapping that one closure in `cpu::install`
puts the whole engine inside the budget: all three nesting levels, blake3's `update_rayon`
included. No call site knows the budget exists. A unit test in `cpu.rs` pins that inheritance,
because everything depends on it.

## What it costs

`cpu_budget_sweep` in `tests/bench.rs` measures the trade-off
(`cargo test --release --test bench -- --ignored --nocapture`). It times a first commit and a cold
layer raster at 100, 75 and 50. One run on a 4-core Windows machine, so treat small differences as
noise:

```text
budget      threads        commit        raster
100               4         2.46s         1.43s   (baseline)
75                3         2.02s         1.27s   (-18% / -11%)
50                2         2.56s         1.26s   ( +4% /  -12%)
```

Headroom is close to free here: 75% was not slower than 100%, and even halving the pool cost about
4% on the commit and nothing measurable on the raster. Both paths are limited by I/O and serial
work (the zip walk, the chain fold) as much as by parallel throughput, so the last cores bought
little. That's what justifies 75% as the default. It is one machine, though, and a many-core box
with fast storage would show a real cost, which is why the setting exists.

## The kvc CLI lowers its whole process

`cpu::lower_process_priority` (`SetPriorityClass`, the process-wide sibling of the per-thread hint)
is called once at the top of `kvc.rs`'s `main`, and the dispatch runs inside `cpu::install`. The CLI
needs more protection than the desktop app, not less. The plugin starts it inside Krita's process
tree while Krita is painting, so it has no idle moment to work in, and its main thread does engine
work directly instead of only feeding rayon workers. `install` sits inside the existing
`catch_unwind`, so a panic on a worker still ends as the `{"error": ...}` JSON the plugin expects.

## Two heavy operations at a time

The pool bounds cores; it did nothing for memory. Every command gets its own `spawn_blocking`
(tokio's default cap is 512 threads), and each heavy operation carries its own 64 MB
`RESTORE_CHUNK_BUDGET`. And cancelling a diff in the UI does not cancel the backend: `useArtLayers`'
cleanup only sets a flag that makes `onmessage` drop messages, while Rust rasterizes every layer to
the end. So clicking quickly through history used to stack up unbounded work.

`cpu::heavy_permit` is a two-permit `tokio::sync::Semaphore`. `commands::run_heavy` takes one and
holds it for the call. Two covers the normal case of one view in flight and one arriving, with no
added latency. Cheap reads (`list_commits`, `list_branches`, the config getters) deliberately stay
on plain `run`, so they never queue behind a diff. The permit is always taken outside `RepoLock`, so
there is no lock-ordering hazard, and two commits queued in the app now run one after the other
instead of the second failing with `Locked`.

## The plugin's poll spawns one process, not two

`vc_docker.py`'s 1.5-second timer used to call both `kvc status` and `kvc branches`, each a
synchronous `subprocess.run` on Krita's GUI thread. On Windows the process spawn dominates the cost,
and the second call was pure duplication: both run `Repo::open_light`, which parses the whole
`commits.log`, and `run_status` already had `repo.branches` in memory. It printed
`branches.current`, and the docker threw it away.

`run_status` now prints the whole branch list too, at no extra I/O, through a `branch_list` helper
it shares with `run_branches` so the two shapes can't drift (`kvc_cli.rs` asserts they match). That
follows the precedent of the `stashes` count, which `status` already carried. The poll reads both
from the one result. `refresh()` also returns early when the docker isn't visible, placed below the
page-selection code so switching documents stays instant. It's the only early exit that needs no
correctness argument; one based on `doc.modified()` would be wrong, because a document becomes
unmodified exactly when a save creates the change the poll has to notice.

See also: [history/07](history/07-cpu-headroom-v1.1.md) for how this came about, and
[performance.md](performance.md#streamed-layers-dont-re-render-everything) for the frontend
rendering fix that shipped with it.
