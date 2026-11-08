#![allow(dead_code)]
//! The lit material of Night Watch (GLSL 100, so it runs on WebGL1). Lambert model: ambient + up to 4 omni lights +
//! up to 3 spot lights, optional exponential fog. Vertex `normal.w` is the mode: 1.0 lit, 0.0 unlit/emissive.
//! `pack_lights` is pure (no GL) so the packing can be tested.

use macroquad::miniquad::{Comparison, PipelineParams, UniformType};
use macroquad::prelude::*;

pub const MAX_OMNI: usize = 4;
pub const MAX_SPOT: usize = 3;
/// Vertex mode (normal.w) of a lit surface.
pub const LIT_MODE: f32 = 1.0;
/// Vertex mode of an unlit/emissive surface.
pub const EMISSIVE_MODE: f32 = 0.0;

#[derive(Clone, Copy)]
pub struct Omni {
    pub pos: Vec3,
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
}

#[derive(Clone, Copy)]
pub struct Spot {
    pub pos: Vec3,
    pub dir: Vec3,
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
    /// Cosine of the HALF angle.
    pub cos_outer: f32,
}

#[derive(Clone)]
pub struct LightSet {
    pub ambient: [f32; 3],
    pub omni: Vec<Omni>,
    pub spot: Vec<Spot>,
    pub fog: Option<([f32; 3], f32)>,
}

impl LightSet {
    pub fn office_default() -> LightSet {
        LightSet { ambient: [0.38, 0.40, 0.55], omni: Vec::new(), spot: Vec::new(), fog: None }
    }

    /// Camera feeds: bright flat ambient, no lights.
    pub fn feed() -> LightSet {
        LightSet { ambient: [0.95 * 1.15, 0.9 * 1.15, 0.8 * 1.15], omni: Vec::new(), spot: Vec::new(), fog: None }
    }
}

/// The lights as the uniform arrays hold them. Unused slots have range 0, which the shader skips.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedLights {
    pub ambient: Vec3,
    /// xyz position, w range
    pub omni_pos: [Vec4; MAX_OMNI],
    /// rgb * energy
    pub omni_col: [Vec4; MAX_OMNI],
    pub spot_pos: [Vec4; MAX_SPOT],
    /// xyz unit direction, w cos_outer
    pub spot_dir: [Vec4; MAX_SPOT],
    pub spot_col: [Vec4; MAX_SPOT],
    pub fog_col: Vec3,
    pub fog_density: f32,
}

pub fn pack_lights(l: &LightSet) -> PackedLights {
    let mut p = PackedLights {
        ambient: vec3(l.ambient[0], l.ambient[1], l.ambient[2]),
        omni_pos: [Vec4::ZERO; MAX_OMNI],
        omni_col: [Vec4::ZERO; MAX_OMNI],
        spot_pos: [Vec4::ZERO; MAX_SPOT],
        spot_dir: [Vec4::ZERO; MAX_SPOT],
        spot_col: [Vec4::ZERO; MAX_SPOT],
        fog_col: Vec3::ZERO,
        fog_density: 0.0,
    };
    for (i, o) in l.omni.iter().take(MAX_OMNI).enumerate() {
        p.omni_pos[i] = vec4(o.pos.x, o.pos.y, o.pos.z, o.range.max(0.0));
        p.omni_col[i] = vec4(o.color[0] * o.energy, o.color[1] * o.energy, o.color[2] * o.energy, 0.0);
    }
    for (i, s) in l.spot.iter().take(MAX_SPOT).enumerate() {
        let d = if s.dir.length_squared() > 1e-8 { s.dir.normalize() } else { vec3(0.0, -1.0, 0.0) };
        p.spot_pos[i] = vec4(s.pos.x, s.pos.y, s.pos.z, s.range.max(0.0));
        p.spot_dir[i] = vec4(d.x, d.y, d.z, s.cos_outer);
        p.spot_col[i] = vec4(s.color[0] * s.energy, s.color[1] * s.energy, s.color[2] * s.energy, 0.0);
    }
    if let Some((c, density)) = l.fog {
        p.fog_col = vec3(c[0], c[1], c[2]);
        p.fog_density = density.max(0.0);
    }
    p
}

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
    nrm = vec4((Model * vec4(normal.xyz, 0.0)).xyz, normal.w);
}"#;

