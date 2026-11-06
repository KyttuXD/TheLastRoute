//! The application: loading, the main loop (menus -> run -> death moment -> run card), input,
//! the half-resolution pixel render targets, and the `--screenshot` / `--self-test` modes.
//!
//! Screenshot hooks (with `--screenshot out.png`, captured at frame 120):
//!   LR_SHOT_SIZE=WxH        window size (e.g. 1920x1080, 844x390)
//!   LR_SCHEME=city          the City colour scheme (palette.gd SCHEMES)
//!   LR_SHOT_SCREEN=...      title | create | join | quit | account | settings | lobby | dossier | admin
//!   LR_SHOT_TAB=n           the settings tab (0 GAMEPLAY, 1 AUDIO, 2 VIDEO, 3 CONTROLS, 4 SKIN, 5 VOICE, 6 MODS)
//!   LR_SHOT_DOSSIER_TAB=n   the dossier tab (0 ROUTE DOSSIER, 1 CUSTOM HUNTERS)
//!   LR_SHOT_ACCOUNT=1       the account panel (same as LR_SHOT_SCREEN=account)
//!   LR_SHOT_RUN=1           start a run (LR_SHOT_INF=1: the Infinite route)
//!   LR_SHOT_FLASH=0         with a run: start with the flashlight off (default on)
//!   LR_SHOT_SEED=n          with LR_SHOT_RUN=1: the run seed (default 20261101)
//!   LR_SHOT_LEVEL=n         with LR_SHOT_RUN=1 (Infinite route): start on level n (-3 has spirals in open bays)
//!   LR_SHOT_STAIRS=shape    with a run: stand on the landing of the first flight down of a shape (spiral | switchback |
//!                           straight) found in the window, 1 m back from it, looking down at the well
//!   LR_SHOT_PAUSE=...       with a run: pause | statistics | players | settings | dossier | confirm
//!   LR_SHOT_ADMIN=n         with a run: the admin menu on tab n (0 STATUS .. 9 FIELDS)
//!   LR_SHOT_MAP=1           with a run: the full map
//!   LR_SHOT_DEATH=...       with a run: moment | card (a capture by the floor's hunter)
//!   LR_SHOT_UPDATE=1        the web build's UPDATING screen over whatever else is shown (with LR_SHOT_RUN=1: over a run, with LATER)

pub use crate::shared::{assets, checkpoint, paths, skins};
pub mod gfx;
pub mod log;
pub mod pacing;
pub mod perf;
pub mod platform;
pub mod profile;
pub mod public_route;
pub mod quality;
pub mod screenshot;
pub mod selftest;
pub mod settings;
pub mod stress;
pub mod updater;
pub mod web_pointer;
pub mod web_update;
#[cfg(target_arch = "wasm32")]
pub mod web_storage;

use crate::audio::{self, Sounds};
use crate::config::look::{self, LookFilter};
use crate::config::*;
use crate::content::Content;
use crate::game::{Controls, Game};
use crate::render::{scene, shader, viewmodel};
use crate::ui::menus::{self, Action, Screen, Ui};
use crate::ui::pause::{self, Pause, Sub};
use crate::ui::{hud, map, report, settings_panel, style};
use crate::net::hosted::{self, JoinProgress, OwnerAction, OwnerStep};
use crate::net::world_sync::{self, NetEvent, NetSession, RemoteView, RouteCommand};
use crate::net::{Backend, BackendConfig, BackendEvent, Pending};
use crate::world::catalog::Catalog;
use crate::world::route;
use assets::Assets;
use macroquad::prelude::*;
use screenshot::Screenshots;
use settings::Settings;

pub struct Targets {
    pub scene: RenderTarget,
    pub view: RenderTarget,
    size: (u32, u32),
}

/// The 3D view renders at half resolution and is scaled up with nearest sampling: the
/// original's restrained pixel filter (DEFAULT_PIXEL_EFFECT_PERCENT 42). Lower quality
/// settings deliberately reduce the internal render target so the GPU has less fragment work.
pub fn targets(old: Option<Targets>, quality: &str) -> Targets {
    // AUTO and HIGH preserve the reference presentation (the browser build picks AUTO's tier in app/quality.rs)
    let tier = quality::Tier::parse(quality).unwrap_or(quality::Tier::High);
    targets_scaled(old, quality::config().render_scale(tier, 0))
}

/// [`targets`] at an explicit render scale: the quality manager's (`quality::Live::render_scale`), which follows the AUTO
/// pick and the governor's notch. A new scale makes new render targets; the same size keeps the old ones.
pub fn targets_scaled(old: Option<Targets>, scale: f32) -> Targets {
    let w = (screen_width() * scale).max(64.0) as u32;
    let h = (screen_height() * scale).max(36.0) as u32;
    if let Some(t) = old {
        if t.size == (w, h) {
            return t;
        }
    }
    // new render targets hold nothing known: the held-item layer clears its target once before it skips an empty frame
    crate::render::viewmodel::forget_layer();
    let make = || {
        let rt = render_target_ex(
            w,
            h,
            RenderTargetParams {
                sample_count: 1,
                depth: true,
            },
        );
        rt.texture.set_filter(FilterMode::Nearest);
        rt
    };
    Targets {
        scene: make(),
        view: make(),
        size: (w, h),
    }
}

pub fn render_play(g: &Game, a: &Assets, mat: &Material, t: &Targets) {
    render_play_with(g, a, mat, t, &[]);
}

/// The remotes on the viewer's own level: what the HUD, the map pins and a guest event's placing may use. A teammate one floor
/// away is drawn in the 3D world but never as a dot on the wrong map.
pub fn same_level_remotes(remotes: &[RemoteView], depth: i32) -> Vec<RemoteView> {
    remotes.iter().filter(|r| r.level == depth).cloned().collect()
}

/// [`render_play`] with the other runners of an open room, drawn into the 3D view on this floor.
pub fn render_play_with(g: &Game, a: &Assets, mat: &Material, t: &Targets, remotes: &[RemoteView]) {
    scene::draw_scene(g, a, mat, &t.scene);
    {
        let _t = perf::section("remote players");
        world_sync::draw_remotes(g, a, mat, &t.scene, remotes);
    }
    // The scene's draw calls are queued by macroquad until the next camera change, which the viewmodel's camera makes. While
    // the full panel shows, they are sent here instead, so the panel can time the GL submit ("gl flush") apart from the
    // viewmodel; the same work in the same order (the camera is unchanged), only started a few lines early.
    perf::flush_timed("gl flush");
    let view_layer = {
        let _t = perf::section("viewmodel");
        viewmodel::draw_view_layer(g, a, mat, &t.view)
    };
    perf::flush_timed("gl flush view");
    set_default_camera();
    clear_background(BLACK);
    let p = DrawTextureParams {
        dest_size: Some(vec2(screen_width(), screen_height())),
        // OpenGL render targets are bottom-up; Metal's are top-down
        flip_y: !platform::is_metal(),
        ..Default::default()
    };
    draw_texture_ex(&t.scene.texture, 0.0, 0.0, WHITE, p.clone());
    // the held-item layer is blitted only when something was drawn into it (an empty layer is fully transparent)
    if view_layer {
        draw_texture_ex(&t.view.texture, 0.0, 0.0, WHITE, p);
    }
    crate::ui::markers::draw_nametags(g, a, &t.scene, &crate::ui::markers::all_markers(g, remotes));
}

/// The extra rows of the F7 panel: the render counters, then the quality manager's.
fn overlay_rows() -> Vec<(String, String)> {
    let mut rows = crate::render::draw_stats::rows();
    rows.extend(quality::rows());
    rows
}

pub struct World {
    pub content: Content,
    pub catalog: Catalog,
    pub assets: Assets,
    pub sounds: Sounds,
    /// The world material, in the lamp-count variants the quality governor switches between; it derefs to the one the
    /// current lamp cap needs, so `&w.mat` stands wherever a `&Material` is taken.
    pub mat: shader::WorldMats,
}

/// Keyboard look for one frame: Godot's `Input.get_vector` caps the combined stick at length 1,
/// so a diagonal turns no faster than a straight key, and the rate follows the mouse sensitivity
/// relative to its default (player_controller_3d.gd, `sensitivity_scale`).
fn keyboard_look(stick: Vec2, dt: f32, sens: f32) -> Vec2 {
    let stick = if stick.length() > 1.0 { stick.normalize() } else { stick };
    stick * KEY_LOOK_RADIANS_PER_SECOND * dt * (sens / MOUSE_SENSITIVITY)
}

fn read_controls(
    mouse_last: &mut Option<Vec2>,
    filter: &mut LookFilter,
    grabbed: bool,
    sens: f32,
    shop: bool,
    dt: f32,
) -> Controls {
    let mut c = Controls::default();
    if is_key_down(KeyCode::W) {
        c.fwd += 1.0;
    }
    if is_key_down(KeyCode::S) {
        c.fwd -= 1.0;
    }
    if is_key_down(KeyCode::D) {
        c.strafe += 1.0;
    }
    if is_key_down(KeyCode::A) {
        c.strafe -= 1.0;
    }
    c.sprint = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    c.crouch = is_key_down(KeyCode::C);
    c.jump = is_key_pressed(KeyCode::Space);
    c.interact = is_key_pressed(KeyCode::E) || is_mouse_button_pressed(MouseButton::Right);
    c.interact_held = is_key_down(KeyCode::E) || is_mouse_button_down(MouseButton::Right);
    c.use_item = grabbed && is_mouse_button_pressed(MouseButton::Left);
    c.flashlight = is_key_pressed(KeyCode::F);
    c.taser = is_key_pressed(KeyCode::G);
    c.drop = is_key_pressed(KeyCode::Q);
    let digits = [
        KeyCode::Key1,
        KeyCode::Key2,
        KeyCode::Key3,
        KeyCode::Key4,
        KeyCode::Key5,
    ];
    for (i, k) in digits.iter().enumerate() {
        if is_key_pressed(*k) {
            if shop {
                c.shop_choice = Some(i);
            } else {
                c.slot = Some(i);
            }
        }
    }
    // the hunter specialty bar, keys 6 to 9 (data/ui/specialty_slots.json); digits mean shop choices only while the shop
    // is open, and the shop has no slot past 5, so the specialty keys do nothing then
    if !shop {
        for (i, def) in crate::game::specialty::rules().slots.iter().enumerate() {
            if crate::game::specialty::key_code(&def.key).is_some_and(is_key_pressed) {
                c.specialty = Some(i);
            }
        }
    }
    let wheel = mouse_wheel().1;
    c.wheel = if wheel < 0.0 {
        1
    } else if wheel > 0.0 {
        -1
    } else {
        0
    };
    if grabbed {
        // Prefer the relative motion reported by the platform.  Some Windows pointer
        // backends only start producing it after the cursor has been moved once, so keep a
        // position-delta fallback as well; otherwise the first mouse turn can appear dead.
        let pos: Vec2 = mouse_position().into();
        let screen = vec2(screen_width(), screen_height());
        let look_cfg = look::cfg();
        let px = look::delta_px(
            mouse_delta_position(),
            screen,
            screen_dpi_scale(),
            mouse_last.map(|last| pos - last),
        );
        // the settle after a capture, the warp-spike rejection and the per-frame cap (config/look.rs)
        let px = filter.apply(px, dt, look_cfg);
        c.look = look::px_to_radians(px, sens, look_cfg.multiplier());
        *mouse_last = Some(pos);
    } else {
        *mouse_last = None;
    }
    let mut stick = Vec2::ZERO;
    if is_key_down(KeyCode::J) || is_key_down(KeyCode::Left) {
        stick.x -= 1.0;
    }
    if is_key_down(KeyCode::L) || is_key_down(KeyCode::Right) {
        stick.x += 1.0;
    }
    if is_key_down(KeyCode::I) || is_key_down(KeyCode::Up) {
        stick.y -= 1.0;
    }
    if is_key_down(KeyCode::K) || is_key_down(KeyCode::Down) {
        stick.y += 1.0;
    }
    c.look += keyboard_look(stick, dt, sens);
    c
}

