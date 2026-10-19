//! Sponsorship (oracle tag 30/22): a funder underwrites a research agent's
//! source candidate. The sponsor's oracle collateral posts the product's exact
//! listing bond and the candidate becomes an oracle source whose id is the
//! candidate's content hash, proposed by the sponsorship agreement. When the
//! source's bond settles the agreement sweeps it back to the sponsor
//! (Reconcile); once the month's rewards are frozen, its proposer reward is
//! split: 20% to the researcher (a fifth of that held in their reserve), the
//! rest to the sponsor (ClaimBounty). Mainnet's oracle predates that split
//! and takes the researcher's half (`researcher_bps`). A challenger who proves the source wrong
//! takes the bond, as with any listing.

use ameba_oracle::{
    instruction::{self as ix, ProposeOracleSourceV3Params, SponsoredAccounts},
    sponsorship::{
        DEFAULT_RETENTION_BPS, SourceSponsorship, SponsorTerms, exposure_address, reserve_address,
    },
    state::OraclePhase,
};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    args::{EvidenceKind, SendArgs},
    chain::{MonthRef, Oracle},
    reads,
    send::{self, Tx, Write},
    skus::{self, hex, parse_hex32},
    writes::{
        SOURCE_TYPE_DOMAIN, evidence_preimage, evidence_txs, month_json, month_line, semantic_hash,
    },
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::{parse_usdc, usdc},
    view::{Paint, Table},
};

/// A research candidate, as the futures API's candidate store has it.
pub struct Candidate {
    pub content_hash: [u8; 32],
    pub agent_id: [u8; 32],
    pub researcher: Pubkey,
    pub market: String,
    pub expiry: String,
    pub bucket_id: [u8; 32],
    pub url: String,
    pub source_type: String,
    /// "Product: …; Price field: …; Region: …; Currency: …".
    pub source_definition: String,
    raw: Value,
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::msg(format!("the candidate has no {key}")))
}

/// The candidate `hash` from the futures API.
pub fn candidate(ctx: &Ctx, hash: &str) -> Result<Candidate> {
    let hash = hex(&parse_hex32(hash, "candidate")?);
    let data = reads::document(ctx, &crate::endpoints::oracle_candidate(&hash))?;
    let c = &data["candidate"];
    let d = &c["details"];
    Ok(Candidate {
        content_hash: parse_hex32(field(c, "contentHash")?, "content hash")?,
        agent_id: parse_hex32(field(c, "agentId")?, "agent id")?,
        researcher: field(c, "payoutAuthority")?
            .parse()
            .map_err(|_| Error::msg("the candidate's researcher address is not a public key"))?,
        market: field(d, "marketId")?.to_string(),
        expiry: field(d, "expiryId")?.to_string(),
        bucket_id: parse_hex32(field(d, "bucketId")?, "bucket id")?,
        url: field(d, "canonicalUrl")?.to_string(),
        source_type: field(d, "sourceType")?.to_string(),
        source_definition: field(c, "sourceDefinition")?.to_string(),
        raw: c.clone(),
    })
}

/// Whether `month` takes new sources now (the program checks the placement
/// window itself).
fn takes_sources(month: &MonthRef) -> bool {
    matches!(
        month.state.phase,
        OraclePhase::SourceSubmission | OraclePhase::Scramble
    )
}

/// The sponsor's own cap across a researcher's sponsorships: what is out to
/// them already plus what the collateral can post now. The program counts the
/// new bond into the outstanding before it checks this cap.
fn maximum_outstanding(outstanding: u64, available: u64, bond: u64) -> u64 {
    outstanding.saturating_add(available).max(bond)
}

