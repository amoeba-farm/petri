//! Email evidence (`email_dkim`): a quote in a DKIM-signed message, read
//! from the signed body, in the same pipeline as web proofs.
//!
//! What a passing check establishes: the domain `d=` signed this body and
//! these headers with the key the verifier's own key policy names for
//! `<selector>._domainkey.<d>`. What it does not: that the displayed sender
//! is that domain's mailbox (`fromAligned` says whether the From address is
//! in `d=`), who received it, when it was delivered (a signed `Date:` is the
//! sender's claim), or that the quote became a sale. A quote stays a
//! quotation.
//!
//! This method discloses the whole message to whoever verifies the bundle:
//! DKIM covers the whole body, so proving one line of it without the rest
//! needs a zero-knowledge circuit (`email_dkim_zk`, `crate::email_zk`).
//! Petri keeps a message local unless the contributor chooses to disclose
//! it.
//!
//! Extraction reads labelled lines (`Unit price: 41.25`) of the one
//! `text/plain` part, never quoted (`>`) lines, and refuses forwarded
//! messages, several text parts, partial-body signatures and keys the
//! policy does not name.

use base64::Engine as _;

use crate::decimal;
use crate::dkim::{self, DkimError, KeyRecord, Message, Verified};
use crate::hash::tagged;
use crate::json::{self, Json, Node};
use crate::manifest::{ASSUMABLE, EvidenceType};
use crate::policy::Trust;

pub const EMAIL_MANIFEST_FORMAT: &str = "amoeba-email-manifest-v1";
/// Bound on hostile input (prompt v2, section 9).
pub const MAX_MESSAGE_BYTES: usize = 512 * 1024;

/// Line labels the manifest may name, as for web records.
pub const EMAIL_FIELDS: &[&str] = &[
    "price",
    "currency",
    "unit",
    "product",
    "productName",
    "priceType",
    "quantityTier",
    "terms",
    "recordId",
    "observedAt",
];

#[derive(Debug, Clone, PartialEq)]
pub struct EmailManifest {
    pub name: String,
    pub evidence_type: EvidenceType,
    /// The DKIM `d=` the quote must be signed by.
    pub signing_domain: String,
    /// Field → the line label it follows (`"Unit price:"`).
    pub fields: Vec<(String, String)>,
    pub assume: Vec<(String, String)>,
    pub canonical: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid email manifest: {0}")]
pub struct EmailManifestError(pub String);

impl EmailManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, EmailManifestError> {
        let bad = |s: String| EmailManifestError(s);
        if bytes.len() > crate::manifest::MAX_MANIFEST_BYTES {
            return Err(bad("larger than 16 KiB".into()));
        }
        let root = json::parse(bytes).map_err(|e| bad(e.to_string()))?;
        let Json::Object(members) = &root.value else {
            return Err(bad("not a JSON object".into()));
        };
        const KEYS: &[&str] = &[
            "format",
            "name",
            "evidenceType",
            "signingDomain",
            "fields",
            "assume",
        ];
        if let Some((k, _)) = members.iter().find(|(k, _)| !KEYS.contains(&k.as_str())) {
            return Err(bad(format!("unknown key `{k}`")));
        }
        let s = |k: &str| match root.pointer(&format!("/{k}")).map(|n| &n.value) {
            Some(Json::String(v)) => Ok(v.clone()),
            _ => Err(bad(format!("`{k}` must be a string"))),
        };
        if s("format")? != EMAIL_MANIFEST_FORMAT {
            return Err(bad(format!("`format` must be `{EMAIL_MANIFEST_FORMAT}`")));
        }
        let evidence_type = s("evidenceType")?;
        let evidence_type = EvidenceType::parse(&evidence_type).ok_or_else(|| {
            bad(format!(
                "`evidenceType` {evidence_type} is not one of the four"
            ))
        })?;
        let signing_domain = s("signingDomain")?.to_ascii_lowercase();
        if !signing_domain.contains('.') || signing_domain.contains(['/', '@', ' ']) {
            return Err(bad("`signingDomain` must be a domain".into()));
        }
        let mut fields = Vec::new();
        let Some(Node {
            value: Json::Object(pairs),
            ..
        }) = root.pointer("/fields")
        else {
            return Err(bad("`fields` must be an object".into()));
        };
        for (k, v) in pairs {
            if !EMAIL_FIELDS.contains(&k.as_str()) {
                return Err(bad(format!("unknown field `{k}`")));
            }
            let Json::String(label) = &v.value else {
                return Err(bad(format!("field `{k}` must be a line label")));
            };
            if label.trim().is_empty() || label.len() > 100 || label.contains(['\r', '\n']) {
                return Err(bad(format!("field `{k}` must be a one-line label")));
            }
            fields.push((k.clone(), label.trim().to_string()));
        }
        if !fields.iter().any(|(k, _)| k == "price") {
            return Err(bad("`fields.price` is required".into()));
        }
        let mut assume = Vec::new();
        if let Some(Node {
            value: Json::Object(pairs),
            ..
        }) = root.pointer("/assume")
        {
            for (k, v) in pairs {
                if !ASSUMABLE.contains(&k.as_str()) || fields.iter().any(|(f, _)| f == k) {
                    return Err(bad(format!("`assume.{k}` is not allowed")));
                }
                let Json::String(text) = &v.value else {
                    return Err(bad(format!("`assume.{k}` must be a string")));
                };
                assume.push((k.clone(), text.clone()));
            }
        }
        Ok(Self {
            name: s("name")?,
            evidence_type,
            signing_domain,
            fields,
            assume,
            canonical: json::canonical(&root),
        })
    }

