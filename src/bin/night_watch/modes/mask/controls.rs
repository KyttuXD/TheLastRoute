#![allow(dead_code)]
//! Masked Hours' controls (port of `mask_controls.gd`, from the 2D game's toggles and the control half of its Masked Hours
//! rules): the mask and the camera monitor are one exclusive pair with short raise / lower times (they never share the
//! screen; only a fully-on mask counts for the hunters; a press mid-transition is queued, the latest wins). Lights work only
//! with both down and still; winding only with the monitor fully up on CAM 1; the vent lights also work on their own duct's
//! camera. A held key is remembered (`held`), so a light or the wind comes back on its own once the panel is down / up. The
//! desk lamp is a plain switch. Every input reaches the rules through `press()`; `control_input()` is what the rules see
//! each frame. In 3D the physical stations and the quick keys both call `press()`.

use super::data::{map, rules};
use super::sim::SimInput;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    None,
    Mask,
    Monitor,
}

impl Panel {
    pub fn name(self) -> &'static str {
        match self {
            Panel::None => "",
            Panel::Mask => "mask",
            Panel::Monitor => "monitor",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Raising,
    Up,
    Lowering,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtlKind {
    Raise,
    Lower,
    Up,
    Down,
    Lamp,
}

/// A control event: a panel started to move, arrived, or the lamp flipped.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CtlEvent {
    pub kind: CtlKind,
    pub panel: Panel,
    pub gen: u32,
    /// For `Lamp`: the new state.
    pub on: bool,
}

#[derive(Clone, Debug)]
pub struct MaskControls {
    pub phase: Phase,
    pub panel: Panel,
    pub t: f64,
    pub gen: u32,
    pub want: Panel,
    pub cam: String,
    pub light_l: bool,
    pub light_r: bool,
    pub light_h: bool,
    pub win_key: bool,
    pub win_ptr: bool,
    pub lamp: bool,
}

impl Default for MaskControls {
    fn default() -> Self {
        MaskControls::new()
    }
}

impl MaskControls {
    pub fn new() -> MaskControls {
        MaskControls {
            phase: Phase::Idle,
            panel: Panel::None,
            t: 0.0,
            gen: 0,
            want: Panel::None,
            cam: "music".to_string(),
            light_l: false,
            light_r: false,
            light_h: false,
            win_key: false,
            win_ptr: false,
            lamp: false,
        }
    }

    fn dur(&self) -> f64 {
        let (raise, lower) = rules().panels.get(self.panel.name()).copied().unwrap_or((0.0, 0.0));
        match self.phase {
            Phase::Raising => raise,
            Phase::Lowering => lower,
            _ => 0.0,
        }
    }

    fn begin(&mut self, phase: Phase, p: Panel, ev: &mut Vec<CtlEvent>) {
        self.phase = phase;
        self.panel = p;
        self.t = 0.0;
        self.gen += 1;
        ev.push(CtlEvent { kind: if phase == Phase::Raising { CtlKind::Raise } else { CtlKind::Lower }, panel: p, gen: self.gen, on: false });
    }

    fn kick(&mut self, ev: &mut Vec<CtlEvent>) {
        if self.phase == Phase::Idle && self.want != Panel::None {
            let w = self.want;
            self.begin(Phase::Raising, w, ev);
        } else if self.phase == Phase::Up && self.want != self.panel {
            let p = self.panel;
            self.begin(Phase::Lowering, p, ev);
        }
    }

    fn finish(&mut self, ev: &mut Vec<CtlEvent>) {
        if self.phase == Phase::Raising {
            self.phase = Phase::Up;
            self.t = 0.0;
            if self.want == self.panel {
                ev.push(CtlEvent { kind: CtlKind::Up, panel: self.panel, gen: self.gen, on: false });
            }
        } else if self.phase == Phase::Lowering {
            ev.push(CtlEvent { kind: CtlKind::Down, panel: self.panel, gen: self.gen, on: false });
            self.phase = Phase::Idle;
            self.panel = Panel::None;
            self.t = 0.0;
        }
        self.kick(ev);
    }

    pub fn is_up(&self, p: Panel) -> bool {
        self.phase == Phase::Up && self.panel == p
    }

    /// Both panels down and still.
    pub fn clear(&self) -> bool {
        self.phase == Phase::Idle
    }

    pub fn busy(&self) -> bool {
        self.phase == Phase::Raising || self.phase == Phase::Lowering
    }

    /// The player's hands are on the monitor: it is wanted, or it is on its way.
    pub fn monitor_wanted(&self) -> bool {
        self.want == Panel::Monitor
    }

