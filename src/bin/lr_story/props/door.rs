//! Room doors: the leaf's closed pose, the raise to the hunter's opening, the widen to the doorway and the
//! placement across a double door. Ported from Godot's room door drawing: `RouteInteractable3D` closes the
//! hinges (route_interactable_3d.gd:891-893, 923-928) and raises and widens the visual (1005-1026, with
//! `DoorwayFrames.raise_scale`, doorway_frames.gd:17-19, 62-88), and the planner offsets a double door across
//! the wall (streamed_route_generator.gd:5117-5118).
//!
//! Units are metres; one grid cell is one metre. The door model's local X runs along the wall, Y is up,
//! and Z is the depth into the room. Its origin is the opening's centre on the floor.

use macroquad::prelude::{vec3, Mat3, Mat4, Vec3};

/// How a room door is drawn: the doorway's width in cells (`span`) and the offset of its centre from the threshold
/// cell. A copy of `world::facility::DoorDraw` (the world module is not copied into lr_story).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorDraw {
    pub span: i32,
    pub offset: [f32; 2],
}

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
    use macroquad::prelude::vec3;

    // The door-model tests (single, double, gothic and luxury doors measured from their bbmodel files) stay with the
    // game: those models are not in lr_story's assets. The tests kept here need no model file.

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
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
    fn doorway_offset_centres_the_cells_of_a_double_door() {
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
        assert!(close(draw.offset[0], 0.5, 1e-6));
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
}
