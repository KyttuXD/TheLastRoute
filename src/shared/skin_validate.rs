//! Full validation of a skin PNG, shared by the game (`src/bin/last_route/shared/skins.rs`) and the account server
//! (`src/bin/lr_server/skins/skins.rs`) so that both ends refuse exactly the same files.
//!
//! A skin travels between players as raw PNG bytes, and any signed-in player can upload one. Checking the 24-byte header
//! is not enough: a PNG with a good header and a broken body makes the image decoder panic on every machine that
//! downloads it, and a fully transparent one makes the runner invisible. So the whole file is checked without trusting
//! it, in the same way as the Godot backend (`backend/skins.mjs` `validateSkinPng`):
//!
//! * the size cap, the PNG signature, every chunk's length and CRC, IHDR first (13 bytes, exactly 64x64), a legal colour
//!   type and bit depth, no unknown critical chunk, IEND last with nothing after it;
//! * the zlib stream (own bounded inflate, no dependency) must end cleanly, match its Adler-32 and produce exactly the
//!   number of bytes the header promises, never more (a decompression bomb stops at that number);
//! * every scanline filter byte must be 0 to 4;
//! * the pixels are decoded to RGBA8, and every pixel outside the second-layer rectangles must be fully opaque
//!   (the shader discards alpha below 0.5, so a transparent base layer would hide the player).
//!
//! The file is NOT required to be the game's canonical re-encoding: Godot clients hash their own encoder's bytes, so
//! asking for byte equality would break cross-play. Interlaced PNGs are refused (as the Godot backend does).
//! The second-layer rectangles come from the game's own `data/skins/rules.json`, embedded here so that there is one
//! copy of the numbers.

use std::sync::OnceLock;

/// The side length every skin PNG must have.
pub const SKIN_SIZE: u32 = 64;
/// The largest skin file accepted anywhere (the Godot `SKIN_MAX_BYTES`, `PlayerSkin.MAX_BYTES`).
pub const SKIN_MAX_BYTES: usize = 65_536;
/// Bytes of a decoded 64x64 RGBA image.
pub const SKIN_RGBA_BYTES: usize = (SKIN_SIZE * SKIN_SIZE * 4) as usize;
/// A fully opaque pixel.
const OPAQUE: u8 = 255;

/// The eight bytes every PNG file starts with.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// The rules file the second-layer rectangles are read from.
const RULES_JSON: &str = include_str!("../bin/last_route/data/skins/rules.json");

/// The second-layer rectangles `[x, y, w, h]` of `data/skins/rules.json` (`layout.overlay_rects`). Only inside them may a
/// pixel be transparent. Empty when the rules are broken, which makes every transparent pixel a refusal.
pub fn overlay_rects() -> &'static [[u32; 4]] {
    static RECTS: OnceLock<Vec<[u32; 4]>> = OnceLock::new();
    RECTS.get_or_init(|| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(RULES_JSON) else { return Vec::new() };
        let Some(list) = value["layout"]["overlay_rects"].as_array() else { return Vec::new() };
        list.iter()
            .filter_map(|r| {
                let a = r.as_array()?;
                let n = |i: usize| a.get(i).and_then(|x| x.as_u64()).map(|x| x as u32);
                (a.len() == 4).then(|| Some([n(0)?, n(1)?, n(2)?, n(3)?])).flatten()
            })
            .collect()
    })
}

/// True when pixel (x, y) lies in a second-layer rectangle.
pub fn is_overlay_pixel(x: u32, y: u32) -> bool {
    overlay_rects().iter().any(|r| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
}

/// True when every pixel outside the second-layer rectangles has alpha 255. `rgba` is a 64x64 RGBA image.
pub fn base_layer_is_opaque(rgba: &[u8]) -> bool {
    rgba.len() == SKIN_RGBA_BYTES
        && (0..SKIN_SIZE).all(|y| (0..SKIN_SIZE).all(|x| is_overlay_pixel(x, y) || rgba[((y * SKIN_SIZE + x) * 4 + 3) as usize] == OPAQUE))
}

/// The one-argument entry point: the full check with the shared size cap ([`SKIN_MAX_BYTES`]). The game's skin code and
/// `lr_server` both call this (or [`check_skin_png`] with their own configured cap, which never exceeds the shared one).
pub fn validate_skin_png(bytes: &[u8]) -> Result<(), String> {
    check_skin_png(bytes, SKIN_MAX_BYTES)
}

/// The whole check of a skin that came from somewhere else (a download, a cache file, an upload): every rule in the
/// module docs. Ok means the bytes decode, are exactly 64x64 and have an opaque base layer, so drawing them is safe.
pub fn check_skin_png(bytes: &[u8], cap: usize) -> Result<(), String> {
    check_skin_png_with(bytes, Limits { max_bytes: cap, max_chunks: DEFAULT_MAX_CHUNKS })
}

/// [`check_skin_png`] with the chunk limit given too (the game reads it from `data/skins/config.json`).
pub fn check_skin_png_with(bytes: &[u8], limits: Limits) -> Result<(), String> {
    let rgba = decode_skin_png_with(bytes, limits)?;
    if !base_layer_is_opaque(&rgba) {
        return Err("the base layer must be fully opaque".to_string());
    }
    Ok(())
}

/// Checks the structure of the file and decodes it to 64x64 RGBA8 (`SKIN_RGBA_BYTES` bytes). Does not look at transparency.
pub fn decode_skin_png(bytes: &[u8], cap: usize) -> Result<Vec<u8>, String> {
    decode_skin_png_with(bytes, Limits { max_bytes: cap, max_chunks: DEFAULT_MAX_CHUNKS })
}

/// [`decode_skin_png`] with the chunk limit given too. Only the website's shape (64x64, not interlaced) decodes here.
pub fn decode_skin_png_with(bytes: &[u8], limits: Limits) -> Result<Vec<u8>, String> {
    let checked = check_structure(bytes, limits, Shape::Remote)?;
    let (png, raw) = (checked.png, checked.raw);
    let header = png.header;
    let row_bytes = (SKIN_SIZE as usize * checked.channels * header.depth as usize).div_ceil(8);
    let pixels = unfilter(&raw, row_bytes, ((checked.channels * header.depth as usize) / 8).max(1))?;
    to_rgba(&pixels, row_bytes, &header, &png)
}

/// What a skin file may look like beyond the shared rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A skin the website stores and other runners send: exactly 64x64, not interlaced.
    Remote,
    /// A skin the player picks from a file: 64x64, or the old 64x32 (the game upgrades it), interlaced allowed. It is
    /// only ever structurally checked here; the game decodes it and re-encodes it as a `Remote` skin before it is hashed.
    Local,
}

