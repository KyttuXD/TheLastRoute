//! The tool in the runner's hand: the original's item models (assets/interactables/*.bbmodel) placed by their grip exactly as
//! held_item_poses.gd lays them out, moved by the player controller's tweens (render/anim/held.rs: the crowbar / axe swing,
//! the taser kick, the torch click) and slid back from walls, drawn into their own layer so walls never cut through them.

use super::anim::held::{self, HeldId, HeldState, Motion};
use super::anim::hide;
use super::builder::Builder;
use super::scene::{camera_from_view, flickered, item_file, item_model, lighting_from_view, view, BATTERY};
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

/// Brings the trackers up to the game's current signals without starting a tween, for a frame where the held item is not
/// drawn (hidden, caught, on a ladder), so a swing or torch change made meanwhile does not fire when the item returns.
fn sync_trackers(s: &mut HeldState, swing_t: f32, taser_fx: f32, torch_on: bool) {
    s.last_swing_t = swing_t;
    s.last_taser_fx = taser_fx;
    s.last_torch = Some(torch_on);
    s.motion = None;
    s.last_time = f32::NAN;
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

/// What the first-person layer (the held item and the hands, drawn into their own target so walls never cut through them)
/// shows this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerContent {
    /// Nothing: no item in hand, or the runner is hidden away, caught or dead.
    Empty,
    /// The held item, with the runner's hand under it.
    HeldItem,
    /// The two hands of a ladder climb.
    LadderHands,
}

/// What the layer shows. The held item is drawn unless the runner is fully hidden (`hidden_away`), the run has ended or a
/// hunter holds her, or she is on a ladder (which draws its own hands, `ladder_hands`).
pub fn layer_content(hidden_away: bool, report: bool, held: bool, on_ladder: bool, has_item: bool, ladder_hands: bool) -> LayerContent {
    if !(hidden_away || report || held || on_ladder) && has_item {
        LayerContent::HeldItem
    } else if ladder_hands {
        LayerContent::LadderHands
    } else {
        LayerContent::Empty
    }
}

/// What to do with the layer's target this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerAction {
    /// Nothing: the target is already clear and stays so, so no pass is run and the layer is not blitted.
    Skip,
    /// Clear the target once (it held last frame's item, or is new), and do not blit it.
    ClearOnly,
    /// Draw the layer and blit it.
    Draw,
}

/// Which target the layer is known to be clear for, so an empty layer costs a pass once and not every frame.
pub struct LayerState<K> {
    clean_for: Option<K>,
}

impl<K: PartialEq + Copy> LayerState<K> {
    pub const fn new() -> Self {
        LayerState { clean_for: None }
    }

    /// The action for a frame with `content` going to `target`.
    pub fn next(&mut self, content: LayerContent, target: K) -> LayerAction {
        if content != LayerContent::Empty {
            self.clean_for = None;
            return LayerAction::Draw;
        }
        if self.clean_for == Some(target) {
            LayerAction::Skip
        } else {
            self.clean_for = Some(target);
            LayerAction::ClearOnly
        }
    }

    /// The target's contents are unknown again (it was made anew).
    pub fn forget(&mut self) {
        self.clean_for = None;
    }
}

type LayerTarget = (macroquad::miniquad::TextureId, u32, u32);

thread_local! {
    static LAYER: RefCell<LayerState<LayerTarget>> = const { RefCell::new(LayerState::new()) };
}

/// The render targets were made anew: the layer's contents are unknown until it is cleared once.
pub fn forget_layer() {
    LAYER.with(|l| l.borrow_mut().forget());
}

