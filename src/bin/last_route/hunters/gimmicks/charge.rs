//! Hunter charge abilities: a hunter's passive roll to lunge at a hiding spot and the windup and
//! charge timing of that lunge. Every number is per hunter, read from data/hunters/charge.json
//! (`lr.charge/1`) through the embedded data loader. Standard values match Godot
//! game/config/game_config.gd LOXELLE_* (lines 497-515).

use crate::content::embedded;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The data file's path under the embedded data root (see content/mod.rs, which skips it as a hunter).
pub const CHARGE_PATH: &str = "hunters/charge.json";

/// How often and how likely a hunter lunges while calm, and what the lunge costs and breaks.
#[derive(Clone, Debug, PartialEq)]
pub struct Passive {
    /// Seconds between rolls while calm (LOXELLE_INTERVAL).
    pub interval: f32,
    /// Chance per roll that she lunges (LOXELLE_CHANCE).
    pub chance: f32,
    /// Seconds an empty spot stays dark after a lunge (LOXELLE_REGEN_SECONDS).
    pub regen: f32,
    /// Seconds she is stunned after a lunge (LOXELLE_BREAK_STALL).
    pub stall: f32,
    /// Temperature added when she breaks a spot with you in it (LOXELLE_BREAK_TEMPERATURE).
    pub break_temperature: f32,
    /// Noise added when she breaks a spot with you in it (LOXELLE_BREAK_NOISE).
    pub break_noise: f32,
}

/// The lunge itself: windup, then the run at the spot.
#[derive(Clone, Debug, PartialEq)]
pub struct Lunge {
    /// Seconds of bracing before the run (LOXELLE_CHARGE_WINDUP).
    pub windup: f32,
    /// Seconds allowed for the run (LOXELLE_CHARGE_SECONDS).
    pub seconds: f32,
    /// Run speed in cells per second (LOXELLE_CHARGE_SPEED).
    pub speed: f32,
    /// Distance at which the approach ends and the windup starts (LOXELLE_CHARGE_START_DISTANCE).
    pub start_distance: f32,
    /// Distance at which the run lands (LOXELLE_CHARGE_IMPACT_DISTANCE).
    pub impact_distance: f32,
}

/// The chase-only room-door breach (Godot loxelle_gimmick.gd `start_door_breach`, 137-150, and
/// `_update_door_charge`, 152-182). It shares the lunge's speed and impact distance; only its own timing lives here.
#[derive(Clone, Debug, PartialEq)]
pub struct Door {
    /// Seconds of bracing before the door run (LOXELLE_CHARGE_WINDUP, game_config.gd 505).
    pub windup: f32,
    /// Cap on the door run's seconds; the run uses min(cap, LOXELLE_CHARGE_SECONDS) (loxelle_gimmick.gd 162, literal `.42`).
    pub charge_cap: f32,
    /// Seconds she is stunned after the door breaks (loxelle_gimmick.gd 182, literal `.65`).
    pub stun: f32,
}

impl Door {
    /// The door run's timer: min(charge_cap, the lunge's run seconds), as loxelle_gimmick.gd line 162 does.
    pub fn run_seconds(&self, lunge: &Lunge) -> f32 {
        self.charge_cap.min(lunge.seconds)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChargeSpec {
    pub passive: Passive,
    pub lunge: Lunge,
    pub door: Door,
}

/// Parses data/hunters/charge.json into one spec per hunter id.
pub fn parse(v: &Value) -> Result<HashMap<String, ChargeSpec>, String> {
    if v["schema"].as_str() != Some("lr.charge/1") {
        return Err("charge.json: schema must be lr.charge/1".into());
    }
    let hunters = v["hunters"].as_object().ok_or("charge.json: hunters must be an object")?;
    let mut out = HashMap::new();
    for (id, h) in hunters {
        let p = &h["passive"];
        let c = &h["charge"];
        let num = |o: &Value, k: &str| -> Result<f32, String> {
            o[k].as_f64().map(|x| x as f32).ok_or_else(|| format!("charge.json: {id}.{k} must be a number"))
        };
        let spec = ChargeSpec {
            passive: Passive {
                interval: num(p, "interval_seconds")?,
                chance: num(p, "chance")?,
                regen: num(p, "regen_seconds")?,
                stall: num(p, "stall_seconds")?,
                break_temperature: num(p, "break_temperature")?,
                break_noise: num(p, "break_noise")?,
            },
            lunge: Lunge {
                windup: num(c, "windup_seconds")?,
                seconds: num(c, "charge_seconds")?,
                speed: num(c, "speed")?,
                start_distance: num(c, "start_distance")?,
                impact_distance: num(c, "impact_distance")?,
            },
            door: {
                let d = &h["door"];
                Door {
                    windup: num(d, "windup_seconds")?,
                    charge_cap: num(d, "charge_cap_seconds")?,
                    stun: num(d, "stun_seconds")?,
                }
            },
        };
        out.insert(id.clone(), spec);
    }
    Ok(out)
}

/// Every hunter's charge spec from the embedded data (parsed once).
fn table() -> &'static HashMap<String, ChargeSpec> {
    static TABLE: OnceLock<HashMap<String, ChargeSpec>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let (_, text) = embedded::data_under(CHARGE_PATH).next().expect("data/hunters/charge.json is embedded");
        let v: Value = serde_json::from_str(text).expect("charge.json is valid JSON");
        parse(&v).expect("charge.json matches lr.charge/1")
    })
}