/// The limits of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest file, in bytes.
    pub max_bytes: usize,
    /// The most chunks a file may hold (every chunk is read, so this bounds the work).
    pub max_chunks: usize,
}

/// The chunk limit when the caller has no configured one (the server). The game's `data/skins/config.json` ships the same value.
pub const DEFAULT_MAX_CHUNKS: usize = 64;

/// The structural check of a skin PNG of either [`Shape`]: signature, every chunk's length and CRC, chunk order, the header,
/// and a zlib stream that inflates to exactly the promised size. Ok means a decoder can read the file without panicking.
pub fn check_png(bytes: &[u8], limits: Limits, shape: Shape) -> Result<(), String> {
    check_structure(bytes, limits, shape).map(|_| ())
}

struct Checked {
    png: Chunks,
    channels: usize,
    raw: Vec<u8>,
}

fn check_structure(bytes: &[u8], limits: Limits, shape: Shape) -> Result<Checked, String> {
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    if bytes.len() > limits.max_bytes {
        return Err("the file is too big".into());
    }
    if bytes.len() < PNG_SIGNATURE.len() + 12 || bytes[..8] != PNG_SIGNATURE {
        return Err("that is not a PNG file".into());
    }
    let png = parse_chunks(bytes, limits, shape)?;
    let channels = channels_of(png.header.colour).ok_or("unsupported PNG format")?;
    let expected = expected_len(&png.header, channels);
    // One byte of slack, so a stream that is a byte too long is told apart from one that is corrupt.
    let raw = inflate_zlib(&png.idat, expected + 1).map_err(|_| "the pixel data is corrupt".to_string())?;
    if raw.len() != expected {
        return Err("the pixel data has the wrong size".into());
    }
    Ok(Checked { png, channels, raw })
}

/// The inflated size the header promises: every scanline plus its filter byte, or the seven Adam7 passes.
fn expected_len(h: &Header, channels: usize) -> usize {
    let row_bytes = |width: u32| (width as usize * channels * h.depth as usize).div_ceil(8);
    if h.interlace == 0 {
        return (row_bytes(h.width) + 1) * h.height as usize;
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
                (row_bytes(pw) + 1) * ph as usize
            }
        })
        .sum()
}

#[derive(Clone, Copy)]
struct Header {
    width: u32,
    height: u32,
    depth: u8,
    colour: u8,
    compression: u8,
    filter: u8,
    interlace: u8,
}

struct Chunks {
    header: Header,
    idat: Vec<u8>,
    palette: Vec<u8>,
    trns: Vec<u8>,
}

fn channels_of(colour: u8) -> Option<usize> {
    match colour {
        0 | 3 => Some(1),
        2 => Some(3),
        4 => Some(2),
        6 => Some(4),
        _ => None,
    }
}

fn allowed_depth(colour: u8, depth: u8) -> bool {
    match colour {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        _ => matches!(depth, 8 | 16),
    }
}

/// The CRC-32 of a PNG chunk (type and body), the standard reflected polynomial 0xEDB88320.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    !data.iter().fold(!0u32, |crc, &b| table[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8))
}

