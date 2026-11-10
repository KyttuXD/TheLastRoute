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
    /// Starts the battle with this id (`{"op": "battle", "battle": "id"}`).
    Battle(String),
    /// Heals the player up to max HP (`{"op": "heal", "amount": N}`).
    Heal(i32),
    /// Sets the current map's weather (`{"op": "weather", "kind": "rain", "intensity"?: 0..1}`). Outdoor maps only;
    /// `intensity` None keeps the default (`DEFAULT_WEATHER_INTENSITY`).
    Weather { kind: WeatherKind, intensity: Option<f32> },
}

/// The weather a map can have (JSON "clear", "rain", "snow", "fog", "storm", "leaves").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WeatherKind {
    Clear,
    Rain,
    Snow,
    Fog,
    Storm,
    Leaves,
}

impl WeatherKind {
    /// The names a pack may use for a weather kind.
    pub const NAMES: [&'static str; 6] = ["clear", "rain", "snow", "fog", "storm", "leaves"];

    pub fn parse(name: &str) -> Result<WeatherKind, String> {
        match name {
            "clear" => Ok(WeatherKind::Clear),
            "rain" => Ok(WeatherKind::Rain),
            "snow" => Ok(WeatherKind::Snow),
            "fog" => Ok(WeatherKind::Fog),
            "storm" => Ok(WeatherKind::Storm),
            "leaves" => Ok(WeatherKind::Leaves),
            other => Err(format!("unknown weather kind '{other}'; use one of {}", WeatherKind::NAMES.join(", "))),
        }
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            WeatherKind::Clear => "clear",
            WeatherKind::Rain => "rain",
            WeatherKind::Snow => "snow",
            WeatherKind::Fog => "fog",
            WeatherKind::Storm => "storm",
            WeatherKind::Leaves => "leaves",
        }
    }
}

/// The intensity a weather entry or op gets when it sets none.
pub const DEFAULT_WEATHER_INTENSITY: f32 = 0.6;
/// Seconds between possible weather changes when a map sets no `every_secs`.
pub const DEFAULT_WEATHER_EVERY_SECS: f32 = 90.0;

/// A map's weather (JSON `"weather"`, outdoor maps only): the starting kind and intensity, and the kinds it may change
/// to with their weights, checked every `every_secs`.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherDef {
    pub kind: WeatherKind,
    /// 0..=1.
    pub intensity: f32,
    /// Kind and weight (above 0) of each possible change. Empty: the weather never changes by itself.
    pub changes: Vec<(WeatherKind, f32)>,
    pub every_secs: f32,
}

/// What a player or NPC does at a usable prop (JSON `"use": {"action"}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropAction {
    Sit,
    Lean,
    Work,
    DrawWater,
    Read,
    Sleep,
    WarmHands,
    Look,
}

impl PropAction {
    /// The names a pack may use for a prop action.
    pub const NAMES: [&'static str; 8] = ["sit", "lean", "work", "draw_water", "read", "sleep", "warm_hands", "look"];

    pub fn parse(name: &str) -> Result<PropAction, String> {
        match name {
            "sit" => Ok(PropAction::Sit),
            "lean" => Ok(PropAction::Lean),
            "work" => Ok(PropAction::Work),
            "draw_water" => Ok(PropAction::DrawWater),
            "read" => Ok(PropAction::Read),
            "sleep" => Ok(PropAction::Sleep),
            "warm_hands" => Ok(PropAction::WarmHands),
            "look" => Ok(PropAction::Look),
            other => Err(format!("unknown prop action '{other}'; use one of {}", PropAction::NAMES.join(", "))),
        }
    }

    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            PropAction::Sit => "sit",
            PropAction::Lean => "lean",
            PropAction::Work => "work",
            PropAction::DrawWater => "draw_water",
            PropAction::Read => "read",
            PropAction::Sleep => "sleep",
            PropAction::WarmHands => "warm_hands",
            PropAction::Look => "look",
        }
    }

    /// The prompt word when the legend gives no `text`.
    pub fn default_text(self) -> &'static str {
        match self {
            PropAction::Sit => "Sit",
            PropAction::Lean => "Lean",
            PropAction::Work => "Work",
            PropAction::DrawWater => "Draw water",
            PropAction::Read => "Read",
            PropAction::Sleep => "Sleep",
            PropAction::WarmHands => "Warm hands",
            PropAction::Look => "Look",
        }
    }
}

