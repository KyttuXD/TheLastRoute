//! The per-frame 3D scene: the floor, its fixtures and pickups drawn with the original's
//! authored Blender models (assets/interactables/*.glb: relay, battery, coin, locker, lift,
//! safe zone, vent, music box, heater station, light switch, and the tools), Ravelle's wires,
//! and the hunters in their Blockbench bodies. Simple shapes stand in if a model is missing.

use super::anim::{
    self, grab,
    hunter::{HunterView, Input},
};
use super::builder::Builder;
use super::glb::GlbModel;
use super::shader::{self, Lamp, Lighting};
use crate::shared::assets::Assets;
use crate::config::*;
use crate::game::{Game, Heater, Item};
use crate::hunters::brain::State;
use crate::hunters::gimmicks::Gimmick;
use macroquad::prelude::*;

pub const FOG: Vec3 = Vec3::new(0.012, 0.008, 0.014);
pub const RELAY: Color = Color::new(0.26, 0.88, 0.92, 1.0);
pub const BATTERY: Color = Color::new(0.95, 0.82, 0.25, 1.0);
const SAFE: Color = Color::new(0.35, 0.95, 0.52, 1.0);
const LIFT: Color = Color::new(0.76, 0.55, 0.96, 1.0);
const LOCKER: Color = Color::new(0.45, 0.60, 0.70, 1.0);
const TOOL: Color = Color::new(0.64, 0.84, 0.96, 1.0);

