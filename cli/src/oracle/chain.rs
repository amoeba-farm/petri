//! Oracle accounts read from chain and checked before use: owner, exact
//! length, layout tag, canonical address and the account it belongs to.
//! Periods are discovered from the oracle program itself, so writes do not
//! depend on the futures API.

use ameba_commodity_spec::Product;
use ameba_oracle::{
    instruction as ix,
    interface::{Period, PeriodStatus, find_period_address},
    sponsorship::{SourceSponsorship, SponsorExposure, agreement_address, exposure_address},
    state::{
        OracleConfig, OracleMonthState, OracleSkuCoverageManifest, OracleSourceState,
        OracleUsdcSkuPool, UserCollateral, derive_oracle_sku_coverage_manifest_pda,
        derive_oracle_usdc_reward_schedule_pda, derive_oracle_usdc_sku_pool_pda,
        derive_user_collateral_pda, find_config_address,
    },
};
use borsh::BorshDeserialize;
use chrono::{DateTime, Datelike, Utc};
use solana_program::{hash::hashv, pubkey::Pubkey};

use super::{
    skus::{self, SetKind, hex},
    writes::{OBJECT_HEADER, definition_text},
};
use crate::futures::{
    ctx::Ctx,
    error::{Error, Result, bail},
    rpc::{Account, Filter},
};

/// Decodes a fixed-size oracle account: owner, exact length, then the layout
/// with every byte after it zero.
pub fn decode<T: BorshDeserialize>(
    program: &Pubkey,
    key: &Pubkey,
    account: Option<&Account>,
    len: usize,
    what: &str,
) -> Result<T> {
    let account = account.ok_or_else(|| Error::msg(format!("{what} {key} does not exist")))?;
    if account.owner != *program || account.executable || account.data.len() != len {
        bail(format!(
            "{what} {key} is not an account of the oracle program {program}"
        ))?;
    }
    let mut data = account.data.as_slice();
    let value = T::deserialize(&mut data)
        .map_err(|_| Error::msg(format!("{what} {key} has an unknown layout")))?;
    if data.iter().any(|b| *b != 0) {
        bail(format!("{what} {key} has nonzero padding"))?;
    }
    Ok(value)
}

/// `RAMX` from a zero-padded product id.
pub fn product_symbol(id: &[u8; 32]) -> String {
    skus::decode_label(id).unwrap_or_else(|| hex(id))
}

/// A product id from its symbol: ASCII, upper case, zero-padded.
#[cfg(test)]
pub fn product_id(symbol: &str) -> Result<[u8; 32]> {
    skus::encode_label(&symbol.trim().to_ascii_uppercase())
}

/// The UTC month a period is named after: the month that ends at its expiry.
fn label_month(expiry_ts: u64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(expiry_ts as i64 - 1, 0).unwrap_or_default()
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// One oracle period, read from chain.
#[derive(Clone, Debug)]
pub struct PeriodRef {
    pub address: Pubkey,
    pub period: Period,
    pub symbol: String,
}

impl PeriodRef {
    /// `RAMX-202610`, the futures label month (as the API names it).
    pub fn expiry_id(&self) -> String {
        let at = label_month(self.period.expiry_ts);
        format!("{}-{}{:02}", self.symbol, at.year(), at.month())
    }

    /// `RAMX Oct 26`.
    pub fn label(&self) -> String {
        crate::futures::market::label(&self.symbol, self.period.expiry_ts)
    }

    fn expiry_keys(&self) -> Vec<String> {
        let at = label_month(self.period.expiry_ts);
        let mon = MONTHS[at.month0() as usize];
        vec![
            self.period.expiry_ts.to_string(),
            format!("{mon}{:02}", at.year().rem_euclid(100)),
            mon.to_string(),
            format!("{}{:02}", at.year(), at.month()),
            format!("{}{}{:02}", self.symbol, at.year(), at.month()),
        ]
    }

    pub fn status_name(&self) -> &'static str {
        match self.period.status {
            PeriodStatus::Forming => "forming",
            PeriodStatus::Live => "live",
            PeriodStatus::Settled => "settled",
            PeriodStatus::Failed => "failed",
        }
    }
}

