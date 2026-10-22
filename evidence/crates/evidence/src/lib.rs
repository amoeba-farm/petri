//! Authenticated oracle evidence.
//!
//! A contributor proves what an HTTPS source showed them (`web_proof`,
//! TLSNotary), including pages behind their own sign-in, and submits that
//! proof through the oracle's existing paths. This crate is the one place
//! the proof is checked: petri (before export or submission), the futures
//! API (on submission, so a client can't skip it) and anyone re-checking an
//! archived proof run the same `verify_bundle`.
//!
//! Each check is a separate stage, and none of them is the oracle's
//! decision:
//!
//! ```text
//! received → provenance verification → extraction → eligibility → (authoritative oracle outcome)
//! ```
//!
//! - **received**: the bundle parses (format, sizes).
//! - **provenance**: the TLSNotary presentation verifies under the verifier
//!   policy (`policy`): trusted notary, server certificate, disclosed bytes.
//! - **extraction**: the manifest reads exactly one record from the
//!   authenticated response (`extract`).
//! - **eligibility**: the evidence fits the place it is submitted to
//!   (`check_destination`): same locator, same frozen selector, a
//!   publishable locator, the binding to the submitter.
//! - The oracle's outcome (sealed source, accepted update, challenge,
//!   council) is read from chain, never inferred from these stages.
//!
//! Authentication is not market truth: a genuine server can return a
//! negotiated quote, a user-entered field or an echo. The economic type
//! (catalog price, quotation, benchmark, transaction) stays what the record
//! is; an authenticated quote is still a quote, with no extra weight.

pub mod bundle;
pub mod decimal;
pub mod dkim;
pub mod email;
pub mod email_zk;
pub mod extract;
pub mod hash;
pub mod http;
pub mod ids;
pub mod json;
pub mod manifest;
pub mod policy;
pub mod tlsn;

use serde::Serialize;

use crate::bundle::{Bundle, Method};
use crate::extract::{Extracted, Field, Provenance};
use crate::hash::hex32;
use crate::http::HeaderView;
use crate::manifest::EvidenceType;
use crate::policy::{Policy, Trust};
use crate::tlsn::ServerTrust;

