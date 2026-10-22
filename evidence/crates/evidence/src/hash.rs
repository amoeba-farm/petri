//! Domain-separated SHA-256, the one hashing convention of this crate.

use sha2::{Digest, Sha256};

/// `SHA-256(domain || 0x00 || for each field: u32-LE length || bytes)`.
///
/// Every field is length-prefixed, so no two different field lists hash
/// alike, and the domain keeps one kind of identifier from ever equalling
/// another kind.
pub fn tagged(domain: &str, fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    for field in fields {
        hasher.update((field.len() as u32).to_le_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

/// Plain SHA-256 of some bytes (proof artifacts are addressed by it).
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn hex32(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

/// A 64-character lowercase hex digest.
pub fn parse_hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || text.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(text, &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefixes_separate_fields() {
        assert_ne!(tagged("d", &[b"ab", b"c"]), tagged("d", &[b"a", b"bc"]));
        assert_ne!(tagged("d1", &[b"x"]), tagged("d2", &[b"x"]));
    }

    #[test]
    fn hex_round_trip() {
        let h = sha256(b"x");
        assert_eq!(parse_hex32(&hex32(&h)), Some(h));
        assert_eq!(parse_hex32(&hex32(&h).to_uppercase()), None);
        assert_eq!(parse_hex32("00"), None);
    }
}
