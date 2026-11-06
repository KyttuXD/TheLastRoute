//! The wolf spiders' home: the Inferno, level -3 (inferno_secrets.gd, docs/SECRET_WOLF_SPIDER.md). The state lives on the game and
//! is updated from the host-side hunter tick; it does nothing unless the runner is on the home level, and nothing at all on a guest
//! (the host spawns the creature like any hunter and its hunter frames carry it).
//!
//! Trigger (`data/hunters/secret_creatures.json`): "inferno" = the runner has spent `dwell_seconds` on level -3 (the clock runs only
//! there and keeps what it has when the runner goes back up); "off" = nothing ever spawns by itself, only the admin picker can summon
//! them. The relationship and the per-difficulty plan decide who comes and whether it is a pair (`secret_creatures::plan_for`, a
//! shared rest through `wolf_spider::PackState`). A creature appears out of reach, already telegraphing (bark or hiss): the warning
//! comes first. It then lurks and hunts on -3 only, its lair on the widest open ground (its legs sprawl 3.5 m).

use super::resident_hunters::hunter_def;
use super::Game;
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::wolf_spider::WolfSpider;
use crate::hunters::gimmicks::Gimmick;
use crate::hunters::secret_creatures::{self as secret, Knobs, Plan, Rules, BLACK, WHITE};
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::f32::consts::TAU;

/// The second creature of a "hard" pair, waiting for the white one to have noticed the runner.
#[derive(Clone, Debug)]
struct PendingJoin {
    id: &'static str,
    left: f32,
    knobs: Knobs,
    stagger: f32,
    flank: bool,
    first_position: (f32, f32),
}

#[derive(Default)]
pub struct InfernoSecrets {
    /// At least one creature has been spawned this run.
    pub fired: bool,
    /// Seconds the runner has spent on the home level.
    pub dwell: f32,
    spawned: Vec<&'static str>,
    pending_join: Option<PendingJoin>,
    /// Test hooks: a trigger, relationship or difficulty without touching the shipped data.
    pub trigger_override: Option<String>,
    pub relationship_override: Option<String>,
    pub difficulty_override: Option<String>,
}

impl InfernoSecrets {
    fn trigger(&self) -> &str {
        self.trigger_override.as_deref().unwrap_or(&Rules::get().trigger)
    }

    fn relationship(&self) -> &str {
        self.relationship_override.as_deref().unwrap_or(&Rules::get().relationship)
    }