/// The sponsorship of candidate `hash` to review and send.
pub fn sponsor_write(ctx: &Ctx, hash: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let sponsor = ctx.wallet_pubkey()?;
    let c = candidate(ctx, hash)?;
    let month = oracle.month(&c.market, Some(&c.expiry))?;
    if !takes_sources(&month) {
        bail(format!(
            "{} isn't taking new sources (it is in {}); sponsoring opens with the next month's source window",
            month.period.expiry_id(),
            month.phase_name()
        ))?;
    }
    let product = skus::decode_label(&c.bucket_id).unwrap_or_else(|| hex(&c.bucket_id));
    // The candidate's commitments must be the ones this source will register.
    let (_, locator_bytes, locator_hash) = evidence_preimage(EvidenceKind::Locator, &c.url)?;
    let (_, definition_bytes, definition_hash) =
        evidence_preimage(EvidenceKind::Definition, &c.source_definition)?;
    let source_type_hash = semantic_hash(SOURCE_TYPE_DOMAIN, &c.source_type);
    for (name, ours, theirs) in [
        ("locator", locator_hash, "canonicalLocatorHash"),
        ("definition", definition_hash, "sourceDefinitionHash"),
        ("source type", source_type_hash, "sourceTypeHash"),
    ] {
        if c.raw[theirs].as_str().map(str::to_ascii_lowercase) != Some(hex(&ours)) {
            bail(format!(
                "the candidate's {name} hash does not match its details; it cannot be sponsored as is"
            ))?;
        }
    }
    let (sku_index, sku_proof) = oracle.sku_proof(&month, &c.bucket_id)?;
    let pool = oracle.sku_pool(&month, &c.bucket_id)?;
    let bond = pool.listing_bond;
    let available = oracle
        .collateral(&sponsor)?
        .map_or(0, |l| l.available_balance);
    if available < bond {
        bail(format!(
            "sponsoring takes a {} USDC bond and your oracle collateral has {} USDC; deposit first",
            usdc(bond),
            usdc(available)
        ))?;
    }
    let source = ix::find_oracle_source_address(&oracle.program, &month.address, &c.content_hash);
    if oracle.sponsorship(&source)?.is_some() || ctx.rpc.account(&source)?.is_some() {
        bail("this candidate is already on the oracle")?;
    }
    let reserve = reserve_address(&oracle.program, &c.agent_id, &c.researcher).0;
    let outstanding = oracle.exposure(&reserve, &sponsor)?;
    let researcher_bps = researcher_bps(ctx)?;
    let terms = SponsorTerms {
        agent_id: c.agent_id,
        researcher: c.researcher,
        candidate_hash: c.content_hash,
        funding: 0,
        researcher_bps,
        retention_bps: DEFAULT_RETENTION_BPS,
        maximum_bond: bond,
        maximum_outstanding: maximum_outstanding(outstanding, available, bond),
    };
    let mut txs = Vec::new();
    let (locator_object, locator_txs, _) =
        evidence_txs(&oracle, &sponsor, 1, &locator_bytes, locator_hash)?;
    let (definition_object, definition_txs, _) =
        evidence_txs(&oracle, &sponsor, 2, &definition_bytes, definition_hash)?;
    txs.extend(locator_txs);
    txs.extend(definition_txs);
    txs.push(Tx::one(
        "sponsorSource",
        ix::sponsored_activate(
            &oracle.program,
            &sponsor,
            &sponsor,
            &sponsor,
            &month.period.address,
            &month.address,
            terms,
            ProposeOracleSourceV3Params {
                source_id: c.content_hash,
                bucket_id: c.bucket_id,
                source_type_hash,
                canonical_locator_hash: locator_hash,
                source_definition_hash: definition_hash,
                listing_bond: bond,
                sku_index,
                sku_proof,
            },
        ),
    ));
    Ok(Write {
        action: "oracle.sponsor",
        title: format!(
            "Sponsor {product} for {}: a {} USDC bond",
            month.period.expiry_id(),
            usdc(bond)
        ),
        review: vec![
            month_line(&month),
            format!("Page {}", c.url),
            format!("Found by agent {} (pays {})", &hex(&c.agent_id)[..8], c.researcher),
            "The bond comes from your oracle collateral and comes back when the source settles, unless a challenger proves it wrong".to_string(),
            format!(
                "If it goes live, the researcher gets {}% of its reward (a fifth of that held in their reserve) and you the rest",
                researcher_bps / 100
            ),
            format!("Evidence: locator {locator_object}, definition {definition_object}"),
        ],
        details: json!({
            "month": month_json(&month),
            "source": source.to_string(),
            "candidate": hex(&c.content_hash),
            "bondAtoms": bond.to_string(),
            "researcher": c.researcher.to_string(),
        }),
        txs,
    })
}

