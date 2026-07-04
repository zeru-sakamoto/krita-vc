//! End-to-end engine tests against real file I/O in tempdirs (no logic mocked).

use krita_vc_lib::{
    branch, commands, commit, delta, error::KvcError, kra, raster, repo, scan, tiles,
};
use std::io::Write;

// --- fixtures --------------------------------------------------------------------------

/// Build a real Krita-style tiled layer block.
fn tiled(items: &[(i64, i64, &[u8])]) -> Vec<u8> {
    let mut out = format!(
        "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA {}\n",
        items.len()
    )
    .into_bytes();
    for (x, y, d) in items {
        out.extend_from_slice(format!("{x},{y},LZF,{}\n", d.len()).as_bytes());
        out.extend_from_slice(d);
    }
    out
}

/// Pack a minimal but valid .kra ZIP (mimetype stored first, like Krita writes it).
fn pack_kra(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for (name, data) in entries {
            let method = if *name == "mimetype" {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            zw.start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }
    out
}

fn maindoc(lines_opacity: i64) -> Vec<u8> {
    format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img"><layers>
<layer name="Background" uuid="bg" opacity="255" compositeop="normal" nodetype="paintlayer"/>
<layer name="Lines" uuid="lines" opacity="{lines_opacity}" compositeop="normal" nodetype="paintlayer"/>
</layers></IMAGE></DOC>"#
    )
    .into_bytes()
}

/// Object files across the sharded (`objects/<xx>/`) and legacy flat layouts.
fn count_objects(root: &std::path::Path) -> usize {
    fn walk(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        let p = e.path();
                        if p.is_dir() {
                            walk(&p)
                        } else {
                            1
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    walk(&root.join(".kvc/objects"))
}

// --- the critical path: delta chains reconstruct exactly -------------------------------

#[test]
fn delta_roundtrip_and_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let key = "file:test.bin";

    let mut hashes = Vec::new();
    let mut bodies = Vec::new();
    for i in 0..25u32 {
        let mut b = vec![(i % 251) as u8; 1000];
        b.extend_from_slice(&i.to_le_bytes()); // make every version distinct
        hashes.push(r.store_stream(key, &b).unwrap());
        bodies.push(b);
    }

    // Every historical version rebuilds byte-for-byte.
    for (h, body) in hashes.iter().zip(&bodies) {
        assert_eq!(&r.reconstruct(key, h).unwrap(), body);
    }

    let chain = r.chains.chain(key).unwrap();
    let max = r.config.delta_chain_max;
    assert!(
        chain.iter().all(|v| v.chain_len <= max),
        "chain exceeded threshold"
    );
    let fulls = chain.iter().filter(|v| v.base.is_none()).count();
    assert!(
        fulls >= 2,
        "threshold should force a fresh snapshot, got {fulls} fulls"
    );
}

// --- every stored version must rebuild, even on bsdiff-hostile binary data -------------

#[test]
fn random_binary_versions_all_reconstruct() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let key = "kra:art.kra:tile:layers/layer2:3200,4672";

    // Cheap deterministic LCG -> pseudo-random bytes, far more bsdiff-hostile than the
    // repetitive fixture above. Each version is a fresh random buffer (worst case for deltas).
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut rng = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 33) as u8
            })
            .collect()
    };

    let mut versions = Vec::new();
    for _ in 0..40 {
        let body = rng(4096);
        let h = r.store_stream(key, &body).unwrap();
        versions.push((h, body));
    }

    // The whole point: no stored version may fail its integrity check on reconstruct.
    for (h, body) in &versions {
        assert_eq!(
            &r.reconstruct(key, h).unwrap(),
            body,
            "version {h} failed to rebuild"
        );
    }
}

// --- patch object collision (repro) ----------------------------------------------------

#[test]
fn patches_to_same_content_from_different_bases() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    // Two streams converge on identical content from different bases.
    r.store_stream("file:a", b"aaaaaaaaaa").unwrap();
    let target = r.store_stream("file:a", b"zzzzzzzzzz").unwrap(); // a.patch vs "aaaa..."
    r.store_stream("file:b", b"bbbbbbbbbb").unwrap();
    r.store_stream("file:b", b"zzzzzzzzzz").unwrap(); // same result, base "bbbb..."

    // b's version must rebuild its own content, not a's patch applied to b's base.
    assert_eq!(r.reconstruct("file:b", &target).unwrap(), b"zzzzzzzzzz");
}

// --- tiles -----------------------------------------------------------------------------

#[test]
fn tiles_roundtrip_and_change_detection() {
    let a = tiled(&[(0, 0, b"AAAA"), (0, 64, b"BBBB")]);
    let block = tiles::parse(&a).unwrap();
    assert_eq!(block.tiles.len(), 2);
    assert_eq!(
        tiles::serialize(&block),
        a,
        "tile serialize must round-trip exactly"
    );

    let b = tiled(&[(0, 0, b"AAAA"), (0, 64, b"CCCC")]);
    let block_b = tiles::parse(&b).unwrap();
    assert_eq!(
        block.tiles[0].data, block_b.tiles[0].data,
        "unchanged tile stays equal"
    );
    assert_ne!(
        block.tiles[1].data, block_b.tiles[1].data,
        "changed tile differs"
    );
}

// --- .kra engine: tile-level dedup + exact restore ------------------------------------

