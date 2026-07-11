//! Color-palette parsing + swatch-level diffing for the four formats Krita VCS understands:
//! GIMP `.gpl` (text), Krita `.kpl` (zip + XML), Adobe `.aco` (binary), and Adobe `.ase`
//! (binary). Everything is reduced to a flat list of named sRGB swatches; the diff matches
//! swatches by name so a recolor reads as "modified" rather than remove+add.
//!
//! Tauri-free on purpose (unit-tested directly). The command layer wraps [`diff`] into a
//! serde DTO — see `commands.rs`.

use std::io::Read;

/// One named color, reduced to 8-bit sRGB.
pub struct Swatch {
    pub name: String,
    pub rgb: (u8, u8, u8),
}

/// A parsed palette: its swatches plus the grid column count when the format records one
/// (0 = unknown; the frontend falls back to 4).
pub struct Palette {
    pub columns: u32,
    pub swatches: Vec<Swatch>,
}

/// One swatch's fate between two palette versions. `before`/`after` are `#RRGGBB` (uppercase),
/// `None` on the side where the swatch is absent.
pub struct SwatchDiff {
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub change: &'static str, // "added" | "removed" | "modified" | "unchanged"
}

pub struct PaletteDiff {
    pub columns: u32,
    pub swatches: Vec<SwatchDiff>,
}

/// True for the palette extensions Krita VCS tracks + diffs. Case-insensitive.
pub fn is_palette(path: &str) -> bool {
    let lower = path.to_lowercase();
    [".gpl", ".kpl", ".aco", ".ase"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Parse a palette by extension. `None` when the extension isn't a palette or the bytes don't
/// parse — callers degrade to a plain text/placeholder entry rather than failing the whole diff.
pub fn parse(path: &str, bytes: &[u8]) -> Option<Palette> {
    let lower = path.to_lowercase();
    if lower.ends_with(".gpl") {
        parse_gpl(bytes)
    } else if lower.ends_with(".kpl") {
        parse_kpl(bytes)
    } else if lower.ends_with(".aco") {
        parse_aco(bytes)
    } else if lower.ends_with(".ase") {
        parse_ase(bytes)
    } else {
        None
    }
}

fn hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2)
}

/// Diff two parsed palettes (either side may be absent — an add or a delete). Swatches match by
/// name, first-unconsumed (names can repeat); leftover old swatches are "removed", appended last.
pub fn diff(old: Option<&Palette>, new: Option<&Palette>) -> PaletteDiff {
    let empty: Vec<Swatch> = Vec::new();
    let olds = old.map(|p| &p.swatches).unwrap_or(&empty);
    let news = new.map(|p| &p.swatches).unwrap_or(&empty);

    let mut used = vec![false; olds.len()];
    let mut swatches = Vec::with_capacity(news.len() + olds.len());

    for ns in news {
        let a = hex(ns.rgb);
        let matched = olds
            .iter()
            .enumerate()
            .find(|(i, os)| !used[*i] && os.name == ns.name);
        match matched {
            Some((i, os)) => {
                used[i] = true;
                let b = hex(os.rgb);
                let change = if b == a { "unchanged" } else { "modified" };
                swatches.push(SwatchDiff {
                    name: ns.name.clone(),
                    before: Some(b),
                    after: Some(a),
                    change,
                });
            }
            None => swatches.push(SwatchDiff {
                name: ns.name.clone(),
                before: None,
                after: Some(a),
                change: "added",
            }),
        }
    }
    for (i, os) in olds.iter().enumerate() {
        if !used[i] {
            swatches.push(SwatchDiff {
                name: os.name.clone(),
                before: Some(hex(os.rgb)),
                after: None,
                change: "removed",
            });
        }
    }

    let columns = new
        .map(|p| p.columns)
        .filter(|c| *c > 0)
        .unwrap_or_else(|| old.map(|p| p.columns).unwrap_or(0));
    PaletteDiff { columns, swatches }
}

// --- GIMP .gpl (text) ---------------------------------------------------------------------

