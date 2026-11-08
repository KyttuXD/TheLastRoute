//! Account tests that are not tied to one method: session lifetime and sign-out, the password change and the account
//! deletion flows, and loading an `accounts.json` the Node server wrote (`testdata/node_accounts.json`).

use crate::account_service::{AccountService, AccountServiceOptions};
use crate::util::{hex, manual_clock, random_bytes, Clock, ManualClock};
use serde_json::{json, Value};
use std::path::PathBuf;

const PASSWORD: &str = "correct horse battery";
const NODE_FIXTURE: &str = include_str!("testdata/node_accounts.json");
/// The passwords of the Node fixture accounts (the fixture stores only their scrypt records).
const ADA_PASSWORD: &str = "fixture password 1";
const BOB_PASSWORD: &str = "another fixture pw";
const START: i64 = 1_700_000_000_000;

struct Rig {
    root: PathBuf,
    store: PathBuf,
    clock: Clock,
    time: ManualClock,
}

impl Rig {
    fn new() -> Rig {
        let root = std::env::temp_dir().join(format!("account-tests-{}", hex(&random_bytes(6))));
        std::fs::create_dir_all(&root).unwrap();
        let (clock, time) = manual_clock(START);
        Rig { store: root.join("accounts.json"), root, clock, time }
    }

