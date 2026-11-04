//! What the crosshair is on and what [E] does to it (route_interactable_3d.gd): relays,
//! batteries, equipment, coins, lockers, the safe zone, the service lift, room light switches,
//! the heater station, the music box (hold to wind), vent grates and Zellia's store. Opalie's
//! guard vetoes anything near her post.

use super::{pdist, Game, Heater, Item};
use crate::audio::Sounds;
use crate::config::*;
use crate::content::Content;
use crate::hunters::gimmicks::{ceryth::Entry, Gimmick};
use crate::world::catalog::Catalog;
use crate::world::facility::{Cover, DoorToggle, LinkKind};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Focus {
    Relay(usize),
    Battery(usize),
    Pickup(usize),
    Coin(usize),
    Locker(usize),
    Safe,
    Lift,
    Stairs(usize),
    LightSwitch(usize),
    Heater,
    MusicBox,
    Vent(usize),
    Shop(usize),
    /// A room door, by its index in `Facility::props`.
    Door(usize),
}

impl Game {
    /// One automatic save of the run (Godot `autosave`, infinite_route_test.gd): a relay, a newly active checkpoint or
    /// a level change (the stairs and the lift) or leaving the route. A finished run never saves (Godot
    /// `_save_write_denial`: THE RUN HAS ENDED), so a death still clears the save. Returns true when it saved.
    pub fn autosave(&mut self, reason: &'static str) -> bool {
        if self.report.is_some() {
            return false;
        }
        self.save_checkpoint();
        self.last_autosave = reason;
        true
    }

    /// The load of a flight, run behind the fade (game/transition.rs): a flight taken saves the run on its new level
    /// (Godot `_update_run_statistics`: autosave("level")).
    pub(super) fn take_flight(&mut self, i: usize, content: &Content, catalog: &Catalog) {
        let from = self.inf.as_ref().map(|inf| inf.level);
        self.take_stairs(i, content, catalog);
        if self.inf.as_ref().map(|inf| inf.level) != from {
            self.autosave("level");
        }
    }

    pub fn shop_open(&self) -> bool {
        self.hunters
            .iter()
            .any(|h| matches!(&h.gimmick, Gimmick::Zellia(z) if z.shop_open))
    }

    fn safe_beacon(&self) -> Option<(f32, f32)> {
        self.fac.safe_zone.map(|s| s.center())
    }

