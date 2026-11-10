//! The running game: where the player is, what they carry and know, and the dialogue. No drawing and no input here;
//! `main.rs` feeds it key presses and reads its state. Everything here is testable without a window.

use crate::body::{Body, Input, RADIUS, TILE_M};
use crate::inventory::Inventory;
use crate::model::*;
use crate::npc_ai::{partner_of, push_apart, tick_agent, Env, NpcAgent, NPC_RADIUS};
use crate::rpg::{yaw_of, ActionBattle, Ctx, EnemyState, Outcome, Party, Rng, END_SECS, ITEM_RANGE};
use crate::save::PartySave;
use crate::weather::{Weather, DEFAULT_EVERY_SECS};
use std::collections::{BTreeMap, BTreeSet};

/// How far ahead E looks. A little over half a tile, so from a tile centre it lands inside the neighbour in every
/// direction (exactly half a tile would sit on the boundary, which floors into the wrong tile facing north or west).
const REACH_M: f32 = 1.1;

/// Whether map tile (tx, ty) of `map` stops a body. Off-map tiles stop it too.
fn tile_blocks(pack: &Pack, map: &str, flags: &BTreeSet<String>, inv: &[String], tx: i32, ty: i32) -> bool {
    match pack.maps.get(map).and_then(|m| m.grid.at(tx, ty)) {
        None => true,
        // An NPC with a routine walks about, so its home tile does not stop anyone.
        Some(Kind::Npc { routine: Some(_), .. }) => false,
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
/// How far (metres) an NPC may be from the body, and still in front of it, for E to talk to it.
const TALK_RANGE_M: f32 = 2.0;
/// How long the short use of a prop (not a seat) nudges the camera.
pub const USE_SECS: f32 = 0.8;

/// An agent for every NPC tile of every map.
fn build_agents(pack: &Pack) -> BTreeMap<String, NpcAgent> {
    let mut out = BTreeMap::new();
    for (id, m) in &pack.maps {
        for y in 0..m.grid.h as i32 {
            for x in 0..m.grid.w as i32 {
                if let Some(Kind::Npc { name, routine, .. }) = m.grid.at(x, y) {
                    out.insert(format!("{id}@{x},{y}"), NpcAgent::new(id, (x, y), name, routine.clone()));
                }
            }
        }
    }
    out
}

/// The weather of every outdoor map that sets one (indoor maps never have any).
fn build_weather(pack: &Pack) -> BTreeMap<String, Weather> {
    pack.maps
        .iter()
        .filter(|(_, m)| m.outdoor)
        .filter_map(|(id, m)| m.weather.as_ref().map(|def| (id.clone(), Weather::from_def(id, def))))
        .collect()
}

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
    /// The distinct held item ids, for conditions. Always synced from `items` by `sync_inv`.
    pub inv: Vec<String>,
    /// What is carried: the nine hotbar slots and the bag.
    pub items: Inventory,
    /// The stack a use takes from (set by use_item_at for the length of one use).
    consume_from: Option<crate::inventory::Place>,
    /// Item tiles already picked up, as `map@x,y`.
    pub taken: BTreeSet<String>,
    /// Once-only scenes that have run (by scene id) and once-only trigger tiles (`tile:map@x,y`).
    pub fired: BTreeSet<String>,
    pub talk: Option<Talk>,
    /// A short message and the seconds it has left.
    pub toast: Option<(String, f32)>,
    pub move_cool: f32,
    /// The id of the story being played (a pack id; the sample story's is the core pack's).
    pub story: String,
    /// The player character of this story: the name shown on speaker "You", the speed, and the body.
    pub player: Player,
    /// The third-person view (the V key): the camera sits behind the body and the body is drawn. First person by default.
    pub third_person: bool,
    /// Hit points and stats (no experience, no levels).
    pub party: Party,
    /// The real-time battle being fought, if any. While it is Some, exits do not fire and NPC talk is off; movement
    /// stays live (until the player is down).
    pub battle: Option<ActionBattle>,
    /// The deterministic generator for damage rolls and attack picks, seeded from the story id.
    pub rng: Rng,
    /// Every NPC in the world, by `map@x,y` of its home tile. Only the agents on the current map are ticked.
    pub npcs: BTreeMap<String, NpcAgent>,
    /// The key of the NPC the player is talking to (it stands still until the scene ends).
    pub talk_npc: Option<String>,
    /// The weather of each outdoor map that has any, by map id. A map keeps its state while the player is away.
    pub weather: BTreeMap<String, Weather>,
    /// Sitting or lying on a prop: the seat position (x, z) and the seat height in metres. Movement keys stand up.
    pub seated: Option<(f32, f32, f32)>,
    /// A short prop use under way (the camera nudges) and the seconds it has left.
    pub use_anim: Option<(PropAction, f32)>,
}

/// One quest as the journal shows it: each step with whether it is done.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestView {
    pub title: String,
    pub description: String,
    pub steps: Vec<(String, bool)>,
    pub done: bool,
}

impl Game {
    /// A game for the first story in the pack, with no saved flags. Tests use it.
    #[cfg(test)]
    pub fn new(pack: Pack) -> Result<Game, String> {
        let id = pack.stories.first().map(|s| s.id.clone()).ok_or("the pack offers no story")?;
        Game::for_story(pack, &id, BTreeSet::new())
    }

    /// A game that plays `story_id` from its own start. The story's start items go into the pack, its start flags are set
    /// along with the `saved` flags (so the start scene sees both), and its player profile is kept on the game.
    pub fn for_story(pack: Pack, story_id: &str, saved: BTreeSet<String>) -> Result<Game, String> {
        let story = pack.story(story_id).cloned().ok_or_else(|| format!("no story '{story_id}' in the loaded packs"))?;
        let pos = pack
            .maps
            .get(&story.start_map)
            .and_then(|m| m.spawns.get(&story.start_spawn).copied())
            .ok_or_else(|| format!("story '{story_id}': start spawn '{}' is not on map '{}'", story.start_spawn, story.start_map))?;
        let (bx, bz) = tile_centre(pos.0, pos.1);
        // The start faces south unless the start spawn names a "facing".
        let yaw = pack
            .maps
            .get(&story.start_map)
            .and_then(|m| m.spawn_facing.get(&story.start_spawn).copied())
            .unwrap_or(std::f32::consts::PI);
        let mut body = Body::new((bx, bz), yaw);
        body.speed = story.player.speed;
        let mut flags = saved;
        flags.extend(story.player.start_flags.iter().cloned());
        let mut game = Game {
            map: story.start_map.clone(),
            pos,
            // Start facing south (+z), into the room; yaw 0 would face the map's north wall.
            body,
            flags,
            inv: Vec::new(),
            items: Inventory::new(),
            consume_from: None,
            taken: BTreeSet::new(),
            fired: BTreeSet::new(),
            talk: None,
            toast: None,
            move_cool: 0.0,
            story: story.id.clone(),
            player: story.player.clone(),
            third_person: false,
            party: Party::new(story.player.stats),
            battle: None,
            rng: Rng::new(Rng::seed_from(&story.id)),
            npcs: build_agents(&pack),
            talk_npc: None,
            weather: build_weather(&pack),
            seated: None,
            use_anim: None,
            pack,
        };
        for item in &story.player.start_items {
            game.give_item(item);
        }
        if let Some(scene) = story.start_scene.clone() {
            game.start_scene(&scene);
        }
        Ok(game)
    }

    /// The name a line's speaker shows as. The speaker "You" is the player, so it shows the player's name; anything
    /// else is shown as written. (An empty speaker is narration, and the caller does not ask for its name.)
    pub fn speaker_label(&self, speaker: &str) -> String {
        if speaker == "You" {
            self.player.name.clone()
        } else {
            speaker.to_string()
        }
    }

    pub fn current_map(&self) -> &MapDef {
        &self.pack.maps[&self.map]
    }

    pub fn kind_at(&self, x: i32, y: i32) -> Option<&Kind> {
        self.current_map().grid.at(x, y)
    }

