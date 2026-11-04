//! Waterfalls from the first sewer down to the second (Godot _spawn_falls): a falling water sheet that
//! curls out over the lip and drops straight to the cistern, mist rising where it lands, a foam splash
//! with a cool pool light, and the key of the sound loop. Every number is data/render/waterfall.json.
//! This module only builds geometry (local coordinates, y up, `out` = -1 or +1 along z); drawing it
//! and playing the loop are hooked in by the scene.

use crate::content::embedded;
use std::sync::OnceLock;

pub const PATH: &str = "render/waterfall.json";

#[derive(Clone, Debug, PartialEq)]
pub struct MistCfg {
    pub count: usize,
    pub lifetime_s: f32,
    pub half_extents_m: [f32; 3],
    pub offset_out_m: f32,
    pub above_surface_m: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub gravity: f32,
    pub size_min_m: f32,
    pub size_max_m: f32,
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplashCfg {
    pub foam_size_m: [f32; 2],
    pub offset_out_m: f32,
    pub light_height_m: f32,
    pub light_color: [f32; 3],
    pub light_energy: f32,
    pub light_range_m: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaterfallCfg {
    pub sheet_width_m: f32,
    pub throw_m: f32,
    pub curl_drop_m: f32,
    pub curl_segments: usize,
    pub sheet_color: [f32; 4],
    pub flow_speed: f32,
    pub mist: MistCfg,
    pub splash: SplashCfg,
    pub sound_key: String,
    pub sound_range_m: f32,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .ok_or_else(|| format!("{key}: expected a number"))
}

fn arr<const N: usize>(v: &serde_json::Value, key: &str) -> Result<[f32; N], String> {
    let a = v
        .get(key)
        .and_then(|x| x.as_array())
        .filter(|a| a.len() == N)
        .ok_or_else(|| format!("{key}: expected {N} numbers"))?;
    let mut out = [0.0; N];
    for (i, x) in a.iter().enumerate() {
        out[i] = x.as_f64().ok_or_else(|| format!("{key}: expected numbers"))? as f32;
    }
    Ok(out)
}

impl WaterfallCfg {
    pub fn parse(text: &str) -> Result<WaterfallCfg, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let m = v.get("mist").ok_or("mist: missing")?;
        let s = v.get("splash").ok_or("splash: missing")?;
        let cfg = WaterfallCfg {
            sheet_width_m: num(&v, "sheet_width_m")?,
            throw_m: num(&v, "throw_m")?,
            curl_drop_m: num(&v, "curl_drop_m")?,
            curl_segments: num(&v, "curl_segments")? as usize,
            sheet_color: arr(&v, "sheet_color")?,
            flow_speed: num(&v, "flow_speed")?,
            mist: MistCfg {
                count: num(m, "count")? as usize,
                lifetime_s: num(m, "lifetime_s")?,
                half_extents_m: arr(m, "box_half_extents_m")?,
                offset_out_m: num(m, "offset_out_m")?,
                above_surface_m: num(m, "above_surface_m")?,
                speed_min: num(m, "speed_min")?,
                speed_max: num(m, "speed_max")?,
                gravity: num(m, "gravity")?,
                size_min_m: num(m, "size_min_m")?,
                size_max_m: num(m, "size_max_m")?,
                color: arr(m, "color")?,
            },
            splash: SplashCfg {
                foam_size_m: arr(s, "foam_size_m")?,
                offset_out_m: num(s, "offset_out_m")?,
                light_height_m: num(s, "light_height_m")?,
                light_color: arr(s, "light_color")?,
                light_energy: num(s, "light_energy")?,
                light_range_m: num(s, "light_range_m")?,
            },
            sound_key: v
                .get("sound_key")
                .and_then(|x| x.as_str())
                .filter(|k| !k.is_empty())
                .ok_or("sound_key: expected a non-empty string")?
                .to_string(),
            sound_range_m: num(&v, "sound_range_m")?,
        };
        if cfg.sheet_width_m <= 0.0 || cfg.curl_segments == 0 || cfg.mist.lifetime_s <= 0.0 {
            return Err("sheet width, curl segments and mist lifetime must be positive".into());
        }
        Ok(cfg)
    }
}

/// The waterfall tunables compiled in. A bad file is reported on stderr and the Standard values are used.
pub fn config() -> &'static WaterfallCfg {
    static CFG: OnceLock<WaterfallCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        WaterfallCfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            WaterfallCfg::parse(include_str!("../data/render/waterfall.json"))
                .expect("waterfall.json is valid")
        })
    })
}

