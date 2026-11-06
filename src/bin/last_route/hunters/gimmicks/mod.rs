//! Each hunter's own trick (game/hunters/gimmicks/*.gd), hooked into the shared brain at the
//! original's points: before perception, after perception, speed / sight scales, holding
//! position, breaching hiding, capture immunity and inescapable grabs. Specs: docs/GIMMICKS.md.

pub mod aki;
pub mod charge;
pub mod briselle;
pub mod ceryth;
pub mod elise;
pub mod ferryn;
pub mod glyra;
pub mod haelith;
pub mod ilyra;
pub mod javelle;
pub mod kathy;
pub mod loxelle;
pub mod mavrine;
pub mod nyxie;
pub mod opalie;
pub mod quenelle;
pub mod ravelle;
pub mod solyth;
pub mod thessine;
pub mod umbriel;
pub mod verrine;
pub mod void;
pub mod wolf_spider;
pub mod wynelle;
pub mod xivvy;
pub mod ysolde;
pub mod zellia;

pub use ceryth::Ceryth;
pub use glyra::Glyra;
pub use nyxie::Nyxie;
pub use opalie::Opalie;
pub use verrine::Verrine;
pub use zellia::Zellia;

use super::brain::{Brain, Target};
use crate::config::Difficulty;
use crate::world::{Cell, Facility};

/// The world as gimmicks see it this frame.
pub struct Ctx<'a> {
    pub fac: &'a Facility,
    pub relays_taken: &'a [bool],
    pub batteries_taken: &'a [bool],
    pub hiding_disabled: &'a [bool],
    pub vents: &'a [Cell],
    pub heater_on: bool,
    pub heater_thawed: bool,
    pub diff: Difficulty,
    /// Seconds left on the floor's music box (Glyra).
    pub music_box_left: f32,
    /// Centres of the music box floor pickups still uncollected in this window (Glyra guards the nearest).
    pub music_boxes: &'a [(f32, f32)],
    pub exposure: f32,
}

/// What a gimmick asks the game to do (main_3d's world_events and player effects).
#[derive(Clone, Debug)]
pub enum Event {
    /// Stun the runner (movement and items locked).
    Stun(f32),
    /// Every active hunter (optionally but this one) investigates the point.
    FacilityAlert { at: (f32, f32), except_self: bool },
    /// Pull the runner out of hiding to this point.
    Eject { to: (f32, f32) },
    /// Player meters: exposure, banked noise, temperature.
    Meter {
        exposure_add: f32,
        noise_add: f32,
        temperature_add: f32,
    },
    /// A hiding spot is burnt out (true) or usable again (false).
    SpotDisabled(usize, bool),
    /// Thessine's fake cue.
    Mimic { rare: bool },
    /// Ysolde took a battery off the floor.
    TakeBattery(usize),
    /// A capture that skips the struggle (Nyxie's dead lamp, Wynelle's freeze).
    InescapableCapture,
    /// Void Orange: one system goes offline for `seconds` (void_orange_gimmick.gd; the player side is game/void_player.rs).
    SystemOutage(void::void_orange::Outage),
    /// Void Purple: radar static or a full blackout (void_purple_gimmick.gd `radar_corrupt`).
    RadarCorrupt(void::void_purple::RadarCorrupt),
    /// Void Pink: a run of fake footsteps near the runner (void_pink_gimmick.gd `fake_footsteps`).
    FakeFootsteps(void::void_pink::FakeFootsteps),
    /// Shown to the runner (only the ones the original shows).
    Message(String),
}

/// Godot `HunterBrain.ANNOUNCE_HUNTER_EVENTS` (hunters/ai/hunter_brain.gd:82): off on purpose, so players discover what the
/// hunters do on their own. The hunters still act; only the callouts are dropped.
pub const ANNOUNCE_HUNTER_EVENTS: bool = false;

/// Godot `HunterBrain.emit_world_event` (hunter_brain.gd:88-95) with the flag off: a pure announcement (`Event::Message`) is
/// dropped, so no toast is shown. Mechanic events stay, including `FacilityAlert` (the investigation the hunters raise), so
/// the other hunters still react, and `Mimic` keeps its cue.
pub fn silence_callouts(ev: &mut Vec<Event>) {
    if !ANNOUNCE_HUNTER_EVENTS {
        ev.retain(|e| !matches!(e, Event::Message(_)));
    }
}

