//! Sewer ladders, climbed by hand (Godot `game/player/player_controller_3d.gd`: `begin_ladder_climb`, the mount, the
//! climb and the dismount, lines 1265-1386). The runner mounts on the rungs (interact, or walking into the rung point),
//! the body lerps to the rung line and faces the wall, then forward climbs up and back climbs down at
//! `LADDER_CLIMB_SPEED`, and no key hangs the body still. The climb ends at either end, or by jump at or below
//! `LADDER_LET_GO_HEIGHT`, and the body dismounts onto the open floor beside that end. An end on another storey
//! (`level_from_height`, as the stairs use) changes storey on the frame the shaft's end is reached: the window loads
//! that storey and the runner is put beside the ladder there, with no fade and no hold (game/transition.rs).
//!
//! One climber at a time: `Game::ladder` holds the one climb, and a second mount is refused while it runs.

use super::{pdist, Controls, Game};
use crate::config::PLAYER_RADIUS;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::Cell;
use crate::world::infinite;
use crate::world::vertical::{
    level_from_height, LADDER_CLIMB_SPEED, LADDER_DISMOUNT_SECONDS, LADDER_LET_GO_HEIGHT, LADDER_MOUNT_SECONDS,
    LADDER_RAIL_REACH, STOREY,
};
use std::f32::consts::{PI, TAU};

/// How near the rung point the body must walk to mount without interact (our number: the rung trigger).
const WALK_IN_REACH: f32 = 0.3;
/// How far beside the ladder's foot a dismount lands, from the cell centre, away from the wall (metres).
const BESIDE: f32 = 0.75;
/// The noise a climbing body makes while it moves: the rungs clank (player_controller_3d.gd:1326, noise .35).
const CLANK_NOISE: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Mount,
    Climb,
    Dismount,
}

/// The end of the shaft a dismount goes off: the top (`STOREY`) or the bottom (0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Top,
    Bottom,
}

/// One runner on one ladder. `stair` indexes `fac.stairs` on the storey the climb started on. `height` is metres up
/// the shaft from its lower end: 0 at the bottom, `STOREY` at the top.
#[derive(Clone, Copy, Debug)]
pub struct Climb {
    pub stair: usize,
    pub cell: Cell,
    pub dir: i32,
    pub side: (i32, i32),
    pub phase: Phase,
    /// Seconds into the mount or the dismount.
    pub t: f32,
    pub height: f32,
    pub end: End,
    /// Whether the dismount at `end` leaves this storey (decided when the dismount starts).
    pub changes: bool,
    /// The storey the dismount at `end` reaches (the storey this one when it does not change).
    next: i32,
    from: (f32, f32),
    from_yaw: f32,
    from_lift: f32,
    /// Where a dismount lands on this storey; None when it changes storey (the change takes the body at once).
    exit: Option<(f32, f32)>,
}

impl Climb {
    /// A mount onto the ladder at `cell` (its direction `dir`, rails on `side`), from the body's place now. A shaft going
    /// down is mounted at its top, one going up at its bottom: the storey the runner stands on is that end.
    fn mount(stair: usize, cell: Cell, dir: i32, side: (i32, i32), from: (f32, f32), yaw: f32, lift: f32) -> Climb {
        Climb {
            stair,
            cell,
            dir,
            side,
            phase: Phase::Mount,
            t: 0.0,
            height: if dir < 0 { STOREY } else { 0.0 },
            end: End::Bottom,
            changes: false,
            next: 0,
            from,
            from_yaw: yaw,
            from_lift: lift,
            exit: None,
        }
    }
}

/// The eye height of a body at `height` on the shaft, relative to the floor of the storey it started on: a shaft going
/// down runs from `-STOREY` to 0, one going up from 0 to `STOREY`.
pub fn lift_at(height: f32, dir: i32) -> f32 {
    height + (dir as f32 * STOREY).min(0.0)
}