/// Saves the window's current picture (the framebuffer reads bottom-up). Under Metal there is no read-back
/// (miniquad's Metal backend and macroquad's `grab_screen` are OpenGL only), so nothing is written and the log says so.
pub fn save_screen(path: &str) {
    if platform::is_metal() {
        crate::lr_warn!("screenshot {path} skipped: the Metal window cannot be read back (use --gfx gl)");
        return;
    }
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut img = get_screen_data();
    for px in img.bytes.chunks_mut(4) {
        px[3] = 255;
    }
    img.export_png(path);
}

/// The world with no boot card: the headless and self-test paths.
pub async fn load_world() -> World {
    let content = Content::load();
    let catalog = Catalog::load();
    let assets = Assets::load(&content);
    let sounds = Sounds::new().await;
    let mat = shader::WorldMats::new();
    World {
        content,
        catalog,
        assets,
        sounds,
        mat,
    }
}

/// The game's own load, the same steps as `load_world`, each reported to the boot card (ui/boot.rs):
/// the route data, the art, then the signal (sounds and the world material). A load with no hunters
/// or no room types fails the card.
pub async fn load_world_booted(boot: &mut crate::ui::boot::BootLoader) -> World {
    let t = crate::shared::clock::Instant::now();
    let content = Content::load();
    crate::lr_info!("startup timing: content {} ms", t.elapsed().as_millis());
    let catalog = Catalog::load();
    crate::lr_info!("startup timing: catalog {} ms", t.elapsed().as_millis());
    if content.hunters.is_empty() || catalog.types.is_empty() {
        boot.fail(crate::ui::boot::FAIL_LOAD);
    }
    boot.step(crate::ui::boot::Step::Route).await;
    crate::lr_info!("startup timing: route step {} ms", t.elapsed().as_millis());
    let assets = Assets::load(&content);
    crate::lr_info!("startup timing: assets {} ms", t.elapsed().as_millis());
    boot.step(crate::ui::boot::Step::Art).await;
    crate::lr_info!("startup timing: art step {} ms", t.elapsed().as_millis());
    let sounds = Sounds::new().await;
    crate::lr_info!("startup timing: sounds {} ms", t.elapsed().as_millis());
    let mat = shader::WorldMats::new();
    crate::lr_info!("startup timing: world material {} ms", t.elapsed().as_millis());
    boot.step(crate::ui::boot::Step::Signal).await;
    crate::lr_info!("startup timing: signal step {} ms", t.elapsed().as_millis());
    World {
        content,
        catalog,
        assets,
        sounds,
        mat,
    }
}

/// A run waiting behind the BUILDING ROUTE screen (route_loading.gd).
struct Loading {
    diff: Difficulty,
    /// The run seed (the decimal of the room text's djb2 for a new run, the saved one otherwise).
    seed: u32,
    /// The run's room seed text, shown on the loading screen and carried into the run.
    room: String,
    /// Infinite level to enter (0 = top), or None for a finite route at `depth`.
    level: Option<i32>,
    depth: i32,
    /// Absolute feet y to resume at (a saved infinite run), or None for the floor of `level`.
    y: Option<f32>,
    score: i32,
    coins: i32,
    frames: u32,
    built: Option<Game>,
    shown: f32,
}

impl Loading {
    fn new(diff: Difficulty, seed: u32, room: String, level: Option<i32>) -> Loading {
        Loading {
            diff,
            seed,
            room,
            level,
            depth: 1,
            y: None,
            score: 0,
            coins: 0,
            frames: 0,
            built: None,
            shown: 0.0,
        }
    }

    /// A run of a room seed text: its run seed is the text's djb2 (RoomState.run_seed).
    fn for_room(diff: Difficulty, room: String, level: Option<i32>) -> Loading {
        let seed = route::run_seed_of_text(&room);
        Loading::new(diff, seed, room, level)
    }

    /// A new run from a new room seed text (route::new_room_text).
    fn new_room(diff: Difficulty, level: Option<i32>) -> Loading {
        Loading::for_room(diff, route::new_room_text(), level)
    }

    fn from_checkpoint(c: &checkpoint::Checkpoint) -> Loading {
        let mut l = Loading::new(
            c.difficulty,
            c.run_seed,
            c.room_text.clone(),
            if c.infinite { Some(c.level()) } else { None },
        );
        l.depth = c.depth.max(1);
        l.y = if c.infinite { Some(c.migrated_y()) } else { None };
        l.score = c.score;
        l.coins = c.coins;
        l
    }

    fn build(&self, w: &World) -> Game {
        let mut g = match self.level {
            Some(level) => Game::new_infinite(
                self.diff,
                self.seed,
                &self.room,
                level,
                &w.content,
                &w.catalog,
                &w.assets,
            ),
            None => {
                let mut g = Game::new(self.diff, self.seed, &w.content, &w.catalog, &w.assets);
                g.room_text = self.room.clone();
                if self.depth > 1 {
                    g.depth = self.depth;
                    g.start_floor(&w.content, &w.catalog, &w.assets);
                }
                g
            }
        };
        self.place_resumed(&mut g);
        g.score = self.score;
        g.coins = self.coins;
        g
    }

    /// A resumed infinite run stands at `y` (`Checkpoint::migrated_y`: the saved height when it was standing near
    /// its floor, the floor of the level otherwise).
    fn place_resumed(&self, g: &mut Game) {
        if let (Some(y), true) = (self.y, g.inf.is_some()) {
            // a saved run resumes at its absolute height (a legacy save at the floor of its level)
            g.p.set_abs_y(y);
            // a window with a noisy floor (the Inferno) puts the body on the real surface, not on the flat floor line
            if g.fac.terrain().is_some() {
                g.p.stand_on_ground(&g.fac);
            }
        }
    }
}

/// The end of a run: the death moment, then the run card.
enum Death {
    Moment(report::DeathMoment),
    Card { focus: usize },
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A room request that is in flight. No client hosts an online room (the server does, `net/hosted.rs`): a player's game
/// joins a room by its code, or takes part in the public route.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NetIntent {
    Join,
    /// PUBLIC ROUTE: the session turns out hosted or joined once the directory has answered. It is the one room a client
    /// still hosts, until the public route becomes an always-on server room.
    Public,
}

/// A line of the owner's feedback stays this long.
const OWNER_STATUS_SECONDS: f64 = 6.0;

/// The owner's command from the lobby or the pause menu: taken only when this player owns the open room, and one at a time.
fn submit_owner(cmd: &mut hosted::OwnerCommander, ui: &mut Ui, in_room: bool, action: OwnerAction, until: &mut f64, now: f64) -> bool {
    if !ui.owner.owner || !in_room {
        return false;
    }
    let taken = cmd.submit(action);
    let accepted = taken.is_ok();
    ui.owner.status = match taken {
        Ok(note) | Err(note) => note,
    };
    *until = now + OWNER_STATUS_SECONDS;
    accepted
}

/// A line for the title or lobby (`ui.notice`, four seconds).
fn flash(ui: &mut Ui, text: &str) {
    ui.notice = text.to_string();
    ui.notice_t = 4.0;
}

/// A room event during a run is announced on the HUD; outside one it is a title notice.
fn show_room_notice(playing: Option<&mut Game>, ui: &mut Ui, text: &str) {
    match playing {
        Some(g) => g.announce(text),
        None => flash(ui, text),
    }
}

fn shot_screen(name: &str) -> Option<Screen> {
    Some(match name {
        "title" => Screen::Title,
        "create" => Screen::Create,
        "join" => Screen::Join,
        "quit" => Screen::Quit,
        "account" => Screen::Account,
        "settings" => Screen::Settings,
        "lobby" => Screen::Lobby,
        "dossier" => Screen::Dossier,
        "admin" => Screen::Admin,
        _ => return None,
    })
}

