//! Game updates (tags 166-169, 198, 199, 201) and how they end. An update is
//! a new value for an active source while its month is in Game: committed
//! sealed with the product's exact update stake (166), revealed with its
//! archive evidence within 150 slots (198; `--reveal` does both in one
//! step), open to challenge with an exact bond (168) until it is finalized,
//! and accepted (199) once its challenge window has passed. Petri holds each
//! write to the program's rules before anything is sent.
//!
//! Who acts: the keeper (`futures-keeper`, signing as the oracle authority,
//! on the dev chain and on mainnet) accepts an unchallenged claim once its
//! challenge window has passed (a web proof's claim once its proof
//! verifies), refunds a commitment nobody revealed (167) and times out a
//! claim that can no longer be accepted (201). A challenged claim waits for
//! the oracle authority's ruling, made by hand (`futures-ops resolve-update`:
//! accept, reject or send it to the council), and the council's (30/18).
//! Time alone never ends a challenged claim: it stays locked until it is
//! ruled, or until its source takes another value (then 201 refunds both
//! sides). Nothing pays out until someone settles it (`stakes settle`, 169,
//! permissionless). `petri oracle updates` lists a wallet's update claims and
//! challenges, the updates open to challenge now and the sources taking one.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use ameba_oracle::{
    constants::{
        ORACLE_SETTLEMENT_GRACE_SECONDS, ORACLE_UPDATE_CHALLENGE_SECONDS,
        ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS, ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
    },
    instruction::{
        self as ix, ChallengeOracleUpdateClaimParams, CommitOracleUpdateClaimV2Params,
        RevealOracleUpdateClaimV3Params,
    },
    processor::oracle_update_claim_v2_commitment_hash,
    state::{
        OracleChallengeStatus, OracleClaimStatus, OracleEscrowDisposition, OracleMonthState,
        OraclePhase, OracleSourceState, OracleSourceStatus, OracleUpdateChallenge,
        OracleUpdateClaimV2, OracleUsdcSkuPool,
    },
};
use serde_json::{Value, json};
use solana_program::{hash::hashv, pubkey::Pubkey};

use super::{
    args::Target,
    chain::{self, MonthRef, Oracle, SourceRef, decode},
    challenges::{Terms, iso},
    claims,
    prints::ChainNow as Now,
    send::{self, Tx, Write},
    skus::{hex, parse_hex32},
    writes::{
        Cite, Cited, archive_parts, cite, cited_kind, claim_month, evidence_preimage, evidence_txs,
        id_or_random, month_json, month_line, random32, source_json, update_claim, usdc_amount,
    },
};
use crate::{
    backend::terminal_safe_text,
    futures::{
        Report,
        ctx::Ctx,
        error::{Error, Result, bail},
        market::Deployment,
        rpc::Filter,
        units::{short, trim_decimal, usdc},
        view::{Paint, Table, utc},
    },
};

// ------------------------------------------------------------------ timing

/// The dev chain's localnet releases (`localnet-dev`, `localnet-governance`)
/// shorten the challenge window of a new update to 30 seconds
/// (`ORACLE_UPDATE_CHALLENGE_SECONDS` under their features; `oracle/docs/
/// architecture.md`, "Continuous dev Game simulation"). Petri builds the
/// oracle crate without them, so its constant is production's day.
const DEV_CHALLENGE_SECONDS: u64 = 30;

/// How long a revealed update stays open to challenge before 199 can accept
/// it: a day on mainnet, 30 seconds on the dev chain.
pub fn challenge_seconds(dep: &Deployment) -> u64 {
    let localnet = dep
        .oracle_release_tag
        .as_deref()
        .is_some_and(|tag| tag.starts_with("localnet"));
    if localnet && dep.dev_chain() {
        DEV_CHALLENGE_SECONDS
    } else {
        ORACLE_UPDATE_CHALLENGE_SECONDS
    }
}

/// When one of a claim's slots was (or will be), estimated from the nearest
/// exact time: its reveal's (slot and clock) once it has one, else the
/// chain's clock now.
fn claim_time(now: &Now, claim: &OracleUpdateClaimV2, slot: u64) -> u64 {
    let anchor = if claim.revealed_slot > 0 && claim.revealed_at_ts > 0 {
        Now {
            slot: claim.revealed_slot,
            unix: claim.revealed_at_ts,
            ..*now
        }
    } else {
        *now
    };
    anchor.time_of(slot)
}

fn at(ts: u64) -> String {
    utc(i64::try_from(ts).unwrap_or(i64::MAX))
}

/// A month's update calendar on the chain's clock (the program's
/// `ensure_oracle_game_window` and `ensure_current_cash_update_resolution_window`):
/// updates are committed and revealed in Game from listing to expiry, and
/// challenged and ruled on until a day after expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Game {
    pub phase: OraclePhase,
    pub listing: u64,
    pub expiry: u64,
    /// Its openings are resolved and its active weights set (the program's
    /// `ensure_oracle_opening_resolution_complete`).
    pub ready: bool,
}

/// The program's `ensure_oracle_opening_resolution_complete`, on the month's fields.
fn resolved(m: &OracleMonthState) -> bool {
    m.weight_scheme_version == 1
        && m.effective_weight_total_bps == 10_000
        && m.weight_manifest_hash != [0; 32]
        && m.recipe_hash != [0; 32]
        && m.frozen_source_count > 0
        && m.opening_resolved_source_count == m.frozen_source_count
        && m.active_weight_scheme_version == OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION
        && m.active_weight_group_count > 0
        && m.active_weight_manifest_hash != [0; 32]
}

impl Game {
    pub fn of(month: &MonthRef) -> Self {
        Self {
            phase: month.state.phase,
            listing: month.state.listing_ts,
            expiry: month.period.period.expiry_ts,
            ready: resolved(&month.state),
        }
    }

    /// A day after expiry: the end of challenges and rulings.
    pub fn grace_end(&self) -> u64 {
        self.expiry.saturating_add(ORACLE_SETTLEMENT_GRACE_SECONDS)
    }

    fn check(&self, now: u64, month: &str, until: u64, what: &str) -> Result<()> {
        if self.phase != OraclePhase::Game {
            bail(format!(
                "{month} is in {:?}: {what} in Game, from listing ({}) to {}",
                self.phase,
                at(self.listing),
                at(until)
            ))?;
        }
        if !self.ready {
            bail(format!(
                "{month}'s openings or active weights are not settled yet, so it takes no updates"
            ))?;
        }
        if self.listing >= self.expiry || now < self.listing {
            bail(format!(
                "{month} lists at {}: {what} from then; the chain's clock says {}",
                at(self.listing),
                at(now)
            ))?;
        }
        if now >= until {
            bail(format!(
                "{month}'s window for this closed at {}; the chain's clock says {}",
                at(until),
                at(now)
            ))?;
        }
        Ok(())
    }

    /// Refuses unless updates are committed and revealed now (listing to expiry).
    pub fn check_updates(&self, now: u64, month: &str) -> Result<()> {
        self.check(
            now,
            month,
            self.expiry,
            "updates are committed and revealed",
        )
    }

    /// Refuses unless updates are challenged now (listing to a day after expiry).
    pub fn check_challenges(&self, now: u64, month: &str) -> Result<()> {
        self.check(now, month, self.grace_end(), "updates are challenged")
    }

    pub fn takes_updates(&self, now: u64) -> bool {
        self.check_updates(now, "").is_ok()
    }

    pub fn takes_challenges(&self, now: u64) -> bool {
        self.check_challenges(now, "").is_ok()
    }
}

/// When 199 can accept a revealed claim: its challenge window after its reveal.
pub fn accept_from(claim: &OracleUpdateClaimV2, challenge_seconds: u64) -> Option<u64> {
    (claim.revealed_at_ts > 0).then(|| claim.revealed_at_ts.saturating_add(challenge_seconds))
}

/// A revealed claim whose source moved on can't be accepted or challenged
/// (the program's `ensure_live_revealed_oracle_update_claim` and its step
/// check): another update of the source was accepted first, or the source
/// left the recipe.
fn overtaken(claim: &OracleUpdateClaimV2, source: &OracleSourceState) -> bool {
    source.status != OracleSourceStatus::Active
        || !source.opening_submitted
        || claim.claim.prior_state != source.current_state
        || claim.prior_finalized_step != source.last_finalized_step
}

// ---------------------------------------------------------------- standing

/// Why a revealed claim can no longer be accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overtaken {
    /// Its source took another value (or left the recipe) first.
    Source,
    /// Unchallenged and not accepted by a day after expiry.
    Late,
}

/// Where an update claim stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Sealed: waiting for its reveal.
    Committed,
    /// Not revealed by its deadline: its stake goes back (167).
    Expired,
    /// Revealed and open to challenge; `due` once its window has passed and
    /// the keeper can accept it.
    Challengeable { due: bool },
    /// Revealed, but it can no longer be accepted: 201 times it out.
    Stale(Overtaken),
    /// Challenged: waiting for the oracle authority's ruling, by hand.
    Challenged,
    /// The ruling left it to the council.
    Council,
    /// Accepted (199): its value is the source's.
    Accepted,
    /// Rejected: the challenger's alternative won.
    Replaced,
    /// Rejected with its challenge (the council's "neither").
    Rejected,
    /// Timed out after its reveal (201).
    Cancelled,
}

/// What a claim's standing reads besides the claim: the chain's slot and
/// clock, the challenge window and the month's end of challenges.
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub slot: u64,
    pub now: u64,
    pub challenge_seconds: u64,
    pub grace_end: u64,
}

impl Standing {
    /// A claim's standing from its status, its challenge's (when it has
    /// one), its source and the chain's clock.
    pub fn of(
        claim: &OracleUpdateClaimV2,
        challenge: Option<OracleChallengeStatus>,
        source: Option<&OracleSourceState>,
        when: When,
    ) -> Self {
        use OracleClaimStatus as S;
        match claim.claim.status {
            S::Open | S::Committed if when.slot > claim.reveal_deadline_slot => Self::Expired,
            S::Open | S::Committed => Self::Committed,
            S::Revealed => {
                let moved = source.is_none_or(|s| overtaken(claim, s));
                match challenge {
                    Some(_) if moved => Self::Stale(Overtaken::Source),
                    Some(OracleChallengeStatus::RuleReviewUnresolved) => Self::Council,
                    Some(_) if claim.council_review_pending => Self::Council,
                    Some(_) => Self::Challenged,
                    None if moved => Self::Stale(Overtaken::Source),
                    None if when.now >= when.grace_end => Self::Stale(Overtaken::Late),
                    None => Self::Challengeable {
                        due: accept_from(claim, when.challenge_seconds)
                            .is_some_and(|t| when.now >= t),
                    },
                }
            }
            S::Finalized => Self::Accepted,
            S::Rejected if challenge == Some(OracleChallengeStatus::Accepted) => Self::Replaced,
            S::Rejected => Self::Rejected,
            S::TimedOut if claim.revealed_slot == 0 => Self::Expired,
            S::TimedOut => Self::Cancelled,
        }
    }

