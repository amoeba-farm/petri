//! Observations of a chip month's comparison series: one published price, its
//! period and its evidence (`observation_fields` of the commodity spec). On
//! chain an observation is one print of its source: the opening print in the
//! month's Opening, a sealed Game update after it (value = `price_atoms` of
//! its `rawPrice`). Its `evidenceReference` is what the print cites, as
//! `prints opening` and `updates commit` take it: a Wayback archive URL, or a
//! web proof (`petri evidence`: its bundle digest, its bundle file, or its
//! `amoeba-evidence-v1;…` record), cited as its evidence record (kind 4) and
//! published with the print. The metadata the
//! chain does not hold goes to the futures API's observation records
//! (`POST /oracle/observations`) once the print is on chain; an update's after
//! its reveal. The record is a convenience: when the API cannot take it, the
//! print still stands and `observe record` stores it later.

use std::path::Path;

use ameba_commodity_spec::{
    self as spec, Observation, ObservationContext, PriorObservation, Routing, SeriesDefinition,
    Stage,
};
use ameba_oracle::{instruction as ix, state::OraclePhase};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    chain::{MonthRef, Oracle, SourceRef},
    claims::{self, PendingObservation},
    send,
    series::{self, Parsed},
    skus::hex,
    writes::{self, month_json, usdc_amount},
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::trim_decimal,
    view::Paint,
};

/// The month and source an observation is of.
struct Of<'a> {
    oracle: Oracle<'a>,
    month: MonthRef,
    source: SourceRef,
    series: SeriesDefinition,
    product: ameba_commodity_spec::Product,
}

fn of<'a>(ctx: &'a Ctx, market: &str, expiry: Option<&str>, source: &str) -> Result<Of<'a>> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(market, expiry)?;
    let product = series::chip_product(&oracle, &month)?;
    let source = oracle.source(&month, source)?;
    let text = oracle
        .definitions(std::slice::from_ref(&source))?
        .pop()
        .flatten();
    let series = match Parsed::of(text.as_deref()) {
        Parsed::Series(series) => *series,
        Parsed::Malformed(error) => bail(format!(
            "source {} is not a valid comparison series: {error}",
            source.address
        ))?,
        Parsed::FreeText => bail(format!(
            "source {} is not a comparison series: its definition is free text",
            source.address
        ))?,
        Parsed::Missing => bail(format!(
            "source {}'s definition is not on chain, so it has no series to observe",
            source.address
        ))?,
    };
    Ok(Of {
        oracle,
        month,
        source,
        series,
        product,
    })
}

/// `YYYY-MM-DD` of a unix time.
fn date(ts: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(ts).unwrap_or(0), 0)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string()
}

/// The source's observation records the API holds, if it can be read.
fn records(ctx: &Ctx, month: &MonthRef, source: &SourceRef) -> Result<Vec<Value>> {
    let path = crate::endpoints::oracle_observations(
        &month.period.expiry_id(),
        &hex(&source.state.source_id),
    );
    let data = super::reads::document(ctx, &path)?;
    Ok(data["items"].as_array().cloned().unwrap_or_default())
}

/// Release ids and period ends of earlier records.
fn priors(items: &[Value]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|item| {
            let o = &item["observation"];
            (
                o["releaseId"].as_str().unwrap_or_default().to_string(),
                o["referencePeriodEnd"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .filter(|(release, _)| !release.is_empty())
        .collect()
}

/// Routes `observation` of `of`'s series as the oracle month stands.
fn route(of: &Of, observation: &Observation, prior: &[(String, String)]) -> Routing {
    let prior: Vec<PriorObservation> = prior
        .iter()
        .map(|(release_id, end)| PriorObservation {
            release_id,
            reference_period_end: end,
        })
        .collect();
    let expiry = date(of.month.period.period.expiry_ts);
    spec::route_observation(
        &of.series,
        observation,
        &ObservationContext {
            product: of.product,
            committed: series::stage(of.month.state.phase) == Stage::Committed
                && series::in_recipe(of.source.state.status),
            prior: &prior,
            expiry: Some(&expiry),
        },
    )
}

fn read_observation(path: &Path) -> Result<Observation> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display())))?;
    Observation::parse(&text).map_err(|e| Error::msg(format!("{}: {e}", path.display())))
}

