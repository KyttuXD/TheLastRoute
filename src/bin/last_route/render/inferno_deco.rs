//! Inferno caves decoration (level -3): lava glow, embers, stalactites and stalagmites, charred props and
//! floor cracks. Pure placement: given a carved chunk it returns where each piece goes and how it looks. The
//! rules and colours come from data/render/inferno.json (compiled in by build.rs); the code only places.

use std::sync::OnceLock;

use crate::world::facility::Cell;
use crate::world::infinite::{ChunkKey, CHUNK};
use crate::world::inferno::{InfernoChunk, Tier};

const PATH: &str = "render/inferno.json";

/// The decoration kinds, in the order they are placed (so the output order is stable).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecoKind {
    LavaGlow,
    Ember,
    Stalactite,
    Stalagmite,
    CharredProp,
    FloorCrack,
}

const KINDS: [(DecoKind, &str); 6] = [
    (DecoKind::LavaGlow, "lava_glow"),
    (DecoKind::Ember, "ember"),
    (DecoKind::Stalactite, "stalactite"),
    (DecoKind::Stalagmite, "stalagmite"),
    (DecoKind::CharredProp, "charred_prop"),
    (DecoKind::FloorCrack, "floor_crack"),
];

/// One kind's placement rules and look.
#[derive(Clone, Debug)]
pub struct KindRule {
    pub tiers: Vec<Tier>,
    /// Chance per eligible open cell, 0..=1.
    pub chance: f32,
    /// Cells (Chebyshev) kept clear between two pieces of this kind.
    pub min_spacing: i32,
    pub max_per_chunk: usize,
    pub size_m: [f32; 2],
    pub color: [f32; 3],
    pub glow_radius_m: f32,
    pub glow_strength: f32,
    pub flicker_hz: f32,
    pub rise_speed_mps: [f32; 2],
    pub life_s: [f32; 2],
}

/// All kinds' rules.
#[derive(Clone, Debug)]
pub struct InfernoDeco {
    pub rules: Vec<(DecoKind, KindRule)>,
}

/// One placed piece. `cell` is chunk-local.
#[derive(Clone, Copy, Debug)]
pub struct Deco {
    pub kind: DecoKind,
    pub cell: Cell,
    /// Offset inside the cell, in cells from its centre, each -0.5..0.5.
    pub offset: [f32; 2],
    pub size_m: f32,
    pub yaw: f32,
    pub color: [f32; 3],
    /// Floor height under the piece: the carve's tier height from `place` (0 on a ledge, negative in the basin), set to
    /// the noisy terrain height by the window placement and the mesh.
    pub floor_y: f32,
    /// Per-piece phase for flicker and ember timing, 0..1.
    pub phase: f32,
}

impl InfernoDeco {
    /// Parses `{ "kinds": { "<name>": { tiers, chance, min_spacing, max_per_chunk, size_m, color, ... } } }`.
    /// Every kind must be present; the optional glow and ember fields default to zero.
    pub fn parse(text: &str) -> Result<InfernoDeco, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let kinds = root.get("kinds").ok_or("kinds: missing")?;
        let mut rules = Vec::new();
        for (kind, name) in KINDS {
            let v = kinds.get(name).ok_or_else(|| format!("{name}: missing"))?;
            rules.push((kind, parse_rule(name, v)?));
        }
        Ok(InfernoDeco { rules })
    }

    pub fn rule(&self, kind: DecoKind) -> Option<&KindRule> {
        self.rules.iter().find(|(k, _)| *k == kind).map(|(_, r)| r)
    }

    /// Places every piece for a carved chunk. Deterministic from `seed` and `key`; nothing lands on a closed cell.
    pub fn place(&self, seed: &str, key: ChunkKey, inf: &InfernoChunk) -> Vec<Deco> {
        let mut out: Vec<Deco> = Vec::new();
        let open = inf.open_cells();
        for (kind, rule) in &self.rules {
            let mut kept: Vec<Cell> = Vec::new();
            for &c in &open {
                if kept.len() >= rule.max_per_chunk {
                    break;
                }
                // the stair bay is the runner's way down: no stalagmite, lava glow or crack stands on the flight's foot
                if inf.in_bay(c) {
                    continue;
                }
                if !rule.tiers.contains(&inf.tier_at(c)) {
                    continue;
                }
                let h = hash(seed, key, c, *kind as i32);
                if unit(h, 0) >= rule.chance {
                    continue;
                }
                if kept.iter().any(|k| (k.x - c.x).abs().max((k.y - c.y).abs()) < rule.min_spacing) {
                    continue;
                }
                kept.push(c);
                out.push(Deco {
                    kind: *kind,
                    cell: c,
                    offset: [unit(h, 1) - 0.5, unit(h, 2) - 0.5],
                    size_m: lerp(rule.size_m, unit(h, 3)),
                    yaw: unit(h, 4) * std::f32::consts::TAU,
                    color: rule.color,
                    floor_y: inf.height_at(c),
                    phase: unit(h, 5),
                });
            }
        }
        out
    }
}