    /// The monitor holds the player (no walking, no looking around).
    pub fn monitor_busy(&self) -> bool {
        self.want == Panel::Monitor || self.panel == Panel::Monitor
    }

    /// How far panel `p` covers the screen, 0..1.
    pub fn cover(&self, p: Panel) -> f32 {
        if self.panel != p {
            return 0.0;
        }
        if self.phase == Phase::Up {
            return 1.0;
        }
        let d = self.dur();
        let k = if d > 0.0 { (self.t / d).min(1.0) } else { 1.0 };
        (if self.phase == Phase::Raising { k } else { 1.0 - k }) as f32
    }

    /// Everything down at once (night over, jump scare, quit).
    pub fn cancel(&mut self) {
        self.phase = Phase::Idle;
        self.panel = Panel::None;
        self.t = 0.0;
        self.want = Panel::None;
        self.gen += 1;
    }

    /// Is this held control down?
    pub fn held(&self, action: &str) -> bool {
        match action {
            "lightL" => self.light_l,
            "lightR" => self.light_r,
            "lightH" => self.light_h,
            "winKey" => self.win_key,
            "winPtr" => self.win_ptr,
            _ => false,
        }
    }

    /// One input, as a pure rule. Returns the toggle events it started. Actions: "mask" / "monitor" (a press on), "lightL" /
    /// "lightR" / "lightH" / "winKey" / "winPtr" (held down or let go; only one vent light at a time, the latest press wins),
    /// "lamp" (a press flips it), "cam:<room>". An unknown action does nothing.
    pub fn press(&mut self, action: &str, on: bool) -> Vec<CtlEvent> {
        let mut ev = Vec::new();
        match action {
            "mask" | "monitor" => {
                if on {
                    let p = if action == "mask" { Panel::Mask } else { Panel::Monitor };
                    self.want = if self.want == p { Panel::None } else { p };
                    self.kick(&mut ev);
                }
            }
            "winKey" | "winPtr" => {
                if action == "winKey" {
                    self.win_key = on;
                } else {
                    self.win_ptr = on;
                }
                if on && self.is_up(Panel::Monitor) {
                    self.cam = "music".to_string();
                }
            }
            "lightL" => {
                self.light_l = on;
                if on {
                    self.light_r = false;
                }
            }
            "lightR" => {
                self.light_r = on;
                if on {
                    self.light_l = false;
                }
            }
            "lightH" => self.light_h = on,
            "lamp" => {
                if on {
                    self.lamp = !self.lamp;
                    ev.push(CtlEvent { kind: CtlKind::Lamp, panel: Panel::None, gen: self.gen, on: self.lamp });
                }
            }
            _ => {
                if let Some(room) = action.strip_prefix("cam:") {
                    let ok = rules().room(room).map(|r| r.cam > 0).unwrap_or(false);
                    if self.is_up(Panel::Monitor) && ok {
                        self.cam = room.to_string();
                    }
                }
            }
        }
        ev
    }

    /// Advance by game time. A wind key already held when the monitor comes fully up jumps to CAM 1.
    pub fn step(&mut self, dt_ms: f64) -> Vec<CtlEvent> {
        let mut ev = Vec::new();
        let mut left = dt_ms;
        let mut guard = 0;
        while guard < 8 && (self.phase == Phase::Raising || self.phase == Phase::Lowering) {
            guard += 1;
            let need = self.dur() - self.t;
            if left < need {
                self.t += left;
                break;
            }
            left -= need;
            self.finish(&mut ev);
        }
        if self.phase == Phase::Up {
            self.t += left;
        }
        if self.win_key || self.win_ptr {
            if ev.iter().any(|e| e.kind == CtlKind::Up && e.panel == Panel::Monitor) {
                self.cam = "music".to_string();
            }
        }
        ev
    }

    /// What the rules see this frame (the lights need both panels down, or the duct camera for a vent light).
    pub fn control_input(&self) -> SimInput {
        let monitor_up = self.is_up(Panel::Monitor);
        let duct_up_l = monitor_up && self.cam == "ductL";
        let duct_up_r = monitor_up && self.cam == "ductR";
        SimInput {
            mask: self.is_up(Panel::Mask),
            monitor: monitor_up,
            cam: self.cam.clone(),
            light_l: self.light_l && (self.clear() || duct_up_l),
            light_r: self.light_r && !self.light_l && (self.clear() || duct_up_r),
            light_h: self.light_h && self.clear(),
            winding: monitor_up && self.cam == "music" && (self.win_key || self.win_ptr),
            lamp: self.lamp,
        }
    }

