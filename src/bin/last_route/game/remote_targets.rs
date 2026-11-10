//! Hunting guests: the game of a room nobody plays in (`Game::no_local_runner`, the dedicated room host of
//! `headless/dedicated.rs`). A room hosted by a server process has no runner of its own, so the hunters choose among the
//! guests' runners instead (the TODO "remote (guest) players as hunter targets", host only).
//!
//! What the dedicated loop hands the game each frame ([`Game::set_remote_targets`]): every guest with a fresh sample, as
//! an [`RemoteTarget`] in ABSOLUTE cells (the samples' own frame), plus which one is the party anchor
//! ([`choose_anchor`]). What the game does with them:
//!
//! * The streamed world window is 3x3 chunks and exists once per room, so it follows the anchor: the parked body `p` is
//!   put on the anchor guest every frame and the window recentres, relabels or rebuilds around it like it does around a
//!   local runner. Only guests that stand inside that window, on the active level or a resident neighbour, are in play
//!   ([`Game::guest_targets`]); a guest elsewhere is not chased until the window reaches it.
//! * Each hunter takes the nearest guest in play as its default target and a bot may replace it where
//!   `hunters::brain::choose_target` allows ([`Game::target_for_guests`]); the capture test is the brain's own.
//! * A hunter that reaches a guest records a [`GuestCatch`] for the dedicated loop ([`Game::take_guest_catches`]) and
//!   stuns for the profile's `escape_stun`; the guest is not hunted again until the net layer reports it escaped,
//!   caught, blocked or timed out ([`Game::guest_released`]) and the grace has passed.
//! * With no guest in play the hunters do not run at all: an empty room costs one sample check a frame.
//!
//! Limits (see headless/dedicated.rs): relays, pickups and doors are still local to each client, so the room's relay
//! count (and with it the hunter population) is set at the start ([`Game::relays_collected`]) and does not grow; a guest
//! in a safe room is protected only where `fac.safe_zone` says so; a guest's flashlight is read from its sample, so a lit guest
//! is seen with the beam its sample says.

use super::{pdist, Game};
use crate::audio::Sounds;
use crate::config::{NOISE_CROUCH, NOISE_SPRINT, NOISE_WALK, SAFE_ZONE_RADIUS};
use crate::content::Content;
use crate::hunters::brain::{choose_target, Brain, Target, TargetRules};
use crate::world::catalog::Catalog;

/// Another guest must be this much closer to the party's centre than the anchor is before the anchor changes (m).
pub const ANCHOR_STICKY_M: f32 = 12.0;
/// Seconds after a guest's release (escape, timeout) before hunters may take it again (the local runner's `ESCAPE_GRACE`).
pub const RELEASE_GRACE_S: f32 = crate::config::ESCAPE_GRACE;

/// One guest runner as the hunters see it. `abs` is its absolute ground cell; the target's own `pos` is filled per frame
/// in the current window's cells ([`Game::guest_targets`]).
#[derive(Clone)]
pub struct RemoteTarget {
    pub peer: u32,
    pub abs: (f32, f32),
    /// Whether a hunter may take it: false for a runner that is spectating, over, or already in a hunter's grip.
    pub huntable: bool,
    pub target: Target,
}

impl RemoteTarget {
    /// The target of a guest standing at absolute cell `abs`, feet at absolute height `h` on `level`, facing `yaw` (the
    /// runner convention: forward is `(-sin, -cos)`), with the flags of its last sample.
    #[allow(clippy::too_many_arguments)]
    pub fn new(peer: u32, abs: (f32, f32), h: f32, level: i32, yaw: f32, moving: bool, sprinting: bool, crouching: bool, hidden: bool, huntable: bool) -> RemoteTarget {
        let noise = if !moving {
            0.0
        } else if crouching {
            NOISE_CROUCH
        } else if sprinting {
            NOISE_SPRINT
        } else {
            NOISE_WALK
        };
        RemoteTarget {
            peer,
            abs,
            huntable,
            target: Target {
                pos: abs,
                facing: (-yaw.sin(), -yaw.cos()),
                hidden,
                noise,
                sprinting: sprinting && moving,
                // neutral reading: the sample carries no temperature
                temperature: 60.0,
                level,
                y: h,
                ..Target::default()
            },
        }
    }

