//! Tests for the Simulora pilot pack (mods/simulora): the two side chapters imported from KyttusLab-LORE, placeholder
//! rooms, a pencil, and one pilot choice at the end of the guarded door. The pack tests skip when the LORE folder or the
//! pilot folder is missing. The override tests build their own small packs in a temp folder.

use crate::load::{self, Config};
use crate::model::Pack;
use crate::play::Game;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const PILOT_SCENE: &str = "simulora_simulora_side_the_guarded_door_ch3";
const PENCIL: &str = "simulora_pencil_stub";

/// The pilot pack folder, or None when the LORE repo or the pilot pack is not there (the test is then skipped).
fn pilot_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join("..").join("KyttusLab-LORE").is_dir() {
        return None;
    }
    let pack = root.join("mods").join("simulora");
    pack.join("pack.json").is_file().then_some(pack)
}

/// The sample core pack with one mod folder added, loaded and checked.
fn load_with_mod(mod_dir: &Path) -> Result<Pack, Vec<String>> {
    let cfg = Config {
        core: crate::pack_dir(),
        mod_dirs: vec![mod_dir.to_path_buf()],
        mod_roots: Vec::new(),
    };
    load::load(&cfg)
}

/// A fresh game on the pilot story (its start, its player, no saved flags).
fn pilot_game(pack: Pack) -> Game {
    Game::for_story(pack, "simulora", BTreeSet::new()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_pilot_pack_plays_linda_from_the_reading_room_to_the_wash() {
    let Some(dir) = pilot_dir() else { return };
    let pack = load_with_mod(&dir).unwrap_or_else(|errs| panic!("the pilot pack has problems: {errs:?}"));
    let story = pack.story("simulora").expect("the pilot pack is the simulora story").clone();
    assert_eq!(story.player.name, "Linda");
    assert_eq!(story.start_map, "simulora_reading_room");

    let mut g = pilot_game(pack);
    assert_eq!(g.player.name, "Linda");
    assert_eq!(g.map, "simulora_reading_room");
    assert_eq!(g.pos, (2, 3), "the start spawn");

    // Nine steps east along row three: the exit on the east wall takes the player to the wash.
    for _ in 0..9 {
        g.try_move(1, 0);
    }
    assert_eq!(g.map, "simulora_bluebell_wash");
    assert_eq!(g.pos, (1, 3), "arrives on the wash's from_reading_room spawn");

    // The door stays sealed: its flag is never set, so bumping it shows the locked text and does not open.
    for _ in 0..7 {
        g.try_move(1, 0);
    }
    assert_eq!(g.pos, (8, 3));
    g.try_move(1, 0);
    assert_eq!(g.pos, (8, 3), "the door blocks the way");
    assert!(!g.flags.contains("heard-the-door-open"));
    assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Sealed. Intact."));

    // The pencil: four steps back west, two south, onto the item tile. It is picked up once.
    for _ in 0..4 {
        g.try_move(-1, 0);
    }
    g.try_move(0, 1);
    g.try_move(0, 1);
    assert_eq!(g.pos, (4, 5));
    assert_eq!(g.inv, vec![PENCIL.to_string()]);
    assert_eq!(g.toast.as_ref().map(|t| t.0.as_str()), Some("Picked up Pencil stub"));
    g.try_move(0, -1);
    g.try_move(0, 1);
    assert_eq!(g.inv, vec![PENCIL.to_string()], "stepping off and back on does not pick it up again");
}

#[test]
fn the_pilot_choice_on_chapter_three_sets_its_flag_and_ends_the_scene() {
    let Some(dir) = pilot_dir() else { return };
    let pack = load_with_mod(&dir).unwrap_or_else(|errs| panic!("the pilot pack has problems: {errs:?}"));
    assert!(pack.scenes.contains_key(PILOT_SCENE));
    let mut g = pilot_game(pack);
    g.start_scene(PILOT_SCENE);
    // Read through the lines until the choices come up (each advance either finishes a line or moves to the next).
    for _ in 0..100_000 {
        if g.talk.as_ref().map_or(true, |t| t.choosing) {
            break;
        }
        g.advance();
    }
    assert_eq!(g.offered_choices(), vec!["Log the pause in pencil", "Report it to Corran"]);
    g.choose(0);
    assert!(g.flags.contains("pencil-logged"), "the first choice sets pencil-logged");
    assert!(!g.flags.contains("reported-pause"));
    assert!(g.talk.is_none(), "the choice has no next scene, so the scene ends");
}

/// Writes a mod folder: pack.json (with `extra` appended to its fields, when given) and the given scene files.
fn write_mod(dir: &Path, extra: &str, files: &[(&str, &str)]) {
    fs::create_dir_all(dir.join("scenes")).unwrap();
    let comma = if extra.is_empty() { "" } else { "," };
    fs::write(dir.join("pack.json"), format!(r#"{{"schema": "lr.pack/1", "id": "ovr_test"{comma}{extra}}}"#)).unwrap();
    for (name, text) in files {
        fs::write(dir.join("scenes").join(name), text).unwrap();
    }
}

#[test]
fn a_scene_repeated_in_one_pack_is_replaced_only_when_listed_and_in_an_overrides_file() {
    let root = std::env::temp_dir().join(format!("lr_story_pilot_overrides_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let scene = |text: &str| {
        format!(r#"{{"schema": "lr.story.scenes/1", "scenes": {{"ovr_test_scene": {{"lines": [{{"text": "{text}"}}]}}}}}}"#)
    };
    let listed = r#""overrides": ["scenes/ovr_test_scene"]"#;

    // Listed, and the second copy is in a file named "overrides": the second copy replaces the first.
    let good = root.join("good").join("ovr_test");
    write_mod(&good, listed, &[("a.json", &scene("first")), ("b_overrides.json", &scene("second"))]);
    let pack = load_with_mod(&good).unwrap_or_else(|errs| panic!("{errs:?}"));
    assert_eq!(pack.scenes["ovr_test_scene"].lines[0].text, "second");

    // Not listed: a repeat is an error, whatever the file names.
    let unlisted = root.join("unlisted").join("ovr_test");
    write_mod(&unlisted, "", &[("a.json", &scene("first")), ("b_overrides.json", &scene("second"))]);
    let errs = load_with_mod(&unlisted).err().expect("an unlisted repeat is an error");
    assert!(errs.iter().any(|e| e.contains("defined more than once")), "{errs:?}");

    // Listed, but the second copy is in an ordinary file: still an error.
    let ordinary = root.join("ordinary").join("ovr_test");
    write_mod(&ordinary, listed, &[("a.json", &scene("first")), ("b.json", &scene("second"))]);
    let errs = load_with_mod(&ordinary).err().expect("a repeat outside an overrides file is an error");
    assert!(errs.iter().any(|e| e.contains("defined more than once")), "{errs:?}");

    let _ = fs::remove_dir_all(&root);
}
