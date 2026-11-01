//! Bit-for-bit port of SeededRandom (game/world/generation/seeded_random.gd): FNV-1a over the
//! seed text, then mulberry32. The same seed text gives the same draws as the original.

#[derive(Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(text: &str) -> Rng {
        let value = if text.is_empty() { "NIGHT-01" } else { text };
        let mut hash: u32 = 2166136261;
        for ch in value.chars() {
            hash ^= ch as u32;
            hash = hash.wrapping_mul(16777619);
        }
        Rng(hash)
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut value = self.0;
        value = (value ^ (value >> 15)).wrapping_mul(value | 1);
        let mixed = value.wrapping_add((value ^ (value >> 7)).wrapping_mul(value | 61));
        value ^= mixed;
        value ^ (value >> 14)
    }

    /// [0, 1), as `float(... ) / 4294967296.0` in the original.
    pub fn next_float(&mut self) -> f64 {
        self.next_u32() as f64 / 4294967296.0
    }

    /// Math.floor(random() * n)
    pub fn next_index(&mut self, n: usize) -> usize {
        (self.next_float() * n as f64).floor() as usize
    }

    pub fn next_f32(&mut self) -> f32 {
        self.next_float() as f32
    }

    /// Fisher-Yates from the end, as FacilityGenerator._shuffle_array.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.next_index(i + 1);
            items.swap(i, j);
        }
    }
}
