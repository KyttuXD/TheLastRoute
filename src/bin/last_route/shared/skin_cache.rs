//! The texture cache of runner skins: which skin ids have a texture on the GPU, which are still waiting for their file,
//! and which can never be drawn. It is generic over the texture type and reaches the outside only through
//! [`SkinBackend`], so every decision (a pending skin, a refused one, the least-recently-used drop, the bound on
//! bookkeeping) is tested here with no GPU; `Assets` plugs in macroquad textures.
//!
//! The one rule the game relies on: **a skin that cannot be drawn is never an error for the frame.** [`SkinCache::get`]
//! answers `None` for a pending, refused, wrongly sized or unreadable skin, and the caller draws the default mannequin
//! atlas instead. A skin that is refused (it passed the hash and structure check but would not decode) is remembered
//! as such and never read again, so a hostile file costs one read, not one per frame.
//!
//! Dropping a texture is how a skin's GPU memory is freed, so the cache is the only place that decides when a skin
//! texture goes: the least recently used one once more than the cap are held (Godot `PEER_CACHE_MAX`).

use crate::shared::clock::Instant;
use crate::shared::skins::SKIN_SIZE;
use std::collections::HashMap;
use std::time::Duration;

/// How often a missing skin file is looked for again on disk.
pub const RECHECK: Duration = Duration::from_secs(2);
/// How often a missing skin is asked for again from the website.
pub const REFETCH: Duration = Duration::from_secs(30);
/// The most skin ids that are not (yet) drawable the cache keeps notes about (pending and refused ones). The ids come from
/// other runners' profiles, so they are not to be trusted to stay few; the oldest note goes first.
pub const MAX_UNREADY: usize = 64;
/// The bytes of a decoded skin: 64 x 64 RGBA8. A texture is only made from exactly this many.
pub const SKIN_RGBA_BYTES: usize = (SKIN_SIZE * SKIN_SIZE * 4) as usize;

/// What the cache needs from the world around it.
pub trait SkinBackend<T> {
    /// The verified PNG bytes of skin `id` (the skins folder, the fetch cache or the runner's own memory), when they are here.
    fn read(&mut self, id: &str) -> Option<Vec<u8>>;
    /// Makes the texture of decoded 64x64 RGBA8 pixels (the GPU upload; on the GL thread).
    fn upload(&mut self, rgba: &[u8]) -> T;
    /// Asks the website for skin `id` in the background (never blocks the frame).
    fn fetch(&mut self, id: &str);
    /// Skin `id` is on the GPU (`true`) or its texture was freed (`false`): its cache file is kept or may be pruned.
    fn in_use(&mut self, id: &str, on: bool);
    /// How many skin textures are kept at once.
    fn cap(&self) -> usize;
}

enum Slot<T> {
    /// The texture and the tick of its last use.
    Ready(T, u64),
    /// A file that passed the hash and structure check but would not decode (or decoded to the wrong size): never read
    /// again, the default skin is drawn. The tick is when it was noted.
    Bad(u64),
    /// No file yet: looked for again at `check_at`, fetched from the website again at `fetch_at`. The tick is when it was noted.
    Waiting { check_at: Instant, fetch_at: Instant, noted: u64 },
}

/// The skin textures by skin id.
pub struct SkinCache<T> {
    slots: HashMap<String, Slot<T>>,
    tick: u64,
}

impl<T> Default for SkinCache<T> {
    fn default() -> Self {
        SkinCache { slots: HashMap::new(), tick: 0 }
    }
}

