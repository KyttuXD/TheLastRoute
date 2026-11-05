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
//!   LR_SHOT_PAUSE=...       with a run: pause | statistics | players | settings | dossier | confirm
//!   LR_SHOT_ADMIN=n         with a run: the admin menu on tab n (0 STATUS .. 9 FIELDS)
//!   LR_SHOT_MAP=1           with a run: the full map
//!   LR_SHOT_DEATH=...       with a run: moment | card (a capture by the floor's hunter)

pub use crate::shared::{assets, checkpoint, paths, skins};
pub mod gfx;
pub mod log;
pub mod pacing;
pub mod perf;
pub mod platform;
pub mod profile;
pub mod public_route;
pub mod screenshot;
pub mod selftest;
pub mod settings;
pub mod stress;
pub mod updater;

use crate::audio::{self, Sounds};
use crate::config::look::{self, LookFilter};
use crate::config::*;
use crate::content::Content;
use crate::game::{Controls, Game};
use crate::render::{scene, shader, viewmodel};
use crate::ui::menus::{self, Action, Screen, Ui};
use crate::ui::pause::{self, Pause, Sub};
use crate::ui::{hud, map, report, settings_panel, style};
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
    let scale = match quality {
        "LOW" => 0.32,
        "MEDIUM" => 0.40,
        _ => 0.50, // AUTO and HIGH preserve the reference presentation
    };
    let w = (screen_width() * scale).max(64.0) as u32;
    let h = (screen_height() * scale).max(36.0) as u32;
    if let Some(t) = old {
        if t.size == (w, h) {
            return t;
        }
    }
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
    {
        let _t = perf::section("viewmodel");
        viewmodel::draw_viewmodel(g, a, mat, &t.view);
        crate::render::fp_hands::draw_ladder_hands(g, a, mat, &t.view);
    }
    set_default_camera();
    clear_background(BLACK);
    let p = DrawTextureParams {
        dest_size: Some(vec2(screen_width(), screen_height())),
        // OpenGL render targets are bottom-up; Metal's are top-down
        flip_y: !platform::is_metal(),
        ..Default::default()
    };
    draw_texture_ex(&t.scene.texture, 0.0, 0.0, WHITE, p.clone());
    draw_texture_ex(&t.view.texture, 0.0, 0.0, WHITE, p);
}

