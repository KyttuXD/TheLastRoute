//! LR Story: a data-driven story game in first person. The content is a pack folder (pack.json, maps/, scenes/,
//! items/); the engine reads it and plays it. A map is a PNG picture (the floor) plus a mask PNG whose colours say what
//! each tile is (walls are boxes, doors, NPCs, items, triggers, exits). The body and its movement come from the Last
//! Route runner (see body.rs). See docs/LR_STORY.md for the format.
//!
//! `cargo run --bin lr_story -- [--pack DIR] [--mod DIR]... [--check] [--shot PATH [--scene ID]]`
//!   --pack DIR    the pack folder (default: the LR_STORY_PACK env var, else data/lr_story in this crate)
//!   --mod DIR     an extra mod pack folder (repeatable)
//!   --check       load and validate the pack, print a summary and exit (status 1 when it has problems)
//!   --shot PATH   render a few frames, save the screen to PATH and exit (for checking the view); --scene starts a scene
//!
//! Subcommands (they take the same `--pack` and `--mod` flags):
//!   check                              the same as --check
//!   init <id> [parent]                 writes a starter mod pack to <parent or ./mods>/<id>, then checks it loads
//!   pack <dir>                         prints the pack's id, version, file count, total bytes and a sha256 fingerprint
//!   import-lore <files...> --id <id> [--into DIR]
//!                                      imports KyttusLab-LORE files as scenes (default DIR: ./mods/<id>), then checks

mod anim;
mod body;
mod load;
mod lore;
mod model;
mod play;
mod props;
mod save;
mod textures;
mod yaml;

#[cfg(test)]
mod guard;

use body::{Input, FOV_DEG, MOUSE_SENSITIVITY, TILE_M};
use macroquad::camera::Camera;
use macroquad::prelude::{
    clear_background, draw_cube, draw_plane, draw_rectangle, draw_rectangle_lines, draw_sphere, draw_text,
    get_frame_time, get_screen_data, get_time, is_key_down, is_key_pressed, measure_text, mouse_delta_position,
    next_frame, screen_height, screen_width, set_camera, set_cursor_grab, set_default_camera, show_mouse, vec2,
    vec3, Camera3D, Color, Conf, FilterMode, Image, ImageFormat, KeyCode, Texture2D, Vec2, Vec3, WHITE,
};
use model::{Kind, Pack, Rgb};
use play::Game;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const BOX_H: f32 = 180.0;
const TEXT_SIZE: f32 = 22.0;
const LINE_H: f32 = 28.0;
/// Frames rendered before a `--shot` is saved (so the first frame's textures and camera are settled).
const SHOT_FRAMES: u32 = 4;

fn arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

/// Every value given with a repeated flag, in order (`--mod A --mod B`).
fn arg_values(flag: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).filter(|w| w[0] == flag).map(|w| w[1].clone()).collect()
}

/// Where mod packs are read from: `mods/` beside the executable, `mods/` in the working folder, and `mods/` under the
/// per-user folder (the same three roots last_route uses).
fn default_mod_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(exe) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("mods"))) {
        roots.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("mods"));
    }
    if let Some(user) = last_route_core::shared::paths::user_dir() {
        roots.push(user.join("mods"));
    }
    roots
}

