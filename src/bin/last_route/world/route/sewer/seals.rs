//! The sealed utility rooms and the breakables of the sewer levels -1 and -2 (stage 3d-ii), ported from
//! `SewerRouteGenerator._streamed_schema` (sewer_route_generator.gd:531-556, the seal and pipe part), `_frame_door_seal`
//! (565-589), `_pipe_breakables` (914-940) and `_flood_cells` (961-985), with the parts of `SealPlanner` (seal_planner.gd,
//! `plan_rooms` 95-147, `specs` 172-193, `_tools` 197, `_room_gate` 149-168, the masks 254-317), `SealRules`
//! (seal_rules.gd) and `Breach` (breach.gd) that they reach.
//!
//! Godot's order inside `_streamed_schema`, kept here: the plan (`plan_rooms`, SEALS draws), the seal specs (SEAL-KINDS
//! draws), the frame of each spec (which rewrites two cells of the sewer grid per framed locked door), the sealed
//! cells, then the pipes' specs on the framed grid (their flood reads the grid after the frame).
//!
//! Not ported here (they are the pickups' and fixtures' stages): the relays and tools that sit against `sealed`, the
//! `gate_cells` neighbourhood of the seal gates, and the streamed walk grid after the frame (`streamed_grid` gives it
//! again: the frame keeps `walkable(sewer_grid)` equal to the streamed grid, so a later stage can recompute it).
//!
//! The sewer's levels only: the seal props are the sewer's (`SealRules.LEVEL_SEALS` -1 and -2). The Inferno (-3) is
//! stage 3e.

use std::collections::HashMap;

use crate::world::rng::Rng;
use crate::world::route::layout::{Coord, Grid, V2};

use super::breach::{self, BreachSpec, Extras, PipeExtras, SealExtras, KIND_PIPE, KIND_SEAL};
use super::core::{is_chunk_allowed, Rect, SewerProp, SewerRoom, BRIDGE, CHUNK, DRY, N, SOLID};

/// `SEAL_FIRST_INDEX` (sewer:52): the seals' breakable ids count from here, the pipes' from 0.
pub const SEAL_FIRST_INDEX: usize = 100;
/// `SealPlanner.MIN_ROOM` (seal_planner.gd:16): the fewest cells a room's gate may shut in.
const MIN_ROOM: i64 = 20;
/// `SealRules.SECOND_RELAY_SEALED` (seal_rules.gd:28), read by the relays (stage 3d-iii).
pub const SECOND_RELAY_SEALED: f64 = 0.80;
/// `SealRules.GUARANTEED_TOOL_RING` (seal_rules.gd:32).
const GUARANTEED_TOOL_RING: i32 = 2;
/// `SealRules.RINGS` (seal_rules.gd:16-21): (first ring, open share, seals per chunk). The open share is not used here.
const RINGS: [(i32, f64, f64); 4] = [(0, 0.75, 1.5), (2, 0.50, 2.5), (5, 0.35, 3.5), (9, 0.25, 5.0)];
/// `SealRules.SEAL_MIX` (seal_rules.gd:37), in Godot's order (the mix is built in this order).
const SEAL_MIX: [(&str, f64); 4] = [("barricade", 0.40), ("locked_door", 0.25), ("weak_wall", 0.20), ("vent_grate", 0.15)];
/// `PIPE_TOOLS` (sewer:907).
const PIPE_TOOLS: [&str; 2] = ["rescue_axe", "crowbar"];
/// `PIPE_HP` (sewer:908).
const PIPE_HP_CRAWL: i32 = 3;
const PIPE_HP_OUTFLOW: i32 = 4;
/// `FLOOD_REACH` and `FLOOD_MAX_CELLS` (sewer:910-911).
pub const FLOOD_REACH: i32 = 12;
pub const FLOOD_MAX_CELLS: usize = 160;

/// `SealRules.SEAL_HP` (seal_rules.gd:39), with `SEAL_HP.get(kind, 3)` (sewer:188 and 587).
pub fn seal_hp(kind: &str) -> i32 {
    match kind {
        "barricade" => 4,
        "locked_door" => 3,
        "weak_wall" => 4,
        "vent_grate" => 2,
        _ => 3,
    }
}

/// `SealRules.SEAL_TOOLS` (seal_rules.gd:43-44) as Godot names them ("axe"). The planner's `_tools` maps it (below).
fn seal_tools(kind: &str) -> &'static [&'static str] {
    match kind {
        "barricade" | "weak_wall" => &["axe", "crowbar"],
        "locked_door" => &["lockpick", "crowbar", "axe"],
        "vent_grate" => &["crowbar", "axe"],
        _ => &[],
    }
}

/// `SealPlanner._tools` (seal_planner.gd:197-201): the seal kind's tools, with "axe" named the player's "rescue_axe".
pub fn planner_tools(level: i32, kind: &str) -> Vec<&'static str> {
    let _ = level; // the sewer's seal tools have no per-level override (LEVEL_OVERRIDES holds only level 5)
    seal_tools(kind)
        .iter()
        .map(|t| if *t == "axe" { "rescue_axe" } else { *t })
        .collect()
}

