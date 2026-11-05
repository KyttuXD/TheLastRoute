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

/// The vertex mode (normal.w) of an ordinary lit, opaque surface.
pub const LIT_MODE: f32 = 1.0;
/// The vertex mode of a lit surface that blends by its colour alpha (a crouching runner's faded body): lit like
/// `LIT_MODE`, but the alpha it draws with is the texture times the vertex colour, not 1.
pub const GHOST_MODE: f32 = 4.0;

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
    float ghost = mode > 3.5 ? 1.0 : 0.0;
    if (ghost > 0.5) mode = 1.0;
    vec2 tuv = mode < 0.5 ? fract(uv) : uv;
    vec4 tex = texture2D(Texture, tuv) * color;
    if (mode < 2.5 && tex.a < (ghost > 0.5 ? 0.02 : 0.5)) discard;
    float fd = length(wpos - CamPos);
    if (mode > 1.5) {
        float f = clamp(fd / (FogFar * 1.6), 0.0, 1.0);
        gl_FragColor = vec4(mix(tex.rgb, FogColor, f * f), mode > 2.5 ? tex.a : 1.0);
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
    gl_FragColor = vec4(mix(col, FogColor, f * f), ghost > 0.5 ? tex.a : 1.0);
}"#;

/// The Metal twin of VERTEX and FRAGMENT. It must stay line for line with them (the test below checks the
/// constants). Two Metal differences: clip z runs 0..w where macroquad's matrices give -w..w, and macroquad packs
/// uniform bytes tightly with no std140 padding, so every float3 is a `packed_float3` and the lamp arrays are
/// `packed_float4`; the field order is the byte order (see `world_uniforms`).
const MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Uniforms {
    float4x4 Projection;
    float4x4 Model;
    float4 _Time;
    packed_float3 CamPos;
    packed_float3 LightPos;
    packed_float3 LightDir;
    float LightOn;
    packed_float3 AmbientColor;
    packed_float3 FogColor;
    float FogFar;
    packed_float4 Lamps[16];
    packed_float4 LampColors[16];
};
struct Vertex {
    float3 position [[attribute(0)]];
    float2 texcoord [[attribute(1)]];
    float4 color0   [[attribute(2)]];
    float4 normal   [[attribute(3)]];
};
struct VOut {
    float4 position [[position]];
    float2 uv;
    float4 color;
    float3 wpos;
    float4 nrm;
};
vertex VOut vertexShader(Vertex v [[stage_in]], constant Uniforms& u [[buffer(0)]]) {
    VOut o;
    float4 w = u.Model * float4(v.position, 1.0);
    float4 p = u.Projection * w;
    p.z = (p.z + p.w) * 0.5;
    o.position = p;
    o.wpos = w.xyz;
    o.uv = v.texcoord;
    o.color = v.color0 / 255.0;
    o.nrm = v.normal;
    return o;
}
fragment float4 fragmentShader(VOut in [[stage_in]], constant Uniforms& u [[buffer(0)]],
                               texture2d<float> Texture [[texture(0)]], sampler TextureSmplr [[sampler(0)]]) {
    float mode = in.nrm.w;
    float ghost = mode > 3.5 ? 1.0 : 0.0;
    if (ghost > 0.5) mode = 1.0;
    float2 tuv = mode < 0.5 ? fract(in.uv) : in.uv;
    float4 tex = Texture.sample(TextureSmplr, tuv) * in.color;
    if (mode < 2.5 && tex.a < (ghost > 0.5 ? 0.02 : 0.5)) discard_fragment();
    float fd = length(in.wpos - float3(u.CamPos));
    if (mode > 1.5) {
        float f = clamp(fd / (u.FogFar * 1.6), 0.0, 1.0);
        return float4(mix(tex.rgb, float3(u.FogColor), f * f), mode > 2.5 ? tex.a : 1.0);
    }
    float3 n = normalize(in.nrm.xyz);
    float3 light = float3(u.AmbientColor);
    for (int i = 0; i < 16; i++) {
        float4 lamp = float4(u.Lamps[i]);
        float range = lamp.w;
        if (range > 0.0) {
            float3 d = lamp.xyz - in.wpos;
            float dist = length(d) + 0.001;
            float att = clamp(1.0 - dist / range, 0.0, 1.0);
            float ndl = max(dot(n, d / dist), 0.0) * 0.7 + 0.3;
            float4 lc = float4(u.LampColors[i]);
            light += lc.rgb * lc.a * att * att * ndl;
        }
    }
    if (u.LightOn > 0.5) {
        float3 d = in.wpos - float3(u.LightPos);
        float dist = length(d) + 0.001;
        float3 dir = d / dist;
        float c = dot(dir, float3(u.LightDir));
        float cone = smoothstep(0.80, 0.93, c) + 0.6 * smoothstep(0.955, 0.985, c);
        float att = clamp(1.0 - dist / 16.0, 0.0, 1.0);
        float ndl = max(dot(n, -dir), 0.0);
        light += float3(1.0, 0.95, 0.86) * cone * att * att * (0.25 + 0.75 * ndl) * 1.9;
    }
    float3 col = tex.rgb * light;
    float f = clamp(fd / u.FogFar, 0.0, 1.0);
    return float4(mix(col, float3(u.FogColor), f * f), ghost > 0.5 ? tex.a : 1.0);
}
"#;

