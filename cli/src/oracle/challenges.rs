//! Source challenges (tag 161) and how they end. A challenge says a candidate
//! source is wrong and backs that with a bond. Petri holds it to the
//! program's rules before anything is sent (the window, the sources, the
//! exact bond, the collateral) and, with `--why`, seals the reason as a
//! public evidence object the challenge commits to, so whoever rules can read
//! it. The oracle authority rules on Scramble day 7 (tag 185, `futures-ops
//! resolve-challenge`), or leaves it to the council (30/18); the bond comes
//! back through `stakes settle --kind source-challenge` (tag 169). `petri
//! oracle challenges` lists a wallet's challenges and the sources open to
//! challenge now, read from chain.

use std::collections::{HashMap, HashSet};

use ameba_oracle::{
    constants::{
        ORACLE_KILL_WINDOW_SECONDS, ORACLE_PLACEMENT_WINDOW_SECONDS,
        ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS,
    },
    instruction::{self as ix, ChallengeOracleSourceParams},
    state::{
        OracleChallengeStatus, OracleEscrowDisposition, OracleMonthState, OraclePhase,
        OracleSourceChallengeGuard, OracleSourceStatus, OracleUsdcRewardSchedulePhase,
        OracleUsdcSkuPool,
    },
};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    args::{EvidenceKind, Target},
    chain::{MonthRef, Oracle, SourceRef, definition_in},
    send::{self, Tx, Write},
    skus::{hex, parse_hex32},
    writes::{
        evidence_preimage, evidence_txs, id_or_random, month_json, month_line, read_body,
        source_json, usdc_amount,
    },
};
use crate::{
    backend::terminal_safe_text,
    futures::{
        Report,
        ctx::Ctx,
        error::{Result, bail},
        units::{short, trim_decimal, usdc},
        view::{Paint, Table, utc},
    },
};

/// A reason a source is challenged for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reason {
    /// What `--reason` takes and JSON names.
    pub name: &'static str,
    /// The code the challenge stores.
    pub code: u8,
    /// What people read.
    pub label: &'static str,
}

/// The program's non-independent reason
/// (`ORACLE_SOURCE_REASON_NON_INDEPENDENT`), the only code it reads.
pub const DUPLICATE: u8 = 8;

/// The challenge reasons of the contributor docs
/// (`docs/src/content/docs/contributors/03-challenges.md`) and the codes
/// petri sends for them. The program stores the code as given; only
/// `duplicate` means anything to it: a comparison source of the same bucket,
/// a bond on the larger backing of the two, and a ruling that can merge them.
pub const REASONS: [Reason; 6] = [
    Reason {
        name: "wrong-component",
        code: 1,
        label: "Wrong component",
    },
    Reason {
        name: "unavailable",
        code: 2,
        label: "Source unavailable",
    },
    Reason {
        name: "stale",
        code: 3,
        label: "Stale observation",
    },
    Reason {
        name: "methodology",
        code: 4,
        label: "Unsupported methodology",
    },
    Reason {
        name: "other",
        code: 5,
        label: "Other evidence conflict",
    },
    Reason {
        name: "duplicate",
        code: DUPLICATE,
        label: "Duplicate source",
    },
];

/// `--reason`'s help: every name of [`REASONS`].
pub const REASON_HELP: &str = "The reason: wrong-component, unavailable, stale, methodology, other, or duplicate (with --comparison); a number is sent as that code";

/// `--reason`: a name of [`REASONS`], or a code from 0 to 255, sent as is.
pub fn parse_reason(text: &str) -> std::result::Result<u8, String> {
    let wanted = text.trim().to_ascii_lowercase();
    if let Some(reason) = REASONS.iter().find(|r| r.name == wanted) {
        return Ok(reason.code);
    }
    wanted.parse::<u8>().map_err(|_| {
        let names: Vec<&str> = REASONS.iter().map(|r| r.name).collect();
        format!(
            "`{text}` is not a reason: use {}, or a code from 0 to 255",
            names.join(", ")
        )
    })
}

fn reason(code: u8) -> Option<&'static Reason> {
    REASONS.iter().find(|r| r.code == code)
}

/// A code's name (`duplicate`), or `reason 7` outside [`REASONS`].
pub fn reason_name(code: u8) -> String {
    reason(code).map_or_else(|| format!("reason {code}"), |r| r.name.to_string())
}

/// A code's label (`Duplicate source`), or `reason 7` outside [`REASONS`].
pub fn reason_label(code: u8) -> String {
    reason(code).map_or_else(|| format!("reason {code}"), |r| r.label.to_string())
}

/// A product's challenge terms, from its SKU pool (tag 157).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms {
    pub bps: u16,
    pub minimum: u64,
    pub maximum: u64,
}

impl Terms {
    pub fn of(pool: &OracleUsdcSkuPool) -> Self {
        Self {
            bps: pool.challenge_bond_bps,
            minimum: pool.challenge_min_bond,
            maximum: pool.challenge_max_bond,
        }
    }

    /// The bond a challenge must post, exactly (the program's
    /// `calculate_oracle_usdc_challenge_bond`): `floor(backing × bps /
    /// 10 000)`, held between the minimum and the maximum. The backing is the
    /// source's support (its listing bond does not count), for a `duplicate`
    /// the larger support of the two sources.
    pub fn bond(&self, support: u64, comparison: Option<u64>) -> Result<u64> {
        let backing = comparison.map_or(support, |c| support.max(c));
        if backing == 0 {
            bail(if comparison.is_some() {
                "nobody supports either source yet, so they can't be challenged"
            } else {
                "nobody supports this source yet, so it can't be challenged"
            })?;
        }
        self.share(backing).ok_or_else(|| {
            crate::futures::error::Error::msg(
                "this product's challenge terms are not set, so its sources can't be challenged",
            )
        })
    }

