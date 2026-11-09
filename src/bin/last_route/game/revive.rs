//! Downed runners and NPC teammates, with no revive kit (data/revive.json).
//!
//! A runner who would die goes down instead ([`Game::go_down`], called from `end_run`) when someone could revive it: a
//! living teammate in the room or a joined NPC bot that is not down ([`Game::runner_has_reviver`]). With nobody, the
//! capture ends the run at once with a report. A downed runner lies still: no
//! move, no interact, no ladder, and the hunters that caught it cannot catch it again. A teammate revives it by holding
//! interact beside it for `hold_seconds`: a bot standing within `reach` on the same floor does it on its own, and the
//! runner revives a downed bot the same way by holding interact (`Controls::interact_held`). A downed body that nobody
//! revives within `timeout_seconds` dies: the runner's run ends ([`Game::finish_run`]), a bot is taken off the floor.
//!
//! A hunter in a chase that reaches a joined bot on its floor puts it down the same way ([`Game::npc_catches`]): the
//! hunter is stunned, a hidden bot is out of reach. A bot walks to a downed teammate in range (the "revive" intent,
//! npc.rs) and holds the revive; a bot hidden in a locker cannot.

use super::{pdist, Controls, Game};
use serde_json::Value;
use std::sync::OnceLock;

/// The embedded file (data/revive.json), by its path under the data folder.
pub const REVIVE_PATH: &str = "revive.json";

/// The numbers of a revive, from data/revive.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Metres on the floor within which a teammate (or the runner, for a bot) can hold the revive.
    pub reach: f32,
    /// Seconds of continuous holding that revive a body.
    pub hold_seconds: f32,
    /// Seconds a downed body lasts before nobody's revive saves it.
    pub timeout_seconds: f32,
    /// Seconds a hunter that caught the runner stays stunned when the runner goes down.
    pub hunter_stun_seconds: f32,
    /// Metres within which a chasing hunter puts a joined bot down.
    pub npc_catch_radius: f32,
    /// Share of `hold_seconds` a revive takes for a runner holding a revive kit (0..=1).
    pub kit_hold_scale: f32,
    /// Seconds of one escape-struggle loop (the hunters' hold-and-shake cycle), ported from Godot into data.
    pub escape_loop_seconds: f32,
    /// Seconds a hunter is knocked back after a struggle is won.
    pub escape_knockback_seconds: f32,
    /// How many times a solo runner with only an NPC bot reviver can be stood back up before a capture ends the run.
    pub max_bot_revives: u32,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |k: &str| {
            v.get(k)
                .and_then(Value::as_f64)
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("{k} must be a number of 0 or more"))
        };
        Ok(Rules {
            reach: num("reach")?,
            hold_seconds: num("hold_seconds")?,
            timeout_seconds: num("timeout_seconds")?,
            hunter_stun_seconds: num("hunter_stun_seconds")?,
            npc_catch_radius: num("npc_catch_radius")?,
            kit_hold_scale: num("kit_hold_scale").unwrap_or(0.35).min(1.0),
            escape_loop_seconds: num("escape_loop_seconds").unwrap_or(0.6),
            escape_knockback_seconds: num("escape_knockback_seconds").unwrap_or(0.4),
            max_bot_revives: num("max_bot_revives").unwrap_or(2.0) as u32,
        })
    }

    /// Seconds of holding that revive a body: shorter for a helper with a revive kit.
    pub fn hold_needed(&self, kit: bool) -> f32 {
        if kit {
            self.hold_seconds * self.kit_hold_scale
        } else {
            self.hold_seconds
        }
    }
}

/// The embedded rules (data/revive.json), parsed once.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == REVIVE_PATH)
            .map(|(_, t)| t)
            .expect("data/revive.json is embedded by build.rs");
        Rules::parse(text).expect("data/revive.json is valid")
    })
}

/// A downed body: the seconds left before it dies, the seconds of revive hold so far, and (for the runner) the hunter
/// that caught it, for the run report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Downed {
    pub remaining: f32,
    pub held: f32,
    pub hunter: usize,
}