    fn open(&self, tweak: impl FnOnce(&mut AccountServiceOptions)) -> AccountService {
        let mut options = AccountServiceOptions { store_path: Some(self.store.clone()), now: self.clock.clone(), ..Default::default() };
        tweak(&mut options);
        AccountService::new(options)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn token_of(reply: &(u16, Value)) -> String {
    assert_eq!(reply.0, 200, "{}", reply.1);
    reply.1["sessionToken"].as_str().expect("a session token").to_string()
}

fn sign_up(accounts: &AccountService, name: &str) -> String {
    token_of(&accounts.register(&json!({ "username": name, "password": PASSWORD }), false))
}

fn code(reply: &(u16, Value)) -> String {
    reply.1["code"].as_str().unwrap_or_default().to_string()
}

#[test]
fn a_session_dies_at_the_configured_lifetime_and_stays_dead_after_a_restart() {
    let rig = Rig::new();
    let accounts = rig.open(|o| o.session_ttl_ms = 60_000);
    let token = sign_up(&accounts, "SHORTLIVED");
    rig.time.advance(59_000);
    assert_eq!(accounts.session_username(&token).as_deref(), Some("SHORTLIVED"));
    rig.time.advance(2_000);
    assert_eq!(accounts.session_username(&token), None, "the token is refused once the lifetime has passed");
    assert_eq!(accounts.current_session(&json!({ "sessionToken": token })).0, 401);
    // the file no longer holds it, so a restart cannot bring it back
    let reopened = rig.open(|o| o.session_ttl_ms = 60_000);
    assert_eq!(reopened.session_username(&token), None);
    // a longer lifetime is a different setting, not a different behaviour
    let long = rig.open(|o| o.session_ttl_ms = 10 * 60_000);
    let fresh = token_of(&long.login(&json!({ "username": "shortlived", "password": PASSWORD })));
    rig.time.advance(5 * 60_000);
    assert_eq!(long.session_username(&fresh).as_deref(), Some("SHORTLIVED"));
}

#[test]
fn signing_out_revokes_that_token_everywhere_and_only_that_token() {
    let rig = Rig::new();
    let accounts = rig.open(|_| {});
    let pc = sign_up(&accounts, "TWO_PCS");
    let laptop = token_of(&accounts.login(&json!({ "username": "two_pcs", "password": PASSWORD })));
    assert_ne!(pc, laptop);
    assert_eq!(accounts.logout(&json!({ "sessionToken": pc })).0, 200);
    assert_eq!(accounts.session_username(&pc), None, "the signed-out token is refused");
    assert_eq!(accounts.load_progress(&json!({ "sessionToken": pc })).0, 401);
    assert_eq!(accounts.save(&json!({ "sessionToken": pc, "progress": { "floor": 1 } })).0, 401);
    assert_eq!(accounts.session_username(&laptop).as_deref(), Some("TWO_PCS"), "the other device stays signed in");
    let reopened = rig.open(|_| {});
    assert_eq!(reopened.session_username(&pc), None, "a restart does not bring the token back");
    assert_eq!(reopened.session_username(&laptop).as_deref(), Some("TWO_PCS"));
}

#[test]
fn a_password_change_needs_the_current_password_and_signs_other_devices_out() {
    let rig = Rig::new();
    let accounts = rig.open(|_| {});
    let pc = sign_up(&accounts, "CHANGER");
    let laptop = token_of(&accounts.login(&json!({ "username": "changer", "password": PASSWORD })));
    let change = |current: Value, new: &str| {
        let mut body = json!({ "sessionToken": pc, "password": new });
        if !current.is_null() {
            body["currentPassword"] = current;
        }
        accounts.change_password(&body)
    };
    assert_eq!(code(&change(Value::Null, "brand new pass")), "CURRENT_PASSWORD_REQUIRED");
    assert_eq!(change(json!(""), "brand new pass").0, 400);
    let wrong = change(json!("not my password"), "brand new pass");
    assert_eq!((wrong.0, code(&wrong)), (403, "WRONG_PASSWORD".to_string()));
    assert_eq!(accounts.login(&json!({ "username": "CHANGER", "password": PASSWORD })).0, 200, "nothing changed");
    assert_eq!(change(json!(PASSWORD), "tiny").0, 400, "the new password must still be a valid one");
    // without a session there is no change either
    let anonymous = accounts.change_password(&json!({ "currentPassword": PASSWORD, "password": "brand new pass" }));
    assert_eq!(anonymous.0, 401);

    let done = change(json!(PASSWORD), "brand new pass");
    assert_eq!(done.0, 200, "{}", done.1);
    assert_eq!(accounts.login(&json!({ "username": "CHANGER", "password": PASSWORD })).0, 401, "the old password is gone");
    assert_eq!(accounts.login(&json!({ "username": "CHANGER", "password": "brand new pass" })).0, 200);
    assert_eq!(accounts.session_username(&pc).as_deref(), Some("CHANGER"), "the device that asked stays in");
    assert_eq!(accounts.session_username(&laptop), None, "every other device is signed out");
}

#[test]
fn deleting_an_account_takes_the_password_then_a_one_time_confirm_token() {
    let rig = Rig::new();
    let accounts = rig.open(|o| o.delete_confirm_ttl_ms = 60_000);
    let token = sign_up(&accounts, "LEAVING");
    let other = sign_up(&accounts, "STAYING");
    let ask = |extra: Value| {
        let mut body = json!({ "sessionToken": token, "currentPassword": PASSWORD });
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            body[key] = value;
        }
        accounts.delete_account(&body)
    };
    // no session, a wrong password and a missing password all stop at the door
    assert_eq!(accounts.delete_account(&json!({ "currentPassword": PASSWORD })).0, 401);
    assert_eq!(code(&ask(json!({ "currentPassword": "nope nope" }))), "WRONG_PASSWORD");
    assert_eq!(code(&ask(json!({ "currentPassword": "" }))), "WRONG_PASSWORD");

    // step one deletes nothing and hands out a token
    let first = ask(json!({}));
    assert_eq!(first.0, 200, "{}", first.1);
    assert_eq!(first.1["confirmRequired"], json!(true));
    assert_eq!(first.1["expiresInSeconds"], json!(60));
    let confirm = first.1["confirmToken"].as_str().unwrap().to_string();
    assert!(accounts.account_exists("LEAVING"));
    assert_eq!(accounts.session_username(&token).as_deref(), Some("LEAVING"));

    // a wrong token, another account's token and a stolen session without the password do nothing
    assert_eq!(code(&ask(json!({ "confirmToken": "00000000" }))), "BAD_CONFIRM_TOKEN");
    let theirs = accounts.delete_account(&json!({ "sessionToken": other, "currentPassword": PASSWORD, "confirmToken": confirm }));
    assert_eq!(code(&theirs), "BAD_CONFIRM_TOKEN");
    assert!(accounts.account_exists("STAYING"));
    assert_eq!(code(&ask(json!({ "currentPassword": "nope nope", "confirmToken": confirm }))), "WRONG_PASSWORD");
    assert!(accounts.account_exists("LEAVING"));

    // an expired token is refused, and asking again issues a fresh one that works
    rig.time.advance(61_000);
    assert_eq!(code(&ask(json!({ "confirmToken": confirm }))), "BAD_CONFIRM_TOKEN");
    let second = ask(json!({}));
    let fresh = second.1["confirmToken"].as_str().unwrap().to_string();
    assert_ne!(fresh, confirm);
    let done = ask(json!({ "confirmToken": fresh }));
    assert_eq!((done.0, done.1["deleted"].clone(), done.1["username"].clone()), (200, json!(true), json!("LEAVING")));

    assert!(!accounts.account_exists("LEAVING"));
    assert_eq!(accounts.session_username(&token), None, "its sessions end with it");
    assert_eq!(accounts.login(&json!({ "username": "LEAVING", "password": PASSWORD })).0, 401);
    assert_eq!(code(&ask(json!({ "confirmToken": fresh }))), "NO_SESSION", "the token cannot be used twice");
    assert_eq!(accounts.session_username(&other).as_deref(), Some("STAYING"));
    // the name is free again, and the file on disk agrees
    assert!(!std::fs::read_to_string(&rig.store).unwrap().contains("\"LEAVING\""));
    assert_eq!(accounts.register(&json!({ "username": "leaving", "password": PASSWORD }), false).0, 200);
}

#[test]
fn the_built_in_owner_accounts_cannot_be_deleted_from_the_account_panel() {
    let rig = Rig::new();
    let accounts = rig.open(|_| {});
    let owner = token_of(&accounts.register(&json!({ "username": "KYTTU", "password": PASSWORD }), true));
    let refused = accounts.delete_account(&json!({ "sessionToken": owner, "currentPassword": PASSWORD }));
    assert_eq!((refused.0, code(&refused)), (403, "OWNER_PROTECTED".to_string()));
    assert!(accounts.account_exists("KYTTU"));
}

// ---------------------------------------------------------------------------------------------------------------
// Migration of `accounts.json` written by the Node server.

fn write_node_fixture(rig: &Rig) {
    std::fs::write(&rig.store, NODE_FIXTURE).unwrap();
}

#[test]
fn a_node_accounts_file_loads_without_loss_and_names_are_canonical_upper_case() {
    let rig = Rig::new();
    write_node_fixture(&rig);
    let accounts = rig.open(|_| {});
    assert!(accounts.load_warning().is_none());
    // the lower-case Node name is the upper-case account, however it is typed
    for typed in ["ada", "ADA", " Ada "] {
        assert!(accounts.account_exists(typed), "{typed}");
    }
    let login = accounts.login(&json!({ "username": "ada", "password": ADA_PASSWORD }));
    assert_eq!(login.0, 200, "{}", login.1);
    assert_eq!(login.1["username"], json!("ADA"));
    assert_eq!(login.1["progress"], json!({ "floor": 3, "note": "saved by the Node server" }), "progress is not lost");
    assert_eq!(accounts.login(&json!({ "username": "ADA", "password": BOB_PASSWORD })).0, 401);
    let bob = token_of(&accounts.login(&json!({ "username": "bob_runner", "password": BOB_PASSWORD })));
    assert_eq!(accounts.session_admin_identity(&bob).unwrap()["adminPermissions"], json!(["world"]), "admin permissions are kept");
    // the stored file keeps every account under its canonical name
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&rig.store).unwrap()).unwrap();
    let names: Vec<&str> = raw["accounts"].as_array().unwrap().iter().map(|a| a["username"].as_str().unwrap()).collect();
    assert_eq!(names, ["ADA", "BOB_RUNNER"]);
}