fn parse_gpl(bytes: &[u8]) -> Option<Palette> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut columns = 0u32;
    let mut swatches = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Columns:") {
            columns = rest.trim().parse().unwrap_or(0);
            continue;
        }
        // Header lines ("GIMP Palette", "Name: ...") and any other non-color line are skipped.
        let mut it = line.split_whitespace();
        let (r, g, b) = match (
            it.next().and_then(|t| t.parse::<i32>().ok()),
            it.next().and_then(|t| t.parse::<i32>().ok()),
            it.next().and_then(|t| t.parse::<i32>().ok()),
        ) {
            (Some(r), Some(g), Some(b)) => (r, g, b),
            _ => continue,
        };
        let name = it.collect::<Vec<_>>().join(" ");
        let rgb = (clamp8(r), clamp8(g), clamp8(b));
        swatches.push(Swatch {
            name: if name.is_empty() { hex(rgb) } else { name },
            rgb,
        });
    }
    if swatches.is_empty() {
        return None;
    }
    Some(Palette { columns, swatches })
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

// --- Krita .kpl (zip of colorset.xml) -----------------------------------------------------

fn parse_kpl(bytes: &[u8]) -> Option<Palette> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut xml = String::new();
    zip.by_name("colorset.xml")
        .ok()?
        .read_to_string(&mut xml)
        .ok()?;
    let doc = roxmltree::Document::parse(&xml).ok()?;
    let root = doc.root_element();
    let columns = root
        .attribute("columns")
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);

    let mut swatches = Vec::new();
    for entry in root.children().filter(|n| n.has_tag_name("ColorSetEntry")) {
        let name = entry.attribute("name").unwrap_or("").trim().to_string();
        // The color lives in a child element whose tag names the color space: RGB/sRGB carry
        // r/g/b as 0..1 floats. Other spaces are rare in .kpl; convert what's cheap, else skip.
        let rgb = entry
            .children()
            .filter(|n| n.is_element())
            .find_map(kpl_node_rgb);
        if let Some(rgb) = rgb {
            swatches.push(Swatch {
                name: if name.is_empty() { hex(rgb) } else { name },
                rgb,
            });
        }
    }
    if swatches.is_empty() {
        return None;
    }
    Some(Palette { columns, swatches })
}

fn kpl_node_rgb(n: roxmltree::Node) -> Option<(u8, u8, u8)> {
    let f = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok());
    match n.tag_name().name() {
        "RGB" | "sRGB" => Some((unit8(f("r")?), unit8(f("g")?), unit8(f("b")?))),
        "Gray" | "GRAY" => {
            let g = unit8(f("g")?);
            Some((g, g, g))
        }
        "CMYK" => Some(cmyk_to_rgb(f("c")?, f("m")?, f("y")?, f("k")?)),
        _ => None,
    }
}

/// 0..1 float → 0..255 (rounded, clamped).
fn unit8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> (u8, u8, u8) {
    let ch = |x: f64| unit8((1.0 - x) * (1.0 - k));
    (ch(c), ch(m), ch(y))
}

// --- binary reader ------------------------------------------------------------------------

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Option<f32> {
        self.take(4)
            .map(|s| f32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
}

// --- Adobe .aco (binary) ------------------------------------------------------------------

fn parse_aco(bytes: &[u8]) -> Option<Palette> {
    let mut r = Reader::new(bytes);
    if r.u16()? != 1 {
        return None; // every .aco opens with a v1 section
    }
    let count = r.u16()? as usize;
    let mut colors = Vec::with_capacity(count);
    for _ in 0..count {
        let space = r.u16()?;
        let comps = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        colors.push((space, comps));
    }

    // Optional v2 section repeats the colors with UTF-16BE names. Best-effort: on any misparse,
    // keep the v1 colors with hex names (a recolor then reads as remove+add, still correct).
    let names = parse_aco_v2_names(&mut r, count).unwrap_or_default();

    let swatches = colors
        .into_iter()
        .enumerate()
        .map(|(i, (space, comps))| {
            let rgb = aco_rgb(space, comps);
            let name = names
                .get(i)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| hex(rgb));
            Swatch { name, rgb }
        })
        .collect::<Vec<_>>();
    if swatches.is_empty() {
        return None;
    }
    Some(Palette {
        columns: 0,
        swatches,
    })
}

fn parse_aco_v2_names(r: &mut Reader, count: usize) -> Option<Vec<String>> {
    if r.remaining() == 0 {
        return None;
    }
    if r.u16()? != 2 {
        return None;
    }
    let v2_count = r.u16()? as usize;
    let n = count.min(v2_count);
    let mut names = Vec::with_capacity(n);
    for _ in 0..v2_count {
        let _space = r.u16()?;
        let _comps = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let len = r.u32()? as usize; // UTF-16 code units including trailing null
        let units = r.take(len.checked_mul(2)?)?;
        names.push(utf16be_string(units));
    }
    names.truncate(n);
    Some(names)
}