/// `SealRules.LEVEL_SEALS` (seal_rules.gd:57-58) for the sewer levels: each seal kind's themed prop, "" where the kind
/// does not occur. Levels 0 to 5 and the Inferno (-3) are other stages and panic here.
pub fn seal_prop(level: i32, kind: &str) -> &'static str {
    let themed: [(&str, &'static str); 4] = match level {
        -1 | -2 => [
            ("barricade", "sluiceGate"),
            ("locked_door", "lockedMaintenanceDoor"),
            ("weak_wall", "brickedUpWall"),
            ("vent_grate", ""),
        ],
        _ => panic!("sewer seal props are ported for levels -1 and -2 only, not {level}"),
    };
    themed.iter().find(|(k, _)| *k == kind).map(|(_, p)| *p).unwrap_or("")
}

/// `SealRules.seal_mix` (seal_rules.gd:94-104): SEAL_MIX restricted to the kinds the level themes, renormalised. The
/// total is summed in Godot's order.
pub fn seal_mix(level: i32) -> Vec<(&'static str, f64)> {
    let themed: Vec<&'static str> = SEAL_MIX
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| !seal_prop(level, k).is_empty())
        .collect();
    let mut total = 0.0;
    for (kind, share) in SEAL_MIX {
        if themed.contains(&kind) {
            total += share;
        }
    }
    SEAL_MIX
        .iter()
        .filter(|(k, _)| themed.contains(k))
        .map(|(k, share)| (*k, *share / total))
        .collect()
}

/// `SealRules.ring` (seal_rules.gd:68): the Chebyshev distance of the chunk from spawn (the y level is not in it).
pub fn ring(coord: Coord) -> i32 {
    coord.x.abs().max(coord.z.abs())
}

/// `SealRules.seals_per_chunk` (seal_rules.gd:85): the last band whose first ring the chunk reaches.
pub fn seals_per_chunk(coord: Coord) -> f64 {
    let r = ring(coord);
    let mut band = RINGS[0];
    for entry in RINGS {
        if r >= entry.0 {
            band = entry;
        }
    }
    band.2
}

/// `SealRules.guaranteed_tool` (seal_rules.gd:111), read by the pickups (stage 3d-iii).
pub fn guaranteed_tool(coord: Coord) -> bool {
    ring(coord) >= GUARANTEED_TOOL_RING
}

/// The walk grid `_streamed_schema` builds from the sewer grid (sewer:519-524): 0 where `is_walkable` (dry floor or a
/// bridge deck), 1 elsewhere.
pub fn streamed_grid(sewer_grid: &Grid) -> Grid {
    let mut out = [[1u8; N]; N];
    for z in 0..N {
        for x in 0..N {
            let code = sewer_grid[z][x];
            out[z][x] = if code == DRY || code == BRIDGE { 0 } else { 1 };
        }
    }
    out
}

/// A gate of a plan (`SealPlanner`'s gate dictionary): the cells it shuts, its centre, its direction, its radius and
/// the index of the tunnel (here the room) it sits on. `shut` is the count of cells it closes off.
#[derive(Clone, Debug, PartialEq)]
pub struct SealGate {
    pub cells: Vec<V2>,
    pub centre: [f64; 2],
    pub dir: [f64; 2],
    pub radius: f64,
    pub tunnel: usize,
    pub shut: i64,
}

/// `plan_rooms`'s result: the chosen gates and the sealed cells in z-major order (`result["sealed"]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SealPlan {
    pub gates: Vec<SealGate>,
    pub sealed: Vec<V2>,
}

const NN: usize = N * N;

fn idx(c: V2) -> usize {
    c.y as usize * N + c.x as usize
}

fn inb(c: V2) -> bool {
    c.x >= 0 && c.y >= 0 && c.x < CHUNK && c.y < CHUNK
}

/// `_open_mask` (seal_planner.gd:254): 1 where the grid is 0.
fn open_mask(grid: &Grid) -> Vec<u8> {
    let mut m = vec![0u8; NN];
    for z in 0..N {
        for x in 0..N {
            if grid[z][x] == 0 {
                m[z * N + x] = 1;
            }
        }
    }
    m
}

/// `_block_mask` (seal_planner.gd:265): the gates' cells.
fn block_mask(gates: &[SealGate]) -> Vec<u8> {
    let mut m = vec![0u8; NN];
    for gate in gates {
        for cell in &gate.cells {
            m[idx(*cell)] = 1;
        }
    }
    m
}

/// `_count` (seal_planner.gd:275): the cells where both masks are set.
fn count(mask: &[u8], within: &[u8]) -> i64 {
    mask.iter().zip(within).map(|(a, b)| (a & b) as i64).sum()
}