    /// Converts the host relay's sampled guest pose into the same target used
    /// by a dedicated room. Keeping this conversion here prevents the client
    /// host and dedicated host from drifting on flashlight/noise semantics.
    pub fn from_guest_pose(p: &crate::net::world_sync::GuestPose) -> RemoteTarget {
        let mut t = RemoteTarget::new(p.peer, (p.x, p.y), p.h, p.level, p.yaw, p.moving, p.sprinting, p.crouching, p.hidden, p.huntable);
        t.target.flashlight_on = p.flashlight;
        t
    }
}

/// A hunter reached a guest (the dedicated loop turns it into the room's capture: `NetSession::expect_capture_by`, the
/// grab event and the player event the guest answers with escaped / caught).
#[derive(Clone, Debug, PartialEq)]
pub struct GuestCatch {
    pub peer: u32,
    /// The hunter's id (`HunterDef::id`) and the place it stood, in the window's cells.
    pub hunter_id: String,
    pub at: (f32, f32),
    /// Where the hunter is in the frame the host sends (`Game::hunter_frame` order).
    pub frame_index: usize,
    /// The hunter's grab cannot be escaped (`Gimmick::capture_inescapable`).
    pub inescapable: bool,
}

/// A guest for the anchor choice: its peer, absolute cell and level.
pub type AnchorCandidate = (u32, (f32, f32), i32);

/// The guest the world window follows. The party is the guests on the most crowded level (a tie goes to the level of
/// the current anchor, then to the nearer to level 0); the anchor is the guest nearest that group's centroid, and the
/// current anchor keeps the role unless another guest is [`ANCHOR_STICKY_M`] closer to the centre, so the window does
/// not jump between two guests walking apart. `None` with no guest.
pub fn choose_anchor(guests: &[AnchorCandidate], current: Option<u32>) -> Option<u32> {
    if guests.is_empty() {
        return None;
    }
    let mut levels: Vec<i32> = guests.iter().map(|g| g.2).collect();
    levels.sort_unstable();
    levels.dedup();
    let count = |l: i32| guests.iter().filter(|g| g.2 == l).count();
    let current_level = current.and_then(|c| guests.iter().find(|g| g.0 == c)).map(|g| g.2);
    let level = *levels
        .iter()
        .max_by_key(|l| (count(**l), Some(**l) == current_level, std::cmp::Reverse(l.abs()), std::cmp::Reverse(**l)))
        .expect("at least one level");
    let group: Vec<&AnchorCandidate> = guests.iter().filter(|g| g.2 == level).collect();
    let n = group.len() as f32;
    let centre = (group.iter().map(|g| g.1 .0).sum::<f32>() / n, group.iter().map(|g| g.1 .1).sum::<f32>() / n);
    let to_centre = |g: &&AnchorCandidate| pdist(g.1, centre);
    let best = group
        .iter()
        .min_by(|a, b| to_centre(a).total_cmp(&to_centre(b)).then(a.0.cmp(&b.0)))
        .expect("a non-empty group");
    if let Some(keep) = current.and_then(|c| group.iter().find(|g| g.0 == c)) {
        if to_centre(keep) <= to_centre(best) + ANCHOR_STICKY_M {
            return Some(keep.0);
        }
    }
    Some(best.0)
}

/// The nearest of `guests` to `from` (cells), by straight distance; ties go to the lower peer.
pub fn nearest_guest(from: (f32, f32), guests: &[RemoteTarget]) -> Option<usize> {
    guests
        .iter()
        .enumerate()
        .min_by(|a, b| pdist(from, a.1.target.pos).total_cmp(&pdist(from, b.1.target.pos)).then(a.1.peer.cmp(&b.1.peer)))
        .map(|(i, _)| i)
}

/// Whether a cell position lies inside a window of `cols` x `rows` cells (the streamed 3x3 chunk window).
pub fn in_window(pos: (f32, f32), cols: i32, rows: i32) -> bool {
    pos.0 >= 0.0 && pos.1 >= 0.0 && pos.0 < cols as f32 && pos.1 < rows as f32
}

impl Game {
    /// Dedicated room: this frame's guests and which one is the anchor. Absolute cells; nothing else of the game changes.
    pub fn set_remote_targets(&mut self, targets: Vec<RemoteTarget>, anchor: Option<u32>) {
        self.remote_targets = targets;
        self.party_anchor = anchor.filter(|a| self.remote_targets.iter().any(|t| t.peer == *a));
    }

    /// Dedicated room: the catches since the last call, oldest first.
    pub fn take_guest_catches(&mut self) -> Vec<GuestCatch> {
        std::mem::take(&mut self.guest_catches)
    }