/// One quad of the falling sheet: four corners [x, y, z] and the v texture coordinate at its top and bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetQuad {
    pub corners: [[f32; 3]; 4],
    pub v_top: f32,
    pub v_bottom: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Puff {
    pub pos: [f32; 3],
    pub size_m: f32,
    pub alpha: f32,
}

/// Everything drawn and played for one drop cell, in coordinates local to the lip's centre.
#[derive(Clone, Debug)]
pub struct Waterfall {
    pub sheet: Vec<SheetQuad>,
    pub mist: Vec<Puff>,
    pub foam_center: [f32; 3],
    pub light_pos: [f32; 3],
    pub sound_key: String,
    pub sound_pos: [f32; 3],
}

/// The sheet: a ballistic curl out from the lip (fast out, then down), then straight down to the cistern's water.
pub fn sheet(cfg: &WaterfallCfg, top_y: f32, bottom_y: f32, out: f32) -> Vec<SheetQuad> {
    let n = cfg.curl_segments;
    let mut rail: Vec<(f32, f32)> = (0..=n)
        .map(|i| {
            let k = i as f32 / n as f32;
            (cfg.throw_m * k.sqrt(), top_y - cfg.curl_drop_m * k)
        })
        .collect();
    rail.push((cfg.throw_m, bottom_y));
    let mut lengths = vec![0.0f32];
    for i in 1..rail.len() {
        let d = ((rail[i].0 - rail[i - 1].0).powi(2) + (rail[i].1 - rail[i - 1].1).powi(2)).sqrt();
        lengths.push(lengths[i - 1] + d);
    }
    let total = *lengths.last().unwrap();
    let h = cfg.sheet_width_m * 0.5;
    (1..rail.len())
        .map(|i| {
            let (a, b) = (rail[i - 1], rail[i]);
            SheetQuad {
                corners: [
                    [-h, a.1, a.0 * out],
                    [h, a.1, a.0 * out],
                    [h, b.1, b.0 * out],
                    [-h, b.1, b.0 * out],
                ],
                v_top: lengths[i - 1] / total,
                v_bottom: lengths[i] / total,
            }
        })
        .collect()
}

/// Mist at time `t` seconds: a fixed count of puffs, each on its own phase of the lifetime, rising from the
/// emission box with a slight gravity, fading in and out. Deterministic in (index, t).
pub fn mist(cfg: &WaterfallCfg, surface_y: f32, out: f32, t: f32) -> Vec<Puff> {
    let m = &cfg.mist;
    let hash = |i: usize, salt: u32| -> f32 {
        let mut x = (i as u32)
            .wrapping_mul(2654435761)
            .wrapping_add(salt.wrapping_mul(40503));
        x ^= x >> 15;
        x = x.wrapping_mul(2246822519);
        x ^= x >> 13;
        (x & 0xffff) as f32 / 65535.0
    };
    let base_z = out * (cfg.throw_m + m.offset_out_m);
    (0..m.count)
        .map(|i| {
            let phase = (t / m.lifetime_s + i as f32 / m.count as f32).rem_euclid(1.0);
            let age = phase * m.lifetime_s;
            let speed = m.speed_min + (m.speed_max - m.speed_min) * hash(i, 1);
            let rise = speed * age + 0.5 * m.gravity * age * age;
            let size = m.size_min_m + (m.size_max_m - m.size_min_m) * hash(i, 2);
            let fade = (phase * (1.0 - phase) * 4.0).clamp(0.0, 1.0);
            Puff {
                pos: [
                    (hash(i, 3) * 2.0 - 1.0) * m.half_extents_m[0],
                    surface_y
                        + m.above_surface_m
                        + (hash(i, 4) * 2.0 - 1.0) * m.half_extents_m[1]
                        + rise,
                    base_z + (hash(i, 5) * 2.0 - 1.0) * m.half_extents_m[2],
                ],
                size_m: size,
                alpha: m.color[3] * fade,
            }
        })
        .collect()
}

