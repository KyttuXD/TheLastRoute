//! The running game: where the player is, what they carry and know, and the dialogue. No drawing and no input here;
//! `main.rs` feeds it key presses and reads its state. Everything here is testable without a window.

use crate::body::{Body, Input, TILE_M};
use crate::model::*;
use std::collections::BTreeSet;

/// How far ahead of the body (metres) an NPC or door counts as "in front" for E.
const REACH_M: f32 = 1.0;

/// Whether map tile (tx, ty) of `map` stops a body. Off-map tiles stop it too.
fn tile_blocks(pack: &Pack, map: &str, flags: &BTreeSet<String>, inv: &[String], tx: i32, ty: i32) -> bool {
    match pack.maps.get(map).and_then(|m| m.grid.at(tx, ty)) {
        None => true,
        Some(kind) => kind.blocks(flags, inv),
    }
}

/// The centre of tile (x, y) in metres.
fn tile_centre(x: i32, y: i32) -> (f32, f32) {
    ((x as f32 + 0.5) * TILE_M, (y as f32 + 0.5) * TILE_M)
}

/// Typewriter speed for dialogue lines.
pub const CHARS_PER_SEC: f32 = 42.0;
const TOAST_SECS: f32 = 2.5;
/// How many scenes may start one after another without the player doing anything (a `next` loop guard).
const MAX_CHAIN: usize = 32;

/// A scene being played. `choosing` is set once the lines are done and choices are offered; `choices` holds
/// the indices (into the scene's `choices`) that are available, in order, and `sel` is the highlighted one.
#[derive(Clone, Debug, PartialEq)]
pub struct Talk {
    pub scene: String,
    pub line: usize,
    /// Characters of the current line revealed so far.
    pub shown: f32,
    pub choosing: bool,
    pub sel: usize,
    pub choices: Vec<usize>,
}

pub struct Game {
    pub pack: Pack,
    pub map: String,
    /// The tile the body is in. The body (`body`) is the real position; this follows it tile by tile.
    pub pos: (i32, i32),
    /// The first-person body that moves through the 3D map.
    pub body: Body,
    pub flags: BTreeSet<String>,
    pub inv: Vec<String>,
    /// Item tiles already picked up, as `map@x,y`.
    pub taken: BTreeSet<String>,
    /// Once-only scenes that have run (by scene id) and once-only trigger tiles (`tile:map@x,y`).
    pub fired: BTreeSet<String>,
    pub talk: Option<Talk>,
    /// A short message and the seconds it has left.
    pub toast: Option<(String, f32)>,
    pub move_cool: f32,
}

impl Game {
    /// A game with no flags set. Tests use it; the game starts with `with_flags` so it can load the saved flags.
    #[cfg(test)]
    pub fn new(pack: Pack) -> Result<Game, String> {
        Game::with_flags(pack, BTreeSet::new())
    }

    /// A game that starts with `flags` already set (the saved ones), so the start scene sees them.
    pub fn with_flags(pack: Pack, flags: BTreeSet<String>) -> Result<Game, String> {
        let map = pack.start_map.clone();
        let spawn = pack.start_spawn.clone();
        let pos = pack
            .maps
            .get(&map)
            .and_then(|m| m.spawns.get(&spawn).copied())
            .ok_or_else(|| format!("start spawn '{spawn}' is not on map '{map}'"))?;
        let start_scene = pack.start_scene.clone();
        let (bx, bz) = tile_centre(pos.0, pos.1);
        let mut game = Game {
            pack,
            map,
            pos,
            // Start facing south (+z), into the room; yaw 0 would face the map's north wall.
            body: Body::new((bx, bz), std::f32::consts::PI),
            flags,
            inv: Vec::new(),
            taken: BTreeSet::new(),
            fired: BTreeSet::new(),
            talk: None,
            toast: None,
            move_cool: 0.0,
        };
        if let Some(scene) = start_scene {
            game.start_scene(&scene);
        }
        Ok(game)
    }

    pub fn current_map(&self) -> &MapDef {
        &self.pack.maps[&self.map]
    }

    pub fn kind_at(&self, x: i32, y: i32) -> Option<&Kind> {
        self.current_map().grid.at(x, y)
    }

