//! The nine game modes. Hub file (integrator): one line per mode.
#![allow(dead_code)]

pub mod bedroom;
pub mod classic;
pub mod dayshift;
pub mod descent;
pub mod lure;
pub mod mask;
pub mod mixed;
pub mod openair;
pub mod story;

#[cfg(test)]
mod camera_coverage_tests;
