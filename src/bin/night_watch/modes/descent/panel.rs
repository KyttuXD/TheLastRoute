//! The service panel's screen (`descent_panel.gd`): the fuse boxes and their buttons, the four signal pads, the pipe grid or
//! the door hold. The layout and the mouse hit tests are pure (`hit`); `draw` paints the same rectangles in the 480 x 270
//! virtual canvas. Sizes are the Godot ones: the panel is 172 x 150, centred on (240, 124), its body 164 x 114.
#![allow(dead_code)]

use super::sim::{Act, Over, Phase, Puzzle, Sim, Stage};
use crate::hud::status::{c3, dot, fill, line, stroke, vtext, vtext_center};
use macroquad::prelude::*;

pub const PANEL_W: f32 = 172.0;
pub const PANEL_H: f32 = 150.0;
/// Top left of the panel: `Vector2(240, 124) - SIZE * 0.5`.
pub const PANEL_X: f32 = 240.0 - PANEL_W * 0.5;
pub const PANEL_Y: f32 = 124.0 - PANEL_H * 0.5;
pub const BODY_X: f32 = 4.0;
pub const BODY_Y: f32 = 14.0;
pub const BODY_W: f32 = PANEL_W - 8.0;
pub const BODY_H: f32 = PANEL_H - 36.0;

pub const AMBER: Color = Color::new(0.96, 0.72, 0.28, 1.0);
pub const INK: Color = Color::new(0.93, 0.89, 0.80, 1.0);
pub const BACK: Color = Color::new(0.05, 0.04, 0.07, 0.92);
const WIRE_COLOURS: [[f32; 3]; 6] = [
    [0.878, 0.690, 0.251],
    [0.314, 0.690, 0.878],
    [0.878, 0.376, 0.376],
    [0.439, 0.816, 0.565],
    [0.753, 0.502, 0.878],
    [0.878, 0.565, 0.314],
];
const PAD_COLOURS: [[f32; 3]; 4] = [
    [0.878, 0.376, 0.376],
    [0.878, 0.690, 0.251],
    [0.314, 0.690, 0.878],
    [0.439, 0.816, 0.565],
];
const WET: Color = Color::new(0.498, 0.878, 1.0, 1.0);
const DRY: Color = Color::new(0.604, 0.627, 0.690, 1.0);

/// A rectangle in the virtual canvas: x, y, w, h.
pub type R = [f32; 4];

fn from_body(x: f32, y: f32, w: f32, h: f32) -> R {
    [PANEL_X + BODY_X + x, PANEL_Y + BODY_Y + y, w, h]
}

pub fn panel_rect() -> R {
    [PANEL_X, PANEL_Y, PANEL_W, PANEL_H]
}

pub fn contains(r: R, p: (f32, f32)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

pub fn wire_box_rect(i: usize, boxes: usize) -> R {
    let rows = BODY_H / (boxes + 1) as f32;
    from_body(6.0, (i + 1) as f32 * rows - 5.0, 16.0, 10.0)
}

pub fn wire_button_rect(b: usize, buttons: usize) -> R {
    let brows = BODY_H / (buttons + 1) as f32;
    from_body(BODY_W - 24.0, (b + 1) as f32 * brows - 6.0, 18.0, 12.0)
}

/// The wire of box `i` to button `button`: start and end in the canvas.
pub fn wire_line(i: usize, boxes: usize, button: usize, buttons: usize) -> ((f32, f32), (f32, f32)) {
    let rows = BODY_H / (boxes + 1) as f32;
    let brows = BODY_H / (buttons + 1) as f32;
    let a = from_body(26.0, (i + 1) as f32 * rows, 0.0, 0.0);
    let b = from_body(BODY_W - 26.0, (button + 1) as f32 * brows, 0.0, 0.0);
    ((a[0], a[1]), (b[0], b[1]))
}

pub fn pad_rect(i: usize) -> R {
    from_body(BODY_W * 0.5 - 44.0 + (i % 2) as f32 * 48.0, 6.0 + (i / 2) as f32 * 42.0, 40.0, 34.0)
}

/// Tile size and left offset of the pipe grid.
pub fn pipe_geometry(n: usize) -> (f32, f32) {
    let n = n.max(1) as f32;
    let tile = ((BODY_W - 44.0) / n).min((BODY_H - 4.0) / n).floor();
    (tile, (BODY_W - tile * n) * 0.5)
}

pub fn pipe_rect(n: usize, r: usize, c: usize) -> R {
    let (tile, ox) = pipe_geometry(n);
    from_body(ox + c as f32 * tile, 2.0 + r as f32 * tile, tile - 1.0, tile - 1.0)
}

pub fn hold_bar_rect() -> R {
    from_body(10.0, 42.0, BODY_W - 20.0, 6.0)
}

pub fn hold_button_rect() -> R {
    from_body(10.0, 58.0, BODY_W - 20.0, 16.0)
}

/// What a click on the panel means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelAction {
    Act(Act),
    /// The door-hold button (held while the mouse is down on it).
    HoldDown,
}