/// `_reach_mask` (seal_planner.gd:283): the flood from the entries over open, unblocked cells (4-neighbour).
fn reach_mask(open: &[u8], entries: &[V2], blocked: &[u8]) -> Vec<u8> {
    let mut seen = vec![0u8; NN];
    let mut stack: Vec<usize> = Vec::new();
    for e in entries {
        let at = idx(*e);
        if open[at] == 1 && blocked[at] == 0 && seen[at] == 0 {
            stack.push(at);
            seen[at] = 1;
        }
    }
    while let Some(at) = stack.pop() {
        let x = at % N;
        let mut next = [None; 4];
        if x > 0 {
            next[0] = Some(at - 1);
        }
        if x < N - 1 {
            next[1] = Some(at + 1);
        }
        if at >= N {
            next[2] = Some(at - N);
        }
        if at < N * (N - 1) {
            next[3] = Some(at + N);
        }
        for n in next.into_iter().flatten() {
            if open[n] == 1 && seen[n] == 0 && blocked[n] == 0 {
                seen[n] = 1;
                stack.push(n);
            }
        }
    }
    seen
}

/// `_all_reached` (seal_planner.gd:312).
fn all_reached(seen: &[u8], entries: &[V2]) -> bool {
    entries.iter().all(|e| seen[idx(*e)] != 0)
}

/// `_room_gate` (seal_planner.gd:149-168): the gate across a room's doorway, the three cells just outside the side
/// of the rect whose middle outside cell is open, one deep, with the cell beyond the gate open too.
fn room_gate(grid: &Grid, rect: Rect, index: usize) -> Option<SealGate> {
    let middle = rect.center();
    for side in [V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)] {
        let along = V2::new(-side.y, side.x);
        let edge = V2::new(middle.x + side.x * (rect.w / 2 + 1), middle.y + side.y * (rect.h / 2 + 1));
        let mut cells = Vec::new();
        for k in -1..=1 {
            let cell = V2::new(edge.x + along.x * k, edge.y + along.y * k);
            if cell.x < 1 || cell.y < 1 || cell.x > CHUNK - 2 || cell.y > CHUNK - 2 || grid[cell.y as usize][cell.x as usize] != 0 {
                break;
            }
            cells.push(cell);
        }
        if cells.len() != 3 {
            continue;
        }
        // a doorway is a real branch: the cell beyond the gate is open too
        let beyond = V2::new(edge.x + side.x, edge.y + side.y);
        if grid[beyond.y as usize][beyond.x as usize] != 0 {
            continue;
        }
        return Some(SealGate {
            cells,
            centre: [edge.x as f64 + 0.5, edge.y as f64 + 0.5],
            dir: [side.x as f64, side.y as f64],
            radius: 1.5,
            tunnel: index,
            shut: 0,
        });
    }
    None
}

/// `SealPlanner.plan_rooms` (seal_planner.gd:95-144) on the sewer's walk grid. Draws: one float for the seal count
/// (SEALS:x:y:z), then one index per swap of the candidate shuffle.
pub fn plan_rooms(world: &str, coord: Coord, grid: &Grid, rooms: &[SewerRoom]) -> SealPlan {
    // a corner of the 13 x 13 junction chamber: floor on both sewer levels (its middle is the water channel crossing)
    let centre_cell = V2::new(CHUNK / 2 - 6, CHUNK / 2 - 6);
    if rooms.is_empty() || grid[centre_cell.y as usize][centre_cell.x as usize] != 0 {
        return SealPlan::default();
    }
    let entries = [centre_cell];
    let candidates: Vec<SealGate> = rooms
        .iter()
        .enumerate()
        .filter_map(|(index, room)| room_gate(grid, room.rect, index))
        .collect();
    if candidates.is_empty() {
        return SealPlan::default();
    }
    let mut rng = Rng::new(&format!("{world}:SEALS:{}:{}:{}", coord.x, coord.y, coord.z));
    let base = seals_per_chunk(coord);
    // floori(base) + (1 if next_float < base - floor(base) else 0): the draw happens for every chunk
    let want = base.floor() as usize + usize::from(rng.next_float() < base - base.floor());
    let open = open_mask(grid);
    let base_reach = reach_mask(&open, &entries, &vec![0u8; NN]);
    let base_open = count(&base_reach, &open);
    let mut usable: Vec<SealGate> = Vec::new();
    for gate in candidates {
        let blocked = block_mask(std::slice::from_ref(&gate));
        let reach = reach_mask(&open, &entries, &blocked);
        let shut = base_open - count(&reach, &open) - count(&blocked, &open);
        if all_reached(&reach, &entries) && shut >= MIN_ROOM {
            usable.push(SealGate { shut, ..gate });
        }
    }
    if usable.is_empty() {
        return SealPlan::default();
    }
    rng.shuffle(&mut usable);
    let take = want.max(1).min(usable.len());
    usable.truncate(take);
    let chosen = usable;
    let blocked_all = block_mask(&chosen);
    let reach_all = reach_mask(&open, &entries, &blocked_all);
    let mut sealed = Vec::new();
    for z in 0..N {
        for x in 0..N {
            let at = z * N + x;
            if base_reach[at] == 1 && reach_all[at] == 0 && blocked_all[at] == 0 {
                sealed.push(V2::new(x as i32, z as i32));
            }
        }
    }
    if sealed.is_empty() {
        return SealPlan::default();
    }
    SealPlan { gates: chosen, sealed }
}

