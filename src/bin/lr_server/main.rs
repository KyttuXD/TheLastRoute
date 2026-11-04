//! Rust port of the Last Route Node.js backend (`KyttusLabApp/backend/*.mjs`).
//!
//! One module per original `.mjs` file; `server` is the HTTP router (`server.mjs`) and `main.rs` the `index.js`
//! entry point. Not ported: skins.mjs, city_maps.mjs, ws_relay.mjs, github_*.mjs, deploy_mod_pack.mjs,
//! region_agent.mjs, zip_archive.mjs (see `server/mod.rs` and `main.rs`).
//!
//! `lr_server`: the Rust port of the Last Route backend entry point (`index.js` + `startLastRouteServerFromEnvironment`).
//!
//! Loads `backend/.env` and then `.env` from the working directory (real environment variables win), reads the same
//! `LASTROUTE_*` variables as the Node server, binds HTTP (default TCP 8105) and the UDP gameplay relay (default
//! 1105), and serves until the process is stopped.
//!
//! Not ported (network-bound deployment tooling): the GitHub release mirror (`github_release_sync.mjs`), the source
//! watcher that exits with status 75 (`github_source_watch.mjs`), `deploy_mod_pack.mjs` and `region_agent.mjs`.
//!
//! `lr_server --check` builds the server from the environment, prints the resolved configuration and exits.
//! `lr_server --check-pack <folder>` checks a mod pack folder (see `pack_check`) without starting the server; it
//! exits 0 when the folder is clean and 1 when it has errors.


mod accounts;
mod common;
mod ops;
mod packs;
mod relay;
mod rooms;
mod saves;

// Re-exports keep the flat `crate::module` paths short.
use accounts::{account_service, guest_names};
use common::{upstream, util};
use ops::{crash_reports, env_file, moderation};
use packs::{mod_packs, pack_check, pack_kinds, regions};
use relay::enet_relay;
use rooms::{hunter_id_aliases, public_route, reserved_rooms, room_service};
use saves::game_saves;

mod server;


use crate::server::{options_from_environment, public_update_status, LastRouteServer};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// `--check-pack <folder>`: prints every problem with its file and key; returns the exit status (0 clean, 1 errors).
fn run_pack_check(folder: Option<&str>) -> i32 {
    let Some(folder) = folder else {
        eprintln!("usage: lr_server --check-pack <folder>");
        return 1;
    };
    let folder = Path::new(folder);
    if !folder.is_dir() {
        eprintln!("--check-pack: {} is not a folder", folder.display());
        return 1;
    }
    let check = pack_check::check_pack_folder(folder);
    for warning in &check.warnings {
        println!("warning: {warning}");
    }
    for error in &check.errors {
        println!("error: {error}");
    }
    if check.errors.is_empty() {
        println!("{}: clean", folder.display());
        0
    } else {
        println!("{}: {} error(s)", folder.display(), check.errors.len());
        1
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--check-pack") {
        std::process::exit(run_pack_check(args.get(at + 1).map(String::as_str)));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    for file in [cwd.join("backend").join(".env"), cwd.join(".env")] {
        if let Err(error) = crate::env_file::load_environment_file(&file) {
            util::log_error(&format!("Could not read {}: {error}", file.display()));
        }
    }
    let env: HashMap<String, String> = std::env::vars().collect();
    let config = match options_from_environment(&env, &cwd) {
        Ok(config) => config,
        Err(error) => {
            util::log_error(&format!("Configuration error: {error}"));
            std::process::exit(1);
        }
    };
    let check_only = std::env::args().any(|a| a == "--check");
    let updates = config.options.updates.clone();
    let relay_enabled = config.relay_enabled;
    let upstream = config.options.account_upstream.as_ref().map(|upstream| upstream.authority().to_string());
    let server = Arc::new(LastRouteServer::new(config.options));
    let address = format!("{}:{}", config.host, config.http_port);
    util::log_info(&format!("Last Route backend (rust) listening on http://{address}"));
    match upstream {
        Some(authority) => util::log_info(&format!("Accounts and saves: forwarded to {authority} (accounts.json is not used)")),
        None => util::log_info(&format!("Account data: {}", config.account_store_path.display())),
    }
    if let Some(status) = public_update_status(&updates).as_object() {
        for (channel, s) in status {
            util::log_info(&format!("Published {channel} update: {} @ {}", s["version"], s["revision"]));
        }
    }
    if check_only {
        util::log_info(&format!(
            "relay: {} (UDP {})",
            if relay_enabled { "enabled" } else { "disabled" },
            config.relay_port
        ));
        return;
    }
    if relay_enabled {
        if let Err(error) = server.start_enet_relay(&config.host, config.relay_port) {
            util::log_error(&format!("ENet gameplay relay error: {error}"));
        }
    }
    let http = match tiny_http::Server::http(&address) {
        Ok(http) => Arc::new(http),
        Err(error) => {
            util::log_error(&format!("Could not listen on {address}: {error}"));
            std::process::exit(1);
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    for thread in server.serve(http, 8, stop) {
        let _ = thread.join();
    }
    server.close();
}
