//! Hunters on every resident floor (docs/STAIRS_CONTINUOUS_DESIGN.md D5, phase 8). `Game::hunters` is the list of the active
//! level; this module keeps one more list per resident neighbour level, in the same window frame, and moves a hunter
//! between lists when its own derived level changes (it walked a flight or a ladder). A level that leaves the stack hands
//! its hunters back to the stores of `game/endless.rs` at absolute cells (`InfState::floor_hunters` for ids-less hunters,
//! `InfState::stored` for the level-0 hunters residency owns), and a level that joins pulls them out again.
//!
//! Everything here is behaviour. The numbers are data: the resident hunter cap, the capture height and the adjacent
//! hearing and sight rules are in `data/hunters/flights.json` (`hunters/flight_edges.rs`), the hunter's eye height in
//! `data/resident_hunters.json`.

use super::endless::shift_brain;
use super::hunting::ability_held;
use super::level_stack::floor_y_of;
use super::{pdist, Game, Heater};
use crate::config::*;
use crate::content::hunters::HunterDef;
use crate::content::Content;
use crate::hunters::brain::{choose_target, Brain, CrossSense, Target, TargetRules};
use crate::hunters::flight_edges::{self, sight_clear_3d, Floors, StackNav};
use crate::hunters::gimmicks::{Ctx, Event, Gimmick};
use crate::hunters::water::WaterField;
use crate::hunters::{director, residency};
use crate::net::hunter_events;
use crate::world::fixtures::Fixtures;
use crate::world::infinite::CHUNK;
use crate::world::rng::Rng;
use crate::world::Facility;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------------------------------------------------
// Data (data/resident_hunters.json)
// ---------------------------------------------------------------------------------------------------------------------

/// The height above a hunter's feet its sight line to a runner on another storey starts from (m).
pub fn hunter_eye_height() -> f32 {
    static EYE: OnceLock<f32> = OnceLock::new();
    *EYE.get_or_init(|| {
        parse_eye(include_str!("../data/resident_hunters.json")).expect("data/resident_hunters.json has hunter_eye_height")
    })
}

/// `hunter_eye_height` from the text of a resident hunters file: finite and between 0 and 5 m.
pub fn parse_eye(text: &str) -> Option<f32> {
    let v: Value = serde_json::from_str(text).ok()?;
    v["hunter_eye_height"].as_f64().map(|x| x as f32).filter(|x| x.is_finite() && *x > 0.0 && *x <= 5.0)
}

// ---------------------------------------------------------------------------------------------------------------------
// The lists
// ---------------------------------------------------------------------------------------------------------------------

/// One hunter of a resident list. `id` is the level-0 residency id (empty on every other level and for a hunter the
/// route did not admit), so the id travels with the hunter between the list and the stores.
pub struct Slot {
    pub id: String,
    pub brain: Brain,
}

/// What decides whether the shared navigation graph must be built again.
#[derive(Clone, Debug, PartialEq)]
struct NavKey {
    active: i32,
    levels: Vec<i32>,
    center: (i32, i32),
    builds: u32,
}

/// The hunters of every resident level beside the active one, by level, in the window frame.
#[derive(Default)]
pub struct ResidentHunters {
    by_level: BTreeMap<i32, Vec<Slot>>,
    nav_key: Option<NavKey>,
    dry: Option<Arc<WaterField>>,
}

impl ResidentHunters {
    /// The dry water field every resident-list hunter reads: the hunters' water belongs to the level the runner's sewer
    /// chunk was read for, so a pipe flood or sewer water never reaches a hunter on another storey.
    pub fn dry_field(&mut self) -> Arc<WaterField> {
        self.dry.get_or_insert_with(|| Arc::new(WaterField::from_depths(&[], &[]))).clone()
    }

    /// The levels that have a list, ascending.
    pub fn levels(&self) -> Vec<i32> {
        self.by_level.keys().copied().collect()
    }

    /// Whether `level` has a list (it may be empty).
    pub fn has(&self, level: i32) -> bool {
        self.by_level.contains_key(&level)
    }

    /// The hunters of `level` (none when it has no list).
    pub fn list(&self, level: i32) -> &[Slot] {
        self.by_level.get(&level).map_or(&[], |v| v.as_slice())
    }

    /// The list of `level`, made empty when it has none.
    pub fn list_mut(&mut self, level: i32) -> &mut Vec<Slot> {
        self.by_level.entry(level).or_default()
    }

    /// Removes the list of `level` and returns its hunters.
    pub fn take(&mut self, level: i32) -> Vec<Slot> {
        self.by_level.remove(&level).unwrap_or_default()
    }