/// `petri oracle observe template MONTH --source ID [--out FILE]`.
pub fn template(
    ctx: &Ctx,
    market: &str,
    expiry: Option<&str>,
    source: &str,
    out: Option<&Path>,
) -> Result<Report> {
    let of = of(ctx, market, expiry, source)?;
    let template = spec::observation_template(&of.series);
    let file = out.map(|p| p.display().to_string());
    let plain = series::json_out(
        &template,
        out,
        format!(
            "Wrote an observation template of series {} to {}. Fill in every null, then send it: petri oracle observe {} --source {} --file {}",
            of.series.series_id.as_deref().unwrap_or_default(),
            file.as_deref().unwrap_or_default(),
            of.month.period.expiry_id(),
            of.source.address,
            file.as_deref().unwrap_or_default(),
        ),
    )?;
    let payload = json!({
        "ok": true,
        "month": month_json(&of.month),
        "source": of.source.address.to_string(),
        "sourceId": hex(&of.source.state.source_id),
        "series": of.series,
        "template": template,
        "file": file,
    });
    Ok(Report::new(payload, plain))
}

/// The stake a print takes: exactly the product's bond for it (the program's
/// rule), `--stake` when it says the same.
fn stake(given: Option<&str>, bond: u64, what: &str) -> Result<u64> {
    match given {
        None => Ok(bond),
        Some(text) if usdc_amount(text, "the stake")? == bond => Ok(bond),
        Some(_) => {
            let bond = trim_decimal(u128::from(bond), 6);
            bail(format!(
                "the {what} of this product is {bond} USDC: pass --stake {bond}, or leave it out"
            ))
        }
    }
}

/// `petri oracle observe MONTH --source ID --file OBS.json [--stake USDC]`.
#[allow(clippy::too_many_arguments)]
pub fn observe(
    ctx: &Ctx,
    p: Paint,
    market: &str,
    expiry: Option<&str>,
    source: &str,
    file: &Path,
    given_stake: Option<&str>,
    simulate: bool,
) -> Result<Report> {
    let of = of(ctx, market, expiry, source)?;
    let observation = read_observation(file)?;
    let (prior, prior_note) = match records(ctx, &of.month, &of.source) {
        Ok(items) => (priors(&items), None),
        Err(e) => (
            Vec::new(),
            Some(format!(
                "Earlier observations could not be read from the API ({}); a repeat of a release it already has is not caught here",
                e.message
            )),
        ),
    };
    let routing = route(&of, &observation, &prior);
    // An observation the spec does not admit never counts: refusing it keeps
    // the stake off a disqualified print (as petri refuses an opening the
    // oracle's rules would reject). Routing itself refuses nothing.
    if !routing.status.admissible() {
        return Err(series::refuse(&routing, "this observation"));
    }
    let value = observation
        .raw_price
        .as_deref()
        .and_then(spec::price_atoms)
        .ok_or_else(|| Error::msg("the observation's rawPrice is not a price"))?;
    let reference = observation
        .evidence_reference
        .as_deref()
        .map(str::trim)
        .ok_or_else(|| Error::msg("the observation has no evidenceReference"))?
        .to_string();
    // The evidence the print cites: an archive, or a web proof whose price
    // must be the observation's (`writes::cite` checks it, and that the proof
    // is of this source's page and bound to this wallet).
    let signer = ctx.wallet_pubkey()?;
    let proof = proof_named(&reference);
    let cited = writes::cite(
        ctx,
        &of.oracle,
        &of.month,
        &of.source,
        match &proof {
            Some(bundle) => writes::Cite::Proof(bundle),
            None => writes::Cite::Archive(&reference),
        },
        Some(&trim_decimal(u128::from(value), 6)),
        &signer,
    )?;
    let pending = PendingObservation {
        market_id: of.month.period.symbol.to_ascii_lowercase(),
        expiry_id: of.month.period.expiry_id(),
        source_id: hex(&of.source.state.source_id),
        observation: serde_json::to_value(&observation).expect("an observation encodes"),
    };
    let pool = of.oracle.sku_pool(&of.month, &of.source.state.bucket_id)?;
    let checked = format!("Checked: {}", series::status_line(&routing));
    match of.month.state.phase {
        OraclePhase::Opening => {
            let amount = stake(given_stake, pool.opening_bond, "opening bond")?;
            let mut write = writes::opening_write(
                &of.oracle, &of.month, &of.source, &cited, amount, None, simulate,
            )?;
            write.action = "oracle.observe.opening";
            write.review.insert(1, checked);
            write.review.extend(prior_note);
            write.details["routing"] = serde_json::to_value(&routing).unwrap_or(Value::Null);
            let mut report = send::run(ctx, p, write, simulate)?;
            if !simulate {
                let claim = ix::find_oracle_opening_claim_address(
                    &of.oracle.program,
                    &of.month.address,
                    &of.source.address,
                );
                record_and_note(ctx, &mut report, &pending, &claim, &signer, Some(file));
            }
            Ok(report)
        }
        OraclePhase::Game => {
            let amount = stake(given_stake, pool.update_min_bond, "update stake")?;
            let (mut write, claim) = writes::update_commit_write(
                &of.oracle,
                &of.month,
                &of.source,
                &cited,
                amount,
                simulate,
                Some(pending),
            )?;
            write.action = "oracle.observe.update";
            write.review.insert(1, checked);
            write.review.extend(prior_note);
            write
                .review
                .push("Its observation record goes to the API once you reveal it".to_string());
            write.details["routing"] = serde_json::to_value(&routing).unwrap_or(Value::Null);
            let mut report = send::run(ctx, p, write, simulate)?;
            if !simulate {
                report.plain.push_str(&format!(
                    "\nReveal it within a minute: petri oracle updates reveal {claim}\nIts observation is recorded with the API then."
                ));
            }
            Ok(report)
        }
        phase => bail(format!(
            "{} is in {phase:?}: a month takes observations in Opening (its opening print) and in Game (updates)",
            of.month.period.expiry_id()
        )),
    }
}

