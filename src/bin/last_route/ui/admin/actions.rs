//! What the admin rows do to the run (infinite_route_test.gd's `_admin_*` handlers and
//! `_accept_stream_admin`), on the port's Game: creative inventory, teleports, run speed, the
//! player toggles, the hunter picker and the all-hunter actions, game speed, relays, region
//! lights, seed / room code copy, Ravelle traps, the debug view, safe teleport, and the STATUS
//! readout (`_admin_status`). Rows for systems this build does not have (room chat, the admin
//! roster server, NPC teammates, the story director, waypoints, safe-room integrity, emotes)
//! answer with a HUD line instead, as the original does when the system is unavailable.

use super::menu::Cmd;
use super::menu_data::menu;
use super::{Admin, Request, View};
use crate::config::*;
use crate::content::Content;
use crate::game::{npc::NAV_RADIUS, pdist, Game, Item, Slot};
use crate::hunters::brain::{Brain, State};
use crate::hunters::gimmicks::Gimmick;
use crate::world::infinite::CHUNK;

/// ADMIN_HUNTER_RING_START / _STEP / _ANGLE: summons spread on a golden-angle ring round the runner.
const RING_START: f32 = 2.5;
const RING_STEP: f32 = 1.2;
const RING_ANGLE: f32 = 2.39996;
/// ADMIN_SAFE_SEARCH: how far (cells) a teleport looks for open floor.
const SAFE_SEARCH: i32 = 16;

/// NPC TELEPORT TO ME: the NPC lands on the open floor nearest the runner, on the runner's level.
/// `shift` moves it and drops its path, so it replans from there.
fn npc_teleport(g: &mut Game, i: usize) -> String {
    let runner = g.p.pos;
    let level = g.runner_level();
    let Some(spot) = g.fac.nearest_open(runner.0, runner.1, NAV_RADIUS) else {
        return "ADMIN // NO OPEN FLOOR NEAR YOU".to_string();
    };
    let n = &mut g.npcs[i];
    let d = (spot.0 - n.at.0, spot.1 - n.at.1);
    n.shift(d);
    n.level = level;
    format!("ADMIN // NPC TELEPORTED // {}", n.name)
}

/// NPC FOLLOW ME: the NPC joins the runner again (an admin NPC follows from the moment it spawns, and
/// STOP FOLLOWING parks it; across levels it walks the stairs to the runner, game/npc.rs).
fn npc_follow(g: &mut Game, i: usize) -> String {
    let n = &mut g.npcs[i];
    n.joined = true;
    format!("ADMIN // {} FOLLOWS YOU", n.name)
}

/// NPC STOP FOLLOWING: the NPC stands where it is. An admin NPC has no join radius, so it stays put until
/// FOLLOW ME or TELEPORT TO ME.
fn npc_unfollow(g: &mut Game, i: usize) -> String {
    let n = &mut g.npcs[i];
    n.joined = false;
    format!("ADMIN // {} STAYS HERE", n.name)
}

/// NPC SMART = SPAWN SMART: the NPC takes the smartness the SMART dropdown shows.
fn npc_set_smart(g: &mut Game, i: usize, smart: i32) -> String {
    let n = &mut g.npcs[i];
    n.smart = smart.clamp(0, 100);
    format!("ADMIN // {} SMART {}", n.name, n.smart)
}

/// The NPC commands this build runs, by id (`npc_live` in data/admin/menu.json). The rest of the grid
/// (`npc_pending`) is drawn gray and answers with a line that names the command.
pub(super) fn npc_op_live(op: &str) -> bool {
    menu().npc_live.contains(&op)
}

/// How an NPC grid button is drawn and what its tooltip says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NpcButton {
    /// A live command and there is an NPC to act on.
    Ready,
    /// A live command, but the run has no NPC: gray until one is spawned.
    NoNpcs,
    /// The build cannot do this yet; the text says what is missing (data/admin/menu.json `npc_pending`).
    Pending(&'static str),
}

/// The state of the NPC grid button `op` in a run with `npcs` NPCs. Only a missing NPC or a missing feature grays one.
pub(super) fn npc_button(op: &str, npcs: usize) -> NpcButton {
    if let Some((_, reason)) = menu().npc_pending.iter().find(|(id, _)| *id == op) {
        NpcButton::Pending(reason)
    } else if npcs == 0 {
        NpcButton::NoNpcs
    } else {
        NpcButton::Ready
    }
}

impl NpcButton {
    /// The tooltip of a gray button (the hint line under the menu); None for a ready one.
    pub(super) fn tip(self) -> Option<String> {
        match self {
            NpcButton::Ready => None,
            NpcButton::NoNpcs => Some(menu().npc_no_npcs.to_string()),
            NpcButton::Pending(reason) => Some(format!("{}{}", menu().npc_pending_prefix, reason)),
        }
    }
}

/// The label of an NPC command id, for the "not in this build" line.
fn npc_op_label(op: &str) -> &'static str {
    let m = menu();
    m.npc_ops.iter().chain(m.npc_all.iter()).find(|(id, _)| *id == op).map_or("THAT COMMAND", |(_, l)| l)
}

