//! Hunters that interact with the map while they wander. A wandering hunter picks its next goal from the points of
//! interest the facility offers (room centres, lockers, relays, doors, vents and grates, the safe zone, lit spots and
//! noise spots), weighted by its personality, the region and the room type, never the same one twice in a row, and
//! it remembers the last few. On arrival it does an activity for a data duration (inspect a locker, look around,
//! sniff, check the room, listen, knock or push a shut door) and then pauses in a real Idle state.
//!
//! This file is rules and selection only (everything tunable is in data/hunters/activities.json). The brain
//! (hunters/brain.rs) walks to the goal and runs the activity clock.

use crate::config::HUNTER_RADIUS;
use crate::world::{Cell, Facility};
use std::collections::HashMap;
use std::sync::OnceLock;

pub const PATH: &str = "hunters/activities.json";

/// What kind of place a point of interest is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PoiKind {
    RoomCentre,
    Locker,
    Relay,
    Door,
    Vent,
    SafeZone,
    LitProp,
    NoiseSpot,
    /// A corridor cell with three or more open sides (hunters/idle_life.rs builds these near a hunter).
    Junction,
    /// A bed (data/sleep.json `bed.prop_rules`): the hunter walks to it and naps (Godot hunter_naps.gd).
    Bed,
}

impl PoiKind {
    pub const ALL: [PoiKind; 10] = [
        PoiKind::RoomCentre,
        PoiKind::Locker,
        PoiKind::Relay,
        PoiKind::Door,
        PoiKind::Vent,
        PoiKind::SafeZone,
        PoiKind::LitProp,
        PoiKind::NoiseSpot,
        PoiKind::Junction,
        PoiKind::Bed,
    ];

    pub fn id(self) -> &'static str {
        match self {
            PoiKind::RoomCentre => "room_centre",
            PoiKind::Locker => "locker",
            PoiKind::Relay => "relay",
            PoiKind::Door => "door",
            PoiKind::Vent => "vent",
            PoiKind::SafeZone => "safe_zone",
            PoiKind::LitProp => "lit_prop",
            PoiKind::NoiseSpot => "noise_spot",
            PoiKind::Junction => "junction",
            PoiKind::Bed => "bed",
        }
    }

    pub fn from_id(s: &str) -> Option<PoiKind> {
        PoiKind::ALL.into_iter().find(|k| k.id() == s)
    }
}

/// What a hunter does on arrival.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Activity {
    InspectLocker,
    LookAround,
    Sniff,
    CheckRoom,
    Listen,
    Knock,
    Push,
    /// Idle life (hunters/idle_life.rs). Appended last so the wire indices of the older ones stay.
    Fidget,
    Pace,
    CornerPeek,
    /// Asleep in a bed (hunters/naps.rs): the brain is not run except the wake rule. Appended last (wire index 10).
    Nap,
}

impl Activity {
    pub const ALL: [Activity; 11] = [
        Activity::InspectLocker,
        Activity::LookAround,
        Activity::Sniff,
        Activity::CheckRoom,
        Activity::Listen,
        Activity::Knock,
        Activity::Push,
        Activity::Fidget,
        Activity::Pace,
        Activity::CornerPeek,
        Activity::Nap,
    ];

    /// The wire / data name (upper case, as the other state names are).
    pub fn name(self) -> &'static str {
        match self {
            Activity::InspectLocker => "INSPECT_LOCKER",
            Activity::LookAround => "LOOK_AROUND",
            Activity::Sniff => "SNIFF",
            Activity::CheckRoom => "CHECK_ROOM",
            Activity::Listen => "LISTEN",
            Activity::Knock => "KNOCK",
            Activity::Push => "PUSH",
            Activity::Fidget => "FIDGET",
            Activity::Pace => "PACE",
            Activity::CornerPeek => "CORNER_PEEK",
            Activity::Nap => "NAP",
        }
    }

    pub fn from_name(s: &str) -> Option<Activity> {
        Activity::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(s))
    }

    /// The wire index (optional key `act`): position in `ALL`.
    pub fn index(self) -> u8 {
        Activity::ALL.iter().position(|a| *a == self).unwrap_or(0) as u8
    }

    pub fn from_index(i: u8) -> Option<Activity> {
        Activity::ALL.get(i as usize).copied()
    }
}

/// One place a wandering hunter may go.
#[derive(Clone, Debug, PartialEq)]
pub struct Poi {
    pub kind: PoiKind,
    /// Where the hunter stands (an open cell centre the body fits).
    pub stand: (f32, f32),
    /// What it looks at from there (the locker, the door, the vent); the stand itself for a room centre.
    pub focus: (f32, f32),
    pub room: Option<usize>,
    pub rtype: String,
    pub region: String,
    /// Index into `Facility::hiding` for a locker.
    pub locker: Option<usize>,
    /// The threshold cell of a room door.
    pub door: Option<Cell>,
    /// A bed's long axis (unit, along the mattress): the way a napper lies. (0, 0) for every other point.
    pub axis: (f32, f32),
}

