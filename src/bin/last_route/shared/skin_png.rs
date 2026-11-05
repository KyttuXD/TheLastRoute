//! The strict PNG check every skin passes before any decoder sees it (Godot2 backend/skins.mjs `validateSkinPng`).
//!
//! A skin comes from a file, a link, a Minecraft profile, the website cache or another runner, so its bytes are never
//! trusted: the signature, the length and CRC of every chunk, the chunk order, the header (the right size, a legal colour
//! type and depth), and the zlib stream, which must inflate to exactly the number of bytes the header promises (bounded,
//! so a small file can never expand into a large buffer). Only a file that passes goes on to the image decoder, and the
//! decoder is the fallible one ([`decode_rgba`]); nothing here, and nothing after it, can panic on a bad file.

use macroquad::prelude::{Image, ImageFormat};

/// The side length every skin PNG must have.
pub const SKIN_SIZE: u32 = 64;
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Colour type -> samples per pixel (PNG spec table 11.1; skins.mjs COLOUR_CHANNELS).
fn channels(colour: u8) -> Option<u32> {
    match colour {
        0 | 3 => Some(1),
        2 => Some(3),
        4 => Some(2),
        6 => Some(4),
        _ => None,
    }
}

/// Colour type -> the bit depths the spec allows (skins.mjs ALLOWED_DEPTHS).
fn depth_allowed(colour: u8, depth: u8) -> bool {
    match colour {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        _ => false,
    }
}

/// What the check allows beyond the shared rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A skin the website stores and other runners send: exactly 64x64, not interlaced (skins.mjs).
    Remote,
    /// A skin the player picks: 64x64, or the old 64x32 (the game upgrades it), interlaced allowed.
    Local,
}

/// The limits of the check, from data/skins/config.json.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest file, in bytes.
    pub max_bytes: usize,
    /// The most chunks a file may hold (every chunk is read, so this bounds the work).
    pub max_chunks: usize,
}

/// CRC-32 of a PNG chunk (type and body), the polynomial the PNG spec names.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

