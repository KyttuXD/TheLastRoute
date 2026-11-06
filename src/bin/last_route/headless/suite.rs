//! `--test-suite <files|dirs|globs...>`: runs JSON suite files against the headless game.
//!
//! A suite: `{"suite": name, "description", "ported_from", "defaults": {start_run fields},
//! "tests": [{"name", "start_run": {...} (merged over defaults; false = no run),
//! "steps": [...]}]}`. A step is a protocol command (`{"cmd": "step", "frames": 60}`) and/or
//! an `expect` list checked on the probe document after it (expect.rs; sections run, player,
//! meters, hotbar, hunters, relays, focus, objects, rooms, report, director, nav, map,
//! discovered, obs, events, bot, last = this step's reply). Step options: `save_as` (keep the
//! reply as a variable for `"var"` expectations), `expect_ok` (default true; false expects
//! an error reply), `repeat` (run the command n times).
//!
//! Output: one JSON report on stdout (`--out FILE` also writes it); progress on stderr in the
//! validator's form (`== suite`, `  FAIL ...`, `-- suite NAME PASS 0.4s`). Exit 0 all
//! passed, 1 a check failed, 2 nothing could run (no files, unreadable JSON).

use super::expect;
use super::protocol::Session;
use super::sim::Sim;
use super::Args;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use crate::shared::clock::Instant;

fn wildcard(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    go(&p, &n)
}

/// Files for the arguments: plain files, directories (their *.json), and globs in the last
/// path part (PowerShell does not expand them).
pub fn expand(args: &[String]) -> Vec<PathBuf> {
    let mut out = vec![];
    for a in args {
        let p = Path::new(a);
        if a.contains('*') || a.contains('?') {
            let dir = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let pat = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|f| {
                    f.is_file() && wildcard(&pat, &f.file_name().unwrap().to_string_lossy())
                })
                .collect();
            found.sort();
            out.extend(found);
        } else if p.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(p)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|f| f.extension().map(|x| x == "json").unwrap_or(false))
                .collect();
            found.sort();
            out.extend(found);
        } else {
            out.push(p.to_path_buf());
        }
    }
    out
}

fn merge(base: &Value, over: &Value) -> Value {
    let mut o = base.as_object().cloned().unwrap_or_default();
    if let Some(m) = over.as_object() {
        for (k, v) in m {
            o.insert(k.clone(), v.clone());
        }
    }
    Value::Object(o)
}

/// Runs one test; returns its failures.
fn run_test(s: &mut Session, defaults: &Value, t: &Value) -> (Vec<String>, usize) {
    let mut failures = vec![];
    let mut vars: Map<String, Value> = Map::new();
    let start = t.get("start_run").cloned().unwrap_or(json!({}));
    if start != Value::Bool(false) {
        let mut c = merge(defaults, &start);
        c["cmd"] = json!("start_run");
        let r = s.handle_safe(&c);
        if r["ok"] != true {
            return (vec![format!("start_run failed: {}", r["error"])], 0);
        }
    } else {
        s.sim.game = None;
    }
    let empty = vec![];
    let steps = t.get("steps").and_then(|v| v.as_array()).unwrap_or(&empty);
    for (i, step) in steps.iter().enumerate() {
        let label = step
            .get("label")
            .and_then(|v| v.as_str())
            .map(|l| format!("step {} ({l})", i + 1))
            .unwrap_or_else(|| format!("step {}", i + 1));
        if step.get("cmd").is_some() {
            let times = step
                .get("repeat")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1);
            let mut cmd = step.clone();
            let until = step["cmd"] == "step_until";
            if let Some(o) = cmd.as_object_mut() {
                for k in [
                    "expect",
                    "save_as",
                    "expect_ok",
                    "repeat",
                    "label",
                    "must_meet",
                ] {
                    if !(until && k == "expect") {
                        o.remove(k);
                    }
                }
            }
            let mut r = Value::Null;
            for _ in 0..times {
                r = s.handle_safe(&cmd);
                if r.is_null() {
                    r = json!({ "ok": false, "error": "deferred reply (screenshot) in a headless suite" });
                }
            }
            let want_ok = step
                .get("expect_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if (r["ok"] == true) != want_ok {
                failures.push(format!(
                    "{label}: {} {} ({})",
                    cmd["cmd"].as_str().unwrap_or("?"),
                    if want_ok {
                        "failed"
                    } else {
                        "should have failed"
                    },
                    r.get("error").cloned().unwrap_or(Value::Null)
                ));
            }
            if cmd["cmd"] == "step_until"
                && step
                    .get("must_meet")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                && r["met"] == false
            {
                failures.push(format!(
                    "{label}: step_until not met in {} frames: {}",
                    r["frames"], r["unmet"]
                ));
            }
            s.last = r.clone();
            if let Some(name) = step.get("save_as").and_then(|v| v.as_str()) {
                vars.insert(name.to_string(), r);
            }
        }
        if let Some(list) = step.get("expect").filter(|_| step["cmd"] != "step_until") {
            let mut keys = HashSet::new();
            expect::sections(list, &mut keys);
            let doc = s.probe(&keys);
            for f in expect::check_all(&doc, list, &vars) {
                failures.push(format!("{label}: {f}"));
            }
        }
    }
    (failures, steps.len())
}

