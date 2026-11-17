//! A layer is "modified" exactly when a pixel inside the canvas changed, and every modified layer
//! gets a change region that can be seen.
//!
//! Regression tests for edits found during the BSc evaluation (Chapter 5 testing) that showed a
//! layer as modified with nothing highlighted: a tap past the canvas edge, a transparent tile
//! appearing, the same pixels stored differently, faint edits under the old highlight threshold,
//! and a single-pixel edit on an A4 canvas lost in the 2048 px downscale.

use krita_vc_lib::{commands, commit, kra, raster, repo};
use std::io::Write;

const N: usize = 64 * 64;

/// A raw planar BGRA tile (flag 0), every pixel one colour.
fn solid(b: u8, g: u8, r: u8, a: u8) -> Vec<u8> {
    let mut d = vec![0u8];
    for v in [b, g, r, a] {
        d.extend(std::iter::repeat_n(v, N));
    }
    d
}

/// Change pixel `i` of a raw tile by `delta` on its three colour channels.
fn bump(t: &[u8], i: usize, delta: i32) -> Vec<u8> {
    let mut t = t.to_vec();
    for plane in 0..3 {
        let o = 1 + plane * N + i;
        t[o] = (t[o] as i32 + delta).clamp(0, 255) as u8;
    }
    t
}

/// The same pixels as a raw tile, LZF-compressed instead (flag 1).
fn as_lzf(raw: &[u8]) -> Vec<u8> {
    let mut d = vec![1u8];
    d.extend(raster::lzf_compress(&raw[1..]));
    d
}

type Tiles = Vec<(i64, i64, Vec<u8>)>;

fn kra_bytes(tiles: &Tiles, w: i64, h: i64) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut block = format!(
        "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA {}\n",
        tiles.len()
    )
    .into_bytes();
    for (x, y, d) in tiles {
        block.extend(format!("{x},{y},LZF,{}\n", d.len()).as_bytes());
        block.extend(d);
    }
    let md = format!(
        r#"<!DOCTYPE DOC><DOC><IMAGE name="img" width="{w}" height="{h}"><layers><layer name="Paint" uuid="l0" opacity="255" compositeop="normal" nodetype="paintlayer" filename="layer0" visible="1"/></layers></IMAGE></DOC>"#
    );
    let mut out = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/x-krita").unwrap();
        z.start_file("maindoc.xml", o).unwrap();
        z.write_all(md.as_bytes()).unwrap();
        z.start_file("img/layers/layer0", o).unwrap();
        z.write_all(&block).unwrap();
        z.finish().unwrap();
    }
    out
}

fn grid(cols: i64, rows: i64) -> Tiles {
    let mut t = Vec::new();
    for y in 0..rows {
        for x in 0..cols {
            t.push((x * 64, y * 64, solid(128, 128, 128, 255)));
        }
    }
    t
}

/// What the diff says about the single layer, and the document-level region.
#[derive(Debug)]
struct Seen {
    change: String,
    layer_regions: Vec<(f64, f64, f64, f64)>,
    doc_regions: usize,
}

