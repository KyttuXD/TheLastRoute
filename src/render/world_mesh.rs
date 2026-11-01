//! The floor's static geometry: floors and walls in each room region's own pictures (halls in
//! the maze set), baseboard and brass rail trims, ceilings, archways where halls open into
//! rooms, and the room props: the original's authored Blender models where a type has them
//! (assets/props/<set>/*.glb), else the blocky rebuilds as Blockbench meshes with smoothed
//! normals. Lights: the switchable room lights and the lamp props' own lights.

use super::builder::Builder;
use super::shader::Lamp;
use super::surfaces;
use super::textures::{self, TexRef, TextureChoice};
use crate::app::assets::Assets;
use crate::config::{PROP_MODEL_SCALE, WALL_HEIGHT};
use crate::content::props::{stable_index, visual_y};
use crate::content::Content;
use crate::world::facility::{stair_well_cells, STAIR_FLIGHT};
use crate::world::{region_texture, Facility, TEXTURE_SETS};
use macroquad::prelude::*;
use std::collections::HashMap;

#[derive(Default)]
pub struct WorldMeshes {
    pub meshes: Vec<Mesh>,
    pub lamps: Vec<Lamp>,
}

const UV1: [Vec2; 4] = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)];

/// The room lights plus each lamp prop's `light` from data/props (the original's fixture lights,
/// budget 10 per floor, offset turning with the prop).
pub fn lamps(f: &Facility, lights_on: &[bool], content: &Content) -> Vec<Lamp> {
    let mut out = vec![];
    for (i, l) in f.room_lights.iter().enumerate() {
        if lights_on.get(i).copied().unwrap_or(true) {
            let (x, z) = l.center.center();
            out.push(Lamp {
                pos: vec3(x, WALL_HEIGHT - 0.34, z),
                range: l.range * 1.4,
                color: Color::new(l.color.0, l.color.1, l.color.2, 1.0),
                energy: l.energy,
            });
        }
    }
    let mut budget = 10;
    for p in &f.props {
        if budget == 0 {
            break;
        }
        let Some(light) = content.props.get(&p.ptype).and_then(|d| d.light.clone()) else {
            continue;
        };
        budget -= 1;
        let (x, z) = p.cell.center();
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        let off = Mat3::from_rotation_y(yaw) * vec3(light.offset.0, 0.0, light.offset.2);
        let y = visual_y(&p.ptype, WALL_HEIGHT, p.mount_y.unwrap_or(0.0) as f32) + light.offset.1;
        out.push(Lamp {
            pos: vec3(x + off.x, y, z + off.z),
            range: light.range,
            color: Color::new(light.color.0, light.color.1, light.color.2, 1.0),
            // Godot's omni energy reads brighter than this shader's linear falloff; doubled to match
            energy: light.energy * 2.0,
        });
    }
    out
}

fn room_set(f: &Facility, x: i32, y: i32) -> &'static str {
    match f.room_at(x as f64 + 0.5, y as f64 + 0.5) {
        Some(i) if !f.rooms[i].lobby() => region_texture(&f.rooms[i].region),
        // halls take their district's set on the Infinite route (the maze on a finite floor)
        _ => {
            let r = f.region_at(x, y);
            TEXTURE_SETS
                .iter()
                .copied()
                .find(|s| *s == r)
                .unwrap_or("maze")
        }
    }
}

/// Stairwell flights: each hole cell's step number (1 = next to the landing) and direction.
fn stair_steps(f: &Facility) -> HashMap<(i32, i32), (i32, i32)> {
    let mut out = HashMap::new();
    for s in &f.stairs {
        for k in 1..=STAIR_FLIGHT {
            for c in stair_well_cells(*s, k) {
                out.insert((c.x, c.y), (k, s.dir));
            }
        }
    }
    out
}

