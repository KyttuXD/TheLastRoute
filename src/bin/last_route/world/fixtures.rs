//! The floor fixtures some hunters depend on, each on its own seeded stream like the
//! original's on-demand placements: the ring of four vent grates (Verrine's, one per room, "<seed>:VENT-NETWORK") and five
//! coins for Zellia's store ("<seed>:COINS", one per random room centre). The music box (Glyra's, 1-4 cells
//! from the start) is a collectable pickup, not a fixture: `place_with_box` returns its cell for the caller to
//! put on the floor. Exact cells are estimated where the original places them in presentation code.

use super::door_clearance;
use super::facility::{Cell, Facility};
use super::rng::Rng;
use std::collections::HashSet;

#[derive(Clone, Default)]
pub struct Fixtures {
    pub vents: Vec<Cell>,
    pub coins: Vec<Cell>,
    /// Hiding spots of the data kinds (wardrobe, stall, freezer ...) on top of `Facility.hiding`'s lockers.
    pub extra_hiding: Vec<ExtraHide>,
}

fn taken(f: &Facility) -> HashSet<Cell> {
    let mut t: HashSet<Cell> = HashSet::from([f.start, f.exit]);
    t.extend(f.relays.iter());
    t.extend(f.batteries.iter());
    t.extend(f.hiding.iter().map(|h| h.cell));
    t.extend(f.room_lights.iter().map(|l| l.switch_cell));
    t.extend(f.pickups.iter().map(|p| p.0));
    t
}

fn near_start(
    f: &Facility,
    taken: &HashSet<Cell>,
    rng: &mut Rng,
    min: f32,
    max: f32,
) -> Option<Cell> {
    let cands: Vec<Cell> = f
        .floor_cells()
        .into_iter()
        .filter(|c| {
            let d = c.len_to(f.start);
            d >= min
                && d <= max
                && !taken.contains(c)
                && !f.is_water(*c)
                && f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.23)
        })
        .collect();
    if cands.is_empty() {
        None
    } else {
        Some(cands[rng.next_index(cands.len())])
    }
}

pub fn place(f: &Facility) -> Fixtures {
    place_with_box(f).0
}

/// The fixtures of a finite floor, and the cell of its music box (`<seed>:MUSIC-BOX`, 1.5-4 cells from the start).
/// The box is a collectable: the caller adds it to the floor as a `Pickup::MusicBox` (game/mod.rs, infinite.rs). Its
/// cell is still reserved here, so the vents and coins keep the cells they had as a fixture.
pub fn place_with_box(f: &Facility) -> (Fixtures, Option<Cell>) {
    let mut t = taken(f);
    // The retired heater station's cell (archive/heater-stations/) stays reserved, though nothing is placed on it, so the
    // music box, the vents and the coins keep the exact cells they had when the station existed: a seed's floor does not
    // move, and the host and a guest on either side of this change place the same fixtures.
    t.extend(near_start(
        f,
        &t,
        &mut Rng::new(&format!("{}:HEATER", f.seed_text)),
        3.0,
        7.0,
    ));
    let music_box = near_start(
        f,
        &t,
        &mut Rng::new(&format!("{}:MUSIC-BOX", f.seed_text)),
        1.5,
        4.0,
    );
    t.extend(music_box);

    // no vent grate in a door's way (world::door_clearance)
    let door_clear = door_clearance::Clearance::of(f);
    let mut vents = vec![];
    let mut rng = Rng::new(&format!("{}:VENT-NETWORK", f.seed_text));
    let mut rooms: Vec<usize> = (0..f.rooms.len())
        .filter(|i| !f.rooms[*i].lobby())
        .collect();
    while vents.len() < 4 && !rooms.is_empty() {
        let ri = rooms.remove(rng.next_index(rooms.len()));
        let r = &f.rooms[ri];
        let mut cells = vec![];
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let c = Cell::new(x, y);
                let backed = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| f.is_wall_cell(x + dx, y + dy));
                if backed && !t.contains(&c) && !f.is_water(c) && !door_clear.blocks(door_clearance::Kind::Vent, c) && f.can_occupy(x as f64 + 0.5, y as f64 + 0.5, 0.23) {
                    cells.push(c);
                }
            }
        }
        if !cells.is_empty() {
            let c = cells[rng.next_index(cells.len())];
            t.insert(c);
            vents.push(c);
        }
    }

    let mut coins = vec![];
    let mut rng = Rng::new(&format!("{}:COINS", f.seed_text));
    // a window with no rooms (a stair or sewer chunk) has nowhere to put a coin
    for _ in 0..5 {
        if f.rooms.is_empty() {
            break;
        }
        let r = &f.rooms[rng.next_index(f.rooms.len())];
        let (cx, cy) = r.center.center();
        let ox = rng.next_f32() * 2.0 - 1.0;
        let oy = rng.next_f32() * 2.0 - 1.0;
        if let Some(p) = f.nearest_open(cx + ox * 1.5, cy + oy * 1.5, 0.23) {
            let c = Cell::new(p.0 as i32, p.1 as i32);
            if !t.contains(&c) && !f.is_water(c) {
                t.insert(c);
                coins.push(c);
            }
        }
    }
    (
        Fixtures {
            vents,
            coins,
            extra_hiding: extra_hiding(f, &t),
        },
        music_box,
    )
}

