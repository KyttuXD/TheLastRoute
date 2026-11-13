//! Hunters: the shared brain, each hunter's gimmick, and the per-floor lineup.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod activities;
pub mod idle_life;
pub mod brain;
pub mod director;
pub mod emotes;
pub mod flight_edges;
pub mod gimmicks;
pub mod hard_abilities;
pub mod light_field;
pub mod mechanics;
pub mod naps;
pub mod navigation;
pub mod locker_inspect;
pub mod patrol_anchor;
pub mod noise_events;
pub mod path_scheduler;
pub mod peek;
pub mod residency;
pub mod sabotage;
pub mod secret_creatures;
pub mod target_filter;
pub mod vent_nav;
pub mod water;

#[cfg(test)]
mod bulk_bench;
#[cfg(test)]
mod collision_tests;

use crate::config::{tier_weights_for, Difficulty};
use crate::content::Content;
use crate::world::rng::Rng;

/// HunterRoster.pick_for_floor: one tier-weighted draw from the pool on
/// "<level seed>:HUNTER-LINEUP:D<depth>". Tiers are walked in sorted order
/// (difficult, easy, intermediate), then one index inside the tier.
pub fn pick_for_floor(content: &Content, level_seed: &str, depth: i32, diff: Difficulty) -> usize {
    pick_for_floor_weighted(content, level_seed, depth, diff.tier_weights())
}

/// pick_for_floor by difficulty id, so "nightmare" draws with its own tier weights
/// (TIER_WEIGHTS_BY_DIFFICULTY.get(difficulty_id, standard)).
pub fn pick_for_floor_id(content: &Content, level_seed: &str, depth: i32, difficulty_id: &str) -> usize {
    pick_for_floor_weighted(content, level_seed, depth, tier_weights_for(difficulty_id))
}

fn pick_for_floor_weighted(content: &Content, level_seed: &str, depth: i32, w: [f64; 3]) -> usize {
    let pool = content.draw_pool();
    let mut rng = Rng::new(&format!("{level_seed}:HUNTER-LINEUP:D{depth}"));
    // sorted tier names: difficult(2), easy(0), intermediate(1)
    let order = [(2usize, w[2]), (0, w[0]), (1, w[1])];
    let roll = rng.next_float();
    let mut acc = 0.0;
    let mut tier = order[order.len() - 1].0;
    for (t, weight) in order {
        acc += weight;
        if roll < acc {
            tier = t;
            break;
        }
    }
    let in_tier: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|i| content.hunters[*i].tier == tier)
        .collect();
    let list = if in_tier.is_empty() { pool } else { in_tier };
    list[rng.next_index(list.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NIGHTMARE_TIER_WEIGHTS;

    #[test]
    fn nightmare_weights_match_godot_roster() {
        assert_eq!(tier_weights_for("nightmare"), [0.175, 0.525, 0.30]);
        for d in crate::config::difficulty::ALL {
            assert_eq!(tier_weights_for(d.id()), d.tier_weights());
        }
        assert_eq!(tier_weights_for("unknown"), Difficulty::Standard.tier_weights());
        let sum: f64 = NIGHTMARE_TIER_WEIGHTS.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn draw_by_id_matches_draw_by_difficulty() {
        let content = Content::load();
        for d in crate::config::difficulty::ALL {
            for depth in 1..=12 {
                assert_eq!(
                    pick_for_floor_id(&content, "SEED", depth, d.id()),
                    pick_for_floor(&content, "SEED", depth, d)
                );
            }
        }
    }

    #[test]
    fn nightmare_floors_draw_difficult_more_often() {
        let content = Content::load();
        let difficult = |id: &str| {
            (1..=400)
                .filter(|depth| content.hunters[pick_for_floor_id(&content, "SEED", *depth, id)].tier == 2)
                .count()
        };
        assert!(difficult("nightmare") > difficult("hard"));
        assert!(difficult("hard") > difficult("standard"));
    }
}
