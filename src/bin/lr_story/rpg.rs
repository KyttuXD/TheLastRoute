//! The RPG rules: the party (hp and stats; no xp, no levels), the real-time (action) battle and a tiny deterministic random number
//! generator. No drawing, no files and no clock here: the caller passes the time step, the player's position and a
//! tile-blocking test. `play.rs` owns the `Game` and drives this; `main.rs`, `hud.rs` and `battle_ui.rs` only read it.
//! Everything is testable without a window.
//!
//! Positions are (x, z) in metres. A yaw uses the body's convention: the facing is `(-sin yaw, -cos yaw)`.
//! Every `t` in [`EnemyState`] and [`Telegraph`] is seconds elapsed in that state.

use crate::body::{RADIUS, TILE_M};
use crate::model::{AttackKind, Battle, EnemyAttack, Stats};

/// Seconds the enemy stands still after it appears.
pub const INTRO_SECS: f32 = 1.2;
/// How long a floating number lives (seconds).
pub const HIT_SECS: f32 = 1.1;
/// The enemy's body radius against walls (metres).
pub const ENEMY_RADIUS: f32 = 0.4;
/// The most the enemy closes in on the player (centre to centre). The real stop distance is [`stop_dist`], which is
/// smaller when the enemy only has short-range attacks.
pub const STOP_DIST: f32 = 0.9;
/// Seconds between tile-path recomputations while the enemy is chasing around a wall.
pub const REPATH_SECS: f32 = 0.3;
/// Seconds the enemy may have no tile path to the player before the no-path rule fires: it is teleported to the
/// nearest spot that fits, next to the player (see `ActionBattle::chase`). The fight never soft-locks.
pub const NO_PATH_SECS: f32 = 4.0;
/// A shot starts this far in front of the enemy (metres).
pub const SHOT_START: f32 = 0.6;
/// A charge hurts the player inside this distance (centre to centre), and its lane is drawn this wide (half width).
pub const CHARGE_HIT: f32 = ENEMY_RADIUS + RADIUS + 0.1;
/// Most tiles the path finder visits, so a huge map cannot stall a frame.
pub const PATH_MAX_TILES: usize = 6000;
/// The player's melee reach (centre to centre, metres) and the width of the arc in front of the body (degrees).
pub const PLAYER_REACH: f32 = 2.0;
pub const PLAYER_ARC_DEG: f32 = 100.0;
/// Seconds between player swings.
pub const SWING_SECS: f32 = 0.45;
/// Seconds the player cannot be hurt after a hit.
pub const IFRAME_SECS: f32 = 0.6;
/// Seconds a normal enemy is staggered by a hit.
pub const STAGGER_SECS: f32 = 0.3;
/// Share of damage a held block removes, and the share of walking speed a block leaves.
pub const BLOCK_CUT: f32 = 0.6;
pub const BLOCK_SLOW: f32 = 0.5;
/// An enemy melee swing: the reach when the attack names no range, and the arc in front of the enemy (degrees).
pub const MELEE_REACH: f32 = 1.6;
pub const ENEMY_ARC_DEG: f32 = 120.0;
/// How long the strike lasts for a non-charge attack, and the pause after any attack (seconds).
pub const STRIKE_SECS: f32 = 0.12;
pub const RECOVER_SECS: f32 = 0.4;
/// The flash after a hit on the enemy (seconds).
pub const FLASH_SECS: f32 = 0.15;
/// A damage item reaches this far (metres).
pub const ITEM_RANGE: f32 = 6.0;
/// Seconds the VICTORY / DEFEATED banner shows before the battle closes.
pub const END_SECS: f32 = 2.5;

/// A small deterministic generator (splitmix64), so a story's battles play out the same way every run and tests are stable.
#[derive(Clone, Debug, PartialEq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// A seed from text (FNV-1a), for seeding from a story id.
    pub fn seed_from(text: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 32) as u32
    }

    /// A number in 0..n (0 when n is 0).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }

    /// The damage roll: 0.8 to 1.2 (plus or minus 20 percent).
    pub fn roll(&mut self) -> f32 {
        0.8 + 0.4 * (self.next_u32() as f32 / u32::MAX as f32)
    }
}

/// The player's progress: hit points plus the stats the damage formula reads. There is no experience and no level.
#[derive(Clone, Debug, PartialEq)]
pub struct Party {
    pub hp: i32,
    pub max_hp: i32,
    pub stats: Stats,
}

impl Party {
    /// A party at full health with the given stats.
    pub fn new(stats: Stats) -> Party {
        Party { hp: stats.max_hp.max(1), max_hp: stats.max_hp.max(1), stats }
    }

    /// A party rebuilt from a save: `base` are the story's stats and the saved max hp wins over the base.
    #[cfg(test)]
    pub fn restored(base: Stats, hp: i32, max_hp: i32) -> Party {
        let max_hp = max_hp.max(1);
        let mut stats = base;
        stats.max_hp = max_hp;
        Party { hp: hp.clamp(0, max_hp), max_hp, stats }
    }

    /// Heals up to max hp. Returns how much was actually restored.
    pub fn heal(&mut self, amount: i32) -> i32 {
        let gain = amount.max(0).min(self.max_hp - self.hp).max(0);
        self.hp += gain;
        gain
    }
}

/// The damage a player swing deals: `max(1, attack * 2 - defense)` times the roll (0.8 to 1.2), at least 1.
pub fn player_damage(attack: i32, enemy_defense: i32, roll: f32) -> i32 {
    let base = (attack * 2 - enemy_defense).max(1) as f32;
    ((base * roll).round() as i32).max(1)
}

/// The damage an enemy attack deals to the player: `max(1, damage - defense / 2)`; a held block takes 60 percent off
/// (still at least 1).
pub fn enemy_damage(raw: i32, player_defense: i32, blocking: bool) -> i32 {
    let dmg = (raw - player_defense / 2).max(1);
    if blocking {
        (((dmg as f32) * (1.0 - BLOCK_CUT)).round() as i32).max(1)
    } else {
        dmg
    }
}

/// What the enemy is doing. `t` is seconds elapsed in the state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EnemyState {
    /// Standing still just after appearing.
    Intro { t: f32 },
    /// Walking toward the player, picking an attack when one is in range.
    Chase,
    /// Telegraphing attack `attack` (an index into the battle's attacks). `aim` is the Slam's target spot, or the
    /// direction (unit) the other kinds point along; Melee and Shot keep turning toward the player until the end.
    Windup { attack: usize, t: f32, aim: (f32, f32) },
    /// The hit has landed (or, for a Charge, the dash is under way).
    Strike { attack: usize, t: f32 },
    /// A short pause after an attack.
    Recover { t: f32 },
    /// Knocked back a moment by a player hit (not a boss).
    Stagger { t: f32 },
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Won,
    Lost,
}

/// A shot in flight.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub pos: (f32, f32),
    pub vel: (f32, f32),
    pub radius: f32,
    pub damage: i32,
    /// Seconds left before it fades.
    pub life: f32,
}

/// What the floor shows before a hit: a circle for Slam, a cone for Melee, a lane for Charge and Shot. `t` is seconds
/// into the windup and `total` the windup length, so `t / total` is how full it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Telegraph {
    pub centre: (f32, f32),
    /// Slam: the circle's radius. Melee: the reach. Charge and Shot: the lane's half width.
    pub radius: f32,
    pub t: f32,
    pub total: f32,
    pub kind: AttackKind,
    /// Unit direction of the cone or lane (zero for a Slam).
    pub dir: (f32, f32),
    /// Lane length in metres from `centre` (Melee: the reach, Slam: 0). A Shot's lane is its whole flight.
    pub length: f32,
    /// Melee: the full width of the cone in degrees, the very arc the hit test uses ([`ENEMY_ARC_DEG`]); 0 for the rest.
    pub arc_deg: f32,
}

/// A floating number or word over the enemy or the player, at a spot on the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub text: String,
    pub pos: (f32, f32),
    pub age: f32,
    pub on_enemy: bool,
    pub heal: bool,
}

