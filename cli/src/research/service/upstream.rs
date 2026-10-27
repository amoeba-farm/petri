//! What the workspace reads and writes at the futures API: its candidate
//! store (researcher registration, candidates, web proofs) and the oracle's
//! public documents that discovery assesses.

use crate::research::workspace::{Failure, endpoint, fail_status};
use reqwest::{
    Method,
    blocking::{Client, Response},
};
use serde_json::{Value, json};
use std::{io::Read, time::Duration};

/// The candidate store, as the workspace uses it.
pub trait Upstream: Send + Sync {
    /// A new researcher for the owner's wallet: `{ agent: { agentId,
    /// payoutAuthority }, credential }`.
    fn register(&self, payout_authority: &str) -> Result<Value, Failure>;
    fn submit(&self, details: &Value, credential: &str) -> Result<Value, Failure>;
    /// A web proof bundle, its bytes unchanged: the API verifies and stores
    /// it and answers with its bundleDigest (422 when it does not verify).
    fn upload_evidence(&self, bundle: &[u8], credential: &str) -> Result<Value, Failure>;
    /// The agent's candidates and outcomes: `{ candidates, complete }`.
    fn candidates(&self, agent_id: &str) -> Result<Value, Failure>;
}

/// The oracle's coverage documents: `{ snapshot, catalog }`.
pub trait OracleReader: Send + Sync {
    fn read(&self) -> Result<Value, Failure>;
}

const UPSTREAM_LIMIT: u64 = 2 * 1024 * 1024;
const ORACLE_LIMIT: u64 = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("an HTTP client")
}

/// A response body of at most `limit` bytes (a declared longer one is
/// refused before reading), as JSON; `too_large` otherwise.
fn bounded(
    response: Response,
    limit: u64,
    too_large: impl Fn() -> Failure,
) -> Result<Result<Value, ()>, Failure> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| too_large())?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(serde_json::from_slice(&bytes).map_err(|_| ()))
}

/// JavaScript's falsy JSON values.
fn falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// The futures API's candidate store (`/oracle/*`).
pub struct Api {
    base: String,
    http: Client,
}