/// NPC REMOVE: takes the picked NPC off the floor. Nothing else holds an NPC's index.
fn npc_remove(g: &mut Game, i: usize) -> String {
    let name = g.npcs.remove(i).name;
    format!("ADMIN // NPC REMOVED // {name}")
}

/// _admin_safe_landing: the nearest cell (rings outward) a runner fits in, else None.
pub(super) fn safe_landing(g: &Game, x: f32, y: f32) -> Option<(f32, f32)> {
    let (cx, cy) = (x.floor() as i32, y.floor() as i32);
    for radius in 0..=SAFE_SEARCH {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dy.abs()) != radius {
                    continue;
                }
                let p = ((cx + dx) as f32 + 0.5, (cy + dy) as f32 + 0.5);
                if g.fac.body_fits(p.0, p.1, PLAYER_RADIUS) {
                    return Some(p);
                }
            }
        }
    }
    None
}

impl Admin {
    pub(super) fn is_toggle(&self, id: &str) -> bool {
        matches!(
            id,
            "no_target"
                | "god_mode"
                | "noclip"
                | "region_lights"
                | "show_traps"
                | "light_sight"
                | "map_reveal"
                | "region_map"
                | "show_collision"
                | "hunter_reveal"
                | "perf_overlay"
                | "fullbright"
                | "wireframe"
        )
    }

    pub(super) fn state_of(&self, id: &str) -> bool {
        match id {
            "no_target" => self.no_target,
            "god_mode" => self.god_mode,
            "noclip" => self.noclip,
            "region_lights" => self.lights_all_on,
            "show_traps" => self.show_traps,
            "light_sight" => self.light_sight,
            "map_reveal" => self.map_reveal,
            "region_map" => self.region_map,
            "show_collision" => self.show_collision,
            "hunter_reveal" => self.hunter_reveal,
            "perf_overlay" => self.perf_overlay,
            "fullbright" => self.view == View::Fullbright,
            "wireframe" => self.view == View::Wireframe,
            _ => false,
        }
    }

    pub(super) fn toggle(&mut self, id: &str, g: &mut Game, content: Option<&Content>) {
        match id {
            "no_target" => {
                self.no_target = !self.no_target;
                self.prefs.set("no_target", self.no_target);
            }
            "god_mode" => {
                self.god_mode = !self.god_mode;
                self.prefs.set("god_mode", self.god_mode);
            }
            "noclip" => {
                self.noclip = !self.noclip;
                self.prefs.set("noclip", self.noclip);
            }
            "region_lights" => {
                // facility.set_all_lights(not facility.all_lights_enabled())
                let on = !g.lights_on.iter().all(|l| *l);
                for l in g.lights_on.iter_mut() {
                    *l = on;
                }
                if let Some(content) = content {
                    g.world.lamps = crate::render::world_mesh::lamps(&g.fac, &g.lights_on, content);
                }
            }
            "show_traps" => self.show_traps = !self.show_traps,
            "light_sight" => {
                self.light_sight = !self.light_sight;
                g.say(&format!(
                    "MUTUAL LIGHT SIGHT {}",
                    if self.light_sight { "ON" } else { "OFF" }
                ));
            }
            "map_reveal" => {
                self.map_reveal = !self.map_reveal;
                self.prefs.set("map_reveal", self.map_reveal);
            }
            "region_map" => {
                self.region_map = !self.region_map;
                self.prefs.set("region_map", self.region_map);
            }
            "show_collision" => {
                self.show_collision = !self.show_collision;
                self.prefs.set("show_collision", self.show_collision);
            }
            "hunter_reveal" => {
                self.hunter_reveal = !self.hunter_reveal;
                self.prefs.set("hunter_reveal", self.hunter_reveal);
            }
            "perf_overlay" => {
                self.perf_overlay = !self.perf_overlay;
                self.prefs.set("perf_overlay", self.perf_overlay);
            }
            "fullbright" => {
                self.view = if self.view == View::Fullbright {
                    View::Off
                } else {
                    View::Fullbright
                }
            }
            "wireframe" => {
                self.view = if self.view == View::Wireframe {
                    View::Off
                } else {
                    View::Wireframe
                }
            }
            _ => {}
        }
        self.mark_debug();
    }

    /// The NPC the NPC picker shows, as an index into `g.npcs`, clamped the way the picker draws it.
    pub(super) fn picked_npc(&self, g: &Game) -> Option<usize> {
        let n = g.npcs.len();
        (n > 0).then(|| self.sel_of("npc_pick").min(n - 1))
    }

