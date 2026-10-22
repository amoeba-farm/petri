//! The portable evidence bundle: what a contributor exports, imports on
//! another machine, submits, and what anyone re-verifies later.
//!
//! ```json
//! {
//!   "format": "amoeba-evidence-bundle-v1",
//!   "method": "web_proof",
//!   "backend": { "name": "tlsnotary", "revision": "5f67389…", "mode": "mpc" },
//!   "presentation": "<base64 of the bincode TLSNotary presentation>",
//!   "manifest": { …extraction manifest… },
//!   "annotations": { "note": "…" }
//! }
//! ```
//!
//! A bundle holds no credentials: the presentation discloses only what the
//! prover chose (cookies and authorization values stay hidden), and nothing
//! else in it comes from the session. Verifiers never read values from a
//! bundle's own statements; they recompute everything from the
//! presentation and the manifest. `annotations` are the contributor's
//! unverified notes, outside the digest.

use base64::Engine as _;

use crate::hash::{sha256, tagged};
use crate::json::{self, Json, Node};
use crate::manifest::Manifest;

pub const BUNDLE_FORMAT: &str = "amoeba-evidence-bundle-v1";
pub const BUNDLE_DIGEST_DOMAIN: &str = "amoeba-evidence-bundle-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// A TLSNotary presentation of an HTTPS exchange.
    WebProof,
    /// A DKIM-signed message, disclosed whole (`crate::email`).
    EmailDkim,
    /// A zero-knowledge proof about a DKIM-signed message that discloses
    /// only its labelled lines (`crate::email_zk`).
    EmailDkimZk,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebProof => "web_proof",
            Self::EmailDkim => "email_dkim",
            Self::EmailDkimZk => "email_dkim_zk",
        }
    }
}

/// A bundle's `method`, read before the rest (`None` when unreadable).
pub fn peek_method(bytes: &[u8]) -> Option<String> {
    let root = json::parse(bytes).ok()?;
    match &root.pointer("/method")?.value {
        Json::String(m) => Some(m.clone()),
        _ => None,
    }
}

/// An `email_dkim` bundle: the message bytes and the email manifest.
///
/// ```json
/// { "format": "amoeba-evidence-bundle-v1", "method": "email_dkim",
///   "backend": { "name": "dkim", "revision": "rfc6376", "mode": "whole-message" },
///   "message": "<base64 of the .eml>", "manifest": { …email manifest… } }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EmailBundle {
    pub message: Vec<u8>,
    pub manifest: crate::email::EmailManifest,
    pub annotations: Vec<(String, String)>,
}

impl EmailBundle {
    pub fn parse(bytes: &[u8], max_bytes: usize) -> Result<Self, BundleError> {
        if bytes.len() > max_bytes {
            return bad(format!("larger than {max_bytes} bytes"));
        }
        let root = json::parse(bytes).map_err(|e| BundleError(e.to_string()))?;
        let Json::Object(members) = &root.value else {
            return bad("not a JSON object");
        };
        const KEYS: &[&str] = &[
            "format",
            "method",
            "backend",
            "message",
            "manifest",
            "annotations",
        ];
        if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
            return bad(format!("unknown key `{k}`"));
        }
        if str_at(&root, "/format")? != BUNDLE_FORMAT || str_at(&root, "/method")? != "email_dkim" {
            return bad("not an email_dkim bundle");
        }
        if str_at(&root, "/backend/name")? != "dkim"
            || str_at(&root, "/backend/revision")? != "rfc6376"
            || str_at(&root, "/backend/mode")? != "whole-message"
        {
            return bad("backend must be dkim rfc6376 whole-message");
        }
        let message = base64::engine::general_purpose::STANDARD
            .decode(str_at(&root, "/message")?)
            .map_err(|_| BundleError("`message` is not base64".into()))?;
        let m = root
            .pointer("/manifest")
            .ok_or_else(|| BundleError("`manifest` is missing".into()))?;
        let manifest = crate::email::EmailManifest::parse(&json::canonical(m))
            .map_err(|e| BundleError(e.to_string()))?;
        let mut annotations = Vec::new();
        if let Some(Node {
            value: Json::Object(pairs),
            ..
        }) = root.pointer("/annotations")
        {
            for (k, v) in pairs {
                let Json::String(s) = &v.value else {
                    return bad("annotations must be strings");
                };
                annotations.push((k.clone(), s.clone()));
            }
        }
        Ok(Self {
            message,
            manifest,
            annotations,
        })
    }

    pub fn to_json(&self) -> String {
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let mut out = String::from("{\n");
        out += &format!("  \"format\": {},\n", q(BUNDLE_FORMAT));
        out += "  \"method\": \"email_dkim\",\n";
        out += "  \"backend\": { \"name\": \"dkim\", \"revision\": \"rfc6376\", \"mode\": \"whole-message\" },\n";
        out += &format!(
            "  \"manifest\": {},\n",
            String::from_utf8_lossy(&self.manifest.canonical)
        );
        if !self.annotations.is_empty() {
            let pairs: Vec<String> = self
                .annotations
                .iter()
                .map(|(k, v)| format!("{}: {}", q(k), q(v)))
                .collect();
            out += &format!("  \"annotations\": {{ {} }},\n", pairs.join(", "));
        }
        out += &format!(
            "  \"message\": {}\n}}\n",
            q(&base64::engine::general_purpose::STANDARD.encode(&self.message))
        );
        out
    }

    pub fn digest(&self) -> [u8; 32] {
        tagged(
            BUNDLE_DIGEST_DOMAIN,
            &[
                b"email_dkim",
                b"dkim",
                b"rfc6376",
                b"whole-message",
                &self.message,
                &self.manifest.canonical,
            ],
        )
    }
}

