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
- **Tile preparation on commit** — `commit_kra` (`kra.rs`) runs `prepare_stream` (the CPU-heavy
  reconstruct+bsdiff+verify/zstd) over every tile with `par_iter()`, then folds the results into
  the repo serially with `commit_prepared` — each tile is a distinct chain key, so parallel
  prepare can't race, only the final write needs `&mut`.
- **`.kra` reconstruction** — `reconstruct_kra` (`kra.rs`) resolves every manifest entry's bytes
  (tile-chain replay included) with `par_iter()` before writing the zip serially in manifest
  order; branch switch/rollback normally take the cheaper incremental path (`materialize_kra`,
  below) and fall back to this.
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
- **Delta-chain heuristic** (`delta.rs::looks_compressed` + a 64 KB floor) — `bsdiff` is skipped
  for small streams (a chain-walk reconstruct + suffix-sort to save a few KB isn't worth it) and
  for already-compressed payloads (PNG/zip/zstd magic — a patch against compressed bytes comes out
  near full size). Both go straight to a single-object zstd snapshot.
- **Manifest reuse** — `kra::load_manifest` reconstructs+parses a `.kra` manifest once per diff
  request; every layer/region/composite read reuses the parsed struct instead of re-walking the
  patch chain per call.
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
- **Working-tree diffs never touch the store** — `parse_working`/`WorkingKra` (`kra.rs`) decode a
  working `.kra` straight from an in-memory `ZipArchive`; no `bsdiff`, no chain reconstruct, no
  object writes. Older code staged the working file into the object store just to reuse the
  rasterizer — viewing a diff should never write.
- **`Repo::open_light`** — skips loading the chains file (by far the largest state file — every
  version of every tile stream ever committed) for read paths that never touch storage
  (`scan_repository`, `list_commits`).
- **Incremental `.kra` materialization on switch/merge/rollback** — `kra::materialize_kra`
  rebuilds the target version *out of the working file*: entries identical between the current
  and target manifests are `raw_copy_file`d from the on-disk zip (no store reads, no
  inflate/deflate; each entry verified against the manifest's recorded crc32+size first), and a
  changed tiled entry lifts its unchanged tiles from the working copy in memory — only tiles
  whose content differs replay from the object store. Switch cost tracks the diff between
  branches, not document size. Any mismatch or error falls back to the full `reconstruct_kra`.
- **Chains skipped when clean** — `Repo::save` only rewrites the chains file when a new stream
  version was actually committed (`chains_dirty`); switch, merge, and undo mutate only
  index/commits/branches, so they no longer pay an O(history) state rewrite.
- **Read-path integrity check dropped** — `reconstruct` no longer re-hashes rebuilt bytes; every
  patch is already round-trip-verified when it's *written* (`prepare_stream`), and objects are
  content-addressed, so a second hash on every read was pure redundancy on the hottest loop in the
  diff.

## Output size / encode cost

- **Raster downscaling** (`raster.rs::MAX_RASTER_DIM = 2048`) — both per-layer rasters
  (`cap_rgba`) and the composite (`cap_png`, decoding an already-encoded PNG just to re-cap it)
  are capped to a 2048px longest side before encoding. A diff preview never needs full document
  resolution (Krita canvases run into the thousands of px); this was the dominant cost in both
  encode time and the IPC payload shipped to the webview.
- **Fast PNG encoding** (`raster::rgba_to_png`) — `Compression::Fast` + `FilterType::NoFilter`.
  These PNGs are transient data-URLs consumed once by the webview; encode speed matters, byte size
  doesn't.
- **`blit`'s row-memcpy fast path** (`raster.rs`) — when a tile sits fully inside the canvas
  (the common case), one row is one `copy_from_slice` instead of a per-pixel loop with bounds
  checks.
- **No recompression of already-compressed zip entries** — `reconstruct_kra` stores (rather than
  deflates) any entry whose bytes already look compressed (`delta::looks_compressed`), since
  deflating a PNG/zstd/zip payload a second time buys nothing.

## Caching across requests

- **Content-addressed disk cache** (`.kvc/cache/`, `raster::cache_read`/`cache_write`) — every
  capped PNG (composite or per-layer) is keyed by a hash of everything that determines its pixels
  (tile positions + hashes + dims + the resolution cap, or the composite entry's content hash).
  Keys never need invalidation; unchanged layers share one cache entry across commits and across
  the committed/working diff paths, and a repeat view — even after an app restart — skips
  reconstruct/decode/encode entirely.
- **Frontend session caches** (`repoData.ts`) — `diffCache` (commit-diff results) and
  `layerCache` (streamed layer sets) are small LRU maps (cap 20) keyed by request identity
  including `nonce`, so re-visiting a commit renders instantly without re-invoking the backend.
  Cancelled/partial layer requests are never cached (a torn-down effect's `received` map may be
  incomplete — caching it would poison the key for later visits).

## State-file writes

- **Compact JSON** (`repo.rs::write_json`) — `.kvc/*.json` is machine state, not something a human
  reads; `serde_json::to_vec` instead of `to_vec_pretty`.
- **Binary chains file** (`repo.rs::write_chains`/`read_chains`) — chains are rewritten in full on
  every commit and parsed on every `Repo::open`, and dwarf the other state files, so they moved
  from JSON to zstd-compressed bincode (`chains.bin`): an order of magnitude faster to
  parse/serialize *and* several times smaller on disk. Legacy `chains.json` repos are read
  transparently and migrated by the next commit's save.

## Build configuration

`src-tauri/Cargo.toml`:

```toml
[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3
[profile.release]
lto = "thin"
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
- `.kvc/cache/` has no eviction — capped PNGs are small; add LRU pruning if that stops being true.
- Raster downscaling is nearest-neighbour, not a box filter — cheap and adequate for a scaled-down
  preview; revisit if a diff ever needs pixel-accurate zoom.
