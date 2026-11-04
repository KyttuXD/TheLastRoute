//! The skin menu's rotating 3D mannequin. The runner body (or its slim variant) is scaled to the preview height, posed
//! by the rig's idle clip (rest pose when the rig is missing), turned at a data rate, and drawn unlit with its own small
//! camera into an offscreen target that is blitted into the menu's rectangle. Every number lives in
//! data/render/skin_preview.json (Godot: game/ui/skin_panel.gd).

use crate::render::anim::clip::{self, Rig};
use crate::render::anim::player::AnimPlayer;
use crate::render::bbmodel::Model;
use crate::render::shader;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use std::f32::consts::TAU;
use std::rc::Rc;

const DATA: &str = include_str!("../data/render/skin_preview.json");

/// The preview's settings, from skin_preview.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub model: String,
    pub slim_model: String,
    pub rig: String,
    pub idle_clip: String,
    pub height_m: f32,
    pub turn_deg_per_sec: f32,
    pub start_deg: f32,
    pub distance: f32,
    pub look_height_frac: f32,
    pub fov_deg: f32,
    pub background: [f32; 3],
    pub resolution_scale: f32,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v[key].as_f64().map(|n| n as f32).ok_or_else(|| format!("{key} is missing or not a number"))
}

fn text(v: &serde_json::Value, key: &str) -> Result<String, String> {
    v[key].as_str().map(str::to_string).ok_or_else(|| format!("{key} is missing or not a string"))
}

impl Config {
    /// Reads the file. A missing or bad value is an error, never a silent default.
    pub fn parse(src: &str) -> Result<Config, String> {
        let v: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        let cam = &v["camera"];
        let bg = v["background"].as_array().ok_or("background is missing")?;
        if bg.len() != 3 {
            return Err("background needs three numbers".into());
        }
        let ch = |i: usize| bg[i].as_f64().map(|n| n as f32).ok_or("background holds a non-number");
        let c = Config {
            model: text(&v, "model")?,
            slim_model: text(&v, "slim_model")?,
            rig: text(&v, "rig")?,
            idle_clip: text(&v, "idle_clip")?,
            height_m: num(&v, "height_m")?,
            turn_deg_per_sec: num(&v, "turn_deg_per_sec")?,
            start_deg: num(&v, "start_deg")?,
            distance: num(cam, "distance")?,
            look_height_frac: num(cam, "look_height_frac")?,
            fov_deg: num(cam, "fov_deg")?,
            background: [ch(0)?, ch(1)?, ch(2)?],
            resolution_scale: num(&v, "resolution_scale")?,
        };
        if c.height_m <= 0.0 || c.distance <= 0.0 || c.fov_deg <= 0.0 || c.resolution_scale <= 0.0 {
            return Err("height_m, camera.distance, camera.fov_deg and resolution_scale must be above 0".into());
        }
        Ok(c)
    }
}

/// The settings from the embedded file; None (printed) when the file is invalid.
fn config() -> Option<Config> {
    match Config::parse(DATA) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("skin_preview.json: {e}");
            None
        }
    }
}

/// The angle (radians, wrapped into 0..TAU) after `dt` seconds of turning at `deg_per_sec`.
pub fn advance_angle(angle: f32, deg_per_sec: f32, dt: f32) -> f32 {
    (angle + deg_per_sec.to_radians() * dt).rem_euclid(TAU)
}

/// The render target size wanted for a menu rectangle of `w` x `h` at `scale` (at least 16 px each way).
pub fn target_size(w: f32, h: f32, scale: f32) -> (u32, u32) {
    (((w * scale).round() as u32).max(16), ((h * scale).round() as u32).max(16))
}

/// True when the target must be (re)made: there is none yet, or its size differs from the wanted one.
pub fn needs_resize(current: Option<(u32, u32)>, wanted: (u32, u32)) -> bool {
    current != Some(wanted)
}

