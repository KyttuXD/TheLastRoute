//! Arcade machines and carnival booths the runner can play (docs/PROPS_PASS.md): interact on a cabinet, a claw
//! machine, the high striker, the mole table, the duck pond and the other booths plays its cue at once and makes the
//! runner's noise burst, and a moment later the result comes: a win or a lose cue and a line of text. A few machines
//! pay a coin on a win, once per machine and a few times a run. Which prop types, their cues, chances and numbers are
//! data/amusements.json. It is the stage instruments' path (game/instruments.rs) with a delayed result; nothing is
//! sent to other players.
//!
//! The win roll is a hash of the prop's id and the press count, so no random stream of the run is touched.

use crate::content::embedded;
use crate::game::Game;
use serde_json::Value;
use std::sync::OnceLock;

/// The file's path under data/ (the embedded file's name).
pub const PATH: &str = "amusements.json";
const SCHEMA: &str = "lr.amusements/1";

/// What a play ends in: the cue and the message line, and for a win its chance and prize.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The full cue name (`carnival_bell`).
    pub cue: String,
    pub text: String,
    /// 0..1, read for a win only.
    pub chance: f32,
    /// Coins a win pays, at most once per machine.
    pub coins: i32,
}

/// One playable prop type.
#[derive(Clone, Debug, PartialEq)]
pub struct Amusement {
    /// The prompt after the interact key (upper case).
    pub prompt: String,
    /// The full name of the cue played on the press.
    pub press: String,
    pub result_seconds: f32,
    pub win: Option<Outcome>,
    pub lose: Option<Outcome>,
    /// The runner's noise burst and how long it lasts.
    pub noise: f32,
    pub noise_seconds: f32,
    pub cooldown_seconds: f32,
}

/// The parsed data/amusements.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub noise_enabled: bool,
    pub reach: f32,
    pub aim_height: f32,
    /// How many prizes a run pays in all.
    pub max_prizes: usize,
    kinds: Vec<(String, Amusement)>,
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as f32)
        .ok_or_else(|| format!("{PATH}: '{key}' missing or not a number of at least 0"))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{PATH}: '{key}' missing or empty"))
}

/// A cue name this file may play: the booth and instrument families, which have a play call.
fn cue(v: &Value, key: &str) -> Result<String, String> {
    let name = text(v, key)?;
    if name.starts_with("carnival_") || name.starts_with("music_") {
        Ok(name)
    } else {
        Err(format!("{PATH}: '{key}': {name} is not a carnival_ or music_ cue"))
    }
}

fn outcome(v: &Value, key: &str, win: bool) -> Result<Option<Outcome>, String> {
    let Some(o) = v.get(key).filter(|o| o.is_object()) else {
        return Ok(None);
    };
    let chance = if win { num(o, "chance")? } else { 1.0 };
    if chance > 1.0 {
        return Err(format!("{PATH}: '{key}.chance' must be 0 to 1"));
    }
    let coins = o.get("coins").and_then(Value::as_i64).unwrap_or(0);
    if !(0..=5).contains(&coins) || (!win && coins != 0) {
        return Err(format!("{PATH}: '{key}.coins' must be 0 to 5, and only a win pays"));
    }
    Ok(Some(Outcome { cue: cue(o, "cue")?, text: text(o, "text")?, chance, coins: coins as i32 }))
}

pub fn parse(src: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("{PATH}: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("{PATH}: 'schema' must be {SCHEMA}"));
    }
    let cooldown = num(&v, "cooldown_seconds")?;
    let mut kinds = vec![];
    for (ptype, k) in v["kinds"].as_object().ok_or_else(|| format!("{PATH}: 'kinds' missing"))? {
        kinds.push((
            ptype.clone(),
            Amusement {
                prompt: text(k, "prompt")?,
                press: cue(k, "press")?,
                result_seconds: num(k, "result_seconds")?,
                win: outcome(k, "win", true)?,
                lose: outcome(k, "lose", false)?,
                noise: num(k, "noise")?,
                noise_seconds: num(k, "noise_seconds")?,
                cooldown_seconds: if k.get("cooldown_seconds").is_some() { num(k, "cooldown_seconds")? } else { cooldown },
            },
        ));
    }
    Ok(Rules {
        noise_enabled: v["noise_enabled"].as_bool().unwrap_or(true),
        reach: num(&v, "reach")?,
        aim_height: num(&v, "aim_height")?,
        max_prizes: v["prize"]["max_per_run"].as_u64().ok_or_else(|| format!("{PATH}: 'prize.max_per_run' missing"))? as usize,
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
            Rules { noise_enabled: false, reach: 0.0, aim_height: 0.0, max_prizes: 0, kinds: vec![] }
        })
    })
}

