//! Opens an address in the default browser: the fallback when no native build can run. No dependencies; the browser
//! handles pointer lock and the gamepad, which an embedded web view on macOS and Linux would not (docs/LAUNCHER.md).

use std::process::{Command, Stdio};

/// Only plain http(s) addresses without spaces or quotes are opened, so the address can never become a command.
pub fn open_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) || url.bytes().any(|b| b <= b' ' || b == b'"' || b == b'\'' || b == b'^' || b == b'`' || b == 0x7f) {
        return Err("not a plain web address".into());
    }
    let mut command = if cfg!(windows) {
        // rundll32 takes the address as one argument: no shell parsing of & or % as `cmd /c start` would do.
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_web_addresses_are_opened() {
        for bad in ["file:///C:/x.exe", "javascript:alert(1)", "https://x.org/a b", "https://x.org/\"&calc", "calc.exe", "", "https://x.org/a'b"] {
            assert!(open_url(bad).is_err(), "{bad:?}");
        }
    }
}
