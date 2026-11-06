//! Stress mode: `last_route --stress SECONDS --stress-log PATH --stress-scenario NAME --stress-vsync
//! on|off` (main.rs reads the flags; SECONDS defaults to 60, PATH to `stress.csv`, NAME to `fly`,
//! vsync to off). The scenarios are listed on `Scenario`.
//!
//! A solo run on the fixed seed `SEED` = 20261101 (room text `route::room_text(SEED)`, the same seed
//! the screenshot runs use), the Infinite route from the top level (the start CREATE ROOM gives). It
//! skips the title and the boot card, sets vsync from `--stress-vsync` (off by default) and does not
//! apply the FPS LIMIT, so the frame times are the game's own. The saved settings file is never
//! written. The menus, the pause menu and the admin tools take no input in this mode, and a run that
//! is caught ends there, with the summary covering the run up to the catch.
//!
//! The fly scenario flies, as the Godot flythrough does: admin noclip is on and the run speed is
//! `FLY_SPEED` (`fly`), so walls do not stop the runner and it covers ground fast. The other
//! scenarios walk the same route with noclip off at normal speed, and steer round walls with whiskers
//! (`whisker_pick`). The synthetic input is a
//! `Controls` fed to `Game::update` the way `read_controls` feeds the keyboard and mouse: forward all
//! the time, no sprint, and a turn of `TURN_DEGREES` at each `LEG_SECONDS` boundary. The route is a
//! pure function of the elapsed time, and each turn's direction comes from the fixed seed, so the
//! run is repeatable and keeps reaching new ground.
//!
//! The log is CSV: a header, one row per whole second (`second` counts from 0), then a
//! `# summary` line once SECONDS have passed (it ends with the scenario and vsync). Each row is
//! flushed as it is written, so a crash leaves the rows so far. The columns are `second, fps (frames
//! that second), ms_avg, ms_p99` (the 99th percentile frame time), then `update_ms, render_ms,
//! hud_ms`, `hunters` (active), `depth` and `travelled_m` (metres the runner has moved in the run so
//! far, 1 cell = 1 m, counted from its absolute cell so it keeps counting when the window moves). The
//! three section times are the perf sections' smoothed values, sampled once a frame and averaged
//! over the second.

use super::{perf, World};
use crate::config::{Difficulty, HUNTER_RADIUS, PLAYER_RADIUS};
use crate::content::Content;
use crate::game::{Controls, Game};
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Gimmick;
use crate::world::route;
use macroquad::prelude::Vec2;
use std::fs::File;
use std::io::{BufWriter, Write};
use crate::shared::clock::Instant;

/// The run's seed. The screenshot runs start on the same seed.
const SEED: u32 = 20261101;
/// The run speed (the admin RUN SPEED multiplier) while the runner flies.
const FLY_SPEED: f32 = 4.0;
/// Seconds of straight flight between turns: the route turns at each multiple of this after the start.
const LEG_SECONDS: f32 = 20.0;
/// The size of each turn, to the left or the right by the seed's sign (`turn_sign`).
const TURN_DEGREES: f32 = 110.0;
/// Whiskers: each walking frame the runner probes headings at these offsets (degrees) from the route
/// heading, out to WHISKER_METRES in WHISKER_STEP_METRES steps, and steers toward the open one closest
/// to the route. The side last steered to wins ties, and an offset is held WHISKER_HOLD_SECONDS so the
/// steering does not flip back and forth at a corner.
const WHISKER_DEGREES: [f32; 7] = [-90.0, -60.0, -30.0, 0.0, 30.0, 60.0, 90.0];
const WHISKER_METRES: f32 = 2.5;
const WHISKER_STEP_METRES: f32 = 0.25;
const WHISKER_HOLD_SECONDS: f32 = 0.4;
/// The most the steering moves the heading off the route, in rad per second.
const STEER_RATE: f32 = 6.0;
/// With every heading shut the runner backs off for BACK_SECONDS, then turns AVOID_DEGREES over
/// AVOID_SECONDS and goes on.
const BACK_SECONDS: f32 = 0.5;
const AVOID_DEGREES: f32 = 150.0;
const AVOID_SECONDS: f32 = 1.0;
/// Seconds between presses on a shut door in the walking modes.
const DOOR_PRESS_SECONDS: f32 = 0.7;
/// Seconds over which a turn swings round, from its leg boundary.
const TURN_SECONDS: f32 = 0.25;
/// The NPCs the npcs scenario keeps near the runner, and their smartness (the admin's default).
const NPC_TARGET: usize = 8;
const NPC_SMART: i32 = 70;
/// The active hunters the hunters scenario keeps up.
const HUNTER_TARGET: usize = 6;
/// Summons spread on a golden-angle ring round the runner (the admin's ADMIN_HUNTER_RING_*).
const RING_START: f32 = 2.5;
const RING_STEP: f32 = 1.2;
const RING_ANGLE: f32 = 2.39996;
/// Seconds between interact presses in the interact scenario.
const INTERACT_SECONDS: f32 = 3.0;
/// The mixed scenario: this many seconds of flight, then a phase of `MIXED_PHASE_SECONDS` for each
/// of npcs, hunters and interact in turn.
const MIXED_FLY_SECONDS: f32 = 20.0;
const MIXED_PHASE_SECONDS: f32 = 10.0;
const HEADER: &str =
    "second,fps,ms_avg,ms_p99,update_ms,render_ms,hud_ms,hunters,depth,travelled_m";

