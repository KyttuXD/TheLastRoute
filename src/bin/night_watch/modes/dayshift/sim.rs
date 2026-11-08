//! Salvage Run's rules (mode `dayshift`): a port of `dayshift_logic.gd` and `dayshift_data.gd`, number for number. Pure
//! logic: no scene, no drawing, no input device. The shift scene feeds `DayshiftSim::step` the doors, the lights and the
//! rack (hold on a piece) and shows what happens. It draws from `Mulberry32` in the same order as the Godot code, so the
//! same seed gives the same shift, event for event.
//!
//! The tunables (the `T` table, the upgrades, the budget / quota / level tables, the decks) live in
//! `data/dayshift/tunables.json`, embedded below.
//!
//! `step(dt_ms, &DayInput)` returns the events: item / expire / salvage / close / repelled / warned / knock / window / gone /
//! out / caught / win / short.
#![allow(dead_code)]

use crate::core::{ai_bump, hour_index, jf64, ji32, ji32s, jitter, jstr, jstrs, spread_ms, Fnv, Mulberry32, Outcome, Pacer, NIGHT_MS, TIME_SCALE};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const TUNABLES_JSON: &str = include_str!("../../data/dayshift/tunables.json");

/// The two door visitors, left door first.
pub const VISITORS: [&str; 2] = ["ravelle", "ceryth"];
/// The whole cast (Ysolde comes to the window).
pub const CAST_IDS: [&str; 3] = ["ravelle", "ceryth", "ysolde"];

// ---- the tunables ------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Upgrade {
    pub id: String,
    pub name: String,
    pub cost: i32,
    pub blurb: String,
}

#[derive(Clone, Debug, Default)]
pub struct ItemKind {
    pub id: String,
    pub label: String,
    pub value: i32,
    pub ms: f64,
    pub weight: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Deck {
    pub id: String,
    pub weight: i32,
    /// Pieces per kind, in `item_kinds` order.
    pub counts: Vec<i32>,
}

/// The `T` table of `dayshift_logic.gd`.
#[derive(Clone, Debug, Default)]
pub struct Tuning {
    pub combo_level: i32,
    pub combo_ms: f64,
    pub at_door_ms: f64,
    pub leave_ms: f64,
    pub start_dist: i32,
    pub at_door_by_week_ms: Vec<f64>,
    pub noise_per_sec: f64,
    pub noise_decay_per_sec: f64,
    pub loud: f64,
    pub uneasy: f64,
    pub power_base: f64,
    pub power_per_device: f64,
    pub out_attack_ms: f64,
    pub slots: usize,
    pub spawn_ms: f64,
    pub first_spawn_ms: f64,
    pub expire_ms: f64,
    pub window_gap_ms: f64,
    pub window_gap_per_level_ms: f64,
    pub window_gap_min_ms: f64,
    pub knock_ms: f64,
    pub present_ms: f64,
    pub expose_ms: f64,
    pub knock_max_ms: f64,
    pub window_gap_spread: f64,
    pub spawn_spread: f64,
    pub first_spawn_spread: f64,
    pub lead_steps: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Tunables {
    pub cast: Vec<String>,
    pub names: Vec<(String, String)>,
    pub pan: Vec<(String, f32)>,
    pub upgrades: Vec<Upgrade>,
    pub budget: Vec<i32>,
    pub quota: Vec<i32>,
    pub levels: Vec<(String, Vec<i32>)>,
    pub carry_cap: i32,
    pub item_kinds: Vec<ItemKind>,
    pub decks: Vec<Deck>,
    pub t: Tuning,
    /// Seconds a door visitor takes over one step (Godot `walker.go` default budget).
    pub step_budget_s: f32,
    /// Seconds the first walk in from her end of the ship takes.
    pub first_budget_s: f32,
    /// Seconds Ysolde takes to come across the yard to the glass.
    pub knock_budget_s: f32,
}

impl Tunables {
    pub fn parse(src: &str) -> Result<Tunables, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("dayshift tunables: {e}"))?;
        let mut out = Tunables::default();
        out.cast = jstrs(&v, "cast");
        if let Some(o) = v.get("names").and_then(|n| n.as_object()) {
            for (k, x) in o {
                out.names.push((k.clone(), x.as_str().unwrap_or("").to_string()));
            }
        }
        if let Some(o) = v.get("pan").and_then(|n| n.as_object()) {
            for (k, x) in o {
                out.pan.push((k.clone(), x.as_f64().unwrap_or(0.0) as f32));
            }
        }
        if let Some(a) = v.get("upgrades").and_then(|n| n.as_array()) {
            for u in a {
                out.upgrades.push(Upgrade { id: jstr(u, "id"), name: jstr(u, "name"), cost: ji32(u, "cost", 0), blurb: jstr(u, "blurb") });
            }
        }
        out.budget = ji32s(&v, "budget");
        out.quota = ji32s(&v, "quota");
        if let Some(o) = v.get("levels").and_then(|n| n.as_object()) {
            for (k, _) in o {
                out.levels.push((k.clone(), ji32s(&v["levels"], k)));
            }
        }
        out.carry_cap = ji32(&v, "carry_cap", 40);
        let kinds = jstrs(&v, "item_kinds");
        for k in &kinds {
            let it = &v["items"][k.as_str()];
            out.item_kinds.push(ItemKind { id: k.clone(), label: jstr(it, "label"), value: ji32(it, "value", 0), ms: jf64(it, "ms", 1000.0), weight: ji32(it, "weight", 1) });
        }
        if let Some(a) = v.get("decks").and_then(|n| n.as_array()) {
            for d in a {
                out.decks.push(Deck { id: jstr(d, "id"), weight: ji32(d, "weight", 1), counts: kinds.iter().map(|k| ji32(d, k, 0)).collect() });
            }
        }
        let t = &v["t"];
        out.t = Tuning {
            combo_level: ji32(t, "comboLevel", 6),
            combo_ms: jf64(t, "comboMs", 2200.0),
            at_door_ms: jf64(t, "atDoorMs", 5200.0),
            leave_ms: jf64(t, "leaveMs", 450.0),
            start_dist: ji32(t, "startDist", 3),
            at_door_by_week_ms: crate::core::jf64s(t, "atDoorByWeekMs"),
            noise_per_sec: jf64(t, "noisePerSec", 14.0),
            noise_decay_per_sec: jf64(t, "noiseDecayPerSec", 10.0),
            loud: jf64(t, "loud", 40.0),
            uneasy: jf64(t, "uneasy", 20.0),
            power_base: jf64(t, "powerBase", 0.10),
            power_per_device: jf64(t, "powerPerDevice", 0.07),
            out_attack_ms: jf64(t, "outAttackMs", 9000.0),
            slots: ji32(t, "slots", 4).max(1) as usize,
            spawn_ms: jf64(t, "spawnMs", 11000.0),
            first_spawn_ms: jf64(t, "firstSpawnMs", 2000.0),
            expire_ms: jf64(t, "expireMs", 30000.0),
            window_gap_ms: jf64(t, "windowGapMs", 42000.0),
            window_gap_per_level_ms: jf64(t, "windowGapPerLevelMs", 1800.0),
            window_gap_min_ms: jf64(t, "windowGapMinMs", 14000.0),
            knock_ms: jf64(t, "knockMs", 3000.0),
            present_ms: jf64(t, "presentMs", 6000.0),
            expose_ms: jf64(t, "exposeMs", 1500.0),
            knock_max_ms: jf64(t, "knockMaxMs", 4200.0),
            window_gap_spread: jf64(t, "windowGapSpread", 0.3),
            spawn_spread: jf64(t, "spawnSpread", 0.2),
            first_spawn_spread: jf64(t, "firstSpawnSpread", 0.5),
            lead_steps: ji32(t, "leadSteps", 1),
        };
        let w = &v["walk"];
        out.step_budget_s = jf64(w, "step_budget_s", 2.0) as f32;
        out.first_budget_s = jf64(w, "first_budget_s", 7.0) as f32;
        out.knock_budget_s = jf64(w, "knock_budget_s", 3.0) as f32;
        let problems = out.validate();
        if problems.is_empty() {
            Ok(out)
        } else {
            Err(format!("dayshift tunables: {}", problems.join("; ")))
        }
    }

