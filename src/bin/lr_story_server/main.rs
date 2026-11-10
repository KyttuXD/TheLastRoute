//! `lr_story_server`: the room server for multiplayer Last Route story packs. A separate process from `lr_server`: it
//! shares no state and no data folder with it.
//!
//! It serves the HTTP API (default TCP 8307) and the UDP relay (default UDP 1307, the LRR1 frames of `relay/enet_relay`).
//! Accounts and guest names work as in lr_server; rooms, votes and the shared party save are in `rooms/` and `saves/`.
//!
//! Environment (see `config.rs`; a value that is set but not valid stops the server, nothing falls back silently):
//!   LR_STORY_HTTP_PORT, LR_STORY_RELAY_PORT   ports 1-65535 (defaults 8205 and 1205)
//!   LR_STORY_RELAY_ENABLED                    true or false (default true)
//!   LR_STORY_HOST                             the address both sockets bind (default 0.0.0.0)
//!   LR_STORY_DATA_DIR                         accounts, guest names, party saves (default ./.data/lr_story_server)
//!   LR_STORY_PACKS_DIR                        one folder per story pack id (default ./mods)
//!   LR_STORY_TRUST_PROXY                      read X-Forwarded-For (default false; behind a reverse proxy only)
//!   LR_STORY_RATE_LIMIT_WINDOW_SECONDS, LR_STORY_API_RATE_LIMIT, LR_STORY_ACCOUNT_RATE_LIMIT
//!
//! `lr_story_server --check` reads the configuration, prints it and exits without binding anything.

#![allow(dead_code)]

mod accounts;
mod common;
mod config;
mod packs;
mod relay;
mod rooms;
mod saves;
mod server;

// Re-exports keep the flat `crate::module` paths short, as in lr_server.
use accounts::{account_service, guest_names};
use common::{scrub, upstream, util};
use relay::enet_relay;

use server::{ServerOptions, StoryServer};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn print_configuration(config: &config::StoryConfig) {
    util::log_info(&format!("LR story server: HTTP {}:{}", config.host, config.http_port));
    if config.relay_enabled {
        util::log_info(&format!("LR story server: relay UDP {}:{}", config.host, config.relay_port));
    } else {
        util::log_info("LR story server: relay disabled (LR_STORY_RELAY_ENABLED)");
    }
    util::log_info(&format!("LR story server: data folder {}", config.data_dir.display()));
    util::log_info(&format!("LR story server: packs folder {}", config.packs_dir.display()));
    match &config.account_upstream {
        Some(up) => util::log_info(&format!("LR story server: accounts from the website at http://{}", up.authority())),
        None => util::log_info("LR story server: accounts in its own data folder (accounts.json)"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let env: HashMap<String, String> = std::env::vars().collect();
    let config = match config::from_env(&env, &cwd) {
        Ok(config) => config,
        Err(error) => {
            util::log_error(&format!("Configuration error: {error}"));
            std::process::exit(1);
        }
    };
    if args.iter().any(|a| a == "--check") {
        print_configuration(&config);
        util::log_info("LR story server: configuration is valid");
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&config.data_dir) {
        util::log_error(&format!("Could not create the data folder {}: {error}", config.data_dir.display()));
        std::process::exit(1);
    }
    print_configuration(&config);
    let server = Arc::new(StoryServer::new(ServerOptions::from_config(&config)));
    if config.relay_enabled {
        if let Err(error) = server.start_relay(&config.host, config.relay_port) {
            util::log_error(&format!("Could not start the relay on UDP {}: {error}", config.relay_port));
            std::process::exit(1);
        }
    }
    let address = format!("{}:{}", config.host, config.http_port);
    let http = match tiny_http::Server::http(&address) {
        Ok(http) => Arc::new(http),
        Err(error) => {
            util::log_error(&format!("Could not listen on {address}: {error}"));
            std::process::exit(1);
        }
    };
    util::log_info(&format!("LR story server listening on http://{address}"));
    let stop = Arc::new(AtomicBool::new(false));
    for thread in server.serve(http, 8, stop) {
        let _ = thread.join();
    }
    server.close();
}
