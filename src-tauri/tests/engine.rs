//! End-to-end engine tests against real file I/O in tempdirs (no logic mocked).

use krita_vc_lib::{
    branch, commands, commit, delta, error::KvcError, gc, kra, merge, palette, raster, repo, scan,
    stash, tiles,
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

/// A tiny but *real* `.kra`. Tracked documents can't be arbitrary bytes the way the old `.gpl`
/// fixtures were — the commit path parses a `.kra` as a zip.
fn kra_bytes(lines_opacity: i64) -> Vec<u8> {
    // Shaped like a real document, not a one-entry stub: a mimetype, a maindoc, and a tiled
    // layer. That matters because a `.kra` is stored as one stream per zip entry (and one per
    // tile) — a stub yields a single content object, too few for the dedup, storage-stats and
    // corruption tests to say anything. The tile payloads vary with `lines_opacity` so two
    // revisions genuinely differ and something is left to delta.
    let a = format!("tileA{lines_opacity:03}").into_bytes();
    let b = format!("tileB{lines_opacity:03}").into_bytes();
    pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(lines_opacity)),
        ("img/layers/layer1", tiled(&[(0, 0, &a), (0, 64, &b)])),
    ])
}

/// Track one document in `root` and return its path. One store tracks exactly one `.kra`, so
/// this replaces the old "init a folder as a repo" setup that every test opened with.
fn init_doc(root: &std::path::Path) -> std::path::PathBuf {
    init_doc_named(root, "art.kra")
}

fn init_doc_named(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    if !path.exists() {
        std::fs::write(&path, kra_bytes(100)).unwrap();
    }
    repo::Repo::init(&path).unwrap();
    path
}

/// A restored `.kra` is *rebuilt*, not byte-copied — entries are re-deflated, so the archive
/// bytes differ from what was committed while the content is identical. Tests therefore compare
/// the document's content (its maindoc), which the old byte-exact `.gpl` fixtures never had to.
fn doc_content(path: &std::path::Path) -> Vec<u8> {
    kra::read_entry(&std::fs::read(path).unwrap(), "maindoc.xml").unwrap()
}

/// The document `init_doc` tracks in `root`. Most tests only need the path, not the init.
fn tracked_doc(root: &std::path::Path) -> std::path::PathBuf {
    root.join("art.kra")
}

/// Where a document's history lives. Tests that poke at store internals need this now that the
/// store is `<root>/.kvc/<slug>/` rather than `<root>/.kvc`.
fn store_of(doc: &std::path::Path) -> std::path::PathBuf {
    repo::store_dir_for(doc)
}

/// Any loose `.full` snapshot in a store. Under the folder model a test could name an object by
/// hashing the fixture's bytes; a `.kra` is stored as one stream per zip entry, so the object
/// names are internal and a corruption test has to pick one off disk instead.
fn any_loose_object(store: &std::path::Path) -> std::path::PathBuf {
    all_loose_objects(store)
        .pop()
        .expect("store should hold at least one loose full snapshot")
}

/// A loose object holding a `.kra`'s *content* rather than its manifest. Corrupting the manifest
/// breaks the chain read before the content scrub ever runs, so a scrub test has to pick a
/// content stream deliberately — under the old `.gpl` fixtures every object was content.
fn corruptible_object(r: &repo::Repo, store: &std::path::Path) -> std::path::PathBuf {
    let chains = r.chains.export_all();
    for (key, versions) in &chains.0 {
        if key.ends_with(":manifest") {
            continue;
        }
        for v in versions {
            let p = store
                .join("objects")
                .join(&v.hash[..2])
                .join(v.object_name());
            if p.is_file() {
                return p;
            }
        }
    }
    panic!("no corruptible content object in {store:?}")
}

fn all_loose_objects(store: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "full") {
                    out.push(p);
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(&store.join("objects"), &mut found);
    found.sort();
    found
}

/// Object files across the sharded (`objects/<xx>/`) and legacy flat layouts.
fn count_objects(store: &std::path::Path) -> usize {
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
    walk(&store.join("objects"))
}

// --- the critical path: delta chains reconstruct exactly -------------------------------

#[test]
fn delta_roundtrip_and_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    let objs1 = count_objects(&store_of(&tracked_doc(root)));

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
    let objs2 = count_objects(&store_of(&tracked_doc(root)));

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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    let objs_before = count_objects(&store_of(&tracked_doc(root)));

    // A working copy with one edited tile, parsed in memory (the working-diff path).
    let kra2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, b"tileAAAA"), (0, 64, b"tileZZZZ")]),
        ),
    ]);
    let working = kra::parse_working(&kra2, false).unwrap();

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
    let same = kra::parse_working(&kra1, false).unwrap();
    assert!(kra::changed_entry_paths(&manifest.tile_index(), &same.tile_index()).is_empty());

    // Viewing a working diff writes nothing to the object store.
    assert_eq!(count_objects(&store_of(&tracked_doc(root))), objs_before);
}

/// A `.kpl` blob (zip of a colorset.xml) with the given named sRGB swatches — the shape Krita
/// embeds inside a `.kra` under `<image>/palettes/`.
fn kpl_blob(swatches: &[(&str, (u8, u8, u8))]) -> Vec<u8> {
    let mut xml = String::from(r#"<ColorSet version="1.0" columns="4">"#);
    for (name, (r, g, b)) in swatches {
        xml.push_str(&format!(
            r#"<ColorSetEntry name="{name}"><sRGB r="{}" g="{}" b="{}"/></ColorSetEntry>"#,
            *r as f64 / 255.0,
            *g as f64 / 255.0,
            *b as f64 / 255.0,
        ));
    }
    xml.push_str("</ColorSet>");
    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        zw.start_file::<_, ()>("colorset.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    out
}

/// A recolored document palette embedded in a `.kra` is discovered on both sides, flagged
/// changed, and its recolored swatch reads as "modified" — the backend half of the
/// embedded-palette diff feature.
#[test]
fn kra_embedded_palette_color_change_detected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let kra1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, b"tileAAAA")])),
        ("img/palettes/pal.kpl", kpl_blob(&[("Skin", (255, 0, 0))])),
    ]);
    std::fs::write(root.join("art.kra"), &kra1).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "tester").unwrap();
    let manifest_hash = c1
        .files
        .iter()
        .find(|f| f.path == "art.kra")
        .unwrap()
        .content
        .clone()
        .unwrap();
    let manifest = kra::load_manifest(&r, "art.kra", &manifest_hash).unwrap();
    let old_src = kra::KraSource::Committed(&manifest);

    // Working copy: same palette entry, one swatch recolored red -> blue.
    let kra2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, b"tileAAAA")])),
        ("img/palettes/pal.kpl", kpl_blob(&[("Skin", (0, 0, 255))])),
    ]);
    let working = kra::parse_working(&kra2, false).unwrap();
    let new_src = kra::KraSource::Working(&working);

    // Discovered on the working side, and flagged changed (different content hash).
    let name = "img/palettes/pal.kpl";
    assert!(new_src.palette_entry_names().iter().any(|n| n == name));
    assert_ne!(new_src.entry_hash(name), old_src.entry_hash(name));

    // The swatch diff sees the recolor as a modification.
    let old_bytes = old_src.entry_bytes(&r, "art.kra", name).unwrap().unwrap();
    let new_bytes = new_src.entry_bytes(&r, "art.kra", name).unwrap().unwrap();
    let old_pal = palette::parse("pal.kpl", &old_bytes).unwrap();
    let new_pal = palette::parse("pal.kpl", &new_bytes).unwrap();
    let d = palette::diff(Some(&old_pal), Some(&new_pal));
    let skin = d.swatches.iter().find(|s| s.name == "Skin").unwrap();
    assert_eq!(skin.change, "modified");
    assert_eq!(skin.before.as_deref(), Some("#FF0000"));
    assert_eq!(skin.after.as_deref(), Some("#0000FF"));
}

// --- scanner ---------------------------------------------------------------------------

/// Krita's autosave artifact ends in `.kra`, so a naive suffix test tracks the artist's scratch
/// state as a document — its own chain shard, its own changelist row, committed as junk. The
/// Krita plugin drives saves now, which makes these appear and vanish under the scanner.
#[test]
fn autosave_artifacts_cannot_be_tracked() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Krita's autosave artifact ends in `.kra` but isn't the artist's document. It's rejected in
    // `is_supported`, which is what `Repo::init` gates on — the scanner no longer walks the
    // folder looking for things to track, so this is the only place the rule can still bite.
    for junk in ["hero.kra-autosave.kra", ".hero.kra-autosave.kra"] {
        assert!(!scan::is_supported(junk), "{junk} must never be trackable");
        let path = root.join(junk);
        std::fs::write(&path, kra_bytes(2)).unwrap();
        assert!(matches!(
            repo::Repo::init(&path),
            Err(KvcError::Unsupported(_))
        ));
    }

    assert!(scan::is_supported("hero.kra"));
    // Standalone palettes stopped being trackable when tracking went per-document — a `.kra`'s
    // embedded palettes are still diffed off the `.kra` itself.
    for pal in ["swatches.gpl", "set.kpl", "sw.aco", "sw.ase"] {
        assert!(!scan::is_supported(pal), "{pal} must not be trackable");
    }
}

/// The scanner's whole job now: report the one tracked document, and nothing else in the folder.
#[test]
fn scan_reports_only_the_tracked_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    // Neighbours of every kind — another artwork, a palette, junk — are invisible here. The
    // other artwork has its own store and its own history; this one is not a folder view.
    std::fs::write(root.join("other.kra"), kra_bytes(3)).unwrap();
    std::fs::write(
        root.join("swatches.gpl"),
        b"GIMP Palette
",
    )
    .unwrap();
    std::fs::write(root.join("readme.txt"), b"hi").unwrap();
    assert!(scan::scan(&r).unwrap().is_empty());

    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
    assert_eq!(
        scan::scan(&r).unwrap(),
        vec![("art.kra".to_string(), "M".to_string())]
    );

    // A deleted document is still reported, so the UI can say so rather than showing nothing.
    std::fs::remove_file(tracked_doc(root)).unwrap();
    assert_eq!(
        scan::scan(&r).unwrap(),
        vec![("art.kra".to_string(), "D".to_string())]
    );
}

/// `keep_bytes` retention is budgeted (`scan::RETAIN_BUDGET`), and the direction of that guard
/// is not otherwise observable: invert it and commits still succeed, because
/// `commit::store_change` falls back to re-reading the file. They would just silently re-read
/// every ordinary document — the exact cost retention exists to avoid. So assert the ordinary
/// case actually keeps its buffer.
#[test]
fn scan_keeps_bytes_for_documents_under_the_retain_budget() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
    let changes = scan::scan_detailed(&r, true).unwrap();
    let doc = changes.iter().find(|c| c.rel == "art.kra").unwrap();
    assert!(
        (doc.size as u64) < scan::RETAIN_BUDGET,
        "fixture must be under budget for this to test the retaining branch"
    );
    assert!(
        doc.bytes.is_some(),
        "an under-budget document must hand its buffer to the commit path"
    );

    // And `keep_bytes: false` never retains, budget or not.
    let changes = scan::scan_detailed(&r, false).unwrap();
    assert!(changes.iter().all(|c| c.bytes.is_none()));
}

#[test]
fn scan_status_and_lockfile_ignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(40)).unwrap();
    std::fs::write(root.join("scratch.kra~"), b"krita lock").unwrap();
    std::fs::write(root.join("readme.txt"), b"unsupported").unwrap();

    let s = scan::scan(&r).unwrap();
    assert!(s.iter().any(|(p, st)| p == "art.kra" && st == "U"));
    assert!(
        !s.iter().any(|(p, _)| p == "scratch.kra~"),
        "*.kra~ must be ignored"
    );
    assert!(
        !s.iter().any(|(p, _)| p == "readme.txt"),
        "unsupported file types must never be tracked"
    );

    commit::commit_snapshot(&mut r, "init", "t").unwrap();
    assert!(
        scan::scan(&r).unwrap().is_empty(),
        "clean tree after commit"
    );

    std::fs::write(tracked_doc(root), kra_bytes(41)).unwrap();
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "art.kra" && st == "M"));

    std::fs::remove_file(tracked_doc(root)).unwrap();
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "art.kra" && st == "D"));
}

// --- repo lifecycle --------------------------------------------------------------------

#[test]
fn open_errors_and_index_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(matches!(
        repo::Repo::open(&tracked_doc(root)),
        Err(KvcError::NotARepo(_))
    ));

    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
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

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert!(r2.index.files.contains_key("x"));
    assert!(r2.chains.chain("file:x").is_some());

    assert_eq!(repo::epoch_to_iso(0), "1970-01-01T00:00:00Z");
    assert_eq!(repo::epoch_to_iso(1_609_459_200), "2021-01-01T00:00:00Z");
}

#[test]
fn init_refuses_a_second_store_for_the_same_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    assert!(matches!(
        repo::Repo::init(&tracked_doc(root)),
        Err(KvcError::AlreadyRepo(_))
    ));
}

#[test]
fn init_refuses_non_kra_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let palette = root.join("swatches.gpl");
    std::fs::write(
        &palette,
        b"GIMP Palette
",
    )
    .unwrap();
    assert!(matches!(
        repo::Repo::init(&palette),
        Err(KvcError::Unsupported(_))
    ));

    // A .kra that isn't there is not a document to track.
    assert!(matches!(
        repo::Repo::init(&root.join("ghost.kra")),
        Err(KvcError::NotARepo(_))
    ));
}

