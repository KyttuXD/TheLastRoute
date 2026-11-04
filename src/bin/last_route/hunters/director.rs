//! Host-side coordination of the hunters on one level (game/hunters/ai/hunter_director.gd),
//! ported rule for rule and extended for smarter squads. Every mode cooperates, scaled by
//! difficulty: a hunter that sees a runner calls nearby idle hunters (share), a small team splits
//! so one comes straight in while the others flank, cover an exit or take their own room (split),
//! a free hunter that hears the others chase flanks the runner (hear), crowded idle hunters spread
//! out (scatter), and no runner gets more than `cap` hunters committed (the surplus is sent away).
//! A search point a hunter has cleared is marked for the whole squad (cleared), so directed hunters
//! are not sent back into it. Relay rush (tip 088): once RELAY_RUSH_COUNT relays have been
//! recovered and that has held RELAY_RUSH_DELAY seconds, a few free hunters investigate the nearest
//! remaining relays. It only ever directs wandering / idle hunters (Brain::direct_to), never a
//! chasing, stunned or inactive one.
//!
//! Every number is per difficulty in data/hunters/director.json (`lr.hunter_director/1`).

use super::brain::{cell_of, dist, Brain, State};
use crate::config::Difficulty;
use crate::world::godot_sort::sort_custom;
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One difficulty's squad rules. Read from data/hunters/director.json (see `profile`).
#[derive(Clone, Debug, PartialEq)]
pub struct DirectorProfile {
    /// Radius (cells) a spotter calls idle hunters from, and the share radius of a team.
    pub share_radius: f32,
    /// Chance each called hunter actually comes.
    pub share_chance: f32,
    /// Most hunters committed to one runner at a time; the surplus is sent away.
    pub cap: i32,
    /// Most hunters one spotter calls up at once.
    pub team: i32,
    /// Chance a team member after the first flanks the sighting instead of coming straight in.
    pub flank: f32,
    /// Chance a team member after the first covers the nearest relay exit.
    pub cover: f32,
    /// Crowding distance (cells): idle hunters closer than this walk apart (scatter).
    pub gap: f32,
    /// Seconds between director passes.
    pub interval: f32,
    /// Free hunters the relay rush sends out at once.
    pub rushers: i32,
    /// Distance (cells) a free hunter can hear a chasing hunter from, with no spotter needed.
    pub hear_radius: f32,
    /// Chance a free hunter that hears a chase flanks the runner it is after (0 turns hearing off).
    pub hear_flank: f32,
    /// Chance a team member (after the first) takes its own room instead of the sighting (0 turns it off).
    pub split: f32,
    /// Distance (cells) from the sighting to a split point.
    pub split_dist: f32,
    /// Radians between team slots when they fan out to split points.
    pub split_turn: f32,
    /// A search point a hunter came within this many cells of is cleared for the squad (0 turns it off).
    pub clear_radius: f32,
    /// How many cleared search points the squad remembers.
    pub clear_memory: usize,
}

/// The parsed director file: one profile per difficulty id ("relaxed", "standard", "hard", "nightmare").
#[derive(Clone, Debug, Default)]
pub struct Rules {
    by_difficulty: HashMap<String, DirectorProfile>,
}

pub const DIRECTOR_SCHEMA: &str = "lr.hunter_director/1";

/// Parses data/hunters/director.json. A difficulty entry takes every field from its own object,
/// falling back to the optional "default" object. Returns None when the schema is wrong or a field is missing.
pub fn parse_rules(text: &str) -> Option<Rules> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("schema")?.as_str()? != DIRECTOR_SCHEMA {
        return None;
    }
    let default = v
        .get("default")
        .and_then(|d| d.as_object())
        .cloned()
        .unwrap_or_default();
    let mut by_difficulty = HashMap::new();
    for (id, body) in v.get("difficulty")?.as_object()? {
        let mut merged = default.clone();
        for (k, x) in body.as_object()? {
            merged.insert(k.clone(), x.clone());
        }
        by_difficulty.insert(id.clone(), profile_from(&merged)?);
    }
    Some(Rules { by_difficulty })
}

fn profile_from(m: &serde_json::Map<String, serde_json::Value>) -> Option<DirectorProfile> {
    let num = |key: &str| m.get(key).and_then(|x| x.as_f64()).map(|x| x as f32);
    Some(DirectorProfile {
        share_radius: num("share_radius")?,
        share_chance: num("share_chance")?,
        cap: num("cap")? as i32,
        team: num("team")? as i32,
        flank: num("flank")?,
        cover: num("cover")?,
        gap: num("gap")?,
        interval: num("interval")?,
        rushers: num("rushers")? as i32,
        hear_radius: num("hear_radius")?,
        hear_flank: num("hear_flank")?,
        split: num("split")?,
        split_dist: num("split_dist")?,
        split_turn: num("split_turn")?,
        clear_radius: num("clear_radius")?,
        clear_memory: num("clear_memory")?.max(0.0) as usize,
    })
}

