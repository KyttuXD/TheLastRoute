//! The application: loading, the main loop (menus -> run -> death moment -> run card), input,
//! the half-resolution pixel render targets, and the `--screenshot` / `--self-test` modes.
//!
//! Screenshot hooks (with `--screenshot out.png`, captured at frame 120):
//!   LR_SHOT_SIZE=WxH        window size (e.g. 1920x1080, 844x390)
//!   LR_SCHEME=city          the City colour scheme (palette.gd SCHEMES)
//!   LR_SHOT_SCREEN=...      title | create | join | quit | account | settings | lobby | dossier | admin
//!   LR_SHOT_TAB=n           the settings tab (0 AUDIO .. 5 MODS)
//!   LR_SHOT_ACCOUNT=1       the account panel (same as LR_SHOT_SCREEN=account)
//!   LR_SHOT_RUN=1           start a run (LR_SHOT_INF=1: the Infinite route)
//!   LR_SHOT_PAUSE=...       with a run: pause | statistics | players | settings | dossier | confirm
//!   LR_SHOT_MAP=1           with a run: the full map
//!   LR_SHOT_DEATH=...       with a run: moment | card (a capture by the floor's hunter)

pub mod assets;
pub mod checkpoint;
pub mod log;
pub mod paths;
pub mod perf;
pub mod platform;
pub mod profile;
pub mod selftest;
pub mod settings;

use crate::audio::{self, Sounds};
use crate::config::*;
use crate::content::Content;
use crate::game::{Controls, Game};
use crate::render::{scene, shader, viewmodel};
use crate::ui::menus::{self, Action, Screen, Ui};
use crate::ui::pause::{self, Pause, Sub};
use crate::ui::{hud, map, report, settings_panel, style};
use crate::world::catalog::Catalog;
use assets::Assets;
use macroquad::prelude::*;
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
    scene::draw_scene(g, a, mat, &t.scene);
    viewmodel::draw_viewmodel(g, a, mat, &t.view);
    set_default_camera();
    clear_background(BLACK);
    let p = DrawTextureParams {
        dest_size: Some(vec2(screen_width(), screen_height())),
        flip_y: true,
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

fn read_controls(mouse_last: &mut Option<Vec2>, grabbed: bool, sens: f32, shop: bool) -> Controls {
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
        let delta = mouse_delta_position();
        let delta = if delta.length_squared() > 0.0 {
            delta
        } else {
            mouse_last.map(|last| pos - last).unwrap_or(Vec2::ZERO)
        };
        // Screen-space mouse X is opposite to the game's signed turn direction.
        // Negate only the horizontal axis so left/right movement is intuitive while
        // preserving the existing vertical look behavior.
        c.look = vec2(-delta.x, delta.y) * sens;
        *mouse_last = Some(pos);
    } else {
        *mouse_last = None;
    }
    let k = KEY_LOOK_RADIANS_PER_SECOND * get_frame_time();
    if is_key_down(KeyCode::J) || is_key_down(KeyCode::Left) {
        c.look.x -= k;
    }
    if is_key_down(KeyCode::L) || is_key_down(KeyCode::Right) {
        c.look.x += k;
    }
    if is_key_down(KeyCode::I) || is_key_down(KeyCode::Up) {
        c.look.y -= k;
    }
    if is_key_down(KeyCode::K) || is_key_down(KeyCode::Down) {
        c.look.y += k;
    }
    c
}

fn new_seed() -> u32 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(7);
    ((t as u64 ^ (t >> 32) as u64) as u32).max(1)
}

fn room_code(seed: u32) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut v = seed;
    (0..5)
        .map(|_| {
            let c = ALPHA[(v % ALPHA.len() as u32) as usize] as char;
            v = v / ALPHA.len() as u32 ^ 0x9e37;
            c
        })
        .collect()
}

/// Saves the window's current picture (the framebuffer reads bottom-up).
pub fn save_screen(path: &str) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut img = get_screen_data();
    for px in img.bytes.chunks_mut(4) {
        px[3] = 255;
    }
    img.export_png(path);
}

#[cfg(windows)]
fn utc_offset_minutes() -> i64 {
    #[repr(C)]
    struct SystemTime([u16; 8]);
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(tz: *mut TimeZoneInformation) -> u32;
    }
    let mut tz: TimeZoneInformation = unsafe { std::mem::zeroed() };
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = match r {
        1 => tz.bias + tz.standard_bias,
        2 => tz.bias + tz.daylight_bias,
        _ => tz.bias,
    };
    -(bias as i64)
}

#[cfg(not(windows))]
fn utc_offset_minutes() -> i64 {
    0
}

