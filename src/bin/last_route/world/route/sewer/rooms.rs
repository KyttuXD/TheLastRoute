//! More sewer rooms: pump room, valve hall, junction chamber, overflow chamber and storage alcove.
//! Everything tunable (layouts, props, loot, hunter rules, placement chance) is in `src/bin/last_route/data/regions/sewer_rooms.json`
//! (`lr.sewer_rooms/1`). This module holds behaviour only: parsing, picking a free spot, stamping the layout into a
//! chunk grid, and rolling the room's loot and hunter spawns. It is not wired into the chunk generator yet (see hooks).
//!
//! Layout glyphs: `#` solid, `.` dry, `~` water, `=` bridge. A `.`/`~` on the room's border is a doorway.
//! RNG text: `<world>:SEWER-ROOMS:<chunk id>` (picking), `...:LOOT:<room>` and `...:HUNTERS:<room>`.

use std::sync::OnceLock;

use serde_json::Value;

use crate::world::rng::Rng;
use crate::world::route::layout::V2;

use super::core::{BRIDGE, CHUNK, DRY, SOLID, WATER};

const DATA: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/last_route/data/regions/sewer_rooms.json"));

#[derive(Clone, Debug, PartialEq)]
pub struct RoomProp {
    pub prop_type: String,
    /// The sewer prop type the room prop becomes (`prop_types` in the data; the room type itself when unmapped).
    pub kind: &'static str,
    pub at: V2,
    pub yaw: f64,
    pub solid: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LootRule {
    pub chance: f64,
    pub max: usize,
    pub cells: Vec<V2>,
    pub items: Vec<(String, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HunterRule {
    pub chance_min: f64,
    pub chance_max: f64,
    pub max: usize,
    pub min_distance: f64,
    pub cells: Vec<V2>,
    pub prefer: Vec<String>,
    pub avoid: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoomDef {
    pub id: String,
    pub weight: f64,
    pub w: i32,
    pub h: i32,
    /// Grid codes, row-major (`rows[y][x]`), using the `core` codes.
    pub rows: Vec<Vec<u8>>,
    pub props: Vec<RoomProp>,
    pub loot: LootRule,
    pub hunters: HunterRule,
}

#[derive(Clone, Debug)]
pub struct Rooms {
    pub chance: f64,
    pub margin: i32,
    pub max_tries: usize,
    pub rooms: Vec<RoomDef>,
}

/// A room put into a chunk: the definition and the chunk cell of its top-left corner.
#[derive(Clone, Copy, Debug)]
pub struct Placed<'a> {
    pub def: &'a RoomDef,
    pub origin: V2,
}

fn v2(v: &Value) -> Option<V2> {
    let a = v.as_array()?;
    Some(V2::new(a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32))
}

fn cells(v: &Value) -> Vec<V2> {
    v.as_array().map(|a| a.iter().filter_map(v2).collect()).unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn glyph(c: char) -> Option<u8> {
    match c {
        '#' => Some(SOLID),
        '.' => Some(DRY),
        '~' => Some(WATER),
        '=' => Some(BRIDGE),
        _ => None,
    }
}

/// Parse the room data. `Err` names the first problem.
pub fn parse(text: &str) -> Result<Rooms, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let place = &v["place"];
    let kind_of = |t: &str| -> &'static str {
        Box::leak(v["prop_types"][t].as_str().unwrap_or(t).to_string().into_boxed_str())
    };
    let mut rooms = Vec::new();
    for r in v["rooms"].as_array().ok_or("rooms missing")? {
        let id = r["id"].as_str().ok_or("room without id")?.to_string();
        let mut rows = Vec::new();
        for line in r["layout"].as_array().ok_or_else(|| format!("{id}: layout missing"))? {
            let row: Option<Vec<u8>> = line.as_str().unwrap_or("").chars().map(glyph).collect();
            rows.push(row.ok_or_else(|| format!("{id}: bad glyph"))?);
        }
        let w = rows.first().map_or(0, |r| r.len()) as i32;
        if w == 0 || rows.iter().any(|r| r.len() as i32 != w) {
            return Err(format!("{id}: rows differ in width"));
        }
        let props = r["props"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let prop_type = p["type"].as_str()?.to_string();
                        Some(RoomProp {
                            kind: kind_of(&prop_type),
                            prop_type,
                            at: v2(&p["at"])?,
                            yaw: p["yaw"].as_f64().unwrap_or(0.0),
                            solid: p["solid"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let l = &r["loot"];
        let items = l["items"]
            .as_array()
            .map(|a| a.iter().filter_map(|i| Some((i["item"].as_str()?.to_string(), i["weight"].as_f64().unwrap_or(1.0)))).collect())
            .unwrap_or_default();
        let h = &r["hunters"];
        rooms.push(RoomDef {
            id,
            weight: r["weight"].as_f64().unwrap_or(1.0),
            w,
            h: rows.len() as i32,
            rows,
            props,
            loot: LootRule {
                chance: l["chance"].as_f64().unwrap_or(0.0),
                max: l["max"].as_u64().unwrap_or(0) as usize,
                cells: cells(&l["cells"]),
                items,
            },
            hunters: HunterRule {
                chance_min: h["chance_min"].as_f64().unwrap_or(0.0),
                chance_max: h["chance_max"].as_f64().unwrap_or(0.0),
                max: h["max"].as_u64().unwrap_or(0) as usize,
                min_distance: h["min_distance"].as_f64().unwrap_or(4.0),
                cells: cells(&h["cells"]),
                prefer: strings(&h["prefer"]),
                avoid: strings(&h["avoid"]),
            },
        });
    }
    Ok(Rooms {
        chance: place["chance"].as_f64().unwrap_or(0.0),
        margin: place["margin"].as_i64().unwrap_or(3) as i32,
        max_tries: place["max_tries"].as_u64().unwrap_or(40) as usize,
        rooms,
    })
}

/// The built-in rooms, parsed once.
pub fn rooms() -> &'static Rooms {
    static CELL: OnceLock<Rooms> = OnceLock::new();
    CELL.get_or_init(|| parse(DATA).expect("sewer_rooms.json parses"))
}

/// Whether the room of this size, put at (x, z), is open on a doorway: some border cell that is not solid has a
/// non-solid neighbour outside the room in `grid`. `grid` is indexed `[z][x]`.
pub fn connects(def: &RoomDef, grid: &[[u8; CHUNK as usize]; CHUNK as usize], x: i32, z: i32) -> bool {
    let open = |cx: i32, cz: i32| {
        cx >= 0 && cz >= 0 && cx < CHUNK && cz < CHUNK && grid[cz as usize][cx as usize] != SOLID
    };
    for ry in 0..def.h {
        for rx in 0..def.w {
            let edge = rx == 0 || ry == 0 || rx == def.w - 1 || ry == def.h - 1;
            if !edge || def.code_at(rx, ry) == Some(SOLID) {
                continue;
            }
            let out = [(-1, 0, rx == 0), (1, 0, rx == def.w - 1), (0, -1, ry == 0), (0, 1, ry == def.h - 1)];
            if out.iter().any(|(dx, dz, on)| *on && open(x + rx + dx, z + ry + dz)) {
                return true;
            }
        }
    }
    false
}

/// Whether a room rectangle may go into a chunk grid: every cell under it is solid, no `keep_clear` cell lies in it
/// or the ring round it, and some room of this size opens onto existing floor (a doorway that leads somewhere).
pub fn spot_is_free(
    rooms: &Rooms,
    grid: &[[u8; CHUNK as usize]; CHUNK as usize],
    keep_clear: &[V2],
    x: i32,
    z: i32,
    w: i32,
    h: i32,
) -> bool {
    for cz in z..z + h {
        for cx in x..x + w {
            if grid[cz as usize][cx as usize] != SOLID {
                return false;
            }
        }
    }
    if keep_clear.iter().any(|c| c.x >= x - 1 && c.x <= x + w && c.y >= z - 1 && c.y <= z + h) {
        return false;
    }
    rooms.rooms.iter().any(|d| d.w == w && d.h == h && connects(d, grid, x, z))
}

impl RoomDef {
    pub fn code_at(&self, x: i32, y: i32) -> Option<u8> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        Some(self.rows[y as usize][x as usize])
    }
}

/// Pick a room and a spot for one sewer chunk. `free(x, y, w, h)` says whether the rectangle may be used.
/// Draws: chance, weighted room, then two draws (x, z) per try.
pub fn pick<'a>(
    rooms: &'a Rooms,
    world: &str,
    chunk_id: &str,
    free: &dyn Fn(i32, i32, i32, i32) -> bool,
) -> Option<Placed<'a>> {
    if rooms.rooms.is_empty() {
        return None;
    }
    let mut rng = Rng::new(&format!("{world}:SEWER-ROOMS:{chunk_id}"));
    if rng.next_float() >= rooms.chance {
        return None;
    }
    let total: f64 = rooms.rooms.iter().map(|r| r.weight.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut roll = rng.next_float() * total;
    let mut def = &rooms.rooms[0];
    for r in &rooms.rooms {
        def = r;
        roll -= r.weight.max(0.0);
        if roll < 0.0 {
            break;
        }
    }
    let lo = rooms.margin;
    let (xs, zs) = (CHUNK - lo - def.w - lo + 1, CHUNK - lo - def.h - lo + 1);
    if xs <= 0 || zs <= 0 {
        return None;
    }
    for _ in 0..rooms.max_tries {
        let x = lo + rng.next_index(xs as usize) as i32;
        let z = lo + rng.next_index(zs as usize) as i32;
        if free(x, z, def.w, def.h) {
            return Some(Placed { def, origin: V2::new(x, z) });
        }
    }
    None
}

impl Placed<'_> {
    /// Write the layout into a chunk grid (`grid[z][x]`).
    pub fn stamp(&self, grid: &mut [[u8; CHUNK as usize]; CHUNK as usize]) {
        for (y, row) in self.def.rows.iter().enumerate() {
            for (x, code) in row.iter().enumerate() {
                grid[self.origin.y as usize + y][self.origin.x as usize + x] = *code;
            }
        }
    }

    fn world_cell(&self, c: V2) -> V2 {
        V2::new(self.origin.x + c.x, self.origin.y + c.y)
    }

    /// The props in chunk cells.
    pub fn props(&self) -> Vec<RoomProp> {
        self.def.props.iter().map(|p| RoomProp { at: self.world_cell(p.at), ..p.clone() }).collect()
    }

    /// Loot as (item, chunk cell). Draws: chance, then per pickup a cell and an item.
    pub fn loot(&self, world: &str, chunk_id: &str) -> Vec<(String, V2)> {
        let rule = &self.def.loot;
        let mut rng = Rng::new(&format!("{world}:SEWER-ROOMS:{chunk_id}:LOOT:{}", self.def.id));
        let mut out = Vec::new();
        if rule.items.is_empty() || rng.next_float() >= rule.chance {
            return out;
        }
        let mut cells = rule.cells.clone();
        let total: f64 = rule.items.iter().map(|i| i.1.max(0.0)).sum();
        while out.len() < rule.max && !cells.is_empty() {
            let cell = cells.remove(rng.next_index(cells.len()));
            let mut roll = rng.next_float() * total;
            let mut item = &rule.items[0].0;
            for (name, w) in &rule.items {
                item = name;
                roll -= w.max(0.0);
                if roll < 0.0 {
                    break;
                }
            }
            out.push((item.clone(), self.world_cell(cell)));
        }
        out
    }

    /// Hunter spawns as (chunk cell, preferred ids, avoided ids). `danger` is 0..1 and lerps the chance.
    pub fn hunter_spawns(&self, world: &str, chunk_id: &str, danger: f64) -> Vec<(V2, Vec<String>, Vec<String>)> {
        let rule = &self.def.hunters;
        let mut rng = Rng::new(&format!("{world}:SEWER-ROOMS:{chunk_id}:HUNTERS:{}", self.def.id));
        let chance = rule.chance_min + (rule.chance_max - rule.chance_min) * danger.clamp(0.0, 1.0);
        let mut out: Vec<(V2, Vec<String>, Vec<String>)> = Vec::new();
        if rng.next_float() >= chance {
            return out;
        }
        let mut cells = rule.cells.clone();
        while out.len() < rule.max && !cells.is_empty() {
            let cell = cells.remove(rng.next_index(cells.len()));
            out.push((self.world_cell(cell), rule.prefer.clone(), rule.avoid.clone()));
            cells.retain(|c| {
                let (dx, dz) = ((c.x - cell.x) as f64, (c.y - cell.y) as f64);
                (dx * dx + dz * dz).sqrt() >= rule.min_distance
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_free(_: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }

    fn picks(rs: &Rooms) -> Vec<(String, V2)> {
        (0..50).filter_map(|n| pick(rs, "W", &format!("c{n}"), &all_free).map(|p| (p.def.id.clone(), p.origin))).collect()
    }

    #[test]
    fn the_five_rooms_load() {
        let ids: Vec<&str> = rooms().rooms.iter().map(|r| r.id.as_str()).collect();
        for want in ["pump_room", "valve_hall", "junction_chamber", "overflow_chamber", "storage_alcove"] {
            assert!(ids.contains(&want), "{want} missing");
        }
    }

    #[test]
    fn props_loot_and_hunter_cells_sit_on_dry_floor_inside_the_room() {
        for r in &rooms().rooms {
            let dry = |c: &V2| r.code_at(c.x, c.y) == Some(DRY);
            assert!(r.props.iter().all(|p| dry(&p.at)), "{} prop off the floor", r.id);
            assert!(r.loot.cells.iter().all(dry), "{} loot cell", r.id);
            assert!(r.hunters.cells.iter().all(dry), "{} hunter cell", r.id);
            assert!(!r.loot.items.is_empty() && r.loot.max > 0, "{} has loot", r.id);
        }
    }

    #[test]
    fn every_room_has_a_doorway_on_its_border() {
        for r in &rooms().rooms {
            let mut door = false;
            for y in 0..r.h {
                for x in 0..r.w {
                    let edge = x == 0 || y == 0 || x == r.w - 1 || y == r.h - 1;
                    if edge && r.code_at(x, y) != Some(SOLID) {
                        door = true;
                    }
                }
            }
            assert!(door, "{} is sealed", r.id);
        }
    }

    #[test]
    fn bad_data_is_rejected() {
        assert!(parse("{\"rooms\":[{\"id\":\"x\",\"layout\":[\"##\",\"#\"]}]}").is_err());
        assert!(parse("{\"rooms\":[{\"id\":\"x\",\"layout\":[\"#?\"]}]}").is_err());
    }

    #[test]
    fn a_placed_room_stays_inside_the_margin_and_stamps_its_layout() {
        let rs = rooms();
        let mut found = 0;
        for n in 0..200 {
            let Some(p) = pick(rs, "W", &format!("c{n}"), &all_free) else { continue };
            found += 1;
            assert!(p.origin.x >= rs.margin && p.origin.x + p.def.w <= CHUNK - rs.margin);
            assert!(p.origin.y >= rs.margin && p.origin.y + p.def.h <= CHUNK - rs.margin);
            let mut grid = [[SOLID; CHUNK as usize]; CHUNK as usize];
            p.stamp(&mut grid);
            let c = p.def.loot.cells[0];
            assert_eq!(grid[(p.origin.y + c.y) as usize][(p.origin.x + c.x) as usize], DRY);
        }
        assert!(found > 40 && found < 160, "chance applies: {found}/200");
    }

    #[test]
    fn picking_is_deterministic_and_respects_free() {
        let rs = rooms();
        assert_eq!(picks(rs), picks(rs));
        assert!((0..50).all(|n| pick(rs, "W", &format!("c{n}"), &|_, _, _, _| false).is_none()));
    }

    #[test]
    fn data_drives_the_room_mix() {
        let mut one = parse(DATA).unwrap();
        one.chance = 1.0;
        one.rooms.retain(|r| r.id == "storage_alcove");
        let got = picks(&one);
        assert!(!got.is_empty() && got.iter().all(|p| p.0 == "storage_alcove"));
    }

    #[test]
    fn loot_is_bounded_distinct_and_in_the_room() {
        for r in &rooms().rooms {
            let p = Placed { def: r, origin: V2::new(10, 10) };
            for n in 0..40 {
                let loot = p.loot("W", &format!("c{n}"));
                assert!(loot.len() <= r.loot.max);
                for (i, (item, at)) in loot.iter().enumerate() {
                    assert!(r.loot.items.iter().any(|x| &x.0 == item));
                    assert_eq!(r.code_at(at.x - 10, at.y - 10), Some(DRY));
                    assert!(loot.iter().skip(i + 1).all(|o| o.1 != *at));
                }
            }
        }
    }

    #[test]
    fn danger_raises_hunter_spawns_and_the_cap_holds() {
        let r = rooms().rooms.iter().find(|r| r.id == "valve_hall").unwrap();
        let p = Placed { def: r, origin: V2::new(5, 5) };
        let count = |d: f64| (0..300).map(|n| p.hunter_spawns("W", &format!("c{n}"), d).len()).sum::<usize>();
        assert!(count(1.0) > count(0.0));
        for n in 0..100 {
            let s = p.hunter_spawns("W", &format!("c{n}"), 1.0);
            assert!(s.len() <= r.hunters.max);
            if let [a, b] = &s[..] {
                let (dx, dz) = ((a.0.x - b.0.x) as f64, (a.0.y - b.0.y) as f64);
                assert!((dx * dx + dz * dz).sqrt() >= r.hunters.min_distance);
            }
        }
    }

    #[test]
    fn the_storage_alcove_keeps_hunters_rare_and_avoids_the_shy_ones() {
        let r = rooms().rooms.iter().find(|r| r.id == "storage_alcove").unwrap();
        assert!(r.hunters.chance_max <= 0.2);
        assert!(r.hunters.avoid.iter().any(|a| a == "kathy"));
    }
}
