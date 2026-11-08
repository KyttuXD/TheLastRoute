//! Verrine: vent lurker (the finite floor's ring of four grates). She is invisible and inert
//! until you enter the vents; then she appears two grates on and hunts you, getting faster the
//! longer you stay in (x0.4 rising 0.025/s to x2.4) (verrine_gimmick.gd, legacy mode).
//!
//! With two or more grates on the floor (`Ctx.vents`) she is PHYSICAL (verrine_gimmick.gd, physical mode): she lives
//! in the duct network (hunters/vent_nav.rs), patrols it grate to grate at crawl speed, follows a runner who crawls
//! in, and climbs out at the grate beside a runner on the floor who comes within `lurk_radius` of one. Out on the
//! floor the brain hunts as usual; with no chase for `return_seconds` she climbs back in at the nearest grate. The
//! speed ramp and every distance and time are data (data/hunters/vents.json). With fewer grates she keeps the legacy
//! behaviour above.

use crate::hunters::brain::{Brain, State, Target};
use crate::hunters::gimmicks::Ctx;
use crate::hunters::vent_nav::{dist, rules, Crawl, Ducts, Phase};

pub struct Verrine {
    pub active_in_vent: bool,
    pub speed_scale: f32,
    /// Set each frame by `before`: the floor has a duct network, so she crawls it instead of hopping grates.
    physical: bool,
    primed: bool,
    ducts: Ducts,
    crawl: Crawl,
    /// The grate she patrols toward.
    patrol: Option<usize>,
    /// The grate she is crawling to climb out at.
    exit_via: Option<usize>,
    out_idle: f32,
    /// Where she thumped this frame (a noise event each; the hook drains them with `take_thumps`).
    thumps: Vec<(f32, f32)>,
    /// Where she burst out of a grate (the hook plays the vent burst and may alert).
    burst: Option<(f32, f32)>,
}

impl Default for Verrine {
    fn default() -> Self {
        Verrine {
            active_in_vent: false,
            speed_scale: 1.0,
            physical: false,
            primed: false,
            ducts: Ducts::default(),
            crawl: Crawl::default(),
            patrol: None,
            exit_via: None,
            out_idle: 0.0,
            thumps: vec![],
            burst: None,
        }
    }
}

/// What the duct tick needs to know about the runner.
pub struct Runner {
    pub pos: (f32, f32),
    pub in_vent: bool,
    /// The brain is chasing (resets her return timer while she is out on the floor).
    pub chasing: bool,
}

