//! Hunters: the shared brain, each hunter's gimmick, and the per-floor lineup.

pub mod brain;
pub mod director;
pub mod gimmicks;
pub mod navigation;
pub mod residency;

use crate::config::Difficulty;
use crate::content::Content;
use crate::world::rng::Rng;

/// HunterRoster.pick_for_floor: one tier-weighted draw from the pool on
/// "<level seed>:HUNTER-LINEUP:D<depth>". Tiers are walked in sorted order
/// (difficult, easy, intermediate), then one index inside the tier.
pub fn pick_for_floor(content: &Content, level_seed: &str, depth: i32, diff: Difficulty) -> usize {
    let pool = content.draw_pool();
    let mut rng = Rng::new(&format!("{level_seed}:HUNTER-LINEUP:D{depth}"));
    let w = diff.tier_weights();
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
