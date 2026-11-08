//! HUNTER VIEW (admin HUNTERS tab) and the debug status helpers around it: the toggle and the
//! selected-hunter dropdown (built from the live hunter list, so an admin-spawned hunter is in it),
//! the state / target / last-sighting / path-cost readout, the sight cone and hearing circle, blocked
//! cells with the path fail reason, click-to-pick in 3D, and the owner moderation queue. Pure logic:
//! the minimap drawing is ui/map/hunter_view.rs and the 3D drawing is render/hunter_view_overlay.rs.
//! Every number is in data/admin/menu.json under "hunter_view".

use crate::content::embedded;
use crate::world::facility::{Cell, Facility, PathFail, PathStats};
use serde_json::Value;
use std::sync::OnceLock;

/// The tunables (data/admin/menu.json, "hunter_view").
#[derive(Clone, Debug, PartialEq)]
pub struct HunterViewCfg {
    /// Metres from a click ray to a hunter's centre within which it is picked.
    pub pick_radius: f32,
    /// Metres of hearing for a hunter whose `hearing` stat is 1 and whose scale is 1.
    pub hearing_base: f32,
    /// Half angle of the drawn sight cone, in degrees.
    pub sight_half_angle_deg: f32,
    /// Points on the cone's arc (the hearing circle uses twice as many).
    pub arc_points: usize,
    /// Cells around the hunter that are checked for the blocked-cell tint.
    pub blocked_radius: i32,
    /// Seconds the admin find-player marker (beam and rings) lasts.
    pub pulse_secs: f32,
    pub pulse_ring_max: f32,
    pub pulse_beam_height: f32,
    /// The longest a moderation queue shows.
    pub queue_max: usize,
}

/// The parsed "hunter_view" section of the embedded menu.json.
pub fn cfg() -> &'static HunterViewCfg {
    static CFG: OnceLock<HunterViewCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let (path, text) = embedded::data_under("admin/menu.json")
            .next()
            .expect("data/admin/menu.json is embedded");
        let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{path}: {e}"));
        parse_cfg(&v).unwrap_or_else(|e| panic!("{path}: hunter_view: {e}"))
    })
}

pub fn parse_cfg(v: &Value) -> Result<HunterViewCfg, String> {
    let h = v.get("hunter_view").ok_or("missing hunter_view")?;
    let num = |k: &str| -> Result<f32, String> {
        h.get(k)
            .and_then(Value::as_f64)
            .filter(|n| *n > 0.0)
            .map(|n| n as f32)
            .ok_or_else(|| format!("{k} must be a number above 0"))
    };
    Ok(HunterViewCfg {
        pick_radius: num("pick_radius")?,
        hearing_base: num("hearing_base")?,
        sight_half_angle_deg: num("sight_half_angle_deg")?,
        arc_points: num("arc_points")? as usize,
        blocked_radius: num("blocked_radius")? as i32,
        pulse_secs: num("pulse_secs")?,
        pulse_ring_max: num("pulse_ring_max")?,
        pulse_beam_height: num("pulse_beam_height")?,
        queue_max: num("queue_max")? as usize,
    })
}

/// One live hunter as the dropdown and the overlays see it.
#[derive(Clone, Debug, PartialEq)]
pub struct HunterSnapshot {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub pos: (f32, f32),
    pub facing: (f32, f32),
    /// Upper-case state name (State::name).
    pub state: String,
    /// Where it is heading: the search or wander target, else the end of its route.
    pub target: Option<(f32, f32)>,
    pub last_sighting: Option<(f32, f32)>,
    pub path: Vec<Cell>,
    pub sight: f32,
    pub hearing: f32,
    pub admin_spawned: bool,
}

impl HunterSnapshot {
    /// Reads a hunter. `path` is `Brain::path_cells()` (the hook in hunters/brain.rs), `spawned` whether the
    /// admin summoned it.
    pub fn from_brain(index: usize, b: &crate::hunters::brain::Brain, path: &[Cell], spawned: bool) -> HunterSnapshot {
        HunterSnapshot {
            index,
            id: b.def.id.clone(),
            name: b.def.name.to_uppercase(),
            pos: b.pos,
            facing: b.facing,
            state: b.state.name().to_string(),
            target: b
                .search_target
                .or(b.wander_target)
                .or_else(|| path.last().map(|c| (c.x as f32 + 0.5, c.y as f32 + 0.5))),
            last_sighting: b.last_known,
            path: path.to_vec(),
            sight: b.def.sight,
            hearing: cfg().hearing_base * b.def.hearing * b.hearing_scale,
            admin_spawned: spawned,
        }
    }
}

