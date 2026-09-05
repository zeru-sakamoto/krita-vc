# Backup & restore overhaul

**Timeframe:** 2026-08-28 – 2026-08-29 · **Commits:** `2d3d59e` … `af73620`

Overlapping with the [Bento redesign](09-the-bento-redesign.md) and following straight on from the
[per-document rewrite](10-version-map-and-the-per-document-rewrite.md), the application tour first
gets Version Map steps with per-step gating on shell state (`2d3d59e`). A fresh install has no
commits and no second branch, so tour steps needed a way to skip themselves when their target
doesn't exist yet. A diff-panel/storage-report/`RETAIN_BUDGET` performance-bug fix follows
(`655b992`).

Then backup and restore are rebuilt for the new per-document world (`af73620` "Add multi-artwork
backup, restore, and a restore version compare"): where the old model backed up one folder-wide
repository at a time, the new one can only make sense of backup as "several independently-stored
artworks," so this commit adds a multi-select backup that writes every chosen artwork's `.kra` +
`.kvc/<slug>/` into a single archive, a restore flow that resolves each artwork back to its
original folder when possible, and a version-compare view showing whether a backup is ahead of or
behind what's already on this computer before you're asked to overwrite it. That last piece is what
makes "replace" a real decision instead of a coin flip.

**See also:** [`data-integrity.md`](../data-integrity.md) for the current backup/restore mechanics
(verified backups, the `MANIFEST.json` layout, `import_zip`'s store-root-aware extraction rules).