    /// Where it stands, for JSON (`settled` once its money settled).
    pub fn code(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "settled",
            Self::Committed => "committed",
            Self::Expired => "expired",
            Self::Challengeable { .. } => "challengeable",
            Self::Stale(_) => "stale",
            Self::Challenged => "challenged",
            Self::Council => "council",
            Self::Accepted => "accepted",
            Self::Replaced => "replaced",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    /// Where it stands and what happens to the money, in plain words.
    pub fn words(self, settled: bool, challenged: bool) -> &'static str {
        match (self, settled, challenged) {
            (Self::Expired, true, _) => "Not revealed in time: its stake went back",
            (_, true, _) => "Settled: paid out",
            (Self::Committed, ..) => {
                "Committed: waiting for its reveal, within 150 slots of the commit"
            }
            (Self::Expired, ..) => {
                "Not revealed in time: its stake goes back (the keeper refunds it, or anyone with expire)"
            }
            (Self::Challengeable { due: false }, ..) => {
                "Open to challenge; unchallenged, the keeper accepts it once its challenge window ends"
            }
            (Self::Challengeable { due: true }, ..) => {
                "Challenge window over: the keeper accepts it on its next pass (a web proof must verify first); open to challenge until then"
            }
            (Self::Stale(Overtaken::Source), _, false) => {
                "Overtaken: its source changed first, so it can't be accepted; the keeper times it out and its stake goes back"
            }
            (Self::Stale(Overtaken::Source), _, true) => {
                "Overtaken while challenged: its source changed first; it times out and the stake and the bond go back"
            }
            (Self::Stale(Overtaken::Late), ..) => {
                "Not accepted by a day after expiry: the keeper times it out and its stake goes back"
            }
            (Self::Challenged, ..) => {
                "Challenged: waiting for the oracle authority's ruling, made by hand (accept, reject, or the council); the winner takes the stake and the bond"
            }
            (Self::Council, ..) => {
                "With the council: three of five seats choose the claim, the alternative or neither"
            }
            (Self::Accepted, _, false) => {
                "Accepted: its value is the source's; its stake goes back"
            }
            (Self::Accepted, _, true) => {
                "Accepted over its challenge: the claimant takes the stake and the bond"
            }
            (Self::Replaced, ..) => {
                "Rejected: the challenger's value stands; the challenger takes the stake and the bond"
            }
            (Self::Rejected, ..) => {
                "Rejected with its challenge: the source keeps its value; the stake and the bond go back"
            }
            (Self::Cancelled, _, false) => "Timed out: its stake goes back",
            (Self::Cancelled, _, true) => {
                "Timed out with its challenge: the stake and the bond go back"
            }
        }
    }

    /// Whose collateral the money settles into: the claimant, the
    /// challenger, or both their own back; None while it is undecided.
    pub fn pays(self, challenged: bool) -> Option<&'static str> {
        match self {
            Self::Committed | Self::Challengeable { .. } | Self::Challenged | Self::Council => None,
            Self::Accepted | Self::Expired => Some("claimant"),
            Self::Replaced => Some("challenger"),
            Self::Rejected => Some("both"),
            Self::Stale(_) | Self::Cancelled if challenged => Some("both"),
            Self::Stale(_) | Self::Cancelled => Some("claimant"),
        }
    }
}

/// How a challenge of an update stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ruling {
    Waiting,
    Council,
    /// The alternative won.
    Upheld,
    /// The claim stood.
    Dismissed,
    /// The council chose neither value.
    Neither,
    Cancelled,
}

impl Ruling {
    pub fn of(status: OracleChallengeStatus, claim: OracleClaimStatus) -> Self {
        use OracleChallengeStatus as S;
        match status {
            S::Open | S::RuleReview => Self::Waiting,
            S::RuleReviewUnresolved => Self::Council,
            S::Accepted => Self::Upheld,
            S::Rejected if claim == OracleClaimStatus::Rejected => Self::Neither,
            S::Rejected => Self::Dismissed,
            S::Cancelled => Self::Cancelled,
        }
    }

    pub fn code(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "settled",
            Self::Waiting => "waiting",
            Self::Council => "council",
            Self::Upheld => "upheld",
            Self::Dismissed => "dismissed",
            Self::Neither => "neither",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn words(self, settled: bool) -> &'static str {
        match self {
            _ if settled => "Settled: paid out",
            Self::Waiting => "Waiting for the oracle authority's ruling (by hand)",
            Self::Council => "With the council",
            Self::Upheld => {
                "Upheld: the alternative stands; the challenger takes the stake and the bond"
            }
            Self::Dismissed => "Dismissed: the claim stands; the claimant takes the bond",
            Self::Neither => "Neither value: the stake and the bond go back",
            Self::Cancelled => "Cancelled: the stake and the bond go back",
        }
    }
}

fn settled(claim: &OracleUpdateClaimV2, challenge: Option<&OracleUpdateChallenge>) -> bool {
    claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled
        && challenge.is_none_or(|c| c.escrow_disposition != OracleEscrowDisposition::Unsettled)
}

/// The `stakes settle` command once the claim's money can settle (the
/// program's `settle_cash_update_claim` and `settle_cash_update_challenge`),
/// else None.
fn settle_command(
    month: &str,
    key: &Pubkey,
    claim: &OracleUpdateClaimV2,
    challenge: Option<(&Pubkey, &OracleUpdateChallenge)>,
) -> Option<String> {
    use OracleChallengeStatus as C;
    use OracleClaimStatus as S;
    let c = &claim.claim;
    if c.escrow_disposition != OracleEscrowDisposition::Unsettled || claim.council_review_pending {
        return None;
    }
    match challenge {
        None => matches!(c.status, S::Finalized | S::TimedOut).then(|| {
            format!(
                "petri oracle stakes settle {month} --kind update-claim --source {} --claim {key}",
                c.source
            )
        }),
        Some((address, ch)) => (ch.escrow_disposition == OracleEscrowDisposition::Unsettled
            && matches!(
                (ch.status, c.status),
                (C::Rejected, S::Finalized)
                    | (C::Accepted, S::Rejected)
                    | (C::Rejected, S::Rejected)
                    | (C::Cancelled, S::TimedOut)
            ))
        .then(|| {
            format!(
                "petri oracle stakes settle {month} --kind update-challenge --source {} --claim {key} --challenge {address}",
                c.source
            )
        }),
    }
}

fn reveal_command(key: &Pubkey) -> String {
    format!("petri oracle updates reveal {key}")
}

fn expire_command(month: &str, key: &Pubkey) -> String {
    format!("petri oracle updates expire {month} --claim {key}")
}

/// A bucket in a table: its name, or the start of its 64-hex id.
fn bucket_cell(source: &SourceRef) -> String {
    let name = source.bucket_name();
    if name == hex(&source.state.bucket_id) {
        format!("{}…", &name[..8])
    } else {
        name
    }
}

fn value(atoms: u64) -> String {
    trim_decimal(u128::from(atoms), 6)
}

// ------------------------------------------------------------ chain reads

/// Where an update claim's and an update challenge's account markers sit
/// (`UC2`, `UCH`): a scan filters on them (a test holds them to the layouts).
const CLAIM_MARKER_AT: usize = 260;
const CHALLENGE_MARKER_AT: usize = 268;

impl Oracle<'_> {
    /// Every update claim of `month`, or of every month: one program scan by
    /// size and marker (and the month at offset 2), each claim decoded and
    /// at its canonical address.
    pub fn update_claims(
        &self,
        month: Option<&Pubkey>,
    ) -> Result<Vec<(Pubkey, OracleUpdateClaimV2)>> {
        let mut filters = vec![
            Filter::DataSize(OracleUpdateClaimV2::LEN as u64),
            Filter::Memcmp {
                offset: CLAIM_MARKER_AT,
                bytes: OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR.to_vec(),
            },
        ];
        if let Some(month) = month {
            filters.push(Filter::Memcmp {
                offset: 2,
                bytes: month.to_bytes().to_vec(),
            });
        }
        let mut out = Vec::new();
        for (key, account) in self.program_accounts(&filters)? {
            let Ok(state) = decode::<OracleUpdateClaimV2>(
                &self.program,
                &key,
                Some(&account),
                OracleUpdateClaimV2::LEN,
                "update claim",
            ) else {
                continue;
            };
            let c = &state.claim;
            if c.is_initialized
                && c.account_discriminator == OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR
                && month.is_none_or(|m| c.month == *m)
                && ix::find_oracle_update_claim_address(
                    &self.program,
                    &c.month,
                    &c.source,
                    &c.claimant,
                    &c.claim_id,
                ) == key
            {
                out.push((key, state));
            }
        }
        out.sort_by_key(|(key, c)| (c.commit_slot, *key));
        Ok(out)
    }

    /// Every update challenge of `month`, or of every month, as
    /// [`Self::update_claims`] reads claims.
    pub fn update_challenges(
        &self,
        month: Option<&Pubkey>,
    ) -> Result<Vec<(Pubkey, OracleUpdateChallenge)>> {
        let mut filters = vec![
            Filter::DataSize(OracleUpdateChallenge::LEN as u64),
            Filter::Memcmp {
                offset: CHALLENGE_MARKER_AT,
                bytes: OracleUpdateChallenge::ACCOUNT_DISCRIMINATOR.to_vec(),
            },
        ];
        if let Some(month) = month {
            filters.push(Filter::Memcmp {
                offset: 2,
                bytes: month.to_bytes().to_vec(),
            });
        }
        let mut out = Vec::new();
        for (key, account) in self.program_accounts(&filters)? {
            let Ok(c) = decode::<OracleUpdateChallenge>(
                &self.program,
                &key,
                Some(&account),
                OracleUpdateChallenge::LEN,
                "update challenge",
            ) else {
                continue;
            };
            if c.is_initialized
                && c.has_valid_account_layout()
                && month.is_none_or(|m| c.month == *m)
                && ix::find_oracle_update_challenge_address(
                    &self.program,
                    &c.month,
                    &c.claim,
                    &c.challenge_id,
                ) == key
            {
                out.push((key, c));
            }
        }
        out.sort_by_key(|(key, c)| (c.rule_review_slot, *key));
        Ok(out)
    }

    /// The challenge of one claim, if it has one: its guard names it (one
    /// challenge per claim).
    pub fn update_challenge_of(
        &self,
        month: &Pubkey,
        claim: &Pubkey,
    ) -> Result<Option<(Pubkey, OracleUpdateChallenge)>> {
        let guard = ix::find_oracle_update_guard_address(&self.program, month, claim);
        let Some(account) = self.ctx.rpc.account(&guard)? else {
            return Ok(None);
        };
        // The guard names its challenge after its marker, version, month and claim.
        let d = &account.data;
        if account.owner != self.program || d.len() < 6 + 32 + 32 + 32 + 32 {
            bail(format!("challenge guard {guard} is not the oracle's"))?;
        }
        let challenge = Pubkey::new_from_array(d[102..134].try_into().unwrap());
        let account = self.ctx.rpc.account(&challenge)?;
        let state: OracleUpdateChallenge = decode(
            &self.program,
            &challenge,
            account.as_ref(),
            OracleUpdateChallenge::LEN,
            "update challenge",
        )?;
        if state.claim != *claim || state.month != *month {
            bail(format!(
                "challenge {challenge} named by guard {guard} is not of claim {claim}"
            ))?;
        }
        Ok(Some((challenge, state)))
    }

    /// The archive evidence each `(payer, archive hash)` names, in order (one
    /// batched read): the text of the payer's sealed archive (kind 3) or
    /// evidence record (kind 4) object committed to that hash, which a claim
    /// or challenge stores only as its hash. None where neither exists.
    pub fn archives(&self, items: &[(Pubkey, [u8; 32])]) -> Result<Vec<Option<String>>> {
        let keys: Vec<Pubkey> = items
            .iter()
            .flat_map(|(payer, hash)| chain::archive_objects(&self.program, payer, hash))
            .collect();
        let accounts = self.ctx.rpc.accounts(&keys)?;
        Ok(items
            .iter()
            .enumerate()
            .map(|(i, (payer, hash))| {
                if *hash == [0; 32] {
                    return None;
                }
                (0..2).find_map(|j| {
                    let (by, text) = chain::archive_in(
                        &self.program,
                        &keys[2 * i + j],
                        accounts[2 * i + j].as_ref()?,
                        hash,
                    )?;
                    (by == *payer).then_some(text)
                })
            })
            .collect())
    }
}

// ------------------------------------------------------------------ checks

