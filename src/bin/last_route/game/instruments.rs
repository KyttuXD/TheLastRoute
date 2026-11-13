//! Playable stage instruments (Godot `game/config/music_props.gd`, `route_interactable_3d.gd` "music"): interact on a
//! mic or an instrument plays its short cue and makes a noise burst the hunters hear, like the water splash. One play
//! per instrument per cooldown. The instruments, their prompts and the numbers are data/instruments.json.

/// The arcade machines and carnival booths: the same press, with a result that comes a moment later.
pub mod amusements;

use super::Game;
use crate::content::embedded;
use serde_json::Value;
use std::sync::OnceLock;

/// The file's path under data/ (the embedded file's name).
pub const PATH: &str = "instruments.json";
const SCHEMA: &str = "lr.instruments/1";

/// One playable prop type.
#[derive(Clone, Debug, PartialEq)]
pub struct Instrument {
    /// The sound: audio cue `music_<cue>`.
    pub cue: String,
    /// The prompt after the interact key (upper case).
    pub prompt: String,
    /// The runner's noise burst and how long it lasts.
    pub noise: f32,
    pub noise_seconds: f32,
}

/// The parsed data/instruments.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub cooldown_seconds: f32,
    pub noise_enabled: bool,
    pub reach: f32,
    pub aim_height: f32,
    kinds: Vec<(String, Instrument)>,
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as f32)
        .ok_or_else(|| format!("instruments.json: '{key}' missing or not a number of at least 0"))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("instruments.json: '{key}' missing or empty"))
}

pub fn parse(src: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("instruments.json: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("instruments.json: 'schema' must be {SCHEMA}"));
    }
    let mut kinds = vec![];
    for (ptype, k) in v["kinds"].as_object().ok_or("instruments.json: 'kinds' missing")? {
        kinds.push((
            ptype.clone(),
            Instrument { cue: text(k, "cue")?, prompt: text(k, "prompt")?, noise: num(k, "noise")?, noise_seconds: num(k, "noise_seconds")? },
        ));
    }
    Ok(Rules {
        cooldown_seconds: num(&v, "cooldown_seconds")?,
        noise_enabled: v["noise_enabled"].as_bool().unwrap_or(true),
        reach: num(&v, "reach")?,
        aim_height: num(&v, "aim_height")?,
        kinds,
    })
}

/// The rules compiled into the executable. A file that fails to parse is reported on stderr and nothing is playable.
pub fn rules() -> &'static Rules {
    static C: OnceLock<Rules> = OnceLock::new();
    C.get_or_init(|| {
        let src = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        parse(src).unwrap_or_else(|e| {
            eprintln!("{e}");
            Rules { cooldown_seconds: 0.0, noise_enabled: false, reach: 0.0, aim_height: 0.0, kinds: vec![] }
        })
    })
}

impl Rules {
    /// The instrument a prop type is, None for a prop that is dressing (Godot `MusicProps.is_playable`).
    pub fn of(&self, ptype: &str) -> Option<&Instrument> {
        self.kinds.iter().find(|(t, _)| t == ptype).map(|(_, k)| k)
    }
}

/// The interact prompt of an instrument prop type ("[E] TAP THE MIC"), empty for any other prop.
pub fn prompt(ptype: &str) -> String {
    match rules().of(ptype) {
        Some(k) => format!("[E] {}", k.prompt),
        None => String::new(),
    }
}

/// The instruments still cooling down: (prop id, the run time it was played at). A runner reaches one or two at a
/// time, so the list stays that short; it is pruned on every press.
#[derive(Clone, Debug, Default)]
pub struct Played {
    last: Vec<(String, f32)>,
    /// The arcade machines and booths in play (game/instruments/amusements.rs).
    rides: amusements::State,
}

