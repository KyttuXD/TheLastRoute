//! World-space teammate markers (TODO "World-space teammate markers"): a name, a distance and a colour over each teammate
//! on screen; a teammate off screen (or behind the camera) gets an arrow on the screen edge pointing the way. A downed or
//! caught teammate shows DOWN or CAUGHT instead of the distance. A teammate whose last sample is old dims, then goes.
//! Tunables are in `data/hud/teammate_markers.json` (the same file sets the map pins, quick-chat pings and the waypoint
//! chevrons). The maths here takes plain numbers, so it is tested without a window; [`draw_world`] is the macroquad pass.

use super::markers::{self, Marker};
use super::style::*;
use crate::app::assets::Assets;
use crate::content::embedded;
use crate::game::Game;
use crate::render::{scene, stack_actors};
use macroquad::camera::Camera as _;
use macroquad::prelude::*;
use serde_json::Value;
use std::sync::OnceLock;

pub const FILE: &str = "hud/teammate_markers.json";

/// What the teammate is doing, for the word under the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Down,
    Caught,
}

/// The per-player waypoint tunables (the file's `waypoint` object).
#[derive(Clone, Debug, PartialEq)]
pub struct WaypointRules {
    pub chevron_spacing: f32,
    pub chevron_count: usize,
    pub max_range: f32,
    pub label: String,
    pub pin_alpha: f32,
}

/// Everything the file sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub stale_after_s: f32,
    pub stale_full_s: f32,
    pub stale_min_alpha: f32,
    pub gone_s: f32,
    pub show_distance: bool,
    pub max_distance: f32,
    pub edge_margin: f32,
    pub edge_arrow_size: f32,
    pub font_size: f32,
    pub outline: f32,
    pub down_label: String,
    pub caught_label: String,
    pub other_floor_alpha_minimap: f32,
    pub other_floor_alpha_full: f32,
    pub down_ring_hz: f32,
    pub down_ring_radius: f32,
    pub ping_seconds: f32,
    pub ping_radius: f32,
    pub waypoint: WaypointRules,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            stale_after_s: 2.0,
            stale_full_s: 8.0,
            stale_min_alpha: 0.35,
            gone_s: 20.0,
            show_distance: true,
            max_distance: 400.0,
            edge_margin: 28.0,
            edge_arrow_size: 5.0,
            font_size: 6.5,
            outline: 1.4,
            down_label: "DOWN".into(),
            caught_label: "CAUGHT".into(),
            other_floor_alpha_minimap: 0.3,
            other_floor_alpha_full: 0.35,
            down_ring_hz: 1.2,
            down_ring_radius: 4.5,
            ping_seconds: 4.0,
            ping_radius: 6.0,
            waypoint: WaypointRules { chevron_spacing: 1.6, chevron_count: 8, max_range: 90.0, label: "WAYPOINT".into(), pin_alpha: 1.0 },
        }
    }
}

