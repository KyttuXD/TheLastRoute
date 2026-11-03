//! The Infinite route: an endless grid of 45x45 chunks per level (each built by the finite
//! generator, the same footprint as a Standard floor), streamed as a live 3x3 window, with
//! levels going down from level 0 linked by stairwells.
//!
//! From game/world/generation/streamed_route_generator.gd: CHUNK_SIZE 45, REGION_SIZE_CHUNKS 4,
//! the region rules (world::route::regions, SPAWN_MAZE_SHARE 0.6), EDGE_OFFSETS (where chunk
//! seams may open), CORRIDOR_WIDTH 3,
//! STAIR_BAY_ANCHORS and STAIR_BAYS_BY_BOUNDARY_PARITY (a stair link belongs to the boundary
//! between two levels, so both levels agree on it, and up/down links never share a bay), and
//! MEZZANINE_ENABLED false (level 0 is the top). District-specific room banks and shared
//! hall topology are applied by the facility builder, with sewers and the Inferno district
//! selected by level/region while the finite catalogue supplies authored room content.

use super::catalog::Catalog;
use super::facility::{Cell, Facility, Options, Pickup, Stair, STAIR_FLIGHT};
use super::fixtures::{self, Fixtures};
use super::rng::Rng;
use super::route::regions;
use crate::content::loot::Loot;
use std::collections::HashMap;

pub const CHUNK: i32 = 45;
pub const WINDOW: i32 = 3;
pub const REGION_SIZE_CHUNKS: i32 = regions::REGION_SIZE_CHUNKS;
pub const EDGE_OFFSETS: [i32; 7] = [5, 11, 17, 22, 27, 33, 39];
const STAIR_BAY_ANCHORS: [[(i32, i32); 4]; 4] = [
    [(8, 8), (12, 8), (8, 12), (12, 12)],
    [(32, 8), (36, 8), (32, 12), (36, 12)],
    [(8, 32), (12, 32), (8, 36), (12, 36)],
    [(32, 32), (36, 32), (32, 36), (36, 36)],
];
const STAIR_BAYS_BY_PARITY: [[usize; 2]; 2] = [[0, 3], [1, 2]];
/// Share of boundaries (away from the origin column) that carry a stair link (estimated).
const STAIR_CHANCE: f64 = 0.3;
/// Relays and batteries per chunk (estimated: "denser interactables", spread over many chunks).
const CHUNK_RELAYS: usize = 3;
const CHUNK_BATTERIES: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub cx: i32,
    pub cy: i32,
    pub level: i32,
}

/// The district a chunk belongs to (world::route::regions::region_for, Godot's region_for:
/// `seed` is the world seed, str(run_seed) (route::world_seed), the chunk is (x, z) = (cx, cy)
/// on level k.level).
pub fn region_for(seed: &str, k: ChunkKey) -> &'static str {
    regions::region_for(seed, k.cx, k.cy, k.level)
}

/// The catalogue room region each district prefers to dress its rooms in.
pub fn hall_set(region: &str) -> &'static str {
    match region {
        "maze" => "maze",
        "sewer" => "maze",
        "mansion" => "mansion",
        "hotel" => "hotel",
        "apartment" => "apartment",
        "beach" => "beach",
        "gothic" => "gothic",
        "prison" => "prison",
        "inferno" => "inferno",
        "carnival" => "carnival",
        "hospital" => "hospital",
        "lab" => "lab",
        "greenhouse" => "greenhouse",
        "mall" => "mall",
        _ => "maze",
    }
}

fn boundary_offsets(seed: &str, key: &str) -> Vec<i32> {
    let mut rng = Rng::new(&format!("{seed}:SEAM:{key}"));
    let mut pool = EDGE_OFFSETS.to_vec();
    let count = 1 + (rng.next_float() < 0.5) as usize;
    let mut out = vec![];
    for _ in 0..count {
        out.push(pool.remove(rng.next_index(pool.len())));
    }
    out
}

/// A stair link on the boundary between `upper` and `upper - 1` in column (cx, cy):
/// Some((anchor, run)) when it exists. Shared by both levels.
pub(crate) fn stair_link(seed: &str, cx: i32, cy: i32, upper: i32) -> Option<((i32, i32), (i32, i32))> {
    if upper > 0 {
        return None;
    }
    let mut rng = Rng::new(&format!("{seed}:STAIRS:{upper}:{cx},{cy}"));
    let roll = rng.next_float();
    if !(cx == 0 && cy == 0) && roll >= STAIR_CHANCE {
        return None;
    }
    let bays = STAIR_BAYS_BY_PARITY[upper.rem_euclid(2) as usize];
    let bay = bays[rng.next_index(2)];
    let anchor = STAIR_BAY_ANCHORS[bay][rng.next_index(4)];
    let run = if bay == 0 || bay == 2 {
        (1, 0)
    } else {
        (-1, 0)
    };
    Some((anchor, run))
}