/// The seat height (metres) of a sit or sleep prop that sets none.
pub const DEFAULT_SEAT_HEIGHT: f32 = 0.45;
/// The tallest seat height a prop may set.
pub const MAX_SEAT_HEIGHT: f32 = 2.0;

/// A prop the player (and NPCs in their routines) can use: `{"action", "text"?, "scene"?, "seat_height"?}`.
#[derive(Clone, Debug, PartialEq)]
pub struct PropUse {
    pub action: PropAction,
    /// The prompt word ("[E] Sit").
    pub text: String,
    /// A scene run when the PLAYER uses the prop.
    pub scene: Option<String>,
    /// Metres; the camera height when seated or asleep.
    pub seat_height: f32,
}

/// What an NPC does in a routine step (JSON `"do"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcDo {
    Idle,
    Sit,
    Lean,
    Work,
    DrawWater,
    Read,
    Sleep,
    WarmHands,
    Look,
    Wave,
    Chat,
    Wander,
}

impl NpcDo {
    /// The names a pack may use for a routine step action.
    pub const NAMES: [&'static str; 12] =
        ["idle", "sit", "lean", "work", "draw_water", "read", "sleep", "warm_hands", "look", "wave", "chat", "wander"];

    pub fn parse(name: &str) -> Result<NpcDo, String> {
        Ok(match name {
            "idle" => NpcDo::Idle,
            "sit" => NpcDo::Sit,
            "lean" => NpcDo::Lean,
            "work" => NpcDo::Work,
            "draw_water" => NpcDo::DrawWater,
            "read" => NpcDo::Read,
            "sleep" => NpcDo::Sleep,
            "warm_hands" => NpcDo::WarmHands,
            "look" => NpcDo::Look,
            "wave" => NpcDo::Wave,
            "chat" => NpcDo::Chat,
            "wander" => NpcDo::Wander,
            other => return Err(format!("unknown \"do\" action '{other}'; use one of {}", NpcDo::NAMES.join(", "))),
        })
    }
}

/// The radius (metres) of a wander step that sets none.
pub const DEFAULT_WANDER_RADIUS: f32 = 3.0;
/// The seconds of a step that sets none.
pub const DEFAULT_STEP_SECS: f32 = 5.0;

/// One step of an NPC routine. `go` is a spawn name or `prop:<x>,<y>` (None: stay where the NPC is).
#[derive(Clone, Debug, PartialEq)]
pub struct RoutineStep {
    pub go: Option<String>,
    pub do_: NpcDo,
    /// Seconds spent on the action once there (above 0).
    pub secs: f32,
    /// The NPC to talk to, for `Chat`.
    pub with: Option<String>,
    /// Metres, for `Wander`.
    pub radius: f32,
}

/// An NPC's daily routine (`routines/*.json`).
#[derive(Clone, Debug, PartialEq)]
pub struct Routine {
    pub looping: bool,
    pub steps: Vec<RoutineStep>,
    /// Spawn name to run to in rain, snow or a storm.
    pub shelter: Option<String>,
    /// Spawn name to run to in a fight.
    pub flee: Option<String>,
}

/// The tile of a `go` that points at a prop (`prop:<x>,<y>`): None when `go` is a spawn name, an error when the
/// coordinates are malformed.
pub fn parse_prop_target(go: &str) -> Option<Result<(i32, i32), String>> {
    let rest = go.strip_prefix("prop:")?;
    let bad = || format!("'{go}' is not a prop target; write prop:<x>,<y>");
    let Some((x, y)) = rest.split_once(',') else { return Some(Err(bad())) };
    match (x.trim().parse::<i32>(), y.trim().parse::<i32>()) {
        (Ok(x), Ok(y)) => Some(Ok((x, y))),
        _ => Some(Err(bad())),
    }
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
    /// Blocks; press E facing it to play `scene`. Drawn as a mannequin of `body` with its name above it, facing the
    /// player. `skin` names a skin in a pack's skins folder (None: the default atlas).
    /// `routine` names a routine (`routines/*.json`) the NPC follows; None: it stands at home.
    Npc { name: String, scene: String, color: Rgb, body: BodyChoice, skin: Option<String>, routine: Option<String> },
    /// Plays `scene` when the player steps on it (once per tile when `once`). Invisible.
    Trigger { scene: String, once: bool },
    /// Steps the player onto the spawn `spawn` of map `map`. Drawn as a glowing pad.
    Exit { map: String, spawn: String },
    /// A prop model from the pack's `props` folder, standing on the tile centre and turned `yaw` degrees about the
    /// vertical axis. Walkable. `color` is the legend colour: the cube drawn when the model does not load.
    /// `use_` (JSON `"use"`) makes the prop usable by the player and by NPC routines.
    Prop { prop: String, yaw: f32, color: Rgb, use_: Option<PropUse> },
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

/// What using an item does (`"use": {"heal": N}` or `"use": {"damage": N}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemUse {
    Heal(i32),
    Damage(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub description: String,
    /// What using it does, if it can be used.
    pub use_effect: Option<ItemUse>,
    /// Whether using it takes it from the inventory (JSON "consumable"; true by default when "use" is set).
    pub consumable: bool,
    /// How many fit in one inventory slot, 1..=99 (JSON "stack"; 1 by default, 9 when the item has a "use").
    pub stack: u32,
    /// The item's picture: after loading, the full path of a PNG (at most 64x64) in the pack's items/icons/ folder
    /// (JSON "icon" is just the file name).
    pub icon: Option<PathBuf>,
}

impl Item {
    /// A plain item that cannot be used.
    #[cfg(test)]
    pub fn plain(name: &str, description: &str) -> Item {
        Item {
            name: name.to_string(),
            description: description.to_string(),
            use_effect: None,
            consumable: false,
            stack: 1,
            icon: None,
        }
    }
}

/// The numbers a fighter has. Defaults: 30 HP, 5 attack, 2 defense, 5 speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub max_hp: i32,
    pub attack: i32,
    pub defense: i32,
    pub speed: i32,
}

impl Default for Stats {
    fn default() -> Stats {
        Stats { max_hp: 30, attack: 5, defense: 2, speed: 5 }
    }
}

/// How an enemy attack plays out in real time. Written in JSON as "melee", "charge", "shot" or "slam".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackKind {
    /// A short arc in front of the enemy.
    Melee,
    /// A dash along a lane toward where the player was.
    Charge,
    /// A projectile fired at the player.
    Shot,
    /// A circle on the floor at the player's position.
    Slam,
}

