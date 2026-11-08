//! Gray Void (`void_gray`): the relay-pressure anti-idle signal (game/hunters/gimmicks/void_gray_gimmick.gd; chart entry
//! void_hunter_gimmick_chart.gd:24-27).
//!
//! She is DORMANT (unseen, still, uncatchable) while the team keeps collecting relays. A grace window (180 s, 120 s on
//! Nightmare) restarts at every relay. Once it has run out, standing still in the open (not hiding, not in a safe room)
//! for 45 s (30 s on Nightmare) opens a SIGNAL ROUTE: 5 E presses with 2.5 s for each (1.8 s on Nightmare). Clearing it
//! keeps her dormant. Missing one (or letting the route's clock run out) shows her and starts a normal chase, which only
//! the next relay resets. She chases whoever failed, regardless of light.
//!
//! The route runs on the failing player's machine: the game counts the E presses and reports back through
//! `route_result`. This brain keeps a watchdog (`route_left`) so a lost report still ends the route
//! (void_gray_gimmick.gd:10-11).
//!
//! Integration (mod.rs is not edited here; the integrator wires it):
//! - `Gimmick::VoidGray(void::void_gray::VoidGray)`, id "void_gray", built as `VoidGray::new(nightmare)`. The Rust
//!   `Difficulty` has no Nightmare yet, so pass `false` until it does.
//! - `before_perception` -> `before(b, dt, c, t, peer_id)`, then drain `take_events()`. `peer_id` is the runner's peer
//!   (Godot `target.get("peer_id", 1)`); the Rust `Target` carries no peer id.
//! - `after_perception` -> `after(b)`.
//! - `holds_position()`, `immune_to_capture()`, `hidden_from_view()` are the Godot `dormant and not hunting` checks.
//! - Speed and sight scales are 1.0 in the gimmick. The chart's "long-sighted" comes from the roster data.
//! - Route prompts (the E presses) and the report back are the player's side: on `SignalRoute` the failing runner
//!   presses E `nodes` times, each within `seconds`, then calls `route_result(b, ok)`.

use crate::hunters::brain::{dist, Brain, State, Target};
use crate::hunters::gimmicks::Ctx;

/// void_gray_gimmick.gd:13: seconds of standing still (in the open) before the route opens.
const IDLE_SECONDS: f32 = 45.0;
/// void_gray_gimmick.gd:14: the idle limit on Nightmare.
const NIGHTMARE_IDLE_SECONDS: f32 = 30.0;
/// void_gray_gimmick.gd:15: E presses in one route.
const NODES: u32 = 5;
/// void_gray_gimmick.gd:16: seconds allowed per E press.
const SECONDS_PER_NODE: f32 = 2.5;
/// void_gray_gimmick.gd:17: seconds per E press on Nightmare.
const NIGHTMARE_SECONDS_PER_NODE: f32 = 1.8;
/// void_gray_gimmick.gd:18: the relay grace window.
const RELAY_GRACE_SECONDS: f32 = 180.0;
/// void_gray_gimmick.gd:19: the relay grace window on Nightmare.
const NIGHTMARE_RELAY_GRACE_SECONDS: f32 = 120.0;
/// void_gray_gimmick.gd:20-21: slower than this (cells/s) counts as standing still.
const STILL_SPEED: f32 = 0.15;
/// void_gray_gimmick.gd:22: extra seconds the route's watchdog allows for the report to arrive.
const REPORT_SLACK: f32 = 3.0;
/// void_gray_gimmick.gd:23: a hunting Gray stays locked on while the runner is within this many cells of where she woke.
const LOCK_RELEASE_DISTANCE: f32 = 5.0;

/// The world events Gray raises. `SignalRoute` is the only one that survives the callout rule: the Godot
/// `emit_world_event` drops every `message` and strips `sound: "alert"` while `ANNOUNCE_HUNTER_EVENTS` is off
/// (hunter_brain.gd:88-95). Gray's "GRAY PRESSURE RESET", "SIGNAL ROUTE CLEARED" and "SIGNAL ROUTE FAILED" messages are
/// therefore not emitted at all here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VoidGrayEvent {
    /// `{what: "signal_route", nodes, seconds, peer_id}` (void_gray_gimmick.gd:111-112), after the callout strip.
    SignalRoute {
        nodes: u32,
        seconds: f32,
        peer_id: usize,
    },
}