fn pack_dir() -> PathBuf {
    if let Some(p) = arg_value("--pack") {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("LR_STORY_PACK").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    // The sample pack lives under last_route's data folder (moved there 2026-11-05).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("bin")
        .join("last_route")
        .join("data")
        .join("lr_story")
}

fn window_conf() -> Conf {
    Conf {
        window_title: "LR Story".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

/// Flags that take a value. The subcommands skip their values when they look for positional words.
const VALUE_FLAGS: &[&str] = &["--pack", "--mod", "--id", "--into"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("init") => cmd_init(&args[1..]),
        Some("pack") => cmd_pack(&args[1..]),
        Some("import-lore") => cmd_import_lore(&args[1..]),
        Some("check") => check_with(None),
        _ if args.iter().any(|a| a == "--check") => check_with(None),
        Some(other) if !other.starts_with("--") => {
            usage(&format!("unknown command '{other}' (commands: check, init, pack, import-lore)"))
        }
        _ => {
            play();
            return;
        }
    };
    std::process::exit(code);
}

/// The pack set the flags name: the core pack, the `--mod` folders and the mod roots.
fn load_config() -> load::Config {
    load::Config {
        core: pack_dir(),
        mod_dirs: arg_values("--mod").into_iter().map(PathBuf::from).collect(),
        mod_roots: default_mod_roots(),
    }
}

/// Loads the pack set, printing every problem. None when the set has any.
fn load_or_report(cfg: &load::Config) -> Option<Pack> {
    match load::load(cfg) {
        Ok(pack) => Some(pack),
        Err(errs) => {
            eprintln!("lr_story: the pack at {} has {} problem(s):", cfg.core.display(), errs.len());
            for e in &errs {
                eprintln!("  - {e}");
            }
            None
        }
    }
}

/// Loads the pack set the flags name, with `extra_mod` added, and prints the OK line. Status 1 when it has problems.
fn check_with(extra_mod: Option<&Path>) -> i32 {
    let mut cfg = load_config();
    cfg.mod_dirs.extend(extra_mod.map(Path::to_path_buf));
    match load_or_report(&cfg) {
        Some(pack) => {
            println!(
                "OK: '{}' - {} map(s), {} scene(s), {} item(s)",
                pack.title,
                pack.maps.len(),
                pack.scenes.len(),
                pack.items.len()
            );
            0
        }
        None => 1,
    }
}

/// Opens the game window on the pack the flags name. Exits with status 1 when the pack has problems.
fn play() {
    let Some(pack) = load_or_report(&load_config()) else {
        std::process::exit(1);
    };
    let shot = arg_value("--shot").map(PathBuf::from);
    let scene = arg_value("--scene");
    macroquad::Window::from_config(window_conf(), run(pack, shot, scene));
}

/// Prints a usage problem and returns status 2.
fn usage(problem: &str) -> i32 {
    eprintln!("lr_story: {problem}");
    2
}

/// The value after `flag` in `args`, if the flag is there with a value.
fn value_of(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

/// The words in `args` that are neither flags nor the values of `value_flags`. An unknown `--` flag is an error.
fn positionals(args: &[String], value_flags: &[&str]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if value_flags.contains(&a) {
            if i + 1 >= args.len() {
                return Err(format!("{a} needs a value"));
            }
            i += 2;
        } else if a.starts_with("--") {
            return Err(format!("unknown option '{a}'"));
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    Ok(out)
}

/// `init <id> [parent]`: writes a starter mod pack to `<parent or ./mods>/<id>`, then checks that the pack loads.
fn cmd_init(args: &[String]) -> i32 {
    let words = match positionals(args, VALUE_FLAGS) {
        Ok(w) => w,
        Err(e) => return usage(&e),
    };
    let (id, rest) = match words.as_slice() {
        [id, rest @ ..] => (id, rest),
        [] => return usage("init needs a pack id: init <id> [parent]"),
    };
    if rest.len() > 1 {
        return usage("init takes an id and an optional parent folder: init <id> [parent]");
    }
    let parent = PathBuf::from(rest.first().map_or("mods", String::as_str));
    match write_starter_pack(&parent, id) {
        Ok(dir) => {
            println!("created {}", dir.display());
            check_with(Some(dir.as_path()))
        }
        Err(e) => {
            eprintln!("lr_story: {e}");
            1
        }
    }
}

/// `pack <dir>`: prints the fingerprint of a pack folder.
fn cmd_pack(args: &[String]) -> i32 {
    let words = match positionals(args, VALUE_FLAGS) {
        Ok(w) => w,
        Err(e) => return usage(&e),
    };
    let [dir] = words.as_slice() else {
        return usage("pack takes one folder: pack <dir>");
    };
    match pack_summary(Path::new(dir)) {
        Ok(s) => {
            println!("id: {}", s.id);
            println!("version: {}", s.version);
            println!("files: {}", s.files);
            println!("bytes: {}", s.bytes);
            println!("sha256: {}", s.sha256);
            0
        }
        Err(e) => {
            eprintln!("lr_story: {e}");
            1
        }
    }
}

/// `import-lore <files...> --id <id> [--into DIR]`: imports the LORE files as scenes, then checks the pack.
fn cmd_import_lore(args: &[String]) -> i32 {
    let words = match positionals(args, VALUE_FLAGS) {
        Ok(w) => w,
        Err(e) => return usage(&e),
    };
    if words.is_empty() {
        return usage("import-lore needs at least one file: import-lore <files...> --id <id> [--into DIR]");
    }
    let Some(id) = value_of(args, "--id") else {
        return usage("import-lore needs --id <id>");
    };
    if !lore::is_pack_id(&id) {
        return usage(&format!("'{id}' is not a valid pack id; try '{}'", lore::slug(&id)));
    }
    let into = value_of(args, "--into").map_or_else(|| PathBuf::from("mods").join(&id), PathBuf::from);
    let files: Vec<PathBuf> = words.iter().map(PathBuf::from).collect();
    match lore::import(&files, &id, &into) {
        Ok(ids) => {
            println!("imported {} scene(s) into {}", ids.len(), into.display());
            check_with(Some(into.as_path()))
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("lr_story: {e}");
            }
            1
        }
    }
}

/// Writes the starter files of mod pack `id` into `parent/<id>`: pack.json, one scene with one narration line, and an
/// empty item list. Refuses an id that is not valid and a folder that already exists. Returns the new folder.
fn write_starter_pack(parent: &Path, id: &str) -> Result<PathBuf, String> {
    if !lore::is_pack_id(id) {
        return Err(format!(
            "'{id}' is not a valid pack id (lower-case letters, digits and _, starting with a letter, 3 to 32 characters); try '{}'",
            lore::slug(id)
        ));
    }
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let dir = parent.join(id);
    fs::create_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{} already exists", dir.display()),
        _ => format!("{}: {e}", dir.display()),
    })?;

    let pack_doc = json!({ "schema": load::SCHEMA_PACK, "id": id, "name": id, "version": "0.1.0" });
    let mut scene_doc = json!({ "schema": load::SCHEMA_SCENES, "scenes": {} });
    scene_doc["scenes"][format!("{id}_intro").as_str()] = json!({ "lines": [{ "text": "Your story begins here." }] });
    let items_doc = json!({ "schema": load::SCHEMA_ITEMS, "items": {} });

    for (rel, doc) in [("pack.json", pack_doc), ("scenes/main.json", scene_doc), ("items/items.json", items_doc)] {
        let path = dir.join(rel);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(dir)
}

/// What `pack` prints about a pack folder.
#[derive(Debug, PartialEq)]
struct PackSummary {
    id: String,
    version: String,
    files: usize,
    bytes: u64,
    sha256: String,
}

/// The fingerprint of a pack folder. Files are sorted by their path (with `/` separators). The hash reads each file's
/// path, a NUL, the file's bytes and a NUL, so renaming or editing any file changes it.
fn pack_summary(dir: &Path) -> Result<PackSummary, String> {
    let manifest_path = dir.join("pack.json");
    let text = fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let id = manifest["id"]
        .as_str()
        .ok_or_else(|| format!("{}: no \"id\"", manifest_path.display()))?
        .to_string();
    let version = manifest["version"].as_str().unwrap_or("0.0.0").to_string();

    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for (rel, path) in &files {
        let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        bytes += data.len() as u64;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update(&data);
        hasher.update([0u8]);
    }
    let sha256: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(PackSummary { id, version, files: files.len(), bytes, sha256 })
}

/// Every file under `dir`, as (its path relative to `root` with `/` separators, its path).
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let read = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in read {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push((parts.join("/"), path));
        }
    }
    Ok(())
}