/// An oracle month and the period it forms.
#[derive(Clone, Debug)]
pub struct MonthRef {
    pub period: PeriodRef,
    pub address: Pubkey,
    pub state: OracleMonthState,
}

impl MonthRef {
    pub fn phase_name(&self) -> String {
        format!("{:?}", self.state.phase)
    }
}

/// One source of a month.
#[derive(Clone, Debug)]
pub struct SourceRef {
    pub address: Pubkey,
    pub state: OracleSourceState,
}

impl SourceRef {
    /// Its SKU label (`DDR5_16GBIT`, `KSM64R52BD8-32HA`), else its hex id.
    pub fn bucket_label(&self) -> String {
        skus::decode_label(&self.state.bucket_id).unwrap_or_else(|| hex(&self.state.bucket_id))
    }

    /// What people read for its bucket: "DDR5 16Gb" for a chip bucket, else its label.
    pub fn bucket_name(&self) -> String {
        skus::bucket_name(&self.state.bucket_id)
    }
}

/// The role of a source's definition link (0 is its locator's).
const DEFINITION_ROLE: u8 = 1;
/// Evidence object kind 2: a source definition.
const DEFINITION_KIND: u8 = 2;
/// An evidence link's exact length.
const LINK_LEN: usize = 251;

/// The evidence object a source's definition link points to, when the link is
/// the canonical role-1 link of exactly this source and definition commitment.
fn definition_object(
    program: &Pubkey,
    source: &SourceRef,
    link: &Pubkey,
    account: Option<&Account>,
) -> Option<Pubkey> {
    let account = account?;
    let d = &account.data;
    let s = &source.state;
    let ok = account.owner == *program
        && !account.executable
        && d.len() == LINK_LEN
        && d[..5] == *b"OEL\x01\x01"
        && *link
            == ix::find_evidence_link_address(
                program,
                &source.address,
                DEFINITION_ROLE,
                &s.source_definition_hash,
            )
        && d[6..38] == s.month.to_bytes()
        && d[38..70] == source.address.to_bytes()
        && d[70..102] == source.address.to_bytes()
        && d[166] == DEFINITION_ROLE
        && d[187..219] == s.source_definition_hash
        && d[219..251] == s.source_id;
    ok.then(|| Pubkey::new_from_array(d[102..134].try_into().unwrap()))
}

/// The definition text of a sealed definition object committed to `hash`.
fn definition_in(
    program: &Pubkey,
    key: &Pubkey,
    account: &Account,
    hash: &[u8; 32],
) -> Option<String> {
    let d = &account.data;
    if account.owner != *program
        || account.executable
        || d.len() < OBJECT_HEADER
        || d[..5] != *b"OEB\x01\x01"
    {
        return None;
    }
    let payer = Pubkey::new_from_array(d[6..38].try_into().unwrap());
    let total = usize::from(u16::from_le_bytes([d[71], d[72]]));
    let written = usize::from(u16::from_le_bytes([d[73], d[74]]));
    let body = &d[OBJECT_HEADER..];
    let ok = *key == ix::find_evidence_object_address(program, &payer, DEFINITION_KIND, hash)
        && d[38] == DEFINITION_KIND
        && d[39..71] == *hash
        && d[75] == 1
        && total > 0
        && written == total
        && body.len() == total
        && hashv(&[body]).to_bytes() == *hash;
    if !ok {
        return None;
    }
    definition_text(body)
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// The period of `market` among `periods` (see [`Oracle::period`]).
fn pick(periods: Vec<PeriodRef>, market: &str, expiry: Option<&str>) -> Result<PeriodRef> {
    if let Ok(address) = market.trim().parse::<Pubkey>() {
        return periods
            .into_iter()
            .find(|p| p.address == address)
            .ok_or_else(|| Error::msg(format!("{address} is not a period of the oracle")));
    }
    let query = squash(market);
    let wanted = expiry.map(squash);
    let mut matches: Vec<PeriodRef> = periods
        .iter()
        .filter(|p| {
            let symbol = squash(&p.symbol);
            let keys = p.expiry_keys();
            let by_name = query == symbol
                || keys.iter().any(|k| {
                    query == format!("{symbol}{k}") || query == *k && k.starts_with(&symbol)
                });
            let by_expiry = wanted.as_ref().is_none_or(|e| keys.iter().any(|k| k == e));
            by_name && by_expiry
        })
        .cloned()
        .collect();
    if matches.is_empty() {
        let known: Vec<String> = periods.iter().map(PeriodRef::expiry_id).collect();
        bail(format!(
            "no oracle period matches `{market}`{}; periods: {}",
            expiry.map(|e| format!(" expiring {e}")).unwrap_or_default(),
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        ))?;
    }
    matches.sort_by_key(|p| {
        (
            match p.period.status {
                PeriodStatus::Live => 0,
                PeriodStatus::Forming => 1,
                _ => 2,
            },
            p.period.expiry_ts,
        )
    });
    Ok(matches.remove(0))
}

/// The periods among `periods` that `market` names (see [`Oracle::periods_of`]).
fn select(
    periods: Vec<PeriodRef>,
    market: Option<&str>,
    expiry: Option<&str>,
) -> Result<Vec<PeriodRef>> {
    let Some(market) = market else {
        return Ok(periods);
    };
    if expiry.is_none() {
        let symbol = squash(market);
        let all: Vec<PeriodRef> = periods
            .iter()
            .filter(|p| squash(&p.symbol) == symbol)
            .cloned()
            .collect();
        if !all.is_empty() {
            return Ok(all);
        }
    }
    Ok(vec![pick(periods, market, expiry)?])
}

/// The oracle program of the trusted deployment, read through the RPC.
pub struct Oracle<'a> {
    pub ctx: &'a Ctx,
    pub program: Pubkey,
}