/// One fight, in real time. `Game` owns it while a battle is on; it is None otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionBattle {
    /// The battle's id (the key in the pack's battles).
    pub battle: String,
    pub enemy_pos: (f32, f32),
    pub enemy_yaw: f32,
    pub enemy_hp: i32,
    /// Eases down to `enemy_hp`; the boss bar's pale recent-damage segment.
    pub trail_hp: f32,
    /// Phase marks passed so far (0 at the start).
    pub phase: usize,
    pub state: EnemyState,
    /// Seconds left before each attack (same order as the battle's attacks) can be picked again.
    pub cooldowns: Vec<f32>,
    pub projectiles: Vec<Projectile>,
    pub telegraphs: Vec<Telegraph>,
    pub hits: Vec<Hit>,
    /// Seconds of white flash left on the enemy.
    pub flash: f32,
    /// Camera shake strength, fading to 0.
    pub shake: f32,
    /// Seconds before the player can swing again.
    pub player_swing: f32,
    /// Seconds the player cannot be hurt.
    pub player_iframes: f32,
    /// The block button is held.
    pub blocking: bool,
    pub outcome: Option<Outcome>,
    /// The outcome's rewards have been given (set by `Game`).
    pub settled: bool,
    /// Seconds since the outcome (the banner clock).
    pub end_t: f32,
    /// A dash has already hit the player.
    strike_hit: bool,
    /// The tile path the chase follows when a wall is in the way (next tile first), and the seconds until it is redone.
    path: Vec<(i32, i32)>,
    repath: f32,
    /// Seconds the chase has had no path to the player.
    no_path: f32,
    /// The last path search found a way (true at the start).
    path_ok: bool,
}

/// What `tick` needs from the world: where the player is, who takes the damage, the random numbers and whether a map
/// tile (tx, ty) stops the enemy.
pub struct Ctx<'a> {
    pub player: (f32, f32),
    pub party: &'a mut Party,
    pub rng: &'a mut Rng,
    pub blocks: &'a dyn Fn(i32, i32) -> bool,
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The unit vector from a to b ((0, 0) when they coincide).
fn dir_to(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let d = dist(a, b);
    if d < 1e-4 {
        (0.0, 0.0)
    } else {
        ((b.0 - a.0) / d, (b.1 - a.1) / d)
    }
}

/// The yaw that faces along (dx, dz), in the body's convention.
pub fn yaw_of(dx: f32, dz: f32) -> f32 {
    (-dx).atan2(-dz)
}

/// The facing of a yaw as a unit (x, z).
pub fn facing(yaw: f32) -> (f32, f32) {
    (-yaw.sin(), -yaw.cos())
}

/// Whether a circle of the enemy's size fits at `p`.
fn fits(p: (f32, f32), blocks: &dyn Fn(i32, i32) -> bool) -> bool {
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let tx = ((p.0 + sx * ENEMY_RADIUS) / TILE_M).floor() as i32;
        let tz = ((p.1 + sz * ENEMY_RADIUS) / TILE_M).floor() as i32;
        if blocks(tx, tz) {
            return false;
        }
    }
    true
}

/// Moves `p` by `d`, sliding along walls (each axis on its own). Returns the new spot and whether it moved at all.
fn slide(p: (f32, f32), d: (f32, f32), blocks: &dyn Fn(i32, i32) -> bool) -> ((f32, f32), bool) {
    let mut q = p;
    if fits((q.0 + d.0, q.1), blocks) {
        q.0 += d.0;
    }
    if fits((q.0, q.1 + d.1), blocks) {
        q.1 += d.1;
    }
    (q, q != p)
}

fn tile_of(p: (f32, f32)) -> (i32, i32) {
    ((p.0 / TILE_M).floor() as i32, (p.1 / TILE_M).floor() as i32)
}

fn tile_centre(t: (i32, i32)) -> (f32, f32) {
    ((t.0 as f32 + 0.5) * TILE_M, (t.1 as f32 + 0.5) * TILE_M)
}

/// Whether no blocked tile lies on the straight line from `a` to `b` (sampled every quarter metre, ends included).
/// Melee uses it so nothing is hit through a wall.
pub fn line_clear(a: (f32, f32), b: (f32, f32), blocks: &dyn Fn(i32, i32) -> bool) -> bool {
    let d = dist(a, b);
    let steps = (d / 0.25).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let k = i as f32 / steps as f32;
        let t = tile_of((a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k));
        if blocks(t.0, t.1) {
            return false;
        }
    }
    true
}

/// A shortest tile path (4-neighbour BFS over tiles `blocks` calls open) from tile `start` to tile `goal`. The result
/// lists the tiles to walk through, next one first, ending on `goal`; it is empty when they are the same tile and None
/// when the goal cannot be reached (or the search hits [`PATH_MAX_TILES`]). The goal tile itself is allowed even if
/// `blocks` says so, since the player may stand on a tile the enemy could not enter.
pub fn find_path(start: (i32, i32), goal: (i32, i32), blocks: &dyn Fn(i32, i32) -> bool) -> Option<Vec<(i32, i32)>> {
    if start == goal {
        return Some(Vec::new());
    }
    let mut from: std::collections::HashMap<(i32, i32), (i32, i32)> = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    from.insert(start, start);
    queue.push_back(start);
    while let Some(cur) = queue.pop_front() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cur.0 + dx, cur.1 + dy);
            if from.contains_key(&next) || (next != goal && blocks(next.0, next.1)) {
                continue;
            }
            from.insert(next, cur);
            if next == goal {
                let mut path = vec![goal];
                let mut at = goal;
                while from[&at] != start {
                    at = from[&at];
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            if from.len() > PATH_MAX_TILES {
                return None;
            }
            queue.push_back(next);
        }
    }
    None
}

/// The nearest point to `p` where the enemy fits (all four corners on open tiles): `p` itself when it fits, else the
/// closest fitting point found by searching outward in rings. Falls back to `p` when nothing within 30 m fits.
pub fn fit_spawn(p: (f32, f32), blocks: &dyn Fn(i32, i32) -> bool) -> (f32, f32) {
    if fits(p, blocks) {
        return p;
    }
    let mut r = 0.25f32;
    while r <= 30.0 {
        let n = ((r * 8.0).ceil() as usize).max(8);
        let mut best: Option<((f32, f32), f32)> = None;
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let q = (p.0 + a.cos() * r, p.1 + a.sin() * r);
            if fits(q, blocks) {
                let d = dist(p, q);
                if best.map_or(true, |(_, bd)| d < bd) {
                    best = Some((q, d));
                }
            }
        }
        if let Some((q, _)) = best {
            return q;
        }
        r += 0.25;
    }
    p
}

/// Where the no-path rule puts the enemy: 2 m from the player in the first of eight directions where it fits and sees
/// the player, else the nearest fitting point to the player.
pub fn rescue_spot(player: (f32, f32), blocks: &dyn Fn(i32, i32) -> bool) -> (f32, f32) {
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let q = (player.0 + a.cos() * 2.0, player.1 + a.sin() * 2.0);
        if fits(q, blocks) && line_clear(q, player, blocks) {
            return q;
        }
    }
    fit_spawn(player, blocks)
}

/// How close the enemy walks to the player: [`STOP_DIST`], or 80 percent of the shortest attack range when that is
/// smaller, so an attack with a short range is still reachable and used.
pub fn stop_dist(def: &Battle) -> f32 {
    let shortest = def.attacks.iter().map(|a| a.range).filter(|r| *r > 0.0).fold(f32::INFINITY, f32::min);
    STOP_DIST.min(shortest * 0.8)
}

/// The reach of an enemy melee swing, centre to centre: the attack's range (or [`MELEE_REACH`]) plus the player's body
/// radius. The hit test and the drawn cone both use this.
pub fn melee_reach(a: &EnemyAttack) -> f32 {
    (if a.range > 0.0 { a.range } else { MELEE_REACH }) + RADIUS
}

/// A shot's real hit half width (its radius plus the player's body radius) and the length of its whole flight from the
/// enemy's centre. The Shot lane is drawn with these.
pub fn shot_hit_width(a: &EnemyAttack) -> f32 {
    a.radius.max(0.1) + RADIUS
}

