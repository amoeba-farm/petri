//! The prover: petri's side of a web proof.
//!
//! It opens the TLS connection to the source itself, from the
//! contributor's machine, with the contributor's own session (cookies or an
//! authorization header taken from their browser). The notary takes part
//! in the TLS key exchange through MPC, so the server's bytes cannot be
//! forged, yet it never sees the plaintext. The contributor's credentials
//! reach only the source server, inside TLS; they are never in the
//! attestation, the presentation, the bundle or a log.
//!
//! Disclosure is fixed, so a verifier can rely on it and the preview shows
//! exactly it:
//!
//! - request: the request line and `Host` in full; the names of the other
//!   headers, never their values except `Accept`, `Accept-Encoding` and
//!   `Connection`;
//! - response: the status line, every header (but `Set-Cookie` values) and
//!   the whole body (`amoeba_evidence::http` explains why the whole body).

use std::future::IntoFuture;
use std::time::{Duration, Instant};

use futures::{AsyncRead, AsyncWrite};
use http_body_util::{BodyExt, Empty};
use hyper::{Request, body::Bytes};
use hyper_util::rt::TokioIo;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

use tlsn::{
    Session,
    attestation::{
        Attestation, CryptoProvider, Extension, Secrets,
        presentation::Presentation,
        request::{Request as AttestationRequest, RequestConfig},
    },
    config::{
        prove::ProveConfig, prover::ProverConfig, tls::TlsClientConfig,
        tls_commit::mpc::MpcTlsConfig,
    },
    connection::{HandshakeData, ServerName},
    prover::ProverOutput,
    transcript::TranscriptCommitConfig,
    webpki::{CertificateDer, RootCertStore},
};
use tlsn_formats::http::{DefaultHttpCommitter, HttpCommit, HttpTranscript};

use crate::wire::{self, WireError};
use amoeba_evidence::tlsn::{BINDING_EXTENSION, bincode_options};

/// Default preprocessing sizes. MPC cost grows with them; the notary and
/// the verifier policy cap them at 8 KiB / 64 KiB
/// (`amoeba_evidence::policy::DEFAULT_LIMITS`).
pub const DEFAULT_MAX_SENT: usize = 4 * 1024;
pub const DEFAULT_MAX_RECV: usize = 16 * 1024;

/// Request headers whose values are disclosed; every other value is hidden.
const DISCLOSED_REQUEST_HEADERS: &[&str] = &["host", "accept", "accept-encoding", "connection"];

#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    /// `/path?query`
    pub target: String,
}

impl Target {
    /// `https://host[:port]/path?query` (no userinfo, no fragment).
    pub fn parse(url: &str) -> Result<Self, ProveError> {
        let bad = || ProveError::Url(url.to_string());
        let rest = url.strip_prefix("https://").ok_or_else(bad)?;
        if rest.contains(['#', '@', '\\']) || rest.bytes().any(|c| c <= b' ' || c >= 0x7f) {
            return Err(bad());
        }
        let (authority, target) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_string()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| bad())?),
            None => (authority, 443),
        };
        if host.is_empty() || host != host.to_ascii_lowercase() {
            return Err(bad());
        }
        Ok(Self {
            host: host.to_string(),
            port,
            target,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProveRequest {
    pub url: String,
    /// Extra headers (`Cookie`, `Authorization`, `User-Agent`…). Their
    /// values are sent to the source only and never disclosed.
    pub headers: Vec<(String, String)>,
    /// Connect here instead of resolving the host (tests, local fixtures).
    pub connect_to: Option<String>,
    /// Extra trusted roots (DER) for the source's certificate; empty means
    /// Mozilla's roots only.
    pub extra_roots: Vec<Vec<u8>>,
    /// The wallet to bind the proof to (an attestation extension).
    pub binding: Option<[u8; 32]>,
    pub max_sent: usize,
    pub max_recv: usize,
}

impl ProveRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            connect_to: None,
            extra_roots: Vec::new(),
            binding: None,
            max_sent: DEFAULT_MAX_SENT,
            max_recv: DEFAULT_MAX_RECV,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProveError {
    #[error("`{0}` is not an https URL petri can prove")]
    Url(String),
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("the source could not be reached: {0}")]
    Source(String),
    #[error("the TLSNotary session failed: {0}")]
    Session(String),
    #[error("the notary's attestation does not match this session: {0}")]
    BadAttestation(String),
}

fn session_err(e: impl std::fmt::Display) -> ProveError {
    ProveError::Session(e.to_string())
}

/// Wall-clock phases, for measurement (`petri evidence web prove --json`).
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    pub setup_ms: u128,
    pub request_ms: u128,
    pub prove_ms: u128,
    pub attest_ms: u128,
    pub total_ms: u128,
}