/// What HUNTER VIEW draws this frame: the followed hunter (None when the view is off or none is picked) and the DEBUG
/// VIEW PATHS and NAV switches. The admin publishes it once a frame (`Admin::after_update`); the maps
/// (ui/map/hunter_view.rs) and the 3D pass (render/hunter_view_overlay.rs) read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub view: Option<HunterSnapshot>,
    pub paths: bool,
    pub nav: bool,
}

thread_local! {
    static FRAME: std::cell::RefCell<Frame> = std::cell::RefCell::new(Frame::default());
}

/// Replaces what HUNTER VIEW draws this frame.
pub fn set_frame(frame: Frame) {
    FRAME.with(|f| *f.borrow_mut() = frame);
}

/// Reads what HUNTER VIEW draws this frame.
pub fn with_frame<R>(read: impl FnOnce(&Frame) -> R) -> R {
    FRAME.with(|f| read(&f.borrow()))
}

/// The HUNTER VIEW dropdown rows from the live hunters (name, admin-spawned), as (hunter index, label). An
/// admin-spawned hunter is marked. Rebuilt from the live list, so a hunter summoned or removed is in or out at once.
pub fn dropdown_entries(live: &[(String, bool)]) -> Vec<(usize, String)> {
    live.iter()
        .enumerate()
        .map(|(i, (name, spawned))| {
            let tag = if *spawned { " (ADMIN)" } else { "" };
            (i, format!("{} #{}{}", name.to_uppercase(), i + 1, tag))
        })
        .collect()
}

/// The live hunters of a game as dropdown input; `summoned` are the ids the admin spawned.
pub fn live_list(g: &crate::game::Game, summoned: &[String]) -> Vec<(String, bool)> {
    g.hunters.iter().map(|b| (b.def.name.clone(), summoned.contains(&b.def.id))).collect()
}

/// The HUNTER VIEW state kept by the admin menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HunterView {
    pub on: bool,
    /// Index into the live hunter list; None until one is picked.
    pub selected: Option<usize>,
}

impl HunterView {
    /// Keeps the selection valid when the live list shrinks (a removed hunter).
    pub fn clamp(&mut self, live_len: usize) {
        if self.selected.is_some_and(|i| i >= live_len) {
            self.selected = if live_len == 0 { None } else { Some(live_len - 1) };
        }
    }

    /// A 3D click: selects the hunter nearest the ray within the pick radius.
    pub fn pick(&mut self, origin: [f32; 3], dir: [f32; 3], hunters: &[(usize, [f32; 3])]) -> bool {
        match pick_nearest(origin, dir, hunters, cfg().pick_radius) {
            Some(i) => {
                self.selected = Some(i);
                true
            }
            None => false,
        }
    }
}

/// The hunter whose centre is nearest the ray (origin, direction) and within `radius` of it; ties go to the
/// one nearer the origin. `hunters` pairs a hunter index with its centre.
pub fn pick_nearest(origin: [f32; 3], dir: [f32; 3], hunters: &[(usize, [f32; 3])], radius: f32) -> Option<usize> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-6 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let mut best: Option<(f32, f32, usize)> = None;
    for (i, c) in hunters {
        let rel = [c[0] - origin[0], c[1] - origin[1], c[2] - origin[2]];
        let t = rel[0] * d[0] + rel[1] * d[1] + rel[2] * d[2];
        if t < 0.0 {
            continue;
        }
        let q = [rel[0] - d[0] * t, rel[1] - d[1] * t, rel[2] - d[2] * t];
        let dist = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
        if dist > radius {
            continue;
        }
        if best.map_or(true, |(bd, bt, _)| dist < bd - 1e-4 || ((dist - bd).abs() <= 1e-4 && t < bt)) {
            best = Some((dist, t, *i));
        }
    }
    best.map(|b| b.2)
}

/// The readout lines under the dropdown: state, target, last sighting, then the path cost or the fail reason.
pub fn readout_lines(s: &HunterSnapshot, stats: Option<&PathStats>) -> Vec<String> {
    let at = |p: Option<(f32, f32)>| match p {
        Some((x, z)) => format!("{x:.1}, {z:.1}"),
        None => "NONE".to_string(),
    };
    let mut out = vec![
        format!("STATE {}", s.state),
        format!("TARGET {}", at(s.target)),
        format!("LAST SIGHTING {}", at(s.last_sighting)),
    ];
    out.push(match stats {
        None => "PATH NOT CHECKED".to_string(),
        Some(p) => match p.fail {
            Some(f) => format!("NO PATH // {}", f.label()),
            None => format!(
                "PATH {} CELLS  COST {:.1}  NODES {}  {:.2} MS",
                p.length,
                p.cost,
                p.nodes_searched,
                p.elapsed_us as f32 / 1000.0
            ),
        },
    });
    out
}