    /// Problems in the tables (empty = fine). `--check` reports them.
    pub fn validate(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.cast != CAST_IDS.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
            p.push("cast must be ravelle, ceryth, ysolde".to_string());
        }
        if self.upgrades.len() != 5 {
            p.push(format!("{} upgrades, want 5", self.upgrades.len()));
        }
        if self.budget.len() != 5 || self.quota.len() != 5 {
            p.push("budget and quota need 5 weeks".to_string());
        }
        for id in CAST_IDS {
            match self.levels.iter().find(|(k, _)| k == id) {
                Some((_, l)) if l.len() == 5 => {}
                _ => p.push(format!("levels for {id} need 5 weeks")),
            }
        }
        if self.item_kinds.len() != 3 {
            p.push("three item kinds".to_string());
        }
        if self.decks.is_empty() || self.decks.iter().any(|d| d.counts.len() != self.item_kinds.len() || d.weight <= 0) {
            p.push("decks need a weight and a count per kind".to_string());
        }
        if self.t.at_door_by_week_ms.len() != 5 {
            p.push("atDoorByWeekMs needs 5 weeks".to_string());
        }
        if self.t.slots != 4 {
            p.push("the rack has 4 slots".to_string());
        }
        p
    }

    /// The base level of a hunter in a week (0 for an unknown id or week).
    pub fn level_of(&self, id: &str, week_idx: usize) -> i32 {
        self.levels.iter().find(|(k, _)| k == id).and_then(|(_, l)| l.get(week_idx.min(4)).copied()).unwrap_or(0)
    }
}

/// The parsed tables (an empty set if the embedded file is broken: `Tunables::validate` and `--check` say so).
pub fn tun() -> &'static Tunables {
    static TUN: OnceLock<Tunables> = OnceLock::new();
    TUN.get_or_init(|| Tunables::parse(TUNABLES_JSON).unwrap_or_default())
}

/// Problems in the embedded data (for `--check`).
pub fn data_problems() -> Vec<String> {
    match Tunables::parse(TUNABLES_JSON) {
        Ok(_) => Vec::new(),
        Err(e) => vec![e],
    }
}

/// "Ravelle".
pub fn name_of(id: &str) -> String {
    tun().names.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_else(|| crate::core::display_name(id))
}

/// Where a visitor is heard from: -1 left .. 1 right.
pub fn pan_of(id: &str) -> f32 {
    tun().pan.iter().find(|(k, _)| k == id).map(|(_, v)| *v).unwrap_or(0.0)
}

// ---- the shop ----------------------------------------------------------------------------------------------------------------

pub fn total_cost(ids: &[String]) -> i32 {
    ids.iter().map(|id| tun().upgrades.iter().filter(|u| &u.id == id).map(|u| u.cost).sum::<i32>()).sum()
}

/// Last week's surplus brought forward (only when you play on from one week to the next).
pub fn carry_from(money: i32, quota: i32) -> i32 {
    (money - quota).min(tun().carry_cap).max(0)
}

pub fn budget_for(week: i32, carry: i32) -> i32 {
    let t = tun();
    let base = t.budget.get((week.clamp(1, 5) - 1) as usize).copied().unwrap_or(0);
    base + carry.min(t.carry_cap).max(0)
}

pub fn quota_for(week: i32) -> i32 {
    tun().quota.get((week.clamp(1, 5) - 1) as usize).copied().unwrap_or(0)
}

fn is_upgrade(id: &str) -> bool {
    tun().upgrades.iter().any(|u| u.id == id)
}

/// Whether a basket of upgrades can be bought with the day's budget (every id real, none twice).
pub fn can_buy(ids: &[String], week: i32, carry: i32) -> bool {
    let mut seen: Vec<&String> = Vec::new();
    for id in ids {
        if seen.contains(&id) || !is_upgrade(id) {
            return false;
        }
        seen.push(id);
    }
    total_cost(ids) <= budget_for(week, carry)
}

/// One click in the shop: toggles `id` in the basket. The new basket, or None when it would go over the budget.
pub fn shop_toggle(owned: &[String], id: &str, week: i32, carry: i32) -> Option<Vec<String>> {
    let next: Vec<String> = if owned.iter().any(|x| x == id) {
        owned.iter().filter(|x| x.as_str() != id).cloned().collect()
    } else {
        let mut n = owned.to_vec();
        n.push(id.to_string());
        n
    };
    if can_buy(&next, week, carry) {
        Some(next)
    } else {
        None
    }
}

/// What Ysolde hears at the glass: a light on, or salvaging. Not the noise meter and not the doors.
pub fn ysolde_hears(dev: &Dev, working: i32) -> bool {
    dev.light_l || dev.light_r || working >= 0
}

/// The hunters with a level above zero in a week (week_index 0-based).
pub fn active_hunters(week_index: usize) -> Vec<String> {
    CAST_IDS.iter().filter(|id| tun().level_of(id, week_index) > 0).map(|s| s.to_string()).collect()
}

/// The upgrades a bot buys: the first affordable ones in this order.
pub fn bot_basket(week: i32, carry: i32) -> Vec<String> {
    let mut owned: Vec<String> = Vec::new();
    for id in ["gloves", "soles", "bench", "cell", "hinges"] {
        if let Some(next) = shop_toggle(&owned, id, week, carry) {
            owned = next;
        }
    }
    owned
}

// ---- state ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    None,
    Win,
    Short,
    Caught,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum YState {
    Away,
    Knock,
    Here,
}

/// The doors and lights as the step applied them (all off while the power is out).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Dev {
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
}

/// What the player does: hold on a rack slot (`target`, -1 none), shut the doors, hold the door lights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayInput {
    pub work: bool,
    pub target: i32,
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
}

