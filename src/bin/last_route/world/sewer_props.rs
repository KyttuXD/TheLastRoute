//! The sewer chunk's props (`SewerChunk::region_props`: pipe runs, valve wheels, pumps, the outflow main, sluice gates,
//! lamps, bridge decks) as live `Facility` props, so the grid, the hunters' navigation, the interact focus and the
//! mesh builder all see them. The ids stay the sewer's own (`SEWER:x:y:z:PROP:n`), which is what a valve turn names.
//!
//! Collision: only the pump, the outflow main, the extra rooms' own solid props (barrel, crate, locker) and a closed
//! sluice gate are solid. A crawl pipe hangs at crawl height and is not solid to the runner here (nothing in the port
//! crawls), but it is to a hunter (`Prop::crawl_under`, `Facility::hunter_fits`): Godot keeps the tall hunters out of the
//! cells of a raised collider. A valve, lamp or bridge deck is never in the way. A sluice gate is solid while closed (a wall
//! across its channel cells) and `Facility::set_gate_open` makes it floor.

use super::catalog::Catalog;
use super::facility::{Cell, Prop, SewerKeys, SLUICE_GATE};
use super::route::sewer::core::{rules, SewerProp};
use super::route::sewer::SewerChunk;
use crate::hunters::navigation;

/// The prop type of a ladder: the stairs draw it, so it is not a prop here.
const LADDER: &str = "sewerLadder";
const PUMP: &str = "sewerPump";
const VALVE: &str = "sewerValveWheel";

/// The live props of a sewer chunk, in the chunk's cells. `rooms` are the facility rooms built from the chunk, used to
/// give each prop its room index.
pub fn props(sc: &SewerChunk, rooms: &[super::facility::Room], cat: &Catalog) -> Vec<Prop> {
    sc.region_props
        .iter()
        .filter(|p| p.prop_type != LADDER && p.link.is_none())
        .map(|p| prop(p, rooms, cat))
        .collect()
}

fn prop(p: &SewerProp, rooms: &[super::facility::Room], cat: &Catalog) -> Prop {
    let rotation = ((p.yaw / std::f64::consts::FRAC_PI_2).round() as i32).rem_euclid(4);
    let gate = p.prop_type == SLUICE_GATE;
    let wanted_solid = (p.solid && !p.crawl_under) || gate;
    let (half, offset) = if gate {
        (gate_half(p), (0.0, 0.0))
    } else if p.outflow {
        (navigation::half_extents(p.collider_size.unwrap_or(rules().outflow_size), rotation), (0.0, 0.0))
    } else if p.prop_type == PUMP {
        let at = p.mount_offset.unwrap_or([0.0, 0.0]);
        (navigation::half_extents(p.collider_size.unwrap_or(rules().pump_collider), rotation), (at[0], at[1]))
    } else if wanted_solid {
        (cat.footprint(p.prop_type, rotation), (0.0, 0.0))
    } else {
        let size = p.collider_size.unwrap_or([0.5, 0.5, 0.5]);
        let at = p.placement_offset.or(p.mount_offset).unwrap_or([0.0, 0.0]);
        (navigation::half_extents(size, rotation), (at[0], at[1]))
    };
    let keys = match p.prop_type {
        VALVE => Some(SewerKeys {
            effect: p.valve_effect.unwrap_or("flood").to_string(),
            gate: p.gate.clone().unwrap_or_default(),
        }),
        SLUICE_GATE => Some(SewerKeys { effect: String::new(), gate: p.gate.clone().unwrap_or_default() }),
        _ => None,
    };
    Prop {
        id: p.id.clone(),
        cell: Cell::new(p.cell.x, p.cell.y),
        ptype: p.prop_type.to_string(),
        room: rooms
            .iter()
            .position(|r| p.cell.x >= r.x && p.cell.y >= r.y && p.cell.x < r.x + r.w && p.cell.y < r.y + r.h)
            .unwrap_or(0),
        rotation,
        mount_y: p.mount_y.filter(|y| *y != 0.0),
        solid: wanted_solid,
        half,
        offset,
        door: None,
        sewer: keys,
        crawl_under: p.crawl_under,
    }
}

