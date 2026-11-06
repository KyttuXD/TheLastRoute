//! The story pack's data model: conditions, operations, map tiles, scenes and items. Pure data and rules (no
//! drawing and no file access), so the validator and the game logic can be tested without a window.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A test on the player's flags and inventory. Written in JSON as `{"flag": "x"}`, `{"not_flag": "x"}`,
/// `{"has": "item"}`, `{"not": cond}`, `{"all": [cond, ...]}` or `{"any": [cond, ...]}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Flag(String),
    NotFlag(String),
    Has(String),
    Not(Box<Cond>),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

impl Cond {
    pub fn eval(&self, flags: &BTreeSet<String>, inv: &[String]) -> bool {
        match self {
            Cond::Flag(f) => flags.contains(f),
            Cond::NotFlag(f) => !flags.contains(f),
            Cond::Has(item) => inv.iter().any(|i| i == item),
            Cond::Not(c) => !c.eval(flags, inv),
            Cond::All(cs) => cs.iter().all(|c| c.eval(flags, inv)),
            Cond::Any(cs) => cs.iter().any(|c| c.eval(flags, inv)),
        }
    }

    /// Every item id this condition names, so validation can check they exist.
    pub fn item_refs(&self, out: &mut Vec<String>) {
        match self {
            Cond::Has(item) => out.push(item.clone()),
            Cond::Not(c) => c.item_refs(out),
            Cond::All(cs) | Cond::Any(cs) => cs.iter().for_each(|c| c.item_refs(out)),
            Cond::Flag(_) | Cond::NotFlag(_) => {}
        }
    }
}

/// Something a scene or choice does when it runs. Written as `{"op": "set_flag", "flag": "x", "value": true}`,
/// `{"op": "give", "item": "id"}`, `{"op": "take", "item": "id"}`, `{"op": "exit", "map": "id", "spawn": "name"}`
/// or `{"op": "toast", "text": "..."}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    SetFlag { flag: String, value: bool },
    Give(String),
    Take(String),
    Exit { map: String, spawn: String },
    Toast(String),
}

/// An RGB colour from a hex code in the pack.
pub type Rgb = [u8; 3];

/// What a legend colour means on a map. The mask picture picks the kind of every tile. In the 3D view the map's art
/// picture is the floor, walls are boxes of `height` tiles, and the other kinds are small solids or markers.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Floor,
    /// A box `height` tiles tall (1.0 is one tile).
    Wall { height: f32, color: Rgb },
    /// A named place the player can be put (the pack start, or the arrival point of an exit).
    Spawn { name: String },
    /// Blocks until `open_if` is true; `locked` is shown when the player bumps into it. Drawn as a box while shut.
    Door { open_if: Cond, locked: String, height: f32, color: Rgb },
    /// Picked up once when the player steps on it; drawn as a spinning orb.
    Item { item: String, color: Rgb },
    /// Blocks; press E facing it to play `scene`. Drawn as a figure with its name above it.
    Npc { name: String, scene: String, color: Rgb },
    /// Plays `scene` when the player steps on it (once per tile when `once`). Invisible.
    Trigger { scene: String, once: bool },
    /// Steps the player onto the spawn `spawn` of map `map`. Drawn as a glowing pad.
    Exit { map: String, spawn: String },
}

impl Kind {
    /// Walls and NPCs block; a door blocks until its condition holds; everything else can be walked onto.
    pub fn blocks(&self, flags: &BTreeSet<String>, inv: &[String]) -> bool {
        match self {
            Kind::Wall { .. } | Kind::Npc { .. } => true,
            Kind::Door { open_if, .. } => !open_if.eval(flags, inv),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub description: String,
}

/// One spoken line. An empty speaker is narration.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub speaker: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub cond: Option<Cond>,
    pub ops: Vec<Op>,
    pub next: Option<String>,
}

/// A piece of dialogue: lines, then either choices (when any are offered) or the scene's own ops and `next`.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub once: bool,
    pub lines: Vec<Line>,
    pub choices: Vec<Choice>,
    pub ops: Vec<Op>,
    pub next: Option<String>,
}

/// One tile per mask pixel, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Kind>,
}

impl Grid {
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    pub fn at(&self, x: i32, y: i32) -> Option<&Kind> {
        if self.in_bounds(x, y) {
            self.cells.get(y as usize * self.w + x as usize)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct MapDef {
    pub id: String,
    pub name: String,
    /// The picture that is drawn. It may be any size that is a whole multiple of the mask.
    pub art: PathBuf,
    pub legend: BTreeMap<String, Kind>,
    pub grid: Grid,
    /// Spawn name to tile.
    pub spawns: BTreeMap<String, (i32, i32)>,
}

/// One pack that went into the merged game, in load order (the core pack first).
#[derive(Clone, Debug, PartialEq)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub core: bool,
    pub dir: PathBuf,
    pub scenes: usize,
    pub items: usize,
    pub maps: usize,
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub title: String,
    pub start_map: String,
    pub start_spawn: String,
    pub start_scene: Option<String>,
    pub items: BTreeMap<String, Item>,
    pub scenes: BTreeMap<String, Scene>,
    pub maps: BTreeMap<String, MapDef>,
    pub packs: Vec<PackInfo>,
}
