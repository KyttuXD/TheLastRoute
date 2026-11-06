//! Ambient light and fog per place (data/render/ambient.json, `lr.ambient/1`): one baseline, with optional rows for a
//! region (`region.<id>`) or a level (`level.<n>`). A region row wins over a level row, and a key a row leaves out falls
//! back to the baseline, so the file starts as one row and a place gains its own look by adding a row.

use crate::content::embedded;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const PATH: &str = "render/ambient.json";
const SCHEMA: &str = "lr.ambient/1";

/// The ambient and fog constants for one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub ambient_color: [f32; 3],
    pub ambient_energy: f32,
    pub fog_color: [f32; 3],
    pub fog_far: f32,
    /// Metres to full fog for the ambient light alone: past it a surface no lamp or beam lights is black. Much shorter
    /// than `fog_far` in the maze, so the dark shows a few cells; lamps fade out at `fog_far`, the beam at the flashlight's
    /// `fog_reach` (see `Flashlight`).
    pub dark_far: f32,
}

/// One override row: the keys it sets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Row {
    ambient_color: Option<[f32; 3]>,
    ambient_energy: Option<f32>,
    fog_color: Option<[f32; 3]>,
    fog_far: Option<f32>,
    dark_far: Option<f32>,
}

/// The held flashlight's numbers (`flashlight` in the file). The shader lights a surface in the cone with
/// `intensity x (1 - d / range)^2` and fades that light out at `fog_reach` metres, a longer reach than the dark view of
/// the place (`Look::dark_far`), so the beam shows what the ambient cannot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flashlight {
    /// Metres the beam throws.
    pub range: f32,
    /// The beam's strength, a multiplier on the cone light.
    pub intensity: f32,
    /// Metres to full fog for the beam-lit light.
    pub fog_reach: f32,
}

/// The beam's numbers when the file names none (and the fallback if it fails to load). Godot's player flashlight is a
/// SpotLight3D of range 16 and energy 7.5 (main_3d.tscn); this port's cone falls off as a plain square, which at 16 m
/// leaves the far half of the throw nearly dark, so the nominal range is longer to give the same readable throw.
pub const FLASHLIGHT_FALLBACK: Flashlight = Flashlight { range: 26.0, intensity: 2.4, fog_reach: 30.0 };

impl Flashlight {
    /// The strength of the cone light `dist` metres down the beam's axis (the shader's `intensity x att^2`).
    pub fn beam_light(&self, dist: f32) -> f32 {
        let att = (1.0 - dist / self.range).clamp(0.0, 1.0);
        self.intensity * att * att
    }

    /// The farthest a surface on the beam's axis is lit at `min_light` or more (the throw a player can read by),
    /// capped by the beam's fog reach. Zero when the beam is never that strong.
    pub fn lit_reach(&self, min_light: f32) -> f32 {
        if min_light <= 0.0 {
            return self.range.min(self.fog_reach);
        }
        if min_light >= self.intensity {
            return 0.0;
        }
        (self.range * (1.0 - (min_light / self.intensity).sqrt())).min(self.fog_reach)
    }
}

/// How much of a surface `dist` metres off shows through the fog that is full at `far` (the shader's `1 - (d / far)^2`).
pub fn fog_visibility(dist: f32, far: f32) -> f32 {
    let f = (dist / far.max(1e-3)).clamp(0.0, 1.0);
    1.0 - f * f
}

/// The distance at which the fog leaves `min_visibility` (0..1) of a surface: how far the dark view reaches.
pub fn view_reach(far: f32, min_visibility: f32) -> f32 {
    far * (1.0 - min_visibility.clamp(0.0, 1.0)).sqrt()
}

/// The parsed table.
#[derive(Clone, Debug, PartialEq)]
pub struct Ambient {
    default: Look,
    regions: HashMap<String, Row>,
    levels: HashMap<i32, Row>,
    /// Time constant of the blend toward a new place's look, in seconds (0: the look jumps).
    blend_seconds: f32,
    flashlight: Flashlight,
}

