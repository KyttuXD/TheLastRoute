//! The players and bots as the maps and the nametag pass see them. One [`Marker`] per body: remote players come from
//! [`remote_markers`] (their `RemoteView`s), NPC teammates from [`bot_markers`] (`Game::npcs`). The maps draw them as pins
//! (`map::draw_runner_pins`) and [`draw_nametags`] writes each name above its head in screen space, after the 3D figures.
//! Tunables are in `data/render/nametags.json`.

use super::style::*;
use crate::app::assets::Assets;
use crate::content::embedded;
use crate::game::Game;
use crate::net::runner_figure::hidden_for_good;
use crate::net::world_sync::{clean_name, RemoteView};
use crate::render::anim::runner::Pose;
use crate::render::{scene, stack_actors};
use macroquad::camera::Camera as _;
use macroquad::prelude::*;
use std::sync::OnceLock;

const FILE: &str = "render/nametags.json";

/// Who a marker stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerId {
    Peer(u32),
    /// An NPC teammate, by its id within the run.
    Bot(u32),
}

/// One player or bot on the maps and under a nametag.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: MarkerId,
    pub name: String,
    /// The floor position, in the same window frame as `Game::p.pos`.
    pub x: f32,
    pub y: f32,
    /// The absolute height of the feet.
    pub abs_y: f32,
    /// The level that owns `abs_y`.
    pub level: i32,
    pub yaw: f32,
    pub colour: Color,
    pub is_bot: bool,
    /// Hiding in a locker or lying down: on the map, but no nametag.
    pub concealed: bool,
    /// Lying down (caught): the maps ring the pin and the world marker says DOWN.
    pub downed: bool,
    /// Seconds since the last sample of this body (0 for a bot and for a fresh sample): the pin and marker dim with it.
    pub age_s: f32,
}

/// What the nametag file sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub height_m: f32,
    pub fade_start_m: f32,
    pub fade_end_m: f32,
    pub font_size: f32,
    pub outline_width: f32,
    pub screen_margin: f32,
    pub bot_colour: Color,
    pub other_level_alpha_minimap: f32,
    pub other_level_alpha_full: f32,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(|s| s.as_str()) != Some("lr.nametags/1") {
            return Err("schema: expected lr.nametags/1".into());
        }
        let num = |o: &serde_json::Value, key: &str| -> Result<f32, String> {
            o.get(key)
                .and_then(|x| x.as_f64())
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("{key}: expected a number, zero or more"))
        };
        let colour = v
            .get("bot_colour")
            .and_then(|c| c.as_array())
            .filter(|c| c.len() == 3)
            .map(|c| c.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect::<Vec<f32>>())
            .filter(|c| c.iter().all(|x| (0.0..=1.0).contains(x)))
            .ok_or("bot_colour: expected three numbers from 0 to 1")?;
        let map = v.get("map").ok_or("map: missing")?;
        let r = Rules {
            height_m: num(&v, "height_m")?,
            fade_start_m: num(&v, "fade_start_m")?,
            fade_end_m: num(&v, "fade_end_m")?,
            font_size: num(&v, "font_size")?,
            outline_width: num(&v, "outline_width")?,
            screen_margin: num(&v, "screen_margin")?,
            bot_colour: Color::new(colour[0], colour[1], colour[2], 1.0),
            other_level_alpha_minimap: num(map, "other_level_alpha_minimap")?.min(1.0),
            other_level_alpha_full: num(map, "other_level_alpha_full")?.min(1.0),
        };
        if r.fade_end_m <= r.fade_start_m {
            return Err("fade_end_m must be past fade_start_m".into());
        }
        if r.font_size <= 0.0 {
            return Err("font_size: must be above zero".into());
        }
        Ok(r)
    }
}

/// The shipped rules, read once. A bad file prints its error once and the game keeps these defaults.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let Some((path, src)) = embedded::data_under(FILE).next() else { return fallback() };
        Rules::parse(src).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            fallback()
        })
    })
}

fn fallback() -> Rules {
    Rules {
        height_m: 2.1,
        fade_start_m: 8.0,
        fade_end_m: 26.0,
        font_size: 6.5,
        outline_width: 1.4,
        screen_margin: 0.05,
        bot_colour: Color::new(0.56, 0.91, 1.0, 1.0),
        other_level_alpha_minimap: 0.0,
        other_level_alpha_full: 0.35,
    }
}