#[test]
fn kra_tile_dedup_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let kra1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, b"tileAAAA"), (0, 64, b"tileBBBB")]),
        ),
    ]);
    std::fs::write(root.join("art.kra"), &kra1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "tester").unwrap();
    let objs1 = count_objects(root);

    // Edit exactly one tile.
    let kra2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, b"tileAAAA"), (0, 64, b"tileZZZZ")]),
        ),
    ]);
    std::fs::write(root.join("art.kra"), &kra2).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "tester").unwrap();
    let objs2 = count_objects(root);

    // Only the changed tile + a new manifest are stored; the rest dedups.
    assert_eq!(
        objs2 - objs1,
        2,
        "second commit should store only the changed tile + manifest"
    );

    // Both versions restore to their exact tile contents.
    let got1 = commit::file_at_commit(&r, "art.kra", &c1.id).unwrap();
    let got2 = commit::file_at_commit(&r, "art.kra", &c2.id).unwrap();
    let l1 = tiles::parse(&kra::read_entry(&got1, "img/layers/layer1").unwrap()).unwrap();
    let l2 = tiles::parse(&kra::read_entry(&got2, "img/layers/layer1").unwrap()).unwrap();
    assert_eq!(l1.tiles[1].data, b"tileBBBB");
    assert_eq!(l2.tiles[1].data, b"tileZZZZ");
    assert_eq!(l1.tiles[0].data, l2.tiles[0].data);
}

// --- working diff path: in-memory, never writes ----------------------------------------

#[test]
fn working_kra_diff_is_read_only_and_detects_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let kra1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, b"tileAAAA"), (0, 64, b"tileBBBB")]),
        ),
    ]);
    std::fs::write(root.join("art.kra"), &kra1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "tester").unwrap();
    let objs_before = count_objects(root);

    // A working copy with one edited tile, parsed in memory (the working-diff path).
    let kra2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, b"tileAAAA"), (0, 64, b"tileZZZZ")]),
        ),
    ]);
    let working = kra::parse_working(&kra2).unwrap();

    // Change detection against the committed manifest flags exactly the edited layer.
    let manifest_hash = c1
        .files
        .iter()
        .find(|f| f.path == "art.kra")
        .unwrap()
        .content
        .clone()
        .unwrap();
    let manifest = kra::load_manifest(&r, "art.kra", &manifest_hash).unwrap();
    let changed = kra::changed_entry_paths(&manifest.tile_index(), &working.tile_index());
    assert_eq!(
        changed,
        std::iter::once("img/layers/layer1".to_string()).collect()
    );

    // An untouched working copy reports no changed entries.
    let same = kra::parse_working(&kra1).unwrap();
    assert!(kra::changed_entry_paths(&manifest.tile_index(), &same.tile_index()).is_empty());

    // Viewing a working diff writes nothing to the object store.
    assert_eq!(count_objects(root), objs_before);
}

// --- scanner ---------------------------------------------------------------------------

#[test]
fn scan_status_and_lockfile_ignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    std::fs::write(root.join("notes.txt"), b"hello").unwrap();
    std::fs::write(root.join("scratch.kra~"), b"krita lock").unwrap();

    let s = scan::scan(&r).unwrap();
    assert!(s.iter().any(|(p, st)| p == "notes.txt" && st == "U"));
    assert!(
        !s.iter().any(|(p, _)| p == "scratch.kra~"),
        "*.kra~ must be ignored"
    );

    commit::commit_snapshot(&mut r, "init", "t").unwrap();
    assert!(
        scan::scan(&r).unwrap().is_empty(),
        "clean tree after commit"
    );

    std::fs::write(root.join("notes.txt"), b"hello world").unwrap();
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "notes.txt" && st == "M"));

    std::fs::remove_file(root.join("notes.txt")).unwrap();
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "notes.txt" && st == "D"));
}

// --- repo lifecycle --------------------------------------------------------------------

#[test]
fn open_errors_and_index_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(matches!(repo::Repo::open(root), Err(KvcError::NotARepo(_))));

    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    r.store_stream("file:x", b"data").unwrap();
    r.index.files.insert(
        "x".into(),
        repo::TrackedFile {
            hash: "h".into(),
            is_kra: false,
            size: 0,
            mtime: 0,
        },
    );
    r.save().unwrap();

    let r2 = repo::Repo::open(root).unwrap();
    assert!(r2.index.files.contains_key("x"));
    assert!(r2.chains.chain("file:x").is_some());

    assert_eq!(repo::epoch_to_iso(0), "1970-01-01T00:00:00Z");
    assert_eq!(repo::epoch_to_iso(1_609_459_200), "2021-01-01T00:00:00Z");
}

#[test]
fn delete_guarded_then_removes() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("not-a-repo");
    std::fs::create_dir(&plain).unwrap();
    std::fs::write(plain.join("keep.txt"), b"data").unwrap();

    // Guard: a non-.kvc folder is refused and left untouched.
    assert!(matches!(
        repo::Repo::delete(&plain),
        Err(KvcError::NotARepo(_))
    ));
    assert!(
        plain.join("keep.txt").exists(),
        "guarded delete must not touch the folder"
    );

    // A real repo is removed whole.
    let real = dir.path().join("repo");
    std::fs::create_dir(&real).unwrap();
    repo::Repo::init(&real).unwrap();
    repo::Repo::delete(&real).unwrap();
    assert!(!real.exists(), "delete should remove the repository folder");
}

// --- maindoc.xml layer metadata diff ---------------------------------------------------

#[test]
fn maindoc_layer_diff() {
    let d = kra::diff_maindoc(&maindoc(255), &maindoc(128)).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].name, "Lines");
    assert_eq!(d[0].change, "modified");
    assert!(d[0].details[0].contains("opacity"));
}

// --- undo last commit (soft: keep working tree) ---------------------------------------

#[test]
fn undo_last_commit_keeps_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    std::fs::write(root.join("notes.txt"), b"v1").unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(root.join("notes.txt"), b"v2").unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(r.commits.len(), 2);

    // Undo c2: log shrinks, working file is untouched, index rewinds so the edit resurfaces.
    let head = commit::undo_last_commit(&mut r).unwrap();
    assert_eq!(r.commits.len(), 1);
    assert_eq!(head.unwrap().id, c1.id);
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"v2");
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "notes.txt" && st == "M"));

    // Undo c1 (the add): file becomes untracked again.
    let head2 = commit::undo_last_commit(&mut r).unwrap();
    assert!(head2.is_none());
    assert!(r.commits.is_empty());
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "notes.txt" && st == "U"));

    // Undo on an empty log is a no-op.
    assert!(commit::undo_last_commit(&mut r).unwrap().is_none());
}

