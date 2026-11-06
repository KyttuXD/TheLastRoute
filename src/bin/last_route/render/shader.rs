//! The world material: a near-black ambient (rooms are dark unless something lights them),
//! up to MAX_LAMPS coloured point lights (switchable room lights and lamp props), the held
//! flashlight's cone, and distance fog. Vertex `normal.w` selects the mode:
//! 0 tiled texture (fract UVs), 1 atlas texture, 2 unlit/emissive.

use std::cell::{Cell, OnceCell, RefCell};

use macroquad::miniquad::{
    BlendFactor, BlendState, BlendValue, Comparison, Equation, PipelineParams, UniformType,
};
use macroquad::prelude::*;

pub const MAX_LAMPS: usize = 16;

/// The lamp counts the world material is compiled for. The fragment shader loops over its lamp array, so a frame lit with
/// at most 8 (or 4) lamps runs a material whose loop is 8 (or 4) long instead of a fixed 16 of mostly empty slots.
pub const LAMP_VARIANTS: [usize; 3] = [4, 8, MAX_LAMPS];

/// The smallest compiled lamp count that holds `cap` lamps (a cap of 0 to 4 takes the 4-lamp material, 9 to 16 the full one).
pub fn variant_for_cap(cap: usize) -> usize {
    LAMP_VARIANTS.iter().copied().find(|n| *n >= cap).unwrap_or(MAX_LAMPS)
}

/// The vertex mode (normal.w) of an ordinary lit, opaque surface.
pub const LIT_MODE: f32 = 1.0;
/// The vertex mode of a lit surface that blends by its colour alpha (a crouching runner's faded body): lit like
/// `LIT_MODE`, but the alpha it draws with is the texture times the vertex colour, not 1.
pub const GHOST_MODE: f32 = 4.0;

// The texture coordinate varying is `highp` in both stages: lowp is about -2..2 at 8 bits on mobile GPUs, and tiled
// wall surfaces and door-header strips run well past that, so a lowp uv breaks the tiling on phones (the MSL twin
// below already carries a full float2).
const VERTEX: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;
varying highp vec2 uv;
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

/// The 16-lamp fragment shader, which `fragment_for` narrows to the other lamp counts.
const FRAGMENT: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
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
uniform vec4 Reach;
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
    vec3 lamp = vec3(0.0);
    for (int i = 0; i < 16; i++) {
        float range = Lamps[i].w;
        if (range > 0.0) {
            vec3 d = Lamps[i].xyz - wpos;
            float dist = length(d) + 0.001;
            float att = clamp(1.0 - dist / range, 0.0, 1.0);
            float ndl = max(dot(n, d / dist), 0.0) * 0.7 + 0.3;
            lamp += LampColors[i].rgb * LampColors[i].a * att * att * ndl;
        }
    }
    vec3 beam = vec3(0.0);
    if (LightOn > 0.5) {
        vec3 d = wpos - LightPos;
        float dist = length(d) + 0.001;
        vec3 dir = d / dist;
        float c = dot(dir, LightDir);
        float cone = smoothstep(0.80, 0.93, c) + 0.6 * smoothstep(0.955, 0.985, c);
        float att = clamp(1.0 - dist / Reach.y, 0.0, 1.0);
        float ndl = max(dot(n, -dir), 0.0);
        beam = vec3(1.0, 0.95, 0.86) * cone * att * att * (0.25 + 0.75 * ndl) * Reach.z;
    }
    float fa = clamp(fd / Reach.x, 0.0, 1.0);
    float fl = clamp(fd / FogFar, 0.0, 1.0);
    float fb = clamp(fd / Reach.w, 0.0, 1.0);
    float va = 1.0 - fa * fa;
    vec3 col = tex.rgb * (AmbientColor * va + lamp * (1.0 - fl * fl) + beam * (1.0 - fb * fb));
    gl_FragColor = vec4(col + FogColor * (1.0 - va), ghost > 0.5 ? tex.a : 1.0);
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
    packed_float4 Reach;
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
    float4 reach = float4(u.Reach);
    float3 lamp = float3(0.0);
    for (int i = 0; i < 16; i++) {
        float4 lp = float4(u.Lamps[i]);
        float range = lp.w;
        if (range > 0.0) {
            float3 d = lp.xyz - in.wpos;
            float dist = length(d) + 0.001;
            float att = clamp(1.0 - dist / range, 0.0, 1.0);
            float ndl = max(dot(n, d / dist), 0.0) * 0.7 + 0.3;
            float4 lc = float4(u.LampColors[i]);
            lamp += lc.rgb * lc.a * att * att * ndl;
        }
    }
    float3 beam = float3(0.0);
    if (u.LightOn > 0.5) {
        float3 d = in.wpos - float3(u.LightPos);
        float dist = length(d) + 0.001;
        float3 dir = d / dist;
        float c = dot(dir, float3(u.LightDir));
        float cone = smoothstep(0.80, 0.93, c) + 0.6 * smoothstep(0.955, 0.985, c);
        float att = clamp(1.0 - dist / reach.y, 0.0, 1.0);
        float ndl = max(dot(n, -dir), 0.0);
        beam = float3(1.0, 0.95, 0.86) * cone * att * att * (0.25 + 0.75 * ndl) * reach.z;
    }
    float fa = clamp(fd / reach.x, 0.0, 1.0);
    float fl = clamp(fd / u.FogFar, 0.0, 1.0);
    float fb = clamp(fd / reach.w, 0.0, 1.0);
    float va = 1.0 - fa * fa;
    float3 col = tex.rgb * (float3(u.AmbientColor) * va + lamp * (1.0 - fl * fl) + beam * (1.0 - fb * fb));
    return float4(col + float3(u.FogColor) * (1.0 - va), ghost > 0.5 ? tex.a : 1.0);
}
"#;

