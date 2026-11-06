//! Noise EVENTS a hunter or bot can hear: a plunge splash, a hard landing, a wading splash, a pipe burst, a valve turn
//! and a sluice gate opening. Each kind has a hearing radius, a loudness and a response (investigate the point, or
//! search the area) in `data/hunters/noise_events.json`. `heard` is pure: events and a listener in, what to do out.
//!
//! Soaked footsteps are not an event. The runner's own noise is already multiplied by the soaked factor
//! (`game/hunting.rs`, `SwimState::noise_scale`) and the hunter's hearing meter reads that, so an event for it would
//! count the same steps twice.
//!
//! Only the host's hunters react (`react`; a guest runs no hunter simulation) and a reaction only sets a hunter to
//! investigate or search. It never chases, so no event feeds the danger readout, the heartbeat or the chase cue.

use std::sync::OnceLock;

use serde_json::Value;

use super::brain::{Brain, State};
use crate::world::Facility;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    PlungeSplash,
    HardLanding,
    WadingSplash,
    PipeBurst,
    ValveTurn,
    SluiceOpen,
    /// A hunter landing from a catwalk jump (heard by the bots and the runner, never by the other hunters).
    HunterLanding,
    /// A relay recovered by the runner or a bot, where the one who recovered it stands (`Game::claim_relay`).
    RelayRecover,
}

impl NoiseKind {
    pub const ALL: [NoiseKind; 8] = [
        NoiseKind::PlungeSplash,
        NoiseKind::HardLanding,
        NoiseKind::WadingSplash,
        NoiseKind::PipeBurst,
        NoiseKind::ValveTurn,
        NoiseKind::SluiceOpen,
        NoiseKind::HunterLanding,
        NoiseKind::RelayRecover,
    ];

    /// The key in the data file.
    pub fn id(self) -> &'static str {
        match self {
            NoiseKind::PlungeSplash => "plunge_splash",
            NoiseKind::HardLanding => "hard_landing",
            NoiseKind::WadingSplash => "wading_splash",
            NoiseKind::PipeBurst => "pipe_burst",
            NoiseKind::ValveTurn => "valve_turn",
            NoiseKind::SluiceOpen => "sluice_open",
            NoiseKind::HunterLanding => "hunter_landing",
            NoiseKind::RelayRecover => "relay_recover",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// What a listener does about a sound it heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    /// Walk to the point.
    Investigate,
    /// Start a search from the point.
    Search,
}

/// One noise at a place on a level. `loudness` multiplies the kind's radius (1 = the data value).
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseEvent {
    pub kind: NoiseKind,
    pub pos: (f32, f32),
    pub loudness: f32,
    /// The infinite level it happened on; only listeners on that level hear it.
    pub level: i32,
}

impl NoiseEvent {
    /// An event at the kind's data loudness.
    pub fn new(kind: NoiseKind, pos: (f32, f32), level: i32) -> NoiseEvent {
        NoiseEvent { kind, pos, loudness: rules().kind(kind).loudness, level }
    }

    pub fn with_loudness(mut self, loudness: f32) -> NoiseEvent {
        self.loudness = loudness;
        self
    }
}

/// What a listener heard: where, and what to do about it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Investigate {
    pub at: (f32, f32),
    pub response: Response,
    pub search_seconds: f32,
    pub kind: NoiseKind,
    /// The level the sound happened on (a listener one storey away walks there through the nearest flight).
    pub level: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindRule {
    pub radius: f32,
    pub loudness: f32,
    pub response: Response,
    pub search_seconds: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub max_radius: f32,
    pub flooded_depth: f32,
    pub water_rebuild_seconds: f32,
    pub bot_hearing_scale: f32,
    pub bot_look_seconds: f32,
    pub hunter_wade_min_depth: f32,
    pub hunter_wade_splash_every: f32,
    /// Scales the runner's heard step volume for the thud of a hunter landing from a catwalk jump.
    pub landing_thud_volume_scale: f32,
    kinds: [KindRule; 8],
}

fn positive(v: &Value, key: &str) -> Result<f32, String> {
    match v[key].as_f64() {
        Some(f) if f.is_finite() && f > 0.0 => Ok(f as f32),
        _ => Err(format!("noise_events.json: `{key}` must be a positive number")),
    }
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut kinds: Vec<KindRule> = Vec::new();
        for k in NoiseKind::ALL {
            let e = &v["kinds"][k.id()];
            let response = match e["response"].as_str() {
                Some("investigate") => Response::Investigate,
                Some("search") => Response::Search,
                _ => return Err(format!("noise_events.json: `{}` needs a response of investigate or search", k.id())),
            };
            let search_seconds = e["search_seconds"]
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.0)
                .ok_or_else(|| format!("noise_events.json: `{}` needs search_seconds", k.id()))?;
            kinds.push(KindRule {
                radius: positive(e, "radius")?,
                loudness: positive(e, "loudness")?,
                response,
                search_seconds: search_seconds as f32,
            });
        }
        Ok(Rules {
            max_radius: positive(&v, "max_radius")?,
            flooded_depth: positive(&v, "flooded_depth")?,
            water_rebuild_seconds: positive(&v, "water_rebuild_seconds")?,
            bot_hearing_scale: positive(&v, "bot_hearing_scale")?,
            bot_look_seconds: positive(&v, "bot_look_seconds")?,
            hunter_wade_min_depth: positive(&v, "hunter_wade_min_depth")?,
            hunter_wade_splash_every: positive(&v, "hunter_wade_splash_every")?,
            landing_thud_volume_scale: positive(&v, "landing_thud_volume_scale")?,
            kinds: [kinds[0], kinds[1], kinds[2], kinds[3], kinds[4], kinds[5], kinds[6], kinds[7]],
        })
    }

    pub fn kind(&self, k: NoiseKind) -> &KindRule {
        &self.kinds[k.index()]
    }

    /// How far `e` carries to a listener with `hearing_scale`: the kind's radius times the event's loudness, capped at
    /// `max_radius` (never louder than a sprint), times the listener's hearing.
    pub fn radius_of(&self, e: &NoiseEvent, hearing_scale: f32) -> f32 {
        let r = self.kind(e.kind).radius * e.loudness.max(0.0);
        r.min(self.max_radius) * hearing_scale.max(0.0)
    }
}

