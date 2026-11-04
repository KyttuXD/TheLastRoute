//! Small glue for the leftover wiring hooks (see the hook list in the Integrate notes): which emote rule a hunter
//! state change fires, and the NPC to `BotView` mapping `draw_bots` takes. Behaviour only; every number is data.

use super::runner_figure::BotView;
use crate::game::npc::Npc;

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
        .map(|n| BotView { name: &n.name, at: n.at, facing: n.facing, stride: n.stride, walking: n.walking })
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

    #[test]
    fn no_npcs_no_views() {
        assert!(bot_views(&[], 0).is_empty());
    }
}