    /// The net layer closed a capture (the guest escaped, was caught, was blocked or never answered): hunters may take
    /// that guest again after [`RELEASE_GRACE_S`].
    pub fn guest_released(&mut self, peer: u32) {
        if let Some(hold) = self.guest_holds.get_mut(&peer) {
            hold.waiting = false;
            hold.left = RELEASE_GRACE_S;
        }
    }

    /// Whether `peer` is held by a hunter now (a catch with no release yet) or in the grace after one.
    pub fn guest_held(&self, peer: u32) -> bool {
        self.guest_holds.contains_key(&peer)
    }

    /// The hunter (frame index) holding a guest now, for the grab event: the held guest with the lowest peer id.
    pub fn guest_grab_index(&self) -> Option<usize> {
        let (_, hold) = self.guest_holds.iter().filter(|(_, h)| h.waiting).min_by_key(|(peer, _)| **peer)?;
        self.hunter_frame_index(&hold.hunter_id)
    }

    /// Where the hunter with this id is in the frame order of [`Game::hunter_frame`].
    pub(super) fn hunter_frame_index(&self, id: &str) -> Option<usize> {
        if let Some(i) = self.hunters.iter().position(|h| h.def.id == id) {
            return Some(i);
        }
        let base = self.hunters.len();
        self.resident_hunters.iter_all().position(|h| h.def.id == id).map(|k| base + k)
    }

    /// The guests the hunters can take this frame, in the window's cells: huntable, not held, inside the window, and on
    /// the active level or a resident neighbour. Empty on a game with a local runner.
    pub(super) fn guest_targets(&self) -> Vec<RemoteTarget> {
        let origin = self.window_origin();
        let active = self.runner_level();
        let resident = self.resident_hunters.levels();
        self.remote_targets
            .iter()
            .filter(|t| t.huntable && !self.guest_holds.contains_key(&t.peer))
            .map(|t| {
                let mut t = t.clone();
                t.target.pos = Self::abs_to_window(origin, t.abs);
                t.target.in_safe_zone = t.target.level == active && self.point_in_safe_zone(t.target.pos);
                t
            })
            .filter(|t| in_window(t.target.pos, self.fac.cols, self.fac.rows) && (t.target.level == active || resident.contains(&t.target.level)))
            .collect()
    }

    /// The target hunter `h` of the list on `list_level` acts on this frame in a room with no local runner: the nearest
    /// guest in play, or a bot where `choose_target` allows. Returns the target, whether it is a bot, and the guest's peer
    /// (None for a bot). None when there is no guest at all.
    pub(super) fn target_for_guests(&self, h: &Brain, list_level: i32, guests: &[RemoteTarget], bots: &[super::resident_hunters::BotTarget]) -> Option<(Target, bool, Option<u32>)> {
        // a guest on the hunter's own level first (a hunter on a flight sees through the well, not through a slab)
        let on_level: Vec<&RemoteTarget> = guests.iter().filter(|g| g.target.level == h.level).collect();
        let pool: Vec<&RemoteTarget> = if on_level.is_empty() { guests.iter().collect() } else { on_level };
        let nearest = pool
            .iter()
            .min_by(|a, b| pdist(h.pos, a.target.pos).total_cmp(&pdist(h.pos, b.target.pos)).then(a.peer.cmp(&b.peer)))?;
        let own: Vec<&super::resident_hunters::BotTarget> = bots.iter().filter(|b| b.level == list_level).collect();
        let pos: Vec<(f32, f32)> = own.iter().map(|b| b.target.pos).collect();
        let fac = self.level_parts(list_level).map_or(&self.fac, |p| p.0);
        let chosen = choose_target(TargetRules::get(), self.difficulty, h.pos, nearest.target.pos, &pos, |a, b| fac.has_line_of_sight(a, b));
        Some(match chosen {
            Some(k) => (own[k].target.clone(), true, None),
            None => {
                let mut t = nearest.target.clone();
                t.cross = self.cross_for(h, &t);
                (t, false, Some(nearest.peer))
            }
        })
    }

