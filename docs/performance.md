# Performance

How the `.kra` diff path stays fast, and the specific techniques used. This complements
[version-control.md](version-control.md) and [visual-diff-viewer.md](visual-diff-viewer.md), which
describe the same code from an architecture angle — this doc is the "why is it fast" index.

The driving cost center is the **visual diff**: a `.kra` can be a large tiled document, and naively
reconstructing every layer at full resolution on every commit click would be slow enough to feel
broken. Everything below exists to keep that path off the UI's critical path or off the CPU
entirely.

## Diff loading: two stages instead of one blocking call

`commit_diff`/`working_diff` return the composite (`mergedimage.png`) and layer *metadata* only —
no per-layer pixels. The panel renders immediately. Per-layer rasters — the expensive part — are
fetched afterwards by `commit_layers`/`working_layers`, called lazily from the frontend
(`useArtLayers`, [`src/lib/repoData.ts`](../src/lib/repoData.ts)) once the fast diff has already
painted. See `commands.rs::art_diff_dto`'s `with_rasters` flag.

## Streaming over a Tauri `Channel`

`commit_layers`/`working_layers` don't return a `Vec<LayerDto>` — they take a
`Channel<LayerDto>` and send each layer the instant it's rasterized. Layers finish out of order
(rayon), so the frontend merges by layer id as messages land (`useArtLayers`'s `onmessage`) instead
of waiting for the slowest layer to block all the others.

## Parallelism (rayon)

Independent per-layer/per-tile work is farmed across cores instead of running serially:

- **Layer rasterization** — `art_diff_dto` rasterizes all of a document's layers with
  `par_iter()` (`commands.rs`); order is preserved via indexed collect.
- **Whole-commit preparation, chunked** — `commit_kra` (`kra.rs`) walks the zip serially (the
  reader is inherently serial; unchanged entries skip via crc32+size) accumulating inflated
  entries into `RESTORE_CHUNK_BUDGET`-bounded chunks (64 MB uncompressed), and per chunk runs a
  rayon pass over its changed entries — all layers' tiles *and* raw entries like `mergedimage.png`
  — through `prepare_stream` (the CPU-heavy reconstruct+bsdiff+verify/zstd), then a single serial
  fold (`flush_entry_chunk` → `commit_prepared_batch`), dropping the buffers before the next chunk
  reads. A multi-layer edit costs ~max(layer) instead of sum(layers); each stream key appears
  once per commit, so parallel prepare can't race — only the fold needs `&mut`. **Peak RAM is one
  chunk**, not the whole decompressed document — previously a first commit or a big edit inflated
  every changed entry at once (the mirror of `reconstruct_kra`'s restore-side chunking).
- **`.kra` reconstruction, chunked** — `reconstruct_kra` (`kra.rs`) resolves manifest entries'
  bytes (tile-chain replay included) with `par_iter()` in **budget-bounded chunks**
  (`RESTORE_CHUNK_BUDGET`, 64 MB of uncompressed entry data), each written serially to the zip
  and dropped before the next chunk builds. Previously every decompressed entry *and* the whole
  output zip sat in RAM simultaneously (~2× document size peak — a paging risk on 4 GB
  devices); peak is now output + one chunk. `materialize_kra`'s build runs are chunked the same
  way; branch switch/rollback normally take that cheaper incremental path and fall back to this.
- **Commit dedup filter** — `commit_prepared_batch` (`delta.rs`) probes each candidate object's
  existence in parallel, cheapest check first (in-memory pack-index snapshot → sharded loose
  path → legacy flat); thousands of serial stats per large commit hurt on cold HDDs. The pack
  index is handed out as an `Arc` snapshot so parallel lookups never hold its mutex.
- **Raster downscale** — `box_downscale` (`raster.rs`) runs parallel over destination rows with
  integer accumulation (the old body was serial f64 per source pixel over the full-resolution
  canvas). Same area-average premultiplied semantics — the `box1` cache token stays valid; a
  unit test pins the integer version to the f64 reference within ±1.
- **Object writes on commit** — `commit_prepared_batch` (`delta.rs`) writes all of a layer's new
  tile objects in parallel before the serial chain fold; content-addressed writes are independent
  and idempotent, and thousands of serial small-file creates (NTFS + Defender) were a dominant
  commit cost on Windows.
- **Per-tile decode within one layer** — `layer_raster` reconstructs + LZF-decodes each tile in
  parallel, then blits serially into the shared canvas (nested rayon is fine — one work-stealing
  pool).
- **blake3 hashing** — `hash_bytes` (`repo.rs`) uses blake3's rayon-parallel `update_rayon` for
  buffers ≥1 MB (whole `.kra` files on scan/commit); small buffers (tiles) stay on the
  cheap single-threaded path, since spinning up parallel hashing for a few KB is pure overhead.

The `prepare_stream`/`commit_prepared` split (`delta.rs`) is the general shape this depends on:
read-only preparation (`&self`) can run in parallel across streams; only the serial fold
(`&mut self`) touches shared state.

## Skipping work entirely

- **Scanner fast path** (`scan.rs`) — a tracked file whose size+mtime still match the index
  (`TrackedFile.size`/`mtime`, `repo::size_mtime`, nanosecond resolution) is assumed unchanged and
  never read or hashed. Big `.kra` files are the case this matters for.
- **Tracking guardrail** (`scan::is_supported`) — only `.kra` and palette files (`.gpl`/`.kpl`/
  `.aco`/`.ase`) are newly tracked; any other file is rejected on a single lowercased-extension
  check instead of being read and blake3-hashed. The guard only runs for files **not already in
  the index** (`contains_key` short-circuits the `&&`), so a steady-state scan pays nothing for
  already-tracked files, and a project folder full of unsupported assets scans strictly faster than
  before (no per-file I/O for the ignored ones). One `to_lowercase` alloc per untracked file, kept
  minimal on purpose.
- **Scan→commit byte + hash handoff** (`scan::scan_detailed`) — the scan already read and
  blake3-hashed every changed file; `commit_snapshot` reuses the hash (plus size/mtime) *and*
  the file bytes themselves (`ScanChange.bytes`, kept under a 512 MB cumulative retention
  budget), so a big `.kra` is read exactly once per commit — the old "page-cache hit" re-read
  was a full extra HDD pass on a 4 GB machine where the cache had already evicted it.
  (`scan()` passes `keep_bytes = false`, so status-only paths never hold buffers.)
- **Undo without reconstruction** (`CommittedFile.fileHash`) — every commit records the blake3
  of each file as it sat on disk; `undo_last_commit` rewinds the index from that recorded hash
  instead of reconstructing a whole `.kra` from the store just to re-hash it. Records from
  before the field existed still take the reconstruct fallback. Restores get the same
  treatment: `bytes_of`/`restore_bytes` return the hash alongside the bytes (for a generic
  blob it *is* the stream hash — zero extra hashing).
- **Single-pass tile diff** (`kra::diff_tile_indexes` over borrowed `TileIndexRef`s) — the
  changed-layer set and the union change region come out of one pass that builds each entry's
  old `(x,y)→hash` map once; previously two functions each rebuilt the maps and the owned
  `tile_index()` cloned every 64-char tile hash (multi-MB string churn on a Krita-scale doc).
- **Rollback without re-commit** (`commit::rollback_to_commit`) — a rollback used to materialize
  the target tree, then run a full `commit_snapshot` (re-scan, re-read, re-decompose every
  restored `.kra`) purely to rediscover content hashes already recorded in the target tree. The
  commit is now synthesized directly from the target-vs-current tree diff — no scan, no `.kra`
  decompose, zero object writes — roughly halving rollback time.
- **Delta-chain heuristic** (`delta.rs::looks_compressed` + a 64 KB floor) — `bsdiff` is skipped
  for small streams (a chain-walk reconstruct + suffix-sort to save a few KB isn't worth it) and
  for already-compressed payloads (PNG/zip/zstd magic — a patch against compressed bytes comes out
  near full size). Both go straight to a single-object zstd snapshot.
- **Manifest reuse** — `kra::load_manifest` reconstructs+parses a `.kra` manifest once per diff
  request; every layer/region/composite read reuses the parsed struct instead of re-walking the
  patch chain per call.
- **GC manifest memo** — mark-and-sweep loads every reachable commit's `.kra` manifest to walk its
  referenced streams. Plain `reconstruct` replays each manifest version's patch chain from the
  nearest full snapshot independently, re-doing the shared prefix every time — quadratic in a
  file's history length. GC threads one content-hash memo (`Repo::reconstruct_cached` via
  `kra::load_manifest_memo`) through the marking loop, so each version is rebuilt from its
  immediate predecessor exactly once — O(N) patch-applies instead of O(N²), turning long-history
  marking from seconds into milliseconds. A pure content hash keys the memo, so it dedups safely
  across paths.
- **Commit-time crc32/size skip** — `commit_kra` compares each zip entry's crc32 + uncompressed
  size (from the central directory, no inflate needed) against the previous commit's manifest for
  that path (`commit_snapshot` passes it in); a match reuses the old manifest entry untouched, so
  an edit to one layer doesn't re-inflate or re-store every other entry in the archive.
- **`layer_diff` command** — pulls only `maindoc.xml` out of each side's manifest instead of
  reconstructing the whole archive for a single small entry.
- **`TileCache`** (`delta.rs`) — a request-scoped cache keyed by content hash; the before/after
  sides of a modified layer usually share most tiles, so each shared tile reconstructs once, not
  twice.
- **Unchanged-layer raster reuse** — `art_diff_dto` clones the `after` raster into `before` for
  layers marked `unchanged` instead of decoding/encoding the identical pixels twice.
- **Per-layer change highlight rides the layer stream** — a modified layer's own mask + outline +
  region (`layer_diff_overlay` → `raster::diff_overlay_full`) is diffed from the before/after capped
  PNGs the raster path already produced (`kra::LayerRaster` now returns the PNG bytes + cache key
  alongside the URL), so it adds one capped-resolution pixel compare + a ~200px outline trace per
  modified layer — negligible next to the tile reconstruction + PNG encode already paid, and it runs
  inside the same rayon `par_iter`. The mask PNG is cached content-addressed by both layer raster
  keys (`kra::diff_cache_key`), so repeat views skip the diff.
- **Palette diffs stay off the raster machinery** (`palette.rs`, `commands::palette_dto`) — the
  four palette formats are KB-sized and their swatch diff is O(swatches), so unlike `.kra` there is
  deliberately **no two-stage load, no streaming, and no `.kvc/cache/` entry**: the diff is parsed
  and computed inline inside `commit_diff`/`working_diff` (already on the blocking pool). Adding a
  cache would cost more than it saves. Version-to-version *storage* still delta-compresses for free
  — palettes go through the same `store_stream` bsdiff chain as any generic blob (below the 64 KB
  patch floor they snapshot, which is correct for files this small).
- **Working-tree diffs never touch the store** — `parse_working`/`WorkingKra` (`kra.rs`) decode a
  working `.kra` straight from an in-memory `ZipArchive`; no `bsdiff`, no chain reconstruct, no
  object writes. Older code staged the working file into the object store just to reuse the
  rasterizer — viewing a diff should never write. The opt-in **`lowMemoryDiff`** flag trades a
  little CPU for bounded peak memory here: instead of holding the whole decompressed document,
  `WorkingKra` keeps only the compressed archive plus per-entry metadata and re-inflates each
  entry on demand, so peak RAM is the compressed document plus one decoded entry. Off by default
  (the in-memory path is faster for interactive diffs); change detection is identical either way.
- **`Repo::open_light`** — skips chains entirely (even the legacy-monolith parse a pre-sharding
  repo would pay) for read paths that never touch storage (`scan_repository`, `list_commits`).
  With sharded chains a full `Repo::open` is nearly as cheap — shards load on first touch — but
  light opens keep the invariant explicit.
- **Incremental `.kra` materialization on switch/merge/rollback** — `kra::materialize_kra`
  rebuilds the target version *out of the working file*: entries identical between the current
  and target manifests are `raw_copy_file`d from the on-disk zip (no store reads, no
  inflate/deflate; each entry verified against the manifest's recorded crc32+size first), and a
  changed tiled entry lifts its unchanged tiles from the working copy in memory — only tiles
  whose content differs replay from the object store. Switch cost tracks the diff between
  branches, not document size. Any mismatch or error falls back to the full `reconstruct_kra`.
- **Chains skipped when clean** — `Repo::save` only rewrites chain shards a commit actually
  dirtied (`ChainStore` per-shard dirty tracking); switch, merge, and undo mutate only
  index/commits/branches, so they never pay a chains rewrite at all.
- **Read-path integrity check dropped** — `reconstruct` no longer re-hashes rebuilt bytes; every
  patch is already round-trip-verified when it's *written* (`prepare_stream`), and objects are
  content-addressed, so a second hash on every read was pure redundancy on the hottest loop in the
  diff.

## Output size / encode cost

- **Raster downscaling** (`raster.rs::MAX_RASTER_DIM = 2048`) — both per-layer rasters
  (`cap_rgba`) and the composite (`cap_png`, decoding an already-encoded PNG just to re-cap it)
  are capped to a 2048px longest side before encoding. A diff preview never needs full document
  resolution (Krita canvases run into the thousands of px); this was the dominant cost in both
  encode time and the IPC payload shipped to the webview. Downscale is an **area-average box
  filter** (`box_downscale`, premultiplied-alpha) — one extra pass over the source vs the old
  nearest-neighbour, negligible next to PNG encode, but crisp/alias-free now that the viewer can
  zoom. The filter is versioned in the cache keys (`box1` token) so changing it invalidates cleanly.
- **Fast PNG encoding** (`raster::rgba_to_png`) — `Compression::Fast` + `FilterType::NoFilter`.
  These PNGs are transient data-URLs consumed once by the webview; encode speed matters, byte size
  doesn't.
- **Changed-pixel diff at capped resolution** (`raster::changed_grid`) — the changed-pixel mask
  caps each composite to `MAX_RASTER_DIM` right after decode, before the pixel compare. Holding
  two full-resolution composite RGBA buffers at once was a transient 2×(w·h·4) spike that stacked
  with per-layer streaming; the mask is capped to the same bound afterwards anyway, so the output
  is unchanged while peak memory roughly halves.
- **`blit`'s row-memcpy fast path** (`raster.rs`) — when a tile sits fully inside the canvas
  (the common case), one row is one `copy_from_slice` instead of a per-pixel loop with bounds
  checks.
- **No recompression of already-compressed zip entries** — `reconstruct_kra` stores (rather than
  deflates) any entry whose bytes already look compressed (`delta::looks_compressed`), since
  deflating a PNG/zstd/zip payload a second time buys nothing.
- **Deflate-fast restored files** (`kra.rs::opts`) — rebuilt tile blocks (and other
  non-compressed entries) are written with fastest deflate instead of Stored. Krita itself
  deflates layer entries; writing them uncompressed left restored working files several× larger
  on disk, which then inflated every later scan/hash/switch read (worst on HDDs).
- **zstd level by payload kind** (`delta.rs::prepare_stream`) — object snapshots use zstd
  level 1 for tile streams and anything that sniffs as already compressed (Krita tiles are
  already LZF; level 3 over them bought ~nothing while being the single largest CPU term of a
  whole-document commit), keeping level 3 only for diff-friendly text like the JSON manifests.

## Raster delivery (`kvcimg` URI scheme)

Cached diff rasters used to ship as base64 data-URLs: every layer view — even a full disk-cache
hit — paid a file read → base64 re-encode (+33% size) → a multi-MB string over Tauri IPC → V8
heap retention in the frontend caches. The desktop shell now registers a `kvcimg` URI scheme
(`lib.rs`; handler `commands::serve_raster`), and `raster::raster_url` emits plain URLs the
webview fetches directly from `.kvc/cache/` — no base64, no IPC payload, and since keys are
content-addressed and immutable the response carries `Cache-Control: immutable`, so repeat
views are browser-cache hits with zero backend work. The handler serves nothing but
`<registered repo root>/.kvc/cache/<hex key>.png` for roots the diff commands have registered
(`register_served_repo`) — it cannot be steered at arbitrary paths. Outside the shell (tests,
a failed cache write) everything falls back to the data-URL path, which is always correct.
Frontend session caches now hold tiny URL strings instead of multi-MB base64 payloads.

## Storage: composite tiling (`kra.rs::CompositePng`)

The single largest storage cost used to be invisible: every `.kra` embeds a full-canvas
`mergedimage.png`, it changes whenever any visible pixel changes, and PNG trips the
`looks_compressed` gate — so nearly every commit permanently added the whole multi-MB
composite as a fresh object. Eligible composites (8-bit RGB/RGBA, no ICC profile; an sRGB
chunk is recorded and re-emitted) are now decoded once per commit and stored as **256 px
raw-pixel blocks** (`COMPOSITE_TILE`), content-addressed like layer tiles: unchanged canvas
regions dedup across commits, and a changed block — at 256 KB, above the patch floor — bsdiffs
against its previous version at the same position. Restores reassemble the blocks and
re-encode a valid PNG (deterministic fast settings): **pixels exact, bytes not identical** to
Krita's original encoding. Anything ineligible falls back to the byte-exact `Raw` path
forever, `preview.png` stays `Raw` deliberately (tens of KB), and old manifests keep
reconstructing unchanged — the manifest is self-describing per entry.

## Storage: opt-in tile pixel deltas (`Config.tilePixelDeltas`)

A `config.json` knob (off by default; surfaced in the Settings modal as **"Compact storage for
heavily-revised art"**): when set, new commits store each tile's
**decoded planar pixels** (which bsdiff across versions — `patch_floor: 0` bypasses the 64 KB
gate for them) instead of Krita's opaque LZF payload, and restores re-encode LZF
(`raster::lzf_compress`, a hand-rolled liblzf encoder — any valid LZF stream decodes
identically, byte-parity with Krita is not required). Shrinks heavily-revised layers 2-10×
(measure with `tile_storage_experiment` in `tests/bench.rs`) at the cost of LZF decode/encode
CPU on the commit/restore paths — which is why it's opt-in on low-end hardware. The `raw`
flag lives per tile ref in the manifest, so mixed histories are fine and turning the flag off
never breaks existing raw-tiled commits.

## Storage reclamation (`gc.rs`)

Nothing on the hot path ever deletes stored data (undo and branch-delete orphan objects by
design — content-addressed orphans are harmless), so a long-lived repo only grows. The
user-facing "Clean up storage" action (`cleanup_repository`, mark-and-sweep in `gc.rs`)
reclaims everything unreachable from any branch tip **or stash** (stashes are rooted explicitly —
nothing in the commit log references them): unreachable commits leave the log, dead
chain versions leave their shards, dead loose objects are deleted, and packs are dropped
(fully dead) or rewritten with survivors only — but only when **>25 % of the pack is dead**
(`worth_rewriting`): rewriting rereads every survivor, so reclaiming a few KB from a big pack
would cost more IO than it frees, and kept dead bytes are excluded from the report. Patch
**bases are closed over** — a live patch keeps its whole chain back to the full snapshot.
State files are rewritten **before** any object is deleted, so a crash mid-sweep leaves only
re-collectable orphans, never a dangling reference. A dry-run mode powers the confirmation
dialog ("about N MB can be freed").

GC also handles three things reachability can't see: the **raster cache** is pruned to its
budget unconditionally (and wiped whole when its `.filter-version` marker mismatches
`raster::FILTER_VERSION` — the token is hashed into every key, so per-entry staleness is
unrecoverable), reported separately as `cacheBytesReclaimed`; **stale `*.tmp` files** (crash
leftovers of atomic writes, >1 h old) are swept from `.kvc/`, `chains/`, and `objects/pack/`;
and **small live packs consolidate** — ≥8 packs under 4 MB merge into one (every pack header
is parsed on index load, so dozens of small packs from mid-size commits add up).

## Caching across requests

- **Content-addressed disk cache** (`.kvc/cache/`, `raster::cache_read`/`cache_write`) — every
  capped PNG (composite or per-layer) is keyed by a hash of everything that determines its pixels
  (tile positions + hashes + dims + the resolution cap, or the composite entry's content hash).
  Keys never need invalidation; unchanged layers share one cache entry across commits and across
  the committed/working diff paths, and a repeat view — even after an app restart — skips
  reconstruct/decode/encode entirely.
- **Frontend session caches** (`repoData.ts`) — `diffCache` (commit-diff results) and
  `layerCache` (streamed layer sets) are small LRU maps (cap 20) keyed by request identity.
  Committed entries key on `path|commitId` only — commits are immutable by id, so a mutation
  (commit/rollback/undo) no longer cold-starts every previously viewed diff; only the *working*
  layer key includes the refresh `nonce`, since the working copy genuinely changes.
  Cancelled/partial layer requests are never cached (a torn-down effect's `received` map may be
  incomplete — caching it would poison the key for later visits).
- **Bounded raster cache** (`raster::cache_prune`) — `.kvc/cache/` was append-only for the life
  of the repo; it now holds a size budget (`Config.cacheMaxBytes`, default 256 MB — a config v1→v2
  migration lowers old 512 MB defaults; the Settings modal exposes it as a **"Preview cache size"**
  preset, 128 MB–2 GB). Reads touch the entry's mtime so hot entries survive, and an oldest-first prune runs
  after layer streaming, rate-limited by a marker file (`cache_prune_throttled`); "Clean up
  storage" prunes unconditionally (see Storage reclamation). A pruned entry is a regeneration,
  never an error.

## State-file writes

- **Compact JSON** (`repo.rs::write_json`) — `.kvc/*.json` is machine state, not something a human
  reads; `serde_json::to_vec` instead of `to_vec_pretty`.
- **Append-only commit log** (`repo.rs::flush_commits`) — `commits.json` was fully re-serialized
  and rewritten on every commit and grew with total history (the same class of cost the chains
  sharding removed). History now lives in `.kvc/commits.log`, JSON-lines: a normal commit is one
  O(1) append; only undo/GC (which truncate history) rewrite it. `branches.json` is written
  *after* the log, so a torn append is always an unreachable orphan record, never a dangling
  branch tip — reads drop a torn trailing line and the next save scrubs it. Legacy
  `commits.json` repos migrate on first save (the old file is then retired), mirroring the
  chains-monolith pattern.
- **Slim chain versions (`KVCC2`)** (`repo.rs::Version::object_name`) — each chain version
  stored its object filename redundantly (derivable from `hash` + `base`), duplicating a 64-hex
  hash per version forever. Bincode isn't self-describing, so the fix rides an explicit shard
  format tag: `KVCC2`-prefixed shards hold the slim shape; bare-zstd files are pre-KVCC2 and
  decode through a legacy struct, upgrading lazily when next dirtied (or all at once in GC's
  `rewrite_all`). Old monolithic `chains.bin`/`chains.json` decode through the same dual path.
- **Per-file chain shards, loaded lazily** (`repo.rs::ChainStore`) — the chains store (every
  version of every delta stream) was one monolithic file, rewritten in full on every commit and
  parsed in full on every `Repo::open`: the one cost that grew with *total repo history* instead
  of with the change at hand. It is now sharded one file per tracked file
  (`.kvc/chains/<blake3(relpath)[..16]>.bin`, same zstd-bincode encoding), faulted in on first
  touch and flushed per-dirty-shard — a commit rewrites exactly the shards of the files it
  touched, and opens parse nothing up front. Repos still carrying a monolithic `chains.bin` (or
  the older `chains.json`) are read transparently and split on their next save, which then
  retires the monolith; until that delete the monolith stays the source of truth, so a crash
  mid-split just re-runs it.
- **Sharded objects directory** (`delta.rs::write_loose`/`read_loose`) — loose objects land in
  `objects/<hash[..2]>/` (256-way fan-out) instead of one flat directory: 100k+ tiny files in a
  single folder degrade NTFS lookups and amplify Defender scans. Reads fall back to the flat
  path, so pre-sharding repos never migrate.
- **Pack-per-commit object writes** (`delta.rs::Packs`, `commit_prepared_batch`) — a batch of
  ≥32 distinct new objects (the whole-document first commit, a many-tile edit) is written as
  **one** `objects/pack/<hash>.pack` file (header + compressed index + concatenated payloads)
  instead of one file per object. Measured on Windows: the per-file *create* cost (Defender
  real-time screening, worst for low-reputation binaries like a freshly installed app) was ~28s
  of a 33s initial large-canvas commit, and parallelism can't hide it because the cost is in
  the create itself. Reads try loose (sharded, then legacy flat), then a lazily-built in-memory
  index over all pack headers with thread-safe positional reads — parallel tile reconstructs
  hit one pack concurrently. Small batches stay loose, so per-object dedup stays observable
  and tiny commits pay no pack indirection.

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

`tauri dev` runs the same image/compression hot loops as a release build. Fully unoptimized
(`opt-level = 0`, the default dev profile) they're 10-50x slower — enough to make the app feel
broken during development. Dependencies (image codecs, zstd, blake3, rayon) build at full
optimization while our own crate stays at `opt-level = 1` for tolerable rebuild times. Release
builds use thin LTO for a faster binary without paying full-LTO link times.

`blake3 = { version = "1", features = ["rayon"] }` enables the parallel hashing path described
above.

## Ceilings / deferred (ponytail)

Marked inline where they occur — noted here as a single index:

- Scanner fast path relies on the OS updating mtime on every save (Krita does); upgrade path if
  that ever breaks is git's "racy" index rule (re-hash anything whose mtime isn't strictly older
  than the last index write).
- Commit-time entry skip uses crc32+size as the change detector (~2⁻³² false-match chance per
  changed entry); upgrade path is hashing the compressed bytes instead.
- Delta patch/snapshot thresholds (64 KB, chain length 20) are untuned constants — revisit if
  storage size ever matters more than it does now.
- Raster downscaling is an area-average box filter (premultiplied-alpha) — crisp under the viewer's
  zoom. For truly pixel-accurate deep zoom the 2048px cap itself would need raising (costs cache
  disk), which is deliberately not done: storage stays flat.
- Scan byte retention (`scan::RETAIN_BUDGET`, 512 MB) bounds commit-path RAM when many large
  files change at once; files past the budget re-read on commit. Untuned constant.
- Composite tiling re-encodes `mergedimage.png` on restore — pixels exact, bytes different from
  Krita's encoding, so the entry's crc changes and the *first* commit after a restore
  re-processes the composite (all blocks dedup; only the manifest is new). Self-healing, but a
  known one-commit blip.
- Tile pixel deltas (`tilePixelDeltas`) stay opt-in until the LZF decode/encode cost is
  benchmarked as affordable on 2-core hardware; the old "decode LZF and delta raw pixels"
  upgrade-path note in `tiles.rs` is now this flag.
- Low-memory working diffs (`lowMemoryDiff`) stay opt-in: re-inflating one entry at a time bounds
  peak RAM but re-decompresses per layer, so the default in-memory path stays faster for
  interactive diffs. Only the working-tree diff view is affected; committed diffs and stored data
  are untouched.
