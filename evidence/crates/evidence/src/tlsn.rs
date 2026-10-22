//! Verifying a TLSNotary presentation (the `web_proof` method).
//!
//! What a passing check establishes, and no more:
//!
//! - a notary in the verifier policy signed an attestation of a TLS 1.2
//!   session (MPC mode: the notary held half of the session keys and never
//!   saw plaintext);
//! - the session was with the named server: its certificate chain verifies
//!   to a trusted root as of the session time, and its key signed the
//!   handshake;
//! - the disclosed bytes are exactly what was sent and received in that
//!   session; every other byte is hidden (zero).
//!
//! It does not establish that the content is a market price, that it was
//! not influenced by what the contributor sent, or anything about the
//! source's independence. Those stay with the oracle's methodology.

use tlsn_attestation::CryptoProvider;
use tlsn_attestation::presentation::{Presentation, PresentationOutput};
use tlsn_core::connection::ServerName;
use tlsn_core::webpki::{CertificateDer, RootCertStore, ServerCertVerifier};

use crate::http::Transcript;
use crate::policy::{Notary, Policy, Trust};

/// The attestation extension that binds a proof to the wallet that will
/// submit it (32-byte public key), signed by the notary in the session.
pub const BINDING_EXTENSION: &[u8] = b"amoeba.binding.v1";

/// Which roots the server's certificate chained to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerTrust {
    /// Mozilla's root program (`webpki-root-certs`, compiled in).
    MozillaRoots,
    /// A development root from the policy (test certificates).
    DevelopmentRoot,
    /// Not a TLS proof: a DKIM key from the verifier's key policy.
    DkimKeyPolicy,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub server_name: String,
    /// Notary-attested session start (Unix seconds). In MPC mode the
    /// notary refuses a prover clock more than five seconds off its own.
    pub time: u64,
    pub sent: Transcript,
    pub recv: Transcript,
    pub notary: Notary,
    pub server_trust: ServerTrust,
    /// The wallet the proof is bound to, when it carries the binding.
    pub binding: Option<[u8; 32]>,
}

impl Session {
    /// The weakest trust any part of the check relied on.
    pub fn trust(&self) -> Trust {
        if self.notary.trust == Trust::Production && self.server_trust == ServerTrust::MozillaRoots
        {
            Trust::Production
        } else {
            Trust::Development
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvenanceError {
    #[error("the proof is larger than {0} bytes")]
    TooLarge(usize),
    #[error("the proof is not a TLSNotary presentation of this version: {0}")]
    Decode(String),
    #[error("the proof is signed by {alg} key {key}, which is not a notary this verifier accepts")]
    UntrustedNotary { alg: String, key: String },
    #[error("the proof does not verify: {0}")]
    Invalid(String),
    #[error("the proof does not disclose the server's identity")]
    NoServerName,
    #[error("the proof discloses no transcript")]
    NoTranscript,
    #[error("the session is longer than this verifier accepts")]
    TranscriptTooLong,
    #[error("the proof carries an unknown or malformed attestation extension")]
    Extension,
}

/// The bincode options both sides use (`bincode::serialize` compatible:
/// fixed-width integers), with a size limit for hostile input.
pub fn bincode_options(limit: usize) -> impl bincode::Options {
    use bincode::Options;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(limit as u64)
}

pub fn decode_presentation(bytes: &[u8], policy: &Policy) -> Result<Presentation, ProvenanceError> {
    use bincode::Options;
    if bytes.len() > policy.limits.max_presentation_bytes {
        return Err(ProvenanceError::TooLarge(
            policy.limits.max_presentation_bytes,
        ));
    }
    bincode_options(policy.limits.max_presentation_bytes)
        .deserialize(bytes)
        .map_err(|e| ProvenanceError::Decode(e.to_string()))
}

fn provider(roots: Vec<CertificateDer>) -> Result<CryptoProvider, ProvenanceError> {
    let cert = ServerCertVerifier::new(&RootCertStore { roots })
        .map_err(|e| ProvenanceError::Invalid(e.to_string()))?;
    Ok(CryptoProvider {
        cert,
        ..Default::default()
    })
}

fn key_alg(alg: &tlsn_attestation::signing::KeyAlgId) -> String {
    if *alg == tlsn_attestation::signing::KeyAlgId::K256 {
        "secp256k1".into()
    } else {
        alg.to_string()
    }
}

pub fn verify(bytes: &[u8], policy: &Policy) -> Result<Session, ProvenanceError> {
    let presentation = decode_presentation(bytes, policy)?;
    let key = presentation.verifying_key();
    let alg = key_alg(&key.alg);
    let notary = policy.notary(&alg, &key.data).cloned().ok_or_else(|| {
        ProvenanceError::UntrustedNotary {
            alg: alg.clone(),
            key: hex::encode(&key.data),
        }
    })?;
    // Mozilla roots first; development roots only when the policy has them,
    // and then the result says so.
    let mozilla = presentation.clone().verify(&CryptoProvider::default());
    let (output, server_trust): (PresentationOutput, ServerTrust) = match mozilla {
        Ok(out) => (out, ServerTrust::MozillaRoots),
        Err(first) if !policy.development_roots.is_empty() => {
            let roots = policy
                .development_roots
                .iter()
                .map(|d| CertificateDer(d.clone()))
                .collect();
            match presentation.verify(&provider(roots)?) {
                Ok(out) => (out, ServerTrust::DevelopmentRoot),
                Err(_) => return Err(ProvenanceError::Invalid(first.to_string())),
            }
        }
        Err(e) => return Err(ProvenanceError::Invalid(e.to_string())),
    };
    let server_name = match output.server_name {
        Some(ServerName::Dns(name)) => name.as_str().to_ascii_lowercase(),
        None => return Err(ProvenanceError::NoServerName),
    };
    let transcript = output.transcript.ok_or(ProvenanceError::NoTranscript)?;
    let lengths = &output.connection_info.transcript_length;
    let (sent_len, recv_len) = (lengths.sent as usize, lengths.received as usize);
    if sent_len > policy.limits.max_sent || recv_len > policy.limits.max_recv {
        return Err(ProvenanceError::TranscriptTooLong);
    }
    let sent = transcript
        .sent_unsafe(sent_len)
        .map_err(|e| ProvenanceError::Invalid(e.to_string()))?;
    let recv = transcript
        .received_unsafe(recv_len)
        .map_err(|e| ProvenanceError::Invalid(e.to_string()))?;
    let sent = Transcript::new(sent, transcript.sent_authed().iter());
    let recv = Transcript::new(recv, transcript.received_authed().iter());
    let mut binding = None;
    for ext in &output.extensions {
        if ext.id != BINDING_EXTENSION || ext.value.len() != 32 || binding.is_some() {
            return Err(ProvenanceError::Extension);
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&ext.value);
        binding = Some(key);
    }
    Ok(Session {
        server_name,
        time: output.connection_info.time,
        sent,
        recv,
        notary,
        server_trust,
        binding,
    })
}
