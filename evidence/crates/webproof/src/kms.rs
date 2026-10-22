//! A notary key held in Google Cloud KMS: an `EC_SIGN_SECP256K1_SHA256` key
//! version whose secret never leaves KMS (secp256k1 keys are HSM-only
//! there). The notary asks KMS to sign each attestation, with the access
//! token of the machine's own service account from the metadata server, so
//! the notary host holds no key material at all.
//!
//! Every signature KMS returns is normalised to low S (KMS may return
//! either; TLSNotary's verifier takes only low S) and checked against the
//! key version's public key before the notary uses it.

use std::time::{Duration, Instant};

use base64::Engine;
use http_body_util::{BodyExt, Full};
use hyper::{Request, body::Bytes};
use hyper_util::rt::TokioIo;
use k256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use sha2::{Digest, Sha256};

const KMS_HOST: &str = "cloudkms.googleapis.com";
const METADATA_ADDR: &str = "169.254.169.254:80";
const TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
/// Per call to KMS or the metadata server.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// The most of a reply read from either.
const MAX_REPLY: usize = 64 * 1024;
/// DER `SubjectPublicKeyInfo` header of an uncompressed secp256k1 key
/// (id-ecPublicKey, secp256k1), followed by the 65-byte point.
const SPKI_SECP256K1_PREFIX: [u8; 23] = [
    0x30, 0x56, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
    0x81, 0x04, 0x00, 0x0a, 0x03, 0x42, 0x00,
];

/// A KMS key version that signs for the notary.
pub struct KmsSigner {
    name: String,
    key: VerifyingKey,
    api: Api,
}

/// KMS's REST API, as the machine's service account.
#[derive(Default)]
struct Api {
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
}

impl KmsSigner {
    /// Reads the key version's public key and checks its algorithm.
    /// `name` is `projects/…/locations/…/keyRings/…/cryptoKeys/…/cryptoKeyVersions/…`.
    pub async fn connect(name: &str) -> anyhow::Result<Self> {
        check_name(name)?;
        let api = Api::default();
        let reply = api
            .call("GET", &format!("/v1/{name}/publicKey"), None)
            .await?;
        let algorithm = reply["algorithm"].as_str().unwrap_or_default();
        if algorithm != "EC_SIGN_SECP256K1_SHA256" {
            anyhow::bail!("{name} is a {algorithm} key, not EC_SIGN_SECP256K1_SHA256");
        }
        let pem = reply["pem"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("KMS returned no public key for {name}"))?;
        Ok(Self {
            name: name.to_string(),
            key: public_key_from_pem(pem)?,
            api,
        })
    }

    /// The compressed SEC1 public key a verifier policy lists.
    pub fn public_key(&self) -> Vec<u8> {
        self.key.to_encoded_point(true).as_bytes().to_vec()
    }

    /// Signs `msg` (SHA-256 by KMS): the 64-byte `r ‖ s` signature, low S,
    /// checked against the public key.
    pub async fn sign(&self, msg: &[u8]) -> anyhow::Result<[u8; 64]> {
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(msg));
        let body = serde_json::json!({ "digest": { "sha256": digest } }).to_string();
        let reply = self
            .api
            .call(
                "POST",
                &format!("/v1/{}:asymmetricSign", self.name),
                Some(body),
            )
            .await?;
        let der = base64::engine::general_purpose::STANDARD
            .decode(reply["signature"].as_str().unwrap_or_default())
            .map_err(|_| anyhow::anyhow!("KMS returned no signature"))?;
        signature_from_der(&self.key, msg, &der)
    }
}

impl Api {
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let token = self.token().await?;
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", KMS_HOST)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body.unwrap_or_default())))?;
        let (status, bytes) = tokio::time::timeout(CALL_TIMEOUT, https(req))
            .await
            .map_err(|_| {
                anyhow::anyhow!("KMS did not answer within {} s", CALL_TIMEOUT.as_secs())
            })??;
        let reply: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        if status != 200 {
            let message = reply["error"]["message"].as_str().unwrap_or("no message");
            anyhow::bail!("KMS refused ({status}): {message}");
        }
        Ok(reply)
    }

    /// The service account's access token, cached until a minute before it
    /// expires.
    async fn token(&self) -> anyhow::Result<String> {
        let mut cached = self.token.lock().await;
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let req = Request::builder()
            .uri(TOKEN_PATH)
            .header("host", "metadata.google.internal")
            .header("metadata-flavor", "Google")
            .body(Full::new(Bytes::new()))?;
        let (status, bytes) = tokio::time::timeout(CALL_TIMEOUT, async {
            let tcp = tokio::net::TcpStream::connect(METADATA_ADDR).await?;
            send(TokioIo::new(tcp), req).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("the metadata server did not answer (is this a GCE VM?)"))??;
        if status != 200 {
            anyhow::bail!("the metadata server refused a token ({status})");
        }
        let reply: serde_json::Value = serde_json::from_slice(&bytes)?;
        let token = reply["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("the metadata server returned no token"))?
            .to_string();
        let life = reply["expires_in"].as_u64().unwrap_or(0).saturating_sub(60);
        *cached = Some((token.clone(), Instant::now() + Duration::from_secs(life)));
        Ok(token)
    }
}

/// Only resource-name characters, in the key-version shape, so nothing
/// else can reach the request path.
fn check_name(name: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = name.split('/').collect();
    let shape = [
        "projects",
        "locations",
        "keyRings",
        "cryptoKeys",
        "cryptoKeyVersions",
    ];
    let ok = parts.len() == 10
        && parts.chunks(2).zip(shape).all(|(pair, label)| {
            pair[0] == label
                && !pair[1].is_empty()
                && pair[1]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        });
    if !ok {
        anyhow::bail!(
            "`{name}` is not a KMS key version (projects/…/locations/…/keyRings/…/cryptoKeys/…/cryptoKeyVersions/…)"
        );
    }
    Ok(())
}

fn public_key_from_pem(pem: &str) -> anyhow::Result<VerifyingKey> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| anyhow::anyhow!("KMS's public key is not PEM"))?;
    let point = der
        .strip_prefix(&SPKI_SECP256K1_PREFIX[..])
        .filter(|p| p.len() == 65)
        .ok_or_else(|| anyhow::anyhow!("KMS's public key is not a secp256k1 key"))?;
    VerifyingKey::from_sec1_bytes(point).map_err(|_| anyhow::anyhow!("KMS's public key is invalid"))
}

