//! The five-slot hotbar and the tools in it (player_controller_3d.gd): flashlight, taser
//! (slows taser-immune hunters), crowbar and rescue axe (melee stun, durability), batteries
//! (+25 lamp) and the stim pack.

use super::{FloorPickup, Game};
use crate::audio::Sounds;
use crate::config::*;
use crate::hunters::brain::Brain;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Flashlight,
    Taser,
    Crowbar,
    Axe,
    Battery,
    Stim,
    /// The music box: wound from the pack, then spent (game/music_box.rs).
    MusicBox,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Flashlight => "FLASHLIGHT",
            Item::Taser => "TASER",
            Item::Crowbar => "CROWBAR",
            Item::Axe => "AXE",
            Item::Battery => "FLASHLIGHT BATTERY",
            Item::Stim => "STIM PACK",
            Item::MusicBox => "MUSIC BOX",
        }
    }

    pub fn stack_max(self) -> i32 {
        match self {
            Item::Battery => BATTERY_STACK_MAX,
            Item::Stim => STIM_STACK_MAX,
            _ => 1,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Slot {
    pub item: Item,
    /// Stack count for batteries / stims, durability for crowbar / axe.
    pub n: i32,
}

impl Game {
    /// True when `grant(item, _)` would find room: a stack with space, or an empty slot.
    pub(super) fn can_grant(&self, item: Item) -> bool {
        // while Glyra is out a box belongs in slot 6, and only when that slot is empty (never two boxes)
        if item == Item::MusicBox && self.specialty.box_slot_visible() {
            return self.specialty.box_slot_free();
        }
        (item.stack_max() > 1 && self.slots.iter().flatten().any(|s| s.item == item && s.n < item.stack_max()))
            || self.slots.iter().any(|s| s.is_none())
    }

    pub(super) fn grant(&mut self, item: Item, n: i32) -> bool {
        if item == Item::MusicBox && self.specialty.box_slot_visible() {
            return self.specialty.put_box();
        }
        if item.stack_max() > 1 {
            for s in self.slots.iter_mut().flatten() {
                if s.item == item && s.n < item.stack_max() {
                    s.n += 1;
                    return true;
                }
            }
        }
        if let Some(i) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[i] = Some(Slot { item, n });
            return true;
        }
        false
    }

    /// The admin `give` helper (agents and tests): adds one item, or one to a stack.
    pub fn give(&mut self, item: Item, n: i32) -> bool {
        self.grant(item, n)
    }

    pub(super) fn select(&mut self, i: usize) {
        self.selected = i % HOTBAR_SIZE;
        self.specialty.selected = None; // a hotbar slot in hand puts the specialty slot away
        if self.p.flashlight_on && self.selected_item() != Some(Item::Flashlight) {
            self.p.flashlight_on = false;
        }
    }

    fn slot_of(&self, item: Item) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.map(|s| s.item == item).unwrap_or(false))
    }

    /// Strobes the runner's beam for `duration` seconds (player_controller_3d.gd `force_flashlight_flicker`): a landing
    /// in the pool (1.4 s) and the troll's fake step (0.4-1.2 s). Only a runner who owns a flashlight is affected.
    pub fn force_flashlight_flicker(&mut self, duration: f32) -> bool {
        if self.slot_of(Item::Flashlight).is_none() {
            return false;
        }
        self.p.force_flashlight_flicker(duration);
        true
    }

    fn toggle_flashlight(&mut self, _sounds: &Sounds) {
        let Some(i) = self.slot_of(Item::Flashlight) else {
            return;
        };
        if self.p.flashlight_seized() {
            return; // the flicker holds the beam; [F] does not fight it (player_controller_3d.gd 2407)
        }
        if self.void_fx.offline(crate::hunters::gimmicks::void::void_orange::System::Flashlight) {
            crate::audio::sound::light_switch(); // Void Orange: forced off and cannot be switched on (void_player_effects.gd:64-70)
            return;
        }
        self.select(i);
        if self.p.lamp <= 0.0 && !self.p.flashlight_on {
            crate::audio::sound::light_switch(); // an empty flashlight just clicks (player_controller_3d.gd 2442)
            return;
        }
        self.p.flashlight_on = !self.p.flashlight_on;
    }

    /// The nearest active hunter inside a cone in front of the runner.
    fn cone_target(&self, range: f32, half_angle: f32) -> Option<usize> {
        let f = self.forward2();
        let mut best: Option<(f32, usize)> = None;
        for (i, h) in self.hunters.iter().enumerate() {
            if !h.active || h.gimmick.hidden_from_view() {
                continue;
            }
            if Brain::beam_reaches(&self.fac, self.p.pos, f, h.pos, range, half_angle) {
                let d = super::pdist(h.pos, self.p.pos);
                if best.map(|b| d < b.0).unwrap_or(true) {
                    best = Some((d, i));
                }
            }
        }
        best.map(|b| b.1)
    }

    fn fire_taser(&mut self, _sounds: &Sounds) {
        let Some(i) = self.slot_of(Item::Taser) else {
            return;
        };
        self.select(i);
        if self.taser_cd > 0.0 {
            return;
        }
        let prof = self.prof();
        self.taser_cd = prof.taser_cooldown;
        self.taser_fx = 0.25;
        self.shots += 1;
        crate::audio::sound::taser();
        match self.cone_target(TASER_RANGE, TASER_HALF_ANGLE) {
            Some(h) => {
                self.hits += 1;
                if self.hunters[h].def.taser_immune {
                    // a taser-immune hunter is slowed, and no message is sent. Her own taser_slow_factor /
                    // taser_slow_seconds (data file) override the profile values when set (hunter_roster.gd:243-248).
                    let def = &self.hunters[h].def;
                    let factor = def.taser_slow_factor.unwrap_or(prof.taser_slow_factor);
                    let seconds = def.taser_slow_seconds.unwrap_or(prof.taser_slow_seconds);
                    self.hunters[h].apply_slow(seconds, factor);
                } else {
                    let stun = taser_stun(&prof);
                    self.hunters[h].apply_stun(stun, false);
                    self.say(&format!("STUNNED {:.1}s", stun));
                }
            }
            None => self.say("TASER // NO TARGET"),
        }
    }

    fn swing(&mut self, item: Item, _sounds: &Sounds) {
        if self.melee_cd > 0.0 {
            return;
        }
        let prof = self.prof();
        self.melee_cd = MELEE_STRIKE_COOLDOWN;
        self.swing_t = 0.35;
        self.shots += 1;
        let axe = item == Item::Axe;
        if axe {
            self.p.noise_burst = AXE_NOISE;
            self.p.noise_burst_timer = AXE_NOISE_SECONDS;
        }
        let target = self.cone_target(MELEE_STRIKE_RANGE, MELEE_STRIKE_HALF_ANGLE);
        // axe: a whoosh now; on a hit the chop (axe) or the breach thud (crowbar) at impact (player_controller_3d.gd 3447-3469)
        crate::audio::sound::tool_swing(
            if axe {
                crate::audio::sound::SwingTool::Axe
            } else {
                crate::audio::sound::SwingTool::Crowbar
            },
            target.is_some(),
        );
        if axe && target.is_none() {
            if let Some((chunk, n, need)) = self.axe_pipe_target(MELEE_STRIKE_RANGE, MELEE_STRIKE_HALF_ANGLE) {
                if self.strike_pipe(chunk, n, need) {
                    self.say("PIPE BURST");
                } else {
                    self.say("PIPE DENTED");
                }
            }
        }
        if let Some(h) = target {
            if !self.hunters[h].def.melee_immune {
                let s = if axe {
                    prof.axe_stun
                } else {
                    prof.crowbar_stun
                };
                self.hunters[h].apply_stun(s, true);
                self.hits += 1;
            }
        }
        if let Some(slot) = self.slots[self.selected].as_mut() {
            slot.n -= if axe {
                AXE_SWING_COST
            } else {
                CROWBAR_SWING_COST
            };
            if slot.n <= 0 {
                self.slots[self.selected] = None;
                crate::audio::sound::tool_break();
                self.say(&format!("{} BROKE", item.label()));
            }
        }
    }

    fn consume_selected(&mut self) {
        if let Some(s) = self.slots[self.selected].as_mut() {
            s.n -= 1;
            if s.n <= 0 {
                self.slots[self.selected] = None;
            }
        }
    }

    fn use_selected(&mut self, sounds: &Sounds) {
        let Some(slot) = self.slots[self.selected] else {
            return;
        };
        match slot.item {
            Item::Flashlight => self.toggle_flashlight(sounds),
            Item::Taser => self.fire_taser(sounds),
            Item::Crowbar | Item::Axe => self.swing(slot.item, sounds),
            Item::Battery => {
                if self.p.lamp >= MAX_LAMP_CHARGE {
                    self.say("FLASHLIGHT ALREADY FULL");
                    return;
                }
                self.p.lamp = (self.p.lamp + BATTERY_RECHARGE).min(MAX_LAMP_CHARGE);
                crate::audio::sound::recharge(BATTERY_RECHARGE);
                self.consume_selected();
                self.say("FLASHLIGHT RECHARGED");
            }
            Item::Stim => {
                if self.p.stim_inject > 0.0 {
                    return;
                }
                self.p.stim_inject = STIM_INJECT_SECONDS;
                crate::audio::sound::menu_blip(); // the injection starts (player_controller_3d.gd 3326)
                self.consume_selected();
                self.say("STIM PACK");
            }
            Item::MusicBox => {
                self.wind_music_box();
                self.consume_selected(); // a pack box is spent by the wind (slot 6 spends its own, specialty.rs)
            }
        }
    }

    /// The box's use: it winds for its difficulty's time (game/music_box.rs) and the winding phrase sounds
    /// (route_interactable_3d.gd 818). The caller spends the box, from the pack or from specialty slot 6. The room
    /// effect is the clock `run_music_box` reads.
    pub(super) fn wind_music_box(&mut self) {
        self.music_box_left = super::music_box::wound_seconds(self.difficulty.id());
        crate::audio::sound::music_box();
        self.say("Music box wound. The room settles.");
    }

    /// A tap of the drop key: one item of the selected stack (a non-stackable item drops whole).
    pub(super) fn drop_selected(&mut self) {
        self.drop_selected_amount(false);
    }

    /// Drops the selected slot to the floor: one item, or with `whole` the entire stack (Godot: hold the drop key).
    /// A non-stackable item drops as it is either way. The flashlight never drops.
    pub(super) fn drop_selected_amount(&mut self, whole: bool) {
        let Some(slot) = self.slots[self.selected] else {
            return;
        };
        if slot.item == Item::Flashlight {
            return;
        }
        let (fx, fy) = self.forward2();
        let p = self
            .fac
            .nearest_open_or(self.p.pos.0 + fx * 0.8, self.p.pos.1 + fy * 0.8, 0.2);
        let stack = slot.item.stack_max() > 1;
        let one = stack && !whole;
        self.pickups.push(FloorPickup {
            pos: p,
            item: slot.item,
            n: if one { 1 } else { slot.n },
            taken: false,
        });
        if one {
            self.consume_selected();
        } else {
            self.slots[self.selected] = None;
        }
        self.say(&format!("DROPPED {}", slot.item.label()));
    }

    /// The drop key each frame: `pressed` is the press edge (drops one), `held_for` the seconds the key has been down
    /// (0 when up). Held past data/player/drop.json `hold_seconds` drops what is left of the stack, once per hold.
    pub(super) fn drop_input(&mut self, pressed: bool, held_for: f32) {
        thread_local!(static FIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
        if held_for <= 0.0 && !pressed {
            FIRED.with(|f| f.set(false));
            return;
        }
        if pressed {
            FIRED.with(|f| f.set(false));
            self.drop_selected_amount(false);
        }
        if held_for >= drop_hold_seconds() && !FIRED.with(|f| f.replace(true)) {
            self.drop_selected_amount(true);
        }
    }

    /// The wheel moves through one ring: the five hotbar slots, then every specialty slot whose hunter is out (an empty
    /// placeholder slot is skipped), so slots 6 and up swap into the hand like 1 to 5.
    pub(super) fn scroll_hotbar(&mut self, wheel: i32) {
        use super::specialty::{rules, Kind};
        let spec: Vec<usize> = (0..self.specialty.visible.len())
            .filter(|&i| self.specialty.visible[i] && !matches!(rules().slots[i].kind, Kind::Empty))
            .collect();
        let ring = HOTBAR_SIZE + spec.len();
        let here = match self.specialty.selected {
            Some(s) => spec.iter().position(|&x| x == s).map_or(0, |p| HOTBAR_SIZE + p),
            None => self.selected,
        };
        let next = (here as i32 + wheel).rem_euclid(ring as i32) as usize;
        if next < HOTBAR_SIZE {
            self.select(next);
        } else {
            self.press_specialty(spec[next - HOTBAR_SIZE]);
        }
    }

    pub(super) fn update_tools(&mut self, c: &super::Controls, sounds: &Sounds) {
        if self.shop_open() {
            return;
        }
        if let Some(s) = c.slot {
            self.select(s);
        }
        if c.wheel != 0 {
            self.scroll_hotbar(c.wheel);
        }
        if c.flashlight {
            self.toggle_flashlight(sounds);
        }
        if c.taser {
            self.fire_taser(sounds);
        }
        if let Some(i) = c.specialty {
            self.press_specialty(i);
        }
        if c.use_item {
            if self.specialty.selected.is_some() {
                self.use_specialty();
            } else {
                self.use_selected(sounds);
            }
        }
        // a specialty item cannot be dropped: it leaves with its hunter
        if self.specialty.selected.is_none() {
            self.drop_input(c.drop, c.drop_held_for);
        }
    }
}