impl Default for DayInput {
    fn default() -> Self {
        DayInput { work: false, target: -1, door_l: false, door_r: false, light_l: false, light_r: false }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub slot: usize,
    pub kind: String,
    pub value: i32,
    pub need: f64,
    pub progress: f64,
    pub age: f64,
}

#[derive(Clone, Debug)]
pub struct Stalker {
    /// Steps from her door (0 = at it).
    pub dist: i32,
    pub pacer: Pacer,
    pub at_ms: f64,
    pub leave: f64,
}

#[derive(Clone, Debug)]
pub struct YsoldeState {
    pub state: YState,
    pub t: f64,
    pub clock: f64,
    pub gap: f64,
    pub expose: f64,
    pub stay: f64,
    pub knock: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DayEvent {
    Item { slot: usize },
    Expire { slot: usize },
    Salvage { slot: usize, value: i32 },
    /// `idx` 0 = Ravelle (left), 1 = Ceryth (right); `dist` is the new distance.
    Close { idx: usize, dist: i32 },
    Repelled { idx: usize },
    Warned { idx: usize },
    Knock,
    Window,
    Gone,
    Out,
    Caught { by: String },
    Win { money: i32, quota: i32 },
    Short { money: i32, quota: i32 },
}

impl DayEvent {
    /// A short stable name (the headless event list and its digest).
    pub fn name(&self) -> String {
        match self {
            DayEvent::Item { slot } => format!("item:{slot}"),
            DayEvent::Expire { slot } => format!("expire:{slot}"),
            DayEvent::Salvage { slot, value } => format!("salvage:{slot}:{value}"),
            DayEvent::Close { idx, dist } => format!("close:{}:{dist}", VISITORS[(*idx).min(1)]),
            DayEvent::Repelled { idx } => format!("repelled:{}", VISITORS[(*idx).min(1)]),
            DayEvent::Warned { idx } => format!("warned:{}", VISITORS[(*idx).min(1)]),
            DayEvent::Knock => "knock".to_string(),
            DayEvent::Window => "window".to_string(),
            DayEvent::Gone => "gone".to_string(),
            DayEvent::Out => "out".to_string(),
            DayEvent::Caught { by } => format!("caught:{by}"),
            DayEvent::Win { .. } => "win".to_string(),
            DayEvent::Short { .. } => "short".to_string(),
        }
    }
}

/// Options of a shift (Godot `opts`).
#[derive(Clone, Debug, Default)]
pub struct DayOpts {
    pub upgrades: Vec<String>,
    /// Custom Night sliders (id -> level); an id missing here uses the week's table.
    pub levels: BTreeMap<String, i32>,
    pub infinite_power: bool,
    pub custom: bool,
    pub quota: Option<i32>,
}

// ---- the rules -----------------------------------------------------------------------------------------------------------------

pub struct DayshiftSim {
    pub week_idx: usize,
    pub rng: Mulberry32,
    pub upgrades: Vec<String>,
    pub levels: BTreeMap<String, i32>,
    pub infinite_power: bool,
    pub custom: bool,
    pub need: i32,
    pub door_ms: f64,
    /// Admin: a multiplier on how fast the doors' visitors move (1.0 = the 2D game's numbers exactly).
    pub hunter_speed: f64,
    pub elapsed: f64,
    pub over: Over,
    pub by: String,
    pub power: f64,
    pub out: bool,
    pub out_ms: f64,
    pub noise: f64,
    pub money: i32,
    pub items: Vec<Item>,
    pub spawn: f64,
    pub spawn_at: f64,
    pub working: i32,
    /// Index into `Tunables::decks`.
    pub deck: usize,
    /// Item kind indices still to deal; the last is dealt first.
    pub deck_left: Vec<usize>,
    pub lead: Option<usize>,
    pub stalkers: [Stalker; 2],
    pub ysolde: YsoldeState,
    pub dev: Dev,
    dt: f64,
}

impl DayshiftSim {
    /// Starts a shift: the three variety draws happen here, once, in this order: which deck, which door visitor leads, the
    /// wait for the first piece.
    pub fn new(week: i32, rng: Mulberry32, opts: DayOpts) -> DayshiftSim {
        let t = tun();
        let week_idx = (week.clamp(1, 5) - 1) as usize;
        let door_ms = if opts.custom { t.t.at_door_ms } else { t.t.at_door_by_week_ms.get(week_idx).copied().unwrap_or(t.t.at_door_ms) };
        let need = opts.quota.unwrap_or_else(|| quota_for(week));
        let stalker = || Stalker { dist: t.t.start_dist, pacer: Pacer::new(), at_ms: 0.0, leave: 0.0 };
        let mut sim = DayshiftSim {
            week_idx,
            rng,
            upgrades: opts.upgrades,
            levels: opts.levels,
            infinite_power: opts.infinite_power,
            custom: opts.custom,
            need,
            door_ms,
            hunter_speed: 1.0,
            elapsed: 0.0,
            over: Over::None,
            by: String::new(),
            power: 100.0,
            out: false,
            out_ms: 0.0,
            noise: 0.0,
            money: 0,
            items: Vec::new(),
            spawn: 0.0,
            spawn_at: 0.0,
            working: -1,
            deck: 0,
            deck_left: Vec::new(),
            lead: None,
            stalkers: [stalker(), stalker()],
            ysolde: YsoldeState { state: YState::Away, t: 0.0, clock: 0.0, gap: -1.0, expose: 0.0, stay: t.t.present_ms, knock: t.t.knock_ms },
            dev: Dev::default(),
            dt: 0.0,
        };
        let total: i32 = t.decks.iter().map(|d| d.weight).sum();
        let mut r = sim.rng.next() * total as f64;
        for (i, d) in t.decks.iter().enumerate() {
            r -= d.weight as f64;
            if r < 0.0 {
                sim.deck = i;
                break;
            }
        }
        let u = sim.rng.next();
        if sim.base_level("ravelle") > 0 && sim.base_level("ceryth") > 0 {
            let lead = if u < 0.5 { 0 } else { 1 };
            sim.lead = Some(lead);
            let other = 1 - lead;
            sim.stalkers[lead].dist = t.t.start_dist - t.t.lead_steps;
            sim.stalkers[other].dist = t.t.start_dist + t.t.lead_steps;
        }
        sim.spawn_at = jitter(t.t.first_spawn_ms, t.t.first_spawn_spread, &mut sim.rng);
        sim
    }

    pub fn has_upgrade(&self, id: &str) -> bool {
        self.upgrades.iter().any(|u| u == id)
    }

    /// The level before the hour, the noise and the soft soles (a Custom Night slider wins over the week's table).
    pub fn base_level(&self, id: &str) -> i32 {
        match self.levels.get(id) {
            Some(l) if *l >= 0 => *l,
            _ => tun().level_of(id, self.week_idx),
        }
    }

    pub fn level(&self, id: &str) -> i32 {
        let b = self.base_level(id);
        if b <= 0 {
            return 0;
        }
        let t = &tun().t;
        let bonus = if self.noise >= t.loud {
            3
        } else if self.noise >= t.uneasy {
            1
        } else {
            0
        };
        let soles = if self.has_upgrade("soles") { 2 } else { 0 };
        (b + ai_bump(hour_index(self.elapsed)) + bonus - soles).clamp(1, 20)
    }

    /// The hunters with a level above zero in this shift.
    pub fn active(&self) -> Vec<&'static str> {
        CAST_IDS.iter().copied().filter(|id| self.base_level(id) > 0).collect()
    }

    pub fn outcome(&self) -> Outcome {
        match self.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Short | Over::Caught => Outcome::Lost,
        }
    }

    /// The next piece off the deck; the deck is shuffled again when it is empty.
    fn deal_kind(&mut self) -> usize {
        if self.deck_left.is_empty() {
            let mut bag: Vec<usize> = Vec::new();
            if let Some(d) = tun().decks.get(self.deck) {
                for (ki, n) in d.counts.iter().enumerate() {
                    for _ in 0..(*n).max(0) {
                        bag.push(ki);
                    }
                }
            }
            let mut i = bag.len() as i64 - 1;
            while i > 0 {
                let j = (self.rng.next() * (i as f64 + 1.0)).floor() as usize;
                bag.swap(i as usize, j.min(i as usize));
                i -= 1;
            }
            self.deck_left = bag;
        }
        self.deck_left.pop().unwrap_or(0)
    }

