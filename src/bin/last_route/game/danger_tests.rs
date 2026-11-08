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