#[derive(Default)]
pub enum Gimmick {
    #[default]
    None,
    Aki(aki::Aki),
    Briselle(briselle::Briselle),
    Ceryth(Ceryth),
    Elise(elise::Elise),
    Ferryn(ferryn::Ferryn),
    Glyra(Glyra),
    Haelith,
    Ilyra(ilyra::Ilyra),
    Javelle(javelle::Javelle),
    Kathy,
    Loxelle(loxelle::Loxelle),
    Mavrine(mavrine::Mavrine),
    Nyxie(Nyxie),
    Opalie(Opalie),
    Porcelle,
    Quenelle(quenelle::Quenelle),
    Ravelle(ravelle::Ravelle),
    Solyth(solyth::Solyth),
    Thessine(thessine::Thessine),
    Umbriel(umbriel::Umbriel),
    Verrine(Verrine),
    WolfSpider(wolf_spider::WolfSpider),
    Wynelle(wynelle::Wynelle),
    Xivvy(xivvy::Xivvy),
    Ysolde(ysolde::Ysolde),
    Zellia(Zellia),
    VoidBlack(void::void_black::VoidBlack),
    VoidBlue(void::void_blue::VoidBlue),
    VoidBrown(void::void_brown::VoidBrown),
    VoidCyan(void::void_cyan::VoidCyan),
    VoidGray(void::void_gray::VoidGray),
    VoidGreen(void::void_green::VoidGreen),
    VoidOrange(void::void_orange::VoidOrange),
    VoidPink(void::void_pink::VoidPink),
    VoidPurple(void::void_purple::VoidPurple),
    VoidRed(void::void_red::VoidRed),
    VoidWhite(void::void_white::VoidWhite),
    VoidYellow(void::void_yellow::VoidYellow),
}

impl Gimmick {
    pub fn for_id(id: &str, diff: Difficulty) -> Gimmick {
        match id {
            "aki" => Gimmick::Aki(Default::default()),
            "briselle" => Gimmick::Briselle(Default::default()),
            "ceryth" => Gimmick::Ceryth(Default::default()),
            "elise" => Gimmick::Elise(Default::default()),
            "ferryn" => Gimmick::Ferryn(Default::default()),
            "glyra" => Gimmick::Glyra(Glyra::new()),
            "haelith" => Gimmick::Haelith,
            "ilyra" => Gimmick::Ilyra(Default::default()),
            "javelle" => Gimmick::Javelle(Default::default()),
            "kathy" => Gimmick::Kathy,
            "loxelle" => Gimmick::Loxelle(Default::default()),
            "mavrine" => Gimmick::Mavrine(mavrine::Mavrine::new()),
            "nyxie" => Gimmick::Nyxie(Default::default()),
            "opalie" => Gimmick::Opalie(Default::default()),
            "porcelle" => Gimmick::Porcelle,
            "quenelle" => Gimmick::Quenelle(quenelle::Quenelle::for_difficulty(diff)),
            "ravelle" => Gimmick::Ravelle(Default::default()),
            "solyth" => Gimmick::Solyth(Default::default()),
            "thessine" => Gimmick::Thessine(thessine::Thessine::new()),
            "umbriel" => Gimmick::Umbriel(umbriel::Umbriel::new()),
            "verrine" => Gimmick::Verrine(Default::default()),
            "white_wolf_spider" | "black_wolf_spider" => Gimmick::WolfSpider(wolf_spider::WolfSpider::new(id)),
            "wynelle" => Gimmick::Wynelle(Default::default()),
            "xivvy" => Gimmick::Xivvy(Default::default()),
            "ysolde" => Gimmick::Ysolde(Default::default()),
            "zellia" => Gimmick::Zellia(Zellia::new(diff)),
            "void_black" => Gimmick::VoidBlack(Default::default()),
            "void_blue" => Gimmick::VoidBlue(Default::default()),
            "void_brown" => Gimmick::VoidBrown(Default::default()),
            "void_cyan" => Gimmick::VoidCyan(Default::default()),
            // Godot void_gray_gimmick.gd: Nightmare uses its own idle, grace and node timings
            "void_gray" => Gimmick::VoidGray(void::void_gray::VoidGray::new(diff == Difficulty::Nightmare)),
            "void_green" => Gimmick::VoidGreen(Default::default()),
            "void_orange" => Gimmick::VoidOrange(Default::default()),
            "void_pink" => Gimmick::VoidPink(Default::default()),
            "void_purple" => Gimmick::VoidPurple(Default::default()),
            "void_red" => Gimmick::VoidRed(Default::default()),
            "void_white" => Gimmick::VoidWhite(Default::default()),
            "void_yellow" => Gimmick::VoidYellow(Default::default()),
            _ => Gimmick::None,
        }
    }