/// Draws the first-person layer into `rt`: the held item and the hand under it, or the ladder hands, lit once with the
/// room's lamps from one reading of the runner's view. An empty layer is cleared once and then left alone. Returns whether
/// the layer holds anything to blit over the scene.
pub fn draw_view_layer(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget) -> bool {
    let item = g.selected_item();
    let content = layer_content(
        hide::player_now().weight >= 1.0,
        g.report.is_some(),
        g.escape.is_some(),
        g.ladder.is_some(),
        item.is_some(),
        super::fp_hands::ladder_hands_wanted(g),
    );
    if content != LayerContent::HeldItem {
        HELD.with(|s| sync_trackers(&mut s.borrow_mut(), g.swing_t, g.taser_fx, g.p.flashlight_on));
    }
    let target = (rt.texture.raw_miniquad_id(), rt.texture.width() as u32, rt.texture.height() as u32);
    let action = LAYER.with(|l| l.borrow_mut().next(content, target));
    if action == LayerAction::Skip {
        return false;
    }
    let v = view(g);
    set_camera(&camera_from_view(g, rt, v));
    super::draw_stats::note_flush();
    clear_background(Color::new(0.0, 0.0, 0.0, 0.0));
    if action == LayerAction::ClearOnly {
        return false;
    }
    let world = super::pipeline::world_material(mat);
    shader::apply(mat, &lighting_from_view(g, v), &flickered(g, &g.world.lamps));
    match (content, item) {
        (LayerContent::HeldItem, Some(item)) => draw_held_item(g, a, item, v),
        (LayerContent::LadderHands, _) => super::fp_hands::draw_ladder_hands(g, a, v),
        _ => {}
    }
    drop(world);
    true
}

/// The held `item` in the layer, the camera and the material already set.
fn draw_held_item(g: &Game, a: &Assets, item: Item, (eye, fwd, right, up): (Vec3, Vec3, Vec3, Vec3)) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layer_holds_the_item_unless_hidden_caught_or_climbing() {
        // (hidden away, report, held by a hunter, on a ladder, has an item, ladder hands wanted)
        let c = layer_content;
        assert_eq!(c(false, false, false, false, true, false), LayerContent::HeldItem);
        assert_eq!(c(true, false, false, false, true, false), LayerContent::Empty, "hidden away");
        assert_eq!(c(false, true, false, false, true, false), LayerContent::Empty, "the run has ended");
        assert_eq!(c(false, false, true, false, true, false), LayerContent::Empty, "held by a hunter");
        assert_eq!(c(false, false, false, false, false, false), LayerContent::Empty, "nothing in hand");
        // on a ladder the item is not drawn, the hands are
        assert_eq!(c(false, false, false, true, true, true), LayerContent::LadderHands);
        assert_eq!(c(false, false, false, true, false, true), LayerContent::LadderHands);
        assert_eq!(c(false, false, false, true, true, false), LayerContent::Empty);
    }

    #[test]
    fn changes_made_while_the_item_is_not_drawn_start_no_tween_on_return() {
        let mut s = HeldState { last_time: 3.0, last_swing_t: 1.0, last_taser_fx: 0.5, last_torch: Some(true), ..Default::default() };
        s.motion = Some((HeldId::Crowbar, Motion::Swing, 2.9));
        sync_trackers(&mut s, 4.0, 0.9, false);
        assert_eq!((s.last_swing_t, s.last_taser_fx, s.last_torch), (4.0, 0.9, Some(false)));
        assert!(s.motion.is_none(), "a tween from before does not resume");
        assert!(s.last_time.is_nan(), "the first frame back takes no step");
    }

    #[test]
    fn an_empty_layer_costs_one_clear_and_then_nothing() {
        let mut s: LayerState<u32> = LayerState::new();
        // the first empty frame clears whatever the target held (it may be new)
        assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::ClearOnly);
        for _ in 0..5 {
            assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::Skip, "no pass and no blit while nothing is held");
        }
        // drawing dirties it; the first empty frame after clears it again
        assert_eq!(s.next(LayerContent::HeldItem, 1), LayerAction::Draw);
        assert_eq!(s.next(LayerContent::LadderHands, 1), LayerAction::Draw);
        assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::ClearOnly);
        assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::Skip);
    }

    #[test]
    fn a_new_target_or_a_forgotten_one_is_cleared_before_it_is_skipped() {
        let mut s: LayerState<u32> = LayerState::new();
        assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::ClearOnly);
        assert_eq!(s.next(LayerContent::Empty, 1), LayerAction::Skip);
        // the targets were resized: a different target is not known clear
        assert_eq!(s.next(LayerContent::Empty, 2), LayerAction::ClearOnly);
        assert_eq!(s.next(LayerContent::Empty, 2), LayerAction::Skip);
        // made anew under the same id: forgotten
        s.forget();
        assert_eq!(s.next(LayerContent::Empty, 2), LayerAction::ClearOnly);
    }
}
