//! Locks in the HUD danger meter (`Game::danger()`, Infinite only). The meter is a ROUTE BASE by
//! depth/difficulty (`Difficulty::route_danger`, from `data/danger.json`) that a nearby alert/chasing
//! hunter SPIKES toward 100%: `danger() = min(1, base + (1-base)*spike)`. So the meter is never empty
//! on a normal run (it reads at least `base_min` from the route), it differs by difficulty and depth,
//! and a point-blank chaser still pins it to exactly 1.0 on every difficulty. On the fixed route the
//! base is 0, so `danger()` there is the pure hunter spike (and the HUD hides the meter outside Infinite).

use super::Game;
use crate::config::Difficulty;
use crate::content::Content;
use crate::hunters::brain::{Brain, State};
use crate::world::catalog::Catalog;

/// The first hunter definition whose fresh `Brain` is not hidden from view, so
/// `danger()` will count it. (Void hunters start out of view and must not be picked.)
fn visible_def(content: &Content) -> crate::content::hunters::HunterDef {
    content
        .hunters
        .iter()
        .find(|d| {
            !Brain::new((*d).clone(), (0.0, 0.0), "DANGER-TEST", Difficulty::Standard)
                .gimmick
                .hidden_from_view()
        })
        .expect("at least one hunter def is visible on spawn")
        .clone()
}

/// A visible hunter at `pos`, active and in the given state.
fn hunter_at(content: &Content, pos: (f32, f32), state: State) -> Brain {
    let mut b = Brain::new(visible_def(content), pos, "DANGER-TEST", Difficulty::Standard);
    b.active = true;
    b.state = state;
    b
}

/// A bare Standard level-0 run with no hunters loaded (fresh arrival floor, relays not banked).
fn empty_run() -> (Game, Content, Catalog) {
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "DANGER-TEST-7", 0, &content, &catalog);
    g.hunters.clear();
    (g, content, catalog)
}

#[test]
fn danger_shows_the_route_base_with_no_hunters() {
    let (g, _c, _cat) = empty_run();
    assert!(g.hunters.is_empty());
    // No hunter means no spike, so the meter reads exactly the Infinite route base: never empty.
    assert!(g.danger() > 0.0, "an empty Infinite window still shows the route base");
    assert_eq!(g.danger(), g.difficulty.route_danger(g.depth), "with no spike the meter is the route base");
    assert!(g.danger() >= crate::config::difficulty::meter().base_min, "the base never drops below base_min");
}

#[test]
fn a_chasing_hunter_on_top_of_the_runner_pins_the_meter() {
    let (mut g, content, _cat) = empty_run();
    // distance 0, State::Chase: base 0.75 + proximity 0.36 = 1.11, capped to 1.0 (100%).
    g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
    assert_eq!(g.danger(), 1.0, "a hunter in Chase at zero distance pins the meter at 100%");
}

#[test]
fn the_meter_ignores_bot_targets_shadows_and_the_inactive() {
    let (mut g, content, _cat) = empty_run();
    // the route base with no contributing hunter: an excluded hunter must add no spike over this.
    let base = g.danger();
    assert_eq!(base, g.difficulty.route_danger(g.depth));

    g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
    assert_eq!(g.danger(), 1.0, "a contributing point-blank chaser pins the meter");

    // a hunter hunting a bot instead of the runner is not the runner's danger
    g.hunters[0].target_is_bot = true;
    assert_eq!(g.danger(), base, "a bot-targeting hunter adds no spike over the route base");
    g.hunters[0].target_is_bot = false;

    // a shadow (a guest's remote echo) draws but never counts toward local danger
    g.hunters[0].shadow = true;
    assert_eq!(g.danger(), base, "a shadow hunter adds no spike over the route base");
    g.hunters[0].shadow = false;

    // an inactive (unstreamed / asleep) hunter does not count
    g.hunters[0].active = false;
    assert_eq!(g.danger(), base, "an inactive hunter adds no spike over the route base");
}

