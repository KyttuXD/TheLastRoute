# Night Watch 3D port (Godot to Rust)

Contract for the port of the Godot 3D Night Watch to `src/bin/night_watch/`. Edit agents follow it exactly. They cannot compile while editing, so names, signatures and module paths below are binding. If a signature is wrong, the integrator fixes it here first, then the code.

Paths used in this file:

- `NW` = `C:\Users\Kyttu\Documents\Projects\LR Game Stuff\LastRoute Godot\KyttusLabApp\game\night_watch` (Godot sources; every Godot file named below is `NW\<name>`).
- `GD` = `C:\Users\Kyttu\Documents\Projects\LR Game Stuff\LastRoute Godot\KyttusLabApp` (data and assets root).
- `RS` = `C:\Users\Kyttu\Documents\Projects\LR Game Stuff\LastRoute Rust` (this repo). The port lives in `RS\src\bin\night_watch\`.

## 1. Goal and scope

A purely 3D Rust port of all ten Night Watch modes. The office, the building, the hunters and every camera feed are real 3D (macroquad `Camera3D`, `Mesh`, a custom lit `Material`, render targets). HUD text, panels, menus and the minimap are drawn in screen space on top of the 3D view (allowed). There is no 2D sprite fallback anywhere.

| Mode id | Menu label | Godot folder or file | Notes |
|---|---|---|---|
| classic | 1. CLASSIC | `night_watch_game.gd`, `night_watch_classic.gd`, `night_watch_stalker.gd` | Doors, lights, 8 cameras, power. Night 7 is Kathy only. |
| mask | 2. MASKED HOURS | `mask\` | No doors; mask, hall light, vent lights, music box. |
| lure | 3. DEAD AIR | `lure\` | Vents, reboots, climate. Mode id is `lure`. |
| bedroom | 4. BEDTIME | `bedroom\` | No cameras; hold A, D, F. |
| story | 5. THE FACILITY | `story\` | Task list over 5 rooms plus lift. |
| descent | 6. THE DESCENT | `descent\` | Elevator puzzle panel, fixed camera. |
| dayshift | 7. SALVAGE RUN | `dayshift\` | Shop, then a night in the cargo bay. |
| extra | 8. OPEN AIR | `openair\` | Rooftop, bus stop, campfire. Mode id is `extra`. |
| mixed | 9. MIXED SHIFT | `mixed\` | Director, drawn threats, Endless. |

Custom Night is in scope for every mode that has presets (all but mixed). The shell (title, board, custom, settings, credits, account stub, pause, end cards) is in scope.

Deliberately left out:

- VR/XR and phone touch controls: macroquad has no XR, and the keyboard and mouse build is the target.
- The online account backend (`Backend`, sign in, register, save sync, admin roster): the port has no server contract. ACCOUNT is a stub screen (guest only, sign in disabled). Guest name is kept in the save.
- The content registry, schemas, pack overrides and hot reload (`ContentRegistry`, `nw_schemas.gd`): data is compiled in, so validation happens in `--check` only.
- The shared Godot admin menu is replaced by a small dev overlay (key `0`, only in debug builds or with env `NW_DEV=1`). A used admin control marks the run DEBUG and nothing is saved.
- Godot `RandomNumberGenerator` parity: not reproducible. All modes use mulberry32 (section 3.2). Draw ORDER is kept as in GDScript so a seed is deterministic within the port.
- Settings that live in the shared Godot Settings autoload are reduced to: master volume, calm (reduced motion), scare (full, short, off), mouse sensitivity (default 0.0075).

Hard constraints: `Cargo.toml` is unchanged. Available crates: `macroquad` 0.4 (feature `audio`), `serde_json` (with `preserve_order`), `last_route_core`, `miniz_oxide`, `sha2`. `serde` itself is optional and OFF, so there is no `derive(Serialize/Deserialize)`: all JSON is read through `serde_json::Value` by hand (`v["key"].as_f64()` and so on) and written with `serde_json::json!`. `quad-storage` exists on wasm32 only; native builds save with `std::fs` under `last_route_core::shared::paths::user_file("night_watch_save.json")`.

## 2. Module tree and ownership sets

All paths are under `RS\src\bin\night_watch\`. `main.rs` finds modules with `mod core; mod gfx; ...` in `main.rs` (hub). Inside a directory, `mod.rs` re-exports the public items listed in section 3.

```
main.rs                    HUB (integrator): arg parsing, window, screen state machine, main loop
registry.rs                HUB (integrator): mode id -> constructors, playable table
cli.rs                     HUB (integrator): --mode/--level/--shot/--check, headless runner
assets.rs                  HUB (integrator): include_str!/include_bytes! of every embedded file
data/                      copied JSON (section 4), written by the integrator
core/mod.rs                core: re-exports
core/rng.rs                core: Mulberry32, Stream, Pacer, SplitSeed
core/data.rs               core: constants, NightWatchData formulas, ModesData, Building/Cast/Rules loaders
core/session.rs            core: Session (mode, level, seed, custom)
core/save.rs               core: Save (progress, settings, keys)
core/controls.rs           core: Bindings, action table, hint lines
core/input.rs              core: InputFrame (per-frame snapshot, also built by tests)
core/calls.rs              core: CallsData, CallScript (Zellia)
core/graph.rs              core: Graph (Dijkstra), Leg, Walker
core/events.rs             core: shared Cue enum, Toast
core/mode.rs               core: Mode, Headless, ShiftCtx, HudState, Outcome, EndCard
core/check.rs              core: data validation used by --check
gfx/mod.rs                 render: re-exports, Gfx struct
gfx/shader.rs              render: lit Material (glsl 100), LightSet
gfx/mesh.rs                render: Cube, Group, MeshSet, batching, draw
gfx/art.rs                 render: kinds, ramps, 16x16 textures, kind_for
gfx/prims.rs               render: wall_with_gaps, room_shell, stairs, helpers for other modes
gfx/camera.rs              render: first-person camera, FeedTarget (render targets)
gfx/office.rs              world: classic office (and OfficeConfig hooks)
gfx/building.rs            world: classic building (8 cam rooms) and camera mounts
gfx/palette.rs             world: palette + signature pass for the classic office
hunters/mod.rs             hunters: re-exports, HunterLib
hunters/bbmodel.rs         hunters: copy of lr_story/props/bbmodel.rs
hunters/anim/{mod,clip,player}.rs  hunters: copy of lr_story/anim (without runner.rs)
hunters/rig.rs             hunters: Actor (rig + model + pose + draw)
hunters/poses.rs           hunters: night_watch_poses.json -> clips "nw/<pose>"
hunters/pose_state.rs      hunters: PoseState enum and state -> pose table
hunters/place.rs           hunters: stand spots, hunter_scale, face(), yaw helper
audio/mod.rs               audio: Sounds (macroquad playback, cache), #[path] include of cues
audio/cue_names.rs         audio: NightWatchSounds mapping (event -> cue, pan, intensity)
ui/mod.rs                  ui: re-exports
ui/theme.rs               ui: palette consts, scale k
ui/text.rs                 ui: draw_text helpers, word wrap, glyph fallbacks
ui/widgets.rs              ui: button, slider, checkbox, text field, tile
shell/mod.rs               shell: Screen enum, Shell struct
shell/menu_title.rs        shell: backdrop + title screen
shell/menu_board.rs        shell: shift board
shell/menu_custom.rs       shell: custom night
shell/menu_misc.rs         shell: settings, credits, account stub, controls screen
shell/modes_data.rs        shell: re-exports core::data ModesData helpers for the board (thin)
hud/mod.rs                 hud: Hud struct
hud/status.rs              hud: status block, clock, prompt, note, crosshair, call box
hud/monitor.rs             hud: monitor overlay, static flash, feed blit
hud/cam_map.rs             hud: CamMap minimap
hud/cards.rs               hud: pause card, end card
hud/death.rs               hud: DeathMoment, death lines pick
hud/admin.rs               hud: dev overlay (key 0)
player.rs                  hud set: first-person walker, aim, station picker
modes/classic/{mod,sim,stalker,game}.rs
modes/mask/{mod,sim,controls,world,game}.rs
modes/lure/{mod,sim,world,game}.rs
modes/bedroom/{mod,sim,world,game}.rs
modes/story/{mod,sim,world,game}.rs
modes/descent/{mod,sim,world,game}.rs
modes/dayshift/{mod,sim,shop,world,game}.rs
modes/openair/{mod,sim,world,game}.rs
modes/mixed/{mod,sim,data,skin,world,game}.rs
```

Rules for every file: logic modules (anything named `sim`, `stalker`, `data`, `rng`, `calls`, `graph`, `save`, `session`, `controls`, `check`, `skin`, `poses` compile step, `pose_state`) must not call macroquad drawing, audio, window or input functions and must not use `macroquad::prelude::*` types other than `Vec3`/`Vec2`/`Color` plain value types (these are plain data and are allowed). Rendering code lives in `world`, `game`, `gfx`, `hud`, `ui`, `shell`, `audio` and `hunters/rig.rs`.

### 2.1 Core pass ownership sets (8 sets, disjoint)

| Set | Owner writes exactly these files |
|---|---|
| C1 foundation | `core/mod.rs core/rng.rs core/data.rs core/session.rs core/save.rs core/controls.rs core/input.rs core/calls.rs core/graph.rs core/events.rs core/mode.rs core/check.rs` |
| C2 render core | `gfx/mod.rs gfx/shader.rs gfx/mesh.rs gfx/art.rs gfx/prims.rs gfx/camera.rs` |
| C3 classic world | `gfx/office.rs gfx/building.rs gfx/palette.rs` |
| C4 hunters | `hunters/mod.rs hunters/bbmodel.rs hunters/anim/mod.rs hunters/anim/clip.rs hunters/anim/player.rs hunters/rig.rs hunters/poses.rs hunters/pose_state.rs hunters/place.rs` |
| C5 audio | `audio/mod.rs audio/cue_names.rs` |
| C6 ui kit and shell | `ui/mod.rs ui/theme.rs ui/text.rs ui/widgets.rs shell/mod.rs shell/menu_title.rs shell/menu_board.rs shell/menu_custom.rs shell/menu_misc.rs shell/modes_data.rs` |
| C7 hud | `hud/mod.rs hud/status.rs hud/monitor.rs hud/cam_map.rs hud/cards.rs hud/death.rs hud/admin.rs` |
| C8 player | `player.rs` (first-person controller, station picker, aim ray, collision AABBs) |

Integrator (one agent, after the core and modes passes, never in parallel with edit agents on the same file): `main.rs registry.rs cli.rs assets.rs`, the `data/` copies, and fixing any cross-set signature mismatch. The integrator edits other sets' files only to apply a listed hook.

### 2.2 Modes pass ownership sets (disjoint, one per mode)

| Set | Files |
|---|---|
| M1 classic | `modes/classic/mod.rs sim.rs stalker.rs game.rs` |
| M2 mask | `modes/mask/mod.rs sim.rs controls.rs world.rs game.rs` |
| M3 lure (Dead Air) | `modes/lure/mod.rs sim.rs world.rs game.rs` |
| M4 bedroom (Bedtime) | `modes/bedroom/mod.rs sim.rs world.rs game.rs` |
| M5 story (Facility) | `modes/story/mod.rs sim.rs world.rs game.rs` |
| M6 descent | `modes/descent/mod.rs sim.rs world.rs game.rs` |
| M7 dayshift | `modes/dayshift/mod.rs sim.rs shop.rs world.rs game.rs` |
| M8 openair (mode id `extra`) | `modes/openair/mod.rs sim.rs world.rs game.rs` |
| M9 mixed | `modes/mixed/mod.rs sim.rs data.rs skin.rs world.rs game.rs` |

Each `modes/<id>/mod.rs` exposes exactly:

```rust
pub fn new_mode(session: &crate::core::Session) -> Box<dyn crate::core::Mode>;
pub fn new_headless(session: &crate::core::Session) -> Box<dyn crate::core::Headless>;
```

No mode set edits anything outside its folder. Needs from core are requested through the integrator hook list in section 7.

## 3. Core API (exact Rust)

Conventions: time in milliseconds is `f64`, positions `macroquad::prelude::Vec3`, colours `[f32; 3]` or `Color` as stated. All public items below are re-exported from their `mod.rs` so other sets write `use crate::core::*;` / `use crate::gfx::*;` etc.

### 3.1 core/data.rs (C1)

```rust
pub const NIGHT_MS: f64 = 270_000.0;
pub const HOUR_MS: f64 = 45_000.0;
pub const TUNED_NIGHT_MS: f64 = 480_000.0;
pub const TIME_SCALE: f64 = NIGHT_MS / TUNED_NIGHT_MS;      // 0.5625
pub const NIGHTS: i32 = 7;                                   // classic
pub const MAX_AI: i32 = 20;
pub const POINTS_PER_LEVEL: i32 = 10;
pub const RATINGS: [(f64, &str); 6] = [(1.0,"Unbeatable!"),(0.8,"Perfect!"),(0.6,"Stupendous!"),(0.4,"Amazing!"),(0.2,"Fantastic!"),(0.0,"Great Job!")];

