//! Phone touch controls (Godot game/hud/touch_controls.gd; TODO "Port phone touch controls").
//!
//! Native phones (Android, iOS) turn them on by themselves. `--touch` turns them on for a desktop test, where the left
//! mouse button stands in for a finger. Each frame the fingers are read and turned into what the keyboard would press,
//! so the game sees the same inputs: a button holds its binding (the bindings answer `down` and `pressed` from here,
//! app/bindings.rs), the D-pad gives the move axes at full strength (diagonals by two fingers or a slide), a double tap
//! on an arrow sprints that way while the finger stays down, and a drag on the free part of the screen is the look.
//! The two hotbar keys have no binding: the < and > buttons set the mouse wheel for the frame instead.
//!
//! While a menu is open (pause, map, chat, the update screen) the touches belong to the menu: `frame` releases
//! everything. Not in this port yet: the floating stick (Settings.touch_move "stick"), the look smoothing and
//! acceleration options, the notch insets, the CHAT button (typing needs a soft keyboard, which the game does not
//! open yet, so a phone could not leave the chat box), and the inventory screen (the BAG button presses its binding; the
//! screen is not ported, so nothing answers it yet).
//!
//! The gesture logic is pure (`Gesture::update` takes the touches as data) and is what the tests cover.

use macroquad::input::{is_mouse_button_down, mouse_position, Touch, TouchPhase};
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// The smallest touch target as a share of the screen height (Godot: 26 of its 270 canvas units, about 48 dp on a phone
/// held landscape). Android and iOS report a device scale of 1 and raw pixel touches, so the height is what scales.
const MIN_TARGET_SHARE: f32 = 26.0 / 270.0;
/// The smallest touch target on a screen that is not a phone (pixels at a device scale of 1).
const MIN_TARGET_DP: f32 = 48.0;
/// The button unit as a share of the screen height (Godot: 13 % of the screen height).
const UNIT_SHARE: f32 = 0.13;
/// The gap between buttons, as a share of the unit (Godot: 0.16).
const GAP_SHARE: f32 = 0.16;
/// A second press on the same arrow within this many seconds of letting go sprints.
const DOUBLE_TAP_SECONDS: f32 = 0.3;
/// A drag across the screen's short side turns this far at look speed 1 (degrees; Godot LOOK_DEGREES_PER_SHORT_SIDE).
const LOOK_DEGREES_PER_SHORT_SIDE: f32 = 180.0;
/// The finger id of the desktop stand-in (`--touch` with the mouse).
const MOUSE_FINGER: u64 = u64::MAX;

/// The on-screen buttons, in Godot's order (touch_controls.gd BUTTONS, plus the two wheel buttons).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Button {
    Interact,
    Jump,
    Crouch,
    Light,
    Tool,
    Prev,
    Next,
    Bag,
    Map,
    Quick,
    Pause,
}

impl Button {
    pub const ALL: [Button; 11] = [
        Button::Interact,
        Button::Jump,
        Button::Crouch,
        Button::Light,
        Button::Tool,
        Button::Prev,
        Button::Next,
        Button::Bag,
        Button::Map,
        Button::Quick,
        Button::Pause,
    ];

    /// The binding this button presses (data/controls.json ids). The wheel buttons press none.
    pub fn action(self) -> Option<&'static str> {
        Some(match self {
            Button::Interact => "interact",
            Button::Jump => "jump",
            Button::Crouch => "crouch",
            Button::Light => "flashlight",
            Button::Tool => "use_item",
            Button::Bag => "inventory",
            Button::Map => "map",
            Button::Quick => "quick_chat",
            Button::Pause => "pause",
            Button::Prev | Button::Next => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Button::Interact => "USE",
            Button::Jump => "JUMP",
            Button::Crouch => "DUCK",
            Button::Light => "LIGHT",
            Button::Tool => "TOOL",
            Button::Prev => "<",
            Button::Next => ">",
            Button::Bag => "BAG",
            Button::Map => "MAP",
            Button::Quick => "QUICK",
            Button::Pause => "II",
        }
    }
}

/// Where everything sits on a screen of `view` pixels, with `dpi` the device scale (1.0 on a desktop).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub buttons: Vec<(Button, Rect)>,
    /// The D-pad: three by three cells of `cell` pixels, the arrows on the edges.
    pub pad: Rect,
    pub cell: f32,
    /// The hotbar's place when the HUD has not told us (Godot's fallback): the touches stay off it.
    pub hotbar: Rect,
    pub unit: f32,
    /// The screen's short side in pixels (the look scale).
    pub short: f32,
}