    /// Target selection for a client-hosted room. The local runner and fresh
    /// guest samples are one party; a hunter prefers the nearest runner on its
    /// level, then applies the normal NPC-bot rule to that runner candidate.
    pub(super) fn target_for_party(
        &self,
        h: &Brain,
        list_level: i32,
        local: &Target,
        guests: &[RemoteTarget],
        bots: &[super::resident_hunters::BotTarget],
    ) -> (Target, bool, Option<u32>) {
        let mut candidates: Vec<(Option<u32>, Target)> = vec![(None, local.clone())];
        candidates.extend(guests.iter().map(|g| (Some(g.peer), g.target.clone())));
        let same_level = candidates.iter().any(|(_, t)| t.level == h.level);
        let nearest = candidates
            .iter()
            .filter(|(_, t)| !same_level || t.level == h.level)
            .min_by(|a, b| super::pdist(h.pos, a.1.pos).total_cmp(&super::pdist(h.pos, b.1.pos)).then(a.0.cmp(&b.0)))
            .expect("the local runner is always a target");
        let own: Vec<&super::resident_hunters::BotTarget> = bots.iter().filter(|b| b.level == list_level).collect();
        let pos: Vec<(f32, f32)> = own.iter().map(|b| b.target.pos).collect();
        let fac = self.level_parts(list_level).map_or(&self.fac, |p| p.0);
        let chosen = choose_target(TargetRules::get(), self.difficulty, h.pos, nearest.1.pos, &pos, |a, b| fac.has_line_of_sight(a, b));
        match chosen {
            Some(k) => (own[k].target.clone(), true, None),
            None => {
                let mut t = nearest.1.clone();
                t.cross = self.cross_for(h, &t);
                (t, false, nearest.0)
            }
        }
    }

    /// The hunter `hunter_id` (at `frame_index` in the frame order) reached `peer`: the guest is held until its report or
    /// the net layer's timeout, and the catch is queued. The caller calms the hunter (`escape_stun`).
    pub(super) fn note_guest_catch(&mut self, peer: u32, hunter_id: String, at: (f32, f32), frame_index: usize, inescapable: bool) {
        if self.guest_holds.contains_key(&peer) {
            return;
        }
        self.guest_holds.insert(peer, GuestHold { hunter_id: hunter_id.clone(), waiting: true, left: f32::INFINITY });
        self.guest_catches.push(GuestCatch { peer, hunter_id, at, frame_index, inescapable });
    }

    /// One frame of a room with no local runner (what `Game::step` does instead of the runner's frame): the parked body
    /// goes to the anchor, the window follows it, the population tops up and the hunters hunt the guests in play.
    pub(super) fn step_dedicated(&mut self, dt: f32, content: &Content, catalog: &Catalog, sounds: &Sounds) {
        self.tick_void_effects(dt);
        self.time += dt;
        self.fade = 0.0;
        self.toast_t = (self.toast_t - dt).max(0.0);
        self.message_t = (self.message_t - dt).max(0.0);
        // a release's grace runs down; a catch still waiting for its report does not
        for hold in self.guest_holds.values_mut() {
            if !hold.waiting {
                hold.left = (hold.left - dt).max(0.0);
            }
        }
        let present: Vec<u32> = self.remote_targets.iter().map(|t| t.peer).collect();
        // a hold of a guest who left, or whose grace is over, is dropped
        self.guest_holds.retain(|peer, h| present.contains(peer) && (h.waiting || h.left > 0.0));
        let Some(anchor) = self.anchor_target() else {
            // nobody here: nothing moves and nothing is simulated until a guest comes
            return;
        };
        self.park_on(&anchor, content, catalog);
        let prof = self.prof();
        let level = self.runner_level();
        let guests = self.guest_targets();
        let openers: Vec<(f32, f32)> = self.hunters.iter().filter(|h| h.active).map(|h| h.pos).collect();
        let bodies: Vec<(f32, f32)> = guests.iter().filter(|g| g.target.level == level).map(|g| g.target.pos).chain(openers.iter().copied()).collect();
        self.fac.update_doors(&openers, &bodies, dt);
        self.discover_timer -= dt;
        if self.discover_timer <= 0.0 {
            self.discover_timer = 0.2;
            if self.inf.is_some() {
                self.ensure_hunters(content);
            }
        }
        if self.inf.is_some() && self.ladder.is_none() {
            self.stream_window(content, catalog);
        }
        self.update_hunters(dt, &prof, sounds);
    }

    /// The party anchor's target, or the first guest's when the anchor was not named.
    fn anchor_target(&self) -> Option<RemoteTarget> {
        self.party_anchor
            .and_then(|a| self.remote_targets.iter().find(|t| t.peer == a))
            .or_else(|| self.remote_targets.first())
            .cloned()
    }

