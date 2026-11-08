//! Locks in the HUD danger meter's one and only source: `Game::danger()`, a live-hunter
//! proximity readout (ui/hud.rs reads nothing else). These tests exist because Task 4
//! reported the meter as "stuck/empty" and blamed the flattened Standard `data/danger.json`
//! or the concept-map level-0 change. Both are refuted here: the meter never reads
//! `danger.json`, a `DangerProfile`, or any level/region, and an empty meter on a fresh
//! Standard arrival floor is correct — zero hunters are streamed in until the first
//! activation relay is banked, so there is nothing within range to raise it.

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
fn danger_is_zero_with_no_hunters() {
    let (g, _c, _cat) = empty_run();
    assert!(g.hunters.is_empty());
    assert_eq!(g.danger(), 0.0, "an empty window reads no danger");
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
    g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
    assert_eq!(g.danger(), 1.0);

    // a hunter hunting a bot instead of the runner is not the runner's danger
    g.hunters[0].target_is_bot = true;
    assert_eq!(g.danger(), 0.0, "a bot-targeting hunter drops out of the meter");
    g.hunters[0].target_is_bot = false;

    // a shadow (a guest's remote echo) draws but never counts toward local danger
    g.hunters[0].shadow = true;
    assert_eq!(g.danger(), 0.0, "a shadow hunter drops out of the meter");
    g.hunters[0].shadow = false;

    // an inactive (unstreamed / asleep) hunter does not count
    g.hunters[0].active = false;
    assert_eq!(g.danger(), 0.0, "an inactive hunter drops out of the meter");
}

#[test]
fn danger_is_identical_across_difficulties_proving_it_ignores_danger_json() {
    // danger.json's depth rows feed only hunter speed_scale / extra_hunters (config/difficulty.rs),
    // never the meter. The same hunter at the same spot must read the same on every difficulty.
    let content = Content::load();
    let catalog = Catalog::load();
    let mut values = Vec::new();
    for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
        let mut g = Game::new_infinite_sim(diff, 7, "DANGER-TEST-7", 0, &content, &catalog);
        g.hunters.clear();
        g.hunters.push(hunter_at(&content, g.p.pos, State::Chase));
        values.push(g.danger());
    }
    assert!(
        values.iter().all(|v| (*v - values[0]).abs() < f32::EPSILON),
        "danger() must be identical across difficulties (independent of danger.json): {values:?}"
    );
    assert_eq!(values[0], 1.0);
}

#[test]
fn a_fresh_standard_floor_reads_empty_until_the_activation_relay() {
    // The reported "empty meter": on Standard the arrival floor streams ZERO hunters while
    // relays_collected < activation_relays (Standard activation_relays = 1), so danger() == 0.
    // This is working as coded, not a defect in the meter or in danger.json.
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "DANGER-TEST-7", 0, &content, &catalog);
    assert_eq!(g.relays_collected, 0, "a fresh run has banked no relays");
    assert!(g.hunters.is_empty(), "no hunters are streamed before the activation relay");
    assert_eq!(g.danger(), 0.0, "an empty pre-activation floor reads an empty meter (expected)");

    // Once the activation relay is banked the residency admits hunters; with a loaded hunter
    // within 16 cells in Chase the meter demonstrably moves off zero.
    g.relays_collected = 1;
    g.ensure_hunters(&content);
    assert!(!g.hunters.is_empty(), "hunters are admitted once the activation relay is banked");
    // place a loaded hunter beside the runner, chasing, to show the meter responds
    g.hunters.push(hunter_at(&content, (g.p.pos.0 + 1.0, g.p.pos.1), State::Chase));
    assert!(g.danger() > 0.75, "a near chasing hunter moves the meter above the Chase floor");
}