pub async fn run(screenshot: Option<String>, stress_cfg: Option<stress::Config>, update: Option<updater::UpdateHandle>) {
    // a stress run (`--stress`) opens its log before the load, so a bad --stress-log path fails at once
    let stress_on = stress_cfg.is_some();
    let stress_vsync = stress_cfg.as_ref().map(|c| c.vsync);
    let stress_cap = stress_cfg.as_ref().and_then(|c| c.fps_limit);
    let mut stress_run = match stress_cfg.map(stress::Stress::open) {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => {
            eprintln!("--stress: {e}");
            std::process::exit(2);
        }
        None => None,
    };
    // the window is created at the system's default position; keep it on the work area (above the taskbar)
    if screenshot.is_none() {
        platform::centre_on_work_area();
    }
    if screenshot.is_some() {
        if let Some((Some(sw), Some(sh))) = env("LR_SHOT_SIZE").and_then(|s| {
            s.split_once('x')
                .map(|(a, b)| (a.parse::<f32>().ok(), b.parse::<f32>().ok()))
        }) {
            request_new_screen_size(sw, sh);
            for _ in 0..3 {
                next_frame().await;
            }
        }
    }
    // the boot loader (ui/boot.rs) draws its card before the first load step and yields a frame after each
    // a stress run skips the boot card (ui/boot.rs) and loads with none
    let mut boot = crate::ui::boot::BootLoader::new();
    // the web build's UPDATING screen (ui/update_screen.rs): drawn last, holds the input while it is up
    let mut update_screen = crate::ui::update_screen::Overlay::new();
    if let Some(h) = update {
        boot.set_update(h);
    }
    let w = if stress_on {
        load_world().await
    } else {
        boot.pump().await;
        load_world_booted(&mut boot).await
    };
    let mut settings = Settings::load();
    style::set_scheme(&env("LR_SCHEME").unwrap_or_else(|| settings.scheme.clone()));
    style::set_reduced_motion(settings.reduced_motion);
    if settings.fullscreen && screenshot.is_none() {
        set_fullscreen(true);
    }
    // the quality manager: the AUTO tier (the browser's hardware report, then the first seconds of play), the governor, the
    // lamp cap and the render scale (app/quality.rs). A native build has no report and keeps AUTO = HIGH.
    let mut qm = quality::Manager::new(quality::config(), quality::Hardware::from_page());
    let live = qm.update(&settings.quality, settings.adaptive_quality, settings.fps_limit, 0.0, false);
    qm.publish(&live, &settings.quality);
    let mut tg = targets_scaled(None, live.render_scale);
    let mut ui = Ui::default();
    // a stress run starts its route at once, with no title
    let mut playing: Option<Game> = if stress_on {
        Some(stress::start_game(&w))
    } else {
        None
    };
    let mut diff = Difficulty::Standard;
    // the lobby's room seed text (ROOM-<code>-<hex>); the run seed is its djb2
    let mut room = route::new_room_text();
    let mut pz = Pause::default();
    let mut map_open = false;
    // the fullscreen map's pan, zoom and double click (ui/map.rs); recentred each time it opens
    let mut map_view = map::FullView::default();
    let mut hud_hidden = false;
    let mut shots = Screenshots::default();
    let mut death: Option<Death> = None;
    let mut last_save: Option<checkpoint::Checkpoint> = None;
    let mut save_poll = 0.0f32;
    let mut mouse_last: Option<Vec2> = None;
    // the look filter: a short settle after a re-lock while the cursor moves to the capture point, then spike rejection
    let mut look_filter = LookFilter::default();
    let mut grabbed = false;
    let mut save = checkpoint::load();
    if !stress_on {
        boot.step(crate::ui::boot::Step::Saves).await;
    }
    let mut loading: Option<Loading> = None;
    let mut frame = 0u32;
    let mut audio_volumes = (f32::NAN, f32::NAN, f32::NAN);
    // the admin tools: the signed-in admin's account (KYTTU or AYTTU) only, like Build.tools_enabled
    let mut admin = crate::ui::admin::Admin::new();
    let mut hub: Option<crate::ui::admin::HubPanel> = None;
    let (mut fps_t, mut fps_n, mut fps) = (0.0f32, 0, 60);
    // the VSYNC row, as last applied to the live GL context; the FPS LIMIT row paces the loop
    let mut vsync_applied: Option<bool> = None;
    let mut pacer = pacing::Pacer::default();
    // the room: the open session (host or guest), the request still in flight, and the directory it talks to
    // (`--backend=` applies here like to the menus; release builds use the primary server)
    let net_cfg: BackendConfig = BackendConfig::from_args(std::env::args().skip(1), cfg!(debug_assertions)).0;
    // runner skins are fetched from the same website the account uses (never when offline)
    skins::set_backend_base(if net_cfg.offline { "" } else { &net_cfg.base_url });
    // the account session: the title start checks the saved one once, and each frame takes the answers below
    let mut backend = Backend::new(net_cfg.clone(), &crate::net::store::default_dir());
    if backend.account.session_token.is_empty() && !settings.username.is_empty() {
        // a name with no saved session was never confirmed by the server: the title shows it signed out
        settings.username.clear();
        // a stress run never writes the settings file
        if !stress_on {
            settings.save();
        }
    }
    backend.restore_session();
    // the skin the local runner wears, resolved once per change to (wire hash, slim arms); the first-person hands, the
    // upload and the room all take it from here, never from the list id (which is CUSTOM for every applied picture)
    let mut worn = skins::WornTracker::default();
    // a skin was worn and DEFAULT was chosen since: the website's copy is cleared once the account is signed in
    let mut had_skin = false;
    let mut clear_pending = false;
    // when the worn skin is next offered to the website (a new hash at once, a refused one again after a delay)
    let mut upload_plan = skins::UploadPlan::default();
    let upload_retry = skins::config().map_or(60.0, |c| c.upload_retry_secs);
    // the skin last offered to the open room, the uploaded count it was judged against, and when to look again
    let mut room_skin_seen = String::new();
    // whether this runner's chosen skin and ARMS setting give slim arms, sent in its sample so peers draw the same arms
    let mut room_uploaded_seen = usize::MAX;
    let mut room_skin_check = 0.0f64;
    // when the SKIN tab's sharing line is next refreshed (it reads a small file)
    let mut share_text_at = 0.0f64;
    let mut net: Option<NetSession> = None;
    let mut net_pending: Option<(NetIntent, Pending<NetSession>)> = None;
    // CREATE ROOM (online): the server's answer, then the wait for the room to be listed and the guest join
    let mut hosted_create: Option<Pending<hosted::Created>> = None;
    let mut hosted_join: Option<hosted::HostedJoin> = None;
    // the room owner's commands: a ticket from the directory, then the command to the room
    let mut owner_cmd = hosted::OwnerCommander::default();
    let mut owner_status_until = 0.0f64;
    if screenshot.is_some() {
        if let Some(s) = env("LR_SHOT_SCREEN").as_deref().and_then(shot_screen) {
            ui.go(s);
        }
        if env("LR_SHOT_ACCOUNT").is_some() {
            ui.go(Screen::Account);
        }
        if let Some(t) = env("LR_SHOT_TAB").and_then(|t| t.parse().ok()) {
            ui.settings_tab = t;
        }
        if let Some(t) = env("LR_SHOT_DOSSIER_TAB").and_then(|t| t.parse().ok()) {
            ui.dossier_tab = t;
        }
    }

    loop {
        frame += 1;
        gfx::boot_guard_tick();
        // a stress run takes --stress-vsync without touching the saved setting
        let vsync = stress_vsync.unwrap_or(settings.vsync);
        if vsync_applied != Some(vsync) {
            pacing::apply_vsync(vsync);
            vsync_applied = Some(vsync);
        }
        perf::frame(get_frame_time());
        perf::handle_keys();
        if let Some(st) = stress_run.as_mut() {
            if st.frame(get_frame_time(), playing.as_ref()) {
                st.finish();
                log::flush();
                std::process::exit(0);
            }
        }
        let raw_dt = get_frame_time();
        let dt = raw_dt.min(0.05);
        if update_screen.is_shown() {
            update_screen.swallow_input();
        }
        let arms = skins::ArmsSetting::parse(&settings.arms).unwrap_or(skins::ArmsSetting::Auto);
        if worn.update(&settings.skin, settings.skin_slim, &settings.arms, skins::generation(), || {
            skins::worn_skin(&settings.skin, settings.skin_slim, arms)
        }) {
            let w = worn.get();
            crate::render::fp_hands::set_look(&w.hash, w.slim);
            if w.hash == world_sync::SKIN_DEFAULT {
                clear_pending = had_skin;
                had_skin = false;
            } else {
                had_skin = true;
                clear_pending = false;
            }
        }
        ui.t += dt;
        ui.notice_t = (ui.notice_t - dt).max(0.0);
        fps_t += dt;
        fps_n += 1;
        if fps_t >= 0.5 {
            fps = (fps_n as f32 / fps_t).round() as i32;
            fps_t = 0.0;
            fps_n = 0;
        }
        // only a frame that drew the 3D world counts towards the AUTO pick and the governor (not menus, loading or the pause screen)
        let in_play = playing.is_some() && loading.is_none() && !pz.open && !stress_on;
        let live = qm.update(&settings.quality, settings.adaptive_quality, settings.fps_limit, raw_dt * 1000.0, in_play);
        qm.publish(&live, &settings.quality);
        tg = targets_scaled(Some(tg), live.render_scale);
        // a narrower lamp variant is compiled the first frame it is needed; do it here, not mid-draw
        w.mat.prewarm(crate::render::shader::lamp_cap());
        let next_audio_volumes = (
            settings.master_volume,
            settings.gameplay_volume,
            settings.ui_volume,
        );
        if next_audio_volumes != audio_volumes {
            audio::set_volumes(
                next_audio_volumes.0,
                next_audio_volumes.1,
                next_audio_volumes.2,
            );
            audio_volumes = next_audio_volumes;
        }
        // delayed cues and timed loops run on every frame, paused or on the report screen too (game `update` is skipped then)
        audio::tick();
        if is_key_pressed(KeyCode::F11) {
            settings.fullscreen = !settings.fullscreen;
            set_fullscreen(settings.fullscreen);
            settings.save();
        }
        // read now, saved at the end of the frame, after everything is drawn
        let shot_key = is_key_pressed(KeyCode::F2);
        let s = style::ui_scale();

        // the worn skin goes to the website once the account is signed in (in the background): keyed on its hash, so a
        // second picture applied in the same session uploads too, and a refused upload is offered again after a delay
        if !net_cfg.offline && skins::verify_held(&worn.get().hash) && upload_plan.due(get_time(), &worn.get().hash, backend.signed_in(), skins::uploaded_ids, upload_retry) {
            skins::upload_in_background(net_cfg.clone(), backend.account.session_token.clone(), worn.get().hash);
        }
        // DEFAULT was picked: the website stops holding the old skin (the call is retried on later frames until it is accepted)
        if clear_pending && !net_cfg.offline && backend.signed_in() {
            clear_pending = false;
            skins::clear_in_background(net_cfg.clone(), backend.account.session_token.clone());
        }

        // the account answers: the profile name is set only when the server confirms it
        for event in backend.update() {
            match event {
                BackendEvent::SignedIn(reply) => {
                    settings.username = reply.username;
                    if !stress_on {
                        settings.save();
                    }
                    ui.acct_user.clear();
                    ui.acct_pass.clear();
                    ui.acct_status.clear();
                }
                BackendEvent::SignedOut => {
                    // a sign out, or a saved session the server refused: a sign in that came since keeps its name
                    if !backend.signed_in() {
                        settings.username.clear();
                        if !stress_on {
                            settings.save();
                        }
                    }
                }
                BackendEvent::AccountError(text) => ui.acct_status = text,
                _ => {}
            }
        }
        ui.acct_busy = backend.account_pending();

        // a host or join request that has answered: adopt the session and show its lobby
        let answered = net_pending.as_mut().and_then(|(intent, p)| p.poll().map(|r| (*intent, r)));
        if let Some((intent, result)) = answered {
            net_pending = None;
            match result {
                Ok(mut session) => {
                    let code = session.code.clone();
                    // PUBLIC ROUTE is a host when nobody held the room, else a guest of the runner who does; a JOIN is
                    // always a guest (no client hosts an online room)
                    let public = intent == NetIntent::Public;
                    match session.is_host() {
                        true => {
                            room = world_sync::room_text_for_code(&code, &route::new_room_text());
                            // the guests are sent the text this host runs on, so both start the same route
                            session.set_room_seed(room.clone());
                            ui.room_mode = menus::RoomMode::Host;
                            ui.go(Screen::Lobby);
                            flash(&mut ui, &format!("ROOM {code} IS OPEN // FRIENDS JOIN WITH THIS CODE"));
                            // the public route has no lobby to wait in: the runner who opens it starts it
                            if public && crate::net::public_route::rules().start_when_hosting {
                                loading = Some(Loading::for_room(diff, room.clone(), Some(0)));
                                session.start_run(&room, diff.name());
                            }
                        }
                        false => {
                            room = format!("ROOM-{code}-");
                            ui.room_mode = menus::RoomMode::Guest;
                            ui.go(Screen::Lobby);
                            flash(&mut ui, &format!("JOINED ROOM {code} // WAITING FOR THE ROUTE"));
                        }
                    }
                    // a new room is offered the chosen skin on the next frame
                    room_skin_seen.clear();
                    room_uploaded_seen = usize::MAX;
                    net = Some(session);
                }
                Err(e) => {
                    let text = e.message_or("ROOM REQUEST FAILED");
                    match intent {
                        NetIntent::Public => flash(&mut ui, &text),
                        NetIntent::Join if ui.screen == Screen::Join => ui.join_status = text,
                        NetIntent::Join => flash(&mut ui, &text),
                    }
                }
            }
        }

        // CREATE ROOM (online): the directory has opened a room (or refused), then the room is checked until it is listed
        // and joined as a guest. Leaving for the title cancels the wait.
        if ui.screen == Screen::Title {
            hosted_create = None;
            hosted_join = None;
        }
        if let Some(result) = hosted_create.as_mut().and_then(|p| p.poll()) {
            hosted_create = None;
            match result {
                Ok(created) => {
                    room = format!("ROOM-{}-", created.code);
                    ui.room_mode = menus::RoomMode::Guest;
                    ui.go(Screen::Lobby);
                    hosted_join = Some(hosted::HostedJoin::new(&created.code, &created.owner, crate::shared::clock::Instant::now()));
                }
                Err(e) => {
                    let text = hosted::create_error_text(&e);
                    ui.create_note = text.clone();
                    flash(&mut ui, &text);
                }
            }
        }
        let join_progress = hosted_join.as_mut().map(|join| {
            let (cfg, code) = (net_cfg.clone(), join.code.clone());
            let name = world_sync::clean_name(&settings.username);
            let skin = worn.get().hash;
            join.update(crate::shared::clock::Instant::now(), move || world_sync::join_room(cfg, code, name, skin, String::new(), false))
        });
        match join_progress {
            Some(JoinProgress::Joined(session)) => {
                let join = hosted_join.take();
                let code = session.code.clone();
                let owned = hosted::is_room_owner(
                    if session.room_owner().is_empty() { join.as_ref().map_or("", |j| j.owner.as_str()) } else { session.room_owner() },
                    backend.signed_in(),
                    &backend.account.username,
                );
                room = format!("ROOM-{code}-");
                ui.room_mode = menus::RoomMode::Guest;
                let text = if owned { hosted::fill(&hosted::rules().strings.opened_owner, &code, 0, 0, "") } else { format!("JOINED ROOM {code} // WAITING FOR THE ROUTE") };
                flash(&mut ui, &text);
                room_skin_seen.clear();
                room_uploaded_seen = usize::MAX;
                net = Some(*session);
            }
            Some(JoinProgress::Failed(text)) => {
                hosted_join = None;
                ui.go(Screen::Title);
                ui.room_mode = menus::RoomMode::Solo;
                flash(&mut ui, &text);
            }
            _ => {}
        }
        // what the lobby and the pause menu show of the room's owner: the controls only to the account that created the
        // room, joined as a guest of an online room (nobody else, not a signed-out player)
        {
            let room_owner = net
                .as_ref()
                .map(|n| n.room_owner().to_string())
                .or_else(|| hosted_join.as_ref().map(|j| j.owner.clone()))
                .unwrap_or_default();
            let online_guest = net.as_ref().map_or(hosted_join.is_some(), |n| !n.is_host());
            ui.signed_in = backend.signed_in();
            ui.owner.owner = hosted::owner_controls_visible(online_guest, &room_owner, backend.signed_in(), &backend.account.username);
            ui.owner.joined = net.as_ref().is_some_and(|n| !n.is_host() && n.linked());
            ui.owner.peers = net.as_ref().filter(|_| ui.owner.owner).map(|n| n.roster_peers()).unwrap_or_default();
            ui.owner.waiting = hosted_join.as_ref().map(|j| j.status_lines());
            if net.is_none() {
                owner_cmd.cancel();
            }
            if !owner_cmd.busy() && get_time() >= owner_status_until {
                ui.owner.status.clear();
            }
        }
        // an owner command waits for the directory's ticket, then goes to the room
        if owner_cmd.busy() {
            let (cfg, code, token) = (net_cfg.clone(), net.as_ref().map(|n| n.code.clone()).unwrap_or_default(), backend.account.session_token.clone());
            match owner_cmd.update(move || Pending::spawn(move || hosted::issue_owner_ticket(&cfg, &code, &token))) {
                OwnerStep::Idle => {}
                OwnerStep::Say(text) => {
                    ui.owner.status = text;
                    owner_status_until = get_time() + OWNER_STATUS_SECONDS;
                }
                OwnerStep::Send { ticket, action } => {
                    if let Some(n) = net.as_mut() {
                        n.send_owner_command(ticket.reveal(), &action);
                    }
                    ui.owner.status.clear();
                }
            }
        }

        // the open room: pump it, send this runner's sample while a route runs, and act on what the others sent
        let sample = match (playing.as_ref(), loading.is_none()) {
            (Some(g), true) => Some(
                world_sync::Sample::of(&g.p, g.depth, g.window_origin())
                    .with_run_state(g.report.is_some(), g.escape.is_some())
                    .with_held(g.selected_item())
                    .with_body(
                        crate::net::runner_figure::local_state(g).pose(),
                        g.p.vy(),
                        worn.get().slim,
                        crate::net::runner_figure::local_hide(g),
                    ),
            ),
            _ => None,
        };
        let room_events = match net.as_mut() {
            Some(n) => n.tick(sample),
            None => Vec::new(),
        };
        // the room's difficulty: the host sets it and sends it; a guest takes the host's and never sends its own
        if let Some(n) = net.as_mut() {
            if n.is_host() {
                n.set_room_difficulty(diff);
            } else {
                diff = world_sync::difficulty_from_name(n.room_difficulty());
            }
        }
        // the chosen skin goes to the room: at once on a setting change or a new room, and again when the website
        // accepts the upload (set_skin sends nothing when the published id stays the same)
        if let Some(n) = net.as_mut() {
            let hash = worn.get().hash;
            if hash != room_skin_seen || get_time() >= room_skin_check {
                room_skin_check = get_time() + crate::net::hunter_events::rules().skin_recheck.as_secs_f64();
                let uploaded = skins::uploaded_ids();
                if hash != room_skin_seen || uploaded.len() != room_uploaded_seen {
                    room_uploaded_seen = uploaded.len();
                    n.set_skin(&hash, &uploaded);
                    room_skin_seen = hash;
                }
            }
            // the arm style and OTHERS MAY COPY go with the id (Godot's skin_slim and skin_copy)
            n.set_skin_flags(world_sync::SkinFlags { slim: worn.get().slim, copy: settings.skin_copy });
        }
        // the SKIN tab says whether the worn skin is on its way to the other runners (shared, signing in, upload failing)
        if get_time() >= share_text_at {
            share_text_at = get_time() + 0.5;
            crate::ui::skin_tab::set_share_text(skins::share_state(&worn.get().hash, backend.signed_in(), net_cfg.offline, &skins::uploaded_ids()).text());
        }
        // the SKIN tab's COPY SKIN row lists the runners in the open room who allow it
        crate::ui::skin_tab::set_copy_peers(
            net.as_ref()
                .map(|n| {
                    n.copy_candidates()
                        .into_iter()
                        .map(|(name, hash, slim)| crate::ui::skin_tab::CopyPeer { name, hash, slim })
                        .collect()
                })
                .unwrap_or_default(),
        );
        // a guest's game runs no hunter simulation: the host's frames drive the hunters (game/hunting.rs)
        if let Some(g) = playing.as_mut() {
            g.remote_hunters = net.as_ref().is_some_and(|n| !n.is_host());
        }
        // a guest that joined a running route is told how many relays the host has sabotaged (the late joiner's snapshot)
        if loading.is_none() {
            if let (Some(n), Some(g)) = (net.as_mut().filter(|n| !n.is_host()), playing.as_mut()) {
                if let Some(s) = n.take_snapshot() {
                    g.apply_relay_snapshot(s.relays_sabotaged);
                }
                // the valves this guest turned are asks: the host decides each one and publishes the turn
                for ask in g.take_valve_requests() {
                    n.request_valve(&ask.valve_id, &ask.effect, &ask.gate, ask.valve_at);
                }
                for flow_id in g.take_burst_requests() {
                    n.request_burst(&flow_id);
                }
            }
        }
        // the host's game resolves what the guests reported (taser, escape, caught) and the valves they asked to turn;
        // the sabotaged relay count rides in the snapshot the next late joiner is sent
        if loading.is_none() {
            if let (Some(n), Some(g)) = (net.as_mut().filter(|n| n.is_host()), playing.as_mut()) {
                let remotes = same_level_remotes(&n.remotes_at(g.p.y(), g.depth, g.window_origin()), g.depth);
                for mut ev in n.take_guest_events() {
                    crate::net::wiring::place_guest_event(&mut ev, &remotes);
                    g.apply_guest_event(ev);
                }
                for req in n.take_valve_requests() {
                    if let Some((x, y, depth)) = n.guest_position(req.peer) {
                        // the guest's sample is an absolute cell; the host's valve props are in its window
                        let at = crate::game::Game::abs_to_window(g.window_origin(), (x, y));
                        if let Some(fields) = g.host_turn_valve(&req, at, depth) {
                            let rules = crate::net::hunter_events::rules();
                            g.hunter_out.emit(rules, g.difficulty, crate::net::valve_events::VALVE, fields, crate::shared::clock::Instant::now());
                        }
                    }
                }
                for req in n.take_burst_requests() {
                    if let Some((x, y, depth)) = n.guest_position(req.peer) {
                        g.host_burst_pipe(&req, (x, y), depth);
                    }
                }
                n.set_relays_sabotaged(g.relays_sabotaged_count());
            }
        }
        // the host's hunters and their events go to the guests (a guest never sends them)
        if let (Some(n), Some(g)) = (net.as_mut().filter(|n| n.is_host()), playing.as_ref()) {
            if loading.is_none() {
                let grab = g.escape.as_ref().map(|e| e.hunter);
                n.send_hunter_frame(g.hunter_frame(), g.difficulty, grab, crate::shared::clock::Instant::now());
            }
        }
        // the world events the host's game raised this frame go to the guests; a solo run only drops them
        if let Some(g) = playing.as_mut() {
            let out = g.hunter_out.take();
            if let Some(n) = net.as_mut().filter(|n| n.is_host()) {
                n.send_world_events(out);
            }
        }
        let mut room_closed: Option<String> = None;
        for event in room_events {
            match event {
                NetEvent::Start { seed, difficulty } => {
                    // the host started its route: enter the same seeded world (not while one is already running)
                    if playing.is_none() && loading.is_none() {
                        diff = world_sync::difficulty_from_name(&difficulty);
                        room = seed.clone();
                        loading = Some(Loading::for_room(diff, seed, Some(0)));
                    }
                }
                NetEvent::Seed(seed) => room = seed,
                NetEvent::Hunters(data) => {
                    // the host's hunters: a guest shows them, the host's frame is the one that counts
                    if let Some(g) = playing.as_mut() {
                        g.apply_hunter_frame(&data);
                    }
                }
                NetEvent::WorldEvent(event) => {
                    // a valve turn or a burst pipe changes the sewer water here too; anything else may be a notice
                    let piped = playing.as_mut().is_some_and(|g| g.apply_pipe_event(&event));
                    if !piped {
                        if let Some(text) = crate::net::hunter_events::notice(&event) {
                            show_room_notice(playing.as_mut(), &mut ui, &text);
                        }
                    }
                }
                NetEvent::Packs(ids) => {
                    // the room's mod packs: packs load at boot, so a pack this game lacks is named, not applied now
                    let missing: Vec<&str> = ids
                        .iter()
                        .filter(|id| !w.content.packs.iter().any(|p| p.id == **id))
                        .map(String::as_str)
                        .collect();
                    if !ids.is_empty() {
                        let text = if missing.is_empty() {
                            format!("ROOM MODS: {}", ids.join(", ").to_uppercase())
                        } else {
                            format!("ROOM MODS MISSING: {} // INSTALL THEN RESTART", missing.join(", ").to_uppercase())
                        };
                        show_room_notice(playing.as_mut(), &mut ui, &text);
                    }
                }
                NetEvent::GuestJoined(name) => show_room_notice(playing.as_mut(), &mut ui, &format!("{name} JOINED THE ROOM")),
                NetEvent::GuestLeft(name) => show_room_notice(playing.as_mut(), &mut ui, &format!("{name} LEFT THE ROOM")),
                NetEvent::HostLeft(reason) => room_closed = Some(reason),
                NetEvent::Problem(text) => show_room_notice(playing.as_mut(), &mut ui, &text),
                NetEvent::OwnerRefused(reason) => {
                    // the room refused this owner's command: the reason shows where the owner is looking
                    ui.owner.status = reason.clone();
                    owner_status_until = get_time() + OWNER_STATUS_SECONDS;
                    show_room_notice(playing.as_mut(), &mut ui, &reason);
                }
                // a server-side room's business: a player's game is never asked to run an owner command
                NetEvent::OwnerCommand { .. } => {}
                NetEvent::RouteCommand(RouteCommand::Restart { same_seed, seed, difficulty }) => {
                    // the owner's RESTART ROUTE (Godot carry_out_route_command "restart"): a running route ends for
                    // everyone (ROUTE RESTARTING), then a new run starts on the new seed that the guests enter
                    let current = playing.as_ref().map_or_else(|| room.clone(), |g| g.room_text.clone());
                    room = RouteCommand::restart_seed(same_seed, &seed, &current, &route::new_room_text());
                    // a guest's restart never changes the room's difficulty (only the host's does)
                    if let Some(d) = world_sync::commanded_difficulty(ui.room_mode == menus::RoomMode::Guest, difficulty) {
                        diff = d;
                    }
                    if let Some(n) = net.as_mut().filter(|n| n.is_host()) {
                        if playing.is_some() {
                            n.end_run("ROUTE RESTARTING");
                        }
                        n.start_run(&room, diff.name());
                    }
                    playing = None;
                    death = None;
                    pz = Pause::default();
                    map_open = false;
                    set_cursor_grab(false);
                    show_mouse(true);
                    grabbed = false;
                    loading = Some(Loading::for_room(diff, room.clone(), Some(0)));
                }
                NetEvent::RouteCommand(RouteCommand::End) => {
                    // the owner's END ROUTE (Godot "end"): only a running route ends, and everyone waits in the room's lobby
                    if playing.is_some() {
                        let reason = "PRIVATE ROOM // THE OWNER ENDED THE ROUTE";
                        if let Some(n) = net.as_mut().filter(|n| n.is_host()) {
                            n.end_run(reason);
                        }
                        playing = None;
                        crate::audio::world::reset(); // the box and hunter loops end with the run
                        death = None;
                        pz = Pause::default();
                        map_open = false;
                        set_cursor_grab(false);
                        show_mouse(true);
                        grabbed = false;
                        ui.go(Screen::Lobby);
                        flash(&mut ui, reason);
                    }
                }
                NetEvent::RouteEnded(reason) => {
                    // the host ended the route: this guest's run is over and it waits in the room's lobby
                    if playing.is_some() || loading.is_some() {
                        playing = None;
                        crate::audio::world::reset(); // the box and hunter loops end with the run
                        loading = None;
                        death = None;
                        pz = Pause::default();
                        map_open = false;
                        set_cursor_grab(false);
                        show_mouse(true);
                        grabbed = false;
                        ui.go(Screen::Lobby);
                        flash(&mut ui, &reason);
                    }
                }
            }
        }
        if let Some(reason) = room_closed {
            // the host closed the room: this guest goes back to the title with the reason
            net = None;
            playing = None;
            crate::audio::world::reset(); // the box and hunter loops end with the run
            loading = None;
            death = None;
            pz = Pause::default();
            map_open = false;
            set_cursor_grab(false);
            show_mouse(true);
            grabbed = false;
            ui.go(Screen::Title);
            ui.room_mode = menus::RoomMode::Solo;
            flash(&mut ui, &reason);
        }

        if let Some(l) = loading.as_mut() {
            // draw the screen first, build on the next frame, then let the bar reach 100%
            l.frames += 1;
            if l.frames == 3 && l.built.is_none() {
                let t0 = crate::shared::clock::Instant::now();
                let mut g = l.build(&w);
                g.fov_deg = settings.fov;
                crate::lr_info!(
                    "run start seed={} difficulty={} mode={} level={:?} built in {:.0} ms",
                    l.seed,
                    l.diff.name(),
                    if l.level.is_some() {
                        "infinite"
                    } else {
                        "finite"
                    },
                    l.level,
                    t0.elapsed().as_secs_f64() * 1000.0
                );
                l.built = Some(g);
            }
            let target = if l.built.is_some() { 1.0 } else { 0.35 };
            l.shown = (l.shown + dt * 2.6).min(target);
            let mode = if l.level.is_some() {
                "INFINITE"
            } else {
                "FINITE"
            };
            let detail = format!("DEFAULT // {mode} // {}", l.diff.name());
            menus::draw_loading(&w.assets, &l.room, &detail, l.shown);
            if l.shown >= 1.0 {
                playing = l.built.take();
                loading = None;
                death = None;
                pz = Pause::default();
                map_open = false;
                last_save = checkpoint::load();
            }
        } else if let Some(g) = playing.as_mut() {
            let in_report = g.report.is_some();
            let remotes = net.as_ref().map(|n| n.remotes_at(g.p.y(), g.depth, g.window_origin())).unwrap_or_default();
            let marks = crate::ui::markers::all_markers(g, &remotes);
            // the admin tools follow the server's admin flag for the signed-in account, never the local name
            admin.enabled = (backend.admin || (screenshot.is_some() && env("LR_SHOT_ADMIN").is_some())) && !stress_on;
            let admin_esc = if in_report || pz.open {
                false
            } else {
                admin.input(g)
            };
            if is_key_pressed(KeyCode::Escape) && !in_report && !admin_esc && !stress_on && !update_screen.is_shown() {
                if pz.open {
                    pause::escape(&mut pz);
                } else if map_open {
                    map_open = false;
                } else if g.shop_open() {
                    for h in g.hunters.iter_mut() {
                        if let crate::hunters::gimmicks::Gimmick::Zellia(z) = &mut h.gimmick {
                            z.shop_open = false;
                        }
                    }
                } else {
                    pz.show();
                }
            }
            if is_key_pressed(KeyCode::M) && !pz.open && !in_report && !stress_on {
                map_open = !map_open;
                if map_open {
                    map_view.recentre();
                }
            }
            if is_key_pressed(KeyCode::F1) {
                hud_hidden = !hud_hidden;
            }
            let mut want_grab = cursor_grab_wanted(
                pz.open,
                in_report,
                admin.is_open(),
                map_open,
                screenshot.is_none() && !stress_on && !update_screen.is_shown(),
            );
            // a long frame (a hitch, or the window was in the background) may have left the capture half-undone and
            // the cursor warp pending: settle again, re-issue the capture now and again on the next click
            if grabbed && want_grab && raw_dt > look::cfg().gap_relock_s {
                look_filter.relock(look::cfg());
                mouse_last = None;
                set_cursor_grab(true);
                show_mouse(false);
            }
            let click = is_mouse_button_pressed(MouseButton::Left) || is_mouse_button_pressed(MouseButton::Right);
            // the browser may have dropped the pointer lock (Escape, focus loss): then the game is not grabbed any more
            if web_pointer::lost_after_held(grabbed) {
                grabbed = false;
                mouse_last = None;
                // the browser already let go: tell macroquad too, or it keeps its grabbed flag, ignores the real cursor
                // position and the pause menu cannot be clicked
                set_cursor_grab(false);
                show_mouse(true);
                // the browser takes Escape to release the lock and never sends it to the game, so a lock lost during
                // play opens the pause menu instead (the player is not left stuck in the game)
                if !pz.open && !in_report && !update_screen.is_shown() {
                    pz.show();
                    // the pause menu is open now: this frame must not ask the browser for the lock again
                    want_grab = false;
                }
            } else if want_grab && grabbed && !web_pointer::locked() && web_pointer::may_request(false) {
                // the browser has not granted the lock yet, or refused it: ask again until it does
                set_cursor_grab(true);
            }
            let action = look::relock_action(want_grab != grabbed, want_grab && grabbed && click, cfg!(target_os = "macos"));
            // a browser grants the lock only on a user gesture, so on the web a request waits for a click (or a retry)
            if want_grab != grabbed && (!want_grab || web_pointer::may_request(click)) {
                set_cursor_grab(want_grab);
                show_mouse(!want_grab);
                grabbed = want_grab;
                if want_grab {
                    look_filter.relock(look::cfg());
                }
            } else if want_grab && click && web_pointer::may_request(true) {
                // Re-issue capture on a click. Native Windows backends can reject a grab requested during the scene
                // transition before the window is focused. A click never freezes the look (relock_action).
                set_cursor_grab(true);
                show_mouse(false);
            }
            if cfg!(target_arch = "wasm32") && !want_grab && web_pointer::locked() {
                // a lock the browser granted after the game stopped wanting it (a request still in flight when a menu
                // opened): release it, or the menu is stuck behind a locked, hidden cursor
                set_cursor_grab(false);
                show_mouse(true);
            }
            if action.clear_last {
                mouse_last = None;
            }
            // a stress run drives the run with its own input (app/stress.rs) and reads no keys or mouse
            let mut c = match stress_run.as_mut() {
                Some(st) => st.controls(dt, g),
                None => read_controls(
                    &mut mouse_last,
                    &mut look_filter,
                    grabbed,
                    settings.sensitivity,
                    g.shop_open(),
                    dt,
                ),
            };
            if pz.open || map_open || admin.is_open() || update_screen.is_shown() {
                c = Controls::default();
            }
            // the open map takes the mouse and the keys (pan, zoom, waypoints) while the run itself is still
            if map_open && !pz.open && !in_report && !admin.is_open() {
                map::full_map_input(g, &mut map_view);
            }
            g.fov_deg = settings.fov;
            let gdt = admin.game_dt(dt);
            if let Some(st) = stress_run.as_mut() {
                st.prepare(g, &w.content);
            }
            admin.before_update(g);
            if !pz.open {
                let _t = perf::section("update");
                g.map_open = map_open; // Void Yellow reads the full-screen map (main_3d.gd:444)
                g.update(gdt, &c, &w.content, &w.catalog, &w.assets, &w.sounds);
            }
            admin.after_update(g, &c, &w.content, gdt);
            // hunter dossier: entries unlock from real encounters, kept on the local account
            for id in g.take_sightings() {
                if profile::discover(&settings.username, &id, admin.debug_run()) {
                    g.announce("HUNTER DOSSIER ENTRY DISCOVERED");
                }
            }
            perf::counter("hunters", g.hunters.len() as f64);
            perf::counter("props", g.fac.props.len() as f64);
            perf::counter("world meshes", g.world.meshes.len() as f64);
            perf::counter("lamps", g.world.lamps.len() as f64);
            if g.report.is_some() && death.is_none() {
                let r = g.report.as_ref().unwrap();
                crate::lr_info!(
                    "run end: {} by {} at depth {} score {} relays {}",
                    r.cause,
                    r.hunter,
                    r.depth,
                    r.score,
                    r.relays
                );
            }
            // keep the last checkpoint: death clears it, RESPAWN AT CHECKPOINT reloads it
            save_poll += dt;
            if save_poll > 2.0 && g.report.is_none() {
                save_poll = 0.0;
                if let Some(cp) = checkpoint::load() {
                    last_save = Some(cp);
                }
            }
            let seed_text = g.room_text.clone();
            let mut leave = false;
            // a caught stress run ends here: the summary covers the run up to the catch
            if stress_on && g.report.is_some() {
                if let Some(st) = stress_run.as_mut() {
                    st.finish();
                }
                log::flush();
                std::process::exit(0);
            }
            if g.report.is_some() {
                if death.is_none() {
                    let r = g.report.as_ref().unwrap();
                    let category = w
                        .content
                        .hunters
                        .iter()
                        .find(|h| h.id == r.hunter)
                        .map(|h| h.category.clone())
                        .unwrap_or_default();
                    death = Some(
                        match report::pick_line(&r.hunter, r.cause, &seed_text, r.depth, &category)
                        {
                            Some((line, caption)) => Death::Moment(report::DeathMoment::new(
                                &r.hunter_name,
                                &line,
                                caption,
                            )),
                            None => Death::Card { focus: 0 },
                        },
                    );
                }
                if grabbed {
                    set_cursor_grab(false);
                    show_mouse(true);
                    grabbed = false;
                }
            }
            match death.as_mut() {
                Some(Death::Moment(m)) => {
                    render_play_with(g, &w.assets, &w.mat, &tg, &remotes);
                    m.update(dt);
                    m.draw(&w.assets);
                    if m.done {
                        death = Some(Death::Card { focus: 0 });
                    }
                }
                Some(Death::Card { focus }) => {
                    let can_respawn = last_save.is_some();
                    if let Some(card) = report::card_for(g, &seed_text, can_respawn) {
                        if is_key_pressed(KeyCode::Left)
                            || is_key_pressed(KeyCode::Right)
                            || is_key_pressed(KeyCode::Tab)
                        {
                            *focus = 1 - *focus;
                        }
                        let mut choice = report::draw_card(&w.assets, &card, *focus);
                        if is_key_pressed(KeyCode::Enter) {
                            choice = Some(*focus);
                        }
                        if is_key_pressed(KeyCode::Escape) {
                            choice = Some(1);
                        }
                        match choice {
                            Some(0) => {
                                loading = Some(match &last_save {
                                    Some(cp) => {
                                        checkpoint::save(cp);
                                        Loading::from_checkpoint(cp)
                                    }
                                    None => {
                                        Loading::new_room(g.difficulty, g.inf.as_ref().map(|_| 0))
                                    }
                                });
                                leave = true;
                            }
                            Some(_) => leave = true,
                            None => {}
                        }
                    }
                }
                None => {
                    {
                        let _t = perf::section("render 3d");
                        render_play_with(g, &w.assets, &w.mat, &tg, &remotes);
                    }
                    if !hud_hidden || g.escape.is_some() {
                        let _t = perf::section("hud");
                        hud::draw_hud_with_runners(g, &w.assets, fps, s, &marks);
                    }
                    if !hud_hidden {
                        hud::draw_crosshair(g, grabbed);
                    }
                    // Hold O: player list overlay (Godot player_list; Tab/T are labels until inventory/chat UI land).
                    if !stress_on && !pz.open && !in_report && is_key_down(KeyCode::O) {
                        let roster = net.as_ref().map(|n| n.roster()).unwrap_or_default();
                        let local_host = net.as_ref().map_or(true, |n| n.is_host());
                        pause::draw_hold_player_list(
                            &w.assets,
                            g,
                            &settings.username,
                            &roster,
                            local_host,
                        );
                    }
                    if map_open {
                        map::draw_full_map_with_runners(g, &w.assets, &map_view, &marks);
                    }
                    admin.draw(g, &w.assets, &w.content);
                    match admin.take_request() {
                        Some(crate::ui::admin::Request::RestartNewSeed) => {
                            loading = Some(Loading::new_room(
                                g.difficulty,
                                g.inf.as_ref().map(|_| 0),
                            ));
                            leave = true;
                        }
                        Some(crate::ui::admin::Request::RestartSameSeed) => {
                            loading = Some(Loading::new(
                                g.difficulty,
                                g.run_seed,
                                g.room_text.clone(),
                                g.inf.as_ref().map(|_| 0),
                            ));
                            leave = true;
                        }
                        Some(crate::ui::admin::Request::EndRoute) => leave = true,
                        None => {}
                    }
                    if pz.open {
                        let name = settings.username.clone();
                        let roster = net.as_ref().map(|n| n.roster()).unwrap_or_default();
                        let local_host = net.as_ref().map_or(true, |n| n.is_host());
                        match pause::draw_pause(
                            &w.assets,
                            &mut pz,
                            &mut ui,
                            &mut settings,
                            &w.content,
                            g,
                            &name,
                            &roster,
                            local_host,
                        ) {
                            pause::Out::Abandon => {
                                // leaving the route saves it first (Godot `_leave_test`: autosave("quit")); a guest never saves
                                if local_host {
                                    g.autosave("quit");
                                }
                                leave = true;
                            }
                            pause::Out::Owner(action) => {
                                submit_owner(&mut owner_cmd, &mut ui, net.is_some(), action, &mut owner_status_until, get_time());
                            }
                            _ => {}
                        }
                    }
                }
            }
            if leave {
                playing = None;
                crate::audio::world::reset(); // the box and hunter loops end with the run
                death = None;
                save = checkpoint::load();
                ui.go(Screen::Title);
                pz = Pause::default();
                map_open = false;
                set_cursor_grab(false);
                show_mouse(true);
                grabbed = false;
            }
        } else {
            let username = settings.username.clone();
            // the lobby's live capacity and guest names (a guest's session has neither)
            let live = net.as_ref().and_then(|n| n.player_count());
            let guests = net.as_ref().map(|n| n.guest_names()).unwrap_or_default();
            // back on the title the room is closed (the host's quit tells the guests)
            if ui.screen == Screen::Title {
                if let Some(n) = net.take() {
                    n.leave();
                }
                owner_cmd.cancel();
                ui.room_mode = menus::RoomMode::Solo;
            }
            let action = match ui.screen {
                Screen::Title => menus::draw_title(&w.assets, &mut ui, &username, true),
                Screen::Create => menus::draw_create(&w.assets, &mut ui, &username),
                Screen::Join => menus::draw_join(&w.assets, &mut ui, &username),
                Screen::Quit => menus::draw_quit(&w.assets, &mut ui, &username),
                Screen::Account => {
                    let before = settings.username.clone();
                    let a = menus::draw_account(
                        &w.assets,
                        &mut ui,
                        &mut settings.username,
                        save.as_ref(),
                    );
                    if settings.username != before {
                        settings.save();
                    }
                    a
                }
                Screen::Settings => {
                    menus::draw_title(&w.assets, &mut ui, &username, false);
                    if settings_panel::draw_settings(
                        &w.assets,
                        &mut ui,
                        &mut settings,
                        &w.content,
                        false,
                    ) {
                        ui.go(Screen::Title);
                        ui.focus = menus::title_focus(menus::TitleEntry::Settings, &username);
                    }
                    Action::None
                }
                Screen::Admin => {
                    menus::draw_title(&w.assets, &mut ui, &username, false);
                    if crate::ui::admin::draw_hub(
                        &w.assets,
                        hub.get_or_insert_with(|| crate::ui::admin::HubPanel::new(net_cfg.clone())),
                        &settings.username,
                    ) {
                        hub = None;
                        Action::Go(Screen::Title)
                    } else {
                        Action::None
                    }
                }
                Screen::Lobby => menus::draw_lobby(
                    &w.assets,
                    &mut ui,
                    &mut diff,
                    route::room_text_code(&room),
                    &room,
                    &username,
                    save.as_ref(),
                    live,
                    &guests,
                    true,
                ),
                Screen::Dossier => {
                    menus::draw_lobby(
                        &w.assets,
                        &mut ui,
                        &mut diff,
                        route::room_text_code(&room),
                        &room,
                        &username,
                        save.as_ref(),
                        live,
                        &guests,
                        false,
                    );
                    if menus::draw_dossier(&w.assets, &mut ui, &w.content, &username) {
                        ui.go(Screen::Lobby);
                        ui.focus = menus::lobby_focus_for(ui.room_mode, ui.owner.owner, menus::LobbyButton::CharacterFiles);
                    }
                    Action::None
                }
            };
            // the boot card covers the title until it has faded: its buttons take no clicks
            let action = if boot.blocks_input() || update_screen.is_shown() { Action::None } else { action };
            match action {
                Action::Go(next) => {
                    // cancelling the JOIN dialog drops its request
                    if next == Screen::Title && ui.screen == Screen::Join && matches!(net_pending, Some((NetIntent::Join, _))) {
                        net_pending = None;
                    }
                    // back from the account screen the lobby keeps its room (the room is closed only on the title)
                    if next == Screen::Lobby && ui.screen != Screen::Dossier && ui.screen != Screen::Account {
                        room = route::new_room_text();
                        ui.room_mode = menus::RoomMode::Solo;
                    }
                    if next == Screen::Account {
                        save = checkpoint::load();
                    }
                    let from = ui.screen;
                    if next == Screen::Account {
                        ui.account_back = from;
                    }
                    ui.go(next);
                    if next == Screen::Title {
                        // focus goes back to the button that opened the dialog
                        let who = settings.username.as_str();
                        ui.focus = title_return_focus(from, who);
                    }
                }
                // CREATE ROOM's route is DEFAULT // INFINITE, starting on the top level
                Action::StartRun => {
                    // a guest enters the host's route when the host starts it; a guest's own room text is not a route
                    if ui.room_mode == menus::RoomMode::Guest {
                        flash(&mut ui, "WAITING FOR THE HOST TO START THE ROUTE");
                    } else {
                        loading = Some(Loading::for_room(diff, room.clone(), Some(0)));
                        // a hosted room's guests enter the same route
                        if let Some(n) = net.as_mut() {
                            n.start_run(&room, diff.name());
                        }
                    }
                }
                Action::Reroll => {
                    // a hosted or joined room keeps its seed: the code is in it
                    if ui.room_mode == menus::RoomMode::Solo {
                        room = route::new_room_text();
                    }
                }
                Action::CreateOnline => {
                    // a room belongs to an account: signed out is told to sign in, and nothing is asked of the server
                    if net.is_none() && net_pending.is_none() && hosted_create.is_none() && hosted_join.is_none() {
                        let s = &hosted::rules().strings;
                        if net_cfg.offline {
                            ui.create_note = s.offline.clone();
                        } else if !backend.signed_in() {
                            ui.create_note = s.sign_in_required.clone();
                            flash(&mut ui, &s.sign_in_required);
                        } else {
                            ui.create_note = s.creating.clone();
                            let (cfg, token, d) = (net_cfg.clone(), backend.account.session_token.clone(), diff);
                            hosted_create = Some(Pending::spawn(move || hosted::create(&cfg, &token, d)));
                        }
                    }
                }
                Action::Owner(action) => {
                    submit_owner(&mut owner_cmd, &mut ui, net.is_some(), action, &mut owner_status_until, get_time());
                }
                Action::PublicRoute => {
                    if net.is_none() && net_pending.is_none() {
                        let messages = crate::content::public_roster::PublicRoster::load().messages;
                        if net_cfg.offline {
                            flash(&mut ui, &messages.unavailable);
                        } else {
                            flash(&mut ui, &messages.joining);
                            let name = world_sync::clean_name(&settings.username);
                            let skin = worn.get().hash;
                            net_pending = Some((NetIntent::Public, world_sync::join_public_room(net_cfg.clone(), name, skin)));
                        }
                    }
                }
                Action::JoinCode(code) => {
                    if net.is_none() && net_pending.is_none() {
                        ui.join_status = format!("JOINING ROOM {code}...");
                        let name = world_sync::clean_name(&settings.username);
                        let skin = worn.get().hash;
                        // an owner's miss reserves the typed code and hosts it (world_sync::join_room); everyone else joins as a guest
                        let owner = backend.signed_in() && crate::ui::admin::is_admin_owner(&backend.account.username);
                        net_pending = Some((NetIntent::Join, world_sync::join_room(net_cfg.clone(), code, name, skin, backend.account.session_token.clone(), owner)));
                    }
                }
                Action::ContinueRun => {
                    if let Some(c) = &save {
                        settings.save();
                        loading = Some(Loading::from_checkpoint(c));
                    }
                }
                Action::ClearSave => {
                    checkpoint::clear();
                    save = None;
                }
                Action::SignIn { name, password } => backend.sign_in(&name, &password),
                Action::CreateAccount { name, password } => backend.create_account(&name, &password, ""),
                Action::SignOut => {
                    backend.sign_out();
                    settings.username.clear();
                    settings.save();
                }
                Action::Quit => {
                    // the browser build has no desktop to quit to: its title never offers QUIT (menus::QUIT_OFFERED)
                    if menus::QUIT_OFFERED {
                        settings.save();
                        break;
                    }
                }
                Action::None => {}
            }
        }

        // the web build's UPDATING screen goes over everything drawn so far (the menus, the run, the pause menu); native builds never see an update
        let update_ready = web_update::update_available() || (screenshot.is_some() && env("LR_SHOT_UPDATE").is_some());
        update_screen.frame(&w.assets, raw_dt, update_ready, loading.is_none() && playing.as_ref().is_some_and(|g| g.report.is_none()));
        if let Some(path) = &screenshot {
            // LR_SHOT_RUN=1 starts a run (flashlight on) to look at a floor or the run's overlays
            if frame == 20 && playing.is_none() && env("LR_SHOT_RUN").is_some() {
                let shot_seed: u32 = env("LR_SHOT_SEED").and_then(|s| s.parse().ok()).unwrap_or(20261101);
                let mut g = if env("LR_SHOT_FINITE").is_some() {
                    Game::new(diff, 20261101, &w.content, &w.catalog, &w.assets)
                } else {
                    Game::new_infinite(
                        diff,
                        shot_seed,
                        &route::room_text(shot_seed),
                        env("LR_SHOT_LEVEL").and_then(|l| l.parse().ok()).unwrap_or(0),
                        &w.content,
                        &w.catalog,
                        &w.assets,
                    )
                };
                // LR_SHOT_FLASH=0 leaves the flashlight off (the dark view), for a before/after pair at the same spot
                g.p.flashlight_on = env("LR_SHOT_FLASH").as_deref() != Some("0");
                g.p.flashlight_on = true;
                    shot_at_stairs(&mut g, &shape);
                }
                if let Ok(turn) = std::env::var("LR_SHOT_YAW") {
                    g.p.yaw = turn.parse().unwrap_or(0.0);
                }
                if let Some(pg) = env("LR_SHOT_PAUSE") {
                    pz.show();
                    match pg.as_str() {
                        "statistics" => pz.tab = 1,
                        "players" => pz.tab = 2,
                        "settings" => pz.sub = Sub::Settings,
                        "skin" => {
                            pz.sub = Sub::Skin;
                            ui.settings_tab = 4;
                        }
                        "dossier" => pz.sub = Sub::Dossier,
                        "confirm" => pz.sub = Sub::Confirm,
                        _ => {}
                    }
                }
                map_open = env("LR_SHOT_MAP").is_some();
                playing = Some(g);
            }
            // LR_SHOT_ADMIN=n opens the admin menu on tab n (LR_SHOT_ADMIN_POPUP=key opens a dropdown)
            if frame == 30 && playing.is_some() {
                if let Some(t) = env("LR_SHOT_ADMIN").and_then(|t| t.parse::<usize>().ok()) {
                    admin.enabled = true;
                    admin.show_tab(t);
                }
            }
            if frame == 60 {
                if let (Some(kind), Some(g)) = (env("LR_SHOT_DEATH"), playing.as_mut()) {
                    let h = &g.hunters[0];
                    g.report = Some(crate::game::Report {
                        hunter: h.def.id.clone(),
                        hunter_name: h.def.name.clone(),
                        cause: "caught",
                        depth: g.depth,
                        score: g.score,
                        relays: g.relays_collected,
                        coins: g.coins,
                        seed: g.run_seed,
                        difficulty: g.difficulty,
                    });
                    if kind == "card" {
                        death = Some(Death::Card { focus: 0 });
                    }
                }
            }
            if frame >= 120 {
                // LR_PERF_MODE=2 puts the performance panel (and its render counters) in the capture
                perf::draw_overlay(w.assets.font.as_ref(), &overlay_rows());
                save_screen(path);
                return;
            }
        }
        // the boot card goes over the title once the first title frame is drawn, then fades out
        if frame == 1 {
            boot.ready();
        }
        if !stress_on {
            boot.draw_frame();
        }
        perf::draw_overlay(w.assets.font.as_ref(), &overlay_rows());
        if shot_key {
            shots.capture();
        }
        shots.draw(&w.assets, dt);
        // the FPS LIMIT row would cap the numbers a stress run measures, so a stress run is paced only by
        // --stress-limit (none by default)
        pacer.wait(if stress_on { stress_cap } else { settings.frame_cap_fps() });
        next_frame().await;
    }
    // quitting the game closes an open room, so the guests get HOST_LEFT now and not after the directory's TTL
    if let Some(n) = net.take() {
        n.leave();
    }
}



