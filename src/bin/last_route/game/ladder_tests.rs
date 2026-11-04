//! The sewer ladders (`world::level_links`) in a running Infinite game: down from the maze and back up.

use super::*;
use crate::config::Difficulty;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::game::ladder::{facing, End};
use crate::world::facility::{Cell, Cover, LinkKind};
use crate::world::vertical::LADDER_CLIMB_SPEED;

#[test]
fn a_ladder_leads_down_to_the_sewer_and_back_up_to_the_same_shaft() {
    let content = Content::load();
    let catalog = Catalog::load();
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
    let i = g
        .fac
        .stairs
        .iter()
        .position(|s| s.is_ladder() && s.dir < 0)
        .expect("the start window has a ladder down (the origin chunk always does)");
    let top = g.fac.stairs[i];
    // the shaft is open at the top: not walkable, so the runner gets on from beside it
    assert!(g.fac.is_hole(top.cell.x, top.cell.y));
    g.focus = Some(Focus::Ladder(i));
    let prompt = g.focus_prompt().unwrap();
    match top.kind {
        LinkKind::Ladder(Cover::Manhole) => assert!(prompt.contains("MANHOLE"), "{prompt}"),
        LinkKind::Ladder(Cover::Grate) => assert!(prompt.contains("GRATE"), "{prompt}"),
        LinkKind::Stairs => unreachable!(),
    }

    g.go_to_level(g.inf.as_ref().unwrap().level + top.dir, top.cell.center(), 0.0, &content, &catalog);
    assert_eq!(g.inf.as_ref().unwrap().level, -1);
    let j = g
        .fac
        .stairs
        .iter()
        .position(|s| s.is_ladder() && s.dir > 0 && s.cell == top.cell)
        .expect("the sewer has the bottom of the same ladder");
    // the sewer floor stays under the ladder, and the runner stands at its foot
    assert!(!g.fac.is_hole(top.cell.x, top.cell.y));
    assert!(g.fac.body_fits(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS));
    assert!(pdist(g.p.pos, top.cell.center()) < 2.0);
    g.focus = Some(Focus::Ladder(j));
    assert_eq!(g.focus_prompt().as_deref(), Some("[E] CLIMB THE LADDER UP"));

    g.go_to_level(0, top.cell.center(), 0.0, &content, &catalog);
    assert_eq!(g.inf.as_ref().unwrap().level, 0);
    assert!(g.fac.body_fits(g.p.pos.0, g.p.pos.1, PLAYER_RADIUS));
    assert!(pdist(g.p.pos, top.cell.center()) < 2.0, "back up beside the shaft");
}

/// A level 0 window with a catwalk link: the link, its deck cell and the run.
fn catwalk_game(content: &Content, catalog: &Catalog) -> (Game, crate::world::route::catwalk_links::CatwalkLink) {
    for seed in 1..60u32 {
        let g = Game::new_infinite_sim(Difficulty::Standard, seed, "ROOM-PROBE", 0, content, catalog);
        if let Some(l) = g.fac.catwalk_links.first().copied() {
            return (g, l);
        }
    }
    panic!("no catwalk link in seeds 1..60");
}

fn step_game(g: &mut Game, c: &crate::game::Controls, content: &Content, catalog: &Catalog, frames: usize) {
    for _ in 0..frames {
        g.step(1.0 / 60.0, c, content, catalog, &crate::audio::Sounds::default());
    }
}

#[test]
fn a_catwalk_ladder_climbs_to_the_deck_height_and_lands_on_the_deck_with_no_jump() {
    use crate::game::Controls;
    let content = Content::load();
    let catalog = Catalog::load();
    let (mut g, l) = catwalk_game(&content, &catalog);
    assert!(l.deck_height < 3.0 && crate::config::WALL_HEIGHT - l.deck_height >= 2.0, "headroom: deck {}", l.deck_height);
    let run = if l.run == (0, 0) { (1, 0) } else { l.run };
    let deck = Cell { x: l.cell.x + run.0, y: l.cell.y + run.1 };
    g.p.pos = l.cell.center();
    g.p.yaw = facing(run);
    g.p.set_abs_y(0.0);
    let fwd = Controls { fwd: 1.0, ..Controls::default() };
    let mut frames = 0;
    while g.ladder.is_none() {
        step_game(&mut g, &fwd, &content, &catalog, 1);
        frames += 1;
        assert!(frames < 90, "walking at the deck face never mounted; at {:?}", g.p.pos);
    }
    let k = g.ladder.unwrap();
    assert!((k.y_top - k.y_bottom - l.deck_height).abs() < 1e-4, "the shaft is the deck's height, not a storey");
    // climb: y rises smoothly, x and z do not move, and it ends on the deck
    let mut last = g.p.y();
    let mut guard = 0;
    while g.ladder.is_some() {
        let before = g.p.pos;
        step_game(&mut g, &fwd, &content, &catalog, 1);
        let y = g.p.y();
        assert!((y - last).abs() <= LADDER_CLIMB_SPEED / 60.0 + 0.02, "y step {last} -> {y}");
        last = y;
        assert!(pdist(g.p.pos, before) < 0.2, "no teleport: {before:?} -> {:?}", g.p.pos);
        guard += 1;
        assert!(guard < 1200, "never finished");
    }
    assert!((g.p.y() - l.deck_height).abs() < 1e-3, "standing on the deck: {}", g.p.y());
    assert!(pdist(g.p.pos, deck.center()) < 0.05, "on the deck cell: {:?} want {:?}", g.p.pos, deck.center());
    assert!(g.p.eye_y() + 0.2 < crate::config::WALL_HEIGHT, "the head clears the ceiling");
    // standing there with no key held, the body stays on the deck (no fall, no snap back)
    step_game(&mut g, &Controls::default(), &content, &catalog, 60);
    assert!((g.p.y() - l.deck_height).abs() < 1e-3 && g.ladder.is_none() && !g.p.falling() && g.p.grounded);
    // back toward the ladder from the deck: mounts at the top and climbs down to the ground
    g.p.yaw = facing((-run.0, -run.1));
    let mut frames = 0;
    while g.ladder.is_none() {
        step_game(&mut g, &fwd, &content, &catalog, 1);
        frames += 1;
        assert!(frames < 90, "walking at the ladder from the deck never mounted; at {:?}", g.p.pos);
    }
    assert_eq!(g.ladder.unwrap().end, End::Top);
    let back = Controls { fwd: -1.0, ..Controls::default() };
    let mut guard = 0;
    while g.ladder.is_some() {
        let before = g.p.pos;
        step_game(&mut g, &back, &content, &catalog, 1);
        assert!(pdist(g.p.pos, before) < 0.2, "no teleport going down: {before:?} -> {:?}", g.p.pos);
        guard += 1;
        assert!(guard < 1200, "never finished");
    }
    assert!(g.p.y().abs() < 1e-3, "back on the floor: {}", g.p.y());
    assert!(pdist(g.p.pos, l.cell.center()) < 0.05, "on the foot cell: {:?}", g.p.pos);
}