/// The embedded rules (`data/hunters/noise_events.json`).
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        Rules::parse(include_str!("../data/hunters/noise_events.json")).expect("data/hunters/noise_events.json parses")
    })
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// `heard` with explicit rules, so a test can vary the data.
pub fn heard_with(rules: &Rules, events: &[NoiseEvent], listener: (f32, f32), hearing_scale: f32) -> Option<Investigate> {
    let mut best: Option<(f32, &NoiseEvent)> = None;
    for e in events {
        let reach = rules.radius_of(e, hearing_scale);
        let margin = reach - dist(e.pos, listener);
        if margin >= 0.0 && best.map_or(true, |(m, _)| margin > m) {
            best = Some((margin, e));
        }
    }
    best.map(|(_, e)| {
        let rule = rules.kind(e.kind);
        Investigate { at: e.pos, response: rule.response, search_seconds: rule.search_seconds, kind: e.kind, level: e.level }
    })
}

/// `heard_with` for a listener standing on `level`: an event on the same level carries at its radius, one a single
/// storey away at `adjacent_scale` times it (sound passes the slab, so no 3D check; `flight_edges::rules().hear_adjacent_scale`).
/// An event further away, or an adjacent one when `adjacent_scale` is not positive, is not heard.
pub fn heard_levels_with(
    rules: &Rules,
    events: &[NoiseEvent],
    listener: (f32, f32),
    hearing_scale: f32,
    level: i32,
    adjacent_scale: f32,
) -> Option<Investigate> {
    let mut best: Option<(f32, &NoiseEvent)> = None;
    for e in events {
        let scale = match (e.level - level).abs() {
            0 => 1.0,
            1 if adjacent_scale > 0.0 => adjacent_scale,
            _ => continue,
        };
        let margin = rules.radius_of(e, hearing_scale * scale) - dist(e.pos, listener);
        if margin >= 0.0 && best.map_or(true, |(m, _)| margin > m) {
            best = Some((margin, e));
        }
    }
    best.map(|(_, e)| {
        let rule = rules.kind(e.kind);
        Investigate { at: e.pos, response: rule.response, search_seconds: rule.search_seconds, kind: e.kind, level: e.level }
    })
}

/// The event a listener at `listener` with `hearing_scale` reacts to, if any: of the events in range, the one it hears
/// by the widest margin. Pure.
pub fn heard(events: &[NoiseEvent], listener: (f32, f32), hearing_scale: f32) -> Option<Investigate> {
    heard_with(rules(), events, listener, hearing_scale)
}