/// Two paintings in one folder are two *independent* histories that merely share a container.
/// This is the whole reason `objects/` isn't shared between them.
#[test]
fn sibling_documents_have_independent_stores() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let a = init_doc_named(root, "a.kra");
    let b = init_doc_named(root, "b.kra");
    assert_ne!(store_of(&a), store_of(&b));
    assert_eq!(store_of(&a).parent(), store_of(&b).parent());

    let mut ra = repo::Repo::open(&a).unwrap();
    commit::commit_snapshot(&mut ra, "a1", "t").unwrap();
    let mut rb = repo::Repo::open(&b).unwrap();
    commit::commit_snapshot(&mut rb, "b1", "t").unwrap();

    // Each store sees only its own document, and only its own history.
    assert_eq!(ra.commits.len(), 1);
    assert_eq!(ra.commits[0].files[0].path, "a.kra");
    assert_eq!(rb.commits[0].files[0].path, "b.kra");

    // Committing to one leaves the other's log untouched, byte for byte.
    let log_b = std::fs::read(store_of(&b).join("commits.log")).unwrap();
    std::fs::write(&a, kra_bytes(7)).unwrap();
    commit::commit_snapshot(&mut ra, "a2", "t").unwrap();
    assert_eq!(
        std::fs::read(store_of(&b).join("commits.log")).unwrap(),
        log_b
    );
}

/// The load-bearing distinction: a store that isn't reachable must never read as "never
/// versioned", because the UI answers that by offering to start tracking — which would mint an
/// The load-bearing distinction: a store that isn't reachable must never read as "never
/// versioned", because the UI answers that by offering to start tracking — which would mint an
/// empty store and orphan every version the artist saved.
#[test]
fn unreachable_store_is_not_reported_as_untracked() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("art.kra");
    let detached = dir.path().join("drive-that-isnt-plugged-in");

    assert!(matches!(
        repo::locate_failure(&doc, Some(&detached)),
        KvcError::StoreUnreachable(_)
    ));
    // Mounted but empty is genuinely "not tracked yet".
    std::fs::create_dir(&detached).unwrap();
    assert!(matches!(
        repo::locate_failure(&doc, Some(&detached)),
        KvcError::NotARepo(_)
    ));
    assert!(matches!(
        repo::locate_failure(&doc, None),
        KvcError::NotARepo(_)
    ));
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

    // A tracked document has its *store* removed, preferring the OS Recycle Bin — and the
    // artwork itself is left alone. Deleting someone's painting because they stopped versioning
    // it would be indefensible, so this half of the assertion is the important one.
    let real = dir.path().join("repo");
    std::fs::create_dir(&real).unwrap();
    let doc = init_doc(&real);
    let store = store_of(&doc);
    let used_trash = repo::Repo::delete(&doc).unwrap();
    assert!(!store.exists(), "delete should remove the store");
    assert!(doc.exists(), "delete must never touch the artwork");
    assert!(
        used_trash,
        "expected the store to move to the Recycle Bin, not be permanently deleted"
    );
}

#[test]
fn export_zip_round_trips_into_a_reopenable_repo() {
    let dir = tempfile::tempdir().unwrap();
    let r = seeded_repo(&dir);
    let root = dir.path();

    // Destination lives outside `root` — zipping into a folder being walked would grow forever.
    let out = tempfile::tempdir().unwrap();
    let zip_path = out.path().join("backup.zip");
    repo::Repo::export_zip(&tracked_doc(root), &zip_path).unwrap();

    let extracted = out.path().join("extracted");
    std::fs::create_dir(&extracted).unwrap();
    let file = std::fs::File::open(&zip_path).unwrap();
    let mut za = zip::ZipArchive::new(file).unwrap();
    za.extract(&extracted).unwrap();

    // The archive holds `<name>.kra` plus `.kvc/<slug>/`, i.e. the on-disk shape — so extracting
    // it anywhere gives a tracked document there.
    assert!(repo::Repo::is_repo(&extracted.join("art.kra")));
    let r2 = repo::Repo::open(&extracted.join("art.kra")).unwrap();
    assert_eq!(r2.commits.len(), 1);
    assert_eq!(doc_content(&extracted.join("art.kra")), maindoc(1));

    // The manifest describes the backup so a recovered zip is self-describing.
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(extracted.join("MANIFEST.json")).unwrap()).unwrap();
    assert_eq!(manifest["branch"], "main");
    assert_eq!(manifest["tipCommit"], r.commits[0].id);
    assert_eq!(manifest["appVersion"], env!("CARGO_PKG_VERSION"));
    assert!(manifest["timestamp"].as_str().unwrap().ends_with('Z'));
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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(77)).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(107)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(r.commits.len(), 2);

    // Undo c2: log shrinks, working file is untouched, index rewinds so the edit resurfaces.
    let head = commit::undo_last_commit(&mut r).unwrap();
    assert_eq!(r.commits.len(), 1);
    assert_eq!(head.unwrap().id, c1.id);
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(107));
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "art.kra" && st == "M"));

    // Undo c1 (the add): file becomes untracked again.
    let head2 = commit::undo_last_commit(&mut r).unwrap();
    assert!(head2.is_none());
    assert!(r.commits.is_empty());
    assert!(scan::scan(&r)
        .unwrap()
        .iter()
        .any(|(p, st)| p == "art.kra" && st == "U"));

    // Undo on an empty log is a no-op.
    assert!(commit::undo_last_commit(&mut r).unwrap().is_none());
}

/// `commit_selected`'s path subset survives per-document tracking — the CLI still passes it, and
/// layer-level staging will build on the same entry point.
#[test]
fn commit_selected_only_includes_named_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(10)).unwrap();
    let c1 = commit::commit_selected(&mut r, "the document", "t", Some(&["art.kra".to_string()]))
        .unwrap();
    assert_eq!(c1.files.len(), 1);
    assert_eq!(c1.files[0].path, "art.kra");
    assert!(scan::scan(&r).unwrap().is_empty());

    // A selection that matches nothing dirty errors `Nothing` rather than committing everything.
    std::fs::write(tracked_doc(root), kra_bytes(11)).unwrap();
    assert!(matches!(
        commit::commit_selected(&mut r, "nothing", "t", Some(&["missing.kra".to_string()])),
        Err(KvcError::Nothing)
    ));
    assert!(
        !scan::scan(&r).unwrap().is_empty(),
        "the edit must still be pending"
    );
}

#[test]
fn commit_records_original_size_and_it_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let body = kra_bytes(41);
    std::fs::write(tracked_doc(root), &body).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    assert_eq!(c1.files[0].original_size, body.len() as u64);

    // Survives the append-only JSONL round-trip (serde default keeps legacy lines readable).
    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r2.commits[0].files[0].original_size, body.len() as u64);

    // Deletions record 0.
    std::fs::remove_file(tracked_doc(root)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(c2.files[0].status, "D");
    assert_eq!(c2.files[0].original_size, 0);
}

#[test]
fn storage_stats_sums_per_version_and_beats_full_copies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // Two revisions that share most of their content — one repainted tile, as a real edit
    // between saves would be. Two wholly unrelated documents would say nothing about deltas.
    let shared: Vec<u8> = (0..40_000u32).map(|i| (i % 251) as u8).collect();
    let mk = |tile: &[u8]| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            (
                "img/layers/layer1",
                tiled(&[(0, 0, &shared), (0, 64, tile)]),
            ),
        ])
    };
    let v1 = mk(&[7u8; 20_000]);
    let v2 = mk(&[9u8; 20_000]);
    std::fs::write(tracked_doc(root), &v1).unwrap();
    commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    std::fs::write(tracked_doc(root), &v2).unwrap();
    commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let s = commands::compute_storage_stats(&r);
    // One row per commit, folding the FULL tree (not just the diff) each version — which for one
    // tracked document means the document, every time.
    assert_eq!(s.per_version.len(), 2);
    assert_eq!(
        (
            s.per_version[0].version,
            s.per_version[0].file_count,
            s.per_version[0].original_bytes
        ),
        (1, 1, v1.len() as u64)
    );
    assert_eq!(
        (s.per_version[1].file_count, s.per_version[1].original_bytes),
        (1, v2.len() as u64)
    );
    // Naive "full copy per version" cost is the sum; the delta store must not exceed it.
    assert_eq!(s.naive_bytes, (v1.len() + v2.len()) as u64);
    assert!(
        s.actual_bytes <= s.naive_bytes,
        "delta store should not exceed full copies"
    );
    assert_eq!(s.saved_bytes, s.naive_bytes - s.actual_bytes);

    // Each row carries its commit id + message, for the per-version cards.
    assert_eq!(s.per_version[0].message, "v1");
    assert_eq!(s.per_version[1].commit_id, r.commits[1].id);

    for row in &s.per_version {
        assert!(row.stored_bytes > 0, "v{} stored nothing", row.version);
    }
    let attributed: u64 = s.per_version.iter().map(|r| r.stored_bytes).sum();
    assert!(
        attributed <= s.actual_bytes,
        "attributed {attributed} exceeds store {}",
        s.actual_bytes
    );
}

#[test]
fn rollback_restores_tree_as_new_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(77)).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(39)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();

    let c3 = commit::rollback_to_commit(&mut r, &c1.id, "t").unwrap();
    // Working tree matches c1 again.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(77));
    // A new commit captured the restored state; nothing left to commit afterwards.
    assert_eq!(r.commits.len(), 3);
    assert!(c3.message.contains("Restored to Version 1"));
    assert!(scan::scan(&r).unwrap().is_empty());

    // Rolling back to the current head (the tree already matches) is a no-op -> Nothing.
    assert!(matches!(
        commit::rollback_to_commit(&mut r, &c3.id, "t"),
        Err(KvcError::Nothing)
    ));

    // The historical rollback commit records what it restored.
    assert_eq!(c3.restored_from.as_deref(), Some(c1.id.as_str()));
}
#[test]
fn rollback_to_tip_discards_dirty_changes_without_new_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(77)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    // Dirty the tree *after* the tip commit, then delete the document outright — the "D" branch
    // of the restore, which has to put the file back.
    std::fs::write(tracked_doc(root), kra_bytes(42)).unwrap();
    std::fs::remove_file(tracked_doc(root)).unwrap();
    assert!(!scan::scan(&r).unwrap().is_empty());

    let tip = r.branches.tip().unwrap().to_string();
    let restored = commit::rollback_to_commit(&mut r, &tip, "t").unwrap();

    assert_eq!(
        restored.id, tip,
        "discard must not synthesize a new commit id"
    );
    assert_eq!(
        r.commits.len(),
        1,
        "discarding to the tip must not record a new commit"
    );
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(77));
    assert!(scan::scan(&r).unwrap().is_empty());

    // Clean tree afterwards: rolling back to the tip again is a no-op -> Nothing.
    assert!(matches!(
        commit::rollback_to_commit(&mut r, &tip, "t"),
        Err(KvcError::Nothing)
    ));
}
#[test]
fn rollback_kra_writes_no_new_objects() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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

    let objs = count_objects(&store_of(&tracked_doc(root)));
    let c3 = commit::rollback_to_commit(&mut r, &c1.id, "t").unwrap();
    assert_eq!(
        count_objects(&store_of(&tracked_doc(root))),
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
fn parse_image_meta_rejects_oversize_canvas() {
    // Canvas dimensions drive full width*height*4 raster allocations; an absurd size from a
    // crafted maindoc must be refused up front rather than attempted.
    let xml = br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="999999" height="999999"><layers></layers></IMAGE></DOC>"#;
    assert!(matches!(
        kra::parse_image_meta(xml),
        Err(KvcError::CorruptZip(_))
    ));
    // A normal canvas still parses.
    assert!(kra::parse_image_meta(&maindoc_raster()).is_ok());
}

#[test]
fn low_memory_working_diff_matches_full_path() {
    // The opt-in low-memory diff path (re-inflating entries on demand) must produce byte-identical
    // change detection and rasters to the default in-memory path.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let cache_dir = store_of(&tracked_doc(root)).join("cache");

    let kra_bytes = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc_raster()),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
        ("mergedimage.png", b"\x89PNG\r\n\x1a\n merged".to_vec()),
    ]);

    let full = kra::parse_working(&kra_bytes, false).unwrap();
    let lazy = kra::parse_working(&kra_bytes, true).unwrap();

    // Metadata + change-detection inputs are identical.
    assert_eq!(full.tile_index(), lazy.tile_index());
    assert_eq!(
        full.entry_hash("mergedimage.png"),
        lazy.entry_hash("mergedimage.png")
    );
    assert_eq!(
        lazy.entry_bytes("mergedimage.png").map(|b| b.into_owned()),
        Some(b"\x89PNG\r\n\x1a\n merged".to_vec())
    );

    // Rasterize via the lazy path first (cache miss → on-demand re-inflate + decode), then the
    // full path (cache hit): identical key and PNG bytes either way.
    let via_lazy = lazy
        .layer_raster("img/layers/layer1", 64, 64, &cache_dir)
        .unwrap()
        .unwrap();
    let via_full = full
        .layer_raster("img/layers/layer1", 64, 64, &cache_dir)
        .unwrap()
        .unwrap();
    assert_eq!(via_lazy.key, via_full.key);
    assert_eq!(via_lazy.png, via_full.png);
}

