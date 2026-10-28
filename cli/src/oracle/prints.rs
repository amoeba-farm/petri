//! Opening prints (tags 162 to 165, and 81) and how each ends. In a month's
//! Opening, from the end of Scramble's seven days to listing on the chain's
//! clock, each frozen source takes one bonded opening print (162): its value,
//! cited with a Wayback archive or an evidence record of its page. For 300
//! slots after it lands anyone but its claimant can challenge it with an
//! alternative (163), bonding the product's challenge share of the print's
//! stake; after that anyone accepts an unchallenged one (165). The oracle
//! authority rules on a challenged one by hand on the opening day (164,
//! `futures-ops resolve-opening`), or leaves it to the council (30/18, kind
//! Opening); the bonds then settle (169, anyone may), which also gives a kept
//! or replaced print a new window. From listing, a frozen source without a
//! print is set aside (81). Nothing accepts, rules on or refunds a print still
//! open at listing.
//!
//! Petri holds each write to the program's rules before anything is sent, and
//! `petri oracle prints` lists a wallet's prints and challenges, the prints
//! open to challenge now and the sources still waiting for one, read from
//! chain.

use std::collections::{HashMap, HashSet};

use ameba_oracle::{
    constants::{ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS, ORACLE_SCRAMBLE_WINDOW_SECONDS},
    instruction::{self as ix, ChallengeOracleOpeningClaimParams, SubmitOracleOpeningClaimParams},
    state::{
        OracleChallengeStatus, OracleEscrowDisposition, OracleOpeningClaim,
        OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OraclePhase, OracleSourceState,
        OracleSourceStatus,
    },
};
use serde_json::{Value, json};
use solana_program::{hash::hashv, pubkey::Pubkey, sysvar};

use super::{
    args::Target,
    chain::{self, ClaimRef, MonthRef, Oracle, PrintLink, SourceRef},
    challenges::{Terms, iso},
    send::{self, Tx, Write},
    skus::hex,
    writes::{
        Cite, Cited, archive_parts, cite, evidence_preimage, evidence_txs, id_or_random,
        month_json, month_line, source_json, usdc_amount,
    },
};
use crate::{
    backend::terminal_safe_text,
    futures::{
        Report,
        ctx::Ctx,
        error::{Error, Result, bail},
        rpc::Account,
        units::{short, trim_decimal, usdc},
        view::{Paint, Table, utc},
    },
};

/// Solana's target slot length: the pace assumed while the current epoch is
/// too young to measure the chain's own.
const TARGET_SLOT_MS: u64 = 400;
/// Slots an epoch needs behind it before its pace is measured.
const MEASURED_FROM_SLOTS: u64 = 1_000;

/// The chain's clock and slot now, and its pace: chain-clock milliseconds a
/// slot, measured over the current epoch so far (the Clock sysvar's epoch
/// start time against the epoch's first slot). A print's challenge window is
/// counted in slots; its times are estimates at this pace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainNow {
    pub unix: u64,
    pub slot: u64,
    pub slot_ms: u64,
}

impl ChainNow {
    /// Read from the Clock and EpochSchedule sysvars (one batched read).
    pub fn read(ctx: &Ctx) -> Result<Self> {
        let accounts = ctx
            .rpc
            .accounts(&[sysvar::clock::ID, sysvar::epoch_schedule::ID])?;
        let clock = accounts[0]
            .as_ref()
            .ok_or_else(|| Error::msg("the cluster has no Clock sysvar"))?;
        Self::from_sysvars(&clock.data, accounts[1].as_ref().map(|a| a.data.as_slice()))
    }

    /// From the Clock sysvar's bytes (slot, epoch start time, epoch, leader
    /// schedule epoch, unix time) and the EpochSchedule's (slots per epoch,
    /// leader schedule offset, warm-up, first normal epoch, first normal slot).
    fn from_sysvars(clock: &[u8], schedule: Option<&[u8]>) -> Result<Self> {
        if clock.len() < 40 {
            bail("the Clock sysvar is too short")?;
        }
        let word = |d: &[u8], at: usize| u64::from_le_bytes(d[at..at + 8].try_into().unwrap());
        let signed = |at: usize| i64::from_le_bytes(clock[at..at + 8].try_into().unwrap());
        let slot = word(clock, 0);
        let (epoch_start, epoch, unix) = (signed(8), word(clock, 16), signed(32));
        let slot_ms = schedule
            .filter(|d| d.len() >= 33)
            .and_then(|d| {
                let first = first_slot(epoch, word(d, 0), word(d, 17), word(d, 25));
                let slots = slot
                    .checked_sub(first)
                    .filter(|s| *s >= MEASURED_FROM_SLOTS)?;
                let seconds = u64::try_from(unix - epoch_start).ok().filter(|s| *s > 0)?;
                Some((seconds.saturating_mul(1_000) / slots).max(1))
            })
            .unwrap_or(TARGET_SLOT_MS);
        Ok(Self {
            unix: u64::try_from(unix).unwrap_or(0),
            slot,
            slot_ms,
        })
    }

    /// When `slot` comes (or came) on the chain's clock, at its pace.
    pub fn time_of(&self, slot: u64) -> u64 {
        let ms = |slots: u64| slots.saturating_mul(self.slot_ms) / 1_000;
        if slot >= self.slot {
            self.unix.saturating_add(ms(slot - self.slot))
        } else {
            self.unix.saturating_sub(ms(self.slot - slot))
        }
    }
}

/// The first slot of `epoch` (Solana's `EpochSchedule::get_first_slot_in_epoch`):
/// warm-up epochs double from 32 slots, the rest have `slots_per_epoch` each.
fn first_slot(
    epoch: u64,
    slots_per_epoch: u64,
    first_normal_epoch: u64,
    first_normal_slot: u64,
) -> u64 {
    if epoch <= first_normal_epoch {
        2u64.saturating_pow(u32::try_from(epoch).unwrap_or(u32::MAX))
            .saturating_sub(1)
            .saturating_mul(32)
    } else {
        (epoch - first_normal_epoch)
            .saturating_mul(slots_per_epoch)
            .saturating_add(first_normal_slot)
    }
}

/// A time on the chain's clock, to the second (a window of 300 slots is a
/// couple of minutes).
fn at(ts: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(ts).unwrap_or(i64::MAX), 0)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// A month's Opening on the chain's clock: from the end of Scramble's seven
/// days to listing (the program's `ensure_oracle_opening_window`, for 162 to
/// 165). From listing until the month's expiry a frozen source without a
/// print is set aside (81, `ensure_oracle_game_transition_window`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    pub phase: OraclePhase,
    pub start: u64,
    pub listing: u64,
    pub expiry: u64,
}

impl Opening {
    pub fn of(month: &MonthRef) -> Self {
        Self {
            phase: month.state.phase,
            start: month
                .state
                .scramble_start_ts
                .saturating_add(ORACLE_SCRAMBLE_WINDOW_SECONDS),
            listing: month.state.listing_ts,
            expiry: month.period.period.expiry_ts,
        }
    }

    pub fn is_open(&self, now: u64) -> bool {
        self.phase == OraclePhase::Opening && now >= self.start && now < self.listing
    }

    /// Refuses unless the month is in Opening and the chain's clock between
    /// its start and listing, saying what `what` (taken, challenged,
    /// accepted) needs.
    pub fn check(&self, now: u64, what: &str) -> Result<()> {
        if self.phase != OraclePhase::Opening {
            bail(format!(
                "the month is in {:?}: {what} in Opening, from {} to listing at {}",
                self.phase,
                utc(self.start as i64),
                utc(self.listing as i64)
            ))?;
        }
        if now < self.start || now >= self.listing {
            bail(format!(
                "the Opening runs from {} to listing at {} on the chain's clock, which says {}",
                utc(self.start as i64),
                utc(self.listing as i64),
                utc(now as i64)
            ))?;
        }
        Ok(())
    }
}

/// The archive rules of an opening print or its alternative: captured from
/// the Opening's start up to the chain's clock, of the source's own page (the
/// program's `validate_oracle_opening_evidence_fields`).
fn check_archive(start: u64, now: u64, archive_url: &str, locator_hash: &[u8; 32]) -> Result<()> {
    let (captured, page) = archive_parts(archive_url)?;
    if captured < start {
        bail(format!(
            "the archive was captured at {}, before the Opening began at {}: archive the page again",
            utc(captured as i64),
            utc(start as i64)
        ))?;
    }
    if captured > now {
        bail(format!(
            "the archive was captured at {}, after the chain's clock ({}): use a capture from {} up to the chain's clock, or wait for the chain to pass {}",
            utc(captured as i64),
            utc(now as i64),
            utc(start as i64),
            utc(captured as i64)
        ))?;
    }
    if hashv(&[b"locator", page.as_bytes()]).to_bytes() != *locator_hash {
        bail(format!(
            "{page} is not this source's page: archive its URL exactly as the source names it"
        ))?;
    }
    Ok(())
}

/// What an opening must meet, by the oracle's rules, checked before anything
/// is sent (its archive goes on chain first): the month is in Opening and the
/// chain's clock (`now`) between the Opening's start and listing; the archive
/// is of the source's own page, captured from the Opening's start up to that
/// clock; the stake is the product's opening bond.
pub(super) fn check_opening(
    month: Opening,
    now: u64,
    archive_url: &str,
    locator_hash: &[u8; 32],
    stake: u64,
    bond: u64,
) -> Result<()> {
    month.check(now, "opening values are taken")?;
    check_archive(month.start, now, archive_url, locator_hash)?;
    if stake != bond {
        let bond = trim_decimal(u128::from(bond), 6);
        bail(format!(
            "the opening bond of this product is {bond} USDC: pass --stake {bond}, or leave --stake out"
        ))?;
    }
    Ok(())
}

/// Where an opening print stands, or (the same words) a challenge of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Waiting, in its challenge window: anyone but its claimant can
    /// challenge it.
    Challengeable,
    /// Waiting, its window passed unchallenged: anyone can accept it (165)
    /// until listing.
    Acceptable,
    /// Challenged: the oracle authority rules on it by hand, on the opening
    /// day (164).
    Challenged,
    /// The ruling left it to the council (30/18, kind Opening).
    Council,
    /// The ruling kept the print: once the challenge settles its bond goes to
    /// the claimant and the print takes a new challenge window.
    Kept,
    /// The challenger's alternative won: once the challenge settles it is the
    /// opening print (with a new window), the challenger its claimant, and
    /// the first claimant's stake goes to the challenger.
    Replaced,
    /// The print was rejected (the source takes another, or sits the month
    /// out): the challenger gets the bond back and the print's stake.
    Rejected,
    /// Accepted: the source's opening value. Its stake goes back to its
    /// claimant once settled.
    Accepted,
    /// Called off (a print timed out, its challenge cancelled): the stake and
    /// the bond go back. The program has the outcome; no opening path leads
    /// to it today.
    Cancelled,
    /// Still waiting or challenged at listing: from then nothing can accept,
    /// rule on or refund it, so its stake (and a challenge's bond) stays
    /// locked, and the month cannot enter Game.
    Expired,
}

impl Standing {
    /// A print's standing from its claim, the challenge that bears on it now
    /// (the unsettled one of its attempt), whether the Opening is open on the
    /// chain's clock, and the chain's slot.
    pub fn of_print(
        claim: &OracleOpeningClaim,
        active: Option<&OracleOpeningClaimChallenge>,
        open: bool,
        slot: u64,
    ) -> Self {
        use OracleOpeningClaimStatus as S;
        match claim.status {
            S::Accepted => Self::Accepted,
            S::Rejected => Self::Rejected,
            S::TimedOut => Self::Cancelled,
            S::Challenged => match active {
                Some(c) => Self::of_challenge(c, claim.status, open),
                None if open => Self::Challenged,
                None => Self::Expired,
            },
            S::Pending | S::Empty if !open => Self::Expired,
            S::Pending | S::Empty if slot < claim.challenge_deadline_slot => Self::Challengeable,
            S::Pending | S::Empty => Self::Acceptable,
        }
    }