    pub fn manifest_hash(&self) -> [u8; 32] {
        tagged(EMAIL_MANIFEST_FORMAT, &[&self.canonical])
    }

    /// The series an email quote belongs to: its signer and the price label.
    pub fn selector_digest(&self) -> [u8; 32] {
        let f = |k: &str| {
            self.fields
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, l)| l.as_bytes().to_vec())
                .unwrap_or_default()
        };
        tagged(
            "amoeba-email-selector-v1",
            &[
                self.signing_domain.as_bytes(),
                &f("price"),
                &f("currency"),
                &f("product"),
            ],
        )
    }
}

/// A DKIM key the verifier accepts for `selector._domainkey.domain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedKey {
    pub domain: String,
    pub selector: String,
    /// The TXT record value (`v=DKIM1; k=…; p=…`).
    pub record: String,
    pub trust: Trust,
    /// Where the binding of key to domain comes from, recorded with every
    /// result (`pinned in the verifier policy`, `DNS TXT as resolved by the
    /// verifier at …`).
    pub provenance: String,
}

/// How a verifier finds the key for a signature. Never the bundle's word:
/// a pinned key in the policy, or a lookup the verifier does itself.
pub trait KeyResolver {
    fn key(&self, domain: &str, selector: &str) -> Option<PinnedKey>;
}