    /// `floor(backing × bps / 10 000)` held between the minimum and the
    /// maximum: the program's `calculate_oracle_usdc_challenge_bond`, for a
    /// source's backing or (an opening print's challenge) the print's stake.
    /// None when the terms are not set or nothing backs it.
    pub fn share(&self, backing: u64) -> Option<u64> {
        if backing == 0
            || self.bps > 10_000
            || self.minimum == 0
            || self.maximum == 0
            || self.minimum > self.maximum
        {
            return None;
        }
        let proportional = u128::from(backing) * u128::from(self.bps) / 10_000;
        Some(
            u64::try_from(proportional)
                .unwrap_or(u64::MAX)
                .clamp(self.minimum, self.maximum),
        )
    }

    /// `20% of the backing, at least 1.00 and at most 10.00 USDC`.
    fn words(&self) -> String {
        self.words_of("the backing")
    }

    /// The terms in words, as a share of `what`.
    pub fn words_of(&self, what: &str) -> String {
        format!(
            "{}% of {what}, at least {} and at most {} USDC",
            trim_decimal(u128::from(self.bps), 2),
            usdc(self.minimum),
            usdc(self.maximum)
        )
    }
}

/// A month's challenge calendar on the chain's clock (the program's
/// `ensure_oracle_kill_window` and `ensure_oracle_resolution_freeze_window`):
/// sources are challenged on Scramble days 5 and 6, from the end of its four
/// placement days, and ruled on day 7, by the Opening's start. A challenge
/// still unruled then settles as dismissed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub phase: OraclePhase,
    pub opens: u64,
    pub closes: u64,
    pub ruled_by: u64,
    /// The recipe freeze has begun (weight scheme 255).
    pub freezing: bool,
}

impl Window {
    pub fn of(month: &OracleMonthState) -> Self {
        let opens = month
            .scramble_start_ts
            .saturating_add(ORACLE_PLACEMENT_WINDOW_SECONDS);
        let closes = opens.saturating_add(ORACLE_KILL_WINDOW_SECONDS);
        Self {
            phase: month.phase,
            opens,
            closes,
            ruled_by: closes.saturating_add(ORACLE_RESOLUTION_FREEZE_WINDOW_SECONDS),
            freezing: month.weight_scheme_version == 255,
        }
    }

    pub fn is_open(&self, now: u64) -> bool {
        self.check(now, "").is_ok()
    }

    /// Refuses unless `now` is inside the window, saying when it opens or
    /// when it closed.
    pub fn check(&self, now: u64, month: &str) -> Result<()> {
        let at = |t: u64| utc(i64::try_from(t).unwrap_or(i64::MAX));
        match self.phase {
            OraclePhase::Scramble => {}
            OraclePhase::SourceSubmission => bail(format!(
                "{month} is still taking sources: its challenge days, Scramble days 5 and 6, come once its source window closes"
            ))?,
            phase => bail(format!(
                "{month} is in {phase:?}: sources are challenged in Scramble, on days 5 and 6; its window closed at {}",
                at(self.closes)
            ))?,
        }
        if self.freezing {
            bail(format!(
                "{month}'s recipe is being frozen: its challenges are over"
            ))?;
        }
        if now < self.opens {
            bail(format!(
                "the challenge window of {month} opens at {} (Scramble day 5) and closes at {}; the chain's clock says {}",
                at(self.opens),
                at(self.closes),
                at(now)
            ))?;
        }
        if now >= self.closes {
            bail(format!(
                "the challenge window of {month} closed at {} (the end of Scramble day 6); the chain's clock says {}",
                at(self.closes),
                at(now)
            ))?;
        }
        Ok(())
    }
}

/// Where a source challenge stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// In review: the oracle authority rules on Scramble day 7.
    Waiting,
    /// The ruling left it unresolved: the council decides it (30/18).
    Council,
    /// Upheld, the source rejected: the challenger gets the bond back and the
    /// source's listing bond.
    Rejected,
    /// Upheld, the source merged into its comparison source: paid as
    /// [`Self::Rejected`].
    Merged,
    /// Dismissed: the source stays and the bond goes to its proposer.
    Dismissed,
    /// Not ruled by the Opening's start: it settles as dismissed.
    Lapsed,
    /// Cancelled (tag 192): the bond goes back to the challenger.
    Cancelled,
}

impl Standing {
    /// A challenge's standing from its status, the challenged source's
    /// status (an upheld one is rejected or merged) and the chain's clock.
    pub fn of(
        status: OracleChallengeStatus,
        source: Option<OracleSourceStatus>,
        now: u64,
        ruled_by: u64,
    ) -> Self {
        use OracleChallengeStatus as S;
        match status {
            S::Open | S::RuleReview if now >= ruled_by => Self::Lapsed,
            S::Open | S::RuleReview => Self::Waiting,
            S::RuleReviewUnresolved => Self::Council,
            S::Accepted if source == Some(OracleSourceStatus::Merged) => Self::Merged,
            S::Accepted => Self::Rejected,
            S::Rejected => Self::Dismissed,
            S::Cancelled => Self::Cancelled,
        }
    }

