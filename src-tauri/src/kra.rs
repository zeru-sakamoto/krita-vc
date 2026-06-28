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
    Raw { path: String, blob: String, stored: bool },
    /// A tiled layer-data entry: header + per-tile stream references.
    Tiled { path: String, header: String, tiles: Vec<TileRef> },
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
                refs.push(TileRef { x: t.x, y: t.y, compression: t.compression.clone(), hash });
            }
            entries.push(KraEntry::Tiled { path: name, header: block.header, tiles: refs });
        } else {
            let stored = name == "mimetype";
            let hash = repo.store_stream(&entry_key(relpath, &name), &buf)?;
            entries.push(KraEntry::Raw { path: name, blob: hash, stored });
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
                    zw.start_file(path.as_str(), opts(*stored)).map_err(zip_err)?;
                    let bytes = repo.reconstruct(&entry_key(relpath, path), blob)?;
                    zw.write_all(&bytes)?;
                }
                KraEntry::Tiled { path, header, tiles: refs } => {
                    let mut block = TiledBlock { header: header.clone(), tiles: Vec::new() };
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
    let text = std::str::from_utf8(xml).map_err(|_| KvcError::CorruptZip("non-utf8 maindoc".into()))?;
    // Real Krita maindoc.xml carries a `<!DOCTYPE DOC>`, which roxmltree rejects by default.
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
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
                opacity: n.attribute("opacity").and_then(|v| v.parse().ok()).unwrap_or(255),
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
    let id = |l: &LayerMeta| if l.uuid.is_empty() { l.name.clone() } else { l.uuid.clone() };

    let mut out = Vec::new();
    for n in &new {
        match old.iter().find(|o| id(o) == id(n)) {
            None => out.push(LayerDiff { name: n.name.clone(), change: "added".into(), details: vec![] }),
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
                    out.push(LayerDiff { name: n.name.clone(), change: "modified".into(), details });
                }
            }
        }
    }
    for o in &old {
        if !new.iter().any(|n| id(n) == id(o)) {
            out.push(LayerDiff { name: o.name.clone(), change: "removed".into(), details: vec![] });
        }
    }
    Ok(out)
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
    let method = if stored { CompressionMethod::Stored } else { CompressionMethod::Deflated };
    SimpleFileOptions::default().compression_method(method)
}

fn zip_err(e: zip::result::ZipError) -> KvcError {
    KvcError::CorruptZip(e.to_string())
}
