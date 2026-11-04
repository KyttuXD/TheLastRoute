//! Hiding animation, driven from `g.p.hidden` (set by `Game::enter_locker`, cleared by `leave_locker`):
//!
//! * the spot's door swings open and shut again when the runner goes in or comes out (`door_angle`);
//! * the runner's glide weight (`HideView::player`) runs over the data glide times; `render/hiding_cam.rs` turns it
//!   into the camera.
//!
//! `HideView` keeps the last seen hiding state between frames, like the other views in this folder, and advances on
//! the game clock. The spot's hinge side and the camera hook are applied by the caller (`render/scene.rs` for the prop,
//! `render/viewmodel.rs` / the camera for the player) from the values returned here.

use std::cell::RefCell;

/// Time the door takes to swing from shut to fully open, and again to shut.
pub const DOOR_SWING_TIME: f32 = 0.25;
/// Time the door stays fully open between the two swings.
pub const DOOR_HOLD_TIME: f32 = 0.15;
/// Fully open door angle, radians (about 100 degrees).
pub const DOOR_OPEN_ANGLE: f32 = 1.75;
/// Total length of one door open-hold-close cycle.
pub const DOOR_CYCLE_TIME: f32 = DOOR_SWING_TIME * 2.0 + DOOR_HOLD_TIME;

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The door's opening angle `elapsed` seconds after the runner went in or out: 0 shut, `DOOR_OPEN_ANGLE` open.
pub fn door_angle(elapsed: f32) -> f32 {
    if !(0.0..DOOR_CYCLE_TIME).contains(&elapsed) {
        return 0.0;
    }
    let open = if elapsed < DOOR_SWING_TIME {
        smooth(elapsed / DOOR_SWING_TIME)
    } else if elapsed < DOOR_SWING_TIME + DOOR_HOLD_TIME {
        1.0
    } else {
        1.0 - smooth((elapsed - DOOR_SWING_TIME - DOOR_HOLD_TIME) / DOOR_SWING_TIME)
    };
    open * DOOR_OPEN_ANGLE
}

/// The runner's glide into or out of the spot: `weight` 0 standing, 1 inside (the camera is `render/hiding_cam.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PlayerHide {
    pub weight: f32,
}

#[derive(Default)]
pub struct HideView {
    spot: Option<usize>,
    /// the spot whose door is cycling, and the game time the cycle began
    door: Option<(usize, f32)>,
    weight: f32,
    last_time: f32,
}

impl HideView {
    pub fn new() -> Self {
        HideView {
            last_time: f32::NAN,
            ..Default::default()
        }
    }

    /// Advance to `time` with the current hiding state. A clock that went backwards (a new run) resets the view.
    pub fn update(&mut self, hidden: Option<usize>, time: f32) {
        let dt = if self.last_time.is_nan() {
            0.0
        } else {
            time - self.last_time
        };
        if dt < 0.0 {
            *self = HideView::new();
        }
        self.last_time = time;
        let dt = dt.clamp(0.0, 0.1);
        if hidden != self.spot {
            if let Some(i) = hidden.or(self.spot) {
                self.door = Some((i, time));
            }
            self.spot = hidden;
        }
        let step = dt / crate::render::hiding_cam::glide_seconds(hidden.is_some());
        self.weight = if hidden.is_some() {
            (self.weight + step).min(1.0)
        } else {
            (self.weight - step).max(0.0)
        };
        if let Some((_, t0)) = self.door {
            if time - t0 >= DOOR_CYCLE_TIME {
                self.door = None;
            }
        }
    }

    /// The door angle for hiding spot `i` at `time`.
    pub fn door(&self, i: usize, time: f32) -> f32 {
        match self.door {
            Some((d, t0)) if d == i => door_angle(time - t0),
            _ => 0.0,
        }
    }

    pub fn player(&self) -> PlayerHide {
        PlayerHide { weight: self.weight }
    }
}

thread_local! {
    static HIDE: RefCell<HideView> = RefCell::new(HideView::new());
}

/// Advance the shared view with this frame's hiding state; call once per drawn frame.
pub fn tick(hidden: Option<usize>, time: f32) {
    HIDE.with(|h| h.borrow_mut().update(hidden, time));
}

pub fn door_for(i: usize, time: f32) -> f32 {
    HIDE.with(|h| h.borrow().door(i, time))
}

pub fn player_now() -> PlayerHide {
    HIDE.with(|h| h.borrow().player())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn door_opens_holds_and_shuts() {
        assert_eq!(door_angle(0.0), 0.0);
        assert!((door_angle(DOOR_SWING_TIME + 0.01) - DOOR_OPEN_ANGLE).abs() < 1e-5);
        assert!(door_angle(DOOR_SWING_TIME * 0.5) > 0.0);
        assert_eq!(door_angle(DOOR_CYCLE_TIME + 0.01), 0.0);
        assert_eq!(door_angle(-1.0), 0.0);
    }

    #[test]
    fn glide_weight_rises_and_returns() {
        let mut v = HideView::new();
        v.update(None, 0.0);
        v.update(Some(2), 0.0);
        let mut t = 0.0;
        while t < 1.0 {
            t += 0.02;
            v.update(Some(2), t);
        }
        assert_eq!(v.player().weight, 1.0);
        let mid = t;
        while t < mid + 1.0 {
            t += 0.02;
            v.update(None, t);
        }
        assert_eq!(v.player(), PlayerHide::default());
    }

    #[test]
    fn leaving_cycles_the_same_spots_door() {
        let mut v = HideView::new();
        v.update(Some(1), 0.0);
        v.update(Some(1), 5.0);
        v.update(None, 5.1);
        assert!(v.door(1, 5.1 + DOOR_SWING_TIME) > 0.0);
        assert_eq!(v.door(0, 5.1 + DOOR_SWING_TIME), 0.0);
        v.update(None, 5.1 + DOOR_CYCLE_TIME + 0.2);
        assert_eq!(v.door(1, 5.1 + DOOR_CYCLE_TIME + 0.2), 0.0);
    }
}