impl<T: Clone> SkinCache<T> {
    /// The texture skin `id` draws with, or `None` when the default skin is to be drawn (DEFAULT itself, a skin whose file
    /// is not here yet, a skin that was refused). Looks for a missing file every [`RECHECK`] and asks the website for it
    /// every [`REFETCH`].
    pub fn get(&mut self, id: &str, now: Instant, backend: &mut dyn SkinBackend<T>) -> Option<T> {
        if id == crate::net::world_sync::SKIN_DEFAULT {
            return None;
        }
        self.tick += 1;
        let tick = self.tick;
        match self.slots.get_mut(id) {
            Some(Slot::Ready(t, used)) => {
                *used = tick;
                return Some(t.clone());
            }
            Some(Slot::Bad(_)) => return None,
            _ => {}
        }
        let (check_at, fetch_at, noted) = match self.slots.get(id) {
            Some(Slot::Waiting { check_at, fetch_at, noted }) => (*check_at, *fetch_at, *noted),
            _ => (now, now, tick),
        };
        if now < check_at {
            return None;
        }
        if let Some(bytes) = backend.read(id) {
            // verified (hash, every chunk checksum, 64x64), then decoded through the fallible path with the base layer opaque
            let decoded = crate::shared::skins::remote_skin_rgba(&bytes).and_then(|px| {
                if px.len() == SKIN_RGBA_BYTES {
                    Ok(px)
                } else {
                    // the GPU upload asserts on the length, and an assert in the draw loop is a lost frame loop
                    Err(format!("{} bytes of pixels, not {SKIN_RGBA_BYTES}", px.len()))
                }
            });
            return match decoded {
                Ok(px) => {
                    let t = backend.upload(&px);
                    self.slots.insert(id.to_string(), Slot::Ready(t.clone(), tick));
                    // a skin that is on the GPU is in use: its cache file is not pruned from under it
                    backend.in_use(id, true);
                    self.evict(id, backend);
                    Some(t)
                }
                Err(why) => {
                    eprintln!("skins: {id} was refused ({why}); the default skin is drawn");
                    self.slots.insert(id.to_string(), Slot::Bad(noted));
                    self.trim_unready(id);
                    None
                }
            };
        }
        let fetch_at = if now >= fetch_at {
            backend.fetch(id);
            now + REFETCH
        } else {
            fetch_at
        };
        self.slots.insert(id.to_string(), Slot::Waiting { check_at: now + RECHECK, fetch_at, noted });
        self.trim_unready(id);
        None
    }

    /// Keeps at most `backend.cap()` textures: the least recently used one is freed (dropping the last handle frees the GPU
    /// texture) and is read from the disk cache again if it is needed later. `keep` is the one just loaded.
    fn evict(&mut self, keep: &str, backend: &mut dyn SkinBackend<T>) {
        let max = backend.cap().max(1);
        while self.ready() > max {
            let oldest = self
                .slots
                .iter()
                .filter_map(|(k, s)| match s {
                    Slot::Ready(_, used) if k != keep => Some((*used, k.clone())),
                    _ => None,
                })
                .min()
                .map(|(_, k)| k);
            let Some(key) = oldest else { return };
            self.slots.remove(&key);
            backend.in_use(&key, false);
        }
    }

    /// Forgets the oldest notes about pending or refused skins beyond [`MAX_UNREADY`]; `keep` is the one just noted.
    fn trim_unready(&mut self, keep: &str) {
        while self.unready() > MAX_UNREADY {
            let oldest = self
                .slots
                .iter()
                .filter_map(|(k, s)| match s {
                    Slot::Bad(n) | Slot::Waiting { noted: n, .. } if k != keep => Some((*n, k.clone())),
                    _ => None,
                })
                .min()
                .map(|(_, k)| k);
            let Some(key) = oldest else { return };
            self.slots.remove(&key);
        }
    }

