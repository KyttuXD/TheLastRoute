//! Absolute vertical frame for bodies (docs/STAIRS_CONTINUOUS_DESIGN.md phases 2-3): the floor of level `n` sits at
//! `n * STOREY`, and `ground_y` reports the walkable surface under a point in that frame. The active level plus the
//! resident neighbour levels (`LevelStack`) answer the same queries, so a flight is one continuous space.

use super::player::stair_height_at;
use crate::world::facility::{Cell, Stair};
use crate::world::fixtures::Fixtures;
use crate::world::vertical::{level_from_height, STOREY};
use crate::world::Facility;

/// The absolute y of the floor of `level` (`level * STOREY`).
pub fn floor_y_of(level: i32) -> f32 {
    level as f32 * STOREY
}

/// The level a body at absolute height `y` is on, given the level it was on last (the hysteresis of
/// `vertical::level_from_height`).
pub fn level_at(y: f32, previous: i32) -> i32 {
    level_from_height(y, previous)
}

/// The walkable surface under a point: its absolute height and the level that owns it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub y: f32,
    pub level: i32,
}

/// Highest step a body climbs between surfaces (m): `ground_y` takes the highest surface at most this far above the feet.
pub const STEP_UP: f32 = 0.55;
/// Height of the body the head test uses (m).
const BODY_HEIGHT: f32 = 1.8;

/// One neighbour level kept built beside the active one: it shares the window centre and is drawn at
/// `dy = (level - active) * STOREY`.
pub struct Resident {
    pub level: i32,
    pub center: (i32, i32),
    pub fac: Facility,
    pub fix: Fixtures,
}

impl Resident {
    /// The memory the window holds (the input to the preload memory cap).
    pub fn bytes(&self) -> usize {
        super::endless::window_bytes(&self.fac, &self.fix)
    }
}

/// The neighbour levels held resident beside the active level.
#[derive(Default)]
pub struct LevelStack {
    pub residents: Vec<Resident>,
}

impl LevelStack {
    pub const EMPTY: LevelStack = LevelStack { residents: Vec::new() };

    pub fn get(&self, level: i32) -> Option<&Resident> {
        self.residents.iter().find(|r| r.level == level)
    }

    pub fn has(&self, level: i32) -> bool {
        self.get(level).is_some()
    }

    pub fn take(&mut self, level: i32) -> Option<Resident> {
        let i = self.residents.iter().position(|r| r.level == level)?;
        Some(self.residents.remove(i))
    }

    /// Adds a resident, replacing one of the same level.
    pub fn put(&mut self, r: Resident) {
        self.residents.retain(|x| x.level != r.level);
        self.residents.push(r);
    }

    /// Drops every resident not built for window centre `center`.
    pub fn retain_center(&mut self, center: (i32, i32)) {
        self.residents.retain(|r| r.center == center);
    }

    /// Drops every resident whose level is not in `levels`.
    pub fn retain_levels(&mut self, levels: &[i32]) {
        self.residents.retain(|r| levels.contains(&r.level));
    }

    pub fn levels(&self) -> Vec<i32> {
        self.residents.iter().map(|r| r.level).collect()
    }

    /// The facility of `level`: the active one, else the resident of that level.
    pub fn fac_for<'a>(&'a self, active: &'a Facility, level: i32) -> Option<&'a Facility> {
        if active.level == level {
            Some(active)
        } else {
            self.get(level).map(|r| &r.fac)
        }
    }
}

/// The level whose storey holds absolute height `y_abs`: floor((y + 1e-3) / STOREY).
pub fn owning_level(y_abs: f32) -> i32 {
    ((y_abs + 1e-3) / STOREY).floor() as i32
}

/// Whether `fac` offers a walkable surface at (x, z): a cell that is not a hole, or a flight surface.
fn has_surface(fac: &Facility, x: f32, z: f32) -> bool {
    stair_height_at(fac, (x, z)).is_some() || !fac.is_hole(x.floor() as i32, z.floor() as i32)
}

/// The absolute surface under (x, z) of `fac`'s own level only: a stair tread (`stair_height_at`), else the cell's walk
/// surface (`Facility::ground_height`, the Inferno's noisy floor), else the plain floor.
pub fn ground_y_single(fac: &Facility, x: f32, z: f32, _y_hint: f32) -> Ground {
    Ground { y: floor_y_of(fac.level) + local_ground(fac, x, z), level: fac.level }
}

