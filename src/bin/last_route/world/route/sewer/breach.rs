//! The parts of `Breach` (game/world/regions/breach.gd) that the sewer's breakables use: the spec, its id and
//! the two kinds the sewer makes (`KIND_SEAL`, `KIND_PIPE`).
//!
//! Ported: `make_spec` (36), `make_id` (42), `KIND_PIPE` (25) and `KIND_SEAL` (23). Not ported, because nothing in
//! the route reads them yet: the hooks, the damage tables (`TOOL_DAMAGE`, `NOISE_LEVEL`), `damage_for` and `parse_id`.
//!
//! A spec is Godot's dictionary `{id, kind, variant, chunk, cell, hp, tools, locked, extras}`. `extras` is one of
//! two shapes, the seal's (`SealPlanner.specs`) or the pipe's (`_pipe_breakables`), so it is an enum.

use serde_json::{json, Value};

use crate::world::route::layout::{Coord, V2};

/// `Breach.KIND_SEAL` (breach.gd:23).
pub const KIND_SEAL: &str = "seal";
/// `Breach.KIND_PIPE` (breach.gd:25).
pub const KIND_PIPE: &str = "pipe";

/// A spec's `extras`: the seal's dictionary or the pipe's.
#[derive(Clone, Debug, PartialEq)]
pub enum Extras {
    Seal(SealExtras),
    Pipe(PipeExtras),
}

/// A seal's extras (`SealPlanner.specs`, seal_planner.gd:189-192): `{seal, cells, centre, dir, radius, tunnel}`, and
/// `door_frame` once `_frame_door_seal` (sewer:569-584) frames a locked door.
#[derive(Clone, Debug, PartialEq)]
pub struct SealExtras {
    pub seal: &'static str,
    pub cells: Vec<V2>,
    pub centre: [f64; 2],
    pub dir: [f64; 2],
    pub radius: f64,
    pub tunnel: usize,
    /// The two jamb cells outside the gate that become brick (`door_frame`).
    pub door_frame: Option<[V2; 2]>,
}

/// A pipe's extras (`_pipe_breakables`, sewer:935-939): `{pipe, props, cells, centre, dir, flood_cells, flood_steps,
/// wall}`. `flood_cells` are the cells the burst floods, nearest first; `flood_steps` is each one's step from the pipe.
#[derive(Clone, Debug, PartialEq)]
pub struct PipeExtras {
    pub pipe: &'static str,
    pub props: Vec<String>,
    pub cells: Vec<V2>,
    pub centre: [f64; 2],
    pub dir: [f64; 2],
    pub flood_cells: Vec<V2>,
    pub flood_steps: Vec<i32>,
    pub wall: [f64; 2],
}

/// One breakable (`Breach.make_spec`, breach.gd:36-39).
#[derive(Clone, Debug, PartialEq)]
pub struct BreachSpec {
    pub id: String,
    pub kind: &'static str,
    pub variant: &'static str,
    pub chunk: Coord,
    pub cell: V2,
    pub hp: i32,
    pub tools: Vec<&'static str>,
    pub locked: bool,
    pub extras: Extras,
}

/// `Breach.make_id` (breach.gd:42): `BRK:<x>:<y>:<z>:<index>`.
pub fn make_id(chunk: Coord, index: usize) -> String {
    format!("BRK:{}:{}:{}:{}", chunk.x, chunk.y, chunk.z, index)
}

/// `Breach.make_spec` (breach.gd:36): hit points are at least 1; `tools` and `extras` are copied.
#[allow(clippy::too_many_arguments)]
pub fn make_spec(
    id: String,
    kind: &'static str,
    variant: &'static str,
    chunk: Coord,
    cell: V2,
    hp: i32,
    tools: Vec<&'static str>,
    extras: Extras,
    locked: bool,
) -> BreachSpec {
    BreachSpec { id, kind, variant, chunk, cell, hp: hp.max(1), tools, locked, extras }
}