/// The control under `p` (canvas coordinates), if the panel is live: nothing while the car rides or after the end, and the
/// pads only answer once the show is over.
pub fn hit(sim: &Sim, p: (f32, f32)) -> Option<PanelAction> {
    if sim.over != Over::None || sim.ride > 0.0 {
        return None;
    }
    match &sim.puzzle {
        Puzzle::Wires(z) => {
            for b in 0..z.buttons {
                if contains(wire_button_rect(b, z.buttons), p) {
                    return Some(PanelAction::Act(Act::Wire(b)));
                }
            }
            None
        }
        Puzzle::Memory(m) => {
            if m.phase != Phase::Input {
                return None;
            }
            (0..4).find(|i| contains(pad_rect(*i), p)).map(|i| PanelAction::Act(Act::Pad(i)))
        }
        Puzzle::Pipes(pp) => {
            for r in 0..pp.n {
                for c in 0..pp.n {
                    if contains(pipe_rect(pp.n, r, c), p) {
                        return Some(PanelAction::Act(Act::Pipe(r, c)));
                    }
                }
            }
            None
        }
        Puzzle::Hold => {
            if contains(hold_button_rect(), p) {
                Some(PanelAction::HoldDown)
            } else {
                None
            }
        }
        Puzzle::None => None,
    }
}

/// The text the panel shows: its title and the line under the body.
pub fn labels(sim: &Sim) -> (String, String) {
    if sim.over != Over::None {
        return (String::new(), String::new());
    }
    if sim.ride > 0.0 {
        return (String::new(), String::new());
    }
    match &sim.puzzle {
        Puzzle::Wires(z) => (Stage::Wires.title(), format!("{} / {}", z.step, z.lit.len())),
        Puzzle::Memory(m) => (
            Stage::Memory.title(),
            if m.phase == Phase::Show { "WATCH".to_string() } else { format!("REPEAT \u{b7} {} / {}", m.step, m.seq.len()) },
        ),
        Puzzle::Pipes(_) => (Stage::Pipes.title(), "TURN THE PIECES \u{b7} CONNECT IN TO OUT".to_string()),
        Puzzle::Hold => (Stage::Hold.title(), String::new()),
        Puzzle::None => (String::new(), String::new()),
    }
}

/// The big text while the car drops a floor.
pub fn ride_text(sim: &Sim) -> Option<String> {
    if sim.over == Over::None && sim.ride > 0.0 {
        Some(format!("DESCENDING\nFLOOR {}", sim.stage_idx + 1))
    } else {
        None
    }
}

// ------------------------------------------------------------------------------------------------ drawing

fn darkened(c: [f32; 3], amount: f32) -> Color {
    Color::new(c[0] * (1.0 - amount), c[1] * (1.0 - amount), c[2] * (1.0 - amount), 1.0)
}

