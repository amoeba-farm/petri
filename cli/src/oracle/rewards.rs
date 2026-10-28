//! Oracle cash rewards (tag 174). Once a month is Settled its operator
//! registers every entitlement and freezes them (tags 170, 171, 173); then
//! each recipient claims its own into its oracle collateral: a source's
//! proposer (keyed on the source), a supporter (on its support position),
//! the claimant of an accepted opening or of a finalized update (on the
//! claim). A sponsored source's proposer reward is `sponsorships claim`'s
//! (`sponsor.rs`); it is listed here with the wallet's share.

use std::collections::{HashMap, HashSet};

use ameba_oracle::{
    constants::MAX_ORACLE_USDC_MERGE_DEPTH,
    instruction as ix,
    sponsorship::{AgentReserve, SourceSponsorship, reserve_address, split_bounty},
    state::{
        OracleClaimStatus, OracleMonthState, OracleOpeningClaim, OracleOpeningClaimStatus,
        OraclePhase, OracleSourceState, OracleSourceStatus, OracleSupportPosition,
        OracleUpdateClaimV2, OracleUsdcRewardKind, OracleUsdcRewardReceipt,
        OracleUsdcRewardRegistration, OracleUsdcRewardSchedule, OracleUsdcRewardSchedulePhase,
        OracleUsdcSkuPool, OracleUsdcSourceReward, derive_oracle_usdc_reward_receipt_pda,
        derive_oracle_usdc_reward_registration_pda, derive_oracle_usdc_reward_schedule_pda,
        derive_oracle_usdc_sku_pool_pda, derive_oracle_usdc_source_reward_pda,
    },
};
use borsh::BorshDeserialize;
use serde_json::{Value, json};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{
    args::{RewardKind, Target},
    chain::{MonthRef, Oracle, PeriodRef, decode},
    send::{self, Write},
    skus::{self, hex},
    writes::month_json,
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    rpc::{Account, Filter},
    units::usdc,
    view::{Paint, Table},
};

/// Where the wallet sits in the accounts scanned for it: a support
/// position's supporter follows its flag, bump and month; an update claim's
/// claimant also its claim id, source and source id. Both start with the
/// month after the flag and bump.
const MONTH_OFFSET: usize = 2;
const SUPPORTER_OFFSET: usize = 2 + 32;
const CLAIMANT_OFFSET: usize = 2 + 4 * 32;

pub fn kind_of(kind: RewardKind) -> OracleUsdcRewardKind {
    match kind {
        RewardKind::Proposer => OracleUsdcRewardKind::SourceProposer,
        RewardKind::Supporter => OracleUsdcRewardKind::SourceSupport,
        RewardKind::Opening => OracleUsdcRewardKind::Opening,
        RewardKind::Update => OracleUsdcRewardKind::Update,
    }
}

pub fn kind_name(kind: OracleUsdcRewardKind) -> &'static str {
    match kind {
        OracleUsdcRewardKind::SourceProposer => "proposer",
        OracleUsdcRewardKind::SourceSupport => "supporter",
        OracleUsdcRewardKind::Opening => "opening",
        OracleUsdcRewardKind::Update => "update",
    }
}

/// Where a month's rewards stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// No reward schedule, or one with nothing in it.
    None,
    /// The month still runs.
    Running,
    /// Settled; its entitlements are not frozen yet.
    Settled,
    /// Frozen: each entitlement can be claimed.
    Frozen,
    /// The month failed and released its rewards.
    Aborted,
}

pub fn stage(month: &OracleMonthState, schedule: Option<&OracleUsdcRewardSchedule>) -> Stage {
    let Some(s) = schedule else {
        return Stage::None;
    };
    match s.phase {
        _ if s.total_reward_budget == 0
            && s.trading_fee_bounty_total == 0
            && s.bounty_fee_sweep_finalized =>
        {
            Stage::None
        }
        OracleUsdcRewardSchedulePhase::EntitlementsFinalized => Stage::Frozen,
        OracleUsdcRewardSchedulePhase::Aborted => Stage::Aborted,
        _ if matches!(month.phase, OraclePhase::Settled | OraclePhase::Closed) => Stage::Settled,
        _ => Stage::Running,
    }
}

/// One entitlement, in USDC atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Frozen, not claimed yet.
    Claimable(u64),
    /// Paid (its receipt's amount).
    Claimed(u64),
    /// Frozen at zero: nothing to claim.
    Zero,
    /// Not frozen yet.
    Waiting,
}

impl Status {
    fn json(self) -> (&'static str, Option<String>) {
        match self {
            Status::Claimable(a) => ("claimable", Some(a.to_string())),
            Status::Claimed(a) => ("claimed", Some(a.to_string())),
            Status::Zero => ("zero", Some("0".into())),
            Status::Waiting => ("waiting", None),
        }
    }

    fn amount(self) -> String {
        match self {
            Status::Claimable(a) | Status::Claimed(a) => format!("{} USDC", usdc(a)),
            Status::Zero => "0 USDC".into(),
            Status::Waiting => "-".into(),
        }
    }

    fn word(self, stage: Stage) -> &'static str {
        match (self, stage) {
            (Status::Claimable(_), _) => "to claim",
            (Status::Claimed(_), _) => "claimed",
            (Status::Zero, _) => "nothing",
            (Status::Waiting, Stage::Settled) => "not frozen yet",
            // Frozen: a sponsorship whose bond has not settled.
            (Status::Waiting, Stage::Frozen) => "settle first",
            (Status::Waiting, _) => "after settlement",
        }
    }
}

/// A reward of the wallet's in one month.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entitlement {
    pub kind: OracleUsdcRewardKind,
    /// What its receipt is keyed on: the source, support position or claim.
    pub subject: Pubkey,
    /// The source it is paid against.
    pub source: Pubkey,
    pub bucket_id: [u8; 32],
    /// The claim's accounts after the common thirteen, the SKU pool first
    /// (empty until frozen).
    pub trailing: Vec<AccountMeta>,
    pub status: Status,
}

/// A sponsorship the wallet funded, or whose researcher it is.
#[derive(Clone, Debug)]
pub struct Sponsorship {
    pub state: SourceSponsorship,
    pub funder: bool,
    pub researcher: bool,
}

/// A sponsored source's bounty: the wallet's share of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bounty {
    pub source: Pubkey,
    pub bucket_id: [u8; 32],
    pub status: Status,
}

/// What one month's claims depend on, read from chain.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub program: Pubkey,
    pub month: Pubkey,
    pub schedule: Option<OracleUsdcRewardSchedule>,
    /// The month's sources, by address.
    pub sources: HashMap<Pubkey, OracleSourceState>,
    /// Source reward records, by source.
    pub rewards: HashMap<Pubkey, OracleUsdcSourceReward>,
    /// SKU reward pools, by address.
    pub pools: HashMap<Pubkey, OracleUsdcSkuPool>,
    /// Sources carried from an earlier month: they pay no proposer reward.
    pub carried: HashSet<Pubkey>,
    /// The wallet's support positions, by address.
    pub supports: Vec<(Pubkey, OracleSupportPosition)>,
    /// The wallet's opening claims: source, claim address, claim.
    pub openings: Vec<(Pubkey, Pubkey, OracleOpeningClaim)>,
    /// The wallet's update claims, by address.
    pub updates: Vec<(Pubkey, OracleUpdateClaimV2)>,
    /// Update reward registrations, by claim.
    pub registrations: HashMap<Pubkey, OracleUsdcRewardRegistration>,
    /// Paid receipts, by address.
    pub receipts: HashMap<Pubkey, OracleUsdcRewardReceipt>,
    pub sponsored: Vec<Sponsorship>,
}

fn writable(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}

fn readonly(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}