pub const ENVELOPE_FORMAT: &str = "amoeba-authenticated-evidence-v1";
pub const VERIFICATION_FORMAT: &str = "amoeba-evidence-verification-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
    /// Well-formed, but uses something this verifier does not support.
    Unsupported,
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stage {
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Stage {
    pub fn passed() -> Self {
        Self {
            outcome: Outcome::Passed,
            reason: None,
        }
    }
    pub fn not_checked() -> Self {
        Self {
            outcome: Outcome::NotChecked,
            reason: None,
        }
    }
    fn with(outcome: Outcome, reason: impl ToString) -> Self {
        Self {
            outcome,
            reason: Some(reason.to_string()),
        }
    }
    pub fn ok(&self) -> bool {
        self.outcome == Outcome::Passed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stages {
    pub received: Stage,
    pub provenance: Stage,
    pub extraction: Stage,
    pub eligibility: Stage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Authentication {
    pub method: Method,
    pub backend: String,
    pub backend_revision: String,
    pub mode: String,
    pub notary: String,
    pub notary_alg: String,
    pub notary_key: String,
    /// The weakest trust the check relied on (development notary or root
    /// makes the whole result development trust).
    pub trust: Trust,
    pub server_trust: ServerTrust,
    pub verifier_policy: String,
    pub verifier_policy_digest: String,
    pub proof_digest: String,
    pub bundle_digest: String,
    /// The wallet the notary bound the proof to in the session, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub server_name: String,
    /// `https://<server name><request target>`: the exact resource proven.
    pub locator: String,
    pub method: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionInfo {
    pub manifest_format: String,
    pub manifest_name: String,
    pub manifest_hash: String,
    pub selector_digest: String,
    pub record_pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    /// As the source printed it.
    pub text: String,
    /// Six-decimal atoms (`41.25` → 41 250 000), derived exactly.
    pub atoms: u64,
    pub provenance: Provenance,
    pub pointer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    /// The economic kind of record, as the manifest declares it.
    pub evidence_type: EvidenceType,
    pub price: Price,
    /// Named record fields (authenticated) and stated assumptions
    /// (contributor annotations), each with its provenance.
    pub fields: std::collections::BTreeMap<String, Field>,
    /// Context pointer → value (authenticated commercial qualifiers).
    pub context: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Time {
    pub value: String,
    pub provenance: TimeProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeProvenance {
    /// When the TLS session started, signed by the notary (±5 s of its
    /// clock). When the source served the record, not when the economic
    /// observation happened.
    NotaryAttestedSessionTime,
    /// A date the source itself printed in the authenticated record. The
    /// source's statement, not an independently verified time.
    SourceStatedInRecord,
    /// The clock of whoever recorded the submission.
    RecorderClock,
    /// A DKIM signature's own `t=`: the signer's statement of when it
    /// signed, not a verified delivery time.
    SignerStatedSignatureTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Times {
    /// Unix seconds.
    pub witnessed_at: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_observed_at: Option<Time>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<Time>,
}

/// What the evidence says, recomputed from the proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub format: &'static str,
    pub authentication: Authentication,
    pub origin: Origin,
    pub extraction: ExtractionInfo,
    pub observation: Observation,
    pub times: Times,
    pub observation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission_id: Option<String>,
    /// The contributor's unverified notes.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub annotations: std::collections::BTreeMap<String, String>,
}

/// Exactly what the bundle discloses, for the preview before export or
/// submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disclosure {
    pub server_name: String,
    /// The request as disclosed; hidden bytes show as `·`.
    pub request: String,
    /// Request headers whose values stay hidden (cookies, authorization).
    pub hidden_request_headers: Vec<String>,
    pub response_headers: Vec<HeaderView>,
    /// The response body as disclosed (all of it).
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub format: &'static str,
    pub stages: Stages,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosure: Option<Disclosure>,
    /// The parsed bundle digest (also when a later stage fails).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_digest: Option<String>,
}

impl Verification {
    /// Provenance and extraction both passed (eligibility is separate).
    pub fn authenticated(&self) -> bool {
        self.stages.received.ok() && self.stages.provenance.ok() && self.stages.extraction.ok()
    }

    fn failed(
        received: Stage,
        provenance: Stage,
        extraction: Stage,
        digest: Option<String>,
    ) -> Self {
        Self {
            format: VERIFICATION_FORMAT,
            stages: Stages {
                received,
                provenance,
                extraction,
                eligibility: Stage::not_checked(),
            },
            envelope: None,
            disclosure: None,
            bundle_digest: digest,
        }
    }
}

/// Runs the received, provenance and extraction stages on bundle bytes.
/// Eligibility stays `not_checked` until `check_destination`.
pub fn verify_bundle(bytes: &[u8], policy: &Policy) -> Verification {
    let nc = Stage::not_checked;
    match bundle::peek_method(bytes).as_deref() {
        Some("email_dkim") => {
            return match bundle::EmailBundle::parse(bytes, policy.limits.max_bundle_bytes) {
                Ok(b) => verify_email(&b, policy),
                Err(e) => Verification::failed(Stage::with(Outcome::Failed, e), nc(), nc(), None),
            };
        }
        Some("email_dkim_zk") => {
            return match bundle::EmailZkBundle::parse(bytes, policy.limits.max_bundle_bytes) {
                Ok(b) => verify_email_zk(&b, policy),
                Err(e) => Verification::failed(Stage::with(Outcome::Failed, e), nc(), nc(), None),
            };
        }
        _ => {}
    }
    let bundle = match Bundle::parse(bytes, policy.limits.max_bundle_bytes) {
        Ok(b) => b,
        Err(e) => {
            let unsupported = e.0.contains("not supported");
            let outcome = if unsupported {
                Outcome::Unsupported
            } else {
                Outcome::Failed
            };
            return Verification::failed(Stage::with(outcome, e), nc(), nc(), None);
        }
    };
    verify_parsed(&bundle, policy)
}

pub fn verify_parsed(bundle: &Bundle, policy: &Policy) -> Verification {
    let nc = Stage::not_checked;
    let digest = Some(hex32(&bundle.digest()));
    let received = Stage::passed();
    if bundle.backend_revision != policy::TLSN_REVISION {
        return Verification::failed(
            received,
            Stage::with(
                Outcome::Unsupported,
                format!(
                    "this verifier checks TLSNotary {}, the proof says {}",
                    policy::TLSN_REVISION,
                    bundle.backend_revision
                ),
            ),
            nc(),
            digest,
        );
    }
    let session = match tlsn::verify(&bundle.presentation, policy) {
        Ok(s) => s,
        Err(e) => {
            return Verification::failed(received, Stage::with(Outcome::Failed, e), nc(), digest);
        }
    };
    let request = match http::parse_request(&session.sent) {
        Ok(r) => r,
        Err(e) => {
            return Verification::failed(
                received,
                Stage::passed(),
                Stage::with(outcome_of(&e), e),
                digest,
            );
        }
    };
    let disclosure_partial = |body: String, headers: Vec<HeaderView>| Disclosure {
        server_name: session.server_name.clone(),
        request: session.sent.display(),
        hidden_request_headers: request
            .headers
            .iter()
            .filter(|h| h.value.is_none())
            .map(|h| h.name.clone())
            .collect(),
        response_headers: headers,
        body,
    };
    if request.host != session.server_name {
        let mut v = Verification::failed(
            received,
            Stage::passed(),
            Stage::with(
                Outcome::Failed,
                format!(
                    "the request names host {}, the TLS session was with {}",
                    request.host, session.server_name
                ),
            ),
            digest,
        );
        v.disclosure = Some(disclosure_partial(session.recv.display(), vec![]));
        return v;
    }
    let response = match http::parse_response(&session.recv) {
        Ok(r) => r,
        Err(e) => {
            let mut v = Verification::failed(
                received,
                Stage::passed(),
                Stage::with(outcome_of(&e), e),
                digest,
            );
            v.disclosure = Some(disclosure_partial(session.recv.display(), vec![]));
            return v;
        }
    };
    let disclosure = disclosure_partial(
        String::from_utf8_lossy(&response.body).into_owned(),
        response.headers.clone(),
    );
    let x: Extracted = match extract::extract(&bundle.manifest, &request, &response) {
        Ok(x) => x,
        Err(e) => {
            let mut v = Verification::failed(
                received,
                Stage::passed(),
                Stage::with(Outcome::Failed, e),
                digest,
            );
            v.disclosure = Some(disclosure);
            return v;
        }
    };
    let m = &bundle.manifest;
    let val = |k: &str| x.value(k).unwrap_or("").to_string();
    let path = extract::request_path(&request.target).to_string();
    let observation_id = ids::observation_id(&ids::ObservationKey {
        server_name: &session.server_name,
        path: &path,
        selector_digest: m.selector_digest(),
        record_id: &val("recordId"),
        revision: &val("revision"),
        value_atoms: x.price_atoms,
        currency: &val("currency"),
        unit: &val("unit"),
        quantity_tier: &val("quantityTier"),
        source_observed_at: &val("observedAt"),
    });
    let source_observed_at = x.get("observedAt").map(|f| Time {
        value: f.value.clone(),
        provenance: TimeProvenance::SourceStatedInRecord,
    });
    let envelope = Envelope {
        format: ENVELOPE_FORMAT,
        authentication: Authentication {
            method: bundle.method,
            backend: bundle.backend_name.clone(),
            backend_revision: bundle.backend_revision.clone(),
            mode: bundle.backend_mode.clone(),
            notary: session.notary.name.clone(),
            notary_alg: session.notary.alg.clone(),
            notary_key: hex::encode(&session.notary.key),
            trust: session.trust(),
            server_trust: session.server_trust,
            verifier_policy: policy.id.clone(),
            verifier_policy_digest: hex32(&policy.digest()),
            proof_digest: hex32(&bundle.proof_digest()),
            bundle_digest: hex32(&bundle.digest()),
            bound_to: session.binding.map(|b| hex32(&b)),
        },
        origin: Origin {
            server_name: session.server_name.clone(),
            locator: format!("https://{}{}", session.server_name, request.target),
            method: request.method.clone(),
            status: response.status,
            content_type: response.content_type.clone(),
        },
        extraction: ExtractionInfo {
            manifest_format: manifest::MANIFEST_FORMAT.into(),
            manifest_name: m.name.clone(),
            manifest_hash: hex32(&m.manifest_hash()),
            selector_digest: hex32(&m.selector_digest()),
            record_pointer: x.record_pointer.clone(),
        },
        observation: Observation {
            evidence_type: m.evidence_type,
            price: Price {
                text: x.price.value.clone(),
                atoms: x.price_atoms,
                provenance: Provenance::Authenticated,
                pointer: x.price.pointer.clone().unwrap_or_default(),
            },
            fields: x.fields.iter().cloned().collect(),
            context: x.context.iter().cloned().collect(),
        },
        times: Times {
            witnessed_at: Time {
                value: session.time.to_string(),
                provenance: TimeProvenance::NotaryAttestedSessionTime,
            },
            source_observed_at,
            submitted_at: None,
        },
        observation_id: hex32(&observation_id),
        submission_id: None,
        annotations: bundle.annotations.iter().cloned().collect(),
    };
    Verification {
        format: VERIFICATION_FORMAT,
        stages: Stages {
            received,
            provenance: Stage::passed(),
            extraction: Stage::passed(),
            eligibility: Stage::not_checked(),
        },
        envelope: Some(envelope),
        disclosure: Some(disclosure),
        bundle_digest: digest,
    }
}

/// The email method: DKIM under the policy's keys, then the manifest's
/// labelled lines from the signed text.
pub fn verify_email(b: &bundle::EmailBundle, policy: &Policy) -> Verification {
    use email::EmailError;
    let nc = Stage::not_checked;
    let digest = Some(hex32(&b.digest()));
    let m = &b.manifest;
    let e = match email::verify_message(&b.message, &m.signing_domain, &policy.dkim_keys) {
        Ok(e) => e,
        Err(err) => {
            let outcome = match err {
                EmailError::Unsupported(_) => Outcome::Unsupported,
                _ => Outcome::Failed,
            };
            // Body problems come after a good signature: provenance passed.
            let (prov, ext) = match err {
                EmailError::Body(_) | EmailError::Unsupported(_) => {
                    (Stage::passed(), Stage::with(outcome, err))
                }
                _ => (Stage::with(outcome, err), nc()),
            };
            return Verification::failed(Stage::passed(), prov, ext, digest);
        }
    };
    let disclosure = Disclosure {
        server_name: e.key.domain.clone(),
        request: String::new(),
        hidden_request_headers: Vec::new(),
        response_headers: e
            .verified
            .signed_headers
            .iter()
            .map(|(n, v)| HeaderView {
                name: n.clone(),
                value: Some(v.clone()),
            })
            .collect(),
        body: String::from_utf8_lossy(&b.message).into_owned(),
    };
    let (atoms, values) = match email::extract(m, &e) {
        Ok(x) => x,
        Err(err) => {
            let mut v = Verification::failed(
                Stage::passed(),
                Stage::passed(),
                Stage::with(Outcome::Failed, err),
                digest,
            );
            v.disclosure = Some(disclosure);
            return v;
        }
    };
    let val = |k: &str| {
        values
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let (fields, observation_id) = email_fields(m, &e.key.domain, &values, atoms);
    let mut context = std::collections::BTreeMap::new();
    if let Some(f) = &e.from {
        context.insert("signed From".to_string(), f.clone());
    }
    context.insert(
        "From is in the signing domain".to_string(),
        e.from_aligned.to_string(),
    );
    if let Some(s) = &e.subject {
        context.insert("signed Subject".to_string(), s.clone());
    }
    if let Some(d) = &e.date {
        context.insert("signed Date (the sender's claim)".to_string(), d.clone());
    }
    let sig = &e.verified.signature;
    let envelope = Envelope {
        format: ENVELOPE_FORMAT,
        authentication: Authentication {
            method: Method::EmailDkim,
            backend: "dkim".into(),
            backend_revision: "rfc6376".into(),
            mode: sig.algorithm.as_str().into(),
            notary: format!(
                "{}._domainkey.{} ({})",
                e.key.selector, e.key.domain, e.key.provenance
            ),
            notary_alg: sig.algorithm.as_str().into(),
            notary_key: hex32(&hash::sha256(e.key.record.as_bytes())),
            trust: e.key.trust,
            server_trust: ServerTrust::DkimKeyPolicy,
            verifier_policy: policy.id.clone(),
            verifier_policy_digest: hex32(&policy.digest()),
            proof_digest: hex32(&hash::sha256(&b.message)),
            bundle_digest: hex32(&b.digest()),
            bound_to: None,
        },
        origin: Origin {
            server_name: e.key.domain.clone(),
            locator: format!("dkim:{}._domainkey.{}", e.key.selector, e.key.domain),
            method: "EMAIL".into(),
            status: 0,
            content_type: Some("text/plain".into()),
        },
        extraction: ExtractionInfo {
            manifest_format: email::EMAIL_MANIFEST_FORMAT.into(),
            manifest_name: m.name.clone(),
            manifest_hash: hex32(&m.manifest_hash()),
            selector_digest: hex32(&m.selector_digest()),
            record_pointer: "text/plain".into(),
        },
        observation: Observation {
            evidence_type: m.evidence_type,
            price: Price {
                text: val("price"),
                atoms,
                provenance: Provenance::Authenticated,
                pointer: m
                    .fields
                    .iter()
                    .find(|(k, _)| k == "price")
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default(),
            },
            fields,
            context,
        },
        times: Times {
            witnessed_at: Time {
                value: sig.timestamp.map(|t| t.to_string()).unwrap_or_default(),
                provenance: TimeProvenance::SignerStatedSignatureTime,
            },
            source_observed_at: (!val("observedAt").is_empty()).then(|| Time {
                value: val("observedAt"),
                provenance: TimeProvenance::SourceStatedInRecord,
            }),
            submitted_at: None,
        },
        observation_id: hex32(&observation_id),
        submission_id: None,
        annotations: b.annotations.iter().cloned().collect(),
    };
    Verification {
        format: VERIFICATION_FORMAT,
        stages: Stages {
            received: Stage::passed(),
            provenance: Stage::passed(),
            extraction: Stage::passed(),
            eligibility: nc(),
        },
        envelope: Some(envelope),
        disclosure: Some(disclosure),
        bundle_digest: digest,
    }
}

/// The observation of an email quote, the same for both email methods: the
/// manifest's fields (authenticated) and assumptions (annotations), and an
/// observation id that ignores the signature, the Message-ID and the proof,
/// so a reissued quote, or the same quote proven either way, is one
/// observation.
fn email_fields(
    m: &email::EmailManifest,
    domain: &str,
    values: &[(String, String)],
    atoms: u64,
) -> (std::collections::BTreeMap<String, Field>, [u8; 32]) {
    let val = |k: &str| {
        values
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let mut fields: std::collections::BTreeMap<String, Field> = values
        .iter()
        .filter(|(k, _)| k != "price")
        .map(|(k, v)| {
            (
                k.clone(),
                Field {
                    value: v.clone(),
                    provenance: Provenance::Authenticated,
                    pointer: None,
                },
            )
        })
        .collect();
    for (k, v) in &m.assume {
        fields.insert(
            k.clone(),
            Field {
                value: v.clone(),
                provenance: Provenance::ContributorAnnotation,
                pointer: None,
            },
        );
    }
    let assumed = |k: &str| fields.get(k).map(|f| f.value.clone()).unwrap_or_default();
    let observation_id = ids::observation_id(&ids::ObservationKey {
        server_name: domain,
        path: "dkim",
        selector_digest: m.selector_digest(),
        record_id: &val("recordId"),
        revision: "",
        value_atoms: atoms,
        currency: &assumed("currency"),
        unit: &assumed("unit"),
        quantity_tier: &assumed("quantityTier"),
        source_observed_at: &val("observedAt"),
    });
    (fields, observation_id)
}

/// The zero-knowledge email method: the proof under the policy's pinned
/// RSA key, then the manifest's fields from the proven claims.
pub fn verify_email_zk(b: &bundle::EmailZkBundle, policy: &Policy) -> Verification {
    use email::{EmailError, KeyResolver};
    let nc = Stage::not_checked;
    let digest = Some(hex32(&b.digest()));
    let m = &b.manifest;
    let c = &b.claims;
    let fail =
        |prov: Stage, ext: Stage| Verification::failed(Stage::passed(), prov, ext, digest.clone());
    let circuit = email_zk::circuit_id();
    if b.circuit != circuit {
        return fail(
            Stage::with(
                Outcome::Unsupported,
                format!(
                    "this verifier checks circuit {circuit}, the proof says {}",
                    b.circuit
                ),
            ),
            nc(),
        );
    }
    if !c.domain.eq_ignore_ascii_case(&m.signing_domain) {
        return fail(
            Stage::with(
                Outcome::Failed,
                format!(
                    "the proof is for a signature by {}, the manifest names {}",
                    c.domain, m.signing_domain
                ),
            ),
            nc(),
        );
    }
    let Some(key) = policy.dkim_keys.key(&c.domain, &c.selector) else {
        return fail(
            Stage::with(
                Outcome::Failed,
                EmailError::NoTrustedSignature(c.domain.clone()),
            ),
            nc(),
        );
    };
    let modulus = match email_zk::rsa_key(&key) {
        Ok(n) => n,
        Err(e) => {
            let outcome = match e {
                EmailError::Unsupported(_) => Outcome::Unsupported,
                _ => Outcome::Failed,
            };
            return fail(Stage::with(outcome, e), nc());
        }
    };
    let public = match email_zk::public_inputs(&modulus, m, c) {
        Ok(p) => p,
        Err(e) => {
            return fail(
                Stage::with(
                    Outcome::Failed,
                    format!("the claims do not fit the circuit: {e}"),
                ),
                nc(),
            );
        }
    };
    if let Err(e) = email_zk::verify_proof(&b.proof, &public) {
        return fail(
            Stage::with(
                Outcome::Failed,
                format!(
                    "the zero-knowledge proof does not verify for these claims and this key ({e})"
                ),
            ),
            nc(),
        );
    }
    let disclosure = Disclosure {
        server_name: c.domain.clone(),
        request: String::new(),
        hidden_request_headers: Vec::new(),
        response_headers: [
            ("DKIM d=", Some(c.domain.clone())),
            ("DKIM s=", Some(c.selector.clone())),
            (
                "DKIM t=",
                (c.signed_at != 0).then(|| c.signed_at.to_string()),
            ),
            ("signed Content-Type", Some(c.content_type.clone())),
            (
                "signed Content-Transfer-Encoding",
                c.transfer_encoding.clone(),
            ),
            (
                "From is in the signing domain",
                Some(c.from_aligned.to_string()),
            ),
        ]
        .into_iter()
        .filter(|(_, v)| v.is_some())
        .map(|(n, v)| HeaderView {
            name: n.into(),
            value: v,
        })
        .collect(),
        body: m
            .fields
            .iter()
            .filter_map(|(k, label)| {
                c.values
                    .iter()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| format!("{label} {v}\n"))
            })
            .collect(),
    };
    if let Err(e) = email_zk::check_mime(&c.content_type, c.transfer_encoding.as_deref()) {
        let mut v = fail(Stage::passed(), Stage::with(Outcome::Unsupported, e));
        v.disclosure = Some(disclosure);
        return v;
    }
    let values: Vec<(String, String)> = m
        .fields
        .iter()
        .filter_map(|(k, _)| c.values.iter().find(|(n, _)| n == k).cloned())
        .collect();
    let price = values
        .iter()
        .find(|(k, _)| k == "price")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let atoms = match decimal::parse_atoms(&price) {
        Ok(a) => a,
        Err(e) => {
            let mut v = fail(
                Stage::passed(),
                Stage::with(Outcome::Failed, EmailError::Body(format!("price: {e}"))),
            );
            v.disclosure = Some(disclosure);
            return v;
        }
    };
    let (fields, observation_id) = email_fields(m, &key.domain, &values, atoms);
    let mut context = std::collections::BTreeMap::new();
    context.insert(
        "From is in the signing domain".to_string(),
        c.from_aligned.to_string(),
    );
    let val = |k: &str| {
        values
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let envelope = Envelope {
        format: ENVELOPE_FORMAT,
        authentication: Authentication {
            method: Method::EmailDkimZk,
            backend: bundle::ZK_BACKEND.into(),
            backend_revision: circuit,
            mode: bundle::ZK_MODE.into(),
            notary: format!(
                "{}._domainkey.{} ({})",
                key.selector, key.domain, key.provenance
            ),
            notary_alg: dkim::Algorithm::RsaSha256.as_str().into(),
            notary_key: hex32(&hash::sha256(key.record.as_bytes())),
            trust: key.trust,
            server_trust: ServerTrust::DkimKeyPolicy,
            verifier_policy: policy.id.clone(),
            verifier_policy_digest: hex32(&policy.digest()),
            proof_digest: hex32(&hash::sha256(&b.proof)),
            bundle_digest: hex32(&b.digest()),
            bound_to: None,
        },
        origin: Origin {
            server_name: key.domain.clone(),
            locator: format!("dkim:{}._domainkey.{}", key.selector, key.domain),
            method: "EMAIL".into(),
            status: 0,
            content_type: Some("text/plain".into()),
        },
        extraction: ExtractionInfo {
            manifest_format: email::EMAIL_MANIFEST_FORMAT.into(),
            manifest_name: m.name.clone(),
            manifest_hash: hex32(&m.manifest_hash()),
            selector_digest: hex32(&m.selector_digest()),
            record_pointer: "text/plain".into(),
        },
        observation: Observation {
            evidence_type: m.evidence_type,
            price: Price {
                text: price.clone(),
                atoms,
                provenance: Provenance::Authenticated,
                pointer: m
                    .fields
                    .iter()
                    .find(|(k, _)| k == "price")
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default(),
            },
            fields,
            context,
        },
        times: Times {
            witnessed_at: Time {
                value: if c.signed_at == 0 {
                    String::new()
                } else {
                    c.signed_at.to_string()
                },
                provenance: TimeProvenance::SignerStatedSignatureTime,
            },
            source_observed_at: (!val("observedAt").is_empty()).then(|| Time {
                value: val("observedAt"),
                provenance: TimeProvenance::SourceStatedInRecord,
            }),
            submitted_at: None,
        },
        observation_id: hex32(&observation_id),
        submission_id: None,
        annotations: b.annotations.iter().cloned().collect(),
    };
    Verification {
        format: VERIFICATION_FORMAT,
        stages: Stages {
            received: Stage::passed(),
            provenance: Stage::passed(),
            extraction: Stage::passed(),
            eligibility: nc(),
        },
        envelope: Some(envelope),
        disclosure: Some(disclosure),
        bundle_digest: digest,
    }
}

fn outcome_of(e: &http::HttpError) -> Outcome {
    match e {
        http::HttpError::Unsupported(_) => Outcome::Unsupported,
        _ => Outcome::Failed,
    }
}

/// Where a bundle is submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A source candidate: the proof shows the proposed source's record.
    Candidate {
        market: String,
        expiry: String,
        bucket: String,
        /// The candidate's canonical URL: the proof must be for it.
        canonical_url: String,
        /// The submitting wallet, if the proof must be bound to it.
        submitter: Option<[u8; 32]>,
    },
    /// A Game update (or an opening) for a registered source.
    Update {
        oracle_program: String,
        month: String,
        source_id: [u8; 32],
        /// The source's registered locator: the proof must be for it.
        locator: String,
        /// The selector frozen with the source, when it declared one.
        selector_digest: Option<[u8; 32]>,
        claimant: Option<[u8; 32]>,
    },
}

/// Credential-looking query keys the oracle program and the API refuse in
/// any published locator (`oracle_evidence::public_locator`,
/// `api/api/src/candidates/rules.rs`).
pub const SECRET_FRAGMENTS: &[&str] = &[
    "access_token=",
    "api_key=",
    "apikey=",
    "authorization=",
    "password=",
    "secret=",
    "x-amz-signature=",
    "token=",
];

/// Why `url` cannot be published as an oracle locator, if it cannot.
pub fn locator_problem(url: &str) -> Option<String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Some("the locator is not https".into());
    };
    if url.len() > 384 {
        return Some("the locator is longer than 384 bytes".into());
    }
    if url.contains('#')
        || url.contains('@')
        || url.contains('\\')
        || url.bytes().any(|c| c <= b' ' || c == 0x7f)
    {
        return Some("the locator has a fragment, credentials or spaces".into());
    }
    let host = rest.split(['/', '?']).next().unwrap_or("");
    if host.contains(':') || host != host.to_ascii_lowercase() || host.is_empty() {
        return Some("the locator's host must be a lowercase name without a port".into());
    }
    let lower = url.to_ascii_lowercase();
    if let Some(f) = SECRET_FRAGMENTS.iter().find(|f| lower.contains(*f)) {
        return Some(format!(
            "the locator carries a credential-looking parameter (`{f}`)"
        ));
    }
    None
}

/// The eligibility stage for one destination; on success, sets the
/// envelope's submission id for that destination's accounting scope.
pub fn check_destination(v: &mut Verification, dest: &Destination) {
    let Some(env) = v.envelope.as_mut() else {
        v.stages.eligibility = Stage::not_checked();
        return;
    };
    let fail = |r: String| Stage::with(Outcome::Failed, r);
    let (want_locator, scope, binder) = match dest {
        Destination::Candidate {
            market,
            expiry,
            bucket,
            canonical_url,
            submitter,
        } => (
            canonical_url,
            ids::candidate_scope(market, expiry, bucket),
            *submitter,
        ),
        Destination::Update {
            oracle_program,
            month,
            source_id,
            locator,
            selector_digest,
            claimant,
        } => {
            if let Some(sel) = selector_digest
                && hex32(sel) != env.extraction.selector_digest
            {
                v.stages.eligibility =
                    fail("the manifest reads a different field or record than the one frozen with this source".into());
                return;
            }
            (
                locator,
                ids::update_scope(oracle_program, month, source_id),
                *claimant,
            )
        }
    };
    // The evidence rules admit a source only as a repeatable series others
    // can check over time (docs: evidence rules, "quote-on-request prices"
    // and private invoices are excluded). Authentication does not change
    // that: a proven quote is still one buyer's quote.
    if matches!(
        env.observation.evidence_type,
        EvidenceType::Quotation | EvidenceType::TransactionRecord
    ) {
        v.stages.eligibility = fail(format!(
            "a {} is not a public price series, so the evidence rules do not admit it as a source",
            env.observation.evidence_type.as_str().replace('_', " ")
        ));
        return;
    }
    if let Some(p) = locator_problem(&env.origin.locator) {
        v.stages.eligibility = fail(p);
        return;
    }
    if env.origin.locator != *want_locator {
        v.stages.eligibility = fail(format!(
            "the proof is for {}, the destination's source is {}",
            env.origin.locator, want_locator
        ));
        return;
    }
    if let (Some(bound), Some(who)) = (&env.authentication.bound_to, binder)
        && *bound != hex32(&who)
    {
        v.stages.eligibility = fail("the proof is bound to another wallet".into());
        return;
    }
    let observation = hash::parse_hex32(&env.observation_id).unwrap_or_default();
    env.submission_id = Some(hex32(&ids::submission_id(&scope, &observation)));
    v.stages.eligibility = Stage::passed();
}

/// The on-chain authenticated-evidence record (oracle evidence object kind
/// 4): `amoeba-evidence-v1;m=web;t=<unix>;d=<bundle digest>;u=<locator>`.
/// The program checks its form and that `u` is the source's registered
/// locator and `t` the claim's source time; the proof itself is checked off
/// chain by anyone who fetches the bundle by its digest, with the challenge
/// window as the enforcement path (as for Wayback captures today).
pub fn onchain_record(envelope: &Envelope) -> String {
    format!(
        "{ONCHAIN_RECORD_PREFIX}m=web;t={};d={};u={}",
        envelope.times.witnessed_at.value,
        envelope.authentication.bundle_digest,
        envelope.origin.locator
    )
}

pub const ONCHAIN_RECORD_PREFIX: &str = "amoeba-evidence-v1;";

/// Parses a kind-4 record: (method, unix time, bundle digest, locator).
pub fn parse_onchain_record(text: &str) -> Option<(&str, u64, [u8; 32], &str)> {
    let rest = text.strip_prefix(ONCHAIN_RECORD_PREFIX)?;
    let rest = rest.strip_prefix("m=")?;
    let (method, rest) = rest.split_once(";t=")?;
    let (time, rest) = rest.split_once(";d=")?;
    let (digest, locator) = rest.split_once(";u=")?;
    if !matches!(method, "web" | "email")
        || time.is_empty()
        || time.len() > 20
        || !time.bytes().all(|c| c.is_ascii_digit())
        || time.starts_with('0')
    {
        // No leading zeros, and never time 0 (the oracle refuses it too).
        return None;
    }
    Some((
        method,
        time.parse().ok()?,
        hash::parse_hex32(digest)?,
        locator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_rules_match_the_oracle() {
        assert_eq!(
            locator_problem("https://portal.example.com/api/prices?sku=A"),
            None
        );
        for bad in [
            "http://a.com/x",
            "https://a.com:8443/x",
            "https://A.com/x",
            "https://a.com/x#y",
            "https://u@a.com/x",
            "https://a.com/x?token=1",
            "https://a.com/x?Api_Key=1",
        ] {
            assert!(locator_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn onchain_record_round_trip() {
        let d = [0xab; 32];
        let text = format!(
            "amoeba-evidence-v1;m=web;t=1760900000;d={};u=https://a.com/x?y=1",
            hex32(&d)
        );
        assert_eq!(
            parse_onchain_record(&text),
            Some(("web", 1_760_900_000, d, "https://a.com/x?y=1"))
        );
        assert!(parse_onchain_record(&text.replace("t=1", "t=01")).is_none());
        assert!(parse_onchain_record(&text.replace("t=1760900000", "t=0")).is_none());
        assert!(parse_onchain_record(&text.replace("m=web", "m=wayback")).is_none());
    }
}
