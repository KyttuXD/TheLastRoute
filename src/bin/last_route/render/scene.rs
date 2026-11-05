//! The per-frame 3D scene: the floor, its fixtures and pickups drawn with the original's
//! authored Blender models (assets/interactables/*.bbmodel: relay, battery, coin, locker, lift,
//! safe zone, vent, music box, heater station, light switch, and the tools), Ravelle's wires,
//! and the hunters in their Blockbench bodies. Simple shapes stand in if a model is missing.

use super::anim::{
    self, grab, hide,
    hunter::{HunterView, Input},
};
use super::builder::{submit_all, Builder};
use super::hunter_cache::{self, HunterMeshCache};
use super::prop_model::{draw_builders, Placement, PropModel, StaticCache};
use super::shader::{self, Lamp, Lighting};
use super::{catwalk, inferno_deco, waterfall, world_stack};
use crate::shared::assets::Assets;
use crate::config::*;
use crate::game::{Game, Heater, Item};
use crate::hunters::brain::State;
use crate::hunters::gimmicks::Gimmick;
use crate::net::runner_figure;
use crate::world::facility::{Cell, DeckKind, Facility};
use crate::world::infinite::INFERNO_LEVEL;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

thread_local! {
    /// The meshes of the static props (relays, batteries, lockers' bodies and the like), kept while they stay drawn.
    static STATIC_PROPS: RefCell<StaticCache<Placement, Vec<Mesh>>> = RefCell::new(StaticCache::default());
    /// The hunters' posed meshes, rebuilt only when a hunter's model, pose or tint changes (render/hunter_cache.rs).
    static HUNTER_MESHES: RefCell<HunterMeshes> = RefCell::new(HunterMeshes::default());
}

/// The hunter mesh cache: slots are keyed by (level, index) and a vanished slot or a new run drops its entry
/// (render/hunter_cache.rs), so a removed hunter's mesh or a stale slot never shows.
#[derive(Default)]
struct HunterMeshes {
    cache: HunterMeshCache,
}

/// Slot offset for the hunters drawn without a rig, so their cache entries never clash with the rigged ones.
const UNRIGGED_SLOT_BASE: usize = 1000;
/// First view slot of the resident floors' hunters (the drawn list's slots are its indexes).
const RESIDENT_VIEW_BASE: usize = 1_000_000;

/// Draws a static prop with its placement: its meshes come from the cache, built on first use.
fn draw_static(m: &PropModel, xf: Mat4, tint: Color) {
    STATIC_PROPS.with(|c| submit_all(m.static_meshes(&mut c.borrow_mut(), xf, tint)));
}

/// How far the fog reaches, metres.
const FOG_RANGE: f32 = 24.0;
pub const FOG: Vec3 = Vec3::new(0.012, 0.008, 0.014);
pub const RELAY: Color = Color::new(0.26, 0.88, 0.92, 1.0);
pub const BATTERY: Color = Color::new(0.95, 0.82, 0.25, 1.0);
const SAFE: Color = Color::new(0.35, 0.95, 0.52, 1.0);
const LIFT: Color = Color::new(0.76, 0.55, 0.96, 1.0);
const LOCKER: Color = Color::new(0.45, 0.60, 0.70, 1.0);
const TOOL: Color = Color::new(0.64, 0.84, 0.96, 1.0);

thread_local! {
    /// The spot the runner hid in last: the leave glide still needs it while `g.p.hidden` is already None.
    static LAST_SPOT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The runner's eye and view basis (forward, right, up): the player's own, or, while hiding (and gliding in or out),
/// the locker's window view from `render/hiding_cam.rs` (the eye at the spot's vent, the look clamped to its cone).
pub fn view(g: &Game) -> (Vec3, Vec3, Vec3, Vec3) {
    let eye = g.eye();
    let (fwd, right, up) = g.camera_basis();
    let h = hide::player_now();
    if let Some(i) = g.p.hidden {
        LAST_SPOT.with(|s| s.set(Some(i)));
    }
    if h.weight <= 0.0 {
        return (eye, fwd, right, up);
    }
    let spot = g.p.hidden.or_else(|| LAST_SPOT.with(|s| s.get()));
    let Some(l) = spot.and_then(|i| g.fac.hiding.get(i)) else {
        return (eye, fwd, right, up);
    };
    let centre = g.fac.locker_center(l);
    let facing = (l.facing.0 as f32, l.facing.1 as f32);
    super::hiding_cam::view_from(eye, h.weight, (g.p.yaw, g.p.pitch), centre, facing, "locker")
}

pub fn camera(g: &Game, rt: &RenderTarget) -> Camera3D {
    let (eye, fwd, _, _) = view(g);
    // held by a hunter: the view rides the grabbed body's eyes and looks at her face (HunterGrab._solve_camera)
    let (position, target, up) = match (g.escape.as_ref(), grab::camera()) {
        (Some(_), Some(c)) => c,
        _ => (eye, eye + fwd, vec3(0.0, 1.0, 0.0)),
    };
    Camera3D {
        position,
        target,
        up,
        fovy: g.fov_deg.to_radians(),
        aspect: Some(rt.texture.width() / rt.texture.height()),
        render_target: Some(rt.clone()),
        z_near: 0.04,
        z_far: 80.0,
        ..Default::default()
    }
}

pub fn lighting(g: &Game) -> Lighting {
    let (eye, fwd, right, up) = view(g);
    Lighting {
        eye,
        flashlight_pos: eye + right * 0.08 - up * 0.05 + fwd * 0.12,
        flashlight_dir: fwd,
        flashlight_on: g.p.beam_lit(),
        // rooms are dark unless something lights them: Godot's RefLook ambient (shader::AMBIENT_COLOR x AMBIENT_ENERGY)
        ambient: shader::AMBIENT_COLOR * shader::AMBIENT_ENERGY,
        fog: FOG,
        fog_far: FOG_RANGE,
    }
}

/// Energy and reach of a grab's fill light (`grab_fill`): Godot's grab staging (dev/look/grabs.gd) puts an omni fill of
/// 1.5 over 5 m on the camera, so the hunter at arm's length and the runner's hands read in a dark room.
pub const GRAB_FILL_ENERGY: f32 = 1.5;
pub const GRAB_FILL_RANGE: f32 = 5.0;

/// The fill light for a grab: on while the escape struggle holds the runner (`held`) and her grab camera is set, off once
/// the grab is released (the camera goes with it). The lamp sits behind and above the runner's eye, as Godot's fill does
/// (camera space (0, .4, .3)), so it lights the grabbed pair. The flashlight is off from the catch (hunting.rs), so this
/// fill and the room's lamps are what light her.
pub fn grab_fill(held: bool, camera: Option<(Vec3, Vec3, Vec3)>) -> Option<Lamp> {
    if !held {
        return None;
    }
    let (eye, target, up) = camera?;
    let fwd = (target - eye).normalize_or_zero();
    Some(Lamp {
        pos: eye - fwd * 0.3 + up * 0.4,
        range: GRAB_FILL_RANGE,
        color: Color::new(1.0, 0.95, 0.86, 1.0),
        energy: GRAB_FILL_ENERGY,
    })
}

/// An interactable's model: assets/interactables/<name>.bbmodel.
pub fn item_model<'a>(a: &'a Assets, name: &str) -> Option<&'a PropModel> {
    a.models.get(&format!("interactables/{name}.bbmodel"))
}

