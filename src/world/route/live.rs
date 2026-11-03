//! The live Infinite route's level-0 chunks, read from the ported Godot layout.
//!
//! `chunk` takes one level-0 chunk from `layout::chunk_layout` (the golden-checked generator:
//! rooms, grid, props, fixtures, pickups, music boxes, hunter spawns) and turns it into the
//! `(Facility, Fixtures)` pair that `world::infinite::window` merges into the 3x3 live window.
//! Levels -1 to -3 never come here; `world::infinite::chunk` keeps the old builder for them.
//!
//! What the adapter keeps from the layout, field by field:
//! - `grid` becomes the walls (every non-zero code is solid), cell for cell.
//! - `rooms` become `Facility.rooms` (rect, centre, type, region, great-hall flag).
//! - `region_props` become `Facility.props`: the type, cell, rotation (quarter turns from `yaw`),
//!   the solid flag and the footprint from `blocker_size`. Props with no model render nothing.
//! - `pickups` by kind: relay and battery become `Facility.relays` and `Facility.batteries`; coin
//!   becomes `Fixtures.coins` (each claimed coin is worth `COIN_PICKUP_VALUE`, 2, Godot
//!   `_sync_stream_progress` 5995-6006); tool, aid and backpack become `Facility.pickups` when the
//!   `Pickup` enum can hold them, and `Game::floor_pickups` takes the ones the `Item` enum can hold.
//! - `fixtures` of kind hiding_spot (without a prop) become `Facility.hiding`; vent fixtures become
//!   `Fixtures.vents`; the first music box becomes `Fixtures.music_box`.
//! - The spawn is the chunk centre cell (22,22), Godot's spawn cell (SRG:1432), which is open on
//!   every layout (the centre cross is always carved).
//!
//! Claims are keyed by Godot entity id (`pickup_id`, streamed_route_generator.gd:1528). `layout_pickups`
//! gives each chunk's ids in layout order, and `window_ids` gives the window's id lists in the order
//! `infinite::window` merges them, so `world::route` callers can key claims without a cell lookup.
//!
//! Safe rooms are not in the `Facility`: `safe_room` and `is_safe_chunk` give a chunk's spec (the Godot
//! `safe_room_data`), and `game::endless` keeps the state of the middle chunk's room (`shared::safe_room`).
//! Level 0 has no breakable props, gates or valves in any golden (those are the sewer levels -1 and -2
//! and the Inferno -3), so the live window has nothing to port for them on this level.
//!
//! What is deliberately not taken here (see docs/PARITY.md, "Live level 0"): the old centre heater,
//! the old vent ring, the old room light switches, the Rust stair flights beyond the shared level 0
//! to -1 link, and the layout's checkpoints, hunter spawns and stairwells, which stay in the layout
//! until their behaviour stages land.
//!
//! `KYTTU_LEGACY_CHUNKS=1` (read in `world::infinite::chunk`) puts the old level-0 builder back
//! for comparison.

use std::collections::HashMap;

use super::hunter_spawns::HunterSpawn;
use super::layout::{self, ChunkLayout, Coord, Rect, V2};
use crate::hunters::navigation;
use crate::render::door;
use crate::shared::safe_room::SafeRoom;
use crate::world::catalog::Catalog;
use crate::world::facility::{Cell, Facility, Hiding, Options, Pickup, Prop, Room, Stair, STAIR_FLIGHT};
use crate::world::fixtures::Fixtures;
use crate::world::infinite::{self, ChunkKey, CHUNK};
use crate::world::rng::Rng;

/// The chunk centre cell, Godot's spawn cell (streamed_route_generator.gd:1432) and the start of
/// every level-0 chunk.
pub const CENTRE: i32 = CHUNK / 2;

