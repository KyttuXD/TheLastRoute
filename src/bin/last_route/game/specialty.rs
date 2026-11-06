//! The hunter specialty bar: four slots numbered 6 to 9, SEPARATE from the five common hotbar slots (`HOTBAR_SIZE`).
//! Each slot belongs to hunters (data/ui/specialty_slots.json, `lr.specialty_slots/1`). A slot is visible while one of
//! its hunters exists IN THE RUN (the drawn floor's list, every resident floor's list and the shelves, `run_hunter_ids`),
//! whatever that hunter's state or floor: Solyth is built into the lineup of the floors below and above level 0, so the
//! item must already be in hand on level 0, where she arrives by the stairs. The slot's item is granted the moment the slot
//! becomes visible and REMOVED (not returned to the pack) the moment it stops being visible.
//!
//! - Slot 6, the music box (Glyra). Using it winds the box (game/music_box.rs) and spends it. A box picked up off the
//!   floor goes into this slot while Glyra is out, and into the pack otherwise (game/items.rs `grant`). When the slot is
//!   granted, a box already in the pack folds into it, so the runner never holds two.
//! - Slot 7, the temperature item. ONE item for both Wynelle (cold) and Solyth (heat). Its type is heat or cold, toggled
//!   by pressing 7 a second time or by using it; heat deters Wynelle and cold deters Solyth. The effect is real:
//!   `Specialty::felt_local` moves the body temperature the meters chase, a heat source counts as a heater for Wynelle
//!   (hunters/gimmicks/wynelle.rs reads `heater_on` and `heater_thawed`), and a cold source slows Solyth and keeps the
//!   runner under her pounce heat (game/hunting.rs). The numbers are data.
//! - Slots 8 and 9 are reserved: drawn empty, never hold an item.

use std::sync::OnceLock;

use macroquad::prelude::KeyCode;
use serde_json::Value;

use super::{Game, Item};
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Gimmick;

const FILE: &str = "ui/specialty_slots.json";
const SCHEMA: &str = "lr.specialty_slots/1";

/// Slots on the specialty bar (numbers 6 to 9).
pub const SLOT_COUNT: usize = 4;
/// The number of the first specialty slot; the common hotbar is 1 to `HOTBAR_SIZE`.
pub const FIRST_NUMBER: u8 = 6;

/// What a specialty slot carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    MusicBox,
    Temperature,
    Empty,
}

/// The temperature item's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Heat,
    Cold,
}

impl Mode {
    pub fn other(self) -> Mode {
        match self {
            Mode::Heat => Mode::Cold,
            Mode::Cold => Mode::Heat,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Heat => "HEAT",
            Mode::Cold => "COLD",
        }
    }
}

/// An item that lives in a specialty slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecItem {
    MusicBox,
    Temperature(Mode),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotDef {
    pub id: String,
    pub number: u8,
    /// A macroquad key code name, `Key6` to `Key9`.
    pub key: String,
    pub label: String,
    pub kind: Kind,
    /// Hunter ids that make this slot visible.
    pub hunters: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeatRules {
    /// Degrees the heat source adds to the temperature the body chases.
    pub target_offset: f32,
    /// The warmest the source takes the body target (below the overheating line, never a latch).
    pub cap: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColdRules {
    /// Degrees the cold source takes off the temperature the body chases.
    pub target_offset: f32,
    /// The coldest the source takes the body target (above the freezing line).
    pub floor: f32,
    /// Speed factor on Solyth while the cold source is on.
    pub solyth_slow_factor: f32,
    /// The temperature the hunters read on a runner carrying the cold source, at most (Solyth's pounce gate is 60).
    pub solyth_pounce_reading: f32,
    /// Seconds Solyth keeps backing off after a frame with the cold source on (she is out of the chase, cannot grab).
    pub solyth_deter_seconds: f32,
    /// How far (cells) from the runner she retreats when the cold source drives her off.
    pub solyth_retreat_cells: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub slots: Vec<SlotDef>,
    pub default_mode: Mode,
    pub heat: HeatRules,
    pub cold: ColdRules,
}

/// The shipped rules. A missing or broken file is a build defect (the file is embedded), so this panics with the reason.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(FILE)
            .next()
            .map(|x| x.1)
            .unwrap_or_else(|| panic!("data/{FILE} is missing from the build"));
        parse(text).unwrap_or_else(|e| panic!("data/{FILE}: {e}"))
    })
}

fn num(v: &Value, path: &str) -> Result<f32, String> {
    let mut at = v;
    for k in path.split('.') {
        at = &at[k];
    }
    let x = at.as_f64().ok_or_else(|| format!("{path} is not a number"))?;
    if !x.is_finite() {
        return Err(format!("{path} is not finite"));
    }
    Ok(x as f32)
}

fn mode_of(s: &str) -> Option<Mode> {
    match s {
        "heat" => Some(Mode::Heat),
        "cold" => Some(Mode::Cold),
        _ => None,
    }
}

