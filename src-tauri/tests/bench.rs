//! Performance baseline for the large-canvas targets (<10s commit/switch/rollback/diff).
//! Ignored by default — run in release mode with output:
//!
//! ```text
//! cargo test --release --test bench -- --ignored --nocapture
//! ```
//!
//! Synthesizes a Krita-scale document (several layers, thousands of raw RGBA tiles of
//! incompressible pseudo-random pixels — the worst case for dedup/compression, like real
//! LZF payloads) and times every user-facing operation plus `.kvc/` disk cost.

use krita_vc_lib::{branch, commit, delta, kra, repo};
use std::io::Write;
use std::time::Instant;

// --- fixtures (mirrors tests/engine.rs — test binaries can't share helpers) -------------

const TILE_GRID: i64 = 50; // 50x50 tiles per layer = 2500 tiles, 3200x3200 px canvas
const LAYERS: usize = 3;
const EDIT_ROUNDS: usize = 10;
const EDIT_TILES: usize = 125; // ~5% of one layer per edit round

/// Deterministic LCG byte stream — incompressible, like real LZF tile payloads.
struct Rng(u64);
impl Rng {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (self.0 >> 33) as u8
            })
            .collect()
    }
}

/// A 64x64 RGBA8 tile: compression flag 0 (raw) + 4 planar channels of random pixels.
fn random_tile(rng: &mut Rng) -> Vec<u8> {
    let mut data = vec![0u8];
    data.extend(rng.bytes(64 * 64 * 4));
    data
}

/// One layer = TILE_GRID x TILE_GRID tiles.
fn layer_block(tiles: &[(i64, i64, Vec<u8>)]) -> Vec<u8> {
    let mut out = format!(
        "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA {}\n",
        tiles.len()
    )
    .into_bytes();
    for (x, y, d) in tiles {
        out.extend_from_slice(format!("{x},{y},LZF,{}\n", d.len()).as_bytes());
        out.extend_from_slice(d);
    }
    out
}

fn maindoc(layers: usize) -> Vec<u8> {
    let px = TILE_GRID * 64;
    let body: String = (0..layers)
        .map(|i| {
            format!(
                r#"<layer name="Layer {i}" uuid="l{i}" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer{i}"/>"#
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="{px}" height="{px}"><layers>{body}</layers></IMAGE></DOC>"#
    )
    .into_bytes()
}

/// Pack a .kra zip; layer entries stored (not deflated) so building versions stays fast —
/// the engine reads either, and crc32/size skip works identically.
fn pack_kra(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (name, data) in entries {
            zw.start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }
    out
}

fn full_grid(rng: &mut Rng) -> Vec<(i64, i64, Vec<u8>)> {
    let mut tiles = Vec::with_capacity((TILE_GRID * TILE_GRID) as usize);
    for ty in 0..TILE_GRID {
        for tx in 0..TILE_GRID {
            tiles.push((tx * 64, ty * 64, random_tile(rng)));
        }
    }
    tiles
}

/// Assemble the document from per-layer tile sets.
fn doc(layer_tiles: &[Vec<(i64, i64, Vec<u8>)>]) -> Vec<u8> {
    let blocks: Vec<(String, Vec<u8>)> = layer_tiles
        .iter()
        .enumerate()
        .map(|(i, t)| (format!("img/layers/layer{i}"), layer_block(t)))
        .collect();
    let md = maindoc(layer_tiles.len());
    let mut entries: Vec<(&str, &[u8])> =
        vec![("mimetype", b"application/x-krita"), ("maindoc.xml", &md)];
    for (name, block) in &blocks {
        entries.push((name.as_str(), block.as_slice()));
    }
    pack_kra(&entries)
}

fn dir_size(path: &std::path::Path) -> (u64, usize) {
    let mut bytes = 0u64;
    let mut files = 0usize;
    for e in walkdir(path) {
        bytes += e;
        files += 1;
    }
    (bytes, files)
}

fn walkdir(path: &std::path::Path) -> Vec<u64> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walkdir(&p));
            } else if let Ok(m) = e.metadata() {
                out.push(m.len());
            }
        }
    }
    out
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

