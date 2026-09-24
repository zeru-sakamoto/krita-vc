# Performance

Why the `.kra` diff path is fast, and the specific techniques behind it. This complements
[version-control.md](version-control.md) and [visual-diff-viewer.md](visual-diff-viewer.md), which
describe the same code from an architecture angle; this page is the "why is it fast" index. How the
engine avoids starving Krita while it works, the other side of the same coin, is in
[cpu-headroom.md](cpu-headroom.md). The Performance tab's own metrics are in
[performance-report.md](performance-report.md).

The main cost center is the visual diff. A `.kra` can be a large tiled document, and naively
rebuilding every layer at full resolution each time a version is opened would be slow enough to feel
broken. Everything below exists to keep that work off the UI's critical path, or to skip it
entirely.

## Diff loading: two stages instead of one blocking call

`commit_diff` and `working_diff` return the composite (`mergedimage.png`) and layer metadata only,
with no per-layer pixels, so the panel renders immediately. The per-layer rasters, the expensive
part, are fetched afterwards by `commit_layers` and `working_layers`, called lazily from the
frontend (`useArtLayers` in [`src/lib/repoData.ts`](../src/lib/repoData.ts)) once the fast diff has
painted. See the `with_rasters` flag on `commands.rs`'s `art_diff_dto`.

## Streaming over a Tauri `Channel`

