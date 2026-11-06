//! The MODS tab of the browser build: IMPORT FOLDER and IMPORT .ZIP, the installed mods with REMOVE, and what the last
//! import said. The browser has no mods folder, so the player picks a pack folder or `.zip` files in the page's file chooser
//! (`shared/file_pick.rs`); `content/pack_import.rs` checks them with the pack rules, keeps them in the page's storage and
//! loads them at the next page start, which is why every import and removal says "reload the page".
//!
//! Native builds keep the folder-based tab drawn by `settings_panel.rs`; this tab is drawn on wasm32 only (its logic is
//! plain and tested everywhere).

use super::menus::{wheel_notch, Ui};
use super::style::*;
use crate::app::assets::Assets;
use crate::content::pack_import::{self, browser::Browser, Meta, Persist, Registry};
use crate::content::Content;
use crate::shared::file_pick::{self, PickedFile, State};
use macroquad::prelude::*;
use std::cell::RefCell;

/// The HTML `accept` string of IMPORT .ZIP.
const ZIP_KIND: &str = ".zip,application/zip";

pub const FOLDER_LABEL: &str = "IMPORT FOLDER";
pub const ZIP_LABEL: &str = "IMPORT .ZIP";

/// The notes under the list (muted).
const KEPT_NOTE: &str = "Imported mods are kept in this browser (IndexedDB, up to 256 MB in all) and load when the page starts: reload the page after an import or a remove.";
const HOW_NOTE: &str = "FOLDER: pick the pack folder (pack.json, hunters/ ...). .ZIP: pick one or more .zip files, each a pack.";
const NO_STORAGE_NOTE: &str = "This browser is not allowing site storage, so mods cannot be imported here.";
const SAVE_FAILED_NOTE: &str = "The last save to browser storage failed (it may be full). Remove a mod and try again.";

/// What the tab remembers between frames.
#[derive(Default)]
struct ModsUi {
    /// The chooser that is open, if any.
    picking: Option<file_pick::Request>,
    /// What the last action said: whether it went well, and the words.
    lines: Vec<(bool, String)>,
}

thread_local! {
    static MODS: RefCell<ModsUi> = RefCell::new(ModsUi::default());
}

/// The tab is not showing: a chooser still open is let go of, so a hidden tab never takes a pick later.
pub fn blur() {
    MODS.with(|c| {
        if let Ok(mut s) = c.try_borrow_mut() {
            if let Some(req) = s.picking.take() {
                file_pick::release(req);
            }
        }
    });
}

// ------------------------------------------------------------------------------------------- picking

/// The files of a done chooser, as the import wants them. Only files a pack holds (and zips) are looked at, and only those
/// are read, so a chosen folder with a movie in it costs nothing; a folder that is far too big is refused before any file
/// is read. The four closures answer for the chooser (the page, or a test).
pub fn gather_with(
    count: usize,
    name: &dyn Fn(usize) -> String,
    size: &dyn Fn(usize) -> Option<usize>,
    read: &dyn Fn(usize, usize) -> Result<Vec<u8>, String>,
) -> Result<Vec<PickedFile>, String> {
    if count > pack_import::MAX_LISTED {
        return Err(format!(
            "{count} files were chosen and at most {} are looked at. Choose the pack folder itself, not a folder of folders.",
            pack_import::MAX_LISTED
        ));
    }
    let wanted: Vec<(usize, String)> = (0..count).map(|i| (i, name(i))).filter(|(_, n)| pack_import::is_wanted_name(n)).collect();
    let total: usize = wanted.iter().map(|(i, _)| size(*i).unwrap_or(0)).sum();
    if total > pack_import::MAX_TOTAL_BYTES {
        return Err(format!(
            "the chosen files add up to {} MB and at most {} MB are taken. Choose just the pack folder.",
            total / (1024 * 1024),
            pack_import::MAX_TOTAL_BYTES / (1024 * 1024)
        ));
    }
    if wanted.is_empty() {
        return Err("none of the chosen files is something a mod pack holds (pack.json, .json, .bbmodel, .png, .ogg, .glb, or a .zip).".to_string());
    }
    Ok(wanted
        .into_iter()
        .map(|(i, n)| {
            let cap = if n.to_ascii_lowercase().ends_with(".zip") { pack_import::MAX_PACK_BYTES } else { pack_import::MAX_FILE_BYTES };
            let bytes = match size(i) {
                Some(s) if s > cap => Err("the file is too big".to_string()),
                _ => read(i, cap),
            };
            PickedFile { name: n, bytes }
        })
        .collect())
}