impl Verrine {
    /// The mechanic pose she is in: crawling low through the vents after the runner.
    pub fn mechanic(&self) -> Option<&'static str> {
        if self.physical {
            return self.crawl.inside().then_some("vent_stalk");
        }
        self.active_in_vent.then_some("vent_stalk")
    }

    /// The speed ramp: x`initial_scale` rising `per_second` to `max_scale` the longer the runner stays in.
    pub fn ramp(vent_time_in: f32) -> f32 {
        let v = &rules().verrine;
        (v.initial_scale + vent_time_in * v.per_second).clamp(0.0, v.max_scale)
    }

    /// She has a duct network to live in (game/interact.rs then does not hop her to a far grate).
    pub fn physical(&self) -> bool {
        self.physical
    }

    /// In a duct or climbing through a grate (not on the floor).
    pub fn in_duct(&self) -> bool {
        self.physical && self.crawl.inside()
    }

    /// 0 on the floor .. 1 in the duct, for the renderer's climb.
    pub fn duct_height(&self) -> f32 {
        if self.physical {
            self.crawl.height()
        } else {
            0.0
        }
    }

    /// The brain must not walk her: legacy she stands still unless the runner is in the vents; physical the crawl moves her.
    pub fn holds_position(&self) -> bool {
        if self.physical {
            self.crawl.inside()
        } else {
            !self.active_in_vent
        }
    }

    /// She cannot catch a runner on the floor from inside a duct.
    pub fn immune_to_capture(&self) -> bool {
        if self.physical {
            self.crawl.inside() && !self.active_in_vent
        } else {
            !self.active_in_vent
        }
    }

    /// Legacy she is invisible until the runner is in the vents; physical she is seen in the ducts, at the grates and out.
    pub fn hidden_from_view(&self) -> bool {
        !self.physical && !self.active_in_vent
    }

    /// The thumps she made since the last call (positions), for the noise system.
    pub fn take_thumps(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.thumps)
    }

    /// Where she burst out of a grate since the last call.
    pub fn take_burst(&mut self) -> Option<(f32, f32)> {
        self.burst.take()
    }

    /// Before perception: crawl the ducts. Does nothing without a duct network.
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        self.physical = c.vents.len() >= 2;
        if !self.physical {
            self.primed = false;
            return;
        }
        if self.ducts.sync(c.vents) || !self.primed {
            self.prime(b.pos);
            b.pos = self.crawl.pos;
        }
        let runner = Runner { pos: t.pos, in_vent: t.in_vent, chasing: b.state == State::Chase };
        let fac = c.fac;
        let step = rules().verrine.exit_step;
        let clearance = rules().verrine.exit_radius;
        let floor_beside = move |at: (f32, f32), toward: (f32, f32)| {
            let d = dist(at, toward);
            let dir = if d > 0.1 { ((toward.0 - at.0) / d, (toward.1 - at.1) / d) } else { (1.0, 0.0) };
            let preferred = (at.0 + dir.0 * step, at.1 + dir.1 * step);
            let clear = fac.nearest_open_or(preferred.0, preferred.1, clearance);
            if dist(clear, at) <= rules().climb_reach {
                clear
            } else {
                preferred
            }
        };
        let here = b.pos;
        let was_inside = self.crawl.inside();
        let mut rnd = || b.next_float();
        let pos = self.tick(dt, here, &runner, &floor_beside, &mut rnd);
        b.pos = pos;
        if was_inside && !self.crawl.inside() {
            // landed beside the runner: she gives chase from there (verrine_gimmick.gd, climb out done)
            b.chase(t.pos);
        }
        if self.crawl.inside() {
            b.state = if self.active_in_vent { State::Chase } else { State::Wander };
        }
    }

    /// Put her in the duct at the grate nearest `pos` (once per floor). She was never meant to walk the floors.
    pub fn prime(&mut self, pos: (f32, f32)) {
        if let Some((g, _)) = self.ducts.graph.nearest(pos) {
            let at = self.ducts.graph.grates[g];
            self.crawl = Crawl::default();
            self.crawl.place_in_duct(g, at);
            self.patrol = None;
            self.exit_via = None;
            self.out_idle = 0.0;
            self.primed = true;
        }
    }

    /// One frame of her duct life. `brain_pos` is where the brain has her (read while she is on the floor), the closure
    /// gives a clear floor point beside a grate toward a point, `rnd` a number in 0..1. Returns where she is now.
    pub fn tick(
        &mut self,
        dt: f32,
        brain_pos: (f32, f32),
        runner: &Runner,
        floor_beside: &dyn Fn((f32, f32), (f32, f32)) -> (f32, f32),
        rnd: &mut dyn FnMut() -> f32,
    ) -> (f32, f32) {
        let r = rules();
        match self.crawl.phase {
            Phase::Out => {
                self.crawl.pos = brain_pos;
                self.active_in_vent = false;
                self.out_idle = if runner.chasing { 0.0 } else { self.out_idle + dt };
                if self.out_idle >= r.verrine.return_seconds {
                    if let Some(g) = self.ducts.graph.grates_around(brain_pos, r.climb_reach).first().copied() {
                        let at = self.ducts.graph.grates[g];
                        if self.crawl.enter(r, g, at, brain_pos, vec![]) {
                            self.out_idle = 0.0;
                        }
                    }
                }
                return self.crawl.pos;
            }
            Phase::ClimbIn | Phase::ClimbOut => {
                if self.crawl.step(r, dt, 1.0).landed {
                    self.out_idle = 0.0;
                }
                return self.crawl.pos;
            }
            Phase::Duct => {}
        }
        self.active_in_vent = runner.in_vent;
        if self.crawl.path_empty() {
            self.decide(runner, floor_beside, rnd);
        }
        if self.crawl.phase != Phase::Duct {
            // she started climbing out this frame
            self.crawl.step(r, dt, 1.0);
            return self.crawl.pos;
        }
        let s = self.crawl.step(r, dt, self.speed_scale);
        for _ in 0..s.thumps {
            self.thumps.push(self.crawl.pos);
        }
        self.crawl.pos
    }

    /// At a grate with nothing to crawl: pick where to go next, or climb out.
    fn decide(&mut self, runner: &Runner, floor_beside: &dyn Fn((f32, f32), (f32, f32)) -> (f32, f32), rnd: &mut dyn FnMut() -> f32) {
        let r = rules();
        let Some(here) = self.crawl.grate else { return };
        let g = &self.ducts.graph;
        let nearest = g.nearest(runner.pos);
        self.exit_via = None;
        let mut goal = None;
        if runner.in_vent {
            goal = nearest.map(|(i, _)| i);
        } else if let Some((i, d)) = nearest {
            if d <= r.verrine.lurk_radius {
                goal = Some(i);
                self.exit_via = Some(i);
            }
        }
        if goal.is_none() {
            if self.patrol.is_none() || self.patrol == Some(here) {
                let mut near: Vec<usize> =
                    (0..g.grates.len()).filter(|i| *i != here && dist(g.grates[*i], g.grates[here]) <= r.verrine.patrol_reach).collect();
                if near.is_empty() {
                    near = (0..g.grates.len()).filter(|i| *i != here).collect();
                }
                self.patrol = if near.is_empty() { None } else { Some(near[((rnd() * near.len() as f32) as usize).min(near.len() - 1)]) };
            }
            goal = self.patrol;
        }
        let Some(goal) = goal else { return };
        if goal == here {
            if self.exit_via == Some(here) && !runner.in_vent {
                let at = g.grates[here];
                let onto = floor_beside(at, runner.pos);
                if self.crawl.exit_at(r, onto) {
                    self.burst = Some(at);
                    self.patrol = None;
                }
            }
            // in the vents with the runner at this grate she waits on them; on patrol she picks again next frame
            if !runner.in_vent {
                self.patrol = None;
            }
            return;
        }
        if let Some((_, nodes)) = g.shortest(here, goal) {
            // one duct run at a time: the goal can change at every grate
            let hop: Vec<_> = g.waypoints(&nodes[..2]).into_iter().skip(1).collect();
            self.crawl.set_path(hop);
        }
    }

    pub fn after(&mut self, b: &mut Brain, t: &Target) {
        if self.physical {
            self.speed_scale = if self.active_in_vent { Self::ramp(t.vent_time_in) } else { 1.0 };
            if self.crawl.inside() {
                if self.active_in_vent {
                    b.state = State::Chase;
                    b.last_known = Some(t.pos);
                    b.lost_sight = 0.0;
                } else {
                    b.state = State::Wander;
                }
                b.pos = self.crawl.pos;
            }
            return;
        }
        self.active_in_vent = t.in_vent;
        if !t.in_vent {
            b.state = State::Idle;
            b.idle_timer = 1.0;
            b.last_known = None;
            b.lost_sight = 0.0;
            return;
        }
        b.chase(t.pos);
        self.speed_scale = Self::ramp(t.vent_time_in);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::vent_nav::VentGraph;
    use crate::world::Cell;

    fn spot() -> Verrine {
        let mut v = Verrine::default();
        let cells = [Cell::new(2, 2), Cell::new(30, 2), Cell::new(30, 30), Cell::new(2, 30)];
        v.ducts.sync(&cells);
        v.physical = true;
        v.prime((2.5, 2.5));
        v
    }

    fn runner(pos: (f32, f32), in_vent: bool) -> Runner {
        Runner { pos, in_vent, chasing: false }
    }

    fn beside(at: (f32, f32), toward: (f32, f32)) -> (f32, f32) {
        let d = dist(at, toward).max(0.1);
        (at.0 + (toward.0 - at.0) / d, at.1 + (toward.1 - at.1) / d)
    }

    fn run(v: &mut Verrine, secs: f32, who: &Runner, mut pos: (f32, f32)) -> (f32, f32) {
        let mut rnd = || 0.3;
        for _ in 0..(secs / 0.05) as usize {
            pos = v.tick(0.05, pos, who, &beside, &mut rnd);
        }
        pos
    }

    #[test]
    fn the_legacy_ramp_keeps_its_standard_values() {
        assert!((Verrine::ramp(0.0) - 0.4).abs() < 1e-6);
        assert!((Verrine::ramp(20.0) - 0.9).abs() < 1e-6);
        assert!((Verrine::ramp(1000.0) - 2.4).abs() < 1e-6, "capped");
    }

    #[test]
    fn she_patrols_the_ducts_by_crawling_and_never_hops() {
        let mut v = spot();
        let who = runner((60.0, 60.0), false);
        let mut rnd = || 0.3;
        let mut pos = (2.5, 2.5);
        let mut farthest = 0.0f32;
        let mut last = pos;
        for _ in 0..1200 {
            pos = v.tick(0.05, pos, &who, &beside, &mut rnd);
            let step = dist(pos, last);
            farthest = farthest.max(step);
            last = pos;
            assert!(v.crawl.inside(), "far from the runner she stays in the ducts");
        }
        assert!(farthest <= rules().crawl_speed * 0.05 + 1e-3, "no jump from one grate to another");
        assert!(dist(pos, (2.5, 2.5)) > 1.0 || v.patrol.is_some(), "she goes somewhere");
        assert!(!v.take_thumps().is_empty(), "crawling makes sheet-metal thumps");
        assert!(v.take_burst().is_none());
        let on_a_duct = |p: (f32, f32)| {
            let g: &VentGraph = &v.ducts.graph;
            g.ducts.iter().any(|d| d.points.windows(2).any(|w| {
                let (lo_x, hi_x) = (w[0].0.min(w[1].0) - 1e-3, w[0].0.max(w[1].0) + 1e-3);
                let (lo_y, hi_y) = (w[0].1.min(w[1].1) - 1e-3, w[0].1.max(w[1].1) + 1e-3);
                p.0 >= lo_x && p.0 <= hi_x && p.1 >= lo_y && p.1 <= hi_y
            }))
        };
        assert!(on_a_duct(pos), "she is always on a duct");
    }

    #[test]
    fn she_leaves_the_ducts_at_the_grate_beside_a_runner_on_the_floor() {
        let mut v = spot();
        let near_grate_2 = (28.0, 28.0);
        let who = runner(near_grate_2, false);
        let mut pos = (2.5, 2.5);
        let mut left = false;
        for _ in 0..4000 {
            pos = v.tick(0.05, pos, &who, &beside, &mut || 0.3);
            if v.crawl.phase == Phase::ClimbOut {
                left = true;
                break;
            }
        }
        assert!(left, "she climbs out");
        assert!(v.in_duct() && v.duct_height() > 0.9, "the climb starts from the duct");
        let burst = v.take_burst().expect("the burst is reported");
        assert!(dist(burst, (30.5, 30.5)) < 1e-3, "at the grate nearest the runner");
        pos = run(&mut v, rules().exit_seconds + 0.5, &who, pos);
        assert_eq!(v.crawl.phase, Phase::Out);
        assert!(!v.in_duct() && !v.holds_position(), "back on the floor the brain hunts with her");
        assert!(dist(pos, (30.5, 30.5)) <= rules().climb_reach + 0.01, "she lands beside the grate");
    }

    #[test]
    fn a_far_runner_does_not_draw_her_out_and_a_runner_in_the_vents_does_not_either() {
        let mut v = spot();
        let who = runner((16.0, 16.0), false); // 14 cells from every grate: beyond the lurk radius
        run(&mut v, 60.0, &who, (2.5, 2.5));
        assert!(v.crawl.inside() && v.take_burst().is_none());
        let mut w = spot();
        let in_vent = runner((30.5, 30.5), true);
        let pos = run(&mut w, 120.0, &in_vent, (2.5, 2.5));
        assert!(w.crawl.inside(), "she hunts the runner through the ducts");
        assert!(w.active_in_vent);
        assert!(dist(pos, (30.5, 30.5)) < 0.5, "and reaches their grate");
    }

    #[test]
    fn out_on_the_floor_with_no_chase_she_climbs_back_in() {
        let mut v = spot();
        v.crawl = Crawl::default();
        let at = (2.5, 2.9);
        let who = runner((60.0, 60.0), false);
        let mut pos = at;
        let mut back = false;
        for _ in 0..((rules().verrine.return_seconds + 1.0) / 0.05) as usize {
            pos = v.tick(0.05, pos, &who, &beside, &mut || 0.3);
            if v.crawl.inside() {
                back = true;
                break;
            }
        }
        assert!(back, "after return_seconds with no chase");
        let mut chased = spot();
        chased.crawl = Crawl::default();
        let hunting = Runner { pos: (60.0, 60.0), in_vent: false, chasing: true };
        let mut p = at;
        for _ in 0..((rules().verrine.return_seconds * 3.0) / 0.05) as usize {
            p = chased.tick(0.05, p, &hunting, &beside, &mut || 0.3);
        }
        assert!(!chased.crawl.inside(), "a chase keeps her out");
    }

    #[test]
    fn the_speed_ramps_with_time_in_the_vents_and_she_cannot_catch_from_a_duct_on_the_floor() {
        let mut v = spot();
        v.active_in_vent = true;
        assert!(v.holds_position(), "the crawl moves her, not the brain");
        assert!(!v.immune_to_capture(), "she can reach a runner in the vents");
        assert!(!v.hidden_from_view());
        v.active_in_vent = false;
        assert!(v.immune_to_capture(), "from the duct she cannot catch a runner on the floor");
        assert_eq!(v.mechanic(), Some("vent_stalk"));
    }
}