    /// Puts the parked body on the anchor guest (its level, height and cell), loading the level when it is not the active
    /// one or a resident neighbour of it. The window recentres around it in `stream_window`.
    fn park_on(&mut self, anchor: &RemoteTarget, content: &Content, catalog: &Catalog) {
        let want = anchor.target.level;
        let height = anchor.target.y;
        if self.inf.is_some() && want != self.runner_level() && crate::world::infinite::level_allowed(want) {
            let resident = self.inf.as_ref().is_some_and(|i| i.stack.get(want).is_some_and(|r| r.center == i.center));
            if resident {
                self.relabel(want, content, catalog);
            } else {
                // the level is not at hand: the admin jump's rebuild (one of change_storey's allowed callers), and the body
                // is put on the anchor just below
                self.respawn_on_level(want, content, catalog);
            }
        }
        self.p.pos = Self::abs_to_window(self.window_origin(), anchor.abs);
        self.p.set_abs_y(height);
        self.p.yaw = anchor.target.facing.0.atan2(anchor.target.facing.1) + std::f32::consts::PI;
        self.p.hidden = None;
        self.p.stun = 0.0;
    }

    /// Whether a window position is inside the safe zone of this floor (`fac.safe_zone`), for a guest's target.
    pub(super) fn point_in_safe_zone(&self, pos: (f32, f32)) -> bool {
        self.fac.safe_zone.map(|s| pdist(pos, s.center()) <= SAFE_ZONE_RADIUS).unwrap_or(false)
    }
}

/// A guest a hunter holds: which hunter, whether its release is still awaited, and the seconds of grace left (infinite
/// while the release is awaited).
#[derive(Clone, Debug, PartialEq)]
pub struct GuestHold {
    pub hunter_id: String,
    pub waiting: bool,
    pub left: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::hunters::brain::State;

    const DT: f32 = 1.0 / 60.0;
    /// The chunk width in cells, as a float for the window maths of the tests.
    const CHUNK_CELLS: f32 = crate::world::infinite::CHUNK as f32;

    fn room() -> (Game, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-REMOTE-7", 0, &content, &catalog);
        g.no_local_runner = true;
        g.fade = 0.0;
        g.relays_collected = Difficulty::Standard.profile().activation_relays.max(1);
        (g, content, catalog)
    }

    fn step(g: &mut Game, content: &Content, catalog: &Catalog, frames: usize) {
        let sounds = Sounds::default();
        for _ in 0..frames {
            g.step(DT, &super::super::Controls::default(), content, catalog, &sounds);
        }
    }

    /// A guest on level 0 at window cell `at` of the current window.
    fn guest(g: &Game, peer: u32, at: (f32, f32)) -> RemoteTarget {
        let abs = Game::window_to_abs(g.window_origin(), at);
        RemoteTarget::new(peer, abs, 0.0, 0, 0.0, false, false, false, false, true)
    }

    #[test]
    fn the_anchor_is_the_guest_nearest_the_party_centre_and_sticks() {
        assert_eq!(choose_anchor(&[], None), None);
        assert_eq!(choose_anchor(&[(7, (10.0, 10.0), 0)], None), Some(7));
        // three in a row: the middle one is the centre
        let row = [(1, (0.0, 0.0), 0), (2, (20.0, 0.0), 0), (3, (40.0, 0.0), 0)];
        assert_eq!(choose_anchor(&row, None), Some(2));
        // the anchor keeps the role until another is ANCHOR_STICKY_M closer to the centre
        let apart = [(1, (0.0, 0.0), 0), (2, (30.0, 0.0), 0), (3, (36.0, 0.0), 0)];
        assert_eq!(choose_anchor(&apart, None), Some(2), "nearest the centroid (22, 0)");
        assert_eq!(choose_anchor(&apart, Some(1)), Some(2), "peer 1 is 22 from the centre, 14 more than peer 2");
        assert_eq!(choose_anchor(&apart, Some(3)), Some(3), "peer 3 is 14 from the centre, within the stickiness of peer 2");
        // a vanished anchor is replaced
        assert_eq!(choose_anchor(&row, Some(99)), Some(2));
        // the most crowded level wins; the others are out of the party
        let split = [(1, (0.0, 0.0), 0), (2, (500.0, 500.0), -1), (3, (510.0, 500.0), -1)];
        let a = choose_anchor(&split, None).unwrap();
        assert!(a == 2 || a == 3, "{a}");
        // a tie keeps the current anchor's level
        let tie = [(1, (0.0, 0.0), 0), (2, (500.0, 0.0), -1)];
        assert_eq!(choose_anchor(&tie, Some(2)), Some(2));
        assert_eq!(choose_anchor(&tie, Some(1)), Some(1));
        assert_eq!(choose_anchor(&tie, None), Some(1), "no anchor: the level nearer the top");
    }

