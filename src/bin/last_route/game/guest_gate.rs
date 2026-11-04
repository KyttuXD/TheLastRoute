//! Guest gate for the level-change hunter storage in game/endless.rs.
//!
//! A guest runs no hunter simulation: the host's frames set every hunter (hunting.rs `apply_hunter_frame`), so the
//! guest's hunters are display only. The residency pass and the store / restore on a stairwell or ladder would
//! otherwise admit, store or respawn hunters on the guest and fight the host's frames. `is_display_only` is the one
//! test; the hooks list in the module doc of the integrator note says where endless.rs asks it.
//!
//! Hooks for game/endless.rs (each is the first line of the function body):
//!   residency            `if super::guest_gate::is_display_only(self) { return; }`
//!   store_level0_hunters `if super::guest_gate::is_display_only(self) { return; }`
//!   store_floor_hunters  `if super::guest_gate::is_display_only(self) { return; }`
//!   restore_level_hunters `if super::guest_gate::is_display_only(self) { return; }`
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
