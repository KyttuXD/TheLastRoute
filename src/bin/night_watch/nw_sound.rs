//! Night Watch's sound: the game's `nw_` cues ported as a small procedural renderer. Night Watch cannot import the game
//! binary's cue table, so each cue's layers (tone or filtered noise: hz, ms, gain, wave, delay) are data in
//! `data/sounds.json`, rendered here to a mono 16-bit WAV like `blip.rs` does and played with macroquad's audio.
//! API for every mode: `cue(name, pan, k)`, `event(mode, event, pan)` (the mode's event-to-cue map in `sounds.json`
//! under `events`) and `room_tone(name, on)`. Unknown names and a missing audio context are silent with one log line.

use crate::blip::wav_bytes;
use macroquad::audio::{load_sound_from_bytes, play_sound, stop_sound, PlaySoundParams, Sound};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f64::consts::{PI, TAU};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::task::{Context, Poll, Waker};

const SOUNDS: &str = include_str!("data/sounds.json");

/// Playing is off until `init` runs (after the window and audio context exist), so tests and screenshot runs stay silent.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns playback on. Call once after the window opens.
pub fn init() {
    ENABLED.store(true, Ordering::Relaxed);
}

fn on() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn data() -> &'static Value {
    static V: OnceLock<Value> = OnceLock::new();
    V.get_or_init(|| serde_json::from_str(SOUNDS).unwrap_or_else(|e| panic!("night_watch data/sounds.json does not parse: {e}")))
}

fn num(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn sample_rate() -> u32 {
    num(data(), "sample_rate", 22050.0) as u32
}

/// Every cue name in the table.
#[allow(dead_code)]
pub fn cue_names() -> Vec<&'static str> {
    data()["cues"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default()
}

/// The cue an event plays in `mode`, or an error naming the missing key.
pub fn resolve(mode: &str, event: &str) -> Result<&'static str, String> {
    let cue = data()["events"][mode][event]
        .as_str()
        .ok_or_else(|| format!("night_watch sounds.json: events.{mode}.{event} is missing"))?;
    if data()["cues"].get(cue).is_none() {
        return Err(format!("night_watch sounds.json: events.{mode}.{event} names unknown cue '{cue}'"));
    }
    Ok(cue)
}

fn hash(text: &str) -> u32 {
    text.bytes().fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(u32::from(b)))
}

/// A cue as 16-bit mono samples, or None for an unknown name. `pan` -1..1 (the far side is quieter), `k` the intensity.
/// Deterministic: the noise is a seeded LCG.
pub fn render(name: &str, pan: f32, k: f32) -> Option<Vec<i16>> {
    let layers = data()["cues"].get(name)?.as_array()?;
    let rate = f64::from(sample_rate());
    let pan = f64::from(pan.clamp(-1.0, 1.0));
    let k = f64::from(k.clamp(0.0, 4.0));
    let side = 1.0 - 0.3 * pan.abs();
    let end = layers.iter().map(|c| num(c, "delay_ms", 0.0) + num(c, "ms", 0.0)).fold(50.0, f64::max) + 25.0;
    let frames = ((end / 1000.0 * rate).ceil() as usize).max(1);
    let mut mix = vec![0.0f64; frames];
    let mut state = i64::from(hash(&format!("{name}|{pan:.2}|{k:.2}"))).max(1);
    for c in layers {
        let start = (num(c, "delay_ms", 0.0) / 1000.0 * rate) as usize;
        let count = ((num(c, "ms", 1.0) / 1000.0 * rate) as usize).max(1);
        let hz = num(c, "hz", 440.0).max(20.0);
        let gain = num(c, "gain", 0.1) * if c["k"].as_bool().unwrap_or(false) { k } else { 1.0 };
        let noise = c["type"].as_str() == Some("noise");
        let wave = c["wave"].as_str().unwrap_or("sine");
        let alpha = (hz * num(c, "rate", 1.0) / rate * 5.0).clamp(0.015, 0.55);
        let span = (count as f64 - 1.0).max(1.0);
        let mut filtered = 0.0;
        for i in 0..count {
            let f = start + i;
            if f >= frames {
                break;
            }
            let t = i as f64 / rate;
            let env = (t / 0.009).min(1.0) * (1.0 - i as f64 / span).powf(2.25);
            let s = if noise {
                state = (state.wrapping_mul(1103515245).wrapping_add(12345)) & 0x7fff_ffff;
                filtered += (state as f64 / 1073741823.5 - 1.0 - filtered) * alpha;
                filtered
            } else {
                let phase = TAU * hz * t;
                match wave {
                    "square" => {
                        if phase.sin() >= 0.0 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    "triangle" => phase.sin().asin() * 2.0 / PI,
                    "saw" => 2.0 * ((phase / TAU) % 1.0) - 1.0,
                    _ => phase.sin(),
                }
            };
            mix[f] += s * gain * env * side;
        }
    }
    Some(mix.iter().map(|x| (x.clamp(-0.98, 0.98) * 32767.0).round() as i16).collect())
}

/// `load_sound_from_bytes` finishes on its first poll on native targets (as in `blip.rs`).
fn load(bytes: &[u8]) -> Option<Sound> {
    let mut fut = std::pin::pin!(load_sound_from_bytes(bytes));
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(s)) => Some(s),
        _ => None,
    }
}