/// The local hour and minute the solo checkpoint was last written (RunSaves.status_line).
pub fn checkpoint_clock() -> Option<(u32, u32)> {
    let p = paths::user_file("checkpoint.json")?;
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    let secs =
        t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64 + utc_offset_minutes() * 60;
    let day = secs.rem_euclid(86_400);
    Some(((day / 3600) as u32, ((day / 60) % 60) as u32))
}

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

/// A run waiting behind the BUILDING ROUTE screen (route_loading.gd).
struct Loading {
    diff: Difficulty,
    seed: u32,
    /// Infinite level to enter (0 = top), or None for a finite route at `depth`.
    level: Option<i32>,
    depth: i32,
    score: i32,
    coins: i32,
    frames: u32,
    built: Option<Game>,
    shown: f32,
}

impl Loading {
    fn new(diff: Difficulty, seed: u32, level: Option<i32>) -> Loading {
        Loading {
            diff,
            seed,
            level,
            depth: 1,
            score: 0,
            coins: 0,
            frames: 0,
            built: None,
            shown: 0.0,
        }
    }

    fn from_checkpoint(c: &checkpoint::Checkpoint) -> Loading {
        let mut l = Loading::new(
            c.difficulty,
            c.run_seed,
            if c.infinite { Some(c.depth) } else { None },
        );
        l.depth = c.depth.max(1);
        l.score = c.score;
        l.coins = c.coins;
        l
    }