fn regions_of(v: &serde_json::Value) -> Vec<(f64, f64, f64, f64)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|r| {
                    let f = |k: &str| r[k].as_f64().unwrap();
                    (f("x"), f("y"), f("w"), f("h"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn seen(dto: &commands::ArtDiffDto) -> Seen {
    let v = serde_json::to_value(dto).unwrap();
    let l = &v["layers"][0];
    Seen {
        change: l["change"].as_str().unwrap().to_string(),
        layer_regions: regions_of(&l["regions"]),
        doc_regions: regions_of(&v["regions"]).len(),
    }
}

/// Commit `base`, then diff `edited` both ways the app does: as a committed version against its
/// parent (history), and as unsaved work against the tip (Changes panel), in normal and
/// low-memory mode. All three must agree.
fn diff(base: &Tiles, edited: &Tiles, w: i64, h: i64) -> Seen {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("p.kra");
    std::fs::write(&doc, kra_bytes(base, w, h)).unwrap();
    repo::Repo::init(&doc).unwrap();
    let mut r = repo::Repo::open(&doc).unwrap();
    let c0 = commit::commit_snapshot(&mut r, "base", "test").unwrap();
    let f0 = c0.files.iter().find(|f| f.is_kra).unwrap().clone();
    let m0 = kra::load_manifest(&r, &f0.path, f0.content.as_deref().unwrap()).unwrap();

    let edited_bytes = kra_bytes(edited, w, h);
    let mut working = Vec::new();
    for low_memory in [false, true] {
        let wk = kra::parse_working(&edited_bytes, low_memory).unwrap();
        let dto = commands::art_diff_dto(
            &r,
            &f0.path,
            "M",
            &kra::KraSource::Working(&wk),
            Some(&*m0),
            true,
            None,
        )
        .unwrap();
        working.push(seen(&dto));
    }

    std::fs::write(&doc, &edited_bytes).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "edit", "test").unwrap();
    let f1 = c1.files.iter().find(|f| f.is_kra).unwrap().clone();
    let committed = seen(&commands::committed_art_dto(&r, &f1, Some(&f0), true, None).unwrap());

    for w in &working {
        assert_eq!(
            w.change, committed.change,
            "working vs committed diff disagree"
        );
        assert_eq!(w.layer_regions.len(), committed.layer_regions.len());
        assert_eq!(w.doc_regions, committed.doc_regions);
    }
    committed
}

fn assert_unchanged(s: &Seen) {
    assert_eq!(s.change, "unchanged", "{s:?}");
    assert!(s.layer_regions.is_empty(), "{s:?}");
    assert_eq!(
        s.doc_regions, 0,
        "an invisible edit must not draw a region: {s:?}"
    );
}

fn assert_highlighted(s: &Seen) {
    assert_eq!(s.change, "modified", "{s:?}");
    assert_eq!(
        s.layer_regions.len(),
        1,
        "a modified layer needs a region: {s:?}"
    );
    assert_eq!(s.doc_regions, 1, "{s:?}");
    let (_, _, rw, rh) = s.layer_regions[0];
    assert!(rw > 0.0 && rh > 0.0, "region must not be empty: {s:?}");
}

#[test]
fn visible_edit_is_modified_and_highlighted() {
    let mut e = grid(2, 2);
    e[0].2 = bump(&e[0].2, 0, 100);
    assert_highlighted(&diff(&grid(2, 2), &e, 128, 128));
}

#[test]
fn tile_outside_the_canvas_is_not_a_change() {
    for at in [(-64, 0), (128, 0), (0, -64), (0, 128)] {
        let mut e = grid(2, 2);
        e.push((at.0, at.1, solid(9, 9, 9, 255)));
        assert_unchanged(&diff(&grid(2, 2), &e, 128, 128));
    }
}

#[test]
fn tile_straddling_the_edge_counts_only_its_inside() {
    // Canvas 100 x 100: tile (64, 0) is half inside. Changing a pixel in its outside half is
    // invisible; one in its inside half is a real edit.
    let base = grid(2, 2);
    let mut outside = base.clone();
    outside[1].2 = bump(&outside[1].2, 50, 100); // local x 50 -> canvas x 114
    assert_unchanged(&diff(&base, &outside, 100, 100));
    let mut inside = base.clone();
    inside[1].2 = bump(&inside[1].2, 10, 100); // local x 10 -> canvas x 74
    assert_highlighted(&diff(&base, &inside, 100, 100));
}

#[test]
fn transparent_tile_appearing_is_not_a_change() {
    let mut base = grid(2, 2);
    base.truncate(3); // (64, 64) absent: the transparent default
    let mut e = base.clone();
    e.push((64, 64, solid(0, 0, 0, 0)));
    assert_unchanged(&diff(&base, &e, 128, 128));
    // ...and the reverse, the tile going away again.
    assert_unchanged(&diff(&e, &base, 128, 128));
}

#[test]
fn same_pixels_stored_differently_is_not_a_change() {
    let mut e = grid(2, 2);
    e[0].2 = as_lzf(&e[0].2);
    assert_unchanged(&diff(&grid(2, 2), &e, 128, 128));
}

#[test]
fn faint_edits_are_highlighted() {
    let mut one = grid(2, 2);
    one[0].2 = bump(&one[0].2, 0, 10);
    assert_highlighted(&diff(&grid(2, 2), &one, 128, 128));
    let mut wash = grid(2, 2);
    wash[0].2 = solid(138, 138, 138, 255);
    assert_highlighted(&diff(&grid(2, 2), &wash, 128, 128));
    let mut step = grid(2, 2);
    step[0].2 = bump(&step[0].2, 0, 1);
    assert_highlighted(&diff(&grid(2, 2), &step, 128, 128));
}

#[test]
fn single_pixel_on_a4_is_highlighted_at_a_visible_size() {
    // 2560 x 3520: downscaled to 2048 px on the long side for the preview.
    let (cols, rows) = (40, 55);
    let (w, h) = (cols * 64, rows * 64);
    let mut e = grid(cols, rows);
    e[100].2 = bump(&e[100].2, 0, 60);
    let s = diff(&grid(cols, rows), &e, w, h);
    assert_highlighted(&s);
    let (x, y, rw, rh) = s.layer_regions[0];
    let min = kra::MIN_REGION_FRACTION * w.max(h) as f64;
    assert!(
        rw * w as f64 >= min - 1.0 && rh * h as f64 >= min - 1.0,
        "{s:?}"
    );
    // Tile 100 sits at (20*64, 2*64) = (1280, 128); the region must cover that pixel.
    let (px, py) = (1280.0 / w as f64, 128.0 / h as f64);
    assert!(x <= px && px <= x + rw && y <= py && py <= y + rh, "{s:?}");
}

#[test]
fn min_size_region_stays_inside_the_canvas() {
    for b in [(0, 0, 1, 1), (99, 99, 1, 1), (50, 0, 1, 100)] {
        let (x, y, w, h) = kra::region_with_min_size(b, 100, 100);
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1.0 + 1e-9 && y + h <= 1.0 + 1e-9);
        assert!(w * 100.0 >= kra::MIN_REGION_PX as f64 - 1e-9);
    }
}
