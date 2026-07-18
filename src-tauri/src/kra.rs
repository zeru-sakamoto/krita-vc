//! `.kra` engine. A .kra is a ZIP. We decompose it into versioned streams:
//! tiled layer-data entries become one stream per tile (so an edit only stores the
//! changed 64x64 tiles), every other entry becomes one stream, and a JSON manifest
//! records how to reassemble the archive. `maindoc.xml` layer metadata is also parsed
//! and diffable for change reporting.

use crate::error::{KvcError, Result};
use crate::repo::Repo;
use crate::tiles::{self, Tile, TiledBlock};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Serialize, Deserialize)]
pub struct KraManifest {
    entries: Vec<KraEntry>,
}

/// Reconstruct + parse a manifest once. The diff path reuses one parsed `KraManifest` across all
/// layer/region/composite reads instead of re-reconstructing (walking the patch chain) per call.
pub fn load_manifest(repo: &Repo, relpath: &str, manifest_hash: &str) -> Result<KraManifest> {
    let mbytes = repo.reconstruct(&manifest_key(relpath), manifest_hash)?;
    serde_json::from_slice(&mbytes).map_err(|e| KvcError::BadIndex(e.to_string()))
}

/// [`load_manifest`] threading a caller-owned reconstruct memo (see [`Repo::reconstruct_cached`]),
/// so loading many manifest versions of the same file in one pass (GC marking) stays linear
/// instead of re-walking each version's patch chain from scratch.
pub fn load_manifest_memo(
    repo: &Repo,
    relpath: &str,
    manifest_hash: &str,
    memo: &mut std::collections::HashMap<String, Vec<u8>>,
) -> Result<KraManifest> {
    let mbytes = repo.reconstruct_cached(&manifest_key(relpath), manifest_hash, memo)?;
    serde_json::from_slice(&mbytes).map_err(|e| KvcError::BadIndex(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum KraEntry {
    /// A non-tiled archive entry stored as a single stream (`blob` = content hash).
    Raw {
        path: String,
        blob: String,
        stored: bool,
        /// zip crc32 + uncompressed size of the source entry — lets the next commit skip
        /// re-inflating an unchanged entry. 0/0 on manifests from before these fields existed
        /// (never matches, so those fall back to full processing).
        #[serde(default)]
        crc32: u32,
        #[serde(default)]
        size: u64,
    },
    /// A tiled layer-data entry: header + per-tile stream references.
    Tiled {
        path: String,
        header: String,
        tiles: Vec<TileRef>,
        #[serde(default)]
        crc32: u32,
        #[serde(default)]
        size: u64,
    },
    /// The document composite (`mergedimage.png`), stored as content-addressed raw-pixel
    /// blocks instead of one opaque PNG. The composite changes on nearly every commit, so
    /// storing it whole permanently added the full PNG per commit — the store's dominant
    /// cost; blocks dedup the unchanged canvas regions across commits (and bsdiff at the
    /// same position when they do change). Restore re-encodes a valid PNG: pixels exact,
    /// bytes not identical to Krita's original encoding.
    CompositePng {
        path: String,
        width: u32,
        height: u32,
        /// Source color type, `"rgb"` or `"rgba"` — blocks hold raw interleaved pixels in
        /// this layout and restore re-encodes the same type.
        color: String,
        /// sRGB rendering-intent byte of the source PNG, re-emitted on restore
        /// (`None` = chunk absent). Sources with an ICC profile are never tiled (see
        /// `raster::decode_png_plain`) — they stay `Raw`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        srgb: Option<u8>,
        /// blake3 of the raw pixel canvas — the composite's identity for cache keys and
        /// materialize reuse.
        pixels_hash: String,
        /// `x`/`y` are pixel offsets on the [`COMPOSITE_TILE`] grid; `compression` is `"raw"`.
        tiles: Vec<TileRef>,
        #[serde(default)]
        crc32: u32,
        #[serde(default)]
        size: u64,
    },
}

/// Composite block edge in pixels. 256×256 RGBA = 256 KB raw: coarse enough that a large
/// canvas stays a few hundred objects, fine enough that a localized edit dedups most of the
/// image — and above the 64 KB patch floor, so a changed block bsdiffs against its previous
/// version at the same position.
const COMPOSITE_TILE: u32 = 256;

/// The one archive entry that gets block-tiled. `preview.png` stays `Raw` deliberately —
/// it's tens of KB; tiling it would cost objects for no meaningful saving.
const COMPOSITE_ENTRY: &str = "mergedimage.png";

impl KraEntry {
    fn path(&self) -> &str {
        match self {
            KraEntry::Raw { path, .. }
            | KraEntry::Tiled { path, .. }
            | KraEntry::CompositePng { path, .. } => path,
        }
    }

    fn crc_size(&self) -> (u32, u64) {
        match self {
            KraEntry::Raw { crc32, size, .. }
            | KraEntry::Tiled { crc32, size, .. }
            | KraEntry::CompositePng { crc32, size, .. } => (*crc32, *size),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TileRef {
    x: i64,
    y: i64,
    compression: String,
    /// content hash of this tile's stream
    hash: String,
    /// `true`: the stream holds the tile's decoded planar *pixels* (bsdiff-able across
    /// versions; restore re-encodes LZF) — written only under `Config.tile_pixel_deltas`.
    /// `false`/absent: the stream holds the original opaque tile bytes. Interpretation is
    /// per-ref, so mixed histories are fine and restores never depend on the current flag.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    raw: bool,
}

/// Decompose `file_bytes` (a .kra) into streams and return the manifest's content hash.
/// Unchanged tiles dedup automatically inside [`Repo::store_stream`]. When `prev` (the
/// path's manifest at the current tip) is given, entries whose zip crc32 + uncompressed
/// size match the previous commit are reused verbatim without even being inflated — the
/// commit cost becomes proportional to the layers that actually changed.
pub fn commit_kra(
    repo: &mut Repo,
    relpath: &str,
    file_bytes: &[u8],
    prev: Option<&KraManifest>,
) -> Result<String> {
    let prev_by_name: std::collections::HashMap<&str, &KraEntry> = prev
        .map(|m| m.entries.iter().map(|e| (e.path(), e)).collect())
        .unwrap_or_default();
    let mut zip = ZipArchive::new(Cursor::new(file_bytes)).map_err(zip_err)?;

    // Read entries from the zip and process them in size-budgeted chunks: a chunk accumulates
    // inflated entry buffers until `RESTORE_CHUNK_BUDGET`, then prepares (parallel) + folds them
    // and drops the buffers before the next chunk reads. Peak RAM is ~one chunk of decompressed
    // entries instead of the whole document — the same bound the restore path already applies.
    // Verbatim reuses carry no buffer, so they don't count toward the budget. Entries keep their
    // original zip order across chunks (the manifest folds them in that order).
    let mut entries: Vec<KraEntry> = Vec::new();
    let mut chunk: Vec<EntryWork> = Vec::new();
    let mut chunk_bytes: u64 = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        // crc32/size come from the central directory — no decompression needed to compare.
        // Uses crc32+size as the change detector (~2^-32 false-match per changed entry);
        // upgrade path is hashing the compressed bytes.
        let (crc32, size) = (f.crc32(), f.size());
        if size > 0 {
            if let Some(pe) = prev_by_name.get(name.as_str()) {
                if pe.crc_size() == (crc32, size) {
                    chunk.push(EntryWork::Reuse((*pe).clone()));
                    drop(f);
                    continue;
                }
            }
        }
        let buf = crate::repo::read_entry_capped(&mut f)?;
        drop(f);
        chunk_bytes += buf.len() as u64;
        chunk.push(EntryWork::Fresh {
            name,
            crc32,
            size,
            buf,
        });
        if chunk_bytes >= RESTORE_CHUNK_BUDGET {
            entries.extend(flush_entry_chunk(
                repo,
                relpath,
                std::mem::take(&mut chunk),
            )?);
            chunk_bytes = 0;
        }
    }
    if !chunk.is_empty() {
        entries.extend(flush_entry_chunk(repo, relpath, chunk)?);
    }

    let manifest = serde_json::to_vec(&KraManifest { entries })
        .map_err(|e| KvcError::BadIndex(e.to_string()))?;
    repo.store_stream(&manifest_key(relpath), &manifest)
}

/// One zip entry queued for commit: a verbatim reuse of the previous manifest's entry, or a
/// freshly inflated entry still to be decomposed into streams.
enum EntryWork {
    Reuse(KraEntry),
    Fresh {
        name: String,
        crc32: u32,
        size: u64,
        buf: Vec<u8>,
    },
}

/// Prepare + fold one chunk of [`EntryWork`]: decompose every changed entry into streams in a
/// single parallel rayon pass (a multi-layer edit costs ~max(layer) instead of sum(layers) —
/// `prepare_stream` is `&self` and each stream key appears once, so parallel prepare can't race),
/// then commit the objects and push the chains serially. Returns the chunk's `KraEntry`s in order.
fn flush_entry_chunk(
    repo: &mut Repo,
    relpath: &str,
    chunk: Vec<EntryWork>,
) -> Result<Vec<KraEntry>> {
    let repo_ref: &Repo = repo;
    let prepared: Vec<(KraEntry, Vec<(String, crate::delta::Prepared)>)> = chunk
        .into_par_iter()
        .map(|w| prepare_entry_work(repo_ref, relpath, w))
        .collect::<Result<Vec<_>>>()?;
    let mut entries = Vec::with_capacity(prepared.len());
    let mut items = Vec::new();
    for (entry, mut its) in prepared {
        entries.push(entry);
        items.append(&mut its);
    }
    repo.commit_prepared_batch(items)?;
    Ok(entries)
}

/// Decompose one queued entry into its [`KraEntry`] manifest record plus the `(stream key,
/// Prepared)` pairs its content produced (tiles for a tiled layer, blocks for the composite, one
/// blob otherwise). Read-only against the repo, so it runs inside the parallel chunk pass.
fn prepare_entry_work(
    repo_ref: &Repo,
    relpath: &str,
    w: EntryWork,
) -> Result<(KraEntry, Vec<(String, crate::delta::Prepared)>)> {
    match w {
        EntryWork::Reuse(e) => Ok((e, Vec::new())),
        EntryWork::Fresh {
            name,
            crc32,
            size,
            buf,
        } => {
            if tiles::is_tiled(&buf) {
                let block = tiles::parse(&buf)?;
                drop(buf); // tiles own their bytes now — don't hold both copies
                           // Opt-in pixel deltas: store the decoded planar pixels (which
                           // bsdiff across versions) instead of the opaque LZF payload.
                           // Per-tile fallback to opaque on any decode failure.
                let planar_len = {
                    let (tw, th, ps) = tile_dims(&block.header);
                    (tw > 0 && th > 0 && ps > 0).then(|| (tw * th) as usize * ps)
                };
                let pixel_deltas = repo_ref.config.tile_pixel_deltas;
                let items: Vec<(String, crate::delta::Prepared, bool)> = block
                    .tiles
                    .par_iter()
                    .map(|t| {
                        let key = tile_key(relpath, &name, t.x, t.y);
                        if pixel_deltas {
                            if let Some(planar) =
                                planar_len.and_then(|n| crate::raster::tile_planar(&t.data, n))
                            {
                                // patch_floor 0: 16 KB planes sit under the default 64 KB gate
                                // but are the whole point — a small edit inside a tile becomes
                                // a patch.
                                let p = repo_ref.prepare_stream_opts(
                                    &key,
                                    &planar,
                                    crate::delta::StoreOpts {
                                        zstd_level: 3,
                                        patch_floor: 0,
                                    },
                                )?;
                                return Ok((key, p, true));
                            }
                        }
                        let p = repo_ref.prepare_stream(&key, &t.data)?;
                        Ok((key, p, false))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let refs = block
                    .tiles
                    .iter()
                    .zip(&items)
                    .map(|(t, (_, p, raw))| TileRef {
                        x: t.x,
                        y: t.y,
                        compression: t.compression.clone(),
                        hash: p.hash().to_string(),
                        raw: *raw,
                    })
                    .collect();
                let items = items.into_iter().map(|(k, p, _)| (k, p)).collect();
                Ok((
                    KraEntry::Tiled {
                        path: name,
                        header: block.header,
                        tiles: refs,
                        crc32,
                        size,
                    },
                    items,
                ))
            } else {
                // The composite gets block-tiled when eligible (see `prepare_composite`);
                // anything else — and any composite we can't safely re-encode — stays a
                // whole-entry Raw stream.
                if name == COMPOSITE_ENTRY {
                    if let Some(prepared) =
                        prepare_composite(repo_ref, relpath, &name, &buf, crc32, size)?
                    {
                        return Ok(prepared);
                    }
                }
                let stored = name == "mimetype";
                let key = entry_key(relpath, &name);
                let p = repo_ref.prepare_stream(&key, &buf)?;
                let blob = p.hash().to_string();
                Ok((
                    KraEntry::Raw {
                        path: name,
                        blob,
                        stored,
                        crc32,
                        size,
                    },
                    vec![(key, p)],
                ))
            }
        }
    }
}

/// Block origins covering a `w`×`h` canvas on the [`COMPOSITE_TILE`] grid.
fn composite_block_coords(w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    let mut by = 0;
    while by < h {
        let mut bx = 0;
        while bx < w {
            v.push((bx, by));
            bx += COMPOSITE_TILE;
        }
        by += COMPOSITE_TILE;
    }
    v
}

/// `(block width, block height)` for the block at `(bx, by)` — edge blocks are partial.
fn composite_block_dims(w: u32, h: u32, bx: u32, by: u32) -> (usize, usize) {
    (
        COMPOSITE_TILE.min(w - bx) as usize,
        COMPOSITE_TILE.min(h - by) as usize,
    )
}

/// Interleaved raw pixels of one block, copied row by row out of the full canvas.
fn composite_block_bytes(px: &[u8], w: u32, h: u32, bpp: usize, bx: u32, by: u32) -> Vec<u8> {
    let (bw, bh) = composite_block_dims(w, h, bx, by);
    let mut out = Vec::with_capacity(bw * bh * bpp);
    for row in 0..bh {
        let y = by as usize + row;
        let start = (y * w as usize + bx as usize) * bpp;
        out.extend_from_slice(&px[start..start + bw * bpp]);
    }
    out
}

/// Decompose the composite PNG into content-addressed raw-pixel blocks. `None` when the PNG
/// isn't eligible for lossless re-encoding (see `raster::decode_png_plain`) — the caller
/// falls back to the byte-exact Raw path.
fn prepare_composite(
    repo: &Repo,
    relpath: &str,
    name: &str,
    buf: &[u8],
    crc32: u32,
    size: u64,
) -> Result<Option<(KraEntry, Vec<(String, crate::delta::Prepared)>)>> {
    let Some((px, w, h, has_alpha, srgb)) = crate::raster::decode_png_plain(buf) else {
        return Ok(None);
    };
    let bpp = if has_alpha { 4 } else { 3 };
    let pixels_hash = crate::repo::hash_bytes(&px);
    let coords = composite_block_coords(w, h);
    let prepared: Vec<((u32, u32), (String, crate::delta::Prepared))> = coords
        .par_iter()
        .map(|&(bx, by)| {
            let block = composite_block_bytes(&px, w, h, bpp, bx, by);
            let key = tile_key(relpath, name, bx as i64, by as i64);
            let p = repo.prepare_stream(&key, &block)?;
            Ok(((bx, by), (key, p)))
        })
        .collect::<Result<Vec<_>>>()?;
    let refs = prepared
        .iter()
        .map(|((bx, by), (_, p))| TileRef {
            x: *bx as i64,
            y: *by as i64,
            compression: "raw".into(),
            hash: p.hash().to_string(),
            raw: false, // composite blocks have their own decode path; flag unused
        })
        .collect();
    let entry = KraEntry::CompositePng {
        path: name.to_string(),
        width: w,
        height: h,
        color: if has_alpha { "rgba" } else { "rgb" }.into(),
        srgb,
        pixels_hash,
        tiles: refs,
        crc32,
        size,
    };
    Ok(Some((
        entry,
        prepared.into_iter().map(|(_, kp)| kp).collect(),
    )))
}

/// Rebuild a [`KraEntry::CompositePng`]'s raw pixel canvas from its blocks (parallel
/// reconstruct, serial row copy).
fn composite_pixels(
    repo: &Repo,
    relpath: &str,
    path: &str,
    width: u32,
    height: u32,
    bpp: usize,
    refs: &[TileRef],
) -> Result<Vec<u8>> {
    let blocks: Vec<(i64, i64, Vec<u8>)> = refs
        .par_iter()
        .map(|tr| -> Result<(i64, i64, Vec<u8>)> {
            let bytes = repo.reconstruct(&tile_key(relpath, path, tr.x, tr.y), &tr.hash)?;
            Ok((tr.x, tr.y, bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut canvas = vec![0u8; width as usize * height as usize * bpp];
    for (bx, by, bytes) in blocks {
        let (bw, bh) = composite_block_dims(width, height, bx as u32, by as u32);
        for row in 0..bh {
            let y = by as usize + row;
            let dst = (y * width as usize + bx as usize) * bpp;
            let src = row * bw * bpp;
            // A tampered store could hand back a short block; bounds-check before the copy so a
            // corrupt object is a clean error, not an out-of-bounds panic.
            if src + bw * bpp > bytes.len() || dst + bw * bpp > canvas.len() {
                return Err(crate::error::KvcError::BadTiles(
                    "composite block out of bounds".into(),
                ));
            }
            canvas[dst..dst + bw * bpp].copy_from_slice(&bytes[src..src + bw * bpp]);
        }
    }
    Ok(canvas)
}

/// Re-encode a [`KraEntry::CompositePng`] back into a valid PNG (exact pixels, deterministic
/// bytes — not Krita's original encoding).
fn composite_png_bytes(repo: &Repo, relpath: &str, entry: &KraEntry) -> Result<Vec<u8>> {
    let KraEntry::CompositePng {
        path,
        width,
        height,
        color,
        srgb,
        tiles: refs,
        ..
    } = entry
    else {
        return Err(KvcError::BadIndex("not a composite entry".into()));
    };
    let has_alpha = color == "rgba";
    let bpp = if has_alpha { 4 } else { 3 };
    let px = composite_pixels(repo, relpath, path, *width, *height, bpp, refs)?;
    crate::raster::encode_composite_png(&px, *width, *height, has_alpha, *srgb)
}

/// In-flight budget for restore pipelines: entries are rebuilt (in parallel) and written to
/// the output zip in chunks of at most this many *uncompressed* bytes, so a whole-document
/// restore never holds every decompressed entry in RAM at once (previously ~2× document size
/// peak — a paging risk on 4 GB devices). Entries with unknown size (pre-crc manifests record
/// 0) count as 64 KB.
const RESTORE_CHUNK_BUDGET: u64 = 64 << 20;

/// `entry.size` with the unknown-size floor applied, for chunk budgeting.
fn budget_size(size: u64) -> u64 {
    size.max(64 * 1024)
}

/// Reassemble a valid .kra from a manifest version. Krita reads entries by name, so the
/// rebuilt archive is logically identical (mimetype stays first/stored, tiles uncompressed).
pub fn reconstruct_kra(repo: &Repo, relpath: &str, manifest_hash: &str) -> Result<Vec<u8>> {
    let manifest = load_manifest(repo, relpath, manifest_hash)?;

    // Reconstruct entries' bytes in parallel — this is the branch-switch CPU cost (delta-chain
    // replay per tile) — but in budget-bounded chunks, each written serially to the zip and
    // dropped before the next chunk builds (peak RAM = output + one chunk, not the whole doc).
    let mut out = Vec::new();
    {
        let mut zw = ZipWriter::new(Cursor::new(&mut out));
        let entries = &manifest.entries;
        let mut i = 0;
        while i < entries.len() {
            let mut j = i;
            let mut acc = 0u64;
            while j < entries.len() {
                let sz = budget_size(entries[j].crc_size().1);
                if j > i && acc + sz > RESTORE_CHUNK_BUDGET {
                    break;
                }
                acc += sz;
                j += 1;
            }
            let chunk: Vec<(&str, Vec<u8>, bool)> = entries[i..j]
                .par_iter()
                .map(|entry| -> Result<(&str, Vec<u8>, bool)> {
                    match entry {
                        KraEntry::Raw {
                            path, blob, stored, ..
                        } => {
                            let bytes = repo.reconstruct(&entry_key(relpath, path), blob)?;
                            // Already-compressed entries (previews etc.) gain ~nothing
                            // from deflate — store them and skip the recompression.
                            let stored = *stored || crate::delta::looks_compressed(&bytes);
                            Ok((path.as_str(), bytes, stored))
                        }
                        KraEntry::CompositePng { path, .. } => {
                            // Re-encoded PNG: pixels exact, stored uncompressed in the zip.
                            Ok((
                                path.as_str(),
                                composite_png_bytes(repo, relpath, entry)?,
                                true,
                            ))
                        }
                        KraEntry::Tiled {
                            path,
                            header,
                            tiles: refs,
                            ..
                        } => {
                            let tiles = refs
                                .par_iter()
                                .map(|tr| -> Result<Tile> {
                                    let key = tile_key(relpath, path, tr.x, tr.y);
                                    let data = repo.reconstruct(&key, &tr.hash)?;
                                    // Pixel-delta refs hold planar pixels — re-encode LZF.
                                    let data = if tr.raw {
                                        crate::raster::tile_from_planar(&data)
                                    } else {
                                        data
                                    };
                                    Ok(Tile {
                                        x: tr.x,
                                        y: tr.y,
                                        compression: tr.compression.clone(),
                                        data,
                                    })
                                })
                                .collect::<Result<Vec<_>>>()?;
                            let block = TiledBlock {
                                header: header.clone(),
                                tiles,
                            };
                            // Deflate-fast (see `opts`): LZF tile payloads still shrink
                            // meaningfully under deflate, and Krita writes them deflated too.
                            Ok((path.as_str(), tiles::serialize(&block), false))
                        }
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            for (path, bytes, stored) in &chunk {
                zw.start_file(*path, opts(*stored)).map_err(zip_err)?;
                zw.write_all(bytes)?;
            }
            i = j;
        }
        zw.finish().map_err(zip_err)?;
    }
    Ok(out)
}

/// Sort-normalized tile identity of a tiled entry, for order-independent comparison.
fn tile_set(ts: &[TileRef]) -> Vec<(i64, i64, &str, &str)> {
    let mut v: Vec<_> = ts
        .iter()
        .map(|t| (t.x, t.y, t.hash.as_str(), t.compression.as_str()))
        .collect();
    v.sort_unstable();
    v
}

/// Rebuild `relpath`'s bytes at manifest `target_hash`, reusing `working_bytes` — the on-disk
/// working copy, which matches the `current_hash` manifest (switch/merge/rollback run only on a
/// clean tree; each zip entry is re-verified against the manifest's recorded crc32+size anyway).
///
/// Entries identical between the two manifests are **raw-copied** out of the working zip — no
/// store reads, no inflate/deflate. A changed tiled entry lifts its unchanged tiles from the
/// working copy in memory and replays only differing tiles from the object store. The cost of a
/// branch switch becomes proportional to what actually changed, not to document size. Callers
/// fall back to the full [`reconstruct_kra`] on any error.
pub fn materialize_kra(
    repo: &Repo,
    relpath: &str,
    target_hash: &str,
    current_hash: &str,
    working_bytes: &[u8],
) -> Result<Vec<u8>> {
    use std::collections::{HashMap, HashSet};

    let target = load_manifest(repo, relpath, target_hash)?;
    let current = load_manifest(repo, relpath, current_hash)?;
    let cur_by_path: HashMap<&str, &KraEntry> =
        current.entries.iter().map(|e| (e.path(), e)).collect();
    let mut zip = ZipArchive::new(Cursor::new(working_bytes)).map_err(zip_err)?;

    /// How one target entry reaches the output zip.
    enum Plan {
        /// Identical to the working copy: raw-copy that zip entry (index into the working zip).
        Copy(usize),
        /// Rebuilt bytes (index into `built`).
        Build(usize),
    }
    enum Build<'a> {
        Raw(&'a str, &'a str, bool),
        /// Whole composite entry — rebuilt from its blocks + re-encoded as PNG.
        Composite(&'a KraEntry),
        Tiled {
            path: &'a str,
            header: &'a str,
            refs: &'a [TileRef],
            /// (x, y) -> raw tile data lifted from the working copy; only tiles absent here
            /// are reconstructed from the store.
            reuse: HashMap<(i64, i64), Vec<u8>>,
        },
    }

    // Serial classification pass (the working ZipArchive hands out entries one at a time).
    let mut plan: Vec<Plan> = Vec::with_capacity(target.entries.len());
    let mut builds: Vec<Build> = Vec::new();
    let mut build_sizes: Vec<u64> = Vec::new();
    for entry in &target.entries {
        let path = entry.path();
        let cur = cur_by_path.get(path).copied();
        // The working zip entry is trustworthy iff its crc32+size match what the current
        // manifest recorded at commit time ((0, 0) = pre-crc manifest, never trusted).
        let valid_idx = cur.and_then(|c| {
            let (crc, size) = c.crc_size();
            if (crc, size) == (0, 0) {
                return None;
            }
            let idx = zip.index_for_name(path)?;
            let f = zip.by_index_raw(idx).ok()?;
            ((f.crc32(), f.size()) == (crc, size)).then_some(idx)
        });

        let same = match (entry, cur) {
            (KraEntry::Raw { blob, .. }, Some(KraEntry::Raw { blob: cb, .. })) => blob == cb,
            (
                KraEntry::CompositePng { pixels_hash, .. },
                Some(KraEntry::CompositePng {
                    pixels_hash: cph, ..
                }),
            ) => pixels_hash == cph,
            (
                KraEntry::Tiled { header, tiles, .. },
                Some(KraEntry::Tiled {
                    header: ch,
                    tiles: ct,
                    ..
                }),
            ) => header == ch && tile_set(tiles) == tile_set(ct),
            _ => false,
        };
        if same {
            if let Some(idx) = valid_idx {
                plan.push(Plan::Copy(idx));
                continue;
            }
        }

        match entry {
            KraEntry::Raw {
                path, blob, stored, ..
            } => builds.push(Build::Raw(path, blob, *stored)),
            KraEntry::CompositePng { .. } => builds.push(Build::Composite(entry)),
            KraEntry::Tiled {
                path,
                header,
                tiles: refs,
                ..
            } => {
                let mut reuse = HashMap::new();
                if let (Some(KraEntry::Tiled { tiles: ct, .. }), Some(idx)) = (cur, valid_idx) {
                    // Reuse requires matching hash *and* matching `raw` flag — planar and
                    // opaque hashes live in different domains, so cross-domain equality
                    // never happens, but the explicit guard keeps that invariant local.
                    let cur_hash: HashMap<(i64, i64), (&str, bool)> = ct
                        .iter()
                        .map(|t| ((t.x, t.y), (t.hash.as_str(), t.raw)))
                        .collect();
                    let wanted: HashSet<(i64, i64)> = refs
                        .iter()
                        .filter(|t| cur_hash.get(&(t.x, t.y)) == Some(&(t.hash.as_str(), t.raw)))
                        .map(|t| (t.x, t.y))
                        .collect();
                    if !wanted.is_empty() {
                        let mut f = zip.by_index(idx).map_err(zip_err)?;
                        let buf = crate::repo::read_entry_capped(&mut f)?;
                        drop(f);
                        if tiles::is_tiled(&buf) {
                            for t in tiles::parse(&buf)?.tiles {
                                if wanted.contains(&(t.x, t.y)) {
                                    reuse.insert((t.x, t.y), t.data);
                                }
                            }
                        }
                    }
                }
                builds.push(Build::Tiled {
                    path,
                    header,
                    refs,
                    reuse,
                });
            }
        }
        build_sizes.push(budget_size(entry.crc_size().1));
        plan.push(Plan::Build(builds.len() - 1));
    }

    // Rebuild + write in target-manifest order: raw copies stream straight through; runs of
    // consecutive builds are rebuilt in parallel per budget-bounded chunk then written and
    // dropped (same peak-memory bound as `reconstruct_kra`). Build indices are contiguous in
    // plan order by construction.
    let build_chunk = |first: usize, last: usize| -> Result<Vec<(String, Vec<u8>, bool)>> {
        builds[first..=last]
            .par_iter()
            .map(|b| -> Result<(String, Vec<u8>, bool)> {
                match b {
                    Build::Raw(path, blob, stored) => {
                        let bytes = repo.reconstruct(&entry_key(relpath, path), blob)?;
                        let stored = *stored || crate::delta::looks_compressed(&bytes);
                        Ok((path.to_string(), bytes, stored))
                    }
                    Build::Composite(entry) => Ok((
                        entry.path().to_string(),
                        composite_png_bytes(repo, relpath, entry)?,
                        true,
                    )),
                    Build::Tiled {
                        path,
                        header,
                        refs,
                        reuse,
                    } => {
                        let tiles = refs
                            .par_iter()
                            .map(|tr| -> Result<Tile> {
                                let data = match reuse.get(&(tr.x, tr.y)) {
                                    // Lifted from the working copy: already opaque tile bytes.
                                    Some(d) => d.clone(),
                                    None => {
                                        let data = repo.reconstruct(
                                            &tile_key(relpath, path, tr.x, tr.y),
                                            &tr.hash,
                                        )?;
                                        if tr.raw {
                                            crate::raster::tile_from_planar(&data)
                                        } else {
                                            data
                                        }
                                    }
                                };
                                Ok(Tile {
                                    x: tr.x,
                                    y: tr.y,
                                    compression: tr.compression.clone(),
                                    data,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let block = TiledBlock {
                            header: (*header).to_string(),
                            tiles,
                        };
                        // Deflate-fast, matching `reconstruct_kra`'s tiled entries.
                        Ok((path.to_string(), tiles::serialize(&block), false))
                    }
                }
            })
            .collect::<Result<Vec<_>>>()
    };

    let mut out = Vec::new();
    {
        let mut zw = ZipWriter::new(Cursor::new(&mut out));
        let mut p = 0;
        while p < plan.len() {
            match &plan[p] {
                Plan::Copy(idx) => {
                    let f = zip.by_index_raw(*idx).map_err(zip_err)?;
                    zw.raw_copy_file(f).map_err(zip_err)?;
                    p += 1;
                }
                Plan::Build(first) => {
                    let first = *first;
                    let mut last = first;
                    let mut acc = build_sizes[first];
                    let mut q = p + 1;
                    while q < plan.len() {
                        let Plan::Build(bi) = &plan[q] else { break };
                        let sz = build_sizes[*bi];
                        if acc + sz > RESTORE_CHUNK_BUDGET {
                            break;
                        }
                        acc += sz;
                        last = *bi;
                        q += 1;
                    }
                    let chunk = build_chunk(first, last)?;
                    for (path, bytes, stored) in &chunk {
                        zw.start_file(path, opts(*stored)).map_err(zip_err)?;
                        zw.write_all(bytes)?;
                    }
                    p = q;
                }
            }
        }
        zw.finish().map_err(zip_err)?;
    }
    Ok(out)
}

/// Read a single entry's bytes out of a .kra archive by name.
pub fn read_entry(kra_bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(kra_bytes)).map_err(zip_err)?;
    let mut f = zip.by_name(name).map_err(zip_err)?;
    let buf = crate::repo::read_entry_capped(&mut f)?;
    Ok(buf)
}

// --- maindoc.xml layer metadata --------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LayerMeta {
    pub name: String,
    pub uuid: String,
    pub opacity: i64,
    pub blend: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDiff {
    pub name: String,
    /// "added" | "removed" | "modified"
    pub change: String,
    pub details: Vec<String>,
}

/// Flatten every `<layer>` node's tracked metadata.
pub fn parse_maindoc(xml: &[u8]) -> Result<Vec<LayerMeta>> {
    let text =
        std::str::from_utf8(xml).map_err(|_| KvcError::CorruptZip("non-utf8 maindoc".into()))?;
    // Real Krita maindoc.xml carries a `<!DOCTYPE DOC>`, which roxmltree rejects by default.
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, opts)
        .map_err(|e| KvcError::CorruptZip(format!("maindoc: {e}")))?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("layer"))
        .map(|n| {
            let a = |k: &str| n.attribute(k).unwrap_or("").to_string();
            LayerMeta {
                name: a("name"),
                uuid: a("uuid"),
                opacity: n
                    .attribute("opacity")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(255),
                blend: a("compositeop"),
                kind: a("nodetype"),
            }
        })
        .collect())
}

/// Report layer-level changes between two maindoc.xml versions (matched by uuid, name fallback).
pub fn diff_maindoc(old: &[u8], new: &[u8]) -> Result<Vec<LayerDiff>> {
    let old = parse_maindoc(old)?;
    let new = parse_maindoc(new)?;
    let id = |l: &LayerMeta| {
        if l.uuid.is_empty() {
            l.name.clone()
        } else {
            l.uuid.clone()
        }
    };

    let mut out = Vec::new();
    for n in &new {
        match old.iter().find(|o| id(o) == id(n)) {
            None => out.push(LayerDiff {
                name: n.name.clone(),
                change: "added".into(),
                details: vec![],
            }),
            Some(o) => {
                let mut details = Vec::new();
                if o.opacity != n.opacity {
                    details.push(format!("opacity {} -> {}", o.opacity, n.opacity));
                }
                if o.blend != n.blend {
                    details.push(format!("blend {} -> {}", o.blend, n.blend));
                }
                if o.name != n.name {
                    details.push(format!("renamed {} -> {}", o.name, n.name));
                }
                if !details.is_empty() {
                    out.push(LayerDiff {
                        name: n.name.clone(),
                        change: "modified".into(),
                        details,
                    });
                }
            }
        }
    }
    for o in &old {
        if !new.iter().any(|n| id(n) == id(o)) {
            out.push(LayerDiff {
                name: o.name.clone(),
                change: "removed".into(),
                details: vec![],
            });
        }
    }
    Ok(out)
}

// --- raster reconstruction (visual diff) -----------------------------------------------

/// Image + paint-layer metadata parsed from maindoc.xml, enough to rebuild per-layer rasters.
#[derive(Debug, Clone)]
pub struct ImageMeta {
    pub name: String,
    pub width: i64,
    pub height: i64,
    /// Canvas resolution (`x-res`), in dots-per-inch. 0 when absent.
    pub dpi: f64,
    /// Color space name from `<IMAGE colorspacename>` (e.g. "RGBA"). Empty when absent.
    pub color_model: String,
    /// ICC profile name from `<IMAGE profile>`. Empty when absent.
    pub color_profile: String,
    /// Paint layers in document order (Krita writes top-first).
    pub layers: Vec<LayerNode>,
}

#[derive(Debug, Clone)]
pub struct LayerNode {
    pub name: String,
    pub uuid: String,
    pub opacity: i64,
    pub blend: String,
    /// `<layer visible>` — Krita hides a layer with `visible="0"`. Defaults to visible.
    pub visible: bool,
    /// `<layer nodetype>` (e.g. "paintlayer", "grouplayer", "filterlayer"). Empty when absent.
    pub kind: String,
    /// The `layers/<filename>` data file; empty for group/non-paint layers.
    pub filename: String,
}

/// Upper bound on a parsed canvas dimension. Canvas dims come from maindoc.xml unchecked and
/// drive full `width*height*4` raster allocations; a corrupt or hostile file requesting absurd
/// dimensions would otherwise attempt an enormous allocation (Rust aborts on OOM). Krita's own
/// maximum canvas is far below this, so a real document never trips it.
const MAX_CANVAS_DIM: i64 = 32_768;

/// Parse the `<IMAGE>` element (dimensions + name) and its paint layers' data-file names.
pub fn parse_image_meta(xml: &[u8]) -> Result<ImageMeta> {
    let text =
        std::str::from_utf8(xml).map_err(|_| KvcError::CorruptZip("non-utf8 maindoc".into()))?;
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, opts)
        .map_err(|e| KvcError::CorruptZip(format!("maindoc: {e}")))?;
    let image = doc
        .descendants()
        .find(|n| n.has_tag_name("IMAGE"))
        .ok_or_else(|| KvcError::CorruptZip("maindoc: no <IMAGE>".into()))?;
    let num =
        |n: &roxmltree::Node, k: &str| n.attribute(k).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (width, height) = (num(&image, "width"), num(&image, "height"));
    if width > MAX_CANVAS_DIM || height > MAX_CANVAS_DIM {
        return Err(KvcError::CorruptZip(format!(
            "maindoc: canvas {width}x{height} exceeds the {MAX_CANVAS_DIM}px limit"
        )));
    }
    let layers = doc
        .descendants()
        .filter(|n| n.has_tag_name("layer"))
        .map(|n| LayerNode {
            name: n.attribute("name").unwrap_or("").to_string(),
            uuid: n.attribute("uuid").unwrap_or("").to_string(),
            opacity: n
                .attribute("opacity")
                .and_then(|v| v.parse().ok())
                .unwrap_or(255),
            blend: n.attribute("compositeop").unwrap_or("normal").to_string(),
            // Krita writes `visible="0"` for a hidden layer; anything else (incl. absent) = shown.
            visible: n.attribute("visible") != Some("0"),
            kind: n.attribute("nodetype").unwrap_or("").to_string(),
            filename: n.attribute("filename").unwrap_or("").to_string(),
        })
        .collect();
    Ok(ImageMeta {
        name: image.attribute("name").unwrap_or("").to_string(),
        width,
        height,
        dpi: image
            .attribute("x-res")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        color_model: image.attribute("colorspacename").unwrap_or("").to_string(),
        color_profile: image.attribute("profile").unwrap_or("").to_string(),
        layers,
    })
}

/// Parse `TILEWIDTH`/`TILEHEIGHT`/`PIXELSIZE` out of a tiled block's text header.
fn tile_dims(header: &str) -> (i64, i64, usize) {
    let get = |key: &str| -> i64 {
        header
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    };
    (
        get("TILEWIDTH "),
        get("TILEHEIGHT "),
        get("PIXELSIZE ") as usize,
    )
}

/// Cache key for one layer raster: a hash of everything that determines the output pixels —
/// the tiles (position + content hash), canvas dims, and the resolution cap. Derivable from a
/// committed manifest's refs and a working file's precomputed hashes alike, so unchanged layers
/// share one cache entry across commits and across the committed/working paths.
fn raster_cache_key(
    entry_path: &str,
    tiles: &mut [(i64, i64, &str)],
    width: i64,
    height: i64,
    default_pixel: Option<[u8; 4]>,
) -> String {
    tiles.sort_unstable();
    let mut h = blake3::Hasher::new();
    // The filter token versions the downscale filter (area-average box); bump
    // `raster::FILTER_VERSION` if resampling changes so stale PNGs are never served.
    h.update(
        format!(
            "layer\0{entry_path}\0{width}x{height}\0{}\0{}",
            crate::raster::MAX_RASTER_DIM,
            crate::raster::FILTER_VERSION
        )
        .as_bytes(),
    );
    // Fold in the default-pixel fill so a change there (with no tile record touched) still
    // invalidates the cache — see `decode_default_pixel`.
    h.update(&default_pixel.unwrap_or([0, 0, 0, 0]));
    for (x, y, hash) in tiles.iter() {
        h.update(format!("\0{x},{y},{hash}").as_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// Cache key for a capped composite (mergedimage.png), from its content hash.
pub fn composite_cache_key(content_hash: &str) -> String {
    blake3::hash(
        format!(
            "composite\0{content_hash}\0{}\0{}",
            crate::raster::MAX_RASTER_DIM,
            crate::raster::FILTER_VERSION
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// Cache key for a changed-pixel diff mask, from the before/after composite content hashes.
/// Keyed by both sides + the cap + filter token, so it's immutable and invalidates with them.
pub fn diff_cache_key(before_hash: &str, after_hash: &str) -> String {
    blake3::hash(
        format!(
            "diffmask\0{before_hash}\0{after_hash}\0{}\0{}",
            crate::raster::MAX_RASTER_DIM,
            crate::raster::FILTER_VERSION
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// A reconstructed layer raster: the webview `kvcimg://`/data URL plus the capped PNG bytes and
/// its content-addressed cache key. The bytes + key let callers diff two layer rasters (per-layer
/// change highlight) without re-decoding — the pixels are already in hand from building the URL.
pub struct LayerRaster {
    pub url: String,
    pub png: Vec<u8>,
    pub key: String,
}

/// A tiled entry only stores tiles for its painted-on regions — Krita fills everything else
/// (e.g. a freshly created, uniformly-colored layer with few or no real tiles, like a solid
/// white "Background" layer) from a sibling `<entry path>.defaultpixel` archive entry: one
/// pixel's raw bytes in the layer's native channel order (BGRA for the RGBA8 tiles this parser
/// supports). `None` for any other size — caller then keeps the transparent-zero fallback.
fn decode_default_pixel(bytes: &[u8]) -> Option<[u8; 4]> {
    if bytes.len() != 4 {
        return None;
    }
    Some([bytes[2], bytes[1], bytes[0], bytes[3]]) // BGRA -> RGBA
}

/// [`decode_default_pixel`] for a committed manifest's sibling `.defaultpixel` entry.
fn default_pixel_rgba(
    repo: &Repo,
    relpath: &str,
    manifest: &KraManifest,
    entry_path: &str,
) -> Option<[u8; 4]> {
    let name = format!("{entry_path}.defaultpixel");
    let blob = manifest.entries.iter().find_map(|e| match e {
        KraEntry::Raw { path, blob, .. } if *path == name => Some(blob.as_str()),
        _ => None,
    })?;
    let bytes = repo.reconstruct(&entry_key(relpath, &name), blob).ok()?;
    decode_default_pixel(&bytes)
}

/// Reconstruct one paint layer's pixels as a full `width`x`height` PNG data URL, or `None` if
/// the layer has no tile data / uses an unsupported colorspace.
pub fn layer_raster(
    repo: &Repo,
    relpath: &str,
    manifest: &KraManifest,
    image_name: &str,
    layer_filename: &str,
    width: i64,
    height: i64,
    cache: &crate::delta::TileCache,
) -> Result<Option<LayerRaster>> {
    if layer_filename.is_empty() || width <= 0 || height <= 0 {
        return Ok(None);
    }
    let entry_path = format!("{image_name}/layers/{layer_filename}");
    let Some((header, refs)) = manifest.entries.iter().find_map(|e| match e {
        KraEntry::Tiled {
            path,
            header,
            tiles,
            ..
        } if *path == entry_path => Some((header, tiles)),
        _ => None,
    }) else {
        return Ok(None);
    };

    let (tw, th, ps) = tile_dims(header);
    if tw <= 0 || th <= 0 || ps != 4 {
        return Ok(None); // RGBA8 tiles only.
    }
    let cache_dir = repo.cache_dir();
    let default_pixel = default_pixel_rgba(repo, relpath, manifest, &entry_path);
    let mut key_tiles: Vec<(i64, i64, &str)> =
        refs.iter().map(|t| (t.x, t.y, t.hash.as_str())).collect();
    let key = raster_cache_key(&entry_path, &mut key_tiles, width, height, default_pixel);
    if let Some(png) = crate::raster::cache_read(&cache_dir, &key) {
        let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &png);
        return Ok(Some(LayerRaster { url, png, key }));
    }
    // Reconstruct + LZF-decode tiles in parallel (nested rayon inside the per-layer par_iter is
    // fine — one work-stealing pool), then blit serially into the shared canvas.
    let decoded: Vec<Option<(i64, i64, Vec<u8>)>> = refs
        .par_iter()
        .map(|tr| -> Result<Option<(i64, i64, Vec<u8>)>> {
            let data = cache.get_or_reconstruct(
                repo,
                &tile_key(relpath, &entry_path, tr.x, tr.y),
                &tr.hash,
            )?;
            // Pixel-delta refs are already planar — skip the flag/LZF step.
            let px = if tr.raw {
                crate::raster::planar_to_rgba(&data, tw as usize, th as usize)
            } else {
                crate::raster::tile_to_rgba(&data, tw as usize, th as usize, ps)
            };
            Ok(px.map(|px| (tr.x, tr.y, px)))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    if let Some(fill) = default_pixel {
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&fill);
        }
    }
    for (x, y, px) in decoded.into_iter().flatten() {
        crate::raster::blit(&mut canvas, width, height, x, y, &px, tw, th);
    }
    // Cap the raster resolution before encoding — a diff preview never needs full document pixels,
    // and full-res PNG encode was the diff's dominant cost.
    let (capped, cw, ch) = crate::raster::cap_rgba(&canvas, width as u32, height as u32);
    let png = crate::raster::rgba_to_png(&capped, cw, ch)?;
    crate::raster::cache_write(&cache_dir, &key, &png);
    let url = crate::raster::raster_url(&repo.root, &cache_dir, &key, &png);
    Ok(Some(LayerRaster { url, png, key }))
}

/// Reconstruct a single non-tiled archive entry's raw bytes from a manifest (cheap — avoids
/// rebuilding the whole .kra). A block-tiled composite reassembles + re-encodes its PNG.
/// `None` if the entry isn't in the manifest.
pub fn entry_bytes(
    repo: &Repo,
    relpath: &str,
    manifest: &KraManifest,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(entry) = manifest.entries.iter().find(|e| e.path() == name) else {
        return Ok(None);
    };
    match entry {
        KraEntry::Raw { blob, .. } => Ok(Some(repo.reconstruct(&entry_key(relpath, name), blob)?)),
        KraEntry::CompositePng { .. } => Ok(Some(composite_png_bytes(repo, relpath, entry)?)),
        KraEntry::Tiled { .. } => Ok(None),
    }
}

/// Reconstruct a non-tiled archive entry (e.g. `mergedimage.png`) and wrap it as a PNG data URL.
pub fn entry_data_url(
    repo: &Repo,
    relpath: &str,
    manifest: &KraManifest,
    name: &str,
) -> Result<Option<String>> {
    Ok(entry_bytes(repo, relpath, manifest, name)?
        .map(|b| crate::raster::png_bytes_to_data_url(&b)))
}

/// entry path -> (tile width, tile height, [(x, y, content hash)]) for every tiled entry —
/// the common shape the change detectors below compare. Buildable from a committed manifest
/// or an in-memory working file, so both diff paths share one implementation.
pub type TileIndex = std::collections::HashMap<String, (i64, i64, Vec<(i64, i64, String)>)>;

/// Borrowed [`TileIndex`]: the hot diff path builds this instead, so a Krita-scale document
/// (thousands of tiles × 64-char hashes) never clones its hash strings just to compare them.
pub type TileIndexRef<'a> =
    std::collections::HashMap<&'a str, (i64, i64, Vec<(i64, i64, &'a str)>)>;

/// Everything one pass over two tile indexes can tell the diff: which tiled entries changed,
/// and the normalized (0..1) union bounding box of the changed tiles (`None` when nothing
/// changed or the canvas is degenerate).
pub struct TileDiff {
    pub changed_paths: std::collections::HashSet<String>,
    pub region: Option<(f64, f64, f64, f64)>,
}

/// Compare two tile indexes in one pass — the old side's per-entry `(x,y) -> hash` map is
/// built exactly once and feeds both the changed-entry set and the union region (previously
/// two functions each rebuilt it).
/// A single union box across all layers — cheap and enough for the highlight overlay;
/// per-layer boxes if the UI ever needs them.
pub fn diff_tile_indexes(
    old: &TileIndexRef,
    new: &TileIndexRef,
    width: i64,
    height: i64,
) -> TileDiff {
    let mut changed_paths = std::collections::HashSet::new();
    let mut min = (i64::MAX, i64::MAX);
    let mut max = (i64::MIN, i64::MIN);
    let mut seen = false;
    for (path, (tw, th, tiles)) in new {
        let old_tiles: std::collections::HashMap<(i64, i64), &str> = old
            .get(path)
            .map(|(_, _, ts)| ts.iter().map(|(x, y, h)| ((*x, *y), *h)).collect())
            .unwrap_or_default();
        let mut entry_changed = tiles.len() != old_tiles.len();
        for (x, y, h) in tiles {
            if old_tiles.get(&(*x, *y)) != Some(h) {
                entry_changed = true;
                seen = true;
                min = (min.0.min(*x), min.1.min(*y));
                max = (max.0.max(x + tw), max.1.max(y + th));
            }
        }
        if entry_changed {
            changed_paths.insert(path.to_string());
        }
    }
    let region = if seen && width > 0 && height > 0 {
        let x = (min.0.max(0) as f64) / width as f64;
        let y = (min.1.max(0) as f64) / height as f64;
        let w = ((max.0.min(width) - min.0.max(0)) as f64 / width as f64).clamp(0.0, 1.0);
        let h = ((max.1.min(height) - min.1.max(0)) as f64 / height as f64).clamp(0.0, 1.0);
        Some((x, y, w, h))
    } else {
        None
    };
    TileDiff {
        changed_paths,
        region,
    }
}

/// Union bounding box (in image pixels) of one layer-data entry's tiles — the layer's painted
/// area. Cheap: reads tile grid coords already in the manifest, no pixel decode; tile-granular,
/// so it slightly over-reports vs. pixel-exact content. `None` when the entry has no tiles or
/// clamps to nothing.
pub fn layer_bounds(
    index: &TileIndexRef,
    entry_path: &str,
    width: i64,
    height: i64,
) -> Option<(i64, i64, i64, i64)> {
    let (tw, th, tiles) = index.get(entry_path)?;
    if tiles.is_empty() {
        return None;
    }
    let mut min = (i64::MAX, i64::MAX);
    let mut max = (i64::MIN, i64::MIN);
    for (x, y, _h) in tiles {
        min = (min.0.min(*x), min.1.min(*y));
        max = (max.0.max(x + tw), max.1.max(y + th));
    }
    let x = min.0.max(0);
    let y = min.1.max(0);
    let w = (max.0.min(width) - x).max(0);
    let h = (max.1.min(height) - y).max(0);
    (w > 0 && h > 0).then_some((x, y, w, h))
}

/// Borrow an owned [`TileIndex`] for [`diff_tile_indexes`] (tests/back-compat wrappers).
fn borrow_index(ix: &TileIndex) -> TileIndexRef<'_> {
    ix.iter()
        .map(|(p, (tw, th, ts))| {
            (
                p.as_str(),
                (
                    *tw,
                    *th,
                    ts.iter().map(|(x, y, h)| (*x, *y, h.as_str())).collect(),
                ),
            )
        })
        .collect()
}

impl KraManifest {
    /// Content hash of a non-tiled entry, if present — a cache key without reconstructing
    /// bytes. A block-tiled composite answers with its `pixels_hash` (a different hash
    /// *domain* than raw PNG bytes, but callers only ever use this as a cache key /
    /// same-manifest identity — never for cross-domain equality).
    pub fn entry_hash(&self, name: &str) -> Option<String> {
        self.entries.iter().find_map(|e| match e {
            KraEntry::Raw { path, blob, .. } if path == name => Some(blob.clone()),
            KraEntry::CompositePng {
                path, pixels_hash, ..
            } if path == name => Some(pixels_hash.clone()),
            _ => None,
        })
    }

    /// Paths of non-tiled (`Raw`) archive entries — the only entries an embedded palette can be.
    pub fn raw_entry_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                KraEntry::Raw { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn tile_index(&self) -> TileIndex {
        self.entries
            .iter()
            .filter_map(|e| match e {
                KraEntry::Tiled {
                    path,
                    header,
                    tiles,
                    ..
                } => {
                    let (tw, th, _) = tile_dims(header);
                    let ts = tiles.iter().map(|t| (t.x, t.y, t.hash.clone())).collect();
                    Some((path.clone(), (tw, th, ts)))
                }
                _ => None,
            })
            .collect()
    }

    /// Borrowed counterpart of [`KraManifest::tile_index`] — no hash-string clones.
    pub fn tile_index_ref(&self) -> TileIndexRef<'_> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                KraEntry::Tiled {
                    path,
                    header,
                    tiles,
                    ..
                } => {
                    let (tw, th, _) = tile_dims(header);
                    let ts = tiles.iter().map(|t| (t.x, t.y, t.hash.as_str())).collect();
                    Some((path.as_str(), (tw, th, ts)))
                }
                _ => None,
            })
            .collect()
    }
}

/// The set of tiled layer-data entry paths whose tiles differ between two sides (added,
/// removed, or hash-changed tiles). Thin wrapper over [`diff_tile_indexes`] for owned indexes.
pub fn changed_entry_paths(old: &TileIndex, new: &TileIndex) -> std::collections::HashSet<String> {
    diff_tile_indexes(&borrow_index(old), &borrow_index(new), 0, 0).changed_paths
}

/// One normalized (0..1) bounding box over the tiles that changed between two sides.
/// Thin wrapper over [`diff_tile_indexes`] for owned indexes.
pub fn changed_region(
    old: &TileIndex,
    new: &TileIndex,
    width: i64,
    height: i64,
) -> Option<(f64, f64, f64, f64)> {
    diff_tile_indexes(&borrow_index(old), &borrow_index(new), width, height).region
}

// --- working-tree .kra (in-memory, read-only diff path) --------------------------------

/// A working-tree .kra parsed once into memory. Viewing a diff must never write to the
/// store: tiles keep their raw bytes (rasters decode straight from RAM — no chain
/// reconstruct, no bsdiff, no object writes) and per-tile content hashes are computed up
/// front for change detection against a committed manifest.
///
/// In **low-memory mode** (`Config.low_memory_diff`) the bulk payloads (tile data, raw entry
/// bytes) are dropped after hashing and `source` retains the compressed archive instead; each
/// entry is re-inflated on demand from `source`, so peak RAM is the compressed document plus one
/// decoded entry rather than the whole decompressed document. Metadata (headers, per-tile
/// coords + content hashes) is always retained, so change detection is identical either way.
pub struct WorkingKra {
    entries: Vec<WorkingEntry>,
    /// Compressed archive bytes, present only in low-memory mode (drives on-demand entry decode).
    source: Option<Vec<u8>>,
}

enum WorkingEntry {
    Raw {
        path: String,
        /// Empty in low-memory mode — re-read from [`WorkingKra::source`] on demand.
        bytes: Vec<u8>,
    },
    Tiled {
        path: String,
        header: String,
        /// Per-tile data is empty in low-memory mode; `x`/`y`/`compression` are always kept.
        tiles: Vec<Tile>,
        /// content hash per tile, parallel to `tiles`
        hashes: Vec<String>,
    },
}

/// Decompose `file_bytes` (a .kra) in memory — the read-only counterpart of [`commit_kra`].
/// With `low_memory`, entry payloads are dropped after hashing and re-inflated on demand.
pub fn parse_working(file_bytes: &[u8], low_memory: bool) -> Result<WorkingKra> {
    let mut zip = ZipArchive::new(Cursor::new(file_bytes)).map_err(zip_err)?;
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let buf = crate::repo::read_entry_capped(&mut f)?;
        drop(f);

        if tiles::is_tiled(&buf) {
            let block = tiles::parse(&buf)?;
            let hashes = block
                .tiles
                .par_iter()
                .map(|t| crate::repo::hash_bytes(&t.data))
                .collect();
            // Low-memory: keep tile coords + hashes but drop the (compressed) tile bytes.
            let tiles = if low_memory {
                block
                    .tiles
                    .into_iter()
                    .map(|t| Tile {
                        data: Vec::new(),
                        ..t
                    })
                    .collect()
            } else {
                block.tiles
            };
            entries.push(WorkingEntry::Tiled {
                path: name,
                header: block.header,
                tiles,
                hashes,
            });
        } else {
            entries.push(WorkingEntry::Raw {
                path: name,
                bytes: if low_memory { Vec::new() } else { buf },
            });
        }
    }
    Ok(WorkingKra {
        entries,
        source: low_memory.then(|| file_bytes.to_vec()),
    })
}

impl WorkingKra {
    pub fn tile_index(&self) -> TileIndex {
        self.entries
            .iter()
            .filter_map(|e| match e {
                WorkingEntry::Tiled {
                    path,
                    header,
                    tiles,
                    hashes,
                } => {
                    let (tw, th, _) = tile_dims(header);
                    let ts = tiles
                        .iter()
                        .zip(hashes)
                        .map(|(t, h)| (t.x, t.y, h.clone()))
                        .collect();
                    Some((path.clone(), (tw, th, ts)))
                }
                _ => None,
            })
            .collect()
    }

    /// Borrowed counterpart of [`WorkingKra::tile_index`] — no hash-string clones.
    pub fn tile_index_ref(&self) -> TileIndexRef<'_> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                WorkingEntry::Tiled {
                    path,
                    header,
                    tiles,
                    hashes,
                } => {
                    let (tw, th, _) = tile_dims(header);
                    let ts = tiles
                        .iter()
                        .zip(hashes)
                        .map(|(t, h)| (t.x, t.y, h.as_str()))
                        .collect();
                    Some((path.as_str(), (tw, th, ts)))
                }
                _ => None,
            })
            .collect()
    }

    /// Paths of non-tiled (`Raw`) archive entries — the only entries an embedded palette can be.
    pub fn raw_entry_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                WorkingEntry::Raw { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Raw bytes of a non-tiled entry: borrowed from RAM in normal mode, re-inflated from
    /// `source` (owned) in low-memory mode. `None` if `name` isn't a non-tiled entry.
    pub fn entry_bytes(&self, name: &str) -> Option<std::borrow::Cow<'_, [u8]>> {
        let is_raw = self
            .entries
            .iter()
            .any(|e| matches!(e, WorkingEntry::Raw { path, .. } if path == name));
        if !is_raw {
            return None;
        }
        match &self.source {
            Some(src) => read_entry(src, name).ok().map(std::borrow::Cow::Owned),
            None => self.entries.iter().find_map(|e| match e {
                WorkingEntry::Raw { path, bytes } if path == name => {
                    Some(std::borrow::Cow::Borrowed(bytes.as_slice()))
                }
                _ => None,
            }),
        }
    }

    /// Content hash of a non-tiled entry, if present (working counterpart of
    /// [`KraManifest::entry_hash`]).
    pub fn entry_hash(&self, name: &str) -> Option<String> {
        self.entry_bytes(name).map(|b| crate::repo::hash_bytes(&b))
    }

    /// Same output as [`layer_raster`] but decoded from the in-memory tiles (normal mode) or by
    /// re-inflating this one layer entry from `source` (low-memory mode). `cache_dir` is the
    /// repo's `.kvc/cache/` — keys are content-derived, so working and committed rasters of
    /// identical pixels share entries.
    pub fn layer_raster(
        &self,
        entry_path: &str,
        width: i64,
        height: i64,
        cache_dir: &std::path::Path,
    ) -> Result<Option<LayerRaster>> {
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let Some((header, hashes)) = self.entries.iter().find_map(|e| match e {
            WorkingEntry::Tiled {
                path,
                header,
                hashes,
                ..
            } if path == entry_path => Some((header.clone(), hashes.clone())),
            _ => None,
        }) else {
            return Ok(None);
        };
        // The repo root is two levels up from `.kvc/cache` — derived here so the
        // test-facing signature (cache_dir only) stays unchanged.
        let root = cache_dir
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(cache_dir)
            .to_path_buf();
        // See `default_pixel_rgba`'s doc comment: the sibling `.defaultpixel` entry fills
        // whatever the tile records don't cover.
        let default_pixel = self
            .entry_bytes(&format!("{entry_path}.defaultpixel"))
            .and_then(|b| decode_default_pixel(&b));
        match &self.source {
            // Normal mode: tiles carry their data — rasterize straight from RAM.
            None => {
                let tiles = self
                    .entries
                    .iter()
                    .find_map(|e| match e {
                        WorkingEntry::Tiled { path, tiles, .. } if path == entry_path => {
                            Some(tiles)
                        }
                        _ => None,
                    })
                    .expect("tiled entry located above");
                rasterize_working_tiles(
                    entry_path,
                    &header,
                    tiles,
                    &hashes,
                    width,
                    height,
                    cache_dir,
                    &root,
                    default_pixel,
                )
            }
            // Low-memory mode: re-inflate just this layer entry, then rasterize it.
            Some(src) => {
                let buf = read_entry(src, entry_path)?;
                if !tiles::is_tiled(&buf) {
                    return Ok(None);
                }
                let block = tiles::parse(&buf)?;
                rasterize_working_tiles(
                    entry_path,
                    &header,
                    &block.tiles,
                    &hashes,
                    width,
                    height,
                    cache_dir,
                    &root,
                    default_pixel,
                )
            }
        }
    }
}

/// Rasterize one working layer's tiles into a capped PNG (cache-aware). `tiles` carry their
/// decoded/compressed data; `hashes` are the parallel content hashes used only for the cache key
/// (so a cache hit never touches `tiles`' data — which is why the low-memory path can leave it out
/// until a miss forces a re-inflate).
#[allow(clippy::too_many_arguments)]
fn rasterize_working_tiles(
    entry_path: &str,
    header: &str,
    tiles: &[Tile],
    hashes: &[String],
    width: i64,
    height: i64,
    cache_dir: &std::path::Path,
    root: &std::path::Path,
    default_pixel: Option<[u8; 4]>,
) -> Result<Option<LayerRaster>> {
    let (tw, th, ps) = tile_dims(header);
    if tw <= 0 || th <= 0 || ps != 4 {
        return Ok(None); // RGBA8 tiles only.
    }
    let mut key_tiles: Vec<(i64, i64, &str)> = tiles
        .iter()
        .zip(hashes)
        .map(|(t, h)| (t.x, t.y, h.as_str()))
        .collect();
    let key = raster_cache_key(entry_path, &mut key_tiles, width, height, default_pixel);
    if let Some(png) = crate::raster::cache_read(cache_dir, &key) {
        let url = crate::raster::raster_url(root, cache_dir, &key, &png);
        return Ok(Some(LayerRaster { url, png, key }));
    }
    // LZF-decode tiles in parallel, blit serially (same pattern as the committed path).
    let decoded: Vec<(i64, i64, Vec<u8>)> = tiles
        .par_iter()
        .filter_map(|t| {
            crate::raster::tile_to_rgba(&t.data, tw as usize, th as usize, ps)
                .map(|px| (t.x, t.y, px))
        })
        .collect();
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    if let Some(fill) = default_pixel {
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&fill);
        }
    }
    for (x, y, px) in decoded {
        crate::raster::blit(&mut canvas, width, height, x, y, &px, tw, th);
    }
    let (capped, cw, ch) = crate::raster::cap_rgba(&canvas, width as u32, height as u32);
    let png = crate::raster::rgba_to_png(&capped, cw, ch)?;
    crate::raster::cache_write(cache_dir, &key, &png);
    let url = crate::raster::raster_url(root, cache_dir, &key, &png);
    Ok(Some(LayerRaster { url, png, key }))
}

/// The "new" side of an art diff: a committed manifest (tiles come from the object store) or
/// an in-memory working file (tiles already in RAM). Lets one diff builder serve both paths.
pub enum KraSource<'a> {
    Committed(&'a KraManifest),
    Working(&'a WorkingKra),
}

impl KraSource<'_> {
    pub fn tile_index(&self) -> TileIndex {
        match self {
            KraSource::Committed(m) => m.tile_index(),
            KraSource::Working(w) => w.tile_index(),
        }
    }

    /// Borrowed counterpart of [`KraSource::tile_index`] — no hash-string clones.
    pub fn tile_index_ref(&self) -> TileIndexRef<'_> {
        match self {
            KraSource::Committed(m) => m.tile_index_ref(),
            KraSource::Working(w) => w.tile_index_ref(),
        }
    }

    pub fn entry_bytes(&self, repo: &Repo, relpath: &str, name: &str) -> Result<Option<Vec<u8>>> {
        match self {
            KraSource::Committed(m) => entry_bytes(repo, relpath, m, name),
            KraSource::Working(w) => Ok(w.entry_bytes(name).map(|b| b.to_vec())),
        }
    }

    /// Names of embedded document-palette entries. Krita stores document palettes as `.kpl`
    /// blobs under `<image>/palettes/`; the `palettes/` substring plus a palette-extension
    /// check keep this robust to Krita-version path differences.
    pub fn palette_entry_names(&self) -> Vec<String> {
        let names = match self {
            KraSource::Committed(m) => m.raw_entry_names(),
            KraSource::Working(w) => w.raw_entry_names(),
        };
        names
            .into_iter()
            .filter(|n| {
                let l = n.to_lowercase();
                l.contains("palettes/") || crate::palette::is_palette(&l)
            })
            .map(str::to_string)
            .collect()
    }

    /// Content hash of a non-tiled entry, if present — the composite's cache key.
    pub fn entry_hash(&self, name: &str) -> Option<String> {
        match self {
            KraSource::Committed(m) => m.entry_hash(name),
            KraSource::Working(w) => w.entry_hash(name),
        }
    }

    pub fn layer_raster(
        &self,
        repo: &Repo,
        relpath: &str,
        image_name: &str,
        layer_filename: &str,
        width: i64,
        height: i64,
        cache: &crate::delta::TileCache,
    ) -> Result<Option<LayerRaster>> {
        match self {
            KraSource::Committed(m) => layer_raster(
                repo,
                relpath,
                m,
                image_name,
                layer_filename,
                width,
                height,
                cache,
            ),
            KraSource::Working(w) => {
                if layer_filename.is_empty() {
                    return Ok(None);
                }
                w.layer_raster(
                    &format!("{image_name}/layers/{layer_filename}"),
                    width,
                    height,
                    &repo.cache_dir(),
                )
            }
        }
    }
}

/// Every `(stream key, content hash)` a manifest version references — the GC mark set for one
/// committed `.kra`: its raw entry blobs and every tile. The manifest's own stream is the
/// caller's to add (it knows the manifest hash).
pub fn referenced_streams(relpath: &str, m: &KraManifest) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in &m.entries {
        match e {
            KraEntry::Raw { path, blob, .. } => {
                out.push((entry_key(relpath, path), blob.clone()));
            }
            // Composite blocks share the tile keyspace — every block is GC-live.
            KraEntry::Tiled { path, tiles, .. } | KraEntry::CompositePng { path, tiles, .. } => {
                for t in tiles {
                    out.push((tile_key(relpath, path, t.x, t.y), t.hash.clone()));
                }
            }
        }
    }
    out
}

/// The manifest stream's key for a `.kra` path (public for GC marking).
pub fn manifest_stream_key(relpath: &str) -> String {
    manifest_key(relpath)
}

// --- helpers ---------------------------------------------------------------------------

fn manifest_key(relpath: &str) -> String {
    format!("kra:{relpath}:manifest")
}
fn entry_key(relpath: &str, entry: &str) -> String {
    format!("kra:{relpath}:entry:{entry}")
}
fn tile_key(relpath: &str, entry: &str, x: i64, y: i64) -> String {
    format!("kra:{relpath}:tile:{entry}:{x},{y}")
}

/// Zip entry options for rebuilt archives. `stored` = no compression (the mimetype must be
/// stored; already-compressed payloads gain nothing). Everything else — including rebuilt
/// tile blocks — gets **fastest deflate**: Krita itself deflates layer entries, and writing
/// them uncompressed left restored working files several× larger on disk, which also made
/// every later scan/hash/switch read that much more (worst on HDDs).
fn opts(stored: bool) -> SimpleFileOptions {
    if stored {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    } else {
        SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(1))
    }
}

fn zip_err(e: zip::result::ZipError) -> KvcError {
    KvcError::CorruptZip(e.to_string())
}