fn parse_rule(name: &str, v: &serde_json::Value) -> Result<KindRule, String> {
    let num = |key: &str, default: Option<f32>| -> Result<f32, String> {
        match v.get(key) {
            Some(x) => x
                .as_f64()
                .filter(|n| *n >= 0.0)
                .map(|n| n as f32)
                .ok_or_else(|| format!("{name}.{key}: expected a number, zero or more")),
            None => default.ok_or_else(|| format!("{name}.{key}: missing")),
        }
    };
    let pair = |key: &str, required: bool| -> Result<[f32; 2], String> {
        match v.get(key).and_then(|a| a.as_array()) {
            Some(a) if a.len() == 2 => {
                let lo = a[0].as_f64().filter(|n| *n >= 0.0);
                let hi = a[1].as_f64().filter(|n| *n >= 0.0);
                match (lo, hi) {
                    (Some(lo), Some(hi)) if lo <= hi => Ok([lo as f32, hi as f32]),
                    _ => Err(format!("{name}.{key}: expected [low, high], zero or more")),
                }
            }
            Some(_) => Err(format!("{name}.{key}: expected [low, high]")),
            None if required => Err(format!("{name}.{key}: missing")),
            None => Ok([0.0, 0.0]),
        }
    };
    let tiers = v
        .get("tiers")
        .and_then(|t| t.as_array())
        .ok_or_else(|| format!("{name}.tiers: missing"))?
        .iter()
        .map(|t| match t.as_str() {
            Some("ledge") => Ok(Tier::Ledge),
            Some("slope") => Ok(Tier::Slope),
            Some("basin") => Ok(Tier::Basin),
            _ => Err(format!("{name}.tiers: expected ledge, slope or basin")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let color = v
        .get("color")
        .and_then(|c| c.as_array())
        .filter(|c| c.len() == 3)
        .and_then(|c| {
            let ch: Vec<f32> = c
                .iter()
                .filter_map(|x| x.as_f64().filter(|n| (0.0..=1.0).contains(n)).map(|n| n as f32))
                .collect();
            (ch.len() == 3).then(|| [ch[0], ch[1], ch[2]])
        })
        .ok_or_else(|| format!("{name}.color: expected [r, g, b], each 0 to 1"))?;
    let chance = num("chance", None)?;
    if chance > 1.0 {
        return Err(format!("{name}.chance: expected 0 to 1"));
    }
    Ok(KindRule {
        tiers,
        chance,
        min_spacing: num("min_spacing", None)? as i32,
        max_per_chunk: num("max_per_chunk", None)? as usize,
        size_m: pair("size_m", true)?,
        color,
        glow_radius_m: num("glow_radius_m", Some(0.0))?,
        glow_strength: num("glow_strength", Some(0.0))?,
        flicker_hz: num("flicker_hz", Some(0.0))?,
        rise_speed_mps: pair("rise_speed_mps", false)?,
        life_s: pair("life_s", false)?,
    })
}

/// The decoration rules compiled into the executable. A table that fails to parse is reported on stderr and
/// nothing is decorated.
pub fn inferno_deco() -> &'static InfernoDeco {
    static DECO: OnceLock<InfernoDeco> = OnceLock::new();
    DECO.get_or_init(|| {
        let text = crate::content::embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        InfernoDeco::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            InfernoDeco { rules: Vec::new() }
        })
    })
}

/// Flicker multiplier for a glow at time `t_s`: 1 plus or minus a fifth.
pub fn flicker(t_s: f32, phase: f32, hz: f32) -> f32 {
    (1.0 + 0.2 * ((t_s * hz + phase) * std::f32::consts::TAU).sin()).max(0.0)
}

fn lerp(r: [f32; 2], t: f32) -> f32 {
    r[0] + (r[1] - r[0]) * t
}

/// FNV-1a over the seed text, the chunk, the cell and the kind.
fn hash(seed: &str, key: ChunkKey, c: Cell, kind: i32) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    };
    seed.bytes().for_each(&mut eat);
    for v in [key.cx, key.cy, key.level, c.x, c.y, kind] {
        v.to_le_bytes().into_iter().for_each(&mut eat);
    }
    h
}