enum Body<'a> {
    None,
    Json(&'a Value),
    Raw(&'a [u8]),
}

impl Api {
    pub fn new(base: &str) -> Result<Self, Failure> {
        Ok(Self {
            base: endpoint(base)?,
            http: client(),
        })
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        body: Body,
        credential: Option<&str>,
        rejected: Option<(&str, &str)>,
    ) -> Result<Value, Failure> {
        let unavailable = || {
            fail_status(
                "UPSTREAM_UNAVAILABLE",
                "The source validation service is unavailable. Your workspace is still available.",
                503,
            )
        };
        let mut builder = self
            .http
            .request(method, format!("{}{path}", self.base))
            .timeout(TIMEOUT)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("cache-control", "no-store");
        if let Some(credential) = credential {
            builder = builder.bearer_auth(credential);
        }
        builder = match body {
            Body::None => builder,
            Body::Json(value) => builder.body(value.to_string()),
            Body::Raw(bytes) => builder.body(bytes.to_vec()),
        };
        let response = builder.send().map_err(|_| unavailable())?;
        // A redirect is refused like a lost connection: credentials never follow one.
        if response.status().is_redirection() {
            return Err(unavailable());
        }
        let status = response.status();
        let too_large = || {
            fail_status(
                "UPSTREAM_INVALID",
                "The validation response is too large.",
                502,
            )
        };
        let result = bounded(response, UPSTREAM_LIMIT, too_large)?.map_err(|_| {
            fail_status(
                "UPSTREAM_INVALID",
                "The validation service returned an invalid response.",
                502,
            )
        })?;
        if !status.is_success() || result["ok"] != true || falsy(&result["data"]) {
            // Remote error text is untrusted and may contain transport details or secrets.
            if status.as_u16() < 500
                && let Some((code, message)) = rejected
            {
                return Err(fail_status(code, message, 422));
            }
            return Err(if status.as_u16() >= 500 {
                fail_status("SOURCE_REJECTED", "Source validation is unavailable.", 503)
            } else {
                fail_status(
                    "SOURCE_REJECTED",
                    "The source service rejected this request. Check the required source fields and the connected wallet.",
                    422,
                )
            });
        }
        Ok(result["data"].clone())
    }
}

/// `encodeURIComponent`.
fn component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn hex64(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

impl Upstream for Api {
    fn register(&self, payout_authority: &str) -> Result<Value, Failure> {
        let body = json!({ "payoutAuthority": payout_authority });
        self.request(
            Method::POST,
            "/oracle/agents",
            Body::Json(&body),
            None,
            None,
        )
    }

    fn submit(&self, details: &Value, credential: &str) -> Result<Value, Failure> {
        self.request(
            Method::POST,
            "/oracle/source-candidates",
            Body::Json(details),
            Some(credential),
            None,
        )
    }

    fn upload_evidence(&self, bundle: &[u8], credential: &str) -> Result<Value, Failure> {
        self.request(
            Method::POST,
            "/oracle/evidence",
            Body::Raw(bundle),
            Some(credential),
            Some((
                "EVIDENCE_REJECTED",
                "The API could not verify this web proof. Check it with petri evidence verify <bundle file>, prove the page again if it fails, and resubmit.",
            )),
        )
    }

    fn candidates(&self, agent_id: &str) -> Result<Value, Failure> {
        let mut candidates = Vec::new();
        let mut after = String::new();
        for _ in 0..10 {
            let path = format!(
                "/oracle/source-candidates?agentId={}&limit=100{}",
                component(agent_id),
                if after.is_empty() {
                    String::new()
                } else {
                    format!("&after={after}")
                }
            );
            let result = self.request(Method::GET, &path, Body::None, None, None)?;
            let page = result["candidates"]
                .as_array()
                .filter(|page| page.iter().all(|c| c["agentId"].as_str() == Some(agent_id)))
                .ok_or_else(|| {
                    fail_status("UPSTREAM_INVALID", "Invalid candidate attribution.", 502)
                })?;
            candidates.extend(page.iter().cloned());
            if page.len() < 100 {
                return Ok(json!({ "candidates": candidates, "complete": true }));
            }
            let next = result["nextCursor"]
                .as_str()
                .filter(|_| hex64(&result["nextCursor"]));
            match next {
                Some(next) if next > after.as_str() => after = next.to_string(),
                _ => {
                    return Err(fail_status(
                        "UPSTREAM_INVALID",
                        "Invalid candidate cursor.",
                        502,
                    ));
                }
            }
        }
        Ok(json!({ "candidates": candidates, "complete": false }))
    }
}

// The futures API's oracle documents.
pub const ORACLE_SNAPSHOT_PATH: &str = "/v1/oracle/public-snapshot";
pub const ORACLE_CATALOG_PATH: &str = "/v1/oracle/catalog";

/// The oracle's prepared public views at the futures API: only these two
/// GET routes, never per-source RPC reads, and no workspace credential.
pub struct Oracle {
    base: String,
    http: Client,
}

impl Oracle {
    pub fn new(base: &str) -> Result<Self, Failure> {
        Ok(Self {
            base: endpoint(base)?,
            http: client(),
        })
    }

    fn get(&self, path: &str) -> Result<Value, Failure> {
        let unavailable = || {
            fail_status(
                "ORACLE_UNAVAILABLE",
                "The oracle read service is unavailable.",
                503,
            )
        };
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .timeout(TIMEOUT)
            .header("accept", "application/json")
            .header("cache-control", "no-store")
            .send()
            .map_err(|_| unavailable())?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|l| l > ORACLE_LIMIT)
        {
            return Err(unavailable());
        }
        let too_large = || {
            fail_status(
                "ORACLE_UNAVAILABLE",
                "The oracle response exceeds the read bound.",
                503,
            )
        };
        let result = bounded(response, ORACLE_LIMIT, too_large)?.map_err(|_| Failure {
            code: None,
            message: "The oracle response is not JSON.".into(),
            status: None,
        })?;
        if result["ok"] != true || falsy(&result["data"]) {
            return Err(unavailable());
        }
        Ok(result["data"].clone())
    }
}

impl OracleReader for Oracle {
    fn read(&self) -> Result<Value, Failure> {
        let (snapshot, catalog) = std::thread::scope(|scope| {
            let snapshot = scope.spawn(|| self.get(ORACLE_SNAPSHOT_PATH));
            let catalog = self.get(ORACLE_CATALOG_PATH);
            (
                snapshot.join().unwrap_or_else(|_| {
                    Err(fail_status(
                        "ORACLE_UNAVAILABLE",
                        "The oracle read service is unavailable.",
                        503,
                    ))
                }),
                catalog,
            )
        });
        Ok(json!({ "snapshot": snapshot?, "catalog": catalog? }))
    }
}
