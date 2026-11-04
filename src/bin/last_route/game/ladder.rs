//! Sewer ladders, climbed by hand (Godot `game/player/player_controller_3d.gd`: `begin_ladder_climb`, the mount, the
//! climb and the dismount, lines 1265-1386), in one continuous space. A ladder is a shaft between two floors 5 m apart,
//! on the same cell on both levels, with its rails on the side away from the one clear step cell beside the hatch
//! (`Stair::rails_side`, `Stair::step_cell`). The runner mounts on the rungs (interact, or walking into the hatch from
//! the step cell), the body lerps to the rung line and faces the rails, then forward climbs up and back climbs down at
//! `LADDER_CLIMB_SPEED`, and no key hangs the body still. The body's absolute y moves smoothly the whole way
//! (`Player::set_abs_y`); the level label flips mid-shaft by the same height rule as a walked flight
//! (`Game::walk_relabel`), with no fade, no hold and no move of the body. A dismount lerps to the centre of the step
//! cell at that end. Letting go (jump at or below `LADDER_LET_GO_HEIGHT` above the bottom) hands the body back to the
//! normal movement, which falls to the floor below (game/player.rs).
//!
//! One climber at a time: `Game::ladder` holds the one climb, and a second mount is refused while it runs.

use super::level_stack::floor_y_of;
use super::{pdist, Controls, Game};
use crate::config::PLAYER_RADIUS;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::Cell;
use crate::world::infinite;
use crate::world::vertical::{
    LADDER_CLIMB_SPEED, LADDER_DISMOUNT_SECONDS, LADDER_LET_GO_HEIGHT, LADDER_MOUNT_SECONDS, LADDER_RAIL_REACH,
};
use std::f32::consts::{PI, TAU};

/// The noise a climbing body makes while it moves: the rungs clank (player_controller_3d.gd:1326, noise .35).
const CLANK_NOISE: f32 = 0.35;

/// The walk-in numbers (data/ladders.json): when walking into a hatch from its step cell mounts without interact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderRules {
    /// Metres added to the player radius: how near the hatch edge (facing the step cell) the body must walk to mount.
    /// The body stops one radius short of the open hatch, so the reach is that radius and this margin.
    pub walk_in_reach_margin: f32,
    /// How closely the body must face the hatch (dot of the facing and the way to the hatch) to walk into it.
    pub walk_in_facing: f32,
}

impl Default for LadderRules {
    fn default() -> Self {
        LadderRules { walk_in_reach_margin: 0.2, walk_in_facing: 0.5 }
    }
}

impl LadderRules {
    /// Reads and checks a ladders file: both numbers must be present, finite and inside their limits, or it is refused.
    pub fn parse(text: &str) -> Result<LadderRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("ladders.json: {e}"))?;
        let num = |key: &str, lo: f32, hi: f32| -> Result<f32, String> {
            let x = v
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| format!("ladders.json: {key} is missing or not a number"))? as f32;
            if x.is_finite() && x >= lo && x <= hi {
                Ok(x)
            } else {
                Err(format!("ladders.json: {key} = {x} is outside {lo}..={hi}"))
            }
        };
        Ok(LadderRules { walk_in_reach_margin: num("walk_in_reach_margin", 0.0, 2.0)?, walk_in_facing: num("walk_in_facing", 0.0, 1.0)? })
    }

    /// Whether a body at `pos` facing `forward` mounts the hatch at `cell` whose step cell lies along `run`.
    pub fn walks_in(&self, cell: Cell, run: (i32, i32), pos: (f32, f32), forward: (f32, f32)) -> bool {
        let (cx, cy) = cell.center();
        let (rx, ry) = (run.0 as f32, run.1 as f32);
        let edge = (cx + rx * 0.5, cy + ry * 0.5);
        let on_step_side = (pos.0 - cx) * rx + (pos.1 - cy) * ry > 0.0;
        let toward = -(forward.0 * rx + forward.1 * ry) > self.walk_in_facing;
        on_step_side && toward && pdist(edge, pos) <= PLAYER_RADIUS + self.walk_in_reach_margin
    }
}

/// The embedded walk-in rules (data/ladders.json); the shipped numbers if the file is refused.
pub fn rules() -> &'static LadderRules {
    static RULES: std::sync::OnceLock<LadderRules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| LadderRules::parse(include_str!("../data/ladders.json")).unwrap_or_default())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Mount,
    Climb,
    Dismount,
}