fn gather(req: file_pick::Request) -> Result<Vec<PickedFile>, String> {
    gather_with(
        file_pick::count(req),
        &|i| file_pick::name(req, i),
        &|i| file_pick::size(req, i),
        &|i, cap| file_pick::read(req, i, cap),
    )
}

/// IMPORT FOLDER / IMPORT .ZIP: opens the browser's chooser. A chooser still open from an earlier press is let go of first.
fn start(s: &mut ModsUi, zip: bool) {
    if let Some(old) = s.picking.take() {
        file_pick::release(old);
    }
    let (kind, multiple, directory) = if zip { (ZIP_KIND, true, false) } else { ("", false, true) };
    match file_pick::open(kind, multiple, directory) {
        Ok(req) => {
            s.picking = Some(req);
            s.lines = vec![(true, if zip { "Choose one or more .zip files in the file window." } else { "Choose the pack folder in the file window." }.to_string())];
        }
        Err(why) => s.lines = vec![(false, why)],
    }
}

/// The chooser's answer, once it has one. A closed chooser changes nothing.
fn poll(s: &mut ModsUi) {
    let Some(req) = s.picking else { return };
    match file_pick::state(req) {
        State::Pending => {}
        State::Cancelled => {
            file_pick::release(req);
            s.picking = None;
            s.lines.clear();
        }
        State::Done => {
            let gathered = gather(req);
            file_pick::release(req);
            s.picking = None;
            pack_import::with_registry(|reg| finish(s, gathered, reg, &mut Browser));
        }
    }
}

/// What a finished pick does: the files are checked and kept, and the tab says how each pack went.
fn finish(s: &mut ModsUi, gathered: Result<Vec<PickedFile>, String>, reg: &mut Registry, store: &mut dyn Persist) {
    s.lines = match gathered {
        Err(why) => vec![(false, format!("Not imported: {why}"))],
        Ok(files) => {
            let report = reg.accept(pack_import::candidates(files), store);
            let lines = report.lines();
            if lines.is_empty() {
                vec![(false, "Nothing was imported.".to_string())]
            } else {
                lines
            }
        }
    };
}

/// REMOVE: forgets the pack and its stored copy; the game drops it at the next page start.
fn remove(s: &mut ModsUi, reg: &mut Registry, store: &mut dyn Persist, id: &str, loaded: bool) {
    s.lines = match reg.remove(id, store) {
        Some(name) if loaded => vec![(true, format!("Removed {name}. Reload the page to drop it from the game."))],
        Some(name) => vec![(true, format!("Removed {name}."))],
        None => vec![(false, "That mod is already gone.".to_string())],
    };
}

// ------------------------------------------------------------------------------------------- the list

/// How a pack in the list stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Loaded from a mods folder (a desktop build, or a pack that came with the page): not removable here.
    Folder,
    /// Imported and in the game now.
    InGame,
    /// Imported in this session: the game loads it at the next page start.
    WaitsForReload,
    /// Removed, but still in the game until the page is reloaded.
    GoneAfterReload,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModRow {
    pub id: String,
    pub title: String,
    pub adds: String,
    pub kind: RowKind,
}

impl ModRow {
    pub fn removable(&self) -> bool {
        matches!(self.kind, RowKind::InGame | RowKind::WaitsForReload)
    }

    fn note(&self) -> &'static str {
        match self.kind {
            RowKind::WaitsForReload => " - LOADS AFTER RELOAD",
            RowKind::GoneAfterReload => " - GONE AFTER RELOAD",
            _ => "",
        }
    }
}

fn title_of(name: &str, version: &str, author: &str) -> String {
    format!("{} {} // {}", name.to_uppercase(), version, author)
}

