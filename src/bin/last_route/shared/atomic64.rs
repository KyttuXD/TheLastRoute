//! `AtomicU64` and `AtomicI64` for every target. Most targets have them and get the standard types unchanged.
//! 32-bit PowerPC has no 64-bit atomic instruction (docs/POWERPC.md): there the same names are a mutex around the
//! number, with the methods the game uses.

#[cfg(target_has_atomic = "64")]
pub use std::sync::atomic::{AtomicI64, AtomicU64};

#[cfg(not(target_has_atomic = "64"))]
mod fallback {
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    macro_rules! atomic {
        ($name:ident, $int:ty) => {
            #[derive(Debug, Default)]
            pub struct $name(Mutex<$int>);

            impl $name {
                pub const fn new(v: $int) -> Self {
                    Self(Mutex::new(v))
                }
                fn with<R>(&self, f: impl FnOnce(&mut $int) -> R) -> R {
                    f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
                }
                pub fn load(&self, _: Ordering) -> $int {
                    self.with(|v| *v)
                }
                pub fn store(&self, new: $int, _: Ordering) {
                    self.with(|v| *v = new)
                }
                pub fn swap(&self, new: $int, _: Ordering) -> $int {
                    self.with(|v| std::mem::replace(v, new))
                }
                pub fn fetch_add(&self, n: $int, _: Ordering) -> $int {
                    self.with(|v| std::mem::replace(v, v.wrapping_add(n)))
                }
                pub fn fetch_sub(&self, n: $int, _: Ordering) -> $int {
                    self.with(|v| std::mem::replace(v, v.wrapping_sub(n)))
                }
                pub fn fetch_max(&self, n: $int, _: Ordering) -> $int {
                    self.with(|v| std::mem::replace(v, (*v).max(n)))
                }
                pub fn compare_exchange(&self, current: $int, new: $int, _: Ordering, _: Ordering) -> Result<$int, $int> {
                    self.with(|v| if *v == current { Ok(std::mem::replace(v, new)) } else { Err(*v) })
                }
            }
        };
    }
    atomic!(AtomicU64, u64);
    atomic!(AtomicI64, i64);
}

#[cfg(not(target_has_atomic = "64"))]
pub use fallback::{AtomicI64, AtomicU64};
