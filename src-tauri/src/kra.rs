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
use std::io::{Cursor, Read, Write};
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum KraEntry {
    /// A non-tiled archive entry stored as a single stream (`blob` = content hash).
    Raw {
        path: String,
        blob: String,
        stored: bool,
    },
    /// A tiled layer-data entry: header + per-tile stream references.
    Tiled {
        path: String,
        header: String,
        tiles: Vec<TileRef>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct TileRef {
    x: i64,
    y: i64,
    compression: String,
    /// content hash of this tile's stream
    hash: String,
}

/// Decompose `file_bytes` (a .kra) into streams and return the manifest's content hash.
/// Unchanged tiles dedup automatically inside [`Repo::store_stream`].
pub fn commit_kra(repo: &mut Repo, relpath: &str, file_bytes: &[u8]) -> Result<String> {
    let mut zip = ZipArchive::new(Cursor::new(file_bytes)).map_err(zip_err)?;
    let mut entries = Vec::new();

    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        drop(f);

        if tiles::is_tiled(&buf) {
            let block = tiles::parse(&buf)?;
            // Prepare every tile in parallel (the CPU cost: reconstruct base + bsdiff + verify, or
            // zstd for a fresh tile), then fold the results into the repo serially. Each tile is a
            // distinct stream key, so the parallel prepares don't race on the same chain.
            let repo_ref: &Repo = repo;
            let prepared: Vec<crate::delta::Prepared> = block
                .tiles
                .par_iter()
                .map(|t| repo_ref.prepare_stream(&tile_key(relpath, &name, t.x, t.y), &t.data))
                .collect::<Result<Vec<_>>>()?;
            let mut refs = Vec::with_capacity(block.tiles.len());
            for (t, p) in block.tiles.iter().zip(prepared) {
                let key = tile_key(relpath, &name, t.x, t.y);
                let hash = repo.commit_prepared(&key, p)?;
                refs.push(TileRef {
                    x: t.x,
                    y: t.y,
                    compression: t.compression.clone(),
                    hash,
                });
            }
            entries.push(KraEntry::Tiled {
                path: name,
                header: block.header,
                tiles: refs,
            });
        } else {
            let stored = name == "mimetype";
            let hash = repo.store_stream(&entry_key(relpath, &name), &buf)?;
            entries.push(KraEntry::Raw {
                path: name,
                blob: hash,
                stored,
            });
        }
    }

    let manifest = serde_json::to_vec(&KraManifest { entries })
        .map_err(|e| KvcError::BadIndex(e.to_string()))?;
    repo.store_stream(&manifest_key(relpath), &manifest)
}

/// Reassemble a valid .kra from a manifest version. Krita reads entries by name, so the
/// rebuilt archive is logically identical (mimetype stays first/stored, tiles uncompressed).
pub fn reconstruct_kra(repo: &Repo, relpath: &str, manifest_hash: &str) -> Result<Vec<u8>> {
    let mbytes = repo.reconstruct(&manifest_key(relpath), manifest_hash)?;
    let manifest: KraManifest =
        serde_json::from_slice(&mbytes).map_err(|e| KvcError::BadIndex(e.to_string()))?;

    let mut out = Vec::new();
    {
        let mut zw = ZipWriter::new(Cursor::new(&mut out));
        for entry in &manifest.entries {
            match entry {
                KraEntry::Raw { path, blob, stored } => {
                    zw.start_file(path.as_str(), opts(*stored))
                        .map_err(zip_err)?;
                    let bytes = repo.reconstruct(&entry_key(relpath, path), blob)?;
                    zw.write_all(&bytes)?;
                }
                KraEntry::Tiled {
                    path,
                    header,
                    tiles: refs,
                } => {
                    let mut block = TiledBlock {
                        header: header.clone(),
                        tiles: Vec::new(),
                    };
                    for tr in refs {
                        let key = tile_key(relpath, path, tr.x, tr.y);
                        let data = repo.reconstruct(&key, &tr.hash)?;
                        block.tiles.push(Tile {
                            x: tr.x,
                            y: tr.y,
                            compression: tr.compression.clone(),
                            data,
                        });
                    }
                    zw.start_file(path.as_str(), opts(true)).map_err(zip_err)?;
                    zw.write_all(&tiles::serialize(&block))?;
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
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
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
    /// Paint layers in document order (Krita writes top-first).
    pub layers: Vec<LayerNode>,
}

#[derive(Debug, Clone)]
pub struct LayerNode {
    pub name: String,
    pub uuid: String,
    pub opacity: i64,
    pub blend: String,
    /// The `layers/<filename>` data file; empty for group/non-paint layers.
    pub filename: String,
}

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
            filename: n.attribute("filename").unwrap_or("").to_string(),
        })
        .collect();
    Ok(ImageMeta {
        name: image.attribute("name").unwrap_or("").to_string(),
        width: num(&image, "width"),
        height: num(&image, "height"),
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
) -> String {
    tiles.sort_unstable();
    let mut h = blake3::Hasher::new();
    h.update(
        format!(
            "layer\0{entry_path}\0{width}x{height}\0{}",
            crate::raster::MAX_RASTER_DIM
        )
        .as_bytes(),
    );
    for (x, y, hash) in tiles.iter() {
        h.update(format!("\0{x},{y},{hash}").as_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// Cache key for a capped composite (mergedimage.png), from its content hash.
pub fn composite_cache_key(content_hash: &str) -> String {
    blake3::hash(
        format!(
            "composite\0{content_hash}\0{}",
            crate::raster::MAX_RASTER_DIM
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
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
) -> Result<Option<String>> {
    if layer_filename.is_empty() || width <= 0 || height <= 0 {
        return Ok(None);
    }
    let entry_path = format!("{image_name}/layers/{layer_filename}");
    let Some((header, refs)) = manifest.entries.iter().find_map(|e| match e {
        KraEntry::Tiled {
            path,
            header,
            tiles,
        } if *path == entry_path => Some((header, tiles)),
        _ => None,
    }) else {
        return Ok(None);
    };

    let (tw, th, ps) = tile_dims(header);
    if tw <= 0 || th <= 0 || ps != 4 {
        return Ok(None); // ponytail: RGBA8 tiles only.
    }
    let cache_dir = repo.cache_dir();
    let mut key_tiles: Vec<(i64, i64, &str)> =
        refs.iter().map(|t| (t.x, t.y, t.hash.as_str())).collect();
    let key = raster_cache_key(&entry_path, &mut key_tiles, width, height);
    if let Some(png) = crate::raster::cache_read(&cache_dir, &key) {
        return Ok(Some(crate::raster::png_bytes_to_data_url(&png)));
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
            Ok(
                crate::raster::tile_to_rgba(&data, tw as usize, th as usize, ps)
                    .map(|px| (tr.x, tr.y, px)),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    for (x, y, px) in decoded.into_iter().flatten() {
        crate::raster::blit(&mut canvas, width, height, x, y, &px, tw, th);
    }
    // Cap the raster resolution before encoding — a diff preview never needs full document pixels,
    // and full-res PNG encode was the diff's dominant cost.
    let (capped, cw, ch) = crate::raster::cap_rgba(&canvas, width as u32, height as u32);
    let png = crate::raster::rgba_to_png(&capped, cw, ch)?;
    crate::raster::cache_write(&cache_dir, &key, &png);
    Ok(Some(crate::raster::png_bytes_to_data_url(&png)))
}

/// Reconstruct a single non-tiled archive entry's raw bytes from a manifest (cheap — avoids
/// rebuilding the whole .kra). `None` if the entry isn't in the manifest.
pub fn entry_bytes(
    repo: &Repo,
    relpath: &str,
    manifest: &KraManifest,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(blob) = manifest.entries.iter().find_map(|e| match e {
        KraEntry::Raw { path, blob, .. } if path == name => Some(blob.clone()),
        _ => None,
    }) else {
        return Ok(None);
    };
    Ok(Some(repo.reconstruct(&entry_key(relpath, name), &blob)?))
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

impl KraManifest {
    /// Content hash of a non-tiled entry, if present — a cache key without reconstructing bytes.
    pub fn entry_hash(&self, name: &str) -> Option<String> {
        self.entries.iter().find_map(|e| match e {
            KraEntry::Raw { path, blob, .. } if path == name => Some(blob.clone()),
            _ => None,
        })
    }

    pub fn tile_index(&self) -> TileIndex {
        self.entries
            .iter()
            .filter_map(|e| match e {
                KraEntry::Tiled {
                    path,
                    header,
                    tiles,
                } => {
                    let (tw, th, _) = tile_dims(header);
                    let ts = tiles.iter().map(|t| (t.x, t.y, t.hash.clone())).collect();
                    Some((path.clone(), (tw, th, ts)))
                }
                _ => None,
            })
            .collect()
    }
}

/// The set of tiled layer-data entry paths whose tiles differ between two sides (added,
/// removed, or hash-changed tiles). Used to flag which layers actually changed pixels.
pub fn changed_entry_paths(old: &TileIndex, new: &TileIndex) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for (path, (_, _, tiles)) in new {
        let old_tiles: std::collections::HashMap<(i64, i64), &str> = old
            .get(path)
            .map(|(_, _, ts)| ts.iter().map(|(x, y, h)| ((*x, *y), h.as_str())).collect())
            .unwrap_or_default();
        let changed = tiles.len() != old_tiles.len()
            || tiles
                .iter()
                .any(|(x, y, h)| old_tiles.get(&(*x, *y)) != Some(&h.as_str()));
        if changed {
            out.insert(path.clone());
        }
    }
    out
}

/// One normalized (0..1) bounding box over the tiles that changed between two sides.
/// ponytail: a single union box across all layers — cheap and enough for the highlight overlay;
/// per-layer boxes if the UI ever needs them.
pub fn changed_region(
    old: &TileIndex,
    new: &TileIndex,
    width: i64,
    height: i64,
) -> Option<(f64, f64, f64, f64)> {
    if width <= 0 || height <= 0 {
        return None;
    }

    // Map each tiled entry to (x,y)->hash for the old side, then flag changed/new tiles.
    let mut min = (i64::MAX, i64::MAX);
    let mut max = (i64::MIN, i64::MIN);
    let mut seen = false;
    for (path, (tw, th, tiles)) in new {
        let old_tiles: std::collections::HashMap<(i64, i64), &str> = old
            .get(path)
            .map(|(_, _, ts)| ts.iter().map(|(x, y, h)| ((*x, *y), h.as_str())).collect())
            .unwrap_or_default();
        for (x, y, h) in tiles {
            if old_tiles.get(&(*x, *y)) != Some(&h.as_str()) {
                seen = true;
                min = (min.0.min(*x), min.1.min(*y));
                max = (max.0.max(x + tw), max.1.max(y + th));
            }
        }
    }
    if !seen {
        return None;
    }
    let x = (min.0.max(0) as f64) / width as f64;
    let y = (min.1.max(0) as f64) / height as f64;
    let w = ((max.0.min(width) - min.0.max(0)) as f64 / width as f64).clamp(0.0, 1.0);
    let h = ((max.1.min(height) - min.1.max(0)) as f64 / height as f64).clamp(0.0, 1.0);
    Some((x, y, w, h))
}

// --- working-tree .kra (in-memory, read-only diff path) --------------------------------

/// A working-tree .kra parsed once into memory. Viewing a diff must never write to the
/// store: tiles keep their raw bytes (rasters decode straight from RAM — no chain
/// reconstruct, no bsdiff, no object writes) and per-tile content hashes are computed up
/// front for change detection against a committed manifest.
pub struct WorkingKra {
    entries: Vec<WorkingEntry>,
}

enum WorkingEntry {
    Raw {
        path: String,
        bytes: Vec<u8>,
    },
    Tiled {
        path: String,
        header: String,
        tiles: Vec<Tile>,
        /// content hash per tile, parallel to `tiles`
        hashes: Vec<String>,
    },
}

/// Decompose `file_bytes` (a .kra) in memory — the read-only counterpart of [`commit_kra`].
pub fn parse_working(file_bytes: &[u8]) -> Result<WorkingKra> {
    let mut zip = ZipArchive::new(Cursor::new(file_bytes)).map_err(zip_err)?;
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        drop(f);

        if tiles::is_tiled(&buf) {
            let block = tiles::parse(&buf)?;
            let hashes = block
                .tiles
                .par_iter()
                .map(|t| crate::repo::hash_bytes(&t.data))
                .collect();
            entries.push(WorkingEntry::Tiled {
                path: name,
                header: block.header,
                tiles: block.tiles,
                hashes,
            });
        } else {
            entries.push(WorkingEntry::Raw {
                path: name,
                bytes: buf,
            });
        }
    }
    Ok(WorkingKra { entries })
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

    pub fn entry_bytes(&self, name: &str) -> Option<&[u8]> {
        self.entries.iter().find_map(|e| match e {
            WorkingEntry::Raw { path, bytes } if path == name => Some(bytes.as_slice()),
            _ => None,
        })
    }

    /// Content hash of a non-tiled entry, if present (working counterpart of
    /// [`KraManifest::entry_hash`] — the bytes are already in RAM, so hash them directly).
    pub fn entry_hash(&self, name: &str) -> Option<String> {
        self.entry_bytes(name).map(crate::repo::hash_bytes)
    }

    /// Same output as [`layer_raster`] but decoded straight from the in-memory tiles.
    /// `cache_dir` is the repo's `.kvc/cache/` — keys are content-derived, so working and
    /// committed rasters of identical pixels share entries.
    pub fn layer_raster(
        &self,
        entry_path: &str,
        width: i64,
        height: i64,
        cache_dir: &std::path::Path,
    ) -> Result<Option<String>> {
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let Some((header, tiles, hashes)) = self.entries.iter().find_map(|e| match e {
            WorkingEntry::Tiled {
                path,
                header,
                tiles,
                hashes,
            } if path == entry_path => Some((header, tiles, hashes)),
            _ => None,
        }) else {
            return Ok(None);
        };
        let (tw, th, ps) = tile_dims(header);
        if tw <= 0 || th <= 0 || ps != 4 {
            return Ok(None); // ponytail: RGBA8 tiles only.
        }
        let mut key_tiles: Vec<(i64, i64, &str)> = tiles
            .iter()
            .zip(hashes)
            .map(|(t, h)| (t.x, t.y, h.as_str()))
            .collect();
        let key = raster_cache_key(entry_path, &mut key_tiles, width, height);
        if let Some(png) = crate::raster::cache_read(cache_dir, &key) {
            return Ok(Some(crate::raster::png_bytes_to_data_url(&png)));
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
        for (x, y, px) in decoded {
            crate::raster::blit(&mut canvas, width, height, x, y, &px, tw, th);
        }
        let (capped, cw, ch) = crate::raster::cap_rgba(&canvas, width as u32, height as u32);
        let png = crate::raster::rgba_to_png(&capped, cw, ch)?;
        crate::raster::cache_write(cache_dir, &key, &png);
        Ok(Some(crate::raster::png_bytes_to_data_url(&png)))
    }
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

    pub fn entry_bytes(&self, repo: &Repo, relpath: &str, name: &str) -> Result<Option<Vec<u8>>> {
        match self {
            KraSource::Committed(m) => entry_bytes(repo, relpath, m, name),
            KraSource::Working(w) => Ok(w.entry_bytes(name).map(|b| b.to_vec())),
        }
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
    ) -> Result<Option<String>> {
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

fn opts(stored: bool) -> SimpleFileOptions {
    let method = if stored {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    SimpleFileOptions::default().compression_method(method)
}

fn zip_err(e: zip::result::ZipError) -> KvcError {
    KvcError::CorruptZip(e.to_string())
}
