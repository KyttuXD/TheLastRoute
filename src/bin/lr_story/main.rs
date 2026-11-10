//! KYTTU - LRS (lr_story): a data-driven story game in first person. The content is a pack folder (pack.json, maps/, scenes/,
//! items/); the engine reads it and plays it. A map is a PNG picture (the floor) plus a mask PNG whose colours say what
//! each tile is (walls are boxes, doors, NPCs, items, triggers, exits). The body and its movement come from the Last
//! Route runner (see body.rs). See docs/LR_STORY.md for the format.
//!
//! `cargo run --bin lr_story -- [--pack DIR] [--mod DIR]... [--story ID] [--check] [--shot PATH [--scene ID]] [--menu-shot PATH]`
//!   --pack DIR    the pack folder (default: the LR_STORY_PACK env var, else data/lr_story in this crate)
//!   --mod DIR     an extra mod pack folder (repeatable)
//!   --story ID    play the story with this id (a pack id) directly. Without it a menu lists the stories, sorted by
//!                 order then title: W/S or the arrows move, Enter picks, Esc quits. With --shot and no --story, the first.
//!   --check       load and validate the pack, print a summary and exit (status 1 when it has problems)
//!   --shot PATH   render a few frames, save the screen to PATH and exit (for checking the view); --scene starts a scene.
//!                 PATH's folder must exist; --scene must name a scene of the packs. A shot never pauses.
//!   --menu-shot PATH   draw the start menu for a few frames, save the screen to PATH and exit
//!   --flat-textures   paint the lab surfaces flat (textures::Style::Flat) wherever textures::paint is used
//!   --texture-sheet PATH   write the lab texture contact sheet (a painted and a flat row) to PATH as a PNG, then exit
//!
//! In a fight (action battles only): left mouse attacks, right mouse held blocks, 1-4 use the first four usable items,
//! J opens the journal and I or Tab the inventory (both hold the fight still).
//!
//! In the game, Esc opens a pause menu (Resume, Settings, Back to menu, Quit), a fight included. Flags take `--flag value` or `--flag=value`; an
//! unknown option or a flag with no value is a usage error (status 2). A subcommand may come before or after the flags.
//!
//! Subcommands (they take the same `--pack` and `--mod` flags):
//!   check                              the same as --check
//!   init <id> [parent]                 writes a starter mod pack to <parent or ./mods>/<id>, then checks it loads
//!   pack <dir>                         prints the pack's id, version, file count, total bytes and a sha256 fingerprint
//!   import-lore <files...> --id <id> [--into DIR]
//!                                      imports KyttusLab-LORE files as scenes (default DIR: ./mods/<id>), then checks

mod anim;
mod assets_vfs;
mod battle_ui;
mod body;
mod floor;
mod hud;
mod input;
mod inventory;
mod load;
mod lore;
mod look;
mod mannequin;
mod menu;
mod model;
mod multiplayer;
mod net;
mod npc_ai;
mod npc_anim;
mod play;
mod props;
mod rpg;
mod save;
mod scenery;
mod settings;
mod settings_ui;
mod textures;
mod ui;
mod weather;
mod yaml;

#[cfg(test)]
mod guard;

#[cfg(test)]
mod simulora_pilot;

use body::{Input, TILE_M};
use input::Action;
use macroquad::camera::Camera;
use macroquad::prelude::{
    clear_background, draw_cube, draw_line, draw_mesh, draw_plane, draw_rectangle, draw_rectangle_ex, draw_sphere, get_frame_time, get_internal_gl, get_screen_data,
    get_time, is_key_down, is_key_pressed, mouse_delta_position, mouse_wheel, next_frame, screen_height, screen_width, set_camera,
    set_cursor_grab, set_default_camera, show_mouse, vec2, vec3, Camera3D, Color, Conf, DrawRectangleParams, Image, ImageFormat,
    KeyCode, Mat4, Mesh, Texture2D, Vec2, Vec3, Vertex, WHITE,
};
use mannequin::{third_person_offset, Figures, TP_LOOK_M};
use model::{Kind, Pack, Rgb};
use play::Game;
use props::builder::submit_all;
use props::prop_model::{AtlasCache, Placement, PropModel, StaticCache};
use props::prop_scale::PropDrawScale;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// Frames rendered before a `--shot` or `--menu-shot` is saved (so the first frame's textures and camera are settled).
const SHOT_FRAMES: u32 = 4;
const MENU_SHOT_FRAMES: u32 = 30;

/// The command line after the program name, with every `--flag=value` split into `--flag` and `value`.
fn cli_args() -> Vec<String> {
    normalize(&std::env::args().skip(1).collect::<Vec<String>>())
}

/// Splits `--flag=value` words into two. Any other word is kept as written.
fn normalize(raw: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(raw.len());
    for a in raw {
        match a.strip_prefix("--").and_then(|rest| rest.split_once('=')) {
            Some((name, value)) => {
                out.push(format!("--{name}"));
                out.push(value.to_string());
            }
            None => out.push(a.clone()),
        }
    }
    out
}

fn arg_value(flag: &str) -> Option<String> {
    value_of(&cli_args(), flag)
}

/// Every value given with a repeated flag, in order (`--mod A --mod B`).
fn arg_values(flag: &str) -> Vec<String> {
    let args = cli_args();
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
    // The browser has no working folder or user folder: its mods are the ones the web manifest lists, under the crate's
    // mods folder (see assets_vfs). Native builds never take this branch.
    if cfg!(target_arch = "wasm32") {
        roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mods"));
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
        window_title: "KYTTU - LRS".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

/// Flags that take a value. A word after one of them is its value, never a subcommand or a positional word.
const VALUE_FLAGS: &[&str] = &[
    "--pack",
    "--mod",
    "--id",
    "--into",
    "--story",
    "--shot",
    "--scene",
    "--texture-sheet",
    "--menu-shot",
];
/// Flags that stand alone.
const BOOL_FLAGS: &[&str] = &["--check", "--flat-textures", "--help"];
/// The subcommands.
const COMMANDS: &[&str] = &["check", "init", "pack", "import-lore"];

const USAGE: &str = "usage:
  lr_story [--pack DIR] [--mod DIR]... [--story ID] [--shot PATH [--scene ID]] [--menu-shot PATH] [--flat-textures]
  lr_story --check | --texture-sheet PATH
  lr_story check | init <id> [parent] | pack <dir> | import-lore <files...> --id <id> [--into DIR]
flags take `--flag value` or `--flag=value`; a subcommand may come before or after the flags";

fn main() {
    let args = cli_args();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }
    let (command, rest) = match parse_command(&args) {
        Ok(parsed) => parsed,
        Err(e) => std::process::exit(usage(&e)),
    };
    textures::set_flat_textures(rest.iter().any(|a| a == "--flat-textures"));
    if rest.iter().any(|a| a == "--texture-sheet") {
        let code = match value_of(&rest, "--texture-sheet") {
            Some(path) => texture_sheet(Path::new(&path)),
            None => usage("--texture-sheet needs a path: --texture-sheet PATH"),
        };
        std::process::exit(code);
    }
    let code = match command.as_deref() {
        Some("init") => cmd_init(&rest),
        Some("pack") => cmd_pack(&rest),
        Some("import-lore") => cmd_import_lore(&rest),
        Some("check") => cmd_check(&rest),
        _ if rest.iter().any(|a| a == "--check") => check_with(None),
        _ => {
            play();
            return;
        }
    };
    std::process::exit(code);
}

/// Checks every flag of the (normalised) command line and splits off the subcommand: the first plain word, when it is
/// one of `COMMANDS`, wherever it sits among the flags. Returns it with the other words and flags in their order. A plain
/// first word that is no command, an unknown `--option`, and a flag that needs a value and has none are errors.
fn parse_command(args: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let words = positional_words(args, VALUE_FLAGS)?;
    match words.first() {
        None => Ok((None, args.to_vec())),
        Some((i, word)) if COMMANDS.contains(&word.as_str()) => {
            let mut rest = args.to_vec();
            rest.remove(*i);
            Ok((Some(word.clone()), rest))
        }
        Some((_, word)) => Err(format!("unknown command '{word}' (commands: {})", COMMANDS.join(", "))),
    }
}

/// `check`: the same as `--check`; it takes no words.
fn cmd_check(args: &[String]) -> i32 {
    match positionals(args, VALUE_FLAGS) {
        Ok(words) if words.is_empty() => check_with(None),
        Ok(_) => usage("check takes no arguments, only flags"),
        Err(e) => usage(&e),
    }
}

/// Whether `path` can be written as a picture: its folder must exist, and it must not be a folder itself. `flag` names
/// the option in the message.
#[cfg(not(target_arch = "wasm32"))]
fn check_shot_path(flag: &str, path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!("{flag} needs a file name, but {} is a folder", path.display()));
    }
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(folder) if !folder.is_dir() => {
            Err(format!("{flag}: the folder {} does not exist; make it first", folder.display()))
        }
        _ => Ok(()),
    }
}

