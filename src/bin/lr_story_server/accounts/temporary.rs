//! The one-time temporary password of an admin reset (copied from lr_server accounts/password_reset.rs).

use crate::util::random_bytes;

const TEMP_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const TEMP_GROUPS: usize = 4;
const TEMP_GROUP_LEN: usize = 4;

/// A one-time temporary password: 16 random symbols from [`TEMP_ALPHABET`] (80 bits) in four dash-separated groups of
/// four, like `K7QF-2MXD-H9TA-WP4C`. It meets the register rule (6 to 200 characters).
pub fn generate_temporary_password() -> String {
    let bytes = random_bytes(TEMP_GROUPS * TEMP_GROUP_LEN);
    let mut out = String::with_capacity(TEMP_GROUPS * (TEMP_GROUP_LEN + 1));
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && i % TEMP_GROUP_LEN == 0 {
            out.push('-');
        }
        out.push(TEMP_ALPHABET[(byte & 31) as usize] as char);
    }
    out
}