    pub fn runs_while_inactive(&self) -> bool {
        matches!(
            self,
            Gimmick::Nyxie(_) | Gimmick::Solyth(_) | Gimmick::Wynelle(_)
        )
    }

    pub fn before_perception(
        &mut self,
        b: &mut Brain,
        dt: f32,
        c: &Ctx,
        t: &Target,
        ev: &mut Vec<Event>,
    ) {
        match self {
            Gimmick::Aki(g) => g.before(b, dt, c, ev),
            Gimmick::Elise(g) => g.before(b, dt, t),
            Gimmick::Kathy => kathy::before(b, c, t),
            Gimmick::Mavrine(g) => g.before(b, dt, c, t),
            Gimmick::Nyxie(g) => g.before(b, dt, c, t),
            Gimmick::Quenelle(g) => g.before(b, dt, t),
            Gimmick::Ravelle(g) => g.before(b, dt, c, t, ev),
            Gimmick::Solyth(g) => g.before(b, dt, c, t, ev),
            Gimmick::Thessine(g) => g.before(b, dt, ev),
            Gimmick::Umbriel(g) => g.before(b, dt),
            Gimmick::WolfSpider(g) => g.before(b, dt, c, t),
            Gimmick::Wynelle(g) => g.before(b, dt, c, t, ev),
            Gimmick::Xivvy(g) => g.before(b, dt, c, t),
            Gimmick::VoidBlack(g) => g.before(b, c, t),
            Gimmick::VoidBlue(g) => g.before(b, dt, c, t, ev),
            Gimmick::VoidGray(g) => {
                // Godot passes the runner's peer id (default 1); the Rust Target has none yet. The route prompt
                // (signal_route) is not ported, so its events are dropped here for now.
                g.before(b, dt, c, t, 1);
                let _ = g.take_events();
            }
            Gimmick::VoidOrange(g) => {
                if let Some(o) = g.before(b, dt, t) {
                    ev.push(Event::SystemOutage(o));
                }
            }
            Gimmick::VoidPurple(g) => {
                // has_target: the Rust brain always has a runner Target (Godot `not target.is_empty()`)
                if let Some(r) = g.before(b, dt, true) {
                    ev.push(Event::RadarCorrupt(r));
                }
            }
            Gimmick::VoidRed(g) => g.before(b, c),
            Gimmick::VoidWhite(g) => g.before(b, c, t),
            Gimmick::VoidYellow(g) => g.before(dt, t.map_open),
            _ => {}
        }
        silence_callouts(ev);
    }

    pub fn after_perception(
        &mut self,
        b: &mut Brain,
        dt: f32,
        c: &Ctx,
        t: &Target,
        ev: &mut Vec<Event>,
    ) {
        match self {
            Gimmick::Briselle(g) => g.after(b, dt, c),
            Gimmick::Ceryth(g) => g.after(b, c),
            Gimmick::Ferryn(g) => g.after(b, c, t, ev),
            Gimmick::Glyra(g) => g.after(b, dt, c, t),
            Gimmick::Haelith => haelith::after(b, c),
            Gimmick::Ilyra(g) => g.after(b, dt, c, t, ev),
            Gimmick::Javelle(g) => g.after(b, dt, c, t, ev),
            Gimmick::Loxelle(g) => g.after(b, dt, c, t, ev),
            Gimmick::Opalie(g) => g.after(b, c, t),
            Gimmick::Verrine(g) => g.after(b, t),
            Gimmick::Ysolde(g) => g.after(b, dt, c, t, ev),
            Gimmick::WolfSpider(g) => g.after(b),
            Gimmick::Zellia(g) => g.after(b, dt, c, t),
            Gimmick::VoidBrown(g) => g.after(b, dt, c, t, ev),
            Gimmick::VoidCyan(g) => g.after(b, dt, t, ev),
            Gimmick::VoidGray(g) => g.after(b),
            Gimmick::VoidGreen(g) => g.after(b, dt, c, t, ev),
            Gimmick::VoidPink(g) => {
                if let Some(f) = g.after(b, dt) {
                    ev.push(Event::FakeFootsteps(f));
                }
            }
            _ => {}
        }
        silence_callouts(ev);
    }

