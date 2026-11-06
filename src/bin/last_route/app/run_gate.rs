//! What the pause menu does to the world, and what a long frame does to a hosting client.
//!
//! * Pause. A single-player run stops under the pause menu (the world is not stepped at all). A run in an open room
//!   (`net` is `Some`: the host of the public route, or a guest of any room) never stops: Godot builds its pause menu
//!   with `pauses_tree = not RoomState.online` (main_3d.gd 124), so the hunters, timers, meters and the net broadcast go
//!   on, and the pausing runner is only uncontrolled: it stands in the world with blank controls and can be caught
//!   (`_player_controls_blocked` freezes input, not the body). [`controls_for`] is the one test.
//! * A long frame. The window loop caps one frame's step at [`MAX_FRAME_DT`], so a hitch is never one huge step that
//!   tunnels a hunter through a wall. For a client that hosts a room the stall would otherwise be lost for every guest
//!   (their hunters are the host's), so [`extra_steps`] asks for a few more capped steps, at most
//!   `data/net/sync.json` `host_catch_up_steps`. A tab the browser kept in the background resumes with one frame as long
//!   as the time it was hidden (requestAnimationFrame stops): the host then runs the hunter simulation for that capped
//!   catch-up, never skipping it, and the rest of the stall is dropped, like the dedicated room's loop does.

use crate::game::Controls;

/// The longest step one frame simulates (the window loop's `dt` cap).
pub const MAX_FRAME_DT: f32 = 0.05;

/// The controls the world is stepped with this frame, or `None` when the world is not stepped at all (a single-player
/// run under the pause menu). In an open room the pause menu only blanks the runner's controls.
pub fn controls_for(paused: bool, online: bool, c: &Controls) -> Option<Controls> {
    match (paused, online) {
        (false, _) => Some(c.clone()),
        (true, true) => Some(Controls::default()),
        (true, false) => None,
    }
}

/// Extra capped steps to run before the frame's own, for a frame that took `raw_dt` seconds. Only a hosting client asks
/// for them (`hosting`), at most `max_extra`; a frame within the cap needs none. The remainder of a long stall is
/// dropped.
pub fn extra_steps(raw_dt: f32, hosting: bool, max_extra: u32) -> u32 {
    if !hosting || !raw_dt.is_finite() || raw_dt <= MAX_FRAME_DT {
        return 0;
    }
    (((raw_dt - MAX_FRAME_DT) / MAX_FRAME_DT).floor() as u32).min(max_extra)
}