impl<'a> Oracle<'a> {
    pub fn new(ctx: &'a Ctx) -> Result<Self> {
        let program = ctx.deployment()?.oracle;
        Ok(Self { ctx, program })
    }

    pub fn config_address(&self) -> Pubkey {
        find_config_address(&self.program).0
    }

    /// The program's config: current layout, canonical address.
    pub fn config(&self) -> Result<OracleConfig> {
        let key = self.config_address();
        let account = self.ctx.rpc.account(&key)?;
        let config: OracleConfig = decode(
            &self.program,
            &key,
            account.as_ref(),
            OracleConfig::LEN,
            "oracle config",
        )?;
        if !config.is_initialized
            || !config.has_current_layout()
            || config.bump != find_config_address(&self.program).1
        {
            bail(format!("oracle config {key} is not initialized"))?;
        }
        Ok(config)
    }

    /// Every period of the program, by expiry then product.
    pub fn periods(&self) -> Result<Vec<PeriodRef>> {
        let accounts = self.ctx.rpc.program_accounts(
            &self.program,
            &[
                Filter::DataSize(Period::LEN as u64),
                Filter::Memcmp {
                    offset: 0,
                    bytes: Period::DISCRIMINATOR.to_vec(),
                },
            ],
        )?;
        let mut out: Vec<PeriodRef> = accounts
            .into_iter()
            .filter_map(|(address, account)| {
                if account.owner != self.program {
                    return None;
                }
                let period = Period::decode(&account.data).ok()?;
                let (canonical, bump) =
                    find_period_address(&self.program, &period.product_id, period.expiry_ts);
                (canonical == address && bump == period.bump).then(|| PeriodRef {
                    address,
                    symbol: product_symbol(&period.product_id),
                    period,
                })
            })
            .collect();
        out.sort_by(|a, b| (a.period.expiry_ts, &a.symbol).cmp(&(b.period.expiry_ts, &b.symbol)));
        Ok(out)
    }

    /// The period of `market` (a product symbol, `RAMX-202610`, or a period
    /// address) and an optional expiry (`oct26`, `2026-10`, unix seconds). A
    /// bare symbol picks the live period with the nearest expiry.
    pub fn period(&self, market: &str, expiry: Option<&str>) -> Result<PeriodRef> {
        pick(self.periods()?, market, expiry)
    }

    /// The periods `market` names: every one without it, each month of a
    /// bare symbol (`ramx`), else the one it names (as [`Self::period`]).
    pub fn periods_of(&self, market: Option<&str>, expiry: Option<&str>) -> Result<Vec<PeriodRef>> {
        select(self.periods()?, market, expiry)
    }