pub struct VoidGray {
    nightmare: bool,
    /// void_gray_gimmick.gd `hunting`: she has shown herself and chases.
    pub hunting: bool,
    /// void_gray_gimmick.gd `locked`: she knows the runner's position and keeps it while they stay near `lock_origin`.
    pub locked: bool,
    /// void_gray_gimmick.gd `dormant`: unseen and still. Starts true.
    pub dormant: bool,
    /// void_gray_gimmick.gd `route_active`: a signal route is open.
    pub route_active: bool,
    /// void_gray_gimmick.gd `route_peer`: the runner the route is for.
    pub route_peer: usize,
    /// void_gray_gimmick.gd `route_nodes`.
    pub route_nodes: u32,
    /// void_gray_gimmick.gd `route_seconds`: seconds per E press.
    pub route_seconds: f32,
    /// void_gray_gimmick.gd `_grace`: the relay grace accumulated, capped at the grace limit.
    grace: f32,
    /// void_gray_gimmick.gd `_idle`: seconds the runner has been still in the open.
    idle: f32,
    /// void_gray_gimmick.gd `_route_left`: the watchdog clock of the open route.
    route_left: f32,
    /// E presses landed on the open route.
    route_done: u32,
    /// Seconds left for the next E press.
    node_left: f32,
    /// void_gray_gimmick.gd `_relays`: the relay count last seen. None before the first tick (the Godot -1).
    relays: Option<usize>,
    /// void_gray_gimmick.gd `_last_position`: None is Vector2.INF.
    last_position: Option<(f32, f32)>,
    /// void_gray_gimmick.gd `_last_target_position`.
    last_target_position: (f32, f32),
    /// void_gray_gimmick.gd `_lock_origin`.
    lock_origin: (f32, f32),
    events: Vec<VoidGrayEvent>,
}

impl Default for VoidGray {
    fn default() -> Self {
        Self::new(false)
    }
}

/// void_gray_gimmick.gd `idle_limit` (lines 44-45).
pub fn idle_limit(nightmare: bool) -> f32 {
    if nightmare {
        NIGHTMARE_IDLE_SECONDS
    } else {
        IDLE_SECONDS
    }
}

/// void_gray_gimmick.gd `grace_limit` (lines 48-49).
pub fn grace_limit(nightmare: bool) -> f32 {
    if nightmare {
        NIGHTMARE_RELAY_GRACE_SECONDS
    } else {
        RELAY_GRACE_SECONDS
    }
}

/// void_gray_gimmick.gd `node_seconds` (lines 52-53).
pub fn node_seconds(nightmare: bool) -> f32 {
    if nightmare {
        NIGHTMARE_SECONDS_PER_NODE
    } else {
        SECONDS_PER_NODE
    }
}

/// void_gray_gimmick.gd `_fail` also restarts the chase at the last known spot. The chase reset mirrors
/// hunter_brain.gd `continue_chase_from` (lines 533-542).
fn continue_chase_from(b: &mut Brain, at: (f32, f32)) {
    b.state = State::Chase;
    b.last_known = Some(at);
    b.lost_sight = 0.0;
    b.search_timer = 0.0;
    b.search_target = None;
    b.clear_path();
    b.direct_visible = false;
}

impl VoidGray {
    /// The window moved by `d`.
    pub fn shift(&mut self, d: (f32, f32)) {
        let s = |p: (f32, f32)| (p.0 + d.0, p.1 + d.1);
        self.last_position = self.last_position.map(s);
        self.last_target_position = s(self.last_target_position);
        self.lock_origin = s(self.lock_origin);
    }