/// What the run does besides moving along the route.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scenario {
    /// Noclip, 4x speed, nothing else (the default).
    Fly,
    /// Walk with 8 NPCs kept near the runner.
    Npcs,
    /// Walk with 6 hunters kept active, summoned as the admin does.
    Hunters,
    /// Walk, and press interact on the focus every 3 s.
    Interact,
    /// Fly 20 s, then npcs, hunters and interact in turn, 10 s each.
    Mixed,
}

impl Scenario {
    pub fn parse(s: &str) -> Option<Scenario> {
        Some(match s {
            "fly" => Scenario::Fly,
            "npcs" => Scenario::Npcs,
            "hunters" => Scenario::Hunters,
            "interact" => Scenario::Interact,
            "mixed" => Scenario::Mixed,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Scenario::Fly => "fly",
            Scenario::Npcs => "npcs",
            Scenario::Hunters => "hunters",
            Scenario::Interact => "interact",
            Scenario::Mixed => "mixed",
        }
    }

    /// What the run does `t` seconds in: mixed picks its phase here, the others are themselves.
    fn mode(self, t: f32) -> Scenario {
        if self != Scenario::Mixed {
            return self;
        }
        if t < MIXED_FLY_SECONDS {
            return Scenario::Fly;
        }
        match ((t - MIXED_FLY_SECONDS) / MIXED_PHASE_SECONDS) as u32 % 3 {
            0 => Scenario::Npcs,
            1 => Scenario::Hunters,
            _ => Scenario::Interact,
        }
    }
}

/// What `--stress` asked for.
pub struct Config {
    pub seconds: f32,
    pub log: String,
    pub scenario: Scenario,
    /// The vsync for the run, at window creation and live (`--stress-vsync`).
    pub vsync: bool,
    /// The frame cap for the run in frames per second (`--stress-limit N`); None is uncapped. Measures the FPS
    /// LIMIT row's pacer above the display's refresh rate.
    pub fps_limit: Option<u32>,
}

/// The stress run: its clock, the open log, the second being counted and every frame time.
pub struct Stress {
    seconds: f32,
    scenario: Scenario,
    vsync: bool,
    /// The interact tick already handled, so each `INTERACT_SECONDS` presses once.
    interact_tick: u32,
    /// Hunters summoned so far, for the brain seeds.
    summoned: u32,
    /// The mode of the last frame, to catch the move from flying to walking.
    last_mode: Option<Scenario>,
    log: BufWriter<File>,
    /// Set on the first frame of the run, so the load before it is not counted.
    start: Option<Instant>,
    /// The whole second being counted; its row is written when the clock passes it.
    second: u32,
    row: Row,
    /// Every frame of the run in ms, for the summary.
    frame_ms: Vec<f32>,
    /// The hunters and depth at the last frame, written into each row.
    hunters: usize,
    depth: i32,
    travel: Travel,
    /// The route heading (the facing the route alone would give), in rad; it turns with the route only.
    route: Option<f32>,
    /// The whisker steering of the walking runner.
    steer: Steer,
    /// Set while a runner with every heading shut backs off and turns.
    recover: Option<Recover>,
    /// When the runner last pressed interact on a shut door (seconds into the run).
    door_press: f32,
}

/// The whisker steering: the offset from the route heading the runner steers to (rad), the side it
/// last steered to (+1 or -1, 0 before any steer), and the time the offset may change again.
#[derive(Default, Clone, Copy)]
struct Steer {
    offset: f32,
    side: f32,
    hold_until: f32,
}

/// A runner with every heading shut: it backs off until `back_until`, then turns at `rate` (rad per
/// second) until `turn_until`.
#[derive(Clone, Copy)]
struct Recover {
    back_until: f32,
    turn_until: f32,
    rate: f32,
}

/// The frames of one second: their times and the sum of each section's samples.
#[derive(Default)]
struct Row {
    frame_ms: Vec<f32>,
    update_ms: f32,
    render_ms: f32,
    hud_ms: f32,
}

/// The metres the runner has flown, summed from its steps between frames in absolute cells.
#[derive(Default)]
struct Travel {
    last: Option<(f32, f32)>,
    metres: f32,
}

impl Travel {
    /// Adds the step from the last frame's cell to `cell`. A frame with no game clears the last cell,
    /// so the first cell of the next game starts a new path and adds nothing.
    fn add(&mut self, cell: Option<(f32, f32)>) {
        if let (Some(last), Some(cell)) = (self.last, cell) {
            self.metres += (cell.0 - last.0).hypot(cell.1 - last.1);
        }
        self.last = cell;
    }
}

impl Stress {
    /// Creates the log and writes its header. The clock starts on the first frame.
    pub fn open(cfg: Config) -> Result<Stress, String> {
        if let Some(dir) = std::path::Path::new(&cfg.log).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = File::create(&cfg.log).map_err(|e| format!("cannot create {}: {e}", cfg.log))?;
        let mut log = BufWriter::new(file);
        writeln!(log, "{}", HEADER)
            .and_then(|()| log.flush())
            .map_err(|e| format!("cannot write {}: {e}", cfg.log))?;
        crate::lr_info!(
            "stress run: {} s on seed {}, scenario {}, vsync {}, log {}",
            cfg.seconds,
            SEED,
            cfg.scenario.name(),
            on_off(cfg.vsync),
            cfg.log
        );
        Ok(Stress {
            seconds: cfg.seconds,
            scenario: cfg.scenario,
            vsync: cfg.vsync,
            interact_tick: 0,
            summoned: 0,
            last_mode: None,
            log,
            start: None,
            second: 0,
            row: Row::default(),
            frame_ms: Vec::new(),
            hunters: 0,
            depth: 0,
            travel: Travel::default(),
            route: None,
            steer: Steer::default(),
            recover: None,
            door_press: -DOOR_PRESS_SECONDS,
        })
    }

