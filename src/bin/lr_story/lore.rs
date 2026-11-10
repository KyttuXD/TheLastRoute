//! Imports KyttusLab-LORE story files (stories/**/*.yml) into a pack. Each `Story` chapter becomes one scene, and the
//! chapters of a file are chained in order with `next`. A `???` speaker is narration. The chapter text is copied as
//! written; nothing is added or rewritten.
//!
//! A file looks like this (the top-level name varies):
//!
//! ```text
//! The Last Route Storyline:
//!   Story:
//!     Chapter 1:
//!       Title: Prep
//!       Summary:
//!         - ???: The lights hum.
//!         - Felix: '"Night shift snacks."'
//! ```
//!
//! An optional `Choices:` list in a chapter makes the chapter's scene offer choices (the scene then has no `next`; each
//! choice carries one). Each entry has `Text`, optional `Needs` (flags that must all be set for it to show), `Sets`
//! (flags it sets) and `Goto` (`Chapter N` of the same file). Without `Goto` a choice goes to the next chapter, and the
//! last chapter's choice ends the scene.
//!
//! ```text
//!     Chapter 1:
//!       Summary:
//!         - ???: The corridor splits.
//!       Choices:
//!         - Text: Go left
//!           Needs:
//!             - lantern
//!           Sets:
//!             - went_left
//!           Goto: Chapter 2
//! ```

use crate::yaml::{self, Node};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_SCENES: &str = "lr.story.scenes/1";
pub const SCHEMA_ITEMS: &str = "lr.story.items/1";
pub const SCHEMA_PACK: &str = "lr.pack/1";

/// A chapter's lines, in order: (speaker, text). An empty speaker is narration.
#[derive(Debug, PartialEq)]
pub struct Chapter {
    pub number: usize,
    pub title: String,
    pub lines: Vec<(String, String)>,
    /// The chapter's `Choices:` entries, in order. Empty when the chapter has none.
    pub choices: Vec<Choice>,
}

/// One `Choices:` entry. `goto` is the chapter number it leads to (None means the next chapter).
#[derive(Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub needs: Vec<String>,
    pub sets: Vec<String>,
    pub goto: Option<usize>,
}

/// The chapters of one story file.
pub fn chapters(src: &str) -> Result<Vec<Chapter>, String> {
    let root = yaml::parse(src)?;
    let story = root
        .entries()
        .iter()
        .find_map(|(_, v)| v.get("Story"))
        .ok_or("no 'Story' section")?;
    let mut out = Vec::new();
    for (index, (name, body)) in story.entries().iter().enumerate() {
        let number = name
            .split_whitespace()
            .last()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(index + 1);
        let title = match body.get("Title") {
            Some(Node::Str(t)) => t.clone(),
            _ => String::new(),
        };
        let mut lines = Vec::new();
        if let Some(Node::List(items)) = body.get("Summary") {
            for item in items {
                lines.push(match item {
                    Node::Str(text) => (String::new(), text.clone()),
                    Node::Map(pairs) if pairs.len() == 1 => match &pairs[0] {
                        (speaker, Node::Str(text)) => {
                            let speaker = if speaker == "???" { String::new() } else { speaker.clone() };
                            (speaker, text.clone())
                        }
                        _ => return Err(format!("chapter {number}: a speaker line needs text")),
                    },
                    _ => return Err(format!("chapter {number}: unexpected line in Summary")),
                });
            }
        }
        let mut choices = Vec::new();
        match body.get("Choices") {
            None | Some(Node::Null) => {}
            Some(Node::List(items)) => {
                for item in items {
                    choices.push(parse_choice(number, item)?);
                }
            }
            Some(_) => return Err(format!("chapter {number}: Choices is a list of '- Text: ...' entries")),
        }
        out.push(Chapter { number, title, lines, choices });
    }
    if out.is_empty() {
        return Err("the Story section has no chapters".to_string());
    }
    let numbers: Vec<usize> = out.iter().map(|c| c.number).collect();
    for (i, n) in numbers.iter().enumerate() {
        if numbers[..i].contains(n) {
            return Err(format!("two chapters are numbered {n} (a chapter without a trailing number takes its position)"));
        }
    }
    for chapter in &out {
        for choice in &chapter.choices {
            if let Some(goto) = choice.goto.filter(|g| !numbers.contains(g)) {
                return Err(format!("chapter {}: Goto 'Chapter {goto}' is not a chapter in this file", chapter.number));
            }
        }
    }
    Ok(out)
}