/// The web proof an `evidenceReference` names, as `--evidence` takes it (a
/// bundle digest or file): itself, or the digest of the evidence record it
/// is (`amoeba-evidence-v1;…;d=<digest>;…`). None for a Wayback archive URL.
fn proof_named(reference: &str) -> Option<String> {
    use ameba_oracle::constants::{
        ORACLE_EVIDENCE_RECORD_PREFIX, ORACLE_OPENING_ARCHIVE_URL_PREFIX,
    };
    if reference.starts_with(ORACLE_OPENING_ARCHIVE_URL_PREFIX) {
        return None;
    }
    if reference.starts_with(ORACLE_EVIDENCE_RECORD_PREFIX)
        && let Some((_, _, digest, _)) = amoeba_evidence::parse_onchain_record(reference)
    {
        return Some(amoeba_evidence::hash::hex32(&digest));
    }
    Some(reference.to_string())
}

/// The record's signature by `claimant`, the wallet that made its claim,
/// over `observation_record_message` (the bytes the API verifies). The user
/// decided on 2026-10-21 that a record must carry its claimant's signature.
fn sign(
    ctx: &Ctx,
    pending: &PendingObservation,
    claim: &Pubkey,
    claimant: &Pubkey,
) -> Result<(Pubkey, solana_signature::Signature)> {
    sign_with(ctx.signer()?.as_ref(), pending, claim, claimant)
}

fn sign_with(
    signer: &dyn solana_signer::Signer,
    pending: &PendingObservation,
    claim: &Pubkey,
    claimant: &Pubkey,
) -> Result<(Pubkey, solana_signature::Signature)> {
    let observation: Observation = serde_json::from_value(pending.observation.clone())
        .map_err(|e| Error::msg(format!("the observation to record is not one: {e}")))?;
    let message = spec::observation_record_message(
        &pending.market_id,
        &pending.expiry_id,
        &pending.source_id,
        &claim.to_string(),
        &observation,
    );
    if signer.pubkey() != *claimant {
        bail(format!(
            "claim {claim} was made by {claimant}, not the attached wallet {}: its record is signed by the wallet that made it, so attach that one",
            signer.pubkey()
        ))?;
    }
    let signature = signer
        .try_sign_message(&message)
        .map_err(|e| Error::msg(format!("the wallet did not sign the record: {e}")))?;
    Ok((signer.pubkey(), signature))
}