    /// A tile is blocked when it is off the map, or its kind blocks (walls, NPCs, locked doors).
    #[cfg(test)]
    fn blocked(&self, x: i32, y: i32) -> bool {
        match self.kind_at(x, y) {
            None => true,
            Some(kind) => kind.blocks(&self.flags, &self.inv),
        }
    }

    fn tile_key(&self, x: i32, y: i32) -> String {
        format!("{}@{},{}", self.map, x, y)
    }

    fn item_name(&self, id: &str) -> String {
        self.pack.items.get(id).map_or_else(|| id.to_string(), |i| i.name.clone())
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), TOAST_SECS));
    }

    /// Moves the body for one frame of input, then runs what the tile it entered does. While a scene is playing the
    /// body only looks around.
    pub fn update_body(&mut self, dt: f32, input: &Input) {
        let input = if self.talk.is_some() {
            Input {
                look: input.look,
                ..Default::default()
            }
        } else {
            *input
        };
        let pack = &self.pack;
        let (map, flags, inv) = (&self.map, &self.flags, &self.inv);
        self.body
            .update(dt, &input, &|tx, ty| tile_blocks(pack, map, flags, inv, tx, ty));
        let now = self.body.tile();
        if now != self.pos {
            self.arrive(now.0, now.1);
        }
    }

    /// The tile the player is facing, a metre ahead of the body. E talks to what is there.
    pub fn facing_tile(&self) -> (i32, i32) {
        let (fx, fz) = self.body.forward2();
        let p = (self.body.pos.0 + fx * REACH_M, self.body.pos.1 + fz * REACH_M);
        ((p.0 / TILE_M).floor() as i32, (p.1 / TILE_M).floor() as i32)
    }

    /// Puts the body at the centre of tile (x, y), keeping its facing. The tile is not entered (no effects run).
    fn place_body(&mut self, x: i32, y: i32) {
        self.body.pos = tile_centre(x, y);
    }

    /// Tests and the tile-based helpers: puts the player on a tile, facing a direction (dx, dy).
    #[cfg(test)]
    pub fn place_at(&mut self, tile: (i32, i32), dir: (i32, i32)) {
        self.pos = tile;
        self.place_body(tile.0, tile.1);
        self.body.yaw = (-dir.0 as f32).atan2(-dir.1 as f32);
    }

    /// The body has walked onto tile (x, y): remember it and run what the tile does.
    fn arrive(&mut self, x: i32, y: i32) {
        self.pos = (x, y);
        self.on_step(x, y);
    }

    /// One step in direction (dx, dy), used by the tile-based tests. Stepping onto a tile runs what that tile does.
    #[cfg(test)]
    pub fn try_move(&mut self, dx: i32, dy: i32) {
        if self.talk.is_some() {
            return;
        }
        let (tx, ty) = (self.pos.0 + dx, self.pos.1 + dy);
        if self.blocked(tx, ty) {
            if let Some(Kind::Door { locked, .. }) = self.kind_at(tx, ty).cloned() {
                self.toast(locked);
            }
            return;
        }
        self.place_body(tx, ty);
        self.arrive(tx, ty);
    }

    fn on_step(&mut self, x: i32, y: i32) {
        match self.kind_at(x, y).cloned() {
            Some(Kind::Item { item, .. }) => {
                if self.taken.insert(self.tile_key(x, y)) {
                    let name = self.item_name(&item);
                    self.inv.push(item);
                    self.toast(format!("Picked up {name}"));
                }
            }
            Some(Kind::Trigger { scene, once }) => {
                let go = !once || self.fired.insert(format!("tile:{}", self.tile_key(x, y)));
                if go {
                    self.start_scene(&scene);
                }
            }
            Some(Kind::Exit { map, spawn }) => self.leave(&map, &spawn),
            _ => {}
        }
    }

    /// Puts the player on a spawn point of a map. A missing spawn only shows a message (validation prevents it).
    pub fn leave(&mut self, map: &str, spawn: &str) {
        match self.pack.maps.get(map).and_then(|m| m.spawns.get(spawn).copied()) {
            Some(at) => {
                self.map = map.to_string();
                self.pos = at;
                self.place_body(at.0, at.1);
            }
            None => self.toast(format!("No spawn '{spawn}' on '{map}'")),
        }
    }

    /// E / Space / Enter: talk to the NPC in front, or move the dialogue on.
    pub fn interact(&mut self) {
        if self.talk.is_some() {
            self.advance();
            return;
        }
        let (tx, ty) = self.facing_tile();
        if let Some(Kind::Npc { scene, .. }) = self.kind_at(tx, ty).cloned() {
            self.start_scene(&scene);
        }
    }

    /// Reveals the rest of the current line, or moves to the next line (and to the choices after the last).
    pub fn advance(&mut self) {
        let Some(t) = self.talk.clone() else { return };
        if t.choosing {
            return;
        }
        let total = self.line_chars(&t.scene, t.line);
        if t.shown < total as f32 {
            if let Some(talk) = self.talk.as_mut() {
                talk.shown = total as f32;
            }
            return;
        }
        let lines = self.pack.scenes.get(&t.scene).map_or(0, |s| s.lines.len());
        if t.line + 1 < lines {
            if let Some(talk) = self.talk.as_mut() {
                talk.line += 1;
                talk.shown = 0.0;
            }
        } else {
            self.finish_lines(0);
        }
    }

    /// Moves the highlight through the offered choices (wraps around).
    pub fn move_sel(&mut self, delta: i32) {
        if let Some(t) = self.talk.as_mut() {
            if t.choosing && !t.choices.is_empty() {
                let n = t.choices.len() as i32;
                t.sel = (t.sel as i32 + delta).rem_euclid(n) as usize;
            }
        }
    }

    /// Picks the offered choice at position `n` (0-based), runs its ops and goes to its `next` scene, if any.
    pub fn choose(&mut self, n: usize) {
        let Some(t) = self.talk.clone() else { return };
        if !t.choosing {
            return;
        }
        let Some(&index) = t.choices.get(n) else { return };
        let Some(scene) = self.pack.scenes.get(&t.scene) else { return };
        let choice = scene.choices[index].clone();
        self.run_ops(&choice.ops);
        self.end_scene(choice.next, 0);
    }

    pub fn start_scene(&mut self, id: &str) {
        self.start_at(id, 0);
    }

    fn start_at(&mut self, id: &str, depth: usize) {
        self.talk = None;
        if depth > MAX_CHAIN {
            self.toast(format!("Scene loop stopped at '{id}'"));
            return;
        }
        let Some(scene) = self.pack.scenes.get(id) else {
            self.toast(format!("Missing scene '{id}'"));
            return;
        };
        if scene.once && !self.fired.insert(id.to_string()) {
            return;
        }
        let no_lines = scene.lines.is_empty();
        self.talk = Some(Talk {
            scene: id.to_string(),
            line: 0,
            shown: 0.0,
            choosing: false,
            sel: 0,
            choices: Vec::new(),
        });
        if no_lines {
            self.finish_lines(depth);
        }
    }

    /// The lines are done: offer the available choices, or run the scene's own ops and go on to `next`.
    fn finish_lines(&mut self, depth: usize) {
        let Some(t) = self.talk.clone() else { return };
        let Some(scene) = self.pack.scenes.get(&t.scene).cloned() else {
            self.talk = None;
            return;
        };
        let available: Vec<usize> = scene
            .choices
            .iter()
            .enumerate()
            .filter(|(_, c)| c.cond.as_ref().map_or(true, |cond| cond.eval(&self.flags, &self.inv)))
            .map(|(i, _)| i)
            .collect();
        if !available.is_empty() {
            self.talk = Some(Talk {
                choosing: true,
                sel: 0,
                choices: available,
                ..t
            });
            return;
        }
        self.run_ops(&scene.ops);
        self.end_scene(scene.next, depth);
    }

    fn end_scene(&mut self, next: Option<String>, depth: usize) {
        match next {
            Some(id) => self.start_at(&id, depth + 1),
            None => self.talk = None,
        }
    }

    fn line_chars(&self, scene: &str, line: usize) -> usize {
        self.pack
            .scenes
            .get(scene)
            .and_then(|s| s.lines.get(line))
            .map_or(0, |l| l.text.chars().count())
    }

    /// The line on screen right now, if a scene is playing and is showing lines.
    pub fn current_line(&self) -> Option<&Line> {
        let t = self.talk.as_ref()?;
        self.pack.scenes.get(&t.scene)?.lines.get(t.line)
    }

    /// The choices on offer, as their text.
    pub fn offered_choices(&self) -> Vec<String> {
        let Some(t) = self.talk.as_ref().filter(|t| t.choosing) else { return Vec::new() };
        let Some(scene) = self.pack.scenes.get(&t.scene) else { return Vec::new() };
        t.choices.iter().map(|&i| scene.choices[i].text.clone()).collect()
    }

    fn run_ops(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::SetFlag { flag, value } => {
                    if *value {
                        self.flags.insert(flag.clone());
                    } else {
                        self.flags.remove(flag);
                    }
                }
                Op::Give(item) => {
                    if !self.inv.contains(item) {
                        self.inv.push(item.clone());
                    }
                }
                Op::Take(item) => self.inv.retain(|i| i != item),
                Op::Exit { map, spawn } => self.leave(map, spawn),
                Op::Toast(text) => self.toast(text.clone()),
            }
        }
    }

    /// Advances the clock: the typewriter and the toast timer.
    pub fn tick(&mut self, dt: f32) {
        self.move_cool -= dt;
        if let Some(t) = self.talk.as_mut() {
            t.shown += dt * CHARS_PER_SEC;
        }
        if let Some(t) = self.toast.as_mut() {
            t.1 -= dt;
        }
        if self.toast.as_ref().map_or(false, |t| t.1 <= 0.0) {
            self.toast = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::validate;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn floor() -> Kind {
        Kind::Floor
    }

    fn wall() -> Kind {
        Kind::Wall {
            height: 1.0,
            color: [90, 99, 120],
        }
    }

    #[test]
    fn the_body_walks_onto_an_item_and_picks_it_up() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((2, 1), (1, 0)); // east, the keycard is at (3,1)
        g.update_body(0.5, &Input { fwd: 1.0, ..Default::default() }); // 2 m east: into tile (3,1)
        assert_eq!(g.pos, (3, 1));
        assert_eq!(g.inv, vec!["keycard".to_string()]);
    }

    #[test]
    fn the_body_is_stopped_by_walls_and_the_facing_tile_is_the_one_in_front() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((1, 1), (0, -1)); // north: (1,0) is a wall
        g.update_body(1.0, &Input { fwd: 1.0, ..Default::default() });
        assert_eq!(g.pos, (1, 1));
        assert_eq!(g.facing_tile(), (1, 0));
    }

    /// A map from rows of characters; `legend` maps each character to its kind.
    fn map(id: &str, rows: &[&str], legend: &[(char, Kind)]) -> MapDef {
        let h = rows.len();
        let w = rows[0].len();
        let mut cells = Vec::new();
        let mut spawns = BTreeMap::new();
        for (y, row) in rows.iter().enumerate() {
            assert_eq!(row.len(), w, "test map rows must be the same width");
            for (x, ch) in row.chars().enumerate() {
                let kind = legend
                    .iter()
                    .find(|(c, _)| *c == ch)
                    .map(|(_, k)| k.clone())
                    .unwrap_or_else(|| panic!("no legend for '{ch}'"));
                if let Kind::Spawn { name } = &kind {
                    spawns.insert(name.clone(), (x as i32, y as i32));
                }
                cells.push(kind);
            }
        }
        MapDef {
            id: id.to_string(),
            name: id.to_string(),
            art: PathBuf::new(),
            legend: legend.iter().map(|(c, k)| (c.to_string(), k.clone())).collect(),
            grid: Grid { w, h, cells },
            spawns,
        }
    }

    fn line(speaker: &str, text: &str) -> Line {
        Line {
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    fn scene(lines: Vec<Line>) -> Scene {
        Scene {
            once: false,
            lines,
            choices: Vec::new(),
            ops: Vec::new(),
            next: None,
        }
    }

    /// Two maps: a lab with a keycard, a locked door and an exit; and a corridor with a trigger and an exit back.
    fn test_pack() -> Pack {
        let lab_legend = [
            ('#', wall()),
            ('.', floor()),
            ('S', Kind::Spawn { name: "start".into() }),
            ('P', Kind::Spawn { name: "inside".into() }),
            ('K', Kind::Item { item: "keycard".into(), color: [255, 216, 74] }),
            (
                'D',
                Kind::Door {
                    open_if: Cond::Has("keycard".into()),
                    locked: "Needs a keycard.".into(),
                    height: 1.0,
                    color: [176, 112, 48],
                },
            ),
            ('E', Kind::Exit { map: "corridor".into(), spawn: "from-lab".into() }),
            ('F', Kind::Npc { name: "Felix".into(), scene: "felix".into(), color: [111, 191, 138] }),
        ];
        let lab = map(
            "lab",
            &["#######", "#S.K.F#", "#.....#", "#P#D#E#", "#######"],
            &lab_legend,
        );
        let corridor_legend = [
            ('#', wall()),
            ('.', floor()),
            ('Q', Kind::Spawn { name: "from-lab".into() }),
            ('T', Kind::Trigger { scene: "note".into(), once: true }),
            ('B', Kind::Exit { map: "lab".into(), spawn: "inside".into() }),
        ];
        let corridor = map("corridor", &["#####", "BQ.T#", "#####"], &corridor_legend);

        let mut items = BTreeMap::new();
        items.insert("keycard".to_string(), Item { name: "Keycard".into(), description: String::new() });
        let mut scenes = BTreeMap::new();
        scenes.insert(
            "felix".to_string(),
            Scene {
                once: false,
                lines: vec![line("Felix", "Hi.")],
                choices: vec![
                    Choice {
                        text: "Take the card".into(),
                        cond: Some(Cond::Not(Box::new(Cond::Has("keycard".into())))),
                        ops: vec![Op::Give("keycard".into()), Op::SetFlag { flag: "asked".into(), value: true }],
                        next: Some("thanks".into()),
                    },
                    Choice {
                        text: "Leave".into(),
                        cond: None,
                        ops: vec![],
                        next: None,
                    },
                ],
                ops: vec![],
                next: None,
            },
        );
        scenes.insert("thanks".to_string(), scene(vec![line("Felix", "Don't lose it.")]));
        scenes.insert("note".to_string(), scene(vec![line("", "A note.")]));

        let mut maps = BTreeMap::new();
        maps.insert("lab".to_string(), lab);
        maps.insert("corridor".to_string(), corridor);
        Pack {
            title: "test".into(),
            start_map: "lab".into(),
            start_spawn: "start".into(),
            start_scene: None,
            items,
            scenes,
            maps,
            packs: Vec::new(),
        }
    }

    /// Moves the player along a path of single steps, ticking the clock in between so repeats never block.
    fn walk(g: &mut Game, steps: &str) {
        for ch in steps.chars() {
            let d = match ch {
                'U' => (0, -1),
                'D' => (0, 1),
                'L' => (-1, 0),
                'R' => (1, 0),
                _ => continue,
            };
            g.try_move(d.0, d.1);
        }
    }

    #[test]
    fn a_clean_pack_validates() {
        assert!(validate(&test_pack()).is_empty());
    }

    #[test]
    fn walls_block_and_the_keycard_is_picked_up_once() {
        let mut g = Game::new(test_pack()).unwrap();
        g.try_move(-1, 0); // wall at x=0
        assert_eq!(g.pos, (1, 1));
        walk(&mut g, "RR"); // onto K at (3,1): picked up
        assert_eq!(g.inv, vec!["keycard".to_string()]);
        walk(&mut g, "LL");
        walk(&mut g, "RR");
        assert_eq!(g.inv.len(), 1, "a tile gives its item once");
    }

    #[test]
    fn a_locked_door_opens_only_with_the_card() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((3, 2), (0, 1)); // above the door at (3,3)
        g.try_move(0, 1);
        assert_eq!(g.pos, (3, 2), "the door is locked");
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Needs a keycard."));
        g.inv.push("keycard".into());
        g.try_move(0, 1);
        assert_eq!(g.pos, (3, 3), "the card opens it");
    }

    #[test]
    fn talking_offers_only_the_choices_whose_condition_holds() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((4, 1), (1, 0)); // Felix at (5,1)
        g.interact();
        assert!(g.talk.is_some());
        g.advance(); // reveal the line
        g.advance(); // past the last line: offer choices
        assert_eq!(g.offered_choices(), vec!["Take the card", "Leave"]);
        g.choose(0);
        assert!(g.inv.contains(&"keycard".to_string()));
        assert!(g.flags.contains("asked"));
        assert_eq!(g.current_line().map(|l| l.text.as_str()), Some("Don't lose it."));
    }

    #[test]
    fn once_scenes_and_trigger_tiles_do_not_repeat() {
        let mut g = Game::new(test_pack()).unwrap();
        g.map = "corridor".into();
        g.place_at((1, 1), (1, 0));
        g.try_move(1, 0); // onto (2,1)
        g.try_move(1, 0); // onto the trigger at (3,1)
        assert!(g.talk.is_some());
        g.advance();
        g.advance(); // scene ends
        assert!(g.talk.is_none());
        g.try_move(-1, 0);
        g.try_move(1, 0); // back onto the trigger
        assert!(g.talk.is_none(), "a once trigger fires once");
    }

    #[test]
    fn exits_move_the_player_to_the_other_map() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((5, 2), (0, 1));
        g.try_move(0, 1); // onto the exit at (5,3)
        assert_eq!(g.map, "corridor");
        assert_eq!(g.pos, (1, 1));
    }

    #[test]
    fn a_scene_that_chains_to_itself_stops() {
        let mut pack = test_pack();
        pack.scenes.insert(
            "loop".into(),
            Scene {
                once: false,
                lines: vec![],
                choices: vec![],
                ops: vec![],
                next: Some("loop".into()),
            },
        );
        let mut g = Game::new(pack).unwrap();
        g.start_scene("loop");
        assert!(g.talk.is_none());
        assert!(g.toast.is_some());
    }

    #[test]
    fn validation_reports_bad_references() {
        let mut pack = test_pack();
        pack.scenes.get_mut("felix").unwrap().next = Some("nowhere".into());
        pack.scenes.get_mut("thanks").unwrap().ops.push(Op::Give("ghost".into()));
        pack.maps.get_mut("corridor").unwrap().legend.clear();
        let errs = validate(&pack);
        assert!(errs.iter().any(|e| e.contains("unknown scene 'nowhere'")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("unknown item 'ghost'")), "{errs:?}");
    }

    #[test]
    fn validation_reports_a_missing_exit_spawn() {
        let mut pack = test_pack();
        for kind in pack.maps.get_mut("lab").unwrap().legend.values_mut() {
            if let Kind::Exit { spawn, .. } = kind {
                *spawn = "nowhere".into();
            }
        }
        let errs = validate(&pack);
        assert!(errs.iter().any(|e| e.contains("has no spawn 'nowhere'")), "{errs:?}");
    }

    /// Two chapters. Chapter one has one choice: it shows only when the flag `ready` is set, it sets `agreed`, and it
    /// goes to chapter two, which is a scene of its own with one line.
    fn two_chapter_pack() -> Pack {
        let mut pack = test_pack();
        pack.scenes.insert(
            "ch1".into(),
            Scene {
                once: false,
                lines: vec![line("Felix", "Ready?")],
                choices: vec![Choice {
                    text: "Yes".into(),
                    cond: Some(Cond::Flag("ready".into())),
                    ops: vec![Op::SetFlag { flag: "agreed".into(), value: true }],
                    next: Some("ch2".into()),
                }],
                ops: vec![],
                next: None,
            },
        );
        pack.scenes.insert("ch2".into(), scene(vec![line("Felix", "Let's go.")]));
        pack
    }

    #[test]
    fn a_choice_whose_condition_is_not_met_is_hidden() {
        let pack = two_chapter_pack();
        assert!(validate(&pack).is_empty());
        let mut g = Game::new(pack).unwrap();
        g.start_scene("ch1");
        g.advance(); // reveal the line
        g.advance(); // past the last line: no choice is available, so the scene ends
        assert!(g.offered_choices().is_empty());
        assert!(g.talk.is_none());
        assert!(!g.flags.contains("agreed"));
    }

    #[test]
    fn picking_the_choice_sets_its_flag_and_chains_to_chapter_two() {
        let mut g = Game::new(two_chapter_pack()).unwrap();
        g.flags.insert("ready".into());
        g.start_scene("ch1");
        g.advance(); // reveal the line
        g.advance(); // offer the choices
        assert_eq!(g.offered_choices(), vec!["Yes"]);
        g.choose(0);
        assert!(g.flags.contains("agreed"));
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("ch2"));
        assert_eq!(g.current_line().map(|l| l.text.as_str()), Some("Let's go."));
    }
}
