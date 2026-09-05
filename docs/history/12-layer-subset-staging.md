# Layer-subset staging

**Timeframe:** 2026-09-01 – 2026-09-03 (most recent era at time of writing) · **Commits:**
`c686231` … `e5b02ea`

Refresh-trigger improvements for file-change detection (`c686231`) precede the newest major
feature: **layer-subset staging**, committing only the ticked top-level layers of a `.kra` instead
of the whole document (`95df784`). The implementation synthesizes a partial commit: the working
file with every *unticked* top-level layer reverted to its last-committed form, with
`mergedimage.png`/`preview.png` dropped from the synthesized archive since they'd render layers the
version doesn't actually contain. A new `layers` argument threads through `commit_snapshot`/
`commit_selected` and the `kvc commit --layers` CLI flag; the index marks a partial commit so a
scan keeps reporting it dirty instead of the unticked layers silently vanishing on the next poll;
and `stacked_composite_url` renders a stand-in composite for the Version Map, since a layer-subset
version has no `mergedimage.png` of its own.

Two days later, the same feature gets a **3.3x speedup and a full documentation audit**
(`020d55f`). The headline fix: a partial commit had been reconstructing the *entire* committed
document just to discard nearly all of it (235 MB rebuilt to keep a couple of layers); rebuilding
only the entries the partial commit actually reads cut that to 39 MB and 5.4x faster. The same
commit also replaced an earlier "zero out size/mtime to force a dirty scan" trick with an explicit
`TrackedFile.partial` flag. The zeroing approach worked, but made *every* later scan re-read and
blake3-hash the whole painting, which mattered because `kvc status` runs on the Krita docker's
1.5-second poll (145 ms → 0.1 ms per scan after the fix). Alongside the performance work, the
commit audited both `CLAUDE.md` and the `docs/` files against the actual code and corrected roughly
a dozen drifted details (a stale Settings tab count, an undocumented Performance tab and two
undocumented backend modules, a wrong TopBar pixel measurement, a deleted-but-still-specified
docker tab strip, and more). This is the kind of doc rot that accumulates silently until someone
reads the code side by side with the docs it is supposed to describe.

A same-day follow-up (`e5b02ea`) fixes a bug where the Changes panel was reporting every
non-new layer as modified regardless of whether it actually changed.

**See also:** [`version-control.md`](../version-control.md#layer-subset-staging--saving-only-some-layers)
for the current mechanics (`stage.rs`, `committed_subset`, `plan_pieces`);
[`performance.md`](../performance.md#layer-subset-staging) for the measured numbers.
