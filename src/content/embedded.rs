//! Files compiled into the executable by build.rs: data/**.json and assets/** (pictures and
//! Blockbench models). Mod packs on disk are layered over these at runtime.

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// An embedded asset by its path under assets/ (e.g. "hunters/aki.bbmodel").
pub fn asset(path: &str) -> Option<&'static [u8]> {
    ASSET_FILES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, b)| *b)
}

/// Every embedded asset path starting with `prefix`.
pub fn assets_under(prefix: &str) -> impl Iterator<Item = (&'static str, &'static [u8])> + '_ {
    ASSET_FILES
        .iter()
        .filter(move |(p, _)| p.starts_with(prefix))
        .map(|(p, b)| (*p, *b))
}

/// Every embedded data file under `folder` (e.g. "hunters/"), in path order.
pub fn data_under(folder: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
    DATA_FILES
        .iter()
        .filter(move |(p, _)| p.starts_with(folder))
        .map(|(p, t)| (*p, *t))
}

/// A small standard base64 decoder for textures embedded in .bbmodel files.
pub fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    };
    let mut out = vec![];
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}
