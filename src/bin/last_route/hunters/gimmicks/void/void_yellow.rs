//! Yellow Void (mapPressure): while the full-screen map is open her speed multiplier grows 0.06 per second, up to 1.7x,
//! and stays there for the rest of the route (void_yellow_gimmick.gd). Closing the map never lowers it.
//!
//! Integration: the Godot target dict carries `"map_open"` (main_3d.gd:444, `hud.map_is_open()`). The Rust `Target` has
//! no such field yet, so the integrator adds `pub map_open: bool` to `hunters::brain::Target`, fills it from the app's
//! `map_open` (app/mod.rs), and calls `before(dt, t.map_open)` from `Gimmick::before_perception`. The speed multiplier
//! goes through `Gimmick::speed_scale`. This power emits no `Event`: it has no callout and no player-side effect.
//! The map's own blackout is not part of this power; it is the player's HUD state.
//!
//! Integration checklist (for the integrator, not done here):
//! - `Gimmick::VoidYellow(void::void_yellow::VoidYellow)` in the `Gimmick` enum, id "void_yellow",
//!   built as `VoidYellow::default()`.
//! - `pub mod void;` in hunters/gimmicks/mod.rs (the folder's `mod.rs` lists `void_yellow` once this is wired).
//! - Networking: `network_state()` / `apply_network_state()` carry only the scale (void_yellow_gimmick.gd:22-27).

/// void_yellow_gimmick.gd:7 (`const SPEED_PER_SECOND := 0.06`): speed multiplier gained per second of open map.
pub const SPEED_PER_SECOND: f32 = 0.06;
/// void_yellow_gimmick.gd:8 (`const MAXIMUM_SPEED_SCALE := 1.7`): the multiplier never exceeds this.
pub const MAXIMUM_SPEED_SCALE: f32 = 1.7;
/// void_yellow_gimmick.gd:10 (`var scale := 1.0`): she starts at her base speed.
pub const START_SCALE: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoidYellow {
    /// void_yellow_gimmick.gd `scale`: the speed multiplier, from START_SCALE up to MAXIMUM_SPEED_SCALE.
    pub scale: f32,
}

impl Default for VoidYellow {
    /// void_yellow_gimmick.gd:10 (`var scale := 1.0`).
    fn default() -> Self {
        VoidYellow { scale: START_SCALE }
    }
}

impl VoidYellow {
    /// void_yellow_gimmick.gd `before_perception` (lines 13-15). `map_open` is `target.get("map_open", false)`
    /// (main_3d.gd:444). The scale grows only while the map is open, by `SPEED_PER_SECOND * dt`, capped at
    /// `MAXIMUM_SPEED_SCALE`. Nothing lowers it.
    pub fn before(&mut self, dt: f32, map_open: bool) {
        if map_open {
            // void_yellow_gimmick.gd:15: scale = minf(MAXIMUM_SPEED_SCALE, scale + SPEED_PER_SECOND * delta)
            self.scale = MAXIMUM_SPEED_SCALE.min(self.scale + SPEED_PER_SECOND * dt);
        }
    }

    /// void_yellow_gimmick.gd `speed_scale` (lines 18-19): the current multiplier.
    pub fn speed_scale(&self) -> f32 {
        self.scale
    }

    /// void_yellow_gimmick.gd `network_state` (lines 22-23): `{"scale": scale}`. Sent as the bare float.
    pub fn network_state(&self) -> f32 {
        self.scale
    }

    /// void_yellow_gimmick.gd `apply_network_state` (lines 26-27): `scale = float(state.get("scale", scale))`.
    pub fn apply_network_state(&mut self, scale: f32) {
        self.scale = scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `frames` ticks of `dt` seconds with the map open or closed.
    fn run(g: &mut VoidYellow, frames: usize, dt: f32, map_open: bool) {
        for _ in 0..frames {
            g.before(dt, map_open);
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn constants_match_godot() {
        // void_yellow_gimmick.gd:7-8, :10
        assert_eq!(SPEED_PER_SECOND, 0.06);
        assert_eq!(MAXIMUM_SPEED_SCALE, 1.7);
        assert_eq!(START_SCALE, 1.0);
    }

    #[test]
    fn starts_at_base_speed() {
        let g = VoidYellow::default();
        assert_eq!(g.scale, 1.0);
        assert_eq!(g.speed_scale(), 1.0);
    }

    #[test]
    fn grows_six_hundredths_per_second_while_the_map_is_open() {
        let mut g = VoidYellow::default();
        // 10 one-second ticks with the map open: 1.0 + 10 * 0.06 = 1.6
        run(&mut g, 10, 1.0, true);
        assert!(close(g.speed_scale(), 1.6), "after 10 s: {}", g.speed_scale());
        // a half-second tick adds half a step: 0.03
        run(&mut g, 1, 0.5, true);
        assert!(close(g.speed_scale(), 1.63), "after a half second more: {}", g.speed_scale());
    }

    #[test]
    fn stays_flat_while_the_map_is_closed() {
        let mut g = VoidYellow::default();
        run(&mut g, 60, 1.0, false);
        assert_eq!(g.speed_scale(), 1.0, "no growth without the map");
    }

    #[test]
    fn caps_at_one_point_seven() {
        let mut g = VoidYellow::default();
        // 0.7 / 0.06 is about 11.67 s to reach the cap, so 20 s of map must stop exactly at 1.7
        run(&mut g, 20, 1.0, true);
        assert_eq!(g.speed_scale(), MAXIMUM_SPEED_SCALE, "capped at 1.7");
        run(&mut g, 60, 1.0, true);
        assert_eq!(g.speed_scale(), MAXIMUM_SPEED_SCALE, "stays capped");
    }

    #[test]
    fn the_increase_lasts_after_the_map_closes() {
        let mut g = VoidYellow::default();
        run(&mut g, 5, 1.0, true);
        let peak = g.speed_scale();
        assert!(close(peak, 1.3));
        // closing the map does not lower the multiplier for the rest of the route
        run(&mut g, 100, 1.0, false);
        assert_eq!(g.speed_scale(), peak, "the increase stays after the map closes");
        // reopening resumes from where she was
        run(&mut g, 1, 1.0, true);
        assert!(close(g.speed_scale(), 1.36), "resumes from 1.3: {}", g.speed_scale());
    }

    #[test]
    fn network_state_round_trips_the_scale() {
        let mut g = VoidYellow::default();
        run(&mut g, 3, 1.0, true);
        let mut peer = VoidYellow::default();
        peer.apply_network_state(g.network_state());
        assert_eq!(peer.scale, g.scale);
        assert_eq!(peer.speed_scale(), g.speed_scale());
    }
}