pub struct Notarized {
    pub attestation: Attestation,
    pub secrets: Secrets,
    pub notary_key: String,
    pub status: u16,
    pub timings: Timings,
}

impl std::fmt::Debug for Notarized {
    // Never print the secrets: they hold the full transcript, cookies included.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notarized")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Runs one MPC-TLS session with the notary on `notary`, sends the request
/// to the source and returns the attestation with its secrets.
pub async fn notarize<S>(mut notary: S, req: &ProveRequest) -> Result<Notarized, ProveError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let t0 = Instant::now();
    let target = Target::parse(&req.url)?;
    wire::write_line(&mut notary, &wire::hello_line()).await?;
    let reply = wire::read_line(&mut notary).await?;
    let notary_key = match reply.strip_prefix("ok secp256k1 ") {
        Some(key) => key.to_string(),
        None => {
            return Err(WireError::Refused(
                reply.strip_prefix("no ").unwrap_or(&reply).to_string(),
            )
            .into());
        }
    };

    let session = Session::new(notary);
    let (driver, mut handle) = session.split();
    let driver_task = tokio::spawn(driver);

    let prover = handle
        .new_prover(ProverConfig::builder().build().map_err(session_err)?)
        .map_err(session_err)?
        .commit(
            MpcTlsConfig::builder()
                .max_sent_data(req.max_sent)
                .max_recv_data(req.max_recv)
                .build()
                .map_err(session_err)?,
        )
        .await
        .map_err(session_err)?;
    let setup = t0.elapsed();

    let addr = req
        .connect_to
        .clone()
        .unwrap_or_else(|| format!("{}:{}", target.host, target.port));
    let tcp = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| ProveError::Source(format!("{addr}: {e}")))?;
    tcp.set_nodelay(true)
        .map_err(|e| ProveError::Source(e.to_string()))?;

    let mut roots = RootCertStore::mozilla();
    roots
        .roots
        .extend(req.extra_roots.iter().map(|d| CertificateDer(d.clone())));
    let server_name = ServerName::Dns(
        target
            .host
            .as_str()
            .try_into()
            .map_err(|_| ProveError::Url(req.url.clone()))?,
    );
    let (tls, prover) = prover
        .connect(
            TlsClientConfig::builder()
                .server_name(server_name.clone())
                .root_store(roots)
                .build()
                .map_err(session_err)?,
            tcp.compat(),
        )
        .map_err(session_err)?;
    let tls = TokioIo::new(tls.compat());
    let prover_task = tokio::spawn(prover.into_future());

    let (mut sender, connection) = hyper::client::conn::http1::handshake(tls)
        .await
        .map_err(|e| ProveError::Source(e.to_string()))?;
    tokio::spawn(connection);
    let mut builder = Request::builder()
        .uri(target.target.as_str())
        .header("Host", target.host.as_str())
        .header("Accept", "application/json")
        // TLSNotary cannot read compressed bodies.
        .header("Accept-Encoding", "identity")
        .header("Connection", "close");
    for (k, v) in &req.headers {
        let lower = k.to_ascii_lowercase();
        if !matches!(
            lower.as_str(),
            "host" | "accept-encoding" | "connection" | "content-length"
        ) {
            builder = builder.header(k.as_str(), v.as_str());
        }
    }
    let request = builder
        .body(Empty::<Bytes>::new())
        .map_err(|e| ProveError::Source(e.to_string()))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| ProveError::Source(e.to_string()))?;
    let status = response.status().as_u16();
    // Drain the body so the whole response is in the transcript.
    let _ = response.into_body().collect().await;
    let mut prover = prover_task
        .await
        .map_err(session_err)?
        .map_err(session_err)?;
    let request_done = t0.elapsed();

    let transcript = HttpTranscript::parse(prover.transcript()).map_err(session_err)?;
    let mut commit = TranscriptCommitConfig::builder(prover.transcript());
    DefaultHttpCommitter::default()
        .commit_transcript(&mut commit, &transcript)
        .map_err(session_err)?;
    let mut rc = RequestConfig::builder();
    rc.transcript_commit(commit.build().map_err(session_err)?);
    if let Some(wallet) = req.binding {
        rc.extension(Extension {
            id: BINDING_EXTENSION.to_vec(),
            value: wallet.to_vec(),
        });
    }
    let request_config = rc.build().map_err(session_err)?;
    let mut pb = ProveConfig::builder(prover.transcript());
    if let Some(c) = request_config.transcript_commit() {
        pb.transcript_commit(c.clone());
    }
    let ProverOutput {
        transcript_commitments,
        transcript_secrets,
        ..
    } = prover
        .prove(&pb.build().map_err(session_err)?)
        .await
        .map_err(session_err)?;
    let prover_transcript = prover.transcript().clone();
    let tls_transcript = prover.tls_transcript().clone();
    prover.close().await.map_err(session_err)?;
    let proved = t0.elapsed();

    let mut ab = AttestationRequest::builder(&request_config);
    ab.server_name(server_name)
        .handshake_data(HandshakeData {
            certs: tls_transcript
                .server_cert_chain()
                .ok_or_else(|| session_err("no server certificate chain"))?
                .to_vec(),
            sig: tls_transcript
                .server_signature()
                .ok_or_else(|| session_err("no server signature"))?
                .clone(),
            binding: tls_transcript.certificate_binding().clone(),
        })
        .transcript(prover_transcript)
        .transcript_commitments(transcript_secrets, transcript_commitments);
    let provider = CryptoProvider::default();
    let (attestation_request, secrets) = ab.build(&provider).map_err(session_err)?;

    handle.close();
    let mut io = driver_task
        .await
        .map_err(session_err)?
        .map_err(session_err)?;
    use bincode::Options;
    let bytes = bincode_options(wire::MAX_FRAME)
        .serialize(&attestation_request)
        .map_err(session_err)?;
    wire::write_frame(&mut io, &bytes).await?;
    let reply = wire::read_frame(&mut io).await?;
    let attestation: Attestation = bincode_options(wire::MAX_FRAME)
        .deserialize(&reply)
        .map_err(|e| ProveError::BadAttestation(e.to_string()))?;
    attestation_request
        .validate(&attestation, &provider)
        .map_err(|e| ProveError::BadAttestation(e.to_string()))?;
    let total = t0.elapsed();
    let ms = |d: Duration| d.as_millis();
    Ok(Notarized {
        attestation,
        secrets,
        notary_key,
        status,
        timings: Timings {
            setup_ms: ms(setup),
            request_ms: ms(request_done - setup),
            prove_ms: ms(proved - request_done),
            attest_ms: ms(total - proved),
            total_ms: ms(total),
        },
    })
}

