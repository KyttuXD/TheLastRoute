//! The wolf spider's home: the Inferno, level -3 (inferno_secrets.gd, docs/SECRET_WOLF_SPIDER.md). The state lives on the game and
//! is updated from the host-side hunter tick; it does nothing unless the runner is on the home level, and nothing at all on a guest
//! (the host spawns the creature like any hunter and its hunter frames carry it).
//!
//! Trigger (`data/hunters/secret_creatures.json`): "inferno" = the runner has spent `dwell_seconds` on level -3 (the clock runs only
//! there and keeps what it has when the runner goes back up); "off" = nothing ever spawns by itself, only the admin picker can summon
//! it. The creature comes once per run, with the difficulty's knobs. It appears out of reach, already telegraphing (bark): the warning
//! comes first. It then lurks and hunts on -3 only, its lair on the widest open ground (its legs sprawl 3.5 m).
//!
//! Who comes is the plan of the difficulty (`Rules::plan_for`, data `difficulty.<id>.plan`): the white one alone, the black one alone
//! or a PAIR. A pair starts from two lairs at least `pair.min_gap` cells apart (on opposite sides of the runner when it flanks); the
//! second creature spawns `second_delay` seconds after the first (the stagger, or the join delay on hard) and the two share a rest
//! and a recall (`WolfSpider::link`).

use super::resident_hunters::hunter_def;
use super::Game;
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Gimmick;
use crate::hunters::secret_creatures::{self as secret, Knobs, Rules};
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::f32::consts::TAU;

#[derive(Default)]
pub struct InfernoSecrets {
    /// At least one creature has been spawned this run.
    pub fired: bool,
    /// Seconds the runner has spent on the home level.
    pub dwell: f32,
    spawned: Vec<&'static str>,
    /// The second creature of a pair, waiting for its turn.
    pending: Option<PendingSecret>,
    /// Test hooks: a trigger or difficulty without touching the shipped data.
    pub trigger_override: Option<String>,
    pub difficulty_override: Option<String>,
}

/// A creature that spawns after a delay (the second of a pair).
#[derive(Clone, Debug)]
pub struct PendingSecret {
    pub id: &'static str,
    pub left: f32,
    pub at: (f32, f32),
    pub knobs: Knobs,
}

impl InfernoSecrets {
    /// The creature waiting to join, if any (tests read it).
    pub fn pending(&self) -> Option<&PendingSecret> {
        self.pending.as_ref()
    }

    fn trigger(&self) -> &str {
        self.trigger_override.as_deref().unwrap_or(&Rules::get().trigger)
    }
}

/// Share of the square of `radius` around `cell` that a hunter can stand on (1.0 = open ground all round).
pub fn clearance(fac: &Facility, cell: Cell, radius: i32) -> f32 {
    let mut open = 0;
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            if fac.is_floor((cell.x + dx) as f64 + 0.5, (cell.y + dz) as f64 + 0.5) {
                open += 1;
            }
        }
    }
    let side = radius * 2 + 1;
    open as f32 / (side * side) as f32
}

/// A deterministic open cell `spawn_min..spawn_max` cells from the runner, reachable from the runner, on the widest ground around
/// (seeded by the route seed and the creature). None while no such cell exists (yet).
pub fn pick_lair(fac: &Facility, rules: &Rules, seed: &str, runner: (f32, f32), id: &str) -> Option<(f32, f32)> {
    pick_lair_where(fac, rules, seed, runner, id, &|_| true)
}

