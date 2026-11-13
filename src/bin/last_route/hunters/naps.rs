//! Hunters napping in beds (Godot `game/hunters/ai/hunter_naps.gd`, docs/SLEEP.md). A nap is a wander activity
//! (`Activity::Nap` in activities.rs, a `bed` point of interest): the hunter walks to a bed, lies down for the data
//! duration (`activities.json` `nap`) and wakes on its own. While it sleeps the brain runs no perception and no
//! movement, only the clock and this wake rule. A runner close by, a loud one, or a torch in the napper's face wakes
//! it with a startle: a flinch (a short stun), then it investigates where the runner was.
//!
//! The numbers are `params` in data/hunters/activities.json (`nap_wake_*`, `nap_startle_stun`).

use super::activities::Rules;
use super::brain::Target;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Whether the runner wakes a hunter asleep at `hunter_at` on `hunter_level` (Godot `HunterNaps.wakes_for`). A hidden
/// runner never does, and neither does one on another level: a floor or a flight between them keeps it asleep.
pub fn wakes_for(rules: &Rules, hunter_at: (f32, f32), hunter_level: i32, target: &Target) -> bool {
    if target.hidden || target.cross.is_some() || target.level != hunter_level {
        return false;
    }
    let d = dist(hunter_at, target.pos);
    if d <= rules.nap_wake_distance {
        return true;
    }
    if target.noise >= rules.nap_wake_noise && d <= rules.nap_wake_noise_distance {
        return true;
    }
    if target.flashlight_on && d <= rules.nap_wake_torch_distance {
        // the beam faces the hunter: the runner's facing against the way from the runner to the hunter
        let (fx, fz) = target.facing;
        let flen = (fx * fx + fz * fz).sqrt();
        if flen > 0.0 && d > 0.0 {
            let (tx, tz) = ((hunter_at.0 - target.pos.0) / d, (hunter_at.1 - target.pos.1) / d);
            if (fx * tx + fz * tz) / flen > rules.nap_wake_torch_cone {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(pos: (f32, f32)) -> Target {
        Target {
            pos,
            facing: (1.0, 0.0),
            hidden: false,
            hiding_spot: None,
            in_safe_zone: false,
            noise: 0.0,
            flashlight_on: false,
            lamp_charge: 100.0,
            sprinting: false,
            in_vent: false,
            vent_time_in: 0.0,
            temperature: 0.0,
            pesticide: false,
            map_open: false,
            level: 0,
            y: 0.0,
            cross: None,
        }
    }

    #[test]
    fn a_runner_close_by_wakes_a_napper_and_a_far_quiet_one_does_not() {
        let r = Rules::default();
        assert!(wakes_for(&r, (0.0, 0.0), 0, &runner((2.0, 0.0))));
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &runner((8.0, 0.0))));
        // a runner one floor up, 2 m away, is not heard through the floor
        let mut above = runner((2.0, 0.0));
        above.level = 1;
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &above));
    }

    #[test]
    fn noise_wakes_within_its_distance_only() {
        let r = Rules::default();
        let mut loud = runner((7.0, 0.0));
        loud.noise = r.nap_wake_noise;
        assert!(wakes_for(&r, (0.0, 0.0), 0, &loud));
        loud.pos = (r.nap_wake_noise_distance + 1.0, 0.0);
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &loud));
    }

    #[test]
    fn a_torch_in_the_napper_face_wakes_it_and_one_off_to_the_side_does_not() {
        let r = Rules::default();
        let mut lit = runner((5.0, 0.0));
        lit.flashlight_on = true;
        // the runner faces -x, the hunter is at the origin: the beam is in its face
        lit.facing = (-1.0, 0.0);
        assert!(wakes_for(&r, (0.0, 0.0), 0, &lit));
        // the runner faces away from the hunter: no beam in its face
        lit.facing = (1.0, 0.0);
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &lit));
        lit.facing = (-1.0, 0.0);
        // the hunter off to the side of the beam
        assert!(!wakes_for(&r, (5.0, 5.0), 0, &lit));
        lit.flashlight_on = false;
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &lit), "no torch, no wake at this distance");
    }

    #[test]
    fn a_hidden_runner_never_wakes_a_nap() {
        let r = Rules::default();
        let mut hidden = runner((0.5, 0.0));
        hidden.hidden = true;
        assert!(!wakes_for(&r, (0.0, 0.0), 0, &hidden));
    }
}