#[test]
fn kra_layer_raster_decodes_to_png() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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

    // Painted-area bounds come from the tile grid (one 64×64 tile at origin, no decode).
    let bounds = kra::layer_bounds(&manifest.tile_index_ref(), "img/layers/layer1", 64, 64)
        .expect("layer1 has a tile → bounds");
    assert_eq!(bounds, (0, 0, 64, 64));

    // The layer decodes to a real PNG data URL.
    let cache = delta::TileCache::new();
    let raster = kra::layer_raster(&r, "art.kra", &manifest, "img", "layer1", 64, 64, &cache)
        .unwrap()
        .expect("layer1 should decode to a raster");
    assert!(raster.url.starts_with("data:image/png;base64,"));
    assert!(raster.url.len() > 100, "expected a non-trivial PNG payload");

    // The composite entry is surfaced too.
    let comp = kra::entry_data_url(&r, "art.kra", &manifest, "mergedimage.png").unwrap();
    assert!(comp.unwrap().starts_with("data:image/png;base64,"));
}

#[test]
fn kra_layer_raster_fills_untiled_region_from_default_pixel() {
    // Krita only stores tiles for a layer's painted-on regions; anywhere else — e.g. most of a
    // freshly created, uniformly-filled "Background" layer — is filled from a sibling
    // `<entry>.defaultpixel` file instead. A 128x64 canvas with one real 64x64 tile on the left
    // and a `.defaultpixel` covering the rest must decode to that fill color on the right, not
    // transparent black.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let maindoc = br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="128" height="64"><layers>
<layer name="Background" uuid="bg" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer1"/>
</layers></IMAGE></DOC>"#
        .to_vec();

    let kra = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc),
        (
            "img/layers/layer1",
            tiled(&[(0, 0, &solid_rgba_tile(10, 20, 30, 255))]),
        ),
        // BGRA opaque white — the untiled region's fill.
        ("img/layers/layer1.defaultpixel", vec![255, 255, 255, 255]),
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

    let cache = delta::TileCache::new();
    let rst = kra::layer_raster(&r, "art.kra", &manifest, "img", "layer1", 128, 64, &cache)
        .unwrap()
        .expect("layer1 should decode to a raster");

    let (pixels, w, h, has_alpha, _) = raster::decode_png_plain(&rst.png).expect("valid PNG");
    assert!(has_alpha);
    let px = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        &pixels[i..i + 4]
    };
    // Untiled region (right half): the default-pixel fill, fully opaque — not transparent.
    assert_eq!(px(w * 3 / 4, h / 2), [255, 255, 255, 255]);
    // Tiled region (left half): the real painted color, unaffected by the default pixel.
    assert_eq!(px(w / 4, h / 2), [10, 20, 30, 255]);
}

#[test]
fn parse_image_meta_reads_richer_fields() {
    let xml = br#"<!DOCTYPE DOC>
<DOC><IMAGE name="img" width="64" height="64" x-res="300" y-res="300" colorspacename="RGBA" profile="sRGB-elle-V2-srgbtrc.icc"><layers>
<layer name="Base" uuid="base" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer1" visible="1"/>
<layer name="Hidden" uuid="hid" opacity="128" compositeop="multiply" nodetype="grouplayer" filename="" visible="0"/>
</layers></IMAGE></DOC>"#;
    let meta = kra::parse_image_meta(xml).unwrap();
    assert_eq!(meta.dpi, 300.0);
    assert_eq!(meta.color_model, "RGBA");
    assert_eq!(meta.color_profile, "sRGB-elle-V2-srgbtrc.icc");
    assert!(meta.layers[0].visible);
    assert_eq!(meta.layers[0].kind, "paintlayer");
    assert!(!meta.layers[1].visible);
    assert_eq!(meta.layers[1].kind, "grouplayer");
}

#[test]
fn kra_changed_entry_paths_flags_edited_layer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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

/// A modified layer carries its **own** changed-pixel highlight (mask + outline + region box),
/// diffed from that layer's before/after rasters — not the composite's. An unchanged layer carries
/// none, so the viewer draws no overlay when it's selected.
#[test]
fn modified_layer_carries_its_own_overlay() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // Two layers on both sides; only layer1 (Base) changes between v1 and v2. layer2 (Top) is
    // byte-identical, so it must come back "unchanged" with no per-layer overlay.
    let mk = |base_tile: Vec<u8>| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            (
                "maindoc.xml",
                maindoc_layers(&[("Base", "base", "layer1"), ("Top", "top", "layer2")]),
            ),
            ("img/layers/layer1", tiled(&[(0, 0, &base_tile)])),
            (
                "img/layers/layer2",
                tiled(&[(0, 0, &solid_rgba_tile(40, 50, 60, 255))]),
            ),
        ])
    };
    std::fs::write(root.join("art.kra"), mk(solid_rgba_tile(10, 20, 30, 255))).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    std::fs::write(root.join("art.kra"), mk(solid_rgba_tile(200, 20, 30, 255))).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    let parent_tree = commit::tree_at_commit(&r.commits, &c1.id).unwrap();
    let f = c2.files.iter().find(|f| f.path == "art.kra").unwrap();
    let dto = commands::committed_art_dto(&r, f, parent_tree.get("art.kra"), true, None).unwrap();

    let base = dto
        .layers
        .iter()
        .find(|l| l.name == "Base")
        .expect("Base layer present");
    assert_eq!(base.change, "modified");
    assert!(
        base.diff_image.is_some() && base.diff_outline.is_some() && !base.regions.is_empty(),
        "a modified layer must carry its own mask, outline, and region box"
    );
    // The region box must be normalized 0..1 (the frontend scales it to the viewBox). A
    // pixel-scaled region overflows the canvas bottom/right — this pins that regression.
    let region = &serde_json::to_value(base).unwrap()["regions"][0];
    for k in ["x", "y", "w", "h"] {
        let v = region[k].as_f64().unwrap();
        assert!(
            (0.0..=1.0).contains(&v),
            "region {k}={v} must be normalized 0..1"
        );
    }

    let top = dto
        .layers
        .iter()
        .find(|l| l.name == "Top")
        .expect("Top layer present");
    assert_eq!(top.change, "unchanged");
    assert!(
        top.diff_image.is_none() && top.diff_outline.is_none() && top.regions.is_empty(),
        "an unchanged layer must carry no per-layer overlay"
    );
}

/// A second rasterization of identical content must come from `.kvc/cache/`, not a re-decode.
#[test]
fn layer_raster_reads_from_disk_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    .unwrap()
    .url;

    // Exactly one cached PNG was written; replace its bytes to prove the next read uses it.
    let cache_dir = store_of(&tracked_doc(root)).join("cache");
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
    .unwrap()
    .url;
    assert_eq!(second, raster::png_bytes_to_data_url(b"MARKER"));
    assert_ne!(first, second);

    // Same pixels via the in-memory working path share the cache entry.
    let working = kra::parse_working(&kra_bytes, false).unwrap();
    let from_working = working
        .layer_raster("img/layers/layer1", 64, 64, &cache_dir)
        .unwrap()
        .unwrap()
        .url;
    assert_eq!(from_working, second);
}

// --- branching: create / switch / merge / delete ---------------------------------------

/// Fresh repo with two committed files, ready for branch tests.
/// A store with one committed version of its document. The old fixture seeded two `.gpl` files
/// to exercise multi-file commits; one store now tracks exactly one `.kra`, so the second file
/// has nowhere to go.
fn seeded_repo(dir: &tempfile::TempDir) -> repo::Repo {
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(1)).unwrap();
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
    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on idea", "t").unwrap();
    assert_eq!(c2.parents, vec![c1.id.clone()]);
    assert_eq!(c2.branch, "idea");

    branch::switch_branch(&mut r, "main").unwrap();
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
    assert!(scan::scan(&r).unwrap().is_empty());

    branch::switch_branch(&mut r, "idea").unwrap();
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(4));
    assert!(scan::scan(&r).unwrap().is_empty());

    // State survives a reopen (branches.json persisted).
    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r2.branches.current, "idea");
    assert_eq!(r2.branches.tip(), Some(c2.id.as_str()));
}

#[test]
fn switch_skips_unchanged_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    // Both branches sit on the same tip, so the document is identical on each — switching must
    // not rewrite it. (Rewriting a several-hundred-MB `.kra` for nothing is the cost this
    // avoids, and a bumped mtime would also make the next scan re-hash it.)
    branch::create_branch(&mut r, "idea", None).unwrap();
    let before = std::fs::metadata(tracked_doc(root))
        .unwrap()
        .modified()
        .unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    let after = std::fs::metadata(tracked_doc(root))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "unchanged file was rewritten on switch");
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));

    // And when it *has* changed, it is rewritten.
    branch::switch_branch(&mut r, "idea").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
    commit::commit_snapshot(&mut r, "on idea", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
}

#[test]
fn switch_refuses_dirty_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    branch::create_branch(&mut r, "idea", None).unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(23)).unwrap();
    assert!(matches!(
        branch::switch_branch(&mut r, "idea"),
        Err(KvcError::DirtyTree)
    ));
    // The unsaved edit is untouched.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(23));
    assert_eq!(r.branches.current, "main");
}

#[test]
fn create_branch_at_an_older_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].id.clone();

    std::fs::write(tracked_doc(root), kra_bytes(19)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(20)).unwrap();
    let c3 = commit::commit_snapshot(&mut r, "c3", "t").unwrap();

    // Branch off the *first* version, two commits back from the tip.
    branch::create_branch_at(&mut r, "retry", &c1).unwrap();
    assert_eq!(r.branches.current, "retry");
    assert_eq!(r.branches.tip(), Some(c1.as_str()));
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
    assert!(scan::scan(&r).unwrap().is_empty(), "tree left dirty");

    // The skipped commits stay reachable from the branch they were made on.
    branch::switch_branch(&mut r, "main").unwrap();
    assert_eq!(r.branches.tip(), Some(c3.id.as_str()));
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(20));
    assert!(commit::ancestors(&r.commits, &c3.id).contains(&c2.id));

    // Unknown id, duplicate name, and a dirty tree are all refused.
    assert!(matches!(
        branch::create_branch_at(&mut r, "nope", "deadbeef"),
        Err(KvcError::NoCommit(_))
    ));
    assert!(matches!(
        branch::create_branch_at(&mut r, "retry", &c1),
        Err(KvcError::BranchExists(_))
    ));
    std::fs::write(tracked_doc(root), kra_bytes(23)).unwrap();
    assert!(matches!(
        branch::create_branch_at(&mut r, "other", &c1),
        Err(KvcError::DirtyTree)
    ));
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(23));
    assert_eq!(r.branches.current, "main");
}

#[test]
fn merge_fast_forward() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(5)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    // main has not moved -> fast-forward: no new commit, tip jumps, tree materializes.
    let merged = branch::merge_branch(&mut r, "feat", "t").unwrap();
    assert_eq!(merged.id, c2.id);
    assert_eq!(r.commits.len(), 2);
    assert_eq!(r.branches.tip(), Some(c2.id.as_str()));
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(5));
    assert!(scan::scan(&r).unwrap().is_empty());

    // Merging again: nothing to do.
    assert!(matches!(
        branch::merge_branch(&mut r, "feat", "t"),
        Err(KvcError::NothingToMerge(_))
    ));
}

#[test]
fn merge_three_way_conflict_takes_source_and_flags() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(5)).unwrap();
    commit::commit_snapshot(&mut r, "feat edits a", "t").unwrap();

    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(2)).unwrap();
    commit::commit_snapshot(&mut r, "main edits a", "t").unwrap();

    let m = branch::merge_branch(&mut r, "feat", "t").unwrap();
    let entry = m.files.iter().find(|f| f.path == "art.kra").unwrap();
    assert_eq!(entry.status, "C");
    // Source wins on disk.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(5));
    assert!(scan::scan(&r).unwrap().is_empty());
}

#[test]
fn merge_three_way_delete_vs_modify_conflict_keeps_edit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::remove_file(tracked_doc(root)).unwrap();
    commit::commit_snapshot(&mut r, "feat deletes a", "t").unwrap();

    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(2)).unwrap();
    commit::commit_snapshot(&mut r, "main edits a", "t").unwrap();

    let m = branch::merge_branch(&mut r, "feat", "t").unwrap();
    let entry = m.files.iter().find(|f| f.path == "art.kra").unwrap();
    assert_eq!(entry.status, "C");
    // The edit is kept rather than losing it to the delete.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(2));
    assert!(scan::scan(&r).unwrap().is_empty());
}

#[test]
fn list_commits_scoped_by_branch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(32)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(2)).unwrap();
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
    std::fs::remove_file(store_of(&tracked_doc(root)).join("branches.json")).unwrap();
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.branches.current, "main");
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));

    // The next commit persists branches.json and chains parentage correctly.
    std::fs::write(tracked_doc(root), kra_bytes(8)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    assert_eq!(c2.parents, vec![c1.id.clone()]);
    assert!(store_of(&tracked_doc(root)).join("branches.json").is_file());
}