/// Whether the cursor is captured for look this frame. Only the run itself captures it: the pause menu, a report, the admin
/// menu, the fullscreen map (M, which pans and places waypoints with the pointer) and a screenshot all leave it free.
fn cursor_grab_wanted(paused: bool, in_report: bool, admin_open: bool, map_open: bool, live: bool) -> bool {
    !paused && !in_report && !admin_open && !map_open && live
}

/// The focus index of the title button that opened the dialog `from`: cancelling the dialog hands
/// focus back to that button (CREATE ROOM's cancel lands on CREATE ROOM).
fn title_return_focus(from: Screen, who: &str) -> usize {
    match from {
        Screen::Create => menus::title_focus(menus::TitleEntry::Create, who),
        Screen::Join => menus::title_focus(menus::TitleEntry::Join, who),
        Screen::Settings => menus::title_focus(menus::TitleEntry::Settings, who),
        Screen::Account => menus::title_focus(menus::TitleEntry::Account, who),
        Screen::Quit => menus::title_focus(menus::TitleEntry::Quit, who),
        _ => 0,
    }
}

/// The real resume path: `Loading::from_checkpoint` then the placement `Loading::build` does.
#[cfg(test)]
mod resume_tests {
    use super::*;
    use crate::game::level_stack::floor_y_of;