/// The path stats from a hunter's cell to its target cell (None without a target).
pub fn stats_for(fac: &Facility, s: &HunterSnapshot) -> Option<PathStats> {
    let (tx, tz) = s.target?;
    let from = Cell::new(s.pos.0.floor() as i32, s.pos.1.floor() as i32);
    let to = Cell::new(tx.floor() as i32, tz.floor() as i32);
    Some(fac.find_path_stats(from, to))
}

/// Cells within the data radius of `centre` that a hunter cannot enter (tinted on the overlay).
pub fn blocked_cells(fac: &Facility, centre: Cell) -> Vec<Cell> {
    let r = cfg().blocked_radius;
    let mut out = vec![];
    for dy in -r..=r {
        for dx in -r..=r {
            let c = Cell::new(centre.x + dx, centre.y + dy);
            if c.x >= 0 && c.y >= 0 && c.x < fac.cols && c.y < fac.rows && !fac.hunter_open(c) {
                out.push(c);
            }
        }
    }
    out
}

/// The sight cone's outline in the floor plane: the apex, then `arc_points` points along the arc.
pub fn sight_cone(pos: (f32, f32), facing: (f32, f32), range: f32) -> Vec<(f32, f32)> {
    let c = cfg();
    let base = facing.1.atan2(facing.0);
    let half = c.sight_half_angle_deg.to_radians();
    let n = c.arc_points.max(2);
    let mut v = vec![pos];
    for i in 0..n {
        let a = base - half + 2.0 * half * i as f32 / (n - 1) as f32;
        v.push((pos.0 + a.cos() * range, pos.1 + a.sin() * range));
    }
    v
}

/// The hearing circle's outline.
pub fn hearing_circle(pos: (f32, f32), radius: f32) -> Vec<(f32, f32)> {
    let n = cfg().arc_points.max(8) * 2;
    (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            (pos.0 + a.cos() * radius, pos.1 + a.sin() * radius)
        })
        .collect()
}

/// The fail reason's label for the overlay caption (empty when the route exists).
pub fn fail_caption(f: Option<PathFail>) -> &'static str {
    f.map_or("", PathFail::label)
}

/// The kind of net state the STATUS page reports (TODO 26 / 321).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetState {
    Solo,
    Hosted,
    Joined,
}

/// The STATUS line for a net state and room code: upper case, and a hosted room is never OFFLINE.
pub fn net_status_text(state: NetState, code: &str) -> String {
    match state {
        NetState::Solo => "SOLO // NO ROOM".to_string(),
        NetState::Hosted => format!("HOSTING ROOM {}", code.to_uppercase()),
        NetState::Joined => format!("JOINED ROOM {}", code.to_uppercase()),
    }
}

/// One flagged item awaiting the owner's decision (TODO 877).
#[derive(Clone, Debug, PartialEq)]
pub struct Flag {
    pub id: String,
    pub who: String,
    pub reason: String,
}

/// What the owner decided; the app turns it into a net/api.rs call (hook for the moderation endpoint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Approve(String),
    Dismiss(String),
}

/// The owner moderation review queue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModQueue {
    pub flags: Vec<Flag>,
    pub decided: Vec<Verdict>,
}

impl ModQueue {
    /// Replaces the list with the server's flags, cut to the data maximum.
    pub fn set(&mut self, mut flags: Vec<Flag>) {
        flags.truncate(cfg().queue_max);
        self.flags = flags;
    }

    fn decide(&mut self, id: &str, approve: bool) -> Option<Verdict> {
        let i = self.flags.iter().position(|f| f.id == id)?;
        self.flags.remove(i);
        let v = if approve { Verdict::Approve(id.to_string()) } else { Verdict::Dismiss(id.to_string()) };
        self.decided.push(v.clone());
        Some(v)
    }

    pub fn approve(&mut self, id: &str) -> Option<Verdict> {
        self.decide(id, true)
    }

    pub fn dismiss(&mut self, id: &str) -> Option<Verdict> {
        self.decide(id, false)
    }

    /// The row text for a flag, upper case.
    pub fn row(f: &Flag) -> String {
        format!("{} // {} // {}", f.who.to_uppercase(), f.reason.to_uppercase(), f.id.to_uppercase())
    }
}
