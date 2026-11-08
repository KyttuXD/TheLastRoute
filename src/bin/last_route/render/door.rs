//! Room doors: the leaf's closed pose, the raise to the hunter's opening, the widen to the doorway and the
//! placement across a double door. Ported from Godot's room door drawing: `RouteInteractable3D` closes the
//! hinges (route_interactable_3d.gd:891-893, 923-928) and raises and widens the visual (1005-1026, with
//! `DoorwayFrames.raise_scale`, doorway_frames.gd:17-19, 62-88), and the planner offsets a double door across
//! the wall (streamed_route_generator.gd:5117-5118).
//!
//! Units are metres; one grid cell is one metre. The door model's local X runs along the wall, Y is up,
//! and Z is the depth into the room. Its origin is the opening's centre on the floor.

use crate::world::facility::DoorDraw;
use macroquad::prelude::{vec3, Mat3, Mat4, Vec3};

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

/// The smoothed swing for an open amount (0 closed, 1 open): slow at both ends, so a leaf eases off its latch.
pub fn swing_ease(open: f32) -> f32 {
    let t = open.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A hinge's rotation for an open amount: the closed pose at 0 and the authored open leaf (no turn) at 1.
/// `None` for any node that is not a hinge. Feed it to `closed_hinge_local` in place of the closed angle.
pub fn hinge_deg(node: &str, open: f32) -> Option<f32> {
    closed_hinge_deg(node).map(|closed| closed * (1.0 - swing_ease(open)))
}


/// How far the posts' inner faces stand inside the wall gap on each side (Godot's `DoorwayFrames.POST_PROUD`
/// is 1 cm). Flush, they would z-fight with the gap's faces; 1 mm is far under the pixel at any distance the
/// door is drawn from, and keeps the clear opening within 2 mm of the doorway.
pub const POST_PROUD: f32 = 0.001;
/// The wall over a door overlaps the frame's top by this much, so the soffit never leaves a crack on the bar.
pub const HEADER_OVERLAP: f32 = 0.001;
/// Half the frame's depth (the models' posts and bar run z -0.09 to 0.09).
pub const FRAME_HALF_DEPTH: f32 = 0.09;
/// Where a widened frame's posts start to slide out: this share of the clear half-width, so the middle (a crest,
/// the lintel's ornament) stays as authored (Godot's `WIDE_DOOR_OUTER_SHARE`).
pub const WIDE_OUTER_SHARE: f32 = 0.6;

/// The door's own measures, read from its model's nodes (see `DoorMeasure`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorDims {
    /// The opening's height: the lower of the leaf's top and the header's underside.
    pub opening: f32,
    /// The whole visual's top (the frame, or a gothic spire).
    pub top: f32,
    /// The clear width of a single doorway: twice the inner face of the jamb posts.
    pub inner: f32,
    /// The top of the leaf, closed.
    pub leaf_top: f32,
    /// The header bar's underside: where the leaf must reach.
    pub header: f32,
    /// The header bar's top: where the wall over the door meets the frame.
    pub bar_top: f32,
    /// Two leaves (a `DoorHingeRight` hinge), meeting in the middle.
    pub double: bool,
}

/// Collects the door measures while a model's nodes are walked. The leaf is every vertex under a closed
/// hinge, the header is `DoorFrameTop`, and the jamb posts are `DoorFrameL` and `DoorFrameR`.
#[derive(Clone, Copy, Debug)]
pub struct DoorMeasure {
    hinged: bool,
    double: bool,
    leaf_top: f32,
    header_low: f32,
    header_high: f32,
    jamb_inner: f32,
}

impl Default for DoorMeasure {
    fn default() -> Self {
        DoorMeasure {
            hinged: false,
            double: false,
            leaf_top: f32::MIN,
            header_low: f32::MAX,
            header_high: f32::MIN,
            jamb_inner: f32::MAX,
        }
    }
}

impl DoorMeasure {
    /// The model has a hinge node (named `name`): it is a door.
    pub fn hinge(&mut self, name: &str) {
        self.hinged = true;
        self.double |= name == "DoorHingeRight";
    }

    /// A world-space vertex under a closed hinge.
    pub fn leaf_vertex(&mut self, y: f32) {
        self.leaf_top = self.leaf_top.max(y);
    }