    pub(super) fn find_focus(&self) -> Option<Focus> {
        let eye = self.eye();
        let (fwd, _, _) = self.camera_basis();
        let mut best: Option<(f32, Focus)> = None;
        let mut consider = |p: (f32, f32), h: f32, reach: f32, f: Focus| {
            if pdist(p, self.p.pos) > reach + 0.3 {
                return;
            }
            let to = macroquad::prelude::vec3(p.0, h, p.1) - eye;
            let len = to.length().max(0.001);
            let ang = fwd.dot(to / len).clamp(-1.0, 1.0).acos();
            let tol = 0.42 + (0.35 / len).min(0.6);
            if ang > tol
                || (!self.fac.has_line_of_sight(self.p.pos, p) && pdist(p, self.p.pos) > 0.9)
            {
                return;
            }
            if best.map(|b| ang < b.0).unwrap_or(true) {
                best = Some((ang, f));
            }
        };
        for (i, c) in self.fac.relays.iter().enumerate() {
            if !self.relays_taken[i] {
                consider(c.center(), 0.8, INTERACT_REACH, Focus::Relay(i));
            }
        }
        for (i, c) in self.fac.batteries.iter().enumerate() {
            if !self.batteries_taken[i] {
                consider(c.center(), 0.15, INTERACT_REACH, Focus::Battery(i));
            }
        }
        for (i, p) in self.pickups.iter().enumerate() {
            if !p.taken {
                consider(p.pos, 0.15, INTERACT_REACH, Focus::Pickup(i));
            }
        }
        for (i, c) in self.fix.coins.iter().enumerate() {
            if !self.coins_taken[i] {
                consider(c.center(), 0.1, INTERACT_REACH, Focus::Coin(i));
            }
        }
        for (i, h) in self.fac.hiding.iter().enumerate() {
            consider(
                self.fac.locker_center(h),
                1.0,
                LOCKER_REACH + 0.35,
                Focus::Locker(i),
            );
        }
        if let Some(b) = self.safe_beacon() {
            consider(b, 0.7, INTERACT_REACH, Focus::Safe);
        }
        if self.fac.has_lift {
            consider(self.fac.exit.center(), 1.2, 1.8, Focus::Lift);
        }
        for (i, s) in self.fac.stairs.iter().enumerate() {
            // the flight's first step, just past the landing
            let (cx, cy) = s.cell.center();
            consider(
                (cx + s.run.0 as f32 * 0.9, cy + s.run.1 as f32 * 0.9),
                0.3,
                2.0,
                Focus::Stairs(i),
            );
        }
        for (i, l) in self.fac.room_lights.iter().enumerate() {
            consider(
                l.switch_cell.center(),
                1.2,
                INTERACT_REACH,
                Focus::LightSwitch(i),
            );
        }
        if let Some(c) = self.fix.heater {
            consider(c.center(), 0.6, INTERACT_REACH, Focus::Heater);
        }
        if let Some(c) = self.fix.music_box {
            consider(c.center(), 0.5, INTERACT_REACH, Focus::MusicBox);
        }
        for (i, c) in self.fix.vents.iter().enumerate() {
            consider(c.center(), 0.4, INTERACT_REACH, Focus::Vent(i));
        }
        for (i, h) in self.hunters.iter().enumerate() {
            if h.active && matches!(h.gimmick, Gimmick::Zellia(_)) {
                consider(h.pos, 1.6, 1.35, Focus::Shop(i));
            }
        }
        // a door is a wide target: the crosshair on its leaf, from the reach of a switch
        for (i, c) in self.fac.doors() {
            consider(c, 1.0, INTERACT_REACH, Focus::Door(i));
            // flush against the leaf the centre is steeply above the crosshair: aim at the slab in front instead
            if let Some(n) = self.fac.door_nearest(i, self.p.pos).filter(|n| pdist(*n, self.p.pos) < 0.75) {
                consider(n, eye.y.clamp(0.3, 1.8), INTERACT_REACH, Focus::Door(i));
            }
        }
        best.map(|b| b.1)
    }

    pub fn focus_prompt(&self) -> Option<String> {
        if self.shop_open() {
            return Some(
                "ZELLIA'S STORE // [1] COOL -2 // [2] WARM -2 // [3] REPAIR -3 // [4] PESTICIDE -4"
                    .into(),
            );
        }
        let f = self.focus?;
        Some(match f {
            Focus::Relay(_) => "[E] RECOVER RELAY".into(),
            Focus::Battery(_) => "[E] TAKE BATTERY".into(),
            Focus::Pickup(i) => format!("[E] COLLECT {}", self.pickups[i].item.label()),
            Focus::Coin(_) => "[E] TAKE COIN".into(),
            Focus::Locker(i) => {
                if self.hiding_disabled[i] {
                    "BURNT OUT".into()
                } else {
                    "[E] HIDE".into()
                }
            }
            Focus::Safe => "[E] REST AT SAFE ZONE".into(),
            // Godot `route_interactable_3d.gd` prompt: "USE SERVICE LIFT", with the relay target until the floor is cleared
            Focus::Lift => {
                let target = self.fac.relays.len() as i32;
                if self.relays_collected < target {
                    format!("[E] USE SERVICE LIFT \u{2014} NEED {target} RELAY")
                } else {
                    "[E] USE SERVICE LIFT".into()
                }
            }
            Focus::Stairs(i) => {
                let s = self.fac.stairs[i];
                match (s.kind, s.dir < 0) {
                    (LinkKind::Ladder(Cover::Manhole), true) => "[E] OPEN THE MANHOLE // CLIMB DOWN".into(),
                    (LinkKind::Ladder(Cover::Grate), true) => "[E] LIFT THE GRATE // CLIMB DOWN".into(),
                    (LinkKind::Ladder(_), false) => "[E] CLIMB THE LADDER UP".into(),
                    (LinkKind::Stairs, true) => "[E] TAKE STAIRS DOWN".into(),
                    (LinkKind::Stairs, false) => "[E] TAKE STAIRS UP".into(),
                }
            }
            Focus::LightSwitch(_) => "[E] TOGGLE ROOM LIGHT".into(),
            Focus::Heater => match self.heater {
                Heater::Frozen => "[E] THAW HEATER STATION".into(),
                Heater::Thawed => "[E] POWER HEATER STATION".into(),
                Heater::Powered => "[E] POWER DOWN HEATER".into(),
            },
            Focus::MusicBox => "HOLD [E] // WIND MUSIC BOX".into(),
            Focus::Vent(_) => "[E] VENT".into(),
            Focus::Shop(_) => "[E] OPEN ZELLIA'S STORE".into(),
            // Godot `route_interactable_3d.gd` get_prompt: "OPEN DOOR" / "CLOSE DOOR"
            Focus::Door(i) => {
                if self.fac.door_is_open(self.fac.props[i].cell) {
                    "[E] CLOSE DOOR".into()
                } else {
                    "[E] OPEN DOOR".into()
                }
            }
        })
    }

