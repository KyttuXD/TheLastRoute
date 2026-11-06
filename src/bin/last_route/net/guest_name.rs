//! The name a runner who is not signed in shows to the others: `GUEST_` and a four digit number, e.g. `GUEST_4829`.
//!
//! * The client picks it ONCE per install ([`random_guest_name`]) and keeps it ([`super::store::GuestStore`]: a file on
//!   native, the page's localStorage on the web), so a guest keeps the number across a reload and gets a new one only
//!   when the stored name is gone. The server's guest name lease (`POST /api/guest/name`, `R-4823` style) is optional
//!   and never changes this name.
//! * The shape passes every name rule on the way: upper-case letters, digits and an underscore (the account username
//!   rule `[A-Z0-9_]{3,24}`, [`super::world_sync::clean_name`]), ten characters (a nametag, a profile message).
//! * Numbers `1000..=9999` are drawn; `0000..=0999` are never drawn and stand in for a runner whose name is not known
//!   yet ([`placeholder`]), so a placeholder never equals a real guest.
//! * A registered account must not look like a guest ([`reserved_for_guests`]): the account screen refuses such a name
//!   and the server does too (`account_service`).
//! * Within one room the host keeps the names apart: the later guest with a taken name is given a free one
//!   ([`unique_guest_name`]) and told (see `NetSession`).

use super::random_below;

/// Every guest name starts with this.
pub const PREFIX: &str = "GUEST_";
/// The smallest and the largest number drawn.
pub const MIN_NUMBER: u32 = 1000;
pub const MAX_NUMBER: u32 = 9999;

/// `GUEST_` and `number` as four digits (`n` is taken modulo 10000).
pub fn from_number(n: u32) -> String {
    format!("{PREFIX}{:04}", n % 10_000)
}

/// A fresh random guest name, `GUEST_1000` to `GUEST_9999`.
pub fn random_guest_name() -> String {
    from_number(MIN_NUMBER + random_below((MAX_NUMBER - MIN_NUMBER + 1) as usize) as u32)
}

/// The shape of a guest name: `GUEST_` and exactly four digits (the drawn range and the placeholders). Case matters:
/// names on the wire and in the store are upper-case already.
pub fn is_guest_name(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|d| d.len() == 4 && d.bytes().all(|b| b.is_ascii_digit()))
}

/// A name the client could have drawn: the shape above with a number in `1000..=9999` (not a placeholder).
pub fn is_drawn_name(name: &str) -> bool {
    is_guest_name(name) && name[PREFIX.len()..].parse::<u32>().is_ok_and(|n| (MIN_NUMBER..=MAX_NUMBER).contains(&n))
}

/// Names a registered account may not take: `GUEST_` followed only by digits or hex letters (`GUEST_4829`, and the
/// session names `GUEST_45858185` / `GUEST_3FA9C1` the account store has handed out). Any case.
pub fn reserved_for_guests(username: &str) -> bool {
    let upper = username.trim().to_ascii_uppercase();
    upper.strip_prefix(PREFIX).is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)))
}

/// A guest name no one has: random draws first, then every free number in order. `taken` says whether a name is in use.
pub fn unique_guest_name(taken: &dyn Fn(&str) -> bool) -> String {
    for _ in 0..64 {
        let name = random_guest_name();
        if !taken(&name) {
            return name;
        }
    }
    (MIN_NUMBER..=MAX_NUMBER).map(from_number).find(|n| !taken(n)).unwrap_or_else(random_guest_name)
}

/// The name shown for a runner whose real name has not arrived: `GUEST_0` and three digits from its peer id. Never in
/// the drawn range (`GUEST_0000` to `GUEST_0999`).
pub fn placeholder(peer: u32) -> String {
    from_number(peer % 1000)
}

/// The name this runner plays under online: the signed-in account name, else its guest name. `username` is the account
/// name the app keeps (empty when signed out). A guest name that is somehow empty draws a fresh one.
pub fn local_name(username: &str, guest: &str) -> String {
    let username = username.trim();
    if !username.is_empty() {
        username.to_ascii_uppercase()
    } else if guest.trim().is_empty() {
        random_guest_name()
    } else {
        guest.trim().to_ascii_uppercase()
    }
}

