//! The Infinite route's seed text and region rules.
//!
//! A run is started from a room seed text, `ROOM-<code>-<20 hex>` (`new_room_text`, as Godot's
//! RoomState makes it from a random stamp). The run seed is `run_seed_of_text(text)`, and the
//! Infinite route's world seed is the decimal of that run seed (RoomState.run_seed,
//! room_state.gd:1353, and infinite_route_test.gd:280). Saves written before the text was stored
//! have no text: `room_text(run_seed)` (`ROOM-<code>-<8 hex>`) stands in for it, which keeps the
//! world those saves were started in.

use crate::world::rng::Rng;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use crate::shared::clock::{SystemTime, UNIX_EPOCH};

pub mod clearance;
pub mod concept_map;
pub mod danger;
pub mod fixtures;
pub mod furnished;
pub mod grid;
pub mod hunter_spawns;
pub mod layout;
pub mod live;
pub mod loot;
pub mod maze_regions;
pub mod music_boxes;
pub mod pickups;
pub mod prop_tables;
pub mod props;
pub mod regions;
pub mod sewer;

/// The room code alphabet (no 0, 1, I or O), as RoomState.make_room_code.
const CODE_ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const HEX_UPPER: &[u8] = b"0123456789ABCDEF";

/// Godot's `String.hash()`: djb2 over the code points, `h = h * 33 + c` from 5381, kept to 32 bits.
pub fn djb2(text: &str) -> u32 {
    text.chars()
        .fold(5381u32, |h, c| h.wrapping_mul(33).wrapping_add(c as u32))
}

/// `RoomState.run_seed` for a room seed text: `maxi(1, abs(int(text.hash())))`.
pub fn run_seed_of_text(text: &str) -> u32 {
    djb2(text).max(1)
}

/// The Infinite route's world seed for a room seed text: `str(run_seed_of_text(text))`. Every
/// SeededRandom draw of the generator is keyed on this decimal text (`"<world>:BIOME:..."`).
pub fn world_seed(text: &str) -> String {
    run_seed_of_text(text).to_string()
}

/// The five-character room code of a run seed (the same algorithm as the menus' room_code).
pub fn room_code(seed: u32) -> String {
    let mut v = seed;
    (0..5)
        .map(|_| {
            let c = CODE_ALPHA[(v % CODE_ALPHA.len() as u32) as usize] as char;
            v = v / CODE_ALPHA.len() as u32 ^ 0x9e37;
            c
        })
        .collect()
}

/// The room seed text of a run seed, `ROOM-<code>-<8 hex>`. Only saves that predate the stored
/// room text use it (and the headless tools, which take a numeric seed): the Infinite route's
/// world seed is still derived from the text (`world_seed`), as Godot derives its world seed
/// from the room text.
pub fn room_text(run_seed: u32) -> String {
    format!("ROOM-{}-{:08X}", room_code(run_seed), run_seed)
}

/// A new room seed text, `ROOM-<code>-<20 hex uppercase>`, from the clock and a process-wide
/// counter (no new crates: the draws come from the SeededRandom port).
pub fn new_room_text() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(7);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    room_text_from_stamp(&format!("{nanos}:{n}:{}", crate::shared::task::process_id()))
}

/// The room seed text drawn from a stamp (a random stamp in Godot, md5 of it in RoomState).
pub fn room_text_from_stamp(stamp: &str) -> String {
    let mut rng = Rng::new(&format!("ROOM:{stamp}"));
    let code: String = (0..5)
        .map(|_| CODE_ALPHA[rng.next_index(CODE_ALPHA.len())] as char)
        .collect();
    let hex: String = (0..20)
        .map(|_| HEX_UPPER[rng.next_index(16)] as char)
        .collect();
    format!("ROOM-{code}-{hex}")
}

/// The room code inside a room seed text (`ROOM-<code>-<hex>`), as the lobby and the menus show it.
pub fn room_text_code(text: &str) -> &str {
    text.split('-').nth(1).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn djb2_matches_godot_string_hash() {
        // the world seeds of the golden dumps (LastRoute Godot, tests/parity/dump.tscn)
        assert_eq!(world_seed("ROOM-PARITY-0000000000000001"), "1426541270");
        assert_eq!(world_seed("ROOM-K7QXM-3A9F1C0D2B4E5F607182"), "4157433391");
        assert_eq!(world_seed("ROOM-DEMO-00000000000000000000"), "2899547009");
        assert_eq!(djb2(""), 5381);
        assert_eq!(run_seed_of_text(""), 5381);
    }

    #[test]
    fn room_text_has_the_menu_shape() {
        for seed in [1u32, 20261101, 0xDEADBEEF, u32::MAX] {
            let t = room_text(seed);
            assert_eq!(t.len(), "ROOM-".len() + 5 + 1 + 8);
            assert!(t.starts_with("ROOM-"));
            assert_eq!(&t[5..10], room_code(seed));
            assert_eq!(&t[11..], format!("{seed:08X}"));
        }
    }

    #[test]
    fn new_room_text_has_the_godot_shape() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let t = new_room_text();
            // ROOM-<5 code>-<20 hex uppercase>
            assert_eq!(t.len(), "ROOM-".len() + 5 + 1 + 20, "{t}");
            assert!(t.starts_with("ROOM-"));
            let code = room_text_code(&t);
            assert_eq!(code.len(), 5);
            assert!(code.bytes().all(|b| CODE_ALPHA.contains(&b)), "{t}");
            let hex = &t[11..];
            assert!(hex.bytes().all(|b| HEX_UPPER.contains(&b)), "{t}");
            assert!(run_seed_of_text(&t) >= 1);
            seen.insert(t);
        }
        assert!(seen.len() > 190, "room texts should differ between runs");
    }

    #[test]
    fn room_text_from_stamp_is_deterministic() {
        let a = room_text_from_stamp("stamp-1");
        assert_eq!(a, room_text_from_stamp("stamp-1"));
        assert_ne!(a, room_text_from_stamp("stamp-2"));
        assert_eq!(room_text_code(&a), &a[5..10]);
    }

    #[test]
    fn stored_text_drives_the_world_seed() {
        // a stored text and its run seed: the world is str(djb2(text)), not the 8 hex fallback
        let t = "ROOM-PARITY-0000000000000001";
        assert_eq!(world_seed(t), run_seed_of_text(t).to_string());
        assert_eq!(room_text_code(t), "PARITY");
        assert_eq!(room_text_code("garbage"), "");
    }
}