/// Builds one level-0 chunk from the Godot layout. `None` for any other level, and for a chunk the
/// layout does not lay out (`chunk_layout` returns `None` for those).
pub fn chunk(seed: &str, k: ChunkKey, cat: &Catalog) -> Option<(Facility, Fixtures)> {
    if k.level != 0 {
        return None;
    }
    let lay = layout::chunk_layout(seed, Coord::new(k.cx, 0, k.cy))?;
    Some(from_layout(seed, k, &lay, cat))
}

/// The adapter proper: a level-0 `ChunkLayout` as the live chunk pair. `seed` is the world seed
/// text the layout was made with (the decimal `route::world_seed`).
pub fn from_layout(seed: &str, k: ChunkKey, lay: &ChunkLayout, cat: &Catalog) -> (Facility, Fixtures) {
    // The base floor only supplies the 45x45 shell and the catalogue; every field is overwritten below.
    let base_seed = format!("{seed}:LIVE:L0:C{},{}", k.cx, k.cy);
    let opt = Options {
        relay_count: 1,
        battery_count: 1,
        safe_zone_count: 0,
        ..Default::default()
    };
    let mut f = Facility::generate(&base_seed, &opt, cat);

    // walls: the layout grid, code 0 open and every other code solid (level 0 has no water or ramps)
    f.wall = (0..CHUNK as usize)
        .map(|z| (0..CHUNK as usize).map(|x| lay.grid[z][x] != 0).collect())
        .collect();

    f.start = Cell::new(CENTRE, CENTRE);
    f.exit = Cell::new(-10000, -10000);
    f.has_lift = false;
    f.safe_zone = None;
    f.room_lights.clear();

    f.rooms = lay.rooms.iter().map(room_of_layout).collect();
    f.props = lay
        .region_props
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|p| prop_of_layout(p, &lay.rooms))
        .collect();

    // The level 0 to -1 link. Godot has no stairs on level 0 (stair_links, SRG:1399), but the old
    // -1 builder links its upper end to this same shared rule (infinite::stair_link), so the flight
    // is kept until the -1 builder honours the layout's wells (stage 3e). It carves before the
    // lists below are set, so layout pickups and fixtures are not cut out by it.
    f.stairs.clear();
    if let Some((anchor, run)) = infinite::stair_link(seed, k.cx, k.cy, 0) {
        carve_flight(
            &mut f,
            Stair {
                cell: Cell::new(anchor.0, anchor.1),
                dir: -1,
                run,
            },
        );
    }

    f.relays.clear();
    f.batteries.clear();
    f.pickups.clear();
    let mut coins = Vec::new();
    for p in &lay.pickups {
        let cell = Cell::new(p.cell.x, p.cell.y);
        match slot_of(p.kind, p.item) {
            Slot::Relay => f.relays.push(cell),
            Slot::Battery => f.batteries.push(cell),
            Slot::Coin => coins.push(cell),
            Slot::Pickup(item) => f.pickups.push((cell, item)),
            Slot::Dropped => {}
        }
    }

    f.hiding = lay
        .fixtures
        .iter()
        .filter(|x| x.kind == "hiding_spot" && x.hide.is_none())
        .map(|x| Hiding {
            cell: Cell::new(x.cell.x, x.cell.y),
            facing: (x.open_direction.x, x.open_direction.y),
            room: room_index(&lay.rooms, x.cell),
        })
        .collect();

    let fixtures = Fixtures {
        heater: None,
        music_box: lay.music_boxes.first().map(|m| Cell::new(m.cell.x, m.cell.y)),
        vents: lay
            .fixtures
            .iter()
            .filter(|x| x.kind == "vent")
            .map(|x| Cell::new(x.cell.x, x.cell.y))
            .collect(),
        coins,
    };

    f.hunter_spawn = f.start;
    f.rebuild_grids();
    (f, fixtures)
}

/// A layout room as the live room (the fields the live route reads).
fn room_of_layout(r: &layout::Room) -> Room {
    Room {
        id: r.id.clone(),
        sector: String::new(),
        x: r.rect.x,
        y: r.rect.y,
        w: r.rect.w,
        h: r.rect.h,
        center: rect_centre(r.rect),
        rtype: r.room_type.clone(),
        label: r.room_type.clone(),
        region: r.region.to_string(),
        landmark: r.great_hall,
        prop_types: vec![],
    }
}

