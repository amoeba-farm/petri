//! Concrete implementation of a Solana `Signer` from raw bytes
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use {
    ed25519_dalek::Signer as DalekSigner,
    rand0_7::{rngs::OsRng, CryptoRng, RngCore},
    solana_pubkey::Pubkey,
    solana_seed_phrase::generate_seed_from_seed_phrase_and_passphrase,
    solana_signature::{error::Error as SignatureError, Signature},
    solana_signer::{EncodableKey, EncodableKeypair, Signer, SignerError},
    std::{
        error,
        io::{Read, Write},
        path::Path,
    },
};

#[cfg(feature = "seed-derivable")]
pub mod seed_derivable;
pub mod signable;

/// A vanilla Ed25519 key pair
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Debug)]
pub struct Keypair(ed25519_dalek::SigningKey);

pub const KEYPAIR_LENGTH: usize = 64;

impl Keypair {
    /// Can be used for generating a Keypair without a dependency on `rand` types
    pub const SECRET_KEY_LENGTH: usize = 32;

    /// Constructs a new, random `Keypair` using a caller-provided RNG
    #[deprecated(
        since = "2.2.2",
        note = "Use `Keypair::new()` instead or generate 32 random bytes and use `Keypair::new_from_array`"
    )]
    pub fn generate<R>(csprng: &mut R) -> Self
    where
        R: CryptoRng + RngCore,
    {
        let mut secret_key = [0_u8; Self::SECRET_KEY_LENGTH];
        csprng.fill_bytes(&mut secret_key);
        Self(ed25519_dalek::SigningKey::from_bytes(&secret_key))
    }

    /// Constructs a new, random `Keypair` using `OsRng`
    pub fn new() -> Self {
        let mut rng = OsRng;
        let mut secret_key = [0_u8; Self::SECRET_KEY_LENGTH];
        rng.fill_bytes(&mut secret_key);
        Self(ed25519_dalek::SigningKey::from_bytes(&secret_key))
    }

    /// Constructs a new `Keypair` using secret key bytes
    pub fn new_from_array(secret_key: [u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(&secret_key))
    }

    /// Recovers a `Keypair` from a byte array
    #[deprecated(since = "2.2.2", note = "Use Keypair::try_from(&[u8]) instead")]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ed25519_dalek::SignatureError> {
        Self::try_from(bytes).map_err(ed25519_dalek::SignatureError::from_source)
    }

    /// Returns this `Keypair` as a byte array
    pub fn to_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        self.0.to_keypair_bytes()
    }

    /// Recovers a `Keypair` from a base58-encoded string
    pub fn from_base58_string(s: &str) -> Self {
        let mut buf = [0u8; ed25519_dalek::KEYPAIR_LENGTH];
        five8::decode_64(s, &mut buf).unwrap();
        Self::try_from(&buf[..]).unwrap()
    }

    /// Returns this `Keypair` as a base58-encoded string
    pub fn to_base58_string(&self) -> String {
        let mut out = [0u8; five8::BASE58_ENCODED_64_MAX_LEN];
        let len = five8::encode_64(&self.0.to_keypair_bytes(), &mut out);
        unsafe { String::from_utf8_unchecked(out[..len as usize].to_vec()) }
    }

    /// Gets this `Keypair`'s SecretKey
    #[deprecated(since = "2.2.2", note = "Use secret_bytes()")]
    pub fn secret(&self) -> &ed25519_dalek::SecretKey {
        self.0.as_bytes()
    }

    /// Gets this `Keypair`'s secret key bytes
    pub fn secret_bytes(&self) -> &[u8; Self::SECRET_KEY_LENGTH] {
        self.0.as_bytes()
    }

    /// Allows Keypair cloning
    ///
    /// Note that the `Clone` trait is intentionally unimplemented because making a
    /// second copy of sensitive secret keys in memory is usually a bad idea.
    ///
    /// Only use this in tests or when strictly required. Consider using [`std::sync::Arc<Keypair>`]
    /// instead.
    pub fn insecure_clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl TryFrom<&[u8]> for Keypair {
    type Error = SignatureError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let keypair_bytes: &[u8; ed25519_dalek::KEYPAIR_LENGTH] =
            bytes.try_into().map_err(|_| {
                SignatureError::from_source(String::from(
                    "candidate keypair byte array is the wrong length",
                ))
            })?;
        ed25519_dalek::SigningKey::from_keypair_bytes(keypair_bytes)
            .map_err(|_| {
                SignatureError::from_source(String::from(
                    "keypair bytes do not specify same pubkey as derived from their secret key",
                ))
            })
            .map(Self)
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(non_snake_case)]
#[wasm_bindgen]
impl Keypair {
    /// Create a new `Keypair `
    #[wasm_bindgen(constructor)]
    pub fn constructor() -> Keypair {
        Keypair::new()
    }

    /// Convert a `Keypair` to a `Uint8Array`
    pub fn toBytes(&self) -> Box<[u8]> {
        self.to_bytes().into()
    }

    /// Recover a `Keypair` from a `Uint8Array`
    pub fn fromBytes(bytes: &[u8]) -> Result<Keypair, JsValue> {
        Keypair::try_from(bytes).map_err(|e| e.to_string().into())
    }

    /// Return the `Pubkey` for this `Keypair`
    #[wasm_bindgen(js_name = pubkey)]
    pub fn js_pubkey(&self) -> Pubkey {
        // `wasm_bindgen` does not support traits (`Signer) yet
        self.pubkey()
    }
}