/// The embedded squad rules (data/hunters/director.json), read once. Empty when the file is missing or broken,
/// in which case `profile` uses the built-in numbers.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("hunters/director.json")
            .next()
            .and_then(|(_, text)| parse_rules(text))
            .unwrap_or_default()
    })
}

/// This difficulty's squad rules from data/hunters/director.json, or the built-in numbers when the file has no entry.
pub fn profile(d: Difficulty) -> DirectorProfile {
    rules()
        .by_difficulty
        .get(d.id())
        .cloned()
        .unwrap_or_else(|| builtin_profile(d))
}

/// The built-in numbers, used only when data/hunters/director.json is missing or has no entry for the
/// difficulty. They equal the data file (test `builtin_matches_the_data_file`); change the data, not these.
pub fn builtin_profile(d: Difficulty) -> DirectorProfile {
    match d {
        Difficulty::Relaxed => DirectorProfile {
            share_radius: 10.0,
            share_chance: 0.5,
            cap: 2,
            team: 1,
            flank: 0.0,
            cover: 0.0,
            gap: 7.0,
            interval: 2.0,
            rushers: 1,
            hear_radius: 0.0,
            hear_flank: 0.0,
            split: 0.0,
            split_dist: 6.0,
            split_turn: 2.1,
            clear_radius: 0.0,
            clear_memory: 0,
        },
        Difficulty::Standard => DirectorProfile {
            share_radius: 16.0,
            share_chance: 0.8,
            cap: 3,
            team: 2,
            flank: 0.1,
            cover: 0.25,
            gap: 8.0,
            interval: 1.5,
            rushers: 2,
            hear_radius: 0.0,
            hear_flank: 0.0,
            split: 0.0,
            split_dist: 6.0,
            split_turn: 2.1,
            clear_radius: 4.0,
            clear_memory: 6,
        },
        Difficulty::Hard => DirectorProfile {
            share_radius: 24.0,
            share_chance: 1.0,
            cap: 3,
            team: 2,
            flank: 0.6,
            cover: 0.5,
            gap: 9.0,
            interval: 1.0,
            rushers: 2,
            hear_radius: 16.0,
            hear_flank: 0.5,
            split: 0.4,
            split_dist: 7.0,
            split_turn: 2.1,
            clear_radius: 4.0,
            clear_memory: 8,
        },
        Difficulty::Nightmare => DirectorProfile {
            share_radius: 24.0,
            share_chance: 1.0,
            cap: 3,
            team: 2,
            flank: 0.6,
            cover: 0.5,
            gap: 9.0,
            interval: 1.0,
            rushers: 2,
            hear_radius: 20.0,
            hear_flank: 0.7,
            split: 0.6,
            split_dist: 7.0,
            split_turn: 2.1,
            clear_radius: 5.0,
            clear_memory: 10,
        },
    }
}

pub const DIRECTED_HOLD: f32 = 9.0;
pub const SCATTER_HOLD: f32 = 16.0;
pub const SCATTER_CLEAR: f32 = 12.0;
pub const SEPARATION: f32 = 1.1;
pub const SEPARATION_SPEED: f32 = 2.5;
/// Relay rush starts at this many relays (data/progression.json relay_rush.relays; Standard 7).
pub fn relay_rush_count() -> i32 {
    crate::game::progression::Progression::load().rush_relays
}
/// ... once that has held this long, in seconds (relay_rush.delay_seconds; Standard 45.0).
pub fn relay_rush_delay() -> f32 {
    crate::game::progression::Progression::load().rush_delay
}
/// Whether a searching hunter may join the relay rush (relay_rush.include_searching), and the
/// runner gap it must keep (relay_rush.search_clear). None when the rush does not take searchers.
pub fn relay_rush_search_clear() -> Option<f32> {
    let p = crate::game::progression::Progression::load();
    p.rush_search.then_some(p.rush_search_clear)
}
/// A hunter the rush may direct: a free hunter (Wander or Idle) more than SCATTER_CLEAR from every
/// runner, or, when `search` is set, a hunter in Search at least that far from every runner.
fn rush_eligible(b: &Brain, runners: &[(f32, f32)], search: Option<f32>) -> bool {
    if !b.active || b.stun > 0.0 {
        return false;
    }
    let clear = match b.state {
        State::Wander | State::Idle => SCATTER_CLEAR,
        State::Search => match search {
            Some(c) => c,
            None => return false,
        },
        _ => return false,
    };
    !runners.iter().any(|r| dist(b.pos, *r) < clear)
}
pub const RUSH_HOLD: f32 = 14.0;
pub const RUSH_ARRIVED: f32 = 3.0;

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub shares: i32,
    pub flanks: i32,
    pub covers: i32,
    pub splits: i32,
    pub heard: i32,
    pub cleared: i32,
    pub scatters: i32,
    pub released: i32,
    pub rushes: i32,
}