    fn build(&self, w: &World) -> Game {
        let mut g = match self.level {
            Some(level) => Game::new_infinite(
                self.diff, self.seed, level, &w.content, &w.catalog, &w.assets,
            ),
            None => {
                let mut g = Game::new(self.diff, self.seed, &w.content, &w.catalog, &w.assets);
                if self.depth > 1 {
                    g.depth = self.depth;
                    g.start_floor(&w.content, &w.catalog, &w.assets);
                }
                g
            }
        };
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

pub async fn run(screenshot: Option<String>) {
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
    let w = load_world().await;
    let mut settings = Settings::load();
    style::set_scheme(&env("LR_SCHEME").unwrap_or_else(|| settings.scheme.clone()));
    if settings.fullscreen && screenshot.is_none() {
        set_fullscreen(true);
    }
    let mut tg = targets(None, &settings.quality);
    let mut ui = Ui::default();
    let mut playing: Option<Game> = None;
    let mut diff = Difficulty::Standard;
    let mut seed = new_seed();
    let mut pz = Pause::default();
    let mut map_open = false;
    let mut hud_hidden = false;
    let mut death: Option<Death> = None;
    let mut last_save: Option<checkpoint::Checkpoint> = None;
    let mut save_poll = 0.0f32;
    let mut mouse_last: Option<Vec2> = None;
    let mut grabbed = false;
    let mut save = checkpoint::load();
    let mut loading: Option<Loading> = None;
    let mut frame = 0u32;
    let mut audio_volumes = (f32::NAN, f32::NAN, f32::NAN);
    // the admin tools: the signed-in admin's account (KYTTU) only, like Build.tools_enabled
    let mut admin = crate::ui::admin::Admin::new();
    let mut hub: Option<crate::ui::admin::HubPanel> = None;
    let (mut fps_t, mut fps_n, mut fps) = (0.0f32, 0, 60);
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
    }

    loop {
        frame += 1;
        perf::frame(get_frame_time());
        perf::handle_keys();
        let dt = get_frame_time().min(0.05);
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
        let s = style::ui_scale();

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
            menus::draw_loading(
                &w.assets,
                &format!("ROOM-{}-{:08X}", room_code(l.seed), l.seed),
                &detail,
                l.shown,
            );
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
            admin.enabled = settings.username == "KYTTU";
            let admin_esc = if in_report || pz.open {
                false
            } else {
                admin.input(g)
            };
            if is_key_pressed(KeyCode::Escape) && !in_report && !admin_esc {
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
            if is_key_pressed(KeyCode::M) && !pz.open && !in_report {
                map_open = !map_open;
            }
            if is_key_pressed(KeyCode::F1) {
                hud_hidden = !hud_hidden;
            }
            let want_grab = !pz.open && !in_report && !admin.is_open() && screenshot.is_none();
            if want_grab != grabbed {
                set_cursor_grab(want_grab);
                show_mouse(!want_grab);
                grabbed = want_grab;
                mouse_last = None;
            } else if want_grab
                && (is_mouse_button_pressed(MouseButton::Left)
                    || is_mouse_button_pressed(MouseButton::Right))
            {
                // Re-issue capture on the first click. Native Windows backends can reject a
                // grab requested during the scene transition before the window is focused.
                set_cursor_grab(true);
                show_mouse(false);
                mouse_last = None;
            }
            let mut c = read_controls(
                &mut mouse_last,
                grabbed,
                settings.sensitivity,
                g.shop_open(),
            );
            if pz.open || map_open || admin.is_open() {
                c = Controls::default();
            }
            g.fov_deg = settings.fov;
            let gdt = admin.game_dt(dt);
            admin.before_update(g);
            if !pz.open {
                let _t = perf::section("update");
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
            let code = room_code(g.run_seed);
            let seed_text = format!("ROOM-{code}-{:08X}", g.run_seed);
            let mut leave = false;
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
                    render_play(g, &w.assets, &w.mat, &tg);
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
                                    None => Loading::new(
                                        g.difficulty,
                                        new_seed(),
                                        g.inf.as_ref().map(|_| 0),
                                    ),
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
                        render_play(g, &w.assets, &w.mat, &tg);
                    }
                    if !hud_hidden || g.escape.is_some() {
                        let _t = perf::section("hud");
                        hud::draw_hud(g, &w.assets, fps, s);
                    }
                    if map_open {
                        map::draw_full_map(g, &w.assets);
                    }
                    admin.draw(g, &w.assets, &w.content);
                    match admin.take_request() {
                        Some(crate::ui::admin::Request::RestartNewSeed) => {
                            loading = Some(Loading::new(
                                g.difficulty,
                                new_seed(),
                                g.inf.as_ref().map(|_| 0),
                            ));
                            leave = true;
                        }
                        Some(crate::ui::admin::Request::RestartSameSeed) => {
                            loading = Some(Loading::new(
                                g.difficulty,
                                g.run_seed,
                                g.inf.as_ref().map(|_| 0),
                            ));
                            leave = true;
                        }
                        Some(crate::ui::admin::Request::EndRoute) => leave = true,
                        None => {}
                    }
                    if pz.open {
                        let name = settings.username.clone();
                        if let pause::Out::Abandon = pause::draw_pause(
                            &w.assets,
                            &mut pz,
                            &mut ui,
                            &mut settings,
                            &w.content,
                            g,
                            &name,
                            &code,
                        ) {
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
                        ui.focus = 2;
                    }
                    Action::None
                }
                Screen::Admin => {
                    menus::draw_title(&w.assets, &mut ui, &username, false);
                    if crate::ui::admin::draw_hub(
                        &w.assets,
                        hub.get_or_insert_with(crate::ui::admin::HubPanel::new),
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
                    &room_code(seed),
                    seed,
                    &username,
                    save.as_ref(),
                    true,
                ),
                Screen::Dossier => {
                    menus::draw_lobby(
                        &w.assets,
                        &mut ui,
                        &mut diff,
                        &room_code(seed),
                        seed,
                        &username,
                        save.as_ref(),
                        false,
                    );
                    if menus::draw_dossier(&w.assets, &mut ui, &w.content, &username) {
                        ui.go(Screen::Lobby);
                        ui.focus = 2;
                    }
                    Action::None
                }
            };
            match action {
                Action::Go(next) => {
                    if next == Screen::Lobby && ui.screen != Screen::Dossier {
                        seed = new_seed();
                    }
                    if next == Screen::Account {
                        save = checkpoint::load();
                    }
                    let from = ui.screen;
                    ui.go(next);
                    if next == Screen::Title {
                        // focus goes back to the button that opened the dialog
                        ui.focus = match from {
                            Screen::Join => 1,
                            Screen::Settings => 2,
                            Screen::Account => 3,
                            Screen::Quit => 5,
                            _ => 0,
                        };
                    }
                }
                // CREATE ROOM's route is DEFAULT // INFINITE, starting on the top level
                Action::StartRun => loading = Some(Loading::new(diff, seed, Some(0))),
                Action::Reroll => seed = new_seed(),
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
                Action::Quit => break,
                Action::None => {}
            }
        }

        if let Some(path) = &screenshot {
            // LR_SHOT_RUN=1 starts a run (flashlight on) to look at a floor or the run's overlays
            if frame == 20 && playing.is_none() && env("LR_SHOT_RUN").is_some() {
                let mut g = if env("LR_SHOT_FINITE").is_some() {
                    Game::new(diff, 20261101, &w.content, &w.catalog, &w.assets)
                } else {
                    Game::new_infinite(diff, 20261101, 0, &w.content, &w.catalog, &w.assets)
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
        perf::draw_overlay(w.assets.font.as_ref(), &[]);
        next_frame().await;
    }
}