/// A gate's footprint: its channel cells, one cell thick.
fn gate_half(p: &SewerProp) -> (f32, f32) {
    let cells = p.gate_cells.as_deref().unwrap_or(&[]);
    let span = |f: fn(&crate::world::route::layout::V2) -> i32| -> f32 {
        let lo = cells.iter().map(f).min().unwrap_or(0);
        let hi = cells.iter().map(f).max().unwrap_or(0);
        (hi - lo + 1) as f32 / 2.0
    };
    if cells.is_empty() {
        (0.5, 0.5)
    } else {
        (span(|c| c.x), span(|c| c.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::{self, ChunkKey, CHUNK};
    use crate::world::route::sewer::core::{LEVEL, LOWER_LEVEL, WATER};
    use crate::world::route::sewer::{self, Inputs};
    use crate::world::route::layout::Coord;
    use crate::content::loot::LootTables;
    use std::collections::{HashSet, VecDeque};

    const WORLDS: [&str; 4] = ["wired-sewer", "1426541270", "water-a", "water-b"];

    fn cat() -> &'static Catalog {
        static CAT: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
        CAT.get_or_init(Catalog::load)
    }

    fn built(world: &str, cx: i32, cy: i32, level: i32) -> super::super::facility::Facility {
        let loot = LootTables::default();
        infinite::chunk(world, ChunkKey { cx, cy, level }, cat(), &loot).0
    }

    #[test]
    fn the_origin_chunk_of_the_first_sewer_has_a_valve_a_pipe_run_and_a_pump_with_sewer_ids() {
        let mut seen: Vec<String> = vec![];
        for world in WORLDS {
            let f = built(world, 0, 0, LEVEL);
            for p in &f.props {
                assert!(p.id.starts_with("SEWER:0:-1:0:PROP:"), "{}", p.id);
            }
            seen.extend(f.props.iter().map(|p| p.ptype.clone()));
        }
        for kind in ["sewerValveWheel", "sewerPipeRun", "sewerPump"] {
            assert!(seen.iter().any(|t| t == kind), "no {kind} at chunk (0,0) level -1 of any sample world: {seen:?}");
        }
    }

    #[test]
    fn level_zero_has_no_sewer_props() {
        for world in WORLDS {
            let f = built(world, 0, 0, 0);
            assert!(f.props.iter().all(|p| !p.ptype.starts_with("sewer")), "{world}");
        }
    }

    #[test]
    fn only_pumps_outflows_and_closed_gates_are_solid_and_valves_carry_their_keys() {
        for world in WORLDS {
            for level in [LEVEL, LOWER_LEVEL] {
                for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1), (2, 1)] {
                    let f = built(world, cx, cy, level);
                    for p in &f.props {
                        if p.solid {
                            assert!(
                                ["sewerPump", "sewerOutflowPipe", SLUICE_GATE, "barrel", "crate", "locker"].contains(&p.ptype.as_str()),
                                "{} is solid",
                                p.ptype
                            );
                        }
                        if p.ptype == "sewerPipeRun" || p.ptype == "sewerValveWheel" || p.ptype == "sewerLamp" {
                            assert!(!p.solid, "{} must not be solid", p.ptype);
                        }
                        if p.ptype == "sewerValveWheel" {
                            let k = p.sewer.as_ref().expect("a valve carries its keys");
                            assert!(["flood", "gate", "flow", "redirect", "count"].contains(&k.effect.as_str()), "{}", k.effect);
                            assert_eq!(!k.gate.is_empty(), k.effect == "gate", "a gate valve names its gate");
                        }
                    }
                }
            }
        }
    }

    /// No Facility prop (its footprint's cell centres) stands on a water or waterfall cell, a gate aside.
    #[test]
    fn no_facility_prop_stands_in_water() {
        let mut bad = vec![];
        let mut count = 0;
        for world in WORLDS {
            for level in [LEVEL, LOWER_LEVEL] {
                for cx in -3..3 {
                    for cy in -3..3 {
                        let Some(sc) = sewer::chunk(Coord::new(cx, level, cy), &Inputs { world }) else { continue };
                        for p in props(&sc, &[], cat()) {
                            count += 1;
                            if p.ptype == SLUICE_GATE {
                                continue;
                            }
                            let (px, py) = (p.cell.x as f32 + 0.5 + p.offset.0, p.cell.y as f32 + 0.5 + p.offset.1);
                            for y in 0..CHUNK {
                                for x in 0..CHUNK {
                                    let inside = (x as f32 + 0.5 - px).abs() < p.half.0 && (y as f32 + 0.5 - py).abs() < p.half.1;
                                    let code = sc.sewer_grid[y as usize][x as usize];
                                    if (inside || (x, y) == (p.cell.x, p.cell.y))
                                        && (code == WATER || code == crate::world::route::sewer::core::FALL)
                                    {
                                        bad.push(format!("{world} {cx},{level},{cy} {} {} on {x},{y}", p.id, p.ptype));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(count > 100, "scanned {count} props");
        assert!(bad.is_empty(), "{} props on water; first: {:?}", bad.len(), &bad[..bad.len().min(6)]);
    }

    /// The chunk's open cells that a body can stand on, from its first one, 4-connected on cell centres.
    fn reach(f: &super::super::facility::Facility, from: Cell) -> HashSet<Cell> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([from]);
        seen.insert(from);
        while let Some(c) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if n.x < 0 || n.y < 0 || n.x >= CHUNK || n.y >= CHUNK || seen.contains(&n) {
                    continue;
                }
                if f.can_occupy(n.x as f64 + 0.5, n.y as f64 + 0.5, 0.23) {
                    seen.insert(n);
                    queue.push_back(n);
                }
            }
        }
        seen
    }

    /// Every cell a body could reach with no props at all is still reachable with them (a cell under a prop aside).
    #[test]
    fn props_cut_off_no_dry_cell() {
        let mut lost = vec![];
        for world in WORLDS {
            for level in [LEVEL, LOWER_LEVEL] {
                for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, -1), (2, 0), (0, 2)] {
                    let f = built(world, cx, cy, level);
                    let mut bare = f.clone();
                    bare.props.clear();
                    bare.rebuild_grids();
                    let from = f.start;
                    if !bare.can_occupy(from.x as f64 + 0.5, from.y as f64 + 0.5, 0.23) {
                        continue;
                    }
                    let with = reach(&f, from);
                    for c in reach(&bare, from) {
                        let under = f.props.iter().any(|p| {
                            p.solid
                                && (c.x as f32 + 0.5 - (p.cell.x as f32 + 0.5 + p.offset.0)).abs() < p.half.0 + 0.23
                                && (c.y as f32 + 0.5 - (p.cell.y as f32 + 0.5 + p.offset.1)).abs() < p.half.1 + 0.23
                        });
                        if !under && !with.contains(&c) {
                            lost.push(format!("{world} {cx},{level},{cy} cell {},{}", c.x, c.y));
                        }
                    }
                }
            }
        }
        assert!(lost.is_empty(), "{} cells cut off; first: {:?}", lost.len(), &lost[..lost.len().min(8)]);
    }

    /// A closed gate is a wall across its channel cells, an open one is floor.
    #[test]
    fn a_gate_closes_its_channel_until_its_valve_opens_it() {
        let mut tested = 0;
        for world in WORLDS {
            for level in [LEVEL, LOWER_LEVEL] {
                for cx in -4..4 {
                    for cy in -4..4 {
                        let mut f = built(world, cx, cy, level);
                        let Some(g) = f.props.iter().find(|p| p.ptype == SLUICE_GATE).cloned() else { continue };
                        let id = g.sewer.as_ref().unwrap().gate.clone();
                        assert!(g.solid && !id.is_empty());
                        let (x, y) = (g.cell.x as f64 + 0.5, g.cell.y as f64 + 0.5);
                        assert!(!f.can_occupy(x, y, 0.23), "closed: the channel cell is a wall");
                        assert!(f.set_gate_open(&id, true));
                        assert!(f.can_occupy(x, y, 0.23), "open: it is a crossing");
                        assert!(!f.set_gate_open(&id, true), "no change, no rebuild");
                        assert!(f.set_gate_open(&id, false));
                        assert!(!f.can_occupy(x, y, 0.23));
                        tested += 1;
                    }
                }
            }
        }
        assert!(tested > 0, "no sample chunk has a sluice gate");
    }
}
