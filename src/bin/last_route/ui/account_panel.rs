//! The ACCOUNT panel's logic (TODO 222 / 843 sign-up and sign-in, 224 sign-out, 227 password change and account deletion
//! with a confirm step, 857 reserved-room owner tools, 851/853 cloud copies ride on the session it holds).
//!
//! [`Panel`] is a state machine with no drawing: the menu calls the setters while the player types, [`Panel::submit`] /
//! [`Panel::confirm`] return the [`Request`] to run, [`spawn`] runs it on a worker thread through `net/api.rs`, and the
//! finished result goes back through [`Panel::finish`]. While a request is out the panel is busy (`pending_*` texts) and
//! refuses a second one. Names are 3-24 characters A-Z 0-9 _ and are sent canonical upper case. Texts are
//! `data/startup.json` (`account_panel`) and `data/net/errors.json`.

use crate::app::startup;
use crate::net::api::{self, AccountReply};
use crate::net::config::BackendConfig;
use crate::net::store::StoredSession;
use crate::net::{NetError, Pending};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    SignIn,
    Create,
    /// Signed in: change password, delete account, sign out.
    Settings,
}

/// What the panel is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    None,
    SignIn,
    Create,
    Password,
    Delete,
}

/// A step that needs a second press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    None,
    Password,
    Delete,
}

/// A reserved-room owner call (TODO 857).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerCall {
    Reserve { code: String },
    Kick { code: String, player: String },
    Rename { code: String, title: String },
}

/// One call for the worker thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    SignIn { name: String, password: String },
    Create { name: String, password: String },
    ChangePassword { token: String, password: String },
    DeleteAccount { token: String, password: String },
    SignOut { token: String },
    Owner { token: String, call: OwnerCall },
}

/// What a finished call gives back.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Account(AccountReply),
    PasswordChanged,
    Deleted,
    SignedOut,
    OwnerDone,
}

fn text(key: &str) -> String {
    startup::data().get("account_panel").and_then(|p| p.get(key)).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub mode: Mode,
    pub name: String,
    pub password: String,
    pub new_password: String,
    /// The line under the form (upper case).
    pub message: String,
    pub busy: Busy,
    pub confirm: Confirm,
    /// The signed-in session; empty name = guest.
    pub session: StoredSession,
    pub admin: bool,
}

impl Panel {
    pub fn new(session: StoredSession) -> Panel {
        let mode = if session.session_token.is_empty() { Mode::SignIn } else { Mode::Settings };
        Panel { mode, name: String::new(), password: String::new(), new_password: String::new(), message: String::new(), busy: Busy::None, confirm: Confirm::None, session, admin: false }
    }

    pub fn signed_in(&self) -> bool {
        !self.session.session_token.is_empty()
    }

    /// The text of the busy state, empty when idle.
    pub fn pending_text(&self) -> String {
        match self.busy {
            Busy::None => String::new(),
            Busy::SignIn => text("pending_signin"),
            Busy::Create => text("pending_create"),
            Busy::Password => text("pending_password"),
            Busy::Delete => text("pending_delete"),
        }
    }

    fn refuse_if_unavailable(&mut self, cfg: &BackendConfig, debug_build: bool) -> bool {
        if cfg.offline {
            self.message = api::describe_error(&NetError::Offline);
            return true;
        }
        if !cfg.accounts_available(debug_build) || !cfg.origin_transport_ok(debug_build) {
            self.message = cfg.account_transport_warning(debug_build).to_string();
            if self.message.is_empty() {
                self.message = api::error_texts().text("http_insecure");
            }
            return true;
        }
        false
    }

    /// The sign-in / create button. Validates locally, then returns the request (the panel is busy until `finish`).
    pub fn submit(&mut self, cfg: &BackendConfig, debug_build: bool) -> Option<Request> {
        if self.busy != Busy::None || self.mode == Mode::Settings {
            return None;
        }
        self.message.clear();
        if self.refuse_if_unavailable(cfg, debug_build) {
            return None;
        }
        let name = match api::validate_account_name(&self.name) {
            Ok(n) => n,
            Err(e) => {
                self.message = api::describe_error(&e);
                return None;
            }
        };
        if self.password.is_empty() {
            self.message = api::error_texts().text("password_empty");
            return None;
        }
        let password = self.password.clone();
        if self.mode == Mode::Create {
            self.busy = Busy::Create;
            Some(Request::Create { name, password })
        } else {
            self.busy = Busy::SignIn;
            Some(Request::SignIn { name, password })
        }
    }