    /// Counts one frame of `dt` seconds, with the run `g` (none while a run is being built).
    /// Returns true once SECONDS have passed.
    pub fn frame(&mut self, dt: f32, g: Option<&Game>) -> bool {
        let Some(start) = self.start else {
            self.start = Some(Instant::now());
            return false;
        };
        let t = start.elapsed().as_secs_f32();
        let whole = t as u32;
        while self.second < whole {
            self.write_row();
        }
        let ms = dt * 1000.0;
        self.row.frame_ms.push(ms);
        self.frame_ms.push(ms);
        self.row.update_ms += perf::section_ms("update");
        self.row.render_ms += perf::section_ms("render 3d");
        self.row.hud_ms += perf::section_ms("hud");
        let (hunters, depth) = g.map_or((0, 0), |g| {
            (g.hunters.iter().filter(|h| h.active).count(), g.depth)
        });
        self.hunters = hunters;
        self.depth = depth;
        let cell = g.map(|g| {
            let (ox, oy) = g.window_origin();
            (g.p.pos.0 + ox as f32, g.p.pos.1 + oy as f32)
        });
        self.travel.add(cell);
        t >= self.seconds
    }

    /// The synthetic input for this frame; `dt` is the frame's game time and `g` the run.
    pub fn controls(&mut self, dt: f32, g: &Game) -> Controls {
        let t = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let turn = turn_look(t, dt);
        let mut c = Controls {
            fwd: 1.0,
            look: Vec2::new(turn, 0.0),
            ..Default::default()
        };
        // the route heading turns with the route alone; the facing is the game's own yaw
        let facing = facing_angle(g.p.yaw);
        let route_prev = self.route.unwrap_or(facing);
        self.route = Some(route_prev + turn);
        if self.scenario.mode(t) != Scenario::Fly {
            self.steer(g, t, dt, facing, route_prev, &mut c);
        }
        // every INTERACT_SECONDS in the interact mode: press interact when the crosshair is on
        // something (or to step out of a locker the last press hid the runner in)
        let tick = (t / INTERACT_SECONDS) as u32;
        if tick != self.interact_tick {
            self.interact_tick = tick;
            if self.scenario.mode(t) == Scenario::Interact
                && (g.focus.is_some() || g.p.hidden.is_some())
            {
                c.interact = true;
            }
        }
        c
    }

