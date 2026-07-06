//! Decode Krita layer tiles into pixel rasters for the visual diff viewer.
//!
//! A tiled layer entry stores each 64x64 tile as `x,y,LZF,len` + `len` bytes. Those bytes are
//! `[1-byte compression flag][payload]`: flag `1` = liblzf-compressed, `0` = raw. The decoded
//! tile is `TILEWIDTH*TILEHEIGHT*PIXELSIZE` bytes, stored **planar** (one full channel plane
//! after another) in the colorspace's native channel order.
//!
//! ponytail: only RGBA 8-bit (PIXELSIZE 4, planar B,G,R,A) is decoded — the overwhelmingly
//! common Krita paint layer. Anything else returns `None` and the UI falls back to the
//! composite (mergedimage.png). Upgrade path: branch on the layer's colorspace/depth here.

use crate::error::{KvcError, Result};
use std::io::Cursor;

// Planar channel order for Krita's RGBA8 colorspace (in-memory BGRA). This is the one knob to
// tune if colors look swapped against a real .kra.
const PLANE_B: usize = 0;
const PLANE_G: usize = 1;
const PLANE_R: usize = 2;
const PLANE_A: usize = 3;

/// Decompress a liblzf stream into `expected` bytes. Returns `None` on any malformed input.
pub fn lzf_decompress(data: &[u8], expected: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut ip = 0usize;
    while ip < data.len() {
        let ctrl = data[ip] as usize;
        ip += 1;
        if ctrl < 32 {
            // Literal run of ctrl+1 bytes.
            let len = ctrl + 1;
            let end = ip.checked_add(len)?;
            if end > data.len() {
                return None;
            }
            out.extend_from_slice(&data[ip..end]);
            ip = end;
        } else {
            // Back-reference: length in the top 3 bits, offset in the low 5 bits + next byte.
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *data.get(ip)? as usize;
                ip += 1;
            }
            let off = ((ctrl & 0x1f) << 8) | *data.get(ip)? as usize;
            ip += 1;
            let mut r = out.len().checked_sub(off + 1)?;
            for _ in 0..len + 2 {
                let b = *out.get(r)?;
                out.push(b);
                r += 1;
            }
        }
    }
    Some(out)
}

/// Compress with liblzf's format (greedy hash-table matcher). Round-trips through
/// [`lzf_decompress`]; byte-parity with Krita's own encoder is *not* required — any valid
/// LZF stream decodes identically, and Krita reads whatever the flag byte says.
/// ponytail: ~50-line port of `lzf_c` — no crate carries this format reliably.
pub fn lzf_compress(data: &[u8]) -> Vec<u8> {
    const MAX_OFF: usize = 1 << 13; // 13-bit offset field
    const MAX_REF: usize = (1 << 8) + (1 << 3) - 1; // 264: 3-bit len code + extension byte
    const MAX_LIT: usize = 32;
    const HSIZE: usize = 1 << 13;

    fn flush_lit(out: &mut Vec<u8>, data: &[u8], from: usize, to: usize) {
        let mut s = from;
        while s < to {
            let n = (to - s).min(MAX_LIT);
            out.push((n - 1) as u8); // ctrl < 32 => literal run of n bytes
            out.extend_from_slice(&data[s..s + n]);
            s += n;
        }
    }

    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    let mut table = vec![usize::MAX; HSIZE];
    let hash = |i: usize| -> usize {
        let v = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
        (v.wrapping_mul(0x9E37_79B1) >> 19) as usize & (HSIZE - 1)
    };
    let (mut ip, mut lit_start) = (0usize, 0usize);
    while ip + 2 < data.len() {
        let h = hash(ip);
        let rp = table[h];
        table[h] = ip;
        if rp != usize::MAX
            && rp < ip
            && ip - rp <= MAX_OFF
            && data[rp] == data[ip]
            && data[rp + 1] == data[ip + 1]
            && data[rp + 2] == data[ip + 2]
        {
            let max = (data.len() - ip).min(MAX_REF);
            let mut len = 3;
            while len < max && data[rp + len] == data[ip + len] {
                len += 1;
            }
            flush_lit(&mut out, data, lit_start, ip);
            let off = ip - rp - 1;
            let lc = len - 2;
            if lc < 7 {
                out.push(((lc as u8) << 5) | (off >> 8) as u8);
            } else {
                out.push((7u8 << 5) | (off >> 8) as u8);
                out.push((lc - 7) as u8);
            }
            out.push((off & 0xff) as u8);
            ip += len;
            lit_start = ip;
        } else {
            ip += 1;
        }
    }
    flush_lit(&mut out, data, lit_start, data.len());
    out
}

/// Decode a tile's stored bytes (`[flag][payload]`) into its planar pixel buffer of exactly
/// `expected` bytes. `None` on malformed input or a length mismatch.
pub fn tile_planar(stored: &[u8], expected: usize) -> Option<Vec<u8>> {
    let (&flag, payload) = stored.split_first()?;
    let planar = match flag {
        0 => payload.to_vec(),
        1 => lzf_decompress(payload, expected)?,
        _ => return None,
    };
    (planar.len() == expected).then_some(planar)
}

/// Re-encode planar pixels as a stored tile payload, mirroring Krita's own rule: LZF when it
/// actually shrinks, raw otherwise (the flag byte tells the reader which).
pub fn tile_from_planar(planar: &[u8]) -> Vec<u8> {
    let c = lzf_compress(planar);
    let (flag, body): (u8, &[u8]) = if c.len() < planar.len() {
        (1, &c)
    } else {
        (0, planar)
    };
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(flag);
    out.extend_from_slice(body);
    out
}

