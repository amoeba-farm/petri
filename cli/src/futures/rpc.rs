//! A small Solana JSON-RPC client over blocking HTTP: account reads, the
//! Clock sysvar, program-account scans, simulation and sending.

use std::{
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use solana_program::{hash::Hash, pubkey::Pubkey, sysvar};
use solana_signature::Signature;
use solana_transaction::Transaction;

use super::error::{Error, Result};

/// Where petri's RPC methods are served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Served {
    /// By any full Solana RPC node: the production default (Solana's public
    /// mainnet-beta RPC, `settings::PRODUCTION_RPC_URL`), the dev chain's
    /// validator, or a provider's. The API's `/rpc` browser gateway admits it
    /// too.
    FullRpcAndGateway,
    /// By a full Solana RPC node only: the API's `/rpc` browser gateway does
    /// not admit it (it serves the web app's methods), so `--rpc` must not
    /// point at the gateway.
    FullRpc,
    /// Only by a local validator, and called only by a `dev-faucet` build.
    LocalValidator,
}

/// Every JSON-RPC method petri calls (a test holds this to the source), what
/// for, and where it is served. The command guide lists them ("RPC methods").
pub const METHODS: &[(&str, &str, Served)] = &[
    (
        "getAccountInfo",
        "accounts: markets, the oracle, gates, the Clock sysvar",
        Served::FullRpcAndGateway,
    ),
    (
        "getMultipleAccounts",
        "accounts, up to 100 at a time",
        Served::FullRpcAndGateway,
    ),
    (
        "getProgramAccounts",
        "the oracle's periods, sources, sponsorships and a wallet's reward claims",
        Served::FullRpc,
    ),
    ("getBalance", "a wallet's SOL", Served::FullRpcAndGateway),
    ("getSlot", "`petri status`", Served::FullRpcAndGateway),
    (
        "getGenesisHash",
        "the deployment identity gate",
        Served::FullRpcAndGateway,
    ),
    ("getVersion", "`petri status`", Served::FullRpc),
    (
        "getBlockHeight",
        "whether a sent transaction expired",
        Served::FullRpcAndGateway,
    ),
    (
        "getLatestBlockhash",
        "building transactions",
        Served::FullRpcAndGateway,
    ),
    (
        "simulateTransaction",
        "oracle writes with `--simulate`",
        Served::FullRpc,
    ),
    (
        "sendTransaction",
        "sending signed transactions",
        Served::FullRpcAndGateway,
    ),
    (
        "getSignatureStatuses",
        "confirming them",
        Served::FullRpcAndGateway,
    ),
    (
        "getTransaction",
        "a landed transaction's logs",
        Served::FullRpcAndGateway,
    ),
    (
        "requestAirdrop",
        "`petri faucet --sol` (dev builds)",
        Served::LocalValidator,
    ),
];

#[derive(Clone, Debug)]
pub struct Account {
    pub lamports: u64,
    pub owner: Pubkey,
    pub data: Vec<u8>,
    pub executable: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub unix_timestamp: i64,
}

/// A JSON-RPC level failure, with the server's `data` (logs, err) kept.
#[derive(Clone, Debug)]
pub struct RpcFailure {
    pub message: String,
    pub err: Option<Value>,
    pub logs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Simulation {
    pub err: Option<Value>,
    pub logs: Vec<String>,
    pub units: Option<u64>,
}

/// A landed transaction's metadata.
#[derive(Clone, Debug, Default)]
pub struct Landed {
    pub slot: u64,
    pub err: Option<Value>,
    pub logs: Vec<String>,
}

/// `getProgramAccounts` filters.
#[derive(Clone, Debug)]
pub enum Filter {
    DataSize(u64),
    Memcmp { offset: usize, bytes: Vec<u8> },
}

/// What an RPC refusing `method` most likely means.
fn refusal_hint(method: &str) -> &'static str {
    match METHODS.iter().find(|(m, _, _)| *m == method) {
        Some((_, _, Served::FullRpc)) => {
            "; petri needs a full Solana RPC node for it (--rpc), not the API's /rpc gateway"
        }
        Some((_, _, Served::LocalValidator)) => "; only a local validator serves it",
        _ => "",
    }
}