/// The installed mods: every pack the game has loaded, then the imports it has not loaded yet. `imported` is what this
/// browser holds (id, pack.json fields, what it adds).
pub fn mod_rows(content: &Content, imported: &[(String, Meta, Vec<String>)]) -> Vec<ModRow> {
    let mut rows = Vec::new();
    for pk in &content.packs {
        let adds = format!("ADDS: {}", pk.provides.join(", "));
        let title = title_of(&pk.name, &pk.version, &pk.author);
        match pack_import::virtual_id(&pk.dir) {
            None => rows.push(ModRow { id: pk.id.clone(), title, adds, kind: RowKind::Folder }),
            Some(id) => {
                let kept = imported.iter().any(|(i, _, _)| i == id);
                rows.push(ModRow { id: id.to_string(), title, adds, kind: if kept { RowKind::InGame } else { RowKind::GoneAfterReload } });
            }
        }
    }
    let loaded = pack_import::loaded_ids(content);
    for (id, meta, provides) in imported {
        if !loaded.contains(id) {
            rows.push(ModRow {
                id: id.clone(),
                title: title_of(&meta.name, &meta.version, &meta.author),
                adds: format!("ADDS: {}", provides.join(", ")),
                kind: RowKind::WaitsForReload,
            });
        }
    }
    rows
}

// ------------------------------------------------------------------------------------------- drawing

/// One thing in the scrolling list.
enum Item {
    Text { text: String, size: f32, col: Color, dx: f32, step: f32 },
    Gap(f32),
    Pack(ModRow),
}

const PACK_ADDS_SIZE: f32 = 6.0;
const REMOVE_W: f32 = 46.0;
const ROW_H: f32 = 14.0;

fn text_item(text: String, size: f32, col: Color, dx: f32) -> Item {
    Item::Text { text, size, col, dx, step: lh(F::Body, size) }
}

fn item_height(it: &Item) -> f32 {
    match it {
        Item::Text { step, .. } => *step,
        Item::Gap(h) => *h,
        Item::Pack(_) => ROW_H + lh(F::Body, PACK_ADDS_SIZE) + 4.0,
    }
}

fn items(a: &Assets, s: &ModsUi, rows: Vec<ModRow>, store: &dyn Persist) -> Vec<Item> {
    let p = pal();
    let mut out = Vec::new();
    for (good, text) in &s.lines {
        for l in wrap(a, F::Body, text, 400.0, 7.0) {
            out.push(text_item(l, 7.0, if *good { p.teal_bright } else { p.alert }, 0.0));
        }
    }
    if !store.available() {
        for l in wrap(a, F::Body, NO_STORAGE_NOTE, 400.0, 7.0) {
            out.push(text_item(l, 7.0, p.alert, 0.0));
        }
    } else if store.save_failed() {
        for l in wrap(a, F::Body, SAVE_FAILED_NOTE, 400.0, 7.0) {
            out.push(text_item(l, 7.0, p.alert, 0.0));
        }
    }
    if !out.is_empty() {
        out.push(Item::Gap(4.0));
    }
    out.push(Item::Text { text: track("INSTALLED MODS"), size: 8.0, col: p.amber, dx: 0.0, step: lh(F::Body, 8.0) + 4.0 });
    if rows.is_empty() {
        out.push(text_item("NO MODS INSTALLED".into(), 7.0, p.muted, 0.0));
    }
    out.extend(rows.into_iter().map(Item::Pack));
    out.push(Item::Gap(2.0));
    for note in [KEPT_NOTE, HOW_NOTE] {
        for l in wrap(a, F::Body, note, 400.0, 6.0) {
            out.push(text_item(l, 6.0, p.muted, 0.0));
        }
        out.push(Item::Gap(3.0));
    }
    out
}

/// What this browser holds, for the list.
fn held() -> Vec<(String, Meta, Vec<String>)> {
    pack_import::with_registry(|r| r.all().iter().map(|p| (p.id.clone(), p.meta(), p.provides.clone())).collect())
}