    /// First press of CHANGE PASSWORD or DELETE ACCOUNT: asks for the confirm step, sends nothing.
    pub fn ask_confirm(&mut self, what: Confirm) {
        if self.busy != Busy::None || !self.signed_in() {
            return;
        }
        self.confirm = what;
        self.message = match what {
            Confirm::Password => text("confirm_password"),
            Confirm::Delete => text("confirm_delete"),
            Confirm::None => String::new(),
        };
    }

    pub fn cancel_confirm(&mut self) {
        self.confirm = Confirm::None;
        self.message.clear();
    }

    /// The CONFIRM press: only now is the call made.
    pub fn confirm(&mut self, cfg: &BackendConfig, debug_build: bool) -> Option<Request> {
        if self.busy != Busy::None || !self.signed_in() || self.confirm == Confirm::None {
            return None;
        }
        if self.refuse_if_unavailable(cfg, debug_build) {
            return None;
        }
        let token = self.session.session_token.clone();
        match self.confirm {
            Confirm::Password => {
                if self.new_password.is_empty() {
                    self.message = api::error_texts().text("password_empty");
                    return None;
                }
                self.confirm = Confirm::None;
                self.busy = Busy::Password;
                Some(Request::ChangePassword { token, password: self.new_password.clone() })
            }
            Confirm::Delete => {
                if self.password.is_empty() {
                    self.message = api::error_texts().text("password_empty");
                    return None;
                }
                self.confirm = Confirm::None;
                self.busy = Busy::Delete;
                Some(Request::DeleteAccount { token, password: self.password.clone() })
            }
            Confirm::None => None,
        }
    }

    /// TODO 224: sign out. The session is dropped here at once (the caller also deletes the saved file with
    /// `net::session_store::sign_out`) and the name goes back to guest; the server is told in the background.
    pub fn sign_out(&mut self) -> Option<Request> {
        if !self.signed_in() {
            return None;
        }
        let token = std::mem::take(&mut self.session.session_token);
        self.session = StoredSession::default();
        self.admin = false;
        self.mode = Mode::SignIn;
        self.password.clear();
        self.new_password.clear();
        self.confirm = Confirm::None;
        self.message = text("signed_out");
        Some(Request::SignOut { token })
    }

    /// Applies the result of the call that was out.
    pub fn finish(&mut self, result: Result<Outcome, NetError>) {
        let was = std::mem::replace(&mut self.busy, Busy::None);
        match result {
            Err(e) => self.message = api::describe_error(&e),
            Ok(Outcome::Account(reply)) => {
                self.session = StoredSession { username: reply.username.clone(), session_token: reply.session_token.clone() };
                self.admin = reply.grants_admin_ui();
                self.mode = Mode::Settings;
                self.password.clear();
                self.message.clear();
            }
            Ok(Outcome::PasswordChanged) => {
                self.new_password.clear();
                self.message = text("password_done");
            }
            Ok(Outcome::Deleted) => {
                self.session = StoredSession::default();
                self.admin = false;
                self.mode = Mode::SignIn;
                self.password.clear();
                self.message = text("deleted");
            }
            Ok(Outcome::SignedOut) | Ok(Outcome::OwnerDone) => {}
        }
        let _ = was;
    }
}

/// Runs a request (blocking; call it from a worker thread).
pub fn run(cfg: &BackendConfig, request: Request) -> Result<Outcome, NetError> {
    match request {
        Request::SignIn { name, password } => api::sign_in(cfg, &name, &password).map(Outcome::Account),
        Request::Create { name, password } => api::create_account(cfg, &name, &password, "").map(Outcome::Account),
        Request::ChangePassword { token, password } => api::change_password(cfg, &token, &password).map(|_| Outcome::PasswordChanged),
        Request::DeleteAccount { token, password } => api::delete_account(cfg, &token, &password).map(|_| Outcome::Deleted),
        Request::SignOut { token } => api::logout(cfg, &token).map(|_| Outcome::SignedOut),
        Request::Owner { token, call } => match call {
            OwnerCall::Reserve { code } => api::admin_reserve_room(cfg, &code, &token).map(|_| Outcome::OwnerDone),
            OwnerCall::Kick { code, player } => api::reserved_kick(cfg, &token, &code, &player).map(|_| Outcome::OwnerDone),
            OwnerCall::Rename { code, title } => api::reserved_rename(cfg, &token, &code, &title).map(|_| Outcome::OwnerDone),
        },
    }
}

