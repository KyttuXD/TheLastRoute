//! The quiet hunter sounds (data/audio/hunter_ambient.json): what a hunter makes while she is NOT chasing - her steps
//! as she wanders, investigates or searches, a sniff, a breath at the locker, Glyra's hum, the spiders' skitter and the
//! lair's crackle, and the heartbeat while one is near but not after you. Each has a gain (x its normal loudness) and,
//! where it has a reach, a range that replaces the normal one. Chase, jump-scare, capture and grab cues never read this.
//! A missing file, key or a bad value keeps today's loudness (gain 1, normal reach).

use std::sync::OnceLock;

use super::cues::rules::HunterStepHearing;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HunterAmbient {
    pub step_gain: f64,
    pub step_range: Option<f64>,
    pub sniff_gain: f64,
    pub sniff_range: Option<f64>,
    pub breath_gain: f32,
    pub breath_range: Option<f32>,
    pub hum_gain: f32,
    pub hum_max_distance: Option<f32>,
    pub skitter_gain: f32,
    pub crackle_gain: f32,
    pub calm_heartbeat_gain: f32,
}

impl Default for HunterAmbient {
    fn default() -> Self {
        HunterAmbient {
            step_gain: 1.0,
            step_range: None,
            sniff_gain: 1.0,
            sniff_range: None,
            breath_gain: 1.0,
            breath_range: None,
            hum_gain: 1.0,
            hum_max_distance: None,
            skitter_gain: 1.0,
            crackle_gain: 1.0,
            calm_heartbeat_gain: 1.0,
        }
    }
}

impl HunterAmbient {
    pub fn parse(text: &str) -> HunterAmbient {
        let d = HunterAmbient::default();
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let gain = |block: &str, key: &str, d: f64| v[block][key].as_f64().filter(|x| x.is_finite() && *x >= 0.0).map(|x| x.min(4.0)).unwrap_or(d);
        let range = |block: &str, key: &str| v[block][key].as_f64().filter(|x| x.is_finite() && *x > 0.0);
        HunterAmbient {
            step_gain: gain("step", "gain", d.step_gain),
            step_range: range("step", "range"),
            sniff_gain: gain("sniff", "gain", d.sniff_gain),
            sniff_range: range("sniff", "range"),
            breath_gain: gain("breath", "gain", 1.0) as f32,
            breath_range: range("breath", "range").map(|x| x as f32),
            hum_gain: gain("hum", "gain", 1.0) as f32,
            hum_max_distance: range("hum", "max_distance").map(|x| x as f32),
            skitter_gain: gain("creature", "skitter_gain", 1.0) as f32,
            crackle_gain: gain("creature", "crackle_gain", 1.0) as f32,
            calm_heartbeat_gain: gain("heartbeat", "calm_gain", 1.0) as f32,
        }
    }

    /// The embedded data's tuning.
    pub fn get() -> &'static HunterAmbient {
        static H: OnceLock<HunterAmbient> = OnceLock::new();
        H.get_or_init(|| HunterAmbient::parse(include_str!("../data/audio/hunter_ambient.json")))
    }

    /// The hearing of a non-chasing hunter's step: the chase hearing with the tuned range and the volume x gain.
    pub fn step_hearing(&self, base: HunterStepHearing) -> HunterStepHearing {
        HunterStepHearing { range: self.step_range.unwrap_or(base.range), pan_span: base.pan_span, volume: base.volume * self.step_gain }
    }

    /// The hearing of a sniff.
    pub fn sniff_hearing(&self, base: HunterStepHearing) -> HunterStepHearing {
        HunterStepHearing { range: self.sniff_range.unwrap_or(base.range), pan_span: base.pan_span, volume: base.volume * self.sniff_gain }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_tuning_is_quieter_and_shorter_than_the_chase_figures() {
        let a = HunterAmbient::get();
        let base = HunterStepHearing::default();
        let quiet = a.step_hearing(base);
        assert!(quiet.volume <= base.volume * 0.6 && quiet.range < base.range, "{quiet:?}");
        assert!(a.sniff_hearing(base).range < base.range);
        assert!(a.breath_gain < 1.0 && a.breath_range.unwrap() < 3.5);
        assert!(a.hum_gain < 1.0 && a.hum_max_distance.unwrap() < 14.0);
        assert!(a.skitter_gain < 1.0 && a.crackle_gain < 1.0 && a.calm_heartbeat_gain < 1.0);
    }

    #[test]
    fn a_missing_file_or_bad_value_keeps_todays_loudness() {
        assert_eq!(HunterAmbient::parse("{}"), HunterAmbient::default());
        assert_eq!(HunterAmbient::parse("not json"), HunterAmbient::default());
        let p = HunterAmbient::parse(r#"{"step": {"gain": -3, "range": -1}}"#);
        assert_eq!((p.step_gain, p.step_range), (1.0, None));
    }

    #[test]
    fn the_chase_hearing_is_left_alone() {
        let base = HunterStepHearing::default();
        let a = HunterAmbient::get();
        assert_eq!(HunterStepHearing::get(), base, "the chase figure in cues.json is unchanged");
        let _ = a.step_hearing(base);
        assert_eq!(base, HunterStepHearing::default());
    }
}