/// The pickup model of an inventory item (route_interactable_3d.gd TOOL_PICKUP_MODELS).
pub fn item_file(item: Item) -> &'static str {
    match item {
        Item::Flashlight => "flashlight_item",
        Item::Taser => "taser_item",
        Item::Crowbar => "crowbar",
        Item::Axe => "rescue_axe",
        Item::Battery => "battery",
        Item::Stim => "stim_pack",
    }
}

fn at(x: f32, y: f32, z: f32, yaw: f32) -> Mat4 {
    Mat4::from_translation(vec3(x, y, z)) * Mat4::from_rotation_y(yaw)
}

pub fn draw_scene(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget) {
    hide::tick(g.p.hidden, g.time);
    STATIC_PROPS.with(|c| c.borrow_mut().begin_frame());
    set_camera(&camera(g, rt));
    clear_background(Color::new(FOG.x, FOG.y, FOG.z, 1.0));
    gl_use_material(mat);
    // the grab's fill joins the room's lamps while the struggle holds the runner (grab_fill); the flashlight is unchanged
    let light = lighting(g);
    // this storey's own lights join the room's lamps: the falls' pool lights, the inferno's lava and cracks, the catwalks' lamps
    let fx = level_fx(g, light.eye);
    let mut extra = fx.lamps.clone();
    extra.extend(grab_fill(g.escape.is_some(), grab::camera()));
    // the resident neighbour's lamps join the room's (the stairwell lamp and the two nearest), see world_stack::merge_lamps
    let mut lamps = if g.world_nb.items.is_empty() {
        g.world.lamps.clone()
    } else {
        let nb = g.world_nb.lamps(&g.fac);
        world_stack::merge_lamps(&g.world.lamps, &nb, light.eye, shader::MAX_LAMPS)
    };
    lamps.extend(extra);
    shader::apply(mat, &light, &lamps);
    // the falls' loop plays from the nearest fall in range, and stops on a storey without one
    {
        let (_, fwd, right, up) = view(g);
        let near = waterfall::nearest_sound(waterfall::config(), &fx.sites, (light.eye.x, light.eye.z), g.time);
        crate::audio::falls::update(near, light.eye.to_array(), right.to_array(), up.to_array(), fwd.to_array());
    }
    // the floor's meshes wholly past the fog's full reach are skipped: they would paint the fog colour alone (see
    // `box_fully_fogged`), which the clear colour already is
    let t_world = crate::app::perf::section("world meshes");
    for (i, m) in g.world.meshes.iter().enumerate() {
        // a mesh without a box (never the case once `bounds` is kept in step) is drawn, so nothing vanishes
        let fogged = g
            .world
            .bounds
            .get(i)
            .is_some_and(|(lo, hi)| box_fully_fogged(light.eye, *lo, *hi, WORLD_CULL_RANGE));
        if !fogged {
            draw_mesh(m);
        }
    }
    // the resident neighbour level, in the same frame (baked at its offset), with the same fog cull and a slab-plane test
    // that skips it unless the eye is near an opening (nothing of it is visible through the slab otherwise)
    if !g.world_nb.items.is_empty() {
        let reach = crate::game::transition::links().prefetch_distance;
        for nb in &g.world_nb.items {
            let openings = super::world_mesh::openings(&g.fac);
            let near_opening = world_stack::wants_neighbour((light.eye.x, light.eye.y, light.eye.z), &openings, reach, world_stack::eye_on_flight(light.eye.y));
            if !near_opening && world_stack::behind_slab(light.eye.y, nb.dy, &nb.bounds) {
                continue;
            }
            for (i, m) in nb.meshes.iter().enumerate() {
                let fogged = nb
                    .bounds
                    .get(i)
                    .is_some_and(|(lo, hi)| box_fully_fogged(light.eye, *lo, *hi, WORLD_CULL_RANGE));
                if !fogged {
                    draw_mesh(m);
                }
            }
        }
    }
    drop(t_world);
    {
        let _t = crate::app::perf::section("doors");
        draw_doors(g, a);
    }
    let t_items = crate::app::perf::section("scene items");
    let mut b = Builder::new(Some(a.white.clone()));
    let t = g.time;
    let f = &g.fac;
    // on the Inferno's noisy floor every item rests on the ground under it
    let ground = |x: f32, z: f32| f.terrain().map_or(0.0, |t| t.floor_at(x, z));

    // safe zone: the authored marker at the spawn, plus a faint ring showing its 2.5-cell radius
    if let Some(s) = f.safe_zone {
        let (sx, sy) = s.center();
        for i in 0..32 {
            let ang = i as f32 / 32.0 * std::f32::consts::TAU;
            let p = vec3(
                sx + ang.cos() * SAFE_ZONE_RADIUS,
                0.01,
                sy + ang.sin() * SAFE_ZONE_RADIUS,
            );
            b.cube(
                p,
                vec3(0.5, 0.015, 0.08),
                -ang + std::f32::consts::FRAC_PI_2,
                Color::new(0.2, 0.6, 0.32, 1.0),
                2.0,
            );
        }
        match item_model(a, "safe_zone") {
            Some(m) => draw_static(m, at(sx, 0.0, sy, 0.0), WHITE),
            // lit (mode 0), so the safe zone's checkpoint light falls on its pad
            None => b.cube(vec3(sx, 0.05, sy), vec3(0.8, 0.1, 0.8), 0.0, SAFE, 0.0),
        }
    }
    for (i, c) in f.relays.iter().enumerate() {
        let (x, y) = c.center();
        let taken = g.relays_taken[i];
        if taken {
            continue;
        }
        match item_model(a, "relay") {
            Some(m) => draw_static(m, at(x, ground(x, y), y, 0.0), WHITE),
            None => b.cube(
                vec3(x, ground(x, y) + 0.32, y),
                vec3(0.42, 0.64, 0.42),
                0.0,
                Color::new(0.22, 0.24, 0.27, 1.0),
                0.0,
            ),
        }
        if !taken {
            // a live signal glows faintly so it reads in the dark
            let pulse = 0.6 + 0.4 * (t * 2.0 + i as f32).sin();
            b.cube(
                vec3(x, ground(x, y) + 0.01, y),
                vec3(0.62, 0.012, 0.62),
                0.0,
                Color::new(
                    RELAY.r * 0.4 * pulse,
                    RELAY.g * 0.4 * pulse,
                    RELAY.b * 0.45 * pulse,
                    1.0,
                ),
                2.0,
            );
        }
    }
    for (i, c) in f.batteries.iter().enumerate() {
        if !g.batteries_taken[i] {
            let (x, y) = c.center();
            match item_model(a, "battery") {
                Some(m) => draw_static(m, at(x, ground(x, y), y, i as f32), WHITE),
                None => battery_shape(&mut b, vec3(x, ground(x, y), y), i as f32),
            }
        }
    }
    for (i, p) in g.pickups.iter().enumerate() {
        if p.taken {
            continue;
        }
        let base = vec3(p.pos.0, ground(p.pos.0, p.pos.1), p.pos.1);
        // lit like the floor (mode 0): unlit, the pad read as a flat blue hole in the floor
        b.cube(
            base + vec3(0.0, 0.006, 0.0),
            vec3(0.56, 0.01, 0.56),
            0.4,
            Color::new(TOOL.r * 0.35, TOOL.g * 0.35, TOOL.b * 0.4, 1.0),
            0.0,
        );
        match item_model(a, item_file(p.item)) {
            // configure_tool_pickup: scale .72 (stim x1.3), lifted .10, tipped -18 deg and turned 25 deg
            Some(m) => {
                let s = 0.72 * if p.item == Item::Stim { 1.3 } else { 1.0 };
                let xf = Mat4::from_translation(base + vec3(0.0, 0.10, 0.0))
                    * Mat4::from_rotation_y(25f32.to_radians() + i as f32 * 1.7)
                    * Mat4::from_rotation_x(-18f32.to_radians())
                    * Mat4::from_scale(Vec3::splat(s));
                draw_static(m, xf, WHITE);
            }
            None => pickup_shape(&mut b, p.item, base, i as f32 * 1.7),
        }
    }
    for (i, c) in g.fix.coins.iter().enumerate() {
        if !g.coins_taken[i] {
            let (x, y) = c.center();
            let bob = (t * 2.0 + i as f32).sin() * 0.03;
            match item_model(a, "coin") {
                Some(m) => m.draw(at(x, ground(x, y) + 0.05 + bob, y, t * 2.0 + i as f32), WHITE),
                None => b.cube(
                    vec3(x, ground(x, y) + 0.12 + bob, y),
                    vec3(0.16, 0.16, 0.03),
                    t * 2.0 + i as f32,
                    Color::new(0.95, 0.78, 0.2, 1.0),
                    2.0,
                ),
            }
        }
    }
    for (i, l) in f.hiding.iter().enumerate() {
        let swing = hide::door_for(i, t);
        if g.p.hidden == Some(i) && swing == 0.0 {
            continue;
        }
        let (cx, cy) = f.locker_center(l);
        let yaw = (l.facing.0 as f32).atan2(l.facing.1 as f32);
        let burnt = g.hiding_disabled[i];
        match item_model(a, "hiding_spot") {
            Some(m) => {
                if locker_fully_fogged(light.eye, cx, cy, locker_reach(m), locker_cull().margin_m) {
                    continue;
                }
                let xf = at(cx, 0.0, cy, yaw);
                let tint = if burnt {
                    Color::new(0.25, 0.2, 0.18, 1.0)
                } else {
                    WHITE
                };
                // the body is fixed; only the door swings, so it is the one part built each frame
                draw_static(m, xf, tint);
                let mut parts = HashMap::new();
                m.add_locker_door(&mut parts, xf, tint, swing);
                draw_builders(parts);
            }
            None => locker_shape(&mut b, vec3(cx, 0.0, cy), yaw, burnt, swing),
        }
    }
    for (i, l) in f.room_lights.iter().enumerate() {
        let (x, y) = l.switch_cell.center();
        let on = g.lights_on.get(i).copied().unwrap_or(true);
        // facility_view_3d.gd: the switch sits .9 up and .34 back from its cell centre
        match item_model(a, "wall_light_switch") {
            Some(m) => draw_static(
                m,
                at(x, 0.9, y - 0.34, 0.0),
                if on {
                    WHITE
                } else {
                    Color::new(0.6, 0.6, 0.6, 1.0)
                },
            ),
            None => b.cube(
                vec3(x, 1.2, y),
                vec3(0.14, 0.2, 0.14),
                0.0,
                Color::new(0.85, 0.82, 0.74, 1.0),
                0.0,
            ),
        }
        if on {
            b.cube(
                vec3(x, 1.0, y - 0.3),
                vec3(0.03, 0.03, 0.03),
                0.0,
                Color::new(0.95, 0.75, 0.3, 1.0),
                2.0,
            );
        }
    }
    if let Some(c) = g.fix.heater {
        let (x, y) = c.center();
        let tint = match g.heater {
            Heater::Frozen => Color::new(0.7, 0.85, 1.0, 1.0),
            Heater::Thawed => WHITE,
            Heater::Powered => Color::new(1.0, 0.85, 0.7, 1.0),
        };
        match item_model(a, "heater_station") {
            Some(m) => draw_static(m, at(x, 0.0, y, 0.0), tint),
            None => b.cube(
                vec3(x, 0.55, y),
                vec3(0.7, 1.1, 0.4),
                0.0,
                Color::new(0.3, 0.28, 0.27, 1.0),
                0.0,
            ),
        }
        if g.heater == Heater::Powered {
            b.cube(
                vec3(x, 0.06, y + 0.3),
                vec3(0.4, 0.02, 0.08),
                0.0,
                Color::new(0.98, 0.55, 0.25, 1.0),
                2.0,
            );
        }
    }
    if let Some(c) = g.fix.music_box {
        let (x, y) = c.center();
        let pct = (g.music_box_left / 45.0).clamp(0.0, 1.0);
        let spin = if pct > 0.0 {
            t * (0.2 + pct * 0.6)
        } else {
            0.0
        };
        match item_model(a, "music_box") {
            Some(m) => m.draw(at(x, 0.0, y, spin), WHITE),
            None => b.cube(
                vec3(x, 0.35, y),
                vec3(0.3, 0.7, 0.3),
                0.0,
                Color::new(0.3, 0.18, 0.12, 1.0),
                0.0,
            ),
        }
    }
    for (i, c) in g.fix.vents.iter().enumerate() {
        let (x, y) = c.center();
        let wall = [(0, -1), (0, 1), (-1, 0), (1, 0)]
            .into_iter()
            .find(|(dx, dy)| f.is_wall_cell(c.x + dx, c.y + dy))
            .unwrap_or((0, -1));
        // the grate faces into the room from its wall
        let yaw = (-(wall.0 as f32)).atan2(-(wall.1 as f32));
        let active = g.p.in_vent && i == g.p.vent_entry;
        match item_model(a, "vent") {
            Some(m) => draw_static(
                m,
                at(x + wall.0 as f32 * 0.42, 0.0, y + wall.1 as f32 * 0.42, yaw),
                if active {
                    Color::new(0.7, 1.0, 0.85, 1.0)
                } else {
                    WHITE
                },
            ),
            None => b.cube(
                vec3(x + wall.0 as f32 * 0.47, 0.42, y + wall.1 as f32 * 0.47),
                vec3(0.6, 0.4, 0.04),
                yaw,
                Color::new(0.35, 0.37, 0.38, 1.0),
                0.0,
            ),
        }
    }
    // the service lift, with its status lamp red until the route signal is complete
    if f.has_lift {
        let (x, y) = f.exit.center();
        let ready = g.relays_collected >= f.relays.len() as i32;
        let lamp = if ready {
            LIFT
        } else {
            Color::new(0.75, 0.16, 0.2, 1.0)
        };
        let top = match item_model(a, "lift") {
            Some(m) => {
                draw_static(m, at(x, 0.0, y, 0.0), WHITE);
                m.max.y
            }
            None => {
                b.cube(
                    vec3(x, 1.4, y),
                    vec3(1.0, 2.8, 1.0),
                    0.0,
                    Color::new(0.22, 0.2, 0.26, 1.0),
                    0.0,
                );
                2.8
            }
        };
        b.cube(
            vec3(x, top.min(3.4) + 0.06, y),
            vec3(0.3, 0.1, 0.3),
            0.0,
            lamp,
            2.0,
        );
    }
    // Ravelle's tripwires, drawn faintly
    for h in g.hunters.iter().filter(|h| h.active) {
        let wire_dy = crate::render::stack_actors::actor_dy(h.y, g.fac.level);
        if let Gimmick::Ravelle(r) = &h.gimmick {
            for w in r.wires.iter().filter(|w| !w.triggered) {
                let a3 = vec3(w.a.0, 0.25 + wire_dy, w.a.1);
                let b3 = vec3(w.b.0, 0.25 + wire_dy, w.b.1);
                let mid = (a3 + b3) * 0.5;
                let len = (b3 - a3).length();
                let yaw = (b3.x - a3.x).atan2(b3.z - a3.z);
                b.cube(
                    mid,
                    vec3(0.012, 0.012, len),
                    yaw,
                    Color::new(0.75, 0.75, 0.7, 1.0),
                    0.0,
                );
            }
        }
    }
    b.draw_and_clear();
    drop(t_items);

    // hunters: posed by their rig (render/anim, the original's clips + view layers). Dormant hunters are drawn too, slumped
    // in their slowed stun clip (HunterView3D.model_alpha: an inactive hunter shows whole).
    let t_hunters = crate::app::perf::section("hunters");
    let (cam_fwd, _, _) = g.camera_basis();
    let runner_feet = vec3(g.p.pos.0, (g.p.height + g.p.feet_y).max(0.0), g.p.pos.1);
    let tune = hunter_cache::tuning();
    // every hunter to pose: the drawn floor's list (dormant ones too), then each resident floor's list. A body on
    // another floor is drawn only when an opening shows it (render/stack_actors.rs).
    let cull = crate::render::stack_actors::ActorCull::new(g);
    let cull_eye = g.eye();
    let mut lists: Vec<(i32, usize)> = vec![(g.fac.level, g.hunters.len() + UNRIGGED_SLOT_BASE)];
    let mut posed: Vec<(i32, usize, usize, &crate::hunters::brain::Brain)> =
        g.hunters.iter().enumerate().map(|(i, h)| (g.fac.level, i, i, h)).collect();
    for l in g.resident_hunters.levels() {
        let list = g.resident_hunters.list(l);
        lists.push((l, list.len() + UNRIGGED_SLOT_BASE));
        for (i, s) in list.iter().enumerate() {
            // the view slot is past the drawn list's, so a rig view never clashes with another floor's
            posed.push((l, i, RESIDENT_VIEW_BASE + posed.len(), &s.brain));
        }
    }
    HUNTER_MESHES.with(|m| m.borrow_mut().cache.sync(&lists, g.run_seed));
    anim::with_views(g.time, |views, dt| {
        let mut stars = Builder::new(Some(a.white.clone()));
        for &(lvl, cslot, slot, h) in posed.iter() {
            if !cull.visible(g, lvl, h.y, h.pos, cull_eye) {
                continue;
            }
            let dy = crate::render::stack_actors::actor_dy(h.y, g.fac.level);
            let faded = h.gimmick.hidden_from_view();
            if faded && !matches!(h.gimmick, Gimmick::Ilyra(_)) {
                continue;
            }
            let id = h
                .disguise
                .as_deref()
                .filter(|d| a.hunters.contains_key(*d))
                .unwrap_or(&h.def.id);
            let id = if a.hunters.contains_key(id) {
                id
            } else {
                "dovelle"
            };
            let Some(model) = a.hunter_model(id) else {
                continue;
            };
            let tint = if faded {
                Color::new(0.35, 0.35, 0.45, 1.0)
            } else if h.stun > 0.0 {
                Color::new(0.7, 0.8, 1.0, 1.0)
            } else {
                WHITE
            };
            let Some(rig) = anim::clip::rig(id) else {
                crate::render::hunter_cache::warn_unrigged_once(id);
                if !h.active {
                    continue;
                }
                let yaw = (-h.facing.0).atan2(-h.facing.1);
                let amp = (h.moving_speed / 3.0).min(1.0) * 0.6;
                // Xivvy's flight height and float (xivvy_flight.gd), on foot while a grab holds her
                let grabbed = g.escape.as_ref().map(|e| e.hunter == slot).unwrap_or(false);
                let (lift, sway) = match &h.gimmick {
                    Gimmick::Xivvy(x) if !grabbed => (x.lift(), x.sway_offset(h.facing)),
                    _ => (0.0, (0.0, 0.0)),
                };
                let at = vec3(h.pos.0 + sway.0, lift + dy, h.pos.1 + sway.1);
                let (swing, amp_scale) = (h.stride * 2.4, amp);
                let pose = hunter_cache::pose_key(
                    tune,
                    &Mat4::from_translation(at),
                    &[Mat4::from_rotation_y(yaw), Mat4::from_rotation_x(swing), Mat4::from_scale(Vec3::splat(amp_scale))],
                    0,
                );
                HUNTER_MESHES.with(|m| {
                    let mut m = m.borrow_mut();
                    let mesh = m.cache.get_or_build(
                        tune,
                        (lvl, UNRIGGED_SLOT_BASE + cslot),
                        id,
                        pose,
                        hunter_cache::tint_key(tune, tint),
                        Some(model.texture.clone()),
                        |v, i| model.build(v, i, at, yaw, swing, amp_scale, tint),
                    );
                    draw_mesh(mesh);
                });
                continue;
            };
            let entry = views
                .by_slot
                .entry(slot)
                .or_insert_with(|| (id.to_string(), HunterView::new(id, rig.clone())));
            if entry.0 != id {
                *entry = (id.to_string(), HunterView::new(id, rig.clone()));
            }
            let view = &mut entry.1;
            let held = g.escape.as_ref().map(|e| e.hunter == slot).unwrap_or(false);
            let look_target = if !h.active || h.stun > 0.0 {
                None
            } else {
                match h.state {
                    State::Chase | State::Investigate => h.last_known,
                    State::Search => h.search_target.or(h.last_known),
                    _ => None,
                }
            };
            let input = Input {
                pos: h.pos,
                brain_facing: h.facing,
                active: h.active,
                stun: h.stun,
                heavy_stun: h.stun_heavy,
                state: h.state,
                direct_visible: h.direct_visible,
                winded: h.winded,
                airborne: matches!(&h.gimmick, Gimmick::Xivvy(x) if x.airborne()) && !held,
                lift: match &h.gimmick {
                    Gimmick::Xivvy(x) if !held => x.lift(),
                    _ => 0.0,
                },
                sway: match &h.gimmick {
                    Gimmick::Xivvy(x) if !held => x.sway_offset(h.facing),
                    _ => (0.0, 0.0),
                },
                charging: matches!(&h.gimmick, Gimmick::Loxelle(l) if l.holds_position()),
                look_target,
                path_ahead: Some(&|d| h.path_look_ahead(d)),
                chase_style: &h.def.chase_style,
                camera: g.eye(),
                grabbing: held,
                peek_lean: h.peeking_out().then_some(h.peek_lean),
            };
            let (pose, normal) = view.update(&input, dt, g.time);
            let out = grab::drive(
                slot,
                held,
                view,
                pose,
                normal,
                g.time,
                dt,
                g.run_seed,
                runner_feet,
                g.p.eye_height(),
                (g.eye(), cam_fwd),
                &g.fac,
                WALL_HEIGHT,
            );
            let bone_of = |k: usize| rig.bone(&model.groups[k].name).map(|b| out.bones[b]);
            let bones: Vec<Mat4> = (0..model.groups.len()).filter_map(|k| bone_of(k)).collect();
            let root = if dy == 0.0 { out.root } else { Mat4::from_translation(vec3(0.0, dy, 0.0)) * out.root };
            let pose = hunter_cache::pose_key(tune, &root, &bones, out.face_cell as i32);
            HUNTER_MESHES.with(|m| {
                let mut m = m.borrow_mut();
                let mesh = m.cache.get_or_build(
                    tune,
                    (lvl, cslot),
                    id,
                    pose,
                    hunter_cache::tint_key(tune, tint),
                    Some(model.texture.clone()),
                    |v, i| model.build_posed(v, i, root, &bone_of, model.face_cell_uv(out.face_cell), tint),
                );
                draw_mesh(mesh);
            });
            let star = if view.heavy(&input) {
                Color::new(1.0, 0.42, 0.18, 1.0)
            } else {
                Color::new(1.0, 0.86, 0.25, 1.0)
            };
            for (p, rot) in view.stars(&input, &out, g.time) {
                stars.cube_basis(p, Vec3::splat(0.09), rot, star, 2.0);
            }
        }
        stars.draw_and_clear();
    });
    // NPC teammates (game/npc.rs) and bots: the runner mannequin in its skin, bent like a runner (net/runner_figure.rs),
    // in this camera and material scope like the remote runners. A bot on another floor shows through an opening only.
    let bot_views: Vec<runner_figure::BotView> = crate::render::stack_actors::draw_list(g)
        .iter()
        .filter(|r| r.kind == crate::render::stack_actors::ActorKind::Bot)
        .filter(|r| cull.visible(g, r.level, r.y, r.pos, cull_eye))
        .map(|r| {
            let n = &g.npcs[r.index];
            runner_figure::BotView {
                name: &n.name,
                at: n.at,
                facing: n.facing,
                stride: n.stride,
                walking: n.walking,
                y: crate::render::stack_actors::actor_dy(r.y, g.fac.level),
            }
        })
        .collect();
    runner_figure::draw_bots(a, &bot_views);
    for h in g.hunters.iter().filter(|h| h.active) {
        let faded = h.gimmick.hidden_from_view();
        if faded && !matches!(h.gimmick, Gimmick::Ilyra(_)) {
            continue;
        }
        let yaw = (-h.facing.0).atan2(-h.facing.1);
        let eye_dy = crate::render::stack_actors::actor_dy(h.y, g.fac.level);
        if let Some(eye) = h.gimmick.eye_color() {
            let mut e = Builder::new(Some(a.white.clone()));
            let fwd = vec3(h.facing.0, 0.0, h.facing.1);
            let right = vec3(-h.facing.1, 0.0, h.facing.0);
            for s in [-1.0, 1.0] {
                e.cube(
                    vec3(h.pos.0, HUNTER_HEIGHT * 0.86 + eye_dy, h.pos.1) + fwd * 0.33 + right * 0.1 * s,
                    vec3(0.06, 0.04, 0.02),
                    yaw,
                    Color::new(eye.0, eye.1, eye.2, 1.0),
                    2.0,
                );
            }
            e.draw_and_clear();
        }
    }
    drop(t_hunters);
    draw_level_fx(g, a, &fx, light.eye);
    gl_use_default_material();
}