/// `petri oracle sponsor <CANDIDATE>`.
pub fn sponsor(ctx: &Ctx, p: Paint, hash: &str, send: &SendArgs) -> Result<Report> {
    send::run(ctx, p, sponsor_write(ctx, hash)?, send.simulate)
}

/// The researcher's share a new sponsorship must name on this deployment:
/// mainnet's oracle predates the 80/20 split and takes the legacy half.
fn researcher_bps(ctx: &Ctx) -> Result<u16> {
    let mainnet = ctx.deployment()?.genesis_hash.as_deref()
        == Some(crate::futures::settings::MAINNET_GENESIS);
    Ok(ameba_oracle::sponsorship::researcher_bps_for(mainnet))
}

/// What became of a sponsorship, in words.
pub fn outcome(a: &SourceSponsorship) -> &'static str {
    match (a.outcome, a.reward_settled) {
        (0, _) => "waiting",
        (1, false) => "live, reward to claim",
        (1, true) => "live, reward paid",
        (2, _) => "rejected",
        _ => "closed without a reward",
    }
}

/// `petri oracle sponsorships`: the sponsorships the wallet paid for.
pub fn sponsorships(ctx: &Ctx, p: Paint) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let sponsor = ctx.wallet_pubkey()?;
    let list = oracle.sponsorships(&sponsor)?;
    let mut table = Table::new(&["Source", "Product", "Bond", "Outcome", "Reward paid"]);
    let mut items = Vec::new();
    for (agreement, a) in &list {
        let product = skus::decode_label(&a.bucket_id).unwrap_or_else(|| hex(&a.bucket_id));
        table.row(vec![
            a.source.to_string(),
            product.clone(),
            format!("{} USDC", usdc(a.bond)),
            outcome(a).to_string(),
            format!("{} USDC", usdc(a.reward_paid)),
        ]);
        items.push(json!({
            "agreement": agreement.to_string(),
            "source": a.source.to_string(),
            "month": a.month.to_string(),
            "candidate": hex(&a.candidate_hash),
            "product": product,
            "bondAtoms": a.bond.to_string(),
            "outcome": outcome(a),
            "rewardPaidAtoms": a.reward_paid.to_string(),
        }));
    }
    let mut lines = vec![p.bold(&format!("Sponsorships paid by {sponsor}"))];
    if table.is_empty() {
        lines.push(
            "None yet: `petri oracle sponsor <CANDIDATE>` sponsors a research candidate.".into(),
        );
    } else {
        lines.extend(table.lines());
    }
    Ok(Report::new(
        json!({"ok": true, "sponsor": sponsor.to_string(), "sponsorships": items}),
        lines.join("\n"),
    ))
}

/// A sponsorship and its accounts, by its source.
fn sponsored(
    oracle: &Oracle,
    source: &str,
) -> Result<(Pubkey, SourceSponsorship, SponsoredAccounts)> {
    let source: Pubkey = source
        .parse()
        .map_err(|_| Error::msg("give the sponsored source's address"))?;
    let (agreement, a) = oracle
        .sponsorship(&source)?
        .ok_or_else(|| Error::msg(format!("{source} was not sponsored")))?;
    let accounts = SponsoredAccounts {
        agreement,
        reserve: a.reserve,
        funder: a.funder,
        exposure: exposure_address(&oracle.program, &a.reserve, &a.funder).0,
    };
    Ok((source, a, accounts))
}

/// Once the source's bond can settle: settle it to the agreement, then
/// record the outcome, which sweeps the agreement's cash to the sponsor.
pub fn settle_write(ctx: &Ctx, source: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let (source, a, accounts) = sponsored(&oracle, source)?;
    let month = oracle.month_at(&a.month)?;
    let mut tx = Tx::new();
    tx.push(
        "settleListingBond",
        ix::settle_oracle_usdc_prelisting_escrow(
            &oracle.program,
            &cranker,
            &month.period.address,
            &month.address,
            &ix::PrelistingEscrow::ListingBond {
                source,
                proposer: accounts.agreement,
            },
        ),
    );
    tx.push(
        "reconcileSponsorship",
        ix::sponsored_reconcile(
            &oracle.program,
            &cranker,
            &month.address,
            &source,
            &accounts,
        ),
    );
    Ok(Write {
        action: "oracle.sponsorships.settle",
        title: format!("Settle the sponsorship of {source}"),
        review: vec![
            month_line(&month),
            format!("Bond {} USDC, back to {}", usdc(a.bond), a.funder),
        ],
        details: json!({"source": source.to_string(), "agreement": accounts.agreement.to_string()}),
        txs: vec![tx],
    })
}

