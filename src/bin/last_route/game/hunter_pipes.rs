//! The hunters' side of the pipes: noise events placed on the storey where the sound happens, and the window cells a
//! running pipe flow floods (the hunters' `WaterField` input). Behaviour only: the loudness, radii and flood depth
//! are data (`data/hunters/noise_events.json`, `data/pipes.json`, the sewer's own `flood_cells` / `flood_steps`).

use super::hunting::PipeCells;
use super::pipes::{self, TurnEvent};
use super::Game;
use crate::hunters::noise_events::{NoiseEvent, NoiseKind};
use crate::world::route::sewer::breach::Extras;
use crate::world::route::sewer::core as sewer_core;

impl Game {
    /// A noise event at `pos` on storey `level` (the place the sound happens, not the runner's label). A guest
    /// records nothing: its runner never wakes the host's hunters.
    pub fn push_noise_at(&mut self, kind: NoiseKind, pos: (f32, f32), level: i32) {
        if self.remote_hunters {
            return;
        }
        self.noise_events.push(NoiseEvent::new(kind, pos, level));
    }

    /// `push_noise_at` with the kind's loudness times `scale` (a taller fall is heard farther; the event's radius is
    /// still capped by the data `max_radius`).
    pub fn push_noise_scaled(&mut self, kind: NoiseKind, pos: (f32, f32), level: i32, scale: f32) {
        if self.remote_hunters {
            return;
        }
        let e = NoiseEvent::new(kind, pos, level);
        let loud = e.loudness * scale.max(0.0);
        self.noise_events.push(e.with_loudness(loud));
    }

    /// The noise events of a valve turn, whoever turned it: the turn itself, and the sluice when an odd turn of a
    /// gate valve lowers the gate. The runner's meter noise (`TurnEvent::noise`) is separate and player-only.
    pub fn push_valve_events(&mut self, ev: &TurnEvent, at: (f32, f32), level: i32) {
        if !ev.noise_event() {
            return;
        }
        self.push_noise_at(NoiseKind::ValveTurn, at, level);
        if !ev.gate.is_empty() && ev.turns % 2 == 1 {
            self.push_noise_at(NoiseKind::SluiceOpen, at, level);
        }
    }
}

/// `(window cell, step)` for a flow's flood cells: the chunk's absolute cells minus the window origin, zipped with
/// each cell's step from the pipe. A shorter `steps` list ends the zip.
pub fn cells_for_flow(flood_cells: &[(i32, i32)], steps: &[i32], origin: (i32, i32)) -> Vec<((i32, i32), i32)> {
    flood_cells.iter().zip(steps.iter()).map(|(c, s)| ((c.0 - origin.0, c.1 - origin.1), *s)).collect()
}

/// The window cells every flow of the sewer chunk in `water_cache` reaches. A burst flow `BRK:x:y:z:<n>` floods the
/// chunk's breakable number `n`; a flow valve's flow, named after its valve, floods the pipe that lists the valve
/// among its props. The sewer gives one flood list per pipe, so every outlet uses it.
pub(super) fn cells(g: &Game) -> Vec<PipeCells> {
    let mut out = sewer_cells(g);
    out.extend(authored_cells(g, authored_floods()));
    out
}

fn sewer_cells(g: &Game) -> Vec<PipeCells> {
    let Some((key, Some(ch))) = &g.water_cache else { return vec![] };
    let origin = g.window_origin();
    let base = (key.0 * sewer_core::CHUNK, key.2 * sewer_core::CHUNK);
    let prefix = pipes::flow_prefix(*key);
    let mut out = vec![];
    for (id, outlet, n) in g.pipes.flows_in(*key) {
        let rest = id.strip_prefix(&prefix).unwrap_or("");
        let spec = if rest.parse::<usize>().is_ok() {
            ch.breakables.get(n)
        } else {
            ch.breakables.iter().find(|b| matches!(&b.extras, Extras::Pipe(p) if p.props.iter().any(|x| x == rest)))
        };
        let Some(Extras::Pipe(p)) = spec.map(|b| &b.extras) else { continue };
        let absolute: Vec<(i32, i32)> = p.flood_cells.iter().map(|c| (base.0 + c.x, base.1 + c.y)).collect();
        out.push(PipeCells { id, outlet, cells: cells_for_flow(&absolute, &p.flood_steps, origin) });
    }
    out
}