/// Whether a storey's catwalk lamps, falls and inferno glow are in this frame, found once per frame.
struct LevelFx {
    /// The point lights this storey adds to the room's lamps.
    lamps: Vec<Lamp>,
    /// The falls within reach of the eye, each with its drawn sheet, mist and foam.
    falls: Vec<(waterfall::Site, waterfall::Waterfall)>,
    /// All of the window's falls (the loop picks the nearest in range, seen or not).
    sites: Vec<waterfall::Site>,
}

/// The lamps of the catwalk cells within `radius` metres of `eye` (x, z): every cell of a deck that carries one
/// (`catwalk::lamp`, by absolute cell), placed above the deck at the cell's middle.
pub fn catwalk_lamps(fac: &Facility, eye: (f32, f32), radius: f32, cfg: &catwalk::CatwalkCfg) -> Vec<Lamp> {
    let r = radius.ceil() as i32;
    let (cx, cy) = (eye.0.floor() as i32, eye.1.floor() as i32);
    let mut out = Vec::new();
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let surface = fac.walk_surface_at(Cell::new(x, y));
            if surface.kind != DeckKind::Catwalk {
                continue;
            }
            if let Some(l) = catwalk::lamp(cfg, (x + fac.origin.0, y + fac.origin.1)) {
                out.push(Lamp {
                    pos: vec3(x as f32 + 0.5 + l.pos[0], surface.height + l.pos[1], y as f32 + 0.5 + l.pos[2]),
                    range: l.range_m,
                    color: Color::new(l.color[0], l.color[1], l.color[2], 1.0),
                    energy: l.energy,
                });
            }
        }
    }
    out
}