impl Rules {
    /// Reads the file; a missing or wrong-typed field keeps its default, so a half-edited file still loads.
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(Value::as_str) != Some("lr.teammate_markers/1") {
            return Err("schema: expected lr.teammate_markers/1".into());
        }
        let d = Rules::default();
        let num = |o: &Value, k: &str, d: f32| o.get(k).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= 0.0).unwrap_or(d);
        let text_of = |o: &Value, k: &str, d: &str| o.get(k).and_then(Value::as_str).map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| d.to_string());
        let wp = v.get("waypoint").cloned().unwrap_or(Value::Null);
        let wd = d.waypoint.clone();
        let mut r = Rules {
            stale_after_s: num(&v, "stale_after_seconds", d.stale_after_s),
            stale_full_s: num(&v, "stale_full_seconds", d.stale_full_s),
            stale_min_alpha: num(&v, "stale_min_alpha", d.stale_min_alpha).min(1.0),
            gone_s: num(&v, "gone_seconds", d.gone_s),
            show_distance: v.get("show_distance").and_then(Value::as_bool).unwrap_or(d.show_distance),
            max_distance: num(&v, "max_distance", d.max_distance),
            edge_margin: num(&v, "edge_margin", d.edge_margin),
            edge_arrow_size: num(&v, "edge_arrow_size", d.edge_arrow_size),
            font_size: num(&v, "font_size", d.font_size).max(1.0),
            outline: num(&v, "outline", d.outline),
            down_label: text_of(&v, "down_label", &d.down_label),
            caught_label: text_of(&v, "caught_label", &d.caught_label),
            other_floor_alpha_minimap: num(&v, "other_floor_alpha_minimap", d.other_floor_alpha_minimap).min(1.0),
            other_floor_alpha_full: num(&v, "other_floor_alpha_full", d.other_floor_alpha_full).min(1.0),
            down_ring_hz: num(&v, "down_ring_hz", d.down_ring_hz),
            down_ring_radius: num(&v, "down_ring_radius", d.down_ring_radius),
            ping_seconds: num(&v, "ping_seconds", d.ping_seconds),
            ping_radius: num(&v, "ping_radius", d.ping_radius),
            waypoint: WaypointRules {
                chevron_spacing: num(&wp, "chevron_spacing", wd.chevron_spacing).max(0.1),
                chevron_count: wp.get("chevron_count").and_then(Value::as_u64).map(|n| (n as usize).min(64)).unwrap_or(wd.chevron_count),
                max_range: num(&wp, "max_range", wd.max_range),
                label: text_of(&wp, "label", &wd.label),
                pin_alpha: num(&wp, "pin_alpha", wd.pin_alpha).min(1.0),
            },
        };
        if r.stale_full_s <= r.stale_after_s {
            r.stale_full_s = r.stale_after_s + 1.0;
        }
        Ok(r)
    }
}

/// The shipped rules, read once; a bad file prints its error once and the defaults stand.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let Some((path, src)) = embedded::data_under(FILE).next() else { return Rules::default() };
        Rules::parse(src).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            Rules::default()
        })
    })
}

/// How strongly a teammate drawn from a sample `age_s` seconds old shows: 1 while fresh, then down to
/// `stale_min_alpha` at `stale_full_s`. Zero once it is older than `gone_s`.
pub fn stale_alpha(age_s: f32, r: &Rules) -> f32 {
    if !age_s.is_finite() || age_s < 0.0 || age_s >= r.gone_s {
        return if age_s.is_finite() && age_s < 0.0 { 1.0 } else { 0.0 };
    }
    if age_s <= r.stale_after_s {
        return 1.0;
    }
    if age_s >= r.stale_full_s {
        return r.stale_min_alpha;
    }
    let k = (age_s - r.stale_after_s) / (r.stale_full_s - r.stale_after_s);
    1.0 + (r.stale_min_alpha - 1.0) * k
}

/// The strength of a map pin: the stale dimming, times the other-floor strength when the pin is on a floor that is not
/// the shown one. Other floors are dimmed, never dropped (unless the data sets the strength to zero).
pub fn pin_strength(level: i32, shown_level: i32, full: bool, age_s: f32, r: &Rules) -> Option<f32> {
    let stale = stale_alpha(age_s, r);
    if stale <= 0.0 {
        return None;
    }
    let floor = if level == shown_level {
        1.0
    } else if full {
        r.other_floor_alpha_full
    } else {
        r.other_floor_alpha_minimap
    };
    (floor > 0.0).then_some(stale * floor)
}

/// The pulse of the ring round a downed teammate's pin at `time` seconds: a radius in cells and an alpha, both from the
/// data. `reduced` holds it still (reduced motion).
pub fn down_ring(time: f32, reduced: bool, r: &Rules) -> (f32, f32) {
    if reduced || r.down_ring_hz <= 0.0 {
        return (r.down_ring_radius, 0.6);
    }
    let phase = (time * r.down_ring_hz).fract().abs();
    (r.down_ring_radius * (0.7 + 0.3 * phase), 0.9 * (1.0 - phase) + 0.1)
}

/// The text under a name: the status word for a downed or caught teammate, else the distance in metres (when shown).
pub fn status_text(status: Status, dist_m: f32, r: &Rules) -> String {
    match status {
        Status::Down => r.down_label.clone(),
        Status::Caught => r.caught_label.clone(),
        Status::Ok if r.show_distance && dist_m.is_finite() => format!("{}M", dist_m.round().max(0.0) as i64),
        Status::Ok => String::new(),
    }
}