/// The GLSL fragment shader for a material lit with `lamps` lamps: the 16-lamp source with the two lamp arrays and the
/// loop bound narrowed. Only those three places mention the count, so the variants cannot drift from the template.
fn fragment_for(lamps: usize) -> String {
    FRAGMENT
        .replace("Lamps[16]", &format!("Lamps[{lamps}]"))
        .replace("LampColors[16]", &format!("LampColors[{lamps}]"))
        .replace("i < 16", &format!("i < {lamps}"))
}

/// The Metal twin for `lamps` lamps: the uniform struct's arrays and the loop bound narrowed, as `fragment_for` does.
fn msl_for(lamps: usize) -> String {
    MSL.replace("Lamps[16]", &format!("Lamps[{lamps}]"))
        .replace("LampColors[16]", &format!("LampColors[{lamps}]"))
        .replace("i < 16", &format!("i < {lamps}"))
}

/// The world material's own uniforms, in declaration order. macroquad packs them tightly after its built-in
/// block (Projection, Model, _Time: `BUILTIN_UNIFORM_BYTES`), so this order is the byte order of the MSL struct.
fn world_uniforms() -> Vec<UniformDesc> {
    world_uniforms_for(MAX_LAMPS)
}

/// [`world_uniforms`] with lamp arrays `lamps` long.
fn world_uniforms_for(lamps: usize) -> Vec<UniformDesc> {
    vec![
        UniformDesc::new("CamPos", UniformType::Float3),
        UniformDesc::new("LightPos", UniformType::Float3),
        UniformDesc::new("LightDir", UniformType::Float3),
        UniformDesc::new("LightOn", UniformType::Float1),
        UniformDesc::new("AmbientColor", UniformType::Float3),
        UniformDesc::new("FogColor", UniformType::Float3),
        UniformDesc::new("FogFar", UniformType::Float1),
        UniformDesc::new("Lamps", UniformType::Float4).array(lamps),
        UniformDesc::new("LampColors", UniformType::Float4).array(lamps),
        // (dark_far, flashlight range, flashlight intensity, flashlight fog reach): data/render/ambient.json
        UniformDesc::new("Reach", UniformType::Float4),
    ]
}

/// macroquad's built-in uniforms ahead of the material's own: Projection and Model (4x4 floats each) and _Time.
#[cfg(test)]
const BUILTIN_UNIFORM_BYTES: usize = 64 + 64 + 16;

/// The full 16-lamp world material.
pub fn load_world_material() -> Material {
    load_world_material_for(MAX_LAMPS)
}