/// A source's share of its pool, split as the program splits it: the
/// proposer's part and its supporters' (all of it to the proposer without
/// supporters).
fn allocation(pool: &OracleUsdcSkuPool, supporters: u32) -> Option<(u64, u64)> {
    if pool.registered_source_count == 0
        || pool.proposer_reward_bps == 0
        || pool.proposer_reward_bps > 10_000
    {
        return None;
    }
    let share = pool.source_reward_budget / u64::from(pool.registered_source_count);
    if supporters == 0 {
        return Some((share, 0));
    }
    let proposer = (u128::from(share) * u128::from(pool.proposer_reward_bps) / 10_000) as u64;
    Some((proposer, share - proposer))
}

impl Facts {
    pub fn schedule_key(&self) -> Pubkey {
        derive_oracle_usdc_reward_schedule_pda(&self.program, &self.month).0
    }

    fn frozen(&self) -> bool {
        self.schedule
            .as_ref()
            .is_some_and(|s| s.phase == OracleUsdcRewardSchedulePhase::EntitlementsFinalized)
    }

    pub fn pool_key(&self, bucket: &[u8; 32]) -> Pubkey {
        derive_oracle_usdc_sku_pool_pda(&self.program, &self.schedule_key(), bucket).0
    }

    pub fn reward_key(&self, source: &Pubkey) -> Pubkey {
        derive_oracle_usdc_source_reward_pda(&self.program, &self.schedule_key(), source).0
    }

    pub fn registration_key(&self, claim: &Pubkey) -> Pubkey {
        derive_oracle_usdc_reward_registration_pda(
            &self.program,
            &self.schedule_key(),
            OracleUsdcRewardKind::Update,
            claim,
        )
        .0
    }

    pub fn receipt_key(&self, kind: OracleUsdcRewardKind, subject: &Pubkey, to: &Pubkey) -> Pubkey {
        derive_oracle_usdc_reward_receipt_pda(
            &self.program,
            &self.schedule_key(),
            kind,
            subject,
            to,
        )
        .0
    }

    /// Paid when its receipt exists; else the frozen amount, or waiting.
    fn status(
        &self,
        kind: OracleUsdcRewardKind,
        subject: &Pubkey,
        to: &Pubkey,
        amount: Option<u64>,
    ) -> Status {
        if let Some(receipt) = self.receipts.get(&self.receipt_key(kind, subject, to)) {
            return Status::Claimed(receipt.amount);
        }
        match amount {
            None => Status::Waiting,
            Some(0) => Status::Zero,
            Some(a) => Status::Claimable(a),
        }
    }

    /// A source whose reward is registered as the program pays it
    /// (`validate_claim_source_family`): its state, reward, pool and pool
    /// address.
    fn registered(
        &self,
        source: &Pubkey,
    ) -> Option<(
        &OracleSourceState,
        &OracleUsdcSourceReward,
        Pubkey,
        &OracleUsdcSkuPool,
    )> {
        let state = self.sources.get(source)?;
        let reward = self.rewards.get(source)?;
        let pool_key = self.pool_key(&state.bucket_id);
        let pool = self.pools.get(&pool_key)?;
        (reward.registered
            && reward.month == self.month
            && reward.source == *source
            && reward.source_id == state.source_id
            && reward.proposer == state.proposer
            && reward.terminal_status == state.status
            && reward.merged_into_source == Pubkey::default()
            && reward.max_merge_depth <= MAX_ORACLE_USDC_MERGE_DEPTH
            && reward.sku_pool == pool_key
            && pool.bucket_id == state.bucket_id
            && matches!(
                state.status,
                OracleSourceStatus::Active | OracleSourceStatus::Inactive
            )
            && pool.registered_source_count > 0)
            .then_some((state, reward, pool_key, pool))
    }

    /// The proposer reward of a registered source discovered this month.
    fn proposer_amount(&self, source: &Pubkey) -> Option<u64> {
        if self.carried.contains(source) {
            return None;
        }
        let (_, reward, _, pool) = self.registered(source)?;
        Some(allocation(pool, reward.supporter_count)?.0)
    }

    /// Every reward `wallet` has in the month, by kind.
    pub fn entitlements(&self, wallet: &Pubkey) -> Vec<Entitlement> {
        let mut out = Vec::new();
        let mut proposed: Vec<(&Pubkey, &OracleSourceState)> = self
            .sources
            .iter()
            .filter(|(_, s)| s.proposer == *wallet)
            .collect();
        proposed.sort_by_key(|(key, _)| **key);
        for (key, state) in proposed {
            if let Some(e) = self.proposer(wallet, key, state) {
                out.push(e);
            }
        }
        out.extend(
            self.supports
                .iter()
                .filter_map(|(key, support)| self.supporter(wallet, key, support)),
        );
        out.extend(
            self.openings
                .iter()
                .filter_map(|(source, key, claim)| self.opening(wallet, source, key, claim)),
        );
        out.extend(
            self.updates
                .iter()
                .filter_map(|(key, claim)| self.update(wallet, key, claim)),
        );
        out
    }

    fn proposer(
        &self,
        wallet: &Pubkey,
        key: &Pubkey,
        state: &OracleSourceState,
    ) -> Option<Entitlement> {
        let kind = OracleUsdcRewardKind::SourceProposer;
        if self.carried.contains(key) {
            return None;
        }
        let (amount, trailing) = if self.frozen() {
            let amount = self.proposer_amount(key)?;
            let trailing = vec![
                writable(self.pool_key(&state.bucket_id)),
                readonly(*key),
                readonly(self.reward_key(key)),
                readonly(ix::find_carry_source_address(&self.program, key)),
            ];
            (Some(amount), trailing)
        } else if matches!(
            state.status,
            OracleSourceStatus::Rejected
                | OracleSourceStatus::Merged
                | OracleSourceStatus::TimedOut
        ) {
            return None;
        } else {
            (None, Vec::new())
        };
        Some(Entitlement {
            kind,
            subject: *key,
            source: *key,
            bucket_id: state.bucket_id,
            trailing,
            status: self.status(kind, key, wallet, amount),
        })
    }

    fn supporter(
        &self,
        wallet: &Pubkey,
        key: &Pubkey,
        support: &OracleSupportPosition,
    ) -> Option<Entitlement> {
        let kind = OracleUsdcRewardKind::SourceSupport;
        let origin = support.source;
        if support.supporter != *wallet || support.month != self.month || support.support_stake == 0
        {
            return None;
        }
        if !self.frozen() {
            let state = self.sources.get(&origin)?;
            if matches!(
                state.status,
                OracleSourceStatus::Rejected | OracleSourceStatus::TimedOut
            ) {
                return None;
            }
            return Some(Entitlement {
                kind,
                subject: *key,
                source: origin,
                bucket_id: state.bucket_id,
                trailing: Vec::new(),
                status: self.status(kind, key, wallet, None),
            });
        }
        // The merge lineage from the supported source to the one that pays.
        let mut path = vec![origin];
        let mut reward = self.rewards.get(&origin)?;
        while reward.merged_into_source != Pubkey::default() {
            let next = reward.merged_into_source;
            if path.len() > usize::from(MAX_ORACLE_USDC_MERGE_DEPTH) || path.contains(&next) {
                return None;
            }
            path.push(next);
            reward = self.rewards.get(&next)?;
        }
        let root = *path.last()?;
        let lineage = &path[..path.len() - 1];
        let (state, root_reward, pool_key, pool) = self.registered(&root)?;
        for (hop, source) in lineage.iter().enumerate() {
            let r = self.rewards.get(source)?;
            if r.registered
                || r.terminal_status != OracleSourceStatus::Candidate
                || r.merged_into_source != path[hop + 1]
                || r.max_merge_depth > MAX_ORACLE_USDC_MERGE_DEPTH
            {
                return None;
            }
        }
        let origin_id = match lineage.first() {
            Some(first) => self.rewards.get(first)?.source_id,
            None => state.source_id,
        };
        if usize::from(root_reward.max_merge_depth) < lineage.len()
            || root_reward.supporter_count == 0
            || support.source_id != origin_id
            || ix::find_oracle_support_address(&self.program, &self.month, &origin, wallet) != *key
        {
            return None;
        }
        let (_, budget) = allocation(pool, root_reward.supporter_count)?;
        let amount = budget / u64::from(root_reward.supporter_count);
        let mut trailing = vec![
            writable(pool_key),
            readonly(root),
            readonly(self.reward_key(&root)),
        ];
        for source in lineage {
            trailing.push(readonly(*source));
            trailing.push(readonly(self.reward_key(source)));
        }
        trailing.push(readonly(*key));
        Some(Entitlement {
            kind,
            subject: *key,
            source: root,
            bucket_id: state.bucket_id,
            trailing,
            status: self.status(kind, key, wallet, Some(amount)),
        })
    }

