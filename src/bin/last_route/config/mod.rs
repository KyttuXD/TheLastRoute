//! Every number that shapes how the run plays. Values and sources are listed in MECHANICS.md;
//! "estimated" there means it was looked for in the original and not found.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

// -- player movement (game/config/game_config.gd, player_controller_3d.gd) --------------------
/// px/s in the 2D tile space; the 3D controller divides by TILE_SIZE (16) to get m/s.
pub const TILE_SIZE: f32 = 16.0;
pub const PLAYER_WALK_SPEED: f32 = 64.0 / TILE_SIZE; // 4.0 m/s
pub const PLAYER_SPRINT_SPEED: f32 = 100.0 / TILE_SIZE; // 6.25 m/s
pub const PLAYER_CROUCH_SPEED: f32 = 36.0 / TILE_SIZE; // 2.25 m/s
pub const PLAYER_GRAVITY: f32 = 12.0;
pub const PLAYER_JUMP_VELOCITY: f32 = 3.15;
pub const PLAYER_RADIUS: f32 = 0.24;
/// Godot's runner body is deliberately shorter than the 2.58 m hunter body.
pub const PLAYER_HEIGHT: f32 = 1.90;
pub const PLAYER_MODEL_SCALE: f32 = 1.067;
pub const CAMERA_HEIGHT: f32 = 1.6 * PLAYER_MODEL_SCALE;
pub const CROUCH_EYE_RATIO: f32 = 0.69;
pub const CAMERA_BOB_WALK: f32 = 0.024 * PLAYER_MODEL_SCALE;
pub const CAMERA_BOB_SPRINT: f32 = 0.038 * PLAYER_MODEL_SCALE;
pub const CAMERA_BOB_CROUCH: f32 = 0.012 * PLAYER_MODEL_SCALE;
pub const FIELD_OF_VIEW_DEG: f32 = 78.0;
pub const MOUSE_SENSITIVITY: f32 = 0.0075;
/// Mouse look is scaled so one physical mouse pixel turns the same as in the Godot original.
/// Godot's `relative` is in viewport pixels: physical pixels divided by the window's integer scale
/// k (480x270 base, canvas_items/expand, DPI-aware on Windows), so it turns `sens / k` radians per
/// physical pixel. This window is not DPI-aware (`high_dpi: false` in main.rs), so Windows reports
/// its mouse motion in logical pixels, and one physical pixel is `1 / display scale` logical pixels.
/// Matching the two gives `display scale / k`.
/// Measured on the target machine: a 2256x1504 panel at 175 % scale, Godot windowed (the default,
/// `fullscreen=false` in its saved settings), so the window is 1440x810 physical, k = 3.
/// Other machines need their own value (a 100 % display at k = 3 would use 1 / 3). Check with one
/// identical physical swipe in both builds at the same sensitivity: multiply by Godot's turn over
/// this build's turn.
pub const MOUSE_LOOK_DISPLAY_SCALE: f32 = 1.75;
pub const MOUSE_LOOK_GODOT_WINDOW_SCALE: f32 = 3.0;
pub const MOUSE_LOOK_MULTIPLIER: f32 = MOUSE_LOOK_DISPLAY_SCALE / MOUSE_LOOK_GODOT_WINDOW_SCALE;
/// Settings FOV bounds (Godot MAX_FIELD_OF_VIEW = 105 in game/autoload/settings.gd).
pub const FOV_MIN_DEG: f32 = 60.0;
pub const FOV_MAX_DEG: f32 = 105.0;
pub const SENSITIVITY_MIN: f32 = 0.0005;
pub const SENSITIVITY_MAX: f32 = 0.012;
pub const KEY_LOOK_RADIANS_PER_SECOND: f32 = 2.8;

// -- stamina ----------------------------------------------------------------------------------
// Native trace: 0x14008a600 (frame/state update).
pub const MAX_STAMINA: f32 = 100.0;
pub const STAMINA_SPRINT_DRAIN: f32 = 32.0;
pub const STAMINA_RECOVERY: f32 = 21.0;
pub const STAMINA_EXHAUSTED_RECOVERY: f32 = 25.0;
pub const STIM_INJECT_SECONDS: f32 = 0.6;
pub const STIM_BOOST_SECONDS: f32 = 6.0;
pub const STIM_SPEED_SCALE: f32 = 1.2;
pub const STIM_DRAIN_SCALE: f32 = 0.5;
pub const STIM_CRASH_SECONDS: f32 = 10.0;
pub const STIM_CRASH_REGEN_SCALE: f32 = 0.5;

// -- flashlight -------------------------------------------------------------------------------
// Native item/update path: 0x140067e40 and 0x14008a600.
pub const MAX_LAMP_CHARGE: f32 = 100.0;
pub const LAMP_IDLE_DRAIN: f32 = 0.8;
pub const LAMP_SPRINT_DRAIN: f32 = 0.35;
pub const BATTERY_RECHARGE: f32 = 25.0;

// -- noise / meters ---------------------------------------------------------------------------
pub const NOISE_CROUCH: f32 = 0.9;
pub const NOISE_WALK: f32 = 4.2;
pub const NOISE_SPRINT: f32 = 10.5;
pub const HEARING_OUTER_SCALE: f32 = 1.45;
pub const HEARING_OPEN_AIR_SCALE: f32 = 1.25;

