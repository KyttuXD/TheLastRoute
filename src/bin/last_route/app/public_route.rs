//! PUBLIC route patrol around a live runner. Godot twin: game/hunters/ai/hunter_brain.gd (PATROL_RADIUS L142,
//! _pick_wander_target L1120-1126) and game/infinite_route/infinite_route_test.gd (PUBLIC_PATROL_DWELL_MSEC L5869,
//! _assign_public_patrol L5874-5905, _public_runner_positions L5907).
//!
//! Radius, fallback scale, dwell and speed come from data/public_roster.json "patrol" (content::public_roster::
//! PublicPatrol). This module holds only the decisions; the caller owns the hunters, the clock and the runners.
//! Idle hunters patrol around one runner for `dwell_msec`, then re-roll to the runner with the fewest hunters on it.
//! Callers (hooks): `JoinThrottle::try_join` before the client joins the public route (app/mod.rs), and
//! `PatrolBook::anchor` + `filter_candidates` + `patrol_speed` where the public room's host picks an idle hunter's
//! wander goal (hunters/brain.rs `pick_wander_target`, fed from game/hunting.rs).

use crate::content::public_roster::{PublicLimits, PublicPatrol};
use std::collections::HashMap;

/// The patrol rules from data/public_roster.json, read once.
pub fn patrol() -> &'static PublicPatrol {
    static PATROL: std::sync::OnceLock<PublicPatrol> = std::sync::OnceLock::new();
    PATROL.get_or_init(|| crate::content::public_roster::PublicRoster::load().patrol)
}

/// A position on the route grid, in cells.
pub type Pos = (f32, f32);

/// One hunter's current patrol: the runner (peer id) it guards and the time it re-rolls.
#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub peer: u64,
    pub until_msec: u64,
}

/// Every hunter's assignment, keyed by hunter id (Godot: _public_patrol).
#[derive(Default, Debug)]
pub struct PatrolBook {
    entries: HashMap<String, Assignment>,
}

impl PatrolBook {
    /// The anchor for `hunter` this frame, or None when it has no runner to guard.
    /// `runners` maps each hunt-able runner's peer id to its position. A missing or expired entry re-rolls to the
    /// runner with the fewest other hunters on it; ties keep the lowest peer id (Godot sorts the keys first).
    /// With no runners at all the entry is left alone (Godot L5880-5881).
    pub fn anchor(
        &mut self,
        patrol: &PublicPatrol,
        hunter: &str,
        now_msec: u64,
        runners: &HashMap<u64, Pos>,
    ) -> Option<Pos> {
        let expired = self.entries.get(hunter).map_or(true, |e| now_msec >= e.until_msec);
        if expired {
            if runners.is_empty() {
                return None;
            }
            let mut load: HashMap<u64, usize> = HashMap::new();
            for (other, entry) in &self.entries {
                if other != hunter {
                    *load.entry(entry.peer).or_insert(0) += 1;
                }
            }
            let mut keys: Vec<u64> = runners.keys().copied().collect();
            keys.sort_unstable();
            let mut best: Option<(u64, usize)> = None;
            for peer in keys {
                let l = load.get(&peer).copied().unwrap_or(0);
                if best.map_or(true, |(_, bl)| l < bl) {
                    best = Some((peer, l));
                }
            }
            let (peer, _) = best?;
            self.entries
                .insert(hunter.to_string(), Assignment { peer, until_msec: now_msec + patrol.dwell_msec });
        }
        let peer = self.entries.get(hunter)?.peer;
        runners.get(&peer).copied()
    }
}

/// The wander candidates to choose from while patrolling `anchor`: those within `radius`, else those within
/// `radius * fallback_radius_scale`, else all of them (Godot _pick_wander_target L1120-1126). No anchor keeps all.
pub fn filter_candidates(patrol: &PublicPatrol, anchor: Option<Pos>, candidates: &[Pos]) -> Vec<Pos> {
    let Some(a) = anchor else {
        return candidates.to_vec();
    };
    let near = within(candidates, a, patrol.radius);
    if !near.is_empty() {
        return near;
    }
    let far = within(candidates, a, patrol.radius * patrol.fallback_radius_scale);
    if far.is_empty() {
        candidates.to_vec()
    } else {
        far
    }
}

/// A patrolling hunter's wander speed: the base speed times `speed_scale` while it has an anchor.
pub fn patrol_speed(base: f32, patrol: &PublicPatrol, anchored: bool) -> f32 {
    if anchored {
        base * patrol.speed_scale
    } else {
        base
    }
}

