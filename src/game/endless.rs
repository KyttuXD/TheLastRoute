//! The Infinite route inside a run: the live 3x3 chunk window that recentres as the runner
//! walks, stairwells between levels, chunk state that survives unloading (relays, batteries,
//! coins and pickups taken, cells discovered), and the hunter population that grows with
//! recovered relays (GameConfig.streamed_hunter_population).

use super::{FloorPickup, Game};
use crate::config::*;
use crate::content::Content;
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Gimmick;
use crate::world::catalog::Catalog;
use crate::world::facility::{Facility, STAIR_FLIGHT};
use crate::world::fixtures::Fixtures;
use crate::world::infinite::{self, ChunkKey, CHUNK};
use crate::world::rng::Rng;
use crate::world::Cell;
use std::collections::{HashMap, HashSet};

pub struct InfState {
    pub seed: String,
    pub level: i32,
    /// Chunk at the middle of the window.
    pub center: (i32, i32),
    cache: HashMap<ChunkKey, (Facility, Fixtures)>,
    /// (level, absolute x, absolute y, kind): 0 relay, 1 battery, 2 coin, 3 pickup.
    taken: HashSet<(i32, i32, i32, u8)>,
    seen: HashSet<(i32, i32, i32)>,
    pub chunks_visited: HashSet<ChunkKey>,
}

/// streamed_hunter_population: none before the activation relay, then the base lineup and one
/// more every STREAMED_HUNTER_RELAY_STEP relays (relaxed 3, standard 2, hard 1).
pub fn population(diff: Difficulty, relays: i32) -> i32 {
    let activation = diff.profile().activation_relays;
    if relays < activation {
        return 0;
    }
    let step = match diff {
        Difficulty::Relaxed => 3,
        Difficulty::Standard => 2,
        Difficulty::Hard => 1,
    };
    1 + (relays - activation) / step
}

impl InfState {
    pub fn new(seed: &str, level: i32) -> InfState {
        InfState {
            seed: seed.to_string(),
            level,
            center: (0, 0),
            cache: HashMap::new(),
            taken: HashSet::new(),
            seen: HashSet::new(),
            chunks_visited: HashSet::new(),
        }
    }

    fn origin(&self) -> (i32, i32) {
        ((self.center.0 - 1) * CHUNK, (self.center.1 - 1) * CHUNK)
    }
}

impl Game {
    pub fn is_infinite(&self) -> bool {
        self.inf.is_some()
    }

    /// Writes this window's live state into the persistent per-chunk sets.
    fn sync_out(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        let (ox, oy) = inf.origin();
        let l = inf.level;
        for (i, c) in self.fac.relays.iter().enumerate() {
            if self.relays_taken[i] {
                inf.taken.insert((l, c.x + ox, c.y + oy, 0));
            }
        }
        for (i, c) in self.fac.batteries.iter().enumerate() {
            if self.batteries_taken[i] {
                inf.taken.insert((l, c.x + ox, c.y + oy, 1));
            }
        }
        for (i, c) in self.fix.coins.iter().enumerate() {
            if self.coins_taken[i] {
                inf.taken.insert((l, c.x + ox, c.y + oy, 2));
            }
        }
        for p in self.pickups.iter().filter(|p| p.taken) {
            inf.taken.insert((
                l,
                p.pos.0.floor() as i32 + ox,
                p.pos.1.floor() as i32 + oy,
                3,
            ));
        }
        for (y, row) in self.discovered.iter().enumerate() {
            for (x, d) in row.iter().enumerate() {
                if *d {
                    inf.seen.insert((l, x as i32 + ox, y as i32 + oy));
                }
            }
        }
    }

    /// Restores what was already taken or discovered in the chunks now in the window.
    fn sync_in(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        let (ox, oy) = inf.origin();
        let l = inf.level;
        for (i, c) in self.fac.relays.iter().enumerate() {
            self.relays_taken[i] = inf.taken.contains(&(l, c.x + ox, c.y + oy, 0));
        }
        for (i, c) in self.fac.batteries.iter().enumerate() {
            self.batteries_taken[i] = inf.taken.contains(&(l, c.x + ox, c.y + oy, 1));
        }
        for (i, c) in self.fix.coins.iter().enumerate() {
            self.coins_taken[i] = inf.taken.contains(&(l, c.x + ox, c.y + oy, 2));
        }
        for p in self.pickups.iter_mut() {
            p.taken = inf.taken.contains(&(
                l,
                p.pos.0.floor() as i32 + ox,
                p.pos.1.floor() as i32 + oy,
                3,
            ));
        }
        for (y, row) in self.discovered.iter_mut().enumerate() {
            for (x, d) in row.iter_mut().enumerate() {
                *d = inf.seen.contains(&(l, x as i32 + ox, y as i32 + oy));
            }
        }
    }