    fn save(depth: i32, y: Option<f32>, infinite: bool) -> checkpoint::Checkpoint {
        checkpoint::Checkpoint {
            run_seed: 7,
            room_text: "ROOM-RESUME".into(),
            depth,
            score: 3,
            coins: 4,
            difficulty: Difficulty::Standard,
            infinite,
            y,
        }
    }

    fn resumed_y(cp: &checkpoint::Checkpoint) -> (Option<i32>, f32) {
        let l = Loading::from_checkpoint(cp);
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(cp.difficulty, cp.run_seed, &cp.room_text, l.level.unwrap_or(0), &content, &catalog);
        l.place_resumed(&mut g);
        (l.level, g.p.y())
    }

    #[test]
    fn a_standing_save_resumes_at_its_saved_height_and_a_mid_flight_save_on_the_floor() {
        let standing = floor_y_of(-1) + 0.25;
        let (level, y) = resumed_y(&save(-1, Some(standing), true));
        assert_eq!(level, Some(-1));
        assert!((y - standing).abs() < 1e-4, "standing save resumed at {y}");
        let (level, y) = resumed_y(&save(-1, Some(floor_y_of(-1) + 2.6), true));
        assert_eq!(level, Some(-1));
        assert!((y - floor_y_of(-1)).abs() < 1e-4, "a mid-flight save resumed in the air at {y}");
        let (level, y) = resumed_y(&save(-2, None, true));
        assert_eq!((level, y), (Some(-2), floor_y_of(-2)), "a legacy save stands on its floor");
    }

