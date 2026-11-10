//! The Descent's rules: a port of `descent_logic.gd` and `descent_data.gd` (createDescentGame in the 2D game).
//! Pure state machine: no drawing, no audio, no window, no real clock. Everything random comes from one
//! `Mulberry32` seeded from the session seed, drawn in the same order as the GDScript.
//!
//! * wires   boxes light in turn; press the button at the end of each lit box's wire. A wrong button starts the run over
//!           and the wires are dealt again.
//! * memory  four pads light in turn, then you press them back in the same order. A wrong pad starts the show again.
//! * pipes   turn the pieces until one line runs from the left edge, through the grid, to the right edge. She closes in
//!           at half speed here.
//! * hold    (the last stage of the late descents) hold the door shut until the car stops.
//!
//! Her closeness (`threat`, 0..100) rises with time and with every wrong attempt, and falls when a stage is done. At the
//! top of the bar she is in. `step(dt_ms, hold)` and `act(..)` return events.
#![allow(dead_code)]

use crate::core::{jitter, Fnv, Mulberry32};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

pub const TUNABLES_JSON: &str = include_str!("../../data/descent/tunables.json");

// ------------------------------------------------------------------------------------------------ stages

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Wires,
    Memory,
    Pipes,
    Hold,
}

impl Stage {
    pub fn parse(s: &str) -> Option<Stage> {
        match s {
            "wires" => Some(Stage::Wires),
            "memory" => Some(Stage::Memory),
            "pipes" => Some(Stage::Pipes),
            "hold" => Some(Stage::Hold),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Stage::Wires => "wires",
            Stage::Memory => "memory",
            Stage::Pipes => "pipes",
            Stage::Hold => "hold",
        }
    }

    /// STAGE_NAMES of the Godot data.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Wires => "FUSE BOXES",
            Stage::Memory => "SIGNAL PANEL",
            Stage::Pipes => "PIPEWORK",
            Stage::Hold => "DOOR",
        }
    }

    /// The panel title: the name, plus the pipes' note.
    pub fn title(self) -> String {
        if self == Stage::Pipes {
            format!("{} \u{b7} SHE SLOWS HERE", self.name())
        } else {
            self.name().to_string()
        }
    }

    fn order(self) -> u8 {
        match self {
            Stage::Wires => 0,
            Stage::Memory => 1,
            Stage::Pipes => 2,
            Stage::Hold => 3,
        }
    }
}

// ------------------------------------------------------------------------------------------------ tunables

/// Every constant of `descent_logic.gd` and `descent_data.gd`, read from `data/descent/tunables.json` (missing keys keep
/// the Godot value).
#[derive(Clone, Debug)]
pub struct Tun {
    pub rate_per_level: f64,
    pub wrong_bump: f64,
    pub stage_relief: f64,
    pub ride_ms: f64,
    pub show_on_ms: f64,
    pub show_off_ms: f64,
    pub show_lead_ms: f64,
    pub hold_ms: f64,
    pub hold_slip: f64,
    pub hold_rate: f64,
    pub pipe_rate: f64,
    pub surge_spread: f64,
    pub surge_every_ms: f64,
    pub surge_every_spread: f64,
    pub show_pace_spread: f64,
    pub lit_spread: i32,
    pub length_spread: i32,
    pub descents: i32,
    pub elise_levels: Vec<i32>,
    pub runs: Vec<Vec<Stage>>,
    pub wires_lit: Vec<i32>,
    pub wires_buttons: Vec<i32>,
    pub memory_length: Vec<i32>,
    pub pipes_grid: Vec<i32>,
    pub pipes_detours: Vec<i32>,
    pub pipes_turns: Vec<i32>,
}

fn num(v: &Value, key: &str, d: f64) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(d)
}

fn ints(v: &Value, key: &str, d: &[i32]) -> Vec<i32> {
    match v.get(key).and_then(|x| x.as_array()) {
        Some(a) if !a.is_empty() => a.iter().map(|x| x.as_i64().unwrap_or(0) as i32).collect(),
        _ => d.to_vec(),
    }
}

fn default_runs() -> Vec<Vec<Stage>> {
    use Stage::*;
    vec![
        vec![Wires],
        vec![Wires, Memory],
        vec![Wires, Memory, Pipes],
        vec![Wires, Memory, Pipes, Hold],
        vec![Wires, Memory, Pipes, Wires, Hold],
    ]
}

impl Tun {
    pub fn parse(doc: &Value) -> Tun {
        let rules = doc.get("rules").cloned().unwrap_or(Value::Null);
        let data = doc.get("data").cloned().unwrap_or(Value::Null);
        let defaults = default_runs();
        let mut runs: Vec<Vec<Stage>> = Vec::new();
        for i in 1..=5usize {
            let key = i.to_string();
            let parsed: Vec<Stage> = data
                .get("runs")
                .and_then(|r| r.get(key.as_str()))
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().and_then(Stage::parse)).collect())
                .unwrap_or_default();
            runs.push(if parsed.is_empty() { defaults[i - 1].clone() } else { parsed });
        }
        Tun {
            rate_per_level: num(&rules, "rate_per_level", 0.22),
            wrong_bump: num(&rules, "wrong_bump", 13.0),
            stage_relief: num(&rules, "stage_relief", 34.0),
            ride_ms: num(&rules, "ride_ms", 1800.0),
            show_on_ms: num(&rules, "show_on_ms", 520.0),
            show_off_ms: num(&rules, "show_off_ms", 260.0),
            show_lead_ms: num(&rules, "show_lead_ms", 700.0),
            hold_ms: num(&rules, "hold_ms", 3200.0),
            hold_slip: num(&rules, "hold_slip", 22.0),
            hold_rate: num(&rules, "hold_rate", 2.6),
            pipe_rate: num(&rules, "pipe_rate", 0.5),
            surge_spread: num(&rules, "surge_spread", 0.2),
            surge_every_ms: num(&rules, "surge_every_ms", 9000.0),
            surge_every_spread: num(&rules, "surge_every_spread", 0.33),
            show_pace_spread: num(&rules, "show_pace_spread", 0.15),
            lit_spread: num(&rules, "lit_spread", 1.0) as i32,
            length_spread: num(&rules, "length_spread", 1.0) as i32,
            descents: (num(&data, "descents", 5.0) as i32).clamp(1, 5),
            elise_levels: ints(&data, "elise_levels", &[0, 5, 8, 11, 14]),
            runs,
            wires_lit: ints(&data, "wires_lit", &[3, 3, 4, 4, 5]),
            wires_buttons: ints(&data, "wires_buttons", &[4, 4, 5, 6, 6]),
            memory_length: ints(&data, "memory_length", &[3, 3, 4, 4, 5]),
            pipes_grid: ints(&data, "pipes_grid", &[4, 4, 4, 5, 5]),
            pipes_detours: ints(&data, "pipes_detours", &[0, 0, 0, 1, 1]),
            pipes_turns: ints(&data, "pipes_turns", &[2, 2, 2, 2, 2]),
        }
    }

    pub fn get() -> &'static Tun {
        static TUN: OnceLock<Tun> = OnceLock::new();
        TUN.get_or_init(|| Tun::parse(doc()))
    }

    /// A table entry for descent `d` (1-based); the last entry when the table is short.
    fn at(table: &[i32], d: i32) -> i32 {
        let i = (d.max(1) - 1) as usize;
        table.get(i).or_else(|| table.last()).copied().unwrap_or(0)
    }
}