async fn run(pack: Pack, shot: Option<PathBuf>, scene: Option<String>) {
    let mut textures: HashMap<String, Texture2D> = HashMap::new();
    for (id, map) in &pack.maps {
        let loaded = std::fs::read(&map.art)
            .ok()
            .and_then(|bytes| Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).ok());
        match loaded {
            Some(img) => {
                let tex = Texture2D::from_image(&img);
                tex.set_filter(FilterMode::Nearest);
                textures.insert(id.clone(), tex);
            }
            None => eprintln!("lr_story: could not load the picture for map '{id}'"),
        }
    }
    // The flags the player set last time, read before the start scene runs so it sees them.
    let flags_file = flags_file(&pack);
    let saved = match &flags_file {
        Some((path, id)) => save::load_flags(path, id).unwrap_or_else(|e| {
            eprintln!("lr_story: could not read the saved flags, starting without them: {e}");
            BTreeSet::new()
        }),
        None => BTreeSet::new(),
    };
    let mut game = match Game::with_flags(pack, saved) {
        Ok(game) => game,
        Err(e) => {
            eprintln!("lr_story: {e}");
            return;
        }
    };
    if let Some(id) = scene {
        game.start_scene(&id);
    }
    if shot.is_none() {
        set_cursor_grab(true);
        show_mouse(false);
    }

    let mut frame = 0u32;
    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
        let dt = get_frame_time().min(0.1);
        let input = read_input(&game);
        handle_actions(&mut game, flags_file.as_ref());
        game.update_body(dt, &input);
        game.tick(dt);
        draw(&game, &textures);
        frame += 1;
        if let Some(path) = &shot {
            if frame >= SHOT_FRAMES {
                get_screen_data().export_png(&path.to_string_lossy());
                break;
            }
        }
        next_frame().await;
    }
}