impl Layout {
    pub fn new(view: Vec2, dpi: f32) -> Layout {
        let min = (view.y * MIN_TARGET_SHARE).max(MIN_TARGET_DP * dpi.max(1.0));
        let unit = (view.y * UNIT_SHARE).max(min);
        let gap = unit * GAP_SHARE;
        let mut buttons = Vec::with_capacity(Button::ALL.len());
        // the action cluster in the bottom-right corner: (button, column, row) counted from the right and the bottom
        for (button, col, row) in [
            (Button::Interact, 0, 0),
            (Button::Jump, 1, 0),
            (Button::Tool, 0, 1),
            (Button::Crouch, 1, 1),
            (Button::Light, 2, 1),
        ] {
            let x = view.x - (col as f32 + 1.0) * (unit + gap);
            let y = view.y - (row as f32 + 1.0) * (unit + gap);
            buttons.push((button, Rect::new(x, y, unit, unit)));
        }
        let cell = unit;
        let pad = Rect::new(gap * 2.0, view.y - cell * 3.0 - gap, cell * 3.0, cell * 3.0);
        let hotbar = Rect::new(view.x * 0.5 - view.x * 0.19, view.y - min * 1.6, view.x * 0.38, min * 1.6);
        let arrow_y = hotbar.y + hotbar.h - min;
        buttons.push((Button::Prev, Rect::new(hotbar.x - min - gap * 0.5, arrow_y, min, min)));
        buttons.push((Button::Next, Rect::new(hotbar.x + hotbar.w + gap * 0.5, arrow_y, min, min)));
        // the menu buttons along the top-left
        for (i, button) in [Button::Pause, Button::Map, Button::Bag, Button::Quick].into_iter().enumerate() {
            let x = (view.x * 0.08).max(gap) + i as f32 * (min + gap);
            buttons.push((button, Rect::new(x, gap, min, min)));
        }
        Layout { buttons, pad, cell, hotbar, unit, short: view.x.min(view.y) }
    }

    pub fn button_at(&self, at: Vec2) -> Option<Button> {
        self.buttons.iter().find(|(_, r)| r.contains(at)).map(|(b, _)| *b)
    }

    #[cfg(test)]
    pub fn button_rect(&self, button: Button) -> Option<Rect> {
        self.buttons.iter().find(|(b, _)| *b == button).map(|(_, r)| *r)
    }
}

/// The D-pad direction of a point in pad-local pixels (Godot pad_direction): an edge or a corner gives the arrow or the
/// diagonal between two arrows, the centre cell and anything outside the pad give nothing.
pub fn pad_direction(local: Vec2, cell: f32) -> Vec2 {
    let size = cell * 3.0;
    if local.x < 0.0 || local.y < 0.0 || local.x > size || local.y > size {
        return Vec2::ZERO;
    }
    let col = ((local.x / cell) as i32).clamp(0, 2) - 1;
    let row = ((local.y / cell) as i32).clamp(0, 2) - 1;
    vec2(col as f32, row as f32)
}

/// -1, 0 or 1 (`f32::signum` gives 1 for zero, which would move a finger that is still).
fn sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// A look drag in pixels as radians: x turns right, y tilts down, on the mouse's sign convention
/// (`config::look::px_to_radians`). A drag across the short side turns 180 degrees at speed 1.
pub fn look_turn(drag: Vec2, short_side: f32) -> Vec2 {
    drag * (LOOK_DEGREES_PER_SHORT_SIDE.to_radians() / short_side.max(1.0))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Button(Button),
    /// A finger on the pad: its arrow (zero in the centre) and whether it started a double tap.
    Pad { dir: Vec2, sprint: bool },
    Look,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Finger {
    role: Role,
    pos: Vec2,
}

/// What the touches ask of the game this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Output {
    /// Move axes at full strength (-1, 0 or 1), as the keyboard gives them.
    pub fwd: f32,
    pub strafe: f32,
    pub sprint: bool,
    /// The look drag this frame, radians (x right, y down).
    pub look: Vec2,
    /// The hotbar: -1 for <, 1 for > (the mouse wheel's sign), 0 for none. Set on the frame a wheel button is pressed.
    pub wheel: i32,
}

/// What owns the screen this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The run: every control works.
    Play,
    /// The full map is open: only the MAP and PAUSE buttons stay (to close it, or to pause), the rest of the screen is the map's.
    Map,
    /// A menu, the chat box, the update screen or the report owns every touch.
    Menu,
}