    /// Builds the window around the current centre chunk and level.
    fn load_window(&mut self, content: &Content, catalog: &Catalog) {
        let Some(inf) = self.inf.as_mut() else { return };
        let (fac, fix) = infinite::window(
            &inf.seed,
            inf.center,
            inf.level,
            catalog,
            &content.loot,
            &mut inf.cache,
        );
        for dy in -1..=1 {
            for dx in -1..=1 {
                inf.chunks_visited.insert(ChunkKey {
                    cx: inf.center.0 + dx,
                    cy: inf.center.1 + dy,
                    level: inf.level,
                });
            }
        }
        self.depth = 1 - inf.level;
        self.fac = fac;
        self.fix = fix;
        self.adopt_floor(content);
        self.sync_in();
    }

    /// Enters level `level` with the runner at the window's centre lobby (a new run or a
    /// continued checkpoint).
    pub fn enter_infinite(&mut self, content: &Content, catalog: &Catalog) {
        self.load_window(content, catalog);
        self.p.place(self.fac.start.center());
        self.reset_hunters(content);
        self.fade = 1.0;
        self.reveal();
    }

    /// Recentres the window once the runner leaves the middle chunk.
    pub(super) fn stream_window(&mut self, content: &Content, catalog: &Catalog) {
        if self.inf.is_none() {
            return;
        }
        let rx = (self.p.pos.0 / CHUNK as f32).floor() as i32 - 1;
        let ry = (self.p.pos.1 / CHUNK as f32).floor() as i32 - 1;
        if rx == 0 && ry == 0 {
            return;
        }
        self.sync_out();
        if let Some(inf) = self.inf.as_mut() {
            inf.center = (inf.center.0 + rx, inf.center.1 + ry);
        }
        let (sx, sy) = ((rx * CHUNK) as f32, (ry * CHUNK) as f32);
        self.p.pos = (self.p.pos.0 - sx, self.p.pos.1 - sy);
        for h in self.hunters.iter_mut() {
            h.pos = (h.pos.0 - sx, h.pos.1 - sy);
            h.last_known = h.last_known.map(|k| (k.0 - sx, k.1 - sy));
            h.wander_target = h.wander_target.map(|k| (k.0 - sx, k.1 - sy));
            h.search_target = h.search_target.map(|k| (k.0 - sx, k.1 - sy));
            h.clear_path();
        }
        self.load_window(content, catalog);
        // hunters that fell outside the window come back in out of sight
        let size = (CHUNK * 3) as f32;
        let mut rng = Rng::new(&format!(
            "{}:RESPAWN:{}",
            self.level_seed(),
            self.time as i32
        ));
        for i in 0..self.hunters.len() {
            let p = self.hunters[i].pos;
            if p.0 < 0.0
                || p.1 < 0.0
                || p.0 >= size
                || p.1 >= size
                || !self.fac.body_fits(p.0, p.1, HUNTER_RADIUS)
            {
                if let Some(spot) = self.spawn_spot(&mut rng) {
                    self.hunters[i].pos = spot;
                    self.hunters[i].clear_path();
                }
            }
        }
    }

    /// Takes a stairwell to the level above or below.
    pub(super) fn take_stairs(&mut self, i: usize, content: &Content, catalog: &Catalog) {
        let s = self.fac.stairs[i];
        let other = Cell::new(
            s.cell.x + s.run.0 * (STAIR_FLIGHT + 1),
            s.cell.y + s.run.1 * (STAIR_FLIGHT + 1),
        );
        self.sync_out();
        if let Some(inf) = self.inf.as_mut() {
            inf.level += s.dir;
        }
        self.load_window(content, catalog);
        let landing = self.fac.stairs.iter().find(|t| t.cell == other).copied();
        match landing {
            Some(t) => {
                let (cx, cy) = t.cell.center();
                self.p.pos = self.fac.nearest_open_or(
                    cx - t.run.0 as f32 * 0.6,
                    cy - t.run.1 as f32 * 0.6,
                    PLAYER_RADIUS,
                );
                self.p.yaw = (t.run.0 as f32).atan2(t.run.1 as f32);
            }
            None => self.p.pos = self.fac.start.center(),
        }
        self.reset_hunters(content);
        let level = self.inf.as_ref().map(|i| i.level).unwrap_or(0);
        self.announce(&format!("LEVEL {level}"));
        self.fade = 1.0;
        self.save_checkpoint();
    }