impl Game {
    /// True while the runner lies down (and is not yet revived or dead).
    pub fn runner_downed(&self) -> bool {
        self.downed.is_some()
    }

    /// True when someone could revive the runner if it went down: another living runner in the room
    /// (`room_teammates`, set from the net roster in app/mod.rs), or an NPC bot on the run that has joined the runner and
    /// is not down itself (a hidden bot counts: it can leave its locker). Solo with no such bot, a capture ends the run
    /// at once.
    ///
    /// Co-op note: `room_teammates` counts the room's remote human runners, who go down for them too (the gate is the
    /// same on every client). A downed runner stands back up by a local helper's hold, a revive kit, or — when a remote
    /// teammate attends it over the wire — a host-reported revive; nobody near, it dies at the timeout (`tick_revives`).
    pub fn runner_has_reviver(&self) -> bool {
        self.room_teammates > 0 || self.npcs.iter().any(|n| n.joined && !self.downed_npcs.contains_key(&n.id))
    }

    /// The brief immunity a runner gets the instant it stands back up (a hold-revive in `tick_revives`, or a revive kit
    /// in `game/items.rs`): the `escape_grace` window the struggle-win path grants at `game/hunting.rs:623-645`, so the
    /// hunter still standing over the body cannot re-grab it on the very next frame (`resolve_capture` bails while
    /// `escape_grace > 0`, hunting.rs:526-528). Without it an adjacent hunter re-enters an escapable grab at once, the
    /// escape flips None->Some, and the "<HUNTER> HAS THE HOST" notice re-fires forever (net/hunter_events.rs). This is
    /// the TODO 714 capture-immunity behaviour (game/capture_immunity_tests.rs).
    ///
    /// The catcher is NOT re-stunned or shoved into a search here: `go_down` already staggered it once when the runner
    /// first went down, and re-stunning + relocating it on every revive froze the chaser and sent it wandering off.
    /// Each revive is also counted (`down_revives`): once a solo runner with only a bot reviver has spent the bot-revive
    /// budget, the next capture ends the run instead of downing again (`go_down`).
    pub(super) fn grant_post_revive_grace(&mut self, _hunter: usize) {
        // no struggle is in flight for a runner that just stood up, and the grace blocks the next grab for a moment
        self.escape = None;
        self.escape_grace = crate::config::ESCAPE_GRACE;
        self.down_revives += 1;
    }

    /// The runner would die: it goes down instead when someone could revive it ([`Game::runner_has_reviver`]). Returns
    /// true when the runner is down after the call (it went down now, or it was down already and a further catch does
    /// not kill it; only the timeout does), false when nobody could revive it and the run should end now.
    pub(super) fn go_down(&mut self, hunter: usize) -> bool {
        // a run that already has its report is finished: it is never taken down (and never ended twice)
        if self.downed.is_some() || self.report.is_some() {
            return true;
        }
        // nobody could revive the runner: the capture ends the run at once (end_run then writes the report)
        if !self.runner_has_reviver() {
            return false;
        }
        // solo with only a bot reviver: once the bot-revive budget is spent, the capture ends the run instead of downing
        // again (an ever-present bot auto-revives, so without this cap the runner is immortal). Co-op (a human teammate in
        // the room) keeps its unlimited down/revive mechanic.
        if self.room_teammates == 0 && self.down_revives >= rules().max_bot_revives {
            return false;
        }
        let r = rules();
        self.downed = Some(Downed { remaining: r.timeout_seconds, held: 0.0, hunter });
        self.downed_hunter = self.hunters.get(hunter).map(|h| (h.def.id.clone(), h.def.name.clone()));
        self.escape = None;
        self.hide_check = None;
        if self.p.hidden.is_some() {
            self.leave_locker(None);
        }
        self.p.flashlight_on = false;
        if let Some(h) = self.hunters.get_mut(hunter) {
            h.apply_stun(r.hunter_stun_seconds, false);
        }
        self.say("YOU ARE DOWN");
        true
    }

