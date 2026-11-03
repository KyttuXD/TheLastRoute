//! The per-level danger profile of the streamed route: a port of `StreamedRouteGenerator.difficulty_for_level`
//! (streamed_route_generator.gd:1290-1300) and the depth rules that read it.
//!
//! Level 0 is the central arrival floor (danger 0.48). Each step down adds 0.075 danger, each step up takes it away,
//! clamped to 0.12..=1.0, so there is no hard depth cap until level -7 saturates it. Godot reads the profile in:
//! - `_hunter_spawns` (5644-5650): a chunk's hunter spawn chance `lerpf(0.16, 0.42, danger)`;
//! - `_hunter_spawns` (5675): at level -6 and below, a second hunter with chance `minf(0.22, danger * 0.18)`;
//! - `is_safe_room` (1403-1413): the private route's safe-room chance `0.055 * safe_room_bias` (the public route 0.22);
//! - `safe_room_data` (1416-1425): `breach_pressure = snappedf(lerpf(0.55, 1.65, danger), 0.01)`;
//! - the sewer's `_hunter_spawns` (sewer_route_generator.gd:395-451), with the same rules.
//!
//! `hunter_speed_scale` and `hunter_count_bias` are in the Godot dictionary but no Godot runtime code reads them
//! (HunterBrain's speed is the run difficulty's `hunter_speed`); they are kept here for parity and tools. The Rust
//! brain's own depth speed bonus (`HUNTER_DEPTH_SPEED_BONUS_PER_LEVEL`) is separate and unchanged.

/// The private route's safe-room chance per chunk is `SAFE_ROOM_BASE_CHANCE * safe_room_bias` (is_safe_room, 1412).
pub const SAFE_ROOM_BASE_CHANCE: f64 = 0.055;
/// The public route's flat safe-room chance (is_safe_room, 1412).
pub const PUBLIC_SAFE_ROOM_CHANCE: f64 = 0.22;
/// The deepest level with a single hunter per chunk; at `DEEP_PAIR_LEVEL` and below a chunk may roll a second one.
pub const DEEP_PAIR_LEVEL: i32 = -6;
/// From this level down a placed hunter may roll the Office Void (5689, sewer:458).
pub const OFFICE_VOID_LEVEL: i32 = -2;
/// The per-spawn Office Void chance (5689).
pub const OFFICE_VOID_CHANCE: f64 = 0.015;

/// `lerpf(a, b, t)`, in the order Godot evaluates it.
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// `difficulty_for_level(level)`: the Godot dictionary's fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DangerProfile {
    pub level: i32,
    /// `clampf(0.48 - level * 0.075, 0.12, 1.0)`.
    pub danger: f64,
    /// `lerpf(0.86, 1.28, danger)`.
    pub hunter_speed_scale: f64,
    /// `floori(maxf(0, -level) / 3)`: one more per three levels down.
    pub hunter_count_bias: i32,
    /// `lerpf(1.45, 0.55, danger)`: safe rooms thin out as danger rises.
    pub safe_room_bias: f64,
}

impl DangerProfile {
    /// `difficulty_for_level(level)` (streamed_route_generator.gd:1290).
    pub fn for_level(level: i32) -> Self {
        let danger = (0.48 - level as f64 * 0.075).clamp(0.12, 1.0);
        DangerProfile {
            level,
            danger,
            hunter_speed_scale: lerp(0.86, 1.28, danger),
            hunter_count_bias: ((-(level as f64)).max(0.0) / 3.0).floor() as i32,
            safe_room_bias: lerp(1.45, 0.55, danger),
        }
    }

    /// The profile of a finite-route depth (depth 1 is level 0, `depth = 1 - level`).
    pub fn for_depth(depth: i32) -> Self {
        Self::for_level(1 - depth)
    }

    /// A chunk's hunter spawn chance, `lerpf(0.16, 0.42, danger)` (5650, sewer:397).
    pub fn hunter_spawn_chance(&self) -> f64 {
        lerp(0.16, 0.42, self.danger)
    }

    /// The chance of a second hunter in a chunk, `minf(0.22, danger * 0.18)`, at `DEEP_PAIR_LEVEL` and below (5675).
    /// `None` above it, where Godot draws no roll.
    pub fn deep_pair_chance(&self) -> Option<f64> {
        (self.level <= DEEP_PAIR_LEVEL).then(|| 0.22f64.min(self.danger * 0.18))
    }