pub fn shot_flight_len(a: &EnemyAttack) -> f32 {
    SHOT_START + a.range.max(8.0) * 1.5
}

/// The one phase number people see: 1 at the start, 2 after the first HP mark, and so on. The "Phase N" word over the
/// enemy and the boss bar both show this.
pub fn phase_number(phase: usize) -> usize {
    phase + 1
}

/// The phase a battle is in when the enemy has `hp` left: how many of its HP percent marks are passed.
pub fn phase_for(def: &Battle, hp: i32) -> usize {
    let max = def.max_hp.max(1) as i64;
    def.phases.iter().filter(|&&m| (hp.max(0) as i64) * 100 <= (m as i64) * max).count()
}

/// Picks one of `ok` (indices into `attacks`) by weight. A zero weight is never picked unless every weight is zero (then
/// the first is). None when `ok` is empty.
pub fn choose_attack(attacks: &[crate::model::EnemyAttack], ok: &[usize], rng: &mut Rng) -> Option<usize> {
    if ok.is_empty() {
        return None;
    }
    let total: u32 = ok.iter().map(|&i| attacks[i].weight).sum();
    if total == 0 {
        return ok.first().copied();
    }
    let mut pick = rng.below(total);
    for &i in ok {
        let w = attacks[i].weight;
        if pick < w {
            return Some(i);
        }
        pick -= w;
    }
    ok.last().copied()
}

impl ActionBattle {
    /// A fight that starts with the enemy at `pos`, full of health and in its intro.
    pub fn new(def: &Battle, pos: (f32, f32)) -> ActionBattle {
        let hp = def.max_hp.max(1);
        ActionBattle {
            battle: def.id.clone(),
            enemy_pos: pos,
            enemy_yaw: 0.0,
            enemy_hp: hp,
            trail_hp: hp as f32,
            phase: 0,
            state: EnemyState::Intro { t: 0.0 },
            cooldowns: vec![0.0; def.attacks.len()],
            projectiles: Vec::new(),
            telegraphs: Vec::new(),
            hits: Vec::new(),
            flash: 0.0,
            shake: 0.0,
            player_swing: 0.0,
            player_iframes: 0.0,
            blocking: false,
            outcome: None,
            settled: false,
            end_t: 0.0,
            strike_hit: false,
            path: Vec::new(),
            repath: 0.0,
            no_path: 0.0,
            path_ok: true,
        }
    }

    /// The enemy's share of its max hp left, 0 to 1.
    #[cfg(test)]
    pub fn enemy_fraction(&self, def: &Battle) -> f32 {
        (self.enemy_hp as f32 / def.max_hp.max(1) as f32).clamp(0.0, 1.0)
    }

    /// Walking speed now: the battle's move speed, +20 percent per phase mark passed.
    pub fn move_speed(&self, def: &Battle) -> f32 {
        def.move_speed * (1.0 + 0.2 * self.phase as f32)
    }

    /// Cooldown multiplier now: -15 percent per phase mark passed.
    pub fn cooldown_scale(&self) -> f32 {
        0.85f32.powi(self.phase as i32)
    }

    fn add_hit(&mut self, text: String, pos: (f32, f32), on_enemy: bool, heal: bool) {
        self.hits.push(Hit { text, pos, age: 0.0, on_enemy, heal });
    }

    /// A floating "+N" over the player (the hp itself was already restored by the caller).
    pub fn note_player_heal(&mut self, at: (f32, f32), gained: i32) {
        self.add_hit(format!("+{gained}"), at, false, true);
    }

    fn face(&mut self, target: (f32, f32)) {
        let d = dir_to(self.enemy_pos, target);
        if d != (0.0, 0.0) {
            self.enemy_yaw = yaw_of(d.0, d.1);
        }
    }

    /// Deals `dmg` to the enemy. Returns true when that defeated it (the outcome becomes Won). Crossing an HP mark raises
    /// the phase with a "Phase N" word and a bigger shake.
    pub fn hurt_enemy(&mut self, def: &Battle, dmg: i32) -> bool {
        if self.outcome.is_some() || self.state == EnemyState::Dead {
            return false;
        }
        let dmg = dmg.max(1);
        self.enemy_hp = (self.enemy_hp - dmg).max(0);
        self.add_hit(format!("{dmg}"), self.enemy_pos, true, false);
        self.flash = FLASH_SECS;
        self.shake = self.shake.max(3.0);
        if self.enemy_hp == 0 {
            self.state = EnemyState::Dead;
            self.outcome = Some(Outcome::Won);
            self.projectiles.clear();
            self.telegraphs.clear();
            return true;
        }
        let now = phase_for(def, self.enemy_hp);
        if now > self.phase {
            self.phase = now;
            self.add_hit(format!("Phase {}", phase_number(now)), self.enemy_pos, true, false);
            self.shake = self.shake.max(8.0);
        }
        false
    }

    /// Deals an enemy attack's raw `dmg` to the player at `at`. Nothing happens while the player is invulnerable or the
    /// fight is over. Returns the damage taken. Hp 0 makes the outcome Lost.
    pub fn hurt_player(&mut self, party: &mut Party, raw: i32, at: (f32, f32)) -> i32 {
        if self.outcome.is_some() || self.player_iframes > 0.0 {
            return 0;
        }
        let dmg = enemy_damage(raw, party.stats.defense, self.blocking);
        party.hp = (party.hp - dmg).max(0);
        self.player_iframes = IFRAME_SECS;
        self.add_hit(format!("{dmg}"), at, false, false);
        self.shake = self.shake.max(if self.blocking { 2.0 } else { 5.0 });
        if party.hp == 0 {
            self.outcome = Some(Outcome::Lost);
            self.projectiles.clear();
            self.telegraphs.clear();
        }
        dmg
    }

    /// The player swings from `player` facing `yaw`. Returns true when the swing hit. A swing starts its cooldown whether
    /// or not it hits; one inside the cooldown does nothing. A hit staggers a non-boss for 0.3 s. A wall (a tile
    /// `blocks` stops) on the line between the two stops the swing.
    pub fn player_attack(
        &mut self,
        def: &Battle,
        player: (f32, f32),
        yaw: f32,
        party: &Party,
        rng: &mut Rng,
        blocks: &dyn Fn(i32, i32) -> bool,
    ) -> bool {
        if self.outcome.is_some() || self.player_swing > 0.0 {
            return false;
        }
        self.player_swing = SWING_SECS;
        let d = dist(player, self.enemy_pos);
        if d > PLAYER_REACH || !line_clear(player, self.enemy_pos, blocks) {
            return false;
        }
        if d > 0.05 {
            let to = dir_to(player, self.enemy_pos);
            let f = facing(yaw);
            if f.0 * to.0 + f.1 * to.1 < (PLAYER_ARC_DEG / 2.0).to_radians().cos() {
                return false;
            }
        }
        let dmg = player_damage(party.stats.attack, def.defense, rng.roll());
        self.hurt_enemy(def, dmg);
        let interruptible = matches!(self.state, EnemyState::Chase | EnemyState::Windup { .. } | EnemyState::Recover { .. });
        if !def.boss && interruptible && self.outcome.is_none() {
            self.telegraphs.clear();
            self.state = EnemyState::Stagger { t: 0.0 };
        }
        true
    }

    /// Advances the fight by `dt` seconds (capped at a quarter second so a slow frame cannot skip a state).
    pub fn tick(&mut self, def: &Battle, dt: f32, cx: &mut Ctx) {
        let dt = dt.clamp(0.0, 0.25);
        self.tick_visuals(dt);
        if self.outcome.is_some() {
            return;
        }
        for c in &mut self.cooldowns {
            *c = (*c - dt).max(0.0);
        }
        self.tick_projectiles(dt, cx);
        if self.outcome.is_some() {
            return;
        }
        match self.state {
            EnemyState::Intro { t } => {
                self.face(cx.player);
                let t = t + dt;
                self.state = if t >= INTRO_SECS { EnemyState::Chase } else { EnemyState::Intro { t } };
            }
            EnemyState::Chase => self.chase(def, dt, cx),
            EnemyState::Windup { attack, t, aim } => self.windup(def, attack, t + dt, aim, dt, cx),
            EnemyState::Strike { attack, t } => self.strike(def, attack, t + dt, dt, cx),
            EnemyState::Recover { t } => {
                let t = t + dt;
                self.state = if t >= RECOVER_SECS { EnemyState::Chase } else { EnemyState::Recover { t } };
            }
            EnemyState::Stagger { t } => {
                let t = t + dt;
                self.state = if t >= STAGGER_SECS { EnemyState::Chase } else { EnemyState::Stagger { t } };
            }
            EnemyState::Dead => {}
        }
    }