impl Poi {
    /// The goal's identity for the "not twice in a row" memory: the cell it is stood in.
    pub fn id(&self) -> (i32, i32) {
        (self.stand.0.floor() as i32, self.stand.1.floor() as i32)
    }
}

#[derive(Clone, Debug)]
pub struct KindRule {
    pub weight: f32,
    pub activities: Vec<(Activity, f32)>,
}

/// data/hunters/activities.json.
#[derive(Clone, Debug)]
pub struct Rules {
    pub memory: usize,
    pub min_distance: f32,
    pub pause: (f32, f32),
    pub spawn_pause: (f32, f32),
    pub locker_front: f32,
    pub vent_types: Vec<String>,
    pub noise_types: Vec<String>,
    pub kinds: HashMap<PoiKind, KindRule>,
    pub durations: HashMap<Activity, (f32, f32)>,
    pub sweep_degrees: f32,
    pub sweeps: f32,
    pub lean_at: f32,
    pub push_at: f32,
    pub listen_hearing_scale: f32,
    pub sniff_creep_speed: f32,
    pub check_room_inset: f32,
    pub check_room_speed_scale: f32,
    /// A runner this close wakes a napping hunter (metres).
    pub nap_wake_distance: f32,
    /// A runner this loud (noise) wakes a napping hunter within `nap_wake_noise_distance`.
    pub nap_wake_noise: f32,
    pub nap_wake_noise_distance: f32,
    /// A torch in the napper's face (cone: the dot of the beam with the way to the hunter) wakes it within the distance.
    pub nap_wake_torch_distance: f32,
    pub nap_wake_torch_cone: f32,
    /// Seconds of stun after a startled wake (it flinches, then looks where the runner was).
    pub nap_startle_stun: f32,
    pub default_personality: String,
    pub by_tier: Vec<String>,
    pub by_hunter: HashMap<String, String>,
    pub personalities: HashMap<String, HashMap<PoiKind, f32>>,
    pub regions: HashMap<String, HashMap<PoiKind, f32>>,
    pub room_types: HashMap<String, f32>,
}

impl Default for Rules {
    /// What the brain falls back to when the data file is missing or broken: room centres only, a plain look round.
    fn default() -> Self {
        Rules {
            memory: 4,
            min_distance: 4.0,
            pause: (0.8, 2.4),
            spawn_pause: (0.6, 1.8),
            locker_front: 0.9,
            vent_types: vec![],
            noise_types: vec![],
            kinds: HashMap::from([(
                PoiKind::RoomCentre,
                KindRule { weight: 1.0, activities: vec![(Activity::LookAround, 1.0)] },
            )]),
            durations: HashMap::from([(Activity::LookAround, (2.0, 3.5))]),
            sweep_degrees: 100.0,
            sweeps: 1.5,
            lean_at: 0.5,
            push_at: 0.5,
            listen_hearing_scale: 1.5,
            sniff_creep_speed: 0.3,
            check_room_inset: 1.5,
            check_room_speed_scale: 0.7,
            nap_wake_distance: 3.0,
            nap_wake_noise: 3.0,
            nap_wake_noise_distance: 10.0,
            nap_wake_torch_distance: 7.0,
            nap_wake_torch_cone: 0.85,
            nap_startle_stun: 0.5,
            default_personality: "steady".into(),
            by_tier: vec![],
            by_hunter: HashMap::new(),
            personalities: HashMap::new(),
            regions: HashMap::new(),
            room_types: HashMap::new(),
        }
    }
}

fn pair(v: &serde_json::Value, d: (f32, f32)) -> (f32, f32) {
    let a = v.as_array();
    let lo = a.and_then(|a| a.first()).and_then(|x| x.as_f64()).map(|x| x as f32);
    let hi = a.and_then(|a| a.get(1)).and_then(|x| x.as_f64()).map(|x| x as f32);
    match (lo, hi) {
        (Some(l), Some(h)) if l >= 0.0 && h >= l => (l, h),
        _ => d,
    }
}