// --- rollback to a version (records a new commit) -------------------------------------

#[test]
fn rollback_restores_tree_as_new_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    std::fs::write(root.join("notes.txt"), b"v1").unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(root.join("notes.txt"), b"v2").unwrap();
    std::fs::write(root.join("extra.txt"), b"added later").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();

    let _ = c2;
    let c3 = commit::rollback_to_commit(&mut r, &c1.id, "t").unwrap();
    // Working tree matches c1: notes reverted, extra.txt (added in c2) removed.
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"v1");
    assert!(!root.join("extra.txt").exists());
    // A new commit captured the restored state; nothing left to commit afterwards.
    assert_eq!(r.commits.len(), 3);
    assert!(c3.message.contains("Restored to Version 1"));
    assert!(scan::scan(&r).unwrap().is_empty());

    // Rolling back to the current head (the tree already matches) is a no-op → Nothing.
    assert!(matches!(
        commit::rollback_to_commit(&mut r, &c3.id, "t"),
        Err(KvcError::Nothing)
    ));
}

/// Rollback synthesizes its commit from already-stored content — it must not store a single
/// new object (everything it restores is by definition already in the store), and undoing it
/// must round-trip cleanly.
#[test]
fn rollback_kra_writes_no_new_objects() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let mk = |tile: &[u8]| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&[(0, 0, tile)])),
        ])
    };
    std::fs::write(root.join("art.kra"), mk(b"tileAAAA")).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    std::fs::write(root.join("art.kra"), mk(b"tileZZZZ")).unwrap();
    commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let objs = count_objects(root);
    let c3 = commit::rollback_to_commit(&mut r, &c1.id, "t").unwrap();
    assert_eq!(
        count_objects(root),
        objs,
        "rollback must not store new objects"
    );
    // The restored working file carries v1's content and the tree is clean.
    let on_disk = std::fs::read(root.join("art.kra")).unwrap();
    let l = tiles::parse(&kra::read_entry(&on_disk, "img/layers/layer1").unwrap()).unwrap();
    assert_eq!(l.tiles[0].data, b"tileAAAA");
    assert!(scan::scan(&r).unwrap().is_empty());
    // The rollback commit records the same content hash as the original version.
    assert_eq!(
        c3.files[0].content,
        c1.files
            .iter()
            .find(|f| f.path == "art.kra")
            .unwrap()
            .content
    );
    // And it round-trips through undo (index rewinds, working file untouched).
    commit::undo_last_commit(&mut r).unwrap();
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "art.kra" && st == "M"));
}

// --- .kra per-layer raster reconstruction (visual diff) -------------------------------

/// A 64x64 RGBA8 tile filled with one color, planar B,G,R,A, uncompressed (flag byte 0).
fn solid_rgba_tile(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
    let n = 64 * 64;
    let mut data = vec![0u8]; // compression flag 0 = raw
    for c in [b, g, r, a] {
        data.extend(std::iter::repeat(c).take(n));
    }
    data
}

fn maindoc_raster() -> Vec<u8> {
    br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="64" height="64"><layers>
<layer name="Base" uuid="base" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
</layers></IMAGE></DOC>"#
        .to_vec()
}

#[test]
fn kra_layer_raster_decodes_to_png() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let kra = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_raster()),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
        ("mergedimage.png", b"\x89PNG\r\n\x1a\n fake".to_vec()),
    ]);
    std::fs::write(root.join("art.kra"), &kra).unwrap();
    let c = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    let manifest_hash = c
        .files
        .iter()
        .find(|f| f.path == "art.kra")
        .unwrap()
        .content
        .clone()
        .unwrap();
    let manifest = kra::load_manifest(&r, "art.kra", &manifest_hash).unwrap();

    // maindoc metadata parses.
    let meta = kra::parse_image_meta(&maindoc_raster()).unwrap();
    assert_eq!(
        (meta.width, meta.height, meta.name.as_str()),
        (64, 64, "img")
    );
    assert_eq!(meta.layers[0].filename, "layer1");

    // The layer decodes to a real PNG data URL.
    let cache = delta::TileCache::new();
    let url = kra::layer_raster(&r, "art.kra", &manifest, "img", "layer1", 64, 64, &cache).unwrap();
    let url = url.expect("layer1 should decode to a raster");
    assert!(url.starts_with("data:image/png;base64,"));
    assert!(url.len() > 100, "expected a non-trivial PNG payload");

    // The composite entry is surfaced too.
    let comp = kra::entry_data_url(&r, "art.kra", &manifest, "mergedimage.png").unwrap();
    assert!(comp.unwrap().starts_with("data:image/png;base64,"));
}

#[test]
fn kra_changed_entry_paths_flags_edited_layer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let mk = |tile: Vec<u8>| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc_raster()),
            ("img/layers/layer1", tiled(&[(0, 0, &tile)])),
        ])
    };
    std::fs::write(root.join("art.kra"), mk(solid_rgba_tile(10, 20, 30, 255))).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    std::fs::write(root.join("art.kra"), mk(solid_rgba_tile(99, 20, 30, 255))).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let m1 = kra::load_manifest(&r, "art.kra", &c1.files[0].content.clone().unwrap()).unwrap();
    let m2 = kra::load_manifest(&r, "art.kra", &c2.files[0].content.clone().unwrap()).unwrap();
    let (t1, t2) = (m1.tile_index(), m2.tile_index());
    let changed = kra::changed_entry_paths(&t1, &t2);
    assert!(
        changed.contains("img/layers/layer1"),
        "edited layer must be flagged"
    );

    let region = kra::changed_region(&t1, &t2, 64, 64);
    assert!(region.is_some(), "an edited tile yields a change region");
}

// --- progressive layer streaming + persistent raster cache -----------------------------