    /// The next camera (+1) or the previous one (-1) from the current, wrapping.
    pub fn cycle_cam(&self, step: i32) -> String {
        let rooms = rules().cam_rooms();
        if rooms.is_empty() {
            return self.cam.clone();
        }
        let i = rooms.iter().position(|r| *r == self.cam).unwrap_or(0) as i32;
        let n = rooms.len() as i32;
        rooms[((i + step) % n + n) as usize % rooms.len()].clone()
    }

    /// Is this room one of the monitor's cameras?
    pub fn is_camera(room: &str) -> bool {
        rules().room(room).map(|r| r.cam > 0).unwrap_or(false) && map().camera(room).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settle every transition (the player waits for it).
    fn settle(c: &mut MaskControls) -> Vec<CtlEvent> {
        c.step(500.0)
    }

    #[test]
    fn a_mask_press_raises_in_200_ms_and_is_up_at_once_after() {
        let mut c = MaskControls::new();
        let ev = c.press("mask", true);
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].kind, ev[0].panel), (CtlKind::Raise, Panel::Mask));
        assert!(!c.is_up(Panel::Mask) && c.busy());
        assert!(c.step(199.0).is_empty());
        assert!((c.cover(Panel::Mask) - 199.0 / 200.0).abs() < 1e-6);
        let ev = c.step(1.0);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, CtlKind::Up);
        assert!(c.is_up(Panel::Mask) && c.cover(Panel::Mask) == 1.0);
        assert!(c.control_input().mask, "only a fully-on mask counts");
    }

    #[test]
    fn the_mask_does_not_count_while_it_is_still_on_its_way() {
        let mut c = MaskControls::new();
        c.press("mask", true);
        c.step(150.0);
        assert!(!c.control_input().mask);
    }

    #[test]
    fn the_monitor_takes_250_ms_up_and_200_ms_down() {
        let mut c = MaskControls::new();
        c.press("monitor", true);
        assert!(c.step(249.0).is_empty());
        assert_eq!(c.step(1.0)[0].kind, CtlKind::Up);
        let ev = c.press("monitor", true);
        assert_eq!(ev[0].kind, CtlKind::Lower);
        assert!(c.step(199.0).is_empty());
        let ev = c.step(1.0);
        assert_eq!(ev[0].kind, CtlKind::Down);
        assert!(c.clear() && c.panel == Panel::None);
    }

    #[test]
    fn the_mask_and_the_monitor_never_share_the_screen_and_the_latest_press_wins() {
        let mut c = MaskControls::new();
        c.press("mask", true);
        settle(&mut c);
        assert!(c.is_up(Panel::Mask));
        // ask for the monitor: the mask comes down first
        let ev = c.press("monitor", true);
        assert_eq!((ev[0].kind, ev[0].panel), (CtlKind::Lower, Panel::Mask));
        assert!(!c.control_input().mask);
        let ev = c.step(150.0);
        let kinds: Vec<(CtlKind, Panel)> = ev.iter().map(|e| (e.kind, e.panel)).collect();
        assert_eq!(kinds, vec![(CtlKind::Down, Panel::Mask), (CtlKind::Raise, Panel::Monitor)]);
        settle(&mut c);
        assert!(c.is_up(Panel::Monitor) && !c.is_up(Panel::Mask));
    }

    #[test]
    fn a_press_in_the_middle_of_a_transition_is_queued() {
        let mut c = MaskControls::new();
        c.press("mask", true);
        c.step(100.0);
        // change my mind: monitor instead
        c.press("monitor", true);
        assert_eq!(c.want, Panel::Monitor);
        let ev = c.step(100.0);
        // the mask arrives but nobody wants it: no Up event, it is lowered at once
        assert!(!ev.iter().any(|e| e.kind == CtlKind::Up));
        assert!(ev.iter().any(|e| e.kind == CtlKind::Lower && e.panel == Panel::Mask));
        // press the monitor again while it comes: the request is withdrawn
        let mut d = MaskControls::new();
        d.press("monitor", true);
        d.step(100.0);
        d.press("monitor", true);
        assert_eq!(d.want, Panel::None);
        let ev = d.step(200.0);
        assert!(!ev.iter().any(|e| e.kind == CtlKind::Up));
        assert!(ev.iter().any(|e| e.kind == CtlKind::Lower));
    }

    #[test]
    fn lights_need_both_panels_down_except_a_vent_light_on_its_duct_camera() {
        let mut c = MaskControls::new();
        c.press("lightH", true);
        c.press("lightL", true);
        let i = c.control_input();
        assert!(i.light_h && i.light_l);
        // monitor up on the prize corner: no lights
        c.press("monitor", true);
        settle(&mut c);
        let i = c.control_input();
        assert!(!i.light_h && !i.light_l);
        // on CAM 6 the left vent light works (lights the duct), the hall light still does not
        c.press("cam:ductL", true);
        let i = c.control_input();
        assert!(i.light_l && !i.light_h);
        // CAM 7: the left light is not for that duct
        c.press("cam:ductR", true);
        assert!(!c.control_input().light_l);
    }

    #[test]
    fn only_one_vent_light_at_a_time_the_latest_press_wins() {
        let mut c = MaskControls::new();
        c.press("lightL", true);
        c.press("lightR", true);
        let i = c.control_input();
        assert!(i.light_r && !i.light_l);
        assert!(!c.held("lightL"));
        c.press("lightL", true);
        let i = c.control_input();
        assert!(i.light_l && !i.light_r);
    }

    #[test]
    fn a_held_light_comes_back_on_its_own_when_the_panel_is_down() {
        let mut c = MaskControls::new();
        c.press("lightH", true);
        c.press("monitor", true);
        settle(&mut c);
        assert!(!c.control_input().light_h);
        c.press("monitor", true);
        assert!(!c.control_input().light_h, "still lowering");
        settle(&mut c);
        assert!(c.control_input().light_h, "remembered");
        c.press("lightH", false);
        assert!(!c.control_input().light_h);
    }

    #[test]
    fn winding_needs_the_monitor_up_on_cam_one_and_jumps_there() {
        let mut c = MaskControls::new();
        c.press("winKey", true);
        assert!(!c.control_input().winding);
        // a wind key already held when the monitor comes up jumps to CAM 1
        c.press("monitor", true);
        c.cam = "ballroom".to_string();
        c.step(250.0);
        assert_eq!(c.cam, "music");
        assert!(c.control_input().winding);
        // another camera: no winding
        c.press("cam:gallery", true);
        assert!(!c.control_input().winding);
        // pressing the wind key again on another camera jumps back
        c.press("winKey", false);
        c.press("winKey", true);
        assert_eq!(c.cam, "music");
        c.press("winKey", false);
        assert!(!c.control_input().winding);
    }

    #[test]
    fn cameras_only_change_while_the_monitor_is_up_and_only_to_camera_rooms() {
        let mut c = MaskControls::new();
        c.press("cam:hall", true);
        assert_eq!(c.cam, "music");
        c.press("monitor", true);
        settle(&mut c);
        c.press("cam:hall", true);
        assert_eq!(c.cam, "hall");
        c.press("cam:office", true);
        assert_eq!(c.cam, "hall", "the office has no camera");
        c.press("cam:nowhere", true);
        assert_eq!(c.cam, "hall");
    }

    #[test]
    fn the_lamp_is_a_plain_switch() {
        let mut c = MaskControls::new();
        let ev = c.press("lamp", true);
        assert_eq!((ev[0].kind, ev[0].on), (CtlKind::Lamp, true));
        assert!(c.control_input().lamp);
        let ev = c.press("lamp", true);
        assert!(!ev[0].on);
        assert!(c.press("lamp", false).is_empty(), "letting go does nothing");
    }

    #[test]
    fn cancel_puts_everything_down() {
        let mut c = MaskControls::new();
        c.press("mask", true);
        settle(&mut c);
        c.cancel();
        assert!(c.clear() && c.want == Panel::None && !c.is_up(Panel::Mask));
    }

    #[test]
    fn the_camera_cycle_wraps() {
        let mut c = MaskControls::new();
        assert_eq!(c.cycle_cam(1), "costume");
        assert_eq!(c.cycle_cam(-1), "hall");
        c.cam = "hall".to_string();
        assert_eq!(c.cycle_cam(1), "music");
        assert!(MaskControls::is_camera("hall") && !MaskControls::is_camera("office"));
    }

    #[test]
    fn a_big_step_runs_the_whole_machine_in_order() {
        let mut c = MaskControls::new();
        c.press("monitor", true);
        c.step(250.0);
        c.press("mask", true);
        let ev = c.step(2000.0);
        let kinds: Vec<(CtlKind, Panel)> = ev.iter().map(|e| (e.kind, e.panel)).collect();
        assert_eq!(
            kinds,
            vec![(CtlKind::Down, Panel::Monitor), (CtlKind::Raise, Panel::Mask), (CtlKind::Up, Panel::Mask)]
        );
    }
}