/// The point light of a fall's splash pool (waterfall.rs `Waterfall::light_pos`, placed at the fall's lip).
pub fn splash_lamp(cfg: &waterfall::WaterfallCfg, site: &waterfall::Site, w: &waterfall::Waterfall) -> Lamp {
    let c = cfg.splash.light_color;
    Lamp {
        pos: vec3(site.lip[0] + w.light_pos[0], w.light_pos[1], site.lip[1] + w.light_pos[2]),
        range: cfg.splash.light_range_m,
        color: Color::new(c[0], c[1], c[2], 1.0),
        energy: cfg.splash.light_energy,
    }
}

/// The lights and falls of the runner's storey this frame: catwalk lamps on any storey, the falls and their pool lights
/// on the two sewer storeys, the lava and cracks of the inferno (level -3) with their flicker.
fn level_fx(g: &Game, eye: Vec3) -> LevelFx {
    let mut fx = LevelFx { lamps: Vec::new(), falls: Vec::new(), sites: Vec::new() };
    fx.lamps.extend(catwalk_lamps(&g.fac, (eye.x, eye.z), scene_lights().catwalk_scan_radius_m, catwalk::config()));
    let Some(inf) = g.inf.as_ref() else { return fx };
    let cfg = waterfall::config();
    fx.sites = waterfall::sites_cached(&inf.seed, inf.level, inf.center);
    for s in &fx.sites {
        let d = ((s.lip[0] - eye.x).powi(2) + (s.lip[1] - eye.z).powi(2)).sqrt();
        if d > cfg.view.cull_m {
            continue;
        }
        let w = waterfall::build(cfg, s.top_y, s.bottom_y, s.out, g.time);
        fx.lamps.push(splash_lamp(cfg, s, &w));
        fx.falls.push((s.clone(), w));
    }
    if inf.level == INFERNO_LEVEL {
        let (deco, scene) = (inferno_deco::inferno_deco(), inferno_deco::scene_cfg());
        inferno_deco::with_window_decos(&inf.seed, inf.center, |placed| {
            for gl in inferno_deco::glows(placed, [eye.x, eye.y, eye.z], g.time, deco, scene) {
                fx.lamps.push(Lamp {
                    pos: Vec3::from_array(gl.pos),
                    range: gl.range_m,
                    color: Color::new(gl.color[0], gl.color[1], gl.color[2], 1.0),
                    energy: gl.energy,
                });
            }
        });
    }
    fx
}