    /// The walking runner's steering for one frame. A shut door on the way is pressed (never opened). With
    /// every heading shut the runner backs off and turns; otherwise it steers toward the open whisker
    /// heading closest to the route. `facing` is its yaw as a heading, `route_prev` the route heading
    /// before this frame's turn. The look is the route's turn plus the steer, so with an open route the
    /// runner follows the route exactly.
    fn steer(&mut self, g: &Game, t: f32, dt: f32, facing: f32, route_prev: f32, c: &mut Controls) {
        // a shut door on the way never opens for the runner: the crosshair on one is a press, held off
        // for DOOR_PRESS_SECONDS so the next press does not shut it again
        if g.closed_door_in_focus() && t - self.door_press >= DOOR_PRESS_SECONDS {
            self.door_press = t;
            c.interact = true;
        }
        if let Some(r) = self.recover {
            if t < r.back_until {
                c.fwd = -1.0;
                return;
            }
            if t < r.turn_until {
                c.look.x += r.rate * dt;
                // the route turns with the facing, so it leaves the dead-end heading for good
                self.route = Some(recovery_route(self.route.unwrap_or(route_prev), r.rate, dt));
                return;
            }
            self.recover = None;
        }
        let route = self.route.unwrap_or(route_prev);
        let open = |x: f32, y: f32| g.fac.body_fits(x, y, PLAYER_RADIUS);
        let Some(want) = whisker_pick(g.p.pos, route, self.steer.side, &open) else {
            let side = if self.steer.side != 0.0 {
                self.steer.side
            } else {
                turn_sign(t as u32)
            };
            self.recover = Some(Recover {
                back_until: t + BACK_SECONDS,
                turn_until: t + BACK_SECONDS + AVOID_SECONDS,
                rate: side * AVOID_DEGREES.to_radians() / AVOID_SECONDS,
            });
            c.fwd = -1.0;
            return;
        };
        // an offset is held for a moment after it is chosen, so the runner does not flip back at once
        let keep = t < self.steer.hold_until
            && whisker_clear(g.p.pos, route + self.steer.offset, &open);
        let target = if keep { self.steer.offset } else { want };
        if target != self.steer.offset {
            self.steer.hold_until = t + WHISKER_HOLD_SECONDS;
        }
        if target != 0.0 {
            self.steer.side = target.signum();
        }
        self.steer.offset = target;
        // the facing now is route_prev + cur; the steer turns it toward the target at most STEER_RATE
        let cur = wrap_angle(facing - route_prev);
        let step = wrap_angle(target - cur).clamp(-STEER_RATE * dt, STEER_RATE * dt);
        c.look.x += step;
    }

    /// Sets the run up for this frame, before the update: the flight switches (noclip and
    /// `FLY_SPEED`) in the flying mode, none in the walking ones, and the NPCs or hunters the mode
    /// keeps up. A runner that stops flying inside a wall is moved to the nearest open floor. Called
    /// each frame, so a game that starts after a level change keeps the mode.
    pub fn prepare(&mut self, g: &mut Game, content: &Content) {
        let t = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let mode = self.scenario.mode(t);
        if mode == Scenario::Fly {
            fly(g);
        } else {
            g.admin.noclip = false;
            g.admin.run_speed = 1.0;
            if self.last_mode == Some(Scenario::Fly)
                && !g.fac.body_fits(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS)
            {
                if let Some(at) = g.fac.nearest_open(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS) {
                    g.p.pos = at;
                }
            }
        }
        self.last_mode = Some(mode);
        if mode != Scenario::Npcs {
            // the run's own NPCs (data/npcs.json) would skew the fly and hunters timings; only the admin's stay
            g.npcs.retain(|n| n.key.is_none());
        }
        match mode {
            Scenario::Npcs => keep_npcs(g),
            Scenario::Hunters => self.keep_hunters(g, content),
            _ => {}
        }
    }