    /// The month of a period: canonical address, current layout, bound to the period.
    pub fn month_of(&self, period: PeriodRef) -> Result<MonthRef> {
        let address =
            ix::find_oracle_month_address(&self.program, &period.address, period.period.expiry_ts);
        let account = self.ctx.rpc.account(&address)?;
        if account.is_none() {
            bail(format!(
                "{} has no oracle month yet (month {address})",
                period.expiry_id()
            ))?;
        }
        let state: OracleMonthState = decode(
            &self.program,
            &address,
            account.as_ref(),
            OracleMonthState::LEN,
            "oracle month",
        )?;
        if !state.is_initialized || !state.has_current_layout() || state.period != period.address {
            bail(format!(
                "oracle month {address} is not the initialized month of {}",
                period.expiry_id()
            ))?;
        }
        Ok(MonthRef {
            period,
            address,
            state,
        })
    }

    pub fn month(&self, market: &str, expiry: Option<&str>) -> Result<MonthRef> {
        let period = self.period(market, expiry)?;
        self.month_of(period)
    }

    /// A source of `month`, named by its 64-hex source id or its address.
    pub fn source(&self, month: &MonthRef, source: &str) -> Result<SourceRef> {
        let address = match source.trim().parse::<Pubkey>() {
            Ok(address) if source.trim().len() < 64 => address,
            _ => {
                let id = skus::parse_hex32(source, "source id")?;
                ix::find_oracle_source_address(&self.program, &month.address, &id)
            }
        };
        let account = self.ctx.rpc.account(&address)?;
        let state: OracleSourceState = decode(
            &self.program,
            &address,
            account.as_ref(),
            OracleSourceState::LEN,
            "oracle source",
        )?;
        if !state.is_initialized
            || state.month != month.address
            || ix::find_oracle_source_address(&self.program, &month.address, &state.source_id)
                != address
        {
            bail(format!(
                "{address} is not a source of {}",
                month.period.expiry_id()
            ))?;
        }
        Ok(SourceRef { address, state })
    }

    /// The SKU coverage manifest of a month.
    pub fn coverage(&self, month: &MonthRef) -> Result<OracleSkuCoverageManifest> {
        let key = derive_oracle_sku_coverage_manifest_pda(&self.program, &month.address).0;
        let account = self.ctx.rpc.account(&key)?;
        let coverage: OracleSkuCoverageManifest = decode(
            &self.program,
            &key,
            account.as_ref(),
            OracleSkuCoverageManifest::LEN,
            "SKU coverage manifest",
        )?;
        if !coverage.is_initialized
            || !coverage.has_canonical_layout()
            || coverage.month != month.address
        {
            bail(format!(
                "SKU coverage manifest {key} does not belong to this month"
            ))?;
        }
        Ok(coverage)
    }

    /// Every source of a month, read from chain.
    pub fn sources(&self, month: &MonthRef) -> Result<Vec<SourceRef>> {
        self.sources_in(Some(&month.address))
    }

    /// Every source of `month`, or of every month.
    pub fn sources_in(&self, month: Option<&Pubkey>) -> Result<Vec<SourceRef>> {
        let mut filters = vec![Filter::DataSize(OracleSourceState::LEN as u64)];
        if let Some(month) = month {
            filters.push(Filter::Memcmp {
                offset: 2,
                bytes: month.to_bytes().to_vec(),
            });
        }
        let accounts = self.ctx.rpc.program_accounts(&self.program, &filters)?;
        let mut out = Vec::new();
        for (address, account) in accounts {
            let state: OracleSourceState = match decode(
                &self.program,
                &address,
                Some(&account),
                OracleSourceState::LEN,
                "oracle source",
            ) {
                Ok(state) => state,
                Err(_) => continue,
            };
            if state.is_initialized
                && month.is_none_or(|m| state.month == *m)
                && ix::find_oracle_source_address(&self.program, &state.month, &state.source_id)
                    == address
            {
                out.push(SourceRef { address, state });
            }
        }
        Ok(out)
    }