    #[test]
    fn a_finite_route_resume_carries_no_height() {
        let l = Loading::from_checkpoint(&save(3, Some(4.0), false));
        assert_eq!((l.level, l.y), (None, None));
    }
}

#[cfg(test)]
mod title_focus_tests {
    use super::*;

    #[test]
    fn cancelling_create_room_returns_focus_to_create_room() {
        for who in ["ALICE", "KYTTU"] {
            let i = title_return_focus(Screen::Create, who);
            assert_eq!(menus::title_entries(who)[i], menus::TitleEntry::Create, "{who}");
        }
    }

    #[test]
    fn other_dialogs_still_return_to_their_own_button() {
        assert_eq!(
            menus::title_entries("ALICE")[title_return_focus(Screen::Join, "ALICE")],
            menus::TitleEntry::Join
        );
        assert_eq!(
            menus::title_entries("ALICE")[title_return_focus(Screen::Quit, "ALICE")],
            menus::TitleEntry::Quit
        );
    }
}

#[cfg(test)]
mod grab_tests {
    use super::*;

    #[test]
    fn the_open_map_leaves_the_mouse_free() {
        // the run with nothing on top of it captures the mouse (look)...
        assert!(cursor_grab_wanted(false, false, false, false, true));
        // ...and the fullscreen map does not, so the pointer moves and drags the map
        assert!(!cursor_grab_wanted(false, false, false, true, true), "the map frees the mouse");
        assert!(!cursor_grab_wanted(true, false, false, true, true));
        assert!(!cursor_grab_wanted(false, true, false, true, true));
        assert!(!cursor_grab_wanted(false, false, true, true, true));
        assert!(!cursor_grab_wanted(false, false, false, false, false), "a screenshot never captures");
    }
}