/// The whole tunables document (the map module reads its `view` section).
pub fn doc() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(TUNABLES_JSON).unwrap_or(Value::Null))
}

// ------------------------------------------------------------------------------------------------ puzzles

/// N, E, S, W as 0 to 3: the step (row, column) of each side.
pub const STEP: [[i32; 2]; 4] = [[-1, 0], [0, 1], [1, 0], [0, -1]];
/// A straight piece has two ends on opposite sides, a corner two at right angles (OPEN of the GDScript).
static OPEN_I: [[usize; 2]; 2] = [[3, 1], [0, 2]];
static OPEN_L: [[usize; 2]; 4] = [[0, 1], [1, 2], [2, 3], [3, 0]];

#[derive(Clone, Debug)]
pub struct Wires {
    pub boxes: usize,
    pub buttons: usize,
    /// Which button each box's wire ends at.
    pub wire: Vec<usize>,
    /// The boxes that light, in order.
    pub lit: Vec<usize>,
    pub colors: Vec<usize>,
    pub step: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Show,
    Input,
}

#[derive(Clone, Debug)]
pub struct Memory {
    pub seq: Vec<usize>,
    pub pace: f64,
    pub hues: Vec<usize>,
    pub step: usize,
    pub phase: Phase,
    pub t: f64,
    /// The pad that is lit now, -1 for none.
    pub lit: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PipeKind {
    I,
    L,
}

impl PipeKind {
    pub fn opens(self) -> &'static [[usize; 2]] {
        match self {
            PipeKind::I => &OPEN_I,
            PipeKind::L => &OPEN_L,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub kind: PipeKind,
    pub rot: usize,
    /// The rotation that joins the path (for tiles on the hidden path).
    pub solved: usize,
}

impl Tile {
    pub fn openings(&self) -> [usize; 2] {
        let o = self.kind.opens();
        o[self.rot % o.len()]
    }
}

#[derive(Clone, Debug)]
pub struct Pipes {
    pub n: usize,
    pub tiles: Vec<Vec<Tile>>,
    pub path: Vec<(usize, usize)>,
}

impl Pipes {
    /// Cells the water reaches from the west edge of (0,0), and whether it comes out of the east edge of the last cell.
    pub fn flow(&self) -> (Vec<(usize, usize)>, bool) {
        let n = self.n as i32;
        let (mut r, mut c, mut enter) = (0i32, 0i32, 3usize);
        let mut cells = Vec::new();
        let mut done = false;
        for _ in 0..(self.n * self.n + 2) {
            let open = self.tiles[r as usize][c as usize].openings();
            if !open.contains(&enter) {
                break;
            }
            cells.push((r as usize, c as usize));
            let out = if open[0] == enter { open[1] } else { open[0] };
            if r == n - 1 && c == n - 1 && out == 1 {
                done = true;
                break;
            }
            r += STEP[out][0];
            c += STEP[out][1];
            if r < 0 || c < 0 || r >= n || c >= n {
                break;
            }
            enter = (out + 2) % 4;
        }
        (cells, done)
    }

    pub fn connected(&self) -> bool {
        self.flow().1
    }