impl AttackKind {
    /// The names a pack may use for an attack kind.
    pub const NAMES: [&'static str; 4] = ["melee", "charge", "shot", "slam"];

    pub fn parse(name: &str) -> Result<AttackKind, String> {
        match name {
            "melee" => Ok(AttackKind::Melee),
            "charge" => Ok(AttackKind::Charge),
            "shot" => Ok(AttackKind::Shot),
            "slam" => Ok(AttackKind::Slam),
            other => Err(format!("unknown attack kind '{other}'; use one of {}", AttackKind::NAMES.join(", "))),
        }
    }

    #[cfg(test)]
    pub fn name(self) -> &'static str {
        match self {
            AttackKind::Melee => "melee",
            AttackKind::Charge => "charge",
            AttackKind::Shot => "shot",
            AttackKind::Slam => "slam",
        }
    }
}

/// One attack an enemy can pick. `weight` is how likely it is among the attacks in range and off cooldown.
/// `range` is in metres; `windup` is the seconds of floor telegraph before it lands; `radius` is the Slam area or the
/// Shot projectile size; `speed` is metres per second for Shot and Charge; `min_phase` is the boss phase from which
/// the attack is allowed (0: from the start). `text` is an optional line shown when it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct EnemyAttack {
    pub name: String,
    pub kind: AttackKind,
    pub damage: i32,
    pub range: f32,
    pub windup: f32,
    pub cooldown: f32,
    pub weight: u32,
    pub min_phase: usize,
    pub radius: f32,
    pub speed: f32,
    pub text: String,
}

