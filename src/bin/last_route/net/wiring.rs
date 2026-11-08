//! Small glue for the leftover wiring hooks (see the hook list in the Integrate notes): which emote rule a hunter
//! state change fires, and the NPC to `BotView` mapping `draw_bots` takes. Behaviour only; every number is data.

use super::runner_figure::BotView;
use super::world_sync::RemoteView;
use crate::game::npc::Npc;
use serde_json::{json, Map, Value};

/// Adds the guest's place to an event the host takes from the net session, so the host game can resolve it: `x`, `y`
/// (this window's cells) and `yaw` of the runner whose `peer_id` the event carries, from `remotes` (the host's
/// `NetSession::remotes_at` for its own floor). A guest on another floor, or one with no fresh sample, is not in
/// `remotes`, so the event stays unplaced and the game ignores a taser or an escape it cannot place.
pub fn place_guest_event(ev: &mut Map<String, Value>, remotes: &[RemoteView]) {
    let Some(peer) = ev.get("peer_id").and_then(Value::as_u64) else { return };
    if let Some(r) = remotes.iter().find(|r| u64::from(r.peer) == peer) {
        // a place the host recorded itself (an escape's capture place, set by the net layer) is kept
        if !(ev.contains_key("x") && ev.contains_key("y")) {
            ev.insert("x".into(), json!(r.x));
            ev.insert("y".into(), json!(r.y));
        }
        ev.insert("yaw".into(), json!(r.yaw));
    }
}

/// The emote trigger rule a hunter state change fires (Godot `hunter_emoter.gd`): CHASE begun is `spot`, CHASE ended is
/// `lose`, SEARCH begun is `search` (the brain then holds it for the rule's `min_seconds`). Other changes fire nothing.
/// Names are the wire state names (`IDLE`, `WANDER`, `INVESTIGATE`, `CHASE`, `SEARCH`, `PEEK`).
pub fn transition_rule(prev: &str, next: &str) -> Option<&'static str> {
    if prev == next {
        return None;
    }
    match (prev, next) {
        (_, "CHASE") => Some("spot"),
        ("CHASE", _) => Some("lose"),
        (_, "SEARCH") => Some("search"),
        _ => None,
    }
}

/// The NPCs on `level` as the views `draw_bots` takes (the old scene.rs path drew them with `body.build`).
pub fn bot_views(npcs: &[Npc], level: i32) -> Vec<BotView<'_>> {
    npcs.iter()
        .filter(|n| n.level == level)
        .map(|n| BotView { name: &n.name, at: n.at, facing: n.facing, stride: n.stride, walking: n.walking, y: 0.0, climbing: false, falling: false })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_map_to_rules() {
        assert_eq!(transition_rule("WANDER", "CHASE"), Some("spot"));
        assert_eq!(transition_rule("CHASE", "SEARCH"), Some("lose"));
        assert_eq!(transition_rule("IDLE", "SEARCH"), Some("search"));
        assert_eq!(transition_rule("CHASE", "CHASE"), None);
        assert_eq!(transition_rule("IDLE", "WANDER"), None);
    }

    #[test]
    fn rules_exist_in_catalogue() {
        let cat = crate::hunters::emotes::Catalogue::get();
        for r in ["spot", "lose", "search"] {
            assert!(cat.rule(r).is_some(), "{r}");
        }
    }

    fn remote(peer: u32) -> RemoteView {
        RemoteView { peer, name: String::new(), x: 3.5, y: 4.5, yaw: 1.25, h: 0.0, level: 0, skin: String::new(), moving: false, sprinting: false, crouching: false, slim: false, speed: 0.0, vy: 0.0, clip: None, held: None, flash_pitch: None, look_pitch: 0.0, hide: None, hide_since: None }
    }

    #[test]
    fn a_guest_event_is_placed_from_that_guests_sample() {
        let mut ev = Map::new();
        ev.insert("what".into(), json!("taser"));
        ev.insert("peer_id".into(), json!(3));
        place_guest_event(&mut ev, &[remote(2), remote(3)]);
        assert_eq!((ev["x"].as_f64(), ev["y"].as_f64(), ev["yaw"].as_f64()), (Some(3.5), Some(4.5), Some(1.25)));
    }

    #[test]
    fn a_place_the_host_recorded_is_not_replaced_by_the_guests_sample() {
        let mut ev = Map::new();
        ev.insert("peer_id".into(), json!(2));
        ev.insert("x".into(), json!(7.5));
        ev.insert("y".into(), json!(8.5));
        place_guest_event(&mut ev, &[remote(2)]);
        assert_eq!((ev["x"].as_f64(), ev["y"].as_f64()), (Some(7.5), Some(8.5)));
    }

    #[test]
    fn an_event_from_a_guest_on_another_floor_or_with_no_peer_stays_unplaced() {
        let mut ev = Map::new();
        ev.insert("peer_id".into(), json!(9));
        place_guest_event(&mut ev, &[remote(2)]);
        assert!(!ev.contains_key("x"));
        let mut anon = Map::new();
        place_guest_event(&mut anon, &[remote(2)]);
        assert!(anon.is_empty());
    }

    #[test]
    fn no_npcs_no_views() {
        assert!(bot_views(&[], 0).is_empty());
    }
}
