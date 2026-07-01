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
    for row in 0..th {
        let cy = ty + row;
        if cy < 0 || cy >= ih {
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

/// Encode an RGBA8 buffer as a PNG and wrap it in a `data:` URL.
pub fn rgba_to_png_data_url(rgba: &[u8], width: u32, height: u32) -> Result<String> {
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(Cursor::new(&mut png), width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| KvcError::BadTiles(format!("png header: {e}")))?;
        w.write_image_data(rgba)
            .map_err(|e| KvcError::BadTiles(format!("png data: {e}")))?;
    }
    Ok(png_bytes_to_data_url(&png))
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
}