/// The world material for a frame lit with at most `lamps` lamps (one of `LAMP_VARIANTS`).
pub fn load_world_material_for(lamps: usize) -> Material {
    // macroquad picks its own shaders by backend; ours needs the matching source
    let (msl, fragment);
    let source = if crate::app::platform::is_metal() {
        msl = msl_for(lamps);
        ShaderSource::Msl { program: &msl }
    } else {
        fragment = fragment_for(lamps);
        ShaderSource::Glsl {
            vertex: VERTEX,
            fragment: &fragment,
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
            uniforms: world_uniforms_for(lamps),
            ..Default::default()
        },
    )
    .expect("world shader")
}

/// The world material in each lamp-count variant. The 16-lamp one is compiled at boot; a narrower one is compiled the
/// first frame a quality tier or the governor asks for it (a one-off shader compile), so a run that never drops below
/// 16 lamps pays for none. It derefs to the variant the current lamp cap needs, so it stands wherever a `&Material` does.
pub struct WorldMats {
    variants: [OnceCell<Material>; 3],
}

impl WorldMats {
    pub fn new() -> WorldMats {
        let m = WorldMats { variants: Default::default() };
        let _ = m.variants[2].set(load_world_material());
        m
    }

    /// The material compiled for `lamps` (one of `LAMP_VARIANTS`), compiling it on first use.
    pub fn get(&self, lamps: usize) -> &Material {
        let at = LAMP_VARIANTS.iter().position(|n| *n == variant_for_cap(lamps)).unwrap_or(2);
        self.variants[at].get_or_init(|| load_world_material_for(LAMP_VARIANTS[at]))
    }

    /// Compiles the variant for `lamps` now, so the frame that switches to it does not stall on the compile.
    pub fn prewarm(&self, lamps: usize) {
        let _ = self.get(lamps);
    }
}

impl Default for WorldMats {
    fn default() -> Self {
        WorldMats::new()
    }
}

impl std::ops::Deref for WorldMats {
    type Target = Material;

    /// The material that matches `lamp_cap()` (see `lamp_variant`).
    fn deref(&self) -> &Material {
        self.get(lamp_variant())
    }
}

/// A light in the world: position, reach, colour and energy.
#[derive(Clone, Copy)]
pub struct Lamp {
    pub pos: Vec3,
    pub range: f32,
    pub color: Color,
    pub energy: f32,
    /// A slow wobble of `energy` with time (render/lamp_flicker.rs); `Flicker::NONE` is steady.
    pub flicker: Flicker,
}

/// How a lamp's energy wobbles: `hz` cycles a second, swinging the energy by `depth` (0..1) either way, started at
/// `phase` (0..1 of a cycle, drawn from the lamp's place so a row of lamps does not pulse together).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flicker {
    pub hz: f32,
    pub depth: f32,
    pub phase: f32,
}

impl Flicker {
    pub const NONE: Flicker = Flicker { hz: 0.0, depth: 0.0, phase: 0.0 };
}

/// The global ambient, Godot's RefLook environment (game/look/ref_look.gd make_environment): colour #5a3550 at
/// energy AMBIENT_ENERGY. These are the fallback if data/render/ambient.json (render/ambient.rs) fails to load; the live values
/// are per region and level there, and Godot has neither (the night-watch office moods are their own scenes). Lit surfaces get `AMBIENT_COLOR * AMBIENT_ENERGY` added
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
    /// Metres to full fog on lamp-lit light (the lamps' reach).
    pub fog_far: f32,
    /// Metres to full fog on the ambient light alone: the dark view of the place.
    pub dark_far: f32,
    /// The held beam's throw, strength and fog reach (data/render/ambient.json `flashlight`).
    pub flash: super::ambient::Flashlight,
}

impl Lighting {
    /// The `fog_far` the world cull follows: the lamps' reach, widened to the beam's fog reach while the beam is lit
    /// (the cull multiplies by the unlit scale, so the beam's reach is divided by it here).
    pub fn cull_fog_far(&self) -> f32 {
        if self.flashlight_on {
            self.fog_far.max(self.flash.fog_reach / 1.6)
        } else {
            self.fog_far
        }
    }
}

/// The lamps nearest the eye, packed for the shader: `pos` holds (x, y, z, range) and `col` holds (r, g, b, energy),
/// nearest first. Slots past the lamp count stay zero, which the shader reads as "no lamp" (range 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearLamps {
    pos: [Vec4; MAX_LAMPS],
    col: [Vec4; MAX_LAMPS],
}

