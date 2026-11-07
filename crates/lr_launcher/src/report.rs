//! `--check` and `--dry-run`: say what is installed, what the site offers and what a run would do. Nothing is written
//! (no download, no state change, no log file) and nothing is started.

use crate::config::{Config, Mode};
use crate::flow::{self, Checked, Fail, Kind, Offer, Outcome, EXIT_CHECK_UPDATE_AVAILABLE, EXIT_OFFLINE};
use crate::log::Log;
use lr_update::install::join_rel;
use lr_update::state;
use std::sync::atomic::AtomicBool;

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

pub fn report(cfg: &Config) -> Outcome {
    let log = Log::open(None, false);
    let (st, damaged) = state::load(&cfg.dir);
    let mut out: Vec<String> = Vec::new();
    out.push(format!(
        "lr_launcher {}: platform {}, channel {}",
        flow::LAUNCHER_VERSION,
        cfg.platform,
        if cfg.channel.is_empty() { "main" } else { &cfg.channel }
    ));
    out.push(format!("site: {}", cfg.base_url));
    out.push(format!("folder: {}{}", cfg.dir.display(), if damaged { " (state.json is damaged; it would be rebuilt)" } else { "" }));
    let installed = st.current.and_then(|b| flow::usable(cfg, b, &log).map(|m| (b, m)));
    match (&st.current, &installed) {
        (Some(b), Some(_)) => out.push(format!("installed: build {b} (previous {:?}, marked bad {:?}, lowest accepted {})", st.previous, st.bad, st.floor)),
        (Some(b), None) => out.push(format!("installed: build {b} is damaged or untrusted and would be dropped")),
        (None, _) => out.push(format!("installed: nothing (marked bad {:?}, lowest accepted {})", st.bad, st.floor)),
    }
    let cancel = AtomicBool::new(false);
    let mut code = 0;
    let mut would_install = false;
    match flow::look(cfg, &st, &log, &cancel) {
        Ok(Offer::Available { manifest: m, .. }) => {
            out.push(format!(
                "available: build {} (version {}, revision {}), {} to download, {} installed{}",
                m.build,
                m.version,
                m.revision.chars().take(9).collect::<String>(),
                mb(m.total_download()),
                mb(m.total_size()),
                if m.notes.is_empty() { String::new() } else { format!(": {}", m.notes) }
            ));
            out.push("decision: a newer build is available".into());
            code = EXIT_CHECK_UPDATE_AVAILABLE;
            would_install = true;
        }
        Ok(Offer::Nothing(Checked::UpToDate(b))) => out.push(format!("available: build {b}\ndecision: up to date")),
        Ok(Offer::Nothing(Checked::Skipped(why))) => out.push(format!("decision: nothing to install ({why})")),
        Ok(Offer::Nothing(Checked::Installed(_))) => {}
        Err(Fail::NoNative) => {
            out.push("decision: no native build is published for this system; the web version would open".into());
            code = 3;
        }
        Err(Fail::Offline(w)) => {
            out.push(format!(
                "decision: the site cannot be reached ({w}); {}",
                if installed.is_some() { "the installed build would start" } else { "nothing can start" }
            ));
            code = EXIT_OFFLINE;
        }
        Err(f) => {
            out.push(format!(
                "decision: refused ({}); {}",
                f.text(),
                if installed.is_some() { "the installed build would start" } else { "the web version would open" }
            ));
            code = 4;
        }
    }
    if cfg.mode == Mode::DryRun {
        if would_install {
            out.push("would: download and install that build into the folder above".into());
        }
        match (&installed, would_install) {
            (Some((b, m)), _) => out.push(format!(
                "would: start build {b}: {} {:?}{}",
                join_rel(&state::build_dir(&cfg.dir, *b), &m.run).display(),
                cfg.game_args,
                if would_install && (cfg.update_first || cfg.force) { " (after the update)" } else { "" }
            )),
            (None, true) => out.push("would: then start the new build".into()),
            (None, false) => out.push(format!("would: open {} in the browser", cfg.web_url)),
        }
        out.push("dry run: nothing was changed".into());
    }
    Outcome { kind: Kind::Report, exit_code: code, build: installed.map(|(b, _)| b), message: out.join("\n"), opened: None }
}