/// The n-th independent 0..1 value drawn from a hash.
fn unit(h: u64, n: u64) -> f32 {
    let mut z = h.wrapping_add(n.wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// How the scene shows the glowing kinds (the numbers are in the `scene` block of data/render/inferno.json).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCfg {
    /// A glow further than this from the eye (along the floor) casts no light.
    pub glow_cull_m: f32,
    /// An ember further than this is not drawn.
    pub ember_cull_m: f32,
    /// Height of a lava glow's light above its floor.
    pub glow_height_m: f32,
    /// Height of a floor crack's light above its floor.
    pub crack_height_m: f32,
    /// Height above the floor where an ember starts to rise.
    pub ember_start_m: f32,
    /// How far an ember wanders sideways by the end of its life.
    pub ember_drift_m: f32,
}

impl SceneCfg {
    pub fn parse(text: &str) -> Result<SceneCfg, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = root.get("scene").ok_or("scene: missing")?;
        let num = |key: &str| -> Result<f32, String> {
            s.get(key)
                .and_then(|x| x.as_f64())
                .filter(|n| *n >= 0.0)
                .map(|n| n as f32)
                .ok_or_else(|| format!("scene.{key}: expected a number, zero or more"))
        };
        Ok(SceneCfg {
            glow_cull_m: num("glow_cull_m")?,
            ember_cull_m: num("ember_cull_m")?,
            glow_height_m: num("glow_height_m")?,
            crack_height_m: num("crack_height_m")?,
            ember_start_m: num("ember_start_m")?,
            ember_drift_m: num("ember_drift_m")?,
        })
    }
}

/// The scene numbers compiled into the executable. A table that fails to parse is reported on stderr and the
/// glowing kinds are not shown (every distance is zero).
pub fn scene_cfg() -> &'static SceneCfg {
    static CFG: OnceLock<SceneCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = crate::content::embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        SceneCfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            SceneCfg { glow_cull_m: 0.0, ember_cull_m: 0.0, glow_height_m: 0.0, crack_height_m: 0.0, ember_start_m: 0.0, ember_drift_m: 0.0 }
        })
    })
}

/// How the lava pit surface looks and moves (the `lava_surface` block of data/render/inferno.json). A table without the
/// block draws a still, plain surface (`LavaLook::still`).
#[derive(Clone, Debug, PartialEq)]
pub struct LavaLook {
    /// Surface colour at the hottest and coolest point of the pulse (0..1 rgb).
    pub hot: [f32; 3],
    pub cool: [f32; 3],
    /// Pulses per second; 0 is still.
    pub pulse_hz: f32,
    /// Texture scroll in cells per second along x and z; 0 is still.
    pub flow_cps: [f32; 2],
    /// Colour of the pit walls and bed under the surface.
    pub pit_color: [f32; 3],
}

impl LavaLook {
    pub fn still() -> LavaLook {
        LavaLook { hot: [1.0, 0.45, 0.1], cool: [0.8, 0.25, 0.05], pulse_hz: 0.0, flow_cps: [0.0, 0.0], pit_color: [0.15, 0.05, 0.03] }
    }

    pub fn parse(text: &str) -> Result<LavaLook, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = root.get("lava_surface").ok_or("lava_surface: missing")?;
        let col = |k: &str| -> Result<[f32; 3], String> {
            let a = s.get(k).and_then(|x| x.as_array()).filter(|a| a.len() == 3).ok_or_else(|| format!("lava_surface.{k}: expected 3 numbers"))?;
            let mut o = [0.0; 3];
            for (i, v) in a.iter().enumerate() {
                o[i] = v.as_f64().filter(|n| (0.0..=1.0).contains(n)).ok_or_else(|| format!("lava_surface.{k}[{i}]: expected 0..1"))? as f32;
            }
            Ok(o)
        };
        let num = |k: &str| s.get(k).and_then(|x| x.as_f64()).filter(|n| *n >= 0.0).map(|n| n as f32).ok_or_else(|| format!("lava_surface.{k}: expected a number, zero or more"));
        Ok(LavaLook { hot: col("hot")?, cool: col("cool")?, pulse_hz: num("pulse_hz")?, flow_cps: [num("flow_x_cps")?, num("flow_z_cps")?], pit_color: col("pit_color")? })
    }

    /// The surface colour at time `t_s` for a point with pulse `phase` (0..1): between `cool` and `hot`.
    pub fn surface_color(&self, t_s: f32, phase: f32) -> [f32; 3] {
        let k = 0.5 + 0.5 * ((t_s * self.pulse_hz + phase) * std::f32::consts::TAU).sin();
        [0, 1, 2].map(|i| self.cool[i] + (self.hot[i] - self.cool[i]) * k)
    }

    /// The texture offset (cells) the surface has flowed by at time `t_s`.
    pub fn uv_offset(&self, t_s: f32) -> [f32; 2] {
        [(t_s * self.flow_cps[0]).rem_euclid(1.0), (t_s * self.flow_cps[1]).rem_euclid(1.0)]
    }
}