    /// A tile is blocked when it is off the map, or its kind blocks (walls, NPCs, locked doors).
    fn blocked(&self, x: i32, y: i32) -> bool {
        tile_blocks(&self.pack, &self.map, &self.flags, &self.inv, x, y)
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

    /// The V key: switches between the first-person view and the third-person one.
    pub fn toggle_third_person(&mut self) {
        self.third_person = !self.third_person;
    }

    /// Moves the body for one frame of input, then runs what the tile it entered does. While a scene is playing (or the
    /// player is down in a battle) the body only looks around. A held block slows it to half speed.
    pub fn update_body(&mut self, dt: f32, input: &Input) {
        let down = self.battle.as_ref().map_or(false, |b| b.outcome == Some(Outcome::Lost));
        let input = if self.talk.is_some() || down {
            Input {
                look: input.look,
                ..Default::default()
            }
        } else {
            *input
        };
        // Seated: only looking around is allowed; a move key stands the body up first.
        if self.seated.is_some() {
            if input.fwd != 0.0 || input.strafe != 0.0 || input.jump {
                self.stand_up();
            } else {
                self.body.look(input.look);
                self.body.moving = false;
                return;
            }
        }
        self.body.speed = if self.blocking() { self.player.speed * crate::rpg::BLOCK_SLOW } else { self.player.speed };
        let start = self.body.pos;
        let pack = &self.pack;
        let (map, flags, inv) = (&self.map, &self.flags, &self.inv);
        self.body
            .update(dt, &input, &|tx, ty| tile_blocks(pack, map, flags, inv, tx, ty));
        // Every tile the body crossed this frame is entered in order, so a fast frame cannot skip an item or a trigger.
        let end = self.body.pos;
        let map_at_start = self.map.clone();
        let dist = ((end.0 - start.0).powi(2) + (end.1 - start.1).powi(2)).sqrt();
        let steps = ((dist / (TILE_M * 0.25)).ceil() as usize).clamp(1, 64);
        for i in 1..=steps {
            if self.talk.is_some() || self.map != map_at_start {
                break;
            }
            let t = i as f32 / steps as f32;
            let p = (start.0 + (end.0 - start.0) * t, start.1 + (end.1 - start.1) * t);
            let tile = ((p.0 / TILE_M).floor() as i32, (p.1 / TILE_M).floor() as i32);
            if tile != self.pos {
                self.arrive(tile.0, tile.1);
            }
        }
        // Walking into a locked door says why it will not open (once per toast).
        if input.fwd > 0.0 && self.toast.is_none() {
            let (fx, fz) = self.body.forward2();
            let ahead = self.body.pos.0 + fx * (RADIUS + 0.2);
            let ahead_z = self.body.pos.1 + fz * (RADIUS + 0.2);
            let tile = ((ahead / TILE_M).floor() as i32, (ahead_z / TILE_M).floor() as i32);
            if tile != self.pos && self.blocked(tile.0, tile.1) {
                if let Some(Kind::Door { locked, .. }) = self.kind_at(tile.0, tile.1).cloned() {
                    self.toast(locked);
                }
            }
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
                    self.give_item(&item);
                    self.toast(format!("Picked up {name}"));
                }
            }
            // Scenes from the floor wait until the fight is over.
            Some(Kind::Trigger { .. }) if self.battle.is_some() => {}
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
        if self.battle.is_some() {
            self.toast("You can't leave now");
            return;
        }
        match self.pack.maps.get(map).and_then(|m| m.spawns.get(spawn).copied()) {
            Some(at) => {
                self.seated = None;
                self.map = map.to_string();
                self.pos = at;
                self.place_body(at.0, at.1);
                self.face_spawn(spawn);
            }
            None => self.toast(format!("No spawn '{spawn}' on '{map}'")),
        }
    }

    /// E / Space / Enter: talk to the NPC in front, or move the dialogue on.
    pub fn interact(&mut self) {
        if self.battle.is_some() {
            return;
        }
        if self.talk.is_some() {
            self.advance();
            return;
        }
        if self.seated.is_some() {
            self.stand_up();
            return;
        }
        // The nearest NPC in front of the body, by position (a walking NPC is not on its home tile).
        if let Some((key, scene)) = self.npc_ahead() {
            self.start_scene(&scene);
            if self.talk.is_some() {
                self.talk_npc = Some(key);
            }
            return;
        }
        if self.use_prop() {
            return;
        }
        // A still NPC on the tile in front (no routine) is always reachable.
        let (tx, ty) = self.facing_tile();
        if let Some(Kind::Npc { scene, routine: None, .. }) = self.kind_at(tx, ty).cloned() {
            self.start_scene(&scene);
        }
    }

    /// The NPC E would talk to: the nearest one within 2 m whose direction is within about 70 degrees of the facing.
    /// Returns its key and scene.
    /// Whether a round body of radius `r` at `p` stands only on open tiles (its four corners checked).
    fn fits(&self, p: (f32, f32), r: f32) -> bool {
        [(-r, -r), (r, -r), (-r, r), (r, r)].iter().all(|(ox, oz)| {
            let (x, z) = (p.0 + ox, p.1 + oz);
            !self.blocked((x / TILE_M).floor() as i32, (z / TILE_M).floor() as i32)
        })
    }

    /// The name of the NPC E would talk to now (the HUD's "[E] Talk to" uses this, so the prompt and E agree).
    pub fn npc_ahead_name(&self) -> Option<String> {
        let (key, _) = self.npc_ahead()?;
        self.npcs.get(&key).map(|a| a.name.clone())
    }

    fn npc_ahead(&self) -> Option<(String, String)> {
        let (fx, fz) = self.body.forward2();
        let (bx, bz) = self.body.pos;
        let mut best: Option<(f32, &String, &NpcAgent)> = None;
        for (key, a) in self.npcs.iter().filter(|(_, a)| a.map == self.map) {
            let (dx, dz) = (a.pos.0 - bx, a.pos.1 - bz);
            let d = (dx * dx + dz * dz).sqrt();
            if d > TALK_RANGE_M + 1e-3 {
                continue;
            }
            let dot = if d < 0.4 { 1.0 } else { (dx * fx + dz * fz) / d };
            if dot < 0.35 {
                continue;
            }
            // Not through a wall: the tiles between the body and the NPC must be open.
            if !crate::rpg::line_clear((bx, bz), a.pos, &|tx, ty| self.blocked(tx, ty)) {
                continue;
            }
            if best.map_or(true, |b| d < b.0) {
                best = Some((d, key, a));
            }
        }
        let (_, key, a) = best?;
        match self.pack.maps.get(&a.map)?.grid.at(a.home.0, a.home.1)? {
            Kind::Npc { scene, .. } => Some((key.clone(), scene.clone())),
            _ => None,
        }
    }

    /// The usable prop on the tile in front, if any (the HUD shows its prompt).
    #[allow(dead_code)]
    pub fn usable_prop_ahead(&self) -> Option<&PropUse> {
        let (tx, ty) = self.facing_tile();
        match self.kind_at(tx, ty) {
            Some(Kind::Prop { use_: Some(u), .. }) => Some(u),
            _ => None,
        }
    }

    /// E on a usable prop: sit or sleep seats the body, other uses nudge the camera and toast the prop's text; a scene
    /// the prop names starts. False when there is nothing to use.
    pub fn use_prop(&mut self) -> bool {
        let (tx, ty) = self.facing_tile();
        let Some(Kind::Prop { use_: Some(u), .. }) = self.kind_at(tx, ty).cloned() else { return false };
        match u.action {
            PropAction::Sit | PropAction::Sleep => {
                let c = tile_centre(tx, ty);
                self.seated = Some((c.0, c.1, u.seat_height));
                self.body.pos = c;
                self.body.moving = false;
                self.pos = (tx, ty);
            }
            _ => {
                self.use_anim = Some((u.action, USE_SECS));
                self.toast(u.text.clone());
            }
        }
        if let Some(scene) = u.scene.clone() {
            self.start_scene(&scene);
        }
        true
    }

    /// Gets up from a seat (the body stays where the seat was).
    pub fn stand_up(&mut self) {
        self.seated = None;
    }

    /// The weather of the current map (None indoors and on maps without any).
    pub fn weather_now(&self) -> Option<&Weather> {
        if !self.current_map().outdoor {
            return None;
        }
        self.weather.get(&self.map)
    }

    /// The `weather` op: sets the current map's weather (outdoor maps only), blending in.
    fn set_weather(&mut self, kind: WeatherKind, intensity: Option<f32>) {
        let intensity = intensity.unwrap_or(DEFAULT_WEATHER_INTENSITY);
        if !self.current_map().outdoor {
            return;
        }
        let id = self.map.clone();
        match self.weather.get_mut(&id) {
            Some(w) => w.set(kind, intensity),
            None => {
                let w = Weather::new(&id, kind, intensity, &[], DEFAULT_EVERY_SECS);
                self.weather.insert(id, w);
            }
        }
    }

    /// Advances the weather of the current map.
    fn weather_tick(&mut self, dt: f32) {
        if !self.current_map().outdoor {
            return;
        }
        if let Some(w) = self.weather.get_mut(&self.map) {
            w.tick(dt);
        }
    }

