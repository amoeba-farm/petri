//! The notary: the verifier half of an MPC-TLS session, which signs an
//! attestation of the transcript commitments when the session ends.
//!
//! In MPC mode the notary never sees the plaintext, nor even the server's
//! name (that is proven later, in the presentation). It holds a share of
//! the session keys, so the prover cannot forge server bytes, and it
//! attests the session time from its own clock. Anyone can run one
//! (`amoeba-notary`); whose signatures count is the verifier policy's
//! business, not the notary's.

use std::sync::Arc;
use std::time::Instant;

use futures::{AsyncRead, AsyncWrite};
use tlsn::{
    Session,
    attestation::{
        Attestation, AttestationConfig, CryptoProvider, InvalidExtension,
        request::Request as AttestationRequest,
        signing::{
            KeyAlgId, Secp256k1Signer, Signature, SignatureAlgId, SignatureError, Signer,
            VerifyingKey,
        },
    },
    config::verifier::VerifierConfig,
    connection::{CertBinding, ConnectionInfo, TranscriptLength},
    transcript::ContentType,
    verifier::{VerifierCommitStart, VerifierOutput},
    webpki::RootCertStore,
};

use crate::wire::{self, WireError};
use amoeba_evidence::policy::DEFAULT_LIMITS;
use amoeba_evidence::tlsn::{BINDING_EXTENSION, bincode_options};

#[derive(Clone)]
pub struct NotaryConfig {
    pub key: NotaryKey,
    /// The most a prover may preprocess for (bytes sent / received).
    pub max_sent: usize,
    pub max_recv: usize,
}

/// The key that signs attestations.
#[derive(Clone)]
pub enum NotaryKey {
    /// A secp256k1 secret key this process holds (a dev notary's key file).
    Local([u8; 32]),
    /// A Cloud KMS key version: the secret never leaves KMS.
    Kms(Arc<crate::kms::KmsSigner>),
}

impl NotaryConfig {
    pub fn new(signing_key: [u8; 32]) -> Self {
        Self::with_key(NotaryKey::Local(signing_key))
    }

    pub fn with_key(key: NotaryKey) -> Self {
        Self {
            key,
            max_sent: DEFAULT_LIMITS.max_sent,
            max_recv: DEFAULT_LIMITS.max_recv,
        }
    }

    /// The compressed SEC1 public key a verifier policy lists.
    pub fn public_key(&self) -> anyhow::Result<Vec<u8>> {
        match &self.key {
            NotaryKey::Local(secret) => {
                let sk = k256::ecdsa::SigningKey::from_bytes(&(*secret).into())?;
                Ok(sk
                    .verifying_key()
                    .to_encoded_point(true)
                    .as_bytes()
                    .to_vec())
            }
            NotaryKey::Kms(kms) => Ok(kms.public_key()),
        }
    }

    fn signer(&self) -> Result<Box<dyn Signer + Send + Sync>, NotaryError> {
        Ok(match &self.key {
            NotaryKey::Local(secret) => {
                Box::new(Secp256k1Signer::new(secret).map_err(session_err)?)
            }
            NotaryKey::Kms(kms) => Box::new(KmsAttestationSigner(kms.clone())),
        })
    }
}

/// TLSNotary signs synchronously; a KMS signature is a network call, made
/// on this worker thread while the runtime's other workers carry on.
struct KmsAttestationSigner(Arc<crate::kms::KmsSigner>);

impl Signer for KmsAttestationSigner {
    fn alg_id(&self) -> SignatureAlgId {
        SignatureAlgId::SECP256K1
    }