/// Gap #10: `branches.json`'s generation counter must bump on every write that touches it,
/// through every write path — `save()`, `save_branches()` — so a read command can tell a write
/// landed mid-read and retry instead of returning a torn snapshot.
#[test]
fn generation_bumps_on_every_branches_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);

    let g0 = repo::read_branches_generation(&store_of(&tracked_doc(root))).unwrap();
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.branches.generation, g0);

    // A full save() (the commit path) bumps it.
    std::fs::write(tracked_doc(root), kra_bytes(7)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    let g1 = repo::read_branches_generation(&store_of(&tracked_doc(root))).unwrap();
    assert!(g1 > g0, "commit's save() must bump the generation");
    assert_eq!(
        r.branches.generation, g1,
        "in-memory copy matches what was written"
    );

    // save_branches() (the open_light mutator path — branch create/switch) bumps it too.
    branch::create_branch(&mut r, "idea", None).unwrap();
    let g2 = repo::read_branches_generation(&store_of(&tracked_doc(root))).unwrap();
    assert!(g2 > g1, "save_branches() must bump the generation too");
}

#[test]
fn branches_json_corruption_recovers_from_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let r = seeded_repo(&dir);
    drop(r);

    // One commit's save() already backed up init's branches.json to branches.json.bak before
    // overwriting it with the post-commit tip.
    assert!(store_of(&tracked_doc(root))
        .join("branches.json.bak")
        .is_file());
    std::fs::write(
        store_of(&tracked_doc(root)).join("branches.json"),
        b"{not json",
    )
    .unwrap();

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(
        r2.branches.current, "main",
        "a corrupt primary should fall back to the previous generation, not fail to open"
    );
}

#[test]
fn index_json_corruption_recovers_from_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let r = seeded_repo(&dir);
    drop(r);

    assert!(store_of(&tracked_doc(root))
        .join("index.json.bak")
        .is_file());
    std::fs::write(
        store_of(&tracked_doc(root)).join("index.json"),
        b"{not json",
    )
    .unwrap();

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    // The backup is the pre-commit (empty) index — recovery is "don't fail to open", not
    // byte-perfect restoration of the corrupted generation.
    assert!(r2.index.files.is_empty());
}

#[test]
fn stashes_json_corruption_recovers_from_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    // stashes.json is written for the first time by the seeding commit's save() — a second save
    // is needed before a `.bak` generation exists to fall back to.
    std::fs::write(tracked_doc(root), kra_bytes(8)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    drop(r);

    assert!(store_of(&tracked_doc(root))
        .join("stashes.json.bak")
        .is_file());
    std::fs::write(
        store_of(&tracked_doc(root)).join("stashes.json"),
        b"{not json",
    )
    .unwrap();

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert!(r2.stashes.stashes.is_empty());
}

#[test]
fn open_fails_cleanly_when_primary_and_backup_both_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let r = seeded_repo(&dir);
    drop(r);

    assert!(store_of(&tracked_doc(root))
        .join("branches.json.bak")
        .is_file());
    std::fs::write(
        store_of(&tracked_doc(root)).join("branches.json"),
        b"{not json",
    )
    .unwrap();
    std::fs::write(
        store_of(&tracked_doc(root)).join("branches.json.bak"),
        b"{also not json",
    )
    .unwrap();

    assert!(matches!(
        repo::Repo::open(&tracked_doc(root)),
        Err(KvcError::BadIndex(_))
    ));
}

#[test]
fn undo_respects_branch_tip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    let c1 = r.commits[0].clone();

    branch::create_branch(&mut r, "feat", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(32)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "on feat", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(2)).unwrap();
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
    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
    commit::commit_snapshot(&mut r, "on idea", "t").unwrap();

    branch::create_branch(&mut r, "third", Some("main")).unwrap();
    assert_eq!(r.branches.current, "third");
    assert_eq!(r.branches.tip(), Some(c1.id.as_str()));
    // The working tree was materialized to main's files.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
    assert!(scan::scan(&r).unwrap().is_empty());

    // Unknown base -> error; unsaved changes -> refused, nothing moves.
    assert!(matches!(
        branch::create_branch(&mut r, "x", Some("ghost")),
        Err(KvcError::NoBranch(_))
    ));
    std::fs::write(tracked_doc(root), kra_bytes(22)).unwrap();
    assert!(matches!(
        branch::create_branch(&mut r, "x", Some("idea")),
        Err(KvcError::DirtyTree)
    ));
    assert_eq!(r.branches.current, "third");
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(22));
}

#[test]
fn commit_crc_skip_reuses_unchanged_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    let rb = kra::parse_working(&rebuilt, false).unwrap();
    let wk = kra::parse_working(&kra2, false).unwrap();
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
        let objs = count_objects(&store_of(&tracked_doc(root)));
        let c3 = commit::commit_snapshot(&mut r, "rebuilt", "t").unwrap();
        assert_eq!(c3.files[0].content.clone().unwrap(), h2);
        assert_eq!(count_objects(&store_of(&tracked_doc(root))), objs);
    }
}

// --- chains persistence: sharded format, legacy monolith migration, skip-on-clean --------

fn shard_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(store_of(&tracked_doc(root)).join("chains"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
fn chains_sharded_format_and_legacy_monolith_migration() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    let h = r.store_stream("file:x", b"some data").unwrap();
    r.save().unwrap();
    // Fresh repos write per-file shards, never a monolith.
    assert_eq!(shard_files(root).len(), 1);
    assert!(!store_of(&tracked_doc(root)).join("chains.bin").exists());

    // Simulate a pre-sharding repo: all chains in one monolithic chains.bin, no shards.
    // Real monoliths predate KVCC2, so they carry the old 4-field Version (with `object`).
    #[derive(serde::Serialize)]
    struct V1 {
        hash: String,
        object: String,
        base: Option<String>,
        chain_len: usize,
    }
    let all = r.chains.export_all();
    let legacy: std::collections::BTreeMap<String, Vec<V1>> = all
        .0
        .iter()
        .map(|(k, vs)| {
            (
                k.clone(),
                vs.iter()
                    .map(|v| V1 {
                        hash: v.hash.clone(),
                        object: v.object_name(),
                        base: v.base.clone(),
                        chain_len: v.chain_len,
                    })
                    .collect(),
            )
        })
        .collect();
    let monolith = zstd::encode_all(&bincode::serialize(&legacy).unwrap()[..], 1).unwrap();
    std::fs::write(store_of(&tracked_doc(root)).join("chains.bin"), &monolith).unwrap();
    std::fs::remove_dir_all(store_of(&tracked_doc(root)).join("chains")).unwrap();

    // Opens read the monolith transparently...
    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r2.reconstruct("file:x", &h).unwrap(), b"some data");

    // ...and the next save splits it into shards and retires it.
    r2.store_stream("file:y", b"more").unwrap();
    r2.save().unwrap();
    assert_eq!(shard_files(root).len(), 2, "one shard per tracked file");
    assert!(!store_of(&tracked_doc(root)).join("chains.bin").exists());
    let r3 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert!(r3.chains.chain("file:x").is_some() && r3.chains.chain("file:y").is_some());
    assert_eq!(r3.reconstruct("file:x", &h).unwrap(), b"some data");

    // The oldest format (chains.json) migrates the same way.
    let json = serde_json::to_vec(&all).unwrap();
    std::fs::write(store_of(&tracked_doc(root)).join("chains.json"), &json).unwrap();
    std::fs::remove_dir_all(store_of(&tracked_doc(root)).join("chains")).unwrap();
    let mut r4 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r4.reconstruct("file:x", &h).unwrap(), b"some data");
    r4.store_stream("file:x", b"newer data").unwrap();
    r4.save().unwrap();
    assert!(!store_of(&tracked_doc(root)).join("chains.json").exists());
    assert!(!shard_files(root).is_empty());
}

#[test]
fn switch_skips_chains_rewrite_commit_touches_only_changed_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    branch::create_branch(&mut r, "idea", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(4)).unwrap();
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
    assert_eq!(originals.len(), 1, "one shard per tracked document");
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

    // A commit rewrites exactly the tracked document's shard.
    let before: std::collections::HashMap<_, _> = shard_files(root)
        .into_iter()
        .map(|p| (p.clone(), std::fs::read(&p).unwrap()))
        .collect();
    std::fs::write(tracked_doc(root), kra_bytes(3)).unwrap();
    commit::commit_snapshot(&mut r, "on main", "t").unwrap();
    let changed = shard_files(root)
        .into_iter()
        .filter(|p| std::fs::read(p).unwrap() != before[p])
        .count();
    assert_eq!(changed, 1, "only the committed file's shard is rewritten");

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // `extra` is deliberately large and near-incompressible so its versions bsdiff-chain — the
    // patch-chain bases GC must not reclaim. Under the folder model this was a second tracked
    // file; a `.kra` entry gives the same shape inside one document.
    let bulk = |seed: u8| -> Vec<u8> {
        (0..90_000u32)
            .map(|i| ((i as u8) ^ seed).wrapping_mul(31))
            .collect()
    };
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
    std::fs::write(root.join("art.kra"), mk(b"tileAAAA", &bulk(1))).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    std::fs::write(root.join("art.kra"), mk(b"tileBBBB", &bulk(2))).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "c2", "t").unwrap();

    // Branch with a divergent edit, then go back to main.
    branch::create_branch(&mut r, "scrap", None).unwrap();
    std::fs::write(root.join("art.kra"), mk(b"tileSCRAP", &bulk(3))).unwrap();
    commit::commit_snapshot(&mut r, "scrap edit", "t").unwrap();
    branch::switch_branch(&mut r, "main").unwrap();

    // Another main commit, then undo it (its edits resurface as pending and get re-committed
    // — dedup makes that cheap), and delete scrap: the stranded branch history is the garbage.
    std::fs::write(root.join("art.kra"), mk(b"tileCCCC", &bulk(4))).unwrap();
    commit::commit_snapshot(&mut r, "c3", "t").unwrap();
    commit::undo_last_commit(&mut r).unwrap();
    commit::commit_snapshot(&mut r, "c3 again", "t").unwrap();
    branch::delete_branch(&mut r, "scrap").unwrap();

    let objs_before = count_objects(&store_of(&tracked_doc(root)));

    // Dry run reports without deleting.
    let dry = gc::collect_garbage(&mut r, true).unwrap();
    assert!(dry.dry_run && dry.bytes_reclaimed > 0 && dry.versions_removed > 0);
    assert_eq!(
        count_objects(&store_of(&tracked_doc(root))),
        objs_before,
        "dry run must not delete"
    );

    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert!(!report.dry_run);
    // The undone commit already left the log (undo removes it; only its objects orphan);
    // the deleted branch's commit is dropped here.
    assert_eq!(report.commits_removed, 1, "the stranded branch commit");
    assert!(report.bytes_reclaimed > 0);
    assert!(count_objects(&store_of(&tracked_doc(root))) < objs_before);

    // Everything reachable reconstructs byte-for-byte, in this session and after reopen.
    for repo_ref in [&r, &repo::Repo::open(&tracked_doc(root)).unwrap()] {
        for (cid, tile, extra) in [
            (&c1.id, b"tileAAAA".as_slice(), bulk(1)),
            (&c2.id, b"tileBBBB".as_slice(), bulk(2)),
        ] {
            let doc = commit::file_at_commit(repo_ref, "art.kra", cid).unwrap();
            let l = tiles::parse(&kra::read_entry(&doc, "img/layers/layer1").unwrap()).unwrap();
            assert_eq!(l.tiles[0].data, tile);
            // The bsdiff-chained entry: its patch bases had to survive the sweep.
            assert_eq!(kra::read_entry(&doc, "img/extra.bin").unwrap(), extra);
        }
    }

    // The tree is still clean and a fresh commit works after GC.
    assert!(scan::scan(&r).unwrap().is_empty());
    std::fs::write(root.join("art.kra"), mk(b"tileDDDD", &bulk(5))).unwrap();
    commit::commit_snapshot(&mut r, "post-gc", "t").unwrap();
    assert!(scan::scan(&r).unwrap().is_empty());
}

/// True if `dir` (or anything under it) contains at least one regular file.
fn any_file_under(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                let p = e.path();
                p.is_file() || (p.is_dir() && any_file_under(&p))
            })
        })
        .unwrap_or(false)
}

/// Gap #5: a real cleanup must quarantine dead objects into `.kvc/trash/`, not unlink them —
/// so a bad reachability call, or cleanup run right after a branch delete, stays recoverable.
#[test]
fn cleanup_moves_dead_objects_to_trash_instead_of_deleting() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(7)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(9)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    // Undoing c2 drops it from the log — its stored version is now referenced by nothing.
    commit::undo_last_commit(&mut r).unwrap();

    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert!(
        report.objects_deleted > 0,
        "expected c2's orphaned object to be swept"
    );

    let trash = store_of(&tracked_doc(root)).join("trash");
    assert!(trash.is_dir(), "quarantined objects land in .kvc/trash");
    let runs: Vec<_> = std::fs::read_dir(&trash).unwrap().flatten().collect();
    assert_eq!(runs.len(), 1, "one run directory for this cleanup");
    assert!(
        any_file_under(&runs[0].path()),
        "the orphaned object was moved into trash, not deleted"
    );
}

