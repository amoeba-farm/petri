//! Sending proofs to the futures API: the upload (`POST /oracle/evidence`,
//! which verifies the bundle itself and keeps it for anyone to re-check)
//! and a source candidate with the proof attached
//! (`POST /oracle/source-candidates` with `authentication.bundleDigest`).
//!
//! Candidates are submitted as you: Petri registers its own candidate
//! submitter once per API, with your wallet as the payout authority, and
//! keeps the credential in a private file (mode 0600, never printed).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::store;
use crate::futures::{
    ctx::Ctx,
    error::{Error, Result},
};
use crate::petri_config;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Submitter {
    api: String,
    agent_id: String,
    payout_authority: String,
    credential: String,
}

/// The oracle routes answer `{ ok: true, data }`; the data, either way.
fn data(reply: Value) -> Value {
    match reply {
        Value::Object(mut m)
            if m.get("ok") == Some(&Value::Bool(true)) && m.contains_key("data") =>
        {
            m.remove("data").unwrap_or(Value::Null)
        }
        other => other,
    }
}

fn submitters_path(dir: &Path) -> std::path::PathBuf {
    dir.join("submitters.json")
}

/// Petri's own candidate submitter at this API for this wallet, registered
/// on first use.
fn submitter(ctx: &Ctx, dir: &Path) -> Result<Submitter> {
    let wallet = ctx.wallet_pubkey()?.to_string();
    let path = submitters_path(dir);
    let mut all: Vec<Submitter> = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    if let Some(s) = all
        .iter()
        .find(|s| s.api == ctx.api.base() && s.payout_authority == wallet)
    {
        return Ok(s.clone());
    }
    let reply = data(
        ctx.api
            .post("/oracle/agents", &json!({ "payoutAuthority": wallet }))?,
    );
    let agent_id = reply["agent"]["agentId"]
        .as_str()
        .ok_or_else(|| Error::msg("the API registered no submitter"))?
        .to_string();
    let credential = reply["credential"]
        .as_str()
        .ok_or_else(|| Error::msg("the API returned no submitter credential"))?
        .to_string();
    let s = Submitter {
        api: ctx.api.base().to_string(),
        agent_id,
        payout_authority: wallet,
        credential,
    };
    all.push(s.clone());
    petri_config::create_private_dir_all(dir).map_err(|e| Error::msg(e.to_string()))?;
    petri_config::write_private_file(&path, &serde_json::to_vec_pretty(&all).expect("encodes"))
        .map_err(Error::msg)?;
    Ok(s)
}

/// Uploads the bundle bytes as they are (a JSON library could re-print the
/// manifest's numbers and change the digest).
pub fn upload(ctx: &Ctx, dir: &Path, digest: &str, json_text: &str) -> Result<Value> {
    let reply = data(
        ctx.api
            .post_bytes("/oracle/evidence", json_text.as_bytes())?,
    );
    let summary = if reply["duplicate"].as_bool() == Some(true) {
        "uploaded (the API already had it)"
    } else {
        "uploaded; the API verified and stored it"
    };
    store::record_in(
        dir,
        digest,
        store::Event {
            kind: "uploaded".into(),
            at: store::now(),
            summary: summary.into(),
            detail: json!({
                "api": ctx.api.base(),
                "bundleDigest": reply["bundleDigest"],
                "observationId": reply["observationId"],
                "trust": reply["trust"],
            }),
        },
    )?;
    Ok(reply)
}

/// The ten detail fields a candidate names, and with them its comparison
/// `series` (the API refuses others).
const CANDIDATE_FIELDS: &[&str] = &[
    "marketId",
    "expiryId",
    "bucketId",
    "canonicalUrl",
    "sourceType",
    "productDefinition",
    "priceField",
    "region",
    "currency",
    "evidence",
];

/// What a series candidate must name itself; the API derives the rest of
/// its details from the series (a field given anyway must agree).
const SERIES_FIELDS: &[&str] = &["marketId", "expiryId", "evidence"];

