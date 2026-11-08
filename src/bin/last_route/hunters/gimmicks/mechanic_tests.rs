//! One test per hunter mechanic animation (docs/HUNTER_ANIMATIONS.md): the real gameplay event, driven through
//! `Brain::update`, puts the hunter in the named state, and the state clears by itself when the event is over. The
//! names are what the host sends in its hunter frame and the view plays (`Brain::mechanic`).

use super::bench::{Bench, DT};
use super::{Event, Gimmick};
use crate::config::Difficulty;
use crate::hunters::brain::State;

/// A point `d` cells from `p` that a hunter can stand on with a clear line to `p`.
fn open_beside(x: &Bench, p: (f32, f32), d: f32) -> (f32, f32) {
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let q = (p.0 + dx * d, p.1 + dy * d);
        if x.fac.can_occupy(q.0 as f64, q.1 as f64, 0.33) && x.fac.has_line_of_sight(p, q) {
            return q;
        }
    }
    panic!("no open point {d} cells from {p:?}");
}

// ---------------------------------------------------------------------------------------------- Ysolde

#[test]
fn ysolde_reaches_for_a_battery_then_pockets_it_and_both_clear() {
    let mut x = Bench::new("ysolde");
    assert!(!x.fac.batteries.is_empty(), "the test floor has batteries");
    for i in 1..x.batteries_taken.len() {
        x.batteries_taken[i] = true;
    }
    let bat = x.fac.batteries[0].center();
    let from = open_beside(&x, bat, 1.4);
    x.place(from);
    x.frame();
    assert_eq!(x.mech(), Some("steal_battery"), "within reach of the battery she is heading for");
    assert!(x.until(6.0, |x| x.batteries_taken[0]), "she takes it");
    assert_eq!(x.mech(), Some("pocket_battery"), "the frame the battery is taken she is pocketing it");
    assert!(x.events.iter().any(|e| matches!(e, Event::TakeBattery(0))));
    x.step(0.5);
    assert_eq!(x.mech(), Some("pocket_battery"), "and keeps it up for the cue's length");
    x.step(0.6);
    assert_eq!(x.mech(), None, "then both are over: the battery is gone, nothing to reach for");
}

#[test]
fn ysolde_only_reaches_when_a_battery_is_close_and_not_while_fleeing() {
    let mut x = Bench::new("ysolde");
    let bat = x.fac.batteries[0].center();
    // far from every battery: nothing to reach for
    let far = x.fac.floor_cells().into_iter().map(|c| c.center()).find(|p| x.fac.batteries.iter().all(|b| crate::hunters::brain::dist(b.center(), *p) > 6.0)).unwrap();
    x.place(far);
    x.frame();
    assert_eq!(x.mech(), None);
    // the runner in plain sight close by: she flees instead of reaching
    for i in 1..x.batteries_taken.len() {
        x.batteries_taken[i] = true;
    }
    let from = open_beside(&x, bat, 1.4);
    x.place(from);
    x.t.hidden = false;
    x.t.pos = open_beside(&x, from, 3.0);
    x.frame();
    assert_ne!(x.mech(), Some("steal_battery"), "she keeps her distance from the runner before she reaches");
}

// ---------------------------------------------------------------------------------------------- Solyth

#[test]
fn solyth_burns_a_battery_every_thirty_seconds_and_the_cue_clears() {
    let mut x = Bench::new("solyth");
    if let Gimmick::Solyth(s) = &mut x.b.gimmick {
        s.heat_locked = true;
    }
    assert!(x.until(35.0, |x| x.batteries_taken.iter().any(|t| *t)), "a battery burns");
    assert_eq!(x.mech(), Some("burn_battery"));
    x.step(1.0);
    assert_eq!(x.mech(), Some("burn_battery"), "1.1 s long");
    x.step(0.3);
    assert_eq!(x.mech(), None);
}

#[test]
fn solyth_recoils_while_the_cold_source_holds_her_off_and_stops_when_it_ends() {
    let mut x = Bench::new("solyth");
    if let Gimmick::Solyth(s) = &mut x.b.gimmick {
        s.heat_locked = true;
        s.deter(2.0, 8.0);
    }
    x.frame();
    assert_eq!(x.mech(), Some("cold_recoil"));
    x.step(2.5);
    assert_eq!(x.mech(), None, "the deterrence ran out");
}