/// A real-time fight. The enemy appears at the map spawn `spawn`. `phases` are HP percent marks (for example
/// [66, 33], highest first); each mark passed raises the phase. `win` and `lose` are scene ids started after the fight.
/// `skin` names a skin in a pack's skins folder for the enemy's figure.
#[derive(Clone, Debug, PartialEq)]
pub struct Battle {
    pub id: String,
    pub name: String,
    pub boss: bool,
    pub max_hp: i32,
    pub defense: i32,
    pub move_speed: f32,
    pub skin: Option<String>,
    pub spawn: String,
    pub attacks: Vec<EnemyAttack>,
    pub phases: Vec<i32>,
    pub intro: String,
    pub win: Option<String>,
    pub lose: Option<String>,
}

/// One step of a quest: its text and the condition that marks it done.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestStep {
    pub text: String,
    pub done_if: Cond,
}

/// A journal entry. Shown once `show_if` holds (always when None) or any step is done.
#[derive(Clone, Debug, PartialEq)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub description: String,
    pub show_if: Option<Cond>,
    pub steps: Vec<QuestStep>,
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
    /// Spawn name to the body yaw (radians) on arriving there, from the legend spawn's optional "facing". A spawn
    /// without one keeps the body's current facing (the story start faces south).
    pub spawn_facing: BTreeMap<String, f32>,
    /// Open air (map JSON `"outdoor"`, default false): no ceiling, a sky instead.
    pub outdoor: bool,
    /// Ceiling height in metres (JSON `"ceiling": {"height"}`, 1..=16). Default: the tallest wall in the legend,
    /// else 3.0. Unused on outdoor maps.
    pub ceiling_height: f32,
    /// Ceiling colour (JSON `"ceiling": {"color"}`); None lets the painter pick its own ceiling texture.
    pub ceiling_color: Option<Rgb>,
    /// Sky gradient (top, bottom) of an outdoor map (JSON `"sky"`, default a dusk gradient); None indoors.
    pub sky: Option<(Rgb, Rgb)>,
    /// Wall look, one of `WALL_STYLES` (JSON `"walls"`, default "lab").
    pub wall_style: String,
    /// Floor look, one of `FLOOR_STYLES` (JSON `"floor"`, default "blocks").
    pub floor_style: String,
    /// The weather (JSON `"weather"`); only outdoor maps have one. None: no weather system on this map.
    pub weather: Option<WeatherDef>,
}

/// The wall styles a map may name. The first is the default.
pub const WALL_STYLES: [&str; 6] = ["lab", "brick", "concrete", "plaster", "timber", "stone"];
/// The floor styles a map may name. The first is the default (the 1 m blocks).
pub const FLOOR_STYLES: [&str; 6] = ["blocks", "asphalt", "paving", "tiles", "cobble", "dirt"];
/// The sky of an outdoor map that sets none: a dusk gradient, top then bottom.
pub const DEFAULT_SKY: (Rgb, Rgb) = ([0x1c, 0x24, 0x4a], [0xe0, 0x8a, 0x5a]);
/// The ceiling height when a map sets none and has no walls in its legend.
pub const DEFAULT_CEILING_HEIGHT: f32 = 3.0;

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

/// Where a story sits in the world's timeline. The menu shows it; the rules do not depend on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    /// The core pack's own sample story.
    Sample,
    Past,
    LastRoute,
    Simulora,
    Future,
}