/// What a commit must meet, by the program's rules (166, and the reveal's
/// evidence rules, 198, since only a hash goes on chain at the commit): the
/// month takes updates now, the source is active with a value, the new value
/// differs from it (the program refuses to reveal an unchanged value), and
/// the evidence is of the source's own page, captured from listing up to the
/// chain's clock and by expiry.
fn check_commit(
    game: &Game,
    now: u64,
    month: &str,
    source: &SourceRef,
    new_value: u64,
    cited: &str,
) -> Result<()> {
    game.check_updates(now, month)?;
    let s = &source.state;
    if s.status != OracleSourceStatus::Active {
        bail(format!(
            "source {} is {:?}: only an active source takes updates",
            source.address, s.status
        ))?;
    }
    if s.current_state == 0 {
        bail(format!(
            "source {} has no value yet, so it takes no updates",
            source.address
        ))?;
    }
    if s.latest_source_time >= game.expiry || s.observation_count == u32::MAX {
        bail(format!(
            "source {} has its last print already (at {}), so it takes no more updates",
            source.address,
            at(s.latest_source_time)
        ))?;
    }
    if new_value == s.current_state {
        bail(format!(
            "{} is the source's current value: an update must change it (the program refuses to reveal an unchanged value, so the stake would only come back with expire)",
            value(new_value)
        ))?;
    }
    check_evidence(game, now, source, cited)
}

/// The program's `validate_oracle_update_evidence`: the evidence is of the
/// source's own page, its time from listing up to the chain's clock and by
/// expiry.
fn check_evidence(game: &Game, now: u64, source: &SourceRef, cited: &str) -> Result<()> {
    let (time, page) = archive_parts(cited)?;
    if time < game.listing {
        bail(format!(
            "the evidence was captured at {}, before the month listed at {}: capture the page again",
            at(time),
            at(game.listing)
        ))?;
    }
    if time > now {
        bail(format!(
            "the evidence was captured at {}, after the chain's clock ({}): use a capture up to the chain's clock, or wait for it to pass {}",
            at(time),
            at(now),
            at(time)
        ))?;
    }
    if time > game.expiry {
        bail(format!(
            "the evidence was captured at {}, after the month's expiry ({})",
            at(time),
            at(game.expiry)
        ))?;
    }
    if hashv(&[b"locator", page.as_bytes()]).to_bytes() != source.state.canonical_locator_hash {
        bail(format!(
            "{page} is not this source's page: cite its URL exactly as the source names it"
        ))?;
    }
    Ok(())
}

/// What a reveal must meet (198): the claim is still committed, inside its
/// reveal window (from one slot after the commit for 150 slots), and its
/// source still has the value it was committed against.
fn check_reveal(
    claim: &OracleUpdateClaimV2,
    prior: u64,
    source: &OracleSourceState,
    now: &Now,
    expire: &str,
) -> Result<()> {
    use OracleClaimStatus as S;
    let key_words = match claim.claim.status {
        S::Open | S::Committed => None,
        S::Revealed => Some("it is revealed already"),
        S::Finalized => Some("it was revealed and accepted already"),
        S::Rejected => Some("it was revealed and ruled on already"),
        S::TimedOut if claim.revealed_slot == 0 => {
            Some("it was not revealed in time and its stake went back")
        }
        S::TimedOut => Some("it was revealed and has timed out"),
    };
    if let Some(words) = key_words {
        bail(format!("nothing to reveal: {words}"))?;
    }
    if now.slot < claim.earliest_reveal_slot {
        bail(format!(
            "its reveal window opens at slot {} (about {}), one slot after the commit",
            claim.earliest_reveal_slot,
            at(now.time_of(claim.earliest_reveal_slot))
        ))?;
    }
    if now.slot > claim.reveal_deadline_slot {
        bail(format!(
            "its reveal window closed at slot {} (about {}): the stake comes back with `{expire}` (the keeper also refunds it)",
            claim.reveal_deadline_slot,
            at(now.time_of(claim.reveal_deadline_slot))
        ))?;
    }
    if source.status != OracleSourceStatus::Active {
        bail(format!(
            "its source is {:?} now: the program refuses the reveal; after slot {} the stake comes back with `{expire}`",
            source.status, claim.reveal_deadline_slot
        ))?;
    }
    if source.current_state != prior {
        bail(format!(
            "its source's value changed since the commit ({} then, {} now): the program refuses the reveal; after slot {} the stake comes back with `{expire}`",
            value(prior),
            value(source.current_state),
            claim.reveal_deadline_slot
        ))?;
    }
    Ok(())
}

/// What a challenge must meet (168): the claim is revealed and still live
/// (not overtaken by another update of its source, not finalized), not yet
/// challenged (one challenge per claim), not the challenger's own, and the
/// alternative is another value with evidence that holds.
#[allow(clippy::too_many_arguments)]
fn check_challenge(
    game: &Game,
    now: u64,
    month: &str,
    key: &Pubkey,
    claim: &OracleUpdateClaimV2,
    challenged: Option<&Pubkey>,
    challenger: &Pubkey,
    source: &SourceRef,
    alternative: u64,
    cited: &str,
) -> Result<()> {
    use OracleClaimStatus as S;
    game.check_challenges(now, month)?;
    match claim.claim.status {
        S::Revealed => {}
        S::Open | S::Committed => bail(format!(
            "claim {key} is not revealed yet: an update is challenged once it is revealed"
        ))?,
        S::Finalized => bail(format!(
            "claim {key} was accepted already: a claim is open to challenge only until it is accepted"
        ))?,
        status => bail(format!(
            "claim {key} has ended ({status:?}): it can't be challenged"
        ))?,
    }
    if let Some(challenge) = challenged {
        bail(format!(
            "claim {key} is already challenged (challenge {challenge}), one challenge per claim: `petri oracle updates` shows how it stands"
        ))?;
    }
    if claim.claim.claimant == *challenger {
        bail(format!(
            "you made claim {key}: its claimant can't challenge it"
        ))?;
    }
    if overtaken(claim, &source.state) {
        bail(format!(
            "claim {key} was overtaken: its source changed (or left the recipe) since, so it can't be accepted or challenged"
        ))?;
    }
    if alternative == claim.claim.new_state {
        bail(format!(
            "{} is the claim's own value: a challenge names another value",
            value(alternative)
        ))?;
    }
    check_evidence(game, now, source, cited)
}

/// What an expire must meet (167): a commitment nobody revealed, past its
/// reveal deadline, its stake not yet refunded.
fn check_expire(key: &Pubkey, claim: &OracleUpdateClaimV2, now: &Now) -> Result<()> {
    use OracleClaimStatus as S;
    match claim.claim.status {
        S::Open | S::Committed => {}
        S::TimedOut if claim.revealed_slot == 0 => {
            bail("its stake went back already: nothing to refund")?
        }
        _ => bail("it was revealed: only a commitment nobody revealed expires")?,
    }
    if claim.claim.escrow_disposition != OracleEscrowDisposition::Unsettled {
        bail("its stake went back already: nothing to refund")?;
    }
    if now.slot <= claim.reveal_deadline_slot {
        bail(format!(
            "it can still be revealed until slot {} (about {}): reveal it (`{}`), or expire it after that",
            claim.reveal_deadline_slot,
            at(now.time_of(claim.reveal_deadline_slot)),
            reveal_command(key)
        ))?;
    }
    Ok(())
}

/// The exact amount the program takes (`--stake`, `--bond`): `given` when it
/// says the same, else a refusal naming it.
fn exact(given: Option<&str>, amount: u64, what: &str, flag: &str) -> Result<u64> {
    match given {
        None => Ok(amount),
        Some(text) if usdc_amount(text, what)? == amount => Ok(amount),
        Some(_) => {
            let exact = trim_decimal(u128::from(amount), 6);
            bail(format!(
                "the {what} is exactly {exact} USDC: pass {flag} {exact}, or leave {flag} out"
            ))
        }
    }
}

/// The bond to challenge a claim, exactly (the program's
/// `calculate_oracle_usdc_challenge_bond` on the claim's stake).
pub fn challenge_bond(terms: &Terms, stake: u64) -> Result<u64> {
    terms.bond(stake, None)
}

fn bond_words(terms: &Terms, stake: u64) -> String {
    terms.words_of(&format!("the claim's {} USDC stake", usdc(stake)))
}

fn collateral_available(oracle: &Oracle, owner: &Pubkey) -> Result<u64> {
    Ok(oracle.collateral(owner)?.map_or(0, |l| l.available_balance))
}

fn need_collateral(available: u64, amount: u64, what: &str) -> Result<()> {
    if available < amount {
        bail(format!(
            "the {what} is {} USDC and your oracle collateral has {} USDC available: deposit first (`petri oracle collateral deposit <USDC>`)",
            usdc(amount),
            usdc(available)
        ))?;
    }
    Ok(())
}

fn source_url(oracle: &Oracle, source: &SourceRef) -> Result<Option<String>> {
    Ok(oracle
        .locators(std::slice::from_ref(source))?
        .pop()
        .flatten())
}

/// A card's common fields: the month, its source and the source's page.
fn card(month: &MonthRef, source: &SourceRef, url: Option<&str>) -> Value {
    json!({
        "month": month_json(month),
        "monthAddress": month.address.to_string(),
        "source": source_json(source),
        "sourceId": hex(&source.state.source_id),
        "url": url,
        "bucket": source.bucket_label(),
        "bucketName": source.bucket_name(),
    })
}

fn merge(into: &mut Value, more: Value) {
    if let (Some(into), Value::Object(more)) = (into.as_object_mut(), more) {
        into.extend(more);
    }
}

// ----------------------------------------------------------------- commits

/// What `updates commit` (and `observe` in Game) asks for.
pub struct CommitRequest<'a> {
    pub source: &'a str,
    pub value: Option<&'a str>,
    pub cited: Cite<'a>,
    pub stake: Option<&'a str>,
    pub reveal: bool,
}

/// `petri oracle updates commit`: tag 166, held to the program's rules
/// first, and with `--reveal` its reveal as soon as the chain takes it.
pub fn commit(ctx: &Ctx, p: Paint, target: &Target, r: &CommitRequest) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let source = oracle.source(&month, r.source)?;
    let claimant = ctx.wallet_pubkey()?;
    let c = cite(ctx, &oracle, &month, &source, r.cited, r.value, &claimant)?;
    let planned = commit_write(
        &oracle,
        &month,
        &source,
        &c,
        r.stake,
        target.send.simulate,
        None,
    )?;
    send_commit(ctx, p, planned, target.send.simulate, r.reveal)
}

/// A commit ready to send: its write, its claim, and what its card shows.
pub struct Planned {
    pub write: Write,
    pub claim: Pubkey,
    pub month: MonthRef,
    /// Petri's private claims file, when the reveal material was saved.
    pub saved_to: Option<std::path::PathBuf>,
    /// The reveal window's 150 slots in seconds, at the chain's slot length.
    pub window_seconds: u64,
}