/// Builds the presentation under the fixed disclosure rules (module docs).
pub fn present(n: &Notarized) -> Result<Vec<u8>, ProveError> {
    let transcript = HttpTranscript::parse(n.secrets.transcript()).map_err(session_err)?;
    let mut b = n.secrets.transcript_proof_builder();
    let request = transcript
        .requests
        .first()
        .ok_or_else(|| session_err("no request"))?;
    b.reveal_sent(request.without_data()).map_err(session_err)?;
    b.reveal_sent(&request.request.target)
        .map_err(session_err)?;
    for h in &request.headers {
        let name = h.name.as_str().to_ascii_lowercase();
        if DISCLOSED_REQUEST_HEADERS.contains(&name.as_str()) {
            b.reveal_sent(h).map_err(session_err)?;
        } else {
            b.reveal_sent(h.without_value()).map_err(session_err)?;
        }
    }
    let response = transcript
        .responses
        .first()
        .ok_or_else(|| session_err("no response"))?;
    b.reveal_recv(response.without_data())
        .map_err(session_err)?;
    for h in &response.headers {
        if h.name.as_str().eq_ignore_ascii_case("set-cookie") {
            b.reveal_recv(h.without_value()).map_err(session_err)?;
        } else {
            b.reveal_recv(h).map_err(session_err)?;
        }
    }
    if let Some(body) = &response.body {
        b.reveal_recv(body).map_err(session_err)?;
    }
    let proof = b.build().map_err(session_err)?;
    let provider = CryptoProvider::default();
    let mut pb = n.attestation.presentation_builder(&provider);
    pb.identity_proof(n.secrets.identity_proof())
        .transcript_proof(proof);
    let presentation: Presentation = pb.build().map_err(session_err)?;
    use bincode::Options;
    bincode_options(amoeba_evidence::policy::DEFAULT_LIMITS.max_presentation_bytes)
        .serialize(&presentation)
        .map_err(session_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        let t = Target::parse("https://portal.example.com/api/prices?sku=A").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.target.as_str()),
            ("portal.example.com", 443, "/api/prices?sku=A")
        );
        let t = Target::parse("https://a.test:8443?x=1").unwrap();
        assert_eq!((t.port, t.target.as_str()), (8443, "/?x=1"));
        for bad in [
            "http://a.com/",
            "https://u@a.com/",
            "https://A.com/",
            "https://a.com/#x",
            "https://a.com:x/",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
    }
}