/// The angle in degrees (0..=180) between the directions of `a` and `b` as seen from `from`.
pub fn angle_between(from: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (va, vb) = ((a.0 - from.0, a.1 - from.1), (b.0 - from.0, b.1 - from.1));
    let (la, lb) = ((va.0 * va.0 + va.1 * va.1).sqrt(), (vb.0 * vb.0 + vb.1 * vb.1).sqrt());
    if la < 1e-3 || lb < 1e-3 {
        return 0.0;
    }
    ((va.0 * vb.0 + va.1 * vb.1) / (la * lb)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// The lair of the second creature of a pair: like [`pick_lair`] but at least `pair_min_gap` cells from `first`, and when
/// `flank` at least `pair_flank_degrees` away from it around the runner. Falls back to the gap alone when no cell flanks.
pub fn pick_second_lair(fac: &Facility, rules: &Rules, seed: &str, runner: (f32, f32), id: &str, first: (f32, f32), flank: bool) -> Option<(f32, f32)> {
    let far = |at: (f32, f32)| ((at.0 - first.0).powi(2) + (at.1 - first.1).powi(2)).sqrt() >= rules.pair_min_gap;
    if flank {
        let found = pick_lair_where(fac, rules, seed, runner, id, &|at| far(at) && angle_between(runner, first, at) >= rules.pair_flank_degrees);
        if found.is_some() {
            return found;
        }
    }
    pick_lair_where(fac, rules, seed, runner, id, &far)
}

fn pick_lair_where(fac: &Facility, rules: &Rules, seed: &str, runner: (f32, f32), id: &str, accept: &dyn Fn((f32, f32)) -> bool) -> Option<(f32, f32)> {
    let mut rng = Rng::new(&format!("{seed}:SECRET-LAIR:{id}"));
    let here = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
    let (mut wide, mut roomy, mut any): (Vec<(f32, f32)>, Vec<(f32, f32)>, Vec<(f32, f32)>) = (vec![], vec![], vec![]);
    for _ in 0..rules.lair_samples {
        let angle = rng.next_float() as f32 * TAU;
        let distance = rules.spawn_min_distance + (rules.spawn_max_distance - rules.spawn_min_distance) * rng.next_float() as f32;
        let cell = Cell::new(here.x + (angle.cos() * distance).round() as i32, here.y + (angle.sin() * distance).round() as i32);
        if !fac.hunter_open(cell) {
            continue;
        }
        let world = cell.center();
        if !accept(world) {
            continue;
        }
        let open = clearance(fac, cell, rules.clearance_radius);
        if open >= rules.wide_clearance {
            wide.push(world);
        } else if open >= rules.roomy_clearance {
            roomy.push(world);
        } else {
            any.push(world);
        }
    }
    for mut pool in [wide, roomy, any] {
        while !pool.is_empty() {
            let at = pool.remove(rng.next_index(pool.len()));
            let from = Cell::new(at.0.floor() as i32, at.1.floor() as i32);
            if !fac.find_path(from, here).is_empty() {
                return Some(at);
            }
        }
    }
    None
}

impl Game {
    fn secret_seed(&self) -> String {
        self.inf.as_ref().map_or_else(|| self.run_seed.to_string(), |i| i.seed.clone())
    }

    fn secret_difficulty(&self) -> String {
        self.inferno_secrets.difficulty_override.clone().unwrap_or_else(|| self.difficulty.id().to_string())
    }

    /// Every hunter of the active and the resident lists that is the creature.
    fn secret_exists(&self, id: &str) -> bool {
        self.hunters.iter().chain(self.resident_hunters.iter_all()).any(|h| h.def.id == id)
    }

    /// A creature that already exists (spawned now, earlier in a saved route, or taken out by an admin) counts as done.
    fn secrets_finished(&mut self) -> bool {
        for id in secret::IDS {
            if self.secret_exists(id) && !self.inferno_secrets.spawned.contains(&id) {
                self.inferno_secrets.spawned.push(id);
            }
        }
        self.inferno_secrets.fired = !self.inferno_secrets.spawned.is_empty();
        self.inferno_secrets.fired
    }

    /// One host tick: the dwell clock and the trigger.
    pub(super) fn update_inferno_secrets(&mut self, dt: f32) {
        let rules = Rules::get();
        if self.inferno_secrets.trigger() == "off" || self.report.is_some() || self.runner_level() != rules.home_level || self.fac.level != rules.home_level {
            return;
        }
        self.tick_pending_secret(dt);
        if self.secrets_finished() {
            return;
        }
        self.inferno_secrets.dwell += dt;
        if self.inferno_secrets.dwell >= rules.dwell_seconds {
            self.fire_secrets();
        }
    }

    fn fire_secrets(&mut self) {
        let rules = Rules::get();
        let runner = self.p.pos;
        let seed = self.secret_seed();
        let plan = rules.plan_for(&self.secret_difficulty(), &seed);
        let first = plan.ids[0];
        let lair = pick_lair(&self.fac, rules, &seed, runner, first);
        let spawned = lair.is_some_and(|at| self.spawn_secret(first, at, runner, plan.knobs));
        if !spawned {
            self.inferno_secrets.dwell = rules.dwell_seconds * 0.9; // nowhere open yet (the floor is still streaming in): try again soon
            return;
        }
        self.inferno_secrets.dwell = 0.0;
        // a pair: the second lair is picked now (the first is known), the creature comes after its delay; with no room for it the
        // first one comes alone
        if let (Some(&second), Some(first_at)) = (plan.ids.get(1), lair) {
            if let Some(at) = pick_second_lair(&self.fac, rules, &seed, runner, second, first_at, plan.flank) {
                self.inferno_secrets.pending = Some(PendingSecret { id: second, left: plan.second_delay, at, knobs: plan.knobs });
            }
        }
    }

    /// Counts down the second creature of a pair and spawns it, marking the runner, then links the two.
    fn tick_pending_secret(&mut self, dt: f32) {
        let Some(p) = self.inferno_secrets.pending.as_mut() else { return };
        p.left -= dt;
        if p.left > 0.0 {
            return;
        }
        let Some(p) = self.inferno_secrets.pending.take() else { return };
        if self.spawn_secret(p.id, p.at, self.p.pos, p.knobs) {
            self.link_spider_pair();
        }
    }

    /// Puts every secret creature of the route into one pair: they share a rest and a recall.
    pub fn link_spider_pair(&mut self) {
        let mut pack = None;
        for h in self.hunters.iter_mut().filter(|h| secret::is_secret(&h.def.id)) {
            if let Gimmick::WolfSpider(s) = &mut h.gimmick {
                match &pack {
                    None => pack = Some(s.pack_handle()),
                    Some(p) if !s.in_pack() => s.join_pack(p.clone()),
                    Some(_) => {}
                }
            }
        }
    }

    /// Puts creature `id` on the home level at `at`, already marking `target` (its tell comes first). False when the runner is not
    /// on the home level or the creature has no definition.
    pub fn spawn_secret(&mut self, id: &'static str, at: (f32, f32), target: (f32, f32), knobs: Knobs) -> bool {
        let level = Rules::get().home_level;
        if self.runner_level() != level || self.fac.level != level {
            return false;
        }
        let Some(def) = hunter_def(id) else { return false };
        let mut b = Brain::new(def, at, &format!("{}:SECRET:{id}", self.level_seed()), self.difficulty);
        self.settle_new_brain(&mut b);
        b.activate_at(target);
        let look = (target.0 - at.0, target.1 - at.1);
        let l = (look.0 * look.0 + look.1 * look.1).sqrt();
        if l > 1e-3 {
            b.facing = (look.0 / l, look.1 / l);
        }
        let mut g = std::mem::take(&mut b.gimmick);
        if let Gimmick::WolfSpider(s) = &mut g {
            s.start_with_bark();
            s.apply_knobs(&mut b, &knobs);
        }
        b.gimmick = g;
        self.hunters.push(b);
        let s = &mut self.inferno_secrets;
        if !s.spawned.contains(&id) {
            s.spawned.push(id);
        }
        s.fired = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Controls;
    use crate::hunters::secret_creatures::{BLACK, WHITE};
    use crate::world::catalog::Catalog;
    use crate::world::infinite::INFERNO_LEVEL;

    fn inferno(difficulty: Difficulty, n: usize, content: &Content, catalog: &Catalog) -> Game {
        let g = Game::new_infinite_sim(difficulty, 7, &format!("ROOM-INFERNO-{n}"), INFERNO_LEVEL, content, catalog);
        assert_eq!(g.fac.level, INFERNO_LEVEL);
        g
    }

    fn spiders(g: &Game) -> Vec<&Brain> {
        g.hunters.iter().filter(|h| secret::is_secret(&h.def.id)).collect()
    }

    /// Runs the trigger clock past the dwell on the home level.
    fn dwell_out(g: &mut Game) {
        for _ in 0..40 {
            g.update_inferno_secrets(1.0);
            if g.inferno_secrets.fired {
                break;
            }
        }
    }

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn thirty_seconds_on_level_minus_three_bring_the_creature_out_of_reach() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = inferno(Difficulty::Relaxed, 1, &content, &catalog);
        g.update_inferno_secrets(10.0);
        g.update_inferno_secrets(10.0);
        assert!(spiders(&g).is_empty(), "20 s is not enough");
        assert!(g.inferno_secrets.dwell > 19.0);
        dwell_out(&mut g);
        let found = spiders(&g);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].def.id, WHITE);
        let d = dist(found[0].pos, g.p.pos);
        let rules = Rules::get();
        assert!(d >= rules.spawn_min_distance - 1.5 && d <= rules.spawn_max_distance + 1.5, "spawned {d} cells away");
        assert!(g.fac.find_path(Cell::new(found[0].pos.0 as i32, found[0].pos.1 as i32), Cell::new(g.p.pos.0 as i32, g.p.pos.1 as i32)).len() > 1, "on ground reachable from the runner");
        assert_eq!(found[0].level, INFERNO_LEVEL);
        // it appears already telegraphing: the first tick of its brain starts the bark
        let (c, cat) = (&content, &catalog);
        g.step(1.0 / 30.0, &Controls::default(), c, cat, &Sounds::default());
        let spider = g.hunters.iter().find(|h| secret::is_secret(&h.def.id)).unwrap();
        match &spider.gimmick {
            Gimmick::WolfSpider(s) => assert!(s.bark_seq >= 1, "already barking"),
            _ => panic!("no gimmick"),
        }
    }

    #[test]
    fn it_fires_once_per_run_and_the_same_seed_picks_the_same_lair() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut a = inferno(Difficulty::Relaxed, 2, &content, &catalog);
        let mut b = inferno(Difficulty::Relaxed, 2, &content, &catalog);
        dwell_out(&mut a);
        dwell_out(&mut b);
        assert_eq!(spiders(&a)[0].pos, spiders(&b)[0].pos, "same seed, same lair");
        for _ in 0..80 {
            a.update_inferno_secrets(1.0);
        }
        assert_eq!(spiders(&a).len(), 1, "never a second spawn");
    }

    #[test]
    fn nightmare_brings_the_pair_with_the_harder_knobs_the_white_one_first() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = inferno(Difficulty::Nightmare, 3, &content, &catalog);
        dwell_out(&mut g);
        let found = spiders(&g);
        assert_eq!(found.len(), 1, "the white one tells first");
        assert_eq!(found[0].def.id, WHITE);
        let base = content.hunter(WHITE).unwrap().chase;
        assert!((found[0].def.chase - base * Rules::get().knobs_for("nightmare").chase).abs() < 1e-4, "nightmare chases faster");
        let wait = g.inferno_secrets.pending().expect("the black one waits for its turn");
        assert_eq!(wait.id, BLACK);
        let delay = Rules::get().plan_for("nightmare", "x").second_delay;
        g.update_inferno_secrets(delay * 0.5);
        assert_eq!(spiders(&g).len(), 1, "not before the stagger is over");
        g.update_inferno_secrets(delay);
        let found = spiders(&g);
        assert_eq!(found.len(), 2, "then the second joins");
        assert!(g.inferno_secrets.pending().is_none());
        assert_eq!(found.iter().filter(|h| h.def.id == BLACK).count(), 1);
        let gap = dist(found[0].pos, found[1].pos);
        assert!(gap >= Rules::get().pair_min_gap - 1.5, "the pair starts {gap} cells apart");
        // they share a pack: one rest and a recall (needs black_wolf_spider wired to the wolf spider gimmick in gimmicks/mod.rs)
        for h in found {
            match &h.gimmick {
                Gimmick::WolfSpider(s) => assert!(s.in_pack(), "{} is in the pair", h.def.id),
                _ => panic!("{} has no wolf spider gimmick", h.def.id),
            }
        }
        for _ in 0..80 {
            g.update_inferno_secrets(1.0);
        }
        assert_eq!(spiders(&g).len(), 2, "never a third");
    }

    #[test]
    fn the_second_lair_keeps_its_distance_and_the_angle_is_measured_around_the_runner() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let g = inferno(Difficulty::Standard, 8, &content, &catalog);
        let (rules, runner) = (Rules::get(), g.p.pos);
        let first = pick_lair(&g.fac, rules, "PAIR-SEED", runner, WHITE).expect("a first lair");
        for flank in [false, true] {
            let second = pick_second_lair(&g.fac, rules, "PAIR-SEED", runner, BLACK, first, flank).expect("a second lair");
            assert!(dist(first, second) >= rules.pair_min_gap, "flank {flank}: {} apart", dist(first, second));
            assert_eq!(second, pick_second_lair(&g.fac, rules, "PAIR-SEED", runner, BLACK, first, flank).unwrap(), "deterministic");
        }
        assert!((angle_between((0.0, 0.0), (1.0, 0.0), (-1.0, 0.0)) - 180.0).abs() < 1e-3);
        assert!((angle_between((0.0, 0.0), (1.0, 0.0), (0.0, 2.0)) - 90.0).abs() < 1e-3);
        assert_eq!(angle_between((0.0, 0.0), (1.0, 0.0), (3.0, 0.0)), 0.0);
    }

    #[test]
    fn a_solo_plan_spawns_the_creature_the_plan_names() {
        let (content, catalog) = (Content::load(), Catalog::load());
        // the relaxed difficulty is always the white one alone, whatever the seed
        for n in [11, 12] {
            let mut g = inferno(Difficulty::Relaxed, n, &content, &catalog);
            dwell_out(&mut g);
            assert!(g.inferno_secrets.pending().is_none());
            let found = spiders(&g);
            assert_eq!((found.len(), found[0].def.id.as_str()), (1, WHITE));
        }
    }

    #[test]
    fn nothing_spawns_off_the_home_level_or_with_the_trigger_off() {
        let (content, catalog) = (Content::load(), Catalog::load());
        // the first sewer: the clock never runs
        let mut sewer = Game::new_infinite_sim(Difficulty::Nightmare, 7, "ROOM-SEWER-1", -1, &content, &catalog);
        for _ in 0..80 {
            sewer.update_inferno_secrets(1.0);
        }
        assert!(spiders(&sewer).is_empty() && sewer.inferno_secrets.dwell == 0.0);
        assert!(!sewer.spawn_secret(WHITE, sewer.p.pos, sewer.p.pos, Knobs::default()), "the summon refuses another level");
        // the off switch
        let mut g = inferno(Difficulty::Nightmare, 4, &content, &catalog);
        g.inferno_secrets.trigger_override = Some("off".into());
        for _ in 0..80 {
            g.update_inferno_secrets(1.0);
        }
        assert!(spiders(&g).is_empty() && !g.inferno_secrets.fired, "off leaves no trace");
    }

    #[test]
    fn a_guest_receives_the_phase_and_the_bark_number_in_the_host_frame() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut host = inferno(Difficulty::Relaxed, 7, &content, &catalog);
        dwell_out(&mut host);
        for _ in 0..3 {
            host.step(1.0 / 30.0, &Controls::default(), &content, &catalog, &Sounds::default());
        }
        let bark = match &host.hunters.iter().find(|h| h.def.id == WHITE).unwrap().gimmick {
            Gimmick::WolfSpider(s) => s.network_state(),
            _ => panic!("no gimmick"),
        };
        assert!(bark.1 >= 1, "the host spider has barked");
        let frame = host.hunter_frame();
        let entry = frame.iter().find(|e| e["id"] == WHITE).expect("the spider is in the frame");
        assert_eq!((entry["phase"].as_u64(), entry["bark_seq"].as_u64()), (Some(bark.0 as u64), Some(bark.1 as u64)));
        // a hunter that is no spider carries neither key
        assert!(frame.iter().filter(|e| e["id"] != WHITE).all(|e| e.get("phase").is_none() && e.get("bark_seq").is_none()));
        let mut guest = inferno(Difficulty::Relaxed, 7, &content, &catalog);
        guest.remote_hunters = true;
        guest.apply_hunter_frame(&frame);
        let shown = guest.hunters.iter().find(|h| h.def.id == WHITE).expect("the guest made it");
        match &shown.gimmick {
            Gimmick::WolfSpider(s) => assert_eq!(s.network_state(), bark),
            _ => panic!("no gimmick on the guest's copy"),
        }
    }

    #[test]
    fn clearance_counts_open_ground() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let g = inferno(Difficulty::Standard, 6, &content, &catalog);
        let cells = g.fac.floor_cells();
        let best = cells.iter().step_by(11).map(|c| clearance(&g.fac, *c, 3)).fold(0.0f32, f32::max);
        let worst = cells.iter().step_by(11).map(|c| clearance(&g.fac, *c, 3)).fold(1.0f32, f32::min);
        assert!(best > worst, "ground varies: {worst}..{best}");
        assert!((0.0..=1.0).contains(&best) && (0.0..=1.0).contains(&worst));
    }
}
