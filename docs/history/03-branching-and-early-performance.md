# Branching, merging, and the first performance passes

Dates: 2026-07-03 to 2026-07-06. Commits: `433f292` to `998d8c3`.

Local branching and merging land in one commit (`433f292`, "Implemented Branching and Merging").
That is the point where the engine stops being a store of versioned saves and becomes a version
control system.

Three optimization passes follow over the next three days, each aimed at branch-switch, save and
restore latency: `ae0f4c6` ("Minor Performance Optimizations for Branch Switching"), `e07397b`
("Performance Optimizations for saving functions / branch switching / restoration") and `998d8c3`
("Performance & Storage Optimizations for Branch Switching - Also optimizes the rest of the app").
The commits don't say why, but the cadence suggests switch latency showed up as soon as branching
met a real document and wasn't planned for in advance. `e07397b` also adds a loading page and
changes the app's starting window size, which points the same way: switching and restoring were
slow enough to need a "this is working" screen.

`e07397b` is also where garbage collection enters the engine (`src-tauri/src/gc.rs` first appears
here), the mark-and-sweep behind today's "Clean up storage". Branching is what made it necessary.
Once history can fork and a branch can be deleted, stored content can become unreachable from every
branch tip, and something has to be able to reclaim it.

See also: [`version-control.md`](../version-control.md#branches-create-switch-merge) for how
switching works today (it rewrites only the files that differ between the two branches), and
[`performance.md`](../performance.md) for the techniques that grew out of this era.
