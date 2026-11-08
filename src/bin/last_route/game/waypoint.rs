//! One waypoint per player (TODO "Teammate waypoints", "Waypoints per player"). The runner's own waypoint, its route and its
//! map pin stay in `game::endless` (`Waypoint`, `WaypointRoutes`); this keeps the waypoints of everyone in the room, so a
//! teammate's can be drawn in the world (chevrons from the teammate toward it, [`crate::ui::teammate_markers`]) and as a pin on
//! the maps ([`crate::ui::map::set_extras`]). Setting a new waypoint replaces the player's old one; clearing removes it.
//! The tunables (chevron spacing and count, the longest reach, the label) are in `data/hud/teammate_markers.json`.
//!
//! Sync (hook for `net/world_sync.rs`): the sender's sample carries `wp` (`[x, height, z]` in absolute cells and metres) and
//! `wl` (its level) while it has a waypoint, and neither when it has none; [`PeerWaypoint::to_wire`] and
//! [`PeerWaypoint::from_wire`] are the two ends. Only the player's own client changes its own waypoint.
//! Open: the ground trail (the chevrons on the floor along the route) stays the own-waypoint PATH guidance in `endless.rs`.

use crate::ui::teammate_markers::WaypointRules;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// One player's waypoint: a point `[x, height, z]` and the level it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeerWaypoint {
    pub pos: [f32; 3],
    pub level: i32,
}

impl PeerWaypoint {
    /// The fields to add to a sample: `wp` and `wl`.
    pub fn to_wire(&self, into: &mut Map<String, Value>) {
        into.insert("wp".into(), json!(self.pos));
        into.insert("wl".into(), json!(self.level));
    }

    /// The waypoint a sample carries, or None when it has none or the numbers are not finite (a hostile peer).
    pub fn from_wire(sample: &Map<String, Value>) -> Option<PeerWaypoint> {
        let arr = sample.get("wp")?.as_array()?;
        if arr.len() != 3 {
            return None;
        }
        let mut pos = [0.0f32; 3];
        for (slot, v) in pos.iter_mut().zip(arr) {
            let x = v.as_f64()?;
            if !x.is_finite() || x.abs() > 1.0e7 {
                return None;
            }
            *slot = x as f32;
        }
        let level = sample.get("wl").and_then(Value::as_i64).unwrap_or(0).clamp(-1000, 1000) as i32;
        Some(PeerWaypoint { pos, level })
    }
}

/// The waypoints of the room, by peer id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waypoints {
    by_peer: BTreeMap<u32, PeerWaypoint>,
}

impl Waypoints {
    /// Sets the player's waypoint, replacing its old one (one per player).
    pub fn set(&mut self, peer: u32, wp: PeerWaypoint) {
        self.by_peer.insert(peer, wp);
    }

    /// Removes the player's waypoint; true if it had one.
    pub fn clear(&mut self, peer: u32) -> bool {
        self.by_peer.remove(&peer).is_some()
    }

    pub fn get(&self, peer: u32) -> Option<&PeerWaypoint> {
        self.by_peer.get(&peer)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &PeerWaypoint)> {
        self.by_peer.iter().map(|(p, w)| (*p, w))
    }

    pub fn len(&self) -> usize {
        self.by_peer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_peer.is_empty()
    }

    /// Updates from a peer's sample: sets its waypoint when the sample carries one, clears it when not.
    pub fn apply_sample(&mut self, peer: u32, sample: &Map<String, Value>) {
        match PeerWaypoint::from_wire(sample) {
            Some(w) => self.set(peer, w),
            None => {
                self.clear(peer);
            }
        }
    }