    fn sign(&self, msg: &[u8]) -> Result<Signature, SignatureError> {
        let handle = tokio::runtime::Handle::try_current()
            .ok()
            .filter(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
            .ok_or_else(|| {
                SignatureError::from_str("KMS signing needs the multi-threaded runtime")
            })?;
        let data = tokio::task::block_in_place(|| handle.block_on(self.0.sign(msg)))
            .map_err(|e| SignatureError::from_str(&e.to_string()))?;
        Ok(Signature {
            alg: SignatureAlgId::SECP256K1,
            data: data.to_vec(),
        })
    }

    fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey {
            alg: KeyAlgId::K256,
            data: self.0.public_key(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NotarySummary {
    pub sent_bytes: u32,
    pub recv_bytes: u32,
    pub session_time: u64,
    pub millis: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum NotaryError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("{0}")]
    Refused(String),
    #[error("the TLSNotary session failed: {0}")]
    Session(String),
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn session_err(e: impl std::fmt::Display) -> NotaryError {
    NotaryError::Session(e.to_string())
}

/// Serves one prover on `io`, start to finish.
pub async fn serve<S>(mut io: S, cfg: &NotaryConfig) -> Result<NotarySummary, NotaryError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let started = Instant::now();
    let hello = wire::read_line(&mut io).await?;
    let want = wire::hello_line();
    if hello != want {
        let reason = format!("this notary runs {want}");
        wire::write_line(&mut io, &format!("no {reason}")).await?;
        return Err(NotaryError::Refused(format!("prover sent `{hello}`")));
    }
    let public = cfg.public_key().map_err(session_err)?;
    wire::write_line(&mut io, &format!("ok secp256k1 {}", hex::encode(&public))).await?;

    let session = Session::new(io);
    let (driver, mut handle) = session.split();
    let driver_task = tokio::spawn(driver);
    // Dropping this session (a time limit) also stops its connection.
    let _stop_driver = AbortOnDrop(driver_task.abort_handle());

    let verifier_config = VerifierConfig::builder()
        .root_store(RootCertStore::mozilla())
        .build()
        .map_err(session_err)?;
    let verifier = match handle
        .new_verifier(verifier_config)
        .map_err(session_err)?
        .commit()
        .await
        .map_err(session_err)?
    {
        VerifierCommitStart::Mpc(v) => {
            let c = v.config();
            if c.max_sent_data() > cfg.max_sent || c.max_recv_data() > cfg.max_recv {
                let msg = format!(
                    "this notary preprocesses at most {} bytes sent and {} received",
                    cfg.max_sent, cfg.max_recv
                );
                v.reject(Some(&msg)).await.map_err(session_err)?;
                return Err(NotaryError::Refused(msg));
            }
            v.accept()
                .await
                .map_err(session_err)?
                .run()
                .await
                .map_err(session_err)?
        }
        VerifierCommitStart::Proxy(v) => {
            v.reject(Some("this notary runs MPC mode only"))
                .await
                .map_err(session_err)?;
            return Err(NotaryError::Refused("proxy mode requested".into()));
        }
    };
    let (
        VerifierOutput {
            transcript_commitments,
            ..
        },
        verifier,
    ) = verifier
        .verify()
        .await
        .map_err(session_err)?
        .accept()
        .await
        .map_err(session_err)?;
    let tls_transcript = verifier.tls_transcript().clone();
    verifier.close().await.map_err(session_err)?;

    let app_len = |records: &[tlsn::transcript::Record]| -> usize {
        records
            .iter()
            .filter(|r| matches!(r.typ, ContentType::ApplicationData))
            .map(|r| r.ciphertext.len())
            .sum()
    };
    let sent_len = app_len(tls_transcript.sent());
    let recv_len = app_len(tls_transcript.recv());

    handle.close();
    let mut io = driver_task
        .await
        .map_err(session_err)?
        .map_err(session_err)?;

    let request_bytes = wire::read_frame(&mut io).await?;
    use bincode::Options;
    let request: AttestationRequest = bincode_options(wire::MAX_FRAME)
        .deserialize(&request_bytes)
        .map_err(session_err)?;

    let mut provider = CryptoProvider::default();
    provider.signer.set_signer(cfg.signer()?);
    let mut config = AttestationConfig::builder();
    config
        .supported_signature_algs(Vec::from_iter(provider.signer.supported_algs()))
        .extension_validator(|extensions| {
            // The only extension: the submitting wallet, once.
            if extensions.len() > 1
                || extensions
                    .iter()
                    .any(|e| e.id != BINDING_EXTENSION || e.value.len() != 32)
            {
                return Err(InvalidExtension::new(
                    "only amoeba.binding.v1 (32 bytes) is accepted",
                ));
            }
            Ok(())
        });
    let config = config.build().map_err(session_err)?;
    let CertBinding::V1_2(binding) = tls_transcript.certificate_binding() else {
        return Err(NotaryError::Session(
            "unsupported certificate binding".into(),
        ));
    };
    let mut builder = Attestation::builder(&config)
        .accept_request(request)
        .map_err(session_err)?;
    builder
        .connection_info(ConnectionInfo {
            time: tls_transcript.time(),
            version: tls_transcript.version(),
            transcript_length: TranscriptLength {
                sent: sent_len as u32,
                received: recv_len as u32,
            },
        })
        .server_ephemeral_key(binding.server_ephemeral_key.clone())
        .transcript_commitments(transcript_commitments);
    let attestation = builder.build(&provider).map_err(session_err)?;
    let bytes = bincode_options(wire::MAX_FRAME)
        .serialize(&attestation)
        .map_err(session_err)?;
    wire::write_frame(&mut io, &bytes).await?;
    Ok(NotarySummary {
        sent_bytes: sent_len as u32,
        recv_bytes: recv_len as u32,
        session_time: tls_transcript.time(),
        millis: started.elapsed().as_millis(),
    })
}