/// Interleave a planar RGBA8 buffer (`tw*th*4`, planes B,G,R,A) into RGBA pixels.
/// `None` on a length mismatch.
pub fn planar_to_rgba(planar: &[u8], tw: usize, th: usize) -> Option<Vec<u8>> {
    let n = tw * th;
    if planar.len() != n * 4 {
        return None;
    }
    let mut rgba = vec![0u8; n * 4];
    for i in 0..n {
        rgba[i * 4] = planar[PLANE_R * n + i];
        rgba[i * 4 + 1] = planar[PLANE_G * n + i];
        rgba[i * 4 + 2] = planar[PLANE_B * n + i];
        rgba[i * 4 + 3] = planar[PLANE_A * n + i];
    }
    Some(rgba)
}

/// Decode one tile's stored bytes into interleaved RGBA (`tw*th*4`). `None` if unsupported.
pub fn tile_to_rgba(stored: &[u8], tw: usize, th: usize, pixelsize: usize) -> Option<Vec<u8>> {
    if pixelsize != 4 {
        return None; // ponytail: RGBA8 only.
    }
    let planar = tile_planar(stored, tw * th * pixelsize)?;
    planar_to_rgba(&planar, tw, th)
}

/// Copy a `tw*th` RGBA tile into a `iw*ih` RGBA canvas at pixel offset `(tx, ty)`, clipping to
/// the canvas bounds (tiles can extend past the image edge, or sit at negative offsets).
pub fn blit(canvas: &mut [u8], iw: i64, ih: i64, tx: i64, ty: i64, tile: &[u8], tw: i64, th: i64) {
    let full_rows = tx >= 0 && tx + tw <= iw;
    for row in 0..th {
        let cy = ty + row;
        if cy < 0 || cy >= ih {
            continue;
        }
        // Common case: the whole tile row is inside the canvas — one memcpy per row.
        if full_rows {
            let src = (row * tw * 4) as usize;
            let dst = ((cy * iw + tx) * 4) as usize;
            let len = (tw * 4) as usize;
            canvas[dst..dst + len].copy_from_slice(&tile[src..src + len]);
            continue;
        }
        for col in 0..tw {
            let cx = tx + col;
            if cx < 0 || cx >= iw {
                continue;
            }
            let src = ((row * tw + col) * 4) as usize;
            let dst = ((cy * iw + cx) * 4) as usize;
            canvas[dst..dst + 4].copy_from_slice(&tile[src..src + 4]);
        }
    }
}

/// Longest-side cap for per-layer diff rasters. These are only ever shown scaled-to-fit in a
/// preview pane or a ~30px thumbnail, so full document resolution (Krita canvases run into the
/// thousands of px) is wasted — it bloats PNG-encode time and the base64 payload shipped to the
/// webview. Downscale uses an area-average box filter (see [`box_downscale`]) so a scaled diff
/// stays crisp under moderate zoom instead of aliasing. The cap value is baked into the raster
/// cache keys, and the `box1` token there covers this filter — bump the token if it changes.
pub const MAX_RASTER_DIM: u32 = 2048;

/// Area-average (box filter) downscale of an RGBA8 buffer from `(w,h)` to `(nw,nh)`
/// (`nw<=w`, `nh<=h`). Each destination pixel averages the source pixels in its covered box,
/// **in premultiplied-alpha space** so fully/partly transparent source pixels don't bleed their
/// (often black) RGB into the edges; the result is un-premultiplied back to straight alpha.
/// Sharper and alias-free versus nearest-neighbour, at one extra pass over the source.
fn box_downscale(rgba: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    use rayon::prelude::*;
    let mut out = vec![0u8; nw * nh * 4];
    // Parallel over destination rows (each row reads a disjoint source band), integer
    // accumulation — same area-average premultiplied semantics as the old f64 body
    // (the `count` term cancels: color = Σ(c·a)/Σa, alpha = Σa/count), so the `box1`
    // cache token stays valid; only ±1 rounding at exact ties can differ.
    out.par_chunks_mut(nw * 4)
        .enumerate()
        .for_each(|(y, out_row)| {
            let sy0 = y * h / nh;
            let sy1 = (((y + 1) * h) / nh).max(sy0 + 1).min(h);
            for x in 0..nw {
                let sx0 = x * w / nw;
                let sx1 = (((x + 1) * w) / nw).max(sx0 + 1).min(w);
                let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
                for sy in sy0..sy1 {
                    let row = sy * w;
                    for sx in sx0..sx1 {
                        let i = (row + sx) * 4;
                        let af = rgba[i + 3] as u64;
                        r += rgba[i] as u64 * af;
                        g += rgba[i + 1] as u64 * af;
                        b += rgba[i + 2] as u64 * af;
                        a += af;
                    }
                }
                let count = ((sy1 - sy0) * (sx1 - sx0)) as u64;
                let dst = x * 4;
                // Half-up rounding: (2·num + den) / (2·den).
                out_row[dst + 3] = ((2 * a + count) / (2 * count)).min(255) as u8;
                if a > 0 {
                    // Un-premultiply: weighted mean color Σ(c·a)/Σa, straight alpha.
                    out_row[dst] = ((2 * r + a) / (2 * a)).min(255) as u8;
                    out_row[dst + 1] = ((2 * g + a) / (2 * a)).min(255) as u8;
                    out_row[dst + 2] = ((2 * b + a) / (2 * a)).min(255) as u8;
                }
                // Fully transparent → leave RGB at 0.
            }
        });
    out
}