/// An `email_dkim_zk` bundle: a zero-knowledge proof, the claims it
/// discloses and the email manifest. No message bytes.
///
/// ```json
/// { "format": "amoeba-evidence-bundle-v1", "method": "email_dkim_zk",
///   "backend": { "name": "noir-ultrahonk", "revision": "<circuit id>", "mode": "zk-keccak" },
///   "manifest": { …email manifest… },
///   "claims": { "domain": "…", "selector": "…", "signedAt": 1760868000,
///               "contentType": "text/plain; charset=utf-8", "transferEncoding": "7bit",
///               "fromAligned": true, "values": { "price": "39.80", … } },
///   "proof": "<base64 of the UltraHonk proof>" }
/// ```
///
/// `revision` is the circuit's id (SHA-256 of its verification key); a
/// verifier refuses a proof for another circuit. The claims are what the
/// proof is checked against, never accepted on their own.
#[derive(Debug, Clone, PartialEq)]
pub struct EmailZkBundle {
    pub circuit: String,
    pub proof: Vec<u8>,
    pub claims: crate::email_zk::Claims,
    pub manifest: crate::email::EmailManifest,
    pub annotations: Vec<(String, String)>,
}

pub const ZK_BACKEND: &str = "noir-ultrahonk";
pub const ZK_MODE: &str = "zk-keccak";

impl EmailZkBundle {
    pub fn parse(bytes: &[u8], max_bytes: usize) -> Result<Self, BundleError> {
        if bytes.len() > max_bytes {
            return bad(format!("larger than {max_bytes} bytes"));
        }
        let root = json::parse(bytes).map_err(|e| BundleError(e.to_string()))?;
        let Json::Object(members) = &root.value else {
            return bad("not a JSON object");
        };
        const KEYS: &[&str] = &[
            "format",
            "method",
            "backend",
            "manifest",
            "claims",
            "proof",
            "annotations",
        ];
        if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
            return bad(format!("unknown key `{k}`"));
        }
        if str_at(&root, "/format")? != BUNDLE_FORMAT
            || str_at(&root, "/method")? != "email_dkim_zk"
        {
            return bad("not an email_dkim_zk bundle");
        }
        if str_at(&root, "/backend/name")? != ZK_BACKEND
            || str_at(&root, "/backend/mode")? != ZK_MODE
        {
            return bad(format!("backend must be {ZK_BACKEND} {ZK_MODE}"));
        }
        let circuit = str_at(&root, "/backend/revision")?.to_string();
        let proof = base64::engine::general_purpose::STANDARD
            .decode(str_at(&root, "/proof")?)
            .map_err(|_| BundleError("`proof` is not base64".into()))?;
        let m = root
            .pointer("/manifest")
            .ok_or_else(|| BundleError("`manifest` is missing".into()))?;
        let manifest = crate::email::EmailManifest::parse(&json::canonical(m))
            .map_err(|e| BundleError(e.to_string()))?;
        let claims = parse_claims(
            root.pointer("/claims")
                .ok_or_else(|| BundleError("`claims` is missing".into()))?,
        )?;
        let mut annotations = Vec::new();
        if let Some(Node {
            value: Json::Object(pairs),
            ..
        }) = root.pointer("/annotations")
        {
            for (k, v) in pairs {
                let Json::String(s) = &v.value else {
                    return bad("annotations must be strings");
                };
                annotations.push((k.clone(), s.clone()));
            }
        }
        Ok(Self {
            circuit,
            proof,
            claims,
            manifest,
            annotations,
        })
    }

    /// The claims as canonical JSON (part of the digest).
    pub fn claims_json(&self) -> String {
        let c = &self.claims;
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let values: Vec<String> = c
            .values
            .iter()
            .map(|(k, v)| format!("{}:{}", q(k), q(v)))
            .collect();
        let text = format!(
            "{{\"domain\":{},\"selector\":{},\"signedAt\":{},\"contentType\":{},{}\"fromAligned\":{},\"values\":{{{}}}}}",
            q(&c.domain),
            q(&c.selector),
            c.signed_at,
            q(&c.content_type),
            c.transfer_encoding
                .as_deref()
                .map(|t| format!("\"transferEncoding\":{},", q(t)))
                .unwrap_or_default(),
            c.from_aligned,
            values.join(",")
        );
        json::parse(text.as_bytes())
            .map(|n| String::from_utf8_lossy(&json::canonical(&n)).into_owned())
            .unwrap_or(text)
    }

    pub fn to_json(&self) -> String {
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let mut out = String::from("{\n");
        out += &format!("  \"format\": {},\n", q(BUNDLE_FORMAT));
        out += "  \"method\": \"email_dkim_zk\",\n";
        out += &format!(
            "  \"backend\": {{ \"name\": {}, \"revision\": {}, \"mode\": {} }},\n",
            q(ZK_BACKEND),
            q(&self.circuit),
            q(ZK_MODE)
        );
        out += &format!(
            "  \"manifest\": {},\n",
            String::from_utf8_lossy(&self.manifest.canonical)
        );
        out += &format!("  \"claims\": {},\n", self.claims_json());
        if !self.annotations.is_empty() {
            let pairs: Vec<String> = self
                .annotations
                .iter()
                .map(|(k, v)| format!("{}: {}", q(k), q(v)))
                .collect();
            out += &format!("  \"annotations\": {{ {} }},\n", pairs.join(", "));
        }
        out += &format!(
            "  \"proof\": {}\n}}\n",
            q(&base64::engine::general_purpose::STANDARD.encode(&self.proof))
        );
        out
    }

    pub fn digest(&self) -> [u8; 32] {
        tagged(
            BUNDLE_DIGEST_DOMAIN,
            &[
                b"email_dkim_zk",
                ZK_BACKEND.as_bytes(),
                self.circuit.as_bytes(),
                ZK_MODE.as_bytes(),
                &self.proof,
                self.claims_json().as_bytes(),
                &self.manifest.canonical,
            ],
        )
    }
}

