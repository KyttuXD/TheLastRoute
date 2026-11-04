//! Corner peeking (game/hunters/ai/hunter_peek.gd; KT: "make them peek from a corner, and either run/chase if they
//! see something"). A searching or investigating hunter whose route turns at a real corner may stop in the cell before
//! the turn, lean out past the edge and look down the hall. Sight during the lean is taken from the leaned-out head
//! (`Brain::sight_eye`), so she only sees what that head could see.
//!
//! Geometry, in grid cells (1 cell = 1 m). The route goes A -> C -> C + d2, turning from d1 = C - A to d2 at C, and the
//! inner corner A + d2 is solid. She stands at `body()` (behind the edge plane between A and C by more than her radius)
//! and her eye is at `head()` (`head_past` m past that plane).
//!
//! Styles: "burst" charges once the reaction time has passed; "ambush" ducks back behind the corner, waits for the
//! runner to come round (a chase the moment she sees them from cover) and then investigates where she saw them.
//!
//! The numbers live in data/peek.json (`lr.peek/1`), parsed once; the brain side is in `brain.rs`. The hard cap per
//! hunter (`max_per_hunter`) is the Rust port's addition: the original has only the cooldown.

use crate::config::Difficulty;
use crate::world::{Cell, Facility};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Burst,
    Ambush,
}

/// Per hunter: chance a qualifying corner gets a peek, look seconds, style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub chance: f32,
    pub look: f32,
    pub style: Style,
}

/// Per difficulty: chance scale, seconds from first sight to the burst, and how long an ambusher waits in cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub chance: f32,
    pub react: f32,
    pub ambush: f32,
}

#[derive(Clone, Debug)]
pub struct Rules {
    /// Body centre to the edge plane (HUNTER_RADIUS is 0.33).
    pub body_behind: f32,
    /// Leaned head past the edge plane.
    pub head_past: f32,
    /// Route cells ahead checked for a turn.
    pub lookahead: usize,
    /// Seconds before the same hunter considers another corner.
    pub cooldown: f32,
    /// Seconds to slide into the lean spot before the look starts.
    pub settle: f32,
    /// Peeks one hunter makes in her life (a floor), so no hunter is ever stuck peeking.
    pub max_per_hunter: u32,
    default: Tuning,
    never: Vec<String>,
    hunters: HashMap<String, Tuning>,
    difficulty: HashMap<String, Scale>,
}

const STANDARD: Scale = Scale { chance: 1.0, react: 0.45, ambush: 2.5 };

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            body_behind: 0.42,
            head_past: 0.28,
            lookahead: 4,
            cooldown: 6.0,
            settle: 0.25,
            max_per_hunter: 12,
            default: Tuning { chance: 0.35, look: 1.4, style: Style::Burst },
            never: vec![],
            hunters: HashMap::new(),
            difficulty: HashMap::new(),
        }
    }
}

fn num(v: &Value, key: &str, lo: f32, hi: f32, into: &mut f32) {
    if let Some(x) = v[key].as_f64() {
        *into = (x as f32).clamp(lo, hi);
    }
}

fn tuning_of(v: &Value, base: Tuning) -> Tuning {
    let mut t = base;
    num(v, "chance", 0.0, 1.0, &mut t.chance);
    num(v, "look", 0.0, 10.0, &mut t.look);
    match v["style"].as_str() {
        Some("ambush") => t.style = Style::Ambush,
        Some("burst") => t.style = Style::Burst,
        _ => {}
    }
    t
}

impl Rules {
    pub fn parse(v: &Value) -> Rules {
        let mut r = Rules::default();
        let g = &v["geometry"];
        num(g, "body_behind", 0.0, 0.5, &mut r.body_behind);
        num(g, "head_past", 0.0, 0.5, &mut r.head_past);
        num(g, "cooldown", 0.0, 120.0, &mut r.cooldown);
        num(g, "settle", 0.0, 5.0, &mut r.settle);
        if let Some(n) = g["lookahead"].as_i64() {
            r.lookahead = n.clamp(1, 12) as usize;
        }
        if let Some(n) = g["max_per_hunter"].as_i64() {
            r.max_per_hunter = n.clamp(0, 1000) as u32;
        }
        r.default = tuning_of(&v["default"], r.default);
        r.never = v["never"]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if let Some(m) = v["hunters"].as_object() {
            for (id, t) in m {
                r.hunters.insert(id.clone(), tuning_of(t, r.default));
            }
        }
        if let Some(m) = v["difficulty"].as_object() {
            for (id, s) in m {
                let mut sc = STANDARD;
                num(s, "chance", 0.0, 4.0, &mut sc.chance);
                num(s, "react", 0.0, 10.0, &mut sc.react);
                num(s, "ambush", 0.0, 30.0, &mut sc.ambush);
                r.difficulty.insert(id.clone(), sc);
            }
        }
        r
    }

    /// HunterPeek.tuning: the hunters whose own rules drive their movement (objective, dormant, stationary) never peek.
    pub fn tuning(&self, hunter_id: &str) -> Tuning {
        if self.never.iter().any(|n| n == hunter_id) {
            return Tuning { chance: 0.0, look: 0.0, style: Style::Burst };
        }
        self.hunters.get(hunter_id).copied().unwrap_or(self.default)
    }