pub struct Director {
    pub mode: Difficulty,
    pub stats: Stats,
    pub relays_collected: i32,
    pub relay_rush_enabled: bool,
    rng: Rng,
    clock: f32,
    timer: f32,
    /// hunter index -> clock until which the director leaves it alone
    held: HashMap<usize, f32>,
    /// hunter index -> the relay cell it was sent to
    rushing: HashMap<usize, Cell>,
    rush_since: f32,
    /// search points the squad has cleared, newest last (bounded by the profile's clear_memory)
    cleared: Vec<(f32, f32)>,
    /// hunter index -> the search point it last cleared, so one point is only marked once
    last_cleared: HashMap<usize, (f32, f32)>,
}

fn is_free(b: &Brain) -> bool {
    b.active && b.stun <= 0.0 && matches!(b.state, State::Wander | State::Idle)
}

fn nearest_runner(p: (f32, f32), runners: &[(f32, f32)]) -> Option<usize> {
    let mut best = None;
    let mut gap = f32::INFINITY;
    for (i, r) in runners.iter().enumerate() {
        let d = dist(p, *r);
        if d < gap {
            gap = d;
            best = Some(i);
        }
    }
    best
}

fn rotated(v: (f32, f32), a: f32) -> (f32, f32) {
    let (s, c) = a.sin_cos();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

fn normalized(v: (f32, f32)) -> (f32, f32) {
    let l = (v.0 * v.0 + v.1 * v.1).sqrt();
    if l > 0.0 {
        (v.0 / l, v.1 / l)
    } else {
        (0.0, 0.0)
    }
}

/// _open_near: an open spot within 2 cells of `point` that is at least 1 cell away and reachable.
fn open_near(fac: &Facility, point: (f32, f32), from: (f32, f32)) -> Option<(f32, f32)> {
    let found = fac.nearest_open(point.0, point.1, 2.0)?;
    if dist(found, from) < 1.0 || fac.find_path(cell_of(from), cell_of(found)).is_empty() {
        return None;
    }
    Some(found)
}

/// A point `dist` cells from the sighting, turned `slot * turn` radians from the way the runner was heading.
/// Each team member gets its own slot, so the squad fans out into different rooms instead of one spot.
pub fn split_point(seen_at: (f32, f32), toward: (f32, f32), slot: i32, dist: f32, turn: f32) -> (f32, f32) {
    let d = rotated(toward, slot as f32 * turn);
    (seen_at.0 + d.0 * dist, seen_at.1 + d.1 * dist)
}

/// Marks a cleared search point for the squad. A point within 1 cell of one already marked is not added again.
/// Only the newest `memory` points are kept. Returns true when the point was added.
pub fn remember_cleared(list: &mut Vec<(f32, f32)>, point: (f32, f32), memory: usize) -> bool {
    if memory == 0 || list.iter().any(|c| dist(*c, point) < 1.0) {
        return false;
    }
    list.push(point);
    let excess = list.len().saturating_sub(memory);
    list.drain(..excess);
    true
}

/// True when `point` lies within `radius` cells of a cleared search point. Always false when radius is 0 (off).
pub fn is_cleared(list: &[(f32, f32)], point: (f32, f32), radius: f32) -> bool {
    radius > 0.0 && list.iter().any(|c| dist(*c, point) <= radius)
}

impl Director {
    pub fn new(mode: Difficulty, seed_text: &str) -> Director {
        Director {
            mode,
            stats: Stats::default(),
            relays_collected: 0,
            relay_rush_enabled: true,
            rng: Rng::new(&format!("{seed_text}-{}", mode.id())),
            clock: 0.0,
            timer: 0.0,
            held: HashMap::new(),
            rushing: HashMap::new(),
            rush_since: -1.0,
            cleared: Vec::new(),
            last_cleared: HashMap::new(),
        }
    }

    /// The search points the squad has cleared, newest last. A search picker can skip these (Brain::pick_search_target).
    pub fn cleared(&self) -> &[(f32, f32)] {
        &self.cleared
    }

    fn held_until(&self, i: usize) -> f32 {
        self.held.get(&i).copied().unwrap_or(0.0)
    }

    fn send(&mut self, hunters: &mut [Brain], i: usize, point: (f32, f32), hold: f32) {
        hunters[i].direct_to(point);
        self.held.insert(i, self.clock + hold);
    }

    pub fn relay_rush_active(&self) -> bool {
        self.relay_rush_enabled
            && self.rush_since >= 0.0
            && self.clock - self.rush_since >= relay_rush_delay()
    }

    /// Marks the search point of each searching hunter that has come within clear_radius of it (once per point).
    fn note_cleared(&mut self, hunters: &[Brain], p: &DirectorProfile) {
        if p.clear_radius <= 0.0 || p.clear_memory == 0 {
            return;
        }
        for (i, b) in hunters.iter().enumerate() {
            if !b.active || b.state != State::Search {
                continue;
            }
            let Some(t) = b.search_target else { continue };
            if dist(b.pos, t) > p.clear_radius || self.last_cleared.get(&i) == Some(&t) {
                continue;
            }
            self.last_cleared.insert(i, t);
            if remember_cleared(&mut self.cleared, t, p.clear_memory) {
                self.stats.cleared += 1;
            }
        }
    }

    /// One frame. `targets[i]` is the runner position hunter i is after (None when it has none);
    /// `relays` the uncollected relay cells.
    pub fn tick(
        &mut self,
        dt: f32,
        hunters: &mut [Brain],
        targets: &[Option<(f32, f32)>],
        fac: &Facility,
        relays: &[Cell],
    ) {
        self.clock += dt;
        self.timer += dt;
        if self.relay_rush_enabled && self.relays_collected >= relay_rush_count() {
            if self.rush_since < 0.0 {
                self.rush_since = self.clock;
            }
        } else {
            self.rush_since = -1.0;
        }
        let p = profile(self.mode);
        self.note_cleared(hunters, &p);
        if self.timer < p.interval {
            return;
        }
        self.timer = 0.0;
        let mut runners: Vec<(f32, f32)> = vec![];
        for t in targets.iter().flatten() {
            if !runners.iter().any(|r| dist(*r, *t) < 1.0) {
                runners.push(*t);
            }
        }
        if runners.is_empty() {
            self.relay_rush(hunters, fac, &runners, relays);
            return;
        }
        let n = hunters.len();
        // who is committed to which runner
        let mut committed = vec![0i32; runners.len()];
        for i in 0..n {
            let b = &hunters[i];
            if !b.active {
                continue;
            }
            let directed = self.held_until(i) > self.clock && b.state == State::Investigate;
            if b.state == State::Chase || directed {
                if let Some(w) = nearest_runner(b.pos, &runners) {
                    if dist(b.pos, runners[w]) <= p.share_radius + 8.0 {
                        committed[w] += 1;
                    }
                }
            }
        }
        // share: one spotter per runner calls up a team
        for r in 0..runners.len() {
            let spotter = (0..n).find(|&i| {
                let b = &hunters[i];
                b.active
                    && b.state == State::Chase
                    && b.direct_visible
                    && nearest_runner(b.pos, &runners) == Some(r)
            });
            let Some(spotter) = spotter else { continue };
            let spot_at = hunters[spotter].pos;
            let seen_at = runners[r];
            let members = (p.cap - committed[r]).min(p.team);
            if members <= 0 {
                continue;
            }
            let mut cands: Vec<(f32, usize)> = (0..n)
                .filter(|&i| {
                    i != spotter && is_free(&hunters[i]) && self.held_until(i) <= self.clock
                })
                .map(|i| (dist(hunters[i].pos, seen_at), i))
                .filter(|(g, _)| *g <= p.share_radius)
                .collect();
            sort_custom(&mut cands, |a, b| a.0 < b.0);
            let mut sent = 0;
            for (_, i) in cands {
                if sent >= members {
                    break;
                }
                if self.rng.next_f32() >= p.share_chance {
                    continue;
                }
                let mut point = seen_at;
                let mut role = 0; // 0 share, 1 flank, 2 cover, 3 split
                let toward = if dist(seen_at, spot_at) > 0.1 {
                    normalized((seen_at.0 - spot_at.0, seen_at.1 - spot_at.1))
                } else {
                    (1.0, 0.0)
                };
                if sent > 0 && self.rng.next_f32() < p.flank {
                    role = 1;
                    let side = if sent % 2 == 0 { -1.0 } else { 1.0 };
                    let d = rotated(toward, side * 1.2);
                    point = (seen_at.0 + d.0 * 4.0, seen_at.1 + d.1 * 4.0);
                } else if sent > 0 && self.rng.next_f32() < p.cover {
                    role = 2;
                    point = exit_point(relays, seen_at, toward);
                } else if sent > 0 && p.split > 0.0 && self.rng.next_f32() < p.split {
                    role = 3;
                    point = split_point(seen_at, toward, sent, p.split_dist, p.split_turn);
                }
                // a point the squad already cleared is not sent back into: the member comes to the sighting
                if role > 0 && is_cleared(&self.cleared, point, p.clear_radius) {
                    point = seen_at;
                    role = 0;
                }
                let from = hunters[i].pos;
                let mut open = open_near(fac, point, from);
                if open.is_none() {
                    open = open_near(fac, seen_at, from);
                    role = 0;
                }
                let Some(open) = open else { continue };
                self.send(hunters, i, open, DIRECTED_HOLD);
                match role {
                    1 => self.stats.flanks += 1,
                    2 => self.stats.covers += 1,
                    3 => self.stats.splits += 1,
                    _ => self.stats.shares += 1,
                }
                committed[r] += 1;
                sent += 1;
            }
        }
        // hear: a free hunter that hears a chase nearby (no spotter needed) flanks the runner that chase is after
        if p.hear_flank > 0.0 && p.hear_radius > 0.0 {
            for i in 0..n {
                if !is_free(&hunters[i]) || self.held_until(i) > self.clock {
                    continue;
                }
                let at = hunters[i].pos;
                let mut heard: Option<(f32, usize)> = None;
                for j in 0..n {
                    if j == i || !hunters[j].active || hunters[j].state != State::Chase {
                        continue;
                    }
                    let gap = dist(at, hunters[j].pos);
                    if gap <= p.hear_radius && heard.map_or(true, |(g, _)| gap < g) {
                        heard = Some((gap, j));
                    }
                }
                let Some((_, j)) = heard else { continue };
                let Some(r) = nearest_runner(hunters[j].pos, &runners) else { continue };
                if committed[r] >= p.cap || self.rng.next_f32() >= p.hear_flank {
                    continue;
                }
                let runner = runners[r];
                let toward = normalized((runner.0 - hunters[j].pos.0, runner.1 - hunters[j].pos.1));
                let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                let d = rotated(toward, side * 1.2);
                let point = (runner.0 + d.0 * 4.0, runner.1 + d.1 * 4.0);
                let Some(open) = open_near(fac, point, at).or_else(|| open_near(fac, runner, at)) else {
                    continue;
                };
                self.send(hunters, i, open, DIRECTED_HOLD);
                self.stats.heard += 1;
                committed[r] += 1;
            }
        }
        // overload: directed (not chasing) hunters over the cap are sent away
        for r in 0..runners.len() {
            let mut excess = committed[r] - p.cap;
            if excess <= 0 {
                continue;
            }
            let mut extras: Vec<(f32, usize)> = (0..n)
                .filter(|&i| {
                    self.held_until(i) > self.clock
                        && hunters[i].state == State::Investigate
                        && nearest_runner(hunters[i].pos, &runners) == Some(r)
                })
                .map(|i| (dist(hunters[i].pos, runners[r]), i))
                .collect();
            sort_custom(&mut extras, |a, b| a.0 > b.0);
            for (_, i) in extras {
                if excess <= 0 {
                    break;
                }
                let at = hunters[i].pos;
                let away = normalized((at.0 - runners[r].0, at.1 - runners[r].1));
                if let Some(open) = open_near(fac, (at.0 + away.0 * 14.0, at.1 + away.1 * 14.0), at)
                {
                    self.send(hunters, i, open, SCATTER_HOLD);
                    self.stats.released += 1;
                    excess -= 1;
                }
            }
        }
        // scatter: crowded idle hunters walk apart
        for i in 0..n {
            if !is_free(&hunters[i]) || self.held_until(i) > self.clock {
                continue;
            }
            let at = hunters[i].pos;
            if runners.iter().any(|r| dist(at, *r) < SCATTER_CLEAR) {
                continue;
            }
            let mut crowd = (0.0, 0.0);
            let mut crowded = false;
            for j in 0..n {
                if j == i || !hunters[j].active {
                    continue;
                }
                let gap = dist(at, hunters[j].pos);
                if gap < p.gap {
                    crowded = true;
                    let k = gap.max(0.5);
                    crowd.0 += (at.0 - hunters[j].pos.0) / k;
                    crowd.1 += (at.1 - hunters[j].pos.1) / k;
                }
            }
            if !crowded {
                continue;
            }
            let away = if (crowd.0 * crowd.0 + crowd.1 * crowd.1).sqrt() > 0.01 {
                normalized(crowd)
            } else {
                let a = self.rng.next_f32() * std::f32::consts::TAU;
                (a.cos(), a.sin())
            };
            let to = (at.0 + away.0 * p.gap * 1.8, at.1 + away.1 * p.gap * 1.8);
            if let Some(open) = open_near(fac, to, at) {
                self.send(hunters, i, open, SCATTER_HOLD);
                self.stats.scatters += 1;
            }
        }
        self.relay_rush(hunters, fac, &runners, relays);
    }

    fn relay_rush(
        &mut self,
        hunters: &mut [Brain],
        fac: &Facility,
        runners: &[(f32, f32)],
        relays: &[Cell],
    ) {
        if !self.relay_rush_active() {
            self.rushing.clear();
            return;
        }
        if relays.is_empty() {
            return;
        }
        let mut claimed: Vec<Cell> = vec![];
        let ids: Vec<usize> = self.rushing.keys().copied().collect();
        for i in ids {
            let keep = hunters
                .get(i)
                .map(|b| {
                    b.active && self.held_until(i) > self.clock && b.state == State::Investigate
                })
                .unwrap_or(false);
            if keep {
                claimed.push(self.rushing[&i]);
            } else {
                self.rushing.remove(&i);
            }
        }
        let mut slots = profile(self.mode).rushers - self.rushing.len() as i32;
        if slots <= 0 {
            return;
        }
        let centre = |c: &Cell| (c.x as f32 + 0.5, c.y as f32 + 0.5);
        let search = relay_rush_search_clear();
        let mut cands: Vec<(f32, usize)> = vec![];
        for i in 0..hunters.len() {
            if self.held_until(i) > self.clock || !rush_eligible(&hunters[i], runners, search) {
                continue;
            }
            let at = hunters[i].pos;
            let best = relays
                .iter()
                .map(|c| dist(at, centre(c)))
                .fold(f32::INFINITY, f32::min);
            cands.push((best, i));
        }
        sort_custom(&mut cands, |a, b| a.0 < b.0);
        for (_, i) in cands {
            if slots <= 0 {
                break;
            }
            let at = hunters[i].pos;
            let (mut pick, mut pick_gap, mut fallback, mut fallback_gap) =
                (None, f32::INFINITY, None, f32::INFINITY);
            for c in relays {
                let gap = dist(at, centre(c));
                if gap < fallback_gap {
                    fallback_gap = gap;
                    fallback = Some(*c);
                }
                if !claimed.contains(c) && gap < pick_gap {
                    pick_gap = gap;
                    pick = Some(*c);
                }
            }
            if pick.is_none() {
                pick = fallback;
                pick_gap = fallback_gap;
            }
            let Some(relay) = pick else { continue };
            if pick_gap <= RUSH_ARRIVED {
                continue;
            }
            let Some(open) = open_near(fac, centre(&relay), at) else {
                continue;
            };
            self.send(hunters, i, open, RUSH_HOLD);
            self.rushing.insert(i, relay);
            claimed.push(relay);
            self.stats.rushes += 1;
            slots -= 1;
        }
    }
}

/// _exit_point: the nearest relay within 16 cells of the sighting, else straight ahead.
fn exit_point(relays: &[Cell], seen_at: (f32, f32), toward: (f32, f32)) -> (f32, f32) {
    let mut best = f32::INFINITY;
    let mut found = (seen_at.0 + toward.0 * 6.0, seen_at.1 + toward.1 * 6.0);
    for c in relays {
        let at = (c.x as f32 + 0.5, c.y as f32 + 0.5);
        let gap = dist(at, seen_at);
        if gap < best && gap <= 16.0 {
            best = gap;
            found = at;
        }
    }
    found
}

/// HunterDirector.separate: active hunters closer than SEPARATION push apart, skipping pushes
/// into walls or props. Returns how many moved.
pub fn separate(dt: f32, hunters: &mut [Brain], fac: &Facility) -> i32 {
    let mut pushed = 0;
    for i in 0..hunters.len() {
        if !hunters[i].active {
            continue;
        }
        let mut push = (0.0f32, 0.0f32);
        for j in 0..hunters.len() {
            if i == j || !hunters[j].active {
                continue;
            }
            // a hunter on a deck and one on the floor below it do not crowd each other
            if (hunters[i].y - hunters[j].y).abs() > super::flight_edges::rules().same_layer_eps {
                continue;
            }
            let off = (
                hunters[i].pos.0 - hunters[j].pos.0,
                hunters[i].pos.1 - hunters[j].pos.1,
            );
            let gap = (off.0 * off.0 + off.1 * off.1).sqrt();
            if gap >= SEPARATION {
                continue;
            }
            let away = if gap > 0.001 {
                (off.0 / gap, off.1 / gap)
            } else {
                rotated((1.0, 0.0), i as f32 * 2.4)
            };
            let k = (SEPARATION - gap) / SEPARATION;
            push.0 += away.0 * k;
            push.1 += away.1 * k;
        }
        if push == (0.0, 0.0) {
            continue;
        }
        let len = (push.0 * push.0 + push.1 * push.1).sqrt();
        let lim = if len > 1.0 {
            (push.0 / len, push.1 / len)
        } else {
            push
        };
        let to = (
            hunters[i].pos.0 + lim.0 * SEPARATION_SPEED * dt,
            hunters[i].pos.1 + lim.1 * SEPARATION_SPEED * dt,
        );
        // can_occupy at its default radius (DEFAULT_AGENT_RADIUS 0.23)
        if !fac.can_occupy(to.0 as f64, to.1 as f64, 0.23) {
            continue;
        }
        hunters[i].pos = to;
        pushed += 1;
    }
    pushed
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = include_str!("../data/hunters/director.json");

    const ALL: [Difficulty; 4] = [
        Difficulty::Relaxed,
        Difficulty::Standard,
        Difficulty::Hard,
        Difficulty::Nightmare,
    ];

    #[test]
    fn data_file_parses_with_an_entry_for_every_difficulty() {
        let r = parse_rules(DATA).expect("director.json parses");
        for d in ALL {
            assert!(r.by_difficulty.contains_key(d.id()), "missing entry for {}", d.id());
        }
    }

    #[test]
    fn standard_matches_the_current_game() {
        let r = parse_rules(DATA).unwrap();
        let p = &r.by_difficulty["standard"];
        assert_eq!(p.share_radius, 16.0);
        assert_eq!(p.share_chance, 0.8);
        assert_eq!(p.cap, 3);
        assert_eq!(p.team, 2);
        assert_eq!(p.flank, 0.1);
        assert_eq!(p.cover, 0.25);
        assert_eq!(p.gap, 8.0);
        assert_eq!(p.interval, 1.5);
        assert_eq!(p.rushers, 2);
        // the new squad behaviours stay off in Standard, so its random draws and relay rush are unchanged
        assert_eq!(p.hear_flank, 0.0);
        assert_eq!(p.split, 0.0);
    }

    #[test]
    fn builtin_matches_the_data_file() {
        let r = parse_rules(DATA).unwrap();
        for d in ALL {
            assert_eq!(builtin_profile(d), r.by_difficulty[d.id()], "{}", d.id());
        }
    }

    #[test]
    fn hearing_and_splitting_are_on_for_hard_and_off_for_relaxed() {
        assert!(profile(Difficulty::Hard).hear_radius > 0.0);
        assert!(profile(Difficulty::Hard).split > 0.0);
        assert_eq!(profile(Difficulty::Relaxed).hear_flank, 0.0);
        assert_eq!(profile(Difficulty::Relaxed).split, 0.0);
        assert_eq!(profile(Difficulty::Relaxed).clear_radius, 0.0);
    }

    #[test]
    fn changing_a_value_in_the_data_changes_the_profile() {
        let mut v: serde_json::Value = serde_json::from_str(DATA).unwrap();
        v["difficulty"]["hard"]["hear_radius"] = serde_json::json!(40.0);
        v["difficulty"]["hard"]["clear_memory"] = serde_json::json!(3);
        let r = parse_rules(&v.to_string()).unwrap();
        assert_eq!(r.by_difficulty["hard"].hear_radius, 40.0);
        assert_eq!(r.by_difficulty["hard"].clear_memory, 3);
        // the other difficulties are untouched
        assert_eq!(r.by_difficulty["standard"], builtin_profile(Difficulty::Standard));
    }

    #[test]
    fn a_wrong_schema_or_a_missing_field_is_refused() {
        assert!(parse_rules(r#"{"schema":"lr.other/1","difficulty":{}}"#).is_none());
        let mut v: serde_json::Value = serde_json::from_str(DATA).unwrap();
        v["difficulty"]["hard"].as_object_mut().unwrap().remove("split_turn");
        assert!(parse_rules(&v.to_string()).is_none());
    }

    #[test]
    fn cleared_points_are_marked_once_and_bounded_by_memory() {
        let mut list = vec![];
        assert!(remember_cleared(&mut list, (1.0, 1.0), 2));
        assert!(!remember_cleared(&mut list, (1.2, 1.0), 2), "within 1 cell of a marked point");
        remember_cleared(&mut list, (5.0, 5.0), 2);
        remember_cleared(&mut list, (9.0, 9.0), 2);
        assert_eq!(list, vec![(5.0, 5.0), (9.0, 9.0)]);
    }

    #[test]
    fn zero_memory_remembers_nothing() {
        let mut list = vec![];
        assert!(!remember_cleared(&mut list, (1.0, 1.0), 0));
        assert!(list.is_empty());
    }

    #[test]
    fn a_point_near_a_cleared_one_counts_as_cleared_only_when_the_radius_is_on() {
        let list = vec![(10.0, 10.0)];
        assert!(is_cleared(&list, (12.0, 10.0), 4.0));
        assert!(!is_cleared(&list, (20.0, 10.0), 4.0));
        assert!(!is_cleared(&list, (10.0, 10.0), 0.0), "radius 0 is off");
    }

    #[test]
    fn split_points_fan_the_team_out_at_the_data_distance() {
        let seen = (10.0, 10.0);
        let toward = (1.0, 0.0);
        let a = split_point(seen, toward, 1, 6.0, 2.1);
        let b = split_point(seen, toward, 2, 6.0, 2.1);
        assert!((dist(a, seen) - 6.0).abs() < 0.01);
        assert!(dist(a, b) > 4.0, "two team slots end up in different rooms");
        let far = split_point(seen, toward, 1, 9.0, 2.1);
        assert!((dist(far, seen) - 9.0).abs() < 0.01, "distance comes from the data");
    }

    #[test]
    fn stats_start_at_zero_for_every_squad_counter() {
        let s = Stats::default();
        assert_eq!(s.splits + s.heard + s.cleared + s.shares + s.flanks + s.covers, 0);
    }

    fn rush_hunter(state: State, at: (f32, f32)) -> Brain {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../data/hunters/aki.json")).unwrap();
        let def = crate::content::hunters::parse(&v, None, "").unwrap();
        let mut b = Brain::new(def, at, "RUSH", Difficulty::Standard);
        b.state = state;
        b.active = true;
        b
    }

    #[test]
    fn rush_standard_takes_a_searching_hunter_far_from_the_runner() {
        let runners = [(30.0, 0.0)];
        let b = rush_hunter(State::Search, (0.0, 0.0));
        let clear = relay_rush_search_clear();
        assert_eq!(clear, Some(12.0), "Standard data: include_searching on, search_clear 12");
        assert!(rush_eligible(&b, &runners, clear));
    }

    #[test]
    fn rush_ignores_searching_hunters_when_the_data_turns_it_off() {
        let runners = [(30.0, 0.0)];
        let b = rush_hunter(State::Search, (0.0, 0.0));
        assert!(!rush_eligible(&b, &runners, None));
    }

    #[test]
    fn rush_keeps_the_search_gap_from_the_runner() {
        let b = rush_hunter(State::Search, (0.0, 0.0));
        assert!(!rush_eligible(&b, &[(8.0, 0.0)], Some(12.0)), "too close to the runner");
        assert!(rush_eligible(&b, &[(8.0, 0.0)], Some(6.0)), "the data gap decides");
    }

    #[test]
    fn rush_free_hunters_keep_the_scatter_clear_gap() {
        let runners = [(13.0, 0.0)];
        let idle = rush_hunter(State::Idle, (0.0, 0.0));
        assert!(rush_eligible(&idle, &runners, None), "13 cells is more than 12");
        let near = [(11.0, 0.0)];
        assert!(!rush_eligible(&idle, &near, None), "11 cells is inside 12");
        let wander = rush_hunter(State::Wander, (0.0, 0.0));
        assert!(!rush_eligible(&wander, &near, Some(0.0)));
    }

    #[test]
    fn rush_never_takes_a_chasing_or_stunned_hunter() {
        let far = [(40.0, 0.0)];
        assert!(!rush_eligible(&rush_hunter(State::Chase, (0.0, 0.0)), &far, Some(12.0)));
        assert!(!rush_eligible(&rush_hunter(State::Peek, (0.0, 0.0)), &far, Some(12.0)));
        let mut stunned = rush_hunter(State::Search, (0.0, 0.0));
        stunned.stun = 1.0;
        assert!(!rush_eligible(&stunned, &far, Some(12.0)));
    }

    #[test]
    fn separation_ignores_hunters_at_different_heights() {
        let cat = crate::world::catalog::Catalog::load();
        let fac = Facility::generate("SEP-1", &crate::world::facility::Options::default(), &cat);
        let at = fac
            .floor_cells()
            .into_iter()
            .find(|c| fac.hunter_open(*c) && fac.can_occupy(c.x as f64 + 0.9, c.y as f64 + 0.5, 0.23))
            .expect("an open cell")
            .center();
        let pair = |dy: f32| {
            let mut a = rush_hunter(State::Wander, at);
            let mut b = rush_hunter(State::Wander, (at.0 + 0.2, at.1));
            b.y = dy;
            a.y = 0.0;
            vec![a, b]
        };
        let mut level = pair(0.0);
        assert!(separate(1.0 / 60.0, &mut level, &fac) > 0, "two hunters on one floor push apart");
        let mut stacked = pair(crate::world::route::catwalk::params().height);
        assert_eq!(separate(1.0 / 60.0, &mut stacked, &fac), 0, "a hunter on a deck and one below it do not crowd each other");
        assert_eq!(stacked[0].pos, at);
    }

    #[test]
    fn rush_search_data_changes_the_behaviour() {
        let p = crate::game::progression::Progression::parse(&serde_json::json!({
            "relay_rush": { "include_searching": false, "search_clear": 20.0 }
        }));
        assert!(!p.rush_search);
        assert_eq!(p.rush_search_clear, 20.0);
        let on = crate::game::progression::Progression::parse(&serde_json::json!({
            "relay_rush": { "search_clear": 20.0 }
        }));
        assert!(on.rush_search, "searchers stay in the rush unless the data turns them off");
    }
}