/// The fields of `candidate` the API takes: a research-workspace candidate
/// may carry its own bookkeeping, and one without a series (`null` there
/// too) names all ten details.
fn submission(
    mut candidate: serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>> {
    if candidate.get("series") == Some(&Value::Null) {
        candidate.remove("series");
    }
    candidate.retain(|k, _| k == "series" || CANDIDATE_FIELDS.contains(&k.as_str()));
    let required = if candidate.contains_key("series") {
        SERIES_FIELDS
    } else {
        CANDIDATE_FIELDS
    };
    if let Some(missing) = required.iter().find(|f| !candidate.contains_key(**f)) {
        return Err(Error::msg(format!("the candidate has no `{missing}`")));
    }
    Ok(candidate)
}

pub fn candidate(ctx: &Ctx, dir: &Path, digest: &str, file: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", file.display())))?;
    let candidate: serde_json::Map<String, Value> = serde_json::from_str(&text).map_err(|e| {
        Error::msg(format!(
            "{} is not a candidate JSON object: {e}",
            file.display()
        ))
    })?;
    candidate_map(ctx, dir, digest, candidate)
}

/// Submits `candidate` (its ten detail fields, or its series and the
/// fields the series doesn't give) with the proof attached.
pub fn candidate_map(
    ctx: &Ctx,
    dir: &Path,
    digest: &str,
    candidate: serde_json::Map<String, Value>,
) -> Result<Value> {
    let mut candidate = submission(candidate)?;
    candidate.insert("authentication".into(), json!({ "bundleDigest": digest }));
    let who = submitter(ctx, dir)?;
    let reply = data(ctx.api.post_bearer(
        "/oracle/source-candidates",
        &Value::Object(candidate),
        &who.credential,
    )?);
    let duplicate = reply["duplicate"].as_bool() == Some(true);
    store::record_in(
        dir,
        digest,
        store::Event {
            kind: "candidate".into(),
            at: store::now(),
            summary: if duplicate {
                "submitted; someone submitted this observation for this slot first".into()
            } else {
                "submitted as a source candidate".into()
            },
            detail: json!({
                "api": ctx.api.base(),
                "duplicate": duplicate,
                "contentHash": reply["candidate"]["contentHash"],
                "authentication": reply["candidate"]["authentication"],
            }),
        },
    )?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_oracle_envelopes() {
        assert_eq!(data(json!({"ok": true, "data": {"a": 1}})), json!({"a": 1}));
        assert_eq!(data(json!({"a": 1})), json!({"a": 1}));
    }

    fn map(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_candidate_keeps_its_details_and_its_series() {
        let mut classic = json!({"workId": "w1", "series": null});
        for f in CANDIDATE_FIELDS {
            classic[*f] = json!(format!("{f} text"));
        }
        // A classic candidate: its ten details, without the bookkeeping.
        let sent = submission(map(classic.clone())).unwrap();
        assert_eq!(sent.len(), 10);
        assert!(!sent.contains_key("workId") && !sent.contains_key("series"));
        let mut short = map(classic.clone());
        short.remove("region");
        assert_eq!(
            submission(short).unwrap_err().message,
            "the candidate has no `region`"
        );
        // A series candidate: the series goes too, and the API derives the
        // details it leaves out.
        let series = json!({"seriesId": "s1", "bucket": "DDR5_16GBIT"});
        let found = json!({
            "marketId": "ramx", "expiryId": "RAMX-202701", "evidence": "e",
            "series": series, "contentHash": "aa",
        });
        let sent = submission(map(found.clone())).unwrap();
        assert_eq!(
            Value::Object(sent),
            json!({"marketId": "ramx", "expiryId": "RAMX-202701", "evidence": "e", "series": series})
        );
        let mut details = classic;
        details["series"] = series.clone();
        assert_eq!(submission(map(details)).unwrap().len(), 11);
        let mut no_evidence = map(found);
        no_evidence.remove("evidence");
        assert_eq!(
            submission(no_evidence).unwrap_err().message,
            "the candidate has no `evidence`"
        );
    }
}