/// Builds the waterfall for a drop cell: `top_y` is the first sewer's water surface, `bottom_y` the cistern's
/// surface a storey down, `out` -1 (north) or +1 (south), `t` the clock in seconds.
pub fn build(cfg: &WaterfallCfg, top_y: f32, bottom_y: f32, out: f32, t: f32) -> Waterfall {
    let foam_z = out * (cfg.throw_m + cfg.splash.offset_out_m);
    Waterfall {
        sheet: sheet(cfg, top_y, bottom_y, out),
        mist: mist(cfg, bottom_y, out, t),
        foam_center: [0.0, bottom_y + 0.03, foam_z],
        light_pos: [0.0, bottom_y + cfg.splash.light_height_m, foam_z],
        sound_key: cfg.sound_key.clone(),
        sound_pos: [
            0.0,
            bottom_y + cfg.mist.above_surface_m,
            out * (cfg.throw_m + cfg.mist.offset_out_m),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WaterfallCfg {
        WaterfallCfg::parse(include_str!("../data/render/waterfall.json")).unwrap()
    }

    #[test]
    fn embedded_file_parses_with_standard_values() {
        let c = config();
        assert_eq!(c.throw_m, 0.9);
        assert_eq!(c.curl_drop_m, 0.7);
        assert_eq!(c.mist.count, 22);
        assert_eq!(c.mist.lifetime_s, 1.8);
        assert_eq!(c.sheet_width_m, 3.0);
        assert!(!c.sound_key.is_empty());
    }

    #[test]
    fn sheet_curls_out_then_drops_to_the_pool() {
        let c = cfg();
        let q = sheet(&c, 0.0, -3.0, 1.0);
        assert_eq!(q.len(), c.curl_segments + 1);
        assert_eq!(q[0].corners[0][2], 0.0);
        let last = q.last().unwrap();
        assert_eq!(last.corners[2][1], -3.0);
        assert_eq!(last.corners[2][2], c.throw_m);
        assert!((last.v_bottom - 1.0).abs() < 1e-5);
        let north = sheet(&c, 0.0, -3.0, -1.0);
        assert_eq!(north.last().unwrap().corners[2][2], -c.throw_m);
    }

    #[test]
    fn mist_stays_in_range_and_moves() {
        let c = cfg();
        let a = mist(&c, -3.0, 1.0, 0.0);
        let b = mist(&c, -3.0, 1.0, 0.4);
        assert_eq!(a.len(), c.mist.count);
        assert_ne!(a[0].pos, b[0].pos);
        for p in a.iter().chain(b.iter()) {
            assert!(p.alpha >= 0.0 && p.alpha <= c.mist.color[3]);
            assert!(p.size_m >= c.mist.size_min_m && p.size_m <= c.mist.size_max_m);
            assert!(p.pos[0].abs() <= c.mist.half_extents_m[0] + 1e-4);
        }
    }

    #[test]
    fn data_changes_behaviour() {
        let mut c = cfg();
        c.mist.count = 5;
        c.throw_m = 2.0;
        c.sound_key = "gush".into();
        let w = build(&c, 0.0, -3.0, 1.0, 0.0);
        assert_eq!(w.mist.len(), 5);
        assert_eq!(w.sheet.last().unwrap().corners[2][2], 2.0);
        assert_eq!(w.sound_key, "gush");
    }

    #[test]
    fn splash_sits_at_the_landing_pool() {
        let c = cfg();
        let w = build(&c, 0.0, -3.0, 1.0, 0.0);
        assert!(w.foam_center[1] > -3.0 && w.foam_center[1] < -2.9);
        assert!(w.light_pos[1] > w.foam_center[1]);
    }

    #[test]
    fn bad_tables_are_rejected() {
        assert!(WaterfallCfg::parse("{}").is_err());
        assert!(WaterfallCfg::parse("nope").is_err());
    }
}