/// Stores an observation record with the API, signed by `claimant`: its
/// claim must be the opening claim or a revealed update claim of the source,
/// made by that wallet (the API checks both on chain, and the signature).
fn post(
    ctx: &Ctx,
    pending: &PendingObservation,
    claim: &Pubkey,
    claimant: &Pubkey,
) -> Result<Value> {
    let (signer, signature) = sign(ctx, pending, claim, claimant)?;
    let body = json!({
        "marketId": pending.market_id,
        "expiryId": pending.expiry_id,
        "sourceId": pending.source_id,
        "claim": claim.to_string(),
        "observation": pending.observation,
        "signer": signer.to_string(),
        "signature": signature.to_string(),
    });
    let value = ctx.api.post(crate::endpoints::ORACLE_OBSERVATIONS, &body)?;
    if value.get("ok") != Some(&Value::Bool(true)) {
        bail("the API did not take the observation record")?;
    }
    Ok(value["data"].clone())
}

/// After a print landed: stores its record, or says how to store it later.
fn record_and_note(
    ctx: &Ctx,
    report: &mut Report,
    pending: &PendingObservation,
    claim: &Pubkey,
    claimant: &Pubkey,
    file: Option<&Path>,
) {
    match post(ctx, pending, claim, claimant) {
        Ok(data) => {
            let summary = data["routing"]["status"]
                .as_str()
                .and_then(spec::Status::parse)
                .map(|s| format!(": {}", s.summary()))
                .unwrap_or_default();
            report
                .plain
                .push_str(&format!("\nObservation recorded with the API{summary}."));
            report.payload["observationRecord"] = data;
        }
        Err(e) => {
            let retry = format!(
                "petri oracle observe record {} --source {} --claim {claim}{}",
                pending.expiry_id,
                pending.source_id,
                file.map(|f| format!(" --file {}", f.display()))
                    .unwrap_or_default()
            );
            report.plain.push_str(&format!(
                "\nThe print is on chain, but the API did not store its observation record ({}).\nStore it later: {retry}",
                e.message.replace('\n', " ")
            ));
            report.payload["observationRecord"] = Value::Null;
            report.payload["observationRecordRetry"] = Value::String(retry);
        }
    }
}

/// After `updates reveal` of an update `observe` committed: its record.
pub(super) fn record_after_send(
    ctx: &Ctx,
    report: &mut Report,
    pending: &PendingObservation,
    claim: &Pubkey,
    claimant: &Pubkey,
) {
    record_and_note(ctx, report, pending, claim, claimant, None);
}