/// `--texture-sheet PATH`: writes the lab contact sheet (a painted row and a flat row) as a PNG. Status 1 when it fails.
fn texture_sheet(path: &Path) -> i32 {
    match textures::sheet::write_lab_sheet(path) {
        Ok(()) => {
            println!("lr_story: wrote the lab texture sheet to {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("lr_story: could not write the texture sheet: {e}");
            1
        }
    }
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
                "OK: '{}' - {} map(s), {} scene(s), {} item(s), {} story(ies)",
                pack.title,
                pack.maps.len(),
                pack.scenes.len(),
                pack.items.len(),
                pack.stories.len()
            );
            0
        }
        None => 1,
    }
}

/// Opens the game window on the pack the flags name. Exits with status 2 when a picture path has no folder, and with
/// status 1 when the pack has problems, or when `--story` or `--scene` names one the packs do not have. All of this is
/// checked before the window opens.
#[cfg(not(target_arch = "wasm32"))]
fn play() {
    let shot = arg_value("--shot").map(PathBuf::from);
    let menu_shot = arg_value("--menu-shot").map(PathBuf::from);
    for (flag, path) in [("--shot", &shot), ("--menu-shot", &menu_shot)] {
        if let Some(path) = path {
            if let Err(e) = check_shot_path(flag, path) {
                std::process::exit(usage(&e));
            }
        }
    }
    let Some(pack) = load_or_report(&load_config()) else {
        std::process::exit(1);
    };
    let scene = arg_value("--scene");
    let story = arg_value("--story");
    if let Some(id) = &story {
        if pack.story(id).is_none() {
            let ids: Vec<&str> = pack.stories.iter().map(|s| s.id.as_str()).collect();
            eprintln!("lr_story: no story '{id}'; the stories are: {}", ids.join(", "));
            std::process::exit(1);
        }
    }
    if let Some(id) = &scene {
        if !pack.scenes.contains_key(id) {
            eprintln!("lr_story: no scene '{id}' in the packs ({} scene(s) are loaded)", pack.scenes.len());
            std::process::exit(1);
        }
    }
    macroquad::Window::from_config(window_conf(), run(pack, shot, scene, story, menu_shot));
}

/// The browser build. A page has no command line, so the window opens the story menu of the sample pack set. The
/// game files are fetched first (assets_vfs::preload) inside the window's own loop; the packs are read from them after.
/// A failure stays on screen: macroquad ends the window when its future returns, and on the web that leaves a black
/// canvas with the message only in the console, so every failure draws its text and waits instead of returning.
#[cfg(target_arch = "wasm32")]
fn play() {
    macroquad::Window::from_config(window_conf(), async {
        if let Err(e) = WithLoadingScreen::new(assets_vfs::preload()).await {
            show_error(vec!["could not load the game files".to_string(), e]).await;
            return;
        }
        let pack = match load::load(&load_config()) {
            Ok(pack) => pack,
            Err(errs) => {
                let mut lines = vec!["the story pack has problems:".to_string()];
                lines.extend(errs.iter().map(|e| format!("  - {e}")));
                // The game files are in now, so the fonts can be read for the error screen.
                ui::init();
                show_error(lines).await;
                return;
            }
        };
        run(pack, None, None, None, None).await;
    });
}

/// Wraps the preload so a frame is drawn each time it is still waiting: "Loading..." and the preload's status line.
/// macroquad polls the window's future inside its frame, so the draws made here reach the screen.
#[cfg(target_arch = "wasm32")]
struct WithLoadingScreen<F> {
    inner: std::pin::Pin<Box<F>>,
}

#[cfg(target_arch = "wasm32")]
impl<F: std::future::Future> WithLoadingScreen<F> {
    fn new(inner: F) -> Self {
        Self { inner: Box::pin(inner) }
    }
}

#[cfg(target_arch = "wasm32")]
impl<F: std::future::Future> std::future::Future for WithLoadingScreen<F> {
    type Output = F::Output;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        let this = self.get_mut();
        match this.inner.as_mut().poll(cx) {
            std::task::Poll::Ready(value) => std::task::Poll::Ready(value),
            std::task::Poll::Pending => {
                // The fonts are among the files still coming, so this screen uses macroquad's built-in one.
                hud::draw_loading(&assets_vfs::status(), get_time() as f32);
                std::task::Poll::Pending
            }
        }
    }
}

/// Draws the error lines each frame, forever: the window stays open so the text can be read.
#[cfg(target_arch = "wasm32")]
async fn show_error(lines: Vec<String>) {
    loop {
        hud::draw_error(&lines, get_time() as f32);
        next_frame().await;
    }
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

/// The words in `args` that are neither flags nor the values of `value_flags`, each with its place in `args`. An unknown
/// `--` flag is an error, and so is a value flag with no value (nothing after it, an empty word, or another `--` flag).
fn positional_words(args: &[String], value_flags: &[&str]) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if value_flags.contains(&a) {
            match args.get(i + 1) {
                Some(v) if !v.is_empty() && !v.starts_with("--") => i += 2,
                _ => return Err(format!("{a} needs a value: {a} <value> or {a}=<value>")),
            }
        } else if BOOL_FLAGS.contains(&a) {
            i += 1;
        } else if a.starts_with("--") {
            return Err(format!("unknown option '{a}'\n{USAGE}"));
        } else {
            out.push((i, args[i].clone()));
            i += 1;
        }
    }
    Ok(out)
}