/// `SealPlanner.specs` (seal_planner.gd:172-193): one spec per gate, each drawing its seal kind from the level's mix
/// (SEAL-KINDS:x:y:z, one float per gate). The ids count from `first_index`.
pub fn plan_specs(world: &str, coord: Coord, plan: &SealPlan, level: i32, first_index: usize) -> Vec<BreachSpec> {
    let mut rng = Rng::new(&format!("{world}:SEAL-KINDS:{}:{}:{}", coord.x, coord.y, coord.z));
    let mix = seal_mix(level);
    let mut out = Vec::new();
    for gate in &plan.gates {
        let roll = rng.next_float();
        // the first kind when nothing matches (the mix is renormalised, so a roll past its sum falls through)
        let mut seal_kind: &'static str = mix.first().map(|(k, _)| *k).unwrap_or("barricade");
        let mut acc = 0.0;
        for &(kind, share) in &mix {
            acc += share;
            if roll <= acc {
                seal_kind = kind;
                break;
            }
        }
        let variant = seal_prop(level, seal_kind);
        let hp = seal_hp(seal_kind);
        let cell = V2::new(gate.centre[0] as i32, gate.centre[1] as i32);
        out.push(breach::make_spec(
            breach::make_id(coord, first_index + out.len()),
            KIND_SEAL,
            variant,
            coord,
            cell,
            hp,
            planner_tools(level, seal_kind),
            Extras::Seal(SealExtras {
                seal: seal_kind,
                cells: gate.cells.clone(),
                centre: gate.centre,
                dir: gate.dir,
                radius: gate.radius,
                tunnel: gate.tunnel,
                door_frame: None,
            }),
            seal_kind == "locked_door",
        ));
    }
    out
}

/// `_frame_door_seal` (sewer:569-589). A locked door needs a wall on both jambs: where the gate's three cells have
/// solid cells beyond both ends, the two outer cells become brick (in `sewer_grid` and in the streamed grid) and the
/// spec gets `door_frame`. Anywhere else the seal becomes a barricade.
pub fn frame_door_seal(spec: &mut BreachSpec, streamed: &mut Grid, sewer_grid: &mut Grid, level: i32) {
    let cells = match &spec.extras {
        Extras::Seal(s) if s.seal == "locked_door" => s.cells.clone(),
        _ => return,
    };
    let along = V2::new(cells[1].x - cells[0].x, cells[1].y - cells[0].y);
    let jambs = [V2::new(cells[0].x - along.x, cells[0].y - along.y), V2::new(cells[2].x + along.x, cells[2].y + along.y)];
    let framed = jambs
        .iter()
        .all(|c| inb(*c) && sewer_grid[c.y as usize][c.x as usize] == SOLID);
    if framed {
        for side in [cells[0], cells[2]] {
            sewer_grid[side.y as usize][side.x as usize] = SOLID;
            streamed[side.y as usize][side.x as usize] = 1;
        }
        if let Extras::Seal(s) = &mut spec.extras {
            s.door_frame = Some(jambs);
        }
        return;
    }
    if let Extras::Seal(s) = &mut spec.extras {
        s.seal = "barricade";
    }
    spec.variant = seal_prop(level, "barricade");
    spec.hp = seal_hp("barricade");
    spec.tools = planner_tools(level, "barricade");
    spec.locked = false;
}

/// `_flood_cells` (sewer:961-985): a walk outward from the pipe's cells over the bank's dry floor (`!= DRY` stops it,
/// so a bridge deck does not carry the water), FLOOD_REACH steps, nearest first, at most FLOOD_MAX_CELLS cells
/// (the pipe's own cells included). Returns the cells and each one's step.
pub fn flood_cells(sewer_grid: &Grid, cells: &[V2]) -> (Vec<V2>, Vec<i32>) {
    let mut seen: HashMap<V2, i32> = HashMap::new();
    let mut queue: Vec<V2> = Vec::new();
    for cell in cells {
        seen.insert(*cell, 0);
        queue.push(*cell);
    }
    let mut cursor = 0;
    while cursor < queue.len() && queue.len() < FLOOD_MAX_CELLS {
        let cell = queue[cursor];
        cursor += 1;
        let step_here = seen[&cell];
        if step_here >= FLOOD_REACH {
            continue;
        }
        for step in [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)] {
            let next = V2::new(cell.x + step.x, cell.y + step.y);
            if seen.contains_key(&next) || !inb(next) {
                continue;
            }
            if sewer_grid[next.y as usize][next.x as usize] != DRY {
                continue;
            }
            seen.insert(next, step_here + 1);
            queue.push(next);
            if queue.len() >= FLOOD_MAX_CELLS {
                break;
            }
        }
    }
    let steps = queue.iter().map(|c| seen[c]).collect();
    (queue, steps)
}