    /// Tops the active hunters up to `HUNTER_TARGET`, placed as the admin summon places them.
    fn keep_hunters(&mut self, g: &mut Game, content: &Content) {
        if content.hunters.is_empty() || g.report.is_some() {
            return;
        }
        let mut have = g.hunters.iter().filter(|h| h.active).count();
        let mut slot = 0usize;
        // a bounded number of tries, so a floor with no room cannot stall the frame
        while have < HUNTER_TARGET && slot < HUNTER_TARGET {
            let def = &content.hunters[self.summoned as usize % content.hunters.len()];
            if summon(g, &def.id, content, self.summoned, slot) {
                have += 1;
            }
            self.summoned += 1;
            slot += 1;
        }
    }

    /// Writes the summary to the log and to stdout. The caller ends the process.
    pub fn finish(&mut self) {
        let secs = self.start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let total = self.frame_ms.len();
        let avg_fps = if secs > 0.0 { total as f32 / secs } else { 0.0 };
        let worst = self.frame_ms.iter().copied().fold(0.0, f32::max);
        let line = format!(
            "# summary total_frames={} avg_fps={:.2} low_1pct_fps={:.2} worst_frame_ms={:.2} scenario={} vsync={}",
            total,
            avg_fps,
            one_percent_low_fps(&self.frame_ms),
            worst,
            self.scenario.name(),
            on_off(self.vsync)
        );
        self.emit(&line);
        println!("{}", line);
    }

    /// Writes the row for `self.second` and moves on to the next second.
    fn write_row(&mut self) {
        let row = std::mem::take(&mut self.row);
        let frames = row.frame_ms.len();
        let mean = |sum: f32| if frames == 0 { 0.0 } else { sum / frames as f32 };
        let ms_avg = mean(row.frame_ms.iter().sum::<f32>());
        let line = format!(
            "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{:.2}",
            self.second,
            frames,
            ms_avg,
            percentile(&row.frame_ms, 99.0),
            mean(row.update_ms),
            mean(row.render_ms),
            mean(row.hud_ms),
            self.hunters,
            self.depth,
            self.travel.metres,
        );
        self.emit(&line);
        self.second += 1;
    }

