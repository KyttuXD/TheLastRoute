//! The world material: a near-black ambient (rooms are dark unless something lights them),
//! up to MAX_LAMPS coloured point lights (switchable room lights and lamp props), the held
//! flashlight's cone, and distance fog. Vertex `normal.w` selects the mode:
//! 0 tiled texture (fract UVs), 1 atlas texture, 2 unlit/emissive.

use std::cell::RefCell;

use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, UniformType,
};
use macroquad::prelude::*;

pub const MAX_LAMPS: usize = 16;

const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;
varying lowp vec2 uv;
varying lowp vec4 color;
varying highp vec3 wpos;
varying mediump vec4 nrm;
uniform mat4 Model;
uniform mat4 Projection;
void main() {
    vec4 w = Model * vec4(position, 1.0);
    gl_Position = Projection * w;
    wpos = w.xyz;
    uv = texcoord;
    color = color0 / 255.0;
    nrm = normal;
}"#;

const FRAGMENT: &str = r#"#version 100
precision highp float;
varying lowp vec2 uv;
varying lowp vec4 color;
varying highp vec3 wpos;
varying mediump vec4 nrm;
uniform sampler2D Texture;
uniform vec3 CamPos;
uniform vec3 LightPos;
uniform vec3 LightDir;
uniform float LightOn;
uniform vec3 AmbientColor;
uniform vec3 FogColor;
uniform float FogFar;
uniform vec4 Lamps[16];
uniform vec4 LampColors[16];
void main() {
    float mode = nrm.w;
    vec2 tuv = mode < 0.5 ? fract(uv) : uv;
    vec4 tex = texture2D(Texture, tuv) * color;
    if (tex.a < 0.5) discard;
    float fd = length(wpos - CamPos);
    if (mode > 1.5) {
        float f = clamp(fd / (FogFar * 1.6), 0.0, 1.0);
        gl_FragColor = vec4(mix(tex.rgb, FogColor, f * f), 1.0);
        return;
    }
    vec3 n = normalize(nrm.xyz);
    vec3 light = AmbientColor;
    for (int i = 0; i < 16; i++) {
        float range = Lamps[i].w;
        if (range > 0.0) {
            vec3 d = Lamps[i].xyz - wpos;
            float dist = length(d) + 0.001;
            float att = clamp(1.0 - dist / range, 0.0, 1.0);
            float ndl = max(dot(n, d / dist), 0.0) * 0.7 + 0.3;
            light += LampColors[i].rgb * LampColors[i].a * att * att * ndl;
        }
    }
    if (LightOn > 0.5) {
        vec3 d = wpos - LightPos;
        float dist = length(d) + 0.001;
        vec3 dir = d / dist;
        float c = dot(dir, LightDir);
        float cone = smoothstep(0.80, 0.93, c) + 0.6 * smoothstep(0.955, 0.985, c);
        float att = clamp(1.0 - dist / 16.0, 0.0, 1.0);
        float ndl = max(dot(n, -dir), 0.0);
        light += vec3(1.0, 0.95, 0.86) * cone * att * att * (0.25 + 0.75 * ndl) * 1.9;
    }
    vec3 col = tex.rgb * light;
    float f = clamp(fd / FogFar, 0.0, 1.0);
    gl_FragColor = vec4(mix(col, FogColor, f * f), 1.0);
}"#;

pub fn load_world_material() -> Material {
    load_material(
        ShaderSource::Glsl {
            vertex: VERTEX,
            fragment: FRAGMENT,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                depth_write: true,
                depth_test: Comparison::LessOrEqual,
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::Value(BlendValue::SourceAlpha),
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            uniforms: vec![
                UniformDesc::new("CamPos", UniformType::Float3),
                UniformDesc::new("LightPos", UniformType::Float3),
                UniformDesc::new("LightDir", UniformType::Float3),
                UniformDesc::new("LightOn", UniformType::Float1),
                UniformDesc::new("AmbientColor", UniformType::Float3),
                UniformDesc::new("FogColor", UniformType::Float3),
                UniformDesc::new("FogFar", UniformType::Float1),
                UniformDesc::new("Lamps", UniformType::Float4).array(MAX_LAMPS),
                UniformDesc::new("LampColors", UniformType::Float4).array(MAX_LAMPS),
            ],
            ..Default::default()
        },
    )
    .expect("world shader")
}

