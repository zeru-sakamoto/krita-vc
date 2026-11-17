//! A layer's own properties (visibility, opacity, blend mode, name, place in the stack) show up in
//! the diff as what they were **and** what they became, alongside any pixel change.
//!
//! Before this, `LayerDto` only carried the new side's properties, so hiding a layer showed
//! "Hidden" with no sign it used to be shown, and a visibility toggle on its own didn't even count
//! as a change.

use krita_vc_lib::{commands, commit, kra, raster, repo};
use serde_json::Value;
use std::io::Write;

const N: usize = 64 * 64;

/// One LZF-compressed 64 x 64 tile layer data file, every pixel the same grey.
fn layer_file(grey: u8) -> Vec<u8> {
    let mut raw = Vec::new();
    for v in [grey, grey, grey, 255] {
        raw.extend(std::iter::repeat_n(v, N));
    }
    let mut d = vec![1u8];
    d.extend(raster::lzf_compress(&raw));
    let mut block = b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 1\n".to_vec();
    block.extend(format!("0,0,LZF,{}\n", d.len()).as_bytes());
    block.extend(d);
    block
}

/// A paint layer element. `extra` is spliced into the tag, e.g. `visible="0"`.
fn paint(uuid: &str, name: &str, file: &str, extra: &str) -> String {
    format!(
        r#"<layer name="{name}" uuid="{uuid}" nodetype="paintlayer" filename="{file}" {}/>"#,
        props(extra)
    )
}

fn group(uuid: &str, name: &str, extra: &str, children: &[String]) -> String {
    format!(
        r#"<layer name="{name}" uuid="{uuid}" nodetype="grouplayer" {}><layers>{}</layers></layer>"#,
        props(extra),
        children.concat()
    )
}

/// visible/opacity/compositeop at their defaults, with any given in `extra` (`key="value"`
/// pairs) taking their place. XML refuses a repeated attribute, so overrides replace.
fn props(extra: &str) -> String {
    [
        ("visible", "1"),
        ("opacity", "255"),
        ("compositeop", "normal"),
    ]
    .iter()
    .map(|(k, v)| {
        extra
            .split_whitespace()
            .find(|kv| kv.starts_with(&format!("{k}=")))
            .map_or_else(|| format!(r#"{k}="{v}""#), str::to_string)
    })
    .collect::<Vec<_>>()
    .join(" ")
}

/// A `.kra` with `stack` (top-first, as Krita writes it) and one data file per name in `files`.
fn kra_bytes(stack: &[String], files: &[(&str, u8)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let md = format!(
        r#"<!DOCTYPE DOC><DOC><IMAGE name="img" width="64" height="64"><layers>{}</layers></IMAGE></DOC>"#,
        stack.concat()
    );
    let mut out = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/x-krita").unwrap();
        z.start_file("maindoc.xml", o).unwrap();
        z.write_all(md.as_bytes()).unwrap();
        for (f, grey) in files {
            z.start_file(format!("img/layers/{f}"), o).unwrap();
            z.write_all(&layer_file(*grey)).unwrap();
        }
        z.finish().unwrap();
    }
    out
}

/// Commit `base`, diff `edited` against it as unsaved work, then commit `edited` and diff it as
/// history. Both must agree; returns the history diff's layers.
fn diff(base: Vec<u8>, edited: Vec<u8>) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("p.kra");
    std::fs::write(&doc, &base).unwrap();
    repo::Repo::init(&doc).unwrap();
    let mut r = repo::Repo::open(&doc).unwrap();
    let c0 = commit::commit_snapshot(&mut r, "base", "test").unwrap();
    let f0 = c0.files.iter().find(|f| f.is_kra).unwrap().clone();
    let m0 = kra::load_manifest(&r, &f0.path, f0.content.as_deref().unwrap()).unwrap();

    let wk = kra::parse_working(&edited, false).unwrap();
    let working = commands::art_diff_dto(
        &r,
        &f0.path,
        "M",
        &kra::KraSource::Working(&wk),
        Some(&*m0),
        true,
        None,
    )
    .unwrap();

    std::fs::write(&doc, &edited).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "edit", "test").unwrap();
    let f1 = c1.files.iter().find(|f| f.is_kra).unwrap().clone();
    let committed = commands::committed_art_dto(&r, &f1, Some(&f0), true, None).unwrap();

    let strip = |d: &commands::ArtDiffDto| -> Vec<Value> {
        // Raster URLs differ between the two paths; everything this test asserts on doesn't.
        let v = serde_json::to_value(d).unwrap();
        v["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                let mut l = l.clone();
                let o = l.as_object_mut().unwrap();
                for k in [
                    "before",
                    "after",
                    "beforeThumb",
                    "afterThumb",
                    "diffImage",
                    "diffOutline",
                    "regions",
                    "bounds",
                ] {
                    o.remove(k);
                }
                l
            })
            .collect()
    };
    assert_eq!(
        strip(&working),
        strip(&committed),
        "working vs committed diff disagree"
    );
    serde_json::to_value(&committed).unwrap()["layers"]
        .as_array()
        .unwrap()
        .clone()
}