thread_local! {
    /// The most lamps the graphics quality tier allows (`lamp_flicker` `lamp_cap`).
    static TIER_LAMP_CAP: Cell<usize> = const { Cell::new(MAX_LAMPS) };
    /// The most lamps the quality governor allows right now (app/quality.rs); MAX_LAMPS while it is off or at notch 0.
    static GOVERNOR_LAMP_CAP: Cell<usize> = const { Cell::new(MAX_LAMPS) };
}

/// The most lamps a frame is lit with: the smaller of the graphics quality tier's cap (`lamp_flicker` `lamp_cap`, 8 on the
/// slowest tier) and the governor's.
pub fn lamp_cap() -> usize {
    effective_cap(TIER_LAMP_CAP.with(Cell::get), GOVERNOR_LAMP_CAP.with(Cell::get))
}

/// The lamp cap from the tier's and the governor's: the smaller, at least 1 and at most MAX_LAMPS.
pub fn effective_cap(tier: usize, governor: usize) -> usize {
    tier.min(governor).clamp(1, MAX_LAMPS)
}

/// The compiled lamp count (one of `LAMP_VARIANTS`) the current `lamp_cap()` lights with: the material `WorldMats` hands
/// out and the number of lamp slots `apply` uploads.
pub fn lamp_variant() -> usize {
    variant_for_cap(lamp_cap())
}

/// Sets the lamp cap for the graphics quality tier; the shader's array is the narrowest variant that holds it and the
/// rest of the slots read as no lamp.
pub fn set_lamp_cap_for(quality: &str) {
    TIER_LAMP_CAP.with(|c| c.set(super::lamp_flicker::table().lamp_cap(quality).min(MAX_LAMPS)));
}

/// Sets the governor's lamp limit (16, 8 or 4 from the notch table); `lamp_cap` is the smaller of this and the tier's.
pub fn set_governor_lamp_cap(limit: usize) {
    GOVERNOR_LAMP_CAP.with(|c| c.set(limit.clamp(1, MAX_LAMPS)));
}

/// The up to `lamp_cap()` lamps nearest `eye`, nearest first. Equal distances keep their input order (stable sort).
pub fn nearest_lamps(eye: Vec3, lamps: &[Lamp]) -> NearLamps {
    nearest_lamps_capped(eye, lamps, lamp_cap())
}

/// The up to `cap` (at most MAX_LAMPS) lamps nearest `eye`, nearest first.
pub fn nearest_lamps_capped(eye: Vec3, lamps: &[Lamp], cap: usize) -> NearLamps {
    #[cfg(test)]
    SORTS.with(|c| c.set(c.get() + 1));
    let mut near: Vec<(f32, &Lamp)> = lamps
        .iter()
        .map(|lp| ((lp.pos - eye).length_squared(), lp))
        .collect();
    near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = NearLamps {
        pos: [Vec4::ZERO; MAX_LAMPS],
        col: [Vec4::ZERO; MAX_LAMPS],
    };
    for (i, (_, lp)) in near.iter().take(cap.min(MAX_LAMPS)).enumerate() {
        out.pos[i] = vec4(lp.pos.x, lp.pos.y, lp.pos.z, lp.range);
        out.col[i] = vec4(lp.color.r, lp.color.g, lp.color.b, lp.energy);
    }
    out
}

