//! The preview of a proof, in plain words, for `petri evidence show/prove`
//! and the TUI's Evidence view: what it shows, what it discloses, how far
//! it got. Local checks never read as the oracle's acceptance: that line
//! comes only from what was submitted and what the chain or the API said.

use amoeba_evidence::{
    Outcome, Stage, Verification, bundle::Method, extract::Provenance, policy::Trust,
    tlsn::ServerTrust,
};

use super::store::Entry;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub title: String,
    /// Label → value, in display order.
    pub rows: Vec<(String, String)>,
    /// One line per stage, and whether it passed.
    pub steps: Vec<(String, bool)>,
    /// What the proof discloses.
    pub disclosed: Vec<String>,
    /// What a reader has to trust.
    pub trust: String,
}

fn when(unix: &str) -> String {
    unix.parse::<i64>()
        .map(crate::futures::view::utc)
        .unwrap_or_else(|_| unix.to_string())
}

fn step(stage: &Stage, ok: &str, failed: &str) -> (String, bool) {
    match stage.outcome {
        Outcome::Passed => (ok.to_string(), true),
        Outcome::NotChecked => (format!("{failed}: not checked"), false),
        Outcome::Unsupported => (
            format!(
                "{failed}: not supported ({})",
                stage.reason.as_deref().unwrap_or("")
            ),
            false,
        ),
        Outcome::Failed => (
            format!("{failed}: {}", stage.reason.as_deref().unwrap_or("failed")),
            false,
        ),
    }
}

