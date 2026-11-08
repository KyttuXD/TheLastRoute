//! The wander patrol-anchor branch (Godot hunter_brain.gd `has_patrol_anchor`, `patrol_anchor`, PATROL_RADIUS and
//! `_pick_wander_target` 1120-1126) and the public-route anchor on a live runner.
//!
//! A hunter with an anchor wanders to a candidate within the anchor radius; with none there, within the radius times the
//! fallback scale; with none there either, to any candidate. Radius, scale and the live-runner switch come from
//! data/hunters/nav_tuning.json `patrol` (`navigation::tuning().patrol`). Host-only: the side that simulates hunters sets
//! the anchor, a guest only sees the hunter move.
//!
//! Hooks: `Brain::pick_wander_target_with` keeps its `filter_candidates` call; where it picks a stand from the kept points
//! it can use `pick_in_radius` instead. game/hunting.rs sets `brain.patrol_anchor = live_runner_anchor(..)` for every idle
//! hunter of a public route each frame (see the task hooks).

use super::navigation::PatrolRules;

pub type Pos = (f32, f32);

fn dist(a: Pos, b: Pos) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Indices of the candidates a patrolling hunter may pick: within `radius` of the anchor, else within `radius * scale`,
/// else all of them. No anchor keeps all.
pub fn candidates_near(rules: &PatrolRules, anchor: Option<Pos>, candidates: &[Pos]) -> Vec<usize> {
    let all = || (0..candidates.len()).collect::<Vec<usize>>();
    let Some(a) = anchor else { return all() };
    let within = |r: f32| -> Vec<usize> {
        candidates.iter().enumerate().filter(|(_, c)| dist(**c, a) <= r).map(|(i, _)| i).collect()
    };
    let near = within(rules.radius);
    if !near.is_empty() {
        return near;
    }
    let far = within(rules.radius * rules.fallback_radius_scale);
    if far.is_empty() {
        all()
    } else {
        far
    }
}

/// The anchor-radius pick: one of the kept candidates by `roll` in 0..1 (the hunter's activity stream). None when there
/// are no candidates at all.
pub fn pick_in_radius(rules: &PatrolRules, anchor: Option<Pos>, candidates: &[Pos], roll: f32) -> Option<usize> {
    let kept = candidates_near(rules, anchor, candidates);
    if kept.is_empty() {
        return None;
    }
    let i = ((roll.clamp(0.0, 0.999_999) * kept.len() as f32) as usize).min(kept.len() - 1);
    Some(kept[i])
}

/// The anchor of an idle hunter on a public route: the live runner nearest to it, so the patrol stays on a real runner.
/// None when the switch is off or no runner is live. `runners` are the positions of the runners that can be hunted.
pub fn live_runner_anchor(rules: &PatrolRules, hunter: Pos, runners: &[Pos]) -> Option<Pos> {
    if !rules.live_runner_anchor {
        return None;
    }
    runners
        .iter()
        .copied()
        .min_by(|a, b| dist(*a, hunter).partial_cmp(&dist(*b, hunter)).unwrap_or(std::cmp::Ordering::Equal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::navigation::NavTuning;

    fn rules() -> PatrolRules {
        NavTuning::GODOT.patrol
    }

    fn ring() -> Vec<Pos> {
        vec![(0.0, 0.0), (10.0, 0.0), (29.0, 0.0), (45.0, 0.0), (59.0, 0.0), (200.0, 0.0)]
    }

    #[test]
    fn the_standard_radius_is_the_godot_patrol_radius() {
        assert_eq!(rules().radius, 30.0);
        assert_eq!(rules().fallback_radius_scale, 2.0);
    }

    #[test]
    fn picks_stay_within_the_radius_of_the_anchor() {
        let r = rules();
        let c = ring();
        for k in 0..50 {
            let i = pick_in_radius(&r, Some((0.0, 0.0)), &c, k as f32 / 50.0).unwrap();
            assert!(dist(c[i], (0.0, 0.0)) <= r.radius, "pick {i} at {:?}", c[i]);
        }
    }

    #[test]
    fn with_none_in_the_radius_the_doubled_radius_then_everything_is_tried() {
        let r = rules();
        let c = ring();
        // anchor far from the first three: only 45 and 59 are within 30, 60 of (100, 0)? none in 30; 45, 59, 200 -> 55, 41
        let far = candidates_near(&r, Some((100.0, 0.0)), &c);
        assert_eq!(far, vec![3, 4], "within 60 of the anchor when none is within 30");
        let none = candidates_near(&r, Some((1000.0, 1000.0)), &c);
        assert_eq!(none.len(), c.len(), "nothing near: every candidate stays");
        assert_eq!(candidates_near(&r, None, &c).len(), c.len(), "no anchor keeps all");
        assert_eq!(pick_in_radius(&r, Some((0.0, 0.0)), &[], 0.5), None);
    }

    #[test]
    fn a_changed_radius_changes_the_picks() {
        let tight = PatrolRules { radius: 5.0, ..rules() };
        let c = ring();
        assert_eq!(candidates_near(&tight, Some((0.0, 0.0)), &c), vec![0]);
        assert_eq!(candidates_near(&rules(), Some((0.0, 0.0)), &c), vec![0, 1, 2]);
    }

    #[test]
    fn the_anchor_on_a_public_route_is_the_nearest_live_runner() {
        let r = rules();
        let runners = [(50.0, 0.0), (5.0, 5.0), (-30.0, 0.0)];
        assert_eq!(live_runner_anchor(&r, (0.0, 0.0), &runners), Some((5.0, 5.0)));
        assert_eq!(live_runner_anchor(&r, (0.0, 0.0), &[]), None, "no live runner, no anchor");
        let off = PatrolRules { live_runner_anchor: false, ..r };
        assert_eq!(live_runner_anchor(&off, (0.0, 0.0), &runners), None, "the data switch turns it off");
    }
}