    /// A world-space vertex of `DoorFrameTop`.
    pub fn header_vertex(&mut self, y: f32) {
        self.header_low = self.header_low.min(y);
        self.header_high = self.header_high.max(y);
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
            leaf_top: self.leaf_top,
            header: self.header_low,
            bar_top: self.header_high,
            double: self.double,
        })
    }
}

/// The raise (`DoorwayFrames.raise_scale`): the scale of the visual's Y that brings the opening to the hunter's
/// clearance, held to the cap on the top and to between 1 and 2.
pub fn raise_scale(opening: f32, top: f32, cap: f32) -> f32 {
    (TARGET_OPENING / opening).min(cap / top).clamp(1.0, 2.0)
}

/// The widen (route_interactable_3d.gd:1025): the scale of the visual's X that fills a doorway `span` metres
/// wide, with the frame's inner width `inner`, held to between 1 and 1.25. Z is not scaled. A doorway wider
/// than the clamp allows is finished by `DoorFit::shift`.
pub fn widen_scale(span: f32, inner: f32) -> f32 {
    ((span - 2.0 * POST_PROUD) / inner).clamp(1.0, MAX_WIDEN)
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

/// How a door model is fitted to its doorway: the uniform raise and widen (`door_transform`), and for a doorway
/// wider than the widen allows (a three-cell double door) the posts slide out the rest of the way, as Godot's
/// `_widen_double_door` rebuilds the frame instead of stretching it. All lengths but `sx` and `sy` are in the
/// model's own units, before the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorFit {
    pub sx: f32,
    pub sy: f32,
    /// How far each side's posts and sill slide outward (0 when the widen alone fills the doorway).
    pub shift: f32,
    /// Static-frame vertices at |x| of at least this slide.
    pub from: f32,
    /// Half the clear width once fitted: the posts' inner faces, and where the leaves end.
    pub jamb: f32,
}

impl DoorFit {
    /// The fit of `dims` into a doorway `span` cells wide.
    pub fn new(dims: DoorDims, ptype: &str, span: i32) -> DoorFit {
        let (sx, sy) = door_scale(dims, ptype, span);
        let want = (span as f32 - 2.0 * POST_PROUD) / sx;
        let shift = ((want - dims.inner) * 0.5).max(0.0);
        DoorFit {
            sx,
            sy,
            shift,
            from: dims.inner * 0.5 * WIDE_OUTER_SHARE,
            jamb: dims.inner * 0.5 + shift,
        }
    }

    /// A frame vertex's model X after the posts slide out.
    pub fn body_x(&self, x: f32) -> f32 {
        if self.shift > 0.0 && x.abs() >= self.from {
            x + x.signum() * self.shift
        } else {
            x
        }
    }
}

/// How one leaf is fitted between the posts: its X span is stretched along its own width (`widen`) and moved
/// (`dx`, model space) so the closed leaf ends exactly on the jamb's inner face and on the other leaf, and its
/// height is stretched (`ky`) to meet the header bar. `axis` is the leaf's width direction in the hinge's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeafFit {
    pub widen: f32,
    pub dx: f32,
    pub ky: f32,
    pub axis: Vec3,
}

impl LeafFit {
    /// No change.
    pub const NONE: LeafFit = LeafFit {
        widen: 1.0,
        dx: 0.0,
        ky: 1.0,
        axis: Vec3::X,
    };
}

/// The fit of the leaf under hinge `name`, whose closed leaf spans model X `closed_x` (low, high) and whose
/// pivot is the end at the jamb (the low end for `DoorHinge`, the high end for `DoorHingeRight`). A single door's
/// leaf fills jamb to jamb; each leaf of a double door fills its jamb to the middle.
pub fn leaf_fit(dims: DoorDims, fit: &DoorFit, name: &str, closed_x: (f32, f32)) -> LeafFit {
    let Some(deg) = closed_hinge_deg(name) else {
        return LeafFit::NONE;
    };
    let right = name == "DoorHingeRight";
    let (lo, hi) = closed_x;
    let (t_lo, t_hi) = match (right, dims.double) {
        (false, false) => (-fit.jamb, fit.jamb),
        (false, true) => (-fit.jamb, 0.0),
        (true, _) => (0.0, fit.jamb),
    };
    let widen = if hi - lo > 1e-4 { (t_hi - t_lo) / (hi - lo) } else { 1.0 };
    let dx = if right { t_hi - hi } else { t_lo - lo };
    let ky = if dims.leaf_top > 0.0 && dims.header > 0.0 {
        dims.header / dims.leaf_top
    } else {
        1.0
    };
    LeafFit {
        widen,
        dx,
        ky,
        axis: Mat4::from_rotation_y((-deg).to_radians()).transform_vector3(Vec3::X),
    }
}

