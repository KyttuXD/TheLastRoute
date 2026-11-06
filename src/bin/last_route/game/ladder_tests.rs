//! The sewer ladders (`world::level_links`) in a running Infinite game: down from the maze and back up.

use super::*;
use crate::config::Difficulty;
use crate::content::Content;
use crate::world::catalog::Catalog;
use crate::world::facility::{Cover, LinkKind};

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