    fn focus_point(&self, f: Focus) -> (f32, f32) {
        match f {
            Focus::Relay(i) => self.fac.relays[i].center(),
            Focus::Battery(i) => self.fac.batteries[i].center(),
            Focus::Pickup(i) => self.pickups[i].pos,
            Focus::Coin(i) => self.fix.coins[i].center(),
            Focus::Locker(i) => self.fac.hiding[i].cell.center(),
            Focus::Safe => self.safe_beacon().unwrap_or(self.p.pos),
            Focus::Lift => self.fac.exit.center(),
            Focus::Stairs(i) => self.fac.stairs[i].cell.center(),
            Focus::LightSwitch(i) => self.fac.room_lights[i].switch_cell.center(),
            Focus::Heater => self.fix.heater.map(|c| c.center()).unwrap_or(self.p.pos),
            Focus::MusicBox => self.fix.music_box.map(|c| c.center()).unwrap_or(self.p.pos),
            Focus::Vent(i) => self.fix.vents[i].center(),
            Focus::Shop(i) => self.hunters[i].pos,
            Focus::Door(i) => self.fac.doors().find(|d| d.0 == i).map_or(self.p.pos, |d| d.1),
        }
    }

    pub(super) fn interact(
        &mut self,
        f: Focus,
        content: &Content,
        catalog: &Catalog,
        sounds: &Sounds,
    ) {
        // Opalie's guard: anything near her post is refused and every hunter searches here
        let at = self.focus_point(f);
        if let Some(name) = self
            .hunters
            .iter()
            .find(|h| h.active && matches!(&h.gimmick, Gimmick::Opalie(o) if o.blocks(at)))
            .map(|h| h.def.name.to_uppercase())
        {
            let pos = self.p.pos;
            for h in self.hunters.iter_mut().filter(|h| h.active) {
                h.enter_search_from(pos, 1.8, &self.fac);
            }
            crate::audio::sound::interaction_fail(); // the interaction veto (player_controller_3d.gd 3886)
            self.say(&format!("{name} IS GUARDING THIS // DRAW HER OFF"));
            return;
        }
        match f {
            Focus::Relay(i) => {
                if self.relays_taken.get(i).copied().unwrap_or(true) {
                    return;
                }
                self.relays_taken[i] = true;
                self.relays_collected += 1;
                self.score += RELAY_SCORE;
                crate::audio::sound::collect();
                self.announce("RELAY ONLINE");
                // a recovered relay saves the run (Godot `_sync_stream_progress`: autosave("relay"))
                self.autosave("relay");
                let need = self
                    .prof()
                    .activation_relays
                    .min(self.fac.relays.len() as i32);
                if self.is_infinite() {
                    // the streamed population grows instead (ensure_hunters)
                    self.ensure_hunters(content);
                } else if !self.hunters[0].active
                    && self.relays_collected >= need
                    && !self.hunters[0].gimmick.runs_while_inactive()
                {
                    let p = self.p.pos;
                    self.hunters[0].activate_at(p);
                }
            }
            Focus::Battery(i) => {
                if self.grant(Item::Battery, 1) {
                    self.batteries_taken[i] = true;
                    crate::audio::sound::collect();
                    self.say("FLASHLIGHT BATTERY");
                } else {
                    crate::audio::sound::interaction_fail();
                    self.say("INVENTORY FULL");
                }
            }
            Focus::Pickup(i) => {
                let (item, n) = (self.pickups[i].item, self.pickups[i].n);
                if self.grant(item, n) {
                    self.pickups[i].taken = true;
                    crate::audio::sound::collect();
                    self.say(item.label());
                } else {
                    crate::audio::sound::interaction_fail();
                    self.say("INVENTORY FULL");
                }
            }
            Focus::Coin(i) => {
                self.coins_taken[i] = true;
                self.coins += 2;
                crate::audio::sound::collect();
                self.say("COIN +2");
            }
            Focus::Locker(i) => self.enter_locker(i, sounds),
            Focus::Safe => {
                self.p.stamina = MAX_STAMINA;
                self.p.lamp = MAX_LAMP_CHARGE;
                self.p.exhausted = false;
                crate::audio::sound::recharge(20.0);
                self.say("Safe zone: lamp and stamina restored.");
            }
            Focus::Stairs(i) => self.start_stairs(i),
            Focus::Lift => {
                if self.relays_collected < self.fac.relays.len() as i32 {
                    crate::audio::sound::interaction_fail();
                    self.say("The service lift has no route signal.");
                    return;
                }
                self.depth += 1;
                self.start_floor_sim(content, catalog);
                self.autosave("level");
                let d = self.depth;
                self.say(&format!("Descending to depth {d}..."));
            }
            Focus::LightSwitch(i) => {
                self.lights_on[i] = !self.lights_on[i];
                self.world.lamps =
                    crate::render::world_mesh::lamps(&self.fac, &self.lights_on, content);
                self.say(if self.lights_on[i] {
                    "ROOM LIGHT ON"
                } else {
                    "ROOM LIGHT OFF"
                });
            }
            Focus::Heater => {
                self.heater = match self.heater {
                    Heater::Frozen => {
                        self.say("Heater station thawed.");
                        Heater::Thawed
                    }
                    Heater::Thawed => {
                        self.say("Heater station powered. The facility is warming.");
                        Heater::Powered
                    }
                    Heater::Powered => {
                        self.say("Heater station powered down.");
                        Heater::Thawed
                    }
                };
            }
            Focus::MusicBox => crate::audio::sound::music_box(), // on the press (route_interactable_3d.gd 818)
            Focus::Vent(i) => self.use_vent(i),
            Focus::Shop(i) => {
                if let Gimmick::Zellia(z) = &mut self.hunters[i].gimmick {
                    z.shop_open = true;
                }
            }
            Focus::Door(i) => self.use_door(i),
        }
    }

