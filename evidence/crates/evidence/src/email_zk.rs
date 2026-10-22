//! Zero-knowledge email evidence (`email_dkim_zk`): the quote's labelled
//! lines from a DKIM-signed message, proven without disclosing the rest of
//! the message.
//!
//! The circuit (`evidence/circuits/email_dkim_zk`, Noir, proven with
//! Barretenberg's UltraHonk in its zero-knowledge flavor) proves that an
//! RSA-2048 key signed a DKIM header input and a body such that the body
//! holds each manifest label exactly once outside quoted lines, at the start
//! of its line, followed by the disclosed value. What a verifier learns:
//!
//! - the signing domain `d=`, the selector `s=` and the signature time `t=`;
//! - the signed `Content-Type` and `Content-Transfer-Encoding` values (so it
//!   can apply the email method's MIME rules);
//! - whether the signed From address is within `d=` (computed in the
//!   circuit, the address itself stays hidden);
//! - each manifest label with its value.
//!
//! Nothing else: not the recipient, the subject, the date header, the other
//! lines, the body hash or the signature. The key is never taken from the
//! bundle: the verifier rebuilds the public inputs from the RSA key its own
//! policy pins for (`d=`, `s=`), so a proof made with any other key fails.
//!
//! Limits of this circuit (sizes, not policy): a signed header input up to
//! 1024 bytes, a canonical body up to 4096 bytes in up to 256 lines, ten
//! labels, labelled lines up to 192 bytes (indentation included), RSA-2048 with exponent 65537, relaxed header
//! canonicalization, a single-part `text/plain` body in 7bit or 8bit
//! (quoted-printable and base64 would need decoding in the circuit). Other
//! messages are `unsupported` here and can still use `email_dkim`.

use num_bigint::BigUint;

use crate::dkim::{self, KeyType, Message};
use crate::email::{self, EmailError, EmailManifest, KeyResolver, PinnedKey};

pub const CIRCUIT_NAME: &str = "email_dkim_zk";
/// The circuit's verification key (`bb write_vk -t evm`, bb 3.0.3), built
/// from `evidence/circuits/email_dkim_zk` with nargo 1.0.0-beta.18.
pub const VK: &[u8] = include_bytes!("../../../circuits/email_dkim_zk/vk.bin");
/// The circuit source, for provers that compile it themselves (petri).
pub const CIRCUIT_SOURCE: &[(&str, &str)] = &[
    (
        "Nargo.toml",
        include_str!("../../../circuits/email_dkim_zk/Nargo.toml"),
    ),
    (
        "src/main.nr",
        include_str!("../../../circuits/email_dkim_zk/src/main.nr"),
    ),
];
/// The toolchain the verification key comes from. A prover on another
/// version makes proofs this key does not verify.
pub const NARGO_VERSION: &str = "1.0.0-beta.18";
pub const BB_VERSION: &str = "3.0.3";

pub const MAX_HEADER: usize = 1024;
pub const MAX_BODY: usize = 4096;
pub const MAX_LINES: usize = 256;
pub const SLOTS: usize = 10;
pub const LINE_BYTES: usize = 192;
pub const FROM_BYTES: usize = 256;
const KEY_LIMBS: usize = 18;
const LABEL_FIELDS: usize = 4;
const VALUE_FIELDS: usize = 4;
const DOMAIN_FIELDS: usize = 9;
const SELECTOR_FIELDS: usize = 4;
const CT_FIELDS: usize = 4;
const CTE_FIELDS: usize = 1;
pub const RSA_EXPONENT: u32 = 65537;

/// The circuit's identity: SHA-256 of its verification key.
pub fn circuit_id() -> String {
    crate::hash::hex32(&crate::hash::sha256(VK))
}

/// What an `email_dkim_zk` bundle discloses (everything else stays in the
/// prover's message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub domain: String,
    pub selector: String,
    /// DKIM `t=`, 0 when the signature has none.
    pub signed_at: u64,
    pub content_type: String,
    pub transfer_encoding: Option<String>,
    pub from_aligned: bool,
    /// Manifest field → value, in the manifest's field order.
    pub values: Vec<(String, String)>,
}

/// One 32-byte big-endian field element.
pub type PublicInput = [u8; 32];

fn field_u64(v: u64) -> PublicInput {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&v.to_be_bytes());
    out
}

fn field_big(v: &BigUint) -> PublicInput {
    let bytes = v.to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    out
}

fn limbs(v: &BigUint) -> Vec<PublicInput> {
    let mask = (BigUint::from(1u8) << 120u32) - 1u8;
    (0..KEY_LIMBS)
        .map(|i| field_big(&((v >> (120 * i as u32)) & &mask)))
        .collect()
}

fn packed(bytes: &[u8], fields: usize) -> Result<Vec<PublicInput>, String> {
    if bytes.len() > fields * 31 {
        return Err(format!(
            "{} bytes, more than this circuit's {}",
            bytes.len(),
            fields * 31
        ));
    }
    if bytes.contains(&0) {
        return Err("a zero byte".into());
    }
    let mut out = Vec::with_capacity(fields);
    for f in 0..fields {
        let mut e = [0u8; 32];
        for j in 0..31 {
            e[1 + j] = bytes.get(f * 31 + j).copied().unwrap_or(0);
        }
        out.push(e);
    }
    Ok(out)
}

/// The RSA key the circuit takes: modulus and Barrett parameter
/// (`2^(2*2048+6) / modulus`, noir-bignum's `redc_param`). Only RSA keys of
/// 2048 bits with exponent 65537 fit this circuit.
pub fn rsa_key(key: &PinnedKey) -> Result<BigUint, EmailError> {
    let record = dkim::parse_key_record(&key.record)?;
    if record.key_type != KeyType::Rsa {
        return Err(EmailError::Unsupported(
            "a zero-knowledge proof for a key other than RSA".into(),
        ));
    }
    let (n, e) = dkim::rsa_modulus_exponent(&record)?;
    if BigUint::from_bytes_be(&e) != BigUint::from(RSA_EXPONENT) {
        return Err(EmailError::Unsupported(
            "an RSA exponent other than 65537 in a zero-knowledge proof".into(),
        ));
    }
    if n.len() != 256 {
        return Err(EmailError::Unsupported(format!(
            "a {}-bit RSA key in a zero-knowledge proof (the circuit takes 2048)",
            n.len() * 8
        )));
    }
    Ok(BigUint::from_bytes_be(&n))
}

fn redc(modulus: &BigUint) -> BigUint {
    (BigUint::from(1u8) << (2 * 2048 + 6) as u32) / modulus
}