    /// How it ended, for JSON: None while it waits for a ruling.
    pub fn outcome(self) -> Option<&'static str> {
        match self {
            Self::Waiting | Self::Council => None,
            Self::Rejected => Some("upheld-rejected"),
            Self::Merged => Some("upheld-merged"),
            Self::Dismissed | Self::Lapsed => Some("dismissed"),
            Self::Cancelled => Some("cancelled"),
        }
    }

    /// Where it stands, for JSON (`settled` once its bond settled).
    pub fn code(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "settled",
            Self::Waiting => "waiting",
            Self::Council => "council",
            other => other.outcome().unwrap_or("waiting"),
        }
    }

    pub fn words(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "Settled",
            Self::Waiting => "Waiting for the ruling (day 7)",
            Self::Council => "With the council",
            Self::Rejected => "Upheld — source rejected",
            Self::Merged => "Upheld — merged",
            Self::Dismissed => "Dismissed",
            Self::Lapsed => "Dismissed (not ruled in time)",
            Self::Cancelled => "Cancelled",
        }
    }

    /// Whose collateral its bond settles into once it is ruled.
    pub fn pays(self) -> Option<&'static str> {
        match self {
            Self::Waiting | Self::Council => None,
            Self::Rejected | Self::Merged | Self::Cancelled => Some("challenger"),
            Self::Dismissed | Self::Lapsed => Some("proposer"),
        }
    }
}

