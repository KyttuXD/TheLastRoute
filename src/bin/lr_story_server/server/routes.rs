//! The request routing of the story server: health, accounts and guest names (the same routes and payloads as lr_server),
//! the room API, and the rate limits.

use super::*;
use crate::util::{str_or_empty, truthy};
use serde_json::{json, Value};

fn unknown_endpoint() -> Response {
    reply((404, json!({ "ok": false, "code": "UNKNOWN_ENDPOINT", "error": "Unknown endpoint" })))
}

/// The code of a `GET /api/rooms/<code>` path.
fn room_code_of(path: &str) -> Option<String> {
    path.strip_prefix("/api/rooms/").filter(|code| !code.is_empty() && !code.contains('/')).map(str::to_string)
}

/// The JSON body of a POST: an empty body is `{}`; anything else must be a JSON object.
fn read_json(request: &Request) -> Result<Value, Response> {
    if request.body.is_empty() {
        return Ok(json!({}));
    }
    if request.body.len() > MAX_BODY_BYTES {
        return Err(json_response(413, &json!({ "ok": false, "code": "BODY_TOO_LARGE", "error": "The request body is too large" }), &[]));
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => Err(json_response(400, &json!({ "ok": false, "code": "BAD_JSON", "error": "The body must be a JSON object" }), &[])),
    }
}

impl StoryServer {
    /// The response to one request (the body of the HTTP callback).
    pub fn route(&self, request: &Request) -> Response {
        let method = request.method.as_str();
        let path = request.path.as_str();
        if method == "GET" && path == "/api/health" {
            return self.health();
        }
        if request.body_overflow {
            return json_response(413, &json!({ "ok": false, "code": "BODY_TOO_LARGE", "error": "The request body is too large" }), &[]);
        }
        if !path.starts_with("/api/") {
            return unknown_endpoint();
        }
        let retry_after = self.rate_limit(request, path);
        if retry_after > 0 {
            let headers = [("retry-after", retry_after.to_string())];
            return json_response(429, &json!({ "ok": false, "code": "RATE_LIMITED", "error": "Too many requests; try again shortly" }), &headers);
        }
        if method == "GET" {
            return match room_code_of(path) {
                Some(code) => reply(self.lock().rooms.get(&code)),
                None => unknown_endpoint(),
            };
        }
        if method != "POST" {
            return unknown_endpoint();
        }
        let body = match read_json(request) {
            Ok(body) => body,
            Err(refused) => return refused,
        };
        match path {
            "/api/guest/name" => reply(self.lock().guests.issue(&body)),
            "/api/account/register" => {
                let allow = self.owner_session(&body);
                reply(self.accounts.register(&body, allow))
            }
            "/api/account/login" => reply(self.accounts.login(&body)),
            "/api/account/session" => reply(self.accounts.current_session(&body)),
            "/api/account/logout" => reply(self.accounts.logout(&body)),
            "/api/account/password" => reply(self.accounts.change_password(&body)),
            "/api/account/rename" => {
                let allow = self.owner_session(&body);
                reply(self.accounts.rename(&body, allow))
            }
            "/api/rooms/create" | "/api/rooms/join" => {
                let identity = match self.identity(&body) {
                    Ok(name) => name,
                    Err(refused) => return refused,
                };
                let result = if path == "/api/rooms/create" {
                    self.lock().rooms.create(&identity, &body)
                } else {
                    self.lock().rooms.join(&identity, &body)
                };
                reply(result)
            }
            "/api/rooms/leave" => reply(self.lock().rooms.leave(&body)),
            "/api/rooms/ready" => reply(self.lock().rooms.set_ready(&body)),
            "/api/rooms/pass-host" => reply(self.lock().rooms.pass_host(&body)),
            "/api/rooms/vote" => reply(self.lock().rooms.vote(&body)),
            "/api/rooms/state" => reply(self.lock().rooms.write_state(&body)),
            _ => unknown_endpoint(),
        }
    }

    fn health(&self) -> Response {
        let rooms = self.lock().rooms.room_count();
        let body = json!({
            "ok": true,
            "service": "Last Route story server",
            "health": "/api/health",
            "rooms": rooms,
            "relay": self.relay_running(),
        });
        json_response(200, &body, &[])
    }

    /// Whether the request's session belongs to a full admin owner (who may register or rename into reserved names).
    fn owner_session(&self, body: &Value) -> bool {
        if !truthy(body.get("sessionToken")) {
            return false;
        }
        let token = str_or_empty(body.get("sessionToken"));
        self.accounts
            .session_admin_identity(&token)
            .map(|identity| identity.get("adminOwner") == Some(&Value::Bool(true)))
            .unwrap_or(false)
    }

    /// The identity a room join or create acts as: an account session (`sessionToken`), or a guest name whose live lease
    /// `guestToken` proves (`guestName`, `guestToken`, from POST /api/guest/name).
    fn identity(&self, body: &Value) -> Result<String, Response> {
        if truthy(body.get("sessionToken")) {
            let token = str_or_empty(body.get("sessionToken"));
            return self.accounts.session_username(&token).ok_or_else(|| {
                reply((401, json!({ "ok": false, "code": "SESSION_INVALID", "error": "Your sign-in has ended; sign in again" })))
            });
        }
        let name = str_or_empty(body.get("guestName")).trim().to_uppercase();
        let token = str_or_empty(body.get("guestToken"));
        if self.lock().guests.token_matches(&name, &token) {
            return Ok(name);
        }
        Err(reply((401, json!({ "ok": false, "code": "IDENTITY_REQUIRED", "error": "Sign in, or take a guest name, to join a room" }))))
    }

    /// Counts a request against its window. Returns 0 when it may proceed, else the seconds to wait.
    fn rate_limit(&self, request: &Request, pathname: &str) -> i64 {
        let sensitive = ACCOUNT_RATE_LIMIT_PATHS.contains(&pathname);
        let maximum = if sensitive { self.options.account_rate_limit_max } else { self.options.api_rate_limit_max }.max(1);
        let window = self.options.rate_limit_window_ms.max(1000);
        let timestamp = self.now();
        let address = requester_address(request, self.options.trust_proxy);
        let address = if address.is_empty() { "unknown".to_string() } else { address };
        let key = format!("{}|{address}", if sensitive { "account" } else { "api" });
        let mut state = self.lock();
        let rate = state.request_rates.entry(key).or_insert((0, timestamp + window));
        if timestamp >= rate.1 {
            *rate = (0, timestamp + window);
        }
        rate.0 += 1;
        if rate.0 <= maximum {
            0
        } else {
            (((rate.1 - timestamp) as f64) / 1000.0).ceil().max(1.0) as i64
        }
    }
}
