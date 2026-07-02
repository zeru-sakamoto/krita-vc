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

/// Decode one tile's stored bytes into interleaved RGBA (`tw*th*4`). `None` if unsupported.
pub fn tile_to_rgba(stored: &[u8], tw: usize, th: usize, pixelsize: usize) -> Option<Vec<u8>> {
    if pixelsize != 4 {
        return None; // ponytail: RGBA8 only.
    }
    let (&flag, payload) = stored.split_first()?;
    let planar_len = tw * th * pixelsize;
    let planar = match flag {
        0 => payload.to_vec(),
        1 => lzf_decompress(payload, planar_len)?,
        _ => return None,
    };
    if planar.len() != planar_len {
        return None;
    }
    let n = tw * th;
    let mut rgba = vec![0u8; planar_len];
    for i in 0..n {
        rgba[i * 4] = planar[PLANE_R * n + i];
        rgba[i * 4 + 1] = planar[PLANE_G * n + i];
        rgba[i * 4 + 2] = planar[PLANE_B * n + i];
        rgba[i * 4 + 3] = planar[PLANE_A * n + i];
    }
    Some(rgba)
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
/// webview. ponytail: a flat cap with nearest-neighbour resampling; bump it or switch to a box
/// filter if a diff ever needs pixel-accurate zoom.
pub const MAX_RASTER_DIM: u32 = 2048;

/// Downscale an RGBA8 buffer so its longest side is at most `MAX_RASTER_DIM`, returning the new
/// buffer and dimensions. Returns the input unchanged when it's already within the cap.
/// Nearest-neighbour — cheap and adequate for a scaled-down diff preview.
pub fn cap_rgba(rgba: &[u8], width: u32, height: u32) -> (std::borrow::Cow<'_, [u8]>, u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_RASTER_DIM || width == 0 || height == 0 {
        return (std::borrow::Cow::Borrowed(rgba), width, height);
    }
    let scale = MAX_RASTER_DIM as f64 / longest as f64;
    let nw = ((width as f64 * scale).round() as u32).max(1);
    let nh = ((height as f64 * scale).round() as u32).max(1);
    let mut out = vec![0u8; (nw as usize) * (nh as usize) * 4];
    for y in 0..nh {
        let sy = (y as u64 * height as u64 / nh as u64) as usize;
        for x in 0..nw {
            let sx = (x as u64 * width as u64 / nw as u64) as usize;
            let src = (sy * width as usize + sx) * 4;
            let dst = (y as usize * nw as usize + x as usize) * 4;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (std::borrow::Cow::Owned(out), nw, nh)
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
// immutable and never need invalidation. Both helpers are best-effort: a cold or unwritable
// cache must never fail a diff. ponytail: no eviction — capped PNGs are small; add LRU pruning
// if .kvc/cache ever matters.

/// Read a cached capped PNG, or `None` on miss/any error.
pub fn cache_read(cache_dir: &std::path::Path, key: &str) -> Option<Vec<u8>> {
    std::fs::read(cache_dir.join(format!("{key}.png"))).ok()
}

/// Write a capped PNG into the cache (creating the dir for pre-cache repos).
pub fn cache_write(cache_dir: &std::path::Path, key: &str, png: &[u8]) {
    let _ = std::fs::create_dir_all(cache_dir);
    let _ = std::fs::write(cache_dir.join(format!("{key}.png")), png);
}

/// Wrap already-encoded PNG bytes (e.g. mergedimage.png) in a `data:` URL.
pub fn png_bytes_to_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64(png))
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