/// Same as above, but for a whole dead pack file (large commits pack instead of writing loose
/// objects) — the fully-dead-pack branch of the sweep must quarantine too.
#[test]
fn cleanup_moves_dead_packs_to_trash() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // 40 distinct tiles (> PACK_MIN_OBJECTS) packs into one file, per
    // `large_commit_packs_objects_and_reconstructs`.
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
    commit::commit_snapshot(&mut r, "packed", "t").unwrap();
    assert_eq!(
        std::fs::read_dir(store_of(&tracked_doc(root)).join("objects/pack"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
            .count(),
        1,
        "the tile batch must have packed"
    );

    // Undo strands the whole commit — the pack backing it is now 100% dead.
    commit::undo_last_commit(&mut r).unwrap();
    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert!(report.objects_deleted > 0);

    assert_eq!(
        std::fs::read_dir(store_of(&tracked_doc(root)).join("objects/pack"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
            .count(),
        0,
        "the dead pack must be gone from objects/pack"
    );
    let trash = store_of(&tracked_doc(root)).join("trash");
    let runs: Vec<_> = std::fs::read_dir(&trash).unwrap().flatten().collect();
    assert_eq!(runs.len(), 1);
    let quarantined_pack = std::fs::read_dir(runs[0].path().join("pack"))
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert_eq!(
        quarantined_pack, 1,
        "the dead pack landed in trash, not deleted"
    );
}

/// A dry run reports what it would reclaim without moving anything — trash must stay untouched.
#[test]
fn dry_run_cleanup_never_touches_trash() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(7)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(9)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    commit::undo_last_commit(&mut r).unwrap();

    let report = gc::collect_garbage(&mut r, true).unwrap();
    assert!(report.dry_run && report.bytes_reclaimed > 0);
    assert!(
        !store_of(&tracked_doc(root)).join("trash").exists(),
        "dry run must not create or populate a trash run"
    );
}

// --- objects layout: sharded writes, flat legacy reads -----------------------------------

#[test]
fn objects_sharded_layout_with_flat_read_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let data = b"object payload".to_vec();
    let h = r.store_stream("file:x", &data).unwrap();
    let objects = store_of(&tracked_doc(root)).join("objects");
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
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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
    let packs: Vec<_> = std::fs::read_dir(store_of(&tracked_doc(root)).join("objects/pack"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
        .collect();
    assert_eq!(packs.len(), 1, "one pack per large batch");
    let loose = count_objects(&store_of(&tracked_doc(root))) - packs.len();
    assert!(
        loose < 10,
        "tiles must be packed, not loose (found {loose} loose objects)"
    );

    // Same-session restore and fresh-open restore both round-trip from the pack.
    let restored = commit::file_at_commit(&r, "art.kra", &c1.id).unwrap();
    let l = tiles::parse(&kra::read_entry(&restored, "img/layers/layer1").unwrap()).unwrap();
    assert_eq!(l.tiles.len(), 40);
    assert_eq!(l.tiles[7].data, tiles_v1[7].2);

    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let restored2 = commit::file_at_commit(&r2, "art.kra", &c1.id).unwrap();
    assert_eq!(restored, restored2);

    // Re-committing identical content dedups against packed objects (no new pack, no loose).
    drop(r2);
    std::fs::remove_file(root.join("art.kra")).unwrap();
    std::fs::write(root.join("art.kra"), &kra).unwrap();
    let before = count_objects(&store_of(&tracked_doc(root)));
    // Deleting + rewriting identical bytes leaves content identical to the tip — a clean scan
    // (mtime changed but hash matches) means nothing to commit.
    assert!(matches!(
        commit::commit_snapshot(&mut r, "again", "t"),
        Err(KvcError::Nothing)
    ));
    assert_eq!(count_objects(&store_of(&tracked_doc(root))), before);
}

// --- incremental .kra materialization on switch ------------------------------------------

#[test]
fn kra_switch_materializes_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

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

// --- commits.log: append-only history, legacy migration, torn-tail tolerance -------------

#[test]
fn commits_log_appends_and_migrates_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    let log = store_of(&tracked_doc(root)).join("commits.log");
    assert!(log.is_file(), "init writes the log");
    assert!(!store_of(&tracked_doc(root)).join("commits.json").exists());

    std::fs::write(tracked_doc(root), kra_bytes(15)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(16)).unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 2, "one JSON line per commit");

    // Legacy migration: fabricate a commits.json-era repo from the same history.
    let commits = r.commits.clone();
    drop(r);
    std::fs::write(
        store_of(&tracked_doc(root)).join("commits.json"),
        serde_json::to_vec(&commits).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(&log).unwrap();
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.commits.len(), 2, "legacy commits.json readable");
    std::fs::write(tracked_doc(root), kra_bytes(17)).unwrap();
    commit::commit_snapshot(&mut r, "c3", "t").unwrap();
    assert!(log.is_file(), "first save writes the log");
    assert!(
        !store_of(&tracked_doc(root)).join("commits.json").exists(),
        "legacy file retired after the log is in place"
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 3);

    // Torn tail: a crash mid-append leaves a partial line — dropped on read, scrubbed on the
    // next save (branches.json is written after the log, so the torn record was never a tip).
    drop(r);
    let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    f.write_all(b"{\"id\":\"torn").unwrap();
    drop(f);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.commits.len(), 3, "torn tail dropped, history intact");

    // Undo rewrites the log without the popped commit (and scrubs the torn tail with it).
    commit::undo_last_commit(&mut r).unwrap();
    assert_eq!(r.commits.len(), 2);
    drop(r);
    let r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.commits.len(), 2, "log rewritten after undo");
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert!(!text.contains("torn"));
}

// --- undo rewinds the index from the recorded file hash (no reconstruct-to-hash) ---------

#[test]
fn undo_uses_recorded_file_hash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let doc1 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, &[1u8; 64][..])])),
    ]);
    std::fs::write(root.join("art.kra"), &doc1).unwrap();
    commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    let doc2 = pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, &[2u8; 64][..])])),
    ]);
    std::fs::write(root.join("art.kra"), &doc2).unwrap();
    commit::commit_snapshot(&mut r, "v2", "t").unwrap();

    commit::undo_last_commit(&mut r).unwrap();
    let tf = r.index.files.get("art.kra").unwrap();
    assert_eq!(
        tf.hash,
        repo::hash_bytes(&doc1),
        "index rewound to v1's exact on-disk file hash (recorded at commit time)"
    );
    // Soft undo leaves the v2 bytes on disk — the next scan must flag them as modified.
    assert_eq!(
        scan::scan(&r).unwrap(),
        vec![("art.kra".to_string(), "M".to_string())]
    );
}

// --- GC: raster-cache prune, stale-filter wipe, tmp sweep, pack consolidation ------------

#[test]
fn gc_prunes_cache_and_sweeps_stale_tmp() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    r.config.cache_max_bytes = 300; // in-memory only — tiny budget for the test

    // 5 x 100-byte cache PNGs with strictly increasing mtimes.
    let cache = store_of(&tracked_doc(root)).join("cache");
    let base = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for i in 0..5u32 {
        let p = cache.join(format!("entry{i}.png"));
        std::fs::write(&p, [0u8; 100]).unwrap();
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(base + std::time::Duration::from_secs(i as u64 * 60))
            .unwrap();
    }
    // One stale .tmp (crash leftover, >1h old) and one fresh .tmp (in-flight, must survive).
    let stale = store_of(&tracked_doc(root)).join("config.tmp");
    std::fs::write(&stale, [0u8; 40]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
        .unwrap();
    // (In the pack dir with a name no real atomic write uses — GC's own `repo.save()`
    // legitimately creates and renames `.kvc/*.tmp` names mid-run.)
    let fresh = store_of(&tracked_doc(root)).join("objects/pack/inflight.tmp");
    std::fs::create_dir_all(fresh.parent().unwrap()).unwrap();
    std::fs::write(&fresh, [0u8; 40]).unwrap();

    // Dry run: reports the over-budget cache bytes + the stale tmp, touches nothing.
    let dry = gc::collect_garbage(&mut r, true).unwrap();
    assert_eq!(
        dry.cache_bytes_reclaimed, 200,
        "500 bytes cached, 300 budget"
    );
    assert_eq!(dry.bytes_reclaimed, 40, "the stale tmp file");
    assert!(cache.join("entry0.png").exists() && stale.exists());

    // Real run: prunes oldest-first to budget, writes the filter marker, sweeps only the
    // stale tmp.
    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert_eq!(report.cache_bytes_reclaimed, 200);
    assert!(!cache.join("entry0.png").exists() && !cache.join("entry1.png").exists());
    assert!(cache.join("entry4.png").exists());
    assert_eq!(
        std::fs::read_to_string(cache.join(".filter-version")).unwrap(),
        raster::FILTER_VERSION
    );
    assert!(!stale.exists(), "stale tmp swept");
    assert!(fresh.exists(), "fresh tmp untouched");

    // Stale filter marker: the whole cache is wiped regardless of budget.
    std::fs::write(cache.join(".filter-version"), "box0").unwrap();
    std::fs::write(cache.join("old-filter.png"), [0u8; 50]).unwrap();
    let report = gc::collect_garbage(&mut r, false).unwrap();
    assert!(report.cache_bytes_reclaimed >= 50, "stale-filter wipe");
    assert!(!cache.join("old-filter.png").exists());
    assert!(!cache.join("entry4.png").exists(), "wipe takes everything");
    assert_eq!(
        std::fs::read_to_string(cache.join(".filter-version")).unwrap(),
        raster::FILTER_VERSION
    );
}

#[test]
fn gc_consolidates_small_packs() {
    use krita_vc_lib::gc;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // 9 commits, each rewriting 33 tiles with fresh content — every commit crosses
    // PACK_MIN_OBJECTS and writes its own small pack.
    let doc_of = |round: u8| {
        // First byte = round, rest = tile index: unique content per (round, tile) so
        // content-addressed dedup can't collapse tiles across rounds.
        let datas: Vec<Vec<u8>> = (0..33u8)
            .map(|i| {
                let mut d = vec![i; 300];
                d[0] = round;
                d
            })
            .collect();
        let items: Vec<(i64, i64, &[u8])> = datas
            .iter()
            .enumerate()
            .map(|(i, d)| (i as i64 * 64, 0i64, d.as_slice()))
            .collect();
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&items)),
        ])
    };
    let mut ids = Vec::new();
    for round in 0..6u8 {
        std::fs::write(root.join("art.kra"), doc_of(round)).unwrap();
        ids.push(
            commit::commit_snapshot(&mut r, &format!("r{round}"), "t")
                .unwrap()
                .id,
        );
    }
    let packs = |root: &std::path::Path| -> usize {
        std::fs::read_dir(store_of(&tracked_doc(root)).join("objects/pack"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
                    .count()
            })
            .unwrap_or(0)
    };
    // Below the consolidation threshold: packs stay as-is.
    let before = packs(root);
    assert!(before >= 6, "one pack per large commit, got {before}");
    gc::collect_garbage(&mut r, false).unwrap();
    assert_eq!(packs(root), before, "under {} packs: no consolidation", 8);

    for round in 6..9u8 {
        std::fs::write(root.join("art.kra"), doc_of(round)).unwrap();
        ids.push(
            commit::commit_snapshot(&mut r, &format!("r{round}"), "t")
                .unwrap()
                .id,
        );
    }
    assert!(packs(root) >= 8);
    gc::collect_garbage(&mut r, false).unwrap();
    assert_eq!(packs(root), 1, "small live packs merged into one");

    // Every version still reconstructs from the consolidated pack — this session and reopened.
    for repo_ref in [&r, &repo::Repo::open(&tracked_doc(root)).unwrap()] {
        for (round, id) in ids.iter().enumerate() {
            let bytes = commit::file_at_commit(repo_ref, "art.kra", id).unwrap();
            let block =
                tiles::parse(&kra::read_entry(&bytes, "img/layers/layer1").unwrap()).unwrap();
            assert_eq!(block.tiles.len(), 33);
            let expected = {
                let mut d = vec![0u8; 300];
                d[0] = round as u8;
                d
            };
            assert_eq!(block.tiles[0].data, expected);
        }
    }
}

// --- chains format: KVCC2 shards, legacy (object-carrying) shards stay readable -----------

#[test]
fn chains_read_legacy_shards_and_rewrite_as_kvcc2() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);

    // Fabricate a pre-KVCC2 shard by hand: bare zstd(bincode) with the old 4-field Version
    // (including the redundant `object`), plus its loose object, exactly as the old code
    // laid them out.
    #[derive(serde::Serialize)]
    struct V1 {
        hash: String,
        object: String,
        base: Option<String>,
        chain_len: usize,
    }
    let content = b"hello legacy".to_vec();
    let hash = repo::hash_bytes(&content);
    let obj_dir = store_of(&tracked_doc(root))
        .join("objects")
        .join(&hash[..2]);
    std::fs::create_dir_all(&obj_dir).unwrap();
    std::fs::write(
        obj_dir.join(format!("{hash}.full")),
        zstd::encode_all(&content[..], 3).unwrap(),
    )
    .unwrap();
    let mut chains = std::collections::BTreeMap::new();
    chains.insert(
        "file:notes.txt".to_string(),
        vec![V1 {
            hash: hash.clone(),
            object: format!("{hash}.full"),
            base: None,
            chain_len: 0,
        }],
    );
    let plain = bincode::serialize(&chains).unwrap();
    let shard = store_of(&tracked_doc(root)).join("chains").join(format!(
        "{}.bin",
        &blake3::hash(b"notes.txt").to_hex()[..16]
    ));
    std::fs::write(&shard, zstd::encode_all(&plain[..], 1).unwrap()).unwrap();

    // The legacy shard reads transparently.
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r.reconstruct("file:notes.txt", &hash).unwrap(), content);

    // Dirtying the shard rewrites it in the new format; both versions still reconstruct.
    let h2 = r
        .store_stream("file:notes.txt", b"hello legacy v2")
        .unwrap();
    r.save().unwrap();
    let bytes = std::fs::read(&shard).unwrap();
    assert!(bytes.starts_with(b"KVCC2"), "dirtied shard upgraded");
    let r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert_eq!(r2.reconstruct("file:notes.txt", &hash).unwrap(), content);
    assert_eq!(
        r2.reconstruct("file:notes.txt", &h2).unwrap(),
        b"hello legacy v2"
    );

    // A rotten shard still reads as empty (never a panic).
    let bad = store_of(&tracked_doc(root))
        .join("chains")
        .join("deadbeefdeadbeef.bin");
    std::fs::write(&bad, b"not a shard").unwrap();
    let r3 = repo::Repo::open(&tracked_doc(root)).unwrap();
    assert!(r3.chains.chain("file:whatever").is_none());
}