/// `positional_words` without the places.
fn positionals(args: &[String], value_flags: &[&str]) -> Result<Vec<String>, String> {
    Ok(positional_words(args, value_flags)?.into_iter().map(|(_, w)| w).collect())
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

/// How a story's play ended.
enum Leave {
    /// Back to the start menu, to pick again.
    Menu,
    /// The program ends.
    Quit,
}

/// The program: the start menu, then the story it picks, then the menu again after "Back to menu". `--story` names the
/// first story (the menu is skipped once); `--scene` starts that scene in it. A `--shot` plays one story and exits
/// (without `--story`, the first in menu order, since a shot cannot wait for a pick); `--menu-shot` draws the menu and exits.
async fn run(
    pack: Pack,
    shot: Option<PathBuf>,
    scene: Option<String>,
    story: Option<String>,
    menu_shot: Option<PathBuf>,
) {
    ui::init();
    if let Some(path) = menu_shot {
        menu_shot_run(&pack, &path).await;
        return;
    }
    let mut story = story;
    let mut scene = scene;
    loop {
        // The story to play: the one `--story` names (once), else the menu's pick. Escape at the menu quits.
        let story_id = match story.take() {
            Some(id) => id,
            None if shot.is_some() => match pack.stories_in_menu_order().first() {
                Some(first) => first.id.clone(),
                None => return,
            },
            None => match menu::run(&pack).await {
                Some(id) => id,
                None => return,
            },
        };
        match play_story(&pack, &story_id, scene.take(), shot.as_deref()).await {
            Leave::Menu if shot.is_none() => continue,
            _ => return,
        }
    }
}

/// `--menu-shot`: draws the start menu for a few frames, saves the screen and returns.
async fn menu_shot_run(pack: &Pack, path: &Path) {
    let mut menu = menu::Menu::new(pack);
    for frame in 1..=MENU_SHOT_FRAMES {
        let _ = menu.frame(0.016);
        if frame == MENU_SHOT_FRAMES {
            save_shot(path);
            break;
        }
        next_frame().await;
    }
}

/// Plays one story until the player leaves it. The flags are saved before it returns.
async fn play_story(pack: &Pack, story_id: &str, scene: Option<String>, shot: Option<&Path>) -> Leave {
    let story_id = story_id.to_string();
    let pack = pack.clone();
    // The flat-textures setting applies from the story start, unless the command line asked for them.
    if !cli_args().iter().any(|a| a == "--flat-textures") {
        textures::set_flat_textures(settings::get().flat_textures);
    }
    // The decoded art of each map: the floor is drawn from it as 1 m blocks (floor.rs), so the picture stays an Image.
    let mut images: HashMap<String, Image> = HashMap::new();
    let mut floor_cache = floor::FloorCache::default();
    let mut scenery_cache = scenery::SceneryCache::default();
    for (id, map) in &pack.maps {
        let loaded = assets_vfs::read(&map.art)
            .ok()
            .and_then(|bytes| Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).ok());
        match loaded {
            Some(img) => {
                images.insert(id.clone(), img);
            }
            None => eprintln!("lr_story: could not load the picture for map '{id}'"),
        }
    }
    // The prop models, read before the pack moves into the game.
    let mut prop_set = load_props(&pack);
    // The NPC skins, read the same way: each is the mannequin atlas of the NPCs that name it.
    let skins = load_skins(&pack);
    // The flags the player set last time, read before the start scene runs so it sees them.
    // A save that cannot be read is left alone: this run starts without it and does not write over it.
    let mut saver = Saver { file: flags_file(&story_id), last: BTreeSet::new(), last_party: None };
    let mut unreadable = None;
    let saved = match &saver.file {
        Some((path, id)) => save::load_flags(path, id).unwrap_or_else(|e| {
            eprintln!("lr_story: could not read the saved flags, starting without them: {e}");
            unreadable = Some(e);
            BTreeSet::new()
        }),
        None => BTreeSet::new(),
    };
    if unreadable.is_some() {
        saver.file = None;
    }
    saver.last = saved.clone();
    let mut game = match Game::for_story(pack, &story_id, saved) {
        Ok(game) => game,
        Err(e) => {
            eprintln!("lr_story: {e}");
            return Leave::Quit;
        }
    };
    // The party (hp) saved beside the flags. A save that cannot be read is left alone, like the flags.
    if let Some((path, id)) = &saver.file {
        match save::load_party(path, id) {
            Ok(Some(ps)) => apply_party_save(&mut game.party, &ps),
            Ok(None) => {}
            Err(e) => {
                eprintln!("lr_story: could not read the saved party, starting without it: {e}");
                unreadable = Some(e);
                saver.file = None;
            }
        }
    }
    saver.last_party = Some(party_key(&game.party));
    if let Some(id) = scene {
        game.start_scene(&id);
    }
    if unreadable.is_some() {
        game.toast("Saved progress could not be read; this run will not save");
    }
    if shot.is_none() {
        set_cursor_grab(true);
        show_mouse(false);
    }

    let walls = wall_textures(&game.pack);
    // The item icons for the hotbar and the inventory screen, loaded once by item id.
    let icons = load_icons(&game.pack);
    let mut figures = Figures::default();
    let mut overlay = hud::Hud::new();
    let mut panels = hud::Panels::new();
    let mut battle_ui = battle_ui::BattleUi::new();
    // The pause menu, while it is open. The scene stands still behind it and the cursor is free.
    let mut pause: Option<hud::Pause> = None;
    // The settings screen, opened from the pause menu (the pause stays under it).
    let mut settings_screen: Option<settings_ui::SettingsScreen> = None;
    // Set on resume: the mouse moved while the cursor was free, so that frame's look step is dropped.
    let mut skip_look = false;
    let mut frame = 0u32;
    loop {
        if is_key_pressed(KeyCode::Escape) {
            if shot.is_some() {
                // A shot never pauses.
                saver.sync(&mut game);
                return Leave::Quit;
            }
            // On the journal or inventory Esc closes the screen; otherwise it pauses, a fight included (the fight is not
            // ticked while the menu is open, and a held block is let go).
            if pause.is_none() && !panels.is_open() {
                battle_ui::release(&mut game);
                pause = Some(hud::Pause::new());
                set_cursor_grab(false);
                show_mouse(true);
            }
        }
        if pause.is_some() {
            draw(&game, &images, &mut floor_cache, &mut scenery_cache, &skins, &walls, &mut prop_set, &mut figures, &battle_ui, 0.0);
            overlay.draw(&game);
            draw_battle_ui(&game, &battle_ui);
            panels.draw(&game, &icons);
            let outcome = if let Some(screen) = settings_screen.as_mut() {
                let skin_of = |id: &str| skins.get(id).cloned();
                if matches!(screen.frame(get_frame_time().min(0.1), &game.pack, &skin_of), settings_ui::SettingsAct::Close) {
                    settings_screen = None;
                }
                None
            } else {
                pause.as_mut().and_then(|p| p.frame(get_time() as f32))
            };
            match outcome {
                Some(hud::PauseChoice::Settings) => {
                    settings_screen = Some(settings_ui::SettingsScreen::new());
                }
                Some(hud::PauseChoice::Resume) => {
                    pause = None;
                    set_cursor_grab(true);
                    show_mouse(false);
                    let _ = mouse_delta_position();
                    skip_look = true;
                }
                Some(hud::PauseChoice::Menu) => {
                    saver.sync(&mut game);
                    set_cursor_grab(false);
                    show_mouse(true);
                    return Leave::Menu;
                }
                Some(hud::PauseChoice::Quit) => {
                    saver.sync(&mut game);
                    return Leave::Quit;
                }
                None => {}
            }
            next_frame().await;
            continue;
        }
        let dt = get_frame_time().min(0.1);
        let mut input = read_input(&game);
        if std::mem::take(&mut skip_look) {
            input.look = vec2(0.0, 0.0);
        }
        // The journal and the inventory hold the world still: no walking or looking while one is open.
        if panels.is_open() {
            input = Input { fwd: 0.0, strafe: 0.0, sprint: false, crouch: false, jump: false, look: vec2(0.0, 0.0) };
        }
        if settings::get().bindings.pressed(Action::View) && !panels.is_open() {
            game.toggle_third_person();
        }
        // The first frame of a story ignores the action keys: the Enter that picked the story is still down.
        if frame > 0 {
            if panels.is_open() {
                battle_ui::release(&mut game);
                panels.handle(&mut game);
            } else {
                panels.poll_open(&game);
                if panels.is_open() {
                    battle_ui::release(&mut game);
                } else {
                    // Left mouse attacks, right mouse held blocks, 1-9 / wheel / F are the hotbar (a fight has no talk and no E).
                    battle_ui.handle(&mut game);
                    // 1-9 and the wheel select a hotbar slot, F uses it (digits still pick dialogue choices).
                    if hud::digit_route(game.talk.as_ref().map(|t| t.choosing), false) == hud::DigitRoute::Slot {
                        handle_hotbar(&mut game, &mut overlay);
                    }
                    if game.battle.is_none() {
                        handle_actions(&mut game);
                    }
                }
            }
        }
        game.update_body(dt, &input);
        game.tick(dt);
        // An open journal or inventory holds the fight still.
        if !panels.is_open() {
            game.battle_tick(dt);
            // NPCs walk their routines, react to the weather and to a fight (a pause, the journal or the inventory holds them).
            game.npc_tick(dt);
        }
        // Flags and the party change through choices, scene ops, triggers, exits and fights alike; each change is saved once.
        saver.sync(&mut game);
        overlay.update(&game, dt);
        battle_ui.update(&game, dt);
        draw(&game, &images, &mut floor_cache, &mut scenery_cache, &skins, &walls, &mut prop_set, &mut figures, &battle_ui, dt);
        overlay.draw(&game);
        if !panels.is_open() {
            overlay.draw_hotbar(&game, &icons);
        }
        draw_battle_ui(&game, &battle_ui);
        panels.draw(&game, &icons);
        frame += 1;
        if let Some(path) = shot {
            if frame >= SHOT_FRAMES {
                save_shot(path);
                return Leave::Quit;
            }
        }
        next_frame().await;
    }
}

/// Saves the screen as a PNG. The frame buffer keeps the alpha that translucent panels blended into it, which a picture
/// viewer would show as a washed-out grey, so every pixel is written opaque (the window itself is always opaque).
fn save_shot(path: &Path) {
    let mut img = get_screen_data();
    for px in img.bytes.chunks_mut(4) {
        px[3] = 255;
    }
    img.export_png(&path.to_string_lossy());
}

/// The body's input for this frame: keys for movement, the mouse for look. Mouse look is in radians.
fn read_input(g: &Game) -> Input {
    let s = settings::get();
    let b = &s.bindings;
    let held = |keys: &[KeyCode]| keys.iter().any(|k| is_key_down(*k));
    let axis = |plus: bool, minus: bool| (plus as i32 - minus as i32) as f32;
    // macroquad's delta is last minus current, the opposite of how the mouse moved, so it is negated: moving down or
    // right gives a positive step (last_route config/look.rs delta_px does the same). Scaled to pixels, then radians.
    // The same maths as last_route (look.rs, copied from config/look.rs): half the screen per delta unit, its
    // sensitivity and its per-platform multiplier.
    let look = look_step(look::radians(mouse_delta_position(), vec2(screen_width(), screen_height())), s.mouse_sensitivity, s.invert_y);
    Input {
        fwd: axis(b.down(Action::Forward) || held(&[KeyCode::Up]), b.down(Action::Back) || held(&[KeyCode::Down])),
        strafe: axis(b.down(Action::Right) || held(&[KeyCode::Right]), b.down(Action::Left) || held(&[KeyCode::Left])),
        sprint: b.down(Action::Sprint),
        crouch: b.down(Action::Crouch),
        jump: g.talk.is_none() && b.down(Action::Jump),
        look,
    }
}

