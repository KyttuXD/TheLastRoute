//! Host-side coordination of the hunters on one level (game/hunters/ai/hunter_director.gd),
//! ported rule for rule. Every mode cooperates, scaled by difficulty: a hunter that sees a
//! runner calls nearby idle hunters (share), a small team splits so one comes straight in while
//! the others flank or cover an exit, crowded idle hunters spread out (scatter), and no runner
//! gets more than `cap` hunters committed (the surplus is sent away). Relay rush (tip 088): once
//! RELAY_RUSH_COUNT relays have been recovered and that has held RELAY_RUSH_DELAY seconds, a few
//! free hunters investigate the nearest remaining relays. It only ever directs wandering / idle
//! hunters (Brain::direct_to), never a chasing, stunned or inactive one.

use super::brain::{cell_of, dist, Brain, State};
use crate::config::Difficulty;
use crate::world::godot_sort::sort_custom;
use crate::world::rng::Rng;
use crate::world::{Cell, Facility};
use std::collections::HashMap;

pub struct DirectorProfile {
    pub share_radius: f32,
    pub share_chance: f32,
    pub cap: i32,
    pub team: i32,
    pub flank: f32,
    pub cover: f32,
    pub gap: f32,
    pub interval: f32,
    pub rushers: i32,
}

/// PROFILES (relaxed, standard, hard). Nightmare has no director profile port yet (TODO.md), so it
/// reuses the Hard profile.
pub fn profile(d: Difficulty) -> DirectorProfile {
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
        },
        Difficulty::Hard | Difficulty::Nightmare => DirectorProfile {
            share_radius: 24.0,
            share_chance: 1.0,
            cap: 3,
            team: 2,
            flank: 0.6,
            cover: 0.5,
            gap: 9.0,
            interval: 1.0,
            rushers: 2,
        },
    }
}

pub const DIRECTED_HOLD: f32 = 9.0;
pub const SCATTER_HOLD: f32 = 16.0;
pub const SCATTER_CLEAR: f32 = 12.0;
pub const SEPARATION: f32 = 1.1;
pub const SEPARATION_SPEED: f32 = 2.5;
pub const RELAY_RUSH_COUNT: i32 = 7;
pub const RELAY_RUSH_DELAY: f32 = 45.0;
pub const RUSH_HOLD: f32 = 14.0;
pub const RUSH_ARRIVED: f32 = 3.0;

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub shares: i32,
    pub flanks: i32,
    pub covers: i32,
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
        }
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
            && self.clock - self.rush_since >= RELAY_RUSH_DELAY
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
        if self.relay_rush_enabled && self.relays_collected >= RELAY_RUSH_COUNT {
            if self.rush_since < 0.0 {
                self.rush_since = self.clock;
            }
        } else {
            self.rush_since = -1.0;
        }
        let p = profile(self.mode);
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
                let mut role = 0; // 0 share, 1 flank, 2 cover
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
                    _ => self.stats.shares += 1,
                }
                committed[r] += 1;
                sent += 1;
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
        let mut cands: Vec<(f32, usize)> = vec![];
        for i in 0..hunters.len() {
            if !is_free(&hunters[i]) || self.held_until(i) > self.clock {
                continue;
            }
            let at = hunters[i].pos;
            if runners.iter().any(|r| dist(at, *r) < SCATTER_CLEAR) {
                continue;
            }
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
