//! DKIM signature verification (RFC 6376; Ed25519-SHA256 from RFC 8463).
//!
//! A passing check establishes that the holder of the key published for
//! `<selector>._domainkey.<d>` signed these header fields and this body.
//! It does not establish who wrote the text, that the displayed From is
//! that domain's mailbox (see `from_aligned`), when it was delivered, that
//! the recipient owns the address it was sent to, or that the quote in it
//! is a sale.
//!
//! Stricter than a mail filter on purpose:
//!
//! - a signature with `l=` that leaves any body byte uncovered is refused
//!   (appended text after the signed length would read as signed);
//! - a header used as evidence must be signed as often as it occurs, so an
//!   added unsigned `From:` or `Subject:` cannot sit above the signed one;
//! - the key comes from the verifier's own key policy, never from the
//!   message or the bundle alone.

use base64::Engine as _;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DkimError {
    #[error("the message is malformed: {0}")]
    Malformed(&'static str),
    #[error("the message has no DKIM signature")]
    Unsigned,
    #[error("the DKIM signature is malformed: {0}")]
    BadSignature(String),
    #[error("{0} is not supported")]
    Unsupported(String),
    #[error("the DKIM key record is malformed: {0}")]
    BadKey(String),
    #[error("the body does not match the signed body hash")]
    BodyHash,
    #[error("the signature does not verify")]
    Signature,
    #[error(
        "the signature covers only the first {0} bytes of the body; text after them would read as signed"
    )]
    PartialBody(usize),
}

/// One header field: its name and the whole raw field as received (name,
/// colon, value, folding, final CRLF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub raw: Vec<u8>,
}

impl Header {
    /// The value, unfolded and trimmed (for display and extraction).
    pub fn value(&self) -> String {
        let text = String::from_utf8_lossy(&self.raw);
        let v = text.split_once(':').map(|(_, v)| v).unwrap_or("");
        v.replace("\r\n", "").trim().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub headers: Vec<Header>,
    /// Everything after the blank line, CRLF line ends.
    pub body: Vec<u8>,
}

/// Parses a message, turning bare LF line ends (an `.eml` saved on Unix)
/// into the CRLF DKIM signs over.
pub fn parse_message(bytes: &[u8]) -> Result<Message, DkimError> {
    let mut crlf = Vec::with_capacity(bytes.len() + bytes.len() / 32);
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && (i == 0 || bytes[i - 1] != b'\r') {
            crlf.push(b'\r');
        }
        crlf.push(*b);
    }
    let split = crlf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p + 2, p + 4))
        .ok_or(DkimError::Malformed("no blank line after the headers"))?;
    let (head, body) = (&crlf[..split.0], crlf[split.1..].to_vec());
    let mut headers: Vec<Header> = Vec::new();
    let mut i = 0;
    while i < head.len() {
        let end = head[i..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|p| i + p + 2)
            .ok_or(DkimError::Malformed("a header line without CRLF"))?;
        let line = &head[i..end];
        if line[0] == b' ' || line[0] == b'\t' {
            let last = headers.last_mut().ok_or(DkimError::Malformed(
                "a continuation line before any header",
            ))?;
            last.raw.extend_from_slice(line);
        } else {
            let colon = line
                .iter()
                .position(|c| *c == b':')
                .ok_or(DkimError::Malformed("a header line without a colon"))?;
            let name = std::str::from_utf8(&line[..colon])
                .map_err(|_| DkimError::Malformed("a header name that is not ASCII"))?;
            if name.is_empty() || name.bytes().any(|c| c <= b' ' || c >= 0x7f) {
                return Err(DkimError::Malformed("a malformed header name"));
            }
            headers.push(Header {
                name: name.to_string(),
                raw: line.to_vec(),
            });
        }
        i = end;
    }
    Ok(Message { headers, body })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    RsaSha256,
    Ed25519Sha256,
}

impl Algorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RsaSha256 => "rsa-sha256",
            Self::Ed25519Sha256 => "ed25519-sha256",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Canon {
    Simple,
    Relaxed,
}

