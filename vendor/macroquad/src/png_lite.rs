//! Last Route: a small PNG reader and writer in place of the `image` crate (and with it `png`, `flate2`, `fdeflate`,
//! `crc32fast`, `simd-adler32`, `color_quant`, `bytemuck`, `byteorder` and `num-traits`). The game only ever loads
//! PNG, and only ever asks for 8-bit RGBA; this gives exactly that. See LAST_ROUTE_PATCHES.md.
//!
//! Read: every PNG colour type (grey, RGB, palette, grey+alpha, RGBA), bit depths 1 to 16, `tRNS`, Adam7 interlace.
//! The result is 8 bits a channel RGBA, converted as the `image` crate converted it (a sample below 8 bits is scaled
//! to 0..255; a 16-bit sample is divided by 257 with rounding), so every picture in the repository decodes to the
//! same bytes as before (`shared/png_parity_tests.rs` in the game checks that).
//! Write: 8-bit RGBA, no filtering, zlib level 6.
//!
//! The zlib work is miniz_oxide's, which the game already links.

/// The file formats `Image::from_file_with_format` accepts. Only PNG; the variant keeps the call sites as they were.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
}

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The Paeth predictor of the PNG specification.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Undoes the filter of one scanline in place. `prev` is the unfiltered line above (zeros for the first), `bpp` the
/// whole bytes a pixel takes (at least 1).
fn unfilter(kind: u8, line: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), String> {
    match kind {
        0 => {}
        1 => {
            for i in bpp..line.len() {
                line[i] = line[i].wrapping_add(line[i - bpp]);
            }
        }
        2 => {
            for i in 0..line.len() {
                line[i] = line[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..line.len() {
                let left = if i >= bpp { line[i - bpp] as u16 } else { 0 };
                line[i] = line[i].wrapping_add(((left + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..line.len() {
                let (left, up_left) = if i >= bpp { (line[i - bpp], prev[i - bpp]) } else { (0, 0) };
                line[i] = line[i].wrapping_add(paeth(left, prev[i], up_left));
            }
        }
        other => return Err(format!("PNG: unknown filter {other}")),
    }
    Ok(())
}

struct Header {
    width: usize,
    height: usize,
    depth: u8,
    colour: u8,
    interlaced: bool,
}

impl Header {
    fn channels(&self) -> usize {
        match self.colour {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Bytes of one unfiltered scanline `w` pixels wide.
    fn line_bytes(&self, w: usize) -> usize {
        (w * self.channels() * self.depth as usize + 7) / 8
    }
}

/// The 16-bit sample as the `image` crate narrows it to 8 bits.
fn narrow16(v: u16) -> u8 {
    ((v as u32 + 128) / 257) as u8
}

/// Sample `i` of a scanline at `depth` bits, as its raw value (not scaled).
fn sample(line: &[u8], i: usize, depth: u8) -> u16 {
    match depth {
        8 => line[i] as u16,
        16 => u16::from_be_bytes([line[2 * i], line[2 * i + 1]]),
        d => {
            let bit = i * d as usize;
            let shift = 8 - d as usize - (bit % 8);
            ((line[bit / 8] >> shift) & ((1u16 << d) - 1) as u8) as u16
        }
    }
}

/// One unfiltered scanline of `w` pixels as RGBA8, written to `out` (4 * w bytes).
fn expand(h: &Header, line: &[u8], w: usize, palette: &[u8], trns: &[u8], out: &mut [u8]) -> Result<(), String> {
    let depth = h.depth;
    // a grey or colour sample as 8 bits
    let to8 = |v: u16| -> u8 {
        match depth {
            16 => narrow16(v),
            8 => v as u8,
            d => (v as u32 * 255 / ((1u32 << d) - 1)) as u8,
        }
    };
    for x in 0..w {
        let px = &mut out[4 * x..4 * x + 4];
        match h.colour {
            0 => {
                let v = sample(line, x, depth);
                let g = to8(v);
                // tRNS for grey: one 16-bit value that is fully transparent
                let clear = trns.len() >= 2 && u16::from_be_bytes([trns[0], trns[1]]) == v;
                px.copy_from_slice(&[g, g, g, if clear { 0 } else { 255 }]);
            }
            2 => {
                let (r, g, b) = (sample(line, 3 * x, depth), sample(line, 3 * x + 1, depth), sample(line, 3 * x + 2, depth));
                let clear = trns.len() >= 6
                    && u16::from_be_bytes([trns[0], trns[1]]) == r
                    && u16::from_be_bytes([trns[2], trns[3]]) == g
                    && u16::from_be_bytes([trns[4], trns[5]]) == b;
                px.copy_from_slice(&[to8(r), to8(g), to8(b), if clear { 0 } else { 255 }]);
            }
            3 => {
                let i = sample(line, x, depth) as usize;
                let rgb = palette.get(3 * i..3 * i + 3).ok_or("PNG: a pixel names a colour outside the palette")?;
                px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], trns.get(i).copied().unwrap_or(255)]);
            }
            4 => {
                let g = to8(sample(line, 2 * x, depth));
                px.copy_from_slice(&[g, g, g, to8(sample(line, 2 * x + 1, depth))]);
            }
            _ => {
                for c in 0..4 {
                    px[c] = to8(sample(line, 4 * x + c, depth));
                }
            }
        }
    }
    Ok(())
}

/// Decodes a PNG file to (width, height, RGBA8 bytes, top row first).
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if bytes.get(0..8) != Some(&SIGNATURE[..]) {
        return Err("not a PNG file".into());
    }
    let mut at = 8;
    let mut header: Option<Header> = None;
    let (mut palette, mut trns, mut idat): (Vec<u8>, Vec<u8>, Vec<u8>) = (vec![], vec![], vec![]);
    loop {
        let len = be32(bytes, at).ok_or("PNG: cut short")? as usize;
        let kind = bytes.get(at + 4..at + 8).ok_or("PNG: cut short")?;
        let data = bytes.get(at + 8..at + 8 + len).ok_or("PNG: cut short")?;
        match kind {
            b"IHDR" => {
                if len < 13 {
                    return Err("PNG: bad header".into());
                }
                let h = Header {
                    width: be32(data, 0).unwrap_or(0) as usize,
                    height: be32(data, 4).unwrap_or(0) as usize,
                    depth: data[8],
                    colour: data[9],
                    interlaced: data[12] == 1,
                };
                let depth_ok = match h.colour {
                    0 => matches!(h.depth, 1 | 2 | 4 | 8 | 16),
                    3 => matches!(h.depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => matches!(h.depth, 8 | 16),
                    _ => false,
                };
                if !depth_ok || h.width == 0 || h.height == 0 || data[10] != 0 || data[11] != 0 || data[12] > 1 {
                    return Err("PNG: unsupported header".into());
                }
                if h.width.checked_mul(h.height).map_or(true, |n| n > 1 << 28) {
                    return Err("PNG: too large".into());
                }
                header = Some(h);
            }
            b"PLTE" => palette = data.to_vec(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = header.ok_or("PNG: no header")?;
    if h.colour == 3 && palette.is_empty() {
        return Err("PNG: no palette".into());
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).map_err(|e| format!("PNG: bad image data ({e:?})"))?;
    let bpp = ((h.channels() * h.depth as usize + 7) / 8).max(1);
    let mut out = vec![0u8; h.width * h.height * 4];
    // the passes: (x start, y start, x step, y step); one whole-image pass when not interlaced
    let passes: &[(usize, usize, usize, usize)] = if h.interlaced {
        &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    } else {
        &[(0, 0, 1, 1)]
    };
    let mut pos = 0;
    let mut row = vec![];
    for &(x0, y0, dx, dy) in passes {
        if x0 >= h.width || y0 >= h.height {
            continue;
        }
        let (w, rows) = ((h.width - x0 + dx - 1) / dx, (h.height - y0 + dy - 1) / dy);
        let n = h.line_bytes(w);
        let mut prev = vec![0u8; n];
        row.resize(4 * w, 0);
        for r in 0..rows {
            let kind = *raw.get(pos).ok_or("PNG: image data cut short")?;
            let mut line = raw.get(pos + 1..pos + 1 + n).ok_or("PNG: image data cut short")?.to_vec();
            pos += 1 + n;
            unfilter(kind, &mut line, &prev, bpp)?;
            expand(&h, &line, w, &palette, &trns, &mut row)?;
            let y = y0 + r * dy;
            if dx == 1 {
                out[4 * y * h.width..4 * (y * h.width + w)].copy_from_slice(&row);
            } else {
                for i in 0..w {
                    let to = 4 * (y * h.width + x0 + i * dx);
                    out[to..to + 4].copy_from_slice(&row[4 * i..4 * i + 4]);
                }
            }
            prev = line;
        }
    }
    Ok((h.width as u32, h.height as u32, out))
}

fn crc32(chunks: &[&[u8]]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for bytes in chunks {
        for &b in *bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// Encodes RGBA8 pixels (top row first) as a PNG file.
pub fn encode_rgba8(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::with_capacity((4 * w + 1) * h);
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgba[4 * w * y..4 * w * (y + 1)]);
    }
    let mut out = SIGNATURE.to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_written_reads_back_the_same() {
        let (w, h) = (7u32, 5u32);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 251) as u8).collect();
        let file = encode_rgba8(w, h, &rgba);
        assert_eq!(decode(&file).unwrap(), (w, h, rgba));
    }

    #[test]
    fn the_crc_is_the_png_one() {
        // the CRC of "IEND" with no data is in every PNG file
        assert_eq!(crc32(&[b"IEND"]), 0xAE42_6082);
    }

    /// A one-row PNG built by hand: `colour`/`depth` as in the header, `line` the unfiltered scanline.
    fn png(width: u32, colour: u8, depth: u8, line: &[u8], extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[depth, colour, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        for (kind, data) in extra {
            chunk(&mut out, kind, data);
        }
        let mut raw = vec![0u8];
        raw.extend_from_slice(line);
        chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn every_colour_type_becomes_rgba() {
        // grey, 1 bit: 1 0 1 -> white black white
        assert_eq!(decode(&png(3, 0, 1, &[0b1010_0000], &[])).unwrap().2, [255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255]);
        // grey, 2 bits: value 2 of 3 is 170
        assert_eq!(decode(&png(1, 0, 2, &[0b1000_0000], &[])).unwrap().2, [170, 170, 170, 255]);
        // RGB with a colour key
        let key: &[u8] = &[0, 9, 0, 8, 0, 7];
        assert_eq!(decode(&png(2, 2, 8, &[9, 8, 7, 1, 2, 3], &[(b"tRNS", key)])).unwrap().2, [9, 8, 7, 0, 1, 2, 3, 255]);
        // palette with one transparent entry
        let pal: &[u8] = &[10, 20, 30, 40, 50, 60];
        let alpha: &[u8] = &[128];
        assert_eq!(decode(&png(2, 3, 8, &[1, 0], &[(b"PLTE", pal), (b"tRNS", alpha)])).unwrap().2, [40, 50, 60, 255, 10, 20, 30, 128]);
        // grey + alpha
        assert_eq!(decode(&png(1, 4, 8, &[77, 200], &[])).unwrap().2, [77, 77, 77, 200]);
        // 16-bit RGBA narrows with rounding: 0xFFFF -> 255, 0x0101 -> 1, 0x0080 -> 0, 0x0081 -> 1
        assert_eq!(decode(&png(1, 6, 16, &[0xFF, 0xFF, 0x01, 0x01, 0x00, 0x80, 0x00, 0x81], &[])).unwrap().2, [255, 1, 0, 1]);
    }

    #[test]
    fn what_is_not_a_png_is_refused_without_a_panic() {
        assert!(decode(b"").is_err());
        assert!(decode(b"GIF89a....").is_err());
        let mut cut = encode_rgba8(4, 4, &[9; 64]);
        cut.truncate(cut.len() / 2);
        assert!(decode(&cut).is_err());
        let mut bad = png(1, 3, 8, &[5], &[(b"PLTE", &[1, 2, 3])]);
        assert!(decode(&bad).is_err(), "a pixel past the palette");
        bad[8 + 8 + 8] = 7; // depth 7
        assert!(decode(&bad).is_err());
    }
}
