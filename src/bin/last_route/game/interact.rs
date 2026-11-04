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
    /// A sewer ladder, by its index in `Facility::stairs` (a stair flight is walked and is never a focus).
    Ladder(usize),
    LightSwitch(usize),
    Heater,
    MusicBox,
    Vent(usize),
    Shop(usize),
    /// A room door, by its index in `Facility::props`.
    Door(usize),
    /// A sewer valve wheel, by its index in `Facility::props` (game/pipes.rs).
    Valve(usize),
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
            // a stair flight is walked, never asked for (D3 of docs/STAIRS_CONTINUOUS_DESIGN.md): no focus, no prompt
            if !s.is_ladder() {
                continue;
            }
            // the ladder's first step, just past the landing
            let (cx, cy) = s.cell.center();
            consider(
                (cx + s.run.0 as f32 * 0.9, cy + s.run.1 as f32 * 0.9),
                0.3,
                2.0,
                Focus::Ladder(i),
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
        // sewer valve wheels: the prop type is data (data/pipes.json)
        let valve_type = super::pipes::rules().valve.prop_type.as_str();
        for (i, p) in self.fac.props.iter().enumerate() {
            if p.ptype == valve_type {
                consider(p.cell.center(), 1.0, INTERACT_REACH, Focus::Valve(i));
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
            Focus::Ladder(i) => {
                let s = self.fac.stairs[i];
                match (s.kind, s.dir < 0) {
                    (LinkKind::Ladder(Cover::Manhole), true) => "[E] OPEN THE MANHOLE // CLIMB DOWN".into(),
                    (LinkKind::Ladder(Cover::Grate), true) => "[E] LIFT THE GRATE // CLIMB DOWN".into(),
                    (LinkKind::Ladder(_), false) => "[E] CLIMB THE LADDER UP".into(),
                    // a flight is walked: it is never a focus, so this arm cannot be reached
                    (LinkKind::Stairs, _) => String::new(), // a stair flight is never a focus (D3): no prompt
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
            Focus::Valve(i) => {
                let p = &self.fac.props[i];
                self.pipes.prompt(&p.id, &valve_keys(p).0)
            }
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
            Focus::Ladder(i) => self.fac.stairs[i].cell.center(),
            Focus::LightSwitch(i) => self.fac.room_lights[i].switch_cell.center(),
            Focus::Heater => self.fix.heater.map(|c| c.center()).unwrap_or(self.p.pos),
            Focus::MusicBox => self.fix.music_box.map(|c| c.center()).unwrap_or(self.p.pos),
            Focus::Vent(i) => self.fix.vents[i].center(),
            Focus::Valve(i) => self.fac.props[i].cell.center(),
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
                self.score += crate::game::progression::Progression::load().relay_score;
                crate::audio::sound::collect();
                self.announce("RELAY ONLINE");
                // a recovered relay saves the run (Godot `_sync_stream_progress`: autosave("relay"))
                self.autosave("relay");
                let need = crate::game::progression::Progression::load()
                    .activation_relays(self.difficulty)
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
            Focus::Ladder(i) => {
                // only a ladder is interacted with; a flight is walked
                if self.fac.stairs[i].is_ladder() {
                    self.start_ladder(i);
                }
            }
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
            // the turn is a hold (`update_valve_hold`), not a press
            Focus::Valve(_) => {}
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

    /// The valve wheel held for `Valve::hold_seconds` (data/pipes.json) turns. Called every frame with whether interact is
    /// held: the hold counts up while the crosshair stays on a wheel and resets when it leaves or is released. Returns
    /// true on the frame the turn is made. `valve_spin` counts the wheel's spin down for the renderer.
    pub(super) fn update_valve_hold(&mut self, dt: f32, held: bool) -> bool {
        let target = match self.focus {
            Some(Focus::Valve(i)) if held && self.p.stun <= 0.0 => Some(i),
            _ => None,
        };
        let hold_seconds = super::pipes::rules().valve.hold_seconds;
        let Some(inf) = self.inf.as_mut() else { return false };
        inf.valve_spin = (inf.valve_spin - dt).max(0.0);
        let Some(i) = target else {
            inf.valve_hold = 0.0;
            return false;
        };
        inf.valve_hold += dt;
        if inf.valve_hold < hold_seconds {
            return false;
        }
        inf.valve_hold = 0.0;
        self.turn_valve(i)
    }

    /// The runner turns the valve wheel `props[i]`: the pipe network decides (reach and storey), the turn is applied here,
    /// the runner's noise is a normal noise burst (not a danger cue) and the event is queued for the net layer
    /// (`take_valve_events`). Returns whether the wheel turned.
    pub(super) fn turn_valve(&mut self, i: usize) -> bool {
        let Some(level) = self.inf.as_ref().map(|inf| inf.level) else { return false };
        let Some(prop) = self.fac.props.get(i) else { return false };
        let (effect, gate) = valve_keys(prop);
        let q = super::pipes::TurnAt {
            valve_id: &prop.id,
            effect: &effect,
            gate: &gate,
            valve_at: prop.cell.center(),
            valve_storey: level,
            at: self.p.pos,
            storey: level,
            by_player: true,
        };
        match self.pipes.turn(q) {
            Ok(ev) => {
                self.pipes.apply(&ev, self.time);
                let at = self.p.pos;
                self.push_noise(crate::hunters::noise_events::NoiseKind::ValveTurn, at);
                if !ev.gate.is_empty() && ev.turns % 2 == 1 {
                    self.push_noise(crate::hunters::noise_events::NoiseKind::SluiceOpen, at);
                }
                if !self.remote_hunters {
                    self.hunter_out.emit(
                        crate::net::hunter_events::rules(),
                        self.difficulty,
                        crate::net::valve_events::VALVE,
                        crate::net::valve_events::turn_fields(&ev),
                        std::time::Instant::now(),
                    );
                }
                if let Some((radius, seconds)) = ev.noise {
                    self.p.noise_burst = radius;
                    self.p.noise_burst_timer = seconds;
                }
                let spin = super::pipes::rules().valve.turn_seconds;
                if let Some(inf) = self.inf.as_mut() {
                    inf.valve_spin = spin;
                    inf.valve_outbox.push(ev);
                }
                crate::audio::sound::door(true);
                true
            }
            Err(_) => {
                crate::audio::sound::interaction_fail();
                false
            }
        }
    }

    /// The valve turns made on this machine, to be sent by the net layer; the queue is emptied.
    pub fn take_valve_events(&mut self) -> Vec<super::pipes::TurnEvent> {
        self.inf.as_mut().map(|inf| std::mem::take(&mut inf.valve_outbox)).unwrap_or_default()
    }

    /// A valve turn published by the host (or echoed to this machine): applied here the same way. Replaying one is harmless.
    pub fn apply_valve_event(&mut self, ev: &super::pipes::TurnEvent) {
        self.pipes.apply(ev, self.time);
    }

    /// Held interactions: winding the music box, and Zellia's store choices.
    pub(super) fn update_holds(&mut self, dt: f32, c: &super::Controls, _sounds: &Sounds) {
        self.update_valve_hold(dt, c.interact_held);
        if c.interact_held && self.focus == Some(Focus::MusicBox) {
            self.wind_hold += dt;
            if self.wind_hold >= 1.5 {
                self.wind_hold = 0.0;
                self.music_box_left = match self.difficulty {
                    Difficulty::Hard => 35.0,
                    Difficulty::Nightmare => 25.0,
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

/// The `valve_effect` and `gate` keys of a valve wheel prop. The facility prop does not carry them yet, so a wheel is a
/// flood valve with no gate until it does (`""` is the flood effect, game/pipes.rs).
fn valve_keys(_prop: &crate::world::facility::Prop) -> (String, String) {
    (String::new(), String::new())
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
            .position(|s| s.is_ladder() && s.dir < 0)
            .expect("the start window has a ladder down");
        let dir = g.fac.stairs[i].dir;
        // a ladder is climbed by hand (game/ladder.rs): hold back to go down the shaft until the storey changes
        g.interact(Focus::Ladder(i), &content, &catalog, &Sounds::default());
        let back = crate::game::Controls { fwd: -1.0, ..Default::default() };
        for _ in 0..400 {
            if g.last_autosave == "level" {
                break;
            }
            g.step(0.05, &back, &content, &catalog, &Sounds::default());
        }
        // the autosave is the label change (relabel), not an interact: it happens when the body's height reaches the new storey
        assert_eq!(g.last_autosave, "level", "the label change saves the run");
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert!((g.p.y() - crate::game::level_stack::floor_y_of(dir)).abs() < 0.5, "the label flips at the new storey's floor");
        for _ in 0..400 {
            if g.ladder.is_none() {
                break;
            }
            g.step(0.05, &back, &content, &catalog, &Sounds::default());
        }
        assert!(g.ladder.is_none(), "the climb ends by dismount");
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert_eq!(g.last_autosave, "level", "a new level saves the run");
    }

    /// The room texts of the golden seeds (world/stack_contract_tests.rs GOLDEN).
    const GOLDEN: [&str; 3] = ["1426541270", "k7qxm", "demo"];

    /// Every stair-record focus to ask `interact` for: the old stairs focus was an index into `Facility::stairs`, so a
    /// regression would show as `interact(Focus::Ladder(j))` on a FLIGHT index moving the body. (Other focus kinds, such as the
    /// lift, move the body by design and are not what this test is about.)
    fn every_focus(g: &Game) -> Vec<Focus> {
        (0..g.fac.stairs.len()).map(Focus::Ladder).collect()
    }

    /// Both ends of every walked flight of the golden windows, each with the body standing there.
    fn for_each_flight_end(mut check: impl FnMut(&mut Game, &Content, &Catalog, &str)) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut checked = 0;
        for text in GOLDEN {
            for level in [0, -1, -2] {
                let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, text, level, &content, &catalog);
                let flights: Vec<_> = g.fac.stairs.iter().copied().filter(|s| !s.is_ladder()).collect();
                for s in flights {
                    // the stair record sits on this level (A above, B below); its far end is one level over
                    for (cell, lvl) in [(s.cell, level), (s.far_end(), level + s.dir)] {
                        if !crate::world::infinite::level_allowed(lvl) {
                            continue;
                        }
                        g.go_to_level(lvl, cell.center(), 0.0, &content, &catalog);
                        g.fade = 0.0;
                        check(&mut g, &content, &catalog, text);
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "the golden windows have a walked flight");
    }

    #[test]
    fn stairs_cannot_be_used_to_teleport() {
        for_each_flight_end(|g, content, catalog, text| {
            let level = g.inf.as_ref().unwrap().level;
            assert!(
                !matches!(g.find_focus(), Some(Focus::Ladder(j)) if !g.fac.stairs[j].is_ladder()),
                "{text}: a flight is no focus"
            );
            // absolute cells: the window origin can shift on a step, which moves the window-relative position
            let at = |g: &Game| (g.p.pos.0 + g.fac.origin.0 as f32, g.p.pos.1 + g.fac.origin.1 as f32);
            let (pos, y) = (at(g), g.p.y());
            for f in every_focus(g) {
                if let Focus::Ladder(j) = f {
                    // a ladder may be mounted; the check is about flights
                    if g.fac.stairs[j].is_ladder() {
                        continue;
                    }
                }
                g.interact(f, content, catalog, &Sounds::default());
                let now = at(g);
                let moved = (now.0 - pos.0).hypot(now.1 - pos.1);
                assert!(moved < PLAYER_SPRINT_SPEED / 60.0, "{text}: interact({f:?}) moved the body {moved} m");
            }
            let c = crate::game::Controls { interact: true, ..Default::default() };
            let dt = 1.0 / 60.0;
            g.step(dt, &c, content, catalog, &Sounds::default());
            let now = at(g);
            let moved = (now.0 - pos.0).hypot(now.1 - pos.1);
            assert!(moved < PLAYER_SPRINT_SPEED * dt, "{text}: the body moved {moved} m in one frame with only interact held");
            assert_eq!(g.inf.as_ref().unwrap().level, level, "{text}: interact changed the level");
            // the body is set down at height 0 on a cell that may be a tread, so it eases onto the surface (a fraction of a metre in one
            // frame, up to about a metre on a steep tread); a teleport to the other floor would be a whole storey (5 m)
            assert!((g.p.y() - y).abs() < crate::world::vertical::STOREY * 0.5, "{text}: interact changed the height by {} m", g.p.y() - y);
        });
    }

    #[test]
    fn a_flight_has_no_prompt_at_either_end() {
        for_each_flight_end(|g, _, _, text| {
            assert!(g.find_focus().map_or(true, |f| !matches!(f, Focus::Ladder(j) if !g.fac.stairs[j].is_ladder())), "{text}");
            g.focus = g.find_focus();
            if let Some(Focus::Ladder(_)) = g.focus {
                return; // a ladder prompt near the flight is the ladder's own
            }
            let prompt = g.focus_prompt().unwrap_or_default();
            assert!(!prompt.to_uppercase().contains("STAIR"), "{text}: prompt {prompt:?} at a flight end");
        });
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

    fn valve_prop(id: &str, at: (i32, i32)) -> crate::world::facility::Prop {
        crate::world::facility::Prop {
            id: id.to_string(),
            cell: crate::world::Cell::new(at.0, at.1),
            ptype: crate::game::pipes::rules().valve.prop_type.clone(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.3, 0.3),
            offset: (0.0, 0.0),
            door: None,
        }
    }

    fn valve_game() -> (Game, usize, String) {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-VALVE-7", 0, &content, &catalog);
        let id = "SEWER:0:0:0:PROP:1".to_string();
        let at = (g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        g.fac.props.push(valve_prop(&id, at));
        let i = g.fac.props.len() - 1;
        g.focus = Some(Focus::Valve(i));
        (g, i, id)
    }

    #[test]
    fn a_valve_wheel_turns_only_after_the_hold_and_makes_normal_noise() {
        let (mut g, _, id) = valve_game();
        let hold = crate::game::pipes::rules().valve.hold_seconds;
        assert!(!g.update_valve_hold(hold * 0.4, true));
        assert!(!g.update_valve_hold(hold * 0.4, true), "not yet held long enough");
        assert_eq!(g.pipes.turns(&id), 0);
        assert!(g.update_valve_hold(hold * 0.4, true), "the hold is complete");
        assert_eq!(g.pipes.turns(&id), 1, "the wheel turned once");
        let v = &crate::game::pipes::rules().valve;
        assert_eq!((g.p.noise_burst, g.p.noise_burst_timer), (v.noise, v.turn_seconds), "the runner's noise burst");
        let events = g.take_valve_events();
        assert_eq!(events.len(), 1);
        assert!(events[0].noise.is_some());
        assert!(g.take_valve_events().is_empty(), "the queue is emptied");
    }

    #[test]
    fn releasing_or_looking_away_resets_the_valve_hold() {
        let (mut g, i, id) = valve_game();
        let hold = crate::game::pipes::rules().valve.hold_seconds;
        assert!(!g.update_valve_hold(hold * 0.9, true));
        assert!(!g.update_valve_hold(0.01, false), "released");
        assert!(!g.update_valve_hold(hold * 0.9, true), "the hold started again from zero");
        g.focus = None;
        assert!(!g.update_valve_hold(hold * 0.9, true));
        g.focus = Some(Focus::Valve(i));
        assert!(!g.update_valve_hold(hold * 0.9, true));
        assert_eq!(g.pipes.turns(&id), 0);
    }

    #[test]
    fn a_valve_out_of_reach_does_not_turn() {
        let (mut g, i, id) = valve_game();
        g.p.pos.0 += crate::game::pipes::rules().valve.reach + 2.0;
        assert!(!g.turn_valve(i));
        assert_eq!(g.pipes.turns(&id), 0);
        assert!(g.take_valve_events().is_empty());
    }

    #[test]
    fn the_valve_prompt_comes_from_the_pipe_data() {
        let (g, _, id) = valve_game();
        assert_eq!(g.focus_prompt(), Some(g.pipes.prompt(&id, "")));
        assert!(g.focus_prompt().is_some_and(|p| !p.is_empty()));
    }
}
