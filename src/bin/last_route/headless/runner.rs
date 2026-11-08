//! `--bot` / `--playtest-runner`: the bot plays a run headless (dev/playtest/playtest_runner.gd
//! + playtest_scenarios.gd + PlaytestReporter). Same options (`--key=value` or `--key value`):
//! scenario (survive, collect_relays, reach_depth, explore_coverage, goto_check), seed,
//! difficulty, mode, minutes, bot_seed, target, goal_x / goal_z, out, name, fixed_fps,
//! hash_frames, the limits max_stuck / max_soft_locks / max_unreachable / max_idle_s /
//! max_errors, and json (print report.json on stdout). Writes `<out>/report.json` and
//! `<out>/events.jsonl`; prints the original's `PLAYTEST PASS|FAIL: ...` lines; exit 0 pass,
//! 1 thresholds failed, 2 could not run.

use super::bot::Bot;
use super::protocol::{seed_text, Session};
use super::sim::{parse_difficulty, parse_mode, r2, Mode, Sim};
use super::Args;
use serde_json::{json, Value};
use std::io::Write;
use crate::shared::clock::Instant;

pub const SCENARIOS: &[&str] = &[
    "survive",
    "collect_relays",
    "reach_depth",
    "explore_coverage",
    "goto_check",
];

const MAX_MINUTES: f64 = 1440.0;
const MAX_DEADLINE_S: f64 = 1.0e7;

/// A playtest length that can run: finite, above zero, and a day at most.
fn minutes_ok(m: f64) -> bool {
    m.is_finite() && m > 0.0 && m <= MAX_MINUTES
}