    fn spawn_item(&mut self) -> Option<usize> {
        let t = tun();
        let free: Vec<usize> = (0..t.t.slots).filter(|i| !self.items.iter().any(|it| it.slot == *i)).collect();
        if free.is_empty() {
            return None;
        }
        let kind = self.deal_kind();
        let pick = ((self.rng.next() * free.len() as f64).floor() as usize).min(free.len() - 1);
        let slot = free[pick];
        let k = t.item_kinds.get(kind)?;
        let gloves = if self.has_upgrade("gloves") { 0.7 } else { 1.0 };
        self.items.push(Item { slot, kind: k.id.clone(), value: k.value, need: k.ms * gloves, progress: 0.0, age: 0.0 });
        Some(slot)
    }

    /// A rack piece as the admin menu adds it (no draws).
    pub fn add_piece(&mut self, kind: &str) -> bool {
        let t = tun();
        let Some(k) = t.item_kinds.iter().find(|k| k.id == kind) else { return false };
        let Some(slot) = (0..t.t.slots).find(|i| !self.items.iter().any(|it| it.slot == *i)) else { return false };
        let gloves = if self.has_upgrade("gloves") { 0.7 } else { 1.0 };
        self.items.push(Item { slot, kind: k.id.clone(), value: k.value, need: k.ms * gloves, progress: 0.0, age: 0.0 });
        true
    }

    /// A doorway visitor: closes in a step at a time on the random wait (held still by the light), then waits at the door.
    /// True when she gets in.
    fn visitor(&mut self, i: usize, shut: bool, lit: bool, ev: &mut Vec<DayEvent>) -> bool {
        let t = &tun().t;
        let dt = self.dt;
        let level = self.level(VISITORS[i]);
        let speed = self.hunter_speed;
        let door_ms = self.door_ms;
        let rng = &mut self.rng;
        let h = &mut self.stalkers[i];
        if h.dist > 0 {
            if !lit && h.pacer.step(dt * speed, level, &mut || rng.next()) {
                h.dist -= 1;
                ev.push(DayEvent::Close { idx: i, dist: h.dist });
            }
            return false;
        }
        if shut {
            h.at_ms = 0.0;
            h.leave += dt;
            if h.leave >= t.leave_ms {
                h.dist = 2;
                h.leave = 0.0;
                h.pacer.reset();
                ev.push(DayEvent::Repelled { idx: i });
            }
            return false;
        }
        h.leave = 0.0;
        if !lit {
            h.at_ms += dt;
        }
        h.at_ms >= door_ms
    }

    fn finish(&mut self, mut ev: Vec<DayEvent>, by: &str) -> Vec<DayEvent> {
        self.over = Over::Caught;
        self.by = by.to_string();
        ev.push(DayEvent::Caught { by: by.to_string() });
        ev
    }

    /// Steps the night by `dt_ms` with the doors, lights and rack as `input` has them.
    pub fn step(&mut self, dt_ms: f64, input: &DayInput) -> Vec<DayEvent> {
        if self.over != Over::None {
            return Vec::new();
        }
        let t = &tun().t;
        let mut ev: Vec<DayEvent> = Vec::new();
        self.dt = dt_ms;
        self.elapsed += dt_ms;
        let dt = dt_ms / 1000.0;
        let dev = if self.out {
            Dev::default()
        } else {
            Dev { door_l: input.door_l, door_r: input.door_r, light_l: input.light_l, light_r: input.light_r }
        };
        self.dev = dev;
        // Power: doors and lights draw it. At zero everything drops and someone arrives a few seconds later.
        if !self.out {
            let doors = dev.door_l as i32 + dev.door_r as i32;
            let lights = dev.light_l as i32 + dev.light_r as i32;
            let hinge = if self.has_upgrade("hinges") { 0.5 } else { 1.0 };
            let cell = if self.has_upgrade("cell") { 0.75 } else { 1.0 };
            let draw = (t.power_base / TIME_SCALE + t.power_per_device / TIME_SCALE * (doors as f64 * hinge + lights as f64)) * cell;
            if !self.infinite_power {
                self.power = (self.power - draw * dt).max(0.0);
            }
            if self.power <= 0.0 {
                self.out = true;
                ev.push(DayEvent::Out);
            }
        } else {
            self.out_ms += dt_ms;
            if self.out_ms >= t.out_attack_ms {
                return self.finish(ev, "ravelle");
            }
        }
        // The rack: pieces turn up, wait, and are taken by holding on one. Salvaging is loud.
        self.noise = (self.noise - t.noise_decay_per_sec * dt).max(0.0);
        self.spawn += dt_ms;
        if self.spawn >= self.spawn_at {
            self.spawn = 0.0;
            let slot = self.spawn_item();
            self.spawn_at = jitter(t.spawn_ms, t.spawn_spread, &mut self.rng);
            if let Some(slot) = slot {
                ev.push(DayEvent::Item { slot });
            }
        }
        let mut i = 0;
        while i < self.items.len() {
            self.items[i].age += dt_ms;
            if self.items[i].age >= t.expire_ms {
                let it = self.items.remove(i);
                ev.push(DayEvent::Expire { slot: it.slot });
            } else {
                i += 1;
            }
        }
        let target = if input.target >= 0 { self.items.iter().position(|it| it.slot as i32 == input.target) } else { None };
        self.working = match target {
            Some(k) if input.work && !self.out => self.items[k].slot as i32,
            _ => -1,
        };
        if self.working >= 0 {
            if let Some(k) = target {
                self.items[k].progress += dt_ms;
                let bench = if self.has_upgrade("bench") { 0.5 } else { 1.0 };
                self.noise = (self.noise + t.noise_per_sec * bench * dt + t.noise_decay_per_sec * dt).min(100.0);
                if self.items[k].progress >= self.items[k].need {
                    let it = self.items.remove(k);
                    self.money += it.value;
                    ev.push(DayEvent::Salvage { slot: it.slot, value: it.value });
                }
            }
        }
        // Doorway visitors.
        let before = ev.len();
        if self.visitor(0, dev.door_l, dev.light_l, &mut ev) {
            return self.finish(ev, "ravelle");
        }
        if self.visitor(1, dev.door_r, dev.light_r, &mut ev) {
            return self.finish(ev, "ceryth");
        }
        // Working together: when one is turned away from a shut door she warns the other, who (once both are keen) is that
        // much nearer a step.
        let fresh: Vec<DayEvent> = ev[before..].to_vec();
        for e in fresh {
            if let DayEvent::Repelled { idx } = e {
                let other = 1 - idx;
                if self.stalkers[other].dist > 0 && self.level("ravelle") >= t.combo_level && self.level("ceryth") >= t.combo_level {
                    self.stalkers[other].pacer.skip(t.combo_ms);
                    ev.push(DayEvent::Warned { idx: other });
                }
            }
        }
        // Ysolde: a knock, then she stays at the window; be quiet while she does.
        let ul = self.level("ysolde");
        if ul > 0 {
            let working = self.working;
            let rng = &mut self.rng;
            let u = &mut self.ysolde;
            match u.state {
                YState::Away => {
                    u.clock += dt_ms;
                    if u.gap < 0.0 {
                        let base = t.window_gap_min_ms.max(t.window_gap_ms - ul as f64 * t.window_gap_per_level_ms);
                        u.gap = jitter(base, t.window_gap_spread, rng);
                    }
                    if u.clock >= u.gap {
                        u.state = YState::Knock;
                        u.t = 0.0;
                        u.clock = 0.0;
                        u.gap = -1.0;
                        let lo = t.knock_ms;
                        let hi = t.knock_max_ms;
                        u.knock = lo.max(jitter((lo + hi) / 2.0, (hi - lo) / (lo + hi), rng));
                        ev.push(DayEvent::Knock);
                    }
                }
                YState::Knock => {
                    u.t += dt_ms;
                    if u.t >= u.knock {
                        u.state = YState::Here;
                        u.t = 0.0;
                        u.expose = 0.0;
                        u.stay = spread_ms(t.present_ms, rng, 0.2);
                        ev.push(DayEvent::Window);
                    }
                }
                YState::Here => {
                    u.t += dt_ms;
                    let loud = ysolde_hears(&dev, working);
                    u.expose = if loud { u.expose + dt_ms } else { (u.expose - dt_ms).max(0.0) };
                    if u.expose >= t.expose_ms {
                        return self.finish(ev, "ysolde");
                    }
                    if u.t >= u.stay {
                        u.state = YState::Away;
                        u.t = 0.0;
                        u.expose = 0.0;
                        u.gap = -1.0;
                        ev.push(DayEvent::Gone);
                    }
                }
            }
        }
        // 6 AM: bring the quota in.
        if self.elapsed >= NIGHT_MS {
            if self.money >= self.need {
                self.over = Over::Win;
                ev.push(DayEvent::Win { money: self.money, quota: self.need });
            } else {
                self.over = Over::Short;
                ev.push(DayEvent::Short { money: self.money, quota: self.need });
            }
        }
        ev
    }