/// Reads and validates the file text. Every range is checked: nothing is silently defaulted.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if v["schema"] != SCHEMA {
        return Err(format!("schema must be {SCHEMA}"));
    }
    let list = v["slots"].as_array().ok_or("slots is not a list")?;
    if list.len() != SLOT_COUNT {
        return Err(format!("slots must list exactly {SLOT_COUNT} slots (numbers 6 to 9)"));
    }
    let mut slots: Vec<SlotDef> = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let want = FIRST_NUMBER + i as u8;
        let number = s["number"].as_u64().ok_or_else(|| format!("slots[{i}].number is not a number"))?;
        if number != want as u64 {
            return Err(format!("slots[{i}].number must be {want} (slots run 6, 7, 8, 9 in order)"));
        }
        let id = s["id"].as_str().filter(|x| !x.is_empty()).ok_or_else(|| format!("slots[{i}].id is missing"))?;
        if slots.iter().any(|o| o.id == id) {
            return Err(format!("slot id {id} is used twice"));
        }
        let key = s["key"].as_str().ok_or_else(|| format!("slots[{i}].key is missing"))?;
        if key_code(key).is_none() {
            return Err(format!("slots[{i}].key {key} is not one of Key6, Key7, Key8, Key9"));
        }
        if slots.iter().any(|o| o.key == key) {
            return Err(format!("key {key} is bound twice"));
        }
        let kind = match s["kind"].as_str() {
            Some("music_box") => Kind::MusicBox,
            Some("temperature") => Kind::Temperature,
            Some("empty") => Kind::Empty,
            other => return Err(format!("slots[{i}].kind {other:?} is not music_box, temperature or empty")),
        };
        if slots.iter().any(|o| o.kind == kind && kind != Kind::Empty) {
            return Err(format!("kind of slots[{i}] is used twice"));
        }
        let hunters: Vec<String> = s["hunters"]
            .as_array()
            .ok_or_else(|| format!("slots[{i}].hunters is not a list"))?
            .iter()
            .map(|h| h.as_str().filter(|x| !x.is_empty()).map(str::to_string).ok_or_else(|| format!("slots[{i}].hunters has a bad id")))
            .collect::<Result<_, _>>()?;
        match kind {
            Kind::Empty if !hunters.is_empty() => return Err(format!("slots[{i}] is empty and cannot own hunters")),
            Kind::MusicBox | Kind::Temperature if hunters.is_empty() => {
                return Err(format!("slots[{i}] needs at least one owning hunter"))
            }
            _ => {}
        }
        slots.push(SlotDef {
            id: id.to_string(),
            number: number as u8,
            key: key.to_string(),
            label: s["label"].as_str().unwrap_or("").to_string(),
            kind,
            hunters,
        });
    }
    let t = &v["temperature"];
    let default_mode = t["default_mode"]
        .as_str()
        .and_then(mode_of)
        .ok_or("temperature.default_mode must be heat or cold")?;
    let heat = HeatRules {
        target_offset: num(&v, "temperature.heat.target_offset")?,
        cap: num(&v, "temperature.heat.cap")?,
    };
    let cold = ColdRules {
        target_offset: num(&v, "temperature.cold.target_offset")?,
        floor: num(&v, "temperature.cold.floor")?,
        solyth_slow_factor: num(&v, "temperature.cold.solyth_slow_factor")?,
        solyth_pounce_reading: num(&v, "temperature.cold.solyth_pounce_reading")?,
        solyth_deter_seconds: num(&v, "temperature.cold.solyth_deter_seconds")?,
        solyth_retreat_cells: num(&v, "temperature.cold.solyth_retreat_cells")?,
    };
    if !(0.0..=60.0).contains(&heat.target_offset) || heat.target_offset <= 0.0 {
        return Err("temperature.heat.target_offset must be in (0, 60]".into());
    }
    // a heat source must never reach the 100 latch (Solyth) or the 90 overheating line
    if !(60.0..90.0).contains(&heat.cap) {
        return Err("temperature.heat.cap must be at least 60 and under 90".into());
    }
    if !(0.0..=60.0).contains(&cold.target_offset) || cold.target_offset <= 0.0 {
        return Err("temperature.cold.target_offset must be in (0, 60]".into());
    }
    // a cold source must never reach the freezing line (10)
    if !(10.0..60.0).contains(&cold.floor) || cold.floor <= 10.0 {
        return Err("temperature.cold.floor must be above 10 and under 60".into());
    }
    if !(cold.solyth_slow_factor > 0.0 && cold.solyth_slow_factor <= 1.0) {
        return Err("temperature.cold.solyth_slow_factor must be in (0, 1]".into());
    }
    if !(0.0..60.0).contains(&cold.solyth_pounce_reading) {
        return Err("temperature.cold.solyth_pounce_reading must be under 60 (her pounce gate)".into());
    }
    if !(0.1..=10.0).contains(&cold.solyth_deter_seconds) {
        return Err("temperature.cold.solyth_deter_seconds must be in [0.1, 10]".into());
    }
    if !(4.0..=60.0).contains(&cold.solyth_retreat_cells) {
        return Err("temperature.cold.solyth_retreat_cells must be in [4, 60]".into());
    }
    Ok(Rules { slots, default_mode, heat, cold })
}

/// The key code named in the data file (digit row; the browser build delivers Digit6 to Digit9 as these).
pub fn key_code(name: &str) -> Option<KeyCode> {
    match name {
        "Key6" => Some(KeyCode::Key6),
        "Key7" => Some(KeyCode::Key7),
        "Key8" => Some(KeyCode::Key8),
        "Key9" => Some(KeyCode::Key9),
        _ => None,
    }
}

/// What a sync changed, for the game to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The slot became visible and received its item.
    Granted(Kind),
    /// The slot stopped being visible and lost its item.
    Removed(Kind),
}

/// What pressing a slot's key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Ignored,
    Selected,
    Toggled(Mode),
}

/// What using the selected slot did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Nothing,
    /// The music box left the slot; the game winds it.
    WindBox,
    Toggled(Mode),
}

/// The bar's state: which slots show, what they hold, and which one is selected.
#[derive(Clone, Debug)]
pub struct Specialty {
    pub items: [Option<SpecItem>; SLOT_COUNT],
    pub visible: [bool; SLOT_COUNT],
    /// The selected specialty slot (0 to 3), or None when the common hotbar slot is the one in hand.
    pub selected: Option<usize>,
}

impl Default for Specialty {
    fn default() -> Self {
        let mut visible = [false; SLOT_COUNT];
        for (i, s) in rules().slots.iter().enumerate() {
            visible[i] = s.kind == Kind::Empty;
        }
        Specialty { items: [None; SLOT_COUNT], visible, selected: None }
    }
}

impl Specialty {
    /// The slot index (0 to 3) for a kind.
    pub fn index_of(kind: Kind) -> Option<usize> {
        rules().slots.iter().position(|s| s.kind == kind)
    }

