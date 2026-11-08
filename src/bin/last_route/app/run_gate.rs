//! What the pause menu does to the world, and what a long frame does to a hosting client.
//!
//! * Pause. A single-player run stops under the pause menu (the world is not stepped at all). A run in an open room
//!   (`net` is `Some`: the host of the public route, or a guest of any room) never stops: Godot builds its pause menu
//!   with `pauses_tree = not RoomState.online` (main_3d.gd 124), so the hunters, timers, meters and the net broadcast go
//!   on, and the pausing runner is only uncontrolled: it stands in the world with blank controls and can be caught
//!   (`_player_controls_blocked` freezes input, not the body). [`controls_for`] is the one test.
//! * A grab in an open room. The pausing runner can be caught (above), and the struggle (mash [E] or the right button) is
//!   input the paused menu blanks, so [`hand_back_on_grab`] closes the menu (every sub-screen, a pending quit question
//!   included: the grab wins) the moment the local runner is held, and [`pause_allowed`] keeps Escape (and the web's
//!   lost pointer lock) from opening it again until the struggle is over (Escape is ignored while held, online). A browser only grants the pointer lock on a user gesture, so the hand-back never asks for one:
//!   [`PointerWait`] leaves the cursor free after a close that could not take the lock and waits for the next click, the
//!   struggle works from the keyboard meanwhile, and nothing retries on a timer.
//! * A long frame. The window loop caps one frame's step at [`MAX_FRAME_DT`], so a hitch is never one huge step that
//!   tunnels a hunter through a wall. For a client that hosts a room the stall would otherwise be lost for every guest
//!   (their hunters are the host's), so [`extra_steps`] asks for a few more capped steps, at most
//!   `data/net/sync.json` `host_catch_up_steps`. A tab the browser kept in the background resumes with one frame as long
//!   as the time it was hidden (requestAnimationFrame stops): the host then runs the hunter simulation for that capped
//!   catch-up, never skipping it, and the rest of the stall is dropped, like the dedicated room's loop does.

use crate::game::Controls;
use crate::ui::pause::Pause;

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

/// Whether Escape (or a browser-dropped pointer lock) may open the pause menu: not for a runner an online hunter is holding,
/// whose struggle needs the keys. A single-player run cannot be grabbed while the menu is open, and its menu is always allowed.
pub fn pause_allowed(online: bool, held_by_hunter: bool) -> bool {
    !(online && held_by_hunter)
}

/// What closing the pause menu on a grab left for the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandBack {
    /// The menu was open and has been closed.
    pub closed: bool,
    /// The pointer lock was not held at that moment: the caller must not ask the browser for it (no user gesture) and waits
    /// for a click instead ([`PointerWait`]).
    pub wait_for_click: bool,
}

/// An open room's hunter holds the local runner (`held_by_hunter`: the escape struggle is on) while the pause menu is open:
/// close the menu, with its settings, players, dossier, skin or confirm screen, back to the plain game. `lock_held` is
/// whether the pointer is really locked now (always true off the web).
pub fn hand_back_on_grab(pz: &mut Pause, online: bool, held_by_hunter: bool, lock_held: bool) -> HandBack {
    if !(online && held_by_hunter && pz.open) {
        return HandBack { closed: false, wait_for_click: false };
    }
    *pz = Pause::default();
    HandBack { closed: true, wait_for_click: !lock_held }
}

/// The pointer lock the game has given up without a user gesture to take it back with. While it is set the cursor stays
/// free and the lock is not requested; the next click clears it and re-locks as it always did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerWait(bool);

impl PointerWait {
    pub fn waiting(self) -> bool {
        self.0
    }

    /// Whether this frame's click is the one that takes the lock back (call before [`Self::allows_grab`]): it is a
    /// gesture for the browser, not a use of the held item, so the caller drops the frame's `use_item`.
    pub fn relock_click(self, click: bool) -> bool {
        self.0 && click
    }

