//! Client for the Amoeba futures API (`api/API.md`): the deployment config,
//! the market reads and quotes (`/v1/markets`, `/v1/accounts`, `/v1/quote`),
//! sponsored prepare/submit and, in a dev build, the dev faucet. All atom amounts are strings;
//! `*Value` fields are index values.

use std::time::Duration;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::error::{Error, Result};

pub struct Api {
    base: String,
    http: reqwest::blocking::Client,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Candle {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prepared {
    pub id: String,
    pub transaction: String,
    pub last_valid_block_height: u64,
    pub summary: Value,
    /// Transactions that must land first, in order: each moves compressed
    /// USDC into a classic USDC account (API.md, "Spending compressed USDC").
    pub before: Vec<Prepared>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiConfig {
    pub oracle_release_tag: Option<String>,
    pub governance_controller: Option<super::governance::GovernanceDeployment>,
    pub cluster: String,
    /// The deployment's public RPC URL; `null` when its manifest names none.
    pub rpc_url: Option<String>,
    pub genesis_hash: String,
    pub programs: Programs,
    pub usdc_mint: String,
    pub usdc_decimals: u8,
    pub sponsor: String,
    /// The sponsorship terms and the months' lookup tables.
    pub sponsorship: Option<SponsorshipConfig>,
}

/// `/v1/config`'s `sponsorship`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SponsorshipConfig {
    /// The researcher's share a new sponsorship names (basis points): the
    /// deployment's oracle's, so it follows that oracle's upgrades.
    pub researcher_bps: Option<u64>,
    /// Each month's sponsorship lookup table (base58), by its series key
    /// (`RAMX-1798761600`, as the deployment manifest's `oracle.periods`).
    pub lookup_tables: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Programs {
    pub oracle: String,
    /// The futures program, when the deployment has one.
    pub futures: Option<String>,
}

pub fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| Error::msg(format!("unexpected API response: {e}")))
}

impl Api {
    pub fn new(base: &str) -> Self {
        // No request timeout (preflight constraint C35): a slow API is waited for.
        let http = reqwest::blocking::Client::builder()
            .timeout(None::<Duration>)
            .build()
            .expect("HTTP client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn finish(
        &self,
        what: &str,
        response: reqwest::Result<reqwest::blocking::Response>,
    ) -> Result<Value> {
        let response = response.map_err(|e| {
            Error::msg(format!(
                "futures API {} is unreachable ({what}): {e}",
                self.base
            ))
        })?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = response.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if status.is_success() {
            return Ok(value);
        }
        // Futures routes answer `{ error }`; oracle routes `{ ok: false, code, message }`.
        let message = value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                let text = value.get("message").and_then(Value::as_str)?;
                Some(match value.get("code").and_then(Value::as_str) {
                    Some(code) => format!("{text} [{code}]"),
                    None => text.to_string(),
                })
            })
            .unwrap_or_else(|| value.to_string());
        let logs = value
            .get("logs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut message = format!("{what}: {message} (HTTP {})", status.as_u16());
        if let Some(retry) = retry {
            message.push_str(&format!("; retry after {retry} s"));
        }
        Err(Error::msg(message).with_logs(logs))
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        self.finish(
            path.split('?').next().unwrap_or(path),
            self.http.get(url).send(),
        )
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        self.finish(path, self.http.post(url).json(body).send())
    }

    /// POSTs `bytes` as they are (`application/json`): evidence bundles,
    /// whose digest a re-serialisation could change.
    pub fn post_bytes(&self, path: &str, bytes: &[u8]) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        self.finish(
            path,
            self.http
                .post(url)
                .header("content-type", "application/json")
                .body(bytes.to_vec())
                .send(),
        )
    }

    /// POSTs with an `Authorization: Bearer` credential (the candidate store).
    pub fn post_bearer(&self, path: &str, body: &Value, credential: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        self.finish(
            path,
            self.http
                .post(url)
                .bearer_auth(credential)
                .json(body)
                .send(),
        )
    }

    pub fn config(&self) -> Result<ApiConfig> {
        parse(self.get("/v1/config")?)
    }

    pub fn health(&self) -> Result<Value> {
        self.get("/v1/health")
    }