/// The original's surfaces for the space a cell is in (a room by its region and type, else the
/// hall of its district) and the floor kind of that cell: the room's one kind, the district's
/// hall kind per chunk with the carpet runner where the district lays one, the safe-room pad.
fn cell_surfaces<'c>(
    f: &Facility,
    x: i32,
    y: i32,
    cache: &'c mut HashMap<(String, String, bool), TextureChoice>,
) -> (&'c TextureChoice, TexRef) {
    let room = f
        .room_at(x as f64 + 0.5, y as f64 + 0.5)
        .filter(|i| !f.rooms[*i].lobby());
    let key = match room {
        Some(i) => (f.rooms[i].region.clone(), f.rooms[i].rtype.clone(), false),
        None => (f.region_at(x, y).to_string(), String::new(), true),
    };
    let choice = cache
        .entry(key.clone())
        .or_insert_with(|| textures::region_set(&key.0, &key.1, key.2));
    let (gx, gy) = ((x + f.origin.0) as i64, (y + f.origin.1) as i64);
    let mut floor = match room {
        _ if choice.per_cell => choice.floor_at(gx, gy),
        Some(i) => choice.floor_at(
            (f.rooms[i].x + f.origin.0) as i64,
            (f.rooms[i].y + f.origin.1) as i64,
        ),
        None => choice.floor_at(gx.div_euclid(45), gy.div_euclid(45)),
    };
    if choice.runner {
        let open = |a: i32, b: i32| {
            !f.is_wall_cell(a, b) && f.room_at(a as f64 + 0.5, b as f64 + 0.5).is_none()
        };
        if let Some(k) = textures::sets::runner_kind(open, x, y, 3) {
            floor = TexRef::new("floor", k);
        }
    }
    if let Some(sz) = f.safe_zone {
        if let Some(k) = textures::sets::safe_pad_kind(x - sz.x, y - sz.y) {
            floor = TexRef::new("floor", k);
        }
    }
    (choice, floor)
}

/// A builder per painted kind, falling back to the region swatch PNG for an unknown kind.
fn painted<'b>(
    map: &'b mut HashMap<(&'static str, &'static str), Builder>,
    r: TexRef,
    fallback: &Texture2D,
) -> &'b mut Builder {
    map.entry((r.group, r.kind)).or_insert_with(|| {
        Builder::new(Some(
            surfaces::texture(r).unwrap_or_else(|| fallback.clone()),
        ))
    })
}