/// The gesture state of every finger, kept between frames.
#[derive(Clone, Debug, Default)]
pub struct Gesture {
    fingers: HashMap<u64, Finger>,
    /// Fingers that are down but not ours: they began on the hotbar, under a menu or the map, or as a second look
    /// finger. The platform keeps one event per finger a frame, so a Moved for an id we did not track would otherwise
    /// read as a new press; these never do.
    ignored: HashSet<u64>,
    look_id: Option<u64>,
    last_pad_release: (Vec2, f32),
    /// Buttons down now, and the ones pressed this frame (the bindings' `down` and `pressed`).
    held: HashSet<&'static str>,
    tapped: HashSet<&'static str>,
    /// The wheel button pressed this frame, if any (set in `press`, cleared each frame).
    wheel: i32,
    out: Output,
}

impl Gesture {
    /// One frame. `touches` are the fingers now (any phase), `now` seconds, `focus` what owns the screen.
    ///
    /// macroquad keeps one event per finger between frames, so Started followed by Moved reads as Moved, and a tap
    /// (Started then Ended inside one frame) reads as Ended alone. A finger first seen moving is therefore a new
    /// press, and one first seen ended is a tap that is held for this frame and let go after it.
    pub fn update(&mut self, now: f32, touches: &[Touch], layout: &Layout, focus: Focus) {
        let previously_held = std::mem::take(&mut self.held);
        self.tapped.clear();
        self.wheel = 0;
        self.out = Output::default();
        let live = |phase: TouchPhase| !matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        if focus == Focus::Menu {
            // a menu owns every touch: nothing is held, and the fingers down now are not ours when it closes
            self.fingers.clear();
            self.look_id = None;
            self.ignored = touches.iter().filter(|t| live(t.phase)).map(|t| t.id).collect();
            return;
        }
        if focus == Focus::Map {
            // only MAP and PAUSE stay: every other finger is let go, and not picked up again mid-gesture
            let dropped: Vec<u64> = self
                .fingers
                .iter()
                .filter(|(_, f)| !matches!(f.role, Role::Button(Button::Map | Button::Pause)))
                .map(|(id, _)| *id)
                .collect();
            for id in dropped {
                self.release(now, id);
                self.ignored.insert(id);
            }
        }
        let mut ordered: Vec<&Touch> = touches.iter().collect();
        ordered.sort_by_key(|t| t.id);
        let mut seen = HashSet::new();
        // fingers that started and ended inside one frame: held for this frame, let go after the outputs are read
        let mut taps: Vec<u64> = vec![];
        for t in ordered {
            seen.insert(t.id);
            match t.phase {
                TouchPhase::Started => {
                    self.ignored.remove(&t.id);
                    self.press(now, t.id, t.position, layout, focus);
                }
                TouchPhase::Moved | TouchPhase::Stationary => {
                    if self.fingers.contains_key(&t.id) {
                        self.move_finger(t.id, t.position, layout);
                    } else if !self.ignored.contains(&t.id) {
                        self.press(now, t.id, t.position, layout, focus);
                    }
                }
                TouchPhase::Ended => {
                    if self.fingers.contains_key(&t.id) {
                        self.release(now, t.id);
                    } else if !self.ignored.remove(&t.id) {
                        self.press(now, t.id, t.position, layout, focus);
                        if self.fingers.contains_key(&t.id) {
                            taps.push(t.id);
                        }
                    }
                }
                TouchPhase::Cancelled => {
                    self.ignored.remove(&t.id);
                    self.release(now, t.id);
                }
            }
        }
        // a finger the platform no longer lists is up
        let gone: Vec<u64> = self.fingers.keys().copied().filter(|id| !seen.contains(id)).collect();
        for id in gone {
            self.release(now, id);
        }
        self.ignored.retain(|id| seen.contains(id));
        // the outputs from every finger together
        let mut axes = Vec2::ZERO;
        let mut sprint = false;
        for finger in self.fingers.values() {
            match finger.role {
                Role::Pad { dir, sprint: s } => {
                    axes += dir;
                    sprint |= s;
                }
                Role::Button(b) => {
                    if let Some(action) = b.action() {
                        self.held.insert(action);
                    }
                }
                Role::Look => {}
            }
        }
        // up on the pad is forward (+1), right is strafe right (+1), like the keyboard's move_up and move_right
        self.out.fwd = -sign(axes.y);
        self.out.strafe = sign(axes.x);
        self.out.sprint = sprint && axes != Vec2::ZERO;
        self.tapped = self.held.difference(&previously_held).copied().collect();
        self.out.wheel = self.wheel;
        for id in taps {
            self.release(now, id);
        }
    }