fn kind_map(v: &serde_json::Value) -> HashMap<PoiKind, f32> {
    let mut m = HashMap::new();
    if let Some(o) = v.as_object() {
        for (k, x) in o {
            if let (Some(kind), Some(w)) = (PoiKind::from_id(k), x.as_f64()) {
                m.insert(kind, w.max(0.0) as f32);
            }
        }
    }
    m
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

impl Rules {
    pub fn parse(v: &serde_json::Value) -> Rules {
        let d = Rules::default();
        let num = |key: &str, d: f32| v["params"][key].as_f64().map(|x| x as f32).filter(|x| x.is_finite()).unwrap_or(d);
        let mut kinds = HashMap::new();
        if let Some(o) = v["kinds"].as_object() {
            for (k, x) in o {
                let Some(kind) = PoiKind::from_id(k) else { continue };
                let weight = x["weight"].as_f64().unwrap_or(1.0).max(0.0) as f32;
                let mut activities: Vec<(Activity, f32)> = x["activities"]
                    .as_object()
                    .map(|a| {
                        a.iter()
                            .filter_map(|(n, w)| Some((Activity::from_name(n)?, w.as_f64()?.max(0.0) as f32)))
                            .collect()
                    })
                    .unwrap_or_default();
                if activities.is_empty() {
                    activities.push((Activity::LookAround, 1.0));
                }
                kinds.insert(kind, KindRule { weight, activities });
            }
        }
        if kinds.is_empty() {
            kinds = d.kinds.clone();
        }
        let mut durations = d.durations.clone();
        if let Some(o) = v["activities"].as_object() {
            for (n, x) in o {
                if let Some(a) = Activity::from_name(n) {
                    durations.insert(a, pair(&x["duration"], (2.0, 3.5)));
                }
            }
        }
        let nested = |key: &str| -> HashMap<String, HashMap<PoiKind, f32>> {
            v[key]
                .as_object()
                .map(|o| o.iter().map(|(k, x)| (k.clone(), kind_map(x))).collect())
                .unwrap_or_default()
        };
        Rules {
            memory: v["memory"].as_u64().map_or(d.memory, |m| (m as usize).max(1)),
            min_distance: v["min_distance"].as_f64().map_or(d.min_distance, |x| x.max(0.0) as f32),
            pause: pair(&v["pause"], d.pause),
            spawn_pause: pair(&v["spawn_pause"], d.spawn_pause),
            locker_front: v["locker_front"].as_f64().map_or(d.locker_front, |x| x.clamp(0.3, 2.0) as f32),
            vent_types: strings(&v["vent_types"]),
            noise_types: strings(&v["noise_types"]),
            kinds,
            durations,
            sweep_degrees: num("sweep_degrees", d.sweep_degrees),
            sweeps: num("sweeps", d.sweeps),
            lean_at: num("lean_at", d.lean_at).clamp(0.0, 1.0),
            push_at: num("push_at", d.push_at).clamp(0.0, 1.0),
            listen_hearing_scale: num("listen_hearing_scale", d.listen_hearing_scale).max(0.1),
            sniff_creep_speed: num("sniff_creep_speed", d.sniff_creep_speed).max(0.0),
            check_room_inset: num("check_room_inset", d.check_room_inset).max(0.0),
            check_room_speed_scale: num("check_room_speed_scale", d.check_room_speed_scale).max(0.1),
            nap_wake_distance: num("nap_wake_distance", d.nap_wake_distance).max(0.0),
            nap_wake_noise: num("nap_wake_noise", d.nap_wake_noise).max(0.0),
            nap_wake_noise_distance: num("nap_wake_noise_distance", d.nap_wake_noise_distance).max(0.0),
            nap_wake_torch_distance: num("nap_wake_torch_distance", d.nap_wake_torch_distance).max(0.0),
            nap_wake_torch_cone: num("nap_wake_torch_cone", d.nap_wake_torch_cone),
            nap_startle_stun: num("nap_startle_stun", d.nap_startle_stun).max(0.0),
            default_personality: v["default_personality"].as_str().map_or(d.default_personality, String::from),
            by_tier: strings(&v["by_tier"]),
            by_hunter: v["by_hunter"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect())
                .unwrap_or_default(),
            personalities: nested("personalities"),
            regions: nested("regions"),
            room_types: v["room_types"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?.max(0.0) as f32))).collect())
                .unwrap_or_default(),
        }
    }

    /// The embedded rules, read once.
    pub fn get() -> &'static Rules {
        static RULES: OnceLock<Rules> = OnceLock::new();
        RULES.get_or_init(|| {
            crate::content::embedded::data_under(PATH)
                .next()
                .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
                .map(|v| Rules::parse(&v))
                .unwrap_or_default()
        })
    }

    /// The personality a hunter wanders with: its own entry, else the one for its tier, else the default.
    pub fn personality_of(&self, hunter_id: &str, tier: usize) -> &str {
        self.by_hunter
            .get(hunter_id)
            .or_else(|| self.by_tier.get(tier))
            .map_or(self.default_personality.as_str(), String::as_str)
    }

    /// kind weight x personality x region x room type; 0 for a kind the data does not list.
    pub fn weight(&self, poi: &Poi, personality: &str) -> f32 {
        let Some(k) = self.kinds.get(&poi.kind) else { return 0.0 };
        let p = self.personalities.get(personality).and_then(|m| m.get(&poi.kind)).copied().unwrap_or(1.0);
        let r = self.regions.get(&poi.region).and_then(|m| m.get(&poi.kind)).copied().unwrap_or(1.0);
        let t = self.room_types.get(&poi.rtype).copied().unwrap_or(1.0);
        (k.weight * p * r * t).max(0.0)
    }

    /// The next goal: a weighted draw (`roll` in 0..1) among the points at least `min_distance` away that are not in
    /// `recent` (cell ids, newest last). With none left, the memory is relaxed but the last goal is still never
    /// repeated; then the distance is relaxed too.
    pub fn choose(
        &self,
        pois: &[Poi],
        from: (f32, f32),
        recent: &[(i32, i32)],
        personality: &str,
        roll: f64,
    ) -> Option<usize> {
        let last = recent.last().copied();
        let dist = |p: &Poi| ((p.stand.0 - from.0).powi(2) + (p.stand.1 - from.1).powi(2)).sqrt();
        let tiers: [(f32, bool); 3] = [(self.min_distance, true), (self.min_distance, false), (1.5, false)];
        for (min_d, use_recent) in tiers {
            let cands: Vec<(usize, f32)> = pois
                .iter()
                .enumerate()
                .filter(|(_, p)| dist(p) >= min_d)
                .filter(|(_, p)| if use_recent { !recent.contains(&p.id()) } else { Some(p.id()) != last })
                .map(|(i, p)| (i, self.weight(p, personality)))
                .filter(|(_, w)| *w > 0.0)
                .collect();
            let total: f32 = cands.iter().map(|(_, w)| *w).sum();
            if cands.is_empty() || total <= 0.0 {
                continue;
            }
            let mut left = (roll.clamp(0.0, 0.999_999) as f32) * total;
            for (i, w) in &cands {
                if left < *w {
                    return Some(*i);
                }
                left -= *w;
            }
            return cands.last().map(|(i, _)| *i);
        }
        None
    }

    /// The activity done at `poi`: a weighted draw among the kind's list that can happen here (a locker needs a
    /// locker, a knock or a push needs a shut door).
    pub fn pick_activity(&self, poi: &Poi, door_shut: bool, roll: f64) -> Activity {
        self.pick_activity_scaled(poi, door_shut, roll, |_| 1.0)
    }

    /// `pick_activity` with each activity's weight scaled (the hunter's personality, hunters/idle_life.rs).
    pub fn pick_activity_scaled(&self, poi: &Poi, door_shut: bool, roll: f64, scale: impl Fn(Activity) -> f32) -> Activity {
        let Some(k) = self.kinds.get(&poi.kind) else { return Activity::LookAround };
        let ok = |a: Activity| match a {
            Activity::InspectLocker => poi.locker.is_some(),
            Activity::Knock | Activity::Push => poi.door.is_some() && door_shut,
            _ => true,
        };
        let list: Vec<(Activity, f32)> = k.activities.iter().map(|(a, w)| (*a, *w * scale(*a))).filter(|(a, w)| *w > 0.0 && ok(*a)).collect();
        let total: f32 = list.iter().map(|(_, w)| *w).sum();
        if list.is_empty() || total <= 0.0 {
            return Activity::LookAround;
        }
        let mut left = (roll.clamp(0.0, 0.999_999) as f32) * total;
        for (a, w) in &list {
            if left < *w {
                return *a;
            }
            left -= *w;
        }
        list[list.len() - 1].0
    }

    /// Seconds an activity runs, from its [min, max] and a roll in 0..1.
    pub fn duration(&self, a: Activity, roll: f64) -> f32 {
        let (lo, hi) = self.durations.get(&a).copied().unwrap_or((2.0, 3.5));
        lo + (hi - lo) * roll.clamp(0.0, 1.0) as f32
    }

    /// Seconds of the Idle pause after an activity, or `spawn` for the first one after spawning.
    pub fn pause_for(&self, spawn: bool, roll: f64) -> f32 {
        let (lo, hi) = if spawn { self.spawn_pause } else { self.pause };
        lo + (hi - lo) * roll.clamp(0.0, 1.0) as f32
    }
}