    pub fn speed_scale(&self, b: &Brain) -> f32 {
        match self {
            Gimmick::Briselle(g) => g.speed_scale(),
            Gimmick::Ceryth(g) => g.speed_scale(b),
            Gimmick::Glyra(g) => g.speed_scale(),
            Gimmick::Quenelle(g) => g.speed_scale,
            Gimmick::Ravelle(g) => g.speed_scale(),
            Gimmick::Verrine(g) => g.speed_scale,
            Gimmick::Xivvy(g) => g.speed_scale,
            Gimmick::Nyxie(g) => g.speed_scale(),
            Gimmick::Solyth(g) => g.speed_scale(),
            Gimmick::VoidBlack(g) => g.speed_scale(),
            Gimmick::VoidRed(g) => g.speed_scale(),
            Gimmick::VoidWhite(g) => g.speed_scale(),
            Gimmick::VoidYellow(g) => g.speed_scale(),
            Gimmick::WolfSpider(g) => g.speed_scale(),
            _ => 1.0,
        }
    }

    pub fn sight_scale(&self) -> f32 {
        match self {
            Gimmick::Ilyra(_) => 1.8,
            _ => 1.0,
        }
    }

    pub fn holds_position(&self, _b: &Brain) -> bool {
        match self {
            Gimmick::Ceryth(g) => g.holds_position(),
            Gimmick::Ilyra(_) => true,
            Gimmick::Nyxie(g) => g.holds_position(),
            Gimmick::Solyth(g) => g.holds_position(),
            Gimmick::Loxelle(g) => g.holds_position(),
            Gimmick::Mavrine(g) => g.holds_position(),
            Gimmick::Opalie(g) => g.on_station,
            Gimmick::Verrine(g) => !g.active_in_vent,
            Gimmick::Zellia(g) => g.holds_position(),
            Gimmick::VoidGray(g) => g.holds_position(),
            Gimmick::VoidGreen(g) => g.holds_position(),
            Gimmick::WolfSpider(g) => g.holds_position(),
            _ => false,
        }
    }

    /// HunterGimmick.peek_chance: the odds of peeking at a corner, from the tuning's `base`.
    pub fn peek_chance(&self, base: f32) -> f32 {
        match self {
            // calm she never peeks while her music plays (GlyraGimmick.peek_chance)
            Gimmick::Glyra(g) if g.playing() => 0.0,
            _ => base,
        }
    }

    pub fn breaches_hiding(&self) -> bool {
        matches!(self, Gimmick::Elise(_))
    }

    pub fn immune_to_capture(&self, _b: &Brain) -> bool {
        match self {
            Gimmick::Ceryth(_) | Gimmick::Ilyra(_) | Gimmick::Loxelle(_) => true,
            Gimmick::Glyra(g) => g.playing(),
            Gimmick::Opalie(g) => g.on_station,
            Gimmick::Verrine(g) => !g.active_in_vent,
            Gimmick::Zellia(g) => g.trading(),
            Gimmick::VoidGray(g) => g.immune_to_capture(),
            Gimmick::VoidGreen(g) => g.immune_to_capture(),
            // void_pink_gimmick.gd:51-52: her footsteps can never catch the runner
            Gimmick::VoidPink(_) => true,
            _ => false,
        }
    }

    pub fn capture_inescapable(&self) -> bool {
        match self {
            Gimmick::Nyxie(g) => g.inescapable(),
            _ => false,
        }
    }

    /// Drawn faded (alpha 0.28) instead of solid.
    pub fn hidden_from_view(&self) -> bool {
        match self {
            Gimmick::Ceryth(g) => g.dispersed,
            Gimmick::Ilyra(g) => g.banished(),
            Gimmick::VoidBlue(g) => g.hidden_from_view(),
            Gimmick::VoidGray(g) => g.hidden_from_view(),
            Gimmick::VoidGreen(g) => g.hidden_from_view(),
            Gimmick::Verrine(g) => !g.active_in_vent,
            _ => false,
        }
    }