impl Rules {
    /// The amusement a prop type is, None for a prop that is dressing.
    pub fn of(&self, ptype: &str) -> Option<&Amusement> {
        self.kinds.iter().find(|(t, _)| t == ptype).map(|(_, k)| k)
    }
}

/// The interact prompt of an amusement prop type ("[E] DROP THE CLAW"), empty for any other prop.
pub fn prompt(ptype: &str) -> String {
    match rules().of(ptype) {
        Some(k) => format!("[E] {}", k.prompt),
        None => String::new(),
    }
}

/// Plays a cue of the two families the file may name.
fn play_cue(name: &str) {
    if let Some(kind) = name.strip_prefix("carnival_") {
        crate::audio::sound::carnival(kind, 0.0, 1.0);
    } else if let Some(kind) = name.strip_prefix("music_") {
        crate::audio::sound::music(kind, 0.0, 1.0);
    }
}

/// The win roll of press number `press` on prop `id`, 0..1: a hash (FNV-1a, then a mixing step), the same for the
/// same press and well spread from one press to the next.
pub fn roll(id: &str, press: u32) -> f32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes().chain(press.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// A result on its way: what plays and shows at run time `at`.
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    at: f32,
    cue: String,
    text: String,
    coins: i32,
}

/// The amusements in play: the machines still cooling down (prop id, the run time of the press, its cooldown), the
/// results on their way, the press count the roll reads, and the machines that paid their prize. All short lists: a
/// runner reaches one or two machines at a time and a run pays a handful of prizes.
#[derive(Clone, Debug, Default)]
pub struct State {
    last: Vec<(String, f32, f32)>,
    pending: Vec<Pending>,
    presses: u32,
    paid: Vec<String>,
}

impl Game {
    /// Interact on amusement prop `i`: its press cue plays, the runner makes its noise burst and the result is on its
    /// way. False for a prop that is no amusement, or one played less than its cooldown ago.
    pub(in crate::game) fn play_amusement(&mut self, i: usize) -> bool {
        let r = rules();
        let Some((id, kind)) = self.fac.props.get(i).and_then(|p| r.of(&p.ptype).map(|k| (p.id.clone(), k))) else {
            return false;
        };
        let now = self.time;
        let st = &mut self.played.rides;
        st.last.retain(|(_, at, cooldown)| now >= *at && now - *at < *cooldown);
        if st.last.iter().any(|(p, _, _)| *p == id) {
            return false;
        }
        st.last.push((id.clone(), now, kind.cooldown_seconds));
        st.presses = st.presses.wrapping_add(1);
        play_cue(&kind.press);
        let won = kind.win.as_ref().filter(|w| roll(&id, st.presses) < w.chance);
        if let Some(out) = won.or(kind.lose.as_ref()) {
            let mut coins = 0;
            if out.coins > 0 && st.paid.len() < r.max_prizes && !st.paid.iter().any(|p| *p == id) {
                st.paid.push(id);
                coins = out.coins;
            }
            st.pending.push(Pending { at: now + kind.result_seconds, cue: out.cue.clone(), text: out.text.clone(), coins });
        }
        if r.noise_enabled && kind.noise > 0.0 {
            self.p.noise_burst = kind.noise;
            self.p.noise_burst_timer = kind.noise_seconds;
        }
        true
    }