pub fn camera(g: &Game, rt: &RenderTarget) -> Camera3D {
    let eye = g.eye();
    let (fwd, _, _) = g.camera_basis();
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
    let eye = g.eye();
    let (fwd, right, up) = g.camera_basis();
    Lighting {
        eye,
        flashlight_pos: eye + right * 0.08 - up * 0.05 + fwd * 0.12,
        flashlight_dir: fwd,
        flashlight_on: g.p.flashlight_on,
        // rooms are dark unless something lights them: Godot's RefLook ambient (shader::AMBIENT_COLOR x AMBIENT_ENERGY)
        ambient: shader::AMBIENT_COLOR * shader::AMBIENT_ENERGY,
        fog: FOG,
        fog_far: 24.0,
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

/// An interactable's model: assets/interactables/<name>.glb.
pub fn item_model<'a>(a: &'a Assets, name: &str) -> Option<&'a GlbModel> {
    a.glb.get(&format!("interactables/{name}.glb"))
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
    set_camera(&camera(g, rt));
    clear_background(Color::new(FOG.x, FOG.y, FOG.z, 1.0));
    gl_use_material(mat);
    // the grab's fill joins the room's lamps while the struggle holds the runner (grab_fill); the flashlight is unchanged
    let light = lighting(g);
    match grab_fill(g.escape.is_some(), grab::camera()) {
        Some(fill) => {
            let mut lamps = g.world.lamps.clone();
            lamps.push(fill);
            shader::apply(mat, &light, &lamps);
        }
        None => shader::apply(mat, &light, &g.world.lamps),
    }
    for m in &g.world.meshes {
        draw_mesh(m);
    }
    let mut b = Builder::new(Some(a.white.clone()));
    let t = g.time;
    let f = &g.fac;

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
            Some(m) => m.draw(at(sx, 0.0, sy, 0.0), WHITE),
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
            Some(m) => m.draw(at(x, 0.0, y, 0.0), WHITE),
            None => b.cube(
                vec3(x, 0.32, y),
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
                vec3(x, 0.01, y),
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
                Some(m) => m.draw(at(x, 0.0, y, i as f32), WHITE),
                None => battery_shape(&mut b, vec3(x, 0.0, y), i as f32),
            }
        }
    }
    for (i, p) in g.pickups.iter().enumerate() {
        if p.taken {
            continue;
        }
        let base = vec3(p.pos.0, 0.0, p.pos.1);
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
                m.draw(xf, WHITE);
            }
            None => pickup_shape(&mut b, p.item, base, i as f32 * 1.7),
        }
    }
    for (i, c) in g.fix.coins.iter().enumerate() {
        if !g.coins_taken[i] {
            let (x, y) = c.center();
            let bob = (t * 2.0 + i as f32).sin() * 0.03;
            match item_model(a, "coin") {
                Some(m) => m.draw(at(x, 0.05 + bob, y, t * 2.0 + i as f32), WHITE),
                None => b.cube(
                    vec3(x, 0.12 + bob, y),
                    vec3(0.16, 0.16, 0.03),
                    t * 2.0 + i as f32,
                    Color::new(0.95, 0.78, 0.2, 1.0),
                    2.0,
                ),
            }
        }
    }
    for (i, l) in f.hiding.iter().enumerate() {
        if g.p.hidden == Some(i) {
            continue;
        }
        let (cx, cy) = f.locker_center(l);
        let yaw = (l.facing.0 as f32).atan2(l.facing.1 as f32);
        let burnt = g.hiding_disabled[i];
        match item_model(a, "hiding_spot") {
            Some(m) => m.draw(
                at(cx, 0.0, cy, yaw),
                if burnt {
                    Color::new(0.25, 0.2, 0.18, 1.0)
                } else {
                    WHITE
                },
            ),
            None => locker_shape(&mut b, vec3(cx, 0.0, cy), yaw, burnt),
        }
    }
    for (i, l) in f.room_lights.iter().enumerate() {
        let (x, y) = l.switch_cell.center();
        let on = g.lights_on.get(i).copied().unwrap_or(true);
        // facility_view_3d.gd: the switch sits .9 up and .34 back from its cell centre
        match item_model(a, "wall_light_switch") {
            Some(m) => m.draw(
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
            Some(m) => m.draw(at(x, 0.0, y, 0.0), tint),
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
            Some(m) => m.draw(
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
                m.draw(at(x, 0.0, y, 0.0), WHITE);
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
        if let Gimmick::Ravelle(r) = &h.gimmick {
            for w in r.wires.iter().filter(|w| !w.triggered) {
                let a3 = vec3(w.a.0, 0.25, w.a.1);
                let b3 = vec3(w.b.0, 0.25, w.b.1);
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

    // hunters: posed by their rig (render/anim, the original's clips + view layers). Dormant hunters are drawn too, slumped
    // in their slowed stun clip (HunterView3D.model_alpha: an inactive hunter shows whole).
    let (cam_fwd, _, _) = g.camera_basis();
    let runner_feet = vec3(g.p.pos.0, g.p.feet_y.max(0.0), g.p.pos.1);
    anim::with_views(g.time, |views, dt| {
        let mut stars = Builder::new(Some(a.white.clone()));
        for (slot, h) in g.hunters.iter().enumerate() {
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
            let mut verts = vec![];
            let mut idx = vec![];
            let tint = if faded {
                Color::new(0.35, 0.35, 0.45, 1.0)
            } else if h.stun > 0.0 {
                Color::new(0.7, 0.8, 1.0, 1.0)
            } else {
                WHITE
            };
            let Some(rig) = anim::clip::rig(id) else {
                if !h.active {
                    continue;
                }
                let yaw = (-h.facing.0).atan2(-h.facing.1);
                let amp = (h.moving_speed / 3.0).min(1.0) * 0.6;
                let lift = if matches!(&h.gimmick, Gimmick::Xivvy(x) if x.flying) {
                    0.48
                } else {
                    0.0
                };
                model.build(
                    &mut verts,
                    &mut idx,
                    vec3(h.pos.0, lift, h.pos.1),
                    yaw,
                    h.stride * 2.4,
                    amp,
                    tint,
                );
                draw_mesh(&Mesh {
                    vertices: verts,
                    indices: idx,
                    texture: Some(model.texture.clone()),
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
                airborne: matches!(&h.gimmick, Gimmick::Xivvy(x) if x.flying) && !held,
                charging: matches!(&h.gimmick, Gimmick::Loxelle(l) if l.holds_position()),
                look_target,
                path_ahead: Some(&|d| h.path_look_ahead(d)),
                chase_style: &h.def.chase_style,
                camera: g.eye(),
                grabbing: held,
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
            model.build_posed(
                &mut verts,
                &mut idx,
                out.root,
                &bone_of,
                model.face_cell_uv(out.face_cell),
                tint,
            );
            draw_mesh(&Mesh {
                vertices: verts,
                indices: idx,
                texture: Some(model.texture.clone()),
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
    // NPC teammates (game/npc.rs): the hunter body at the runner's height, drawn without a rig (the same
    // fallback build a rig-less hunter gets). Only NPCs on the runner's level are drawn.
    if let Some(body) = a.npc_body.as_ref() {
        let level = g.runner_level();
        for n in g.npcs.iter().filter(|n| n.level == level) {
            let mut verts = vec![];
            let mut idx = vec![];
            let amp = if n.walking { 0.6 } else { 0.0 };
            body.build(
                &mut verts,
                &mut idx,
                vec3(n.at.0, 0.0, n.at.1),
                n.facing,
                n.stride * 2.4,
                amp,
                WHITE,
            );
            draw_mesh(&Mesh {
                vertices: verts,
                indices: idx,
                texture: Some(body.texture.clone()),
            });
        }
    }
    for h in g.hunters.iter().filter(|h| h.active) {
        let faded = h.gimmick.hidden_from_view();
        if faded && !matches!(h.gimmick, Gimmick::Ilyra(_)) {
            continue;
        }
        let yaw = (-h.facing.0).atan2(-h.facing.1);
        if let Some(eye) = h.gimmick.eye_color() {
            let mut e = Builder::new(Some(a.white.clone()));
            let fwd = vec3(h.facing.0, 0.0, h.facing.1);
            let right = vec3(-h.facing.1, 0.0, h.facing.0);
            for s in [-1.0, 1.0] {
                e.cube(
                    vec3(h.pos.0, HUNTER_HEIGHT * 0.86, h.pos.1) + fwd * 0.33 + right * 0.1 * s,
                    vec3(0.06, 0.04, 0.02),
                    yaw,
                    Color::new(eye.0, eye.1, eye.2, 1.0),
                    2.0,
                );
            }
            e.draw_and_clear();
        }
    }
    gl_use_default_material();
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

fn locker_shape(b: &mut Builder, p: Vec3, yaw: f32, burnt: bool) {
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
}
