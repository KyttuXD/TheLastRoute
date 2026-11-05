//! Hiding state across the network (audit C1, C2, C5, C6; Godot remote_players.gd 501-504, 556, 602-604, 737-746
//! and infinite_route_test.gd 3626-3704). Tuning lives in `data/net/hide_sync.json` (`lr.hide_sync/1`).
//!
//! * [`HideState`]: what a runner broadcasts while in a locker (hidden, spot index, style, slot) with JSON
//!   encode/decode. Old or unknown messages decode to "not hidden"; the spot index is clamped.
//! * [`occupancy`]: who is in which spot, honouring the capacity and the front (0) / beside (1) slot choice.
//! * [`host_hides`]: the host-side list of hidden runners (local and remote) the hunter locker check consults.
//! * [`search_tell_shake`]: the offset a locker shows while a hunter searches it, broadcast so all see it.
use serde_json::{json, Value};
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.hide_sync/1";
const TEXT: &str = include_str!("../data/net/hide_sync.json");

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub max_capacity: i32,
    pub max_spot_index: i32,
    pub max_style_len: usize,
    /// The style a facility locker has (data/render/locker_shell.json `occupant.bed_styles` lists the lying ones).
    pub locker_style: String,
    pub tell_duration_s: f32,
    pub tell_amplitude: f32,
    pub tell_rate_hz: f32,
}

fn num(v: &Value, key: &str, path: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("hide_sync.json: key '{path}' missing or not a number"))
}

pub fn parse(text: &str) -> Result<Config, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("hide_sync.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("hide_sync.json: key 'schema' must be {SCHEMA}"));
    }
    let tell = v.get("search_tell").ok_or("hide_sync.json: key 'search_tell' missing")?;
    let cfg = Config {
        max_capacity: num(&v, "max_capacity", "max_capacity")? as i32,
        max_spot_index: num(&v, "max_spot_index", "max_spot_index")? as i32,
        max_style_len: num(&v, "max_style_len", "max_style_len")? as usize,
        locker_style: v
            .get("locker_style")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("hide_sync.json: key 'locker_style' missing or not a name")?
            .to_string(),
        tell_duration_s: num(tell, "duration_s", "search_tell.duration_s")? as f32,
        tell_amplitude: num(tell, "amplitude", "search_tell.amplitude")? as f32,
        tell_rate_hz: num(tell, "rate_hz", "search_tell.rate_hz")? as f32,
    };
    if cfg.max_capacity < 1 {
        return Err("hide_sync.json: key 'max_capacity' must be at least 1".into());
    }
    if cfg.max_spot_index < 0 {
        return Err("hide_sync.json: key 'max_spot_index' must not be negative".into());
    }
    if cfg.tell_duration_s < 0.0 || cfg.tell_amplitude < 0.0 || cfg.tell_rate_hz < 0.0 {
        return Err("hide_sync.json: key 'search_tell' values must not be negative".into());
    }
    Ok(cfg)
}

pub fn config() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

/// One runner's hiding state as broadcast. `slot` 0 is in front, 1 is beside.
#[derive(Clone, Debug, PartialEq)]
pub struct HideState {
    pub hidden: bool,
    pub spot_index: i32,
    pub style: String,
    pub slot: u8,
}

impl Default for HideState {
    fn default() -> Self {
        HideState { hidden: false, spot_index: -1, style: String::new(), slot: 0 }
    }
}

impl HideState {
    pub fn hiding(spot_index: i32, style: &str, slot: u8) -> Self {
        HideState { hidden: true, spot_index, style: style.to_string(), slot }
    }

    pub fn encode(&self) -> Value {
        if !self.hidden {
            return json!({ "hidden": false });
        }
        json!({ "hidden": true, "spot": self.spot_index, "style": self.style, "slot": self.slot })
    }

    /// Missing, malformed or old fields decode to "not hidden"; the spot index and slot are clamped.
    pub fn decode(v: &Value) -> HideState {
        let c = config();
        if v.get("hidden").and_then(Value::as_bool) != Some(true) {
            return HideState::default();
        }
        let Some(spot) = v.get("spot").and_then(Value::as_i64) else { return HideState::default() };
        let style: String = v.get("style").and_then(Value::as_str).unwrap_or("").chars().take(c.max_style_len).collect();
        let slot = v.get("slot").and_then(Value::as_i64).unwrap_or(0).clamp(0, (c.max_capacity - 1) as i64) as u8;
        HideState { hidden: true, spot_index: spot.clamp(0, c.max_spot_index as i64) as i32, style, slot }
    }
}

/// The result for one spot: accepted occupants as `(state index, slot)` and the refused state indexes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpotOccupancy {
    pub occupants: Vec<(usize, u8)>,
    pub refused: Vec<usize>,
}

/// Assign each hidden state to its spot. `capacities[i]` is spot i's capacity (clamped to the data maximum).
/// A state takes its wanted slot when free, else the other free slot; with no free slot it is refused.
/// States are served in order, so the earlier hider keeps the spot.
pub fn occupancy(capacities: &[i32], states: &[HideState]) -> Vec<SpotOccupancy> {
    let max = config().max_capacity;
    let mut out = vec![SpotOccupancy::default(); capacities.len()];
    for (i, s) in states.iter().enumerate() {
        if !s.hidden || s.spot_index < 0 || s.spot_index as usize >= capacities.len() {
            continue;
        }
        let spot = s.spot_index as usize;
        let cap = capacities[spot].clamp(1, max) as u8;
        let used = &out[spot].occupants;
        let want = s.slot.min(cap - 1);
        let slot = if !used.iter().any(|&(_, sl)| sl == want) {
            Some(want)
        } else {
            (0..cap).find(|c| !used.iter().any(|&(_, sl)| sl == *c))
        };
        match slot {
            Some(sl) => out[spot].occupants.push((i, sl)),
            None => out[spot].refused.push(i),
        }
    }
    out
}

