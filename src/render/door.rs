//! Room doors: the leaf's closed pose, the raise to the hunter's opening, the widen to the doorway and the
//! placement across a double door. Ported from Godot's room door drawing: `RouteInteractable3D` closes the
//! hinges (route_interactable_3d.gd:891-893, 923-928) and raises and widens the visual (1005-1026, with
//! `DoorwayFrames.raise_scale`, doorway_frames.gd:17-19, 62-88), and the planner offsets a double door across
//! the wall (streamed_route_generator.gd:5117-5118).
//!
//! Units are metres; one grid cell is one metre. The door model's local X runs along the wall, Y is up,
//! and Z is the depth into the room. Its origin is the opening's centre on the floor.

use crate::world::facility::DoorDraw;
use macroquad::prelude::{vec3, Mat4, Vec3};

/// The hunter clearance the opening must reach (`DoorwayFrames.TARGET_OPENING`, 3.15 m).
pub const TARGET_OPENING: f32 = 3.15;
/// The tallest a normal door's visual may be raised to (`DoorwayFrames`' cap, 3.65 m).
pub const RAISE_CAP: f32 = 3.65;
/// The tallest a gothic door may be raised to: its spire is taller (4.02 m).
pub const GOTHIC_RAISE_CAP: f32 = 4.02;
/// The widest a door may be stretched across its doorway (route_interactable_3d.gd:1025).
pub const MAX_WIDEN: f32 = 1.25;

/// The hinge pose when closed (route_interactable_3d.gd:891-893): `DoorHinge` turns -68 degrees and
/// `DoorHingeRight` +68 degrees, which turns the authored open leaf (+68 under DoorHinge) flat in the wall.
/// `None` for any node that is not a hinge.
pub fn closed_hinge_deg(node: &str) -> Option<f32> {
    match node {
        "DoorHinge" => Some(-68.0),
        "DoorHingeRight" => Some(68.0),
        _ => None,
    }
}

/// A hinge's local transform with its rotation set to the closed pose. The translation is kept, so the pivot
/// stays at the jamb; the authored hinges have no rotation, so replacing it loses nothing.
pub fn closed_hinge_local(local: Mat4, deg: f32) -> Mat4 {
    Mat4::from_translation(local.w_axis.truncate()) * Mat4::from_rotation_y(deg.to_radians())
}

/// The door's own measures, read from its model's nodes (see `DoorMeasure`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorDims {
    /// The opening's height: the lower of the leaf's top and the header's underside.
    pub opening: f32,
    /// The whole visual's top (the frame, or a gothic spire).
    pub top: f32,
    /// The clear width of a single doorway: twice the inner face of the jamb posts.
    pub inner: f32,
}

/// Collects the door measures while a model's nodes are walked. The leaf is every vertex under a closed
/// hinge, the header is `DoorFrameTop`, and the jamb posts are `DoorFrameL` and `DoorFrameR`.
#[derive(Clone, Copy, Debug)]
pub struct DoorMeasure {
    hinged: bool,
    leaf_top: f32,
    header_low: f32,
    jamb_inner: f32,
}

impl Default for DoorMeasure {
    fn default() -> Self {
        DoorMeasure {
            hinged: false,
            leaf_top: f32::MIN,
            header_low: f32::MAX,
            jamb_inner: f32::MAX,
        }
    }
}

impl DoorMeasure {
    /// The model has a hinge node: it is a door.
    pub fn hinge(&mut self) {
        self.hinged = true;
    }

    /// A world-space vertex under a closed hinge.
    pub fn leaf_vertex(&mut self, y: f32) {
        self.leaf_top = self.leaf_top.max(y);
    }

    /// A world-space vertex of `DoorFrameTop`.
    pub fn header_vertex(&mut self, y: f32) {
        self.header_low = self.header_low.min(y);
    }

    /// A world-space vertex of `DoorFrameL` or `DoorFrameR`; the post's inner face is its smallest |x|.
    pub fn jamb_vertex(&mut self, x: f32) {
        self.jamb_inner = self.jamb_inner.min(x.abs());
    }

    /// The door's dimensions, with `top` the whole model's top. `None` for a model without a hinge, or
    /// without the leaf or posts it needs.
    pub fn finish(&self, top: f32) -> Option<DoorDims> {
        if !self.hinged || self.leaf_top == f32::MIN || self.header_low == f32::MAX || self.jamb_inner == f32::MAX {
            return None;
        }
        Some(DoorDims {
            opening: self.leaf_top.min(self.header_low),
            top,
            inner: 2.0 * self.jamb_inner,
        })
    }
}

/// The raise (`DoorwayFrames.raise_scale`): the scale of the visual's Y that brings the opening to the hunter's
/// clearance, held to the cap on the top and to between 1 and 2.
pub fn raise_scale(opening: f32, top: f32, cap: f32) -> f32 {
    (TARGET_OPENING / opening).min(cap / top).clamp(1.0, 2.0)
}

