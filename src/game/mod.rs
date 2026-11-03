//! One run: the floor, the runner, the inventory, the hunters, and the rules that tie them
//! together (relays -> service lift, captures -> escape struggle -> run report).

pub mod endless;
mod hunting;
mod interact;
#[cfg(test)]
mod ladder_tests;
mod items;
pub mod meters;
pub mod npc;
pub mod player;
mod transition;

pub use interact::Focus;
pub use items::{Item, Slot};

use crate::shared::assets::Assets;
use crate::audio::Sounds;
use crate::config::*;
use crate::content::Content;
use crate::hunters::brain::Brain;
use crate::hunters::director::Director;
use crate::hunters::gimmicks::Gimmick;
use crate::render::world_mesh::{ChunkFrame, WorldMeshes};
use crate::world::catalog::Catalog;
use crate::world::facility::{Facility, Options};
use crate::world::fixtures::{self, Fixtures};
use macroquad::prelude::*;
use meters::Meters;
use player::Player;

/// What the player is asking for this frame (keyboard/mouse, or the self-test's script).
#[derive(Default, Clone)]
pub struct Controls {
    pub fwd: f32,
    pub strafe: f32,
    pub sprint: bool,
    pub crouch: bool,
    pub jump: bool,
    pub interact: bool,
    pub interact_held: bool,
    pub use_item: bool,
    pub flashlight: bool,
    pub taser: bool,
    pub drop: bool,
    pub slot: Option<usize>,
    pub wheel: i32,
    pub look: Vec2,
    /// Digit keys while Zellia's store is open.
    pub shop_choice: Option<usize>,
}

/// The admin switches (admin_menu.gd; the playtest bridge's admin helpers): `god` and
/// `no_target` skip captures and the run ending (no_target: hunters do not target the runner
/// at all), `noclip` lets the body pass walls and props, `run_speed` multiplies movement.
/// Set per run by agents and tests; the in-run admin overlay sets the process-wide
/// `ui::admin::flags()`, which `admin_flags` merges in.
#[derive(Clone, Copy, Debug)]
pub struct Admin {
    pub god: bool,
    pub no_target: bool,
    pub noclip: bool,
    pub run_speed: f32,
    /// The admin HUNTER REVEAL, on and admin-enabled. The admin UI sets it every frame; the maps then draw every hunter on
    /// the floor (`Game::map_hunters`).
    pub hunter_reveal: bool,
}

impl Default for Admin {
    fn default() -> Self {
        Admin {
            god: false,
            no_target: false,
            noclip: false,
            run_speed: 1.0,
            hunter_reveal: false,
        }
    }
}

pub struct FloorPickup {
    pub pos: (f32, f32),
    pub item: Item,
    pub n: i32,
    pub taken: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Heater {
    Frozen,
    Thawed,
    Powered,
}

pub struct Escape {
    pub remaining: f32,
    pub progress: i32,
    pub hunter: usize,
}

pub struct Report {
    pub hunter: String,
    pub hunter_name: String,
    pub cause: &'static str,
    pub depth: i32,
    pub score: i32,
    pub relays: i32,
    pub coins: i32,
    pub seed: u32,
    pub difficulty: Difficulty,
}

pub struct Game {
    pub difficulty: Difficulty,
    pub run_seed: u32,
    /// The run's room seed text (`ROOM-<code>-<hex>`): shown by the menus, the loading screen,
    /// the pause menu and the run report, and the source of the Infinite route's world seed.
    pub room_text: String,
    pub depth: i32,
    pub score: i32,
    pub coins: i32,
    pub fac: Facility,
    pub fix: Fixtures,
    /// Render meshes (built on demand by `sync_world`; empty in headless runs). The lamps are
    /// kept current either way.
    pub world: WorldMeshes,
    world_dirty: bool,
    pub relays_taken: Vec<bool>,
    pub batteries_taken: Vec<bool>,
    pub coins_taken: Vec<bool>,
    pub pickups: Vec<FloorPickup>,
    pub hiding_disabled: Vec<bool>,
    pub lights_on: Vec<bool>,
    pub heater: Heater,
    pub music_box_left: f32,
    pub wind_hold: f32,
    pub relays_collected: i32,
    /// Per-room climate offsets: hot and cold spots on the floor.
    pub climate: Vec<f32>,
    pub discovered: Vec<Vec<bool>>,
    discover_timer: f32,

