//! Mutual light sight on the host (docs/LIGHT_SIGHT.md, hunters/light_field.rs). About five times a second per hunter the
//! host works out how lit the hunter stands and how lit the runner (or bot) it is after stands, from the lamps that
//! light the floor on screen, and gives both to the brain (`Brain::self_exposure`, `target_exposure`). The brain's
//! `sight_limit` does the rest: a runner standing in the building's light is seen far off with the flashlight off, a
//! runner in the dark only close up, and a hunter standing in light sees at full range. A guest runs no hunters and
//! computes nothing; the admin MUTUAL LIGHT SIGHT switch turns the rule off (both exposures None, the old flat scale).

use super::Game;
use crate::hunters::brain::Target;
use crate::hunters::light_field::{self, Source};
use crate::render::{ambient, lamp_flicker};

/// What one `update_hunters` frame shares between its hunters: the lamps as the field reads them, the ambient floor, and
/// the exposure of each point already asked for (every hunter after the same runner asks for the same spot).
#[derive(Default)]
pub(super) struct LightFrame {
    sources: Option<Vec<Source>>,
    ambient: f32,
    asked: Vec<([f32; 3], f32)>,
}

impl Game {
    /// The lamps of the floor the runner is on, with this moment's flicker applied (a lamp dark in its flicker lights
    /// nothing). The same lamps the renderer draws, so what the player sees lit counts as lit for the hunters. The
    /// flicker is applied whatever the reduced motion setting says: the setting is a view option, not a game rule.
    fn light_sources(&self) -> Vec<Source> {
        self.world
            .lamps
            .iter()
            .map(|l| Source {
                pos: [l.pos.x, l.pos.y, l.pos.z],
                range: l.range,
                energy: l.energy * lamp_flicker::factor(self.time, l.flicker),
            })
            .collect()
    }

    /// The exposure of a spot with no lamp on it: the ambient the renderer lights the runner's place with (its luminance),
    /// never below Godot's near-black floor. The render ambient is brighter than Godot's, and a hunter should lose the
    /// runner no sooner than the player loses the hunter.
    fn light_ambient(&self) -> f32 {
        let (x, z) = self.p.pos;
        let level = self.runner_level();
        let region = if level == 0 { self.fac.region_at(x.floor() as i32, z.floor() as i32) } else { "" };
        let look = ambient::table().look(region, level);
        let [r, g, b] = look.ambient_color;
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        (look.ambient_energy * luma).max(light_field::AMBIENT)
    }

    /// Refreshes hunter `i`'s two exposures when they are due. The runner's side is left None when the target is on
    /// another level (the 3D `cross` sight answers there), and the hunter's side when the hunter is not on the floor whose
    /// lamps are held. A floor with no lamps at all has no light data, so the rule is off for it.
    pub(super) fn refresh_light_sight(&mut self, i: usize, dt: f32, target: &Target, frame: &mut LightFrame) {
        let on = !self.remote_hunters && crate::shared::admin_flags::flags().light_sight;
        if !on {
            let h = &mut self.hunters[i];
            h.self_exposure = None;
            h.target_exposure = None;
            h.light_timer = 0.0;
            return;
        }
        if !self.hunters[i].light_due(dt, 0.02 * (i % 4) as f32) {
            return;
        }
        if frame.sources.is_none() {
            frame.sources = Some(self.light_sources());
            frame.ambient = self.light_ambient();
        }
        let sources = frame.sources.as_deref().unwrap_or(&[]);
        if sources.is_empty() {
            let h = &mut self.hunters[i];
            h.self_exposure = None;
            h.target_exposure = None;
            return;
        }
        let amb = frame.ambient;
        let fac = &self.fac;
        let clear = |a: (f32, f32), b: (f32, f32)| fac.has_line_of_sight(a, b);
        let h = &self.hunters[i];
        let own = (h.level == self.runner_level())
            .then(|| light_field::exposure_at([h.pos.0, h.y + light_field::BODY_HEIGHT, h.pos.1], amb, sources, clear));
        let theirs = if target.cross.is_some() {
            None
        } else {
            let at = [target.pos.0, target.y + light_field::BODY_HEIGHT, target.pos.1];
            Some(match frame.asked.iter().find(|(p, _)| *p == at) {
                Some((_, e)) => *e,
                None => {
                    let e = light_field::exposure_at(at, amb, sources, clear);
                    frame.asked.push((at, e));
                    e
                }
            })
        };
        let h = &mut self.hunters[i];
        h.self_exposure = own;
        h.target_exposure = theirs;
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Game;
    use crate::hunters::brain::{Brain, State};
    use crate::hunters::light_field::{self, VISIBLE_EXPOSURE};
    use crate::world::catalog::Catalog;

    fn game() -> (Game, Content) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.hunters.clear();
        g.world.lamps = crate::render::world_mesh::lamps(&g.fac, &g.lights_on, &content);
        (g, content)
    }