    // ---- admin / god helpers (the Godot `admin_*` hooks that change the rules' state)

    /// God mode: whoever would catch you (or a missed quota at 6 AM) is sent back, the power comes back.
    pub fn god_rescue(&mut self) {
        self.over = Over::None;
        self.by.clear();
        for h in self.stalkers.iter_mut() {
            h.dist = tun().t.start_dist;
            h.at_ms = 0.0;
            h.leave = 0.0;
        }
        self.ysolde.state = YState::Away;
        self.ysolde.t = 0.0;
        self.ysolde.expose = 0.0;
        self.ysolde.gap = -1.0;
        self.out = false;
        self.out_ms = 0.0;
        self.power = self.power.max(20.0);
        if self.elapsed >= NIGHT_MS {
            self.elapsed = NIGHT_MS - 1000.0;
        }
    }

    /// Place a visitor: far = three steps down the hall, near = one, door = at it.
    pub fn place_visitor(&mut self, idx: usize, place: &str) {
        let h = &mut self.stalkers[idx.min(1)];
        h.dist = match place {
            "far" => 3,
            "near" => 1,
            _ => 0,
        };
        h.at_ms = 0.0;
        h.leave = 0.0;
        h.pacer.reset();
    }

    /// Place Ysolde: far = gone, near = knocking, door / window = at the glass.
    pub fn place_ysolde(&mut self, place: &str) {
        let u = &mut self.ysolde;
        match place {
            "far" => {
                u.state = YState::Away;
                u.t = 0.0;
                u.expose = 0.0;
                u.clock = 0.0;
                u.gap = -1.0;
            }
            "near" => {
                u.state = YState::Knock;
                u.t = 0.0;
                u.expose = 0.0;
                u.knock = 3000.0;
            }
            _ => {
                u.state = YState::Here;
                u.t = 0.0;
                u.expose = 0.0;
                u.stay = 6000.0;
            }
        }
    }

    /// Jump the clock to an hour (0..5).
    pub fn set_hour(&mut self, hour: i32) {
        self.elapsed = (hour as f64 * crate::core::HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0);
    }

    /// FNV-1a of the state that matters (stable across runs).
    pub fn digest(&self) -> u64 {
        let mut f = Fnv::new();
        f.f64(self.elapsed).i32(self.over as i32).f64(self.power).f64(self.noise).i32(self.money).i32(self.out as i32);
        f.i32(self.working);
        for it in &self.items {
            f.i32(it.slot as i32).str(&it.kind).f64(it.progress).f64(it.age);
        }
        for h in &self.stalkers {
            f.i32(h.dist).f64(h.at_ms).f64(h.leave).f64(h.pacer.t).f64(h.pacer.next);
        }
        f.i32(self.ysolde.state as i32).f64(self.ysolde.t).f64(self.ysolde.clock).f64(self.ysolde.expose);
        f.u64(self.rng.state() as u64);
        f.str(&self.by);
        f.finish()
    }
}

// ---- the scripted player (headless runs) -------------------------------------------------------------------------------------

/// A seeded autoplay policy: shuts a door when its visitor is at it, lets go of the rack while Ysolde is about, otherwise
/// salvages the most valuable piece on the rack. It has its own stream, so it never touches the shift's draws.
#[derive(Clone, Debug)]
pub struct Autoplay {
    rng: Mulberry32,
    target: i32,
    lapse_ms: f64,
}

impl Autoplay {
    pub fn new(seed: u32) -> Autoplay {
        Autoplay { rng: Mulberry32::new(seed ^ 0x4441_5953), target: -1, lapse_ms: 0.0 }
    }

    pub fn input(&mut self, sim: &DayshiftSim, dt_ms: f64) -> DayInput {
        let mut inp = DayInput::default();
        inp.door_l = sim.base_level("ravelle") > 0 && sim.stalkers[0].dist == 0;
        inp.door_r = sim.base_level("ceryth") > 0 && sim.stalkers[1].dist == 0;
        if sim.ysolde.state != YState::Away || sim.out {
            self.target = -1;
            return inp;
        }
        if self.lapse_ms > 0.0 {
            self.lapse_ms -= dt_ms;
            return inp;
        }
        // now and then she looks away for a few seconds
        if self.rng.next() < 0.0015 {
            self.lapse_ms = 2000.0 + self.rng.next() * 3000.0;
            return inp;
        }
        if !sim.items.iter().any(|it| it.slot as i32 == self.target) {
            let best = sim.items.iter().max_by(|a, b| a.value.cmp(&b.value).then(b.slot.cmp(&a.slot)));
            self.target = best.map(|it| it.slot as i32).unwrap_or(-1);
        }
        inp.work = self.target >= 0;
        inp.target = self.target;
        inp
    }
}