/// The controls of a catch-up step: what is held (walking, sprinting, crouching, the interact hold) carries on, and
/// everything pressed or turned for one frame belongs to the frame's own step only, so a stall never repeats a click.
pub fn held_only(c: &Controls) -> Controls {
    Controls {
        fwd: c.fwd,
        strafe: c.strafe,
        sprint: c.sprint,
        crouch: c.crouch,
        interact_held: c.interact_held,
        ..Controls::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Game;
    use crate::hunters::brain::State;
    use crate::world::catalog::Catalog;

    const DT: f32 = 1.0 / 60.0;

    fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    /// A run with one active hunter in chase, `gap` cells from the standing runner, on open floor.
    fn staged(gap: f32) -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-PAUSE-7", 0, &content, &catalog);
        g.relays_collected = 1;
        g.hunters.clear();
        assert!(crate::app::stress::summon(&mut g, "elise", &content, 0, 0));
        let want = (g.p.pos.0 + gap, g.p.pos.1);
        let at = g.fac.nearest_open(want.0, want.1, crate::config::HUNTER_RADIUS).expect("open floor near the runner");
        g.hunters[0].pos = at;
        g.hunters[0].state = State::Chase;
        g.hunters[0].active = true;
        (g, content, catalog)
    }

    /// `frames` window frames with the pause menu open (`paused`), the way the loop runs them: the gate decides, then the
    /// world steps. Returns how many frames stepped the world.
    fn frames(g: &mut Game, content: &Content, catalog: &Catalog, n: u32, paused: bool, online: bool) -> u32 {
        frames_with(g, content, catalog, n, paused, online, Controls { fwd: 1.0, sprint: true, ..Controls::default() })
    }

    /// As `frames`, with the controls the player holds (a paused menu must ignore them).
    fn frames_with(g: &mut Game, content: &Content, catalog: &Catalog, n: u32, paused: bool, online: bool, held: Controls) -> u32 {
        let sounds = Sounds::default();
        let mut stepped = 0;
        for _ in 0..n {
            // the window loop blanks the controls under the menu; the gate must not need that to hold
            // the run starts in the lobby's safe zone, where nothing is caught, and the streamed window puts it back
            g.fac.safe_zone = None;
            if let Some(c) = controls_for(paused, online, &held) {
                g.step(DT, &c, content, catalog, &sounds);
                stepped += 1;
            }
        }
        stepped
    }

    #[test]
    fn the_gate_stops_a_solo_run_under_the_menu_and_nothing_else() {
        let c = Controls { fwd: 1.0, ..Controls::default() };
        assert!(controls_for(false, false, &c).is_some_and(|k| k.fwd == 1.0), "an open game takes its input");
        assert!(controls_for(false, true, &c).is_some_and(|k| k.fwd == 1.0), "and so does an open room's");
        assert!(controls_for(true, false, &c).is_none(), "a paused solo run is not stepped");
        let online = controls_for(true, true, &c).expect("a paused room's world still steps");
        assert_eq!((online.fwd, online.strafe, online.sprint, online.interact, online.jump), (0.0, 0.0, false, false, false), "its runner is uncontrolled");
    }

    #[test]
    fn online_every_hunter_keeps_moving_while_the_menu_is_open() {
        let (mut g, content, catalog) = staged(9.0);
        let before = g.hunters[0].pos;
        let stepped = frames(&mut g, &content, &catalog, 90, true, true);
        assert_eq!(stepped, 90, "every paused frame stepped the world");
        assert!(dist(g.hunters[0].pos, before) > 0.5, "the hunter moved: {:?} -> {:?}", before, g.hunters[0].pos);
        assert!(g.time > 1.4, "the run's clock ran too: {}", g.time);
    }

    #[test]
    fn online_a_hunter_attacks_the_runner_who_stands_in_the_pause_menu() {
        let (mut g, content, catalog) = staged(1.5);
        let start = g.p.pos;
        frames(&mut g, &content, &catalog, 600, true, true);
        assert!(g.escape.is_some() || g.report.is_some(), "the standing runner was caught while paused");
        assert!(dist(g.p.pos, start) < 1.0, "and it never walked: the menu blanked its controls");
    }

    #[test]
    fn solo_nothing_moves_or_attacks_while_the_menu_is_open() {
        let (mut g, content, catalog) = staged(1.5);
        let (hunter, time) = (g.hunters[0].pos, g.time);
        let stepped = frames(&mut g, &content, &catalog, 600, true, false);
        assert_eq!(stepped, 0, "no frame stepped the world");
        assert_eq!(g.hunters[0].pos, hunter, "the hunter held still");
        assert_eq!(g.time, time, "and so did the clock");
        assert!(g.escape.is_none() && g.report.is_none(), "nothing attacked");
        // the same staging, not paused, does attack: the test would pass for the wrong reason otherwise
        let (mut g, content, catalog) = staged(1.5);
        frames_with(&mut g, &content, &catalog, 600, false, false, Controls::default());
        assert!(g.escape.is_some() || g.report.is_some(), "an unpaused runner is caught");
    }

    #[test]
    fn a_paused_guest_still_shows_the_hosts_hunters_where_the_host_put_them() {
        // a guest's world has no hunter simulation of its own: the host's frame sets every hunter, paused or not, and the
        // guest's stepping under the menu does not undo it
        let (mut host, content, catalog) = staged(9.0);
        frames(&mut host, &content, &catalog, 60, false, true);
        let (mut guest, content, catalog) = staged(9.0);
        guest.remote_hunters = true;
        let frame = host.hunter_frame();
        guest.apply_hunter_frame(&frame);
        frames(&mut guest, &content, &catalog, 60, true, true);
        guest.apply_hunter_frame(&host.hunter_frame());
        assert_eq!(guest.hunters[0].pos, host.hunters[0].pos, "no snap: the guest shows the host's hunter exactly");
        frames(&mut host, &content, &catalog, 30, true, true);
        guest.apply_hunter_frame(&host.hunter_frame());
        assert_eq!(guest.hunters[0].pos, host.hunters[0].pos, "a hunter the paused host's world moved reaches the paused guest");
    }

    #[test]
    fn only_a_hosting_client_catches_up_after_a_long_frame_and_never_more_than_the_data_allows() {
        assert_eq!(extra_steps(0.016, true, 4), 0, "a normal frame needs none");
        assert_eq!(extra_steps(MAX_FRAME_DT, true, 4), 0, "nor one at the cap");
        assert_eq!(extra_steps(0.12, true, 4), 1, "0.12 s is the capped step and one more");
        assert_eq!(extra_steps(0.3, true, 4), 4);
        assert_eq!(extra_steps(30.0, true, 4), 4, "a tab hidden for 30 s resumes with a bounded catch-up");
        assert_eq!(extra_steps(30.0, true, 0), 0, "0 turns it off");
        assert_eq!(extra_steps(30.0, false, 4), 0, "a guest and a solo run keep one capped step");
        assert_eq!(extra_steps(f32::NAN, true, 4), 0);
        assert_eq!(extra_steps(f32::INFINITY, true, 4), 0);
    }

    #[test]
    fn a_catch_up_step_repeats_what_is_held_and_nothing_pressed() {
        let c = Controls { fwd: 1.0, strafe: -1.0, sprint: true, crouch: true, interact_held: true, interact: true, jump: true, taser: true, flashlight: true, look: macroquad::math::vec2(3.0, 1.0), slot: Some(2), ..Controls::default() };
        let h = held_only(&c);
        assert_eq!((h.fwd, h.strafe, h.sprint, h.crouch, h.interact_held), (1.0, -1.0, true, true, true));
        assert!(!h.interact && !h.jump && !h.taser && !h.flashlight && h.slot.is_none() && h.look == macroquad::math::Vec2::ZERO);
    }

    #[test]
    fn the_data_file_keeps_the_catch_up_a_small_whole_number() {
        let r = crate::net::interp::rules();
        assert_eq!(r.host_catch_up_steps, 4);
        let off = crate::net::interp::Rules::parse(r#"{"schema":"lr.net_sync/1","host_catch_up_steps":0}"#).unwrap();
        assert_eq!(off.host_catch_up_steps, 0);
        assert!(crate::net::interp::Rules::parse(r#"{"schema":"lr.net_sync/1","host_catch_up_steps":-1}"#).is_err());
    }

    #[test]
    fn a_paused_room_world_is_the_same_world_as_an_unpaused_one_with_nobody_at_the_keys() {
        // the menu must change nothing but the runner's input: same hunters, same clock, same runner, frame for frame
        let (mut a, content, catalog) = staged(6.0);
        let (mut b, _, _) = staged(6.0);
        frames(&mut a, &content, &catalog, 120, true, true);
        frames_with(&mut b, &content, &catalog, 120, false, true, Controls::default());
        assert_eq!(a.hunters[0].pos, b.hunters[0].pos, "paused and idle hunters agree");
        assert_eq!(a.p.pos, b.p.pos, "and so does the runner");
        assert_eq!(a.time, b.time);
    }

    #[test]
    fn the_catch_up_never_exceeds_the_cap_for_any_frame_length_and_never_shrinks_as_the_stall_grows() {
        let mut last = 0;
        for i in 0..4000 {
            let raw = i as f32 * 0.01;
            let n = extra_steps(raw, true, 4);
            assert!(n <= 4 && n >= last, "{raw}: {n} after {last}");
            last = n;
        }
        for bad in [f32::NEG_INFINITY, -1.0, -0.0, 0.0, f32::MIN, f32::MAX] {
            assert!(extra_steps(bad, true, 20) <= 20, "{bad}");
        }
    }

    #[test]
    fn a_hostile_catch_up_value_is_bounded_or_refused() {
        let parse = |v: &str| crate::net::interp::Rules::parse(&format!(r#"{{"schema":"lr.net_sync/1","host_catch_up_steps":{v}}}"#));
        assert_eq!(parse("1000000").unwrap().host_catch_up_steps, 20, "a huge value is clamped, never a stall of its own");
        assert_eq!(parse("2.9").unwrap().host_catch_up_steps, 2);
        assert!(parse("1e999").is_err() && parse("\"4\"").is_err() && parse("null").is_err());
    }
}