#[cfg(test)]
mod look_tests {
    use super::*;

    const SENS: f32 = crate::config::MOUSE_SENSITIVITY;

    #[test]
    fn keyboard_diagonal_is_not_faster_and_scales_with_sensitivity() {
        let straight = keyboard_look(vec2(1.0, 0.0), 1.0, MOUSE_SENSITIVITY).length();
        let diagonal = keyboard_look(vec2(1.0, 1.0), 1.0, MOUSE_SENSITIVITY).length();
        assert!((straight - diagonal).abs() < 1e-5, "{straight} vs {diagonal}");
        assert!((straight - KEY_LOOK_RADIANS_PER_SECOND).abs() < 1e-5);
        let max = keyboard_look(vec2(1.0, 0.0), 1.0, 0.012).length();
        assert!((max / straight - 0.012 / MOUSE_SENSITIVITY).abs() < 1e-4);
    }

    #[test]
    fn platform_multiplier_turns_a_physical_pixel_like_godot() {
        // macOS: points, Godot k = 3 -> sens / 3 per point. Windows: 175 % display, logical px, sens * 1.75 / 3.
        use crate::config::look;
        use crate::config::{MOUSE_LOOK_DISPLAY_SCALE, MOUSE_LOOK_GODOT_WINDOW_SCALE};
        let k = MOUSE_LOOK_GODOT_WINDOW_SCALE;
        let c = look::cfg();
        let mac = look::px_to_radians(vec2(100.0, 0.0), SENS, c.mouse_multiplier_macos);
        assert!((mac.x - 100.0 * SENS / k).abs() < 1e-4, "mac {}", mac.x);
        let screen = vec2(2256.0 / MOUSE_LOOK_DISPLAY_SCALE, 1504.0 / MOUSE_LOOK_DISPLAY_SCALE);
        let logical = 100.0 / MOUSE_LOOK_DISPLAY_SCALE;
        let px = look::delta_px(vec2(-logical * 2.0 / screen.x, 0.0), screen, 1.0, None);
        let win = look::px_to_radians(px, SENS, c.mouse_multiplier_windows);
        assert!((win.x - 100.0 * SENS / k).abs() < 1e-4, "win {}", win.x);
        assert!(c.multiplier() > 0.0);
    }
}