/// ACO stores 0..65535 components. Space 0 = RGB, 8 = grayscale (0..10000), 2 = CMYK
/// (0..65535, inverted). Others (HSB, Lab, wide CMYK) are rare here — mid-gray placeholder.
fn aco_rgb(space: u16, c: [u16; 4]) -> (u8, u8, u8) {
    let to8 = |v: u16| (v as f64 / 65535.0 * 255.0).round() as u8;
    match space {
        0 => (to8(c[0]), to8(c[1]), to8(c[2])),
        8 => {
            let g = (c[0] as f64 / 10000.0 * 255.0).round().clamp(0.0, 255.0) as u8;
            (g, g, g)
        }
        2 => cmyk_to_rgb(
            1.0 - c[0] as f64 / 65535.0,
            1.0 - c[1] as f64 / 65535.0,
            1.0 - c[2] as f64 / 65535.0,
            1.0 - c[3] as f64 / 65535.0,
        ),
        _ => (128, 128, 128),
    }
}

// --- Adobe .ase (binary) ------------------------------------------------------------------

fn parse_ase(bytes: &[u8]) -> Option<Palette> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != b"ASEF" {
        return None;
    }
    let _version = r.u32()?; // major/minor as two u16s; unused
    let blocks = r.u32()?;
    let mut swatches = Vec::new();
    for _ in 0..blocks {
        let kind = r.u16()?;
        let len = r.u32()? as usize;
        let body = r.take(len)?; // consume the whole block; parse color blocks from the slice
        if kind == 0x0001 {
            if let Some(sw) = parse_ase_color(body) {
                swatches.push(sw);
            }
        }
        // 0xC001 group-start / 0xC002 group-end carry no color — skipped.
    }
    if swatches.is_empty() {
        return None;
    }
    Some(Palette {
        columns: 0,
        swatches,
    })
}

fn parse_ase_color(body: &[u8]) -> Option<Swatch> {
    let mut r = Reader::new(body);
    let name_len = r.u16()? as usize; // UTF-16 code units including trailing null
    let name_units = r.take(name_len.checked_mul(2)?)?;
    let name = utf16be_string(name_units);
    let model = r.take(4)?;
    let rgb = match model {
        b"RGB " => (
            unit8(r.f32()? as f64),
            unit8(r.f32()? as f64),
            unit8(r.f32()? as f64),
        ),
        b"CMYK" => cmyk_to_rgb(
            r.f32()? as f64,
            r.f32()? as f64,
            r.f32()? as f64,
            r.f32()? as f64,
        ),
        b"Gray" => {
            let g = unit8(r.f32()? as f64);
            (g, g, g)
        }
        b"LAB " => lab_to_rgb(r.f32()? as f64, r.f32()? as f64, r.f32()? as f64),
        _ => return None,
    };
    Some(Swatch {
        name: if name.is_empty() { hex(rgb) } else { name },
        rgb,
    })
}

/// UTF-16BE decode, dropping the trailing null and any decode errors.
fn utf16be_string(units: &[u8]) -> String {
    let u16s: Vec<u16> = units
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&u16s)
}