/// The MIME rules of the email method, applied to the disclosed signed
/// values: one `text/plain` part in US-ASCII or UTF-8, 7bit or 8bit.
pub fn check_mime(content_type: &str, transfer_encoding: Option<&str>) -> Result<(), EmailError> {
    let mut parts = content_type.split(';');
    let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if mime != "text/plain" {
        return Err(EmailError::Unsupported(format!(
            "a {mime} message in a zero-knowledge proof (it reads one text/plain body)"
        )));
    }
    for p in parts {
        if let Some((k, v)) = p.split_once('=')
            && k.trim().eq_ignore_ascii_case("charset")
        {
            let cs = v.trim().trim_matches('"').to_ascii_lowercase();
            if !matches!(cs.as_str(), "us-ascii" | "utf-8" | "utf8") {
                return Err(EmailError::Unsupported(format!(
                    "charset {cs} in a zero-knowledge proof"
                )));
            }
        }
    }
    if let Some(enc) = transfer_encoding {
        let enc = enc.trim().to_ascii_lowercase();
        if !matches!(enc.as_str(), "7bit" | "8bit" | "binary") {
            return Err(EmailError::Unsupported(format!(
                "transfer encoding {enc} in a zero-knowledge proof (decoding is not in the circuit)"
            )));
        }
    }
    Ok(())
}

/// The manifest's fields in circuit slot order: by field name, as the
/// canonical manifest lists them (a manifest written in another order fills
/// the same slots).
pub fn slots(m: &EmailManifest) -> Vec<(String, String)> {
    let mut f = m.fields.clone();
    f.sort();
    f
}

/// The circuit's public inputs, in its parameter order, rebuilt from the
/// verifier's own key, the manifest and the claims.
pub fn public_inputs(
    modulus: &BigUint,
    m: &EmailManifest,
    c: &Claims,
) -> Result<Vec<PublicInput>, String> {
    if m.fields.len() > SLOTS {
        return Err(format!(
            "the manifest has {} fields; this circuit reads {SLOTS}",
            m.fields.len()
        ));
    }
    let mut out = limbs(modulus);
    out.extend(limbs(&redc(modulus)));
    out.extend(packed(c.domain.as_bytes(), DOMAIN_FIELDS).map_err(|e| format!("d=: {e}"))?);
    out.push(field_u64(c.domain.len() as u64));
    out.extend(packed(c.selector.as_bytes(), SELECTOR_FIELDS).map_err(|e| format!("s=: {e}"))?);
    out.push(field_u64(c.selector.len() as u64));
    out.push(field_u64(c.signed_at));
    out.extend(
        packed(c.content_type.as_bytes(), CT_FIELDS).map_err(|e| format!("Content-Type: {e}"))?,
    );
    out.push(field_u64(c.content_type.len() as u64));
    let cte = c.transfer_encoding.as_deref().unwrap_or("");
    if c.transfer_encoding.is_some() && cte.is_empty() {
        return Err("an empty Content-Transfer-Encoding".into());
    }
    out.extend(
        packed(cte.as_bytes(), CTE_FIELDS)
            .map_err(|e| format!("Content-Transfer-Encoding: {e}"))?,
    );
    out.push(field_u64(cte.len() as u64));
    out.push(field_u64(c.from_aligned as u64));
    let value_of = |k: &str| {
        c.values
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    };
    let fields = slots(m);
    let mut labels = Vec::new();
    let mut label_lens = Vec::new();
    let mut values = Vec::new();
    let mut value_lens = Vec::new();
    for k in 0..SLOTS {
        let (label, value) = match fields.get(k) {
            Some((name, label)) => {
                let v = value_of(name).ok_or_else(|| format!("no value for `{name}`"))?;
                if v.is_empty()
                    || v.starts_with([' ', '\t'])
                    || v.ends_with([' ', '\t'])
                    || v.contains(['\r', '\n'])
                {
                    return Err(format!("the value for `{name}` is not one trimmed line"));
                }
                if !label.as_bytes()[0].is_ascii_graphic() || label.starts_with('>') {
                    return Err(format!(
                        "the label `{label}` must start with a visible ASCII character other than >"
                    ));
                }
                (label.as_str(), v)
            }
            None => ("", ""),
        };
        labels.extend(
            packed(label.as_bytes(), LABEL_FIELDS).map_err(|e| format!("label `{label}`: {e}"))?,
        );
        label_lens.push(field_u64(label.len() as u64));
        values.extend(
            packed(value.as_bytes(), VALUE_FIELDS).map_err(|e| format!("value `{value}`: {e}"))?,
        );
        value_lens.push(field_u64(value.len() as u64));
    }
    if c.values.len() != m.fields.len() {
        return Err("the claims name other fields than the manifest".into());
    }
    out.extend(labels);
    out.extend(label_lens);
    out.extend(values);
    out.extend(value_lens);
    Ok(out)
}

/// Verifies an UltraHonk (ZK, Keccak) proof against this circuit's key.
pub fn verify_proof(proof: &[u8], public: &[PublicInput]) -> Result<(), String> {
    let proof = ultrahonk_no_std::ProofType::ZK(proof.to_vec().into_boxed_slice());
    ultrahonk_no_std::verify::<()>(VK, &proof, public).map_err(|e| format!("{e:?}"))
}

/// What a prover hands the circuit: the claims it will disclose and the
/// circuit's inputs (`Prover.toml`).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub claims: Claims,
    pub key: PinnedKey,
    pub prover_toml: String,
}