fn parse_claims(node: &Node) -> Result<crate::email_zk::Claims, BundleError> {
    let Json::Object(members) = &node.value else {
        return bad("`claims` must be an object");
    };
    const KEYS: &[&str] = &[
        "domain",
        "selector",
        "signedAt",
        "contentType",
        "transferEncoding",
        "fromAligned",
        "values",
    ];
    if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
        return bad(format!("unknown claim `{k}`"));
    }
    let signed_at = match node.pointer("/signedAt").map(|n| &n.value) {
        Some(Json::Number(t))
            if !t.is_empty()
                && t.bytes().all(|c| c.is_ascii_digit())
                && (t == "0" || !t.starts_with('0')) =>
        {
            t.parse::<u64>()
                .map_err(|_| BundleError("`claims.signedAt` is out of range".into()))?
        }
        _ => return bad("`claims.signedAt` must be a whole number"),
    };
    let from_aligned = match node.pointer("/fromAligned").map(|n| &n.value) {
        Some(Json::Bool(b)) => *b,
        _ => return bad("`claims.fromAligned` must be true or false"),
    };
    let transfer_encoding = match node.pointer("/transferEncoding") {
        None => None,
        Some(_) => Some(str_at(node, "/transferEncoding")?.to_string()),
    };
    let Some(Node {
        value: Json::Object(pairs),
        ..
    }) = node.pointer("/values")
    else {
        return bad("`claims.values` must be an object");
    };
    let mut values = Vec::new();
    for (k, v) in pairs {
        let Json::String(s) = &v.value else {
            return bad("claim values must be strings");
        };
        values.push((k.clone(), s.clone()));
    }
    Ok(crate::email_zk::Claims {
        domain: str_at(node, "/domain")?.to_string(),
        selector: str_at(node, "/selector")?.to_string(),
        signed_at,
        content_type: str_at(node, "/contentType")?.to_string(),
        transfer_encoding,
        from_aligned,
        values,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub method: Method,
    pub backend_name: String,
    pub backend_revision: String,
    pub backend_mode: String,
    pub presentation: Vec<u8>,
    pub manifest: Manifest,
    /// Unverified contributor notes: key → text.
    pub annotations: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid evidence bundle: {0}")]
pub struct BundleError(pub String);

fn bad<T>(what: impl Into<String>) -> Result<T, BundleError> {
    Err(BundleError(what.into()))
}

fn str_at<'a>(node: &'a Node, ptr: &str) -> Result<&'a str, BundleError> {
    match node.pointer(ptr).map(|n| &n.value) {
        Some(Json::String(s)) => Ok(s),
        _ => bad(format!(
            "`{}` must be a string",
            &ptr[1..].replace('/', ".")
        )),
    }
}