/// CIE Lab (L 0..100, a/b ~-128..127) → 8-bit sRGB, D50 white (Adobe's Lab reference). Enough
/// for a swatch preview; not a color-managed conversion.
fn lab_to_rgb(l: f64, a: f64, b: f64) -> (u8, u8, u8) {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let g = |t: f64| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    // D50 white point.
    let (xn, yn, zn) = (0.9642, 1.0, 0.8249);
    let (x, y, z) = (xn * g(fx), yn * g(fy), zn * g(fz));
    // XYZ (D50) → linear sRGB (Bradford-adapted matrix).
    let rl = 3.1338561 * x - 1.6168667 * y - 0.4906146 * z;
    let gl = -0.9787684 * x + 1.9161415 * y + 0.0334540 * z;
    let bl = 0.0719453 * x - 0.2289914 * y + 1.4052427 * z;
    let comp = |c: f64| {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.0031308 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    (comp(rl), comp(gl), comp(bl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpl_parses_colors_and_columns() {
        let src = b"GIMP Palette\nName: Test\nColumns: 3\n#\n255 0 0 Red\n0 255 0\tGreen\n";
        let p = parse_gpl(src).unwrap();
        assert_eq!(p.columns, 3);
        assert_eq!(p.swatches.len(), 2);
        assert_eq!(p.swatches[0].name, "Red");
        assert_eq!(p.swatches[0].rgb, (255, 0, 0));
        assert_eq!(p.swatches[1].name, "Green");
    }

    #[test]
    fn aco_v1_rgb() {
        // version=1, count=1, space=0 (RGB), R=65535 G=0 B=0 (+unused Z)
        let bytes = [0, 1, 0, 1, 0, 0, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0];
        let p = parse_aco(&bytes).unwrap();
        assert_eq!(p.swatches.len(), 1);
        assert_eq!(p.swatches[0].rgb, (255, 0, 0));
    }

    #[test]
    fn ase_rgb_with_name() {
        // "ASEF", version 1.0, 1 block; color block: name "Hi\0" (3 u16), "RGB ", 0,0.5,1
        let mut b = Vec::new();
        b.extend_from_slice(b"ASEF");
        b.extend_from_slice(&[0, 1, 0, 0]); // version 1.0
        b.extend_from_slice(&1u32.to_be_bytes()); // 1 block
        b.extend_from_slice(&0x0001u16.to_be_bytes()); // color block
        let mut body = Vec::new();
        body.extend_from_slice(&3u16.to_be_bytes()); // name len (incl null)
        for u in [b'H' as u16, b'i' as u16, 0u16] {
            body.extend_from_slice(&u.to_be_bytes());
        }
        body.extend_from_slice(b"RGB ");
        body.extend_from_slice(&0.0f32.to_be_bytes());
        body.extend_from_slice(&0.5f32.to_be_bytes());
        body.extend_from_slice(&1.0f32.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes()); // color type
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(&body);

        let p = parse_ase(&b).unwrap();
        assert_eq!(p.swatches.len(), 1);
        assert_eq!(p.swatches[0].name, "Hi");
        assert_eq!(p.swatches[0].rgb, (0, 128, 255));
    }

    #[test]
    fn kpl_parses_colorset_xml() {
        let xml = r#"<ColorSet version="1.0" name="T" columns="2">
            <ColorSetEntry name="Red"><RGB r="1" g="0" b="0"/></ColorSetEntry>
            <ColorSetEntry name="Half"><sRGB r="0.5" g="0.5" b="0.5"/></ColorSetEntry>
        </ColorSet>"#;
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zw.start_file::<_, ()>("colorset.xml", zip::write::SimpleFileOptions::default())
                .unwrap();
            use std::io::Write;
            zw.write_all(xml.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let p = parse_kpl(&buf).unwrap();
        assert_eq!(p.columns, 2);
        assert_eq!(p.swatches.len(), 2);
        assert_eq!(p.swatches[0].rgb, (255, 0, 0));
        assert_eq!(p.swatches[1].rgb, (128, 128, 128));
    }

    #[test]
    fn diff_classifies_changes() {
        let old = Palette {
            columns: 4,
            swatches: vec![
                Swatch {
                    name: "A".into(),
                    rgb: (255, 0, 0),
                },
                Swatch {
                    name: "B".into(),
                    rgb: (0, 255, 0),
                },
                Swatch {
                    name: "Gone".into(),
                    rgb: (1, 1, 1),
                },
            ],
        };
        let new = Palette {
            columns: 4,
            swatches: vec![
                Swatch {
                    name: "A".into(),
                    rgb: (255, 0, 0),
                }, // unchanged
                Swatch {
                    name: "B".into(),
                    rgb: (0, 0, 255),
                }, // modified
                Swatch {
                    name: "New".into(),
                    rgb: (9, 9, 9),
                }, // added
            ],
        };
        let d = diff(Some(&old), Some(&new));
        let by = |n: &str| d.swatches.iter().find(|s| s.name == n).unwrap();
        assert_eq!(by("A").change, "unchanged");
        assert_eq!(by("B").change, "modified");
        assert_eq!(by("B").before.as_deref(), Some("#00FF00"));
        assert_eq!(by("B").after.as_deref(), Some("#0000FF"));
        assert_eq!(by("New").change, "added");
        assert_eq!(by("Gone").change, "removed");
    }
}