pub struct World {
    pub content: Content,
    pub catalog: Catalog,
    pub assets: Assets,
    pub sounds: Sounds,
    pub mat: Material,
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
    let mat = shader::load_world_material();
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
    let t = std::time::Instant::now();
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
    let mat = shader::load_world_material();
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
        if let (Some(y), true) = (self.y, g.inf.is_some()) {
            // a saved run resumes at its absolute height (a legacy save at the floor of its level)
            g.p.set_abs_y(y);
            // a window with a noisy floor (the Inferno) puts the body on the real surface, not on the flat floor line
            if g.fac.terrain().is_some() {
                g.p.stand_on_ground(&g.fac);
            }
        }
        g.score = self.score;
        g.coins = self.coins;
        g
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

/// A room request that is in flight: hosting a room, or joining one by its code.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NetIntent {
    Host,
    Join,
    /// PUBLIC ROUTE: the session turns out hosted or joined once the directory has answered.
    Public,
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
    let mut tg = targets(None, &settings.quality);
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
    // when the worn skin is next offered to the website (a new hash at once, a refused one again after a delay)
    let mut upload_plan = skins::UploadPlan::default();
    let upload_retry = skins::config().map_or(60.0, |c| c.upload_retry_secs);
    // the skin last offered to the open room, the uploaded count it was judged against, and when to look again
    let mut room_skin_seen = String::new();
    // whether this runner's chosen skin and ARMS setting give slim arms, sent in its sample so peers draw the same arms
    let mut room_uploaded_seen = usize::MAX;
    let mut room_skin_check = 0.0f64;
    let mut net: Option<NetSession> = None;
    let mut net_pending: Option<(NetIntent, Pending<NetSession>)> = None;
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
        let arms = skins::ArmsSetting::parse(&settings.arms).unwrap_or(skins::ArmsSetting::Auto);
        if worn.update(&settings.skin, settings.skin_slim, &settings.arms, skins::generation(), || {
            skins::worn_skin(&settings.skin, settings.skin_slim, arms)
        }) {
            let w = worn.get();
            crate::render::fp_hands::set_look(&w.hash, w.slim);
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
        tg = targets(Some(tg), &settings.quality);
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
        if !net_cfg.offline && upload_plan.due(get_time(), &worn.get().hash, backend.signed_in(), skins::uploaded_ids, upload_retry) {
            skins::upload_in_background(net_cfg.clone(), backend.account.session_token.clone(), worn.get().hash);
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
                    // PUBLIC ROUTE is a host when nobody held the room, else a guest of the runner who does
                    let public = intent == NetIntent::Public;
                    let intent = match intent {
                        NetIntent::Public if session.is_host() => NetIntent::Host,
                        NetIntent::Public => NetIntent::Join,
                        other => other,
                    };
                    match intent {
                        NetIntent::Host => {
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
                        NetIntent::Join => {
                            room = format!("ROOM-{code}-");
                            ui.room_mode = menus::RoomMode::Guest;
                            ui.go(Screen::Lobby);
                            flash(&mut ui, &format!("JOINED ROOM {code} // WAITING FOR THE ROUTE"));
                        }
                        NetIntent::Public => {}
                    }
                    // a new room is offered the chosen skin on the next frame
                    room_skin_seen.clear();
                    room_uploaded_seen = usize::MAX;
                    net = Some(session);
                }
                Err(e) => {
                    let text = e.message_or("ROOM REQUEST FAILED");
                    match intent {
                        NetIntent::Host | NetIntent::Public => flash(&mut ui, &text),
                        NetIntent::Join if ui.screen == Screen::Join => ui.join_status = text,
                        NetIntent::Join => flash(&mut ui, &text),
                    }
                }
            }
        }