pub fn hour_index(elapsed_ms: f64) -> i32;                   // min(5, floor(e / HOUR_MS))
pub fn hour_label(h: i32) -> String;                         // "12 AM" for 0 else "{h} AM"
pub fn ai_bump(hour: i32) -> i32;                            // 2 if >=4, 1 if >=3, else 0
pub fn effective_level(base: i32, hour: i32) -> i32;         // base + bump if base > 0 else 0
pub fn move_window(level: i32) -> (f64, f64);                // lo = max(2500, 13000 - 500*level), (lo, lo + 3000)
pub fn rating_of(score: i32, max: i32) -> &'static str;
pub fn display_name(id: &str) -> String;                     // capitalised id; "mavrine" in lure keeps "The air" only inside lure's own text
pub fn level_for(id: &str, night: i32) -> i32;               // classic LEVELS table
pub fn nyxie_level(night: i32) -> i32;                       // 0 on night 7
pub fn is_kathy_night(night: i32, custom: bool) -> bool;
pub fn blackout_ms(level: i32) -> f64;                       // max(4000, 11500 - 400*level)
pub fn kathy_dip_on(elapsed_ms: f64) -> bool;
pub fn roll_hold(u: f64, lo: f64, hi: f64, tail: f64) -> f64;
pub fn distances_to(target: &str, graph: &BuildingGraph) -> std::collections::HashMap<String, i32>;
```

`ModesData` (the 9 mode JSON files, parsed with serde_json::Value into plain structs):

```rust
pub struct Preset { pub id: String, pub name: String, pub ai: Vec<(String, i32)> }
pub struct ModeEntry {
    pub id: String, pub order: i32, pub num: i32, pub label: String, pub title: String, pub tag: String,
    pub blurb: String, pub hint: String, pub levels: Vec<String>, pub scored: usize, pub open_all: bool,
    pub art: Vec<String>, pub cast: Vec<String>,
    pub custom: Option<Vec<(String, String)>>,   // slider id, label; None for mixed
    pub presets: Vec<Preset>,
}
pub struct ModesData { pub modes: Vec<ModeEntry> }      // sorted by (order, id)
impl ModesData {
    pub fn load() -> ModesData;                          // parses the include_str! blobs in crate::assets
    pub fn get(&self, id: &str) -> Option<&ModeEntry>;
}
pub struct BuildingRoom { pub id: String, pub cam: i32, pub label: String, pub pan: f64, pub grid: (i32,i32), pub map: (f64,f64) }
pub struct BuildingGraph { pub rooms: Vec<BuildingRoom>, pub edges: Vec<(String, String)>, pub adj: std::collections::HashMap<String, Vec<String>> }
pub struct Rules { /* every field of GD\data\night_watch\rules\classic.json params, f64/Vec<f64>/i32 */ }
pub struct CastDef { pub id: String, pub floor: Option<i32>, pub doors: Vec<String>, pub start: String, pub retreat_to: String,
    pub push_to: String, pub levels: Vec<i32>, pub alternate: bool, pub only_when_cams_down: bool, pub freezes_when_watched: bool,
    pub power_out: bool, pub solo_night: bool, pub combo: Option<(String, i32, f64)> }
pub fn building() -> &'static BuildingGraph;             // OnceLock, parsed from buildings/classic.json
pub fn rules() -> &'static Rules;
pub fn cast() -> &'static Vec<CastDef>;                  // file order; hunters sorted by floor via fn floor_hunters()
pub fn floor_hunters() -> Vec<&'static CastDef>;         // dovelle, ravelle, aki, ilyra
```

### 3.2 core/rng.rs (C1)

```rust
pub struct Mulberry32 { a: u32 }
impl Mulberry32 {
    pub fn new(seed: u32) -> Self;               // a = seed
    pub fn state(&self) -> u32; pub fn set_state(&mut self, s: u32);
    pub fn next(&mut self) -> f64;               // [0,1): a = a.wrapping_add(0x6d2b79f5); t = a; t = (t^(t>>15)).wrapping_mul(t|1);
                                                 // t ^= t.wrapping_add((t^(t>>7)).wrapping_mul(t|61)); ((t^(t>>14)) as f64)/4294967296.0
    pub fn range(&mut self, lo: f64, hi: f64) -> f64;
    pub fn pick(&mut self, n: usize) -> usize;   // min(n-1, floor(next*n))
    pub fn shuffle<T>(&mut self, v: &mut [T]);   // Fisher-Yates from the end: j = floor(next*(i+1))
}
pub fn jitter(base: f64, spread: f64, r: &mut Mulberry32) -> f64;       // k=clamp(spread,0,.9); base*(1-k+2k*next)
pub fn spread_ms(ms: f64, r: &mut Mulberry32, spread: f64) -> f64;      // ms*(1-s+2s*next); default spread callers pass 0.2

/// NightWatchStream: a Mulberry32 that can be a "stub" (fixed answer) or neutral (0.5).
pub struct Stream { r: Mulberry32, stub: Option<f64>, pub neutral: bool }
impl Stream {
    pub fn seeded(seed: u32) -> Stream; pub fn fixed(v: f64) -> Stream; pub fn neutral() -> Stream;
    pub fn next(&mut self) -> f64; pub fn has_state(&self) -> bool; pub fn state(&self) -> u32;
    pub fn variety_of(rng: &Stream, given: Option<Stream>) -> Stream;    // seeded((state ^ 0x7a3c19e5)) else neutral
    pub fn tempo(&mut self, spread: f64) -> f64;                          // 1/jitter(1,spread)
    pub fn chance(&mut self, p: f64) -> bool;                             // ALWAYS draws; true iff !neutral && u<p
    pub fn pick_index(&mut self, n: usize) -> usize;                      // draws; 0 if neutral
}
/// Pacer on a Mulberry32 (NightWatchMulberryPacer) or Stream (NightWatchPacer): same logic.
pub struct Pacer { pub t: f64, pub next: f64 }
impl Pacer {
    pub fn new() -> Pacer;                                                 // t=0, next=-1; draws nothing
    pub fn step(&mut self, dt_ms: f64, level: i32, draw: &mut dyn FnMut() -> f64) -> bool;
    pub fn reset(&mut self); pub fn skip(&mut self, ms: f64);
}
```

`Pacer::step` semantics: `level<=0` resets and returns false; if `next<0` draw `w.0 + draw()*(w.1-w.0)`; `t+=dt`; if `t<next` false else `t=0`, redraw `next`, true. Callers pass `&mut || rng.next()`.

### 3.3 core/session.rs, core/save.rs, core/controls.rs, core/input.rs (C1)

```rust
#[derive(Clone, Debug)]
pub struct Session { pub mode_id: String, pub night: i32, pub seed_value: u32, pub custom_levels: std::collections::BTreeMap<String, i32>, pub is_custom: bool }
impl Session {
    pub fn new() -> Session;                                    // classic, night 1, seed 0 (made fresh on read), not custom
    pub fn start_mode(&mut self, mode: &str, level: i32, custom: std::collections::BTreeMap<String,i32>, seed: Option<u32>);
        // night = max(1, level); classic clamps 1..=7; seed None -> fresh from miniquad::date::now() & 0x7fffffff; is_custom = !custom.is_empty()
    pub fn level(&self) -> i32;
    pub fn fresh_seed() -> u32;
    pub fn retry(&mut self);                                    // same mode, level, custom, SAME seed
    pub fn next_level(&mut self, modes: &ModesData, classic_nights: i32) -> bool;   // only if !is_custom && level < scored (classic: NIGHTS)
    pub fn has_next(&self, modes: &ModesData) -> bool;
}
```

`miniquad::date::now()` is the only clock for seeds (std `SystemTime` panics on wasm).

```rust
#[derive(Clone, Debug)]
pub struct Save {
    pub cleared: BTreeMap<String, Vec<u32>>, pub unlocked: u32,
    pub stars: BTreeMap<String, Vec<String>>, pub best: BTreeMap<String, i32>,
    pub custom: BTreeMap<String, BTreeMap<String, i32>>,
    pub subtitles: bool /*true*/, pub calm: bool, pub scare: String /*"full"|"short"|"off"*/,
    pub guest: String /*"GUEST_%08X"*/, pub keys: BTreeMap<String, String> /*action -> KeyCode debug name*/,
    pub master_volume: f32 /*0..100, default 100*/, pub mouse_sens: f32 /*0.0075*/, pub debug_run: bool /*never persisted*/,
}
impl Save {
    pub fn load() -> Save;                      // native: user_file; wasm: quad-storage key "night_watch_save"; any failure -> Save::default()
    pub fn store(&self);                        // no-op when debug_run
    pub fn from_json(s: &str) -> Save; pub fn to_json(&self) -> String;
    pub fn cleared_levels(&self, mode: &str) -> Vec<u32>;
    pub fn level_open(&self, mode: &str, level: i32, open_all: bool) -> bool;   // open_all || level<=1 || cleared contains level-1
    pub fn record_level_clear(&mut self, mode: &str, level: i32);               // classic also bumps unlocked; stores
    pub fn custom_unlocked(&self, mode: &str) -> bool;                          // cleared contains 5 (literal)
    pub fn record_star(&mut self, mode: &str, preset: &str); pub fn record_best(&mut self, mode: &str, score: i32);
    pub fn best_score(&self, mode: &str) -> i32; pub fn star_count(&self, mode: &str) -> usize;
    pub fn custom_values(&self, mode: &str) -> BTreeMap<String, i32>; pub fn set_custom_values(&mut self, mode: &str, v: BTreeMap<String,i32>);
    pub fn short_death_scenes(&self) -> bool;       // scare != "full"
    pub fn reduced_motion(&self) -> bool;           // calm
    pub fn reset(&mut self);                        // erase everything incl. settings; keeps nothing
    /// record_win for a finished shift: non custom -> record_level_clear, returns 0; custom -> best + stars for presets whose ai equals custom on all preset keys; returns total score.
    pub fn record_win(&mut self, session: &Session, modes: &ModesData) -> i32;
}
```

```rust
// core/controls.rs
pub struct ActionDef { pub name: &'static str, pub label: &'static str, pub key: KeyCode, pub alt: Option<KeyCode> }
pub const ACTIONS: &[ActionDef];     // nw_door_left Z, nw_door_right X, nw_light_left G, nw_light_right H, nw_lamp Y,
                                     // nw_monitor Space alt U, nw_cam_prev Left, nw_cam_next Right, nw_meet O, nw_cam_1..8 Key1..8 alt Kp1..8,
                                     // plus interact E, office_radio R, pause P (alt Escape on pause card), plus per-mode extras (below)