/// The lava look compiled into the executable (`LavaLook::still` when the block is missing or bad).
pub fn lava_look() -> &'static LavaLook {
    static LOOK: OnceLock<LavaLook> = OnceLock::new();
    LOOK.get_or_init(|| {
        let text = crate::content::embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        LavaLook::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            LavaLook::still()
        })
    })
}

/// One quad of the lava pit in window metres. A horizontal one has its corners in the order of the floor quads
/// ((x, z), (x+1, z), (x+1, z+1), (x, z+1)); `normal` faces the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitQuad {
    pub corners: [[f32; 3]; 4],
    pub normal: [f32; 3],
}

/// The geometry of one chunk's lava pits: the glowing surface at `floor.lava_y`, the bed at `floor.lava_basin_y` and the
/// pit walls (open, from the bed up to the walkable floor) on every side that faces a cell which is not lava.
#[derive(Clone, Debug, Default)]
pub struct LavaPit {
    pub surface: Vec<PitQuad>,
    pub bed: Vec<PitQuad>,
    pub walls: Vec<PitQuad>,
}

/// Builds the pit geometry of the chunk `inf`. `origin` is the chunk's corner in window metres (x, z); `lava_at` tells
/// whether a chunk-local cell (which may lie outside the chunk, for the neighbour across an edge) is lava, so a pit that
/// runs over a chunk edge gets no wall in its middle.
pub fn lava_pit(inf: &InfernoChunk, origin: [f32; 2], lava_at: &dyn Fn(i32, i32) -> bool) -> LavaPit {
    let r = crate::world::inferno::InfernoRules::builtin();
    let (top, surf, bed) = (r.walk_height, r.lava_y, r.lava_basin_y);
    let mut out = LavaPit::default();
    for c in inf.lava_cells() {
        let (x, z) = (origin[0] + c.x as f32, origin[1] + c.y as f32);
        let flat = |y: f32| [[x, y, z], [x + 1., y, z], [x + 1., y, z + 1.], [x, y, z + 1.]];
        out.surface.push(PitQuad { corners: flat(surf), normal: [0., 1., 0.] });
        out.bed.push(PitQuad { corners: flat(bed), normal: [0., 1., 0.] });
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if lava_at(c.x + dx, c.y + dz) {
                continue;
            }
            // the shared edge, its two ends in order along the edge
            let (a, b) = match (dx, dz) {
                (-1, 0) => ([x, z], [x, z + 1.]),
                (1, 0) => ([x + 1., z], [x + 1., z + 1.]),
                (0, -1) => ([x, z], [x + 1., z]),
                _ => ([x, z + 1.], [x + 1., z + 1.]),
            };
            out.walls.push(PitQuad {
                corners: [[a[0], bed, a[1]], [b[0], bed, b[1]], [b[0], top, b[1]], [a[0], top, a[1]]],
                normal: [-dx as f32, 0., -dz as f32],
            });
        }
    }
    out
}

/// A placed piece in window metres: `pos` is (x, floor height, z).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub deco: Deco,
    pub pos: [f32; 3],
}

/// Every piece of the 3x3 window of level -3 around `center` (the streamed window's centre chunk), placed in window
/// metres (the window's (0, 0) is the corner of chunk (centre - 1)).
pub fn window_decos(deco: &InfernoDeco, seed: &str, center: (i32, i32)) -> Vec<Placed> {
    let mut out = Vec::new();
    // the window's noisy floor (world/inferno_terrain.rs): every piece stands on it
    let terrain = crate::world::inferno_terrain::Terrain::new(seed, ((center.0 - 1) * CHUNK, (center.1 - 1) * CHUNK), 3);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let key = ChunkKey { cx: center.0 + dx, cy: center.1 + dy, level: crate::world::infinite::INFERNO_LEVEL };
            let inf = InfernoChunk::carve(seed, key);
            let (ox, oy) = (((dx + 1) * CHUNK) as f32, ((dy + 1) * CHUNK) as f32);
            for mut d in deco.place(seed, key, &inf) {
                let (px, pz) = (ox + d.cell.x as f32 + 0.5 + d.offset[0], oy + d.cell.y as f32 + 0.5 + d.offset[1]);
                d.floor_y = terrain.floor_at(px, pz);
                let pos = [px, d.floor_y, pz];
                out.push(Placed { deco: d, pos });
            }
        }
    }
    out
}