/// Seconds the blend takes when the file names none.
const DEFAULT_BLEND_SECONDS: f32 = 0.5;
/// A gap between calls longer than this (a pause, a load) restarts the blend at the target.
const MAX_GAP: f32 = 1.0;

impl Look {
    /// This look moved toward `target` by `t` (0..1) of the way, every constant alike.
    pub fn lerp(self, target: Look, t: f32) -> Look {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        let mix3 = |a: [f32; 3], b: [f32; 3]| [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])];
        Look {
            ambient_color: mix3(self.ambient_color, target.ambient_color),
            ambient_energy: mix(self.ambient_energy, target.ambient_energy),
            fog_color: mix3(self.fog_color, target.fog_color),
            fog_far: mix(self.fog_far, target.fog_far),
            dark_far: mix(self.dark_far, target.dark_far),
        }
    }
}

impl Ambient {
    /// The look at a place: the region's row, else the level's row, key by key over the default.
    pub fn look(&self, region: &str, level: i32) -> Look {
        let region = self.regions.get(region);
        let level = self.levels.get(&level);
        let pick = |f: fn(&Row) -> Option<f32>, d: f32| region.and_then(f).or_else(|| level.and_then(f)).unwrap_or(d);
        let pick3 = |f: fn(&Row) -> Option<[f32; 3]>, d: [f32; 3]| region.and_then(f).or_else(|| level.and_then(f)).unwrap_or(d);
        let d = self.default;
        Look {
            ambient_color: pick3(|r| r.ambient_color, d.ambient_color),
            ambient_energy: pick(|r| r.ambient_energy, d.ambient_energy),
            fog_color: pick3(|r| r.fog_color, d.fog_color),
            fog_far: pick(|r| r.fog_far, d.fog_far),
            dark_far: pick(|r| r.dark_far, d.dark_far),
        }
    }

    /// The held flashlight's numbers.
    pub fn flashlight(&self) -> Flashlight {
        self.flashlight
    }

    /// The look the eye shows at `now` (game seconds) for a place whose own look is `target`: the look of the last call
    /// eased toward it with the file's time constant, so a region border or a stair does not pop. Calls in the same
    /// frame (`now` unchanged) return the same look; the first call, or one after a long gap, starts at `target`.
    pub fn blended(&self, target: Look, now: f32) -> Look {
        thread_local! {
            static SHOWN: std::cell::Cell<Option<(Look, f32)>> = const { std::cell::Cell::new(None) };
        }
        let look = match SHOWN.with(|s| s.get()) {
            Some((_, at)) if now == at => return SHOWN.with(|s| s.get()).map_or(target, |(l, _)| l),
            Some((prev, at)) if (0.0..=MAX_GAP).contains(&(now - at)) && self.blend_seconds > 0.0 => {
                prev.lerp(target, 1.0 - (-(now - at) / self.blend_seconds).exp())
            }
            _ => target,
        };
        SHOWN.with(|s| s.set(Some((look, now))));
        look
    }

    pub fn parse(text: &str) -> Result<Ambient, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let obj = v.as_object().ok_or("expected an object")?;
        if obj.get("schema").and_then(|s| s.as_str()) != Some(SCHEMA) {
            return Err(format!("schema: expected {SCHEMA}"));
        }
        let default = parse_row(obj.get("default").ok_or("default: missing")?).map_err(|e| format!("default: {e}"))?;
        let fog_far = need(default.fog_far, "fog_far")?;
        let default = Look {
            ambient_color: need(default.ambient_color, "ambient_color")?,
            ambient_energy: need(default.ambient_energy, "ambient_energy")?,
            fog_color: need(default.fog_color, "fog_color")?,
            fog_far,
            // a file with no dark reach keeps the ambient as far as the lamps
            dark_far: default.dark_far.unwrap_or(fog_far),
        };
        let blend_seconds = match obj.get("blend_seconds") {
            None => DEFAULT_BLEND_SECONDS,
            Some(n) => n.as_f64().filter(|n| *n >= 0.0).ok_or("blend_seconds: expected a number of 0 or more")? as f32,
        };
        let flashlight = match obj.get("flashlight") {
            None => FLASHLIGHT_FALLBACK,
            Some(f) => parse_flashlight(f).map_err(|e| format!("flashlight: {e}"))?,
        };
        let mut out = Ambient { default, regions: HashMap::new(), levels: HashMap::new(), blend_seconds, flashlight };
        for (key, row) in obj {
            if key.starts_with('_') || ["schema", "default", "blend_seconds", "flashlight"].contains(&key.as_str()) {
                continue;
            }
            let row_value = parse_row(row).map_err(|e| format!("{key}: {e}"))?;
            if let Some(id) = key.strip_prefix("region.").filter(|id| !id.is_empty()) {
                out.regions.insert(id.to_string(), row_value);
            } else if let Some(n) = key.strip_prefix("level.").and_then(|n| n.parse::<i32>().ok()) {
                out.levels.insert(n, row_value);
            } else {
                return Err(format!("{key}: expected `region.<id>` or `level.<n>`"));
            }
        }
        Ok(out)
    }
}