struct Header {
    width: u32,
    height: u32,
    depth: u8,
    colour: u8,
    interlace: u8,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Bytes in one scanline of `width` pixels, without the filter byte.
fn row_bytes(width: u32, channels: u32, depth: u8) -> usize {
    (width as usize * channels as usize * depth as usize).div_ceil(8)
}

/// The inflated size the header promises: every scanline plus its filter byte, or the seven Adam7 passes.
fn expected_len(h: &Header, channels: u32) -> usize {
    if h.interlace == 0 {
        return (row_bytes(h.width, channels, h.depth) + 1) * h.height as usize;
    }
    // (x start, y start, x step, y step) of the seven passes
    const PASSES: [(u32, u32, u32, u32); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    PASSES
        .iter()
        .map(|&(xs, ys, dx, dy)| {
            let pass_len = |size: u32, start: u32, step: u32| if size > start { (size - start).div_ceil(step) } else { 0 };
            let (pw, ph) = (pass_len(h.width, xs, dx), pass_len(h.height, ys, dy));
            if pw == 0 || ph == 0 {
                0
            } else {
                (row_bytes(pw, channels, h.depth) + 1) * ph as usize
            }
        })
        .sum()
}

/// Checks the structure of a skin PNG without trusting it. Ok means every chunk is in range with a good CRC, the header is
/// a legal skin header and the pixel data inflates to exactly the promised size. The error names the first problem.
pub fn check_png(bytes: &[u8], limits: Limits, shape: Shape) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    if bytes.len() > limits.max_bytes {
        return Err("the file is too big".into());
    }
    if bytes.len() < PNG_SIGNATURE.len() + 12 || bytes[..8] != PNG_SIGNATURE {
        return Err("not a PNG".into());
    }
    let mut offset = 8usize;
    let mut header: Option<Header> = None;
    let (mut chunks, mut seen_plte, mut seen_idat, mut idat_done, mut seen_end) = (0usize, false, false, false, false);
    let mut idat: Vec<u8> = Vec::new();
    while offset < bytes.len() {
        if seen_end {
            return Err("data after the end of the PNG".into());
        }
        if offset + 12 > bytes.len() {
            return Err("truncated PNG".into());
        }
        let length = be32(&bytes[offset..]) as usize;
        if length > limits.max_bytes || offset + 12 + length > bytes.len() {
            return Err("truncated PNG".into());
        }
        let kind: [u8; 4] = [bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7]];
        let body = &bytes[offset + 8..offset + 8 + length];
        if be32(&bytes[offset + 8 + length..]) != crc32(&bytes[offset + 4..offset + 8 + length]) {
            return Err("a chunk has a bad checksum".into());
        }
        chunks += 1;
        if chunks > limits.max_chunks {
            return Err("too many chunks".into());
        }
        if header.is_none() {
            if &kind != b"IHDR" || length != 13 {
                return Err("not a PNG".into());
            }
            header = Some(Header {
                width: be32(body),
                height: be32(&body[4..]),
                depth: body[8],
                colour: body[9],
                interlace: body[12],
            });
            let (compression, filter) = (body[10], body[11]);
            let h = header.as_ref().unwrap();
            let legacy = shape == Shape::Local && h.height == SKIN_SIZE / 2;
            if h.width != SKIN_SIZE || !(h.height == SKIN_SIZE || legacy) {
                return Err("a skin must be 64x64 pixels".into());
            }
            if !depth_allowed(h.colour, h.depth) || compression != 0 || filter != 0 || h.interlace > 1 {
                return Err("an unsupported PNG format".into());
            }
            if h.interlace == 1 && shape == Shape::Remote {
                return Err("interlaced PNGs are not supported".into());
            }
        } else {
            if &kind != b"IDAT" && seen_idat {
                idat_done = true;
            }
            match &kind {
                b"IHDR" => return Err("a second header".into()),
                b"PLTE" => {
                    let colour = header.as_ref().map_or(0, |h| h.colour);
                    if seen_idat || seen_plte || matches!(colour, 0 | 4) || length == 0 || length % 3 != 0 || length > 768 {
                        return Err("a bad palette".into());
                    }
                    seen_plte = true;
                }
                b"IDAT" => {
                    if idat_done {
                        return Err("pixel data is split by another chunk".into());
                    }
                    seen_idat = true;
                    idat.extend_from_slice(body);
                }
                b"IEND" => {
                    if length != 0 {
                        return Err("a bad end chunk".into());
                    }
                    seen_end = true;
                }
                // an unknown chunk the decoder must understand is refused; an ancillary one (text, gamma, ...) is skipped
                _ if kind[0] & 0x20 == 0 => return Err("a chunk this game cannot read".into()),
                _ if kind[2] & 0x20 != 0 => return Err("a bad chunk name".into()),
                _ => {}
            }
        }
        offset += 12 + length;
    }
    let Some(h) = header else { return Err("not a PNG".into()) };
    if !seen_end || idat.is_empty() {
        return Err("truncated PNG".into());
    }
    if h.colour == 3 && !seen_plte {
        return Err("a bad palette".into());
    }
    let expected = expected_len(&h, channels(h.colour).unwrap_or(4));
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, expected + 1)
        .map_err(|_| "the pixel data is corrupt".to_string())?;
    if raw.len() != expected {
        return Err("the pixel data has the wrong size".into());
    }
    Ok(())
}