/// The markers of the remote players, in the order given: the peer's name and colour, its position and height.
pub fn remote_markers(remotes: &[RemoteView]) -> Vec<Marker> {
    remotes
        .iter()
        .map(|r| Marker {
            id: MarkerId::Peer(r.peer),
            name: if r.name.is_empty() { String::new() } else { clean_name(&r.name) },
            x: r.x,
            y: r.y,
            abs_y: r.h,
            level: r.level,
            yaw: r.yaw,
            colour: super::map::teammate_colour(r.peer),
            is_bot: false,
            // only a caught runner is DOWN: one lying on a bed (`Pose::Lie`) or sitting keeps its tag and gets no ring
            concealed: hidden_for_good(r) || r.clip == Some(Pose::Down),
            downed: r.clip == Some(Pose::Down),
            age_s: 0.0,
        })
        .collect()
}

/// Sets the sample age of the peer markers from `(peer, seconds since its last sample)` pairs (the net layer's
/// `remote_ages`), so the pins and world markers dim as a teammate goes quiet. A peer not listed keeps its age.
pub fn with_ages(marks: &mut [Marker], ages: &[(u32, f32)]) {
    for m in marks.iter_mut() {
        if let MarkerId::Peer(p) = m.id {
            if let Some((_, age)) = ages.iter().find(|(peer, _)| *peer == p) {
                m.age_s = if age.is_finite() { age.max(0.0) } else { 0.0 };
            }
        }
    }
}

/// The map direction a body at `yaw` faces on the floor: `Player::forward2`, (-sin, -cos).
pub fn facing_dir(yaw: f32) -> Vec2 {
    vec2(-yaw.sin(), -yaw.cos())
}

/// One bot's marker. `hidden` is a bot inside a locker.
pub fn bot_marker(id: u32, name: &str, at: (f32, f32), abs_y: f32, level: i32, yaw: f32, hidden: bool, colour: Color) -> Marker {
    Marker {
        id: MarkerId::Bot(id),
        name: name.to_string(),
        x: at.0,
        y: at.1,
        abs_y,
        level,
        yaw,
        colour,
        is_bot: true,
        concealed: hidden,
        downed: false,
        age_s: 0.0,
    }
}

/// The markers of the NPC teammates, named by their guest names.
pub fn bot_markers(g: &Game) -> Vec<Marker> {
    let colour = rules().bot_colour;
    g.npcs.iter().map(|n| bot_marker(n.id, &n.name, n.at, n.y, n.level, n.facing, n.hidden.is_some(), colour)).collect()
}

/// Every marker the viewer's frame shows: the remote players, then the bots.
pub fn all_markers(g: &Game, remotes: &[RemoteView]) -> Vec<Marker> {
    let mut out = remote_markers(remotes);
    out.extend(bot_markers(g));
    out
}

/// How strongly a map pin is drawn: whole on the shown level, else the strength the data sets for the minimap or the full
/// map (`data/hud/teammate_markers.json`, dimmed and never dropped); None only when the data sets it to zero.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn pin_alpha(level: i32, shown_level: i32, full: bool) -> Option<f32> {
    super::teammate_markers::pin_strength(level, shown_level, full, 0.0, super::teammate_markers::rules())
}

/// The strength of a nametag `dist` metres from the eye: 1 up to the start of the fade, 0 at its end.
pub fn distance_fade(dist: f32, start: f32, end: f32) -> f32 {
    if dist <= start {
        1.0
    } else if dist >= end {
        0.0
    } else {
        1.0 - (dist - start) / (end - start)
    }
}

/// A world point on the screen, in window pixels (y down). None when it is behind the camera, past the far plane or more
/// than `margin` (a fraction of the screen) outside it.
pub fn project(view_proj: Mat4, p: Vec3, screen: (f32, f32), margin: f32) -> Option<(f32, f32)> {
    if !p.is_finite() {
        return None;
    }
    let clip = view_proj * p.extend(1.0);
    if !clip.is_finite() || clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let lim = 1.0 + margin * 2.0;
    if ndc.x.abs() > lim || ndc.y.abs() > lim || ndc.z > 1.0 {
        return None;
    }
    Some(((ndc.x * 0.5 + 0.5) * screen.0, (0.5 - ndc.y * 0.5) * screen.1))
}

/// Where a nametag goes and how strongly: the screen point above the head and its alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tag {
    pub at: (f32, f32),
    pub alpha: f32,
    pub dist: f32,
}

