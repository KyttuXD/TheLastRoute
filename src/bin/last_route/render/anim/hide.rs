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

/// The doors of the lockers other runners hide in: a door swings once when the number of hiders in a locker changes
/// (one goes in, one comes out). Keyed by the locker's cell in the window's cells.
#[derive(Default)]
pub struct RemoteDoors {
    inside: std::collections::HashMap<(i32, i32), usize>,
    /// locker cell -> the game time its door began to swing
    cycles: std::collections::HashMap<(i32, i32), f32>,
    last_time: f32,
    primed: bool,
}

impl RemoteDoors {
    /// Advance to `time` with the lockers (one entry per hider) the remote runners are in. The first call only learns
    /// who is already inside, so a runner met mid-hide does not swing a door; a clock that ran backwards resets.
    pub fn update(&mut self, spots: &[(i32, i32)], time: f32) {
        if self.primed && time < self.last_time {
            *self = RemoteDoors::default();
        }
        let mut now: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
        for s in spots {
            *now.entry(*s).or_insert(0) += 1;
        }
        if self.primed {
            for (cell, n) in &now {
                if self.inside.get(cell) != Some(n) {
                    self.cycles.insert(*cell, time);
                }
            }
            for cell in self.inside.keys() {
                if !now.contains_key(cell) {
                    self.cycles.insert(*cell, time);
                }
            }
        }
        self.cycles.retain(|_, t0| time - *t0 < DOOR_CYCLE_TIME);
        self.inside = now;
        self.last_time = time;
        self.primed = true;
    }

    /// The door angle of the locker at `cell` at `time`.
    pub fn door(&self, cell: (i32, i32), time: f32) -> f32 {
        self.cycles.get(&cell).map_or(0.0, |t0| door_angle(time - t0))
    }
}

thread_local! {
    static HIDE: RefCell<HideView> = RefCell::new(HideView::new());
    static REMOTE: RefCell<RemoteDoors> = RefCell::new(RemoteDoors::default());
}

/// Advance the remote runners' locker doors with where they hide now; call once per drawn frame.
pub fn tick_remote(spots: &[(i32, i32)], time: f32) {
    REMOTE.with(|r| r.borrow_mut().update(spots, time));
}

/// The door angle of the locker at window cell `cell` from the remote runners' hiding.
pub fn remote_door_for(cell: (i32, i32), time: f32) -> f32 {
    REMOTE.with(|r| r.borrow().door(cell, time))
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
    fn a_remote_hider_swings_its_lockers_door_going_in_and_coming_out() {
        let mut d = RemoteDoors::default();
        let cell = (4, 7);
        d.update(&[], 0.0);
        d.update(&[], 1.0);
        assert_eq!(d.door(cell, 1.0 + DOOR_SWING_TIME), 0.0, "an empty locker stays shut");
        d.update(&[cell], 2.0);
        assert!(d.door(cell, 2.0 + DOOR_SWING_TIME) > 0.0, "going in swings the door");
        d.update(&[cell], 2.0 + DOOR_CYCLE_TIME + 0.5);
        assert_eq!(d.door(cell, 3.5 + DOOR_CYCLE_TIME), 0.0, "and it shuts again");
        d.update(&[], 10.0);
        assert!(d.door(cell, 10.0 + DOOR_SWING_TIME) > 0.0, "coming out swings it again");
        assert_eq!(d.door((0, 0), 10.0 + DOOR_SWING_TIME), 0.0, "another locker is unaffected");
    }

    #[test]
    fn a_runner_already_inside_when_first_seen_does_not_swing_a_door() {
        let mut d = RemoteDoors::default();
        d.update(&[(1, 1)], 5.0);
        assert_eq!(d.door((1, 1), 5.0 + DOOR_SWING_TIME), 0.0);
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
