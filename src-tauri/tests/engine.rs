//! End-to-end engine tests against real file I/O in tempdirs (no logic mocked).

use krita_vc_lib::{commit, error::KvcError, kra, repo, scan, tiles};
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

fn count_objects(root: &std::path::Path) -> usize {
    std::fs::read_dir(root.join(".kvc/objects"))
        .unwrap()
        .count()
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

    let chain = &r.chains.0[key];
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
        },
    );
    r.save().unwrap();

    let r2 = repo::Repo::open(root).unwrap();
    assert!(r2.index.files.contains_key("x"));
    assert!(r2.chains.0.contains_key("file:x"));

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
    let manifest = c
        .files
        .iter()
        .find(|f| f.path == "art.kra")
        .unwrap()
        .content
        .clone()
        .unwrap();

    // maindoc metadata parses.
    let meta = kra::parse_image_meta(&maindoc_raster()).unwrap();
    assert_eq!(
        (meta.width, meta.height, meta.name.as_str()),
        (64, 64, "img")
    );
    assert_eq!(meta.layers[0].filename, "layer1");

    // The layer decodes to a real PNG data URL.
    let url = kra::layer_raster(&r, "art.kra", &manifest, "img", "layer1", 64, 64).unwrap();
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

    let m1 = c1.files[0].content.clone().unwrap();
    let m2 = c2.files[0].content.clone().unwrap();
    let changed = kra::changed_entry_paths(&r, "art.kra", &m1, &m2).unwrap();
    assert!(
        changed.contains("img/layers/layer1"),
        "edited layer must be flagged"
    );

    let region = kra::changed_region(&r, "art.kra", &m1, &m2, 64, 64).unwrap();
    assert!(region.is_some(), "an edited tile yields a change region");
}