    /// A challenge's standing from its status and money and the status of
    /// the claim it challenged (in its attempt: an older attempt ended
    /// rejected).
    pub fn of_challenge(
        c: &OracleOpeningClaimChallenge,
        claim: OracleOpeningClaimStatus,
        open: bool,
    ) -> Self {
        use OracleChallengeStatus as C;
        match c.status {
            C::Open | C::RuleReview if open => Self::Challenged,
            C::Open | C::RuleReview => Self::Expired,
            C::RuleReviewUnresolved => Self::Council,
            C::Rejected => Self::Kept,
            C::Accepted => match c.escrow_disposition {
                OracleEscrowDisposition::Transferred => Self::Replaced,
                OracleEscrowDisposition::Unsettled
                    if claim == OracleOpeningClaimStatus::Challenged =>
                {
                    Self::Replaced
                }
                _ => Self::Rejected,
            },
            C::Cancelled => Self::Cancelled,
        }
    }

    /// For JSON: `settled` once its money settled.
    pub fn code(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "settled",
            Self::Challengeable => "challengeable",
            Self::Acceptable => "acceptable",
            Self::Challenged => "challenged",
            Self::Council => "council",
            Self::Kept => "kept",
            Self::Replaced => "replaced",
            Self::Rejected => "rejected",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    pub fn words(self, settled: bool) -> &'static str {
        match (self, settled) {
            (Self::Accepted, true) => "Accepted; its stake went back to the claimant",
            (Self::Rejected, true) => {
                "Rejected; the challenger got the bond back and the print's stake"
            }
            (Self::Replaced, true) => {
                "Replaced by the challenger's value; the first claimant's stake went to the challenger"
            }
            (Self::Kept, true) => "Kept; the challenge bond went to the claimant",
            (Self::Cancelled, true) => "Called off; the stake and the bond went back",
            (_, true) => "Settled",
            (Self::Challengeable, _) => "Open to challenge",
            (Self::Acceptable, _) => "Unchallenged: anyone can accept it now",
            (Self::Challenged, _) => {
                "Challenged: waiting for the oracle authority's ruling, made by hand on the opening day"
            }
            (Self::Council, _) => "With the council",
            (Self::Kept, _) => {
                "Kept by the ruling: once the challenge settles its bond goes to the claimant, and the print can be challenged again"
            }
            (Self::Replaced, _) => {
                "The challenger's value won: once the challenge settles it is the opening print, and the first claimant's stake goes to the challenger"
            }
            (Self::Rejected, _) => {
                "Rejected: once the challenge settles, the challenger gets the bond back and the print's stake"
            }
            (Self::Accepted, _) => "Accepted: its stake can go back to the claimant",
            (Self::Cancelled, _) => "Called off: the stake and the bond go back once settled",
            (Self::Expired, _) => {
                "Not resolved before listing: nothing can accept, rule on or refund it now, so its money stays locked"
            }
        }
    }
}

/// The exact bond of a challenge of a print with `stake` (the program's
/// `calculate_oracle_usdc_challenge_bond` on the claim's stake).
pub fn challenge_bond(terms: &Terms, stake: u64) -> Result<u64> {
    terms.share(stake).ok_or_else(|| {
        Error::msg(
            "this product's challenge terms are not set, so its opening prints can't be challenged",
        )
    })
}

fn accept_command(month: &MonthRef, source: &Pubkey) -> String {
    format!(
        "petri oracle prints finalize {} --source {source}",
        month.period.expiry_id()
    )
}

fn claim_settle_command(month: &MonthRef, source: &Pubkey) -> String {
    format!(
        "petri oracle stakes settle {} --kind opening-claim --source {source}",
        month.period.expiry_id()
    )
}

fn challenge_settle_command(month: &MonthRef, source: &Pubkey, challenge: &Pubkey) -> String {
    format!(
        "petri oracle stakes settle {} --kind opening-challenge --source {source} --challenge {challenge}",
        month.period.expiry_id()
    )
}

/// What settles a print's money once it can: its own stake when accepted
/// (169, opening-claim), else the challenge that decided it (169,
/// opening-challenge, which also pays out a rejected or replaced print's
/// stake).
fn print_settle(
    standing: Standing,
    settled: bool,
    month: &MonthRef,
    source: &Pubkey,
    active: Option<&Pubkey>,
) -> Option<String> {
    match (standing, active) {
        _ if settled => None,
        (Standing::Accepted, _) | (Standing::Cancelled, None) => {
            Some(claim_settle_command(month, source))
        }
        (
            Standing::Kept | Standing::Replaced | Standing::Rejected | Standing::Cancelled,
            Some(c),
        ) => Some(challenge_settle_command(month, source, c)),
        _ => None,
    }
}

fn challenge_settle(
    standing: Standing,
    settled: bool,
    month: &MonthRef,
    source: &Pubkey,
    challenge: &Pubkey,
) -> Option<String> {
    (!settled
        && matches!(
            standing,
            Standing::Kept | Standing::Replaced | Standing::Rejected | Standing::Cancelled
        ))
    .then(|| challenge_settle_command(month, source, challenge))
}

/// The unsettled challenge of a claim's current attempt, if any: one at a
/// time, as the program takes them.
fn active<'a>(
    claim: &OracleOpeningClaim,
    of_claim: &[&'a (Pubkey, OracleOpeningClaimChallenge)],
) -> Option<&'a (Pubkey, OracleOpeningClaimChallenge)> {
    of_claim.iter().rev().copied().find(|(_, c)| {
        c.claim_attempt == claim.attempt
            && c.escrow_disposition == OracleEscrowDisposition::Unsettled
    })
}

/// The challenge a print's standing comes from: the unsettled one of its
/// attempt, else the last one of its attempt.
fn bearing<'a>(
    claim: &OracleOpeningClaim,
    of_claim: &[&'a (Pubkey, OracleOpeningClaimChallenge)],
) -> Option<&'a (Pubkey, OracleOpeningClaimChallenge)> {
    active(claim, of_claim).or_else(|| {
        of_claim
            .iter()
            .rev()
            .copied()
            .find(|(_, c)| c.claim_attempt == claim.attempt)
    })
}

/// The challenge that ended an earlier print of the claim, recorded by its
/// link: the first of that attempt whose alternative replaced it, else the
/// one that had it rejected.
fn ended_by<'a>(
    link: &PrintLink,
    of_claim: &[&'a (Pubkey, OracleOpeningClaimChallenge)],
) -> (Standing, Option<&'a (Pubkey, OracleOpeningClaimChallenge)>) {
    let of_attempt = || {
        of_claim
            .iter()
            .copied()
            .filter(|(_, c)| c.claim_attempt == link.attempt)
    };
    if let Some(c) =
        of_attempt().find(|(_, c)| c.escrow_disposition == OracleEscrowDisposition::Transferred)
    {
        return (Standing::Replaced, Some(c));
    }
    (
        Standing::Rejected,
        of_attempt().find(|(_, c)| c.status == OracleChallengeStatus::Accepted),
    )
}

/// Where a print's evidence text is read from.
#[derive(Clone, Copy, Debug)]
enum EvidenceAt {
    /// The archive object its payer wrote, committed to this hash.
    Payer(Pubkey, [u8; 32]),
    /// An object named by a print's link.
    Object(Pubkey),
}

/// The evidence objects read for a listing, by address.
struct Objects<'a> {
    program: &'a Pubkey,
    found: HashMap<Pubkey, Account>,
}

impl Objects<'_> {
    fn keys(program: &Pubkey, at: EvidenceAt) -> Vec<Pubkey> {
        match at {
            EvidenceAt::Payer(payer, hash) => {
                chain::archive_objects(program, &payer, &hash).to_vec()
            }
            EvidenceAt::Object(key) => vec![key],
        }
    }

    /// The payer and text of the evidence at `at`, when it is a sealed
    /// archive object.
    fn read(&self, at: EvidenceAt) -> Option<(Pubkey, String)> {
        Self::keys(self.program, at).iter().find_map(|key| {
            let account = self.found.get(key)?;
            let hash: [u8; 32] = match at {
                EvidenceAt::Payer(_, hash) => hash,
                EvidenceAt::Object(_) => account.data.get(39..71)?.try_into().ok()?,
            };
            chain::archive_in(self.program, key, account, &hash)
        })
    }

    fn text(&self, at: Option<EvidenceAt>) -> Option<String> {
        self.read(at?).map(|(_, text)| text)
    }
}

/// What a print says: its claimant, value, source time, stake (None until
/// its product's bond is read) and where its evidence is.
#[derive(Clone, Copy, Debug)]
struct Printed {
    claimant: Pubkey,
    value: u64,
    time: u64,
    stake: Option<u64>,
    evidence: EvidenceAt,
}

impl Printed {
    fn of(c: &OracleOpeningClaim) -> Self {
        Self {
            claimant: c.claimant,
            value: c.opening_state,
            time: c.source_time,
            stake: Some(c.stake),
            evidence: EvidenceAt::Payer(c.claimant, c.archive_url_hash),
        }
    }
}

/// A print as listed.
struct Row<'a> {
    month: &'a MonthRef,
    source: &'a SourceRef,
    claim: Pubkey,
    role: Option<&'static str>,
    /// What it says; None for an earlier attempt a claim no longer holds.
    print: Option<Printed>,
    standing: Standing,
    settled: bool,
    /// Its challenge deadline (slot), while it waits.
    deadline: Option<u64>,
    settle: Option<String>,
    challenge: Option<&'a (Pubkey, OracleOpeningClaimChallenge)>,
    /// The challenge's own standing and whether its bond settled.
    challenge_standing: Option<(Standing, bool)>,
    /// Its order: an earlier print of the same claim first.
    order: u64,
}