    fn opening(
        &self,
        wallet: &Pubkey,
        source: &Pubkey,
        key: &Pubkey,
        claim: &OracleOpeningClaim,
    ) -> Option<Entitlement> {
        let kind = OracleUsdcRewardKind::Opening;
        let state = self.sources.get(source)?;
        if claim.claimant != *wallet
            || ix::find_oracle_opening_claim_address(&self.program, &self.month, source) != *key
        {
            return None;
        }
        let (amount, trailing) = if self.frozen() {
            let (state, reward, pool_key, pool) = self.registered(source)?;
            if state.status != OracleSourceStatus::Active
                || reward.opening_claim != *key
                || claim.status != OracleOpeningClaimStatus::Accepted
                || pool.registered_opening_count == 0
            {
                return None;
            }
            let amount = pool.opening_reward_budget / u64::from(pool.registered_opening_count);
            let trailing = vec![
                writable(pool_key),
                readonly(*source),
                readonly(self.reward_key(source)),
                readonly(*key),
            ];
            (Some(amount), trailing)
        } else if !matches!(
            claim.status,
            OracleOpeningClaimStatus::Pending
                | OracleOpeningClaimStatus::Challenged
                | OracleOpeningClaimStatus::Accepted
        ) || !matches!(
            state.status,
            OracleSourceStatus::OpeningPending | OracleSourceStatus::Active
        ) {
            return None;
        } else {
            (None, Vec::new())
        };
        Some(Entitlement {
            kind,
            subject: *key,
            source: *source,
            bucket_id: state.bucket_id,
            trailing,
            status: self.status(kind, key, wallet, amount),
        })
    }

    fn update(
        &self,
        wallet: &Pubkey,
        key: &Pubkey,
        claim: &OracleUpdateClaimV2,
    ) -> Option<Entitlement> {
        let kind = OracleUsdcRewardKind::Update;
        let c = &claim.claim;
        if c.claimant != *wallet
            || c.month != self.month
            || ix::find_oracle_update_claim_address(
                &self.program,
                &self.month,
                &c.source,
                wallet,
                &c.claim_id,
            ) != *key
        {
            return None;
        }
        let bucket_id = self.sources.get(&c.source)?.bucket_id;
        let (amount, trailing) = if self.frozen() {
            let registration = self.registrations.get(key)?;
            let pool = self.pools.get(&registration.sku_pool)?;
            if registration.recipient != *wallet
                || registration.kind != kind
                || registration.subject != *key
                || registration.reward_units == 0
                || registration.sku_pool != self.pool_key(&bucket_id)
                || c.status != OracleClaimStatus::Finalized
                || pool.registered_update_count == 0
                || pool.registered_update_reward_units == 0
            {
                return None;
            }
            let amount = u128::from(pool.update_reward_budget)
                * u128::from(registration.reward_units)
                / u128::from(pool.registered_update_reward_units);
            let trailing = vec![
                writable(registration.sku_pool),
                readonly(self.registration_key(key)),
                readonly(*key),
            ];
            (Some(u64::try_from(amount).ok()?), trailing)
        } else if !matches!(
            c.status,
            OracleClaimStatus::Revealed | OracleClaimStatus::Finalized
        ) {
            return None;
        } else {
            (None, Vec::new())
        };
        Some(Entitlement {
            kind,
            subject: *key,
            source: c.source,
            bucket_id,
            trailing,
            status: self.status(kind, key, wallet, amount),
        })
    }

    /// The wallet's share of each sponsored bounty of the month.
    pub fn bounties(&self) -> Vec<Bounty> {
        let mut out = Vec::new();
        for s in &self.sponsored {
            let a = &s.state;
            let share = |paid: u64| {
                split_bounty(paid, a.researcher_bps, a.retention_bps).map(|(funder, cash, kept)| {
                    let funded = if s.funder { funder } else { 0 };
                    funded + if s.researcher { cash + kept } else { 0 }
                })
            };
            let status = match a.outcome {
                0 => Status::Waiting,
                1 if a.reward_settled => Status::Claimed(share(a.reward_paid).unwrap_or(0)),
                1 if self.frozen() => match self.proposer_amount(&a.source).and_then(share) {
                    Some(0) => Status::Zero,
                    Some(amount) => Status::Claimable(amount),
                    None => continue,
                },
                1 => Status::Waiting,
                _ => continue,
            };
            out.push(Bounty {
                source: a.source,
                bucket_id: a.bucket_id,
                status,
            });
        }
        out
    }
}

/// One month's rewards for the wallet.
pub struct MonthRewards {
    pub month: MonthRef,
    pub stage: Stage,
    pub entitlements: Vec<Entitlement>,
    pub bounties: Vec<Bounty>,
}

impl MonthRewards {
    fn claimable(&self) -> u64 {
        self.entitlements
            .iter()
            .filter_map(|e| match e.status {
                Status::Claimable(a) => Some(a),
                _ => None,
            })
            .sum()
    }

    fn is_empty(&self) -> bool {
        self.entitlements.is_empty() && self.bounties.is_empty()
    }
}

fn memcmp(offset: usize, key: &Pubkey) -> Filter {
    Filter::Memcmp {
        offset,
        bytes: key.to_bytes().to_vec(),
    }
}

/// The program's accounts of one layout that pass `filters` and decode.
fn scan<T: BorshDeserialize>(
    oracle: &Oracle,
    len: usize,
    mut filters: Vec<Filter>,
    what: &str,
) -> Result<Vec<(Pubkey, T)>> {
    filters.insert(0, Filter::DataSize(len as u64));
    Ok(oracle
        .ctx
        .rpc
        .program_accounts(&oracle.program, &filters)?
        .into_iter()
        .filter_map(|(key, account)| {
            decode(&oracle.program, &key, Some(&account), len, what)
                .ok()
                .map(|v| (key, v))
        })
        .collect())
}

/// The accounts of `keys` that exist, by address.
fn fetch(oracle: &Oracle, keys: &[Pubkey]) -> Result<HashMap<Pubkey, Account>> {
    let mut keys = keys.to_vec();
    keys.sort();
    keys.dedup();
    let accounts = oracle.ctx.rpc.accounts(&keys)?;
    Ok(keys
        .into_iter()
        .zip(accounts)
        .filter_map(|(key, account)| account.map(|a| (key, a)))
        .collect())
}

/// The ones of `found` at `keys` that decode as `T`.
fn decoded<T: BorshDeserialize>(
    oracle: &Oracle,
    found: &HashMap<Pubkey, Account>,
    keys: &[Pubkey],
    len: usize,
    what: &str,
) -> HashMap<Pubkey, T> {
    keys.iter()
        .filter_map(|key| {
            decode(&oracle.program, key, found.get(key), len, what)
                .ok()
                .map(|v| (*key, v))
        })
        .collect()
}