fn rect(r: R) -> Rect {
    Rect::new(r[0], r[1], r[2], r[3])
}

fn button(r: R, label: &str, size: f32, hot: bool, down: bool) {
    let bg = if down {
        Color::new(0.30, 0.28, 0.34, 1.0)
    } else if hot {
        Color::new(0.26, 0.25, 0.30, 1.0)
    } else {
        Color::new(0.18, 0.17, 0.21, 1.0)
    };
    fill(rect(r), bg);
    stroke(rect(r), if hot { AMBER } else { Color::new(0.45, 0.45, 0.5, 1.0) }, 1.0);
    vtext_center(label, r[0] + r[2] * 0.5, r[1] + (r[3] - size) * 0.5 - 0.5, size, INK);
}

/// Paint the panel for the sim's state. `mouse` is in canvas coordinates; `holding` is the door button (key or mouse).
pub fn draw(sim: &Sim, mouse: (f32, f32), holding: bool) {
    fill(rect(panel_rect()), BACK);
    let (title, sub) = labels(sim);
    vtext_center(&title, PANEL_X + PANEL_W * 0.5, PANEL_Y + 2.0, 7.0, AMBER);
    vtext_center(&sub, PANEL_X + PANEL_W * 0.5, PANEL_Y + PANEL_H - 18.0, 7.0, INK);
    if let Some(t) = ride_text(sim) {
        let body = from_body(0.0, 0.0, BODY_W, BODY_H);
        for (k, l) in t.split('\n').enumerate() {
            vtext_center(l, body[0] + body[2] * 0.5, body[1] + body[3] * 0.5 - 10.0 + k as f32 * 12.0, 9.0, AMBER);
        }
        return;
    }
    if sim.over != Over::None {
        return;
    }
    match &sim.puzzle {
        Puzzle::Wires(z) => {
            let target = z.lit.get(z.step).copied().unwrap_or(usize::MAX);
            for i in 0..z.boxes {
                let ci = z.colors.get(i).copied().unwrap_or(i % 6) % 6;
                let col = WIRE_COLOURS[ci];
                let (a, b) = wire_line(i, z.boxes, z.wire.get(i).copied().unwrap_or(0), z.buttons);
                line(vec2(a.0, a.1), vec2(b.0, b.1), c3(col), 1.5);
            }
            for i in 0..z.boxes {
                let ci = z.colors.get(i).copied().unwrap_or(i % 6) % 6;
                let col = WIRE_COLOURS[ci];
                let done = z.lit.iter().position(|b| *b == i).map_or(false, |k| k < z.step);
                let c = if i == target {
                    WHITE
                } else if done {
                    darkened(col, 0.55)
                } else {
                    darkened(col, 0.25)
                };
                fill(rect(wire_box_rect(i, z.boxes)), c);
            }
            for b in 0..z.buttons {
                let r = wire_button_rect(b, z.buttons);
                button(r, &(b + 1).to_string(), 7.0, contains(r, mouse), false);
            }
        }
        Puzzle::Memory(m) => {
            let show = m.phase == Phase::Show;
            for i in 0..4 {
                let col = PAD_COLOURS[m.hues.get(i).copied().unwrap_or(i) % 4];
                let lit = m.lit == i as i32;
                let r = pad_rect(i);
                fill(rect(r), if lit { c3(col) } else { darkened(col, 0.55) });
                stroke(rect(r), if lit { WHITE } else { darkened(col, 0.2) }, 1.0);
                let hot = !show && contains(r, mouse);
                if hot {
                    stroke(rect(r), AMBER, 1.0);
                }
                vtext_center(&(i + 1).to_string(), r[0] + r[2] * 0.5, r[1] + (r[3] - 9.0) * 0.5 - 0.5, 9.0, INK);
            }
        }
        Puzzle::Pipes(p) => {
            let (_, done) = p.flow();
            let (cells, _) = p.flow();
            let (tile, _) = pipe_geometry(p.n);
            for r in 0..p.n {
                for c in 0..p.n {
                    let rc = pipe_rect(p.n, r, c);
                    let hot = contains(rc, mouse);
                    fill(rect(rc), if hot { Color::new(0.26, 0.25, 0.30, 1.0) } else { Color::new(0.16, 0.15, 0.19, 1.0) });
                    stroke(rect(rc), Color::new(0.35, 0.35, 0.4, 1.0), 1.0);
                    let wet = cells.contains(&(r, c));
                    let col = if wet { WET } else { DRY };
                    let mid = vec2(rc[0] + rc[2] * 0.5, rc[1] + rc[3] * 0.5);
                    let reach = rc[2] * 0.5;
                    for d in p.tiles[r][c].openings() {
                        let dir = [vec2(0.0, -1.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(-1.0, 0.0)][d % 4];
                        line(mid, mid + dir * reach, col, 3.0);
                    }
                    dot(mid, 2.5, col);
                }
            }
            vtext("IN >", PANEL_X + BODY_X, PANEL_Y + BODY_Y + 2.0 + tile * 0.5 - 5.0, 6.0, AMBER);
            vtext(
                "> OUT",
                PANEL_X + BODY_X + BODY_W - 24.0,
                PANEL_Y + BODY_Y + 2.0 + tile * (p.n as f32 - 0.5) - 5.0,
                6.0,
                if done { WET } else { AMBER },
            );
        }
        Puzzle::Hold => {
            let body = from_body(0.0, 0.0, BODY_W, BODY_H);
            vtext_center("SHE IS TRYING TO GET ON", body[0] + body[2] * 0.5, body[1] + 8.0, 8.0, INK);
            vtext_center("hold the door shut", body[0] + body[2] * 0.5, body[1] + 20.0, 8.0, INK);
            let bar = hold_bar_rect();
            fill(rect(bar), Color::new(0.2, 0.2, 0.25, 1.0));
            let t = (sim.held / sim.tun.hold_ms).clamp(0.0, 1.0) as f32;
            fill(Rect::new(bar[0], bar[1], bar[2] * t, bar[3]), AMBER);
            let hb = hold_button_rect();
            button(hb, "HOLD THE DOOR  SPACE", 7.0, contains(hb, mouse), holding);
        }
        Puzzle::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: R, b: R) -> bool {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    }

    fn inside_body(r: R) -> bool {
        let b = from_body(0.0, 0.0, BODY_W, BODY_H);
        r[0] >= b[0] && r[1] >= b[1] && r[0] + r[2] <= b[0] + b[2] + 0.01 && r[1] + r[3] <= b[1] + b[3] + 0.01
    }

    fn live(d: i32, stages: &[Stage], ms: f64) -> Sim {
        let mut s = Sim::new(d, 17, Some(8), Some(stages));
        s.step(ms, false);
        s
    }

    #[test]
    fn the_panel_is_centred_on_240_124_with_a_164_by_114_body() {
        assert_eq!((PANEL_X + PANEL_W * 0.5, PANEL_Y + PANEL_H * 0.5), (240.0, 124.0));
        assert_eq!((BODY_W, BODY_H), (164.0, 114.0));
        assert_eq!((PANEL_X, PANEL_Y), (154.0, 49.0));
    }

    #[test]
    fn wire_boxes_and_buttons_fit_the_body_and_do_not_overlap() {
        for n in 4..=6usize {
            for i in 0..n {
                assert!(inside_body(wire_box_rect(i, n)));
                assert!(inside_body(wire_button_rect(i, n)), "button {i} of {n}");
                for j in (i + 1)..n {
                    assert!(!overlap(wire_button_rect(i, n), wire_button_rect(j, n)), "{i} {j} of {n}");
                }
            }
        }
    }

    #[test]
    fn the_four_pads_are_a_two_by_two_block_inside_the_body() {
        for i in 0..4 {
            assert!(inside_body(pad_rect(i)));
            for j in (i + 1)..4 {
                assert!(!overlap(pad_rect(i), pad_rect(j)));
            }
        }
        assert_eq!(pad_rect(0)[1], pad_rect(1)[1]);
        assert!(pad_rect(2)[1] > pad_rect(0)[1]);
    }

    #[test]
    fn the_pipe_grid_fits_for_four_and_five() {
        assert_eq!(pipe_geometry(4).0, 27.0);
        assert_eq!(pipe_geometry(5).0, 22.0);
        for n in [4usize, 5] {
            for r in 0..n {
                for c in 0..n {
                    assert!(inside_body(pipe_rect(n, r, c)), "{n}: {r},{c}");
                }
            }
        }
    }

    #[test]
    fn a_click_on_a_wire_button_presses_it() {
        let s = live(1, &[Stage::Wires], 2000.0);
        let r = wire_button_rect(2, 4);
        assert_eq!(hit(&s, (r[0] + 3.0, r[1] + 3.0)), Some(PanelAction::Act(Act::Wire(2))));
        assert_eq!(hit(&s, (10.0, 10.0)), None);
    }

    #[test]
    fn pads_only_answer_after_the_show() {
        let mut s = live(2, &[Stage::Memory, Stage::Wires], 1800.0);
        let r = pad_rect(1);
        let p = (r[0] + 2.0, r[1] + 2.0);
        assert_eq!(hit(&s, p), None, "the show is running");
        s.step(20_000.0, false);
        assert_eq!(hit(&s, p), Some(PanelAction::Act(Act::Pad(1))));
    }

    #[test]
    fn a_click_on_a_pipe_turns_that_piece() {
        let s = live(3, &[Stage::Pipes, Stage::Wires, Stage::Memory], 1800.0);
        let r = pipe_rect(4, 2, 3);
        assert_eq!(hit(&s, (r[0] + 5.0, r[1] + 5.0)), Some(PanelAction::Act(Act::Pipe(2, 3))));
    }

    #[test]
    fn the_hold_button_answers_on_the_door_stage_only() {
        let mut s = Sim::new(4, 3, Some(8), Some(&[Stage::Wires, Stage::Memory, Stage::Pipes, Stage::Hold]));
        s.admin_goto_stage(4);
        let r = hold_button_rect();
        assert_eq!(hit(&s, (r[0] + 4.0, r[1] + 4.0)), Some(PanelAction::HoldDown));
        let w = live(1, &[Stage::Wires], 2000.0);
        assert_eq!(hit(&w, (r[0] + 4.0, r[1] + 4.0)), None);
    }

    #[test]
    fn nothing_answers_during_a_ride_or_after_the_end() {
        let s = Sim::new(1, 3, None, Some(&[Stage::Wires]));
        let r = wire_button_rect(0, 4);
        assert_eq!(hit(&s, (r[0] + 2.0, r[1] + 2.0)), None, "the first ride is still running");
        assert_eq!(ride_text(&s), Some("DESCENDING\nFLOOR 1".to_string()));
        let mut e = live(1, &[Stage::Wires], 2000.0);
        e.over = Over::Caught;
        assert_eq!(hit(&e, (r[0] + 2.0, r[1] + 2.0)), None);
        assert_eq!(labels(&e), (String::new(), String::new()));
    }

    #[test]
    fn titles_and_sub_lines_are_the_godot_texts() {
        let w = live(1, &[Stage::Wires], 2000.0);
        let (t, s) = labels(&w);
        assert_eq!(t, "FUSE BOXES");
        assert!(s.starts_with("0 / "));
        let p = live(3, &[Stage::Pipes, Stage::Wires, Stage::Memory], 1800.0);
        let (t, s) = labels(&p);
        assert!(t.starts_with("PIPEWORK"));
        assert!(t.ends_with("SHE SLOWS HERE"));
        assert!(s.contains("CONNECT IN TO OUT"));
        let m = live(2, &[Stage::Memory, Stage::Wires], 1800.0);
        assert_eq!(labels(&m).1, "WATCH");
    }
}
