# The September audit and its fixes

Dates: 2026-09-25 to 2026-09-28. Commits: `38bf63a` to `d3b353a`.

`38bf63a` ("Reorganize the docs by feature and correct them against the code") splits the three
long reference docs (`frontend-architecture.md`, `version-control.md` and `performance.md`) into one
page per feature: the Version Map, the welcome and tour, stashes, backup and restore, layer staging
and CPU headroom. The Tauri command reference moves into `backend-architecture.md`, filled out to
all 39 commands, next to a new reference for the `kvc` CLI. Every doc is brought up to the
one-painting-per-history model from [10](10-version-map-and-the-per-document-rewrite.md), and the
history chapters are checked against `git log`, which corrected dates, commit order and the beta tag
versions. The same commit adds chapter [13](13-welcome-license-and-v2.1.md) and the release-tag
table. `f7d0974` merges it into `main` the same day.

## The audit

With the docs matching the code, v2.1.0 got a performance, optimization and stability audit at
`f7d0974`. It read every Rust module, the CLI, the plugin, the frontend's data hooks and the release
workflow, and measured with a throwaway probe against the engine's public API: the real A4 and A3
test paintings, plus two synthetic long histories (200 versions of a 45,000-tile document, and 100
versions of a 2,500-tile one). The benchmarks in `tests/bench.rs` stop at eleven versions, and most
of what the audit found only shows up past that.

It came back with 18 stability findings and 16 performance ones. The headline was two ways to lose
history silently, both reproduced end to end:

- **One damaged line in `commits.log` shortened history for good.** The log parser stopped at the
  first line it couldn't decode and treated everything after it as a torn append, so the next save
  of any kind rewrote the log without those versions. One bad byte on line 2 of a 400-version log
  would leave one version. The check couldn't see it afterwards, because the rewritten log decoded
  cleanly.
- **"Clean up storage" swept everything behind a gap.** Marking walks back from each branch tip, and
  a tip or parent that the log didn't have simply ended the walk. On a three-version store missing
  its tip, the cleanup moved all 53 objects to the trash and emptied the log.

An interrupted undo was one way into the second state: `save()` rewrote the log before it moved the
branch tip, which is the right order for adding a commit and the wrong one for removing it. The
rest were narrower: object data that a power cut could lose while the references to it survived,
pack consolidation deleting instead of quarantining, a backup taken without the store lock, Restore's
Replace permanently deleting on drives without a Recycle Bin, memory that grew with history length,
a truncated raster-cache entry trusted forever, and a free-space check that asked for twice the
painting's size to save a version of a few MB.

## The stability fixes

`4793bf1` fixes all 18 stability findings except one part of the memory one. Each lands with a test
that fails without it, most of them the audit's reproductions.

**History with a hole in it is refused, not written over.** Only an undecodable *last* line is
treated as a torn append now. A bad line with good ones after it opens the store with every line
that decodes, and every write refuses with a new `KvcError::DamagedHistory` ("version history is
damaged"), checked by `Repo::ensure_writable` before anything touches the artwork and again in
every save. The cleanup refuses the same way when the walk from any tip reaches a version the log
doesn't have, and its dialog points to "Check for problems…" instead of offering to delete. The
check gained `missingParent` and `badChains`, so the state a loss leaves behind is visible. Undo
now moves the tip before truncating the log, and every log rewrite keeps a
`commits.log.<time>.bak`. A chain shard that won't decode is set aside as `.corrupt-<time>`
instead of being overwritten by an empty one.

**Durability matches what the docs said.** Loose objects and packs are fsynced before their rename,
so the chain shard and log line that name them can't outlive them. Pack consolidation quarantines
the packs it merges. Backups take each artwork's `RepoLock` and are written as `<dest>.partial`,
renamed into place only once verified. Replace unpacks the incoming history beside the old one
first, then renames the old history and artwork aside (`<store>.replaced-<time>`,
`<name>.replaced-<time>.kra`), undoing each step if a later one fails, and the Restore dialog says
where they went. Cleanup ages the replaced histories and old log copies out after 14 days, by the
time in their names.

**Smaller things.** The reconstruct memo keeps only the last four patch bases
(`delta::ReconstructMemo`), which bounds the scrub and cleanup's marking pass; the per-layer
full-resolution canvases in layer diffs are still unbounded and left for the performance work.
Raster-cache writes go through a temp file. The Krita docker reopens changed documents even when
the second step of "set aside & switch" fails. Renames wait out a Windows sharing violation for
about a second, and I/O errors carry their path. The free-space check asks for what's actually
written, on the drive it lands on. Tracking and restoring refuse a missing custom store root.
Stale `.kvctmp` files beside the artwork are cleaned up by the scan. And the release workflow gets
a `test` job, so nothing is built or drafted unless the Rust suite, `tsc`, the Version Map check
and the plugin self-check pass.

## The performance fixes

`11354fb` works through all 16 performance findings and the complexity cuts, each measured before and
after with the audit's probe on the same laptop, with the old and new builds run alternately where
single runs were too noisy to tell apart. Most of the gains are in two states the benchmarks never
reached: a painting saved but not yet a version, and a history hundreds of versions long.

**Krita stops paying for the poll.** The docker's 1.5-second `kvc status` read and hashed the whole
painting on every tick while it had saved changes, on Krita's UI thread: 80 ms at 105 MB, 140 ms at
195 MB, and seconds once the file had dropped out of the page cache. The scan now records the hash
of what it read in `worktree.json`, so a save is read once and later polls take about 9 ms, and
the docker skips the spawn entirely while the document and the store's three state files are
unchanged, which is most ticks. `kvc status` stopped reading `commits.log`, the one file that
grows with every version. And a commit from the docker runs through `QProcess`, where it used to
freeze Krita for its three seconds.

**Work that grew with history stops growing.** The storage report replayed every version's
manifest on every refresh, in every view: 27.5 s at 200 versions of a 45,000-tile painting. Each
version now records the bytes it stored as it's committed, the report sums them in 34 ms, and the
frontend only asks while the Performance tab is open. Chain shards are split per layer, so a commit
rewrites the layers it touched and a command decodes only the document's small shard (87 ms to
34 ms for the first lookup at 200 versions); a store in the old layout splits itself on its next
save, tile shards first, so a crash can't strand a chain. Parsed manifests are cached across
commands and manifest patch chains are capped at five, and a Version Map node on that history went
from 1.13 s to 0.35 s.