    /// Advances the NPCs on the current map: routines, greetings, shelter, fleeing; then keeps them and the body apart.
    /// `tick` calls this every frame.
    pub fn npc_tick(&mut self, dt: f32) {
        if self.talk.is_none() {
            self.talk_npc = None;
        }
        let mut agents = std::mem::take(&mut self.npcs);
        let map_id = self.map.clone();
        let danger = self.battle.is_some();
        let wet = self.weather_now().map_or(false, |w| w.is_wet());
        let partners: Vec<_> = agents.values().filter(|a| a.map == map_id).map(partner_of).collect();
        if let Some(map) = self.pack.maps.get(&map_id) {
            let (pack, flags, inv) = (&self.pack, &self.flags, &self.inv);
            let blocks = |x: i32, y: i32| tile_blocks(pack, &map_id, flags, inv, x, y);
            let env = Env { pack, map, blocks: &blocks, player: self.body.pos, danger, wet, partners: &partners };
            for (key, a) in agents.iter_mut().filter(|(_, a)| a.map == map_id) {
                a.talk_lock = self.talk.is_some() && self.talk_npc.as_deref() == Some(key.as_str());
                tick_agent(a, &env, dt);
            }
        }
        // Bodies do not overlap: a walking NPC and the body share the push, a still NPC leaves it all to the body.
        let min = NPC_RADIUS + RADIUS;
        for a in agents.values_mut().filter(|a| a.map == map_id) {
            let Some((dir, overlap)) = push_apart(self.body.pos, a.pos, min) else { continue };
            let share = if a.is_static() { 1.0 } else { 0.5 };
            let to = (self.body.pos.0 + dir.0 * overlap * share, self.body.pos.1 + dir.1 * overlap * share);
            let mut rest = overlap;
            // The whole body must fit where it is pushed (its radius clear of walls), not just its centre.
            if self.fits(to, RADIUS) {
                self.body.pos = to;
                rest -= overlap * share;
            }
            if rest > 1e-4 && !a.is_static() && !a.talk_lock {
                let np = (a.pos.0 - dir.0 * rest, a.pos.1 - dir.1 * rest);
                if self.fits(np, NPC_RADIUS) {
                    a.pos = np;
                }
            }
        }
        self.npcs = agents;
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
        // A battle started by this scene's ops takes over: no dialogue under it.
        if self.battle.is_some() {
            self.talk = None;
            return;
        }
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
                    self.give_item(item);
                }
                Op::Take(item) => self.take_item(item),
                Op::Exit { map, spawn } => self.leave(map, spawn),
                Op::Toast(text) => self.toast(text.clone()),
                Op::Battle(id) => self.start_battle(id),
                Op::Heal(amount) => {
                    self.party.heal(*amount);
                }
                Op::Weather { kind, intensity } => self.set_weather(*kind, *intensity),
            }
        }
    }

    // ---- RPG: party, action battles, items and quests ----

    /// The party as it is saved: hp and max hp only. Hp 0 is never saved (a loss shows the DEFEATED banner with 0 hp
    /// before the restore), so it is clamped to 1.
    #[allow(dead_code)]
    pub fn party_save(&self) -> PartySave {
        PartySave { hp: self.party.hp.max(1), max_hp: self.party.max_hp }
    }

    /// Puts a saved party back, keeping the story's own stats as the base.
    #[cfg(test)]
    pub fn restore_party(&mut self, save: &PartySave) {
        self.party = Party::restored(self.player.stats, save.hp, save.max_hp);
    }

    /// Where a battle's enemy appears: the battle's `spawn` on the current map, or (no such spawn) a few metres in
    /// front of the body, or behind it when that is a wall. The spot is then moved to the nearest one where the enemy's
    /// body (all four 0.4 m corners) fits on open tiles.
    fn battle_spawn(&self, def: &Battle) -> (f32, f32) {
        let blocks = |tx: i32, ty: i32| self.blocked(tx, ty);
        if let Some(at) = self.current_map().spawns.get(&def.spawn) {
            return crate::rpg::fit_spawn(tile_centre(at.0, at.1), &blocks);
        }
        let (fx, fz) = self.body.forward2();
        let (x, z) = self.body.pos;
        for k in [3.0f32, -3.0, 1.5, -1.5] {
            let p = (x + fx * k, z + fz * k);
            if !self.blocked((p.0 / TILE_M).floor() as i32, (p.1 / TILE_M).floor() as i32) {
                return crate::rpg::fit_spawn(p, &blocks);
            }
        }
        crate::rpg::fit_spawn((x, z), &blocks)
    }

    /// Starts battle `id`: the current talk ends first and the enemy appears at the battle's spawn. A battle that is
    /// already on does nothing; an unknown id shows a message and changes nothing else (the talk stays).
    pub fn start_battle(&mut self, id: &str) {
        if self.battle.is_some() {
            return;
        }
        let Some(def) = self.pack.battles.get(id).cloned() else {
            self.toast(format!("Missing battle '{id}'"));
            return;
        };
        self.talk = None;
        let at = self.battle_spawn(&def);
        let mut fight = ActionBattle::new(&def, at);
        fight.enemy_yaw = yaw_of(self.body.pos.0 - at.0, self.body.pos.1 - at.1);
        self.battle = Some(fight);
        self.seated = None;
        if !def.intro.trim().is_empty() {
            self.toast(def.intro.clone());
        }
    }

    /// Advances the fight: the enemy, the shots and the banner clock. When the enemy falls the `won_<id>` flag is set at
    /// once (no experience); at 0 hp the `lost_<id>` flag is set. The battle closes after the banner and the win
    /// or lose scene starts (a loss also restores hp and puts the body back at the story's start).
    pub fn battle_tick(&mut self, dt: f32) {
        // Look the definition up before taking the battle, so a missing definition never drops the fight.
        let Some(def) = self.battle.as_ref().and_then(|f| self.pack.battles.get(&f.battle)).cloned() else { return };
        let Some(mut fight) = self.battle.take() else { return };
        {
            let (pack, map, flags, inv) = (&self.pack, &self.map, &self.flags, &self.inv);
            let blocks = |tx: i32, ty: i32| tile_blocks(pack, map, flags, inv, tx, ty);
            let mut cx = Ctx { player: self.body.pos, party: &mut self.party, rng: &mut self.rng, blocks: &blocks };
            fight.tick(&def, dt, &mut cx);
        }
        let Some(outcome) = fight.outcome else {
            self.battle = Some(fight);
            return;
        };
        if !fight.settled {
            fight.settled = true;
            match outcome {
                Outcome::Won => {
                    self.flags.insert(format!("won_{}", def.id));
                }
                Outcome::Lost => {
                    self.flags.insert(format!("lost_{}", def.id));
                }
            }
        }
        fight.end_t += dt.max(0.0);
        if fight.end_t < END_SECS {
            self.battle = Some(fight);
            return;
        }
        // The banner is done: the battle closes and the follow-up scene starts.
        let scene = match outcome {
            Outcome::Won => def.win.clone(),
            Outcome::Lost => {
                self.party.hp = self.party.max_hp;
                self.back_to_start();
                def.lose.clone()
            }
        };
        if let Some(scene) = scene {
            self.start_scene(&scene);
        }
    }

    /// Puts the body back at the story's start spawn (after a lost fight).
    fn back_to_start(&mut self) {
        let Some(story) = self.pack.story(&self.story).cloned() else { return };
        if let Some(at) = self.pack.maps.get(&story.start_map).and_then(|m| m.spawns.get(&story.start_spawn).copied()) {
            self.seated = None;
            self.map = story.start_map;
            self.pos = at;
            self.place_body(at.0, at.1);
            self.face_spawn(&story.start_spawn);
        }
    }

    /// Turns the body the way spawn `spawn` on the current map faces, when it names a facing (an exit or a reset
    /// arrives there). A spawn without one leaves the facing as it was.
    fn face_spawn(&mut self, spawn: &str) {
        if let Some(yaw) = self.current_map().spawn_facing.get(spawn).copied() {
            self.body.yaw = yaw;
        }
    }

    /// The attack button. Hits the enemy when it is within 2 m and inside the 100 degree arc in front of the body;
    /// a swing has a 0.45 s cooldown whether it hits or not. True when it hit. Does nothing outside a battle.
    pub fn player_attack(&mut self) -> bool {
        let Some(mut fight) = self.battle.take() else { return false };
        let hit = match self.pack.battles.get(&fight.battle) {
            Some(def) => {
                let (pack, map, flags, inv) = (&self.pack, &self.map, &self.flags, &self.inv);
                let blocks = |tx: i32, ty: i32| tile_blocks(pack, map, flags, inv, tx, ty);
                fight.player_attack(def, self.body.pos, self.body.yaw, &self.party, &mut self.rng, &blocks)
            }
            None => false,
        };
        self.battle = Some(fight);
        hit
    }

    /// The block button (held): cuts damage taken by 60 percent and slows the body to half speed.
    pub fn set_block(&mut self, on: bool) {
        if let Some(fight) = self.battle.as_mut() {
            fight.blocking = on;
        }
    }

    /// Whether the block is up right now.
    pub fn blocking(&self) -> bool {
        self.battle.as_ref().map_or(false, |b| b.blocking)
    }

    /// Rebuilds `inv` (the distinct held ids) from the inventory. Called after every change.
    pub fn sync_inv(&mut self) {
        self.inv = self.items.held();
    }

    /// The most of `item` one stack holds (at least 1).
    pub fn stack_limit(&self, item: &str) -> u32 {
        self.pack.items.get(item).map_or(1, |i| i.stack.max(1))
    }

    /// Adds one of `item` to the inventory.
    pub fn give_item(&mut self, item: &str) {
        let max = self.stack_limit(item);
        self.items.add(item, 1, max);
        self.sync_inv();
    }

    /// Removes one of `item` when held.
    pub fn take_item(&mut self, item: &str) {
        self.items.remove(item, 1);
        self.sync_inv();
    }

    /// Uses the item in the selected hotbar slot. False when the slot is empty or the item is not usable right now.
    pub fn use_selected(&mut self) -> bool {
        self.use_item_at(crate::inventory::Place::Slot(self.items.selected))
    }

    /// Uses the item in the stack at `place` (the selected slot, or the one under the inventory cursor); a consumable is
    /// taken from that same stack, not from the first stack of that item.
    pub fn use_item_at(&mut self, place: crate::inventory::Place) -> bool {
        let item = match place {
            crate::inventory::Place::Slot(i) => self.items.slots.get(i).and_then(|s| s.as_ref()).map(|s| s.item.clone()),
            crate::inventory::Place::Bag(j) => self.items.bag.get(j).map(|s| s.item.clone()),
        };
        let Some(item) = item else { return false };
        self.consume_from = Some(place);
        let used = self.use_item(&item);
        self.consume_from = None;
        used
    }

    /// Removes a used item from the pack when it is consumable (from the stack the player used, when known).
    fn consume(&mut self, item: &str) {
        if self.pack.items.get(item).map_or(false, |i| i.consumable) {
            match self.consume_from {
                Some(place) => {
                    self.items.remove_one_at(place, item);
                    self.sync_inv();
                }
                None => self.take_item(item),
            }
        }
    }

    /// Uses an item, in a battle or out of one. A heal item heals (refused at full hp, so it is not wasted); a damage
    /// item hurts the enemy when one is within 6 m (refused otherwise). A consumable item is removed. True when used.
    pub fn use_item(&mut self, item: &str) -> bool {
        if !self.items.has(item) {
            return false;
        }
        let Some(effect) = self.pack.items.get(item).and_then(|i| i.use_effect) else {
            return false;
        };
        if self.battle.as_ref().map_or(false, |b| b.outcome == Some(Outcome::Lost)) {
            return false;
        }
        let name = self.item_name(item);
        let at = self.body.pos;
        match effect {
            ItemUse::Heal(n) => {
                if self.party.hp >= self.party.max_hp {
                    return false;
                }
                let gained = self.party.heal(n);
                if let Some(fight) = self.battle.as_mut() {
                    fight.note_player_heal(at, gained);
                }
                self.toast(format!("Used {name}: +{gained} HP"));
            }
            ItemUse::Damage(n) => {
                let Some(mut fight) = self.battle.take() else { return false };
                let near = fight.outcome.is_none()
                    && fight.state != EnemyState::Dead
                    && ((fight.enemy_pos.0 - at.0).powi(2) + (fight.enemy_pos.1 - at.1).powi(2)).sqrt() <= ITEM_RANGE;
                if !near {
                    self.battle = Some(fight);
                    return false;
                }
                if let Some(def) = self.pack.battles.get(&fight.battle) {
                    fight.hurt_enemy(def, n.max(1));
                }
                self.battle = Some(fight);
                self.toast(format!("Used {name}: {} damage", n.max(1)));
            }
        }
        self.consume(item);
        true
    }

    /// The journal: every quest whose `show_if` holds (a quest with none is always shown) or that has any step done.
    /// Whether quest `id` belongs to the story being played. A mod's ids start with "<pack id>_", so the quest belongs to
    /// the mod pack whose prefix it carries; a quest with no mod prefix is the core pack's. Packs merge into one set, so
    /// without this every story would list every other story's quests.
    fn quest_belongs_here(&self, id: &str) -> bool {
        let owner = self
            .pack
            .packs
            .iter()
            .filter(|p| !p.core && id.starts_with(&format!("{}_", p.id)))
            .max_by_key(|p| p.id.len())
            .or_else(|| self.pack.packs.iter().find(|p| p.core));
        owner.map_or(true, |p| p.id == self.story)
    }

    pub fn quest_log(&self) -> Vec<QuestView> {
        self.pack
            .quests
            .values()
            .filter(|q| self.quest_belongs_here(&q.id))
            .filter_map(|q| {
                let steps: Vec<(String, bool)> = q
                    .steps
                    .iter()
                    .map(|s| (s.text.clone(), s.done_if.eval(&self.flags, &self.inv)))
                    .collect();
                let any_done = steps.iter().any(|(_, d)| *d);
                let shown = q.show_if.as_ref().map_or(true, |c| c.eval(&self.flags, &self.inv));
                if !(shown || any_done) {
                    return None;
                }
                let done = !steps.is_empty() && steps.iter().all(|(_, d)| *d);
                Some(QuestView {
                    title: q.title.clone(),
                    description: q.description.clone(),
                    steps,
                    done,
                })
            })
            .collect()
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
        if let Some(u) = self.use_anim.as_mut() {
            u.1 -= dt;
        }
        if self.use_anim.as_ref().map_or(false, |u| u.1 <= 0.0) {
            self.use_anim = None;
        }
        // NPCs are ticked by the frame loop (main.rs calls npc_tick once, and not while a panel is open).
        self.weather_tick(dt);
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
            spawn_facing: BTreeMap::new(),
            outdoor: false,
            weather: None,
            ceiling_height: 3.0,
            ceiling_color: None,
            sky: None,
            wall_style: "lab".into(),
            floor_style: "blocks".into(),
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
            (
                'F',
                Kind::Npc {
                    name: "Felix".into(),
                    scene: "felix".into(),
                    color: [111, 191, 138],
                    body: BodyChoice::Blocky,
                    skin: None,
                    routine: None,
                },
            ),
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
        items.insert("keycard".to_string(), Item { name: "Keycard".into(), description: String::new(), use_effect: None, consumable: false, stack: 1, icon: None });
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
            battles: BTreeMap::new(),
            quests: BTreeMap::new(),
            props: BTreeMap::new(),
            skins: BTreeMap::new(),
            routines: BTreeMap::new(),
            packs: Vec::new(),
            stories: vec![story("test", "lab", "start", Player::default())],
        }
    }

    /// A story that starts at `map`'s spawn `spawn` with the given player.
    fn story(id: &str, map: &str, spawn: &str, player: Player) -> StoryInfo {
        StoryInfo {
            id: id.into(),
            title: id.into(),
            era: Era::Sample,
            order: 0.0,
            start_map: map.into(),
            start_spawn: spawn.into(),
            start_scene: None,
            player,
            multiplayer: false,
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
        assert_eq!(g.items.count("keycard"), 1, "and only one is carried");
    }

    #[test]
    fn a_locked_door_opens_only_with_the_card() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((3, 2), (0, 1)); // above the door at (3,3)
        g.try_move(0, 1);
        assert_eq!(g.pos, (3, 2), "the door is locked");
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Needs a keycard."));
        g.give_item("keycard");
        g.try_move(0, 1);
        assert_eq!(g.pos, (3, 3), "the card opens it");
    }

    /// Walking into a locked door (not the tile-step helper) says why it stays shut, and E from a tile centre reaches the
    /// neighbouring tile facing north as well as south.
    #[test]
    fn walking_into_a_locked_door_shows_its_message_and_e_reaches_every_neighbour() {
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((3, 2), (0, 1)); // above the door at (3,3)
        g.update_body(0.3, &Input { fwd: 1.0, ..Default::default() });
        assert_eq!(g.pos, (3, 2), "the door is locked");
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Needs a keycard."));
        let mut g = Game::new(test_pack()).unwrap();
        g.place_at((3, 2), (0, -1));
        assert_eq!(g.facing_tile(), (3, 1), "north from a tile centre");
        g.place_at((3, 2), (-1, 0));
        assert_eq!(g.facing_tile(), (2, 2), "west from a tile centre");
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

    /// The test pack plus a second story, "night", that starts in the corridor with a keycard and a flag, played by Mara.
    fn two_story_pack() -> Pack {
        let mut pack = test_pack();
        pack.stories.push(story(
            "night",
            "corridor",
            "from-lab",
            Player {
                name: "Mara".into(),
                role: "Tester".into(),
                speed: 1.5,
                body: BodyChoice::BlockySlim,
                start_items: vec!["keycard".into()],
                start_flags: vec!["briefed".into()],
                stats: Stats::default(),
            },
        ));
        pack
    }

    #[test]
    fn the_v_key_toggles_third_person_and_the_story_player_sets_the_body() {
        let pack = two_story_pack();
        let mut game = Game::for_story(pack, "night", BTreeSet::new()).unwrap();
        assert!(!game.third_person, "first person is the default");
        assert_eq!(game.player.body, BodyChoice::BlockySlim, "the story's player body is kept on the game");
        game.toggle_third_person();
        assert!(game.third_person);
        game.toggle_third_person();
        assert!(!game.third_person);
    }

    #[test]
    fn for_story_starts_at_the_story_start_with_its_items_flags_and_player() {
        let pack = two_story_pack();
        let saved: BTreeSet<String> = ["asked".to_string()].into_iter().collect();
        let g = Game::for_story(pack, "night", saved).unwrap();
        assert_eq!(g.story, "night");
        assert_eq!(g.map, "corridor");
        assert_eq!(g.pos, (1, 1), "the story's spawn, not the pack default");
        assert_eq!(g.inv, vec!["keycard".to_string()], "the start items are carried");
        assert!(g.flags.contains("briefed") && g.flags.contains("asked"), "start and saved flags are both set");
        assert_eq!(g.player.name, "Mara");
        assert_eq!(g.body.speed, 1.5, "the story's speed reaches the body");
    }

    #[test]
    fn the_default_story_still_starts_at_the_pack_start() {
        let g = Game::new(two_story_pack()).unwrap();
        assert_eq!(g.story, "test");
        assert_eq!((g.map.as_str(), g.pos), ("lab", (1, 1)));
        assert_eq!(g.body.speed, 1.0);
        assert!(g.inv.is_empty());
    }

    #[test]
    fn an_unknown_story_id_is_an_error() {
        let err = Game::for_story(two_story_pack(), "nowhere", BTreeSet::new()).err().expect("no such story");
        assert!(err.contains("nowhere"), "{err}");
    }

    #[test]
    fn speaker_you_shows_the_player_name_and_other_speakers_do_not_change() {
        let g = Game::for_story(two_story_pack(), "night", BTreeSet::new()).unwrap();
        assert_eq!(g.speaker_label("You"), "Mara");
        assert_eq!(g.speaker_label("Felix"), "Felix");
        assert_eq!(g.speaker_label("you"), "you", "only the exact speaker \"You\" is the player");
    }

    // ---- RPG ----

    /// A harmless enemy that stands still: 0 move speed, no attacks. It appears at the lab's "inside" spawn, 4 m south of
    /// the start (the player faces south at the start).
    fn foe(id: &str, hp: i32, _xp: i32) -> Battle {
        Battle {
            id: id.into(),
            name: "Rat".into(),
            boss: false,
            max_hp: hp,
            defense: 0,
            move_speed: 0.0,
            skin: None,
            spawn: "inside".into(),
            attacks: Vec::new(),
            phases: Vec::new(),
            intro: String::new(),
            win: Some("won_scene".into()),
            lose: Some("lost_scene".into()),
        }
    }

    fn rpg_pack(battle: Battle) -> Pack {
        let mut pack = test_pack();
        pack.scenes.insert("won_scene".into(), scene(vec![line("", "Won.")]));
        pack.scenes.insert("lost_scene".into(), scene(vec![line("", "Lost.")]));
        pack.battles.insert(battle.id.clone(), battle);
        pack.items.insert(
            "tonic".into(),
            Item { name: "Tonic".into(), description: String::new(), use_effect: Some(ItemUse::Heal(10)), consumable: true, stack: 9, icon: None },
        );
        pack.items.insert(
            "bomb".into(),
            Item { name: "Bomb".into(), description: String::new(), use_effect: Some(ItemUse::Damage(9)), consumable: true, stack: 9, icon: None },
        );
        pack
    }

    /// A game with the fight "foe" running and its intro skipped.
    fn fighting(battle: Battle) -> Game {
        let mut g = Game::new(rpg_pack(battle)).unwrap();
        g.start_battle("foe");
        g.battle.as_mut().unwrap().state = EnemyState::Chase;
        g
    }

    fn tick_for(g: &mut Game, secs: f32) {
        for _ in 0..(secs / 0.05).round() as usize {
            g.battle_tick(0.05);
        }
    }

    /// Puts the enemy 1.5 m in front of the player (who faces south).
    fn enemy_in_front(g: &mut Game) {
        let (x, z) = g.body.pos;
        g.battle.as_mut().unwrap().enemy_pos = (x, z + 1.5);
    }

    #[test]
    fn a_battle_puts_the_enemy_at_its_spawn_and_a_missing_spawn_falls_back_in_front() {
        let mut g = Game::new(rpg_pack(foe("foe", 20, 0))).unwrap();
        g.start_battle("foe");
        let b = g.battle.as_ref().unwrap();
        assert_eq!(b.enemy_pos, tile_centre(1, 3));
        assert!(matches!(b.state, EnemyState::Intro { .. }));

        let mut lost = foe("foe", 20, 0);
        lost.spawn = "nowhere".into();
        let mut g = Game::new(rpg_pack(lost)).unwrap();
        g.start_battle("foe");
        let (e, p) = (g.battle.as_ref().unwrap().enemy_pos, g.body.pos);
        assert!((((e.0 - p.0).powi(2) + (e.1 - p.1).powi(2)).sqrt() - 3.0).abs() < 1e-3, "3 m ahead");

        // An intro line shows as a toast; the talk ends.
        let mut with_intro = foe("foe", 20, 0);
        with_intro.intro = "A rat!".into();
        let mut g = Game::new(rpg_pack(with_intro)).unwrap();
        g.start_scene("note");
        g.start_battle("foe");
        assert!(g.talk.is_none());
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("A rat!"));
    }

    #[test]
    fn the_player_swing_hits_in_range_and_arc_only_and_has_a_cooldown() {
        let mut g = fighting(foe("foe", 1000, 0));
        let hp = |g: &Game| g.battle.as_ref().unwrap().enemy_hp;
        // The enemy spawned 4 m away: out of reach.
        assert!(!g.player_attack());
        assert_eq!(hp(&g), 1000);
        tick_for(&mut g, 0.5);
        enemy_in_front(&mut g);
        assert!(g.player_attack());
        let after_first = hp(&g);
        assert!((988..1000).contains(&after_first), "{after_first}");
        assert!(!g.player_attack(), "inside the 0.45 s cooldown");
        assert_eq!(hp(&g), after_first);
        tick_for(&mut g, 0.5);
        assert!(g.player_attack(), "ready again");
        assert!(hp(&g) < after_first);
        // Behind the player: in reach but outside the arc.
        tick_for(&mut g, 0.5);
        let (x, z) = g.body.pos;
        g.battle.as_mut().unwrap().enemy_pos = (x, z - 1.0);
        let before = hp(&g);
        assert!(!g.player_attack());
        assert_eq!(hp(&g), before);
        // No battle: nothing to hit.
        g.battle = None;
        assert!(!g.player_attack());
    }

    #[test]
    fn winning_gives_the_flag_and_no_xp_and_the_win_scene_after_the_banner() {
        let mut g = fighting(foe("foe", 10, 25));
        enemy_in_front(&mut g);
        for _ in 0..10 {
            g.player_attack();
            tick_for(&mut g, 0.5);
            if g.battle.as_ref().unwrap().outcome.is_some() {
                break;
            }
        }
        assert_eq!(g.battle.as_ref().unwrap().outcome, Some(Outcome::Won));
        assert!(g.flags.contains("won_foe"));
        assert_eq!((g.party.max_hp, g.party.hp), (30, 30), "no xp, no level up");
        assert!(g.toast.is_none(), "no level toast");
        assert!(g.battle.is_some() && g.talk.is_none(), "the VICTORY banner shows first");
        tick_for(&mut g, 3.0);
        assert!(g.battle.is_none());
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("won_scene"));
    }

    #[test]
    fn losing_restores_hp_sets_the_flag_resets_the_position_and_plays_the_lose_scene() {
        let mut g = fighting(foe("foe", 500, 10));
        g.place_at((5, 2), (0, 1));
        let at = g.body.pos;
        g.party.hp = 3;
        g.battle.as_mut().unwrap().hurt_player(&mut g.party, 999, at);
        g.battle_tick(0.05);
        assert_eq!(g.battle.as_ref().unwrap().outcome, Some(Outcome::Lost));
        assert!(g.flags.contains("lost_foe") && !g.flags.contains("won_foe"));
        assert_eq!(g.party.hp, 0, "the DEFEATED banner shows with 0 hp first");
        // The body stands still while down.
        let still = g.body.pos;
        g.update_body(0.3, &Input { fwd: 1.0, ..Default::default() });
        assert_eq!(g.body.pos, still);
        tick_for(&mut g, 2.6);
        assert!(g.battle.is_none());
        assert_eq!(g.party.hp, g.party.max_hp);
        assert_eq!((g.map.as_str(), g.pos, g.body.pos), ("lab", (1, 1), tile_centre(1, 1)));
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("lost_scene"));
    }

    #[test]
    fn a_battle_with_no_follow_up_scene_just_closes() {
        let mut b = foe("foe", 1, 0);
        b.win = None;
        let mut g = fighting(b);
        enemy_in_front(&mut g);
        assert!(g.player_attack());
        tick_for(&mut g, 3.0);
        assert!(g.battle.is_none() && g.talk.is_none());
        assert!(g.flags.contains("won_foe"));
    }

    #[test]
    fn exits_do_not_fire_and_talk_is_off_during_a_battle() {
        let mut g = fighting(foe("foe", 100, 0));
        g.leave("corridor", "from-lab");
        assert_eq!(g.map, "lab");
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("You can't leave now"));
        // Stepping onto the exit tile does not take the player through, and an exit op is refused too.
        g.toast = None;
        g.place_at((5, 2), (0, 1));
        g.try_move(0, 1);
        assert_eq!((g.map.as_str(), g.pos), ("lab", (5, 3)));
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("You can't leave now"));
        g.run_ops(&[Op::Exit { map: "corridor".into(), spawn: "from-lab".into() }]);
        assert_eq!(g.map, "lab");
        // Pressing E at Felix does nothing.
        g.place_at((4, 1), (1, 0));
        g.interact();
        assert!(g.talk.is_none());
        // Movement stays live.
        let before = g.body.pos;
        g.update_body(0.2, &Input { fwd: 1.0, ..Default::default() });
        assert_ne!(g.body.pos, before);
        // After the fight the exit works again.
        g.battle = None;
        g.leave("corridor", "from-lab");
        assert_eq!(g.map, "corridor");
    }

    #[test]
    fn blocking_halves_the_walking_speed_and_the_damage() {
        let walk = |block: bool| {
            let mut g = fighting(foe("foe", 100, 0));
            g.set_block(block);
            let start = g.body.pos;
            g.update_body(0.2, &Input { fwd: 1.0, ..Default::default() });
            let d = ((g.body.pos.0 - start.0).powi(2) + (g.body.pos.1 - start.1).powi(2)).sqrt();
            (d, g.blocking())
        };
        let (free, free_flag) = walk(false);
        let (slow, slow_flag) = walk(true);
        assert!(!free_flag && slow_flag);
        assert!(free > 0.5);
        assert!((slow - free * 0.5).abs() < 0.02, "{slow} vs {free}");

        let mut g = fighting(foe("foe", 100, 0));
        g.set_block(true);
        let at = g.body.pos;
        let taken = g.battle.as_mut().unwrap().hurt_player(&mut g.party, 20, at);
        assert_eq!(taken, 8, "19 after defense, 60 percent off");
        g.set_block(false);
        assert!(!g.blocking());
        g.set_block(true);
        g.battle = None;
        g.set_block(true);
        assert!(!g.blocking(), "no battle, no block");
    }

    #[test]
    fn the_enemy_hurts_the_party_through_the_game() {
        let mut b = foe("foe", 100, 0);
        b.attacks.push(EnemyAttack {
            name: "bite".into(),
            kind: AttackKind::Melee,
            damage: 6,
            range: 2.0,
            windup: 0.3,
            cooldown: 5.0,
            weight: 1,
            min_phase: 0,
            radius: 0.0,
            speed: 0.0,
            text: String::new(),
        });
        let mut g = fighting(b);
        enemy_in_front(&mut g);
        tick_for(&mut g, 1.0);
        assert_eq!(g.party.hp, 30 - 5, "6 - defense 2 / 2");
        let shown: Vec<_> = g.battle.as_ref().unwrap().hits.iter().map(|h| h.text.clone()).collect();
        assert_eq!(shown, vec!["5".to_string()]);
    }

    #[test]
    fn items_heal_and_are_consumed_in_a_battle_and_outside() {
        let mut g = Game::new(rpg_pack(foe("foe", 50, 5))).unwrap();
        g.give_item("tonic");
        assert!(!g.use_item("tonic"), "full hp does not waste the item");
        assert_eq!(g.inv, vec!["tonic".to_string()]);
        g.party.hp = 5;
        assert!(g.use_item("tonic"));
        assert_eq!(g.party.hp, 15);
        assert!(g.inv.is_empty(), "consumed");
        assert!(!g.use_item("tonic"), "none left");
        assert!(!g.use_item("keycard"), "not a usable item");
        g.give_item("tonic");
        g.party.hp = 25;
        assert!(g.use_item("tonic"));
        assert_eq!(g.party.hp, 30, "capped at max hp");

        // In a battle the heal lands too, with a floating number, and the fight goes on.
        g.give_item("tonic");
        g.party.hp = 8;
        g.start_battle("foe");
        assert!(g.use_item("tonic"));
        assert_eq!(g.party.hp, 18);
        assert!(g.inv.is_empty());
        assert!(g.battle.as_ref().unwrap().hits.iter().any(|h| h.heal && !h.on_enemy && h.text == "+10"));
        assert!(g.battle.as_ref().unwrap().outcome.is_none());
    }

    #[test]
    fn a_damage_item_hurts_a_near_enemy_only_in_a_battle_and_can_win() {
        let mut g = Game::new(rpg_pack(foe("foe", 5, 0))).unwrap();
        g.give_item("bomb");
        assert!(!g.use_item("bomb"), "nothing to hit outside a battle");
        assert_eq!(g.inv, vec!["bomb".to_string()]);
        g.start_battle("foe");
        // Too far away (8 m): refused and kept.
        g.battle.as_mut().unwrap().enemy_pos = (11.0, 3.0);
        assert!(!g.use_item("bomb"));
        assert_eq!(g.inv.len(), 1);
        // The spawn is 4 m away: in range.
        g.battle.as_mut().unwrap().enemy_pos = tile_centre(1, 3);
        assert!(g.use_item("bomb"));
        assert!(g.inv.is_empty());
        assert_eq!(g.battle.as_ref().unwrap().outcome, Some(Outcome::Won));
        g.battle_tick(0.05);
        assert!(g.flags.contains("won_foe"));
    }

    #[test]
    fn a_non_consumable_item_stays_after_use() {
        let mut pack = rpg_pack(foe("foe", 50, 5));
        pack.items.insert(
            "charm".into(),
            Item { name: "Charm".into(), description: String::new(), use_effect: Some(ItemUse::Heal(2)), consumable: false, stack: 1, icon: None },
        );
        let mut g = Game::new(pack).unwrap();
        g.give_item("charm");
        g.party.hp = 10;
        assert!(g.use_item("charm") && g.use_item("charm"));
        assert_eq!(g.party.hp, 14);
        assert_eq!(g.inv, vec!["charm".to_string()]);
    }

    #[test]
    fn inv_stays_synced_through_give_take_and_stacks() {
        let mut g = Game::new(rpg_pack(foe("foe", 50, 5))).unwrap();
        g.give_item("tonic");
        g.give_item("tonic");
        g.give_item("keycard");
        assert_eq!(g.items.count("tonic"), 2);
        assert_eq!(g.items.slots[0].as_ref().unwrap().count, 2, "tonics stack");
        assert_eq!(g.inv, vec!["tonic".to_string(), "keycard".to_string()]);
        g.take_item("tonic");
        assert_eq!(g.inv, vec!["tonic".to_string(), "keycard".to_string()], "one tonic is left");
        g.take_item("tonic");
        assert_eq!(g.inv, vec!["keycard".to_string()]);
        g.take_item("tonic"); // none held: harmless
        assert_eq!(g.inv, vec!["keycard".to_string()]);
    }

    #[test]
    fn a_key_item_gives_a_second_copy_its_own_slot() {
        let mut g = Game::new(test_pack()).unwrap();
        g.give_item("keycard");
        g.give_item("keycard");
        assert_eq!(g.items.count("keycard"), 2);
        assert_eq!(g.items.slots.iter().flatten().count(), 2, "stack 1: two slots");
        assert_eq!(g.inv, vec!["keycard".to_string()], "inv lists the id once");
    }

    #[test]
    fn give_and_take_ops_go_through_the_inventory() {
        let mut g = Game::new(rpg_pack(foe("foe", 50, 5))).unwrap();
        g.run_ops(&[Op::Give("tonic".into()), Op::Give("tonic".into())]);
        assert_eq!(g.items.count("tonic"), 2);
        assert_eq!(g.inv, vec!["tonic".to_string()]);
        g.run_ops(&[Op::Take("tonic".into())]);
        assert_eq!(g.items.count("tonic"), 1);
        g.run_ops(&[Op::Take("tonic".into())]);
        assert!(g.inv.is_empty());
    }

    #[test]
    fn use_selected_heals_and_consumes_the_selected_slot() {
        let mut g = Game::new(rpg_pack(foe("foe", 50, 5))).unwrap();
        assert!(!g.use_selected(), "empty slot");
        g.give_item("keycard"); // slot 0
        g.give_item("tonic"); // slot 1
        g.give_item("tonic");
        g.party.hp = 5;
        g.items.select(0);
        assert!(!g.use_selected(), "a key item is not usable");
        g.items.select(1);
        assert!(g.use_selected());
        assert_eq!(g.party.hp, 15);
        assert_eq!(g.items.count("tonic"), 1);
        assert!(g.use_selected());
        assert_eq!(g.party.hp, 25);
        assert!(g.items.slots[1].is_none(), "the empty stack cleared its slot");
        assert_eq!(g.inv, vec!["keycard".to_string()]);
        assert!(!g.use_selected());
    }

    #[test]
    fn scene_ops_start_a_battle_and_heal_work() {
        let mut pack = rpg_pack(foe("foe", 50, 5));
        pack.scenes.insert(
            "ambush".into(),
            Scene {
                once: false,
                lines: vec![line("", "Something moves.")],
                choices: vec![],
                ops: vec![Op::Battle("foe".into())],
                next: Some("won_scene".into()),
            },
        );
        let mut g = Game::new(pack).unwrap();
        g.start_scene("ambush");
        g.advance();
        g.advance(); // lines done: the scene's ops start the battle, and its `next` does not play under it
        assert!(g.battle.is_some() && g.talk.is_none());

        let mut g = Game::new(rpg_pack(foe("foe", 5, 0))).unwrap();
        g.party.hp = 4;
        g.run_ops(&[Op::Heal(10)]);
        assert_eq!(g.party.hp, 14);
        g.party.hp = 1;
        g.run_ops(&[Op::Heal(1000)]);
        assert_eq!(g.party.hp, g.party.max_hp);
        g.run_ops(&[Op::Battle("ghost".into())]);
        assert!(g.battle.is_none());
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Missing battle 'ghost'"));
    }

    #[test]
    fn the_party_starts_from_the_story_players_stats() {
        let mut pack = two_story_pack();
        pack.stories[1].player.stats = Stats { max_hp: 50, attack: 9, defense: 4, speed: 6 };
        let g = Game::for_story(pack, "night", BTreeSet::new()).unwrap();
        assert_eq!((g.party.hp, g.party.max_hp, g.party.stats.attack), (50, 50, 9));
    }

    #[test]
    fn the_battle_is_the_same_every_run() {
        let run = || {
            let mut g = fighting(foe("foe", 1000, 0));
            enemy_in_front(&mut g);
            for _ in 0..6 {
                g.player_attack();
                tick_for(&mut g, 0.5);
            }
            g.battle.as_ref().unwrap().enemy_hp
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_quest_log_shows_quests_by_condition_or_progress_and_tracks_steps() {
        let mut pack = test_pack();
        pack.quests.insert(
            "card".into(),
            Quest {
                id: "card".into(),
                title: "The keycard".into(),
                description: "Get into the lab.".into(),
                show_if: Some(Cond::Flag("met".into())),
                steps: vec![
                    QuestStep { text: "Ask Felix".into(), done_if: Cond::Flag("asked".into()) },
                    QuestStep { text: "Hold the card".into(), done_if: Cond::Has("keycard".into()) },
                ],
            },
        );
        pack.quests.insert(
            "open".into(),
            Quest {
                id: "open".into(),
                title: "Always on".into(),
                description: String::new(),
                show_if: None,
                steps: vec![],
            },
        );
        let mut g = Game::new(pack).unwrap();
        let titles = |g: &Game| g.quest_log().into_iter().map(|q| q.title).collect::<Vec<_>>();
        assert_eq!(titles(&g), vec!["Always on"], "a quest with a closed show_if is hidden");
        g.flags.insert("met".into());
        assert_eq!(titles(&g), vec!["The keycard", "Always on"]);
        let log = g.quest_log();
        let q = &log[0];
        assert_eq!(q.steps, vec![("Ask Felix".to_string(), false), ("Hold the card".to_string(), false)]);
        assert!(!q.done);
        g.flags.insert("asked".into());
        assert_eq!(g.quest_log()[0].steps[0].1, true);
        assert!(!g.quest_log()[0].done);
        g.give_item("keycard");
        assert!(g.quest_log()[0].done);
        assert!(!g.quest_log()[1].done, "a quest with no steps is never done");

        // Packs merge into one set: a quest shows only in the story whose pack owns it (by its "<pack id>_" prefix).
        let info = |id: &str, core: bool| PackInfo {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            core,
            dir: PathBuf::new(),
            scenes: 0,
            items: 0,
            maps: 0,
        };
        g.pack.packs = vec![info("core_pack", true), info("city", false)];
        g.pack.quests.insert(
            "city_parcel".into(),
            Quest { id: "city_parcel".into(), title: "Deliver".into(), description: String::new(), show_if: None, steps: vec![] },
        );
        g.story = "core_pack".into();
        assert!(!titles(&g).contains(&"Deliver".to_string()), "another story's quest is hidden");
        assert!(titles(&g).contains(&"Always on".to_string()), "the core story keeps its own quests");
        g.story = "city".into();
        assert_eq!(titles(&g), vec!["Deliver"], "the city story lists only its quests");
        g.story = "core_pack".into();
        // A step done shows the quest even when show_if closes again.
        g.flags.remove("met");
        assert!(titles(&g).contains(&"The keycard".to_string()));
    }

    #[test]
    fn the_party_saves_and_restores_through_the_game() {
        let mut g = Game::new(rpg_pack(foe("foe", 5, 0))).unwrap();
        g.party.hp = 12;
        let save = g.party_save();
        let mut other = Game::new(rpg_pack(foe("foe", 5, 0))).unwrap();
        other.restore_party(&save);
        assert_eq!(other.party, g.party);
    }

    #[test]
    fn hp_zero_is_never_saved() {
        let mut g = Game::new(rpg_pack(foe("foe", 5, 0))).unwrap();
        g.party.hp = 0;
        assert_eq!(g.party_save().hp, 1, "clamped while the DEFEATED banner shows");
        assert_eq!(g.party_save().max_hp, 30);
    }

    #[test]
    fn a_battle_with_a_missing_definition_is_not_dropped_by_the_tick() {
        let mut g = fighting(foe("foe", 20, 0));
        g.pack.battles.clear();
        g.battle_tick(0.05);
        assert!(g.battle.is_some(), "the fight is kept, not silently dropped");
    }

    #[test]
    fn a_spawn_inside_a_wall_is_moved_out_by_the_game() {
        let mut g = Game::new(rpg_pack(foe("foe", 20, 0))).unwrap();
        // Put the battle's spawn on a wall tile of the lab (the corner (0, 0) is a wall).
        let m = g.pack.maps.get_mut("lab").unwrap();
        m.spawns.insert("inside".into(), (0, 0));
        g.start_battle("foe");
        let at = g.battle.as_ref().unwrap().enemy_pos;
        assert!(crate::rpg::fit_spawn(at, &|tx, ty| g.blocked(tx, ty)) == at, "fits");
        assert!(!g.blocked((at.0 / TILE_M).floor() as i32, (at.1 / TILE_M).floor() as i32));
    }

    // ---- NPC routines, props and weather ----

    use crate::npc_ai::{angle_diff, NpcAction};
    use crate::model::WeatherKind as Wk;

    fn do_of(a: NpcAction) -> NpcDo {
        NpcAction::NAMES.iter().position(|n| *n == a.name()).map(|i| NpcDo::parse(NpcDo::NAMES[i]).unwrap()).unwrap_or(NpcDo::Idle)
    }

    fn st(go: &str, act: NpcAction, secs: f32) -> RoutineStep {
        RoutineStep { go: Some(go.into()), do_: do_of(act), secs, with: None, radius: 0.0 }
    }

    fn rt(steps: Vec<RoutineStep>, shelter: Option<&str>, flee: Option<&str>) -> Routine {
        Routine { looping: true, steps, shelter: shelter.map(String::from), flee: flee.map(String::from) }
    }

    /// An open yard (11 x 5 tiles): Ann (5,1) and Bo (6,3) with routines "ann" and "bo" (when the pack gives them), a
    /// sit prop at (3,1), a work prop at (3,2), the spawns start (1,1), bench (7,1), well (9,1) and shed (1,3).
    fn yard_pack(routines: Vec<(&str, Routine)>) -> Pack {
        let sit = PropUse { action: PropAction::Sit, text: "Sit".into(), scene: None, seat_height: 0.45 };
        let hammer = PropUse { action: PropAction::Work, text: "Hammer".into(), scene: Some("note".into()), seat_height: 0.45 };
        let npc = |name: &str, scene: &str, routine: &str| Kind::Npc {
            name: name.into(),
            scene: scene.into(),
            color: [111, 191, 138],
            body: BodyChoice::Blocky,
            skin: None,
            routine: Some(routine.into()),
        };
        let legend = [
            ('#', wall()),
            ('.', floor()),
            ('S', Kind::Spawn { name: "start".into() }),
            ('B', Kind::Spawn { name: "bench".into() }),
            ('W', Kind::Spawn { name: "well".into() }),
            ('H', Kind::Spawn { name: "shed".into() }),
            ('N', npc("Ann", "felix", "ann")),
            ('M', npc("Bo", "thanks", "bo")),
            ('P', Kind::Prop { prop: "bench".into(), yaw: 0.0, color: [1, 2, 3], use_: Some(sit) }),
            ('K', Kind::Prop { prop: "anvil".into(), yaw: 0.0, color: [1, 2, 3], use_: Some(hammer) }),
        ];
        let mut yard = map(
            "yard",
            &["###########", "#S.P.N.B.W#", "#..K......#", "#H....M...#", "###########"],
            &legend,
        );
        yard.outdoor = true;
        yard.weather = Some(WeatherDef { kind: Wk::Clear, intensity: 0.0, changes: Vec::new(), every_secs: 1000.0 });
        let mut pack = test_pack();
        pack.maps.insert("yard".into(), yard);
        for (id, r) in routines {
            pack.routines.insert(id.into(), r);
        }
        pack.battles.insert("foe".into(), foe("foe", 20, 0));
        pack.stories = vec![story("test", "yard", "start", Player::default())];
        pack
    }

    fn yard_game(routines: Vec<(&str, Routine)>) -> Game {
        Game::new(yard_pack(routines)).unwrap()
    }

    fn npc_named<'a>(g: &'a Game, name: &str) -> &'a NpcAgent {
        g.npcs.values().find(|a| a.name == name).expect("no such npc")
    }

    fn run_npcs(g: &mut Game, secs: f32) {
        for _ in 0..(secs / 0.1).round() as usize {
            g.npc_tick(0.1);
        }
    }

    fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn every_npc_tile_gets_an_agent_and_a_routine_npc_does_not_block_its_tile() {
        let g = yard_game(vec![]);
        assert_eq!(g.npcs.len(), 3, "Ann and Bo in the yard, Felix in the lab");
        assert!(g.npcs.contains_key("yard@5,1") && g.npcs.contains_key("lab@5,1"));
        assert_eq!(npc_named(&g, "Ann").pos, tile_centre(5, 1));
        assert!(!g.blocked(5, 1), "Ann has a routine");
        let lab = Game::new(test_pack()).unwrap();
        assert!(lab.blocked(5, 1), "Felix has none and still blocks");
    }

    #[test]
    fn a_routine_walks_to_a_spawn_sits_for_its_time_then_goes_on_and_loops() {
        let steps = vec![st("bench", NpcAction::Sit, 3.0), st("well", NpcAction::Work, 2.0)];
        let mut g = yard_game(vec![("ann", rt(steps, None, None))]);
        run_npcs(&mut g, 0.1);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Walk);
        run_npcs(&mut g, 4.0);
        let a = npc_named(&g, "Ann");
        assert_eq!((a.action, a.tile()), (NpcAction::Sit, (7, 1)), "on the bench");
        run_npcs(&mut g, 3.0);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Walk, "the sit is over, on to the well");
        let (mut worked, mut sat_again) = (false, false);
        for _ in 0..600 {
            g.npc_tick(0.1);
            let a = npc_named(&g, "Ann");
            match a.action {
                NpcAction::Work => {
                    assert_eq!(a.tile(), (9, 1));
                    worked = true;
                }
                NpcAction::Sit if worked => {
                    sat_again = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(worked && sat_again, "worked at the well, then looped back to sit");
    }

    #[test]
    fn a_routine_that_does_not_loop_ends_standing_idle() {
        let mut r = rt(vec![st("bench", NpcAction::Sit, 1.0)], None, None);
        r.looping = false;
        let mut g = yard_game(vec![("ann", r)]);
        run_npcs(&mut g, 12.0);
        let a = npc_named(&g, "Ann");
        assert_eq!((a.base, a.tile()), (NpcAction::Idle, (7, 1)), "standing at the bench, not sitting again");
    }

    #[test]
    fn an_npc_greets_and_faces_the_player_then_carries_on_two_seconds_after_they_leave() {
        let mut g = yard_game(vec![("ann", rt(vec![st("bench", NpcAction::Sit, 3.0)], None, None))]);
        g.place_at((4, 1), (1, 0)); // 2 m west of Ann
        g.npc_tick(0.1);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Wave, "waves once");
        run_npcs(&mut g, 0.8);
        let (a, p) = (npc_named(&g, "Ann"), g.body.pos);
        assert!(angle_diff(a.yaw, yaw_of(p.0 - a.pos.0, p.1 - a.pos.1)).abs() < 0.05, "faces the player");
        assert!(!a.moving && a.greeted);
        assert_eq!(a.pos, tile_centre(5, 1), "stopped where she was");
        g.place_at((1, 3), (0, 1)); // far away
        run_npcs(&mut g, 0.6);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Idle, "still waiting");
        run_npcs(&mut g, 2.0);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Walk, "back to the routine");
        run_npcs(&mut g, 2.0);
        assert!(npc_named(&g, "Ann").pos.0 > tile_centre(5, 1).0 + 0.5, "walking toward the bench");
        assert!(!npc_named(&g, "Ann").greeted);
    }

    #[test]
    fn talking_locks_the_npc_until_the_scene_ends() {
        let mut g = yard_game(vec![("ann", rt(vec![st("bench", NpcAction::Sit, 3.0)], None, None))]);
        g.place_at((4, 1), (1, 0));
        g.interact();
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("felix"));
        assert_eq!(g.talk_npc.as_deref(), Some("yard@5,1"));
        run_npcs(&mut g, 2.0);
        let a = npc_named(&g, "Ann");
        assert!(a.talk_lock && !a.moving);
        assert_eq!(a.pos, tile_centre(5, 1));
        let p = g.body.pos;
        assert!(angle_diff(a.yaw, yaw_of(p.0 - a.pos.0, p.1 - a.pos.1)).abs() < 0.05);
        g.advance();
        g.advance();
        assert!(g.talk.as_ref().unwrap().choosing);
        g.choose(1);
        assert!(g.talk.is_none());
        g.npc_tick(0.1);
        assert!(!npc_named(&g, "Ann").talk_lock);
        assert_eq!(g.talk_npc, None);
    }

    #[test]
    fn rain_sends_a_sheltering_npc_to_its_shelter_and_clearing_up_sends_it_back() {
        let steps = vec![st("bench", NpcAction::Sit, 3.0), st("well", NpcAction::Work, 2.0)];
        let mut g = yard_game(vec![("ann", rt(steps, Some("shed"), None))]);
        g.place_at((9, 3), (0, 1));
        g.run_ops(&[Op::Weather { kind: Wk::Rain, intensity: Some(0.8) }]);
        for _ in 0..250 {
            g.tick(0.1);
            g.npc_tick(0.1);
        }
        assert!(g.weather_now().unwrap().is_wet());
        let a = npc_named(&g, "Ann");
        assert!(a.sheltered);
        assert_eq!((a.tile(), a.base), ((1, 3), NpcAction::Idle), "waiting under the shed");
        g.run_ops(&[Op::Weather { kind: Wk::Clear, intensity: Some(0.0) }]);
        for _ in 0..200 {
            g.tick(0.1);
            g.npc_tick(0.1);
        }
        assert!(!npc_named(&g, "Ann").sheltered, "the rain stopped");
        assert!(!g.weather_now().unwrap().is_wet());
    }

    #[test]
    fn a_fight_sends_npcs_to_flee_or_cower_in_place() {
        let ann = rt(vec![st("bench", NpcAction::Sit, 3.0)], None, Some("shed"));
        let bo = rt(vec![st("bench", NpcAction::Sit, 5.0)], None, None);
        let mut g = yard_game(vec![("ann", ann), ("bo", bo)]);
        g.place_at((9, 3), (0, 1));
        run_npcs(&mut g, 0.1);
        g.start_battle("foe");
        assert!(g.battle.is_some());
        run_npcs(&mut g, 0.1);
        assert_eq!(npc_named(&g, "Ann").action, NpcAction::Flee);
        assert_eq!(npc_named(&g, "Bo").action, NpcAction::Cower, "no flee spawn: cowers where it stands");
        let bo_at = npc_named(&g, "Bo").pos;
        run_npcs(&mut g, 15.0);
        let a = npc_named(&g, "Ann");
        assert_eq!((a.tile(), a.action), ((1, 3), NpcAction::Cower), "reached the flee spawn");
        assert_eq!(npc_named(&g, "Bo").pos, bo_at);
        g.battle = None;
        run_npcs(&mut g, 0.1);
        assert_ne!(npc_named(&g, "Ann").action, NpcAction::Cower, "the fight is over");
        assert_ne!(npc_named(&g, "Bo").action, NpcAction::Cower);
    }

    #[test]
    fn chat_partners_walk_to_their_spots_and_face_each_other() {
        let chat = |go: &str, with: &str| {
            rt(vec![RoutineStep { go: Some(go.into()), do_: NpcDo::Chat, secs: 5.0, with: Some(with.into()), radius: 0.0 }], None, None)
        };
        let mut g = yard_game(vec![("ann", chat("prop:4,2", "Bo")), ("bo", chat("prop:5,2", "Ann"))]);
        run_npcs(&mut g, 10.0);
        let (a, b) = (npc_named(&g, "Ann"), npc_named(&g, "Bo"));
        assert_eq!((a.action, a.tile()), (NpcAction::Chat, (4, 2)));
        assert_eq!((b.action, b.tile()), (NpcAction::Chat, (5, 2)));
        assert!(angle_diff(a.yaw, yaw_of(b.pos.0 - a.pos.0, b.pos.1 - a.pos.1)).abs() < 0.1, "Ann faces Bo");
        assert!(angle_diff(b.yaw, yaw_of(a.pos.0 - b.pos.0, a.pos.1 - b.pos.1)).abs() < 0.1, "Bo faces Ann");
    }

    #[test]
    fn a_chat_with_nobody_there_gives_up() {
        let r = rt(vec![RoutineStep { go: Some("bench".into()), do_: NpcDo::Chat, secs: 5.0, with: Some("Bo".into()), radius: 0.0 }], None, None);
        let mut g = yard_game(vec![("ann", r)]);
        g.place_at((1, 3), (0, 1));
        run_npcs(&mut g, 36.0);
        // It waited 30 s, then the step ended and the loop started it again, so it is standing at the bench.
        assert_eq!(npc_named(&g, "Ann").tile(), (7, 1));
    }

    #[test]
    fn e_talks_to_the_nearest_npc_in_front_by_position() {
        let mut g = yard_game(vec![]);
        g.body.pos = (10.0, 5.0);
        g.body.yaw = -std::f32::consts::FRAC_PI_2; // east
        let set = |g: &mut Game, key: &str, p: (f32, f32)| g.npcs.get_mut(key).unwrap().pos = p;
        set(&mut g, "yard@5,1", (12.5, 5.0)); // 2.5 m: out of reach
        set(&mut g, "yard@6,3", (9.2, 5.0)); // close but behind
        g.interact();
        assert!(g.talk.is_none(), "nobody in front within 2 m");
        set(&mut g, "yard@5,1", (11.5, 5.0));
        g.interact();
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("felix"), "Ann, not the closer Bo behind");
        g.talk = None;
        g.talk_npc = None;
        set(&mut g, "yard@6,3", (11.0, 5.0));
        g.interact();
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("thanks"), "Bo is now nearer in front");
        assert_eq!(g.talk_npc.as_deref(), Some("yard@6,3"));
    }

    #[test]
    fn the_body_and_a_walking_npc_are_pushed_apart() {
        let mut g = yard_game(vec![("ann", rt(vec![st("bench", NpcAction::Sit, 3.0)], None, None))]);
        g.body.pos = (11.3, 3.0); // 0.3 m from Ann at (11, 3)
        g.npc_tick(0.1);
        let d = dist2(g.body.pos, npc_named(&g, "Ann").pos);
        assert!(d >= NPC_RADIUS + RADIUS - 1e-3, "kept apart: {d}");
    }

    #[test]
    fn a_still_npc_stays_at_home_and_glances_around() {
        let mut g = Game::new(test_pack()).unwrap();
        let mut looked = false;
        for _ in 0..200 {
            g.npc_tick(0.1);
            looked |= g.npcs["lab@5,1"].action == NpcAction::Look;
        }
        assert_eq!(g.npcs["lab@5,1"].pos, tile_centre(5, 1), "the still NPC stays put");
        assert!(looked, "and now and then looks about");
    }

    #[test]
    fn using_a_seat_seats_the_body_and_a_move_key_stands_it_up() {
        let mut g = yard_game(vec![]);
        g.place_at((2, 1), (1, 0));
        assert_eq!(g.usable_prop_ahead().map(|u| u.text.as_str()), Some("Sit"));
        g.interact();
        assert_eq!(g.seated, Some((7.0, 3.0, 0.45)));
        g.update_body(0.5, &Input::default());
        assert!(g.seated.is_some());
        assert_eq!(g.body.pos, (7.0, 3.0), "does not drift while seated");
        g.update_body(0.1, &Input { fwd: 1.0, ..Default::default() });
        assert!(g.seated.is_none(), "a move key stands up");
        g.place_at((2, 1), (1, 0));
        g.interact();
        assert!(g.seated.is_some());
        g.interact();
        assert!(g.seated.is_none(), "E on a seat stands up too");
        g.place_at((2, 1), (1, 0));
        g.interact();
        assert!(g.seated.is_some());
        g.leave("lab", "start");
        assert!(g.seated.is_none(), "leaving the map stands up");
    }

    #[test]
    fn using_a_work_prop_nudges_the_camera_toasts_and_runs_its_scene() {
        let mut g = yard_game(vec![]);
        g.place_at((2, 2), (1, 0));
        g.interact();
        assert!(matches!(g.use_anim, Some((PropAction::Work, _))));
        assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Hammer"));
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("note"));
        assert!(g.seated.is_none());
        g.talk = None;
        for _ in 0..20 {
            g.tick(0.1);
        }
        assert!(g.use_anim.is_none(), "the nudge ends");
    }

    #[test]
    fn indoor_maps_have_no_weather_and_the_weather_op_does_nothing_there() {
        let mut g = Game::new(test_pack()).unwrap();
        assert!(g.weather.is_empty() && g.weather_now().is_none());
        g.run_ops(&[Op::Weather { kind: Wk::Rain, intensity: Some(1.0) }]);
        g.tick(0.1);
        assert!(g.weather.is_empty() && g.weather_now().is_none());
    }

    #[test]
    fn outdoor_weather_blends_in_and_each_map_keeps_its_own_state() {
        let mut g = yard_game(vec![]);
        assert_eq!(g.weather_now().map(|w| w.kind), Some(Wk::Clear));
        g.run_ops(&[Op::Weather { kind: Wk::Storm, intensity: Some(0.9) }]);
        {
            let w = g.weather_now().unwrap();
            assert_eq!((w.kind, w.target.0, w.blend), (Wk::Clear, Wk::Storm, 0.0), "blending, not jumping");
        }
        for _ in 0..50 {
            g.tick(0.1);
        }
        let w = g.weather_now().unwrap();
        assert!(w.blend > 0.4 && w.blend < 0.8, "part way: {}", w.blend);
        let time = w.time;
        g.leave("lab", "start");
        assert!(g.weather_now().is_none());
        for _ in 0..50 {
            g.tick(0.1);
        }
        g.leave("yard", "start");
        assert_eq!(g.weather_now().unwrap().time, time, "the yard's weather waited for the player");
        assert_eq!(g.weather_now().unwrap().target.0, Wk::Storm);
    }

    #[test]
    fn the_weather_op_on_an_outdoor_map_without_weather_starts_one() {
        let mut pack = yard_pack(vec![]);
        pack.maps.get_mut("yard").unwrap().weather = None;
        let mut g = Game::new(pack).unwrap();
        assert!(g.weather_now().is_none());
        g.run_ops(&[Op::Weather { kind: Wk::Snow, intensity: Some(0.5) }]);
        assert_eq!(g.weather_now().map(|w| w.kind), Some(Wk::Snow));
    }
}