/// The body's input for this frame: keys for movement, the mouse for look. Mouse look is in radians.
fn read_input(g: &Game) -> Input {
    let held = |keys: &[KeyCode]| keys.iter().any(|k| is_key_down(*k));
    let axis = |plus: bool, minus: bool| (plus as i32 - minus as i32) as f32;
    // macroquad's delta is last minus current, the opposite of how the mouse moved, so it is negated: moving down or
    // right gives a positive step (last_route config/look.rs delta_px does the same). Scaled to pixels, then radians.
    let px = -mouse_delta_position() * vec2(screen_width(), screen_height());
    let look = vec2(px.x, px.y) * MOUSE_SENSITIVITY;
    Input {
        fwd: axis(held(&[KeyCode::W, KeyCode::Up]), held(&[KeyCode::S, KeyCode::Down])),
        strafe: axis(held(&[KeyCode::D, KeyCode::Right]), held(&[KeyCode::A, KeyCode::Left])),
        sprint: held(&[KeyCode::LeftShift, KeyCode::RightShift]),
        crouch: held(&[KeyCode::LeftControl, KeyCode::RightControl, KeyCode::C]),
        jump: g.talk.is_none() && held(&[KeyCode::Space]),
        look,
    }
}

/// The flags file and the core pack id the flags are saved under. None when there is no user folder or pack id.
fn flags_file(pack: &Pack) -> Option<(PathBuf, String)> {
    Some((save::default_path()?, save::core_pack_id(pack)?))
}

/// Picks choice `n` and saves the flags, so a restart keeps what the choice set. A failed save is shown as a message.
fn pick(g: &mut Game, n: usize, flags_file: Option<&(PathBuf, String)>) {
    g.choose(n);
    if let Some((path, id)) = flags_file {
        if let Err(e) = save::save_flags(path, id, &g.flags) {
            eprintln!("lr_story: could not save the flags: {e}");
            g.toast("Progress not saved");
        }
    }
}

