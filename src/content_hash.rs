//! Exact byte hashing only. Callers retain their domain separation, canonical
//! serialization, identity checks and secret handling.

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    solana_program::hash::hash(bytes)
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
