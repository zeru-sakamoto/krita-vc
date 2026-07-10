//! Parser for Krita's tiled layer-data blocks (the binary files inside a .kra under
//! `<doc>/layers/`). Format: a small text header then `DATA n` tile records, each a text
//! descriptor line `x,y,compression,len\n` followed by `len` bytes of (already compressed)
//! tile data.
//!
//! By default tiles are diffed as opaque LZF-compressed blobs. The opt-in
//! `Config.tile_pixel_deltas` flag instead stores decoded planar pixels (which bsdiff across
//! versions — see `kra.rs` and `raster::lzf_compress`); it stays off by default because the
//! LZF decode/encode cost lands on the commit/restore paths of low-end devices.

use crate::error::{KvcError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub x: i64,
    pub y: i64,
    pub compression: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiledBlock {
    /// The verbatim text header (5 lines incl. trailing newlines) — re-emitted as-is.
    pub header: String,
    pub tiles: Vec<Tile>,
}

/// A layer-data entry is tiled iff it begins with Krita's `VERSION ` header.
pub fn is_tiled(bytes: &[u8]) -> bool {
    bytes.starts_with(b"VERSION ")
}

pub fn parse(bytes: &[u8]) -> Result<TiledBlock> {
    let mut pos = 0usize;
    let mut header = String::new();
    let mut count: Option<usize> = None;

    // Fixed 5-line header: VERSION / TILEWIDTH / TILEHEIGHT / PIXELSIZE / DATA <n>.
    for _ in 0..5 {
        let line = read_line(bytes, &mut pos).ok_or_else(|| bad("truncated header"))?;
        let text = std::str::from_utf8(line).map_err(|_| bad("non-utf8 header"))?;
        if let Some(rest) = text.strip_prefix("DATA ") {
            count = Some(rest.trim().parse().map_err(|_| bad("bad DATA count"))?);
        }
        header.push_str(text);
    }
    let count = count.ok_or_else(|| bad("missing DATA count"))?;

    // `count` is an untrusted header value; a tile record is at least its descriptor line, so the
    // block's byte length bounds the real tile count. Clamp the preallocation to that so a crafted
    // `DATA 99999999999` can't force a giant up-front allocation (the loop still errors on any
    // actual truncation).
    let mut tiles = Vec::with_capacity(count.min(bytes.len()));
    for _ in 0..count {
        let line = read_line(bytes, &mut pos).ok_or_else(|| bad("truncated tile header"))?;
        let text = std::str::from_utf8(line)
            .map_err(|_| bad("non-utf8 tile header"))?
            .trim_end_matches('\n');
        let parts: Vec<&str> = text.split(',').collect();
        if parts.len() != 4 {
            return Err(bad("malformed tile descriptor"));
        }
        let x = parts[0].parse().map_err(|_| bad("bad tile x"))?;
        let y = parts[1].parse().map_err(|_| bad("bad tile y"))?;
        let compression = parts[2].to_string();
        let len: usize = parts[3].parse().map_err(|_| bad("bad tile len"))?;

        let end = pos
            .checked_add(len)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| bad("tile data past end of block"))?;
        let data = bytes[pos..end].to_vec();
        pos = end;
        tiles.push(Tile {
            x,
            y,
            compression,
            data,
        });
    }

    Ok(TiledBlock { header, tiles })
}

/// Rebuild the original block bytes. Round-trips `parse` exactly.
pub fn serialize(block: &TiledBlock) -> Vec<u8> {
    let mut out = block.header.clone().into_bytes();
    for t in &block.tiles {
        out.extend_from_slice(
            format!("{},{},{},{}\n", t.x, t.y, t.compression, t.data.len()).as_bytes(),
        );
        out.extend_from_slice(&t.data);
    }
    out
}

/// Slice from `*pos` up to and including the next `\n`, advancing `*pos` past it.
fn read_line<'a>(bytes: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    if *pos >= bytes.len() {
        return None;
    }
    let nl = bytes[*pos..].iter().position(|&b| b == b'\n')?;
    let line = &bytes[*pos..*pos + nl + 1];
    *pos += nl + 1;
    Some(line)
}

fn bad(msg: &str) -> KvcError {
    KvcError::BadTiles(msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrips() {
        let block = TiledBlock {
            header: "VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 1\n".to_string(),
            tiles: vec![Tile {
                x: 0,
                y: 0,
                compression: "LZF".into(),
                data: vec![1, 2, 3, 4],
            }],
        };
        let bytes = serialize(&block);
        assert_eq!(parse(&bytes).unwrap(), block);
    }

    #[test]
    fn huge_data_count_does_not_over_allocate_or_panic() {
        // A crafted header claiming a preposterous tile count must not force a giant
        // preallocation (the fix clamps `with_capacity` to the block byte length) and must
        // fail cleanly on the very first missing tile descriptor rather than aborting.
        let bytes =
            b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 99999999999\n".to_vec();
        let err = parse(&bytes).unwrap_err();
        assert!(matches!(err, KvcError::BadTiles(_)));
    }
}