#[cfg(test)]
mod remote_level_tests {
    use super::*;

    fn view(peer: u32, h: f32, level: i32) -> RemoteView {
        RemoteView { peer, name: String::new(), x: 1.0, y: 2.0, yaw: 0.0, h, level, skin: String::new(), moving: false, sprinting: false, crouching: false, slim: false, speed: 0.0, vy: 0.0, clip: None, held: None, hide: None, hide_since: None }
    }

    #[test]
    fn a_remote_one_floor_away_is_not_on_the_map_list() {
        let near = [view(1, -0.5, 0), view(2, -4.2, -1)];
        let map = same_level_remotes(&near, 0);
        assert_eq!(map.iter().map(|r| r.peer).collect::<Vec<_>>(), vec![1], "the 3D list keeps both, the map list one");
        assert_eq!(near.len(), 2);
    }
}

#[cfg(test)]
mod owner_command_tests {
    use super::*;

    fn owner_ui(owner: bool) -> Ui {
        let mut ui = Ui::default();
        ui.owner.owner = owner;
        ui
    }

    #[test]
    fn a_player_who_does_not_own_the_room_cannot_send_owner_commands() {
        // nobody but the owner has the controls on screen, and the app refuses a command that arrives anyway
        let mut cmd = hosted::OwnerCommander::default();
        let mut ui = owner_ui(false);
        let mut until = 0.0;
        assert!(!submit_owner(&mut cmd, &mut ui, true, OwnerAction::End, &mut until, 10.0));
        assert!(!cmd.busy() && ui.owner.status.is_empty() && until == 0.0);
    }

    #[test]
    fn the_owner_needs_a_room_to_send_to() {
        let mut cmd = hosted::OwnerCommander::default();
        let mut ui = owner_ui(true);
        let mut until = 0.0;
        assert!(!submit_owner(&mut cmd, &mut ui, false, OwnerAction::Start, &mut until, 10.0), "still waiting for the room to open");
        assert!(!cmd.busy());
    }

    #[test]
    fn the_owner_takes_one_command_at_a_time_and_sees_the_confirming_line() {
        let mut cmd = hosted::OwnerCommander::default();
        let mut ui = owner_ui(true);
        let mut until = 0.0;
        assert!(submit_owner(&mut cmd, &mut ui, true, OwnerAction::Restart, &mut until, 10.0));
        assert_eq!(ui.owner.status, hosted::rules().strings.owner_confirming);
        assert_eq!(until, 10.0 + OWNER_STATUS_SECONDS);
        // the second is refused with the busy line, and the first is still the one waiting
        assert!(!submit_owner(&mut cmd, &mut ui, true, OwnerAction::End, &mut until, 11.0));
        assert_eq!(ui.owner.status, hosted::rules().strings.owner_busy);
        assert!(cmd.busy());
    }

}

/// `LR_SHOT_STAIRS=shape`: put the player on the landing row of the first flight down of that shape (spiral, switchback or
/// straight) in the streamed window, a metre back from the landing, looking down at the well's middle.
fn shot_at_stairs(g: &mut Game, shape: &str) {
    use crate::world::flight::FlightShape;
    let want = match shape {
        "spiral" => FlightShape::Spiral,
        "straight" => FlightShape::Straight,
        _ => FlightShape::Switchback,
    };
    let Some(s) = g.fac.stairs.iter().copied().find(|s| s.dir < 0 && !s.is_ladder() && s.shape == want) else {
        eprintln!("LR_SHOT_STAIRS: no {shape} flight down in the window ({} stairs)", g.fac.stairs.len());
        return;
    };
    let cells = s.well_cells();
    let n = cells.len().max(1) as f32;
    let mid = (
        cells.iter().map(|c| c.x as f32 + 0.5).sum::<f32>() / n,
        cells.iter().map(|c| c.y as f32 + 0.5).sum::<f32>() / n,
    );
    let (lx, lz) = s.landing().center();
    g.p.pos = (lx - s.run.0 as f32, lz - s.run.1 as f32);
    g.p.yaw = (-(mid.0 - g.p.pos.0)).atan2(-(mid.1 - g.p.pos.1));
    g.p.pitch = -0.5;
    eprintln!("LR_SHOT_STAIRS: {shape} flight at {:?}, player at {:?}", s.cell, g.p.pos);
}