    /// The SKU index and proof for `bucket` in `month`. The month's coverage
    /// manifest commits to its required SKU root; Petri proves membership
    /// against the governed set of the product with that root (its
    /// part-number list for older months, its chip buckets for months expiring
    /// after 2026-11-01), or (for a month formed outside the governed sets,
    /// such as a dev bootstrap month) against the set of bucket ids its sources
    /// carry, whichever reproduces that root.
    pub fn sku_proof(&self, month: &MonthRef, bucket: &[u8; 32]) -> Result<(u16, Vec<[u8; 32]>)> {
        let coverage = self.coverage(month)?;
        let symbol = &month.period.symbol;
        let set = match skus::for_root(symbol, &coverage.required_sku_root) {
            Some(governed) => governed,
            None => {
                // The month's source buckets, with or without this one (a
                // product's first source, or a one-SKU month).
                let ids: Vec<[u8; 32]> = self
                    .sources(month)?
                    .into_iter()
                    .map(|s| s.state.bucket_id)
                    .collect();
                let with_bucket = ids.iter().copied().chain([*bucket]).collect();
                [
                    skus::SkuSet::from_ids(symbol, with_bucket),
                    skus::SkuSet::from_ids(symbol, ids),
                ]
                .into_iter()
                .find(|set| set.root == coverage.required_sku_root)
                .ok_or_else(|| {
                    Error::msg(format!(
                        "{} commits to SKU root {} ({} SKUs), which neither a governed {symbol} set nor the month's source buckets reproduce; Petri cannot prove membership",
                        month.period.expiry_id(),
                        hex(&coverage.required_sku_root),
                        coverage.required_sku_count,
                    ))
                })?
            }
        };
        if usize::from(coverage.required_sku_count) != set.ids.len() {
            bail("the SKU coverage manifest's count does not match its SKU set")?;
        }
        set.proof(bucket)
    }

    /// The commodity product of a chip month: its coverage commits to the
    /// product's chip buckets, or (a month formed from its own source
    /// buckets, such as a dev bootstrap month) every source it has is in one
    /// of them. None for a month on part numbers or other categories, or a
    /// product without chip buckets.
    pub fn chip_product(&self, month: &MonthRef) -> Result<Option<Product>> {
        let Some(product) = Product::from_symbol(&month.period.symbol) else {
            return Ok(None);
        };
        let root = self.coverage(month)?.required_sku_root;
        Ok(
            match skus::for_root(&month.period.symbol, &root).and_then(|set| set.kind) {
                Some(SetKind::Chips) => Some(product),
                Some(SetKind::PartNumbers) => None,
                None => {
                    let sources = self.sources(month)?;
                    (!sources.is_empty()
                        && sources.iter().all(|s| {
                            skus::chip_bucket(&s.state.bucket_id)
                                .is_some_and(|b| b.product == product)
                        }))
                    .then_some(product)
                }
            },
        )
    }

    /// The definition text of the sources at `addresses`, in order (three
    /// batched reads): None for an address that is not a canonical source or
    /// whose definition is not readable.
    pub fn definitions_at(&self, addresses: &[Pubkey]) -> Result<Vec<Option<String>>> {
        let accounts = self.ctx.rpc.accounts(addresses)?;
        let sources: Vec<Option<SourceRef>> = addresses
            .iter()
            .zip(&accounts)
            .map(|(address, account)| {
                let state: OracleSourceState = decode(
                    &self.program,
                    address,
                    account.as_ref(),
                    OracleSourceState::LEN,
                    "oracle source",
                )
                .ok()?;
                (state.is_initialized
                    && ix::find_oracle_source_address(
                        &self.program,
                        &state.month,
                        &state.source_id,
                    ) == *address)
                    .then_some(SourceRef {
                        address: *address,
                        state,
                    })
            })
            .collect();
        let known: Vec<SourceRef> = sources.iter().flatten().cloned().collect();
        let mut texts = self.definitions(&known)?.into_iter();
        Ok(sources
            .iter()
            .map(|source| source.as_ref().and_then(|_| texts.next().flatten()))
            .collect())
    }