/// A look step (radians, x turns right, y looks down) scaled by the mouse sensitivity setting; `invert_y` flips the vertical.
fn look_step(step: Vec2, sensitivity: f32, invert_y: bool) -> Vec2 {
    vec2(step.x * sensitivity, step.y * sensitivity * if invert_y { -1.0 } else { 1.0 })
}

/// The camera's vertical field of view in radians from the setting (clamped to the 60..=100 degrees the screen offers).
fn fov_radians(deg: f32) -> f32 {
    deg.clamp(60.0, 100.0).to_radians()
}

/// The flags file and the story id the flags are saved under. None when there is no user folder.
fn flags_file(story_id: &str) -> Option<(PathBuf, String)> {
    Some((save::default_path()?, story_id.to_string()))
}

/// Writes the story's flags to the flags file whenever they differ from what was last written. `file` is None when
/// there is no user folder, when the saved progress could not be read (it is not written over), or after a failed write
/// (the player is told once).
struct Saver {
    file: Option<(PathBuf, String)>,
    last: BTreeSet<String>,
    /// The party as last written (see `party_key`); None until the start-up load has set it.
    last_party: Option<(i32, i32)>,
}

/// The party values that are saved (hp, max_hp), to compare against what was last written.
fn party_key(p: &rpg::Party) -> (i32, i32) {
    (p.hp, p.max_hp)
}

/// Puts a saved party on a fresh one. Only hp and max hp are kept; an old save's level and xp are ignored. The saved HP
/// is kept between 1 and the saved maximum.
fn apply_party_save(p: &mut rpg::Party, s: &save::PartySave) {
    restore_hp(p, s.hp, s.max_hp);
}

/// Sets the party's maximum HP (at least 1) and its HP between 1 and that maximum.
fn restore_hp(p: &mut rpg::Party, hp: i32, max_hp: i32) {
    p.max_hp = max_hp.max(1);
    p.hp = hp.clamp(1, p.max_hp);
    p.stats.max_hp = p.max_hp;
}

impl Saver {
    fn sync(&mut self, g: &mut Game) {
        if g.flags != self.last {
            if let Some((path, id)) = &self.file {
                match save::save_flags(path, id, &g.flags) {
                    Ok(()) => self.last = g.flags.clone(),
                    Err(e) => {
                        eprintln!("lr_story: could not save the flags: {e}");
                        g.toast("Progress not saved");
                        self.file = None;
                    }
                }
            }
        }
        let key = party_key(&g.party);
        if self.last_party != Some(key) {
            if let Some((path, id)) = &self.file {
                let ps = save::PartySave { hp: key.0, max_hp: key.1 };
                match save::save_party(path, id, &ps) {
                    Ok(()) => self.last_party = Some(key),
                    Err(e) => {
                        eprintln!("lr_story: could not save the party: {e}");
                        g.toast("Progress not saved");
                        self.file = None;
                    }
                }
            }
        }
    }
}

/// The presses that are not movement: E / Enter talk or go on, Space goes on while a scene plays, digits and
/// arrows pick a choice.
fn handle_actions(g: &mut Game) {
    let b = settings::get().bindings;
    let pressed = |keys: &[KeyCode]| keys.iter().any(|k| is_key_pressed(*k));
    let confirm_pressed = || pressed(&[KeyCode::Space, KeyCode::Enter]) || b.pressed(Action::Interact);

    if let Some((choosing, sel)) = g.talk.as_ref().map(|t| (t.choosing, t.sel)) {
        if choosing {
            if pressed(&[KeyCode::Up]) || b.pressed(Action::Forward) {
                g.move_sel(-1);
            }
            if pressed(&[KeyCode::Down]) || b.pressed(Action::Back) {
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
                g.choose(n);
            } else if confirm_pressed() {
                let now = g.talk.as_ref().map_or(sel, |t| t.sel);
                g.choose(now);
            }
        } else if confirm_pressed() {
            g.advance();
        }
        return;
    }
    if pressed(&[KeyCode::Enter]) || b.pressed(Action::Interact) {
        g.interact();
    }
}

/// The camera moved `shake` = (sideways, up) metres, both position and target, so the view jolts without turning.
fn shake_camera(mut cam: Camera3D, shake: (f32, f32)) -> Camera3D {
    if shake == (0.0, 0.0) {
        return cam;
    }
    let right = (cam.target - cam.position).cross(cam.up).normalize_or_zero();
    let off = right * shake.0 + cam.up * shake.1;
    cam.position += off;
    cam.target += off;
    cam
}

/// The camera with the battle's screen shake applied.
fn camera_shaken(g: &Game, shake: (f32, f32)) -> Camera3D {
    shake_camera(camera_plain(g), shake)
}

/// The camera: the first-person one at the body's eye, looking along its view direction; or, in the third-person view,
/// behind the body (see `mannequin::third_person_offset`), looking at it.
fn camera_plain(g: &Game) -> Camera3D {
    let fovy = fov_radians(settings::get().fov_deg);
    if g.third_person {
        let feet = vec3(g.body.pos.0, g.body.feet_y, g.body.pos.1);
        return Camera3D {
            position: feet + third_person_offset(g.body.forward2()),
            target: feet + vec3(0.0, TP_LOOK_M, 0.0),
            up: vec3(0.0, 1.0, 0.0),
            fovy,
            z_near: 0.05,
            z_far: 300.0,
            ..Default::default()
        };
    }
    let eye = match g.seated {
        Some(seat) => seated_eye(seat),
        None => g.body.eye(),
    };
    Camera3D {
        position: eye,
        target: eye + g.body.view_dir(),
        up: vec3(0.0, 1.0, 0.0),
        fovy,
        z_near: 0.05,
        z_far: 300.0,
        ..Default::default()
    }
}

/// How far above the seat the eyes are on a seated body (metres).
const SEATED_EYE_M: f32 = 0.68;