impl KeyResolver for Vec<PinnedKey> {
    fn key(&self, domain: &str, selector: &str) -> Option<PinnedKey> {
        self.iter()
            .find(|k| {
                k.domain.eq_ignore_ascii_case(domain) && k.selector.eq_ignore_ascii_case(selector)
            })
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    #[error("the message is larger than {0} bytes")]
    TooLarge(usize),
    #[error(transparent)]
    Dkim(#[from] DkimError),
    #[error("no signature by {0} verifies with a key the verifier accepts")]
    NoTrustedSignature(String),
    #[error("{0}")]
    Body(String),
    #[error("{0} is not supported")]
    Unsupported(String),
}

/// One verified message.
#[derive(Debug, Clone)]
pub struct VerifiedEmail {
    pub verified: Verified,
    pub key: PinnedKey,
    /// The signed From header value and whether its address is in `d=`.
    pub from: Option<String>,
    pub from_aligned: bool,
    pub subject: Option<String>,
    /// A signed `Date:` (the sender's own claim).
    pub date: Option<String>,
    pub message_id: Option<String>,
    /// The decoded text/plain body, quoted lines left out.
    pub text: String,
}

fn address_domain(from: &str) -> Option<String> {
    let addr = from
        .rsplit_once('<')
        .map(|(_, r)| r.trim_end_matches('>'))
        .unwrap_or(from);
    addr.rsplit_once('@')
        .map(|(_, d)| d.trim().trim_end_matches('>').to_ascii_lowercase())
}

fn qp_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b.get(i + 1..i + 3) == Some(b"\r\n") {
                i += 3;
                continue;
            }
            if b.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let Some(h) = b.get(i + 1..i + 3)
                && let Ok(v) = u8::from_str_radix(std::str::from_utf8(h).unwrap_or("zz"), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn header_param(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// A MIME entity: its headers and its body (raw).
fn entity(bytes: &[u8]) -> Result<(Vec<dkim::Header>, Vec<u8>), EmailError> {
    // A part may have no headers at all (then it starts with a blank line).
    if bytes.starts_with(b"\r\n") {
        return Ok((Vec::new(), bytes[2..].to_vec()));
    }
    let m = dkim::parse_message(bytes)
        .map_err(|_| EmailError::Body("a MIME part is malformed".into()))?;
    Ok((m.headers, m.body))
}

fn get(headers: &[dkim::Header], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value())
}

/// The one text/plain part of a body, decoded.
fn text_plain(
    headers: &[dkim::Header],
    body: &[u8],
    depth: usize,
) -> Result<Vec<String>, EmailError> {
    if depth > 4 {
        return Err(EmailError::Unsupported(
            "MIME nested more than four levels".into(),
        ));
    }
    let ctype = get(headers, "content-type").unwrap_or_else(|| "text/plain".into());
    let mime = ctype
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "message/rfc822" {
        return Err(EmailError::Unsupported("a forwarded message".into()));
    }
    if mime.starts_with("multipart/") {
        let boundary = header_param(&ctype, "boundary")
            .ok_or_else(|| EmailError::Body("a multipart body without a boundary".into()))?;
        let text = String::from_utf8_lossy(body).into_owned();
        let delim = format!("--{boundary}");
        let mut out = Vec::new();
        for (i, part) in text.split(&delim).enumerate() {
            if i == 0 || part.starts_with("--") {
                continue;
            }
            let part = part.strip_prefix("\r\n").unwrap_or(part);
            let part = part.strip_suffix("\r\n").unwrap_or(part);
            let (h, b) = entity(part.as_bytes())?;
            let disposition = get(&h, "content-disposition")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if disposition.starts_with("attachment") {
                continue;
            }
            out.extend(text_plain(&h, &b, depth + 1)?);
        }
        return Ok(out);
    }
    if mime != "text/plain" {
        return Ok(Vec::new());
    }
    let charset = header_param(&ctype, "charset")
        .unwrap_or_else(|| "us-ascii".into())
        .to_ascii_lowercase();
    let encoding = get(headers, "content-transfer-encoding")
        .unwrap_or_else(|| "7bit".into())
        .to_ascii_lowercase();
    let raw = match encoding.as_str() {
        "7bit" | "8bit" | "binary" => body.to_vec(),
        "quoted-printable" => qp_decode(&String::from_utf8_lossy(body)),
        "base64" => {
            let compact: String = String::from_utf8_lossy(body)
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect();
            base64::engine::general_purpose::STANDARD
                .decode(compact)
                .map_err(|_| EmailError::Body("a base64 part does not decode".into()))?
        }
        other => {
            return Err(EmailError::Unsupported(format!(
                "transfer encoding {other}"
            )));
        }
    };
    let text = match charset.as_str() {
        "us-ascii" | "utf-8" | "utf8" => {
            String::from_utf8(raw).map_err(|_| EmailError::Body("the text is not UTF-8".into()))?
        }
        "iso-8859-1" | "latin1" => raw.iter().map(|b| char::from(*b)).collect(),
        other => return Err(EmailError::Unsupported(format!("charset {other}"))),
    };
    Ok(vec![text])
}

/// Verifies the message's signatures by the manifest's domain with keys
/// from `keys`, and decodes its text.
pub fn verify_message(
    bytes: &[u8],
    signing_domain: &str,
    keys: &dyn KeyResolver,
) -> Result<VerifiedEmail, EmailError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(EmailError::TooLarge(MAX_MESSAGE_BYTES));
    }
    let msg: Message = dkim::parse_message(bytes)?;
    let sigs = dkim::signatures(&msg);
    if sigs.is_empty() {
        return Err(DkimError::Unsigned.into());
    }
    let mut last_error = None;
    let mut found = None;
    for sig in sigs.into_iter().flatten() {
        if !sig.domain.eq_ignore_ascii_case(signing_domain) {
            continue;
        }
        let Some(pinned) = keys.key(&sig.domain, &sig.selector) else {
            continue;
        };
        let record: KeyRecord = match dkim::parse_key_record(&pinned.record) {
            Ok(r) => r,
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        };
        match dkim::verify(&msg, &sig, &record) {
            Ok(v) => {
                found = Some((v, pinned));
                break;
            }
            Err(e) => last_error = Some(e),
        }
    }
    let Some((verified, key)) = found else {
        return Err(match last_error {
            Some(e @ DkimError::PartialBody(_)) => e.into(),
            Some(e @ DkimError::BodyHash) => e.into(),
            Some(e @ DkimError::Signature) => e.into(),
            _ => EmailError::NoTrustedSignature(signing_domain.to_string()),
        });
    };
    let from = verified.header("from").map(str::to_string);
    let from_aligned = from
        .as_deref()
        .and_then(address_domain)
        .is_some_and(|d| d == key.domain || d.ends_with(&format!(".{}", key.domain)));
    // The body MIME structure comes from the signed top-level headers only.
    let top: Vec<dkim::Header> = msg
        .headers
        .iter()
        .filter(|h| {
            let n = h.name.to_ascii_lowercase();
            (n == "content-type" || n == "content-transfer-encoding")
                && verified.header(&n).is_some()
        })
        .cloned()
        .collect();
    let has_unsigned_mime = msg.headers.iter().any(|h| {
        let n = h.name.to_ascii_lowercase();
        (n == "content-type" || n == "content-transfer-encoding") && verified.header(&n).is_none()
    });
    if has_unsigned_mime {
        return Err(EmailError::Body(
            "the message's Content-Type is not signed, so its text cannot be read as signed".into(),
        ));
    }
    let texts = text_plain(&top, &msg.body, 0)?;
    let text = match texts.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(EmailError::Unsupported(
                "a message without a text/plain part".into(),
            ));
        }
        _ => {
            return Err(EmailError::Body(
                "the message has several text parts; which one quotes is ambiguous".into(),
            ));
        }
    };
    if text.contains("---------- Forwarded message")
        || text.contains("-----Original Message-----")
        || text.contains("Begin forwarded message")
    {
        return Err(EmailError::Unsupported(
            "a forwarded or replied-to quote".into(),
        ));
    }
    let unquoted: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(VerifiedEmail {
        subject: verified.header("subject").map(str::to_string),
        date: verified.header("date").map(str::to_string),
        message_id: verified.header("message-id").map(str::to_string),
        from,
        from_aligned,
        verified,
        key,
        text: unquoted,
    })
}

