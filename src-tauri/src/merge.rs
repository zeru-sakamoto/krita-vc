//! Combining two versions of the same `.kra` by folding their layer stacks together.
//!
//! Used when bringing a set-aside change back (`stash::pop`) onto a `.kra` that's been edited
//! since. Instead of refusing the conflict, we merge the layers the set-aside version actually
//! added or modified into the working file so the artist can reconcile them by hand in Krita.
//! Given the committed `ancestor` both sides diverged from, incoming layers unchanged since then
//! are skipped (compared on pixel/vector data + meaningful metadata, *not* the raw `maindoc.xml`
//! — Krita rewrites volatile attributes like `selected` every save) so unchanged layers aren't
//! duplicated. The rest land on top; a top-level layer whose name already exists is suffixed
//! ` [2]`, ` [3]`, … Every folded layer's data files and its uuid are remapped to fresh,
//! collision-free ids so the merged archive stays internally consistent.
//!
//! Deliberately narrow — it combines *whole* layers, it does not reconcile pixels within a layer
//! (that's the manual part the artist does). It refuses rather than emit a file Krita can't open
//! when the two versions use different color spaces; a canvas-*size* difference is allowed
//! through (Krita opens it — the incoming layers may just sit at an offset).
//!
//! The rewrite works on the raw `maindoc.xml` text: `roxmltree` (read-only) locates the exact
//! byte range of each incoming `<layer>` subtree and the base `<layers>` insertion point, and the
//! id/name swaps are string replaces of whole `attr="value"` tokens. uuids (`{hex}`) and
//! filenames (`layerN`) never contain XML-special characters, so those replaces are unambiguous.

use crate::error::{KvcError, Result};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive};

