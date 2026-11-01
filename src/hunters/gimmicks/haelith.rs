//! Haelith: relay pursuer. She camps the nearest uncollected relay (leaving the first one to
//! Opalie when more than one is open) and never stays in a chase (haelith_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{dist, Brain, State};

pub fn after(b: &mut Brain, c: &Ctx) {
    let open: Vec<usize> = (0..c.relays_taken.len())
        .filter(|i| !c.relays_taken[*i])
        .collect();
    let mut pursuit = open.clone();
    if open.len() > 1 {
        pursuit.retain(|i| *i != open[0]);
    }
    let best = pursuit
        .iter()
        .map(|i| c.fac.relays[*i].center())
        .min_by(|p, q| dist(*p, b.pos).partial_cmp(&dist(*q, b.pos)).unwrap());
    if let Some(r) = best {
        b.state = State::Investigate;
        b.last_known = Some(r);
    }
}