/// The presses that are not movement: E / Enter talk or go on, Space goes on while a scene plays, digits and
/// arrows pick a choice.
fn handle_actions(g: &mut Game, flags_file: Option<&(PathBuf, String)>) {
    let pressed = |keys: &[KeyCode]| keys.iter().any(|k| is_key_pressed(*k));
    let confirm = [KeyCode::Space, KeyCode::Enter, KeyCode::E];

    if let Some((choosing, sel)) = g.talk.as_ref().map(|t| (t.choosing, t.sel)) {
        if choosing {
            if pressed(&[KeyCode::Up, KeyCode::W]) {
                g.move_sel(-1);
            }
            if pressed(&[KeyCode::Down, KeyCode::S]) {
                g.move_sel(1);
            }
            const DIGITS: [KeyCode; 9] = [
                KeyCode::Key1,
                KeyCode::Key2,
                KeyCode::Key3,
                KeyCode::Key4,
                KeyCode::Key5,
                KeyCode::Key6,
                KeyCode::Key7,
                KeyCode::Key8,
                KeyCode::Key9,
            ];
            if let Some(n) = DIGITS.iter().position(|k| is_key_pressed(*k)) {
                pick(g, n, flags_file);
            } else if pressed(&confirm) {
                let now = g.talk.as_ref().map_or(sel, |t| t.sel);
                pick(g, now, flags_file);
            }
        } else if pressed(&confirm) {
            g.advance();
        }
        return;
    }
    if pressed(&[KeyCode::E, KeyCode::Enter]) {
        g.interact();
    }
}

/// The first-person camera at the body's eye, looking along its view direction.
fn camera(g: &Game) -> Camera3D {
    let eye = g.body.eye();
    Camera3D {
        position: eye,
        target: eye + g.body.view_dir(),
        up: vec3(0.0, 1.0, 0.0),
        fovy: FOV_DEG.to_radians(),
        z_near: 0.05,
        z_far: 300.0,
        ..Default::default()
    }
}

fn col(c: Rgb) -> Color {
    Color::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0)
}

/// Where a world point lands on screen, or None when it is behind the camera or off the screen.
fn project(cam: &Camera3D, p: Vec3, sw: f32, sh: f32) -> Option<Vec2> {
    let clip = cam.matrix() * p.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let n = clip.truncate() / clip.w;
    if n.x.abs() > 1.0 || n.y.abs() > 1.0 {
        return None;
    }
    Some(vec2((n.x + 1.0) * 0.5 * sw, (1.0 - n.y) * 0.5 * sh))
}