pub(super) fn region_of(fac: &Facility, p: (f32, f32), room: Option<usize>) -> String {
    if let Some(r) = room.and_then(|i| fac.rooms.get(i)) {
        if !r.region.is_empty() {
            return r.region.clone();
        }
    }
    let (cx, cy) = (p.0.floor() as i32, p.1.floor() as i32);
    if cx >= 0 && cy >= 0 && cx < fac.cols && cy < fac.rows {
        if let Some(&i) = fac.cell_region.get((cy * fac.cols + cx) as usize) {
            if let Some(n) = fac.region_names.get(i as usize) {
                return n.clone();
            }
        }
    }
    "maze".into()
}

/// Every point of interest the facility offers. Stands are open cells a hunter's body fits.
/// The way a napper lies on a bed: along its long axis (the larger half-extent, as game/bed.rs lie_down picks it), with
/// the head at the end away from the stand the hunter comes to, so the body lies on the far side of the mattress. The
/// data does not name the pillow end; this is the rule until it does.
fn nap_axis(p: &crate::world::facility::Prop, at: (f32, f32), stand: (f32, f32)) -> (f32, f32) {
    let base = if p.half.0 >= p.half.1 { (1.0, 0.0) } else { (0.0, 1.0) };
    let away = (at.0 - stand.0) * base.0 + (at.1 - stand.1) * base.1;
    if away >= 0.0 { base } else { (-base.0, -base.1) }
}