fn parse_chunks(bytes: &[u8], limits: Limits, shape: Shape) -> Result<Chunks, String> {
    let cap = limits.max_bytes;
    let mut offset = 8usize;
    let mut header: Option<Header> = None;
    let (mut chunks, mut seen_plte, mut seen_idat, mut idat_done, mut saw_end) = (0usize, false, false, false, false);
    let mut idat = Vec::new();
    let mut palette = Vec::new();
    let mut trns = Vec::new();
    while offset < bytes.len() {
        if saw_end {
            return Err("data after the end of the PNG".into());
        }
        if offset + 12 > bytes.len() {
            return Err("truncated PNG".into());
        }
        let length = u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length > cap || offset + 12 + length > bytes.len() {
            return Err("truncated PNG".into());
        }
        let kind = &bytes[offset + 4..offset + 8];
        let body = &bytes[offset + 8..offset + 8 + length];
        let stored = u32::from_be_bytes([bytes[offset + 8 + length], bytes[offset + 9 + length], bytes[offset + 10 + length], bytes[offset + 11 + length]]);
        if stored != crc32(&bytes[offset + 4..offset + 8 + length]) {
            return Err("a chunk has a bad checksum".into());
        }
        chunks += 1;
        if chunks > limits.max_chunks {
            return Err("too many chunks".into());
        }
        if header.is_none() {
            if kind != b"IHDR" || length != 13 {
                return Err("that is not a PNG file".into());
            }
            let h = Header {
                width: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
                height: u32::from_be_bytes([body[4], body[5], body[6], body[7]]),
                depth: body[8],
                colour: body[9],
                compression: body[10],
                filter: body[11],
                interlace: body[12],
            };
            let legacy = shape == Shape::Local && h.height == SKIN_SIZE / 2;
            if h.width != SKIN_SIZE || !(h.height == SKIN_SIZE || legacy) {
                return Err("a skin must be 64x64 pixels".into());
            }
            if h.interlace == 1 && shape == Shape::Remote {
                return Err("interlaced PNGs are not supported".into());
            }
            if channels_of(h.colour).is_none() || !allowed_depth(h.colour, h.depth) || h.compression != 0 || h.filter != 0 || h.interlace > 1 {
                return Err("an unsupported PNG format".into());
            }
            header = Some(h);
        } else {
            if kind != b"IDAT" && seen_idat {
                idat_done = true;
            }
            match kind {
                b"IHDR" => return Err("a second header".into()),
                b"IDAT" => {
                    if idat_done {
                        return Err("pixel data is split by another chunk".into());
                    }
                    seen_idat = true;
                    idat.extend_from_slice(body);
                }
                b"PLTE" => {
                    let colour = header.as_ref().map_or(0, |h| h.colour);
                    if seen_idat || seen_plte || matches!(colour, 0 | 4) || length == 0 || length % 3 != 0 || length > 768 {
                        return Err("a bad palette".into());
                    }
                    seen_plte = true;
                    palette = body.to_vec();
                }
                b"tRNS" => trns = body.to_vec(),
                b"IEND" => {
                    if length != 0 {
                        return Err("a bad end chunk".into());
                    }
                    saw_end = true;
                }
                // A chunk whose first letter is upper case is critical: a decoder may not skip one it does not know.
                _ if kind[0] & 0x20 == 0 => return Err("a chunk this game cannot read".into()),
                // The third letter must be upper case in every chunk name the spec allows.
                _ if kind[2] & 0x20 != 0 => return Err("a bad chunk name".into()),
                _ => {}
            }
        }
        offset += 12 + length;
    }
    match header {
        Some(header) if saw_end && !idat.is_empty() => {
            if header.colour == 3 && !seen_plte {
                return Err("a bad palette".into());
            }
            Ok(Chunks { header, idat, palette, trns })
        }
        _ => Err("truncated PNG".into()),
    }
}

// ---- zlib / deflate (RFC 1950, RFC 1951), bounded ------------------------------------------------------------------

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    held: u32,
}

impl Reader<'_> {
    fn bits(&mut self, n: u32) -> Result<u32, &'static str> {
        let mut value = 0u32;
        for i in 0..n {
            if self.held == 0 {
                let byte = *self.data.get(self.pos).ok_or("corrupt PNG")?;
                self.pos += 1;
                self.bit = byte as u32;
                self.held = 8;
            }
            value |= (self.bit & 1) << i;
            self.bit >>= 1;
            self.held -= 1;
        }
        Ok(value)
    }
}

/// A canonical Huffman code: how many codes of each length, and the symbols in code order (as in zlib's `puff.c`).
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, &'static str> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left = 1i32;
        for len in 1..16 {
            left = (left << 1) - count[len] as i32;
            if left < 0 {
                return Err("corrupt PNG");
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Huffman { count, symbol })
    }

    fn decode(&self, r: &mut Reader) -> Result<usize, &'static str> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize] as usize);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("corrupt PNG")
    }
}