    fn spawn_spot(&self, rng: &mut Rng) -> Option<(f32, f32)> {
        let cands: Vec<Cell> = self
            .fac
            .floor_cells()
            .into_iter()
            .filter(|c| {
                let p = c.center();
                let d = super::pdist(p, self.p.pos);
                d >= 12.0
                    && d <= 34.0
                    && self.fac.hunter_open(*c)
                    && !self.fac.has_line_of_sight(p, self.p.pos)
            })
            .collect();
        if cands.is_empty() {
            return None;
        }
        Some(cands[rng.next_index(cands.len())].center())
    }

    /// The level's lineup: route hunters for the current relay count, plus Nyxie and Solyth.
    fn reset_hunters(&mut self, content: &Content) {
        self.hunters.clear();
        self.reset_director();
        let seed = self.level_seed();
        let mut rng = Rng::new(&format!("{seed}:SPECIALS"));
        for (id, tag) in [("nyxie", "NYXIE"), ("solyth", "SOLYTH")] {
            if let Some(def) = content.hunter(id) {
                let p = self.spawn_spot(&mut rng).unwrap_or(self.fac.start.center());
                let mut b = Brain::new(def.clone(), p, &format!("{seed}:{tag}"), self.difficulty);
                if let Gimmick::Solyth(s) = &mut b.gimmick {
                    s.heat_locked = self.meters.heat_locked;
                }
                self.hunters.push(b);
            }
        }
        self.ensure_hunters(content);
    }

    /// Adds route hunters until the population matches the relays recovered so far.
    pub(super) fn ensure_hunters(&mut self, content: &Content) {
        let want = population(self.difficulty, self.relays_collected);
        let have = self
            .hunters
            .iter()
            .filter(|h| !matches!(h.def.id.as_str(), "nyxie" | "solyth"))
            .count() as i32;
        let seed = self.level_seed();
        for n in have..want {
            let mut rng = Rng::new(&format!("{seed}:SPAWN:{n}"));
            let Some(p) = self.spawn_spot(&mut rng) else {
                break;
            };
            let pick = crate::hunters::pick_for_floor(
                content,
                &format!("{seed}:H{n}"),
                self.depth,
                self.difficulty,
            );
            let mut b = Brain::new(
                content.hunters[pick].clone(),
                p,
                &format!("{seed}:HUNTER:{n}"),
                self.difficulty,
            );
            let at = self.p.pos;
            b.activate_at(at);
            self.hunters.push(b);
        }
    }

    pub fn save_checkpoint(&self) {
        crate::app::checkpoint::save(&crate::app::checkpoint::Checkpoint {
            run_seed: self.run_seed,
            depth: self.inf.as_ref().map(|i| i.level).unwrap_or(self.depth),
            score: self.score,
            coins: self.coins,
            difficulty: self.difficulty,
            infinite: self.inf.is_some(),
        });
    }

    pub fn chunks_visited(&self) -> usize {
        self.inf
            .as_ref()
            .map(|i| i.chunks_visited.len())
            .unwrap_or(0)
    }

    pub(super) fn floor_pickups(&self) -> Vec<FloorPickup> {
        use crate::game::Item;
        use crate::world::facility::Pickup as K;
        self.fac
            .pickups
            .iter()
            .filter_map(|(c, k)| {
                let (item, n) = match k {
                    K::Taser => (Item::Taser, 1),
                    K::Crowbar => (Item::Crowbar, CROWBAR_DURABILITY),
                    K::Axe => (Item::Axe, AXE_DURABILITY),
                    K::Battery => (Item::Battery, 1),
                    K::Stim => (Item::Stim, 1),
                    _ => return None,
                };
                Some(FloorPickup {
                    pos: c.center(),
                    item,
                    n,
                    taken: false,
                })
            })
            .collect()
    }
}