/// The charge spec for a hunter id, or None when that hunter has no charge ability.
pub fn for_hunter(id: &str) -> Option<&'static ChargeSpec> {
    table().get(id)
}

impl Passive {
    /// Whether a roll in 0..1000 (as from next_index(1000)) lunges: the roll must fall under chance * 1000
    /// (Godot `int(LOXELLE_CHANCE * 1000.0)`).
    pub fn roll_lunges(&self, roll: usize) -> bool {
        roll < (self.chance * 1000.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(chance: f64, windup: f64) -> Value {
        serde_json::json!({
            "schema": "lr.charge/1",
            "hunters": { "loxelle": {
                "passive": { "interval_seconds": 5.0, "chance": chance, "regen_seconds": 30.0,
                    "stall_seconds": 10.0, "break_temperature": 30.0, "break_noise": 11.0 },
                "charge": { "windup_seconds": windup, "charge_seconds": 0.70, "speed": 6.8,
                    "start_distance": 0.95, "impact_distance": 0.20 },
                "door": { "windup_seconds": 0.42, "charge_cap_seconds": 0.42, "stun_seconds": 0.65 }
            } }
        })
    }

    /// Standard values in the shipped data match the Godot constants (game_config.gd 497-515).
    #[test]
    fn shipped_loxelle_values_match_godot_standard() {
        let s = for_hunter("loxelle").expect("loxelle has a charge spec");
        assert_eq!(s.passive, Passive { interval: 5.0, chance: 0.25, regen: 30.0, stall: 10.0, break_temperature: 30.0, break_noise: 11.0 });
        assert_eq!(s.lunge, Lunge { windup: 0.42, seconds: 0.70, speed: 6.8, start_distance: 0.95, impact_distance: 0.20 });
    }

    /// The shipped door breach matches the Godot literals: windup LOXELLE_CHARGE_WINDUP, run cap .42, stun .65.
    #[test]
    fn shipped_loxelle_door_matches_godot_standard() {
        let s = for_hunter("loxelle").expect("loxelle has a charge spec");
        assert_eq!(s.door, Door { windup: 0.42, charge_cap: 0.42, stun: 0.65 });
        assert!((s.door.run_seconds(&s.lunge) - 0.42).abs() < 1e-6);
    }

    /// Editing the door values in the data changes the door timing, and the run cap never exceeds the lunge's run.
    #[test]
    fn editing_the_door_data_changes_the_door_timing() {
        let mut v = doc(0.25, 0.42);
        v["hunters"]["loxelle"]["door"]["stun_seconds"] = Value::from(1.5);
        v["hunters"]["loxelle"]["door"]["charge_cap_seconds"] = Value::from(0.30);
        let t = parse(&v).unwrap();
        assert_eq!(t["loxelle"].door.stun, 1.5);
        assert_eq!(t["loxelle"].door.run_seconds(&t["loxelle"].lunge), 0.30);
        v["hunters"]["loxelle"]["door"]["charge_cap_seconds"] = Value::from(2.0);
        let u = parse(&v).unwrap();
        assert_eq!(u["loxelle"].door.run_seconds(&u["loxelle"].lunge), 0.70);
    }

    /// A hunter's door breach needs its door block; a missing number is an error, not a silent default.
    #[test]
    fn missing_door_numbers_are_rejected() {
        let mut v = doc(0.25, 0.42);
        v["hunters"]["loxelle"].as_object_mut().unwrap().remove("door");
        assert!(parse(&v).is_err());
    }

    /// Hunters without a charge ability have no spec.
    #[test]
    fn other_hunters_have_no_charge_spec() {
        assert!(for_hunter("quenelle").is_none());
    }

    /// Each hunter in the data keeps its own windup and run; one hunter's numbers never leak into another's.
    #[test]
    fn each_hunter_has_its_own_windup_and_run() {
        let mut v = doc(0.25, 0.42);
        let mut other = v["hunters"]["loxelle"].clone();
        other["charge"]["windup_seconds"] = Value::from(0.90);
        other["charge"]["speed"] = Value::from(3.0);
        other["passive"]["interval_seconds"] = Value::from(9.0);
        v["hunters"]["ysolde"] = other;
        let table = parse(&v).unwrap();
        assert_eq!(table["loxelle"].lunge.windup, 0.42);
        assert_eq!(table["loxelle"].lunge.speed, 6.8);
        assert_eq!(table["loxelle"].passive.interval, 5.0);
        assert_eq!(table["ysolde"].lunge.windup, 0.90);
        assert_eq!(table["ysolde"].lunge.speed, 3.0);
        assert_eq!(table["ysolde"].passive.interval, 9.0);
        assert!(!table.contains_key("quenelle"));
    }

    /// Changing a value in the data changes the spec and the roll that decides a lunge.
    #[test]
    fn editing_the_data_changes_the_lunge_and_the_roll() {
        let base = parse(&doc(0.25, 0.42)).unwrap();
        let edited = parse(&doc(0.50, 0.10)).unwrap();
        let b = &base["loxelle"];
        let e = &edited["loxelle"];
        assert_eq!(b.lunge.windup, 0.42);
        assert_eq!(e.lunge.windup, 0.10);
        assert!(!b.passive.roll_lunges(300));
        assert!(e.passive.roll_lunges(300));
        assert!(b.passive.roll_lunges(249));
        assert!(!b.passive.roll_lunges(250));
    }

    #[test]
    fn bad_schema_is_rejected() {
        let mut v = doc(0.25, 0.42);
        v["schema"] = Value::String("lr.hunter/1".into());
        assert!(parse(&v).is_err());
    }
}