    /// Drops the waypoints of players who left the room.
    pub fn retain_peers(&mut self, present: &[u32]) {
        self.by_peer.retain(|p, _| present.contains(p));
    }
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Whether a waypoint at `to` may be set from `from`: within the data's longest reach.
pub fn in_reach(from: [f32; 3], to: [f32; 3], rules: &WaypointRules) -> bool {
    let d = dist3(from, to);
    d.is_finite() && d <= rules.max_range
}

/// The world points of the chevrons from `from` toward `to`: the first one a spacing in, one each spacing after, at most
/// `chevron_count`, none past the waypoint. Empty when the two are on top of each other.
pub fn chevron_points(from: [f32; 3], to: [f32; 3], rules: &WaypointRules) -> Vec<[f32; 3]> {
    let d = dist3(from, to);
    if !d.is_finite() || d < rules.chevron_spacing {
        return Vec::new();
    }
    let dir = [(to[0] - from[0]) / d, (to[1] - from[1]) / d, (to[2] - from[2]) / d];
    let mut out = Vec::new();
    let mut t = rules.chevron_spacing;
    while t < d && out.len() < rules.chevron_count {
        out.push([from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t]);
        t += rules.chevron_spacing;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::teammate_markers::rules;

    fn wp(x: f32, z: f32) -> PeerWaypoint {
        PeerWaypoint { pos: [x, 0.0, z], level: 0 }
    }

    #[test]
    fn a_new_waypoint_replaces_the_players_old_one_and_clearing_removes_it() {
        let mut w = Waypoints::default();
        assert!(w.is_empty());
        w.set(3, wp(1.0, 1.0));
        w.set(3, wp(5.0, 5.0));
        w.set(4, wp(9.0, 9.0));
        assert_eq!(w.len(), 2, "one per player");
        assert_eq!(w.get(3), Some(&wp(5.0, 5.0)));
        assert!(w.clear(3));
        assert!(!w.clear(3), "nothing left to clear");
        assert_eq!(w.get(3), None);
        assert_eq!(w.get(4), Some(&wp(9.0, 9.0)), "another player's is untouched");
    }

    #[test]
    fn a_sample_sets_or_clears_and_a_bad_one_clears() {
        let mut w = Waypoints::default();
        let mut sample = Map::new();
        PeerWaypoint { pos: [1.5, 2.0, -3.0], level: -1 }.to_wire(&mut sample);
        w.apply_sample(7, &sample);
        assert_eq!(w.get(7), Some(&PeerWaypoint { pos: [1.5, 2.0, -3.0], level: -1 }));
        w.apply_sample(7, &Map::new());
        assert_eq!(w.get(7), None, "a sample without one clears it");
        sample.insert("wp".into(), json!([1.0, 2.0]));
        w.apply_sample(7, &sample);
        assert_eq!(w.get(7), None, "a short array is refused");
        sample.insert("wp".into(), json!([1.0, 2.0, 1.0e12]));
        assert_eq!(PeerWaypoint::from_wire(&sample), None, "an absurd coordinate is refused");
    }

    #[test]
    fn players_who_left_lose_their_waypoint() {
        let mut w = Waypoints::default();
        w.set(1, wp(0.0, 0.0));
        w.set(2, wp(0.0, 0.0));
        w.retain_peers(&[2]);
        assert_eq!(w.iter().map(|(p, _)| p).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn chevrons_run_from_the_player_toward_the_waypoint_and_stop_before_it() {
        let r = rules().waypoint.clone();
        let pts = chevron_points([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], &r);
        assert!(!pts.is_empty() && pts.len() <= r.chevron_count);
        assert!((pts[0][0] - r.chevron_spacing).abs() < 1e-4, "the first is a spacing in");
        assert!(pts.windows(2).all(|p| p[1][0] > p[0][0]));
        assert!(pts.iter().all(|p| p[0] < 10.0));
        let near = chevron_points([0.0, 0.0, 0.0], [0.5, 0.0, 0.0], &r);
        assert!(near.is_empty(), "closer than a spacing draws none");
        let short = chevron_points([0.0, 0.0, 0.0], [r.chevron_spacing * 2.5, 0.0, 0.0], &r);
        assert_eq!(short.len(), 2);
    }

    #[test]
    fn the_chevron_data_changes_the_trail() {
        let mut r = rules().waypoint.clone();
        r.chevron_count = 2;
        assert_eq!(chevron_points([0.0; 3], [100.0, 0.0, 0.0], &r).len(), 2);
        r.chevron_count = 0;
        assert!(chevron_points([0.0; 3], [100.0, 0.0, 0.0], &r).is_empty());
    }

    #[test]
    fn a_waypoint_can_only_be_set_within_reach() {
        let r = rules().waypoint.clone();
        assert!(in_reach([0.0; 3], [r.max_range - 1.0, 0.0, 0.0], &r));
        assert!(!in_reach([0.0; 3], [r.max_range + 1.0, 0.0, 0.0], &r));
        assert!(!in_reach([0.0; 3], [f32::NAN, 0.0, 0.0], &r));
    }
}