/// The host's hunters react to this frame's events (`hearing_mult` is the difficulty's `hunter_hearing`). A hunter that
/// is inactive, stunned, chasing or peeking is left alone; the others investigate or search from the sound. Hunters on
/// `level` hear events on `level` at the normal radius and events one storey away at `hear_adjacent_scale` times it
/// (data/hunters/flights.json); an adjacent-level sound sends the hunter to that floor through the nearest flight
/// (`investigate_on`), and the arrival logic then searches. Returns how many hunters reacted. Never chases and never
/// touches the player's meters.
pub fn react(hunters: &mut [Brain], events: &[NoiseEvent], level: i32, hearing_mult: f32, fac: &Facility) -> usize {
    let adjacent = super::flight_edges::rules().hear_adjacent_scale;
    let here: Vec<NoiseEvent> = events
        .iter()
        .filter(|e| e.level == level || ((e.level - level).abs() == 1 && adjacent > 0.0))
        .cloned()
        .collect();
    if here.is_empty() {
        return 0;
    }
    let mut n = 0;
    for h in hunters.iter_mut() {
        if !h.active || h.stun > 0.0 || matches!(h.state, State::Chase | State::Peek) {
            continue;
        }
        let Some(ear) = heard_levels_with(rules(), &here, h.pos, h.hearing_scale * hearing_mult, level, adjacent) else {
            continue;
        };
        if ear.level == level {
            match ear.response {
                Response::Investigate => h.investigate(ear.at),
                Response::Search => h.enter_search_from(ear.at, ear.search_seconds, fac),
            }
        } else {
            // Another floor: walk there first; on arrival the brain searches the spot. The secret wolf spiders never leave theirs.
            if super::secret_creatures::stays_home(&h.def.id) {
                continue;
            }
            h.investigate_on(ear.at, ear.level);
        }
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn ev(kind: NoiseKind, x: f32) -> NoiseEvent {
        NoiseEvent::new(kind, (x, 0.0), 0)
    }

    fn edited(f: impl Fn(&mut Value)) -> Rules {
        let mut v: Value = serde_json::from_str(include_str!("../data/hunters/noise_events.json")).unwrap();
        f(&mut v);
        Rules::parse(&v.to_string()).expect("still valid")
    }

    #[test]
    fn the_embedded_file_parses_and_a_bad_one_is_refused() {
        assert_eq!(rules().kind(NoiseKind::PlungeSplash).response, Response::Search);
        assert!(Rules::parse("{}").is_err());
        let text = include_str!("../data/hunters/noise_events.json");
        assert!(Rules::parse(&text.replace("\"investigate\"", "\"flee\"")).is_err());
    }

    #[test]
    fn standard_ranges_are_modest_and_never_louder_than_a_sprint() {
        let r = rules();
        assert!((r.max_radius - crate::config::NOISE_SPRINT).abs() < 1e-6);
        for k in NoiseKind::ALL {
            let e = NoiseEvent::new(k, (0.0, 0.0), 0);
            assert!(r.radius_of(&e, 1.0) > 0.0);
            assert!(r.radius_of(&e, 1.0) <= crate::config::NOISE_SPRINT + 1e-6, "{} is louder than a sprint", k.id());
            // a shout of loudness 50 is still capped
            assert!(r.radius_of(&e.clone().with_loudness(50.0), 1.0) <= crate::config::NOISE_SPRINT + 1e-6);
        }
    }

    #[test]
    fn soaked_footsteps_are_no_event() {
        assert!(NoiseKind::ALL.iter().all(|k| !k.id().contains("soaked") && !k.id().contains("footstep")));
    }

    #[test]
    fn a_splash_inside_the_radius_is_heard_and_outside_it_is_not() {
        let r = rules().radius_of(&ev(NoiseKind::PlungeSplash, 0.0), 1.0);
        let e = [ev(NoiseKind::PlungeSplash, 0.0)];
        let inside = heard(&e, (r - 0.5, 0.0), 1.0).expect("heard inside");
        assert_eq!(inside.at, (0.0, 0.0));
        assert_eq!(inside.response, Response::Search);
        assert!(heard(&e, (r + 0.5, 0.0), 1.0).is_none());
    }

    #[test]
    fn a_keener_listener_hears_further() {
        let e = [ev(NoiseKind::ValveTurn, 0.0)];
        let r = rules().radius_of(&e[0], 1.0);
        assert!(heard(&e, (r + 1.0, 0.0), 1.0).is_none());
        assert!(heard(&e, (r + 1.0, 0.0), 2.0).is_some());
        assert!(heard(&e, (1.0, 0.0), 0.0).is_none(), "a deaf listener hears nothing");
    }

    #[test]
    fn the_widest_margin_wins_and_data_changes_the_radius() {
        let events = [ev(NoiseKind::WadingSplash, 3.0), ev(NoiseKind::PipeBurst, -3.0)];
        let got = heard(&events, (0.0, 0.0), 1.0).unwrap();
        assert_eq!(got.kind, NoiseKind::PipeBurst);
        let quiet = edited(|v| v["kinds"]["valve_turn"]["radius"] = Value::from(2.0));
        let loud = edited(|v| v["kinds"]["valve_turn"]["radius"] = Value::from(9.0));
        let e = [NoiseEvent { kind: NoiseKind::ValveTurn, pos: (0.0, 0.0), loudness: 1.0, level: 0 }];
        assert!(heard_with(&quiet, &e, (5.0, 0.0), 1.0).is_none());
        assert!(heard_with(&loud, &e, (5.0, 0.0), 1.0).is_some());
        let search = edited(|v| v["kinds"]["valve_turn"]["response"] = Value::from("search"));
        assert_eq!(heard_with(&search, &e, (1.0, 0.0), 1.0).unwrap().response, Response::Search);
    }

    fn game_with_hunter(at: (f32, f32)) -> crate::game::Game {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = crate::game::Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.hunters.clear();
        let mut h = Brain::new(content.hunters[0].clone(), at, "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        g.hunters.push(h);
        g
    }

    #[test]
    fn a_plunge_splash_in_range_sends_a_hunter_to_search_the_point_and_out_of_range_does_not() {
        let mut g = game_with_hunter((5.5, 5.5));
        let near = NoiseEvent::new(NoiseKind::PlungeSplash, (6.5, 5.5), 0);
        assert_eq!(react(&mut g.hunters, &[near.clone()], 0, 1.0, &g.fac), 1);
        assert_eq!(g.hunters[0].state, State::Search);
        assert_eq!(g.hunters[0].last_known, Some(near.pos));

        let mut g = game_with_hunter((5.5, 5.5));
        let far = NoiseEvent::new(NoiseKind::PlungeSplash, (5.5 + 200.0, 5.5), 0);
        assert_eq!(react(&mut g.hunters, &[far], 0, 1.0, &g.fac), 0);
        assert_eq!(g.hunters[0].state, State::Wander);
    }

    #[test]
    fn a_wading_splash_makes_a_hunter_investigate_and_another_level_is_deaf() {
        let mut g = game_with_hunter((5.5, 5.5));
        let e = NoiseEvent::new(NoiseKind::WadingSplash, (6.5, 5.5), 0);
        assert_eq!(react(&mut g.hunters, &[e.clone()], 2, 1.0, &g.fac), 0, "an event two storeys away is not heard");
        assert_eq!(react(&mut g.hunters, &[e], 0, 1.0, &g.fac), 1);
        assert_eq!(g.hunters[0].state, State::Investigate);
    }

    #[test]
    fn noise_on_the_other_floor_is_heard_at_the_adjacent_scale_and_walked_to() {
        let scale = super::super::flight_edges::rules().hear_adjacent_scale;
        assert!(scale > 0.0 && scale < 1.0, "the data sets a reduced adjacent hearing");
        let e = NoiseEvent::new(NoiseKind::WadingSplash, (5.5, 5.5), 1);
        let reach = rules().radius_of(&e, 1.0);
        // inside the reduced radius: heard, and the brain is sent to level 1
        let mut g = game_with_hunter((5.5 + reach * scale * 0.5, 5.5));
        assert_eq!(react(&mut g.hunters, &[e.clone()], 0, 1.0, &g.fac), 1);
        assert_eq!(g.hunters[0].state, State::Investigate);
        assert_eq!(g.hunters[0].last_known, Some(e.pos));
        assert_eq!(g.hunters[0].last_known_level, 1);
        // between the reduced and the full radius: the same sound on the same floor is heard, from the next floor not
        let d = reach * (scale + 1.0) * 0.5;
        let mut g = game_with_hunter((5.5 + d, 5.5));
        assert_eq!(react(&mut g.hunters, &[e.clone()], 0, 1.0, &g.fac), 0, "outside the adjacent radius");
        let same = NoiseEvent::new(NoiseKind::WadingSplash, (5.5, 5.5), 0);
        assert_eq!(react(&mut g.hunters, &[same], 0, 1.0, &g.fac), 1, "inside it on the same floor");
        // the adjacent scale is data: switched off, nothing carries between floors
        assert!(heard_levels_with(rules(), &[e], (5.5, 5.5), 1.0, 0, 0.0).is_none());
    }

    #[test]
    fn a_chasing_stunned_or_inactive_hunter_is_left_alone_and_nothing_here_chases() {
        for setup in 0..3 {
            let mut g = game_with_hunter((5.5, 5.5));
            match setup {
                0 => g.hunters[0].state = State::Chase,
                1 => g.hunters[0].stun = 2.0,
                _ => g.hunters[0].active = false,
            }
            let before = g.hunters[0].state;
            let e = NoiseEvent::new(NoiseKind::PipeBurst, (6.0, 5.5), 0);
            assert_eq!(react(&mut g.hunters, &[e], 0, 1.0, &g.fac), 0);
            assert_eq!(g.hunters[0].state, before);
        }
        // a reacting hunter never ends in Chase, so no chase cue or heartbeat comes from an event
        let mut g = game_with_hunter((5.5, 5.5));
        for k in NoiseKind::ALL {
            g.hunters[0].state = State::Idle;
            react(&mut g.hunters, &[NoiseEvent::new(k, (6.0, 5.5), 0)], 0, 1.0, &g.fac);
            assert_ne!(g.hunters[0].state, State::Chase, "{}", k.id());
        }
    }
}