/// Storage experiment for the documented tile upgrade path (decode LZF, store/delta raw
/// pixels): compares bytes/tile between the current approach (zstd over Krita's LZF bytes)
/// and zstd over decoded raw pixels, on a REAL .kra — synthetic tiles would bias it, so
/// point `KVC_BENCH_KRA` at representative art. Decision gate per the plan: build the
/// migration only if raw-pixel storage comes out ≥2x smaller.
#[test]
#[ignore = "experiment — set KVC_BENCH_KRA=<path to a real .kra> and run with --nocapture"]
fn tile_storage_experiment() {
    let Ok(path) = std::env::var("KVC_BENCH_KRA") else {
        println!("set KVC_BENCH_KRA=<path to a real .kra> to run this experiment");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let working = kra::parse_working(&bytes).unwrap();
    let index = working.tile_index();

    let (mut n, mut lzf, mut cur, mut raw_z) = (0u64, 0u64, 0u64, 0u64);
    let mut undecodable = 0u64;
    // Re-parse entries from the zip to reach the tile payloads (tile_index only carries hashes).
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().to_string();
        if !index.contains_key(&name) {
            continue;
        }
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        drop(f);
        let block = krita_vc_lib::tiles::parse(&buf).unwrap();
        let (tw, th, _) = (64usize, 64usize, 4usize); // parse header if it ever differs
        for t in &block.tiles {
            n += 1;
            lzf += t.data.len() as u64;
            cur += zstd::encode_all(&t.data[..], 3).unwrap().len() as u64;
            // stored tile = [flag][payload]; flag 1 = LZF, 0 = raw planar
            let planar = match t.data.split_first() {
                Some((1, payload)) => krita_vc_lib::raster::lzf_decompress(payload, tw * th * 4),
                Some((0, payload)) => Some(payload.to_vec()),
                _ => None,
            };
            match planar {
                Some(px) => raw_z += zstd::encode_all(&px[..], 3).unwrap().len() as u64,
                None => {
                    undecodable += 1;
                    raw_z += zstd::encode_all(&t.data[..], 3).unwrap().len() as u64;
                }
            }
        }
    }
    println!("tiles: {n} ({undecodable} undecodable, counted at current cost)");
    println!("LZF bytes (in .kra):     {:>10.2} MB", mb(lzf));
    println!("current zstd(LZF):       {:>10.2} MB", mb(cur));
    println!("candidate zstd(raw px):  {:>10.2} MB", mb(raw_z));
    println!(
        "ratio current/candidate: {:.2}x  (gate: build the migration only if >= 2.0)",
        cur as f64 / raw_z.max(1) as f64
    );
}