    /// Every frame: the results whose time has come play their cue, show their line and pay their prize. Nothing is
    /// done while no result is on its way.
    pub(in crate::game) fn update_amusements(&mut self) {
        if self.played.rides.pending.is_empty() {
            return;
        }
        let now = self.time;
        let mut i = 0;
        while i < self.played.rides.pending.len() {
            if self.played.rides.pending[i].at > now {
                i += 1;
                continue;
            }
            let done = self.played.rides.pending.remove(i);
            play_cue(&done.cue);
            if done.coins > 0 {
                self.coins += done.coins;
                self.say(&format!("{} // COIN +{}", done.text, done.coins));
            } else {
                self.say(&done.text);
            }
        }
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

    /// A run with a high striker 0.9 m in front of the runner (along +z) and a claw machine 0.9 m behind it, the
    /// runner looking at the striker, and no hunter awake.
    fn midway() -> (Game, usize, usize, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-MIDWAY-7", 0, &content, &catalog);
        g.ensure_hunters(&content);
        for h in g.hunters.iter_mut() {
            h.active = false;
        }
        let cell = crate::world::Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.p.pos = cell.center();
        g.fac.props.push(prop("TEST:STRIKER", "strengthTester", cell, (0.0, 0.9)));
        g.fac.props.push(prop("TEST:CLAW", "prizeClawMachine", cell, (0.0, -0.9)));
        let striker = g.fac.props.len() - 2;
        g.p.yaw = std::f32::consts::PI;
        g.p.pitch = (rules().aim_height - g.p.eye().y).atan2(0.9);
        g.p.stun = 0.0;
        g.p.noise_burst = 0.0;
        g.p.noise_burst_timer = 0.0;
        (g, striker, striker + 1, content, catalog)
    }

    #[test]
    fn the_shipped_rules_parse_and_every_cue_and_prop_type_they_name_exists() {
        let src = embedded::data_under(PATH).next().expect("amusements.json is embedded").1;
        let r = parse(src).expect("amusements.json parses");
        assert_eq!(&r, rules());
        assert!(r.kinds.len() >= 15, "{} kinds", r.kinds.len());
        let content = Content::load();
        let cues = crate::audio::cues::cue_names();
        for (ptype, k) in &r.kinds {
            let def = content.props.get(ptype.as_str()).unwrap_or_else(|| panic!("{ptype} is not a prop type"));
            assert!(!def.models.is_empty(), "{ptype} has a model");
            assert_eq!(k.prompt, k.prompt.to_uppercase(), "{ptype}");
            assert!(k.cooldown_seconds > 0.0 && k.result_seconds <= 2.0, "{ptype}");
            for name in std::iter::once(&k.press).chain(k.win.iter().map(|o| &o.cue)).chain(k.lose.iter().map(|o| &o.cue)) {
                assert!(cues.iter().any(|c| *c == name.as_str()), "{ptype}: the cue {name} does not exist");
            }
            for o in k.win.iter().chain(k.lose.iter()) {
                assert_eq!(o.text, o.text.to_uppercase(), "{ptype}");
                assert!((0.0..=1.0).contains(&o.chance), "{ptype}");
            }
            // one prompt per prop: an amusement is no instrument, bed or seat
            assert!(crate::game::instruments::rules().of(ptype).is_none(), "{ptype} is an instrument too");
            assert!(crate::game::bed::seat_of(ptype).is_none() && crate::game::bed::sit_seat_of(ptype).is_none(), "{ptype} is a seat too");
        }
        // the booths the pass gave their own models, and the machines, are all playable
        for ptype in ["arcadeCabinet", "prizeClawMachine", "strengthTester", "whackAMole", "duckPond", "shootingGallery"] {
            assert!(r.of(ptype).is_some(), "{ptype}");
        }
        assert_eq!(prompt("strengthTester"), "[E] SWING THE MALLET");
        assert_eq!(prompt("officeChair"), "");
        // only a few machines pay, and a run pays a handful
        assert!(r.kinds.iter().filter(|(_, k)| k.win.as_ref().map_or(false, |w| w.coins > 0)).count() <= 3);
        assert!(r.max_prizes >= 1 && r.max_prizes <= 10);
        assert!(parse("{}").is_err());
        assert!(parse(&src.replace("\"cooldown_seconds\": 1.5,", "")).is_err(), "the cooldown is required data");
        assert!(parse(&src.replace("carnival_bell", "alert")).is_err(), "only booth and instrument cues may be named");
        assert!(parse(&src.replace("\"chance\": 0.4,", "\"chance\": 1.4,")).is_err(), "a chance is 0 to 1");
    }

    #[test]
    fn the_win_roll_is_stable_and_spread() {
        let mut sum = 0.0;
        let mut low = 0;
        for press in 0..2000u32 {
            let a = roll("C:3:0:-2:PROP:7", press);
            assert!((0.0..1.0).contains(&a));
            assert_eq!(a, roll("C:3:0:-2:PROP:7", press), "the same press rolls the same");
            sum += a;
            if a < 0.2 {
                low += 1;
            }
        }
        assert!((sum / 2000.0 - 0.5).abs() < 0.05, "mean {}", sum / 2000.0);
        assert!((300..500).contains(&low), "a 0.2 chance wins about one press in five: {low} of 2000");
        assert_ne!(roll("C:3:0:-2:PROP:7", 1), roll("C:3:0:-2:PROP:8", 1), "machines roll apart");
    }

    #[test]
    fn an_amusement_in_reach_offers_its_prompt() {
        let (mut g, striker, _, _, _) = midway();
        assert_eq!(g.find_focus(), Some(Focus::Amusement(striker)));
        g.focus = Some(Focus::Amusement(striker));
        assert_eq!(g.focus_prompt().as_deref(), Some("[E] SWING THE MALLET"));
        g.fac.props[striker].offset = (0.0, rules().reach + 1.0);
        assert_ne!(g.find_focus(), Some(Focus::Amusement(striker)));
    }

    #[test]
    fn a_press_makes_the_noise_at_once_and_the_result_comes_after_its_delay() {
        let (mut g, striker, _, content, catalog) = midway();
        let k = rules().of("strengthTester").unwrap();
        g.message.clear();
        g.interact(Focus::Amusement(striker), &content, &catalog, &Sounds::default());
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (k.noise, k.noise_seconds), "the hunters hear the mallet");
        assert_eq!(g.played.rides.pending.len(), 1);
        // not yet
        g.time += k.result_seconds * 0.5;
        g.update_amusements();
        assert!(g.message.is_empty() && g.played.rides.pending.len() == 1);
        g.time += k.result_seconds;
        g.update_amusements();
        let texts = [k.win.as_ref().unwrap().text.as_str(), k.lose.as_ref().unwrap().text.as_str()];
        assert!(texts.contains(&g.message.as_str()), "{}", g.message);
        assert!(g.played.rides.pending.is_empty());
        // mashed inside the cooldown: nothing plays, and a prop that is no amusement plays nothing
        g.p.noise_burst = 0.0;
        assert!(!g.play_amusement(striker));
        assert_eq!(g.p.noise_burst, 0.0);
        g.time += k.cooldown_seconds;
        assert!(g.play_amusement(striker));
        g.fac.props[striker].ptype = "musicStand".to_string();
        g.time += 60.0;
        assert!(!g.play_amusement(striker));
        assert!(!g.play_amusement(usize::MAX));
    }