// --- composite tiling: mergedimage.png stored as deduped pixel blocks ---------------------

/// Decode a PNG to interleaved RGBA (tests encode RGBA, so no expansion needed).
fn decode_rgba_png(png_bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    (buf, info.width, info.height)
}

/// Deterministic RGBA canvas.
fn test_canvas(w: u32, h: u32, seed: u8) -> Vec<u8> {
    (0..w as usize * h as usize * 4)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
        .collect()
}

fn kra_with_composite(composite_png: &[u8], tile: &[u8]) -> Vec<u8> {
    pack_kra(&[
        ("mimetype", b"application/x-krita".to_vec()),
        ("maindoc.xml", maindoc(255)),
        ("img/layers/layer1", tiled(&[(0, 0, tile)])),
        ("mergedimage.png", composite_png.to_vec()),
    ])
}

#[test]
fn composite_tiles_dedup_and_pixel_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // 1280x1280 = 5x5 composite blocks.
    let (w, h) = (1280u32, 1280u32);
    let px1 = test_canvas(w, h, 1);
    let png1 = raster::rgba_to_png(&px1, w, h).unwrap();
    std::fs::write(root.join("art.kra"), kra_with_composite(&png1, b"tile-v1")).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    // Edit a 50x50 region fully inside one block: only that block (+ the manifest) is new.
    let mut px2 = px1.clone();
    for y in 300..350usize {
        for x in 300..350usize {
            let i = (y * w as usize + x) * 4;
            px2[i] = 255 - px2[i];
        }
    }
    let png2 = raster::rgba_to_png(&px2, w, h).unwrap();
    std::fs::write(root.join("art.kra"), kra_with_composite(&png2, b"tile-v1")).unwrap();
    let before = count_objects(&store_of(&tracked_doc(root)));
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();
    let added = count_objects(&store_of(&tracked_doc(root))) - before;
    assert!(
        added <= 3,
        "a one-block composite edit must add ~2 objects (block + manifest), added {added}"
    );

    // Both versions reconstruct to pixel-exact composites (bytes are re-encoded, pixels not).
    for (cid, px) in [(&c1.id, &px1), (&c2.id, &px2)] {
        let bytes = commit::file_at_commit(&r, "art.kra", cid).unwrap();
        let entry = kra::read_entry(&bytes, "mergedimage.png").unwrap();
        let (got, gw, gh) = decode_rgba_png(&entry);
        assert_eq!((gw, gh), (w, h));
        assert_eq!(&got, px, "composite pixels must round-trip exactly");
        // The rest of the archive is intact too.
        assert_eq!(
            kra::read_entry(&bytes, "mimetype").unwrap(),
            b"application/x-krita"
        );
        let block = tiles::parse(&kra::read_entry(&bytes, "img/layers/layer1").unwrap()).unwrap();
        assert_eq!(block.tiles[0].data, b"tile-v1");
    }
}

#[test]
fn composite_fallback_stays_raw_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // A 16-bit grayscale PNG is ineligible for re-encoding — must stay Raw, byte-for-byte.
    let mut gray16 = Vec::new();
    {
        let mut enc = png::Encoder::new(std::io::Cursor::new(&mut gray16), 64, 64);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Sixteen);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&vec![0x42u8; 64 * 64 * 2]).unwrap();
    }
    std::fs::write(root.join("art.kra"), kra_with_composite(&gray16, b"tile-x")).unwrap();
    let c = commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    let bytes = commit::file_at_commit(&r, "art.kra", &c.id).unwrap();
    assert_eq!(
        kra::read_entry(&bytes, "mergedimage.png").unwrap(),
        gray16,
        "ineligible composite must reconstruct byte-identical"
    );
}

#[test]
fn composite_materialize_across_branch_switch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let (w, h) = (600u32, 500u32); // non-multiple of 256: exercises partial edge blocks
    let px_main = test_canvas(w, h, 7);
    let png_main = raster::rgba_to_png(&px_main, w, h).unwrap();
    std::fs::write(
        root.join("art.kra"),
        kra_with_composite(&png_main, b"t-main"),
    )
    .unwrap();
    commit::commit_snapshot(&mut r, "main v1", "t").unwrap();

    branch::create_branch(&mut r, "idea", None).unwrap();
    let mut px_idea = px_main.clone();
    for i in (0..px_idea.len()).step_by(97) {
        px_idea[i] = px_idea[i].wrapping_add(13);
    }
    let png_idea = raster::rgba_to_png(&px_idea, w, h).unwrap();
    std::fs::write(
        root.join("art.kra"),
        kra_with_composite(&png_idea, b"t-idea"),
    )
    .unwrap();
    commit::commit_snapshot(&mut r, "idea v1", "t").unwrap();

    // Bounce between branches; the restored composite is pixel-exact each time and the tree
    // stays clean (index hash matches what was written).
    for (branch_name, px) in [("main", &px_main), ("idea", &px_idea), ("main", &px_main)] {
        branch::switch_branch(&mut r, branch_name).unwrap();
        let on_disk = std::fs::read(root.join("art.kra")).unwrap();
        let entry = kra::read_entry(&on_disk, "mergedimage.png").unwrap();
        let (got, gw, gh) = decode_rgba_png(&entry);
        assert_eq!((gw, gh), (w, h));
        assert_eq!(&got, px, "restored composite pixels exact on {branch_name}");
        assert!(scan::scan(&r).unwrap().is_empty(), "clean after switch");
    }
}

// --- opt-in tile pixel deltas: mixed-flag history, patch-floor bypass ---------------------

#[test]
fn tile_pixel_deltas_flag_mixed_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let planar = |seed: u8| -> Vec<u8> {
        (0..64 * 64 * 4u32)
            .map(|i| ((i / 97) as u8).wrapping_add(seed))
            .collect()
    };
    let doc = |p: &[u8]| {
        let stored = raster::tile_from_planar(p); // Krita-style [flag][LZF] payload
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&[(0, 0, &stored)])),
        ])
    };

    // v1: flag off — opaque tile stream (the default path).
    let p1 = planar(1);
    std::fs::write(root.join("art.kra"), doc(&p1)).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    // v2 + v3: flag on — planar streams; v3 must patch against v2 (the 64 KB floor is
    // bypassed for pixel-delta tiles, or the whole feature stores fulls).
    r.config.tile_pixel_deltas = true;
    let p2 = planar(2);
    std::fs::write(root.join("art.kra"), doc(&p2)).unwrap();
    let c2 = commit::commit_snapshot(&mut r, "v2", "t").unwrap();
    let mut p3 = p2.clone();
    for b in p3.iter_mut().take(500) {
        *b = b.wrapping_add(9);
    }
    std::fs::write(root.join("art.kra"), doc(&p3)).unwrap();
    let c3 = commit::commit_snapshot(&mut r, "v3", "t").unwrap();
    let chain = r
        .chains
        .chain("kra:art.kra:tile:img/layers/layer1:0,0")
        .unwrap();
    assert!(
        chain.last().unwrap().base.is_some(),
        "a 16 KB planar tile edit must store as a patch, not a full"
    );

    // v4: flag off again — new commits go back to opaque; older raw refs stay readable.
    r.config.tile_pixel_deltas = false;
    let p4 = planar(4);
    std::fs::write(root.join("art.kra"), doc(&p4)).unwrap();
    let c4 = commit::commit_snapshot(&mut r, "v4", "t").unwrap();

    // Every version reconstructs to a .kra whose tile payload is valid (LZF or raw) and
    // decodes to the exact source pixels — across both storage modes in one history.
    for (cid, p) in [(&c1.id, &p1), (&c2.id, &p2), (&c3.id, &p3), (&c4.id, &p4)] {
        let bytes = commit::file_at_commit(&r, "art.kra", cid).unwrap();
        let block = tiles::parse(&kra::read_entry(&bytes, "img/layers/layer1").unwrap()).unwrap();
        let got = raster::tile_planar(&block.tiles[0].data, p.len()).unwrap();
        assert_eq!(&got, p, "planar pixels exact for {cid}");
    }
}

// --- settings: config persists across a fresh open (get_repo_config/set_repo_config path) --

#[test]
fn repo_config_round_trips_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);

    let mut r = repo::Repo::open_light(&tracked_doc(root)).unwrap();
    assert_eq!(
        r.config.cache_max_bytes,
        256 * 1024 * 1024,
        "default budget"
    );
    assert!(!r.config.tile_pixel_deltas, "default off");

    r.config.cache_max_bytes = 512 * 1024 * 1024;
    r.config.tile_pixel_deltas = true;
    r.save_config().unwrap();

    let reopened = repo::Repo::open_light(&tracked_doc(root)).unwrap();
    assert_eq!(reopened.config.cache_max_bytes, 512 * 1024 * 1024);
    assert!(reopened.config.tile_pixel_deltas);
}

// --- setting work aside ----------------------------------------------------------------

#[test]
fn stash_all_reverts_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(6)).unwrap();
    let s = stash::create(&mut r, "lighting pass", "t", None).unwrap();

    assert_eq!(s.label, "lighting pass");
    assert_eq!(s.branch, "main");
    assert_eq!(s.files.len(), 1);
    assert_eq!(stash::list(&r).len(), 1);
    // The tree is clean again: the edit is reverted.
    assert!(scan::scan(&r).unwrap().is_empty());
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
    // History is untouched - a stash is not a commit.
    assert_eq!(r.commits.len(), 1);
}
#[test]
fn stash_then_pop_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(6)).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();

    stash::pop(&mut r, &s.id).unwrap();

    // The set-aside work is back, and the shelf is empty again.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(6));
    assert!(stash::list(&r).is_empty());
    // The index must still hold the *committed* head, so the restored work reads as unsaved
    // rather than silently looking already-committed.
    assert_eq!(
        scan::scan(&r).unwrap(),
        vec![("art.kra".to_string(), "M".to_string())]
    );
}
#[test]
fn stash_selected_paths_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(6)).unwrap();
    assert!(matches!(
        stash::create(&mut r, "nothing", "t", Some(&["other.kra".to_string()])),
        Err(KvcError::Nothing)
    ));
    assert_eq!(
        doc_content(&tracked_doc(root)),
        maindoc(6),
        "edit must survive"
    );

    let only = vec!["art.kra".to_string()];
    let s = stash::create(&mut r, "just the document", "t", Some(&only)).unwrap();
    assert_eq!(s.files.len(), 1);
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(1));
    assert!(scan::scan(&r).unwrap().is_empty());
}
#[test]
fn pop_refuses_when_stashed_deletion_meets_an_edit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::remove_file(tracked_doc(root)).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();
    assert_eq!(s.files[0].status, "D");

    // The document is back and edited since — restoring the deletion would destroy that work.
    std::fs::write(tracked_doc(root), kra_bytes(14)).unwrap();

    match stash::pop(&mut r, &s.id) {
        Err(KvcError::StashConflict(files)) => assert_eq!(files, "art.kra"),
        other => panic!("expected StashConflict, got {:?}", other.map(|_| ())),
    }
    // Nothing lost on either side: the newer edit stands and the stash survives.
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(14));
    assert_eq!(stash::list(&r).len(), 1);
    // The conflict message must not collide with the branch-switch prompt's prefix.
    assert!(!KvcError::StashConflict("art.kra".into())
        .to_string()
        .starts_with("unsaved changes"));
}
#[test]
fn gc_keeps_stashed_content() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(6)).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();

    // A stash is reachable from no branch tip — only rooting it keeps its content alive.
    gc::collect_garbage(&mut r, false).unwrap();

    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    stash::pop(&mut r, &s.id).unwrap();
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(6));
}

#[test]
fn dropped_stash_content_is_collectable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(6)).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();
    let with_stash = count_objects(&store_of(&tracked_doc(root)));

    stash::drop_one(&mut r, &s.id).unwrap();
    assert!(stash::list(&r).is_empty());

    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    gc::collect_garbage(&mut r, false).unwrap();
    assert!(
        count_objects(&store_of(&tracked_doc(root))) < with_stash,
        "dropping the last stash should make its content collectable"
    );
}

#[test]
fn stash_requires_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(13)).unwrap();

    // No tip means no committed state to revert to.
    assert!(matches!(
        stash::create(&mut r, "", "t", None),
        Err(KvcError::NoCommit(_))
    ));
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(13));
}

#[test]
fn stash_with_nothing_dirty_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = seeded_repo(&dir);
    assert!(matches!(
        stash::create(&mut r, "", "t", None),
        Err(KvcError::Nothing)
    ));
}