/// Draws the storey's effects in the world material: the inferno's embers (small unlit cubes), then, alpha-blended and
/// unlit, each fall's sheet (with flowing streaks), its mist puffs (billboards) and its foam on the pool.
fn draw_level_fx(g: &Game, a: &Assets, fx: &LevelFx, eye: Vec3) {
    if let Some(inf) = g.inf.as_ref().filter(|i| i.level == INFERNO_LEVEL) {
        let (deco, scene) = (inferno_deco::inferno_deco(), inferno_deco::scene_cfg());
        let mut b = Builder::new(Some(a.white.clone()));
        inferno_deco::with_window_decos(&inf.seed, inf.center, |placed| {
            for e in inferno_deco::embers(placed, [eye.x, eye.y, eye.z], g.time, deco, scene) {
                let c = e.color;
                b.cube(Vec3::from_array(e.pos), Vec3::splat(e.size_m), 0.0, Color::new(c[0], c[1], c[2], 1.0), 2.0);
            }
        });
        b.draw_and_clear();
    }
    if fx.falls.is_empty() {
        return;
    }
    let cfg = waterfall::config();
    let mode = scene_lights().translucent_mode;
    let (_, _, right, up) = view(g);
    let mut b = Builder::new(Some(a.white.clone()));
    for (site, w) in &fx.falls {
        let at = |p: [f32; 3]| vec3(site.lip[0] + p[0], p[1], site.lip[1] + p[2]);
        let s = cfg.sheet_color;
        for q in &w.sheet {
            let shade = waterfall::streak(cfg, (q.v_top + q.v_bottom) * 0.5, g.time);
            b.quad(
                [at(q.corners[0]), at(q.corners[1]), at(q.corners[2]), at(q.corners[3])],
                [vec2(0.0, q.v_top), vec2(1.0, q.v_top), vec2(1.0, q.v_bottom), vec2(0.0, q.v_bottom)],
                Vec3::Y,
                Color::new(s[0] * shade, s[1] * shade, s[2] * shade, s[3]),
                mode,
            );
        }
        let f = cfg.view.foam_color;
        let (fx_, fz) = (cfg.splash.foam_size_m[0] * 0.5, cfg.splash.foam_size_m[1] * 0.5);
        let c = w.foam_center;
        b.quad(
            [at([c[0] - fx_, c[1], c[2] - fz]), at([c[0] + fx_, c[1], c[2] - fz]), at([c[0] + fx_, c[1], c[2] + fz]), at([c[0] - fx_, c[1], c[2] + fz])],
            [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)],
            Vec3::Y,
            Color::new(f[0], f[1], f[2], f[3]),
            mode,
        );
        let m = cfg.mist.color;
        for p in &w.mist {
            let (c, h) = (at(p.pos), p.size_m * 0.5);
            b.quad(
                [c - right * h + up * h, c + right * h + up * h, c + right * h - up * h, c - right * h - up * h],
                [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)],
                Vec3::Y,
                Color::new(m[0], m[1], m[2], p.alpha),
                mode,
            );
        }
    }
    b.draw_and_clear();
}