    /// Whether the focus is a room door that is shut (the playtest bot and the stress runner press interact on it).
    pub fn closed_door_in_focus(&self) -> bool {
        matches!(self.focus, Some(Focus::Door(i)) if !self.fac.door_is_open(self.fac.props[i].cell))
    }

    /// The interact press on the door at threshold `cell`, for the runner and for a player bot alike: it toggles the
    /// door unless a body (the runner, a hunter or a bot) stands in a closing leaf's sweep.
    pub(super) fn press_door(&mut self, cell: crate::world::Cell) -> Option<DoorToggle> {
        let level = self.runner_level();
        let bodies: Vec<(f32, f32)> = std::iter::once(self.p.pos)
            .chain(self.hunters.iter().filter(|h| h.active).map(|h| h.pos))
            .chain(self.npcs.iter().filter(|n| n.level == level).map(|n| n.at))
            .collect();
        self.fac.toggle_door(cell, &bodies)
    }

    /// The interact press on a room door (Godot `route_interactable_3d.gd` activate): it opens or closes with the
    /// swing the renderer already plays. A leaf does not swing shut through a body standing in its sweep.
    fn use_door(&mut self, i: usize) {
        let cell = self.fac.props[i].cell;
        match self.press_door(cell) {
            Some(DoorToggle::Opened) => crate::audio::sound::door(true),
            Some(DoorToggle::Closed) => crate::audio::sound::door(false),
            Some(DoorToggle::Blocked) => {
                crate::audio::sound::interaction_fail();
                self.say("SOMETHING IS IN THE DOORWAY");
            }
            None => {}
        }
    }