pub fn points_of_interest(fac: &Facility, rules: &Rules) -> Vec<Poi> {
    let mut out = Vec::new();
    let mut add = |kind: PoiKind, near: (f32, f32), focus: Option<(f32, f32)>, room: Option<usize>, locker: Option<usize>, door: Option<Cell>| {
        let Some(stand) = fac.nearest_open(near.0, near.1, HUNTER_RADIUS) else { return };
        let room = room.or_else(|| fac.room_at(near.0 as f64, near.1 as f64));
        out.push(Poi {
            kind,
            stand,
            focus: focus.unwrap_or(stand),
            room,
            rtype: room.and_then(|i| fac.rooms.get(i)).map(|r| r.rtype.clone()).unwrap_or_default(),
            region: region_of(fac, stand, room),
            locker,
            door,
            axis: (0.0, 0.0),
        });
    };
    let safe_room = fac.safe_zone.and_then(|c| fac.room_at(c.x as f64 + 0.5, c.y as f64 + 0.5));
    for (i, room) in fac.rooms.iter().enumerate() {
        if room.lobby() || Some(i) == safe_room {
            continue;
        }
        add(PoiKind::RoomCentre, room.center.center(), None, Some(i), None, None);
    }
    for (i, h) in fac.hiding.iter().enumerate() {
        let c = h.cell.center();
        let front = (c.0 + h.facing.0 as f32 * rules.locker_front, c.1 + h.facing.1 as f32 * rules.locker_front);
        add(PoiKind::Locker, front, Some(c), h.room, Some(i), None);
    }
    for c in &fac.relays {
        add(PoiKind::Relay, c.center(), Some(c.center()), None, None, None);
    }
    for (pi, centre) in fac.doors() {
        let cell = fac.props[pi].cell;
        add(PoiKind::Door, centre, Some(centre), None, None, Some(cell));
    }
    for p in &fac.props {
        let at = (p.cell.center().0 + p.offset.0, p.cell.center().1 + p.offset.1);
        if rules.vent_types.iter().any(|t| *t == p.ptype) {
            add(PoiKind::Vent, at, Some(at), Some(p.room), None, None);
        } else if rules.noise_types.iter().any(|t| *t == p.ptype) {
            add(PoiKind::NoiseSpot, at, Some(at), Some(p.room), None, None);
        }
    }
    if let Some(c) = fac.safe_zone {
        add(PoiKind::SafeZone, c.center(), Some(c.center()), safe_room, None, None);
    }
    for l in &fac.room_lights {
        add(PoiKind::LitProp, l.center.center(), Some(l.center.center()), Some(l.room), None, None);
    }
    // beds (hunters/naps.rs): the stand is the open cell nearest the bed; the nap looks at the mattress and lies along
    // the bed's long axis (the larger half-extent, as game/bed.rs lie_down picks it). Added after the others so the
    // order of every other point is unchanged.
    for p in &fac.props {
        if crate::game::bed::rules().kind_of(&p.ptype).is_none() {
            continue;
        }
        let at = (p.cell.center().0 + p.offset.0, p.cell.center().1 + p.offset.1);
        let Some(stand) = fac.nearest_open(at.0, at.1, HUNTER_RADIUS) else { continue };
        let room = Some(p.room);
        out.push(Poi {
            kind: PoiKind::Bed,
            stand,
            focus: at,
            room,
            rtype: room.and_then(|i| fac.rooms.get(i)).map(|r| r.rtype.clone()).unwrap_or_default(),
            region: region_of(fac, stand, room),
            locker: None,
            door: None,
            axis: nap_axis(p, at, stand),
        });
    }
    out
}