    /// Turn one piece; true when the line now runs through.
    pub fn turn(&mut self, r: usize, c: usize) -> bool {
        if r < self.n && c < self.n {
            let t = &mut self.tiles[r][c];
            let m = t.kind.opens().len();
            t.rot = (t.rot + 1) % m;
        }
        self.connected()
    }
}

#[derive(Clone, Debug)]
pub enum Puzzle {
    None,
    Wires(Wires),
    Memory(Memory),
    Pipes(Pipes),
    Hold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Press {
    Ok,
    Done,
    Wrong,
    /// A pad pressed while the show runs.
    Nothing,
}

fn draw_below(rng: &mut Mulberry32, n: usize) -> usize {
    ((rng.next() * n as f64).floor() as usize).min(n.saturating_sub(1))
}

/// Deal the wires: which button each box's wire ends at.
pub fn deal(buttons: usize, boxes: usize, rng: &mut Mulberry32) -> Vec<usize> {
    let mut v: Vec<usize> = (0..buttons).collect();
    rng.shuffle(&mut v);
    v.truncate(boxes);
    v
}

pub fn draw_lit(base: i32, spread: i32, rng: &mut Mulberry32) -> i32 {
    (base + (rng.next() * (2 * spread + 1) as f64).floor() as i32 - spread).max(2)
}

pub fn draw_length(base: i32, spread: i32, rng: &mut Mulberry32) -> i32 {
    (base + (rng.next() * (2 * spread + 1) as f64).floor() as i32 - spread).max(3)
}

/// The order of a descent's stages: the listed stages in a drawn order, the door always last.
pub fn draw_stages(descent: i32, rng: &mut Mulberry32) -> Vec<Stage> {
    let tun = Tun::get();
    let list = &tun.runs[(descent.clamp(1, 5) - 1) as usize];
    let mut rest: Vec<Stage> = list.iter().copied().filter(|s| *s != Stage::Hold).collect();
    rng.shuffle(&mut rest);
    if list.contains(&Stage::Hold) {
        rest.push(Stage::Hold);
    }
    rest
}

pub fn valid_order(descent: i32, list: &[Stage]) -> bool {
    let want = &Tun::get().runs[(descent.clamp(1, 5) - 1) as usize];
    if list.len() != want.len() {
        return false;
    }
    if want.contains(&Stage::Hold) && list.last() != Some(&Stage::Hold) {
        return false;
    }
    let mut a: Vec<u8> = list.iter().map(|s| s.order()).collect();
    let mut b: Vec<u8> = want.iter().map(|s| s.order()).collect();
    a.sort();
    b.sort();
    a == b
}

pub fn make_wires(n: usize, buttons: usize, lit: usize, rng: &mut Mulberry32) -> Wires {
    let wire = deal(buttons, n, rng);
    let mut order: Vec<usize> = (0..n).collect();
    rng.shuffle(&mut order);
    order.truncate(lit);
    let mut colors: Vec<usize> = (0..6).collect();
    rng.shuffle(&mut colors);
    colors.truncate(n);
    Wires { boxes: n, buttons, wire, lit: order, colors, step: 0 }
}

/// A wrong press starts over and the wires are dealt again.
pub fn press_wire(z: &mut Wires, button: usize, rng: &mut Mulberry32) -> Press {
    let target = z.lit.get(z.step).and_then(|b| z.wire.get(*b)).copied();
    if target == Some(button) {
        z.step += 1;
        if z.step == z.lit.len() {
            Press::Done
        } else {
            Press::Ok
        }
    } else {
        z.step = 0;
        z.wire = deal(z.buttons, z.boxes, rng);
        Press::Wrong
    }
}

pub fn make_memory(length: usize, rng: &mut Mulberry32, tun: &Tun) -> Memory {
    let mut seq = Vec::with_capacity(length);
    for _ in 0..length {
        seq.push(draw_below(rng, 4));
    }
    let pace = jitter(1.0, tun.show_pace_spread, rng);
    let mut hues: Vec<usize> = vec![0, 1, 2, 3];
    rng.shuffle(&mut hues);
    Memory { seq, pace, hues, step: 0, phase: Phase::Show, t: -tun.show_lead_ms, lit: -1 }
}

impl Memory {
    /// Run the show for `dt_ms`: each pad lights for 520 ms with 260 ms between, scaled by `pace`.
    pub fn advance_show(&mut self, dt_ms: f64, tun: &Tun) {
        if self.phase != Phase::Show {
            return;
        }
        self.t += dt_ms;
        let on_ms = tun.show_on_ms * self.pace;
        let cycle = (tun.show_on_ms + tun.show_off_ms) * self.pace;
        let t = self.t;
        let k = (t / cycle).floor();
        if t < 0.0 {
            self.lit = -1;
        } else if k >= self.seq.len() as f64 {
            self.phase = Phase::Input;
            self.lit = -1;
            self.step = 0;
        } else {
            self.lit = if t.rem_euclid(cycle) < on_ms { self.seq[k as usize] as i32 } else { -1 };
        }
    }