fn maindoc_layers(layers: &[(&str, &str, &str)]) -> Vec<u8> {
    let body: String = layers
        .iter()
        .map(|(name, uuid, filename)| {
            format!(
                r#"<layer name="{name}" uuid="{uuid}" opacity="255" compositeop="normal" nodetype="paintlayer" filename="{filename}"/>"#
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="64" height="64"><layers>{body}</layers></IMAGE></DOC>"#
    )
    .into_bytes()
}

/// Every layer of a raster diff (kept and removed alike) must be streamed through `on_layer`
/// exactly as it appears in the returned set — the frontend renders from the stream alone.
#[test]
fn art_diff_streams_every_layer_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    // v1: two layers; v2: layer2 removed → the v2 diff has one kept + one removed layer.
    let v1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        (
            "maindoc.xml",
            maindoc_layers(&[("Base", "base", "layer1"), ("Top", "top", "layer2")]),
        ),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
        (
            "img/layers/layer2",
            tiled(&[(0, 0, &solid_rgba_tile(40, 50, 60, 255))]),
        ),
    ]);
    let v2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_layers(&[("Base", "base", "layer1")])),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
    ]);
    std::fs::write(root.join("art.kra"), &v1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    std::fs::write(root.join("art.kra"), &v2).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let parent_tree = commit::tree_at_commit(&r.commits, &c1.id).unwrap();
    let f = c2.files.iter().find(|f| f.path == "art.kra").unwrap();
    let streamed = std::sync::Mutex::new(Vec::new());
    let dto = commands::committed_art_dto(
        &r,
        f,
        parent_tree.get("art.kra"),
        true,
        Some(&|l| streamed.lock().unwrap().push(l)),
    )
    .unwrap();
    let streamed = streamed.into_inner().unwrap();

    assert_eq!(dto.layers.len(), 2, "one kept + one removed layer");
    assert!(dto.layers.iter().any(|l| l.change == "removed"));
    assert_eq!(streamed.len(), dto.layers.len());
    for l in &dto.layers {
        assert!(
            streamed.iter().any(|s| s.id == l.id
                && s.change == l.change
                && s.before == l.before
                && s.after == l.after),
            "layer {} must be streamed with identical payload",
            l.id
        );
    }
}

/// A second rasterization of identical content must come from `.kvc/cache/`, not a re-decode.
#[test]
fn layer_raster_reads_from_disk_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let kra_bytes = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_raster()),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
    ]);
    std::fs::write(root.join("art.kra"), &kra_bytes).unwrap();
    let c = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    let manifest = kra::load_manifest(&r, "art.kra", &c.files[0].content.clone().unwrap()).unwrap();

    let first = kra::layer_raster(
        &r,
        "art.kra",
        &manifest,
        "img",
        "layer1",
        64,
        64,
        &delta::TileCache::new(),
    )
    .unwrap()
    .unwrap();

    // Exactly one cached PNG was written; replace its bytes to prove the next read uses it.
    let cache_dir = root.join(".kvc/cache");
    let cached: Vec<_> = std::fs::read_dir(&cache_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(cached.len(), 1, "first rasterization populates the cache");
    std::fs::write(&cached[0], b"MARKER").unwrap();

    let second = kra::layer_raster(
        &r,
        "art.kra",
        &manifest,
        "img",
        "layer1",
        64,
        64,
        &delta::TileCache::new(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(second, raster::png_bytes_to_data_url(b"MARKER"));
    assert_ne!(first, second);

    // Same pixels via the in-memory working path share the cache entry.
    let working = kra::parse_working(&kra_bytes).unwrap();
    let from_working = working
        .layer_raster("img/layers/layer1", 64, 64, &cache_dir)
        .unwrap()
        .unwrap();
    assert_eq!(from_working, second);
}

// --- branching: create / switch / merge / delete ---------------------------------------

/// Fresh repo with two committed files, ready for branch tests.
fn seeded_repo(dir: &tempfile::TempDir) -> repo::Repo {
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    std::fs::write(root.join("a.txt"), b"base-a").unwrap();
    std::fs::write(root.join("b.txt"), b"base-b").unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    r
}

#[test]
fn create_and_switch_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    // Create is instant and switches immediately, at the same tip.
    branch::create_branch(&mut r, "idea", None).unwrap();
    assert_eq!(r.branches.current, "idea");
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));

    // Commit on the branch, then bounce between the two trees.
    std::fs::write(root.join("a.txt"), b"idea-a").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on idea", "t").unwrap();
    assert_eq!(c2.parents, vec![c1.id.clone()]);
    assert_eq!(c2.branch, "idea");

    branch::switch_branch(&mut r, "main").unwrap();
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"base-a");
    assert!(scan::scan(&r).unwrap().is_empty());

    branch::switch_branch(&mut r, "idea").unwrap();
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"idea-a");
    assert!(scan::scan(&r).unwrap().is_empty());

    // State survives a reopen (branches.json persisted).
    let r2 = repo::Repo::open(root).unwrap();
    assert_eq!(r2.branches.current, "idea");
    assert_eq!(r2.branches.tip(), Some(c2.id.as_str()));
}

#[test]
fn switch_skips_unchanged_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "idea", None).unwrap();
    std::fs::write(root.join("a.txt"), b"idea-a").unwrap();
    commit::commit_snapshot(&mut r, "on idea", "t").unwrap();

    // b.txt is identical on both branches; switching must never rewrite it.
    let before = std::fs::metadata(root.join("b.txt"))
        .unwrap()
        .modified()
        .unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    let after = std::fs::metadata(root.join("b.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "unchanged file was rewritten on switch");
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"base-a");
}

#[test]
fn switch_refuses_dirty_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    branch::create_branch(&mut r, "idea", None).unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    std::fs::write(root.join("a.txt"), b"unsaved edit").unwrap();
    assert!(matches!(
        branch::switch_branch(&mut r, "idea"),
        Err(KvcError::DirtyTree)
    ));
    // The unsaved edit is untouched.
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"unsaved edit");
    assert_eq!(r.branches.current, "main");
}