    /// A hunter spot and a runner spot in one lit room: the runner stands under a room lamp, the hunter has a clear
    /// line to her and stands further off than her unlit sight (half her range) but inside her full range.
    fn lit_room(g: &Game, content: &Content) -> ((f32, f32), (f32, f32)) {
        let prof = g.prof();
        let probe = Brain::new(content.hunters[0].clone(), (0.0, 0.0), "T", Difficulty::Standard);
        let full = probe.def.sight * prof.hunter_sight * probe.gimmick.sight_scale();
        let unlit = probe.sight_limit(false, &prof, &probe.gimmick);
        assert!(full > unlit + 0.6, "a window between unlit ({unlit}) and full ({full}) sight");
        let (sources, amb) = (g.light_sources(), g.light_ambient());
        for l in &g.fac.room_lights {
            let runner = l.center.center();
            // how far the lit runner is seen (the light rule), and never further than the hunter's full range
            let e = light_field::exposure_at([runner.0, 1.2, runner.1], amb, &sources, |a, b| g.fac.has_line_of_sight(a, b));
            let reach = (full * light_field::unlit_scale(e)).min(full);
            for c in g.fac.floor_cells() {
                let at = c.center();
                let d = crate::game::pdist(at, runner);
                if d > unlit + 0.3
                    && d < reach - 0.3
                    && g.fac.hunter_open(c)
                    && g.fac.body_fits(at.0, at.1, crate::config::HUNTER_RADIUS)
                    && g.fac.has_line_of_sight(at, runner)
                {
                    return (at, runner);
                }
            }
        }
        panic!("a room with a clear line of {unlit}..{full} cells");
    }

