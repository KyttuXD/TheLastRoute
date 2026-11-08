//! Xivvy: chase accelerator. Coming within a short radius (0.75 cells, up to 1.5 with full
//! exposure) locks her on without sight; the longer a chase runs the faster she gets, up to
//! x1.8, taking wing after 0.65 s (xivvy_gimmick.gd).
//!
//! Flight is xivvy_flight.gd (XivvyFlight). The chase takeoff only sets `flying`; she is airborne
//! (`airborne()`) where the route ahead has headroom. The survey (hunter_view_3d.gd `_survey_route`,
//! run every SURVEY_INTERVAL while she chases or is in the air) reads the ceiling over the route,
//! lands her for a doorway-sized gap or a low stretch, hovers her HOVER_HEIGHT under the ceiling,
//! dips her to a grab near the runner and floats her (a bob and a sway). The view reads `lift()`
//! and `sway_offset()`. The facility has no per-cell ceiling: the wall height of the cell's region
//! is the ceiling (world_mesh.rs `region_wall_height`), so a 3 m region is a walk-only room and a
//! 5 m hall is flown through.

use super::Ctx;
use crate::config::WALL_HEIGHT;
use crate::hunters::brain::{cell_of, dist, Brain, State, Target};
use crate::world::facility::Facility;

/// Xivvy's flight tunables (data/gimmicks/xivvy.json; xivvy_flight.gd, hunter_view_3d.gd survey). A missing or
/// bad key keeps the Godot value.
#[derive(Clone, Debug, PartialEq)]
pub struct Flight {
    pub takeoff_seconds: f32,
    pub body_height: f32,
    pub ceiling_gap: f32,
    pub hover_height: f32,
    pub float_amplitude: f32,
    pub float_hz: f32,
    pub fly_enter_headroom: f32,
    pub fly_exit_headroom: f32,
    pub enter_dwell: f32,
    pub gap_half_width: f32,
    pub swoop_range: f32,
    pub grab_range: f32,
    pub swoop_altitude: f32,
    pub rise_rate: f32,
    pub survey_interval: f32,
    pub survey_distances: Vec<f32>,
    pub survey_footprint: f32,
    pub near_reach: f32,
}

impl Flight {
    pub fn parse(text: &str) -> Flight {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let num = |k: &str, d: f32| match v.get(k).and_then(|x| x.as_f64()) {
            Some(n) if n >= 0.0 && n.is_finite() => n as f32,
            _ => d,
        };
        let pos = |k: &str, d: f32| {
            let n = num(k, d);
            if n > 0.0 {
                n
            } else {
                d
            }
        };
        let mut distances: Vec<f32> = v
            .get("survey_distances")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|n| n.as_f64()).filter(|n| *n >= 0.0 && n.is_finite()).map(|n| n as f32).collect())
            .unwrap_or_default();
        if distances.is_empty() {
            distances = vec![0.0, 1.5, 3.0, 4.5, 6.0, 8.0, 10.0, 12.0];
        }
        let enter = pos("fly_enter_headroom", 3.6);
        Flight {
            takeoff_seconds: num("takeoff_seconds", 0.65),
            body_height: pos("body_rig_height", 1.78) * pos("model_scale", 1.5),
            ceiling_gap: num("ceiling_gap", 0.14),
            hover_height: num("hover_height", 1.2),
            float_amplitude: num("float_amplitude", 0.05),
            float_hz: pos("float_hz", 0.7),
            fly_enter_headroom: enter,
            // she never lands above the height she needs to lift off
            fly_exit_headroom: pos("fly_exit_headroom", 3.2).min(enter),
            enter_dwell: num("enter_dwell", 0.45),
            gap_half_width: pos("gap_half_width", 1.0),
            swoop_range: pos("swoop_range", 6.0),
            grab_range: pos("grab_range", 1.4),
            swoop_altitude: num("swoop_altitude", 0.30),
            rise_rate: pos("rise_rate", 3.4),
            survey_interval: pos("survey_interval", 0.1),
            survey_distances: distances,
            survey_footprint: num("survey_footprint", 0.45),
            near_reach: num("near_reach", 3.0),
        }
    }
}

/// The shipped flight tunables, parsed once.
pub fn k() -> &'static Flight {
    static F: std::sync::OnceLock<Flight> = std::sync::OnceLock::new();
    F.get_or_init(|| Flight::parse(include_str!("../../data/gimmicks/xivvy.json")))
}