fn end_height(end: End) -> f32 {
    match end {
        End::Top => STOREY,
        End::Bottom => 0.0,
    }
}

/// The side of a ladder cell the rails are fixed to: the first wall next to it, north first (render/stairs.rs
/// `ladder_side`, which draws the same rails).
pub fn wall_side(cell: Cell, is_wall: impl Fn(i32, i32) -> bool) -> (i32, i32) {
    [(0, -1), (1, 0), (0, 1), (-1, 0)]
        .into_iter()
        .find(|m| is_wall(cell.x + m.0, cell.y + m.1))
        .unwrap_or((0, -1))
}

/// The body's line on the rungs: `LADDER_RAIL_REACH` out from the rails, which stand against the wall side of the cell.
pub fn stand(cell: Cell, side: (i32, i32)) -> (f32, f32) {
    let (cx, cy) = cell.center();
    let d = 0.5 - LADDER_RAIL_REACH;
    (cx + side.0 as f32 * d, cy + side.1 as f32 * d)
}

/// The yaw that faces `dir` (the convention of `Player::forward2`: forward is `(-sin yaw, -cos yaw)`). The body faces
/// the wall: `facing(side)`.
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

    fn ladder_side(&self, cell: Cell) -> (i32, i32) {
        wall_side(cell, |x, y| self.fac.is_wall_cell(x, y))
    }

    /// The open floor beside a ladder's foot on the storey the runner is on: away from the wall first, then the other
    /// three sides, and the nearest open floor if all four are shut.
    fn beside(&self, cell: Cell, side: (i32, i32)) -> (f32, f32) {
        let (cx, cy) = cell.center();
        let away = (-side.0, -side.1);
        let sides = [away, (1, 0), (-1, 0), (0, 1), (0, -1)];
        for d in sides {
            let (x, y) = (cx + d.0 as f32 * BESIDE, cy + d.1 as f32 * BESIDE);
            if self.fac.body_fits(x, y, PLAYER_RADIUS) {
                return (x, y);
            }
        }
        self.fac.nearest_open_or(cx + away.0 as f32 * BESIDE, cy + away.1 as f32 * BESIDE, PLAYER_RADIUS)
    }

    /// Mounts the ladder `i` of this storey (interact, or walking into its rung point). Refused while another climb
    /// runs (one climber), or when the storey beyond may not be entered. Returns true on a mount.
    pub(super) fn start_ladder(&mut self, i: usize) -> bool {
        let s = self.fac.stairs[i];
        if !s.is_ladder() || self.ladder.is_some() {
            return false;
        }
        if !infinite::level_allowed(self.storey() + s.dir) {
            return false;
        }
        let side = self.ladder_side(s.cell);
        self.focus = None;
        self.p.moving = false;
        self.ladder = Some(Climb::mount(i, s.cell, s.dir, side, self.p.pos, self.p.yaw, self.p.climb_lift));
        true
    }

    /// The runner walked into a rung point: mount that ladder without interact. Called while the body moves.
    pub(super) fn ladder_walk_in(&mut self) {
        if !self.p.moving || self.ladder.is_some() || self.p.stun > 0.0 {
            return;
        }
        let hit = (0..self.fac.stairs.len()).find(|&i| {
            let s = self.fac.stairs[i];
            s.is_ladder() && pdist(stand(s.cell, self.ladder_side(s.cell)), self.p.pos) <= WALK_IN_REACH
        });
        if let Some(i) = hit {
            self.start_ladder(i);
        }
    }

    /// Begins a dismount at `end`: the body goes to the open floor beside that end, or, when that end is on another
    /// storey, the storey changes at once on the frame the end is reached (`advance_ladder`).
    fn dismount(&self, k: &mut Climb, end: End) {
        k.phase = Phase::Dismount;
        k.t = 0.0;
        k.end = end;
        k.from = self.p.pos;
        k.from_lift = self.p.climb_lift;
        let level = self.storey();
        let next = level_from_height(level as f32 * STOREY + lift_at(end_height(end), k.dir), level);
        k.changes = self.inf.is_some() && next != level;
        k.next = next;
        k.exit = if k.changes { None } else { Some(self.beside(k.cell, k.side)) };
    }

    /// Ends a dismount onto the floor of this storey: the body is beside that end.
    fn finish_ladder(&mut self, k: Climb) {
        self.ladder = None;
        self.p.pos = k.exit.unwrap_or(self.p.pos);
        self.p.climb_lift = 0.0;
    }

    /// Lands the runner beside the ladder on the storey the change just loaded (`Game::change_storey`).
    pub(super) fn place_beside_ladder(&mut self, cell: Cell, _side: (i32, i32)) {
        let side = self.ladder_side(cell);
        self.p.pos = self.beside(cell, side);
        self.p.yaw = facing((-side.0, -side.1));
        self.p.climb_lift = 0.0;
    }

    /// One frame of the climb (mount, climb or dismount). Replaces the movement while mounted.
    pub(super) fn advance_ladder(&mut self, dt: f32, c: &Controls, content: &Content, catalog: &Catalog) {
        let Some(mut k) = self.ladder else {
            return;
        };
        let mut moving = false;
        match k.phase {
            Phase::Mount => {
                k.t = (k.t + dt / LADDER_MOUNT_SECONDS).min(1.0);
                let e = smooth(k.t);
                self.p.pos = lerp2(k.from, stand(k.cell, k.side), e);
                self.p.yaw = k.from_yaw + turn(k.from_yaw, facing(k.side)) * e;
                let to_lift = lift_at(k.height, k.dir);
                self.p.climb_lift = k.from_lift + (to_lift - k.from_lift) * e;
                if k.t >= 1.0 {
                    k.phase = Phase::Climb;
                }
            }
            Phase::Climb => {
                // forward climbs up, back climbs down; jump at or below the let-go height drops off the bottom
                let stopped = self.p.stun > 0.0;
                let axis = if stopped { 0.0 } else { c.fwd };
                let h = k.height;
                if c.jump && h <= LADDER_LET_GO_HEIGHT {
                    self.dismount(&mut k, End::Bottom);
                } else if axis > 0.2 && h >= STOREY {
                    self.dismount(&mut k, End::Top);
                } else if axis < -0.2 && h <= 0.0 {
                    self.dismount(&mut k, End::Bottom);
                } else if axis.abs() > 0.2 {
                    k.height = (h + axis.signum() * LADDER_CLIMB_SPEED * dt).clamp(0.0, STOREY);
                    moving = true;
                }
                if k.phase == Phase::Dismount && k.changes {
                    // the shaft's end is the connection: the storey changes on this frame, with no hold and no fade
                    self.ladder = None;
                    self.p.moving = false;
                    self.change_storey(k.next, content, catalog, |g| g.place_beside_ladder(k.cell, k.side));
                    self.autosave("level");
                    return;
                }
                if k.phase == Phase::Climb {
                    self.p.pos = stand(k.cell, k.side);
                    self.p.yaw = facing(k.side);
                    self.p.climb_lift = lift_at(k.height, k.dir);
                }
            }
            Phase::Dismount => {
                k.t = (k.t + dt / LADDER_DISMOUNT_SECONDS).min(1.0);
                let e = smooth(k.t);
                let to = k.exit.unwrap_or(k.from);
                self.p.pos = lerp2(k.from, to, e);
                let to_lift = lift_at(end_height(k.end), k.dir);
                self.p.climb_lift = k.from_lift + (to_lift - k.from_lift) * e;
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
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Cover;
    use crate::world::vertical::{LADDER_LET_GO_HEIGHT as LET_GO, LADDER_MOUNT_SECONDS as MOUNT};

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

    /// The start window's ladder going down from the maze (the origin chunk always has one).
    fn ladder_game(content: &Content, catalog: &Catalog) -> (Game, usize) {
        let g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, content, catalog);
        let i = g
            .fac
            .stairs
            .iter()
            .position(|s| s.is_ladder() && s.dir < 0)
            .expect("the start window has a ladder down");
        (g, i)
    }

    /// The ladder cell of this storey nearest the runner (read back after a re-centre moves the window's coordinates).
    fn nearest_ladder(g: &Game) -> Cell {
        g.fac
            .stairs
            .iter()
            .filter(|s| s.is_ladder())
            .map(|s| s.cell)
            .min_by(|a, b| pdist(a.center(), g.p.pos).total_cmp(&pdist(b.center(), g.p.pos)))
            .expect("this storey has a ladder")
    }

    /// Runs `c` for `secs` of play, one step at a time.
    fn run(g: &mut Game, c: &Controls, secs: f32, content: &Content, catalog: &Catalog) {
        let sounds = Sounds::default();
        let mut t = 0.0;
        while t < secs {
            g.step(DT, c, content, catalog, &sounds);
            t += DT;
        }
    }

    #[test]
    fn walking_into_the_rung_point_mounts_and_a_second_mount_is_refused() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        assert!(g.start_ladder(i), "interact mounts the ladder");
        let k = g.ladder.expect("the climb runs");
        assert_eq!(k.phase, Phase::Mount);
        assert!(!g.start_ladder(i), "one climber: a second mount is refused while the first runs");
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        let k = g.ladder.expect("still on the rungs");
        assert_eq!(k.phase, Phase::Climb, "the mount ends on the rungs");
        assert_eq!(k.height, STOREY, "a ladder down is mounted at its top");
        let stand = stand(k.cell, k.side);
        assert!(pdist(g.p.pos, stand) < 1e-3, "the body is on the rung line");
    }

    #[test]
    fn forward_climbs_up_at_the_climb_speed_and_back_climbs_down() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // the top of a ladder down: back climbs down
        let before = g.ladder.unwrap().height;
        run(&mut g, &back(), 1.0, &content, &catalog);
        let after = g.ladder.unwrap().height;
        assert!((before - after - LADDER_CLIMB_SPEED).abs() < 0.1, "down 1 s at the climb speed: {before} -> {after}");
        assert!((g.p.climb_lift - lift_at(after, -1)).abs() < 1e-4, "the camera drops with the body");
        run(&mut g, &forward(), 1.0, &content, &catalog);
        assert!((g.ladder.unwrap().height - before).abs() < 0.1, "forward climbs back up");
    }

    #[test]
    fn with_no_key_the_body_hangs_still_and_makes_no_clank() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        run(&mut g, &back(), 0.3, &content, &catalog);
        let h = g.ladder.unwrap().height;
        run(&mut g, &quiet(), 2.0, &content, &catalog);
        assert_eq!(g.ladder.unwrap().height, h, "hanging keeps the height");
        assert_eq!(g.p.noise, 0.0, "no clank while still");
        run(&mut g, &back(), DT, &content, &catalog);
        assert_eq!(g.p.noise, CLANK_NOISE, "the rungs clank while the body moves");
    }

    #[test]
    fn jump_at_the_let_go_height_drops_off_the_bottom() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // high on the ladder a jump does nothing
        run(&mut g, &jump(), DT, &content, &catalog);
        assert_eq!(g.ladder.unwrap().phase, Phase::Climb, "a jump high up does not let go");
        // down to the let-go height, then jump
        g.ladder.as_mut().unwrap().height = LET_GO * 0.5;
        let level = g.inf.as_ref().unwrap().level;
        g.fade = 0.0;
        run(&mut g, &jump(), DT, &content, &catalog);
        // the bottom of this shaft is the storey below: the jump lets go and the storey changes on that frame, no hold, no fade
        assert!(g.ladder.is_none(), "low jump lets go, and the change ends the climb");
        assert_eq!(g.inf.as_ref().unwrap().level, level - 1, "the storey below");
        assert_eq!(g.fade, 0.0, "no fade");
    }

    #[test]
    fn a_dismount_at_the_top_lands_beside_the_ladder_on_the_same_storey() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let level = g.inf.as_ref().unwrap().level;
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        // a ladder down is mounted at the top; forward there is a dismount onto the maze floor
        run(&mut g, &forward(), DT, &content, &catalog);
        let k = g.ladder.unwrap();
        assert_eq!((k.phase, k.end, k.changes), (Phase::Dismount, End::Top, false));
        run(&mut g, &quiet(), LADDER_DISMOUNT_SECONDS + DT, &content, &catalog);
        assert!(g.ladder.is_none(), "the climb is over");
        assert_eq!(g.inf.as_ref().unwrap().level, level, "no storey changed");
        assert_eq!(g.p.climb_lift, 0.0);
        // the window may have re-centred since the dismount: the ladder is read back from this storey's own list
        let cell = nearest_ladder(&g);
        let beside = g.beside(cell, g.ladder_side(cell));
        assert!(pdist(g.p.pos, beside) < 1e-3, "on the open floor beside the top: {:?}", g.p.pos);
        assert!(pdist(g.p.pos, cell.center()) > 0.5, "not in the shaft");
    }

    #[test]
    fn climbing_down_to_the_bottom_changes_storey_on_that_frame_with_no_fade() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (mut g, i) = ladder_game(&content, &catalog);
        let level = g.inf.as_ref().unwrap().level;
        g.start_ladder(i);
        run(&mut g, &quiet(), MOUNT + DT, &content, &catalog);
        g.fade = 0.0;
        // back down the whole shaft: the bottom is the storey below (level_from_height)
        let mut frames = 0;
        while g.inf.as_ref().unwrap().level == level {
            g.step(DT, &back(), &content, &catalog, &Sounds::default());
            assert_eq!(g.fade, 0.0, "a ladder creates no fade");
            frames += 1;
            assert!(frames < 2000, "the climb never reached the bottom");
        }
        // the change came on the frame the shaft's end was reached: no dismount hold, no fade, the climb is over
        assert!(g.ladder.is_none(), "the climb ends on the change frame");
        assert_eq!(g.inf.as_ref().unwrap().level, level - 1, "the storey below");
        assert_eq!(g.fade, 0.0);
        assert_eq!(g.p.climb_lift, 0.0);
        // the runner lands beside the ladder's foot on the storey below. The window re-centres on the runner after the
        // change, so the foot is read back from this storey's own list (the old cell's coordinates no longer apply)
        let foot = nearest_ladder(&g);
        let d = pdist(g.p.pos, foot.center());
        assert!((0.6..1.2).contains(&d), "beside the ladder's foot, not in it: {d}");
    }

    #[test]
    fn a_ladder_is_climbed_only_on_its_own_rung_point_and_its_cover_still_reads() {
        let content = Content::load();
        let catalog = Catalog::load();
        let (g, i) = ladder_game(&content, &catalog);
        let s = g.fac.stairs[i];
        assert!(matches!(s.kind, crate::world::facility::LinkKind::Ladder(Cover::Manhole | Cover::Grate)));
        assert_eq!(wall_side(s.cell, |_, _| false), (0, -1), "no wall next to it: north");
        assert_eq!(wall_side(s.cell, |x, y| (x, y) == (s.cell.x + 1, s.cell.y)), (1, 0), "the first wall, east");
    }

    #[test]
    fn facing_the_wall_points_the_body_at_it() {
        for side in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let yaw = facing(side);
            let ahead = (-yaw.sin(), -yaw.cos());
            assert!((ahead.0 - side.0 as f32).abs() < 1e-4 && (ahead.1 - side.1 as f32).abs() < 1e-4, "{side:?}");
        }
    }
}