/// `Rect2i.get_center` (position + size / 2).
fn rect_centre(r: Rect) -> Cell {
    Cell::new(r.x + r.w / 2, r.y + r.h / 2)
}

/// Index of the layout room that holds `cell`, when one does.
fn room_index(rooms: &[layout::Room], cell: V2) -> Option<usize> {
    rooms.iter().position(|r| {
        cell.x >= r.rect.x && cell.y >= r.rect.y && cell.x < r.rect.x + r.rect.w && cell.y < r.rect.y + r.rect.h
    })
}

/// A region prop as the live prop. `yaw` is in radians; the live footprint uses quarter turns. The footprint is
/// the collider (`navigation::footprint`), and a prop low enough to walk over is not solid (`navigation::walk_over`).
fn prop_of_layout(p: &crate::world::route::props::Prop, rooms: &[layout::Room]) -> Prop {
    let rotation = ((p.yaw / std::f64::consts::FRAC_PI_2).round() as i32).rem_euclid(4);
    let size = navigation::footprint(p.collider_size, p.blocker_size);
    Prop {
        id: p.id.clone(),
        cell: Cell::new(p.cell.x, p.cell.y),
        ptype: p.prop_type.to_string(),
        room: room_index(rooms, p.cell).unwrap_or(0),
        rotation,
        mount_y: (p.mount_y != 0.0).then_some(p.mount_y),
        solid: p.solid && !navigation::walk_over(size),
        half: navigation::half_extents(size, rotation),
        door: p.room_door.then(|| {
            let cells: Vec<(i32, i32)> = p.doorway_cells.iter().map(|c| (c.x, c.y)).collect();
            door::draw_for((p.cell.x, p.cell.y), &cells)
        }),
    }
}

/// The live item of a layout tool, aid or backpack. `None` for a name the `Pickup` enum does not
/// hold (none on level 0 today).
fn live_pickup(item: &str) -> Option<Pickup> {
    Some(match item {
        "taser" => Pickup::Taser,
        "lockpick" => Pickup::Lockpick,
        "crowbar" => Pickup::Crowbar,
        "rescue_axe" => Pickup::Axe,
        "stim_pack" => Pickup::Stim,
        "revive_kit" => Pickup::ReviveKit,
        "backpack_small" => Pickup::BackpackSmall,
        "backpack_medium" => Pickup::BackpackMedium,
        "backpack_large" => Pickup::BackpackLarge,
        _ => return None,
    })
}

/// Where a layout pickup lands in the live window. Shared by `from_layout` and `window_ids`, so the
/// id lists follow the lists exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Relay,
    Battery,
    Coin,
    Pickup(Pickup),
    Dropped,
}

fn slot_of(kind: &str, item: &str) -> Slot {
    match kind {
        "relay" => Slot::Relay,
        "battery" => Slot::Battery,
        "coin" => Slot::Coin,
        _ => live_pickup(item).map_or(Slot::Dropped, Slot::Pickup),
    }
}

/// One layout pickup as the claim ledger keys it: its Godot entity id and its kind and item.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPickup {
    pub id: String,
    pub kind: &'static str,
    pub item: &'static str,
}

/// The Godot entity ids of a level-0 chunk's pickups, in layout order (`pickup_id`,
/// streamed_route_generator.gd:1528). `None` on other levels and where the layout does not lay the
/// chunk out.
pub fn layout_pickups(seed: &str, k: ChunkKey) -> Option<Vec<LayoutPickup>> {
    if k.level != 0 {
        return None;
    }
    let lay = layout::chunk_layout(seed, Coord::new(k.cx, 0, k.cy))?;
    Some(
        lay.pickups
            .iter()
            .map(|p| LayoutPickup {
                id: p.id.clone(),
                kind: p.kind,
                item: p.item,
            })
            .collect(),
    )
}