    pub(super) fn run_cmd(
        &mut self,
        cmd: Cmd,
        g: &mut Game,
        content: &Content,
        hunters: &[(String, String)],
    ) {
        let line = match &cmd {
            Cmd::Toggle(id) => format!("admin: toggle {id} to {}", !self.state_of(id)),
            Cmd::Run(key, i) => format!("admin: run {key} index {i}"),
            Cmd::Hunter(h, op, emote) => {
                let id = hunters.get(*h).map_or("?", |(id, _)| id.as_str());
                format!("admin: hunter {id} op {op} emote {emote}")
            }
            Cmd::Npc(op) => match self.picked_npc(g) {
                Some(i) if matches!(*op, "teleport" | "follow" | "unfollow" | "smart" | "remove") => {
                    format!("admin: npc {op} {}", g.npcs[i].name)
                }
                _ => format!("admin: npc {op}"),
            },
            Cmd::Field(i) => format!("admin: field {i}"),
        };
        crate::lr_info!("{line}");
        match cmd {
            Cmd::Toggle(id) => self.toggle(id, g, Some(content)),
            Cmd::Run(key, i) => self.run_row(key, i, g, content),
            Cmd::Hunter(h, op, emote) => {
                let Some((id, name)) = hunters.get(h).cloned() else {
                    return;
                };
                match op {
                    0 => self.summon(g, content, &[id]),
                    1 => self.remove_hunter(g, &id, &name),
                    2 => {
                        if g.hunters.iter().any(|b| b.def.id == id) {
                            self.remove_hunter(g, &id, &name);
                        } else {
                            self.summon(g, content, &[id]);
                        }
                    }
                    _ => {
                        let near = g
                            .hunters
                            .iter()
                            .filter(|b| b.active && pdist(b.pos, g.p.pos) <= 40.0)
                            .count();
                        g.say(&format!(
                            "HUNTER EMOTE // {} // {near} HUNTERS (NO EMOTE CLIPS IN THIS BUILD)",
                            menu().emotes.get(emote).unwrap_or(&"?")
                        ));
                    }
                }
                self.mark_debug();
            }
            Cmd::Npc(op) => {
                let smart = menu().npc_smarts[self.sel_of("npc_smart").min(menu().npc_smarts.len() - 1)];
                let picked = self.picked_npc(g);
                // offline: the NPCs live on the game, no server (game/npc.rs). Spawn, the removes, teleport,
                // follow / stop following and smart are wired (per NPC and for all); freeze, go to, ignore,
                // target, chat, down, revive, kill, clear and spectate need systems the NPCs do not have yet.
                let line = match op {
                    "spawn" => {
                        let eye = g.eye();
                        let (fwd, _, _) = g.camera_basis();
                        let n = g.npc_spawn(
                            self.npc_count,
                            smart,
                            (eye.x, eye.y, eye.z),
                            (fwd.x, fwd.y, fwd.z),
                        );
                        format!("ADMIN // {n} NPC(S) SPAWNED")
                    }
                    "remove_nearest" => match g.npc_remove_nearest() {
                        Some(name) => format!("ADMIN // NPC REMOVED // {name}"),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "remove_all" => match g.npc_remove_all() {
                        0 => "ADMIN // NO NPCS".to_string(),
                        n => format!("ADMIN // {n} NPCS REMOVED"),
                    },
                    "teleport" => match picked {
                        Some(i) => npc_teleport(g, i),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "follow" => match picked {
                        Some(i) => npc_follow(g, i),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "unfollow" => match picked {
                        Some(i) => npc_unfollow(g, i),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "smart" => match picked {
                        Some(i) => npc_set_smart(g, i, smart),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "remove" => match picked {
                        Some(i) => npc_remove(g, i),
                        None => "ADMIN // NO NPCS".to_string(),
                    },
                    "all_teleport" | "all_follow" | "all_unfollow" | "all_smart" if g.npcs.is_empty() => {
                        "ADMIN // NO NPCS".to_string()
                    }
                    "all_teleport" => {
                        for i in 0..g.npcs.len() {
                            npc_teleport(g, i);
                        }
                        format!("ADMIN // {} NPCS TELEPORTED", g.npcs.len())
                    }
                    "all_follow" | "all_unfollow" => {
                        let follow = op == "all_follow";
                        for n in g.npcs.iter_mut() {
                            n.joined = follow;
                        }
                        format!(
                            "ADMIN // {} NPCS {}",
                            g.npcs.len(),
                            if follow { "FOLLOW YOU" } else { "STAY HERE" }
                        )
                    }
                    "all_smart" => {
                        for n in g.npcs.iter_mut() {
                            n.smart = smart.clamp(0, 100);
                        }
                        format!("ADMIN // {} NPCS SMART {smart}", g.npcs.len())
                    }
                    other => format!("ADMIN // {} IS NOT IN THIS BUILD YET", npc_op_label(other)),
                };
                g.say(&line);
                self.mark_debug();
            }
            Cmd::Field(i) => {
                let text = self.fields[i].trim().to_string();
                match i {
                    0 => self.teleport_coordinates(g, content, &text),
                    // an owner's RESET PASSWORD: the app sends it (Admin::run_password_reset) and the answer opens a message box
                    super::password::RESET_FIELD => {
                        let line = self.ask_password_reset(&text);
                        g.say(&line);
                    }
                    _ => g.say(&format!(
                        "ADMIN ROSTER FAILED // {} NEEDS THE PRIMARY SERVER",
                        menu().fields[i].0
                    )),
                }
                if i == 0 {
                    self.mark_debug();
                }
            }
        }
    }

    fn run_row(&mut self, key: &str, i: usize, g: &mut Game, content: &Content) {
        let mut debug = true;
        match key {
            "give_item" => self.give_item(g, i),
            "teleport" => self.teleport_chunk(g, menu().teleports[i.min(menu().teleports.len() - 1)].0),
            "run_speed" => self.run_speed = menu().run_speeds[i.min(menu().run_speeds.len() - 1)].0,
            "game_speed" => {
                let s = menu().game_speeds[i.min(menu().game_speeds.len() - 1)].0;
                if s < 0.0 {
                    self.game_speed = 0.0;
                    self.step_frames = 1;
                } else {
                    self.game_speed = s;
                }
            }
            "story_start" => g.say(&format!(
                "ADMIN STORY FAILED // {} // NO STORY DIRECTOR IN THIS BUILD",
                menu().stories[i.min(2)].1
            )),
            "act_general" => match i {
                0 => {
                    debug = false;
                    g.say("ADMIN ROSTER FAILED // THE ROSTER LIVES ON THE PRIMARY SERVER");
                }
                1 => {
                    debug = false;
                    self.request = Some(Request::RestartNewSeed);
                }
                2 => {
                    debug = false;
                    self.request = Some(Request::RestartSameSeed);
                }
                3 => {
                    debug = false;
                    self.request = Some(Request::EndRoute);
                }
                4 | 6 => {
                    debug = false;
                    g.say("ROOM CHAT // NO GUEST HAS SPOKEN HERE YET");
                }
                _ => {
                    debug = false;
                    g.say("ROOM CHAT UNMUTED");
                }
            },
            "act_player" => match i {
                0 => g.p.lamp = MAX_LAMP_CHARGE,
                1 => self.restore_player(g),
                _ => {
                    g.p.stun = 0.0;
                    g.taser_cd = 0.0;
                    g.melee_cd = 0.0;
                }
            },
            "act_hunters" => self.all_hunters(g, content, i),
            "act_world" => match i {
                0 => self.grant_relays(g, false),
                1 => self.grant_relays(g, true),
                2 | 3 => {
                    // Godot _admin_edit_checkpoint: -25 / +25 on the runner's safe room
                    let amount = if i == 2 { -25 } else { 25 };
                    match g.admin_edit_checkpoint(amount) {
                        Some(s) => g.say(&format!(
                            "ADMIN // CHECKPOINT {}%{}",
                            s.integrity,
                            if s.breached { " BREACHED" } else { "" }
                        )),
                        None => g.say("ADMIN // NO CHECKPOINT HERE: STAND IN A SAFE ROOM"),
                    }
                }
                4 => {
                    debug = false;
                    crate::ui::clipboard::copy(&g.run_seed.to_string());
                    g.say("SEED COPIED");
                }
                _ => {
                    debug = false;
                    crate::ui::clipboard::copy(
                        crate::world::route::room_text_code(&g.room_text),
                    );
                    g.say("ROOM CODE COPIED");
                }
            },
            "act_story" => match i {
                0 => g.say("ADMIN STORY // NOTHING ACTIVE"),
                _ => g.say("ADMIN STORY STATE RESET"),
            },
            "act_visual" => match i {
                0 | 1 => {
                    debug = false;
                    g.say("ADMIN // WAYPOINTS ARE NOT IN THIS BUILD YET");
                }
                2 => self.spawn_ravelle_trap(g),
                3 => g.say(&format!(
                    "LIGHT // HERE {:.2} (VISIBLE AT 0.36)",
                    g.meters.exposure
                )),
                _ => {}
            },
            _ => {}
        }
        if debug {
            self.mark_debug();
        }
    }

    /// _admin_give_item / grant_inventory_item: stacks first, then the first empty slot.
    fn give_item(&mut self, g: &mut Game, i: usize) {
        let (id, label) = menu().items[i.min(menu().items.len() - 1)];
        let (item, n) = match id {
            "flashlight" => (Item::Flashlight, 100),
            "taser" => (Item::Taser, 1),
            "crowbar" => (Item::Crowbar, CROWBAR_DURABILITY),
            "rescue_axe" => (Item::Axe, AXE_DURABILITY),
            "battery" => (Item::Battery, 1),
            "stim_pack" => (Item::Stim, 1),
            _ => {
                g.say(&format!(
                    "ADMIN GIVE FAILED // {label} IS NOT IN THIS BUILD"
                ));
                return;
            }
        };
        let mut done = false;
        if item.stack_max() > 1 {
            for s in g.slots.iter_mut().flatten() {
                if s.item == item && s.n < item.stack_max() {
                    s.n += 1;
                    done = true;
                    break;
                }
            }
        }
        if !done {
            if let Some(slot) = g.slots.iter().position(|s| s.is_none()) {
                g.slots[slot] = Some(Slot { item, n });
                done = true;
            }
        }
        g.say(&if done {
            format!("ADMIN GIVE // {label}")
        } else {
            "ADMIN GIVE FAILED // INVENTORY FULL".to_string()
        });
    }

    /// _admin_restore_player: out of hiding, back at the safe zone, unstunned, cooldowns clear,
    /// and the safe zone's refill (lamp and stamina).
    fn restore_player(&mut self, g: &mut Game) {
        g.p.hidden = None;
        g.p.hide_lock = 0.0;
        g.p.in_vent = false;
        let home = g
            .fac
            .safe_zone
            .map(|c| c.center())
            .unwrap_or(g.fac.start.center());
        if let Some(p) = safe_landing(g, home.0, home.1) {
            g.p.pos = p;
        }
        g.p.feet_y = 0.0;
        g.p.stun = 0.0;
        g.taser_cd = 0.0;
        g.melee_cd = 0.0;
        g.p.stamina = MAX_STAMINA;
        g.p.exhausted = false;
        g.p.lamp = MAX_LAMP_CHARGE;
        g.meters = crate::game::meters::Meters::default();
    }

    /// _admin_teleport_to: leaves hiding / ducts, closes the admin menu and lands on open floor.
    /// A point outside the loaded window is reached by the window's own streaming: the runner is
    /// placed there and lands on the nearest open cell once the window has recentred.
    pub(super) fn teleport_to(&mut self, g: &mut Game, x: f32, y: f32) {
        g.p.hidden = None;
        g.p.in_vent = false;
        g.p.feet_y = 0.0;
        g.escape = None;
        let inside = x >= 0.0 && y >= 0.0 && x < g.fac.cols as f32 && y < g.fac.rows as f32;
        match safe_landing(g, x, y).filter(|_| inside) {
            Some(p) => g.p.pos = p,
            None if g.is_infinite() && !inside => {
                g.p.pos = (x, y);
                self.pending_landing = true;
            }
            None => {
                g.say("ADMIN // NO SAFE CELL NEAR THERE");
                return;
            }
        }
        self.set_open(false);
    }

    /// _admin_teleport_chunk: N/S/E/W land on the neighbouring chunk's centre cell (where the
    /// main halls cross), or the safe room, nearest relay, start, checkpoint.
    fn teleport_chunk(&mut self, g: &mut Game, id: &str) {
        let c = CHUNK as f32;
        let (px, py) = g.p.pos;
        let here = ((px / c).floor(), (py / c).floor());
        let centre = |cx: f32, cy: f32| {
            (
                cx * c + (CHUNK / 2) as f32 + 0.5,
                cy * c + (CHUNK / 2) as f32 + 0.5,
            )
        };
        let target = match id {
            "north" => centre(here.0, here.1 - 1.0),
            "south" => centre(here.0, here.1 + 1.0),
            "east" => centre(here.0 + 1.0, here.1),
            "west" => centre(here.0 - 1.0, here.1),
            "safe_room" | "checkpoint" => match g.fac.safe_zone {
                Some(cell) => cell.center(),
                None if id == "checkpoint" => g.fac.start.center(),
                None => {
                    g.say("ADMIN // NO SAFE ROOM WITHIN 5 CHUNKS");
                    return;
                }
            },
            "relay" => {
                let best = g
                    .fac
                    .relays
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !g.relays_taken[*i])
                    .map(|(_, r)| (*r, pdist(r.center(), g.p.pos)))
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                match best {
                    Some((cell, _)) => {
                        g.teleport_beside(cell);
                        g.p.hidden = None;
                        g.p.in_vent = false;
                        self.set_open(false);
                        return;
                    }
                    None => {
                        g.say("ADMIN // NO LOADED RELAY");
                        return;
                    }
                }
            }
            // the route's start: the lobby on a finite floor or while the window sits on chunk
            // 0,0; else chunk 0,0's centre (the window streams there)
            _ => match g.inf.as_ref() {
                Some(inf) if inf.center != (0, 0) => {
                    centre((1 - inf.center.0) as f32, (1 - inf.center.1) as f32)
                }
                _ => g.fac.start.center(),
            },
        };
        let inside = target.0 >= 0.0
            && target.1 >= 0.0
            && target.0 < g.fac.cols as f32
            && target.1 < g.fac.rows as f32;
        if matches!(id, "north" | "south" | "east" | "west") && !inside && !g.is_infinite() {
            g.say("ADMIN // THAT CHUNK DOES NOT EXIST");
            return;
        }
        self.teleport_to(g, target.0, target.1);
    }

    /// SAFE TELEPORT "X,Z" or "X,Z,LEVEL" in world cells (the route's absolute coordinates).
    /// A different LEVEL uses respawn semantics (`Game::respawn_on_level`): the floor is rebuilt
    /// round the window centre, the body lands on that level's start cell, the label changes once.
    /// This is an explicit teleport, on the change_storey audit allow list (not a walk).
    fn teleport_coordinates(&mut self, g: &mut Game, content: &Content, text: &str) {
        let parts: Vec<&str> = text
            .split(|ch: char| ch == ',' || ch == ';' || ch == ' ')
            .filter(|s| !s.is_empty())
            .collect();
        let (Some(x), Some(z)) = (
            parts.first().and_then(|s| s.parse::<f32>().ok()),
            parts.get(1).and_then(|s| s.parse::<f32>().ok()),
        ) else {
            g.say("ADMIN // TELEPORT NEEDS X,Z");
            return;
        };
        if !x.is_finite() || !z.is_finite() || x.abs() > 1.0e6 || z.abs() > 1.0e6 {
            g.say("ADMIN // TELEPORT NEEDS X,Z");
            return;
        }
        let origin = |g: &Game| match g.inf.as_ref() {
            Some(inf) => (
                ((inf.center.0 - 1) * CHUNK) as f32,
                ((inf.center.1 - 1) * CHUNK) as f32,
                inf.level,
            ),
            None => (0.0, 0.0, 0),
        };
        let (mut ox, mut oy, level) = origin(g);
        let mut jumped = None;
        if let Some(l) = parts.get(2).and_then(|s| s.parse::<f32>().ok()) {
            let l = l as i32;
            if l != level {
                if !crate::world::infinite::level_allowed(l) {
                    g.say("ADMIN // NO SUCH LEVEL");
                    return;
                }
                let catalog = crate::world::catalog::Catalog::load();
                if !g.respawn_on_level(l, content, &catalog) {
                    g.say("ADMIN // CANNOT CHANGE LEVEL HERE");
                    return;
                }
                let o = origin(g);
                ox = o.0;
                oy = o.1;
                jumped = Some(l);
            }
        }
        self.teleport_to(g, x - ox, z - oy);
        match jumped {
            Some(l) => g.say(&format!("ADMIN // LEVEL {l} // SAFE TELEPORT {x:.0}, {z:.0}")),
            None => g.say(&format!("ADMIN // SAFE TELEPORT // {x:.0}, {z:.0}")),
        }
    }

    /// _admin_spawn_hunters: one brain per id on the golden-angle ring round the runner, awake and
    /// heading for the runner.
    fn summon(&mut self, g: &mut Game, content: &Content, ids: &[String]) {
        use crate::hunters::secret_creatures as secret;
        // the picker's pair entry is both wolf spiders, linked afterwards (SecretCreatures.link_admin_pair)
        let pair = ids.iter().any(|id| id == secret::PAIR_ID);
        let expanded: Vec<String> = ids
            .iter()
            .flat_map(|id| if id == secret::PAIR_ID { secret::IDS.iter().map(|s| s.to_string()).collect() } else { vec![id.clone()] })
            .collect();
        let ids: &[String] = &expanded;
        let (fx, fy) = g.forward2();
        let base = (fx).atan2(-fy);
        let mut placed = 0;
        let mut last = String::new();
        let mut refused: Option<i32> = None;
        for (slot, id) in ids.iter().enumerate() {
            let Some(def) = content.hunter(id) else {
                continue;
            };
            // the wolf spiders live on their home level only
            let home = secret::Rules::get().home_level;
            if secret::stays_home(id) && g.runner_level() != home {
                refused = Some(home);
                continue;
            }
            let angle = base + slot as f32 * RING_ANGLE;
            let radius = RING_START + RING_STEP * (slot as f32).sqrt();
            let want = (
                g.p.pos.0 + angle.cos() * radius,
                g.p.pos.1 + angle.sin() * radius,
            );
            let Some(at) = g
                .fac
                .nearest_open(want.0, want.1, HUNTER_RADIUS)
                .or_else(|| g.fac.nearest_open(g.p.pos.0, g.p.pos.1, HUNTER_RADIUS))
            else {
                continue;
            };
            let seq = self.summoned.iter().filter(|s| *s == id).count() + 1;
            let mut b = Brain::new(
                def.clone(),
                at,
                &format!("{}:ADMIN:{id}:{seq}", g.level_seed()),
                g.difficulty,
            );
            // stand on the runner's level: a Brain starts on level 0 at y 0. On the ground (the Inferno's floor follows the
            // terrain on its first step)
            let level = g.runner_level();
            b.set_level(level);
            if let Some(t) = g.fac.terrain().filter(|_| g.fac.level == level) {
                b.y += t.floor_at(at.0, at.1);
            }
            if let Gimmick::Solyth(s) = &mut b.gimmick {
                s.heat_locked = g.meters.heat_locked;
            }
            b.activate_at(g.p.pos);
            b.last_known_level = level;
            g.hunters.push(b);
            self.summoned.push(id.clone());
            placed += 1;
            last = def.name.to_uppercase();
        }
        if pair && placed == 2 {
            g.link_admin_wolf_spiders();
        }
        if let Some(home) = refused.filter(|_| placed == 0) {
            g.say(&format!("ADMIN SUMMON FAILED // THIS ONE LIVES ON LEVEL {home}"));
            return;
        }
        if ids.len() == 1 {
            g.say(&if placed == 1 {
                format!("ADMIN SUMMONED // {last}")
            } else {
                "ADMIN SUMMON FAILED // NO OPEN FLOOR".to_string()
            });
        } else if placed == 0 {
            g.say("ADMIN SUMMON ALL FAILED // NO OPEN FLOOR");
        } else {
            g.say(&format!(
                "ADMIN SUMMONED // {placed} OF {} HUNTERS{}",
                ids.len(),
                if placed == ids.len() {
                    ""
                } else {
                    " (NO ROOM FOR THE REST)"
                }
            ));
        }
    }

    /// Removing brains mid-capture or while a hunter checks the runner's locker would leave
    /// those indices dangling, so removals wait for a calm moment.
    fn can_remove(&self, g: &mut Game) -> bool {
        if g.escape.is_some() || g.p.hidden.is_some() {
            g.say("ADMIN REMOVE // NOT WHILE CAUGHT OR HIDING");
            return false;
        }
        true
    }

    /// Removes brain `i`. Slot 0 (the floor's route hunter, read by the relay logic) is never
    /// taken out of the list: it is parked instead, asleep and frozen.
    fn take_out(g: &mut Game, i: usize) {
        if i == 0 {
            let b = &mut g.hunters[0];
            b.active = false;
            b.state = State::Idle;
            b.last_known = None;
            b.apply_stun(1.0e6, false);
            b.pos = g.fac.hunter_spawn.center();
        } else {
            g.hunters.remove(i);
        }
    }

    /// _admin_remove_hunter_authoritative: every brain with that id.
    fn remove_hunter(&mut self, g: &mut Game, id: &str, name: &str) {
        if !self.can_remove(g) {
            return;
        }
        // the picker's wolf spider pair entry takes both out
        if id == crate::hunters::secret_creatures::PAIR_ID {
            for member in crate::hunters::secret_creatures::IDS {
                self.remove_hunter(g, member, name);
            }
            return;
        }
        let mut removed = 0;
        let mut i = g.hunters.len();
        while i > 0 {
            i -= 1;
            if g.hunters[i].def.id == id {
                Admin::take_out(g, i);
                removed += 1;
            }
        }
        self.summoned.retain(|s| s != id);
        g.say(&if removed > 0 {
            format!("ADMIN REMOVED // {name} x{removed}")
        } else {
            format!("ADMIN REMOVE // NO {name} ACTIVE")
        });
    }

    fn all_hunters(&mut self, g: &mut Game, content: &Content, i: usize) {
        let prof = g.prof();
        let at = g.p.pos;
        match i {
            // taser_stun is escape_stun - 0.2 on every difficulty in game_config.gd
            0 => g
                .hunters
                .iter_mut()
                .for_each(|h| h.apply_stun((prof.escape_stun - 0.2).max(0.5), false)),
            1 => g.hunters.iter_mut().for_each(|h| h.activate_at(at)),
            2 => g.hunters.iter_mut().for_each(|h| {
                // calm_reset(2.0)
                h.state = State::Idle;
                h.last_known = None;
                h.idle_timer = 0.6 + h.next_float() * 1.4;
                h.clear_path();
                h.direct_visible = false;
                h.apply_stun(2.0, false);
            }),
            3 => g
                .hunters
                .iter_mut()
                .for_each(|h| h.apply_stun(1.0e6, false)),
            4 => g.hunters.iter_mut().for_each(|h| h.stun = 0.0),
            5 => {
                let ids: Vec<String> = super::menu::hunter_list(content)
                    .into_iter()
                    .map(|(id, _)| id)
                    .filter(|id| !self.summoned.contains(id))
                    // the wolf spiders come with the list only on their home level; the pair entry is both of them again
                    .filter(|id| {
                        use crate::hunters::secret_creatures as secret;
                        id != secret::PAIR_ID && (!secret::stays_home(id) || g.runner_level() == secret::Rules::get().home_level)
                    })
                    .collect();
                if ids.is_empty() {
                    g.say("ADMIN SUMMON ALL // ALL HUNTERS ALREADY SUMMONED");
                } else {
                    self.summon(g, content, &ids);
                }
            }
            _ => {
                if !self.can_remove(g) {
                    return;
                }
                let mut removed = 0;
                let mut pending = std::mem::take(&mut self.summoned);
                let mut j = g.hunters.len();
                while j > 0 {
                    j -= 1;
                    if let Some(k) = pending.iter().rposition(|s| *s == g.hunters[j].def.id) {
                        // summoned brains were pushed last: take the newest copies of each id
                        pending.remove(k);
                        Admin::take_out(g, j);
                        removed += 1;
                    }
                }
                g.say(&if removed == 0 {
                    "ADMIN REMOVE // NO SUMMONED HUNTERS".to_string()
                } else {
                    format!(
                        "ADMIN REMOVED // {removed} SUMMONED HUNTER{}",
                        if removed == 1 { "" } else { "S" }
                    )
                });
            }
        }
    }

    /// GRANT RELAY / ALL LOADED RELAYS: collected as the relay's [E] does (score, count, the
    /// route hunter's activation on a finite floor).
    fn grant_relays(&mut self, g: &mut Game, all: bool) {
        let mut order: Vec<usize> = (0..g.fac.relays.len())
            .filter(|i| !g.relays_taken[*i])
            .collect();
        order.sort_by(|a, b| {
            pdist(g.fac.relays[*a].center(), g.p.pos)
                .partial_cmp(&pdist(g.fac.relays[*b].center(), g.p.pos))
                .unwrap()
        });
        if order.is_empty() {
            g.say("ADMIN // NO LOADED RELAY");
            return;
        }
        for i in order.into_iter().take(if all { usize::MAX } else { 1 }) {
            g.relays_taken[i] = true;
            g.relays_collected += 1;
            g.score += RELAY_SCORE;
        }
        g.announce("RELAY ONLINE");
        g.say("ADMIN // SIGNAL RELAY GRANTED");
        let need = g.prof().activation_relays.min(g.fac.relays.len() as i32);
        if !g.is_infinite()
            && !g.hunters.is_empty()
            && !g.hunters[0].active
            && g.relays_collected >= need
            && !g.hunters[0].gimmick.runs_while_inactive()
        {
            let p = g.p.pos;
            g.hunters[0].activate_at(p);
        }
    }

    /// _admin_spawn_ravelle_trap: a wire 1.5 m ahead, across the runner's facing.
    fn spawn_ravelle_trap(&mut self, g: &mut Game) {
        let (fx, fy) = g.forward2();
        let at = (g.p.pos.0 + fx * 1.5, g.p.pos.1 + fy * 1.5);
        let across = (-fy * 1.42, fx * 1.42);
        for h in g.hunters.iter_mut() {
            if let Gimmick::Ravelle(r) = &mut h.gimmick {
                r.wires.push(crate::hunters::gimmicks::ravelle::Wire {
                    a: (at.0 - across.0, at.1 - across.1),
                    b: (at.0 + across.0, at.1 + across.1),
                    triggered: false,
                });
                g.say("ADMIN // RAVELLE TRAP PLACED");
                return;
            }
        }
        g.say("ADMIN // NO RAVELLE IN THIS ROUTE");
    }

    /// _admin_status: the STATUS page readout.
    pub(super) fn status(&self, g: &Game) -> String {
        let server = crate::net::config::PRIMARY_BASE_URL.trim_start_matches("https://").trim_start_matches("http://");
        let mut out = format!(
            "BUILD {}\nSERVER {server}\n",
            crate::app::updater::build_label()
        );
        let region = g
            .fac
            .region_at(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32)
            .to_uppercase();
        match g.inf.as_ref() {
            Some(inf) => {
                let cx = inf.center.0 - 1 + (g.p.pos.0 / CHUNK as f32).floor() as i32;
                let cy = inf.center.1 - 1 + (g.p.pos.1 / CHUNK as f32).floor() as i32;
                out += &format!("CHUNK {cx}:{}:{cy}  {region}\n", inf.level);
                out += &format!(
                    "LOADED 9  DISCOVERED {}  HUNTERS {}\n",
                    g.chunks_visited(),
                    g.hunters.len()
                );
            }
            None => {
                out += &format!("FLOOR DEPTH {}  {region}\n", g.depth);
                out += &format!("LOADED 1  DISCOVERED 1  HUNTERS {}\n", g.hunters.len());
            }
        }
        // Godot _admin_status: the runner's room integrity, then the active checkpoint id
        let safe = match g.live_room_status() {
            Some(s) => format!("{}%{}", s.integrity, if s.breached { " BREACHED" } else { "" }),
            None if g.fac.safe_zone.is_some() => "100%".to_string(),
            None => "NONE".to_string(),
        };
        let checkpoint = match g.active_checkpoint() {
            "" if crate::app::checkpoint::load().is_some() => "SAVED".to_string(),
            "" => "NONE".to_string(),
            id => id.to_string(),
        };
        out += &format!("SAFE {safe}  CHECKPOINT {checkpoint}\n");
        out += &format!(
            "RELAYS {}/{}  SCORE {}  SEED {}\n",
            g.relays_collected,
            g.fac.relays.len(),
            g.score,
            g.run_seed
        );
        out += &format!(
            "MAP REVEAL {}\n",
            if self.map_reveal { "ON" } else { "OFF" }
        );
        let mut rows: Vec<(f32, String)> = g
            .hunters
            .iter()
            .map(|h| {
                let d = pdist(h.pos, g.p.pos);
                let state = format!("{:?}", h.state).to_uppercase();
                let extra = if h.stun > 999.0 {
                    " FROZEN"
                } else if !h.active {
                    " ASLEEP"
                } else {
                    ""
                };
                (
                    d,
                    format!("{} {state} {}m{extra}", h.def.name.to_uppercase(), d as i32),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        out += "HUNTERS (nearest)";
        for (_, r) in rows.iter().take(6) {
            out += &format!("\n  {r}");
        }
        if self.game_speed != 1.0 || self.step_frames > 0 {
            out += &format!(
                "\nGAME SPEED {}",
                if self.game_speed == 0.0 {
                    "PAUSED".to_string()
                } else {
                    format!("{}X", self.game_speed)
                }
            );
        }
        out
    }
}