fn wsp(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// The circuit's DKIM tag starts in the header input: the first non-WSP
/// byte after the field's colon or a `;`. Positions are header indices.
fn tag_starts(h: &[u8], region: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut after_semi = true;
    for (i, &c) in h.iter().enumerate().skip(region) {
        if !wsp(c) {
            if after_semi {
                out.push(i);
            }
            after_semi = c == b';';
        }
    }
    out
}

/// A tag value as the circuit reads it: after `=` and at most one space, up
/// to `;`, a space or the end. Returns (value start, value).
fn tag_value(h: &[u8], eq: usize) -> Option<(usize, &[u8])> {
    let v = if h.get(eq + 1) == Some(&b' ') {
        eq + 2
    } else {
        eq + 1
    };
    let end = h[v..]
        .iter()
        .position(|c| *c == b';' || wsp(*c))
        .map(|p| v + p)
        .unwrap_or(h.len());
    (end > v).then(|| (v, &h[v..end]))
}

fn find_field(h: &[u8], name: &[u8]) -> Vec<usize> {
    // Field starts follow a CRLF; the first field follows a virtual one.
    let mut padded = b"\r\n".to_vec();
    padded.extend_from_slice(h);
    let mut pat = b"\r\n".to_vec();
    pat.extend_from_slice(name);
    padded
        .windows(pat.len())
        .enumerate()
        .filter(|(_, w)| *w == pat.as_slice())
        .map(|(i, _)| i) // header index of the field name
        .collect()
}

fn field_value(h: &[u8], name_at: usize, name_len: usize) -> Option<&[u8]> {
    let v = name_at + name_len;
    let end = h[v..].windows(2).position(|w| w == b"\r\n")? + v;
    Some(&h[v..end])
}

/// The circuit's From rule: after the last `@`, strip trailing spaces, tabs
/// and `>`; the rest is the signing domain or ends with `.` + it.
pub fn from_aligned(from_value: &[u8], domain: &[u8]) -> bool {
    let Some(at) = from_value.iter().rposition(|c| *c == b'@') else {
        return false;
    };
    let tail_end = from_value
        .iter()
        .rposition(|c| !(wsp(*c) || *c == b'>'))
        .map(|p| p + 1)
        .unwrap_or(0);
    if tail_end <= at + 1 {
        return false;
    }
    let dom = &from_value[at + 1..tail_end];
    if dom.len() < domain.len() {
        return false;
    }
    let off = dom.len() - domain.len();
    dom[off..].eq_ignore_ascii_case(domain) && (off == 0 || dom[off - 1] == b'.')
}

/// What the circuit treats as possible whitespace before a line's text:
/// spaces, tabs, CR, VT, FF and any non-ASCII byte.
fn loose(c: u8) -> bool {
    wsp(c) || matches!(c, b'\r' | 0x0b | 0x0c) || c >= 0x80
}

/// The circuit's lines: (start, text start) for each, split at LF, the
/// last one after the final LF (possibly empty).
fn lines(b: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = vec![0];
    starts.extend(
        b.iter()
            .enumerate()
            .filter(|(_, c)| **c == b'\n')
            .map(|(i, _)| i + 1),
    );
    starts
        .iter()
        .map(|&ls| {
            let mut cs = ls;
            while cs < b.len() && loose(b[cs]) {
                cs += 1;
            }
            (ls, cs)
        })
        .collect()
}

/// (line index, value position, value) for one label, or why not:
/// exactly one unquoted line whose text starts with the label, after
/// spaces or tabs only.
fn labelled_line(
    b: &[u8],
    lines: &[(usize, usize)],
    label: &str,
) -> Result<(usize, usize, String), EmailError> {
    let l = label.as_bytes();
    let hits: Vec<(usize, (usize, usize))> = lines
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, (_, cs))| b.get(cs) != Some(&b'>') && b[cs..].starts_with(l))
        .collect();
    let (li, (ls, at)) = match hits.as_slice() {
        [one] => *one,
        [] => {
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
    };
    if !b[ls..at].iter().all(|c| wsp(*c)) {
        return Err(EmailError::Unsupported(format!(
            "a `{label}` line indented with other than spaces or tabs in a zero-knowledge proof"
        )));
    }
    let mut v = at + l.len();
    while v < b.len() && wsp(b[v]) {
        v += 1;
    }
    let cr = b[v..]
        .iter()
        .position(|c| *c == b'\r' || *c == b'\n')
        .map(|p| v + p)
        .ok_or_else(|| EmailError::Body(format!("the `{label}` line does not end")))?;
    if b[cr] != b'\r' || b.get(cr + 1) != Some(&b'\n') {
        return Err(EmailError::Body(format!(
            "the `{label}` line does not end with CRLF"
        )));
    }
    let mut end = cr;
    while end > v && wsp(b[end - 1]) {
        end -= 1;
    }
    if end == v {
        return Err(EmailError::Body(format!(
            "the signed text has no `{label}` line"
        )));
    }
    if cr - ls + 2 > LINE_BYTES {
        return Err(EmailError::Unsupported(format!(
            "a `{label}` line longer than {LINE_BYTES} bytes in a zero-knowledge proof"
        )));
    }
    if cr + 1 >= b.len() {
        return Err(EmailError::Body(format!(
            "the `{label}` line does not end inside the body"
        )));
    }
    let value = std::str::from_utf8(&b[v..end])
        .map_err(|_| EmailError::Body(format!("the `{label}` value is not UTF-8")))?
        .to_string();
    Ok((li, v, value))
}

fn toml_bytes(bytes: &[u8], max: usize) -> String {
    let mut s = String::with_capacity(max * 4);
    s.push('[');
    for i in 0..max {
        if i > 0 {
            s.push(',');
        }
        s += &bytes.get(i).copied().unwrap_or(0).to_string();
    }
    s.push(']');
    s
}

fn toml_fields(fields: &[PublicInput]) -> String {
    let items: Vec<String> = fields
        .iter()
        .map(|f| format!("\"0x{}\"", hex::encode(f)))
        .collect();
    format!("[{}]", items.join(","))
}

