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

use lr_server::server::{options_from_environment, public_update_status, LastRouteServer};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    for file in [cwd.join("backend").join(".env"), cwd.join(".env")] {
        if let Err(error) = lr_server::env_file::load_environment_file(&file) {
            eprintln!("Could not read {}: {error}", file.display());
        }
    }
    let env: HashMap<String, String> = std::env::vars().collect();
    let config = options_from_environment(&env, &cwd);
    let check_only = std::env::args().any(|a| a == "--check");
    let updates = config.options.updates.clone();
    let relay_enabled = config.relay_enabled;
    let server = Arc::new(LastRouteServer::new(config.options));
    let address = format!("{}:{}", config.host, config.http_port);
    println!("Last Route backend (rust) listening on http://{address}");
    println!("Account data: {}", config.account_store_path.display());
    if let Some(status) = public_update_status(&updates).as_object() {
        for (channel, s) in status {
            println!("Published {channel} update: {} @ {}", s["version"], s["revision"]);
        }
    }
    if check_only {
        println!("relay: {} (UDP {})", if relay_enabled { "enabled" } else { "disabled" }, config.relay_port);
        return;
    }
    if relay_enabled {
        if let Err(error) = server.start_enet_relay(&config.host, config.relay_port) {
            eprintln!("ENet gameplay relay error: {error}");
        }
    }
    let http = match tiny_http::Server::http(&address) {
        Ok(http) => Arc::new(http),
        Err(error) => {
            eprintln!("Could not listen on {address}: {error}");
            std::process::exit(1);
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    for thread in server.serve(http, 8, stop) {
        let _ = thread.join();
    }
    server.close();
}