    /// Whether a placed hunter rolls the Office Void at this level (5689).
    pub fn rolls_office_void(&self) -> bool {
        self.level <= OFFICE_VOID_LEVEL
    }

    /// The private route's per-chunk safe-room chance, `0.055 * safe_room_bias` (1412).
    pub fn safe_room_chance(&self) -> f64 {
        SAFE_ROOM_BASE_CHANCE * self.safe_room_bias
    }

    /// A safe room's `breach_pressure`, `snappedf(lerpf(0.55, 1.65, danger), 0.01)` (1424).
    pub fn breach_pressure(&self) -> f64 {
        (lerp(0.55, 1.65, self.danger) * 100.0).round() / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn level_zero_is_the_godot_arrival_profile() {
        let p = DangerProfile::for_level(0);
        assert!(close(p.danger, 0.48));
        assert!(close(p.hunter_speed_scale, 1.0616));
        assert_eq!(p.hunter_count_bias, 0);
        assert!(close(p.safe_room_bias, 1.018));
        assert!(close(p.hunter_spawn_chance(), 0.2848));
        assert!(close(p.safe_room_chance(), 0.05599));
        assert!(close(p.breach_pressure(), 1.08));
        assert_eq!(p.deep_pair_chance(), None);
        assert!(!p.rolls_office_void());
        assert_eq!(DangerProfile::for_depth(1), p);
    }

    #[test]
    fn deeper_levels_are_harder_and_saturate() {
        let mut last = DangerProfile::for_level(4);
        for level in (-12..=3).rev() {
            let p = DangerProfile::for_level(level);
            assert!(p.danger >= last.danger, "{level}");
            assert!(p.hunter_speed_scale >= last.hunter_speed_scale, "{level}");
            assert!(p.hunter_spawn_chance() >= last.hunter_spawn_chance(), "{level}");
            assert!(p.safe_room_chance() <= last.safe_room_chance(), "{level}");
            assert!(p.hunter_count_bias >= last.hunter_count_bias, "{level}");
            last = p;
        }
        // -7 is the first saturated level: 0.48 + 7 * 0.075 = 1.005 -> 1.0
        let deep = DangerProfile::for_level(-7);
        assert!(close(deep.danger, 1.0));
        assert!(close(deep.hunter_speed_scale, 1.28));
        assert!(close(deep.safe_room_bias, 0.55));
        assert!(close(deep.hunter_spawn_chance(), 0.42));
        assert!(close(deep.breach_pressure(), 1.65));
        assert_eq!(DangerProfile::for_level(-40).danger, 1.0);
        // the top floor floors at 0.12
        assert!(close(DangerProfile::for_level(5).danger, 0.12));
        assert!(close(DangerProfile::for_level(5).safe_room_bias, 1.342));
    }

    #[test]
    fn count_bias_and_deep_rolls_follow_the_levels() {
        let bias: Vec<i32> = [0, -1, -2, -3, -5, -6, -9, 2].iter().map(|l| DangerProfile::for_level(*l).hunter_count_bias).collect();
        assert_eq!(bias, vec![0, 0, 0, 1, 1, 2, 3, 0]);
        assert_eq!(DangerProfile::for_level(-5).deep_pair_chance(), None);
        // -6: danger 0.93, 0.93 * 0.18 = 0.1674; -7 and below cap at 0.18 (danger 1.0), under the 0.22 ceiling
        assert!(close(DangerProfile::for_level(-6).deep_pair_chance().unwrap(), 0.1674));
        assert!(close(DangerProfile::for_level(-9).deep_pair_chance().unwrap(), 0.18));
        assert!(!DangerProfile::for_level(-1).rolls_office_void());
        assert!(DangerProfile::for_level(-2).rolls_office_void());
    }

    #[test]
    fn sewer_levels_match_the_godot_formulas() {
        assert!(close(DangerProfile::for_level(-1).danger, 0.555));
        assert!(close(DangerProfile::for_level(-2).danger, 0.63));
        assert!(close(DangerProfile::for_level(-1).hunter_spawn_chance(), 0.3043));
        assert_eq!(DangerProfile::for_depth(3).level, -2);
    }
}