/// A sealed update commit to review and send, held to the program's rules
/// ([`check_commit`], the exact stake, the collateral). Unless only
/// simulating, its reveal material (with the web proof it cites, if any,
/// and `observation`, the record to store once it is revealed) is saved to
/// Petri's private claims file first.
pub(super) fn commit_write(
    oracle: &Oracle,
    month: &MonthRef,
    source: &SourceRef,
    c: &Cited,
    stake: Option<&str>,
    simulate: bool,
    observation: Option<claims::PendingObservation>,
) -> Result<Planned> {
    let ctx = oracle.ctx;
    let claimant = ctx.wallet_pubkey()?;
    let id = month.period.expiry_id();
    let now = Now::read(ctx)?;
    let game = Game::of(month);
    let (new_state, source_time, archive_url) = (c.state, c.time, c.text.as_str());
    check_commit(&game, now.unix, &id, source, new_state, archive_url)?;
    let pool = oracle.sku_pool(month, &source.state.bucket_id)?;
    if pool.update_min_bond == 0 {
        bail(format!(
            "{} has no update stake for this product, so it takes no updates",
            id
        ))?;
    }
    let stake = exact(stake, pool.update_min_bond, "update stake", "--stake")?;
    let available = collateral_available(oracle, &claimant)?;
    need_collateral(available, stake, "update stake")?;
    let s = &source.state;
    let evidence_hash = ix::update_evidence_hash(
        &month.address,
        &source.address,
        &s.source_id,
        new_state,
        source_time,
        &s.canonical_locator_hash,
        &s.source_definition_hash,
        archive_url,
    );
    let claim_id = random32()?;
    let salt = random32()?;
    let commit_hash = oracle_update_claim_v2_commitment_hash(
        &oracle.program,
        &month.address,
        &source.address,
        &s.source_id,
        &claimant,
        &claim_id,
        s.current_state,
        new_state,
        source_time,
        &evidence_hash,
        &ix::archive_url_hash(archive_url),
        &salt,
    );
    let claim = ix::find_oracle_update_claim_address(
        &oracle.program,
        &month.address,
        &source.address,
        &claimant,
        &claim_id,
    );
    let saved_to = if simulate {
        None
    } else {
        Some(claims::save(
            &claim.to_string(),
            claims::Claim {
                oracle: oracle.program.to_string(),
                month: month.address.to_string(),
                source: source.address.to_string(),
                claimant: claimant.to_string(),
                claim_id: hex(&claim_id),
                prior_state: s.current_state,
                new_state,
                source_time,
                evidence_hash: hex(&evidence_hash),
                archive_url: archive_url.to_string(),
                secret_salt: hex(&salt),
                revealed: false,
                evidence_bundle: c.bundle.clone(),
                observation,
            },
        )?)
    };
    let url = source_url(oracle, source)?;
    // The window opens one slot after the commit lands; it lands a slot or
    // two from now.
    let reveal_by = now.time_of(
        now.slot + 1 + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS + ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
    );
    let mut details = card(month, source, url.as_deref());
    merge(
        &mut details,
        json!({
            "claim": claim.to_string(),
            "claimId": hex(&claim_id),
            "prior": value(s.current_state),
            "value": value(new_state),
            "valueAtoms": new_state.to_string(),
            "sourceTimeUtc": iso(source_time),
            "stakeAtoms": stake.to_string(),
            "evidence": archive_url,
            "status": Standing::Committed.code(false),
            "statusText": Standing::Committed.words(false, false),
            "committedUtc": iso(now.unix),
            "revealByUtc": iso(reveal_by),
            "revealedUtc": null,
            "acceptFromUtc": null,
            "revealMaterial": saved_to.as_ref().map(|p| p.display().to_string()),
            "revealCommand": reveal_command(&claim),
            "nextCommand": reveal_command(&claim),
        }),
    );
    let write = Write {
        action: "oracle.updates.commit",
        title: format!(
            "Commit a sealed update of source {} with a {} USDC stake",
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
            format!(
                "{} → {} as of {}",
                value(s.current_state),
                value(new_state),
                at(source_time)
            ),
            format!(
                "Stake {} USDC from your oracle collateral ({} available): the product's update stake",
                usdc(stake),
                usdc(available)
            ),
            format!("Claim {claim}"),
            "Only the commitment goes on chain now; the value, evidence and salt stay in Petri's private file until the reveal".to_string(),
        ],
        details,
        txs: vec![Tx::one(
            "commitUpdate",
            ix::commit_oracle_update_claim_v3(
                &oracle.program,
                &claimant,
                &month.period.address,
                &month.address,
                &source.address,
                &s.bucket_id,
                CommitOracleUpdateClaimV2Params {
                    claim_id,
                    commit_hash,
                    stake,
                },
            ),
        )],
    };
    Ok(Planned {
        write,
        claim,
        month: month.clone(),
        saved_to,
        window_seconds: ORACLE_UPDATE_REVEAL_WINDOW_SLOTS * now.slot_ms / 1_000,
    })
}

/// The slot a sent write's first transaction landed in.
fn landed_slot(report: &Report) -> Option<u64> {
    report.payload["transactions"]
        .as_array()?
        .first()?
        .get("slot")?
        .as_u64()
}

/// Sends a planned commit and, with `reveal`, reveals it as soon as the
/// chain takes the reveal. A reveal that fails leaves the commit standing:
/// the result says so, with the reveal command and its deadline.
pub(super) fn send_commit(
    ctx: &Ctx,
    p: Paint,
    mut planned: Planned,
    simulate: bool,
    reveal: bool,
) -> Result<Report> {
    let claim = planned.claim;
    let id = planned.month.period.expiry_id();
    let window = planned.window_seconds;
    if reveal {
        planned.write.review.push(format!(
            "Then reveals it at once, from the slot after the commit (its window is 150 slots, about {window} s here): the value and its evidence go on chain then"
        ));
    } else {
        planned.write.review.push(format!(
            "Reveal it within 150 slots (about {window} s here): `{}`, or commit with --reveal to do both at once",
            reveal_command(&claim)
        ));
    }
    let mut report = send::run(ctx, p, planned.write, simulate)?;
    if simulate {
        if reveal {
            report.plain.push_str(
                "\n  With --reveal, petri reveals it right after the commit lands (not simulated: the claim is not on chain yet).",
            );
            report.payload["reveal"] = json!({
                "simulated": false,
                "note": "revealed right after the commit lands; not simulated, the claim is not on chain yet",
            });
        }
        return Ok(report);
    }
    let oracle = Oracle::new(ctx)?;
    let landed = landed_slot(&report).unwrap_or(0);
    // When the window closes: the claim's own deadline once it reads back.
    let deadline_of = |state: Option<&OracleUpdateClaimV2>| {
        state.map_or(
            landed + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS + ORACLE_UPDATE_REVEAL_WINDOW_SLOTS,
            |s| s.reveal_deadline_slot,
        )
    };
    let reveal_by = |deadline: u64| {
        Now::read(ctx)
            .map(|now| (deadline, now.time_of(deadline)))
            .unwrap_or((deadline, 0))
    };
    if !reveal {
        let state = update_claim(&oracle, &claim.to_string())
            .ok()
            .map(|(_, s)| s);
        if let (Ok(now), Some(state)) = (Now::read(ctx), &state) {
            report.payload["details"]["committedUtc"] = iso(now.time_of(state.commit_slot));
        }
        let (slot, by) = reveal_by(deadline_of(state.as_ref()));
        report.payload["details"]["revealByUtc"] = iso(by);
        report.plain.push_str(&format!(
            "\nReveal it by {} (slot {slot}): {}",
            at(by),
            reveal_command(&claim)
        ));
        return Ok(report);
    }
    report.payload["revealed"] = json!(false);
    // Straight on to the reveal: its window is short, so nothing else is read first.
    let known = Known {
        month: &planned.month,
        url: report.payload["details"]["url"]
            .as_str()
            .map(str::to_string),
    };
    let mut deadline = deadline_of(None);
    let revealed = await_reveal_slot(ctx, &oracle, &claim, landed).and_then(|state| {
        deadline = state.reveal_deadline_slot;
        reveal_send(ctx, p, &claim.to_string(), false, true, Some(&known))
    });
    match revealed {
        Ok(revealed) => {
            report.payload["revealed"] = json!(true);
            for key in [
                "status",
                "statusText",
                "committedUtc",
                "revealByUtc",
                "revealedUtc",
                "acceptFromUtc",
                "nextCommand",
                "revealCommand",
                "settleCommand",
            ] {
                report.payload["details"][key] = revealed.payload["details"][key].clone();
            }
            report.plain.push('\n');
            report.plain.push_str(&revealed.plain);
            report.payload["reveal"] = revealed.payload;
            Ok(report)
        }
        Err(e) => {
            let (slot, by) = reveal_by(deadline);
            let expire = expire_command(&id, &claim);
            let saved = planned
                .saved_to
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let why = e.message.replace('\n', " | ");
            report.plain.push_str(&format!(
                "\n{}\n  Your reveal material stays in {saved}.\n  Reveal it by {} (slot {slot}): {}\n  After that the stake comes back: {expire} (the keeper also refunds it).",
                p.red(&format!("The commit landed, but the reveal failed: {why}")),
                at(by),
                reveal_command(&claim),
            ));
            report.payload["ok"] = json!(false);
            report.payload["error"] = json!({
                "code": "ORACLE_REVEAL_FAILED",
                "category": "execution_failed",
                "message": format!("the commit landed, but the reveal failed: {why}"),
                "retryable": true,
            });
            report.payload["reveal"] = Value::Null;
            let details = &mut report.payload["details"];
            details["revealByUtc"] = iso(by);
            details["revealCommand"] = json!(reveal_command(&claim));
            details["expireCommand"] = json!(expire);
            details["nextCommand"] = json!(reveal_command(&claim));
            Ok(report)
        }
    }
}

/// What a commit just read, so the reveal right after it reads no more
/// than it must: the month and the source's page.
struct Known<'a> {
    month: &'a MonthRef,
    url: Option<String>,
}