#[test]
fn stash_dedups_kra_tiles_against_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let mk = |t1: &[u8], t2: &[u8]| {
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&[(0, 0, t1), (64, 0, t2)])),
        ])
    };
    std::fs::write(root.join("art.kra"), mk(b"tileAAAA", b"tileBBBB")).unwrap();
    commit::commit_snapshot(&mut r, "v1", "t").unwrap();
    let committed = count_objects(&store_of(&tracked_doc(root)));

    // Only the second tile changes — the first must dedup against the committed version, which
    // is the whole premise of storing stashes through the same relpath-keyed streams.
    std::fs::write(root.join("art.kra"), mk(b"tileAAAA", b"tileZZZZ")).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();
    let grew = count_objects(&store_of(&tracked_doc(root))) - committed;
    assert!(
        grew <= 3,
        "a one-tile edit should cost a handful of objects, not a full copy (grew by {grew})"
    );

    // ...and it still restores byte-exactly.
    stash::pop(&mut r, &s.id).unwrap();
    let on_disk = std::fs::read(root.join("art.kra")).unwrap();
    let l = tiles::parse(&kra::read_entry(&on_disk, "img/layers/layer1").unwrap()).unwrap();
    assert_eq!(l.tiles[1].data, b"tileZZZZ");
}

// --- bringing set-aside .kra work back: layer merge ------------------------------------

/// A `.kra` whose `<IMAGE>` carries `colorspace` and whose `<layers>` are the given
/// `(name, uuid, filename, tile_data)` paint layers, each with a matching `img/layers/<filename>`
/// tiled entry.
fn layered_kra(image: &str, colorspace: &str, layers: &[(&str, &str, &str, &[u8])]) -> Vec<u8> {
    let mut xml = format!(
        "<!DOCTYPE DOC>\n<DOC><IMAGE name=\"{image}\" width=\"128\" height=\"128\" \
         colorspacename=\"{colorspace}\"><layers>\n"
    );
    for (name, uuid, filename, _) in layers {
        xml.push_str(&format!(
            "<layer name=\"{name}\" uuid=\"{uuid}\" opacity=\"255\" compositeop=\"normal\" \
             nodetype=\"paintlayer\" filename=\"{filename}\"/>\n"
        ));
    }
    xml.push_str("</layers></IMAGE></DOC>");

    let mut keys: Vec<String> = vec!["mimetype".into(), "maindoc.xml".into()];
    let mut datas: Vec<Vec<u8>> = vec![b"application/x-krita".to_vec(), xml.into_bytes()];
    for (_, _, filename, data) in layers {
        // `data` is &&[u8] here (match ergonomics over &[tuple]); tiled() wants &[u8].
        keys.push(format!("{image}/layers/{filename}"));
        datas.push(tiled(&[(0, 0, *data)]));
    }
    let refs: Vec<(&str, Vec<u8>)> = keys
        .iter()
        .zip(datas)
        .map(|(k, d)| (k.as_str(), d))
        .collect();
    pack_kra(&refs)
}

/// Names, filenames and uuids of a merged .kra's layers, for assertions.
fn layer_meta(kra_bytes: &[u8]) -> kra::ImageMeta {
    kra::parse_image_meta(&kra::read_entry(kra_bytes, "maindoc.xml").unwrap()).unwrap()
}

#[test]
fn merge_layers_folds_stashed_layers_and_renames_clashes() {
    // Working file: Background + Lines. Set-aside version: a re-touched Background (name clash)
    // plus a unique Sketch layer.
    let base = layered_kra(
        "img",
        "RGBA",
        &[
            ("Background", "{bg}", "layer1", b"tileBGxx"),
            ("Lines", "{ln}", "layer2", b"tileLN.."),
        ],
    );
    let incoming = layered_kra(
        "img",
        "RGBA",
        &[
            ("Background", "{bg}", "layer1", b"tileBGed"),
            ("Sketch", "{sk}", "layer3", b"tileSK.."),
        ],
    );

    let merged = merge::merge_layers(&base, &incoming, None).unwrap();
    let meta = layer_meta(&merged);

    let names: std::collections::HashSet<&str> =
        meta.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains("Background"), "base layer kept");
    assert!(names.contains("Lines"), "base layer kept");
    assert!(names.contains("Sketch"), "set-aside layer folded in");
    assert!(names.contains("Background [2]"), "name clash suffixed");
    assert_eq!(meta.layers.len(), 4, "all four layers present");

    // Filenames + uuids are unique and every data file exists under its (remapped) name.
    let uniq = |vals: Vec<&str>| {
        let n = vals.len();
        let mut v = vals;
        v.sort();
        v.dedup();
        v.len() == n
    };
    assert!(
        uniq(meta.layers.iter().map(|l| l.filename.as_str()).collect()),
        "layer filenames unique"
    );
    assert!(
        uniq(meta.layers.iter().map(|l| l.uuid.as_str()).collect()),
        "layer uuids unique"
    );
    for l in &meta.layers {
        assert!(
            kra::read_entry(&merged, &format!("img/layers/{}", l.filename)).is_ok(),
            "missing data file for {}",
            l.name
        );
    }

    // The set-aside's unique tile data crossed over under its remapped filename.
    let sketch = meta.layers.iter().find(|l| l.name == "Sketch").unwrap();
    let block = tiles::parse(
        &kra::read_entry(&merged, &format!("img/layers/{}", sketch.filename)).unwrap(),
    )
    .unwrap();
    assert_eq!(block.tiles[0].data, b"tileSK..");
}

#[test]
fn merge_layers_refuses_on_colorspace_mismatch() {
    let base = layered_kra("img", "RGBA", &[("L", "{a}", "layer1", b"aaaa")]);
    let incoming = layered_kra("img", "CMYK", &[("L", "{b}", "layer1", b"bbbb")]);
    assert!(matches!(
        merge::merge_layers(&base, &incoming, None),
        Err(KvcError::MergeFailed(_))
    ));
}

/// A `.kra` with realistic Krita layer attributes — including the volatile `selected` on whichever
/// layer is active — so a merge's "unchanged" test can't lean on the raw `<layer>` XML matching.
fn krita_kra(layers: &[(&str, &str, &str, bool, &[u8])]) -> Vec<u8> {
    let mut xml = String::from(
        "<!DOCTYPE DOC>\n<DOC><IMAGE name=\"img\" width=\"128\" height=\"128\" \
         colorspacename=\"RGBA\"><layers>\n",
    );
    for (name, uuid, filename, selected, _) in layers {
        let sel = if *selected { " selected=\"true\"" } else { "" };
        xml.push_str(&format!(
            "<layer name=\"{name}\" uuid=\"{uuid}\" filename=\"{filename}\" opacity=\"255\" \
             compositeop=\"normal\" visible=\"1\" x=\"0\" y=\"0\" collapsed=\"0\" locked=\"0\" \
             colorlabel=\"0\" nodetype=\"paintlayer\"{sel}/>\n"
        ));
    }
    xml.push_str("</layers></IMAGE></DOC>");

    let mut keys: Vec<String> = vec!["mimetype".into(), "maindoc.xml".into()];
    let mut datas: Vec<Vec<u8>> = vec![b"application/x-krita".to_vec(), xml.into_bytes()];
    for (_, _, filename, _, data) in layers {
        keys.push(format!("img/layers/{filename}"));
        datas.push(tiled(&[(0, 0, *data)]));
    }
    let refs: Vec<(&str, Vec<u8>)> = keys
        .iter()
        .zip(datas)
        .map(|(k, d)| (k.as_str(), d))
        .collect();
    pack_kra(&refs)
}

#[test]
fn merge_layers_skips_unchanged_despite_krita_attr_churn() {
    // Ancestor (committed): Background is the active layer.
    let ancestor = krita_kra(&[
        ("Background", "{bg}", "layer1", true, b"tileBG.."),
        ("Lines", "{ln}", "layer2", false, b"tileLN.."),
    ]);
    // Set-aside: the artist modified Lines and added Sketch. Background is *untouched*, but Krita
    // renumbered its data file (layer1 -> layer5) and moved `selected` off it — exactly the churn
    // that made a raw-XML compare treat every unchanged layer as changed.
    let incoming = krita_kra(&[
        ("Background", "{bg}", "layer5", false, b"tileBG.."),
        ("Lines", "{ln}", "layer6", true, b"tileLNmd"),
        ("Sketch", "{sk}", "layer7", false, b"tileSK.."),
    ]);
    // Working: edited since it was set aside, so the pop is a conflict.
    let base = krita_kra(&[
        ("Background", "{bg}", "layer1", true, b"tileBGwk"),
        ("Lines", "{ln}", "layer2", false, b"tileLNwk"),
    ]);

    let merged = merge::merge_layers(&base, &incoming, Some(&ancestor)).unwrap();
    let names: std::collections::HashSet<String> = layer_meta(&merged)
        .layers
        .into_iter()
        .map(|l| l.name)
        .collect();

    // Only the set-aside's real changes fold in: the modified Lines (name clash -> [2]) and the
    // added Sketch. The untouched Background is NOT duplicated despite its churned attributes.
    assert!(names.contains("Lines [2]"), "modified layer folded in");
    assert!(names.contains("Sketch"), "added layer folded in");
    assert!(
        !names.contains("Background [2]"),
        "unchanged layer must not be duplicated"
    );
    assert_eq!(
        names.len(),
        4,
        "base Background+Lines plus Lines [2]+Sketch"
    );
}

#[test]
fn merge_layers_skips_unchanged_despite_tile_reordering() {
    // A .kra whose paint layers carry the given (already tile-encoded) data files.
    fn kra(layers: &[(&str, &str, &str, Vec<u8>)]) -> Vec<u8> {
        let mut xml = String::from(
            "<!DOCTYPE DOC>\n<DOC><IMAGE name=\"img\" width=\"128\" height=\"128\" \
             colorspacename=\"RGBA\"><layers>\n",
        );
        for (name, uuid, filename, _) in layers {
            xml.push_str(&format!(
                "<layer name=\"{name}\" uuid=\"{uuid}\" filename=\"{filename}\" opacity=\"255\" \
                 compositeop=\"normal\" visible=\"1\" nodetype=\"paintlayer\"/>\n"
            ));
        }
        xml.push_str("</layers></IMAGE></DOC>");
        let mut keys: Vec<String> = vec!["mimetype".into(), "maindoc.xml".into()];
        let mut datas: Vec<Vec<u8>> = vec![b"application/x-krita".to_vec(), xml.into_bytes()];
        for (_, _, filename, data) in layers {
            keys.push(format!("img/layers/{filename}"));
            datas.push(data.clone());
        }
        let refs: Vec<(&str, Vec<u8>)> = keys
            .iter()
            .zip(datas)
            .map(|(k, d)| (k.as_str(), d))
            .collect();
        pack_kra(&refs)
    }

    let ancestor = kra(&[(
        "Base",
        "{b}",
        "layer1",
        tiled(&[(0, 0, b"tileAAAA"), (64, 0, b"tileBBBB")]),
    )]);
    // Set-aside: Base is untouched but Krita wrote its two tiles in the opposite order (tile order
    // isn't stable across saves, so it reconstructs to different bytes), and a Sketch was added.
    let incoming = kra(&[
        (
            "Base",
            "{b}",
            "layer1",
            tiled(&[(64, 0, b"tileBBBB"), (0, 0, b"tileAAAA")]),
        ),
        ("Sketch", "{s}", "layer2", tiled(&[(0, 0, b"tileSKSK")])),
    ]);
    let base = kra(&[(
        "Base",
        "{b}",
        "layer1",
        tiled(&[(0, 0, b"tileAAAA"), (64, 0, b"tileBBBB")]),
    )]);

    let merged = merge::merge_layers(&base, &incoming, Some(&ancestor)).unwrap();
    let names: Vec<String> = layer_meta(&merged)
        .layers
        .into_iter()
        .map(|l| l.name)
        .collect();

    // Base must not be duplicated despite the reordered tiles; only the added Sketch folds in.
    assert_eq!(
        names.iter().filter(|n| n.starts_with("Base")).count(),
        1,
        "unchanged layer duplicated because its tiles were reordered"
    );
    assert!(names.iter().any(|n| n == "Sketch"), "added layer folded in");
    assert_eq!(names.len(), 2);
}

#[test]
fn pop_merges_conflicting_kra_instead_of_refusing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let v1 = layered_kra(
        "img",
        "RGBA",
        &[
            ("Background", "{bg}", "layer1", b"tileBG.."),
            ("Lines", "{ln}", "layer2", b"tileLN.."),
        ],
    );
    std::fs::write(root.join("art.kra"), &v1).unwrap();
    commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    // Set aside a version that *modifies* Background, leaves Lines untouched, and *adds* Sketch;
    // the tree reverts to v1.
    let aside = layered_kra(
        "img",
        "RGBA",
        &[
            ("Background", "{bg}", "layer1", b"tileBGmd"),
            ("Lines", "{ln}", "layer2", b"tileLN.."),
            ("Sketch", "{sk}", "layer3", b"tileSK.."),
        ],
    );
    std::fs::write(root.join("art.kra"), &aside).unwrap();
    let s = stash::create(&mut r, "sketch pass", "t", None).unwrap();
    // The revert reconstructs the .kra (re-deflated, not byte-identical to the fixture), so assert
    // the tree is *clean* rather than byte-equal to v1.
    assert!(
        scan::scan(&r).unwrap().is_empty(),
        "stash reverted the tree"
    );

    // Edit the file again so art.kra conflicts with the set-aside version.
    let edited = layered_kra(
        "img",
        "RGBA",
        &[
            ("Background", "{bg}", "layer1", b"tileBGwk"),
            ("Lines", "{ln}", "layer2", b"tileLN.."),
        ],
    );
    std::fs::write(root.join("art.kra"), &edited).unwrap();

    // Pops without refusing, and the shelf empties.
    stash::pop(&mut r, &s.id).unwrap();
    assert!(stash::list(&r).is_empty());

    // Only the set-aside's *changes* fold in: the modified Background (clash suffixed) and the
    // added Sketch. The untouched Lines is NOT duplicated — no "Lines [2]".
    let meta = layer_meta(&std::fs::read(root.join("art.kra")).unwrap());
    let names: std::collections::HashSet<&str> =
        meta.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains("Background"), "working Background kept");
    assert!(names.contains("Lines"), "working Lines kept");
    assert!(names.contains("Sketch"), "added layer folded in");
    assert!(names.contains("Background [2]"), "modified layer folded in");
    assert!(
        !names.contains("Lines [2]"),
        "unchanged layer not duplicated"
    );
    assert_eq!(meta.layers.len(), 4);
}