/// The camera position of a seated body: `seat` is (x, z, seat height), the eyes sit above the seat.
fn seated_eye(seat: (f32, f32, f32)) -> Vec3 {
    vec3(seat.0, seat.2 + SEATED_EYE_M, seat.1)
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

/// The prop models the loaded packs place, read once when the game starts: each model by the `.bbmodel` file it came
/// from (`None` when it did not load), the draw scale table, and the cached meshes of each placed prop.
struct PropSet {
    models: HashMap<PathBuf, Option<PropModel>>,
    scale: PropDrawScale,
    meshes: StaticCache<Placement, Vec<Mesh>>,
}

/// Reads every prop a map legend places. A prop that does not load is reported on stderr once and kept as `None`, so it
/// is drawn as a cube of its legend colour.
fn load_props(pack: &Pack) -> PropSet {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets");
    let mut atlases = AtlasCache::default();
    let mut models: HashMap<PathBuf, Option<PropModel>> = HashMap::new();
    for map in pack.maps.values() {
        for kind in map.legend.values() {
            let Kind::Prop { prop, .. } = kind else { continue };
            // load.rs checks that every legend prop is in the pack, so a missing one is skipped rather than drawn
            let Some(file) = pack.props.get(prop) else { continue };
            if models.contains_key(file) {
                continue;
            }
            let loaded = file
                .parent()
                .zip(file.file_name())
                .and_then(|(dir, name)| props::load_prop(dir, &name.to_string_lossy(), &mut atlases, false));
            if loaded.is_none() {
                eprintln!("lr_story: could not load prop '{prop}' from {}; drawing a cube", file.display());
            }
            models.insert(file.clone(), loaded);
        }
    }
    PropSet {
        models,
        scale: props::prop_scale::table(&assets),
        meshes: StaticCache::default(),
    }
}

/// The prop type a prop id names, for the draw-scale table: the id without a trailing "_<number>" (labStool_0 is
/// labStool). An id with no such suffix is its own type.
fn prop_type(id: &str) -> &str {
    match id.rsplit_once('_') {
        Some((head, tail)) if !head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => id,
    }
}

/// Draws one placed prop standing on the tile centre (cx, cz), turned `yaw` degrees, at the table's draw scale for its
/// type. Its meshes come from the set's cache while the prop stays on screen. A prop with no model is a cube of its
/// legend colour.
fn draw_prop(set: &mut PropSet, file: Option<&Path>, id: &str, yaw: f32, cx: f32, cz: f32, color: Rgb) {
    let PropSet { models, scale, meshes } = set;
    match file.and_then(|f| models.get(f)).and_then(|m| m.as_ref()) {
        Some(model) => {
            let s = props::prop_scale::draw_scale(scale, prop_type(id));
            let xf = Mat4::from_translation(vec3(cx, 0.0, cz))
                * Mat4::from_rotation_y(yaw.to_radians())
                * Mat4::from_scale(Vec3::splat(s));
            submit_all(model.static_meshes(meshes, xf, WHITE));
        }
        None => draw_cube(vec3(cx, 0.5, cz), vec3(TILE_M * 0.5, 1.0, TILE_M * 0.5), None, col(color)),
    }
}

/// Every skin the pack set ships, read once as a nearest-filter texture. A skin that does not load is reported, and the
/// NPCs that name it are drawn in the default atlas.
fn load_skins(pack: &Pack) -> HashMap<String, Texture2D> {
    let mut skins = HashMap::new();
    for (name, file) in &pack.skins {
        match assets_vfs::read(file).ok().and_then(|bytes| props::bbmodel::try_tex(&bytes)) {
            Some(tex) => {
                skins.insert(name.clone(), tex);
            }
            None => eprintln!("lr_story: could not load the skin '{name}'; its NPCs use the default atlas"),
        }
    }
    skins
}

/// The item icons: each item with an `icon` is read through the asset layer (browser-safe) into a texture. A missing or
/// broken one is left out, and the hotbar draws the hashed colour square for that item.
fn load_icons(pack: &Pack) -> hud::Icons {
    let mut icons = HashMap::new();
    for (id, item) in &pack.items {
        let Some(file) = item.icon.as_ref() else { continue };
        match assets_vfs::read(file).ok().and_then(|bytes| props::bbmodel::try_tex(&bytes)) {
            Some(tex) => {
                icons.insert(id.clone(), tex);
            }
            None => eprintln!("lr_story: could not load the icon of item '{id}'; it uses a colour square"),
        }
    }
    icons
}

/// The hotbar keys while the world is live: 1-9 select a slot, the wheel scrolls the selection, F uses the selected item.
fn handle_hotbar(g: &mut Game, overlay: &mut hud::Hud) {
    if let Some(n) = hud::digit_pressed() {
        g.items.select(n);
        overlay.poke_hotbar();
    }
    let step = hud::wheel_step(mouse_wheel().1);
    if step != 0 {
        g.items.scroll(step);
        overlay.poke_hotbar();
    }
    if settings::get().bindings.pressed(Action::UseItem) {
        overlay.poke_hotbar();
        let empty = g.items.selected_item().is_none();
        if !g.use_selected() {
            g.toast(if empty { "Nothing in that slot" } else { "That cannot be used now" });
        }
    }
}

/// The camera of this frame: the body's view, jolted by the battle's shake.
fn view_camera(g: &Game, battle_ui: &battle_ui::BattleUi) -> Camera3D {
    camera_shaken(g, battle_ui.camera_shake_for(g))
}

/// Draws the battle UI over the scene, projecting world points (hit numbers, the small enemy bar) through this frame's camera.
fn draw_battle_ui(g: &Game, battle_ui: &battle_ui::BattleUi) {
    let (sw, sh) = (screen_width(), screen_height());
    let cam = view_camera(g, battle_ui);
    battle_ui.draw(g, &|p| project(&cam, p, sw, sh));
}

/// The enemy of a fight as its mannequin, at its place and facing its yaw (the convention of body.rs: yaw 0 faces -z), with
/// the battle's skin when the pack has it. A dead enemy is not drawn. A hit flash shows as a white shell round the body.
/// A cube stands in when the body does not load.
fn draw_enemy(g: &Game, st: &rpg::ActionBattle, skins: &HashMap<String, Texture2D>, figures: &mut Figures, dt: f32) {
    if matches!(st.state, rpg::EnemyState::Dead) {
        return;
    }
    let Some(def) = g.pack.battles.get(&st.battle) else { return };
    let (x, z) = st.enemy_pos;
    let facing = (-st.enemy_yaw.sin(), -st.enemy_yaw.cos());
    let toward = (x + facing.0, z + facing.1);
    let atlas = def.skin.as_deref().and_then(|name| skins.get(name).map(|tex| (name, tex)));
    let key = format!("battle:{}", st.battle);
    if !figures.draw_npc(&key, (0, 0), (x, z), model::BodyChoice::default(), atlas, toward, dt, st.flash.clamp(0.0, 1.0)) {
        draw_cube(vec3(x, 0.9, z), vec3(0.7, 1.8, 0.7), None, Color::new(0.8, 0.25, 0.25, 1.0));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    g: &Game,
    images: &HashMap<String, Image>,
    floor_cache: &mut floor::FloorCache,
    scenery_cache: &mut scenery::SceneryCache,
    skins: &HashMap<String, Texture2D>,
    walls: &HashMap<String, Texture2D>,
    prop_set: &mut PropSet,
    figures: &mut Figures,
    battle_ui: &battle_ui::BattleUi,
    dt: f32,
) {
    clear_background(Color::new(0.03, 0.03, 0.05, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let map = g.current_map();
    let (w, h) = (map.grid.w, map.grid.h);
    let cam = view_camera(g, battle_ui);

    set_camera(&cam);
    // An outdoor map's sky goes first: a dome far behind every tile, so the world draws over it.
    let weather = if map.outdoor { hud::weather_of(g) } else { None };
    match weather {
        Some(w) => {
            let (tint, _) = weather::sky_tint(w);
            let (top, bottom) = scenery::sky_of(map);
            let (vertices, indices) = scenery::sky_mesh(tint_rgb(top, tint), tint_rgb(bottom, tint), cam.position);
            draw_mesh(&Mesh { vertices, indices, texture: None });
        }
        None => scenery::draw_sky(map, cam.position),
    }
    // The wall texture of the map's style (the lab one when its style did not paint).
    let wall_tex = walls.get(&map.wall_style).or_else(|| walls.get("lab"));
    // The floor: 1 m blocks cut from the map picture (floor.rs), not one stretched plane.
    if let Some(art) = images.get(&g.map) {
        floor::draw_floor(floor_cache, &g.map, map, art);
    }
    // Every frame asks for each placed prop, so the cache keeps the ones drawn this frame and drops the rest.
    prop_set.meshes.begin_frame();

    let t = get_time() as f32;
    // The player's chosen skin: a pack skin atlas by id; "mannequin" or an unknown id is the default atlas.
    let player_skin = settings::get().player_skin;
    let player_atlas = skins.get(&player_skin).map(|tex| (player_skin.as_str(), tex));
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let cx = (x as f32 + 0.5) * TILE_M;
            let cz = (y as f32 + 0.5) * TILE_M;
            match map.grid.at(x, y) {
                Some(Kind::Wall { height, color }) => draw_wall(cx, cz, *height, col(*color), wall_tex),
                Some(kind @ Kind::Door { height, color, .. }) if kind.blocks(&g.flags, &g.inv) => {
                    draw_cube(vec3(cx, height / 2.0, cz), vec3(TILE_M, *height, TILE_M), None, col(*color));
                }
                Some(Kind::Item { color, .. }) if !g.taken.contains(&format!("{}@{},{}", g.map, x, y)) => {
                    let bob = 0.6 + 0.08 * (t * 2.5 + x as f32).sin();
                    draw_sphere(vec3(cx, bob, cz), 0.22, None, col(*color));
                }
                // NPCs are not drawn from their tiles: they move, so they are drawn from `game.npcs` below.
                Some(Kind::Npc { .. }) => {}
                Some(Kind::Exit { .. }) => {
                    // A soft warm glow on the floor that breathes slowly: it says "a way out" without a painted square.
                    let pulse = 0.5 + 0.5 * (t * 1.6 + (x + y) as f32).sin();
                    draw_plane(vec3(cx, 0.015, cz), vec2(TILE_M * 0.48, TILE_M * 0.48), None, Color::new(1.0, 0.85, 0.55, 0.10 + 0.08 * pulse));
                    draw_plane(vec3(cx, 0.02, cz), vec2(TILE_M * 0.28, TILE_M * 0.28), None, Color::new(1.0, 0.9, 0.65, 0.12 + 0.10 * pulse));
                }
                Some(Kind::Prop { prop, yaw, color, .. }) => {
                    let file = g.pack.props.get(prop).map(PathBuf::as_path);
                    draw_prop(prop_set, file, prop, *yaw, cx, cz, *color);
                }
                _ => {}
            }
        }
    }
    // The NPCs, from their agents (npc_ai.rs): where they walk, how they face, what they do. The coloured cube stands in
    // when the body does not load. Its skin, when it names one that loaded, is the atlas it is painted with.
    for (key, a) in g.npcs.iter().filter(|(_, a)| a.map == g.map) {
        let Some(Kind::Npc { color, body, skin, .. }) = map.grid.at(a.home.0, a.home.1) else { continue };
        let atlas = skin.as_deref().and_then(|name| skins.get(name).map(|tex| (name, tex)));
        let moving = a.moving;
        if !figures.draw_agent(key, a.pos, a.yaw, a.action, moving, *body, atlas, dt, 0.0) {
            draw_cube(vec3(a.pos.0, 0.5, a.pos.1), vec3(0.6, 1.0, 0.6), None, col(*color));
            draw_sphere(vec3(a.pos.0, 1.25, a.pos.1), 0.25, None, col(*color));
        }
    }
    // An indoor map's ceiling, over every non-wall tile (left out while the camera is above it).
    scenery::draw_ceiling(scenery_cache, &g.map, map, cam.position.y);
    // The player's own body, in the third-person view only (first person sees no body). A cube stands in for it when
    // the body does not load.
    if g.third_person && g.seated.is_some() {
        // A seated body sits in the seat: the pose of the sit action on the seat's spot.
        let (sx, sz, _) = g.seated.unwrap_or_default();
        if !figures.draw_agent("player", (sx, sz), g.body.yaw, npc_ai::NpcAction::Sit, false, g.player.body, player_atlas, dt, 0.0) {
            draw_cube(vec3(sx, 0.5, sz), vec3(0.5, 1.0, 0.5), None, WHITE);
        }
    } else if g.third_person && !figures.draw_player(&g.body, g.player.body, player_atlas, dt) {
        draw_cube(vec3(g.body.pos.0, 0.9, g.body.pos.1), vec3(0.5, 1.8, 0.5), None, WHITE);
    }
    // The enemy of a fight, then the floor telegraphs and the projectiles (translucent, so last).
    if let Some(st) = g.battle.as_ref() {
        draw_enemy(g, st, skins, figures, dt);
        battle_ui::draw_world(st);
    }
    // Walls just outside a walkable map edge, so no view ends in the void: the room's height indoors, 6 m outdoors.
    let edge_h = if map.outdoor { 6.0 } else { map.ceiling_height };
    set_camera(&cam);
    for (ex, ez) in edge_wall_cells(map) {
        let (cx, cz) = ((ex as f32 + 0.5) * TILE_M, (ez as f32 + 0.5) * TILE_M);
        draw_wall(cx, cz, edge_h, Color::new(0.78, 0.75, 0.70, 1.0), wall_tex);
    }
    // Fog: haze layers across the view at growing distances, depth tested, so the far world fades more than the near.
    if let Some(w) = weather {
        draw_fog_layers(&cam, weather::sky_tint(w).1);
    }
    set_default_camera();

    const TAG_RANGE_M: f32 = 25.0;
    // Names over the NPCs, placed where the figures are on screen, on a small dark tag so they read over any floor.
    // Far names are smaller and fade out (tag_style), so a crowd in the distance is not a wall of big labels.
    if settings::get().show_name_tags {
        for a in g.npcs.values().filter(|a| a.map == g.map) {
            let (x, y) = a.home;
            if let Some(Kind::Npc { name, .. }) = map.grid.at(x, y) {
                // Only a name the player could see: close enough, and no wall between the body and the NPC (the tag is
                // drawn over the 3D view, so without this it shows through buildings). The tag follows the walking NPC.
                let centre = a.pos;
                let (dx, dz) = (centre.0 - g.body.pos.0, centre.1 - g.body.pos.1);
                if dx * dx + dz * dz > TAG_RANGE_M * TAG_RANGE_M {
                    continue;
                }
                let walls = |tx: i32, ty: i32| matches!(map.grid.at(tx, ty), None | Some(Kind::Wall { .. }));
                if !rpg::line_clear(g.body.pos, centre, &walls) {
                    continue;
                }
                let world = vec3(centre.0, 1.8, centre.1);
                if let Some(p) = project(&cam, world, sw, sh) {
                    let (size, alpha) = tag_style((dx * dx + dz * dz).sqrt(), TAG_RANGE_M);
                    if alpha <= 0.01 {
                        continue;
                    }
                    let width = ui::measure(name, size);
                    let (pad, box_h) = (size * 0.45, size * 1.35);
                    draw_rectangle(p.x - width / 2.0 - pad, p.y - size * 0.95, width + pad * 2.0, box_h, Color::new(0.0, 0.0, 0.0, 0.5 * alpha));
                    ui::text_shadow(name, p.x - width / 2.0, p.y, size, ui::fade(ui::GOOD, alpha));
                }
            }
        }
    }
    if let Some(w) = weather {
        draw_weather(w, &cam, sw, sh, t);
    }
}

/// The most weather particles drawn in a frame.
const MAX_PARTICLES: usize = 1500;

/// A stable hash of a particle index and a channel to [0, 1): the same index always lands in the same place, so no
/// particle list is kept between frames.
fn particle_hash(i: u32, salt: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h % 100_000) as f32 / 100_000.0
}



/// Where particle `i` is at time `t`: inside a box of 28 m round the camera, 12 m high, drifting with the wind and falling
/// at `fall` m/s, wrapped so it stays where it is in the world while the camera moves.
fn particle_pos(i: u32, t: f32, cam: Vec3, wind: (f32, f32), fall: f32) -> Vec3 {
    const R: f32 = 14.0;
    const H: f32 = 12.0;
    let x0 = particle_hash(i, 1) * 2.0 * R;
    let z0 = particle_hash(i, 2) * 2.0 * R;
    let y0 = particle_hash(i, 3) * H;
    let x = cam.x - R + (x0 + wind.0 * t - cam.x + R).rem_euclid(2.0 * R);
    let z = cam.z - R + (z0 + wind.1 * t - cam.z + R).rem_euclid(2.0 * R);
    let y = (y0 - fall * t).rem_euclid(H);
    vec3(x, y, z)
}

/// A sky colour multiplied by the weather's tint.
fn tint_rgb(c: Rgb, tint: [f32; 3]) -> Rgb {
    let ch = |k: usize| (c[k] as f32 * tint[k]).round().clamp(0.0, 255.0) as u8;
    [ch(0), ch(1), ch(2)]
}

/// Haze across the view: translucent quads facing the camera at growing distances. They are depth tested, so a wall
/// nearer than a layer is not hazed by it and the far world picks up every layer. `density` is 0..1.
fn draw_fog_layers(cam: &Camera3D, density: f32) {
    if density <= 0.01 {
        return;
    }
    let dir = (cam.target - cam.position).normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }
    let right = dir.cross(cam.up).normalize_or_zero();
    let up = right.cross(dir);
    for d in [5.0f32, 10.0, 20.0, 40.0] {
        let c = cam.position + dir * d;
        let half = d * 2.0;
        let color = Color::new(0.72, 0.75, 0.80, (density * 0.22).clamp(0.0, 0.4));
        let corner = |sx: f32, sy: f32| {
            let p = c + right * (half * sx) + up * (half * sy);
            Vertex::new(p.x, p.y, p.z, 0.0, 0.0, color)
        };
        draw_mesh(&Mesh {
            vertices: vec![corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: None,
        });
    }
}

/// The weather over the finished 3D view, in screen space (call with the default camera set): a faint darkening of the
/// scene by the sky tint, the particles of the current kind and of the one it is changing to (rain streaks, snow
/// flakes, tumbling leaves), and a white flash for lightning. Particle positions come from `particle_pos`.
fn draw_weather(w: &weather::Weather, cam: &Camera3D, sw: f32, sh: f32, t: f32) {
    use model::WeatherKind as K;
    let (tint, _) = weather::sky_tint(w);
    let dark = (1.0 - (tint[0] + tint[1] + tint[2]) / 3.0).clamp(0.0, 1.0) * 0.45;
    if dark > 0.01 {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.04, dark));
    }
    let m = cam.matrix();
    let to_screen = |p: Vec3| -> Option<(Vec2, f32)> {
        let clip = m * p.extend(1.0);
        if clip.w <= 0.05 {
            return None;
        }
        let n = clip.truncate() / clip.w;
        if n.x.abs() > 1.05 || n.y.abs() > 1.05 {
            return None;
        }
        Some((vec2((n.x + 1.0) * 0.5 * sw, (1.0 - n.y) * 0.5 * sh), clip.w))
    };
    let k = sh / 720.0;
    // Each kind draws its own count (its level in the blend times its maximum), so a cross-fade fades each layer by its
    // own share and leaves are not cut to a fraction; the total stays under MAX_PARTICLES.
    let mut layers = weather::layers(w);
    let total: usize = layers.iter().map(|l| l.1).sum();
    if total > MAX_PARTICLES {
        for l in layers.iter_mut() {
            l.1 = l.1 * MAX_PARTICLES / total;
        }
    }
    for (layer, (kind, count)) in layers.iter().enumerate() {
        let base = layer as u32 * 100_000;
        for i in 0..*count as u32 {
            let idx = base + i;
            match kind {
                K::Rain | K::Storm => {
                    let fall = if *kind == K::Storm { 13.0 } else { 9.0 };
                    let p = particle_pos(idx, t, cam.position, w.wind, fall);
                    let tail = p + vec3(w.wind.0 * 0.03, 0.55, w.wind.1 * 0.03);
                    if let (Some((a, _)), Some((b, _))) = (to_screen(p), to_screen(tail)) {
                        draw_line(a.x, a.y, b.x, b.y, 1.0, Color::new(0.72, 0.78, 0.92, 0.32));
                    }
                }
                K::Snow => {
                    let mut p = particle_pos(idx, t, cam.position, w.wind, 1.2);
                    p.x += 0.3 * (t * 1.3 + i as f32).sin();
                    if let Some((a, depth)) = to_screen(p) {
                        let s = (2.4 * k * (6.0 / depth.max(1.0))).clamp(1.0, 4.0 * k.max(1.0));
                        draw_rectangle(a.x, a.y, s, s, Color::new(0.95, 0.97, 1.0, 0.8));
                    }
                }
                K::Leaves => {
                    let p = particle_pos(idx, t, cam.position, (w.wind.0 * 3.0, w.wind.1 * 3.0), 0.8);
                    if let Some((a, depth)) = to_screen(p) {
                        let s = (5.0 * k * (6.0 / depth.max(1.0))).clamp(2.0, 8.0 * k.max(1.0));
                        let shade = particle_hash(idx, 4);
                        let color = Color::new(0.55 + 0.35 * shade, 0.32 + 0.25 * shade, 0.08, 0.9);
                        let spin = t * (1.5 + 2.0 * shade) + i as f32;
                        draw_rectangle_ex(a.x, a.y, s, s * 0.6, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: spin, color });
                    }
                }
                K::Fog | K::Clear => {}
            }
        }
    }
    if w.flash > 0.01 {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.95, 0.97, 1.0, (w.flash * 0.7).clamp(0.0, 0.8)));
    }
}