/// A runner the host treats as hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct HiddenRunner {
    pub peer_id: String,
    pub spot_index: i32,
    pub style: String,
    pub slot: u8,
}

/// The host's hidden runners in input order, after capacity is applied (a refused hider is not hidden).
/// `runners` is every runner (local first) as `(peer id, state)`; the hunter locker check consults the result
/// for local and remote runners alike.
pub fn host_hides(capacities: &[i32], runners: &[(String, HideState)]) -> Vec<HiddenRunner> {
    let states: Vec<HideState> = runners.iter().map(|(_, s)| s.clone()).collect();
    let mut found = Vec::new();
    for (spot, occ) in occupancy(capacities, &states).iter().enumerate() {
        for &(i, slot) in &occ.occupants {
            found.push((i, HiddenRunner { peer_id: runners[i].0.clone(), spot_index: spot as i32, style: states[i].style.clone(), slot }));
        }
    }
    found.sort_by_key(|(i, _)| *i);
    found.into_iter().map(|(_, h)| h).collect()
}

/// The locker's shake offset `t` seconds after a hunter began searching it: zero before the tell and after its
/// duration, otherwise a sine bounded by the amplitude.
pub fn search_tell_shake(t: f32) -> f32 {
    let c = config();
    if t < 0.0 || t > c.tell_duration_s {
        return 0.0;
    }
    c.tell_amplitude * (t * c.tell_rate_hz * std::f32::consts::TAU).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(id: &str, s: HideState) -> (String, HideState) {
        (id.to_string(), s)
    }

    #[test]
    fn round_trip() {
        let s = HideState::hiding(3, "wardrobe", 1);
        assert_eq!(HideState::decode(&s.encode()), s);
        assert_eq!(HideState::decode(&HideState::default().encode()), HideState::default());
    }

    #[test]
    fn old_messages_decode_visible() {
        assert!(!HideState::decode(&json!({})).hidden);
        assert!(!HideState::decode(&json!({"x": 1})).hidden);
        assert!(!HideState::decode(&json!({"hidden": true})).hidden);
        assert!(!HideState::decode(&json!("junk")).hidden);
    }

    #[test]
    fn spot_index_is_clamped() {
        let d = HideState::decode(&json!({"hidden": true, "spot": -9, "slot": 7}));
        assert!(d.hidden && d.spot_index == 0 && (d.slot as i32) < config().max_capacity);
        let d = HideState::decode(&json!({"hidden": true, "spot": 99999999}));
        assert_eq!(d.spot_index, config().max_spot_index);
    }

    #[test]
    fn capacity_one_refuses_second() {
        let s = [HideState::hiding(0, "bed", 0), HideState::hiding(0, "bed", 0)];
        let o = occupancy(&[1], &s);
        assert_eq!(o[0].occupants, vec![(0, 0)]);
        assert_eq!(o[0].refused, vec![1]);
    }

    #[test]
    fn capacity_two_assigns_both_slots() {
        let s = [HideState::hiding(0, "wardrobe", 0), HideState::hiding(0, "wardrobe", 0)];
        let o = occupancy(&[2], &s);
        assert_eq!(o[0].occupants, vec![(0, 0), (1, 1)]);
        assert!(o[0].refused.is_empty());
        let third = [s[0].clone(), s[1].clone(), s[0].clone()];
        assert_eq!(occupancy(&[2], &third)[0].refused, vec![2]);
    }

    #[test]
    fn host_hides_includes_remote() {
        let r = [runner("host", HideState::default()), runner("peer", HideState::hiding(0, "bed", 0))];
        let h = host_hides(&[1], &r);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].peer_id, "peer");
        let both = [runner("host", HideState::hiding(0, "bed", 0)), runner("peer", HideState::hiding(0, "bed", 0))];
        let h = host_hides(&[1], &both);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].peer_id, "host");
    }

    #[test]
    fn shake_is_zero_before_and_bounded() {
        let c = config();
        assert_eq!(search_tell_shake(-0.5), 0.0);
        assert_eq!(search_tell_shake(c.tell_duration_s + 1.0), 0.0);
        for i in 0..200 {
            let t = i as f32 * c.tell_duration_s / 200.0;
            assert!(search_tell_shake(t).abs() <= c.tell_amplitude + 1e-6);
        }
    }

    #[test]
    fn standard_data_parses_and_errors_name_the_key() {
        assert!(parse(TEXT).is_ok());
        let e = parse(&TEXT.replace("\"max_capacity\": 2,", "")).unwrap_err();
        assert!(e.contains("max_capacity"), "{e}");
        let e = parse(&TEXT.replace("\"locker_style\"", "\"style\"")).unwrap_err();
        assert!(e.contains("locker_style"), "{e}");
        let e = parse(&TEXT.replace("\"amplitude\"", "\"amp\"")).unwrap_err();
        assert!(e.contains("amplitude"), "{e}");
        let e = parse(&TEXT.replace("\"max_capacity\": 2", "\"max_capacity\": 0")).unwrap_err();
        assert!(e.contains("max_capacity"), "{e}");
    }
}