/// One `Choices:` entry: a map with `Text` and optional `Needs`, `Sets` and `Goto`. Any other key is an error.
fn parse_choice(number: usize, node: &Node) -> Result<Choice, String> {
    let Node::Map(pairs) = node else {
        return Err(format!("chapter {number}: a Choices entry is a '- Text: ...' item"));
    };
    let mut text = None;
    let mut needs = Vec::new();
    let mut sets = Vec::new();
    let mut goto = None;
    for (at, (key, value)) in pairs.iter().enumerate() {
        if pairs[..at].iter().any(|(k, _)| k == key) {
            return Err(format!("chapter {number}: Choices key '{key}' is repeated in one entry"));
        }
        match (key.as_str(), value) {
            ("Text", Node::Str(t)) => text = Some(t.clone()),
            ("Needs", _) => needs = flag_list(number, "Needs", value)?,
            ("Sets", _) => sets = flag_list(number, "Sets", value)?,
            ("Goto", Node::Str(g)) => goto = Some(parse_goto(number, g)?),
            _ => return Err(format!("chapter {number}: unknown or malformed Choices key '{key}'")),
        }
    }
    let text = text.ok_or(format!("chapter {number}: a Choices entry needs Text"))?;
    Ok(Choice { text, needs, sets, goto })
}

/// A list of flag names. A missing or empty list is no flags.
fn flag_list(number: usize, key: &str, node: &Node) -> Result<Vec<String>, String> {
    match node {
        Node::Null => Ok(Vec::new()),
        Node::List(items) => items
            .iter()
            .map(|item| match item {
                Node::Str(flag) if is_flag(flag) => Ok(flag.clone()),
                _ => Err(format!("chapter {number}: {key} takes a list of flag names (lower case, digits, _)")),
            })
            .collect(),
        _ => Err(format!("chapter {number}: {key} takes a list of flag names")),
    }
}