pub fn run_file(s: &mut Session, path: &Path, only: &Option<String>) -> Value {
    let t0 = Instant::now();
    let name_of = || {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let doc: Value = match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            return json!({ "suite": name_of(), "file": path.to_string_lossy(), "pass": false, "error": e, "tests": [], "seconds": 0.0 })
        }
    };
    let suite = doc
        .get("suite")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(name_of);
    eprintln!("== {suite}");
    let defaults = doc.get("defaults").cloned().unwrap_or(json!({}));
    let mut tests = vec![];
    let mut pass = true;
    let empty = vec![];
    for t in doc
        .get("tests")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty)
    {
        let tname = t
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        if let Some(o) = only {
            if !tname.contains(o.as_str()) {
                continue;
            }
        }
        let tt = Instant::now();
        let (failures, steps) = run_test(s, &defaults, t);
        for f in &failures {
            eprintln!("  FAIL  {tname}: {f}");
        }
        pass &= failures.is_empty();
        tests.push(json!({ "name": tname, "pass": failures.is_empty(), "steps": steps, "failures": failures, "seconds": (tt.elapsed().as_secs_f64() * 100.0).round() / 100.0, "frames": s.sim.frame }));
    }
    let secs = (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0;
    eprintln!(
        "-- suite {suite} {} {secs}s",
        if pass { "PASS" } else { "FAIL" }
    );
    json!({
        "suite": suite, "file": path.to_string_lossy(), "description": doc.get("description").cloned().unwrap_or(Value::Null),
        "ported_from": doc.get("ported_from").cloned().unwrap_or(Value::Null),
        "pass": pass, "seconds": secs, "tests": tests,
    })
}

pub fn main(a: &Args) -> i32 {
    let files = expand(&a.values("test-suite"));
    if files.is_empty() {
        eprintln!("--test-suite: no suite files given (e.g. --test-suite tests/*.json)");
        return 2;
    }
    let only = a.value("only");
    let t0 = Instant::now();
    let mut s = Session::new(Sim::new());
    let mut suites = vec![];
    for f in &files {
        suites.push(run_file(&mut s, f, &only));
    }
    let n_tests: usize = suites
        .iter()
        .map(|s| s["tests"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    let passed: usize = suites
        .iter()
        .map(|s| {
            s["tests"]
                .as_array()
                .map(|a| a.iter().filter(|t| t["pass"] == true).count())
                .unwrap_or(0)
        })
        .sum();
    let broken = suites.iter().filter(|s| s.get("error").is_some()).count();
    let suites_passed = suites.iter().filter(|s| s["pass"] == true).count();
    let all = suites_passed == suites.len();
    let report = json!({
        "version": 1, "pass": all, "seconds": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0,
        "totals": { "suites": suites.len(), "suites_passed": suites_passed, "tests": n_tests, "passed": passed, "failed": n_tests - passed, "unreadable": broken },
        "suites": suites,
    });
    let text = serde_json::to_string_pretty(&report).unwrap();
    if let Some(out) = a.value("out") {
        if let Some(d) = Path::new(&out).parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&out, &text);
    }
    println!("{text}");
    eprintln!(
        "RESULT: {} ({} of {} tests passed, {} of {} suites)",
        if all { "PASS" } else { "FAIL" },
        passed,
        n_tests,
        suites_passed,
        report["totals"]["suites"]
    );
    if broken == suites.len() {
        2
    } else if all {
        0
    } else {
        1
    }
}