pub fn preview(v: &Verification, entry: Option<&Entry>) -> Preview {
    let mut p = Preview::default();
    let s = &v.stages;
    p.steps.push(step(&s.received, "Bundle read", "Bundle"));
    let trust_word = v.envelope.as_ref().map(|e| match e.authentication.trust {
        Trust::Production => "production trust",
        Trust::Development => "development trust, not evidence on production",
    });
    p.steps.push(step(
        &s.provenance,
        &format!("Proof verified here ({})", trust_word.unwrap_or("")),
        "Proof",
    ));
    p.steps.push(step(
        &s.extraction,
        "Price read from the proven record",
        "Reading the record",
    ));
    if s.eligibility.outcome != Outcome::NotChecked {
        p.steps.push(step(
            &s.eligibility,
            "Fits where it was submitted",
            "Where it goes",
        ));
    }
    let submitted = entry
        .map(|e| e.events.iter().filter(|ev| ev.kind != "uploaded").count())
        .unwrap_or(0);
    let uploaded = entry.is_some_and(|e| e.events.iter().any(|ev| ev.kind == "uploaded"));
    p.steps.push((
        match (uploaded, submitted) {
            (_, n) if n > 0 => {
                "Submitted: the oracle decides on chain (see `petri oracle`)".to_string()
            }
            (true, _) => "Uploaded, not submitted to the oracle yet".to_string(),
            _ => "Not submitted: the oracle has not seen it".to_string(),
        },
        false,
    ));

    let Some(env) = &v.envelope else {
        p.title = "Web proof".into();
        if let Some(d) = &v.disclosure {
            p.rows.push(("Source".into(), d.server_name.clone()));
            p.disclosed.push(format!(
                "Request: {}",
                d.request.lines().next().unwrap_or("")
            ));
        }
        if let Some(digest) = &v.bundle_digest {
            p.rows.push(("Bundle".into(), digest.clone()));
        }
        return p;
    };
    let o = &env.observation;
    let field = |k: &str| o.fields.get(k).map(|f| f.value.clone());
    let method = env.authentication.method;
    let email = matches!(method, Method::EmailDkim | Method::EmailDkimZk);
    p.title = match method {
        Method::WebProof => format!(
            "Web proof  {}",
            env.origin.locator.trim_start_matches("https://")
        ),
        Method::EmailDkim => format!("Email quote  {}", env.origin.server_name),
        Method::EmailDkimZk => format!(
            "Email quote, zero-knowledge proof  {}",
            env.origin.server_name
        ),
    };
    let mut price = format!("{}", o.price.text);
    if let Some(c) = field("currency") {
        price.push_str(&format!(" {c}"));
    }
    if let Some(u) = field("unit") {
        price.push_str(&format!(" per {u}"));
    }
    let mut extra = Vec::new();
    if let Some(t) = field("priceType") {
        extra.push(t);
    }
    if let Some(q) = field("quantityTier") {
        extra.push(format!("min qty {q}"));
    }
    if !extra.is_empty() {
        price.push_str(&format!("  ({})", extra.join(", ")));
    }
    let assumed: Vec<String> = o
        .fields
        .iter()
        .filter(|(_, f)| f.provenance == Provenance::ContributorAnnotation)
        .map(|(k, f)| format!("{k} {}", f.value))
        .collect();
    let mut product = field("product").unwrap_or_default();
    if let Some(n) = field("productName") {
        product = if product.is_empty() {
            n
        } else {
            format!("{product}  {n}")
        };
    }
    let mut record = env.extraction.record_pointer.clone();
    if let Some(id) = field("recordId") {
        record.push_str(&format!("  id {id}"));
    }
    if let Some(rev) = field("revision") {
        record.push_str(&format!(", revision {rev}"));
    }
    let rows = &mut p.rows;
    rows.push(("Source".into(), env.origin.locator.clone()));
    rows.push(("Product".into(), product));
    rows.push(("Price".into(), price));
    if !assumed.is_empty() {
        rows.push((
            "You stated".into(),
            format!("{} (not proven)", assumed.join(", ")),
        ));
    }
    rows.push(("Kind".into(), o.evidence_type.as_str().replace('_', " ")));
    rows.push(("Record".into(), record));
    for (k, v) in &o.context {
        rows.push((
            "Context".into(),
            format!("{} {v}", k.trim_start_matches('/')),
        ));
    }
    if let Some(t) = &env.times.source_observed_at {
        rows.push((
            "Source date".into(),
            format!("{} (as the source states it)", t.value),
        ));
    }
    let seen = &env.times.witnessed_at.value;
    rows.push((
        "Seen".into(),
        match (email, seen.is_empty()) {
            (true, true) => "not stated (the signature has no time)".to_string(),
            (true, false) => format!("{} (the signer's own signature time)", when(seen)),
            _ => format!("{} (the notary's session time)", when(seen)),
        },
    ));
    rows.push((
        "Proof".into(),
        if method == Method::EmailDkimZk {
            format!(
                "zero-knowledge (UltraHonk, circuit {}), key {}",
                &env.authentication.backend_revision
                    [..12.min(env.authentication.backend_revision.len())],
                env.authentication.notary
            )
        } else if email {
            format!(
                "DKIM {}, key {}",
                env.authentication.mode, env.authentication.notary
            )
        } else {
            format!(
                "TLSNotary {}, notary {}",
                env.authentication.mode.to_uppercase(),
                env.authentication.notary
            )
        },
    ));
    if let Some(b) = &env.authentication.bound_to {
        rows.push(("Bound to".into(), format!("wallet {}", bs58_of_hex(b))));
    }
    rows.push(("Bundle".into(), env.authentication.bundle_digest.clone()));
    rows.push(("Observation".into(), env.observation_id.clone()));
    if !env.annotations.is_empty() {
        let notes: Vec<String> = env
            .annotations
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        rows.push(("Your notes".into(), notes.join("; ")));
    }
    match (&v.disclosure, method) {
        (Some(d), Method::EmailDkimZk) => {
            for h in &d.response_headers {
                p.disclosed
                    .push(format!("{}: {}", h.name, h.value.as_deref().unwrap_or("")));
            }
            for line in d.body.lines() {
                p.disclosed.push(format!("The line \"{line}\""));
            }
            p.disclosed.push(
                "Nothing else: the recipient, the subject, the other lines, the From address and the signature stay with you"
                    .into(),
            );
        }
        (Some(d), Method::EmailDkim) => {
            p.disclosed.push(format!(
                "The whole message: {} bytes, every header and line",
                d.body.len()
            ));
        }
        (Some(d), Method::WebProof) => {
            p.disclosed.push(format!(
                "Request: {}",
                d.request.lines().next().unwrap_or("")
            ));
            p.disclosed.push(format!("Host: {}", d.server_name));
            if !d.hidden_request_headers.is_empty() {
                p.disclosed.push(format!(
                    "Hidden: {} (names only)",
                    d.hidden_request_headers.join(", ")
                ));
            }
            p.disclosed.push(format!(
                "The whole response: {} bytes, every header but Set-Cookie values",
                d.body.len()
            ));
        }
        (None, _) => {}
    }
    let server = match env.authentication.server_trust {
        ServerTrust::MozillaRoots => "Mozilla's root certificates",
        ServerTrust::DevelopmentRoot => "a development root certificate",
        ServerTrust::DkimKeyPolicy => "the DKIM key the verifier's policy names",
    };
    p.trust = if email {
        format!(
            "Readers trust {server}. It shows that {} signed this message, not who wrote it, when it arrived or that the quote is a market price.{}",
            env.origin.server_name,
            if method == Method::EmailDkimZk {
                " The proof shows the lines above without the rest of the message."
            } else {
                ""
            }
        )
    } else {
        format!(
            "Readers trust the notary's key ({}), {server} and the notary's clock. The proof shows what {} sent, not that it is a market price.",
            env.authentication.notary, env.origin.server_name
        )
    };
    p
}