    fn press(&mut self, now: f32, id: u64, at: Vec2, layout: &Layout, focus: Focus) {
        // the platform reuses a finger id: a new press on an id still tracked ends the old finger first
        if self.fingers.contains_key(&id) {
            self.release(now, id);
        }
        let button = layout.button_at(at);
        if focus == Focus::Map && !matches!(button, Some(Button::Map | Button::Pause)) {
            self.ignored.insert(id);
            return;
        }
        if let Some(button) = button {
            match button {
                Button::Prev => self.wheel = -1,
                Button::Next => self.wheel = 1,
                _ => {}
            }
            self.fingers.insert(id, Finger { role: Role::Button(button), pos: at });
        } else if layout.hotbar.contains(at) {
            // the hotbar is the HUD's: a touch on it does nothing here
            self.ignored.insert(id);
        } else if layout.pad.contains(at) {
            let dir = pad_direction(at - layout.pad.point(), layout.cell);
            let (last_dir, last_at) = self.last_pad_release;
            let sprint = dir != Vec2::ZERO && dir == last_dir && now - last_at <= DOUBLE_TAP_SECONDS;
            self.fingers.insert(id, Finger { role: Role::Pad { dir, sprint }, pos: at });
        } else if self.look_id.is_none() {
            self.look_id = Some(id);
            self.fingers.insert(id, Finger { role: Role::Look, pos: at });
        } else {
            // a second finger on the free screen: the first one looks
            self.ignored.insert(id);
        }
    }

    fn move_finger(&mut self, id: u64, at: Vec2, layout: &Layout) {
        let Some(finger) = self.fingers.get_mut(&id) else { return };
        match finger.role {
            Role::Pad { dir, sprint } => {
                // a slide keeps the way it was when it leaves the pad (Godot _pad_slide)
                let mut next = pad_direction(at - layout.pad.point(), layout.cell);
                if next == Vec2::ZERO && !layout.pad.contains(at) {
                    next = dir;
                }
                finger.role = Role::Pad { dir: next, sprint };
            }
            Role::Look => {
                let delta = at - finger.pos;
                self.out.look += look_turn(delta, layout.short);
            }
            Role::Button(_) => {}
        }
        finger.pos = at;
    }

    fn release(&mut self, now: f32, id: u64) {
        let Some(finger) = self.fingers.remove(&id) else { return };
        match finger.role {
            Role::Pad { dir, .. } => self.last_pad_release = (dir, now),
            Role::Look => {
                if self.look_id == Some(id) {
                    self.look_id = None;
                }
            }
            Role::Button(_) => {}
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Default)]
struct State {
    enabled: bool,
    /// `--touch` on a desktop: the mouse stands in for a finger.
    mouse_stand_in: bool,
    mouse_down: bool,
    mouse_last: Vec2,
    gesture: Gesture,
    layout: Option<Layout>,
    /// The output this frame, copied out by `take_output`.
    out: Output,
}

/// Whether touch controls are on for this run: a phone, or `--touch` on a desktop.
pub fn enabled() -> bool {
    cfg!(any(target_os = "android", target_os = "ios")) || std::env::args().any(|a| a == "--touch")
}

/// Turns the touch controls on or off for the run (the desktop test sets it from `--touch`).
pub fn init() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.enabled = enabled();
        s.mouse_stand_in = s.enabled && !cfg!(any(target_os = "android", target_os = "ios"));
    });
}

/// Whether the touch controls are on (the tool button uses the same path as the grab on a desktop).
pub fn active() -> bool {
    STATE.with(|s| s.borrow().enabled)
}

/// The action is held by a touch button this frame (the bindings' `down`).
pub fn held(action: &str) -> bool {
    STATE.with(|s| s.borrow().gesture.held.contains(action))
}

/// The action was pressed by a touch button this frame (the bindings' `pressed`).
pub fn tapped(action: &str) -> bool {
    STATE.with(|s| s.borrow().gesture.tapped.contains(action))
}

/// This frame's touch output for the controls (zero while the touch controls are off).
pub fn output() -> Output {
    STATE.with(|s| s.borrow().out)
}

/// Reads the touches for this frame. Call once a frame, before the controls are read. `view` is the screen in pixels,
/// `dpi` the device scale, `now` seconds, `focus` what owns the screen.
pub fn frame(view: Vec2, dpi: f32, now: f32, focus: Focus) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.enabled {
            return;
        }
        let mut touches = macroquad::input::touches();
        if touches.is_empty() && s.mouse_stand_in {
            let down = is_mouse_button_down(MouseButton::Left);
            let at = mouse_position().into();
            let phase = match (s.mouse_down, down) {
                (false, true) => Some(TouchPhase::Started),
                (true, true) => Some(TouchPhase::Moved),
                (true, false) => Some(TouchPhase::Ended),
                (false, false) => None,
            };
            if let Some(phase) = phase {
                let position = if phase == TouchPhase::Ended { s.mouse_last } else { at };
                touches.push(Touch { id: MOUSE_FINGER, phase, position });
            }
            s.mouse_down = down;
            s.mouse_last = at;
        }
        let layout = Layout::new(view, dpi);
        s.gesture.update(now, &touches, &layout, focus);
        s.out = s.gesture.out;
        s.layout = Some(layout);
    });
}