/// The size and opacity of an NPC name tag at `dist` metres: full size (18 px) up to 4 m, then shrinking with distance
/// down to 10 px, and fading out over the last 40% of `range`.
fn tag_style(dist: f32, range: f32) -> (f32, f32) {
    let size = (18.0 * (4.0 / dist.max(0.1))).clamp(10.0, 18.0);
    let fade_from = range * 0.6;
    let alpha = if dist <= fade_from { 1.0 } else { (1.0 - (dist - fade_from) / (range - fade_from)).clamp(0.0, 1.0) };
    (size, alpha)
}

/// The wall texture of one wall style (see `scenery::wall_texture_kind`): the lab look is the flat `lab0`, the others
/// are painted from textures/walls.rs. Nearest filter, repeat wrap. None when the style does not paint.
fn wall_texture(style: &str) -> Option<Texture2D> {
    scenery::painted_texture(scenery::wall_texture_kind(style), style)
}

/// The wall textures of every style the pack's maps use, plus the lab one as the fallback; painted once, by style name.
fn wall_textures(pack: &Pack) -> HashMap<String, Texture2D> {
    let mut styles: BTreeSet<&str> = pack.maps.values().map(|m| m.wall_style.as_str()).collect();
    styles.insert("lab");
    let mut out = HashMap::new();
    for style in styles {
        if let Some(tex) = wall_texture(style) {
            out.insert(style.to_string(), tex);
        }
    }
    out
}