fn fixed_tables() -> Result<(Huffman, Huffman), &'static str> {
    let mut lengths = [0u8; 288];
    for (i, l) in lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(r: &mut Reader) -> Result<(Huffman, Huffman), &'static str> {
    let nlen = r.bits(5)? as usize + 257;
    let ndist = r.bits(5)? as usize + 1;
    let ncode = r.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err("corrupt PNG");
    }
    let mut lengths = [0u8; 320];
    for &slot in CODE_LENGTH_ORDER.iter().take(ncode) {
        lengths[slot] = r.bits(3)? as u8;
    }
    let code_lengths = Huffman::new(&lengths[..19])?;
    let mut lengths = [0u8; 320];
    let mut index = 0;
    while index < nlen + ndist {
        let sym = code_lengths.decode(r)?;
        if sym < 16 {
            lengths[index] = sym as u8;
            index += 1;
            continue;
        }
        let (value, repeat) = match sym {
            16 => {
                if index == 0 {
                    return Err("corrupt PNG");
                }
                (lengths[index - 1], 3 + r.bits(2)? as usize)
            }
            17 => (0, 3 + r.bits(3)? as usize),
            _ => (0, 11 + r.bits(7)? as usize),
        };
        if index + repeat > nlen + ndist {
            return Err("corrupt PNG");
        }
        for _ in 0..repeat {
            lengths[index] = value;
            index += 1;
        }
    }
    if lengths[256] == 0 {
        return Err("corrupt PNG");
    }
    Ok((Huffman::new(&lengths[..nlen])?, Huffman::new(&lengths[nlen..nlen + ndist])?))
}

/// Inflates a zlib stream, producing at most `limit` bytes: a stream that would produce more is refused on the spot, so
/// a few kilobytes of input can never be blown up into memory. The header, the end of the stream, the Adler-32 and the
/// absence of trailing bytes are all checked.
fn inflate_zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    if data.len() < 6 {
        return Err("corrupt PNG");
    }
    let (cmf, flg) = (data[0], data[1]);
    // Deflate with a window of at most 32 KB, a valid header check, and no preset dictionary.
    if cmf & 0x0f != 8 || cmf >> 4 > 7 || ((cmf as u32) << 8 | flg as u32) % 31 != 0 || flg & 0x20 != 0 {
        return Err("corrupt PNG");
    }
    let mut r = Reader { data, pos: 2, bit: 0, held: 0 };
    let mut out: Vec<u8> = Vec::with_capacity(limit.min(1 << 20));
    loop {
        let last = r.bits(1)? == 1;
        match r.bits(2)? {
            0 => {
                r.held = 0; // skip to the byte boundary
                let head = data.get(r.pos..r.pos + 4).ok_or("corrupt PNG")?;
                let (len, nlen) = (u16::from_le_bytes([head[0], head[1]]), u16::from_le_bytes([head[2], head[3]]));
                if len != !nlen {
                    return Err("corrupt PNG");
                }
                r.pos += 4;
                let body = data.get(r.pos..r.pos + len as usize).ok_or("corrupt PNG")?;
                if out.len() + body.len() > limit {
                    return Err("corrupt PNG");
                }
                out.extend_from_slice(body);
                r.pos += len as usize;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 { fixed_tables()? } else { dynamic_tables(&mut r)? };
                loop {
                    let sym = lit.decode(&mut r)?;
                    if sym < 256 {
                        if out.len() >= limit {
                            return Err("corrupt PNG");
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = sym - 257;
                        if li >= 29 {
                            return Err("corrupt PNG");
                        }
                        let len = LENGTH_BASE[li] as usize + r.bits(LENGTH_EXTRA[li] as u32)? as usize;
                        let di = dist.decode(&mut r)?;
                        if di >= 30 {
                            return Err("corrupt PNG");
                        }
                        let back = DIST_BASE[di] as usize + r.bits(DIST_EXTRA[di] as u32)? as usize;
                        if back > out.len() || out.len() + len > limit {
                            return Err("corrupt PNG");
                        }
                        for _ in 0..len {
                            out.push(out[out.len() - back]);
                        }
                    }
                }
            }
            _ => return Err("corrupt PNG"),
        }
        if last {
            break;
        }
    }
    r.held = 0;
    let tail = data.get(r.pos..).ok_or("corrupt PNG")?;
    if tail.len() != 4 || u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]) != adler32(&out) {
        return Err("corrupt PNG");
    }
    Ok(out)
}

/// The Adler-32 checksum of a zlib stream's data.
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// ---- scanlines and pixels ------------------------------------------------------------------------------------------

/// Reverses the per-row filters. `raw` is `SKIN_SIZE` rows of a filter byte plus `row_bytes` bytes.
fn unfilter(raw: &[u8], row_bytes: usize, bpp: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; row_bytes * SKIN_SIZE as usize];
    for y in 0..SKIN_SIZE as usize {
        let line = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = line[0];
        if filter > 4 {
            return Err("corrupt PNG".into());
        }
        let (done, rest) = out.split_at_mut(y * row_bytes);
        let prev: &[u8] = if y == 0 { &[] } else { &done[(y - 1) * row_bytes..] };
        let row = &mut rest[..row_bytes];
        for i in 0..row_bytes {
            let left = if i >= bpp { row[i - bpp] as i32 } else { 0 };
            let up = prev.get(i).copied().unwrap_or(0) as i32;
            let up_left = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) as i32 } else { 0 };
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => (left + up) / 2,
                _ => paeth(left, up, up_left),
            };
            row[i] = line[1 + i].wrapping_add(predictor as u8);
        }
    }
    Ok(out)
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// One sample of a row at index `i` (a channel of a pixel, or a palette index), of `depth` bits, as its raw value.
fn sample(row: &[u8], i: usize, depth: u8) -> u16 {
    match depth {
        16 => u16::from_be_bytes([row[i * 2], row[i * 2 + 1]]),
        8 => row[i] as u16,
        d => {
            let per_byte = 8 / d as usize;
            let shift = 8 - d as usize * (i % per_byte + 1);
            ((row[i / per_byte] >> shift) & ((1u8 << d) - 1)) as u16
        }
    }
}

