# CPU headroom, and v1.1.0

**Timeframe:** 2026-08-01 (a single day) · **Commits:** `e6cae2e`, `c16a5e4`

A single, well-documented root-cause commit. `e6cae2e`'s own message states the problem plainly:

> The engine was tuned purely for throughput: rayon's global pool sized to num_cpus across 17
> parallel sites nested three deep, at normal priority, with no cap on concurrent operations. On a
> 2-4 core laptop a commit or diff pinned every core and starved Krita.

And Krita, the commit goes on to note, was often the very thing that triggered that commit, by way
of the plugin. The irony is the point: the plugin fires a commit from inside Krita's own process
tree mid-paint, so an engine that maxes out every core to finish that commit as fast as possible
ends up fighting the application it exists to serve.

The fix, shipped as v1.1.0 (`c16a5e4`), is a dedicated below-normal-priority worker pool
(`cpu.rs`) sized to a user-configurable share of cores (default 75%), installed once at the single
command-dispatch funnel so every nested `par_iter` inherits it for free. Alongside it, a two-permit
semaphore caps heavy operations (diffs, commits), so rapid history-clicking can't stack unbounded
64 MB decode buffers behind a cancelled-in-the-UI-but-still-running backend call. The `kvc` CLI
gets the same treatment, since the plugin spawns it inside Krita's process tree where the headroom
matters even more than in the desktop app. A few frontend memo dependency arrays were also
narrowed to stop rebuilding multi-megabyte SVG strings on every streamed-layer update for no visual
change. The commit's own measurement: on 4 cores, 75% was not slower than 100%, and 50% cost ~4% on
commit time. Both paths are gated on I/O and serial work as much as raw parallel throughput.

**See also:** [`CLAUDE.md`](../../CLAUDE.md)'s "CPU headroom" section for the mechanism as it works
today; `src-tauri/tests/bench.rs`'s `cpu_budget_sweep` for the ongoing measurement.