/// A light in the world: position, reach, colour and energy.
#[derive(Clone, Copy)]
pub struct Lamp {
    pub pos: Vec3,
    pub range: f32,
    pub color: Color,
    pub energy: f32,
}

/// The global ambient, Godot's RefLook environment (game/look/ref_look.gd make_environment): colour #5a3550 at
/// energy AMBIENT_ENERGY. Godot has no player setting for it and no per-region or per-level ambient (the night-watch
/// office moods are their own scenes), so neither does this. Lit surfaces get `AMBIENT_COLOR * AMBIENT_ENERGY` added
/// before the lamps and the flashlight. Godot's 0.035 leaves this shader's rooms black, so the port uses 0.35: a faint
/// plum floor and walls away from the lamps, with the flashlight still the main light.
pub const AMBIENT_COLOR: Vec3 = Vec3::new(0.352941, 0.207843, 0.313725);
pub const AMBIENT_ENERGY: f32 = 0.35;

pub struct Lighting {
    pub eye: Vec3,
    pub flashlight_pos: Vec3,
    pub flashlight_dir: Vec3,
    pub flashlight_on: bool,
    /// The ambient light's colour already multiplied by its energy (AMBIENT_COLOR * AMBIENT_ENERGY).
    pub ambient: Vec3,
    pub fog: Vec3,
    pub fog_far: f32,
}

/// The lamps nearest the eye, packed for the shader: `pos` holds (x, y, z, range) and `col` holds (r, g, b, energy),
/// nearest first. Slots past the lamp count stay zero, which the shader reads as "no lamp" (range 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearLamps {
    pos: [Vec4; MAX_LAMPS],
    col: [Vec4; MAX_LAMPS],
}

/// The up to MAX_LAMPS lamps nearest `eye`, nearest first. Equal distances keep their input order (stable sort).
pub fn nearest_lamps(eye: Vec3, lamps: &[Lamp]) -> NearLamps {
    let mut near: Vec<(f32, &Lamp)> = lamps
        .iter()
        .map(|lp| ((lp.pos - eye).length_squared(), lp))
        .collect();
    near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = NearLamps {
        pos: [Vec4::ZERO; MAX_LAMPS],
        col: [Vec4::ZERO; MAX_LAMPS],
    };
    for (i, (_, lp)) in near.iter().take(MAX_LAMPS).enumerate() {
        out.pos[i] = vec4(lp.pos.x, lp.pos.y, lp.pos.z, lp.range);
        out.col[i] = vec4(lp.color.r, lp.color.g, lp.color.b, lp.energy);
    }
    out
}

/// True when two lamp lists hold the same lamps in the same order.
fn same_lamps(a: &[Lamp], b: &[Lamp]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.pos == y.pos
                && x.range == y.range
                && x.energy == y.energy
                && x.color.r == y.color.r
                && x.color.g == y.color.g
                && x.color.b == y.color.b
                && x.color.a == y.color.a
        })
}

/// The last `nearest_lamps` result and the inputs it came from.
struct LampCache {
    eye: Vec3,
    lamps: Vec<Lamp>,
    near: NearLamps,
}

thread_local! {
    static LAMP_CACHE: RefCell<Option<LampCache>> = const { RefCell::new(None) };
}

/// `nearest_lamps`, reused while the eye and the lamp list are unchanged. Every material is drawn with the same
/// lighting in a frame, so the sort runs once per frame rather than once per material. Any change to the eye or to
/// a lamp (position, reach, colour or energy, or order) is a miss, so the result never goes stale.
fn cached_nearest_lamps(eye: Vec3, lamps: &[Lamp]) -> NearLamps {
    LAMP_CACHE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(hit) = &*slot {
            if hit.eye == eye && same_lamps(&hit.lamps, lamps) {
                return hit.near;
            }
        }
        let near = nearest_lamps(eye, lamps);
        *slot = Some(LampCache {
            eye,
            lamps: lamps.to_vec(),
            near,
        });
        near
    })
}