#[test]
fn the_route_base_differs_by_difficulty_while_a_point_blank_chaser_still_pins_every_one() {
    // The route base now DOES read danger.json's depth rows, so it must differ by difficulty and depth;
    // but the spike saturates, so a Chase hunter at zero distance still reads exactly 1.0 everywhere.
    assert!(
        Difficulty::Nightmare.route_danger(12) > Difficulty::Standard.route_danger(12),
        "a harder difficulty reads a higher route base at the same depth"
    );
    assert!(
        Difficulty::Standard.route_danger(20) > Difficulty::Standard.route_danger(1),
        "a deeper floor reads a higher route base"
    );

    let content = Content::load();
    let catalog = Catalog::load();
    for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
        let mut g = Game::new_infinite_sim(diff, 7, "DANGER-TEST-7", 0, &content, &catalog);
        g.hunters.clear();
        g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
        assert_eq!(g.danger(), 1.0, "a point-blank chaser pins the meter to 100% on {}", diff.id());
    }
}

#[test]
fn a_fresh_standard_floor_shows_the_route_base_not_empty() {
    // On the fresh arrival floor the residency streams ZERO hunters until the activation relay is
    // banked, yet the meter is no longer empty: it shows the Infinite route base (>= base_min).
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "DANGER-TEST-7", 0, &content, &catalog);
    assert_eq!(g.relays_collected, 0, "a fresh run has banked no relays");
    assert!(g.hunters.is_empty(), "no hunters are streamed before the activation relay");
    assert!(
        g.danger() >= crate::config::difficulty::meter().base_min,
        "a pre-activation floor still shows the route base, never an empty meter"
    );
    assert_eq!(g.danger(), g.difficulty.route_danger(g.depth), "with no hunter the meter is the route base");

    // Once the activation relay is banked the residency admits hunters; a loaded hunter within range
    // in Chase spikes the meter well past the base.
    g.relays_collected = 1;
    g.ensure_hunters(&content);
    assert!(!g.hunters.is_empty(), "hunters are admitted once the activation relay is banked");
    // place a loaded hunter beside the runner, chasing, to show the spike responds
    g.hunters.push(hunter_at(&content, (g.p.pos.0 + 1.0, g.p.pos.1), State::Chase));
    assert!(g.danger() > 0.75, "a near chasing hunter spikes the meter past the Chase floor");
    // and at zero distance the spike pins it to exactly 100%, whatever the base.
    g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
    assert_eq!(g.danger(), 1.0, "a point-blank chaser still pins the meter to 100%");
}

// ---- the chase cue against an admin god-mode runner (audio/world.rs `chase_cue_frame`, hunting.rs `hold_refused_capture`) ----

/// The hunters as the sound sees them, built the way `Game::update` builds them.
fn audio_list(g: &Game) -> Vec<crate::audio::world::HunterAudio<'_>> {
    g.hunters
        .iter()
        .map(|h| crate::audio::world::HunterAudio {
            id: &h.def.id,
            pos: h.pos,
            active: h.active,
            chasing: h.state == State::Chase,
            idle: h.state == State::Idle,
            direct_visible: h.direct_visible,
            winded: h.winded,
            stunned: h.stun > 0.0,
            in_vent: false,
            nyxie_empty_wake: false,
            glyra_humming: false,
            targets_runner: !h.target_is_bot && !h.shadow,
        })
        .collect()
}

/// A finite-route run with one plain chaser standing a step from an unsheltered runner, in clear sight.
fn runner_with_a_chaser_in_reach() -> Game {
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
    g.hunters.clear();
    let spots: Vec<(f32, f32)> = g.fac.room_lights.iter().map(|l| l.center.center()).collect();
    for runner in spots {
        g.p.pos = runner;
        if g.in_safe_zone() {
            continue;
        }
        let near = g.fac.floor_cells().into_iter().map(|c| (c, c.center())).find(|(c, at)| {
            let d = super::pdist(*at, runner);
            d > 0.9
                && d < 1.6
                && g.fac.hunter_open(*c)
                && g.fac.body_fits(at.0, at.1, crate::config::HUNTER_RADIUS)
                && g.fac.has_line_of_sight(*at, runner)
        });
        if let Some((_, at)) = near {
            let mut h = hunter_at(&content, at, State::Chase);
            h.gimmick = crate::hunters::gimmicks::Gimmick::None;
            h.chase(runner);
            g.hunters.push(h);
            return g;
        }
    }
    panic!("no unsheltered room spot with an open cell beside it");
}