    pub fn new(nightmare: bool) -> VoidGray {
        VoidGray {
            nightmare,
            hunting: false,
            locked: false,
            dormant: true,
            route_active: false,
            route_peer: 1,
            route_nodes: NODES,
            route_seconds: node_seconds(nightmare),
            grace: 0.0,
            idle: 0.0,
            route_left: 0.0,
            route_done: 0,
            node_left: 0.0,
            relays: None,
            last_position: None,
            last_target_position: (0.0, 0.0),
            lock_origin: (0.0, 0.0),
            events: Vec::new(),
        }
    }

    /// The events raised since the last call (the integrator maps them onto the game's route prompt).
    pub fn take_events(&mut self) -> Vec<VoidGrayEvent> {
        std::mem::take(&mut self.events)
    }

    /// before_perception (void_gray_gimmick.gd:56-112). `c.relays_taken` gives the relays collected so far, which is the
    /// Godot `brain.relays_seen`. `peer_id` is the runner the route is for.
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, peer_id: usize) {
        let relays = c.relays_taken.iter().filter(|taken| **taken).count();
        self.tick(b, dt, relays, t, peer_id);
    }

    /// The body of `before`, with the relay count given directly so it can be tested without a facility.
    fn tick(&mut self, b: &mut Brain, dt: f32, relays: usize, t: &Target, peer_id: usize) {
        let nightmare = self.nightmare;
        // :58-59: the first tick only records the count (the Godot `_relays < 0` branch).
        let previous = *self.relays.get_or_insert(relays);
        self.relays = Some(relays);
        // :60-71: a relay collected (the count went up) resets the whole pressure.
        if relays > previous {
            self.grace = 0.0;
            self.idle = 0.0;
            self.hunting = false;
            self.dormant = true;
            self.route_active = false;
            b.state = State::Idle;
            b.last_known = None;
        }
        // :72-85: hunting (woken by the pressure or a failed route) follows the runner.
        if self.hunting {
            self.dormant = false;
            if self.locked {
                let at_now = t.pos;
                if dist(at_now, self.lock_origin) > LOCK_RELEASE_DISTANCE {
                    // :76-77: they moved on, so a normal hunt from here
                    self.locked = false;
                } else {
                    self.last_target_position = at_now;
                    b.lost_sight = 0.0;
                    b.last_known = Some(at_now);
                    if b.state != State::Chase {
                        continue_chase_from(b, at_now);
                    }
                }
            }
            return;
        }
        // :86-92: dormant. The grace accumulates up to its limit, and an open route runs its watchdog.
        self.dormant = true;
        self.grace = (self.grace + dt).min(grace_limit(nightmare));
        if self.route_active {
            self.route_left -= dt;
            self.node_left -= dt;
            if self.route_left <= 0.0 || self.node_left <= 0.0 {
                self.fail(b);
            }
            return;
        }
        // :93-95: no idle counting until the grace has run out.
        if self.grace < grace_limit(nightmare) {
            self.idle = 0.0;
            return;
        }
        // :96-102: the runner counts as still when slower than STILL_SPEED, and hiding or a safe room is a reason to be still.
        let at = t.pos;
        self.last_target_position = at;
        let moving = match self.last_position {
            Some(prev) => dist(at, prev) / dt.max(0.001) > STILL_SPEED,
            None => false,
        };
        self.last_position = Some(at);
        let still = !moving && !t.hidden && !t.in_safe_zone;
        self.idle = if still { self.idle + dt } else { 0.0 };
        if self.idle < idle_limit(nightmare) {
            return;
        }
        // :105-112: the idle limit is reached, so the signal route opens.
        self.idle = 0.0;
        self.route_active = true;
        self.route_peer = peer_id;
        self.route_nodes = NODES;
        self.route_seconds = node_seconds(nightmare);
        self.route_left = self.route_nodes as f32 * self.route_seconds + REPORT_SLACK;
        self.route_done = 0;
        self.node_left = self.route_seconds;
        self.events.push(VoidGrayEvent::SignalRoute {
            nodes: self.route_nodes,
            seconds: self.route_seconds,
            peer_id: self.route_peer,
        });
    }

    /// after_perception (void_gray_gimmick.gd:115-119): dormant, she does not notice anyone, so whatever perception
    /// decided is undone.
    pub fn after(&mut self, b: &mut Brain) {
        if self.dormant && !self.hunting {
            b.state = State::Idle;
            b.last_known = None;
        }
    }

    /// The open route as the prompt shows it: (presses done, presses needed, seconds left for the next press).
    pub fn route_prompt(&self) -> Option<(u32, u32, f32)> {
        self.route_active.then_some((self.route_done, self.route_nodes, self.node_left.max(0.0)))
    }

    /// The runner pressed E on the open route. Each press must land within `route_seconds` of the last (the timer
    /// runs in tick); the last press clears the route.
    pub fn press_route(&mut self, b: &mut Brain) {
        if !self.route_active {
            return;
        }
        self.route_done += 1;
        self.node_left = self.route_seconds;
        if self.route_done >= self.route_nodes {
            self.route_result(b, true);
        }
    }

    /// The route report from the failing or clearing player (void_gray_gimmick.gd:122-131). `ok`: every node pressed in time.
    pub fn route_result(&mut self, b: &mut Brain, ok: bool) {
        if !self.route_active {
            return;
        }
        if ok {
            self.route_active = false;
            self.idle = 0.0;
        } else {
            self.fail(b);
        }
    }

    /// The anti-idle pressure reached an idle runner: Gray stops being dormant and goes for where they stand
    /// (void_gray_gimmick.gd:134-142).
    pub fn force_hunt(&mut self, b: &mut Brain, at: (f32, f32)) {
        self.route_active = false;
        self.hunting = true;
        self.dormant = false;
        self.locked = true;
        self.lock_origin = at;
        self.last_target_position = at;
        continue_chase_from(b, at);
    }

    /// void_gray_gimmick.gd `_fail` (lines 145-153). The "GRAY IS HUNTING" message is dropped by the callout rule.
    fn fail(&mut self, b: &mut Brain) {
        self.route_active = false;
        self.grace = 0.0;
        self.hunting = true;
        self.dormant = false;
        b.state = State::Chase;
        b.last_known = Some(self.last_target_position);
    }

    /// void_gray_gimmick.gd:156-157. Drawn faded while dormant.
    pub fn hidden_from_view(&self) -> bool {
        self.dormant && !self.hunting
    }

    /// void_gray_gimmick.gd:160-161.
    pub fn holds_position(&self) -> bool {
        self.dormant && !self.hunting
    }

    /// void_gray_gimmick.gd:164-165. Nothing can catch her while dormant.
    pub fn immune_to_capture(&self) -> bool {
        self.dormant && !self.hunting
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;

    fn brain() -> Brain {
        let content = Content::load();
        let pool = content.draw_pool();
        let def = content.hunters[pool[0]].clone();
        Brain::new(def, (5.5, 5.5), "VOID-GRAY-TEST", Difficulty::Standard)
    }

    fn still_in_the_open(x: f32) -> Target {
        Target {
            pos: (x, 5.5),
            ..Default::default()
        }
    }

    /// Ticks at 1 s each with a standing, exposed runner. Returns the tick number (1-based) the route opened on, if any.
    fn ticks_until_route(g: &mut VoidGray, b: &mut Brain, relays: usize, limit: usize, t: &Target) -> Option<usize> {
        for i in 1..=limit {
            g.tick(b, 1.0, relays, t, 1);
            if g.route_active {
                return Some(i);
            }
        }
        None
    }

    #[test]
    fn chart_constants_match_the_godot_file() {
        assert_eq!(idle_limit(false), 45.0);
        assert_eq!(idle_limit(true), 30.0);
        assert_eq!(grace_limit(false), 180.0);
        assert_eq!(grace_limit(true), 120.0);
        assert_eq!(node_seconds(false), 2.5);
        assert_eq!(node_seconds(true), 1.8);
        assert_eq!(NODES, 5);
    }

    #[test]
    fn the_route_opens_after_the_grace_and_the_idle_limit_run_out() {
        // grace 180 s, then 45 s of standing still: the route opens on the 224th tick (180 + 44).
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(224));
        assert_eq!(
            g.take_events(),
            vec![VoidGrayEvent::SignalRoute { nodes: 5, seconds: 2.5, peer_id: 1 }]
        );
        assert!((g.route_left - 15.5).abs() < 1e-4, "5 x 2.5 + 3 s slack: {}", g.route_left);
        assert!(g.dormant && !g.hunting, "the route opens while she stays dormant");
    }

    #[test]
    fn pressing_e_in_time_clears_the_route_and_a_slow_press_wakes_her() {
        let t = still_in_the_open(5.5);
        let mut g = VoidGray::new(false);
        let mut b = brain();
        g.tick(&mut b, 1.0, 0, &t, 1);
        ticks_until_route(&mut g, &mut b, 0, 400, &t).expect("route opens");
        assert_eq!(g.route_prompt().map(|p| (p.0, p.1)), Some((0, 5)));
        for _ in 0..5 {
            g.tick(&mut b, 1.0, 0, &t, 1);
            g.press_route(&mut b);
        }
        assert!(!g.route_active && !g.hunting, "five timely presses clear it");
        assert_eq!(g.route_prompt(), None);

        let mut g = VoidGray::new(false);
        let mut b = brain();
        g.tick(&mut b, 1.0, 0, &t, 1);
        ticks_until_route(&mut g, &mut b, 0, 400, &t).expect("route opens");
        g.press_route(&mut b);
        for _ in 0..3 {
            g.tick(&mut b, 1.0, 0, &t, 1);
        }
        assert!(g.hunting, "a press later than the per-node seconds fails the route");
    }

    #[test]
    fn nightmare_shortens_the_grace_the_idle_and_the_presses() {
        // grace 120 s, then 30 s of standing still: opens on the 149th tick.
        let mut g = VoidGray::new(true);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(149));
        assert_eq!(g.route_seconds, 1.8);
        assert!((g.route_left - 12.0).abs() < 1e-4, "5 x 1.8 + 3 s slack: {}", g.route_left);
    }

    #[test]
    fn a_relay_restarts_the_grace_and_only_a_rise_counts() {
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        // the first tick only records the baseline, even with relays already taken
        g.tick(&mut b, 1.0, 2, &t, 1);
        assert_eq!(g.grace, 1.0, "no reset on the first tick");
        for _ in 0..100 {
            g.tick(&mut b, 1.0, 2, &t, 1);
        }
        // a relay at tick 102: the grace starts over (it is 1 s after this tick's own delta)
        g.tick(&mut b, 1.0, 3, &t, 1);
        assert_eq!(g.grace, 1.0, "the relay resets the grace, then this tick's delta is added");
        // the route then opens 223 ticks after the relay (179 grace ticks + 44 idle ticks)
        let next = ticks_until_route(&mut g, &mut b, 3, 400, &t);
        assert_eq!(next, Some(223));
    }

    #[test]
    fn a_drop_in_the_relay_count_is_not_a_collection() {
        // only a rise resets (void_gray_gimmick.gd:61-63): a lower count keeps the grace running
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        g.tick(&mut b, 1.0, 3, &t, 1);
        for _ in 0..100 {
            g.tick(&mut b, 1.0, 3, &t, 1);
        }
        g.tick(&mut b, 1.0, 1, &t, 1);
        assert_eq!(g.grace, 102.0, "no reset: 101 s of grace plus this tick");
    }

    #[test]
    fn a_relay_resets_a_hunt_back_to_idle() {
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        g.tick(&mut b, 1.0, 0, &t, 1);
        ticks_until_route(&mut g, &mut b, 0, 400, &t);
        g.route_result(&mut b, false);
        assert!(g.hunting && !g.dormant);
        g.tick(&mut b, 1.0, 1, &t, 1);
        assert!(!g.hunting && g.dormant, "a relay secures the route and she goes dormant again");
        assert_eq!(b.state, State::Idle);
        assert!(!g.route_active);
    }

    #[test]
    fn moving_or_hiding_or_a_safe_room_keeps_the_idle_count_down() {
        // moving 1 cell per second (above STILL_SPEED): the idle never builds
        let mut g = VoidGray::new(false);
        let mut b = brain();
        g.tick(&mut b, 1.0, 0, &still_in_the_open(0.0), 1);
        for i in 1..=400 {
            g.tick(&mut b, 1.0, 0, &still_in_the_open(i as f32), 1);
        }
        assert!(!g.route_active, "a runner who keeps moving never opens a route");

        // hiding: still, but not in the open
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let hidden = Target { hidden: true, ..still_in_the_open(5.5) };
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &hidden), None);

        // a safe room counts the same way
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let safe = Target { in_safe_zone: true, ..still_in_the_open(5.5) };
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &safe), None);
    }

    #[test]
    fn a_cleared_route_keeps_her_dormant_and_the_idle_starts_again() {
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(224));
        g.route_result(&mut b, true);
        assert!(g.dormant && !g.hunting && !g.route_active, "cleared: still dormant");
        // the grace is not reset by a clear, so 45 more still seconds reopen the route
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(45));
    }

    #[test]
    fn a_missed_route_hunts_the_failing_runner() {
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(224));
        g.route_result(&mut b, false);
        assert!(g.hunting && !g.dormant && !g.route_active);
        assert_eq!(b.state, State::Chase);
        assert_eq!(b.last_known, Some((5.5, 5.5)));
        assert!(!g.hidden_from_view() && !g.holds_position() && !g.immune_to_capture());
        g.after(&mut b);
        assert_eq!(b.state, State::Chase, "hunting, after_perception leaves her alone");
    }

    #[test]
    fn a_route_with_no_press_fails_when_the_first_node_runs_out() {
        // the route opens on tick 224; with no E press the first node's seconds run out long before the 15.5 s watchdog
        let mut g = VoidGray::new(false);
        let mut b = brain();
        let t = still_in_the_open(5.5);
        assert_eq!(ticks_until_route(&mut g, &mut b, 0, 400, &t), Some(224));
        let per_node = g.route_prompt().expect("open").2;
        let mut ticks = 0;
        while g.route_active && ticks < 20 {
            g.tick(&mut b, 1.0, 0, &t, 1);
            ticks += 1;
        }
        assert!(g.hunting && !g.route_active, "a missed node fails the route");
        assert!(ticks as f32 <= per_node.ceil() + 1.0, "it fails at the node timer ({per_node} s), not the watchdog: {ticks} ticks");
    }

    #[test]
    fn a_forced_hunt_locks_on_until_the_runner_moves_five_cells() {
        let mut g = VoidGray::new(false);
        let mut b = brain();
        g.force_hunt(&mut b, (5.5, 5.5));
        assert!(g.hunting && g.locked && !g.dormant);
        assert_eq!(b.state, State::Chase);
        // a runner within 5 cells of the origin: she keeps their position
        g.tick(&mut b, 1.0, 0, &still_in_the_open(8.0), 1);
        assert!(g.locked, "2.5 cells off is inside the release distance");
        assert_eq!(b.last_known, Some((8.0, 5.5)));
        // beyond 5 cells: the lock lets go and the hunt goes on as normal
        g.tick(&mut b, 1.0, 0, &still_in_the_open(11.0), 1);
        assert!(!g.locked && g.hunting, "5.5 cells off releases the lock");
    }

    #[test]
    fn constants_and_the_default_shape() {
        assert_eq!(RELAY_GRACE_SECONDS, 180.0);
        assert_eq!(IDLE_SECONDS, 45.0);
        assert_eq!(REPORT_SLACK, 3.0);
        assert_eq!(LOCK_RELEASE_DISTANCE, 5.0);
        // a fresh Gray is dormant, with the standard route shape
        let g = VoidGray::default();
        assert!(g.dormant && !g.hunting && !g.route_active);
        assert_eq!((g.route_nodes, g.route_seconds), (NODES, SECONDS_PER_NODE));
    }
}