    pub p: Player,
    pub meters: Meters,
    pub slots: [Option<Slot>; HOTBAR_SIZE],
    pub selected: usize,
    pub taser_cd: f32,
    pub melee_cd: f32,
    pub swing_t: f32,
    pub taser_fx: f32,

    /// 0 = the floor's route hunter, then the dormant specials Nyxie and Solyth.
    pub hunters: Vec<Brain>,
    /// Host-side coordination of the hunters (hunter_director.gd): share / flank / cover /
    /// scatter / relay rush. Its ids are indices into `hunters`, so it is re-created whenever
    /// that list is rebuilt.
    pub director: Director,
    pub escape: Option<Escape>,
    escape_cooldown: f32,
    escape_cooldown_scale: f32,
    escape_grace: f32,
    locker_cooldown: Vec<f32>,
    hide_check: Option<(usize, usize, f32)>,

    pub focus: Option<Focus>,
    pub toast: String,
    pub toast_t: f32,
    pub message: String,
    pub message_t: f32,
    pub report: Option<Report>,
    pub fade: f32,
    pub time: f32,
    pub hits: i32,
    pub shots: i32,
    /// Field of view from the settings.
    pub fov_deg: f32,
    /// The Infinite route's streamed window and levels (None on a finite route).
    pub inf: Option<endless::InfState>,
    pub admin: Admin,
    /// NPC teammates the admin spawned this run (game/npc.rs, phase one), and the next id (never reused).
    pub npcs: Vec<npc::Npc>,
    pub npc_next: u32,
    /// Hunters met this run (within 7.5 m, in line of sight) not yet handed to the dossier.
    sightings: Vec<String>,
    sighted: std::collections::HashSet<String>,
    /// Why the run last autosaved ("relay", "checkpoint", "quit"), "" before any (Godot `_last_autosave_reason`).
    pub last_autosave: &'static str,
    /// The climb to another level and its fade (game/transition.rs): input is frozen while it runs.
    pub transition: Option<transition::Transition>,
}

pub fn pdist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl Game {
    /// A finite route at depth 1, with its render meshes built (the windowed game).
    pub fn new(
        difficulty: Difficulty,
        run_seed: u32,
        content: &Content,
        catalog: &Catalog,
        assets: &Assets,
    ) -> Game {
        let mut g = Game::new_sim(difficulty, run_seed, content, catalog);
        g.sync_world(assets, content);
        g
    }

    /// A finite route at depth 1 without render data (headless runs: no window, no GPU).
    /// `sync_world` builds the meshes later if a renderer wants them.
    pub fn new_sim(
        difficulty: Difficulty,
        run_seed: u32,
        content: &Content,
        catalog: &Catalog,
    ) -> Game {
        let opt = Options {
            safe_zone_count: difficulty.safe_zone_limit(),
            ..Default::default()
        };
        let fac = Facility::generate(&format!("{run_seed}-D1"), &opt, catalog);
        let fix = fixtures::place(&fac);
        let world = WorldMeshes::default();
        let mut g = Game {
            difficulty,
            run_seed,
            room_text: crate::world::route::room_text(run_seed),
            depth: 1,
            score: 0,
            coins: 0,
            relays_taken: vec![],
            batteries_taken: vec![],
            coins_taken: vec![],
            pickups: vec![],
            hiding_disabled: vec![],
            lights_on: vec![],
            heater: Heater::Frozen,
            music_box_left: 45.0,
            wind_hold: 0.0,
            relays_collected: 0,
            climate: vec![],
            discovered: vec![],
            discover_timer: 0.0,
            p: Player::new(fac.start.center()),
            meters: Meters::default(),
            slots: [
                Some(Slot {
                    item: Item::Flashlight,
                    n: 100,
                }),
                None,
                None,
                None,
                None,
            ],
            selected: 0,
            taser_cd: 0.0,
            melee_cd: 0.0,
            swing_t: 0.0,
            taser_fx: 0.0,
            hunters: vec![],
            director: Director::new(difficulty, &format!("{run_seed}-director")),
            escape: None,
            escape_cooldown: 0.0,
            escape_cooldown_scale: 1.0,
            escape_grace: 0.0,
            locker_cooldown: vec![],
            hide_check: None,
            focus: None,
            toast: String::new(),
            toast_t: 0.0,
            message: String::new(),
            message_t: 0.0,
            report: None,
            fade: 1.0,
            time: 0.0,
            hits: 0,
            shots: 0,
            fov_deg: FIELD_OF_VIEW_DEG,
            inf: None,
            admin: Admin::default(),
            npcs: vec![],
            npc_next: 0,
            sightings: vec![],
            sighted: Default::default(),
            last_autosave: "",
            transition: None,
            fac,
            fix,
            world,
            world_dirty: true,
        };
        g.start_floor_sim(content, catalog);
        g
    }

    /// An Infinite route (CREATE ROOM's default): streamed chunks and stairwells between
    /// levels; level 0 is the top and depth = 1 - level.
    pub fn new_infinite(
        difficulty: Difficulty,
        run_seed: u32,
        room_text: &str,
        level: i32,
        content: &Content,
        catalog: &Catalog,
        assets: &Assets,
    ) -> Game {
        let mut g =
            Game::new_infinite_sim(difficulty, run_seed, room_text, level, content, catalog);
        g.sync_world(assets, content);
        g
    }

    /// `new_infinite` without render data (headless runs). `room_text` is the run's room seed
    /// text (a new run's `route::new_room_text`, or `route::room_text(seed)` for a numeric seed);
    /// its djb2 is the Infinite route's world seed, `run_seed` stays the decimal run seed.
    pub fn new_infinite_sim(
        difficulty: Difficulty,
        run_seed: u32,
        room_text: &str,
        level: i32,
        content: &Content,
        catalog: &Catalog,
    ) -> Game {
        // the Infinite route's world seed is str(RoomState.run_seed) of the run's room seed text
        // (route::world_seed); every run start (new run, checkpoint, headless) passes the text
        // the run was made with, so every start reaches the same world
        let seed = crate::world::route::world_seed(room_text);
        // a 1x1 placeholder floor; enter_infinite swaps in the streamed window
        let fac = crate::world::infinite::placeholder(&seed, catalog);
        let mut g = Game::new_shell(difficulty, run_seed, fac, Fixtures::default());
        g.room_text = room_text.to_string();
        g.inf = Some(endless::InfState::new(&seed, level));
        g.enter_infinite(content, catalog);
        g
    }

    fn new_shell(difficulty: Difficulty, run_seed: u32, fac: Facility, fix: Fixtures) -> Game {
        Game {
            difficulty,
            run_seed,
            room_text: crate::world::route::room_text(run_seed),
            depth: 1,
            score: 0,
            coins: 0,
            relays_taken: vec![],
            batteries_taken: vec![],
            coins_taken: vec![],
            pickups: vec![],
            hiding_disabled: vec![],
            lights_on: vec![],
            heater: Heater::Frozen,
            music_box_left: 45.0,
            wind_hold: 0.0,
            relays_collected: 0,
            climate: vec![],
            discovered: vec![],
            discover_timer: 0.0,
            p: Player::new((0.5, 0.5)),
            meters: Meters::default(),
            slots: [
                Some(Slot {
                    item: Item::Flashlight,
                    n: 100,
                }),
                None,
                None,
                None,
                None,
            ],
            selected: 0,
            taser_cd: 0.0,
            melee_cd: 0.0,
            swing_t: 0.0,
            taser_fx: 0.0,
            hunters: vec![],
            director: Director::new(difficulty, &format!("{run_seed}-director")),
            escape: None,
            escape_cooldown: 0.0,
            escape_cooldown_scale: 1.0,
            escape_grace: 0.0,
            locker_cooldown: vec![],
            hide_check: None,
            focus: None,
            toast: String::new(),
            toast_t: 0.0,
            message: String::new(),
            message_t: 0.0,
            report: None,
            fade: 1.0,
            time: 0.0,
            hits: 0,
            shots: 0,
            fov_deg: FIELD_OF_VIEW_DEG,
            inf: None,
            admin: Admin::default(),
            npcs: vec![],
            npc_next: 0,
            sightings: vec![],
            sighted: Default::default(),
            last_autosave: "",
            transition: None,
            world: WorldMeshes::default(),
            world_dirty: true,
            fac,
            fix,
        }
    }

    /// A fresh director for a rebuilt hunter list (main_3d.gd makes one per level).
    pub(crate) fn reset_director(&mut self) {
        self.director = Director::new(self.difficulty, &format!("{}-director", self.run_seed));
    }

    /// The hunters the minimap and the full map draw (Godot `_streamed_map_marks`): Porcelle while she is awake, and every
    /// hunter on the floor while the admin HUNTER REVEAL is on. The maps dim the ones that are asleep.
    pub fn map_hunters(&self) -> impl Iterator<Item = &Brain> + '_ {
        let reveal = self.admin.hunter_reveal;
        self.hunters
            .iter()
            .filter(move |h| reveal || (h.active && h.gimmick.on_radar()))
    }

