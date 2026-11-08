//! Nearby-player text to speech (TODO Multiplayer: nearby-player text to speech first). Godot speaks chat aloud
//! (`MAX_CHAT_LENGTH` 80, short lines). Pure rules (range, depth, toggle, mute, length, duplicates, volume, queue) are
//! separate from the OS speaker. Text never reaches a shell: it is one argument, or stdin on Windows.
//! Voice chat reuses `should_speak` and the mute set later. Tuning: `data/net/tts.json`.

use crate::net::chat::ChatLine;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

const TEXT: &str = include_str!("../data/net/tts.json");

#[derive(Clone, Debug)]
pub struct Config {
    pub radius: f64,
    pub max_chars: usize,
    pub trim_over_length: bool,
    pub duplicate_window_ms: i64,
    pub queue_max: usize,
    pub volume_floor: f64,
    /// Speech rate for windows, macos, linux.
    pub rate: [i64; 3],
    pub voices: [String; 3],
    pub linux_programs: Vec<String>,
}

fn num(v: &Value, group: &str, key: &str) -> Result<f64, String> {
    let n = if group.is_empty() { v.get(key) } else { v.get(group).and_then(|g| g.get(key)) };
    let path = if group.is_empty() { key.to_string() } else { format!("{group}.{key}") };
    n.and_then(Value::as_f64).ok_or_else(|| format!("tts.json: key '{path}' missing or not a number"))
}

fn text(v: &Value, group: &str, key: &str) -> Result<String, String> {
    v.get(group)
        .and_then(|g| g.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("tts.json: key '{group}.{key}' missing or not a string"))
}

pub fn parse(src: &str) -> Result<Config, String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("tts.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some("lr.tts/1") {
        return Err("tts.json: key 'schema' missing or not lr.tts/1".into());
    }
    let over = v.get("over_length").and_then(Value::as_str).ok_or("tts.json: key 'over_length' missing or not a string")?;
    let programs = v
        .get("linux_programs")
        .and_then(Value::as_array)
        .ok_or("tts.json: key 'linux_programs' missing or not a list")?
        .iter()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect();
    Ok(Config {
        radius: num(&v, "", "radius")?,
        max_chars: num(&v, "", "max_chars")? as usize,
        trim_over_length: over != "refuse",
        duplicate_window_ms: num(&v, "", "duplicate_window_ms")? as i64,
        queue_max: num(&v, "", "queue_max")? as usize,
        volume_floor: num(&v, "", "volume_floor")?.clamp(0.0, 1.0),
        rate: [num(&v, "rate", "windows")? as i64, num(&v, "rate", "macos")? as i64, num(&v, "rate", "linux")? as i64],
        voices: [text(&v, "voices", "windows")?, text(&v, "voices", "macos")?, text(&v, "voices", "linux")?],
        linux_programs: programs,
    })
}

pub fn config() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).expect("tts.json"))
}

/// Settings the caller owns (HOOK: `speech_enabled` in app/settings.rs).
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub enabled: bool,
}

/// Strip control characters (newlines become spaces), collapse spaces, then apply the length rule.
pub fn clean(raw: &str, c: &Config) -> Option<String> {
    let s: String = raw.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = s.chars().count();
    if s.is_empty() || (n > c.max_chars && !c.trim_over_length) {
        return None;
    }
    Some(s.chars().take(c.max_chars).collect())
}

/// The cleaned text to speak, or None. `recent` holds (text, ms) of lines already spoken, for duplicates.
pub fn should_speak(
    line: &ChatLine,
    listener_world: [f64; 3],
    listener_depth: i64,
    settings: Settings,
    muted: &HashSet<i64>,
    recent: &[(String, i64)],
    now_ms: i64,
) -> Option<String> {
    let c = config();
    if !settings.enabled || line.system || muted.contains(&line.peer_id) {
        return None;
    }
    if !line.audible_from(listener_world, listener_depth, c.radius) {
        return None;
    }
    let t = clean(&line.text, c)?;
    if recent.iter().any(|(r, at)| *r == t && now_ms - *at < c.duplicate_window_ms) {
        return None;
    }
    Some(t)
}