/// Downscale an RGBA8 buffer so its longest side is at most `MAX_RASTER_DIM`, returning the new
/// buffer and dimensions. Returns the input unchanged when it's already within the cap.
pub fn cap_rgba(rgba: &[u8], width: u32, height: u32) -> (std::borrow::Cow<'_, [u8]>, u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_RASTER_DIM || width == 0 || height == 0 {
        return (std::borrow::Cow::Borrowed(rgba), width, height);
    }
    let scale = MAX_RASTER_DIM as f64 / longest as f64;
    let nw = ((width as f64 * scale).round() as u32).max(1);
    let nh = ((height as f64 * scale).round() as u32).max(1);
    let out = box_downscale(
        rgba,
        width as usize,
        height as usize,
        nw as usize,
        nh as usize,
    );
    (std::borrow::Cow::Owned(out), nw, nh)
}

/// Decode a PNG into a straight-alpha RGBA8 buffer + dimensions. Returns `None` for anything we
/// don't handle (16-bit, palette, grayscale, malformed) — callers treat that as "skip", never
/// an error. RGB is expanded to opaque RGBA. Shared by [`diff_mask_png`].
fn decode_png_rgba(png_bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder.read_info().ok()?;
    let (w, h, color, depth) = {
        let info = reader.info();
        (info.width, info.height, info.color_type, info.bit_depth)
    };
    if w == 0
        || h == 0
        || depth != png::BitDepth::Eight
        || !matches!(color, png::ColorType::Rgba | png::ColorType::Rgb)
    {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    buf.truncate(frame.buffer_size());
    let rgba = match color {
        png::ColorType::Rgba => buf,
        _ => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
    };
    Some((rgba, w, h))
}

/// Decode a PNG into raw 8-bit interleaved pixels when it's a *plain* RGB/RGBA image with no
/// ICC profile: `(pixels, width, height, has_alpha, srgb rendering intent)`. `None` = not
/// eligible (16-bit, palette, grayscale, ICC-managed, malformed) — callers keep the original
/// bytes instead. This is the eligibility gate for composite tiling: re-encoding must not
/// change how the pixels are interpreted, so anything carrying color-management state beyond
/// an sRGB chunk (which is recorded and re-emitted) is left alone.
pub fn decode_png_plain(png_bytes: &[u8]) -> Option<(Vec<u8>, u32, u32, bool, Option<u8>)> {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder.read_info().ok()?;
    let (w, h, color, depth, srgb, has_icc) = {
        let info = reader.info();
        (
            info.width,
            info.height,
            info.color_type,
            info.bit_depth,
            info.srgb.map(|i| i as u8),
            info.icc_profile.is_some(),
        )
    };
    if w == 0
        || h == 0
        || depth != png::BitDepth::Eight
        || has_icc
        || !matches!(color, png::ColorType::Rgba | png::ColorType::Rgb)
    {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    buf.truncate(frame.buffer_size());
    Some((buf, w, h, color == png::ColorType::Rgba, srgb))
}

/// Encode raw interleaved pixels back into a PNG for a restored composite. Fixed, fast,
/// deterministic settings (restore hashing depends on determinism); adaptive filtering buys
/// a meaningfully smaller file than `rgba_to_png`'s none — this PNG persists inside the
/// working `.kra` (Stored zip entry, so PNG-level compression is all it gets).
pub fn encode_composite_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    srgb: Option<u8>,
) -> Result<Vec<u8>> {
    use png::SrgbRenderingIntent as Intent;
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(Cursor::new(&mut out), width, height);
        enc.set_color(if has_alpha {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        if let Some(i) = srgb {
            enc.set_source_srgb(match i {
                1 => Intent::RelativeColorimetric,
                2 => Intent::Saturation,
                3 => Intent::AbsoluteColorimetric,
                _ => Intent::Perceptual,
            });
        }
        let mut w = enc
            .write_header()
            .map_err(|e| KvcError::BadTiles(format!("png header: {e}")))?;
        w.write_image_data(pixels)
            .map_err(|e| KvcError::BadTiles(format!("png data: {e}")))?;
    }
    Ok(out)
}

/// Accent color of the change highlight — mirrors `ACCENT` in `ArtCanvas.tsx`.
const HIGHLIGHT_RGBA: [u8; 4] = [0xE0, 0x7B, 0x39, 128];
/// Per-channel delta (0..255) above which a pixel counts as changed. Small enough to catch real
/// edits, large enough to ignore encode/rounding noise.
const DIFF_THRESHOLD: i32 = 16;
/// Longest side of the coarse grid the change outline is traced on. Bounds the traced path's size
/// (and its on-screen blockiness) independent of canvas resolution.
const OUTLINE_GRID_MAX: usize = 200;
/// Bail out of tracing (no outline) past this many boundary edges — a heavily speckled diff would
/// otherwise emit a huge path for little visual benefit; the tint + hatch still convey the change.
const OUTLINE_EDGE_CAP: usize = 8000;

/// Per-pixel "changed" grid of the after-composite (index-sampling the before when dims differ).
/// `true` = the two composites differ at that pixel. Shared by the mask raster and the outline.
fn changed_grid(before_png: &[u8], after_png: &[u8]) -> Option<(Vec<bool>, usize, usize)> {
    let (before, bw, bh) = decode_png_rgba(before_png)?;
    let (after, aw, ah) = decode_png_rgba(after_png)?;
    let (w, h) = (aw as usize, ah as usize);
    let (bwu, bhu) = (bw as usize, bh as usize);
    let same_dims = aw == bw && ah == bh;
    let mut grid = vec![false; w * h];
    for y in 0..h {
        let by = if same_dims {
            y
        } else {
            (y * bhu / h).min(bhu - 1)
        };
        for x in 0..w {
            let ai = (y * w + x) * 4;
            let bx = if same_dims {
                x
            } else {
                (x * bwu / w).min(bwu - 1)
            };
            let bi = (by * bwu + bx) * 4;
            grid[y * w + x] = (0..4)
                .any(|c| (after[ai + c] as i32 - before[bi + c] as i32).abs() > DIFF_THRESHOLD);
        }
    }
    Some((grid, w, h))
}

/// Accent-tinted PNG (transparent elsewhere) from a changed grid, capped to `MAX_RASTER_DIM`.
fn mask_png_from_grid(grid: &[bool], w: usize, h: usize) -> Option<Vec<u8>> {
    let mut mask = vec![0u8; w * h * 4];
    for (i, &c) in grid.iter().enumerate() {
        if c {
            mask[i * 4..i * 4 + 4].copy_from_slice(&HIGHLIGHT_RGBA);
        }
    }
    let (capped, cw, ch) = cap_rgba(&mask, w as u32, h as u32);
    rgba_to_png(&capped, cw, ch).ok()
}

/// Trace the boundary between changed and unchanged pixels into SVG path data, in a **normalized
/// 0..1** space (the frontend scales it to the document box). This hugs the changed pixels'
/// silhouette — not a bounding box — so a dashed stroke over it outlines exactly what changed.
/// The grid is first downsampled to `OUTLINE_GRID_MAX` so the path stays small and dash-friendly
/// (a blocky-but-faithful outline). `None` if there's nothing to outline or it's too speckled.
fn outline_from_grid(grid: &[bool], w: usize, h: usize) -> Option<String> {
    use std::collections::HashMap;
    if w == 0 || h == 0 {
        return None;
    }
    // Downsample: a coarse cell is "changed" if any pixel it covers changed.
    let scale = OUTLINE_GRID_MAX as f64 / w.max(h) as f64;
    let gw = ((w as f64 * scale).round() as usize).clamp(1, w);
    let gh = ((h as f64 * scale).round() as usize).clamp(1, h);
    let mut cells = vec![false; gw * gh];
    for y in 0..h {
        let gy = (y * gh / h).min(gh - 1);
        for x in 0..w {
            if grid[y * w + x] {
                let gx = (x * gw / w).min(gw - 1);
                cells[gy * gw + gx] = true;
            }
        }
    }
    let on = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < gw
            && (y as usize) < gh
            && cells[y as usize * gw + x as usize]
    };
    // Lattice vertex (gw+1)×(gh+1) → packed key. Each changed cell contributes the edges it shares
    // with a non-changed neighbour (or the border); those edges form closed loops around the set.
    let vk = |x: usize, y: usize| (y * (gw + 1) + x) as u32;
    let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
    fn add_edge(adj: &mut HashMap<u32, Vec<u32>>, a: u32, b: u32) {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    let mut edges = 0usize;
    for cy in 0..gh {
        for cx in 0..gw {
            if !cells[cy * gw + cx] {
                continue;
            }
            let (x, y) = (cx, cy);
            if !on(cx as i64, cy as i64 - 1) {
                add_edge(&mut adj, vk(x, y), vk(x + 1, y));
                edges += 1;
            }
            if !on(cx as i64, cy as i64 + 1) {
                add_edge(&mut adj, vk(x, y + 1), vk(x + 1, y + 1));
                edges += 1;
            }
            if !on(cx as i64 - 1, cy as i64) {
                add_edge(&mut adj, vk(x, y), vk(x, y + 1));
                edges += 1;
            }
            if !on(cx as i64 + 1, cy as i64) {
                add_edge(&mut adj, vk(x + 1, y), vk(x + 1, y + 1));
                edges += 1;
            }
            if edges > OUTLINE_EDGE_CAP {
                return None;
            }
        }
    }
    if edges == 0 {
        return None;
    }
    let coord = |k: u32| -> (usize, usize) {
        let k = k as usize;
        (k % (gw + 1), k / (gw + 1))
    };
    fn take_edge(adj: &mut HashMap<u32, Vec<u32>>, a: u32, b: u32) {
        if let Some(v) = adj.get_mut(&a) {
            if let Some(p) = v.iter().position(|&x| x == b) {
                v.swap_remove(p);
            }
        }
    }
    // Walk each closed loop (every lattice vertex has even degree, so a walk returns to its start).
    let mut starts: Vec<u32> = adj.keys().copied().collect();
    starts.sort_unstable();
    let mut d = String::new();
    let emit = |d: &mut String, cmd: char, k: u32| {
        let (x, y) = coord(k);
        d.push(cmd);
        d.push_str(&format!(
            "{:.4} {:.4}",
            x as f64 / gw as f64,
            y as f64 / gh as f64
        ));
    };
    for s in starts {
        while adj.get(&s).is_some_and(|v| !v.is_empty()) {
            emit(&mut d, 'M', s);
            let mut cur = s;
            while let Some(next) = adj.get_mut(&cur).and_then(|v| v.pop()) {
                take_edge(&mut adj, next, cur);
                emit(&mut d, 'L', next);
                cur = next;
                if cur == s {
                    break;
                }
            }
            d.push('Z');
        }
    }
    (!d.is_empty()).then_some(d)
}

/// A transparent PNG that is opaque (accent-tinted) only where the two composites differ. Both
/// PNGs are decoded to RGBA and compared pixel-for-pixel; a size mismatch index-samples the
/// `before` into the `after`'s grid. Capped to `MAX_RASTER_DIM` like the composites. `None` if
/// either side can't be decoded (highlight simply absent — never fatal).
pub fn diff_mask_png(before_png: &[u8], after_png: &[u8]) -> Option<Vec<u8>> {
    let (grid, w, h) = changed_grid(before_png, after_png)?;
    mask_png_from_grid(&grid, w, h)
}

/// Both halves of the changed-pixel highlight from one decode: the accent mask PNG and the SVG
/// path outlining the changed pixels (normalized 0..1; `None` if not outline-able).
pub fn diff_overlay(before_png: &[u8], after_png: &[u8]) -> Option<(Vec<u8>, Option<String>)> {
    let (grid, w, h) = changed_grid(before_png, after_png)?;
    let png = mask_png_from_grid(&grid, w, h)?;
    let outline = outline_from_grid(&grid, w, h);
    Some((png, outline))
}

/// Rebuild the change outline from an already-cached mask PNG (alpha > 0 = changed), so a cache
/// hit gets the outline without re-reading the source composites. Normalized 0..1 path data.
pub fn outline_from_mask_png(mask_png: &[u8]) -> Option<String> {
    let (rgba, w, h) = decode_png_rgba(mask_png)?;
    let (w, h) = (w as usize, h as usize);
    let grid: Vec<bool> = (0..w * h).map(|i| rgba[i * 4 + 3] > 0).collect();
    outline_from_grid(&grid, w, h)
}

/// Encode an RGBA8 buffer as PNG bytes.
pub fn rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(Cursor::new(&mut png), width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // ponytail: these rasters are transient data-URLs for the webview — encode speed matters,
        // byte size doesn't. Fast deflate + no row filter over max compression.
        enc.set_compression(png::Compression::Fast);
        enc.set_filter(png::FilterType::NoFilter);
        let mut w = enc
            .write_header()
            .map_err(|e| KvcError::BadTiles(format!("png header: {e}")))?;
        w.write_image_data(rgba)
            .map_err(|e| KvcError::BadTiles(format!("png data: {e}")))?;
    }
    Ok(png)
}

/// Encode an RGBA8 buffer as a PNG and wrap it in a `data:` URL.
pub fn rgba_to_png_data_url(rgba: &[u8], width: u32, height: u32) -> Result<String> {
    Ok(png_bytes_to_data_url(&rgba_to_png(rgba, width, height)?))
}

/// Re-encode an already-encoded PNG (e.g. mergedimage.png) so its longest side fits
/// `MAX_RASTER_DIM`. Full-resolution composites of large canvases dominated the diff's IPC
/// payload — the webview only ever shows them scaled to fit. Returns the input untouched when
/// it's already small enough or on anything we can't decode (16-bit, palette, grayscale,
/// malformed) — shipping the original is always a safe fallback.
pub fn cap_png(png_bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let Ok(mut reader) = decoder.read_info() else {
        return std::borrow::Cow::Borrowed(png_bytes);
    };
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let (color, depth) = (info.color_type, info.bit_depth);
    if w.max(h) <= MAX_RASTER_DIM
        || depth != png::BitDepth::Eight
        || !matches!(color, png::ColorType::Rgba | png::ColorType::Rgb)
    {
        return std::borrow::Cow::Borrowed(png_bytes);
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let Ok(frame) = reader.next_frame(&mut buf) else {
        return std::borrow::Cow::Borrowed(png_bytes);
    };
    buf.truncate(frame.buffer_size());
    let rgba: Vec<u8> = match color {
        png::ColorType::Rgba => buf,
        _ => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
    };
    let (capped, cw, ch) = cap_rgba(&rgba, w, h);
    match rgba_to_png(&capped, cw, ch) {
        Ok(png) => std::borrow::Cow::Owned(png),
        Err(_) => std::borrow::Cow::Borrowed(png_bytes),
    }
}

// --- persistent raster cache (.kvc/cache/) ----------------------------------------------
// Final capped PNGs keyed by a content hash of everything that produced them, so entries are
// immutable and never need invalidation. All helpers are best-effort: a cold or unwritable
// cache must never fail a diff — and a pruned entry is just a regeneration, never an error.

/// Versions the downscale filter baked into every cache key (see `kra::raster_cache_key` and
/// friends). Bump it if resampling semantics change so stale entries are never served — and
/// so GC can wipe a cache whose `.filter-version` marker no longer matches (the token is
/// hashed *into* each key, so per-entry staleness can't be recovered from filenames).
pub const FILTER_VERSION: &str = "box1";

const FILTER_MARKER: &str = ".filter-version";

/// Whether the cache's recorded filter version mismatches the current one. A missing marker
/// counts as current (pre-marker caches are warm and valid — don't wipe them on upgrade);
/// `cache_sync_filter_version` writes the marker so future bumps are detected.
pub fn cache_filter_stale(cache_dir: &std::path::Path) -> bool {
    match std::fs::read_to_string(cache_dir.join(FILTER_MARKER)) {
        Ok(v) => v.trim() != FILTER_VERSION,
        Err(_) => false,
    }
}

/// If the marker mismatches, delete every cached PNG (all keyed to the old filter — dead
/// weight LRU would otherwise retain indefinitely); always (re)write the marker. Returns
/// bytes deleted. Best-effort like the rest of the cache.
pub fn cache_sync_filter_version(cache_dir: &std::path::Path) -> u64 {
    let mut deleted = 0u64;
    if cache_filter_stale(cache_dir) {
        if let Ok(rd) = std::fs::read_dir(cache_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "png") {
                    let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                    if std::fs::remove_file(&p).is_ok() {
                        deleted += len;
                    }
                }
            }
        }
    }
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(cache_dir.join(FILTER_MARKER), FILTER_VERSION);
    deleted
}

