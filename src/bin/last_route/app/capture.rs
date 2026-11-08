//! Movie capture: `--capture DIR --capture-fps N` writes one PNG per frame (`frame_000001.png`) at a fixed
//! timestep of 1/N seconds, whatever the real frame time, so ffmpeg can assemble them (docs/FLYTHROUGH.md).
use std::path::PathBuf;

pub const DEFAULT_FPS: u32 = 30;

pub struct Capture {
    pub dir: PathBuf,
    pub fps: u32,
    frame: u32,
}

impl Capture {
    pub fn new(dir: &str, fps: Option<u32>) -> Capture {
        Capture { dir: PathBuf::from(dir), fps: fps.filter(|f| *f > 0).unwrap_or(DEFAULT_FPS), frame: 0 }
    }

    /// The fixed game time step for every captured frame.
    pub fn dt(&self) -> f32 {
        1.0 / self.fps as f32
    }

    /// The path of the next frame; counts from 1.
    pub fn next_path(&mut self) -> PathBuf {
        self.frame += 1;
        self.dir.join(format!("frame_{:06}.png", self.frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_numbered_and_timestep_fixed() {
        let mut c = Capture::new("out", Some(60));
        assert!((c.dt() - 1.0 / 60.0).abs() < 1e-9);
        assert!(c.next_path().ends_with("frame_000001.png"));
        assert!(c.next_path().ends_with("frame_000002.png"));
        assert_eq!(Capture::new("o", Some(0)).fps, DEFAULT_FPS);
    }
}