/// Whether the client may send another PUBLIC route join, given the joins it sent in the last minute. The limit is
/// `limits.join_per_minute` from data/public_roster.json "limits" (server JOIN_LIMIT_PER_MINUTE, 20 in Standard).
pub fn join_allowed(limits: &PublicLimits, joins_last_minute: u32) -> bool {
    joins_last_minute < limits.join_per_minute
}

/// Client-side sliding one-minute window of PUBLIC route joins; refuses once `join_per_minute` were sent.
#[derive(Default, Debug)]
pub struct JoinThrottle {
    sent: Vec<u64>,
}

impl JoinThrottle {
    /// Records a join at `now_msec` and returns true when allowed; a refused join is not recorded.
    pub fn try_join(&mut self, limits: &PublicLimits, now_msec: u64) -> bool {
        self.sent.retain(|t| now_msec.saturating_sub(*t) < 60_000);
        if !join_allowed(limits, self.sent.len().min(u32::MAX as usize) as u32) {
            return false;
        }
        self.sent.push(now_msec);
        true
    }
}

fn within(candidates: &[Pos], anchor: Pos, radius: f32) -> Vec<Pos> {
    candidates.iter().copied().filter(|p| dist(*p, anchor) <= radius).collect()
}

fn dist(p: Pos, q: Pos) -> f32 {
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::public_roster::PublicRoster;
    use serde_json::Value;

    fn patrol_from(text: &str) -> PublicPatrol {
        let v: Value = serde_json::from_str(text).unwrap();
        PublicRoster::parse(&serde_json::json!({ "patrol": v["patrol"].clone() })).patrol
    }

    fn runners(list: &[(u64, Pos)]) -> HashMap<u64, Pos> {
        list.iter().copied().collect()
    }

    #[test]
    fn the_embedded_patrol_block_is_the_standard_godot_patrol() {
        let (_, text) = crate::content::embedded::data_under("public_roster.json")
            .next()
            .expect("data/public_roster.json is embedded");
        let p = PublicRoster::parse(&serde_json::from_str::<Value>(text).unwrap()).patrol;
        assert_eq!(p, PublicPatrol::default());
        assert_eq!(p.radius, 30.0);
        assert_eq!(p.dwell_msec, 20_000);
    }

    #[test]
    fn candidates_prefer_the_radius_around_the_anchor() {
        let std_p = PublicPatrol::default();
        let cands = [(5.0, 0.0), (25.0, 0.0), (50.0, 0.0)];
        assert_eq!(filter_candidates(&std_p, Some((0.0, 0.0)), &cands), vec![(5.0, 0.0), (25.0, 0.0)]);
        // Changing the radius in data changes who is kept.
        let tight = patrol_from(r#"{"patrol":{"radius":10}}"#);
        assert_eq!(filter_candidates(&tight, Some((0.0, 0.0)), &cands), vec![(5.0, 0.0)]);
    }

    #[test]
    fn the_fallback_radius_is_tried_before_giving_up() {
        let cands = [(45.0, 0.0), (100.0, 0.0)];
        let std_p = PublicPatrol::default();
        // Nothing within 30, but 45 is within 60 (30 * 2.0).
        assert_eq!(filter_candidates(&std_p, Some((0.0, 0.0)), &cands), vec![(45.0, 0.0)]);
        // A fallback scale of 1.0 means no wider ring: every candidate stays.
        let no_fallback = patrol_from(r#"{"patrol":{"fallback_radius_scale":1.0}}"#);
        assert_eq!(filter_candidates(&no_fallback, Some((0.0, 0.0)), &cands), cands.to_vec());
        // A wider scale (4.0, ring of 120 cells) also takes in the 100-cell candidate.
        let wide = patrol_from(r#"{"patrol":{"fallback_radius_scale":4.0}}"#);
        assert_eq!(filter_candidates(&wide, Some((0.0, 0.0)), &cands), vec![(45.0, 0.0), (100.0, 0.0)]);
    }

    #[test]
    fn no_anchor_keeps_every_candidate() {
        let cands = [(1.0, 1.0), (500.0, 500.0)];
        assert_eq!(filter_candidates(&PublicPatrol::default(), None, &cands), cands.to_vec());
    }

    #[test]
    fn a_hunter_picks_the_runner_with_the_fewest_hunters_on_it() {
        let p = PublicPatrol::default();
        let both = runners(&[(1, (0.0, 0.0)), (2, (100.0, 0.0))]);
        assert_eq!(new_book_anchor(&p, "a", 0, &both), Some((0.0, 0.0)));
        // "b" sees "a" on peer 1, so peer 2 is the lighter runner.
        assert_eq!(new_book_anchor_pair(&p, &both), Some((100.0, 0.0)));
    }

    fn new_book_anchor(p: &PublicPatrol, id: &str, now: u64, r: &HashMap<u64, Pos>) -> Option<Pos> {
        PatrolBook::default().anchor(p, id, now, r)
    }

    fn new_book_anchor_pair(p: &PublicPatrol, r: &HashMap<u64, Pos>) -> Option<Pos> {
        let mut book = PatrolBook::default();
        book.anchor(p, "a", 0, r);
        book.anchor(p, "b", 0, r)
    }

    #[test]
    fn the_dwell_time_holds_the_anchor_then_rerolls_it() {
        let p = patrol_from(r#"{"patrol":{"dwell_msec":5000}}"#);
        let mut book = PatrolBook::default();
        let first = runners(&[(1, (0.0, 0.0))]);
        assert_eq!(book.anchor(&p, "a", 0, &first), Some((0.0, 0.0)));
        // Runner 1 leaves; before the dwell ends the hunter still guards peer 1 and has no anchor.
        let second = runners(&[(2, (100.0, 0.0))]);
        assert_eq!(book.anchor(&p, "a", 4_999, &second), None);
        // At the dwell end it re-rolls onto the runner that is there now.
        assert_eq!(book.anchor(&p, "a", 5_000, &second), Some((100.0, 0.0)));
    }

    #[test]
    fn the_standard_dwell_is_twenty_seconds() {
        let p = PublicPatrol::default();
        let mut book = PatrolBook::default();
        let first = runners(&[(1, (0.0, 0.0))]);
        book.anchor(&p, "a", 0, &first);
        let moved = runners(&[(2, (9.0, 9.0))]);
        assert_eq!(book.anchor(&p, "a", 19_999, &moved), None);
        assert_eq!(book.anchor(&p, "a", 20_000, &moved), Some((9.0, 9.0)));
    }

    #[test]
    fn with_no_runners_there_is_no_anchor_and_no_entry() {
        let p = PublicPatrol::default();
        let mut book = PatrolBook::default();
        assert_eq!(book.anchor(&p, "a", 0, &HashMap::new()), None);
        // A runner appearing later is picked up at once, because no entry was recorded.
        let r = runners(&[(7, (3.0, 4.0))]);
        assert_eq!(book.anchor(&p, "a", 1, &r), Some((3.0, 4.0)));
    }

    #[test]
    fn the_standard_join_limit_is_twenty_a_minute() {
        let l = PublicLimits::default();
        assert!(join_allowed(&l, 0));
        assert!(join_allowed(&l, 19));
        assert!(!join_allowed(&l, 20));
    }

    #[test]
    fn changing_the_join_limit_in_data_changes_who_is_refused() {
        let l = PublicLimits::parse(&serde_json::json!({ "join_per_minute": 3 }));
        assert!(join_allowed(&l, 2));
        assert!(!join_allowed(&l, 3));
    }

    #[test]
    fn patrol_speed_uses_the_data_scale_only_when_anchored() {
        let std_p = PublicPatrol::default();
        assert_eq!(patrol_speed(0.82, &std_p, true), 0.82);
        assert_eq!(patrol_speed(0.82, &std_p, false), 0.82);
        let slow = patrol_from(r#"{"patrol":{"speed_scale":0.5}}"#);
        assert!((patrol_speed(0.82, &slow, true) - 0.41).abs() < 1e-6);
        assert_eq!(patrol_speed(0.82, &slow, false), 0.82);
    }

    #[test]
    fn the_join_throttle_refuses_past_the_limit_and_recovers_after_a_minute() {
        let limits = PublicLimits { join_per_minute: 3, ..PublicLimits::default() };
        let mut t = JoinThrottle::default();
        assert!(t.try_join(&limits, 0) && t.try_join(&limits, 10) && t.try_join(&limits, 20));
        assert!(!t.try_join(&limits, 30_000));
        assert!(t.try_join(&limits, 60_001));
        let none = PublicLimits { join_per_minute: 0, ..PublicLimits::default() };
        assert!(!JoinThrottle::default().try_join(&none, 0));
    }
}