    /// A press during the show does nothing. In the input phase: ok, done when the sequence is complete, wrong (the
    /// show restarts).
    pub fn press(&mut self, pad: usize, tun: &Tun) -> Press {
        if self.phase != Phase::Input {
            return Press::Nothing;
        }
        if self.seq.get(self.step) == Some(&pad) {
            self.step += 1;
            if self.step == self.seq.len() {
                Press::Done
            } else {
                Press::Ok
            }
        } else {
            self.phase = Phase::Show;
            self.t = -tun.show_lead_ms * 1.3;
            self.step = 0;
            self.lit = -1;
            Press::Wrong
        }
    }
}

fn step_index(a: (i32, i32), b: (i32, i32)) -> Option<usize> {
    (0..4usize).find(|d| a.0 + STEP[*d][0] == b.0 && a.1 + STEP[*d][1] == b.1)
}

/// `detours` caps the steps the hidden path may take up or left; `turns` caps the clicks any of its pieces needs.
pub fn make_pipes(n: usize, rng: &mut Mulberry32, detours: i32, turns: i32) -> Pipes {
    let ni = n as i32;
    let mut path: Vec<(i32, i32)> = Vec::new();
    let mut attempt = 0;
    while attempt < 80 && path.is_empty() {
        attempt += 1;
        let mut walk: Vec<(i32, i32)> = vec![(0, 0)];
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        seen.insert((0, 0));
        let mut back = 0;
        while walk.len() < n * n {
            let (r, c) = walk.last().copied().unwrap_or((0, 0));
            if r == ni - 1 && c == ni - 1 {
                path = walk.clone();
                break;
            }
            let mut options: Vec<(i32, i32, usize)> = Vec::new();
            for d in 0..4usize {
                let nr = r + STEP[d][0];
                let nc = c + STEP[d][1];
                if nr < 0 || nc < 0 || nr >= ni || nc >= ni || seen.contains(&(nr, nc)) {
                    continue;
                }
                if (d == 0 || d == 3) && back >= detours {
                    continue;
                }
                let copies = if d == 1 || d == 2 { 3 } else { 1 };
                for _ in 0..copies {
                    options.push((nr, nc, d));
                }
            }
            if options.is_empty() {
                break;
            }
            let (nr, nc, d) = options[draw_below(rng, options.len())];
            if d == 0 || d == 3 {
                back += 1;
            }
            walk.push((nr, nc));
            seen.insert((nr, nc));
        }
        if path.is_empty() {
            let last = walk.last().copied();
            if let Some((r, c)) = last {
                if r == ni - 1 && c == ni - 1 {
                    path = walk;
                }
            }
        }
    }
    if path.is_empty() {
        path.push((0, 0));
        for k in 1..ni {
            path.push((0, k));
        }
        for k in 1..ni {
            path.push((k, ni - 1));
        }
    }
    let mut tiles: Vec<Vec<Tile>> = Vec::with_capacity(n);
    for _r in 0..n {
        let mut row = Vec::with_capacity(n);
        for _c in 0..n {
            let kind = if rng.next() < 0.5 { PipeKind::I } else { PipeKind::L };
            row.push(Tile { kind, rot: 0, solved: 0 });
        }
        tiles.push(row);
    }
    for row in tiles.iter_mut() {
        for t in row.iter_mut() {
            let m = t.kind.opens().len();
            t.rot = draw_below(rng, m);
        }
    }
    for k in 0..path.len() {
        let (r, c) = path[k];
        let from = if k > 0 { (step_index(path[k - 1], (r, c)).unwrap_or(1) + 2) % 4 } else { 3 };
        let to = if k + 1 < path.len() { step_index((r, c), path[k + 1]).unwrap_or(1) } else { 1 };
        let kind = if (from + 2) % 4 == to { PipeKind::I } else { PipeKind::L };
        let opens = kind.opens();
        let mut solved = 0;
        for (oi, o) in opens.iter().enumerate() {
            if o.contains(&from) && o.contains(&to) {
                solved = oi;
                break;
            }
        }
        let m = opens.len();
        let span = (m - 1).min(turns.max(1) as usize).max(1);
        let need = 1 + draw_below(rng, span);
        let t = &mut tiles[r as usize][c as usize];
        t.kind = kind;
        t.solved = solved;
        t.rot = (solved + m - need % m) % m;
    }
    let path = path.into_iter().map(|(r, c)| (r as usize, c as usize)).collect();
    Pipes { n, tiles, path }
}

// ------------------------------------------------------------------------------------------------ the game

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    L,
    R,
}

impl Side {
    pub fn id(self) -> &'static str {
        match self {
            Side::L => "L",
            Side::R => "R",
        }
    }
    /// -1 for the left window, +1 for the right.
    pub fn sign(self) -> f32 {
        match self {
            Side::L => -1.0,
            Side::R => 1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    None,
    Win,
    Caught,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Step,
    Wrong,
    /// Stage number just finished (1-based).
    Stage(usize),
    /// A ride to the stage with this index starts.
    Ride(usize),
    Win,
    Caught,
}

impl SimEvent {
    pub fn name(&self) -> &'static str {
        match self {
            SimEvent::Step => "step",
            SimEvent::Wrong => "wrong",
            SimEvent::Stage(_) => "stage",
            SimEvent::Ride(_) => "ride",
            SimEvent::Win => "win",
            SimEvent::Caught => "caught",
        }
    }
}

/// A player action on the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Wire(usize),
    Pad(usize),
    Pipe(usize, usize),
}

/// What the screen says about her pace.
#[derive(Clone, Debug, PartialEq)]
pub struct Meter {
    pub slow: bool,
    pub warn: bool,
    pub label: &'static str,
}

pub struct Sim {
    pub tun: &'static Tun,
    pub descent: i32,
    pub level: i32,
    pub elapsed: f64,
    pub over: Over,
    pub by: String,
    pub stages: Vec<Stage>,
    pub stage_idx: usize,
    pub ride: f64,
    pub threat: f64,
    pub wrong: i32,
    pub puzzle: Puzzle,
    pub held: f64,
    pub side: Side,
    pub surge_mult: f64,
    pub surge_left: f64,
    /// Admin: god mode keeps her out of the car, speed scales how fast she closes in.
    pub god: bool,
    pub speed_mult: f64,
    rng: Mulberry32,
}

impl Sim {
    /// `level_override` replaces Elise's level (Custom Night); `forced` forces the stage order (tests).
    pub fn new(descent: i32, seed: u32, level_override: Option<i32>, forced: Option<&[Stage]>) -> Sim {
        let tun = Tun::get();
        let descent = descent.clamp(1, tun.descents);
        let mut rng = Mulberry32::new(seed);
        let level = match level_override {
            Some(l) if l >= 0 => l,
            _ => Tun::at(&tun.elise_levels, descent),
        };
        let side = if rng.next() < 0.5 { Side::L } else { Side::R };
        let stages = match forced {
            Some(f) if valid_order(descent, f) => f.to_vec(),
            _ => draw_stages(descent, &mut rng),
        };
        let surge_mult = jitter(1.0, tun.surge_spread, &mut rng);
        let surge_left = jitter(tun.surge_every_ms, tun.surge_every_spread, &mut rng);
        let mut sim = Sim {
            tun,
            descent,
            level,
            elapsed: 0.0,
            over: Over::None,
            by: String::new(),
            stages,
            stage_idx: 0,
            ride: tun.ride_ms,
            threat: 0.0,
            wrong: 0,
            puzzle: Puzzle::None,
            held: 0.0,
            side,
            surge_mult,
            surge_left,
            god: false,
            speed_mult: 1.0,
            rng,
        };
        let first = sim.stages.first().copied().unwrap_or(Stage::Wires);
        sim.puzzle = sim.build(first);
        sim
    }

    pub fn rng_state(&self) -> u32 {
        self.rng.state()
    }

    fn build(&mut self, kind: Stage) -> Puzzle {
        let t = self.tun;
        let i = self.descent;
        match kind {
            Stage::Wires => {
                let buttons = Tun::at(&t.wires_buttons, i).max(1) as usize;
                let lit = draw_lit(Tun::at(&t.wires_lit, i), t.lit_spread, &mut self.rng).max(1) as usize;
                Puzzle::Wires(make_wires(buttons, buttons, lit, &mut self.rng))
            }
            Stage::Memory => {
                let len = draw_length(Tun::at(&t.memory_length, i), t.length_spread, &mut self.rng).max(1) as usize;
                Puzzle::Memory(make_memory(len, &mut self.rng, t))
            }
            Stage::Pipes => Puzzle::Pipes(make_pipes(
                Tun::at(&t.pipes_grid, i).max(2) as usize,
                &mut self.rng,
                Tun::at(&t.pipes_detours, i),
                Tun::at(&t.pipes_turns, i),
            )),
            Stage::Hold => Puzzle::Hold,
        }
    }