    fn enter_locker(&mut self, i: usize, _sounds: &Sounds) {
        if self.hiding_disabled[i] {
            self.say("BURNT OUT");
            return;
        }
        let lit = self.p.flashlight_on && self.p.lamp > 0.0;
        let mut trapped = false;
        for h in self.hunters.iter_mut().filter(|h| h.active) {
            if let Gimmick::Ceryth(c) = &mut h.gimmick {
                match c.resolve_entry(i, lit) {
                    Entry::Trapped => trapped = true,
                    Entry::Cleared | Entry::Normal => {}
                }
            }
        }
        self.p.hidden = Some(i);
        self.p.flashlight_on = false;
        let l = &self.fac.hiding[i];
        self.p.pos = self.fac.locker_center(l);
        self.p.yaw = (-(l.facing.0 as f32)).atan2(-(l.facing.1 as f32));
        self.p.pitch = 0.0;
        if trapped {
            self.p.hide_lock = 30.0;
            self.say("THE DOOR WON'T OPEN");
        }
    }

    pub(super) fn leave_locker(&mut self, forced_to: Option<(f32, f32)>) {
        if let Some(i) = self.p.hidden.take() {
            self.p.hide_lock = 0.0;
            self.p.pos =
                forced_to.unwrap_or_else(|| crate::hunters::gimmicks::hiding_exit(&self.fac, i));
        }
    }

    /// Verrine's ring: each press hops one grate along; back at the entry grate you leave.
    fn use_vent(&mut self, i: usize) {
        let n = self.fix.vents.len();
        if n == 0 {
            self.say("The grate won't budge.");
            return;
        }
        if !self.p.in_vent {
            self.p.in_vent = true;
            self.p.vent_entry = i;
            self.p.vent_time = 0.0;
            crate::audio::sound::door(true); // the hatch (player_controller_3d.gd 3823)
            let far = self.fix.vents[(i + 2) % n].center();
            for h in self.hunters.iter_mut() {
                if matches!(h.gimmick, Gimmick::Verrine(_)) && h.active {
                    h.pos = far;
                }
            }
        }
        let next = (i + 1) % n;
        if next == self.p.vent_entry {
            self.p.in_vent = false;
            crate::audio::sound::door(false); // the hatch (player_controller_3d.gd 3811)
            self.say("VENT NETWORK EXITED");
            return;
        }
        let (cx, cy) = self.fix.vents[next].center();
        self.p.pos = self.fac.nearest_open_or(cx, cy, 0.27);
        self.say(&format!("VENT TRANSIT // VENT-{}", next + 1));
    }

    /// Held interactions: winding the music box, and Zellia's store choices.
    pub(super) fn update_holds(&mut self, dt: f32, c: &super::Controls, _sounds: &Sounds) {
        if c.interact_held && self.focus == Some(Focus::MusicBox) {
            self.wind_hold += dt;
            if self.wind_hold >= 1.5 {
                self.wind_hold = 0.0;
                self.music_box_left = match self.difficulty {
                    Difficulty::Hard => 35.0,
                    _ => 45.0,
                };
                self.say("Music box wound. The room settles.");
            }
        } else {
            self.wind_hold = 0.0;
        }
        if let Some(choice) = c.shop_choice {
            self.shop_trade(choice);
        }
    }