    #[test]
    fn a_claw_machine_pays_its_coin_once_and_a_run_pays_only_a_few() {
        let (mut g, _, claw, _, _) = midway();
        let k = rules().of("prizeClawMachine").unwrap();
        let prize = k.win.as_ref().unwrap().coins;
        assert!(prize > 0);
        let start = g.coins;
        // enough presses that a 0.2 chance wins many times over: the machine still pays once
        for _ in 0..200 {
            assert!(g.play_amusement(claw));
            g.time += k.cooldown_seconds.max(k.result_seconds) + 0.1;
            g.update_amusements();
        }
        assert_eq!(g.coins, start + prize, "one prize from one machine");
        assert!(g.played.rides.pending.is_empty() && g.played.rides.last.len() <= 1, "the lists stay short");
        // other machines pay until the run's cap
        for n in 0..(rules().max_prizes + 4) {
            g.fac.props[claw].id = format!("TEST:CLAW:{n}");
            for _ in 0..200 {
                g.play_amusement(claw);
                g.time += k.cooldown_seconds.max(k.result_seconds) + 0.1;
                g.update_amusements();
            }
        }
        assert_eq!(g.coins, start + prize * rules().max_prizes as i32, "the run's cap holds");
        assert_eq!(g.played.rides.paid.len(), rules().max_prizes);
    }
}