    /// This run's admin switches merged with the admin overlay's (ui::admin::flags).
    pub fn admin_flags(&self) -> Admin {
        let f = crate::shared::admin_flags::flags();
        Admin {
            god: self.admin.god || f.god_mode,
            no_target: self.admin.no_target || f.no_target,
            noclip: self.admin.noclip || f.noclip,
            run_speed: self.admin.run_speed * if f.run_speed > 0.0 { f.run_speed } else { 1.0 },
            hunter_reveal: self.admin.hunter_reveal,
        }
    }

    pub fn prof(&self) -> Profile {
        self.difficulty.profile()
    }

    pub fn level_seed(&self) -> String {
        match &self.inf {
            Some(i) => format!("{}:L{}", i.seed, i.level),
            None => format!("{}-D{}", self.run_seed, self.depth),
        }
    }

    /// The absolute cell of this run's streamed window's (0, 0) on the infinite route; (0, 0) on a finite floor.
    pub fn window_origin(&self) -> (i32, i32) {
        self.inf.as_ref().map_or((0, 0), |i| i.origin())
    }

    /// Builds the floor for the current depth and its hunters (and its render meshes).
    pub fn start_floor(&mut self, content: &Content, catalog: &Catalog, assets: &Assets) {
        self.start_floor_sim(content, catalog);
        self.sync_world(assets, content);
    }

