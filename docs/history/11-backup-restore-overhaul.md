# Backup and restore overhaul

Dates: 2026-08-28 to 2026-08-29. Commits: `655b992` to `af73620`.

This era overlaps the [Bento redesign](09-the-bento-redesign.md) and follows straight on from the
[per-document rewrite](10-version-map-and-the-per-document-rewrite.md). It opens with two smaller
fixes.

`655b992` implements `scan::RETAIN_BUDGET`, the cap on how many bytes a scan keeps in memory for
the commit that follows it. The docs had described that cap for a while, but it didn't exist. The
same commit speeds up a lookup in the storage report, removes a duplicated heavy `working_diff`
call, stops the minimap recomputing its bounds every frame, and adds a `corpus_baseline` benchmark
that runs against real Krita documents.

`2d3d59e` fixes the first-launch tour for the new default layout. With History and Branches hidden
behind the Legacy toggle, seven tour steps in a row pointed at elements that weren't on screen, and
a step with no target made the tour overlay render nothing at all: no card, no Next, no Skip. Steps
can now depend on what the shell is showing (legacy tabs on or off, whether there are versions,
another branch, or unsaved changes), and the commit adds tour steps for the Version Map.

Then `af73620` ("Add multi-artwork backup, restore, and a restore version compare") rebuilds backup
and restore for the per-document model. The old backup zipped one folder-wide repository at a time.
With every artwork now stored on its own, backup becomes a multi-select that writes each chosen
artwork's `.kra` and its `.kvc/<slug>/` store into a single archive. Restore puts each artwork back
in its original folder when that folder still exists. When the destination already holds a tracked
artwork, a version comparison shows whether the backup is ahead of it or behind it before you
decide to replace anything. Without that comparison, choosing Replace would be a guess.

See also: [`backup-and-restore.md`](../backup-and-restore.md) for how backup and restore work
today, including the `MANIFEST.json` layout and why restoring re-derives where history goes on the
machine you restore to.