/// What the route survey reads (hunter_view_3d.gd `_survey_route`): the lowest headroom over the stretch
/// ahead (`far`), the lowest near her (`near`, the cruise height), and whether a doorway is on the way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Survey {
    pub far: f32,
    pub near: f32,
    pub gap: bool,
}

pub struct Xivvy {
    chase_elapsed: f32,
    pub speed_scale: f32,
    /// The chase takeoff (xivvy_gimmick.gd `flying`): the chase has run takeoff_seconds.
    pub flying: bool,
    /// Clearance mode (xivvy_flight.gd `fly_ok`): false = she walks (a doorway or a low stretch ahead).
    pub fly_ok: bool,
    /// Her feet above the floor, in metres (xivvy_flight.gd `altitude`).
    altitude: f32,
    /// The presentation float: a bob (vertical) and a sway (sideways), from `float_time` (no RNG).
    float_time: f32,
    float_bob: f32,
    float_sway: f32,
    enter_timer: f32,
    survey_timer: f32,
    /// The near headroom the cruise height is taken from (xivvy_flight.gd `_flight_headroom`).
    cruise_headroom: f32,
}

impl Default for Xivvy {
    fn default() -> Self {
        Xivvy {
            chase_elapsed: 0.0,
            speed_scale: 1.0,
            flying: false,
            fly_ok: true,
            altitude: 0.0,
            float_time: 0.0,
            float_bob: 0.0,
            float_sway: 0.0,
            enter_timer: 0.0,
            survey_timer: 0.0,
            cruise_headroom: f32::INFINITY,
        }
    }
}

impl Xivvy {
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        let lock = 1.5 * (0.5 + c.exposure / 200.0);
        if !t.hidden && !t.in_safe_zone && dist(b.pos, t.pos) <= lock {
            b.state = State::Chase;
            b.last_known = Some(t.pos);
        }
        self.chase_elapsed = if b.state == State::Chase {
            self.chase_elapsed + dt
        } else {
            0.0
        };
        self.flying = self.chase_elapsed >= k().takeoff_seconds;
        self.speed_scale = (1.0 + self.chase_elapsed * 0.035).min(1.8);
        self.fly_step(b, dt, c.fac);
    }

    /// She is in the air only while she has taken wing AND the route ahead allows it (xivvy_gimmick.gd airborne()).
    pub fn airborne(&self) -> bool {
        self.flying && self.fly_ok
    }

    /// Her height the view draws, metres above the floor: the flight altitude plus the float bob.
    pub fn lift(&self) -> f32 {
        self.altitude + self.float_bob
    }

    /// The sideways float as a world offset across her heading `facing`. Zero when there is no heading.
    pub fn sway_offset(&self, facing: (f32, f32)) -> (f32, f32) {
        let len = (facing.0 * facing.0 + facing.1 * facing.1).sqrt();
        if len < 1e-4 {
            return (0.0, 0.0);
        }
        let side = (-facing.1 / len, facing.0 / len);
        (side.0 * self.float_sway, side.1 * self.float_sway)
    }

    /// One sim tick of the flight: the survey on its own clock, the mode, then the altitude and the float.
    /// Mirrors xivvy_flight.gd and hunter_view_3d.gd `_update_flight` (lines 1162-1194).
    fn fly_step(&mut self, b: &Brain, dt: f32, fac: &Facility) {
        let chasing = self.flying || b.state == State::Chase;
        // the runner's distance is measured to the look target while she chases (hunter_view_3d.gd:14-15)
        let runner_distance = match (b.state, b.last_known) {
            (State::Chase, Some(p)) => dist(b.pos, p),
            _ => f32::INFINITY,
        };
        self.survey_timer -= dt;
        if self.survey_timer <= 0.0 && (chasing || self.altitude > 0.0) {
            self.survey_timer = k().survey_interval;
            let s = survey(fac, b.pos, b.facing, &|d| b.path_look_ahead(d));
            self.cruise_headroom = s.near;
            self.update_mode(k().survey_interval, s.far, s.gap);
        } else if !chasing && self.altitude <= 0.0 {
            self.fly_ok = true;
        }
        self.update_altitude(dt, self.airborne(), runner_distance);
    }

    /// Walk or fly (XivvyFlight.update_mode). Landing is immediate (she must be down before the door); taking off
    /// needs the way clear for enter_dwell. `headroom` is the lowest ceiling over the stretch ahead.
    pub fn update_mode(&mut self, dt: f32, headroom: f32, gap: bool) {
        if gap || headroom < k().fly_exit_headroom {
            self.fly_ok = false;
            self.enter_timer = 0.0;
        } else if !self.fly_ok && headroom >= k().fly_enter_headroom {
            self.enter_timer += dt;
            if self.enter_timer >= k().enter_dwell {
                self.fly_ok = true;
            }
        } else {
            self.enter_timer = 0.0;
        }
    }

    /// Her hover under a ceiling (XivvyFlight.cruise_altitude): hover_height in the open, lower where the
    /// ceiling would reach her head.
    pub fn cruise_altitude(headroom: f32) -> f32 {
        clampf(headroom - k().body_height - k().ceiling_gap, 0.0, k().hover_height)
    }

    /// Where she wants her feet (XivvyFlight.target_altitude): the cruise height in the air, swooping toward the
    /// floor near the runner, and the floor on foot.
    pub fn target_altitude(flying: bool, headroom: f32, runner_distance: f32) -> f32 {
        if !flying {
            return 0.0;
        }
        let cruise = Self::cruise_altitude(headroom);
        if runner_distance >= k().swoop_range {
            return cruise;
        }
        let closeness = clampf((k().swoop_range - runner_distance) / (k().swoop_range - k().grab_range), 0.0, 1.0);
        lerpf(cruise, cruise.min(k().swoop_altitude), closeness * closeness)
    }

    /// The altitude and the float for one frame (XivvyFlight.update, without the bank, pitch and wing cues).
    fn update_altitude(&mut self, delta: f32, flying: bool, runner_distance: f32) {
        let frame = delta.clamp(1.0 / 240.0, 0.1);
        let headroom = self.cruise_headroom;
        let wanted = Self::target_altitude(flying, headroom, runner_distance);
        let eased = lerpf(
            self.altitude,
            wanted,
            1.0 - (-(if wanted > self.altitude { 4.2 } else { 5.0 }) * frame).exp(),
        );
        self.altitude = move_toward(self.altitude, eased, k().rise_rate * frame);
        self.altitude = move_toward(self.altitude, wanted, 0.5 * frame);
        if self.altitude < 0.004 && wanted == 0.0 {
            self.altitude = 0.0;
        }
        // the float (view only): a bob and a sway that fade in with the lift. The bob stays inside the clearance above
        // her feet (never her head into the ceiling) and never takes her below the floor.
        self.float_time = (self.float_time + frame) % (1.0 / k().float_hz);
        let lift = (self.altitude / 0.25).clamp(0.0, 1.0);
        let clearance = headroom - k().body_height - k().ceiling_gap;
        let phase = std::f32::consts::TAU * k().float_hz * self.float_time;
        self.float_bob = (phase.sin() * k().float_amplitude * lift)
            .clamp(-self.altitude, (clearance - self.altitude).max(0.0));
        self.float_sway = phase.cos() * k().float_amplitude * lift;
    }
}