    /// Each source's definition text, in order: read from its definition link
    /// and the sealed evidence object the link points to (two batched reads
    /// for any number of sources), and held to the source's definition
    /// commitment. None where either is missing or does not match.
    pub fn definitions(&self, sources: &[SourceRef]) -> Result<Vec<Option<String>>> {
        let links: Vec<Pubkey> = sources
            .iter()
            .map(|s| {
                ix::find_evidence_link_address(
                    &self.program,
                    &s.address,
                    DEFINITION_ROLE,
                    &s.state.source_definition_hash,
                )
            })
            .collect();
        let link_accounts = self.ctx.rpc.accounts(&links)?;
        let objects: Vec<Option<Pubkey>> = sources
            .iter()
            .zip(&links)
            .zip(&link_accounts)
            .map(|((source, link), account)| {
                definition_object(&self.program, source, link, account.as_ref())
            })
            .collect();
        let keys: Vec<Pubkey> = objects.iter().flatten().copied().collect();
        let accounts = self.ctx.rpc.accounts(&keys)?;
        let found: std::collections::HashMap<Pubkey, Account> = keys
            .into_iter()
            .zip(accounts)
            .filter_map(|(key, account)| Some((key, account?)))
            .collect();
        Ok(sources
            .iter()
            .zip(objects)
            .map(|(source, object)| {
                let key = object?;
                definition_in(
                    &self.program,
                    &key,
                    found.get(&key)?,
                    &source.state.source_definition_hash,
                )
            })
            .collect())
    }

    /// A month by its address (a sponsorship records only that).
    pub fn month_at(&self, address: &Pubkey) -> Result<MonthRef> {
        let account = self.ctx.rpc.account(address)?;
        let state: OracleMonthState = decode(
            &self.program,
            address,
            account.as_ref(),
            OracleMonthState::LEN,
            "oracle month",
        )?;
        let month = self.month_of(self.period(&state.period.to_string(), None)?)?;
        if month.address != *address {
            bail(format!(
                "{address} is not the canonical month of its period"
            ))?;
        }
        Ok(month)
    }

    /// A product's reward pool for a month: its exact bonds and budgets.
    pub fn sku_pool(&self, month: &MonthRef, bucket: &[u8; 32]) -> Result<OracleUsdcSkuPool> {
        let schedule = derive_oracle_usdc_reward_schedule_pda(&self.program, &month.address).0;
        let key = derive_oracle_usdc_sku_pool_pda(&self.program, &schedule, bucket).0;
        let account = self.ctx.rpc.account(&key)?;
        if account.is_none() {
            bail(format!(
                "{} has no reward pool for this product, so it takes no sources",
                month.period.expiry_id()
            ))?;
        }
        let pool: OracleUsdcSkuPool = decode(
            &self.program,
            &key,
            account.as_ref(),
            OracleUsdcSkuPool::LEN,
            "SKU reward pool",
        )?;
        if !pool.is_initialized || pool.month != month.address || pool.bucket_id != *bucket {
            bail(format!(
                "SKU reward pool {key} is not this month's for this product"
            ))?;
        }
        Ok(pool)
    }

    /// The sponsorship of a source, if it was sponsored.
    pub fn sponsorship(&self, source: &Pubkey) -> Result<Option<(Pubkey, SourceSponsorship)>> {
        let key = agreement_address(&self.program, source).0;
        let account = self.ctx.rpc.account(&key)?;
        if account.is_none() {
            return Ok(None);
        }
        let agreement: SourceSponsorship = decode(
            &self.program,
            &key,
            account.as_ref(),
            SourceSponsorship::LEN,
            "sponsorship",
        )?;
        if agreement.discriminator != SourceSponsorship::MAGIC || agreement.source != *source {
            bail(format!(
                "sponsorship {key} is not the agreement of {source}"
            ))?;
        }
        Ok(Some((key, agreement)))
    }

    /// Every sponsorship `funder` paid for.
    pub fn sponsorships(&self, funder: &Pubkey) -> Result<Vec<(Pubkey, SourceSponsorship)>> {
        // The funder follows the marker, the bump, the source and the reserve.
        self.sponsorships_at(8 + 1 + 32 + 32, funder)
    }

    /// Every sponsorship that pays the agent reserve `reserve` (its
    /// researcher's share).
    pub fn sponsorships_of_reserve(
        &self,
        reserve: &Pubkey,
    ) -> Result<Vec<(Pubkey, SourceSponsorship)>> {
        // The reserve follows the marker, the bump and the source.
        self.sponsorships_at(8 + 1 + 32, reserve)
    }

    /// Every sponsorship of the program.
    pub fn all_sponsorships(&self) -> Result<Vec<(Pubkey, SourceSponsorship)>> {
        self.sponsorships_where(None)
    }