/// [`run`] on a worker thread; poll the result from the frame loop and hand it to [`Panel::finish`].
pub fn spawn(cfg: &BackendConfig, request: Request) -> Pending<Outcome> {
    let cfg = cfg.clone();
    Pending::spawn(move || run(&cfg, request))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BackendConfig {
        BackendConfig::loopback(18105)
    }

    fn signed_in_panel() -> Panel {
        Panel::new(StoredSession { username: "ANNA".into(), session_token: "tok".into() })
    }

    #[test]
    fn sign_in_validates_the_name_then_goes_pending_with_the_canonical_name() {
        let mut p = Panel::new(StoredSession::default());
        p.name = "ab".into();
        p.password = "pw".into();
        assert_eq!(p.submit(&cfg(), false), None);
        assert_eq!(p.message, api::error_texts().text("name_invalid"));
        assert_eq!(p.busy, Busy::None);
        p.name = " anna_2 ".into();
        let req = p.submit(&cfg(), false).unwrap();
        assert_eq!(req, Request::SignIn { name: "ANNA_2".into(), password: "pw".into() });
        assert_eq!(p.busy, Busy::SignIn);
        assert!(!p.pending_text().is_empty());
        assert_eq!(p.submit(&cfg(), false), None, "a second press while pending sends nothing");
    }

    #[test]
    fn create_mode_sends_a_create_request() {
        let mut p = Panel::new(StoredSession::default());
        p.mode = Mode::Create;
        p.name = "new_one".into();
        p.password = "pw".into();
        assert!(matches!(p.submit(&cfg(), false), Some(Request::Create { .. })));
        assert_eq!(p.busy, Busy::Create);
    }

    #[test]
    fn offline_mode_sends_nothing_and_says_so() {
        let mut c = cfg();
        c.offline = true;
        let mut p = Panel::new(StoredSession::default());
        p.name = "anna".into();
        p.password = "pw".into();
        assert_eq!(p.submit(&c, false), None);
        assert_eq!(p.message, api::describe_error(&NetError::Offline));
    }

    #[test]
    fn a_good_answer_signs_in_and_a_failure_shows_upper_case_text() {
        let mut p = Panel::new(StoredSession::default());
        p.name = "anna".into();
        p.password = "pw".into();
        p.submit(&cfg(), false);
        p.finish(Err(NetError::Timeout));
        assert_eq!(p.busy, Busy::None);
        assert_eq!(p.message, api::error_texts().text("timeout"));
        p.submit(&cfg(), false);
        p.finish(Ok(Outcome::Account(AccountReply { username: "ANNA".into(), session_token: "t".into(), admin: true, ..Default::default() })));
        assert!(p.signed_in() && p.admin);
        assert_eq!(p.mode, Mode::Settings);
        assert!(p.password.is_empty(), "the password is not kept");
    }

    #[test]
    fn sign_out_clears_the_session_and_goes_back_to_guest() {
        let mut p = signed_in_panel();
        assert_eq!(p.mode, Mode::Settings);
        let req = p.sign_out().unwrap();
        assert_eq!(req, Request::SignOut { token: "tok".into() });
        assert!(!p.signed_in());
        assert_eq!(p.session.username, "");
        assert_eq!(p.mode, Mode::SignIn);
        assert_eq!(p.sign_out(), None);
    }

    #[test]
    fn password_change_and_delete_need_the_confirm_step() {
        let mut p = signed_in_panel();
        p.new_password = "fresh".into();
        p.password = "old".into();
        assert_eq!(p.confirm(&cfg(), false), None, "nothing to confirm yet");
        p.ask_confirm(Confirm::Password);
        assert_eq!(p.busy, Busy::None, "asking sends nothing");
        assert_eq!(p.confirm(&cfg(), false), Some(Request::ChangePassword { token: "tok".into(), password: "fresh".into() }));
        p.finish(Ok(Outcome::PasswordChanged));
        p.ask_confirm(Confirm::Delete);
        p.cancel_confirm();
        assert_eq!(p.confirm(&cfg(), false), None, "cancelled");
        p.ask_confirm(Confirm::Delete);
        assert_eq!(p.confirm(&cfg(), false), Some(Request::DeleteAccount { token: "tok".into(), password: "old".into() }));
        p.finish(Ok(Outcome::Deleted));
        assert!(!p.signed_in());
    }

    #[test]
    fn panel_texts_are_upper_case() {
        for key in ["pending_signin", "pending_create", "pending_password", "pending_delete", "signed_out", "confirm_delete", "confirm_password", "password_done", "deleted"] {
            let t = text(key);
            assert!(!t.is_empty(), "{key}");
            assert_eq!(t, t.to_uppercase(), "{key}");
        }
    }
}