/// The reported bug: with admin GOD MODE on, a hunter that reaches the runner cannot capture, so it walked onto the
/// runner, "arrived" (Chase -> Search), saw the runner again at the next sense tick (Search -> Chase), and every such
/// re-acquire played chase_start. Ten simulated seconds of that contact play the cue at most once, the hunter's chase
/// does not flap, and nothing is captured.
#[test]
fn a_hunter_colliding_with_a_god_mode_runner_for_ten_seconds_plays_the_chase_cue_at_most_once() {
    let mut g = runner_with_a_chaser_in_reach();
    g.admin.god = true;
    let prof = g.prof();
    let cue = crate::audio::cues::rules::ChaseCueRules::get();
    let mut gate = crate::audio::cues::rules::ChaseCueGate::default();
    let (mut fired, mut left_chase, mut chase_frames) = (0, 0, 0);
    let mut was_chasing = g.hunters[0].state == State::Chase;
    for _ in 0..600 {
        g.update_hunters(1.0 / 60.0, &prof, &crate::audio::Sounds::default());
        let chasing = g.hunters[0].state == State::Chase;
        if was_chasing && !chasing {
            left_chase += 1;
        }
        if chasing {
            chase_frames += 1;
        }
        was_chasing = chasing;
        let immune = g.admin_flags().god;
        let list = audio_list(&g);
        if crate::audio::world::chase_cue_frame(&mut gate, &cue, 1.0 / 60.0, g.p.pos, &list, immune).2 {
            fired += 1;
        }
    }
    assert!(g.escape.is_none() && g.report.is_none(), "god mode: no capture and no run end");
    assert!(fired <= 1, "the chase cue played {fired} times in ten seconds of contact");
    assert!(chase_frames > 300, "the hunter keeps chasing the runner it cannot capture ({chase_frames} of 600 frames)");
    assert!(left_chase <= 5, "the chase state flapped {left_chase} times in ten seconds of contact");
}

/// A hunter that loses the runner and finds it again inside the cooldown is the same chase (no second cue); one that
/// starts chasing after the cooldown is a new chase and plays it again. Normal runner, no admin switch.
#[test]
fn a_chase_regained_inside_the_cooldown_is_silent_and_a_new_chase_after_it_plays() {
    let (mut g, content, _cat) = empty_run();
    g.hunters.push(hunter_at(&content, (g.p.pos.0 + 6.0, g.p.pos.1), State::Chase));
    let cue = crate::audio::cues::rules::ChaseCueRules::get();
    let mut gate = crate::audio::cues::rules::ChaseCueGate::default();
    // `seconds` of frames with the hunter in `state`; how many times the cue played
    let mut run = |g: &mut Game, state: State, seconds: f64| -> usize {
        g.hunters[0].state = state;
        let list = audio_list(g);
        (0..(seconds * 60.0).round() as usize)
            .filter(|_| crate::audio::world::chase_cue_frame(&mut gate, &cue, 1.0 / 60.0, g.p.pos, &list, false).2)
            .count()
    };
    assert_eq!(run(&mut g, State::Chase, 2.0), 1, "the chase starts: one cue");
    assert_eq!(run(&mut g, State::Search, cue.rearm_seconds * 0.5), 0);
    assert_eq!(run(&mut g, State::Chase, 2.0), 0, "the same target regained inside the cooldown: no cue");
    assert_eq!(run(&mut g, State::Search, cue.rearm_seconds + 0.5), 0);
    assert_eq!(run(&mut g, State::Chase, 2.0), 1, "a new chase after the cooldown plays the cue");
}