    /// Recomputes which slots show from the hunter ids on the floor, granting and removing items. `ids` is every hunter
    /// that exists on the current floor, active or not.
    pub fn sync(&mut self, ids: &[&str]) -> Vec<Change> {
        let r = rules();
        let mut out = Vec::new();
        for (i, def) in r.slots.iter().enumerate() {
            let now = match def.kind {
                Kind::Empty => true,
                _ => def.hunters.iter().any(|h| ids.contains(&h.as_str())),
            };
            let was = self.visible[i];
            self.visible[i] = now;
            if now && !was && def.kind != Kind::Empty {
                self.items[i] = Some(match def.kind {
                    Kind::MusicBox => SpecItem::MusicBox,
                    _ => {
                        // the counter for the hunter that is out; both out (or none known) takes the data default
                        let wynelle = ids.contains(&"wynelle");
                        let solyth = ids.contains(&"solyth");
                        SpecItem::Temperature(match (wynelle, solyth) {
                            (true, false) => Mode::Heat,
                            (false, true) => Mode::Cold,
                            _ => r.default_mode,
                        })
                    }
                });
                out.push(Change::Granted(def.kind));
            } else if !now && was {
                if def.kind != Kind::Empty {
                    out.push(Change::Removed(def.kind));
                }
                self.items[i] = None;
            }
            // an empty slot never holds anything
            if def.kind == Kind::Empty {
                self.items[i] = None;
            }
        }
        if let Some(s) = self.selected {
            if !self.visible[s] {
                self.selected = None;
            }
        }
        out
    }

    /// True when the box slot shows (Glyra is out): a floor box goes here, not into the pack.
    pub fn box_slot_visible(&self) -> bool {
        Specialty::index_of(Kind::MusicBox).is_some_and(|i| self.visible[i])
    }

    /// True when the box slot shows and is empty.
    pub fn box_slot_free(&self) -> bool {
        Specialty::index_of(Kind::MusicBox).is_some_and(|i| self.visible[i] && self.items[i].is_none())
    }

    /// Puts a box into the box slot. False when the slot is hidden or already holds one.
    pub fn put_box(&mut self) -> bool {
        match Specialty::index_of(Kind::MusicBox) {
            Some(i) if self.visible[i] && self.items[i].is_none() => {
                self.items[i] = Some(SpecItem::MusicBox);
                true
            }
            _ => false,
        }
    }

    /// The item in the selected specialty slot.
    pub fn held(&self) -> Option<SpecItem> {
        self.selected.and_then(|i| self.items[i])
    }

    /// The temperature item's type, when the runner carries it.
    pub fn temp_mode(&self) -> Option<Mode> {
        Specialty::index_of(Kind::Temperature).and_then(|i| match self.items[i] {
            Some(SpecItem::Temperature(m)) => Some(m),
            _ => None,
        })
    }

    pub fn heat_on(&self) -> bool {
        self.temp_mode() == Some(Mode::Heat)
    }

    pub fn cold_on(&self) -> bool {
        self.temp_mode() == Some(Mode::Cold)
    }

    fn toggle_temp(&mut self) -> Option<Mode> {
        let i = Specialty::index_of(Kind::Temperature)?;
        match self.items[i] {
            Some(SpecItem::Temperature(m)) => {
                self.items[i] = Some(SpecItem::Temperature(m.other()));
                Some(m.other())
            }
            _ => None,
        }
    }

    /// The key for slot `i` was pressed. A hidden slot ignores it. The first press selects the slot; pressing the
    /// selected temperature slot again flips heat and cold.
    pub fn press(&mut self, i: usize) -> Press {
        if i >= SLOT_COUNT || !self.visible[i] {
            return Press::Ignored;
        }
        if self.selected == Some(i) {
            if let Some(SpecItem::Temperature(_)) = self.items[i] {
                return self.toggle_temp().map_or(Press::Ignored, Press::Toggled);
            }
        }
        self.selected = Some(i);
        Press::Selected
    }

    /// The use button with a specialty slot selected: the box is spent (the game winds it), the temperature item flips.
    pub fn use_selected(&mut self) -> Use {
        let Some(i) = self.selected else { return Use::Nothing };
        match self.items[i] {
            Some(SpecItem::MusicBox) => {
                self.items[i] = None;
                Use::WindBox
            }
            Some(SpecItem::Temperature(_)) => self.toggle_temp().map_or(Use::Nothing, Use::Toggled),
            None => Use::Nothing,
        }
    }

    /// The climate offset the meters feel, with the temperature item's source on top of the room's own: `ambient` is the
    /// facility's current ambient and `local` the room's offset. Heat never lowers the natural target and cold never
    /// raises it, so a source only ever pushes its own way, and the rules' cap and floor bound it.
    pub fn felt_local(&self, ambient: f32, local: f32) -> f32 {
        let r = rules();
        let natural = ambient + local;
        let target = match self.temp_mode() {
            Some(Mode::Heat) => natural.max((natural + r.heat.target_offset).min(r.heat.cap)),
            Some(Mode::Cold) => natural.min((natural - r.cold.target_offset).max(r.cold.floor)),
            None => natural,
        };
        target - ambient
    }

    /// A cold source deters Solyth while she is out (an active brain with her gimmick): she is slowed, drops the chase,
    /// backs off to a point away from the runner and cannot grab until the source is off for `solyth_deter_seconds`.
    /// Called every hunter frame; it lapses a moment after the source is switched to heat or is gone.
    pub fn deter(&self, b: &mut Brain) {
        if !(self.cold_on() && b.active) {
            return;
        }
        let Gimmick::Solyth(s) = &mut b.gimmick else {
            return;
        };
        s.deter(rules().cold.solyth_deter_seconds, rules().cold.solyth_retreat_cells);
        b.apply_slow(0.5, rules().cold.solyth_slow_factor);
    }

    /// The temperature the hunters read on the runner: a cold source holds it under Solyth's pounce gate.
    pub fn reading_for_hunters(&self, temperature: f32) -> f32 {
        if self.cold_on() {
            temperature.min(rules().cold.solyth_pounce_reading)
        } else {
            temperature
        }
    }
}