fn need<T>(o: Option<T>, k: &str) -> Result<T, String> {
    o.ok_or_else(|| format!("default.{k}: missing"))
}

fn parse_row(v: &Value) -> Result<Row, String> {
    let o = v.as_object().ok_or("expected an object")?;
    for k in o.keys() {
        if !["ambient_color", "ambient_energy", "fog_color", "fog_far", "dark_far"].contains(&k.as_str()) {
            return Err(format!("unknown key {k}"));
        }
    }
    let colour = |k: &str| o.get(k).map(|c| parse_colour(c).ok_or_else(|| format!("{k}: expected \"#rrggbb\" or [r, g, b] in 0..1"))).transpose();
    let num = |k: &str, min: f64| {
        o.get(k)
            .map(|n| n.as_f64().filter(|n| *n >= min).map(|n| n as f32).ok_or_else(|| format!("{k}: expected a number of {min} or more")))
            .transpose()
    };
    Ok(Row {
        ambient_color: colour("ambient_color")?,
        ambient_energy: num("ambient_energy", 0.0)?,
        fog_color: colour("fog_color")?,
        fog_far: num("fog_far", 0.1)?,
        dark_far: num("dark_far", 0.1)?,
    })
}

fn parse_flashlight(v: &Value) -> Result<Flashlight, String> {
    let o = v.as_object().ok_or("expected an object")?;
    for k in o.keys() {
        if !["range", "intensity", "fog_reach"].contains(&k.as_str()) {
            return Err(format!("unknown key {k}"));
        }
    }
    let num = |k: &str, min: f64, d: f32| match o.get(k) {
        None => Ok(d),
        Some(n) => n.as_f64().filter(|n| *n >= min).map(|n| n as f32).ok_or_else(|| format!("{k}: expected a number of {min} or more")),
    };
    Ok(Flashlight {
        range: num("range", 0.1, FLASHLIGHT_FALLBACK.range)?,
        intensity: num("intensity", 0.0, FLASHLIGHT_FALLBACK.intensity)?,
        fog_reach: num("fog_reach", 0.1, FLASHLIGHT_FALLBACK.fog_reach)?,
    })
}

fn parse_colour(v: &Value) -> Option<[f32; 3]> {
    if let Some(s) = v.as_str() {
        let h = s.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii())?;
        let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|b| b as f32 / 255.0);
        return Some([ch(0)?, ch(2)?, ch(4)?]);
    }
    let a = v.as_array().filter(|a| a.len() == 3)?;
    let mut out = [0.0; 3];
    for (o, c) in out.iter_mut().zip(a) {
        *o = c.as_f64().filter(|c| (0.0..=1.0).contains(c))? as f32;
    }
    Some(out)
}