/// Fold `incoming`'s layers into `base` (both raw `.kra` bytes) and return the merged `.kra`.
///
/// `ancestor` is the committed version both `base` and `incoming` diverged from — the merge base.
/// Incoming top-level layers unchanged from it (same uuid, same pixel/vector data, same meaningful
/// metadata) are the ones the set-aside version never touched; they're skipped so only the artist's
/// added/modified layers fold in, instead of duplicating every unchanged layer. The comparison is
/// on layer *content*, never the raw `maindoc.xml` text — Krita rewrites volatile per-save
/// attributes (`selected`, …) every save, so an untouched layer's XML still differs between two
/// saves. `None` (no committed base) folds all.
pub fn merge_layers(base: &[u8], incoming: &[u8], ancestor: Option<&[u8]>) -> Result<Vec<u8>> {
    let base_doc = read_entry(base, "maindoc.xml")?;
    let inc_doc = read_entry(incoming, "maindoc.xml")?;
    let base_xml =
        std::str::from_utf8(&base_doc).map_err(|_| fail("base maindoc.xml is not UTF-8"))?;
    let inc_xml =
        std::str::from_utf8(&inc_doc).map_err(|_| fail("set-aside maindoc.xml is not UTF-8"))?;

    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let base_tree = roxmltree::Document::parse_with_options(base_xml, opts)
        .map_err(|e| fail(&format!("base maindoc: {e}")))?;
    let inc_tree = roxmltree::Document::parse_with_options(inc_xml, opts)
        .map_err(|e| fail(&format!("set-aside maindoc: {e}")))?;

    let base_image = image_node(&base_tree)?;
    let inc_image = image_node(&inc_tree)?;

    // Color space must match or the copied layer data is in the wrong pixel format and Krita
    // won't open it. Canvas size mismatch is allowed through (opens; layers may sit offset).
    let base_cs = base_image.attribute("colorspacename").unwrap_or("");
    let inc_cs = inc_image.attribute("colorspacename").unwrap_or("");
    if base_cs != inc_cs {
        return Err(fail(&format!(
            "different color space ({} vs {}) — bring the set-aside work back onto a clean file",
            or_q(base_cs),
            or_q(inc_cs)
        )));
    }

    let base_name = base_image.attribute("name").unwrap_or("");
    let inc_name = inc_image.attribute("name").unwrap_or("");
    let base_layers = layers_node(base_image)?;
    let inc_layers = layers_node(inc_image)?;

    // What already exists in base, so new ids/names avoid it. Filenames come from the maindoc
    // *and* the actual archive (an orphan `layers/layerN` with no maindoc entry would still
    // collide with a freshly-assigned name).
    let mut taken_names: HashSet<String> = descendant_attr(base_layers, "name");
    let mut taken_files: HashSet<String> = descendant_attr(base_layers, "filename");
    taken_files.extend(archive_layer_files(base, base_name)?);
    let taken_uuids: HashSet<String> = descendant_attr(base_layers, "uuid");

    // Fresh `layerN` filenames above base's highest numeric suffix.
    let mut next_fn = 1 + taken_files
        .iter()
        .filter_map(|f| f.strip_prefix("layer").and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    let mut used_fn: HashSet<String> = HashSet::new();
    let mut uid_ctr: u64 = 0;

    // Ancestor's top-level layers keyed by uuid, so an unchanged incoming layer can be recognised
    // and skipped rather than folded in as a duplicate.
    let anc_layers = match ancestor {
        Some(anc) => ancestor_top_layers(anc)?,
        None => HashMap::new(),
    };

    let mut fragments: Vec<String> = Vec::new();
    // (old filename, new filename) for every incoming layer data file, so `repackage` can copy
    // the archive entries under their new names.
    let mut file_renames: Vec<(String, String)> = Vec::new();

    // Top-level incoming layers, in document order (Krita writes top-first).
    for layer in inc_layers
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("layer"))
    {
        // Skip a layer the set-aside version left untouched — same tile pixels and same meaningful
        // metadata as the ancestor it diverged from, so folding it in would just duplicate an
        // unchanged layer into the working file. Only added/modified layers cross over. Any
        // difference, or an unmatched uuid, falls through and folds in, so no real change is
        // dropped. Compared on content, not raw XML: Krita's per-save attribute churn (`selected`
        // on the active layer, …) and its unstable tile *ordering* would otherwise make every
        // untouched layer look changed.
        if let Some((anc_sig, anc_content)) =
            layer.attribute("uuid").and_then(|u| anc_layers.get(u))
        {
            let files: HashSet<String> = subtree_attr(layer, "filename").into_iter().collect();
            if *anc_sig == layer_sig(layer)
                && *anc_content == layer_content(incoming, inc_name, &files)?
            {
                continue;
            }
        }

        let mut frag = inc_xml[layer.range()].to_string();

        // Remap every filename in this subtree (self + nested masks/children) to a fresh name.
        for old in subtree_attr(layer, "filename") {
            let new = loop {
                let cand = format!("layer{next_fn}");
                next_fn += 1;
                if !taken_files.contains(&cand) && !used_fn.contains(&cand) {
                    break cand;
                }
            };
            used_fn.insert(new.clone());
            frag = frag.replace(&attr("filename", &old), &attr("filename", &new));
            file_renames.push((old, new));
        }

        // Remap every uuid to a fresh unique one.
        for old in subtree_attr(layer, "uuid") {
            let new = loop {
                let cand = fresh_uuid(&old, uid_ctr);
                uid_ctr += 1;
                if !taken_uuids.contains(&cand) {
                    break cand;
                }
            };
            frag = frag.replace(&attr("uuid", &old), &attr("uuid", &new));
        }

        // Suffix the top-level layer's name on a clash — opening tag only, so nested layer/mask
        // names are left alone (name uniqueness isn't required, only clarity for the artist).
        if let Some(name) = layer.attribute("name") {
            if taken_names.contains(name) {
                let new = unique_name(name, &taken_names);
                let gt = frag.find('>').unwrap_or(frag.len());
                if let Some(open) = set_open_name(&frag[..gt], &xml_escape(&new)) {
                    frag = format!("{open}{}", &frag[gt..]);
                }
                taken_names.insert(new);
            } else {
                taken_names.insert(name.to_string());
            }
        }

        fragments.push(frag);
    }

    if fragments.is_empty() {
        // Either the set-aside file has no layers, or every layer matched the ancestor so its only
        // changes are outside the layer stack (e.g. canvas size) — nothing a layer merge can fold.
        return Err(fail("set-aside file has no changed layers to bring back"));
    }

    // Splice the fragments in as the first children of base's <layers> (top of the stack). The
    // first '>' after `<layers` closes its opening tag — Krita never puts '>' inside an
    // attribute value, so this is unambiguous.
    let lr = base_layers.range();
    let insert_at = lr.start
        + base_xml[lr.clone()]
            .find('>')
            .ok_or_else(|| fail("base <layers> tag is malformed"))?
        + 1;
    let mut merged = String::with_capacity(
        base_xml.len() + fragments.iter().map(String::len).sum::<usize>() + fragments.len(),
    );
    merged.push_str(&base_xml[..insert_at]);
    for frag in &fragments {
        merged.push('\n');
        merged.push_str(frag);
    }
    merged.push_str(&base_xml[insert_at..]);

    repackage(base, incoming, &merged, base_name, inc_name, &file_renames)
}