/// `KYTTU_LEGACY_CHUNKS=1` keeps the old level-0 builder (`legacy_chunk`) for comparison. Unset,
/// level 0 is the Godot layout adapter (`route::live`). Levels -1 to -3 always use `legacy_chunk`.
pub fn legacy_level0() -> bool {
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LEGACY.get_or_init(|| std::env::var("KYTTU_LEGACY_CHUNKS").map(|v| v == "1").unwrap_or(false))
}

/// Builds one chunk: level 0 from the Godot layout (`route::live::chunk`), unless
/// `KYTTU_LEGACY_CHUNKS=1`; every other level from `legacy_chunk`.
pub fn chunk(seed: &str, k: ChunkKey, cat: &Catalog, loot: &Loot) -> (Facility, Fixtures) {
    if k.level == 0 && !legacy_level0() {
        if let Some(pair) = super::route::live::chunk(seed, k, cat) {
            return pair;
        }
    }
    legacy_chunk(seed, k, cat, loot)
}

/// The old builder: a `Facility::generate` floor re-dressed as streamed rooms, with Rust's own
/// loot, seams, stair flights and fixtures.
pub fn legacy_chunk(seed: &str, k: ChunkKey, cat: &Catalog, loot: &Loot) -> (Facility, Fixtures) {
    let origin = k.cx == 0 && k.cy == 0 && k.level == 0;
    let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
    let opt = Options {
        relay_count: CHUNK_RELAYS,
        battery_count: CHUNK_BATTERIES,
        safe_zone_count: if origin { 2 } else { 0 },
        ..Default::default()
    };
    let mut f = Facility::generate(&text, &opt, cat);
    // CREATE ROOM uses Godot's streamed route, whose room banks and shared halls
    // differ from the finite floor generator. Rebuild the chunk topology before
    // seams, stairs and fixtures are added so every downstream system sees the
    // same bounded rooms and circulation mask.
    let region = region_for(seed, k);
    f.apply_godot_streamed_layout(&text, region, cat);
    f.has_lift = false;
    f.exit = Cell::new(-10000, -10000);
    // loot (data/loot/route.json): a base battery, then each roll by its chance
    let mut rng = Rng::new(&format!("{text}:LOOT"));
    let mut free: Vec<Cell> = f
        .floor_cells()
        .into_iter()
        .filter(|c| {
            c.len_to(f.start) >= 3.0
                && !f.relays.contains(c)
                && !f.batteries.contains(c)
                && f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.23)
        })
        .collect();
    let mut place = |item: Pickup, rng: &mut Rng, out: &mut Vec<(Cell, Pickup)>| {
        if !free.is_empty() {
            out.push((free.remove(rng.next_index(free.len())), item));
        }
    };
    let mut pickups = vec![];
    for b in &loot.base {
        if b == "battery" {
            place(Pickup::Battery, &mut rng, &mut pickups);
        }
    }
    for (kind, chance) in &loot.rolls {
        if (rng.next_float() as f32) >= *chance {
            continue;
        }
        match kind.as_str() {
            "tool" => {
                let total: f32 = loot.tools.iter().map(|t| t.1).sum();
                let mut r = rng.next_float() as f32 * total;
                let mut pick = "taser";
                for (name, w) in &loot.tools {
                    if r < *w {
                        pick = name;
                        break;
                    }
                    r -= w;
                }
                let item = match pick {
                    "crowbar" => Pickup::Crowbar,
                    "rescue_axe" => Pickup::Axe,
                    "lockpick" => Pickup::Lockpick,
                    _ => Pickup::Taser,
                };
                place(item, &mut rng, &mut pickups);
            }
            "battery" => place(Pickup::Battery, &mut rng, &mut pickups),
            _ => {}
        }
    }
    for (item, chance) in &loot.aid {
        if item == "stim_pack" && (rng.next_float() as f32) < *chance {
            place(Pickup::Stim, &mut rng, &mut pickups);
        }
    }
    f.pickups = pickups;

    // seams: open the chunk's border where its boundaries open, then run a 3-wide hall inward
    let sides = [
        (format!("V:{}:{}:{}", k.level, k.cx, k.cy), 0),
        (format!("V:{}:{}:{}", k.level, k.cx + 1, k.cy), 1),
        (format!("H:{}:{}:{}", k.level, k.cx, k.cy), 2),
        (format!("H:{}:{}:{}", k.level, k.cx, k.cy + 1), 3),
    ];
    for (key, side) in sides {
        for o in boundary_offsets(seed, &key) {
            for step in 0..CHUNK {
                let (x, y) = match side {
                    0 => (step, o),
                    1 => (CHUNK - 1 - step, o),
                    2 => (o, step),
                    _ => (o, CHUNK - 1 - step),
                };
                let (ax, ay) = if side < 2 { (x, y - 1) } else { (x - 1, y) };
                let (w, h) = if side < 2 { (1, 3) } else { (3, 1) };
                let was_open = !f.is_wall_cell(x, y) && step > 1;
                f.open_cells(ax, ay, w, h);
                if was_open {
                    break;
                }
            }
        }
    }

    // stairwells: down to `level - 1` and up to `level + 1`, each on its boundary's bay
    let mut links = vec![];
    if let Some((anchor, run)) = stair_link(seed, k.cx, k.cy, k.level) {
        links.push(Stair {
            cell: Cell::new(anchor.0, anchor.1),
            dir: -1,
            run,
        });
    }
    if let Some((anchor, run)) = stair_link(seed, k.cx, k.cy, k.level + 1) {
        let far = Cell::new(
            anchor.0 + run.0 * (STAIR_FLIGHT + 1),
            anchor.1 + run.1 * (STAIR_FLIGHT + 1),
        );
        links.push(Stair {
            cell: far,
            dir: 1,
            run: (-run.0, -run.1),
        });
    }
    for s in links {
        let ends = [
            s.cell,
            Cell::new(
                s.cell.x + s.run.0 * (STAIR_FLIGHT + 1),
                s.cell.y + s.run.1 * (STAIR_FLIGHT + 1),
            ),
        ];
        let x0 = ends[0].x.min(ends[1].x) - 1;
        let x1 = ends[0].x.max(ends[1].x) + 1;
        let y0 = s.cell.y - 1;
        let y1 = s.cell.y + 2;
        f.clear_props_in(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        f.open_cells(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        // a 3-wide hall from the stair bay to the lobby so the flight is always reachable
        let (cx, cy) = (f.start.x, f.start.y);
        let mx = (x0 + x1) / 2;
        f.open_cells(mx.min(cx) - 1, y0, (mx - cx).abs() + 3, 3);
        f.open_cells(cx - 1, y0.min(cy), 3, (y0 - cy).abs() + 3);
        f.stairs.push(s);
    }
    f.rebuild_grids();
    let fix = fixtures::place(&f);
    (f, fix)
}

/// The live window: 3x3 chunks around `center` on `level`, merged into one floor, plus the
/// fixtures of the window (heater and music box from the centre chunk, its vent ring, every
/// chunk's coins).
pub fn window(
    seed: &str,
    center: (i32, i32),
    level: i32,
    cat: &Catalog,
    loot: &Loot,
    cache: &mut HashMap<ChunkKey, (Facility, Fixtures)>,
) -> (Facility, Fixtures) {
    let mut parts = vec![];
    let mut fix = Fixtures {
        heater: None,
        music_box: None,
        vents: vec![],
        coins: vec![],
    };
    for dy in -1..=1 {
        for dx in -1..=1 {
            let k = ChunkKey {
                cx: center.0 + dx,
                cy: center.1 + dy,
                level,
            };
            let entry = cache.entry(k).or_insert_with(|| chunk(seed, k, cat, loot));
            let (ox, oy) = ((dx + 1) * CHUNK, (dy + 1) * CHUNK);
            let sh = |c: Cell| Cell::new(c.x + ox, c.y + oy);
            if dx == 0 && dy == 0 {
                fix.heater = entry.1.heater.map(sh);
                fix.music_box = entry.1.music_box.map(sh);
                fix.vents = entry.1.vents.iter().map(|c| sh(*c)).collect();
            }
            fix.coins.extend(entry.1.coins.iter().map(|c| sh(*c)));
            parts.push((
                entry.0.clone(),
                ox,
                oy,
                regions::region_index(region_for(seed, k)),
            ));
        }
    }
    if cache.len() > 192 {
        // CHUNK_CACHE_LIMIT: drop chunks far from the window
        cache.retain(|k, _| {
            k.level == level && (k.cx - center.0).abs() <= 3 && (k.cy - center.1).abs() <= 3
        });
    }
    let names = regions::NAMES.iter().map(|s| s.to_string()).collect();
    let mid = parts[4].0.start;
    let mut f = Facility::merge(
        parts,
        CHUNK * WINDOW,
        CHUNK * WINDOW,
        names,
        &format!("{seed}:L{level}"),
    );
    f.start = Cell::new(mid.x + CHUNK, mid.y + CHUNK);
    f.hunter_spawn = f.start;
    // no service lift on the Infinite route (has_lift false); the exit marks the centre lobby
    f.exit = f.start;
    f.has_lift = false;
    f.origin = ((center.0 - 1) * CHUNK, (center.1 - 1) * CHUNK);
    (f, fix)
}

/// A throwaway floor a Game holds before its first window is built.
pub fn placeholder(seed: &str, cat: &Catalog) -> Facility {
    Facility::generate(&format!("{seed}:SHELL"), &Options::default(), cat)
}