/// Phase breakdown of the initial (whole-document) commit, to attribute its cost.
#[test]
#[ignore = "diagnostic — run manually in release mode with --nocapture"]
fn initial_commit_phases() {
    use krita_vc_lib::tiles;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let mut rng = Rng(0x9E3779B97F4A7C15);

    let layers: Vec<Vec<(i64, i64, Vec<u8>)>> = (0..LAYERS).map(|_| full_grid(&mut rng)).collect();
    let bytes = doc(&layers);
    std::fs::write(root.join("art.kra"), &bytes).unwrap();

    let t = Instant::now();
    let changes = krita_vc_lib::scan::scan_detailed(&r, false).unwrap();
    println!(
        "scan:            {:>8.2?} ({} changes)",
        t.elapsed(),
        changes.len()
    );

    let t = Instant::now();
    let read = std::fs::read(root.join("art.kra")).unwrap();
    println!("re-read:         {:>8.2?}", t.elapsed());

    let t = Instant::now();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&read[..])).unwrap();
    let mut bufs: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
        bufs.push((name, buf));
    }
    println!("zip read:        {:>8.2?}", t.elapsed());

    let t = Instant::now();
    let blocks: Vec<(String, tiles::TiledBlock)> = bufs
        .iter()
        .filter(|(_, b)| tiles::is_tiled(b))
        .map(|(n, b)| (n.clone(), tiles::parse(b).unwrap()))
        .collect();
    let ntiles: usize = blocks.iter().map(|(_, b)| b.tiles.len()).sum();
    println!("tiles parse:     {:>8.2?} ({ntiles} tiles)", t.elapsed());

    let t = Instant::now();
    use rayon::prelude::*;
    let repo_ref: &repo::Repo = &r;
    let prepared: Vec<(String, krita_vc_lib::delta::Prepared)> = blocks
        .par_iter()
        .flat_map(|(name, block)| {
            block.tiles.par_iter().map(move |tile| {
                let key = format!("kra:art.kra:tile:{name}:{},{}", tile.x, tile.y);
                let p = repo_ref.prepare_stream(&key, &tile.data).unwrap();
                (key, p)
            })
        })
        .collect();
    println!("par prepare:     {:>8.2?}", t.elapsed());

    // Control: the same object bytes written to a sibling dir with plain parallel fs::write —
    // isolates write_object's per-file overhead from raw filesystem throughput.
    // KVC_BENCH_RAWDIR overrides the target to test other volumes/locations.
    let raw_dir = std::env::var("KVC_BENCH_RAWDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| root.join("rawtest"));
    std::fs::create_dir_all(&raw_dir).unwrap();
    let t = Instant::now();
    prepared.par_iter().for_each(|(_, p)| {
        if let krita_vc_lib::delta::Prepared::New { object, .. } = p {
            std::fs::write(raw_dir.join(&object.0), &object.1).unwrap();
        }
    });
    println!("raw par writes:  {:>8.2?}", t.elapsed());

    let t = Instant::now();
    r.commit_prepared_batch(prepared).unwrap();
    println!("batch commit:    {:>8.2?}", t.elapsed());

    let t = Instant::now();
    r.save().unwrap();
    println!("save:            {:>8.2?}", t.elapsed());
}