    pub fn prepare(&self, owner: &str, action: &str, params: Value) -> Result<Prepared> {
        parse(self.post(
            "/v1/tx/prepare",
            &json!({"owner": owner, "action": action, "params": params}),
        )?)
    }

    pub fn submit(&self, id: &str, transaction: &str) -> Result<String> {
        let v = self.post(
            "/v1/tx/submit",
            &json!({"id": id, "transaction": transaction}),
        )?;
        v.get("signature")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::msg("submit returned no signature"))
    }

    /// DEV-ONLY: REMOVE FOR PROD. The dev faucet (`dev-faucet` builds only).
    #[cfg(feature = "dev-faucet")]
    pub fn faucet(&self, owner: &str) -> Result<Value> {
        self.post("/v1/faucet", &json!({"owner": owner}))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IndexReading {
    pub value: String,
    pub ts: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PeriodState {
    pub status: String,
    pub effective_listing_ts: Option<i64>,
    pub expiry_ts: Option<i64>,
    pub settlement_value: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Suggestion {
    pub lo: u16,
    pub hi: u16,
    pub low_value: String,
    pub high_value: String,
    pub center_value: String,
    pub sigma_value: String,
    pub vol: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarketView {
    pub market: String,
    pub symbol: String,
    pub label: String,
    pub expiry_ts: u64,
    pub status: String,
    pub tradable: bool,
    pub period: String,
    pub period_state: Option<PeriodState>,
    pub grid: u16,
    pub k: String,
    pub step_value: String,
    pub value_decimals: u8,
    pub lot_usd_per_point: String,
    pub reference_value: Option<String>,
    pub index: Option<IndexReading>,
    pub last_entry_value: Option<String>,
    pub last_trade_ts: Option<i64>,
    pub settlement_value: Option<String>,
    pub trades: u64,
    pub volume_lots: u64,
    pub volume24h_lots: u64,
    pub change24h_value: Option<String>,
    pub claims_open: u64,
    pub suggestion: Option<Suggestion>,
    pub chain_time: i64,
}

impl MarketView {
    /// The price a ticket starts from: the last trade's entry, else the
    /// index, else the middle of the suggested range (the site's rule).
    pub fn reference(&self) -> Option<String> {
        let positive = |v: &Option<String>| {
            v.clone()
                .filter(|t| t.parse::<f64>().is_ok_and(|x| x > 0.0))
        };
        positive(&self.last_entry_value)
            .or_else(|| positive(&self.index.as_ref().map(|i| i.value.clone())))
            .or_else(|| {
                let s = self.suggestion.as_ref()?;
                let mid =
                    (s.low_value.parse::<f64>().ok()? + s.high_value.parse::<f64>().ok()?) / 2.0;
                Some(format!("{mid:.4}"))
            })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Level {
    pub premium: String,
    pub entry_value: String,
    pub lots: u64,
    pub cumulative_lots: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Depth {
    pub lo: u16,
    pub hi: u16,
    pub lo_value: String,
    pub hi_value: String,
    pub asks: Vec<Level>,
    pub bids: Vec<Level>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Trade {
    pub signature: String,
    pub slot: u64,
    pub ts: i64,
    pub side: String,
    pub lo: u16,
    pub hi: u16,
    pub lo_value: String,
    pub hi_value: String,
    pub lots: u64,
    pub premium: String,
    pub entry_value: String,
    pub sources: String,
    pub taker: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TraderView {
    pub slot: u16,
    pub free: String,
    pub reserved: String,
    pub paid_out: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClaimRef {
    pub slot: u32,
    pub seq: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrderView {
    pub slot: u16,
    pub seq: String,
    pub side: String,
    pub lo: u16,
    pub hi: u16,
    pub lo_value: String,
    pub hi_value: String,
    pub lots: u64,
    pub placed_lots: u64,
    pub price: String,
    pub entry_value: String,
    pub escrow: String,
    pub close: bool,
    pub claim: Option<ClaimRef>,
    pub placed_ts: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExitView {
    pub lots: u64,
    pub premium: String,
    pub net: String,
    pub entry_value: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClaimView {
    pub slot: u32,
    pub seq: String,
    pub side: String,
    pub lo: u16,
    pub hi: u16,
    pub lo_value: String,
    pub hi_value: String,
    pub lots: u64,
    pub cost: String,
    pub reserved: u64,
    pub entry_value: String,
    pub opened_ts: i64,
    pub max_payout: String,
    pub payout_at_index: Option<String>,
    pub pnl_at_index: Option<String>,
    pub exit: Option<ExitView>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountMarket {
    pub market: String,
    pub symbol: String,
    pub label: String,
    pub expiry_ts: u64,
    pub status: String,
    pub tradable: bool,
    pub mark_value: Option<String>,
    pub trader: TraderView,
    pub orders: Vec<OrderView>,
    pub claims: Vec<ClaimView>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub owner: String,
    pub usdc: String,
    pub usdc_ata: String,
    pub usdc_ata_exists: bool,
    pub markets: Vec<AccountMarket>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Legs {
    pub orders: u64,
    pub ladder_steps: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Quote {
    pub market: String,
    pub side: String,
    pub lo: u16,
    pub hi: u16,
    pub lo_value: String,
    pub hi_value: String,
    pub lots: u64,
    pub limit: String,
    pub mode: String,
    pub filled: u64,
    pub premium: String,
    pub average_entry_value: Option<String>,
    pub pay: String,
    pub max_payout: String,
    pub legs: Legs,
    pub free: Option<String>,
    pub usdc: Option<String>,
    pub redeemed: Option<String>,
    pub net_pay: Option<String>,
    pub escrow: Option<String>,
    pub deposit: Option<String>,
    pub error: Option<String>,
}

fn path(market: &str) -> String {
    format!("/v1/markets/{market}")
}

pub fn markets(api: &Api) -> Result<Vec<MarketView>> {
    let v = api.get("/v1/markets")?;
    parse(v.get("markets").cloned().unwrap_or(Value::Array(vec![])))
}

pub fn market(api: &Api, market: &str) -> Result<MarketView> {
    parse(api.get(&path(market))?)
}

/// Executable depth of `[lo, hi)`; the suggested range when `shape` is none.
pub fn depth(api: &Api, market: &str, shape: Option<(u16, u16)>, max: u64) -> Result<Depth> {
    let query = match shape {
        Some((lo, hi)) => format!("?lo={lo}&hi={hi}&max={max}"),
        None => format!("?max={max}"),
    };
    parse(api.get(&format!("{}/depth{query}", path(market)))?)
}

pub fn trades(api: &Api, market: &str, limit: u32) -> Result<Vec<Trade>> {
    let v = api.get(&format!("{}/trades?limit={limit}", path(market)))?;
    parse(v.get("trades").cloned().unwrap_or(Value::Array(vec![])))
}

pub fn candles(api: &Api, market: &str, resolution: u32, from: Option<i64>) -> Result<Vec<Candle>> {
    let from = from.map(|f| format!("&from={f}")).unwrap_or_default();
    let v = api.get(&format!(
        "{}/candles?resolution={resolution}{from}",
        path(market)
    ))?;
    parse(v.get("candles").cloned().unwrap_or(Value::Array(vec![])))
}

/// One reference index sample (`value` in index points).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct IndexPoint {
    pub t: i64,
    pub value: f64,
}

/// The newest `limit` index samples of the last day, oldest first.
pub fn index(api: &Api, market: &str, limit: usize) -> Result<Vec<IndexPoint>> {
    let v = api.get(&format!("{}/index?limit={limit}", path(market)))?;
    parse(v.get("points").cloned().unwrap_or(Value::Array(vec![])))
}

pub fn account(api: &Api, owner: &str) -> Result<Account> {
    parse(api.get(&format!("/v1/accounts/{owner}"))?)
}

/// `GET /v1/wallets/:owner/compressed-usdc`: the owner's compressed USDC
/// (Light token ATA plus compressed token accounts), in atoms.
pub fn compressed_usdc(api: &Api, owner: &str) -> Result<u64> {
    let v = api.get(&format!("/v1/wallets/{owner}/compressed-usdc"))?;
    v.get("usdcAtoms")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::msg("unexpected API response: no usdcAtoms"))
}

/// `POST /v1/quote`.
pub fn quote(api: &Api, body: &Value) -> Result<Quote> {
    parse(api.post("/v1/quote", body)?)
}

/// The quote body for an order.
#[allow(clippy::too_many_arguments)]
pub fn quote_body(
    market: &str,
    owner: Option<&str>,
    side: &str,
    lo: u16,
    hi: u16,
    lots: u64,
    limit: Option<u64>,
    mode: &str,
    close: Option<(u32, &str)>,
) -> Value {
    let mut body =
        json!({ "market": market, "side": side, "lo": lo, "hi": hi, "lots": lots, "mode": mode });
    if let Some(owner) = owner {
        body["owner"] = json!(owner);
    }
    if let Some(limit) = limit {
        body["limit"] = json!(limit.to_string());
    }
    if let Some((slot, seq)) = close {
        body["close"] = json!({ "slot": slot, "seq": seq });
    }
    body
}

/// The owner's futures events, newest first (`/v1/accounts/:owner/history`).
pub fn history(api: &Api, owner: &str, limit: u32) -> Result<Value> {
    const TYPES: &str = "deposit,withdraw,take,fill,ladderFill,place,cancel,redeem,transfer,settle";
    api.get(&format!(
        "/v1/accounts/{owner}/history?limit={limit}&types={TYPES}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_a_deployment_without_a_public_rpc_url() {
        let c: ApiConfig = parse(json!({
            "cluster": "localnet", "rpcUrl": null, "wsUrl": null, "genesisHash": "g",
            "programs": {"oracle": "o", "futures": null},
            "usdcMint": "u", "usdcDecimals": 6, "sponsor": "s",
        }))
        .unwrap();
        assert_eq!(c.rpc_url, None);
        assert_eq!(c.programs.futures, None);
        // An older API that still names the retired clearing programs.
        let c: ApiConfig = parse(json!({
            "programs": {"clearing": "c", "matcher": "m", "oracle": "o", "futures": "f"},
        }))
        .unwrap();
        assert_eq!(c.programs.oracle, "o");
        assert_eq!(c.programs.futures.as_deref(), Some("f"));
        // The pre-rename `programs.range` is not read.
        let c: ApiConfig = parse(json!({"programs": {"oracle": "o", "range": "r"}})).unwrap();
        assert_eq!(c.programs.futures, None);
        let c: ApiConfig = parse(json!({"rpcUrl": "http://127.0.0.1:8899"})).unwrap();
        assert_eq!(c.rpc_url.as_deref(), Some("http://127.0.0.1:8899"));
    }

    #[test]
    fn views_parse_the_documented_shapes() {
        let m: MarketView = serde_json::from_value(json!({
            "market": "Fi1t", "symbol": "RAMX", "label": "RAMX Oct 26", "expiryTs": 1793491200u64,
            "status": "open", "tradable": true, "grid": 200, "k": "10000", "stepValue": "1",
            "valueDecimals": 6, "lotUsdPerPoint": "0.01", "index": { "value": "102.05", "ts": 1 },
            "lastEntryValue": null, "volume24hLots": 7, "change24hValue": "0.2239",
            "suggestion": { "lo": 62, "hi": 142, "lowValue": "62", "highValue": "142" }
        }))
        .unwrap();
        assert_eq!(m.volume24h_lots, 7);
        assert_eq!(m.change24h_value.as_deref(), Some("0.2239"));
        // No last trade: the index is the reference.
        assert_eq!(m.reference().as_deref(), Some("102.05"));
        let q: Quote = serde_json::from_value(json!({
            "averageEntryValue": "103.3102", "filled": 100, "hi": 120, "lo": 89, "lots": 100,
            "legs": { "ladderSteps": 31, "orders": 0 }, "limit": "310000", "pay": "14310200",
            "premium": "14310200", "netPay": "1", "error": "NoLiquidity"
        }))
        .unwrap();
        assert_eq!((q.filled, q.legs.ladder_steps), (100, 31));
        assert_eq!(q.net_pay.as_deref(), Some("1"));
        assert_eq!(q.error.as_deref(), Some("NoLiquidity"));
    }

    #[test]
    fn market_routes_carry_no_product_prefix() {
        assert_eq!(path("Fi1t"), "/v1/markets/Fi1t");
    }
}