/// Repeat wrap, so v can run past 1 (props/prop_model.rs does the same for its textures).
fn repeat_wrap(tex: &Texture2D) {
    use macroquad::miniquad::TextureWrap;
    let id = tex.raw_miniquad_id();
    // SAFETY: only called from `run`, inside the window, where the GL context exists.
    unsafe {
        get_internal_gl().quad_context.texture_set_wrap(id, TextureWrap::Repeat, TextureWrap::Repeat);
    }
}

/// One wall tile as a box. With the texture: a mesh of the four sides and the top. u runs one repeat per metre (the
/// texture is 32 texels to the metre) and v comes from `textures::wall_v`, so each storey's cornice sits at the top.
/// The map colour is the vertex colour, so it tints the texture. Without a texture: the plain box in the map colour.
fn draw_wall(cx: f32, cz: f32, height: f32, tint: Color, tex: Option<&Texture2D>) {
    let Some(tex) = tex else {
        draw_cube(vec3(cx, height / 2.0, cz), vec3(TILE_M, height, TILE_M), None, tint);
        return;
    };
    draw_mesh(&wall_mesh(cx, cz, height, tint, tex));
}

/// The tiles just outside the map next to a walkable edge tile. Nothing is drawn outside the grid, so without a wall
/// there the player looks into the void (an exit on the edge of a room showed a black hole).
fn edge_wall_cells(map: &model::MapDef) -> Vec<(i32, i32)> {
    let (w, h) = (map.grid.w as i32, map.grid.h as i32);
    let open = |x: i32, y: i32| !matches!(map.grid.at(x, y), None | Some(Kind::Wall { .. }));
    let mut out = Vec::new();
    for x in 0..w {
        if open(x, 0) {
            out.push((x, -1));
        }
        if open(x, h - 1) {
            out.push((x, h));
        }
    }
    for y in 0..h {
        if open(0, y) {
            out.push((-1, y));
        }
        if open(w - 1, y) {
            out.push((w, y));
        }
    }
    out
}