pub struct Bindings { /* name -> Vec<KeyCode> */ }
impl Bindings {
    pub fn from_save(save: &Save) -> Bindings;           // override replaces primary and drops alt
    pub fn rebind(&mut self, save: &mut Save, action: &str, key: KeyCode);
    pub fn clear(&mut self, save: &mut Save);
    pub fn keys(&self, action: &str) -> &[KeyCode];
    pub fn key_text(&self, action: &str) -> String;      // first key label, "?" if none
    pub fn hint_line(&self, monitor_up: bool, active: &[&str]) -> String;
}
pub fn keycode_name(k: KeyCode) -> String;   pub fn keycode_from_name(s: &str) -> Option<KeyCode>;
```

Per-mode extra actions registered in `ACTIONS` by C1 so rebinding works: mask `nw_wind` (R), `nw_hall_light` (X, alias of `nw_door_right`), mixed `nw_vent_l` V, `nw_vent_t` B, `nw_vent_r` N, `nw_vlight_l` G, `nw_vlight_t` Y, `nw_vlight_r` H, lure `nw_lure` L, `nw_reboot_audio` Q, `nw_reboot_cams` W, `nw_reboot_vent` E, `nw_reboot_heater` R, `nw_reboot_fan` T, bedroom `nw_bed_hold` A, `nw_bed_hall` D, `nw_bed_light` F, story `nw_fac_beacon` F, `nw_fac_hide` H, openair `nw_oa_tent` T, `nw_oa_fetch` F, descent `nw_hold` Space. Mode sets may only use these names; a missing one goes on the hook list.

```rust
// core/input.rs : one snapshot per frame, built by the integrator from macroquad, or by tests
#[derive(Clone, Default)]
pub struct InputFrame {
    pub down: std::collections::HashSet<&'static str>,       // actions held
    pub pressed: std::collections::HashSet<&'static str>,    // went down this frame
    pub released: std::collections::HashSet<&'static str>,
    pub digit: Option<u8>,                                    // 0..=9 pressed this frame (camera and keypad)
    pub mouse: (f32, f32), pub mouse_delta: (f32, f32),       // screen px
    pub click: bool, pub click_down: bool, pub click_right: bool,
    pub dt: f32,                                              // real seconds of this frame, already clamped to 0.1
}
impl InputFrame {
    pub fn is_down(&self, a: &str) -> bool; pub fn just(&self, a: &str) -> bool; pub fn up(&self, a: &str) -> bool;
    pub fn from_macroquad(b: &Bindings) -> InputFrame;        // only fn in core that touches macroquad input
}
```

### 3.4 core/calls.rs, core/graph.rs, core/events.rs (C1)

```rust
pub struct CallLevel { pub at_s: f64, pub lines: Vec<String> }
pub fn call_levels(mode_id: &str) -> Vec<CallLevel>;              // from data/calls/<mode>.json (classic, mask, lure, bedroom, story, descent, dayshift, extra)
pub struct CallScript { pub open: bool, pub done: bool, pub text: String, pub muted: bool /* private fields */ }
impl CallScript {
    pub fn for_level(mode_id: &str, level: i32) -> CallScript;    // empty script (done=true) for custom, mixed
    pub fn empty() -> CallScript;
    pub fn from_lines(lines: Vec<String>, at_s: f64) -> CallScript;
    pub fn update(&mut self, elapsed_ms: f64, dt_ms: f64) -> Vec<CallEvent>;   // INCOMING 2200, WORD 250, LINE_BASE 1800, LINE_WORD 380
    pub fn mute(&mut self) -> bool;
}
pub enum CallEvent { Open, Ring, Talk, Close }
```

```rust
// core/graph.rs
#[derive(Clone, Debug)] pub struct Leg { pub p: Vec3, pub jump: bool, pub key: String }
pub struct Graph { /* nodes: Vec<(String, Vec3)>, adj: Vec<Vec<(usize, f32, bool)>> */ }
impl Graph {
    pub fn new() -> Graph;
    pub fn add_node(&mut self, key: &str, p: Vec3);
    pub fn link(&mut self, a: &str, b: &str);                 // undirected, weight = distance
    pub fn link_jump(&mut self, a: &str, b: &str);            // undirected, jump edge: leg marked jump when traversed
    pub fn pos(&self, key: &str) -> Option<Vec3>;
    pub fn nearest(&self, p: Vec3) -> Option<String>;
    pub fn route(&self, a: &str, b: &str) -> Vec<Leg>;        // Dijkstra; first leg never jump; empty if no path
    pub fn legs_from(&self, pos: Vec3, to_key: &str) -> Vec<Leg>;   // from nearest node
}
pub fn walked_length(legs: &[Leg]) -> f32;                    // sum of dist for i>=1 where !jump
pub struct Walker { pub pos: Vec3, pub yaw: f32, pub room: String, pub moving: bool, pub running: bool, pub final_pos: Vec3, pub look: Vec3 /* + queue */ }
impl Walker {
    pub const WALK_SPEED: f32 = 2.3; pub const MIN_SPEED: f32 = 1.2; pub const MAX_SPEED: f32 = 6.0;
    pub fn place(&mut self, p: Vec3, f: Vec3, look: Vec3, room: &str);
    pub fn go(&mut self, legs: Vec<Leg>, to_room: &str, f: Vec3, budget_s: f32);
    pub fn step(&mut self, dt_s: f32) -> bool;                // exact algorithm from Godot night_watch_walker.gd
    pub fn is_walking(&self) -> bool;
    pub fn face(&mut self, d: Vec3);                          // yaw = atan2(d.x, d.z) + PI when |d.xz|>0.01
}
pub fn budget_for(level: i32) -> f32;                         // clamp(move_window(level).0/1000*0.25, 1.5, 4.0)
```

```rust
// core/events.rs
#[derive(Clone, Debug)] pub struct Cue { pub name: &'static str, pub pan: f32, pub intensity: f32 }   // names from crate::audio list (section 3.9)
pub struct Toast { pub text: String, pub left_s: f32 }
```

### 3.5 core/mode.rs: the Mode trait, ShiftCtx, HudState (C1)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Outcome { Running, Won, Lost }
#[derive(Clone, Debug)] pub struct EndCard { pub title: String, pub body: String, pub can_next: bool }

/// Everything a mode may read or change while a shift runs. Built per frame by main.rs.
pub struct ShiftCtx<'a> {
    pub session: &'a Session,
    pub modes: &'a ModesData,
    pub input: &'a InputFrame,
    pub bindings: &'a Bindings,
    pub save: &'a mut Save,
    pub snd: &'a mut crate::audio::Sounds,
    pub hud: &'a mut crate::core::HudState,        // pure data the Hud draws (status, prompt, note, call, monitor state)
    pub gfx: &'a mut crate::gfx::Gfx,              // shaders, textures, feed target pool
    pub hunters: &'a mut crate::hunters::HunterLib,
    pub player: &'a mut crate::player::Player,
    pub manual_clock: bool,                        // tests and --shot: snap animations, no walkers moving
    pub paused: bool,
}

#[derive(Clone, Default)]
pub struct HudState {
    pub hour: i32, pub power: f32, pub usage: i32, pub night_label: String, pub mode_text: String,
    pub power_text: Option<String>, pub usage_text: Option<String>,      // override the default "Power left: n%" / "Usage: |||" lines
    pub clock_text: Option<String>,                                       // overrides hour_label (descent, mixed endless)
    pub prompt: String, pub note: Option<(String, f32)>, pub hint: String,
    pub monitor_available: bool, pub monitor_up: bool, pub monitor_room: String, pub cam_label: String,
    pub cam_flash_s: f32, pub cam_map: Option<CamLayout>, pub cam_blips: Vec<String>, pub cam_pulse: Vec<String>,
    pub call: CallScript, pub call_text: String, pub zellia_muted: bool,
    pub bars: Vec<(String, f32, [f32;3])>,        // small labelled bars (chill, noise, beacon); drawn under the status block
    pub panels: Vec<HudPanel>,                    // free text blocks in the 480x270 space (task lists, shop lines)
    pub show_crosshair: bool,
}
pub struct HudPanel { pub x: f32, pub y: f32, pub lines: Vec<(String, [f32;3], f32 /*size*/)> }
pub struct CamLayout { pub rooms: Vec<(String, Vec2 /*x,z*/)>, pub edges: Vec<(String,String)>, pub entrances: Vec<String>,
                       pub cams: Vec<String>, pub labels: Vec<(String,String)>, pub vents: Vec<String>, pub office: Option<Vec2> }

/// Render-facing half of a mode. Created by registry::make_mode; one per shift.
pub trait Mode {
    fn id(&self) -> &'static str;
    /// Build GPU resources (meshes, rigs). Called once, after the window exists.
    fn start(&mut self, ctx: &mut ShiftCtx);
    /// Per frame, BEFORE advance: read ctx.input, change sim input state, toggle doors, open monitor, etc.
    fn handle_input(&mut self, ctx: &mut ShiftCtx);
    /// Advance the sim by dt_ms (already clamped to 100 per call; main calls repeatedly). Never called while paused or after the outcome is decided.
    fn advance_ms(&mut self, ctx: &mut ShiftCtx, dt_ms: f64);
    /// Per frame animation that runs even while paused/ended (slabs, walkers, flinch timers), real seconds.
    fn animate(&mut self, ctx: &mut ShiftCtx, dt_s: f32);
    /// Player view pass: call gfx set_camera + draw. The mode owns the 3D scene.
    fn draw_world(&mut self, ctx: &mut ShiftCtx);
    /// Draw the selected camera feed into ctx.gfx.feed (render target). Only called while hud.monitor_up. Modes without a monitor leave the default.
    fn draw_feed(&mut self, ctx: &mut ShiftCtx, room: &str) {}
    /// Screen-space extras beyond the shared Hud (keypad, shop, descent panel, mask overlay). Drawn after the Hud base, under pause/end cards.
    fn draw_overlay(&mut self, ctx: &mut ShiftCtx) {}
    fn outcome(&self) -> Outcome;
    /// Some(card) once Won or Lost; after the death moment for losses.
    fn end_card(&self, ctx: &ShiftCtx) -> Option<EndCard>;
    /// Death line source: Some((hunter_id, cause_or_empty)) when Lost.
    fn killer(&self) -> Option<(String, String)> { None }
    fn wants_mouse_free(&self) -> bool { false }         // monitor, shop, keypad, descent panel
    fn admin_entries(&mut self) -> Vec<AdminEntry> { Vec::new() }
    fn debug_win(&mut self); fn debug_lose(&mut self);
}
pub struct AdminEntry { pub heading: Option<String>, pub label: String, pub action: Box<dyn FnMut(&mut dyn Mode)> /* simple: closure over nothing; use ids */ }
```