#[test]
#[ignore = "benchmark — run manually in release mode with --nocapture"]
fn large_canvas_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let mut rng = Rng(0x9E3779B97F4A7C15);

    // --- build the initial document -----------------------------------------------------
    let mut layers: Vec<Vec<(i64, i64, Vec<u8>)>> =
        (0..LAYERS).map(|_| full_grid(&mut rng)).collect();
    let bytes = doc(&layers);
    println!(
        "document: {} layers x {} tiles, .kra = {:.1} MB",
        LAYERS,
        TILE_GRID * TILE_GRID,
        mb(bytes.len() as u64)
    );
    std::fs::write(root.join("art.kra"), &bytes).unwrap();

    let t = Instant::now();
    let c0 = commit::commit_snapshot(&mut r, "initial", "bench").unwrap();
    println!("initial commit:      {:>8.2?}", t.elapsed());

    // --- incremental commits: each round edits ~EDIT_TILES tiles on one layer ------------
    let mut commit_ids = vec![c0.id.clone()];
    let mut total = std::time::Duration::ZERO;
    for round in 0..EDIT_ROUNDS {
        let li = round % LAYERS;
        for k in 0..EDIT_TILES {
            let idx = (round * 37 + k * 101) % layers[li].len();
            let (x, y, _) = layers[li][idx];
            layers[li][idx] = (x, y, random_tile(&mut rng));
        }
        let bytes = doc(&layers);
        std::fs::write(root.join("art.kra"), &bytes).unwrap();
        let t = Instant::now();
        let c = commit::commit_snapshot(&mut r, &format!("edit {round}"), "bench").unwrap();
        total += t.elapsed();
        commit_ids.push(c.id.clone());
    }
    println!(
        "incremental commit:  {:>8.2?}  (avg over {EDIT_ROUNDS}, ~{EDIT_TILES} tiles changed)",
        total / EDIT_ROUNDS as u32
    );

    // --- Repo::open / open_light (chains parse cost) -------------------------------------
    drop(r);
    let t = Instant::now();
    let r = repo::Repo::open(root).unwrap();
    println!("Repo::open:          {:>8.2?}", t.elapsed());
    let t = Instant::now();
    let _light = repo::Repo::open_light(root).unwrap();
    println!("Repo::open_light:    {:>8.2?}", t.elapsed());

    // --- scan on a clean tree (fast-path check) -------------------------------------------
    let t = Instant::now();
    let changes = krita_vc_lib::scan::scan(&r).unwrap();
    assert!(changes.is_empty());
    println!("clean scan:          {:>8.2?}", t.elapsed());
    let mut r = r;

    // --- diff cost: manifest load + changed-entry detection + one layer raster -----------
    let hash_of = |r: &repo::Repo, id: &str| -> String {
        r.commits
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .files
            .iter()
            .find(|f| f.path == "art.kra")
            .unwrap()
            .content
            .clone()
            .unwrap()
    };
    let last = commit_ids.last().unwrap().clone();
    let prev = commit_ids[commit_ids.len() - 2].clone();
    let (h_prev, h_last) = (hash_of(&r, &prev), hash_of(&r, &last));
    let t = Instant::now();
    let m_prev = kra::load_manifest(&r, "art.kra", &h_prev).unwrap();
    let m_last = kra::load_manifest(&r, "art.kra", &h_last).unwrap();
    let changed = kra::changed_entry_paths(&m_prev.tile_index(), &m_last.tile_index());
    println!(
        "diff detect:         {:>8.2?}  ({} entries changed)",
        t.elapsed(),
        changed.len()
    );
    let px = TILE_GRID * 64;
    let t = Instant::now();
    let url = kra::layer_raster(
        &r,
        "art.kra",
        &m_last,
        "img",
        "layer0",
        px,
        px,
        &delta::TileCache::new(),
    )
    .unwrap();
    assert!(url.is_some());
    println!("layer raster (cold): {:>8.2?}", t.elapsed());
    let t = Instant::now();
    let _ = kra::layer_raster(
        &r,
        "art.kra",
        &m_last,
        "img",
        "layer0",
        px,
        px,
        &delta::TileCache::new(),
    )
    .unwrap();
    println!("layer raster (warm): {:>8.2?}", t.elapsed());

    // --- branch switch (bounce there and back) --------------------------------------------
    branch::create_branch(&mut r, "bench-branch", None).unwrap();
    // Diverge: edit one layer, commit on the branch.
    for k in 0..EDIT_TILES {
        let idx = (k * 13) % layers[0].len();
        let (x, y, _) = layers[0][idx];
        layers[0][idx] = (x, y, random_tile(&mut rng));
    }
    std::fs::write(root.join("art.kra"), doc(&layers)).unwrap();
    commit::commit_snapshot(&mut r, "branch edit", "bench").unwrap();

    let t = Instant::now();
    branch::switch_branch(&mut r, "main").unwrap();
    println!("switch (away):       {:>8.2?}", t.elapsed());
    let t = Instant::now();
    branch::switch_branch(&mut r, "bench-branch").unwrap();
    println!("switch (back):       {:>8.2?}", t.elapsed());

    // --- rollback to an early version -----------------------------------------------------
    let t = Instant::now();
    commit::rollback_to_commit(&mut r, &commit_ids[1], "bench").unwrap();
    println!("rollback:            {:>8.2?}", t.elapsed());

    // --- full restore from store (no working-copy reuse) ----------------------------------
    let t = Instant::now();
    let rebuilt = kra::reconstruct_kra(&r, "art.kra", &h_last).unwrap();
    println!(
        "full reconstruct:    {:>8.2?}  ({:.1} MB)",
        t.elapsed(),
        mb(rebuilt.len() as u64)
    );

    // --- storage ---------------------------------------------------------------------------
    let (obj_bytes, obj_files) = dir_size(&root.join(".kvc/objects"));
    let (cache_bytes, cache_files) = dir_size(&root.join(".kvc/cache"));
    let (kvc_bytes, _) = dir_size(&root.join(".kvc"));
    println!(
        "objects/:            {:>8.1} MB in {} files",
        mb(obj_bytes),
        obj_files
    );
    println!(
        "cache/:              {:>8.1} MB in {} files",
        mb(cache_bytes),
        cache_files
    );
    let (chains_bytes, chains_files) = dir_size(&root.join(".kvc/chains"));
    println!(
        "chains/:             {:>8.1} MB in {} shards",
        mb(chains_bytes),
        chains_files
    );
    println!("total .kvc:          {:>8.1} MB", mb(kvc_bytes));
}