#[test]
fn the_old_session_format_signs_in_once_and_is_rewritten_as_a_hash() {
    let rig = Rig::new();
    write_node_fixture(&rig);
    let accounts = rig.open(|_| {});
    // a plaintext Node session still works, and so does a hashed one; an expired one and one of a missing account do not
    assert_eq!(accounts.session_username("legacy-plain-session-for-ada").as_deref(), Some("ADA"));
    assert_eq!(accounts.session_username("hashed-session-for-bob").as_deref(), Some("BOB_RUNNER"));
    assert_eq!(accounts.session_username("expired-session-for-bob"), None);
    assert_eq!(accounts.session_username("orphan-session-token"), None);
    // the file was rewritten at load: no plain token is left on disk, the live sessions are kept as hashes
    let text = std::fs::read_to_string(&rig.store).unwrap();
    assert!(!text.contains("legacy-plain-session-for-ada") && !text.contains("orphan-session-token"), "{text}");
    let raw: Value = serde_json::from_str(&text).unwrap();
    let sessions = raw["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|s| s.get("token").is_none() && s["tokenHash"].as_str().unwrap().len() == 64));
    // it keeps working after another restart, from the hash alone
    let again = rig.open(|_| {});
    assert_eq!(again.session_username("legacy-plain-session-for-ada").as_deref(), Some("ADA"));
    // and signing out ends it for good
    assert_eq!(again.logout(&json!({ "sessionToken": "legacy-plain-session-for-ada" })).0, 200);
    assert_eq!(rig.open(|_| {}).session_username("legacy-plain-session-for-ada"), None);
}