// This should also be marked deprecated, but it's not possible to put a
// `#[deprecated]` attribute on a trait implementation.
// Remove during the next major version bump.
impl From<ed25519_dalek::SigningKey> for Keypair {
    fn from(value: ed25519_dalek::SigningKey) -> Self {
        Self(value)
    }
}

impl Signer for Keypair {
    #[inline]
    fn pubkey(&self) -> Pubkey {
        Pubkey::from(self.0.verifying_key().to_bytes())
    }

    fn try_pubkey(&self) -> Result<Pubkey, SignerError> {
        Ok(self.pubkey())
    }

    fn sign_message(&self, message: &[u8]) -> Signature {
        Signature::from(self.0.sign(message).to_bytes())
    }

    fn try_sign_message(&self, message: &[u8]) -> Result<Signature, SignerError> {
        Ok(self.sign_message(message))
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

impl<T> PartialEq<T> for Keypair
where
    T: Signer,
{
    fn eq(&self, other: &T) -> bool {
        self.pubkey() == other.pubkey()
    }
}

impl EncodableKey for Keypair {
    fn read<R: Read>(reader: &mut R) -> Result<Self, Box<dyn error::Error>> {
        read_keypair(reader)
    }

    fn write<W: Write>(&self, writer: &mut W) -> Result<String, Box<dyn error::Error>> {
        write_keypair(self, writer)
    }
}

impl EncodableKeypair for Keypair {
    type Pubkey = Pubkey;

    /// Returns the associated pubkey. Use this function specifically for settings that involve
    /// reading or writing pubkeys. For other settings, use `Signer::pubkey()` instead.
    fn encodable_pubkey(&self) -> Self::Pubkey {
        self.pubkey()
    }
}

/// Reads a JSON-encoded `Keypair` from a `Reader` implementor
pub fn read_keypair<R: Read>(reader: &mut R) -> Result<Keypair, Box<dyn error::Error>> {
    let mut buffer = String::new();
    reader.read_to_string(&mut buffer)?;
    let trimmed = buffer.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Input must be a JSON array",
        )
        .into());
    }
    // we already checked that the string has at least two chars,
    // so 1..trimmed.len() - 1 won't be out of bounds
    #[allow(clippy::arithmetic_side_effects)]
    let contents = &trimmed[1..trimmed.len() - 1];
    let elements_vec: Vec<&str> = contents.split(',').map(|s| s.trim()).collect();
    let len = elements_vec.len();
    let elements: [&str; ed25519_dalek::KEYPAIR_LENGTH] =
        elements_vec.try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Expected {} elements, found {}",
                    ed25519_dalek::KEYPAIR_LENGTH,
                    len
                ),
            )
        })?;
    let mut out = [0u8; ed25519_dalek::KEYPAIR_LENGTH];
    for (idx, element) in elements.into_iter().enumerate() {
        let parsed: u8 = element.parse()?;
        out[idx] = parsed;
    }
    Keypair::try_from(&out[..]).map_err(|e| std::io::Error::other(e.to_string()).into())
}

/// Reads a `Keypair` from a file
pub fn read_keypair_file<F: AsRef<Path>>(path: F) -> Result<Keypair, Box<dyn error::Error>> {
    Keypair::read_from_file(path)
}

/// Writes a `Keypair` to a `Write` implementor with JSON-encoding
pub fn write_keypair<W: Write>(
    keypair: &Keypair,
    writer: &mut W,
) -> Result<String, Box<dyn error::Error>> {
    let keypair_bytes = keypair.0.to_bytes();
    let mut result = Vec::with_capacity(64 * 4 + 2); // Estimate capacity: 64 numbers * (up to 3 digits + 1 comma) + 2 brackets

    result.push(b'['); // Opening bracket

    for (i, &num) in keypair_bytes.iter().enumerate() {
        if i > 0 {
            result.push(b','); // Comma separator for all elements except the first
        }

        // Convert number to string and then to bytes
        let num_str = num.to_string();
        result.extend_from_slice(num_str.as_bytes());
    }

    result.push(b']'); // Closing bracket
    writer.write_all(&result)?;
    let as_string = String::from_utf8(result)?;
    Ok(as_string)
}

/// Writes a `Keypair` to a file with JSON-encoding
pub fn write_keypair_file<F: AsRef<Path>>(
    keypair: &Keypair,
    outfile: F,
) -> Result<String, Box<dyn error::Error>> {
    keypair.write_to_file(outfile)
}

/// Constructs a `Keypair` from caller-provided seed entropy
pub fn keypair_from_seed(seed: &[u8]) -> Result<Keypair, Box<dyn error::Error>> {
    if seed.len() < ed25519_dalek::SECRET_KEY_LENGTH {
        return Err("Seed is too short".into());
    }
    let secret_key: [u8; ed25519_dalek::SECRET_KEY_LENGTH] = seed
        [..ed25519_dalek::SECRET_KEY_LENGTH]
        .try_into()
        .map_err(|error| format!("invalid seed bytes: {error}"))?;
    Ok(Keypair::new_from_array(secret_key))
}

pub fn keypair_from_seed_phrase_and_passphrase(
    seed_phrase: &str,
    passphrase: &str,
) -> Result<Keypair, Box<dyn std::error::Error>> {
    keypair_from_seed(&generate_seed_from_seed_phrase_and_passphrase(
        seed_phrase,
        passphrase,
    ))
}