/// Where a marker goes: over the teammate on screen, or an arrow on the edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    OnScreen { x: f32, y: f32 },
    /// A point on the screen rectangle (inset by the margin) and the arrow's angle in radians (0 points right, y down).
    Edge { x: f32, y: f32, angle: f32 },
}

/// The point where a ray from the screen centre along `dir` (x right, y UP, as clip space) meets the rectangle inset by
/// `margin` pixels, and the arrow's angle on the screen. A zero `dir` points down.
pub fn edge_point(dir: (f32, f32), screen: (f32, f32), margin: f32) -> (f32, f32, f32) {
    let (mut dx, mut dy) = dir;
    if !dx.is_finite() || !dy.is_finite() || (dx.abs() < 1e-6 && dy.abs() < 1e-6) {
        dx = 0.0;
        dy = -1.0;
    }
    let (cx, cy) = (screen.0 * 0.5, screen.1 * 0.5);
    let hw = (cx - margin).max(1.0);
    let hh = (cy - margin).max(1.0);
    let t = (hw / dx.abs().max(1e-6)).min(hh / dy.abs().max(1e-6));
    (cx + dx * t, cy - dy * t, (-dy).atan2(dx))
}

/// Places a world point: on the screen if it projects inside it, else on the edge toward it (also when it is behind the
/// camera, where the direction is flipped so the arrow points the way to turn).
pub fn place(view_proj: Mat4, p: Vec3, screen: (f32, f32), margin: f32) -> Option<Placement> {
    if !p.is_finite() {
        return None;
    }
    if let Some((x, y)) = markers::project(view_proj, p, screen, 0.0) {
        return Some(Placement::OnScreen { x, y });
    }
    let clip = view_proj * p.extend(1.0);
    if !clip.is_finite() {
        return None;
    }
    let dir = if clip.w > 1e-4 { (clip.x / clip.w, clip.y / clip.w) } else { (-clip.x, -clip.y) };
    let (x, y, angle) = edge_point(dir, screen, margin);
    Some(Placement::Edge { x, y, angle })
}

/// One teammate to mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// The point the marker sits at (above the head).
    pub at: Vec3,
    pub status: Status,
    pub age_s: f32,
    pub colour: Color,
}

/// A marker ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub text: String,
    pub placement: Placement,
    pub alpha: f32,
    pub colour: Color,
    pub dist: f32,
}

/// The markers to draw for `entries` seen from `eye`: those past `max_distance` or older than `gone_s` are left out.
/// Farthest first, so the near ones draw on top.
pub fn layout(entries: &[Entry], eye: Vec3, view_proj: Mat4, screen: (f32, f32), r: &Rules) -> Vec<Drawn> {
    let mut out: Vec<Drawn> = entries
        .iter()
        .filter_map(|e| {
            let dist = (e.at - eye).length();
            let alpha = stale_alpha(e.age_s, r);
            if !dist.is_finite() || dist > r.max_distance || alpha <= 0.0 {
                return None;
            }
            let placement = place(view_proj, e.at, screen, r.edge_margin)?;
            let word = status_text(e.status, dist, r);
            let name = e.name.to_uppercase();
            let text = if word.is_empty() { name } else if name.is_empty() { word } else { format!("{name} {word}") };
            Some(Drawn { text, placement, alpha, colour: e.colour, dist })
        })
        .collect();
    out.sort_by(|a, b| b.dist.total_cmp(&a.dist));
    out
}

/// The entries of a marker list as seen on `active_level`: a body on another level that is not on a flight gets no marker
/// (the maps show it dimmed instead). `height_m` is how far above the feet the marker sits.
pub fn entries_from(marks: &[Marker], active_level: i32, height_m: f32) -> Vec<Entry> {
    marks
        .iter()
        .filter(|m| !m.is_bot || !m.name.is_empty())
        .filter(|m| m.level == active_level || stack_actors::on_link(m.abs_y))
        .map(|m| Entry {
            name: m.name.clone(),
            at: vec3(m.x, stack_actors::actor_dy(m.abs_y, active_level) + height_m, m.y),
            status: if m.downed { Status::Down } else { Status::Ok },
            age_s: m.age_s,
            colour: m.colour,
        })
        .collect()
}