    /// The sponsorships with `key` at `offset`.
    fn sponsorships_at(
        &self,
        offset: usize,
        key: &Pubkey,
    ) -> Result<Vec<(Pubkey, SourceSponsorship)>> {
        self.sponsorships_where(Some(Filter::Memcmp {
            offset,
            bytes: key.to_bytes().to_vec(),
        }))
    }

    fn sponsorships_where(
        &self,
        filter: Option<Filter>,
    ) -> Result<Vec<(Pubkey, SourceSponsorship)>> {
        let mut filters = vec![
            Filter::DataSize(SourceSponsorship::LEN as u64),
            Filter::Memcmp {
                offset: 0,
                bytes: SourceSponsorship::MAGIC.to_vec(),
            },
        ];
        filters.extend(filter);
        let accounts = self.ctx.rpc.program_accounts(&self.program, &filters)?;
        let mut out = Vec::new();
        for (key, account) in accounts {
            let agreement: SourceSponsorship = decode(
                &self.program,
                &key,
                Some(&account),
                SourceSponsorship::LEN,
                "sponsorship",
            )?;
            if key != agreement_address(&self.program, &agreement.source).0 {
                continue;
            }
            out.push((key, agreement));
        }
        Ok(out)
    }

    pub fn collateral_address(&self, owner: &Pubkey) -> Pubkey {
        derive_user_collateral_pda(&self.program, owner).0
    }

    /// The owner's bond ledger, if it exists.
    pub fn collateral(&self, owner: &Pubkey) -> Result<Option<UserCollateral>> {
        let key = self.collateral_address(owner);
        let account = self.ctx.rpc.account(&key)?;
        if account.is_none() {
            return Ok(None);
        }
        let ledger: UserCollateral = decode(
            &self.program,
            &key,
            account.as_ref(),
            UserCollateral::LEN,
            "collateral ledger",
        )?;
        if !ledger.is_initialized || ledger.owner != *owner {
            bail(format!(
                "collateral ledger {key} does not belong to {owner}"
            ))?;
        }
        Ok(Some(ledger))
    }