/// The table compiled into the executable. One that fails to parse is reported on stderr and the old constants stand.
pub fn table() -> &'static Ambient {
    static T: OnceLock<Ambient> = OnceLock::new();
    T.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Ambient::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Ambient {
                default: Look {
                    ambient_color: [0.352941, 0.207843, 0.313725],
                    ambient_energy: 0.35,
                    fog_color: [0.012, 0.008, 0.014],
                    fog_far: 24.0,
                    dark_far: 24.0,
                },
                regions: HashMap::new(),
                levels: HashMap::new(),
                blend_seconds: DEFAULT_BLEND_SECONDS,
                flashlight: FLASHLIGHT_FALLBACK,
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = include_str!("../data/render/ambient.json");

    fn with(extra: &str) -> Ambient {
        Ambient::parse(&FILE.replacen("\"default\"", &format!("{extra}\"default\""), 1)).unwrap()
    }

    #[test]
    fn the_shipped_file_parses_and_is_the_compiled_table() {
        let t = Ambient::parse(FILE).unwrap();
        assert_eq!(&t, table());
        let l = t.look("maze", 0);
        assert_eq!(l.ambient_energy, 0.30);
        for (a, b) in l.ambient_color.iter().zip([0.352941, 0.207843, 0.313725]) {
            assert!((a - b).abs() < 1e-5);
        }
        assert_eq!((l.fog_color, l.fog_far, l.dark_far), ([0.012, 0.008, 0.014], 24.0, 8.0));
    }

    #[test]
    fn the_sewers_and_the_inferno_keep_their_old_reach() {
        let t = Ambient::parse(FILE).unwrap();
        for level in [-1, -2, -3] {
            let l = t.look("anywhere", level);
            assert_eq!(l.dark_far, l.fog_far, "level {level}: the ambient reaches as far as the lamps, as before");
        }
        assert_eq!(t.look("inferno", -3).ambient_energy, 0.35);
        assert_eq!(t.look("inferno", -3).fog_far, 24.0);
    }

    #[test]
    fn the_dark_is_short_in_level_0_and_lamps_keep_their_reach() {
        let t = Ambient::parse(FILE).unwrap();
        for r in ["maze", "hospital", "mall", "office", "mansion", "unlisted"] {
            let l = t.look(r, 0);
            assert!(l.dark_far <= 13.0, "{r}: the ambient alone shows 13 m at most");
            assert!(l.fog_far >= 2.0 * l.dark_far, "{r}: lamps and lit rooms reach far past the dark view");
            assert!(l.fog_far >= 18.0, "{r}: a lit room keeps its reach");
        }
        // the maze shows only a few cells (a cell is a metre): half of the surface is gone by 6 m
        let maze = t.look("maze", 0);
        assert!(fog_visibility(6.0, maze.dark_far) < 0.5 && fog_visibility(2.0, maze.dark_far) > 0.9);
        assert_eq!(fog_visibility(maze.dark_far, maze.dark_far), 0.0);
    }

    #[test]
    fn the_beam_reveals_far_past_the_dark_view() {
        let t = Ambient::parse(FILE).unwrap();
        let beam = t.flashlight();
        // a surface counts as readable at 0.1 of full light (the ambient of a lit-room edge)
        let thrown = beam.lit_reach(0.1);
        for r in ["maze", "hospital", "mall", "office", "mansion"] {
            let dark = view_reach(t.look(r, 0).dark_far, 0.25);
            assert!(thrown > 1.5 * dark, "{r}: the beam reads at {thrown} m, the dark at {dark} m");
        }
        assert!(thrown > 2.0 * view_reach(t.look("maze", 0).dark_far, 0.25), "in the maze the beam shows over twice as far");
        // and the beam's fog is longer than every level 0 dark view, so it is the fog that does not hide the throw
        assert!(beam.fog_reach > t.look("mall", 0).dark_far * 2.0);
        assert!(beam.range >= 16.0, "at least Godot's 16 m throw");
        assert!(beam.fog_reach <= 1.6 * t.look("maze", 0).fog_far, "the beam never outreaches the world cull");
    }

    #[test]
    fn reach_arithmetic() {
        let f = Flashlight { range: 20.0, intensity: 2.0, fog_reach: 30.0 };
        assert_eq!(f.beam_light(0.0), 2.0);
        assert_eq!(f.beam_light(20.0), 0.0);
        assert_eq!(f.beam_light(40.0), 0.0, "past the range there is no light");
        assert!((f.beam_light(10.0) - 0.5).abs() < 1e-6);
        // the reach that lights to 0.5 is the distance whose light is 0.5
        let r = f.lit_reach(0.5);
        assert!((f.beam_light(r) - 0.5).abs() < 1e-5, "{r}");
        assert_eq!(f.lit_reach(2.0), 0.0, "a beam never that strong reaches nowhere");
        assert_eq!(f.lit_reach(5.0), 0.0);
        assert_eq!(f.lit_reach(0.0), 20.0);
        assert_eq!(Flashlight { fog_reach: 5.0, ..f }.lit_reach(0.01), 5.0, "the fog caps the throw");
        assert_eq!(fog_visibility(0.0, 10.0), 1.0);
        assert_eq!(fog_visibility(10.0, 10.0), 0.0);
        assert_eq!(fog_visibility(99.0, 10.0), 0.0);
        assert!((fog_visibility(5.0, 10.0) - 0.75).abs() < 1e-6);
        let reach = view_reach(10.0, 0.19);
        assert!((fog_visibility(reach, 10.0) - 0.19).abs() < 1e-5);
        assert_eq!(view_reach(10.0, 0.0), 10.0);
        assert_eq!(view_reach(10.0, 1.0), 0.0);
    }

    #[test]
    fn the_flashlight_section_loads_and_defaults() {
        let t = with("\"flashlight\": {\"range\": 40.0, \"intensity\": 3.0, \"fog_reach\": 50.0},");
        assert_eq!(t.flashlight(), Flashlight { range: 40.0, intensity: 3.0, fog_reach: 50.0 });
        // the shipped file has its own section; one without a section, or with a part of it, falls back key by key
        let bare = FILE.replacen("\"flashlight\"", "\"_flashlight\"", 1);
        assert_eq!(Ambient::parse(&bare).unwrap().flashlight(), FLASHLIGHT_FALLBACK);
        let part = with("\"flashlight\": {\"range\": 12.0},");
        assert_eq!(part.flashlight().range, 12.0);
        assert_eq!(part.flashlight().intensity, FLASHLIGHT_FALLBACK.intensity);
        assert_eq!(Ambient::parse(FILE).unwrap().flashlight(), table().flashlight());
    }

    #[test]
    fn a_missing_dark_far_falls_back_to_the_default_then_to_fog_far() {
        let t = with("\"region.x\": {\"fog_far\": 40.0},");
        assert_eq!(t.look("x", 0).dark_far, t.look("maze", 0).dark_far, "the default row's dark reach");
        let old = r##"{"schema":"lr.ambient/1","default":{"ambient_color":"#5a3550","ambient_energy":0.35,"fog_color":[0.01,0.01,0.01],"fog_far":24.0}}"##;
        let l = Ambient::parse(old).unwrap().look("maze", 0);
        assert_eq!(l.dark_far, 24.0, "an old file keeps the ambient as far as the lamps");
    }

    #[test]
    fn the_sewer_levels_are_greener_darker_and_tighter() {
        let t = Ambient::parse(FILE).unwrap();
        let base = t.look("maze", 0);
        for level in [-1, -2] {
            let l = t.look("sewer", level);
            assert!(l.ambient_energy < base.ambient_energy && l.fog_far < base.fog_far, "level {level}");
            assert!(l.ambient_color[1] > l.ambient_color[0] && l.ambient_color[1] > l.ambient_color[2], "level {level} leans green");
        }
        assert!(t.look("sewer", -2).ambient_energy <= t.look("sewer", -1).ambient_energy);
    }

    #[test]
    fn region_beats_level_and_missing_keys_fall_back() {
        let t = with(
            "\"level.-4\": {\"fog_far\": 10.0, \"ambient_energy\": 1.0}, \"region.testzone\": {\"ambient_energy\": 0.5, \"ambient_color\": [1, 0, 0]},",
        );
        let base = t.look("unlisted", 0);
        let l = t.look("testzone", -4);
        assert_eq!((l.ambient_energy, l.ambient_color), (0.5, [1.0, 0.0, 0.0]));
        assert_eq!(l.fog_far, 10.0, "the level row fills what the region row leaves out");
        assert_eq!(l.fog_color, base.fog_color, "and the default fills the rest");
        assert_eq!(t.look("unlisted", -4).ambient_energy, 1.0);
        assert_eq!(t.look("unlisted", 1), base);
    }

    #[test]
    fn level_0_regions_each_have_their_own_look_and_maze_is_the_baseline() {
        let t = Ambient::parse(FILE).unwrap();
        let base = t.look("unlisted", 0);
        assert_eq!(t.look("maze", 0), base);
        let mut seen = vec![];
        for r in ["hospital", "mall", "office", "mansion"] {
            let l = t.look(r, 0);
            assert_ne!(l, base, "{r} has a row");
            assert!(l.ambient_energy > 0.0 && l.fog_far > 0.0);
            seen.push(l.ambient_color.map(f32::to_bits));
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 4, "four different ambient colours");
        let (h, m) = (t.look("hospital", 0).ambient_color, t.look("mall", 0).ambient_color);
        assert!(h[2] > h[0], "the hospital is cold");
        assert!(m[0] > m[2], "the mall is warm");
    }

    #[test]
    fn the_look_eases_toward_a_new_place_and_snaps_after_a_gap() {
        let t = with("\"blend_seconds\": 0.5,");
        let (a, b) = (t.look("hospital", 0), t.look("mansion", 0));
        assert_eq!(t.blended(a, 10.0), a, "the first call starts at the target");
        assert_eq!(t.blended(b, 10.0), a, "the same frame shows the same look");
        let step = t.blended(b, 10.1);
        assert!(step.fog_far < a.fog_far && step.fog_far > b.fog_far, "partway: {}", step.fog_far);
        let mut look = step;
        for i in 2..80 {
            look = t.blended(b, 10.0 + i as f32 * 0.1);
        }
        assert!((look.fog_far - b.fog_far).abs() < 0.01 && (look.ambient_energy - b.ambient_energy).abs() < 1e-3);
        assert_eq!(t.blended(a, 50.0), a, "a long gap restarts at the target");
        let instant = with("\"blend_seconds\": 0,");
        instant.blended(a, 1.0);
        assert_eq!(instant.blended(b, 1.1), b, "zero jumps");
        assert!(Ambient::parse(&FILE.replacen("\"default\"", "\"blend_seconds\": -1, \"default\"", 1)).is_err());
    }

    #[test]
    fn bad_files_are_refused() {
        assert!(Ambient::parse("{}").is_err());
        assert!(Ambient::parse(&FILE.replace("lr.ambient/1", "lr.ambient/2")).is_err());
        assert!(Ambient::parse(&FILE.replace("\"fog_far\": 24.0", "\"fog_far\": 0.0")).is_err());
        assert!(Ambient::parse(&FILE.replace("#5a3550", "#5a35")).is_err());
        assert!(Ambient::parse(&FILE.replace("\"fog_far\": 24.0", "\"fog_far\": 24.0, \"bogus\": 1")).is_err());
        let no_fog = FILE.replace("\"fog_far\": 24.0", "\"fog_x\": 1");
        assert!(Ambient::parse(&no_fog).is_err());
        assert!(Ambient::parse(&FILE.replacen("\"default\"", "\"floor.1\": {}, \"default\"", 1)).is_err());
        assert!(Ambient::parse(&FILE.replace("\"dark_far\": 8.0", "\"dark_far\": 0.0")).is_err());
        assert!(Ambient::parse(&FILE.replace("\"intensity\": 2.4", "\"intensity\": -1.0")).is_err());
        assert!(Ambient::parse(&FILE.replace("\"fog_reach\": 30.0", "\"fog_reach\": 0.0")).is_err());
        assert!(Ambient::parse(&FILE.replace("\"intensity\": 2.4", "\"intensity\": 2.4, \"colour\": 1")).is_err());
    }
}