/// What a challenge commits to as its evidence.
#[derive(Clone, Copy, Debug)]
pub enum Evidence<'a> {
    /// Why, in words: sealed first as a public definition object.
    Why(&'a str),
    /// The same, read from a file.
    WhyFile(&'a std::path::Path),
    /// The commitment of evidence published some other way.
    Hash(&'a str),
}

impl<'a> Evidence<'a> {
    pub fn of(
        why: Option<&'a str>,
        file: Option<&'a std::path::Path>,
        hash: Option<&'a str>,
    ) -> Self {
        match (why, file) {
            (Some(text), _) => Self::Why(text),
            (None, Some(path)) => Self::WhyFile(path),
            (None, None) => Self::Hash(hash.unwrap_or("")),
        }
    }
}

/// What `petri oracle sources challenge` asks for.
pub struct Request<'a> {
    pub source: &'a str,
    pub reason: u8,
    pub bond: Option<&'a str>,
    pub evidence: Evidence<'a>,
    pub comparison: Option<&'a str>,
    pub challenge_id: Option<&'a str>,
}

pub(super) fn iso(ts: u64) -> Value {
    chrono::DateTime::from_timestamp(i64::try_from(ts).unwrap_or(i64::MAX), 0)
        .map(|t| json!(t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()))
        .unwrap_or(Value::Null)
}

fn at(ts: u64) -> String {
    utc(i64::try_from(ts).unwrap_or(i64::MAX))
}

fn settle_command(month: &MonthRef, source: &Pubkey, challenge: &Pubkey) -> String {
    format!(
        "petri oracle stakes settle {} --kind source-challenge --source {source} --challenge {challenge}",
        month.period.expiry_id()
    )
}

/// A guard holds one challenge or council dispute of its source at a time.
fn busy(guard: Option<&OracleSourceChallengeGuard>) -> bool {
    guard.is_some_and(|g| {
        g.active_challenge != Pubkey::default()
            || g.active_challenge_id != [0; 32]
            || g.active_dispute != Pubkey::default()
    })
}

/// The program's rules on the challenged source and its comparison: a
/// candidate, not the challenger's own, free of another challenge or dispute;
/// a comparison is another such candidate of the same bucket.
fn check_sources(
    source: &SourceRef,
    comparison: Option<&SourceRef>,
    challenger: &Pubkey,
    guards: &[Option<OracleSourceChallengeGuard>],
) -> Result<()> {
    if source.state.status != OracleSourceStatus::Candidate {
        bail(format!(
            "source {} is {:?}: only a candidate source can be challenged",
            source.address, source.state.status
        ))?;
    }
    if source.state.proposer == *challenger {
        bail(format!(
            "you proposed source {}: its proposer can't challenge it",
            source.address
        ))?;
    }
    if let Some(c) = comparison {
        if c.address == source.address {
            bail("--comparison names the challenged source itself: name the source it duplicates")?;
        }
        if c.state.status != OracleSourceStatus::Candidate {
            bail(format!(
                "comparison source {} is {:?}: a duplicate is of another candidate source",
                c.address, c.state.status
            ))?;
        }
        if c.state.bucket_id != source.state.bucket_id {
            bail(format!(
                "comparison source {} is a {} source, not {}: a duplicate is of the same bucket",
                c.address,
                c.bucket_name(),
                source.bucket_name()
            ))?;
        }
    }
    for (s, guard) in [Some(source), comparison].into_iter().flatten().zip(guards) {
        let Some(g) = guard.as_ref() else {
            continue;
        };
        if g.active_dispute != Pubkey::default() {
            bail(format!(
                "source {} is in a council dispute ({}): it can't be challenged until that ends",
                s.address, g.active_dispute
            ))?;
        }
        if busy(Some(g)) {
            bail(format!(
                "source {} is already challenged (challenge {}), one challenge at a time: `petri oracle challenges` shows how it stands",
                s.address, g.active_challenge
            ))?;
        }
    }
    Ok(())
}

/// `petri oracle sources challenge`: tag 161, held to the program's rules
/// first and, with `--why`, after the reason's evidence object.
pub fn challenge(ctx: &Ctx, p: Paint, target: &Target, r: &Request) -> Result<Report> {
    send::run(
        ctx,
        p,
        challenge_write(ctx, target, r)?,
        target.send.simulate,
    )
}

fn challenge_write(ctx: &Ctx, target: &Target, r: &Request) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let challenger = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let id = month.period.expiry_id();
    let window = Window::of(&month.state);
    let now = u64::try_from(ctx.rpc.clock()?.unix_timestamp).unwrap_or(0);
    window.check(now, &id)?;
    let source = oracle.source(&month, r.source)?;
    let comparison = match (r.comparison, r.reason == DUPLICATE) {
        (Some(c), true) => Some(oracle.source(&month, c)?),
        (None, false) => None,
        (None, true) => bail(
            "a `duplicate` challenge names the source this one duplicates: give --comparison <SOURCE>",
        )?,
        (Some(_), false) => bail("--comparison is for a `duplicate` challenge only")?,
    };
    let pair: Vec<SourceRef> = [Some(source.clone()), comparison.clone()]
        .into_iter()
        .flatten()
        .collect();
    check_sources(
        &source,
        comparison.as_ref(),
        &challenger,
        &oracle.source_guards(&pair)?,
    )?;
    match oracle.reward_schedule(&month)? {
        Some(s) if s.phase == OracleUsdcRewardSchedulePhase::Funded => {}
        Some(s) => bail(format!(
            "{id}'s rewards are {:?}, not funded: it takes no challenges",
            s.phase
        ))?,
        None => bail(format!(
            "{id} has no reward schedule, so it takes no challenges"
        ))?,
    }
    let terms = Terms::of(&oracle.sku_pool(&month, &source.state.bucket_id)?);
    let bond = terms.bond(
        source.state.support_stake_total,
        comparison.as_ref().map(|c| c.state.support_stake_total),
    )?;
    if let Some(given) = r.bond
        && usdc_amount(given, "the bond")? != bond
    {
        let exact = trim_decimal(u128::from(bond), 6);
        bail(format!(
            "this challenge's bond is exactly {exact} USDC ({}): pass --bond {exact} or leave --bond out",
            terms.words()
        ))?;
    }
    let available = oracle
        .collateral(&challenger)?
        .map_or(0, |l| l.available_balance);
    if available < bond {
        bail(format!(
            "the bond is {} USDC and your oracle collateral has {} USDC available: deposit first (`petri oracle collateral deposit <USDC>`)",
            usdc(bond),
            usdc(available)
        ))?;
    }
    let challenge_id = id_or_random(r.challenge_id, "challenge id")?;
    let challenge = ix::find_oracle_source_challenge_address(
        &oracle.program,
        &month.address,
        &source.address,
        &challenge_id,
    );
    if ctx.rpc.account(&challenge)?.is_some() {
        bail(format!(
            "challenge id {} is taken (challenge {challenge}): leave --challenge-id out for a new one",
            hex(&challenge_id)
        ))?;
    }
    let why = match r.evidence {
        Evidence::Why(text) => Some(text.to_string()),
        Evidence::WhyFile(path) => Some(read_body(None, Some(path))?),
        Evidence::Hash(_) => None,
    };
    let mut txs = Vec::new();
    let (evidence_hash, object) = match (&why, r.evidence) {
        (Some(text), _) => {
            let (kind, bytes, hash) = evidence_preimage(EvidenceKind::Definition, text)?;
            let (object, written, _) = evidence_txs(&oracle, &challenger, kind, &bytes, hash)?;
            txs.extend(written);
            (hash, Some(object))
        }
        (None, Evidence::Hash(text)) => {
            let hash = parse_hex32(text, "evidence hash")?;
            if hash == [0; 32] {
                bail("the evidence hash can't be all zeros")?;
            }
            // Evidence the challenger sealed as a definition object (`evidence
            // write --kind definition`) reads back by its commitment.
            let object = ix::find_evidence_object_address(&oracle.program, &challenger, 2, &hash);
            (hash, ctx.rpc.account(&object)?.map(|_| object))
        }
        (None, _) => bail("give --why, --why-file or --evidence-hash")?,
    };
    txs.push(Tx::one(
        "challengeSource",
        ix::challenge_oracle_source_v2(
            &oracle.program,
            &challenger,
            &month.period.address,
            &month.address,
            &source.address,
            &source.state.bucket_id,
            comparison.as_ref().map(|c| &c.address),
            ChallengeOracleSourceParams {
                challenge_id,
                reason: r.reason,
                bond,
                evidence_hash,
                comparison_source_id: comparison
                    .as_ref()
                    .map(|c| c.state.source_id)
                    .unwrap_or([0; 32]),
            },
        ),
    ));
    let url = oracle
        .locators(std::slice::from_ref(&source))?
        .pop()
        .flatten();
    let settle = settle_command(&month, &source.address, &challenge);
    let mut review = vec![
        month_line(&month),
        format!(
            "Source {} · {} · {}",
            source.address,
            source.bucket_name(),
            url.as_deref()
                .map_or("(URL not on chain)".into(), terminal_safe_text)
        ),
    ];
    if let Some(c) = &comparison {
        review.push(format!("Duplicate of source {}", c.address));
    }
    review.extend([
        format!(
            "Reason: {}; challenge {challenge}",
            reason_label(r.reason)
        ),
        format!(
            "Bond {} USDC from your oracle collateral ({} available): {}",
            usdc(bond),
            usdc(available),
            terms.words()
        ),
        match &object {
            Some(object) if why.is_some() => format!(
                "Why: sealed in evidence object {object}, public and permanent (`petri oracle evidence show {object}`)"
            ),
            _ => format!("Evidence hash {}", hex(&evidence_hash)),
        },
        format!(
            "Ruled on Scramble day 7, {} to {}; unruled by then it counts as dismissed",
            at(window.closes),
            at(window.ruled_by)
        ),
        format!(
            "Upheld: your bond back plus the source's {} USDC listing bond; dismissed: your bond goes to its proposer",
            usdc(source.state.listing_bond_locked)
        ),
        format!("Once it is ruled: {settle}"),
    ]);
    Ok(Write {
        action: "oracle.sources.challenge",
        title: format!(
            "Challenge source {} of {id} ({}) with a {} USDC bond",
            source.address,
            reason_label(r.reason),
            usdc(bond)
        ),
        review,
        details: json!({
            "month": month_json(&month),
            "source": source_json(&source),
            "url": url,
            "comparison": comparison.as_ref().map(source_json),
            "challenge": challenge.to_string(),
            "challengeId": hex(&challenge_id),
            "reason": reason_name(r.reason),
            "reasonCode": r.reason,
            "bondAtoms": bond.to_string(),
            "evidenceHash": hex(&evidence_hash),
            "evidenceObject": object.map(|o| o.to_string()),
            "rulingFromUtc": iso(window.closes),
            "ruledByUtc": iso(window.ruled_by),
            "settleCommand": settle,
        }),
        txs,
    })
}