fn bs58_of_hex(hex: &str) -> String {
    let mut bytes = [0u8; 32];
    match hex::decode_to_slice(hex, &mut bytes) {
        Ok(()) => solana_program::pubkey::Pubkey::new_from_array(bytes).to_string(),
        Err(_) => hex.to_string(),
    }
}

pub fn plain(p: &Preview) -> String {
    let mut out = format!("{}\n", p.title);
    for (label, value) in &p.rows {
        out.push_str(&format!("  {label:<12} {value}\n"));
    }
    out.push_str("\n  Steps\n");
    for (text, ok) in &p.steps {
        out.push_str(&format!("    {} {text}\n", if *ok { "✓" } else { "·" }));
    }
    if !p.disclosed.is_empty() {
        out.push_str("\n  Discloses\n");
        for d in &p.disclosed {
            out.push_str(&format!("    {d}\n"));
        }
    }
    if !p.trust.is_empty() {
        out.push_str(&format!("\n  {}\n", p.trust));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../evidence/crates/evidence/tests/data/"
    );

    #[test]
    fn a_zero_knowledge_email_proof_lists_exactly_what_it_discloses() {
        let read = |f: &str| std::fs::read_to_string(format!("{DATA}{f}")).unwrap();
        let mut policy = amoeba_evidence::policy::Policy::dev();
        policy.dkim_keys.push(amoeba_evidence::email::PinnedKey {
            domain: "distributor.example".into(),
            selector: "q1".into(),
            record: read("zk-key.txt").trim().into(),
            trust: Trust::Development,
            provenance: "the evidence fixture's test key".into(),
        });
        let v = amoeba_evidence::verify_bundle(read("zk-quote.bundle.json").as_bytes(), &policy);
        assert!(v.authenticated(), "{:?}", v.stages);
        let p = preview(&v, None);
        assert!(p.title.contains("zero-knowledge"));
        let shown = p.disclosed.join("\n");
        for line in [
            "DKIM d=: distributor.example",
            "signed Content-Type: text/plain; charset=utf-8",
            "From is in the signing domain: true",
            "The line \"Unit price: 39.80\"",
            "Nothing else",
        ] {
            assert!(shown.contains(line), "{line} missing from:\n{shown}");
        }
        // Nothing of a web proof, and nothing from the hidden part.
        for absent in ["Request:", "whole", "buyer@example.net", "Dear Buyer"] {
            assert!(!shown.contains(absent), "{absent} in:\n{shown}");
        }
    }
}