// -- tools ------------------------------------------------------------------------------------
// Native item interaction path: 0x140084810 / 0x140067e40.
pub const TASER_RANGE: f32 = 5.0;
pub const TASER_HALF_ANGLE: f32 = 0.35;
/// Native item logic (`FUN_140185510`) applies the profile slow for three seconds.
pub const TASER_SLOW_FACTOR: f32 = 0.5;
pub const TASER_SLOW_SECONDS: f32 = 3.0;
pub const MELEE_STRIKE_RANGE: f32 = 2.45;
pub const MELEE_STRIKE_HALF_ANGLE: f32 = 0.9;
pub const MELEE_STRIKE_COOLDOWN: f32 = 1.0;
pub const CROWBAR_DURABILITY: i32 = 40;
pub const AXE_DURABILITY: i32 = 24;
pub const CROWBAR_SWING_COST: i32 = 1;
pub const AXE_SWING_COST: i32 = 6;
pub const AXE_NOISE: f32 = 18.0;
pub const AXE_NOISE_SECONDS: f32 = 0.9;
pub const BATTERY_STACK_MAX: i32 = 5;
pub const STIM_STACK_MAX: i32 = 3;
pub const HOTBAR_SIZE: usize = 5;
pub const INTERACT_REACH: f32 = 1.7;
pub const LOCKER_REACH: f32 = 1.6;

// -- hunters (cells; 1 cell = 1 m) ------------------------------------------------------------
pub const HUNTER_RADIUS: f32 = 0.33;
pub const HUNTER_GRAB_REACH: f32 = 0.9;
pub const HUNTER_REPATH_CHASE: f32 = 0.24;
pub const HUNTER_REPATH_WANDER: f32 = 1.6;
pub const HUNTER_STAMINA_MAX: f32 = 100.0;
pub const HUNTER_STAMINA_REGEN: f32 = 11.0;
pub const HUNTER_STAMINA_RECOVER: f32 = 40.0;
pub const HUNTER_WINDED_SPEED_SCALE: f32 = 0.62;
pub const HUNTER_SENSE_INTERVAL: f32 = 0.10;
pub const HUNTER_VISIBLE_CHASE_SPEED_SCALE: f32 = 1.20;
pub const UNLIT_SIGHT_SCALE: f32 = 0.5;
pub const FLASHLIGHT_SIGHT_SCALE: f32 = 3.0;
pub const FLASHLIGHT_SIGHT_MIN: f32 = 24.0;
// Standard fallbacks for the relay numbers. The game reads data/progression.json through game::progression; these
// stay for the admin constants list and the self-test, and a test in progression.rs keeps them equal to the data.
pub const HUNTER_RELAY_SPEED_BONUS: f32 = 0.1;
pub const FACILITY_RELAY_COUNT: usize = 5;
pub const RELAY_SCORE: i32 = 100;
pub const HUNTER_DEPTH_SPEED_BONUS_PER_LEVEL: f32 = 0.055;
pub const HUNTER_DEPTH_SPEED_BONUS_MAX: f32 = 0.55;
pub const HUNTER_ARRIVAL_DISTANCE: f32 = 0.65;
pub const HUNTER_WAYPOINT_DISTANCE: f32 = 0.18;
pub const UNREACHABLE_GIVE_UP: f32 = 1.6;
pub const HIDE_CHECK_REACH: f32 = 1.8;
pub const CAMP_RADIUS: f32 = 2.0;
pub const CAMP_COOLDOWN: f32 = 30.0;
pub const CAMP_LEAVE_DISTANCE: f32 = 12.0;
/// Hunter model height (2.58 m: "the runner is 1.9 m beside a 2.58 m hunter", game_config.gd).
pub const HUNTER_HEIGHT: f32 = 2.58;

// -- capture / escape -------------------------------------------------------------------------
// Native capture path: 0x14001dc8a string reference and 0x14008a600 update.
pub const ESCAPE_DURATION: f32 = 3.2;
pub const ESCAPE_PRESSES: i32 = 6;
pub const ESCAPE_COOLDOWN: f32 = 16.0;
pub const ESCAPE_GRACE: f32 = 1.5;
pub const ESCAPE_KNOCKBACK: f32 = 1.45;

// -- facility (facility_generator.gd) ---------------------------------------------------------
pub const FACILITY_COLS: i32 = 45;
pub const FACILITY_ROWS: i32 = 45;
pub const FACILITY_BATTERY_COUNT: usize = 7;
pub const FACILITY_ROOM_COUNT: usize = 9;
pub const HIDING_SPOT_COUNT: usize = 6;
pub const HALL_WIDTH: i32 = 3;
pub const LOBBY_SIZE: i32 = 11;
pub const SECTOR_GAP: i32 = 2;
pub const SECTOR_MARGIN: i32 = 2;
pub const MIN_AUTHORED_ROOM_SIZE: i32 = 10;
pub const ROOM_SECTOR_FILL: f32 = 0.86;
pub const WALL_HEIGHT: f32 = 5.0;
pub const SAFE_ZONE_RADIUS: f32 = 2.5;
pub const SAFE_ZONE_BLOCK_RADIUS: f32 = 3.15;

pub mod difficulty;
pub use difficulty::*;