/// `petri oracle challenges [MARKET] [--owner]`: the owner's source
/// challenges (as the challenger, or as the challenged source's proposer),
/// how each stands or ended and what comes next; then, for each month in its
/// challenge window, the sources open to challenge now. Without `--owner` or
/// a wallet it lists every challenge.
pub fn list(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::reads(ctx)?;
    let owner = match owner {
        Some(text) => Some(ctx.owner(Some(text))?),
        None => ctx.wallet_pubkey().ok(),
    };
    let months = oracle.months_in(&oracle.periods_of(market, expiry)?)?;
    let month_of: HashMap<Pubkey, &MonthRef> = months.iter().map(|m| (m.address, m)).collect();
    let single = match months.as_slice() {
        [only] => Some(only.address),
        _ => None,
    };
    let now = u64::try_from(ctx.rpc.clock()?.unix_timestamp).unwrap_or(0);
    let (sources, mut challenges) = if months.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        (
            oracle.sources_in(single.as_ref())?,
            oracle.source_challenges(single.as_ref())?,
        )
    };
    let source_of: HashMap<Pubkey, &SourceRef> = sources.iter().map(|s| (s.address, s)).collect();
    challenges.retain(|(_, c)| {
        month_of.contains_key(&c.month)
            && owner.is_none_or(|o| {
                c.challenger == o
                    || source_of
                        .get(&c.source)
                        .is_some_and(|s| s.state.proposer == o)
            })
    });

    // The sources open to challenge: candidates with support in a month
    // whose window is open and rewards funded, free of any challenge, and
    // not the owner's own.
    let mut open_months: Vec<&MonthRef> = Vec::new();
    for m in &months {
        if Window::of(&m.state).is_open(now)
            && oracle
                .reward_schedule(m)?
                .is_some_and(|s| s.phase == OracleUsdcRewardSchedulePhase::Funded)
        {
            open_months.push(m);
        }
    }
    let in_window: HashSet<Pubkey> = open_months.iter().map(|m| m.address).collect();
    let candidates: Vec<SourceRef> = sources
        .iter()
        .filter(|s| {
            in_window.contains(&s.state.month)
                && s.state.status == OracleSourceStatus::Candidate
                && s.state.support_stake_total > 0
                && owner.is_none_or(|o| s.state.proposer != o)
        })
        .cloned()
        .collect();
    let guards = oracle.source_guards(&candidates)?;
    let mut open: Vec<SourceRef> = candidates
        .into_iter()
        .zip(guards)
        .filter(|(_, g)| !busy(g.as_ref()))
        .map(|(s, _)| s)
        .collect();
    open.sort_by_key(|s| (s.state.month, s.state.bucket_id, s.address));
    let mut terms: HashMap<(Pubkey, [u8; 32]), Option<Terms>> = HashMap::new();
    for s in &open {
        terms
            .entry((s.state.month, s.state.bucket_id))
            .or_insert_with(|| {
                let pool = oracle.sku_pool(month_of[&s.state.month], &s.state.bucket_id);
                pool.ok().map(|pool| Terms::of(&pool))
            });
    }

    // The URLs of every source listed, and each challenge's why when its
    // evidence is a definition object its challenger sealed.
    let listed: Vec<SourceRef> = challenges
        .iter()
        .filter_map(|(_, c)| source_of.get(&c.source).map(|s| (*s).clone()))
        .chain(open.iter().cloned())
        .collect();
    let urls: HashMap<Pubkey, String> = listed
        .iter()
        .zip(oracle.locators(&listed)?)
        .filter_map(|(s, url)| Some((s.address, url?)))
        .collect();
    let objects: Vec<Pubkey> = challenges
        .iter()
        .map(|(_, c)| {
            ix::find_evidence_object_address(&oracle.program, &c.challenger, 2, &c.evidence_hash)
        })
        .collect();
    let whys: Vec<Option<String>> = objects
        .iter()
        .zip(ctx.rpc.accounts(&objects)?)
        .zip(&challenges)
        .map(|((key, account), (_, c))| {
            definition_in(&oracle.program, key, account.as_ref()?, &c.evidence_hash)
        })
        .collect();

    let mut lines = vec![p.bold(&match owner {
        Some(o) => format!("Source challenges of {o}"),
        None => "Source challenges".to_string(),
    })];
    let mut table = Table::new(&[
        "Month", "Source", "Bucket", "Reason", "Bond", "Status", "Next", "URL",
    ])
    .right(&[4]);
    let mut notes = Vec::new();
    let mut items = Vec::new();
    for (((key, c), why), object) in challenges.iter().zip(&whys).zip(&objects) {
        let month = month_of[&c.month];
        let window = Window::of(&month.state);
        let source = source_of.get(&c.source);
        let standing = Standing::of(
            c.status,
            source.map(|s| s.state.status),
            now,
            window.ruled_by,
        );
        let settled = c.escrow_disposition != OracleEscrowDisposition::Unsettled;
        let settle = (!settled && standing.outcome().is_some())
            .then(|| settle_command(month, &c.source, key));
        let next = match standing {
            _ if settled => "-".to_string(),
            Standing::Waiting => format!("ruled by {}", at(window.ruled_by)),
            Standing::Council => "the council decides".to_string(),
            _ => "settle it (below)".to_string(),
        };
        let url = urls.get(&c.source);
        table.row(vec![
            month.period.expiry_id(),
            short(&c.source.to_string()),
            source.map_or("-".into(), |s| s.bucket_name()),
            reason_label(c.reason),
            usdc(c.bond),
            standing.words(settled).to_string(),
            next,
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        if let Some(why) = why {
            notes.push(format!(
                "  {} why: {}",
                short(&key.to_string()),
                terminal_safe_text(why)
            ));
        }
        if let Some(settle) = &settle {
            notes.push(format!("  {settle}"));
        }
        let role = owner.and_then(|o| {
            if c.challenger == o {
                Some("challenger")
            } else {
                source.filter(|s| s.state.proposer == o).map(|_| "proposer")
            }
        });
        items.push(json!({
            "month": month.period.expiry_id(),
            "monthAddress": month.address.to_string(),
            "challenge": key.to_string(),
            "challengeId": hex(&c.challenge_id),
            "role": role,
            "source": c.source.to_string(),
            "sourceId": hex(&c.source_id),
            "url": url,
            "bucket": source.map(|s| s.bucket_label()),
            "bucketName": source.map(|s| s.bucket_name()),
            "comparison": (c.comparison_source != Pubkey::default())
                .then(|| c.comparison_source.to_string()),
            "challenger": c.challenger.to_string(),
            "reason": reason_name(c.reason),
            "reasonCode": c.reason,
            "bondAtoms": c.bond.to_string(),
            "status": standing.code(settled),
            "statusText": standing.words(settled),
            "outcome": standing.outcome(),
            "paysTo": standing.pays(),
            "rulingFromUtc": iso(window.closes),
            "ruledByUtc": iso(window.ruled_by),
            "evidenceHash": hex(&c.evidence_hash),
            "evidenceObject": why.as_ref().map(|_| object.to_string()),
            "why": why,
            "settled": settled,
            "settleCommand": settle,
        }));
    }
    if table.is_empty() {
        lines.push("  none".to_string());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.extend(notes.iter().map(|n| p.dim(n)));
    }

    lines.push(String::new());
    lines.push(p.bold(&format!("Open to challenge now (chain clock {})", at(now))));
    let mut table = Table::new(&[
        "Month", "Source", "Bucket", "Support", "Bond", "Until", "URL",
    ])
    .right(&[3, 4]);
    let mut open_items = Vec::new();
    for s in &open {
        let month = month_of[&s.state.month];
        let window = Window::of(&month.state);
        let bond = terms[&(s.state.month, s.state.bucket_id)]
            .and_then(|t| t.bond(s.state.support_stake_total, None).ok());
        let url = urls.get(&s.address);
        table.row(vec![
            month.period.expiry_id(),
            short(&s.address.to_string()),
            s.bucket_name(),
            usdc(s.state.support_stake_total),
            bond.map_or("-".into(), usdc),
            at(window.closes),
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        open_items.push(json!({
            "month": month.period.expiry_id(),
            "monthAddress": month.address.to_string(),
            "windowClosesUtc": iso(window.closes),
            "ruledByUtc": iso(window.ruled_by),
            "source": s.address.to_string(),
            "sourceId": hex(&s.state.source_id),
            "url": url,
            "bucket": s.bucket_label(),
            "bucketName": s.bucket_name(),
            "supportAtoms": s.state.support_stake_total.to_string(),
            "bondAtoms": bond.map(|b| b.to_string()),
            "command": format!(
                "petri oracle sources challenge {} --source {} --reason <REASON> --why \"<TEXT>\"",
                month.period.expiry_id(),
                s.address
            ),
        }));
    }
    if open_months.is_empty() {
        lines
            .push("  No month is in its challenge window (Scramble days 5 and 6) now.".to_string());
    } else if table.is_empty() {
        let ids: Vec<String> = open_months.iter().map(|m| m.period.expiry_id()).collect();
        lines.push(format!(
            "  Nothing in {} is open to challenge: no other supported candidate free of a challenge.",
            ids.join(", ")
        ));
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.push(p.dim(
            "  petri oracle sources challenge <MONTH> --source <SOURCE> --reason <REASON> --why \"<TEXT>\"   (a duplicate's bond is on the larger support of the two)",
        ));
    }
    Ok(Report::new(
        json!({
            "ok": true,
            "owner": owner.map(|o| o.to_string()),
            "chainTimeUtc": iso(now),
            "challenges": items,
            "open": open_items,
        }),
        lines.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_parse_by_name_or_code_and_print_by_name() {
        for r in REASONS {
            assert_eq!(parse_reason(r.name), Ok(r.code));
            assert_eq!(parse_reason(&r.code.to_string()), Ok(r.code));
            assert_eq!(reason_name(r.code), r.name);
            assert!(REASON_HELP.contains(r.name), "{}", r.name);
        }
        assert_eq!(parse_reason(" Duplicate "), Ok(DUPLICATE));
        assert_eq!(parse_reason("8"), Ok(DUPLICATE));
        // A code outside the table is sent as is and printed as its number.
        assert_eq!(parse_reason("7"), Ok(7));
        assert_eq!(reason_name(7), "reason 7");
        assert_eq!(reason_label(0), "reason 0");
        assert_eq!(reason_label(3), "Stale observation");
        let refused = parse_reason("wrong").unwrap_err();
        assert!(
            refused.contains("wrong-component, unavailable"),
            "{refused}"
        );
        assert!(parse_reason("256").is_err());
        // One code per reason.
        let codes: HashSet<u8> = REASONS.iter().map(|r| r.code).collect();
        assert_eq!(codes.len(), REASONS.len());
    }

    const TERMS: Terms = Terms {
        bps: 2_000,
        minimum: 1_000_000,
        maximum: 10_000_000,
    };

    #[test]
    fn the_bond_is_a_share_of_the_backing_held_between_the_terms() {
        // 20% of 12.5 USDC.
        assert_eq!(TERMS.bond(12_500_000, None).unwrap(), 2_500_000);
        // Floored before the clamp.
        assert_eq!(TERMS.bond(12_500_003, None).unwrap(), 2_500_000);
        // At least the minimum, at most the maximum.
        assert_eq!(TERMS.bond(1, None).unwrap(), 1_000_000);
        assert_eq!(TERMS.bond(4_999_999, None).unwrap(), 1_000_000);
        assert_eq!(TERMS.bond(500_000_000, None).unwrap(), 10_000_000);
        assert_eq!(TERMS.bond(u64::MAX, None).unwrap(), 10_000_000);
        // A duplicate's bond is on the larger support of the two.
        assert_eq!(TERMS.bond(10_000_000, Some(25_000_000)).unwrap(), 5_000_000);
        assert_eq!(TERMS.bond(25_000_000, Some(10_000_000)).unwrap(), 5_000_000);
        assert_eq!(TERMS.bond(0, Some(25_000_000)).unwrap(), 5_000_000);
        // Nothing to back it: refused.
        let refused = |support, comparison| TERMS.bond(support, comparison).unwrap_err().message;
        assert_eq!(
            refused(0, None),
            "nobody supports this source yet, so it can't be challenged"
        );
        assert!(refused(0, Some(0)).contains("nobody supports either source"));
        for broken in [
            Terms {
                bps: 10_001,
                ..TERMS
            },
            Terms {
                minimum: 0,
                ..TERMS
            },
            Terms {
                maximum: 0,
                ..TERMS
            },
            Terms {
                minimum: 11_000_000,
                ..TERMS
            },
        ] {
            assert!(broken.bond(12_500_000, None).is_err(), "{broken:?}");
        }
        assert_eq!(
            TERMS.words(),
            "20% of the backing, at least 1.00 and at most 10.00 USDC"
        );
    }

    fn month(phase: OraclePhase, scramble_start_ts: u64) -> OracleMonthState {
        OracleMonthState {
            phase,
            scramble_start_ts,
            listing_ts: scramble_start_ts + 8 * 86_400,
            ..Default::default()
        }
    }

    #[test]
    fn challenges_are_taken_on_scramble_days_five_and_six() {
        let day = 86_400;
        // Scramble from 2026-10-18 00:00 UTC.
        let start = 1_792_281_600;
        let window = Window::of(&month(OraclePhase::Scramble, start));
        assert_eq!(
            (window.opens, window.closes, window.ruled_by),
            (start + 4 * day, start + 6 * day, start + 7 * day)
        );
        let fails = |w: Window, now: u64| w.check(now, "RAMX-202612").unwrap_err().message;
        // Before: when it opens.
        assert!(!window.is_open(start + 4 * day - 1));
        assert!(fails(window, start + day).contains(
            "opens at 2026-10-22 00:00 UTC (Scramble day 5) and closes at 2026-10-24 00:00 UTC"
        ));
        // Inside, from its first second to its last.
        for now in [start + 4 * day, start + 5 * day, start + 6 * day - 1] {
            assert!(window.check(now, "RAMX-202612").is_ok());
            assert!(window.is_open(now));
        }
        // After: when it closed.
        assert!(!window.is_open(start + 6 * day));
        assert!(fails(window, start + 6 * day).contains("closed at 2026-10-24 00:00 UTC"));
        // Other phases, and the recipe freeze.
        let submission = Window::of(&month(OraclePhase::SourceSubmission, start));
        assert!(fails(submission, start + 5 * day).contains("still taking sources"));
        let opening = Window::of(&month(OraclePhase::Opening, start));
        assert!(fails(opening, start + 5 * day).contains("is in Opening"));
        let freezing = Window::of(&OracleMonthState {
            weight_scheme_version: 255,
            ..month(OraclePhase::Scramble, start)
        });
        assert!(fails(freezing, start + 5 * day).contains("being frozen"));
    }

    #[test]
    fn a_challenge_stands_in_plain_words() {
        use OracleChallengeStatus as S;
        let (ruled_by, before, after) = (1_000, 999, 1_000);
        let words = |status, source, now, settled| {
            let s = Standing::of(status, source, now, ruled_by);
            (s.words(settled), s.code(settled), s.pays())
        };
        assert_eq!(
            words(S::RuleReview, None, before, false),
            ("Waiting for the ruling (day 7)", "waiting", None)
        );
        assert_eq!(
            words(S::RuleReview, None, after, false),
            (
                "Dismissed (not ruled in time)",
                "dismissed",
                Some("proposer")
            )
        );
        assert_eq!(
            words(S::RuleReviewUnresolved, None, after, false),
            ("With the council", "council", None)
        );
        assert_eq!(
            words(
                S::Accepted,
                Some(OracleSourceStatus::Rejected),
                after,
                false
            ),
            (
                "Upheld — source rejected",
                "upheld-rejected",
                Some("challenger")
            )
        );
        assert_eq!(
            words(S::Accepted, Some(OracleSourceStatus::Merged), after, false),
            ("Upheld — merged", "upheld-merged", Some("challenger"))
        );
        assert_eq!(
            words(S::Rejected, None, before, false),
            ("Dismissed", "dismissed", Some("proposer"))
        );
        assert_eq!(
            words(S::Cancelled, None, before, false),
            ("Cancelled", "cancelled", Some("challenger"))
        );
        assert_eq!(
            words(S::Rejected, None, after, true),
            ("Settled", "settled", Some("proposer"))
        );
        assert_eq!(Standing::Waiting.outcome(), None);
        assert_eq!(Standing::Lapsed.outcome(), Some("dismissed"));
    }

    #[test]
    fn a_guard_holds_one_challenge_or_dispute() {
        assert!(!busy(None));
        let free = OracleSourceChallengeGuard::default();
        assert!(!busy(Some(&free)));
        let challenged = OracleSourceChallengeGuard {
            active_challenge: Pubkey::new_unique(),
            active_challenge_id: [1; 32],
            ..free.clone()
        };
        assert!(busy(Some(&challenged)));
        let disputed = OracleSourceChallengeGuard {
            active_dispute: Pubkey::new_unique(),
            ..free
        };
        assert!(busy(Some(&disputed)));
    }

    #[test]
    fn a_challenge_needs_another_candidate_of_the_same_bucket() {
        let challenger = Pubkey::new_unique();
        let candidate = |bucket: u8| SourceRef {
            address: Pubkey::new_unique(),
            state: ameba_oracle::state::OracleSourceState {
                is_initialized: true,
                bucket_id: [bucket; 32],
                proposer: Pubkey::new_unique(),
                support_stake_total: 5_000_000,
                status: OracleSourceStatus::Candidate,
                ..Default::default()
            },
        };
        let source = candidate(1);
        let fails = |source: &SourceRef,
                     comparison: Option<&SourceRef>,
                     guards: &[Option<OracleSourceChallengeGuard>]| {
            check_sources(source, comparison, &challenger, guards)
                .unwrap_err()
                .message
        };
        assert!(check_sources(&source, None, &challenger, &[None]).is_ok());
        assert!(check_sources(&source, Some(&candidate(1)), &challenger, &[None, None]).is_ok());
        let mut frozen = source.clone();
        frozen.state.status = OracleSourceStatus::Frozen;
        assert!(fails(&frozen, None, &[None]).contains("is Frozen: only a candidate"));
        let mut own = source.clone();
        own.state.proposer = challenger;
        assert!(fails(&own, None, &[None]).contains("you proposed source"));
        assert!(fails(&source, Some(&source), &[None, None]).contains("itself"));
        assert!(fails(&source, Some(&candidate(2)), &[None, None]).contains("same bucket"));
        let mut rejected = candidate(1);
        rejected.state.status = OracleSourceStatus::Rejected;
        assert!(fails(&source, Some(&rejected), &[None, None]).contains("is Rejected"));
        let taken = OracleSourceChallengeGuard {
            active_challenge: Pubkey::new_unique(),
            active_challenge_id: [1; 32],
            ..Default::default()
        };
        assert!(fails(&source, None, &[Some(taken.clone())]).contains("already challenged"));
        // The comparison's guard counts as much as the source's.
        assert!(
            fails(&source, Some(&candidate(1)), &[None, Some(taken)])
                .contains("already challenged")
        );
    }

    #[test]
    fn evidence_is_the_why_text_or_a_given_hash() {
        let path = std::path::Path::new("why.txt");
        assert!(matches!(
            Evidence::of(Some("stale"), None, None),
            Evidence::Why("stale")
        ));
        assert!(matches!(
            Evidence::of(None, Some(path), None),
            Evidence::WhyFile(_)
        ));
        assert!(matches!(
            Evidence::of(None, None, Some("ab")),
            Evidence::Hash("ab")
        ));
        // The why text is sealed as a definition object: `evidence show`
        // reads it back.
        let (kind, bytes, hash) = evidence_preimage(EvidenceKind::Definition, "stale").unwrap();
        assert_eq!(kind, 2);
        assert_eq!(
            super::super::writes::definition_text(&bytes).as_deref(),
            Some("stale")
        );
        assert_ne!(hash, [0; 32]);
    }
}