fn draw(g: &Game, textures: &HashMap<String, Texture2D>) {
    clear_background(Color::new(0.03, 0.03, 0.05, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let map = g.current_map();
    let (w, h) = (map.grid.w, map.grid.h);
    let cam = camera(g);

    set_camera(&cam);
    let half = vec3(w as f32 * TILE_M / 2.0, 0.0, h as f32 * TILE_M / 2.0);
    draw_plane(half, vec2(half.x, half.z), textures.get(&g.map), WHITE);

    let t = get_time() as f32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let cx = (x as f32 + 0.5) * TILE_M;
            let cz = (y as f32 + 0.5) * TILE_M;
            match map.grid.at(x, y) {
                Some(Kind::Wall { height, color }) => {
                    draw_cube(vec3(cx, height / 2.0, cz), vec3(TILE_M, *height, TILE_M), None, col(*color));
                }
                Some(kind @ Kind::Door { height, color, .. }) if kind.blocks(&g.flags, &g.inv) => {
                    draw_cube(vec3(cx, height / 2.0, cz), vec3(TILE_M, *height, TILE_M), None, col(*color));
                }
                Some(Kind::Item { color, .. }) if !g.taken.contains(&format!("{}@{},{}", g.map, x, y)) => {
                    let bob = 0.6 + 0.08 * (t * 2.5 + x as f32).sin();
                    draw_sphere(vec3(cx, bob, cz), 0.22, None, col(*color));
                }
                Some(Kind::Npc { color, .. }) => {
                    draw_cube(vec3(cx, 0.5, cz), vec3(0.6, 1.0, 0.6), None, col(*color));
                    draw_sphere(vec3(cx, 1.25, cz), 0.25, None, col(*color));
                }
                Some(Kind::Exit { .. }) => {
                    draw_plane(
                        vec3(cx, 0.02, cz),
                        vec2(TILE_M * 0.4, TILE_M * 0.4),
                        None,
                        Color::new(0.6, 0.4, 0.9, 1.0),
                    );
                }
                _ => {}
            }
        }
    }
    set_default_camera();

    // Names over the NPCs, placed where the figures are on screen.
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if let Some(Kind::Npc { name, .. }) = map.grid.at(x, y) {
                let world = vec3((x as f32 + 0.5) * TILE_M, 1.8, (y as f32 + 0.5) * TILE_M);
                if let Some(p) = project(&cam, world, sw, sh) {
                    let width = measure_text(name, None, 20, 1.0).width;
                    draw_text(name, p.x - width / 2.0, p.y, 20.0, Color::new(0.7, 1.0, 0.8, 1.0));
                }
            }
        }
    }

    draw_rectangle(sw / 2.0 - 1.0, sh / 2.0 - 1.0, 2.0, 2.0, Color::new(1.0, 1.0, 1.0, 0.7));

    draw_text(&map.name, 16.0, 28.0, 24.0, WHITE);
    let inv: Vec<String> = g
        .inv
        .iter()
        .map(|id| g.pack.items.get(id).map_or_else(|| id.clone(), |i| i.name.clone()))
        .collect();
    let inv_text = if inv.is_empty() { "Carrying: nothing".to_string() } else { format!("Carrying: {}", inv.join(", ")) };
    draw_text(&inv_text, 16.0, 52.0, 18.0, Color::new(0.8, 0.8, 0.85, 1.0));
    draw_text(
        "WASD move   mouse look   Shift sprint   Ctrl crouch   Space jump   E talk   Esc quit",
        16.0,
        sh - 8.0,
        16.0,
        Color::new(0.55, 0.55, 0.6, 1.0),
    );

    if let Some((text, left)) = g.toast.as_ref() {
        let alpha = (left / 0.6).min(1.0);
        let width = measure_text(text, None, 22, 1.0).width;
        draw_text(text, (sw - width) / 2.0, 80.0, 22.0, Color::new(1.0, 0.95, 0.7, alpha));
    }

    if let Some(talk) = g.talk.as_ref() {
        draw_talk(g, talk.choosing, talk.shown, talk.sel, sw, sh);
    }
}

fn draw_talk(g: &Game, choosing: bool, shown: f32, sel: usize, sw: f32, sh: f32) {
    let (x, w) = (40.0, sw - 80.0);
    let y = sh - BOX_H - 16.0;
    draw_rectangle(x, y, w, BOX_H, Color::new(0.04, 0.04, 0.07, 0.94));
    draw_rectangle_lines(x, y, w, BOX_H, 2.0, Color::new(0.55, 0.5, 0.35, 1.0));

    if let Some(line) = g.current_line() {
        if !line.speaker.is_empty() {
            draw_text(&line.speaker, x + 20.0, y + 32.0, 24.0, Color::new(1.0, 0.82, 0.45, 1.0));
        }
        let total = line.text.chars().count();
        let shown_text: String = line.text.chars().take((shown as usize).min(total)).collect();
        for (i, row) in wrap(&shown_text, w - 40.0, TEXT_SIZE).iter().enumerate() {
            draw_text(row, x + 20.0, y + 62.0 + i as f32 * LINE_H, TEXT_SIZE, WHITE);
        }
        if !choosing && shown >= total as f32 {
            draw_text("E / Space", x + w - 120.0, y + BOX_H - 14.0, 16.0, Color::new(0.6, 0.6, 0.65, 1.0));
        }
    }

    if choosing {
        let choices = g.offered_choices();
        let panel_h = choices.len() as f32 * 30.0 + 16.0;
        let top = y - panel_h - 8.0;
        draw_rectangle(x, top, w, panel_h, Color::new(0.06, 0.06, 0.1, 0.96));
        draw_rectangle_lines(x, top, w, panel_h, 2.0, Color::new(0.55, 0.5, 0.35, 1.0));
        for (i, text) in choices.iter().enumerate() {
            let selected = i == sel;
            let colour = if selected { Color::new(1.0, 0.85, 0.4, 1.0) } else { WHITE };
            let marker = if selected { ">" } else { " " };
            draw_text(
                format!("{marker} {}. {text}", i + 1),
                x + 20.0,
                top + 28.0 + i as f32 * 30.0,
                TEXT_SIZE,
                colour,
            );
        }
    }
}

