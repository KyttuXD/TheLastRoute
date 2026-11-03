//! The floor's static geometry: floors and walls in each room region's own pictures (halls in
//! the maze set), baseboard and brass rail trims, ceilings, archways where halls open into
//! rooms, and the room props: the original's authored Blender models where a type has them
//! (assets/props/<set>/*.glb), else the blocky rebuilds as Blockbench meshes with smoothed
//! normals. Lights: the switchable room lights and the lamp props' own lights.

use super::builder::Builder;
use super::door;
use super::prop_scale;
use super::shader::Lamp;
use super::surfaces;
use super::textures::{self, TexRef, TextureChoice};
use crate::shared::assets::Assets;
use crate::config::WALL_HEIGHT;
use crate::content::props::{stable_index, visual_y};
use crate::content::regions::{RegionDef, RegionSafeLight};
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
    // the safe zone is a checkpoint: a warm light over its pad, so it reads as a lit area from the halls
    if let Some(s) = f.safe_zone {
        let (x, z) = s.center();
        let light = safe_light_for(f, s.x, s.y);
        out.push(Lamp {
            pos: vec3(x, WALL_HEIGHT - 0.6, z),
            range: 7.0,
            color: Color::new(light.color[0], light.color[1], light.color[2], 1.0),
            energy: light.energy,
        });
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
        let (cx, cz) = p.cell.center();
        let (x, z) = (cx + p.offset.0, cz + p.offset.1);
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

/// A region's wall height in metres: its `geometry.wall_height`, or WALL_HEIGHT when the region has no block
/// (or no region).
fn geometry_wall_height(def: Option<&RegionDef>) -> f32 {
    def.and_then(|d| d.geometry).map_or(WALL_HEIGHT, |g| g.wall_height)
}

/// The wall height of the region named `region` (its installed `lr.region/1` definition), else WALL_HEIGHT.
fn region_wall_height(region: &str) -> f32 {
    geometry_wall_height(crate::content::regions::table().def(region))
}

/// The region a floor cell is in: its room's region, or the district of its hall when the cell is in no room
/// (or the lobby). The same choice `cell_surfaces` makes for the cell's textures.
fn cell_region(f: &Facility, x: i32, y: i32) -> &str {
    match f.room_at(x as f64 + 0.5, y as f64 + 0.5).filter(|i| !f.rooms[*i].lobby()) {
        Some(i) => &f.rooms[i].region,
        None => f.region_at(x, y),
    }
}

/// The wall height of the region a floor cell is in.
fn cell_wall_height(f: &Facility, x: i32, y: i32) -> f32 {
    region_wall_height(cell_region(f, x, y))
}

/// The safe-zone checkpoint light of the region that holds the cell (x, y): its `safe_light` block, or the
/// game's defaults when the region has none (or no region is installed for it).
fn safe_light_for(f: &Facility, x: i32, y: i32) -> RegionSafeLight {
    safe_light_of(crate::content::regions::table().def(cell_region(f, x, y)))
}

/// A region's safe-zone light: its `safe_light` block, or the defaults when the region has no block (or no region).
fn safe_light_of(def: Option<&RegionDef>) -> RegionSafeLight {
    def.and_then(|d| d.safe_light).unwrap_or_default()
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
        // this cell's walls and ceiling take its region's wall height (WALL_HEIGHT without a geometry block)
        let wall_h = cell_wall_height(f, c.x, c.y);
        let wall_tiers = textures::wall_tiers(wall_h) as f32;
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
                vec3(x, wall_h, z + 1.),
                vec3(x + 1., wall_h, z + 1.),
                vec3(x + 1., wall_h, z),
                vec3(x, wall_h, z),
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
                [a + up * wall_h, b + up * wall_h, b, a],
                [vec2(0., 0.), vec2(1., 0.), vec2(1., wall_tiers), vec2(0., wall_tiers)],
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
        // a room door is centred on its doorway (a double door sits across two cells); any other prop is
        // drawn at its placement offset (a flush bed sits against its wall), as the footprint is
        let (x, z) = match p.door {
            Some(d) => (x + d.offset[0], z + d.offset[1]),
            None => (x + p.offset.0, z + p.offset.1),
        };
        let mut y = visual_y(&p.ptype, h, p.mount_y.unwrap_or(0.0) as f32);
        let yaw = p.rotation as f32 * std::f32::consts::FRAC_PI_2;
        // the model's drawn turn: a room door keeps the gameplay yaw, every other prop gets the half turn
        let visual_yaw = prop_visual_yaw(yaw, p.door.is_some());
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
        // Godot's per-type hunter scale (PropModelCatalog.HUNTER_SCALE, data/render/prop_draw_scale.json).
        let scale = prop_scale::draw_scale(&p.ptype);
        if let Some(model) = original {
            // Surface children are authored with different local origins. Align the
            // model's actual lowest vertex to the authored tabletop height so computers,
            // printers and lamps sit on the parent instead of floating above it.
            if p.mount_y.is_some() {
                y -= model.min.y * scale;
            }
            // a room door is raised to the hunter's opening and widened to its doorway (door.rs, Godot's
            // RouteInteractable3D raise); every other prop is the uniform draw scale
            let xf = match p.door.zip(model.door) {
                Some((d, dims)) => {
                    let (sx, sy) = door::door_scale(dims, &p.ptype, d.span);
                    // the wall over the opening, from the raised frame's top up to the ceiling (Godot's door header)
                    let bottom = door::header_bottom(dims, sy, h);
                    if bottom < h {
                        let (choice, _) = cell_surfaces(f, p.cell.x, p.cell.y, &mut choices);
                        let wall = choice.wall_at((p.cell.x + f.origin.0) as i64, (p.cell.y + f.origin.1) as i64);
                        let fallback = assets.wall(room_set(f, p.cell.x, p.cell.y));
                        door_header(&mut surf, wall, fallback, vec3(x, 0.0, z), yaw, d.span as f32, bottom, h, tiers);
                    }
                    door::door_transform(vec3(x, y, z), yaw, sx, sy, scale)
                }
                None => Mat4::from_translation(vec3(x, y, z))
                    * Mat4::from_rotation_y(visual_yaw)
                    * Mat4::from_scale(Vec3::splat(scale)),
            };
            model.add_to(&mut authored, xf, WHITE);
            continue;
        }
        let Some(model) = assets.props.get(&p.ptype) else {
            continue;
        };
        if p.mount_y.is_some() {
            y -= model.min.y * model.scale * scale;
        }
        let mut verts = vec![];
        let mut idx = vec![];
        model.build_scaled(
            &mut verts,
            &mut idx,
            vec3(x, y, z),
            visual_yaw,
            Vec3::splat(model.scale * scale),
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

/// The radians a room prop's model is turned by when it is drawn, from its gameplay yaw. Godot turns every
/// authored prop a half turn on top of its gameplay yaw (PropModelCatalog.authored_visual_yaw, applied in
/// streamed_facility_view_3d.gd `_spawn_authored_prop`): the model's front is local +Z and gameplay forward is -Z.
/// Room doors are not turned; RouteInteractable3D carries the raw yaw (door.rs `door_transform`).
pub fn prop_visual_yaw(yaw: f32, door: bool) -> f32 {
    if door {
        yaw
    } else {
        yaw + std::f32::consts::PI
    }
}

/// The wall over a room door's opening, from `bottom` (the frame's top) up to the ceiling `h` (Godot's door
/// header, streamed_facility_view_3d.gd add_door_header): a wall face on each side of the threshold cell and the
/// soffit under it. The door stands on its threshold's centre line, so the faces sit half a cell either side of the
/// origin, and the header spans the doorway's `span` cells across the wall (the door's local X).
#[allow(clippy::too_many_arguments)]
fn door_header(
    map: &mut HashMap<(&'static str, &'static str), Builder>,
    tex: TexRef,
    fallback: &Texture2D,
    origin: Vec3,
    yaw: f32,
    span: f32,
    bottom: f32,
    h: f32,
    tiers: f32,
) {
    let rot = Mat3::from_rotation_y(yaw);
    let (across, through) = (rot * Vec3::X, rot * Vec3::Z);
    let half = span / 2.0;
    let at = |s: f32, d: f32, y: f32| origin + across * s + through * d + Vec3::Y * y;
    // the wall texture's v runs from 0 at the top of the wall to `tiers` at its floor
    let v = tiers * (h - bottom) / h;
    let b = painted(map, tex, fallback);
    for (d, n) in [(0.5, through), (-0.5, -through)] {
        let (tl, tr, br, bl) = (at(-half, d, h), at(half, d, h), at(half, d, bottom), at(-half, d, bottom));
        let uv = [vec2(0., 0.), vec2(span, 0.), vec2(span, v), vec2(0., v)];
        if d > 0.0 {
            b.quad([tl, bl, br, tr], [uv[0], uv[3], uv[2], uv[1]], n, WHITE, 0.0);
        } else {
            b.quad([tl, tr, br, bl], uv, n, WHITE, 0.0);
        }
    }
    b.quad(
        [
            at(-half, -0.5, bottom),
            at(half, -0.5, bottom),
            at(half, 0.5, bottom),
            at(-half, 0.5, bottom),
        ],
        [vec2(0., 0.), vec2(span, 0.), vec2(span, 1.), vec2(0., 1.)],
        -Vec3::Y,
        WHITE,
        0.0,
    );
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

/// A run of open cells along one side of a room where a hall opens into it: the cells, in order along the
/// side, and the outward step from the room to the hall.
#[derive(Debug, Clone, PartialEq)]
struct ArchwayRun {
    cells: Vec<(i32, i32)>,
    outward: (i32, i32),
}

/// The doorways of a room `(x, y, w, h)`: on each side, each run of open cells whose outward cell is open too,
/// up to five cells wide (a longer run is a wall-less side, not a doorway). `is_wall` reads the wall grid.
fn archway_runs(rect: (i32, i32, i32, i32), is_wall: impl Fn(i32, i32) -> bool) -> Vec<ArchwayRun> {
    let (x0, y0, w, h) = rect;
    // each side: cells just inside the edge and the outward step
    let sides: [(Vec<(i32, i32)>, (i32, i32)); 4] = [
        ((x0..x0 + w).map(|x| (x, y0)).collect(), (0, -1)),
        ((x0..x0 + w).map(|x| (x, y0 + h - 1)).collect(), (0, 1)),
        ((y0..y0 + h).map(|y| (x0, y)).collect(), (-1, 0)),
        ((y0..y0 + h).map(|y| (x0 + w - 1, y)).collect(), (1, 0)),
    ];
    let mut runs = vec![];
    for (cells, outward) in sides {
        let (ox, oy) = outward;
        let open: Vec<bool> = cells
            .iter()
            .map(|(x, y)| !is_wall(*x, *y) && !is_wall(x + ox, y + oy))
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
            runs.push(ArchwayRun {
                cells: cells[s..=e].to_vec(),
                outward,
            });
        }
    }
    runs
}

/// Arched frames where a hall opens into a room: two dark pillars at the sides of the opening
/// and a stepped arch under a lintel.
fn archways(f: &Facility, b: &mut Builder) {
    let wood = Color::from_rgba(72, 42, 32, 255);
    let trim = Color::from_rgba(150, 108, 52, 255);
    for r in f.rooms.iter() {
        // the lintel stands under the room's own wall height
        let h = region_wall_height(&r.region);
        for run in archway_runs((r.x, r.y, r.w, r.h), |x, y| f.is_wall_cell(x, y)) {
            let (ox, oy) = run.outward;
            let (sx, sy) = run.cells[0];
            let (ex, ey) = run.cells[run.cells.len() - 1];
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};
    use crate::world::facility::{Cell, DoorDraw};

    /// A temporary region with a geometry block is parsed like a pack's file: its wall height is its own, and a
    /// region without the block (or no region) keeps WALL_HEIGHT.
    #[test]
    fn a_region_geometry_block_sets_its_wall_height() {
        use crate::content::regions::parse_file;
        let tall = r#"{"schema":"lr.region/1","id":"tmp_tall","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[],"geometry":{"wall_height":6.5}}"#;
        let plain = r#"{"schema":"lr.region/1","id":"tmp_plain","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[]}"#;
        let tall = parse_file("tmp_tall.json", tall).unwrap();
        let plain = parse_file("tmp_plain.json", plain).unwrap();
        assert_eq!(geometry_wall_height(Some(&tall)), 6.5);
        assert_eq!(geometry_wall_height(Some(&plain)), WALL_HEIGHT);
        assert_eq!(geometry_wall_height(None), WALL_HEIGHT);
    }

    /// A region's safe-zone light comes from its `safe_light` block. A region without the block, or no region,
    /// gets the checkpoint light the game shipped with (warm colour, energy 2.0).
    #[test]
    fn a_region_safe_light_block_sets_the_checkpoint_light_and_the_rest_default() {
        use crate::content::regions::parse_file;
        let lit = r#"{"schema":"lr.region/1","id":"tmp_lit","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[],"safe_light":{"color":[0.1,0.2,0.3],"energy":5.0}}"#;
        let plain = r#"{"schema":"lr.region/1","id":"tmp_dim","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[]}"#;
        let lit = parse_file("tmp_lit.json", lit).unwrap();
        let plain = parse_file("tmp_dim.json", plain).unwrap();
        assert_eq!(safe_light_of(Some(&lit)), RegionSafeLight { color: [0.1, 0.2, 0.3], energy: 5.0 });
        assert_eq!(safe_light_of(Some(&plain)), RegionSafeLight::default());
        assert_eq!(safe_light_of(None), RegionSafeLight::default());
        assert_eq!(safe_light_of(None).color, [0.95, 0.78, 0.55]);
        assert_eq!(safe_light_of(None).energy, 2.0);
    }

    /// Where a prop model's front (local +Z, the seat front of the chair GLBs) lands in (x, z) once drawn.
    fn drawn_front(gameplay_yaw: f32) -> (f32, f32) {
        let turn = Mat4::from_rotation_y(prop_visual_yaw(gameplay_yaw, false));
        let v = turn.transform_vector3(vec3(0.0, 0.0, 1.0));
        (v.x, v.z)
    }

    /// Gameplay forward in (x, z): (-sin yaw, -cos yaw). Golden check: yaw 0 gives open_direction (0, -1).
    fn forward(gameplay_yaw: f32) -> (f32, f32) {
        (-gameplay_yaw.sin(), -gameplay_yaw.cos())
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn wooden_chair_at_golden_yaw_zero_faces_its_open_direction() {
        // golden: woodenChairCushion at cell (8,6), rotation 0 (yaw 0), open_direction (0,-1)
        let rotation: i32 = 0;
        let yaw = rotation as f32 * FRAC_PI_2;
        assert!(close(drawn_front(yaw), (0.0, -1.0)));
        assert!(close(drawn_front(yaw), forward(yaw)));
    }

    #[test]
    fn office_chair_at_golden_yaw_minus_pi_faces_its_open_direction() {
        // golden: officeChair, yaw -PI, rotation round(-PI / (PI/2)) rem_euclid 4 = 2, open_direction (0,1)
        let rotation = ((-PI / FRAC_PI_2).round() as i32).rem_euclid(4);
        assert_eq!(rotation, 2);
        let yaw = rotation as f32 * FRAC_PI_2;
        assert!(close(drawn_front(yaw), (0.0, 1.0)));
    }

    #[test]
    fn every_quarter_turn_draws_the_front_on_gameplay_forward() {
        for rotation in 0..4 {
            let yaw = rotation as f32 * FRAC_PI_2;
            assert!(close(drawn_front(yaw), forward(yaw)), "rotation {rotation}");
        }
    }

    #[test]
    fn room_door_keeps_the_gameplay_yaw() {
        assert_eq!(prop_visual_yaw(FRAC_PI_2, true), FRAC_PI_2);
        assert_eq!(prop_visual_yaw(FRAC_PI_2, false), FRAC_PI_2 + PI);
    }

    /// The cells a room door's header covers, placed as `door_header` places it: the doorway's `span` cells
    /// across the wall (local X from the yaw), centred on the threshold cell's centre plus the drawn offset.
    fn door_header_cells(cell: Cell, rotation: i32, door: DoorDraw) -> Vec<(i32, i32)> {
        let yaw = rotation as f32 * FRAC_PI_2;
        let across = Mat3::from_rotation_y(yaw) * Vec3::X;
        let (cx, cz) = cell.center();
        let centre = vec3(cx + door.offset[0], 0.0, cz + door.offset[1]);
        (0..door.span)
            .map(|i| {
                let p = centre + across * (i as f32 - (door.span as f32 - 1.0) / 2.0);
                (p.x.floor() as i32, p.z.floor() as i32)
            })
            .collect()
    }

    /// A door's header and an archway's lintel are both drawn over a doorway, so they must not share a cell.
    ///
    /// Why this checks the cell pickers and not a generated floor: a real floor needs `Facility::generate`
    /// with the content catalog, which a unit test cannot build cheaply. So this runs the archway picker
    /// (`archway_runs`) and the door header's cell placement (`door_header_cells`, which mirrors the `across`
    /// and centre math of `door_header`) on a hand-made 4x4 room with one doorway in its top wall, and puts the
    /// door on its west wall. It shows that these two placements stay apart. It does not show that a generated
    /// door never lands on an archway run.
    #[test]
    fn door_header_and_archway_never_share_a_cell() {
        // room floor x 2..=5, y 2..=5; a hall at y 0..=1 open at x 3 and 4; every other cell is wall
        let is_wall = |x: i32, y: i32| {
            let room = (2..6).contains(&x) && (2..6).contains(&y);
            let hall = (3..5).contains(&x) && (0..2).contains(&y);
            !(room || hall)
        };
        let runs = archway_runs((2, 2, 4, 4), is_wall);
        assert_eq!(runs.len(), 1, "one doorway in the top wall: {runs:?}");
        assert_eq!(runs[0].cells, vec![(3, 2), (4, 2)]);
        assert_eq!(runs[0].outward, (0, -1));
        let archway: Vec<(i32, i32)> = runs.iter().flat_map(|r| r.cells.iter().copied()).collect();

        // a double door on the west wall: threshold (2, 4), across the wall along z (rotation 1)
        let door = door_header_cells(Cell::new(2, 4), 1, DoorDraw { span: 2, offset: [0.0, 0.0] });
        assert_eq!(door.len(), 2, "a double door covers two cells: {door:?}");
        for c in &door {
            assert!(!archway.contains(c), "door cell {c:?} is also an archway cell");
        }
    }
}