    /// What `funder` has out in bonds to the agent reserve `reserve`: 0
    /// before their first sponsorship there.
    pub fn exposure(&self, reserve: &Pubkey, funder: &Pubkey) -> Result<u64> {
        let key = exposure_address(&self.program, reserve, funder).0;
        let account = self.ctx.rpc.account(&key)?;
        if account.is_none() {
            return Ok(0);
        }
        let exposure: SponsorExposure = decode(
            &self.program,
            &key,
            account.as_ref(),
            SponsorExposure::LEN,
            "sponsor exposure",
        )?;
        if exposure.discriminator != SponsorExposure::MAGIC
            || exposure.reserve != *reserve
            || exposure.funder != *funder
        {
            bail(format!(
                "sponsor exposure {key} is not {funder}'s to {reserve}"
            ))?;
        }
        Ok(exposure.outstanding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(symbol: &str, expiry_ts: u64) -> PeriodRef {
        let product = product_id(symbol).unwrap();
        PeriodRef {
            address: Pubkey::new_unique(),
            period: Period::new_forming(255, product, expiry_ts - 86_400 * 20, expiry_ts, 6),
            symbol: symbol.to_string(),
        }
    }

    #[test]
    fn periods_are_named_by_their_label_month() {
        let p = period("RAMX", 1_793_491_200);
        assert_eq!(p.expiry_id(), "RAMX-202610");
        assert_eq!(p.label(), "RAMX Oct 26");
        let keys = p.expiry_keys();
        for key in ["1793491200", "OCT26", "OCT", "202610", "RAMX202610"] {
            assert!(keys.iter().any(|k| k == key), "{key}");
        }
    }

    #[test]
    fn a_symbol_names_each_of_its_months_and_a_month_itself() {
        let all = vec![
            period("RAMX", 1_798_761_600),
            period("RAMX", 1_801_440_000),
            period("NANDX", 1_798_761_600),
        ];
        let names = |market: Option<&str>, expiry: Option<&str>| {
            select(all.clone(), market, expiry)
                .unwrap()
                .iter()
                .map(PeriodRef::expiry_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(None, None).len(), 3);
        assert_eq!(names(Some("ramx"), None), ["RAMX-202612", "RAMX-202701"]);
        assert_eq!(names(Some("RAMX-202612"), None), ["RAMX-202612"]);
        assert_eq!(names(Some("ramx"), Some("jan27")), ["RAMX-202701"]);
        assert_eq!(names(Some("nandx"), None), ["NANDX-202612"]);
        assert!(select(all, Some("nandx"), Some("jan27")).is_err());
    }

    /// A source, its definition link and its sealed definition object, as
    /// the program writes them.
    fn published(program: &Pubkey, text: &str) -> (SourceRef, Pubkey, Account, Pubkey, Account) {
        let month = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let mut body = b"ameba-oracle-source-definition-v1".to_vec();
        body.push(0);
        body.extend_from_slice(&(text.len() as u32).to_le_bytes());
        body.extend_from_slice(text.as_bytes());
        let hash = hashv(&[&body]).to_bytes();
        let state = OracleSourceState {
            is_initialized: true,
            month,
            source_id: [9; 32],
            source_definition_hash: hash,
            ..Default::default()
        };
        let source = SourceRef {
            address: ix::find_oracle_source_address(program, &month, &[9; 32]),
            state,
        };
        let object = ix::find_evidence_object_address(program, &payer, 2, &hash);
        let mut data = b"OEB\x01\x01".to_vec();
        data.push(0);
        data.extend_from_slice(payer.as_ref());
        data.push(2);
        data.extend_from_slice(&hash);
        data.extend_from_slice(&(body.len() as u16).to_le_bytes());
        data.extend_from_slice(&(body.len() as u16).to_le_bytes());
        data.push(1);
        data.extend_from_slice(&body);
        let object_account = Account {
            lamports: 1,
            owner: *program,
            data,
            executable: false,
        };
        let link = ix::find_evidence_link_address(program, &source.address, 1, &hash);
        let mut data = b"OEL\x01\x01".to_vec();
        data.push(0);
        for key in [
            month,
            source.address,
            source.address,
            object,
            source.address,
        ] {
            data.extend_from_slice(key.as_ref());
        }
        data.push(1);
        data.extend_from_slice(&[0; 20]);
        data.extend_from_slice(&hash);
        data.extend_from_slice(&[9; 32]);
        let link_account = Account {
            lamports: 1,
            owner: *program,
            data,
            executable: false,
        };
        (source, link, link_account, object, object_account)
    }

    #[test]
    fn a_sources_definition_reads_from_its_link_and_sealed_object() {
        let program = Pubkey::new_unique();
        let text = r#"{"schema":"amoeba.comparison-series.v1","bucket":"DDR5_16GBIT"}"#;
        let (source, link, link_account, object, object_account) = published(&program, text);
        assert_eq!(
            definition_object(&program, &source, &link, Some(&link_account)),
            Some(object)
        );
        let hash = source.state.source_definition_hash;
        assert_eq!(
            definition_in(&program, &object, &object_account, &hash).as_deref(),
            Some(text)
        );
        // Another source's link, another program's object, a draft, another
        // commitment: nothing.
        let mut other = source.clone();
        other.state.source_id = [8; 32];
        assert_eq!(
            definition_object(&program, &other, &link, Some(&link_account)),
            None
        );
        assert_eq!(definition_object(&program, &source, &link, None), None);
        let mut foreign = object_account.clone();
        foreign.owner = Pubkey::new_unique();
        assert_eq!(definition_in(&program, &object, &foreign, &hash), None);
        let mut draft = object_account.clone();
        draft.data[75] = 0;
        assert_eq!(definition_in(&program, &object, &draft, &hash), None);
        assert_eq!(
            definition_in(&program, &object, &object_account, &[1; 32]),
            None
        );
        let mut tampered = object_account;
        let last = tampered.data.len() - 2;
        tampered.data[last] ^= 1;
        assert_eq!(definition_in(&program, &object, &tampered, &hash), None);
    }

    #[test]
    fn product_ids_are_padded_symbols() {
        let id = product_id("ramx").unwrap();
        assert_eq!(&id[..4], b"RAMX");
        assert!(id[4..].iter().all(|b| *b == 0));
        assert_eq!(product_symbol(&id), "RAMX");
    }
}