/// The scene's own tunables, under data/ (compiled in by build.rs).
const SCENE_LIGHTS_PATH: &str = "render/scene_lights.json";

/// The scene light tunables from data/render/scene_lights.json.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneLights {
    /// How far from the eye (metres, along the floor) the catwalk cells are searched for lamps.
    pub catwalk_scan_radius_m: f32,
    /// The shader mode of an unlit, alpha-blended surface (falls, mist, foam).
    pub translucent_mode: f32,
}

impl SceneLights {
    /// Parses `{ "catwalk_scan_radius_m": <metres>, "translucent_mode": <shader mode> }`; the radius is zero or more
    /// and the mode is above 1.5 (the unlit modes).
    pub fn parse(text: &str) -> Result<SceneLights, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_f64())
                .filter(|v| *v >= 0.0)
                .map(|v| v as f32)
                .ok_or_else(|| format!("{key}: expected a number, zero or more"))
        };
        let out = SceneLights { catwalk_scan_radius_m: num("catwalk_scan_radius_m")?, translucent_mode: num("translucent_mode")? };
        if out.translucent_mode <= 1.5 {
            return Err("translucent_mode: expected an unlit mode above 1.5".into());
        }
        Ok(out)
    }
}

/// The scene light tunables compiled into the executable. A table that fails to parse is reported on stderr and the
/// catwalks are searched out to the fog's reach.
pub fn scene_lights() -> &'static SceneLights {
    static LIGHTS: OnceLock<SceneLights> = OnceLock::new();
    LIGHTS.get_or_init(|| {
        let text = crate::content::embedded::data_under(SCENE_LIGHTS_PATH).next().map(|(_, text)| text).unwrap_or("{}");
        SceneLights::parse(text).unwrap_or_else(|e| {
            eprintln!("{SCENE_LIGHTS_PATH}: {e}");
            SceneLights { catwalk_scan_radius_m: FOG_RANGE, translucent_mode: 2.0 }
        })
    })
}

/// The room doors, each swung to its open amount (the world meshes hold them shut): the hinge leaves of every door near
/// enough to be seen, batched by texture.
fn draw_doors(g: &Game, a: &Assets) {
    let eye = g.eye();
    let mut parts = HashMap::new();
    for d in &g.world.doors {
        let reach = FOG_RANGE + 6.0;
        if (d.xf.w_axis.truncate() - eye).length_squared() > reach * reach {
            continue;
        }
        if let Some(m) = a.models.get(&d.model) {
            m.add_hinges(&mut parts, d.xf, WHITE, g.fac.door_open_amount(d.cell), &d.fit);
        }
    }
    draw_builders(parts);
}

pub fn battery_shape(b: &mut Builder, p: Vec3, spin: f32) {
    b.cube(
        p + vec3(0.0, 0.14, 0.0),
        vec3(0.14, 0.26, 0.14),
        spin,
        BATTERY,
        0.0,
    );
    b.cube(
        p + vec3(0.0, 0.29, 0.0),
        vec3(0.06, 0.04, 0.06),
        spin,
        Color::new(0.8, 0.8, 0.75, 1.0),
        0.0,
    );
}

fn pickup_shape(b: &mut Builder, item: Item, p: Vec3, spin: f32) {
    match item {
        Item::Battery => battery_shape(b, p, spin),
        _ => b.cube(
            p + vec3(0.0, 0.05, 0.0),
            vec3(0.34, 0.08, 0.1),
            spin,
            Color::new(0.3, 0.3, 0.32, 1.0),
            0.0,
        ),
    }
}

/// The locker cull's data file, under data/ (compiled in by build.rs).
const LOCKER_CULL_PATH: &str = "render/locker_cull.json";

/// The locker fog-cull tunables from data/render/locker_cull.json.
pub struct LockerCull {
    /// Metres past the fog's full reach before a locker is skipped, so rounding never drops a visible one.
    pub margin_m: f32,
    /// The door's full open angle, radians (hide.rs DOOR_OPEN_ANGLE): the swing the locker's reach must cover. The reach
    /// covers every angle, so only the tests read it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub door_swing_rad: f32,
}

impl LockerCull {
    /// Parses `{ "margin_m": <metres>, "door_swing_rad": <radians> }`. Both must be numbers, zero or more.
    pub fn parse(text: &str) -> Result<LockerCull, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_f64())
                .filter(|v| *v >= 0.0)
                .map(|v| v as f32)
                .ok_or_else(|| format!("{key}: expected a number, zero or more"))
        };
        Ok(LockerCull {
            margin_m: num("margin_m")?,
            door_swing_rad: num("door_swing_rad")?,
        })
    }
}

/// The locker cull compiled into the executable. A table that fails to parse is reported on stderr and the
/// Standard values (the margin of the Rust cull before the data existed, and hide.rs DOOR_OPEN_ANGLE) are used.
pub fn locker_cull() -> &'static LockerCull {
    static CULL: OnceLock<LockerCull> = OnceLock::new();
    CULL.get_or_init(|| {
        let text = crate::content::embedded::data_under(LOCKER_CULL_PATH)
            .next()
            .map(|(_, text)| text)
            .unwrap_or("{}");
        LockerCull::parse(text).unwrap_or_else(|e| {
            eprintln!("{LOCKER_CULL_PATH}: {e}");
            LockerCull {
                margin_m: 1.0,
                door_swing_rad: 1.75,
            }
        })
    })
}