#[derive(Default)]
struct Player {
    cache: HashMap<String, Option<Sound>>,
    tones: HashMap<String, Sound>,
    warned: HashSet<String>,
}

thread_local! {
    static PLAYER: RefCell<Player> = RefCell::new(Player::default());
}

fn master() -> f32 {
    num(data(), "master", 0.8) as f32
}

fn snap(x: f32) -> f32 {
    (x / 0.05).round() * 0.05
}

fn sound_for(p: &mut Player, key: String, name: &str, pan: f32, k: f32) -> Option<Sound> {
    if let Some(s) = p.cache.get(&key) {
        return s.clone();
    }
    let s = match render(name, pan, k) {
        Some(samples) => {
            let s = load(&wav_bytes(&samples, sample_rate()));
            if s.is_none() && p.warned.insert("no-audio".into()) {
                eprintln!("night_watch: no audio context, sounds are silent");
            }
            s
        }
        None => {
            if p.warned.insert(name.to_owned()) {
                eprintln!("night_watch: unknown sound cue '{name}'");
            }
            None
        }
    };
    p.cache.insert(key, s.clone());
    s
}

/// Plays the named cue once at `pan` (-1 left .. 1 right) and intensity `k` (1.0 normal). Silent if unknown.
pub fn cue(name: &str, pan: f32, k: f32) {
    if !on() {
        return;
    }
    let (pan, k) = (snap(pan.clamp(-1.0, 1.0)), snap(k.clamp(0.0, 4.0)));
    PLAYER.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(s) = sound_for(&mut p, format!("{name}|{pan:.2}|{k:.2}"), name, pan, k) {
            play_sound(&s, PlaySoundParams { looped: false, volume: master() });
        }
    });
}

/// Plays the cue `mode`'s event map names for `event`. An unmapped event is silent with one log line.
pub fn event(mode: &str, event: &str, pan: f32) {
    if !on() {
        return;
    }
    match resolve(mode, event) {
        Ok(name) => cue(name, pan, 1.0),
        Err(e) => PLAYER.with(|p| {
            if p.borrow_mut().warned.insert(e.clone()) {
                eprintln!("{e}");
            }
        }),
    }
}

/// Starts (`play`) or stops a looping low bed (`hum`, `hum_muffled`).
#[allow(dead_code)]
pub fn room_tone(name: &str, play: bool) {
    if !on() {
        return;
    }
    PLAYER.with(|p| {
        let mut p = p.borrow_mut();
        if !play {
            if let Some(s) = p.tones.remove(name) {
                stop_sound(&s);
            }
            return;
        }
        if p.tones.contains_key(name) {
            return;
        }
        if let Some(s) = sound_for(&mut p, format!("{name}|tone"), name, 0.0, 1.0) {
            play_sound(&s, PlaySoundParams { looped: true, volume: master() * num(data(), "tone_volume", 0.35) as f32 });
            p.tones.insert(name.to_owned(), s);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(s: &[i16]) -> i32 {
        s.iter().map(|v| i32::from(*v).abs()).max().unwrap()
    }

    #[test]
    fn every_cue_renders_a_non_empty_buffer() {
        let names = cue_names();
        assert_eq!(names.len(), 37);
        for n in names {
            let s = render(n, 0.0, 1.0).unwrap_or_else(|| panic!("{n}"));
            assert!(!s.is_empty() && s.iter().any(|v| *v != 0), "{n} is silent");
        }
        assert!(render("nope", 0.0, 1.0).is_none());
    }

    #[test]
    fn rendering_is_deterministic() {
        for n in ["step", "scare", "door", "bed_breath"] {
            assert_eq!(render(n, 0.3, 1.0), render(n, 0.3, 1.0));
        }
    }

    #[test]
    fn pan_extremes_stay_in_the_sample_range_and_the_far_side_is_quieter() {
        for n in cue_names() {
            for pan in [-1.0f32, 0.0, 1.0, 5.0] {
                let s = render(n, pan, 4.0).unwrap();
                assert!(s.iter().all(|v| i32::from(*v).abs() <= 32767), "{n} pan {pan}");
            }
        }
        assert!(peak(&render("chime", 1.0, 1.0).unwrap()) < peak(&render("chime", 0.0, 1.0).unwrap()));
    }

    #[test]
    fn intensity_scales_the_k_cues() {
        assert!(peak(&render("step", 0.0, 0.5).unwrap()) < peak(&render("step", 0.0, 1.0).unwrap()));
    }

    #[test]
    fn every_mapped_event_resolves_to_an_existing_cue() {
        let classic = data()["events"]["classic"].as_object().expect("classic map");
        assert!(!classic.is_empty());
        for (ev, cue) in classic {
            let name = resolve("classic", ev).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(Some(name), cue.as_str());
            assert!(render(name, 0.0, 1.0).is_some(), "{ev} -> {name}");
        }
    }

    #[test]
    fn a_missing_key_reports_its_name() {
        let e = resolve("classic", "no_such_event").unwrap_err();
        assert!(e.contains("no_such_event") && e.contains("classic"), "{e}");
        assert!(resolve("no_such_mode", "door_toggle").unwrap_err().contains("no_such_mode"));
    }
}