/// `_pipe_breakables` (sewer:914-940): one spec per pipe (a crawl pipe's two mains share one spec, by their cells).
/// Sets `breakable_id` on each pipe prop it covers. Ids count from 0.
pub fn pipe_breakables(coord: Coord, sewer_grid: &Grid, region_props: &mut [SewerProp]) -> Vec<BreachSpec> {
    let mut out: Vec<BreachSpec> = Vec::new();
    let mut by_cells: HashMap<Vec<V2>, usize> = HashMap::new();
    for prop in region_props.iter_mut() {
        let pipe: &'static str = if prop.outflow {
            "outflow"
        } else if prop.crawl_under {
            "crawl"
        } else {
            continue;
        };
        let cells = prop.occupied_cells.clone().expect("a pipe prop has occupied_cells");
        if let Some(&k) = by_cells.get(&cells) {
            if let Extras::Pipe(p) = &mut out[k].extras {
                p.props.push(prop.id.clone());
            }
            prop.breakable_id = Some(out[k].id.clone());
            continue;
        }
        let middle = cells[cells.len() / 2];
        let along = prop.crawl_step.expect("a pipe prop has crawl_step");
        let (flood, steps) = flood_cells(sewer_grid, &cells);
        let wall = prop.outflow_wall.unwrap_or_default();
        let hp = if pipe == "crawl" { PIPE_HP_CRAWL } else { PIPE_HP_OUTFLOW };
        let id = breach::make_id(coord, out.len());
        prop.breakable_id = Some(id.clone());
        by_cells.insert(cells.clone(), out.len());
        out.push(breach::make_spec(
            id,
            KIND_PIPE,
            "authored",
            coord,
            middle,
            hp,
            PIPE_TOOLS.to_vec(),
            Extras::Pipe(PipeExtras {
                pipe,
                props: vec![prop.id.clone()],
                cells: cells.clone(),
                centre: [middle.x as f64 + 0.5, middle.y as f64 + 0.5],
                dir: [along.x as f64, along.y as f64],
                flood_cells: flood,
                flood_steps: steps,
                wall: [wall.x as f64, wall.y as f64],
            }),
            false,
        ));
    }
    out
}

/// The breakables and sealed cells of one sewer chunk (`_streamed_schema` 531-556).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkBreakables {
    /// The pipes' specs (ids from 0) then the seals' (ids from SEAL_FIRST_INDEX), in Godot's order.
    pub specs: Vec<BreachSpec>,
    /// `sealed_cells`: the cells of the sealed wings, z-major, as `plan_rooms` lists them.
    pub sealed: Vec<V2>,
}