/// KMS's DER signature as TLSNotary takes it: `r ‖ s` with low S, and only
/// if it verifies.
fn signature_from_der(key: &VerifyingKey, msg: &[u8], der: &[u8]) -> anyhow::Result<[u8; 64]> {
    let sig =
        Signature::from_der(der).map_err(|_| anyhow::anyhow!("KMS's signature is not DER"))?;
    let sig = sig.normalize_s().unwrap_or(sig);
    key.verify(msg, &sig)
        .map_err(|_| anyhow::anyhow!("KMS's signature does not verify against its public key"))?;
    Ok(sig.to_bytes().into())
}

async fn https(req: Request<Full<Bytes>>) -> anyhow::Result<(u16, Vec<u8>)> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let tcp = tokio::net::TcpStream::connect((KMS_HOST, 443)).await?;
    let server = rustls::pki_types::ServerName::try_from(KMS_HOST)?;
    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(server, tcp)
        .await?;
    send(TokioIo::new(tls), req).await
}

async fn send<T>(io: T, req: Request<Full<Bytes>>) -> anyhow::Result<(u16, Vec<u8>)>
where
    T: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    let conn = tokio::spawn(conn);
    let res = sender.send_request(req).await?;
    let status = res.status().as_u16();
    let body = http_body_util::Limited::new(res.into_body(), MAX_REPLY)
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("reading the reply: {e}"))?
        .to_bytes()
        .to_vec();
    conn.abort();
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::{SigningKey, signature::Signer};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32].into()).unwrap()
    }

    fn pem_of(key: &SigningKey) -> String {
        pem_with(&SPKI_SECP256K1_PREFIX, key)
    }

    fn pem_with(prefix: &[u8], key: &SigningKey) -> String {
        let mut der = prefix.to_vec();
        der.extend_from_slice(key.verifying_key().to_encoded_point(false).as_bytes());
        let b64 = base64::engine::general_purpose::STANDARD.encode(der);
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n{}\n-----END PUBLIC KEY-----\n",
            &b64[..64],
            &b64[64..]
        )
    }

    #[test]
    fn reads_a_kms_secp256k1_public_key() {
        let k = key();
        assert_eq!(
            public_key_from_pem(&pem_of(&k)).unwrap(),
            *k.verifying_key()
        );
        // Another curve's OID (secp384r1, 1.3.132.0.34) is refused.
        let mut prefix = SPKI_SECP256K1_PREFIX;
        prefix[19] = 0x22;
        assert!(public_key_from_pem(&pem_with(&prefix, &k)).is_err());
        let garbage = "-----BEGIN PUBLIC KEY-----\nnot base64\n-----END PUBLIC KEY-----";
        assert!(public_key_from_pem(garbage).is_err());
    }

    #[test]
    fn high_s_signatures_become_low_s_and_must_verify() {
        let k = key();
        let msg = b"attestation header";
        let low: Signature = k.sign(msg);
        let (r, s) = low.split_scalars();
        let high = Signature::from_scalars(r.to_bytes(), (-*s).to_bytes()).unwrap();
        assert!(
            high.normalize_s().is_some(),
            "the test signature has high S"
        );
        for der in [low.to_der(), high.to_der()] {
            let out = signature_from_der(k.verifying_key(), msg, der.as_bytes()).unwrap();
            assert_eq!(out, <[u8; 64]>::from(low.to_bytes()));
            // TLSNotary's verifier accepts it.
            let sig = Signature::from_slice(&out).unwrap();
            k.verifying_key().verify(msg, &sig).unwrap();
        }
        // Another message, or another key, is refused.
        assert!(signature_from_der(k.verifying_key(), b"other", low.to_der().as_bytes()).is_err());
        let other = SigningKey::from_bytes(&[9u8; 32].into()).unwrap();
        assert!(signature_from_der(other.verifying_key(), msg, low.to_der().as_bytes()).is_err());
        assert!(signature_from_der(k.verifying_key(), msg, b"\x30\x00").is_err());
    }

    #[test]
    fn only_key_version_names_reach_the_path() {
        check_name("projects/amoeba-hm0q2k/locations/global/keyRings/ring/cryptoKeys/evidence-notary-v1/cryptoKeyVersions/1").unwrap();
        for bad in [
            "projects/p/locations/global/keyRings/r/cryptoKeys/k",
            "projects/p/locations/global/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1/x/y",
            "projects/p/locations/global/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1?alt=x",
            "projects/p/locations/glo bal/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1",
            "projects/../locations/global/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1",
            "projects/p/zones/global/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1",
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