/// 1.0 at the speaker, falling linearly to `floor` at `radius` and beyond.
pub fn volume(distance: f64, radius: f64, floor: f64) -> f64 {
    if radius <= 0.0 {
        return floor.clamp(0.0, 1.0);
    }
    (1.0 - distance.max(0.0) / radius).clamp(0.0, 1.0).max(floor).min(1.0)
}

/// The level a line is spoken at: the distance falloff times Master (0..1) times the SPOKEN CHAT slider (0..100). The
/// speech is an OS process, so it takes this level when the line starts; a line already being spoken keeps it.
pub fn speech_volume(distance: f64, radius: f64, floor: f64, master: f32, spoken_chat_pct: f32) -> f64 {
    let chat = f64::from(spoken_chat_pct.clamp(0.0, 100.0)) / 100.0;
    (volume(distance, radius, floor) * f64::from(master.clamp(0.0, 1.0)) * chat).clamp(0.0, 1.0)
}

/// Mute call for the chat box: flips `peer` in the mute set and returns true when the peer is now muted.
pub fn toggle_mute(muted: &mut HashSet<i64>, peer: i64) -> bool {
    if !muted.remove(&peer) {
        muted.insert(peer);
        return true;
    }
    false
}

/// Set a peer's mute state directly (true = never spoken).
pub fn set_muted(muted: &mut HashSet<i64>, peer: i64, on: bool) {
    if on {
        muted.insert(peer);
    } else {
        muted.remove(&peer);
    }
}

/// One-at-a-time queue; the oldest lines are dropped beyond `max`.
pub struct Queue {
    items: VecDeque<(String, f64)>,
    max: usize,
}

