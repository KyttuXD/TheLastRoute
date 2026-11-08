//! Capture immunity (TODO 714): a runner in a state that cannot be caught is not caught, whatever the hunter does.

use super::revive::Downed;
use super::*;
use crate::config::Difficulty;
use crate::content::Content;
use crate::world::catalog::Catalog;

fn game() -> Game {
    let (content, catalog) = (Content::load(), Catalog::load());
    let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-IMMUNE-7", 0, &content, &catalog);
    g.ensure_hunters(&content);
    g.room_teammates = 1;
    g
}

#[test]
fn a_spectator_is_never_caught() {
    let mut g = game();
    g.downed = Some(Downed { remaining: 30.0, held: 0.0, hunter: 0 });
    assert!(g.spectate_enter(&[1, 2]), "a downed runner may watch");
    assert!(g.spectating());
    g.end_run(0, "contact");
    assert!(g.report.is_none(), "a spectator's run does not end");
}

#[test]
fn a_downed_runner_stays_down_and_a_further_catch_does_not_kill_it() {
    let mut g = game();
    g.downed = Some(Downed { remaining: 30.0, held: 0.0, hunter: 0 });
    assert!(g.go_down(0), "already down: the catch is absorbed");
    assert!(g.report.is_none());
    assert!(g.runner_downed());
}

#[test]
fn a_finished_run_is_never_taken_down_again() {
    let mut g = game();
    g.end_run(0, "contact");
    if g.report.is_some() {
        assert!(g.go_down(0));
        assert!(!g.runner_downed());
    }
}