/// A carry record exists: the source came from an earlier month.
fn is_carried(account: Option<&Account>) -> bool {
    account.is_some_and(|a| a.owner != solana_program::system_program::id() || !a.data.is_empty())
}

/// The sponsorships `wallet` funded, or whose researcher it is (its
/// agent reserve's authority).
fn sponsorships(oracle: &Oracle, wallet: &Pubkey) -> Result<Vec<Sponsorship>> {
    let all = oracle.all_sponsorships()?;
    let reserves: Vec<Pubkey> = all.iter().map(|(_, a)| a.reserve).collect();
    let found = fetch(oracle, &reserves)?;
    let researched: HashSet<Pubkey> = decoded::<AgentReserve>(
        oracle,
        &found,
        &reserves,
        AgentReserve::LEN,
        "agent reserve",
    )
    .into_iter()
    .filter(|(key, r)| {
        r.discriminator == AgentReserve::MAGIC
            && r.authority == *wallet
            && reserve_address(&oracle.program, &r.agent_id, wallet).0 == *key
    })
    .map(|(key, _)| key)
    .collect();
    let mut out: Vec<Sponsorship> = all
        .into_iter()
        .filter_map(|(_, state)| {
            let funder = state.funder == *wallet;
            let researcher = researched.contains(&state.reserve);
            (funder || researcher).then_some(Sponsorship {
                state,
                funder,
                researcher,
            })
        })
        .collect();
    out.sort_by_key(|s| s.state.source);
    Ok(out)
}

/// One of [`read`]'s scans, on its own client.
type Scan<'a> = dyn Fn(&Oracle) -> Result<Scanned> + Sync + 'a;

/// What one of [`read`]'s scans found.
enum Scanned {
    Sources(Vec<super::chain::SourceRef>),
    Supports(Vec<(Pubkey, OracleSupportPosition)>),
    Updates(Vec<(Pubkey, OracleUpdateClaimV2)>),
    Sponsored(Vec<Sponsorship>),
}

/// The wallet's rewards in `periods`: the one month, or each month of
/// several where it has any.
pub fn read(oracle: &Oracle, wallet: &Pubkey, periods: &[PeriodRef]) -> Result<Vec<MonthRewards>> {
    let program = oracle.program;
    let month_of =
        |p: &PeriodRef| ix::find_oracle_month_address(&program, &p.address, p.period.expiry_ts);
    let single = match periods {
        [only] => Some(month_of(only)),
        _ => None,
    };
    let in_scope: HashSet<Pubkey> = periods.iter().map(month_of).collect();
    let scope: Vec<Filter> = single.iter().map(|m| memcmp(MONTH_OFFSET, m)).collect();
    // Each scan reads the whole program: they run side by side.
    let settings = &oracle.ctx.s;
    let on = |read: &Scan| {
        let ctx = Ctx::new(settings.clone());
        read(&Oracle { ctx: &ctx, program })
    };
    let scans: [&Scan; 4] = [
        &|o| Ok(Scanned::Sources(o.sources_in(single.as_ref())?)),
        &|o| {
            let found: Vec<(Pubkey, OracleSupportPosition)> = scan(
                o,
                OracleSupportPosition::LEN,
                [vec![memcmp(SUPPORTER_OFFSET, wallet)], scope.clone()].concat(),
                "support position",
            )?;
            Ok(Scanned::Supports(found))
        },
        &|o| {
            let found: Vec<(Pubkey, OracleUpdateClaimV2)> = scan(
                o,
                OracleUpdateClaimV2::LEN,
                [vec![memcmp(CLAIMANT_OFFSET, wallet)], scope.clone()].concat(),
                "update claim",
            )?;
            Ok(Scanned::Updates(found))
        },
        &|o| Ok(Scanned::Sponsored(sponsorships(o, wallet)?)),
    ];
    let results: Vec<Result<Scanned>> = std::thread::scope(|threads| {
        let running: Vec<_> = scans
            .iter()
            .map(|read| threads.spawn(move || on(read)))
            .collect();
        running
            .into_iter()
            .map(|t| {
                t.join()
                    .unwrap_or_else(|_| Err(Error::msg("an oracle read stopped")))
            })
            .collect()
    });
    let (mut sources, mut supports, mut updates, mut sponsored) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for result in results {
        match result? {
            Scanned::Sources(found) => sources = found,
            Scanned::Supports(found) => supports = found,
            Scanned::Updates(found) => updates = found,
            Scanned::Sponsored(found) => sponsored = found,
        }
    }
    supports.retain(|(key, s)| {
        s.is_initialized
            && s.supporter == *wallet
            && ix::find_oracle_support_address(&program, &s.month, &s.source, wallet) == *key
    });

    // Opening claims are keyed by month and source: read each that a print
    // may have made, with the schedules and carry records.
    let opened: Vec<(Pubkey, Pubkey, Pubkey)> = sources
        .iter()
        .filter(|s| {
            in_scope.contains(&s.state.month)
                && (s.state.opening_submitted
                    || matches!(
                        s.state.status,
                        OracleSourceStatus::OpeningPending | OracleSourceStatus::Active
                    ))
        })
        .map(|s| {
            let claim = ix::find_oracle_opening_claim_address(&program, &s.state.month, &s.address);
            (s.state.month, s.address, claim)
        })
        .collect();
    let schedules: Vec<Pubkey> = in_scope
        .iter()
        .map(|m| derive_oracle_usdc_reward_schedule_pda(&program, m).0)
        .collect();
    let carry_of = |source: &Pubkey| ix::find_carry_source_address(&program, source);
    let proposed: Vec<Pubkey> = sources
        .iter()
        .filter(|s| s.state.proposer == *wallet)
        .map(|s| s.address)
        .chain(sponsored.iter().map(|s| s.state.source))
        .collect();
    let carries: Vec<Pubkey> = proposed.iter().map(carry_of).collect();
    let claims: Vec<Pubkey> = opened.iter().map(|(_, _, c)| *c).collect();
    let found = fetch(oracle, &[claims.as_slice(), &schedules, &carries].concat())?;
    let openings: HashMap<Pubkey, OracleOpeningClaim> = decoded(
        oracle,
        &found,
        &claims,
        OracleOpeningClaim::LEN,
        "opening claim",
    );

    let mut months: Vec<&PeriodRef> = periods
        .iter()
        .filter(|p| {
            let m = month_of(p);
            single.is_some()
                || sources
                    .iter()
                    .any(|s| s.state.month == m && s.state.proposer == *wallet)
                || supports.iter().any(|(_, s)| s.month == m)
                || updates.iter().any(|(_, u)| u.claim.month == m)
                || sponsored.iter().any(|s| s.state.month == m)
                || opened.iter().any(|(month, _, claim)| {
                    *month == m && openings.get(claim).is_some_and(|c| c.claimant == *wallet)
                })
        })
        .collect();
    months.sort_by_key(|p| (p.period.expiry_ts, p.symbol.clone()));

    let mut out = Vec::new();
    for period in months {
        let month = oracle.month_of(period.clone())?;
        let mut facts = Facts {
            program,
            month: month.address,
            ..Facts::default()
        };
        let schedule_key = facts.schedule_key();
        facts.schedule = decoded::<OracleUsdcRewardSchedule>(
            oracle,
            &found,
            &[schedule_key],
            OracleUsdcRewardSchedule::LEN,
            "reward schedule",
        )
        .remove(&schedule_key)
        .filter(|s| s.is_initialized && s.month == month.address);
        let stage = stage(&month.state, facts.schedule.as_ref());
        if matches!(stage, Stage::None | Stage::Aborted) {
            if single.is_some() {
                out.push(MonthRewards {
                    month,
                    stage,
                    entitlements: Vec::new(),
                    bounties: Vec::new(),
                });
            }
            continue;
        }
        facts.sources = sources
            .iter()
            .filter(|s| s.state.month == month.address)
            .map(|s| (s.address, s.state))
            .collect();
        facts.carried = proposed
            .iter()
            .filter(|s| is_carried(found.get(&carry_of(s))))
            .copied()
            .collect();
        facts.supports = supports
            .iter()
            .filter(|(_, s)| s.month == month.address)
            .cloned()
            .collect();
        facts.updates = updates
            .iter()
            .filter(|(_, u)| u.claim.month == month.address)
            .cloned()
            .collect();
        facts.openings = opened
            .iter()
            .filter(|(m, _, _)| *m == month.address)
            .filter_map(|(_, source, claim)| {
                let c = openings.get(claim)?;
                (c.claimant == *wallet).then(|| (*source, *claim, c.clone()))
            })
            .collect();
        facts.sponsored = sponsored
            .iter()
            .filter(|s| s.state.month == month.address)
            .cloned()
            .collect();
        if stage == Stage::Frozen {
            read_frozen(oracle, wallet, &mut facts)?;
        }
        out.push(MonthRewards {
            month,
            stage,
            entitlements: facts.entitlements(wallet),
            bounties: facts.bounties(),
        });
    }
    Ok(out)
}