// -- extra hiding kinds (data/hiding.json) ---------------------------------------------------------------

/// A hiding spot of a data kind: the shape of a `Facility.hiding` entry plus its kind, whose capacity is data.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraHide {
    pub cell: Cell,
    pub facing: (i32, i32),
    pub room: usize,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct HideRule {
    pub kind: String,
    pub regions: Vec<String>,
    pub room_types: Vec<String>,
    pub min: usize,
    pub max: usize,
    pub min_area: i32,
    pub spacing: Option<f32>,
    pub only_if_none: bool,
}

#[derive(Clone, Debug)]
pub struct HideRules {
    pub capacities: Vec<(String, usize)>,
    pub labels: Vec<(String, String)>,
    pub default_spacing: f32,
    pub rules: Vec<HideRule>,
    pub prop_styles: Vec<(String, String)>,
}

impl HideRules {
    pub fn parse(text: &str) -> HideRules {
        let v: serde_json::Value = serde_json::from_str(text).expect("hiding.json");
        let strs = |x: &serde_json::Value| -> Vec<String> {
            x.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        let kinds = v["kinds"].as_object().cloned().unwrap_or_default();
        HideRules {
            capacities: kinds.iter().map(|(k, d)| (k.clone(), d["capacity"].as_u64().unwrap_or(1).max(1) as usize)).collect(),
            labels: kinds.iter().map(|(k, d)| (k.clone(), d["label"].as_str().unwrap_or(k).to_uppercase())).collect(),
            default_spacing: v["default_spacing"].as_f64().unwrap_or(2.5) as f32,
            rules: v["rules"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|r| {
                            let min = r["min"].as_u64().unwrap_or(1) as usize;
                            HideRule {
                                kind: r["kind"].as_str().unwrap_or("closet").to_string(),
                                regions: strs(&r["regions"]),
                                room_types: strs(&r["room_types"]),
                                min,
                                max: (r["max"].as_u64().unwrap_or(1) as usize).max(min),
                                min_area: r["min_area"].as_i64().unwrap_or(0) as i32,
                                spacing: r["spacing"].as_f64().map(|s| s as f32),
                                only_if_none: r["only_if_none"].as_bool().unwrap_or(false),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            prop_styles: v["prop_styles"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, s)| s.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default(),
        }
    }

    /// Bodies a spot of `kind` holds (None when the kind is not in the data; lockers stay in hiding_slots.json).
    pub fn capacity_of(&self, kind: &str) -> Option<usize> {
        self.capacities.iter().find(|(k, _)| k == kind).map(|(_, c)| *c)
    }

    pub fn label_of(&self, kind: &str) -> Option<&str> {
        self.labels.iter().find(|(k, _)| k == kind).map(|(_, l)| l.as_str())
    }

    /// The hiding kind a prop type becomes (route/fixtures.rs), if the data names one.
    pub fn prop_style(&self, prop_type: &str) -> Option<&str> {
        self.prop_styles.iter().find(|(p, _)| p == prop_type).map(|(_, s)| s.as_str())
    }
}

/// The embedded `data/hiding.json`.
pub fn hide_rules() -> &'static HideRules {
    static C: std::sync::OnceLock<HideRules> = std::sync::OnceLock::new();
    C.get_or_init(|| HideRules::parse(include_str!("../data/hiding.json")))
}

/// `extra_hiding_with` over the embedded data.
pub fn extra_hiding(f: &Facility, taken_cells: &HashSet<Cell>) -> Vec<ExtraHide> {
    extra_hiding_with(f, taken_cells, hide_rules())
}

/// Appends the data-kind spots to `f.hiding` after the lockers (so locker indices, which the net layer sends, never move).
pub fn adopt_extra_hiding(f: &mut Facility, extra: &[ExtraHide]) {
    for e in extra {
        f.hiding.push(super::facility::Hiding { cell: e.cell, facing: e.facing, room: Some(e.room), kind: e.kind.clone() });
    }
}

/// Wall-backed, door-clear floor cells of each room get the spots its first matching rule asks for, on their own seeded
/// stream (`<seed>:HIDING-KINDS`) so no other placement moves. `only_if_none` rules apply only to rooms with no locker.
pub fn extra_hiding_with(f: &Facility, taken_cells: &HashSet<Cell>, rules: &HideRules) -> Vec<ExtraHide> {
    let mut out: Vec<ExtraHide> = vec![];
    let mut taken = taken_cells.clone();
    taken.extend(f.hiding.iter().map(|h| h.cell));
    // never on a flight's way: its well, landing, foot and clear margins, and the ring of cells round them
    for s in &f.stairs {
        for c in s.carve_cells() {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    taken.insert(Cell::new(c.x + dx, c.y + dy));
                }
            }
        }
    }
    let door_clear = door_clearance::Clearance::of(f);
    let mut rng = Rng::new(&format!("{}:HIDING-KINDS", f.seed_text));
    for (ri, r) in f.rooms.iter().enumerate() {
        if r.lobby() {
            continue;
        }
        let has_lockers = f.hiding.iter().any(|h| h.room == Some(ri));
        let rule = rules.rules.iter().find(|u| {
            let by_region = u.regions.iter().any(|x| *x == r.region);
            let by_type = u.room_types.iter().any(|x| *x == r.rtype);
            let listed = match (u.regions.is_empty(), u.room_types.is_empty()) {
                (true, true) => true,
                (false, false) => by_region && by_type,
                (false, true) => by_region,
                (true, false) => by_type,
            };
            listed && r.w * r.h >= u.min_area && (!u.only_if_none || !has_lockers)
        });
        let Some(rule) = rule else { continue };
        let spacing = rule.spacing.unwrap_or(rules.default_spacing);
        let mut cands: Vec<Cell> = vec![];
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let c = Cell::new(x, y);
                let walls = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().filter(|(dx, dy)| f.is_wall_cell(x + dx, y + dy)).count();
                if walls == 1
                    && !taken.contains(&c)
                    && !f.is_water(c)
                    && !door_clear.blocks(door_clearance::Kind::Locker, c)
                    && !door_clear.blocks(door_clearance::Kind::Hide, c)
                    && f.can_occupy(x as f64 + 0.5, y as f64 + 0.5, 0.23)
                {
                    cands.push(c);
                }
            }
        }
        let want = rule.min + if rule.max > rule.min { rng.next_index(rule.max - rule.min + 1) } else { 0 };
        let mut placed = 0;
        while placed < want && !cands.is_empty() {
            let c = cands.remove(rng.next_index(cands.len()));
            if out.iter().any(|o| o.room == ri && o.cell.len_to(c) < spacing) {
                continue;
            }
            let wall = [(0, 1), (0, -1), (1, 0), (-1, 0)].into_iter().find(|(dx, dy)| f.is_wall_cell(c.x + dx, c.y + dy)).unwrap_or((0, -1));
            taken.insert(c);
            placed += 1;
            out.push(ExtraHide { cell: c, facing: (-wall.0, -wall.1), room: ri, kind: rule.kind.clone() });
        }
    }
    out
}