/// One authored pipe flood on a non-sewer storey (a row of `authored_floods` in `data/pipes.json`).
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredFlood {
    pub id: String,
    pub trigger: String,
    pub level: i32,
    pub cells: Vec<(i32, i32)>,
    pub steps: Vec<i32>,
}

/// Reads the `authored_floods` rows of a pipes table. A malformed row is an error naming it.
pub fn parse_authored(text: &str) -> Result<Vec<AuthoredFlood>, String> {
    let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(rows) = root.get("authored_floods") else { return Ok(vec![]) };
    let rows = rows.as_array().ok_or("authored_floods: a list is needed")?;
    let mut out = vec![];
    for (i, r) in rows.iter().enumerate() {
        let who = format!("authored_floods[{i}]");
        let s = |k: &str| r.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string);
        let id = s("id").ok_or(format!("{who}.id: a string is needed"))?;
        let trigger = s("trigger").ok_or(format!("{who}.trigger: a string is needed"))?;
        let level = r.get("level").and_then(|v| v.as_i64()).ok_or(format!("{who}.level: an integer is needed"))? as i32;
        let pair = |v: &serde_json::Value| -> Option<(i32, i32)> {
            let a = v.as_array()?;
            Some((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32))
        };
        let cells: Vec<(i32, i32)> = r
            .get("cells")
            .and_then(|v| v.as_array())
            .ok_or(format!("{who}.cells: a list is needed"))?
            .iter()
            .map(|c| pair(c).ok_or(format!("{who}.cells: each cell is [x, y]")))
            .collect::<Result<_, _>>()?;
        let steps: Vec<i32> = match r.get("steps").and_then(|v| v.as_array()) {
            Some(a) => a.iter().filter_map(|v| v.as_i64()).map(|v| v as i32).collect(),
            None => (0..cells.len() as i32).collect(),
        };
        out.push(AuthoredFlood { id, trigger, level, cells, steps });
    }
    Ok(out)
}

/// The embedded authored floods.
pub fn authored_floods() -> &'static [AuthoredFlood] {
    static T: std::sync::OnceLock<Vec<AuthoredFlood>> = std::sync::OnceLock::new();
    T.get_or_init(|| parse_authored(include_str!("../data/pipes.json")).unwrap_or_else(|e| panic!("data/pipes.json: {e}")))
}

/// The window cells of the authored floods whose flow runs on the runner's storey.
pub(super) fn authored_cells(g: &Game, floods: &[AuthoredFlood]) -> Vec<PipeCells> {
    let origin = g.window_origin();
    let level = g.runner_level();
    floods
        .iter()
        .filter(|f| f.level == level)
        .filter_map(|f| {
            let run = g.pipes.run(&f.id)?;
            Some(PipeCells { id: f.id.clone(), outlet: run.outlet, cells: cells_for_flow(&f.cells, &f.steps, origin) })
        })
        .collect()
}

impl Game {
    /// A valve turn starts every authored flood it triggers (not yet running). Whoever applies a turn calls this.
    pub fn start_authored_floods(&mut self, valve_id: &str, floods: &[AuthoredFlood]) {
        let now = self.time;
        for f in floods.iter().filter(|f| f.trigger == valve_id) {
            self.pipes.start_flow(&f.id, now);
        }
    }