/// The RGBA pixels of a PNG that [`check_png`] accepted: 64 wide, 64 or 32 tall. The decoder is the fallible `image` one
/// that macroquad wraps, and its answer is checked once more against the size, so a bad file is an `Err` and never a panic.
pub fn decode_rgba(bytes: &[u8], limits: Limits, shape: Shape) -> Result<(u32, u32, Vec<u8>), String> {
    check_png(bytes, limits, shape).map_err(|why| format!("the PNG could not be read ({why})"))?;
    let img = Image::from_file_with_format(bytes, Some(ImageFormat::Png))
        .map_err(|_| "the PNG could not be read (the decoder refused it)".to_string())?;
    let (w, h) = (img.width as u32, img.height as u32);
    if w != SKIN_SIZE || (h != SKIN_SIZE && h != SKIN_SIZE / 2) || img.bytes.len() != (w * h * 4) as usize {
        return Err("a skin must be 64x64 pixels".into());
    }
    Ok((w, h, img.bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::skins::encode_png;

    const LIMITS: Limits = Limits { max_bytes: 65536, max_chunks: 64 };

    fn good() -> Vec<u8> {
        let px: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect();
        encode_png(&px, 64, 64)
    }

    /// A PNG with the given chunks (type, body), CRCs right, after the signature.
    fn build(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        for (kind, body) in chunks {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let mut tagged = kind.to_vec();
            tagged.extend_from_slice(body);
            out.extend_from_slice(&tagged);
            out.extend_from_slice(&crc32(&tagged).to_be_bytes());
        }
        out
    }

    fn ihdr(w: u32, h: u32, depth: u8, colour: u8, interlace: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[depth, colour, 0, 0, interlace]);
        b
    }

    fn zlib_of(raw: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
    }

    #[test]
    fn the_encoders_output_passes_the_strict_check() {
        assert_eq!(check_png(&good(), LIMITS, Shape::Remote), Ok(()));
        let (w, h, px) = decode_rgba(&good(), LIMITS, Shape::Remote).expect("decodes");
        assert_eq!((w, h, px.len()), (64, 64, 64 * 64 * 4));
    }

    #[test]
    fn a_header_only_png_is_refused_and_never_reaches_the_decoder() {
        let mut head = PNG_SIGNATURE.to_vec();
        head.extend_from_slice(&13u32.to_be_bytes());
        head.extend_from_slice(b"IHDR");
        head.extend_from_slice(&64u32.to_be_bytes());
        head.extend_from_slice(&64u32.to_be_bytes());
        assert!(check_png(&head, LIMITS, Shape::Remote).is_err());
        assert!(decode_rgba(&head, LIMITS, Shape::Remote).is_err());
    }

    #[test]
    fn a_flipped_byte_anywhere_in_a_chunk_is_a_bad_checksum() {
        let ok = good();
        for at in [12 + 5, 33 + 20, ok.len() - 20] {
            let mut bad = ok.clone();
            bad[at] ^= 0x40;
            let err = check_png(&bad, LIMITS, Shape::Remote).unwrap_err();
            assert!(err.contains("checksum") || err.contains("truncated"), "{at}: {err}");
        }
    }

    #[test]
    fn truncated_data_and_trailing_data_are_refused() {
        let ok = good();
        assert!(check_png(&ok[..ok.len() - 12], LIMITS, Shape::Remote).is_err(), "no IEND");
        assert!(check_png(&ok[..ok.len() - 3], LIMITS, Shape::Remote).is_err(), "cut inside IEND");
        let mut trailing = ok.clone();
        trailing.extend_from_slice(b"junk");
        assert!(check_png(&trailing, LIMITS, Shape::Remote).is_err(), "data after IEND");
        assert!(check_png(&ok, Limits { max_bytes: ok.len() - 1, ..LIMITS }, Shape::Remote).is_err(), "over the cap");
    }

    #[test]
    fn a_stream_that_inflates_to_the_wrong_size_is_refused() {
        let short = build(&[(b"IHDR", ihdr(64, 64, 8, 6, 0)), (b"IDAT", zlib_of(&vec![0u8; 100])), (b"IEND", vec![])]);
        assert!(check_png(&short, LIMITS, Shape::Remote).unwrap_err().contains("wrong size"));
        let exact = (64 * 4 + 1) * 64;
        let long = build(&[(b"IHDR", ihdr(64, 64, 8, 6, 0)), (b"IDAT", zlib_of(&vec![0u8; exact + 1])), (b"IEND", vec![])]);
        assert!(check_png(&long, LIMITS, Shape::Remote).is_err(), "one byte too many");
        let right = build(&[(b"IHDR", ihdr(64, 64, 8, 6, 0)), (b"IDAT", zlib_of(&vec![0u8; exact])), (b"IEND", vec![])]);
        assert_eq!(check_png(&right, LIMITS, Shape::Remote), Ok(()));
    }

    #[test]
    fn an_inflate_bomb_is_stopped_at_the_promised_size() {
        // 64 MB of zeros compress to about 64 KB; the header promises 16 KB, so inflating stops right after it.
        let bomb = zlib_of(&vec![0u8; 64 * 1024 * 1024]);
        let png = build(&[(b"IHDR", ihdr(64, 64, 8, 6, 0)), (b"IDAT", bomb), (b"IEND", vec![])]);
        let start = std::time::Instant::now();
        if png.len() <= LIMITS.max_bytes {
            assert!(check_png(&png, LIMITS, Shape::Remote).is_err());
        }
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn size_colour_type_and_depth_are_whitelisted() {
        let exact = (64 * 4 + 1) * 64;
        let with = |hdr: Vec<u8>| build(&[(b"IHDR", hdr), (b"IDAT", zlib_of(&vec![0u8; exact])), (b"IEND", vec![])]);
        assert!(check_png(&with(ihdr(64, 32, 8, 6, 0)), LIMITS, Shape::Remote).unwrap_err().contains("64x64"), "64x32 is not stored");
        assert!(check_png(&with(ihdr(128, 128, 8, 6, 0)), LIMITS, Shape::Remote).unwrap_err().contains("64x64"));
        assert!(check_png(&with(ihdr(64, 64, 8, 5, 0)), LIMITS, Shape::Remote).unwrap_err().contains("unsupported"), "colour type 5");
        assert!(check_png(&with(ihdr(64, 64, 4, 6, 0)), LIMITS, Shape::Remote).unwrap_err().contains("unsupported"), "4-bit RGBA");
        assert!(check_png(&with(ihdr(64, 64, 8, 6, 2)), LIMITS, Shape::Remote).unwrap_err().contains("unsupported"), "interlace 2");
        assert!(check_png(&with(ihdr(64, 64, 8, 6, 1)), LIMITS, Shape::Remote).unwrap_err().contains("interlaced"));
    }

    #[test]
    fn the_old_64x32_shape_is_only_for_a_picked_file() {
        let exact = (64 * 4 + 1) * 32;
        let old = build(&[(b"IHDR", ihdr(64, 32, 8, 6, 0)), (b"IDAT", zlib_of(&vec![0u8; exact])), (b"IEND", vec![])]);
        assert_eq!(check_png(&old, LIMITS, Shape::Local), Ok(()));
        assert!(check_png(&old, LIMITS, Shape::Remote).is_err());
        let (w, h, _) = decode_rgba(&old, LIMITS, Shape::Local).expect("decodes");
        assert_eq!((w, h), (64, 32));
    }

    #[test]
    fn an_interlaced_picked_file_gets_the_adam7_size() {
        // the seven passes of a 64x64 RGBA8 image
        let mut total = 0;
        for (xs, ys, dx, dy) in [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)] {
            let (pw, ph) = ((64 - xs + dx - 1) / dx, (64 - ys + dy - 1) / dy);
            total += (pw * 4 + 1) * ph;
        }
        let png = build(&[(b"IHDR", ihdr(64, 64, 8, 6, 1)), (b"IDAT", zlib_of(&vec![0u8; total])), (b"IEND", vec![])]);
        assert_eq!(check_png(&png, LIMITS, Shape::Local), Ok(()));
        assert!(decode_rgba(&png, LIMITS, Shape::Local).is_ok(), "the decoder reads it too");
    }

    #[test]
    fn chunk_order_and_unknown_critical_chunks_are_policed() {
        let exact = (64 * 4 + 1) * 64;
        let data = zlib_of(&vec![0u8; exact]);
        let base = |extra: Vec<(&'static [u8; 4], Vec<u8>)>| {
            let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"IHDR", ihdr(64, 64, 8, 6, 0))];
            chunks.extend(extra);
            chunks.push((b"IDAT", data.clone()));
            chunks.push((b"IEND", vec![]));
            build(&chunks)
        };
        assert_eq!(check_png(&base(vec![(b"tEXt", b"k\0v".to_vec())]), LIMITS, Shape::Remote), Ok(()), "ancillary text is skipped");
        assert!(check_png(&base(vec![(b"ABCD", vec![1])]), LIMITS, Shape::Remote).unwrap_err().contains("cannot read"), "unknown critical");
        assert_eq!(check_png(&base(vec![(b"PLTE", vec![0; 3])]), LIMITS, Shape::Remote), Ok(()), "a suggested palette on RGBA is legal");
        let grey = build(&[(b"IHDR", ihdr(64, 64, 8, 0, 0)), (b"PLTE", vec![0; 3]), (b"IDAT", zlib_of(&vec![0u8; 65 * 64])), (b"IEND", vec![])]);
        assert!(check_png(&grey, LIMITS, Shape::Remote).unwrap_err().contains("palette"), "PLTE on greyscale is not");
        assert!(check_png(&base(vec![(b"IHDR", ihdr(64, 64, 8, 6, 0))]), LIMITS, Shape::Remote).unwrap_err().contains("second header"));
        let split = build(&[
            (b"IHDR", ihdr(64, 64, 8, 6, 0)),
            (b"IDAT", data[..4].to_vec()),
            (b"tEXt", b"k\0v".to_vec()),
            (b"IDAT", data[4..].to_vec()),
            (b"IEND", vec![]),
        ]);
        assert!(check_png(&split, LIMITS, Shape::Remote).unwrap_err().contains("split"));
        let many: Vec<(&[u8; 4], Vec<u8>)> = (0..70).map(|_| (b"tEXt", b"k\0v".to_vec())).collect();
        assert!(check_png(&base(many), LIMITS, Shape::Remote).unwrap_err().contains("too many"));
    }

    #[test]
    fn a_palette_png_needs_its_palette() {
        let exact = 65 * 64; // 8-bit indexed: one byte per pixel plus the filter byte
        let data = zlib_of(&vec![0u8; exact]);
        let without = build(&[(b"IHDR", ihdr(64, 64, 8, 3, 0)), (b"IDAT", data.clone()), (b"IEND", vec![])]);
        assert!(check_png(&without, LIMITS, Shape::Remote).unwrap_err().contains("palette"));
        let with = build(&[(b"IHDR", ihdr(64, 64, 8, 3, 0)), (b"PLTE", vec![255, 0, 0]), (b"IDAT", data), (b"IEND", vec![])]);
        assert_eq!(check_png(&with, LIMITS, Shape::Remote), Ok(()));
        assert!(decode_rgba(&with, LIMITS, Shape::Remote).is_ok());
    }

    #[test]
    fn random_garbage_never_panics_the_check_or_the_decoder() {
        let mut state = 99u32;
        let mut next = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        };
        let ok = good();
        for round in 0..300 {
            let mut bytes = ok.clone();
            for _ in 0..(1 + round % 6) {
                let at = next() as usize * 7 % bytes.len();
                bytes[at] = next();
            }
            if round % 5 == 0 {
                bytes.truncate(next() as usize * 9 % bytes.len());
            }
            let _ = decode_rgba(&bytes, LIMITS, Shape::Remote);
            let _ = decode_rgba(&bytes, LIMITS, Shape::Local);
        }
    }
}
