//! The tool in the runner's hand: the original's item models (assets/interactables/*.bbmodel) placed by their grip exactly as
//! held_item_poses.gd lays them out, moved by the player controller's tweens (render/anim/held.rs: the crowbar / axe swing,
//! the taser kick, the torch click) and slid back from walls, drawn into their own layer so walls never cut through them.

use super::anim::held::{self, HeldId, HeldState, Motion};
use super::anim::hide;
use super::builder::Builder;
use super::scene::{camera, item_file, item_model, lighting, view, BATTERY};
use super::shader;
use crate::shared::assets::Assets;
use crate::config::WALL_HEIGHT;
use crate::game::{Game, Item};
use macroquad::prelude::*;
use std::cell::RefCell;

thread_local! {
    static HELD: RefCell<HeldState> = RefCell::new(HeldState { last_time: f32::NAN, ..Default::default() });
}

fn held_id(item: Item) -> HeldId {
    match item {
        Item::Flashlight => HeldId::Flashlight,
        Item::Taser => HeldId::Taser,
        Item::Crowbar => HeldId::Crowbar,
        Item::Axe => HeldId::RescueAxe,
        Item::Battery => HeldId::Battery,
        Item::Stim => HeldId::StimPack,
        Item::MusicBox => HeldId::MusicBox,
    }
}

/// Distance to the first wall (or the floor / ceiling) straight ahead of the camera, up to VIEWMODEL_PUSH_RANGE.
fn wall_ahead(g: &Game, eye: Vec3, fwd: Vec3) -> Option<f32> {
    let mut d = 0.0;
    while d <= held::VIEWMODEL_PUSH_RANGE {
        let p = eye + fwd * d;
        if p.y <= 0.0
            || p.y >= WALL_HEIGHT
            || g.fac.is_wall_cell(p.x.floor() as i32, p.z.floor() as i32)
        {
            return Some(d);
        }
        d += 0.02;
    }
    None
}

/// The tween running on the held item this frame, started by the game's own signals (a new swing, a taser shot, the torch
/// switched), and the eased wall push.
fn motion_now(g: &Game, item: Item, eye: Vec3, fwd: Vec3) -> (Option<(HeldId, Motion, f32)>, f32) {
    HELD.with(|s| {
        let mut s = s.borrow_mut();
        let delta = if s.last_time.is_nan() || g.time < s.last_time {
            0.0
        } else {
            (g.time - s.last_time).min(0.1)
        };
        s.last_time = g.time;
        let id = held_id(item);
        if g.swing_t > s.last_swing_t + 1e-4 {
            s.motion = Some((
                id,
                if held::pose(id).swing.is_some() {
                    Motion::Swing
                } else {
                    Motion::Use
                },
                g.time,
            ));
        }
        if g.taser_fx > s.last_taser_fx + 1e-4 {
            s.motion = Some((HeldId::Taser, Motion::TaserKick, g.time));
        }
        if item == Item::Flashlight && s.last_torch.is_some_and(|t| t != g.p.flashlight_on) {
            s.motion = Some((HeldId::Flashlight, Motion::TorchClick, g.time));
        }
        s.last_swing_t = g.swing_t;
        s.last_taser_fx = g.taser_fx;
        s.last_torch = Some(g.p.flashlight_on);
        if let Some((mid, m, t0)) = s.motion {
            if g.time - t0 > held::duration(mid, m) || g.time < t0 {
                s.motion = None;
            }
        }
        // _update_viewmodel_push: out fast (3/s), back slowly (1.4/s)
        let wanted = wall_ahead(g, eye, fwd)
            .map(|d| held::push_for(d, held::pose(id).reach))
            .unwrap_or(0.0);
        let rate = if wanted > s.push { 3.0 } else { 1.4 };
        let step = delta * rate;
        s.push = if (wanted - s.push).abs() <= step {
            wanted
        } else {
            s.push + (wanted - s.push).signum() * step
        };
        (s.motion.map(|(mid, m, t0)| (mid, m, g.time - t0)), s.push)
    })
}

pub fn draw_viewmodel(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget) {
    set_camera(&camera(g, rt));
    clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
    if hide::player_now().weight >= 1.0 || g.report.is_some() || g.escape.is_some() || g.ladder.is_some() {
        return;
    }
    let Some(item) = g.selected_item() else {
        return;
    };
    gl_use_material(mat);
    shader::apply(mat, &lighting(g), &super::scene::flickered(g, &g.world.lamps));
    let (eye, fwd, right, up) = view(g);
    let (motion, push) = motion_now(g, item, eye, fwd);
    // the taser kick shows the taser whatever was held (_animate_taser_fire makes its view visible)
    let shown = match motion {
        Some((HeldId::Taser, Motion::TaserKick, _)) => Item::Taser,
        _ => item,
    };
    let id = held_id(shown);
    let p = held::pose(id);
    // camera space (x right, y up, -z forward) to world
    let cam = Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (-fwd).extend(0.0),
        eye.extend(1.0),
    );
    let (off, rx, rz) = match motion {
        Some((mid, m, t)) if mid == id => held::view_offset(id, m, t),
        _ => (Vec3::ZERO, 0.0, 0.0),
    };
    // FirstPersonItems (scale PlayerModels.SCALE, slid back by the push) > the item's view node (hand + tween offset,
    // rotation (x, 0, z)) > the model placed by its grip
    let items = Mat4::from_translation(held::push_offset(push))
        * Mat4::from_scale(Vec3::splat(held::PLAYER_MODELS_SCALE));
    let view = Mat4::from_translation(p.view + off)
        * Mat4::from_rotation_x(rx)
        * Mat4::from_rotation_z(rz);
    let xf = cam * items * view * held::model_transform(id);
    // the runner's own forearm and hand under the item, its prop point on the grip (none while hiding or held: returned above)
    if g.p.hidden.is_none() {
        super::fp_hands::draw_item_hand(a, cam * items * view);
    }
    match item_model(a, item_file(shown)) {
        Some(m) => m.draw(xf, WHITE),
        None => {
            let mut b = Builder::new(Some(a.white.clone()));
            b.cube_basis(
                xf.transform_point3(vec3(0.0, 0.2, 0.0)),
                vec3(0.06, 0.06, 0.3),
                Mat3::from_mat4(cam),
                BATTERY,
                0.0,
            );
            b.draw_and_clear();
        }
    }
    // the lens glows while the flashlight is on
    if shown == Item::Flashlight && g.p.beam_lit() {
        if let Some(m) = item_model(a, "flashlight_item") {
            let tip = xf.transform_point3(vec3(0.0, m.max.y, 0.0));
            let mut b = Builder::new(Some(a.white.clone()));
            b.cube_basis(
                tip,
                vec3(0.035, 0.035, 0.008),
                Mat3::from_mat4(cam),
                Color::new(1.0, 0.96, 0.8, 1.0),
                2.0,
            );
            b.draw_and_clear();
        }
    }
    gl_use_default_material();
}
