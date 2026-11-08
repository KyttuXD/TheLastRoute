//! Command line: argument parsing and the window-less `--check`. Hub file (integrator).
//!
//! ```text
//! night_watch                              the full game (title screen)
//! night_watch --mode ID --level N [--seed S]   start a shift directly
//! night_watch --shot PATH [--mode ID --level N] [--frames N] [--monitor ROOM]
//! night_watch --check                      validate the data, simulate every built mode, exit 0/1
//! ```
#![allow(dead_code)]

use crate::core::{self as nw, ModesData};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub check: bool,
    pub help: bool,
    pub mode: Option<String>,
    pub level: i32,
    pub seed: Option<u32>,
    pub shot: Option<String>,
    pub frames: u32,
    pub monitor: Option<String>,
}

pub const USAGE: &str = "night_watch [--mode ID --level N [--seed S]] [--shot PATH [--frames N] [--monitor ROOM]] [--check]";

impl Args {
    /// Parse the arguments after the program name. An unknown flag or a bad value is an error string.
    pub fn parse(list: &[String]) -> Result<Args, String> {
        let mut a = Args { level: 1, frames: 30, ..Args::default() };
        let mut i = 0;
        while i < list.len() {
            let flag = list[i].as_str();
            let mut value = |name: &str| -> Result<String, String> {
                i += 1;
                list.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))
            };
            match flag {
                "--check" => a.check = true,
                "--help" | "-h" => a.help = true,
                "--mode" => a.mode = Some(value("--mode")?),
                "--level" => {
                    let v = value("--level")?;
                    a.level = v.parse::<i32>().map_err(|_| format!("--level: '{v}' is not a number"))?.max(1);
                }
                "--seed" => {
                    let v = value("--seed")?;
                    a.seed = Some(v.parse::<u32>().map_err(|_| format!("--seed: '{v}' is not a number"))?);
                }
                "--shot" => a.shot = Some(value("--shot")?),
                "--frames" => {
                    let v = value("--frames")?;
                    a.frames = v.parse::<u32>().map_err(|_| format!("--frames: '{v}' is not a number"))?.max(1);
                }
                "--monitor" => a.monitor = Some(value("--monitor")?),
                other => return Err(format!("unknown argument '{other}'")),
            }
            i += 1;
        }
        if let Some(m) = &a.mode {
            if !crate::registry::MODE_IDS.contains(&m.as_str()) {
                return Err(format!("--mode: '{m}' is not one of {}", crate::registry::MODE_IDS.join(" ")));
            }
        }
        Ok(a)
    }
}

/// `--check`: validate every embedded data file, then simulate every built mode level twice and compare. Prints one
/// line per run and returns the process exit code (0 ok, 1 on any problem). Modes that are not built yet are listed and
/// do not fail the check.
pub fn run_check() -> i32 {
    // --check runs with the admin gate open, so the dev overlays are exercised like any other code
    crate::net::account::set_force_admin(true);
    crate::hud::AdminGate::force_admin().publish();
    let mut problems: Vec<String> = nw::check_data();
    problems.extend(mode_data_problems());
    println!("data: {}", if problems.is_empty() { "ok".to_string() } else { format!("{} problem(s)", problems.len()) });

    let modes = ModesData::load();
    let factory = |s: &nw::Session| crate::registry::make_headless(s);
    for id in crate::registry::MODE_IDS {
        if !crate::registry::playable(id) {
            println!("{id}: not built");
            continue;
        }
        let levels = modes.get(id).map_or(0, |m| m.levels.len()) as i32;
        if levels == 0 {
            problems.push(format!("{id}: no levels to run"));
            continue;
        }
        for level in 1..=levels {
            let first = nw::run_headless(&factory, id, level);
            let second = nw::run_headless(&factory, id, level);
            match (first, second) {
                (Ok(a), Ok(b)) => {
                    println!("{id} level {level}: {} {:016x}", a.outcome_name(), a.digest);
                    if a.digest != b.digest || a.event_hash != b.event_hash || a.events != b.events || a.outcome != b.outcome {
                        problems.push(format!("{id} level {level}: two runs of seed {} differ ({:016x} vs {:016x})", nw::CHECK_SEED, a.digest, b.digest));
                    }
                }
                (Err(e), _) | (_, Err(e)) => problems.push(e),
            }
        }
    }

    for p in &problems {
        eprintln!("PROBLEM: {p}");
    }
    if problems.is_empty() {
        println!("night_watch --check: ok");
        0
    } else {
        println!("night_watch --check: {} problem(s)", problems.len());
        1
    }
}

/// The data checks that live with each mode (they stay out of core so core does not depend on modes), plus the shared nav
/// graph, the behaviour file against the pose list, and the local clock.
fn mode_data_problems() -> Vec<String> {
    use crate::modes;
    let mut p: Vec<String> = Vec::new();
    p.extend(crate::hunters::nav::NavMap::classic().validate().into_iter().map(|e| format!("nav classic: {e}")));
    let poses = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
    p.extend(crate::hunters::behaviour::BehaviourData::embedded().validate(&poses).into_iter().map(|e| format!("behaviour: {e}")));
    p.extend(modes::classic::check_data());
    p.extend(modes::mask::check_data());
    p.extend(modes::lure::check_data());
    p.extend(modes::bedroom::check_data());
    p.extend(modes::story::check_data());
    p.extend(modes::descent::map::nav().validate().into_iter().map(|e| format!("nav descent: {e}")));
    p.extend(modes::dayshift::data_problems());
    p.extend(modes::openair::check());
    p.extend(modes::mixed::check_data());
    let clock = crate::core::clock::local_hhmm();
    if clock.len() != 5 || clock.as_bytes()[2] != b':' {
        p.push(format!("clock: local_hhmm gave '{clock}'"));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(list: &[&str]) -> Result<Args, String> {
        Args::parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn defaults() {
        let a = p(&[]).unwrap();
        assert!(!a.check && a.mode.is_none() && a.shot.is_none());
        assert_eq!((a.level, a.frames), (1, 30));
    }

    #[test]
    fn flags_parse() {
        let a = p(&["--mode", "mask", "--level", "3", "--seed", "99", "--shot", "a.png", "--frames", "5", "--monitor", "stage"]).unwrap();
        assert_eq!(a.mode.as_deref(), Some("mask"));
        assert_eq!((a.level, a.seed, a.frames), (3, Some(99), 5));
        assert_eq!(a.shot.as_deref(), Some("a.png"));
        assert_eq!(a.monitor.as_deref(), Some("stage"));
        assert!(p(&["--check"]).unwrap().check);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(p(&["--mode", "nope"]).is_err());
        assert!(p(&["--level"]).is_err());
        assert!(p(&["--level", "x"]).is_err());
        assert!(p(&["--wat"]).is_err());
    }
}