/// The mesh of one wall tile (see `draw_wall`): 20 vertices and 30 indices.
fn wall_mesh(cx: f32, cz: f32, height: f32, tint: Color, tex: &Texture2D) -> Mesh {
    let half = TILE_M / 2.0;
    let corners = [(cx - half, cz - half), (cx + half, cz - half), (cx + half, cz + half), (cx - half, cz + half)];
    let v_bottom = textures::wall_v(0.0, 0.0, height);
    let v_top = textures::wall_v(height, 0.0, height);
    let mut vertices = Vec::with_capacity(20);
    let mut indices: Vec<u16> = Vec::with_capacity(30);
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let base = vertices.len() as u16;
        vertices.push(Vertex::new(a.0, 0.0, a.1, 0.0, v_bottom, tint));
        vertices.push(Vertex::new(b.0, 0.0, b.1, TILE_M, v_bottom, tint));
        vertices.push(Vertex::new(b.0, height, b.1, TILE_M, v_top, tint));
        vertices.push(Vertex::new(a.0, height, a.1, 0.0, v_top, tint));
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let base = vertices.len() as u16;
    for (x, z) in corners {
        vertices.push(Vertex::new(x, height, z, x - corners[0].0, v_top, tint));
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    Mesh {
        vertices,
        indices,
        texture: Some(tex.clone()),
    }
}

#[cfg(test)]
mod tests {
    /// Walkable tiles on the edge get a wall just outside the map; wall tiles on the edge do not.
    #[test]
    fn walkable_map_edges_are_closed_with_walls() {
        use std::collections::BTreeMap;
        let wall = Kind::Wall { height: 3.0, color: [1, 1, 1] };
        // 3x2: top row wall, floor, wall; bottom row all floor
        let cells = vec![wall.clone(), Kind::Floor, wall, Kind::Floor, Kind::Floor, Kind::Floor];
        let map = model::MapDef {
            id: "m".into(),
            name: "m".into(),
            art: std::path::PathBuf::new(),
            legend: BTreeMap::new(),
            grid: model::Grid { w: 3, h: 2, cells },
            spawns: BTreeMap::new(),
            spawn_facing: BTreeMap::new(),
            outdoor: false,
            ceiling_height: 3.0,
            ceiling_color: None,
            sky: None,
            wall_style: "lab".into(),
            floor_style: "blocks".into(),
            weather: None,
        };
        let edges = super::edge_wall_cells(&map);
        assert!(edges.contains(&(1, -1)), "the open top tile gets a wall above it");
        assert!(!edges.contains(&(0, -1)) && !edges.contains(&(2, -1)), "edge walls stay walls");
        for x in 0..3 {
            assert!(edges.contains(&(x, 2)), "the open bottom row is closed below");
        }
        assert!(edges.contains(&(-1, 1)) && edges.contains(&(3, 1)));
    }



    #[test]
    fn particles_stay_in_the_box_round_the_camera_and_fall_with_time() {
        use super::{particle_hash, particle_pos};
        let cam = vec3(40.0, 1.6, -7.0);
        for i in 0..200u32 {
            for t in [0.0f32, 3.7, 120.0] {
                let p = particle_pos(i, t, cam, (1.5, -0.5), 9.0);
                assert!((p.x - cam.x).abs() <= 14.01 && (p.z - cam.z).abs() <= 14.01, "{i} {t}: {p:?}");
                assert!((0.0..=12.0).contains(&p.y), "{i} {t}: {p:?}");
            }
            let h = particle_hash(i, 1);
            assert!((0.0..1.0).contains(&h));
        }
        assert_eq!(particle_pos(7, 2.0, cam, (0.0, 0.0), 9.0), particle_pos(7, 2.0, cam, (0.0, 0.0), 9.0), "stable");
        // Falling: lower a moment later (unless it wrapped to the top).
        let (a, b) = (particle_pos(3, 1.0, cam, (0.0, 0.0), 9.0), particle_pos(3, 1.01, cam, (0.0, 0.0), 9.0));
        assert!(b.y < a.y || b.y > 11.0);
        // The camera moving does not move the particle in the world while it stays inside the box.
        let moved = particle_pos(3, 1.0, cam + vec3(0.5, 0.0, 0.0), (0.0, 0.0), 9.0);
        assert!((moved.x - a.x).abs() < 1e-3 || (moved.x - a.x).abs() > 27.0);
    }

    #[test]
    fn the_sky_tint_multiplies_each_channel_and_clamps() {
        assert_eq!(super::tint_rgb([200, 100, 50], [1.0, 1.0, 1.0]), [200, 100, 50]);
        assert_eq!(super::tint_rgb([200, 100, 50], [0.5, 0.5, 0.5]), [100, 50, 25]);
        assert_eq!(super::tint_rgb([200, 100, 50], [2.0, 0.0, 1.0]), [255, 0, 50]);
    }

    #[test]
    fn a_seated_camera_sits_above_the_seat() {
        let eye = super::seated_eye((3.0, 4.0, 0.45));
        assert_eq!((eye.x, eye.z), (3.0, 4.0));
        assert!((eye.y - (0.45 + super::SEATED_EYE_M)).abs() < 1e-6);
        assert!(eye.y < crate::body::CAMERA_HEIGHT, "lower than standing");
    }

    /// Name tags: full size up close, smaller far away (never below 10 px), and gone at the edge of the range.
    #[test]
    fn name_tags_shrink_and_fade_with_distance() {
        assert_eq!(super::tag_style(2.0, 25.0), (18.0, 1.0));
        let (mid, a) = super::tag_style(8.0, 25.0);
        assert!(mid < 18.0 && mid >= 10.0 && a == 1.0, "{mid} {a}");
        let (far, a) = super::tag_style(20.0, 25.0);
        assert!(far == 10.0 && a > 0.0 && a < 1.0, "{far} {a}");
        assert_eq!(super::tag_style(25.0, 25.0).1, 0.0);
    }

    use super::*;

    #[test]
    fn the_look_step_scales_with_sensitivity_and_flips_when_inverted() {
        let s = vec2(0.2, 0.1);
        assert_eq!(super::look_step(s, 1.0, false), s);
        assert_eq!(super::look_step(s, 2.0, false), vec2(0.4, 0.2));
        let inv = super::look_step(s, 0.5, true);
        assert!((inv.x - 0.1).abs() < 1e-6 && (inv.y + 0.05).abs() < 1e-6, "{inv:?}");
    }

    #[test]
    fn the_fov_setting_is_clamped_to_the_offered_range() {
        assert!((super::fov_radians(78.0) - 78f32.to_radians()).abs() < 1e-6);
        assert!((super::fov_radians(10.0) - 60f32.to_radians()).abs() < 1e-6);
        assert!((super::fov_radians(400.0) - 100f32.to_radians()).abs() < 1e-6);
    }

    #[test]
    fn the_shake_moves_the_camera_without_turning_it() {
        let mk = || Camera3D {
            position: vec3(1.0, 1.6, 2.0),
            target: vec3(1.0, 1.6, 1.0),
            up: vec3(0.0, 1.0, 0.0),
            ..Default::default()
        };
        let cam = mk();
        let still = shake_camera(mk(), (0.0, 0.0));
        assert_eq!((still.position, still.target), (cam.position, cam.target));
        let moved = shake_camera(mk(), (0.03, 0.02));
        assert_ne!(moved.position, cam.position);
        let look_before = cam.target - cam.position;
        let look_after = moved.target - moved.position;
        assert!((look_before - look_after).length() < 1e-6, "the view direction is unchanged");
        assert!(((moved.position - cam.position).length() - (0.03f32 * 0.03 + 0.02 * 0.02).sqrt()).abs() < 1e-5);
    }

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

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn equals_forms_split_into_a_flag_and_its_value() {
        assert_eq!(normalize(&words(&["--pack=dir", "--check"])), words(&["--pack", "dir", "--check"]));
        assert_eq!(normalize(&words(&["--shot=a=b.png"])), words(&["--shot", "a=b.png"]), "only the first = splits");
        assert_eq!(normalize(&words(&["--shot="])), words(&["--shot", ""]));
        assert_eq!(normalize(&words(&["pack", "a=b"])), words(&["pack", "a=b"]), "plain words are kept");
    }

    #[test]
    fn a_subcommand_is_found_before_between_and_after_flags() {
        let before = parse_command(&words(&["check", "--pack", "p"])).unwrap();
        assert_eq!(before, (Some("check".to_string()), words(&["--pack", "p"])));
        let after = parse_command(&words(&["--pack", "p", "--mod", "m", "pack", "somedir"])).unwrap();
        assert_eq!(after, (Some("pack".to_string()), words(&["--pack", "p", "--mod", "m", "somedir"])));
        let between = parse_command(&normalize(&words(&["--pack=p", "init", "my_story", "--flat-textures"]))).unwrap();
        assert_eq!(between.0.as_deref(), Some("init"));
        assert_eq!(between.1, words(&["--pack", "p", "my_story", "--flat-textures"]));
        assert_eq!(parse_command(&words(&["--check"])).unwrap(), (None, words(&["--check"])));
        assert_eq!(parse_command(&[]).unwrap(), (None, Vec::new()));
    }

    #[test]
    fn a_flag_value_that_looks_like_a_command_is_not_one() {
        // "check" here is the pack folder's name, not the subcommand
        assert_eq!(parse_command(&words(&["--pack", "check"])).unwrap(), (None, words(&["--pack", "check"])));
    }

    #[test]
    fn bad_command_lines_are_usage_errors() {
        let e = parse_command(&words(&["--bogus"])).unwrap_err();
        assert!(e.contains("unknown option '--bogus'"), "{e}");
        let e = parse_command(&words(&["--pack"])).unwrap_err();
        assert!(e.contains("--pack needs a value"), "{e}");
        let e = parse_command(&words(&["--shot", "--check"])).unwrap_err();
        assert!(e.contains("--shot needs a value"), "a flag cannot be another flag's value: {e}");
        let e = parse_command(&normalize(&words(&["--story="]))).unwrap_err();
        assert!(e.contains("--story needs a value"), "an empty value is none: {e}");
        let e = parse_command(&words(&["frobnicate"])).unwrap_err();
        assert!(e.contains("unknown command 'frobnicate'"), "{e}");
        assert!(cmd_check(&words(&["extra"])) == 2, "check takes no words");
        assert!(cmd_check(&words(&["--bogus"])) == 2);
    }

    #[test]
    fn a_saved_party_is_put_back_with_hp_only() {
        let mut p = rpg::Party::new(model::Stats::default());
        let base = p.stats.clone();
        restore_hp(&mut p, 12, 40);
        assert_eq!((p.hp, p.max_hp), (12, 40));
        assert_eq!(p.stats.max_hp, 40);
        assert_eq!((p.stats.attack, p.stats.defense), (base.attack, base.defense), "no level stats are added back");
        assert_eq!(party_key(&p), (12, 40));
        restore_hp(&mut p, 99, 20);
        assert_eq!(p.hp, 20, "hp stays within the maximum");
        restore_hp(&mut p, 0, 0);
        assert_eq!((p.hp, p.max_hp), (1, 1), "hp 0 is never restored");
    }

    #[test]
    fn a_picture_path_needs_an_existing_folder() {
        let root = scratch("shotpath");
        assert!(check_shot_path("--shot", &root.join("a.png")).is_ok());
        assert!(check_shot_path("--shot", Path::new("a.png")).is_ok(), "the working folder always exists");
        let e = check_shot_path("--shot", &root.join("nope").join("a.png")).unwrap_err();
        assert!(e.contains("--shot") && e.contains("does not exist"), "{e}");
        let e = check_shot_path("--menu-shot", &root).unwrap_err();
        assert!(e.contains("--menu-shot") && e.contains("is a folder"), "{e}");
        let _ = fs::remove_dir_all(&root);
    }
}