    /// Sets the list of `level`, replacing any.
    pub fn put(&mut self, level: i32, slots: Vec<Slot>) {
        self.by_level.insert(level, slots);
    }

    /// Every resident hunter, levels ascending and list order within a level (render, audio, bots).
    pub fn iter_all(&self) -> impl Iterator<Item = &Brain> {
        self.by_level.values().flat_map(|v| v.iter().map(|s| &s.brain))
    }

    /// `iter_all` with the level of the list each hunter is in.
    pub fn iter_leveled(&self) -> impl Iterator<Item = (i32, &Brain)> {
        self.by_level.iter().flat_map(|(l, v)| v.iter().map(move |s| (*l, &s.brain)))
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut Brain> {
        self.by_level.values_mut().flat_map(|v| v.iter_mut().map(|s| &mut s.brain))
    }

    /// How many hunters are in the resident lists.
    pub fn total(&self) -> usize {
        self.by_level.values().map(Vec::len).sum()
    }
}

fn split(slots: Vec<Slot>) -> (Vec<String>, Vec<Brain>) {
    slots.into_iter().map(|s| (s.id, s.brain)).unzip()
}

fn join(ids: Vec<String>, brains: Vec<Brain>) -> Vec<Slot> {
    ids.into_iter().zip(brains).map(|(id, brain)| Slot { id, brain }).collect()
}

/// The indices of the `cap` smallest distances (ties by index), ascending. These are the hunters a resident floor
/// simulates this frame.
pub fn nearest_within_cap(dist: &[f32], cap: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..dist.len()).collect();
    order.sort_by(|a, b| dist[*a].total_cmp(&dist[*b]).then(a.cmp(b)));
    order.truncate(cap);
    order.sort_unstable();
    order
}

/// A bot as a hunter target: the level it stands on and what the brain reads of it.
pub(super) struct BotTarget {
    pub level: i32,
    pub target: Target,
}

/// A hunter in the lists: the active list or a resident level's list and the index in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spot {
    Active(usize),
    Resident(i32, usize),
}

thread_local! {
    static DEFS: std::cell::RefCell<Option<Vec<HunterDef>>> = const { std::cell::RefCell::new(None) };
}

/// The definition of hunter `id`, for a guest that is shown a hunter its own lineup never built.
fn hunter_def(id: &str) -> Option<HunterDef> {
    DEFS.with(|d| {
        let mut d = d.borrow_mut();
        d.get_or_insert_with(|| Content::load().hunters).iter().find(|h| h.id == id).cloned()
    })
}