impl Game {
    /// Interact on instrument prop `i`: its cue plays and the runner makes its noise burst (Godot `MusicProps.play`,
    /// `emit_noise_burst`). False for a prop that is no instrument, or one played less than the cooldown ago.
    pub(super) fn play_instrument(&mut self, i: usize) -> bool {
        let r = rules();
        let Some((id, kind)) = self.fac.props.get(i).and_then(|p| r.of(&p.ptype).map(|k| (p.id.clone(), k))) else {
            return false;
        };
        let now = self.time;
        self.played.last.retain(|(_, at)| now >= *at && now - *at < r.cooldown_seconds);
        if self.played.last.iter().any(|(p, _)| *p == id) {
            return false;
        }
        self.played.last.push((id, now));
        crate::audio::sound::music(&kind.cue, 0.0, 1.0);
        if r.noise_enabled && kind.noise > 0.0 {
            self.p.noise_burst = kind.noise;
            self.p.noise_burst_timer = kind.noise_seconds;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Focus;
    use crate::world::catalog::Catalog;

    fn prop(id: &str, ptype: &str, cell: crate::world::Cell, offset: (f32, f32)) -> crate::world::facility::Prop {
        crate::world::facility::Prop {
            id: id.to_string(),
            cell,
            ptype: ptype.to_string(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.4, 0.4),
            offset,
            door: None,
            sewer: None,
            crawl_under: false,
        }
    }

    /// A run with a mic stand 0.8 m in front of the runner (along +z) and a drum kit 0.8 m behind it, the runner looking
    /// at the mic, and no hunter awake.
    fn stage() -> (Game, usize, usize, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-STAGE-7", 0, &content, &catalog);
        g.ensure_hunters(&content);
        for h in g.hunters.iter_mut() {
            h.active = false;
        }
        let cell = crate::world::Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.p.pos = cell.center();
        g.fac.props.push(prop("TEST:MIC", "micStand", cell, (0.0, 0.8)));
        g.fac.props.push(prop("TEST:DRUMS", "drumKit", cell, (0.0, -0.8)));
        let mic = g.fac.props.len() - 2;
        g.p.yaw = std::f32::consts::PI;
        g.p.pitch = (rules().aim_height - g.p.eye().y).atan2(0.8);
        g.p.stun = 0.0;
        g.p.noise_burst = 0.0;
        g.p.noise_burst_timer = 0.0;
        (g, mic, mic + 1, content, catalog)
    }

    #[test]
    fn the_shipped_rules_parse_and_name_the_instruments_godot_names() {
        let src = embedded::data_under(PATH).next().expect("instruments.json is embedded").1;
        let r = parse(src).expect("instruments.json parses");
        assert_eq!(&r, rules());
        // music_props.gd PLAYABLE: type, cue, noise level and seconds
        for (ptype, cue, noise, seconds) in [
            ("micStand", "mic", 2.2, 1.4),
            ("podiumMic", "podium", 1.2, 1.0),
            ("drumKit", "drums", 2.6, 2.0),
            ("electricGuitarStand", "electric", 2.0, 1.6),
            ("acousticGuitarStand", "acoustic", 1.2, 1.2),
            ("bassGuitarStand", "bass", 1.8, 1.4),
            ("keyboardStand", "keys", 1.4, 1.2),
            ("uprightPianoMusic", "piano", 1.6, 1.6),
        ] {
            let k = r.of(ptype).unwrap_or_else(|| panic!("{ptype} is playable"));
            assert_eq!(k.cue, cue, "{ptype}");
            assert!((k.noise - noise).abs() < 1e-6 && (k.noise_seconds - seconds).abs() < 1e-6, "{ptype}");
            assert_eq!(k.prompt, k.prompt.to_uppercase(), "{ptype}");
        }
        assert_eq!(r.kinds.len(), 8);
        assert!((r.cooldown_seconds - 0.6).abs() < 1e-6, "Godot COOLDOWN_SECONDS");
        // dressing is not playable
        for other in ["deskMic", "ampStack", "musicStand", "stageMonitor", "officeChair"] {
            assert!(r.of(other).is_none(), "{other}");
            assert_eq!(prompt(other), "");
        }
        assert_eq!(prompt("micStand"), "[E] TAP THE MIC");
        assert!(parse("{}").is_err());
        assert!(parse(&src.replace("\"cooldown_seconds\": 0.6,", "")).is_err(), "the cooldown is required data");
    }

    #[test]
    fn an_instrument_in_reach_offers_its_prompt() {
        let (mut g, mic, _, _, _) = stage();
        assert_eq!(g.find_focus(), Some(Focus::Instrument(mic)));
        g.focus = Some(Focus::Instrument(mic));
        assert_eq!(g.focus_prompt().as_deref(), Some("[E] TAP THE MIC"));
        // out of reach it offers nothing
        g.fac.props[mic].offset = (0.0, rules().reach + 1.0);
        assert_ne!(g.find_focus(), Some(Focus::Instrument(mic)));
    }

    #[test]
    fn a_press_makes_the_noise_burst_of_the_instrument() {
        let (mut g, mic, drums, content, catalog) = stage();
        g.interact(Focus::Instrument(mic), &content, &catalog, &Sounds::default());
        let k = rules().of("micStand").unwrap();
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (k.noise, k.noise_seconds), "the hunters hear the mic");
        // another instrument has its own cooldown and its own noise
        g.interact(Focus::Instrument(drums), &content, &catalog, &Sounds::default());
        let d = rules().of("drumKit").unwrap();
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (d.noise, d.noise_seconds));
        // a prop that is no instrument plays nothing
        g.fac.props[mic].ptype = "musicStand".to_string();
        g.time += 10.0;
        assert!(!g.play_instrument(mic));
        assert!(!g.play_instrument(usize::MAX));
    }

    #[test]
    fn the_cooldown_holds_one_play_per_instrument() {
        let (mut g, mic, _, _, _) = stage();
        let cd = rules().cooldown_seconds;
        assert!(cd > 0.0);
        assert!(g.play_instrument(mic));
        // mashed: nothing plays and the noise is not armed again
        g.p.noise_burst = 0.0;
        g.p.noise_burst_timer = 0.0;
        g.time += cd * 0.5;
        assert!(!g.play_instrument(mic));
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (0.0, 0.0));
        // a refused press does not push the cooldown out
        g.time += cd * 0.6;
        assert!(g.play_instrument(mic));
        assert!(g.p.noise_burst > 0.0);
        assert!(g.played.last.len() <= 1, "the played list holds only what still cools down");
    }
}