    #[test]
    fn a_target_makes_the_noise_of_its_gait() {
        let t = |moving, sprint, crouch| RemoteTarget::new(1, (0.0, 0.0), 0.0, 0, 0.0, moving, sprint, crouch, false, true).target;
        assert_eq!(t(false, true, false).noise, 0.0, "standing still is silent even with the key down");
        assert_eq!(t(true, false, true).noise, NOISE_CROUCH);
        assert_eq!(t(true, true, false).noise, NOISE_SPRINT);
        assert_eq!(t(true, false, false).noise, NOISE_WALK);
        assert!(t(true, true, false).sprinting && !t(false, true, false).sprinting);
        // yaw 0 faces -z, the runner convention
        let f = t(false, false, false).facing;
        assert!(f.0.abs() < 1e-6 && (f.1 + 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_room_with_no_guest_runs_no_hunters_and_stays_cheap() {
        let (mut g, content, catalog) = room();
        step(&mut g, &content, &catalog, 90);
        assert!(g.hunters.is_empty(), "nobody to hunt, so none is made");
        assert!((g.time - 90.0 * DT).abs() < 1e-3, "the room's clock still runs");
        assert!(g.take_guest_catches().is_empty());
        // a guest far outside the window is not in play either
        let far = guest(&g, 9, (5000.0, 5000.0));
        g.set_remote_targets(vec![far], None);
        assert!(g.guest_targets().is_empty());
    }

    #[test]
    fn the_hunters_come_with_the_first_guest_and_each_takes_the_nearest() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        let a = guest(&g, 2, start);
        let b = guest(&g, 3, (start.0 + 14.0, start.1));
        g.set_remote_targets(vec![a, b], Some(2));
        step(&mut g, &content, &catalog, 30);
        assert!(!g.hunters.is_empty(), "the population tops up once a guest is there: relays {}", g.relays_collected);
        let h = &g.hunters[0];
        let (near_a, near_b) = (pdist(h.pos, start), pdist(h.pos, (start.0 + 14.0, start.1)));
        let want = if near_a <= near_b { Some(2) } else { Some(3) };
        assert_eq!(g.hunter_aim[0], want, "hunter 0 is {near_a:.1} from guest 2 and {near_b:.1} from guest 3");
        assert!(!g.hunters[0].target_is_bot);
        // the other guest takes over when the first one is held: it is no longer a candidate
        g.guest_holds.insert(2, GuestHold { hunter_id: "x".into(), waiting: true, left: f32::INFINITY });
        step(&mut g, &content, &catalog, 2);
        assert!(g.hunter_aim.iter().all(|a| *a != Some(2)), "a held guest is not hunted: {:?}", g.hunter_aim);
        assert!(g.hunter_aim.contains(&Some(3)));
    }

    #[test]
    fn a_hunter_closes_in_on_the_only_guest() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        g.set_remote_targets(vec![guest(&g, 2, start)], Some(2));
        step(&mut g, &content, &catalog, 20);
        assert!(!g.hunters.is_empty());
        assert_eq!(g.hunter_aim[0], Some(2));
        // the hunter wakes investigating the guest and goes for it: it ends up nearer, or already on its trail
        let first = pdist(g.hunters[0].pos, start);
        step(&mut g, &content, &catalog, 60 * 8);
        let here = Game::abs_to_window(g.window_origin(), Game::window_to_abs((0, 0), start));
        let later = pdist(g.hunters[0].pos, here);
        let taken = !g.take_guest_catches().is_empty();
        assert!(g.hunters[0].active);
        assert!(
            later < first || taken || matches!(g.hunters[0].state, State::Chase | State::Investigate),
            "first {first:.1}, later {later:.1}, state {:?}",
            g.hunters[0].state
        );
    }

    #[test]
    fn an_admin_summoned_hunter_hunts_a_guest_like_any_other_hunter() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        // the admin summons a hunter into the empty room, then a guest arrives
        assert!(crate::app::stress::summon(&mut g, "elise", &content, 0, 0));
        g.set_remote_targets(vec![guest(&g, 2, start)], Some(2));
        step(&mut g, &content, &catalog, 20);
        assert_eq!(g.hunter_aim[0], Some(2), "a summoned hunter hunts the guest");
        // NO TARGET does not change the summoned hunter's targeting: guests are still hunted, as for any hunter
        g.admin.no_target = true;
        step(&mut g, &content, &catalog, 20);
        assert_eq!(g.hunter_aim[0], Some(2), "a summoned hunter under NO TARGET hunts the guest like an ordinary one");
    }