    pub fn start(&mut self) {
        self.0 = true;
    }

    /// One frame: `want_grab` is what the screen would capture, `click` a mouse press, `menu_or_report` a pause menu or
    /// the run report on screen (the menu's own clicks are gestures and the report frees the pointer anyway, so the wait
    /// ends). Returns whether the pointer may be captured this frame.
    pub fn allows_grab(&mut self, want_grab: bool, click: bool, menu_or_report: bool) -> bool {
        if menu_or_report || click {
            self.0 = false;
        }
        want_grab && !self.0
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
        assert!(g.escape.is_some() || g.report.is_some() || g.runner_downed(), "the standing runner was caught while paused");
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
        assert!(g.escape.is_some() || g.report.is_some() || g.runner_downed(), "an unpaused runner is caught");
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

    // ---- a hunter's grab while the pause menu is open in a room ----

    use crate::ui::pause::Sub;

    /// What the window loop does each frame around the gate in an online room: hand back on a grab, then step the world with
    /// the controls the (possibly just closed) menu leaves. `pressed` is the player's E press this frame.
    fn room_frame(g: &mut Game, content: &Content, catalog: &Catalog, pz: &mut Pause, wait: &mut PointerWait, lock: bool, pressed: bool) -> HandBack {
        let sounds = Sounds::default();
        g.fac.safe_zone = None;
        let hb = hand_back_on_grab(pz, true, g.escape.is_some(), lock);
        if hb.wait_for_click {
            wait.start();
        }
        // the loop blanks the controls under an open menu, then the gate decides
        let c = if pz.open { Controls::default() } else { Controls { interact: pressed, ..Controls::default() } };
        if let Some(step_c) = controls_for(pz.open, true, &c) {
            g.step(DT, &step_c, content, catalog, &sounds);
        }
        hb
    }

    fn open_menu(sub: Sub, tab: usize) -> Pause {
        let mut pz = Pause::default();
        pz.show();
        pz.sub = sub;
        pz.tab = tab;
        pz
    }

    #[test]
    fn a_grab_closes_every_screen_of_the_online_pause_menu() {
        for (sub, tab) in [(Sub::None, 0), (Sub::None, 1), (Sub::None, 2), (Sub::Settings, 0), (Sub::Skin, 0), (Sub::Dossier, 0), (Sub::Confirm, 0)] {
            let mut pz = open_menu(sub, tab);
            pz.copied = "ROOM".into();
            let hb = hand_back_on_grab(&mut pz, true, true, true);
            assert!(hb.closed && !hb.wait_for_click, "{sub:?}/{tab}: closed, and a held lock needs no click");
            assert!(!pz.open && pz.sub == Sub::None && pz.tab == 0 && pz.copied.is_empty(), "{sub:?}/{tab}: a fresh menu state");
        }
        // the quit question is cancelled by the grab, not answered
        let mut pz = open_menu(Sub::Confirm, 0);
        assert!(hand_back_on_grab(&mut pz, true, true, true).closed);
        assert_eq!(pz.sub, Sub::None, "the grab wins over a pending leave-the-route question");
    }

    #[test]
    fn nothing_closes_without_a_room_a_grab_or_an_open_menu() {
        for (online, held, open) in [(false, true, true), (true, false, true), (true, true, false), (false, false, false)] {
            let mut pz = Pause::default();
            if open {
                pz = open_menu(Sub::Settings, 1);
            }
            let hb = hand_back_on_grab(&mut pz, online, held, false);
            assert_eq!(hb, HandBack { closed: false, wait_for_click: false }, "online {online} held {held} open {open}");
            assert_eq!(pz.open, open, "the menu is left as it was");
            if open {
                assert!(pz.sub == Sub::Settings && pz.tab == 1, "its screen too");
            }
        }
    }

    #[test]
    fn a_close_without_the_pointer_lock_waits_for_a_click_and_never_asks_on_a_timer() {
        let mut pz = open_menu(Sub::None, 0);
        let hb = hand_back_on_grab(&mut pz, true, true, false);
        assert!(hb.closed && hb.wait_for_click, "a browser without the lock gives none back without a gesture");
        let mut wait = PointerWait::default();
        assert!(!wait.waiting());
        wait.start();
        // thousands of frames of a free cursor: the pointer is never captured, so nothing is requested (no retry loop)
        for _ in 0..5000 {
            assert!(!wait.allows_grab(true, false, false), "no lock request without a click");
            assert!(wait.waiting());
        }
        // a click is the gesture: the wait ends and that frame captures, then it stays captured
        assert!(wait.allows_grab(true, true, false), "the click takes the lock");
        assert!(!wait.waiting());
        assert!(wait.allows_grab(true, false, false));
        // a screen that does not capture stays uncaptured whatever the wait says
        wait.start();
        assert!(!wait.allows_grab(false, true, false));
        // a pause menu or the report ends the wait too (their own clicks are gestures)
        wait.start();
        assert!(!wait.allows_grab(false, false, true) && !wait.waiting());
    }

    #[test]
    fn escape_is_ignored_while_held_online_and_works_otherwise() {
        assert!(!pause_allowed(true, true), "no pause menu over a struggle in a room");
        assert!(pause_allowed(true, false), "an online runner who is free may pause");
        assert!(pause_allowed(false, true), "a solo run is never held while the menu could open");
        assert!(pause_allowed(false, false));
    }

    #[test]
    fn a_grab_under_the_open_menu_closes_it_and_the_struggle_works_from_the_keyboard() {
        let (mut g, content, catalog) = staged(1.5);
        let mut pz = open_menu(Sub::Settings, 0);
        let mut wait = PointerWait::default();
        let mut frames = 0;
        // the runner stands in the open menu until the hunter has her
        while g.escape.is_none() && g.report.is_none() && frames < 600 {
            let hb = room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, false, false);
            assert!(!hb.closed, "the menu stays up until there is a grab");
            frames += 1;
        }
        assert!(g.escape.is_some(), "the hunter grabbed the standing runner");
        assert!(pz.open, "the grab began inside the step: the menu is still up this frame");
        // the very next frame hands control back
        let hb = room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, false, false);
        assert!(hb.closed && hb.wait_for_click && !pz.open && wait.waiting());
        assert!(!wait.allows_grab(true, false, false), "and the cursor stays free: no lock is requested without a click");
        // mashing E (no pointer lock, no click needed) breaks the hold
        let mut presses = 0;
        while g.escape.is_some() && presses < 6 {
            let before = g.escape.as_ref().unwrap().progress;
            room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, false, true);
            presses += 1;
            if let Some(e) = &g.escape {
                assert_eq!(e.progress, before + 1, "every press counts");
            }
        }
        assert!(g.escape.is_none() && g.report.is_none(), "six presses broke free");
    }

    #[test]
    fn without_the_hand_back_a_menu_that_stays_open_loses_the_struggle() {
        // the old behaviour, to prove the test above is not passing by accident: the menu blanks every press
        let (mut g, content, catalog) = staged(1.5);
        let sounds = Sounds::default();
        let mut frames = 0;
        while g.escape.is_none() && frames < 600 {
            g.fac.safe_zone = None;
            g.step(DT, &Controls::default(), &content, &catalog, &sounds);
            frames += 1;
        }
        assert!(g.escape.is_some());
        for _ in 0..400 {
            g.fac.safe_zone = None;
            let c = controls_for(true, true, &Controls { interact: true, ..Controls::default() }).unwrap();
            g.step(DT, &c, &content, &catalog, &sounds);
        }
        // a lost struggle takes the runner down first (game/revive.rs): the run ends only at the revive timeout
        assert!(g.report.is_some() || g.runner_downed(), "the blanked presses never broke the hold: the runner went down");
    }

    #[test]
    fn a_runner_pulled_out_of_a_locker_under_the_menu_gets_the_menu_closed_too() {
        let (mut g, content, catalog) = staged(1.5);
        let li = 0usize;
        let centre = g.fac.hiding[li].cell.center();
        g.p.hidden = Some(li);
        g.p.pos = centre;
        g.hunters[0].pos = centre;
        g.hunters[0].last_known = Some(centre);
        let mut pz = open_menu(Sub::None, 2);
        let mut wait = PointerWait::default();
        let mut closed = false;
        for _ in 0..600 {
            let hb = room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, true, false);
            if hb.closed {
                closed = true;
                assert!(!hb.wait_for_click, "the lock is held: no click needed");
                break;
            }
            if g.report.is_some() {
                break;
            }
        }
        assert!(closed, "the pulled-out runner's grab closed the menu");
        assert!(g.p.hidden.is_none() && g.escape.is_some(), "she is out of the locker and struggling");
    }

    #[test]
    fn a_menu_opened_by_a_free_online_runner_is_left_alone() {
        let (mut g, content, catalog) = staged(30.0);
        let mut pz = open_menu(Sub::None, 1);
        let mut wait = PointerWait::default();
        for _ in 0..120 {
            let hb = room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, false, false);
            assert!(!hb.closed);
        }
        assert!(pz.open && pz.tab == 1 && !wait.waiting());
    }

    #[test]
    fn the_hand_back_fires_once_per_open_menu_so_a_relock_click_is_never_undone() {
        // the loop's per-frame order: hand back, then the wait decides the capture. The runner stays held for many frames.
        let mut pz = open_menu(Sub::Settings, 1);
        let mut wait = PointerWait::default();
        let mut grabs = Vec::new();
        for frame in 0..300 {
            let hb = hand_back_on_grab(&mut pz, true, true, false);
            if hb.wait_for_click {
                wait.start();
            }
            assert_eq!(hb.closed, frame == 0, "only the first frame closes the menu");
            // a click on frame 100 takes the lock; nothing after it may put the wait back while still held
            let click = frame == 100;
            grabs.push(wait.allows_grab(true, click, pz.open));
        }
        assert!(grabs[..100].iter().all(|g| !*g), "no capture before the click");
        assert!(grabs[100..].iter().all(|g| *g), "captured from the click on and never lost again while held");
        assert!(!wait.waiting());
    }

    #[test]
    fn the_click_that_relocks_is_not_a_use_of_the_held_item() {
        let mut wait = PointerWait::default();
        assert!(!wait.relock_click(true), "not waiting: a click is an ordinary click");
        wait.start();
        assert!(!wait.relock_click(false), "waiting, no click");
        assert!(wait.relock_click(true), "the click that ends the wait is a gesture only");
        assert!(wait.allows_grab(true, true, false));
        assert!(!wait.relock_click(true), "the next click is an ordinary click again");
    }

    #[test]
    fn a_new_wait_after_a_lost_lock_during_the_grab_keeps_the_menu_shut_and_the_struggle_alive() {
        // the web drops the lock (Escape) in the middle of the struggle: no menu (Escape is ignored while held), the cursor
        // stays free, nothing asks for the lock, and the keys still struggle
        let (mut g, content, catalog) = staged(1.5);
        let mut pz = Pause::default();
        let mut wait = PointerWait::default();
        let mut n = 0;
        while g.escape.is_none() && g.report.is_none() && n < 600 {
            room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, true, false);
            n += 1;
        }
        assert!(g.escape.is_some());
        assert!(!pause_allowed(true, g.escape.is_some()), "the lost lock may not open the menu");
        wait.start();
        for _ in 0..6 {
            assert!(!wait.allows_grab(true, false, false));
            room_frame(&mut g, &content, &catalog, &mut pz, &mut wait, false, true);
        }
        assert!(!pz.open && g.escape.is_none() && g.report.is_none(), "free again, menu still shut");
        assert!(pause_allowed(true, g.escape.is_some()), "once free, Escape pauses as always");
    }
}
