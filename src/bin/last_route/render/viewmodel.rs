//! The tool in the runner's hand: the original's item models (assets/interactables/*.bbmodel) placed by their grip exactly as
//! held_item_poses.gd lays them out, moved by the player controller's tweens (render/anim/held.rs: the crowbar / axe swing,
//! the taser kick, the torch click) and slid back from walls, drawn into their own layer so walls never cut through them.

use super::anim::held::{self, HeldId, HeldState, Motion};
use super::anim::hide;
use super::builder::Builder;
use crate::item_path;
use super::scene::{camera_from_view, flickered, item_file, item_model, lighting_from_view, view, BATTERY};
use super::shader;
use crate::shared::assets::Assets;
use crate::config::WALL_HEIGHT;
use crate::game::level_stack::{self, LevelStack};
use crate::game::player::stair_height_at;
use crate::game::{Game, Item};
use crate::world::Facility;
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
        // posed from their data rows (fp_hands::item_pose via held::pose), no swing
        Item::Lockpick => HeldId::Lockpick,
        Item::ReviveKit => HeldId::ReviveKit,
    }
}

/// Distance to the first wall (or the floor / ceiling) straight ahead of the camera, up to VIEWMODEL_PUSH_RANGE.
///
/// On the infinite route the probe runs in the absolute frame (`floor_y + eye.y`, which a relabel does not change), so a
/// body on a flight, below its level's floor line, or on the frame of a swap is not pinned by a floor or ceiling that is
/// not there. The finite routes keep the level-local test.
fn wall_ahead(g: &Game, eye: Vec3, fwd: Vec3) -> Option<f32> {
    let Some(inf) = g.inf.as_ref() else {
        return wall_ahead_local(&g.fac, eye, fwd);
    };
    // the feet off their level's floor line: on a flight (or the Inferno's noisy floor), where the tread test covers the slabs
    let feet = g.p.y();
    let on_flight = (level_stack::ground_y(&g.fac, &inf.stack, g.p.pos.0, g.p.pos.1, feet).y
        - level_stack::floor_y_of(level_stack::owning_level(feet)))
    .abs()
        > 0.02;
    wall_ahead_in(&g.fac, &inf.stack, vec3(eye.x, g.p.floor_y + eye.y, eye.z), fwd, on_flight)
}

/// The level-local probe of a single floor: floor at y 0, ceiling at WALL_HEIGHT, walls of `fac`.
fn wall_ahead_local(fac: &Facility, eye: Vec3, fwd: Vec3) -> Option<f32> {
    let mut d = 0.0;
    while d <= held::VIEWMODEL_PUSH_RANGE {
        let p = eye + fwd * d;
        if p.y <= 0.0 || p.y >= WALL_HEIGHT || fac.is_wall_cell(p.x.floor() as i32, p.z.floor() as i32) {
            return Some(d);
        }
        d += 0.02;
    }
    None
}

/// Whether `fac` has a walkable surface at (x, z): a flight tread or a cell that is not a hole.
fn has_surface(fac: &Facility, x: f32, z: f32) -> bool {
    stair_height_at(fac, (x, z)).is_some() || !fac.is_hole(x.floor() as i32, z.floor() as i32)
}

/// Whether the slab between `up_level - 1` and `up_level` is solid at (x, z): the upper level's floor (a stairwell or
/// hole is open), else, with that level not resident, the lower level's own ceiling (open over its flights).
fn slab_solid(fac: &Facility, stack: &LevelStack, up_level: i32, x: f32, z: f32) -> bool {
    if let Some(up) = stack.fac_for(fac, up_level) {
        return !has_flight_or_hole(up, x, z);
    }
    match stack.fac_for(fac, up_level - 1) {
        Some(lo) => stair_height_at(lo, (x, z)).is_none(),
        None => false,
    }
}

fn has_flight_or_hole(fac: &Facility, x: f32, z: f32) -> bool {
    stair_height_at(fac, (x, z)).is_some() || fac.is_hole(x.floor() as i32, z.floor() as i32)
}