`commit_layers` and `working_layers` don't return a `Vec<LayerDto>`. They take a
`Channel<LayerDto>` and send each layer the moment it's rasterized. Layers finish out of order
(rayon), so the frontend merges them by layer id as messages arrive (`useArtLayers`' `onmessage`)
instead of waiting for the slowest layer to hold up the others.

## Parallelism (rayon)

Independent per-layer and per-tile work is spread across cores instead of running one after another.
All of it runs inside the engine's own budgeted pool (see [cpu-headroom.md](cpu-headroom.md)).

- **Layer rasterization.** `art_diff_dto` rasterizes all of a document's layers with `par_iter()`
  (`commands.rs`), keeping the order through an indexed collect.
- **Preparing a whole commit, in chunks.** `commit_kra` (`kra.rs`) walks the zip serially (the
  reader is inherently serial, and unchanged entries are skipped by crc32 and size), collecting
  inflated entries into chunks bounded by `RESTORE_CHUNK_BUDGET` (64 MB uncompressed). For each
  chunk it runs a rayon pass over the changed entries (every layer's tiles, and raw entries like
  `mergedimage.png`) through `prepare_stream`, the CPU-heavy reconstruct, bsdiff, verify and zstd
  step, then one serial fold (`flush_entry_chunk` → `commit_prepared_batch`), and drops the buffers
  before reading the next chunk. A multi-layer edit costs about as much as its largest layer instead
  of the sum of all of them. Each stream key appears once per commit, so the parallel prepare can't
  race; only the fold needs `&mut`. Peak memory is one chunk, not the whole decompressed document.
  (A first commit or a big edit used to inflate every changed entry at once, the mirror image of the
  restore-side chunking below.)
- **Rebuilding a `.kra`, in chunks.** `reconstruct_kra` (`kra.rs`) resolves the manifest entries'
  bytes, replaying tile chains as needed, with `par_iter()` in chunks bounded by the same 64 MB
  budget, writing each chunk to the zip before the next one is built. Every decompressed entry and
  the whole output zip used to sit in memory together (about twice the document's size, a paging risk
  on a 4 GB machine); now the peak is the output plus one chunk. `materialize_kra`'s full rebuilds are
  chunked the same way, although switching and rolling back normally take its cheaper incremental
  path and only fall back to this.
- **The commit's dedup filter.** `commit_prepared_batch` (`delta.rs`) checks in parallel whether
  each candidate object already exists, cheapest check first (the in-memory pack index snapshot,
  then the sharded loose path, then the legacy flat path). Thousands of serial `stat` calls per large
  commit hurt on a cold hard drive. The pack index is handed out as an `Arc` snapshot, so parallel
  lookups never hold its mutex.
- **Raster downscaling.** `box_downscale` (`raster.rs`) runs in parallel over destination rows with
  integer accumulation (it used to be serial `f64` work per source pixel over the full-resolution
  canvas). The area-average, premultiplied behavior is the same, so the `box1` cache token stays
  valid, and a unit test pins the integer version to the `f64` reference within ±1.
- **Object writes on commit.** `commit_prepared_batch` writes all of a layer's new tile objects in
  parallel before the serial chain fold. Content-addressed writes are independent and idempotent,
  and thousands of small file creates in a row (NTFS plus Defender) were a dominant commit cost on
  Windows.
- **Decoding tiles within one layer.** `layer_raster` reconstructs and LZF-decodes each tile in
  parallel, then blits them serially onto the shared canvas. Nested rayon is fine here; it's one
  work-stealing pool.
- **blake3 hashing.** `hash_bytes` (`repo.rs`) uses blake3's rayon-parallel `update_rayon` for
  buffers of 1 MB or more (whole `.kra` files during a scan or commit). Small buffers such as tiles
  stay on the cheap single-threaded path, because spinning up parallel hashing for a few KB is pure
  overhead.

The `prepare_stream` and `commit_prepared` split in `delta.rs` is what all of this relies on: the
read-only preparation (`&self`) can run in parallel across streams, and only the serial fold
(`&mut self`) touches shared state.

## Rendering streamed layers

### Streamed layers don't re-render everything

Each arriving layer sets `{ layers: new Map(received) }`. The `ArtLayer` objects inside were already
identity-stable; the churn came from further down, where three memos that should have held didn't.
Each arrival cost three full SVG rebuilds plus three `dangerouslySetInnerHTML` subtree replacements
(a re-parse and a re-decode of the base64 rasters). It was worst in the Composite view, the default,
where both canvases render `mergedimage.png`, which never changes while layers stream, and rebuilt it
anyway.

Narrowing three dependency lists fixed it, with no change in behavior and no added latency.
`ArtDiffView` lifts the composite layer into its own memo keyed on `diff.beforeImage` and
`diff.afterImage` (memoizing the one-element array, not just the object, since the outer memo still
re-runs and a fresh `[composite]` would defeat the point). `ArtCanvas`'s `compositeSvg` depends on
`diff.path`, `width` and `height` instead of the whole `diff`. And `LayerStackPanel` splits
`compositeThumb` so the branch that runs doesn't depend on `diff.layers`. The Composite view now
rebuilds nothing while layers stream. These dependency lists are deliberately narrower than the
values the memos close over; don't "fix" them back to exhaustive lists.

`overlay` and `pendingIds` still recompute on every flush and are left alone on purpose: their
results are consumed as stable field references and booleans, so the miss costs one allocation and
nothing downstream. Throttling the flush to a fixed interval was also considered and rejected. Once
the dependencies hold, it buys nothing and makes layers appear in visibly bigger jumps.

## Skipping work entirely

- **The scanner's fast path** (`scan.rs`). A tracked file whose size and mtime still match the index
  (`TrackedFile.size` and `mtime`, `repo::size_mtime`, at nanosecond resolution), and whose mtime is
  older than the index file's own, is assumed unchanged and never read or hashed. Big `.kra` files are
  the case this matters for.
- **Including when the answer is "dirty"** (`TrackedFile.partial`). After a
  [layer-subset commit](#layer-subset-staging), the stored version isn't what sits on disk, so the
  painting has to keep reading as modified. The index says so with a flag, and the scanner reports
  `"M"` from the same size and mtime match instead of reading and hashing the document. The first
  design zeroed `size` and `mtime` to fail the fast path outright. That was correct, and it cost a
  full read on every scan afterwards; `kvc status` runs on the Krita docker's 1.5-second poll, so one
  partial commit meant reading the whole painting twice a second, forever. Callers that want the
  bytes (`keep_bytes`, the commit path) still read.
- **One `stat` per scan.** A store tracks a single document, so there's no directory to walk:
  `scan_detailed` stats one path. `scan::is_supported` (`.kra` only, with Krita's `-autosave.kra`
  artifact rejected on a lowercased suffix check) now gates `Repo::init` instead of a walk, so a
  folder full of other files costs nothing to scan.
- **Handing bytes and hashes from the scan to the commit** (`scan::scan_detailed`). The scan
  already read and blake3-hashed every changed file. `commit_snapshot` reuses the hash, size and
  mtime, and the bytes themselves (`ScanChange.bytes`, kept within a 512 MB cumulative
  `RETAIN_BUDGET`), so a big `.kra` is read exactly once per commit. The old re-read counted on the
  page cache, which on a 4 GB machine had often already evicted the file, so it was a full extra pass
  over a hard drive. (`scan()` passes `keep_bytes = false`, so status-only paths never hold buffers.)
- **Undo without reconstruction** (`CommittedFile.fileHash`). Every commit records the blake3 of
  each file as it sat on disk, so `undo_last_commit` rewinds the index from that hash instead of
  rebuilding a whole `.kra` from the store just to hash it. Records from before the field existed
  still take the rebuild fallback. Restores get the same treatment: `bytes_of` and `restore_bytes`
  return the hash with the bytes (for a generic blob it is the stream hash, so there's nothing extra
  to compute).
- **A single-pass tile diff** (`kra::diff_tile_indexes` over borrowed `TileIndexRef`s). The set of
  changed layers and the union change region come out of one pass that builds each entry's old
  `(x, y) → hash` map once. Two functions used to rebuild the maps separately, and the owned
  `tile_index()` cloned every 64-character tile hash (megabytes of string churn on a Krita-scale
  document).
- **Rollback without a re-commit** (`commit::rollback_to_commit`). A rollback used to write out the
  target tree and then run a full `commit_snapshot` (rescan, re-read, and re-decompose every restored
  `.kra`) just to rediscover content hashes already recorded in the target tree. The commit is now
  built directly from the tree diff between the target and the current tree: no scan, no `.kra`
  decomposition and no object writes, which roughly halved rollback time.
- **The delta-chain heuristic** (`delta.rs::looks_compressed` plus a 64 KB floor). bsdiff is skipped
  for small streams (a chain-walk reconstruct and a suffix sort to save a few KB isn't worth it) and
  for already-compressed payloads (PNG, zip or zstd magic; a patch against compressed bytes comes out
  near full size). Both go straight to a single zstd snapshot.
- **Manifest reuse.** `kra::load_manifest` rebuilds and parses a `.kra` manifest once per diff
  request, and every layer, region and composite read reuses the parsed struct instead of walking the
  patch chain again.
- **GC's manifest memo.** Mark-and-sweep loads every reachable commit's `.kra` manifest to walk the
  streams it references. Plain `reconstruct` replays each manifest version's patch chain from the
  nearest full snapshot independently, redoing the shared prefix every time, which is quadratic in a
  file's history length. GC threads one content-hash memo (`Repo::reconstruct_cached` through
  `kra::load_manifest_memo`) through the marking loop, so each version is built from its immediate
  predecessor exactly once: linear patch applications instead of quadratic, which turns marking a
  long history from seconds into milliseconds. The memo is keyed by a pure content hash, so it dedups
  safely across paths.
- **The crc32 and size skip at commit time.** `commit_kra` compares each zip entry's crc32 and
  uncompressed size (from the central directory, with no inflating) against the previous commit's
  manifest for that path (which `commit_snapshot` passes in). A match reuses the old manifest entry
  as-is, so an edit to one layer doesn't re-inflate or re-store every other entry in the archive.
- **The `layer_diff` command** pulls only `maindoc.xml` out of each side's manifest instead of
  rebuilding the whole archive for one small entry.
- **`TileCache`** (`delta.rs`). A request-scoped cache keyed by content hash. The before and after
  sides of a modified layer usually share most tiles, so each shared tile is rebuilt once, not twice.
- **Reusing an unchanged layer's raster.** `art_diff_dto` copies the `after` raster into `before`
  for layers marked `unchanged` instead of decoding and encoding identical pixels twice.
- **The per-layer change highlight rides the layer stream.** A modified layer's own mask, outline and
  region (`layer_diff_overlay` → `raster::diff_overlay_full`) is diffed from the before and after
  capped PNGs the raster path already produced (`kra::LayerRaster` returns the PNG bytes and cache key
  alongside the URL). It adds one capped-resolution pixel compare and an outline trace of about 200 px
  per modified layer, which is negligible next to the tile rebuild and PNG encode already paid, and it
  runs inside the same rayon `par_iter`. The mask PNG is cached content-addressed by both layer
  raster keys (`kra::diff_cache_key`), so a repeat view skips the diff.
- **Palette diffs stay off the raster machinery** (`palette.rs`, `commands::palette_dto`). Palettes
  are kilobytes and their swatch diff is linear in the swatches, so unlike `.kra` rasters there's
  deliberately no two-stage load, no streaming and no `cache/` entry. The diff is parsed and computed
  inline inside `commit_diff` and `working_diff` (already on the blocking pool); a cache would cost
  more than it saves. Embedded palettes are stored as ordinary `.kra` archive entries, so they
  version and dedup with the rest of the document.
- **Working-tree diffs never touch the store.** `parse_working` and `WorkingKra` (`kra.rs`) decode a
  working `.kra` straight from an in-memory `ZipArchive`: no bsdiff, no chain rebuild, no object
  writes. Older code staged the working file into the object store just to reuse the rasterizer, and
  viewing a diff should never write. The opt-in `lowMemoryDiff` flag trades a little CPU for bounded
  memory here: instead of holding the whole decompressed document, `WorkingKra` keeps only the
  compressed archive plus per-entry metadata and re-inflates each entry on demand, so peak memory is
  the compressed document plus one decoded entry. It's off by default because the in-memory path is
  faster for interactive diffs, and change detection is identical either way.
- **`Repo::open_light`** skips the chains entirely (even the legacy monolith parse a store from
  before sharding would pay) for read paths that never touch storage (`scan_repository`,
  `list_commits`). With sharded chains a full `Repo::open` is nearly as cheap, because shards load on
  first touch, but the light open keeps the rule explicit.
- **Incremental `.kra` writes on switch, merge and rollback.** `kra::materialize_kra` builds the
  target version out of the working file. Entries identical between the current and target
  manifests are copied raw (`raw_copy_file`) from the zip on disk, with no store reads and no
  inflating or deflating, after each one is checked against the manifest's recorded crc32 and size. A
  changed tiled entry lifts its unchanged tiles from the working copy in memory, and only tiles whose
  content differs are replayed from the object store. Switch cost follows the difference between the
  branches, not the document's size. Any mismatch or error falls back to a full `reconstruct_kra`.
- **Chains skipped when clean.** `Repo::save` only rewrites the chain shards a commit actually
  changed (`ChainStore` tracks dirty shards). Switch, merge and undo only change the index, commits
  and branches, so they never pay for a chains rewrite.
- **No integrity re-hash on the read path.** `reconstruct` doesn't re-hash the bytes it rebuilds by
  default: every patch is already verified by a round trip when it's written (`prepare_stream`), and
  objects are content-addressed, so a second hash on every read would be pure redundancy on the
  diff's hottest loop. The restore paths turn it back on with `Repo::verify_reads`, because there a
  bad byte would end up in the artist's file (see [data-integrity.md](data-integrity.md)).

## Layer-subset staging

Saving only some of a document's layers ([layer-staging.md](layer-staging.md)) has to express "the
document minus these layers" as a real version. The first implementation did that by rebuilding the
committed `.kra` in full, repacking the whole working document around it, and handing the result to
`commit_kra`. So the operation that sounded cheaper was 14 times the cost of saving everything, and
it left the painting in a state that re-read itself on every scan afterwards.
`tests/bench.rs::partial_commit_baseline` exists because none of that had been measured.

The fixture is 6 layers × 2,500 tiles (3,200 × 3,200 px, about 235 MB of incompressible tiles), with
two layers edited and one of them held back. That's the shape the UI produces, since `ChangesPanel`
ticks everything and tracks what the artist turns off. Two runs each, release build, 4-core Windows.

| | before | after |
| --- | --- | --- |
| **partial commit, end to end** | 13.27 / 13.34 s | **4.01 / 4.26 s** |
| rebuilding the committed side | 5.02 / 4.32 s (234.9 MB) | 0.65 / 0.73 s (39.1 MB) |
| blake3 of it, then discarded | 35 ms | none |
| `stage_kra` (repack) | 8.52 / 8.50 s | 2.29 / 2.38 s |
| `commit_kra` | 0.23 s | 0.35 / 0.45 s |
| **scan after a partial commit** | 145 / 156 ms | **0.08 / 0.16 ms** |
| whole-painting commit (control) | 0.54 / 0.60 s | 0.56 s |
| store size | 238.2 MB | 238.2 MB |

Four changes, in order of what they bought:

- **Rebuild only the entries staging reads** (`stage::committed_subset` →
  `kra::reconstruct_kra_from` with an entry filter). A partial commit reads `maindoc.xml` plus the
  data files of the layers being reverted, typically one or two out of a stack. It had been replaying
  every tile of the whole document through its delta chains and re-encoding `mergedimage.png` from
  its pixel blocks, then throwing nearly all of it away. Now it rebuilds `maindoc.xml` alone, plans
  the stack against it (`stage::plan_pieces`, shared with `stage_kra` so the two can't disagree), and
  rebuilds only what the plan names: 39 MB instead of 235, 5.4 times faster. The manifest is loaded
  once and both passes go through `reconstruct_kra_from`, since loading a manifest is a chain replay
  of its own.
- **Stop compressing a buffer nobody reads** (`stage::out_opts`). `repackage` wrote the
  synthesized archive through `merge::opts`, which is `SimpleFileOptions::default()`, deflate level
  6, over every entry of the document. That buffer is never written to disk: `commit_kra` reopens it
  immediately, and its reuse check compares crc32 and size, computed over uncompressed bytes, so the
  level can't affect what's stored. Level 1 (the same conclusion `kra::opts` reached for restores,
  for the same reason: Krita's tiles are already LZF-compressed) is 3.5 times faster and, as the
  table shows, stores exactly the same amount.
- **Answer "still dirty" from a `stat`** (`TrackedFile.partial`; see
  [Skipping work entirely](#skipping-work-entirely)). This is the one an artist feels: it had been a
  full read and blake3 of the painting on every scan, and `kvc status` runs on the Krita docker's
  1.5-second poll.
- **Stop hashing what gets thrown away** (`commit::bytes_of` versus `bytes_and_hash_of`).
  `bytes_of` returned a blake3 of the whole rebuilt document, and three of its five callers discarded
  it.

Still on the table, and now the largest term: `stage_kra` repacks the entire working document (2.3 s
of the 4.0 s) to change one layer's worth of it, and `commit_kra` then decomposes that archive again.
The upgrade is manifest-level splicing: commit the surviving working entries as usual and substitute
the previous manifest's `KraEntry` values for the reverted ones, so no document is ever built. That
would also retire the roughly three-times-the-document peak memory noted under
[Ceilings and deferred work](#ceilings-and-deferred-work). It isn't built, because the measured gap
no longer justifies how much it would touch. A plain-language write-up of this audit is kept with
the site copy, in the local (gitignored) `content/PERFORMANCE_AUDIT.md`.

## Output size and encode cost

- **Raster downscaling** (`raster.rs::MAX_RASTER_DIM = 2048`). Both per-layer rasters (`cap_rgba`)
  and the composite (`cap_png`, which decodes an already-encoded PNG just to cap it) are capped to a
  2048 px longest side before encoding. A diff preview never needs full document resolution (Krita
  canvases run to thousands of pixels), and full resolution was the dominant cost in both encode time
  and the IPC payload sent to the webview. The downscale is an area-average box filter
  (`box_downscale`, in premultiplied alpha): one extra pass over the source compared with the old
  nearest-neighbor, negligible next to the PNG encode, and crisp now that the viewer can zoom. The
  filter is versioned in the cache keys (the `box1` token), so changing it invalidates cleanly.
- **Fast PNG encoding** (`raster::rgba_to_png`): `Compression::Fast` with `FilterType::NoFilter`.
  These PNGs are cached previews read once by the webview, so encode speed matters and byte size
  doesn't.
- **The changed-pixel diff runs at capped resolution** (`raster::changed_grid`). The mask caps each
  composite to `MAX_RASTER_DIM` right after decoding, before comparing pixels. Holding two
  full-resolution RGBA composites at once was a transient spike of 2 × (w · h · 4) bytes that stacked
  with layer streaming, and the mask is capped to the same bound afterwards anyway, so the output is
  the same while peak memory roughly halves.
- **`blit`'s row-copy fast path** (`raster.rs`). When a tile sits fully inside the canvas (the
  common case), each row is one `copy_from_slice` instead of a per-pixel loop with bounds checks.
- **No recompressing of compressed zip entries.** `reconstruct_kra` stores, rather than deflates,
  any entry whose bytes already look compressed (`delta::looks_compressed`), since deflating a PNG,
  zstd or zip payload a second time buys nothing.
- **Restored files are written with fast deflate** (`kra.rs::opts`). Rebuilt tile blocks and other
  uncompressed entries use the fastest deflate level instead of being stored. Krita deflates layer
  entries itself, and writing them uncompressed left restored files several times larger on disk,
  which then slowed every later scan, hash and switch (worst on hard drives).
- **The zstd level depends on the payload** (`delta.rs::prepare_stream`). Object snapshots use zstd
  level 1 for tile streams and anything that looks already compressed (Krita's tiles are already
  LZF; level 3 over them bought almost nothing while being the largest single CPU cost of a
  whole-document commit), and keep level 3 only for diff-friendly text like the JSON manifests.

## Raster delivery (`kvcimg` URI scheme)

Cached diff rasters used to travel as base64 data URLs. Every layer view, even a full disk-cache hit,
paid for a file read, a base64 re-encode (33% larger), a multi-megabyte string over Tauri IPC, and
the V8 heap holding it in the frontend caches. The desktop shell now registers a `kvcimg` URI scheme
(in `lib.rs`, handled by `commands::serve_raster`), and `raster::raster_url` emits plain URLs whose
path is `/<hex store path>/<key>.png`, which the webview fetches straight from the store's `cache/`
(on Windows the webview sees the scheme as `http://kvcimg.localhost`, which is why the CSP lists
both forms). There's no base64 and no IPC payload, and since keys are content-addressed and never
change, the response carries `Cache-Control: immutable`, so repeat views are browser-cache hits with
no backend work at all. The handler serves nothing but `<store>/cache/<hex key>.png`, and only for
stores a diff command has registered (`register_served_repo`, which takes the store path and is only
called after that store's `Repo::open` succeeded, so a failed open never joins the allowlist). It
can't be pointed at an arbitrary path. Outside the shell (tests, or a failed cache write) everything
falls back to data URLs, which always work. The frontend's session caches now hold short URL strings
instead of multi-megabyte base64 payloads.

## Storage: composite tiling (`kra.rs::CompositePng`)

The largest storage cost used to be invisible. Every `.kra` embeds a full-canvas `mergedimage.png`,
it changes whenever any visible pixel changes, and PNG trips the `looks_compressed` check, so nearly
every commit permanently added the whole multi-megabyte composite as a new object. Composites that
qualify (8-bit RGB or RGBA with no ICC profile; an sRGB chunk is recorded and written back) are now
decoded once per commit and stored as 256 px raw-pixel blocks (`COMPOSITE_TILE`), content-addressed
like layer tiles. Unchanged regions of the canvas dedup across commits, and a changed block, at
256 KB (above the patch floor), bsdiffs against its previous version at the same position. Restores
reassemble the blocks and re-encode a valid PNG (with fixed, fast settings): the pixels are exact,
but the bytes aren't identical to Krita's original encoding. Anything that doesn't qualify keeps the
byte-exact `Raw` path, `preview.png` stays `Raw` on purpose (it's tens of KB), and old manifests
keep rebuilding unchanged, because the manifest describes each entry itself.

## Storage: opt-in tile pixel deltas (`Config.tilePixelDeltas`)

A `config.json` flag, off by default, shown in Settings → Storage as "Compact storage for
heavily-revised art". When it's on, new commits store each tile's decoded planar pixels, which bsdiff
across versions (`patch_floor: 0` bypasses the 64 KB floor for them), instead of Krita's opaque LZF
payload, and restores re-encode LZF (`raster::lzf_compress`, a hand-written liblzf encoder; any valid
LZF stream decodes the same, so byte parity with Krita isn't needed). It shrinks heavily revised
layers 2 to 10 times (measure it with `tile_storage_experiment` in `tests/bench.rs`), at the cost of
LZF decoding and encoding on the commit and restore paths, which is why it's opt-in on low-end
hardware. The `raw` flag lives on each tile reference in the manifest, so mixed histories are fine
and turning the flag off never breaks commits made while it was on.

## Storage reclamation (`gc.rs`)

Nothing on the hot path ever deletes stored data (undo and branch delete orphan objects by design,
and content-addressed orphans are harmless), so a long-lived store only grows. The "Clean up
storage" action (`cleanup_repository`, the mark-and-sweep in `gc.rs`) reclaims everything
unreachable from any branch tip or any stash. Stashes are rooted explicitly, because nothing in the
commit log refers to them. Unreachable commits leave the log, dead chain versions leave their shards,
and dead loose objects and dead or rewritten packs are quarantined to `<store>/trash/<timestamp>/`
(a rename on the same volume, the same cost class as the delete it replaces) rather than deleted, so
a wrong reachability call, or a cleanup right after a branch delete, stays recoverable by hand.

A partly dead pack is rewritten with only its survivors, but only when more than 25% of it is dead
(`worth_rewriting`): rewriting re-reads every survivor, so reclaiming a few KB from a big pack would
cost more I/O than it frees. Dead bytes that stay are left out of the report, and the old pack is then
quarantined like a fully dead one. Patch bases are closed over, so a live patch keeps its whole chain
back to the full snapshot. State files are rewritten before any object is quarantined, so a crash in
the middle of a sweep leaves only orphans that the next cleanup collects, never a dangling
reference. Quarantined runs older than 14 days are deleted for good on the next real cleanup (never
on a dry run) and reported separately as `trashBytesPruned`: bounded retention, not unbounded growth.
A dry-run mode powers the confirm dialog ("about N MB can be freed") and never touches the trash.

GC also handles three things reachability can't see. The raster cache is pruned to its budget
unconditionally (and wiped whole when its `.filter-version` marker doesn't match
`raster::FILTER_VERSION`; the token is hashed into every key, so stale entries can't be told apart
one by one), reported separately as `cacheBytesReclaimed`. Stale `*.tmp` files (crash leftovers from
atomic writes, older than an hour) are swept from the store, `chains/` and `objects/pack/`. And
small live packs are consolidated: eight or more packs under 4 MB merge into one, because every pack
header is parsed when the index loads, and dozens of small packs from mid-sized commits add up.

## Caching across requests

- **A content-addressed disk cache** (`<store>/cache/`, `raster::cache_read` and `cache_write`).
  Every capped PNG, composite or per-layer, is keyed by a hash of everything that determines its
  pixels (tile positions and hashes, the dimensions and the resolution cap, or the composite entry's
  content hash). Keys never need invalidating, unchanged layers share one entry across commits and
  across the committed and working diff paths, and a repeat view, even after an app restart, skips
  rebuilding, decoding and encoding entirely.
- **Frontend session caches** (`repoData.ts`). `diffCache` (commit-diff results) and `layerCache`
  (streamed layer sets) are small LRU maps (up to 20 entries) keyed by the request. Committed entries
  key on `path|commitId` only, because a commit never changes, so a write (commit, rollback, undo)
  doesn't cold-start every diff viewed before it; only the working layer key includes the refresh
  nonce, since the working copy really does change. Cancelled or partial layer requests are never
  cached, because a torn-down effect's `received` map may be incomplete and caching it would poison
  the key for later visits.
- **A bounded raster cache** (`raster::cache_prune`). `cache/` used to grow for the life of the
  store. It now has a size budget (`Config.cacheMaxBytes`, default 256 MB; a config v1 to v2
  migration lowers old 512 MB defaults), which Settings exposes as "Preview cache size" (128 MB to
  2 GB). Reads touch an entry's mtime so hot entries survive, an oldest-first prune runs after layer
  streaming (rate-limited by a marker file, `cache_prune_throttled`), and "Clean up storage" prunes
  unconditionally. A pruned entry is regenerated when needed, never an error.

## State-file writes

- **Compact JSON** (`repo.rs::write_json`). The store's JSON files are machine state, not something
  people read, so they use `serde_json::to_vec` instead of `to_vec_pretty`.
- **An append-only commit log** (`repo.rs::flush_commits`). `commits.json` used to be re-serialized
  and rewritten in full on every commit, growing with the total history (the same class of cost the
  chains sharding removed). History now lives in `<store>/commits.log` as JSON lines: a normal commit
  is one append, and only undo and GC, which truncate history, rewrite it. `branches.json` is written
  after the log, so a torn append is always an unreachable orphan record, never a dangling branch tip,
  and reads drop a torn last line that the next save cleans up. Legacy `commits.json` stores migrate
  on first save (the old file is then retired), following the chains pattern.
- **Slim chain versions (`KVCC2`)** (`repo.rs::Version::object_name`). Each chain version used to
  store its object filename, which can be derived from `hash` and `base`, duplicating a 64-character
  hash per version forever. Bincode isn't self-describing, so the fix rides on an explicit format tag:
  `KVCC2`-prefixed shards hold the slim shape, and bare-zstd files are older and decode through a
  legacy struct, upgrading the next time they change (or all at once in GC's `rewrite_all`). Old
  monolithic `chains.bin` and `chains.json` files decode through the same dual path.
- **Per-file chain shards, loaded lazily** (`repo.rs::ChainStore`). The chains store (every version
  of every delta stream) used to be one file, rewritten in full on every commit and parsed in full on
  every `Repo::open`, the one cost that grew with the whole history instead of with the change at
  hand. It is now one shard per tracked file (`<store>/chains/<blake3(relpath)[..16]>.bin`, the same
  zstd-bincode encoding), loaded on first touch and flushed per dirty shard. A commit rewrites exactly
  the shards of the files it touched, and an open parses nothing up front. Stores still carrying a
  monolithic `chains.bin` (or the older `chains.json`) are read transparently and split on their next
  save, which then retires the monolith. Until that delete the monolith stays the source of truth, so
  a crash halfway through the split just runs it again.
- **A sharded objects folder** (`delta.rs::write_loose` and `read_loose`). Loose objects go into
  `objects/<hash[..2]>/` (256 subfolders) instead of one flat folder, because 100,000 or more tiny
  files in one folder slow down NTFS lookups and multiply Defender scans. Reads fall back to the flat
  path, so older stores never need to migrate.
- **One pack file per commit** (`delta.rs::Packs`, `commit_prepared_batch`). A batch of 32 or more
  distinct new objects (the whole-document first commit, or an edit touching many tiles) is written as
  one `objects/pack/<hash>.pack` file (a header, a compressed index, then the payloads back to back)
  instead of one file per object. Measured on Windows, the cost of creating each file (Defender's
  real-time scanning, worst for a freshly installed app with no reputation yet) was about 28 s of a
  33 s first commit on a large canvas, and parallelism can't hide it, because the cost is in the create
  itself. Reads try the loose paths (sharded, then legacy flat) and then an in-memory index over all
  pack headers, built lazily, with thread-safe positional reads, so parallel tile rebuilds can hit one
  pack at once. Small batches stay loose, so per-object dedup stays visible and tiny commits pay no
  pack indirection.

## Build configuration

`src-tauri/Cargo.toml`:

```toml
[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3
[profile.release]
lto = "thin"
codegen-units = 1
```

`tauri dev` runs the same image and compression hot loops as a release build. Fully unoptimized
(`opt-level = 0`, the default dev profile), they're 10 to 50 times slower, enough to make the app
feel broken during development. Dependencies (image codecs, zstd, blake3, rayon) build fully
optimized while the app's own crate stays at `opt-level = 1` for bearable rebuild times. Release
builds use thin LTO for a faster binary without paying full-LTO link times.

`blake3 = { version = "1", features = ["rayon"] }` turns on the parallel hashing path described
above. `windows-sys` is Windows-only and enables two features: `Win32_System_Threading`, for the
thread and process priority calls behind [CPU headroom](cpu-headroom.md) (the standard library has no
thread-priority API), and `Win32_Storage_FileSystem`, for the free-space check and for hiding the
`.kvc` container.

## Ceilings and deferred work

The shortcuts with known limits, collected in one place:

- The commit-time entry skip uses crc32 and size to detect changes, which has about a 2⁻³² chance of
  a false match per changed entry. The upgrade is hashing the compressed bytes.
- The delta patch and snapshot thresholds (64 KB, a chain length of 20) are untuned constants.
  Revisit them if storage size ever matters more than it does now.
- Raster downscaling is an area-average box filter in premultiplied alpha, crisp at the viewer's
  zoom. Truly pixel-accurate deep zoom would need a higher 2048 px cap, which costs cache disk space,
  and that's deliberately not done so storage stays flat.
- Scan byte retention (`scan::RETAIN_BUDGET`, 512 MB) bounds memory on the commit path; a document
  larger than the budget is re-read at commit time instead of being held. It's an untuned constant.
  (The bound was documented here, and referenced from `commit.rs`, for a while before it existed; a
  performance audit found that and implemented it in `655b992`.)
- Composite tiling re-encodes `mergedimage.png` on restore: the pixels are exact but the bytes differ
  from Krita's encoding, so the entry's crc32 changes and the first commit after a restore processes
  the composite again (every block dedups; only the manifest is new). It fixes itself, but it's a
  known one-commit blip.
- Tile pixel deltas (`tilePixelDeltas`) stay opt-in until the LZF decode and encode cost is measured
  as affordable on two-core hardware. The old "decode LZF and delta the raw pixels" note in `tiles.rs`
  is now this flag.
- A layer-subset commit still builds a whole synthesized `.kra` and hands it to `commit_kra`, which
  decomposes it again. That design keeps the rest of the engine free of special cases. The upgrade is
  manifest-level splicing (see [Layer-subset staging](#layer-subset-staging)).
- `stage::repackage` holds the working document and the synthesized output in memory at once (plus
  the committed subset, now only the reverted layers), against the 64 MB `RESTORE_CHUNK_BUDGET` every
  other path respects. `run_heavy`'s two permits bound it in practice, and the fix is the same
  manifest splice.
- A change to a `.kra`'s `<IMAGE name>` (Krita derives it from the filename) changes every
  `kra:{relpath}:entry:` and `:tile:` stream key and every manifest entry path, so the next commit
  re-stores the whole document, because `commit_kra`'s crc32 and size reuse misses on every entry.
  This isn't specific to staging; a whole-painting commit pays it too. It isn't worth a special case
  until it shows up in practice.
- Low-memory working diffs (`lowMemoryDiff`) stay opt-in: inflating one entry at a time bounds peak
  memory but decompresses again per layer, so the default in-memory path stays faster for interactive
  diffs. Only the working-tree diff view is affected; committed diffs and stored data are untouched.
