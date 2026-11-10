//! Night Watch foundation: data tables, seeded streams, session and save, controls and input, Zellia's calls,
//! the hunters' path graph and walker, shared events, the Mode traits and the data checks.
//! Logic modules here never draw, play sound or open a window (only `input::InputFrame::from_macroquad` reads input).
#![allow(dead_code)]

pub mod calls;
pub mod check;
pub mod clock;
pub mod controls;
pub mod data;
pub mod events;
pub mod graph;
pub mod input;
pub mod mode;
pub mod rng;
pub mod save;
pub mod session;

pub use calls::*;
pub use check::*;
pub use controls::*;
pub use data::*;
pub use events::*;
pub use graph::*;
pub use input::*;
pub use mode::*;
pub use rng::*;
pub use save::*;
pub use session::*;
