//! Admin switches that game code reads without touching the admin UI (god mode, no target, noclip,
//! run speed, debug view). The admin keeps them current every frame; core game code reads them through
//! `flags()`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Read-anywhere copies of the switches game code may want to honour exactly (see the hook
/// notes: god mode / no target in hunting.rs, noclip and run speed in player.rs, the debug view
/// in render/scene.rs). The admin keeps them current every frame.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Flags {
    pub god_mode: bool,
    pub no_target: bool,
    pub noclip: bool,
    pub run_speed: f32,
    pub fullbright: bool,
    pub wireframe: bool,
}

pub static GOD: AtomicBool = AtomicBool::new(false);
pub static NO_TARGET: AtomicBool = AtomicBool::new(false);
pub static NOCLIP: AtomicBool = AtomicBool::new(false);
pub static FULLBRIGHT: AtomicBool = AtomicBool::new(false);
pub static WIREFRAME: AtomicBool = AtomicBool::new(false);
pub static RUN_SPEED: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

#[allow(dead_code)]
pub fn flags() -> Flags {
    Flags {
        god_mode: GOD.load(Ordering::Relaxed),
        no_target: NO_TARGET.load(Ordering::Relaxed),
        noclip: NOCLIP.load(Ordering::Relaxed),
        run_speed: f32::from_bits(RUN_SPEED.load(Ordering::Relaxed)),
        fullbright: FULLBRIGHT.load(Ordering::Relaxed),
        wireframe: WIREFRAME.load(Ordering::Relaxed),
    }
}