/// Whether an RPC turned a call away for its request rate: HTTP 429, or a
/// JSON-RPC error saying so (Solana's public RPC limits getProgramAccounts
/// per caller: "Too many requests for a specific RPC call").
fn rate_limited(status: u16, error: Option<&Value>) -> bool {
    status == 429
        || error.is_some_and(|e| {
            e.get("code").and_then(Value::as_i64) == Some(429)
                || e.get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.to_ascii_lowercase().contains("too many requests"))
        })
}

/// A rate-limited call, in plain words. Petri doesn't retry (preflight
/// decision: no retries); another RPC is the way past it.
fn rate_limit_message(url: &str, method: &str) -> String {
    format!(
        "The RPC {url} is limiting requests and turned away {method}: use an RPC provider's \
         node with --rpc <provider URL> or PETRI_RPC_URL"
    )
}

pub struct Rpc {
    url: String,
    http: reqwest::blocking::Client,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

impl Rpc {
    pub fn new(url: &str) -> Self {
        // No request timeout (preflight constraint C34): a slow node is waited for.
        let http = reqwest::blocking::Client::builder()
            .timeout(None::<Duration>)
            .build()
            .expect("HTTP client");
        Self {
            url: url.to_string(),
            http,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn call_raw(
        &self,
        method: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, RpcFailure>> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let response = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .map_err(|e| Error::msg(format!("RPC {} is unreachable: {e}", self.url)))?;
        let status = response.status();
        if rate_limited(status.as_u16(), None) {
            return Err(Error::msg(rate_limit_message(&self.url, method)));
        }
        let value: Value = response.json().map_err(|e| {
            Error::msg(format!(
                "RPC {method} returned HTTP {status}: {e}{}",
                refusal_hint(method)
            ))
        })?;
        if !status.is_success() && value.get("error").is_none() {
            // Not a JSON-RPC answer: a gateway's refusal, for one.
            let detail = value
                .get("message")
                .and_then(Value::as_str)
                .map(|m| format!(": {m}"))
                .unwrap_or_default();
            return Err(Error::msg(format!(
                "RPC {} refused {method} (HTTP {status}{detail}){}",
                self.url,
                refusal_hint(method)
            )));
        }
        if rate_limited(status.as_u16(), value.get("error")) {
            return Err(Error::msg(rate_limit_message(&self.url, method)));
        }
        if let Some(error) = value.get("error") {
            let data = error.get("data");
            return Ok(Err(RpcFailure {
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("RPC error")
                    .to_string(),
                err: data
                    .and_then(|d| d.get("err"))
                    .cloned()
                    .filter(|v| !v.is_null()),
                logs: strings(data.and_then(|d| d.get("logs"))),
            }));
        }
        Ok(Ok(value.get("result").cloned().unwrap_or(Value::Null)))
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_raw(method, params)?.map_err(|f| {
            let hint = if f.message.to_ascii_lowercase().contains("method not found") {
                refusal_hint(method)
            } else {
                ""
            };
            Error::msg(format!("RPC {method}: {}{hint}", f.message)).with_logs(f.logs)
        })
    }

    fn decode_account(v: &Value) -> Result<Option<Account>> {
        if v.is_null() {
            return Ok(None);
        }
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg("RPC account without base64 data"))?;
        Ok(Some(Account {
            lamports: v.get("lamports").and_then(Value::as_u64).unwrap_or(0),
            owner: v
                .get("owner")
                .and_then(Value::as_str)
                .and_then(|s| Pubkey::from_str(s).ok())
                .unwrap_or_default(),
            data: B64
                .decode(data)
                .map_err(|e| Error::msg(format!("bad account data: {e}")))?,
            executable: v
                .get("executable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }))
    }

    pub fn account(&self, key: &Pubkey) -> Result<Option<Account>> {
        let v = self.call(
            "getAccountInfo",
            json!([key.to_string(), {"encoding": "base64", "commitment": "confirmed"}]),
        )?;
        Self::decode_account(v.get("value").unwrap_or(&Value::Null))
    }

    /// Up to 100 accounts per call, in order.
    pub fn accounts(&self, keys: &[Pubkey]) -> Result<Vec<Option<Account>>> {
        let mut out = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(100) {
            let list: Vec<String> = chunk.iter().map(ToString::to_string).collect();
            let v = self.call(
                "getMultipleAccounts",
                json!([list, {"encoding": "base64", "commitment": "confirmed"}]),
            )?;
            let values = v
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::msg("RPC getMultipleAccounts without value"))?;
            for value in values {
                out.push(Self::decode_account(value)?);
            }
        }
        Ok(out)
    }

    pub fn program_accounts(
        &self,
        program: &Pubkey,
        filters: &[Filter],
    ) -> Result<Vec<(Pubkey, Account)>> {
        let filters: Vec<Value> = filters
            .iter()
            .map(|f| match f {
                Filter::DataSize(n) => json!({"dataSize": n}),
                Filter::Memcmp { offset, bytes } => json!({"memcmp": {
                    "offset": offset,
                    "bytes": B64.encode(bytes),
                    "encoding": "base64",
                }}),
            })
            .collect();
        let v = self.call(
            "getProgramAccounts",
            json!([program.to_string(), {
                "encoding": "base64",
                "commitment": "confirmed",
                "filters": filters,
            }]),
        )?;
        let list = v
            .as_array()
            .ok_or_else(|| Error::msg("RPC getProgramAccounts without a list"))?;
        let mut out = Vec::with_capacity(list.len());
        for item in list {
            let key = item
                .get("pubkey")
                .and_then(Value::as_str)
                .and_then(|s| Pubkey::from_str(s).ok())
                .ok_or_else(|| Error::msg("RPC program account without pubkey"))?;
            if let Some(account) =
                Self::decode_account(item.get("account").unwrap_or(&Value::Null))?
            {
                out.push((key, account));
            }
        }
        Ok(out)
    }

    pub fn balance(&self, key: &Pubkey) -> Result<u64> {
        let v = self.call(
            "getBalance",
            json!([key.to_string(), {"commitment": "confirmed"}]),
        )?;
        Ok(v.get("value").and_then(Value::as_u64).unwrap_or(0))
    }

    pub fn slot(&self) -> Result<u64> {
        let v = self.call("getSlot", json!([{"commitment": "confirmed"}]))?;
        v.as_u64()
            .ok_or_else(|| Error::msg("RPC getSlot without a slot"))
    }

    /// The cluster's clock. A dev validator's clock can lag wall time, and
    /// programs only ever see this one.
    pub fn clock(&self) -> Result<Clock> {
        let account = self
            .account(&sysvar::clock::ID)?
            .ok_or_else(|| Error::msg("the cluster has no Clock sysvar"))?;
        let d = &account.data;
        if d.len() < 40 {
            return Err(Error::msg("the Clock sysvar is too short"));
        }
        Ok(Clock {
            unix_timestamp: i64::from_le_bytes(d[32..40].try_into().unwrap()),
        })
    }

    pub fn genesis_hash(&self) -> Result<String> {
        Ok(self
            .call("getGenesisHash", json!([]))?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub fn version(&self) -> Result<String> {
        let v = self.call("getVersion", json!([]))?;
        Ok(v.get("solana-core")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    pub fn block_height(&self) -> Result<u64> {
        let v = self.call("getBlockHeight", json!([{"commitment": "confirmed"}]))?;
        Ok(v.as_u64().unwrap_or(0))
    }

    pub fn latest_blockhash(&self) -> Result<(Hash, u64)> {
        let v = self.call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))?;
        let value = v.get("value").cloned().unwrap_or(Value::Null);
        let hash = value
            .get("blockhash")
            .and_then(Value::as_str)
            .and_then(|s| Hash::from_str(s).ok())
            .ok_or_else(|| Error::msg("RPC getLatestBlockhash without a blockhash"))?;
        let last_valid = value
            .get("lastValidBlockHeight")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        Ok((hash, last_valid))
    }

    /// Simulates without signature checks and with a fresh blockhash.
    pub fn simulate(&self, tx: &Transaction) -> Result<Simulation> {
        let wire = B64.encode(bincode::serialize(tx).expect("transaction serializes"));
        let v = self.call(
            "simulateTransaction",
            json!([wire, {
                "encoding": "base64",
                "sigVerify": false,
                "replaceRecentBlockhash": true,
                "commitment": "confirmed",
            }]),
        )?;
        let value = v.get("value").cloned().unwrap_or(Value::Null);
        Ok(Simulation {
            err: value.get("err").cloned().filter(|e| !e.is_null()),
            logs: strings(value.get("logs")),
            units: value.get("unitsConsumed").and_then(Value::as_u64),
        })
    }

    /// Sends with preflight. A preflight failure comes back as `Err` with the
    /// simulation's `err` and logs.
    pub fn send(&self, tx: &Transaction) -> Result<std::result::Result<Signature, RpcFailure>> {
        let wire = B64.encode(bincode::serialize(tx).expect("transaction serializes"));
        let r = self.call_raw(
            "sendTransaction",
            json!([wire, {
                "encoding": "base64",
                "preflightCommitment": "confirmed",
                "maxRetries": 5,
            }]),
        )?;
        Ok(r.map(|v| {
            v.as_str()
                .and_then(|s| Signature::from_str(s).ok())
                .unwrap_or_default()
        }))
    }

    /// Waits until the signature is confirmed (or fails, or expires).
    pub fn confirm(&self, signature: &Signature, last_valid: u64) -> Result<Option<Value>> {
        let started = Instant::now();
        loop {
            let v = self.call(
                "getSignatureStatuses",
                json!([[signature.to_string()], {"searchTransactionHistory": false}]),
            )?;
            if let Some(status) = v
                .get("value")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .filter(|s| !s.is_null())
            {
                let err = status.get("err").cloned().filter(|e| !e.is_null());
                let level = status
                    .get("confirmationStatus")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if err.is_some() || level == "confirmed" || level == "finalized" {
                    return Ok(err);
                }
            }
            if started.elapsed() > Duration::from_secs(90) {
                return Err(Error::msg(format!(
                    "{signature} was not confirmed within 90 s; check it with `petri history`"
                )));
            }
            if started.elapsed() > Duration::from_secs(5)
                && self.block_height().unwrap_or(0) > last_valid
            {
                return Err(Error::msg(format!(
                    "{signature} expired before it landed; nothing was executed"
                )));
            }
            thread::sleep(Duration::from_millis(400));
        }
    }

    /// The landed transaction's logs, asked for until the node has them (it
    /// confirmed the transaction already; no retry cap, preflight constraint C37).
    pub fn transaction(&self, signature: &Signature) -> Result<Option<Landed>> {
        loop {
            let v = self.call(
                "getTransaction",
                json!([signature.to_string(), {
                    "encoding": "json",
                    "commitment": "confirmed",
                    "maxSupportedTransactionVersion": 0,
                }]),
            )?;
            if !v.is_null() {
                let meta = v.get("meta").cloned().unwrap_or(Value::Null);
                return Ok(Some(Landed {
                    slot: v.get("slot").and_then(Value::as_u64).unwrap_or(0),
                    err: meta.get("err").cloned().filter(|e| !e.is_null()),
                    logs: strings(meta.get("logMessages")),
                }));
            }
            thread::sleep(Duration::from_millis(300));
        }
    }

    /// DEV-ONLY: REMOVE FOR PROD. `petri faucet --sol` (`dev-faucet` builds only).
    #[cfg(feature = "dev-faucet")]
    pub fn request_airdrop(&self, key: &Pubkey, lamports: u64) -> Result<Signature> {
        let v = self.call(
            "requestAirdrop",
            json!([key.to_string(), lamports, {"commitment": "confirmed"}]),
        )?;
        v.as_str()
            .and_then(|s| Signature::from_str(s).ok())
            .ok_or_else(|| Error::msg("the RPC did not return an airdrop signature"))
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::{Rpc, rate_limited};
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn a_rate_limited_call_is_named_as_such_whatever_the_rpc_says() {
        assert!(rate_limited(429, None));
        let public = json!({"code": 429, "message": "Too many requests for a specific RPC call"});
        assert!(rate_limited(200, Some(&public)));
        assert!(rate_limited(
            200,
            Some(&json!({"code": -32005, "message": "Too many requests"}))
        ));
        assert!(!rate_limited(
            200,
            Some(&json!({"code": -32601, "message": "Method not found"}))
        ));
        assert!(!rate_limited(503, None));
    }

    /// Solana's public RPC answers a rate-limited getProgramAccounts with
    /// HTTP 429 and a JSON-RPC error: Petri says to use another RPC, and
    /// asks only once.
    #[test]
    fn a_rate_limited_rpc_points_to_rpc_and_is_asked_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let serving = listener.try_clone().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = serving.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#"{"jsonrpc":"2.0","error":{"code":429,"message":"Too many requests for a specific RPC call"},"id":1}"#;
            let reply = format!(
                "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        let error = Rpc::new(&url)
            .call("getProgramAccounts", json!([]))
            .unwrap_err()
            .message;
        server.join().unwrap();
        assert!(error.contains("--rpc <provider URL>"), "{error}");
        assert!(error.contains("PETRI_RPC_URL"), "{error}");
        assert!(error.contains("getProgramAccounts"), "{error}");
        assert!(!error.contains(" is unreachable"), "{error}");
        // No retry: nothing else connected.
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err(), "petri asked again");
    }
}

#[cfg(test)]
mod method_tests {
    use super::{METHODS, Served};
    use std::{fs, path::Path};

    /// The method names passed to `call`/`call_raw` as string literals.
    fn called(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        for opener in ["call(", "call_raw("] {
            let mut rest = source;
            while let Some(at) = rest.find(opener) {
                rest = &rest[at + opener.len()..];
                let arg = rest.trim_start();
                if let Some(text) = arg.strip_prefix('"')
                    && let Some(end) = text.find('"')
                    && !text[..end].is_empty()
                    && text[..end].chars().all(|c| c.is_ascii_alphanumeric())
                {
                    out.push(text[..end].to_string());
                }
            }
        }
        out
    }

    fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((
                    path.display().to_string(),
                    fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }

    /// Every RPC method petri calls is listed in `METHODS` (and so served by
    /// the default RPC, a full node, or marked as the dev build's), and the
    /// command guide documents it.
    #[test]
    fn every_rpc_method_is_served_by_the_default_rpc_or_documented() {
        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut seen = Vec::new();
        for (path, source) in &files {
            for method in called(source) {
                assert!(
                    METHODS.iter().any(|(m, _, _)| *m == method),
                    "{path} calls the RPC method {method}, which rpc::METHODS does not list"
                );
                seen.push(method);
            }
        }
        let guide = include_str!("../../docs/amoeba-cli-command-guide.md");
        for (method, _, served) in METHODS {
            assert!(
                seen.iter().any(|m| m == method),
                "rpc::METHODS lists {method}, which nothing calls"
            );
            assert!(
                guide.contains(&format!("`{method}`")),
                "the command guide's RPC methods leave out {method}"
            );
            // Only the dev build's faucet needs more than a full RPC node.
            assert!(
                *served != Served::LocalValidator || *method == "requestAirdrop",
                "{method}"
            );
        }
    }
}