/// `petri oracle observe record MONTH --source ID --claim CLAIM [--file OBS.json]`:
/// stores (again) the record of a print already on chain. Without `--file`
/// it is the observation saved with an update claim `observe` committed.
pub fn record(
    ctx: &Ctx,
    p: Paint,
    market: &str,
    expiry: Option<&str>,
    source: &str,
    claim: &str,
    file: Option<&Path>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(market, expiry)?;
    let source = oracle.source(&month, source)?;
    let claim = ctx.parse_pubkey(claim, "claim")?;
    let opening =
        ix::find_oracle_opening_claim_address(&oracle.program, &month.address, &source.address);
    // The wallet that made the claim signs its record.
    let claimant = if claim == opening {
        writes::opening_claim(&oracle, &month, &source)?.1.claimant
    } else {
        let (_, state) = writes::update_claim(&oracle, &claim.to_string())?;
        if state.claim.source != source.address {
            bail(format!(
                "{claim} is not a claim of source {}",
                source.address
            ))?;
        }
        state.claim.claimant
    };
    let observation = match file {
        Some(path) => serde_json::to_value(read_observation(path)?).expect("encodes"),
        None if claim == opening => bail(
            "an opening print keeps no observation of its own: pass --file OBS.json, the file it was sent with",
        )?,
        None => claims::load(&claim.to_string())?
            .observation
            .map(|o| o.observation)
            .ok_or_else(|| {
                Error::msg(format!(
                    "no observation is saved with claim {claim}: pass --file OBS.json"
                ))
            })?,
    };
    let pending = PendingObservation {
        market_id: month.period.symbol.to_ascii_lowercase(),
        expiry_id: month.period.expiry_id(),
        source_id: hex(&source.state.source_id),
        observation,
    };
    let data = post(ctx, &pending, &claim, &claimant)?;
    let summary = data["routing"]["status"]
        .as_str()
        .and_then(spec::Status::parse)
        .map(|s| format!("{} ({s})", s.summary()))
        .unwrap_or_else(|| "stored".into());
    let lines = [
        p.bold(&format!(
            "Observation recorded for source {}",
            source.address
        )),
        format!("  Claim {claim}"),
        format!("  Checked: {summary}"),
    ];
    Ok(Report::new(
        json!({"ok": true, "month": month_json(&month), "record": data}),
        lines.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stakes_are_the_products_bond() {
        assert_eq!(stake(None, 1_000_000, "x").unwrap(), 1_000_000);
        assert_eq!(stake(Some("1"), 1_000_000, "x").unwrap(), 1_000_000);
        let error = stake(Some("2.5"), 1_000_000, "opening bond").unwrap_err();
        assert_eq!(
            error.message,
            "the opening bond of this product is 1 USDC: pass --stake 1, or leave it out"
        );
    }

    #[test]
    fn an_evidence_reference_names_an_archive_or_a_web_proof() {
        let archive = "https://web.archive.org/web/20261201010000/https://example.com/price";
        assert_eq!(proof_named(archive), None);
        let digest = "ab".repeat(32);
        assert_eq!(proof_named(&digest).as_deref(), Some(digest.as_str()));
        let record =
            format!("amoeba-evidence-v1;m=web;t=1796086800;d={digest};u=https://example.com/price");
        assert_eq!(proof_named(&record).as_deref(), Some(digest.as_str()));
    }

    #[test]
    fn a_record_is_signed_by_its_claimant_over_the_specs_bytes() {
        use solana_keypair::Keypair;
        use solana_signer::Signer;
        let wallet = Keypair::new();
        let claim = Pubkey::new_unique();
        let observation = Observation {
            schema: ameba_commodity_spec::OBSERVATION_SCHEMA.into(),
            observation_id: Some("obs-1".into()),
            raw_price: Some("2.95".into()),
            ..Default::default()
        };
        let pending = PendingObservation {
            market_id: "ramx".into(),
            expiry_id: "RAMX-202612".into(),
            source_id: "ab".repeat(32),
            // As saved: a JSON value, keys in any order.
            observation: serde_json::to_value(&observation).unwrap(),
        };
        let (signer, signature) = sign_with(&wallet, &pending, &claim, &wallet.pubkey()).unwrap();
        assert_eq!(signer, wallet.pubkey());
        let message = spec::observation_record_message(
            "ramx",
            "RAMX-202612",
            &"ab".repeat(32),
            &claim.to_string(),
            &observation,
        );
        assert!(signature.verify(wallet.pubkey().as_ref(), &message));
        // Another wallet's claim is refused before anything is signed.
        let other = Pubkey::new_unique();
        let error = sign_with(&wallet, &pending, &claim, &other).unwrap_err();
        assert!(
            error.message.contains("attach that one"),
            "{}",
            error.message
        );
    }

    #[test]
    fn earlier_records_give_their_releases() {
        let items = vec![
            json!({"observation": {"releaseId": "2026-10 H2", "referencePeriodEnd": "2026-10-31"}}),
            json!({"observation": {"referencePeriodEnd": "2026-09-30"}}),
        ];
        assert_eq!(
            priors(&items),
            [("2026-10 H2".to_string(), "2026-10-31".to_string())]
        );
        assert_eq!(date(1_798_761_600), "2027-01-01");
    }
}
