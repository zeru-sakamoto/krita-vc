# Branching, merging, and the first performance passes

**Timeframe:** 2026-07-03 – 2026-07-06 · **Commits:** `433f292` … `998d8c3`

Real local branching and merging land in one commit (`433f292` "Implemented Branching and
Merging"), the feature that turns the engine from "versioned saves" into an actual VCS. What
follows is telling: three separate optimization passes in the next three days, each one explicitly
targeting branch-switch, save, and restore latency (`ae0f4c6` "Minor Performance Optimizations for
Branch Switching," `e07397b` "Performance Optimizations for saving functions / branch switching /
restoration," `998d8c3` "Performance & Storage Optimizations for Branch Switching - Also optimizes
the rest of the app").

That cadence (implement, then optimize, then optimize again within days) reads as branch-switch
latency being discovered the moment the feature met a real document, rather than planned for up
front. `e07397b` also adds a loading page and adjusts the app's base startup size,
suggesting switch/restore operations were slow enough to need a "this is working" indicator rather
than blocking silently.

`e07397b` is also where **garbage collection** enters the engine (`src-tauri/src/gc.rs` first
appears here): the mark-and-sweep behind today's "Clean up storage." Branching is what made it
necessary: once history can fork and a branch can be deleted, stored content can become
unreachable from any tip, and something has to be able to reclaim it.

**See also:** [`version-control.md`](../version-control.md#branches--create-switch-merge) for how branch switching works
today (rewriting only the files that differ between branch trees), and
[`performance.md`](../performance.md) for the performance techniques that followed from this era.