/// Checks the message the way `email_dkim` does, then the circuit's own
/// rules, and computes the circuit inputs. Nothing leaves the machine.
pub fn prepare(
    message: &[u8],
    m: &EmailManifest,
    keys: &dyn KeyResolver,
) -> Result<Prepared, EmailError> {
    // Everything the whole-message method refuses is refused here too.
    let verified = email::verify_message(message, &m.signing_domain, keys)?;
    email::extract(m, &verified)?;
    let key = verified.key.clone();
    let sig = verified.verified.signature.clone();
    if sig.algorithm != dkim::Algorithm::RsaSha256 {
        return Err(EmailError::Unsupported(
            "a zero-knowledge proof of an Ed25519 signature (the circuit checks RSA)".into(),
        ));
    }
    if !sig.relaxed_headers() {
        return Err(EmailError::Unsupported(
            "simple header canonicalization in a zero-knowledge proof".into(),
        ));
    }
    let modulus = rsa_key(&key)?;
    let msg: Message = dkim::parse_message(message)?;
    let (h, b) = sig.signed_input(&msg);
    if h.len() > MAX_HEADER {
        return Err(EmailError::Unsupported(format!(
            "a signed header of {} bytes in a zero-knowledge proof (the circuit reads {MAX_HEADER})",
            h.len()
        )));
    }
    if b.len() > MAX_BODY {
        return Err(EmailError::Unsupported(format!(
            "a body of {} bytes in a zero-knowledge proof (the circuit reads {MAX_BODY})",
            b.len()
        )));
    }
    let bad = |what: &str| EmailError::Unsupported(format!("{what} in a zero-knowledge proof"));
    // The DKIM-Signature field: after the last CRLF.
    let ds = h
        .windows(2)
        .rposition(|w| w == b"\r\n")
        .map(|p| p + 2)
        .unwrap_or(0);
    if !h[ds..].starts_with(b"dkim-signature:") {
        return Err(bad(
            "a header input that does not end with the DKIM-Signature",
        ));
    }
    let starts = tag_starts(&h, ds + 15);
    let named = |name: &[u8]| -> Vec<usize> {
        starts
            .iter()
            .copied()
            .filter(|&i| h[i..].starts_with(name))
            .collect()
    };
    if starts.iter().any(|&i| h[i] == b'l') {
        return Err(bad("a signature with an l= tag"));
    }
    let one = |name: &[u8]| -> Result<usize, EmailError> {
        match named(name).as_slice() {
            [i] => Ok(*i),
            _ => Err(bad(&format!(
                "a signature without exactly one {}",
                String::from_utf8_lossy(name)
            ))),
        }
    };
    let d_tag = one(b"d=")?;
    let s_tag = one(b"s=")?;
    let c_tag = one(b"c=")?;
    let bh_tag = one(b"bh=")?;
    let (d_value, domain) = tag_value(&h, d_tag + 1).ok_or_else(|| bad("an empty d="))?;
    let (s_value, selector) = tag_value(&h, s_tag + 1).ok_or_else(|| bad("an empty s="))?;
    let (bh_value, bh) = tag_value(&h, bh_tag + 2).ok_or_else(|| bad("an empty bh="))?;
    if bh_value != bh_tag + 3 || bh.len() != 44 {
        return Err(bad("a bh= the circuit cannot read"));
    }
    let (t_tag, t_len, signed_at) = match named(b"t=").as_slice() {
        [] => (ds, 0, 0),
        [t] => {
            let (v, text) = tag_value(&h, t + 1).ok_or_else(|| bad("an empty t="))?;
            let n: u64 = std::str::from_utf8(text)
                .ok()
                .filter(|s| s.bytes().all(|c| c.is_ascii_digit()) && v == t + 2 && s.len() <= 20)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| bad("a t= the circuit cannot read"))?;
            (*t, text.len(), n)
        }
        _ => return Err(bad("several t= tags")),
    };
    let field_one = |name: &[u8]| -> Result<Option<usize>, EmailError> {
        match find_field(&h, name).as_slice() {
            [] => Ok(None),
            [i] => Ok(Some(*i)),
            _ => Err(bad(&format!(
                "a signed header with several {} fields",
                String::from_utf8_lossy(name)
            ))),
        }
    };
    let ct = field_one(b"content-type:")?
        .ok_or_else(|| bad("a message whose Content-Type is not signed"))?;
    let cte = field_one(b"content-transfer-encoding:")?;
    let from = field_one(b"from:")?.ok_or_else(|| bad("a signature that does not sign From"))?;
    let utf8 = |b: &[u8], what: &str| {
        String::from_utf8(b.to_vec()).map_err(|_| bad(&format!("a {what} that is not UTF-8")))
    };
    let content_type = utf8(
        field_value(&h, ct, 13).ok_or_else(|| bad("a Content-Type"))?,
        "Content-Type",
    )?;
    let transfer_encoding = match cte {
        Some(i) => Some(utf8(
            field_value(&h, i, 26).ok_or_else(|| bad("a Content-Transfer-Encoding"))?,
            "Content-Transfer-Encoding",
        )?),
        None => None,
    };
    check_mime(&content_type, transfer_encoding.as_deref())?;
    let from_value = field_value(&h, from, 5).ok_or_else(|| bad("a From field"))?;
    if from_value.len() + 2 > FROM_BYTES {
        return Err(bad("a From field this long"));
    }
    let aligned = from_aligned(from_value, domain);
    // The body.
    for marker in [
        &b"---------- Forwarded message"[..],
        b"-----Original Message-----",
        b"Begin forwarded message",
    ] {
        if b.windows(marker.len()).any(|w| w == marker) {
            return Err(EmailError::Unsupported(
                "a forwarded or replied-to quote".into(),
            ));
        }
    }
    let lines = lines(&b);
    if lines.len() > MAX_LINES {
        return Err(bad(&format!(
            "a body of {} lines (the circuit reads {MAX_LINES})",
            lines.len()
        )));
    }
    let mut label_line = [0usize; SLOTS];
    let mut value_at = [0usize; SLOTS];
    let mut values = Vec::new();
    if m.fields.len() > SLOTS {
        return Err(bad(&format!("a manifest with more than {SLOTS} fields")));
    }
    for (k, (name, label)) in slots(m).iter().enumerate() {
        let (li, v, value) = labelled_line(&b, &lines, label)?;
        label_line[k] = li;
        value_at[k] = v;
        values.push((name.clone(), value));
    }
    let claims = Claims {
        domain: utf8(domain, "d=")?,
        selector: utf8(selector, "s=")?,
        signed_at,
        content_type,
        transfer_encoding,
        from_aligned: aligned,
        values,
    };
    let public = public_inputs(&modulus, m, &claims).map_err(|e| bad(&e))?;
    // The signature, as 120-bit limbs.
    let sig_limbs = limbs(&BigUint::from_bytes_be(sig.value()));
    // Inputs in the circuit's coordinates: header positions + 2.
    let p = |i: usize| (i + 2).to_string();
    let mut t = String::new();
    t += &format!(
        "header = {{ storage = {}, len = {} }}\n",
        toml_bytes(&h, MAX_HEADER),
        h.len()
    );
    t += &format!(
        "body = {{ storage = {}, len = {} }}\n",
        toml_bytes(&b, MAX_BODY),
        b.len()
    );
    t += &format!("signature = {}\n", toml_fields(&sig_limbs));
    t += &format!("dkim_start = {}\n", p(ds));
    t += &format!("d_tag = {}\nd_value = {}\n", p(d_tag), p(d_value));
    t += &format!("s_tag = {}\ns_value = {}\n", p(s_tag), p(s_value));
    t += &format!("t_tag = {}\nt_len = {t_len}\n", p(t_tag));
    t += &format!("c_tag = {}\nbh_tag = {}\n", p(c_tag), p(bh_tag));
    t += &format!("ct_field = {}\n", p(ct));
    t += &format!("cte_field = {}\n", p(cte.unwrap_or(ct)));
    t += &format!("from_field = {}\n", p(from));
    let list = |a: &[usize]| {
        format!(
            "[{}]",
            a.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let mut starts = vec![b.len(); MAX_LINES];
    let mut texts = vec![b.len(); MAX_LINES];
    for (j, (ls, cs)) in lines.iter().enumerate() {
        starts[j] = *ls;
        texts[j] = *cs;
    }
    t += &format!("n_lines = {}\n", lines.len());
    t += &format!(
        "line_starts = {}\ncontent_starts = {}\n",
        list(&starts),
        list(&texts)
    );
    t += &format!(
        "label_line = {}\nvalue_at = {}\n",
        list(&label_line),
        list(&value_at)
    );
    // Public inputs, by name, from the one encoding the verifier uses.
    let mut it = public.iter().copied();
    let mut take = |n: usize| -> Vec<PublicInput> { (&mut it).take(n).collect() };
    let num = |f: &PublicInput| u64::from_be_bytes(f[24..].try_into().unwrap_or([0; 8]));
    t += &format!("modulus = {}\n", toml_fields(&take(KEY_LIMBS)));
    t += &format!("redc = {}\n", toml_fields(&take(KEY_LIMBS)));
    t += &format!("domain = {}\n", toml_fields(&take(DOMAIN_FIELDS)));
    t += &format!("domain_len = {}\n", num(&take(1)[0]));
    t += &format!("selector = {}\n", toml_fields(&take(SELECTOR_FIELDS)));
    t += &format!("selector_len = {}\n", num(&take(1)[0]));
    t += &format!("sig_time = {}\n", num(&take(1)[0]));
    t += &format!("content_type = {}\n", toml_fields(&take(CT_FIELDS)));
    t += &format!("content_type_len = {}\n", num(&take(1)[0]));
    t += &format!("cte = {}\n", toml_fields(&take(CTE_FIELDS)));
    t += &format!("cte_len = {}\n", num(&take(1)[0]));
    t += &format!("from_aligned = {}\n", num(&take(1)[0]) == 1);
    let labels = take(SLOTS * LABEL_FIELDS);
    let label_lens = take(SLOTS);
    let vals = take(SLOTS * VALUE_FIELDS);
    let value_lens = take(SLOTS);
    let nested = |f: &[PublicInput], w: usize| {
        let rows: Vec<String> = f.chunks(w).map(toml_fields).collect();
        format!("[{}]", rows.join(","))
    };
    let lens = |f: &[PublicInput]| {
        format!(
            "[{}]",
            f.iter()
                .map(|x| num(x).to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    t += &format!("labels = {}\n", nested(&labels, LABEL_FIELDS));
    t += &format!("label_lens = {}\n", lens(&label_lens));
    t += &format!("values = {}\n", nested(&vals, VALUE_FIELDS));
    t += &format!("value_lens = {}\n", lens(&value_lens));
    Ok(Prepared {
        claims,
        key,
        prover_toml: t,
    })
}

/// The two programs a prover runs, at the versions the verification key
/// comes from.
#[derive(Debug, Clone)]
pub struct Tools {
    pub nargo: std::path::PathBuf,
    pub bb: std::path::PathBuf,
}

impl Tools {
    /// Checks both versions (another version makes proofs this circuit's
    /// key does not verify).
    pub fn check(&self) -> Result<(), String> {
        let version = |p: &std::path::Path| -> Result<String, String> {
            let out = std::process::Command::new(p)
                .arg("--version")
                .output()
                .map_err(|e| format!("{} does not run: {e}", p.display()))?;
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let n = version(&self.nargo)?;
        if !n.contains(&format!("nargo version = {NARGO_VERSION}")) {
            return Err(format!(
                "{} is not nargo {NARGO_VERSION} (noirup -v {NARGO_VERSION})",
                self.nargo.display()
            ));
        }
        let b = version(&self.bb)?;
        if b.trim() != BB_VERSION {
            return Err(format!(
                "{} is bb {}, not {BB_VERSION} (bbup -v {BB_VERSION})",
                self.bb.display(),
                b.trim()
            ));
        }
        Ok(())
    }
}

/// Proves `p` with the circuit in `work` (its source is written there; the
/// compiled circuit stays for the next proof). The message's bytes reach
/// only `work`'s `Prover.toml` and witness, both removed afterwards, and
/// the two local programs. Returns the proof.
pub fn prove(p: &Prepared, work: &std::path::Path, tools: &Tools) -> Result<Vec<u8>, String> {
    use std::process::Command;
    let io = |e: std::io::Error| format!("{}: {e}", work.display());
    std::fs::create_dir_all(work.join("src")).map_err(io)?;
    // The inputs and the witness hold the message while they exist.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(work, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    }
    for (name, text) in CIRCUIT_SOURCE {
        let path = work.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*text) {
            std::fs::write(&path, text).map_err(io)?;
        }
    }
    let toml = work.join("Prover.toml");
    let witness = work.join("target").join(format!("{CIRCUIT_NAME}.gz"));
    let secret = |path: &std::path::Path, text: &str| -> Result<(), String> {
        use std::io::Write as _;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(io)
    };
    let run = |cmd: &mut Command, what: &str| -> Result<(), String> {
        let out = cmd
            .output()
            .map_err(|e| format!("{what} does not run: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = err.lines().rev().take(12).collect();
            Err(format!(
                "{what} failed:\n{}",
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            ))
        }
    };
    let result = (|| {
        secret(&toml, &p.prover_toml)?;
        run(
            Command::new(&tools.nargo)
                .arg("execute")
                .arg("--program-dir")
                .arg(work),
            "nargo execute",
        )?;
        let vk = work.join("target").join("vk");
        std::fs::write(&vk, VK).map_err(io)?;
        let out = work.join("target").join("proof");
        run(
            Command::new(&tools.bb)
                .args(["prove", "-t", "evm", "-b"])
                .arg(work.join("target").join(format!("{CIRCUIT_NAME}.json")))
                .arg("-w")
                .arg(&witness)
                .arg("-k")
                .arg(&vk)
                .arg("-o")
                .arg(&out),
            "bb prove",
        )?;
        std::fs::read(out.join("proof")).map_err(io)
    })();
    let _ = std::fs::remove_file(&toml);
    let _ = std::fs::remove_file(&witness);
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bundle::EmailZkBundle;
    use base64::Engine as _;

    /// Signs `message` (CRLF, no DKIM header yet) with a fresh RSA-2048 key
    /// for tests: relaxed/relaxed, `t=` and `h=` as given. Returns the
    /// signed message and the key record to pin. The private key is dropped.
    pub(crate) fn sign_rsa(
        message: &str,
        domain: &str,
        selector: &str,
        h: &str,
        extra: &str,
    ) -> (String, String) {
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        sign_rsa_with(&key, message, domain, selector, h, extra)
    }

    pub(crate) fn sign_rsa_with(
        key: &rsa::RsaPrivateKey,
        message: &str,
        domain: &str,
        selector: &str,
        h: &str,
        extra: &str,
    ) -> (String, String) {
        use rsa::pkcs8::EncodePublicKey;
        use sha2::Digest;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let der = key.to_public_key().to_public_key_der().unwrap();
        let record = format!("v=DKIM1; k=rsa; p={}", b64(der.as_bytes()));
        let msg = dkim::parse_message(message.as_bytes()).unwrap();
        let body = dkim::tests_support::relaxed_body(&msg.body);
        let bh = b64(&sha2::Sha256::digest(&body));
        let unsigned = format!(
            "DKIM-Signature: v=1; a=rsa-sha256; c=relaxed/relaxed; d={domain}; s={selector};{extra} t=1792404000; h={h}; bh={bh}; b=\r\n"
        );
        let with_sig = format!("{unsigned}{message}");
        let m2 = dkim::parse_message(with_sig.as_bytes()).unwrap();
        let input = dkim::tests_support::header_input(&m2, h);
        let digest = sha2::Sha256::digest(&input);
        let sig = key
            .sign(rsa::Pkcs1v15Sign::new::<rsa::sha2::Sha256>(), &digest)
            .unwrap();
        let header = unsigned.replace("b=\r\n", &format!("b={}\r\n", b64(&sig)));
        (format!("{header}{message}"), record)
    }

    pub(crate) const QUOTE: &str = "From: Quotes <quotes@distributor.example>\r\n\
To: Buyer <buyer@example.net>\r\n\
Subject: Quote Q-1042: DDR5 16GB\r\n\
Date: Mon, 19 Oct 2026 10:00:00 +0000\r\n\
Message-ID: <q-1042@distributor.example>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
FIXTURE QUOTE FOR TESTS - NOT MARKET DATA\r\n\
Dear Buyer, thank you for your enquiry of 14 October.\r\n\
Quote number: Q-1042\r\n\
Part number: DDR5-16G-4800\r\n\
Quantity: 1000\r\n\
Unit price: 39.80\r\n\
Currency: USD\r\n\
Quote date: 2026-10-19\r\n\
Payment terms and your account details are attached separately.\r\n\
\r\n\
> Earlier: Unit price: 99.99\r\n";
    pub(crate) const H: &str =
        "from:to:subject:date:message-id:content-type:from:subject:date:content-type";
    pub(crate) const MANIFEST: &str = r#"{"format":"amoeba-email-manifest-v1","name":"Fixture distributor quote","evidenceType":"quotation","signingDomain":"distributor.example","fields":{"price":"Unit price:","currency":"Currency:","product":"Part number:","quantityTier":"Quantity:","recordId":"Quote number:","observedAt":"Quote date:"}}"#;

    pub(crate) fn policy(domain: &str, record: &str) -> crate::policy::Policy {
        let mut p = crate::policy::Policy::development("none", vec![2; 33], vec![]);
        p.dkim_keys.push(PinnedKey {
            domain: domain.into(),
            selector: "q1".into(),
            record: record.into(),
            trust: crate::policy::Trust::Development,
            provenance: "pinned for this test".into(),
        });
        p
    }

    fn manifest() -> EmailManifest {
        EmailManifest::parse(MANIFEST.as_bytes()).unwrap()
    }

    #[test]
    fn prepare_discloses_only_the_labelled_lines() {
        let (signed, record) = sign_rsa(QUOTE, "distributor.example", "q1", H, "");
        let p = prepare(
            signed.as_bytes(),
            &manifest(),
            &policy("distributor.example", &record).dkim_keys,
        )
        .unwrap();
        let c = &p.claims;
        assert_eq!(
            (c.domain.as_str(), c.selector.as_str()),
            ("distributor.example", "q1")
        );
        assert_eq!(c.signed_at, 1_792_404_000);
        assert_eq!(c.content_type, "text/plain; charset=utf-8");
        assert_eq!(c.transfer_encoding, None);
        assert!(c.from_aligned);
        let v = |k: &str| c.values.iter().find(|(n, _)| n == k).unwrap().1.as_str();
        assert_eq!(v("price"), "39.80");
        assert_eq!(v("recordId"), "Q-1042");
        assert_eq!(v("observedAt"), "2026-10-19");
        // The claims are all a bundle carries: no recipient, subject or
        // other line.
        let b = EmailZkBundle {
            circuit: circuit_id(),
            proof: vec![],
            claims: p.claims.clone(),
            manifest: manifest(),
            annotations: vec![],
        };
        let json = b.to_json();
        for hidden in [
            "buyer@example.net",
            "Q-1042: DDR5",
            "Dear Buyer",
            "account details",
            "99.99",
            "Mon, 19 Oct",
        ] {
            assert!(!json.contains(hidden), "{hidden} leaked");
        }
        // The bundle round-trips with its digest.
        let back = EmailZkBundle::parse(json.as_bytes(), 1 << 20).unwrap();
        assert_eq!(back.digest(), b.digest());
        // The circuit inputs carry the message (they stay on the prover's
        // machine).
        assert!(p.prover_toml.contains("header = { storage = ["));
    }

    #[test]
    fn from_alignment_follows_the_address() {
        assert!(from_aligned(
            b"Quotes <quotes@distributor.example>",
            b"distributor.example"
        ));
        assert!(from_aligned(
            b"q@mail.distributor.example",
            b"Distributor.Example"
        ));
        assert!(!from_aligned(
            b"q@notdistributor.example",
            b"distributor.example"
        ));
        assert!(!from_aligned(
            b"\"sales@distributor.example\" <x@evil.example>",
            b"distributor.example"
        ));
        assert!(!from_aligned(b"no address", b"distributor.example"));
    }

    #[test]
    fn the_circuit_refuses_what_email_dkim_refuses() {
        let keys = |r: &str| policy("distributor.example", r).dkim_keys;
        // l=, even when it covers the whole body.
        let body_len =
            dkim::tests_support::relaxed_body(QUOTE.split_once("\r\n\r\n").unwrap().1.as_bytes())
                .len();
        let (signed, record) = sign_rsa(
            QUOTE,
            "distributor.example",
            "q1",
            H,
            &format!(" l={body_len};"),
        );
        let e = prepare(signed.as_bytes(), &manifest(), &keys(&record)).unwrap_err();
        assert!(matches!(e, EmailError::Unsupported(_)), "{e}");
        // A label only in a quoted line.
        let quoted_only = QUOTE.replace("Unit price: 39.80\r\n", "");
        let (signed, record) = sign_rsa(&quoted_only, "distributor.example", "q1", H, "");
        assert!(prepare(signed.as_bytes(), &manifest(), &keys(&record)).is_err());
        // Forwarded text.
        let fwd = QUOTE.replace(
            "FIXTURE QUOTE",
            "---------- Forwarded message ----------\r\nFIXTURE QUOTE",
        );
        let (signed, record) = sign_rsa(&fwd, "distributor.example", "q1", H, "");
        assert!(prepare(signed.as_bytes(), &manifest(), &keys(&record)).is_err());
        // Content-Type not signed.
        let (signed, record) = sign_rsa(
            QUOTE,
            "distributor.example",
            "q1",
            "from:to:subject:date",
            "",
        );
        assert!(prepare(signed.as_bytes(), &manifest(), &keys(&record)).is_err());
        // Two lines starting with a label, even one indented with a
        // non-breaking space (`email_dkim` trims it too).
        for second in ["Unit price: 41.00\r\n", "\u{a0}Unit price: 41.00\r\n"] {
            let twice = QUOTE.replace("Currency: USD\r\n", &format!("Currency: USD\r\n{second}"));
            let (signed, record) = sign_rsa(&twice, "distributor.example", "q1", H, "");
            assert!(
                prepare(signed.as_bytes(), &manifest(), &keys(&record)).is_err(),
                "{second:?}"
            );
        }
        // A mention inside another line is not a line of its own.
        let mention = QUOTE.replace("Dear Buyer,", "Dear Buyer, the Unit price: below is firm,");
        let (signed, record) = sign_rsa(&mention, "distributor.example", "q1", H, "");
        prepare(signed.as_bytes(), &manifest(), &keys(&record)).unwrap();
        // Several text parts / multipart: not this circuit.
        let multi = QUOTE.replace(
            "Content-Type: text/plain; charset=utf-8\r\n\r\n",
            "Content-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n",
        ) + "--b--\r\n";
        let (signed, record) = sign_rsa(&multi, "distributor.example", "q1", H, "");
        assert!(prepare(signed.as_bytes(), &manifest(), &keys(&record)).is_err());
        // A key the policy does not pin.
        let (signed, _) = sign_rsa(QUOTE, "distributor.example", "q1", H, "");
        let (_, other) = sign_rsa(QUOTE, "distributor.example", "q1", H, "");
        assert!(prepare(signed.as_bytes(), &manifest(), &keys(&other)).is_err());
    }

    fn data(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
    }

    pub(crate) const REISSUED: (&str, &str) = (
        "<q-1042@distributor.example>",
        "<q-1042-resend@distributor.example>",
    );

    /// Proves the fixture quote and a reissue of it with nargo and bb, checks
    /// both with the verifier, prints the timings and writes the fixtures the
    /// next test reads. Needs nargo 1.0.0-beta.18 and bb 3.0.3
    /// (`EVIDENCE_NARGO`, `EVIDENCE_BB`, else on PATH). A fresh key each run:
    /// its private half is never written.
    #[test]
    #[ignore]
    fn write_zk_fixtures() {
        let tools = Tools {
            nargo: std::env::var_os("EVIDENCE_NARGO")
                .unwrap_or("nargo".into())
                .into(),
            bb: std::env::var_os("EVIDENCE_BB")
                .unwrap_or("bb".into())
                .into(),
        };
        tools.check().unwrap();
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        // The reissue also signs a 7bit transfer encoding and comes from a
        // reseller's address (both outside the observation).
        let reissued = QUOTE
            .replace(REISSUED.0, REISSUED.1)
            .replace("10:00:00", "16:30:00")
            .replace("quotes@distributor.example", "desk@reseller.example")
            .replace(
                "Content-Type: text/plain; charset=utf-8\r\n",
                "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n",
            );
        let h_reissued = format!("{H}:content-transfer-encoding");
        let work = std::env::temp_dir().join("amoeba-email-dkim-zk-fixtures");
        let mut record = String::new();
        for (name, text, h) in [
            ("zk-quote", QUOTE.to_string(), H.to_string()),
            ("zk-quote-reissued", reissued, h_reissued),
        ] {
            let (signed, r) = sign_rsa_with(&key, &text, "distributor.example", "q1", &h, "");
            record = r;
            let pol = policy("distributor.example", &record);
            let t0 = std::time::Instant::now();
            let p = prepare(signed.as_bytes(), &manifest(), &pol.dkim_keys).unwrap();
            let t1 = std::time::Instant::now();
            let proof = prove(&p, &work, &tools).unwrap();
            let t2 = std::time::Instant::now();
            let b = EmailZkBundle {
                circuit: circuit_id(),
                proof,
                claims: p.claims,
                manifest: manifest(),
                annotations: vec![],
            };
            let json = b.to_json();
            let v = crate::verify_bundle(json.as_bytes(), &pol);
            let t3 = std::time::Instant::now();
            assert!(v.authenticated(), "{:?}", v.stages);
            // The proof check alone, repeated.
            let n = rsa_key(&pol.dkim_keys[0]).unwrap();
            let public = public_inputs(&n, &manifest(), &b.claims).unwrap();
            let mut checks: Vec<std::time::Duration> = (0..20)
                .map(|_| {
                    let s = std::time::Instant::now();
                    verify_proof(&b.proof, &public).unwrap();
                    s.elapsed()
                })
                .collect();
            checks.sort();
            eprintln!(
                "{name}: prepare {:?}, prove {:?}, verify_bundle {:?}, proof check p50 {:?} max {:?} (20 runs), proof {} bytes, bundle {} bytes",
                t1 - t0,
                t2 - t1,
                t3 - t2,
                checks[10],
                checks[19],
                b.proof.len(),
                json.len()
            );
            std::fs::write(data(&format!("{name}.bundle.json")), json).unwrap();
            if name == "zk-quote" {
                std::fs::write(data("zk-quote.eml"), signed).unwrap();
            }
        }
        std::fs::write(data("zk-key.txt"), format!("{record}\n")).unwrap();
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(data(name)).unwrap()
    }

    fn edit(bundle: &str, f: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v: serde_json::Value = serde_json::from_str(bundle).unwrap();
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn proven_quotes_verify_offline_and_count_once() {
        let record = fixture("zk-key.txt").trim().to_string();
        let pol = policy("distributor.example", &record);
        let a = fixture("zk-quote.bundle.json");
        let mut va = crate::verify_bundle(a.as_bytes(), &pol);
        assert!(va.authenticated(), "{:?}", va.stages);
        let env = va.envelope.clone().unwrap();
        assert_eq!(
            env.authentication.method,
            crate::bundle::Method::EmailDkimZk
        );
        assert_eq!(env.authentication.backend_revision, circuit_id());
        assert_eq!(env.observation.price.atoms, 39_800_000);
        assert_eq!(
            env.observation.evidence_type,
            crate::manifest::EvidenceType::Quotation
        );
        assert_eq!(env.observation.fields["recordId"].value, "Q-1042");
        assert_eq!(
            env.observation.context["From is in the signing domain"],
            "true"
        );
        assert_eq!(env.times.witnessed_at.value, "1792404000");
        // The disclosure is the claims: no recipient, subject or other line.
        let shown = serde_json::to_string(&va.disclosure).unwrap();
        for hidden in [
            "buyer@example.net",
            "Dear Buyer",
            "account details",
            "99.99",
            "Mon, 19 Oct",
        ] {
            assert!(
                !shown.contains(hidden) && !a.contains(hidden),
                "{hidden} leaked"
            );
        }
        // A reissued quote is the same observation; so is the same message
        // through the whole-message method.
        let vb = crate::verify_bundle(fixture("zk-quote-reissued.bundle.json").as_bytes(), &pol);
        let eb = vb.envelope.unwrap();
        assert_eq!(
            eb.observation.context["From is in the signing domain"],
            "false"
        );
        assert!(
            serde_json::to_string(&vb.disclosure)
                .unwrap()
                .contains("7bit")
        );
        assert_ne!(
            env.authentication.bundle_digest,
            eb.authentication.bundle_digest
        );
        assert_eq!(env.observation_id, eb.observation_id);
        let whole = crate::bundle::EmailBundle {
            message: fixture("zk-quote.eml").into_bytes(),
            manifest: manifest(),
            annotations: vec![],
        };
        let vw = crate::verify_bundle(whole.to_json().as_bytes(), &pol);
        assert_eq!(vw.envelope.unwrap().observation_id, env.observation_id);
        // A quote is still a quotation: not a source under the rules.
        crate::check_destination(
            &mut va,
            &crate::Destination::Candidate {
                market: "ramx".into(),
                expiry: "RAMX-202612".into(),
                bucket: "b".into(),
                canonical_url: env.origin.locator.clone(),
                submitter: None,
            },
        );
        assert_eq!(va.stages.eligibility.outcome, crate::Outcome::Failed);
    }

    #[test]
    fn a_proof_holds_only_for_its_claims_key_and_circuit() {
        let record = fixture("zk-key.txt").trim().to_string();
        let pol = policy("distributor.example", &record);
        let a = fixture("zk-quote.bundle.json");
        let prov = |bundle: &str, p: &crate::policy::Policy| {
            crate::verify_bundle(bundle.as_bytes(), p)
                .stages
                .provenance
                .outcome
        };
        use crate::Outcome::{Failed, Unsupported};
        for change in [
            edit(&a, |v| v["claims"]["values"]["price"] = "29.80".into()),
            edit(&a, |v| v["claims"]["values"]["recordId"] = "Q-1043".into()),
            edit(&a, |v| v["claims"]["fromAligned"] = false.into()),
            edit(&a, |v| v["claims"]["signedAt"] = 1792404001.into()),
            edit(&a, |v| v["claims"]["contentType"] = "text/plain".into()),
            edit(&a, |v| v["claims"]["transferEncoding"] = "7bit".into()),
            // The manifest reads another line under the same name.
            edit(&a, |v| {
                v["manifest"]["fields"]["price"] = "Unit  price:".into()
            }),
            edit(&a, |v| {
                let p = v["proof"].as_str().unwrap().to_string();
                let mut raw = base64::engine::general_purpose::STANDARD.decode(p).unwrap();
                raw[100] ^= 1;
                v["proof"] = base64::engine::general_purpose::STANDARD.encode(raw).into();
            }),
        ] {
            assert_eq!(prov(&change, &pol), Failed, "{change}");
        }
        // Another domain or selector: no key, or the manifest's domain differs.
        assert_eq!(
            prov(&edit(&a, |v| v["claims"]["selector"] = "q2".into()), &pol),
            Failed
        );
        assert_eq!(
            prov(
                &edit(&a, |v| v["claims"]["domain"] = "other.example".into()),
                &pol
            ),
            Failed
        );
        // The verifier's key, never the prover's: another key pinned fails.
        let (_, other) = sign_rsa(QUOTE, "distributor.example", "q1", H, "");
        assert_eq!(prov(&a, &policy("distributor.example", &other)), Failed);
        // An Ed25519 key pinned for the name: not this circuit.
        let (_, ed) = crate::email::tests::sign(QUOTE, "distributor.example", "q1", H, None);
        assert_eq!(prov(&a, &policy("distributor.example", &ed)), Unsupported);
        // Another circuit.
        assert_eq!(
            prov(
                &edit(&a, |v| v["backend"]["revision"] = "00".repeat(32).into()),
                &pol
            ),
            Unsupported
        );
        // Smuggled keys are refused before anything else.
        let smuggled = edit(&a, |v| v["claims"]["verified"] = true.into());
        assert_eq!(
            crate::verify_bundle(smuggled.as_bytes(), &pol)
                .stages
                .received
                .outcome,
            Failed
        );
        // Slot order follows the field names, not how the manifest is written.
        let reordered = EmailManifest::parse(br#"{"format":"amoeba-email-manifest-v1","name":"Fixture distributor quote","evidenceType":"quotation","signingDomain":"distributor.example","fields":{"observedAt":"Quote date:","recordId":"Quote number:","quantityTier":"Quantity:","product":"Part number:","currency":"Currency:","price":"Unit price:"}}"#).unwrap();
        let b = crate::bundle::EmailZkBundle::parse(a.as_bytes(), 1 << 20).unwrap();
        let n = rsa_key(&pol.dkim_keys[0]).unwrap();
        assert_eq!(
            public_inputs(&n, &reordered, &b.claims),
            public_inputs(&n, &manifest(), &b.claims)
        );
    }

    /// Writes the committed fixture quote's circuit inputs to
    /// `$EVIDENCE_ZK_INPUTS/Prover.toml`, to time `nargo execute` and `bb
    /// prove` by hand (MEASUREMENTS.md). Fixture data only.
    #[test]
    #[ignore]
    fn zk_inputs_for_measurement() {
        let dir = std::path::PathBuf::from(std::env::var_os("EVIDENCE_ZK_INPUTS").unwrap());
        let record = fixture("zk-key.txt").trim().to_string();
        let pol = policy("distributor.example", &record);
        let p = prepare(
            fixture("zk-quote.eml").as_bytes(),
            &manifest(),
            &pol.dkim_keys,
        )
        .unwrap();
        std::fs::write(dir.join("Prover.toml"), p.prover_toml).unwrap();
    }
}