/// The end of the shaft a dismount goes off: the top (the upper floor) or the bottom (the lower floor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Top,
    Bottom,
}

/// One runner on one ladder, in the absolute frame: it knows no level, only the shaft's cell, its step cell's way
/// (`run`), the rails' `side`, and the absolute floors `y_top` and `y_bottom` it spans. `y` is the absolute y of the
/// feet on the shaft. The same cell and run are valid on both levels, so a relabel mid-shaft changes nothing here.
#[derive(Clone, Copy, Debug)]
pub struct Climb {
    pub cell: Cell,
    /// The unit step from the hatch cell to its one clear step cell.
    pub run: (i32, i32),
    /// The side of the cell the rails stand on (away from the step cell).
    pub side: (i32, i32),
    /// The direction of the ladder the climb started on (-1 mounted at the top of a shaft going down, +1 at the foot).
    pub dir: i32,
    pub y_top: f32,
    pub y_bottom: f32,
    pub y: f32,
    pub phase: Phase,
    /// Seconds into the mount or the dismount (0 to 1 over its length).
    pub t: f32,
    pub end: End,
    from: (f32, f32),
    from_yaw: f32,
    /// The eased share of the mount's or dismount's turn already applied (the turn is added in steps, so the mouse's own
    /// yaw is never overwritten).
    turn_done: f32,
    /// Where a dismount lands: the centre of the step cell.
    exit: (f32, f32),
}

impl Climb {
    /// A mount onto the shaft at `cell` going `dir` from the body's place now: a shaft going down is mounted at its top
    /// (the upper floor, from the step cell), one going up at its bottom (the lower floor, from the foot).
    fn mount(
        cell: Cell,
        run: (i32, i32),
        dir: i32,
        (y_top, y_bottom): (f32, f32),
        from: (f32, f32),
        yaw: f32,
    ) -> Climb {
        Climb {
            cell,
            run,
            side: (-run.0, -run.1),
            dir,
            y_top,
            y_bottom,
            y: if dir < 0 { y_top } else { y_bottom },
            phase: Phase::Mount,
            t: 0.0,
            end: if dir < 0 { End::Top } else { End::Bottom },
            from,
            from_yaw: yaw,
            turn_done: 0.0,
            exit: step_center(cell, run),
        }
    }

    /// Metres up the shaft from its lower end.
    pub fn height(&self) -> f32 {
        self.y - self.y_bottom
    }
}

/// The unit step from a ladder's hatch to its one clear step cell: away from the rails (`Stair::rails_side`).
pub fn run_of(s: &crate::world::facility::Stair) -> (i32, i32) {
    let side = s.rails_side();
    (-side.0, -side.1)
}

/// The centre of the step cell beside the hatch: where a dismount at either end lands.
pub fn step_center(cell: Cell, run: (i32, i32)) -> (f32, f32) {
    let (cx, cy) = cell.center();
    (cx + run.0 as f32, cy + run.1 as f32)
}

/// The side of a ladder cell a wall is on: the first wall next to it, north first (a fallback for a ladder record that
/// carries no `run`; the rails' side now comes from `Stair::rails_side`).
pub fn wall_side(cell: Cell, is_wall: impl Fn(i32, i32) -> bool) -> (i32, i32) {
    [(0, -1), (1, 0), (0, 1), (-1, 0)]
        .into_iter()
        .find(|m| is_wall(cell.x + m.0, cell.y + m.1))
        .unwrap_or((0, -1))
}

/// The body's line on the rungs: `LADDER_RAIL_REACH` out from the rails, which stand against the `side` of the cell.
pub fn stand(cell: Cell, side: (i32, i32)) -> (f32, f32) {
    let (cx, cy) = cell.center();
    let d = 0.5 - LADDER_RAIL_REACH;
    (cx + side.0 as f32 * d, cy + side.1 as f32 * d)
}

/// The yaw that faces `dir` (the convention of `Player::forward2`: forward is `(-sin yaw, -cos yaw)`). The body faces
/// the rails: `facing(side)`.
pub fn facing(dir: (i32, i32)) -> f32 {
    (-dir.0 as f32).atan2(-dir.1 as f32)
}