`AdminEntry` is kept simple: `pub struct AdminEntry { pub heading: bool, pub label: String, pub id: String }` and the mode implements `fn admin_do(&mut self, id: &str)` (a default no-op method on `Mode`). Use that form (ignore the closure line above if it still appears in code).

```rust
/// Pure, window-less half. Used by --check, tests and the Mode wrapper.
pub trait Headless {
    fn id(&self) -> &'static str;
    fn step_ms(&mut self, dt_ms: f64) -> Vec<String>;      // scripted autoplay policy inside the sim (seeded); returns short event names
    fn outcome(&self) -> Outcome;
    fn elapsed_ms(&self) -> f64;
    fn digest(&self) -> u64;                               // FNV-1a of key sim state (stable across runs)
}
```

Each mode's `sim.rs` is the pure state machine (`Sim::step(dt_ms, &SimInput) -> Vec<SimEvent>`); `Headless` is a thin wrapper that feeds it a seeded autoplay input. The real `Mode` holds the same `Sim` and feeds it real input.

### 3.6 gfx: lit shader, meshes, lights, world builder (C2)

```rust
// gfx/shader.rs : glsl 100 (works on wasm/WebGL1). Pattern: src/bin/last_route/render/shader.rs (do not edit it, copy the idea).
pub const MAX_OMNI: usize = 4; pub const MAX_SPOT: usize = 3;
#[derive(Clone, Copy)] pub struct Omni { pub pos: Vec3, pub color: [f32;3], pub energy: f32, pub range: f32 }
#[derive(Clone, Copy)] pub struct Spot { pub pos: Vec3, pub dir: Vec3, pub color: [f32;3], pub energy: f32, pub range: f32, pub cos_outer: f32 }
#[derive(Clone)] pub struct LightSet { pub ambient: [f32;3], pub omni: Vec<Omni>, pub spot: Vec<Spot>, pub fog: Option<([f32;3], f32 /*density*/)> }
impl LightSet { pub fn office_default() -> LightSet; pub fn feed() -> LightSet /* ambient (0.95,0.9,0.8)*1.15, no lights */; }
pub struct LitShader { /* macroquad::material::Material */ }
impl LitShader {
    pub fn new() -> LitShader;                       // load_material; uniforms: Ambient vec3, OmniPos[4], OmniCol[4] (rgb*energy), OmniRange[4],
                                                     // SpotPos[3], SpotDir[3], SpotCol[3], SpotRange[3], SpotCos[3], FogCol, FogDensity, CamPos
    pub fn begin(&self, l: &LightSet, cam_pos: Vec3);   // gl_use_material + set_uniforms
    pub fn end(&self);                               // gl_use_default_material
}
```

Lighting model (fragment): `albedo = tex * vertex_color`; vertex `normal.w` is the mode: `1.0` lit, `0.0` unlit/emissive (the same convention as last_route: `LIT_MODE = 1.0`). `lit = ambient + sum(omni.col * max(dot(N,L),0) * pow(max(1 - d/range,0), 2.0)) + sum(spot ... * smoothstep(cos_outer, cos_outer + 0.1, dot(-L, dir)))`; output `albedo * lit`. Godot roughness is 1.0, so this Lambert model is the target. Exact falloff constants are tuned by screenshot; the structure must not change. Spot `cos_outer` is the cosine of the HALF angle (Godot `spot_angle` is a half angle).

```rust
// gfx/art.rs : NightWatchArt
pub fn kind_for(name: &str) -> &'static str;              // ordered table from night_watch_art.gd (first match of lower-cased name contains key)
pub fn is_emissive_name(name: &str) -> bool;              // case-sensitive substring of EMISSIVE_NAMES
pub struct Textures { /* 15 kinds, 16x16, Nearest, Repeat */ }
impl Textures {
    pub fn build() -> Textures;                           // after the GL context exists; uses get_internal_gl().quad_context.texture_set_wrap (see last_route/render/prop_model.rs fn repeat)
    pub fn get(&self, kind: &str) -> &Texture2D;          // unknown kind -> concrete
    pub fn solid_2x2(&mut self, name: &str, px: [[u8;4];4]) -> Texture2D;     // checker textures (FloorChecker, CheckerBand)
}
pub const RAMPS: [(&str, [u32;4]); 15];                   // wood metal plastic fabric concrete glass wall floor screen label paper hazard light red green
```

Texture generation follows the Godot per-kind tone rules (hash with a fixed per-kind salt such as FNV(name) & 0xffff; `String.hash` cannot be reproduced, any fixed salt is fine).

```rust
// gfx/mesh.rs
#[derive(Clone)]
pub struct Cube { pub name: String, pub kind: String, pub tint: [f32;3], pub pos: Vec3 /*centre, local to group*/, pub size: Vec3 /*full extents*/,
                  pub emissive: bool, pub hazard: String /*axis letters or ""*/, pub ao: f32 /*0.72*/, pub shade: f32 /*1*/, pub solid: bool, pub rot_y: f32 }
impl Cube { pub fn new(name: &str, size: Vec3, pos: Vec3) -> Cube; pub fn kind(mut self, k: &str) -> Cube; pub fn tint(mut self, t: [f32;3]) -> Cube;
            pub fn solid(mut self) -> Cube; pub fn emissive(mut self) -> Cube; pub fn hazard(mut self, axes: &str) -> Cube; pub fn ao(mut self, v: f32) -> Cube; pub fn rot_y(mut self, r: f32) -> Cube; }
#[derive(Clone)] pub struct Group { pub name: String, pub origin: Vec3, pub cubes: Vec<Cube> }
impl Group {
    pub fn new(name: &str, origin: Vec3) -> Group;
    pub fn add(&mut self, c: Cube) -> &mut Group;              // kind falls back to art::kind_for(name), emissive to art::is_emissive_name
    pub fn bx(&mut self, name: &str, size: [f32;3], pos: [f32;3]) -> &mut Cube;   // builder shortcut, returns the new cube for chaining tint/kind
}
/// World builder: collects groups, static solids (AABBs) and named stations/markers.
pub struct WorldBuilder { pub groups: Vec<Group>, pub solids: Vec<Aabb>, pub markers: std::collections::HashMap<String, Vec3> }
#[derive(Clone, Copy)] pub struct Aabb { pub min: Vec3, pub max: Vec3 }
impl Aabb { pub fn from_centre(c: Vec3, size: Vec3) -> Aabb; pub fn contains_xz(&self, x: f32, z: f32, r: f32) -> bool; pub fn ray(&self, o: Vec3, d: Vec3) -> Option<f32>; }
impl WorldBuilder { pub fn new() -> WorldBuilder; pub fn group(&mut self, name: &str, origin: Vec3) -> &mut Group; pub fn bake(self, tex: &Textures) -> World; }
/// Baked world: one Mesh per (kind, tint, emissive) batch per static group (split at 10000 verts); dynamic groups keep their own meshes.
pub struct World { /* batches, dynamic groups, solids */ pub solids: Vec<Aabb> }
impl World {
    pub fn draw(&self, shader: &LitShader, lights: &LightSet, cam_pos: Vec3);       // binds shader, draws all visible groups
    pub fn set_visible(&mut self, group: &str, v: bool);
    pub fn set_offset(&mut self, group: &str, off: Vec3);                           // slabs: translate by model matrix (get_internal_gl().quad_gl.push_model_matrix)
    pub fn set_material(&mut self, group: &str, cube: &str, kind: &str, tint: [f32;3], emissive: bool);   // lamp shade, panel buttons
    pub fn set_cube_scale(&mut self, group: &str, cube: &str, s: Vec3);
    pub fn cull_draw(&self, group_prefixes: &[&str]) /* draw only listed groups (feed pass draws building only) */;
    pub fn resolve(&self, pos: Vec3, radius: f32, dir_hint: Vec3) -> Vec3;          // slide against solids (player collision)
}
```

Cube geometry rules: exact face levels (+Y 1.20 clamped to 1.0, -Y 0.50, +Z 1.00, -Z 0.70, +X 0.88, -X 0.80), side-face bottom darkening `lerp(ao,1,clamp((y+h.y)/size.y*2.2,0,1))`, UV tile 0.5 m, hazard tape UV rules, corner order per face: as in the world-art area notes of `NW\night_watch_art.gd` (function `box_mesh`). Triangles `0,1,2 / 0,2,3`. Vertex colour carries the baked grey level multiplied by tint; texture is sampled by UV; `normal.w` = 1.0 lit or 0.0 emissive.