/// The hunter spawns of a level-0 chunk (`ChunkLayout.hunter_spawns`, the Godot `hunter_spawns` of the chunk), in layout
/// order. `None` on other levels and where the layout does not lay the chunk out. A safe chunk has none.
pub fn hunter_spawns(seed: &str, k: ChunkKey) -> Option<Vec<HunterSpawn>> {
    if k.level != 0 {
        return None;
    }
    let lay = layout::chunk_layout(seed, Coord::new(k.cx, 0, k.cy))?;
    Some(lay.hunter_spawns)
}

/// The walls of a level-0 chunk as the discovered floor keeps them (Godot `_discovered_chunks[coord].walls`, read by
/// `_discovered_waypoint_cell_open`): row-major, index `z * CHUNK + x`, true where the layout grid is solid. Only the
/// layout grid counts, not props or stair flights. `None` off level 0 and where the layout does not lay the chunk out.
pub fn walls(seed: &str, k: ChunkKey) -> Option<Vec<bool>> {
    if k.level != 0 {
        return None;
    }
    let lay = layout::chunk_layout(seed, Coord::new(k.cx, 0, k.cy))?;
    Some(lay.grid.iter().flat_map(|row| row.iter().map(|c| *c != 0)).collect())
}

/// The entity ids of the live window's pickup lists. Each list is aligned with the Facility or
/// Fixtures list it names: `relays` with `fac.relays`, `batteries` with `fac.batteries`, `pickups`
/// with `fac.pickups` (every entry, including the ones the Item enum cannot hold), `coins` with
/// `fix.coins`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowIds {
    pub relays: Vec<String>,
    pub batteries: Vec<String>,
    pub pickups: Vec<String>,
    pub coins: Vec<String>,
}

/// The window's entity ids, in the order `infinite::window` merges the chunks (dy outer, dx inner)
/// with each chunk's layout order kept. `lens` is the lengths of `[relays, batteries, pickups,
/// coins]` in the window just built: the ids are returned only when they match, so a window that
/// mixes in another chunk source (a chunk the layout does not lay out, or the legacy builder) falls
/// back to the caller's own keys. `None` on other levels, under `KYTTU_LEGACY_CHUNKS=1`, and then.
pub fn window_ids(
    cache: &mut HashMap<ChunkKey, Vec<LayoutPickup>>,
    seed: &str,
    center: (i32, i32),
    level: i32,
    lens: [usize; 4],
) -> Option<WindowIds> {
    if level != 0 || infinite::legacy_level0() {
        return None;
    }
    let mut out = WindowIds::default();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let k = ChunkKey {
                cx: center.0 + dx,
                cy: center.1 + dy,
                level: 0,
            };
            if !cache.contains_key(&k) {
                let pickups = layout_pickups(seed, k)?;
                cache.insert(k, pickups);
            }
            for p in &cache[&k] {
                match slot_of(p.kind, p.item) {
                    Slot::Relay => out.relays.push(p.id.clone()),
                    Slot::Battery => out.batteries.push(p.id.clone()),
                    Slot::Coin => out.coins.push(p.id.clone()),
                    Slot::Pickup(_) => out.pickups.push(p.id.clone()),
                    Slot::Dropped => {}
                }
            }
        }
    }
    if cache.len() > 192 {
        // the same bound as infinite::window's chunk cache: keep the chunks near the centre
        cache.retain(|k, _| (k.cx - center.0).abs() <= 3 && (k.cy - center.1).abs() <= 3);
    }
    let got = [out.relays.len(), out.batteries.len(), out.pickups.len(), out.coins.len()];
    (got == lens).then_some(out)
}

