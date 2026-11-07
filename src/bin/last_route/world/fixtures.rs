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
        },
        music_box,
    )
}