/// The name the lobby and the players list show for this runner. In a room (`online`) it is [`local_name`], the name
/// the others see. Alone nobody else sees it, so it stays what solo has always shown: the signed-in name, else RUNNER.
pub fn shown_name(online: bool, username: &str, guest: &str) -> String {
    if online {
        local_name(username, guest)
    } else if username.trim().is_empty() {
        "RUNNER".into()
    } else {
        username.trim().to_ascii_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_drawn_name_is_guest_and_four_digits_in_range() {
        for _ in 0..2000 {
            let name = random_guest_name();
            assert!(is_guest_name(&name), "{name}");
            let n: u32 = name[PREFIX.len()..].parse().unwrap();
            assert!((MIN_NUMBER..=MAX_NUMBER).contains(&n), "{name}");
            assert_eq!(name.len(), 10, "fits a nametag and the 24 character name limit");
            assert!(name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'), "{name}");
        }
    }

    #[test]
    fn the_draw_covers_a_wide_spread() {
        let seen: HashSet<String> = (0..400).map(|_| random_guest_name()).collect();
        assert!(seen.len() > 300, "{} distinct names in 400 draws", seen.len());
    }

    #[test]
    fn the_name_shape_is_strict() {
        for good in ["GUEST_1000", "GUEST_4829", "GUEST_9999", "GUEST_0000"] {
            assert!(is_guest_name(good), "{good}");
        }
        for bad in ["GUEST_482", "GUEST_48290", "guest_4829", "GUEST4829", "GUEST_48A9", "RUNNER", "R-4823", "", "GUEST_"] {
            assert!(!is_guest_name(bad), "{bad}");
        }
        assert_eq!(from_number(7), "GUEST_0007");
        assert_eq!(from_number(12_345), "GUEST_2345");
    }

    #[test]
    fn a_registered_account_may_not_look_like_a_guest() {
        for reserved in ["GUEST_4829", "guest_4829", "  Guest_0001 ", "GUEST_45858185", "GUEST_3FA9C1", "GUEST_1"] {
            assert!(reserved_for_guests(reserved), "{reserved}");
        }
        for free in ["GUEST", "GUEST_", "GUEST_ALPHA", "GUESTS_1234", "MY_GUEST_1234", "KYTTU", "RUNNER"] {
            assert!(!reserved_for_guests(free), "{free}");
        }
    }

    #[test]
    fn a_unique_name_skips_every_taken_one() {
        let taken = |n: &str| n != "GUEST_5555";
        // everything is taken but one name: the sweep finds it
        assert_eq!(unique_guest_name(&taken), "GUEST_5555");
        let free = unique_guest_name(&|n| n == "GUEST_1234");
        assert_ne!(free, "GUEST_1234");
        assert!(is_guest_name(&free));
    }

    #[test]
    fn a_placeholder_is_never_a_drawn_name() {
        for peer in [0, 1, 2, 999, 1000, 4821, u32::MAX] {
            let name = placeholder(peer);
            assert!(is_guest_name(&name), "{name}");
            let n: u32 = name[PREFIX.len()..].parse().unwrap();
            assert!(n < MIN_NUMBER, "{name}");
        }
        assert_eq!(placeholder(2), "GUEST_0002");
    }

    #[test]
    fn solo_keeps_runner_and_a_room_shows_the_guest_name() {
        assert_eq!(shown_name(false, "", "GUEST_4829"), "RUNNER");
        assert_eq!(shown_name(false, "ana", "GUEST_4829"), "ANA");
        assert_eq!(shown_name(true, "", "GUEST_4829"), "GUEST_4829");
        assert_eq!(shown_name(true, "ana", "GUEST_4829"), "ANA");
    }

    #[test]
    fn an_account_name_beats_the_guest_name() {
        assert_eq!(local_name("ana", "GUEST_4829"), "ANA");
        assert_eq!(local_name("", "GUEST_4829"), "GUEST_4829");
        assert_eq!(local_name("  ", "guest_4829"), "GUEST_4829");
        assert!(is_guest_name(&local_name("", "")), "a missing guest name draws one");
    }
}