    /// A bot goes down (a teammate can revive it). False when it is not on the floor or is down already.
    pub(super) fn npc_go_down(&mut self, id: u32) -> bool {
        if self.downed_npcs.contains_key(&id) || !self.npcs.iter().any(|n| n.id == id) {
            return false;
        }
        self.downed_npcs.insert(id, Downed { remaining: rules().timeout_seconds, held: 0.0, hunter: 0 });
        true
    }

    /// A hunter that is chasing and reaches a joined bot on its floor puts the bot down (not a hidden bot, not one
    /// already down, not a bot on a ladder or flight). The hunter is stunned as when it downs the runner. Only the
    /// side that simulates the hunters runs this (a guest has no bots of its own).
    pub(super) fn npc_catches(&mut self) {
        if self.report.is_some() || self.npcs.is_empty() {
            return;
        }
        let r = rules();
        let mut caught: Vec<(u32, usize)> = vec![];
        for n in &self.npcs {
            if !n.joined || n.hidden.is_some() || n.walk.is_some() || self.downed_npcs.contains_key(&n.id) {
                continue;
            }
            let hit = self.hunters.iter().position(|h| {
                h.active
                    && matches!(h.state, crate::hunters::brain::State::Chase)
                    && h.stun <= 0.0
                    && h.level == n.level
                    && pdist(h.pos, n.at) <= r.npc_catch_radius
            });
            if let Some(i) = hit {
                caught.push((n.id, i));
            }
        }
        for (id, hunter) in caught {
            let name = self.npcs.iter().find(|n| n.id == id).map(|n| n.name.clone()).unwrap_or_default();
            if self.npc_go_down(id) {
                if let Some(h) = self.hunters.get_mut(hunter) {
                    h.apply_stun(r.hunter_stun_seconds, false);
                }
                self.say(&format!("{name} IS DOWN"));
            }
        }
    }