/// The seal step and the pipe breakables of one sewer chunk, in `_streamed_schema`'s order. `sewer_grid` is the
/// chunk's grid before the step; the step rewrites the framed locked doors' cells in it (so it is `&mut`). `region_props`
/// get their `breakable_id`. The seal planning runs only for a chunk with rooms (sewer:535).
pub fn chunk_breakables(
    coord: Coord,
    world: &str,
    sewer_grid: &mut Grid,
    rooms: &[SewerRoom],
    region_props: &mut [SewerProp],
) -> ChunkBreakables {
    let mut seal_specs = Vec::new();
    let mut sealed = Vec::new();
    if is_chunk_allowed(coord) && !rooms.is_empty() {
        let mut streamed = streamed_grid(sewer_grid);
        let plan = plan_rooms(world, coord, &streamed, rooms);
        seal_specs = plan_specs(world, coord, &plan, coord.y, SEAL_FIRST_INDEX);
        for spec in seal_specs.iter_mut() {
            frame_door_seal(spec, &mut streamed, sewer_grid, coord.y);
        }
        sealed = plan.sealed;
    }
    let mut specs = pipe_breakables(coord, sewer_grid, region_props);
    specs.extend(seal_specs);
    ChunkBreakables { specs, sealed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::sewer::core::{chunk_base as chunk, Inputs};
    use serde_json::{json, Value};

    /// One golden chunk of levels -1 or -2: `data` is the compact extract's (`sewer-seals-<tag>.json`) or the full
    /// dump's chunk data (PARITY_DIR), and `compact` tells which.
    struct Golden {
        tag: String,
        world: String,
        coord: Coord,
        data: Value,
        compact: bool,
    }

    fn parity_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("parity")
    }

    fn read_json(path: &std::path::Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("golden json")
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v.as_array().expect("coord");
        Coord::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32, a[2].as_i64().unwrap() as i32)
    }

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];

    fn load_compact(tag: &str) -> Vec<Golden> {
        let v = read_json(&parity_dir().join(format!("sewer-seals-{tag}.json")));
        let world = v["world_seed"].as_str().expect("world_seed").to_string();
        v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .map(|c| Golden { tag: tag.to_string(), world: world.clone(), coord: coord_of(&c["coord"]), data: c["data"].clone(), compact: true })
            .collect()
    }

    /// The full dump's sewer chunks (levels -1 and -2) of one seed. Needs PARITY_DIR.
    fn load_full(dir: &str, tag: &str) -> Vec<Golden> {
        let v = read_json(&std::path::Path::new(dir).join(format!("golden-{tag}.json")));
        let world = v["world_seed"].as_str().expect("world_seed").to_string();
        v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| c["coord"][1].as_i64().is_some_and(|y| y == -1 || y == -2))
            .map(|c| Golden { tag: tag.to_string(), world: world.clone(), coord: coord_of(&c["coord"]), data: c["data"].clone(), compact: false })
            .collect()
    }

    /// The flood encoding of the compact extract: 45 symbols, three per flood cell (x, y, step).
    const B45: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHI";

    fn b45_value(ch: u8) -> i32 {
        B45.iter().position(|c| *c == ch).expect("base-45 symbol") as i32
    }

    /// The golden breakables in the spec's JSON shape: the compact extract's flood (`flood_b45`) decoded back to
    /// `flood_cells` and `flood_steps`.
    fn want_breakables(data: &Value) -> Value {
        let mut list = data["breakables"].as_array().expect("breakables").clone();
        for b in list.iter_mut() {
            let Some(extras) = b["extras"].as_object_mut() else { continue };
            let Some(Value::String(packed)) = extras.remove("flood_b45") else { continue };
            let bytes = packed.as_bytes();
            let mut cells = Vec::new();
            let mut steps = Vec::new();
            for triple in bytes.chunks(3) {
                cells.push(json!([b45_value(triple[0]), b45_value(triple[1])]));
                steps.push(json!(b45_value(triple[2])));
            }
            extras.insert("flood_cells".into(), Value::Array(cells));
            extras.insert("flood_steps".into(), Value::Array(steps));
        }
        Value::Array(list)
    }

    /// The golden sealed cells in the golden's order: the compact extract stores two base-45 symbols per cell (x, y).
    fn want_sealed(data: &Value, compact: bool) -> Value {
        if !compact {
            return data["sealed_cells"].clone();
        }
        let packed = data["sealed_b45"].as_str().expect("sealed_b45");
        Value::Array(
            packed
                .as_bytes()
                .chunks(2)
                .map(|pair| json!([b45_value(pair[0]), b45_value(pair[1])]))
                .collect(),
        )
    }

    /// The golden's (prop id, breakable id) pairs of the pipe props, in region_props order.
    fn want_pipe_props(data: &Value, compact: bool) -> Value {
        if compact {
            return data["pipe_props"].clone();
        }
        Value::Array(
            data["region_props"]
                .as_array()
                .expect("region_props")
                .iter()
                .filter(|p| p.get("breakable_id").is_some())
                .map(|p| json!([p["id"], p["breakable_id"]]))
                .collect(),
        )
    }

    /// Numbers agree to 1e-9 (the golden prints whole floats without a point). An array element or object item named
    /// by its `id` shows in the path.
    fn first_diff(path: &str, ours: &Value, want: &Value) -> Option<String> {
        match (ours, want) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, av) in a {
                    match b.get(k) {
                        None => return Some(format!("{path}.{k}: only in ours")),
                        Some(bv) => {
                            if let Some(d) = first_diff(&format!("{path}.{k}"), av, bv) {
                                return Some(d);
                            }
                        }
                    }
                }
                b.keys().find(|k| !a.contains_key(*k)).map(|k| format!("{path}.{k}: only in Godot"))
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Some(format!("{path}: {} items vs Godot {}", a.len(), b.len()));
                }
                for (i, (av, bv)) in a.iter().zip(b).enumerate() {
                    let name = bv.get("id").and_then(|v| v.as_str()).map(|id| format!("[{i}]({id})")).unwrap_or(format!("[{i}]"));
                    if let Some(d) = first_diff(&format!("{path}{name}"), av, bv) {
                        return Some(d);
                    }
                }
                None
            }
            (Value::Number(a), Value::Number(b)) => {
                let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                ((x - y).abs() > 1e-9).then(|| format!("{path}: {x} vs Godot {y}"))
            }
            _ => (ours != want).then(|| format!("{path}: {ours} vs Godot {want}")),
        }
    }

    /// Per (seed, level): chunks checked and chunks matching, and the first mismatches, for one field.
    fn check_field(goldens: &[Golden], field: &str) -> Vec<String> {
        let mut counts: std::collections::BTreeMap<(String, i32), (usize, usize)> = Default::default();
        let mut mismatches = Vec::new();
        for g in goldens {
            let entry = counts.entry((g.tag.clone(), g.coord.y)).or_default();
            entry.0 += 1;
            let at = format!("{} {:?}", g.tag, (g.coord.x, g.coord.y, g.coord.z));
            let Some(sewer) = chunk(g.coord, &Inputs { world: &g.world }) else {
                mismatches.push(format!("{at} {field}: no sewer chunk"));
                continue;
            };
            let (ours, want) = match field {
                "breakables" => {
                    let ours = Value::Array(sewer.breakables.iter().map(BreachSpec::to_json).collect());
                    (ours, want_breakables(&g.data))
                }
                "sealed_cells" => {
                    let ours = Value::Array(sewer.sealed_cells.iter().map(|c| json!([c.x, c.y])).collect());
                    (ours, want_sealed(&g.data, g.compact))
                }
                "pipe_props" => {
                    let ours = Value::Array(
                        sewer
                            .region_props
                            .iter()
                            .filter_map(|p| p.breakable_id.as_ref().map(|b| json!([p.id, b])))
                            .collect(),
                    );
                    (ours, want_pipe_props(&g.data, g.compact))
                }
                other => panic!("unknown field {other}"),
            };
            match first_diff(&format!("{at} {field}"), &ours, &want) {
                None => entry.1 += 1,
                Some(d) => mismatches.push(d),
            }
        }
        for ((tag, level), (n, ok)) in &counts {
            eprintln!("  {field} seed {tag} level {level}: matched {ok} of {n} chunks");
        }
        mismatches
    }

    fn assert_field(goldens: &[Golden], field: &str) {
        let mismatches = check_field(goldens, field);
        for m in mismatches.iter().take(25) {
            eprintln!("MISMATCH {m}");
        }
        assert!(mismatches.is_empty(), "{} {field} mismatches; first: {}", mismatches.len(), mismatches.first().cloned().unwrap_or_default());
    }

    fn compact_goldens() -> Vec<Golden> {
        TAGS.iter().flat_map(|t| load_compact(t)).collect()
    }

    /// `breakables` of levels -1 and -2 for the three goldens: every spec's id, kind, variant, cell, hp, tools,
    /// locked flag and extras (the pipes' flood cells and steps included), in Godot's order, and the pipe props'
    /// `breakable_id`.
    #[test]
    fn breakables_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let goldens = compact_goldens();
        assert_field(&goldens, "breakables");
        assert_field(&goldens, "pipe_props");
    }

    /// `sealed_cells` of levels -1 and -2 for the three goldens, in z-major order.
    #[test]
    fn sealed_cells_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        assert_field(&compact_goldens(), "sealed_cells");
    }

    /// The full goldens (PARITY_DIR), raw: no flood encoding, no mask rows. Skips with a note when PARITY_DIR is unset.
    #[test]
    fn breakables_and_sealed_match_full_goldens() {
        let Ok(dir) = std::env::var("PARITY_DIR") else {
            eprintln!("PARITY_DIR unset: the full-golden seal check is skipped");
            return;
        };
        let mut goldens = Vec::new();
        for tag in TAGS {
            goldens.extend(load_full(&dir, tag));
        }
        for field in ["breakables", "sealed_cells", "pipe_props"] {
            assert_field(&goldens, field);
        }
    }

    #[test]
    fn seal_mix_is_the_themed_kinds_renormalised() {
        for level in [-1, -2] {
            let mix = seal_mix(level);
            let kinds: Vec<&str> = mix.iter().map(|(k, _)| *k).collect();
            assert_eq!(kinds, ["barricade", "locked_door", "weak_wall"]);
            let total: f64 = mix.iter().map(|(_, s)| s).sum();
            assert!((total - 1.0).abs() < 1e-12);
            assert!((mix[0].1 - 0.40 / 0.85).abs() < 1e-12);
        }
        assert_eq!(seal_prop(-1, "barricade"), "sluiceGate");
        assert_eq!(seal_prop(-2, "locked_door"), "lockedMaintenanceDoor");
        assert_eq!(seal_prop(-1, "vent_grate"), "");
    }

    #[test]
    fn seal_bands_and_tools_match_the_rules() {
        let c = |x, z| Coord::new(x, -1, z);
        assert_eq!(seals_per_chunk(c(0, 1)), 1.5);
        assert_eq!(seals_per_chunk(c(-2, 0)), 2.5);
        assert_eq!(seals_per_chunk(c(4, -4)), 2.5);
        assert_eq!(seals_per_chunk(c(5, 0)), 3.5);
        assert_eq!(seals_per_chunk(c(8, 8)), 3.5);
        assert_eq!(seals_per_chunk(c(9, 0)), 5.0);
        assert!(!guaranteed_tool(c(1, 1)));
        assert!(guaranteed_tool(c(2, 0)));
        assert_eq!(planner_tools(-1, "barricade"), ["rescue_axe", "crowbar"]);
        assert_eq!(planner_tools(-2, "locked_door"), ["lockpick", "crowbar", "rescue_axe"]);
        assert_eq!(SECOND_RELAY_SEALED, 0.80);
    }

    #[test]
    fn ids_count_pipes_from_zero_and_seals_from_100() {
        assert_eq!(breach::make_id(Coord::new(-3, -1, 4), 7), "BRK:-3:-1:4:7");
        assert_eq!(SEAL_FIRST_INDEX, 100);
        assert_eq!(seal_hp("weak_wall"), 4);
        assert_eq!(seal_hp("nope"), 3);
    }

    /// A locked door whose jambs are not both brick falls back to a barricade (`_frame_door_seal`'s else branch).
    #[test]
    fn unframed_locked_door_becomes_a_barricade() {
        let cells = vec![V2::new(10, 9), V2::new(10, 10), V2::new(10, 11)];
        let mut spec = breach::make_spec(
            breach::make_id(Coord::new(0, -1, 0), 100),
            KIND_SEAL,
            "lockedMaintenanceDoor",
            Coord::new(0, -1, 0),
            V2::new(10, 10),
            3,
            planner_tools(-1, "locked_door"),
            Extras::Seal(SealExtras {
                seal: "locked_door",
                cells,
                centre: [10.5, 10.5],
                dir: [0.0, 1.0],
                radius: 1.5,
                tunnel: 0,
                door_frame: None,
            }),
            true,
        );
        let mut streamed = [[1u8; N]; N];
        let mut sewer = [[SOLID; N]; N];
        // the gate's cells are open; the jamb at (10, 12) is open too, so the door is not framed
        for z in 9..=11 {
            sewer[z][10] = DRY;
        }
        sewer[12][10] = DRY;
        frame_door_seal(&mut spec, &mut streamed, &mut sewer, -1);
        assert_eq!(spec.variant, "sluiceGate");
        assert_eq!(spec.hp, 4);
        assert_eq!(spec.tools, ["rescue_axe", "crowbar"]);
        assert!(!spec.locked);
        match &spec.extras {
            Extras::Seal(s) => {
                assert_eq!(s.seal, "barricade");
                assert!(s.door_frame.is_none());
            }
            _ => panic!("a seal"),
        }
        assert_eq!(sewer[9][10], DRY, "a barricade leaves the grid alone");
        assert_eq!(sewer[12][10], DRY);
    }

    /// A framed locked door: both outer cells become brick in both grids, and `door_frame` holds the jambs.
    #[test]
    fn framed_locked_door_bricks_its_outer_cells() {
        let cells = vec![V2::new(10, 9), V2::new(10, 10), V2::new(10, 11)];
        let mut spec = breach::make_spec(
            breach::make_id(Coord::new(0, -1, 0), 100),
            KIND_SEAL,
            "lockedMaintenanceDoor",
            Coord::new(0, -1, 0),
            V2::new(10, 10),
            3,
            planner_tools(-1, "locked_door"),
            Extras::Seal(SealExtras { seal: "locked_door", cells, centre: [10.5, 10.5], dir: [0.0, 1.0], radius: 1.5, tunnel: 0, door_frame: None }),
            true,
        );
        let mut streamed = [[0u8; N]; N];
        let mut sewer = [[SOLID; N]; N];
        for z in 9..=11 {
            sewer[z][10] = DRY;
            streamed[z][10] = 0;
        }
        frame_door_seal(&mut spec, &mut streamed, &mut sewer, -1);
        assert_eq!(sewer[9][10], SOLID);
        assert_eq!(sewer[11][10], SOLID);
        assert_eq!(sewer[10][10], DRY);
        assert_eq!(streamed[9][10], 1);
        assert_eq!(streamed[10][10], 0);
        assert_eq!(spec.variant, "lockedMaintenanceDoor");
        assert!(spec.locked);
        match &spec.extras {
            Extras::Seal(s) => assert_eq!(s.door_frame, Some([V2::new(10, 8), V2::new(10, 12)])),
            _ => panic!("a seal"),
        }
    }

    #[test]
    fn flood_is_capped_and_walks_dry_floor_only() {
        let mut grid = [[DRY; N]; N];
        grid[5][6] = BRIDGE; // a bridge deck next to the pipe is not flooded
        let (cells, steps) = flood_cells(&grid, &[V2::new(5, 5)]);
        // 160 cells (`FLOOD_MAX_CELLS`, sewer:911); the goldens' floods peak at 125, so the cap is only tested here
        assert_eq!(cells.len(), 160);
        assert_eq!(cells[0], V2::new(5, 5));
        assert_eq!(steps[0], 0);
        assert!(!cells.contains(&V2::new(6, 5)), "a bridge deck is not flooded");
        assert!(steps.iter().all(|s| *s <= FLOOD_REACH + 1));
        // a step-1 cell is next to the pipe, and the cells come nearest first
        assert!(steps.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn streamed_grid_is_walkable_only_on_dry_and_bridge() {
        let mut sewer = [[SOLID; N]; N];
        sewer[0][0] = DRY;
        sewer[0][1] = BRIDGE;
        sewer[0][2] = 2; // water
        let s = streamed_grid(&sewer);
        assert_eq!((s[0][0], s[0][1], s[0][2], s[0][3]), (0, 0, 1, 1));
    }
}