type WindowKey = (String, (i32, i32));

thread_local! {
    static WINDOW: std::cell::RefCell<Option<(WindowKey, Vec<Placed>)>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` on the window's pieces, placed once while the window stays the same (the scene asks every frame).
pub fn with_window_decos<R>(seed: &str, center: (i32, i32), f: impl FnOnce(&[Placed]) -> R) -> R {
    WINDOW.with(|c| {
        let mut c = c.borrow_mut();
        let fresh = matches!(&*c, Some(((s, ce), _)) if s == seed && *ce == center);
        if !fresh {
            *c = Some(((seed.to_string(), center), window_decos(inferno_deco(), seed, center)));
        }
        f(&c.as_ref().expect("just set").1)
    })
}

/// A point light of a glowing piece.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub range_m: f32,
    pub energy: f32,
}

fn near(eye: [f32; 3], p: [f32; 3], within: f32) -> bool {
    (p[0] - eye[0]).powi(2) + (p[2] - eye[2]).powi(2) <= within * within
}

/// The point lights of the lava glows (flickering at their `flicker_hz`) and the floor cracks (their steady
/// `glow_strength`), for pieces within `glow_cull_m` of the eye, at time `t_s`. A kind with no radius or no
/// strength in the data casts no light.
pub fn glows(placed: &[Placed], eye: [f32; 3], t_s: f32, deco: &InfernoDeco, cfg: &SceneCfg) -> Vec<Glow> {
    let mut out = Vec::new();
    for p in placed {
        let (height, steady) = match p.deco.kind {
            DecoKind::LavaGlow => (cfg.glow_height_m, false),
            DecoKind::FloorCrack => (cfg.crack_height_m, true),
            _ => continue,
        };
        let Some(rule) = deco.rule(p.deco.kind) else { continue };
        if rule.glow_radius_m <= 0.0 || rule.glow_strength <= 0.0 || !near(eye, p.pos, cfg.glow_cull_m) {
            continue;
        }
        let flick = if steady || rule.flicker_hz <= 0.0 { 1.0 } else { flicker(t_s, p.deco.phase, rule.flicker_hz) };
        out.push(Glow {
            pos: [p.pos[0], p.pos[1] + height, p.pos[2]],
            color: p.deco.color,
            range_m: rule.glow_radius_m,
            energy: rule.glow_strength * flick,
        });
    }
    out
}

/// One ember on the wing: its place, size and colour. It shrinks as it burns out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ember {
    pub pos: [f32; 3],
    pub size_m: f32,
    pub color: [f32; 3],
}

/// The embers at time `t_s`: each rises from its piece at a speed and over a life drawn from its rule's ranges (by
/// its phase), drifts sideways a little, shrinks to nothing and starts again. Only those within `ember_cull_m`.
pub fn embers(placed: &[Placed], eye: [f32; 3], t_s: f32, deco: &InfernoDeco, cfg: &SceneCfg) -> Vec<Ember> {
    let Some(rule) = deco.rule(DecoKind::Ember) else { return Vec::new() };
    if rule.life_s[1] <= 0.0 {
        return Vec::new();
    }
    placed
        .iter()
        .filter(|p| p.deco.kind == DecoKind::Ember && near(eye, p.pos, cfg.ember_cull_m))
        .map(|p| {
            let ph = p.deco.phase;
            let life = lerp(rule.life_s, ph).max(0.01);
            let speed = lerp(rule.rise_speed_mps, (ph * 7.31).fract());
            let burn = ((t_s / life) + ph).rem_euclid(1.0);
            let age = burn * life;
            let drift = (t_s * 1.3 + ph * std::f32::consts::TAU).sin() * cfg.ember_drift_m * burn;
            Ember {
                pos: [p.pos[0] + drift, p.pos[1] + cfg.ember_start_m + speed * age, p.pos[2] + drift * 0.5],
                size_m: p.deco.size_m * (1.0 - burn),
                color: p.deco.color,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::INFERNO_LEVEL;

    const SEED: &str = "deco-seed";

    fn lava_chunk() -> InfernoChunk {
        for cx in 0..8 {
            for cy in 0..8 {
                let inf = InfernoChunk::carve(SEED, ChunkKey { cx, cy, level: INFERNO_LEVEL });
                if !inf.lava_cells().is_empty() {
                    return inf;
                }
            }
        }
        panic!("no lava in the sample");
    }

    #[test]
    fn a_lava_cell_is_not_a_wall_in_the_drawn_grid() {
        let inf = lava_chunk();
        for c in inf.lava_cells() {
            assert!(!inf.draws_wall(c, true), "lava draws no wall mass");
        }
        let c = inf.open_cells()[0];
        assert!(inf.draws_wall(c, true), "a plain wall cell still draws");
        assert!(!inf.draws_wall(c, false));
    }

    #[test]
    fn the_lava_surface_has_quads_at_the_pit_height_and_the_walls_are_open() {
        let inf = lava_chunk();
        let r = crate::world::inferno::InfernoRules::builtin();
        let pit = lava_pit(&inf, [0.0, 0.0], &|x, y| inf.is_lava(Cell::new(x, y)));
        assert_eq!(pit.surface.len(), inf.lava_cells().len());
        assert_eq!(pit.bed.len(), pit.surface.len());
        assert!(pit.surface.iter().all(|q| q.corners.iter().all(|p| p[1] == r.lava_y)));
        assert!(pit.bed.iter().all(|q| q.corners.iter().all(|p| p[1] == r.lava_basin_y)));
        assert!(r.lava_basin_y < r.lava_y && r.lava_y < r.walk_height);
        assert!(!pit.walls.is_empty(), "a pit has a rim");
        for w in &pit.walls {
            let hi = w.corners.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            let lo = w.corners.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            assert_eq!((hi, lo), (r.walk_height, r.lava_basin_y));
        }
    }

    #[test]
    fn a_pit_edge_inside_lava_has_no_wall() {
        let inf = lava_chunk();
        assert!(lava_pit(&inf, [0.0, 0.0], &|_, _| true).walls.is_empty());
        assert_eq!(lava_pit(&inf, [0.0, 0.0], &|_, _| false).walls.len(), inf.lava_cells().len() * 4);
    }

    #[test]
    fn lava_surface_pulses_between_cool_and_hot_and_flows() {
        let look = LavaLook { hot: [1.0, 0.5, 0.2], cool: [0.5, 0.2, 0.0], pulse_hz: 1.0, flow_cps: [0.5, 0.0], pit_color: [0.1; 3] };
        for i in 0..20 {
            let c = look.surface_color(i as f32 * 0.13, 0.3);
            assert!((0..3).all(|k| c[k] >= look.cool[k] - 1e-5 && c[k] <= look.hot[k] + 1e-5));
        }
        let still = LavaLook::still();
        assert_eq!(still.surface_color(3.0, 0.2), still.surface_color(0.0, 0.2), "no pulse is still");
        assert_eq!(look.uv_offset(0.0), [0.0, 0.0]);
        assert!(look.uv_offset(0.5)[0] > 0.0 && look.uv_offset(0.5)[1] == 0.0);
        let text = r#"{"lava_surface":{"hot":[1,0.5,0.2],"cool":[0.5,0.2,0],"pulse_hz":1,"flow_x_cps":0.5,"flow_z_cps":0,"pit_color":[0.1,0.1,0.1]}}"#;
        assert_eq!(LavaLook::parse(text).unwrap().pulse_hz, 1.0);
        assert!(LavaLook::parse("{}").is_err());
    }

    fn key(cx: i32, cy: i32) -> ChunkKey {
        ChunkKey { cx, cy, level: INFERNO_LEVEL }
    }

    fn placed(deco: &InfernoDeco, cx: i32, cy: i32) -> (InfernoChunk, Vec<Deco>) {
        let inf = InfernoChunk::carve(SEED, key(cx, cy));
        let d = deco.place(SEED, key(cx, cy), &inf);
        (inf, d)
    }

    #[test]
    fn standard_data_parses_and_places_every_kind_somewhere() {
        let deco = inferno_deco();
        assert_eq!(deco.rules.len(), KINDS.len());
        let mut seen = std::collections::HashSet::new();
        for cx in 0..4 {
            for cy in 0..4 {
                for d in placed(deco, cx, cy).1 {
                    seen.insert(d.kind as i32);
                }
            }
        }
        assert_eq!(seen.len(), KINDS.len(), "every kind appears across a few chunks");
    }

    #[test]
    fn placement_is_deterministic_and_stays_on_open_cells() {
        let deco = inferno_deco();
        let (inf, a) = placed(deco, 1, 2);
        let (_, b) = placed(deco, 1, 2);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!((x.kind, x.cell, x.size_m), (y.kind, y.cell, y.size_m));
        }
        for d in &a {
            assert!(inf.open_cells().contains(&d.cell));
            assert_eq!(d.floor_y, inf.height_at(d.cell), "the piece sits on the same flat floor the open cells use");
            assert_eq!(d.floor_y, crate::world::inferno::InfernoRules::builtin().walk_height);
            assert!(d.cell.x < CHUNK && d.cell.y < CHUNK);
        }
    }

    #[test]
    fn rules_hold_tier_spacing_count_and_size() {
        let deco = inferno_deco();
        let (inf, all) = placed(deco, 0, 0);
        for (kind, rule) in &deco.rules {
            let mine: Vec<&Deco> = all.iter().filter(|d| d.kind == *kind).collect();
            assert!(mine.len() <= rule.max_per_chunk);
            for d in &mine {
                assert!(rule.tiers.contains(&inf.tier_at(d.cell)));
                assert!(d.size_m >= rule.size_m[0] && d.size_m <= rule.size_m[1]);
                assert_eq!(d.color, rule.color);
            }
            for (i, a) in mine.iter().enumerate() {
                for b in &mine[i + 1..] {
                    assert!((a.cell.x - b.cell.x).abs().max((a.cell.y - b.cell.y).abs()) >= rule.min_spacing);
                }
            }
        }
    }

    #[test]
    fn lava_glow_sits_in_the_basin_and_stalagmites_stay_out_of_it() {
        let deco = inferno_deco();
        let (inf, all) = placed(deco, 0, 1);
        for d in all {
            match d.kind {
                DecoKind::LavaGlow => assert_eq!(inf.tier_at(d.cell), Tier::Basin),
                DecoKind::Stalagmite => assert_ne!(inf.tier_at(d.cell), Tier::Basin),
                _ => {}
            }
        }
    }

    #[test]
    fn changing_the_data_changes_the_placement() {
        // CRLF checkouts (Windows CI) must match the same text, so compare on LF.
        let text = include_str!("../data/render/inferno.json").replace("\r\n", "\n");
        let off_text = text.replacen("\"chance\": 0.05,\n      \"min_spacing\": 2", "\"chance\": 0.0,\n      \"min_spacing\": 2", 1);
        assert_ne!(off_text, text);
        let off = InfernoDeco::parse(&off_text).unwrap();
        let on = InfernoDeco::parse(&text).unwrap();
        let inf = InfernoChunk::carve(SEED, key(0, 0));
        let count = |d: &InfernoDeco| d.place(SEED, key(0, 0), &inf).iter().filter(|x| x.kind == DecoKind::Stalactite).count();
        assert_eq!(count(&off), 0);
        assert!(count(&on) > 0);
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(InfernoDeco::parse("{}").is_err());
        assert!(InfernoDeco::parse("[1]").is_err());
        let text = include_str!("../data/render/inferno.json");
        assert!(InfernoDeco::parse(&text.replacen("[\"basin\"]", "[\"moon\"]", 1)).is_err());
        assert!(InfernoDeco::parse(&text.replacen("[1.0, 0.42, 0.08]", "[2.0, 0.4, 0.1]", 1)).is_err());
        assert!(InfernoDeco::parse(&text.replacen("[1.2, 2.4]", "[3.0, 1.0]", 1)).is_err());
    }

    fn window() -> Vec<Placed> {
        window_decos(inferno_deco(), SEED, (2, 2))
    }

    #[test]
    fn the_window_places_every_chunk_inside_its_span() {
        let w = window();
        assert!(!w.is_empty());
        let span = 3.0 * CHUNK as f32;
        for p in &w {
            assert!(p.pos[0] >= 0.0 && p.pos[0] < span && p.pos[2] >= 0.0 && p.pos[2] < span);
        }
        let again = with_window_decos(SEED, (2, 2), |p| p.len());
        assert_eq!(again, w.len(), "the cached window is the placed one");
        assert_eq!(with_window_decos(SEED, (2, 2), |p| p.len()), w.len());
    }

    /// A placed piece of `kind` at window (x, z) on a floor at y -1, with the rule's own colour.
    fn piece(kind: DecoKind, x: f32, z: f32, phase: f32) -> Placed {
        let rule = inferno_deco().rule(kind).unwrap();
        Placed {
            deco: Deco {
                kind,
                cell: Cell::new(0, 0),
                offset: [0.0, 0.0],
                size_m: rule.size_m[1],
                yaw: 0.0,
                color: rule.color,
                floor_y: -1.0,
                phase,
            },
            pos: [x, -1.0, z],
        }
    }

    #[test]
    fn lava_glows_flicker_and_cracks_stay_steady() {
        let (deco, cfg) = (inferno_deco(), scene_cfg());
        let lava = deco.rule(DecoKind::LavaGlow).unwrap();
        let crack = deco.rule(DecoKind::FloorCrack).unwrap();
        let placed = [piece(DecoKind::LavaGlow, 10.0, 10.0, 0.1), piece(DecoKind::FloorCrack, 12.0, 10.0, 0.1), piece(DecoKind::Stalactite, 11.0, 10.0, 0.1)];
        let eye = [10.0, 1.6, 10.0];
        let a = glows(&placed, eye, 0.0, deco, cfg);
        let b = glows(&placed, eye, 0.2, deco, cfg);
        assert_eq!(a.len(), 2, "only glowing kinds light");
        assert_eq!((a[0].range_m, a[0].color), (lava.glow_radius_m, lava.color));
        assert!(a[0].pos[1] > -1.0, "the light hangs above the floor");
        assert_ne!(a[0].energy, b[0].energy, "lava flickers with time");
        for g in [a[0], b[0]] {
            assert!(g.energy >= lava.glow_strength * 0.8 - 1e-4 && g.energy <= lava.glow_strength * 1.2 + 1e-4);
        }
        assert_eq!((a[1].energy, b[1].energy), (crack.glow_strength, crack.glow_strength), "cracks do not");
        assert_eq!(a[1].range_m, crack.glow_radius_m);
    }

    #[test]
    fn glows_obey_the_cull_and_the_data() {
        let (deco, cfg) = (inferno_deco(), scene_cfg());
        let placed = [piece(DecoKind::LavaGlow, 10.0, 10.0, 0.0), piece(DecoKind::LavaGlow, 10.0 + cfg.glow_cull_m + 5.0, 10.0, 0.0)];
        let eye = [10.0, 1.6, 10.0];
        assert_eq!(glows(&placed, eye, 0.0, deco, cfg).len(), 1, "the far one casts no light");
        let mut tight = cfg.clone();
        tight.glow_cull_m = 0.5;
        assert_eq!(glows(&placed, [30.0, 1.6, 10.0], 0.0, deco, &tight).len(), 0);
        let dark = InfernoDeco::parse(&include_str!("../data/render/inferno.json").replace("\"glow_strength\": 1.4", "\"glow_strength\": 0.0")).unwrap();
        assert!(glows(&placed, eye, 0.0, &dark, cfg).is_empty(), "no strength, no light");
    }

    #[test]
    fn embers_rise_shrink_and_start_again() {
        let (deco, cfg) = (inferno_deco(), scene_cfg());
        let rule = deco.rule(DecoKind::Ember).unwrap();
        let placed = [piece(DecoKind::Ember, 10.0, 10.0, 0.25), piece(DecoKind::Ember, 11.0, 10.0, 0.6), piece(DecoKind::LavaGlow, 12.0, 10.0, 0.6)];
        let eye = [10.0, 1.6, 10.0];
        let a = embers(&placed, eye, 0.0, deco, cfg);
        assert_eq!(a.len(), 2, "only embers rise");
        let reach = cfg.ember_start_m + rule.rise_speed_mps[1] * rule.life_s[1];
        let mut last = [f32::MIN; 2];
        let mut restarted = false;
        for step in 0..400 {
            let t = step as f32 * 0.05;
            for (i, e) in embers(&placed, eye, t, deco, cfg).iter().enumerate() {
                assert!(e.size_m >= 0.0 && e.size_m <= rule.size_m[1] + 1e-6);
                assert!(e.pos[1] >= -1.0 + cfg.ember_start_m - 1e-4 && e.pos[1] <= -1.0 + reach + 1e-3);
                if e.pos[1] < last[i] {
                    restarted = true;
                } else if last[i] > f32::MIN {
                    assert!(e.pos[1] >= last[i], "it climbs between restarts");
                }
                last[i] = e.pos[1];
            }
        }
        assert!(restarted, "an ember that burns out starts again at the floor");
        assert!(embers(&placed, [9000.0, 0.0, 9000.0], 0.0, deco, cfg).is_empty());
    }

    #[test]
    fn scene_numbers_are_data() {
        let text = include_str!("../data/render/inferno.json");
        let c = SceneCfg::parse(text).unwrap();
        assert_eq!(&c, scene_cfg());
        assert!(SceneCfg::parse("{}").is_err());
        let wider = SceneCfg::parse(&text.replacen("\"glow_cull_m\": 36.0", "\"glow_cull_m\": 60.0", 1)).unwrap();
        assert!(wider.glow_cull_m > c.glow_cull_m);
    }

    #[test]
    fn flicker_stays_near_one() {
        for i in 0..50 {
            let f = flicker(i as f32 * 0.1, 0.3, 1.3);
            assert!((0.8..=1.2).contains(&f));
        }
    }
}
