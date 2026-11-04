//! Guest gate for the hunter shelf in game/resident_hunters.rs.
//!
//! A guest runs no hunter simulation: the host's frames set every hunter (hunting.rs `apply_hunter_frame`), so the
//! guest's hunters are display only. The residency pass and the shelve / unshelve around a floor rebuild would
//! otherwise admit, store or respawn hunters on the guest and fight the host's frames. `is_display_only` is the one
//! test.
//!
//! Hooks (each is the first line of the function body):
//!   residency (game/endless.rs)  `if super::guest_gate::is_display_only(self) { return; }`
//!   shelve_active_hunters, unshelve_active_hunters, store_slots, pull_slots (game/resident_hunters.rs) ask the same.
//! and in game/mod.rs next to the other `mod` lines: `mod guest_gate;`.
//! Bots and chased fake players never reach the player's danger readout, heartbeat or chase cue from here.

use super::Game;

/// True when this game only shows the hunters the host sends (a guest), so it must not store, restore or admit any.
pub(super) fn is_display_only(game: &Game) -> bool {
    game.remote_hunters
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn run() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-GATE-7", 0, &content, &catalog);
        (g, content, catalog)
    }

    #[test]
    fn a_host_or_solo_game_is_not_display_only() {
        let (g, _, _) = run();
        assert!(!is_display_only(&g));
    }

    #[test]
    fn a_guest_is_display_only() {
        let (mut g, _, _) = run();
        g.remote_hunters = true;
        assert!(is_display_only(&g));
    }

    #[test]
    fn the_gate_follows_the_role_each_frame() {
        let (mut g, _, _) = run();
        g.remote_hunters = true;
        g.remote_hunters = false;
        assert!(!is_display_only(&g));
    }
}
