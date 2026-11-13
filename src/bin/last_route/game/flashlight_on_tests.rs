//! Play with the flashlight ON, where a headless suite case (tests/flashlight_on.json) cannot set the scene: the
//! battery, what a lit runner costs in hunter sight, and the seat and the bed (both put the light away).

use super::{pdist, Controls, Game};
use crate::audio::Sounds;
use crate::config::{Difficulty, HUNTER_RADIUS, LAMP_IDLE_DRAIN, MAX_LAMP_CHARGE};
use crate::content::Content;
use crate::hunters::brain::{Brain, State};
use crate::hunters::gimmicks::Gimmick;
use crate::world::catalog::Catalog;

const DT: f32 = 1.0 / 60.0;

fn step(g: &mut Game, c: &Controls, frames: usize, content: &Content, catalog: &Catalog) {
    for _ in 0..frames {
        g.step(DT, c, content, catalog, &Sounds::default());
    }
}

/// The finite floor of tests/meters.json (seed 12345), with the hunters told to leave the runner alone.
fn quiet_run() -> (Game, Content, Catalog) {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_sim(Difficulty::Standard, 12345, &content, &catalog);
    g.admin.no_target = true;
    (g, content, catalog)
}

#[test]
fn the_battery_drains_at_the_idle_rate_while_lit_and_stops_at_empty() {
    let (mut g, content, catalog) = quiet_run();
    let still = Controls::default();
    // [F] switches the light on (the runner starts with a flashlight in the first slot)
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    assert!(g.p.flashlight_on, "[F] switches the flashlight on");
    assert_eq!(g.selected_item(), Some(super::Item::Flashlight));
    let before = g.p.lamp;
    step(&mut g, &still, 60, &content, &catalog);
    let used = before - g.p.lamp;
    let want = LAMP_IDLE_DRAIN * g.prof().lamp_drain;
    assert!((used - want).abs() < 0.25, "one second lit used {used} of the battery, the idle drain is {want}");
    // an unlit second uses nothing
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    assert!(!g.p.flashlight_on);
    let off = g.p.lamp;
    step(&mut g, &still, 60, &content, &catalog);
    assert_eq!(g.p.lamp, off, "the battery does not drain with the light off");
    // the last of the battery: the light dies at exactly zero and stays there
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    assert!(g.p.flashlight_on);
    g.p.lamp = 0.05;
    step(&mut g, &still, 30, &content, &catalog);
    assert!(!g.p.flashlight_on, "an empty battery switches the light off");
    assert_eq!(g.p.lamp, 0.0, "the battery stops at empty");
    step(&mut g, &still, 60, &content, &catalog);
    assert_eq!(g.p.lamp, 0.0, "and never goes below it");
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    assert!(!g.p.flashlight_on, "an empty flashlight only clicks");
}

#[test]
fn every_hunter_sees_a_lit_runner_from_farther_than_an_unlit_one() {
    let content = Content::load();
    for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
        let prof = diff.profile();
        for def in content.hunters.iter() {
            let b = Brain::new(def.clone(), (0.0, 0.0), "LIGHT-TEST", diff);
            let (lit, unlit) = (b.sight_limit(true, &prof, &b.gimmick), b.sight_limit(false, &prof, &b.gimmick));
            assert!(lit >= unlit, "{} on {}: lit {lit} unlit {unlit}", def.id, diff.id());
            if b.gimmick.sight_scale() > 0.0 {
                assert!(lit > unlit, "{} on {}: a lit runner is seen from farther ({lit} against {unlit})", def.id, diff.id());
            }
        }
    }
}