/// Reads the manifest's fields from the verified text: exactly one line
/// per label.
pub fn extract(
    m: &EmailManifest,
    e: &VerifiedEmail,
) -> Result<(u64, Vec<(String, String)>), EmailError> {
    let mut values = Vec::new();
    for (name, label) in &m.fields {
        let hits: Vec<&str> = e
            .text
            .lines()
            .filter_map(|l| l.trim().strip_prefix(label.as_str()))
            .map(str::trim)
            .collect();
        match hits.as_slice() {
            [v] if !v.is_empty() => values.push((name.clone(), v.to_string())),
            [] | [_] => {
                return Err(EmailError::Body(format!(
                    "the signed text has no `{label}` line"
                )));
            }
            many => {
                return Err(EmailError::Body(format!(
                    "the signed text has {} `{label}` lines; which one is ambiguous",
                    many.len()
                )));
            }
        }
    }
    let price = values
        .iter()
        .find(|(k, _)| k == "price")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let atoms =
        decimal::parse_atoms(&price).map_err(|e| EmailError::Body(format!("price: {e}")))?;
    Ok((atoms, values))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Signs `message` (CRLF, no DKIM header yet) with Ed25519 for tests:
    /// relaxed/relaxed, `h=` as given. Returns the signed message and the
    /// key record to pin.
    pub(crate) fn sign(
        message: &str,
        domain: &str,
        selector: &str,
        h: &str,
        l: Option<usize>,
    ) -> (String, String) {
        use ring::signature::KeyPair;
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let record = format!("v=DKIM1; k=ed25519; p={}", b64(key.public_key().as_ref()));
        let msg = dkim::parse_message(message.as_bytes()).unwrap();
        let body = dkim::tests_support::relaxed_body(&msg.body);
        let signed_len = l.unwrap_or(body.len()).min(body.len());
        let bh = b64(&sha2::Sha256::digest_bytes(&body[..signed_len]));
        let l_tag = l.map(|l| format!(" l={l};")).unwrap_or_default();
        let unsigned = format!(
            "DKIM-Signature: v=1; a=ed25519-sha256; c=relaxed/relaxed; d={domain}; s={selector};{l_tag} h={h}; bh={bh}; b=\r\n"
        );
        let with_sig = format!("{unsigned}{message}");
        let m2 = dkim::parse_message(with_sig.as_bytes()).unwrap();
        let input = dkim::tests_support::header_input(&m2, h);
        let digest = sha2::Sha256::digest_bytes(&input);
        let sig = key.sign(&digest);
        let header = unsigned.replace("b=\r\n", &format!("b={}\r\n", b64(sig.as_ref())));
        (format!("{header}{message}"), record)
    }

    trait DigestBytes {
        fn digest_bytes(b: &[u8]) -> Vec<u8>;
    }
    impl DigestBytes for sha2::Sha256 {
        fn digest_bytes(b: &[u8]) -> Vec<u8> {
            use sha2::Digest;
            sha2::Sha256::digest(b).to_vec()
        }
    }

    const QUOTE: &str = "From: Quotes <quotes@distributor.example>\r\n\
To: Buyer <buyer@example.net>\r\n\
Subject: Quote Q-1042: DDR5 16GB\r\n\
Date: Mon, 19 Oct 2026 10:00:00 +0000\r\n\
Message-ID: <q-1042@distributor.example>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
FIXTURE QUOTE FOR TESTS - NOT MARKET DATA\r\n\
Quote number: Q-1042\r\n\
Part number: DDR5-16G-4800\r\n\
Quantity: 1000\r\n\
Unit price: 39.80\r\n\
Currency: USD\r\n\
Quote date: 2026-10-19\r\n\
\r\n\
> Earlier: Unit price: 99.99\r\n";
    const H: &str = "from:to:subject:date:message-id:content-type:from:subject:date:content-type";
    const MANIFEST: &str = r#"{"format":"amoeba-email-manifest-v1","name":"Fixture distributor quote","evidenceType":"quotation","signingDomain":"distributor.example","fields":{"price":"Unit price:","currency":"Currency:","product":"Part number:","quantityTier":"Quantity:","recordId":"Quote number:","observedAt":"Quote date:"}}"#;

    fn policy(domain: &str, record: &str) -> crate::policy::Policy {
        let mut p = crate::policy::Policy::development("none", vec![2; 33], vec![]);
        p.dkim_keys.push(PinnedKey {
            domain: domain.into(),
            selector: "q1".into(),
            record: record.into(),
            trust: Trust::Development,
            provenance: "pinned for this test".into(),
        });
        p
    }

    fn bundle(message: &str) -> String {
        crate::bundle::EmailBundle {
            message: message.as_bytes().to_vec(),
            manifest: EmailManifest::parse(MANIFEST.as_bytes()).unwrap(),
            annotations: vec![],
        }
        .to_json()
    }

    #[test]
    fn a_signed_quote_is_a_quotation() {
        let (signed, record) = sign(QUOTE, "distributor.example", "q1", H, None);
        let mut v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert!(v.authenticated(), "{:?}", v.stages);
        let env = v.envelope.clone().unwrap();
        assert_eq!(env.observation.price.atoms, 39_800_000);
        assert_eq!(env.observation.evidence_type, EvidenceType::Quotation);
        assert_eq!(env.observation.fields["recordId"].value, "Q-1042");
        assert_eq!(
            env.observation.context["From is in the signing domain"],
            "true"
        );
        assert!(
            env.observation
                .context
                .contains_key("signed Date (the sender's claim)")
        );
        // The quoted earlier price is never read.
        assert_ne!(env.observation.price.text, "99.99");
        // A quote is not a public series: not a source under the rules.
        crate::check_destination(
            &mut v,
            &crate::Destination::Candidate {
                market: "ramx".into(),
                expiry: "RAMX-202612".into(),
                bucket: "b".into(),
                canonical_url: env.origin.locator.clone(),
                submitter: None,
            },
        );
        assert_eq!(v.stages.eligibility.outcome, crate::Outcome::Failed);
    }

    #[test]
    fn keys_come_from_the_verifier_not_the_message() {
        let (signed, _record) = sign(QUOTE, "distributor.example", "q1", H, None);
        let (_, other) = sign(QUOTE, "distributor.example", "q1", H, None);
        // Another key under the same name: no signature verifies.
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &other),
        );
        assert_eq!(v.stages.provenance.outcome, crate::Outcome::Failed);
        // No key at all for the domain.
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &crate::policy::Policy::development("n", vec![2; 33], vec![]),
        );
        assert_eq!(v.stages.provenance.outcome, crate::Outcome::Failed);
    }

    #[test]
    fn a_display_name_is_not_an_identity() {
        // Signed by another domain, displaying the distributor's name.
        let (signed, record) = sign(QUOTE, "evil.example", "q1", H, None);
        let v = crate::verify_bundle(bundle(&signed).as_bytes(), &policy("evil.example", &record));
        assert_eq!(
            v.stages.provenance.outcome,
            crate::Outcome::Failed,
            "{:?}",
            v.stages
        );
        // Signed by the distributor's domain with a From outside it: kept,
        // and said so.
        let spoofed = QUOTE.replace("quotes@distributor.example", "ceo@bigvendor.example");
        let (signed, record) = sign(&spoofed, "distributor.example", "q1", H, None);
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(
            v.envelope.unwrap().observation.context["From is in the signing domain"],
            "false"
        );
    }

    #[test]
    fn unsigned_appended_text_is_refused() {
        let body_len = crate::dkim::tests_support::relaxed_body(
            QUOTE.split_once("\r\n\r\n").unwrap().1.as_bytes(),
        )
        .len();
        let (signed, record) = sign(QUOTE, "distributor.example", "q1", H, Some(body_len));
        let appended = format!("{signed}Unit price: 1.00\r\n");
        let v = crate::verify_bundle(
            bundle(&appended).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(v.stages.provenance.outcome, crate::Outcome::Failed);
        assert!(v.stages.provenance.reason.unwrap().contains("covers only"));
        // The same signature over the untouched body is fine.
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert!(v.authenticated(), "{:?}", v.stages);
        // A tampered price fails the body hash.
        let tampered = signed.replace("Unit price: 39.80", "Unit price: 29.80");
        let v = crate::verify_bundle(
            bundle(&tampered).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(v.stages.provenance.outcome, crate::Outcome::Failed);
    }

    #[test]
    fn a_reissued_quote_is_the_same_observation() {
        let (a, ka) = sign(QUOTE, "distributor.example", "q1", H, None);
        let reissued = QUOTE
            .replace(
                "<q-1042@distributor.example>",
                "<q-1042-resend@distributor.example>",
            )
            .replace("10:00:00", "16:30:00");
        let (b, kb) = sign(&reissued, "distributor.example", "q1", H, None);
        let va = crate::verify_bundle(bundle(&a).as_bytes(), &policy("distributor.example", &ka));
        let vb = crate::verify_bundle(bundle(&b).as_bytes(), &policy("distributor.example", &kb));
        let (ea, eb) = (va.envelope.unwrap(), vb.envelope.unwrap());
        assert_ne!(
            ea.authentication.bundle_digest,
            eb.authentication.bundle_digest
        );
        assert_eq!(ea.observation_id, eb.observation_id);
        // A new price is a new observation.
        let (c, kc) = sign(
            &QUOTE.replace("39.80", "38.10"),
            "distributor.example",
            "q1",
            H,
            None,
        );
        let vc = crate::verify_bundle(bundle(&c).as_bytes(), &policy("distributor.example", &kc));
        assert_ne!(vc.envelope.unwrap().observation_id, ea.observation_id);
    }

    #[test]
    fn unsigned_headers_are_not_evidence() {
        // Date not signed: no Date in the evidence.
        let (signed, record) = sign(
            QUOTE,
            "distributor.example",
            "q1",
            "from:to:subject:message-id:content-type:from:subject:content-type",
            None,
        );
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        let env = v.envelope.unwrap();
        assert!(
            !env.observation
                .context
                .contains_key("signed Date (the sender's claim)")
        );
        // Content-Type not signed: the body cannot be read as signed MIME.
        let (signed, record) = sign(
            QUOTE,
            "distributor.example",
            "q1",
            "from:to:subject:date",
            None,
        );
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(
            v.stages.extraction.outcome,
            crate::Outcome::Failed,
            "{:?}",
            v.stages
        );
        // An extra unsigned Subject above the signed ones is not read.
        let (signed, record) = sign(QUOTE, "distributor.example", "q1", H, None);
        let extra = signed.replacen(
            "From: Quotes",
            "Subject: Price list\r\nSubject: again\r\nSubject: third\r\nFrom: Quotes",
            1,
        );
        let v = crate::verify_bundle(
            bundle(&extra).as_bytes(),
            &policy("distributor.example", &record),
        );
        if let Some(env) = v.envelope {
            assert!(!env.observation.context.contains_key("signed Subject"));
        }
    }

    #[test]
    fn forwarded_and_quoted_only_prices_are_not_read() {
        let forwarded = QUOTE.replace(
            "FIXTURE QUOTE",
            "---------- Forwarded message ----------\r\nFIXTURE QUOTE",
        );
        let (signed, record) = sign(&forwarded, "distributor.example", "q1", H, None);
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(
            v.stages.extraction.outcome,
            crate::Outcome::Unsupported,
            "{:?}",
            v.stages
        );
        let quoted_only = QUOTE.replace("Unit price: 39.80\r\n", "");
        let (signed, record) = sign(&quoted_only, "distributor.example", "q1", H, None);
        let v = crate::verify_bundle(
            bundle(&signed).as_bytes(),
            &policy("distributor.example", &record),
        );
        assert_eq!(v.stages.extraction.outcome, crate::Outcome::Failed);
    }
}
