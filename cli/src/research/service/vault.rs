//! The workspace's sealed credentials: AES-256-GCM under the owner's master
//! key, each bound to its purpose (`pair:<id>`, `upstream:<agent>`). A sealed
//! value is base64 of IV (12 bytes), tag (16) and ciphertext, as the Node
//! service sealed it, so a workspace it kept opens here.

use super::super::workspace::{Failure, fail_status};
use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, NewAead, Payload},
};
use base64::Engine as _;

pub struct Vault {
    cipher: Aes256Gcm,
}

fn broken() -> Failure {
    Failure {
        code: None,
        message: "A sealed workspace credential could not be opened.".into(),
        status: None,
    }
}

impl Vault {
    /// The vault of a 32-byte master key in hex (owner.json's `masterKey`).
    pub fn new(master_key: &str) -> Result<Self, Failure> {
        let key = hex::decode(master_key)
            .ok()
            .filter(|key| key.len() == 32)
            .ok_or_else(|| {
                fail_status("INVALID_CONFIG", "Invalid workspace configuration.", 400)
            })?;
        Ok(Self {
            cipher: Aes256Gcm::new(Key::from_slice(&key)),
        })
    }

    pub fn seal(&self, value: &str, purpose: &str) -> String {
        let mut iv = [0; 12];
        getrandom::fill(&mut iv).expect("the system's random source");
        let sealed = self
            .cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: value.as_bytes(),
                    aad: purpose.as_bytes(),
                },
            )
            .expect("AES-GCM sealing of a short value");
        // The cipher appends its tag; the stored form puts it first.
        let (data, tag) = sealed.split_at(sealed.len() - 16);
        let mut out = iv.to_vec();
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        base64::engine::general_purpose::STANDARD.encode(out)
    }

    pub fn open(&self, sealed: &str, purpose: &str) -> Result<String, Failure> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(sealed)
            .map_err(|_| broken())?;
        if bytes.len() < 28 {
            return Err(broken());
        }
        let (iv, rest) = bytes.split_at(12);
        let (tag, data) = rest.split_at(16);
        let mut message = data.to_vec();
        message.extend_from_slice(tag);
        let opened = self
            .cipher
            .decrypt(
                Nonce::from_slice(iv),
                Payload {
                    msg: &message,
                    aad: purpose.as_bytes(),
                },
            )
            .map_err(|_| broken())?;
        String::from_utf8(opened).map_err(|_| broken())
    }
}