/// Rebuild the `.kra`: every base entry (with `maindoc.xml` swapped for the merged one), then the
/// incoming layer data files copied under their new names into base's `layers/` directory.
fn repackage(
    base: &[u8],
    incoming: &[u8],
    merged_maindoc: &str,
    base_image: &str,
    inc_image: &str,
    file_renames: &[(String, String)],
) -> Result<Vec<u8>> {
    let mut base_zip = ZipArchive::new(Cursor::new(base)).map_err(zip_err)?;
    let mut inc_zip = ZipArchive::new(Cursor::new(incoming)).map_err(zip_err)?;
    let rename: HashMap<&str, &str> = file_renames
        .iter()
        .map(|(o, n)| (o.as_str(), n.as_str()))
        .collect();

    let mut out = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(Cursor::new(&mut out));

        // Base entries in original order; mimetype stays first + stored, like Krita writes it.
        for i in 0..base_zip.len() {
            let mut f = base_zip.by_index(i).map_err(zip_err)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            zw.start_file(&name, opts(&name)).map_err(zip_err)?;
            if name == "maindoc.xml" {
                zw.write_all(merged_maindoc.as_bytes())?;
            } else {
                let buf = crate::repo::read_entry_capped(&mut f)?;
                zw.write_all(&buf)?;
            }
        }

        // Incoming layer files -> base's layers/ dir under fresh names. `layer2.defaultpixel` and
        // `layer2.shapelayer/content.svg` ride along on their filename component.
        let inc_prefix = format!("{inc_image}/layers/");
        let base_prefix = format!("{base_image}/layers/");
        for i in 0..inc_zip.len() {
            let mut f = inc_zip.by_index(i).map_err(zip_err)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            let Some(rest) = name.strip_prefix(&inc_prefix) else {
                continue;
            };
            let split = rest.find(['.', '/']).unwrap_or(rest.len());
            let Some(&new_fn) = rename.get(&rest[..split]) else {
                continue;
            };
            let new_name = format!("{base_prefix}{new_fn}{}", &rest[split..]);
            let buf = crate::repo::read_entry_capped(&mut f)?;
            zw.start_file(&new_name, opts(&new_name)).map_err(zip_err)?;
            zw.write_all(&buf)?;
        }
        zw.finish().map_err(zip_err)?;
    }
    Ok(out)
}

// --- small helpers ---------------------------------------------------------------------

fn fail(msg: &str) -> KvcError {
    KvcError::MergeFailed(msg.to_string())
}

fn or_q(s: &str) -> &str {
    if s.is_empty() {
        "?"
    } else {
        s
    }
}

fn zip_err(e: zip::result::ZipError) -> KvcError {
    KvcError::CorruptZip(e.to_string())
}