const FRAGMENT: &str = r#"#version 100
precision highp float;
varying highp vec2 uv;
varying lowp vec4 color;
varying highp vec3 wpos;
varying mediump vec4 nrm;
uniform sampler2D Texture;
uniform vec3 CamPos;
uniform vec3 Ambient;
uniform vec4 OmniPos[4];
uniform vec4 OmniCol[4];
uniform vec4 SpotPos[3];
uniform vec4 SpotDir[3];
uniform vec4 SpotCol[3];
uniform vec3 FogCol;
uniform float FogDensity;
void main() {
    vec4 tex = texture2D(Texture, uv);
    if (tex.a < 0.5) discard;
    vec3 albedo = tex.rgb * color.rgb;
    vec3 outc = albedo;
    if (nrm.w > 0.5) {
        vec3 n = normalize(nrm.xyz);
        vec3 lit = Ambient;
        for (int i = 0; i < 4; i++) {
            float range = OmniPos[i].w;
            if (range > 0.0) {
                vec3 d = OmniPos[i].xyz - wpos;
                float dist = length(d) + 0.001;
                float att = max(1.0 - dist / range, 0.0);
                lit += OmniCol[i].rgb * max(dot(n, d / dist), 0.0) * att * att;
            }
        }
        for (int j = 0; j < 3; j++) {
            float range = SpotPos[j].w;
            if (range > 0.0) {
                vec3 d = SpotPos[j].xyz - wpos;
                float dist = length(d) + 0.001;
                vec3 l = d / dist;
                float att = max(1.0 - dist / range, 0.0);
                float co = SpotDir[j].w;
                float cone = smoothstep(co, co + 0.1, dot(-l, SpotDir[j].xyz));
                lit += SpotCol[j].rgb * max(dot(n, l), 0.0) * att * att * cone;
            }
        }
        outc = albedo * lit;
    }
    if (FogDensity > 0.0) {
        float fd = length(wpos - CamPos);
        float f = clamp(1.0 - exp(-FogDensity * fd), 0.0, 1.0);
        outc = mix(outc, FogCol, f);
    }
    gl_FragColor = vec4(outc, 1.0);
}"#;