/// Draws the controls over the run: the D-pad with the arrows the finger holds lit, and every button, lit while held.
/// Nothing is drawn while `show` is false or the controls are off.
pub fn draw(show: bool) {
    STATE.with(|s| {
        let s = s.borrow();
        if !s.enabled || !show {
            return;
        }
        let Some(layout) = &s.layout else { return };
        let idle = Color::new(1.0, 1.0, 1.0, 0.16);
        let lit = Color::new(1.0, 1.0, 1.0, 0.45);
        let text = Color::new(1.0, 1.0, 1.0, 0.85);
        let cell = layout.cell;
        // the D-pad: the arrow cells, the centre left empty
        let arrows = [(0, -1, "^"), (0, 1, "v"), (-1, 0, "<"), (1, 0, ">")];
        for (dx, dy, glyph) in arrows {
            let x = layout.pad.x + (dx + 1) as f32 * cell;
            let y = layout.pad.y + (dy + 1) as f32 * cell;
            let on = (dx != 0 && dx as f32 == s.out.strafe) || (dy != 0 && dy as f32 == -s.out.fwd);
            draw_rectangle(x + 2.0, y + 2.0, cell - 4.0, cell - 4.0, if on { lit } else { idle });
            let size = (cell * 0.45).max(10.0) as u16;
            let m = measure_text(glyph, None, size, 1.0);
            draw_text(glyph, x + cell * 0.5 - m.width * 0.5, y + cell * 0.5 + m.height * 0.5, size as f32, text);
        }
        // the buttons
        let held = &s.gesture.held;
        for (button, rect) in &layout.buttons {
            let is_held = match button.action() {
                Some(action) => held.contains(action),
                None => match button {
                    Button::Prev => s.out.wheel < 0,
                    Button::Next => s.out.wheel > 0,
                    _ => false,
                },
            };
            draw_rectangle(rect.x, rect.y, rect.w, rect.h, if is_held { lit } else { idle });
            let size = (rect.h * 0.3).max(8.0) as u16;
            let label = button.label();
            let m = measure_text(label, None, size, 1.0);
            draw_text(label, rect.x + rect.w * 0.5 - m.width * 0.5, rect.y + rect.h * 0.5 + m.height * 0.5, size as f32, text);
        }
    });
}

