//! Oracle accounts read from chain and checked before use: owner, exact
//! length, layout tag, canonical address and the account it belongs to.
//! Periods are discovered from the oracle program itself, so writes do not
//! depend on the futures API.

use ameba_oracle::{
    instruction as ix,
    interface::{Period, PeriodStatus, find_period_address},
    state::{
        OracleConfig, OracleMonthState, OracleSkuCoverageManifest, OracleSourceState,
        UserCollateral, derive_oracle_sku_coverage_manifest_pda, derive_user_collateral_pda,
        find_config_address,
    },
};
use borsh::BorshDeserialize;
use chrono::{DateTime, Datelike, Utc};
use solana_program::pubkey::Pubkey;

use super::skus::{self, hex};
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
    pub fn bucket_label(&self) -> String {
        skus::decode_label(&self.state.bucket_id).unwrap_or_else(|| hex(&self.state.bucket_id))
    }
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
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
        let periods = self.periods()?;
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
        let accounts = self.ctx.rpc.program_accounts(
            &self.program,
            &[
                Filter::DataSize(OracleSourceState::LEN as u64),
                Filter::Memcmp {
                    offset: 2,
                    bytes: month.address.to_bytes().to_vec(),
                },
            ],
        )?;
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
                && state.month == month.address
                && ix::find_oracle_source_address(&self.program, &month.address, &state.source_id)
                    == address
            {
                out.push(SourceRef { address, state });
            }
        }
        Ok(out)
    }

    /// The SKU index and proof for `bucket` in `month`. The month's coverage
    /// manifest commits to its required SKU root; Petri proves membership
    /// against the governed SKU set of the product, or (for a month formed
    /// outside the governed set, such as a dev bootstrap month) against the set
    /// of bucket ids its sources carry, whichever reproduces that root.
    pub fn sku_proof(&self, month: &MonthRef, bucket: &[u8; 32]) -> Result<(u16, Vec<[u8; 32]>)> {
        let coverage = self.coverage(month)?;
        let governed = skus::for_symbol(&month.period.symbol)?;
        let set = if coverage.required_sku_root == governed.root {
            governed
        } else {
            let ids = self
                .sources(month)?
                .into_iter()
                .map(|s| s.state.bucket_id)
                .collect();
            let observed = skus::SkuSet::from_ids(&month.period.symbol, ids);
            if coverage.required_sku_root != observed.root {
                bail(format!(
                    "{} commits to SKU root {} ({} SKUs), which neither the governed {} set ({}) nor the month's source buckets reproduce; Petri cannot prove membership",
                    month.period.expiry_id(),
                    hex(&coverage.required_sku_root),
                    coverage.required_sku_count,
                    governed.symbol,
                    hex(&governed.root)
                ))?;
            }
            observed
        };
        if usize::from(coverage.required_sku_count) != set.ids.len() {
            bail("the SKU coverage manifest's count does not match its SKU set")?;
        }
        set.proof(bucket)
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
    fn product_ids_are_padded_symbols() {
        let id = product_id("ramx").unwrap();
        assert_eq!(&id[..4], b"RAMX");
        assert!(id[4..].iter().all(|b| *b == 0));
        assert_eq!(product_symbol(&id), "RAMX");
    }
}