/// The Metal twin of VERTEX and FRAGMENT (same maths; keep them line for line). Metal differences: clip z runs 0..w
/// where macroquad's matrices give -w..w, and macroquad packs uniform bytes tightly after its built-in block
/// (Projection, Model, _Time) with no std140 padding, so every float3 is a `packed_float3`, the light arrays are
/// `packed_float4`, and the field order is the byte order of the uniform list in `uniform_descs`.
const MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Uniforms {
    float4x4 Projection;
    float4x4 Model;
    float4 _Time;
    packed_float3 CamPos;
    packed_float3 Ambient;
    packed_float4 OmniPos[4];
    packed_float4 OmniCol[4];
    packed_float4 SpotPos[3];
    packed_float4 SpotDir[3];
    packed_float4 SpotCol[3];
    packed_float3 FogCol;
    float FogDensity;
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
    o.nrm = float4((u.Model * float4(v.normal.xyz, 0.0)).xyz, v.normal.w);
    return o;
}
fragment float4 fragmentShader(VOut in [[stage_in]], constant Uniforms& u [[buffer(0)]],
                               texture2d<float> Texture [[texture(0)]], sampler TextureSmplr [[sampler(0)]]) {
    float4 tex = Texture.sample(TextureSmplr, in.uv);
    if (tex.a < 0.5) discard_fragment();
    float3 albedo = tex.rgb * in.color.rgb;
    float3 outc = albedo;
    if (in.nrm.w > 0.5) {
        float3 n = normalize(in.nrm.xyz);
        float3 lit = float3(u.Ambient);
        for (int i = 0; i < 4; i++) {
            float4 op = float4(u.OmniPos[i]);
            float range = op.w;
            if (range > 0.0) {
                float3 d = op.xyz - in.wpos;
                float dist = length(d) + 0.001;
                float att = max(1.0 - dist / range, 0.0);
                lit += float4(u.OmniCol[i]).rgb * max(dot(n, d / dist), 0.0) * att * att;
            }
        }
        for (int j = 0; j < 3; j++) {
            float4 sp = float4(u.SpotPos[j]);
            float range = sp.w;
            if (range > 0.0) {
                float3 d = sp.xyz - in.wpos;
                float dist = length(d) + 0.001;
                float3 l = d / dist;
                float att = max(1.0 - dist / range, 0.0);
                float4 sd = float4(u.SpotDir[j]);
                float co = sd.w;
                float cone = smoothstep(co, co + 0.1, dot(-l, sd.xyz));
                lit += float4(u.SpotCol[j]).rgb * max(dot(n, l), 0.0) * att * att * cone;
            }
        }
        outc = albedo * lit;
    }
    if (u.FogDensity > 0.0) {
        float fd = length(in.wpos - float3(u.CamPos));
        float f = clamp(1.0 - exp(-u.FogDensity * fd), 0.0, 1.0);
        outc = mix(outc, float3(u.FogCol), f);
    }
    return float4(outc, 1.0);
}
"#;

