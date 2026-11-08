//! Cutscene runner: a fixed timeline over a list of shots. While it plays, hunters are paused (host side
//! only, per the guest rule) and input is frozen; at the end control returns.
use super::shot::Shot;

#[derive(Clone, Debug)]
pub struct Runner {
    shots: Vec<Shot>,
    index: usize,
    time: f32,
    done: bool,
}

impl Runner {
    pub fn new(shots: Vec<Shot>) -> Runner {
        let done = shots.is_empty();
        Runner { shots, index: 0, time: 0.0, done }
    }

    /// True while the scene owns the game: input frozen, hunters paused (host only).
    pub fn active(&self) -> bool {
        !self.done
    }

    /// The shot playing now and the time inside it.
    pub fn current(&self) -> Option<(&Shot, f32)> {
        self.shots.get(self.index).filter(|_| !self.done).map(|s| (s, self.time))
    }

    /// Advances the fixed timeline by `dt`; returns true on the frame control returns.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.done {
            return false;
        }
        self.time += dt;
        while self.time >= self.shots[self.index].duration {
            self.time -= self.shots[self.index].duration;
            self.index += 1;
            if self.index >= self.shots.len() {
                self.done = true;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_in_order_then_returns_control() {
        let base = super::super::shot::embedded_shots().remove(0);
        let mut a = base.clone();
        a.id = "a".into();
        a.duration = 1.0;
        let mut b = base;
        b.id = "b".into();
        b.duration = 1.0;
        let mut r = Runner::new(vec![a, b]);
        assert!(r.active());
        assert_eq!(r.current().unwrap().0.id, "a");
        assert!(!r.step(1.5));
        assert_eq!(r.current().unwrap().0.id, "b");
        assert!(r.step(0.6));
        assert!(!r.active());
        assert!(r.current().is_none());
    }
}