/// Waits until the chain reaches the claim's first reveal slot (one slot
/// after the commit landed at `landed`), reading the claim as the RPC
/// confirms it; refuses once its reveal window has closed.
fn await_reveal_slot(
    ctx: &Ctx,
    oracle: &Oracle,
    claim: &Pubkey,
    landed: u64,
) -> Result<OracleUpdateClaimV2> {
    let deadline =
        landed + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS + ORACLE_UPDATE_REVEAL_WINDOW_SLOTS;
    loop {
        let slot = ctx.rpc.slot()?;
        let state = update_claim(oracle, &claim.to_string())
            .ok()
            .map(|(_, s)| s);
        let (earliest, last) = state.as_ref().map_or(
            (landed + ORACLE_UPDATE_MIN_REVEAL_DELAY_SLOTS, deadline),
            |s| (s.earliest_reveal_slot, s.reveal_deadline_slot),
        );
        if slot > last {
            bail(format!(
                "its reveal window closed at slot {last} before petri could reveal it"
            ))?;
        }
        if let Some(state) = state
            && slot >= earliest
        {
            return Ok(state);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

// ----------------------------------------------------------------- reveals

/// `petri oracle updates reveal <CLAIM>`: tag 198, from Petri's private file.
pub fn reveal(ctx: &Ctx, p: Paint, claim: &str, simulate: bool) -> Result<Report> {
    reveal_send(ctx, p, claim, simulate, false, None)
}

fn reveal_send(
    ctx: &Ctx,
    p: Paint,
    claim: &str,
    simulate: bool,
    confirmed: bool,
    known: Option<&Known>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let claimant = ctx.wallet_pubkey()?;
    let (key, state) = update_claim(&oracle, claim)?;
    let saved = claims::load(&key.to_string())?;
    if saved.claimant != claimant.to_string()
        || state.claim.claimant != claimant
        || saved.oracle != oracle.program.to_string()
    {
        bail(format!(
            "update claim {key} was committed by {}, not this wallet",
            state.claim.claimant
        ))?;
    }
    let month = match known {
        Some(k) if k.month.address == state.claim.month => k.month.clone(),
        _ => oracle.month_at(&state.claim.month)?,
    };
    let id = month.period.expiry_id();
    let source = oracle.source(&month, &state.claim.source.to_string())?;
    let now = Now::read(ctx)?;
    Game::of(&month).check_updates(now.unix, &id)?;
    check_reveal(
        &state,
        saved.prior_state,
        &source.state,
        &now,
        &expire_command(&id, &key),
    )?;
    let (kind, bytes, hash) =
        evidence_preimage(cited_kind(&saved.archive_url), &saved.archive_url)?;
    let (archive_object, mut txs, _) = evidence_txs(&oracle, &claimant, kind, &bytes, hash)?;
    if let Some(digest) = &saved.evidence_bundle
        && !simulate
    {
        // Revealed, the claim names the proof: publish it so anyone can
        // check it during the challenge window.
        crate::evidence::onchain::publish(ctx, digest)?;
    }
    txs.push(Tx::one(
        "revealUpdate",
        ix::reveal_oracle_update_claim_v3(
            &oracle.program,
            &claimant,
            &claimant,
            &month.period.address,
            &month.address,
            &state.claim.source,
            RevealOracleUpdateClaimV3Params {
                claim_id: state.claim.claim_id,
                prior_state: saved.prior_state,
                new_state: saved.new_state,
                source_time: saved.source_time,
                evidence_hash: parse_hex32(&saved.evidence_hash, "evidence hash")?,
                archive_url: String::new(),
                secret_salt: parse_hex32(&saved.secret_salt, "salt")?,
            },
            &saved.archive_url,
        ),
    ));
    let seconds = challenge_seconds(ctx.deployment()?);
    let url = match known {
        Some(k) => k.url.clone(),
        None => source_url(&oracle, &source)?,
    };
    let mut details = card(&month, &source, url.as_deref());
    merge(
        &mut details,
        json!({
            "claim": key.to_string(),
            "claimId": hex(&state.claim.claim_id),
            "prior": value(saved.prior_state),
            "value": value(saved.new_state),
            "valueAtoms": saved.new_state.to_string(),
            "sourceTimeUtc": iso(saved.source_time),
            "stakeAtoms": state.claim.stake.to_string(),
            "evidence": saved.archive_url,
            "evidenceObject": archive_object.to_string(),
            "committedUtc": iso(claim_time(&now, &state, state.commit_slot)),
            "revealByUtc": iso(claim_time(&now, &state, state.reveal_deadline_slot)),
            "revealedUtc": iso(now.unix),
            "acceptFromUtc": iso(now.unix.saturating_add(seconds)),
            "status": Standing::Challengeable { due: false }.code(false),
            "statusText": Standing::Challengeable { due: false }.words(false, false),
            "revealCommand": null,
            "settleCommand": null,
            "nextCommand": format!("petri oracle updates {id}"),
        }),
    );
    let write = Write {
        action: "oracle.updates.reveal",
        title: format!("Reveal update claim {key}"),
        review: vec![
            month_line(&month),
            format!(
                "{} → {} as of {}",
                value(saved.prior_state),
                value(saved.new_state),
                at(saved.source_time)
            ),
            format!(
                "Evidence {} (object {archive_object})",
                terminal_safe_text(&saved.archive_url)
            ),
            format!(
                "Open to challenge for {}; unchallenged, the keeper accepts it then, and settling it returns the stake",
                crate::futures::view::span(i64::try_from(seconds).unwrap_or(i64::MAX))
            ),
        ],
        details,
        txs,
    };
    let mut report = if confirmed {
        send::run_confirmed(ctx, p, write)?
    } else {
        send::run(ctx, p, write, simulate)?
    };
    if !simulate {
        claims::mark_revealed(&key.to_string())?;
        // The revealed claim's own times, on the chain's clock.
        if let Ok((_, revealed)) = update_claim(&oracle, &key.to_string()) {
            report.payload["details"]["revealedUtc"] = iso(revealed.revealed_at_ts);
            if let Some(t) = accept_from(&revealed, seconds) {
                report.payload["details"]["acceptFromUtc"] = iso(t);
                report.plain.push_str(&format!(
                    "\nOpen to challenge until it is accepted, from {}: petri oracle updates {id}\nOnce it is accepted, its stake comes back with: petri oracle stakes settle {id} --kind update-claim --source {} --claim {key}",
                    at(t),
                    state.claim.source
                ));
            }
        }
        // An update sent by `petri oracle observe`: its observation record goes
        // to the API now that the claim is revealed.
        if let Some(pending) = &saved.observation {
            super::observe::record_after_send(ctx, &mut report, pending, &key, &claimant);
        }
    }
    Ok(report)
}

// -------------------------------------------------------------- challenges

/// What `petri oracle updates challenge` asks for.
pub struct ChallengeRequest<'a> {
    pub claim: &'a str,
    pub value: Option<&'a str>,
    pub cited: Cite<'a>,
    pub bond: Option<&'a str>,
    pub challenge_id: Option<&'a str>,
}

/// `petri oracle updates challenge`: tag 168, held to the program's rules
/// first, with the exact bond.
pub fn challenge(ctx: &Ctx, p: Paint, target: &Target, r: &ChallengeRequest) -> Result<Report> {
    let write = challenge_write(ctx, target, r)?;
    send::run(ctx, p, write, target.send.simulate)
}

fn challenge_write(ctx: &Ctx, target: &Target, r: &ChallengeRequest) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let challenger = ctx.wallet_pubkey()?;
    let (key, state) = update_claim(&oracle, r.claim)?;
    let month = claim_month(&oracle, target, &state.claim.month)?;
    let id = month.period.expiry_id();
    let source = oracle.source(&month, &state.claim.source.to_string())?;
    let c = cite(ctx, &oracle, &month, &source, r.cited, r.value, &challenger)?;
    let (alternative, source_time, archive_url) = (c.state, c.time, c.text.as_str());
    let now = Now::read(ctx)?;
    let existing = oracle.update_challenge_of(&month.address, &key)?;
    check_challenge(
        &Game::of(&month),
        now.unix,
        &id,
        &key,
        &state,
        existing.as_ref().map(|(k, _)| k),
        &challenger,
        &source,
        alternative,
        archive_url,
    )?;
    let terms = Terms::of(&oracle.sku_pool(&month, &source.state.bucket_id)?);
    let bond = challenge_bond(&terms, state.claim.stake)?;
    let bond = exact(r.bond, bond, "bond", "--bond").map_err(|e| {
        Error::msg(format!(
            "{} ({})",
            e.message,
            bond_words(&terms, state.claim.stake)
        ))
    })?;
    let available = collateral_available(&oracle, &challenger)?;
    need_collateral(available, bond, "bond")?;
    let challenge_id = id_or_random(r.challenge_id, "challenge id")?;
    let challenge = ix::find_oracle_update_challenge_address(
        &oracle.program,
        &month.address,
        &key,
        &challenge_id,
    );
    if ctx.rpc.account(&challenge)?.is_some() {
        bail(format!(
            "challenge id {} is taken (challenge {challenge}): leave --challenge-id out for a new one",
            hex(&challenge_id)
        ))?;
    }
    let s = &source.state;
    let evidence_hash = ix::update_evidence_hash(
        &month.address,
        &source.address,
        &s.source_id,
        alternative,
        source_time,
        &s.canonical_locator_hash,
        &s.source_definition_hash,
        archive_url,
    );
    let (kind, bytes, hash) = evidence_preimage(c.kind, archive_url)?;
    let (_, mut txs, _) = evidence_txs(&oracle, &challenger, kind, &bytes, hash)?;
    if let Some(digest) = &c.bundle
        && !target.send.simulate
    {
        crate::evidence::onchain::publish(ctx, digest)?;
    }
    txs.push(Tx::one(
        "challengeUpdate",
        ix::challenge_oracle_update_claim_v2(
            &oracle.program,
            &challenger,
            &challenger,
            &month.period.address,
            &month.address,
            &source.address,
            &s.bucket_id,
            &key,
            ChallengeOracleUpdateClaimParams {
                challenge_id,
                alternative_state: alternative,
                alternative_source_time: source_time,
                bond,
                evidence_hash,
                archive_url: String::new(),
            },
            archive_url,
        ),
    ));
    let seconds = challenge_seconds(ctx.deployment()?);
    let url = source_url(&oracle, &source)?;
    let claim_evidence = oracle
        .archives(&[(state.claim.claimant, state.claim.archive_url_hash)])?
        .pop()
        .flatten();
    let pot = state.claim.stake.saturating_add(bond);
    let mut details = card(&month, &source, url.as_deref());
    merge(
        &mut details,
        json!({
            "claim": key.to_string(),
            "claimId": hex(&state.claim.claim_id),
            "claimant": state.claim.claimant.to_string(),
            "challenge": challenge.to_string(),
            "challengeId": hex(&challenge_id),
            "prior": value(state.claim.prior_state),
            "claimValue": value(state.claim.new_state),
            "claimEvidence": claim_evidence,
            "value": value(alternative),
            "valueAtoms": alternative.to_string(),
            "sourceTimeUtc": iso(source_time),
            "evidence": archive_url,
            "stakeAtoms": state.claim.stake.to_string(),
            "bondAtoms": bond.to_string(),
            "revealedUtc": iso(state.revealed_at_ts),
            "acceptFromUtc": accept_from(&state, seconds).map(iso),
            "status": Ruling::Waiting.code(false),
            "statusText": Ruling::Waiting.words(false),
            "nextCommand": format!("petri oracle updates {id}"),
        }),
    );
    Ok(Write {
        action: "oracle.updates.challenge",
        title: format!(
            "Challenge update claim {key} with {} ({} USDC bond)",
            value(alternative),
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
                "The claim: {} → {} by {}; your value {} as of {}",
                value(state.claim.prior_state),
                value(state.claim.new_state),
                state.claim.claimant,
                value(alternative),
                at(source_time)
            ),
            format!(
                "Bond {} USDC from your oracle collateral ({} available): {}",
                usdc(bond),
                usdc(available),
                bond_words(&terms, state.claim.stake)
            ),
            format!("Challenge {challenge}"),
            "The oracle authority rules on it by hand (accept the claim, reject it, or send it to the council); the winner takes the claim's stake and your bond".to_string(),
            format!(
                "Upheld: {} USDC to you; dismissed: your bond goes to the claimant",
                usdc(pot)
            ),
        ],
        details,
        txs,
    })
}

// ------------------------------------------------------------------ expiry

/// `petri oracle updates expire`: tag 167, once the commitment's reveal
/// window has closed unrevealed. The stake goes straight back to the
/// claimant's collateral.
pub fn expire(ctx: &Ctx, p: Paint, target: &Target, claim: &str) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let (key, state) = update_claim(&oracle, claim)?;
    let month = claim_month(&oracle, target, &state.claim.month)?;
    let source = oracle.source(&month, &state.claim.source.to_string())?;
    let now = Now::read(ctx)?;
    check_expire(&key, &state, &now)?;
    let url = source_url(&oracle, &source)?;
    let mut details = card(&month, &source, url.as_deref());
    merge(
        &mut details,
        json!({
            "claim": key.to_string(),
            "claimId": hex(&state.claim.claim_id),
            "claimant": state.claim.claimant.to_string(),
            "stakeAtoms": state.claim.stake.to_string(),
            "committedUtc": iso(claim_time(&now, &state, state.commit_slot)),
            "revealByUtc": iso(claim_time(&now, &state, state.reveal_deadline_slot)),
            "status": Standing::Expired.code(false),
            "statusText": Standing::Expired.words(true, false),
            "nextCommand": "petri oracle collateral",
        }),
    );
    let write = Write {
        action: "oracle.updates.expire",
        title: format!("Refund unrevealed update claim {key}"),
        review: vec![
            month_line(&month),
            format!(
                "Its {} USDC stake goes back to {}'s oracle collateral",
                usdc(state.claim.stake),
                state.claim.claimant
            ),
            format!(
                "Its reveal window closed at slot {} (about {})",
                state.reveal_deadline_slot,
                at(claim_time(&now, &state, state.reveal_deadline_slot))
            ),
        ],
        details,
        txs: vec![Tx::one(
            "settleExpiredCommitment",
            ix::settle_expired_oracle_update_commitment_v3(
                &oracle.program,
                &cranker,
                &month.period.address,
                &month.address,
                &state.claim.source,
                &key,
                &state.claim.claimant,
            ),
        )],
    };
    send::run(ctx, p, write, target.send.simulate)
}

// -------------------------------------------------------------------- read

/// One claim listed for the owner, and whose money is in it.
struct Row<'a> {
    key: Pubkey,
    claim: &'a OracleUpdateClaimV2,
    challenge: Option<(Pubkey, &'a OracleUpdateChallenge)>,
    role: Option<&'static str>,
}