/// A hinge leaf's model transform at open amount `open`: the swing about the hinge (`hinge_deg`), the leaf
/// stretched in its own frame, and the pivot moved to the fitted jamb. `base` and `local` are the hinge's parent
/// transform and authored local transform.
pub fn leaf_xf(base: Mat4, local: Mat4, name: &str, open: f32, fit: &LeafFit) -> Mat4 {
    let deg = hinge_deg(name, open).unwrap_or(0.0);
    let u = fit.axis;
    let grow = fit.widen - 1.0;
    // I + (widen - 1) u u^T: a stretch along the leaf's width only
    let stretch = Mat3::from_cols(
        Vec3::X + u * (grow * u.x),
        Vec3::Y + u * (grow * u.y),
        Vec3::Z + u * (grow * u.z),
    );
    let tall = Mat3::from_diagonal(vec3(1.0, fit.ky, 1.0));
    Mat4::from_translation(vec3(fit.dx, 0.0, 0.0)) * base * closed_hinge_local(local, deg) * Mat4::from_mat3(tall * stretch)
}

/// The underside of the wall over a door: just under the raised frame's top (`dims.top * sy`, so a crest or spire
/// stands in the recess), and never above `wall_top`, so the header runs up to the ceiling.
pub fn header_bottom(dims: DoorDims, sy: f32, wall_top: f32) -> f32 {
    (dims.top * sy - HEADER_OVERLAP).min(wall_top)
}