**Reads get cheaper everywhere.** Packed objects are looked up in the pack index first, through one
open handle per pack, where each read had paid two failed file opens and a fresh open of the pack:
rebuilding every stream of the A4 painting one by one went from 5.9 s to 1.3 s, and its check from
2.5 s to 0.7 s. Layer rasters accumulate straight into their capped size instead of into a
full-resolution canvas, so a cold A3 layer stream holds 128 MB more rather than 459 MB more; each
comes with a 128 px thumbnail for the layer list; a raster-cache hit is a `stat` rather than a
read; and the Changes view parses the working file once per refresh instead of twice. The cheap
reads left the CPU-budgeted pool, where on a 2-core laptop a `list_commits` could wait 1.9 s behind
a diff; it now takes about a millisecond.

**Compressed bytes stop being recompressed.** Layer-subset staging and set-aside merges raw-copy
zip entries instead of inflating and deflating the whole painting again (saving 10 of the A4
painting's 11 layers went from about 3.6 s to 2.4 s), and backups store the `.kra` and the store's
zstd objects instead of deflating them: an A4 backup went from 7.4 s to 0.5 s, for an archive 13%
bigger. Restores (switch, rollback, discard, bringing work back) stream the rebuilt document into
the temp file beside the artwork instead of assembling it in memory: a switch of the A3 painting
peaked at 206 MB instead of 574 MB, and bringing set-aside work back onto an edited A4 painting went
from 15.8 s and 624 MB to about 6 s and 440 MB.

**Complexity cuts.** The readers for every store format older than per-document stores are gone:
the `chains.bin` and `chains.json` monoliths, untagged chain shards, `commits.json`, `KVCP1` packs,
flat loose objects, the v1 config and the missing-`branches.json` migration. v2.0.0 shipped with no
migration from v1, so no store it can open was written in any of them. So are the generic-file
paths (`file:` streams, the standalone-palette diff, and the frontend's `DiffFileBlock`, which only
ever drew an empty list) and helpers that only tests called. The four rarely opened screens
(Settings, Restore, the welcome and the tour) load on first use, which took the startup chunk from
721 kB to 667 kB.

Measuring the fixes turned up two regressions of their own, both fixed before the numbers were
final: backups opened every file twice, which made backing up a store of thousands of small files
slower rather than faster, and bringing set-aside work back built its merged document, bigger now
that entries are copied rather than recompressed, in memory. The costs that stay are written down
with the fixes: the five-patch manifest cap stores a full manifest every sixth version (a synthetic
200-version history went from 37 MB to 79 MB), a small commit pays an fsync for each layer it
touches (59 ms to 63 ms), and backups are 13% bigger. Marking a long history for the cleanup, about
20 s at 200 versions of that painting, didn't get faster, since none of the fixes aimed at it. The
reference docs and `CLAUDE.md` were brought up to the new behavior in the same work, including the
places the audit had found them disagreeing with the code.

A review of the finished work left four issues, and `d3b353a` fixes them, each with a test that fails
without it. One mattered: a release up to v2.1.0 that writes to a store the new code has split looks
for every chain in the document shard, so it records its tile chains there, and when the new code
split the store again it kept the tile shards' copies and dropped those records. The versions the
older release committed then couldn't be rebuilt, and a cleanup would sweep their tiles. The two
copies are merged now instead of one being preferred. The other three were minor: a parsed
painting, about its file size, could stay in memory between visits to the Changes view, and now
goes after ten seconds; a raster-cache hit could race a prune and show a blank image, and is a miss
now; and two unreachable standalone-palette views in the frontend are gone.

See also: [`data-integrity.md`](../data-integrity.md) for the rules these fixes added,
[`backup-and-restore.md`](../backup-and-restore.md) for how Replace works now, and
[`performance.md`](../performance.md) and [`cpu-headroom.md`](../cpu-headroom.md) for the
performance work as it stands.