/// The pass over the 3D view (call after `markers::draw_nametags`, on the default camera): names, distances and edge
/// arrows for the teammates. Skipped by the caller when the TEAMMATE WAYPOINTS-style overlay is off.
pub fn draw_world(g: &Game, a: &Assets, rt: &RenderTarget, marks: &[Marker]) {
    let r = rules();
    let cam = scene::camera(g, rt);
    let screen = (screen_width(), screen_height());
    let entries = entries_from(marks, g.fac.level, markers::rules().height_m + 0.4);
    let u = unit();
    for d in layout(&entries, cam.position, cam.matrix(), screen, r) {
        let ink = Color::new(0.01, 0.04, 0.05, 0.95 * d.alpha);
        let col = alpha(d.colour, d.alpha);
        match d.placement {
            Placement::OnScreen { x, y } => {
                let top = y / u - lh(F::Body, r.font_size) - 1.0;
                let left = x / u - tw(a, F::Body, &d.text, r.font_size) * 0.5;
                label_outline(a, F::Body, &d.text, left, top, r.font_size, col, ink, r.outline);
            }
            Placement::Edge { x, y, angle } => {
                let c = vec2(x / u, y / u);
                let f = vec2(angle.cos(), angle.sin());
                let s = vec2(-f.y, f.x);
                let l = r.edge_arrow_size;
                tri(c + f * l, c - f * l * 0.7 + s * l * 0.8, c - f * l * 0.7 - s * l * 0.8, col);
                let w = tw(a, F::Body, &d.text, r.font_size);
                // the label sits on the inner side of the arrow, kept on the screen
                let v = view();
                let lx = (c.x - f.x * (l + w * 0.5 + 2.0)).clamp(2.0, (v.x - w - 2.0).max(2.0) + w * 0.5) - w * 0.5;
                let ly = c.y - f.y * (l + 4.0) - lh(F::Body, r.font_size) * 0.5;
                label_outline(a, F::Body, &d.text, lx.max(1.0), ly, r.font_size, col, ink, r.outline);
            }
        }
    }
}

/// One waypoint to draw in the world: where its owner stands, where the waypoint is, and the owner's colour.
#[derive(Clone, Debug, PartialEq)]
pub struct WaypointDraw {
    pub owner: String,
    pub from: Vec3,
    pub to: Vec3,
    pub colour: Color,
}

/// A chevron on the screen: its point and the angle it points (toward the waypoint on the screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChevronMark {
    pub x: f32,
    pub y: f32,
    pub angle: f32,
}

/// The screen chevrons of one waypoint: the world points of [`crate::game::waypoint::chevron_points`] that are in front of the
/// camera and on the screen, each turned toward the next point along the way.
pub fn waypoint_chevrons(w: &WaypointDraw, view_proj: Mat4, screen: (f32, f32), r: &Rules) -> Vec<ChevronMark> {
    let pts = crate::game::waypoint::chevron_points(w.from.to_array(), w.to.to_array(), &r.waypoint);
    let step = r.waypoint.chevron_spacing * 0.5;
    let dir = (w.to - w.from).normalize_or_zero();
    pts.iter()
        .filter_map(|p| {
            let here = vec3(p[0], p[1], p[2]);
            let (x, y) = markers::project(view_proj, here, screen, 0.0)?;
            let (nx, ny) = markers::project(view_proj, here + dir * step, screen, 0.5)?;
            Some(ChevronMark { x, y, angle: (ny - y).atan2(nx - x) })
        })
        .collect()
}