```rust
// gfx/prims.rs : shared helpers for all modes (so rooms are built the same way)
pub fn wall_with_gaps(g: &mut Group, along_x: bool, fixed: f32, a: f32, b: f32, gaps: &[(f32,f32)], height: f32, thick: f32, tint: [f32;3], name: &str);
pub fn room_shell(g: &mut Group, x0: f32, x1: f32, z0: f32, z1: f32, height: f32, tint: [f32;3], ceiling: bool);
pub fn stairs(g: &mut Group, origin: Vec3, steps: u32, rise: f32, run: f32, width: f32, tint: [f32;3]);
pub fn blocky_prop(g: &mut Group, kind: &str, origin: Vec3, tint: [f32;3]);   // small named props (table, crate, bed, bench...)
```

```rust
// gfx/camera.rs
pub struct FpCamera { pub pos: Vec3, pub yaw: f32, pub pitch: f32, pub fov_deg: f32 /*78*/, pub near: f32 /*0.06*/, pub far: f32 /*100*/ }
impl FpCamera { pub fn forward(&self) -> Vec3; pub fn camera3d(&self) -> Camera3D; pub fn apply(&self); }   // set_camera(&camera3d())
pub struct FeedTarget { pub rt: RenderTarget }                   // 480x270, sample_count 1, depth true, texture filter Nearest
impl FeedTarget {
    pub fn new() -> FeedTarget;
    pub fn begin(&self, pos: Vec3, target: Vec3, up: Vec3);      // Camera3D{fovy:62deg, aspect Some(16/9), render_target Some(rt), z_near .05, z_far 80}; clear (0.02,0.03,0.03)
    pub fn end(&self);                                           // set_default_camera
    pub fn draw_to_rect(&self, x: f32, y: f32, w: f32, h: f32, tint: Color);   // draw_texture_ex with flip_y: true
}
pub struct Gfx { pub tex: Textures, pub shader: LitShader, pub feed: FeedTarget, pub white: Texture2D }
impl Gfx { pub fn new() -> Gfx; }
```

Mouse look uses `InputFrame.mouse_delta`; sensitivity `save.mouse_sens`; pitch clamped to +-1.4 rad. Cursor grab (`set_cursor_grab`, `show_mouse`) is owned by main.rs only.

### 3.7 gfx/office.rs, building.rs, palette.rs (C3)

```rust
pub const HALF_W: f32 = 3.0; pub const HALF_D: f32 = 2.5; pub const HEIGHT: f32 = 3.6;
pub const SPAWN: Vec3 = vec3(0.0, 0.0, 0.9);
pub struct OfficeConfig {
    pub variant: OfficeVariant,           // Classic, Cargo (dayshift), Hub (mixed): same shell, different dressing
    pub extra: Vec<Group>,                // modes add groups (rack, ledger, vents) without editing office.rs
    pub drop_hall_end_walls: bool, pub palette: bool, pub signature: bool,
}
pub enum OfficeVariant { Classic, Cargo, Hub }
pub struct Office { pub world: World, pub spawn: Vec3, pub stations: Vec<StationDef>, /* private slab/lamp/button state */ }
pub struct StationDef { pub id: String, pub prompt: String, pub pos: Vec3, pub focus: Vec3 /*aim offset*/, pub radius: f32, pub disabled: bool }
impl Office {
    pub fn build(cfg: OfficeConfig, tex: &Textures) -> Office;
    pub fn door_stand(door: &str, slot: usize, progress: f32) -> Vec3;      // side*(lerp(4.9,3.55,p)+slot*1.3), 0, -0.4*slot - side*0.3
    pub fn animate(&mut self, door_shut: [bool;2], dt_s: f32);              // slab progress move_toward at 7/s
    pub fn snap_slabs(&mut self, door_shut: [bool;2]);
    pub fn set_light(&mut self, door: &str, on: bool); pub fn set_button(&mut self, id: &str, on: bool);
    pub fn set_lamp(&mut self, on: bool);
    pub fn lights(&self, flashlight: Option<Spot>) -> LightSet;             // lamp omni, door spots, hall glows, ambient (0.38,0.40,0.55)
    pub fn draw(&self, gfx: &Gfx, cam_pos: Vec3, flashlight: Option<Spot>);
    pub fn slab_blocks(&self, door: &str) -> bool;                          // progress > 0.41
    pub fn station_mut(&mut self, id: &str) -> Option<&mut StationDef>;
}
// gfx/building.rs
pub const ORIGIN: Vec3 = vec3(-8.0, 0.0, -40.0); pub const CELL: f32 = 8.0;
pub struct Building { pub world: World }
impl Building {
    pub fn build(tex: &Textures) -> Building;
    pub fn room_centre(room: &str) -> Vec3;
    pub fn stand_position(room: &str, slot: usize) -> Vec3;                 // (-0.6,0,0.4) / (1.0,0,-0.6); stage adds (0,0.45,-1.4)
    pub fn camera_transform(room: &str) -> (Vec3 /*pos*/, Vec3 /*target*/);  // centre+(-1.2,2.9,4.4) looking at centre+(0,0.8,-0.8)
    pub fn has_doorway(side: char, room: &str) -> bool;
    pub fn draw_feed_pass(&self, gfx: &Gfx, room: &str);                    // uses LightSet::feed()
    pub fn paths(office_origin: Vec3) -> Graph;                             // NightWatchPaths: C:/S:/L: nodes, DO:/D: office nodes with jump edge
}
```

Geometry, texture and palette numbers come from the world-art area notes. Source files: `NW\night_watch_office.gd`, `NW\night_watch_building.gd`, `NW\night_watch_art.gd`, `NW\office\night_watch_palette.gd`, `NW\office\night_watch_signature.gd` (`_classic` part), `NW\office\night_watch_office_spec.gd`. The palette pass re-tints floor, wall and ceiling cubes; the signature adds posters, orange and checker bands. FloorChecker is drawn as the floor top (or a quad with depth test LessOrEqual) to avoid z-fighting.

### 3.8 hunters (C4)

```rust
pub struct HunterLib { /* HashMap<String, Rc<Rig>> + models, loaded lazily on first use after the window exists */ }
impl HunterLib {
    pub fn new() -> HunterLib;
    pub fn actor(&mut self, id: &str) -> Option<Actor>;          // None only if the id has no embedded rig (never for the 25 ids listed in section 4)
}
pub struct Actor { pub id: String, pub pos: Vec3, pub yaw: f32, pub visible: bool, /* rig, model, AnimPlayer, last pose state, scale */ }
impl Actor {
    pub fn set_pose_state(&mut self, s: PoseState);              // plays the nw/<pose> clip (blend from poses.json) or stops to idle/walk/sprint
    pub fn set_locomotion(&mut self, moving: bool, running: bool);   // used when PoseState::Rig
    pub fn animate(&mut self, dt_s: f32);
    pub fn draw(&self, gfx: &Gfx, lights: &LightSet, cam_pos: Vec3);   // CPU skin via build_posed -> Mesh, lit shader, root = T(pos)*Ry(yaw)*S(rig.draw_scale())
    pub fn crouch(&mut self, amount: f32);                        // 0..1 lowers/flattens for vent crawl (mixed, lure) via pose "vent_crawl"
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoseState { Rig, Idle, Doorway, AtDoor, DoorGrace, PeekLeft, PeekRight, Camera, Monitor, Glass, Vent, Approach, Attack,
                     Retreat, LightFlinch, Glitch, Kneel, Tired, PointOffice, Greet, Listening, Reaching, Waiting }
pub fn pose_for_state(s: PoseState, hunter: &str) -> Option<&'static str>;   // idle->door_lurk, at_door->door_pound, ... table in poses.json "states"
pub const MODEL_SCALE: f32 = 1.5;                                     // hunters are 1.5x (about 2.58 m)
pub fn yaw_toward(from: Vec3, to: Vec3) -> f32;                       // atan2(from.x-to.x, from.z-to.z)
pub fn compile_nw_poses(rig: &mut Rig, json: &str);                   // inserts clips "nw/<pose>" into rig.clips (poses.rs)
```

Pose compile rule: for each bone present on the rig, rotation key = `key_quat(Vec3(x,y,z) degrees) * rest_q(bone)`; position key = `rest_t(Pelvis) + Vec3(x,y,z) * (rig.pelvis_rest_y / 0.796875)` in bbmodel units; loop poses use Catmull-Rom on the Euler triple with wrap, one-shots linear; flinch applies ease 0.5 (`1-(1-t)^2`). Bake on the uniform 1/30 s grid with the last key at `length`, the way `Rig::parse` does. `PoseLayers` and `VentPose` phases are not needed except vent modes (M3, M9) which call `crouch()`.

Loader copies: `hunters/bbmodel.rs` is a copy of `src/bin/lr_story/props/bbmodel.rs` (replace `super::LIT_MODE` by local `pub const LIT_MODE: f32 = 1.0; pub const GHOST_MODE: f32 = 4.0;`); `hunters/anim/*` is a copy of `src/bin/lr_story/anim/{mod,clip,player}.rs` where `asset()` is replaced by `crate::assets::rig_text(id)`. Do not edit the originals.

### 3.9 audio (C5)

Sound is synthesized (the Godot game has no audio files). The Rust repo already renders every cue bit-exactly in `src/bin/last_route/audio/cues/`. Include it unchanged:

```rust
#[path = "../last_route/audio/cues/mod.rs"] pub mod cues;     // in audio/mod.rs; cues is std-only (mod.rs and its children use only super:: and std)
```

If the compiler rejects this (the cues `tests.rs` pulls crate items), the integrator copies the folder to `audio/cues/` without `tests.rs` and removes the `mod tests;` line. API used: `cues::render(cue: &str, pan: f32, intensity: f32) -> Vec<[f32;2]>`, `cues::wav_bytes(frames) -> Vec<u8>`; cue names are `"nw_<name>"`.

```rust
pub struct Sounds { /* HashMap<String, macroquad::audio::Sound>, master volume, mute */ }
impl Sounds {
    pub fn new() -> Sounds;
    pub fn set_master(&mut self, percent: f32);
    pub fn play(&mut self, name: &str, pan: f32, intensity: f32);      // renders lazily, caches by "name|pan|intensity" snapped to 0.05; failures are silent
    pub fn cue(&mut self, c: &Cue);
    pub fn silent() -> Sounds;                                         // for tests and --check: play does nothing
}
// audio/cue_names.rs : NightWatchSounds. Every fn returns Vec<Cue> so logic tests can assert them without audio.
pub fn door(shut: bool) -> Vec<Cue>;           pub fn panel_button() -> Vec<Cue>;       pub fn monitor_toggle() -> Vec<Cue>;
pub fn camera_switch() -> Vec<Cue>;            pub fn hour_change() -> Vec<Cue>;        pub fn win() -> Vec<Cue>;
pub fn caught(short: bool) -> Vec<Cue>;        pub fn power_out() -> Vec<Cue>;          pub fn call_ring() -> Vec<Cue>;   pub fn call_line() -> Vec<Cue>;
pub fn office(kind: &str, pan: f32, vol: f32) -> Vec<Cue>;    // footsteps scrape crimson_door static drain control presence breath warning shift
pub fn attack_warning(pan: f32) -> Vec<Cue>;
pub fn hunter_event(kind: &str, pan: f32) -> Vec<Cue>;         // moved/arrived/flinch/repelled/pushed per the classic table
pub fn menu(kind: &str) -> Vec<Cue>;                           // "move" "pick" "deny"
```

