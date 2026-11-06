//! Hashes and signatures. A build is trusted only when its manifest verifies against a public key compiled into
//! the program (keys/trusted.txt); every file is then checked against the hash the signed manifest lists.
//!
//! The signature is ed25519 over the exact bytes of `manifest.json`, stored as 128 hex digits in
//! `manifest.json.sig`. `ring` does the verification (it is already in the tree through rustls).

use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &[u8]) -> String {
    to_hex(&Sha256::digest(data))
}

pub fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// A hasher that can sit in a `Write` chain, so a download is hashed while it is written to disk.
pub struct HashingWriter<W: std::io::Write> {
    pub inner: W,
    hasher: Sha256,
    pub written: u64,
}

impl<W: std::io::Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        HashingWriter { inner, hasher: Sha256::new(), written: 0 }
    }

    pub fn finish(self) -> (W, String, u64) {
        (self.inner, to_hex(&self.hasher.finalize()), self.written)
    }
}

impl<W: std::io::Write> std::io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Parses a key list: one hex key (32 bytes) per line; `#` starts a comment; a line that is not a key is skipped
/// (the test below makes sure the compiled-in list has none).
pub fn parse_keys(text: &str) -> Vec<[u8; 32]> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .filter_map(|l| from_hex(l).and_then(|b| <[u8; 32]>::try_from(b).ok()))
        .collect()
}

/// The keys compiled into this program.
pub fn trusted_keys() -> Vec<[u8; 32]> {
    parse_keys(include_str!("../keys/trusted.txt"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// No public key is configured, so nothing can be trusted.
    NoKeys,
    BadSignatureFile,
    /// The signature does not verify against any trusted key.
    Rejected,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            VerifyError::NoKeys => "no signing key is built into this program, so native builds cannot be trusted",
            VerifyError::BadSignatureFile => "the signature file is not 128 hex digits",
            VerifyError::Rejected => "the signature does not match any trusted key",
        })
    }
}

/// Checks `signature_text` (hex) over `message` against each trusted key; one match is enough (rotation keeps two keys).
pub fn verify_signature(keys: &[[u8; 32]], message: &[u8], signature_text: &str) -> Result<(), VerifyError> {
    if keys.is_empty() {
        return Err(VerifyError::NoKeys);
    }
    let sig = from_hex(signature_text).filter(|b| b.len() == 64).ok_or(VerifyError::BadSignatureFile)?;
    for key in keys {
        let public = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key);
        if public.verify(message, &sig).is_ok() {
            return Ok(());
        }
    }
    Err(VerifyError::Rejected)
}

#[cfg(any(test, feature = "testsite"))]
pub mod test_keys {
    use ring::signature::{Ed25519KeyPair, KeyPair};

    /// A throwaway key pair from a fixed seed (tests only; never a real key).
    pub fn pair(seed_byte: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed_byte; 32]).unwrap()
    }

    pub fn public(pair: &Ed25519KeyPair) -> [u8; 32] {
        pair.public_key().as_ref().try_into().unwrap()
    }

    pub fn sign(pair: &Ed25519KeyPair, message: &[u8]) -> String {
        super::to_hex(pair.sign(message).as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answer() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        assert_eq!(from_hex("00ff10").unwrap(), vec![0, 255, 16]);
        assert!(from_hex("0").is_none() && from_hex("zz").is_none() && from_hex("é1").is_none());
        assert_eq!(to_hex(&[1, 171]), "01ab");
    }

    #[test]
    fn a_signature_verifies_only_with_the_right_key_and_message() {
        let (a, b) = (test_keys::pair(1), test_keys::pair(2));
        let sig = test_keys::sign(&a, b"manifest bytes");
        assert_eq!(verify_signature(&[test_keys::public(&a)], b"manifest bytes", &sig), Ok(()));
        assert_eq!(verify_signature(&[test_keys::public(&a)], b"manifest bytez", &sig), Err(VerifyError::Rejected));
        assert_eq!(verify_signature(&[test_keys::public(&b)], b"manifest bytes", &sig), Err(VerifyError::Rejected));
        // the second slot (key rotation) is enough
        assert_eq!(verify_signature(&[test_keys::public(&b), test_keys::public(&a)], b"manifest bytes", &sig), Ok(()));
        assert_eq!(verify_signature(&[], b"manifest bytes", &sig), Err(VerifyError::NoKeys));
        assert_eq!(verify_signature(&[test_keys::public(&a)], b"x", "abcd"), Err(VerifyError::BadSignatureFile));
        assert_eq!(verify_signature(&[test_keys::public(&a)], b"x", "not hex at all"), Err(VerifyError::BadSignatureFile));
        // whitespace around the hex (a trailing newline) is fine
        assert_eq!(verify_signature(&[test_keys::public(&a)], b"manifest bytes", &format!("{sig}\n")), Ok(()));
    }

    #[test]
    fn the_compiled_in_key_list_has_only_valid_keys() {
        let text = include_str!("../keys/trusted.txt");
        let lines = text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).count();
        assert_eq!(parse_keys(text).len(), lines, "every non-comment line of keys/trusted.txt must be a 64-digit hex key");
    }

    #[test]
    fn key_lists_skip_comments_and_blank_lines() {
        let key = to_hex(&[7u8; 32]);
        let keys = parse_keys(&format!("# note\n\n{key}  # KT\nnot a key\n"));
        assert_eq!(keys, vec![[7u8; 32]]);
    }

    #[test]
    fn the_hashing_writer_hashes_what_it_writes() {
        use std::io::Write;
        let mut w = HashingWriter::new(Vec::new());
        w.write_all(b"ab").unwrap();
        w.write_all(b"c").unwrap();
        let (data, hash, n) = w.finish();
        assert_eq!((data.as_slice(), n), (b"abc".as_slice(), 3));
        assert_eq!(hash, sha256_hex(b"abc"));
    }
}