/// A hunter with a clear line to the runner, farther off than it sees an unlit runner and well inside the range it
/// sees a lit one: it does not notice the runner with the flashlight off and does with it on.
#[test]
fn a_hunter_notices_a_lit_runner_at_a_range_where_it_misses_an_unlit_one() {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
    g.hunters.clear();
    // no lamp data on the floor: the light rule is off and an unlit runner is seen at the flat scale
    g.world.lamps.clear();
    let prof = g.prof();
    let mut probe = Brain::new(content.hunters[0].clone(), (0.0, 0.0), "LIGHT-TEST", Difficulty::Standard);
    probe.gimmick = Gimmick::None;
    let unlit = probe.sight_limit(false, &prof, &probe.gimmick);
    let lit = probe.sight_limit(true, &prof, &probe.gimmick);
    assert!(lit > unlit + 2.5, "a window between unlit ({unlit}) and lit ({lit}) sight");
    let cells = g.fac.floor_cells();
    let mut found = None;
    'search: for rc in cells.iter() {
        let runner = rc.center();
        g.p.pos = runner;
        if g.in_safe_zone() {
            continue;
        }
        for hc in cells.iter() {
            let at = hc.center();
            let d = pdist(at, runner);
            if d > unlit + 1.2
                && d < lit - 1.2
                && g.fac.hunter_open(*hc)
                && g.fac.body_fits(at.0, at.1, HUNTER_RADIUS)
                && g.fac.has_line_of_sight(at, runner)
            {
                found = Some((at, runner));
                break 'search;
            }
        }
    }
    let (spot, runner) = found.expect("two cells in clear sight of each other past the unlit range");
    g.p.pos = runner;
    g.p.lamp = MAX_LAMP_CHARGE;
    let watch = |g: &mut Game, flashlight: bool| -> State {
        g.p.flashlight_on = flashlight;
        let mut h = Brain::new(content.hunters[0].clone(), spot, "LIGHT-TEST", Difficulty::Standard);
        h.gimmick = Gimmick::None;
        h.active = true;
        h.state = State::Wander;
        g.hunters.clear();
        g.hunters.push(h);
        for _ in 0..30 {
            // held on its spot: only what it senses from there is under test
            g.hunters[0].pos = spot;
            g.update_hunters(DT, &prof, &Sounds::default());
        }
        g.hunters[0].state
    };
    assert_ne!(watch(&mut g, false), State::Chase, "the unlit runner is out of sight at that range");
    assert_eq!(watch(&mut g, true), State::Chase, "the lit runner is seen");
}

/// A seat (or a bed) 0.8 m in front of the runner, as game/bed.rs sets its own tests up; the prop's index.
fn furniture(g: &mut Game, ptype: &str, half: (f32, f32)) -> usize {
    let cell = crate::world::Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
    g.p.pos = cell.center();
    let mut p = g.fac.props.first().cloned().expect("the floor has a prop to copy");
    p.id = "TEST:REST".to_string();
    p.cell = cell;
    p.ptype = ptype.to_string();
    p.rotation = 0;
    p.mount_y = None;
    p.solid = false;
    p.half = half;
    p.offset = (0.0, 0.8);
    p.door = None;
    p.sewer = None;
    p.crawl_under = false;
    g.fac.props.push(p);
    g.p.yaw = std::f32::consts::PI;
    g.p.stun = 0.0;
    g.fac.props.len() - 1
}

fn rest_run() -> (Game, Content, Catalog) {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-LIGHT-7", 0, &content, &catalog);
    g.ensure_hunters(&content);
    for h in g.hunters.iter_mut() {
        h.active = false;
    }
    g.p.lamp = MAX_LAMP_CHARGE;
    g.p.flashlight_on = true;
    (g, content, catalog)
}

#[test]
fn sitting_down_puts_the_light_away_and_it_stays_off_while_seated() {
    let (mut g, content, catalog) = rest_run();
    let i = furniture(&mut g, "officeChair", (0.4, 0.4));
    assert!(g.p.flashlight_on);
    assert!(g.sit_down(i), "the seat is in reach");
    assert!(g.rest.sitting());
    assert!(!g.p.flashlight_on, "a seated runner holds no light");
    let lamp = g.p.lamp;
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    step(&mut g, &Controls::default(), 30, &content, &catalog);
    assert!(!g.p.flashlight_on, "[F] does nothing on a seat");
    assert_eq!(g.p.lamp, lamp, "no battery is used while seated");
}

#[test]
fn lying_down_and_sleeping_put_the_light_away() {
    let (mut g, content, catalog) = rest_run();
    let i = furniture(&mut g, "singleBed", (0.5, 1.0));
    g.meters.drowsiness = 50.0;
    assert!(g.lie_down(i), "the bed is in reach");
    assert!(g.rest.lying());
    assert!(!g.p.flashlight_on, "a lying runner holds no light");
    let lamp = g.p.lamp;
    step(&mut g, &Controls { flashlight: true, ..Controls::default() }, 1, &content, &catalog);
    assert!(!g.p.flashlight_on, "[F] does nothing on a bed");
    // jump falls asleep (game/bed.rs); the light stays off through the sleep
    step(&mut g, &Controls { jump: true, ..Controls::default() }, 1, &content, &catalog);
    step(&mut g, &Controls::default(), 60, &content, &catalog);
    assert!(!g.p.flashlight_on, "no light while resting on the bed");
    assert_eq!(g.p.lamp, lamp, "no battery is used on the bed");
}
