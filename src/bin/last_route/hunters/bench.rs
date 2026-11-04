//! Timing probe for the shared brain at a crowded floor (Godot measured brain.update at 31.8 ms
//! with 38 hunters). Run it alone, in release, to read the number:
//! `cargo test --release hunter_brain_step_cost -- --ignored --nocapture`. It asserts no time.

use super::{Brain, Target};
use crate::config::Difficulty;
use crate::content::Content;
use crate::hunters::gimmicks::Ctx;
use crate::world::catalog::Catalog;
use crate::world::facility::Options;
use crate::world::Facility;
use std::hint::black_box;
use std::time::Instant;

const HUNTERS: usize = 38;
const STEPS: usize = 600;
const DT: f32 = 1.0 / 60.0;

#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn hunter_brain_step_cost() {
    let content = Content::load();
    let catalog = Catalog::load();
    let fac = Facility::generate("BENCH-38", &Options::default(), &catalog);
    let pool = content.draw_pool();
    assert!(!pool.is_empty(), "the draw pool has hunters");
    let cells = fac.floor_cells();
    let stride = (cells.len() / HUNTERS).max(1);
    let spawns: Vec<(f32, f32)> = cells
        .iter()
        .step_by(stride)
        .take(HUNTERS)
        .map(|c| c.center())
        .collect();
    let prof = Difficulty::Standard.profile();
    let runner = fac.start.center();
    let mut hunters: Vec<Brain> = spawns
        .iter()
        .enumerate()
        .map(|(i, &spawn)| {
            let def = content.hunters[pool[i % pool.len()]].clone();
            let mut b = Brain::new(def, spawn, &format!("BENCH-38:{i}"), Difficulty::Standard);
            b.activate_at(runner);
            b
        })
        .collect();

    let relays_taken = vec![false; fac.relays.len()];
    let batteries_taken = vec![false; fac.batteries.len()];
    let hiding_disabled = vec![false; fac.hiding.len()];
    let ctx = Ctx {
        fac: &fac,
        relays_taken: &relays_taken,
        batteries_taken: &batteries_taken,
        hiding_disabled: &hiding_disabled,
        vents: &[],
        heater_on: false,
        heater_thawed: true,
        diff: Difficulty::Standard,
        music_box_left: 0.0,
        exposure: 0.0,
    };
    let target = Target {
        pos: runner,
        facing: (0.0, 1.0),
        ..Default::default()
    };

    let started = Instant::now();
    let mut outcomes = 0usize;
    for _ in 0..STEPS {
        for b in hunters.iter_mut() {
            let (caught, events) = b.update(DT, &ctx, &target, 1, 0, &prof);
            outcomes += events.len() + caught as usize;
        }
    }
    let elapsed = started.elapsed();
    black_box(outcomes);

    println!(
        "{HUNTERS} hunters, {STEPS} steps: {:.3} ms per step",
        elapsed.as_secs_f64() * 1000.0 / STEPS as f64
    );
}

/// Hunters per resident floor in the multi-floor probe (the per-floor cap is data, flights.json).
const PER_FLOOR: usize = 12;
/// Docs section 5: the p95 budget for all hunter work in one tick, in milliseconds.
const BUDGET_MS: f64 = 1.0;

/// Phase 8 (D5): every resident floor simulates, so the per-tick cost grows with the floor
/// count. This steps 1, 2 and 3 floors of `PER_FLOOR` brains (each floor is its own brain list
/// over the same facility, which costs what a floor of its own costs) and prints ms per tick
/// against the 1.0 ms budget. Over budget: lower `resident_hunter_cap` in data/hunters/flights.json.
/// Run alone, in release, on KT-MBA for the target number:
/// `cargo test --release resident_floor_step_cost -- --ignored --nocapture`.
#[test]
#[ignore = "timing benchmark: run with --ignored --nocapture"]
fn resident_floor_step_cost() {
    let content = Content::load();
    let catalog = Catalog::load();
    let fac = Facility::generate("BENCH-FLOORS", &Options::default(), &catalog);
    let pool = content.draw_pool();
    assert!(!pool.is_empty(), "the draw pool has hunters");
    let cells = fac.floor_cells();
    let stride = (cells.len() / PER_FLOOR).max(1);
    let spawns: Vec<(f32, f32)> = cells.iter().step_by(stride).take(PER_FLOOR).map(|c| c.center()).collect();
    let prof = Difficulty::Standard.profile();
    let runner = fac.start.center();
    let relays_taken = vec![false; fac.relays.len()];
    let batteries_taken = vec![false; fac.batteries.len()];
    let hiding_disabled = vec![false; fac.hiding.len()];
    let ctx = Ctx {
        fac: &fac,
        relays_taken: &relays_taken,
        batteries_taken: &batteries_taken,
        hiding_disabled: &hiding_disabled,
        vents: &[],
        heater_on: false,
        heater_thawed: true,
        diff: Difficulty::Standard,
        music_box_left: 0.0,
        exposure: 0.0,
    };
    let target = Target { pos: runner, facing: (0.0, 1.0), ..Default::default() };

    for floors in 1..=3usize {
        let mut lists: Vec<Vec<Brain>> = (0..floors)
            .map(|f| {
                spawns
                    .iter()
                    .enumerate()
                    .map(|(i, &spawn)| {
                        let def = content.hunters[pool[(i + f) % pool.len()]].clone();
                        let mut b = Brain::new(def, spawn, &format!("BENCH-FLOORS:{f}:{i}"), Difficulty::Standard);
                        b.activate_at(runner);
                        b
                    })
                    .collect()
            })
            .collect();
        let started = Instant::now();
        let mut outcomes = 0usize;
        for _ in 0..STEPS {
            for list in lists.iter_mut() {
                for b in list.iter_mut() {
                    let (caught, events) = b.update(DT, &ctx, &target, 1, 0, &prof);
                    outcomes += events.len() + caught as usize;
                }
            }
        }
        let ms = started.elapsed().as_secs_f64() * 1000.0 / STEPS as f64;
        black_box(outcomes);
        println!(
            "{floors} resident floor(s) x {PER_FLOOR} hunters: {ms:.3} ms per tick ({})",
            if ms > BUDGET_MS { "OVER the 1.0 ms budget: lower resident_hunter_cap" } else { "within budget" }
        );
    }
}
