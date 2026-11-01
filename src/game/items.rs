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
    pub(super) fn grant(&mut self, item: Item, n: i32) -> bool {
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
        if self.p.flashlight_on && self.selected_item() != Some(Item::Flashlight) {
            self.p.flashlight_on = false;
        }
    }

    fn slot_of(&self, item: Item) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.map(|s| s.item == item).unwrap_or(false))
    }

    fn toggle_flashlight(&mut self, _sounds: &Sounds) {
        let Some(i) = self.slot_of(Item::Flashlight) else {
            return;
        };
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
                    // a taser-immune hunter is slowed, and no message is sent
                    self.hunters[h].apply_slow(prof.taser_slow_seconds, prof.taser_slow_factor);
                } else {
                    self.hunters[h].apply_stun(prof.escape_stun, false);
                    let n = self.hunters[h].def.name.to_uppercase();
                    self.say(&format!("TASER HIT // {n} STUNNED"));
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
        if let Some(h) = target {
            if !self.hunters[h].def.melee_immune {
                let s = if axe {
                    prof.axe_stun
                } else {
                    prof.crowbar_stun
                };
                self.hunters[h].apply_stun(s, true);
                self.hits += 1;
                let n = self.hunters[h].def.name.to_uppercase();
                self.say(&format!("{n} STUNNED {s:.1}s"));
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
        }
    }

    fn drop_selected(&mut self) {
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
        self.pickups.push(FloorPickup {
            pos: p,
            item: slot.item,
            n: if stack { 1 } else { slot.n },
            taken: false,
        });
        if stack {
            self.consume_selected();
        } else {
            self.slots[self.selected] = None;
        }
        self.say(&format!("DROPPED {}", slot.item.label()));
    }

    pub(super) fn update_tools(&mut self, c: &super::Controls, sounds: &Sounds) {
        if self.shop_open() {
            return;
        }
        if let Some(s) = c.slot {
            self.select(s);
        }
        if c.wheel != 0 {
            let n = (self.selected as i32 + c.wheel).rem_euclid(HOTBAR_SIZE as i32) as usize;
            self.select(n);
        }
        if c.flashlight {
            self.toggle_flashlight(sounds);
        }
        if c.taser {
            self.fire_taser(sounds);
        }
        if c.use_item {
            self.use_selected(sounds);
        }
        if c.drop {
            self.drop_selected();
        }
    }
}