/// The widen (route_interactable_3d.gd:1025): the scale of the visual's X that fills a doorway `span` metres
/// wide, with the frame's inner width `inner`, held to between 1 and 1.25. Z is not scaled.
pub fn widen_scale(span: f32, inner: f32) -> f32 {
    ((span - 0.02) / inner).clamp(1.0, MAX_WIDEN)
}

/// The raise cap by door type: gothic doors have the taller spire.
pub fn raise_cap(ptype: &str) -> f32 {
    if ptype.starts_with("gothic") {
        GOTHIC_RAISE_CAP
    } else {
        RAISE_CAP
    }
}

/// The door's (X, Y) scale for a doorway `span` cells wide.
pub fn door_scale(dims: DoorDims, ptype: &str, span: i32) -> (f32, f32) {
    let sy = raise_scale(dims.opening, dims.top, raise_cap(ptype));
    let sx = widen_scale(span as f32, dims.inner);
    (sx, sy)
}

/// The underside of the wall over a door (Godot's `DoorwayFrames.header_y`): just under the raised frame's top
/// (`dims.top * sy`), and never above `wall_top`, so the header runs up to the ceiling.
pub fn header_bottom(dims: DoorDims, sy: f32, wall_top: f32) -> f32 {
    (dims.top * sy - 0.02).min(wall_top)
}

/// The door's transform: the origin on the floor, then the yaw, then the scale in the door's own axes, as
/// Godot scales the visual before it turns the door.
pub fn door_transform(origin: Vec3, yaw: f32, sx: f32, sy: f32, scale: f32) -> Mat4 {
    Mat4::from_translation(origin) * Mat4::from_rotation_y(yaw) * Mat4::from_scale(vec3(sx * scale, sy * scale, scale))
}

/// The centre of a doorway relative to its threshold cell (streamed_route_generator.gd:5117-5118, the
/// `placement_offset` that `across * (span - 1) * 0.5` gives). The doorway's cells are the threshold cell and
/// its neighbours across the wall, so their mean minus the threshold cell is that offset.
pub fn doorway_offset(cell: (i32, i32), doorway: &[(i32, i32)]) -> [f32; 2] {
    if doorway.is_empty() {
        return [0.0, 0.0];
    }
    let n = doorway.len() as f32;
    let (sx, sy) = doorway
        .iter()
        .fold((0.0f32, 0.0f32), |acc, c| (acc.0 + c.0 as f32, acc.1 + c.1 as f32));
    [sx / n - cell.0 as f32, sy / n - cell.1 as f32]
}