/// Seconds the drop key is held before the whole stack drops (data/player/drop.json `hold_seconds`).
pub fn drop_hold_seconds() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        crate::content::embedded::data_under("player/drop.json")
            .next()
            .and_then(|(_, t)| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|v| v.get("hold_seconds").and_then(|x| x.as_f64()))
            .unwrap_or(0.5) as f32
    })
}

/// The taser's stun in seconds: the profile's escape stun less `stun_offset` (never under `stun_min`), both from
/// data/player/taser.json (game_config.gd: taser_stun = escape_stun - 0.2 on every difficulty). A `taser_stun` field on
/// `Profile` would replace this.
pub fn taser_stun(prof: &Profile) -> f32 {
    static RULES: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
    let (offset, min) = *RULES.get_or_init(|| {
        let v = crate::content::embedded::data_under("player/taser.json")
            .next()
            .and_then(|(_, t)| serde_json::from_str::<serde_json::Value>(t).ok());
        let get = |k: &str, d: f64| v.as_ref().and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d) as f32;
        (get("stun_offset", 0.2), get("stun_min", 0.5))
    });
    (prof.escape_stun - offset).max(min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    /// A standard game with a hunter 0.8 m in front of the runner (None when no spot has a clear line).
    fn face_a_hunter() -> Option<(Game, usize)> {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-ITEMS-7", 0, &content, &catalog);
        g.relays_collected = 1;
        g.ensure_hunters(&content);
        let t = g.hunters.iter().position(|h| h.active && !h.def.taser_immune && !h.def.melee_immune && !h.gimmick.hidden_from_view())?;
        let hp = g.hunters[t].pos;
        for (dx, dz) in [(0.8f32, 0.0f32), (-0.8, 0.0), (0.0, 0.8), (0.0, -0.8)] {
            let from = (hp.0 + dx, hp.1 + dz);
            if g.fac.has_line_of_sight(from, hp) {
                g.p.pos = from;
                g.p.yaw = (-(hp.0 - from.0)).atan2(-(hp.1 - from.1));
                g.hunters[t].stun = 0.0;
                return Some((g, t));
            }
        }
        None
    }

    fn game_with_stack(item: Item, n: i32) -> Game {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "DROP-HOLD-7", 0, &content, &catalog);
        g.pickups.clear();
        g.slots[g.selected] = Some(Slot { item, n });
        g
    }

    fn dropped(g: &Game) -> i32 {
        g.pickups.iter().map(|p| p.n).sum()
    }

    #[test]
    fn a_tap_drops_one_of_a_stack() {
        let mut g = game_with_stack(Item::Battery, 3);
        g.drop_input(true, 0.0);
        g.drop_input(false, 0.0);
        assert_eq!(dropped(&g), 1);
        assert_eq!(g.slots[g.selected].map(|s| s.n), Some(2));
    }

    #[test]
    fn holding_past_the_threshold_drops_the_whole_stack_once() {
        let mut g = game_with_stack(Item::Battery, 3);
        g.drop_input(true, 0.0);
        g.drop_input(false, drop_hold_seconds() * 0.5);
        assert_eq!(dropped(&g), 1, "before the threshold only the tap's one");
        g.drop_input(false, drop_hold_seconds() + 0.01);
        assert_eq!(dropped(&g), 3);
        assert!(g.slots[g.selected].is_none());
        g.drop_input(false, drop_hold_seconds() + 0.5);
        assert_eq!(dropped(&g), 3, "one drop per hold");
        assert!(drop_hold_seconds() > 0.0);
    }

    #[test]
    fn a_non_stackable_drops_one_either_way() {
        let mut a = game_with_stack(Item::Crowbar, 10);
        a.drop_input(true, 0.0);
        let mut b = game_with_stack(Item::Crowbar, 10);
        b.drop_input(true, 0.0);
        b.drop_input(false, drop_hold_seconds() + 0.1);
        assert_eq!(a.pickups.len(), 1);
        assert_eq!(b.pickups.len(), 1);
        assert_eq!(dropped(&a), dropped(&b));
    }

    #[test]
    fn the_standard_taser_stun_is_the_escape_stun_less_the_offset() {
        let prof = Difficulty::Standard.profile();
        assert!((taser_stun(&prof) - (prof.escape_stun - 0.2)).abs() < 1e-5);
        assert!(taser_stun(&prof) < prof.escape_stun);
    }

    #[test]
    fn the_taser_stuns_for_the_taser_time_and_says_so() {
        let Some((mut g, t)) = face_a_hunter() else { return };
        let want = taser_stun(&g.prof());
        g.taser_cd = 0.0;
        g.fire_taser(&Sounds::default());
        assert!((g.hunters[t].stun - want).abs() < 1e-4, "stun {} want {want}", g.hunters[t].stun);
        assert_eq!(g.toast, format!("STUNNED {:.1}s", want));
    }

    #[test]
    fn a_crowbar_hit_stuns_and_prints_no_stunned_message() {
        let Some((mut g, t)) = face_a_hunter() else { return };
        g.slots[g.selected] = Some(Slot { item: Item::Crowbar, n: 10 });
        g.toast.clear();
        g.melee_cd = 0.0;
        g.swing(Item::Crowbar, &Sounds::default());
        assert!(g.hunters[t].stun > 0.0, "the hit lands");
        assert!(!g.toast.contains("STUNNED"), "toast was {:?}", g.toast);
    }

    /// The music box is collected into the pack, wound once from the hotbar, and spent: the wind lasts the difficulty's
    /// wound time, the tune's last 15 s are the wind-down, and a second use finds nothing to wind.
    #[test]
    fn a_music_box_from_the_pack_winds_once_for_the_wind_duration_and_is_spent() {
        use crate::audio::cues::music_box::{mood_for, Mood};
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Hard, 7, "BOX-PACK-7", 0, &content, &catalog);
        assert!(g.give(Item::MusicBox, 1), "the pack takes the box");
        let i = g.slots.iter().position(|s| s.map(|s| s.item == Item::MusicBox).unwrap_or(false)).expect("a slot holds the box");
        g.select(i);
        g.music_box_left = 0.0;
        g.use_selected(&Sounds::default());
        assert_eq!(g.music_box_left, 35.0, "Hard winds to 35 s");
        assert!(g.slots.iter().flatten().all(|s| s.item != Item::MusicBox), "the wind spends the box");
        assert_eq!(mood_for(g.music_box_left), Mood::Playing);
        // the clock runs for the wind duration: playing, then the last 15 s wind down, then it stops
        g.run_music_box(19.5);
        assert_eq!(mood_for(g.music_box_left), Mood::Playing);
        g.run_music_box(0.5);
        assert_eq!(mood_for(g.music_box_left), Mood::WindingDown, "the tune slows in the last 15 s");
        g.run_music_box(15.0);
        assert_eq!(g.music_box_left, 0.0);
        assert_eq!(mood_for(g.music_box_left), Mood::Stopped);
        // with no box in the pack, a use winds nothing
        g.selected = 0;
        g.use_selected(&Sounds::default());
        assert_eq!(g.music_box_left, 0.0, "no box, no wind");
    }

    /// Every finite floor's box is a floor item the runner can walk to from the start: a bot or a player reaches it.
    #[test]
    fn every_finite_floor_box_is_reachable_from_the_start() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut boxes = 0;
        for seed in 1..=8u32 {
            let g = Game::new_sim(Difficulty::Standard, seed, &content, &catalog);
            for p in g.pickups.iter().filter(|p| p.item == Item::MusicBox) {
                boxes += 1;
                let cell = crate::world::Cell::new(p.pos.0.floor() as i32, p.pos.1.floor() as i32);
                assert!(!g.fac.find_path(g.fac.start, cell).is_empty(), "seed {seed}: the box at {cell:?} is unreachable");
            }
        }
        assert!(boxes >= 4, "most floors carry a box: {boxes} of 8");
    }

    /// A finite floor carries its music box as a floor pickup (no fixture). Taking it puts the box in the pack, and the
    /// pack's use winds it for the difficulty's time.
    #[test]
    fn a_floor_box_is_a_pickup_that_goes_into_the_pack_and_winds() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_sim(Difficulty::Standard, 11, &content, &catalog);
        let i = g
            .pickups
            .iter()
            .position(|p| p.item == Item::MusicBox && !p.taken)
            .expect("the floor has its music box on it as a pickup");
        // the box is a floor item the runner can walk to from the start (the placement's `can_occupy` test)
        let at = g.pickups[i].pos;
        let cell = crate::world::Cell::new(at.0.floor() as i32, at.1.floor() as i32);
        assert!(!g.fac.find_path(g.fac.start, cell).is_empty(), "the box at {cell:?} is not reachable from the start");
        assert!(g.take_pickup(i, crate::game::interact::Actor::Player), "the pack takes the box");
        assert!(g.pickups[i].taken, "the floor pickup is spent");
        let slot = g.slots.iter().position(|s| s.map(|s| s.item == Item::MusicBox).unwrap_or(false)).expect("the box is in the pack");
        g.select(slot);
        g.music_box_left = 0.0;
        g.use_selected(&Sounds::default());
        assert_eq!(g.music_box_left, 45.0, "standard winds to 45 s");
        assert!(g.slots.iter().flatten().all(|s| s.item != Item::MusicBox), "the wind spends the box");
    }
}