/// Once the month's rewards are frozen: the source's proposer reward, split
/// between the sponsor, the researcher and the researcher's reserve.
pub fn claim_write(ctx: &Ctx, source: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let payer = ctx.wallet_pubkey()?;
    let mint = ctx.deployment()?.usdc_mint;
    let (source, a, accounts) = sponsored(&oracle, source)?;
    let month = oracle.month_at(&a.month)?;
    let reserve: ameba_oracle::sponsorship::AgentReserve = super::chain::decode(
        &oracle.program,
        &a.reserve,
        ctx.rpc.account(&a.reserve)?.as_ref(),
        ameba_oracle::sponsorship::AgentReserve::LEN,
        "agent reserve",
    )?;
    Ok(Write {
        action: "oracle.sponsorships.claim",
        title: format!("Claim the bounty of {source}"),
        review: vec![
            month_line(&month),
            format!(
                "Researcher {} gets {}%, you the rest",
                reserve.authority,
                a.researcher_bps / 100
            ),
        ],
        details: json!({"source": source.to_string()}),
        txs: vec![Tx::one(
            "claimBounty",
            ix::sponsored_claim_bounty(
                &oracle.program,
                &payer,
                &month.period.address,
                &month.address,
                &mint,
                &a.bucket_id,
                &source,
                &reserve.authority,
                &accounts,
            ),
        )],
    })
}

/// A researcher moves `amount` from an agent's reserve to their own ledger.
pub fn reserve_withdraw_write(ctx: &Ctx, agent: &str, amount: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let researcher = ctx.wallet_pubkey()?;
    let agent = parse_hex32(agent, "agent id")?;
    let reserve = reserve_address(&oracle.program, &agent, &researcher).0;
    let atoms = parse_usdc(amount).map_err(|_| Error::msg("give an amount in USDC"))?;
    Ok(Write {
        action: "oracle.reserve.withdraw",
        title: format!(
            "Withdraw {} USDC from the reserve of agent {}",
            usdc(atoms),
            &hex(&agent)[..8]
        ),
        review: vec![format!("To your oracle collateral ({researcher})")],
        details: json!({"reserve": reserve.to_string(), "amountAtoms": atoms.to_string()}),
        txs: vec![Tx::one(
            "withdrawReserve",
            ix::sponsored_withdraw_reserve(&oracle.program, &researcher, &reserve, atoms),
        )],
    })
}

#[cfg(test)]
mod tests {
    use super::maximum_outstanding;

    /// What the program checks: the outstanding with the new bond counted in.
    fn admits(outstanding: u64, available: u64, bond: u64) -> bool {
        outstanding + bond <= maximum_outstanding(outstanding, available, bond)
    }

    #[test]
    fn the_cap_counts_bonds_already_out_to_the_researcher() {
        // The 2026-10-18 rehearsal: 3 USDC out to one researcher, 2 free, a
        // 1 USDC bond. A cap of just the free balance refused it.
        assert!(admits(3_000_000, 2_000_000, 1_000_000));
        assert_eq!(
            maximum_outstanding(3_000_000, 2_000_000, 1_000_000),
            5_000_000
        );
        // Every bond the collateral can still post goes through.
        assert!(admits(4_000_000, 1_000_000, 1_000_000));
        // A first sponsorship: nothing out yet.
        assert!(admits(0, 1_000_000, 1_000_000));
        assert_eq!(maximum_outstanding(0, 0, 1_000_000), 1_000_000);
        assert_eq!(maximum_outstanding(u64::MAX, 1, 1), u64::MAX);
    }
}