/// Godot `is_safe_room` (streamed_route_generator.gd:1403) for a level-0 chunk on the private route. The spawn
/// chunk is always safe; every other chunk rolls `0.055 * safe_room_bias` (the level-0 danger is 0.48,
/// `difficulty_for_level` 1290). This repeats `layout::Gen::is_safe_room`, which is private to layout.rs; the
/// `safe_rooms_match_the_goldens` test holds the two to the goldens.
pub fn is_safe_chunk(world: &str, cx: i32, cy: i32) -> bool {
    if cx == 0 && cy == 0 {
        return true;
    }
    let mut rng = Rng::new(&format!("{world}:SAFE:{cx}:0:{cy}"));
    let danger = level0_danger();
    let bias = 1.45 + (0.55 - 1.45) * danger;
    rng.next_float() < 0.055 * bias
}

/// Godot `difficulty_for_level(0).danger` (streamed_route_generator.gd:1293): `clampf(0.48 - level * 0.075, 0.12, 1.0)`.
fn level0_danger() -> f64 {
    0.48f64.clamp(0.12, 1.0)
}

/// The safe room of a level-0 chunk as the Godot `safe_room_data` dictionary (streamed_route_generator.gd:1416),
/// or `None` for another level and for a chunk that is not safe. `breach_pressure` is `lerpf(0.55, 1.65, danger)`
/// snapped to 0.01 (1.08 on level 0).
pub fn safe_room(world: &str, k: ChunkKey) -> Option<SafeRoom> {
    if k.level != 0 || !is_safe_chunk(world, k.cx, k.cy) {
        return None;
    }
    let pressure = ((0.55 + (1.65 - 0.55) * level0_danger()) * 100.0).round() / 100.0;
    Some(SafeRoom {
        id: format!("{}:0:{}:SAFE", k.cx, k.cy),
        checkpoint_id: format!("{}:0:{}:CHECKPOINT", k.cx, k.cy),
        max_integrity: 100,
        breachable: true,
        repairable: true,
        breach_pressure: pressure,
    })
}