/// Draws the tab at the panel corner (`ox`, `oy`) and handles its input.
pub fn draw(a: &Assets, ui: &mut Ui, ox: f32, oy: f32, content: &Content) {
    let p = pal();
    let x = ox + 20.0;
    MODS.with(|cell| {
        let mut s = cell.borrow_mut();
        let before = s.lines.clone();
        poll(&mut s);

        let half = (400.0 - 4.0) * 0.5;
        if button(a, Rect::new(x, oy + 66.0, half, 16.0), FOLDER_LABEL, 7.0, Kind::Secondary, plain()) {
            start(&mut s, false);
        }
        if button(a, Rect::new(x + half + 4.0, oy + 66.0, half, 16.0), ZIP_LABEL, 7.0, Kind::Secondary, plain()) {
            start(&mut s, true);
        }

        let store = Browser;
        let rows = mod_rows(content, &held());
        let list = items(a, &s, rows, &store);
        let area = Rect::new(x, oy + 88.0, 400.0, 138.0);
        let total: f32 = list.iter().map(item_height).sum();
        if s.lines != before {
            ui.scroll = 0.0;
        }
        if hovered(area) {
            ui.scroll -= wheel_notch(mouse_wheel().1, 20.0);
        }
        ui.scroll = ui.scroll.clamp(0.0, (total - area.h).max(0.0));

        let mut remove_id: Option<(String, bool)> = None;
        clip_begin(area);
        let mut y = area.y - ui.scroll;
        for it in &list {
            match it {
                Item::Text { text, size, col, dx, step } => {
                    if !text.is_empty() {
                        label(a, F::Body, text, x + dx, y, *size, *col);
                    }
                    y += step;
                }
                Item::Gap(h) => y += h,
                Item::Pack(row) => {
                    let removable = row.removable();
                    let title_w = if removable { 400.0 - REMOVE_W - 6.0 } else { 400.0 };
                    let title = format!("{}{}", row.title, row.note());
                    label_vc(a, F::Body, &clip(a, F::Body, &title, title_w, 7.0), x, y, ROW_H, 7.0, if row.kind == RowKind::GoneAfterReload { p.muted } else { p.ink });
                    if removable {
                        let r = Rect::new(x + 400.0 - REMOVE_W, y, REMOVE_W, ROW_H);
                        // a button only when it lies fully inside the list, so a half-hidden row cannot be pressed from outside it
                        let inside = r.y >= area.y && r.y + r.h <= area.y + area.h;
                        if inside {
                            if button(a, r, "REMOVE", 6.0, Kind::Secondary, plain()) {
                                remove_id = Some((row.id.clone(), row.kind == RowKind::InGame));
                            }
                        } else {
                            sbox(r, p.panel, p.panel_edge, 1.0, p.corner);
                        }
                    }
                    let adds = clip(a, F::Body, &row.adds, 400.0 - 6.0, PACK_ADDS_SIZE);
                    label(a, F::Body, &adds, x + 6.0, y + ROW_H, PACK_ADDS_SIZE, p.body);
                    y += item_height(it);
                }
            }
        }
        clip_end();

        if let Some((id, loaded)) = remove_id {
            pack_import::with_registry(|reg| remove(&mut s, reg, &mut Browser, &id, loaded));
            ui.scroll = 0.0;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::pack_import::testing::Memory;

    fn pack_json(id: &str, name: &str) -> Vec<u8> {
        format!(r#"{{"schema":"lr.pack/1","id":"{id}","name":"{name}","version":"1.2","author":"KT"}}"#).into_bytes()
    }

    fn hunter_json(id: &str) -> Vec<u8> {
        format!(
            r#"{{"schema":"lr.hunter/1","id":"{id}","name":"{id}","tier":"intermediate","category":"az","speed":0.9,"chase":3.7,"prog":0.2,"sight":7.0,"hearing":1.3,"spawn":14,"chase_style":"pump","taser_immune":false,"melee_immune":false}}"#
        )
        .into_bytes()
    }

    fn folder(id: &str, name: &str, hunter: &str) -> Vec<PickedFile> {
        vec![
            PickedFile { name: format!("{id}/pack.json"), bytes: Ok(pack_json(id, name)) },
            PickedFile { name: format!("{id}/hunters/h.json"), bytes: Ok(hunter_json(hunter)) },
        ]
    }

    #[test]
    fn a_finished_pick_says_what_was_imported_and_what_was_refused() {
        let (mut reg, mut store, mut s) = (Registry::default(), Memory::default(), ModsUi::default());
        let mut files = folder("tab_pack", "Tab Pack", "tabling");
        files.push(PickedFile { name: "tab_pack/assets/x.png".into(), bytes: Ok(b"not a png".to_vec()) });
        finish(&mut s, Ok(files), &mut reg, &mut store);
        assert_eq!(s.lines.len(), 1);
        assert!(!s.lines[0].0 && s.lines[0].1.contains("assets/x.png: not a PNG"), "a bad pack is refused with its reason: {:?}", s.lines);
        assert!(reg.is_empty());

        finish(&mut s, Ok(folder("tab_pack", "Tab Pack", "tabling")), &mut reg, &mut store);
        assert_eq!(s.lines[0], (true, "Imported Tab Pack 1.2 (adds 1). Reload the page to use it.".to_string()));
        assert_eq!(reg.ids(), ["tab_pack"]);

        finish(&mut s, Err("the chosen files add up to 999 MB".into()), &mut reg, &mut store);
        assert_eq!(s.lines, [(false, "Not imported: the chosen files add up to 999 MB".to_string())]);
        finish(&mut s, Ok(vec![]), &mut reg, &mut store);
        assert_eq!(s.lines, [(false, "Nothing was imported.".to_string())]);
    }

    #[test]
    fn remove_forgets_the_pack_and_says_when_to_reload() {
        let (mut reg, mut store, mut s) = (Registry::default(), Memory::default(), ModsUi::default());
        finish(&mut s, Ok(folder("tab_pack", "Tab Pack", "tabling")), &mut reg, &mut store);
        remove(&mut s, &mut reg, &mut store, "tab_pack", true);
        assert_eq!(s.lines, [(true, "Removed Tab Pack. Reload the page to drop it from the game.".to_string())]);
        assert!(reg.is_empty() && store.kept.is_empty());
        remove(&mut s, &mut reg, &mut store, "tab_pack", true);
        assert_eq!(s.lines, [(false, "That mod is already gone.".to_string())]);
        finish(&mut s, Ok(folder("tab_pack", "Tab Pack", "tabling")), &mut reg, &mut store);
        remove(&mut s, &mut reg, &mut store, "tab_pack", false);
        assert_eq!(s.lines, [(true, "Removed Tab Pack.".to_string())], "a pack the game never loaded needs no reload");
    }

    /// The list shows folder packs, imported packs in the game, imports waiting for a reload, and removals waiting for one.
    #[test]
    fn the_list_marks_what_a_reload_will_change() {
        use crate::content::packs::Pack;
        use std::path::PathBuf;
        let pack = |id: &str, dir: PathBuf| Pack {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            author: "KT".into(),
            description: String::new(),
            dir,
            provides: vec!["a".into(), "prop:b".into()],
        };
        let mut c = Content::base();
        c.packs = vec![
            pack("from_folder", PathBuf::from("mods/from_folder")),
            pack("in_game", pack_import::virtual_dir("in_game")),
            pack("dropped", pack_import::virtual_dir("dropped")),
        ];
        let meta = |name: &str| Meta { name: name.into(), version: "2".into(), author: "Me".into() };
        let imported = vec![
            ("in_game".to_string(), meta("In Game"), vec!["a".to_string()]),
            ("fresh".to_string(), meta("Fresh Mod"), vec!["hunter_x".to_string(), "prop:y".to_string()]),
        ];
        let rows = mod_rows(&c, &imported);
        let kinds: Vec<(&str, RowKind)> = rows.iter().map(|r| (r.id.as_str(), r.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("from_folder", RowKind::Folder),
                ("in_game", RowKind::InGame),
                ("dropped", RowKind::GoneAfterReload),
                ("fresh", RowKind::WaitsForReload)
            ]
        );
        assert_eq!(rows.iter().map(ModRow::removable).collect::<Vec<_>>(), [false, true, false, true]);
        assert_eq!(rows[0].adds, "ADDS: a, prop:b");
        assert_eq!(rows[3].title, "FRESH MOD 2 // Me");
        assert_eq!(rows[3].note(), " - LOADS AFTER RELOAD");
        assert_eq!(rows[2].note(), " - GONE AFTER RELOAD");
    }

    #[test]
    fn nothing_is_installed_when_nothing_is_loaded_or_held() {
        assert!(mod_rows(&Content::base(), &[]).is_empty());
    }

    #[test]
    fn only_files_a_pack_holds_are_read_from_a_chosen_folder() {
        let names = ["p/pack.json", "p/README.md", "p/movie.mp4", "p/hunters/a.json", "p/.DS_Store", "p/assets/big.png"];
        let sizes = [10usize, 5, 900_000_000, 20, 1, pack_import::MAX_FILE_BYTES + 1];
        let read_count = std::cell::Cell::new(0);
        let files = gather_with(
            names.len(),
            &|i| names[i].to_string(),
            &|i| Some(sizes[i]),
            &|i, _| {
                read_count.set(read_count.get() + 1);
                Ok(names[i].as_bytes().to_vec())
            },
        )
        .unwrap();
        assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["p/pack.json", "p/hunters/a.json", "p/assets/big.png"]);
        assert_eq!(read_count.get(), 2, "the movie and the readme were never read; the oversize picture was refused without a read");
        assert_eq!(files[2].bytes, Err("the file is too big".to_string()));
        assert_eq!(files[0].bytes, Ok(b"p/pack.json".to_vec()));
    }

    #[test]
    fn a_pick_that_is_too_big_or_has_nothing_in_it_is_refused_before_reading() {
        let reads = std::cell::Cell::new(0);
        let read = |_: usize, _: usize| {
            reads.set(reads.get() + 1);
            Ok(vec![])
        };
        let err = gather_with(1, &|_| "p/a.json".into(), &|_| Some(pack_import::MAX_TOTAL_BYTES + 1), &read).unwrap_err();
        assert!(err.contains("add up to 256 MB") || err.contains("at most 256 MB"), "{err}");
        let err = gather_with(pack_import::MAX_LISTED + 1, &|_| "p/a.json".into(), &|_| Some(1), &read).unwrap_err();
        assert!(err.contains("Choose the pack folder itself"), "{err}");
        let err = gather_with(2, &|i| ["notes.txt", "photo.jpg"][i].into(), &|_| Some(1), &read).unwrap_err();
        assert!(err.contains("none of the chosen files"), "{err}");
        assert_eq!(reads.get(), 0);
        // a zip may be as big as a whole pack, a plain file only as big as one file
        let files = gather_with(
            2,
            &|i| ["a.zip", "assets/b.png"][i].into(),
            &|i| Some([pack_import::MAX_FILE_BYTES + 5, pack_import::MAX_FILE_BYTES + 5][i]),
            &|_, _| Ok(vec![1]),
        )
        .unwrap();
        assert_eq!(files[0].bytes, Ok(vec![1]));
        assert!(files[1].bytes.is_err());
    }

    #[test]
    fn a_pick_is_imported_from_a_zip_through_the_tab() {
        // the zip is the tab's input for IMPORT .ZIP; the unpacking itself is tested in content/pack_import.rs
        let (mut reg, mut store, mut s) = (Registry::default(), Memory::default(), ModsUi::default());
        let files = vec![PickedFile { name: "bad.zip".into(), bytes: Ok(b"PK not really".to_vec()) }];
        finish(&mut s, Ok(files), &mut reg, &mut store);
        assert_eq!(s.lines.len(), 1);
        assert!(!s.lines[0].0 && s.lines[0].1.starts_with("Not imported, bad.zip: the .zip is damaged"), "{:?}", s.lines);
    }

    #[test]
    fn a_blocked_browser_store_is_said_before_the_import_fails() {
        let (mut reg, mut store, mut s) = (Registry::default(), Memory { blocked: true, ..Default::default() }, ModsUi::default());
        finish(&mut s, Ok(folder("tab_pack", "Tab Pack", "tabling")), &mut reg, &mut store);
        assert!(!s.lines[0].0 && s.lines[0].1.contains("not allowing site storage"), "{:?}", s.lines);
        assert!(NO_STORAGE_NOTE.contains("not allowing site storage"));
    }
}
