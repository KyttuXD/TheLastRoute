//! Hunter mechanic animations: which named state a hunter is in because of its gimmick (docs/HUNTER_ANIMATIONS.md).
//!
//! The host's gimmick code decides WHEN (it is the only place that knows a battery was just taken or a tripwire hit);
//! the render side (`render/anim/profile.rs`, data/render/hunter_anims.json `states`) decides what it LOOKS like. The two meet
//! on a name:
//!
//! * a HELD name lasts as long as a gimmick state does (`Gimmick::mechanic`: reaching for a battery, holding the music
//!   box, a lunge windup),
//! * a CUE is raised once at an event (`Brain::raise_cue`) and the brain keeps it for `seconds`
//!   (data/hunters/mechanic_cues.json), then it clears by itself. A stun, a floor change or the end of the run drops it
//!   too; it never moves or freezes the hunter.
//!
//! The host sends the name in its hunter frame (`mech`, game/resident_hunters.rs `frame_entry`) so a guest plays the same
//! animation; the guest keeps it for `net_ttl` seconds past the last frame that carried it.

use crate::content::embedded;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Where the cue timings live under data/.
pub const PATH: &str = "hunters/mechanic_cues.json";

/// Every one-shot cue a gimmick may raise. Each needs a `seconds` entry in the data file (tested).
pub const CUES: &[&str] = &[
    "pocket_battery",
    "burn_battery",
    "fuse_burst",
    "lock_on",
    "flash_recoil",
    "pounce_curse",
    "lay_wire",
    "tripwire_snap",
    "mimic_cue",
    "shapeshift",
    "materialize",
    "music_box_startle",
    "music_box_settle",
    "darkness_wake",
    "reveal_flinch",
    "jam_cast",
    "signal_ping",
    "vent_surface",
    "sabotage_cast",
    "footstep_trick",
    "static_cast",
];

/// Every name that lasts as long as a gimmick state does, plus the ladder climb the brain raises itself.
pub const HELD: &[&str] = &[
    "climb",
    "steal_battery",
    "pounce",
    "cold_recoil",
    "music_box_hold",
    "survey",
    "ping_mark",
    "fuse_lit",
    "relay_lock",
    "locker_vigil",
    "relay_camp",
    "corner_watch",
    "locker_loom",
    "beam_spot",
    "charge_windup",
    "charge_lunge",
    "guard_post",
    "torch_frenzy",
    "vent_stalk",
    "freeze_cast",
    "trade_offer",
    "watched_freeze",
    "signal_lock",
    "beam_flinch",
];

/// The cue timings of the embedded file.
#[derive(Debug, PartialEq)]
pub struct Cues {
    pub net_ttl: f32,
    seconds: HashMap<String, f32>,
}

impl Cues {
    pub fn parse(text: &str) -> Result<Cues, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(Value::as_str) != Some("lr.mechanic_cues/1") {
            return Err("schema must be lr.mechanic_cues/1".into());
        }
        let num = |x: &Value, what: &str, lo: f32, hi: f32| -> Result<f32, String> {
            let n = x.as_f64().ok_or_else(|| format!("{what}: must be a number"))? as f32;
            if !(n.is_finite() && (lo..=hi).contains(&n)) {
                return Err(format!("{what}: {n} is outside {lo}..{hi}"));
            }
            Ok(n)
        };
        let net_ttl = match v.get("net_ttl") {
            Some(x) => num(x, "net_ttl", 0.05, 5.0)?,
            None => 0.6,
        };
        let mut seconds = HashMap::new();
        for (k, x) in v.get("seconds").and_then(Value::as_object).into_iter().flatten() {
            if !CUES.contains(&k.as_str()) {
                return Err(format!("seconds.{k}: not a cue (hunters/mechanics.rs CUES)"));
            }
            seconds.insert(k.clone(), num(x, &format!("seconds.{k}"), 0.05, 30.0)?);
        }
        Ok(Cues { net_ttl, seconds })
    }

    /// How long the cue lasts; 1 s for a cue the file does not time.
    pub fn seconds(&self, name: &str) -> f32 {
        self.seconds.get(name).copied().unwrap_or(1.0)
    }
}

/// The timings compiled into the executable, read once. A bad file is reported on stderr and the defaults used.
pub fn cues() -> &'static Cues {
    static CUES_T: OnceLock<Cues> = OnceLock::new();
    CUES_T.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Cues::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Cues { net_ttl: 0.6, seconds: HashMap::new() }
        })
    })
}

/// The `'static` name for a string off the wire (None for anything this build does not know, so a newer host's name is ignored).
pub fn intern(name: &str) -> Option<&'static str> {
    CUES.iter().chain(HELD.iter()).copied().find(|n| *n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_file_parses_and_times_every_cue() {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).expect("embedded");
        let c = Cues::parse(text).expect("parses");
        for name in CUES {
            assert!(c.seconds.contains_key(*name), "{name} has no seconds entry");
        }
        assert_eq!(c.seconds.len(), CUES.len(), "no entry for a name that is not a cue");
    }

    #[test]
    fn names_are_unique() {
        let mut all: Vec<&str> = CUES.iter().chain(HELD.iter()).copied().collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn a_bad_file_is_refused_with_the_reason() {
        assert!(Cues::parse(r#"{"schema":"x"}"#).is_err());
        assert!(Cues::parse(r#"{"schema":"lr.mechanic_cues/1","seconds":{"nonsense":1}}"#).is_err());
        assert!(Cues::parse(r#"{"schema":"lr.mechanic_cues/1","seconds":{"mimic_cue":-1}}"#).is_err());
        assert!(Cues::parse(r#"{"schema":"lr.mechanic_cues/1","net_ttl":100}"#).is_err());
        assert_eq!(Cues::parse(r#"{"schema":"lr.mechanic_cues/1"}"#).unwrap().seconds("mimic_cue"), 1.0);
    }

    #[test]
    fn intern_finds_known_names_only() {
        assert_eq!(intern("steal_battery"), Some("steal_battery"));
        assert_eq!(intern("mimic_cue"), Some("mimic_cue"));
        assert_eq!(intern("from_the_future"), None);
        assert_eq!(intern(""), None);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    /// The timing table is not a hunter: the roster loader must not try to read it as one (it would leave a warning).
    #[test]
    fn the_cue_table_is_not_read_as_a_hunter_definition() {
        let c = crate::content::Content::load();
        assert!(!c.warnings.iter().any(|w| w.contains("mechanic_cues")), "{:?}", c.warnings);
        assert!(c.hunters.iter().all(|h| h.id != "mechanic_cues"));
    }

    /// A hostile `mech` off the wire (huge, wrong case, non-string) never becomes a name.
    #[test]
    fn hostile_wire_names_are_never_interned() {
        let huge = "x".repeat(100_000);
        for bad in [huge.as_str(), "STEAL_BATTERY", " steal_battery", "steal_battery\0", "climb ", "\u{0}"] {
            assert_eq!(intern(bad), None, "{bad:?}");
        }
    }
}