impl Era {
    /// The names a pack.json may use for an era.
    pub const NAMES: [&'static str; 5] = ["sample", "past", "lastroute", "simulora", "future"];

    pub fn parse(name: &str) -> Option<Era> {
        Some(match name {
            "sample" => Era::Sample,
            "past" => Era::Past,
            "lastroute" => Era::LastRoute,
            "simulora" => Era::Simulora,
            "future" => Era::Future,
            _ => return None,
        })
    }

    /// The name the menu shows.
    pub fn label(self) -> &'static str {
        match self {
            Era::Sample => "Sample",
            Era::Past => "Past",
            Era::LastRoute => "LastRoute",
            Era::Simulora => "Simulora",
            Era::Future => "Future",
        }
    }
}

/// The mannequin body an NPC or a story's player is drawn with. Written in JSON as `"blocky"` (the default) or
/// `"blocky_slim"` (the slim-arm variant).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BodyChoice {
    #[default]
    Blocky,
    BlockySlim,
}

impl BodyChoice {
    /// The names a pack may use for a body.
    pub const NAMES: [&'static str; 2] = ["blocky", "blocky_slim"];

    /// The body a pack's name stands for. An unknown name is an error that lists the names.
    pub fn parse(name: &str) -> Result<BodyChoice, String> {
        match name {
            "blocky" => Ok(BodyChoice::Blocky),
            "blocky_slim" => Ok(BodyChoice::BlockySlim),
            other => Err(format!("unknown body '{other}'; use one of {}", BodyChoice::NAMES.join(", "))),
        }
    }

    /// The name a pack uses for this body.
    pub fn name(self) -> &'static str {
        match self {
            BodyChoice::Blocky => "blocky",
            BodyChoice::BlockySlim => "blocky_slim",
        }
    }

    /// Whether this body uses the slim-arm rig.
    pub fn slim(self) -> bool {
        self == BodyChoice::BlockySlim
    }
}

/// The player character a story starts with: the name and role shown in the menu and on speaker "You", the speed that
/// scales walking and sprinting (1.0 is normal), the body drawn in the third-person view, and the items and flags the
/// story begins with.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub name: String,
    pub role: String,
    pub speed: f32,
    pub body: BodyChoice,
    pub start_items: Vec<String>,
    pub start_flags: Vec<String>,
    /// Fighting stats (story block `"player": {"stats": {...}}`).
    pub stats: Stats,
}

impl Default for Player {
    fn default() -> Player {
        Player {
            name: "You".to_string(),
            role: String::new(),
            speed: 1.0,
            body: BodyChoice::default(),
            start_items: Vec::new(),
            start_flags: Vec::new(),
            stats: Stats::default(),
        }
    }
}

#[cfg(test)]
mod attack_tests {
    use super::*;

    #[test]
    fn attack_kinds_round_trip_and_unknown_names_list_the_choices() {
        for name in AttackKind::NAMES {
            assert_eq!(AttackKind::parse(name).unwrap().name(), name);
        }
        let e = AttackKind::parse("laser").unwrap_err();
        assert!(e.contains("unknown attack kind 'laser'") && e.contains("melee, charge, shot, slam"), "{e}");
        assert_eq!(Stats::default(), Stats { max_hp: 30, attack: 5, defense: 2, speed: 5 });
    }
}

#[cfg(test)]
mod life_tests {
    use super::*;

    #[test]
    fn weather_prop_and_npc_action_names_round_trip_and_unknown_names_list_the_choices() {
        for name in WeatherKind::NAMES {
            assert_eq!(WeatherKind::parse(name).unwrap().name(), name);
        }
        for name in PropAction::NAMES {
            assert_eq!(PropAction::parse(name).unwrap().name(), name);
        }
        for name in NpcDo::NAMES {
            assert!(NpcDo::parse(name).is_ok(), "{name}");
        }
        let e = WeatherKind::parse("hail").unwrap_err();
        assert!(e.contains("unknown weather kind 'hail'") && e.contains("clear, rain, snow, fog, storm, leaves"), "{e}");
        let e = PropAction::parse("dance").unwrap_err();
        assert!(e.contains("unknown prop action 'dance'") && e.contains("draw_water"), "{e}");
        assert!(NpcDo::parse("dance").unwrap_err().contains("wander"));
    }