/// `petri oracle prints [MARKET] [--owner]`: the owner's opening prints (and
/// earlier ones a claim no longer holds), the owner's challenges of others'
/// prints, how each stands and what comes next; then the prints open to
/// challenge now, and the frozen sources still waiting for a print. Without
/// `--owner` or a wallet it lists every print.
pub fn list(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let program = oracle.program;
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
    let now = ChainNow::read(ctx)?;
    // One program scan of the sources (of every month, or of one), their
    // claims read by address, and one scan of the challenges when there are
    // claims.
    let (sources, claims, challenges) = if months.is_empty() {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        let sources: Vec<SourceRef> = oracle
            .sources_in(single.as_ref())?
            .into_iter()
            .filter(|s| month_of.contains_key(&s.state.month))
            .collect();
        let claims = oracle.opening_claims(&sources)?;
        // A challenge is of a claim: without claims there is none to scan for.
        let mut challenges = if claims.is_empty() {
            Vec::new()
        } else {
            oracle.opening_challenges(single.as_ref())?
        };
        challenges.retain(|(_, c)| month_of.contains_key(&c.month));
        (sources, claims, challenges)
    };
    let source_of: HashMap<Pubkey, &SourceRef> = sources.iter().map(|s| (s.address, s)).collect();
    let claim_of: HashMap<Pubkey, &ClaimRef> = claims.iter().map(|c| (c.address, c)).collect();
    let mut of_claim: HashMap<Pubkey, Vec<&(Pubkey, OracleOpeningClaimChallenge)>> = HashMap::new();
    for c in &challenges {
        of_claim.entry(c.1.claim).or_default().push(c);
    }
    let none = Vec::new();
    let challenges_of = |claim: &Pubkey| of_claim.get(claim).unwrap_or(&none).as_slice();
    let open_at = |month: &MonthRef| Opening::of(month).is_open(now.unix);

    let mut rows: Vec<Row> = Vec::new();
    // The owner's current prints (every print without an owner).
    for claim in &claims {
        let c = &claim.state;
        if owner.is_some_and(|o| c.claimant != o) {
            continue;
        }
        let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
        else {
            continue;
        };
        let open = open_at(month);
        let of = challenges_of(&claim.address);
        let now_active = active(c, of);
        let standing = Standing::of_print(c, now_active.map(|(_, a)| a), open, now.slot);
        let settled = c.escrow_disposition != OracleEscrowDisposition::Unsettled;
        let shown = bearing(c, of);
        rows.push(Row {
            month,
            source,
            claim: claim.address,
            role: owner.map(|_| "yours"),
            print: Some(Printed::of(c)),
            standing,
            settled,
            deadline: matches!(standing, Standing::Challengeable | Standing::Acceptable)
                .then_some(c.challenge_deadline_slot),
            settle: print_settle(
                standing,
                settled,
                month,
                &source.address,
                now_active.map(|(k, _)| k),
            ),
            challenge: shown,
            challenge_standing: shown.map(|(_, s)| {
                (
                    Standing::of_challenge(s, claim_status(c, s), open),
                    s.escrow_disposition != OracleEscrowDisposition::Unsettled,
                )
            }),
            order: u64::MAX,
        });
    }
    // The owner's challenges of others' prints (one that won made the print
    // the owner's, listed above).
    if let Some(o) = owner {
        for entry in &challenges {
            let (_, c) = entry;
            let Some(claim) = claim_of.get(&c.claim) else {
                continue;
            };
            let same = c.claim_attempt == claim.state.attempt;
            if c.challenger != o || (same && claim.state.claimant == o) {
                continue;
            }
            let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
            else {
                continue;
            };
            let open = open_at(month);
            let s = &claim.state;
            let challenge_standing = Standing::of_challenge(
                c,
                if same {
                    s.status
                } else {
                    OracleOpeningClaimStatus::Rejected
                },
                open,
            );
            let (standing, settled, print, deadline, settle) = if same {
                let now_active = active(s, challenges_of(&claim.address));
                let standing = Standing::of_print(s, now_active.map(|(_, a)| a), open, now.slot);
                let settled = s.escrow_disposition != OracleEscrowDisposition::Unsettled;
                (
                    standing,
                    settled,
                    Some(Printed::of(s)),
                    matches!(standing, Standing::Challengeable | Standing::Acceptable)
                        .then_some(s.challenge_deadline_slot),
                    print_settle(
                        standing,
                        settled,
                        month,
                        &source.address,
                        now_active.map(|(k, _)| k),
                    ),
                )
            } else {
                // The print it challenged was rejected and settled before the
                // source took another.
                (Standing::Rejected, true, None, None, None)
            };
            rows.push(Row {
                month,
                source,
                claim: claim.address,
                role: Some("challenging"),
                print,
                standing,
                settled,
                deadline,
                settle,
                challenge: Some(entry),
                challenge_standing: Some((
                    challenge_standing,
                    c.escrow_disposition != OracleEscrowDisposition::Unsettled,
                )),
                order: c.rule_review_slot,
            });
        }
    }

    // Earlier prints a claim no longer holds (replaced by a challenger's
    // value, or rejected before the source took another) survive only in
    // their evidence links: read those when there can be any.
    let links: Vec<PrintLink> = match owner {
        Some(_)
            if challenges
                .iter()
                .any(|(_, c)| c.escrow_disposition == OracleEscrowDisposition::Transferred)
                || claims.iter().any(|c| c.state.attempt > 1) =>
        {
            oracle
                .print_links(single.as_ref())?
                .into_iter()
                .filter(|l| {
                    claim_of.get(&l.claim).is_some_and(|c| {
                        !(c.state.attempt == l.attempt && c.state.evidence_hash == l.evidence_hash)
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    };

    // The prints open to challenge now: waiting in their window, in an open
    // Opening, not the owner's own.
    let mut open: Vec<(&MonthRef, &SourceRef, &ClaimRef)> = claims
        .iter()
        .filter_map(|claim| {
            let c = &claim.state;
            let month = month_of.get(&c.month)?;
            let source = source_of.get(&c.source)?;
            (open_at(month)
                && c.status == OracleOpeningClaimStatus::Pending
                && c.escrow_disposition == OracleEscrowDisposition::Unsettled
                && source.state.status == OracleSourceStatus::OpeningPending
                && now.slot < c.challenge_deadline_slot
                && owner.is_none_or(|o| c.claimant != o))
            .then_some((*month, *source, claim))
        })
        .collect();
    open.sort_by_key(|(m, s, _)| {
        (
            m.period.period.expiry_ts,
            m.address,
            s.state.bucket_id,
            s.address,
        )
    });

    // The sources still waiting for a print: frozen, without an opening, in
    // an open Opening, and free of a claim (or of a rejected one whose money
    // has settled, as a new print needs).
    let mut unclaimed: Vec<(&MonthRef, &SourceRef)> = sources
        .iter()
        .filter_map(|s| {
            let month = month_of.get(&s.state.month)?;
            let claim = ix::find_oracle_opening_claim_address(&program, &s.state.month, &s.address);
            (open_at(month) && printable(&s.state, claim_of.get(&claim).map(|c| &c.state)).is_ok())
                .then_some((*month, s))
        })
        .collect();
    unclaimed.sort_by_key(|(m, s)| {
        (
            m.period.period.expiry_ts,
            m.address,
            s.state.bucket_id,
            s.address,
        )
    });

    // Every evidence object, in one batch: the listed prints' and challenges'
    // archives, and the links' objects.
    let mut wanted: Vec<EvidenceAt> = Vec::new();
    for row in &rows {
        wanted.extend(row.print.map(|p| p.evidence));
        if let Some((_, c)) = row.challenge {
            wanted.push(EvidenceAt::Payer(c.challenger, c.archive_url_hash));
        }
    }
    for (_, _, claim) in &open {
        wanted.push(EvidenceAt::Payer(
            claim.state.claimant,
            claim.state.archive_url_hash,
        ));
    }
    wanted.extend(links.iter().map(|l| EvidenceAt::Object(l.object)));
    let keys: Vec<Pubkey> = {
        let mut seen = HashSet::new();
        wanted
            .iter()
            .flat_map(|at| Objects::keys(&program, *at))
            .filter(|k| seen.insert(*k))
            .collect()
    };
    let objects = Objects {
        program: &program,
        found: keys
            .iter()
            .copied()
            .zip(ctx.rpc.accounts(&keys)?)
            .filter_map(|(k, a)| Some((k, a?)))
            .collect(),
    };

    // The owner's earlier prints: links whose archive object the owner wrote.
    let mut history = Vec::new();
    if let Some(o) = owner {
        for link in &links {
            let at = EvidenceAt::Object(link.object);
            if objects.read(at).is_none_or(|(payer, _)| payer != o) {
                continue;
            }
            let (Some(&month), Some(&source)) =
                (month_of.get(&link.month), source_of.get(&link.source))
            else {
                continue;
            };
            let (standing, decided) = ended_by(link, challenges_of(&link.claim));
            history.push(Row {
                month,
                source,
                claim: link.claim,
                role: Some("yours"),
                print: Some(Printed {
                    claimant: o,
                    value: link.value,
                    time: link.time,
                    stake: None,
                    evidence: at,
                }),
                standing,
                settled: true,
                deadline: None,
                settle: None,
                challenge: decided,
                challenge_standing: decided.map(|(_, c)| {
                    (
                        Standing::of_challenge(c, OracleOpeningClaimStatus::Rejected, false),
                        c.escrow_disposition != OracleEscrowDisposition::Unsettled,
                    )
                }),
                order: link.attempt.into(),
            });
        }
    }

    // The products' terms: the opening bond (the stake of a waiting source's
    // print, and of an earlier print) and the challenge terms (the bond of an
    // open print's challenge), one batched read.
    let pairs: Vec<(Pubkey, [u8; 32])> = {
        let mut seen = HashSet::new();
        open.iter()
            .map(|(_, s, _)| *s)
            .chain(unclaimed.iter().map(|(_, s)| *s))
            .chain(history.iter().map(|r| r.source))
            .map(|s| (s.state.month, s.state.bucket_id))
            .filter(|pair| seen.insert(*pair))
            .collect()
    };
    let pools: HashMap<(Pubkey, [u8; 32]), _> = pairs
        .iter()
        .copied()
        .zip(oracle.sku_pools(&pairs)?)
        .filter_map(|(pair, pool)| Some((pair, pool?)))
        .collect();
    let pool_of = |s: &SourceRef| pools.get(&(s.state.month, s.state.bucket_id));
    for row in &mut history {
        if let Some(print) = row.print.as_mut() {
            print.stake = pool_of(row.source).map(|p| p.opening_bond);
        }
    }
    rows.extend(history);
    rows.sort_by_key(|r| {
        (
            r.month.period.period.expiry_ts,
            r.month.address,
            r.source.state.bucket_id,
            r.source.address,
            r.order,
        )
    });

    // A chip month's waiting sources take their print from an observation
    // (`petri oracle observe`): its coverage says which months are.
    let waiting_months: Vec<&MonthRef> = {
        let mut seen = HashSet::new();
        unclaimed
            .iter()
            .map(|(m, _)| *m)
            .filter(|m| seen.insert(m.address))
            .collect()
    };
    let mut chips: HashSet<Pubkey> = HashSet::new();
    for (month, coverage) in waiting_months
        .iter()
        .zip(oracle.coverages(&waiting_months)?)
    {
        let Some(coverage) = coverage else { continue };
        let of_month = || {
            Ok(sources
                .iter()
                .filter(|s| s.state.month == month.address)
                .cloned()
                .collect())
        };
        if chain::chip_of(&month.period.symbol, &coverage.required_sku_root, of_month)?.is_some() {
            chips.insert(month.address);
        }
    }

    // The URLs of every source listed.
    let listed: Vec<SourceRef> = {
        let mut seen = HashSet::new();
        rows.iter()
            .map(|r| r.source)
            .chain(open.iter().map(|(_, s, _)| *s))
            .chain(unclaimed.iter().map(|(_, s)| *s))
            .filter(|s| seen.insert(s.address))
            .cloned()
            .collect()
    };
    let urls: HashMap<Pubkey, String> = listed
        .iter()
        .zip(oracle.locators(&listed)?)
        .filter_map(|(s, url)| Some((s.address, url?)))
        .collect();

    let value = |atoms: u64| trim_decimal(u128::from(atoms), 6);
    let mut lines = vec![p.bold(&match owner {
        Some(o) => format!("Opening prints of {o}"),
        None => "Opening prints".to_string(),
    })];
    let mut table = Table::new(&[
        "Month", "Source", "Bucket", "Role", "Value", "Stake", "Status", "Next", "URL",
    ])
    .right(&[4, 5]);
    let mut notes = Vec::new();
    let mut items = Vec::new();
    for row in &rows {
        let opening = Opening::of(row.month);
        let url = urls.get(&row.source.address);
        let accept = (row.standing == Standing::Acceptable)
            .then(|| accept_command(row.month, &row.source.address));
        let until = row.deadline.map(|d| now.time_of(d));
        let next = match row.standing {
            _ if row.settled => "-".to_string(),
            Standing::Challengeable => format!("challengeable until ~{}", at(until.unwrap_or(0))),
            Standing::Acceptable => "accept it (below)".to_string(),
            Standing::Challenged => {
                format!("ruled by hand before listing, {}", at(opening.listing))
            }
            Standing::Council => "the council decides".to_string(),
            Standing::Expired => "nothing can".to_string(),
            _ if row.settle.is_some() => "settle it (below)".to_string(),
            _ => "-".to_string(),
        };
        let print = row.print;
        let (claimant, atoms, time) = (
            print.map(|p| p.claimant),
            print.map(|p| p.value),
            print.map(|p| p.time),
        );
        let (stake, evidence_at) = (print.and_then(|p| p.stake), print.map(|p| p.evidence));
        table.row(vec![
            row.month.period.expiry_id(),
            short(&row.source.address.to_string()),
            row.source.bucket_name(),
            row.role.unwrap_or("-").to_string(),
            atoms.map_or("-".into(), value),
            stake.map_or("-".into(), usdc),
            row.standing.words(row.settled).to_string(),
            next,
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        for command in accept.iter().chain(row.settle.iter()) {
            notes.push(format!("  {command}"));
        }
        let challenge = row.challenge.map(|(key, c)| {
            let (standing, settled) = row
                .challenge_standing
                .unwrap_or((Standing::Challenged, false));
            json!({
                "challenge": key.to_string(),
                "challengeId": hex(&c.challenge_id),
                "challenger": c.challenger.to_string(),
                "value": value(c.alternative_opening_state),
                "valueAtoms": c.alternative_opening_state.to_string(),
                "sourceTimeUtc": iso(c.alternative_source_time),
                "evidence": objects.text(Some(EvidenceAt::Payer(c.challenger, c.archive_url_hash))),
                "bondAtoms": c.bond.to_string(),
                "status": standing.code(settled),
                "statusText": standing.words(settled),
                "settleCommand": challenge_settle(standing, settled, row.month, &row.source.address, key),
                "settled": settled,
            })
        });
        items.push(json!({
            "month": row.month.period.expiry_id(),
            "monthAddress": row.month.address.to_string(),
            "claim": row.claim.to_string(),
            "source": row.source.address.to_string(),
            "sourceId": hex(&row.source.state.source_id),
            "url": url,
            "bucket": row.source.bucket_label(),
            "bucketName": row.source.bucket_name(),
            "role": row.role,
            "claimant": claimant.map(|c| c.to_string()),
            "value": atoms.map(value),
            "valueAtoms": atoms.map(|a| a.to_string()),
            "sourceTimeUtc": time.map(iso),
            "stakeAtoms": stake.map(|s| s.to_string()),
            "evidence": objects.text(evidence_at),
            "status": row.standing.code(row.settled),
            "statusText": row.standing.words(row.settled),
            "challengeDeadlineSlot": row.deadline,
            "challengeableUntilUtc": until.map(iso),
            "acceptFromUtc": until.map(iso),
            "openingClosesUtc": iso(opening.listing),
            "acceptCommand": accept,
            "settleCommand": row.settle,
            "settled": row.settled,
            "challenge": challenge,
        }));
    }
    if table.is_empty() {
        lines.push("  none".to_string());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.extend(notes.iter().map(|n| p.dim(n)));
    }

    lines.push(String::new());
    lines.push(p.bold(&format!(
        "Open to challenge now (chain clock {}, slot {})",
        at(now.unix),
        now.slot
    )));
    let mut table = Table::new(&[
        "Month", "Source", "Bucket", "Value", "Claimant", "Bond", "Until", "URL",
    ])
    .right(&[3, 5]);
    let mut open_items = Vec::new();
    for (month, source, claim) in &open {
        let c = &claim.state;
        let url = urls.get(&source.address);
        let bond = pool_of(source).and_then(|pool| Terms::of(pool).share(c.stake));
        let until = now.time_of(c.challenge_deadline_slot);
        table.row(vec![
            month.period.expiry_id(),
            short(&source.address.to_string()),
            source.bucket_name(),
            value(c.opening_state),
            short(&c.claimant.to_string()),
            bond.map_or("-".into(), usdc),
            format!("~{}", at(until)),
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        open_items.push(json!({
            "month": month.period.expiry_id(),
            "monthAddress": month.address.to_string(),
            "claim": claim.address.to_string(),
            "source": source.address.to_string(),
            "sourceId": hex(&source.state.source_id),
            "url": url,
            "bucket": source.bucket_label(),
            "bucketName": source.bucket_name(),
            "claimant": c.claimant.to_string(),
            "value": value(c.opening_state),
            "valueAtoms": c.opening_state.to_string(),
            "sourceTimeUtc": iso(c.source_time),
            "stakeAtoms": c.stake.to_string(),
            "evidence": objects.text(Some(EvidenceAt::Payer(c.claimant, c.archive_url_hash))),
            "challengeDeadlineSlot": c.challenge_deadline_slot,
            "challengeableUntilUtc": iso(until),
            "bondAtoms": bond.map(|b| b.to_string()),
            "command": format!(
                "petri oracle prints challenge {} --source {} --value <VALUE> --archive-url <ARCHIVE URL>",
                month.period.expiry_id(),
                source.address
            ),
        }));
    }
    let opening_months: Vec<String> = months
        .iter()
        .filter(|m| open_at(m))
        .map(|m| m.period.expiry_id())
        .collect();
    if opening_months.is_empty() {
        lines.push("  No month is in its Opening now.".to_string());
    } else if table.is_empty() {
        lines.push(format!(
            "  No print in {} is open to challenge: none of others' prints is inside its 300 slots.",
            opening_months.join(", ")
        ));
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.push(p.dim(
            "  petri oracle prints challenge <MONTH> --source <SOURCE> --value <VALUE> --archive-url <ARCHIVE URL>   (or --evidence <BUNDLE>; the bond is exact)",
        ));
    }

    lines.push(String::new());
    lines.push(p.bold("Waiting for an opening print"));
    let mut table = Table::new(&["Month", "Source", "Bucket", "Stake", "Until", "URL"]).right(&[3]);
    let mut waiting_items = Vec::new();
    for (month, source) in &unclaimed {
        let url = urls.get(&source.address);
        let stake = pool_of(source).map(|pool| pool.opening_bond);
        let listing = Opening::of(month).listing;
        let id = month.period.expiry_id();
        let chip = chips.contains(&month.address);
        table.row(vec![
            id.clone(),
            short(&source.address.to_string()),
            source.bucket_name(),
            stake.map_or("-".into(), usdc),
            at(listing),
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        waiting_items.push(json!({
            "month": id,
            "monthAddress": month.address.to_string(),
            "source": source.address.to_string(),
            "sourceId": hex(&source.state.source_id),
            "url": url,
            "bucket": source.bucket_label(),
            "bucketName": source.bucket_name(),
            "stakeAtoms": stake.map(|s| s.to_string()),
            "openingClosesUtc": iso(listing),
            "command": if chip {
                format!("petri oracle observe {id} --source {} --file OBS.json", source.address)
            } else {
                format!(
                    "petri oracle prints opening {id} --source {} --value <VALUE> --archive-url <ARCHIVE URL>",
                    source.address
                )
            },
        }));
    }
    if opening_months.is_empty() {
        lines.push("  No month is in its Opening now.".to_string());
    } else if table.is_empty() {
        lines.push(format!(
            "  Every frozen source of {} has a print or is set aside.",
            opening_months.join(", ")
        ));
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.push(p.dim(
            "  petri oracle prints opening <MONTH> --source <SOURCE> --value <VALUE> --archive-url <ARCHIVE URL>   (a chip month: petri oracle observe <MONTH> --source <SOURCE> --file OBS.json)",
        ));
    }
    Ok(Report::new(
        json!({
            "ok": true,
            "owner": owner.map(|o| o.to_string()),
            "chainTimeUtc": iso(now.unix),
            "chainSlot": now.slot,
            "slotMs": now.slot_ms,
            "prints": items,
            "open": open_items,
            "unclaimed": waiting_items,
        }),
        lines.join("\n"),
    ))
}

/// The status of the claim a challenge challenged, as its own attempt saw
/// it: an older attempt ended rejected.
fn claim_status(
    claim: &OracleOpeningClaim,
    challenge: &OracleOpeningClaimChallenge,
) -> OracleOpeningClaimStatus {
    if challenge.claim_attempt == claim.attempt {
        claim.status
    } else {
        OracleOpeningClaimStatus::Rejected
    }
}

/// The program's rules on a new print of a source (162): frozen, without an
/// opening, and free of a claim, or of a rejected one whose money settled.
fn printable(source: &OracleSourceState, claim: Option<&OracleOpeningClaim>) -> Result<()> {
    match source.status {
        OracleSourceStatus::Frozen if !source.opening_submitted => {}
        OracleSourceStatus::OpeningPending => bail(
            "this source has an opening print waiting already: `petri oracle prints` shows how it stands",
        )?,
        OracleSourceStatus::Active => bail("this source has its opening value already")?,
        OracleSourceStatus::Inactive => bail("this source is set aside for the month")?,
        status => bail(format!(
            "this source is {status:?}: only a frozen source of the recipe takes an opening print"
        ))?,
    }
    if let Some(c) = claim
        && (c.status != OracleOpeningClaimStatus::Rejected
            || c.escrow_disposition == OracleEscrowDisposition::Unsettled)
    {
        bail(
            "this source's last print was rejected and its challenge has not settled yet: settle it first (`petri oracle prints` names the command)",
        )?;
    }
    Ok(())
}

// --------------------------------------------------------------- the writes

/// `petri oracle prints opening`: tag 162, the stake the product's opening
/// bond when `--stake` is left out; with `accept`, accepted once its window
/// passes ([`accept_after`]).
#[allow(clippy::too_many_arguments)]
pub fn print_opening(
    ctx: &Ctx,
    p: Paint,
    target: &Target,
    source: &str,
    value: Option<&str>,
    cited: Cite,
    stake: Option<&str>,
    accept: bool,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let actor = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let source = oracle.source(&month, source)?;
    let c = cite(ctx, &oracle, &month, &source, cited, value, &actor)?;
    let stake = stake.map(|s| usdc_amount(s, "the stake")).transpose()?;
    let mut write = opening_write(&oracle, &month, &source, &c, stake, target.send.simulate)?;
    if accept {
        write.review.push(accept_note());
    }
    let mut report = send::run(ctx, p, write, target.send.simulate)?;
    if accept {
        accept_after(ctx, p, &mut report, &month, &source, target.send.simulate);
    }
    Ok(report)
}

/// What `--accept` adds to a print's review.
pub(super) fn accept_note() -> String {
    format!(
        "Then waits out its {ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS} slots and accepts it, unless someone challenges it first"
    )
}

/// How long to wait before looking at the chain again: until the slot
/// comes, at the chain's pace, but at least 0.4 s and at most 5 s between
/// looks (a print's window is a minute or two).
fn pause_until(now: &ChainNow, slot: u64) -> std::time::Duration {
    let ms = slot.saturating_sub(now.slot).saturating_mul(now.slot_ms);
    std::time::Duration::from_millis(ms.clamp(400, 5_000))
}

/// With `--accept`, once a print landed: waits until its challenge window
/// passes (reading the clock and the claim every few seconds), then accepts
/// it (tag 165), unless it was challenged, accepted or rejected meanwhile,
/// or listing came first. The print stands whatever happens here: the
/// result's details say whether it was accepted (`accepted`, with
/// `acceptSignature`), and if not why (`acceptStatus`, `acceptText`) and
/// what comes next (`acceptCommand`).
pub(super) fn accept_after(
    ctx: &Ctx,
    p: Paint,
    report: &mut Report,
    month: &MonthRef,
    source: &SourceRef,
    simulate: bool,
) {
    let accept = accept_command(month, &source.address);
    let set = |report: &mut Report,
               accepted: bool,
               status: &str,
               text: String,
               signature: Option<String>| {
        let d = &mut report.payload["details"];
        d["accepted"] = json!(accepted);
        d["acceptStatus"] = json!(status);
        d["acceptText"] = json!(text);
        d["acceptSignature"] = json!(signature);
        d["acceptCommand"] = if accepted { Value::Null } else { json!(accept) };
        report.plain.push_str(&format!("\n{text}"));
    };
    if simulate {
        set(
            report,
            false,
            "not-simulated",
            format!(
                "With --accept, petri accepts it once its {ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS} slots pass (not simulated: the print is not on chain yet)"
            ),
            None,
        );
        return;
    }
    let waited = (|| -> Result<std::result::Result<(), (String, String)>> {
        let oracle = Oracle::new(ctx)?;
        let opening = Opening::of(month);
        loop {
            let now = ChainNow::read(ctx)?;
            let claim = oracle.opening_claim(source)?;
            let Some(claim) = claim else {
                return Ok(Err((
                    "missing".into(),
                    "The print is not on chain: nothing to accept".into(),
                )));
            };
            let c = &claim.state;
            match c.status {
                OracleOpeningClaimStatus::Pending => {}
                OracleOpeningClaimStatus::Challenged => {
                    return Ok(Err((
                        "challenged".into(),
                        "It was challenged before its window passed, so petri did not accept it: the oracle authority rules on it by hand on the opening day. If the ruling keeps it, settle the challenge (its bond comes to you) and accept it after its new window; `petri oracle prints` shows how it stands".into(),
                    )));
                }
                OracleOpeningClaimStatus::Accepted => return Ok(Ok(())),
                status => {
                    return Ok(Err((
                        "not-waiting".into(),
                        format!("It is {status:?} now, so there is nothing to accept"),
                    )));
                }
            }
            if now.unix >= opening.listing {
                return Ok(Err((
                    "too-late".into(),
                    format!(
                        "Listing came at {} before its window passed: it can't be accepted, and nothing returns its stake",
                        at(opening.listing)
                    ),
                )));
            }
            if now.slot >= c.challenge_deadline_slot {
                return Ok(Ok(()));
            }
            std::thread::sleep(pause_until(&now, c.challenge_deadline_slot));
        }
    })();
    match waited {
        Ok(Err((status, text))) => set(report, false, &status, text, None),
        Err(e) => set(
            report,
            false,
            "failed",
            format!(
                "Waiting to accept it failed ({}): the print stands; accept it once its window passes: {accept}",
                e.message
            ),
            None,
        ),
        Ok(Ok(())) => {
            let sent = Oracle::new(ctx).and_then(|oracle| {
                // Accepted by someone else meanwhile: nothing to send.
                if oracle
                    .opening_claim(source)?
                    .is_some_and(|c| c.state.status == OracleOpeningClaimStatus::Accepted)
                {
                    return Ok(None);
                }
                let write = finalize_write_of(&oracle, month, source)?;
                send::run_confirmed(ctx, p, write).map(Some)
            });
            match sent {
                Ok(None) => set(
                    report,
                    true,
                    "accepted",
                    "It was accepted already".into(),
                    None,
                ),
                Ok(Some(done)) => {
                    let signature = done.payload["transactions"][0]["signature"]
                        .as_str()
                        .map(str::to_string);
                    let settle = claim_settle_command(month, &source.address);
                    report.payload["details"]["settleCommand"] = json!(settle);
                    report.payload["accept"] = done.payload.clone();
                    set(
                        report,
                        true,
                        "accepted",
                        format!(
                            "{}\nAccepted: it is the source's opening value. Its stake comes back with: {settle}",
                            done.plain
                        ),
                        signature,
                    );
                }
                Err(e) => set(
                    report,
                    false,
                    "failed",
                    format!(
                        "Accepting it failed ({}): the print stands; try again: {accept}",
                        e.message.replace('\n', " | ")
                    ),
                    None,
                ),
            }
        }
    }
}

/// The URL of one source, read from its locator.
fn url_of(oracle: &Oracle, source: &SourceRef) -> Result<Option<String>> {
    Ok(oracle
        .locators(std::slice::from_ref(source))?
        .pop()
        .flatten())
}

fn collateral_check(oracle: &Oracle, owner: &Pubkey, amount: u64, what: &str) -> Result<()> {
    let available = oracle.collateral(owner)?.map_or(0, |l| l.available_balance);
    if available < amount {
        bail(format!(
            "{what} is {} USDC and your oracle collateral has {} USDC available: deposit first (`petri oracle collateral deposit {}`)",
            usdc(amount),
            usdc(available),
            trim_decimal(u128::from(amount - available), 6)
        ))?;
    }
    Ok(())
}

/// An opening print to review and send (`prints opening`, and `observe` in
/// Opening): held to the oracle's rules first ([`check_opening`], the source
/// free for a print, the stake in collateral, accepted in time), then its
/// evidence (a Wayback archive, or a web proof's evidence record, published
/// unless only simulating) and the claim. The stake is the product's opening
/// bond when None.
pub(super) fn opening_write(
    oracle: &Oracle,
    month: &MonthRef,
    source: &SourceRef,
    c: &Cited,
    stake: Option<u64>,
    simulate: bool,
) -> Result<Write> {
    let ctx = oracle.ctx;
    let actor = ctx.wallet_pubkey()?;
    let (state, source_time, archive_url) = (c.state, c.time, c.text.as_str());
    let value = trim_decimal(u128::from(state), 6);
    let id = month.period.expiry_id();
    let bond = oracle
        .sku_pool(month, &source.state.bucket_id)?
        .opening_bond;
    let stake = stake.unwrap_or(bond);
    let now = ChainNow::read(ctx)?;
    let opening = Opening::of(month);
    check_opening(
        opening,
        now.unix,
        archive_url,
        &source.state.canonical_locator_hash,
        stake,
        bond,
    )?;
    printable(
        &source.state,
        oracle.opening_claim(source)?.as_ref().map(|c| &c.state),
    )
    .map_err(|e| Error::msg(format!("source {}: {}", source.address, e.message)))?;
    // A print still waiting at listing can never be accepted or refunded, so
    // its stake would be lost (a protection against lost funds, not a limit
    // of the oracle's): refuse one whose window would end after listing.
    let deadline = now
        .slot
        .saturating_add(ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS);
    let accept_from = now.time_of(deadline);
    if accept_from >= opening.listing {
        bail(format!(
            "a print sent now can be accepted from slot {deadline}, about {} at this chain's pace ({} ms a slot), but {id} lists at {}: it could not be accepted in time, and nothing would return its stake",
            at(accept_from),
            now.slot_ms,
            at(opening.listing)
        ))?;
    }
    collateral_check(oracle, &actor, stake, "the stake")?;
    let (kind, bytes, hash) = evidence_preimage(c.kind, archive_url)?;
    let (archive_object, mut txs, _) = evidence_txs(oracle, &actor, kind, &bytes, hash)?;
    if let Some(digest) = &c.bundle
        && !simulate
    {
        // Anyone checking the print fetches the proof by its digest.
        crate::evidence::onchain::publish(ctx, digest)?;
    }
    txs.push(Tx::one(
        "submitOpening",
        ix::submit_oracle_opening_claim_v2(
            &oracle.program,
            &actor,
            &actor,
            &month.period.address,
            &month.address,
            &source.address,
            &source.state.bucket_id,
            SubmitOracleOpeningClaimParams {
                opening_state: state,
                source_time,
                stake,
                canonical_locator_hash: source.state.canonical_locator_hash,
                source_definition_hash: source.state.source_definition_hash,
                archive_url: String::new(),
            },
            archive_url,
        ),
    ));
    let claim =
        ix::find_oracle_opening_claim_address(&oracle.program, &month.address, &source.address);
    let url = url_of(oracle, source)?;
    let accept = accept_command(month, &source.address);
    Ok(Write {
        action: "oracle.prints.opening",
        title: format!(
            "Submit opening print {value} for source {} ({} USDC stake)",
            source.address,
            usdc(stake)
        ),
        review: vec![
            month_line(month),
            format!(
                "Source {} · {} · {}",
                source.address,
                source.bucket_name(),
                url.as_deref()
                    .map_or("(URL not on chain)".into(), terminal_safe_text)
            ),
            format!("Archive {archive_url} (object {archive_object})"),
            format!(
                "Stake {} USDC from your oracle collateral, the product's opening bond: it comes back once the print is accepted, and goes to a challenger who wins",
                usdc(stake)
            ),
            format!(
                "Open to challenge for {ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS} slots once it lands (to about {}); then anyone accepts it, before listing at {}: {accept}",
                at(accept_from),
                at(opening.listing)
            ),
        ],
        details: json!({
            "month": month_json(month),
            "source": source_json(source),
            "url": url,
            "bucketName": source.bucket_name(),
            "claim": claim.to_string(),
            "value": value,
            "valueAtoms": state.to_string(),
            "sourceTime": source_time,
            "sourceTimeUtc": iso(source_time),
            "stakeAtoms": stake.to_string(),
            "evidence": archive_url,
            "evidenceObject": archive_object.to_string(),
            "challengeDeadlineSlot": deadline,
            "challengeableUntilUtc": iso(accept_from),
            "acceptFromUtc": iso(accept_from),
            "openingClosesUtc": iso(opening.listing),
            "acceptCommand": accept,
            "nextCommand": accept,
            "nextFromUtc": iso(accept_from),
        }),
        txs,
    })
}

/// The program's rules on challenging a print (163), checked before anything
/// is sent: a waiting print, its money unsettled, not the challenger's own,
/// inside its window.
fn check_challengeable(
    claim: &OracleOpeningClaim,
    source: &SourceRef,
    challenger: &Pubkey,
    now: &ChainNow,
    accept: &str,
) -> Result<()> {
    use OracleOpeningClaimStatus as S;
    let key = source.address;
    match claim.status {
        S::Pending => {}
        S::Challenged => bail(format!(
            "the opening print of source {key} is challenged already, and takes one challenge at a time: `petri oracle prints` shows how it stands"
        ))?,
        S::Accepted => bail(format!(
            "the opening print of source {key} is accepted: it can't be challenged any more"
        ))?,
        S::Rejected => bail(format!(
            "the opening print of source {key} was rejected: there is nothing to challenge until the source takes another"
        ))?,
        status => bail(format!(
            "the opening print of source {key} is {status:?}: it can't be challenged"
        ))?,
    }
    if claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || source.state.status != OracleSourceStatus::OpeningPending
    {
        bail(format!(
            "source {key} is {:?}, not waiting on its opening print",
            source.state.status
        ))?;
    }
    if claim.claimant == *challenger {
        bail(format!(
            "you made the opening print of source {key}: its claimant can't challenge it"
        ))?;
    }
    if now.slot >= claim.challenge_deadline_slot {
        bail(format!(
            "the challenge window of this print closed at slot {} (about {}; the chain is at slot {}): it can only be accepted now ({accept})",
            claim.challenge_deadline_slot,
            at(now.time_of(claim.challenge_deadline_slot)),
            now.slot
        ))?;
    }
    Ok(())
}

/// What `petri oracle prints challenge` asks for.
pub struct ChallengeRequest<'a> {
    pub source: &'a str,
    pub value: Option<&'a str>,
    pub cited: Cite<'a>,
    pub bond: Option<&'a str>,
    pub challenge_id: Option<&'a str>,
}

/// `petri oracle prints challenge`: tag 163, held to the program's rules
/// first.
pub fn challenge(ctx: &Ctx, p: Paint, target: &Target, r: &ChallengeRequest) -> Result<Report> {
    send::run(
        ctx,
        p,
        challenge_write(ctx, target, r)?,
        target.send.simulate,
    )
}

fn challenge_write(ctx: &Ctx, target: &Target, r: &ChallengeRequest) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let challenger = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let id = month.period.expiry_id();
    let opening = Opening::of(&month);
    let now = ChainNow::read(ctx)?;
    opening.check(now.unix, "opening prints are challenged")?;
    let source = oracle.source(&month, r.source)?;
    let accept = accept_command(&month, &source.address);
    let Some(claim) = oracle.opening_claim(&source)? else {
        return bail(format!(
            "source {} of {id} has no opening print to challenge: `petri oracle prints {id}` lists the prints open to challenge",
            source.address
        ));
    };
    let s = &claim.state;
    check_challengeable(s, &source, &challenger, &now, &accept)?;
    let c = cite(ctx, &oracle, &month, &source, r.cited, r.value, &challenger)?;
    check_archive(
        opening.start,
        now.unix,
        &c.text,
        &source.state.canonical_locator_hash,
    )?;
    if c.state == s.opening_state
        && c.time == s.source_time
        && ix::archive_url_hash(&c.text) == s.archive_url_hash
    {
        bail(
            "the alternative is the print itself (the same value, time and archive): a challenge offers another",
        )?;
    }
    let terms = Terms::of(&oracle.sku_pool(&month, &source.state.bucket_id)?);
    let bond = challenge_bond(&terms, s.stake)?;
    let words = terms.words_of("the print's stake");
    if let Some(given) = r.bond
        && usdc_amount(given, "the bond")? != bond
    {
        let exact = trim_decimal(u128::from(bond), 6);
        bail(format!(
            "this challenge's bond is exactly {exact} USDC ({words}): pass --bond {exact} or leave --bond out"
        ))?;
    }
    collateral_check(&oracle, &challenger, bond, "the bond")?;
    let challenge_id = id_or_random(r.challenge_id, "challenge id")?;
    if challenge_id == [0; 32] {
        bail("the challenge id can't be all zeros")?;
    }
    let challenge = ix::find_oracle_opening_challenge_address(
        &oracle.program,
        &month.address,
        &claim.address,
        &challenge_id,
    );
    if ctx.rpc.account(&challenge)?.is_some() {
        bail(format!(
            "challenge id {} is taken (challenge {challenge}): leave --challenge-id out for a new one",
            hex(&challenge_id)
        ))?;
    }
    let (kind, bytes, hash) = evidence_preimage(c.kind, &c.text)?;
    let (archive_object, mut txs, _) = evidence_txs(&oracle, &challenger, kind, &bytes, hash)?;
    if let Some(digest) = &c.bundle
        && !target.send.simulate
    {
        crate::evidence::onchain::publish(ctx, digest)?;
    }
    txs.push(Tx::one(
        "challengeOpening",
        ix::challenge_oracle_opening_claim_v2(
            &oracle.program,
            &challenger,
            &challenger,
            &month.period.address,
            &month.address,
            &source.address,
            &source.state.bucket_id,
            ChallengeOracleOpeningClaimParams {
                challenge_id,
                alternative_opening_state: c.state,
                alternative_source_time: c.time,
                bond,
                canonical_locator_hash: source.state.canonical_locator_hash,
                source_definition_hash: source.state.source_definition_hash,
                archive_url: String::new(),
            },
            &c.text,
        ),
    ));
    let url = url_of(&oracle, &source)?;
    let value = trim_decimal(u128::from(c.state), 6);
    let printed = trim_decimal(u128::from(s.opening_state), 6);
    let until = now.time_of(s.challenge_deadline_slot);
    let settle = challenge_settle_command(&month, &source.address, &challenge);
    Ok(Write {
        action: "oracle.prints.challenge",
        title: format!(
            "Challenge the opening print {printed} of source {} with {value} ({} USDC bond)",
            source.address,
            usdc(bond)
        ),
        review: vec![
            month_line(&month),
            format!(
                "Source {} · {} · {}",
                source.address,
                source.bucket_name(),
                url.as_deref()
                    .map_or("(URL not on chain)".into(), terminal_safe_text)
            ),
            format!(
                "The print: {printed} by {}, stake {} USDC; its window closes at slot {} (about {}, {} slots from now)",
                s.claimant,
                usdc(s.stake),
                s.challenge_deadline_slot,
                at(until),
                s.challenge_deadline_slot.saturating_sub(now.slot)
            ),
            format!(
                "Your alternative: {value}, archive {} (object {archive_object})",
                c.text
            ),
            format!(
                "Bond {} USDC from your oracle collateral: {words}; challenge {challenge}",
                usdc(bond)
            ),
            format!(
                "The oracle authority rules on it by hand on the opening day, before listing at {}; still unruled at listing, the print's stake and your bond stay locked",
                at(opening.listing)
            ),
            format!(
                "Upheld: your bond back plus the print's {} USDC stake (your value becomes the opening print if the ruling takes it); kept: your bond goes to the claimant",
                usdc(s.stake)
            ),
            format!("Once it is ruled: {settle}"),
        ],
        details: json!({
            "month": month_json(&month),
            "source": source_json(&source),
            "url": url,
            "bucketName": source.bucket_name(),
            "claim": claim.address.to_string(),
            "claimant": s.claimant.to_string(),
            "printValue": printed,
            "printValueAtoms": s.opening_state.to_string(),
            "stakeAtoms": s.stake.to_string(),
            "challenge": challenge.to_string(),
            "challengeId": hex(&challenge_id),
            "value": value,
            "valueAtoms": c.state.to_string(),
            "sourceTime": c.time,
            "sourceTimeUtc": iso(c.time),
            "bondAtoms": bond.to_string(),
            "evidence": c.text,
            "evidenceObject": archive_object.to_string(),
            "challengeDeadlineSlot": s.challenge_deadline_slot,
            "challengeableUntilUtc": iso(until),
            "openingClosesUtc": iso(opening.listing),
            "ruledBy": "the oracle authority, by hand, on the opening day",
            "ruledByUtc": iso(opening.listing),
            "settleCommand": settle,
            "nextCommand": settle,
        }),
        txs,
    })
}

/// The program's rules on accepting a print (165), checked before anything
/// is sent: a waiting print whose window passed unchallenged, its money
/// unsettled, before listing. `challenge` is the one it waits on, if any.
fn check_acceptable(
    claim: &OracleOpeningClaim,
    source: &SourceRef,
    opening: Opening,
    now: &ChainNow,
    challenge: Option<&Pubkey>,
) -> Result<()> {
    use OracleOpeningClaimStatus as S;
    let key = source.address;
    match claim.status {
        S::Pending => {}
        S::Challenged => bail(format!(
            "the opening print of source {key} is challenged{}: the oracle authority rules on it by hand on the opening day; once ruled and its challenge settled, a kept or replaced print can be challenged again for {ORACLE_OPENING_CHALLENGE_WINDOW_SLOTS} slots and then accepted",
            challenge
                .map(|c| format!(" (challenge {c})"))
                .unwrap_or_default()
        ))?,
        S::Accepted => bail(format!(
            "the opening print of source {key} is accepted already: its stake goes back with `petri oracle stakes settle --kind opening-claim`"
        ))?,
        S::Rejected => bail(format!(
            "the opening print of source {key} was rejected: there is nothing to accept"
        ))?,
        status => bail(format!(
            "the opening print of source {key} is {status:?}: it can't be accepted"
        ))?,
    }
    if claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        || source.state.status != OracleSourceStatus::OpeningPending
        || source.state.opening_submitted
    {
        bail(format!(
            "source {key} is {:?}, not waiting on its opening print",
            source.state.status
        ))?;
    }
    if opening.phase == OraclePhase::Opening && now.unix >= opening.listing {
        bail(format!(
            "listing passed at {} (the chain's clock says {}): a print not accepted by then can't be accepted, and nothing returns its stake",
            at(opening.listing),
            at(now.unix)
        ))?;
    }
    opening.check(now.unix, "opening prints are accepted")?;
    if now.slot < claim.challenge_deadline_slot {
        let from = now.time_of(claim.challenge_deadline_slot);
        bail(format!(
            "it is open to challenge until slot {} and can be accepted from then, about {} ({} slots from now, at {} ms a slot){}",
            claim.challenge_deadline_slot,
            at(from),
            claim.challenge_deadline_slot - now.slot,
            now.slot_ms,
            if from >= opening.listing {
                format!(
                    "; that is after listing at {}, so it can't be accepted in time",
                    at(opening.listing)
                )
            } else {
                String::new()
            }
        ))?;
    }
    Ok(())
}

/// `petri oracle prints finalize`: tag 165 (anyone may send it), held to the
/// program's rules first.
pub fn finalize(ctx: &Ctx, p: Paint, target: &Target, source: &str) -> Result<Report> {
    let write = finalize_write(ctx, target, source)?;
    send::run(ctx, p, write, target.send.simulate)
}

fn finalize_write(ctx: &Ctx, target: &Target, source: &str) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let source = oracle.source(&month, source)?;
    finalize_write_of(&oracle, &month, &source)
}

/// The accept of `source`'s print, held to the program's rules first.
fn finalize_write_of(oracle: &Oracle, month: &MonthRef, source: &SourceRef) -> Result<Write> {
    let ctx = oracle.ctx;
    let cranker = ctx.wallet_pubkey()?;
    let (month, source) = (month.clone(), source.clone());
    let id = month.period.expiry_id();
    let now = ChainNow::read(ctx)?;
    let opening = Opening::of(&month);
    let Some(claim) = oracle.opening_claim(&source)? else {
        opening.check(now.unix, "opening prints are accepted")?;
        return bail(format!(
            "source {} of {id} has no opening print yet: `petri oracle prints {id}` lists the sources waiting for one",
            source.address
        ));
    };
    let s = &claim.state;
    let waiting_on = (s.status == OracleOpeningClaimStatus::Challenged)
        .then(|| {
            oracle.opening_challenges(Some(&month.address)).map(|all| {
                all.into_iter()
                    .rev()
                    .find(|(_, c)| {
                        c.claim == claim.address
                            && c.claim_attempt == s.attempt
                            && c.escrow_disposition == OracleEscrowDisposition::Unsettled
                    })
                    .map(|(key, _)| key)
            })
        })
        .transpose()?
        .flatten();
    check_acceptable(s, &source, opening, &now, waiting_on.as_ref())?;
    let url = url_of(oracle, &source)?;
    let objects = chain::archive_objects(&oracle.program, &s.claimant, &s.archive_url_hash);
    let evidence = objects
        .iter()
        .zip(ctx.rpc.accounts(&objects)?)
        .find_map(|(key, account)| {
            chain::archive_in(&oracle.program, key, account.as_ref()?, &s.archive_url_hash)
        })
        .map(|(_, text)| text);
    let value = trim_decimal(u128::from(s.opening_state), 6);
    let settle = claim_settle_command(&month, &source.address);
    Ok(Write {
        action: "oracle.prints.finalize",
        title: format!(
            "Accept the opening print {value} of source {}",
            source.address
        ),
        review: vec![
            month_line(&month),
            format!(
                "Source {} · {} · {}",
                source.address,
                source.bucket_name(),
                url.as_deref()
                    .map_or("(URL not on chain)".into(), terminal_safe_text)
            ),
            format!(
                "Print {value} by {} (claim {}), unchallenged through slot {}",
                s.claimant, claim.address, s.challenge_deadline_slot
            ),
            "It becomes the source's opening value, where it starts in Game".to_string(),
            format!(
                "Then the {} USDC stake goes back to the claimant: {settle}",
                usdc(s.stake)
            ),
        ],
        details: json!({
            "month": month_json(&month),
            "source": source_json(&source),
            "url": url,
            "bucketName": source.bucket_name(),
            "claim": claim.address.to_string(),
            "claimant": s.claimant.to_string(),
            "value": value,
            "valueAtoms": s.opening_state.to_string(),
            "sourceTimeUtc": iso(s.source_time),
            "stakeAtoms": s.stake.to_string(),
            "evidence": evidence,
            "challengeDeadlineSlot": s.challenge_deadline_slot,
            "acceptFromUtc": iso(now.time_of(s.challenge_deadline_slot)),
            "openingClosesUtc": iso(opening.listing),
            "settleCommand": settle,
            "nextCommand": settle,
        }),
        txs: vec![Tx::one(
            "finalizeOpening",
            ix::finalize_oracle_opening_claim_v2(
                &oracle.program,
                &cranker,
                &month.period.address,
                &month.address,
                &source.address,
                &s.claimant,
            ),
        )],
    })
}

/// A carry-source record's status that lets its source be set aside (the
/// program's `NO_ELIGIBLE_CHECKPOINT`): nothing carries over from the
/// previous month.
const NO_CARRY: u8 = 2;
/// A carry-source record's exact length and marker (the program's
/// `CarrySource`: a six-byte header, then the record, its status last).
const CARRY_LEN: usize = 439;

/// The status of a source's carry-over record, when it has one.
fn carry_status(program: &Pubkey, account: Option<&Account>) -> Option<u8> {
    let account = account?;
    let d = &account.data;
    (account.owner == *program && d.len() == CARRY_LEN && d[..3] == *b"OCS")
        .then(|| d[CARRY_LEN - 1])
}

/// The program's rules on setting a source aside (81), checked before
/// anything is sent: the month in Opening, from listing until its expiry; a
/// frozen source without an opening or a print; not every source resolved;
/// nothing carried over from the previous month in progress.
fn check_expirable(
    source: &SourceRef,
    month: &Resolution,
    opening: Opening,
    now: u64,
    carry: Option<u8>,
) -> Result<()> {
    let key = source.address;
    if opening.phase != OraclePhase::Opening {
        bail(format!(
            "the month is in {:?}: a source without an opening is set aside in Opening, from listing at {}",
            opening.phase,
            utc(opening.listing as i64)
        ))?;
    }
    if now < opening.listing {
        bail(format!(
            "a source without an opening is set aside from listing at {}; the chain's clock says {}, and until then the source can still take a print",
            utc(opening.listing as i64),
            utc(now as i64)
        ))?;
    }
    if opening.listing >= opening.expiry || now >= opening.expiry {
        bail(format!(
            "the month expired at {}: its period fails instead of entering Game",
            utc(opening.expiry as i64)
        ))?;
    }
    match source.state.status {
        OracleSourceStatus::Frozen if !source.state.opening_submitted => {}
        OracleSourceStatus::OpeningPending => bail(format!(
            "source {key} has an opening print still waiting: it can't be set aside, and since listing passed nothing can accept it either"
        ))?,
        OracleSourceStatus::Active => bail(format!("source {key} has its opening value"))?,
        OracleSourceStatus::Inactive => bail(format!("source {key} is set aside already"))?,
        status => bail(format!(
            "source {key} is {status:?}: only a frozen source without an opening is set aside"
        ))?,
    }
    if month.resolved >= month.frozen {
        bail("every frozen source of the month is resolved already")?;
    }
    if carry.is_some_and(|status| status != NO_CARRY) {
        bail(format!(
            "source {key} carries an opening over from the previous month: it can't be set aside before the month's expiry"
        ))?;
    }
    Ok(())
}

/// The month's counts [`check_expirable`] reads.
#[derive(Clone, Copy, Debug)]
struct Resolution {
    resolved: u16,
    frozen: u16,
}

/// `petri oracle prints expire`: tag 81 (anyone may send it), held to the
/// program's rules first.
pub fn expire(ctx: &Ctx, p: Paint, target: &Target, source: &str) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let id = month.period.expiry_id();
    let source = oracle.source(&month, source)?;
    let now = u64::try_from(ctx.rpc.clock()?.unix_timestamp).unwrap_or(0);
    let opening = Opening::of(&month);
    let carry = ix::find_carry_source_address(&oracle.program, &source.address);
    check_expirable(
        &source,
        &Resolution {
            resolved: month.state.opening_resolved_source_count,
            frozen: month.state.frozen_source_count,
        },
        opening,
        now,
        carry_status(&oracle.program, ctx.rpc.account(&carry)?.as_ref()),
    )?;
    let url = url_of(&oracle, &source)?;
    let write = Write {
        action: "oracle.prints.expire",
        title: format!(
            "Set aside source {} of {id}: it has no opening value",
            source.address
        ),
        review: vec![
            month_line(&month),
            format!(
                "Source {} · {} · {}",
                source.address,
                source.bucket_name(),
                url.as_deref()
                    .map_or("(URL not on chain)".into(), terminal_safe_text)
            ),
            "It sits the month out; no money moves".to_string(),
            format!(
                "Once every frozen source has its opening or is set aside, the oracle authority weighs them, and then anyone moves the month into Game: petri oracle prints finalize-phase {id}"
            ),
        ],
        details: json!({
            "month": month_json(&month),
            "source": source_json(&source),
            "url": url,
            "bucketName": source.bucket_name(),
            "openingClosesUtc": iso(opening.listing),
            "expiryUtc": iso(opening.expiry),
            "nextCommand": Value::Null,
        }),
        txs: vec![Tx::one(
            "expireOpeningSource",
            ix::expire_oracle_opening_source(
                &oracle.program,
                &cranker,
                &month.period.address,
                &month.address,
                &source.address,
            ),
        )],
    };
    send::run(ctx, p, write, target.send.simulate)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    /// Scramble from 2026-10-18 00:00 UTC: the Opening runs Oct 25 to listing
    /// Oct 26.
    const START: u64 = 1_792_281_600 + 7 * DAY;

    fn opening(phase: OraclePhase) -> Opening {
        Opening {
            phase,
            start: START,
            listing: START + DAY,
            expiry: START + 40 * DAY,
        }
    }

    fn now(unix: u64, slot: u64) -> ChainNow {
        ChainNow {
            unix,
            slot,
            slot_ms: 400,
        }
    }

    #[test]
    fn an_opening_meets_the_oracles_rules_before_anything_is_sent() {
        let month = opening(OraclePhase::Opening);
        let page = "https://example.com/ram?id=1";
        let hash = hashv(&[b"locator", page.as_bytes()]).to_bytes();
        // Captured Oct 25 01:00; the chain says Oct 25 02:00.
        let url = format!("https://web.archive.org/web/20261025010000/{page}");
        let now = START + 2 * 3600;
        assert!(check_opening(month, now, &url, &hash, 1_000_000, 1_000_000).is_ok());
        let fails = |month: Opening, now: u64, url: &str, stake: u64| {
            check_opening(month, now, url, &hash, stake, 1_000_000)
                .unwrap_err()
                .message
        };
        assert!(fails(month, now, &url, 2_000_000).contains(
            "the opening bond of this product is 1 USDC: pass --stake 1, or leave --stake out"
        ));
        let early = format!("https://web.archive.org/web/20261024230000/{page}");
        assert!(fails(month, now, &early, 1_000_000).contains("before the Opening began"));
        // A capture the chain's clock has not reached yet.
        assert!(fails(month, START + 1800, &url, 1_000_000).contains("after the chain's clock"));
        let other = "https://web.archive.org/web/20261025010000/https://example.com/ram";
        assert!(fails(month, now, other, 1_000_000).contains("is not this source's page"));
        assert!(fails(month, month.listing, &url, 1_000_000).contains("on the chain's clock"));
        let game = opening(OraclePhase::Game);
        assert!(fails(game, now, &url, 1_000_000).contains("the month is in Game"));
        assert!(month.is_open(START) && month.is_open(month.listing - 1));
        assert!(!month.is_open(START - 1) && !month.is_open(month.listing));
        assert!(!game.is_open(now));
    }

    /// A Clock sysvar: slot, epoch start time, epoch, leader schedule epoch,
    /// unix time.
    fn clock(slot: u64, epoch_start: i64, epoch: u64, unix: i64) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&slot.to_le_bytes());
        d.extend_from_slice(&epoch_start.to_le_bytes());
        d.extend_from_slice(&epoch.to_le_bytes());
        d.extend_from_slice(&epoch.to_le_bytes());
        d.extend_from_slice(&unix.to_le_bytes());
        d
    }

    /// An EpochSchedule sysvar without warm-up (mainnet's).
    fn schedule(slots_per_epoch: u64) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&slots_per_epoch.to_le_bytes());
        d.extend_from_slice(&slots_per_epoch.to_le_bytes());
        d.push(0);
        d.extend_from_slice(&0u64.to_le_bytes());
        d.extend_from_slice(&0u64.to_le_bytes());
        d
    }

    #[test]
    fn slot_deadlines_become_times_at_the_chains_own_pace() {
        // Epoch 700 of 432,000 slots began at slot 302,400,000; 10,000 slots
        // later the chain's clock has moved 4,000 s: 400 ms a slot.
        let c = clock(302_410_000, 1_800_000_000, 700, 1_800_004_000);
        let now = ChainNow::from_sysvars(&c, Some(&schedule(432_000))).unwrap();
        assert_eq!(
            (now.slot, now.unix, now.slot_ms),
            (302_410_000, 1_800_004_000, 400)
        );
        // 300 slots ahead is two minutes; behind, as far back.
        assert_eq!(now.time_of(now.slot + 300), now.unix + 120);
        assert_eq!(now.time_of(now.slot - 300), now.unix - 120);
        // A slow chain (the dev validator): 2,500 s over 10,000 slots.
        let slow = clock(302_410_000, 1_800_000_000, 700, 1_800_002_500);
        assert_eq!(
            ChainNow::from_sysvars(&slow, Some(&schedule(432_000)))
                .unwrap()
                .slot_ms,
            250
        );
        // An epoch too young to measure, or no schedule: Solana's 400 ms.
        let young = clock(302_400_500, 1_800_000_000, 700, 1_800_000_100);
        assert_eq!(
            ChainNow::from_sysvars(&young, Some(&schedule(432_000)))
                .unwrap()
                .slot_ms,
            TARGET_SLOT_MS
        );
        assert_eq!(
            ChainNow::from_sysvars(&c, None).unwrap().slot_ms,
            TARGET_SLOT_MS
        );
        assert!(ChainNow::from_sysvars(&c[..39], None).is_err());
        // Warm-up epochs double from 32 slots.
        assert_eq!(first_slot(0, 432_000, 14, 524_256), 0);
        assert_eq!(first_slot(1, 432_000, 14, 524_256), 32);
        assert_eq!(first_slot(3, 432_000, 14, 524_256), 224);
        assert_eq!(first_slot(15, 432_000, 14, 524_256), 956_256);
    }

    fn claim(
        status: OracleOpeningClaimStatus,
        escrow: OracleEscrowDisposition,
    ) -> OracleOpeningClaim {
        OracleOpeningClaim {
            is_initialized: true,
            attempt: 1,
            claimant: Pubkey::new_unique(),
            opening_state: 75_500_000,
            source_time: START + 3600,
            stake: 5_000_000,
            challenge_deadline_slot: 1_300,
            status,
            escrow_disposition: escrow,
            ..Default::default()
        }
    }

    fn challenge_of(
        status: OracleChallengeStatus,
        escrow: OracleEscrowDisposition,
    ) -> OracleOpeningClaimChallenge {
        OracleOpeningClaimChallenge {
            is_initialized: true,
            claim_attempt: 1,
            challenger: Pubkey::new_unique(),
            bond: 1_000_000,
            status,
            escrow_disposition: escrow,
            ..Default::default()
        }
    }

    #[test]
    fn a_print_stands_by_its_claim_its_challenge_and_the_clock() {
        use OracleChallengeStatus as C;
        use OracleEscrowDisposition as E;
        use OracleOpeningClaimStatus as S;
        let of =
            |claim: &OracleOpeningClaim, c: Option<&OracleOpeningClaimChallenge>, open, slot| {
                let s = Standing::of_print(claim, c, open, slot);
                let settled = claim.escrow_disposition != E::Unsettled;
                (s.code(settled), s.words(settled))
            };
        let waiting = claim(S::Pending, E::Unsettled);
        // Inside its 300 slots, after them, and at listing.
        assert_eq!(
            of(&waiting, None, true, 1_299),
            ("challengeable", "Open to challenge")
        );
        assert_eq!(
            of(&waiting, None, true, 1_300),
            ("acceptable", "Unchallenged: anyone can accept it now")
        );
        assert_eq!(of(&waiting, None, false, 1_299).0, "expired");
        assert!(of(&waiting, None, false, 2_000).1.contains("stays locked"));
        // Challenged: the ruling is the oracle authority's, by hand.
        let challenged = claim(S::Challenged, E::Unsettled);
        let review = challenge_of(C::RuleReview, E::Unsettled);
        let (code, words) = of(&challenged, Some(&review), true, 2_000);
        assert_eq!(code, "challenged");
        assert!(
            words.contains("oracle authority") && words.contains("by hand"),
            "{words}"
        );
        assert_eq!(of(&challenged, Some(&review), false, 2_000).0, "expired");
        assert_eq!(of(&challenged, None, true, 2_000).0, "challenged");
        let council = challenge_of(C::RuleReviewUnresolved, E::Unsettled);
        assert_eq!(of(&challenged, Some(&council), true, 2_000).0, "council");
        // Ruled, before its challenge settles.
        let kept = challenge_of(C::Rejected, E::Unsettled);
        assert_eq!(of(&challenged, Some(&kept), true, 2_000).0, "kept");
        let upheld = challenge_of(C::Accepted, E::Unsettled);
        assert_eq!(of(&challenged, Some(&upheld), true, 2_000).0, "replaced");
        let rejected = claim(S::Rejected, E::Unsettled);
        assert_eq!(of(&rejected, Some(&upheld), true, 2_000).0, "rejected");
        // Its money settled.
        assert_eq!(
            of(&claim(S::Rejected, E::Slashed), None, true, 2_000),
            (
                "settled",
                "Rejected; the challenger got the bond back and the print's stake"
            )
        );
        assert_eq!(
            of(&claim(S::Accepted, E::Unsettled), None, false, 2_000).0,
            "accepted"
        );
        assert_eq!(
            of(&claim(S::Accepted, E::Refunded), None, false, 2_000),
            ("settled", "Accepted; its stake went back to the claimant")
        );
        assert_eq!(
            of(&claim(S::TimedOut, E::Unsettled), None, false, 2_000).0,
            "cancelled"
        );
    }

    #[test]
    fn a_challenge_stands_by_its_ruling_and_its_money() {
        use OracleChallengeStatus as C;
        use OracleEscrowDisposition as E;
        use OracleOpeningClaimStatus as S;
        let of = |status, escrow, claim| {
            let c = challenge_of(status, escrow);
            let s = Standing::of_challenge(&c, claim, true);
            s.code(escrow != E::Unsettled)
        };
        assert_eq!(of(C::RuleReview, E::Unsettled, S::Challenged), "challenged");
        assert_eq!(
            Standing::of_challenge(
                &challenge_of(C::RuleReview, E::Unsettled),
                S::Challenged,
                false
            ),
            Standing::Expired
        );
        assert_eq!(
            of(C::RuleReviewUnresolved, E::Unsettled, S::Challenged),
            "council"
        );
        assert_eq!(of(C::Rejected, E::Unsettled, S::Challenged), "kept");
        assert_eq!(of(C::Rejected, E::Slashed, S::Pending), "settled");
        assert_eq!(of(C::Accepted, E::Unsettled, S::Challenged), "replaced");
        assert_eq!(of(C::Accepted, E::Unsettled, S::Rejected), "rejected");
        assert_eq!(of(C::Cancelled, E::Unsettled, S::TimedOut), "cancelled");
        // Settled: what became of the bond.
        let settled = |status, escrow| {
            Standing::of_challenge(&challenge_of(status, escrow), S::Pending, false).words(true)
        };
        assert!(settled(C::Accepted, E::Transferred).starts_with("Replaced"));
        assert!(settled(C::Accepted, E::Refunded).starts_with("Rejected"));
        assert!(settled(C::Rejected, E::Slashed).starts_with("Kept"));
    }

    #[test]
    fn a_settled_print_names_no_settle_command_and_a_decided_one_its_challenge() {
        let month = MonthRef {
            period: chain::PeriodRef {
                address: Pubkey::new_unique(),
                period: ameba_oracle::interface::Period::new_forming(
                    255,
                    chain::product_id("RAMX").unwrap(),
                    START - 20 * DAY,
                    1_798_761_600,
                    6,
                ),
                symbol: "RAMX".into(),
            },
            address: Pubkey::new_unique(),
            state: Default::default(),
        };
        let (source, challenge) = (Pubkey::new_unique(), Pubkey::new_unique());
        let settle = |s, settled, c: Option<&Pubkey>| print_settle(s, settled, &month, &source, c);
        assert_eq!(
            settle(Standing::Accepted, false, None).unwrap(),
            format!(
                "petri oracle stakes settle RAMX-202612 --kind opening-claim --source {source}"
            )
        );
        for decided in [Standing::Kept, Standing::Replaced, Standing::Rejected] {
            assert_eq!(
                settle(decided, false, Some(&challenge)).unwrap(),
                format!(
                    "petri oracle stakes settle RAMX-202612 --kind opening-challenge --source {source} --challenge {challenge}"
                )
            );
        }
        assert_eq!(settle(Standing::Accepted, true, None), None);
        assert_eq!(settle(Standing::Challenged, false, Some(&challenge)), None);
        assert_eq!(settle(Standing::Expired, false, None), None);
        assert_eq!(
            accept_command(&month, &source),
            format!("petri oracle prints finalize RAMX-202612 --source {source}")
        );
    }

    #[test]
    fn accept_waits_at_the_chains_pace_a_few_seconds_at_a_time() {
        let now = now(START, 1_000);
        let ms = |slot| pause_until(&now, slot).as_millis();
        // 2 slots of 400 ms: under the floor; 300 slots: capped at 5 s; past: the floor.
        assert_eq!(ms(1_001), 400);
        assert_eq!(ms(1_005), 2_000);
        assert_eq!(ms(1_300), 5_000);
        assert_eq!(ms(900), 400);
        assert!(accept_note().contains("300 slots"));
    }

    #[test]
    fn the_bond_is_the_products_share_of_the_prints_stake() {
        let terms = Terms {
            bps: 2_000,
            minimum: 1_000_000,
            maximum: 10_000_000,
        };
        // 20% of a 5 USDC stake is 1 USDC; of 12.5, 2.5; held between the terms.
        assert_eq!(challenge_bond(&terms, 5_000_000).unwrap(), 1_000_000);
        assert_eq!(challenge_bond(&terms, 12_500_003).unwrap(), 2_500_000);
        assert_eq!(challenge_bond(&terms, 1_000_000).unwrap(), 1_000_000);
        assert_eq!(challenge_bond(&terms, 500_000_000).unwrap(), 10_000_000);
        let unset = Terms {
            minimum: 0,
            ..terms
        };
        assert!(
            challenge_bond(&unset, 5_000_000)
                .unwrap_err()
                .message
                .contains("opening prints can't be challenged")
        );
        assert_eq!(
            terms.words_of("the print's stake"),
            "20% of the print's stake, at least 1.00 and at most 10.00 USDC"
        );
    }

    fn pending_source() -> SourceRef {
        SourceRef {
            address: Pubkey::new_unique(),
            state: OracleSourceState {
                is_initialized: true,
                status: OracleSourceStatus::OpeningPending,
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_challenge_is_of_someone_elses_waiting_print_inside_its_window() {
        use OracleEscrowDisposition as E;
        use OracleOpeningClaimStatus as S;
        let source = pending_source();
        let challenger = Pubkey::new_unique();
        let waiting = claim(S::Pending, E::Unsettled);
        let fails = |c: &OracleOpeningClaim, who: &Pubkey, slot: u64| {
            check_challengeable(
                c,
                &source,
                who,
                &now(START, slot),
                "petri oracle prints finalize",
            )
            .unwrap_err()
            .message
        };
        assert!(
            check_challengeable(&waiting, &source, &challenger, &now(START, 1_299), "").is_ok()
        );
        assert!(
            fails(&waiting, &challenger, 1_300)
                .contains("challenge window of this print closed at slot 1300")
        );
        assert!(
            fails(&waiting, &waiting.claimant, 1_000).contains("its claimant can't challenge it")
        );
        assert!(
            fails(&claim(S::Challenged, E::Unsettled), &challenger, 1_000)
                .contains("challenged already")
        );
        assert!(
            fails(&claim(S::Accepted, E::Unsettled), &challenger, 1_000).contains("is accepted")
        );
        assert!(
            fails(&claim(S::Rejected, E::Unsettled), &challenger, 1_000).contains("was rejected")
        );
        assert!(fails(&claim(S::Pending, E::Refunded), &challenger, 1_000).contains("not waiting"));
    }

    #[test]
    fn a_print_is_accepted_after_its_window_and_before_listing() {
        use OracleEscrowDisposition as E;
        use OracleOpeningClaimStatus as S;
        let source = pending_source();
        let month = opening(OraclePhase::Opening);
        let waiting = claim(S::Pending, E::Unsettled);
        let inside = START + 3 * 3600;
        let fails = |c: &OracleOpeningClaim, month: Opening, unix: u64, slot: u64| {
            check_acceptable(c, &source, month, &now(unix, slot), None)
                .unwrap_err()
                .message
        };
        assert!(check_acceptable(&waiting, &source, month, &now(inside, 1_300), None).is_ok());
        // Still open to challenge: when it can be accepted, about.
        let early = fails(&waiting, month, inside, 1_000);
        assert!(
            early.contains("open to challenge until slot 1300"),
            "{early}"
        );
        assert!(early.contains("300 slots from now"), "{early}");
        // So late that its window outlasts the listing.
        let late = fails(&waiting, month, month.listing - 10, 1_000);
        assert!(late.contains("can't be accepted in time"), "{late}");
        // Listing passed.
        let after = fails(&waiting, month, month.listing, 2_000);
        assert!(
            after.contains("listing passed") && after.contains("nothing returns its stake"),
            "{after}"
        );
        assert!(
            fails(&waiting, opening(OraclePhase::Game), inside, 2_000)
                .contains("the month is in Game")
        );
        let challenged = fails(&claim(S::Challenged, E::Unsettled), month, inside, 2_000);
        assert!(
            challenged.contains("oracle authority rules on it by hand"),
            "{challenged}"
        );
        assert!(
            fails(&claim(S::Accepted, E::Unsettled), month, inside, 2_000)
                .contains("accepted already")
        );
        assert!(
            fails(&claim(S::Rejected, E::Slashed), month, inside, 2_000).contains("was rejected")
        );
    }

    #[test]
    fn a_source_is_set_aside_from_listing_when_it_has_no_print() {
        let month = opening(OraclePhase::Opening);
        let frozen = SourceRef {
            address: Pubkey::new_unique(),
            state: OracleSourceState {
                is_initialized: true,
                status: OracleSourceStatus::Frozen,
                ..Default::default()
            },
        };
        let counts = Resolution {
            resolved: 3,
            frozen: 4,
        };
        let after = month.listing + 60;
        let fails = |s: &SourceRef, counts, opening, now, carry| {
            check_expirable(s, &counts, opening, now, carry)
                .unwrap_err()
                .message
        };
        assert!(check_expirable(&frozen, &counts, month, after, None).is_ok());
        assert!(check_expirable(&frozen, &counts, month, after, Some(NO_CARRY)).is_ok());
        assert!(
            fails(&frozen, counts, month, month.listing - 1, None)
                .contains("can still take a print")
        );
        assert!(fails(&frozen, counts, month, month.expiry, None).contains("expired"));
        assert!(
            fails(&frozen, counts, opening(OraclePhase::Game), after, None)
                .contains("the month is in Game")
        );
        assert!(fails(&pending_source(), counts, month, after, None).contains("still waiting"));
        let mut active = frozen.clone();
        active.state.status = OracleSourceStatus::Active;
        assert!(fails(&active, counts, month, after, None).contains("has its opening value"));
        let done = Resolution {
            resolved: 4,
            frozen: 4,
        };
        assert!(fails(&frozen, done, month, after, None).contains("resolved already"));
        assert!(fails(&frozen, counts, month, after, Some(3)).contains("carries an opening over"));
    }

    #[test]
    fn a_new_print_needs_a_frozen_source_free_of_a_claim() {
        use OracleEscrowDisposition as E;
        use OracleOpeningClaimStatus as S;
        let frozen = OracleSourceState {
            status: OracleSourceStatus::Frozen,
            ..Default::default()
        };
        assert!(printable(&frozen, None).is_ok());
        // A rejected print whose challenge settled: the source takes another.
        assert!(printable(&frozen, Some(&claim(S::Rejected, E::Slashed))).is_ok());
        assert!(
            printable(&frozen, Some(&claim(S::Rejected, E::Unsettled)))
                .unwrap_err()
                .message
                .contains("settle it first")
        );
        let pending = OracleSourceState {
            status: OracleSourceStatus::OpeningPending,
            ..frozen
        };
        assert!(
            printable(&pending, None)
                .unwrap_err()
                .message
                .contains("waiting already")
        );
        let opened = OracleSourceState {
            opening_submitted: true,
            ..frozen
        };
        assert!(printable(&opened, None).is_err());
    }

    #[test]
    fn an_earlier_print_ended_by_the_first_challenge_that_replaced_it() {
        use OracleChallengeStatus as C;
        use OracleEscrowDisposition as E;
        let link = PrintLink {
            address: Pubkey::new_unique(),
            month: Pubkey::new_unique(),
            source: Pubkey::new_unique(),
            claim: Pubkey::new_unique(),
            object: Pubkey::new_unique(),
            attempt: 1,
            value: 1,
            time: 1,
            evidence_hash: [1; 32],
        };
        let kept = (Pubkey::new_unique(), challenge_of(C::Rejected, E::Slashed));
        let won = (
            Pubkey::new_unique(),
            challenge_of(C::Accepted, E::Transferred),
        );
        let later = (Pubkey::new_unique(), challenge_of(C::Accepted, E::Refunded));
        let (standing, by) = ended_by(&link, &[&kept, &won, &later]);
        assert_eq!(
            (standing, by.map(|c| c.0)),
            (Standing::Replaced, Some(won.0))
        );
        let (standing, by) = ended_by(&link, &[&kept, &later]);
        assert_eq!(
            (standing, by.map(|c| c.0)),
            (Standing::Rejected, Some(later.0))
        );
        // Another attempt's challenges don't count.
        let mut other = later.clone();
        other.1.claim_attempt = 2;
        assert_eq!(ended_by(&link, &[&other]), (Standing::Rejected, None));
    }
}