    /// Porcelle shows on the minimap.
    pub fn on_radar(&self) -> bool {
        matches!(self, Gimmick::Porcelle)
    }

    /// Ceryth's eye colour, when she is waiting by a locker.
    pub fn eye_color(&self) -> Option<(f32, f32, f32)> {
        match self {
            Gimmick::Ceryth(g) if !g.dispersed => Some(if g.eyes_red {
                (1.0, 0.1, 0.1)
            } else {
                (0.2, 0.45, 1.0)
            }),
            _ => None,
        }
    }
}

/// One-paragraph dossier text for each hunter (written from the gimmick rules above).
pub fn describe(id: &str) -> &'static str {
    match id {
        "aki" => "Hold your light on her and a fuse starts: ten seconds later she floods you with exposure three times, five seconds apart.",
        "briselle" => "Every relay you recover after she wakes draws her straight to it, a quarter faster. A held beam breaks her lock and stuns her.",
        "ceryth" => "Waits beside a locker with red or blue eyes. Red wants your light off, blue wants it on. Guess right and she runs; guess wrong and the door holds you for 30 seconds.",
        "dovelle" => "No tricks. She sees, she hears, she follows.",
        "elise" => "Counts every second you sprint. After fifty she never lets go again, and a locker will not hide you.",
        "ferryn" => "Always coming, slowly. Hide, rest or flash her and she vanishes back to her room; a flash also calls the others.",
        "glyra" => "Calm while her music box plays. Let it wind down and she hunts fast with sharpened hearing. Hold interact on the box to wind it.",
        "haelith" => "Camps the relays you still need, one after another. She will not chase, but she will catch anyone who comes close.",
        "ilyra" => "Watches from a room corner and never walks. Lose her sight and fifteen seconds later you are pinned for five. A held beam banishes her.",
        "javelle" => "Stand over a locker for eight seconds and she pulls whoever is inside out into the open.",
        "kathy" => "A narrow search beam along her gaze spots you in any darkness.",
        "loxelle" => "Burns lockers in her room: an empty one is ruined for a while, an occupied one throws you out hot.",
        "mavrine" => "Pauses, then lays an invisible mark nearby. Cross it and she knows exactly where.",
        "nyxie" => "Two minutes of darkness wake her. A dead lamp for ten seconds wakes her on top of you. Shine your beam on her to send her back.",
        "opalie" => "Guards a relay. Nothing near her post can be used while she holds it; draw her off.",
        "porcelle" => "The only hunter your map shows.",
        "quenelle" => "Keep your light on for eight seconds and she locks onto you for good, faster every time you flick it.",
        "ravelle" => "Strings tripwires across doorways. Each one you hit stuns you, alerts everything and makes her faster.",
        "solyth" => "When the heat reaches its limit she arrives and never stops, burning batteries as she comes.",
        "thessine" => "Mimics sounds that are not there.",
        "umbriel" => "Wears another hunter's body every thirty seconds.",
        "verrine" => "Lives in the vents. Enter them and she hunts you there, faster the longer you stay.",
        "wynelle" => "Cold calls her. Stay chilled too long and she appears beside you; thaw the heater or reach the safe zone before you freeze.",
        "xivvy" => "Locks on at arm's length without needing to see you, and only gets faster the longer a chase runs.",
        "ysolde" => "Steals batteries off the floor and keeps her distance from you.",
        "zellia" => "A merchant in the middle of the floor. Coins buy warmth, cold, light or pesticide, but every deal carries a price.",
        "void_black" => "This stalker moves 50% faster while you are not watching her. Facing her within 11 cells and with a clear line of sight stops her completely. Check behind you regularly, then use the pause to create distance.",
        "void_blue" => "This anomaly is nearly invisible and hidden on radar. Hitting her with the flashlight beam fully reveals her for 3.5 seconds and pushes her about six cells away. Flash likely approach routes frequently.",
        "void_brown" => "Every 18 seconds, this entity jams up to two hiding spots within nine cells of you. A jam lasts 20 seconds and never targets the spot you currently occupy. Change rooms if nearby hiding spots are unavailable.",
        "void_cyan" => "If this hunter keeps clear line of sight to you for 2.5 seconds, she sends your exact position to every other active hunter. After a successful ping she has a 30-second cooldown. Break sight before the lock completes.",
        "void_gray" => "This entity stays dormant for 180 seconds after activation or collecting a relay, reduced to 120 seconds on Nightmare. When that grace ends, standing still in the open for 45 seconds starts five E-key prompts; Nightmare reduces the idle limit to 30 seconds. Each prompt allows 2.5 seconds, or 1.8 seconds on Nightmare. Completing all five keeps them dormant. Missing one makes her visible and starts a normal chase until another relay resets her.",
        "void_green" => "Every 22 seconds, this hunter can enter the vent network and reappear about nine cells from you after a two-second delay. This relocation only occurs when she has enough distance to move through the network. Keep moving after she disappears.",
        "void_orange" => "Every 26 seconds, this saboteur disables either the flashlight, climate controls, or radar for nine seconds. The selected system returns automatically. Move to a safe position and wait for the outage to end.",
        "void_pink" => "She never attacks and cannot catch you. Every 11 seconds she has a 60% chance to make a few faint footsteps sound somewhere close. They are only a sound: listen, then keep going.",
        "void_purple" => "Every 21 seconds, this entity adds radar static for eight seconds. Each corruption has a 35% chance to make the radar completely black for 11 seconds. Radar returns automatically; use the game view until it does.",
        "void_red" => "This speedster moves at 155% speed in corridors and 78% speed inside rooms. Leave long hallways and cut through rooms whenever she begins chasing.",
        "void_white" => "This hunter moves 35% faster unless your flashlight beam is directly hitting her. A beam within 10 cells stops her completely. Hold the beam long enough to create distance, but watch your remaining battery charge.",
        "void_yellow" => "This hunter gains 6% movement speed for every second the full-screen map remains open, up to 70% faster. The increase lasts for the run. Use the map only in short checks.",
        id if super::secret_creatures::is_secret(id) => super::secret_creatures::Rules::get().dossier_text(id).unwrap_or(""),
        _ => "A hunter added by a mod pack. She runs the shared brain on her own tuning.",
    }
}