Mapping: footsteps->`step`, scrape->`step`@0.5, crimson_door->`clank`, static->`burst`, drain->`power_out`, control->`move`, presence->`clank`, breath->`breath`, warning->`crackle`, shift->`chime`. Cue table, intensities and pans are in the classic-core area notes (`NW\night_watch_sounds.gd`, `GD\game\autoload\sound.gd` lines 780-1160).

### 3.10 ui kit, shell, hud, player (C6, C7, C8)

Screen-space UI is authored in a 480x270 virtual canvas (font sizes 5-22 px) and scaled by `k = min(screen_width()/480, screen_height()/270)`.

```rust
// ui/theme.rs
pub fn k() -> f32; pub fn ox() -> f32; pub fn oy() -> f32;               // scale and letterbox offset
pub const INK: Color; pub const DIM: Color; pub const RED: Color; pub const RED_TEXT: Color; pub const AMBER: Color; pub const BODY: Color; pub const PANEL: Color; // values in the shell notes
// ui/text.rs
pub fn text(s: &str, x: f32, y: f32, size: f32, c: Color);                 // virtual coords, size in virtual px
pub fn text_center(s: &str, cx: f32, y: f32, size: f32, c: Color);  pub fn text_right(s: &str, rx: f32, y: f32, size: f32, c: Color);
pub fn wrap(s: &str, width: f32, size: f32) -> Vec<String>;               // measure_text based
pub fn glyph(s: &str) -> String;     // replaces ▶ ▲ ● ★ ☆ ✔ ▮ ↑ ↓ ← → Ⅱ 🔒 … by ASCII ('>' '^' 'o' '*' '-' 'OK' '|' 'UP/DOWN' '<-' '->' 'II' '[LOCKED]' '...')
// ui/widgets.rs : immediate mode; hit tests use InputFrame.mouse converted to virtual coords
pub struct Ui { pub mouse: Vec2, pub click: bool, pub down: bool, hot: Option<u32> }
impl Ui {
    pub fn begin(input: &InputFrame) -> Ui;
    pub fn button(&mut self, r: Rect, label: &str, size: f32, style: ButtonStyle, enabled: bool) -> UiResult;   // UiResult { clicked, hovered, denied }
    pub fn slider(&mut self, r: Rect, value: f32, lo: f32, hi: f32) -> f32;
    pub fn checkbox(&mut self, r: Rect, value: bool) -> bool;
    pub fn text_field(&mut self, r: Rect, value: &mut String, secret: bool, focused: bool);
    pub fn hover_changed(&self) -> bool;       // for the "move" blip
}
pub enum ButtonStyle { Normal, Primary, Selected, Tab, Row }
```

```rust
// shell/mod.rs
#[derive(Clone, Copy, PartialEq, Eq)] pub enum Screen { Title, Board, Custom, Account, Settings, Credits, Controls }
pub struct Shell { pub screen: Screen, /* title_index, board_mode, board_level, custom_state, toast, backdrop camera/time */ }
pub enum ShellAction { None, StartShift { mode: String, level: i32, custom: BTreeMap<String, i32> }, Quit, PlayCue(Vec<Cue>) }
impl Shell {
    pub fn new(open_on: Screen, session: &Session, modes: &ModesData) -> Shell;
    pub fn update_and_draw(&mut self, input: &InputFrame, save: &mut Save, bindings: &mut Bindings, modes: &ModesData, backdrop: &mut Backdrop, gfx: &Gfx, hunters: &mut HunterLib) -> ShellAction;
}
pub struct Backdrop { /* Office built with OfficeVariant::Classic, camera fov 62 at (0,1.6,-0.25) pitch -0.16, dim rect rgba(0,.03,.04,.6) */ }
impl Backdrop { pub fn new(tex: &Textures) -> Backdrop; pub fn draw(&self, gfx: &Gfx); }
```

`ShellAction::StartShift` is what main.rs turns into `Session::start_mode` + `registry::make_mode`. `playable(id)` comes from `crate::registry::playable`.

```rust
// hud/mod.rs
pub struct Hud { /* card state, typed call text, death moment, static flash, pause/end card */ }
pub enum HudAction { None, Resume, QuitToBoard, Retry, Next, Menu, ToggleMute }
impl Hud {
    pub fn new() -> Hud;
    pub fn draw_base(&mut self, st: &HudState, bindings: &Bindings, save: &Save, gfx: &Gfx);     // status, clock, call box, prompt, note, monitor overlay (feed blit), cam map
    pub fn draw_cards(&mut self, paused: bool, end: Option<&EndCard>, input: &InputFrame) -> HudAction;
    pub fn start_death_moment(&mut self, name: &str, hunter_id: &str, cause: &str, seed: u32, depth: i32, short: bool);
    pub fn death_moment_active(&self) -> bool;
    pub fn update_death_moment(&mut self, dt_s: f32, input: &InputFrame);                       // returns when finished via death_moment_active()
    pub fn cam_map_click(&self, st: &HudState, input: &InputFrame) -> Option<String>;           // camera button hit test
}
pub fn pick_line(hunter_id: &str, cause: &str, seed: &str, depth: i32, name: &str) -> (String, bool /*caption*/, bool /*plays*/);   // copy of last_route ui/report.rs pick_line_with + {name}
```

Frame budget: Hud reads `HudState`, never the mode. Buttons that exist only in a mode's overlay (shop, keypad, descent panel) are drawn by the mode's `draw_overlay` using `ui::Ui`.

```rust
// player.rs
pub struct Player { pub cam: FpCamera, pub frozen: bool, pub look_frozen: bool, pub noclip: bool, pub crouch: bool, pub flashlight: bool, pub footprint: Option<(f32,f32,f32,f32)> }
impl Player {
    pub fn new(spawn: Vec3) -> Player;
    pub fn update(&mut self, input: &InputFrame, bindings: &Bindings, solids: &[Aabb], sens: f32);   // WASD 4.0 m/s (sprint 6.25), mouse look, collision radius 0.24
    pub fn aim(&self, stations: &[StationDef], solids: &[Aabb]) -> Option<usize>;                  // range 4.5 m, half angle 0.42 rad, LOS slack 0.06
    pub fn eye(&self) -> Vec3;                                                                       // y 1.707
    pub fn teleport(&mut self, p: Vec3, yaw: f32, pitch: f32);
}
```

### 3.11 Rendering/draw order per frame (main.rs, integrator)

1. Read `InputFrame`. 2. `shell` or `shift`. For a shift: `mode.handle_input` -> `mode.advance_ms` in chunks of at most 100 ms of `dt` (not while paused or decided) -> `mode.animate`. 3. 3D: if `hud.monitor_up` the mode draws the feed into `gfx.feed` (`draw_feed`), then `mode.draw_world`. 4. `set_default_camera`, `hud.draw_base`, `mode.draw_overlay`, `hud.draw_cards`. Cursor: grabbed while playing and `!mode.wants_mouse_free()`, free in menus, cards, pause (macroquad `set_cursor_grab`, `show_mouse`). On wasm the grab needs a click first, so arrow keys also look.

## 4. Data: copy list and embedding

The integrator copies these into `RS\src\bin\night_watch\data\` (bash `cp`), then `assets.rs` embeds them with `include_str!("data/...")`. Source root `GD\data\night_watch\`:

| Source (under `GD\data\night_watch\`) | Destination (under `src/bin/night_watch/data/`) |
|---|---|
| `modes\classic.json` `mask.json` `lure.json` `bedroom.json` `story.json` `descent.json` `dayshift.json` `extra.json` `mixed.json` | `modes\<same name>` |
| `rules\classic.json` | `rules\classic.json` |
| `buildings\classic.json` | `buildings\classic.json` |
| `casts\classic.json` | `casts\classic.json` |
| `calls\classic.json` `mask.json` `lure.json` `bedroom.json` `story.json` `descent.json` `dayshift.json` `extra.json` | `calls\<same name>` |
| `GD\assets\hunters_blocky\night_watch_poses.json` | `poses\night_watch_poses.json` |

`assets.rs` (integrator) exposes:

```rust
pub fn mode_json(id: &str) -> &'static str;      pub fn calls_json(id: &str) -> &'static str;
pub const RULES_JSON: &str; pub const BUILDING_JSON: &str; pub const CAST_JSON: &str; pub const POSES_JSON: &str;
pub fn rig_text(id: &str) -> Option<&'static str>;           // anim/<id>.json
pub fn bbmodel_text(id: &str) -> Option<&'static str>;       // <id>.bbmodel (texture is an embedded data URI inside it)
pub const DEATH_LINES_JSON: &str;
```

Hunter models and rigs are embedded from the existing last_route assets. `assets.rs` is `src/bin/night_watch/assets.rs`, so the relative paths from that file are:

```rust
include_str!("../last_route/assets/hunters/dovelle.bbmodel")      // models
include_str!("../last_route/assets/hunters/anim/dovelle.json")    // rigs
include_str!("../last_route/assets/ui/death_lines.json")
```

The 25 ids to embed (all verified to exist as both `.bbmodel` and `anim/<id>.json`): dovelle ravelle aki ilyra nyxie kathy thessine umbriel verrine haelith glyra ferryn briselle mavrine solyth wynelle xivvy loxelle javelle elise ceryth ysolde opalie porcelle quenelle. Story's Quenelle uses her own rig (the Godot stand-in is not needed). Total about 4 MB of text; acceptable for wasm.

Other embedded constants (no JSON): the 9 mode sims keep their balance tables as Rust consts (they have no JSON in Godot).

## 5. CLI, tests, build commands

`main.rs` is NOT `#[macroquad::main]`: it parses args first so `--check` never opens a window, then runs `macroquad::Window::from_config(conf, amain())`.

```
night_watch                              the full game (title screen)
night_watch --mode ID --level N          start a shift directly (ID one of classic mask lure bedroom story descent dayshift extra mixed)
night_watch --mode ID --level N --seed S fixed seed
night_watch --shot PATH [--mode ID --level N] [--frames N] [--monitor ROOM]
                                         render N frames (default 30, manual clock, fixed seed), save a PNG with get_screen_data().export_png, exit 0
night_watch --check                      no window: load and validate every data file, simulate every mode headlessly, exit 0/1
```