#[derive(Debug, Clone)]
pub struct Signature {
    pub algorithm: Algorithm,
    header_canon: Canon,
    body_canon: Canon,
    pub domain: String,
    pub selector: String,
    /// The agent or user identity (`i=`), default `@d`.
    pub identity: String,
    /// Signed header names, lowercase, in order (repeats included).
    pub signed: Vec<String>,
    body_hash: Vec<u8>,
    signature: Vec<u8>,
    pub length: Option<usize>,
    pub timestamp: Option<u64>,
    pub expiration: Option<u64>,
    /// Which header this is in the message.
    index: usize,
}

fn tags(value: &str) -> Result<Vec<(String, String)>, DkimError> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in value.split(';') {
        let part = part.trim_matches(|c: char| c.is_ascii_whitespace());
        if part.is_empty() {
            continue;
        }
        let (k, v) = part
            .split_once('=')
            .ok_or_else(|| DkimError::BadSignature(format!("`{part}` is not tag=value")))?;
        let k = k.trim().to_string();
        if out.iter().any(|(old, _)| *old == k) {
            return Err(DkimError::BadSignature(format!("tag {k} repeats")));
        }
        out.push((k, v.trim().to_string()));
    }
    Ok(out)
}

fn b64(text: &str, what: &str) -> Result<Vec<u8>, DkimError> {
    let compact: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|_| DkimError::BadSignature(format!("{what} is not base64")))
}

fn parse_signature(h: &Header, index: usize) -> Result<Signature, DkimError> {
    let t = tags(&h.value())?;
    let get = |k: &str| t.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let need = |k: &str| get(k).ok_or_else(|| DkimError::BadSignature(format!("no {k}= tag")));
    if need("v")? != "1" {
        return Err(DkimError::Unsupported("a DKIM version other than 1".into()));
    }
    let algorithm = match need("a")? {
        "rsa-sha256" => Algorithm::RsaSha256,
        "ed25519-sha256" => Algorithm::Ed25519Sha256,
        other => return Err(DkimError::Unsupported(format!("signing algorithm {other}"))),
    };
    let c = get("c").unwrap_or("simple/simple");
    let (h, b) = c.split_once('/').unwrap_or((c, "simple"));
    let canon = |x: &str| match x {
        "simple" => Ok(Canon::Simple),
        "relaxed" => Ok(Canon::Relaxed),
        other => Err(DkimError::Unsupported(format!("canonicalization {other}"))),
    };
    let (hc, bc) = (canon(h)?, canon(b)?);
    let domain = need("d")?.to_ascii_lowercase();
    let selector = need("s")?.to_ascii_lowercase();
    if domain.is_empty() || selector.is_empty() || !domain.contains('.') {
        return Err(DkimError::BadSignature("d= or s= is empty".into()));
    }
    let identity = get("i")
        .map(str::to_string)
        .unwrap_or_else(|| format!("@{domain}"));
    let i_domain = identity
        .rsplit_once('@')
        .map(|(_, d)| d.to_ascii_lowercase())
        .unwrap_or_default();
    if i_domain != domain && !i_domain.ends_with(&format!(".{domain}")) {
        return Err(DkimError::BadSignature("i= is not within d=".into()));
    }
    let signed: Vec<String> = need("h")?
        .split(':')
        .map(|n| n.trim().to_ascii_lowercase())
        .filter(|n| !n.is_empty())
        .collect();
    if !signed.iter().any(|n| n == "from") {
        return Err(DkimError::BadSignature("h= does not sign From".into()));
    }
    if let Some(q) = get("q")
        && !q.split(':').any(|m| m.trim() == "dns/txt")
    {
        return Err(DkimError::Unsupported(format!("key query method {q}")));
    }
    let num = |k: &str| -> Result<Option<u64>, DkimError> {
        get(k)
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| DkimError::BadSignature(format!("{k}= is not a number")))
            })
            .transpose()
    };
    Ok(Signature {
        algorithm,
        header_canon: hc,
        body_canon: bc,
        domain,
        selector,
        identity,
        signed,
        body_hash: b64(need("bh")?, "bh=")?,
        signature: b64(need("b")?, "b=")?,
        length: num("l")?.map(|l| l as usize),
        timestamp: num("t")?,
        expiration: num("x")?,
        index,
    })
}