    /// HunterPeek.difficulty: an id with no row (the port's "relaxed") uses "standard", as the original does.
    pub fn scale(&self, diff: Difficulty) -> Scale {
        let standard = self.difficulty.get("standard").copied().unwrap_or(STANDARD);
        self.difficulty.get(diff.id()).copied().unwrap_or(standard)
    }

    /// HunterPeek.chance: the hunter's chance times the difficulty's scale.
    pub fn chance(&self, hunter_id: &str, diff: Difficulty) -> f32 {
        (self.tuning(hunter_id).chance * self.scale(diff).chance).clamp(0.0, 1.0)
    }
}

/// The embedded rules (data/peek.json); the defaults when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("peek.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .map(|v| Rules::parse(&v))
            .unwrap_or_default()
    })
}

/// A corner on a route: A -> C -> C + d2, turning from d1 to d2 at C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corner {
    pub a: Cell,
    pub c: Cell,
    pub d1: (i32, i32),
    pub d2: (i32, i32),
}

/// The unit step of an axis-aligned offset; (0, 0) for a diagonal or no offset.
fn axis(from: Cell, to: Cell) -> (i32, i32) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dx != 0 && dy != 0 {
        (0, 0)
    } else {
        (dx.signum(), dy.signum())
    }
}

fn step(c: Cell, d: (i32, i32)) -> Cell {
    Cell::new(c.x + d.0, c.y + d.1)
}

/// Open floor at the cell's centre; the one thing `Facility::has_line_of_sight` does not stop at.
fn open(fac: &Facility, cell: Cell) -> bool {
    let (x, y) = cell.center();
    fac.is_floor(x as f64, y as f64)
}

/// The first real corner on `route` (cells, starting at the hunter's own cell) within the lookahead. A real corner:
/// the route turns 90 degrees at C, the inner corner cell A + d2 is a wall (hides_hall: sight stops there) and A, C
/// and the hall cell C + d2 are open. The route may skip cells on a straight run, so only a leg's axis counts; a
/// diagonal leg is no corner. A hall junction counts when the planned route turns there.
pub fn find_corner(route: &[Cell], fac: &Facility) -> Option<Corner> {
    let last = route.len().saturating_sub(1).min(rules().lookahead + 1);
    for i in 1..last {
        let c = route[i];
        let d1 = axis(route[i - 1], c);
        let d2 = axis(c, route[i + 1]);
        if d1 == (0, 0) || d2 == (0, 0) || d1 == d2 || d1 == (-d2.0, -d2.1) {
            continue;
        }
        let a = step(c, (-d1.0, -d1.1));
        if open(fac, step(a, d2)) || !open(fac, a) || !open(fac, c) || !open(fac, step(c, d2)) {
            continue;
        }
        return Some(Corner { a, c, d1, d2 });
    }
    None
}

impl Corner {
    /// Where she stands: behind the edge plane between A and C.
    pub fn body(&self) -> (f32, f32) {
        let (x, y) = self.a.center();
        let k = 0.5 - rules().body_behind;
        (x + self.d1.0 as f32 * k, y + self.d1.1 as f32 * k)
    }

    /// Where her eye is while she leans out.
    pub fn head(&self) -> (f32, f32) {
        let (x, y) = self.a.center();
        let k = 0.5 + rules().head_past;
        (x + self.d1.0 as f32 * k, y + self.d1.1 as f32 * k)
    }