    /// How many skin textures are held.
    pub fn ready(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Ready(..))).count()
    }

    /// How many ids are noted as pending or refused.
    pub fn unready(&self) -> usize {
        self.slots.values().filter(|s| !matches!(s, Slot::Ready(..))).count()
    }

    /// Whether skin `id` was refused for good (a file that will never draw).
    pub fn is_refused(&self, id: &str) -> bool {
        matches!(self.slots.get(id), Some(Slot::Bad(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::skins::{encode_png, sha256_hex};

    /// A backend with no GPU: textures are numbers, and everything asked of it is counted.
    #[derive(Default)]
    struct Fake {
        files: HashMap<String, Vec<u8>>,
        reads: Vec<String>,
        uploads: Vec<usize>,
        fetches: Vec<String>,
        marks: Vec<(String, bool)>,
        cap: usize,
        next: u32,
    }

    impl SkinBackend<u32> for Fake {
        fn read(&mut self, id: &str) -> Option<Vec<u8>> {
            self.reads.push(id.to_string());
            self.files.get(id).cloned()
        }
        fn upload(&mut self, rgba: &[u8]) -> u32 {
            self.uploads.push(rgba.len());
            self.next += 1;
            self.next
        }
        fn fetch(&mut self, id: &str) {
            self.fetches.push(id.to_string());
        }
        fn in_use(&mut self, id: &str, on: bool) {
            self.marks.push((id.to_string(), on));
        }
        fn cap(&self) -> usize {
            self.cap
        }
    }

    fn skin(seed: u8) -> (String, Vec<u8>) {
        let px: Vec<u8> = (0..SKIN_RGBA_BYTES).map(|i| if i % 4 == 3 { 255 } else { (i as u8).wrapping_mul(seed.wrapping_mul(2).wrapping_add(1)) }).collect();
        let png = encode_png(&px, 64, 64);
        (sha256_hex(&png), png)
    }

    fn fake_with(skins: &[(String, Vec<u8>)], cap: usize) -> Fake {
        Fake { files: skins.iter().cloned().collect(), cap, ..Fake::default() }
    }

    fn at(secs: f64) -> Instant {
        // a fixed base keeps the test clock deterministic
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_secs_f64(secs)
    }

    #[test]
    fn the_default_skin_has_no_texture_and_asks_for_nothing() {
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&[], 4);
        assert_eq!(cache.get("DEFAULT", at(0.0), &mut fake), None);
        assert!(fake.reads.is_empty() && fake.fetches.is_empty() && fake.uploads.is_empty());
        assert_eq!(cache.unready(), 0, "DEFAULT is not even noted");
    }

    #[test]
    fn a_pending_skin_draws_the_default_looks_every_two_seconds_and_asks_the_website_every_thirty() {
        let (id, _) = skin(1);
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&[], 4);
        assert_eq!(cache.get(&id, at(0.0), &mut fake), None, "pending: the caller draws the atlas");
        assert_eq!((fake.reads.len(), fake.fetches.len()), (1, 1), "looked for and asked for at once");
        for frame in 1..100 {
            assert_eq!(cache.get(&id, at(frame as f64 * 0.016), &mut fake), None);
        }
        assert_eq!((fake.reads.len(), fake.fetches.len()), (1, 1), "a hundred frames inside two seconds touch nothing");
        cache.get(&id, at(2.1), &mut fake);
        assert_eq!((fake.reads.len(), fake.fetches.len()), (2, 1), "looked for again after two seconds, not asked again");
        cache.get(&id, at(30.5), &mut fake);
        assert_eq!(fake.fetches.len(), 2, "asked again after thirty");
        assert!(fake.uploads.is_empty(), "a texture is never made without a file");
    }

    #[test]
    fn a_skin_that_arrives_later_is_uploaded_once_and_then_served_from_the_cache() {
        let (id, png) = skin(2);
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&[], 4);
        assert_eq!(cache.get(&id, at(0.0), &mut fake), None);
        fake.files.insert(id.clone(), png);
        assert_eq!(cache.get(&id, at(1.0), &mut fake), None, "still inside the two seconds");
        let t = cache.get(&id, at(2.5), &mut fake).expect("the file is here now");
        assert_eq!(fake.uploads, vec![SKIN_RGBA_BYTES], "one upload of exactly 64x64 RGBA");
        let reads = fake.reads.len();
        for k in 0..50 {
            assert_eq!(cache.get(&id, at(3.0 + k as f64 * 0.016), &mut fake), Some(t));
        }
        assert_eq!((fake.reads.len(), fake.uploads.len()), (reads, 1), "a ready skin costs no read and no upload");
        assert_eq!(fake.marks, vec![(id, true)], "the cache file is kept while the texture is held");
    }

    #[test]
    fn every_hostile_file_is_refused_once_and_never_read_or_uploaded() {
        // files that reach the cache as bytes (a download that passed the hash, a cache file an older build wrote): the decode
        // refuses each one, the cache notes it as refused, the default is drawn, and the GPU never sees it
        let good = skin(3).1;
        let mut header_only = good.clone();
        header_only.truncate(33);
        let mut flipped = good.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 0xff;
        let hostile: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("text", b"this is not a png".to_vec()),
            ("signature only", good[..8].to_vec()),
            ("header only", header_only),
            ("a flipped byte", flipped),
            ("trailing zeros", [good.clone(), vec![0; 100]].concat()),
            ("a PNG signature then noise", [good[..8].to_vec(), vec![0xab; 500]].concat()),
            ("one pixel", encode_png(&[255, 0, 0, 255], 1, 1)),
            ("64x32", encode_png(&vec![200u8; 64 * 32 * 4], 64, 32)),
            ("128x128", encode_png(&vec![200u8; 128 * 128 * 4], 128, 128)),
        ];
        for (what, bytes) in hostile {
            let mut cache = SkinCache::<u32>::default();
            let mut fake = Fake { cap: 4, ..Fake::default() };
            let id = sha256_hex(&bytes);
            fake.files.insert(id.clone(), bytes);
            assert_eq!(cache.get(&id, at(0.0), &mut fake), None, "{what}: the default skin is drawn");
            assert!(cache.is_refused(&id), "{what}: noted as refused");
            assert!(fake.uploads.is_empty(), "{what}: never reaches the GPU");
            let reads = fake.reads.len();
            for k in 1..200 {
                assert_eq!(cache.get(&id, at(k as f64), &mut fake), None);
            }
            assert_eq!(fake.reads.len(), reads, "{what}: a refused skin is never read again");
            assert!(fake.fetches.is_empty(), "{what}: and never asked for again");
        }
    }

    #[test]
    fn the_real_decode_reaches_the_upload_at_exactly_64_by_64_rgba() {
        // the GPU upload asserts on its length: the cache only ever hands it 64x64 RGBA8
        struct Short;
        impl SkinBackend<u32> for Short {
            fn read(&mut self, _: &str) -> Option<Vec<u8>> {
                Some(skin(4).1)
            }
            fn upload(&mut self, rgba: &[u8]) -> u32 {
                assert_eq!(rgba.len(), SKIN_RGBA_BYTES, "an upload of the wrong size would panic the draw loop");
                1
            }
            fn fetch(&mut self, _: &str) {}
            fn in_use(&mut self, _: &str, _: bool) {}
            fn cap(&self) -> usize {
                4
            }
        }
        let mut cache = SkinCache::<u32>::default();
        let (id, _) = skin(4);
        assert_eq!(cache.get(&id, at(0.0), &mut Short), Some(1), "the real 64x64 decode reaches the upload at full size");
    }

    #[test]
    fn the_least_recently_used_texture_is_freed_beyond_the_cap_and_comes_back_from_the_files() {
        let all: Vec<(String, Vec<u8>)> = (1..=5).map(skin).collect();
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&all, 3);
        for (i, (id, _)) in all.iter().enumerate() {
            assert!(cache.get(id, at(i as f64 * 0.1), &mut fake).is_some());
            assert!(cache.ready() <= 3, "never more than the cap on the GPU");
        }
        assert_eq!(cache.ready(), 3);
        assert_eq!(fake.marks.iter().filter(|m| !m.1).map(|m| m.0.clone()).collect::<Vec<_>>(), vec![all[0].0.clone(), all[1].0.clone()], "the two oldest were freed, oldest first");
        // touching the oldest survivor keeps it through the next load
        assert!(cache.get(&all[2].0, at(1.0), &mut fake).is_some());
        let uploads = fake.uploads.len();
        assert!(cache.get(&all[0].0, at(1.1), &mut fake).is_some(), "a freed skin is read again from its file");
        assert_eq!(fake.uploads.len(), uploads + 1);
        assert!(cache.ready() <= 3);
        assert!(cache.get(&all[2].0, at(1.2), &mut fake).is_some());
        assert_eq!(fake.uploads.len(), uploads + 1, "the one touched last survived the eviction");
    }

    #[test]
    fn a_flood_of_announced_ids_cannot_grow_the_notes_without_bound() {
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&[], 4);
        for n in 0..5000u32 {
            let id = sha256_hex(&n.to_le_bytes());
            assert_eq!(cache.get(&id, at(n as f64 * 0.001), &mut fake), None);
            assert!(cache.unready() <= MAX_UNREADY, "{} notes after {n} ids", cache.unready());
        }
        // a flood of refused ones is bounded the same way
        let mut cache = SkinCache::<u32>::default();
        let mut bad = Fake { cap: 4, ..Fake::default() };
        for n in 0..500u32 {
            let bytes = n.to_le_bytes().to_vec();
            let id = sha256_hex(&bytes);
            bad.files.insert(id.clone(), bytes);
            assert_eq!(cache.get(&id, at(0.0), &mut bad), None);
            assert!(cache.unready() <= MAX_UNREADY);
        }
    }

    #[test]
    fn a_skin_that_is_drawn_right_now_is_never_the_one_dropped_for_a_flood() {
        // ids pending or refused never push a ready texture out: only another ready one does
        let (id, png) = skin(9);
        let mut cache = SkinCache::<u32>::default();
        let mut fake = fake_with(&[(id.clone(), png)], 2);
        let t = cache.get(&id, at(0.0), &mut fake).unwrap();
        for n in 0..300u32 {
            cache.get(&sha256_hex(&n.to_le_bytes()), at(0.0), &mut fake);
        }
        assert_eq!(cache.get(&id, at(1.0), &mut fake), Some(t), "still ready and the same texture");
        assert_eq!(fake.uploads.len(), 1);
    }
}
