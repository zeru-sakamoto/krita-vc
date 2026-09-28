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
All of it runs inside the engine's own budgeted pool (see [cpu-headroom.md](cpu-headroom.md)). The
cheap reads (history, branches, a scan) deliberately don't: they do no parallel work, and on a
2-core laptop the pool is a single worker that a diff holds for its whole run.

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
- **Rebuilding a `.kra`, in chunks, straight to disk.** `write_kra_from` (`kra.rs`) resolves the
  manifest entries' bytes, replaying tile chains as needed, with `par_iter()` in chunks bounded by the
  same 64 MB budget, writing each chunk to the zip before the next one is built. Every decompressed
  entry and the whole output zip used to sit in memory together (about twice the document's size, a
  paging risk on a 4 GB machine). The restore paths (discard, `restore_file`, bringing set-aside
  work back, the fallback of a switch or rollback) now hand it the temp file beside the artwork
  (`kra::write_kra`, `repo::write_file_atomic_with`), so the peak is one chunk; only callers that
  want the bytes in memory (`reconstruct_kra`: a set-aside merge, staging's committed subset, undo's
  hash fallback) still build a `Vec`. `materialize_kra_into`'s full rebuilds are chunked the same
  way, although switching and rolling back normally take its cheaper incremental path and only fall
  back to this.