`--check` does, in order: (1) parse all embedded JSON (9 modes, rules, building, cast, 8 calls, poses, death lines) and assert counts and required keys (mode menu has id, title, label; 1..20 levels; every `art` and `cast` id has an embedded rig and bbmodel; every pose in `states` exists in `poses`); (2) `Building::paths`-style graph reachability for the classic graph in `core::graph` terms using the data only (no GPU); (3) for every mode id and every level in `1..=levels` run `new_headless(session)` with seed `12345` for 270 simulated seconds in 100 ms steps (descent: until Won/Lost or 600 s), assert it terminates without panic, record `digest`; run it twice and assert equal digests (determinism); (4) print one line per mode with outcome and digest; exit 1 on any failure.

Tests: every file with logic has a `#[cfg(test)] mod tests` inside it. Tests never create a window or GL object. Minimum tests per set are in section 6. Commands (agents do not run these; the build-and-fix and test agents do, one at a time, `CARGO_BUILD_JOBS=4`):

```
cargo build --bin night_watch
cargo test --no-run --bin night_watch
# then run the newest test binary:
#   target/<dir>/debug/deps/night_watch-*.exe   (pick the newest by mtime)
cargo check --target wasm32-unknown-unknown --bin night_watch
```

The wasm check catches `std::time::Instant`, threads, `std::fs` outside `cfg(not(target_arch = "wasm32"))`, and `quad-storage` misuse. Time comes from `macroquad::time::get_time()` or `miniquad::date::now()`, never `Instant` or `SystemTime`.

## 6. Per-set task briefs

Shared rules for all sets: write only your files; no `unwrap()` on data parsing (return a clear error string or a default; `--check` reports it); no new crates; write `#![allow(dead_code)]` at the top of each file in the first pass (the cleanup pass removes it); keep every public name exactly as in section 3; if you need something not listed, write down the request in your final message under "HOOKS NEEDED" and do not invent it elsewhere.

### C1 foundation