/// mimetype stored first, everything else deflated — the shape Krita reads.
fn opts(name: &str) -> SimpleFileOptions {
    let method = if name == "mimetype" {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    SimpleFileOptions::default().compression_method(method)
}

fn attr(key: &str, value: &str) -> String {
    format!("{key}=\"{value}\"")
}

fn read_entry(zip_bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(zip_bytes)).map_err(zip_err)?;
    let mut f = zip
        .by_name(name)
        .map_err(|_| fail(&format!("missing {name}")))?;
    let buf = crate::repo::read_entry_capped(&mut f)?;
    Ok(buf)
}

fn image_node<'a>(doc: &'a roxmltree::Document<'a>) -> Result<roxmltree::Node<'a, 'a>> {
    doc.descendants()
        .find(|n| n.has_tag_name("IMAGE"))
        .ok_or_else(|| fail("maindoc.xml has no <IMAGE>"))
}

/// The `<layers>` element that is a direct child of `<IMAGE>` — the document's top-level stack.
fn layers_node<'a>(image: roxmltree::Node<'a, 'a>) -> Result<roxmltree::Node<'a, 'a>> {
    image
        .children()
        .find(|n| n.is_element() && n.has_tag_name("layers"))
        .ok_or_else(|| fail("maindoc.xml has no <layers>"))
}

/// Every value of `attr` across `node` and its descendants (`descendants()` includes `node`).
fn descendant_attr(node: roxmltree::Node, attr: &str) -> HashSet<String> {
    node.descendants()
        .filter_map(|n| n.attribute(attr).map(str::to_string))
        .collect()
}

/// Like [`descendant_attr`] but a de-duplicated list in document order (stable id remapping).
fn subtree_attr(node: roxmltree::Node, attr: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    node.descendants()
        .filter_map(|n| n.attribute(attr).map(str::to_string))
        .filter(|v| seen.insert(v.clone()))
        .collect()
}

/// Filename components of every `<image>/layers/<name>...` entry in a `.kra` archive.
fn archive_layer_files(kra: &[u8], image: &str) -> Result<HashSet<String>> {
    let prefix = format!("{image}/layers/");
    let mut zip = ZipArchive::new(Cursor::new(kra)).map_err(zip_err)?;
    let mut out = HashSet::new();
    for i in 0..zip.len() {
        let f = zip.by_index(i).map_err(zip_err)?;
        if let Some(rest) = f.name().strip_prefix(&prefix) {
            let split = rest.find(['.', '/']).unwrap_or(rest.len());
            if split > 0 {
                out.insert(rest[..split].to_string());
            }
        }
    }
    Ok(out)
}

/// Layer attributes that mark a real creative choice — not Krita's per-save UI churn (`selected`
/// on the active layer, `collapsed`, timeline flags, …). A change to one means the layer was
/// modified. Kept deliberately small: an attribute left off here is simply ignored, which at worst
/// skips folding an obscure metadata tweak — never a spurious duplicate, the bug this fixes.
const LAYER_ATTRS: &[&str] = &["name", "opacity", "compositeop", "visible", "x", "y"];

