//! The Descent night type (Godot `game/night_watch/descent/descent_logic.gd`, itself a port of the 2D `descent-logic.js`).
//! A service run down the shaft: each stage is a panel to repair (fuse boxes, the signal panel, the pipework) or, on the
//! last stage, the door to hold shut. Elise closes in while the run drags, and every wrong attempt pushes her nearer.
//! The shell drives `DescentShift::step` with the clock and the door hold, `DescentShift::act` with the panel presses and
//! `DescentShift::call_update` with Zellia's call. Art and sound are left to the shell.

use crate::data::{self, CallsData};
use crate::framework::{jitter, CallEvent, CallScript, NightWatchStream};

const MODE_ID: &str = "descent";

/// Every tunable of the mode, read once from `data/rules/descent.json` when a shift starts. Tables run per descent.
#[derive(Clone, Debug)]
struct Tuning {
    /// Elise's AI level for each descent; descent 1 keeps her away.
    levels: Vec<i32>,
    /// Highest AI level on a Custom Night.
    max_ai: i32,
    /// Score per AI level on a Custom Night.
    points_per_level: i32,
    /// The stages of each descent before the order is drawn; the door hold always comes last.
    runs: Vec<Vec<StageKind>>,
    wires_lit: Vec<usize>,
    wires_buttons: Vec<usize>,
    lit_spread: i64,
    lit_min: i64,
    memory_length: Vec<usize>,
    length_spread: i64,
    length_min: i64,
    show_on_ms: f64,
    show_off_ms: f64,
    show_lead_ms: f64,
    replay_lead_factor: f64,
    show_pace_spread: f64,
    pipes_grid: Vec<usize>,
    pipes_detours: Vec<usize>,
    pipes_turns: Vec<usize>,
    rate_per_level: f64,
    wrong_bump: f64,
    stage_relief: f64,
    pipe_rate: f64,
    surge_spread: f64,
    surge_every_ms: f64,
    surge_every_spread: f64,
    hold_ms: f64,
    hold_slip: f64,
    hold_rate: f64,
    ride_ms: f64,
}

impl Tuning {
    fn load() -> Tuning {
        let r = data::rules(MODE_ID);
        let table = |path: &str| {
            data::list(r, path)
                .iter()
                .map(|v| v.as_i64().unwrap_or_else(|| panic!("night_watch data for mode '{MODE_ID}': '{path}' holds a non-integer")))
                .collect::<Vec<i64>>()
        };
        let sizes = |path: &str| table(path).into_iter().map(|n| n as usize).collect::<Vec<usize>>();
        let tuning = Tuning {
            levels: table("elise_levels").into_iter().map(|n| n as i32).collect(),
            max_ai: data::int(r, "max_ai") as i32,
            points_per_level: data::int(r, "points_per_level") as i32,
            runs: data::list(r, "runs")
                .iter()
                .map(|run| run.as_array().map(Vec::as_slice).unwrap_or(&[]).iter().map(|k| StageKind::from_key(k.as_str().unwrap_or(""))).collect())
                .collect(),
            wires_lit: sizes("wires.lit"),
            wires_buttons: sizes("wires.buttons"),
            lit_spread: data::int(r, "wires.lit_spread"),
            lit_min: data::int(r, "wires.lit_min"),
            memory_length: sizes("memory.length"),
            length_spread: data::int(r, "memory.length_spread"),
            length_min: data::int(r, "memory.length_min"),
            show_on_ms: data::num(r, "memory.show_on_ms"),
            show_off_ms: data::num(r, "memory.show_off_ms"),
            show_lead_ms: data::num(r, "memory.show_lead_ms"),
            replay_lead_factor: data::num(r, "memory.replay_lead_factor"),
            show_pace_spread: data::num(r, "memory.pace_spread"),
            pipes_grid: sizes("pipes.grid"),
            pipes_detours: sizes("pipes.detours"),
            pipes_turns: sizes("pipes.turns"),
            rate_per_level: data::num(r, "threat.rate_per_level"),
            wrong_bump: data::num(r, "threat.wrong_bump"),
            stage_relief: data::num(r, "threat.stage_relief"),
            pipe_rate: data::num(r, "threat.pipe_rate"),
            surge_spread: data::num(r, "threat.surge_spread"),
            surge_every_ms: data::num(r, "threat.surge_every_ms"),
            surge_every_spread: data::num(r, "threat.surge_every_spread"),
            hold_ms: data::num(r, "hold.ms"),
            hold_slip: data::num(r, "hold.slip"),
            hold_rate: data::num(r, "hold.rate"),
            ride_ms: data::num(r, "ride_ms"),
        };
        let n = tuning.levels.len();
        let rows = [
            tuning.runs.len(),
            tuning.wires_lit.len(),
            tuning.wires_buttons.len(),
            tuning.memory_length.len(),
            tuning.pipes_grid.len(),
            tuning.pipes_detours.len(),
            tuning.pipes_turns.len(),
        ];
        assert!(n > 0 && rows.iter().all(|len| *len == n), "night_watch data for mode '{MODE_ID}': the per-descent tables must all have {n} rows");
        tuning
    }