/// A spot on `fac` 12 to 34 m from `runner` and out of its sight, for a hunter that has to come back in (the rule of the
/// active level's `spawn_spot`).
fn spawn_spot_in(fac: &Facility, runner: (f32, f32), rng: &mut Rng) -> Option<(f32, f32)> {
    let cands: Vec<_> = fac
        .floor_cells()
        .into_iter()
        .filter(|c| {
            let p = c.center();
            let d = pdist(p, runner);
            d >= 12.0 && d <= 34.0 && fac.hunter_open(*c) && !fac.has_line_of_sight(p, runner)
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    Some(cands[rng.next_index(cands.len())].center())
}

// ---------------------------------------------------------------------------------------------------------------------
// Lookups, targets and the shared navigation graph
// ---------------------------------------------------------------------------------------------------------------------

impl Game {
    /// The facility and fixtures of `level`: the active level's, else the resident neighbour's.
    pub(super) fn level_parts(&self, level: i32) -> Option<(&Facility, &Fixtures)> {
        if level == self.runner_level() {
            return Some((&self.fac, &self.fix));
        }
        self.inf.as_ref()?.stack.get(level).map(|r| (&r.fac, &r.fix))
    }

    /// Every bot as a target, with the level it stands on (a hunter picks among those on its own level).
    pub(super) fn bot_targets(&self) -> Vec<BotTarget> {
        self.npcs
            .iter()
            .map(|n| BotTarget {
                level: n.level,
                target: Target { pos: n.at, facing: n.heading, y: n.y, level: n.level, ..Default::default() },
            })
            .collect()
    }

    /// How hunter `h` senses the runner target `t` when they are on different levels: the 3D sight line through an open
    /// well (never through a slab) and the adjacent hearing scale. None on one level, where the facility answers.
    pub(super) fn cross_for(&self, h: &Brain, t: &Target) -> Option<CrossSense> {
        if h.level == t.level {
            return None;
        }
        let rules = flight_edges::rules();
        let apart = (t.level - h.level).abs();
        let hear_scale = if apart == 1 { rules.hear_adjacent_scale } else { 0.0 };
        let sees = rules.see_adjacent
            && apart == 1
            && self.inf.as_ref().is_some_and(|inf| {
                let floors = Floors { active: &self.fac, stack: &inf.stack };
                let runner_eye = self.p.eye_y() - self.p.y();
                sight_clear_3d(
                    &floors,
                    [h.pos.0, h.pos.1, h.y + hunter_eye_height()],
                    [t.pos.0, t.pos.1, t.y + runner_eye],
                )
            });
        Some(CrossSense { sees, hear_scale })
    }

    /// The target hunter `h` of the list on `list_level` acts on this frame, and whether it is a bot. The runner is the
    /// default; a bot on the hunter's own level replaces it where the rules allow (`choose_target`). A runner on another
    /// level than the hunter carries `cross`, so a hunter on a flight sees the runner through the well.
    pub(super) fn target_for(&self, h: &Brain, list_level: i32, runner: &Target, bots: &[BotTarget]) -> (Target, bool) {
        let own: Vec<&BotTarget> = bots.iter().filter(|b| b.level == list_level).collect();
        let pos: Vec<(f32, f32)> = own.iter().map(|b| b.target.pos).collect();
        let fac = self.level_parts(list_level).map_or(&self.fac, |p| p.0);
        let chosen = choose_target(TargetRules::get(), self.difficulty, h.pos, runner.pos, &pos, |a, b| {
            fac.has_line_of_sight(a, b)
        });
        match chosen {
            Some(k) => (own[k].target.clone(), true),
            None => {
                let mut t = runner.clone();
                t.cross = self.cross_for(h, &t);
                (t, false)
            }
        }
    }

    /// Builds the navigation graph over the active and resident levels when they change (the level set, the window
    /// centre or a window built), and gives it to every brain of every list. A guest runs no hunters and builds nothing.
    pub(super) fn sync_stack_nav(&mut self) {
        if self.remote_hunters {
            return;
        }
        let Some(inf) = self.inf.as_ref() else { return };
        let mut levels = inf.stack.levels();
        levels.push(inf.level);
        levels.sort_unstable();
        let key = NavKey { active: inf.level, levels, center: inf.center, builds: inf.builds };
        if self.resident_hunters.nav_key.as_ref() != Some(&key) {
            self.stack_nav = Arc::new(StackNav::build(&self.fac, &inf.stack, flight_edges::rules()));
            self.resident_hunters.nav_key = Some(key);
        }
        let nav = self.stack_nav.clone();
        for b in self.hunters.iter_mut() {
            b.set_nav(nav.clone());
        }
        for b in self.resident_hunters.iter_all_mut() {
            b.set_nav(nav.clone());
        }
    }

    /// Puts a brain just built for the active level on that level (its `level`, `y` and the level of what it last
    /// knew), as every lineup and top-up must do before it joins `hunters`.
    pub(super) fn settle_new_brain(&self, b: &mut Brain) {
        let level = self.runner_level();
        b.level = level;
        b.y = floor_y_of(level);
        b.last_known_level = level;
        b.set_nav(self.stack_nav.clone());
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Joining and leaving the stack
// ---------------------------------------------------------------------------------------------------------------------

impl Game {
    /// Run at the end of `keep_stack`: a level that joined the stack gets its list (its hunters pulled out of the stores
    /// into the window frame, or a lineup when the level was never visited), and a level that left hands its hunters back
    /// to the stores at absolute cells. A guest keeps the lists the host's frames fill and only makes the missing ones.
    pub(super) fn sync_resident_hunters(&mut self, content: &Content) {
        let Some(inf) = self.inf.as_ref() else { return };
        let active = inf.level;
        let mut want: Vec<i32> = inf.stack.levels().into_iter().filter(|l| *l != active).collect();
        want.sort_unstable();
        let have = self.resident_hunters.levels();
        if self.remote_hunters {
            // a guest keeps no stores: a list whose level left the stack is dropped (its hunters are the host's to keep and
            // the next frame makes them again), and a level that joined gets an empty list for the frames to fill
            for k in have.iter().copied().filter(|k| !want.contains(k)) {
                self.resident_hunters.take(k);
            }
            for l in want {
                if !have.contains(&l) {
                    self.resident_hunters.put(l, Vec::new());
                }
            }
            return;
        }
        for k in have.iter().copied().filter(|k| !want.contains(k)) {
            let slots = self.resident_hunters.take(k);
            self.store_slots(k, slots);
        }
        for l in want.into_iter().filter(|l| !have.contains(l)) {
            let slots = self.pull_slots(l, content);
            self.resident_hunters.put(l, slots);
        }
    }

    /// Hands `slots` of `level` to the stores at absolute cells: a level-0 hunter with a residency id to `stored`, every
    /// other to `floor_hunters` under the level. The level always gets an entry, so it counts as visited.
    pub(super) fn store_slots(&mut self, level: i32, slots: Vec<Slot>) {
        // a guest keeps no hunters of its own: nothing it shows is ever stored
        if self.remote_hunters {
            return;
        }
        let Some(inf) = self.inf.as_mut() else { return };
        let o = inf.origin();
        let o = (o.0 as f32, o.1 as f32);
        let mut kept = Vec::new();
        for Slot { id, mut brain } in slots {
            shift_brain(&mut brain, o);
            if level == 0 && !id.is_empty() {
                inf.stored_mut().insert(id, brain);
            } else {
                kept.push(brain);
            }
        }
        inf.floor_hunters_mut().entry(level).or_default().extend(kept);
    }

    /// The hunters of `level` as it joins the stack, in the window frame. Level 0: the active hunters of the stored set
    /// that stand in the window, plus any kept id-less ones. Another level: the hunters it kept, put back inside the
    /// window; a level never visited gets its lineup.
    fn pull_slots(&mut self, level: i32, content: &Content) -> Vec<Slot> {
        let Some(inf) = self.inf.as_mut() else { return Vec::new() };
        let o = inf.origin();
        let back = (-(o.0 as f32), -(o.1 as f32));
        let center = inf.center;
        let mut slots = Vec::new();
        let mut visited = level == 0;
        if level == 0 {
            let mut ids: Vec<String> = inf
                .stored_mut()
                .iter()
                .filter(|(_, b)| b.active && residency::in_window(residency::chunk_of(b.pos), center))
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            for id in ids {
                if let Some(mut brain) = inf.stored_mut().remove(&id) {
                    shift_brain(&mut brain, back);
                    slots.push(Slot { id, brain });
                }
            }
        }
        if let Some(kept) = inf.floor_hunters_mut().remove(&level) {
            visited = true;
            for mut brain in kept {
                shift_brain(&mut brain, back);
                slots.push(Slot { id: String::new(), brain });
            }
        }
        if !visited {
            return self.build_lineup(level, content);
        }
        if level != 0 {
            self.fit_into_window(level, &mut slots);
        }
        let nav = self.stack_nav.clone();
        for s in slots.iter_mut() {
            s.brain.set_nav(nav.clone());
        }
        slots
    }

    /// A hunter that stands outside the window of `level`, or in a wall of it, comes back in out of sight (as on a window
    /// move). One that is walking an edge is left alone.
    fn fit_into_window(&self, level: i32, slots: &mut [Slot]) {
        let (Some(inf), Some((fac, _))) = (self.inf.as_ref(), self.level_parts(level)) else { return };
        let size = (CHUNK * 3) as f32;
        let mut rng = Rng::new(&format!("{}:L{}:RESIDENT:{}", inf.seed, level, self.time as i32));
        for s in slots.iter_mut() {
            let p = s.brain.pos;
            let out = p.0 < 0.0 || p.1 < 0.0 || p.0 >= size || p.1 >= size || !fac.body_fits(p.0, p.1, HUNTER_RADIUS);
            if out && !s.brain.on_edge() {
                if let Some(spot) = spawn_spot_in(fac, self.p.pos, &mut rng) {
                    s.brain.pos = spot;
                    s.brain.clear_path();
                    s.brain.level = level;
                    s.brain.y = floor_y_of(level);
                }
            }
        }
    }

    /// The lineup of a level never visited (the rule of `reset_hunters` and `ensure_hunters`): Nyxie and Solyth, then the
    /// route hunters for the relays recovered so far, each at a spot out of the runner's sight. Level 0 has no lineup
    /// here: its residency admits hunters from the layout when it is the active level.
    fn build_lineup(&self, level: i32, content: &Content) -> Vec<Slot> {
        let (Some(inf), Some((fac, _))) = (self.inf.as_ref(), self.level_parts(level)) else { return Vec::new() };
        if level == 0 {
            return Vec::new();
        }
        let seed = format!("{}:L{}", inf.seed, level);
        let settle = |b: &mut Brain| {
            b.level = level;
            b.y = floor_y_of(level);
            b.last_known_level = level;
            b.set_nav(self.stack_nav.clone());
        };
        let mut out = Vec::new();
        let mut rng = Rng::new(&format!("{seed}:SPECIALS"));
        for (id, tag) in [("nyxie", "NYXIE"), ("solyth", "SOLYTH")] {
            if let Some(def) = content.hunter(id) {
                let p = spawn_spot_in(fac, self.p.pos, &mut rng).unwrap_or(fac.start.center());
                let mut b = Brain::new(def.clone(), p, &format!("{seed}:{tag}"), self.difficulty);
                if let Gimmick::Solyth(s) = &mut b.gimmick {
                    s.heat_locked = self.meters.heat_locked;
                }
                settle(&mut b);
                out.push(Slot { id: String::new(), brain: b });
            }
        }
        for n in 0..super::endless::population(self.difficulty, self.relays_collected) {
            let mut rng = Rng::new(&format!("{seed}:SPAWN:{n}"));
            let Some(p) = spawn_spot_in(fac, self.p.pos, &mut rng) else { break };
            let pick = crate::hunters::pick_for_floor(content, &format!("{seed}:H{n}"), 1 - level, self.difficulty);
            let mut b = Brain::new(content.hunters[pick].clone(), p, &format!("{seed}:HUNTER:{n}"), self.difficulty);
            b.activate_at(self.p.pos);
            settle(&mut b);
            out.push(Slot { id: String::new(), brain: b });
        }
        out
    }

    /// Hands every resident list back to the stores at absolute cells, before a load replaces the window
    /// (`change_storey`, which still swaps levels for a respawn or a checkpoint). A guest keeps nothing.
    pub(super) fn drain_resident_hunters(&mut self) {
        for l in self.resident_hunters.levels() {
            let slots = self.resident_hunters.take(l);
            if !self.remote_hunters {
                self.store_slots(l, slots);
            }
        }
    }

    /// The window moved by `d` (`recentre_window`): every resident hunter shifts with it. A guest's hunters stay where
    /// the host's frames put them.
    pub(super) fn shift_resident_hunters(&mut self, d: (f32, f32)) {
        if self.remote_hunters {
            return;
        }
        for b in self.resident_hunters.iter_all_mut() {
            shift_brain(b, d);
        }
    }

    /// `relabel` from level `from` to `to` (already `inf.level`): the active list becomes the list of `from` and the
    /// list of `to` becomes the active one. No brain is shifted or reset, and the residency ids travel with the slots.
    /// When `to` was somehow not synced the old store and restore pair runs instead (they read `inf.level`, so it is
    /// put back to `from` for the store).
    pub(super) fn relabel_hunters(&mut self, from: i32, to: i32, content: &Content) {
        let n = self.hunters.len();
        let ids: Vec<String> = match self.inf.as_mut() {
            Some(inf) if from == 0 => {
                let mut l = std::mem::take(inf.loaded_mut());
                l.resize(n, String::new());
                l
            }
            Some(inf) => {
                inf.loaded_mut().clear();
                vec![String::new(); n]
            }
            None => vec![String::new(); n],
        };
        let brains = std::mem::take(&mut self.hunters);
        if !self.resident_hunters.has(to) && !self.remote_hunters {
            self.hunters = brains;
            if let Some(inf) = self.inf.as_mut() {
                if from == 0 {
                    *inf.loaded_mut() = ids;
                }
                inf.level = from;
            }
            self.store_level0_hunters();
            self.store_floor_hunters();
            if let Some(inf) = self.inf.as_mut() {
                inf.level = to;
            }
            self.restore_level_hunters(content);
            if to == 0 {
                self.adopt_kept_level0();
            }
            return;
        }
        self.resident_hunters.put(from, join(ids, brains));
        let (ids, brains) = split(self.resident_hunters.take(to));
        self.hunters = brains;
        if let Some(inf) = self.inf.as_mut() {
            *inf.loaded_mut() = if to == 0 { ids } else { Vec::new() };
        }
        self.reset_director();
        // a level with fewer hunters than its population is topped up; level 0 runs its residency
        self.ensure_hunters(content);
    }

    /// The id-less level-0 hunters the stores kept while level 0 was not resident come back into the active list.
    fn adopt_kept_level0(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        let o = inf.origin();
        let back = (-(o.0 as f32), -(o.1 as f32));
        let Some(kept) = inf.floor_hunters_mut().remove(&0) else { return };
        for mut brain in kept {
            shift_brain(&mut brain, back);
            self.push_active(Slot { id: String::new(), brain });
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Moving hunters between lists
// ---------------------------------------------------------------------------------------------------------------------

impl Game {
    /// Whether active hunter `i` must stay in the active list: the one struggling with the runner, or checking a locker.
    fn pinned_active(&self, i: usize) -> bool {
        self.escape.as_ref().is_some_and(|e| e.hunter == i) || self.hide_check.is_some_and(|(_, h, _)| h == i)
    }

    /// Adds a hunter to the active list (and its residency id on level 0). The director's ids are indices, so it is
    /// made again.
    pub(super) fn push_active(&mut self, slot: Slot) {
        let Slot { id, brain } = slot;
        let n = self.hunters.len();
        if let Some(inf) = self.inf.as_mut().filter(|i| i.level == 0) {
            let loaded = inf.loaded_mut();
            loaded.resize(n, String::new());
            loaded.push(id);
        }
        self.hunters.push(brain);
        self.reset_director();
    }

    /// Takes active hunter `i` out of the list, with its residency id. The struggle and the locker check keep pointing at
    /// the same hunters.
    pub(super) fn remove_active(&mut self, i: usize) -> Slot {
        let brain = self.hunters.remove(i);
        let after = self.hunters.len();
        let id = match self.inf.as_mut().filter(|inf| inf.level == 0) {
            Some(inf) => {
                let loaded = inf.loaded_mut();
                loaded.resize(after + 1, String::new());
                loaded.remove(i)
            }
            None => String::new(),
        };
        if let Some(e) = self.escape.as_mut() {
            if e.hunter > i {
                e.hunter -= 1;
            }
        }
        if let Some((spot, h, t)) = self.hide_check {
            if h > i {
                self.hide_check = Some((spot, h - 1, t));
            }
        }
        self.reset_director();
        Slot { id, brain }
    }

    /// Puts a hunter on the list of level `to`: the active one, a resident one, or (host only) the stores at once when
    /// that level is not resident. A guest parks it on a list of that level for the host's next frame to move.
    fn place_slot(&mut self, to: i32, slot: Slot) {
        if to == self.runner_level() {
            self.push_active(slot);
        } else if self.remote_hunters || self.resident_hunters.has(to) {
            self.resident_hunters.list_mut(to).push(slot);
        } else {
            self.store_slots(to, vec![slot]);
        }
    }

    /// After every brain update: a hunter whose derived level differs from its list's level moves to that list. Levels
    /// ascending, index ascending, so the result is the same on every run.
    pub(super) fn move_crossers(&mut self) {
        let active = self.runner_level();
        let mut levels = self.resident_hunters.levels();
        if !levels.contains(&active) {
            levels.push(active);
        }
        levels.sort_unstable();
        let mut moves: Vec<(i32, usize, i32)> = Vec::new();
        for &level in &levels {
            if level == active {
                for (i, h) in self.hunters.iter().enumerate() {
                    if h.level != active && !self.pinned_active(i) {
                        moves.push((level, i, h.level));
                    }
                }
            } else {
                for (i, s) in self.resident_hunters.list(level).iter().enumerate() {
                    if s.brain.level != level {
                        moves.push((level, i, s.brain.level));
                    }
                }
            }
        }
        if moves.is_empty() {
            return;
        }
        // out highest index first, so the lower ones keep theirs; in again in the sorted order
        let mut taken: BTreeMap<(i32, usize), Slot> = BTreeMap::new();
        for &(from, i, _) in moves.iter().rev() {
            let slot = if from == active { self.remove_active(i) } else { self.resident_hunters.list_mut(from).remove(i) };
            taken.insert((from, i), slot);
        }
        for (from, i, to) in moves {
            if let Some(slot) = taken.remove(&(from, i)) {
                self.place_slot(to, slot);
            }
        }
    }

    /// A resident hunter that captured the runner joins the active list (the struggle and the run report read
    /// `hunters[i]`). Returns its index there.
    pub(super) fn adopt_resident(&mut self, level: i32, i: usize) -> usize {
        let slot = self.resident_hunters.list_mut(level).remove(i);
        self.push_active(slot);
        self.hunters.len() - 1
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The frame (host) and the guest
// ---------------------------------------------------------------------------------------------------------------------

impl Game {
    /// The brain at `spot`.
    pub(super) fn hunter_at_mut(&mut self, spot: Spot) -> Option<&mut Brain> {
        match spot {
            Spot::Active(i) => self.hunters.get_mut(i),
            Spot::Resident(l, i) => self.resident_hunters.by_level.get_mut(&l)?.get_mut(i).map(|s| &mut s.brain),
        }
    }

    /// Where the hunter `id` of a host frame entry is: the active list at `index` (a hint), else the list of the entry's
    /// level, else any active hunter of that id, else any resident one.
    pub(super) fn locate_hunter(&self, id: &str, index: usize, level: Option<i32>) -> Option<Spot> {
        if self.hunters.get(index).is_some_and(|h| h.def.id == id) {
            return Some(Spot::Active(index));
        }
        let active = self.runner_level();
        if let Some(l) = level {
            if l == active {
                if let Some(i) = self.hunters.iter().position(|h| h.def.id == id) {
                    return Some(Spot::Active(i));
                }
            } else if let Some(j) = self.resident_hunters.list(l).iter().position(|s| s.brain.def.id == id) {
                return Some(Spot::Resident(l, j));
            }
        }
        if let Some(i) = self.hunters.iter().position(|h| h.def.id == id) {
            return Some(Spot::Active(i));
        }
        for l in self.resident_hunters.levels() {
            if let Some(j) = self.resident_hunters.list(l).iter().position(|s| s.brain.def.id == id) {
                return Some(Spot::Resident(l, j));
            }
        }
        None
    }

    /// Guest: a hunter the host sends that this game has no brain for (it is on a floor the guest's own lineup never
    /// built). Made from the hunter definition when the entry names a level the guest holds; the frame then sets it.
    pub(super) fn create_guest_hunter(&mut self, id: &str, level: Option<i32>) -> Option<Spot> {
        if !self.remote_hunters {
            return None;
        }
        let level = level?;
        let active = self.runner_level();
        let held = level == active || self.inf.as_ref().is_some_and(|i| i.stack.has(level));
        if !held {
            return None;
        }
        let def = hunter_def(id)?;
        let mut b = Brain::new(def, (0.5, 0.5), "GUEST", self.difficulty);
        b.display_only = true;
        b.shadow = true;
        b.active = true;
        b.level = level;
        b.y = floor_y_of(level);
        if level == active {
            self.hunters.push(b);
            Some(Spot::Active(self.hunters.len() - 1))
        } else {
            let list = self.resident_hunters.list_mut(level);
            list.push(Slot { id: String::new(), brain: b });
            Some(Spot::Resident(level, list.len() - 1))
        }
    }

    /// The frame entries of the resident hunters, indexed after the active list (`active.len() + k`), so an old guest's
    /// id check skips them.
    pub(super) fn resident_frame_entries(&self) -> Vec<Value> {
        let base = self.hunters.len();
        self.resident_hunters
            .iter_all()
            .enumerate()
            .map(|(k, h)| frame_entry(base + k, h))
            .collect()
    }
}

/// One hunter of a host frame (see `Game::hunter_frame`): the old keys plus `level` and `presentation_y` (absolute feet).
pub(super) fn frame_entry(index: usize, h: &Brain) -> Value {
    json!({
        "index": index,
        "id": h.def.id,
        "position": [h.pos.0, h.pos.1],
        "facing": [h.facing.0, h.facing.1],
        "state": hunter_events::state_name(h.state),
        "active": h.active,
        "stun": h.stun,
        "slow_factor": h.slow_factor,
        "disguise": h.disguise,
        "level": h.level,
        "presentation_y": h.y,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Simulating the resident floors
// ---------------------------------------------------------------------------------------------------------------------

impl Game {
    /// One frame of every resident floor's hunters (D5): on each resident level, ascending, the `resident_hunter_cap`
    /// hunters nearest the runner tick with that level's facility, then the level's hunters are pushed apart. Of what
    /// they do to the runner only Stun, Meter and Eject count, and only from within the capture height; a
    /// FacilityAlert alerts its own list. There is no director tick for a resident list. Returns the first hunter that
    /// caught the runner, as `(level, index)`.
    pub(super) fn update_resident_hunters(&mut self, dt: f32, prof: &Profile) -> Option<(i32, usize)> {
        if self.remote_hunters {
            return None;
        }
        let levels = self.resident_hunters.levels();
        if levels.is_empty() {
            return None;
        }
        let runner = self.target();
        let bots = self.bot_targets();
        let rules = flight_edges::rules();
        let cap = rules.resident_hunter_cap as usize;
        let mut base = self.hunters.len();
        let mut events: Vec<(i32, usize, Event)> = Vec::new();
        let mut emits: Vec<(&'static str, &'static str, String, usize)> = Vec::new();
        let mut caught: Option<(i32, usize)> = None;
        for level in levels {
            let slots = std::mem::take(self.resident_hunters.list_mut(level));
            let count = slots.len();
            let (ids, mut brains) = split(slots);
            if let Some((fac, fix)) = self.level_parts(level) {
                let dists: Vec<f32> = brains.iter().map(|b| pdist(b.pos, runner.pos)).collect();
                for i in nearest_within_cap(&dists, cap) {
                    let (t, is_bot) = self.target_for(&brains[i], level, &runner, &bots);
                    brains[i].target_is_bot = is_bot;
                    if let Gimmick::Solyth(s) = &mut brains[i].gimmick {
                        s.heat_locked = self.meters.heat_locked;
                    }
                    let ctx = Ctx {
                        fac,
                        relays_taken: &[],
                        batteries_taken: &[],
                        hiding_disabled: &[],
                        vents: &fix.vents,
                        heater_on: self.heater_running(),
                        heater_thawed: self.heater != Heater::Frozen,
                        diff: self.difficulty,
                        music_box_left: self.music_box_left,
                        exposure: self.meters.exposure,
                    };
                    let held_before = ability_held(&brains[i]);
                    let (c, ev) = brains[i].update(dt, &ctx, &t, 1 - level, self.relays_collected, prof);
                    if let (None, Some((kind, ability))) = (held_before, ability_held(&brains[i])) {
                        emits.push((kind, ability, brains[i].def.id.clone(), base + i));
                    }
                    if c && !is_bot && caught.is_none() {
                        caught = Some((level, i));
                    }
                    // the runner is reached only from the same height; a FacilityAlert stays on this list
                    let reach = (brains[i].y - runner.y).abs() <= rules.capture_height;
                    for e in ev {
                        match e {
                            Event::FacilityAlert { .. } => events.push((level, i, e)),
                            Event::Stun(_) | Event::Meter { .. } | Event::Eject { .. } if reach => events.push((level, i, e)),
                            _ => {}
                        }
                    }
                }
                if self.inf.is_some() {
                    director::separate(dt, &mut brains, fac);
                }
            }
            *self.resident_hunters.list_mut(level) = join(ids, brains);
            base += count;
        }
        for (kind, ability, id, index) in emits {
            self.hunter_out.emit(
                hunter_events::rules(),
                self.difficulty,
                kind,
                vec![("index", json!(index)), ("hunter_id", json!(id)), ("ability", json!(ability))],
                std::time::Instant::now(),
            );
        }
        for (level, from, e) in events {
            match e {
                Event::FacilityAlert { at, except_self } => {
                    for (j, s) in self.resident_hunters.list_mut(level).iter_mut().enumerate() {
                        if s.brain.active && !(except_self && j == from) {
                            s.brain.state = crate::hunters::brain::State::Investigate;
                            s.brain.last_known = Some(at);
                        }
                    }
                }
                other => self.apply_event(usize::MAX, other),
            }
        }
        caught
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_keeps_the_nearest_hunters_in_list_order() {
        assert_eq!(nearest_within_cap(&[5.0, 1.0, 3.0, 2.0], 2), vec![1, 3]);
        assert_eq!(nearest_within_cap(&[5.0, 1.0, 3.0, 2.0], 3), vec![1, 2, 3]);
        assert_eq!(nearest_within_cap(&[5.0, 1.0], 8), vec![0, 1], "a cap above the count keeps everyone");
        assert!(nearest_within_cap(&[5.0, 1.0], 0).is_empty(), "a zero cap simulates nobody");
        assert_eq!(nearest_within_cap(&[2.0, 2.0, 2.0], 2), vec![0, 1], "ties go to the lower index");
    }

    #[test]
    fn the_eye_height_is_read_from_the_data_and_refuses_nonsense() {
        assert!(hunter_eye_height() > 0.0);
        assert_eq!(parse_eye(r#"{"hunter_eye_height": 1.5}"#), Some(1.5));
        assert_eq!(parse_eye(r#"{"hunter_eye_height": -1.0}"#), None);
        assert_eq!(parse_eye(r#"{"hunter_eye_height": 90.0}"#), None);
        assert_eq!(parse_eye(r#"{}"#), None);
        assert_eq!(parse_eye("not json"), None);
    }

    #[test]
    fn every_resident_hunter_reads_the_same_dry_field() {
        let mut r = ResidentHunters::default();
        let a = r.dry_field();
        assert!(Arc::ptr_eq(&a, &r.dry_field()), "one dry field, shared");
        assert!(a.is_dry(), "no sewer water or pipe flood reaches another storey");
    }

    #[test]
    fn the_lists_take_put_and_count() {
        let mut r = ResidentHunters::default();
        assert!(r.levels().is_empty() && r.total() == 0 && r.list(-1).is_empty());
        r.put(-1, Vec::new());
        assert!(r.has(-1) && !r.has(0));
        assert_eq!(r.levels(), vec![-1]);
        assert!(r.take(-1).is_empty());
        assert!(!r.has(-1), "take removes the list");
        assert!(r.take(7).is_empty(), "taking a level with no list is empty");
        r.list_mut(2);
        assert!(r.has(2), "list_mut makes the list");
    }
}