/// The probe in the absolute frame: `eye` carries the absolute y. Samples every 0.02 m out to VIEWMODEL_PUSH_RANGE (at
/// most 51, no allocation) and returns the first hit: a wall cell of the level owning the sample's height; the surface
/// under it when looking down (tread, floor; open over a drop or the stairwell); the slab above when looking up (open
/// over a stairwell). A level that is not resident is open. `on_flight` skips the slab test.
fn wall_ahead_in(fac: &Facility, stack: &LevelStack, eye: Vec3, fwd: Vec3, on_flight: bool) -> Option<f32> {
    let down = fwd.y < -1e-4;
    let up = fwd.y > 1e-4 && !on_flight;
    let mut prev = level_stack::owning_level(eye.y);
    let mut d = 0.0;
    while d <= held::VIEWMODEL_PUSH_RANGE {
        let (x, y, z) = (eye.x + fwd.x * d, eye.y + fwd.y * d, eye.z + fwd.z * d);
        let level = level_stack::owning_level(y);
        if stack
            .fac_for(fac, level)
            .is_some_and(|f| f.is_wall_cell(x.floor() as i32, z.floor() as i32))
        {
            return Some(d);
        }
        if down {
            let gr = level_stack::ground_y(fac, stack, x, z, y);
            if y <= gr.y
                && gr.y <= y + level_stack::STEP_UP
                && stack.fac_for(fac, gr.level).is_some_and(|f| has_surface(f, x, z))
            {
                return Some(d);
            }
        }
        if up && level > prev && slab_solid(fac, stack, level, x, z) {
            return Some(d);
        }
        prev = level;
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
        if let Some(m) = item_model(a, item_path!("flashlight_item")) {
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

    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::level_stack::Resident;
    use crate::world::catalog::Catalog;
    use crate::world::facility::{LinkKind, Options, Stair};

    /// A level-0 window with a flight going down and its level -1 neighbour built beside it.
    struct Rig {
        g: Game,
        stack: LevelStack,
        s: Stair,
    }

    fn rig(n: usize, content: &Content, cat: &Catalog) -> Option<Rig> {
        let g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-VM-{n}"), 0, content, cat);
        if g.fac.level != 0 {
            return None;
        }
        let s = g.fac.stairs.iter().copied().find(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs))?;
        let inf = g.inf.as_ref()?;
        let (seed, center) = (inf.seed.clone(), inf.center);
        let mut cache = std::collections::HashMap::new();
        let (fac, fix) = crate::world::infinite::window(&seed, center, -1, cat, &content.loot, &mut cache);
        let mut stack = LevelStack::default();
        stack.put(Resident { level: -1, center, fac, fix });
        Some(Rig { g, stack, s })
    }

    /// No wall cell of the active level or the resident one under the 1 m probe.
    fn open_ahead(r: &Rig, eye: Vec3, fwd: Vec3) -> bool {
        let lo = &r.stack.get(-1).expect("resident").fac;
        let mut d = 0.0;
        while d <= held::VIEWMODEL_PUSH_RANGE {
            let (x, z) = ((eye.x + fwd.x * d).floor() as i32, (eye.z + fwd.z * d).floor() as i32);
            if r.g.fac.is_wall_cell(x, z) || lo.is_wall_cell(x, z) {
                return false;
            }
            d += 0.02;
        }
        true
    }

    /// Eyes (absolute) on the tread of the flight where it is about `target` m (level-0 frame), looking along the flight
    /// (`open_only`: only where nothing is in the way for 1 m), with the tread height.
    fn tread_cases(r: &Rig, target: f32, open_only: bool) -> Vec<(Vec3, Vec3, f32)> {
        let eye_h = r.g.p.eye_height();
        let path = r.s.walk_path(0, 0.25);
        let mut out = Vec::new();
        for i in 0..path.len().saturating_sub(1) {
            let p = path[i];
            let Some(t) = stair_height_at(&r.g.fac, p) else { continue };
            if (t - target).abs() > 0.1 {
                continue;
            }
            let (dx, dz) = (path[i + 1].0 - p.0, path[i + 1].1 - p.1);
            let l = (dx * dx + dz * dz).sqrt();
            if l < 1e-3 {
                continue;
            }
            let (eye, fwd) = (vec3(p.0, t + eye_h, p.1), vec3(dx / l, 0.0, dz / l));
            if !open_only || open_ahead(r, eye, fwd) {
                out.push((eye, fwd, t));
            }
        }
        out
    }

    fn plain() -> Facility {
        let mut f = Facility::generate("GROUND-1", &Options::default(), &Catalog::load());
        f.level = 0;
        f
    }

    /// An open floor cell (not a hole or a flight) whose four neighbours are open too: its centre.
    fn open_cell(f: &Facility) -> (f32, f32) {
        for cz in 1..f.rows - 1 {
            for cx in 1..f.cols - 1 {
                let c = (cx as f32 + 0.5, cz as f32 + 0.5);
                if !f.is_wall_cell(cx, cz)
                    && !f.is_hole(cx, cz)
                    && stair_height_at(f, c).is_none()
                    && !f.is_wall_cell(cx + 1, cz)
                    && !f.is_wall_cell(cx - 1, cz)
                    && !f.is_wall_cell(cx, cz + 1)
                    && !f.is_wall_cell(cx, cz - 1)
                {
                    return c;
                }
            }
        }
        panic!("no open cell");
    }

    #[test]
    fn mid_flight_the_item_is_not_pinned_by_the_missing_floor() {
        let (content, cat) = (Content::load(), Catalog::load());
        let mut checked = 0;
        for n in 0..40 {
            let Some(r) = rig(n, &content, &cat) else { continue };
            for target in [-1.8_f32, -2.06] {
                for (eye, fwd, _) in tread_cases(&r, target, true) {
                    assert!(eye.y < 0.0, "the eye is below the level's floor line");
                    assert!(wall_ahead_local(&r.g.fac, eye, fwd).is_some(), "the old level-local probe pinned it");
                    for on_flight in [true, false] {
                        assert_eq!(wall_ahead_in(&r.g.fac, &r.stack, eye, fwd, on_flight), None, "neighbour resident");
                        assert_eq!(wall_ahead_in(&r.g.fac, &LevelStack::EMPTY, eye, fwd, on_flight), None, "neighbour absent");
                    }
                    checked += 1;
                }
            }
            if checked > 0 {
                break;
            }
        }
        assert!(checked > 0, "no test window has an open stretch of flight");
    }

    /// Puts the body on the flight at about `target`, labelled from `from_floor`, and checks `wall_ahead` is the same
    /// before and after relabelling to `to_floor` (the same absolute eye), and open.
    fn flip_case(target: f32, from_floor: f32, to_floor: f32) {
        let (content, cat) = (Content::load(), Catalog::load());
        for n in 0..40 {
            let Some(mut r) = rig(n, &content, &cat) else { continue };
            let Some(&(eye, fwd, t)) = tread_cases(&r, target, true).first() else { continue };
            let stack = std::mem::take(&mut r.stack);
            r.g.inf.as_mut().expect("infinite").stack = stack;
            let p = &mut r.g.p;
            p.pos = (eye.x, eye.z);
            p.floor_y = from_floor;
            p.height = t - from_floor;
            p.feet_y = 0.0;
            p.water_sink = 0.0;
            p.moving = false;
            let e1 = r.g.eye();
            let before = wall_ahead(&r.g, e1, fwd);
            r.g.p.rebase_floor(to_floor);
            let e2 = r.g.eye();
            assert!((e1.y + from_floor - (e2.y + to_floor)).abs() < 1e-3, "same absolute eye");
            let after = wall_ahead(&r.g, e2, fwd);
            assert_eq!(before, after, "the relabel does not change the probe");
            assert_eq!(after, None, "mid-flight, level look: nothing pushes the item");
            return;
        }
        panic!("no test window has an open stretch of flight at {target}");
    }

    #[test]
    fn the_relabel_flip_frame_gives_the_same_result_going_down() {
        flip_case(-1.8, 0.0, -5.0);
    }

    #[test]
    fn going_up_the_flipped_frame_is_open_too() {
        // climbing the lower flight: labelled from level -1, relabelled to level 0 with the height about -2.06
        flip_case(-2.06, -5.0, 0.0);
    }

    #[test]
    fn under_a_real_slab_looking_up_the_item_is_pushed() {
        let f = plain();
        let (x, z) = open_cell(&f);
        // 0.6 m of room under the slab at 5 m
        let d = wall_ahead_in(&f, &LevelStack::EMPTY, vec3(x, WALL_HEIGHT - 0.6, z), vec3(0.0, 1.0, 0.0), false)
            .expect("the ceiling is hit");
        assert!((d - 0.6).abs() < 0.03, "d {d}");
        // out of reach of the slab: open
        assert_eq!(wall_ahead_in(&f, &LevelStack::EMPTY, vec3(x, 2.0, z), vec3(0.0, 1.0, 0.0), false), None);
    }

    #[test]
    fn over_the_stairwell_looking_up_the_item_is_not_pushed() {
        let (content, cat) = (Content::load(), Catalog::load());
        for n in 0..40 {
            let Some(r) = rig(n, &content, &cat) else { continue };
            let path = r.s.walk_path(0, 0.25);
            let p = path[path.len() / 2];
            assert!(stair_height_at(&r.g.fac, p).is_some());
            let eye = vec3(p.0, WALL_HEIGHT - 0.6, p.1);
            assert_eq!(wall_ahead_in(&r.g.fac, &r.stack, eye, vec3(0.0, 1.0, 0.0), false), None, "open well, no slab");
            return;
        }
        panic!("no test window has a flight");
    }

    #[test]
    fn a_stair_side_wall_beside_the_tread_still_pushes() {
        let (content, cat) = (Content::load(), Catalog::load());
        let mut found = 0;
        for n in 0..60 {
            let Some(r) = rig(n, &content, &cat) else { continue };
            let lo = &r.stack.get(-1).expect("resident").fac;
            for target in [-1.8_f32, -2.06] {
                for (eye, _, _) in tread_cases(&r, target, false) {
                    for k in 0..16 {
                        let a = k as f32 * std::f32::consts::TAU / 16.0;
                        let fwd = vec3(a.cos(), 0.0, a.sin());
                        // the first wall cell of the level owning the eye, sampled as the probe does
                        let mut expected = None;
                        let mut d = 0.0;
                        while d <= held::VIEWMODEL_PUSH_RANGE {
                            if lo.is_wall_cell((eye.x + fwd.x * d).floor() as i32, (eye.z + fwd.z * d).floor() as i32) {
                                expected = Some(d);
                                break;
                            }
                            d += 0.02;
                        }
                        if let Some(e) = expected {
                            assert_eq!(wall_ahead_in(&r.g.fac, &r.stack, eye, fwd, true), Some(e));
                            found += 1;
                        }
                    }
                }
            }
            if found > 0 {
                break;
            }
        }
        assert!(found > 0, "no wall beside any tread in the test windows");
    }

    #[test]
    fn standing_on_a_floor_looking_down_still_pushes_as_before() {
        let f = plain();
        let (x, z) = open_cell(&f);
        let eye = vec3(x, 0.7, z);
        let fwd = vec3(0.3, -1.0, 0.0).normalize();
        let new = wall_ahead_in(&f, &LevelStack::EMPTY, eye, fwd, false).expect("the floor is hit");
        let old = wall_ahead_local(&f, eye, fwd).expect("the old probe hit it");
        assert!((new - old).abs() <= 0.021, "new {new} old {old}");
        assert!((new - 0.7 / fwd.y.abs()).abs() < 0.04, "about where the ray meets the floor: {new}");
    }

    #[test]
    fn looking_at_a_wall_pushes_with_the_same_magnitude_as_before() {
        let f = plain();
        for cz in 1..f.rows - 1 {
            for cx in 1..f.cols - 1 {
                if f.is_wall_cell(cx, cz) || !f.is_wall_cell(cx + 1, cz) {
                    continue;
                }
                let eye = vec3(cx as f32 + 0.5, 1.7, cz as f32 + 0.5);
                let fwd = vec3(1.0, 0.0, 0.0);
                let new = wall_ahead_in(&f, &LevelStack::EMPTY, eye, fwd, false);
                assert!(new.is_some());
                assert_eq!(new, wall_ahead_local(&f, eye, fwd));
                return;
            }
        }
        panic!("no wall beside an open cell");
    }

    #[test]
    fn the_lockpick_and_the_revive_kit_hold_their_own_poses() {
        assert_eq!(held_id(Item::Lockpick), HeldId::Lockpick);
        assert_eq!(held_id(Item::ReviveKit), HeldId::ReviveKit);
        let lp = crate::render::fp_hands::item_pose("lockpick").expect("row");
        let p = held::pose(held_id(Item::Lockpick));
        assert_eq!((p.view, p.scale, p.yaw, p.pitch), (lp.view, lp.scale, lp.yaw, lp.pitch));
        let rk = crate::render::fp_hands::item_pose("revive_kit").expect("row");
        let p = held::pose(held_id(Item::ReviveKit));
        assert_eq!((p.view, p.scale, p.yaw, p.pitch), (rk.view, rk.scale, rk.yaw, rk.pitch));
    }

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
