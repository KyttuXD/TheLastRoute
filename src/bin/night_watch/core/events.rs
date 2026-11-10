//! Shared events: a sound cue to play, and a short on-screen toast. Plain data.
#![allow(dead_code)]

/// A sound cue (names come from the audio module's list, without the `nw_` prefix).
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub name: &'static str,
    pub pan: f32,
    pub intensity: f32,
}

impl Cue {
    pub fn new(name: &'static str, pan: f32, intensity: f32) -> Cue {
        Cue { name, pan, intensity }
    }
}

/// A line of text that shows for a few seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub text: String,
    pub left_s: f32,
}

impl Toast {
    pub fn new(text: &str, seconds: f32) -> Toast {
        Toast { text: text.to_string(), left_s: seconds }
    }
    /// Count down; true while it still shows.
    pub fn tick(&mut self, dt_s: f32) -> bool {
        self.left_s -= dt_s;
        self.left_s > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_counts_down() {
        let mut t = Toast::new("hi", 1.0);
        assert!(t.tick(0.5));
        assert!(!t.tick(0.6));
    }

    #[test]
    fn cue_keeps_its_fields() {
        let c = Cue::new("step", -0.8, 0.5);
        assert_eq!((c.name, c.pan, c.intensity), ("step", -0.8, 0.5));
    }
}
