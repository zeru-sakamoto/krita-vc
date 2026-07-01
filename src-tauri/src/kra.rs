//! `.kra` engine. A .kra is a ZIP. We decompose it into versioned streams:
//! tiled layer-data entries become one stream per tile (so an edit only stores the
//! changed 64x64 tiles), every other entry becomes one stream, and a JSON manifest
//! records how to reassemble the archive. `maindoc.xml` layer metadata is also parsed
//! and diffable for change reporting.

use crate::error::{KvcError, Result};
use crate::repo::Repo;
use crate::tiles::{self, Tile, TiledBlock};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, Serialize, Deserialize)]
struct KraManifest {
    entries: Vec<KraEntry>,
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
            let mut refs = Vec::with_capacity(block.tiles.len());
            for t in &block.tiles {
                let key = tile_key(relpath, &name, t.x, t.y);
                let hash = repo.store_stream(&key, &t.data)?;
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

/// Reconstruct one paint layer's pixels as a full `width`x`height` PNG data URL, or `None` if
/// the layer has no tile data / uses an unsupported colorspace.
pub fn layer_raster(
    repo: &Repo,
    relpath: &str,
    manifest_hash: &str,
    image_name: &str,
    layer_filename: &str,
    width: i64,
    height: i64,
) -> Result<Option<String>> {
    if layer_filename.is_empty() || width <= 0 || height <= 0 {
        return Ok(None);
    }
    let mbytes = repo.reconstruct(&manifest_key(relpath), manifest_hash)?;
    let manifest: KraManifest =
        serde_json::from_slice(&mbytes).map_err(|e| KvcError::BadIndex(e.to_string()))?;
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
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    for tr in refs {
        let data = repo.reconstruct(&tile_key(relpath, &entry_path, tr.x, tr.y), &tr.hash)?;
        if let Some(px) = crate::raster::tile_to_rgba(&data, tw as usize, th as usize, ps) {
            crate::raster::blit(&mut canvas, width, height, tr.x, tr.y, &px, tw, th);
        }
    }
    Ok(Some(crate::raster::rgba_to_png_data_url(
        &canvas,
        width as u32,
        height as u32,
    )?))
}

/// Reconstruct a single non-tiled archive entry's raw bytes from a manifest (cheap — avoids
/// rebuilding the whole .kra). `None` if the entry isn't in the manifest.
pub fn entry_bytes(
    repo: &Repo,
    relpath: &str,
    manifest_hash: &str,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let mbytes = repo.reconstruct(&manifest_key(relpath), manifest_hash)?;
    let manifest: KraManifest =
        serde_json::from_slice(&mbytes).map_err(|e| KvcError::BadIndex(e.to_string()))?;
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
    manifest_hash: &str,
    name: &str,
) -> Result<Option<String>> {
    Ok(entry_bytes(repo, relpath, manifest_hash, name)?
        .map(|b| crate::raster::png_bytes_to_data_url(&b)))
}

/// The set of tiled layer-data entry paths whose tiles differ between two manifests (added,
/// removed, or hash-changed tiles). Used to flag which layers actually changed pixels.
pub fn changed_entry_paths(
    repo: &Repo,
    relpath: &str,
    old_manifest: &str,
    new_manifest: &str,
) -> Result<std::collections::HashSet<String>> {
    let load = |h: &str| -> Result<KraManifest> {
        let b = repo.reconstruct(&manifest_key(relpath), h)?;
        serde_json::from_slice(&b).map_err(|e| KvcError::BadIndex(e.to_string()))
    };
    let (old, new) = (load(old_manifest)?, load(new_manifest)?);
    let mut out = std::collections::HashSet::new();
    for entry in &new.entries {
        let KraEntry::Tiled { path, tiles, .. } = entry else {
            continue;
        };
        let old_tiles: std::collections::HashMap<(i64, i64), &str> = old
            .entries
            .iter()
            .find_map(|e| match e {
                KraEntry::Tiled { path: p, tiles, .. } if p == path => Some(tiles),
                _ => None,
            })
            .map(|ts| ts.iter().map(|t| ((t.x, t.y), t.hash.as_str())).collect())
            .unwrap_or_default();
        let changed = tiles.len() != old_tiles.len()
            || tiles
                .iter()
                .any(|t| old_tiles.get(&(t.x, t.y)) != Some(&t.hash.as_str()));
        if changed {
            out.insert(path.clone());
        }
    }
    Ok(out)
}

/// One normalized (0..1) bounding box over the tiles that changed between two manifests.
/// ponytail: a single union box across all layers — cheap and enough for the highlight overlay;
/// per-layer boxes if the UI ever needs them.
pub fn changed_region(
    repo: &Repo,
    relpath: &str,
    old_manifest: &str,
    new_manifest: &str,
    width: i64,
    height: i64,
) -> Result<Option<(f64, f64, f64, f64)>> {
    if width <= 0 || height <= 0 {
        return Ok(None);
    }
    let load = |h: &str| -> Result<KraManifest> {
        let b = repo.reconstruct(&manifest_key(relpath), h)?;
        serde_json::from_slice(&b).map_err(|e| KvcError::BadIndex(e.to_string()))
    };
    let (old, new) = (load(old_manifest)?, load(new_manifest)?);

    // Map each tiled entry to (x,y)->hash for the old side, then flag changed/new tiles.
    let mut min = (i64::MAX, i64::MAX);
    let mut max = (i64::MIN, i64::MIN);
    let mut seen = false;
    for entry in &new.entries {
        let KraEntry::Tiled {
            path,
            header,
            tiles,
        } = entry
        else {
            continue;
        };
        let (tw, th, _) = tile_dims(header);
        let old_tiles: std::collections::HashMap<(i64, i64), &str> = old
            .entries
            .iter()
            .find_map(|e| match e {
                KraEntry::Tiled { path: p, tiles, .. } if p == path => Some(tiles),
                _ => None,
            })
            .map(|ts| ts.iter().map(|t| ((t.x, t.y), t.hash.as_str())).collect())
            .unwrap_or_default();
        for t in tiles {
            if old_tiles.get(&(t.x, t.y)) != Some(&t.hash.as_str()) {
                seen = true;
                min = (min.0.min(t.x), min.1.min(t.y));
                max = (max.0.max(t.x + tw), max.1.max(t.y + th));
            }
        }
    }
    if !seen {
        return Ok(None);
    }
    let x = (min.0.max(0) as f64) / width as f64;
    let y = (min.1.max(0) as f64) / height as f64;
    let w = ((max.0.min(width) - min.0.max(0)) as f64 / width as f64).clamp(0.0, 1.0);
    let h = ((max.1.min(height) - min.1.max(0)) as f64 / height as f64).clamp(0.0, 1.0);
    Ok(Some((x, y, w, h)))
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