    /// How many descents the mode has.
    fn descents(&self) -> usize {
        self.levels.len()
    }

    fn draw_lit(&self, base: usize, rng: &mut NightWatchStream) -> usize {
        let offset = draw_index(rng, (2 * self.lit_spread + 1) as usize) as i64 - self.lit_spread;
        (base as i64 + offset).max(self.lit_min) as usize
    }

    fn draw_length(&self, base: usize, rng: &mut NightWatchStream) -> usize {
        let offset = draw_index(rng, (2 * self.length_spread + 1) as usize) as i64 - self.length_spread;
        (base as i64 + offset).max(self.length_min) as usize
    }
}

/// Pipe grid steps as (row, col) for N, E, S and W.
const STEP: [(isize, isize); 4] = [(-1, 0), (0, 1), (1, 0), (0, -1)];
/// The two open pairs of a straight piece (N-S or E-W, as sides N=0 E=1 S=2 W=3).
const STRAIGHT_PAIRS: [[usize; 2]; 2] = [[3, 1], [0, 2]];
/// The four open pairs of a corner piece, indexed by rotation.
const CORNER_PAIRS: [[usize; 2]; 4] = [[0, 1], [1, 2], [2, 3], [3, 0]];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    Wires,
    Memory,
    Pipes,
    Hold,
}