    fn shop_trade(&mut self, choice: usize) {
        use crate::hunters::gimmicks::zellia::OFFERS;
        let Some(zi) = self
            .hunters
            .iter()
            .position(|h| matches!(&h.gimmick, Gimmick::Zellia(z) if z.shop_open))
        else {
            return;
        };
        if choice >= OFFERS.len() {
            return;
        }
        if let Gimmick::Zellia(z) = &mut self.hunters[zi].gimmick {
            z.shop_open = false;
        }
        let (label, cost) = OFFERS[choice];
        if self.coins < cost {
            self.say(&format!(
                "ZELLIA'S STORE // NOT ENOUGH COINS FOR {label} ({cost})"
            ));
            return;
        }
        self.coins -= cost;
        match choice {
            0 => self.meters.add_temperature(-15.0),
            1 => self.meters.add_temperature(15.0),
            // Native 0x140067e40/0x14008a600: battery recharge is +25 and
            // the lamp meter is capped at its 100-point maximum.
            2 => self.p.lamp = (self.p.lamp + BATTERY_RECHARGE).min(MAX_LAMP_CHARGE),
            _ => self.p.pesticide = self.p.pesticide.max(60.0),
        }
        self.say(&format!(
            "ZELLIA'S STORE // TRADE COMPLETE // {label} // -{cost} COINS"
        ));
        let chance = match &self.hunters[zi].gimmick {
            Gimmick::Zellia(z) => z.death_chance,
            _ => 0.0,
        };
        if self.hunters[zi].next_float() < chance {
            self.say("ZELLIA'S STORE // THE DEAL COST MORE THAN COINS");
            self.end_run(zi, "deal");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;

    #[test]
    fn taking_the_stairs_autosaves_the_new_level() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
        let i = g
            .fac
            .stairs
            .iter()
            .position(|s| crate::world::infinite::level_allowed(s.dir))
            .expect("the start window has a stairwell down");
        let dir = g.fac.stairs[i].dir;
        g.interact(Focus::Stairs(i), &content, &catalog, &Sounds::default());
        for _ in 0..100 {
            if g.transition.is_none() {
                break;
            }
            g.step(0.05, &crate::game::Controls::default(), &content, &catalog, &Sounds::default());
        }
        assert!(g.transition.is_none(), "the climb and the fade finish");
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert_eq!(g.last_autosave, "level", "a new level saves the run");
    }

    #[test]
    fn the_lift_autosaves_the_next_depth() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.relays_collected = g.fac.relays.len() as i32;
        let depth = g.depth;
        g.interact(Focus::Lift, &content, &catalog, &Sounds::default());
        assert_eq!(g.depth, depth + 1);
        assert_eq!(g.last_autosave, "level", "the next depth saves the run");
    }

    #[test]
    fn the_lift_names_the_relay_target_until_the_floor_is_cleared() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        assert!(g.fac.has_lift && !g.fac.relays.is_empty());
        let target = g.fac.relays.len();
        g.focus = Some(Focus::Lift);
        g.relays_collected = 0;
        assert_eq!(
            g.focus_prompt().as_deref(),
            Some(format!("[E] USE SERVICE LIFT \u{2014} NEED {target} RELAY").as_str())
        );
        g.relays_collected = target as i32;
        assert_eq!(g.focus_prompt().as_deref(), Some("[E] USE SERVICE LIFT"));
    }

    #[test]
    fn the_lift_stays_put_without_route_signal() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.relays_collected = g.fac.relays.len() as i32 - 1;
        let (depth, saved) = (g.depth, g.last_autosave);
        g.interact(Focus::Lift, &content, &catalog, &Sounds::default());
        assert_eq!(g.depth, depth, "no descent short of the relay target");
        assert_eq!(g.last_autosave, saved, "a refused lift does not save");
    }
}