    pub fn has_spawned(&self, id: &str) -> bool {
        self.spawned.contains(&id)
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

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// A deterministic open cell `spawn_min..spawn_max` cells from the runner, reachable from the runner, on the widest ground around
/// (seeded by the route seed and the creature). `avoid` (the first creature of a pair): keep at least `pair_min_gap` cells from it,
/// and with `flank` set prefer the far side of the runner (at least `pair_flank_degrees` between the two as seen from the runner).
/// None while no such cell exists (yet).
pub fn pick_lair(
    fac: &Facility,
    rules: &Rules,
    seed: &str,
    runner: (f32, f32),
    id: &str,
    avoid: Option<(f32, f32)>,
    flank: bool,
) -> Option<(f32, f32)> {
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
        if let Some(first) = avoid {
            if dist(world, first) < rules.pair_min_gap {
                continue;
            }
            if flank {
                let a = (first.1 - runner.1).atan2(first.0 - runner.0);
                let b = (world.1 - runner.1).atan2(world.0 - runner.0);
                let mut d = (b - a).rem_euclid(TAU);
                if d > TAU / 2.0 {
                    d = TAU - d;
                }
                if d.to_degrees() < rules.pair_flank_degrees {
                    continue;
                }
            }
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
    if avoid.is_some() && flank {
        return pick_lair(fac, rules, seed, runner, id, avoid, false); // no far side open: relax the flank
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

    /// Every hunter of the active and the resident lists that is one of the two creatures.
    fn secret_exists(&self, id: &str) -> bool {
        self.hunters.iter().chain(self.resident_hunters.iter_all()).any(|h| h.def.id == id)
    }

    /// Creatures that already exist (spawned now, earlier in a saved route, or taken out by an admin) count as done.
    fn secrets_finished(&mut self) -> bool {
        for id in secret::IDS {
            if self.secret_exists(id) && !self.inferno_secrets.spawned.contains(&id) {
                self.inferno_secrets.spawned.push(id);
            }
        }
        self.inferno_secrets.fired = !self.inferno_secrets.spawned.is_empty();
        self.secret_plan().ids.is_empty() && self.inferno_secrets.fired
    }

    /// Which creatures the NEXT trigger brings (empty = nothing more to bring right now).
    fn secret_plan(&self) -> Plan {
        let rules = Rules::get();
        let s = &self.inferno_secrets;
        let seed = self.secret_seed();
        let any = s.has_spawned(WHITE) || s.has_spawned(BLACK);
        match s.relationship() {
            "pair" => {
                if any {
                    Plan::none()
                } else {
                    Plan { ids: vec![WHITE, BLACK], stagger: rules.pair_stagger, ..Plan::none() }
                }
            }
            "black_rare" => {
                if any {
                    return Plan::none();
                }
                let roll = Rng::new(&format!("{seed}:SECRET-PICK")).next_float() as f32;
                Plan { ids: vec![if roll < rules.black_chance { BLACK } else { WHITE }], ..Plan::none() }
            }
            "black_follows_white" => {
                if !s.has_spawned(WHITE) {
                    Plan { ids: vec![WHITE], ..Plan::none() }
                } else if !s.has_spawned(BLACK) && self.sighted.contains(WHITE) {
                    Plan { ids: vec![BLACK], ..Plan::none() }
                } else {
                    Plan::none()
                }
            }
            // one plan per run: the difficulty decided solo or pair, and who, when the trigger first fired
            _ => {
                if any {
                    Plan::none()
                } else {
                    secret::plan_for(rules, &self.secret_difficulty(), &seed)
                }
            }
        }
    }

    /// One host tick: the second creature of a pending pair, then the dwell clock and the trigger.
    pub(super) fn update_inferno_secrets(&mut self, dt: f32) {
        self.update_secret_join(dt);
        let rules = Rules::get();
        if self.inferno_secrets.trigger() == "off" || self.report.is_some() || self.runner_level() != rules.home_level || self.fac.level != rules.home_level {
            return;
        }
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
        let plan = self.secret_plan();
        let Some(&first) = plan.ids.first() else {
            self.inferno_secrets.dwell = 0.0; // armed, but nothing is due yet (the black one waits until the white one has been met)
            return;
        };
        let runner = self.p.pos;
        let seed = self.secret_seed();
        let at = pick_lair(&self.fac, rules, &seed, runner, first, None, false);
        let Some(at) = at.filter(|at| self.spawn_secret(first, *at, runner, plan.knobs, 0.0)) else {
            self.inferno_secrets.dwell = rules.dwell_seconds * 0.9; // nowhere open yet (the floor is still streaming in): try again soon
            return;
        };
        if let Some(&second) = plan.ids.get(1) {
            if plan.join_after_bark {
                // the black one joins once the white one has noticed the runner: `join_delay` s later, from another side
                self.inferno_secrets.pending_join =
                    Some(PendingJoin { id: second, left: plan.join_delay, knobs: plan.knobs, stagger: plan.stagger, flank: plan.flank, first_position: at });
            } else {
                self.spawn_second_secret(second, runner, at, plan.knobs, plan.stagger, plan.flank);
            }
        }
        self.inferno_secrets.dwell = 0.0;
    }

    fn spawn_second_secret(&mut self, id: &'static str, runner: (f32, f32), first: (f32, f32), knobs: Knobs, stagger: f32, flank: bool) -> bool {
        let seed = self.secret_seed();
        let Some(at) = pick_lair(&self.fac, Rules::get(), &seed, runner, id, Some(first), flank) else {
            return false;
        };
        self.spawn_secret(id, at, runner, knobs, stagger)
    }

    fn update_secret_join(&mut self, dt: f32) {
        let Some(join) = self.inferno_secrets.pending_join.as_mut() else { return };
        join.left -= dt;
        if join.left > 0.0 || self.report.is_some() {
            return;
        }
        let join = self.inferno_secrets.pending_join.take().unwrap();
        if self.runner_level() == Rules::get().home_level {
            self.spawn_second_secret(join.id, self.p.pos, join.first_position, join.knobs, 0.0, join.flank);
        }
    }

    /// Puts creature `id` on the home level at `at`, already marking `target` (its tell comes first; `bark_delay` staggers the second
    /// of a pair). False when the runner is not on the home level or the creature has no definition.
    pub fn spawn_secret(&mut self, id: &'static str, at: (f32, f32), target: (f32, f32), knobs: Knobs, bark_delay: f32) -> bool {
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
            s.start_with_bark(bark_delay);
            s.apply_knobs(&mut b, &knobs);
        }
        b.gimmick = g;
        self.hunters.push(b);
        self.link_wolf_spider_pair();
        let s = &mut self.inferno_secrets;
        if !s.spawned.contains(&id) {
            s.spawned.push(id);
        }
        s.fired = true;
        true
    }

    /// Both creatures on the active list: one shared rest timer and recall. Links only once the other one of the pair exists.
    pub fn link_wolf_spider_pair(&mut self) {
        let find = |g: &Game, id: &str| g.hunters.iter().position(|h| h.def.id == id);
        let (Some(i), Some(j)) = (find(self, WHITE), find(self, BLACK)) else { return };
        let mut a = std::mem::take(&mut self.hunters[i].gimmick);
        let mut b = std::mem::take(&mut self.hunters[j].gimmick);
        if let (Gimmick::WolfSpider(x), Gimmick::WolfSpider(y)) = (&mut a, &mut b) {
            WolfSpider::link(x, y);
        }
        self.hunters[i].gimmick = a;
        self.hunters[j].gimmick = b;
    }

    /// The admin pair summon: link the two creatures and give them the current difficulty's knobs (SecretCreatures.link_admin_pair).
    pub fn link_admin_wolf_spiders(&mut self) {
        let knobs = Rules::get().row(&self.secret_difficulty()).knobs;
        for id in secret::IDS {
            let Some(i) = self.hunters.iter().rposition(|h| h.def.id == id) else { return };
            let mut g = std::mem::take(&mut self.hunters[i].gimmick);
            if let Gimmick::WolfSpider(s) = &mut g {
                s.apply_knobs(&mut self.hunters[i], &knobs);
            }
            self.hunters[i].gimmick = g;
        }
        self.link_wolf_spider_pair();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Controls;
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
        assert_eq!(found.len(), 1, "relaxed brings the white one alone");
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
        assert_eq!(spiders(&a).len(), 1, "never a second plan");
    }

    #[test]
    fn nightmare_spawns_a_flanking_pair_that_shares_a_rest() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = inferno(Difficulty::Nightmare, 3, &content, &catalog);
        dwell_out(&mut g);
        let found = spiders(&g);
        assert_eq!(found.len(), 2, "nightmare always brings the pair");
        let rules = Rules::get();
        assert!(dist(found[0].pos, found[1].pos) >= rules.pair_min_gap - 1.0, "placed apart");
        for h in found {
            match &h.gimmick {
                Gimmick::WolfSpider(s) => assert!(s.in_pack(), "the pair is linked"),
                _ => panic!("no gimmick"),
            }
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
        assert!(!sewer.spawn_secret(WHITE, sewer.p.pos, sewer.p.pos, Knobs::default(), 0.0), "the summon refuses another level");
        // the off switch
        let mut g = inferno(Difficulty::Nightmare, 4, &content, &catalog);
        g.inferno_secrets.trigger_override = Some("off".into());
        for _ in 0..80 {
            g.update_inferno_secrets(1.0);
        }
        assert!(spiders(&g).is_empty() && !g.inferno_secrets.fired, "off leaves no trace");
    }

    #[test]
    fn a_hard_pair_brings_the_black_one_after_the_bark() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let rules = Rules::get();
        // find a seed text whose hard plan is a pair that joins after the bark
        let seed = (0..200)
            .map(|n| format!("ROOM-INFERNO-{n}"))
            .find(|room| {
                let g = Game::new_infinite_sim(Difficulty::Hard, 7, room, INFERNO_LEVEL, &content, &catalog);
                secret::plan_for(rules, "hard", &g.secret_seed()).ids.len() == 2
            })
            .expect("a pair seed");
        let mut g = Game::new_infinite_sim(Difficulty::Hard, 7, &seed, INFERNO_LEVEL, &content, &catalog);
        dwell_out(&mut g);
        assert_eq!(spiders(&g).len(), 1, "the white one first");
        assert!(g.inferno_secrets.pending_join.is_some());
        g.update_inferno_secrets(rules.row("hard").join_delay + 0.1);
        assert_eq!(spiders(&g).len(), 2, "the black one joins 3 s later");
    }

    #[test]
    fn the_admin_pair_link_applies_the_difficulty_knobs() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = inferno(Difficulty::Nightmare, 5, &content, &catalog);
        for id in secret::IDS {
            assert!(g.spawn_secret(id, g.p.pos, g.p.pos, Knobs::default(), 0.0));
        }
        let before: Vec<f32> = spiders(&g).iter().map(|h| h.def.chase).collect();
        g.link_admin_wolf_spiders();
        let after: Vec<f32> = spiders(&g).iter().map(|h| h.def.chase).collect();
        assert!(before.iter().zip(&after).all(|(b, a)| a > b), "nightmare chases faster: {before:?} -> {after:?}");
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