/// The scale that makes a model `model_height` units tall stand `target_m` metres tall.
pub fn model_scale(model_height: f32, target_m: f32) -> f32 {
    if model_height <= 0.0 {
        return 1.0;
    }
    target_m / model_height
}

/// The rotating mannequin. Create once with [`SkinPreview::new`] and call [`SkinPreview::draw`] every frame.
pub struct SkinPreview {
    target: Option<(RenderTarget, (u32, u32))>,
    /// Turn angle in radians.
    pub angle: f32,
    cfg: Option<Config>,
    mat: Option<Material>,
    slim: Option<Model>,
    slim_tried: bool,
    rig: Option<Rc<Rig>>,
    rig_tried: bool,
    player: AnimPlayer,
}

impl Default for SkinPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinPreview {
    pub fn new() -> Self {
        let cfg = config();
        SkinPreview {
            target: None,
            angle: cfg.as_ref().map(|c| c.start_deg.to_radians()).unwrap_or(0.0),
            cfg,
            mat: None,
            slim: None,
            slim_tried: false,
            rig: None,
            rig_tried: false,
            player: AnimPlayer::default(),
        }
    }

    /// Draws the mannequin into `area`: `skin` is the texture to wear (None draws the atlas), `slim` picks the slim body.
    /// `dt` advances the turn and the idle clip.
    pub fn draw(&mut self, a: &Assets, area: Rect, skin: Option<&Texture2D>, slim: bool, dt: f32) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        self.angle = advance_angle(self.angle, cfg.turn_deg_per_sec, dt);
        if !self.rig_tried {
            self.rig_tried = true;
            self.rig = clip::rig(&cfg.rig);
        }
        if let Some(rig) = self.rig.clone() {
            if rig.has_clip(&cfg.idle_clip) {
                if !self.player.playing {
                    self.player.play(&rig, &cfg.idle_clip, 0.0);
                }
                self.player.advance(&rig, dt);
            }
        }
        if slim && !self.slim_tried {
            self.slim_tried = true;
            self.slim = load_model(&cfg.slim_model, a);
        }
        let wanted = target_size(area.w, area.h, cfg.resolution_scale);
        if needs_resize(self.target.as_ref().map(|t| t.1), wanted) {
            let rt = render_target_ex(wanted.0, wanted.1, RenderTargetParams { sample_count: 1, depth: true });
            rt.texture.set_filter(FilterMode::Nearest);
            self.target = Some((rt, wanted));
        }
        if self.mat.is_none() {
            self.mat = Some(shader::load_world_material());
        }
        let bones_for = |m: &Model| self.bones(m);
        let model = match (slim, self.slim.as_ref()) {
            (true, Some(m)) => Some(m),
            _ => a.npc_body.as_ref(),
        };
        let (Some((rt, _)), Some(mat)) = (self.target.as_ref(), self.mat.as_ref()) else {
            return;
        };
        let bg = Color::new(cfg.background[0], cfg.background[1], cfg.background[2], 1.0);
        let look = vec3(0.0, cfg.height_m * cfg.look_height_frac, 0.0);
        let eye = look + vec3(0.0, 0.0, cfg.distance);
        set_camera(&Camera3D {
            position: eye,
            target: look,
            up: vec3(0.0, 1.0, 0.0),
            fovy: cfg.fov_deg.to_radians(),
            aspect: Some(wanted.0 as f32 / wanted.1 as f32),
            render_target: Some(rt.clone()),
            z_near: 0.05,
            z_far: 40.0,
            ..Default::default()
        });
        clear_background(bg);
        if let Some(m) = model {
            let texture = skin.cloned().unwrap_or_else(|| m.texture.clone());
            let s = model_scale(m.max.y - m.min.y, cfg.height_m);
            let root = Mat4::from_rotation_y(self.angle) * Mat4::from_scale(Vec3::splat(s));
            let bones = bones_for(m);
            let (mut verts, mut idx) = (vec![], vec![]);
            m.build_posed(&mut verts, &mut idx, root, &|k: usize| bones[k], Vec2::ZERO, WHITE);
            for v in &mut verts {
                v.normal.w = 2.0; // the shader's unlit mode
            }
            mat.set_uniform("CamPos", eye);
            mat.set_uniform("FogColor", vec3(bg.r, bg.g, bg.b));
            mat.set_uniform("FogFar", 1000.0f32);
            gl_use_material(mat);
            draw_mesh(&Mesh { vertices: verts, indices: idx, texture: Some(texture) });
            gl_use_default_material();
        }
        set_default_camera();
        draw_texture_ex(
            &rt.texture,
            area.x,
            area.y,
            WHITE,
            DrawTextureParams { dest_size: Some(vec2(area.w, area.h)), flip_y: true, ..Default::default() },
        );
    }

    /// The model-space transform of each outliner group from the idle pose; None for a group the rig does not drive
    /// (all None, the rest pose, when the rig or its idle clip is missing).
    fn bones(&self, m: &Model) -> Vec<Option<Mat4>> {
        let Some(rig) = self.rig.as_ref().filter(|_| self.player.playing) else {
            return vec![None; m.groups.len()];
        };
        let world = crate::render::anim::hunter::model_space_stretched(rig, &self.player.pose(rig), &[]);
        m.groups.iter().map(|g| rig.bone(&g.name).and_then(|i| world.get(i).copied())).collect()
    }
}