#[test]
fn merge_fast_forward() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(root.join("a.txt"), b"feat-a").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    // main has not moved -> fast-forward: no new commit, tip jumps, tree materializes.
    let merged = branch::merge_branch(&mut r, "feat", "t").unwrap();
    assert_eq!(merged.id, c2.id);
    assert_eq!(r.commits.len(), 2);
    assert_eq!(r.branches.tip(), Some(c2.id.as_str()));
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"feat-a");
    assert!(scan::scan(&r).unwrap().is_empty());

    // Merging again: nothing to do.
    assert!(matches!(
        branch::merge_branch(&mut r, "feat", "t"),
        Err(KvcError::NothingToMerge(_))
    ));
}

#[test]
fn merge_three_way_no_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(root.join("b.txt"), b"feat-b").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "feat edits b", "t").unwrap();

    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(root.join("a.txt"), b"main-a").unwrap();
    let c3 = commit::commit_snapshot(&mut r, "main edits a", "t").unwrap();

    let m = branch::merge_branch(&mut r, "feat", "t").unwrap();
    assert_eq!(m.parents, vec![c3.id.clone(), c2.id.clone()]);
    // Only the source-side change is recorded (diff vs first parent).
    assert_eq!(m.files.len(), 1);
    assert_eq!(m.files[0].path, "b.txt");
    assert_eq!(m.files[0].status, "M");

    // Working tree has both sides; the merged tree folds correctly via first parents.
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"main-a");
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"feat-b");
    assert!(scan::scan(&r).unwrap().is_empty());
    let tree = commit::tree_at_commit(&r.commits, &m.id).unwrap();
    assert_ne!(
        tree["a.txt"].content,
        c1.files.iter().find(|f| f.path == "a.txt").unwrap().content
    );

    // Reachability: everything is now part of main's history.
    let reach = commit::ancestors(&r.commits, &m.id);
    for c in [&c1.id, &c2.id, &c3.id, &m.id] {
        assert!(reach.contains(c.as_str()));
    }
}

#[test]
fn merge_three_way_conflict_takes_source_and_flags() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(root.join("a.txt"), b"feat-a").unwrap();
    commit::commit_snapshot(&mut r, "feat edits a", "t").unwrap();

    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(root.join("a.txt"), b"main-a").unwrap();
    commit::commit_snapshot(&mut r, "main edits a", "t").unwrap();

    let m = branch::merge_branch(&mut r, "feat", "t").unwrap();
    let entry = m.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!(entry.status, "C");
    // Source wins on disk.
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"feat-a");
    assert!(scan::scan(&r).unwrap().is_empty());
}

#[test]
fn list_commits_scoped_by_branch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(root.join("b.txt"), b"feat-b").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(root.join("a.txt"), b"main-a").unwrap();
    let c3 = commit::commit_snapshot(&mut r, "on main", "t").unwrap();

    // main's history excludes the branch-only commit until it is merged.
    let main_reach = commit::ancestors(&r.commits, &c3.id);
    assert!(main_reach.contains(c1.id.as_str()) && main_reach.contains(c3.id.as_str()));
    assert!(!main_reach.contains(c2.id.as_str()));

    let m = branch::merge_branch(&mut r, "feat", "t").unwrap();
    let merged_reach = commit::ancestors(&r.commits, &m.id);
    assert!(
        merged_reach.contains(c2.id.as_str()),
        "merged branch commits join the target's history"
    );
}

#[test]
fn migration_missing_branches_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();
    drop(r);

    // Simulate a pre-branching repo.
    std::fs::remove_file(root.join(".kvc/branches.json")).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    assert_eq!(r.branches.current, "main");
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));

    // The next commit persists branches.json and chains parentage correctly.
    std::fs::write(root.join("a.txt"), b"v2").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(c2.parents, vec![c1.id.clone()]);
    assert!(root.join(".kvc/branches.json").is_file());
}

#[test]
fn undo_respects_branch_tip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(root.join("b.txt"), b"feat-b").unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(root.join("a.txt"), b"main-a").unwrap();
    let c3 = commit::commit_snapshot(&mut r, "on main", "t").unwrap();

    // Commit parent is the branch tip, not the newest commit in the vec.
    assert_eq!(c3.parents, vec![c1.id.clone()]);

    // Undo on main removes only c3; feat's commit survives mid-vec.
    let head = commit::undo_last_commit(&mut r).unwrap();
    assert_eq!(head.unwrap().id, c1.id);
    assert!(r.commits.iter().any(|c| c.id == c2.id));
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));

    // c1 now has a child (c2 on feat) -> undoing it is refused.
    assert!(matches!(
        commit::undo_last_commit(&mut r),
        Err(KvcError::CannotUndo(_))
    ));
}

#[test]
fn branch_name_validation_and_delete_guards() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = seeded_repo(&dir);

    assert!(matches!(
        branch::create_branch(&mut r, "  ", None),
        Err(KvcError::BadBranchName(_))
    ));
    assert!(matches!(
        branch::create_branch(&mut r, "a/b", None),
        Err(KvcError::BadBranchName(_))
    ));
    assert!(matches!(
        branch::create_branch(&mut r, "main", None),
        Err(KvcError::BranchExists(_))
    ));

    branch::create_branch(&mut r, "idea", None).unwrap();
    assert!(matches!(
        branch::delete_branch(&mut r, "idea"),
        Err(KvcError::DeleteCurrent)
    ));
    assert!(matches!(
        branch::delete_branch(&mut r, "ghost"),
        Err(KvcError::NoBranch(_))
    ));

    branch::switch_branch(&mut r, "main").unwrap();
    branch::delete_branch(&mut r, "idea").unwrap();
    assert!(!r.branches.branches.contains_key("idea"));
}