    #[test]
    fn prop_actions_have_default_prompt_words() {
        assert_eq!(PropAction::Sit.default_text(), "Sit");
        assert_eq!(PropAction::DrawWater.default_text(), "Draw water");
        assert_eq!(PropAction::WarmHands.default_text(), "Warm hands");
    }

    #[test]
    fn prop_targets_parse_tile_coordinates() {
        assert_eq!(parse_prop_target("hearth"), None);
        assert_eq!(parse_prop_target("prop:12,3"), Some(Ok((12, 3))));
        assert_eq!(parse_prop_target("prop: -1 , 4"), Some(Ok((-1, 4))));
        assert!(matches!(parse_prop_target("prop:12"), Some(Err(_))));
        assert!(matches!(parse_prop_target("prop:a,b"), Some(Err(_))));
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;

    #[test]
    fn body_names_parse_and_the_default_is_blocky() {
        assert_eq!(BodyChoice::parse("blocky"), Ok(BodyChoice::Blocky));
        assert_eq!(BodyChoice::parse("blocky_slim"), Ok(BodyChoice::BlockySlim));
        assert_eq!(BodyChoice::default(), BodyChoice::Blocky);
        assert_eq!(Player::default().body, BodyChoice::Blocky);
        for body in [BodyChoice::Blocky, BodyChoice::BlockySlim] {
            assert_eq!(BodyChoice::parse(body.name()), Ok(body), "{} round-trips", body.name());
        }
        assert!(!BodyChoice::Blocky.slim() && BodyChoice::BlockySlim.slim());
    }

    #[test]
    fn an_unknown_body_name_is_an_error_that_lists_the_names() {
        let e = BodyChoice::parse("robot").unwrap_err();
        assert!(e.contains("unknown body 'robot'"), "{e}");
        assert!(e.contains("blocky, blocky_slim"), "{e}");
        assert!(BodyChoice::parse("Blocky").is_err(), "names are case-sensitive");
    }
}

/// One story a pack offers. Its id is the pack id. A story starts at its own start point, not at the pack default.
#[derive(Clone, Debug, PartialEq)]
pub struct StoryInfo {
    pub id: String,
    pub title: String,
    pub era: Era,
    /// Sort key in the menu (lower first); equal orders sort by title.
    pub order: f64,
    pub start_map: String,
    pub start_spawn: String,
    pub start_scene: Option<String>,
    pub player: Player,
    /// Rooms may play this story (`"multiplayer": true` in its block). The story server enforces it too.
    pub multiplayer: bool,
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub title: String,
    /// The core default start. A story has its own start (see `stories`); this one is what a pack without a story
    /// block, or a pack that overrides "start", sets.
    pub start_map: String,
    pub start_spawn: String,
    pub start_scene: Option<String>,
    pub items: BTreeMap<String, Item>,
    pub scenes: BTreeMap<String, Scene>,
    pub battles: BTreeMap<String, Battle>,
    pub quests: BTreeMap<String, Quest>,
    /// Routine id to its routine, from every pack's `routines` folder.
    pub routines: BTreeMap<String, Routine>,
    pub maps: BTreeMap<String, MapDef>,
    /// Prop id to its `.bbmodel` file, from every pack's `props` folder (the file's name without `.bbmodel`).
    pub props: BTreeMap<String, PathBuf>,
    /// Skin name to its mannequin atlas PNG, from every pack's `skins` folder (the file's name without `.png`). An NPC
    /// names one with its "skin" field.
    pub skins: BTreeMap<String, PathBuf>,
    pub packs: Vec<PackInfo>,
    /// Every story the loaded packs offer, in load order (the core pack's sample story first).
    pub stories: Vec<StoryInfo>,
}

impl Pack {
    /// The story with this id, if a loaded pack offers it.
    pub fn story(&self, id: &str) -> Option<&StoryInfo> {
        self.stories.iter().find(|s| s.id == id)
    }

    /// The stories in menu order: by `order`, then by title.
    pub fn stories_in_menu_order(&self) -> Vec<&StoryInfo> {
        let mut out: Vec<&StoryInfo> = self.stories.iter().collect();
        out.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.title.cmp(&b.title)));
        out
    }
}