fn num(a: &Args, k: &str, d: f64) -> f64 {
    a.value(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}

pub fn main(a: &Args) -> i32 {
    let scenario = a.value("scenario").unwrap_or_else(|| "survive".into());
    if !SCENARIOS.contains(&scenario.as_str()) {
        eprintln!(
            "PLAYTEST SKIP: unknown scenario '{scenario}' ({})",
            SCENARIOS.join(", ")
        );
        return 2;
    }
    let seed_label = a.value("seed").unwrap_or_else(|| "PLAYTEST".into());
    let Some(diff) = parse_difficulty(&a.value("difficulty").unwrap_or_else(|| "standard".into()))
    else {
        eprintln!("PLAYTEST SKIP: unknown difficulty");
        return 2;
    };
    let Some(mode) = parse_mode(&a.value("mode").unwrap_or_else(|| "infinite".into())) else {
        eprintln!("PLAYTEST SKIP: unknown mode (finite, infinite)");
        return 2;
    };
    let minutes = num(a, "minutes", 2.0);
    if !minutes_ok(minutes) {
        eprintln!("PLAYTEST SKIP: --minutes must be a finite number above 0 (and at most {MAX_MINUTES})");
        return 2;
    }
    let bot_seed = num(a, "bot_seed", 1.0) as u32;
    let target = num(a, "target", 0.0) as i64;
    let hz = num(a, "fixed_fps", 60.0).clamp(10.0, 240.0);
    let name = a
        .value("name")
        .unwrap_or_else(|| format!("last_route_{scenario}_{seed_label}"));
    let out = a
        .value("out")
        .unwrap_or_else(|| format!(".playtest/{name}"));
    let deadline = num(a, "deadline", minutes * 60.0 * 3.0 + 120.0);
    let deadline = if deadline.is_finite() { deadline.clamp(1.0, MAX_DEADLINE_S) } else { MAX_DEADLINE_S };
    let limits = json!({
        "max_stuck": num(a, "max_stuck", 3.0), "max_soft_locks": num(a, "max_soft_locks", 0.0),
        "max_errors": num(a, "max_errors", 0.0), "max_unreachable": num(a, "max_unreachable", 6.0),
        "max_idle_s": num(a, "max_idle_s", 90.0),
    });
    let wall = Instant::now();
    {
        // watchdog: a stuck loop still produces a failing report
        let out = out.clone();
        let (scen, seed) = (scenario.clone(), seed_label.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(deadline));
            let rep = json!({ "game": "last_route", "scenario": scen, "seed": seed, "pass": false, "scenario_failure": "deadline",
                "failures": [format!("wall-clock deadline of {deadline:.0} s exceeded")] });
            let _ = std::fs::create_dir_all(&out);
            let _ = std::fs::write(
                format!("{out}/report.json"),
                serde_json::to_string_pretty(&rep).unwrap(),
            );
            println!("PLAYTEST FAIL: last_route/{scen} seed={seed} (deadline)");
            std::process::exit(1);
        });
    }

    let mut s = Session::new(Sim::new());
    s.sim.dt = 1.0 / hz as f32;
    let seed = seed_text(&seed_label);
    let start = |s: &mut Session, seed: u32| {
        let r = s.handle_safe(&json!({ "cmd": "start_run", "seed": seed, "difficulty": diff.id(), "mode": mode.id(), "bot_seed": bot_seed, "fixed_fps": hz }));
        r["ok"] == true
    };
    if !start(&mut s, seed) {
        eprintln!("PLAYTEST SKIP: start_run failed");
        return 2;
    }
    let mut bot = Bot::new(&seed_label, bot_seed);
    bot.debug = !a.value("debug").map(|v| v == "false").unwrap_or(false);
    let total = (minutes * 60.0 * hz) as u64;
    let mut frames = 0u64;
    let mut outcome = "time_limit";
    let mut scenario_failure = String::new();
    let mut deaths: Vec<Value> = vec![];
    let mut restarts = 0u32;
    let mut sim_ms: Vec<f64> = vec![];
    let mut state_hash = Value::Null;
    let hash_frames = num(a, "hash_frames", 0.0) as u64;
    let mut goto = Value::Null;
    let mut closest = f32::INFINITY;
    let goal = (
        num(a, "goal_x", f64::NAN) as f32,
        num(a, "goal_z", f64::NAN) as f32,
    );
    if scenario == "goto_check" {
        if goal.0.is_nan() || goal.1.is_nan() {
            eprintln!("PLAYTEST SKIP: goto_check needs --goal-x and --goal-z");
            return 2;
        }
        let g = s.sim.game.as_mut().unwrap();
        g.admin.god = true;
        g.admin.no_target = true;
        bot.forced = Some(goal);
    }
    let start_pos = s.sim.game.as_ref().map(|g| g.p.pos).unwrap_or_default();
    let mut dead_wait = 0u32;
    while frames < total {
        if s.sim
            .game
            .as_ref()
            .map(|g| g.report.is_some())
            .unwrap_or(false)
        {
            let g = s.sim.game.as_ref().unwrap();
            let r = g.report.as_ref().unwrap();
            if dead_wait == 0 {
                deaths.push(json!({ "t": r2(frames as f32 / hz as f32), "hunter": r.hunter, "cause": format!("{} / {}", r.cause, r.hunter_name), "depth": r.depth }));
            }
            if scenario != "survive" {
                outcome = "died";
                if scenario != "goto_check" {
                    scenario_failure = format!(
                        "{scenario}: died before target {target} (got {})",
                        progress(&scenario, &bot)
                    );
                }
                break;
            }
            // survive: wait 180 frames on the report, restart, keep playing
            dead_wait += 1;
            if dead_wait >= 180 {
                dead_wait = 0;
                restarts += 1;
                if !start(&mut s, seed.wrapping_add(restarts)) {
                    scenario_failure = "restart failed".into();
                    break;
                }
            }
            frames += 1;
            continue;
        }
        let t = Instant::now();
        bot.step(&mut s.sim);
        sim_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        frames += 1;
        if hash_frames > 0 && frames == hash_frames {
            state_hash = Value::from(s.state_hash());
        }
        match scenario.as_str() {
            "collect_relays" | "reach_depth" | "explore_coverage"
                if target > 0 && progress(&scenario, &bot) >= target =>
            {
                outcome = "target_reached";
                break;
            }
            "goto_check" => {
                let g = s.sim.game.as_ref().unwrap();
                let d = crate::game::pdist(g.p.pos, goal);
                closest = closest.min(d);
                if d <= 1.6 {
                    outcome = "arrived";
                    break;
                }
                bot.forced = Some(goal);
            }
            _ => {}
        }
    }
    if matches!(
        scenario.as_str(),
        "collect_relays" | "reach_depth" | "explore_coverage"
    ) && target > 0
        && outcome == "time_limit"
    {
        scenario_failure = format!(
            "{scenario}: target {target} not reached in {minutes} game minutes (got {})",
            progress(&scenario, &bot)
        );
    }
    if scenario == "goto_check" {
        let arrived = outcome == "arrived";
        goto = json!({ "arrived": arrived, "seconds": r2(frames as f32 / hz as f32), "closest_m": r2(closest), "start": [r2(start_pos.0), r2(start_pos.1)], "goal": [goal.0, goal.1],
            "stuck_spots": bot.events.iter().filter(|e| e["kind"] == "stuck").map(|e| e["at"].clone()).collect::<Vec<_>>() });
        if !arrived {
            scenario_failure = format!("goto_check: did not arrive (closest {:.1} m)", closest);
        }
    }

    let stats = bot.stats_json();
    let mut failures: Vec<String> = vec![];
    let lim = |k: &str| limits[k].as_f64().unwrap_or(0.0);
    let sv = |k: &str| stats[k].as_f64().unwrap_or(0.0);
    if sv("soft_locks") > lim("max_soft_locks") {
        failures.push(format!(
            "soft_locks {} > {}",
            sv("soft_locks"),
            lim("max_soft_locks")
        ));
    }
    if sv("stuck_events") > lim("max_stuck") {
        failures.push(format!(
            "stuck_events {} > {}",
            sv("stuck_events"),
            lim("max_stuck")
        ));
    }
    if sv("idle_max_s") > lim("max_idle_s") {
        failures.push(format!(
            "idle {:.0} s > {:.0} (bot stopped acting)",
            sv("idle_max_s"),
            lim("max_idle_s")
        ));
    }
    if sv("unreachable_goals") > lim("max_unreachable") {
        failures.push(format!(
            "unreachable_goals {} > {}",
            sv("unreachable_goals"),
            lim("max_unreachable")
        ));
    }
    if s.errors as f64 > lim("max_errors") {
        failures.push(format!(
            "engine errors {} > {}",
            s.errors,
            lim("max_errors")
        ));
    }
    if !scenario_failure.is_empty() {
        failures.push(scenario_failure.clone());
    }
    let pass = failures.is_empty();
    let game_s = frames as f64 / hz;
    let avg = if sim_ms.is_empty() {
        0.0
    } else {
        sim_ms.iter().sum::<f64>() / sim_ms.len() as f64
    };
    let mut sorted = sim_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = sorted.get(sorted.len() * 95 / 100).copied().unwrap_or(0.0);
    let g = s.sim.game.as_ref();
    let report = json!({
        "game": "last_route", "scenario": scenario, "seed": seed_label, "run_seed": seed, "difficulty": diff.id(), "mode": mode.id(),
        "machine": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH), "protocol": super::protocol::PROTOCOL,
        "revision": "rust-port", "args": a.raw, "scenario_failure": scenario_failure, "pass": pass, "failures": failures,
        "log": { "errors": s.errors, "warnings": s.warnings }, "log_lines": s.log,
        "bot": stats, "wall_seconds": r2(wall.elapsed().as_secs_f32()), "outcome": outcome,
        "game_seconds": r2(game_s as f32), "frames": frames, "restarts": restarts,
        "perf": { "sim_ms_avg": (avg * 1000.0).round() / 1000.0, "sim_ms_p95": (p95 * 1000.0).round() / 1000.0, "sim_ms_max": sorted.last().copied().unwrap_or(0.0), "samples": sim_ms.len(),
                  "realtime_factor": if avg > 0.0 { ((1000.0 / hz) / avg * 10.0).round() / 10.0 } else { 0.0 } },
        "deaths": deaths, "state_hash": state_hash, "goto_check": goto, "limits": limits,
        "final": { "depth": g.map(|g| g.depth), "relays": g.map(|g| g.relays_collected), "score": g.map(|g| g.score), "director": s.director_json() },
    });
    let _ = std::fs::create_dir_all(&out);
    let _ = std::fs::write(
        format!("{out}/report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    );
    if let Ok(mut f) = std::fs::File::create(format!("{out}/events.jsonl")) {
        for e in bot.events.iter().chain(s.sim.events.iter()) {
            let _ = writeln!(f, "{e}");
        }
    }
    if a.flag("json") {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
    let line = format!(
        "PLAYTEST {}: last_route/{scenario} seed={seed_label} game_s={game_s:.1} frames={frames}  stuck={} soft_locks={} unreachable={} errors={} deaths={} relays={} depth={} chunks={}",
        if pass { "PASS" } else { "FAIL" }, sv("stuck_events"), sv("soft_locks"), sv("unreachable_goals"), s.errors, deaths.len(), sv("relays"), sv("max_depth"), sv("chunks_visited"));
    if a.flag("json") {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
    for f in report["failures"].as_array().unwrap() {
        let l = format!("PLAYTEST   failure: {}", f.as_str().unwrap_or(""));
        if a.flag("json") {
            eprintln!("{l}")
        } else {
            println!("{l}")
        }
    }
    let l = format!("PLAYTEST report: {out}/report.json");
    if a.flag("json") {
        eprintln!("{l}")
    } else {
        println!("{l}")
    }
    if pass {
        0
    } else {
        1
    }
}

fn progress(scenario: &str, bot: &Bot) -> i64 {
    match scenario {
        "collect_relays" => bot.stats.relays as i64,
        "reach_depth" => bot.stats.max_depth as i64,
        "explore_coverage" => bot.stats.chunks_visited as i64,
        _ => 0,
    }
}

#[allow(dead_code)]
fn _mode(m: Mode) -> &'static str {
    m.id()
}

#[cfg(test)]
mod minutes_tests {
    use super::*;

    #[test]
    fn bad_minutes_are_refused() {
        for m in [f64::NAN, f64::INFINITY, -2.0, 0.0, 1e300] {
            assert!(!minutes_ok(m), "{m}");
        }
        assert!(minutes_ok(2.0));
    }
}