/// The world material's own uniforms, in declaration order. macroquad packs them tightly after its built-in
/// block (Projection, Model, _Time: `BUILTIN_UNIFORM_BYTES`), so this order is the byte order of the MSL struct.
fn world_uniforms() -> Vec<UniformDesc> {
    vec![
        UniformDesc::new("CamPos", UniformType::Float3),
        UniformDesc::new("LightPos", UniformType::Float3),
        UniformDesc::new("LightDir", UniformType::Float3),
        UniformDesc::new("LightOn", UniformType::Float1),
        UniformDesc::new("AmbientColor", UniformType::Float3),
        UniformDesc::new("FogColor", UniformType::Float3),
        UniformDesc::new("FogFar", UniformType::Float1),
        UniformDesc::new("Lamps", UniformType::Float4).array(MAX_LAMPS),
        UniformDesc::new("LampColors", UniformType::Float4).array(MAX_LAMPS),
    ]
}

/// macroquad's built-in uniforms ahead of the material's own: Projection and Model (4x4 floats each) and _Time.
#[cfg(test)]
const BUILTIN_UNIFORM_BYTES: usize = 64 + 64 + 16;

pub fn load_world_material() -> Material {
    // macroquad picks its own shaders by backend; ours needs the matching source
    let source = if crate::app::platform::is_metal() {
        ShaderSource::Msl { program: MSL }
    } else {
        ShaderSource::Glsl {
            vertex: VERTEX,
            fragment: FRAGMENT,
        }
    };
    load_material(
        source,
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
            uniforms: world_uniforms(),
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

    /// The byte size of an MSL field type used in the Uniforms struct.
    fn msl_size(ty: &str) -> usize {
        match ty {
            "float4x4" => 64,
            "float4" | "packed_float4" => 16,
            "packed_float3" => 12,
            "float" => 4,
            other => panic!("unexpected uniform type {other}"),
        }
    }

    #[test]
    fn msl_uniform_block_matches_macroquad_byte_layout() {
        // the byte offsets macroquad uses: the built-ins, then the material's uniforms packed in order
        let mut want: Vec<(String, usize)> = vec![("Projection".into(), 0), ("Model".into(), 64), ("_Time".into(), 128)];
        let mut at = BUILTIN_UNIFORM_BYTES;
        for u in world_uniforms() {
            want.push((u.name.clone(), at));
            at += u.uniform_type.size() * u.array_count;
        }
        assert_eq!(at, 724, "total uniform bytes");
        let off = |n: &str| want.iter().find(|(k, _)| k == n).map(|(_, o)| *o);
        assert_eq!(
            [off("CamPos"), off("LightPos"), off("LightDir"), off("LightOn"), off("AmbientColor"), off("FogColor"), off("FogFar"), off("Lamps"), off("LampColors")],
            [144, 156, 168, 180, 184, 196, 208, 212, 468].map(Some)
        );
        // the MSL struct, walked field by field, must land every field on the same offset with no padding
        let body = MSL.split("struct Uniforms {").nth(1).and_then(|b| b.split("};").next()).expect("Uniforms struct");
        let mut got: Vec<(String, usize)> = Vec::new();
        let mut at = 0;
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let line = line.trim_end_matches(';');
            let (ty, name) = line.split_once(' ').expect("type and name");
            assert!(ty != "float3", "bare float3 is 16-byte aligned in MSL; use packed_float3: {line}");
            let (name, count) = match name.split_once('[') {
                Some((n, c)) => (n, c.trim_end_matches(']').parse::<usize>().expect("array length")),
                None => (name, 1),
            };
            got.push((name.to_string(), at));
            at += msl_size(ty) * count;
        }
        assert_eq!(got, want);
        assert_eq!(at, 724);
        // the arrays of 16 in the MSL are MAX_LAMPS
        assert_eq!(MAX_LAMPS, 16);
    }

    #[test]
    fn msl_and_glsl_share_the_same_lighting_constants() {
        for k in ["0.80", "0.93", "0.955", "0.985", "16.0", "1.9", "0.001", "0.7 + 0.3", "0.25 + 0.75", "(FogFar * 1.6)", "1.0, 0.95, 0.86", "0.6 *"] {
            let msl_k = k.replace("FogFar", "u.FogFar");
            assert!(FRAGMENT.contains(k), "GLSL lost {k}");
            assert!(MSL.contains(&msl_k), "MSL lost {k}");
        }
        // the modes and the lamp loop bound
        for k in ["mode < 0.5", "mode < 2.5", "mode > 1.5", "mode > 2.5", "mode > 3.5", "ghost > 0.5 ? tex.a : 1.0", "i < 16"] {
            assert!(FRAGMENT.contains(k) && MSL.contains(k), "{k}");
        }
    }
}