/// A body model from an embedded asset path, scaled to the runner height like `Assets::npc_body`.
fn load_model(path: &str, a: &Assets) -> Option<Model> {
    let text = crate::content::embedded::asset(path).and_then(|b| std::str::from_utf8(b).ok())?;
    let tex = a.npc_body.as_ref().map(|m| m.texture.clone());
    let find = |name: &str| crate::content::embedded::asset(&format!("player/{name}")).map(|b| b.to_vec());
    Model::parse(text, tex, &find, Some(crate::config::PLAYER_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_advances_by_rate_times_dt_and_wraps() {
        let a = advance_angle(0.0, 36.0, 0.5);
        assert!((a - 18.0f32.to_radians()).abs() < 1e-5);
        let w = advance_angle(TAU - 0.01, 90.0, 1.0);
        assert!((0.0..TAU).contains(&w));
        assert!((w - (std::f32::consts::FRAC_PI_2 - 0.01)).abs() < 1e-4);
        assert_eq!(advance_angle(1.0, 0.0, 5.0), 1.0);
    }

    #[test]
    fn area_size_change_asks_for_a_new_target() {
        let want = target_size(300.0, 400.0, 1.0);
        assert!(needs_resize(None, want));
        assert!(!needs_resize(Some(want), want));
        assert!(needs_resize(Some(want), target_size(301.0, 400.0, 1.0)));
        assert_eq!(target_size(0.0, 0.0, 1.0), (16, 16));
        assert_eq!(target_size(100.0, 50.0, 2.0), (200, 100));
    }

    #[test]
    fn json_parses_with_standard_values() {
        let c = Config::parse(DATA).expect("skin_preview.json parses");
        assert!((c.height_m - 1.60).abs() < 1e-6, "the runner is 1.60 m");
        assert!(c.turn_deg_per_sec > 0.0 && c.distance > 0.0 && c.fov_deg > 0.0);
        assert!(c.background.iter().all(|x| (0.0..=1.0).contains(x)));
        assert!(Config::parse("{}").is_err());
        assert!(Config::parse(&DATA.replace("\"height_m\": 1.60", "\"height_m\": 0")).is_err());
    }

    #[test]
    fn scale_makes_the_model_the_target_height() {
        let c = Config::parse(DATA).unwrap();
        for units in [26.0f32, 32.0, 40.5] {
            let s = model_scale(units, c.height_m);
            assert!((units * s - 1.60).abs() < 1e-5);
        }
        assert_eq!(model_scale(0.0, 1.6), 1.0);
    }
}