impl Bundle {
    pub fn parse(bytes: &[u8], max_bytes: usize) -> Result<Self, BundleError> {
        if bytes.len() > max_bytes {
            return bad(format!("larger than {max_bytes} bytes"));
        }
        let root = json::parse(bytes).map_err(|e| BundleError(e.to_string()))?;
        let Json::Object(members) = &root.value else {
            return bad("not a JSON object");
        };
        const KEYS: &[&str] = &[
            "format",
            "method",
            "backend",
            "presentation",
            "manifest",
            "annotations",
        ];
        if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
            return bad(format!("unknown key `{k}`"));
        }
        if str_at(&root, "/format")? != BUNDLE_FORMAT {
            return bad(format!("`format` must be `{BUNDLE_FORMAT}`"));
        }
        let method = match str_at(&root, "/method")? {
            "web_proof" => Method::WebProof,
            other => return bad(format!("method `{other}` is not supported")),
        };
        let backend_name = str_at(&root, "/backend/name")?.to_string();
        let backend_revision = str_at(&root, "/backend/revision")?.to_string();
        let backend_mode = str_at(&root, "/backend/mode")?.to_string();
        if backend_name != "tlsnotary" {
            return bad(format!("backend `{backend_name}` is not supported"));
        }
        if backend_mode != "mpc" {
            return bad(format!(
                "TLSNotary mode `{backend_mode}` is not supported (only mpc)"
            ));
        }
        let presentation = base64::engine::general_purpose::STANDARD
            .decode(str_at(&root, "/presentation")?)
            .map_err(|_| BundleError("`presentation` is not base64".into()))?;
        let manifest_node = root
            .pointer("/manifest")
            .ok_or_else(|| BundleError("`manifest` is missing".into()))?;
        let manifest = Manifest::parse(&json::canonical(manifest_node))
            .map_err(|e| BundleError(e.to_string()))?;
        let mut annotations = Vec::new();
        if let Some(a) = root.pointer("/annotations") {
            let Json::Object(pairs) = &a.value else {
                return bad("`annotations` must be an object");
            };
            for (k, v) in pairs {
                let Json::String(s) = &v.value else {
                    return bad("annotations must be strings");
                };
                if k.len() > 64 || s.len() > 2000 {
                    return bad(
                        "annotations are limited to 64-character keys and 2000-character values",
                    );
                }
                annotations.push((k.clone(), s.clone()));
            }
        }
        Ok(Self {
            method,
            backend_name,
            backend_revision,
            backend_mode,
            presentation,
            manifest,
            annotations,
        })
    }

    /// The bundle as JSON. The manifest is written as its canonical bytes,
    /// so a round trip keeps the digest (a JSON library would re-print its
    /// numbers).
    pub fn to_json(&self) -> String {
        let q = |s: &str| serde_json::to_string(s).unwrap_or_default();
        let mut out = String::from("{\n");
        out += &format!("  \"format\": {},\n", q(BUNDLE_FORMAT));
        out += &format!("  \"method\": {},\n", q(self.method.as_str()));
        out += &format!(
            "  \"backend\": {{ \"name\": {}, \"revision\": {}, \"mode\": {} }},\n",
            q(&self.backend_name),
            q(&self.backend_revision),
            q(&self.backend_mode)
        );
        out += &format!(
            "  \"manifest\": {},\n",
            String::from_utf8_lossy(&self.manifest.canonical)
        );
        if !self.annotations.is_empty() {
            let pairs: Vec<String> = self
                .annotations
                .iter()
                .map(|(k, v)| format!("{}: {}", q(k), q(v)))
                .collect();
            out += &format!("  \"annotations\": {{ {} }},\n", pairs.join(", "));
        }
        out += &format!(
            "  \"presentation\": {}\n}}\n",
            q(&base64::engine::general_purpose::STANDARD.encode(&self.presentation))
        );
        out
    }

    /// SHA-256 of the presentation bytes (the proof artifact's address).
    pub fn proof_digest(&self) -> [u8; 32] {
        sha256(&self.presentation)
    }

    /// The bundle's identity: method, backend, presentation and manifest.
    /// Annotations are not part of it.
    pub fn digest(&self) -> [u8; 32] {
        tagged(
            BUNDLE_DIGEST_DOMAIN,
            &[
                self.method.as_str().as_bytes(),
                self.backend_name.as_bytes(),
                self.backend_revision.as_bytes(),
                self.backend_mode.as_bytes(),
                &self.presentation,
                &self.manifest.canonical,
            ],
        )
    }
}