    /// The stage that is up now (None once the descent is over).
    pub fn kind(&self) -> Option<Stage> {
        match self.puzzle {
            Puzzle::None => None,
            Puzzle::Wires(_) => Some(Stage::Wires),
            Puzzle::Memory(_) => Some(Stage::Memory),
            Puzzle::Pipes(_) => Some(Stage::Pipes),
            Puzzle::Hold => Some(Stage::Hold),
        }
    }

    pub fn is_riding(&self) -> bool {
        self.ride > 0.0 && self.over == Over::None
    }

    /// Slowed only while the pipes are up, the car has stopped and she is out there at all.
    pub fn meter_cue(&self) -> Meter {
        let slow = self.kind() == Some(Stage::Pipes) && self.ride <= 0.0 && self.level > 0;
        Meter { slow, warn: self.threat > 60.0, label: if slow { "Outside \u{b7} slowed" } else { "Outside" } }
    }

    fn finish_stage(&mut self, ev: &mut Vec<SimEvent>) {
        self.threat = (self.threat - self.tun.stage_relief).max(0.0);
        ev.push(SimEvent::Stage(self.stage_idx + 1));
        self.stage_idx += 1;
        if self.stage_idx >= self.stages.len() {
            self.over = Over::Win;
            self.puzzle = Puzzle::None;
            ev.push(SimEvent::Win);
            return;
        }
        let next = self.stages[self.stage_idx];
        self.puzzle = self.build(next);
        self.ride = self.tun.ride_ms;
        self.held = 0.0;
        self.side = if self.rng.next() < 0.5 { Side::L } else { Side::R };
        ev.push(SimEvent::Ride(self.stage_idx));
    }

    fn wrong_press(&mut self, ev: &mut Vec<SimEvent>) {
        self.wrong += 1;
        let bump = if self.level > 0 { self.tun.wrong_bump } else { 0.0 };
        self.threat = (self.threat + bump).min(100.0);
        ev.push(SimEvent::Wrong);
    }

    /// Advance by `dt_ms`; `hold` is the door-hold button, down.
    pub fn step(&mut self, dt_ms: f64, hold: bool) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        if self.over != Over::None {
            return ev;
        }
        let t = self.tun;
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        if self.ride > 0.0 {
            self.ride = (self.ride - dt_ms).max(0.0);
            return ev;
        }
        let kind = self.kind();
        if let Puzzle::Memory(m) = &mut self.puzzle {
            m.advance_show(dt_ms, t);
        }
        // her pace drifts: drawn again every so often, never during the door hold
        if kind != Some(Stage::Hold) {
            self.surge_left -= dt_ms;
            if self.surge_left <= 0.0 {
                self.surge_mult = jitter(1.0, t.surge_spread, &mut self.rng);
                self.surge_left = jitter(t.surge_every_ms, t.surge_every_spread, &mut self.rng);
            }
        }
        if self.level > 0 {
            let rate = if kind == Some(Stage::Hold) {
                t.hold_rate
            } else {
                (if kind == Some(Stage::Pipes) { t.pipe_rate } else { 1.0 }) * self.surge_mult
            };
            self.threat = (self.threat + self.level as f64 * t.rate_per_level * dt * rate * self.speed_mult).min(100.0);
        }
        if kind == Some(Stage::Hold) {
            if hold {
                self.held += dt_ms;
            } else if self.level > 0 {
                self.threat = (self.threat + t.hold_slip * dt).min(100.0);
            }
            if self.held >= t.hold_ms {
                self.finish_stage(&mut ev);
            }
        }
        if self.god {
            self.threat = self.threat.min(99.0);
        }
        if self.threat >= 100.0 && self.over == Over::None {
            self.over = Over::Caught;
            self.by = "elise".to_string();
            ev.push(SimEvent::Caught);
        }
        ev
    }

    /// A player action on the panel. Nothing happens while the car rides, after the end, or for the wrong panel.
    pub fn act(&mut self, a: Act) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        if self.over != Over::None || self.ride > 0.0 {
            return ev;
        }
        let t = self.tun;
        match a {
            Act::Wire(button) => {
                let r = match &mut self.puzzle {
                    Puzzle::Wires(z) => press_wire(z, button, &mut self.rng),
                    _ => return ev,
                };
                match r {
                    Press::Wrong => self.wrong_press(&mut ev),
                    Press::Ok | Press::Done => {
                        ev.push(SimEvent::Step);
                        if r == Press::Done {
                            self.finish_stage(&mut ev);
                        }
                    }
                    Press::Nothing => {}
                }
            }
            Act::Pad(pad) => {
                let r = match &mut self.puzzle {
                    Puzzle::Memory(m) => m.press(pad, t),
                    _ => return ev,
                };
                match r {
                    Press::Wrong => self.wrong_press(&mut ev),
                    Press::Ok | Press::Done => {
                        ev.push(SimEvent::Step);
                        if r == Press::Done {
                            self.finish_stage(&mut ev);
                        }
                    }
                    Press::Nothing => {}
                }
            }
            Act::Pipe(r, c) => {
                let joined = match &mut self.puzzle {
                    Puzzle::Pipes(p) => {
                        if r >= p.n || c >= p.n {
                            return ev;
                        }
                        p.turn(r, c)
                    }
                    _ => return ev,
                };
                ev.push(SimEvent::Step);
                if joined {
                    self.finish_stage(&mut ev);
                }
            }
        }
        ev
    }

    // ---- admin (the game's admin hooks call these)

    /// Jump the elevator to stage `index` (1-based): that stage's puzzle, the car stopped.
    pub fn admin_goto_stage(&mut self, index: i32) {
        if self.over != Over::None || self.stages.is_empty() {
            return;
        }
        self.stage_idx = (index - 1).clamp(0, self.stages.len() as i32 - 1) as usize;
        let kind = self.stages[self.stage_idx];
        self.puzzle = self.build(kind);
        self.ride = 0.0;
        self.held = 0.0;
    }

    /// Finish the stage that is up (its events, as if it had been repaired).
    pub fn admin_solve(&mut self) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        if self.over == Over::None && !matches!(self.puzzle, Puzzle::None) {
            self.ride = 0.0;
            self.finish_stage(&mut ev);
        }
        ev
    }

    /// Deal the current stage's puzzle again.
    pub fn admin_redeal(&mut self) {
        if self.over == Over::None && self.stage_idx < self.stages.len() {
            let kind = self.stages[self.stage_idx];
            self.puzzle = self.build(kind);
            self.held = 0.0;
        }
    }

    pub fn admin_set_threat(&mut self, percent: f64) {
        self.threat = percent.clamp(0.0, if self.god { 99.0 } else { 100.0 });
    }

    pub fn admin_set_level(&mut self, n: i32) {
        self.level = n.clamp(0, crate::core::MAX_AI);
    }

    /// A hash of the state that matters, for `Headless::digest`.
    pub fn digest_into(&self, h: &mut Fnv) {
        h.f64(self.elapsed).i32(self.over as i32).f64(self.threat).i32(self.stage_idx as i32).i32(self.wrong);
        h.f64(self.ride).f64(self.held).i32(self.side.sign() as i32).u64(self.rng.state() as u64);
    }
}