/// Total size of the cached PNGs (the prunable payload; markers excluded).
pub fn cache_total_bytes(cache_dir: &std::path::Path) -> u64 {
    std::fs::read_dir(cache_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Read a cached capped PNG, or `None` on miss/any error. A hit refreshes the file's mtime so
/// LRU pruning treats recently-served entries as hot.
pub fn cache_read(cache_dir: &std::path::Path, key: &str) -> Option<Vec<u8>> {
    let path = cache_dir.join(format!("{key}.png"));
    let bytes = std::fs::read(&path).ok()?;
    touch(&path);
    Some(bytes)
}

/// Write a capped PNG into the cache (creating the dir for pre-cache repos).
pub fn cache_write(cache_dir: &std::path::Path, key: &str, png: &[u8]) {
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(cache_dir.join(format!("{key}.png")), png);
}

/// Best-effort mtime refresh (LRU signal). Failure is fine — the entry just ages normally.
fn touch(path: &std::path::Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Delete the oldest cache PNGs (by mtime) until the cache fits `max_bytes`. Keeps the cache
/// bounded — before this it grew for the life of the repo, one capped PNG per unique layer
/// state ever viewed, which for large canvases is the biggest `.kvc/` append-only cost.
/// Returns the number of bytes deleted.
pub fn cache_prune(cache_dir: &std::path::Path, max_bytes: u64) -> u64 {
    let Ok(rd) = std::fs::read_dir(cache_dir) else {
        return 0;
    };
    let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime, u64)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((e.path(), m.modified().ok()?, m.len()))
        })
        .collect();
    let mut total: u64 = entries.iter().map(|(_, _, len)| len).sum();
    if total <= max_bytes {
        return 0;
    }
    entries.sort_by_key(|(_, mtime, _)| *mtime);
    let mut deleted = 0;
    for (path, _, len) in entries {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            deleted += len;
        }
    }
    deleted
}