#[test]
fn create_branch_from_other_base() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    // Diverge on "idea", then start a new branch from main's tree while standing on idea.
    branch::create_branch(&mut r, "idea", None).unwrap();
    std::fs::write(root.join("a.txt"), b"idea-a").unwrap();
    commit::commit_snapshot(&mut r, "on idea", "t").unwrap();

    branch::create_branch(&mut r, "third", Some("main")).unwrap();
    assert_eq!(r.branches.current, "third");
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));
    // The working tree was materialized to main's files.
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"base-a");
    assert!(scan::scan(&r).unwrap().is_empty());

    // Unknown base -> error; unsaved changes -> refused, nothing moves.
    assert!(matches!(
        branch::create_branch(&mut r, "x", Some("ghost")),
        Err(KvcError::NoBranch(_))
    ));
    std::fs::write(root.join("a.txt"), b"unsaved").unwrap();
    assert!(matches!(
        branch::create_branch(&mut r, "x", Some("idea")),
        Err(KvcError::DirtyTree)
    ));
    assert_eq!(r.branches.current, "third");
    assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"unsaved");
}

#[test]
fn commit_crc_skip_reuses_unchanged_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let kra1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, b"tileAAAA")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"tileBBBB")])),
    ]);
    std::fs::write(root.join("art.kra"), &kra1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    // Edit only layer2; layer1 and maindoc.xml keep their crc32+size and must be reused.
    let kra2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, b"tileAAAA")])),
        ("img/layers/layer2", tiled(&[(0, 0, b"tileZZZZ")])),
    ]);
    std::fs::write(root.join("art.kra"), &kra2).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let h1 = c1.files[0].content.clone().unwrap();
    let h2 = c2.files[0].content.clone().unwrap();
    let m1 = kra::load_manifest(&r, "art.kra", &h1).unwrap();
    let m2 = kra::load_manifest(&r, "art.kra", &h2).unwrap();
    let (t1, t2) = (m1.tile_index(), m2.tile_index());
    assert_eq!(
        t1["img/layers/layer1"], t2["img/layers/layer1"],
        "unchanged layer must keep identical tile refs"
    );
    assert_ne!(t1["img/layers/layer2"], t2["img/layers/layer2"]);
    assert_eq!(m1.entry_hash("maindoc.xml"), m2.entry_hash("maindoc.xml"));

    // The reconstructed v2 archive carries the same logical content as the working file.
    let rebuilt = kra::reconstruct_kra(&r, "art.kra", &h2).unwrap();
    let rb = kra::parse_working(&rebuilt).unwrap();
    let wk = kra::parse_working(&kra2).unwrap();
    assert_eq!(rb.tile_index(), wk.tile_index());
    assert_eq!(
        kra::read_entry(&rebuilt, "maindoc.xml").unwrap(),
        maindoc(255)
    );
    assert_eq!(
        kra::read_entry(&rebuilt, "mimetype").unwrap(),
        b"application/x-krita"
    );

    // Reconstructed files round-trip through the crc fast path: after a rewrite (as a branch
    // switch does), an untouched re-commit sees no changes at all.
    std::fs::write(root.join("art.kra"), &rebuilt).unwrap();
    let s = scan::scan(&r).unwrap();
    if !s.is_empty() {
        // The rebuilt zip's bytes differ from the working file; committing it must reuse
        // every stream (same manifest content) rather than re-storing anything.
        let objs = count_objects(root);
        let c3 = commit::commit_snapshot(&mut r, "rebuilt", "t").unwrap();
        assert_eq!(c3.files[0].content.clone().unwrap(), h2);
        assert_eq!(count_objects(root), objs);
    }
}

// --- chains persistence: sharded format, legacy monolith migration, skip-on-clean --------

fn shard_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(root.join(".kvc/chains"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
fn chains_sharded_format_and_legacy_monolith_migration() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();
    let h = r.store_stream("file:x", b"some data").unwrap();
    r.save().unwrap();
    // Fresh repos write per-file shards, never a monolith.
    assert_eq!(shard_files(root).len(), 1);
    assert!(!root.join(".kvc/chains.bin").exists());

    // Simulate a pre-sharding repo: all chains in one monolithic chains.bin, no shards.
    let all = r.chains.export_all();
    let monolith = zstd::encode_all(&bincode::serialize(&all).unwrap()[..], 1).unwrap();
    std::fs::write(root.join(".kvc/chains.bin"), &monolith).unwrap();
    std::fs::remove_dir_all(root.join(".kvc/chains")).unwrap();

    // Opens read the monolith transparently...
    let mut r2 = repo::Repo::open(root).unwrap();
    assert_eq!(r2.reconstruct("file:x", &h).unwrap(), b"some data");

    // ...and the next save splits it into shards and retires it.
    r2.store_stream("file:y", b"more").unwrap();
    r2.save().unwrap();
    assert_eq!(shard_files(root).len(), 2, "one shard per tracked file");
    assert!(!root.join(".kvc/chains.bin").exists());
    let r3 = repo::Repo::open(root).unwrap();
    assert!(r3.chains.chain("file:x").is_some() && r3.chains.chain("file:y").is_some());
    assert_eq!(r3.reconstruct("file:x", &h).unwrap(), b"some data");

    // The oldest format (chains.json) migrates the same way.
    let json = serde_json::to_vec(&all).unwrap();
    std::fs::write(root.join(".kvc/chains.json"), &json).unwrap();
    std::fs::remove_dir_all(root.join(".kvc/chains")).unwrap();
    let mut r4 = repo::Repo::open(root).unwrap();
    assert_eq!(r4.reconstruct("file:x", &h).unwrap(), b"some data");
    r4.store_stream("file:x", b"newer data").unwrap();
    r4.save().unwrap();
    assert!(!root.join(".kvc/chains.json").exists());
    assert!(!shard_files(root).is_empty());
}

#[test]
fn switch_skips_chains_rewrite_commit_touches_only_changed_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "idea", None).unwrap();
    std::fs::write(root.join("a.txt"), b"idea-a").unwrap();
    commit::commit_snapshot(&mut r, "on idea", "t").unwrap();

    // Sentinel every shard on disk; a switch (no new stream versions) must rewrite none.
    let originals: Vec<(std::path::PathBuf, Vec<u8>)> = shard_files(root)
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            std::fs::write(&p, b"SENTINEL").unwrap();
            (p, bytes)
        })
        .collect();
    assert_eq!(originals.len(), 2, "one shard per seeded file");
    branch::switch_branch(&mut r, "main").unwrap();
    for (p, _) in &originals {
        assert_eq!(
            std::fs::read(p).unwrap(),
            b"SENTINEL",
            "switch must not rewrite any chain shard"
        );
    }
    for (p, bytes) in &originals {
        std::fs::write(p, bytes).unwrap();
    }

    // A commit to a.txt rewrites exactly a.txt's shard; b.txt's shard is untouched.
    let before: std::collections::HashMap<_, _> = shard_files(root)
        .into_iter()
        .map(|p| (p.clone(), std::fs::read(&p).unwrap()))
        .collect();
    std::fs::write(root.join("a.txt"), b"main-a2").unwrap();
    commit::commit_snapshot(&mut r, "on main", "t").unwrap();
    let changed = shard_files(root)
        .into_iter()
        .filter(|p| std::fs::read(p).unwrap() != before[p])
        .count();
    assert_eq!(changed, 1, "only the committed file's shard is rewritten");

    let r2 = repo::Repo::open(root).unwrap();
    assert_eq!(
        r2.chains.export_all().0.len(),
        r.chains.export_all().0.len()
    );
}