/// The surface under (x, z) for a body with feet at absolute `y_abs`: the highest surface of the active level or a
/// resident that is at most `STEP_UP` above the feet; else the surface of the level owning the height; else the active
/// level's.
pub fn ground_y(active: &Facility, stack: &LevelStack, x: f32, z: f32, y_abs: f32) -> Ground {
    let mut best: Option<Ground> = None;
    let facs = std::iter::once(active).chain(stack.residents.iter().map(|r| &r.fac));
    for f in facs {
        if !has_surface(f, x, z) {
            continue;
        }
        let g = ground_y_single(f, x, z, y_abs);
        if g.y <= y_abs + STEP_UP && best.map_or(true, |b| g.y > b.y) {
            best = Some(g);
        }
    }
    if let Some(g) = best {
        return g;
    }
    if let Some(f) = stack.fac_for(active, owning_level(y_abs)) {
        if has_surface(f, x, z) {
            return ground_y_single(f, x, z, y_abs);
        }
    }
    ground_y_single(active, x, z, y_abs)
}

/// Whether a body of radius `r` with feet at absolute `y_abs` fits at (x, z). The level owning the feet decides (its
/// walls, props, doors and flight rules); a head reaching the next floor also needs that level's walls clear when it
/// is resident. A level that is not resident falls back to the active facility.
pub fn body_fits_at_y(active: &Facility, stack: &LevelStack, x: f32, z: f32, y_abs: f32, r: f32) -> bool {
    // A drop cell (the void of a fall) is open air down to the level below: with that level not resident there is
    // nothing to land on, so the body is kept out of it.
    if active.is_drop(Cell::new(x.floor() as i32, z.floor() as i32)) && !stack.has(active.level - 1) {
        return false;
    }
    let owner = owning_level(y_abs);
    let own = stack.fac_for(active, owner).unwrap_or(active);
    if !own.body_fits_at_y(x, z, y_abs, r) {
        // A flight descends from the upper floor, so the feet are in the storey below from the first step while the
        // body (the landing cell under its back corners, the flight's own walls) still belongs to the level above:
        // that level's rule decides when it accepts the height (it needs the tread under every corner, so a body in
        // the lower storey away from the flight is never accepted by it).
        return stack
            .fac_for(active, owner + 1)
            .is_some_and(|up| !std::ptr::eq(up, own) && up.body_fits_at_y(x, z, y_abs, r));
    }
    if y_abs + BODY_HEIGHT >= floor_y_of(owner + 1) {
        if let Some(next) = stack.fac_for(active, owner + 1) {
            if !next.can_occupy(x as f64, z as f64, r as f64) {
                return false;
            }
        }
    }
    true
}

/// The neighbour levels worth holding resident at `pos` on `active_level`: the target of each stair within `reach` (a
/// ladder keeps both), plus both sides while the body is on a flight. Only levels `allowed` admits; never the active one.
pub fn levels_to_keep(
    active_level: i32,
    pos: (f32, f32),
    reach: f32,
    stairs: &[Stair],
    on_flight: bool,
    allowed: impl Fn(i32) -> bool,
) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    let add = |t: i32, out: &mut Vec<i32>| {
        if t != active_level && allowed(t) && !out.contains(&t) {
            out.push(t);
        }
    };
    for s in stairs {
        let c = s.cell.center();
        if (c.0 - pos.0).hypot(c.1 - pos.1) > reach {
            continue;
        }
        if s.is_ladder() {
            add(active_level - 1, &mut out);
            add(active_level + 1, &mut out);
        } else {
            add(active_level + s.dir, &mut out);
        }
    }
    if on_flight {
        add(active_level - 1, &mut out);
        add(active_level + 1, &mut out);
    }
    out
}