/// Why this mode cannot start, if so: Day Shift cannot start: the tunables, the nav file or the map does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    Tunables::parse(TUNABLES_JSON).err().or_else(super::stage::load_error).or_else(super::world::load_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(Tunables::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn sim(week: i32, seed: u32) -> DayshiftSim {
        DayshiftSim::new(week, Mulberry32::new(seed), DayOpts::default())
    }

    fn quiet(s: &mut DayshiftSim) {
        s.spawn_at = 1.0e12;
    }

    fn run(s: &mut DayshiftSim, ms: f64, input: DayInput) -> Vec<DayEvent> {
        let mut out = Vec::new();
        let mut left = ms;
        while left > 0.0 && s.over == Over::None {
            let d = left.min(100.0);
            out.extend(s.step(d, &input));
            left -= d;
        }
        out
    }

    #[test]
    fn the_embedded_tables_validate() {
        assert!(Tunables::parse(TUNABLES_JSON).is_ok(), "{:?}", data_problems());
        assert!(data_problems().is_empty());
        assert_eq!(tun().upgrades.len(), 5);
        assert_eq!(tun().decks.len(), 3);
        assert_eq!(tun().t.slots, 4);
    }

    #[test]
    fn budgets_and_quotas_follow_the_tables() {
        assert_eq!(budget_for(1, 0), 90);
        assert_eq!(budget_for(5, 0), 130);
        assert_eq!(budget_for(3, 99), 110 + 40);
        assert_eq!(budget_for(2, -5), 100);
        assert_eq!(budget_for(9, 0), 130);
        assert_eq!(quota_for(1), 140);
        assert_eq!(quota_for(4), 340);
        assert_eq!(quota_for(9), 380);
        assert_eq!(quota_for(0), 140);
    }

    #[test]
    fn the_carry_is_the_surplus_up_to_the_cap() {
        assert_eq!(carry_from(200, 140), 40);
        assert_eq!(carry_from(150, 140), 10);
        assert_eq!(carry_from(100, 140), 0);
    }

    #[test]
    fn can_buy_checks_ids_duplicates_and_the_budget() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(can_buy(&v(&[]), 1, 0));
        assert!(can_buy(&v(&["gloves", "soles"]), 1, 0)); // 80 of 90
        assert!(!can_buy(&v(&["gloves", "soles", "bench"]), 1, 0)); // 120
        assert!(!can_buy(&v(&["gloves", "gloves"]), 5, 0));
        assert!(!can_buy(&v(&["nothing"]), 5, 0));
        assert!(can_buy(&v(&["gloves", "soles", "bench"]), 1, 40)); // the carry pays for it
        assert_eq!(total_cost(&v(&["hinges", "cell"])), 70);
    }

    #[test]
    fn shop_toggle_adds_removes_and_refuses_over_budget() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(shop_toggle(&[], "hinges", 1, 0), Some(v(&["hinges"])));
        assert_eq!(shop_toggle(&v(&["hinges"]), "hinges", 1, 0), Some(Vec::new()));
        assert_eq!(shop_toggle(&v(&["gloves", "soles"]), "bench", 1, 0), None);
        assert_eq!(shop_toggle(&[], "bogus", 1, 0), None);
        // taking one back is always allowed
        assert!(shop_toggle(&v(&["gloves", "soles"]), "gloves", 1, 0).is_some());
        let bot = bot_basket(1, 0);
        assert!(can_buy(&bot, 1, 0) && !bot.is_empty());
    }

    #[test]
    fn who_is_on_each_week() {
        assert_eq!(active_hunters(0), vec!["ravelle".to_string()]);
        assert_eq!(active_hunters(1), vec!["ravelle".to_string(), "ceryth".to_string()]);
        assert_eq!(active_hunters(2).len(), 3);
        assert_eq!(active_hunters(4).len(), 3);
    }

    #[test]
    fn the_variety_draws_come_in_the_godot_order() {
        // week 1: only Ravelle, so no lead is chosen but the draw is still taken; the third draw is the first spawn wait
        let mut r = Mulberry32::new(7);
        let d1 = r.next();
        let _d2 = r.next();
        let d3 = r.next();
        let s = sim(1, 7);
        let total: i32 = tun().decks.iter().map(|d| d.weight).sum();
        let mut x = d1 * total as f64;
        let mut deck = 0;
        for (i, d) in tun().decks.iter().enumerate() {
            x -= d.weight as f64;
            if x < 0.0 {
                deck = i;
                break;
            }
        }
        assert_eq!(s.deck, deck);
        assert!(s.lead.is_none());
        assert!((s.spawn_at - 2000.0 * (0.5 + d3)).abs() < 1e-9, "{} vs {}", s.spawn_at, 2000.0 * (0.5 + d3));
        // week 2: both door visitors are on, so one leads by a step each way
        let s2 = sim(2, 7);
        let lead = s2.lead.expect("a lead");
        assert_eq!(s2.stalkers[lead].dist, 2);
        assert_eq!(s2.stalkers[1 - lead].dist, 4);
        assert_eq!(s2.lead.is_some(), true);
    }

    #[test]
    fn levels_add_the_hour_the_noise_and_the_soft_soles() {
        let mut s = sim(3, 1); // ravelle 10, ceryth 8, ysolde 6
        assert_eq!(s.level("ravelle"), 10);
        assert_eq!(s.level("ceryth"), 8);
        s.elapsed = 3.0 * 45_000.0;
        assert_eq!(s.level("ravelle"), 11);
        s.elapsed = 4.0 * 45_000.0;
        assert_eq!(s.level("ravelle"), 12);
        s.noise = 25.0;
        assert_eq!(s.level("ravelle"), 13);
        s.noise = 45.0;
        assert_eq!(s.level("ravelle"), 15);
        s.upgrades.push("soles".to_string());
        assert_eq!(s.level("ravelle"), 13);
        // a level-0 hunter never wakes, and the top is 20
        let mut w1 = sim(1, 1);
        assert_eq!(w1.level("ceryth"), 0);
        w1.levels.insert("ceryth".to_string(), 20);
        w1.noise = 90.0;
        w1.elapsed = 5.0 * 45_000.0;
        assert_eq!(w1.level("ceryth"), 20);
    }

    #[test]
    fn salvaging_pays_when_the_hold_time_is_up() {
        let mut s = sim(1, 3);
        quiet(&mut s);
        s.items.push(Item { slot: 0, kind: "scrap".into(), value: 10, need: 2000.0, progress: 0.0, age: 0.0 });
        let hold = DayInput { work: true, target: 0, ..DayInput::default() };
        let ev = run(&mut s, 1900.0, hold);
        assert!(!ev.iter().any(|e| matches!(e, DayEvent::Salvage { .. })));
        assert_eq!(s.money, 0);
        let ev = run(&mut s, 200.0, hold);
        assert!(ev.contains(&DayEvent::Salvage { slot: 0, value: 10 }), "{ev:?}");
        assert_eq!(s.money, 10);
        assert!(s.items.is_empty());
        // letting go keeps the progress, but it does not advance
        s.items.push(Item { slot: 1, kind: "parts".into(), value: 25, need: 3500.0, progress: 0.0, age: 0.0 });
        run(&mut s, 1000.0, hold); // target 0 is empty
        assert_eq!(s.items[0].progress, 0.0);
        assert_eq!(s.working, -1);
        let on1 = DayInput { work: true, target: 1, ..DayInput::default() };
        run(&mut s, 1000.0, on1);
        assert!((s.items[0].progress - 1000.0).abs() < 1e-9);
        run(&mut s, 500.0, DayInput::default());
        assert!((s.items[0].progress - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn gloves_make_pieces_faster_and_the_bench_makes_less_noise() {
        let mut s = DayshiftSim::new(1, Mulberry32::new(5), DayOpts { upgrades: vec!["gloves".into()], ..DayOpts::default() });
        assert!(s.add_piece("scrap"));
        assert!((s.items[0].need - 1400.0).abs() < 1e-9);
        // noise: +14 a second while working (the decay is paid back; the first step starts from zero, so it keeps 1.0 more), +7 with the bench
        let mut a = sim(1, 5);
        quiet(&mut a);
        a.items.push(Item { slot: 0, kind: "prize".into(), value: 60, need: 99_999.0, progress: 0.0, age: 0.0 });
        let hold = DayInput { work: true, target: 0, ..DayInput::default() };
        run(&mut a, 1000.0, hold);
        assert!((a.noise - 15.0).abs() < 1e-6, "{}", a.noise);
        let mut b = DayshiftSim::new(1, Mulberry32::new(5), DayOpts { upgrades: vec!["bench".into()], ..DayOpts::default() });
        quiet(&mut b);
        b.items.push(Item { slot: 0, kind: "prize".into(), value: 60, need: 99_999.0, progress: 0.0, age: 0.0 });
        run(&mut b, 1000.0, hold);
        assert!((b.noise - 8.0).abs() < 1e-6, "{}", b.noise);
        // and it decays when she stops
        a.noise = 50.0;
        run(&mut a, 1000.0, DayInput::default());
        assert!((a.noise - 40.0).abs() < 1e-6);
        s.items.clear();
    }

    #[test]
    fn a_piece_expires_after_thirty_seconds() {
        let mut s = sim(1, 9);
        quiet(&mut s);
        s.items.push(Item { slot: 2, kind: "scrap".into(), value: 10, need: 2000.0, progress: 0.0, age: 0.0 });
        let ev = run(&mut s, 29_900.0, DayInput { door_l: false, ..DayInput::default() });
        assert!(!ev.iter().any(|e| matches!(e, DayEvent::Expire { .. })) || s.over != Over::None);
        if s.over == Over::None {
            let ev = run(&mut s, 200.0, DayInput::default());
            assert!(ev.contains(&DayEvent::Expire { slot: 2 }));
            assert!(s.items.is_empty());
        }
    }

    #[test]
    fn power_drains_with_doors_and_lights_and_the_upgrades_cut_it() {
        let mut s = sim(1, 2);
        quiet(&mut s);
        s.stalkers[0].dist = 3;
        run(&mut s, 10_000.0, DayInput::default());
        let idle = 100.0 - s.power;
        assert!((idle - 10.0 * 0.10 / TIME_SCALE).abs() < 1e-6, "{idle}");
        // both doors and both lights: 0.10 + 0.07 * 4
        let mut d = sim(1, 2);
        quiet(&mut d);
        let all = DayInput { door_l: true, door_r: true, light_l: true, light_r: true, ..DayInput::default() };
        d.stalkers[0].dist = 3;
        d.step(1000.0, &all);
        assert!((100.0 - d.power - (0.10 + 0.07 * 4.0) / TIME_SCALE).abs() < 1e-6);
        // hinges halve the doors, the cell cuts a quarter
        let mut h = DayshiftSim::new(1, Mulberry32::new(2), DayOpts { upgrades: vec!["hinges".into(), "cell".into()], ..DayOpts::default() });
        quiet(&mut h);
        h.step(1000.0, &all);
        let want = (0.10 + 0.07 * (2.0 * 0.5 + 2.0)) / TIME_SCALE * 0.75;
        assert!((100.0 - h.power - want).abs() < 1e-6);
        // infinite power never drains
        let mut inf = DayshiftSim::new(1, Mulberry32::new(2), DayOpts { infinite_power: true, ..DayOpts::default() });
        quiet(&mut inf);
        inf.step(1000.0, &all);
        assert_eq!(inf.power, 100.0);
    }

    #[test]
    fn the_cut_drops_everything_and_ravelle_comes_in_after_nine_seconds() {
        let mut s = sim(1, 4);
        quiet(&mut s);
        s.power = 0.0001;
        let all = DayInput { door_l: true, light_l: true, ..DayInput::default() };
        let ev = s.step(100.0, &all);
        assert!(ev.contains(&DayEvent::Out));
        assert!(s.out);
        // everything is dropped from now on, whatever is pressed
        s.stalkers[0].dist = 3;
        let ev = s.step(100.0, &all);
        assert_eq!(s.dev, Dev::default());
        assert!(!ev.contains(&DayEvent::Out));
        let mut t = 200.0;
        while s.over == Over::None && t < 20_000.0 {
            s.step(100.0, &all);
            t += 100.0;
        }
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "ravelle");
        assert!(t >= 9_000.0 && t <= 9_500.0, "{t}");
    }

    #[test]
    fn a_light_holds_a_visitor_still_and_the_wait_is_the_levels_window() {
        let mut lit = sim(1, 11);
        quiet(&mut lit);
        let hold_light = DayInput { light_l: true, ..DayInput::default() };
        let ev = run(&mut lit, 60_000.0, hold_light);
        assert_eq!(lit.stalkers[0].dist, 3);
        assert!(!ev.iter().any(|e| matches!(e, DayEvent::Close { .. })));
        // no light: she closes in once, between 11 s and 14 s (level 4: lo = 13000 - 500 * 4)
        let mut s = sim(1, 11);
        quiet(&mut s);
        let mut t = 0.0;
        loop {
            t += 100.0;
            let ev = s.step(100.0, &DayInput::default());
            if ev.iter().any(|e| matches!(e, DayEvent::Close { idx: 0, dist: 2 })) {
                break;
            }
            assert!(t < 20_000.0, "she never moved");
        }
        assert!(t >= 11_000.0 && t <= 14_100.0, "{t}");
    }

    #[test]
    fn a_shut_door_repels_after_450_ms_and_an_open_one_lets_her_in() {
        let mut s = sim(1, 6);
        quiet(&mut s);
        s.stalkers[0].dist = 0;
        let shut = DayInput { door_l: true, ..DayInput::default() };
        let ev = run(&mut s, 400.0, shut);
        assert!(ev.is_empty());
        let ev = run(&mut s, 100.0, shut);
        assert!(ev.contains(&DayEvent::Repelled { idx: 0 }), "{ev:?}");
        assert_eq!(s.stalkers[0].dist, 2);
        assert_eq!(s.stalkers[0].leave, 0.0);
        // week 1: 8 s at an open door
        let mut o = sim(1, 6);
        quiet(&mut o);
        o.stalkers[0].dist = 0;
        let ev = run(&mut o, 7_900.0, DayInput::default());
        assert!(ev.is_empty(), "{ev:?}");
        let ev = run(&mut o, 200.0, DayInput::default());
        assert!(ev.contains(&DayEvent::Caught { by: "ravelle".into() }));
        assert_eq!(o.over, Over::Caught);
        assert_eq!(o.outcome(), Outcome::Lost);
        // a light at the door pauses the count
        let mut l = sim(1, 6);
        quiet(&mut l);
        l.stalkers[0].dist = 0;
        run(&mut l, 20_000.0, DayInput { light_l: true, ..DayInput::default() });
        assert_eq!(l.over, Over::None);
        assert_eq!(l.stalkers[0].at_ms, 0.0);
    }

    #[test]
    fn a_door_visitor_turned_away_warns_the_other_when_both_are_keen() {
        let mut s = sim(5, 8); // ravelle 15, ceryth 14
        quiet(&mut s);
        s.stalkers[0].dist = 0;
        s.stalkers[1].dist = 3;
        let shut = DayInput { door_l: true, ..DayInput::default() };
        let ev = run(&mut s, 500.0, shut);
        assert!(ev.contains(&DayEvent::Repelled { idx: 0 }));
        assert!(ev.contains(&DayEvent::Warned { idx: 1 }), "{ev:?}");
        assert!((s.stalkers[1].pacer.t - (500.0 + 2200.0)).abs() < 1e-9, "{}", s.stalkers[1].pacer.t);
        // week 1: Ceryth is not on, so nobody is warned
        let mut w = sim(1, 8);
        quiet(&mut w);
        w.stalkers[0].dist = 0;
        w.stalkers[1].dist = 3;
        let ev = run(&mut w, 500.0, shut);
        assert!(ev.contains(&DayEvent::Repelled { idx: 0 }));
        assert!(!ev.iter().any(|e| matches!(e, DayEvent::Warned { .. })));
        // week 3: Ceryth is keen (8) but Ravelle too (10) -> both >= 6 -> warned; week 2 Ravelle 7 Ceryth 5 -> not
        let mut w2 = sim(2, 8);
        quiet(&mut w2);
        w2.stalkers[0].dist = 0;
        w2.stalkers[1].dist = 3;
        let ev = run(&mut w2, 500.0, shut);
        assert!(!ev.iter().any(|e| matches!(e, DayEvent::Warned { .. })), "{ev:?}");
    }

    #[test]
    fn ysolde_knocks_shows_at_the_window_and_leaves() {
        let mut s = sim(3, 12); // ysolde level 6
        quiet(&mut s);
        for h in s.stalkers.iter_mut() {
            h.dist = 99; // keep the door visitors out of it
        }
        s.ysolde.gap = 100.0;
        let ev = run(&mut s, 200.0, DayInput::default());
        assert!(ev.contains(&DayEvent::Knock), "{ev:?}");
        assert_eq!(s.ysolde.state, YState::Knock);
        assert!(s.ysolde.knock >= 3000.0 && s.ysolde.knock <= 4200.0 + 1e-9, "{}", s.ysolde.knock);
        let mut t = 0.0;
        while s.ysolde.state == YState::Knock {
            run(&mut s, 100.0, DayInput::default());
            t += 100.0;
            assert!(t < 5000.0);
        }
        assert_eq!(s.ysolde.state, YState::Here);
        assert!(t >= s.ysolde.knock - 100.0 && t <= s.ysolde.knock + 100.0);
        assert!(s.ysolde.stay >= 4800.0 && s.ysolde.stay <= 7200.0);
        let stay = s.ysolde.stay;
        let ev = run(&mut s, stay + 100.0, DayInput::default());
        assert!(ev.contains(&DayEvent::Gone), "{ev:?}");
        assert_eq!(s.ysolde.state, YState::Away);
        assert_eq!(s.over, Over::None);
    }

    #[test]
    fn ysolde_hears_a_light_or_salvaging_for_one_and_a_half_seconds() {
        assert!(ysolde_hears(&Dev { light_l: true, ..Dev::default() }, -1));
        assert!(ysolde_hears(&Dev::default(), 2));
        assert!(!ysolde_hears(&Dev { door_l: true, door_r: true, ..Dev::default() }, -1));
        let mut s = sim(3, 12);
        quiet(&mut s);
        for h in s.stalkers.iter_mut() {
            h.dist = 99;
        }
        s.place_ysolde("window");
        let lit = DayInput { light_r: true, ..DayInput::default() };
        let ev = run(&mut s, 1_400.0, lit);
        assert!(ev.is_empty(), "{ev:?}");
        let ev = run(&mut s, 200.0, lit);
        assert!(ev.contains(&DayEvent::Caught { by: "ysolde".into() }), "{ev:?}");
        // quiet and the doors shut: she leaves; and the meter drains while she is quiet
        let mut q = sim(3, 12);
        quiet(&mut q);
        for h in q.stalkers.iter_mut() {
            h.dist = 99;
        }
        q.place_ysolde("window");
        q.ysolde.expose = 1_000.0;
        run(&mut q, 500.0, DayInput { door_l: true, door_r: true, ..DayInput::default() });
        assert!((q.ysolde.expose - 500.0).abs() < 1e-9);
        // she is not on in week 1
        let mut w = sim(1, 12);
        quiet(&mut w);
        w.place_ysolde("window");
        run(&mut w, 3_000.0, DayInput { light_l: true, ..DayInput::default() });
        assert_ne!(w.by, "ysolde");
    }

    #[test]
    fn six_am_brings_the_quota_in() {
        let mut s = sim(1, 1);
        quiet(&mut s);
        for h in s.stalkers.iter_mut() {
            h.dist = 99;
        }
        s.elapsed = NIGHT_MS - 50.0;
        s.money = s.need;
        let ev = s.step(100.0, &DayInput::default());
        assert!(ev.contains(&DayEvent::Win { money: 140, quota: 140 }));
        assert_eq!(s.over, Over::Win);
        assert_eq!(s.outcome(), Outcome::Won);
        assert!(s.step(100.0, &DayInput::default()).is_empty());
        let mut m = sim(1, 1);
        quiet(&mut m);
        for h in m.stalkers.iter_mut() {
            h.dist = 99;
        }
        m.elapsed = NIGHT_MS - 50.0;
        m.money = m.need - 1;
        let ev = m.step(100.0, &DayInput::default());
        assert!(ev.contains(&DayEvent::Short { money: 139, quota: 140 }));
        assert_eq!(m.outcome(), Outcome::Lost);
    }

    #[test]
    fn a_custom_night_uses_its_sliders_and_the_flat_door_time() {
        let mut lv = BTreeMap::new();
        lv.insert("ravelle".to_string(), 0);
        lv.insert("ceryth".to_string(), 12);
        lv.insert("ysolde".to_string(), 3);
        let s = DayshiftSim::new(4, Mulberry32::new(2), DayOpts { levels: lv, custom: true, ..DayOpts::default() });
        assert_eq!(s.base_level("ravelle"), 0);
        assert_eq!(s.base_level("ceryth"), 12);
        assert_eq!(s.door_ms, 5200.0);
        assert_eq!(s.need, 340);
        assert_eq!(s.active(), vec!["ceryth", "ysolde"]);
        assert!(s.lead.is_none()); // Ravelle is off, so nobody leads
    }

    #[test]
    fn no_input_in_week_one_gets_caught_by_ravelle() {
        let mut s = sim(1, 21);
        let ev = run(&mut s, 120_000.0, DayInput::default());
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "ravelle");
        assert!(ev.iter().filter(|e| matches!(e, DayEvent::Close { .. })).count() == 3, "{ev:?}");
        assert!(s.elapsed > 35_000.0 && s.elapsed < 60_000.0, "{}", s.elapsed);
    }

    #[test]
    fn the_same_seed_gives_the_same_shift() {
        let play = |week: i32, seed: u32, bot: u32| {
            let mut s = sim(week, seed);
            let mut a = Autoplay::new(bot);
            let mut names = Vec::new();
            while s.over == Over::None {
                let inp = a.input(&s, 100.0);
                for e in s.step(100.0, &inp) {
                    names.push(e.name());
                }
            }
            (s.digest(), names, s.over)
        };
        for week in 1..=5 {
            assert_eq!(play(week, 77, 5), play(week, 77, 5));
        }
        assert_ne!(play(3, 77, 5).0, play(3, 78, 5).0);
    }

    #[test]
    fn every_week_the_bot_reaches_an_outcome() {
        for week in 1..=5 {
            for seed in [1u32, 2, 3] {
                let mut s = DayshiftSim::new(week, Mulberry32::new(seed), DayOpts { upgrades: bot_basket(week, 0), ..DayOpts::default() });
                let mut a = Autoplay::new(seed);
                let mut steps = 0;
                while s.over == Over::None && steps < 4000 {
                    let inp = a.input(&s, 100.0);
                    s.step(100.0, &inp);
                    steps += 1;
                }
                assert_ne!(s.over, Over::None, "week {week} seed {seed}");
                assert!(s.elapsed <= NIGHT_MS + 100.0);
            }
        }
    }

    #[test]
    fn god_rescue_and_admin_placement() {
        let mut s = sim(3, 3);
        s.over = Over::Caught;
        s.by = "ravelle".into();
        s.out = true;
        s.power = 0.0;
        s.god_rescue();
        assert_eq!(s.over, Over::None);
        assert!(!s.out && s.power >= 20.0);
        assert_eq!(s.stalkers[0].dist, 3);
        s.place_visitor(1, "near");
        assert_eq!(s.stalkers[1].dist, 1);
        s.place_visitor(1, "door");
        assert_eq!(s.stalkers[1].dist, 0);
        s.place_ysolde("near");
        assert_eq!(s.ysolde.state, YState::Knock);
        s.place_ysolde("far");
        assert_eq!(s.ysolde.state, YState::Away);
        s.set_hour(4);
        assert_eq!(hour_index(s.elapsed), 4);
        assert!(s.add_piece("prize"));
        assert_eq!(s.items.len(), 1);
    }
}