#[cfg(test)]
mod hiding_tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn with_region(region: &str) -> Facility {
        let mut f = Facility::generate("HIDE-KINDS-1", &Options::default(), &Catalog::load());
        for r in f.rooms.iter_mut() {
            r.region = region.into();
        }
        f
    }

    #[test]
    fn each_region_places_its_kinds() {
        for (region, kinds) in [
            ("apartment", vec!["wardrobe", "closet"]),
            ("stalls", vec!["stall"]),
            ("offices", vec!["underdesk"]),
            ("lab", vec!["cabinet"]),
            ("cold", vec!["freezer"]),
            ("greenhouse", vec!["planter"]),
            ("prison", vec!["bunk"]),
        ] {
            let f = with_region(region);
            let spots = extra_hiding(&f, &HashSet::new());
            assert!(!spots.is_empty(), "{region} places nothing");
            for s in &spots {
                assert!(kinds.contains(&s.kind.as_str()), "{region} placed {}", s.kind);
                assert!(hide_rules().capacity_of(&s.kind).is_some(), "{} has no data capacity", s.kind);
            }
        }
    }

    #[test]
    fn capacity_comes_from_data() {
        let text = include_str!("../data/hiding.json");
        let before = HideRules::parse(text).capacity_of("freezer").unwrap();
        let changed = HideRules::parse(&text.replace("\"freezer\": { \"capacity\": 2", "\"freezer\": { \"capacity\": 5"));
        assert_ne!(changed.capacity_of("freezer").unwrap(), before);
        assert_eq!(changed.capacity_of("freezer"), Some(5));
        assert_eq!(changed.capacity_of("nonsense"), None);
    }

    #[test]
    fn no_spot_blocks_a_doorway_or_shares_a_cell() {
        for region in ["apartment", "stalls", "prison"] {
            let f = with_region(region);
            let clear = door_clearance::Clearance::of(&f);
            let spots = extra_hiding(&f, &HashSet::new());
            for (i, s) in spots.iter().enumerate() {
                assert!(!clear.blocks(door_clearance::Kind::Locker, s.cell));
                assert!(!clear.blocks(door_clearance::Kind::Hide, s.cell));
                assert!(f.hiding.iter().all(|h| h.cell != s.cell));
                assert!(spots.iter().skip(i + 1).all(|o| o.cell != s.cell));
            }
        }
    }

    #[test]
    fn plain_rooms_get_only_the_fallback_and_placement_is_stable() {
        let f = with_region("maze");
        let a = extra_hiding(&f, &HashSet::new());
        assert_eq!(a, extra_hiding(&f, &HashSet::new()));
        assert!(a.iter().all(|s| s.kind == "closet"));
        assert!(a.iter().all(|s| f.hiding.iter().all(|h| h.room != Some(s.room))));
    }
}
