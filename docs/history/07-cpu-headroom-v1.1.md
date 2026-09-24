# CPU headroom, and v1.1.0

Dates: 2026-08-01, a single day. Commits: `e6cae2e`, `c16a5e4`.

One commit with a well-documented root cause. The opening of `e6cae2e`'s message states the
problem:

> The engine was tuned purely for throughput: rayon's global pool sized to num_cpus across 17
> parallel sites nested three deep, at normal priority, with no cap on concurrent operations. On a
> 2-4 core laptop a commit or diff pinned every core and starved Krita

The message goes on to say that Krita was often the very thing that triggered the commit, through
the plugin, and that is the whole story. The plugin runs a commit from inside Krita's own process
tree while the artist is painting, so an engine that takes every core to finish the commit sooner
ends up fighting the program it exists to help.

The fix is its own worker pool (`cpu.rs`). Its threads start at below-normal priority, and it is
sized to a share of the cores the user can set, 75% by default. It is installed once, at the single
funnel every command goes through, so every nested `par_iter` inherits it. Next to it, a semaphore
with two permits caps heavy operations (diffs and commits): cancelling a diff in the UI never
cancelled the backend, so clicking quickly through history used to stack up 64 MB decode buffers
with no limit. The `kvc` CLI lowers its whole process and uses the same pool, because the plugin
starts it inside Krita's process tree, where the headroom matters even more than in the desktop
app. The plugin's 1.5-second poll also drops from two processes per tick to one, since `kvc status`
now returns the branch list too. Three frontend memo dependency lists were narrowed as well, so
streamed layers stop rebuilding multi-megabyte SVG strings for no visible change.

The commit measures its own trade-off: on 4 cores, 75% was not slower than 100%, and 50% cost about
4% on commit time, because both paths wait on I/O and serial work as much as on parallel
throughput. `c16a5e4` bumps the version to 1.1.0.

The setting first appears as a dropdown under Settings → Storage → "Background CPU use" (Gentle,
Balanced, Full speed). During the redesign it moves to a new Performance tab in Settings (`1fcc980`)
and becomes a slider (`7674a0d`); both are in [09](09-the-bento-redesign.md).

See also: [`cpu-headroom.md`](../cpu-headroom.md) for how the mechanism works today, and
`cpu_budget_sweep` in `src-tauri/tests/bench.rs` for the measurement you can rerun.
