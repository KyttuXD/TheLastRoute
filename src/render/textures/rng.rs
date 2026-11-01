//! Godot 4's `RandomNumberGenerator` (core/math/random_pcg.h + thirdparty/misc/pcg.cpp) and `String::hash()`,
//! bit for bit. `RefTextures._rng(key)` is `rng.seed = hash(key)`, so `GodotRng::from_key(key)` gives the same draws.
//! Checked against Godot 4.7.2 output (see NOTES.md).

const PCG_MULT: u64 = 6364136223846793005;
/// `RandomPCG::DEFAULT_INC` (= PCG_DEFAULT_INC_64). `set_seed` keeps the stream increment, so it is always this.
const DEFAULT_INC: u64 = 1442695040888963407;

/// Godot 4 `String::hash()`: djb2 (`hash * 33 + c`, start 5381) over the UTF-32 code points, wrapping u32.
/// GDScript's `hash(String)` returns this value as a non-negative int.
pub fn godot_string_hash(s: &str) -> u32 {
    let mut h: u32 = 5381;
    for c in s.chars() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(c as u32);
    }
    h
}

#[derive(Clone, Debug)]
pub struct GodotRng {
    state: u64,
    inc: u64,
}

impl GodotRng {
    /// `RandomNumberGenerator.new()` then `seed = p_seed` (pcg32_srandom_r with Godot's default increment).
    pub fn new(seed: u64) -> GodotRng {
        let mut rng = GodotRng {
            state: 0,
            inc: (DEFAULT_INC << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    /// `RefTextures._rng(key)`: seeded with `hash(key)`.
    pub fn from_key(key: &str) -> GodotRng {
        GodotRng::new(godot_string_hash(key) as u64)
    }

    /// pcg32_random_r. GDScript `randi()`.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    pub fn randi(&mut self) -> u32 {
        self.next_u32()
    }

    /// `RandomPCG::randf()`: float32 in [0, 1], two draws. Returned widened to f64 as GDScript sees it
    /// (compare it against f64 literals, as the GDScript does).
    pub fn randf(&mut self) -> f64 {
        self.randf32() as f64
    }

    fn randf32(&mut self) -> f32 {
        let proto_exp_offset = self.next_u32();
        if proto_exp_offset == 0 {
            return 0.0;
        }
        let significand = (self.next_u32() | 0x8000_0001) as f32;
        let exp = -32 - proto_exp_offset.leading_zeros() as i32;
        significand * f32::from_bits(((127 + exp) as u32) << 23)
    }

    /// `randf_range(from, to)`: real_t (float32) maths: `randf() * (to - from) + from`.
    pub fn randf_range(&mut self, from: f64, to: f64) -> f64 {
        let (f, t) = (from as f32, to as f32);
        (self.randf32() * (t - f) + f) as f64
    }

    /// pcg32_boundedrand_r: rejection below `-bound % bound`.
    fn bounded(&mut self, bound: u32) -> u32 {
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % bound;
            }
        }
    }

    /// `randi_range(from, to)`, inclusive both ends, either order.
    pub fn randi_range(&mut self, from: i32, to: i32) -> i32 {
        if from == to {
            return from;
        }
        let bound = (from as i64 - to as i64).unsigned_abs() as u32 + 1;
        (self.bounded(bound) as i64 + from.min(to) as i64) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values printed by Godot 4.7.2 (`hash(key)`, 4x randi, 4x randf, 4x randi_range(-1,1), 4x randf_range(.7,1.3)).
    #[test]
    fn matches_godot() {
        assert_eq!(godot_string_hash("metal0"), 226902760);
        assert_eq!(godot_string_hash("wpaper1"), 3631092069);
        assert_eq!(godot_string_hash("vault"), 277707889);
        assert_eq!(godot_string_hash(""), 5381);
        let mut r = GodotRng::from_key("metal0");
        let ints: Vec<u32> = (0..4).map(|_| r.randi()).collect();
        assert_eq!(ints, vec![2987326945, 1485561128, 1889055027, 2898878787]);
        let floats: Vec<f64> = (0..4).map(|_| r.randf()).collect();
        let want = [
            0.37766754627227783,
            0.70985066890716553,
            0.52403891086578369,
            0.91356426477432251,
        ];
        for (a, b) in floats.iter().zip(want) {
            assert!((a - b).abs() < 1e-15, "{a} vs {b}");
        }
        let ri: Vec<i32> = (0..4).map(|_| r.randi_range(-1, 1)).collect();
        assert_eq!(ri, vec![1, 0, -1, 1]);
        let fr: Vec<f64> = (0..4).map(|_| r.randf_range(0.7, 1.3)).collect();
        let want = [
            1.2532790899276733,
            1.0672054290771484,
            0.94725269079208374,
            0.96802949905395508,
        ];
        for (a, b) in fr.iter().zip(want) {
            assert!((a - b).abs() < 1e-14, "{a} vs {b}");
        }
    }
}