/// Greedy word wrap at `max_w` pixels.
fn wrap(text: &str, max_w: f32, size: f32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && measure_text(&candidate, None, size as u16, 1.0).width > max_w {
            rows.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = candidate;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_story_cli_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A core pack with one map: a 1x1 picture whose only pixel is the green spawn. It is the smallest pack `load` accepts.
    fn core_pack(root: &Path) -> PathBuf {
        let core = root.join("sample_core");
        fs::create_dir_all(core.join("maps")).unwrap();
        fs::write(
            core.join("pack.json"),
            r#"{"schema": "lr.pack/1", "id": "sample_core", "name": "Sample", "start": {"map": "start", "spawn": "here"}}"#,
        )
        .unwrap();
        fs::write(
            core.join("maps").join("start.json"),
            r#"{"schema": "lr.story.map/1", "id": "start", "name": "Start", "art": "start.png", "legend": {"00ff00": {"kind": "spawn", "name": "here"}}}"#,
        )
        .unwrap();
        let art = Image::gen_image_color(1, 1, Color::new(0.0, 1.0, 0.0, 1.0));
        art.export_png(&core.join("maps").join("start.png").to_string_lossy());
        core
    }

    #[test]
    fn init_writes_a_pack_that_loads_and_refuses_a_bad_id_or_an_existing_folder() {
        let root = scratch("init");
        let core = core_pack(&root);
        let mods = root.join("mods");
        let dir = write_starter_pack(&mods, "my_story").unwrap();
        assert!(dir.join("pack.json").is_file());
        assert!(dir.join("scenes").join("main.json").is_file());
        assert!(dir.join("items").join("items.json").is_file());

        let cfg = load::Config { core, mod_dirs: vec![dir.clone()], mod_roots: Vec::new() };
        let pack = load_or_report(&cfg).expect("the starter pack loads");
        assert!(pack.scenes.contains_key("my_story_intro"), "the starter scene is in the merged pack");
        assert_eq!(pack.scenes["my_story_intro"].lines.len(), 1);

        assert!(write_starter_pack(&mods, "my_story").unwrap_err().contains("already exists"));
        assert!(write_starter_pack(&mods, "Bad Id").unwrap_err().contains("not a valid pack id"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_hash_is_the_same_twice_and_changes_when_a_file_does() {
        let root = scratch("hash");
        let dir = write_starter_pack(&root, "hash_pack").unwrap();
        let first = pack_summary(&dir).unwrap();
        let second = pack_summary(&dir).unwrap();
        assert_eq!(first, second, "the same folder gives the same fingerprint");
        assert_eq!(first.id, "hash_pack");
        assert_eq!(first.version, "0.1.0");
        assert_eq!(first.files, 3);
        assert_eq!(first.sha256.len(), 64);

        let items = dir.join("items").join("items.json");
        let old_len = fs::metadata(&items).unwrap().len();
        fs::write(&items, "{}\n").unwrap();
        let changed = pack_summary(&dir).unwrap();
        assert_ne!(first.sha256, changed.sha256, "editing a file changes the hash");
        assert_eq!(changed.bytes, first.bytes - old_len + 3, "the byte count follows the file");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn positionals_skip_flag_values_and_reject_unknown_flags() {
        let owned = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        let words = positionals(&owned(&["a.yml", "--id", "x", "b.yml", "--into", "dir"]), VALUE_FLAGS).unwrap();
        assert_eq!(words, vec!["a.yml".to_string(), "b.yml".to_string()]);
        assert!(positionals(&owned(&["--bogus"]), VALUE_FLAGS).is_err());
        assert!(positionals(&owned(&["--id"]), VALUE_FLAGS).is_err(), "a flag with no value is an error");
    }
}