    /// Builds the floor for the current depth and its hunters; the meshes wait for `sync_world`.
    pub fn start_floor_sim(&mut self, content: &Content, catalog: &Catalog) {
        crate::audio::world::reset(); // a new floor: new hunter views, no hum, nothing queued
        let seed = self.level_seed();
        let opt = Options {
            safe_zone_count: self.difficulty.safe_zone_limit(),
            ..Default::default()
        };
        self.fac = Facility::generate(&seed, &opt, catalog);
        self.fix = fixtures::place(&self.fac);
        self.adopt_floor(content);
        self.heater = Heater::Frozen;
        self.music_box_left = 45.0;
        self.relays_collected = 0;
        self.p.place(self.fac.start.center());
        self.npc_place_beside_runner();
        // the floor's route hunter, then the always-present specials beside her spawn
        let pick = crate::hunters::pick_for_floor(content, &seed, self.depth, self.difficulty);
        let def = content.hunters[pick].clone();
        let spawn = self.fac.hunter_spawn.center();
        let mut hunters = vec![Brain::new(
            def,
            spawn,
            &format!("{seed}:HUNTER"),
            self.difficulty,
        )];
        for (id, angle, d, tag) in [
            ("nyxie", 0.8f32, 2.0f32, "NYXIE"),
            ("solyth", 3.199963, 2.75, "SOLYTH"),
        ] {
            if let Some(def) = content.hunter(id) {
                let p = self.fac.nearest_open_or(
                    spawn.0 + angle.cos() * d,
                    spawn.1 + angle.sin() * d,
                    HUNTER_RADIUS,
                );
                let mut b = Brain::new(def.clone(), p, &format!("{seed}:{tag}"), self.difficulty);
                if let Gimmick::Solyth(s) = &mut b.gimmick {
                    s.heat_locked = self.meters.heat_locked;
                }
                hunters.push(b);
            }
        }
        self.hunters = hunters;
        self.reset_director();
        if self
            .prof()
            .activation_relays
            .min(self.fac.relays.len() as i32)
            <= 0
        {
            let p = self.p.pos;
            self.hunters[0].activate_at(p);
        }
        self.fade = 1.0;
        self.reveal();
    }