/// The nametag of `m`, or None when it is hidden: no name, concealed (locker, down), on a level that is not the drawn one
/// (unless it is on a flight), behind the camera, off screen or past the fade. `active_level` is the drawn level, `eye` the
/// camera position and `view_proj` its matrix.
pub fn tag_for(m: &Marker, active_level: i32, eye: Vec3, view_proj: Mat4, screen: (f32, f32), r: &Rules) -> Option<Tag> {
    if m.name.is_empty() || m.concealed {
        return None;
    }
    if m.level != active_level && !stack_actors::on_link(m.abs_y) {
        return None;
    }
    let head = vec3(m.x, stack_actors::actor_dy(m.abs_y, active_level) + r.height_m, m.y);
    let dist = (head - eye).length();
    let alpha = distance_fade(dist, r.fade_start_m, r.fade_end_m);
    if !dist.is_finite() || !alpha.is_finite() || alpha <= 0.0 {
        return None;
    }
    let at = project(view_proj, head, screen, r.screen_margin)?;
    Some(Tag { at, alpha, dist })
}

/// The nametag pass: each name above its head, in its colour with a dark outline, farthest first. Runs on the default
/// camera after the 3D view is on the screen. `rt` is the scene target the figures were drawn through.
pub fn draw_nametags(g: &Game, a: &Assets, rt: &RenderTarget, marks: &[Marker]) {
    if marks.is_empty() {
        return;
    }
    let r = rules();
    let cam = scene::camera(g, rt);
    let vp = cam.matrix();
    let screen = (screen_width(), screen_height());
    let u = unit();
    let mut tags: Vec<(&Marker, Tag)> =
        marks.iter().filter_map(|m| tag_for(m, g.fac.level, cam.position, vp, screen, r).map(|t| (m, t))).collect();
    tags.sort_by(|x, y| y.1.dist.total_cmp(&x.1.dist));
    for (m, t) in tags {
        let name = m.name.to_uppercase();
        let top = t.at.1 / u - lh(F::Body, r.font_size);
        let x = t.at.0 / u - tw(a, F::Body, &name, r.font_size) * 0.5;
        let ink = Color::new(0.01, 0.04, 0.05, 0.95 * t.alpha);
        label_outline(a, F::Body, &name, x, top, r.font_size, alpha(m.colour, t.alpha), ink, r.outline_width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Rules {
        Rules::parse(embedded::data_under(FILE).next().expect("nametags.json is embedded").1).expect("parses")
    }

    fn view_proj() -> Mat4 {
        // eye at the origin looking down -z, like a runner at yaw 0
        Mat4::perspective_rh_gl(70f32.to_radians(), 16.0 / 9.0, 0.05, 200.0)
            * Mat4::look_at_rh(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y)
    }

    fn peer_view(peer: u32, name: &str) -> RemoteView {
        RemoteView {
            peer,
            name: name.into(),
            x: 1.0,
            y: 2.0,
            yaw: 0.5,
            h: -5.0,
            level: -1,
            skin: String::new(),
            moving: false,
            sprinting: false,
            crouching: false,
            slim: false,
            speed: 0.0,
            vy: 0.0,
            jump: 0.0,
            clip: None,
            held: None,
            flash_pitch: None,
            look_pitch: 0.0,
            emote: None,
            hide: None,
            hide_since: None,
        }
    }

    fn marker_at(x: f32, z: f32) -> Marker {
        bot_marker(1, "R-04821", (x, z), 0.0, 0, 0.0, false, Color::new(1.0, 1.0, 1.0, 1.0))
    }

    #[test]
    fn remote_markers_carry_the_peers_name_place_level_and_colour() {
        let mut down = peer_view(2, "Bea");
        down.clip = Some(Pose::Down);
        let ms = remote_markers(&[peer_view(1, "ANN"), down]);
        assert_eq!(ms.len(), 2);
        let m = &ms[0];
        assert_eq!((m.id, m.name.as_str(), m.x, m.y, m.abs_y, m.level, m.yaw), (MarkerId::Peer(1), "ANN", 1.0, 2.0, -5.0, -1, 0.5));
        assert_eq!(m.colour, super::super::map::teammate_colour(1));
        assert!(!m.is_bot && !m.concealed);
        assert!(ms[1].concealed, "a runner lying down gets no tag");
    }

    #[test]
    fn a_bot_marker_is_a_bot_in_the_bot_colour_and_a_hidden_bot_is_concealed() {
        let c = rules().bot_colour;
        let m = bot_marker(7, "R-00007", (3.5, 4.5), -2.5, 0, 1.25, true, c);
        assert_eq!((m.id, m.is_bot, m.concealed, m.colour), (MarkerId::Bot(7), true, true, c));
        assert_eq!((m.x, m.y, m.abs_y, m.level, m.yaw), (3.5, 4.5, -2.5, 0, 1.25));
    }

    #[test]
    fn the_shipped_rules_parse_and_the_fallback_matches_them() {
        assert_eq!(shipped(), fallback());
        assert!(Rules::parse("{}").is_err());
        let bad = embedded::data_under(FILE).next().unwrap().1.replace("\"fade_end_m\": 26.0", "\"fade_end_m\": 4.0");
        assert!(Rules::parse(&bad).is_err(), "the fade must end after it starts");
    }

    #[test]
    fn a_point_behind_the_camera_has_no_screen_position() {
        let vp = view_proj();
        assert_eq!(project(vp, vec3(0.0, 0.0, 5.0), (1280.0, 720.0), 0.05), None);
        let ahead = project(vp, vec3(0.0, 0.0, -5.0), (1280.0, 720.0), 0.05).expect("ahead of the camera");
        assert!((ahead.0 - 640.0).abs() < 0.01 && (ahead.1 - 360.0).abs() < 0.01, "{ahead:?}");
        // far to the side: off the screen
        assert_eq!(project(vp, vec3(50.0, 0.0, -5.0), (1280.0, 720.0), 0.05), None);
        // above the axis lands above the middle of the screen (y grows downward)
        assert!(project(vp, vec3(0.0, 1.0, -5.0), (1280.0, 720.0), 0.05).unwrap().1 < 360.0);
    }

    #[test]
    fn the_tag_fades_with_distance_and_goes_out_past_the_end() {
        assert_eq!(distance_fade(3.0, 8.0, 26.0), 1.0);
        assert!((distance_fade(17.0, 8.0, 26.0) - 0.5).abs() < 1e-6);
        assert_eq!(distance_fade(26.0, 8.0, 26.0), 0.0);
        let r = shipped();
        let (vp, screen) = (view_proj(), (1280.0, 720.0));
        let near = tag_for(&marker_at(0.0, -4.0), 0, Vec3::ZERO, vp, screen, &r).expect("near and ahead");
        assert_eq!(near.alpha, 1.0);
        let mid = tag_for(&marker_at(0.0, -18.0), 0, Vec3::ZERO, vp, screen, &r).expect("in the fade");
        assert!(mid.alpha > 0.0 && mid.alpha < 1.0, "{}", mid.alpha);
        assert_eq!(tag_for(&marker_at(0.0, -60.0), 0, Vec3::ZERO, vp, screen, &r), None, "past the fade");
    }

    #[test]
    fn a_tag_sits_above_the_head_and_is_none_behind_the_camera() {
        let r = shipped();
        let (vp, screen) = (view_proj(), (1280.0, 720.0));
        let t = tag_for(&marker_at(0.0, -6.0), 0, Vec3::ZERO, vp, screen, &r).unwrap();
        let feet = project(vp, vec3(0.0, 0.0, -6.0), screen, 0.05).unwrap();
        assert!(t.at.1 < feet.1, "the tag is above the feet on the screen");
        assert_eq!(tag_for(&marker_at(0.0, 6.0), 0, Vec3::ZERO, vp, screen, &r), None, "behind the camera");
    }

    #[test]
    fn no_tag_for_a_concealed_unnamed_or_other_level_marker() {
        let r = shipped();
        let (vp, screen) = (view_proj(), (1280.0, 720.0));
        let mut m = marker_at(0.0, -4.0);
        assert!(tag_for(&m, 0, Vec3::ZERO, vp, screen, &r).is_some());
        m.concealed = true;
        assert_eq!(tag_for(&m, 0, Vec3::ZERO, vp, screen, &r), None);
        m.concealed = false;
        m.name.clear();
        assert_eq!(tag_for(&m, 0, Vec3::ZERO, vp, screen, &r), None);
        let mut up = marker_at(0.0, -4.0);
        up.level = 1;
        up.abs_y = crate::game::level_stack::floor_y_of(1);
        assert_eq!(tag_for(&up, 0, Vec3::ZERO, vp, screen, &r), None, "standing on another floor");
        up.abs_y = -1.0;
        assert!(tag_for(&up, 0, Vec3::ZERO, vp, screen, &r).is_some(), "on a flight it is in the open");
    }

    #[test]
    fn a_pin_on_another_level_is_dim_on_both_maps_not_dropped() {
        assert_eq!(pin_alpha(0, 0, false), Some(1.0));
        let mini = pin_alpha(-1, 0, false).expect("dimmed on the minimap, not dropped");
        assert!(mini > 0.0 && mini < 1.0);
        let full = pin_alpha(-1, 0, true).expect("dimmed, not hidden");
        assert!(full > 0.0 && full < 1.0);
    }

    #[test]
    fn ages_reach_the_peer_markers_only() {
        let mut ms = remote_markers(&[peer_view(1, "ANN"), peer_view(2, "BEA")]);
        ms.push(marker_at(0.0, 0.0));
        with_ages(&mut ms, &[(2, 6.5), (9, 1.0)]);
        assert_eq!((ms[0].age_s, ms[1].age_s, ms[2].age_s), (0.0, 6.5, 0.0));
    }

    #[test]
    fn a_downed_remote_is_marked_downed_for_the_ring() {
        let mut down = peer_view(2, "Bea");
        down.clip = Some(Pose::Down);
        let ms = remote_markers(&[peer_view(1, "ANN"), down]);
        assert!(!ms[0].downed && ms[1].downed);
    }

    #[test]
    fn a_remote_lying_on_a_bed_or_sitting_is_not_marked_down_and_keeps_its_tag() {
        let (mut asleep, mut seated) = (peer_view(2, "Bea"), peer_view(3, "Cy"));
        asleep.clip = Some(Pose::Lie);
        seated.clip = Some(Pose::Sit);
        let ms = remote_markers(&[asleep, seated]);
        assert!(ms.iter().all(|m| !m.downed && !m.concealed), "resting is not DOWN: only a caught runner is");
    }

    #[test]
    fn remote_marker_names_are_cleaned_and_an_empty_name_stays_empty() {
        let ms = remote_markers(&[peer_view(1, &format!("ann\u{202e}\u{1F600}{}", "z".repeat(40))), peer_view(2, "")]);
        assert_eq!(ms[0].name.len(), 24);
        assert!(ms[0].name.starts_with("ANN") && ms[0].name.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(ms[1].name, "", "no name, no tag");
    }

    #[test]
    fn a_non_finite_position_or_eye_gives_no_tag() {
        let r = shipped();
        let (vp, screen) = (view_proj(), (1280.0, 720.0));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(tag_for(&marker_at(bad, -4.0), 0, Vec3::ZERO, vp, screen, &r), None);
            assert_eq!(tag_for(&marker_at(0.0, bad), 0, Vec3::ZERO, vp, screen, &r), None);
            let mut m = marker_at(0.0, -4.0);
            m.abs_y = bad;
            assert_eq!(tag_for(&m, 0, Vec3::ZERO, vp, screen, &r), None);
            assert_eq!(tag_for(&marker_at(0.0, -4.0), 0, vec3(bad, 0.0, 0.0), vp, screen, &r), None);
            assert_eq!(project(vp, vec3(0.0, bad, -5.0), screen, 0.05), None);
        }
        assert!(distance_fade(f32::NAN, 8.0, 26.0).is_nan(), "tag_for is what rejects it");
    }

    #[test]
    fn a_bot_arrow_points_the_way_the_bot_walks() {
        for want in [(0.0f32, -1.0f32), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.6, -0.8)] {
            let yaw = crate::game::npc::heading_yaw(want);
            let m = bot_marker(1, "R-00001", (0.0, 0.0), 0.0, 0, yaw, false, Color::new(1.0, 1.0, 1.0, 1.0));
            let d = facing_dir(m.yaw);
            assert!((d.x - want.0).abs() < 1e-4 && (d.y - want.1).abs() < 1e-4, "heading {want:?} arrow {d:?}");
        }
    }

    #[test]
    fn a_marker_on_a_flight_in_the_fade_keeps_its_alpha_and_other_levels_are_not_seen() {
        let r = shipped();
        let (vp, screen) = (view_proj(), (1280.0, 720.0));
        let mut m = marker_at(0.0, -18.0);
        m.level = 1;
        m.abs_y = -1.0;
        let t = tag_for(&m, 0, Vec3::ZERO, vp, screen, &r).expect("on a flight");
        assert!(t.alpha > 0.0 && t.alpha < 1.0);
        assert!(t.dist > r.fade_start_m && t.dist < r.fade_end_m);
    }
}