/// A frozen month's reward records, pools, registrations and receipts.
fn read_frozen(oracle: &Oracle, wallet: &Pubkey, facts: &mut Facts) -> Result<()> {
    let mut sources: Vec<Pubkey> = facts
        .sources
        .iter()
        .filter(|(_, s)| s.proposer == *wallet)
        .map(|(k, _)| *k)
        .collect();
    sources.extend(facts.supports.iter().map(|(_, s)| s.source));
    sources.extend(facts.openings.iter().map(|(s, _, _)| *s));
    sources.extend(facts.sponsored.iter().map(|s| s.state.source));
    let pools: Vec<Pubkey> = facts
        .sources
        .values()
        .map(|s| facts.pool_key(&s.bucket_id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let registrations: Vec<Pubkey> = facts
        .updates
        .iter()
        .map(|(claim, _)| facts.registration_key(claim))
        .collect();
    let found = fetch(oracle, &[pools.as_slice(), &registrations].concat())?;
    facts.pools = decoded(
        oracle,
        &found,
        &pools,
        OracleUsdcSkuPool::LEN,
        "SKU reward pool",
    );
    facts
        .pools
        .retain(|_, p| p.is_initialized && p.month == facts.month);
    let by_key: HashMap<Pubkey, OracleUsdcRewardRegistration> = decoded(
        oracle,
        &found,
        &registrations,
        OracleUsdcRewardRegistration::LEN,
        "reward registration",
    );
    facts.registrations = facts
        .updates
        .iter()
        .filter_map(|(claim, _)| {
            let r = by_key.get(&facts.registration_key(claim))?;
            (r.is_initialized && r.month == facts.month).then(|| (*claim, r.clone()))
        })
        .collect();
    // Reward records along each merge lineage, a hop at a time.
    let mut wanted = sources;
    for _ in 0..=MAX_ORACLE_USDC_MERGE_DEPTH {
        wanted.retain(|s| !facts.rewards.contains_key(s));
        wanted.sort();
        wanted.dedup();
        if wanted.is_empty() {
            break;
        }
        let keys: Vec<Pubkey> = wanted.iter().map(|s| facts.reward_key(s)).collect();
        let found = fetch(oracle, &keys)?;
        let mut next = Vec::new();
        for (source, key) in wanted.iter().zip(&keys) {
            let Some(reward) = decode::<OracleUsdcSourceReward>(
                &oracle.program,
                key,
                found.get(key),
                OracleUsdcSourceReward::LEN,
                "source reward",
            )
            .ok()
            .filter(|r| r.is_initialized && r.month == facts.month && r.source == *source) else {
                continue;
            };
            if reward.merged_into_source != Pubkey::default() {
                next.push(reward.merged_into_source);
            }
            facts.rewards.insert(*source, reward);
        }
        wanted = next;
    }
    let receipts: Vec<Pubkey> = facts
        .entitlements(wallet)
        .iter()
        .map(|e| facts.receipt_key(e.kind, &e.subject, wallet))
        .collect();
    let found = fetch(oracle, &receipts)?;
    facts.receipts = decoded(
        oracle,
        &found,
        &receipts,
        OracleUsdcRewardReceipt::LEN,
        "reward receipt",
    );
    let schedule = facts.schedule_key();
    facts
        .receipts
        .retain(|_, r| r.is_initialized && r.schedule == schedule && r.recipient == *wallet);
    Ok(())
}

fn product(bucket: &[u8; 32]) -> String {
    skus::chip_bucket(bucket)
        .map(|b| b.name.to_string())
        .or_else(|| skus::decode_label(bucket))
        .unwrap_or_else(|| hex(bucket)[..8].to_string())
}

/// `petri oracle rewards [MARKET]`.
pub fn show(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let wallet = ctx.owner(owner)?;
    let periods = oracle.periods_of(market, expiry)?;
    let months = read(&oracle, &wallet, &periods)?;
    let mut lines = vec![p.bold(&format!("Rewards of {wallet}"))];
    let mut table = Table::new(&["Month", "Kind", "Product", "Source", "Amount", "Status"]);
    let (mut claims, mut sponsored) = (Vec::new(), Vec::new());
    let mut items = Vec::new();
    for m in &months {
        let expiry_id = m.month.period.expiry_id();
        for e in &m.entitlements {
            table.row(vec![
                expiry_id.clone(),
                kind_name(e.kind).into(),
                product(&e.bucket_id),
                e.source.to_string(),
                e.status.amount(),
                e.status.word(m.stage).into(),
            ]);
        }
        for b in &m.bounties {
            table.row(vec![
                expiry_id.clone(),
                "sponsored".into(),
                product(&b.bucket_id),
                b.source.to_string(),
                b.status.amount(),
                b.status.word(m.stage).into(),
            ]);
            if matches!(b.status, Status::Claimable(_)) {
                sponsored.push(format!("petri oracle sponsorships claim {}", b.source));
            }
        }
        if m.claimable() > 0 {
            claims.push(format!("petri oracle rewards claim {expiry_id}"));
        }
        items.push(month_item(m));
    }
    match months.as_slice() {
        [only] if only.stage == Stage::None => lines.push(format!(
            "{} pays no rewards.",
            only.month.period.expiry_id()
        )),
        [only] if only.stage == Stage::Aborted => lines.push(format!(
            "{} failed: it pays no rewards.",
            only.month.period.expiry_id()
        )),
        [only] if only.is_empty() => {
            lines.push(format!("Nothing in {}.", only.month.period.expiry_id()))
        }
        _ if table.is_empty() => lines.push("No rewards yet.".into()),
        _ => {
            lines.extend(table.lines());
            lines.extend(claims.iter().chain(&sponsored).map(|c| p.dim(c)));
        }
    }
    Ok(Report::new(
        json!({"ok": true, "wallet": wallet.to_string(), "months": items}),
        lines.join("\n"),
    ))
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::None => "none",
        Stage::Running => "running",
        Stage::Settled => "settled",
        Stage::Frozen => "frozen",
        Stage::Aborted => "aborted",
    }
}

fn month_item(m: &MonthRewards) -> Value {
    let expiry_id = m.month.period.expiry_id();
    let claimed: u64 = m
        .entitlements
        .iter()
        .filter_map(|e| match e.status {
            Status::Claimed(a) => Some(a),
            _ => None,
        })
        .sum();
    let entitlements: Vec<Value> = m
        .entitlements
        .iter()
        .map(|e| {
            let (status, amount) = e.status.json();
            json!({
                "kind": kind_name(e.kind),
                "subject": e.subject.to_string(),
                "source": e.source.to_string(),
                "product": product(&e.bucket_id),
                "status": status,
                "amountAtoms": amount,
            })
        })
        .collect();
    let bounties: Vec<Value> = m
        .bounties
        .iter()
        .map(|b| {
            let (status, amount) = b.status.json();
            json!({
                "source": b.source.to_string(),
                "product": product(&b.bucket_id),
                "status": status,
                "shareAtoms": amount,
                "command": matches!(b.status, Status::Claimable(_))
                    .then(|| format!("petri oracle sponsorships claim {}", b.source)),
            })
        })
        .collect();
    let mut item = month_json(&m.month);
    item["rewards"] = json!(stage_name(m.stage));
    item["claimableAtoms"] = json!(m.claimable().to_string());
    item["claimedAtoms"] = json!(claimed.to_string());
    item["entitlements"] = json!(entitlements);
    item["sponsored"] = json!(bounties);
    item["command"] =
        json!((m.claimable() > 0).then(|| format!("petri oracle rewards claim {expiry_id}")));
    item
}

/// Why nothing can be claimed in a month, in a sentence.
fn refusal(m: &MonthRewards, kind: Option<OracleUsdcRewardKind>) -> String {
    let id = m.month.period.expiry_id();
    match m.stage {
        Stage::None => format!("{id} pays no rewards"),
        Stage::Aborted => format!("{id} failed: it pays no rewards"),
        Stage::Running => format!(
            "{id} is in {}: its rewards can be claimed once it settles and they are frozen",
            m.month.phase_name()
        ),
        Stage::Settled => format!("{id} settled; its rewards are not frozen yet"),
        Stage::Frozen => {
            let what = kind.map_or("nothing".to_string(), |k| {
                format!("no {} reward", kind_name(k))
            });
            let sponsored: Vec<String> = m
                .bounties
                .iter()
                .filter(|b| matches!(b.status, Status::Claimable(_)))
                .map(|b| format!("petri oracle sponsorships claim {}", b.source))
                .collect();
            if sponsored.is_empty() {
                format!("{what} to claim in {id}")
            } else {
                format!(
                    "{what} to claim in {id}; sponsored: {}",
                    sponsored.join(", ")
                )
            }
        }
    }
}

/// The claim instruction of one entitlement.
pub fn claim_instruction(
    program: &Pubkey,
    wallet: &Pubkey,
    month: &MonthRef,
    mint: &Pubkey,
    e: &Entitlement,
) -> Instruction {
    ix::claim_oracle_usdc_reward(
        program,
        wallet,
        &month.period.address,
        &month.address,
        mint,
        e.kind,
        &e.subject,
        e.trailing.clone(),
    )
}

/// `petri oracle rewards claim MONTH [--kind K]`: tag 174 for each frozen,
/// unclaimed reward of the wallet's in the month.
pub fn claim_write(ctx: &Ctx, target: &Target, kind: Option<RewardKind>) -> Result<Write> {
    let oracle = Oracle::new(ctx)?;
    let wallet = ctx.wallet_pubkey()?;
    let mint = ctx.deployment()?.usdc_mint;
    let periods = oracle.periods_of(Some(&target.market), target.expiry.as_deref())?;
    if periods.len() != 1 {
        let names: Vec<String> = periods.iter().map(PeriodRef::expiry_id).collect();
        bail(format!("name the month: {}", names.join(", ")))?;
    }
    let kind = kind.map(kind_of);
    let m = read(&oracle, &wallet, &periods)?
        .pop()
        .expect("one month was read");
    let claims: Vec<&Entitlement> = m
        .entitlements
        .iter()
        .filter(|e| matches!(e.status, Status::Claimable(_)) && kind.is_none_or(|k| e.kind == k))
        .collect();
    if claims.is_empty() {
        bail(refusal(&m, kind))?;
    }
    let mut steps = Vec::new();
    if oracle.collateral(&wallet)?.is_none() {
        steps.push((
            "initCollateral".to_string(),
            ix::init_user_collateral(&oracle.program, &wallet),
        ));
    }
    let mut total = 0u64;
    let mut review = Vec::new();
    let mut details = Vec::new();
    for e in &claims {
        let Status::Claimable(amount) = e.status else {
            continue;
        };
        total += amount;
        review.push(format!(
            "{} {} {} USDC",
            kind_name(e.kind),
            product(&e.bucket_id),
            usdc(amount)
        ));
        details.push(json!({
            "kind": kind_name(e.kind),
            "subject": e.subject.to_string(),
            "source": e.source.to_string(),
            "amountAtoms": amount.to_string(),
        }));
        steps.push((
            "claimReward".to_string(),
            claim_instruction(&oracle.program, &wallet, &m.month, &mint, e),
        ));
    }
    review.push("To your oracle collateral".to_string());
    Ok(Write {
        action: "oracle.rewards.claim",
        title: format!(
            "Claim {} USDC of {} rewards",
            usdc(total),
            m.month.period.expiry_id()
        ),
        review,
        details: json!({
            "month": month_json(&m.month),
            "claims": details,
            "totalAtoms": total.to_string(),
        }),
        txs: send::pack(&oracle.program, &wallet, steps),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::BorshSerialize;

    #[test]
    fn the_wallet_scans_read_the_wallet_where_the_layouts_keep_it() {
        let (month, wallet) = (Pubkey::new_unique(), Pubkey::new_unique());
        let support = OracleSupportPosition {
            is_initialized: true,
            month,
            supporter: wallet,
            source: Pubkey::new_unique(),
            ..Default::default()
        }
        .try_to_vec()
        .unwrap();
        assert_eq!(&support[MONTH_OFFSET..MONTH_OFFSET + 32], month.as_ref());
        assert_eq!(
            &support[SUPPORTER_OFFSET..SUPPORTER_OFFSET + 32],
            wallet.as_ref()
        );
        let mut claim = OracleUpdateClaimV2::default();
        claim.claim.is_initialized = true;
        claim.claim.month = month;
        claim.claim.claim_id = [7; 32];
        claim.claim.source = Pubkey::new_unique();
        claim.claim.source_id = [9; 32];
        claim.claim.claimant = wallet;
        let claim = claim.try_to_vec().unwrap();
        assert_eq!(&claim[MONTH_OFFSET..MONTH_OFFSET + 32], month.as_ref());
        assert_eq!(
            &claim[CLAIMANT_OFFSET..CLAIMANT_OFFSET + 32],
            wallet.as_ref()
        );
        let reserve = ameba_oracle::sponsorship::AgentReserve {
            discriminator: ameba_oracle::sponsorship::AgentReserve::MAGIC,
            agent_id: [3; 32],
            authority: wallet,
            ..Default::default()
        }
        .try_to_vec()
        .unwrap();
        assert_eq!(&reserve[41..73], wallet.as_ref());
    }

    #[test]
    fn a_months_rewards_stand_by_its_schedule_and_phase() {
        let mut month = OracleMonthState::default();
        let mut schedule = OracleUsdcRewardSchedule {
            total_reward_budget: 520_000_000,
            bounty_fee_sweep_finalized: true,
            phase: OracleUsdcRewardSchedulePhase::Funded,
            ..Default::default()
        };
        assert_eq!(stage(&month, None), Stage::None);
        month.phase = OraclePhase::Game;
        assert_eq!(stage(&month, Some(&schedule)), Stage::Running);
        month.phase = OraclePhase::Settled;
        assert_eq!(stage(&month, Some(&schedule)), Stage::Settled);
        schedule.phase = OracleUsdcRewardSchedulePhase::EntitlementsFinalized;
        assert_eq!(stage(&month, Some(&schedule)), Stage::Frozen);
        schedule.phase = OracleUsdcRewardSchedulePhase::Aborted;
        assert_eq!(stage(&month, Some(&schedule)), Stage::Aborted);
        // An empty schedule pays nothing, frozen or not.
        let empty = OracleUsdcRewardSchedule {
            bounty_fee_sweep_finalized: true,
            phase: OracleUsdcRewardSchedulePhase::EntitlementsFinalized,
            ..Default::default()
        };
        assert_eq!(stage(&month, Some(&empty)), Stage::None);
    }

    #[test]
    fn a_source_reward_splits_as_the_program_splits_it() {
        let pool = OracleUsdcSkuPool {
            source_reward_budget: 9_000_000,
            registered_source_count: 3,
            proposer_reward_bps: 6_000,
            ..Default::default()
        };
        assert_eq!(allocation(&pool, 0), Some((3_000_000, 0)));
        assert_eq!(allocation(&pool, 2), Some((1_800_000, 1_200_000)));
        let none = OracleUsdcSkuPool {
            registered_source_count: 0,
            ..pool.clone()
        };
        assert_eq!(allocation(&none, 2), None);
    }

    /// A frozen month: the wallet proposed S1 (two supporters),
    /// opened it and updated it (a fresh update, three units); it supported
    /// M, which merged into another's S2 (three supporters); S3 it proposed
    /// was carried from an earlier month; S4 it sponsored.
    struct Month {
        facts: Facts,
        wallet: Pubkey,
        agreement: Pubkey,
        s: [Pubkey; 4],
        merged: Pubkey,
        support: Pubkey,
        opening: Pubkey,
        update: Pubkey,
    }

    fn month(frozen: bool) -> Month {
        let program = Pubkey::new_unique();
        let month = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let agreement = Pubkey::new_unique();
        let bucket = *b"DDR5 16Gb\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let mut facts = Facts {
            program,
            month,
            schedule: Some(OracleUsdcRewardSchedule {
                is_initialized: true,
                month,
                total_reward_budget: 16_000_000,
                bounty_fee_sweep_finalized: true,
                phase: if frozen {
                    OracleUsdcRewardSchedulePhase::EntitlementsFinalized
                } else {
                    OracleUsdcRewardSchedulePhase::Funded
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let schedule = facts.schedule_key();
        let pool_key = facts.pool_key(&bucket);
        facts.pools.insert(
            pool_key,
            OracleUsdcSkuPool {
                is_initialized: true,
                schedule,
                month,
                bucket_id: bucket,
                source_reward_budget: 9_000_000,
                registered_source_count: 3,
                proposer_reward_bps: 6_000,
                opening_reward_budget: 3_000_000,
                registered_opening_count: 2,
                update_reward_budget: 4_000_000,
                registered_update_count: 2,
                registered_update_reward_units: 4,
                ..Default::default()
            },
        );
        let id = |n: u8| [n; 32];
        let source_key = |n: u8| ix::find_oracle_source_address(&program, &month, &id(n));
        let s = [1, 2, 3, 4].map(source_key);
        let merged = source_key(5);
        let proposers = [wallet, other, wallet, agreement];
        for (i, key) in s.iter().enumerate() {
            facts.sources.insert(
                *key,
                OracleSourceState {
                    is_initialized: true,
                    month,
                    source_id: id(i as u8 + 1),
                    bucket_id: bucket,
                    proposer: proposers[i],
                    status: OracleSourceStatus::Active,
                    opening_submitted: true,
                    ..Default::default()
                },
            );
        }
        let opening = ix::find_oracle_opening_claim_address(&program, &month, &s[0]);
        let reward =
            |key: &Pubkey, n: u8, proposer: Pubkey, supporters: u32| OracleUsdcSourceReward {
                is_initialized: true,
                month,
                schedule,
                sku_pool: pool_key,
                source: *key,
                source_id: id(n),
                proposer,
                supporter_count: supporters,
                registered: true,
                terminal_status: OracleSourceStatus::Active,
                ..Default::default()
            };
        facts.rewards.insert(
            s[0],
            OracleUsdcSourceReward {
                opening_claim: opening,
                ..reward(&s[0], 1, wallet, 2)
            },
        );
        facts.rewards.insert(
            s[1],
            OracleUsdcSourceReward {
                max_merge_depth: 1,
                ..reward(&s[1], 2, other, 3)
            },
        );
        facts.rewards.insert(s[2], reward(&s[2], 3, wallet, 0));
        facts.rewards.insert(s[3], reward(&s[3], 4, agreement, 0));
        facts.rewards.insert(
            merged,
            OracleUsdcSourceReward {
                registered: false,
                terminal_status: OracleSourceStatus::Candidate,
                merged_into_source: s[1],
                ..reward(&merged, 5, other, 1)
            },
        );
        facts.carried.insert(s[2]);
        let support = ix::find_oracle_support_address(&program, &month, &merged, &wallet);
        facts.supports.push((
            support,
            OracleSupportPosition {
                is_initialized: true,
                month,
                supporter: wallet,
                source: merged,
                source_id: id(5),
                support_stake: 1_000_000,
                ..Default::default()
            },
        ));
        facts.openings.push((
            s[0],
            opening,
            OracleOpeningClaim {
                is_initialized: true,
                claimant: wallet,
                status: OracleOpeningClaimStatus::Accepted,
                ..Default::default()
            },
        ));
        let mut claim = OracleUpdateClaimV2::default();
        claim.claim.is_initialized = true;
        claim.claim.month = month;
        claim.claim.claim_id = id(8);
        claim.claim.source = s[0];
        claim.claim.claimant = wallet;
        claim.claim.status = OracleClaimStatus::Finalized;
        let update = ix::find_oracle_update_claim_address(&program, &month, &s[0], &wallet, &id(8));
        facts.updates.push((update, claim));
        facts.registrations.insert(
            update,
            OracleUsdcRewardRegistration {
                is_initialized: true,
                month,
                schedule,
                sku_pool: pool_key,
                kind: OracleUsdcRewardKind::Update,
                subject: update,
                recipient: wallet,
                reward_units: 3,
                ..Default::default()
            },
        );
        facts.sponsored.push(Sponsorship {
            state: SourceSponsorship {
                source: s[3],
                researcher_bps: 2_000,
                retention_bps: 2_000,
                outcome: 1,
                month,
                bucket_id: bucket,
                ..Default::default()
            },
            funder: true,
            researcher: false,
        });
        Month {
            facts,
            wallet,
            agreement,
            s,
            merged,
            support,
            opening,
            update,
        }
    }

    #[test]
    fn a_frozen_month_pays_each_kind_with_the_programs_accounts() {
        let m = month(true);
        let f = &m.facts;
        let pool = f.pool_key(&f.sources[&m.s[0]].bucket_id);
        let all = f.entitlements(&m.wallet);
        let kinds: Vec<&str> = all.iter().map(|e| kind_name(e.kind)).collect();
        // S3 was carried in: no proposer reward.
        assert_eq!(kinds, ["proposer", "supporter", "opening", "update"]);
        let [proposer, supporter, opening, update] = all.as_slice() else {
            unreachable!()
        };
        // 9 USDC over three sources; 60% of S1's 3 to its proposer.
        assert_eq!(proposer.status, Status::Claimable(1_800_000));
        assert_eq!((proposer.subject, proposer.source), (m.s[0], m.s[0]));
        assert_eq!(
            proposer.trailing,
            vec![
                AccountMeta::new(pool, false),
                AccountMeta::new_readonly(m.s[0], false),
                AccountMeta::new_readonly(f.reward_key(&m.s[0]), false),
                AccountMeta::new_readonly(
                    ix::find_carry_source_address(&f.program, &m.s[0]),
                    false
                ),
            ]
        );
        // S2 pays its three supporters 1.2 USDC: the merged M's lineage first.
        assert_eq!(supporter.status, Status::Claimable(400_000));
        assert_eq!((supporter.subject, supporter.source), (m.support, m.s[1]));
        assert_eq!(
            supporter.trailing,
            vec![
                AccountMeta::new(pool, false),
                AccountMeta::new_readonly(m.s[1], false),
                AccountMeta::new_readonly(f.reward_key(&m.s[1]), false),
                AccountMeta::new_readonly(m.merged, false),
                AccountMeta::new_readonly(f.reward_key(&m.merged), false),
                AccountMeta::new_readonly(m.support, false),
            ]
        );
        assert_eq!(opening.status, Status::Claimable(1_500_000));
        assert_eq!(opening.subject, m.opening);
        assert_eq!(
            opening.trailing,
            vec![
                AccountMeta::new(pool, false),
                AccountMeta::new_readonly(m.s[0], false),
                AccountMeta::new_readonly(f.reward_key(&m.s[0]), false),
                AccountMeta::new_readonly(m.opening, false),
            ]
        );
        // Three of the pool's four update units.
        assert_eq!(update.status, Status::Claimable(3_000_000));
        assert_eq!(
            update.trailing,
            vec![
                AccountMeta::new(pool, false),
                AccountMeta::new_readonly(f.registration_key(&m.update), false),
                AccountMeta::new_readonly(m.update, false),
            ]
        );
        // The sponsored S4: its whole 3 USDC (no supporters), 80% the sponsor's.
        assert_eq!(
            f.bounties(),
            vec![Bounty {
                source: m.s[3],
                bucket_id: f.sources[&m.s[3]].bucket_id,
                status: Status::Claimable(2_400_000),
            }]
        );
        assert!(f.sources[&m.s[3]].proposer == m.agreement);
    }

    #[test]
    fn a_receipt_marks_a_reward_claimed() {
        let mut m = month(true);
        let key = m
            .facts
            .receipt_key(OracleUsdcRewardKind::Opening, &m.opening, &m.wallet);
        m.facts.receipts.insert(
            key,
            OracleUsdcRewardReceipt {
                amount: 1_500_000,
                ..Default::default()
            },
        );
        let all = m.facts.entitlements(&m.wallet);
        let opening = all
            .iter()
            .find(|e| e.kind == OracleUsdcRewardKind::Opening)
            .unwrap();
        assert_eq!(opening.status, Status::Claimed(1_500_000));
        // A paid sponsorship shows the wallet's share of what it paid.
        m.facts.sponsored[0].state.reward_settled = true;
        m.facts.sponsored[0].state.reward_paid = 3_000_000;
        assert_eq!(m.facts.bounties()[0].status, Status::Claimed(2_400_000));
    }

    #[test]
    fn what_the_program_would_refuse_is_not_listed() {
        let mut m = month(true);
        // An unregistered source pays nothing; nor does another's claim.
        m.facts.rewards.get_mut(&m.s[0]).unwrap().registered = false;
        m.facts.openings[0].2.claimant = Pubkey::new_unique();
        m.facts.registrations.clear();
        let kinds: Vec<&str> = m
            .facts
            .entitlements(&m.wallet)
            .iter()
            .map(|e| kind_name(e.kind))
            .collect();
        assert_eq!(kinds, ["supporter"]);
        // A broken lineage pays no supporter.
        m.facts.rewards.get_mut(&m.merged).unwrap().terminal_status = OracleSourceStatus::Active;
        assert!(m.facts.entitlements(&m.wallet).is_empty());
    }

    #[test]
    fn before_the_freeze_each_reward_waits_without_accounts() {
        let mut m = month(false);
        m.facts.sources.get_mut(&m.s[1]).unwrap().status = OracleSourceStatus::Rejected;
        // M's own source: still a candidate when the month runs.
        m.facts.sources.insert(
            m.merged,
            OracleSourceState {
                is_initialized: true,
                month: m.facts.month,
                status: OracleSourceStatus::Merged,
                ..Default::default()
            },
        );
        let all = m.facts.entitlements(&m.wallet);
        let kinds: Vec<&str> = all.iter().map(|e| kind_name(e.kind)).collect();
        assert_eq!(kinds, ["proposer", "supporter", "opening", "update"]);
        assert!(
            all.iter()
                .all(|e| e.status == Status::Waiting && e.trailing.is_empty())
        );
        assert_eq!(m.facts.bounties()[0].status, Status::Waiting);
        // A rejected source of the wallet's earns nothing.
        m.facts.sources.get_mut(&m.s[0]).unwrap().status = OracleSourceStatus::Rejected;
        assert!(
            !m.facts
                .entitlements(&m.wallet)
                .iter()
                .any(|e| e.kind == OracleUsdcRewardKind::SourceProposer)
        );
    }

    /// RAMX Dec 2026 at `address`, in `phase`.
    fn month_ref(address: Pubkey, phase: OraclePhase) -> MonthRef {
        MonthRef {
            period: PeriodRef {
                address: Pubkey::new_unique(),
                period: ameba_oracle::interface::Period::new_forming(
                    255,
                    [0; 32],
                    1_796_000_000,
                    1_798_761_600,
                    6,
                ),
                symbol: "RAMX".into(),
            },
            address,
            state: OracleMonthState {
                phase,
                ..Default::default()
            },
        }
    }

    #[test]
    fn nothing_to_claim_says_why_in_a_sentence() {
        let rewards = |stage, bounties| MonthRewards {
            month: month_ref(Pubkey::new_unique(), OraclePhase::Game),
            stage,
            entitlements: Vec::new(),
            bounties,
        };
        let says = |stage, kind| refusal(&rewards(stage, Vec::new()), kind);
        assert_eq!(says(Stage::None, None), "RAMX-202612 pays no rewards");
        assert_eq!(
            says(Stage::Running, None),
            "RAMX-202612 is in Game: its rewards can be claimed once it settles and they are frozen"
        );
        assert_eq!(
            says(Stage::Settled, None),
            "RAMX-202612 settled; its rewards are not frozen yet"
        );
        assert_eq!(says(Stage::Frozen, None), "nothing to claim in RAMX-202612");
        assert_eq!(
            says(Stage::Frozen, Some(OracleUsdcRewardKind::Update)),
            "no update reward to claim in RAMX-202612"
        );
        let source = Pubkey::new_unique();
        let bounty = Bounty {
            source,
            bucket_id: [0; 32],
            status: Status::Claimable(1),
        };
        assert_eq!(
            refusal(&rewards(Stage::Frozen, vec![bounty]), None),
            format!(
                "nothing to claim in RAMX-202612; sponsored: petri oracle sponsorships claim {source}"
            )
        );
        assert_eq!(Status::Waiting.word(Stage::Running), "after settlement");
        assert_eq!(Status::Waiting.word(Stage::Settled), "not frozen yet");
        assert_eq!(Status::Claimable(1).amount(), "0.000001 USDC");
    }

    #[test]
    fn a_claim_is_tag_174_with_the_receipt_of_its_kind_and_subject() {
        let m = month(true);
        let f = &m.facts;
        let mint = Pubkey::new_unique();
        let month_ref = month_ref(f.month, OraclePhase::Settled);
        for e in f.entitlements(&m.wallet) {
            let ix = claim_instruction(&f.program, &m.wallet, &month_ref, &mint, &e);
            assert_eq!(ix.program_id, f.program);
            assert_eq!(ix.data[0], 174);
            assert_eq!(ix.data[1], e.kind as u8);
            assert_eq!(ix.accounts[0], AccountMeta::new(m.wallet, true));
            assert_eq!(ix.accounts[3].pubkey, f.month);
            assert_eq!(ix.accounts[4].pubkey, f.schedule_key());
            assert_eq!(
                ix.accounts[10].pubkey,
                f.receipt_key(e.kind, &e.subject, &m.wallet)
            );
            assert_eq!(ix.accounts[13..], e.trailing[..]);
        }
    }
}