    /// The revive of every downed body, one frame: the holds and the timeouts. Called every frame before the runner's
    /// own step, so a downed runner is revived or dies here too.
    pub(super) fn tick_revives(&mut self, dt: f32, c: &Controls) {
        let r = rules();
        let level = self.runner_level();
        let at = self.p.pos;
        // the teammates who can hold a revive: joined bots that are not down and not hidden in a locker
        let helpers: Vec<(u32, i32, (f32, f32))> = self
            .npcs
            .iter()
            .filter(|n| n.joined && n.hidden.is_none() && !self.downed_npcs.contains_key(&n.id))
            .map(|n| (n.id, n.level, n.at))
            .collect();

        if let Some(mut d) = self.downed {
            let helped = helpers.iter().any(|&(_, lv, p)| lv == level && pdist(p, at) <= r.reach);
            d.held = if helped { d.held + dt } else { 0.0 };
            d.remaining -= dt;
            if d.held >= r.hold_seconds {
                let hunter = d.hunter;
                self.downed = None;
                // a just-revived runner cannot be instantly re-grabbed (resolve_capture, hunting.rs:526)
                self.grant_post_revive_grace(hunter);
                self.say("YOU ARE BACK UP");
            } else if d.remaining <= 0.0 {
                self.downed = None;
                let a = self.admin_flags();
                if !(a.god || a.no_target) {
                    self.finish_run(d.hunter, "contact");
                    // the downed hunter by identity: the active list may have shifted since the catch
                    if let (Some((id, name)), Some(rep)) = (self.downed_hunter.take(), self.report.as_mut()) {
                        rep.hunter = id;
                        rep.hunter_name = name;
                    }
                }
            } else {
                self.downed = Some(d);
            }
        }

        // the runner holds a revive on a downed bot by holding interact beside it (not while it is down itself)
        let runner_helps = self.downed.is_none() && c.interact_held;
        let has_kit = self.slots.iter().flatten().any(|s| s.item == super::items::Item::ReviveKit);
        let ids: Vec<u32> = self.downed_npcs.keys().copied().collect();
        for id in ids {
            let Some(npc) = self.npcs.iter().find(|n| n.id == id) else {
                self.downed_npcs.remove(&id);
                continue;
            };
            let (lv, p, name) = (npc.level, npc.at, npc.name.clone());
            let helped = (runner_helps && lv == level && pdist(at, p) <= r.reach)
                || helpers.iter().any(|&(hid, hlv, hp)| hid != id && hlv == lv && pdist(hp, p) <= r.reach);
            let Some(mut d) = self.downed_npcs.get(&id).copied() else {
                continue;
            };
            d.held = if helped { d.held + dt } else { 0.0 };
            d.remaining -= dt;
            if d.held >= r.hold_needed(runner_helps && has_kit) {
                self.downed_npcs.remove(&id);
                self.say(&format!("{name} IS BACK UP"));
            } else if d.remaining <= 0.0 {
                self.downed_npcs.remove(&id);
                self.npcs.retain(|n| n.id != id);
                self.say(&format!("{name} DID NOT MAKE IT"));
            } else {
                self.downed_npcs.insert(id, d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kit_shortens_the_hold_but_never_below_nothing() {
        let r = rules();
        assert_eq!(r.hold_needed(false), r.hold_seconds);
        assert!(r.hold_needed(true) < r.hold_needed(false) && r.hold_needed(true) > 0.0);
        let fast = Rules::parse(
            r#"{"reach":1,"hold_seconds":4,"timeout_seconds":9,"hunter_stun_seconds":1,"npc_catch_radius":1,"kit_hold_scale":0.5}"#,
        )
        .unwrap();
        assert_eq!(fast.hold_needed(true), 2.0);
        assert!(r.timeout_seconds > r.hold_seconds, "the window outlasts one hold");
        assert!(r.escape_loop_seconds > 0.0 && r.escape_knockback_seconds >= 0.0);
    }
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn game() -> (Game, Content) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-REVIVE-7", 0, &content, &catalog);
        g.npc_ensure_run(&content);
        // the run's own bots are few on some difficulties: the tests need two
        if g.npcs.len() < 2 {
            g.npc_spawn(2, 90, (0.0, 1.5, 0.0), (1.0, 0.0, 0.0));
        }
        // another runner in the room: a capture downs the runner (solo with no joined bot it would end the run)
        g.room_teammates = 1;
        (g, content)
    }

    /// The fixture with nobody who could revive the runner: no other runner, and no bot joined.
    fn solo() -> Game {
        let (mut g, _) = game();
        g.room_teammates = 0;
        for n in g.npcs.iter_mut() {
            n.joined = false;
        }
        g
    }

    #[test]
    fn a_solo_capture_with_nobody_to_revive_ends_the_run_at_once() {
        let mut g = solo();
        assert!(!g.runner_has_reviver());
        g.end_run(0, "contact");
        assert!(!g.runner_downed(), "nobody could revive: the runner is not downed");
        let rep = g.report.as_ref().expect("the capture ends the run with a report");
        assert_eq!(rep.cause, "contact");
    }

    #[test]
    fn a_joined_bot_or_a_teammate_makes_a_capture_down_the_runner() {
        // a joined bot on the run
        let mut g = solo();
        let id = g.npcs[0].id;
        g.npcs[0].joined = true;
        assert!(g.runner_has_reviver());
        g.end_run(0, "contact");
        assert!(g.runner_downed() && g.report.is_none(), "a joined bot could revive: the runner goes down");
        // a joined bot that is down itself revives nobody
        let mut g = solo();
        g.npcs[0].joined = true;
        assert!(g.npc_go_down(id));
        assert!(!g.runner_has_reviver(), "a downed bot cannot revive the runner");
        // another runner in the room
        let mut g = solo();
        g.room_teammates = 1;
        g.end_run(0, "contact");
        assert!(g.runner_downed() && g.report.is_none(), "a teammate in the room could revive: the runner goes down");
    }

    /// Puts bot `id` on the runner's floor, beside the runner, joined.
    fn beside_runner(g: &mut Game, id: u32) {
        let (level, at) = (g.runner_level(), g.p.pos);
        let n = g.npcs.iter_mut().find(|n| n.id == id).expect("the bot exists");
        n.joined = true;
        n.level = level;
        n.at = at;
    }

    fn idle() -> Controls {
        Controls::default()
    }

    #[test]
    fn the_data_file_loads_with_the_agreed_numbers() {
        let r = rules();
        assert_eq!(r.hold_seconds, 3.0);
        assert_eq!(r.timeout_seconds, 45.0);
        assert!(r.reach > 0.0);
        assert!(Rules::parse("{\"reach\": 1.0}").is_err(), "a missing number is refused");
    }

    #[test]
    fn a_downed_runner_is_not_dead() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        assert!(g.runner_downed(), "the catch puts the runner down");
        assert!(g.report.is_none(), "a downed runner has no run report: it is not dead");
        // a downed runner's body reads as down to the other players, not as a finished run
        let state = crate::net::runner_figure::local_state(&g);
        assert!(state.down, "the wire pose shows the runner down");
        // a further catch while down does not kill it
        g.end_run(0, "contact");
        assert!(g.report.is_none());
        assert!(g.runner_downed());
    }

    #[test]
    fn a_held_revive_restores_the_runner() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        let id = g.npcs[0].id;
        beside_runner(&mut g, id);
        // 2.5 s of hold is not enough
        for _ in 0..25 {
            g.tick_revives(0.1, &idle());
        }
        assert!(g.runner_downed(), "two and a half seconds is short of the hold");
        for _ in 0..10 {
            g.tick_revives(0.1, &idle());
        }
        assert!(!g.runner_downed(), "three seconds of hold restores the runner");
        assert!(g.report.is_none());
    }

    #[test]
    fn a_downed_runner_who_is_not_revived_dies_at_the_timeout() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        // nobody beside the runner: the timeout is 45 s
        for _ in 0..44 {
            g.tick_revives(1.0, &idle());
        }
        assert!(g.runner_downed(), "44 s is inside the timeout");
        assert!(g.report.is_none());
        g.tick_revives(1.0, &idle());
        assert!(!g.runner_downed());
        assert!(g.report.is_some(), "the timeout ends the run");
    }