    fn tick_visuals(&mut self, dt: f32) {
        for h in &mut self.hits {
            h.age += dt;
        }
        self.hits.retain(|h| h.age < HIT_SECS);
        self.flash = (self.flash - dt).max(0.0);
        self.shake = (self.shake - dt * 24.0).max(0.0);
        self.player_swing = (self.player_swing - dt).max(0.0);
        self.player_iframes = (self.player_iframes - dt).max(0.0);
        // The trail eases down toward the real hp (never up).
        let target = self.enemy_hp as f32;
        if self.trail_hp > target {
            self.trail_hp -= (self.trail_hp - target) * (dt * 3.0).min(1.0) + dt * 2.0;
            if self.trail_hp < target + 0.05 {
                self.trail_hp = target;
            }
        } else {
            self.trail_hp = target;
        }
    }

    fn tick_projectiles(&mut self, dt: f32, cx: &mut Ctx) {
        let mut shots = std::mem::take(&mut self.projectiles);
        shots.retain_mut(|p| {
            p.pos.0 += p.vel.0 * dt;
            p.pos.1 += p.vel.1 * dt;
            p.life -= dt;
            if p.life <= 0.0 {
                return false;
            }
            let tile = ((p.pos.0 / TILE_M).floor() as i32, (p.pos.1 / TILE_M).floor() as i32);
            if (cx.blocks)(tile.0, tile.1) {
                return false;
            }
            if dist(p.pos, cx.player) <= p.radius + RADIUS {
                // A shot that touches the player is spent, hurt or not: during the iframes it does nothing, and it
                // can never hit a second time.
                self.hurt_player(cx.party, p.damage, cx.player);
                return false;
            }
            true
        });
        // A shot added by a hit above cannot exist; just put the survivors back.
        if self.outcome.is_none() {
            self.projectiles = shots;
        }
    }