/// The surface under (x, z) measured from its own level's floor: a stair tread, else the Inferno's noisy floor or the
/// cell's walk surface, else 0.
/// Callers that work relative to the floor use this instead of subtracting the floor from `ground_y`, which loses
/// precision (a flight on level -1 would come out as 1.8e-7 instead of the tread height).
pub fn local_ground(fac: &Facility, x: f32, z: f32) -> f32 {
    stair_height_at(fac, (x, z)).unwrap_or_else(|| fac.ground_height(x, z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::{LinkKind, Options};

    #[test]
    fn floor_y_is_the_level_times_the_storey() {
        assert_eq!(floor_y_of(0), 0.0);
        assert_eq!(floor_y_of(-1), -STOREY);
        assert_eq!(floor_y_of(2), 2.0 * STOREY);
    }

    #[test]
    fn level_at_matches_level_from_height_including_its_hysteresis() {
        for previous in [-3, -1, 0, 1] {
            let mut y = -20.0_f32;
            while y < 10.0 {
                assert_eq!(level_at(y, previous), level_from_height(y, previous), "y {y} prev {previous}");
                y += 0.05;
            }
        }
        // it stays put near its own floor and moves only past the band
        assert_eq!(level_at(floor_y_of(-1) + 0.5, -1), -1);
        assert_eq!(level_at(floor_y_of(-1), 0), -1, "a body at a floor reads that level from far away");
    }

    #[test]
    fn ground_on_a_plain_floor_is_the_levels_floor() {
        let cat = Catalog::load();
        for level in [0, -1] {
            let mut f = Facility::generate("GROUND-1", &Options::default(), &cat);
            f.level = level;
            let c = f.start;
            let g = ground_y(&f, &LevelStack::EMPTY, c.x as f32 + 0.5, c.y as f32 + 0.5, floor_y_of(level));
            assert_eq!(g, Ground { y: floor_y_of(level), level });
        }
    }

    #[test]
    fn ground_on_a_stair_flight_cell_is_the_tread() {
        // a real window of the Infinite route (the path the game uses), not the single-floor generator, which has no flights
        let content = crate::content::Content::load();
        let cat = Catalog::load();
        for n in 0..40 {
            let text = format!("ROOM-TEST-{n}");
            let g = crate::game::Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &text, -1, &content, &cat);
            let f = &g.fac;
            let Some(s) = f.stairs.iter().copied().find(|s| matches!(s.kind, LinkKind::Stairs)) else { continue };
            let path = s.walk_path(f.level, 0.25);
            let p = path[path.len() / 2];
            let tread = stair_height_at(f, p).expect("on the flight");
            assert!(tread != 0.0, "a flight cell is off the floor");
            let ground = ground_y(f, &LevelStack::EMPTY, p.0, p.1, floor_y_of(f.level));
            assert_eq!(ground.y, floor_y_of(f.level) + tread);
            assert_eq!(ground.level, f.level);
            assert_eq!(local_ground(f, p.0, p.1), tread, "the local height is the tread exactly, with no floor round trip");
            return;
        }
        panic!("no test window has a stair flight");
    }

    /// A flight window on level -1 (the lower level) and its level-0 neighbour at the same centre.
    fn flight_pair() -> (crate::game::Game, Resident, Stair) {
        let content = crate::content::Content::load();
        let cat = Catalog::load();
        for n in 0..60 {
            let text = format!("ROOM-STACK-{n}");
            let g = crate::game::Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &text, -1, &content, &cat);
            let Some(s) = g.fac.stairs.iter().copied().find(|s| matches!(s.kind, LinkKind::Stairs)) else { continue };
            let inf = g.inf.as_ref().expect("infinite run");
            let (seed, center) = (inf.seed.clone(), inf.center);
            let mut cache = std::collections::HashMap::new();
            let (fac, fix) = crate::world::infinite::window(&seed, center, 0, &cat, &content.loot, &mut cache);
            return (g, Resident { level: 0, center, fac, fix }, s);
        }
        panic!("no test window has a stair flight");
    }

    fn stack_of(r: Resident) -> LevelStack {
        let mut st = LevelStack::default();
        st.put(r);
        st
    }

    #[test]
    fn stack_ground_y_picks_the_right_floor() {
        let (g, up, s) = flight_pair();
        let stack = stack_of(up);
        let path = s.walk_path(g.fac.level, 0.25);
        // at the foot (B) with the feet on the lower floor: the lower tread, not the upper floor 5 m higher
        let b = *path.last().unwrap();
        let low = ground_y(&g.fac, &stack, b.0, b.1, floor_y_of(-1));
        assert_eq!(low.level, -1);
        assert!((low.y - (floor_y_of(-1) + local_ground(&g.fac, b.0, b.1))).abs() < 1e-4);
        assert!(low.y < floor_y_of(0) - 1.0);
        // at landing A with the feet at the upper floor: the upper floor
        let a = path[0];
        let high = ground_y(&g.fac, &stack, a.0, a.1, floor_y_of(0));
        assert_eq!(high.level, 0, "the landing belongs to the upper level");
        assert!((high.y - floor_y_of(0)).abs() < 0.3, "{high:?}");
    }

    #[test]
    fn body_on_flight_is_blocked_by_lower_level_walls_below_the_landing() {
        let (g, up, _s) = flight_pair();
        let stack = stack_of(up);
        let upper = &stack.get(0).unwrap().fac;
        let r = 0.3;
        let mut found = false;
        for y in 0..g.fac.rows {
            for x in 0..g.fac.cols {
                let (px, pz) = (x as f32 + 0.5, y as f32 + 0.5);
                if g.fac.is_wall_cell(x, y) && upper.body_fits_at_y(px, pz, floor_y_of(0), r) {
                    // feet 1 m above the lower floor: the lower level owns the height and its wall blocks
                    assert!(!body_fits_at_y(&g.fac, &stack, px, pz, floor_y_of(-1) + 1.0, r));
                    // feet on the upper floor: the upper level owns the height and is open
                    assert!(body_fits_at_y(&g.fac, &stack, px, pz, floor_y_of(0), r));
                    found = true;
                }
            }
        }
        assert!(found, "a lower wall under an open upper cell exists in the window");
    }

    #[test]
    fn body_cannot_enter_a_drop_without_the_level_below() {
        let content = crate::content::Content::load();
        let cat = Catalog::load();
        for n in 0..200 {
            let text = format!("ROOM-DROP-{n}");
            let g = crate::game::Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &text, -1, &content, &cat);
            let Some(c) = g.fac.drop_cells().first().copied() else { continue };
            let (x, z) = c.center();
            let y = floor_y_of(g.fac.level);
            assert!(
                !body_fits_at_y(&g.fac, &LevelStack::EMPTY, x, z, y, 0.3),
                "with nothing below, the drop cell {c:?} is shut"
            );
            // the level below resident: the guard no longer shuts the cell (the fall's own rule decides)
            let center = g.inf.as_ref().expect("infinite run").center;
            let mut st = LevelStack::default();
            st.put(resident(g.fac.level - 1, center));
            assert!(body_fits_at_y(&g.fac, &st, x, z, y, 0.3), "with the level below resident the drop cell is entered");
            return;
        }
        panic!("no test window on level -1 has a drop cell");
    }

    #[test]
    fn owning_level_boundaries() {
        assert_eq!(owning_level(0.0), 0);
        assert_eq!(owning_level(4.99), 0);
        assert_eq!(owning_level(5.0), 1);
        assert_eq!(owning_level(-0.001), 0);
        assert_eq!(owning_level(-0.5), -1);
        assert_eq!(owning_level(-5.0), -1);
        assert_eq!(owning_level(-5.01), -2);
    }

    fn resident(level: i32, center: (i32, i32)) -> Resident {
        let cat = Catalog::load();
        let mut fac = Facility::generate("STACK-RES", &Options::default(), &cat);
        fac.level = level;
        Resident { level, center, fac, fix: Fixtures::default() }
    }

    #[test]
    fn retain_center_drops_stale_residents() {
        let mut st = LevelStack::default();
        st.put(resident(0, (1, 1)));
        st.put(resident(-2, (2, 2)));
        st.retain_center((1, 1));
        assert_eq!(st.levels(), vec![0]);
        st.retain_levels(&[5]);
        assert!(st.residents.is_empty());
    }

    #[test]
    fn put_replaces_same_level() {
        let mut st = LevelStack::default();
        st.put(resident(1, (0, 0)));
        st.put(resident(1, (3, 3)));
        assert_eq!(st.residents.len(), 1);
        assert_eq!(st.get(1).unwrap().center, (3, 3));
        assert!(st.has(1) && !st.has(0));
        assert!(st.take(1).is_some() && st.take(1).is_none());
    }

    #[test]
    fn levels_to_keep_covers_link_and_flight() {
        let (g, _up, s) = flight_pair();
        let c = s.cell.center();
        let near = levels_to_keep(-1, c, 3.0, &g.fac.stairs, false, |_| true);
        assert!(near.contains(&(-1 + s.dir)), "the flight's target is kept");
        assert!(!near.contains(&-1), "the active level is never listed");
        let far = levels_to_keep(-1, (-1000.0, -1000.0), 3.0, &g.fac.stairs, false, |_| true);
        assert!(far.is_empty());
        let on = levels_to_keep(-1, (-1000.0, -1000.0), 3.0, &g.fac.stairs, true, |_| true);
        assert_eq!(on, vec![-2, 0], "both sides while on a flight");
        let gated = levels_to_keep(-1, (-1000.0, -1000.0), 3.0, &g.fac.stairs, true, |l| l <= -1);
        assert_eq!(gated, vec![-2]);
    }
}