#[test]
fn pop_merge_failure_leaves_working_tree_and_stash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    let v1 = layered_kra("img", "RGBA", &[("L", "{a}", "layer1", b"aaaa")]);
    std::fs::write(root.join("art.kra"), &v1).unwrap();
    commit::commit_snapshot(&mut r, "v1", "t").unwrap();

    let aside = layered_kra("img", "RGBA", &[("L", "{a}", "layer1", b"aside")]);
    std::fs::write(root.join("art.kra"), &aside).unwrap();
    let s = stash::create(&mut r, "", "t", None).unwrap();

    // Working copy now uses a different color space, so the merge can't be done cleanly.
    let edited = layered_kra("img", "CMYK", &[("L", "{a}", "layer1", b"cmyk")]);
    std::fs::write(root.join("art.kra"), &edited).unwrap();

    assert!(matches!(
        stash::pop(&mut r, &s.id),
        Err(KvcError::MergeFailed(_))
    ));
    // Nothing written, nothing lost: the working file stands and the stash survives.
    assert_eq!(std::fs::read(root.join("art.kra")).unwrap(), edited);
    assert_eq!(stash::list(&r).len(), 1);
}

#[test]
fn stash_dtos_are_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    std::fs::write(tracked_doc(root), kra_bytes(11)).unwrap();
    stash::create(&mut r, "older", "t", None).unwrap();
    std::fs::write(tracked_doc(root), kra_bytes(36)).unwrap();
    stash::create(&mut r, "newer", "t", None).unwrap();

    // The engine stores oldest-first; the UI's "bring back the latest" takes row 0, so the DTO
    // list must be reversed or that button silently restores the wrong work.
    let dtos = commands::stash_dtos(&r);
    let labels: Vec<&str> = dtos.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels, vec!["newer", "older"]);
    // Statuses survive the CommittedFile -> FileChange mapping for the status chip.
    assert_eq!(dtos[0].changes[0].path, "art.kra");
    assert_eq!(dtos[0].changes[0].status, "M");
    assert_eq!(dtos[0].branch, "main");
}

// --- data integrity ---------------------------------------------------------------------

/// The working tree is written temp-then-rename, so a failed write can never leave the artist's
/// file half-replaced. In-process we can't kill a write mid-flight, so pin the observable
/// contract instead: success replaces cleanly, failure leaves the target byte-intact, and
/// neither leaves a `.kvctmp` behind for the scanner to trip over.
#[test]
fn working_tree_writes_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let target = root.join("art.kra");
    std::fs::write(&target, b"original").unwrap();
    repo::write_file_atomic(&target, b"replaced").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"replaced");
    assert!(!root.join("art.kra.kvctmp").exists(), "temp must be gone");

    // A directory in the way makes the rename fail — the interesting half.
    let blocked = root.join("blocked.kra");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("inner"), b"keep me").unwrap();
    assert!(repo::write_file_atomic(&blocked, b"nope").is_err());
    assert!(blocked.is_dir(), "target must survive a failed write");
    assert_eq!(std::fs::read(blocked.join("inner")).unwrap(), b"keep me");
    assert!(
        !root.join("blocked.kra.kvctmp").exists(),
        "temp must be cleaned up"
    );
}

/// A damaged object in the store must never be written into the working tree. The restore paths
/// re-hash what they rebuild (`Repo::verify_reads`); the diff path deliberately doesn't, and this
/// pins both halves — the second assert is what keeps the check off the hot loop.
#[test]
fn corrupt_object_is_refused_on_restore_but_not_on_the_diff_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(21)).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    // Small first version = a full zstd snapshot named by blake3 of the file bytes. Replace its
    // payload with a *valid* frame holding different content: it decodes fine, so only a hash
    // check can catch it.
    let obj = corruptible_object(&r, &store_of(&tracked_doc(root)));
    std::fs::write(&obj, zstd::encode_all(&b"tampered"[..], 1).unwrap()).unwrap();

    // Restore path: refuses, and leaves the working file exactly as it was.
    std::fs::write(tracked_doc(root), kra_bytes(12)).unwrap();
    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let err = commit::discard_working_changes(&mut r2, &c1.id, None).unwrap_err();
    assert!(
        matches!(err, KvcError::Corrupt(_)),
        "expected Corrupt, got {err:?}"
    );
    assert_eq!(doc_content(&tracked_doc(root)), maindoc(12));

    // Diff path: same object, no verification, no error — a wrong pixel in a preview is not data
    // loss, and this is the loop the app is tuned around.
    let r3 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let rebuilt = commit::file_at_commit(&r3, "art.kra", &c1.id).unwrap();
    assert!(
        !rebuilt.is_empty(),
        "the diff path must rebuild, not refuse"
    );
}

/// `check` answers "is my history intact?" — it must be silent on a healthy repo and name the
/// three failures that make history unreachable.
#[test]
fn check_reports_missing_objects_and_dangling_tips() {
    use krita_vc_lib::check;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);
    std::fs::write(
        root.join("art.kra"),
        pack_kra(&[
            ("mimetype", b"application/x-krita".to_vec()),
            ("maindoc.xml", maindoc(255)),
            ("img/layers/layer1", tiled(&[(0, 0, b"tileAAAA")])),
        ]),
    )
    .unwrap();
    commit::commit_snapshot(&mut r, "c2", "t").unwrap();

    let clean = check::check_repository(&mut r, false).unwrap();
    assert!(clean.ok(), "healthy repo reported {:?}", clean.problems);
    assert!(clean.commits_checked >= 2 && clean.objects_checked > 0);

    // Delete one live object. A *content* stream, deliberately: losing a `.kra`'s manifest is
    // reported as a broken chain instead, which is a different problem from a missing object.
    let obj = corruptible_object(&r, &store_of(&tracked_doc(root)));
    std::fs::remove_file(&obj).unwrap();
    let report =
        check::check_repository(&mut repo::Repo::open(&tracked_doc(root)).unwrap(), false).unwrap();
    assert!(!report.ok());
    assert!(
        report.problems.iter().any(|p| p.kind == "missingObject"),
        "{:?}",
        report.problems
    );

    // A branch tip naming a version that isn't in the log — the state that makes every commit
    // on that branch unreachable.
    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    r2.branches
        .branches
        .insert("main".to_string(), "nosuchcommit".to_string());
    r2.save_branches().unwrap();
    let report =
        check::check_repository(&mut repo::Repo::open(&tracked_doc(root)).unwrap(), false).unwrap();
    assert!(report.problems.iter().any(|p| p.kind == "danglingTip"));

    // Damage in the middle of the commit log is reported, not silently truncated away.
    let log = store_of(&tracked_doc(root)).join("commits.log");
    let text = std::fs::read_to_string(&log).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines[0] = "{not json";
    std::fs::write(&log, lines.join("\n")).unwrap();
    let report =
        check::check_repository(&mut repo::Repo::open(&tracked_doc(root)).unwrap(), false).unwrap();
    assert!(report.problems.iter().any(|p| p.kind == "badLogLine"));
}

/// Gap #8: the default (non-scrub) check is presence-only — it must stay silent about a valid
/// zstd frame holding the wrong content, since only a hash check can catch that. `scrub: true`
/// is what closes the gap, and it's opt-in.
#[test]
fn check_scrub_off_by_default_skips_content_hash() {
    use krita_vc_lib::check;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut r = seeded_repo(&dir);

    // Healthy repo: scrubbing reports what it verified.
    let scrubbed = check::check_repository(&mut r, true).unwrap();
    assert!(scrubbed.ok(), "{:?}", scrubbed.problems);
    assert!(scrubbed.scrub_performed);
    assert!(scrubbed.versions_scrubbed > 0);

    // Tamper a loose object with a *valid* zstd frame holding different content — it decodes
    // fine, so `object_exists` alone can't catch it.
    let obj = corruptible_object(&r, &store_of(&tracked_doc(root)));
    std::fs::write(&obj, zstd::encode_all(&b"tampered"[..], 1).unwrap()).unwrap();

    let plain = check::check_repository(&mut r, false).unwrap();
    assert!(
        plain.ok(),
        "default check must not notice a content-level tamper: {:?}",
        plain.problems
    );
    assert!(!plain.scrub_performed);
    assert_eq!(plain.versions_scrubbed, 0);
}

/// A tampered loose object (valid zstd, wrong content) is exactly what `scrub` exists to catch.
#[test]
fn check_scrub_detects_corrupted_loose_object() {
    use krita_vc_lib::check;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    std::fs::write(tracked_doc(root), kra_bytes(21)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    let obj = any_loose_object(&store_of(&tracked_doc(root)));
    assert!(obj.is_file());
    std::fs::write(&obj, zstd::encode_all(&b"tampered"[..], 1).unwrap()).unwrap();

    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let scrubbed = check::check_repository(&mut r2, true).unwrap();
    assert!(!scrubbed.ok(), "scrub must catch the tampered object");
    assert!(scrubbed.problems.iter().any(|p| p.kind == "corruptContent"));
}

/// Same as the loose-object case, but for a packed entry — large batches store as one pack file
/// instead of loose objects, and scrub must reach into it too.
#[test]
fn check_scrub_detects_corrupted_pack_entry() {
    use krita_vc_lib::check;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // 40 distinct tiles (> PACK_MIN_OBJECTS) pack into one file, per
    // `large_commit_packs_objects_and_reconstructs`.
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
    commit::commit_snapshot(&mut r, "packed", "t").unwrap();

    let pack_path = std::fs::read_dir(store_of(&tracked_doc(root)).join("objects/pack"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "pack"))
        .unwrap();

    // Corrupt the payload region in place — same overall length (header + index + body-length
    // field untouched), so the pack still parses and every offset still lands where it should;
    // only content changes.
    let mut bytes = std::fs::read(&pack_path).unwrap();
    let idx_len = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
    let payload_start = 5 + 4 + idx_len + 8; // KVCP2: magic + idx_len + index + u64 body_len
    for b in &mut bytes[payload_start..payload_start + 32] {
        *b ^= 0xFF;
    }
    std::fs::write(&pack_path, &bytes).unwrap();

    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let plain = check::check_repository(&mut r2, false).unwrap();
    assert!(
        plain.ok(),
        "presence-only check must not notice a content-level tamper: {:?}",
        plain.problems
    );

    let scrubbed = check::check_repository(&mut r2, true).unwrap();
    assert!(!scrubbed.ok(), "scrub must catch the corrupted pack entry");
    assert!(scrubbed.problems.iter().any(|p| p.kind == "corruptContent"));
}

/// One bad version must not stop the scrub from finding the next one.
#[test]
fn check_scrub_reports_multiple_corruptions_without_aborting() {
    use krita_vc_lib::check;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_doc(root);
    let mut r = repo::Repo::open(&tracked_doc(root)).unwrap();

    // A `.kra` stores one stream per zip entry, so a single commit already leaves several loose
    // objects — enough to corrupt two of them without needing a second tracked file.
    std::fs::write(tracked_doc(root), kra_bytes(18)).unwrap();
    commit::commit_snapshot(&mut r, "c1", "t").unwrap();

    let store = store_of(&tracked_doc(root));
    let manifest_keys: Vec<String> = r
        .chains
        .export_all()
        .0
        .keys()
        .filter(|k| k.ends_with(":manifest"))
        .cloned()
        .collect();
    let manifests: Vec<std::path::PathBuf> = manifest_keys
        .iter()
        .filter_map(|k| r.chains.chain(k))
        .flatten()
        .map(|v| {
            store
                .join("objects")
                .join(&v.hash[..2])
                .join(v.object_name())
        })
        .collect();
    // Corrupt two *content* objects: a corrupted manifest breaks the chain read and the scrub
    // never reaches the content it was meant to verify.
    let corrupted: Vec<_> = all_loose_objects(&store)
        .into_iter()
        .filter(|p| !manifests.contains(p))
        .take(2)
        .collect();
    assert_eq!(corrupted.len(), 2, "need two content objects to corrupt");
    for obj in &corrupted {
        std::fs::write(obj, zstd::encode_all(&b"tampered"[..], 1).unwrap()).unwrap();
    }

    let mut r2 = repo::Repo::open(&tracked_doc(root)).unwrap();
    let scrubbed = check::check_repository(&mut r2, true).unwrap();
    let corrupt_count = scrubbed
        .problems
        .iter()
        .filter(|p| p.kind == "corruptContent")
        .count();
    assert_eq!(
        corrupt_count, 2,
        "both corruptions must surface, not just the first: {:?}",
        scrubbed.problems
    );
}