/// The waypoints pass: chevrons along the way and the label over each waypoint (or an edge arrow when it is off screen).
/// Call it only while the TEAMMATE WAYPOINTS setting is on.
pub fn draw_waypoints(g: &Game, a: &Assets, rt: &RenderTarget, items: &[WaypointDraw]) {
    let r = rules();
    let cam = scene::camera(g, rt);
    let vp = cam.matrix();
    let screen = (screen_width(), screen_height());
    let u = unit();
    for w in items {
        let ink = Color::new(0.01, 0.04, 0.05, 0.9);
        for c in waypoint_chevrons(w, vp, screen, r) {
            let at = vec2(c.x / u, c.y / u);
            let f = vec2(c.angle.cos(), c.angle.sin());
            let s = vec2(-f.y, f.x);
            let l = 2.2;
            tri(at + f * l, at - f * l * 0.6 + s * l * 0.8, at - f * l * 0.6 - s * l * 0.8, alpha(w.colour, 0.85));
        }
        let text = if w.owner.is_empty() { r.waypoint.label.clone() } else { format!("{} {}", w.owner.to_uppercase(), r.waypoint.label) };
        let dist = (w.to - cam.position).length();
        let word = status_text(Status::Ok, dist, r);
        let text = if word.is_empty() { text } else { format!("{text} {word}") };
        match place(vp, w.to, screen, r.edge_margin) {
            Some(Placement::OnScreen { x, y }) => {
                let left = x / u - tw(a, F::Body, &text, r.font_size) * 0.5;
                label_outline(a, F::Body, &text, left, y / u - lh(F::Body, r.font_size) * 0.5, r.font_size, w.colour, ink, r.outline);
            }
            Some(Placement::Edge { x, y, angle }) => {
                let c = vec2(x / u, y / u);
                let f = vec2(angle.cos(), angle.sin());
                let s = vec2(-f.y, f.x);
                let l = r.edge_arrow_size;
                tri(c + f * l, c - f * l * 0.7 + s * l * 0.8, c - f * l * 0.7 - s * l * 0.8, w.colour);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Rules {
        Rules::parse(embedded::data_under(FILE).next().expect("teammate_markers.json is embedded").1).expect("parses")
    }

    fn view_proj() -> Mat4 {
        Mat4::perspective_rh_gl(70f32.to_radians(), 16.0 / 9.0, 0.05, 500.0) * Mat4::look_at_rh(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y)
    }

    #[test]
    fn the_shipped_file_parses_and_matches_the_defaults() {
        assert_eq!(shipped(), Rules::default());
    }

    #[test]
    fn a_wrong_schema_is_refused_and_a_missing_field_keeps_its_default() {
        assert!(Rules::parse("{\"schema\":\"x\"}").is_err());
        let r = Rules::parse("{\"schema\":\"lr.teammate_markers/1\",\"gone_seconds\":9}").expect("parses");
        assert_eq!(r.gone_s, 9.0);
        assert_eq!(r.down_label, Rules::default().down_label);
    }

    #[test]
    fn a_changed_data_value_changes_the_behaviour() {
        let mut r = shipped();
        let before = stale_alpha(5.0, &r);
        r.stale_min_alpha = 0.1;
        assert!(stale_alpha(5.0, &r) < before);
        r.down_label = "HURT".into();
        assert_eq!(status_text(Status::Down, 3.0, &r), "HURT");
    }

    #[test]
    fn a_point_ahead_projects_onto_the_screen_and_one_behind_goes_to_the_edge() {
        let screen = (1600.0, 900.0);
        match place(view_proj(), vec3(0.0, 0.0, -10.0), screen, 28.0) {
            Some(Placement::OnScreen { x, y }) => {
                assert!((x - 800.0).abs() < 1.0 && (y - 450.0).abs() < 1.0);
            }
            other => panic!("expected on screen, got {other:?}"),
        }
        match place(view_proj(), vec3(0.0, 0.0, 10.0), screen, 28.0) {
            Some(Placement::Edge { y, .. }) => assert!(y >= 450.0, "straight behind points down"),
            other => panic!("expected an edge arrow, got {other:?}"),
        }
    }

    #[test]
    fn edge_arrows_point_toward_the_target_and_stay_inside_the_margin() {
        let screen = (1600.0, 900.0);
        let (x, y, ang) = edge_point((1.0, 0.0), screen, 28.0);
        assert!((x - 1572.0).abs() < 0.01 && (y - 450.0).abs() < 0.01 && ang.abs() < 1e-4, "right edge, pointing right");
        let (x, y, ang) = edge_point((0.0, 1.0), screen, 28.0);
        assert!((x - 800.0).abs() < 0.01 && (y - 28.0).abs() < 0.01, "top edge");
        assert!((ang + std::f32::consts::FRAC_PI_2).abs() < 1e-4, "up on the screen is minus a quarter turn");
        let (x, y, _) = edge_point((3.0, -2.0), screen, 28.0);
        assert!(x >= 28.0 && x <= 1572.0 && y >= 28.0 && y <= 872.0);
        let (x, y, _) = edge_point((0.0, 0.0), screen, 28.0);
        assert!((x - 800.0).abs() < 0.01 && y > 450.0, "no direction points down");
    }

    #[test]
    fn a_sample_dims_after_the_data_age_and_goes_when_gone() {
        let r = shipped();
        assert_eq!(stale_alpha(0.0, &r), 1.0);
        assert_eq!(stale_alpha(r.stale_after_s, &r), 1.0);
        let mid = stale_alpha((r.stale_after_s + r.stale_full_s) * 0.5, &r);
        assert!(mid < 1.0 && mid > r.stale_min_alpha);
        assert_eq!(stale_alpha(r.stale_full_s + 1.0, &r), r.stale_min_alpha);
        assert_eq!(stale_alpha(r.gone_s, &r), 0.0);
        assert_eq!(stale_alpha(f32::NAN, &r), 0.0);
    }

    #[test]
    fn other_floor_pins_are_dimmed_not_dropped_and_stale_ones_dim_further() {
        let r = shipped();
        assert_eq!(pin_strength(0, 0, false, 0.0, &r), Some(1.0));
        let other = pin_strength(-1, 0, false, 0.0, &r).expect("dimmed, not dropped, on the minimap");
        assert!(other > 0.0 && other < 1.0);
        let stale = pin_strength(-1, 0, false, r.stale_full_s, &r).expect("still there");
        assert!(stale < other);
        assert_eq!(pin_strength(0, 0, true, r.gone_s + 1.0, &r), None);
        let mut off = r.clone();
        off.other_floor_alpha_minimap = 0.0;
        assert_eq!(pin_strength(-1, 0, false, 0.0, &off), None, "data can still hide them");
    }

    #[test]
    fn the_label_names_the_distance_or_the_state() {
        let r = shipped();
        assert_eq!(status_text(Status::Ok, 12.4, &r), "12M");
        assert_eq!(status_text(Status::Down, 12.4, &r), "DOWN");
        assert_eq!(status_text(Status::Caught, 12.4, &r), "CAUGHT");
        let mut quiet = r.clone();
        quiet.show_distance = false;
        assert_eq!(status_text(Status::Ok, 12.4, &quiet), "");
    }

    #[test]
    fn layout_drops_far_and_gone_teammates_and_orders_far_first() {
        let r = shipped();
        let e = |name: &str, z: f32, age: f32| Entry { name: name.into(), at: vec3(0.0, 0.0, z), status: Status::Ok, age_s: age, colour: WHITE };
        let list = vec![e("near", -5.0, 0.0), e("far", -50.0, 0.0), e("gone", -5.0, 99.0), e("away", -(r.max_distance + 10.0), 0.0)];
        let out = layout(&list, Vec3::ZERO, view_proj(), (1600.0, 900.0), &r);
        assert_eq!(out.len(), 2);
        assert!(out[0].text.starts_with("FAR") && out[1].text.starts_with("NEAR"));
        assert_eq!(out[1].text, "NEAR 5M");
    }

    #[test]
    fn waypoint_chevrons_lead_from_the_owner_toward_the_waypoint_on_screen() {
        let r = shipped();
        let w = WaypointDraw { owner: "ANN".into(), from: vec3(0.0, 0.0, -2.0), to: vec3(0.0, 0.0, -30.0), colour: WHITE };
        let marks = waypoint_chevrons(&w, view_proj(), (1600.0, 900.0), &r);
        assert!(!marks.is_empty());
        assert!(marks.windows(2).all(|m| m[1].y <= m[0].y + 0.01), "farther chevrons sit nearer the horizon");
        let behind = WaypointDraw { owner: String::new(), from: vec3(0.0, 0.0, 2.0), to: vec3(0.0, 0.0, 30.0), colour: WHITE };
        assert!(waypoint_chevrons(&behind, view_proj(), (1600.0, 900.0), &r).is_empty(), "behind the camera draws none");
    }

    #[test]
    fn the_down_ring_pulses_and_holds_still_with_reduced_motion() {
        let r = shipped();
        let (r0, a0) = down_ring(0.0, false, &r);
        let (r1, a1) = down_ring(0.4 / r.down_ring_hz, false, &r);
        assert!(r1 > r0 && a1 < a0);
        assert_eq!(down_ring(0.1, true, &r), down_ring(0.9, true, &r));
    }
}