/// Metres past the fog's full reach before a world mesh is skipped, so rounding never drops a visible one.
const WORLD_CULL_MARGIN: f32 = 1.0;

/// Distance from the eye past which a world mesh is skipped: the unlit mode's full fog (FOG_RANGE * 1.6) plus the margin.
/// Lit surfaces reach full fog sooner (FOG_RANGE), so every fragment this far is fog colour in both modes.
const WORLD_CULL_RANGE: f32 = FOG_RANGE * 1.6 + WORLD_CULL_MARGIN;

/// True when the box `lo`..`hi` lies wholly `reach` metres or more from `eye`: every point of it is then fully fogged,
/// and so is every triangle inside it (the fog is a function of distance from the eye, and a box is convex).
fn box_fully_fogged(eye: Vec3, lo: Vec3, hi: Vec3, reach: f32) -> bool {
    let gap = (lo - eye).max(eye - hi).max(Vec3::ZERO);
    gap.length_squared() >= reach * reach
}

/// The farthest any of `points` lies from the origin (zero for none).
fn farthest(points: impl Iterator<Item = Vec3>) -> f32 {
    points.map(|p| p.length()).fold(0.0, f32::max)
}

/// The farthest any point of a locker model lies from its origin, its door included at every swing. The door turns
/// about its pivot, and its parts are relative to that pivot (prop_model.rs `add_locker_door`), so a door point sits
/// within `|pivot| + |part|` of the origin whatever the angle: the reach covers the full swing without reading it.
fn locker_reach(m: &PropModel) -> f32 {
    let body = farthest(m.parts.iter().flat_map(|p| p.verts.iter()).map(|v| v.position));
    let door = m.locker_door.as_ref().map_or(0.0, |d| {
        d.pivot.length() + farthest(d.parts.iter().flat_map(|p| p.verts.iter()).map(|v| v.position))
    });
    body.max(door)
}

/// True when every point of a locker, a sphere of `reach` metres about its origin at (`cx`, `cy`), is past the fog's
/// full reach from the eye by `margin` metres: the shader then writes the fog colour alone, the same as the clear
/// colour, so the locker draws nothing new. The unlit path (mode > 1.5) reaches full fog at `FOG_RANGE * 1.6` (shader.rs).
fn locker_fully_fogged(eye: Vec3, cx: f32, cy: f32, reach: f32, margin: f32) -> bool {
    let (dx, dz) = (cx - eye.x, cy - eye.z);
    (dx * dx + dz * dz).sqrt() - reach >= FOG_RANGE * 1.6 + margin
}