fn layer<'a>(layers: &'a [Value], name: &str) -> &'a Value {
    layers
        .iter()
        .find(|l| l["name"] == name)
        .unwrap_or_else(|| panic!("no layer {name} in {layers:#?}"))
}

fn changes(l: &Value) -> Vec<&str> {
    l["changes"]
        .as_array()
        .map(|a| a.iter().map(|c| c.as_str().unwrap()).collect())
        .unwrap_or_default()
}

#[test]
fn hiding_a_layer_is_a_change_with_its_old_visibility() {
    let base = kra_bytes(&[paint("a", "Sketch", "layer1", "")], &[("layer1", 100)]);
    let edited = kra_bytes(
        &[paint("a", "Sketch", "layer1", r#"visible="0""#)],
        &[("layer1", 100)],
    );
    let ls = diff(base, edited);
    let l = layer(&ls, "Sketch");
    assert_eq!(l["change"], "modified");
    assert_eq!(changes(l), ["visibility"]);
    assert_eq!(l["visible"], false);
    assert_eq!(l["prev"]["visible"], true);
}

#[test]
fn a_property_only_change_has_no_pixel_highlight() {
    let base = kra_bytes(&[paint("a", "Sketch", "layer1", "")], &[("layer1", 100)]);
    let edited = kra_bytes(
        &[paint("a", "Sketch", "layer1", r#"opacity="102""#)],
        &[("layer1", 100)],
    );
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("p.kra");
    std::fs::write(&doc, &base).unwrap();
    repo::Repo::init(&doc).unwrap();
    let mut r = repo::Repo::open(&doc).unwrap();
    let c0 = commit::commit_snapshot(&mut r, "base", "test").unwrap();
    std::fs::write(&doc, &edited).unwrap();
    let c1 = commit::commit_snapshot(&mut r, "edit", "test").unwrap();
    let f0 = c0.files.iter().find(|f| f.is_kra).unwrap();
    let f1 = c1.files.iter().find(|f| f.is_kra).unwrap();
    let dto = commands::committed_art_dto(&r, f1, Some(f0), true, None).unwrap();
    let v = serde_json::to_value(&dto).unwrap();
    let l = &v["layers"][0];
    assert_eq!(l["change"], "modified");
    assert!(l.get("regions").is_none(), "{l:#?}");
    assert!(l.get("diffImage").is_none(), "{l:#?}");
    // Same pixels both sides: the before markup is the after markup, not a second raster.
    assert_eq!(l["before"], l["after"]);
}

#[test]
fn a_repaint_and_an_opacity_change_are_both_listed() {
    let base = kra_bytes(&[paint("a", "Lineart", "layer1", "")], &[("layer1", 100)]);
    let edited = kra_bytes(
        &[paint("a", "Lineart", "layer1", r#"opacity="102""#)],
        &[("layer1", 200)],
    );
    let ls = diff(base, edited);
    let l = layer(&ls, "Lineart");
    assert_eq!(changes(l), ["pixels", "opacity"]);
    assert_eq!(l["opacity"], 40);
    assert_eq!(l["prev"]["opacity"], 100);
}

#[test]
fn a_pixel_only_change_has_no_prev() {
    let base = kra_bytes(&[paint("a", "Lineart", "layer1", "")], &[("layer1", 100)]);
    let edited = kra_bytes(&[paint("a", "Lineart", "layer1", "")], &[("layer1", 200)]);
    let ls = diff(base, edited);
    let l = layer(&ls, "Lineart");
    assert_eq!(changes(l), ["pixels"]);
    assert!(l.get("prev").is_none());
}

#[test]
fn renaming_keeps_the_old_name() {
    let base = kra_bytes(&[paint("a", "Sketch", "layer1", "")], &[("layer1", 100)]);
    let edited = kra_bytes(&[paint("a", "Lineart", "layer1", "")], &[("layer1", 100)]);
    let ls = diff(base, edited);
    let l = layer(&ls, "Lineart");
    assert_eq!(changes(l), ["renamed"]);
    assert_eq!(l["prev"]["name"], "Sketch");
}

#[test]
fn blend_modes_the_canvas_cant_draw_still_diff_by_name() {
    let base = kra_bytes(
        &[paint("a", "Shade", "layer1", r#"compositeop="darken""#)],
        &[("layer1", 100)],
    );
    let edited = kra_bytes(
        &[paint("a", "Shade", "layer1", r#"compositeop="lighten""#)],
        &[("layer1", 100)],
    );
    let ls = diff(base, edited);
    let l = layer(&ls, "Shade");
    assert_eq!(changes(l), ["blend"]);
    // Both fold to "normal" for drawing; the raw op is what tells them apart.
    assert_eq!(l["blendMode"], "normal");
    assert_eq!(l["blendOp"], "lighten");
    assert_eq!(l["prev"]["blendOp"], "darken");
}

#[test]
fn unchanged_layers_carry_no_changes() {
    // Another layer is repainted so there is something to commit.
    let stack = [
        paint("a", "Sketch", "layer1", ""),
        paint("b", "Colour", "layer2", ""),
    ];
    let ls = diff(
        kra_bytes(&stack, &[("layer1", 100), ("layer2", 50)]),
        kra_bytes(&stack, &[("layer1", 100), ("layer2", 60)]),
    );
    let l = layer(&ls, "Sketch");
    assert_eq!(l["change"], "unchanged");
    assert!(l.get("changes").is_none());
    assert!(l.get("prev").is_none());
}

fn four(order: [&str; 4]) -> Vec<String> {
    order
        .iter()
        .map(|n| paint(n, n, &format!("layer_{n}"), ""))
        .collect()
}
const FOUR_FILES: [(&str, u8); 4] = [
    ("layer_A", 10),
    ("layer_B", 20),
    ("layer_C", 30),
    ("layer_D", 40),
];

#[test]
fn dragging_one_layer_flags_only_that_layer() {
    // D dragged from the bottom to the top: A, B and C all shift down one, but only D moved.
    let ls = diff(
        kra_bytes(&four(["A", "B", "C", "D"]), &FOUR_FILES),
        kra_bytes(&four(["D", "A", "B", "C"]), &FOUR_FILES),
    );
    assert_eq!(changes(layer(&ls, "D")), ["moved"]);
    for n in ["A", "B", "C"] {
        assert_eq!(layer(&ls, n)["change"], "unchanged", "{n}");
    }
}

#[test]
fn adding_or_deleting_a_layer_doesnt_move_the_others() {
    let mut with_e = four(["A", "B", "C", "D"]);
    with_e.insert(2, paint("E", "E", "layer_E", ""));
    let mut files = FOUR_FILES.to_vec();
    files.push(("layer_E", 50));
    let ls = diff(
        kra_bytes(&four(["A", "B", "C", "D"]), &FOUR_FILES),
        kra_bytes(&with_e, &files),
    );
    assert_eq!(layer(&ls, "E")["change"], "added");
    for n in ["A", "B", "C", "D"] {
        assert_eq!(layer(&ls, n)["change"], "unchanged", "{n}");
    }

    let three: Vec<String> = ["A", "C", "D"]
        .iter()
        .map(|n| paint(n, n, &format!("layer_{n}"), ""))
        .collect();
    let ls = diff(
        kra_bytes(&four(["A", "B", "C", "D"]), &FOUR_FILES),
        kra_bytes(&three, &FOUR_FILES),
    );
    assert_eq!(layer(&ls, "B")["change"], "removed");
    for n in ["A", "C", "D"] {
        assert_eq!(layer(&ls, n)["change"], "unchanged", "{n}");
    }
}

#[test]
fn moving_a_layer_into_a_group_records_where_it_came_from() {
    let base = kra_bytes(
        &[
            group("g", "Face", "", &[paint("a", "Eyes", "layer1", "")]),
            paint("b", "Mouth", "layer2", ""),
        ],
        &[("layer1", 10), ("layer2", 20)],
    );
    let edited = kra_bytes(
        &[group(
            "g",
            "Face",
            "",
            &[
                paint("a", "Eyes", "layer1", ""),
                paint("b", "Mouth", "layer2", ""),
            ],
        )],
        &[("layer1", 10), ("layer2", 20)],
    );
    let ls = diff(base, edited);
    let m = layer(&ls, "Mouth");
    assert_eq!(changes(m), ["moved"]);
    assert_eq!(m["parentId"], "g");
    assert!(m["prev"].get("parentId").is_none(), "was top-level: {m:#?}");
    assert_eq!(layer(&ls, "Eyes")["change"], "unchanged");
    assert_eq!(layer(&ls, "Eyes")["parentId"], "g");
}

#[test]
fn nested_layers_know_their_own_group() {
    let stack = [group(
        "outer",
        "Character",
        "",
        &[group(
            "inner",
            "Face",
            "",
            &[paint("a", "Eyes", "layer1", "")],
        )],
    )];
    let ls = diff(
        kra_bytes(&stack, &[("layer1", 10)]),
        kra_bytes(&stack, &[("layer1", 20)]),
    );
    let eyes = layer(&ls, "Eyes");
    assert_eq!(eyes["parentId"], "inner");
    assert_eq!(eyes["topLevelId"], "outer");
    assert_eq!(layer(&ls, "Face")["parentId"], "outer");
    assert!(layer(&ls, "Character").get("parentId").is_none());
}

#[test]
fn hiding_a_group_flags_the_group_not_its_children() {
    let kids = [
        paint("a", "Eyes", "layer1", ""),
        paint("b", "Mouth", "layer2", ""),
    ];
    let files = [("layer1", 10), ("layer2", 20)];
    let ls = diff(
        kra_bytes(&[group("g", "Face", "", &kids)], &files),
        kra_bytes(&[group("g", "Face", r#"visible="0""#, &kids)], &files),
    );
    assert_eq!(changes(layer(&ls, "Face")), ["visibility"]);
    assert_eq!(layer(&ls, "Eyes")["change"], "unchanged");
    assert_eq!(layer(&ls, "Mouth")["change"], "unchanged");
}