    #[test]
    fn the_run_card_names_the_hunter_that_downed_the_runner_after_the_list_shifts() {
        let (mut g, _) = game();
        let want = g.hunters.get(0).map(|h| h.def.id.clone());
        g.end_run(0, "caught");
        assert!(g.downed_hunter.is_some() == want.is_some());
        // the active list changes while the runner lies down
        if g.hunters.len() > 1 {
            g.hunters.remove(0);
        }
        g.tick_revives(rules().timeout_seconds + 1.0, &idle());
        let rep = g.report.as_ref().expect("the timeout ends the run");
        // the report names the hunter that downed the runner (empty when the fixture has no hunter)
        assert_eq!(rep.hunter, want.unwrap_or_default());
    }

    #[test]
    fn a_finished_run_is_never_taken_down_again() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        g.tick_revives(rules().timeout_seconds + 1.0, &idle());
        assert!(g.report.is_some());
        g.end_run(0, "caught");
        assert!(!g.runner_downed(), "a capture after the timeout does not down a finished run");
    }

    #[test]
    fn a_hidden_bot_cannot_revive() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        let id = g.npcs[0].id;
        beside_runner(&mut g, id);
        g.npcs.iter_mut().find(|n| n.id == id).unwrap().hidden = Some(0);
        for _ in 0..50 {
            g.tick_revives(0.1, &idle());
        }
        assert!(g.runner_downed(), "a bot inside a locker holds no revive");
        // out of the locker the same bot does revive
        g.npcs.iter_mut().find(|n| n.id == id).unwrap().hidden = None;
        for _ in 0..31 {
            g.tick_revives(0.1, &idle());
        }
        assert!(!g.runner_downed());
    }

    #[test]
    fn a_hidden_bot_does_not_revive_a_downed_bot() {
        let (mut g, _) = game();
        let (a, b) = (g.npcs[0].id, g.npcs[1].id);
        assert!(g.npc_go_down(a));
        beside_runner(&mut g, a);
        beside_runner(&mut g, b);
        g.npcs.iter_mut().find(|n| n.id == b).unwrap().hidden = Some(0);
        for _ in 0..31 {
            g.tick_revives(0.1, &idle());
        }
        assert!(g.downed_npcs.contains_key(&a), "the hidden teammate cannot hold the revive");
    }

    /// Puts hunter 0 in a chase, active and unstunned, on `level` at `pos`. False when the fixture has no hunter.
    fn chase_at(g: &mut Game, level: i32, pos: (f32, f32)) -> bool {
        let Some(h) = g.hunters.get_mut(0) else { return false };
        h.active = true;
        h.state = crate::hunters::brain::State::Chase;
        h.stun = 0.0;
        h.level = level;
        h.pos = pos;
        true
    }

    #[test]
    fn a_chasing_hunter_puts_a_joined_bot_down_and_is_stunned() {
        let (mut g, _) = game();
        let id = g.npcs[0].id;
        let (at, lv) = {
            let n = g.npcs.iter_mut().find(|n| n.id == id).unwrap();
            n.joined = true;
            n.hidden = None;
            n.walk = None;
            (n.at, n.level)
        };
        if !chase_at(&mut g, lv, at) {
            return; // this seed has no hunter in the fixture
        }
        g.npc_catches();
        assert!(g.downed_npcs.contains_key(&id), "the caught bot is down");
        assert!(g.hunters[0].stun > 0.0, "the hunter is stunned, as when it downs the runner");
        // a bot that is down is not caught again, and the stun does not renew
        g.hunters[0].stun = 0.5;
        g.npc_catches();
        assert_eq!(g.hunters[0].stun, 0.5);
        assert!(g.report.is_none(), "a bot going down is no run end");
    }

    #[test]
    fn a_hidden_or_far_bot_is_not_caught() {
        let (mut g, _) = game();
        let (a, b) = (g.npcs[0].id, g.npcs[1].id);
        for n in g.npcs.iter_mut() {
            n.joined = true;
            n.walk = None;
        }
        let (pa, la) = {
            let n = g.npcs.iter_mut().find(|n| n.id == a).unwrap();
            n.hidden = Some(0);
            (n.at, n.level)
        };
        {
            let n = g.npcs.iter_mut().find(|n| n.id == b).unwrap();
            n.at = (pa.0 + 50.0, pa.1);
            n.level = la;
        }
        if !chase_at(&mut g, la, pa) {
            return;
        }
        g.npc_catches();
        assert!(!g.downed_npcs.contains_key(&a), "a hidden bot is out of reach");
        assert!(!g.downed_npcs.contains_key(&b), "a bot beyond the catch radius is safe");
    }

    #[test]
    fn an_owner_summon_stops_at_the_room_cap() {
        let (mut g, _) = game();
        let cap = crate::game::npc::Tuning::get().summon_room_cap;
        assert!(cap >= 1);
        let (level, spot) = (g.runner_level(), g.p.pos);
        for _ in 0..(cap + 8) {
            if g.npc_room_space() == 0 {
                break;
            }
            g.npc_spawn_at(1, 70, spot, level);
        }
        assert_eq!(g.npc_room_space(), 0, "a full room has no space left");
    }

    #[test]
    fn a_bot_revives_a_downed_bot() {
        let (mut g, _) = game();
        let (a, b) = (g.npcs[0].id, g.npcs[1].id);
        assert!(g.npc_go_down(a));
        beside_runner(&mut g, a);
        beside_runner(&mut g, b);
        // the revive is held by the other bot, not by the one that is down
        for _ in 0..31 {
            g.tick_revives(0.1, &idle());
        }
        assert!(!g.downed_npcs.contains_key(&a), "a teammate's three-second hold revives the bot");
        assert!(g.downed_npcs.is_empty());
        assert!(g.npcs.iter().any(|n| n.id == a), "a revived bot stays on the floor");
    }

    #[test]
    fn a_downed_bot_nobody_revives_leaves_the_floor_at_the_timeout() {
        let (mut g, _) = game();
        let a = g.npcs[0].id;
        let before = g.npcs.len();
        assert!(g.npc_go_down(a));
        assert!(!g.npc_go_down(a), "a bot goes down once");
        for _ in 0..46 {
            g.tick_revives(1.0, &idle());
        }
        assert!(!g.npcs.iter().any(|n| n.id == a));
        assert_eq!(g.npcs.len(), before - 1);
        assert!(g.report.is_none(), "a bot's death is no run end");
    }

    #[test]
    fn the_runner_revives_a_downed_bot_by_holding_interact() {
        let (mut g, _) = game();
        let a = g.npcs[0].id;
        assert!(g.npc_go_down(a));
        beside_runner(&mut g, a);
        // the bot is beside the runner but only the runner's hold revives it: no other bot is near
        for id in g.npcs.iter().map(|n| n.id).filter(|&id| id != a).collect::<Vec<_>>() {
            g.npcs.retain(|n| n.id != id);
        }
        let held = Controls { interact_held: true, ..Controls::default() };
        for _ in 0..31 {
            g.tick_revives(0.1, &held);
        }
        assert!(g.downed_npcs.is_empty(), "the runner's held interact revives the bot");
    }

    /// Co-op (a human teammate in the room) downs the runner on a capture; once a local helper's hold stands it back up,
    /// the runner carries a post-revive grace, so the hunter still beside it cannot re-grab it on the next frame. This is
    /// the grab-god bug: without the grace the re-grab re-fired the "HAS THE HOST" notice forever (net/hunter_events.rs).
    #[test]
    fn a_held_revive_in_co_op_leaves_the_runner_briefly_immune_to_a_re_grab() {
        let (mut g, _) = game();
        // only a human teammate in the room: no joined bot gates the down, so this is the pure co-op path
        g.room_teammates = 1;
        for n in g.npcs.iter_mut() {
            n.joined = false;
        }
        // a hunter standing on the runner, chasing: the one that would re-grab the instant the runner stands up
        let level = g.runner_level();
        let pos = g.p.pos;
        let had_hunter = chase_at(&mut g, level, pos);
        g.end_run(0, "contact");
        assert!(g.runner_downed() && g.report.is_none(), "a teammate in the room sends the runner down, not out");
        // a local helper holds the three-second revive
        let id = g.npcs[0].id;
        beside_runner(&mut g, id);
        for _ in 0..31 {
            g.tick_revives(0.1, &idle());
        }
        assert!(!g.runner_downed() && g.report.is_none(), "the hold stood the runner back up");
        assert!(g.escape.is_none(), "no struggle is in flight the instant the runner stands up");
        assert!(g.escape_grace > 0.0, "the just-revived runner is briefly immune to a re-grab");
        if had_hunter {
            // the stagger comes only from go_down's stun when the runner first went down; grant_post_revive_grace no
            // longer re-stuns the catcher, and tick_revives does not decrement stun, so it is still up here
            assert!(g.hunters[0].stun > 0.0, "the hunter over the body is still staggered from the down");
        }
        // one hunter step with the grace up does not start a new grab (resolve_capture bails while escape_grace > 0)
        let prof = g.prof();
        g.update_hunters(0.1, &prof, &crate::audio::Sounds::default());
        assert!(g.escape.is_none(), "the grace blocks an immediate re-grab");
        assert!(!g.runner_downed(), "the revived runner is not downed again on the next frame");
    }

    /// A revive kit stands the runner up with the same post-revive grace as a hold-revive (game/items.rs).
    #[test]
    fn a_kit_revive_also_grants_the_post_revive_grace() {
        let (mut g, _) = game();
        g.end_run(0, "caught");
        assert!(g.runner_downed());
        g.give(crate::game::items::Item::ReviveKit, 1);
        assert!(g.use_revive_kit(), "the kit stands the runner up");
        assert!(!g.runner_downed());
        assert!(g.escape.is_none() && g.escape_grace > 0.0, "the kit revive grants the re-grab immunity too");
    }

    /// Solo with nobody to revive and no kit: a capture ends the run at once, with no down and no repeated notice. Guards
    /// the solo path against the co-op grace change above.
    #[test]
    fn a_solo_capture_still_ends_the_run_with_no_down_and_no_grace() {
        let mut g = solo();
        assert!(!g.runner_has_reviver());
        g.end_run(0, "contact");
        assert!(!g.runner_downed(), "nobody to revive: no down");
        assert!(g.report.is_some(), "the capture ends the run at once");
        assert_eq!(g.escape_grace, 0.0, "a run that simply ends grants no post-revive grace");
    }

    /// Single player with only an NPC bot reviver (the a2e142e regression). The bot stands the runner up a fixed number
    /// of times (`max_bot_revives`), then the next capture ENDS the run instead of downing again (Change B — no more
    /// immortal auto-revive loop). Each revive leaves the chasing hunter that caught the runner in its chase, NOT frozen
    /// or shoved into a search (Change A — the "hunters standing still" / "instantly stun on collision" regression).
    /// Fails before the fix: the catcher would be in `State::Search`, and the final capture would down again (no report).
    #[test]
    fn a_bot_revive_budget_ends_the_run_and_leaves_the_chaser_chasing() {
        use crate::hunters::brain::State;
        let (mut g, _) = game();
        // solo: no human teammate, only a joined bot can revive
        g.room_teammates = 0;
        g.npcs[0].joined = true;
        assert!(g.runner_has_reviver(), "the joined bot is a reviver");
        let id = g.npcs[0].id;
        let budget = rules().max_bot_revives;
        assert!(budget >= 1, "the fixture needs at least one bot revive");
        for _ in 0..budget {
            let (level, pos) = (g.runner_level(), g.p.pos);
            let had_hunter = chase_at(&mut g, level, pos);
            g.end_run(0, "contact");
            assert!(g.runner_downed() && g.report.is_none(), "with a bot reviver the capture downs the runner");
            // the bot stands beside the runner and holds the three-second revive
            beside_runner(&mut g, id);
            for _ in 0..31 {
                g.tick_revives(0.1, &idle());
            }
            assert!(!g.runner_downed(), "the bot's hold stands the runner back up");
            assert!(g.escape_grace > 0.0, "the revive grants the re-grab grace");
            if had_hunter {
                assert!(
                    matches!(g.hunters[0].state, State::Chase),
                    "the catcher keeps chasing: the revive does not freeze it or shove it into a search"
                );
            }
        }
        // the bot-revive budget is spent: the next capture ends the run instead of downing the runner again
        let level = g.runner_level();
        let pos = g.p.pos;
        chase_at(&mut g, level, pos);
        g.end_run(0, "contact");
        assert!(!g.runner_downed(), "the spent bot-revive budget stops another down");
        assert!(g.report.is_some(), "the grab now ends the run");
    }

    /// Contrast with the budget test: in co-op (a human teammate in the room) the repeated down/revive is never capped —
    /// the teammate down mechanic stays unlimited, so the bot-revive cap applies only to the solo-with-a-bot case.
    #[test]
    fn co_op_down_and_revive_is_not_capped_by_the_bot_budget() {
        let (mut g, _) = game();
        g.room_teammates = 1;
        g.npcs[0].joined = true;
        let id = g.npcs[0].id;
        let rounds = rules().max_bot_revives + 2;
        for _ in 0..rounds {
            g.end_run(0, "contact");
            assert!(g.runner_downed() && g.report.is_none(), "co-op downs the runner, it never ends the run here");
            beside_runner(&mut g, id);
            for _ in 0..31 {
                g.tick_revives(0.1, &idle());
            }
            assert!(!g.runner_downed(), "the hold stands the runner back up");
        }
        g.end_run(0, "contact");
        assert!(g.runner_downed() && g.report.is_none(), "past the bot budget, co-op still downs and never ends");
    }
}