#[test]
fn solyth_pounces_in_the_air_only_while_the_leap_runs() {
    let mut x = Bench::with_diff("solyth", Difficulty::Nightmare);
    let Gimmick::Solyth(s) = &mut x.b.gimmick else { panic!() };
    assert_eq!(s.mechanic(), None);
    s.set_leaping(true);
    assert_eq!(x.b.mechanic(), Some("pounce"));
    let Gimmick::Solyth(s) = &mut x.b.gimmick else { panic!() };
    s.set_leaping(false);
    assert_eq!(x.b.mechanic(), None);
}

// ---------------------------------------------------------------------------------------------- Nyxie

#[test]
fn nyxie_unfolds_out_of_the_dark_when_a_dead_lamp_wakes_her_and_it_clears() {
    let mut x = Bench::new("nyxie");
    x.b.active = false;
    x.flashlight_dead = true;
    x.t.flashlight_on = false;
    x.t.lamp_charge = 0.0;
    assert!(x.until(6.0, |x| x.b.active), "a dead flashlight wakes her within seconds");
    assert_eq!(x.mech(), Some("darkness_wake"));
    x.step(1.4);
    assert_eq!(x.mech(), None);
}

#[test]
fn nyxie_pounces_in_the_air_only_while_the_leap_runs() {
    let mut x = Bench::with_diff("nyxie", Difficulty::Nightmare);
    let Gimmick::Nyxie(n) = &mut x.b.gimmick else { panic!() };
    assert_eq!(n.mechanic(), None);
    n.set_leaping(true);
    assert_eq!(x.b.mechanic(), Some("pounce"));
    let Gimmick::Nyxie(n) = &mut x.b.gimmick else { panic!() };
    n.set_leaping(false);
    assert_eq!(x.b.mechanic(), None);
}

// ---------------------------------------------------------------------------------------------- Glyra

#[test]
fn glyra_cradles_the_box_while_it_plays_startles_when_it_stops_and_settles_when_it_is_wound() {
    let mut x = Bench::new("glyra");
    x.music_box_left = 45.0;
    x.step(0.2);
    assert_eq!(x.mech(), Some("music_box_hold"), "the box is playing");
    x.music_box_left = 10.0;
    x.step(0.1);
    assert_eq!(x.mech(), None, "winding down: restless, nothing in her hands");
    x.music_box_left = 0.0;
    x.step(DT);
    assert_eq!(x.mech(), Some("music_box_startle"), "the tune ran out");
    x.step(1.3);
    assert_eq!(x.mech(), None, "and she hunts");
    x.music_box_left = 45.0;
    x.step(DT);
    assert_eq!(x.mech(), Some("music_box_settle"), "the runner wound it");
    x.step(2.2);
    assert_eq!(x.mech(), Some("music_box_hold"), "back to cradling it once the settle is over");
}