    /// Resets the per-floor arrays for whatever is in `fac`/`fix` now and marks the render
    /// meshes stale (rebuilt by `sync_world`; the simulation never reads them).
    fn adopt_floor(&mut self, content: &Content) {
        self.lights_on = vec![true; self.fac.room_lights.len()];
        self.world.lamps = crate::render::world_mesh::lamps(&self.fac, &self.lights_on, content);
        self.world_dirty = true;
        self.relays_taken = vec![false; self.fac.relays.len()];
        self.batteries_taken = vec![false; self.fac.batteries.len()];
        self.coins_taken = vec![false; self.fix.coins.len()];
        self.hiding_disabled = vec![false; self.fac.hiding.len()];
        self.locker_cooldown = vec![0.0; self.fac.hiding.len()];
        self.pickups = self.floor_pickups();
        self.climate = climate_for(&self.fac);
        self.discovered = vec![vec![false; self.fac.cols as usize]; self.fac.rows as usize];
        self.hide_check = None;
        self.escape = None;
    }

    pub fn say(&mut self, text: &str) {
        self.message = text.to_string();
        self.message_t = 2.6;
    }

    pub fn announce(&mut self, text: &str) {
        self.toast = text.to_string();
        self.toast_t = 2.4;
    }

    /// The climate offset where the runner stands (0 in halls).
    pub fn local_climate(&self) -> f32 {
        self.fac
            .room_at(self.p.pos.0 as f64, self.p.pos.1 as f64)
            .and_then(|i| self.climate.get(i).copied())
            .unwrap_or(0.0)
    }

    pub fn in_safe_zone(&self) -> bool {
        self.fac
            .safe_zone
            .map(|s| pdist(self.p.pos, s.center()) <= SAFE_ZONE_RADIUS)
            .unwrap_or(false)
    }

    pub fn eye(&self) -> Vec3 {
        self.p.eye()
    }

    pub fn camera_basis(&self) -> (Vec3, Vec3, Vec3) {
        self.p.camera_basis()
    }

    pub fn forward2(&self) -> (f32, f32) {
        self.p.forward2()
    }

    pub fn selected_item(&self) -> Option<Item> {
        self.slots[self.selected].map(|s| s.item)
    }