/// Distance from a point to a segment.
pub fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let l2 = abx * abx + aby * aby;
    let t = if l2 <= 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / l2).clamp(0.0, 1.0)
    };
    let q = (a.0 + abx * t, a.1 + aby * t);
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
}

/// nearest_open_position for a hunter body (radius 0.33).
pub fn open_near(fac: &Facility, p: (f32, f32)) -> (f32, f32) {
    fac.nearest_open_or(p.0, p.1, 0.33)
}

/// hiding_exit_position: an open cell beside a spot for the runner (radius 0.24).
pub fn hiding_exit(fac: &Facility, spot: usize) -> (f32, f32) {
    let h = &fac.hiding[spot];
    let (cx, cy) = h.cell.center();
    fac.nearest_open_or(
        cx + h.facing.0 as f32 * 0.75,
        cy + h.facing.1 as f32 * 0.75,
        0.24,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::residency::VOID_IDS;

    #[test]
    fn describe_covers_every_void_hunter() {
        for id in VOID_IDS {
            let text = describe(id);
            assert!(!text.is_empty(), "{id}");
            assert!(!text.contains("mod pack"), "{id}");
        }
        assert!(describe("void_red").contains("155%"));
    }

    #[test]
    fn hunter_callouts_are_silent_like_godot() {
        // Godot validate_no_announcements.gd: the announcement goes, the mechanic keys stay.
        assert!(!ANNOUNCE_HUNTER_EVENTS);
        let mut ev = vec![
            Event::Message("SOMETHING WAS STANDING THERE".into()),
            Event::FacilityAlert { at: (1.0, 1.0), except_self: true },
            Event::Stun(1.6),
            Event::Message("BELLS RING DOWN THE HALL".into()),
            Event::Mimic { rare: false },
        ];
        silence_callouts(&mut ev);
        assert_eq!(ev.len(), 3);
        assert!(!ev.iter().any(|e| matches!(e, Event::Message(_))));
        assert!(matches!(ev[0], Event::FacilityAlert { at: (x, y), except_self: true } if x == 1.0 && y == 1.0));
        assert!(matches!(ev[1], Event::Stun(s) if s == 1.6));
        assert!(matches!(ev[2], Event::Mimic { rare: false }));
    }
}