/// A short loop through the room: its four corners pulled in by `inset`, each the nearest open spot, starting at
/// corner `start` so the loops differ.
pub fn room_loop(fac: &Facility, room: usize, inset: f32, start: usize) -> Vec<(f32, f32)> {
    let Some(r) = fac.rooms.get(room) else { return vec![] };
    let inset = inset.min(r.w as f32 / 2.0 - 0.5).min(r.h as f32 / 2.0 - 0.5).max(0.0);
    let (x0, y0, x1, y1) = (r.x as f32 + inset, r.y as f32 + inset, (r.x + r.w) as f32 - inset, (r.y + r.h) as f32 - inset);
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    (0..4)
        .filter_map(|i| {
            let c = corners[(i + start) % 4];
            fac.nearest_open(c.0, c.1, HUNTER_RADIUS)
        })
        .collect()
}

/// Turn `base` (unit) by `angle` radians.
pub fn rotate(base: (f32, f32), angle: f32) -> (f32, f32) {
    let (s, c) = angle.sin_cos();
    (base.0 * c - base.1 * s, base.0 * s + base.1 * c)
}

/// The facing during a look-around: a sweep `sweeps` times across `degrees` (half each side of `base`), over progress 0..1.
pub fn sweep_facing(base: (f32, f32), progress: f32, degrees: f32, sweeps: f32) -> (f32, f32) {
    let a = (progress * sweeps * std::f32::consts::TAU).sin() * degrees.to_radians() / 2.0;
    rotate(base, a)
}

/// The unit vector from `a` to `b`, or `fallback` when they coincide.
pub fn dir_to(a: (f32, f32), b: (f32, f32), fallback: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = (dx * dx + dy * dy).sqrt();
    if l < 1e-3 { fallback } else { (dx / l, dy / l) }
}

/// An activity in progress. The renderer and the wire read `kind` and `progress()`.
#[derive(Clone, Debug)]
pub struct ActivityRun {
    pub kind: Activity,
    pub duration: f32,
    pub elapsed: f32,
    /// The facing the activity started with (the sweep is centred on it).
    pub base: (f32, f32),
    pub focus: (f32, f32),
    pub locker: Option<usize>,
    pub door: Option<Cell>,
    /// Whether the one-shot beat (lean in at the locker, push on the door) has happened.
    pub fired: bool,
    pub leg: usize,
    pub loop_points: Vec<(f32, f32)>,
    /// The hearing multiplier this run applied (undone when it ends); 1.0 when none.
    pub hearing_applied: f32,
    /// A nap's bed axis (see [`Poi::axis`]); (0, 0) for every other activity.
    pub axis: (f32, f32),
}

/// A hunter asleep in a bed: the mattress centre it lies on and the bed's long axis (for the lying pose).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NapSpot {
    pub at: (f32, f32),
    pub axis: (f32, f32),
}