// --- garbage collection -------------------------------------------------------------------

/// GC after undo + branch-delete must reclaim storage while every remaining commit's every
/// file still reconstructs byte-for-byte — including patch chains whose bases must survive.
#[test]
fn gc_reclaims_orphans_and_preserves_reachable_history() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let mk = |tile: &[u8], extra: &[u8]| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&[(0, 0, tile)])),
            ("img/extra.bin", extra.to_vec()),
        ])
    };
    // Three commits on main (so the patch-chaining generic file has history), then a branch
    // with its own commit, then: undo one commit + delete the branch = two orphan sets.
    std::fs::write(root.join("art.kra"), mk(b"tileAAAA", b"x1")).unwrap();
    // A large-ish text file so its versions bsdiff-chain (>64KB, incompressible-ish text).
    let big1: Vec<u8> = (0..90_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(root.join("notes.bin"), &big1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    let mut big2 = big1.clone();
    big2.extend_from_slice(b"more");
    std::fs::write(root.join("notes.bin"), &big2).unwrap();
    std::fs::write(root.join("art.kra"), mk(b"tileBBBB", b"x1")).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();

    // Branch with a divergent edit, then go back to main.
    branch::create_branch(&mut r, "scrap", None).unwrap();
    std::fs::write(root.join("art.kra"), mk(b"tileSCRAP", b"x2")).unwrap();
    commit::commit_snapshot(&mut r, "scrap edit", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    // Another main commit, then undo it (its edits resurface as pending and get re-committed
    // — dedup makes that cheap), and delete scrap: the stranded branch history is the garbage.
    std::fs::write(root.join("art.kra"), mk(b"tileCCCC", b"x3")).unwrap();
    commit::commit_snapshot(&mut r, "c3", "t").unwrap();
    commit::undo_last_commit(&mut r).unwrap();
    commit::commit_snapshot(&mut r, "c3 again", "t").unwrap();
    branch::delete_branch(&mut r, "scrap").unwrap();

    let objs_before = count_objects(root);

    // Dry run reports without deleting.
    let dry = gc::collect_garbage(&mut r, true).unwrap();
    assert!(dry.dry_run && dry.bytes_reclaimed > 0 && dry.versions_removed > 0);
    assert_eq!(count_objects(root), objs_before, "dry run must not delete");

    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert!(!report.dry_run);
    // The undone commit already left the log (undo removes it; only its objects orphan);
    // the deleted branch's commit is dropped here.
    assert_eq!(report.commits_removed, 1, "the stranded branch commit");
    assert!(report.bytes_reclaimed > 0);
    assert!(count_objects(root) < objs_before);

    // Everything reachable reconstructs byte-for-byte, in this session and after reopen.
    for repo_ref in [&r, &repo::Repo::open(root).unwrap()] {
        for (cid, tile, extra, big) in [
            (&c1.id, b"tileAAAA".as_slice(), b"x1".as_slice(), &big1),
            (&c2.id, b"tileBBBB".as_slice(), b"x1".as_slice(), &big2),
        ] {
            let kra_bytes = commit::file_at_commit(repo_ref, "art.kra", cid).unwrap();
            let l =
                tiles::parse(&kra::read_entry(&kra_bytes, "img/layers/layer1").unwrap()).unwrap();
            assert_eq!(l.tiles[0].data, tile);
            assert_eq!(kra::read_entry(&kra_bytes, "img/extra.bin").unwrap(), extra);
            assert_eq!(
                &commit::file_at_commit(repo_ref, "notes.bin", cid).unwrap(),
                big
            );
        }
    }

    // The tree is still clean and a fresh commit works after GC.
    assert!(scan::scan(&r).unwrap().is_empty());
    std::fs::write(root.join("art.kra"), mk(b"tileDDDD", b"x4")).unwrap();
    commit::commit_snapshot(&mut r, "post-gc", "t").unwrap();
    assert!(scan::scan(&r).unwrap().is_empty());
}

// --- objects layout: sharded writes, flat legacy reads -----------------------------------

#[test]
fn objects_sharded_layout_with_flat_read_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let data = b"object payload".to_vec();
    let h = r.store_stream("file:x", &data).unwrap();
    let objects = root.join(".kvc/objects");
    let sharded = objects.join(&h[..2]).join(format!("{h}.full"));
    assert!(sharded.is_file(), "new objects land in objects/<xx>/");

    // Simulate a pre-sharding repo: the same object at the flat path only.
    let flat = objects.join(format!("{h}.full"));
    std::fs::rename(&sharded, &flat).unwrap();
    assert_eq!(
        r.reconstruct("file:x", &h).unwrap(),
        data,
        "flat legacy objects stay readable"
    );

    // Re-storing identical content dedups against the flat copy (no sharded duplicate).
    let h2 = r.store_stream("file:y", &data).unwrap();
    assert_eq!(h2, h);
    assert!(
        !sharded.exists(),
        "existing flat object must not be rewritten"
    );
}

/// A commit with many changed tiles writes ONE pack file instead of thousands of loose
/// objects (per-file create cost dominated large first commits on Windows), and everything
/// reconstructs from the pack byte-for-byte — including after a fresh open.
#[test]
fn large_commit_packs_objects_and_reconstructs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    // 40 tiles (> PACK_MIN_OBJECTS) with distinct contents.
    let tiles_v1: Vec<(i64, i64, Vec<u8>)> = (0..40i64)
        .map(|i| (i * 64, 0, vec![i as u8; 300 + i as usize]))
        .collect();
    let refs: Vec<(i64, i64, &[u8])> = tiles_v1
        .iter()
        .map(|(x, y, d)| (*x, *y, d.as_slice()))
        .collect();
    let kra = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&refs)),
    ]);
    std::fs::write(root.join("art.kra"), &kra).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    // The tile batch became one pack; only the manifest & small entries are loose.
    let packs: Vec<_> = std::fs::read_dir(root.join(".kvc/objects/pack"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
        .collect();
    assert_eq!(packs.len(), 1, "one pack per large batch");
    let loose = count_objects(root) - packs.len();
    assert!(
        loose < 10,
        "tiles must be packed, not loose (found {loose} loose objects)"
    );

    // Same-session restore and fresh-open restore both round-trip from the pack.
    let restored = commit::file_at_commit(&r, "art.kra", &c1.id).unwrap();
    let l = tiles::parse(&kra::read_entry(&restored, "img/layers/layer1").unwrap()).unwrap();
    assert_eq!(l.tiles.len(), 40);
    assert_eq!(l.tiles[7].data, tiles_v1[7].2);

    let r2 = repo::Repo::open(root).unwrap();
    let restored2 = commit::file_at_commit(&r2, "art.kra", &c1.id).unwrap();
    assert_eq!(restored, restored2);

    // Re-committing identical content dedups against packed objects (no new pack, no loose).
    drop(r2);
    std::fs::remove_file(root.join("art.kra")).unwrap();
    std::fs::write(root.join("art.kra"), &kra).unwrap();
    let before = count_objects(root);
    // Deleting + rewriting identical bytes leaves content identical to the tip — a clean scan
    // (mtime changed but hash matches) means nothing to commit.
    assert!(matches!(
        commit::commit_snapshot(&mut r, "again", "t"),
        Err(KvcError::Nothing)
    ));
    assert_eq!(count_objects(root), before);
}

// --- incremental .kra materialization on switch ------------------------------------------

#[test]
fn kra_switch_materializes_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo::Repo::init(root).unwrap();
    let mut r = repo::Repo::open(root).unwrap();

    let t1 = tiled(&[(0, 0, &[1u8; 500]), (64, 0, &[2u8; 500])]);
    let t2 = tiled(&[(0, 0, &[3u8; 500])]);
    let base = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", t1.clone()),
        ("img/layers/layer2", t2.clone()),
    ]);
    std::fs::write(root.join("art.kra"), &base).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "base", "t").unwrap();

    // Branch edits layer2 (one tile changed, one added) and maindoc; layer1 is untouched.
    branch::create_branch(&mut r, "idea", None).unwrap();
    let t2b = tiled(&[(0, 0, &[9u8; 500]), (64, 64, &[7u8; 500])]);
    let edited = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(128)),
        ("img/layers/layer1", t1),
        ("img/layers/layer2", t2b.clone()),
    ]);
    std::fs::write(root.join("art.kra"), &edited).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "edit", "t").unwrap();

    // The incremental path directly: rebuild c1's file from c2's on-disk working copy and
    // compare every entry against the full store reconstruction.
    let hash_of = |c: &repo::Commit| {
        c.files
            .iter()
            .find(|f| f.path == "art.kra")
            .unwrap()
            .content
            .clone()
            .unwrap()
    };
    let (h1, h2) = (hash_of(&c1), hash_of(&c2));
    let working = std::fs::read(root.join("art.kra")).unwrap();
    let incremental = kra::materialize_kra(&r, "art.kra", &h1, &h2, &working).unwrap();
    let full = kra::reconstruct_kra(&r, "art.kra", &h1).unwrap();
    for name in [
        "mimetype",
        "maindoc.xml",
        "img/layers/layer1",
        "img/layers/layer2",
    ] {
        assert_eq!(
            kra::read_entry(&incremental, name).unwrap(),
            kra::read_entry(&full, name).unwrap(),
            "entry {name} differs from the full reconstruction"
        );
    }

    // End-to-end: bounce between branches; content is exact and the tree stays clean.
    branch::switch_branch(&mut r, "main").unwrap();
    let on_main = std::fs::read(root.join("art.kra")).unwrap();
    assert_eq!(
        kra::read_entry(&on_main, "maindoc.xml").unwrap(),
        maindoc(255)
    );
    assert_eq!(kra::read_entry(&on_main, "img/layers/layer2").unwrap(), t2);
    assert!(scan::scan(&r).unwrap().is_empty());

    branch::switch_branch(&mut r, "idea").unwrap();
    let on_idea = std::fs::read(root.join("art.kra")).unwrap();
    assert_eq!(
        kra::read_entry(&on_idea, "maindoc.xml").unwrap(),
        maindoc(128)
    );
    assert_eq!(kra::read_entry(&on_idea, "img/layers/layer2").unwrap(), t2b);
    assert!(scan::scan(&r).unwrap().is_empty());
}