/// A flag name: lower-case letters, digits and `_`, not empty.
fn is_flag(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `Chapter 2` (or just `2`) as a chapter number.
fn parse_goto(number: usize, raw: &str) -> Result<usize, String> {
    let trimmed = raw.trim();
    let digits = trimmed.strip_prefix("Chapter").unwrap_or(trimmed).trim();
    digits
        .parse::<usize>()
        .map_err(|_| format!("chapter {number}: Goto '{raw}' must name a chapter, like 'Chapter 2'"))
}

/// Where a choice goes: its `Goto` chapter's scene, or else the next chapter's scene (None after the last chapter).
fn choice_target(chapters: &[Chapter], names: &[String], index: usize, goto: Option<usize>) -> Option<String> {
    match goto {
        Some(n) => chapters.iter().position(|c| c.number == n).map(|j| names[j].clone()),
        None => names.get(index + 1).cloned(),
    }
}

/// The scene JSON of one `Choices:` entry: `if` (all its flags), `do` (set each flag) and `next`.
fn choice_json(choice: &Choice, chapters: &[Chapter], names: &[String], index: usize) -> Value {
    let mut out = json!({ "text": choice.text });
    if !choice.needs.is_empty() {
        let all: Vec<Value> = choice.needs.iter().map(|f| json!({ "flag": f })).collect();
        out["if"] = json!({ "all": all });
    }
    if !choice.sets.is_empty() {
        let ops: Vec<Value> = choice
            .sets
            .iter()
            .map(|f| json!({ "op": "set_flag", "flag": f, "value": true }))
            .collect();
        out["do"] = json!(ops);
    }
    if let Some(next) = choice_target(chapters, names, index, choice.goto) {
        out["next"] = json!(next);
    }
    out
}

/// A pack id: lower-case letters, digits and `_`, starting with a letter, 3-32 long.
pub fn is_pack_id(s: &str) -> bool {
    let b = s.as_bytes();
    (3..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// A pack id made from any name: lower-case, other characters become `_`, prefixed with `lore_` when needed.
pub fn slug(name: &str) -> String {
    let mut s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    let s = if s.is_empty() {
        "lore_story".to_string()
    } else if s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s
    } else {
        format!("lore_{s}")
    };
    let s: String = s.chars().take(32).collect::<String>().trim_end_matches('_').to_string();
    if s.len() < 3 {
        format!("{s}_story")
    } else {
        s
    }
}

/// Imports the files into the pack folder `out_dir` (created when missing) as `scenes/<file stem>.json`, and writes
/// `pack.json` and an empty `items/items.json` when the folder has none. Returns the scene ids written.
pub fn import(files: &[PathBuf], pack_id: &str, out_dir: &Path) -> Result<Vec<String>, Vec<String>> {
    let mut errs = Vec::new();
    let mut ids = Vec::new();
    let scenes_dir = out_dir.join("scenes");
    if let Err(e) = fs::create_dir_all(&scenes_dir) {
        return Err(vec![format!("{}: {e}", scenes_dir.display())]);
    }
    let mut seen_bases: Vec<String> = Vec::new();
    for file in files {
        let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let key = stem.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        if seen_bases.contains(&key) {
            errs.push(format!("{}: another input file has the same name '{stem}' (its scenes would overwrite or collide)", file.display()));
            continue;
        }
        seen_bases.push(key);
        let src = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                errs.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        let chapters = match chapters(&src) {
            Ok(c) => c,
            Err(e) => {
                errs.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        let base = format!("{pack_id}_{}", stem.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
        let names: Vec<String> = chapters.iter().map(|c| format!("{base}_ch{}", c.number)).collect();
        let mut scenes = Map::new();
        for (i, chapter) in chapters.iter().enumerate() {
            let lines: Vec<Value> = chapter
                .lines
                .iter()
                .map(|(speaker, text)| {
                    if speaker.is_empty() {
                        json!({ "text": text })
                    } else {
                        json!({ "speaker": speaker, "text": text })
                    }
                })
                .collect();
            let mut scene = json!({ "lines": lines });
            if chapter.choices.is_empty() {
                if let Some(next) = names.get(i + 1) {
                    scene["next"] = json!(next);
                }
            } else {
                // The choices carry the way on, so the scene itself has no `next`.
                let choices: Vec<Value> = chapter
                    .choices
                    .iter()
                    .map(|c| choice_json(c, &chapters, &names, i))
                    .collect();
                scene["choices"] = json!(choices);
            }
            scenes.insert(names[i].clone(), scene);
            ids.push(names[i].clone());
        }
        let doc = json!({ "schema": SCHEMA_SCENES, "source": format!("KyttusLab-LORE {}", file.file_name().unwrap_or_default().to_string_lossy()), "scenes": scenes });
        if let Err(e) = write_json(&scenes_dir.join(format!("{stem}.json")), &doc) {
            errs.push(e);
        }
    }
    if errs.is_empty() {
        let manifest = out_dir.join("pack.json");
        if !manifest.exists() {
            let doc = json!({
                "schema": SCHEMA_PACK,
                "id": pack_id,
                "name": pack_id,
                "version": "0.1.0",
                "description": "Story scenes imported from KyttusLab-LORE."
            });
            if let Err(e) = write_json(&manifest, &doc) {
                errs.push(e);
            }
        }
        let items = out_dir.join("items").join("items.json");
        if !items.exists() {
            let _ = fs::create_dir_all(out_dir.join("items"));
            if let Err(e) = write_json(&items, &json!({ "schema": SCHEMA_ITEMS, "items": {} })) {
                errs.push(e);
            }
        }
    }
    if errs.is_empty() {
        Ok(ids)
    } else {
        Err(errs)
    }
}

fn write_json(path: &Path, doc: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "The Test Storyline:\n  Story:\n    Chapter 1:\n      Title: Prep\n      Summary:\n        - ???: The lights hum.\n        - Felix: '\"Don''t tell.\"'\n    Chapter 2:\n      Title: Dive\n      Summary:\n        - Renata: \"Ready?\"\n";

    #[test]
    fn each_chapter_becomes_its_own_list_of_lines() {
        let c = chapters(SAMPLE).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].number, 1);
        assert_eq!(c[0].title, "Prep");
        assert_eq!(c[0].lines[0], (String::new(), "The lights hum.".to_string()));
        assert_eq!(c[0].lines[1], ("Felix".to_string(), "\"Don't tell.\"".to_string()));
        assert_eq!(c[1].lines[0].0, "Renata");
    }

    #[test]
    fn slug_and_ids_follow_the_pack_rules() {
        assert_eq!(slug("KyttusLab-LORE/The Last Route"), "kyttuslab_lore_the_last_route");
        assert_eq!(slug("2026 notes"), "lore_2026_notes");
        assert!(is_pack_id(&slug("the-last-route")));
        assert!(!is_pack_id("Bad Id"));
    }

    #[test]
    fn import_writes_a_pack_whose_scenes_chain_in_order() {
        let dir = std::env::temp_dir().join(format!("lr_story_import_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("book-one.yml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&src, SAMPLE).unwrap();
        let out = dir.join("pack");
        let ids = import(&[src], "lore_test", &out).unwrap();
        assert_eq!(ids, vec!["lore_test_book_one_ch1", "lore_test_book_one_ch2"]);
        let doc: Value = serde_json::from_str(&fs::read_to_string(out.join("scenes/book-one.json")).unwrap()).unwrap();
        assert_eq!(doc["scenes"]["lore_test_book_one_ch1"]["next"], json!("lore_test_book_one_ch2"));
        assert!(doc["scenes"]["lore_test_book_one_ch2"].get("next").is_none());
        assert!(out.join("pack.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A two-branch story: chapter 1 offers two choices (one needs a flag, one jumps to chapter 3), chapter 2 offers a
    /// choice that needs the flag set by chapter 1, and chapter 3 is the last, with a choice that has no `Goto`.
    const CHOICES: &str = "The Choice Storyline:\n  Story:\n    Chapter 1:\n      Title: Fork\n      Summary:\n        - ???: The corridor splits.\n      Choices:\n        - Text: \"Go left\"\n          Needs:\n            - lantern\n          Sets:\n            - went_left\n        - Text: \"Go right\"\n          Sets:\n            - went_right\n          Goto: Chapter 3\n    Chapter 2:\n      Title: Left\n      Summary:\n        - Felix: \"This way.\"\n      Choices:\n        - Text: \"Take the lantern\"\n          Needs:\n            - went_left\n    Chapter 3:\n      Title: Right\n      Summary:\n        - ???: Dark.\n      Choices:\n        - Text: \"Leave\"\n";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_story_lore_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn choices_parse_into_chapter_choices() {
        let c = chapters(CHOICES).unwrap();
        assert_eq!(c[0].choices.len(), 2);
        assert_eq!(c[0].choices[0], Choice { text: "Go left".into(), needs: vec!["lantern".into()], sets: vec!["went_left".into()], goto: None });
        assert_eq!(c[0].choices[1].goto, Some(3));
        assert!(c[0].choices[1].needs.is_empty());
        assert!(c[1].choices[0].sets.is_empty());
    }

    #[test]
    fn choices_import_with_flags_and_targets() {
        let dir = scratch("choices");
        let file = dir.join("fork.yml");
        fs::write(&file, CHOICES).unwrap();
        let out = dir.join("pack");
        let ids = import(&[file], "lore_choice", &out).unwrap();
        let base = "lore_choice_fork";
        assert_eq!(ids, vec![format!("{base}_ch1"), format!("{base}_ch2"), format!("{base}_ch3")]);

        let doc: Value = serde_json::from_str(&fs::read_to_string(out.join("scenes/fork.json")).unwrap()).unwrap();
        let ch1 = &doc["scenes"][format!("{base}_ch1")];
        assert!(ch1.get("next").is_none(), "a chapter with choices has no scene next");
        let choices = ch1["choices"].as_array().unwrap();
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0]["text"], json!("Go left"));
        assert_eq!(choices[0]["if"], json!({ "all": [{ "flag": "lantern" }] }));
        assert_eq!(choices[0]["do"], json!([{ "op": "set_flag", "flag": "went_left", "value": true }]));
        assert_eq!(choices[0]["next"], json!(format!("{base}_ch2")), "no Goto goes to the next chapter");
        assert!(choices[1].get("if").is_none());
        assert_eq!(choices[1]["next"], json!(format!("{base}_ch3")), "Goto Chapter 3");

        let ch2 = &doc["scenes"][format!("{base}_ch2")];
        assert_eq!(ch2["choices"][0]["if"], json!({ "all": [{ "flag": "went_left" }] }));
        assert_eq!(ch2["choices"][0]["next"], json!(format!("{base}_ch3")));

        let ch3 = &doc["scenes"][format!("{base}_ch3")];
        assert!(ch3.get("next").is_none());
        assert!(ch3["choices"][0].get("next").is_none(), "the last chapter's choice ends the scene");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_choices_are_reported() {
        let missing_goto = CHOICES.replace("Goto: Chapter 3", "Goto: Chapter 9");
        assert!(chapters(&missing_goto).unwrap_err().contains("Chapter 9"));
        let bad_flag = CHOICES.replace("- went_left", "- Went Left");
        assert!(chapters(&bad_flag).unwrap_err().contains("flag names"));
        let no_text = CHOICES.replace("- Text: \"Leave\"", "- Needs:\n            - x");
        assert!(chapters(&no_text).unwrap_err().contains("needs Text"));
    }

    /// A semi-choice file: chapter 1 has one choice list with two branches (chapters 2 and 3); each branch's own choice
    /// leads on to chapter 4, so the branches rejoin there.
    #[test]
    fn a_lore_file_with_one_choice_imports_and_the_branches_rejoin_at_the_next_chapter() {
        const SEMI: &str = "S:\n  Story:\n    Chapter 1:\n      Summary:\n        - ???: Fork.\n      Choices:\n        - Text: Left\n          Goto: Chapter 2\n        - Text: Right\n          Goto: Chapter 3\n    Chapter 2:\n      Summary:\n        - ???: Left branch.\n      Choices:\n        - Text: On\n          Goto: Chapter 4\n    Chapter 3:\n      Summary:\n        - ???: Right branch.\n      Choices:\n        - Text: On\n    Chapter 4:\n      Summary:\n        - ???: Together again.\n";
        let dir = scratch("semi");
        let file = dir.join("semi.yml");
        fs::write(&file, SEMI).unwrap();
        let out = dir.join("pack");
        let ids = import(&[file], "lore_semi", &out).unwrap();
        assert_eq!(ids.len(), 4);
        let doc: Value = serde_json::from_str(&fs::read_to_string(out.join("scenes/semi.json")).unwrap()).unwrap();
        let s = |n: u8| format!("lore_semi_semi_ch{n}");
        assert_eq!(doc["scenes"][s(1)]["choices"][0]["next"], json!(s(2)));
        assert_eq!(doc["scenes"][s(1)]["choices"][1]["next"], json!(s(3)));
        assert_eq!(doc["scenes"][s(2)]["choices"][0]["next"], json!(s(4)), "branch one rejoins");
        assert_eq!(doc["scenes"][s(3)]["choices"][0]["next"], json!(s(4)), "branch two rejoins");
        assert!(doc["scenes"][s(4)].get("choices").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn chapters_without_a_number_take_their_position_and_a_clash_is_an_error() {
        let ok = "S:\n  Story:\n    Prologue:\n      Summary:\n        - ???: A.\n    Epilogue:\n      Summary:\n        - ???: B.\n";
        let c = chapters(ok).unwrap();
        assert_eq!((c[0].number, c[1].number), (1, 2));
        let clash = "S:\n  Story:\n    Prologue:\n      Summary:\n        - ???: A.\n    Chapter 1:\n      Summary:\n        - ???: B.\n";
        assert!(chapters(clash).unwrap_err().contains("numbered 1"));
    }

    #[test]
    fn two_input_files_with_the_same_stem_are_an_error() {
        let dir = scratch("stems");
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        let (a, b) = (dir.join("a").join("book.yml"), dir.join("b").join("book.yml"));
        fs::write(&a, SAMPLE).unwrap();
        fs::write(&b, SAMPLE).unwrap();
        let errs = import(&[a, b], "lore_dup", &dir.join("pack")).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("same name")), "{errs:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repeated_choice_key_is_an_error() {
        let dup = CHOICES.replace("Goto: Chapter 3", "Goto: Chapter 3\n          Goto: Chapter 2");
        assert!(chapters(&dup).unwrap_err().contains("repeated"));
    }

    #[test]
    fn slug_always_gives_a_valid_pack_id() {
        for name in ["", "!!!", "a", "ab", "9", "é", "___", "x".repeat(80).as_str(), "12345678901234567890123456789012345", "Story One"] {
            let id = slug(name);
            assert!(is_pack_id(&id), "slug({name:?}) = {id:?}");
        }
        assert_eq!(slug("ab"), "ab_story");
    }

    /// The six real last-route files (KyttusLab-LORE, beside this crate's folder). Skipped when that repo is missing.
    #[test]
    fn the_six_real_last_route_files_import_with_no_choices() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("KyttusLab-LORE").join("stories").join("the-last-route");
        let Ok(read) = fs::read_dir(&dir) else { return };
        let mut files: Vec<PathBuf> = read
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "yml"))
            .collect();
        files.sort();
        assert_eq!(files.len(), 6, "the six last-route files");
        for file in &files {
            let src = fs::read_to_string(file).unwrap();
            let parsed = chapters(&src).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            assert!(parsed.iter().all(|c| !c.lines.is_empty()), "{}: every chapter has lines", file.display());
            assert!(parsed.iter().all(|c| c.choices.is_empty()), "{}: no Choices today", file.display());
        }
        let out = scratch("real");
        import(&files, "lore_real", &out).unwrap_or_else(|e| panic!("{e:?}"));
        for entry in fs::read_dir(out.join("scenes")).unwrap() {
            let doc: Value = serde_json::from_str(&fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
            for scene in doc["scenes"].as_object().unwrap().values() {
                assert!(scene.get("choices").is_none(), "no choices in the imported scenes");
            }
        }
        let _ = fs::remove_dir_all(&out);
    }

    /// The six real last-route files: each has choices-free chapters, and the scene ids they import to are unique across
    /// the whole set (no two books or chapters share one). Skipped when the LORE repo is missing.
    #[test]
    fn the_six_last_route_files_have_no_duplicate_scene_ids() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("KyttusLab-LORE").join("stories").join("the-last-route");
        let Ok(read) = fs::read_dir(&dir) else { return };
        let mut files: Vec<PathBuf> = read
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "yml"))
            .collect();
        files.sort();
        assert_eq!(files.len(), 6, "the six last-route files");
        for file in &files {
            let src = fs::read_to_string(file).unwrap();
            let parsed = chapters(&src).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            assert!(parsed.iter().all(|c| c.choices.is_empty()), "{}: no Choices today", file.display());
        }
        let out = scratch("unique_ids");
        let ids = import(&files, "lore_unique", &out).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!ids.is_empty(), "the six files give scenes");
        let mut seen = std::collections::BTreeSet::new();
        for id in &ids {
            assert!(seen.insert(id.clone()), "duplicate scene id '{id}'");
        }
        let _ = fs::remove_dir_all(&out);
    }

    /// The whole path: a LORE file with two chapters and one choice that sets a flag, imported into a core pack folder,
    /// loaded by `load::load` and played through the real `Game`. Picking the choice sets the flag, and the gated choice
    /// in chapter two shows only once the flag is set.
    #[test]
    fn a_choice_that_sets_a_flag_changes_chapter_two_in_the_real_game() {
        use crate::load::{self, Config};
        use crate::play::Game;
        use macroquad::prelude::{Color, Image};

        const FORK: &str = "The Fork Storyline:\n  Story:\n    Chapter 1:\n      Title: Fork\n      Summary:\n        - ???: Two ways.\n      Choices:\n        - Text: \"Go left\"\n          Sets:\n            - went_left\n    Chapter 2:\n      Title: Corridor\n      Summary:\n        - ???: The corridor is dark.\n      Choices:\n        - Text: \"Take the lantern\"\n          Needs:\n            - went_left\n";

        let root = scratch("e2e");
        let core = root.join("e2e_choice");
        fs::create_dir_all(core.join("maps")).unwrap();
        fs::write(
            core.join("pack.json"),
            r#"{"schema": "lr.pack/1", "id": "e2e_choice", "name": "E2E", "start": {"map": "start", "spawn": "here"}}"#,
        )
        .unwrap();
        fs::write(
            core.join("maps").join("start.json"),
            r#"{"schema": "lr.story.map/1", "id": "start", "name": "Start", "art": "start.png", "legend": {"00ff00": {"kind": "spawn", "name": "here"}, "ffffff": {"kind": "floor"}, "000000": {"kind": "wall"}}}"#,
        )
        .unwrap();
        // A 3x1 map: the spawn, a floor tile and a wall.
        let mut art = Image::gen_image_color(3, 1, Color::new(1.0, 1.0, 1.0, 1.0));
        art.set_pixel(0, 0, Color::new(0.0, 1.0, 0.0, 1.0));
        art.set_pixel(2, 0, Color::new(0.0, 0.0, 0.0, 1.0));
        art.export_png(&core.join("maps").join("start.png").to_string_lossy());

        let fork = root.join("fork.yml");
        fs::write(&fork, FORK).unwrap();
        import(&[fork], "e2e_choice", &core).unwrap();

        let cfg = Config { core: core.clone(), mod_dirs: Vec::new(), mod_roots: Vec::new() };
        let pack = load::load(&cfg).unwrap_or_else(|errs| panic!("the pack has problems: {errs:?}"));
        let (ch1, ch2) = ("e2e_choice_fork_ch1", "e2e_choice_fork_ch2");

        // Chapter two without the flag: its gated choice is hidden, so the scene just ends.
        let mut cold = Game::new(pack.clone()).unwrap();
        cold.start_scene(ch2);
        cold.advance();
        cold.advance();
        assert!(cold.offered_choices().is_empty(), "the gated choice is hidden without the flag");
        assert!(cold.talk.is_none());

        // Chapter one: the choice is offered; picking it sets the flag and goes on to chapter two.
        let mut g = Game::new(pack).unwrap();
        g.start_scene(ch1);
        g.advance();
        g.advance();
        assert_eq!(g.offered_choices(), vec!["Go left"]);
        assert!(!g.flags.contains("went_left"));
        g.choose(0);
        assert!(g.flags.contains("went_left"), "picking the choice sets its flag");
        assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some(ch2));

        // Chapter two with the flag: the gated choice now shows.
        g.advance();
        g.advance();
        assert_eq!(g.offered_choices(), vec!["Take the lantern"]);
        let _ = fs::remove_dir_all(&root);
    }


    /// `docs/LR_STORY.md` names every op the loader accepts and no longer documents the removed `xp` op.
    #[test]
    fn the_docs_list_every_op_the_loader_accepts_and_not_xp() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("LR_STORY.md");
        let docs = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let loader = include_str!("load.rs");
        let start = loader.find("fn parse_op(").expect("parse_op exists");
        let body = &loader[start..start + loader[start..].find("\n}\n").unwrap()];
        let mut ops = Vec::new();
        for line in body.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix('"') {
                if let Some((name, tail)) = rest.split_once('"') {
                    if tail.trim_start().starts_with("=>") && name != "xp" {
                        ops.push(name.to_string());
                    }
                }
            }
        }
        assert!(ops.len() >= 7, "found {ops:?}");
        for op in &ops {
            assert!(docs.contains(&format!("\"op\": \"{op}\"")), "docs/LR_STORY.md does not show {{\"op\": \"{op}\"}}");
        }
        assert!(!docs.contains("\"op\": \"xp\""), "the removed xp op is still documented");
    }
    /// Chapter 1 offers two choices that both go to chapter 3; played through the real `Game`, each reaches chapter 3.
    #[test]
    fn two_branches_rejoin_at_the_next_chapter() {
        use crate::load::{self, Config};
        use crate::play::Game;
        use macroquad::prelude::{Color, Image};

        const JOIN: &str = "The Join Storyline:\n  Story:\n    Chapter 1:\n      Summary:\n        - ???: Two ways.\n      Choices:\n        - Text: \"Left\"\n          Goto: Chapter 3\n        - Text: \"Right\"\n          Goto: Chapter 3\n    Chapter 2:\n      Summary:\n        - ???: Unused.\n    Chapter 3:\n      Summary:\n        - ???: Together again.\n";

        let root = scratch("rejoin");
        let core = root.join("rejoin_pack");
        fs::create_dir_all(core.join("maps")).unwrap();
        fs::write(core.join("pack.json"), r#"{"schema": "lr.pack/1", "id": "rejoin_pack", "name": "R", "start": {"map": "start", "spawn": "here"}}"#).unwrap();
        fs::write(
            core.join("maps").join("start.json"),
            r#"{"schema": "lr.story.map/1", "id": "start", "name": "Start", "art": "start.png", "legend": {"00ff00": {"kind": "spawn", "name": "here"}, "ffffff": {"kind": "floor"}, "000000": {"kind": "wall"}}}"#,
        )
        .unwrap();
        let mut art = Image::gen_image_color(3, 1, Color::new(1.0, 1.0, 1.0, 1.0));
        art.set_pixel(0, 0, Color::new(0.0, 1.0, 0.0, 1.0));
        art.set_pixel(2, 0, Color::new(0.0, 0.0, 0.0, 1.0));
        art.export_png(&core.join("maps").join("start.png").to_string_lossy());
        let file = root.join("join.yml");
        fs::write(&file, JOIN).unwrap();
        import(&[file], "rejoin_pack", &core).unwrap();

        let cfg = Config { core: core.clone(), mod_dirs: Vec::new(), mod_roots: Vec::new() };
        let pack = load::load(&cfg).unwrap_or_else(|errs| panic!("the pack has problems: {errs:?}"));
        for pick in 0..2 {
            let mut g = Game::new(pack.clone()).unwrap();
            g.start_scene("rejoin_pack_join_ch1");
            g.advance();
            g.advance();
            assert_eq!(g.offered_choices(), vec!["Left", "Right"]);
            g.choose(pick);
            assert_eq!(g.talk.as_ref().map(|t| t.scene.as_str()), Some("rejoin_pack_join_ch3"), "choice {pick} reaches chapter 3");
        }
        let _ = fs::remove_dir_all(&root);
    }
}