/// The sim of a shift: Descent `night` (clamped to 1..5), or for a Custom Night the longest descent with Elise at her
/// slider. `seed` is the session's seed (made fresh once by the caller when the session has none).
pub fn from_session(session: &crate::core::Session, seed: u32) -> Sim {
    let tun = Tun::get();
    if session.is_custom {
        let level = session.custom_levels.get("elise").copied().unwrap_or(0).clamp(0, crate::core::MAX_AI);
        Sim::new(tun.descents, seed, Some(level), None)
    } else {
        Sim::new(session.night.clamp(1, tun.descents), seed, None, None)
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u32) -> Mulberry32 {
        Mulberry32::new(seed)
    }

    fn solve_clicks(p: &mut Pipes) {
        let path = p.path.clone();
        for (r, c) in path {
            let t = p.tiles[r][c];
            let m = t.kind.opens().len();
            for _ in 0..((t.solved + m - t.rot) % m) {
                p.turn(r, c);
            }
        }
    }

    #[test]
    fn the_tunables_file_parses_to_the_godot_numbers() {
        let t = Tun::parse(doc());
        assert_eq!(t.rate_per_level, 0.22);
        assert_eq!(t.wrong_bump, 13.0);
        assert_eq!(t.stage_relief, 34.0);
        assert_eq!(t.ride_ms, 1800.0);
        assert_eq!(t.hold_ms, 3200.0);
        assert_eq!(t.elise_levels, vec![0, 5, 8, 11, 14]);
        assert_eq!(t.runs[4], vec![Stage::Wires, Stage::Memory, Stage::Pipes, Stage::Wires, Stage::Hold]);
        assert_eq!(t.wires_buttons, vec![4, 4, 5, 6, 6]);
        assert_eq!(t.pipes_grid, vec![4, 4, 4, 5, 5]);
        assert!(doc().get("view").is_some(), "the view section is there for the map");
    }

    #[test]
    fn a_null_document_keeps_the_godot_numbers() {
        let t = Tun::parse(&Value::Null);
        assert_eq!(t.rate_per_level, 0.22);
        assert_eq!(t.runs.len(), 5);
        assert_eq!(t.memory_length, vec![3, 3, 4, 4, 5]);
    }

    #[test]
    fn stage_order_has_the_door_last_and_only_the_listed_stages() {
        for seed in 0..40u32 {
            for d in 1..=5 {
                let s = draw_stages(d, &mut rng(seed));
                assert!(valid_order(d, &s), "descent {d} seed {seed}: {s:?}");
                if d >= 4 {
                    assert_eq!(s.last(), Some(&Stage::Hold));
                }
            }
        }
        assert!(!valid_order(2, &[Stage::Memory]));
        assert!(!valid_order(4, &[Stage::Hold, Stage::Wires, Stage::Memory, Stage::Pipes]));
    }

    #[test]
    fn generated_pipes_are_solvable_with_one_or_two_turns_per_path_tile() {
        for (n, detours, turns) in [(4usize, 0, 2), (4, 0, 2), (5, 1, 2)] {
            for seed in 0..200u32 {
                let mut r = rng(seed);
                let mut p = make_pipes(n, &mut r, detours, turns);
                assert!(p.path.len() >= 2 * n - 1, "the path crosses the grid");
                assert_eq!(p.path[0], (0, 0));
                assert_eq!(*p.path.last().unwrap(), (n - 1, n - 1));
                for (r, c) in &p.path {
                    let t = p.tiles[*r][*c];
                    let m = t.kind.opens().len();
                    let clicks = (t.solved + m - t.rot) % m;
                    assert!((1..=turns as usize).contains(&clicks), "n{n} seed{seed} tile {r},{c} needs {clicks}");
                }
                solve_clicks(&mut p);
                assert!(p.connected(), "n{n} seed{seed} solves");
            }
        }
    }

    #[test]
    fn the_path_steps_up_or_left_at_most_detours_times() {
        for seed in 0..100u32 {
            let p = make_pipes(4, &mut rng(seed), 0, 2);
            for w in p.path.windows(2) {
                let (a, b) = (w[0], w[1]);
                assert!(b.0 >= a.0 && b.1 >= a.1, "no detours allowed: {a:?} -> {b:?}");
            }
        }
    }

    #[test]
    fn the_flow_enters_from_the_west_edge_of_the_first_cell() {
        let mut p = make_pipes(4, &mut rng(7), 0, 2);
        solve_clicks(&mut p);
        let (cells, done) = p.flow();
        assert!(done);
        assert_eq!(cells.first(), Some(&(0, 0)));
        assert!(p.tiles[0][0].openings().contains(&3), "the first piece opens to the west");
        let last = p.tiles[3][3].openings();
        assert!(last.contains(&1), "the last piece opens to the east");
        // a first piece that does not open west lets nothing in
        let mut q = p.clone();
        q.tiles[0][0].rot = (q.tiles[0][0].rot + 1) % q.tiles[0][0].kind.opens().len();
        if !q.tiles[0][0].openings().contains(&3) {
            assert!(q.flow().0.is_empty());
            assert!(!q.connected());
        }
    }

    #[test]
    fn turning_a_piece_cycles_its_rotations() {
        let mut p = make_pipes(4, &mut rng(3), 0, 2);
        let m = p.tiles[1][1].kind.opens().len();
        let start = p.tiles[1][1].rot;
        for _ in 0..m {
            p.turn(1, 1);
        }
        assert_eq!(p.tiles[1][1].rot, start);
    }

    #[test]
    fn wires_follow_the_lit_boxes_and_a_wrong_button_deals_again() {
        let mut r = rng(11);
        let mut z = make_wires(5, 5, 3, &mut r);
        assert_eq!(z.lit.len(), 3);
        assert_eq!(z.wire.len(), 5);
        let mut sorted = z.wire.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4], "every box has its own button");
        let right = z.wire[z.lit[0]];
        assert_eq!(press_wire(&mut z, right, &mut r), Press::Ok);
        assert_eq!(z.step, 1);
        let wrong = (z.wire[z.lit[1]] + 1) % 5;
        assert_eq!(press_wire(&mut z, wrong, &mut r), Press::Wrong);
        assert_eq!(z.step, 0, "starts over");
        for _ in 0..3 {
            let b = z.wire[z.lit[z.step]];
            let res = press_wire(&mut z, b, &mut r);
            if z.step == 3 {
                assert_eq!(res, Press::Done);
            }
        }
    }

    #[test]
    fn the_signal_show_lights_each_pad_then_waits_for_the_echo() {
        let tun = Tun::get();
        let mut m = make_memory(3, &mut rng(5), tun);
        m.pace = 1.0;
        assert_eq!(m.t, -700.0);
        m.advance_show(500.0, tun);
        assert_eq!(m.lit, -1, "still in the lead");
        m.advance_show(300.0, tun); // t = 100: first pad on
        assert_eq!(m.lit, m.seq[0] as i32);
        m.advance_show(500.0, tun); // t = 600: in the gap
        assert_eq!(m.lit, -1);
        assert_eq!(m.press(0, tun), Press::Nothing, "a press during the show does nothing");
        m.advance_show(3.0 * 780.0, tun);
        assert_eq!(m.phase, Phase::Input);
        assert_eq!(m.step, 0);
        let first = m.seq[0];
        assert_eq!(m.press(first, tun), Press::Ok);
        let bad = (m.seq[1] + 1) % 4;
        assert_eq!(m.press(bad, tun), Press::Wrong);
        assert_eq!(m.phase, Phase::Show);
        assert!((m.t + 910.0).abs() < 1e-9, "the show restarts with a longer lead (700 x 1.3)");
    }

    fn forced(d: i32, seed: u32, level: i32, stages: &[Stage]) -> Sim {
        Sim::new(d, seed, Some(level), Some(stages))
    }

    fn solve_now(sim: &mut Sim) -> Vec<SimEvent> {
        let mut all = Vec::new();
        match sim.puzzle.clone() {
            Puzzle::Wires(w) => {
                for k in 0..w.lit.len() {
                    let cur = match &sim.puzzle {
                        Puzzle::Wires(z) => z.wire[z.lit[k]],
                        _ => break,
                    };
                    all.extend(sim.act(Act::Wire(cur)));
                }
            }
            Puzzle::Memory(m) => {
                sim.step(10_000.0, false);
                for pad in m.seq.iter() {
                    all.extend(sim.act(Act::Pad(*pad)));
                }
            }
            Puzzle::Pipes(p) => {
                for (r, c) in p.path.clone() {
                    let t = p.tiles[r][c];
                    let m = t.kind.opens().len();
                    for _ in 0..((t.solved + m - t.rot) % m) {
                        all.extend(sim.act(Act::Pipe(r, c)));
                    }
                }
            }
            _ => {}
        }
        all
    }

    #[test]
    fn the_first_ride_is_1800_ms_and_blocks_input() {
        let mut s = forced(1, 4, 0, &[Stage::Wires]);
        assert!(s.is_riding());
        assert!(s.act(Act::Wire(0)).is_empty(), "no presses while the car rides");
        s.step(1700.0, false);
        assert!(s.is_riding());
        s.step(100.0, false);
        assert!(!s.is_riding());
        assert_eq!(s.elapsed, 1800.0);
    }

    #[test]
    fn a_wrong_press_adds_13_threat_only_when_elise_is_out_there() {
        let mut calm = forced(1, 9, 0, &[Stage::Wires]);
        calm.step(2000.0, false);
        let bad = match &calm.puzzle {
            Puzzle::Wires(z) => (z.wire[z.lit[0]] + 1) % z.buttons,
            _ => unreachable!(),
        };
        let ev = calm.act(Act::Wire(bad));
        assert_eq!(ev, vec![SimEvent::Wrong]);
        assert_eq!(calm.threat, 0.0, "level 0: nothing outside");
        assert_eq!(calm.wrong, 1);

        let mut live = forced(2, 9, 5, &[Stage::Wires, Stage::Memory]);
        live.step(1800.0, false);
        let before = live.threat;
        let bad = match &live.puzzle {
            Puzzle::Wires(z) => (z.wire[z.lit[0]] + 1) % z.buttons,
            _ => unreachable!(),
        };
        live.act(Act::Wire(bad));
        assert_eq!(live.threat, before + 13.0);
        assert_eq!(live.wrong, 1);
    }

    #[test]
    fn a_finished_stage_takes_34_off_her_and_starts_the_next_ride() {
        let mut s = forced(2, 21, 8, &[Stage::Wires, Stage::Memory]);
        s.step(1800.0, false);
        s.threat = 50.0;
        let ev = solve_now(&mut s);
        assert!(ev.contains(&SimEvent::Stage(1)));
        assert!(ev.contains(&SimEvent::Ride(1)));
        assert_eq!(s.threat, 16.0);
        assert!(s.is_riding());
        assert_eq!(s.kind(), Some(Stage::Memory));
        s.threat = 20.0;
        s.step(1800.0, false);
        let ev = solve_now(&mut s);
        assert!(ev.contains(&SimEvent::Win));
        assert_eq!(s.over, Over::Win);
        assert!(s.threat >= 0.0);
        assert!(s.step(100.0, false).is_empty(), "nothing happens after the end");
    }

    #[test]
    fn relief_never_goes_below_zero() {
        let mut s = forced(2, 2, 5, &[Stage::Wires, Stage::Memory]);
        s.step(1800.0, false);
        s.threat = 10.0;
        solve_now(&mut s);
        assert_eq!(s.threat, 0.0);
    }

    #[test]
    fn threat_rises_level_times_0_22_a_second_and_catches_at_100() {
        let mut s = forced(2, 33, 10, &[Stage::Wires, Stage::Memory]);
        s.step(1800.0, false);
        s.surge_mult = 1.0;
        s.surge_left = 1e9;
        s.step(1000.0, false);
        assert!((s.threat - 2.2).abs() < 1e-9, "{}", s.threat);
        let mut caught = Vec::new();
        for _ in 0..1200 {
            let ev = s.step(100.0, false);
            if ev.contains(&SimEvent::Caught) {
                caught = ev;
                break;
            }
        }
        assert_eq!(caught, vec![SimEvent::Caught]);
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "elise");
        assert!(s.threat >= 100.0);
    }

    #[test]
    fn the_pipes_slow_her_to_half_speed() {
        let mut s = forced(3, 14, 8, &[Stage::Pipes, Stage::Wires, Stage::Memory]);
        s.step(1800.0, false);
        s.surge_mult = 1.0;
        s.surge_left = 1e9;
        assert!(s.meter_cue().slow);
        s.step(1000.0, false);
        assert!((s.threat - 8.0 * 0.22 * 0.5).abs() < 1e-9);
        assert_eq!(s.meter_cue().label, "Outside \u{b7} slowed");
    }

    #[test]
    fn god_mode_keeps_her_out() {
        let mut s = forced(2, 8, 20, &[Stage::Wires, Stage::Memory]);
        s.god = true;
        s.step(1800.0, false);
        for _ in 0..3000 {
            s.step(100.0, false);
        }
        assert_eq!(s.over, Over::None);
        assert!(s.threat <= 99.0);
    }

    #[test]
    fn the_door_hold_takes_3200_ms_and_slips_when_released() {
        let mut s = forced(4, 5, 11, &[Stage::Wires, Stage::Memory, Stage::Pipes, Stage::Hold]);
        s.admin_goto_stage(4);
        assert_eq!(s.kind(), Some(Stage::Hold));
        assert_eq!(s.ride, 0.0);
        let before = s.threat;
        s.step(1000.0, false);
        assert!(s.threat > before + 22.0 * 1.0 - 1e-9, "released: slips by 22 per second plus her pace");
        assert_eq!(s.held, 0.0);
        s.threat = 0.0;
        let mut ev = Vec::new();
        for _ in 0..32 {
            ev.extend(s.step(100.0, true));
        }
        assert!(ev.contains(&SimEvent::Win), "{ev:?}");
        assert_eq!(s.over, Over::Win);
    }

    #[test]
    fn the_hold_stage_uses_2_6_times_her_rate() {
        let mut s = forced(4, 5, 10, &[Stage::Wires, Stage::Memory, Stage::Pipes, Stage::Hold]);
        s.admin_goto_stage(4);
        s.step(1000.0, true);
        assert!((s.threat - 10.0 * 0.22 * 2.6).abs() < 1e-9);
    }

    #[test]
    fn same_seed_same_descent_and_a_different_seed_differs() {
        let a = Sim::new(5, 77, None, None);
        let b = Sim::new(5, 77, None, None);
        assert_eq!(a.stages, b.stages);
        assert_eq!(a.side, b.side);
        assert_eq!(a.surge_left, b.surge_left);
        assert_eq!(format!("{:?}", a.puzzle), format!("{:?}", b.puzzle));
        let mut differs = false;
        for seed in 0..20u32 {
            let c = Sim::new(5, 1000 + seed, None, None);
            if c.stages != a.stages || c.side != a.side || format!("{:?}", c.puzzle) != format!("{:?}", a.puzzle) {
                differs = true;
            }
        }
        assert!(differs);
    }

    #[test]
    fn levels_come_from_the_table_and_a_custom_override_wins() {
        for (d, want) in [(1, 0), (2, 5), (3, 8), (4, 11), (5, 14)] {
            assert_eq!(Sim::new(d, 1, None, None).level, want);
        }
        assert_eq!(Sim::new(5, 1, Some(20), None).level, 20);
        assert_eq!(Sim::new(9, 1, None, None).descent, 5, "descents clamp to 1..5");
    }

    #[test]
    fn the_first_descent_is_a_walk_through_with_only_wires() {
        let s = Sim::new(1, 3, None, None);
        assert_eq!(s.stages, vec![Stage::Wires]);
        assert_eq!(s.level, 0);
        assert!(!s.meter_cue().slow);
    }

    #[test]
    fn admin_hooks_move_the_run() {
        let mut s = forced(3, 6, 8, &[Stage::Wires, Stage::Memory, Stage::Pipes]);
        s.admin_goto_stage(3);
        assert_eq!(s.kind(), Some(Stage::Pipes));
        s.admin_set_threat(60.0);
        assert_eq!(s.threat, 60.0);
        let ev = s.admin_solve();
        assert!(ev.contains(&SimEvent::Win));
        assert_eq!(s.threat, 26.0);
        s.admin_set_level(99);
        assert_eq!(s.level, 20);
    }

    #[test]
    fn digest_input_is_stable() {
        let mut a = Fnv::new();
        Sim::new(3, 5, None, None).digest_into(&mut a);
        let mut b = Fnv::new();
        Sim::new(3, 5, None, None).digest_into(&mut b);
        assert_eq!(a.finish(), b.finish());
    }
}