/// True when this run draws through Metal (macOS with the Metal window). Call only once the window exists.
fn is_metal() -> bool {
    #[cfg(target_os = "macos")]
    {
        macroquad::miniquad::window::apple_gfx_api() == macroquad::miniquad::conf::AppleGfxApi::Metal
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The material's own uniforms, in declaration order (the byte order of the MSL struct).
fn uniform_descs() -> Vec<UniformDesc> {
    vec![
        UniformDesc::new("CamPos", UniformType::Float3),
        UniformDesc::new("Ambient", UniformType::Float3),
        UniformDesc::new("OmniPos", UniformType::Float4).array(MAX_OMNI),
        UniformDesc::new("OmniCol", UniformType::Float4).array(MAX_OMNI),
        UniformDesc::new("SpotPos", UniformType::Float4).array(MAX_SPOT),
        UniformDesc::new("SpotDir", UniformType::Float4).array(MAX_SPOT),
        UniformDesc::new("SpotCol", UniformType::Float4).array(MAX_SPOT),
        UniformDesc::new("FogCol", UniformType::Float3),
        UniformDesc::new("FogDensity", UniformType::Float1),
    ]
}

/// The names of the uniforms the material declares (also asserted by tests against the shader source).
pub const UNIFORM_NAMES: [&str; 10] =
    ["CamPos", "Ambient", "OmniPos", "OmniCol", "SpotPos", "SpotDir", "SpotCol", "FogCol", "FogDensity", "Texture"];

pub struct LitShader {
    mat: Material,
}

impl LitShader {
    /// Needs a live GL context. Panics when the shader fails to compile (a bug, not a runtime condition).
    pub fn new() -> LitShader {
        let source = if is_metal() {
            ShaderSource::Msl { program: MSL }
        } else {
            ShaderSource::Glsl { vertex: VERTEX, fragment: FRAGMENT }
        };
        let mat = load_material(
            source,
            MaterialParams {
                pipeline_params: PipelineParams {
                    depth_write: true,
                    depth_test: Comparison::LessOrEqual,
                    ..Default::default()
                },
                uniforms: uniform_descs(),
                ..Default::default()
            },
        )
        .expect("night watch lit shader");
        LitShader { mat }
    }

    /// Upload the lights and make this the active material.
    pub fn begin(&self, l: &LightSet, cam_pos: Vec3) {
        let p = pack_lights(l);
        self.mat.set_uniform("CamPos", cam_pos);
        self.mat.set_uniform("Ambient", p.ambient);
        self.mat.set_uniform_array("OmniPos", &p.omni_pos[..]);
        self.mat.set_uniform_array("OmniCol", &p.omni_col[..]);
        self.mat.set_uniform_array("SpotPos", &p.spot_pos[..]);
        self.mat.set_uniform_array("SpotDir", &p.spot_dir[..]);
        self.mat.set_uniform_array("SpotCol", &p.spot_col[..]);
        self.mat.set_uniform("FogCol", p.fog_col);
        self.mat.set_uniform("FogDensity", p.fog_density);
        gl_use_material(&self.mat);
    }

    pub fn end(&self) {
        gl_use_default_material();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_source_declares_every_uniform() {
        for n in UNIFORM_NAMES.iter() {
            assert!(FRAGMENT.contains(n), "fragment source lacks {n}");
        }
        assert!(FRAGMENT.contains("OmniPos[4]") && FRAGMENT.contains("SpotPos[3]"));
        assert!(FRAGMENT.contains("nrm.w > 0.5"));
        assert!(VERTEX.contains("attribute vec4 normal;"));
    }

    #[test]
    fn metal_twin_matches_the_glsl_constants_and_uniform_order() {
        assert!(MSL.contains("packed_float4 OmniPos[4];") && MSL.contains("packed_float4 SpotPos[3];"));
        assert!(MSL.contains("in.nrm.w > 0.5") && MSL.contains("tex.a < 0.5"));
        assert!(MSL.contains("vertexShader") && MSL.contains("fragmentShader"));
        assert_eq!(uniform_descs().len(), UNIFORM_NAMES.len() - 1, "every uniform but the texture");
        let mut last = 0;
        for n in UNIFORM_NAMES.iter().filter(|n| **n != "Texture") {
            let at = MSL.find(&format!(" {n}")).unwrap_or_else(|| panic!("MSL lacks {n}"));
            assert!(at >= last, "MSL field {n} is out of order");
            last = at;
        }
    }

    #[test]
    fn defaults() {
        let o = LightSet::office_default();
        assert_eq!(o.ambient, [0.38, 0.40, 0.55]);
        assert!(o.omni.is_empty() && o.spot.is_empty() && o.fog.is_none());
        let f = LightSet::feed();
        assert!((f.ambient[0] - 1.0925).abs() < 1e-5);
        assert!((f.ambient[1] - 1.035).abs() < 1e-5);
        assert!((f.ambient[2] - 0.92).abs() < 1e-5);
    }

    #[test]
    fn packing_multiplies_colour_by_energy_and_caps_counts() {
        let mut l = LightSet::office_default();
        for i in 0..6 {
            l.omni.push(Omni { pos: vec3(i as f32, 1.0, 2.0), color: [1.0, 0.5, 0.25], energy: 2.0, range: 5.0 });
        }
        l.spot.push(Spot {
            pos: vec3(0.0, 3.0, 0.0),
            dir: vec3(0.0, -2.0, 0.0),
            color: [1.0, 1.0, 1.0],
            energy: 5.0,
            range: 9.0,
            cos_outer: 0.8,
        });
        l.fog = Some(([0.1, 0.2, 0.3], 0.05));
        let p = pack_lights(&l);
        assert_eq!(p.omni_pos[3], vec4(3.0, 1.0, 2.0, 5.0));
        assert_eq!(p.omni_col[0], vec4(2.0, 1.0, 0.5, 0.0));
        assert_eq!(p.spot_dir[0], vec4(0.0, -1.0, 0.0, 0.8));
        assert_eq!(p.spot_col[0].x, 5.0);
        assert_eq!(p.spot_pos[1], Vec4::ZERO, "unused spot slot has range 0");
        assert_eq!(p.fog_density, 0.05);
    }
}