/// A raw sample scaled to 0..=255.
fn to_u8(value: u16, depth: u8) -> u8 {
    match depth {
        16 => (value >> 8) as u8,
        8 => value as u8,
        d => (value as u32 * 255 / ((1u32 << d) - 1)) as u8,
    }
}

fn to_rgba(pixels: &[u8], row_bytes: usize, header: &Header, png: &Chunks) -> Result<Vec<u8>, String> {
    let depth = header.depth;
    let n = SKIN_SIZE as usize;
    let mut out = Vec::with_capacity(SKIN_RGBA_BYTES);
    let key = |i: usize| -> Option<u16> { (png.trns.len() >= i * 2 + 2).then(|| u16::from_be_bytes([png.trns[i * 2], png.trns[i * 2 + 1]])) };
    if header.colour == 3 {
        if png.palette.is_empty() || png.palette.len() % 3 != 0 || png.palette.len() > 3 * 256 {
            return Err("corrupt PNG".into());
        }
    }
    for y in 0..n {
        let row = &pixels[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..n {
            let rgba = match header.colour {
                6 => [to_u8(sample(row, x * 4, depth), depth), to_u8(sample(row, x * 4 + 1, depth), depth), to_u8(sample(row, x * 4 + 2, depth), depth), to_u8(sample(row, x * 4 + 3, depth), depth)],
                2 => {
                    let (r, g, b) = (sample(row, x * 3, depth), sample(row, x * 3 + 1, depth), sample(row, x * 3 + 2, depth));
                    let transparent = key(0) == Some(r) && key(1) == Some(g) && key(2) == Some(b);
                    [to_u8(r, depth), to_u8(g, depth), to_u8(b, depth), if transparent { 0 } else { OPAQUE }]
                }
                0 => {
                    let v = sample(row, x, depth);
                    let g = to_u8(v, depth);
                    [g, g, g, if key(0) == Some(v) { 0 } else { OPAQUE }]
                }
                4 => {
                    let g = to_u8(sample(row, x * 2, depth), depth);
                    [g, g, g, to_u8(sample(row, x * 2 + 1, depth), depth)]
                }
                _ => {
                    let index = sample(row, x, depth) as usize;
                    let entry = png.palette.get(index * 3..index * 3 + 3).ok_or("corrupt PNG")?;
                    [entry[0], entry[1], entry[2], png.trns.get(index).copied().unwrap_or(OPAQUE)]
                }
            };
            out.extend_from_slice(&rgba);
        }
    }
    Ok(out)
}

// ---- a tiny PNG writer for tests and fixtures ----------------------------------------------------------------------

/// An 8-bit RGBA PNG of `rgba` with stored (uncompressed) deflate blocks and no filtering. It is valid but big
/// (about 16.5 KB for a skin); the game's own encoder (`shared/skins.rs`) is the one that ships skins. This one exists so
/// tests of both executables can build well-formed and deliberately broken files without a PNG library.
pub fn encode_png_stored(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let stride = (w * 4) as usize;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for row in rgba.chunks(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    png_from_raw(w, h, 8, 6, &zlib_stored(&raw))
}

/// A PNG file around an already compressed zlib stream (`idat`), with a correct CRC on every chunk.
pub fn png_from_raw(w: u32, h: u32, depth: u8, colour: u8, idat: &[u8]) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[depth, colour, 0, 0, 0]);
    let mut out = PNG_SIGNATURE.to_vec();
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", idat);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

/// Appends a chunk with its length and CRC.
pub fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut crc_in = kind.to_vec();
    crc_in.extend_from_slice(body);
    out.extend_from_slice(&crc_in);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// A zlib stream made of stored blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64x64 image: opaque everywhere, with a recognisable colour per pixel.
    fn opaque_rgba() -> Vec<u8> {
        (0..SKIN_SIZE * SKIN_SIZE).flat_map(|i| [(i % 251) as u8, (i / 7 % 256) as u8, (i * 3 % 256) as u8, 255]).collect()
    }

    fn good_png() -> Vec<u8> {
        encode_png_stored(&opaque_rgba(), 64, 64)
    }

    /// A real zlib stream with a dynamic Huffman block (made with zlib level 9, 258 bytes of stream) of a 64x64 RGB image, to prove the
    /// bounded inflate reads what real encoders write and not only the fixed and stored blocks the tests could build.
    const DYNAMIC_RGB_PNG_HEX: &str = include_str!("skin_validate_fixture.hex");

    fn from_hex(text: &str) -> Vec<u8> {
        let text: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    #[test]
    fn a_well_formed_skin_passes_and_decodes_to_its_pixels() {
        let rgba = opaque_rgba();
        let bytes = encode_png_stored(&rgba, 64, 64);
        assert_eq!(check_skin_png(&bytes, SKIN_MAX_BYTES), Ok(()));
        assert_eq!(decode_skin_png(&bytes, SKIN_MAX_BYTES).unwrap(), rgba);
    }

    #[test]
    fn validate_skin_png_is_the_check_with_the_shared_cap() {
        let bytes = encode_png_stored(&opaque_rgba(), 64, 64);
        assert_eq!(validate_skin_png(&bytes), Ok(()));
        assert!(validate_skin_png(&[]).is_err());
        assert!(validate_skin_png(&bytes[..40]).is_err());
        let mut too_big = bytes.clone();
        too_big.resize(SKIN_MAX_BYTES + 1, 0);
        assert!(validate_skin_png(&too_big).is_err());
    }

    #[test]
    fn a_real_dynamic_huffman_stream_inflates_and_decodes() {
        let bytes = from_hex(DYNAMIC_RGB_PNG_HEX);
        let rgba = decode_skin_png(&bytes, SKIN_MAX_BYTES).expect("the fixture is a valid 64x64 RGB PNG");
        // The fixture is made of 8x8 blocks coloured (bx*32, by*32, (bx+by)*16).
        for (x, y) in [(0u32, 0u32), (5, 9), (63, 63), (31, 2)] {
            let i = ((y * 64 + x) * 4) as usize;
            let (bx, by) = (x / 8, y / 8);
            assert_eq!(&rgba[i..i + 4], &[(bx * 32 % 256) as u8, (by * 32 % 256) as u8, ((bx + by) * 16 % 256) as u8, 255], "pixel {x},{y}");
        }
        assert_eq!(check_skin_png(&bytes, SKIN_MAX_BYTES), Ok(()));
    }

    #[test]
    fn a_header_only_png_is_refused_and_does_not_panic() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        push_chunk(&mut bytes, b"IHDR", &[0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0]);
        assert!(check_skin_png(&bytes, SKIN_MAX_BYTES).is_err());
        // The old header-only form: a good IHDR with a zero CRC and nothing after it.
        let mut old = PNG_SIGNATURE.to_vec();
        old.extend_from_slice(&13u32.to_be_bytes());
        old.extend_from_slice(b"IHDR");
        old.extend_from_slice(&[0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert!(check_skin_png(&old, SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn a_transparent_base_layer_is_refused_but_transparent_overlay_pixels_are_fine() {
        let clear = vec![0u8; SKIN_RGBA_BYTES];
        let bytes = encode_png_stored(&clear, 64, 64);
        assert_eq!(check_skin_png(&bytes, SKIN_MAX_BYTES), Err("the base layer must be fully opaque".to_string()));
        // Make only the second-layer rectangles clear: that is what a skin with a hat and jacket does.
        let mut rgba = opaque_rgba();
        for y in 0..64 {
            for x in 0..64 {
                if is_overlay_pixel(x, y) {
                    rgba[((y * 64 + x) * 4 + 3) as usize] = 0;
                }
            }
        }
        assert_eq!(check_skin_png(&encode_png_stored(&rgba, 64, 64), SKIN_MAX_BYTES), Ok(()));
        // One transparent base pixel (the head front, 8,8) is enough to refuse.
        let mut one = opaque_rgba();
        one[((8 * 64 + 8) * 4 + 3) as usize] = 0;
        assert!(check_skin_png(&encode_png_stored(&one, 64, 64), SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn only_exactly_64x64_is_accepted() {
        let wide = encode_png_stored(&vec![255u8; 64 * 32 * 4], 64, 32);
        assert_eq!(decode_skin_png(&wide, SKIN_MAX_BYTES), Err("a skin must be 64x64 pixels".to_string()));
        let big = encode_png_stored(&vec![255u8; 128 * 128 * 4], 128, 128);
        assert!(decode_skin_png(&big, 1 << 20).is_err());
    }

    #[test]
    fn a_corrupt_crc_a_truncation_and_trailing_bytes_are_refused() {
        let good = good_png();
        let mut crc = good.clone();
        let at = crc.len() - 20; // inside the IDAT body's tail
        crc[at] ^= 0x55;
        assert!(check_skin_png(&crc, SKIN_MAX_BYTES).is_err());
        for cut in [0, 7, 8, 20, 33, good.len() / 2, good.len() - 13, good.len() - 1] {
            assert!(check_skin_png(&good[..cut], SKIN_MAX_BYTES).is_err(), "cut at {cut}");
        }
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"junk");
        assert!(check_skin_png(&trailing, SKIN_MAX_BYTES).is_err());
        assert!(check_skin_png(b"GIF89a................................", SKIN_MAX_BYTES).is_err());
        assert!(check_skin_png(&[], SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn a_wrong_inflated_size_a_bad_adler_and_a_bad_filter_byte_are_refused() {
        let rgba = opaque_rgba();
        let stride = 64 * 4;
        let mut raw = Vec::new();
        for row in rgba.chunks(stride) {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        // One byte too many, one too few.
        let longer = png_from_raw(64, 64, 8, 6, &zlib_stored(&[raw.clone(), vec![0]].concat()));
        assert_eq!(decode_skin_png(&longer, SKIN_MAX_BYTES), Err("the pixel data has the wrong size".to_string()));
        let shorter = png_from_raw(64, 64, 8, 6, &zlib_stored(&raw[..raw.len() - 1]));
        assert_eq!(decode_skin_png(&shorter, SKIN_MAX_BYTES), Err("the pixel data has the wrong size".to_string()));
        // A damaged Adler-32.
        let mut z = zlib_stored(&raw);
        let last = z.len() - 1;
        z[last] ^= 1;
        assert!(decode_skin_png(&png_from_raw(64, 64, 8, 6, &z), SKIN_MAX_BYTES).is_err());
        // A filter byte of 9.
        let mut bad = raw.clone();
        bad[0] = 9;
        assert!(decode_skin_png(&png_from_raw(64, 64, 8, 6, &zlib_stored(&bad)), SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn an_inflate_bomb_stops_at_the_promised_size() {
        // A fixed-Huffman stream: one literal then a match of 258 bytes at distance 1, repeated: tiny input, huge output.
        // Build it by hand: BFINAL=1, BTYPE=01, literal 0, then (length 258, dist 1) x 200, end of block.
        let mut bits: Vec<u8> = Vec::new();
        let mut put = |value: u32, n: u32, msb: bool| {
            for i in 0..n {
                let b = if msb { (value >> (n - 1 - i)) & 1 } else { (value >> i) & 1 };
                bits.push(b as u8);
            }
        };
        put(1, 1, false);
        put(1, 2, false);
        put(0x30, 8, true); // literal 0
        for _ in 0..200 {
            put(0xC5, 8, true); // length code 285 (258): 280..287 are 8-bit codes 0xC0.., so 285 = 0xC5
            put(0, 5, true); // distance code 0 (distance 1)
        }
        put(0, 7, true); // end of block
        let mut stream = vec![0x78, 0x01];
        for chunk in bits.chunks(8) {
            stream.push(chunk.iter().enumerate().fold(0u8, |acc, (i, b)| acc | (b << i)));
        }
        stream.extend_from_slice(&[0, 0, 0, 1]);
        assert!(stream.len() < 1000);
        assert_eq!(inflate_zlib(&stream, 100), Err("corrupt PNG"), "the output limit holds");
        // And through the PNG path: the same stream is never inflated past the promised size.
        assert!(decode_skin_png(&png_from_raw(64, 64, 8, 6, &stream), SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn the_file_size_cap_interlace_and_unknown_critical_chunks_are_refused() {
        let good = good_png();
        assert_eq!(decode_skin_png(&good, 1000), Err("the file is too big".to_string()));
        // Interlaced.
        let mut interlaced = PNG_SIGNATURE.to_vec();
        push_chunk(&mut interlaced, b"IHDR", &[0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 1]);
        push_chunk(&mut interlaced, b"IDAT", &zlib_stored(&[0u8; 100]));
        push_chunk(&mut interlaced, b"IEND", &[]);
        assert_eq!(decode_skin_png(&interlaced, SKIN_MAX_BYTES), Err("interlaced PNGs are not supported".to_string()));
        // An unknown critical chunk is refused, an unknown ancillary chunk is skipped.
        for (kind, accepted) in [(b"ZZZZ", false), (b"zzZZ", true)] {
            let mut with = good[..good.len() - 12].to_vec();
            push_chunk(&mut with, kind, b"x");
            push_chunk(&mut with, b"IEND", &[]);
            assert_eq!(check_skin_png(&with, SKIN_MAX_BYTES).is_ok(), accepted, "{}", String::from_utf8_lossy(kind));
        }
        // A bad colour type / depth pair.
        assert!(decode_skin_png(&png_from_raw(64, 64, 16, 3, &zlib_stored(&[0u8; 64 * 64 + 64])), SKIN_MAX_BYTES).is_err());
    }

    #[test]
    fn palette_and_grey_images_decode_with_their_transparency() {
        // Palette, 8-bit: index 0 opaque red, index 1 transparent via tRNS.
        let mut raw = Vec::new();
        for y in 0..64 {
            raw.push(0);
            raw.extend((0..64).map(|x| u8::from(x == 0 && y == 0)));
        }
        let mut png = PNG_SIGNATURE.to_vec();
        push_chunk(&mut png, b"IHDR", &[0, 0, 0, 64, 0, 0, 0, 64, 8, 3, 0, 0, 0]);
        push_chunk(&mut png, b"PLTE", &[255, 0, 0, 0, 255, 0]);
        push_chunk(&mut png, b"tRNS", &[255, 0]);
        push_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
        push_chunk(&mut png, b"IEND", &[]);
        let rgba = decode_skin_png(&png, SKIN_MAX_BYTES).unwrap();
        assert_eq!(&rgba[0..4], &[0, 255, 0, 0], "index 1 is transparent green");
        assert_eq!(&rgba[4..8], &[255, 0, 0, 255], "index 0 is opaque red");
        assert!(!base_layer_is_opaque(&rgba), "the transparent base pixel is caught");
        // 1-bit grey: all white, opaque.
        let mut raw = Vec::new();
        for _ in 0..64 {
            raw.push(0);
            raw.extend([0xffu8; 8]);
        }
        let grey = png_from_raw(64, 64, 1, 0, &zlib_stored(&raw));
        let rgba = decode_skin_png(&grey, SKIN_MAX_BYTES).unwrap();
        assert!(rgba.chunks(4).all(|p| p == [255, 255, 255, 255]));
        assert_eq!(check_skin_png(&grey, SKIN_MAX_BYTES), Ok(()));
    }

    #[test]
    fn the_filters_are_reversed() {
        // Each of the five filters on the same pixels must give back the same image.
        let rgba = opaque_rgba();
        let stride = 64 * 4;
        for filter in 0u8..=4 {
            let mut raw = Vec::new();
            for y in 0..64usize {
                raw.push(filter);
                for i in 0..stride {
                    let at = |yy: i32, ii: i32| -> i32 { if yy < 0 || ii < 0 { 0 } else { rgba[yy as usize * stride + ii as usize] as i32 } };
                    let (a, b, c) = (at(y as i32, i as i32 - 4), at(y as i32 - 1, i as i32), at(y as i32 - 1, i as i32 - 4));
                    let predictor = match filter {
                        0 => 0,
                        1 => a,
                        2 => b,
                        3 => (a + b) / 2,
                        _ => paeth(a, b, c),
                    };
                    raw.push((rgba[y * stride + i] as i32 - predictor) as u8);
                }
            }
            let png = png_from_raw(64, 64, 8, 6, &zlib_stored(&raw));
            assert_eq!(decode_skin_png(&png, SKIN_MAX_BYTES).unwrap(), rgba, "filter {filter}");
        }
    }

    #[test]
    fn the_overlay_rectangles_come_from_the_rules_file() {
        assert_eq!(overlay_rects().len(), 6);
        assert!(is_overlay_pixel(40, 0) && !is_overlay_pixel(8, 8));
    }

    /// The chunks of a PNG as (kind, body).
    fn chunks_of(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        let mut at = 8;
        let mut out = vec![];
        while at + 8 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            out.push((kind, bytes[at + 8..at + 8 + len].to_vec()));
            at += 12 + len;
        }
        out
    }

    /// A hostile peer can announce any file, as long as it hashes to its own id, so every byte of the stream may be chosen by
    /// the attacker with every chunk checksum correct. This feeds the strict check and the decoder thousands of such files
    /// (a stored stream and a real dynamic Huffman stream, damaged in the header and in the compressed data, with stray
    /// palette and transparency chunks) and asserts they never panic and that "accepted" always means a full 64x64 RGBA image.
    #[test]
    fn files_with_good_checksums_and_a_damaged_stream_are_refused_or_decoded_whole_and_never_panic() {
        let seeds = [good_png(), from_hex(DYNAMIC_RGB_PNG_HEX)];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let (mut accepted, mut refused) = (0u32, 0u32);
        for seed in &seeds {
            let original = chunks_of(seed);
            for _ in 0..4000 {
                let mut chunks = original.clone();
                for _ in 0..1 + next() % 4 {
                    let pick = (next() % chunks.len() as u64) as usize;
                    // the header and the compressed data are the two chunks with something to damage
                    let body = &mut chunks[pick].1;
                    match next() % 6 {
                        0 | 1 if !body.is_empty() => {
                            let i = (next() % body.len() as u64) as usize;
                            body[i] = next() as u8;
                        }
                        2 if !body.is_empty() => {
                            let i = (next() % body.len() as u64) as usize;
                            body[i] ^= 1 << (next() % 8);
                        }
                        3 if !body.is_empty() => body.truncate((next() % body.len() as u64) as usize),
                        4 => {
                            let at = (next() % (body.len() as u64 + 1)) as usize;
                            body.insert(at, next() as u8);
                        }
                        _ => {
                            let stray: [&[u8; 4]; 3] = [b"PLTE", b"tRNS", b"gAMA"];
                            let kind = *stray[(next() % 3) as usize];
                            let junk: Vec<u8> = (0..next() % 40).map(|_| next() as u8).collect();
                            chunks.insert(1, (kind, junk));
                        }
                    }
                }
                let mut bytes = PNG_SIGNATURE.to_vec();
                for (kind, body) in &chunks {
                    push_chunk(&mut bytes, kind, body);
                }
                let checked = check_skin_png(&bytes, SKIN_MAX_BYTES);
                let decoded = decode_skin_png(&bytes, SKIN_MAX_BYTES);
                match checked {
                    Ok(()) => {
                        accepted += 1;
                        assert_eq!(decoded.expect("an accepted file decodes").len(), SKIN_RGBA_BYTES);
                    }
                    Err(_) => refused += 1,
                }
            }
        }
        assert!(refused > 1000, "the damage is mostly caught ({refused} refused, {accepted} accepted)");
    }

    #[test]
    fn crc32_and_adler32_match_their_known_values() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