/// The 3-wide hall from a stair landing to the centre lobby and down its flight, as the old builder
/// carved it (infinite::chunk, the stairwell block), but from the chunk centre.
fn carve_flight(f: &mut Facility, s: Stair) {
    let far = Cell::new(
        s.cell.x + s.run.0 * (STAIR_FLIGHT + 1),
        s.cell.y + s.run.1 * (STAIR_FLIGHT + 1),
    );
    let x0 = s.cell.x.min(far.x) - 1;
    let x1 = s.cell.x.max(far.x) + 1;
    let y0 = s.cell.y - 1;
    let y1 = s.cell.y + 2;
    f.clear_props_in(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    f.open_cells(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    let (cx, cy) = (f.start.x, f.start.y);
    let mx = (x0 + x1) / 2;
    f.open_cells(mx.min(cx) - 1, y0, (mx - cx).abs() + 3, 3);
    f.open_cells(cx - 1, y0.min(cy), 3, (y0 - cy).abs() + 3);
    f.stairs.push(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn cat() -> &'static Catalog {
        static CAT: OnceLock<Catalog> = OnceLock::new();
        CAT.get_or_init(Catalog::load)
    }

    /// The world seeds of the three Godot goldens (route::djb2 tests).
    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];
    const CHUNKS: [(i32, i32); 6] = [(0, 0), (1, 0), (0, 1), (-2, 3), (5, -4), (7, 7)];

    #[test]
    fn only_level_zero_is_adapted() {
        let k = ChunkKey { cx: 0, cy: 0, level: -1 };
        assert!(chunk(WORLDS[0], k, cat()).is_none());
    }

    #[test]
    fn discovered_walls_are_the_layout_grid() {
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let k = ChunkKey { cx, cy: cz, level: 0 };
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    assert!(walls(world, k).is_none());
                    continue;
                };
                let w = walls(world, k).expect("level 0 has walls");
                assert_eq!(w.len(), (CHUNK * CHUNK) as usize);
                for z in 0..CHUNK as usize {
                    for x in 0..CHUNK as usize {
                        assert_eq!(w[z * CHUNK as usize + x], lay.grid[z][x] != 0, "{world} {cx},{cz}: {x},{z}");
                    }
                }
            }
        }
        assert!(walls(WORLDS[0], ChunkKey { cx: 0, cy: 0, level: -1 }).is_none());
    }

    #[test]
    fn walls_rooms_and_pickups_follow_the_layout() {
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let k = ChunkKey { cx, cy: cz, level: 0 };
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    continue;
                };
                let (f, fix) = chunk(world, k, cat()).expect("level 0 is adapted");
                assert_eq!(f.cols, CHUNK);
                assert_eq!(f.rows, CHUNK);
                assert_eq!(f.start, Cell::new(CENTRE, CENTRE));
                assert!(f.can_occupy(CENTRE as f64 + 0.5, CENTRE as f64 + 0.5, 0.23), "{world} {cx},{cz}: spawn cell is blocked");
                // every open layout cell is open here; a flight can only open more
                for z in 0..CHUNK {
                    for x in 0..CHUNK {
                        let open = lay.grid[z as usize][x as usize] == 0;
                        if open {
                            assert!(!f.is_wall_cell(x, z), "{world} {cx},{cz}: open cell {x},{z} is a wall");
                        }
                    }
                }
                assert_eq!(f.rooms.len(), lay.rooms.len(), "{world} {cx},{cz}: rooms");
                let relays = lay.pickups.iter().filter(|p| p.kind == "relay").count();
                let batteries = lay.pickups.iter().filter(|p| p.kind == "battery").count();
                assert_eq!(f.relays.len(), relays, "{world} {cx},{cz}: relays");
                assert_eq!(f.batteries.len(), batteries, "{world} {cx},{cz}: batteries");
                assert!(fix.heater.is_none());
                // a stair flight clears the props it crosses (clear_props_in); without one, every solid prop stays
                // a prop low enough to walk over is floor (navigation::walk_over), so it is not solid here
                let solid = lay
                    .region_props
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .filter(|p| p.solid && !navigation::walk_over(navigation::footprint(p.collider_size, p.blocker_size)))
                    .count();
                let kept = f.props.iter().filter(|p| p.solid).count();
                if f.stairs.is_empty() {
                    assert_eq!(kept, solid, "{world} {cx},{cz}: solid props");
                } else {
                    assert!(kept <= solid, "{world} {cx},{cz}: solid props grew");
                }
            }
        }
    }

    #[test]
    fn coins_are_the_layout_coins() {
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    continue;
                };
                let (_, fix) = chunk(world, ChunkKey { cx, cy: cz, level: 0 }, cat()).unwrap();
                let coins = lay.pickups.iter().filter(|p| p.kind == "coin").count();
                assert_eq!(fix.coins.len(), coins, "{world} {cx},{cz}: coins");
            }
        }
    }

    #[test]
    fn layout_pickup_ids_follow_the_layout_order() {
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let k = ChunkKey { cx, cy: cz, level: 0 };
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    continue;
                };
                let ids = layout_pickups(world, k).unwrap();
                assert_eq!(ids.len(), lay.pickups.len());
                for (got, want) in ids.iter().zip(&lay.pickups) {
                    assert_eq!(got.id, want.id);
                    assert_eq!(got.kind, want.kind);
                    assert!(got.id.starts_with(&format!("{cx}:0:{cz}:PICKUP:{}:", want.kind.to_uppercase())));
                }
                // ids name their chunk and are unique within it
                let mut seen: Vec<&String> = ids.iter().map(|p| &p.id).collect();
                seen.sort();
                seen.dedup();
                assert_eq!(seen.len(), ids.len(), "{world} {cx},{cz}: duplicate ids");
            }
        }
    }

    #[test]
    fn window_ids_line_up_with_the_window_lists() {
        if infinite::legacy_level0() {
            // the legacy builder has no layout ids (window_ids is None on purpose)
            return;
        }
        let loot = crate::content::loot::Loot::default();
        for world in WORLDS {
            for centre in [(0, 0), (1, 0), (-2, 3)] {
                let mut chunks = HashMap::new();
                let (f, fix) = infinite::window(world, centre, 0, cat(), &loot, &mut chunks);
                let lens = [f.relays.len(), f.batteries.len(), f.pickups.len(), fix.coins.len()];
                let mut cache = HashMap::new();
                let ids = window_ids(&mut cache, world, centre, 0, lens)
                    .unwrap_or_else(|| panic!("{world} {centre:?}: window ids do not line up"));
                assert_eq!(ids.relays.len(), f.relays.len());
                assert_eq!(ids.batteries.len(), f.batteries.len());
                assert_eq!(ids.pickups.len(), f.pickups.len());
                assert_eq!(ids.coins.len(), fix.coins.len());
                // each relay sits in the chunk its id names: window cell -> chunk = centre + (cell / CHUNK - 1)
                for (cell, id) in f.relays.iter().zip(&ids.relays) {
                    let cx = centre.0 + cell.x.div_euclid(CHUNK) - 1;
                    let cz = centre.1 + cell.y.div_euclid(CHUNK) - 1;
                    assert!(id.starts_with(&format!("{cx}:0:{cz}:PICKUP:RELAY:")), "{world} {centre:?}: {id} at {cell:?}");
                }
                let mut all: Vec<&String> = ids.relays.iter().chain(&ids.batteries).chain(&ids.pickups).chain(&ids.coins).collect();
                let n = all.len();
                all.sort();
                all.dedup();
                assert_eq!(all.len(), n, "{world} {centre:?}: duplicate ids in the window");
            }
        }
    }

    #[test]
    fn dispatch_sends_level_zero_here_and_the_rest_to_the_legacy_builder() {
        let loot = crate::content::loot::Loot::default();
        let below = ChunkKey { cx: 1, cy: 2, level: -1 };
        let (a, _) = infinite::chunk(WORLDS[0], below, cat(), &loot);
        let (b, _) = infinite::legacy_chunk(WORLDS[0], below, cat(), &loot);
        assert_eq!(a.wall, b.wall);
        assert_eq!(a.rooms.len(), b.rooms.len());
        let zero = ChunkKey { cx: 1, cy: 2, level: 0 };
        let (z, _) = infinite::chunk(WORLDS[0], zero, cat(), &loot);
        let (w, _) = chunk(WORLDS[0], zero, cat()).unwrap();
        if !infinite::legacy_level0() {
            assert_eq!(z.wall, w.wall);
        }
    }

    #[test]
    fn window_merges_adapted_chunks() {
        let mut cache = std::collections::HashMap::new();
        let loot = crate::content::loot::Loot::default();
        let (f, _) = infinite::window(WORLDS[1], (0, 0), 0, cat(), &loot, &mut cache);
        assert_eq!(f.cols, CHUNK * infinite::WINDOW);
        assert_eq!(f.rows, CHUNK * infinite::WINDOW);
        assert_eq!(f.start, Cell::new(CHUNK + CENTRE, CHUNK + CENTRE));
    }

    /// The level-0 safe chunks of each golden in the 7x7 square (x and z in -3..=3), read from the full goldens
    /// (`data.safe_room`, golden-<tag>.json; the 1.08 pressure and the ids are checked below too).
    const SAFE_GOLDENS: [(&str, &[(i32, i32)]); 7] = [
        ("1426541270", &[(-1, -3), (-1, 1), (0, 0)]),
        ("4157433391", &[(0, 0)]),
        ("2899547009", &[(0, 0), (1, -1), (2, 0), (2, 2)]),
        ("3833121725", &[(-3, 0), (0, 0)]),
        ("2477555712", &[(-3, -2), (0, -3), (0, 0), (2, -3), (3, -3)]),
        ("223482559", &[(-3, 3), (-2, 3), (-1, 3), (0, 0), (0, 2), (1, -2)]),
        ("938954303", &[(-1, 1), (0, 0)]),
    ];

    #[test]
    fn safe_rooms_match_the_goldens() {
        for (world, safe) in SAFE_GOLDENS {
            for cx in -3..=3 {
                for cy in -3..=3 {
                    let want = safe.contains(&(cx, cy));
                    assert_eq!(is_safe_chunk(world, cx, cy), want, "{world} {cx},{cy}: safe room");
                    assert_eq!(safe_room(world, ChunkKey { cx, cy, level: 0 }).is_some(), want, "{world} {cx},{cy}");
                }
            }
        }
    }

    #[test]
    fn safe_room_data_matches_the_golden_dictionary() {
        let r = safe_room(WORLDS[1], ChunkKey { cx: 0, cy: 0, level: 0 }).expect("the spawn chunk is safe");
        assert_eq!(r.id, "0:0:0:SAFE");
        assert_eq!(r.checkpoint_id, "0:0:0:CHECKPOINT");
        assert_eq!(r.max_integrity, 100);
        assert!(r.breachable && r.repairable);
        assert!((r.breach_pressure - 1.08).abs() < 1e-9, "{}", r.breach_pressure);
        let r = safe_room(WORLDS[0], ChunkKey { cx: -1, cy: -3, level: 0 }).expect("golden safe chunk");
        assert_eq!(r.id, "-1:0:-3:SAFE");
        assert_eq!(r.checkpoint_id, "-1:0:-3:CHECKPOINT");
        // not on level 0: no room
        assert!(safe_room(WORLDS[0], ChunkKey { cx: 0, cy: 0, level: -1 }).is_none());
    }

    #[test]
    fn level_zero_has_no_sewer_props() {
        // breakable pipes, sluice gates and valve wheels are sewer props (levels -1 and -2); no level-0 golden has one
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    continue;
                };
                for p in lay.region_props.as_deref().unwrap_or(&[]) {
                    assert!(!p.prop_type.starts_with("sewer"), "{world} {cx},{cz}: {}", p.prop_type);
                }
            }
        }
    }

    #[test]
    fn layout_hunter_spawns_stand_on_open_window_cells() {
        // a spawn is a cell the hunters' grid leaves open (the window's Facility, built from the layout)
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let k = ChunkKey { cx, cy: cz, level: 0 };
                let Some(spawns) = hunter_spawns(world, k) else {
                    continue;
                };
                let (f, _) = chunk(world, k, cat()).unwrap();
                for s in &spawns {
                    assert!(
                        f.hunter_open(Cell::new(s.cell.x, s.cell.y)),
                        "{world} {cx},{cz}: spawn {} is on a closed cell",
                        s.id
                    );
                    assert!(s.id.starts_with(&format!("{cx}:0:{cz}:HUNTER:")), "{}", s.id);
                }
            }
        }
    }

    #[test]
    fn a_low_prop_is_floor_and_a_tall_prop_keeps_its_footprint() {
        for world in WORLDS {
            for (cx, cz) in CHUNKS {
                let k = ChunkKey { cx, cy: cz, level: 0 };
                let Some(lay) = layout::chunk_layout(world, Coord::new(cx, 0, cz)) else {
                    continue;
                };
                let (f, _) = chunk(world, k, cat()).unwrap();
                for p in lay.region_props.as_deref().unwrap_or(&[]) {
                    let fp = navigation::footprint(p.collider_size, p.blocker_size);
                    let Some(live) = f.props.iter().find(|q| q.id == p.id) else {
                        continue;
                    };
                    if navigation::walk_over(fp) {
                        assert!(!live.solid, "{world} {cx},{cz}: low prop {} is solid", p.id);
                    } else {
                        assert_eq!(live.solid, p.solid, "{world} {cx},{cz}: prop {}", p.id);
                        let (hx, hy) = navigation::half_extents(fp, live.rotation);
                        assert_eq!(live.half, (hx, hy), "{world} {cx},{cz}: footprint of {}", p.id);
                    }
                }
            }
        }
    }
}