    /// +1 when the corner edge is on her left as she faces down the hall (d2), -1 on her right.
    pub fn side(&self) -> i32 {
        if self.d1.0 * self.d2.1 - self.d1.1 * self.d2.0 > 0 {
            1
        } else {
            -1
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::world::facility::Options;

    /// Every L corner of `fac` whose hall runs at least `hall` open cells past C: (A, C, d1, d2).
    pub fn corners(fac: &Facility, hall: i32) -> Vec<Corner> {
        let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let mut out = vec![];
        for a in fac.floor_cells() {
            for d1 in dirs {
                for d2 in dirs {
                    if d1 == d2 || d1 == (-d2.0, -d2.1) {
                        continue;
                    }
                    let c = step(a, d1);
                    let hall_open = (0..=hall).all(|k| open(fac, Cell::new(c.x + d2.0 * k, c.y + d2.1 * k)));
                    if open(fac, c) && hall_open && !open(fac, step(a, d2)) {
                        out.push(Corner { a, c, d1, d2 });
                    }
                }
            }
        }
        out
    }

    pub fn facility() -> Facility {
        let cat = crate::world::catalog::Catalog::load();
        Facility::generate("PEEK-1", &Options::default(), &cat)
    }

    #[test]
    fn the_embedded_file_matches_the_originals_tables() {
        let r = rules();
        assert_eq!((r.lookahead, r.cooldown, r.settle), (4, 6.0, 0.25));
        assert_eq!((r.body_behind, r.head_past), (0.42, 0.28));
        assert!(r.max_per_hunter > 0);
        let t = r.tuning("nyxie");
        assert_eq!((t.chance, t.look, t.style), (0.9, 2.0, Style::Ambush));
        assert_eq!(r.tuning("kathy").style, Style::Burst);
        assert_eq!(r.tuning("someone_new"), r.default);
        for never in ["aki", "zellia", "opalie", "void_gray"] {
            assert_eq!(r.chance(never, Difficulty::Hard), 0.0, "{never}");
        }
        assert_eq!(r.chance("xivvy", Difficulty::Standard), 0.3);
        assert!((r.chance("xivvy", Difficulty::Hard) - 0.345).abs() < 1e-5);
        assert_eq!(r.chance("nyxie", Difficulty::Hard), 1.0, "capped at one");
        let relaxed = r.scale(Difficulty::Relaxed);
        assert_eq!(relaxed, r.scale(Difficulty::Standard));
        assert_eq!((relaxed.react, relaxed.ambush), (0.45, 2.5));
        assert_eq!(r.scale(Difficulty::Hard).react, 0.25);
    }

    #[test]
    fn a_broken_file_falls_back_to_the_defaults() {
        let v = serde_json::json!({ "geometry": { "lookahead": "x" }, "hunters": { "a": { "style": "ambush" } } });
        let r = Rules::parse(&v);
        assert_eq!(r.lookahead, 4);
        assert_eq!(r.tuning("a").style, Style::Ambush);
        assert_eq!(r.tuning("a").chance, 0.35, "missing fields keep the default's");
    }

    #[test]
    fn side_is_plus_one_when_the_edge_is_on_her_left() {
        let c = |d1, d2| Corner { a: Cell::new(0, 0), c: Cell::new(1, 0), d1, d2 };
        assert_eq!(c((1, 0), (0, 1)).side(), 1);
        assert_eq!(c((1, 0), (0, -1)).side(), -1);
        assert_eq!(c((0, 1), (1, 0)).side(), -1);
        assert_eq!(c((0, 1), (-1, 0)).side(), 1);
    }

    /// A route round a wall corner is found, and the stand and eye sit on the A -> C axis either side of the edge
    /// plane; a straight run, a turn with an open inner cell, a diagonal leg and a corner out of reach are not corners.
    #[test]
    fn find_corner_wants_a_wall_on_the_inside_of_the_turn() {
        let fac = facility();
        let all = corners(&fac, 2);
        assert!(!all.is_empty(), "the maze has L corners");
        let k = all[0];
        let hall = step(step(k.c, k.d2), k.d2);
        let before = step(k.a, (-k.d1.0, -k.d1.1));
        let found = find_corner(&[k.a, k.c, step(k.c, k.d2), hall], &fac).expect("the corner");
        assert_eq!((found.a, found.c, found.d1, found.d2), (k.a, k.c, k.d1, k.d2));
        // the route may skip cells on its straight leg: before A is two cells back, the axis still reads
        let skipped = find_corner(&[step(before, (-k.d1.0, -k.d1.1)), k.c, step(k.c, k.d2)], &fac).expect("skipped leg");
        assert_eq!(skipped.d1, k.d1);
        let (b, h) = (found.body(), found.head());
        let (ax, ay) = k.a.center();
        let edge = (ax + k.d1.0 as f32 * 0.5, ay + k.d1.1 as f32 * 0.5);
        let along = |p: (f32, f32)| (p.0 - edge.0) * k.d1.0 as f32 + (p.1 - edge.1) * k.d1.1 as f32;
        assert!((along(b) + 0.42).abs() < 1e-5 && (along(h) - 0.28).abs() < 1e-5);
        assert!((b.0 - ax) * k.d2.0 as f32 + (b.1 - ay) * k.d2.1 as f32 == 0.0, "no sideways drift");
        // straight run, U-turn, diagonal: none
        assert_eq!(find_corner(&[k.a, k.c], &fac), None);
        assert_eq!(find_corner(&[k.a, k.c, step(k.c, (k.d1.0, k.d1.1))], &fac), None);
        assert_eq!(find_corner(&[k.c, k.a, k.c], &fac), None);
        assert_eq!(find_corner(&[k.a, Cell::new(k.a.x + 1, k.a.y + 1), k.c], &fac), None);
        // too far down the route: the lookahead window is four cells
        let mut far = vec![k.a; 6];
        far.extend([k.c, step(k.c, k.d2)]);
        assert_eq!(find_corner(&far, &fac), None, "duplicates are the caller's to remove, a zero leg is no corner");
    }

    /// An open inner cell is a room mouth or a straight hall, not a corner to peek round.
    #[test]
    fn an_open_inner_cell_is_no_corner() {
        let fac = facility();
        let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let mut checked = 0;
        for a in fac.floor_cells() {
            for d1 in dirs {
                for d2 in dirs {
                    let (c, i) = (step(a, d1), step(a, d2));
                    if d1 == d2 || d1 == (-d2.0, -d2.1) || !open(&fac, c) || !open(&fac, step(c, d2)) || !open(&fac, i) {
                        continue;
                    }
                    assert_eq!(find_corner(&[a, c, step(c, d2)], &fac), None);
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "the maze has open turns");
    }
}
