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

/// Track one document in `root` and return its path. One store tracks exactly one `.kra`.
fn init_doc(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("art.kra");
    if !path.exists() {
        std::fs::write(&path, pack_kra(&[("maindoc.xml", &maindoc(1))])).unwrap();
    }
    repo::Repo::init(&path).unwrap();
    path
}

fn store_of(doc: &std::path::Path) -> std::path::PathBuf {
    repo::store_dir_for(doc)
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
    let working = kra::parse_working(&bytes, false).unwrap();
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
    let kra_path = init_doc(root);
    let mut r = repo::Repo::open(&kra_path).unwrap();
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
    let kra_path = init_doc(root);
    let mut r = repo::Repo::open(&kra_path).unwrap();
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
    let r = repo::Repo::open(&kra_path).unwrap();
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
    let (obj_bytes, obj_files) = dir_size(&store_of(&kra_path).join("objects"));
    let (cache_bytes, cache_files) = dir_size(&store_of(&kra_path).join("cache"));
    let (kvc_bytes, _) = dir_size(&store_of(&kra_path));
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
    let (chains_bytes, chains_files) = dir_size(&store_of(&kra_path).join("chains"));
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
    let kra_path = init_doc(root);
    let mut r = repo::Repo::open(&kra_path).unwrap();
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
    let (mut prev_bytes, _) = dir_size(&store_of(&kra_path));
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
        let (now_bytes, _) = dir_size(&store_of(&kra_path));
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

/// What CPU headroom costs. The engine caps its rayon pool to a percentage of logical cores
/// (`cpu::set_budget`) so a commit doesn't pin every core and stall Krita/the browser — this
/// puts a wall-clock number on that tradeoff instead of leaving it a guess.
///
/// The two dominant CPU paths are timed per budget: a first commit (zip walk → tile decompose
/// → bsdiff/zstd/blake3) and a cold layer raster (chain replay → LZF decode → downscale → PNG).
#[test]
#[ignore = "benchmark — run manually in release mode with --nocapture"]
fn cpu_budget_sweep() {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    println!("logical cores: {cores}\n");
    println!(
        "{:<10} {:>8}  {:>12}  {:>12}",
        "budget", "threads", "commit", "raster"
    );

    let mut baseline: Option<(f64, f64)> = None;
    for percent in [100u8, 75, 50] {
        krita_vc_lib::cpu::set_budget(percent);

        // A fresh repo per budget: reusing one would let the previous run's object store
        // dedup the next run's tiles away and time nothing.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let kra_path = init_doc(root);
        let mut r = repo::Repo::open(&kra_path).unwrap();
        let mut rng = Rng(0x9E3779B97F4A7C15);
        let layers: Vec<Vec<(i64, i64, Vec<u8>)>> =
            (0..LAYERS).map(|_| full_grid(&mut rng)).collect();
        std::fs::write(root.join("art.kra"), doc(&layers)).unwrap();

        // install(), not a bare call: that's how commands::run enters the pool in the real
        // app, and it's what makes the nested par_iters honour the budget.
        let t = Instant::now();
        let c = krita_vc_lib::cpu::install(|| {
            commit::commit_snapshot(&mut r, "initial", "bench").unwrap()
        });
        let commit_s = t.elapsed().as_secs_f64();

        let hash = c
            .files
            .iter()
            .find(|f| f.path == "art.kra")
            .unwrap()
            .content
            .clone()
            .unwrap();
        let m = kra::load_manifest(&r, "art.kra", &hash).unwrap();
        let px = TILE_GRID * 64;
        let t = Instant::now();
        krita_vc_lib::cpu::install(|| {
            kra::layer_raster(
                &r,
                "art.kra",
                &m,
                "img",
                "layer0",
                px,
                px,
                &delta::TileCache::new(),
            )
            .unwrap()
        });
        let raster_s = t.elapsed().as_secs_f64();

        let threads = (cores * percent as usize / 100).max(1);
        match baseline {
            None => {
                baseline = Some((commit_s, raster_s));
                println!("{percent:<10} {threads:>8}  {commit_s:>11.2}s  {raster_s:>11.2}s   (baseline)");
            }
            Some((bc, br)) => println!(
                "{percent:<10} {threads:>8}  {commit_s:>11.2}s  {raster_s:>11.2}s   ({:+.0}% / {:+.0}%)",
                (commit_s / bc - 1.0) * 100.0,
                (raster_s / br - 1.0) * 100.0
            ),
        }
    }
    // Leave the process on the shipping default rather than whatever ran last.
    krita_vc_lib::cpu::set_budget(krita_vc_lib::cpu::DEFAULT_BUDGET);
}

// --- real-art corpus ---------------------------------------------------------------------

/// Where the real-art corpus lives, overridable with `KVC_BENCH_CORPUS`. Every `*.kra` in it is
/// a real Krita document. A `foo.kra~` sibling is Krita's backup of `foo.kra` — a genuine
/// *earlier save of the same painting*, which makes the pair a real incremental edit rather
/// than a synthesized one. That matters: synthetic random tiles can't produce a realistic dedup
/// rate, and dedup is the whole storage story.
const DEFAULT_CORPUS: &str = r"D:\Storage\Krita Test Folder\performance-testing";

/// Corpus documents, smallest first so a run degrades gracefully if the big one is slow.
fn corpus_files() -> Vec<std::path::PathBuf> {
    let dir = std::env::var("KVC_BENCH_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_string());
    let mut out: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        // Extension match, not `scan::is_supported` — that deliberately rejects `*.kra~`, and
        // here the backups are wanted (as revision 1), just under a `.kra` name once copied.
        .filter(|p| p.extension().is_some_and(|e| e == "kra"))
        .collect();
    out.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
    out
}

/// The committed content hash of `rel` at `id`.
fn content_hash(r: &repo::Repo, id: &str, rel: &str) -> Option<String> {
    r.commits
        .iter()
        .find(|c| c.id == id)?
        .files
        .iter()
        .find(|f| f.path == rel)?
        .content
        .clone()
}

/// End-to-end baseline on **real** Krita documents, which is the only way the <10s targets and
/// the storage ratios mean anything — the synthetic fixture above is deliberately incompressible
/// and, at 7500 tiles, roughly 6x smaller than a real 110 MB painting.
///
/// Per document: commit revision 1 (the `.kra~` backup when present), then commit revision 2
/// (the current file) as a **real** edit, then measure the read paths and the store.
#[test]
#[ignore = "benchmark — needs a real-art corpus; run in release mode with --nocapture"]
fn corpus_baseline() {
    let files = corpus_files();
    if files.is_empty() {
        println!("no corpus found — set KVC_BENCH_CORPUS=<dir of real .kra files>");
        return;
    }
    for src in &files {
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let work = root.join(&name);

        let backup = src.with_extension("kra~");
        let rev1 = if backup.exists() { &backup } else { src };
        std::fs::copy(rev1, &work).unwrap();

        println!("\n=== {name} ===");
        println!(
            "revision 1:          {:>8.1} MB  ({})",
            mb(std::fs::metadata(&work).unwrap().len()),
            if backup.exists() {
                "from .kra~ backup"
            } else {
                "no backup — same file"
            }
        );

        repo::Repo::init(&work).unwrap();
        let mut r = repo::Repo::open(&work).unwrap();

        let t = Instant::now();
        let c0 = commit::commit_snapshot(&mut r, "revision 1", "bench").unwrap();
        println!("commit rev1:         {:>8.2?}", t.elapsed());
        let (after_first, _) = dir_size(&store_of(&work));

        // Revision 2 — the real edit, only when the backup gave us a distinct earlier state.
        let mut c1 = None;
        if backup.exists() {
            std::fs::copy(src, &work).unwrap();
            println!(
                "revision 2:          {:>8.1} MB",
                mb(std::fs::metadata(&work).unwrap().len())
            );
            let t = Instant::now();
            let c = commit::commit_snapshot(&mut r, "revision 2", "bench").unwrap();
            println!("commit rev2 (real):  {:>8.2?}", t.elapsed());
            c1 = Some(c.id.clone());
        }

        // --- read paths --------------------------------------------------------------------
        drop(r);
        let t = Instant::now();
        let mut r = repo::Repo::open(&work).unwrap();
        println!("Repo::open:          {:>8.2?}", t.elapsed());

        let t = Instant::now();
        let changes = krita_vc_lib::scan::scan(&r).unwrap();
        println!(
            "clean scan:          {:>8.2?}  ({} changes)",
            t.elapsed(),
            changes.len()
        );

        let tip = c1.clone().unwrap_or_else(|| c0.id.clone());
        if let Some(h) = content_hash(&r, &tip, &name) {
            let t = Instant::now();
            let m = kra::load_manifest(&r, &name, &h).unwrap();
            println!(
                "load manifest:       {:>8.2?}  ({} tiled entries)",
                t.elapsed(),
                m.tile_index().len()
            );

            let t = Instant::now();
            let rebuilt = kra::reconstruct_kra(&r, &name, &h).unwrap();
            println!(
                "full reconstruct:    {:>8.2?}  ({:.1} MB)",
                t.elapsed(),
                mb(rebuilt.len() as u64)
            );
        }

        // Rollback to revision 1 (only meaningful when there were two).
        if c1.is_some() {
            let t = Instant::now();
            commit::rollback_to_commit(&mut r, &c0.id, "bench").unwrap();
            println!("rollback to rev1:    {:>8.2?}", t.elapsed());
        }

        // --- storage -----------------------------------------------------------------------
        let store = store_of(&work);
        let (total, nfiles) = dir_size(&store);
        let (objects, _) = dir_size(&store.join("objects"));
        let (chains, _) = dir_size(&store.join("chains"));
        let (cache, _) = dir_size(&store.join("cache"));
        let src_bytes = std::fs::metadata(src).unwrap().len();
        let naive = if backup.exists() {
            src_bytes + std::fs::metadata(&backup).unwrap().len()
        } else {
            src_bytes
        };
        println!("store objects:       {:>8.1} MB", mb(objects));
        println!("store chains:        {:>8.1} MB", mb(chains));
        println!("store cache:         {:>8.1} MB", mb(cache));
        println!(
            "store TOTAL:         {:>8.1} MB  ({nfiles} files)",
            mb(total)
        );
        println!(
            "rev2 delta cost:     {:>8.1} MB  (store growth for the real edit)",
            mb(total.saturating_sub(after_first))
        );
        println!(
            "vs naive copies:     {:>8.1} MB  ->  {:.2}x  ({:.0}% saved)",
            mb(naive),
            naive as f64 / total.max(1) as f64,
            (1.0 - total as f64 / naive.max(1) as f64) * 100.0
        );
    }
}

/// Cost of the Performance tab's storage report, and of the per-tile chain lookup underneath it.
///
/// `stored_bytes_by_commit` resolves an object name for **every referenced stream of every
/// commit** — and `kra::referenced_streams` emits one entry per *tile*, not per layer. A real
/// 110 MB painting holds ~45k tiles, so this is (tiles x commits) lookups; the audit's question
/// was whether the `Vec<Version>` clone inside each one mattered. Times both variants over the
/// real chains so the answer is a measurement rather than an argument.
#[test]
#[ignore = "benchmark — needs a real-art corpus; run in release mode with --nocapture"]
fn storage_stats_cost() {
    let files = corpus_files();
    let Some(src) = files.iter().find(|p| p.to_string_lossy().contains("A4")) else {
        println!("no A4 document in corpus — skipping");
        return;
    };
    let name = src.file_name().unwrap().to_string_lossy().to_string();
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join(&name);
    let backup = src.with_extension("kra~");

    std::fs::copy(if backup.exists() { &backup } else { src }, &work).unwrap();
    repo::Repo::init(&work).unwrap();
    let mut r = repo::Repo::open(&work).unwrap();
    commit::commit_snapshot(&mut r, "revision 1", "bench").unwrap();
    if backup.exists() {
        std::fs::copy(src, &work).unwrap();
        commit::commit_snapshot(&mut r, "revision 2", "bench").unwrap();
    }
    drop(r);
    let r = repo::Repo::open(&work).unwrap();

    // How many lookups the report actually performs, so the per-lookup cost is interpretable.
    let mut streams = 0usize;
    for c in &r.commits {
        for f in &c.files {
            let Some(content) = &f.content else { continue };
            if f.is_kra {
                if let Ok(m) = kra::load_manifest(&r, &f.path, content) {
                    streams += kra::referenced_streams(&f.path, &m).len();
                }
            }
        }
    }
    println!(
        "commits: {}, referenced streams total: {streams}",
        r.commits.len()
    );

    // --- the two lookup variants, over every (key, hash) the report resolves ----------------
    let mut pairs: Vec<(String, String)> = Vec::new();
    for c in &r.commits {
        for f in &c.files {
            let Some(content) = &f.content else { continue };
            if f.is_kra {
                if let Ok(m) = kra::load_manifest(&r, &f.path, content) {
                    pairs.extend(kra::referenced_streams(&f.path, &m));
                }
            }
        }
    }

    let t = Instant::now();
    let mut hits = 0usize;
    for (k, h) in &pairs {
        // The pre-fix shape: clone the whole chain, then scan it.
        if r.chains
            .chain(k)
            .and_then(|c| c.iter().find(|v| &v.hash == h).map(|v| v.object_name()))
            .is_some()
        {
            hits += 1;
        }
    }
    let old = t.elapsed();
    println!("lookup via chain() clone:   {:>8.2?}  ({hits} hits)", old);

    let t = Instant::now();
    let mut hits2 = 0usize;
    for (k, h) in &pairs {
        if r.chains.object_name_of(k, h).is_some() {
            hits2 += 1;
        }
    }
    let new = t.elapsed();
    println!("lookup via object_name_of:  {:>8.2?}  ({hits2} hits)", new);
    assert_eq!(hits, hits2, "the two lookups must agree");
    println!(
        "speedup:                    {:>8.2}x",
        old.as_secs_f64() / new.as_secs_f64().max(1e-9)
    );

    // --- the whole report, as the Performance tab calls it -----------------------------------
    let t = Instant::now();
    let stats = krita_vc_lib::commands::compute_storage_stats(&r);
    println!(
        "compute_storage_stats:      {:>8.2?}  ({} versions, {:.1} MB stored)",
        t.elapsed(),
        stats.per_version.len(),
        mb(stats.actual_bytes)
    );
}

// --- layer-subset staging ----------------------------------------------------------------

/// Layers for the partial-commit fixture. More than `LAYERS` on purpose: the whole question is
/// what reverting the *unticked* ones costs, and with three layers "one ticked" is barely a
/// subset.
const PARTIAL_LAYERS: usize = 6;
/// The layer the artist **unticks** — one of the two the fixture edits, so exactly one edited
/// layer gets reverted. This is the shape the UI produces: `ChangesPanel` pre-ticks everything and
/// tracks what was turned *off*, so a real partial commit keeps most of the stack and holds one or
/// two layers back. (Ticking only one, the opposite, is the worst case for anything that tries to
/// reconstruct less than the whole committed document — noted because it is tempting to bench.)
const UNTICKED_LAYER: usize = 1;

/// Everything else. `maindoc()` stamps `uuid="l{i}"` and `stage::layer_key` prefers the uuid, so
/// these are the ids the frontend would send.
fn ticked_layers() -> Vec<String> {
    (0..PARTIAL_LAYERS)
        .filter(|i| *i != UNTICKED_LAYER)
        .map(|i| format!("l{i}"))
        .collect()
}

/// One tracked document with `PARTIAL_LAYERS` full-grid layers, committed once, then edited on
/// two layers and left dirty on disk. Returns the open repo, the document path, and the layers
/// (so a caller can keep editing).
#[allow(clippy::type_complexity)]
fn partial_fixture(
    root: &std::path::Path,
) -> (
    repo::Repo,
    std::path::PathBuf,
    Vec<Vec<(i64, i64, Vec<u8>)>>,
) {
    let kra_path = init_doc(root);
    let mut r = repo::Repo::open(&kra_path).unwrap();
    let mut rng = Rng(0x51ED_C0FF_EE15_D00D);

    let mut layers: Vec<Vec<(i64, i64, Vec<u8>)>> =
        (0..PARTIAL_LAYERS).map(|_| full_grid(&mut rng)).collect();
    std::fs::write(root.join("art.kra"), doc(&layers)).unwrap();
    commit::commit_snapshot(&mut r, "initial", "bench").unwrap();

    // Edit two layers: the ticked one and one that will be reverted, so staging has real work.
    for li in [0usize, 1usize] {
        for k in 0..EDIT_TILES {
            let idx = (li * 37 + k * 101) % layers[li].len();
            let (x, y, _) = layers[li][idx];
            layers[li][idx] = (x, y, random_tile(&mut rng));
        }
    }
    std::fs::write(root.join("art.kra"), doc(&layers)).unwrap();
    (r, kra_path, layers)
}

/// Baseline for a **layer-subset** commit (`commit_selected` with `layers`), which no other
/// benchmark exercises. Prints the whole-artwork commit of the same edit as the control — the
/// target is that saving one layer is not slower than saving all of them — then the phase
/// breakdown of the partial path, then the cost of a scan afterwards (a partial commit
/// deliberately leaves the tree dirty; the question is whether re-learning that reads the whole
/// document).
#[test]
#[ignore = "benchmark — run manually in release mode with --nocapture"]
fn partial_commit_baseline() {
    println!(
        "document: {} layers x {} tiles, 2 edited, 1 of them held back\n",
        PARTIAL_LAYERS,
        TILE_GRID * TILE_GRID
    );

    // --- control: the same edit committed whole -------------------------------------------
    let ctl_dir = tempfile::tempdir().unwrap();
    let (mut r, ctl_path, _) = partial_fixture(ctl_dir.path());
    let t = Instant::now();
    commit::commit_snapshot(&mut r, "whole", "bench").unwrap();
    let whole = t.elapsed();
    println!("whole-artwork commit:  {whole:>8.2?}   (control)");
    let t = Instant::now();
    let clean = krita_vc_lib::scan::scan(&r).unwrap();
    println!(
        "  scan after:          {:>8.2?}   ({} changes)",
        t.elapsed(),
        clean.len()
    );
    let (ctl_bytes, _) = dir_size(&store_of(&ctl_path));
    drop(r);

    // --- the partial commit ----------------------------------------------------------------
    let dir = tempfile::tempdir().unwrap();
    let (mut r, kra_path, _) = partial_fixture(dir.path());
    let t = Instant::now();
    commit::commit_selected(
        &mut r,
        "all but one layer",
        "bench",
        None,
        Some(&ticked_layers()),
    )
    .unwrap();
    let partial = t.elapsed();
    println!(
        "\npartial commit:        {partial:>8.2?}   ({:.1}x the control)",
        partial.as_secs_f64() / whole.as_secs_f64().max(f64::MIN_POSITIVE)
    );

    // A partial commit leaves the tree dirty on purpose. What it must not do is re-read and
    // re-hash the whole document to rediscover that — the Krita docker polls this every 1.5s.
    for label in ["scan after (1st)", "scan after (2nd)"] {
        let t = Instant::now();
        let changes = krita_vc_lib::scan::scan(&r).unwrap();
        println!(
            "  {label}:    {:>8.2?}   ({} changes)",
            t.elapsed(),
            changes.len()
        );
    }
    let (par_bytes, _) = dir_size(&store_of(&kra_path));
    drop(r);

    // --- phase breakdown, on a third repo in the same pre-commit state ---------------------
    let ph_dir = tempfile::tempdir().unwrap();
    let (mut r, ph_path, _) = partial_fixture(ph_dir.path());
    let working = std::fs::read(ph_dir.path().join("art.kra")).unwrap();
    let head = r
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

    println!("\nphases:");
    let keep: std::collections::HashSet<String> = ticked_layers().into_iter().collect();
    let t = Instant::now();
    let committed =
        krita_vc_lib::stage::committed_subset(&r, "art.kra", &head, &working, &keep).unwrap();
    println!(
        "  committed subset:    {:>8.2?}   ({:.1} MB rebuilt)",
        t.elapsed(),
        mb(committed.len() as u64)
    );

    // What that replaced: the whole committed document rebuilt, plus the blake3 of it that
    // `bytes_of` returned and `stage_changes` threw away.
    let t = Instant::now();
    let full = kra::reconstruct_kra(&r, "art.kra", &head).unwrap();
    let full_t = t.elapsed();
    let t = Instant::now();
    let _discarded = repo::hash_bytes(&full);
    println!(
        "   (was: reconstruct   {:>8.2?}   {:.1} MB, + {:.2?} discarded hash)",
        full_t,
        mb(full.len() as u64),
        t.elapsed()
    );
    drop(full);

    let t = Instant::now();
    let synth = krita_vc_lib::stage::stage_kra(&working, &committed, &keep).unwrap();
    println!(
        "  stage_kra:           {:>8.2?}   ({:.1} MB synthesized)",
        t.elapsed(),
        mb(synth.len() as u64)
    );

    let t = Instant::now();
    let _ = repo::hash_bytes(&synth);
    println!("  hash (synth):        {:>8.2?}", t.elapsed());

    let prev = kra::load_manifest(&r, "art.kra", &head).unwrap();
    let t = Instant::now();
    kra::commit_kra(&mut r, "art.kra", &synth, Some(&prev)).unwrap();
    println!("  commit_kra:          {:>8.2?}", t.elapsed());

    let t = Instant::now();
    r.save().unwrap();
    println!("  save:                {:>8.2?}", t.elapsed());
    drop(r);
    let _ = ph_path;

    println!(
        "\nstore after: partial {:>6.1} MB vs whole {:>6.1} MB",
        mb(par_bytes),
        mb(ctl_bytes)
    );
}