impl ActivityRun {
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 { 1.0 } else { (self.elapsed / self.duration).clamp(0.0, 1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn fac() -> Facility {
        Facility::generate("ACT-1", &Options::default(), &Catalog::load())
    }

    fn poi(kind: PoiKind, x: f32) -> Poi {
        Poi {
            kind,
            stand: (x, 5.5),
            focus: (x, 5.5),
            room: None,
            rtype: String::new(),
            region: "maze".into(),
            locker: None,
            door: None,
            axis: (0.0, 0.0),
        }
    }

    #[test]
    fn a_napper_lies_with_its_head_at_the_end_of_the_bed_away_from_the_stand() {
        let bed = crate::world::facility::Prop {
            id: "TEST:BED".to_string(),
            cell: Cell::new(5, 5),
            ptype: "singleBed".to_string(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (1.0, 0.5),
            offset: (0.0, 0.0),
            door: None,
            sewer: None,
            crawl_under: false,
        };
        let centre = (5.5, 5.5);
        // a stand west of the bed: the head goes east, along +x
        assert_eq!(nap_axis(&bed, centre, (4.5, 5.5)), (1.0, 0.0));
        // a stand east of it: the head goes west
        assert_eq!(nap_axis(&bed, centre, (6.5, 5.5)), (-1.0, 0.0));
    }

    #[test]
    fn the_embedded_data_lists_every_kind_and_activity_with_a_duration() {
        let r = Rules::get();
        for k in PoiKind::ALL {
            let rule = r.kinds.get(&k).unwrap_or_else(|| panic!("kind {}", k.id()));
            assert!(rule.weight > 0.0 && !rule.activities.is_empty());
        }
        for a in Activity::ALL {
            let (lo, hi) = r.durations[&a];
            assert!(lo > 0.0 && hi >= lo, "{}", a.name());
        }
        assert!(r.pause.0 > 0.0 && r.spawn_pause.0 > 0.0);
        assert!(r.memory >= 1);
    }

    #[test]
    fn activity_names_and_wire_indices_round_trip() {
        for a in Activity::ALL {
            assert_eq!(Activity::from_name(a.name()), Some(a));
            assert_eq!(Activity::from_index(a.index()), Some(a));
        }
        assert_eq!(Activity::from_index(99), None);
    }

    #[test]
    fn the_facility_offers_room_centres_lockers_and_doors_as_stands_a_body_fits() {
        let f = fac();
        let pois = points_of_interest(&f, Rules::get());
        assert!(pois.iter().any(|p| p.kind == PoiKind::RoomCentre));
        assert!(pois.iter().any(|p| p.kind == PoiKind::Locker) == !f.hiding.is_empty());
        for p in &pois {
            assert!(f.body_fits(p.stand.0, p.stand.1, HUNTER_RADIUS), "{:?}", p.kind);
        }
        for p in pois.iter().filter(|p| p.kind == PoiKind::Locker) {
            let h = &f.hiding[p.locker.unwrap()];
            assert_eq!(p.focus, h.cell.center());
        }
    }

    #[test]
    fn goals_respect_the_weights_over_many_picks() {
        let mut r = Rules::default();
        r.min_distance = 0.0;
        r.kinds.insert(PoiKind::Locker, KindRule { weight: 3.0, activities: vec![(Activity::InspectLocker, 1.0)] });
        let pois = vec![poi(PoiKind::RoomCentre, 10.5), poi(PoiKind::Locker, 20.5)];
        let mut rng = crate::world::rng::Rng::new("WEIGHTS");
        let mut lockers = 0;
        for _ in 0..4000 {
            let i = r.choose(&pois, (0.5, 5.5), &[], "steady", rng.next_float()).unwrap();
            if pois[i].kind == PoiKind::Locker {
                lockers += 1;
            }
        }
        let share = lockers as f32 / 4000.0;
        assert!((share - 0.75).abs() < 0.05, "locker share {share}");
    }

    #[test]
    fn personality_and_region_scale_the_weights() {
        let mut r = Rules::default();
        r.kinds.insert(PoiKind::Locker, KindRule { weight: 1.0, activities: vec![(Activity::InspectLocker, 1.0)] });
        r.personalities.insert("nosy".into(), HashMap::from([(PoiKind::Locker, 4.0)]));
        r.regions.insert("sewer".into(), HashMap::from([(PoiKind::Locker, 0.5)]));
        let mut l = poi(PoiKind::Locker, 20.5);
        assert_eq!(r.weight(&l, "steady"), 1.0);
        assert_eq!(r.weight(&l, "nosy"), 4.0);
        l.region = "sewer".into();
        assert_eq!(r.weight(&l, "nosy"), 2.0);
        // a kind the data does not list is never chosen
        assert_eq!(r.weight(&poi(PoiKind::Vent, 3.5), "steady"), 0.0);
    }

    #[test]
    fn the_same_goal_is_never_picked_twice_in_a_row_and_recent_ones_are_avoided() {
        let mut r = Rules::default();
        r.min_distance = 0.0;
        let pois: Vec<Poi> = (0..5).map(|i| poi(PoiKind::RoomCentre, 5.5 + i as f32 * 4.0)).collect();
        let mut rng = crate::world::rng::Rng::new("RECENT");
        let mut recent: Vec<(i32, i32)> = vec![];
        let mut last = None;
        for _ in 0..200 {
            let i = r.choose(&pois, (0.5, 5.5), &recent, "steady", rng.next_float()).unwrap();
            assert_ne!(Some(i), last);
            assert!(!recent.contains(&pois[i].id()));
            last = Some(i);
            recent.push(pois[i].id());
            let excess = recent.len().saturating_sub(r.memory);
            recent.drain(..excess);
        }
        // with only two points the memory is relaxed but the last one is still skipped
        let two = &pois[..2];
        let i = r.choose(two, (0.5, 5.5), &[two[0].id(), two[1].id()], "steady", 0.1).unwrap();
        assert_eq!(i, 0);
    }

    #[test]
    fn a_far_enough_goal_is_preferred_and_a_lone_near_one_is_still_taken() {
        let mut r = Rules::default();
        r.min_distance = 5.0;
        let pois = vec![poi(PoiKind::RoomCentre, 2.5), poi(PoiKind::RoomCentre, 30.5)];
        for roll in [0.0, 0.5, 0.99] {
            assert_eq!(r.choose(&pois, (0.5, 5.5), &[], "steady", roll), Some(1));
        }
        assert_eq!(r.choose(&pois[..1], (0.5, 5.5), &[], "steady", 0.5), Some(0));
    }

    #[test]
    fn activities_that_cannot_happen_are_skipped() {
        let mut r = Rules::default();
        r.kinds.insert(
            PoiKind::Door,
            KindRule { weight: 1.0, activities: vec![(Activity::Knock, 1.0), (Activity::Listen, 1.0)] },
        );
        let mut d = poi(PoiKind::Door, 4.5);
        d.door = Some(Cell::new(4, 5));
        for roll in [0.0, 0.3, 0.7, 0.99] {
            assert_eq!(r.pick_activity(&d, false, roll), Activity::Listen);
        }
        assert_eq!(r.pick_activity(&d, true, 0.0), Activity::Knock);
        // no locker, no inspection
        let mut l = poi(PoiKind::Locker, 8.5);
        r.kinds.insert(PoiKind::Locker, KindRule { weight: 1.0, activities: vec![(Activity::InspectLocker, 1.0)] });
        assert_eq!(r.pick_activity(&l, true, 0.0), Activity::LookAround);
        l.locker = Some(0);
        assert_eq!(r.pick_activity(&l, true, 0.0), Activity::InspectLocker);
    }

    #[test]
    fn scaled_picks_follow_the_scale_and_the_new_activities_have_wire_indices_after_the_old() {
        let mut r = Rules::default();
        r.kinds.insert(
            PoiKind::RoomCentre,
            KindRule { weight: 1.0, activities: vec![(Activity::LookAround, 1.0), (Activity::Fidget, 1.0)] },
        );
        let p = poi(PoiKind::RoomCentre, 5.5);
        assert_eq!(r.pick_activity_scaled(&p, false, 0.9, |a| if a == Activity::Fidget { 0.0 } else { 1.0 }), Activity::LookAround);
        assert_eq!(r.pick_activity_scaled(&p, false, 0.1, |a| if a == Activity::LookAround { 0.0 } else { 1.0 }), Activity::Fidget);
        assert_eq!(Activity::Push.index(), 6);
        assert!(Activity::Fidget.index() > Activity::Push.index());
        assert_eq!(Activity::from_name("corner_peek"), Some(Activity::CornerPeek));
    }

    #[test]
    fn durations_and_pauses_stay_in_their_data_ranges() {
        let r = Rules::get();
        for a in Activity::ALL {
            let (lo, hi) = r.durations[&a];
            for roll in [0.0, 0.5, 1.0] {
                let d = r.duration(a, roll);
                assert!(d >= lo - 1e-4 && d <= hi + 1e-4);
            }
        }
        assert!(r.pause_for(false, 0.0) >= r.pause.0 && r.pause_for(false, 1.0) <= r.pause.1 + 1e-4);
        assert!(r.pause_for(true, 0.5) >= r.spawn_pause.0);
    }

    #[test]
    fn a_changed_data_value_changes_the_parsed_rules() {
        let r = Rules::parse(&serde_json::json!({
            "memory": 7, "pause": [3.0, 4.0],
            "kinds": { "vent": { "weight": 2.5, "activities": { "LISTEN": 1 } } },
            "activities": { "listen": { "duration": [9.0, 10.0] } },
            "params": { "sweep_degrees": 40.0 },
            "by_tier": ["a", "b", "c"], "by_hunter": { "aki": "z" }
        }));
        assert_eq!(r.memory, 7);
        assert_eq!(r.pause, (3.0, 4.0));
        assert_eq!(r.kinds[&PoiKind::Vent].weight, 2.5);
        assert_eq!(r.duration(Activity::Listen, 0.0), 9.0);
        assert_eq!(r.sweep_degrees, 40.0);
        assert_eq!(r.personality_of("aki", 0), "z");
        assert_eq!(r.personality_of("other", 2), "c");
        // broken input falls back to the defaults
        assert_eq!(Rules::parse(&serde_json::json!({})).memory, 4);
    }

    #[test]
    fn a_sweep_turns_both_ways_and_ends_where_it_started_at_whole_cycles() {
        let base = (0.0, 1.0);
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for i in 0..=100 {
            let f = sweep_facing(base, i as f32 / 100.0, 100.0, 1.0);
            let l = (f.0 * f.0 + f.1 * f.1).sqrt();
            assert!((l - 1.0).abs() < 1e-4);
            min = min.min(f.0);
            max = max.max(f.0);
        }
        assert!(min < -0.5 && max > 0.5);
        let end = sweep_facing(base, 1.0, 100.0, 1.0);
        assert!(end.0.abs() < 1e-3 && (end.1 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_room_loop_is_open_cells_inside_the_floor() {
        let f = fac();
        let i = f.rooms.iter().position(|r| !r.lobby()).unwrap();
        let pts = room_loop(&f, i, 1.5, 1);
        assert!(!pts.is_empty());
        for p in pts {
            assert!(f.body_fits(p.0, p.1, HUNTER_RADIUS));
        }
    }
}