/// `petri oracle updates [MARKET] [--owner]`: the owner's update claims and
/// their challenges by others, and the owner's challenges of others' claims,
/// how each stands or ended and what comes next; then the updates open to
/// challenge now and the sources taking an update now. Without `--owner` or
/// a wallet it lists every claim whose money is still in escrow. Every
/// month is read with one program scan per account type (sources, claims,
/// challenges) and batched account reads.
pub fn list(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::reads(ctx)?;
    let seconds = challenge_seconds(ctx.deployment()?);
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
    let now = Now::read(ctx)?;
    let (sources, mut all_claims, mut all_challenges) = if months.is_empty() {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        (
            oracle.sources_in(single.as_ref())?,
            oracle.update_claims(single.as_ref())?,
            oracle.update_challenges(single.as_ref())?,
        )
    };
    all_claims.retain(|(_, c)| month_of.contains_key(&c.claim.month));
    all_challenges.retain(|(_, c)| month_of.contains_key(&c.month));
    let source_of: HashMap<Pubkey, &SourceRef> = sources.iter().map(|s| (s.address, s)).collect();
    let challenge_of: HashMap<Pubkey, (Pubkey, &OracleUpdateChallenge)> = all_challenges
        .iter()
        .map(|(key, c)| (c.claim, (*key, c)))
        .collect();
    // This Petri's reveal material: a committed claim's value before its
    // reveal (never its salt).
    let saved = claims::all().unwrap_or_default();
    let when = |month: &MonthRef| When {
        slot: now.slot,
        now: now.unix,
        challenge_seconds: seconds,
        grace_end: Game::of(month).grace_end(),
    };
    let standing = |key: &Pubkey, c: &OracleUpdateClaimV2| {
        let challenge = challenge_of.get(key);
        Standing::of(
            c,
            challenge.map(|(_, ch)| ch.status),
            source_of.get(&c.claim.source).map(|s| &s.state),
            when(month_of[&c.claim.month]),
        )
    };

    // The owner's claims and challenges; without an owner, every claim whose
    // money is still in escrow.
    let mut rows: Vec<Row> = Vec::new();
    for (key, c) in &all_claims {
        let challenge = challenge_of.get(key).copied();
        let role = match owner {
            Some(o) if c.claim.claimant == o => Some("yours"),
            Some(o) if challenge.is_some_and(|(_, ch)| ch.challenger == o) => Some("challenging"),
            Some(_) => continue,
            None if settled(c, challenge.map(|(_, ch)| ch)) => continue,
            None => None,
        };
        rows.push(Row {
            key: *key,
            claim: c,
            challenge,
            role,
        });
    }
    rows.sort_by_key(|r| {
        (
            month_of[&r.claim.claim.month].period.period.expiry_ts,
            r.claim.commit_slot,
            r.key,
        )
    });

    // Others' revealed updates open to challenge now.
    let open: Vec<(Pubkey, &OracleUpdateClaimV2)> = all_claims
        .iter()
        .filter(|(key, c)| {
            matches!(standing(key, c), Standing::Challengeable { .. })
                && Game::of(month_of[&c.claim.month]).takes_challenges(now.unix)
                && owner.is_none_or(|o| c.claim.claimant != o)
        })
        .map(|(key, c)| (*key, c))
        .collect();

    // The sources of months taking updates now.
    let game_months: Vec<&MonthRef> = months
        .iter()
        .filter(|m| Game::of(m).takes_updates(now.unix))
        .collect();
    let in_game: HashSet<Pubkey> = game_months.iter().map(|m| m.address).collect();
    let mut takers: Vec<&SourceRef> = sources
        .iter()
        .filter(|s| {
            let st = &s.state;
            in_game.contains(&st.month)
                && st.status == OracleSourceStatus::Active
                && st.current_state != 0
                && st.observation_count < u32::MAX
                && st.latest_source_time < month_of[&st.month].period.period.expiry_ts
        })
        .collect();
    takers.sort_by_key(|s| {
        let month = month_of[&s.state.month];
        (
            month.period.period.expiry_ts,
            month.period.symbol.clone(),
            s.state.bucket_id,
            s.address,
        )
    });
    // A chip month's update is an observation (`petri oracle observe`): its
    // coverage commits to chip buckets (one batched read of the coverages).
    let mut chip: HashMap<Pubkey, bool> = HashMap::new();
    for (m, coverage) in game_months.iter().zip(oracle.coverages(&game_months)?) {
        let of_month = || {
            Ok(sources
                .iter()
                .filter(|s| s.state.month == m.address)
                .cloned()
                .collect())
        };
        let is_chip = coverage.is_some_and(|c| {
            chain::chip_of(&m.period.symbol, &c.required_sku_root, of_month)
                .ok()
                .flatten()
                .is_some()
        });
        chip.insert(m.address, is_chip);
    }

    // Pools (stakes and bonds), URLs and evidence, each in batched reads.
    let mut pool_keys: Vec<(Pubkey, [u8; 32])> = takers
        .iter()
        .map(|s| (s.state.month, s.state.bucket_id))
        .chain(open.iter().filter_map(|(_, c)| {
            source_of
                .get(&c.claim.source)
                .map(|s| (c.claim.month, s.state.bucket_id))
        }))
        .collect();
    pool_keys.sort();
    pool_keys.dedup();
    let pools: HashMap<(Pubkey, [u8; 32]), OracleUsdcSkuPool> = pool_keys
        .iter()
        .zip(oracle.sku_pools(&pool_keys)?)
        .filter_map(|(k, pool)| Some((*k, pool?)))
        .collect();
    let mut listed: Vec<SourceRef> = rows
        .iter()
        .map(|r| r.claim.claim.source)
        .chain(open.iter().map(|(_, c)| c.claim.source))
        .chain(takers.iter().map(|s| s.address))
        .filter_map(|a| source_of.get(&a).map(|s| (*s).clone()))
        .collect();
    listed.sort_by_key(|s| s.address);
    listed.dedup_by_key(|s| s.address);
    let urls: HashMap<Pubkey, String> = listed
        .iter()
        .zip(oracle.locators(&listed)?)
        .filter_map(|(s, url)| Some((s.address, url?)))
        .collect();
    // Only for claims whose money is still in escrow: an owner's settled
    // history can run to thousands of claims, two evidence reads each.
    let mut wanted: Vec<(Pubkey, [u8; 32])> = rows
        .iter()
        .filter(|r| !settled(r.claim, r.challenge.map(|(_, ch)| ch)))
        .flat_map(|r| {
            [
                Some((r.claim.claim.claimant, r.claim.claim.archive_url_hash)),
                r.challenge
                    .map(|(_, ch)| (ch.challenger, ch.archive_url_hash)),
            ]
        })
        .flatten()
        .chain(
            open.iter()
                .map(|(_, c)| (c.claim.claimant, c.claim.archive_url_hash)),
        )
        .filter(|(_, hash)| *hash != [0; 32])
        .collect();
    wanted.sort();
    wanted.dedup();
    let evidence: HashMap<(Pubkey, [u8; 32]), String> = wanted
        .iter()
        .zip(oracle.archives(&wanted)?)
        .filter_map(|(k, text)| Some((*k, text?)))
        .collect();
    let evidence_of = |payer: Pubkey, hash: [u8; 32]| evidence.get(&(payer, hash)).cloned();

    // ---- the owner's claims
    let mut lines = vec![p.bold(&match owner {
        Some(o) => format!("Game updates of {o}"),
        None => "Game updates with money in escrow".to_string(),
    })];
    let mut table = Table::new(&[
        "Month",
        "Claim",
        "Bucket",
        "Prior → value",
        "Stake",
        "Status",
        "Next",
        "URL",
    ])
    .right(&[4]);
    let mut notes = Vec::new();
    let mut items = Vec::new();
    for r in &rows {
        let c = &r.claim.claim;
        let month = month_of[&c.month];
        let id = month.period.expiry_id();
        let source = source_of.get(&c.source);
        let challenged = r.challenge.is_some();
        let s = standing(&r.key, r.claim);
        let done = settled(r.claim, r.challenge.map(|(_, ch)| ch));
        let local = saved.get(&r.key.to_string()).filter(|l| {
            l.claimant == c.claimant.to_string() && l.oracle == oracle.program.to_string()
        });
        let revealed = c.archive_url_hash != [0; 32];
        // A committed claim's value is only in the private file until its reveal.
        let (prior, new, time, cited) = if revealed {
            (
                Some(c.prior_state),
                Some(c.new_state),
                Some(c.source_time),
                evidence_of(c.claimant, c.archive_url_hash),
            )
        } else {
            (
                local
                    .map(|l| l.prior_state)
                    .or(source.map(|s| s.state.current_state)),
                local.map(|l| l.new_state),
                local.map(|l| l.source_time),
                local.map(|l| l.archive_url.clone()),
            )
        };
        let revealable = s == Standing::Committed
            && local.is_some_and(|l| {
                source.is_some_and(|src| src.state.current_state == l.prior_state)
            });
        let reveal = revealable.then(|| reveal_command(&r.key));
        let expire = (s == Standing::Expired && !done).then(|| expire_command(&id, &r.key));
        let settle = settle_command(
            &id,
            &r.key,
            r.claim,
            r.challenge.as_ref().map(|(k, ch)| (k, *ch)),
        )
        .filter(|_| !done);
        let accept = accept_from(r.claim, seconds);
        let next = match s {
            _ if done => "-".to_string(),
            Standing::Committed if reveal.is_some() => {
                format!(
                    "reveal by {}",
                    at(claim_time(&now, r.claim, r.claim.reveal_deadline_slot))
                )
            }
            Standing::Committed => format!(
                "only its committer reveals it, by {}",
                at(claim_time(&now, r.claim, r.claim.reveal_deadline_slot))
            ),
            Standing::Expired if expire.is_some() => "refund it (below)".to_string(),
            Standing::Challengeable { due: false } => {
                format!("accepted from {}", accept.map_or("-".into(), at))
            }
            Standing::Challengeable { due: true } => "the keeper accepts it".to_string(),
            Standing::Challenged => "the oracle authority rules".to_string(),
            Standing::Council => "the council decides".to_string(),
            Standing::Stale(_) => "the keeper times it out".to_string(),
            _ if settle.is_some() => "settle it (below)".to_string(),
            _ => "-".to_string(),
        };
        let url = urls.get(&c.source);
        table.row(vec![
            id.clone(),
            short(&r.key.to_string()),
            source.map_or("-".into(), |s| bucket_cell(s)),
            format!(
                "{} → {}",
                prior.map_or("?".into(), value),
                new.map_or("sealed".into(), value)
            ),
            usdc(c.stake),
            s.words(done, challenged).to_string(),
            next,
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        for command in [&reveal, &expire, &settle].into_iter().flatten() {
            notes.push(format!("  {command}"));
        }
        let challenge = r.challenge.map(|(address, ch)| {
            let ruling = Ruling::of(ch.status, c.status);
            let ch_done = ch.escrow_disposition != OracleEscrowDisposition::Unsettled;
            json!({
                "challenge": address.to_string(),
                "challengeId": hex(&ch.challenge_id),
                "challenger": ch.challenger.to_string(),
                "value": value(ch.alternative_state),
                "valueAtoms": ch.alternative_state.to_string(),
                "sourceTimeUtc": iso(ch.alternative_source_time),
                "evidence": evidence_of(ch.challenger, ch.archive_url_hash),
                "bondAtoms": ch.bond.to_string(),
                "status": ruling.code(ch_done),
                "statusText": ruling.words(ch_done),
                "settleCommand": settle,
                "settled": ch_done,
            })
        });
        items.push(json!({
            "month": id,
            "monthAddress": month.address.to_string(),
            "claim": r.key.to_string(),
            "claimId": hex(&c.claim_id),
            "source": c.source.to_string(),
            "sourceId": hex(&c.source_id),
            "url": url,
            "bucket": source.map(|s| s.bucket_label()),
            "bucketName": source.map(|s| s.bucket_name()),
            "role": r.role,
            "claimant": c.claimant.to_string(),
            "prior": prior.map(value),
            "value": new.map(value),
            "valueAtoms": new.map(|v| v.to_string()),
            "sourceTimeUtc": time.map(iso),
            "stakeAtoms": c.stake.to_string(),
            "evidence": cited,
            "status": s.code(done),
            "statusText": s.words(done, challenged),
            "paysTo": s.pays(challenged),
            "committedUtc": iso(claim_time(&now, r.claim, r.claim.commit_slot)),
            "revealByUtc": iso(claim_time(&now, r.claim, r.claim.reveal_deadline_slot)),
            "revealedUtc": (r.claim.revealed_at_ts > 0).then(|| iso(r.claim.revealed_at_ts)),
            "acceptFromUtc": accept.map(iso),
            "revealCommand": reveal,
            "expireCommand": expire,
            "settleCommand": settle,
            "settled": done,
            "challenge": challenge,
        }));
    }
    if table.is_empty() {
        lines.push("  none".to_string());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.extend(notes.iter().map(|n| p.dim(n)));
    }

    // ---- open to challenge
    lines.push(String::new());
    lines.push(p.bold(&format!(
        "Open to challenge now (chain clock {})",
        at(now.unix)
    )));
    let mut table = Table::new(&[
        "Month",
        "Claim",
        "Bucket",
        "Prior → value",
        "Bond",
        "Accepted from",
        "URL",
    ])
    .right(&[4]);
    let mut open_items = Vec::new();
    for (key, c) in &open {
        let month = month_of[&c.claim.month];
        let id = month.period.expiry_id();
        let Some(source) = source_of.get(&c.claim.source) else {
            continue;
        };
        let bond = pools
            .get(&(c.claim.month, source.state.bucket_id))
            .and_then(|pool| challenge_bond(&Terms::of(pool), c.claim.stake).ok());
        let url = urls.get(&c.claim.source);
        let accept = accept_from(c, seconds);
        table.row(vec![
            id.clone(),
            short(&key.to_string()),
            bucket_cell(source),
            format!(
                "{} → {}",
                value(c.claim.prior_state),
                value(c.claim.new_state)
            ),
            bond.map_or("-".into(), usdc),
            accept.map_or("-".into(), at),
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        open_items.push(json!({
            "month": id,
            "monthAddress": month.address.to_string(),
            "claim": key.to_string(),
            "source": c.claim.source.to_string(),
            "sourceId": hex(&c.claim.source_id),
            "url": url,
            "bucket": source.bucket_label(),
            "bucketName": source.bucket_name(),
            "claimant": c.claim.claimant.to_string(),
            "prior": value(c.claim.prior_state),
            "value": value(c.claim.new_state),
            "valueAtoms": c.claim.new_state.to_string(),
            "sourceTimeUtc": iso(c.claim.source_time),
            "evidence": evidence_of(c.claim.claimant, c.claim.archive_url_hash),
            "stakeAtoms": c.claim.stake.to_string(),
            "acceptFromUtc": accept.map(iso),
            "bondAtoms": bond.map(|b| b.to_string()),
            "command": format!(
                "petri oracle updates challenge {id} --claim {key} --value <VALUE> --archive-url <URL>"
            ),
        }));
    }
    if table.is_empty() {
        lines.push("  none".to_string());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.push(p.dim(
            "  petri oracle updates challenge <MONTH> --claim <CLAIM> --value <VALUE> (--archive-url <URL> | --evidence <BUNDLE>)   (the bond is exact; open until the keeper accepts it)",
        ));
    }

    // ---- sources taking an update now
    lines.push(String::new());
    lines.push(p.bold("Sources taking an update now"));
    let mut table = Table::new(&[
        "Month", "Source", "Bucket", "Value", "As of", "Stake", "URL",
    ])
    .right(&[3, 5]);
    let mut source_items = Vec::new();
    for s in &takers {
        let month = month_of[&s.state.month];
        let id = month.period.expiry_id();
        let stake = pools
            .get(&(s.state.month, s.state.bucket_id))
            .map(|pool| pool.update_min_bond)
            .filter(|stake| *stake > 0);
        let url = urls.get(&s.address);
        let command = if chip.get(&month.address).copied().unwrap_or(false) {
            format!(
                "petri oracle observe {id} --source {} --file OBS.json --reveal",
                s.address
            )
        } else {
            format!(
                "petri oracle updates commit {id} --source {} --value <VALUE> --archive-url <URL> --reveal",
                s.address
            )
        };
        table.row(vec![
            id.clone(),
            short(&s.address.to_string()),
            bucket_cell(s),
            value(s.state.current_state),
            if s.state.latest_source_time > 0 {
                at(s.state.latest_source_time)
            } else {
                "-".to_string()
            },
            stake.map_or("-".into(), usdc),
            url.map_or("-".into(), |u| terminal_safe_text(u)),
        ]);
        source_items.push(json!({
            "month": id,
            "monthAddress": month.address.to_string(),
            "source": s.address.to_string(),
            "sourceId": hex(&s.state.source_id),
            "url": url,
            "bucket": s.bucket_label(),
            "bucketName": s.bucket_name(),
            "value": value(s.state.current_state),
            "valueAtoms": s.state.current_state.to_string(),
            "sourceTimeUtc": (s.state.latest_source_time > 0).then(|| iso(s.state.latest_source_time)),
            "stakeAtoms": stake.map(|v| v.to_string()),
            "command": command,
        }));
    }
    if game_months.is_empty() {
        lines.push("  No month takes updates now (Game, from listing to expiry).".to_string());
    } else if table.is_empty() {
        let ids: Vec<String> = game_months.iter().map(|m| m.period.expiry_id()).collect();
        lines.push(format!(
            "  {} take updates, but no active source with a value is left to update.",
            ids.join(", ")
        ));
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.push(p.dim(
            "  petri oracle updates commit <MONTH> --source <SOURCE> --value <VALUE> --archive-url <URL> --reveal   (a chip month: petri oracle observe <MONTH> --source <SOURCE> --file OBS.json --reveal)",
        ));
    }
    Ok(Report::new(
        json!({
            "ok": true,
            "owner": owner.map(|o| o.to_string()),
            "chainTimeUtc": iso(now.unix),
            "chainSlot": now.slot,
            "msPerSlot": now.slot_ms,
            "challengeSeconds": seconds,
            "updates": items,
            "open": open_items,
            "sources": source_items,
        }),
        lines.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ameba_oracle::state::OracleUpdateClaimData;

    const TERMS: Terms = Terms {
        bps: 2_000,
        minimum: 1_000_000,
        maximum: 10_000_000,
    };

    #[test]
    fn the_challenge_bond_is_a_share_of_the_claims_stake() {
        // 20% of a 1 USDC stake is under the minimum.
        assert_eq!(challenge_bond(&TERMS, 1_000_000).unwrap(), 1_000_000);
        assert_eq!(challenge_bond(&TERMS, 25_000_000).unwrap(), 5_000_000);
        assert_eq!(challenge_bond(&TERMS, 25_000_003).unwrap(), 5_000_000);
        assert_eq!(challenge_bond(&TERMS, 900_000_000).unwrap(), 10_000_000);
        assert_eq!(
            bond_words(&TERMS, 1_000_000),
            "20% of the claim's 1.00 USDC stake, at least 1.00 and at most 10.00 USDC"
        );
        // --stake and --bond: the exact amount, or a refusal naming it.
        assert_eq!(exact(None, 1_000_000, "bond", "--bond").unwrap(), 1_000_000);
        assert_eq!(
            exact(Some("1"), 1_000_000, "bond", "--bond").unwrap(),
            1_000_000
        );
        assert_eq!(
            exact(Some("2"), 1_000_000, "update stake", "--stake")
                .unwrap_err()
                .message,
            "the update stake is exactly 1 USDC: pass --stake 1, or leave --stake out"
        );
    }

    fn game_month(phase: OraclePhase) -> Game {
        Game {
            phase,
            listing: 1_000,
            expiry: 10_000,
            ready: true,
        }
    }

    #[test]
    fn updates_are_taken_from_listing_to_expiry_and_challenged_a_day_longer() {
        let game = game_month(OraclePhase::Game);
        let refused = |g: Game, now: u64| g.check_updates(now, "RAMX-202610").unwrap_err().message;
        assert!(refused(game, 999).contains("lists at"));
        assert!(game.takes_updates(1_000) && game.takes_updates(9_999));
        assert!(refused(game, 10_000).contains("closed at"));
        assert!(!game.takes_updates(10_000));
        // Challenges and rulings run until a day after expiry.
        assert!(game.takes_challenges(10_000));
        assert!(game.takes_challenges(10_000 + 86_399));
        assert!(!game.takes_challenges(10_000 + 86_400));
        assert!(refused(game_month(OraclePhase::Opening), 5_000).contains("is in Opening"));
        let unready = Game {
            ready: false,
            ..game
        };
        assert!(refused(unready, 5_000).contains("not settled yet"));
        assert!(resolved(&OracleMonthState {
            weight_scheme_version: 1,
            effective_weight_total_bps: 10_000,
            weight_manifest_hash: [1; 32],
            recipe_hash: [1; 32],
            frozen_source_count: 3,
            opening_resolved_source_count: 3,
            active_weight_scheme_version: OracleMonthState::ACTIVE_MEDIAN_SCHEME_VERSION,
            active_weight_group_count: 2,
            active_weight_manifest_hash: [1; 32],
            ..Default::default()
        }));
        assert!(!resolved(&OracleMonthState::default()));
    }

    fn source() -> OracleSourceState {
        OracleSourceState {
            is_initialized: true,
            status: OracleSourceStatus::Active,
            opening_submitted: true,
            current_state: 5_000_000,
            last_finalized_step: 7,
            ..Default::default()
        }
    }

    fn claim(status: OracleClaimStatus) -> OracleUpdateClaimV2 {
        OracleUpdateClaimV2 {
            claim: OracleUpdateClaimData {
                is_initialized: true,
                status,
                prior_state: 5_000_000,
                new_state: 5_500_000,
                stake: 1_000_000,
                ..Default::default()
            },
            commit_slot: 100,
            earliest_reveal_slot: 101,
            reveal_deadline_slot: 251,
            revealed_slot: if status == OracleClaimStatus::Committed {
                0
            } else {
                120
            },
            revealed_at_ts: if status == OracleClaimStatus::Committed {
                0
            } else {
                2_000
            },
            prior_finalized_step: 7,
            ..Default::default()
        }
    }

    const WHEN: When = When {
        slot: 200,
        now: 2_010,
        challenge_seconds: 86_400,
        grace_end: 1_000_000,
    };

    #[test]
    fn a_claim_stands_in_plain_words() {
        use OracleChallengeStatus as C;
        use OracleClaimStatus as S;
        let src = source();
        let of = |c: &OracleUpdateClaimV2, ch: Option<C>, when: When| {
            Standing::of(c, ch, Some(&src), when)
        };
        // Sealed, in its window; past it.
        assert_eq!(of(&claim(S::Committed), None, WHEN), Standing::Committed);
        let late = When { slot: 252, ..WHEN };
        assert_eq!(of(&claim(S::Committed), None, late), Standing::Expired);
        // Refunded by 167: never revealed.
        let mut refunded = claim(S::TimedOut);
        refunded.revealed_slot = 0;
        assert_eq!(of(&refunded, None, WHEN), Standing::Expired);
        // Revealed: open to challenge, due once its window passed.
        let revealed = claim(S::Revealed);
        assert_eq!(
            of(&revealed, None, WHEN),
            Standing::Challengeable { due: false }
        );
        assert_eq!(accept_from(&revealed, 86_400), Some(88_400));
        let due = When {
            now: 88_400,
            ..WHEN
        };
        assert_eq!(
            of(&revealed, None, due),
            Standing::Challengeable { due: true }
        );
        // A day after expiry, unchallenged: it times out.
        let over = When {
            now: 1_000_000,
            ..WHEN
        };
        assert_eq!(of(&revealed, None, over), Standing::Stale(Overtaken::Late));
        // Its source moved on: overtaken, challenged or not.
        let mut moved = source();
        moved.current_state = 6_000_000;
        assert_eq!(
            Standing::of(&revealed, None, Some(&moved), WHEN),
            Standing::Stale(Overtaken::Source)
        );
        assert_eq!(
            Standing::of(&revealed, Some(C::RuleReview), Some(&moved), WHEN),
            Standing::Stale(Overtaken::Source)
        );
        // Time alone never ends a challenged claim.
        assert_eq!(
            of(&revealed, Some(C::RuleReview), over),
            Standing::Challenged
        );
        assert_eq!(
            of(&revealed, Some(C::RuleReviewUnresolved), WHEN),
            Standing::Council
        );
        // How it ended.
        assert_eq!(
            of(&claim(S::Finalized), Some(C::Rejected), WHEN),
            Standing::Accepted
        );
        assert_eq!(
            of(&claim(S::Rejected), Some(C::Accepted), WHEN),
            Standing::Replaced
        );
        assert_eq!(
            of(&claim(S::Rejected), Some(C::Rejected), WHEN),
            Standing::Rejected
        );
        assert_eq!(of(&claim(S::TimedOut), None, WHEN), Standing::Cancelled);
        // Codes, words and who is paid.
        let codes = [
            (Standing::Committed, "committed", None),
            (Standing::Expired, "expired", Some("claimant")),
            (
                Standing::Challengeable { due: false },
                "challengeable",
                None,
            ),
            (
                Standing::Stale(Overtaken::Source),
                "stale",
                Some("claimant"),
            ),
            (Standing::Challenged, "challenged", None),
            (Standing::Council, "council", None),
            (Standing::Accepted, "accepted", Some("claimant")),
            (Standing::Replaced, "replaced", Some("challenger")),
            (Standing::Rejected, "rejected", Some("both")),
            (Standing::Cancelled, "cancelled", Some("claimant")),
        ];
        for (s, code, pays) in codes {
            assert_eq!((s.code(false), s.pays(false)), (code, pays), "{s:?}");
            assert_eq!(s.code(true), "settled");
            assert!(!s.words(false, false).is_empty());
        }
        assert_eq!(Standing::Cancelled.pays(true), Some("both"));
        assert_eq!(
            Standing::Expired.words(true, false),
            "Not revealed in time: its stake went back"
        );
        assert!(Standing::Challenged.words(false, true).contains("by hand"));
        assert!(
            Standing::Accepted
                .words(false, true)
                .contains("the stake and the bond")
        );
    }

    #[test]
    fn a_challenge_ends_upheld_dismissed_or_neither() {
        use OracleChallengeStatus as C;
        use OracleClaimStatus as S;
        assert_eq!(Ruling::of(C::RuleReview, S::Revealed), Ruling::Waiting);
        assert_eq!(
            Ruling::of(C::RuleReviewUnresolved, S::Revealed),
            Ruling::Council
        );
        assert_eq!(Ruling::of(C::Accepted, S::Rejected), Ruling::Upheld);
        assert_eq!(Ruling::of(C::Rejected, S::Finalized), Ruling::Dismissed);
        assert_eq!(Ruling::of(C::Rejected, S::Rejected), Ruling::Neither);
        assert_eq!(Ruling::of(C::Cancelled, S::TimedOut), Ruling::Cancelled);
        assert_eq!(Ruling::Upheld.code(false), "upheld");
        assert_eq!(Ruling::Upheld.code(true), "settled");
        assert!(
            Ruling::Dismissed
                .words(false)
                .contains("claimant takes the bond")
        );
    }

    #[test]
    fn money_settles_once_the_claim_has_ended() {
        use OracleChallengeStatus as C;
        use OracleClaimStatus as S;
        let key = Pubkey::new_unique();
        let challenge_key = Pubkey::new_unique();
        let settle = |c: &OracleUpdateClaimV2, ch: Option<&OracleUpdateChallenge>| {
            settle_command("RAMX-202610", &key, c, ch.map(|ch| (&challenge_key, ch)))
        };
        let accepted = claim(S::Finalized);
        let command = settle(&accepted, None).unwrap();
        assert!(command.starts_with("petri oracle stakes settle RAMX-202610 --kind update-claim"));
        assert!(command.ends_with(&format!("--claim {key}")));
        assert!(settle(&claim(S::Revealed), None).is_none());
        assert!(settle(&claim(S::TimedOut), None).is_some());
        let mut paid = accepted.clone();
        paid.claim.escrow_disposition = OracleEscrowDisposition::Refunded;
        assert!(settle(&paid, None).is_none());
        let challenge = |status| OracleUpdateChallenge {
            status,
            bond: 1_000_000,
            ..Default::default()
        };
        let upheld = challenge(C::Accepted);
        assert!(
            settle(&claim(S::Rejected), Some(&upheld))
                .unwrap()
                .contains(&format!(
                    "--kind update-challenge --source {} --claim {key} --challenge {challenge_key}",
                    Pubkey::default()
                ))
        );
        assert!(settle(&claim(S::Finalized), Some(&challenge(C::Rejected))).is_some());
        assert!(settle(&claim(S::TimedOut), Some(&challenge(C::Cancelled))).is_some());
        // Waiting for its ruling, or before the council: nothing yet.
        assert!(settle(&claim(S::Revealed), Some(&challenge(C::RuleReview))).is_none());
        let mut council = claim(S::Revealed);
        council.council_review_pending = true;
        assert!(settle(&council, Some(&challenge(C::RuleReviewUnresolved))).is_none());
        assert!(settled(&paid, None));
        assert!(!settled(&paid, Some(&upheld)));
    }

    fn source_ref() -> SourceRef {
        let page = "https://example.com/ram?id=1";
        SourceRef {
            address: Pubkey::new_unique(),
            state: OracleSourceState {
                canonical_locator_hash: hashv(&[b"locator", page.as_bytes()]).to_bytes(),
                ..source()
            },
        }
    }

    #[test]
    fn a_commit_meets_the_programs_rules_before_anything_is_sent() {
        // Listed at t=1,000; the evidence at 2026-10-25 01:00 UTC.
        let captured = 1_792_890_000;
        let game = Game {
            phase: OraclePhase::Game,
            listing: 1_000,
            expiry: 1_793_491_200,
            ready: true,
        };
        let url = "https://web.archive.org/web/20261025010000/https://example.com/ram?id=1";
        assert_eq!(archive_parts(url).unwrap().0, captured);
        let src = source_ref();
        let now = captured + 60;
        assert!(check_commit(&game, now, "RAMX-202610", &src, 5_500_000, url).is_ok());
        let refused = |source: &SourceRef, now: u64, value: u64, url: &str| {
            check_commit(&game, now, "RAMX-202610", source, value, url)
                .unwrap_err()
                .message
        };
        assert!(refused(&src, now, 5_000_000, url).contains("is the source's current value"));
        assert!(refused(&src, captured - 1, 5_500_000, url).contains("after the chain's clock"));
        let other = "https://web.archive.org/web/20261025010000/https://example.com/ram";
        assert!(refused(&src, now, 5_500_000, other).contains("is not this source's page"));
        let mut frozen = src.clone();
        frozen.state.status = OracleSourceStatus::Frozen;
        assert!(refused(&frozen, now, 5_500_000, url).contains("only an active source"));
        let mut last = src.clone();
        last.state.latest_source_time = game.expiry;
        assert!(refused(&last, now, 5_500_000, url).contains("its last print already"));
        let before = Game {
            listing: captured + 1,
            ..game
        };
        assert!(
            check_commit(&before, now, "RAMX-202610", &src, 5_500_000, url)
                .unwrap_err()
                .message
                .contains("before the month listed")
        );
    }

    #[test]
    fn a_reveal_needs_its_window_and_its_sources_value() {
        use OracleClaimStatus as S;
        let now = |slot| Now {
            slot,
            unix: 2_000,
            slot_ms: 400,
        };
        let src = source();
        let committed = claim(S::Committed);
        let expire = "petri oracle updates expire RAMX-202610 --claim C";
        assert!(check_reveal(&committed, 5_000_000, &src, &now(101), expire).is_ok());
        assert!(check_reveal(&committed, 5_000_000, &src, &now(251), expire).is_ok());
        let refused = |c: &OracleUpdateClaimV2, prior, slot| {
            check_reveal(c, prior, &src, &now(slot), expire)
                .unwrap_err()
                .message
        };
        assert!(refused(&committed, 5_000_000, 100).contains("opens at slot 101"));
        let closed = refused(&committed, 5_000_000, 252);
        assert!(
            closed.contains("closed at slot 251") && closed.contains(expire),
            "{closed}"
        );
        assert!(refused(&committed, 4_000_000, 150).contains("value changed since the commit"));
        assert!(refused(&claim(S::Revealed), 5_000_000, 150).contains("revealed already"));
        // Expire: only past the window, and only once.
        let key = Pubkey::new_unique();
        assert!(check_expire(&key, &committed, &now(252)).is_ok());
        assert!(
            check_expire(&key, &committed, &now(251))
                .unwrap_err()
                .message
                .contains("can still be revealed until slot 251")
        );
        assert!(check_expire(&key, &claim(S::Revealed), &now(300)).is_err());
    }

    #[test]
    fn a_challenge_needs_a_live_unchallenged_claim_of_someone_else() {
        use OracleClaimStatus as S;
        let game = Game {
            phase: OraclePhase::Game,
            listing: 1_000,
            expiry: 1_793_491_200,
            ready: true,
        };
        let url = "https://web.archive.org/web/20261025010000/https://example.com/ram?id=1";
        let now = 1_792_890_060;
        let src = source_ref();
        let (key, me) = (Pubkey::new_unique(), Pubkey::new_unique());
        let revealed = claim(S::Revealed);
        let check = |c: &OracleUpdateClaimV2, challenged: Option<&Pubkey>, who: &Pubkey, alt| {
            check_challenge(
                &game,
                now,
                "RAMX-202610",
                &key,
                c,
                challenged,
                who,
                &src,
                alt,
                url,
            )
        };
        assert!(check(&revealed, None, &me, 5_200_000).is_ok());
        let refused = |c, challenged, who, alt| check(c, challenged, who, alt).unwrap_err().message;
        let (committed, accepted) = (claim(S::Committed), claim(S::Finalized));
        assert!(refused(&committed, None, &me, 5_200_000).contains("not revealed yet"));
        assert!(refused(&accepted, None, &me, 5_200_000).contains("accepted already"));
        let other = Pubkey::new_unique();
        assert!(
            refused(&revealed, Some(&other), &me, 5_200_000).contains("one challenge per claim")
        );
        assert!(
            refused(&revealed, None, &revealed.claim.claimant, 5_200_000)
                .contains("its claimant can't")
        );
        assert!(refused(&revealed, None, &me, 5_500_000).contains("the claim's own value"));
        let mut moved = revealed.clone();
        moved.prior_finalized_step = 6;
        assert!(refused(&moved, None, &me, 5_200_000).contains("overtaken"));
    }

    #[test]
    fn scans_filter_on_the_account_markers() {
        // The markers follow the claim's and the challenge's fields as the
        // layouts encode them.
        let mut claim = OracleUpdateClaimV2::default();
        claim.claim.account_discriminator = OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR;
        let bytes = borsh::to_vec(&claim).unwrap();
        assert_eq!(
            bytes[CLAIM_MARKER_AT..CLAIM_MARKER_AT + 3],
            OracleUpdateClaimV2::ACCOUNT_DISCRIMINATOR
        );
        let mut challenge = OracleUpdateChallenge::default();
        challenge.stamp_current_account_layout();
        let bytes = borsh::to_vec(&challenge).unwrap();
        assert_eq!(
            bytes[CHALLENGE_MARKER_AT..CHALLENGE_MARKER_AT + 3],
            OracleUpdateChallenge::ACCOUNT_DISCRIMINATOR
        );
        // The month sits at offset 2 in both.
        let month = Pubkey::new_unique();
        claim.claim.month = month;
        assert_eq!(borsh::to_vec(&claim).unwrap()[2..34], month.to_bytes());
    }

    #[test]
    fn the_dev_chain_shortens_the_challenge_window() {
        use crate::futures::settings::{DEV_GENESIS, Profile};
        let dep = |tag: &str, profile, genesis: &str| Deployment {
            oracle_release_tag: Some(tag.to_string()),
            governance_controller: None,
            oracle: Pubkey::new_unique(),
            futures: None,
            usdc_mint: Pubkey::new_unique(),
            sponsor: None,
            genesis_hash: Some(genesis.to_string()),
            source: String::new(),
            profile,
        };
        assert_eq!(
            challenge_seconds(&dep("localnet-governance", Profile::Dev, DEV_GENESIS)),
            30
        );
        assert_eq!(
            challenge_seconds(&dep("localnet-dev", Profile::Dev, DEV_GENESIS)),
            30
        );
        assert_eq!(
            challenge_seconds(&dep("localnet-dev", Profile::Production, DEV_GENESIS)),
            86_400
        );
        assert_eq!(
            challenge_seconds(&dep("v1.3.0", Profile::Production, "x")),
            86_400
        );
    }
}