fn turn(from: f32, to: f32) -> f32 {
    (to - from + PI).rem_euclid(TAU) - PI
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

impl Game {
    fn storey(&self) -> i32 {
        self.inf.as_ref().map_or(0, |inf| inf.level)
    }

    /// Mounts the ladder `i` of this storey's `fac.stairs` (interact, or walking into its hatch). Refused while another
    /// climb runs (one climber), or when the storey beyond may not be entered. Returns true on a mount.
    pub(super) fn start_ladder(&mut self, i: usize) -> bool {
        let s = self.fac.stairs[i];
        if !s.is_ladder() || self.ladder.is_some() || self.p.falling() {
            return false;
        }
        let level = self.storey();
        if !infinite::level_allowed(level + s.dir) {
            return false;
        }
        let (upper, lower) = if s.dir < 0 { (level, level - 1) } else { (level + 1, level) };
        let floors = (floor_y_of(upper), floor_y_of(lower));
        self.focus = None;
        self.p.moving = false;
        self.ladder = Some(Climb::mount(s.cell, run_of(&s), s.dir, floors, self.p.pos, self.p.yaw));
        true
    }

    /// The runner walked into a hatch from its step cell: mount that ladder without interact. Called while the body
    /// moves: it must be on the ground, on the step cell's side, face the hatch, and be within the data reach of the
    /// hatch edge facing the step cell (cell centre + run * 0.5).
    pub(super) fn ladder_walk_in(&mut self) {
        if !self.p.moving || self.ladder.is_some() || self.p.stun > 0.0 || self.p.falling() || !self.p.grounded {
            return;
        }
        let (fx, fy) = self.p.forward2();
        let rules = rules();
        let hit = (0..self.fac.stairs.len()).find(|&i| {
            let s = self.fac.stairs[i];
            s.is_ladder() && rules.walks_in(s.cell, run_of(&s), self.p.pos, (fx, fy))
        });
        if let Some(i) = hit {
            self.start_ladder(i);
        }
    }

    /// One frame of mouse look on the ladder (Godot never writes the yaw while climbing): free horizontal look within
    /// `ladder_free_yaw_deg` of the rails' direction, easing into that edge, plus the normal pitch. Returns false when the
    /// climb is not in its Climb phase (the mount and dismount add their own eased turn on top of the plain look).
    pub(super) fn ladder_look(&mut self, c: &Controls) -> bool {
        let Some(k) = self.ladder else {
            return false;
        };
        if k.phase != Phase::Climb {
            return false;
        }
        let look = crate::config::look::cfg();
        let base = facing(k.side);
        let o = crate::config::look::soft_step(
            turn(base, self.p.yaw),
            -c.look.x,
            look.ladder_free_yaw_deg.to_radians(),
            look.ladder_soft_zone_deg.to_radians(),
        );
        self.p.yaw = base + o;
        self.p.look(macroquad::prelude::vec2(0.0, c.look.y));
        true
    }

    /// Begins a dismount at the end the body is at: it lerps to the centre of the step cell, its y fixed.
    fn dismount(&self, k: &mut Climb, end: End) {
        k.phase = Phase::Dismount;
        k.t = 0.0;
        k.end = end;
        k.y = match end {
            End::Top => k.y_top,
            End::Bottom => k.y_bottom,
        };
        k.from = self.p.pos;
        k.from_yaw = self.p.yaw;
        k.turn_done = 0.0;
    }

    /// Ends a dismount: the body stands in the step cell at the end it went off.
    fn finish_ladder(&mut self, k: Climb) {
        self.ladder = None;
        // the yaw is not written: the dismount's eased turn has finished and the mouse's own turn stays
        self.p.pos = k.exit;
        self.p.set_abs_y(k.y);
    }

    /// Lets go of the rungs where the body is: the climb ends, and the normal movement takes the body, which falls to the
    /// floor below if there is more than a step of air under it.
    pub(super) fn let_go_ladder(&mut self) {
        let Some(k) = self.ladder.take() else {
            return;
        };
        self.p.set_abs_y(k.y);
        self.p.moving = false;
        self.p.noise = 0.0;
    }

    /// One frame of the climb (mount, climb or dismount). Replaces the movement while mounted. The level label follows
    /// the height by `walk_relabel` (the caller's hook), never by this code.
    pub(super) fn advance_ladder(&mut self, dt: f32, c: &Controls, _content: &Content, _catalog: &Catalog) {
        let Some(mut k) = self.ladder else {
            return;
        };
        let mut moving = false;
        match k.phase {
            Phase::Mount => {
                k.t = (k.t + dt / LADDER_MOUNT_SECONDS).min(1.0);
                let e = smooth(k.t);
                self.p.pos = lerp2(k.from, stand(k.cell, k.side), e);
                self.p.yaw += turn(k.from_yaw, facing(k.side)) * (e - k.turn_done);
                k.turn_done = e;
                self.p.set_abs_y(k.y);
                if k.t >= 1.0 {
                    k.phase = Phase::Climb;
                }
            }
            Phase::Climb => {
                // forward climbs up, back climbs down; jump at or below the let-go height drops off the bottom
                let stopped = self.p.stun > 0.0;
                let axis = if stopped { 0.0 } else { c.fwd };
                if c.jump && k.height() <= LADDER_LET_GO_HEIGHT {
                    self.let_go_ladder();
                    return;
                } else if axis > 0.2 && k.y >= k.y_top {
                    self.dismount(&mut k, End::Top);
                } else if axis < -0.2 && k.y <= k.y_bottom {
                    self.dismount(&mut k, End::Bottom);
                } else if axis.abs() > 0.2 {
                    k.y = (k.y + axis.signum() * LADDER_CLIMB_SPEED * dt).clamp(k.y_bottom, k.y_top);
                    moving = true;
                }
                if k.phase == Phase::Climb {
                    self.p.pos = stand(k.cell, k.side);
                    self.p.set_abs_y(k.y);
                }
            }
            Phase::Dismount => {
                k.t = (k.t + dt / LADDER_DISMOUNT_SECONDS).min(1.0);
                let e = smooth(k.t);
                self.p.pos = lerp2(k.from, k.exit, e);
                // turn to face away from the hatch, so holding forward does not climb straight back on
                self.p.yaw += turn(k.from_yaw, facing(k.run)) * (e - k.turn_done);
                k.turn_done = e;
                self.p.set_abs_y(k.y);
                moving = true;
                if k.t >= 1.0 {
                    self.finish_ladder(k);
                    self.p.moving = false;
                    return;
                }
            }
        }
        self.p.noise = if moving { CLANK_NOISE } else { 0.0 };
        self.p.moving = false;
        self.ladder = Some(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::game::level_stack::ground_y;
    use crate::world::catalog::Catalog;
    use crate::world::facility::{Cover, Stair};
    use crate::world::vertical::{LADDER_LET_GO_HEIGHT as LET_GO, LADDER_MOUNT_SECONDS as MOUNT, STOREY};

    const DT: f32 = 0.05;

    fn quiet() -> Controls {
        Controls::default()
    }

    fn forward() -> Controls {
        Controls { fwd: 1.0, ..Controls::default() }
    }

    fn back() -> Controls {
        Controls { fwd: -1.0, ..Controls::default() }
    }

    fn jump() -> Controls {
        Controls { jump: true, ..Controls::default() }
    }

    /// The start window's ladder going down from the maze (the origin chunk always has one), the level below kept
    /// resident beside it (the climb itself builds nothing).
    fn ladder_game(content: &Content, catalog: &Catalog) -> (Game, usize) {
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, content, catalog);
        let i = g
            .fac
            .stairs
            .iter()
            .position(|s| s.is_ladder() && s.dir < 0)
            .expect("the start window has a ladder down");
        g.p.pos = step_center(g.fac.stairs[i].cell, run_of(&g.fac.stairs[i]));
        g.keep_stack(content, catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(-1), "the sewer is resident beside the hatch");
        (g, i)
    }

    /// The ladder of this storey nearest the runner (read back after a re-centre moves the window's coordinates).
    fn nearest_ladder(g: &Game) -> Stair {
        g.fac
            .stairs
            .iter()
            .copied()
            .filter(|s| s.is_ladder())
            .min_by(|a, b| pdist(a.cell.center(), g.p.pos).total_cmp(&pdist(b.cell.center(), g.p.pos)))
            .expect("this storey has a ladder")
    }

    fn step_center_of(s: Stair) -> (f32, f32) {
        step_center(s.cell, run_of(&s))
    }

    /// One frame of play.
    fn frame(g: &mut Game, c: &Controls, content: &Content, catalog: &Catalog) {
        g.step(DT, c, content, catalog, &Sounds::default());
    }

    /// Runs `c` for `secs` of play, one step at a time.
    fn run(g: &mut Game, c: &Controls, secs: f32, content: &Content, catalog: &Catalog) {
        let mut t = 0.0;
        while t < secs {
            frame(g, c, content, catalog);
            t += DT;
        }
    }

    #[test]
    fn mounting_lerps_to_the_rails_and_a_second_mount_is_refused() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let from = g.p.pos;
        assert!(g.start_ladder(i), "interact mounts the ladder");
        let k = g.ladder.expect("the climb runs");
        assert_eq!(k.phase, Phase::Mount);
        assert_eq!(k.side, g.fac.stairs[i].rails_side(), "the rails stand away from the step cell");
        assert!(!g.start_ladder(i), "one climber: a second mount is refused while the first runs");
        run(&mut g, &quiet(), MOUNT * 0.5, &content, &catalog);
        let mid = g.p.pos;
        let rungs = stand(k.cell, k.side);
        assert!(pdist(mid, from) > 1e-3 && pdist(mid, rungs) > 1e-3, "half way along the lerp: {mid:?}");
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        let k = g.ladder.expect("still on the rungs");
        assert_eq!(k.phase, Phase::Climb, "the mount ends on the rungs");
        assert_eq!(k.y, k.y_top, "a ladder down is mounted at its top");
        assert!(pdist(g.p.pos, rungs) < 1e-3, "the body is on the rung line");
        assert!((g.p.y() - k.y_top).abs() < 1e-4, "the feet stay at the end entered");
        let ahead = g.p.forward2();
        assert!(ahead.0 * k.side.0 as f32 + ahead.1 * k.side.1 as f32 > 0.99, "facing the rails");
    }

    #[test]
    fn walking_into_the_hatch_from_the_step_cell_mounts() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let s = g.fac.stairs[i];
        g.p.pos = step_center_of(s);
        g.p.yaw = facing((-run_of(&s).0, -run_of(&s).1));
        let mut frames = 0;
        while g.ladder.is_none() {
            frame(&mut g, &forward(), &content, &catalog);
            frames += 1;
            assert!(frames < 60, "walking at the hatch never mounted; at {:?}", g.p.pos);
        }
        assert_eq!(g.ladder.unwrap().phase, Phase::Mount);
        // walking away from the hatch does not
        let (mut g, i) = ladder_game(&content, &catalog);
        let s = g.fac.stairs[i];
        g.p.pos = step_center_of(s);
        g.p.yaw = facing(run_of(&s));
        run(&mut g, &forward(), 0.4, &content, &catalog);
        assert!(g.ladder.is_none(), "walking the other way mounts nothing");
    }

    #[test]
    fn forward_climbs_up_at_the_climb_speed_and_back_climbs_down() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // the top of a ladder down: back climbs down
        let before = g.ladder.unwrap().y;
        run(&mut g, &back(), 1.0, &content, &catalog);
        let after = g.ladder.unwrap().y;
        assert!((before - after - LADDER_CLIMB_SPEED).abs() < 0.1, "down 1 s at the climb speed: {before} -> {after}");
        assert!((g.p.y() - after).abs() < 1e-3, "the body drops with the climb");
        run(&mut g, &forward(), 1.0, &content, &catalog);
        assert!((g.ladder.unwrap().y - before).abs() < 0.1, "forward climbs back up");
    }

    #[test]
    fn with_no_key_the_body_hangs_still_and_makes_no_clank() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        run(&mut g, &back(), 0.3, &content, &catalog);
        let y = g.ladder.unwrap().y;
        run(&mut g, &quiet(), 2.0, &content, &catalog);
        assert_eq!(g.ladder.unwrap().y, y, "hanging keeps the height");
        assert_eq!(g.p.noise, 0.0, "no clank while still");
        run(&mut g, &back(), DT, &content, &catalog);
        assert_eq!(g.p.noise, CLANK_NOISE, "the rungs clank while the body moves");
    }

    #[test]
    fn climbing_is_continuous_in_y_and_the_label_flips_with_no_side_effect() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        g.fade = 0.0;
        let step = LADDER_CLIMB_SPEED * DT;
        let (mut flips, mut frames) = (0, 0);
        loop {
            let (level, pos, y) = (g.inf.as_ref().unwrap().level, g.p.pos, g.p.y());
            let builds = g.inf.as_ref().unwrap().builds;
            let phase = g.ladder.map(|k| k.phase);
            frame(&mut g, &back(), &content, &catalog);
            frames += 1;
            assert!(frames < 2000, "the climb never ended");
            assert_eq!(g.fade, 0.0, "a ladder creates no fade");
            assert!((g.p.y() - y).abs() <= step + 1e-3, "frame step in y: {} -> {}", y, g.p.y());
            if phase == Some(Phase::Climb) && g.ladder.is_some_and(|k| k.phase == Phase::Climb) {
                assert!(pdist(g.p.pos, pos) < 1e-4, "x and z stay on the rails while climbing");
            }
            let now = g.inf.as_ref().unwrap().level;
            if now != level {
                flips += 1;
                assert_eq!((level, now), (0, -1), "the label goes from the maze to the sewer");
                assert!(pdist(g.p.pos, pos) < 1e-4, "the flip frame did not move the body in x and z");
                assert!((y - g.p.y()).abs() > step * 0.5 && (y - g.p.y()).abs() <= step + 1e-3, "y moved one climb step");
                assert_eq!(g.inf.as_ref().unwrap().builds, builds, "the flip built no window");
                assert!(g.ladder.is_some_and(|k| k.phase == Phase::Climb), "the climb goes on");
            }
            if g.ladder.is_none() {
                break;
            }
        }
        assert_eq!(flips, 1, "the label flips exactly once");
        assert!((g.p.y() + STOREY).abs() < 0.01, "at the sewer floor: {}", g.p.y());
        let low = nearest_ladder(&g);
        let want = step_center_of(low);
        assert!(pdist(g.p.pos, want) < 0.05, "on the lower step cell: {:?} want {want:?}", g.p.pos);
        assert!(g.p.grounded && !g.p.falling());
    }

    #[test]
    fn climbing_up_from_the_sewer_foot_reaches_the_maze_beside_the_hatch() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", -1, &content, &catalog);
        let i = g.fac.stairs.iter().position(|s| s.is_ladder() && s.dir > 0).expect("the sewer window has a ladder foot");
        let s = g.fac.stairs[i];
        g.p.pos = step_center_of(s);
        g.p.set_abs_y(g.p.floor_y);
        g.keep_stack(&content, &catalog);
        assert!(g.start_ladder(i));
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        assert_eq!(g.ladder.unwrap().y, g.ladder.unwrap().y_bottom, "mounted at the foot");
        let mut flips = 0;
        let mut frames = 0;
        while g.ladder.is_some() {
            let level = g.inf.as_ref().unwrap().level;
            frame(&mut g, &forward(), &content, &catalog);
            flips += (g.inf.as_ref().unwrap().level != level) as i32;
            frames += 1;
            assert!(frames < 2000, "the climb never ended");
        }
        assert_eq!(flips, 1);
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        assert!(g.p.y().abs() < 0.01, "on the maze floor: {}", g.p.y());
        let top = nearest_ladder(&g);
        assert!(pdist(g.p.pos, step_center_of(top)) < 0.05, "beside the hatch on the step cell");
    }

    #[test]
    fn a_dismount_at_the_top_lands_on_the_step_cell_and_stands() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let level = g.inf.as_ref().unwrap().level;
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // a ladder down is mounted at the top; forward there is a dismount onto the maze floor
        run(&mut g, &forward(), DT, &content, &catalog);
        let k = g.ladder.unwrap();
        assert_eq!((k.phase, k.end), (Phase::Dismount, End::Top));
        run(&mut g, &quiet(), LADDER_DISMOUNT_SECONDS + DT, &content, &catalog);
        assert!(g.ladder.is_none(), "the climb is over");
        assert_eq!(g.inf.as_ref().unwrap().level, level, "no storey changed");
        // the window may have re-centred since the dismount: the ladder is read back from this storey's own list
        let s = nearest_ladder(&g);
        assert!(pdist(g.p.pos, step_center_of(s)) < 0.05, "on the step cell: {:?}", g.p.pos);
        assert!(g.p.grounded && !g.p.falling(), "standing");
        let stack = &g.inf.as_ref().unwrap().stack;
        let ground = ground_y(&g.fac, stack, g.p.pos.0, g.p.pos.1, g.p.y());
        assert!((ground.y - g.p.y()).abs() < 1e-3, "the feet are on the ground: {} vs {}", g.p.y(), ground.y);
    }

    #[test]
    fn jump_at_the_let_go_height_lets_go_and_a_high_jump_does_not() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        run(&mut g, &jump(), DT, &content, &catalog);
        assert_eq!(g.ladder.unwrap().phase, Phase::Climb, "a jump high up does not let go");
        let bottom = g.ladder.unwrap().y_bottom;
        g.ladder.as_mut().unwrap().y = bottom + LET_GO * 0.5;
        run(&mut g, &jump(), DT, &content, &catalog);
        assert!(g.ladder.is_none(), "a low jump lets go of the rungs");
    }

    #[test]
    fn falling_off_a_ladder_lands_on_ground_y_below() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // part way down, let go of the rungs
        let bottom = g.ladder.unwrap().y_bottom;
        g.ladder.as_mut().unwrap().y = bottom + LET_GO * 0.9;
        run(&mut g, &jump(), DT, &content, &catalog);
        assert!(g.ladder.is_none());
        // `Game::step` takes the landing itself and applies the landing rule (`apply_landing`), so the test watches the
        // fall end (falling, then not) and reads the rule's effect rather than `Player::take_landing`
        let from_y = g.p.y();
        let mut frames = 0;
        while frames == 0 || g.p.falling() {
            frame(&mut g, &quiet(), &content, &catalog);
            frames += 1;
            assert!(frames < 200, "the body never landed: y {}", g.p.y());
        }
        let drop = from_y - g.p.y();
        let stack = &g.inf.as_ref().unwrap().stack;
        let ground = ground_y(&g.fac, stack, g.p.pos.0, g.p.pos.1, g.p.y());
        assert!((g.p.y() - ground.y).abs() < 1e-3, "standing on the ground below: {} vs {}", g.p.y(), ground.y);
        assert!((g.p.y() - bottom).abs() < 1e-3, "on the lower floor");
        assert!(drop > 0.0 && drop < LET_GO, "dropped {drop} from the let-go height");
        assert!(g.p.take_landing().is_none(), "the step already took the landing");
        if drop >= crate::game::swim::fall_rules().min_height {
            assert!(g.p.noise_burst_timer > 0.0, "the landing rule ran on the touchdown frame (stun, noise or splash)");
        }
        assert!(!g.p.falling() && g.ladder.is_none());
    }

    #[test]
    fn the_camera_eye_never_jumps_more_than_a_climb_step_per_frame() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let step = LADDER_CLIMB_SPEED * DT;
        let mut last = g.p.eye_y();
        g.start_ladder(i);
        let mut frames = 0;
        let c = back();
        while g.ladder.is_some() {
            frame(&mut g, &c, &content, &catalog);
            let eye = g.p.eye_y();
            assert!((eye - last).abs() <= step + 1e-3, "eye y {last} -> {eye}");
            last = eye;
            frames += 1;
            assert!(frames < 2000, "the climb never ended");
        }
    }

    #[test]
    fn a_ladder_is_refused_when_the_storey_beyond_is_not_allowed() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        // the route ends at the lowest storey: from there no ladder goes down
        let lowest = (-10..=0).rev().find(|l| !infinite::level_allowed(*l - 1)).expect("the route has a lowest storey");
        g.inf.as_mut().unwrap().level = lowest;
        assert!(!g.start_ladder(i), "a ladder down from the lowest storey is refused");
        assert!(g.ladder.is_none());
    }

    #[test]
    fn a_ladder_stands_on_the_rails_side_away_from_its_step_cell() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (g, i) = ladder_game(&content, &catalog);
        let s = g.fac.stairs[i];
        assert!(matches!(s.kind, crate::world::facility::LinkKind::Ladder(Cover::Manhole | Cover::Grate)));
        assert_eq!((s.step_cell().x - s.cell.x, s.step_cell().y - s.cell.y), run_of(&s), "the step cell is across from the rails");
        assert!(pdist(stand(s.cell, s.rails_side()), step_center_of(s)) > 1.0, "the rungs are across the cell from the step");
        assert_eq!(wall_side(s.cell, |_, _| false), (0, -1), "no wall next to it: north");
        assert_eq!(wall_side(s.cell, |x, y| (x, y) == (s.cell.x + 1, s.cell.y)), (1, 0), "the first wall, east");
    }

    #[test]
    fn the_shipped_walk_in_numbers_parse_and_match_the_defaults() {
        let r = LadderRules::parse(include_str!("../data/ladders.json")).expect("the shipped file parses");
        assert_eq!(r, LadderRules::default());
        assert_eq!(*rules(), r, "the embedded file is the one in the tree");
        assert_eq!((r.walk_in_reach_margin, r.walk_in_facing), (0.2, 0.5));
    }

    #[test]
    fn a_bad_ladders_file_is_refused() {
        assert!(LadderRules::parse("not json").is_err());
        assert!(LadderRules::parse(r#"{"walk_in_facing": 0.5}"#).is_err(), "missing reach");
        assert!(LadderRules::parse(r#"{"walk_in_reach_margin": 0.2}"#).is_err(), "missing facing");
        assert!(LadderRules::parse(r#"{"walk_in_reach_margin": -1, "walk_in_facing": 0.5}"#).is_err());
        assert!(LadderRules::parse(r#"{"walk_in_reach_margin": 0.2, "walk_in_facing": 2}"#).is_err());
        assert!(LadderRules::parse(r#"{"walk_in_reach_margin": "x", "walk_in_facing": 0.5}"#).is_err());
    }

    #[test]
    fn changing_the_data_changes_when_the_walk_in_mounts() {
        let cell = Cell { x: 5, y: 5 };
        let run = (1, 0);
        let (cx, cy) = cell.center();
        // on the step side, `gap` beyond the hatch edge, facing the hatch (forward points along -run)
        let at = |gap: f32| (cx + 0.5 + gap, cy);
        let toward = (-1.0, 0.0);
        let base = LadderRules::default();
        let reach = PLAYER_RADIUS + base.walk_in_reach_margin;
        assert!(base.walks_in(cell, run, at(reach - 0.01), toward));
        assert!(!base.walks_in(cell, run, at(reach + 0.01), toward), "just out of reach");
        let wider = LadderRules { walk_in_reach_margin: base.walk_in_reach_margin + 0.3, ..base };
        assert!(wider.walks_in(cell, run, at(reach + 0.1), toward), "a bigger margin mounts from further");
        let tighter = LadderRules { walk_in_reach_margin: 0.0, ..base };
        assert!(!tighter.walks_in(cell, run, at(PLAYER_RADIUS + 0.1), toward), "no margin mounts only up close");
        // facing: 45 degrees off the hatch has a dot of about 0.71
        let a45 = (-(0.5f32).sqrt(), (0.5f32).sqrt());
        assert!(base.walks_in(cell, run, at(0.1), a45));
        let strict = LadderRules { walk_in_facing: 0.9, ..base };
        assert!(!strict.walks_in(cell, run, at(0.1), a45), "a stricter facing refuses the oblique walk");
        assert!(!base.walks_in(cell, run, at(0.1), (1.0, 0.0)), "facing away never mounts");
    }

    #[test]
    fn mouse_yaw_stays_free_on_the_ladder_and_is_never_overwritten() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let k0 = {
            g.start_ladder(i);
            g.ladder.unwrap()
        };
        let base = facing(k0.side);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        assert_eq!(g.ladder.unwrap().phase, Phase::Climb);
        assert!(turn(base, g.p.yaw).abs() < 1e-3, "faces the rails after the mount");
        // a mouse turn of +0.3 rad (yaw -= x) sticks, and holds with no further input
        let look = Controls { look: macroquad::prelude::vec2(0.3, 0.0), ..Controls::default() };
        frame(&mut g, &look, &content, &catalog);
        assert!((turn(base, g.p.yaw) + 0.3).abs() < 1e-4, "yaw {}", turn(base, g.p.yaw));
        let y = g.p.yaw;
        run(&mut g, &quiet(), 0.5, &content, &catalog);
        assert!((g.p.yaw - y).abs() < 1e-5, "nothing pulls it back while climbing");
        // far past the data limit it slows into the edge and never passes it
        let hard = Controls { look: macroquad::prelude::vec2(0.2, 0.0), ..Controls::default() };
        let limit = crate::config::look::cfg().ladder_free_yaw_deg.to_radians();
        for _ in 0..200 {
            frame(&mut g, &hard, &content, &catalog);
            assert!(turn(base, g.p.yaw).abs() <= limit + 1e-4);
        }
        // the dismount eases back out of the turn without a snap and keeps later mouse input
        g.ladder.as_mut().unwrap().y = g.ladder.unwrap().y_top;
        frame(&mut g, &forward(), &content, &catalog);
        assert_eq!(g.ladder.unwrap().phase, Phase::Dismount);
        let mut prev = g.p.yaw;
        let mut max_step = 0.0f32;
        while g.ladder.is_some() {
            frame(&mut g, &quiet(), &content, &catalog);
            max_step = max_step.max(turn(prev, g.p.yaw).abs());
            prev = g.p.yaw;
        }
        assert!(max_step < 0.5, "no frame of the dismount snaps the view: {max_step}");
        let run_yaw = facing(k0.run);
        assert!(turn(run_yaw, g.p.yaw).abs() < 1e-3, "ends facing away from the hatch");
    }

    #[test]
    fn facing_the_rails_points_the_body_at_them() {
        for side in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let yaw = facing(side);
            let ahead = (-yaw.sin(), -yaw.cos());
            assert!((ahead.0 - side.0 as f32).abs() < 1e-4 && (ahead.1 - side.1 as f32).abs() < 1e-4, "{side:?}");
        }
    }
}