/// The look and the wheel of the frame, once touches are read (see `read_controls`).
pub fn apply(c: &mut crate::game::Controls) {
    let out = output();
    if out == Output::default() {
        return;
    }
    c.fwd += out.fwd;
    c.strafe += out.strafe;
    c.sprint |= out.sprint;
    c.look += out.look;
    if out.wheel != 0 {
        c.wheel = out.wheel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: TouchPhase, x: f32, y: f32) -> Touch {
        Touch { id, phase, position: vec2(x, y) }
    }

    fn screen() -> Layout {
        Layout::new(vec2(1920.0, 1080.0), 1.0)
    }

    /// The centre of a rect, where a finger lands on a button.
    fn centre(r: Rect) -> (f32, f32) {
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    }

    #[test]
    fn the_pad_gives_arrows_diagonals_and_nothing_in_the_centre_or_outside() {
        let cell = 100.0;
        assert_eq!(pad_direction(vec2(150.0, 20.0), cell), vec2(0.0, -1.0), "top edge is up");
        assert_eq!(pad_direction(vec2(150.0, 280.0), cell), vec2(0.0, 1.0), "bottom edge is down");
        assert_eq!(pad_direction(vec2(20.0, 150.0), cell), vec2(-1.0, 0.0), "left edge is left");
        assert_eq!(pad_direction(vec2(280.0, 150.0), cell), vec2(1.0, 0.0), "right edge is right");
        assert_eq!(pad_direction(vec2(20.0, 20.0), cell), vec2(-1.0, -1.0), "a corner is a diagonal");
        assert_eq!(pad_direction(vec2(150.0, 150.0), cell), Vec2::ZERO, "the centre cell is nothing");
        assert_eq!(pad_direction(vec2(-1.0, 50.0), cell), Vec2::ZERO);
        assert_eq!(pad_direction(vec2(301.0, 50.0), cell), Vec2::ZERO);
    }

    #[test]
    fn a_look_drag_across_the_short_side_turns_180_degrees_and_keeps_the_mouse_sign() {
        let turn = look_turn(vec2(1080.0, 0.0), 1080.0);
        assert!((turn.x - std::f32::consts::PI).abs() < 1e-5, "{turn:?}");
        assert_eq!(look_turn(vec2(0.0, 0.0), 1080.0), Vec2::ZERO);
        // dragging down tilts down (y positive), like the mouse path
        assert!(look_turn(vec2(0.0, 10.0), 1080.0).y > 0.0);
        assert!(look_turn(vec2(-10.0, 0.0), 1080.0).x < 0.0, "left turns left");
    }

    #[test]
    fn the_buttons_do_not_overlap_each_other_or_the_pad() {
        let l = screen();
        for (i, (a, ra)) in l.buttons.iter().enumerate() {
            assert!(!l.pad.overlaps(ra), "{a:?} sits on the D-pad");
            for (b, rb) in l.buttons.iter().skip(i + 1) {
                assert!(!ra.overlaps(rb), "{a:?} and {b:?} overlap");
            }
        }
        assert_eq!(l.buttons.len(), Button::ALL.len(), "every button has a place");
        for b in Button::ALL {
            assert!(l.button_rect(b).is_some(), "{b:?}");
        }
    }

    #[test]
    fn a_button_holds_its_action_while_the_finger_is_down_and_taps_once() {
        let l = screen();
        let jump = l.button_rect(Button::Jump).unwrap();
        let (x, y) = centre(jump);
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, x, y)], &l, Focus::Play);
        assert!(g.held.contains("jump") && g.tapped.contains("jump"));
        g.update(0.1, &[touch(1, TouchPhase::Stationary, x, y)], &l, Focus::Play);
        assert!(g.held.contains("jump") && !g.tapped.contains("jump"), "held, not pressed again");
        g.update(0.2, &[touch(1, TouchPhase::Ended, x, y)], &l, Focus::Play);
        assert!(g.held.is_empty() && g.tapped.is_empty(), "let go releases it");
    }

    #[test]
    fn the_pad_moves_at_full_strength_and_a_slide_changes_the_way() {
        let l = screen();
        let p = l.pad.point();
        let c = l.cell;
        // a finger on the top arrow: forward at full strength
        let up = (p.x + c * 1.5, p.y + c * 0.5);
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, up.0, up.1)], &l, Focus::Play);
        assert_eq!((g.out.fwd, g.out.strafe), (1.0, 0.0), "up on the pad is forward (+1)");
        // slide onto the right arrow: strafe right
        let right = (p.x + c * 2.5, p.y + c * 1.5);
        g.update(0.1, &[touch(1, TouchPhase::Moved, right.0, right.1)], &l, Focus::Play);
        assert_eq!((g.out.fwd, g.out.strafe), (0.0, 1.0));
        // a second finger on the left arrow adds to it: a diagonal of two fingers
        let left = (p.x + c * 0.5, p.y + c * 1.5);
        g.update(0.2, &[touch(1, TouchPhase::Moved, right.0, right.1), touch(2, TouchPhase::Started, left.0, left.1)], &l, Focus::Play);
        assert_eq!(g.out.strafe, 0.0, "right and left cancel");
    }

    #[test]
    fn a_double_tap_on_an_arrow_sprints_while_held() {
        let l = screen();
        let p = l.pad.point();
        let c = l.cell;
        let up = (p.x + c * 1.5, p.y + c * 0.5);
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, up.0, up.1)], &l, Focus::Play);
        assert!(!g.out.sprint, "a first press walks");
        g.update(0.05, &[touch(1, TouchPhase::Ended, up.0, up.1)], &l, Focus::Play);
        g.update(0.15, &[touch(2, TouchPhase::Started, up.0, up.1)], &l, Focus::Play);
        assert!(g.out.sprint, "the second press within 0.3 s sprints");
        g.update(0.2, &[touch(2, TouchPhase::Ended, up.0, up.1)], &l, Focus::Play);
        g.update(1.0, &[touch(3, TouchPhase::Started, up.0, up.1)], &l, Focus::Play);
        assert!(!g.out.sprint, "a late press walks again");
    }

    #[test]
    fn a_drag_on_the_free_screen_is_the_look_and_a_finger_on_the_hotbar_does_nothing() {
        let l = screen();
        let mut g = Gesture::default();
        let free = (900.0, 400.0);
        g.update(0.0, &[touch(1, TouchPhase::Started, free.0, free.1)], &l, Focus::Play);
        g.update(0.1, &[touch(1, TouchPhase::Moved, free.0 + 108.0, free.1)], &l, Focus::Play);
        assert!(g.out.look.x > 0.0 && g.out.look.y.abs() < 1e-6, "{:?}", g.out.look);
        assert_eq!((g.out.fwd, g.out.strafe), (0.0, 0.0), "the look moves nothing");
        // a finger on the hotbar: no look, no move
        let hot = (l.hotbar.x + l.hotbar.w * 0.5, l.hotbar.y + l.hotbar.h * 0.5);
        let mut h = Gesture::default();
        h.update(0.0, &[touch(7, TouchPhase::Started, hot.0, hot.1)], &l, Focus::Play);
        h.update(0.1, &[touch(7, TouchPhase::Moved, hot.0 + 200.0, hot.1)], &l, Focus::Play);
        assert_eq!(h.out, Output::default());
        assert!(h.held.is_empty());
    }

    #[test]
    fn the_wheel_buttons_set_the_wheel_for_one_frame() {
        let l = screen();
        let next = centre(l.button_rect(Button::Next).unwrap());
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, next.0, next.1)], &l, Focus::Play);
        assert_eq!(g.out.wheel, 1, "> is the next slot");
        g.update(0.1, &[touch(1, TouchPhase::Stationary, next.0, next.1)], &l, Focus::Play);
        assert_eq!(g.out.wheel, 0, "a held button does not repeat");
        g.update(0.2, &[touch(1, TouchPhase::Ended, next.0, next.1)], &l, Focus::Play);
        let prev = centre(l.button_rect(Button::Prev).unwrap());
        g.update(0.3, &[touch(2, TouchPhase::Started, prev.0, prev.1)], &l, Focus::Play);
        assert_eq!(g.out.wheel, -1, "< is the previous slot");
    }

    #[test]
    fn a_menu_takes_every_touch_and_releases_what_was_held() {
        let l = screen();
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Play);
        assert!(g.held.contains("jump"));
        g.update(0.1, &[touch(1, TouchPhase::Stationary, jump.0, jump.1)], &l, Focus::Menu);
        assert!(g.held.is_empty(), "a menu releases the button");
        // while blocked a new touch is the menu's
        g.update(0.2, &[touch(2, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Menu);
        assert!(g.held.is_empty() && g.fingers.is_empty());
    }

    #[test]
    fn a_finger_the_platform_drops_is_released() {
        let l = screen();
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Play);
        g.update(0.1, &[], &l, Focus::Play);
        assert!(g.held.is_empty() && g.fingers.is_empty(), "a vanished finger is up");
    }

    #[test]
    fn a_second_finger_cannot_steal_the_look() {
        let l = screen();
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, 900.0, 400.0), touch(2, TouchPhase::Started, 1000.0, 400.0)], &l, Focus::Play);
        assert_eq!(g.look_id, Some(1));
        g.update(0.1, &[touch(1, TouchPhase::Moved, 900.0, 400.0), touch(2, TouchPhase::Moved, 1100.0, 400.0)], &l, Focus::Play);
        assert_eq!(g.out.look, Vec2::ZERO, "only the first finger looks");
    }

    #[test]
    fn a_finger_first_seen_moving_or_ended_is_still_a_press_or_a_tap() {
        let l = screen();
        // the platform keeps one event per finger a frame: Started then Moved reads as Moved
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Moved, jump.0, jump.1)], &l, Focus::Play);
        assert!(g.held.contains("jump") && g.tapped.contains("jump"), "a finger first seen moving presses the button under it");
        // on the pad it is a direction, on the free screen the look
        let up = (l.pad.x + l.cell * 1.5, l.pad.y + l.cell * 0.5);
        let mut p = Gesture::default();
        p.update(0.0, &[touch(1, TouchPhase::Moved, up.0, up.1)], &l, Focus::Play);
        assert_eq!(p.out.fwd, 1.0);
        // a tap that started and ended inside one frame reads as Ended alone: pressed and held for that frame only
        let mut t = Gesture::default();
        t.update(0.0, &[touch(1, TouchPhase::Ended, jump.0, jump.1)], &l, Focus::Play);
        assert!(t.held.contains("jump") && t.tapped.contains("jump"), "a one-frame tap still presses");
        t.update(0.1, &[], &l, Focus::Play);
        assert!(t.held.is_empty() && t.fingers.is_empty(), "and is let go after it");
    }

    #[test]
    fn a_reused_finger_id_ends_the_old_finger_first() {
        let l = screen();
        let mut g = Gesture::default();
        g.update(0.0, &[touch(0, TouchPhase::Started, 900.0, 400.0)], &l, Focus::Play);
        assert_eq!(g.look_id, Some(0));
        // the look finger lifts and a new finger takes id 0 on JUMP inside one frame: the frame sees Started
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        g.update(0.1, &[touch(0, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Play);
        assert_eq!(g.look_id, None, "the old look finger is gone");
        g.update(0.2, &[touch(0, TouchPhase::Ended, jump.0, jump.1)], &l, Focus::Play);
        // a free-screen touch can look again
        g.update(0.3, &[touch(5, TouchPhase::Started, 900.0, 400.0)], &l, Focus::Play);
        g.update(0.4, &[touch(5, TouchPhase::Moved, 1000.0, 400.0)], &l, Focus::Play);
        assert!(g.out.look.x > 0.0, "the look still works after an id was reused");
    }

    #[test]
    fn a_finger_that_began_on_the_hotbar_or_under_a_menu_never_turns_into_a_press() {
        let l = screen();
        let hot = (l.hotbar.x + l.hotbar.w * 0.5, l.hotbar.y + l.hotbar.h * 0.5);
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        let mut g = Gesture::default();
        g.update(0.0, &[touch(1, TouchPhase::Started, hot.0, hot.1)], &l, Focus::Play);
        // it slides onto JUMP: still not ours
        g.update(0.1, &[touch(1, TouchPhase::Moved, jump.0, jump.1)], &l, Focus::Play);
        assert!(g.held.is_empty() && g.fingers.is_empty(), "a hotbar finger does not press what it slides over");
        // a finger down under a menu, still down when the menu closes
        let mut m = Gesture::default();
        m.update(0.0, &[touch(2, TouchPhase::Stationary, jump.0, jump.1)], &l, Focus::Menu);
        m.update(0.1, &[touch(2, TouchPhase::Stationary, jump.0, jump.1)], &l, Focus::Play);
        assert!(m.held.is_empty(), "a menu's finger is not picked up when the menu closes");
        // lifting it and touching again works
        m.update(0.2, &[touch(2, TouchPhase::Ended, jump.0, jump.1)], &l, Focus::Play);
        m.update(0.3, &[touch(3, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Play);
        assert!(m.held.contains("jump"));
    }

    #[test]
    fn with_the_map_open_only_map_and_pause_answer_and_the_map_button_can_close_it() {
        let l = screen();
        let map = centre(l.button_rect(Button::Map).unwrap());
        let pause = centre(l.button_rect(Button::Pause).unwrap());
        let jump = centre(l.button_rect(Button::Jump).unwrap());
        let up = (l.pad.x + l.cell * 1.5, l.pad.y + l.cell * 0.5);
        let mut g = Gesture::default();
        // a pad finger held when the map opens is let go
        g.update(0.0, &[touch(1, TouchPhase::Started, up.0, up.1)], &l, Focus::Play);
        assert_eq!(g.out.fwd, 1.0);
        g.update(0.1, &[touch(1, TouchPhase::Stationary, up.0, up.1)], &l, Focus::Map);
        assert_eq!((g.out.fwd, g.out.strafe), (0.0, 0.0), "the map takes the pad");
        // JUMP and the free screen do nothing; MAP and PAUSE do
        g.update(0.2, &[touch(2, TouchPhase::Started, jump.0, jump.1)], &l, Focus::Map);
        assert!(g.held.is_empty(), "JUMP does nothing on the map");
        g.update(0.3, &[touch(3, TouchPhase::Started, map.0, map.1)], &l, Focus::Map);
        assert!(g.tapped.contains("map"), "MAP closes the map");
        g.update(0.4, &[touch(3, TouchPhase::Ended, map.0, map.1)], &l, Focus::Map);
        g.update(0.5, &[touch(4, TouchPhase::Started, pause.0, pause.1)], &l, Focus::Map);
        assert!(g.tapped.contains("pause"), "PAUSE answers on the map");
    }

    #[test]
    fn the_buttons_scale_with_the_screen_height_when_the_device_scale_is_one() {
        // a phone reports a scale of 1 and raw pixels: 2400 x 1080 still gets targets of about 8.5 % of the height
        let phone = Layout::new(vec2(2400.0, 1080.0), 1.0);
        assert!(phone.unit >= 1080.0 * 0.13 - 0.1);
        for (b, r) in &phone.buttons {
            assert!(r.w >= 1080.0 * MIN_TARGET_SHARE - 0.1 && r.h >= 1080.0 * MIN_TARGET_SHARE - 0.1, "{b:?} is {}x{}", r.w, r.h);
        }
    }
}