        // the open room: pump it, send this runner's sample while a route runs, and act on what the others sent
        let sample = match (playing.as_ref(), loading.is_none()) {
            (Some(g), true) => Some(
                world_sync::Sample::of(&g.p, g.depth, g.window_origin())
                    .with_run_state(g.report.is_some(), g.escape.is_some())
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
                        if let Some(fields) = g.host_turn_valve(&req, (x, y), depth) {
                            let rules = crate::net::hunter_events::rules();
                            g.hunter_out.emit(rules, g.difficulty, crate::net::valve_events::VALVE, fields, std::time::Instant::now());
                        }
                    }
                }
                n.set_relays_sabotaged(g.relays_sabotaged_count());
            }
        }
        // the host's hunters and their events go to the guests (a guest never sends them)
        if let (Some(n), Some(g)) = (net.as_mut().filter(|n| n.is_host()), playing.as_ref()) {
            if loading.is_none() {
                let grab = g.escape.as_ref().map(|e| e.hunter);
                n.send_hunter_frame(g.hunter_frame(), g.difficulty, grab, std::time::Instant::now());
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
                let t0 = std::time::Instant::now();
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
            // the admin tools follow the server's admin flag for the signed-in account, never the local name
            admin.enabled = (backend.admin || (screenshot.is_some() && env("LR_SHOT_ADMIN").is_some())) && !stress_on;
            let admin_esc = if in_report || pz.open {
                false
            } else {
                admin.input(g)
            };
            if is_key_pressed(KeyCode::Escape) && !in_report && !admin_esc && !stress_on {
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
            let want_grab = cursor_grab_wanted(
                pz.open,
                in_report,
                admin.is_open(),
                map_open,
                screenshot.is_none() && !stress_on,
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
            let action = look::relock_action(want_grab != grabbed, want_grab && grabbed && click, cfg!(target_os = "macos"));
            if want_grab != grabbed {
                set_cursor_grab(want_grab);
                show_mouse(!want_grab);
                grabbed = want_grab;
                if want_grab {
                    look_filter.relock(look::cfg());
                }
            } else if want_grab && click {
                // Re-issue capture on a click. Native Windows backends can reject a grab requested during the scene
                // transition before the window is focused. A click never freezes the look (relock_action).
                set_cursor_grab(true);
                show_mouse(false);
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
            if pz.open || map_open || admin.is_open() {
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
                        hud::draw_hud(g, &w.assets, fps, s);
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
                        map::draw_full_map(g, &w.assets, &map_view);
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
                        if let pause::Out::Abandon = pause::draw_pause(
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
                            // leaving the route saves it first (Godot `_leave_test`: autosave("quit")); a guest never saves
                            if local_host {
                                g.autosave("quit");
                            }
                            leave = true;
                        }
                    }
                }
            }
            if leave {
                playing = None;
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
                        ui.focus = menus::lobby_focus(ui.room_mode, menus::LobbyButton::CharacterFiles);
                    }
                    Action::None
                }
            };
            // the boot card covers the title until it has faded: its buttons take no clicks
            let action = if boot.blocks_input() { Action::None } else { action };
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
                Action::HostRoom => {
                    if net.is_none() && net_pending.is_none() {
                        flash(&mut ui, "OPENING A ROOM ON THE SERVER...");
                        let name = world_sync::clean_name(&settings.username);
                        let skin = worn.get().hash;
                        net_pending = Some((NetIntent::Host, world_sync::host_room(net_cfg.clone(), route::new_room_text(), name, skin)));
                    }
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
                    settings.save();
                    break;
                }
                Action::None => {}
            }
        }

        if let Some(path) = &screenshot {
            // LR_SHOT_RUN=1 starts a run (flashlight on) to look at a floor or the run's overlays
            if frame == 20 && playing.is_none() && env("LR_SHOT_RUN").is_some() {
                let mut g = if env("LR_SHOT_FINITE").is_some() {
                    Game::new(diff, 20261101, &w.content, &w.catalog, &w.assets)
                } else {
                    Game::new_infinite(
                        diff,
                        20261101,
                        &route::room_text(20261101),
                        0,
                        &w.content,
                        &w.catalog,
                        &w.assets,
                    )
                };
                g.p.flashlight_on = true;
                if let Ok(turn) = std::env::var("LR_SHOT_YAW") {
                    g.p.yaw = turn.parse().unwrap_or(0.0);
                }
                if let Some(pg) = env("LR_SHOT_PAUSE") {
                    pz.show();
                    match pg.as_str() {
                        "statistics" => pz.tab = 1,
                        "players" => pz.tab = 2,
                        "settings" => pz.sub = Sub::Settings,
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
        perf::draw_overlay(w.assets.font.as_ref(), &[]);
        if shot_key {
            shots.capture();
        }
        shots.draw(&w.assets, dt);
        // the FPS LIMIT would cap the numbers a stress run measures, so it does not pace one
        if !stress_on {
            pacer.wait(settings.frame_cap_fps());
        }
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
        RemoteView { peer, x: 1.0, y: 2.0, yaw: 0.0, h, level, skin: String::new(), moving: false, sprinting: false, crouching: false, slim: false, speed: 0.0, vy: 0.0, clip: None, hide: None, hide_since: None }
    }

    #[test]
    fn a_remote_one_floor_away_is_not_on_the_map_list() {
        let near = [view(1, -0.5, 0), view(2, -4.2, -1)];
        let map = same_level_remotes(&near, 0);
        assert_eq!(map.iter().map(|r| r.peer).collect::<Vec<_>>(), vec![1], "the 3D list keeps both, the map list one");
        assert_eq!(near.len(), 2);
    }
}