/// The message's DKIM signatures, in order (each parsed on its own: one
/// malformed signature doesn't hide another).
pub fn signatures(msg: &Message) -> Vec<Result<Signature, DkimError>> {
    msg.headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.name.eq_ignore_ascii_case("dkim-signature"))
        .map(|(i, h)| parse_signature(h, i))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyType {
    Rsa,
    Ed25519,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecord {
    pub key_type: KeyType,
    /// RSA: the RSAPublicKey (PKCS#1) DER; Ed25519: the 32-byte key.
    pub public: Vec<u8>,
    /// The record as published (the TXT value).
    pub record: String,
}

/// Minimal DER: the content and the rest after one TLV.
fn der(input: &[u8], tag: u8) -> Result<(&[u8], &[u8]), DkimError> {
    let bad = || DkimError::BadKey("not a DER public key".into());
    if input.first() != Some(&tag) || input.len() < 2 {
        return Err(bad());
    }
    let (len, head): (usize, usize) = match input[1] {
        n if n < 0x80 => (n as usize, 2),
        0x81 => (*input.get(2).ok_or_else(bad)? as usize, 3),
        0x82 => (
            u16::from_be_bytes([
                *input.get(2).ok_or_else(bad)?,
                *input.get(3).ok_or_else(bad)?,
            ]) as usize,
            4,
        ),
        _ => return Err(bad()),
    };
    let end = head
        .checked_add(len)
        .filter(|e| *e <= input.len())
        .ok_or_else(bad)?;
    Ok((&input[head..end], &input[end..]))
}

/// The RSAPublicKey inside a SubjectPublicKeyInfo (or the key itself when
/// the record holds a bare RSAPublicKey, as some signers publish).
fn rsa_public_key(p: &[u8]) -> Result<Vec<u8>, DkimError> {
    let (spki, _) = der(p, 0x30)?;
    // SPKI: SEQUENCE { SEQUENCE { OID rsaEncryption, NULL }, BIT STRING }.
    if let Ok((alg, rest)) = der(spki, 0x30) {
        const RSA_OID: &[u8] = &[
            0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
        ];
        if !alg.starts_with(RSA_OID) {
            return Err(DkimError::BadKey("the key is not RSA".into()));
        }
        let (bits, _) = der(rest, 0x03)?;
        if bits.first() != Some(&0) {
            return Err(DkimError::BadKey("a malformed key bit string".into()));
        }
        return Ok(bits[1..].to_vec());
    }
    Ok(p.to_vec())
}

pub fn parse_key_record(record: &str) -> Result<KeyRecord, DkimError> {
    let t = tags(record).map_err(|e| DkimError::BadKey(e.to_string()))?;
    let get = |k: &str| t.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if let Some(v) = get("v")
        && v != "DKIM1"
    {
        return Err(DkimError::BadKey(format!("v={v}")));
    }
    if get("t").is_some_and(|f| f.split(':').any(|x| x.trim() == "y")) {
        return Err(DkimError::Unsupported("a key in testing mode (t=y)".into()));
    }
    let p = get("p").ok_or_else(|| DkimError::BadKey("no p= tag".into()))?;
    if p.is_empty() {
        return Err(DkimError::BadKey("the key is revoked (empty p=)".into()));
    }
    let raw = b64(p, "p=").map_err(|e| DkimError::BadKey(e.to_string()))?;
    let (key_type, public) = match get("k").unwrap_or("rsa") {
        "rsa" => (KeyType::Rsa, rsa_public_key(&raw)?),
        "ed25519" => {
            if raw.len() != 32 {
                return Err(DkimError::BadKey("an Ed25519 key must be 32 bytes".into()));
            }
            (KeyType::Ed25519, raw)
        }
        other => return Err(DkimError::Unsupported(format!("key type {other}"))),
    };
    Ok(KeyRecord {
        key_type,
        public,
        record: record.to_string(),
    })
}

fn is_wsp(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn relaxed_header(h: &Header) -> Vec<u8> {
    let raw = &h.raw;
    let colon = raw.iter().position(|c| *c == b':').unwrap_or(raw.len());
    let mut out = h.name.trim().to_ascii_lowercase().into_bytes();
    out.push(b':');
    // Unfold, collapse whitespace runs, trim.
    let mut value = Vec::new();
    let mut space = false;
    for &c in &raw[colon + 1..] {
        if c == b'\r' || c == b'\n' {
            continue;
        }
        if is_wsp(c) {
            space = true;
        } else {
            if space && !value.is_empty() {
                value.push(b' ');
            }
            space = false;
            value.push(c);
        }
    }
    out.extend_from_slice(&value);
    out.extend_from_slice(b"\r\n");
    out
}

fn canon_header(c: Canon, h: &Header) -> Vec<u8> {
    match c {
        Canon::Simple => h.raw.clone(),
        Canon::Relaxed => relaxed_header(h),
    }
}

/// The canonical body (RFC 6376 3.4.3 and 3.4.4).
fn canon_body(c: Canon, body: &[u8]) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = body
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
        .collect();
    // `split` leaves an empty piece after a final CRLF.
    if body.ends_with(b"\n") {
        lines.pop();
    }
    if c == Canon::Relaxed {
        for line in &mut lines {
            let mut out = Vec::new();
            let mut space = false;
            for &ch in line.iter() {
                if is_wsp(ch) {
                    space = true;
                } else {
                    if space {
                        out.push(b' ');
                    }
                    space = false;
                    out.push(ch);
                }
            }
            *line = out;
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return match c {
            Canon::Simple => b"\r\n".to_vec(),
            Canon::Relaxed => Vec::new(),
        };
    }
    let mut out = Vec::new();
    for l in lines {
        out.extend_from_slice(&l);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// The DKIM-Signature header with its `b=` value (and the whitespace
/// around it) emptied, canonicalized, without its final CRLF. Tags split
/// at `;`, which no tag value of a signature contains.
fn signature_header_input(c: Canon, h: &Header) -> Vec<u8> {
    let raw = &h.raw;
    let colon = raw.iter().position(|ch| *ch == b':').unwrap_or(0);
    let mut out = raw[..=colon].to_vec();
    let trim = |s: &[u8]| -> Vec<u8> {
        s.iter()
            .copied()
            .filter(|ch| !matches!(ch, b' ' | b'\t' | b'\r' | b'\n'))
            .collect()
    };
    for (i, part) in raw[colon + 1..].split(|ch| *ch == b';').enumerate() {
        if i > 0 {
            out.push(b';');
        }
        match part.iter().position(|ch| *ch == b'=') {
            Some(eq) if trim(&part[..eq]) == b"b" => out.extend_from_slice(&part[..=eq]),
            _ => out.extend_from_slice(part),
        }
    }
    let mut canon = canon_header(
        c,
        &Header {
            name: h.name.clone(),
            raw: out,
        },
    );
    while canon.ends_with(b"\r\n") {
        canon.truncate(canon.len() - 2);
    }
    canon
}

#[derive(Debug, Clone)]
pub struct Verified {
    pub signature: Signature,
    /// The header fields the signature covers, as (lowercase name, value),
    /// in the order they were signed.
    pub signed_headers: Vec<(String, String)>,
    /// How many times each signed name occurs in the message.
    pub occurrences: Vec<(String, usize)>,
}

impl Verified {
    /// A header usable as evidence: signed, and signed as often as it
    /// occurs (no unsigned copy above it).
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_ascii_lowercase();
        let signed = self
            .signature
            .signed
            .iter()
            .filter(|n| **n == lower)
            .count();
        let present = self
            .occurrences
            .iter()
            .find(|(n, _)| *n == lower)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        if present == 0 || signed < present {
            return None;
        }
        self.signed_headers
            .iter()
            .find(|(n, _)| *n == lower)
            .map(|(_, v)| v.as_str())
    }
}

/// What the signature signs: each name in `h=` takes the last instance not
/// used yet, counting from the bottom (RFC 6376 5.4.2; a missing one adds
/// nothing), then the signature header itself with `b=` emptied.
fn header_input(msg: &Message, sig: &Signature) -> (Vec<u8>, Vec<(String, String)>) {
    let mut used = vec![false; msg.headers.len()];
    let mut input = Vec::new();
    let mut signed_headers = Vec::new();
    for name in &sig.signed {
        if let Some(i) = (0..msg.headers.len()).rev().find(|i| {
            !used[*i] && *i != sig.index && msg.headers[*i].name.eq_ignore_ascii_case(name)
        }) {
            used[i] = true;
            input.extend_from_slice(&canon_header(sig.header_canon, &msg.headers[i]));
            signed_headers.push((name.clone(), msg.headers[i].value()));
        }
    }
    input.extend_from_slice(&signature_header_input(
        sig.header_canon,
        &msg.headers[sig.index],
    ));
    (input, signed_headers)
}

impl Signature {
    /// Relaxed header canonicalization (`c=relaxed/...`).
    pub fn relaxed_headers(&self) -> bool {
        self.header_canon == Canon::Relaxed
    }

    /// The signature value (`b=`).
    pub fn value(&self) -> &[u8] {
        &self.signature
    }

    /// Exactly what the signature signs: the header input (signed fields,
    /// canonicalized, then this signature's field with `b=` empty, no final
    /// CRLF) and the canonical body. The zero-knowledge method proves over
    /// these bytes (`crate::email_zk`).
    pub fn signed_input(&self, msg: &Message) -> (Vec<u8>, Vec<u8>) {
        (
            header_input(msg, self).0,
            canon_body(self.body_canon, &msg.body),
        )
    }
}

/// The modulus and public exponent of an RSA key record's RSAPublicKey
/// (big-endian, without leading zeros).
pub fn rsa_modulus_exponent(key: &KeyRecord) -> Result<(Vec<u8>, Vec<u8>), DkimError> {
    if key.key_type != KeyType::Rsa {
        return Err(DkimError::BadKey("not an RSA key".into()));
    }
    let (seq, _) = der(&key.public, 0x30)?;
    let (n, rest) = der(seq, 0x02)?;
    let (e, _) = der(rest, 0x02)?;
    let strip = |b: &[u8]| -> Vec<u8> {
        let first = b.iter().position(|x| *x != 0).unwrap_or(b.len());
        b[first..].to_vec()
    };
    Ok((strip(n), strip(e)))
}

/// Verifies one signature with `key`.
pub fn verify(msg: &Message, sig: &Signature, key: &KeyRecord) -> Result<Verified, DkimError> {
    // Body.
    let body = canon_body(sig.body_canon, &msg.body);
    if let Some(l) = sig.length
        && l < body.len()
    {
        return Err(DkimError::PartialBody(l));
    }
    let hash: [u8; 32] = Sha256::digest(&body).into();
    if hash.as_slice() != sig.body_hash.as_slice() {
        return Err(DkimError::BodyHash);
    }
    let (input, signed_headers) = header_input(msg, sig);
    let ok = match (sig.algorithm, &key.key_type) {
        (Algorithm::RsaSha256, KeyType::Rsa) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_1024_8192_SHA256_FOR_LEGACY_USE_ONLY,
            &key.public,
        )
        .verify(&input, &sig.signature)
        .is_ok(),
        (Algorithm::Ed25519Sha256, KeyType::Ed25519) => {
            let digest: [u8; 32] = Sha256::digest(&input).into();
            ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key.public)
                .verify(&digest, &sig.signature)
                .is_ok()
        }
        _ => {
            return Err(DkimError::BadKey(
                "the key type does not match the signature algorithm".into(),
            ));
        }
    };
    if !ok {
        return Err(DkimError::Signature);
    }
    let mut occurrences: Vec<(String, usize)> = Vec::new();
    for name in &sig.signed {
        if !occurrences.iter().any(|(n, _)| n == name) {
            let c = msg
                .headers
                .iter()
                .filter(|h| h.name.eq_ignore_ascii_case(name))
                .count();
            occurrences.push((name.clone(), c));
        }
    }
    Ok(Verified {
        signature: sig.clone(),
        signed_headers,
        occurrences,
    })
}

/// Canonicalization for test signers (email.rs tests).
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn relaxed_body(body: &[u8]) -> Vec<u8> {
        canon_body(Canon::Relaxed, body)
    }

    /// The signed input for the message's first DKIM-Signature (whose
    /// `b=` is still empty).
    pub(crate) fn header_input(msg: &Message, _h: &str) -> Vec<u8> {
        let sig = signatures(msg).into_iter().next().unwrap().unwrap();
        super::header_input(msg, &sig).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const EXAMPLE: &[u8] = include_bytes!("../tests/data/rfc8463-example.eml");
    // RFC 8463 A.2, the published public keys (no secret key is kept here).
    pub(crate) const ED25519_RECORD: &str =
        "v=DKIM1; k=ed25519; p=11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
    pub(crate) const RSA_RECORD: &str = "v=DKIM1; k=rsa; p=MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDkHlOQoBTzWRiGs5V6NpP3idY6Wk08a5qhdR6wy5bdOKb2jLQiY/J16JYi0Qvx/byYzCNb3W91y3FutACDfzwQ/BC/e/8uBsCR+yz1Lxj+PL6lHvqMKrM3rG4hstT5QjvHO9PzoxZyVYLzBfO2EeC3Ip3G+2kryOTIKT+l/K4w3QIDAQAB";

    #[test]
    fn rfc8463_example_verifies_with_both_keys() {
        let msg = parse_message(EXAMPLE).unwrap();
        let sigs: Vec<Signature> = signatures(&msg).into_iter().map(Result::unwrap).collect();
        assert_eq!(sigs.len(), 2);
        let ed = verify(&msg, &sigs[0], &parse_key_record(ED25519_RECORD).unwrap()).unwrap();
        assert_eq!(
            (ed.signature.domain.as_str(), ed.signature.selector.as_str()),
            ("football.example.com", "brisbane")
        );
        assert_eq!(ed.header("subject"), Some("Is dinner ready?"));
        assert_eq!(
            ed.header("from"),
            Some("Joe SixPack <joe@football.example.com>")
        );
        verify(&msg, &sigs[1], &parse_key_record(RSA_RECORD).unwrap()).unwrap();
        // The wrong key, or one key for the other algorithm, fails.
        assert!(verify(&msg, &sigs[0], &parse_key_record(RSA_RECORD).unwrap()).is_err());
    }

    #[test]
    fn tampering_fails() {
        let text = String::from_utf8(EXAMPLE.to_vec()).unwrap();
        let key = parse_key_record(ED25519_RECORD).unwrap();
        let check = |t: &str| {
            let msg = parse_message(t.as_bytes()).unwrap();
            let sig = signatures(&msg).into_iter().next().unwrap().unwrap();
            verify(&msg, &sig, &key)
        };
        assert_eq!(
            check(&text.replace("hungry", "Hungry")).unwrap_err(),
            DkimError::BodyHash
        );
        assert_eq!(
            check(&text.replace("Is dinner ready?", "Is lunch ready?")).unwrap_err(),
            DkimError::Signature
        );
        // Appended text changes the body hash (no l= here).
        assert_eq!(
            check(&format!("{text}Unit price: 1.00\r\n")).unwrap_err(),
            DkimError::BodyHash
        );
        // Saved with LF line ends, it still verifies.
        check(&text.replace("\r\n", "\n")).unwrap();
    }

    #[test]
    fn an_unsigned_copy_of_a_signed_header_is_not_evidence() {
        let text = String::from_utf8(EXAMPLE.to_vec()).unwrap();
        // A third Subject above the others: over-signing (two in h=) no
        // longer covers every copy.
        let added = text.replacen(
            "From: Joe",
            "Subject: Price list\r\nSubject: again\r\nFrom: Joe",
            1,
        );
        let msg = parse_message(added.as_bytes()).unwrap();
        let sig = signatures(&msg).into_iter().next().unwrap().unwrap();
        if let Ok(v) = verify(&msg, &sig, &parse_key_record(ED25519_RECORD).unwrap()) {
            assert_eq!(v.header("subject"), None);
        }
    }

    #[test]
    fn key_records() {
        assert!(parse_key_record("v=DKIM1; p=").is_err());
        assert!(
            parse_key_record(
                "v=DKIM1; k=ed25519; t=y; p=11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo="
            )
            .is_err()
        );
        assert!(parse_key_record("v=DKIM1; k=dsa; p=AAAA").is_err());
        assert_eq!(parse_key_record(RSA_RECORD).unwrap().key_type, KeyType::Rsa);
    }

    #[test]
    fn body_canonicalization() {
        assert_eq!(canon_body(Canon::Relaxed, b""), b"");
        assert_eq!(canon_body(Canon::Simple, b""), b"\r\n");
        assert_eq!(
            canon_body(Canon::Relaxed, b"a  b \t\r\n\r\n\r\n"),
            b"a b\r\n"
        );
        assert_eq!(canon_body(Canon::Simple, b"a  b \r\n\r\n"), b"a  b \r\n");
    }
}