/// Where the filler between the header bar and the soffit starts: the bar's top, raised. It is `None` when the
/// bar is the frame's top (a plain door), so the wall meets the bar with nothing between.
pub fn filler_from(dims: DoorDims, sy: f32) -> Option<f32> {
    let from = dims.bar_top * sy - HEADER_OVERLAP;
    (dims.top * sy - HEADER_OVERLAP - from > 2.0 * HEADER_OVERLAP + 1e-4).then_some(from)
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
    use serde_json::Value;

    // the models' bbmodel text: the tests read the same groups and pivots the loader reads (render/prop_model.rs), without a
    // graphics context
    const NORMAL_SINGLE: &str = include_str!("../assets/props/mansion/normal_room_door.bbmodel");
    const NORMAL_DOUBLE: &str = include_str!("../assets/props/mansion/normal_double_room_door.bbmodel");

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn vec_of(v: &Value) -> Vec3 {
        let a = v.as_array().expect("a vector");
        let g = |i: usize| a[i].as_f64().expect("a number") as f32;
        vec3(g(0), g(1), g(2))
    }

    /// An element's corner points in metres: a mesh's vertices, a cube's eight corners.
    fn element_points(e: &Value) -> Vec<Vec3> {
        if e["type"].as_str() == Some("mesh") {
            return e["vertices"].as_object().expect("vertices").values().map(|v| vec_of(v) / 16.0).collect();
        }
        let (f, t) = (vec_of(&e["from"]), vec_of(&e["to"]));
        (0..8)
            .map(|i| vec3(if i & 1 == 0 { f.x } else { t.x }, if i & 2 == 0 { f.y } else { t.y }, if i & 4 == 0 { f.z } else { t.z }) / 16.0)
            .collect()
    }

    /// A model's points by outliner group in metres: (group name, group pivot, points); an element outside any group
    /// is in the group "" with the origin as its pivot.
    fn model_points(text: &str) -> Vec<(String, Vec3, Vec<Vec3>)> {
        let j: Value = serde_json::from_str(text).expect("a bbmodel");
        let by_uuid = |u: &str| -> Vec<Vec3> {
            let e = j["elements"].as_array().expect("elements").iter().find(|e| e["uuid"].as_str() == Some(u)).expect("an element");
            element_points(e)
        };
        let mut out: Vec<(String, Vec3, Vec<Vec3>)> = vec![];
        for node in j["outliner"].as_array().expect("an outliner") {
            match node.as_str() {
                Some(u) => out.push((String::new(), Vec3::ZERO, by_uuid(u))),
                None => {
                    let pts = node["children"].as_array().expect("children").iter().flat_map(|c| by_uuid(c.as_str().expect("an element"))).collect();
                    out.push((node["name"].as_str().expect("a name").to_string(), vec_of(&node["origin"]) / 16.0, pts));
                }
            }
        }
        out
    }

    /// The door's measures, read from the model's groups the same way `PropModel::parse` reads them: the leaf under
    /// the closed hinge, the header and the posts by group name.
    fn measure(text: &str) -> DoorDims {
        let mut measure = DoorMeasure::default();
        let mut top = f32::MIN;
        for (name, pivot, pts) in model_points(text) {
            let deg = closed_hinge_deg(&name);
            if deg.is_some() {
                measure.hinge(&name);
            }
            for p in pts {
                let w = match deg {
                    Some(deg) => closed_hinge_local(Mat4::from_translation(pivot), deg).transform_point3(p - pivot),
                    None => p,
                };
                top = top.max(w.y);
                if deg.is_some() {
                    measure.leaf_vertex(w.y);
                }
                match name.as_str() {
                    "DoorFrameTop" => measure.header_vertex(w.y),
                    "DoorFrameL" | "DoorFrameR" => measure.jamb_vertex(w.x),
                    _ => {}
                }
            }
        }
        measure.finish(top).expect("a door")
    }

    #[test]
    fn normal_single_door_reads_its_opening_and_jambs_from_the_model() {
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
        assert!(close(sx, 1.134, 0.002), "sx {sx}");
        // the frame top and the leaf top after the raise (3.39 m and 3.15 m)
        let xf = door_transform(Vec3::ZERO, 0.0, sx, sy, 1.0);
        let frame_top = xf.transform_point3(vec3(0.0, 2.55, 0.0)).y;
        let leaf_top = xf.transform_point3(vec3(0.0, 2.37, 0.0)).y;
        assert!(close(frame_top, 3.39, 0.01), "frame top {frame_top}");
        assert!(close(leaf_top, 3.15, 0.01), "leaf top {leaf_top}");
        // the hinge pivot (the jamb, x -0.43 m) moves out to -0.4876 m with the widen
        let hinge = xf.transform_point3(vec3(-0.43, 0.0, 0.0));
        assert!(close(hinge.x, -0.4876, 0.002), "hinge x {}", hinge.x);
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
    fn hinges_swing_from_closed_to_the_authored_open_pose() {
        assert_eq!(hinge_deg("DoorHinge", 0.0), Some(-68.0));
        assert_eq!(hinge_deg("DoorHingeRight", 0.0), Some(68.0));
        assert_eq!(hinge_deg("DoorHinge", 1.0), Some(0.0));
        let half = hinge_deg("DoorHinge", 0.5).unwrap();
        assert!(close(half, -34.0, 1e-4), "midway {half}");
        assert_eq!(hinge_deg("DoorFrameL", 0.5), None);
        assert_eq!(swing_ease(-1.0), 0.0);
        assert_eq!(swing_ease(2.0), 1.0);
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
        // The double door model widens by 1.041 (1.998 m over its 1.92 m frame), and the closed hinges sit at +-0.96.
        let dims = measure(NORMAL_DOUBLE);
        assert!(close(dims.inner, 1.92, 0.005), "double inner {}", dims.inner);
        let (sx, sy) = door_scale(dims, "normalDoubleRoomDoor", 2);
        assert!(close(sx, 1.041, 0.002), "double sx {sx}");
        assert!(close(sy, 1.329, 0.002), "double sy {sy}");
    }

    #[test]
    fn raise_is_held_to_the_cap_and_never_below_one() {
        // a tall opening needs no raise, and a low one is capped by the top
        assert!(close(raise_scale(3.5, 3.6, RAISE_CAP), 1.0, 1e-6));
        assert!(close(raise_scale(2.37, 2.55, RAISE_CAP), 1.329, 0.002));
        assert_eq!(raise_scale(2.0, 4.0, RAISE_CAP), 1.0);
        assert!(close(widen_scale(1.0, 0.88), 1.134, 0.002));
        assert!(close(widen_scale(3.0, 0.88), 1.25, 1e-6));
        assert_eq!(raise_cap("gothicRoomDoor"), GOTHIC_RAISE_CAP);
        assert_eq!(raise_cap("normalRoomDoor"), RAISE_CAP);
    }

    const GOTHIC_SINGLE: &str = include_str!("../assets/props/mansion/gothic_room_door.bbmodel");
    const GOTHIC_DOUBLE: &str = include_str!("../assets/props/mansion/gothic_double_room_door.bbmodel");
    const LUXURY_SINGLE: &str = include_str!("../assets/props/mansion/luxury_room_door.bbmodel");
    const LUXURY_DOUBLE: &str = include_str!("../assets/props/mansion/luxury_double_room_door.bbmodel");

    /// Every door model with its prop type, and whether it is a double door.
    fn all_models() -> Vec<(&'static str, &'static str, bool)> {
        vec![
            ("normalRoomDoor", NORMAL_SINGLE, false),
            ("luxuryRoomDoor", LUXURY_SINGLE, false),
            ("gothicRoomDoor", GOTHIC_SINGLE, false),
            ("normalDoubleRoomDoor", NORMAL_DOUBLE, true),
            ("luxuryDoubleRoomDoor", LUXURY_DOUBLE, true),
            ("gothicDoubleRoomDoor", GOTHIC_DOUBLE, true),
        ]
    }

    /// An axis-aligned box.
    #[derive(Clone, Copy)]
    struct Bounds {
        lo: Vec3,
        hi: Vec3,
    }

    impl Bounds {
        fn empty() -> Bounds {
            Bounds {
                lo: Vec3::splat(f32::MAX),
                hi: Vec3::splat(f32::MIN),
            }
        }
        fn add(&mut self, p: Vec3) {
            self.lo = self.lo.min(p);
            self.hi = self.hi.max(p);
        }
    }

    /// One hinge of a door model as the model loader keeps it: its frame, and its leaf's vertices in that frame.
    struct TestHinge {
        name: String,
        base: Mat4,
        local: Mat4,
        verts: Vec<Vec3>,
    }

    /// A door model split the way `PropModel::parse` splits it: the frame's vertices by node name (model space,
    /// hinges closed) and the hinges.
    struct TestDoor {
        body: Vec<(String, Vec3)>,
        hinges: Vec<TestHinge>,
    }

    fn load(text: &str) -> TestDoor {
        let mut door = TestDoor { body: vec![], hinges: vec![] };
        for (name, pivot, pts) in model_points(text) {
            if closed_hinge_deg(&name).is_some() {
                door.hinges.push(TestHinge {
                    name,
                    base: Mat4::IDENTITY,
                    local: Mat4::from_translation(pivot),
                    verts: pts.into_iter().map(|p| p - pivot).collect(),
                });
            } else {
                door.body.extend(pts.into_iter().map(|p| (name.clone(), p)));
            }
        }
        door
    }

    /// The closed leaf's span along the model's X, before any fit (what `Hinge::closed_x` holds).
    fn closed_x(h: &TestHinge) -> (f32, f32) {
        let xf = h.base * closed_hinge_local(h.local, closed_hinge_deg(&h.name).unwrap());
        h.verts.iter().fold((f32::MAX, f32::MIN), |acc, v| {
            let x = xf.transform_point3(*v).x;
            (acc.0.min(x), acc.1.max(x))
        })
    }

    /// The built door of a doorway `span` cells wide, in the doorway's own frame (origin on the floor at the
    /// centre, X along the wall, as `door_xf` places it): the frame node `name`'s box, and each leaf's box at `open`.
    struct Built {
        frame: Vec<(String, Bounds)>,
        leaves: Vec<Bounds>,
        fit: DoorFit,
        dims: DoorDims,
    }

    fn build(text: &str, ptype: &str, span: i32, open: f32) -> Built {
        let dims = measure(text);
        let door = load(text);
        let fit = DoorFit::new(dims, ptype, span);
        let xf = door_transform(Vec3::ZERO, 0.0, fit.sx, fit.sy, 1.0);
        let mut frame: Vec<(String, Bounds)> = vec![];
        for (name, p) in &door.body {
            let w = xf.transform_point3(vec3(fit.body_x(p.x), p.y, p.z));
            match frame.iter_mut().find(|(n, _)| n == name) {
                Some((_, b)) => b.add(w),
                None => {
                    let mut b = Bounds::empty();
                    b.add(w);
                    frame.push((name.clone(), b));
                }
            }
        }
        let leaves = door
            .hinges
            .iter()
            .map(|h| {
                let leaf = leaf_fit(dims, &fit, &h.name, closed_x(h));
                let m = xf * leaf_xf(h.base, h.local, &h.name, open, &leaf);
                let mut b = Bounds::empty();
                for v in &h.verts {
                    b.add(m.transform_point3(*v));
                }
                b
            })
            .collect();
        Built { frame, leaves, fit, dims }
    }

    impl Built {
        fn frame(&self, name: &str) -> Bounds {
            self.frame.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}")).1
        }
    }

    /// The wall opening of a doorway `span` cells wide is `span` metres between the wall ends. For every door model
    /// and doorway width, the posts' inner faces stand within 1 mm of those wall ends (and the posts reach into the
    /// wall, so no gap shows behind them), the closed leaves fill the clear opening edge to edge and meet in the
    /// middle, and the leaves reach the header bar's underside.
    #[test]
    fn doors_fill_their_doorway_flush_with_the_wall_ends() {
        let mm = 0.001 + 1e-4;
        for (ptype, text, double) in all_models() {
            let spans: Vec<i32> = if double { vec![2, 3] } else { vec![1] };
            for span in spans {
                let b = build(text, ptype, span, 0.0);
                let half = span as f32 / 2.0;
                let (l, r, top) = (b.frame("DoorFrameL"), b.frame("DoorFrameR"), b.frame("DoorFrameTop"));
                let ctx = format!("{ptype} span {span}");
                // hinge side and latch side: the posts' inner faces on the wall ends, their outer faces in the wall
                assert!((l.hi.x + half).abs() <= mm, "{ctx}: left post inner {} vs wall end {}", l.hi.x, -half);
                assert!((r.lo.x - half).abs() <= mm, "{ctx}: right post inner {} vs wall end {}", r.lo.x, half);
                assert!(l.lo.x <= -half && r.hi.x >= half, "{ctx}: posts reach into the wall");
                // the bar spans wall end to wall end (it runs into the wall on both sides)
                assert!(top.lo.x <= -half && top.hi.x >= half, "{ctx}: bar {}..{}", top.lo.x, top.hi.x);
                // the leaves fill the clear opening edge to edge
                let lo = b.leaves.iter().map(|x| x.lo.x).fold(f32::MAX, f32::min);
                let hi = b.leaves.iter().map(|x| x.hi.x).fold(f32::MIN, f32::max);
                assert!((lo - l.hi.x).abs() <= 0.001, "{ctx}: leaf low {lo} vs post {}", l.hi.x);
                assert!((hi - r.lo.x).abs() <= 0.001, "{ctx}: leaf high {hi} vs post {}", r.lo.x);
                if double {
                    assert_eq!(b.leaves.len(), 2, "{ctx}");
                    let (a, c) = (b.leaves[0], b.leaves[1]);
                    let (left, right) = if a.lo.x < c.lo.x { (a, c) } else { (c, a) };
                    assert!((left.hi.x - right.lo.x).abs() <= 0.001, "{ctx}: leaves meet at {} / {}", left.hi.x, right.lo.x);
                    assert!(left.hi.x.abs() <= 0.001, "{ctx}: leaves meet in the middle ({})", left.hi.x);
                } else {
                    assert_eq!(b.leaves.len(), 1, "{ctx}");
                }
                // the top: the leaf reaches the bar's underside, so no crack shows over it
                for leaf in &b.leaves {
                    assert!((leaf.hi.y - top.lo.y).abs() <= 0.001, "{ctx}: leaf top {} vs bar underside {}", leaf.hi.y, top.lo.y);
                    assert!(leaf.lo.y >= -1e-4 && leaf.lo.y <= 0.05, "{ctx}: leaf stands on the floor ({})", leaf.lo.y);
                }
                // the wall over the door meets the frame: the soffit overlaps the bar's top by under 1 mm (or a
                // filler band spans the gap above the bar up to a crest's top)
                let bottom = header_bottom(b.dims, b.fit.sy, 5.0);
                let bar_top = top.hi.y;
                match filler_from(b.dims, b.fit.sy) {
                    None => assert!((bottom - bar_top).abs() <= mm, "{ctx}: soffit {bottom} vs bar top {bar_top}"),
                    Some(from) => {
                        assert!((from - bar_top).abs() <= mm, "{ctx}: filler from {from} vs bar top {bar_top}");
                        let crest = b.frame.iter().map(|(_, x)| x.hi.y).fold(f32::MIN, f32::max);
                        assert!((bottom - crest).abs() <= mm, "{ctx}: soffit {bottom} vs crest top {crest}");
                    }
                }
            }
        }
    }

    /// The gap reported in the field: a three-cell double door used to stop at the widen's clamp (1.25), leaving
    /// 29 cm between each post and the wall end. The posts now slide out to the wall ends, and the leaves widen.
    #[test]
    fn three_cell_double_door_no_longer_leaves_a_gap_at_the_wall_ends() {
        let dims = measure(NORMAL_DOUBLE);
        let fit = DoorFit::new(dims, "normalDoubleRoomDoor", 3);
        assert!(close(fit.sx, MAX_WIDEN, 1e-6), "clamped widen {}", fit.sx);
        assert!(fit.shift > 0.2, "the posts slide out the rest of the way ({})", fit.shift);
        // without the slide the inner face stands at 1.2 m of the 1.5 m wall end
        assert!(close(dims.inner * 0.5 * fit.sx, 1.2, 0.01));
        let b = build(NORMAL_DOUBLE, "normalDoubleRoomDoor", 3, 0.0);
        let inner = b.frame("DoorFrameR").lo.x;
        assert!(close(inner, 1.5, 0.0011), "right post inner face {inner}");
        // an open leaf keeps its widened width: it stands further out than a two-cell leaf's
        let open = build(NORMAL_DOUBLE, "normalDoubleRoomDoor", 3, 1.0);
        let two = build(NORMAL_DOUBLE, "normalDoubleRoomDoor", 2, 1.0);
        let reach = |x: &Built| x.leaves.iter().map(|l| l.hi.z - l.lo.z).fold(f32::MIN, f32::max);
        assert!(reach(&open) > reach(&two), "{} vs {}", reach(&open), reach(&two));
    }

    /// A placed door must draw something: for every door model, each doorway it is placed in (single, and double at
    /// spans 2 and 3) and both the closed and the open pose, the frame's three pieces and each leaf are finite,
    /// non-flat boxes, a single door has one leaf and a double door two, and each leaf is a door's size. A model whose
    /// frame or leaves came out empty, flat or non-finite fails here, before it reaches the screen.
    #[test]
    fn every_door_model_draws_its_frame_and_leaves_for_a_placed_door() {
        fn finite_box(b: &Bounds) -> bool {
            b.lo.is_finite() && b.hi.is_finite() && b.lo.cmple(b.hi).all()
        }
        for (ptype, text, double) in all_models() {
            let spans: Vec<i32> = if double { vec![2, 3] } else { vec![1] };
            for span in spans {
                for open in [0.0, 1.0] {
                    let b = build(text, ptype, span, open);
                    let ctx = format!("{ptype} span {span} open {open}");
                    for name in ["DoorFrameL", "DoorFrameR", "DoorFrameTop"] {
                        let f = b.frame(name);
                        assert!(finite_box(&f), "{ctx}: {name} is not a finite box");
                        assert!(f.hi.x > f.lo.x && f.hi.y > f.lo.y, "{ctx}: {name} is flat");
                    }
                    assert_eq!(b.leaves.len(), if double { 2 } else { 1 }, "{ctx}: leaves");
                    for leaf in &b.leaves {
                        assert!(finite_box(leaf), "{ctx}: a leaf is not a finite box");
                        let height = leaf.hi.y - leaf.lo.y;
                        assert!(height > 1.5, "{ctx}: a leaf is {height} m tall");
                        let width = (leaf.hi.x - leaf.lo.x).hypot(leaf.hi.z - leaf.lo.z);
                        assert!(width > 0.3, "{ctx}: a leaf is {width} m across");
                    }
                }
            }
        }
    }

    /// A door with a crest (luxury) or a spire (gothic) is taller than its bar: the wall over it starts above the
    /// crest, and a filler band closes the slit between the bar and that soffit. A plain door needs none.
    #[test]
    fn crest_doors_get_a_filler_and_plain_doors_do_not() {
        let plain = build(NORMAL_SINGLE, "normalRoomDoor", 1, 0.0);
        assert!(filler_from(plain.dims, plain.fit.sy).is_none());
        for (ptype, text) in [("luxuryRoomDoor", LUXURY_SINGLE), ("gothicRoomDoor", GOTHIC_SINGLE)] {
            let b = build(text, ptype, 1, 0.0);
            let from = filler_from(b.dims, b.fit.sy).unwrap_or_else(|| panic!("{ptype} needs a filler"));
            assert!(from < header_bottom(b.dims, b.fit.sy, 5.0), "{ptype}");
        }
    }

    /// The doorway's wall ends are cell boundaries: a doorway of cells x0..x0+span centred at x0 + span/2 has its ends
    /// `span / 2` either side, which is where the posts stand.
    #[test]
    fn doorway_centre_puts_the_wall_ends_on_cell_boundaries() {
        for span in 1..=3 {
            let cells: Vec<(i32, i32)> = (0..span).map(|i| (4 + i, 2)).collect();
            let offset = doorway_offset((4, 2), &cells);
            let centre = 4.5 + offset[0];
            assert!(close(centre - span as f32 / 2.0, 4.0, 1e-5), "span {span}: left end {}", centre - span as f32 / 2.0);
            assert!(close(centre + span as f32 / 2.0, (4 + span) as f32, 1e-5), "span {span}");
        }
    }

    /// The mansion and hotel doors (TODO: "Mansion doors are invisible"): each region's single and double door names
    /// from the room-door table resolve to a luxury model that builds, so a placed mansion door has a frame (three
    /// pieces and the crown over the bar) and a swinging leaf per hinge, all finite and with area, closed and open.
    #[test]
    fn mansion_and_hotel_door_models_build_a_frame_crown_and_leaves() {
        use crate::world::route::grid::ROOM_DOOR_TABLE;
        for region in ["mansion", "hotel"] {
            let (_, single, double) = *ROOM_DOOR_TABLE.iter().find(|(r, _, _)| *r == region).expect("a door table row");
            for (ptype, spans, leaves) in [(single, vec![1], 1usize), (double, vec![2, 3], 2)] {
                let text = all_models().into_iter().find(|(p, _, _)| *p == ptype).unwrap_or_else(|| panic!("{region}: no model for {ptype}")).1;
                for span in spans {
                    for open in [0.0, 1.0] {
                        let b = build(text, ptype, span, open);
                        let ctx = format!("{region} {ptype} span {span} open {open}");
                        assert!(b.dims.double == (leaves == 2), "{ctx}: door kind");
                        assert_eq!(b.leaves.len(), leaves, "{ctx}: leaves");
                        // the crown and the threshold are ungrouped elements (group "")
                        let loose = b.frame("");
                        let bar = b.frame("DoorFrameTop");
                        assert!(loose.hi.y > bar.hi.y + 0.05, "{ctx}: no crown above the bar ({} vs {})", loose.hi.y, bar.hi.y);
                        for leaf in &b.leaves {
                            let size = leaf.hi - leaf.lo;
                            assert!(size.is_finite() && size.y > 1.5 && size.x.hypot(size.z) > 0.3, "{ctx}: leaf {size:?}");
                        }
                    }
                }
            }
        }
    }

    /// A luxury door's crown is part of the frame body (it is in no hinge group, so the static batch draws it shut),
    /// it sits above the bar, and the raised crown stays under the raise cap and inside the wall recess the header
    /// leaves for it.
    #[test]
    fn luxury_crown_is_body_geometry_under_the_raise_cap() {
        for (ptype, text, double) in all_models().into_iter().filter(|(p, _, _)| p.starts_with("luxury")) {
            let load = load(text);
            assert!(load.body.iter().any(|(n, p)| n.is_empty() && p.y > 2.56), "{ptype}: no crown in the body");
            assert!(load.hinges.iter().all(|h| closed_hinge_deg(&h.name).is_some()), "{ptype}");
            let span = if double { 2 } else { 1 };
            let b = build(text, ptype, span, 0.0);
            assert!(b.dims.top > b.dims.bar_top, "{ptype}: the crown is taller than the bar");
            assert!(b.dims.top * b.fit.sy <= raise_cap(ptype) + 1e-4, "{ptype}: raised top {}", b.dims.top * b.fit.sy);
            assert!(header_bottom(b.dims, b.fit.sy, 5.0) >= b.frame("DoorFrameTop").hi.y, "{ptype}: the soffit clears the bar");
        }
    }
}