/// Composite (mergedimage.png) storage behavior at Krita scale: a full-canvas RGBA composite
/// rides along with every commit, but only the blocks covering the edited region should cost
/// storage. Prints per-round commit time and `.kvc` growth (the pre-tiling behavior added the
/// entire multi-MB PNG per commit).
#[test]
#[ignore = "benchmark — run manually in release mode with --nocapture"]
fn composite_commit_growth() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let mut rng = Rng(0xB105F00D);

    let px = (TILE_GRID * 64) as u32; // 3200
    let mut layer = full_grid(&mut rng);
    let mut comp = rng.bytes((px * px * 4) as usize); // full-canvas RGBA composite

    let build = |layer: &[(i64, i64, Vec<u8>)], comp: &[u8]| -> Vec<u8> {
        let png = krita_vc_lib::raster::rgba_to_png(comp, px, px).unwrap();
        let block = layer_block(layer);
        let md = maindoc(1);
        pack_kra(&[
            ("mimetype", b"application/x-krita"),
            ("maindoc.xml", &md),
            ("img/layers/layer0", &block),
            ("mergedimage.png", &png),
        ])
    };

    std::fs::write(root.join("art.kra"), build(&layer, &comp)).unwrap();
    let t = Instant::now();
    commit::commit_snapshot(&mut r, "initial", "bench").unwrap();
    let (mut prev_bytes, _) = dir_size(&root.join(".kvc"));
    println!(
        "initial commit (with {:.0} MB composite): {:>8.2?}  .kvc = {:.1} MB",
        mb((px as u64 * px as u64 * 4) as u64),
        t.elapsed(),
        mb(prev_bytes)
    );

    // Each round: edit ~EDIT_TILES tiles on the layer AND the matching composite pixels
    // (localized change — the realistic case).
    for round in 0..5 {
        for k in 0..EDIT_TILES {
            let idx = (round * 37 + k * 101) % layer.len();
            let (x, y, _) = layer[idx];
            layer[idx] = (x, y, random_tile(&mut rng));
            // Refresh the same 64x64 region of the composite.
            for row in 0..64u32 {
                let start = (((y as u32 + row) * px + x as u32) * 4) as usize;
                let fresh = rng.bytes(64 * 4);
                comp[start..start + 64 * 4].copy_from_slice(&fresh);
            }
        }
        std::fs::write(root.join("art.kra"), build(&layer, &comp)).unwrap();
        let t = Instant::now();
        commit::commit_snapshot(&mut r, &format!("edit {round}"), "bench").unwrap();
        let el = t.elapsed();
        let (now_bytes, _) = dir_size(&root.join(".kvc"));
        println!(
            "round {round}: commit {:>8.2?}  .kvc +{:.1} MB",
            el,
            mb(now_bytes.saturating_sub(prev_bytes))
        );
        prev_bytes = now_bytes;
    }

    // A full restore including the composite re-encode.
    let h = r
        .commits
        .last()
        .unwrap()
        .files
        .iter()
        .find(|f| f.path == "art.kra")
        .unwrap()
        .content
        .clone()
        .unwrap();
    let t = Instant::now();
    let rebuilt = kra::reconstruct_kra(&r, "art.kra", &h).unwrap();
    println!(
        "full reconstruct:    {:>8.2?}  ({:.1} MB)",
        t.elapsed(),
        mb(rebuilt.len() as u64)
    );
}