fn locker_shape(b: &mut Builder, p: Vec3, yaw: f32, burnt: bool, swing: f32) {
    let body = if burnt {
        Color::new(0.16, 0.13, 0.12, 1.0)
    } else {
        LOCKER
    };
    b.cube(
        p + vec3(0.0, 1.0, 0.0),
        vec3(0.78, 2.0, 0.56),
        yaw,
        body,
        0.0,
    );
    let front = vec3(0.0, 1.0, 0.3);
    let hinge = vec3(-0.35, 1.0, 0.3);
    let panel = hinge + Mat3::from_rotation_y(-swing) * (front - hinge);
    b.cube(
        p + Mat3::from_rotation_y(yaw) * panel,
        vec3(0.7, 1.9, 0.04),
        yaw - swing,
        body,
        0.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_light_on_during_grab_off_after() {
        // the runner's eye at 1.6 m looking down -Z at her face 0.8 m away, as the grab camera sets it
        let cam = (vec3(0.0, 1.6, 0.0), vec3(0.0, 1.6, -0.8), Vec3::Y);
        // during the struggle: a fill lamp behind and above the eye, close to the pair
        let on = grab_fill(true, Some(cam)).expect("the grab fill is on while she holds the runner");
        assert!(on.energy > 0.0 && on.range > 0.0);
        assert!(on.pos.z > cam.0.z, "the fill is behind the eye, so it lights her face");
        assert!(on.pos.y > cam.0.y, "the fill is above the eye");
        assert!((on.pos - cam.0).length() < 1.0, "the fill stays with the pair");
        // after the release the escape is cleared and the grab camera is gone: the light is off
        assert!(grab_fill(false, None).is_none());
        // a stale camera with the escape cleared is still off
        assert!(grab_fill(false, Some(cam)).is_none());
        // the escape set before the grab camera exists (the first frame of a catch): off until the grab starts
        assert!(grab_fill(true, None).is_none());
    }

    fn deck_floor() -> Facility {
        use crate::world::catalog::Catalog;
        use crate::world::facility::Options;
        Facility::generate("SCENE-DECK", &Options::default(), &Catalog::load())
    }

    /// A cell whose absolute coordinates both divide by `every`, so a lamp of that spacing stands on it.
    fn lamp_cell(f: &Facility, every: i32) -> Cell {
        let pick = |origin: i32| (0..every).find(|v| (v + origin).rem_euclid(every) == 0).unwrap() + every * 3;
        Cell::new(pick(f.origin.0), pick(f.origin.1))
    }

    #[test]
    fn a_catwalk_lamp_hangs_over_its_deck_and_a_floor_cell_has_none() {
        let cfg = catwalk::config();
        let mut f = deck_floor();
        let c = lamp_cell(&f, cfg.light_every_cells);
        let eye = (c.x as f32 + 0.5, c.y as f32 + 0.5);
        assert!(catwalk_lamps(&f, eye, 6.0, cfg).is_empty(), "no deck, no lamp");
        f.set_walk_surface(c, DeckKind::Catwalk, 4.0);
        let lamps = catwalk_lamps(&f, eye, 6.0, cfg);
        assert_eq!(lamps.len(), 1);
        let l = lamps[0];
        assert_eq!((l.pos.x, l.pos.z), (eye.0, eye.1), "over the middle of the cell");
        assert!((l.pos.y - (4.0 + cfg.light_height_m)).abs() < 1e-5, "above the deck, not the floor");
        assert_eq!((l.range, l.energy), (cfg.light_range_m, cfg.light_energy));
        // a deck cell the lamp spacing skips carries none, and a deck out of the search reach is not looked at
        let skipped = Cell::new(c.x + 1, c.y);
        f.set_walk_surface(skipped, DeckKind::Catwalk, 4.0);
        assert_eq!(catwalk_lamps(&f, eye, 6.0, cfg).len(), 1);
        assert!(catwalk_lamps(&f, (eye.0 + 100.0, eye.1), 6.0, cfg).is_empty());
    }

    #[test]
    fn the_catwalk_lamp_spacing_is_data() {
        let text = include_str!("../data/render/catwalk.json");
        let dense = catwalk::CatwalkCfg::parse(&text.replacen("\"every_cells\": 3", "\"every_cells\": 1", 1)).unwrap();
        let mut f = deck_floor();
        let c = Cell::new(10, 10);
        for dx in 0..3 {
            f.set_walk_surface(Cell::new(c.x + dx, c.y), DeckKind::Catwalk, 4.0);
        }
        let eye = (11.5, 10.5);
        assert!(catwalk_lamps(&f, eye, 6.0, &dense).len() == 3, "a lamp on every deck cell");
        assert!(catwalk_lamps(&f, eye, 6.0, catwalk::config()).len() < 3);
    }

    #[test]
    fn the_splash_lamp_sits_at_the_lip_with_the_data_colour() {
        let cfg = waterfall::config();
        let site = waterfall::Site { lip: [40.0, 30.0], out: -1.0, top_abs: -5.3, bottom_abs: -10.3, top_y: -0.3, bottom_y: -5.3 };
        let w = waterfall::build(cfg, site.top_y, site.bottom_y, site.out, 0.0);
        let l = splash_lamp(cfg, &site, &w);
        assert!((l.pos.x - 40.0).abs() < 1e-4 && l.pos.z < 30.0, "out to the north of the lip");
        assert!(l.pos.y > site.bottom_y && l.pos.y < site.top_y, "over the pool, under the lip");
        assert_eq!((l.range, l.energy), (cfg.splash.light_range_m, cfg.splash.light_energy));
        assert_eq!([l.color.r, l.color.g, l.color.b], cfg.splash.light_color);
    }

    #[test]
    fn the_hunter_cache_clears_when_the_hunters_the_list_or_the_run_change() {
        let tune = hunter_cache::tuning();
        let mut m = HunterMeshes::default();
        assert!(!m.cache.sync(&[], 0), "nothing differs from a fresh state");
        assert!(!m.cache.sync(&[(0, 3)], 7), "a new run with nothing cached drops nothing");
        let root = Mat4::IDENTITY;
        let mut builds = 0;
        for _ in 0..3 {
            m.cache.get_or_build(tune, (0, 2), "dovelle", hunter_cache::pose_key(tune, &root, &[], 0), [100; 4], None, |_, _| builds += 1);
        }
        assert_eq!(builds, 1, "the same pose is built once");
        assert!(!m.cache.sync(&[(0, 3)], 7));
        assert_eq!(m.cache.len(), 1, "an unchanged frame keeps the cache");
        assert!(m.cache.sync(&[(0, 2)], 7), "a removed hunter drops its entry");
        assert!(m.cache.is_empty());
        m.cache.get_or_build(tune, (0, 1), "dovelle", vec![0], [100; 4], None, |_, _| {});
        assert!(!m.cache.sync(&[(0, 2), (-1, 4)], 7), "a second list keeps the first list's entries");
        assert_eq!(m.cache.len(), 1);
        assert!(m.cache.sync(&[(-1, 4)], 7), "a list that is gone drops its entries");
        assert!(m.cache.is_empty());
        m.cache.get_or_build(tune, (-1, 0), "dovelle", vec![0], [100; 4], None, |_, _| {});
        assert!(m.cache.sync(&[(-1, 4)], 8), "a new run drops them all");
        assert!(m.cache.is_empty());
    }

    #[test]
    fn scene_light_tunables_are_data() {
        let text = include_str!("../data/render/scene_lights.json");
        let s = SceneLights::parse(text).unwrap();
        assert_eq!(&s, scene_lights());
        assert!(s.translucent_mode > 1.5 && s.catwalk_scan_radius_m > 0.0);
        assert!(SceneLights::parse("{}").is_err());
        assert!(SceneLights::parse(&text.replacen("3.0", "1.0", 1)).is_err(), "a lit mode cannot be translucent");
        let wide = SceneLights::parse(&text.replacen("24.0", "48.0", 1)).unwrap();
        assert!(wide.catwalk_scan_radius_m > s.catwalk_scan_radius_m);
    }

    #[test]
    fn far_world_meshes_are_skipped_only_when_fully_fogged() {
        let eye = vec3(0.0, 1.6, 0.0);
        // a box 50 m off along x: its nearest face is past the reach, so it is skipped
        assert!(box_fully_fogged(eye, vec3(50.0, 0.0, -2.0), vec3(60.0, 3.0, 2.0), WORLD_CULL_RANGE));
        // a box straddling the eye is drawn
        assert!(!box_fully_fogged(eye, vec3(-1.0, 0.0, -1.0), vec3(1.0, 3.0, 1.0), WORLD_CULL_RANGE));
        // a box 30 m off is still in the fog ramp of the unlit mode, so drawn
        assert!(!box_fully_fogged(eye, vec3(30.0, 0.0, 0.0), vec3(31.0, 3.0, 1.0), WORLD_CULL_RANGE));
        // the vertical gap counts: a box 45 m straight up is past the reach
        assert!(box_fully_fogged(eye, vec3(0.0, 46.0, 0.0), vec3(1.0, 47.0, 1.0), WORLD_CULL_RANGE));
    }

    #[test]
    fn far_lockers_are_skipped_only_when_fully_fogged() {
        let eye = vec3(0.0, 1.6, 0.0);
        let reach = 1.2;
        let margin = locker_cull().margin_m;
        // 45 m off: the nearest point of the locker is past the full fog reach, so it is skipped
        assert!(locker_fully_fogged(eye, 45.0, 0.0, reach, margin));
        // 30 m off: still inside the fog ramp, so it is drawn
        assert!(!locker_fully_fogged(eye, 30.0, 0.0, reach, margin));
        // the vertical offset never shortens the distance, so a locker straight below the eye is judged by its plan distance only
        assert!(!locker_fully_fogged(eye, 0.0, 0.0, reach, margin));
    }

    #[test]
    fn locker_cull_standard_values_match_the_game() {
        // Standard: the 1.0 m margin the cull used before the data existed (scene.rs LOCKER_CULL_MARGIN), and the
        // full door swing hide.rs DOOR_OPEN_ANGLE drives (about 100 degrees).
        let cull = locker_cull();
        assert!((cull.margin_m - 1.0).abs() < 1e-6);
        assert!((cull.door_swing_rad - hide::DOOR_OPEN_ANGLE).abs() < 1e-6);
    }

    #[test]
    fn changing_the_margin_data_changes_the_locker_cull() {
        let eye = vec3(0.0, 1.6, 0.0);
        let tight = LockerCull::parse(r#"{"margin_m": 0.0, "door_swing_rad": 1.75}"#).unwrap();
        let wide = LockerCull::parse(r#"{"margin_m": 5.0, "door_swing_rad": 1.75}"#).unwrap();
        // 41 m off with a 1.2 m reach: 39.8 m clear of the eye, past the 38.4 m fog reach by 1.4 m
        assert!(locker_fully_fogged(eye, 41.0, 0.0, 1.2, tight.margin_m));
        assert!(!locker_fully_fogged(eye, 41.0, 0.0, 1.2, wide.margin_m));
    }

    #[test]
    fn locker_cull_rejects_bad_tables() {
        assert!(LockerCull::parse(r#"{"margin_m": -1.0, "door_swing_rad": 1.75}"#).is_err());
        assert!(LockerCull::parse(r#"{"margin_m": 1.0}"#).is_err());
        assert!(LockerCull::parse(r#"{"margin_m": "wide", "door_swing_rad": 1.75}"#).is_err());
        assert!(LockerCull::parse("[1.0]").is_err());
    }

    #[test]
    fn a_swung_door_stays_inside_the_locker_reach() {
        // a door leaf about a hinge line 0.35 m from the origin; its points are pivot-relative, as add_locker_door places them
        let pivot = vec3(-0.35, 1.0, 0.3);
        let leaf = [vec3(0.35, 0.95, 0.0), vec3(0.35, -0.95, 0.02), vec3(0.0, 0.0, 0.0)];
        let reach = pivot.length() + farthest(leaf.iter().copied());
        let swing = locker_cull().door_swing_rad;
        for angle in [0.0, swing * 0.5, swing] {
            let turn = Mat3::from_rotation_y(-angle);
            for p in leaf {
                assert!((pivot + turn * p).length() <= reach + 1e-5, "a door point at {angle} rad leaves the reach");
            }
        }
    }
}