/// Free height above the floor at one point: the ceiling over its cell.
pub fn headroom_at(fac: &Facility, p: (f32, f32)) -> f32 {
    let cell = cell_of(p);
    ceiling_of(fac, (cell.x, cell.y))
}

/// The ceiling over one cell in metres: its region's wall height, read the way world_mesh.rs draws it (the
/// cell's room region unless it is in the lobby, else the district of the cell; the region's geometry wall height,
/// else WALL_HEIGHT).
pub fn ceiling_of(fac: &Facility, cell: (i32, i32)) -> f32 {
    let (x, y) = cell;
    let region = match fac
        .room_at(x as f64 + 0.5, y as f64 + 0.5)
        .filter(|i| !fac.rooms[*i].lobby())
    {
        Some(i) => fac.rooms[i].region.as_str(),
        None => fac.region_at(x, y),
    };
    crate::content::regions::table()
        .def(region)
        .and_then(|d| d.geometry)
        .map_or(WALL_HEIGHT, |g| g.wall_height)
}

/// The route survey (hunter_view_3d.gd `_survey_route`). `look_ahead(d)` is the brain's route `d` metres ahead,
/// None without a route (the heading then stands in for it, as the original's Vector2.INF fallback does).
pub fn survey(
    fac: &Facility,
    pos: (f32, f32),
    facing: (f32, f32),
    look_ahead: &dyn Fn(f32) -> Option<(f32, f32)>,
) -> Survey {
    let mut far = f32::INFINITY;
    let mut near = f32::INFINITY;
    let mut gap = false;
    let len = (facing.0 * facing.0 + facing.1 * facing.1).sqrt();
    let heading = if len < 1e-4 {
        (0.0, 1.0)
    } else {
        (facing.0 / len, facing.1 / len)
    };
    for &distance in k().survey_distances.iter() {
        let point = if distance > 0.0 {
            look_ahead(distance).unwrap_or((
                pos.0 + heading.0 * distance,
                pos.1 + heading.1 * distance,
            ))
        } else {
            pos
        };
        let mut headroom = headroom_at(fac, point);
        if distance <= 0.0 {
            // B3: her footprint, not only her centre
            for side in [
                (k().survey_footprint, 0.0),
                (-k().survey_footprint, 0.0),
                (0.0, k().survey_footprint),
                (0.0, -k().survey_footprint),
            ] {
                headroom = headroom.min(headroom_at(fac, (point.0 + side.0, point.1 + side.1)));
            }
        }
        far = far.min(headroom);
        if distance <= k().near_reach {
            near = near.min(headroom);
        }
        // a doorway: walls within k().gap_half_width on both sides at this point
        let along = match (distance > 0.0, look_ahead(distance + 1.0)) {
            (true, Some(next)) => (next.0 - point.0, next.1 - point.1),
            _ => heading,
        };
        let along_len = (along.0 * along.0 + along.1 * along.1).sqrt();
        let along = if along_len < 1e-4 {
            heading
        } else {
            (along.0 / along_len, along.1 / along_len)
        };
        let side = (-along.1, along.0);
        if wall_within(fac, point, side) && wall_within(fac, point, (-side.0, -side.1)) {
            gap = true;
        }
    }
    Survey { far, near, gap }
}