    fn put(g: &mut Game, content: &Content, hunter: (f32, f32), runner: (f32, f32)) {
        g.p.pos = runner;
        g.p.flashlight_on = false;
        assert!(!g.in_safe_zone(), "the runner is not sheltered");
        let mut h = Brain::new(content.hunters[0].clone(), hunter, "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        g.hunters.clear();
        g.hunters.push(h);
    }

    fn frames(g: &mut Game, n: usize) {
        let prof = g.prof();
        for _ in 0..n {
            g.update_hunters(1.0 / 60.0, &prof, &Sounds::default());
        }
    }

    /// The reported bug: the runner stands in a room the building lights clearly, the hunter has a clear line to her
    /// and stands inside her sight range, and the hunter did not see her (the host never fed the light rule, so a
    /// runner with the flashlight off was seen at half range whatever the lighting).
    #[test]
    fn a_hunter_with_a_clear_line_sees_a_runner_standing_in_a_lit_room() {
        let (mut g, content) = game();
        let (hunter, runner) = lit_room(&g, &content);
        assert!(!g.world.lamps.is_empty(), "the floor has lamps");
        put(&mut g, &content, hunter, runner);
        frames(&mut g, 6);
        let h = &g.hunters[0];
        assert!(h.target_exposure.is_some_and(|e| e >= VISIBLE_EXPOSURE), "the runner stands in the light: {:?}", h.target_exposure);
        let prof = g.prof();
        let limit = h.sight_limit(false, &prof, &h.gimmick);
        let d = crate::game::pdist(h.pos, runner);
        assert!(limit > d, "the sight limit {limit} reaches the runner {d} cells off ({:?} {:?})", h.self_exposure, h.target_exposure);
        assert_eq!(h.state, State::Chase, "the lit runner is seen");
    }

    /// The same two spots with the building lights gone: the darkened runner is harder to see and is not seen.
    #[test]
    fn a_darkened_runner_is_still_harder_to_see() {
        let (mut g, content) = game();
        let (hunter, runner) = lit_room(&g, &content);
        // the lamps out, but the floor keeps one far away so the light data is there and only this spot is dark
        let far = g.world.lamps[0];
        g.world.lamps.clear();
        let mut away = far;
        away.pos.x = runner.0 + 400.0;
        g.world.lamps.push(away);
        put(&mut g, &content, hunter, runner);
        frames(&mut g, 6);
        let h = &g.hunters[0];
        assert!(h.target_exposure.is_some_and(|e| e < VISIBLE_EXPOSURE), "the runner stands in the dark: {:?}", h.target_exposure);
        assert_ne!(h.state, State::Chase, "the dark runner is not seen at that range");
    }

    /// With the rule off (the admin switch, or a guest) nothing is fed to the brain: the old flat scale.
    #[test]
    fn a_guest_feeds_no_exposures() {
        let (mut g, content) = game();
        let (hunter, runner) = lit_room(&g, &content);
        put(&mut g, &content, hunter, runner);
        g.remote_hunters = true;
        let mut frame = super::LightFrame::default();
        let t = g.target();
        g.refresh_light_sight(0, 1.0, &t, &mut frame);
        assert_eq!((g.hunters[0].self_exposure, g.hunters[0].target_exposure), (None, None));
    }

    /// A floor with no lamp data has no light rule either (the flat scale), rather than reading as pitch black.
    #[test]
    fn a_floor_with_no_lamps_keeps_the_flat_scale() {
        let (mut g, content) = game();
        let (hunter, runner) = lit_room(&g, &content);
        put(&mut g, &content, hunter, runner);
        g.world.lamps.clear();
        let mut frame = super::LightFrame::default();
        let t = g.target();
        g.refresh_light_sight(0, 1.0, &t, &mut frame);
        assert_eq!((g.hunters[0].self_exposure, g.hunters[0].target_exposure), (None, None));
    }

    /// Calibration against the render: inside the inner half of a shipped room lamp's reach, where the world shader
    /// lights a body well (its falloff is quadratic, so half the range still gives a quarter of the energy), the floor
    /// counts as lit enough for the player to make a hunter out, and so for the hunter to make out the runner.
    #[test]
    fn the_inner_half_of_a_real_room_lamp_counts_as_visible() {
        let (g, _) = game();
        assert!(!g.fac.room_lights.is_empty());
        let amb = g.light_ambient();
        for (i, l) in g.fac.room_lights.iter().enumerate() {
            let lamp = g.world.lamps[i];
            let r = lamp.range * 0.5;
            let dy = lamp.pos.y - 1.2;
            let flat = (r * r - dy * dy).max(0.0).sqrt();
            let e = light_field::exposure_at(
                [lamp.pos.x + flat, 1.2, lamp.pos.z],
                amb,
                &g.light_sources(),
                |_, _| true,
            );
            assert!(e >= VISIBLE_EXPOSURE, "room light {i} ({:?}) gives {e} at half its range", l.center);
        }
    }

    /// The exposures are refreshed on a cadence of about 0.2 s, not every frame.
    #[test]
    fn the_exposures_refresh_about_five_times_a_second() {
        let (mut g, content) = game();
        let (hunter, runner) = lit_room(&g, &content);
        put(&mut g, &content, hunter, runner);
        let t = g.target();
        let mut frame = super::LightFrame::default();
        g.refresh_light_sight(0, 1.0 / 60.0, &t, &mut frame);
        let first = g.hunters[0].target_exposure;
        assert!(first.is_some());
        // the runner steps into the dark between refreshes: the brain keeps the last answer until the next is due
        g.world.lamps.iter_mut().for_each(|l| l.pos.x += 400.0);
        let mut frame = super::LightFrame::default();
        for _ in 0..5 {
            g.refresh_light_sight(0, 1.0 / 60.0, &t, &mut frame);
        }
        assert_eq!(g.hunters[0].target_exposure, first, "not due yet");
        for _ in 0..12 {
            g.refresh_light_sight(0, 1.0 / 60.0, &t, &mut frame);
        }
        assert!(g.hunters[0].target_exposure < first, "refreshed once the interval has passed");
    }
}