/// How often a prune is allowed to actually scan the cache dir.
const PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// [`cache_prune`] rate-limited via a marker file's mtime, so the post-stream trigger doesn't
/// re-stat the whole cache dir on every diff view.
pub fn cache_prune_throttled(cache_dir: &std::path::Path, max_bytes: u64) {
    let marker = cache_dir.join(".last-prune");
    if let Ok(m) = std::fs::metadata(&marker) {
        let recent = m
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|e| e < PRUNE_INTERVAL);
        if recent {
            return;
        }
    }
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(&marker, b"");
    cache_prune(cache_dir, max_bytes);
}

/// Wrap already-encoded PNG bytes (e.g. mergedimage.png) in a `data:` URL.
pub fn png_bytes_to_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64(png))
}

// --- raster delivery ---------------------------------------------------------------------
// With the desktop shell's `kvcimg` URI scheme registered (lib.rs), cached rasters are served
// as plain PNG URLs the webview fetches directly: no base64 re-encode per view, no multi-MB
// strings over IPC, and the browser cache handles repeat views (keys are content-addressed and
// immutable). Outside the shell (tests, cache-write failure) everything falls back to the
// data-URL path, which is always correct.

static IMG_PROTOCOL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called once from `lib.rs` after registering the `kvcimg` URI scheme.
pub fn enable_img_protocol() {
    IMG_PROTOCOL.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn img_protocol_enabled() -> bool {
    IMG_PROTOCOL.load(std::sync::atomic::Ordering::SeqCst)
}

/// URL for a cached raster PNG: a `kvcimg` URL when the scheme is live and the cache file is
/// really on disk (the handler serves exactly that file), else an inline data URL.
/// The repo root rides in the URL hex-encoded; the handler only serves roots that commands
/// have registered (`commands::register_served_repo`), so the scheme can't read arbitrary paths.
pub fn raster_url(
    root: &std::path::Path,
    cache_dir: &std::path::Path,
    key: &str,
    png: &[u8],
) -> String {
    if img_protocol_enabled() && cache_dir.join(format!("{key}.png")).is_file() {
        let root_hex = hex(root.to_string_lossy().as_bytes());
        // WebView2 maps custom schemes to http://<scheme>.localhost/; WebKit/GTK keep the
        // scheme itself. Build the final URL here so the frontend stays platform-agnostic.
        #[cfg(windows)]
        return format!("http://kvcimg.localhost/{root_hex}/{key}.png");
        #[cfg(not(windows))]
        return format!("kvcimg://localhost/{root_hex}/{key}.png");
    }
    png_bytes_to_data_url(png)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Minimal standard base64 encoder (no padding omitted). ponytail: ~15 lines beats a dep.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference liblzf compressor (literals-only is a valid LZF stream) so the decoder has a
    /// round-trip check without a real .kra. Emits literal runs of up to 32 bytes.
    fn lzf_literals(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in data.chunks(32) {
            out.push((chunk.len() - 1) as u8); // ctrl < 32 => literal run of len bytes
            out.extend_from_slice(chunk);
        }
        out
    }

    #[test]
    fn lzf_literal_roundtrip() {
        let data: Vec<u8> = (0..200u32).map(|i| (i * 7 % 256) as u8).collect();
        let comp = lzf_literals(&data);
        assert_eq!(lzf_decompress(&comp, data.len()).unwrap(), data);
    }

    #[test]
    fn lzf_backreference() {
        // "abcabcabc": literal "abc" then a back-ref (offset 2, len 6) copying overlapping bytes.
        let mut comp = vec![2u8, b'a', b'b', b'c'];
        // ctrl: len(6)-2=4 in top3 bits => 4<<5=128; offset 2 => low5=0, next byte=2.
        comp.push((4 << 5) | 0);
        comp.push(2);
        assert_eq!(lzf_decompress(&comp, 9).unwrap(), b"abcabcabc");
    }

    #[test]
    fn lzf_compress_roundtrips_through_decoder() {
        let mut seed = 0xC0FFEEu64;
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
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![7],
            vec![1, 2],
            vec![0u8; 20_000], // constant: long matches
            (0..20_000u32).map(|i| (i / 64) as u8).collect(), // gradient
            rng(20_000),       // incompressible
            b"abcabcabcabcabcabc".to_vec(), // overlapping back-refs
        ];
        for data in cases {
            let comp = lzf_compress(&data);
            assert_eq!(
                lzf_decompress(&comp, data.len()).unwrap_or_default(),
                data,
                "round-trip failed for a {}-byte buffer",
                data.len()
            );
            // Constant data must actually compress (sanity that matching works at all).
            if data.len() >= 1000 && data.iter().all(|&b| b == data[0]) {
                assert!(comp.len() < data.len() / 4);
            }
        }
    }

    #[test]
    fn tile_from_planar_roundtrip() {
        let planar: Vec<u8> = (0..64 * 64 * 4u32).map(|i| (i % 251) as u8).collect();
        let stored = tile_from_planar(&planar);
        assert_eq!(tile_planar(&stored, planar.len()).unwrap(), planar);
    }

    #[test]
    fn tile_decode_bgra_to_rgba() {
        // 2x1 tile (pixelsize 4), planar B,G,R,A. Pixel0 = (R=10,G=20,B=30,A=40).
        let tw = 2;
        let th = 1;
        let n = tw * th;
        let mut planar = vec![0u8; n * 4];
        // B plane
        planar[PLANE_B * n] = 30;
        planar[PLANE_G * n] = 20;
        planar[PLANE_R * n] = 10;
        planar[PLANE_A * n] = 40;
        let stored = {
            let mut s = vec![0u8]; // flag 0 = raw
            s.extend_from_slice(&planar);
            s
        };
        let rgba = tile_to_rgba(&stored, tw, th, 4).unwrap();
        assert_eq!(&rgba[0..4], &[10, 20, 30, 40]);
    }

    #[test]
    fn png_encode_is_decodable() {
        let rgba = vec![255u8; 4 * 4 * 4]; // 4x4 opaque white
        let url = rgba_to_png_data_url(&rgba, 4, 4).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn cap_png_shrinks_oversized() {
        let w = MAX_RASTER_DIM + 100;
        let rgba = vec![128u8; w as usize * 4]; // w x 1 strip
        let png = rgba_to_png(&rgba, w, 1).unwrap();
        let capped = cap_png(&png);
        assert!(matches!(capped, std::borrow::Cow::Owned(_)));
        let reader = png::Decoder::new(Cursor::new(&capped[..]))
            .read_info()
            .unwrap();
        let info = reader.info();
        assert_eq!(info.width.max(info.height), MAX_RASTER_DIM);
    }

    #[test]
    fn cache_prune_deletes_oldest_until_under_budget() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        // 5 entries x 100 bytes with strictly increasing mtimes.
        let base = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for i in 0..5u32 {
            let p = cache.join(format!("entry{i}.png"));
            std::fs::write(&p, [0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(base + std::time::Duration::from_secs(i as u64 * 60))
                .unwrap();
        }
        // Budget for 2 entries -> the 3 oldest go, the 2 newest survive.
        let deleted = cache_prune(cache, 250);
        assert_eq!(deleted, 300);
        assert!(!cache.join("entry0.png").exists());
        assert!(!cache.join("entry1.png").exists());
        assert!(!cache.join("entry2.png").exists());
        assert!(cache.join("entry3.png").exists());
        assert!(cache.join("entry4.png").exists());
        // Under budget -> untouched.
        assert_eq!(cache_prune(cache, 250), 0);
        // A cache_read hit refreshes mtime, protecting the entry from the next prune.
        std::fs::write(cache.join("old.png"), [0u8; 100]).unwrap();
        let f = std::fs::File::options()
            .write(true)
            .open(cache.join("old.png"))
            .unwrap();
        f.set_modified(base).unwrap();
        assert!(cache_read(cache, "old").is_some());
        cache_prune(cache, 250);
        assert!(
            cache.join("old.png").exists(),
            "a just-read entry must be treated as hot"
        );
    }

    #[test]
    fn box_downscale_averages_not_samples() {
        // 2x2 opaque checkerboard of R = [0, 255 / 255, 0] → 1x1 must be the MEAN (128),
        // proving area-average vs nearest-neighbour (which would return a single corner: 0 or 255).
        let src = [
            0, 0, 0, 255, 255, 0, 0, 255, // row 0: R=0, R=255
            255, 0, 0, 255, 0, 0, 0, 255, // row 1: R=255, R=0
        ];
        let out = box_downscale(&src, 2, 2, 1, 1);
        assert_eq!(out[3], 255, "opaque");
        assert_eq!(out[0], 128, "mean of 0,255,255,0 rounds to 128");
        assert_eq!(&out[1..3], &[0, 0]);
    }

    #[test]
    fn box_downscale_matches_f64_reference() {
        // The pre-integer f64 body, kept as the semantic reference: the integer version must
        // agree within ±1 per channel (rounding at exact ties is the only allowed difference).
        fn reference(rgba: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
            let mut out = vec![0u8; nw * nh * 4];
            for y in 0..nh {
                let sy0 = y * h / nh;
                let sy1 = (((y + 1) * h) / nh).max(sy0 + 1).min(h);
                for x in 0..nw {
                    let sx0 = x * w / nw;
                    let sx1 = (((x + 1) * w) / nw).max(sx0 + 1).min(w);
                    let (mut r, mut g, mut b, mut a, mut count) = (0f64, 0f64, 0f64, 0f64, 0f64);
                    for sy in sy0..sy1 {
                        let row = sy * w;
                        for sx in sx0..sx1 {
                            let i = (row + sx) * 4;
                            let af = rgba[i + 3] as f64 / 255.0;
                            r += rgba[i] as f64 * af;
                            g += rgba[i + 1] as f64 * af;
                            b += rgba[i + 2] as f64 * af;
                            a += rgba[i + 3] as f64;
                            count += 1.0;
                        }
                    }
                    let dst = (y * nw + x) * 4;
                    let a_avg = a / count;
                    out[dst + 3] = a_avg.round().clamp(0.0, 255.0) as u8;
                    if a_avg > 0.0 {
                        let a_frac = a_avg / 255.0;
                        out[dst] = (r / count / a_frac).round().clamp(0.0, 255.0) as u8;
                        out[dst + 1] = (g / count / a_frac).round().clamp(0.0, 255.0) as u8;
                        out[dst + 2] = (b / count / a_frac).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            out
        }
        let (w, h, nw, nh) = (37usize, 23usize, 13usize, 7usize);
        let mut seed = 0x12345678u64;
        let rgba: Vec<u8> = (0..w * h * 4)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 33) as u8
            })
            .collect();
        let a = box_downscale(&rgba, w, h, nw, nh);
        let b = reference(&rgba, w, h, nw, nh);
        for i in 0..a.len() {
            assert!(
                (a[i] as i32 - b[i] as i32).abs() <= 1,
                "channel {i}: {} vs reference {}",
                a[i],
                b[i]
            );
        }
    }

    #[test]
    fn box_downscale_no_dark_bleed_from_transparent() {
        // A transparent black pixel next to an opaque red pixel: premultiplied averaging must
        // keep the surviving color red (not muddied toward black), only the alpha halves.
        let src = [
            255, 0, 0, 255, // opaque red
            0, 0, 0, 0, // transparent
        ];
        let out = box_downscale(&src, 2, 1, 1, 1);
        assert_eq!(out[3], 128, "alpha is the mean of 255 and 0");
        assert_eq!(&out[0..3], &[255, 0, 0], "color stays red, no dark bleed");
    }

    #[test]
    fn diff_mask_transparent_where_equal() {
        let png = rgba_to_png(&[10, 20, 30, 255, 40, 50, 60, 255], 2, 1).unwrap();
        let mask = diff_mask_png(&png, &png).unwrap();
        let (rgba, _, _) = decode_png_rgba(&mask).unwrap();
        assert!(
            rgba.iter().all(|&b| b == 0),
            "identical composites → fully transparent"
        );
    }

    #[test]
    fn diff_mask_opaque_where_changed() {
        let before = rgba_to_png(&[0, 0, 0, 255, 0, 0, 0, 255], 2, 1).unwrap();
        // Flip only the second pixel well past the threshold.
        let after = rgba_to_png(&[0, 0, 0, 255, 200, 200, 200, 255], 2, 1).unwrap();
        let mask = diff_mask_png(&before, &after).unwrap();
        let (rgba, _, _) = decode_png_rgba(&mask).unwrap();
        assert_eq!(
            &rgba[0..4],
            &[0, 0, 0, 0],
            "unchanged pixel stays transparent"
        );
        assert_eq!(
            &rgba[4..8],
            &HIGHLIGHT_RGBA,
            "changed pixel is accent-tinted"
        );
    }

    #[test]
    fn outline_hugs_changed_pixels_not_bbox() {
        // 3x1: only the middle pixel changed → one closed loop around that single cell, with
        // corners at x = 1/3 and 2/3 — never the 0..1 span a whole-canvas box would have.
        let before = rgba_to_png(&[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255], 3, 1).unwrap();
        let after = rgba_to_png(&[0, 0, 0, 255, 200, 200, 200, 255, 0, 0, 0, 255], 3, 1).unwrap();
        let (_png, outline) = diff_overlay(&before, &after).unwrap();
        let d = outline.expect("a changed pixel yields an outline");
        assert_eq!(d.matches('Z').count(), 1, "one closed loop");
        assert!(
            d.contains("0.3333") && d.contains("0.6667"),
            "loop hugs the middle cell: {d}"
        );
        assert!(!d.contains("M0.0000 0.0000"), "not a full-canvas box: {d}");
    }

    #[test]
    fn outline_none_when_identical() {
        let png = rgba_to_png(&[10, 20, 30, 255], 1, 1).unwrap();
        assert!(diff_overlay(&png, &png).unwrap().1.is_none());
    }

    #[test]
    fn cap_png_passthrough_small_and_malformed() {
        // Already within the cap → returned byte-identical (borrowed).
        let png = rgba_to_png(&[0u8; 16], 2, 2).unwrap();
        assert!(matches!(cap_png(&png), std::borrow::Cow::Borrowed(_)));
        // Undecodable input → returned untouched, never an error.
        assert!(matches!(
            cap_png(b"not a png"),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