/// True where a ray from `from` along `dir` meets a wall cell within gap_half_width (the grid form of the
/// chest-height rays in hunter_view_3d.gd `_survey_route`).
fn wall_within(fac: &Facility, from: (f32, f32), dir: (f32, f32)) -> bool {
    let mut d = 0.0;
    while d <= k().gap_half_width {
        let x = from.0 + dir.0 * d;
        let y = from.1 + dir.1 * d;
        if fac.is_wall_cell(x.floor() as i32, y.floor() as i32) {
            return true;
        }
        d += 0.05;
    }
    false
}

fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

fn lerpf(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Godot's move_toward: step `from` toward `to` by at most `step`.
fn move_toward(from: f32, to: f32, step: f32) -> f32 {
    if (to - from).abs() <= step {
        to
    } else {
        from + (to - from).signum() * step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn near_ceiling(h: f32) -> f32 {
        Xivvy::cruise_altitude(h)
    }

    #[test]
    fn hover_is_the_full_height_in_the_open_and_drops_under_a_low_ceiling() {
        assert!((Xivvy::cruise_altitude(f32::INFINITY) - k().hover_height).abs() < 1e-6, "open ground hovers at 1.2 m");
        assert!((Xivvy::cruise_altitude(5.0) - k().hover_height).abs() < 1e-6, "a 5 m hall still hovers at 1.2 m");
        // 4 m: 4 - 2.67 - 0.14 = 1.19 m, just under the hover
        assert!((near_ceiling(4.0) - 1.19).abs() < 1e-3, "4 m hall: {}", near_ceiling(4.0));
        // at the fly threshold she has no room to hover
        assert!(near_ceiling(3.6) < 0.8, "3.6 m: {}", near_ceiling(3.6));
        assert_eq!(near_ceiling(2.0), 0.0, "a ceiling under her body never lifts her");
    }

    #[test]
    fn the_mode_lands_at_once_and_takes_off_only_after_the_dwell() {
        let mut x = Xivvy::default();
        // a doorway ahead lands her this tick, even in a tall hall
        x.update_mode(k().survey_interval, 5.0, true);
        assert!(!x.fly_ok, "a doorway lands her at once");
        // a low stretch (under the exit height) also lands her at once
        x.fly_ok = true;
        x.update_mode(k().survey_interval, k().fly_exit_headroom - 0.01, false);
        assert!(!x.fly_ok, "under 3.2 m she walks");
        // back in the air only after k().enter_dwell of clear headroom
        let ticks = (k().enter_dwell / k().survey_interval).ceil() as usize;
        for i in 0..ticks {
            x.update_mode(k().survey_interval, 4.0, false);
            if i + 1 < ticks {
                assert!(!x.fly_ok, "still dwelling after {} ticks", i + 1);
            }
        }
        assert!(x.fly_ok, "clear for the dwell: she takes off");
    }

    #[test]
    fn a_three_metre_room_keeps_her_on_foot_and_the_hysteresis_holds_between_the_thresholds() {
        let mut x = Xivvy::default();
        x.fly_ok = false;
        for _ in 0..30 {
            x.update_mode(k().survey_interval, 3.0, false);
        }
        assert!(!x.fly_ok, "a 3 m room never lifts her");
        // inside the band (3.2 to 3.6) she neither takes off nor lands
        for _ in 0..30 {
            x.update_mode(k().survey_interval, 3.4, false);
        }
        assert!(!x.fly_ok, "between the thresholds a walker stays on foot");
        x.fly_ok = true;
        for _ in 0..30 {
            x.update_mode(k().survey_interval, 3.4, false);
        }
        assert!(x.fly_ok, "between the thresholds a flier stays up");
    }

    #[test]
    fn she_rises_to_her_hover_and_lands_back_to_the_floor() {
        let mut x = Xivvy::default();
        for _ in 0..240 {
            x.survey_timer = 1.0;
            x.cruise_headroom = 5.0;
            x.update_altitude(1.0 / 60.0, true, f32::INFINITY);
        }
        assert!((x.altitude - k().hover_height).abs() < 1e-3, "airborne: {}", x.altitude);
        for _ in 0..600 {
            x.update_altitude(1.0 / 60.0, false, f32::INFINITY);
        }
        assert_eq!(x.altitude, 0.0, "landed");
        assert!(x.lift() <= k().float_amplitude + 1e-6, "no float on the ground");
    }

    #[test]
    fn she_swoops_to_a_grab_near_the_runner_and_climbs_back_when_the_runner_is_far() {
        let cruise = Xivvy::cruise_altitude(5.0);
        assert_eq!(Xivvy::target_altitude(true, 5.0, k().swoop_range), cruise, "at the swoop range she cruises");
        let grab = Xivvy::target_altitude(true, 5.0, k().grab_range);
        assert!((grab - k().swoop_altitude).abs() < 1e-6, "at the grab range she is at the swoop height: {grab}");
        assert_eq!(Xivvy::target_altitude(false, 5.0, k().grab_range), 0.0, "on foot she never lifts");
    }

    #[test]
    fn the_float_stays_inside_the_clearance_and_above_the_floor() {
        let mut x = Xivvy::default();
        // a 3.9 m ceiling: a sliver of clearance above her hover, so the bob must not push her head into it
        x.cruise_headroom = 3.9;
        for _ in 0..600 {
            x.update_altitude(1.0 / 60.0, true, f32::INFINITY);
            let clearance = 3.9 - k().body_height - k().ceiling_gap;
            assert!(x.float_bob <= (clearance - x.altitude).max(0.0) + 1e-6, "bob {} past clearance", x.float_bob);
            assert!(x.lift() >= 0.0, "never below the floor: {}", x.lift());
        }
        assert!(x.float_sway.abs() <= k().float_amplitude + 1e-6);
    }

    #[test]
    fn the_sway_runs_across_the_heading() {
        let mut x = Xivvy::default();
        x.float_sway = 0.05;
        let (sx, sz) = x.sway_offset((0.0, 1.0));
        assert!((sx.abs() - 0.05).abs() < 1e-6 && sz.abs() < 1e-6, "sway across a +z heading: {sx}, {sz}");
        assert_eq!(x.sway_offset((0.0, 0.0)), (0.0, 0.0), "no heading, no sway");
    }

    #[test]
    fn the_survey_finds_a_doorway_in_a_walled_corridor_and_not_in_an_open_hall() {
        let cat = Catalog::load();
        let mut f = Facility::generate("XIVVY-HEAD-1", &Options::default(), &cat);
        // a walled grid with a 3-cell-wide corridor along y 8..=10, from x 2 to x 17
        for row in f.wall.iter_mut() {
            for w in row.iter_mut() {
                *w = true;
            }
        }
        for y in 8..=10 {
            for x in 2..=17 {
                f.wall[y as usize][x as usize] = false;
            }
        }
        let pos = (2.5, 9.5);
        let facing = (1.0, 0.0);
        let open = survey(&f, pos, facing, &|_| None);
        assert!(!open.gap, "a 3-cell corridor is not a doorway");
        // a doorway at x 10: walls on the corridor's north and south cells
        f.wall[8][10] = true;
        f.wall[10][10] = true;
        let door = survey(&f, pos, facing, &|_| None);
        assert!(door.gap, "a doorway 8 m ahead lands her");
        assert!(door.far.is_finite() && door.near.is_finite(), "the headroom is read over the route");
    }

    #[test]
    fn the_headroom_is_the_region_wall_height_less_the_floor() {
        let cat = Catalog::load();
        let f = Facility::generate("XIVVY-HEAD-2", &Options::default(), &cat);
        let cell = f.floor_cells()[0];
        let c = (cell.x, cell.y);
        let h = headroom_at(&f, cell.center());
        assert!((h - ceiling_of(&f, c)).abs() < 1e-6);
        assert!(h > 0.0, "a floor cell has headroom: {h}");
    }

    /// The shipped data keeps the Godot flight numbers (xivvy_flight.gd, game_config.gd XIVVY_TAKEOFF_SECONDS).
    #[test]
    fn the_shipped_flight_data_keeps_the_godot_numbers() {
        let f = k();
        assert_eq!(f.takeoff_seconds, 0.65);
        assert!((f.body_height - 1.78 * 1.5).abs() < 1e-5);
        assert_eq!((f.hover_height, f.ceiling_gap), (1.2, 0.14));
        assert_eq!((f.fly_enter_headroom, f.fly_exit_headroom, f.enter_dwell), (3.6, 3.2, 0.45));
        assert_eq!(f.survey_distances.len(), 8);
        assert!(f.fly_exit_headroom <= f.fly_enter_headroom, "hysteresis band points the right way");
    }

    /// A data edit changes the flight; a bad value keeps the default and the exit never passes the entry.
    #[test]
    fn a_data_edit_changes_the_hover_and_bad_values_keep_the_default() {
        let f = Flight::parse(r#"{"hover_height": 0.5, "fly_enter_headroom": 3.0, "fly_exit_headroom": 4.0, "rise_rate": -1, "survey_distances": []}"#);
        assert_eq!(f.hover_height, 0.5);
        assert_eq!(f.fly_exit_headroom, 3.0, "exit is clamped to the entry");
        assert_eq!(f.rise_rate, 3.4, "a negative rate keeps the default");
        assert_eq!(f.survey_distances.len(), 8, "an empty list keeps the default");
        assert_eq!(Flight::parse("nope"), Flight::parse("{}"));
    }

    /// A 4.0 m ceiling (the sewer wall height, world/sewer_height_tests.rs) leaves her room to hover with her head
    /// under it, and the bob never lifts her head into it; she cannot take off there (under the 3.6 m entry it
    /// could, 4.0 is above it) but must never clip.
    #[test]
    fn xivvy_fits_a_four_metre_sewer_ceiling() {
        let sewer = crate::content::regions::table().def("sewer").and_then(|d| d.geometry).map_or(WALL_HEIGHT, |g| g.wall_height);
        assert_eq!(sewer, 4.0, "the sewer ceiling the test reads");
        let hover = Xivvy::cruise_altitude(sewer);
        assert!(hover > 0.0 && hover <= k().hover_height);
        assert!(hover + k().body_height + k().ceiling_gap <= sewer + 1e-4, "head clear of the ceiling: hover {hover}");
        let mut x = Xivvy::default();
        x.cruise_headroom = sewer;
        for _ in 0..600 {
            x.update_altitude(1.0 / 60.0, true, f32::INFINITY);
            assert!(x.lift() + k().body_height <= sewer + 1e-3, "head at {} under a {sewer} m ceiling", x.lift() + k().body_height);
        }
    }

    /// She is on the ground before a doorway: one survey tick that sees a gap ends the flight mode, and the
    /// altitude then eases back to the floor without a dwell.
    #[test]
    fn she_lands_before_the_door() {
        let mut x = Xivvy::default();
        x.flying = true;
        x.cruise_headroom = 5.0;
        for _ in 0..240 {
            x.survey_timer = 1.0;
            x.update_altitude(1.0 / 60.0, x.airborne(), f32::INFINITY);
        }
        assert!(x.lift() > 0.5, "up in a tall hall");
        x.update_mode(k().survey_interval, 5.0, true);
        assert!(!x.airborne(), "the doorway grounds her at once");
        for _ in 0..600 {
            x.survey_timer = 1.0;
            x.update_altitude(1.0 / 60.0, x.airborne(), f32::INFINITY);
        }
        assert_eq!(x.altitude, 0.0, "down on the floor");
    }
}