Ports: `NW\night_watch_data.gd`, `NW\night_watch_modes_data.gd`, `NW\night_watch_session.gd`, `NW\night_watch_save.gd`, `NW\night_watch_controls.gd`, `NW\night_watch_calls_data.gd`, `NW\night_watch_call_script.gd`, `NW\night_watch_stream.gd`, `NW\night_watch_pacer.gd`, `NW\night_watch_pacer_mulberry.gd`, `NW\night_watch_mulberry.gd`, `NW\night_watch_walker.gd`, `NW\night_watch_paths.gd` (graph part), `NW\night_watch_mode.gd`, plus data files in `GD\data\night_watch\`.
Done when: every item in 3.1 to 3.5 exists with the given signature; tests cover mulberry vectors (first 3 outputs for seeds 0, 1, 12345 recorded in the test), pacer windows (level 1: 12.5-15.5 s, level 20: 3-6 s), call script timing (first Ring at 2200 ms, words every 250 ms), save round trip and `level_open`/`custom_unlocked` literal rules, `Session::start_mode` clamps, Dijkstra route with a jump edge, Walker never exceeds 6.0 m/s per step, `roll_hold` bounds, `distances_to` on the classic graph (stage->doorL = 3: stage, backstage, westhall, doorL). `core/check.rs` exposes `pub fn check_data() -> Vec<String>` (list of problems) and `pub fn check_modes(modes: &dyn Fn(&Session) -> Option<Box<dyn Headless>>) -> Vec<String>`.

### C2 render core

Ports: `NW\night_watch_art.gd`, lighting (not a Godot file; follows section 3.6). Reference: `RS\src\bin\last_route\render\shader.rs`, `render\prop_model.rs` (`fn repeat`), `app\mod.rs` (`render_target_ex`).
Done when: section 3.6 compiles in principle (types, fns, shader source string present with all uniforms declared in `MaterialParams`), `art.rs` builds all 15 textures with the Godot tone rules, `kind_for` table is complete, cube geometry follows the face-level and UV rules, `WorldBuilder::bake` splits batches at 10000 vertices and 30000 indices, `World::set_offset` uses the model matrix, `FeedTarget::draw_to_rect` flips Y. Tests (pure): `kind_for("WallBack")=="wall"`, `kind_for("FloorMat")=="floor"`, `kind_for("DeskLeg")=="wood"`, `is_emissive_name("LedAlarm")`, cube vertex count 24 and index count 36, bottom-darkening values, `Aabb::ray` hit/miss, `resolve` slides along a wall.

### C3 classic world

Ports: `NW\night_watch_office.gd`, `NW\night_watch_building.gd`, `NW\office\night_watch_palette.gd`, `NW\office\night_watch_signature.gd` (classic part), `NW\office\night_watch_office_spec.gd`, `NW\night_watch_cam_map.gd` (layout data part: `classic_layout() -> CamLayout` goes in `gfx/building.rs`), `NW\office\night_watch_map_registry.gd`.
Done when: all geometry in the world-art notes exists as Groups with the stated names (so `set_material`/`set_offset` can address `SlabL`, `SlabR`, `LampShade`, `ButtonLDOOR`, `ButtonLLIGHT`, `ButtonRDOOR`, `ButtonRLIGHT`), stations defined with the exact prompts, `OfficeConfig.extra` groups are merged, `lights()` returns lamp (1.0,0.82,0.55) energy 2.2 range 5 at (0.9,1.35,-1.6), two door spots (energy 5, range 9, half angle 34 deg) and two hall glows. Tests: `door_stand("doorL",0,0.0)==(-4.9,0,0.3)`, `door_stand("doorR",1,1.0)==(4.85,0,-0.7)`, `Building::room_centre("stage")==(0,0,-40)`, `camera_transform("dining")` values, `stand_position("stage",0)` adds the platform offset, `paths()` links `L:westhall`-`DO:doorL` as a jump edge and routes stage to doorL.

### C4 hunters

Ports: `NW\night_watch_poses.gd`, `NW\night_watch_pose_layers.gd` (skip), `NW\night_watch_vent_pose.gd` (only the state hooks), `NW\night_watch_hunter_scale.gd`, `GD\game\hunters\view\model_slot.gd`, `GD\assets\hunters_blocky\night_watch_poses.json`. Reference: `RS\src\bin\lr_story\props\bbmodel.rs`, `lr_story\anim\{clip,player}.rs`, `lr_story\mannequin.rs` (`bone_matrices`, `yaw_toward`).
Done when: section 3.8 holds; `compile_nw_poses` produces finite quaternions for all 26 poses x 16 bones; states table matches poses.json `states`; `Actor::draw` skins on the CPU each frame with `normal.w = LIT_MODE`, tint white, face cell 0; fallback box figure if a model fails to parse. Tests: pose compile finite, loop poses have equal first and last key, `flinch` is not looping, `pose_for_state(Attack)=="sprint_lunge"`, `yaw_toward` quadrant checks. `cargo test` must not create GL objects: guard model loading behind `HunterLib::actor`.

### C5 audio

Ports: `NW\night_watch_sounds.gd`; reference `RS\src\bin\last_route\audio\{cues\*, engine.rs, sound.rs}`.
Done when: section 3.9 compiles in principle; `cue_names` returns the exact cue lists documented in the classic-core notes; `Sounds::play` never panics when audio is unavailable. Tests: `door(true)` gives one Cue named `door`, `hunter_event("moved", -0.8)` gives `step`, `attack_warning` gives `crackle`, cache key snapping.

### C6 ui kit and shell

Ports: `NW\night_watch_menu.gd` (+ `night_watch_menu.tscn`), `NW\night_watch_theme.gd`, `NW\theme_nightwatch.tres`, `NW\night_watch_modes_data.gd` (board side). Spec: shell area notes (title, board, custom night, account stub, settings, credits, controls screen, toasts, hints, palette, exact layouts in the 480x270 space).
Done when: all screens of section 3.10 draw and return `ShellAction`s; Title items PLAY, CUSTOM NIGHT (locked text), ACCOUNT (stub: "Accounts are not available in this build."), SETTINGS (volume, calm, scare, erase with two-press arming), CREDITS, QUIT; Board with nine rows, tile grid (3 or 4 columns), stat lines ("NOT BUILT YET" when not playable), START row; Custom with tabs, sliders, presets, All 0/10/20/Random, score line; Controls screen lists `ACTIONS` with rebind capture (Escape cancels). Backdrop is the classic office (`Office::build(OfficeConfig{variant: Classic, ..})`) with the clock text. Portrait on the board is a hunter drawn into a 192x108 render target (`hunters.actor(art[0])`), or a text card if the actor is missing. Tests (pure helper fns): board navigation wraparound, next open tile skips locked ones, custom score and rating text, `glyph()` replacements, word wrap width.

### C7 hud

Ports: `NW\night_watch_hud.gd`, `NW\night_watch_cam_map.gd` (drawing), `GD\game\hud\death_moment.gd`, `GD\game\config\death_lines.gd`, `NW\admin\night_watch_admin.gd` (dev overlay only). Spec: shell area section 6, world-art sections 6 and 7.
Done when: section 3.10 Hud compiles in principle; status block, clock, call box, note, prompt, crosshair; monitor overlay with the 16:9 feed inset (left/right 22, top 12, bottom 30), `CAM n  LABEL`, REC blink, static flash, CLOSE button and guide text; cam map 120x92 with the button layout algorithm (relax then grid pack); pause card (`PAUSED`, RESUME ESC, QUIT TO MENU BACKSPACE) and end card (title, body, NEXT ENTER if `can_next`, RETRY ENTER, MENU ESC); death moment (guard 0.4 s, word typing 0.25 s, auto dismiss 7.0 s or 2.5 s short, Escape dismisses); `pick_line` copied from `src/bin/last_route/ui/report.rs` (do not edit that file) and reading `crate::assets::DEATH_LINES_JSON`. Tests: `pick_line("dovelle","", "nightwatch:1", 1, "Dovelle")` stable across calls; kathy gives plays=false; nyxie with cause "darkness" gives a caption line; cam map pack fallback gives non-overlapping rects; typing word count at t.

### C8 player

Ports: `GD\game\player\player_controller_3d.gd` (the subset in the world-art notes), `NW\night_watch_player.tscn`, `NW\night_watch_station.gd`.
Done when: section 3.10 Player compiles in principle; yaw/pitch convention `forward = (-sin yaw * cos pitch, sin pitch, -cos yaw * cos pitch)`; walking uses `World::resolve`-style AABB sliding supplied by the caller (`solids`); `aim` returns the nearest station inside the cone with a clear line (ray against solids with 0.06 slack). Tests: forward at yaw 0 is -Z, walking into a wall stops, aim picks the nearest of two stations in the cone, ignores disabled.

### M1 classic

Ports: `NW\night_watch_game.gd`, `NW\night_watch_classic.gd`, `NW\night_watch_stalker.gd`, `NW\night_watch_station.gd` (stations are C3 data), `NW\admin\classic_admin.gd`.
Done when: `sim.rs` + `stalker.rs` port the rules 1:1 (step order, RNG draw order in `next_room` with the focus draw only when both closer and sideways are non-empty, combo patience, lines at doors, Nyxie blackout, Kathy night: fade, hold times, light dips, hum, meet), pure and unit-tested; `game.rs` implements `Mode` (PLAYING/PAUSED/WON/LOST, monitor open/close/cycle, doors, held lights, lamp, hunter placement with `Walker` and `Graph`, pose state choice, flinch 0.7 s and retreat 1.4 s timers, end cards with the exact strings, death flow through `hud`); `new_headless` autoplays (random monitor/door/light inputs seeded). Tests: Kathy-only night 7; shut door repels Dovelle after 450 ms; night wins at 270000 ms with infinite power; power out opens doors and Nyxie catches at `blackout_ms`; identical seed gives identical event list; custom night uses sliders and never Kathy.

### M2 mask

Ports: `NW\mask\*.gd` (all 8 files listed in the modes-mask-descent notes, Part A). Done when: `MaskLogic` port with typed enums, `MaskControls` panel machine (mask 200/150 ms, monitor 250/200 ms, latest press wins), wing world from boxes with the camera mounts, hall/vent/duct lights as `Spot`/`Omni`, mask overlay (full-screen cream with eye slits), music box gauge, walker waypoints; tests per the notes (battery drain, vent clock one visitor at a time, tell timing, only fully-up mask fools Thessine).

### M3 lure (Dead Air)

Ports: `NW\lure\*.gd`. Done when: `LureLogic` (draw order exactly as in the notes), station map with 8 cameras rendered one at a time to the feed target, booth with vents and panels, climate ambient shifts, side panel UI (LURE, system rows, REBOOT buttons), end strings. Tests: lure cooldown 7000 ms, every 2nd lure breaks audio, seal retreat after 2000 ms, caught at 5200 ms unsealed, stress finish at 100.

### M4 bedroom (Bedtime)

Ports: `NW\bedroom\*.gd`. Done when: `BedroomLogic` with stage clocks, battery, Haelith sleep meter p, fixed pillow camera with clamped look, hold-key input (A closet, D hall light, F bed light), two flashlight spots, hall and wing rooms, Javelle walker, death scare placements. Tests: breath to near to caught timings per night row, hall light freezes Javelle's stage clock, any light pauses Haelith, bed light drains p at 35/s, restless after 14 s idle.

### M5 story (Facility)

Ports: `NW\story\*.gd`. Done when: `StoryLogic` (tasks per night, task orders, puzzles, creepers, lift state machine), rooms in a row with doorways, glide between rooms, station presses, keypad overlay, noise and beacon HUD panel. Tests: puzzle generators never start solved (dials), switch order wrong resets, creeper strike after `strikeMs`, lift bell then open then leave, win when last task completes.

### M6 descent

Ports: `NW\descent\*.gd`. Done when: `NightWatchDescentLogic` (stages, wires, memory, pipes flow, hold, threat), elevator car world and shaft dress scroll, panel overlay with mouse hit tests and keys 1-6, Space hold, DESCENDING interstitial, Elise sill walk. Tests: pipes generated are solvable with turns in 1..2 per path tile, flow check enters west, wrong press adds 13 threat only if level>0, stage relief 34, caught at 100.

### M7 dayshift

Ports: `NW\dayshift\*.gd`. Done when: shop screen (5 upgrades, budget, quota, START SHIFT), `DayshiftLogic` (draw order in the notes), cargo bay via `OfficeConfig{variant: Cargo, extra: ...}` plus yard and ship rooms, rack pieces with progress bars, Ysolde window sequence, carry-over (a process-wide `static PENDING_CARRY: AtomicI32`). Tests: budget tables, `can_buy` rules, salvage money, quota win/short, repelled warns the other visitor when both levels >= 6.

### M8 openair (mode id extra)

Ports: `NW\openair\*.gd`. Done when: `OpenAirLogic` for three watches (pacer, torch, oil, bars, warmth, chill, fire, logs, tent, weather), three scenes (rooftop, bus stop, campfire) from boxes, particles as small cubes, per-watch HUD strings and end cards, NEXT WATCH flow for custom. Tests: climber dist and bar repel, chill catch, fire out grace 10 s, tent-in Ysolde catch, weather schedule.

### M9 mixed

Ports: `NW\mixed\*.gd`. Done when: `MixedSim` with three seeded streams (`dir`, `th`, `fx`), roster draw, six threat types, director beats, spikes, chaos events, endless rotation; 9 skins as consts; the transit hub world (areas, walls, waypoints, vent run, dressing simplified per kit) plus Hub office config with three vents; HUD strings. Tests: `draw_roster` determinism and type rules, door seal drain/regen, no kill before 1500 ms of tell, shut door pauses a tell, same seed same event list.

## 7. Integration protocol

1. Edit agents work in parallel, one set each, never compiling.
2. Each edit agent ends its message with `HOOKS NEEDED` (anything missing from this contract) and `PUBLIC API ADDED` (anything beyond it).
3. The integrator writes `main.rs`, `registry.rs`, `cli.rs`, `assets.rs`, copies `data/`, applies hooks, and resolves name mismatches.
4. `registry.rs`:

```rust
pub fn playable(mode_id: &str) -> bool;                                     // table; starts true for classic only, flipped as modes land
pub fn make_mode(session: &Session) -> Option<Box<dyn Mode>>;              // match on session.mode_id
pub fn make_headless(session: &Session) -> Option<Box<dyn Headless>>;
pub const MODE_IDS: [&str; 9] = ["classic","mask","lure","bedroom","story","descent","dayshift","extra","mixed"];
```

5. One build-and-fix agent, then one test-and-fix agent (agent build rules). Workflow order: core pass (C1 first to land signatures; C2 to C8 in parallel against this contract), integrate, build, then modes pass (M1 first, other modes in parallel), integrate, build, test.

## 8. Determinism

Simulation never reads real time, the window, the GPU or global RNG. Inputs: `(Session.seed_value, dt_ms sequence, input sequence)`. Each mode derives all streams from `seed_value` (mixed: `seed ^ 0x44495243`, `^ 0x54485254`, `^ 0x46585f31`; classic and others: one `Mulberry32::new(seed)`). HashMap iteration order is random, so ordered data uses `Vec` or `BTreeMap`. Rendering consumes sim state read-only; presentation randomness (static noise, particles) uses its own `Mulberry32` seeded from the frame count. `Headless::digest` hashes elapsed ms, outcome, power or equivalent, and the event name list, so `--check` and tests can compare runs.

## 9. KT's decisions (2026-11-08)

These override the assumptions in section 10.

- **Quenelle** uses her own rig (`../last_route/assets/hunters/quenelle.bbmodel` and `quenelle_atlas.png`) everywhere, Facility included. Do not use Verrine's model as a stand-in.
- **ACCOUNT screen is real.** It signs in to the same `lr_server` account API as Last Route: `/api/account/login`, `register`, `session`, `logout`, `guest`. The client side mirrors `src/bin/last_route/net/api.rs` (`AccountReply`, including `admin`, `admin_owner` and `admin_permissions`). Read it with `#[path]` or copy it. Saves stay local, and the session token is stored with them. Guest play still works offline.
- **Admin debug system, not an env flag.** The dev overlay (ports of `admin/night_watch_admin.gd` and each mode's `*_admin.gd`) opens only for an account the server marks `admin`, the same way Last Route gates `admin.enabled` on `backend.admin`. There is no `NW_DEV` and no debug-build gate. Tests and `--check` may pass a `force_admin` switch.
- **Board clock shows local time** on native builds, read through the OS (Windows `GetTimeZoneInformation`, Unix `localtime_r`, through `extern` FFI with no new crates). On wasm it uses the browser's timezone offset if the page provides it, and falls back to UTC.

### 9.1 Modes, maps and living hunters (KT, 2026-11-08)

- **Every mode has a real 3D map.** That means its own building or area built from its Godot `*_world.gd`, `*_map.gd` and `*_office.gd` files: rooms at the Godot coordinates and sizes, walls, doors, vents, windows, lights, and an office or station. Each map has security cameras at the Godot camera positions, with yaw and FOV, rendered live into the monitor feeds. Props are placed in the rooms, using the bbmodels in `../last_route/assets/props/` where a fitting one exists (include them with `include_str!`). Otherwise use built boxes with the mode's palette. Box-only rooms with nothing in them are not done.
- **Hunters physically walk and pathfind.** They do not teleport between cameras. Each map has a navigation graph (`hunters/nav.rs`): nodes at room stand spots, doorways, vents and task spots, with edges along walkable floor. Paths are found with A* (deterministic tie-breaks). A hunter's sim position moves along the path at its walk or run speed, so she is visible crossing a camera feed and can be seen leaving one room and entering the next. The Godot rule for *when* a hunter moves (move opportunities, AI level, doors, lures) stays as it is. Only the *how* changes: the move picks a destination, and the walker carries her there over real time. Collision and sight checks use the sim positions.
- **Idle behaviour (`hunters/behaviour.rs`).** Between moves, a hunter does one of these things, chosen per hunter from data (`data/behaviour.json`):
  - **Stare at a camera.** She turns to face the nearest camera in her room, holds a stare pose, and the feed shows a stare effect: a soft static pulse and a subtitle cue the first time it happens each night.
  - **Do a task.** She walks to a task spot in or near her room, faces it and plays a task pose. Example tasks: wipe a counter, check a locker, sort boxes, tap a terminal, sit, pace. The task spots come from each map's data.
  - **Wait.** She stands still in the Godot pose.
  Behaviour choice is seeded and deterministic. It never changes the Godot difficulty: timers that decide attacks keep running while she stares or works.

## 10. Open questions for KT (superseded where section 9 decides)

1. Quenelle in Facility: Godot uses Verrine's model as a stand-in, but Quenelle has her own rig in the Rust assets. Use Quenelle's own (assumed yes)?
2. Custom Night of OPEN AIR can never unlock in Godot (rule is "Night 5 cleared", the mode has 3 levels). Keep that literal rule, or unlock after clearing level 3 for modes with fewer than 5 levels?
3. ACCOUNT: stub only (guest) is assumed. Should the Rust build talk to the existing `lr_server` account API later?
4. Dev overlay: debug builds plus `NW_DEV=1` only. Acceptable?
5. Hunter models are real bbmodel rigs with the Night Watch pose clips, not box figures. If load time on wasm matters (about 4 MB embedded), should rigs be fetched lazily instead?
6. Where does QUIT go: exit the process (assumed), or return to the Last Route launcher?
7. Fonts: macroquad's default font lacks arrows, stars and block glyphs, so ASCII fallbacks are used. Is there an approved TTF already in the repo to load instead?
8. Local Board clock shows UTC (no timezone API on wasm). Fine, or drop to the drifting clock only?