    /// A bot (or chased fake player) turns a valve: the network decides as for the runner, the turn is applied, and
    /// the hunters hear it. No runner noise burst and no danger cue. Returns the event for the net layer, or None
    /// when the bot is too far or on the wrong storey.
    pub fn bot_valve_events(&mut self, q: pipes::TurnAt) -> Option<TurnEvent> {
        let q = pipes::TurnAt { by_player: false, ..q };
        let ev = self.pipes.turn(q).ok()?;
        self.pipes.apply(&ev, self.time);
        self.start_authored_floods(q.valve_id, authored_floods());
        self.push_valve_events(&ev, q.at, q.storey);
        Some(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::world::route::layout::{Coord, V2};
    use crate::world::route::sewer::{self, breach};

    /// A sewer game whose water chunk holds one fabricated pipe breakable (the last one), flooding `cells` local cells.
    fn game_with_pipe(cells: &[(i32, i32)]) -> (Game, String) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-SWIM-7", sewer_core::LEVEL, &content, &catalog);
        let center = g.inf.as_ref().unwrap().center;
        let seed = g.inf.as_ref().unwrap().seed.clone();
        let coord = Coord::new(center.0, sewer_core::LEVEL, center.1);
        let mut ch = sewer::chunk(coord, &sewer::Inputs { world: &seed }).expect("a sewer chunk");
        let n = ch.breakables.len();
        let flood: Vec<V2> = cells.iter().map(|c| V2::new(c.0, c.1)).collect();
        let steps: Vec<i32> = (0..cells.len() as i32).collect();
        ch.breakables.push(breach::make_spec(
            breach::make_id(coord, n),
            breach::KIND_PIPE,
            "authored",
            coord,
            flood[0],
            1,
            vec![],
            Extras::Pipe(breach::PipeExtras {
                pipe: "outflow",
                props: vec!["P".into()],
                cells: flood.clone(),
                centre: [0.0, 0.0],
                dir: [1.0, 0.0],
                flood_cells: flood,
                flood_steps: steps,
                wall: [0.0, 0.0],
            }),
            false,
        ));
        g.water_cache = Some(((center.0, sewer_core::LEVEL, center.1), Some(Box::new(ch))));
        let id = format!("{}{n}", pipes::flow_prefix((center.0, sewer_core::LEVEL, center.1)));
        (g, id)
    }

    fn authored(level: i32) -> Vec<AuthoredFlood> {
        parse_authored(&format!(
            r#"{{"authored_floods":[{{"id":"AUTH:lab:1","trigger":"SEWER:2:-1:3:PROP:7","level":{level},"cells":[[20,20],[21,20],[22,20]]}}]}}"#
        ))
        .unwrap()
    }

    fn bot_turn<'a>(level: i32, near: f32) -> pipes::TurnAt<'a> {
        pipes::TurnAt {
            valve_id: "SEWER:2:-1:3:PROP:7",
            effect: "flood",
            gate: "",
            valve_at: (1.0, 1.0),
            valve_storey: level,
            at: (1.0 + near, 1.0),
            storey: level,
            by_player: true,
        }
    }

    #[test]
    fn the_embedded_authored_floods_parse_and_bad_rows_name_themselves() {
        let rows = authored_floods();
        for f in rows {
            assert!(!f.cells.is_empty() && f.steps.len() >= f.cells.len(), "{} has cells and a step each", f.id);
        }
        assert!(parse_authored(r#"{"authored_floods":[{"id":"a"}]}"#).unwrap_err().contains("authored_floods[0]"));
        assert!(parse_authored(r#"{"schema":"x"}"#).unwrap().is_empty());
    }

    #[test]
    fn an_authored_flood_floods_its_cells_only_once_its_valve_runs_on_that_storey() {
        let (mut g, _) = game_with_pipe(&[(3, 3)]);
        let level = g.runner_level();
        let floods = authored(level);
        assert!(authored_cells(&g, &floods).is_empty(), "nothing flows before the valve");
        g.start_authored_floods("SOME:OTHER:VALVE", &floods);
        assert!(authored_cells(&g, &floods).is_empty(), "another valve does not start it");
        g.start_authored_floods("SEWER:2:-1:3:PROP:7", &floods);
        let c = authored_cells(&g, &floods);
        assert_eq!(c.len(), 1);
        let o = g.window_origin();
        assert_eq!(c[0].cells, vec![((20 - o.0, 20 - o.1), 0), ((21 - o.0, 20 - o.1), 1), ((22 - o.0, 20 - o.1), 2)]);
        assert!(authored_cells(&g, &authored(level + 3)).is_empty(), "a flood on another storey is not this window's");
    }

    #[test]
    fn a_bot_turning_a_valve_makes_the_hunters_hear_it_and_a_far_bot_makes_nothing() {
        let (mut g, _) = game_with_pipe(&[(3, 3)]);
        g.noise_events.clear();
        g.p.noise_burst = 0.0;
        let ev = g.bot_valve_events(bot_turn(-1, 0.5)).expect("a bot beside the valve turns it");
        assert_eq!(ev.noise, None, "a bot never feeds the runner's danger readout");
        assert_eq!(g.p.noise_burst, 0.0);
        assert_eq!(g.pipes.turns("SEWER:2:-1:3:PROP:7"), 1);
        let rules = crate::hunters::noise_events::rules();
        let heard = crate::hunters::noise_events::heard_with(rules, &g.noise_events, (1.5, 1.0), 1.0)
            .expect("a hunter beside the valve hears the bot");
        assert_eq!(heard.kind, NoiseKind::ValveTurn);
        assert_ne!(heard.response, crate::hunters::noise_events::Response::Search, "a valve turn is investigated, never a chase");
        g.noise_events.clear();
        let reach = pipes::rules().valve.reach;
        assert!(g.bot_valve_events(bot_turn(-1, reach + 5.0)).is_none(), "out of reach, nothing turns");
        assert!(g.noise_events.is_empty(), "and nothing is heard");
        assert!(g.bot_valve_events(bot_turn(-3, 0.5)).is_none(), "the wrong storey, nothing turns");
    }

    #[test]
    fn a_plunge_is_heard_inside_its_radius_and_not_beyond() {
        let rules = crate::hunters::noise_events::rules();
        let ev = [crate::hunters::noise_events::NoiseEvent::new(NoiseKind::PlungeSplash, (0.0, 0.0), -1)];
        let reach = rules.radius_of(&ev[0], 1.0);
        assert!(reach > 1.0);
        let h = crate::hunters::noise_events::heard_with(rules, &ev, (reach * 0.5, 0.0), 1.0).expect("heard near");
        assert_eq!(h.kind, NoiseKind::PlungeSplash);
        assert!(crate::hunters::noise_events::heard_with(rules, &ev, (reach + 1.0, 0.0), 1.0).is_none());
    }

    #[test]
    fn a_closed_gate_blocks_the_hunters_and_an_open_event_lets_them_through() {
        use crate::hunters::brain::gate_floor_cells;
        use crate::world::Cell;
        let cells = [Cell::new(4, 4), Cell::new(5, 4)];
        let (mut g, _) = game_with_pipe(&[(3, 3)]);
        let id = "SEWER:2:-1:3:PROP:9";
        let gate = pipes::TurnAt { valve_id: id, effect: "gate", gate: "g", ..bot_turn(-1, 0.5) };
        assert!(!g.pipes.is_open(id));
        assert!(gate_floor_cells(&cells, g.pipes.is_open(id)).is_empty(), "a raised gate is no crossing");
        g.bot_valve_events(gate).expect("the bot lowers the gate");
        assert!(g.pipes.is_open(id));
        assert_eq!(gate_floor_cells(&cells, g.pipes.is_open(id)).len(), 2, "the open event makes the cells passable");
        assert!(g.noise_events.iter().any(|e| e.kind == NoiseKind::SluiceOpen), "and the sluice is heard");
    }

    #[test]
    fn cells_for_flow_maps_window_cells_and_steps() {
        let out = cells_for_flow(&[(10, 12), (11, 12), (12, 12)], &[0, 1, 2], (4, 2));
        assert_eq!(out, vec![((6, 10), 0), ((7, 10), 1), ((8, 10), 2)]);
        assert_eq!(cells_for_flow(&[(1, 1), (2, 2)], &[5], (0, 0)), vec![((1, 1), 5)]);
        assert!(cells_for_flow(&[], &[1], (0, 0)).is_empty());
    }

    #[test]
    fn a_valve_turn_floods_a_route_and_the_field_changes() {
        let (mut g, id) = game_with_pipe(&[(3, 3), (4, 3), (5, 3)]);
        g.sync_hunter_water();
        let before = g.hunter_water.as_ref().expect("a field").field.version();
        assert!(cells(&g).is_empty(), "no flow yet");
        let key = g.water_cache.as_ref().unwrap().0;
        g.burst_pipe(key, id.rsplit(':').next().unwrap().parse().unwrap());
        let c = cells(&g);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].cells.len(), 3);
        let rules = crate::hunters::noise_events::rules();
        let mut t = 0.0;
        while t < 60.0 && g.hunter_water.as_ref().map_or(true, |w| !w.field.flooded(c[0].cells[0].0)) {
            g.time += rules.water_rebuild_seconds;
            t += rules.water_rebuild_seconds;
            g.sync_hunter_water();
        }
        let field = g.hunter_water.as_ref().unwrap().field.clone();
        assert!(field.flooded(c[0].cells[0].0), "the pipe cell floods within the data time");
        assert_ne!(field.version(), before, "the field version bumped, so hunters replan");
    }

    #[test]
    fn a_closed_valve_drains_the_field() {
        let (mut g, id) = game_with_pipe(&[(3, 3), (4, 3)]);
        let key = g.water_cache.as_ref().unwrap().0;
        g.burst_pipe(key, id.rsplit(':').next().unwrap().parse().unwrap());
        let rules = crate::hunters::noise_events::rules();
        g.time += 30.0;
        g.sync_hunter_water();
        let cell = cells(&g)[0].cells[0].0;
        assert!(g.hunter_water.as_ref().unwrap().field.flooded(cell));
        // the flood valve shuts the flow; after the drain time the cell is dry again
        let ev = TurnEvent {
            valve_id: "SEWER:0:-1:0:PROP:1".into(),
            chunk: key,
            effect: "flood".into(),
            shut: true,
            turns: 0,
            gate: String::new(),
            noise: None,
        };
        g.pipes.apply(&ev, g.time);
        g.time += pipes::rules().flow.shut_drain_seconds + 2.0 * rules.water_rebuild_seconds;
        g.sync_hunter_water();
        assert!(!g.hunter_water.as_ref().unwrap().field.flooded(cell), "a drained flow no longer floods the cell");
    }

    #[test]
    fn a_bot_valve_turn_pushes_an_event_but_no_meter_noise() {
        let (mut g, _) = game_with_pipe(&[(3, 3)]);
        g.noise_events.clear();
        g.p.noise_burst = 0.0;
        let n = pipes::PipeNetwork::new();
        let ev = n
            .turn(pipes::TurnAt {
                valve_id: "SEWER:2:-1:3:PROP:7",
                effect: "flood",
                gate: "g",
                valve_at: (1.0, 1.0),
                valve_storey: -1,
                at: (1.0, 1.0),
                storey: -1,
                by_player: false,
            })
            .unwrap();
        assert_eq!(ev.noise, None, "no runner meter noise for a bot");
        g.push_valve_events(&ev, (1.0, 1.0), -1);
        assert!(g.noise_events.iter().any(|e| e.kind == NoiseKind::ValveTurn && e.level == -1));
        assert!(g.noise_events.iter().any(|e| e.kind == NoiseKind::SluiceOpen), "an odd gate turn also sounds the sluice");
        assert_eq!(g.p.noise_burst, 0.0);
    }

    #[test]
    fn a_guest_pushes_no_positioned_events() {
        let (mut g, _) = game_with_pipe(&[(3, 3)]);
        g.noise_events.clear();
        g.remote_hunters = true;
        g.push_noise_at(NoiseKind::PipeBurst, (1.0, 1.0), -1);
        g.push_noise_scaled(NoiseKind::HardLanding, (1.0, 1.0), -1, 2.0);
        assert!(g.noise_events.is_empty());
    }
}