pub fn build_world(
    f: &Facility,
    lights_on: &[bool],
    assets: &Assets,
    content: &Content,
) -> WorldMeshes {
    let mut surf: HashMap<(&'static str, &'static str), Builder> = HashMap::new();
    let mut choices: HashMap<(String, String, bool), TextureChoice> = HashMap::new();
    let mut plain = Builder::new(Some(assets.white.clone()));
    let h = WALL_HEIGHT;
    let ceiling = Color::from_rgba(46, 34, 44, 255);
    let up = vec3(0., 1., 0.);
    let steps = stair_steps(f);
    let step_col = Color::from_rgba(92, 78, 82, 255);
    let rise = h / (STAIR_FLIGHT + 1) as f32;
    // one painted storey stretched floor to ceiling, cornice at the top (RefSurfaceMesh.add_wall)
    let tiers = textures::wall_tiers(h) as f32;
    for c in f.floor_cells() {
        let (x, z) = (c.x as f32, c.y as f32);
        let set = room_set(f, c.x, c.y);
        let (choice, floor_kind) = cell_surfaces(f, c.x, c.y, &mut choices);
        let wall_kind = choice.wall_at((c.x + f.origin.0) as i64, (c.y + f.origin.1) as i64);
        let ceil_kind = choice.ceiling;
        let step = steps.get(&(c.x, c.y)).copied();
        match step {
            // a flight down: the floor opens onto steps sinking into the level below
            Some((k, d)) if d < 0 => {
                let top = -(k as f32) * rise;
                let bottom = -h - 0.2;
                plain.cube(
                    vec3(x + 0.5, (top + bottom) / 2.0, z + 0.5),
                    vec3(1.0, top - bottom, 1.0),
                    0.0,
                    step_col,
                    0.0,
                );
            }
            // a flight up: steps rising through an open shaft in the ceiling
            Some((k, _)) => {
                let top = k as f32 * rise;
                plain.cube(
                    vec3(x + 0.5, top / 2.0, z + 0.5),
                    vec3(1.0, top, 1.0),
                    0.0,
                    step_col,
                    0.0,
                );
            }
            None => painted(&mut surf, floor_kind, assets.floor(set)).quad(
                [
                    vec3(x, 0., z),
                    vec3(x + 1., 0., z),
                    vec3(x + 1., 0., z + 1.),
                    vec3(x, 0., z + 1.),
                ],
                UV1,
                up,
                WHITE,
                0.0,
            ),
        }
        if !matches!(step, Some((_, d)) if d > 0) {
            let corners = [
                vec3(x, h, z + 1.),
                vec3(x + 1., h, z + 1.),
                vec3(x + 1., h, z),
                vec3(x, h, z),
            ];
            match ceil_kind {
                Some(k) => painted(&mut surf, k, &assets.white).quad(
                    corners,
                    [vec2(0., 1.), vec2(1., 1.), vec2(1., 0.), vec2(0., 0.)],
                    -up,
                    WHITE,
                    0.0,
                ),
                None => plain.quad(corners, UV1, -up, ceiling, 0.0),
            }
        }
        let edges = [
            (0, -1, vec3(x + 1., 0., z), vec3(x, 0., z), vec3(0., 0., 1.)),
            (
                0,
                1,
                vec3(x, 0., z + 1.),
                vec3(x + 1., 0., z + 1.),
                vec3(0., 0., -1.),
            ),
            (-1, 0, vec3(x, 0., z), vec3(x, 0., z + 1.), vec3(1., 0., 0.)),
            (
                1,
                0,
                vec3(x + 1., 0., z + 1.),
                vec3(x + 1., 0., z),
                vec3(-1., 0., 0.),
            ),
        ];
        for (dx, dz, a, b, n) in edges {
            if !f.is_wall_cell(c.x + dx, c.y + dz) {
                continue;
            }
            // the painted wall carries its own cornice, chair rail and skirting
            painted(&mut surf, wall_kind, assets.wall(set)).quad(
                [a + up * h, b + up * h, b, a],
                [vec2(0., 0.), vec2(1., 0.), vec2(1., tiers), vec2(0., tiers)],
                n,
                WHITE,
                0.0,
            );
        }
    }
    archways(f, &mut plain);

    // room props: the original's Blender models (PropModelCatalog's originals, variant picked by
    // a hash of the room id like _variant_index), falling back to the blocky rebuilds
    let mut props = Builder::new(assets.props.values().next().map(|m| m.texture.clone()));
    let mut authored: HashMap<usize, Builder> = HashMap::new();
    for p in &f.props {
        let (x, z) = p.cell.center();
        let mut y = visual_y(&p.ptype, h, p.mount_y.unwrap_or(0.0) as f32);
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        let original = content
            .props
            .get(&p.ptype)
            .map(|d| d.originals())
            .filter(|o| !o.is_empty())
            .and_then(|o| {
                let room_id = &f.rooms[p.room].id;
                assets
                    .glb
                    .get(&o[stable_index(room_id, o.len())])
                    .or_else(|| o.iter().find_map(|m| assets.glb.get(m)))
            });
        if let Some(model) = original {
            // Surface children are authored with different local origins. Align the
            // model's actual lowest vertex to the authored tabletop height so computers,
            // printers and lamps sit on the parent instead of floating above it.
            if p.mount_y.is_some() {
                y -= model.min.y * PROP_MODEL_SCALE;
            }
            let xf = Mat4::from_translation(vec3(x, y, z))
                * Mat4::from_rotation_y(yaw)
                * Mat4::from_scale(Vec3::splat(PROP_MODEL_SCALE));
            model.add_to(&mut authored, xf, WHITE);
            continue;
        }
        let Some(model) = assets.props.get(&p.ptype) else {
            continue;
        };
        if p.mount_y.is_some() {
            y -= model.min.y * model.scale * PROP_MODEL_SCALE;
        }
        let mut verts = vec![];
        let mut idx = vec![];
        model.build_scaled(
            &mut verts,
            &mut idx,
            vec3(x, y, z),
            yaw,
            Vec3::splat(model.scale * PROP_MODEL_SCALE),
            0.0,
            0.0,
            WHITE,
        );
        soften(&mut verts);
        props.reserve(verts.len());
        let base_i = props.verts.len() as u16;
        props.verts.extend(verts);
        props.idx.extend(idx.into_iter().map(|i| i + base_i));
    }
    // emissive lamp fixtures for the switchable room lights
    for l in &f.room_lights {
        let (x, z) = l.center.center();
        plain.cube(
            vec3(x, h - 0.04, z),
            vec3(0.7, 0.08, 0.7),
            0.0,
            Color::new(l.color.0, l.color.1, l.color.2, 1.0),
            2.0,
        );
    }

    let mut meshes = vec![];
    for (_, b) in surf {
        meshes.extend(b.finish());
    }
    meshes.extend(props.finish());
    for (_, b) in authored {
        meshes.extend(b.finish());
    }
    meshes.extend(plain.finish());
    WorldMeshes {
        meshes,
        lamps: lamps(f, lights_on, content),
    }
}