    /// Appends one line to the log and flushes it.
    fn emit(&mut self, line: &str) {
        if let Err(e) = writeln!(self.log, "{}", line).and_then(|()| self.log.flush()) {
            eprintln!("stress: cannot write the log: {e}");
        }
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// The game a stress run plays: the Infinite route from the top level, as CREATE ROOM starts one.
pub fn start_game(w: &World) -> Game {
    Game::new_infinite(
        Difficulty::Standard,
        SEED,
        &route::room_text(SEED),
        0,
        &w.content,
        &w.catalog,
        &w.assets,
    )
}

/// Turns the flight on for `g`: admin noclip and the `FLY_SPEED` run speed, set on the game's own
/// admin switches (the playtest bridge's `noclip` and `speed` helpers), which `admin_flags` merges
/// with the admin menu's. The saved admin preferences are never written.
fn fly(g: &mut Game) {
    g.admin.noclip = true;
    g.admin.run_speed = FLY_SPEED;
}

/// Keeps `NPC_TARGET` NPCs: spawns the missing ones as the admin SPAWN does (round the runner), and
/// removes the farthest from the runner while there are more.
fn keep_npcs(g: &mut Game) {
    if g.npcs.len() < NPC_TARGET {
        let eye = g.eye();
        let (fwd, _, _) = g.camera_basis();
        let missing = (NPC_TARGET - g.npcs.len()) as i32;
        g.npc_spawn(missing, NPC_SMART, (eye.x, eye.y, eye.z), (fwd.x, fwd.y, fwd.z));
    }
    while g.npcs.len() > NPC_TARGET {
        let runner = g.p.pos;
        let dist = |i: usize| crate::game::pdist(g.npcs[i].at, runner);
        let far = (0..g.npcs.len()).max_by(|&a, &b| dist(a).total_cmp(&dist(b)));
        match far {
            Some(i) => {
                g.npcs.remove(i);
            }
            None => break,
        }
    }
}

/// The admin's `summon` (ui/admin/actions.rs) for one hunter id: a brain on the golden-angle ring
/// round the runner, awake and heading for the runner. Returns false when there is no open floor.
pub fn summon(g: &mut Game, id: &str, content: &Content, seq: u32, slot: usize) -> bool {
    let Some(def) = content.hunter(id) else {
        return false;
    };
    let (fx, fy) = g.forward2();
    let base = fx.atan2(-fy);
    let angle = base + slot as f32 * RING_ANGLE;
    let radius = RING_START + RING_STEP * (slot as f32).sqrt();
    let want = (g.p.pos.0 + angle.cos() * radius, g.p.pos.1 + angle.sin() * radius);
    let Some(at) = g
        .fac
        .nearest_open(want.0, want.1, HUNTER_RADIUS)
        .or_else(|| g.fac.nearest_open(g.p.pos.0, g.p.pos.1, HUNTER_RADIUS))
    else {
        return false;
    };
    let mut b = Brain::new(
        def.clone(),
        at,
        &format!("{}:STRESS:{id}:{seq}", g.level_seed()),
        g.difficulty,
    );
    if let Gimmick::Solyth(s) = &mut b.gimmick {
        s.heat_locked = g.meters.heat_locked;
    }
    b.activate_at(g.p.pos);
    g.hunters.push(b);
    true
}

/// The sign of the turn at leg boundary `k` (the first boundary is 1): a fixed multiplicative hash, so
/// the direction sequence is the same on every run.
fn turn_sign(k: u32) -> f32 {
    let h = k.wrapping_mul(2_654_435_761).wrapping_add(SEED);
    if h >> 31 == 0 {
        1.0
    } else {
        -1.0
    }
}

/// The route's heading in radians at `t` seconds from the start. There is no turn before the first
/// leg boundary; each boundary after it turns the heading by `TURN_DEGREES` in its `turn_sign`
/// direction, swung over `TURN_SECONDS`. Positive turns right (`Player::look` subtracts it from yaw).
fn heading(t: f32) -> f32 {
    let t = t.max(0.0);
    let leg = (t / LEG_SECONDS) as u32;
    if leg == 0 {
        return 0.0;
    }
    let done: f32 = (1..leg).map(turn_sign).sum();
    let swing = ((t - leg as f32 * LEG_SECONDS) / TURN_SECONDS).clamp(0.0, 1.0);
    (done + turn_sign(leg) * swing) * TURN_DEGREES.to_radians()
}

/// The look for a frame at `t` seconds from the start: the heading turned over the frame's `dt`.
fn turn_look(t: f32, dt: f32) -> f32 {
    heading(t) - heading(t - dt)
}

/// The heading (the angle of the forward vector `(cos, sin)` on the grid) for a yaw. A look of `d`
/// turns the yaw by `-d` and the heading by `+d`, so the route's turns carry over to the facing.
fn facing_angle(yaw: f32) -> f32 {
    (-yaw.cos()).atan2(-yaw.sin())
}

/// `a` wrapped into (-PI, PI].
fn wrap_angle(a: f32) -> f32 {
    a.sin().atan2(a.cos())
}

/// The route heading after one frame of a recovery turn at `rate` rad/s: the route turns with the
/// facing, so a runner turned out of a dead end keeps its route pointing away from it.
fn recovery_route(route: f32, rate: f32, dt: f32) -> f32 {
    route + rate * dt
}

/// Whether a walk along heading `angle` from `pos` stays open out to WHISKER_METRES.
fn whisker_clear(pos: (f32, f32), angle: f32, open: &impl Fn(f32, f32) -> bool) -> bool {
    let (dx, dy) = (angle.cos(), angle.sin());
    let steps = (WHISKER_METRES / WHISKER_STEP_METRES).round() as u32;
    (1..=steps).all(|k| {
        let d = k as f32 * WHISKER_STEP_METRES;
        open(pos.0 + dx * d, pos.1 + dy * d)
    })
}

/// The whisker pick from `pos` on the route heading `route`: the offset (rad) from the route of the
/// open whisker closest to it. Ties go to the side `prefer` (+1 or -1; 0 for none). None when every
/// whisker is shut.
fn whisker_pick(
    pos: (f32, f32),
    route: f32,
    prefer: f32,
    open: &impl Fn(f32, f32) -> bool,
) -> Option<f32> {
    // closest first, and at equal distance the preferred side first (a penalty for the other side)
    let rank = |deg: f32| deg.abs() * 2.0 + if deg * prefer < 0.0 { 1.0 } else { 0.0 };
    let mut order = WHISKER_DEGREES;
    order.sort_by(|a, b| rank(*a).total_cmp(&rank(*b)));
    order
        .into_iter()
        .map(|deg| deg.to_radians())
        .find(|o| whisker_clear(pos, route + o, open))
}

/// The `p`th percentile (0 to 100) of frame times in ms, by nearest rank; 0 when there are none.
fn percentile(ms: &[f32], p: f32) -> f32 {
    if ms.is_empty() {
        return 0.0;
    }
    let mut sorted = ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (p * sorted.len() as f32 / 100.0).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The 1% low in fps, as the HUD computes it: the mean of the slowest 1% of frames (at least one),
/// as an fps.
fn one_percent_low_fps(frame_ms: &[f32]) -> f32 {
    if frame_ms.is_empty() {
        return 0.0;
    }
    let mut sorted = frame_ms.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let k = (sorted.len() / 100).max(1);
    let low_ms = sorted[..k].iter().sum::<f32>() / k as f32;
    1000.0 / low_ms.max(1e-4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_and_one_percent_low_from_frame_times() {
        let ms: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        assert_eq!(percentile(&ms, 99.0), 99.0);
        assert_eq!(percentile(&ms, 100.0), 100.0);
        let none: Vec<f32> = Vec::new();
        assert_eq!(percentile(&none, 99.0), 0.0);
        // 200 frames at 10 ms with two 50 ms hitches: the slowest 1% (two frames) averages 50 ms
        let mut frames = vec![10.0f32; 200];
        frames[3] = 50.0;
        frames[90] = 50.0;
        assert!((one_percent_low_fps(&frames) - 20.0).abs() < 1e-3);
        assert!((one_percent_low_fps(&[10.0f32; 50]) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn route_turns_at_each_leg_boundary_and_holds_between() {
        let turn = TURN_DEGREES.to_radians();
        assert_eq!(heading(0.0), 0.0);
        assert_eq!(heading(LEG_SECONDS - 0.5), 0.0);
        for k in 1..=12u32 {
            let boundary = k as f32 * LEG_SECONDS;
            let before = heading(boundary - 0.001);
            let after = heading(boundary + TURN_SECONDS + 0.001);
            assert!((after - before - turn_sign(k) * turn).abs() < 1e-3, "boundary {k}");
            let next_boundary = heading(boundary + LEG_SECONDS - 0.001);
            assert!((next_boundary - after).abs() < 1e-3, "leg after boundary {k}");
        }
    }

    #[test]
    fn heading_is_a_pure_function_of_time_with_both_turn_directions() {
        let t = 137.3;
        assert_eq!(heading(t), heading(t));
        assert_eq!(turn_look(t, 1.0 / 60.0), turn_look(t, 1.0 / 60.0));
        let signs: Vec<f32> = (1..=16u32).map(turn_sign).collect();
        assert!(signs.contains(&1.0) && signs.contains(&-1.0));
    }

    #[test]
    fn scenarios_parse_and_mixed_cycles_its_phases() {
        for n in ["fly", "npcs", "hunters", "interact", "mixed"] {
            assert_eq!(Scenario::parse(n).unwrap().name(), n);
        }
        assert!(Scenario::parse("nope").is_none());
        assert_eq!(Scenario::Mixed.mode(5.0), Scenario::Fly);
        assert_eq!(Scenario::Mixed.mode(25.0), Scenario::Npcs);
        assert_eq!(Scenario::Mixed.mode(35.0), Scenario::Hunters);
        assert_eq!(Scenario::Mixed.mode(45.0), Scenario::Interact);
        assert_eq!(Scenario::Mixed.mode(55.0), Scenario::Npcs);
        assert_eq!(Scenario::Npcs.mode(5.0), Scenario::Npcs);
    }

    #[test]
    fn travelled_metres_carry_across_a_window_move() {
        // 3 m east and 4 m north in the first window (5 m), then the window moves 10 cells east and
        // the runner goes 4 m north: its window x goes from 3 to -7 and its absolute x stays 3
        let mut travel = Travel::default();
        let abs = |origin: (f32, f32), local: (f32, f32)| (origin.0 + local.0, origin.1 + local.1);
        travel.add(Some(abs((0.0, 0.0), (0.0, 0.0))));
        travel.add(Some(abs((0.0, 0.0), (3.0, 4.0))));
        travel.add(Some(abs((10.0, 0.0), (-7.0, 8.0))));
        assert!((travel.metres - 9.0).abs() < 1e-4);
        // a frame with no game, then a new game far away: nothing is added
        travel.add(None);
        travel.add(Some((50.0, 50.0)));
        assert!((travel.metres - 9.0).abs() < 1e-4);
    }

    #[test]
    fn whiskers_steer_a_runner_against_a_wall_to_the_open_side() {
        // a wall across the route ahead on the y >= 0 side: the open side is y < 0
        let open = |x: f32, y: f32| !(x > 1.0 && y >= 0.0);
        let pick = whisker_pick((0.0, 0.0), 0.0, 0.0, &open).expect("an open heading");
        assert!((pick - (-30f32).to_radians()).abs() < 1e-4, "picked {pick}");
    }

    #[test]
    fn whiskers_keep_the_side_last_steered_to_when_both_are_open() {
        // a wall straight ahead only: +30 and -30 are both open, and the memory breaks the tie
        let open = |x: f32, y: f32| !(x > 1.0 && y.abs() < 0.6);
        let right = whisker_pick((0.0, 0.0), 0.0, 1.0, &open).expect("open");
        let left = whisker_pick((0.0, 0.0), 0.0, -1.0, &open).expect("open");
        assert!((right - 30f32.to_radians()).abs() < 1e-4, "right {right}");
        assert!((left + 30f32.to_radians()).abs() < 1e-4, "left {left}");
    }

    #[test]
    fn a_runner_in_an_open_corridor_keeps_the_route_heading() {
        // a corridor along x, two metres either side: the route heading is open, so no offset
        let corridor = |_: f32, y: f32| y.abs() < 2.0;
        assert_eq!(whisker_pick((0.0, 0.0), 0.0, 0.0, &corridor), Some(0.0));
        assert_eq!(whisker_pick((0.0, 0.0), 0.3, 0.0, &corridor), Some(0.0));
        let open_room = |_: f32, _: f32| true;
        assert_eq!(whisker_pick((5.0, 5.0), -1.2, 1.0, &open_room), Some(0.0));
    }

    #[test]
    fn a_runner_with_every_heading_shut_finds_no_pick() {
        assert_eq!(whisker_pick((0.0, 0.0), 0.0, 0.0, &|_: f32, _: f32| false), None);
    }

    #[test]
    fn a_runner_in_a_dead_end_turns_its_route_out_of_it() {
        // a dead-end hallway along x: open within 1 m of the centre line up to x = 3, shut past it
        let hallway = |x: f32, y: f32| x <= 3.0 && y.abs() < 1.0;
        let pos = (2.5, 0.0);
        // the scripted route points into the dead end: every whisker is shut, so the runner recovers
        assert_eq!(whisker_pick(pos, 0.0, 0.0, &hallway), None);
        // the recovery turn carries the route round by AVOID_DEGREES over AVOID_SECONDS
        let rate = AVOID_DEGREES.to_radians() / AVOID_SECONDS;
        let dt = 1.0 / 60.0;
        let mut route = 0.0;
        for _ in 0..(AVOID_SECONDS / dt).round() as u32 {
            route = recovery_route(route, rate, dt);
        }
        assert!((route - AVOID_DEGREES.to_radians()).abs() < 1e-3, "route {route}");
        // the turned route has an open heading back down the hallway (the +30 whisker, heading 180)
        let pick = whisker_pick(pos, route, 0.0, &hallway).expect("an open heading out");
        assert!((pick - 30f32.to_radians()).abs() < 1e-4, "picked {pick}");
    }

    #[test]
    fn a_look_turns_the_facing_by_the_same_angle_as_the_route() {
        // the look subtracts from the yaw, so the facing moves by the look: the route's turn carries over
        let (yaw, d) = (0.3f32, 0.2f32);
        assert!((wrap_angle(facing_angle(yaw - d) - facing_angle(yaw)) - d).abs() < 1e-5);
    }
}