    fn reveal(&mut self) {
        let (px, py) = self.p.pos;
        let r = 7;
        let (cx, cy) = (px.floor() as i32, py.floor() as i32);
        for y in (cy - r).max(0)..=(cy + r).min(self.fac.rows - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.fac.cols - 1) {
                if self.discovered[y as usize][x as usize] {
                    continue;
                }
                let c = (x as f32 + 0.5, y as f32 + 0.5);
                let d = pdist(c, self.p.pos);
                if d > r as f32 + 0.5 {
                    continue;
                }
                let probe = if self.fac.is_wall_cell(x, y) {
                    let d = d.max(0.01);
                    (c.0 + (px - c.0) / d * 0.75, c.1 + (py - c.1) / d * 0.75)
                } else {
                    c
                };
                if self.fac.is_floor(probe.0 as f64, probe.1 as f64)
                    && self.fac.has_line_of_sight(self.p.pos, probe)
                {
                    self.discovered[y as usize][x as usize] = true;
                }
            }
        }
    }

    /// Builds the render meshes for the current floor if it changed (needs the GPU assets).
    pub fn sync_world(&mut self, assets: &Assets, content: &Content) {
        if self.world_dirty {
            let frame = self.inf.as_ref().map(|i| ChunkFrame { level: i.level, center: i.center });
            self.world.rebuild(&self.fac, &self.lights_on, frame, assets, content);
            self.world_dirty = false;
        }
    }

    /// One frame of the run, then the render meshes for any floor it entered.
    pub fn update(
        &mut self,
        dt: f32,
        c: &Controls,
        content: &Content,
        catalog: &Catalog,
        assets: &Assets,
        sounds: &Sounds,
    ) {
        self.step(dt, c, content, catalog, sounds);
        self.sync_world(assets, content);
    }

    /// One frame of the simulation alone: no window, GPU or audio device needed
    /// (`Sounds::default()` is silent). Deterministic for a fixed `dt` and input.
    pub fn step(
        &mut self,
        dt: f32,
        c: &Controls,
        content: &Content,
        catalog: &Catalog,
        sounds: &Sounds,
    ) {
        self.time += dt;
        self.fade = (self.fade - dt * 1.5).max(0.0);
        self.toast_t = (self.toast_t - dt).max(0.0);
        self.message_t = (self.message_t - dt).max(0.0);
        if self.report.is_some() {
            return;
        }
        if self.transition.is_some() {
            self.advance_transition(dt, content, catalog);
            return;
        }
        let prof = self.prof();
        self.taser_cd = (self.taser_cd - dt).max(0.0);
        self.melee_cd = (self.melee_cd - dt).max(0.0);
        self.swing_t = (self.swing_t - dt).max(0.0);
        self.taser_fx = (self.taser_fx - dt).max(0.0);
        self.escape_cooldown = (self.escape_cooldown - dt).max(0.0);
        self.escape_grace = (self.escape_grace - dt).max(0.0);
        for t in self.locker_cooldown.iter_mut() {
            *t = (*t - dt).max(0.0);
        }
        // Glyra's box winds down every frame (the original's finite floor forgot to tick it)
        self.music_box_left = (self.music_box_left - dt).max(0.0);

        // world sound every frame, hidden or struggling too (main_3d.gd _update_local_audio, Glyra's hum)
        {
            use crate::hunters::gimmicks::{nyxie::Wake, Gimmick};
            let (eye, (fwd, right, up)) = (self.eye(), self.camera_basis());
            let listener = crate::audio::world::Listener {
                pos: self.p.pos,
                facing: self.forward2(),
                eye: eye.to_array(),
                right: right.to_array(),
                up: up.to_array(),
                forward: fwd.to_array(),
                hidden: self.p.hidden.is_some(),
                in_safe_zone: self.in_safe_zone(),
            };
            let hunters: Vec<crate::audio::world::HunterAudio> = self.hunters.iter().map(|h| crate::audio::world::HunterAudio {
                id: &h.def.id,
                pos: h.pos,
                active: h.active,
                chasing: h.state == crate::hunters::brain::State::Chase,
                idle: h.state == crate::hunters::brain::State::Idle,
                direct_visible: h.direct_visible,
                winded: h.winded,
                stunned: h.stun > 0.0,
                in_vent: matches!(&h.gimmick, Gimmick::Verrine(v) if v.active_in_vent),
                nyxie_empty_wake: matches!(&h.gimmick, Gimmick::Nyxie(n) if n.wake == Wake::Empty),
                glyra_humming: matches!(&h.gimmick, Gimmick::Glyra(g) if g.playing()),
            }).collect();
            crate::audio::world::update(dt, &listener, &hunters);
        }
        // NPC teammates follow the runner through the escape struggle too (game/npc.rs)
        self.update_npcs(dt);
        let level = self.runner_level();
        let actors: Vec<(f32, f32)> = std::iter::once(self.p.pos)
            .chain(self.hunters.iter().filter(|h| h.active).map(|h| h.pos))
            .chain(self.npcs.iter().filter(|n| n.level == level).map(|n| n.at))
            .collect();
        self.fac.update_doors(&actors, dt);
        if self.escape.is_some() {
            self.update_escape(dt, c, sounds);
            return;
        }

        self.p.look(c.look);
        if self.p.hidden.is_some() {
            self.update_hidden(dt, c, &prof, sounds);
        } else {
            self.hide_check = None;
            let heater_on = self.heater == Heater::Powered;
            let in_safe = self.in_safe_zone();
            let local = self.local_climate();
            let admin = self.admin_flags();
            self.p.noclip = admin.noclip;
            self.p.speed_scale = admin.run_speed;
            self.p
                .update_movement(dt, c, &prof, &self.fac, &mut self.meters, sounds);
            if admin.noclip {
                self.p.pos.0 = self.p.pos.0.clamp(0.5, self.fac.cols as f32 - 0.5);
                self.p.pos.1 = self.p.pos.1.clamp(0.5, self.fac.rows as f32 - 0.5);
            }
            self.meters.update(
                dt,
                &meters::Situation {
                    flashlight_on: self.p.flashlight_on,
                    noise: self.p.noise,
                    crouching: self.p.crouching,
                    hidden: false,
                    in_safe_zone: in_safe,
                    heater_on,
                    local,
                },
            );
            if self.p.lamp <= 0.0 && self.p.flashlight_on {
                self.p.flashlight_on = false;
                crate::audio::sound::light_switch(); // the flashlight runs dead (player_controller_3d.gd 2419)
                self.say("FLASHLIGHT DEAD");
            }
            if self.p.stun <= 0.0 {
                self.update_tools(c, sounds);
            }
            self.focus = self.find_focus();
            if c.interact && self.p.stun <= 0.0 {
                if let Some(f) = self.focus {
                    self.interact(f, content, catalog, sounds);
                }
            }
            self.update_holds(dt, c, sounds);
        }

        self.discover_timer -= dt;
        if self.discover_timer <= 0.0 {
            self.discover_timer = 0.2;
            self.reveal();
            self.note_sightings();
            if self.inf.is_some() {
                self.ensure_hunters(content);
            }
        }
        if self.inf.is_some() {
            self.stream_window(content, catalog);
        }
        self.update_hunters(dt, &prof, sounds);
    }

    /// _discover_visible_hunters (main_3d.gd 260): an active hunter within
    /// HUNTER_DISCOVERY_RANGE 7.5 cells in line of sight is met; Umbriel shows the face she wears.
    fn note_sightings(&mut self) {
        for h in self.hunters.iter().filter(|h| h.active) {
            if pdist(self.p.pos, h.pos) > 7.5 || !self.fac.has_line_of_sight(self.p.pos, h.pos) {
                continue;
            }
            let id = h.disguise.clone().unwrap_or_else(|| h.def.id.clone());
            if self.sighted.insert(id.clone()) {
                self.sightings.push(id);
            }
        }
    }

    /// Hunters met since the last call (for the account's dossier discoveries).
    pub fn take_sightings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sightings)
    }

    pub fn danger(&self) -> f32 {
        let mut best: f32 = 0.0;
        for h in self
            .hunters
            .iter()
            .filter(|h| h.active && !h.gimmick.hidden_from_view())
        {
            let d = pdist(h.pos, self.p.pos);
            let base = match h.state {
                crate::hunters::brain::State::Chase => 0.75,
                crate::hunters::brain::State::Investigate
                | crate::hunters::brain::State::Search => 0.4,
                _ => 0.1,
            };
            best = best.max((base + (1.0 - d / 16.0).clamp(0.0, 1.0) * 0.36).min(1.0));
        }
        best
    }

    pub fn teleport_beside(&mut self, cell: crate::world::Cell) {
        let (cx, cy) = cell.center();
        for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let p = (cx + dx, cy + dy);
            if self.fac.body_fits(p.0, p.1, PLAYER_RADIUS) {
                self.p.pos = p;
                self.p.yaw = (-(cx - p.0)).atan2(-(cy - p.1));
                self.p.pitch = -0.45;
                return;
            }
        }
    }
}

/// Hot and cold spots: each room's climate leans on its region (furnace-hot nether, warm
/// arcades and beaches, cold maintenance bays and server rooms, cool gardens) with a seeded
/// spread. The original's per-room climate profiles were not ported to Godot; these values
/// are estimated so the cold that wakes Wynelle and the heat that latches Solyth exist on
/// the floor.
fn climate_for(f: &Facility) -> Vec<f32> {
    let mut rng = crate::world::rng::Rng::new(&format!("{}:CLIMATE", f.seed_text));
    f.rooms
        .iter()
        .map(|r| {
            let base = match (r.region.as_str(), r.rtype.as_str()) {
                (_, "lobby") => 0.0,
                ("nether", _) => 28.0,
                (_, "server") | (_, "cargoDepot") => -30.0,
                ("maintenance", _) => -22.0,
                ("garden", _) => -12.0,
                ("beach", _) | ("arcade", _) | ("circus", _) => 8.0,
                (_, "kitchen") | (_, "luxuryKitchen") | (_, "gothicKitchen") => 12.0,
                _ => 0.0,
            };
            if r.lobby() {
                0.0
            } else {
                base + (rng.next_f32() - 0.5) * 10.0
            }
        })
        .collect()
}