impl StageKind {
    /// The panel's title, from `stage_titles` in the rules file.
    pub fn title(self) -> &'static str {
        data::text(data::rules(MODE_ID), &format!("stage_titles.{}", self.key()))
    }

    /// The key the rules file uses for this stage.
    fn key(self) -> &'static str {
        match self {
            StageKind::Wires => "wires",
            StageKind::Memory => "memory",
            StageKind::Pipes => "pipes",
            StageKind::Hold => "hold",
        }
    }

    fn from_key(key: &str) -> StageKind {
        [StageKind::Wires, StageKind::Memory, StageKind::Pipes, StageKind::Hold]
            .into_iter()
            .find(|kind| kind.key() == key)
            .unwrap_or_else(|| panic!("night_watch data for mode '{MODE_ID}': unknown stage '{key}' in runs"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Playing,
    Won,
    Caught,
}

/// What a press did on a panel: ignored, a correct press, the last correct press of a stage, or a wrong press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Ignored,
    Ok,
    Done,
    Wrong,
}

/// A player action on the service panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescentAction {
    Wire { button: usize },
    Pad { pad: usize },
    Pipe { row: usize, col: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescentEvent {
    Step,
    Wrong,
    /// A stage is repaired; `number` counts from 1.
    StageCleared { number: usize },
    /// The car moves on to stage `index` (0-based) and waits out the ride.
    Ride { index: usize },
    Won,
    Caught,
}

/// Fuse boxes: the lit boxes light in turn, and each must be wired to its button.
#[derive(Clone, Debug)]
pub struct Wires {
    pub boxes: usize,
    pub buttons: usize,
    /// The button each box is wired to.
    pub wire: Vec<usize>,
    /// The boxes lit, in the order they must be followed.
    pub lit: Vec<usize>,
    pub colors: Vec<usize>,
    pub step: usize,
}

impl Wires {
    fn new(boxes: usize, buttons: usize, lit_count: usize, rng: &mut NightWatchStream) -> Wires {
        let mut wires = Wires { boxes, buttons, wire: Vec::new(), lit: Vec::new(), colors: Vec::new(), step: 0 };
        wires.deal(rng);
        wires.lit = shuffle(range_vec(boxes), rng).into_iter().take(lit_count).collect();
        wires.colors = shuffle(range_vec(6), rng).into_iter().take(boxes).collect();
        wires
    }

    fn deal(&mut self, rng: &mut NightWatchStream) {
        self.wire = shuffle(range_vec(self.buttons), rng).into_iter().take(self.boxes).collect();
    }

    /// A wrong press starts the run over and re-deals the wires.
    pub fn press(&mut self, button: usize, rng: &mut NightWatchStream) -> Press {
        if button == self.wire[self.lit[self.step]] {
            self.step += 1;
            return if self.step == self.lit.len() { Press::Done } else { Press::Ok };
        }
        self.step = 0;
        self.deal(rng);
        Press::Wrong
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryPhase {
    Show,
    Input,
}

/// The signal panel: four pads light in turn, then the same order is pressed back.
#[derive(Clone, Debug)]
pub struct Memory {
    pub seq: Vec<usize>,
    pub pace: f64,
    pub hues: Vec<usize>,
    pub step: usize,
    pub phase: MemoryPhase,
    pub t: f64,
    pub lit: Option<usize>,
    on_ms: f64,
    off_ms: f64,
    lead_ms: f64,
    replay_lead_factor: f64,
}

impl Memory {
    fn new(length: usize, tune: &Tuning, rng: &mut NightWatchStream) -> Memory {
        let seq = (0..length).map(|_| draw_index(rng, 4)).collect();
        let pace = jitter(1.0, tune.show_pace_spread, rng);
        let hues = shuffle(range_vec(4), rng);
        Memory {
            seq,
            pace,
            hues,
            step: 0,
            phase: MemoryPhase::Show,
            t: -tune.show_lead_ms,
            lit: None,
            on_ms: tune.show_on_ms,
            off_ms: tune.show_off_ms,
            lead_ms: tune.show_lead_ms,
            replay_lead_factor: tune.replay_lead_factor,
        }
    }

    fn advance_show(&mut self, dt_ms: f64) {
        if self.phase != MemoryPhase::Show {
            return;
        }
        self.t += dt_ms;
        if self.t < 0.0 {
            self.lit = None;
            return;
        }
        let on_ms = self.on_ms * self.pace;
        let cycle = (self.on_ms + self.off_ms) * self.pace;
        let k = (self.t / cycle).floor() as usize;
        if k >= self.seq.len() {
            self.phase = MemoryPhase::Input;
            self.lit = None;
            self.step = 0;
            return;
        }
        self.lit = if self.t.rem_euclid(cycle) < on_ms { Some(self.seq[k]) } else { None };
    }

    /// Presses during the show are ignored. A wrong press makes the show play again.
    pub fn press(&mut self, pad: usize) -> Press {
        if self.phase != MemoryPhase::Input {
            return Press::Ignored;
        }
        if pad == self.seq[self.step] {
            self.step += 1;
            return if self.step == self.seq.len() { Press::Done } else { Press::Ok };
        }
        self.phase = MemoryPhase::Show;
        self.t = -self.lead_ms * self.replay_lead_factor;
        self.step = 0;
        self.lit = None;
        Press::Wrong
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeKind {
    Straight,
    Corner,
}

impl PipeKind {
    pub fn pairs(self) -> &'static [[usize; 2]] {
        match self {
            PipeKind::Straight => &STRAIGHT_PAIRS,
            PipeKind::Corner => &CORNER_PAIRS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub kind: PipeKind,
    pub rot: usize,
}

impl Tile {
    pub fn opening(&self) -> [usize; 2] {
        self.kind.pairs()[self.rot]
    }
}

/// The pipework: turn the pieces until one line runs from the left edge to the right edge.
#[derive(Clone, Debug)]
pub struct Pipes {
    pub n: usize,
    pub tiles: Vec<Vec<Tile>>,
}

impl Pipes {
    /// Turns one piece a quarter turn; a turn that connects the line is Done.
    pub fn turn(&mut self, row: usize, col: usize) -> Press {
        match self.tiles.get_mut(row).and_then(|r| r.get_mut(col)) {
            None => return Press::Ignored,
            Some(tile) => tile.rot = (tile.rot + 1) % tile.kind.pairs().len(),
        }
        if self.connected() {
            Press::Done
        } else {
            Press::Ok
        }
    }

    fn connected(&self) -> bool {
        let n = self.n;
        let (mut row, mut col, mut enter) = (0usize, 0usize, 3usize);
        for _ in 0..n * n + 2 {
            let open = self.tiles[row][col].opening();
            if !open.contains(&enter) {
                return false;
            }
            let out = if open[0] == enter { open[1] } else { open[0] };
            if row == n - 1 && col == n - 1 && out == 1 {
                return true;
            }
            let next_row = row as isize + STEP[out].0;
            let next_col = col as isize + STEP[out].1;
            if next_row < 0 || next_col < 0 || next_row >= n as isize || next_col >= n as isize {
                return false;
            }
            row = next_row as usize;
            col = next_col as usize;
            enter = (out + 2) % 4;
        }
        false
    }
}

#[derive(Clone, Debug)]
pub enum Puzzle {
    Wires(Wires),
    Memory(Memory),
    Pipes(Pipes),
    Hold,
    Finished,
}

impl Puzzle {
    pub fn stage(&self) -> Option<StageKind> {
        match self {
            Puzzle::Wires(_) => Some(StageKind::Wires),
            Puzzle::Memory(_) => Some(StageKind::Memory),
            Puzzle::Pipes(_) => Some(StageKind::Pipes),
            Puzzle::Hold => Some(StageKind::Hold),
            Puzzle::Finished => None,
        }
    }
}

/// One Descent shift. Built by `DescentShift::start`, the shell's only entry point.
pub struct DescentShift {
    tune: Tuning,
    descent: usize,
    custom: bool,
    level: i32,
    elapsed: f64,
    outcome: Outcome,
    stages: Vec<StageKind>,
    stage_idx: usize,
    puzzle: Puzzle,
    ride: f64,
    threat: f64,
    wrong: u32,
    held: f64,
    side: Side,
    surge_mult: f64,
    surge_left: f64,
    rng: NightWatchStream,
    pub call: CallScript,
}

impl DescentShift {
    /// Starts descent `descent` (1 to 5) from `seed`. `custom_level` is the Custom Night's AI level for Elise and turns
    /// off Zellia's call, as in Godot. Zellia's call comes from `calls` (mode `descent`).
    pub fn start(descent: usize, seed: i64, custom_level: Option<i32>, calls: &CallsData) -> DescentShift {
        let tune = Tuning::load();
        let descent = descent.clamp(1, tune.descents());
        let mut rng = NightWatchStream::seeded(seed);
        let level = match custom_level {
            Some(n) => n.clamp(0, tune.max_ai),
            None => tune.levels[descent - 1],
        };
        let side = draw_side(&mut rng);
        let stages = draw_stages(descent, &tune, &mut rng);
        let surge_mult = jitter(1.0, tune.surge_spread, &mut rng);
        let surge_left = jitter(tune.surge_every_ms, tune.surge_every_spread, &mut rng);
        let puzzle = build(stages[0], descent, &tune, &mut rng);
        let call = if custom_level.is_some() {
            CallScript::new(Vec::new(), 0.0)
        } else {
            CallScript::for_level(calls, MODE_ID, descent as i32)
        };
        let ride = tune.ride_ms;
        DescentShift {
            tune,
            descent,
            custom: custom_level.is_some(),
            level,
            elapsed: 0.0,
            outcome: Outcome::Playing,
            stages,
            stage_idx: 0,
            puzzle,
            ride,
            threat: 0.0,
            wrong: 0,
            held: 0.0,
            side,
            surge_mult,
            surge_left,
            rng,
            call,
        }
    }

    /// Advances the shift by `dt_ms`. `hold` is the door-hold button, down. Returns what happened.
    pub fn step(&mut self, dt_ms: f64, hold: bool) -> Vec<DescentEvent> {
        let mut events = Vec::new();
        if self.outcome != Outcome::Playing {
            return events;
        }
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        if self.ride > 0.0 {
            self.ride = (self.ride - dt_ms).max(0.0);
            return events;
        }
        let kind = self.puzzle.stage();
        if let Puzzle::Memory(memory) = &mut self.puzzle {
            memory.advance_show(dt_ms);
        }
        if kind != Some(StageKind::Hold) {
            self.surge_left -= dt_ms;
            if self.surge_left <= 0.0 {
                self.surge_mult = jitter(1.0, self.tune.surge_spread, &mut self.rng);
                self.surge_left = jitter(self.tune.surge_every_ms, self.tune.surge_every_spread, &mut self.rng);
            }
        }
        if self.level > 0 {
            let rate = match kind {
                Some(StageKind::Hold) => self.tune.hold_rate,
                Some(StageKind::Pipes) => self.tune.pipe_rate * self.surge_mult,
                _ => self.surge_mult,
            };
            self.threat = (self.threat + f64::from(self.level) * self.tune.rate_per_level * dt * rate).min(100.0);
        }
        if kind == Some(StageKind::Hold) {
            if hold {
                self.held += dt_ms;
            } else if self.level > 0 {
                self.threat = (self.threat + self.tune.hold_slip * dt).min(100.0);
            }
            if self.held >= self.tune.hold_ms {
                self.finish_stage(&mut events);
            }
        }
        if self.threat >= 100.0 && self.outcome == Outcome::Playing {
            self.outcome = Outcome::Caught;
            events.push(DescentEvent::Caught);
        }
        events
    }

    /// A press on the service panel. Nothing happens while the car rides or the shift is over, or when the press
    /// belongs to a different stage than the one up.
    pub fn act(&mut self, action: DescentAction) -> Vec<DescentEvent> {
        let mut events = Vec::new();
        if self.outcome != Outcome::Playing || self.ride > 0.0 {
            return events;
        }
        let press = match (action, &mut self.puzzle) {
            (DescentAction::Wire { button }, Puzzle::Wires(wires)) => wires.press(button, &mut self.rng),
            (DescentAction::Pad { pad }, Puzzle::Memory(memory)) => memory.press(pad),
            (DescentAction::Pipe { row, col }, Puzzle::Pipes(pipes)) => pipes.turn(row, col),
            _ => Press::Ignored,
        };
        match press {
            Press::Ignored => {}
            Press::Ok => events.push(DescentEvent::Step),
            Press::Done => {
                events.push(DescentEvent::Step);
                self.finish_stage(&mut events);
            }
            Press::Wrong => self.record_wrong(&mut events),
        }
        events
    }

    /// Zellia's call, driven by the time `step` has run so far.
    pub fn call_update(&mut self, dt_ms: f64) -> Vec<CallEvent> {
        self.call.update(self.elapsed, dt_ms)
    }

    fn finish_stage(&mut self, events: &mut Vec<DescentEvent>) {
        self.threat = (self.threat - self.tune.stage_relief).max(0.0);
        events.push(DescentEvent::StageCleared { number: self.stage_idx + 1 });
        self.stage_idx += 1;
        if self.stage_idx >= self.stages.len() {
            self.outcome = Outcome::Won;
            self.puzzle = Puzzle::Finished;
            events.push(DescentEvent::Won);
            return;
        }
        let kind = self.stages[self.stage_idx];
        self.puzzle = build(kind, self.descent, &self.tune, &mut self.rng);
        self.ride = self.tune.ride_ms;
        self.held = 0.0;
        self.side = draw_side(&mut self.rng);
        events.push(DescentEvent::Ride { index: self.stage_idx });
    }

    fn record_wrong(&mut self, events: &mut Vec<DescentEvent>) {
        self.wrong += 1;
        if self.level > 0 {
            self.threat = (self.threat + self.tune.wrong_bump).min(100.0);
        }
        events.push(DescentEvent::Wrong);
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Elise's closeness, 0 to 100. She is in at 100.
    pub fn threat(&self) -> f64 {
        self.threat
    }

    pub fn wrong_attempts(&self) -> u32 {
        self.wrong
    }

    pub fn stages_cleared(&self) -> usize {
        self.stage_idx
    }

    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    pub fn puzzle(&self) -> &Puzzle {
        &self.puzzle
    }

    /// Milliseconds the door hold has been down on the last stage.
    pub fn held_ms(&self) -> f64 {
        self.held
    }

    /// Milliseconds the door hold needs down on the last stage.
    pub fn hold_target_ms(&self) -> f64 {
        self.tune.hold_ms
    }

    /// Milliseconds of the car's ride still to go before the next stage can be worked.
    pub fn ride_left_ms(&self) -> f64 {
        self.ride
    }

    /// The whole ride between two stages, in milliseconds.
    pub fn ride_total_ms(&self) -> f64 {
        self.tune.ride_ms
    }

    /// True while the car is still moving between stages.
    pub fn riding(&self) -> bool {
        self.ride > 0.0
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn level(&self) -> i32 {
        self.level
    }

    pub fn descent_number(&self) -> usize {
        self.descent
    }

    pub fn is_custom(&self) -> bool {
        self.custom
    }

    /// A Custom Night's score for a win (Godot `record_win`); zero on a normal descent.
    pub fn score(&self) -> i32 {
        if self.custom {
            self.level * self.tune.points_per_level
        } else {
            0
        }
    }
}

fn draw_index(rng: &mut NightWatchStream, n: usize) -> usize {
    ((rng.next() * n as f64).floor() as usize).min(n.saturating_sub(1))
}

fn draw_side(rng: &mut NightWatchStream) -> Side {
    if rng.next() < 0.5 {
        Side::Left
    } else {
        Side::Right
    }
}

fn range_vec(n: usize) -> Vec<usize> {
    (0..n).collect()
}

fn shuffle<T>(mut items: Vec<T>, rng: &mut NightWatchStream) -> Vec<T> {
    for i in (1..items.len()).rev() {
        let j = draw_index(rng, i + 1);
        items.swap(i, j);
    }
    items
}

fn draw_stages(descent: usize, tune: &Tuning, rng: &mut NightWatchStream) -> Vec<StageKind> {
    let list = &tune.runs[descent - 1];
    let rest: Vec<StageKind> = list.iter().copied().filter(|kind| *kind != StageKind::Hold).collect();
    let mut order = shuffle(rest, rng);
    if list.contains(&StageKind::Hold) {
        order.push(StageKind::Hold);
    }
    order
}

fn build(kind: StageKind, descent: usize, tune: &Tuning, rng: &mut NightWatchStream) -> Puzzle {
    let i = descent - 1;
    match kind {
        StageKind::Wires => {
            let lit = tune.draw_lit(tune.wires_lit[i], rng);
            Puzzle::Wires(Wires::new(tune.wires_buttons[i], tune.wires_buttons[i], lit, rng))
        }
        StageKind::Memory => {
            let length = tune.draw_length(tune.memory_length[i], rng);
            Puzzle::Memory(Memory::new(length, tune, rng))
        }
        StageKind::Pipes => Puzzle::Pipes(make_pipes(tune.pipes_grid[i], rng, tune.pipes_detours[i], tune.pipes_turns[i])),
        StageKind::Hold => Puzzle::Hold,
    }
}

/// The hidden route from the left edge to the right edge: a random walk that may take at most `detours` steps up or
/// left, retried a fixed number of times, with a plain staircase route as the last resort.
fn draw_path(n: usize, rng: &mut NightWatchStream, detours: usize) -> Vec<(usize, usize)> {
    for _ in 0..80 {
        let mut walk = vec![(0usize, 0usize)];
        let mut seen = vec![vec![false; n]; n];
        seen[0][0] = true;
        let mut back = 0usize;
        while walk.len() < n * n {
            let (row, col) = walk[walk.len() - 1];
            if row == n - 1 && col == n - 1 {
                return walk;
            }
            let mut options: Vec<(usize, usize, usize)> = Vec::new();
            for d in 0..4 {
                let nr = row as isize + STEP[d].0;
                let nc = col as isize + STEP[d].1;
                if nr < 0 || nc < 0 || nr >= n as isize || nc >= n as isize {
                    continue;
                }
                let (nr, nc) = (nr as usize, nc as usize);
                if seen[nr][nc] {
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
            let (nr, nc, d) = options[draw_index(rng, options.len())];
            if d == 0 || d == 3 {
                back += 1;
            }
            walk.push((nr, nc));
            seen[nr][nc] = true;
        }
        let (row, col) = walk[walk.len() - 1];
        if row == n - 1 && col == n - 1 {
            return walk;
        }
    }
    let mut path = vec![(0, 0)];
    for k in 1..n {
        path.push((0, k));
    }
    for k in 1..n {
        path.push((k, n - 1));
    }
    path
}

fn step_index(from: (usize, usize), to: (usize, usize)) -> usize {
    (0..4)
        .find(|&d| from.0 as isize + STEP[d].0 == to.0 as isize && from.1 as isize + STEP[d].1 == to.1 as isize)
        .unwrap_or(0)
}

/// A grid of random pieces, then each piece on the hidden route is set to connect and turned `need` clicks away.
fn make_pipes(n: usize, rng: &mut NightWatchStream, detours: usize, turns: usize) -> Pipes {
    let path = draw_path(n, rng, detours);
    let mut tiles: Vec<Vec<Tile>> = Vec::with_capacity(n);
    for _ in 0..n {
        let mut row = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = if rng.next() < 0.5 { PipeKind::Straight } else { PipeKind::Corner };
            row.push(Tile { kind, rot: 0 });
        }
        tiles.push(row);
    }
    for tile in tiles.iter_mut().flatten() {
        tile.rot = draw_index(rng, tile.kind.pairs().len());
    }
    for (k, &(row, col)) in path.iter().enumerate() {
        let from = if k == 0 { 3 } else { (step_index(path[k - 1], (row, col)) + 2) % 4 };
        let to = if k + 1 < path.len() { step_index((row, col), path[k + 1]) } else { 1 };
        let kind = if (from + 2) % 4 == to { PipeKind::Straight } else { PipeKind::Corner };
        let pairs = kind.pairs();
        let solved = pairs.iter().position(|pair| pair.contains(&from) && pair.contains(&to)).unwrap_or(0);
        let m = pairs.len();
        let need = 1 + draw_index(rng, (m - 1).min(turns));
        tiles[row][col] = Tile { kind, rot: (solved + m - need) % m };
    }
    Pipes { n, tiles }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_wire_press_restarts_the_run_and_deals_the_wires_again() {
        let mut wires = Wires { boxes: 4, buttons: 4, wire: vec![0, 1, 2, 3], lit: vec![2, 0], colors: vec![0, 1, 2, 3], step: 0 };
        let mut rng = NightWatchStream::seeded(7);
        assert_eq!(wires.press(2, &mut rng), Press::Ok);
        assert_eq!(wires.press(1, &mut rng), Press::Wrong);
        assert_eq!(wires.step, 0);
        let mut dealt = wires.wire.clone();
        dealt.sort_unstable();
        assert_eq!(dealt, vec![0, 1, 2, 3]);
    }

    /// The values the mode used to hard-code, to prove the JSON carries them over unchanged.
    #[test]
    fn the_rules_file_carries_the_old_constants() {
        let t = Tuning::load();
        assert_eq!(t.levels, vec![0, 5, 8, 11, 14]);
        assert_eq!((t.max_ai, t.points_per_level), (20, 10));
        assert_eq!(t.wires_lit, vec![3, 3, 4, 4, 5]);
        assert_eq!(t.wires_buttons, vec![4, 4, 5, 6, 6]);
        assert_eq!(t.memory_length, vec![3, 3, 4, 4, 5]);
        assert_eq!(t.pipes_grid, vec![4, 4, 4, 5, 5]);
        assert_eq!(t.pipes_detours, vec![0, 0, 0, 1, 1]);
        assert_eq!(t.pipes_turns, vec![2, 2, 2, 2, 2]);
        assert_eq!((t.lit_spread, t.lit_min, t.length_spread, t.length_min), (1, 2, 1, 3));
        assert_eq!((t.rate_per_level, t.wrong_bump, t.stage_relief, t.ride_ms), (0.22, 13.0, 34.0, 1800.0));
        assert_eq!(
            (t.show_on_ms, t.show_off_ms, t.show_lead_ms, t.replay_lead_factor, t.show_pace_spread),
            (520.0, 260.0, 700.0, 1.3, 0.15)
        );
        assert_eq!((t.hold_ms, t.hold_slip, t.hold_rate, t.pipe_rate), (3200.0, 22.0, 2.6, 0.5));
        assert_eq!((t.surge_spread, t.surge_every_ms, t.surge_every_spread), (0.2, 9000.0, 0.33));
        use StageKind::*;
        assert_eq!(t.runs[0], vec![Wires]);
        assert_eq!(t.runs[4], vec![Wires, Memory, Pipes, Wires, Hold]);
        assert_eq!(t.descents(), 5);
    }

    #[test]
    fn stage_titles_come_from_the_rules_file() {
        assert_eq!(StageKind::Wires.title(), "FUSE BOXES");
        assert_eq!(StageKind::Memory.title(), "SIGNAL PANEL");
        assert_eq!(StageKind::Pipes.title(), "PIPEWORK");
        assert_eq!(StageKind::Hold.title(), "DOOR");
    }

    #[test]
    fn a_custom_night_scores_its_level_times_the_points() {
        let shift = DescentShift::start(3, 1, Some(99), &CallsData::load());
        assert_eq!(shift.level(), 20);
        assert_eq!(shift.score(), 200);
    }
}