/// Averages normals across corners that share a position, so the blocky props shade as
/// rounded forms instead of hard boxes.
fn soften(verts: &mut [Vertex]) {
    let key = |p: Vec3| {
        (
            (p.x * 64.0).round() as i32,
            (p.y * 64.0).round() as i32,
            (p.z * 64.0).round() as i32,
        )
    };
    let mut sum: HashMap<(i32, i32, i32), Vec3> = HashMap::new();
    for v in verts.iter() {
        *sum.entry(key(v.position)).or_insert(Vec3::ZERO) += v.normal.truncate();
    }
    for v in verts.iter_mut() {
        let face = v.normal.truncate();
        let avg = sum[&key(v.position)].normalize_or_zero();
        let n = (face * 0.45 + avg * 0.55).normalize_or_zero();
        v.normal = vec4(n.x, n.y, n.z, v.normal.w);
    }
}

/// Arched frames where a hall opens into a room: two dark pillars at the sides of the opening
/// and a stepped arch under a lintel.
fn archways(f: &Facility, b: &mut Builder) {
    let wood = Color::from_rgba(72, 42, 32, 255);
    let trim = Color::from_rgba(150, 108, 52, 255);
    let h = WALL_HEIGHT;
    for r in f.rooms.iter() {
        // each side: cells just inside the edge and the outward step
        let sides: [(Vec<(i32, i32)>, (i32, i32)); 4] = [
            ((r.x..r.x + r.w).map(|x| (x, r.y)).collect(), (0, -1)),
            (
                (r.x..r.x + r.w).map(|x| (x, r.y + r.h - 1)).collect(),
                (0, 1),
            ),
            ((r.y..r.y + r.h).map(|y| (r.x, y)).collect(), (-1, 0)),
            (
                (r.y..r.y + r.h).map(|y| (r.x + r.w - 1, y)).collect(),
                (1, 0),
            ),
        ];
        for (cells, (ox, oy)) in sides {
            let open: Vec<bool> = cells
                .iter()
                .map(|(x, y)| !f.is_wall_cell(*x, *y) && !f.is_wall_cell(x + ox, y + oy))
                .collect();
            let mut i = 0;
            while i < cells.len() {
                if !open[i] {
                    i += 1;
                    continue;
                }
                let s = i;
                while i < cells.len() && open[i] {
                    i += 1;
                }
                let e = i - 1;
                if e - s + 1 > 5 {
                    continue; // a wall-less side, not a doorway
                }
                let (sx, sy) = cells[s];
                let (ex, ey) = cells[e];
                // the opening's plane sits on the room's boundary line
                let (px, pz, along) = if ox != 0 {
                    let edge = if ox < 0 { sx as f32 } else { sx as f32 + 1.0 };
                    (edge, 0.0, false)
                } else {
                    let edge = if oy < 0 { sy as f32 } else { sy as f32 + 1.0 };
                    (0.0, edge, true)
                };
                let (a0, a1) = if along {
                    (sx as f32, ex as f32 + 1.0)
                } else {
                    (sy as f32, ey as f32 + 1.0)
                };
                let width = a1 - a0;
                let mid = (a0 + a1) / 2.0;
                let at = |t: f32, y: f32| {
                    if along {
                        vec3(t, y, pz)
                    } else {
                        vec3(px, y, t)
                    }
                };
                let size = |w: f32, hh: f32| {
                    if along {
                        vec3(w, hh, 0.32)
                    } else {
                        vec3(0.32, hh, w)
                    }
                };
                // pillars
                for t in [a0 + 0.08, a1 - 0.08] {
                    b.cube(at(t, 1.35), size(0.16, 2.7), 0.0, wood, 0.0);
                    b.cube(at(t, 1.05), size(0.2, 0.06), 0.0, trim, 0.0);
                }
                // lintel and a stepped arch beneath it
                b.cube(
                    at(mid, (2.7 + h) / 2.0),
                    size(width, h - 2.7),
                    0.0,
                    wood,
                    0.0,
                );
                let steps = 4;
                for k in 0..steps {
                    let frac = (k as f32 + 1.0) / (steps as f32 + 1.0);
                    let inset = (1.0 - (1.0 - frac * frac).sqrt()) * width * 0.5;
                    let y = 2.7 - (k as f32 + 1.0) * 0.09;
                    for t in [a0 + inset * 0.5, a1 - inset * 0.5] {
                        b.cube(
                            at(t, y + 0.045),
                            size(inset.max(0.05), 0.09),
                            0.0,
                            wood,
                            0.0,
                        );
                    }
                }
                b.cube(at(mid, 2.7 - 0.02), size(width, 0.04), 0.0, trim, 0.0);
            }
        }
    }
}