impl Queue {
    pub fn new(max: usize) -> Self {
        Queue { items: VecDeque::new(), max }
    }
    pub fn push(&mut self, text: String, vol: f64) {
        self.items.push_back((text, vol));
        while self.items.len() > self.max {
            self.items.pop_front();
        }
    }
    pub fn pop(&mut self) -> Option<(String, f64)> {
        self.items.pop_front()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

pub fn current_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// A process to run: program, argument vector, and optional stdin payload.
#[derive(Debug, PartialEq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// Build the speaker command. The text is never part of a script string: stdin on Windows, one argv entry elsewhere.
pub fn build(os: Os, program: &str, text: &str, vol: f64, c: &Config) -> Invocation {
    let pct = (vol.clamp(0.0, 1.0) * 100.0).round() as i64;
    match os {
        Os::Windows => {
            let voice = if c.voices[0].is_empty() {
                String::new()
            } else {
                // The voice name comes from data; keep it to safe characters anyway.
                let v: String = c.voices[0].chars().filter(|ch| ch.is_alphanumeric() || *ch == ' ' || *ch == '-').collect();
                format!("$s.SelectVoice('{v}');")
            };
            let script = format!(
                "Add-Type -AssemblyName System.Speech;$s=New-Object System.Speech.Synthesis.SpeechSynthesizer;{voice}$s.Rate={};$s.Volume={pct};$s.Speak([Console]::In.ReadToEnd())",
                c.rate[0]
            );
            Invocation {
                program: program.into(),
                args: vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script],
                stdin: Some(text.to_string()),
            }
        }
        Os::Mac => {
            // `say` has no volume option, so on macOS the speech is at the system level: Master and SPOKEN CHAT do not reach it.
            let mut args = vec!["-r".to_string(), c.rate[1].to_string()];
            if !c.voices[1].is_empty() {
                args.extend(["-v".into(), c.voices[1].clone()]);
            }
            args.extend(["--".into(), text.to_string()]);
            Invocation { program: program.into(), args, stdin: None }
        }
        Os::Linux => {
            let mut args = if program == "spd-say" {
                vec!["-w".to_string(), "-r".into(), (c.rate[2] - 160).clamp(-100, 100).to_string(), "-i".into(), (pct * 2 - 100).to_string()]
            } else {
                vec!["-s".to_string(), c.rate[2].to_string(), "-a".into(), (pct * 2).to_string()]
            };
            if !c.voices[2].is_empty() && program != "spd-say" {
                args.extend(["-v".into(), c.voices[2].clone()]);
            }
            args.extend(["--".into(), text.to_string()]);
            Invocation { program: program.into(), args, stdin: None }
        }
    }
}

fn find_program(os: Os, c: &Config) -> Option<String> {
    match os {
        Os::Windows => Some("powershell".into()),
        Os::Mac => Some("say".into()),
        Os::Linux => c
            .linux_programs
            .iter()
            .find(|p| Command::new(p.as_str()).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok())
            .cloned(),
    }
}

/// The longest one spoken line may take before its process is killed, so one hung program cannot block the queue.
const SPEECH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Wait for `child` up to `timeout`; kill and reap it when the time runs out. True when it was killed.
fn wait_or_kill(child: &mut std::process::Child, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(std::time::Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

fn run(inv: &Invocation) {
    let Ok(mut child) = Command::new(&inv.program)
        .args(&inv.args)
        .stdin(if inv.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let (Some(text), Some(mut si)) = (&inv.stdin, child.stdin.take()) {
        let _ = si.write_all(text.as_bytes());
    }
    wait_or_kill(&mut child, SPEECH_TIMEOUT);
}

/// The speaker: a worker thread speaks one line at a time; `speak_line` never blocks the frame.
pub struct Speaker {
    tx: Option<mpsc::Sender<(String, f64, i64)>>,
    pending: Arc<AtomicUsize>,
    recent: Vec<(String, i64)>,
    /// Senders with a line queued or being spoken: at most one line per sender waits.
    waiting: Arc<Mutex<HashSet<i64>>>,
    /// The local player's peer id; their own lines are never spoken.
    local_peer: Option<i64>,
}

impl Speaker {
    pub fn new() -> Self {
        let os = current_os();
        let pending = Arc::new(AtomicUsize::new(0));
        // the browser has no speech program to run and no thread to run the queue on (shared/task.rs)
        if crate::shared::task::IN_BROWSER {
            return Speaker { tx: None, pending, recent: Vec::new(), waiting: Default::default(), local_peer: None };
        }
        let Some(program) = find_program(os, config()) else {
            eprintln!("tts: no voice available, chat stays silent");
            return Speaker { tx: None, pending, recent: Vec::new(), waiting: Default::default(), local_peer: None };
        };
        let (tx, rx) = mpsc::channel::<(String, f64, i64)>();
        let p2 = pending.clone();
        let waiting: Arc<Mutex<HashSet<i64>>> = Default::default();
        let w2 = waiting.clone();
        std::thread::spawn(move || {
            for (t, v, peer) in rx {
                run(&build(os, &program, &t, v, config()));
                if let Ok(mut w) = w2.lock() {
                    w.remove(&peer);
                }
                p2.fetch_sub(1, Ordering::SeqCst);
            }
        });
        Speaker { tx: Some(tx), pending, recent: Vec::new(), waiting, local_peer: None }
    }

    /// Tell the speaker which peer is the local player (their own lines are skipped).
    pub fn set_local_peer(&mut self, peer: i64) {
        self.local_peer = Some(peer);
    }

    /// Feed one received line (called from net/chat.rs). A line arriving while the queue is full is dropped. `master` and
    /// `spoken_chat_pct` are the Master volume (0..1) and the SPOKEN CHAT slider (0..100) from the settings.
    pub fn speak_line(
        &mut self,
        line: &ChatLine,
        listener_world: [f64; 3],
        listener_depth: i64,
        settings: Settings,
        muted: &HashSet<i64>,
        now_ms: i64,
        master: f32,
        spoken_chat_pct: f32,
    ) {
        let c = config();
        if self.local_peer == Some(line.peer_id) {
            return;
        }
        let Some(t) = should_speak(line, listener_world, listener_depth, settings, muted, &self.recent, now_ms) else { return };
        let Some(tx) = &self.tx else { return };
        if self.pending.load(Ordering::SeqCst) >= c.queue_max {
            return;
        }
        let d: f64 = (0..3).map(|i| (line.world[i] - listener_world[i]).powi(2)).sum::<f64>().sqrt();
        self.recent.retain(|(_, at)| now_ms - *at < c.duplicate_window_ms);
        self.recent.push((t.clone(), now_ms));
        match self.waiting.lock() {
            Ok(mut w) => {
                if !w.insert(line.peer_id) {
                    return;
                }
            }
            Err(_) => return,
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let _ = tx.send((t, speech_volume(d, c.radius, c.volume_floor, master, spoken_chat_pct), line.peer_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(world: [f64; 3], depth: i64, text: &str, peer: i64) -> ChatLine {
        ChatLine {
            peer_id: peer,
            display_name: "A".into(),
            text: text.into(),
            channel: "nearby".into(),
            preset: String::new(),
            ping: false,
            at: [0.0, 0.0],
            world,
            depth,
            system: false,
            target: None,
        }
    }
    const ON: Settings = Settings { enabled: true };

    fn speak(l: &ChatLine, s: Settings, muted: &HashSet<i64>) -> Option<String> {
        should_speak(l, [0.0; 3], 1, s, muted, &[], 0)
    }

    #[test]
    fn in_range_spoken_out_of_range_and_other_depth_not() {
        let r = config().radius;
        let none = HashSet::new();
        assert!(speak(&line([r * 0.5, 0.0, 0.0], 1, "hi", 2), ON, &none).is_some());
        assert!(speak(&line([r * 2.0, 0.0, 0.0], 1, "hi", 2), ON, &none).is_none());
        assert!(speak(&line([1.0, 0.0, 0.0], 2, "hi", 2), ON, &none).is_none());
    }

    #[test]
    fn muted_peer_and_toggle_off_are_silent() {
        let l = line([1.0, 0.0, 0.0], 1, "hi", 7);
        let muted: HashSet<i64> = [7].into();
        assert!(speak(&l, ON, &muted).is_none());
        assert!(speak(&l, Settings { enabled: false }, &HashSet::new()).is_none());
    }

    #[test]
    fn duplicates_suppressed_inside_window_only() {
        let l = line([1.0, 0.0, 0.0], 1, "hi", 2);
        let w = config().duplicate_window_ms;
        let recent = vec![("hi".to_string(), 0)];
        assert!(should_speak(&l, [0.0; 3], 1, ON, &HashSet::new(), &recent, w / 2).is_none());
        assert!(should_speak(&l, [0.0; 3], 1, ON, &HashSet::new(), &recent, w + 1).is_some());
    }

    #[test]
    fn spoken_level_follows_master_and_the_spoken_chat_slider() {
        let (r, f) = (config().radius, config().volume_floor);
        let full = speech_volume(0.0, r, f, 1.0, 100.0);
        assert_eq!(full, 1.0);
        assert!((speech_volume(0.0, r, f, 0.5, 100.0) - 0.5).abs() < 1e-9, "master halves the line");
        assert!((speech_volume(0.0, r, f, 1.0, 45.0) - 0.45).abs() < 1e-9, "the SPOKEN CHAT slider scales it");
        assert_eq!(speech_volume(0.0, r, f, 0.0, 100.0), 0.0, "master 0 silences speech");
        assert_eq!(speech_volume(0.0, r, f, 1.0, 0.0), 0.0, "slider 0 silences speech");
    }

    #[test]
    fn volume_falls_and_respects_floor() {
        let (r, f) = (config().radius, config().volume_floor);
        assert_eq!(volume(0.0, r, f), 1.0);
        assert!(volume(r * 0.3, r, f) > volume(r * 0.6, r, f));
        assert!(volume(r * 5.0, r, f) >= f);
        assert!(volume(r * 0.99, r, f) >= f);
    }

    #[test]
    fn long_line_trimmed_or_refused_per_data() {
        let long = "a".repeat(81);
        let mut c = config().clone();
        c.trim_over_length = true;
        assert_eq!(clean(&long, &c).unwrap().chars().count(), c.max_chars);
        c.trim_over_length = false;
        assert!(clean(&long, &c).is_none());
        assert!(clean(&"a".repeat(c.max_chars), &c).is_some());
    }

    #[test]
    fn hostile_text_is_one_argument_or_stdin() {
        let c = config();
        let evil = "a \"b\" `c` $(x); 'y'\nz\x07";
        let t = clean(evil, c).unwrap();
        assert!(!t.contains('\n') && !t.contains('\x07'));
        for (os, prog) in [(Os::Mac, "say"), (Os::Linux, "espeak")] {
            let inv = build(os, prog, &t, 1.0, c);
            assert_eq!(inv.args.last().unwrap(), &t);
            assert_eq!(inv.args.iter().filter(|a| a.contains("`c`")).count(), 1);
            assert!(inv.stdin.is_none());
        }
        let w = build(Os::Windows, "powershell", &t, 1.0, c);
        assert!(w.args.iter().all(|a| !a.contains("`c`")));
        assert_eq!(w.stdin.as_deref(), Some(t.as_str()));
    }

    #[test]
    fn mute_call_toggles_and_silences_the_peer() {
        let mut m = HashSet::new();
        assert!(toggle_mute(&mut m, 5));
        assert!(speak(&line([1.0, 0.0, 0.0], 1, "hi", 5), ON, &m).is_none());
        assert!(!toggle_mute(&mut m, 5));
        assert!(speak(&line([1.0, 0.0, 0.0], 1, "hi", 5), ON, &m).is_some());
        set_muted(&mut m, 6, true);
        assert!(m.contains(&6));
        set_muted(&mut m, 6, false);
        assert!(m.is_empty());
    }

    #[test]
    fn own_lines_and_second_line_from_a_sender_are_not_queued() {
        let (tx, rx) = mpsc::channel();
        let mut sp = Speaker { tx: Some(tx), pending: Default::default(), recent: Vec::new(), waiting: Default::default(), local_peer: Some(1) };
        let say = |sp: &mut Speaker, peer: i64, t: &str| sp.speak_line(&line([1.0, 0.0, 0.0], 1, t, peer), [0.0; 3], 1, ON, &HashSet::new(), 0, 1.0, 100.0);
        say(&mut sp, 1, "mine");
        say(&mut sp, 2, "one");
        say(&mut sp, 2, "two");
        say(&mut sp, 3, "three");
        let got: Vec<(String, f64, i64)> = rx.try_iter().collect();
        assert_eq!(got.iter().map(|g| g.2).collect::<Vec<_>>(), vec![2, 3]);
        sp.waiting.lock().unwrap().remove(&2);
        say(&mut sp, 2, "four");
        assert_eq!(rx.try_iter().count(), 1, "queued again once the earlier line finished");
    }

    #[test]
    fn queue_drops_oldest() {
        let mut q = Queue::new(2);
        for s in ["a", "b", "c"] {
            q.push(s.into(), 1.0);
        }
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop().unwrap().0, "b");
    }

    #[test]
    fn json_missing_key_reports_its_name() {
        let bad = TEXT.replace("\"queue_max\"", "\"queue_maxx\"");
        assert!(parse(&bad).unwrap_err().contains("queue_max"));
        assert!(parse(TEXT).is_ok());
    }

    #[test]
    fn a_hung_speech_process_is_killed_at_the_deadline() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let Ok(mut child) = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return };
        let t = std::time::Instant::now();
        assert!(wait_or_kill(&mut child, std::time::Duration::from_millis(100)), "timed out and killed");
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
    }
}