/// A room door's drawing for a planned door: its span in cells and the centre offset.
pub fn draw_for(cell: (i32, i32), doorway: &[(i32, i32)]) -> DoorDraw {
    DoorDraw {
        span: doorway.len() as i32,
        offset: doorway_offset(cell, doorway),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gltf::Gltf;

    const NORMAL_SINGLE: &[u8] = include_bytes!("../../assets/props/mansion/normal_room_door.glb");
    const NORMAL_DOUBLE: &[u8] = include_bytes!("../../assets/props/mansion/normal_double_room_door.glb");

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    /// The door's measures, read from the GLB's node tree the same way `GlbModel::parse` reads it.
    fn measure(bytes: &[u8]) -> DoorDims {
        let g = Gltf::from_slice(bytes).expect("a glb");
        let blob = g.blob.as_deref().expect("a binary glb");
        let scene = g.default_scene().or_else(|| g.scenes().next()).expect("a scene");
        let mut stack: Vec<(gltf::Node, Mat4, bool)> =
            scene.nodes().map(|n| (n, Mat4::IDENTITY, false)).collect();
        let mut measure = DoorMeasure::default();
        let mut top = f32::MIN;
        while let Some((node, parent, under)) = stack.pop() {
            let name = node.name().unwrap_or("").to_string();
            let mut local = Mat4::from_cols_array_2d(&node.transform().matrix());
            if let Some(deg) = closed_hinge_deg(&name) {
                local = closed_hinge_local(local, deg);
                measure.hinge();
            }
            let world = parent * local;
            let under = under || closed_hinge_deg(&name).is_some();
            for child in node.children() {
                stack.push((child, world, under));
            }
            let Some(mesh) = node.mesh() else { continue };
            for prim in mesh.primitives() {
                let reader = prim.reader(|_| Some(blob));
                let Some(positions) = reader.read_positions() else { continue };
                for p in positions {
                    let w = world.transform_point3(Vec3::from(p));
                    top = top.max(w.y);
                    if under {
                        measure.leaf_vertex(w.y);
                    }
                    match name.as_str() {
                        "DoorFrameTop" => measure.header_vertex(w.y),
                        "DoorFrameL" | "DoorFrameR" => measure.jamb_vertex(w.x),
                        _ => {}
                    }
                }
            }
        }
        measure.finish(top).expect("a door")
    }

    #[test]
    fn normal_single_door_reads_its_opening_and_jambs_from_the_glb() {
        let dims = measure(NORMAL_SINGLE);
        assert!(close(dims.opening, 2.37, 0.005), "opening {}", dims.opening);
        assert!(close(dims.top, 2.55, 0.005), "top {}", dims.top);
        assert!(close(dims.inner, 0.88, 0.005), "inner {}", dims.inner);
    }

    #[test]
    fn normal_single_door_is_raised_and_widened_to_fill_its_doorway() {
        let dims = measure(NORMAL_SINGLE);
        let (sx, sy) = door_scale(dims, "normalRoomDoor", 1);
        assert!(close(sy, 1.329, 0.002), "sy {sy}");
        assert!(close(sx, 1.114, 0.002), "sx {sx}");
        // the frame top and the leaf top after the raise (3.39 m and 3.15 m)
        let xf = door_transform(Vec3::ZERO, 0.0, sx, sy, 1.0);
        let frame_top = xf.transform_point3(vec3(0.0, 2.55, 0.0)).y;
        let leaf_top = xf.transform_point3(vec3(0.0, 2.37, 0.0)).y;
        assert!(close(frame_top, 3.39, 0.01), "frame top {frame_top}");
        assert!(close(leaf_top, 3.15, 0.01), "leaf top {leaf_top}");
        // the hinge pivot (the jamb, x -0.43 m) moves out to -0.479 m with the widen
        let hinge = xf.transform_point3(vec3(-0.43, 0.0, 0.0));
        assert!(close(hinge.x, -0.479, 0.002), "hinge x {}", hinge.x);
        assert!(close(hinge.y, 0.0, 1e-6) && close(hinge.z, 0.0, 1e-6), "hinge pivot {hinge:?}");
    }

    #[test]
    fn closed_hinge_turns_the_leaf_flat_across_the_opening() {
        // The open leaf's origin is (0.1648, 1.19, -0.408) under DoorHinge at (-0.43, 0, 0), and its
        // authored +68 degree turn cancels the hinge's closed -68 degrees.
        let hinge = Mat4::from_translation(vec3(-0.43, 0.0, 0.0));
        let closed = closed_hinge_local(hinge, closed_hinge_deg("DoorHinge").unwrap());
        let leaf = closed.transform_point3(vec3(0.164_826_9, 1.19, -0.408_079_6));
        assert!(close(leaf.x, 0.01, 0.005), "leaf x {}", leaf.x);
        assert!(close(leaf.z, 0.0, 0.005), "leaf z {}", leaf.z);
        assert!(close(leaf.y, 1.19, 1e-6), "the hinge keeps the leaf's height ({})", leaf.y);
        assert_eq!(closed_hinge_deg("DoorHingeRight"), Some(68.0));
        assert_eq!(closed_hinge_deg("DoorFrameL"), None);
    }

    #[test]
    fn double_door_offset_centres_the_two_cells_and_widens_to_its_span() {
        // Doorway cells (4, 2) and (5, 2) across +x, threshold cell (4, 2): centre at +0.5 m.
        let offset = doorway_offset((4, 2), &[(4, 2), (5, 2)]);
        assert!(close(offset[0], 0.5, 1e-6) && close(offset[1], 0.0, 1e-6), "offset {offset:?}");
        // A three-cell doorway: centre at +1.0 m.
        let wide = doorway_offset((4, 2), &[(4, 2), (5, 2), (6, 2)]);
        assert!(close(wide[0], 1.0, 1e-6), "wide offset {wide:?}");
        // A single cell has no offset.
        assert_eq!(doorway_offset((4, 2), &[(4, 2)]), [0.0, 0.0]);
        let draw = draw_for((4, 2), &[(4, 2), (5, 2)]);
        assert_eq!(draw.span, 2);
        // The double door GLB widens by 1.031 (1.98 m over its 1.92 m frame), and the closed hinges sit at +-0.96.
        let dims = measure(NORMAL_DOUBLE);
        assert!(close(dims.inner, 1.92, 0.005), "double inner {}", dims.inner);
        let (sx, sy) = door_scale(dims, "normalDoubleRoomDoor", 2);
        assert!(close(sx, 1.031, 0.002), "double sx {sx}");
        assert!(close(sy, 1.329, 0.002), "double sy {sy}");
    }

    #[test]
    fn raise_is_held_to_the_cap_and_never_below_one() {
        // a tall opening needs no raise, and a low one is capped by the top
        assert!(close(raise_scale(3.5, 3.6, RAISE_CAP), 1.0, 1e-6));
        assert!(close(raise_scale(2.37, 2.55, RAISE_CAP), 1.329, 0.002));
        assert_eq!(raise_scale(2.0, 4.0, RAISE_CAP), 1.0);
        assert!(close(widen_scale(1.0, 0.88), 1.114, 0.002));
        assert!(close(widen_scale(3.0, 0.88), 1.25, 1e-6));
        assert_eq!(raise_cap("gothicRoomDoor"), GOTHIC_RAISE_CAP);
        assert_eq!(raise_cap("normalRoomDoor"), RAISE_CAP);
    }
}