    fn chase(&mut self, def: &Battle, dt: f32, cx: &mut Ctx) {
        self.face(cx.player);
        let d = dist(self.enemy_pos, cx.player);
        let sight = line_clear(self.enemy_pos, cx.player, cx.blocks);
        // Which attacks could start now? A melee swing needs a clear line (it cannot hit through a wall).
        let ok: Vec<usize> = def
            .attacks
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.min_phase <= self.phase
                    && self.cooldowns.get(*i).copied().unwrap_or(0.0) <= 0.0
                    && d <= a.range
                    && (sight || !matches!(a.kind, AttackKind::Melee))
            })
            .map(|(i, _)| i)
            .collect();
        if let Some(i) = choose_attack(&def.attacks, &ok, cx.rng) {
            self.begin_windup(def, i, cx.player);
            return;
        }
        let speed = self.move_speed(def);
        let stop = stop_dist(def);
        if d <= stop || speed <= 0.0 {
            self.path.clear();
            self.no_path = 0.0;
            return;
        }
        if sight {
            // Nothing in the way: straight at the player.
            self.path.clear();
            self.no_path = 0.0;
            self.repath = 0.0;
            self.path_ok = true;
            let step = (speed * dt).min(d - stop);
            let dir = dir_to(self.enemy_pos, cx.player);
            let (p, _) = slide(self.enemy_pos, (dir.0 * step, dir.1 * step), cx.blocks);
            self.enemy_pos = p;
            return;
        }
        // A wall is between them: follow a tile path, redone a few times a second.
        self.repath -= dt;
        if self.repath <= 0.0 {
            self.repath = REPATH_SECS;
            match find_path(tile_of(self.enemy_pos), tile_of(cx.player), cx.blocks) {
                Some(p) => {
                    self.path = p;
                    self.path_ok = true;
                    self.no_path = 0.0;
                }
                None => {
                    self.path.clear();
                    self.path_ok = false;
                }
            }
        }
        if !self.path_ok {
            // The no-path rule: after NO_PATH_SECS without a path the enemy is moved to the nearest spot that fits
            // beside the player and can see them, so the fight goes on and never soft-locks.
            self.no_path += dt;
            if self.no_path >= NO_PATH_SECS {
                self.enemy_pos = rescue_spot(cx.player, cx.blocks);
                self.path.clear();
                self.no_path = 0.0;
                self.repath = 0.0;
                self.path_ok = true;
            }
            return;
        }
        while let Some(&next) = self.path.first() {
            if dist(self.enemy_pos, tile_centre(next)) < 0.2 {
                self.path.remove(0);
            } else {
                break;
            }
        }
        let target = self.path.first().map_or(cx.player, |&t| tile_centre(t));
        let dir = dir_to(self.enemy_pos, target);
        let step = speed * dt;
        let (p, _) = slide(self.enemy_pos, (dir.0 * step, dir.1 * step), cx.blocks);
        self.enemy_pos = p;
    }

    fn begin_windup(&mut self, def: &Battle, i: usize, player: (f32, f32)) {
        let a = &def.attacks[i];
        self.face(player);
        let dir = dir_to(self.enemy_pos, player);
        let dir = if dir == (0.0, 0.0) { facing(self.enemy_yaw) } else { dir };
        let total = a.windup.max(0.05);
        let tele = match a.kind {
            AttackKind::Melee => {
                // The same reach and arc the hit test uses.
                let reach = melee_reach(a);
                Telegraph {
                    centre: self.enemy_pos,
                    radius: reach,
                    t: 0.0,
                    total,
                    kind: a.kind.clone(),
                    dir,
                    length: reach,
                    arc_deg: ENEMY_ARC_DEG,
                }
            }
            AttackKind::Charge => Telegraph {
                centre: self.enemy_pos,
                radius: CHARGE_HIT,
                t: 0.0,
                total,
                kind: a.kind.clone(),
                dir,
                length: a.range.max(2.0),
                arc_deg: 0.0,
            },
            // The lane is as wide as the shot's real hit width and as long as its whole flight.
            AttackKind::Shot => Telegraph {
                centre: self.enemy_pos,
                radius: shot_hit_width(a),
                t: 0.0,
                total,
                kind: a.kind.clone(),
                dir,
                length: shot_flight_len(a),
                arc_deg: 0.0,
            },
            AttackKind::Slam => Telegraph {
                centre: player,
                radius: a.radius.max(0.5),
                t: 0.0,
                total,
                kind: a.kind.clone(),
                dir: (0.0, 0.0),
                length: 0.0,
                arc_deg: 0.0,
            },
        };
        let aim = if matches!(a.kind, AttackKind::Slam) { player } else { dir };
        self.telegraphs = vec![tele];
        self.state = EnemyState::Windup { attack: i, t: 0.0, aim };
    }

    fn windup(&mut self, def: &Battle, i: usize, t: f32, mut aim: (f32, f32), _dt: f32, cx: &mut Ctx) {
        let Some(a) = def.attacks.get(i) else {
            self.telegraphs.clear();
            self.state = EnemyState::Chase;
            return;
        };
        // Melee and Shot keep turning toward the player; Charge and Slam are locked in.
        if matches!(a.kind, AttackKind::Melee | AttackKind::Shot) {
            self.face(cx.player);
            let d = dir_to(self.enemy_pos, cx.player);
            if d != (0.0, 0.0) {
                aim = d;
            }
            for tele in &mut self.telegraphs {
                tele.dir = aim;
            }
        }
        let total = a.windup.max(0.05);
        for tele in &mut self.telegraphs {
            tele.t = t.min(total);
        }
        if t < total {
            self.state = EnemyState::Windup { attack: i, t, aim };
            return;
        }
        self.telegraphs.clear();
        let scale = self.cooldown_scale();
        if let Some(c) = self.cooldowns.get_mut(i) {
            *c = a.cooldown * scale;
        }
        self.strike_hit = false;
        match a.kind {
            AttackKind::Melee => {
                let reach = melee_reach(a);
                let f = facing(self.enemy_yaw);
                let to = dir_to(self.enemy_pos, cx.player);
                let d = dist(self.enemy_pos, cx.player);
                let in_arc = d < 0.05 || f.0 * to.0 + f.1 * to.1 >= (ENEMY_ARC_DEG / 2.0).to_radians().cos();
                if d <= reach && in_arc && line_clear(self.enemy_pos, cx.player, cx.blocks) {
                    self.hurt_player(cx.party, a.damage, cx.player);
                }
            }
            AttackKind::Slam => {
                if dist(aim, cx.player) <= a.radius.max(0.5) {
                    self.hurt_player(cx.party, a.damage, cx.player);
                }
                self.shake = self.shake.max(4.0);
            }
            AttackKind::Shot => {
                let dir = dir_to(self.enemy_pos, cx.player);
                let dir = if dir == (0.0, 0.0) { facing(self.enemy_yaw) } else { dir };
                let speed = if a.speed > 0.0 { a.speed } else { 8.0 };
                self.projectiles.push(Projectile {
                    pos: (self.enemy_pos.0 + dir.0 * SHOT_START, self.enemy_pos.1 + dir.1 * SHOT_START),
                    vel: (dir.0 * speed, dir.1 * speed),
                    radius: a.radius.max(0.1),
                    damage: a.damage,
                    life: (shot_flight_len(a) - SHOT_START) / speed,
                });
            }
            AttackKind::Charge => {
                self.enemy_yaw = yaw_of(aim.0, aim.1);
            }
        }
        if self.outcome.is_some() {
            return;
        }
        self.state = EnemyState::Strike { attack: i, t: 0.0 };
    }

    fn strike(&mut self, def: &Battle, i: usize, t: f32, dt: f32, cx: &mut Ctx) {
        let Some(a) = def.attacks.get(i) else {
            self.state = EnemyState::Chase;
            return;
        };
        if !matches!(a.kind, AttackKind::Charge) {
            self.state = if t >= STRIKE_SECS { EnemyState::Recover { t: 0.0 } } else { EnemyState::Strike { attack: i, t } };
            return;
        }
        // The dash: straight along the locked lane at the attack's speed until the lane ends or a wall stops it.
        let speed = if a.speed > 0.0 { a.speed } else { 12.0 };
        let dir = facing(self.enemy_yaw);
        let (p, moved) = slide(self.enemy_pos, (dir.0 * speed * dt, dir.1 * speed * dt), cx.blocks);
        self.enemy_pos = p;
        if !self.strike_hit && dist(self.enemy_pos, cx.player) <= CHARGE_HIT {
            self.strike_hit = true;
            self.hurt_player(cx.party, a.damage, cx.player);
            if self.outcome.is_some() {
                return;
            }
        }
        let lane = a.range.max(2.0);
        self.state = if !moved || t * speed >= lane { EnemyState::Recover { t: 0.0 } } else { EnemyState::Strike { attack: i, t } };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EnemyAttack;

    fn atk(kind: AttackKind) -> EnemyAttack {
        EnemyAttack {
            name: "a".into(),
            kind,
            damage: 10,
            range: 3.0,
            windup: 0.5,
            cooldown: 2.0,
            weight: 1,
            min_phase: 0,
            radius: 1.5,
            speed: 10.0,
            text: String::new(),
        }
    }

    fn foe(attacks: Vec<EnemyAttack>) -> Battle {
        Battle {
            id: "foe".into(),
            name: "Foe".into(),
            boss: false,
            max_hp: 100,
            defense: 0,
            move_speed: 2.0,
            skin: None,
            spawn: String::new(),
            attacks,
            phases: Vec::new(),
            intro: String::new(),
            win: None,
            lose: None,
        }
    }

    fn open(_: i32, _: i32) -> bool {
        false
    }

    /// Ticks in small steps for `secs` seconds.
    fn run(b: &mut ActionBattle, def: &Battle, party: &mut Party, rng: &mut Rng, player: (f32, f32), secs: f32) {
        run_with(b, def, party, rng, player, secs, &open);
    }

    /// Like `run`, with a wall test.
    fn run_with(
        b: &mut ActionBattle,
        def: &Battle,
        party: &mut Party,
        rng: &mut Rng,
        player: (f32, f32),
        secs: f32,
        blocks: &dyn Fn(i32, i32) -> bool,
    ) {
        let steps = (secs / 0.05).round() as usize;
        for _ in 0..steps {
            let mut cx = Ctx { player, party, rng, blocks };
            b.tick(def, 0.05, &mut cx);
        }
    }

    fn fresh(def: &Battle, enemy: (f32, f32)) -> (ActionBattle, Party, Rng) {
        let mut b = ActionBattle::new(def, enemy);
        b.state = EnemyState::Chase;
        (b, Party::new(Stats::default()), Rng::new(1))
    }

    // Player at (10, 10) facing +x (yaw -pi/2).
    const P: (f32, f32) = (10.0, 10.0);
    const FACE_X: f32 = -std::f32::consts::FRAC_PI_2;

    #[test]
    fn damage_formulas_and_the_roll_stay_in_bounds() {
        for step in 0..=40 {
            let roll = 0.8 + 0.4 * step as f32 / 40.0;
            let d = player_damage(5, 2, roll);
            assert!((6..=10).contains(&d), "{d} at roll {roll}");
        }
        assert_eq!(player_damage(1, 500, 0.8), 1, "never below 1");
        assert_eq!(enemy_damage(10, 2, false), 9, "damage - defense / 2");
        assert_eq!(enemy_damage(1, 40, false), 1, "never below 1");
        assert_eq!(enemy_damage(20, 2, true), 8, "19 with 60 percent off is 7.6, rounded");
        assert_eq!(enemy_damage(1, 0, true), 1);
        let mut rng = Rng::new(7);
        for _ in 0..1000 {
            let r = rng.roll();
            assert!((0.8..=1.2).contains(&r));
        }
    }

    #[test]
    fn the_party_has_hp_and_stats_only_and_healing_is_capped() {
        let mut p = Party::new(Stats::default());
        assert_eq!((p.hp, p.max_hp, p.stats.attack, p.stats.defense), (30, 30, 5, 2));
        p.hp = 3;
        assert_eq!(p.heal(10), 10);
        assert_eq!(p.heal(1000), 17, "capped at max hp");
        assert_eq!(p.heal(-5), 0);
    }

    #[test]
    fn a_restored_party_matches_the_party_that_was_saved() {
        let mut p = Party::new(Stats::default());
        p.hp = 12;
        let r = Party::restored(Stats::default(), p.hp, p.max_hp);
        assert_eq!(r, p);
        assert_eq!(Party::restored(Stats::default(), 99, 20).hp, 20, "hp stays within the maximum");
    }

    /// A tall wall of open ground with a gap: tiles x = 7 for z tiles 0..8 are walls.
    fn wall_with_gap(tx: i32, tz: i32) -> bool {
        tx == 7 && (0..8).contains(&tz)
    }

    #[test]
    fn the_enemy_walks_a_tile_path_around_a_wall_to_the_player() {
        let def = foe(vec![]);
        // Enemy at tile (10, 5), player at tile (5, 5): the wall column at x = 7 (z tiles 0..8) is between them.
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 10.0));
        assert!(!line_clear((20.0, 10.0), P, &wall_with_gap));
        run_with(&mut b, &def, &mut party, &mut rng, P, 25.0, &wall_with_gap);
        assert!(dist(b.enemy_pos, P) < STOP_DIST + 0.3, "reached the player around the wall: {:?}", b.enemy_pos);
        assert!(b.enemy_pos.0 < 14.0, "now on the player's side");
    }

    #[test]
    fn find_path_goes_around_and_reports_none_when_sealed() {
        let p = find_path((10, 5), (5, 5), &wall_with_gap).expect("a way round");
        assert_eq!(p.last(), Some(&(5, 5)));
        assert!(p.iter().all(|t| !wall_with_gap(t.0, t.1)));
        assert!(p.iter().any(|t| t.1 >= 8), "goes through the gap");
        assert_eq!(find_path((3, 3), (3, 3), &open), Some(Vec::new()));
        // A sealed pocket around the goal.
        let sealed = |tx: i32, tz: i32| (tx - 5).abs().max((tz - 5).abs()) == 1;
        assert_eq!(find_path((10, 5), (5, 5), &sealed), None);
    }

    /// Enemy pathing uses the same `Kind::blocks` as the player: a shut door (its `open_if` false) is a wall, an open one
    /// (its `open_if` true) is floor.
    #[test]
    fn a_closed_door_blocks_the_chase_path_and_an_open_one_does_not() {
        use crate::model::{Cond, Kind};
        use std::collections::BTreeSet;
        let door = Kind::Door { open_if: Cond::Flag("open".into()), locked: String::new(), height: 2.0, color: [1, 2, 3] };
        // A corridor one tile wide along z = 5, with the door at x = 5; everything else is wall.
        let mut flags = BTreeSet::new();
        let path_with = |flags: &BTreeSet<String>| {
            let blocks = |tx: i32, tz: i32| {
                if tz != 5 || !(0..=10).contains(&tx) {
                    return true;
                }
                tx == 5 && door.blocks(flags, &[])
            };
            find_path((1, 5), (9, 5), &blocks)
        };
        assert_eq!(path_with(&flags), None, "a closed door seals the corridor");
        flags.insert("open".to_string());
        let p = path_with(&flags).expect("an open door is passable");
        assert!(p.contains(&(5, 5)) && p.last() == Some(&(9, 5)));
    }

    #[test]
    fn no_path_for_four_seconds_moves_the_enemy_next_to_the_player() {
        let def = foe(vec![]);
        // A wall column with no gap at all.
        let wall = |tx: i32, _tz: i32| tx == 7;
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 10.0));
        run_with(&mut b, &def, &mut party, &mut rng, P, 3.0, &wall);
        assert!(b.enemy_pos.0 >= 16.0, "still walled off after 3 s: {:?}", b.enemy_pos);
        run_with(&mut b, &def, &mut party, &mut rng, P, 1.5, &wall);
        assert!(dist(b.enemy_pos, P) < 2.5, "moved next to the player: {:?}", b.enemy_pos);
        assert!(fits(b.enemy_pos, &wall));
        assert!(line_clear(b.enemy_pos, P, &wall));
        assert!(b.outcome.is_none() && b.state == EnemyState::Chase);
    }

    #[test]
    fn a_spawn_inside_a_wall_is_moved_to_the_nearest_spot_that_fits() {
        let block = |tx: i32, tz: i32| (2..=4).contains(&tx) && (2..=4).contains(&tz); // x and z 4..10
        let inside = (7.0, 7.0);
        assert!(!fits(inside, &block));
        let at = fit_spawn(inside, &block);
        assert!(fits(at, &block), "{at:?}");
        assert!(dist(at, inside) <= 3.5 + 0.3, "nearest edge, {at:?}");
        assert_eq!(fit_spawn((20.0, 20.0), &block), (20.0, 20.0), "a spot that fits is kept");
    }

    #[test]
    fn an_attack_shorter_than_the_stop_distance_is_still_used() {
        let mut bite = atk(AttackKind::Melee);
        bite.range = 0.5;
        bite.windup = 0.2;
        bite.damage = 10;
        let def = foe(vec![bite]);
        assert!((stop_dist(&def) - 0.4).abs() < 1e-5, "80 percent of 0.5");
        assert_eq!(stop_dist(&foe(vec![])), STOP_DIST);
        assert_eq!(stop_dist(&foe(vec![atk(AttackKind::Melee)])), STOP_DIST, "3 m range: capped at 0.9");
        let (mut b, mut party, mut rng) = fresh(&def, (14.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 6.0);
        assert!(party.hp < 30, "the 0.5 m bite landed");
    }

    #[test]
    fn melee_cannot_hit_through_a_wall_either_way() {
        // Tile x = 5 (metres 10 to 12) is a wall between the player at 9.5 and the enemy at 12.5.
        let wall = |tx: i32, _tz: i32| tx == 5;
        let player = (9.5, 10.0);
        let mut m = atk(AttackKind::Melee);
        m.range = 4.0;
        m.windup = 0.2;
        let mut def = foe(vec![m]);
        def.move_speed = 0.0;
        // The enemy never starts a swing it cannot land.
        let (mut b, mut party, mut rng) = fresh(&def, (12.5, 10.0));
        run_with(&mut b, &def, &mut party, &mut rng, player, 1.0, &wall);
        assert_eq!(b.state, EnemyState::Chase);
        assert_eq!(party.hp, 30);
        // A swing already winding up is also stopped by the wall...
        b.state = EnemyState::Windup { attack: 0, t: 0.0, aim: (-1.0, 0.0) };
        run_with(&mut b, &def, &mut party, &mut rng, player, 0.5, &wall);
        assert_eq!(party.hp, 30, "no damage through the wall");
        // ...and lands once the wall is gone.
        let (mut b, mut party, mut rng) = fresh(&def, (12.5, 10.0));
        run(&mut b, &def, &mut party, &mut rng, player, 1.0);
        assert!(party.hp < 30);

        // The player's swing: same.
        let def = foe(vec![]);
        let (mut b, party, mut rng) = fresh(&def, (10.8, 10.0));
        // The enemy overlaps the wall tile (10 to 12 m), so the line from the player crosses it.
        let wall2 = |tx: i32, _tz: i32| tx == 5;
        assert!(!b.player_attack(&def, (9.5, 10.0), FACE_X, &party, &mut rng, &wall2), "wall between");
        assert_eq!(b.enemy_hp, 100);
        let (mut b, party, mut rng) = fresh(&def, (10.8, 10.0));
        assert!(b.player_attack(&def, (9.5, 10.0), FACE_X, &party, &mut rng, &open));
    }

    #[test]
    fn a_shot_is_spent_on_the_player_even_during_the_iframes_and_never_hits_twice() {
        let def = foe(vec![]);
        // During the iframes: removed, no damage.
        let (mut b, mut party, mut rng) = fresh(&def, (30.0, 30.0));
        b.player_iframes = 5.0;
        b.projectiles.push(Projectile { pos: P, vel: (0.0, 0.0), radius: 0.3, damage: 8, life: 5.0 });
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        assert!(b.projectiles.is_empty());
        assert_eq!(party.hp, 30);
        // Not invulnerable: one hit, then gone, and no second hit after the iframes end.
        let (mut b, mut party, mut rng) = fresh(&def, (30.0, 30.0));
        b.projectiles.push(Projectile { pos: P, vel: (0.0, 0.0), radius: 0.3, damage: 8, life: 5.0 });
        run(&mut b, &def, &mut party, &mut rng, P, 2.0);
        assert_eq!(party.hp, 30 - 7);
        assert!(b.projectiles.is_empty());
    }

    #[test]
    fn telegraphs_use_the_same_numbers_as_the_hits() {
        let mut m = atk(AttackKind::Melee);
        m.range = 1.6;
        let mut s = atk(AttackKind::Shot);
        s.range = 10.0;
        s.radius = 0.3;
        s.speed = 10.0;
        let def = foe(vec![m.clone(), s.clone()]);
        let (mut b, ..) = fresh(&def, (14.0, 10.0));
        b.begin_windup(&def, 0, P);
        let t = &b.telegraphs[0];
        assert_eq!((t.radius, t.length, t.arc_deg), (melee_reach(&m), melee_reach(&m), ENEMY_ARC_DEG));
        b.begin_windup(&def, 1, P);
        let t = &b.telegraphs[0];
        assert!((t.radius - (0.3 + RADIUS)).abs() < 1e-5, "the shot's real hit width");
        // Full flight: the start offset plus speed times life.
        let (mut fired, mut party, mut rng) = fresh(&def, (14.0, 10.0));
        fired.cooldowns[0] = 99.0;
        fired.begin_windup(&def, 1, P);
        run(&mut fired, &def, &mut party, &mut rng, (P.0, P.1 + 30.0), 0.55);
        let p = fired.projectiles.first().expect("fired");
        let flight = SHOT_START + p.vel.0.hypot(p.vel.1) * p.life;
        assert!((flight - t.length).abs() < 0.6, "lane {} vs flight {}", t.length, flight);
        assert_eq!(phase_number(0), 1);
        assert_eq!(phase_number(1), 2);
    }

    #[test]
    fn the_generator_is_deterministic_and_the_seed_depends_on_the_text() {
        let mut a = Rng::new(Rng::seed_from("lr_story"));
        let mut b = Rng::new(Rng::seed_from("lr_story"));
        assert_eq!((a.next_u32(), a.next_u32()), (b.next_u32(), b.next_u32()));
        assert_ne!(Rng::seed_from("a"), Rng::seed_from("b"));
        assert_eq!(a.below(0), 0);
    }

    #[test]
    fn attacks_are_chosen_by_weight() {
        let mut rng = Rng::new(1);
        let mut a = atk(AttackKind::Melee);
        a.weight = 0;
        let mut b = atk(AttackKind::Slam);
        b.weight = 5;
        let attacks = [a, b];
        assert!(choose_attack(&attacks, &[], &mut rng).is_none());
        for _ in 0..50 {
            assert_eq!(choose_attack(&attacks, &[0, 1], &mut rng), Some(1));
        }
        assert_eq!(choose_attack(&attacks, &[0], &mut rng), Some(0), "all zero: the first");
    }

    #[test]
    fn a_player_swing_hits_only_in_reach_and_inside_the_arc() {
        let def = foe(vec![]);
        let (mut b, party, mut rng) = fresh(&def, (11.5, 10.0));
        assert!(b.player_attack(&def, P, FACE_X, &party, &mut rng, &open), "1.5 m dead ahead");
        assert!(b.enemy_hp < 100 && b.enemy_hp >= 100 - 12);
        assert!(matches!(b.state, EnemyState::Stagger { .. }), "a non-boss is staggered");
        assert!(b.flash > 0.0 && b.hits.iter().any(|h| h.on_enemy));

        // Out of reach (2.5 m).
        let (mut b, party, mut rng) = fresh(&def, (12.5, 10.0));
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open));
        assert_eq!(b.enemy_hp, 100);
        // Behind the player.
        let (mut b, party, mut rng) = fresh(&def, (8.8, 10.0));
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open));
        // Just outside the 50 degree half-arc: 1.5 m off at 60 degrees.
        let off = (P.0 + 1.5 * 60f32.to_radians().cos(), P.1 + 1.5 * 60f32.to_radians().sin());
        let (mut b, party, mut rng) = fresh(&def, off);
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open));
        // Just inside it: 40 degrees.
        let off = (P.0 + 1.5 * 40f32.to_radians().cos(), P.1 + 1.5 * 40f32.to_radians().sin());
        let (mut b, party, mut rng) = fresh(&def, off);
        assert!(b.player_attack(&def, P, FACE_X, &party, &mut rng, &open));
    }

    #[test]
    fn a_boss_is_not_staggered_and_the_damage_follows_the_formula() {
        let mut def = foe(vec![]);
        def.boss = true;
        def.defense = 2;
        let (mut b, party, mut rng) = fresh(&def, (11.5, 10.0));
        assert!(b.player_attack(&def, P, FACE_X, &party, &mut rng, &open));
        assert_eq!(b.state, EnemyState::Chase);
        // attack 5, defense 2: base 8, 6 to 10 across the roll.
        assert!((90..=94).contains(&b.enemy_hp), "{}", b.enemy_hp);
    }

    #[test]
    fn the_swing_has_a_cooldown_that_a_miss_also_starts() {
        let def = foe(vec![]);
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 20.0));
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open), "a miss");
        assert!(b.player_swing > 0.0);
        b.enemy_pos = (11.5, 10.0);
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open), "still cooling down");
        run(&mut b, &def, &mut party, &mut rng, P, 0.5);
        assert!(b.player_attack(&def, P, FACE_X, &party, &mut rng, &open), "ready after 0.45 s");
    }

    #[test]
    fn blocking_cuts_damage_by_sixty_percent() {
        let def = foe(vec![]);
        let (mut b, mut party, _) = fresh(&def, (20.0, 20.0));
        assert_eq!(b.hurt_player(&mut party, 20, P), 19, "defense 2 takes 1 off");
        assert_eq!(party.hp, 11);
        b.player_iframes = 0.0;
        b.blocking = true;
        assert_eq!(b.hurt_player(&mut party, 20, P), 8);
        assert_eq!(party.hp, 3);
    }

    #[test]
    fn a_hit_gives_invulnerability_for_six_tenths_of_a_second() {
        let def = foe(vec![]);
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 20.0));
        b.hurt_player(&mut party, 5, P);
        let hp = party.hp;
        assert_eq!(b.hurt_player(&mut party, 5, P), 0, "inside the iframes");
        assert_eq!(party.hp, hp);
        run(&mut b, &def, &mut party, &mut rng, P, 0.7);
        assert!(b.hurt_player(&mut party, 5, P) > 0, "iframes are over");
    }

    #[test]
    fn zero_hp_is_a_loss_and_the_enemy_freezes() {
        let def = foe(vec![atk(AttackKind::Melee)]);
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 20.0));
        b.hurt_player(&mut party, 999, P);
        assert_eq!((party.hp, b.outcome), (0, Some(Outcome::Lost)));
        let at = b.enemy_pos;
        run(&mut b, &def, &mut party, &mut rng, P, 1.0);
        assert_eq!(b.enemy_pos, at);
        assert!(!b.player_attack(&def, P, FACE_X, &party, &mut rng, &open), "no swings after the fight");
    }

    #[test]
    fn a_slam_shows_its_circle_then_hurts_only_inside_the_radius() {
        let mut slam = atk(AttackKind::Slam);
        slam.range = 6.0;
        slam.radius = 1.5;
        slam.windup = 0.5;
        slam.damage = 10;
        let mut def = foe(vec![slam]);
        def.move_speed = 0.0;

        // The player stays put: inside the circle.
        let (mut b, mut party, mut rng) = fresh(&def, (14.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        assert!(matches!(b.state, EnemyState::Windup { .. }));
        let tele = &b.telegraphs[0];
        assert_eq!((tele.kind, tele.centre, tele.radius), (AttackKind::Slam, P, 1.5));
        assert!(tele.t > 0.0 && tele.t < tele.total);
        assert_eq!(party.hp, 30, "no damage during the telegraph");
        run(&mut b, &def, &mut party, &mut rng, P, 0.6);
        assert_eq!(party.hp, 30 - 9, "10 - defense 2 / 2");
        assert!(b.telegraphs.is_empty());

        // The player steps out of the circle during the windup.
        let (mut b, mut party, mut rng) = fresh(&def, (14.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        run(&mut b, &def, &mut party, &mut rng, (P.0, P.1 + 3.0), 0.6);
        assert_eq!(party.hp, 30, "the circle stayed where the player was");
    }

    #[test]
    fn a_shot_flies_at_the_player_hits_once_and_expires_when_it_misses() {
        let mut shot = atk(AttackKind::Shot);
        shot.range = 10.0;
        shot.radius = 0.3;
        shot.speed = 10.0;
        shot.windup = 0.3;
        shot.damage = 8;
        shot.cooldown = 100.0;
        let mut def = foe(vec![shot]);
        def.move_speed = 0.0;

        let (mut b, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.5);
        assert_eq!(b.projectiles.len(), 1, "fired at windup end");
        run(&mut b, &def, &mut party, &mut rng, P, 1.0);
        assert_eq!(party.hp, 30 - 7, "8 - defense 2 / 2, once");
        assert!(b.projectiles.is_empty(), "spent on the hit");

        // The player ducks out of the lane after the shot is fired: it flies past and fades.
        let (mut b, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.45);
        assert_eq!(b.projectiles.len(), 1);
        run(&mut b, &def, &mut party, &mut rng, (P.0, P.1 + 5.0), 6.0);
        assert_eq!(party.hp, 30);
        assert!(b.projectiles.is_empty(), "it expired");
    }

    #[test]
    fn a_shot_stops_at_a_wall() {
        let mut shot = atk(AttackKind::Shot);
        shot.windup = 0.1;
        let def = foe(vec![shot]);
        let (mut b, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        b.projectiles.push(Projectile { pos: (12.0, 10.0), vel: (-10.0, 0.0), radius: 0.2, damage: 5, life: 5.0 });
        let wall = |tx: i32, _tz: i32| tx <= 3;
        for _ in 0..40 {
            let mut cx = Ctx { player: (1.0, 10.0), party: &mut party, rng: &mut rng, blocks: &wall };
            b.tick(&def, 0.05, &mut cx);
        }
        assert!(b.projectiles.is_empty());
        assert_eq!(party.hp, 30);
    }

    #[test]
    fn a_charge_locks_its_lane_then_dashes_and_hurts_a_player_still_in_it() {
        let mut ch = atk(AttackKind::Charge);
        ch.range = 8.0;
        ch.windup = 0.4;
        ch.speed = 12.0;
        ch.damage = 12;
        let mut def = foe(vec![ch]);
        def.move_speed = 0.0;
        let (mut b, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        let tele = b.telegraphs[0].clone();
        assert_eq!(tele.kind, AttackKind::Charge);
        assert!(tele.dir.0 < -0.99, "a lane toward the player");
        run(&mut b, &def, &mut party, &mut rng, P, 1.2);
        assert!(b.enemy_pos.0 < 16.0, "dashing");
        assert!(party.hp < 30, "the dash reached the player");
        // Dodging sideways during the windup: the lane was locked.
        let (mut b, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        run(&mut b, &def, &mut party, &mut rng, (P.0, P.1 + 4.0), 1.0);
        assert_eq!(party.hp, 30);
    }

    #[test]
    fn a_melee_swing_telegraphs_then_hits_a_player_in_front() {
        let mut m = atk(AttackKind::Melee);
        m.range = 1.6;
        m.windup = 0.3;
        m.damage = 6;
        let mut def = foe(vec![m]);
        def.move_speed = 0.0;
        let (mut b, mut party, mut rng) = fresh(&def, (11.4, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 0.1);
        assert_eq!(b.telegraphs[0].kind, AttackKind::Melee);
        assert_eq!(party.hp, 30);
        run(&mut b, &def, &mut party, &mut rng, P, 0.4);
        assert_eq!(party.hp, 30 - 5);
        assert!(b.cooldowns[0] > 0.0);
    }

    #[test]
    fn the_enemy_chases_and_is_stopped_by_walls() {
        let def = foe(vec![]);
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 10.0));
        run(&mut b, &def, &mut party, &mut rng, P, 1.0);
        assert!((b.enemy_pos.0 - 18.0).abs() < 0.2, "2 m/s for a second: {:?}", b.enemy_pos);
        run(&mut b, &def, &mut party, &mut rng, P, 10.0);
        assert!((dist(b.enemy_pos, P) - STOP_DIST).abs() < 0.05, "stops next to the player");

        // A wall column at tile x = 7 (metres 14 to 16) between them.
        let (mut b, mut party, mut rng) = fresh(&def, (20.0, 10.0));
        let wall = |tx: i32, _tz: i32| tx == 7;
        // 3 s: well before the 4 s no-path rule would move it.
        for _ in 0..60 {
            let mut cx = Ctx { player: P, party: &mut party, rng: &mut rng, blocks: &wall };
            b.tick(&def, 0.05, &mut cx);
        }
        assert!(b.enemy_pos.0 >= 16.0 + ENEMY_RADIUS - 0.01, "{:?}", b.enemy_pos);
    }

    #[test]
    fn the_intro_ends_and_then_the_enemy_acts() {
        let def = foe(vec![]);
        let mut b = ActionBattle::new(&def, (20.0, 10.0));
        let mut party = Party::new(Stats::default());
        let mut rng = Rng::new(1);
        run(&mut b, &def, &mut party, &mut rng, P, 1.0);
        assert!(matches!(b.state, EnemyState::Intro { .. }));
        assert_eq!(b.enemy_pos, (20.0, 10.0), "stands still during the intro");
        run(&mut b, &def, &mut party, &mut rng, P, 0.4);
        assert_eq!(b.state, EnemyState::Chase);
    }

    #[test]
    fn crossing_an_hp_mark_changes_phase_unlocks_attacks_and_speeds_things_up() {
        let mut early = atk(AttackKind::Melee);
        early.min_phase = 0;
        let mut late = atk(AttackKind::Slam);
        late.min_phase = 1;
        late.range = 9.0;
        let mut def = foe(vec![early, late]);
        def.boss = true;
        def.phases = vec![66, 33];
        let (mut b, _party, _rng) = fresh(&def, (20.0, 10.0));
        assert_eq!(phase_for(&def, 100), 0);
        assert_eq!(phase_for(&def, 67), 0);
        assert_eq!(phase_for(&def, 66), 1);
        assert_eq!(phase_for(&def, 33), 2);
        let base_speed = b.move_speed(&def);
        let base_cd = b.cooldown_scale();
        b.hurt_enemy(&def, 30);
        assert_eq!(b.phase, 0, "70 percent left");
        b.shake = 0.0;
        b.hurt_enemy(&def, 10);
        assert_eq!(b.phase, 1, "60 percent left");
        assert!(b.hits.iter().any(|h| h.text == "Phase 2"));
        assert!(b.shake >= 8.0);
        assert!((b.move_speed(&def) - base_speed * 1.2).abs() < 1e-4);
        assert!((b.cooldown_scale() - base_cd * 0.85).abs() < 1e-4);

        // Phase 0 never picks the late slam, even at its range; phase 1 can.
        let (mut b0, mut party, mut rng) = fresh(&def, (16.0, 10.0));
        b0.cooldowns = vec![99.0, 0.0];
        run(&mut b0, &def, &mut party, &mut rng, P, 0.3);
        assert_eq!(b0.state, EnemyState::Chase, "the only ready attack is not unlocked yet");
        b0.phase = 1;
        run(&mut b0, &def, &mut party, &mut rng, P, 0.1);
        assert!(matches!(b0.state, EnemyState::Windup { attack: 1, .. }));
    }

    #[test]
    fn the_trail_eases_down_to_the_hp_and_numbers_age_out() {
        let def = foe(vec![]);
        let (mut b, mut party, mut rng) = fresh(&def, (30.0, 30.0));
        b.hurt_enemy(&def, 40);
        assert_eq!(b.enemy_hp, 60);
        assert_eq!(b.trail_hp, 100.0, "starts at the old value");
        run(&mut b, &def, &mut party, &mut rng, P, 0.2);
        assert!(b.trail_hp < 100.0 && b.trail_hp > 60.0);
        run(&mut b, &def, &mut party, &mut rng, P, 3.0);
        assert_eq!(b.trail_hp, 60.0);
        assert!(b.hits.is_empty(), "the floating numbers aged out");
        assert!((b.enemy_fraction(&def) - 0.6).abs() < 1e-5);
    }

    #[test]
    fn killing_the_enemy_wins_and_clears_the_field() {
        let def = foe(vec![]);
        let (mut b, ..) = fresh(&def, (30.0, 30.0));
        b.projectiles.push(Projectile { pos: P, vel: (0.0, 0.0), radius: 0.2, damage: 1, life: 1.0 });
        assert!(b.hurt_enemy(&def, 1000));
        assert_eq!((b.outcome, b.state, b.enemy_hp), (Some(Outcome::Won), EnemyState::Dead, 0));
        assert!(b.projectiles.is_empty());
        assert!(!b.hurt_enemy(&def, 5), "nothing after death");
    }
}