impl Game {
    /// The identity of every hunter that exists in the run, wherever it stands: the drawn floor's list, the lists of the
    /// other resident floors, and the shelves (level-0 hunters outside the window, the floors the runner left). The
    /// specialty slots follow this, not the drawn floor alone: Solyth lives in the lineup of a floor below or above and
    /// reaches level 0 by the stairs, so level 0 needs the temperature item though its own list holds neither of the pair.
    fn run_hunter_ids<'a>(
        hunters: &'a [Brain],
        resident: &'a super::resident_hunters::ResidentHunters,
        inf: Option<&'a super::endless::InfState>,
    ) -> Vec<&'a str> {
        hunters
            .iter()
            .chain(resident.iter_all())
            .chain(inf.into_iter().flat_map(|i| i.shelved_hunters()))
            .map(|h| h.def.id.as_str())
            .collect()
    }

    /// Keeps the specialty bar in step with the hunters of the run: shows or hides slots, grants an item the moment a
    /// slot appears and removes it the moment the slot goes. A music box already in the pack folds into the new slot 6.
    pub(super) fn sync_specialty(&mut self) {
        let ids = Self::run_hunter_ids(&self.hunters, &self.resident_hunters, self.inf.as_ref());
        let changes = self.specialty.sync(&ids);
        for c in changes {
            match c {
                Change::Granted(Kind::MusicBox) => {
                    // one box in all: a pack box folds into the slot instead of a second one being minted
                    if let Some(i) = self.slots.iter().position(|s| s.is_some_and(|s| s.item == Item::MusicBox)) {
                        self.slots[i] = None;
                    }
                    self.say("MUSIC BOX // SLOT 6");
                }
                Change::Granted(Kind::Temperature) => {
                    let m = self.specialty.temp_mode().map_or("", Mode::label);
                    self.say(&format!("TEMPERATURE ITEM // SLOT 7 // {m}"));
                }
                _ => {}
            }
        }
    }

    /// A specialty key (index 0 to 3 for keys 6 to 9).
    pub(super) fn press_specialty(&mut self, i: usize) {
        match self.specialty.press(i) {
            Press::Ignored => {}
            Press::Selected => {
                // holding something else puts the flashlight away, as picking any other hotbar item does
                if self.p.flashlight_on {
                    self.p.flashlight_on = false;
                }
            }
            Press::Toggled(m) => self.say_temperature(m),
        }
    }

    /// The use button while a specialty slot is selected.
    pub(super) fn use_specialty(&mut self) {
        match self.specialty.use_selected() {
            Use::Nothing => {}
            Use::WindBox => self.wind_music_box(),
            Use::Toggled(m) => self.say_temperature(m),
        }
    }

    fn say_temperature(&mut self, m: Mode) {
        crate::audio::sound::menu_blip();
        let hunter = match m {
            Mode::Heat => "WYNELLE",
            Mode::Cold => "SOLYTH",
        };
        self.say(&format!("TEMPERATURE // {} // DETERS {hunter}", m.label()));
    }

    /// A heat source counts as a running, thawed heater to Wynelle (she cannot gather, and a materialised one thaws away).
    /// It is the ONLY heater there is: the heater stations are retired (archive/heater-stations/). Void Orange's climate
    /// outage switches it off for the length of the outage, as it used to switch the station off.
    pub(super) fn heat_source_for_hunters(&self) -> bool {
        self.specialty.heat_on()
            && !self
                .void_fx
                .offline(crate::hunters::gimmicks::void::void_orange::System::Climate)
    }

    /// The room climate offset with the temperature item's source on it (what the meters chase).
    pub(super) fn felt_climate(&self) -> f32 {
        self.specialty.felt_local(self.meters.ambient, self.local_climate())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Slot;
    use crate::hunters::brain::Brain;
    use crate::world::catalog::Catalog;

    fn sim() -> (Game, Content) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "SPEC-7", 0, &content, &catalog);
        empty_run(&mut g);
        (g, content)
    }

    /// No hunter anywhere in the run: the drawn list, every resident list and both shelves.
    fn empty_run(g: &mut Game) {
        g.hunters.clear();
        for l in g.resident_hunters.levels() {
            g.resident_hunters.list_mut(l).clear();
        }
        if let Some(inf) = g.inf.as_mut() {
            inf.loaded_mut().clear();
            inf.stored_mut().clear();
            inf.floor_hunters_mut().clear();
        }
    }

    /// A level-0 window with level -1 resident beside it (its lineup as built: Nyxie, Solyth and the route hunters).
    fn stacked_sim() -> (Game, Content) {
        use crate::game::player::stair_height_at;
        use crate::world::facility::LinkKind;
        let (content, catalog) = (Content::load(), Catalog::load());
        for n in 0..60 {
            let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-WALK-{n}"), 0, &content, &catalog);
            let Some(s) = g.fac.stairs.iter().copied().find(|s| s.dir < 0 && matches!(s.kind, LinkKind::Stairs)) else {
                continue;
            };
            let path = s.walk_path(g.fac.level, 0.25);
            let p = path[path.len() / 4];
            g.p.pos = p;
            let Some(h) = stair_height_at(&g.fac, p) else { continue };
            g.p.height = h;
            g.keep_stack(&content, &catalog);
            g.keep_stack(&content, &catalog);
            if g.inf.as_ref().unwrap().stack.has(-1) {
                return (g, content);
            }
        }
        panic!("no test window has a flight going down with its neighbour resident");
    }

    /// A brain of `id` for a resident list of `level`, at the runner's x and z.
    fn slot_of(g: &Game, content: &Content, id: &str, level: i32) -> crate::game::resident_hunters::Slot {
        let def = content.hunter(id).unwrap_or_else(|| panic!("no hunter {id}")).clone();
        let mut brain = Brain::new(def, g.p.pos, &format!("SPEC:{id}:L{level}"), g.difficulty);
        brain.level = level;
        crate::game::resident_hunters::Slot { id: String::new(), brain }
    }

    fn add(g: &mut Game, content: &Content, id: &str) {
        let def = content.hunter(id).unwrap_or_else(|| panic!("no hunter {id}")).clone();
        let at = g.p.pos;
        g.hunters.push(Brain::new(def, at, &format!("SPEC:{id}"), g.difficulty));
    }

    fn drop_hunter(g: &mut Game, id: &str) {
        g.hunters.retain(|h| h.def.id != id);
    }

    fn box_in_pack(g: &Game) -> usize {
        g.slots.iter().flatten().filter(|s| s.item == Item::MusicBox).count()
    }

    #[test]
    fn the_shipped_file_parses_and_lists_slots_six_to_nine() {
        let r = rules();
        let numbers: Vec<u8> = r.slots.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![6, 7, 8, 9]);
        assert_eq!(r.slots[0].kind, Kind::MusicBox);
        assert_eq!(r.slots[0].hunters, vec!["glyra"]);
        assert_eq!(r.slots[1].kind, Kind::Temperature);
        assert_eq!(r.slots[1].hunters, vec!["wynelle", "solyth"]);
        assert_eq!(r.slots[2].kind, Kind::Empty);
        assert_eq!(r.slots[3].kind, Kind::Empty);
        for s in &r.slots {
            assert!(key_code(&s.key).is_some(), "{} has a key", s.id);
        }
        assert_eq!(key_code(&r.slots[0].key), Some(KeyCode::Key6));
    }

    #[test]
    fn the_loader_refuses_a_broken_file() {
        let good = include_str!("../data/ui/specialty_slots.json");
        assert!(parse(good).is_ok());
        let bad = |from: &str, to: &str| {
            assert!(good.contains(from), "the shipped file has {from}");
            assert!(parse(&good.replace(from, to)).is_err(), "{from} -> {to} must be refused");
        };
        bad("lr.specialty_slots/1", "lr.specialty_slots/2");
        bad("\"number\": 7", "\"number\": 8");
        bad("\"Key7\"", "\"Key6\"");
        bad("\"Key8\"", "\"Key0\"");
        bad("\"kind\": \"temperature\"", "\"kind\": \"music_box\"");
        bad("\"hunters\": [\"glyra\"]", "\"hunters\": []");
        bad("\"cap\": 84.0", "\"cap\": 100.0");
        bad("\"floor\": 32.0", "\"floor\": 5.0");
        bad("\"solyth_slow_factor\": 0.6", "\"solyth_slow_factor\": 0.0");
        bad("\"solyth_pounce_reading\": 55.0", "\"solyth_pounce_reading\": 70.0");
        bad("\"default_mode\": \"heat\"", "\"default_mode\": \"warm\"");
    }

    #[test]
    fn slot_six_shows_only_while_glyra_is_on_the_floor_and_slot_seven_for_either_of_the_pair() {
        let (mut g, content) = sim();
        g.sync_specialty();
        assert_eq!(g.specialty.visible, [false, false, true, true], "nobody out: only the reserved slots show");
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        assert_eq!(g.specialty.visible, [true, false, true, true]);
        add(&mut g, &content, "wynelle");
        g.sync_specialty();
        assert_eq!(g.specialty.visible, [true, true, true, true], "Wynelle shows the temperature slot");
        drop_hunter(&mut g, "wynelle");
        add(&mut g, &content, "solyth");
        g.sync_specialty();
        assert!(g.specialty.visible[1], "Solyth shows it too");
        drop_hunter(&mut g, "solyth");
        g.sync_specialty();
        assert!(!g.specialty.visible[1]);
        // a hunter in any state counts: a brand-new brain stands dormant, and still shows the slot
        let b = g.hunters.iter().find(|h| h.def.id == "glyra").unwrap();
        assert!(!b.active, "a fresh brain is dormant");
        assert!(g.specialty.visible[0], "dormant Glyra still shows slot 6");
    }

    #[test]
    fn the_item_is_granted_when_the_hunter_arrives_and_removed_when_she_leaves() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "granted with the hunter");
        assert_eq!(box_in_pack(&g), 0, "slot 6 is not the pack");
        drop_hunter(&mut g, "glyra");
        g.sync_specialty();
        assert_eq!(g.specialty.items[0], None, "removed with the hunter");
        assert_eq!(box_in_pack(&g), 0, "and not returned to the pack");
        // the temperature item too
        add(&mut g, &content, "wynelle");
        g.sync_specialty();
        assert!(matches!(g.specialty.items[1], Some(SpecItem::Temperature(_))));
        drop_hunter(&mut g, "wynelle");
        g.sync_specialty();
        assert_eq!(g.specialty.items[1], None);
        assert_eq!(g.specialty.selected, None);
    }

    #[test]
    fn a_selected_slot_is_deselected_when_its_hunter_leaves() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        g.press_specialty(0);
        assert_eq!(g.specialty.selected, Some(0));
        assert_eq!(g.selected_item(), Some(Item::MusicBox), "the box is in hand");
        drop_hunter(&mut g, "glyra");
        g.sync_specialty();
        assert_eq!(g.specialty.selected, None);
        assert_ne!(g.selected_item(), Some(Item::MusicBox));
    }

    #[test]
    fn the_temperature_item_cycles_between_heat_and_cold_by_key_and_by_use() {
        let (mut g, content) = sim();
        add(&mut g, &content, "wynelle");
        add(&mut g, &content, "solyth");
        g.sync_specialty();
        let start = g.specialty.temp_mode().expect("carried");
        assert_eq!(start, rules().default_mode, "both out: the data default");
        g.press_specialty(1);
        assert_eq!(g.specialty.selected, Some(1));
        assert_eq!(g.specialty.temp_mode(), Some(start), "the first press only selects");
        g.press_specialty(1);
        assert_eq!(g.specialty.temp_mode(), Some(start.other()), "the second press flips");
        g.use_specialty();
        assert_eq!(g.specialty.temp_mode(), Some(start), "use flips it back");
        g.use_specialty();
        assert_eq!(g.specialty.temp_mode(), Some(start.other()));
        assert!(g.message.contains("DETERS"), "the toast says what it deters: {}", g.message);
    }

    #[test]
    fn the_item_starts_as_the_counter_for_the_lone_hunter() {
        let (mut g, content) = sim();
        add(&mut g, &content, "solyth");
        g.sync_specialty();
        assert_eq!(g.specialty.temp_mode(), Some(Mode::Cold), "Solyth alone: cold");
        let (mut g, content) = sim();
        add(&mut g, &content, "wynelle");
        g.sync_specialty();
        assert_eq!(g.specialty.temp_mode(), Some(Mode::Heat), "Wynelle alone: heat");
    }

    #[test]
    fn slots_eight_and_nine_show_empty_and_never_hold_an_item() {
        let (mut g, content) = sim();
        for id in ["glyra", "wynelle", "solyth"] {
            add(&mut g, &content, id);
        }
        g.sync_specialty();
        assert!(g.specialty.visible[2] && g.specialty.visible[3]);
        assert_eq!(g.specialty.items[2], None);
        assert_eq!(g.specialty.items[3], None);
        // selecting one is allowed, using it does nothing
        g.press_specialty(2);
        assert_eq!(g.specialty.selected, Some(2));
        g.use_specialty();
        g.press_specialty(2);
        assert_eq!(g.specialty.items[2], None);
        assert_eq!(g.selected_item(), None);
    }

    #[test]
    fn a_hidden_slot_ignores_its_key() {
        let (mut g, _content) = sim();
        g.sync_specialty();
        g.press_specialty(0);
        g.press_specialty(1);
        assert_eq!(g.specialty.selected, None);
    }

    #[test]
    fn using_slot_six_winds_the_box_and_spends_it_and_the_pack_stays_untouched() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        g.music_box_left = 0.0;
        g.press_specialty(0);
        g.use_specialty();
        assert_eq!(g.music_box_left, crate::game::music_box::wound_seconds(g.difficulty.id()));
        assert_eq!(g.specialty.items[0], None, "the wind spends the box");
        assert_eq!(g.specialty.selected, Some(0), "the empty slot stays shown");
        // a second use winds nothing more
        g.music_box_left = 0.0;
        g.use_specialty();
        assert_eq!(g.music_box_left, 0.0);
    }

    #[test]
    fn a_pack_box_folds_into_slot_six_so_the_runner_never_holds_two() {
        let (mut g, content) = sim();
        assert!(g.give(Item::MusicBox, 1), "Glyra is not out: the box goes to the pack");
        assert_eq!(box_in_pack(&g), 1);
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        assert_eq!(box_in_pack(&g), 0, "the pack box moved");
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "into slot 6");
        // with the slot full a floor box is refused, so there are never two
        assert!(!g.give(Item::MusicBox, 1), "a second box is refused while slot 6 holds one");
        assert_eq!(box_in_pack(&g), 0);
        // once wound, the slot is free again and a floor box goes there, not to the pack
        g.press_specialty(0);
        g.use_specialty();
        assert!(g.give(Item::MusicBox, 1));
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
        assert_eq!(box_in_pack(&g), 0);
    }

    #[test]
    fn a_floor_box_taken_while_glyra_is_out_goes_to_slot_six_and_says_so_when_refused() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        g.pickups.push(crate::game::FloorPickup { pos: g.p.pos, item: Item::MusicBox, n: 1, taken: false });
        g.pickups.push(crate::game::FloorPickup { pos: g.p.pos, item: Item::MusicBox, n: 1, taken: false });
        let (a, b) = (g.pickups.len() - 2, g.pickups.len() - 1);
        // the granted box fills slot 6; the first pickup is refused
        assert!(!g.take_pickup(a, crate::game::interact::Actor::Player));
        assert!(g.message.contains("ALREADY"), "{}", g.message);
        assert!(!g.pickups[a].taken);
        g.press_specialty(0);
        g.use_specialty();
        assert!(g.take_pickup(b, crate::game::interact::Actor::Player));
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
        assert_eq!(box_in_pack(&g), 0);
    }

    #[test]
    fn without_glyra_a_floor_box_goes_to_the_pack_as_before() {
        let (mut g, _content) = sim();
        g.sync_specialty();
        assert!(g.give(Item::MusicBox, 1));
        assert_eq!(box_in_pack(&g), 1);
        assert_eq!(g.specialty.items[0], None);
    }

    #[test]
    fn the_wheel_runs_through_the_hotbar_then_the_specialty_slots_whose_hunter_is_out() {
        let (mut g, content) = sim();
        g.sync_specialty();
        // no hunter out: only the five hotbar slots, and 6 to 9 are empty
        g.select(4);
        g.scroll_hotbar(1);
        assert_eq!((g.selected, g.specialty.selected), (0, None), "wraps from slot 5 to slot 1");
        assert!(g.specialty.items.iter().all(|i| i.is_none()), "no hunters: slots 6 to 9 hold nothing");
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        g.select(4);
        g.scroll_hotbar(1);
        assert_eq!(g.specialty.selected, Some(0), "slot 5 then slot 6");
        g.scroll_hotbar(1);
        assert_eq!((g.selected, g.specialty.selected), (0, None), "slot 6 then back to slot 1");
        g.scroll_hotbar(-1);
        assert_eq!(g.specialty.selected, Some(0), "backwards from slot 1 reaches slot 6");
        g.scroll_hotbar(-1);
        assert_eq!((g.selected, g.specialty.selected), (4, None), "and then slot 5");
    }

    #[test]
    fn selecting_a_common_slot_puts_the_specialty_slot_away() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        g.press_specialty(0);
        g.select(2);
        assert_eq!(g.specialty.selected, None);
        assert_eq!(g.selected, 2);
        g.slots[2] = Some(Slot { item: Item::Crowbar, n: 3 });
        assert_eq!(g.selected_item(), Some(Item::Crowbar));
    }

    #[test]
    fn the_keys_reach_the_specialty_slots_through_update_tools() {
        let (mut g, content) = sim();
        add(&mut g, &content, "glyra");
        g.sync_specialty();
        let c = crate::game::Controls { specialty: Some(0), ..Default::default() };
        g.update_tools(&c, &Sounds::default());
        assert_eq!(g.specialty.selected, Some(0));
        let c = crate::game::Controls { use_item: true, ..Default::default() };
        g.music_box_left = 0.0;
        g.update_tools(&c, &Sounds::default());
        assert!(g.music_box_left > 0.0, "use winds the selected box");
        // the drop key cannot throw a specialty item away
        g.specialty.items[0] = Some(SpecItem::MusicBox);
        let c = crate::game::Controls { drop: true, ..Default::default() };
        g.update_tools(&c, &Sounds::default());
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
    }

    #[test]
    fn heat_warms_and_cold_cools_the_body_target_within_the_data_bounds() {
        let r = rules();
        let mut s = Specialty::default();
        s.items[1] = Some(SpecItem::Temperature(Mode::Heat));
        s.visible[1] = true;
        let ambient = 60.0;
        let t = ambient + s.felt_local(ambient, 0.0);
        assert!((t - (60.0 + r.heat.target_offset).min(r.heat.cap)).abs() < 1e-4, "heat target {t}");
        assert!(t < 90.0, "never overheating, never the latch");
        // a hot room already past the cap is not cooled by the heat source
        assert!((ambient + s.felt_local(ambient, 30.0) - 90.0).abs() < 1e-4);
        s.items[1] = Some(SpecItem::Temperature(Mode::Cold));
        let t = ambient + s.felt_local(ambient, 0.0);
        assert!((t - (60.0 - r.cold.target_offset).max(r.cold.floor)).abs() < 1e-4, "cold target {t}");
        assert!(t > 10.0, "never freezing");
        // a cold room below the floor is not warmed by the cold source
        assert!((ambient + s.felt_local(ambient, -40.0) - 20.0).abs() < 1e-4);
        s.items[1] = None;
        assert_eq!(s.felt_local(ambient, 7.0), 7.0, "no item: the room alone");
    }

    #[test]
    fn the_body_follows_the_source_in_the_real_meters() {
        use crate::game::meters::{Meters, Situation};
        let mut s = Specialty::default();
        s.visible[1] = true;
        let mut run = |mode: Mode| {
            s.items[1] = Some(SpecItem::Temperature(mode));
            let mut m = Meters::default();
            for _ in 0..30 * 20 {
                let local = s.felt_local(m.ambient, 0.0);
                m.update(
                    1.0 / 30.0,
                    &Situation {
                        flashlight_on: false,
                        noise: 0.0,
                        crouching: false,
                        hidden: false,
                        in_safe_zone: false,
                        local,
                    },
                );
            }
            m
        };
        let warm = run(Mode::Heat);
        assert!(warm.temperature > 75.0 && !warm.heat_locked, "heat warms the body but never latches Solyth: {}", warm.temperature);
        let cool = run(Mode::Cold);
        assert!(cool.temperature < 45.0 && !cool.freezing(), "cold cools the body without freezing: {}", cool.temperature);
    }

    #[test]
    fn heat_deters_wynelle_and_cold_feeds_her() {
        use crate::hunters::gimmicks::Ctx;
        use crate::hunters::brain::Target;
        let content = Content::load();
        let fac = crate::world::facility::Facility::generate("SPEC-W", &Default::default(), &Catalog::load());
        let at = fac.start.center();
        let def = content.hunter("wynelle").unwrap().clone();
        let cold_runner = Target { pos: at, facing: (1.0, 0.0), temperature: 40.0, ..Default::default() };
        let run = |heat: bool| {
            let mut b = Brain::new(def.clone(), at, "WYN", Difficulty::Standard);
            let ctx = Ctx {
                fac: &fac,
                relays_taken: &[],
                batteries_taken: &[],
                hiding_disabled: &[],
                vents: &[],
                heater_on: heat,
                heater_thawed: heat,
                diff: Difficulty::Standard,
                music_box_left: 0.0,
                music_boxes: &[],
                exposure: 0.0,
                flashlight_dead: false,
            };
            let mut came = false;
            for _ in 0..30 * 30 {
                b.update(1.0 / 30.0, &ctx, &cold_runner, 1, 0, &Difficulty::Standard.profile());
                came |= b.active;
            }
            came
        };
        assert!(run(false), "a cold runner with no source meets Wynelle");
        assert!(!run(true), "the heat source (a heater to her) keeps her from gathering");
    }

    #[test]
    fn the_game_passes_the_heat_source_to_wynelle_as_a_heater_and_slows_solyth_in_cold() {
        let (mut g, content) = sim();
        add(&mut g, &content, "wynelle");
        add(&mut g, &content, "solyth");
        g.sync_specialty();
        g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Heat));
        assert!(g.specialty.heat_on() && !g.specialty.cold_on());
        assert!(g.heat_source_for_hunters());
        g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Cold));
        assert!(!g.heat_source_for_hunters());
        assert_eq!(g.specialty.reading_for_hunters(95.0), rules().cold.solyth_pounce_reading, "the pounce gate stays shut");
        assert_eq!(g.specialty.reading_for_hunters(40.0), 40.0);
        // one frame of hunting slows the out Solyth only
        let si = g.hunters.iter().position(|h| h.def.id == "solyth").unwrap();
        g.hunters[si].active = true;
        let sp = g.specialty.clone();
        sp.deter(&mut g.hunters[si]);
        assert!(g.hunters[si].slow_timer > 0.0);
        assert_eq!(g.hunters[si].slow_factor, rules().cold.solyth_slow_factor);
        assert!(g.hunters[si].gimmick.immune_to_capture(&g.hunters[si]), "a deterred Solyth cannot grab the runner");
        let wi = g.hunters.iter().position(|h| h.def.id == "wynelle").unwrap();
        assert_eq!(g.hunters[wi].slow_timer, 0.0, "Wynelle is not slowed by cold");
        // heat does not slow Solyth
        g.hunters[si].slow_timer = 0.0;
        g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Heat));
        let sp = g.specialty.clone();
        sp.deter(&mut g.hunters[si]);
        assert_eq!(g.hunters[si].slow_timer, 0.0);
    }

    #[test]
    fn the_heat_source_is_the_only_heater_and_a_climate_outage_switches_it_off() {
        use crate::hunters::gimmicks::void::void_orange::System;
        let (mut g, content) = sim();
        add(&mut g, &content, "wynelle");
        g.sync_specialty();
        g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Cold));
        assert!(!g.heat_source_for_hunters(), "no station is left to count as a heater: cold means no heater at all");
        g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Heat));
        assert!(g.heat_source_for_hunters());
        g.void_fx.outage = Some((System::Climate, 1.0));
        assert!(!g.heat_source_for_hunters(), "Void Orange's climate outage takes the heat source offline");
        g.void_fx.outage = Some((System::Radar, 1.0));
        assert!(g.heat_source_for_hunters(), "another system's outage leaves it running");
    }

    // -- the slots follow the run, not the drawn floor (KT 2026-11-06: the item only showed in the sewers and the Inferno,
    //    where the floor's own lineup holds Solyth; on level 0 she arrives from a resident list or the shelves) --

    #[test]
    fn level_zero_shows_slot_seven_while_either_of_the_pair_lives_in_another_resident_list() {
        let (mut g, content) = sim();
        assert_eq!(g.inf.as_ref().unwrap().level, 0, "the drawn floor is level 0");
        g.sync_specialty();
        assert!(!g.specialty.visible[1] && g.specialty.items[1].is_none(), "nobody in the run: no item");
        // Solyth lives in the lineup of the floor below; level 0's own list holds neither of the pair
        let s = slot_of(&g, &content, "solyth", -1);
        g.resident_hunters.put(-1, vec![s]);
        assert!(g.hunters.is_empty());
        g.sync_specialty();
        assert!(g.specialty.visible[1], "slot 7 shows on level 0");
        assert_eq!(g.specialty.items[1], Some(SpecItem::Temperature(Mode::Cold)), "granted, the counter for Solyth alone");
        // and Wynelle in a list of the floor above
        g.resident_hunters.take(-1);
        g.sync_specialty();
        assert!(!g.specialty.visible[1] && g.specialty.items[1].is_none(), "removed when no hunter of the slot is left");
        let w = slot_of(&g, &content, "wynelle", 1);
        g.resident_hunters.put(1, vec![w]);
        g.sync_specialty();
        assert_eq!(g.specialty.items[1], Some(SpecItem::Temperature(Mode::Heat)), "the counter for Wynelle alone");
        // Glyra in a resident list gives slot 6 its box the same way
        let gl = slot_of(&g, &content, "glyra", 1);
        g.resident_hunters.list_mut(1).push(gl);
        g.sync_specialty();
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
    }

    #[test]
    fn the_shelves_count_as_the_run_and_the_slot_stays_while_the_hunter_exists() {
        let (mut g, content) = sim();
        // Wynelle stored on level 0 outside the window
        let w = slot_of(&g, &content, "wynelle", 0).brain;
        g.inf.as_mut().unwrap().stored_mut().insert("FAR-WYNELLE".into(), w);
        g.sync_specialty();
        assert_eq!(g.specialty.items[1], Some(SpecItem::Temperature(Mode::Heat)));
        g.inf.as_mut().unwrap().stored_mut().clear();
        g.sync_specialty();
        assert!(g.specialty.items[1].is_none());
        // Solyth lives on level -1; the runner presses 7 to hold cold, then leaves by the stairs so the level's list goes
        // to the shelf: the slot, the item and the mode stay
        let s = slot_of(&g, &content, "solyth", -1);
        g.resident_hunters.put(-1, vec![s]);
        g.sync_specialty();
        g.press_specialty(1);
        g.press_specialty(1);
        let held = g.specialty.temp_mode().expect("carried");
        let slots = g.resident_hunters.take(-1);
        g.store_slots(-1, slots);
        assert!(g.resident_hunters.list(-1).is_empty());
        g.sync_specialty();
        assert!(g.specialty.visible[1], "she still exists in the run");
        assert_eq!(g.specialty.temp_mode(), Some(held), "nothing was re-granted");
        assert_eq!(g.specialty.selected, Some(1), "and the selection is kept");
        // she is gone from the run: the item goes
        g.inf.as_mut().unwrap().floor_hunters_mut().clear();
        g.sync_specialty();
        assert!(!g.specialty.visible[1] && g.specialty.items[1].is_none());
        assert_eq!(g.specialty.selected, None);
    }

    fn solyth_state(g: &Game, level: Option<i32>) -> (bool, bool, bool) {
        let b = match level {
            Some(l) => g.resident_hunters.list(l).iter().map(|s| &s.brain).find(|b| b.def.id == "solyth"),
            None => g.hunters.iter().find(|b| b.def.id == "solyth"),
        }
        .expect("Solyth is in the list");
        (b.active, matches!(b.state, crate::hunters::brain::State::Chase), b.gimmick.immune_to_capture(b))
    }

    fn run_frames(g: &mut Game, n: usize) {
        let (prof, sounds) = (g.prof(), Sounds::default());
        for _ in 0..n {
            g.update_hunters(1.0 / 60.0, &prof, &sounds);
        }
    }

    #[test]
    fn a_summoned_solyth_on_level_zero_is_deterred_by_cold_and_still_chases_in_heat() {
        for cold in [true, false] {
            let (mut g, content) = sim();
            add(&mut g, &content, "solyth");
            g.sync_specialty();
            g.specialty.items[1] = Some(SpecItem::Temperature(if cold { Mode::Cold } else { Mode::Heat }));
            g.meters.heat_locked = true;
            run_frames(&mut g, 120);
            let (active, chasing, immune) = solyth_state(&g, None);
            assert!(active, "the latched heat summoned her");
            if cold {
                assert!(!chasing && immune, "cold: she is out of the chase and cannot grab");
            } else {
                assert!(chasing && !immune, "heat: she still chases and can grab");
            }
        }
    }

    #[test]
    fn solyth_summoned_in_a_resident_list_is_deterred_through_the_item_level_zero_now_carries() {
        for cold in [true, false] {
            let (mut g, _content) = stacked_sim();
            g.hunters.clear();
            if let Some(inf) = g.inf.as_mut() {
                inf.loaded_mut().clear();
            }
            // only Solyth lives below, as in the sewer lineup
            g.resident_hunters.list_mut(-1).retain(|s| s.brain.def.id == "solyth");
            assert_eq!(g.resident_hunters.list(-1).len(), 1, "the lineup of level -1 holds Solyth");
            g.sync_specialty();
            assert!(g.specialty.visible[1], "level 0 carries the item although its own list is empty");
            g.specialty.items[1] = Some(SpecItem::Temperature(if cold { Mode::Cold } else { Mode::Heat }));
            g.meters.heat_locked = true;
            run_frames(&mut g, 150);
            let (active, _, immune) = solyth_state(&g, Some(-1));
            assert!(active, "the latched heat summoned her on her own floor");
            if cold {
                assert!(immune, "cold reaches her through the resident update: she cannot grab");
                let b = g.resident_hunters.list(-1).iter().map(|s| &s.brain).find(|b| b.def.id == "solyth").unwrap();
                assert!(!matches!(b.state, crate::hunters::brain::State::Chase), "and she is out of the chase");
            } else {
                assert!(!immune, "heat leaves her able to grab");
            }
        }
    }
}