/// Uploads the lighting for this frame, keeping the lamps nearest the eye.
pub fn apply(mat: &Material, l: &Lighting, lamps: &[Lamp]) {
    mat.set_uniform("CamPos", l.eye);
    mat.set_uniform("LightPos", l.flashlight_pos);
    mat.set_uniform("LightDir", l.flashlight_dir);
    mat.set_uniform("LightOn", if l.flashlight_on { 1.0f32 } else { 0.0 });
    mat.set_uniform("AmbientColor", l.ambient);
    mat.set_uniform("FogColor", l.fog);
    mat.set_uniform("FogFar", l.fog_far);
    let near = cached_nearest_lamps(l.eye, lamps);
    mat.set_uniform_array("Lamps", &near.pos[..]);
    mat.set_uniform_array("LampColors", &near.col[..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp_at(x: f32, range: f32) -> Lamp {
        Lamp {
            pos: vec3(x, 0.0, 0.0),
            range,
            color: Color::new(1.0, 0.5, 0.25, 1.0),
            energy: 2.0,
        }
    }

    #[test]
    fn nearest_lamps_sort_by_distance_with_ties_in_input_order() {
        // the range is the lamp's id; distances from the eye at the origin are 5, 1, 3, 1 and 2
        let lamps = [
            lamp_at(5.0, 1.0),
            lamp_at(-1.0, 2.0),
            lamp_at(3.0, 3.0),
            lamp_at(1.0, 4.0),
            lamp_at(-2.0, 5.0),
        ];
        let near = nearest_lamps(Vec3::ZERO, &lamps);
        let ids: Vec<f32> = (0..lamps.len()).map(|i| near.pos[i].w).collect();
        // the two lamps at distance 1 keep their input order, then 2, 3 and 5
        assert_eq!(ids, vec![2.0, 4.0, 5.0, 3.0, 1.0]);
        assert_eq!(near.col[0], vec4(1.0, 0.5, 0.25, 2.0));
        assert_eq!(near.pos[0], vec4(-1.0, 0.0, 0.0, 2.0));
        // unused slots stay zero
        assert_eq!(near.pos[lamps.len()], Vec4::ZERO);
        assert_eq!(near.col[lamps.len()], Vec4::ZERO);
    }

    #[test]
    fn nearest_lamps_keep_the_sixteen_nearest_in_order() {
        // twenty lamps, input farthest first: lamp j sits at x = 19 - j and has range j + 1
        let lamps: Vec<Lamp> = (0..20)
            .map(|j| lamp_at((19 - j) as f32, (j + 1) as f32))
            .collect();
        let near = nearest_lamps(Vec3::ZERO, &lamps);
        // x = 0 is nearest (range 20), x = 15 is the sixteenth (range 5)
        assert_eq!(near.pos[0].w, 20.0);
        assert_eq!(near.pos[MAX_LAMPS - 1].w, 5.0);
        for i in 0..MAX_LAMPS {
            assert_eq!(near.pos[i].x, i as f32);
        }
    }

    #[test]
    fn cached_lamps_match_a_fresh_sort_and_follow_changes() {
        let eye = Vec3::ZERO;
        let a = [lamp_at(4.0, 1.0), lamp_at(1.0, 2.0)];
        assert_eq!(cached_nearest_lamps(eye, &a), nearest_lamps(eye, &a));
        // the same inputs again are served from the cache and still match
        assert_eq!(cached_nearest_lamps(eye, &a), nearest_lamps(eye, &a));
        // same length, one lamp's reach changed: the old cached order must not come back
        let b = [lamp_at(1.0, 9.0), lamp_at(4.0, 1.0)];
        let got = cached_nearest_lamps(eye, &b);
        assert_eq!(got, nearest_lamps(eye, &b));
        assert_eq!(got.pos[0].w, 9.0);
        // a moved eye re-sorts the same list
        let moved = vec3(4.0, 0.0, 0.0);
        let got = cached_nearest_lamps(moved, &b);
        assert_eq!(got, nearest_lamps(moved, &b));
        assert_eq!(got.pos[0].w, 1.0);
    }
}
