//! A proof as on-chain evidence: the authenticated evidence record (oracle
//! evidence object kind 4) an opening, update or challenge cites instead of
//! a Wayback URL:
//!
//! ```text
//! amoeba-evidence-v1;m=web;t=<witnessed unix seconds>;d=<bundle digest>;u=<the source's locator>
//! ```
//!
//! The program checks the record's form, that `u` is the source's
//! registered page and that `t` is the claim's time. Everything else is
//! checked here before anything is sent, and again by anyone who fetches
//! the bundle by its digest: the proof verifies, it is for this source's
//! page, it reads the field frozen with the source, and it is bound to the
//! wallet that cites it.

use amoeba_evidence::{Destination, check_destination, hash::hex32, onchain_record, verify_bundle};
use solana_program::{hash::hashv, pubkey::Pubkey};

use crate::futures::{
    ctx::Ctx,
    error::{Error, Result},
};

pub struct Record {
    /// The kind-4 record text.
    pub record: String,
    /// The proven price in six-decimal atoms, and as printed.
    pub atoms: u64,
    pub price: String,
    /// The notary-attested session time (the claim's source time).
    pub time: u64,
    pub digest: String,
}

/// The selector frozen with a source, when its definition declares one
/// (`…; Evidence: web proof, selector <hex>`, from the futures API's
/// candidate record of a sponsored source). `None` when it declares none.
fn frozen_selector(ctx: &Ctx, source_id: &[u8; 32]) -> Option<[u8; 32]> {
    let data =
        crate::oracle::reads::document(ctx, &crate::endpoints::oracle_candidate(&hex32(source_id)))
            .ok()?;
    let definition = data["candidate"]["sourceDefinition"].as_str()?;
    let at = definition.find("selector ")?;
    amoeba_evidence::hash::parse_hex32(definition.get(at + 9..at + 9 + 64)?)
}

#[allow(clippy::too_many_arguments)]
pub fn record(
    ctx: &Ctx,
    oracle_program: &Pubkey,
    month: &Pubkey,
    source_id: &[u8; 32],
    locator_hash: &[u8; 32],
    signer: &Pubkey,
    bundle: &str,
) -> Result<Record> {
    let (path, text) = super::load(bundle)?;
    let mut v = verify_bundle(text.as_bytes(), &super::policy(ctx)?);
    if !v.authenticated() {
        let reason = [
            &v.stages.received,
            &v.stages.provenance,
            &v.stages.extraction,
        ]
        .iter()
        .find_map(|s| s.reason.clone())
        .unwrap_or_else(|| "it does not verify".into());
        return Err(Error::msg(format!(
            "{} does not verify: {reason}",
            path.display()
        )));
    }
    let env = v.envelope.clone().expect("authenticated");
    if hashv(&[b"locator", env.origin.locator.as_bytes()]).to_bytes() != *locator_hash {
        return Err(Error::msg(format!(
            "the proof is for {}, which is not this source's registered page",
            env.origin.locator
        )));
    }
    check_destination(
        &mut v,
        &Destination::Update {
            oracle_program: oracle_program.to_string(),
            month: month.to_string(),
            source_id: *source_id,
            locator: env.origin.locator.clone(),
            selector_digest: frozen_selector(ctx, source_id),
            claimant: Some(signer.to_bytes()),
        },
    );
    if !v.stages.eligibility.ok() {
        return Err(Error::msg(format!(
            "this proof can't back it: {}",
            v.stages.eligibility.reason.clone().unwrap_or_default()
        )));
    }
    let time = env
        .times
        .witnessed_at
        .value
        .parse::<u64>()
        .map_err(|_| Error::msg("the proof has no session time"))?;
    Ok(Record {
        record: onchain_record(&env),
        atoms: env.observation.price.atoms,
        price: env.observation.price.text.clone(),
        time,
        digest: env.authentication.bundle_digest.clone(),
    })
}

/// Makes the bundle available to anyone who checks the claim: uploads it to
/// the futures API (which verifies it and serves it by digest).
pub fn publish(ctx: &Ctx, digest: &str) -> Result<()> {
    let (_, text) = super::load(digest)?;
    let dir = super::store::dir()?;
    super::submit::upload(ctx, &dir, digest, &text).map(|_| ())
}