#[cfg(test)]
thread_local! {
    static SORTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

/// One cached `nearest_lamps` result and the inputs it came from.
struct LampCache {
    eye: Vec3,
    cap: usize,
    lamps: Vec<Lamp>,
    near: NearLamps,
}

/// A frame lights with a few distinct lamp lists (the room's merged list, the figures' and the hands'), so the cache
/// keeps one entry for each rather than one that every other list evicts.
const LAMP_CACHE_SLOTS: usize = 4;

thread_local! {
    static LAMP_CACHE: RefCell<Vec<LampCache>> = const { RefCell::new(Vec::new()) };
}

/// `nearest_lamps`, reused while the eye, the cap and the lamp list are unchanged. Every material is drawn with the
/// same lighting in a frame, so each distinct list is sorted once per frame rather than once per material. Any change
/// to the eye, the cap or a lamp (position, reach, colour or energy, or order) is a miss, so the result never goes stale.
fn cached_nearest_lamps(eye: Vec3, lamps: &[Lamp]) -> NearLamps {
    let cap = lamp_cap();
    LAMP_CACHE.with(|slots| {
        let mut slots = slots.borrow_mut();
        if let Some(hit) = slots.iter().find(|h| h.eye == eye && h.cap == cap && same_lamps(&h.lamps, lamps)) {
            return hit.near;
        }
        let near = nearest_lamps_capped(eye, lamps, cap);
        if slots.len() >= LAMP_CACHE_SLOTS {
            slots.remove(0);
        }
        slots.push(LampCache { eye, cap, lamps: lamps.to_vec(), near });
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
    mat.set_uniform("Reach", vec4(l.dark_far, l.flash.range, l.flash.intensity, l.flash.fog_reach));
    let near = cached_nearest_lamps(l.eye, lamps);
    // the material's arrays are as long as its variant (`WorldMats`): slots past the cap are empty anyway
    let slots = lamp_variant();
    mat.set_uniform_array("Lamps", &near.pos[..slots]);
    mat.set_uniform_array("LampColors", &near.col[..slots]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uv_varying_is_high_precision_in_both_stages() {
        for (stage, src) in [("vertex", VERTEX), ("fragment", FRAGMENT)] {
            assert!(src.contains("varying highp vec2 uv;"), "{stage} stage declares uv highp");
            assert!(!src.contains("lowp vec2 uv"), "{stage} stage has no lowp uv");
        }
    }

    fn lamp_at(x: f32, range: f32) -> Lamp {
        Lamp {
            pos: vec3(x, 0.0, 0.0),
            range,
            color: Color::new(1.0, 0.5, 0.25, 1.0),
            energy: 2.0,
            flicker: Flicker::NONE,
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

    #[test]
    fn each_distinct_lamp_list_is_sorted_once_however_many_materials_ask() {
        let eye = Vec3::ZERO;
        let room = [lamp_at(4.0, 1.0), lamp_at(1.0, 2.0)];
        let hands = [lamp_at(2.0, 3.0)];
        let before = SORTS.with(|c| c.get());
        for _ in 0..30 {
            cached_nearest_lamps(eye, &room);
            cached_nearest_lamps(eye, &hands);
            cached_nearest_lamps(eye, &room);
        }
        assert_eq!(SORTS.with(|c| c.get()) - before, 2, "two lists, two sorts, over 90 material uploads");
        // a new frame with a moved eye sorts each again, once
        cached_nearest_lamps(vec3(1.0, 0.0, 0.0), &room);
        cached_nearest_lamps(vec3(1.0, 0.0, 0.0), &room);
        assert_eq!(SORTS.with(|c| c.get()) - before, 3);
    }

    #[test]
    fn the_cap_limits_how_many_lamps_light_the_frame() {
        let lamps: Vec<Lamp> = (0..20).map(|i| lamp_at(i as f32, 5.0)).collect();
        for cap in [0usize, 1, 8, 16, 100] {
            let near = nearest_lamps_capped(Vec3::ZERO, &lamps, cap);
            let lit = near.pos.iter().filter(|p| p.w > 0.0).count();
            assert_eq!(lit, cap.min(MAX_LAMPS), "cap {cap}");
        }
        let eight = nearest_lamps_capped(Vec3::ZERO, &lamps, 8);
        assert_eq!(eight.pos[7].x, 7.0, "the eight kept are the nearest");
        assert_eq!(eight.pos[8], Vec4::ZERO);
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
        assert_eq!(at, 740, "total uniform bytes");
        let off = |n: &str| want.iter().find(|(k, _)| k == n).map(|(_, o)| *o);
        assert_eq!(
            [off("CamPos"), off("LightPos"), off("LightDir"), off("LightOn"), off("AmbientColor"), off("FogColor"), off("FogFar"), off("Lamps"), off("LampColors"), off("Reach")],
            [144, 156, 168, 180, 184, 196, 208, 212, 468, 724].map(Some)
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
        assert_eq!(at, 740);
        // the arrays of 16 in the MSL are MAX_LAMPS
        assert_eq!(MAX_LAMPS, 16);
    }

    /// Compiles the MSL twin with the system's Metal compiler and looks up both entry points. This is the one Metal
    /// check that runs in `cargo test` (no window is needed): a typo in the twin otherwise only shows as a panic at
    /// the first Metal start. Skipped, with a line on stderr, where no Metal device exists (a CI VM).
    #[cfg(target_os = "macos")]
    #[test]
    fn msl_source_compiles_with_the_metal_compiler() {
        use core::ffi::{c_char, c_void};
        #[link(name = "Metal", kind = "framework")]
        extern "C" {
            fn MTLCreateSystemDefaultDevice() -> *mut c_void;
        }
        #[link(name = "Foundation", kind = "framework")]
        extern "C" {}
        #[link(name = "objc")]
        extern "C" {
            fn objc_getClass(name: *const c_char) -> *mut c_void;
            fn sel_registerName(name: *const c_char) -> *mut c_void;
            fn objc_msgSend();
        }
        type Msg0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
        type Msg1 = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void;
        type MsgLib = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> *mut c_void;
        type MsgFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
        unsafe {
            let device = MTLCreateSystemDefaultDevice();
            if device.is_null() {
                eprintln!("no Metal device here: MSL compile check skipped");
                return;
            }
            let sel = |n: &str| sel_registerName(format!("{n}\0").as_ptr() as *const c_char);
            let msg = objc_msgSend as *const ();
            let string_with: Msg1 = std::mem::transmute(msg);
            let src = std::ffi::CString::new(MSL).unwrap();
            let ns_src = string_with(objc_getClass(b"NSString\0".as_ptr() as *const c_char), sel("stringWithUTF8String:"), src.as_ptr());
            assert!(!ns_src.is_null());
            let mut error: *mut c_void = std::ptr::null_mut();
            let new_lib: MsgLib = std::mem::transmute(msg);
            let lib = new_lib(device, sel("newLibraryWithSource:options:error:"), ns_src, std::ptr::null_mut(), &mut error);
            if lib.is_null() {
                let describe: Msg0 = std::mem::transmute(msg);
                let text = describe(error, sel("localizedDescription"));
                let utf8: Msg0 = std::mem::transmute(msg);
                let c = utf8(text, sel("UTF8String")) as *const c_char;
                panic!("the MSL twin does not compile: {}", std::ffi::CStr::from_ptr(c).to_string_lossy());
            }
            let named: MsgFn = std::mem::transmute(msg);
            for entry in ["vertexShader", "fragmentShader"] {
                let c = std::ffi::CString::new(entry).unwrap();
                let ns = string_with(objc_getClass(b"NSString\0".as_ptr() as *const c_char), sel("stringWithUTF8String:"), c.as_ptr());
                assert!(!named(lib, sel("newFunctionWithName:"), ns).is_null(), "entry point {entry} missing");
            }
        }
    }

    #[test]
    fn both_shaders_read_the_beam_and_dark_reach_from_the_one_uniform() {
        // the flashlight's range and strength and the fog reaches come from Reach (data), not from literals in the shader
        for (glsl, msl) in [("Reach.x", "reach.x"), ("Reach.y", "reach.y"), ("Reach.z", "reach.z"), ("Reach.w", "reach.w")] {
            assert!(FRAGMENT.contains(glsl), "GLSL lost {glsl}");
            assert!(MSL.contains(msl), "MSL lost {msl}");
        }
        assert!(!FRAGMENT.contains("/ 16.0") && !MSL.contains("/ 16.0"), "the beam range is data, not 16 m");
        assert!(!FRAGMENT.contains("* 1.9;") && !MSL.contains("* 1.9;"), "the beam strength is data, not 1.9");
    }

    #[test]
    fn a_cap_takes_the_narrowest_variant_that_holds_it() {
        let table: Vec<(usize, usize)> = [0usize, 1, 3, 4, 5, 8, 9, 12, 16, 17, 100].iter().map(|c| (*c, variant_for_cap(*c))).collect();
        assert_eq!(table, vec![(0, 4), (1, 4), (3, 4), (4, 4), (5, 8), (8, 8), (9, 16), (12, 16), (16, 16), (17, 16), (100, 16)]);
        assert_eq!(LAMP_VARIANTS, [4, 8, MAX_LAMPS]);
        assert!(LAMP_VARIANTS.windows(2).all(|w| w[0] < w[1]), "ascending, so the narrowest fit is the first");
    }

    #[test]
    fn the_lamp_cap_is_the_smaller_of_the_tier_and_the_governor() {
        assert_eq!(effective_cap(16, 16), 16);
        assert_eq!(effective_cap(8, 16), 8, "the slow tier's cap holds under an idle governor");
        assert_eq!(effective_cap(16, 4), 4);
        assert_eq!(effective_cap(8, 4), 4);
        assert_eq!(effective_cap(0, 16), 1, "never none");
        assert_eq!(effective_cap(99, 99), MAX_LAMPS);
        // the live cap, which picks the material (these cells are per thread, so no other test sees them)
        assert_eq!(lamp_cap(), MAX_LAMPS);
        set_governor_lamp_cap(8);
        assert_eq!((lamp_cap(), lamp_variant()), (8, 8));
        set_governor_lamp_cap(4);
        assert_eq!((lamp_cap(), lamp_variant()), (4, 4));
        set_governor_lamp_cap(99);
        assert_eq!((lamp_cap(), lamp_variant()), (MAX_LAMPS, MAX_LAMPS));
        set_lamp_cap_for("LOW");
        assert_eq!(lamp_cap(), 8, "the LOW tier's table cap");
        set_governor_lamp_cap(4);
        assert_eq!(lamp_variant(), 4, "the governor goes below the tier");
        set_lamp_cap_for("HIGH");
        set_governor_lamp_cap(MAX_LAMPS);
    }

    #[test]
    fn every_variant_loops_over_exactly_its_own_lamps() {
        for n in LAMP_VARIANTS {
            let (glsl, msl) = (fragment_for(n), msl_for(n));
            for (name, src) in [("GLSL", &glsl), ("MSL", &msl)] {
                assert!(src.contains(&format!("i < {n}")), "{name} {n}: loop bound");
                assert!(src.contains(&format!("Lamps[{n}]")) || name == "MSL", "{name} {n}: lamp array");
                assert!(src.contains(&format!("LampColors[{n}]")) || name == "MSL", "{name} {n}: colour array");
                if n != MAX_LAMPS {
                    assert!(!src.contains("[16]") && !src.contains("i < 16"), "{name} {n} still names 16 lamps");
                }
            }
            assert!(msl.contains(&format!("packed_float4 Lamps[{n}];")) && msl.contains(&format!("packed_float4 LampColors[{n}];")), "MSL {n}: struct arrays");
        }
        assert_eq!(fragment_for(MAX_LAMPS), FRAGMENT);
        assert_eq!(msl_for(MAX_LAMPS), MSL);
    }

    #[test]
    fn each_variants_metal_struct_matches_its_uniform_byte_layout() {
        for n in LAMP_VARIANTS {
            let msl = msl_for(n);
            let mut want: Vec<(String, usize)> = vec![("Projection".into(), 0), ("Model".into(), 64), ("_Time".into(), 128)];
            let mut at = BUILTIN_UNIFORM_BYTES;
            for u in world_uniforms_for(n) {
                want.push((u.name.clone(), at));
                at += u.uniform_type.size() * u.array_count;
            }
            // 212 bytes ahead of the arrays (144 of built-ins + 68 of single uniforms) + Lamps and LampColors (2 arrays of
            // n float4s = 32 * n) + the Reach float4 (16) that follows them: 228 + 32 * n (740 for the 16-lamp material)
            assert_eq!(at, 212 + 32 * n + 16, "{n}: total uniform bytes");
            let body = msl.split("struct Uniforms {").nth(1).and_then(|b| b.split("};").next()).expect("Uniforms struct");
            let mut got: Vec<(String, usize)> = Vec::new();
            let mut off = 0;
            for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (ty, name) = line.trim_end_matches(';').split_once(' ').expect("type and name");
                let (name, count) = match name.split_once('[') {
                    Some((nm, c)) => (nm, c.trim_end_matches(']').parse::<usize>().expect("array length")),
                    None => (name, 1),
                };
                got.push((name.to_string(), off));
                off += msl_size(ty) * count;
            }
            assert_eq!(got, want, "{n}: field offsets");
            assert_eq!(off, at, "{n}: struct size");
        }
    }

    #[test]
    fn msl_and_glsl_share_the_same_lighting_constants() {
        for k in ["0.80", "0.93", "0.955", "0.985", "0.001", "0.7 + 0.3", "0.25 + 0.75", "(FogFar * 1.6)", "1.0, 0.95, 0.86", "0.6 *", "1.0 - fa * fa", "(1.0 - fl * fl)", "(1.0 - fb * fb)", "(1.0 - va)"] {
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