/// Order-independent signature of a layer's meaningful metadata (parsed attributes, not raw text),
/// ignoring the volatile attributes Krita rewrites every save.
fn layer_sig(layer: roxmltree::Node) -> String {
    LAYER_ATTRS
        .iter()
        .map(|k| format!("{k}={}", layer.attribute(*k).unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\u{0}")
}

/// Ancestor's top-level layers keyed by uuid → (metadata signature, canonical tile content). An
/// incoming layer whose uuid, signature *and* content all match one of these is unchanged since the
/// set-aside version diverged, so it's skipped rather than folded in as a duplicate.
fn ancestor_top_layers(anc: &[u8]) -> Result<HashMap<String, (String, Vec<Vec<u8>>)>> {
    let doc = read_entry(anc, "maindoc.xml")?;
    let xml = std::str::from_utf8(&doc).map_err(|_| fail("ancestor maindoc.xml is not UTF-8"))?;
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let tree = roxmltree::Document::parse_with_options(xml, opts)
        .map_err(|e| fail(&format!("ancestor maindoc: {e}")))?;
    let image = image_node(&tree)?;
    let name = image.attribute("name").unwrap_or("");
    let layers = layers_node(image)?;
    let mut out = HashMap::new();
    for layer in layers
        .children()
        .filter(|n| n.is_element() && n.has_tag_name("layer"))
    {
        if let Some(uuid) = layer.attribute("uuid") {
            let files: HashSet<String> = subtree_attr(layer, "filename").into_iter().collect();
            out.insert(
                uuid.to_string(),
                (layer_sig(layer), layer_content(anc, name, &files)?),
            );
        }
    }
    Ok(out)
}

/// A layer's canonical content: every `<image>/layers/<file>...` archive entry for `files`, each
/// normalized (see [`canon_entry`]), collected into a sorted, filename-independent multiset.
/// Sorting (not keying by name) is deliberate — Krita can renumber the `layerN` files between saves
/// even when the pixels are untouched, so two unchanged versions of a layer must still compare
/// equal.
fn layer_content(kra: &[u8], image: &str, files: &HashSet<String>) -> Result<Vec<Vec<u8>>> {
    let prefix = format!("{image}/layers/");
    let mut zip = ZipArchive::new(Cursor::new(kra)).map_err(zip_err)?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(zip_err)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let split = rest.find(['.', '/']).unwrap_or(rest.len());
        if files.contains(&rest[..split]) {
            let buf = crate::repo::read_entry_capped(&mut f)?;
            out.push(canon_entry(buf));
        }
    }
    out.sort();
    Ok(out)
}

/// Canonical, order-independent form of one layer data entry. A tiled paint blob becomes its tiles
/// sorted by position — Krita's tile *order* within a layer's data file isn't stable across saves,
/// so two saves that wrote the same tiles in a different order (and thus reconstruct to different
/// bytes) must still compare equal. Anything that isn't a tile block (`.defaultpixel`, `.icc`,
/// shape-layer SVG, …) is compared verbatim.
fn canon_entry(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(block) = crate::tiles::parse(&bytes) else {
        return bytes;
    };
    let mut tiles: Vec<&crate::tiles::Tile> = block.tiles.iter().collect();
    tiles.sort_by_key(|t| (t.x, t.y));
    let mut out = block.header.into_bytes();
    for t in tiles {
        out.extend_from_slice(
            format!("\n{},{},{},{}\n", t.x, t.y, t.compression, t.data.len()).as_bytes(),
        );
        out.extend_from_slice(&t.data);
    }
    out
}

/// `name`, then `name [2]`, `name [3]`, … until one isn't taken.
fn unique_name(name: &str, taken: &HashSet<String>) -> String {
    (2..)
        .map(|n| format!("{name} [{n}]"))
        .find(|c| !taken.contains(c))
        .unwrap()
}

/// A uuid-shaped `{8-4-4-4-12}` id derived from a seed — no `uuid` crate, and stable/unique
/// enough for layer identity (checked against the base's uuids by the caller).
fn fresh_uuid(seed: &str, ctr: u64) -> String {
    let hex = blake3::hash(format!("kvc-merge\0{seed}\0{ctr}").as_bytes()).to_hex();
    let s = &hex[..32];
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )
}

/// Replace the value of the first ` name="…"` attribute in an opening tag. Leading space anchors
/// it to a real attribute boundary (so it never matches inside `filename="…"`). The closing quote
/// is unambiguous because a literal `"` in a value is always written `&quot;`. `None` if absent.
fn set_open_name(open_tag: &str, escaped_new: &str) -> Option<String> {
    let key = " name=\"";
    let vstart = open_tag.find(key)? + key.len();
    let vend = vstart + open_tag[vstart..].find('"')?;
    Some(format!(
        "{}{}{}",
        &open_tag[..vstart],
        escaped_new,
        &open_tag[vend..]
    ))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