- **The commit's dedup filter.** `commit_prepared_batch` (`delta.rs`) checks in parallel whether
  each candidate object already exists, cheapest check first (the in-memory pack index snapshot,
  then the sharded loose path). Thousands of serial `stat` calls per large
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
- **Decoding tiles within one layer, straight into the capped raster.** `layer_raster` (and the
  working file's `rasterize_working_tiles`) reconstructs and LZF-decodes tiles in parallel batches of
  256 (`raster::rasterize_tiles`), and adds each one's pixels straight into a buffer the size of the
  capped output. Every source pixel belongs to exactly one output pixel's box, so the premultiplied
  sums `box_downscale` takes can be accumulated from the tiles directly, with the same integer
  rounding; a unit test pins the result bit for bit to the old full-canvas path. That canvas was
  `width × height × 4` bytes per layer (278 MB for one 600 dpi A3 layer, three layers at a time),
  filled with the default pixel, blitted into, then read again to shrink it. Tiles off the 64 px grid
  or overlapping (Krita never writes them) and layers already within the cap still take the canvas
  path. Nested rayon is fine here; it's one work-stealing pool.
- **blake3 hashing.** `hash_bytes` (`repo.rs`) uses blake3's rayon-parallel `update_rayon` for
  buffers of 1 MB or more (whole `.kra` files during a scan or commit) when it's already on a worker
  of the budgeted pool. Off it (the cheap reads) it hashes on one thread rather than spill onto
  rayon's global pool, which is every core at normal priority. Small buffers such as tiles stay on
  the cheap single-threaded path, because spinning up parallel hashing for a few KB is pure overhead.
  `hash_file` hashes a file the same way, 16 MB at a time.

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
- **A saved-but-unversioned painting is read once per save, not once per poll**
  (`<store>/worktree.json`). Saved but not yet a version is the normal state while painting, and it
  fails the fast path by definition, so every scan used to read and blake3 the whole file just to
  answer "modified": about 80 ms per `kvc status` at 105 MB and 140 ms at 195 MB, on the Krita docker's
  1.5-second poll, and seconds when the file had dropped out of the page cache. A scan that has to
  read now records the size and mtime it stat'ed and the hash it got, and the next scans answer from
  that while both still match, under the same racy-clean guard as the index (the file's mtime must be
  strictly older than the sidecar's). A document with no versions yet is `U` whatever it holds, so
  it isn't read at all. The file is a cache: written best-effort (temp and rename under a
  per-process name, since `kvc status` and the app can scan at once), left out of backups, and a
  missing or unreadable one only means the next scan reads again. Callers that want the bytes still
  read. Together with the docker skipping the spawn while nothing changed (see
  [cpu-headroom.md](cpu-headroom.md#the-plugins-poll-at-most-one-process-usually-none)), a poll went
  from about 80 ms to 9 ms at 105 MB (140 ms to 9 ms at 195 MB), the same as a clean poll, and
  usually to four `os.stat` calls.
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
  still take the rebuild fallback. A restore records the hash of the file it wrote by reading its
  temp file back before the rename (`repo::write_file_atomic_with`), since the zip writer seeks back
  to patch each entry's header and can't be hashed on the way out.
- **A single-pass tile diff** (`kra::diff_tile_indexes` over borrowed `TileIndexRef`s). The set of
  changed layers and the union change region come out of one pass that builds each entry's old
  `(x, y) → hash` map once. Two functions used to rebuild the maps separately, and an owned
  `tile_index()` (since removed) cloned every 64-character tile hash (megabytes of string churn on a
  Krita-scale document).
- **Rollback without a re-commit** (`commit::rollback_to_commit`). A rollback used to write out the
  target tree and then run a full `commit_snapshot` (rescan, re-read, and re-decompose every restored
  `.kra`) just to rediscover content hashes already recorded in the target tree. The commit is now
  built directly from the tree diff between the target and the current tree: no scan, no `.kra`
  decomposition and no object writes, which roughly halved rollback time.
- **The delta-chain heuristic** (`delta.rs::looks_compressed` plus a 64 KB floor). bsdiff is skipped
  for small streams (a chain-walk reconstruct and a suffix sort to save a few KB isn't worth it) and
  for already-compressed payloads (PNG, zip or zstd magic; a patch against compressed bytes comes out
  near full size). Both go straight to a single zstd snapshot.
- **Manifest reuse, within a request and across them.** Every layer, region and composite read of a
  diff reuses one parsed manifest per side instead of walking the patch chain again, and the parent's
  is loaded once and handed to both the art diff and the embedded-palette diff (`art_diff_dto`'s
  `old_manifest`; each used to load it). Parsed manifests are also kept across commands
  (`kra::load_manifest`, a process-wide cache keyed by store and content hash): a manifest never
  changes, and a Version Map node's parent manifest is its left neighbour's own, so a screenful of
  nodes loads each manifest once instead of three times. The cache is bounded by total tile
  references (400,000, about 65 MB of parsed manifests at the cap), least recently used first, and a
  repo set to verify its reads (the restore paths) never takes a manifest from it. A manifest
  stream's patch chain is also capped at five (`delta::MANIFEST_CHAIN_MAX`) rather than the store's
  20, so a cold load replays at most five patches of a multi-megabyte JSON, for a full snapshot every
  sixth version (a few MB compressed; see the ceilings below for what that costs). A Version Map
  node on a 200-version history of a 45,000-tile painting went from 1.13 s to 0.35 s (median of the
  last 20 nodes, warm raster cache).
- **GC's manifest memo.** Mark-and-sweep loads every reachable commit's `.kra` manifest to walk the
  streams it references. Plain `reconstruct` replays each manifest version's patch chain from the
  nearest full snapshot independently, redoing the shared prefix every time, which is quadratic in a
  file's history length. GC threads one content-hash memo (`Repo::reconstruct_cached` through
  `kra::load_manifest_memo`) through the marking loop, so each version is built from its immediate
  predecessor exactly once: linear patch applications instead of quadratic. The memo is keyed by a
  pure content hash, so it dedups safely across paths. It's bounded (`delta::ReconstructMemo`: the
  last four patch bases, as `Arc`s): walking oldest first, a version's base is the one just rebuilt,
  so four keep the walk linear, where the old unbounded map held every manifest version it had built
  (975 MB at 200 versions of a 45,000-tile painting). Marking such a history still takes about 20 s,
  nearly all of the cleanup's dry run: about two thirds of it rebuilding and parsing 200
  multi-megabyte manifests, the rest collecting their 9 million stream references into one set.
  Nothing in the September 2026 fixes shortened it; the manifest cache serves diffs, not this walk,
  which reads each manifest exactly once anyway.
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
  capped PNGs the raster path already produced (`kra::LayerRaster` carries the cache key, and the PNG
  bytes when it has just encoded them; `LayerRaster::png` reads a cache hit's back from disk, only
  for this). It adds one capped-resolution pixel compare and an outline trace of about 200 px
  per modified layer, which is negligible next to the tile rebuild and PNG encode already paid, and it
  runs inside the same rayon `par_iter`. The mask PNG is cached content-addressed by both layer
  raster keys (`kra::diff_cache_key`), so a repeat view skips the diff.
- **Palette diffs stay off the raster machinery** (`palette.rs`, `commands::palette_dto_from`). Palettes
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
- **One parse per Changes refresh** (`commands::parsed_working`). A refresh is two calls,
  `working_diff` for the metadata and then `working_layers` for the rasters, and each used to read,
  inflate and hash the whole working painting (255 ms apiece on a 105 MB file). The first now keeps
  its parse, keyed by path, size, mtime and the `lowMemoryDiff` flag, and the second takes it and
  lets go of it, so a whole decoded document isn't kept resident between refreshes. The second
  doesn't always come (returning to Changes serves the layers from the frontend's cache), so an
  untaken parse goes after ten seconds (`commands::WORKING_PARSE_TTL`).
- **`Repo::open_without_log`** skips `commits.log`, the one part of a store that grows with every
  version, for the reads that never look at it: `kvc status`, `branches` and `stash-list` (the Krita
  docker's poll), `scan_repository`, `list_branches`, `list_stashes` and the settings getters. It's
  read-only by construction: the log's damage check never ran, so every write refuses
  (`Repo::ensure_writable`). Chains and packs load lazily on every open, so
  `Repo::open_light` is now plain `open`, kept under its own name to mark the paths that read history
  but never rebuild content.
- **Incremental `.kra` writes on switch, merge and rollback.** `kra::materialize_kra_into` builds
  the target version out of the working file. Entries identical between the current and target
  manifests are copied raw (`raw_copy_file`) from the zip on disk, with no store reads and no
  inflating or deflating, after each one is checked against the manifest's recorded crc32 and size. A
  changed tiled entry lifts its unchanged tiles from the working copy, and only tiles whose content
  differs are replayed from the object store. Switch cost follows the difference between the
  branches, not the document's size. It reads the working file through a handle and writes the
  result into the temp file beside it, so neither is held whole in memory (a switch of the 195 MB A3
  painting used to peak at 574 MB). Any mismatch or error deletes the temp and falls back to a full
  rebuild from the store.
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
  table shows, stores exactly the same amount. (Since superseded by the raw copy below.)
- **Answer "still dirty" from a `stat`** (`TrackedFile.partial`; see
  [Skipping work entirely](#skipping-work-entirely)). This is the one an artist feels: it had been a
  full read and blake3 of the painting on every scan, and `kvc status` runs on the Krita docker's
  1.5-second poll.
- **Stop hashing what gets thrown away** (`commit::bytes_of`). `bytes_of` returned a blake3 of the
  whole rebuilt document, and three of its five callers discarded it.

That left `stage_kra`'s repack as the largest term (2.3 s of the 4.0 s): it still inflated every
entry of the working document and deflated it again, level 1 or not, to change one layer's worth of
it. The September 2026 audit's fix **raw-copies** every entry but `maindoc.xml` (`raw_copy_file`,
and `raw_copy_file_rename` for a reverted layer's data files): the compressed bytes, method, crc32
and size go across as they are, and the crc32 and size `commit_kra` compares describe the
uncompressed bytes either way, so what gets stored can't change. On the A4 corpus painting (105 MB,
41 entries) the repack's inflate-and-deflate had been 961 ms against 35 ms for the copy, and saving
10 of its 11 top-level layers went from about 3.56 s to 2.45 s (two runs each, old and new binaries
alternated). `merge::repackage`, which brings set-aside work back onto an edited file, copies the
same way, where it had been deflating every entry at level 6, and writes straight into the temp file
beside the artwork (`merge::merge_layers_into`): 15.8 s and 624 MB became about 6 s and 440 MB.

Still on the table: `commit_kra` then decomposes the synthesized archive again. The upgrade is
manifest-level splicing: commit the surviving working entries as usual and substitute the previous
manifest's `KraEntry` values for the reverted ones, so no document is ever built. That would also
retire the roughly three-times-the-document peak memory noted under
[Ceilings and deferred work](#ceilings-and-deferred-work). It isn't built, because the measured gap
no longer justifies how much it would touch. A plain-language write-up of the first staging audit
is kept with the site copy, in the local (gitignored) `content/PERFORMANCE_AUDIT.md`.

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
- **Layer-list thumbnails** (`raster::thumb_png`, `THUMB_DIM = 128`). The layer navigator draws
  36 × 28 px thumbnails, and it used to point them at the full capped raster, so the webview decoded
  up to 2048 × 2048 (16 MB of bitmap) per layer to draw each row. The raster path now writes a 128 px
  thumbnail beside each capped raster while it has the pixels, cached under a key derived from the
  raster's, and `LayerDto.beforeThumb`/`afterThumb` carry its URL; the list falls back to the full
  raster where there's none. A raster cached before thumbnails existed gets one made the next time
  it's served.
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
- **Backups store what's already compressed** (`repo::zip_file`). The `.kra` is a zip, and objects,
  packs and chain shards are zstd, so the backup archive stores them as they are and deflates only
  the JSON state files and logs; each file streams from its handle instead of being read whole
  first. Deflating everything at level 6 was 7.2 s of a 7.6 s backup of a 105 MB painting and its
  store, to make the archive 13% smaller.

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

- **A content-addressed disk cache** (`<store>/cache/`, `raster::cached_url`, `cache_read` and
  `cache_write`). Every capped PNG, composite or per-layer, is keyed by a hash of everything that
  determines its pixels (tile positions and hashes, the dimensions and the resolution cap, or the
  composite entry's content hash). Keys never need invalidating, unchanged layers share one entry
  across commits and across the committed and working diff paths, and a repeat view, even after an
  app restart, skips rebuilding, decoding and encoding entirely.
- **A cache hit is a `stat`, not a read** (`raster::cached_url`). Most hits only need the entry's
  URL, which the webview then fetches through `kvcimg` anyway, and reading a 2048 px composite just to
  print its URL cost several MB per call, two or three calls per Version Map node. Only the callers
  that need pixels read the file (`LayerRaster::png`: a modified layer's own change highlight, the
  stacked composite), and the base64 fallback reads it when it builds its data URL. An entry pruned
  between the `stat` and that read is a miss, and rebuilt; it used to come back as an empty data URL.
- **Change masks carry their outline and box** (`raster::mask_meta`). The changed-pixel mask is
  cached as a PNG with its outline path and normalized bounding box in `tEXt` chunks (`kvc-outline`,
  `kvc-bbox`) ahead of the pixels, so a hit answers both from the header without decoding the mask
  and tracing it again, which every Version Map node paid on a warm cache only to throw the result
  away. A mask cached before the chunks existed is decoded as before.
- **Frontend session caches** (`repoData.ts`). `diffCache` (commit-diff results) is an LRU of up to
  300 entries, a few KB each now that rasters travel as `kvcimg` URLs, and `layerCache` (streamed
  layer sets) one of 20, both keyed by the request. An in-flight `commit_diff` is shared
  (`diffInflight`, like `useWorkingDiff`'s), so opening a Version Map node whose thumbnail is still
  loading doesn't send the same heavy call twice. Committed entries
  key on `path|commitId` only, because a commit never changes, so a write (commit, rollback, undo)
  doesn't cold-start every diff viewed before it; only the working layer key includes the refresh
  nonce, since the working copy really does change. Cancelled or partial layer requests are never
  cached, because a torn-down effect's `received` map may be incomplete and caching it would poison
  the key for later visits.
- **A bounded raster cache** (`raster::cache_prune`). `cache/` used to grow for the life of the
  store. It now has a size budget (`Config.cacheMaxBytes`, default 256 MB), which Settings exposes as
  "Preview cache size" (128 MB to 2 GB). A hit touches the entry's mtime, at most once a day, so hot
  entries survive (pruning only has to tell this week's entries from last month's, and a touch per
  hit was a file open for write per raster per view), an oldest-first prune runs after layer
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
  and reads drop a torn last line that the next save cleans up.
- **Slim chain versions (`KVCC2`)** (`repo.rs::Version::object_name`). Each chain version used to
  store its object filename, which can be derived from `hash` and `base`, duplicating a 64-character
  hash per version forever. Bincode isn't self-describing, so the fix rides on an explicit format tag:
  `KVCC2`-prefixed shards hold the slim shape. The readers for the older untagged shards, and for
  the monolithic `chains.bin` and `chains.json` before them, are gone: every one of those formats
  predates per-document stores, and v2.0.0 shipped with no migration from v1.
- **Chain shards per layer entry, loaded lazily** (`repo.rs::ChainStore`, `shard_of`). The chains
  store (every version of every delta stream) used to be one file, rewritten in full on every commit
  and parsed in full on every `Repo::open`. Sharding it per tracked file fixed that for a folder of
  paintings, but with one document per store it meant one shard again: every version of every tile,
  rewritten and fsynced on each commit and decoded whole by the first chain lookup of every command
  (4.3 MB at 200 versions of a 45,000-tile painting, 125 to 185 ms to re-encode per commit, 40 to
  57 ms to decode). Now each tiled entry (a layer's tiles, or the composite's blocks) has its own
  shard, and the manifest and small entries share the document's
  (`<store>/chains/<blake3(shard name)[..16]>.bin`, the same zstd-bincode encoding), loaded on first
  touch and flushed per dirty shard. A commit that edits two layers rewrites those two shards and the
  document's. The price is an fsync per shard a commit touches rather than one per commit, which a
  commit that edits only a tile or two pays for without the shard size to win it back. A store
  written before this keeps every chain in its document shard; that shard is split in memory when it
  loads, and the split persists with the next save, which writes the tile shards before the shrunken
  document shard, so a crash in between only means splitting again; a key found in both files is
  merged, not chosen (see [data-integrity.md](data-integrity.md)).
- **A sharded objects folder** (`delta.rs::write_loose` and `read_loose`). Loose objects go into
  `objects/<hash[..2]>/` (256 subfolders) instead of one flat folder, because 100,000 or more tiny
  files in one folder slow down NTFS lookups and multiply Defender scans. The fallback that read the
  flat layout is gone, since no per-document store ever used it, and with it a failed file open per
  object lookup.
- **One pack file per commit** (`delta.rs::Packs`, `commit_prepared_batch`). A batch of 32 or more
  distinct new objects (the whole-document first commit, or an edit touching many tiles) is written as
  one `objects/pack/<hash>.pack` file (a header, a compressed index, then the payloads back to back)
  instead of one file per object. Measured on Windows, the cost of creating each file (Defender's
  real-time scanning, worst for a freshly installed app with no reputation yet) was about 28 s of a
  33 s first commit on a large canvas, and parallelism can't hide it, because the cost is in the create
  itself. Reads ask an in-memory index over all pack headers first (built lazily; after the first
  commit nearly every tile lives in a pack), and only then the loose path, and each pack stays open
  behind one handle for positional reads, which share no cursor, so parallel tile rebuilds can hit one
  pack at once. The old order paid two failed loose-path opens and a fresh open of the pack for every
  packed object, about 88 µs a read against 9.4 µs. Small batches stay loose, so per-object dedup
  stays visible and tiny commits pay no pack indirection.

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
- The delta patch and snapshot thresholds (64 KB, a chain length of 20, five for manifests) are
  untuned constants. Revisit them if storage size ever matters more than it does now. The manifest
  cap has a measured price: a full manifest snapshot every six versions instead of every 21, which
  took a synthetic 200-version history of a 45,000-tile painting from 37 MB to 79 MB, about 0.2 MB
  more per version. Its tiles are 49 bytes, so the manifest is an unusually large share there; next
  to a version that stores a few hundred real 16 KB tiles it's a few percent, but an edit that
  changes only a handful of tiles pays proportionally more.
- The parsed-manifest cache (`kra::load_manifest`) is bounded by tile references, not bytes
  (400,000, about 65 MB at the cap). A byte-exact budget is the upgrade if it ever matters. It lives
  for the process, so in the `kvc` CLI, one command per process, it's filled for nothing, though
  only for as long as the command runs.
- `commands::parsed_working` keys on size and mtime, like the scan's fast path, so a rewrite inside
  one timestamp tick is missed for one refresh of a diff view (never stored data). If
  `working_layers` never follows `working_diff`, the parsed document stays in memory for ten seconds,
  and a `working_layers` held up longer than that behind the other heavy commands parses again.
  `worktree.json` has the index's racy-clean ceiling for the same reason.
- Chain shards per layer entry cost one fsync per shard a commit touches, where one shard cost one.
  A commit that edits a few tiles of one layer pays two where it paid one, without a big shard to
  win it back.
- A set-aside merge still holds three whole documents in memory while it runs: the set-aside
  version, the working file and the ancestor (its output streams to the temp file). The ancestor is
  rebuilt from only the layers the set-aside version shares with it, which is usually all of them,
  so for a 105 MB painting the merge still peaks around 440 MB.
- A version's recorded `storedBytes` counts only the objects that commit wrote. Content first
  stored by set-aside work, or by a version that was later undone, is already on disk when a commit
  reuses it, so the storage report attributes it to no version (it's still in the store total).
- Backups store the `.kra` and the store's already-compressed objects instead of deflating them, so
  an archive is about 13% larger than it was.
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