    #[test]
    fn a_client_host_hunter_can_switch_from_the_local_runner_to_a_guest() {
        let (mut g, content, catalog) = room();
        let local = g.fac.start.center();
        g.p.pos = (local.0 + 10.0, local.1);
        assert!(crate::app::stress::summon(&mut g, "elise", &content, 0, 0));
        g.set_remote_targets(vec![guest(&g, 2, local)], None);
        step(&mut g, &content, &catalog, 20);
        assert_eq!(g.hunter_aim[0], Some(2), "the client host's hunter selects the nearer guest");
        assert!(!g.hunters[0].target_is_bot);
    }

    #[test]
    fn a_hunter_targeting_one_guest_switches_to_a_nearer_guest() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        g.set_remote_targets(vec![guest(&g, 2, start)], Some(2));
        step(&mut g, &content, &catalog, 20);
        assert_eq!(g.hunter_aim[0], Some(2), "the only guest is the target");
        // guest 3 steps onto the hunter, nearer than guest 2: the hunter re-picks it on the next frame
        let at = g.hunters[0].pos;
        g.set_remote_targets(vec![guest(&g, 2, start), guest(&g, 3, at)], Some(2));
        step(&mut g, &content, &catalog, 1);
        assert_eq!(g.hunter_aim[0], Some(3), "the nearer guest in range is chased");
    }

    #[test]
    fn a_hunter_that_reaches_a_guest_takes_it_once_and_the_guest_is_held_until_released() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        g.set_remote_targets(vec![guest(&g, 2, start)], Some(2));
        step(&mut g, &content, &catalog, 20);
        assert!(!g.hunters.is_empty());
        g.take_guest_catches();
        // the guest steps onto the hunter
        let (pos, y) = (g.hunters[0].pos, g.hunters[0].y);
        let abs = Game::window_to_abs(g.window_origin(), pos);
        g.set_remote_targets(vec![RemoteTarget::new(2, abs, y, 0, 0.0, false, false, false, false, true)], Some(2));
        step(&mut g, &content, &catalog, 1);
        let catches = g.take_guest_catches();
        assert_eq!(catches.len(), 1, "{catches:?}");
        assert_eq!(catches[0].peer, 2);
        assert_eq!(catches[0].hunter_id, g.hunters[0].def.id);
        assert_eq!(catches[0].frame_index, 0);
        assert!(g.guest_held(2));
        assert!(g.hunters[0].stun > 0.0, "the hunter calms for the escape stun");
        assert_eq!(g.guest_grab_index(), Some(0), "the grab event names the hunter that holds the guest");
        // the same standing guest is not taken again while the capture is open
        step(&mut g, &content, &catalog, 120);
        assert!(g.take_guest_catches().is_empty());
        assert!(g.report.is_none(), "no run ends in a room with no runner");
        // the guest's report frees it after the grace
        g.guest_released(2);
        assert!(g.guest_held(2) && g.guest_grab_index().is_none(), "held through the grace, but no hunter holds it");
        step(&mut g, &content, &catalog, (RELEASE_GRACE_S / DT) as usize + 10);
        assert!(!g.guest_held(2), "free again");
    }

    #[test]
    fn a_guest_that_leaves_drops_its_hold() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        g.set_remote_targets(vec![guest(&g, 2, start), guest(&g, 3, start)], Some(3));
        g.guest_holds.insert(2, GuestHold { hunter_id: "x".into(), waiting: true, left: f32::INFINITY });
        step(&mut g, &content, &catalog, 2);
        assert!(g.guest_held(2));
        g.set_remote_targets(vec![guest(&g, 3, start)], Some(3));
        step(&mut g, &content, &catalog, 2);
        assert!(!g.guest_held(2), "peer 2 is gone");
    }

    #[test]
    fn the_window_follows_the_anchor_to_where_it_walks() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        let abs = Game::window_to_abs(g.window_origin(), start);
        let centre_before = g.inf.as_ref().unwrap().center;
        // two chunks east of the start
        let far = (abs.0 + 2.0 * CHUNK_CELLS, abs.1);
        g.set_remote_targets(vec![RemoteTarget::new(2, far, 0.0, 0, 0.0, false, false, false, false, true)], Some(2));
        step(&mut g, &content, &catalog, 3);
        let centre_after = g.inf.as_ref().unwrap().center;
        assert_ne!(centre_before, centre_after, "the window recentred on the guest");
        let at = Game::abs_to_window(g.window_origin(), far);
        assert!(in_window(at, g.fac.cols, g.fac.rows), "the guest is inside the window now: {at:?}");
        assert_eq!(g.guest_targets().len(), 1);
    }

    #[test]
    fn the_room_changes_level_with_the_anchor() {
        let (mut g, content, catalog) = room();
        let start = g.fac.start.center();
        let abs = Game::window_to_abs(g.window_origin(), start);
        let level = -1;
        assert!(crate::world::infinite::level_allowed(level));
        // a guest standing on the level below, which is not resident (no flight is near): the room rebuilds there
        let y = super::super::level_stack::floor_y_of(level);
        g.set_remote_targets(vec![RemoteTarget::new(2, abs, y, level, 0.0, false, false, false, false, true)], Some(2));
        step(&mut g, &content, &catalog, 3);
        assert_eq!(g.runner_level(), level, "the active level is the anchor's");
        assert_eq!(g.guest_targets().len(), 1, "and the guest is in play there");
        // a guest on the old level is not in play while the window is down here (no resident list for it)
        let up = RemoteTarget::new(3, abs, 0.0, 0, 0.0, false, false, false, false, true);
        let down = RemoteTarget::new(2, abs, y, level, 0.0, false, false, false, false, true);
        g.set_remote_targets(vec![down, up], Some(2));
        let in_play: Vec<u32> = g.guest_targets().iter().map(|t| t.peer).collect();
        assert!(in_play.contains(&2));
        // the anchor goes back up: the room follows
        let up = RemoteTarget::new(3, abs, 0.0, 0, 0.0, false, false, false, false, true);
        g.set_remote_targets(vec![up], Some(3));
        step(&mut g, &content, &catalog, 3);
        assert_eq!(g.runner_level(), 0);
    }

    #[test]
    fn a_dedicated_frame_names_its_window_and_a_guest_moves_the_hunters_into_its_own() {
        let (mut host, content, catalog) = room();
        let start = host.fac.start.center();
        host.set_remote_targets(vec![guest(&host, 2, start)], Some(2));
        step(&mut host, &content, &catalog, 30);
        assert!(!host.hunters.is_empty());
        let frame = host.hunter_frame();
        let o = host.window_origin();
        assert!(frame.iter().all(|e| e["origin"] == serde_json::json!([o.0, o.1])), "every entry carries the room's window origin");
        // a guest whose window sits somewhere else (here: the same seed, its own origin)
        let mut seen = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-REMOTE-7", 0, &content, &catalog);
        seen.remote_hunters = true;
        let mut moved = frame.clone();
        for e in moved.iter_mut() {
            e["origin"] = serde_json::json!([o.0 + 45, o.1 - 90]);
        }
        seen.apply_hunter_frame(&moved);
        let id = frame[0]["id"].as_str().unwrap();
        let here = seen.hunters.iter().find(|h| h.def.id == id).expect("the guest made the hunter");
        let g_origin = seen.window_origin();
        let x = frame[0]["position"][0].as_f64().unwrap() as f32 + (o.0 + 45 - g_origin.0) as f32;
        let y = frame[0]["position"][1].as_f64().unwrap() as f32 + (o.1 - 90 - g_origin.1) as f32;
        assert!((here.pos.0 - x).abs() < 1e-3 && (here.pos.1 - y).abs() < 1e-3, "{:?} vs ({x}, {y})", here.pos);
        // a frame from a client host has no origin and is taken as it always was
        let mut plain = frame.clone();
        for e in plain.iter_mut() {
            e.as_object_mut().unwrap().remove("origin");
        }
        seen.apply_hunter_frame(&plain);
        let again = seen.hunters.iter().find(|h| h.def.id == id).unwrap();
        assert!((again.pos.0 - frame[0]["position"][0].as_f64().unwrap() as f32).abs() < 1e-3);
    }
}