#[test]
fn glyra_puts_the_box_away_in_a_chase() {
    let mut x = Bench::new("glyra");
    x.step(0.2);
    assert_eq!(x.mech(), Some("music_box_hold"));
    x.b.state = State::Chase;
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Mavrine

#[test]
fn mavrine_surveys_standing_still_then_points_when_the_mark_is_crossed() {
    let mut x = Bench::new("mavrine");
    assert!(x.until_mech(8.0, "survey"), "the survey comes after the 4.2 s cooldown");
    assert!(x.b.gimmick.holds_position(&x.b), "she stands still for it");
    assert!(x.until(2.0, |x| x.mech().is_none()), "and the survey is over after 0.9 s");
    let mark = {
        let Gimmick::Mavrine(m) = &x.b.gimmick else { panic!() };
        m.mark().expect("she projected a mark")
    };
    x.t.pos = mark;
    x.t.hidden = false;
    x.t.in_safe_zone = false;
    x.frame();
    assert_eq!(x.mech(), Some("ping_mark"), "the runner crossed it");
    x.step(0.6);
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Aki

#[test]
fn aki_holds_her_head_while_the_fuse_burns_and_flings_her_arms_at_each_burst() {
    let mut x = Bench::new("aki");
    x.beam_on_hunter(3.0);
    assert!(x.until(6.0, |x| x.b.flash_triggered), "a held beam flashes her");
    x.t.flashlight_on = false;
    x.t.hidden = true;
    x.frame();
    assert_eq!(x.mech(), Some("fuse_lit"), "the ten second fuse");
    let mut bursts = 0;
    let mut seen_burst = false;
    for _ in 0..(22.0 / DT) as usize {
        let ev = x.frame();
        if Bench::has_meter(&ev) {
            bursts += 1;
            assert_eq!(x.mech(), Some("fuse_burst"), "the burst frame");
            seen_burst = true;
        }
    }
    assert!(seen_burst);
    assert_eq!(bursts, 3, "three bursts, 5 s apart");
    assert_eq!(x.mech(), None, "the fuse is spent and the pose with it");
}

// ---------------------------------------------------------------------------------------------- Briselle

#[test]
fn briselle_points_at_the_relay_she_is_locked_on_and_lets_go_after_twelve_seconds() {
    let mut x = Bench::new("briselle");
    x.step(DT);
    assert_eq!(x.mech(), None);
    x.relays_taken[0] = true;
    x.frame();
    assert_eq!(x.mech(), Some("relay_lock"), "a relay was recovered after she woke");
    x.step(11.0);
    assert_eq!(x.mech(), Some("relay_lock"));
    x.step(1.5);
    assert_eq!(x.mech(), None, "the 12 s lock ran out");
}

#[test]
fn briselle_loses_the_pose_when_a_held_beam_breaks_the_lock() {
    let mut x = Bench::new("briselle");
    x.beam_on_hunter(3.0);
    x.frame();
    x.relays_taken[0] = true;
    x.frame();
    assert_eq!(x.mech(), Some("relay_lock"));
    assert!(x.until(6.0, |x| x.b.stun > 0.0), "the beam stuns her");
    assert_eq!(x.mech(), None, "a stunned hunter is not frozen mid-pose");
}

// ---------------------------------------------------------------------------------------------- Ceryth

#[test]
fn ceryth_watches_her_locker_and_stops_once_the_runner_answers_right() {
    let mut x = Bench::new("ceryth");
    x.frame();
    assert_eq!(x.mech(), Some("locker_vigil"));
    let (spot, red) = {
        let Gimmick::Ceryth(c) = &x.b.gimmick else { panic!() };
        (c.trap_spot().expect("waiting by a locker"), c.eyes_red)
    };
    // red eyes want the light off, blue want it on
    let entry = {
        let Gimmick::Ceryth(c) = &mut x.b.gimmick else { panic!() };
        c.resolve_entry(spot, !red)
    };
    assert!(matches!(entry, super::ceryth::Entry::Cleared));
    x.step(2.0 * DT);
    assert_eq!(x.mech(), None, "she runs off to another locker");
}

// ---------------------------------------------------------------------------------------------- Elise

#[test]
fn elise_snaps_round_when_fifty_seconds_of_sprinting_lock_her_on() {
    let mut x = Bench::new("elise");
    x.t.sprinting = true;
    x.t.in_safe_zone = false;
    assert!(x.until_mech(52.0, "lock_on"), "fifty seconds of the runner's sprint");
    x.step(1.6);
    assert_eq!(x.mech(), None, "the snap is a moment, the lock is for good");
}

// ---------------------------------------------------------------------------------------------- Ferryn

#[test]
fn ferryn_shields_her_face_when_a_flash_sends_her_home_and_it_clears() {
    let mut x = Bench::new("ferryn");
    x.frame(); // her first frame sends her to her far-off room
    x.beam_on_hunter(3.0);
    assert!(x.until_mech(8.0, "flash_recoil"), "a held beam flashes her");
    x.step(1.2);
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Haelith

#[test]
fn haelith_crouches_at_the_relay_she_camps_and_stands_when_she_leaves() {
    let mut x = Bench::new("haelith");
    let relay = x.fac.relays[0].center();
    for i in 1..x.relays_taken.len() {
        x.relays_taken[i] = true;
    }
    // with more than one open relay she leaves the first to Opalie; one open relay is hers
    x.place(open_beside(&x, relay, 0.9));
    x.frame();
    assert_eq!(x.mech(), Some("relay_camp"));
    x.place(open_beside(&x, relay, 5.0));
    x.b.state = State::Wander;
    assert_eq!(x.mech(), None, "away from the relay she is just walking");
}

// ---------------------------------------------------------------------------------------------- Ilyra

#[test]
fn ilyra_watches_from_her_corner_and_throws_a_curse_after_losing_sight() {
    let mut x = Bench::new("ilyra");
    x.frame();
    assert_eq!(x.mech(), Some("corner_watch"), "she never walks");
    // the runner stands in her room in plain sight, then hides
    x.t.pos = open_beside(&x, x.b.pos, 1.5);
    x.t.hidden = false;
    x.t.in_safe_zone = false;
    x.step(0.5);
    x.runner_far();
    let seen_stun = x.step(16.0).iter().any(|e| matches!(e, Event::Stun(_)));
    assert!(seen_stun, "fifteen seconds after losing sight the runner is pounced");
}

#[test]
fn ilyra_curse_cue_is_raised_with_the_stun_and_clears() {
    let mut x = Bench::new("ilyra");
    x.frame();
    x.t.pos = open_beside(&x, x.b.pos, 1.5);
    x.t.hidden = false;
    x.t.in_safe_zone = false;
    x.step(0.5);
    x.runner_far();
    assert!(x.until_mech(17.0, "pounce_curse"));
    x.step(1.5);
    assert_ne!(x.mech(), Some("pounce_curse"));
}

// ---------------------------------------------------------------------------------------------- Javelle

#[test]
fn javelle_looms_over_the_locker_the_runner_hides_in_and_straightens_when_she_is_gone() {
    let mut x = Bench::new("javelle");
    let spot = 0;
    let centre = x.fac.hiding[spot].cell.center();
    x.place(open_beside(&x, centre, 1.2));
    x.t.pos = centre;
    x.t.hidden = true;
    x.t.hiding_spot = Some(spot);
    x.step(DT);
    assert_eq!(x.mech(), Some("locker_loom"));
    // the runner leaves the locker
    x.t.hidden = false;
    x.t.hiding_spot = None;
    x.t.pos = x.fac.start.center();
    x.step(DT);
    assert_eq!(x.mech(), None);
}

#[test]
fn javelle_drops_the_pose_when_the_pull_stuns_her() {
    let mut x = Bench::new("javelle");
    let centre = x.fac.hiding[0].cell.center();
    x.place(open_beside(&x, centre, 1.2));
    x.t.pos = centre;
    x.t.hidden = true;
    x.t.hiding_spot = Some(0);
    let ev = x.step(8.5);
    assert!(ev.iter().any(|e| matches!(e, Event::Eject { .. })), "she pulls the runner out after eight seconds");
    assert!(x.b.stun > 0.0, "and is winded");
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Kathy

#[test]
fn kathy_points_along_her_beam_while_it_finds_the_runner_and_not_after() {
    let mut x = Bench::new("kathy");
    let (west, east) = x.clear_pair(4.0);
    x.place(west);
    x.b.facing = (1.0, 0.0);
    x.t = crate::hunters::brain::Target { pos: east, hidden: false, ..Default::default() };
    x.frame();
    assert_eq!(x.mech(), Some("beam_spot"), "the beam found the runner");
    x.runner_far();
    x.b.state = State::Wander;
    x.frame();
    assert_eq!(x.mech(), None, "the runner is out of the beam");
}

// ---------------------------------------------------------------------------------------------- Loxelle

#[test]
fn loxelle_coils_for_the_windup_and_drives_for_the_charge_then_both_clear() {
    let mut x = Bench::new("loxelle");
    // in a room with a locker, where she rolls for a charge every five seconds
    let room_spot = x.fac.hiding.iter().position(|h| h.room.is_some()).expect("a locker in a room");
    // standing at the locker's front, inside the 0.95 cell reach of her charge
    let locker = x.fac.hiding[room_spot].cell.center();
    let face = x.fac.hiding[room_spot].facing;
    let home = (locker.0 + face.0 as f32 * 0.9, locker.1 + face.1 as f32 * 0.9);
    assert!(x.fac.can_occupy(home.0 as f64, home.1 as f64, 0.33), "open floor at the locker's front");
    x.place(home);
    let mut seen = vec![];
    for _ in 0..(900.0 / DT) as usize {
        // her wander now walks to points of interest and leaves the room: bring her back until the roll hits
        if seen.is_empty() && x.mech().is_none() {
            x.place(home);
        }
        x.frame();
        if let Some(m) = x.mech() {
            if seen.last() != Some(&m) {
                seen.push(m);
            }
        }
        if seen.len() == 2 && x.mech().is_none() {
            break;
        }
    }
    assert_eq!(seen, vec!["charge_windup", "charge_lunge"], "windup, then the charge; then nothing");
}

// ---------------------------------------------------------------------------------------------- Opalie

#[test]
fn opalie_guards_her_relay_and_drops_the_pose_when_she_leaves_the_post() {
    let mut x = Bench::new("opalie");
    let relay = x.fac.relays[0].center();
    let post = super::open_near(&x.fac, relay);
    x.place(post);
    x.frame();
    assert_eq!(x.mech(), Some("guard_post"));
    x.place(x.fac.start.center());
    x.frame();
    assert_eq!(x.mech(), None, "away from the post");
}

// ---------------------------------------------------------------------------------------------- Quenelle

#[test]
fn quenelle_locks_on_after_eight_seconds_of_light_and_strains_toward_it_while_it_is_lit() {
    let mut x = Bench::new("quenelle");
    x.t.hidden = false;
    x.t.flashlight_on = true;
    x.t.lamp_charge = 1.0;
    assert!(x.until_mech(10.0, "lock_on"), "eight seconds of continuous light");
    x.step(1.6);
    assert_eq!(x.mech(), Some("torch_frenzy"), "locked on and the torch is lit");
    x.t.flashlight_on = false;
    x.step(DT);
    assert_eq!(x.mech(), None, "the light went out");
}

// ---------------------------------------------------------------------------------------------- Ravelle

#[test]
fn ravelle_crouches_to_lay_wires_and_punches_the_air_when_one_is_hit() {
    let mut x = Bench::new("ravelle");
    x.frame();
    assert_eq!(x.mech(), Some("lay_wire"), "the first wires go down at once");
    x.step(1.5);
    assert_eq!(x.mech(), None);
    // the runner walks into a wire
    let mid = {
        let Gimmick::Ravelle(r) = &x.b.gimmick else { panic!() };
        let w = &r.wires[0];
        ((w.a.0 + w.b.0) / 2.0, (w.a.1 + w.b.1) / 2.0)
    };
    x.t.pos = mid;
    x.t.hidden = false;
    x.t.in_safe_zone = false;
    let ev = x.frame();
    assert!(ev.iter().any(|e| matches!(e, Event::Stun(_))), "the wire stuns the runner");
    assert_eq!(x.mech(), Some("tripwire_snap"));
    x.runner_far();
    x.step(1.2);
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Thessine

#[test]
fn thessine_cups_a_hand_to_her_mouth_when_she_throws_a_fake_sound() {
    let mut x = Bench::new("thessine");
    let mut thrown = false;
    for _ in 0..(300.0 / DT) as usize {
        let ev = x.frame();
        if ev.iter().any(|e| matches!(e, Event::Mimic { .. })) {
            assert_eq!(x.mech(), Some("mimic_cue"), "the frame the cue is sent");
            thrown = true;
            break;
        }
    }
    assert!(thrown, "she mimics something within five minutes");
    x.step(1.5);
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Umbriel

#[test]
fn umbriel_goes_rigid_when_she_changes_bodies_and_comes_back_to_herself() {
    let mut x = Bench::new("umbriel");
    assert!(x.b.disguise.is_none());
    assert!(x.until_mech(31.0, "shapeshift"), "every thirty seconds");
    assert!(x.b.disguise.is_some(), "in her new body");
    x.step(1.4);
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Verrine

#[test]
fn verrine_crawls_low_while_the_runner_is_in_the_vents_and_stops_when_they_leave() {
    let mut x = Bench::new("verrine");
    x.t.in_vent = true;
    x.t.vent_time_in = 3.0;
    x.frame();
    assert_eq!(x.mech(), Some("vent_stalk"));
    x.t.in_vent = false;
    x.frame();
    assert_eq!(x.mech(), None);
}

// ---------------------------------------------------------------------------------------------- Wynelle

#[test]
fn wynelle_rises_beside_the_runner_casts_the_freeze_and_lets_go_when_they_warm() {
    let mut x = Bench::new("wynelle");
    x.b.active = false;
    x.t.hidden = false;
    x.t.temperature = 10.0;
    assert!(x.until(130.0, |x| x.b.active), "a cold runner draws her in");
    assert_eq!(x.mech(), Some("materialize"));
    x.step(1.4);
    assert_eq!(x.mech(), Some("freeze_cast"), "the ten second clock");
    x.heater_thawed = true;
    x.frame();
    assert_eq!(x.mech(), None, "the heat reached the runner: she is gone");
}

// ---------------------------------------------------------------------------------------------- Zellia

#[test]
fn zellia_holds_her_hands_out_while_the_shop_is_open() {
    let mut x = Bench::new("zellia");
    x.frame();
    x.t.pos = x.b.pos;
    x.t.hidden = false;
    if let Gimmick::Zellia(z) = &mut x.b.gimmick {
        z.shop_open = true;
    }
    assert_eq!(x.mech(), Some("trade_offer"));
    x.runner_far();
    x.t.hidden = false;
    x.frame();
    assert_eq!(x.mech(), None, "the runner walked off and the shop closed");
}

// ---------------------------------------------------------------------------------------------- the Voids

#[test]
fn void_black_is_rigid_while_watched() {
    let mut x = Bench::new("void_black");
    let (west, east) = x.clear_pair(4.0);
    x.place(east);
    x.t = crate::hunters::brain::Target { pos: west, facing: (1.0, 0.0), hidden: false, ..Default::default() };
    x.frame();
    assert_eq!(x.mech(), Some("watched_freeze"));
    x.t.facing = (-1.0, 0.0);
    x.frame();
    assert_eq!(x.mech(), None, "the runner looked away");
}

#[test]
fn void_blue_throws_an_arm_up_when_the_beam_reveals_her() {
    let mut x = Bench::new("void_blue");
    x.beam_on_hunter(4.0);
    x.frame();
    assert_eq!(x.mech(), Some("reveal_flinch"));
    x.runner_far();
    x.step(1.2);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_brown_thrusts_both_palms_when_she_jams_the_hiding_spots_near_the_runner() {
    let mut x = Bench::new("void_brown");
    let near = x.fac.hiding[0].cell.center();
    x.t.pos = near;
    x.t.hidden = false;
    assert!(x.until_mech(20.0, "jam_cast"), "every 18 seconds");
    assert!(x.hiding_disabled.iter().any(|d| *d), "the spots are jammed");
    x.step(1.3);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_cyan_raises_a_hand_to_her_ear_while_she_holds_sight_and_throws_both_arms_up_at_the_ping() {
    let mut x = Bench::new("void_cyan");
    x.beam_on_hunter(4.0);
    x.t.flashlight_on = false;
    assert!(x.until_mech(3.0, "signal_lock"), "holding sight of the runner");
    assert!(x.until_mech(3.0, "signal_ping"), "2.5 s of sight sends the ping");
    x.runner_far();
    x.step(1.6);
    assert_eq!(x.mech(), None, "the ping is spent and the cooldown runs");
}

#[test]
fn void_green_rises_from_a_crouch_when_she_comes_back_out_of_the_vents() {
    let mut x = Bench::new("void_green");
    assert!(x.until_mech(30.0, "vent_surface"), "22 s, a 2 s dive, then she surfaces");
    x.step(1.4);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_orange_claws_the_air_when_she_knocks_a_system_offline() {
    let mut x = Bench::new("void_orange");
    assert!(x.until_mech(28.0, "sabotage_cast"), "every 26 seconds");
    assert!(x.events.iter().any(|e| matches!(e, Event::SystemOutage(_))));
    x.step(1.7);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_pink_cups_her_ear_when_she_sends_fake_footsteps() {
    let mut x = Bench::new("void_pink");
    assert!(x.until_mech(200.0, "footstep_trick"), "a 60% roll every 11 seconds");
    assert!(x.events.iter().any(|e| matches!(e, Event::FakeFootsteps(_))));
    x.step(1.4);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_purple_spreads_her_hands_when_she_corrupts_the_radar() {
    let mut x = Bench::new("void_purple");
    assert!(x.until_mech(23.0, "static_cast"), "every 21 seconds");
    assert!(x.events.iter().any(|e| matches!(e, Event::RadarCorrupt(_))));
    x.step(1.7);
    assert_eq!(x.mech(), None);
}

#[test]
fn void_white_covers_her_face_while_the_beam_holds_her_and_stops_when_it_is_off() {
    let mut x = Bench::new("void_white");
    x.beam_on_hunter(4.0);
    x.frame();
    assert_eq!(x.mech(), Some("beam_flinch"));
    x.t.flashlight_on = false;
    x.frame();
    assert_eq!(x.mech(), None);
}

// ------------------------------------------------------------------------------------ the brain's side

#[test]
fn a_stun_cuts_a_cue_short_and_it_does_not_come_back() {
    let mut x = Bench::new("thessine");
    x.b.raise_cue("mimic_cue");
    assert_eq!(x.mech(), Some("mimic_cue"));
    x.b.apply_stun(1.0, false);
    assert_eq!(x.mech(), None);
    x.step(1.5);
    assert_eq!(x.mech(), None, "nor after she gets up");
}

#[test]
fn a_cue_runs_out_by_itself_even_while_the_hunter_is_dormant_or_stunned() {
    let mut x = Bench::new("thessine");
    x.b.raise_cue("shapeshift");
    x.b.active = false;
    x.step(1.5);
    assert_eq!(x.mech(), None, "the clock ticks on a hunter that is not active");
}

#[test]
fn a_name_that_is_not_a_cue_is_ignored() {
    let mut x = Bench::new("thessine");
    x.b.raise_cue("not_a_cue");
    x.b.raise_cue("steal_battery");
    assert_eq!(x.mech(), None);
}

#[test]
fn the_newest_cue_wins() {
    let mut x = Bench::new("thessine");
    x.b.raise_cue("mimic_cue");
    x.b.raise_cue("shapeshift");
    assert_eq!(x.mech(), Some("shapeshift"));
}

#[test]
fn a_guest_plays_the_name_the_host_sent_and_lets_go_when_frames_stop() {
    let mut x = Bench::new("ysolde");
    x.b.set_net_mechanic(Some("pocket_battery"));
    assert_eq!(x.mech(), Some("pocket_battery"));
    x.b.tick_net_mechanic(0.3);
    assert_eq!(x.mech(), Some("pocket_battery"), "kept past a frame or two");
    x.b.tick_net_mechanic(0.5);
    assert_eq!(x.mech(), None, "and dropped when the host's frames stop");
    x.b.set_net_mechanic(Some("pocket_battery"));
    x.b.set_net_mechanic(None);
    assert_eq!(x.mech(), None, "a frame with no name clears it at once");
    x.b.set_net_mechanic(Some("name_from_a_newer_host"));
    assert_eq!(x.mech(), None, "a name this build does not know is ignored");
}




#[test]
fn a_guests_idle_gimmick_never_speaks_only_the_hosts_name_does() {
    // Glyra's gimmick sits at "the box is playing" until a host frame says otherwise: on a guest that must not show
    let mut x = Bench::new("glyra");
    x.step(0.2);
    assert_eq!(x.mech(), Some("music_box_hold"), "on the host she cradles the box");
    x.b.display_only = true;
    assert_eq!(x.mech(), None, "a guest's copy shows nothing of its own");
    x.b.raise_cue("mimic_cue");
    assert_eq!(x.mech(), None, "not even a stray cue");
    x.b.set_net_mechanic(Some("music_box_startle"));
    assert_eq!(x.mech(), Some("music_box_startle"), "only what the host sent");
    let mut y = Bench::new("glyra");
    y.step(0.2);
    y.b.shadow = true;
    assert_eq!(y.mech(), None, "a shadow copy made from a host frame is a guest's too");
}