impl BreachSpec {
    /// The spec in the golden dump's shape (`data["breakables"]` items). Numbers are JSON numbers; the golden prints
    /// whole floats without a point, which compares equal as numbers.
    pub fn to_json(&self) -> Value {
        let extras = match &self.extras {
            Extras::Seal(s) => {
                let mut m = json!({
                    "seal": s.seal,
                    "cells": cells_json(&s.cells),
                    "centre": [s.centre[0], s.centre[1]],
                    "dir": [s.dir[0], s.dir[1]],
                    "radius": s.radius,
                    "tunnel": s.tunnel,
                });
                if let Some(frame) = s.door_frame {
                    m["door_frame"] = json!([cell_json(frame[0]), cell_json(frame[1])]);
                }
                m
            }
            Extras::Pipe(p) => json!({
                "pipe": p.pipe,
                "props": p.props,
                "cells": cells_json(&p.cells),
                "centre": [p.centre[0], p.centre[1]],
                "dir": [p.dir[0], p.dir[1]],
                "flood_cells": cells_json(&p.flood_cells),
                "flood_steps": p.flood_steps,
                "wall": [p.wall[0], p.wall[1]],
            }),
        };
        json!({
            "id": self.id,
            "kind": self.kind,
            "variant": self.variant,
            "chunk": [self.chunk.x, self.chunk.y, self.chunk.z],
            "cell": cell_json(self.cell),
            "hp": self.hp,
            "tools": self.tools,
            "locked": self.locked,
            "extras": extras,
        })
    }
}

fn cell_json(c: V2) -> Value {
    json!([c.x, c.y])
}

fn cells_json(cells: &[V2]) -> Value {
    Value::Array(cells.iter().map(|c| cell_json(*c)).collect())
}

#[cfg(test)]
mod flood_tests {
    use super::*;
    use crate::world::route::sewer::core::{chunk_base, Grid, Inputs, DRY, N, SOLID};
    use crate::world::route::sewer::seals::{flood_cells, FLOOD_MAX_CELLS, FLOOD_REACH};

    /// Godot sewer_route_generator.gd:910-911.
    #[test]
    fn flood_reach_is_twelve_and_the_cap_is_one_hundred_sixty() {
        assert_eq!(FLOOD_REACH, 12);
        assert_eq!(FLOOD_MAX_CELLS, 160);
    }

    /// A one-cell-wide corridor stops at the reach, one cell per step, nearest first.
    #[test]
    fn a_corridor_floods_exactly_the_reach() {
        let mut g: Grid = [[SOLID; N]; N];
        for x in 0..N {
            g[5][x] = DRY;
        }
        let (cells, steps) = flood_cells(&g, &[V2::new(0, 5)]);
        assert_eq!(cells.len(), FLOOD_REACH as usize + 1);
        assert_eq!(steps.iter().copied().max(), Some(FLOOD_REACH));
        assert_eq!(cells.last().copied(), Some(V2::new(FLOOD_REACH, 5)));
        assert!(steps.windows(2).all(|w| w[0] <= w[1]));
    }

    /// An open floor holds more than the cap inside the reach, so the cap is what stops the flood.
    #[test]
    fn an_open_floor_floods_exactly_the_cap() {
        let g: Grid = [[DRY; N]; N];
        let (cells, steps) = flood_cells(&g, &[V2::new(22, 22)]);
        assert_eq!(cells.len(), FLOOD_MAX_CELLS);
        assert_eq!(steps.len(), cells.len());
        assert!(steps.iter().all(|s| *s <= FLOOD_REACH));
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "nearest first");
        let unique: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(unique.len(), cells.len());
    }

    /// Every pipe of generated sewer chunks respects both limits.
    #[test]
    fn generated_pipe_floods_stay_inside_reach_and_cap() {
        let mut pipes = 0;
        for world in ["flood-a", "flood-b"] {
            for level in [-1, -2] {
                for x in -6..6 {
                    for z in -6..6 {
                        let Some(c) = chunk_base(Coord::new(x, level, z), &Inputs { world }) else { continue };
                        for spec in &c.breakables {
                            let Extras::Pipe(p) = &spec.extras else { continue };
                            pipes += 1;
                            assert!(p.flood_cells.len() <= FLOOD_MAX_CELLS, "{}", spec.id);
                            assert_eq!(p.flood_cells.len(), p.flood_steps.len(), "{}", spec.id);
                            assert!(p.flood_steps.iter().all(|s| (0..=FLOOD_REACH).contains(s)), "{}", spec.id);
                            assert!(p.flood_steps.windows(2).all(|w| w[0] <= w[1]), "{}", spec.id);
                        }
                    }
                }
            }
        }
        assert!(pipes > 0, "no pipe in the scanned chunks");
    }
}
