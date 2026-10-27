//! The bot registry in the futures API (`docs/bots.md`, "Publishing,
//! installing, ranking"): publishing a bundle, the published bots and
//! their board, and each running bot's registration and P&L. Signatures
//! are ed25519 over the UTF-8 message, base58 in the body.

use serde_json::{Value, json};
use solana_signer::Signer;

use crate::{
    endpoints::percent_encode,
    futures::{
        api::Api,
        error::{Error, Result},
    },
};

/// What a publish signs: the bundle's hash.
pub fn publish_message(hash: &str) -> String {
    format!("amoeba bot publish v1\n{hash}")
}

/// What a run's registration signs, by the bot's wallet and its owner's.
pub fn run_message(wallet: &str, owner: &str, bot: Option<&str>, hash: &str) -> String {
    format!(
        "amoeba bot run v1\n{wallet}\n{owner}\n{}\n{hash}",
        bot.unwrap_or("")
    )
}

fn sign(signer: &dyn Signer, message: &str, who: &str) -> Result<String> {
    signer
        .try_sign_message(message.as_bytes())
        .map(|s| s.to_string())
        .map_err(|e| Error::msg(format!("{who} did not sign: {e}")))
}

/// `POST /v1/bots`: publishes `text` (the bundle without `from`), signed by
/// `author`, naming the GitHub file that holds it when given (the API then
/// checks the two and marks the listing verified). → `{ id, version, hash,
/// publishedAt }`.
pub fn publish(
    api: &Api,
    author: &dyn Signer,
    text: &str,
    hash: &str,
    github: Option<&Github>,
) -> Result<Value> {
    let signature = sign(author, &publish_message(hash), "your wallet")?;
    let mut body = json!({
        "text": text,
        "author": author.pubkey().to_string(),
        "signature": signature,
    });
    if let Some(g) = github {
        body["github"] = g.spec().into();
    }
    api.post("/v1/bots", &body).map_err(refused)
}

/// The registry's refusals in plain words (the API's other messages say
/// what they are already).
fn refused(e: Error) -> Error {
    let m = &e.message;
    let plain = if m.contains("(HTTP 401)") {
        "The registry did not accept your wallet's signature."
    } else if m.contains("(HTTP 409)") {
        "Another author has published a bot with this id; give yours another id."
    } else if m.contains("(HTTP 413)") {
        "This bot's file is larger than the registry takes (256 KiB)."
    } else {
        return e;
    };
    Error::msg(plain)
}

/// `GET /v1/bots?market=&sort=&days=`: the published bots.
pub fn discover(
    api: &Api,
    market: Option<&str>,
    sort: Option<&str>,
    days: Option<u32>,
) -> Result<Value> {
    let mut query = Vec::new();
    if let Some(m) = market {
        query.push(format!("market={}", percent_encode(m.trim())));
    }
    if let Some(s) = sort {
        query.push(format!("sort={}", percent_encode(s.trim())));
    }
    if let Some(d) = days {
        query.push(format!("days={d}"));
    }
    let path = if query.is_empty() {
        "/v1/bots".to_string()
    } else {
        format!("/v1/bots?{}", query.join("&"))
    };
    api.get(&path)
}

/// `GET /v1/bots/:id`: one published bot, with its latest text.
pub fn show(api: &Api, id: &str) -> Result<Value> {
    api.get(&format!("/v1/bots/{}", percent_encode(id.trim())))
}

/// `GET /v1/bots/leaderboard?days=`.
pub fn leaderboard(api: &Api, days: Option<u32>) -> Result<Value> {
    match days {
        Some(d) => api.get(&format!("/v1/bots/leaderboard?days={d}")),
        None => api.get("/v1/bots/leaderboard"),
    }
}

/// A run to register: the bot's wallet trading `hash` for its owner.
pub struct Registration<'a> {
    /// The registry id it came from, if any.
    pub bot: Option<&'a str>,
    pub hash: &'a str,
    pub name: &'a str,
    /// The market's address.
    pub market: &'a str,
}

/// `POST /v1/bots/runs`, signed by the bot's wallet and by its owner (the
/// main wallet). The latest registration of a wallet replaces the earlier.
pub fn register(
    api: &Api,
    wallet: &dyn Signer,
    owner: &dyn Signer,
    run: &Registration,
) -> Result<Value> {
    let (w, o) = (wallet.pubkey().to_string(), owner.pubkey().to_string());
    let message = run_message(&w, &o, run.bot, run.hash);
    api.post(
        "/v1/bots/runs",
        &json!({
            "wallet": w,
            "owner": o,
            "bot": run.bot,
            "hash": run.hash,
            "name": run.name,
            "market": run.market,
            "walletSignature": sign(wallet, &message, "the bot's wallet")?,
            "ownerSignature": sign(owner, &message, "your wallet")?,
        }),
    )
}

/// What the API says of a bot wallet's run (`GET /v1/bots/runs/:wallet`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pnl {
    pub pnl_atoms: Option<String>,
    /// `[{ t, pnlAtoms }]`, as the API sends them.
    pub points: Value,
    pub counted: Option<bool>,
}

/// Reads `wallet`'s run.
pub fn pnl(api: &Api, wallet: &str) -> Result<Pnl> {
    let v = api.get(&format!("/v1/bots/runs/{}", percent_encode(wallet)))?;
    Ok(Pnl {
        pnl_atoms: v.get("pnlAtoms").and_then(|p| match p {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }),
        points: v.get("points").cloned().unwrap_or(Value::Null),
        counted: v.get("counted").and_then(Value::as_bool),
    })
}

/// A bot file on GitHub: `OWNER/REPO[/PATH][@REF]`, `PATH` `petri-bot.toml`
/// and `REF` `HEAD` unless given (`docs/bots.md`, "GitHub").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Github {
    pub owner: String,
    pub repo: String,
    pub path: String,
    pub reference: String,
}

/// The prefix an install's id takes for a bot on GitHub.
pub const GITHUB: &str = "github:";

impl Github {
    /// Reads `OWNER/REPO[/PATH][@REF]` (a leading `github:` is fine).
    pub fn parse(spec: &str) -> Result<Self> {
        let bad = || {
            Error::msg(format!(
                "`{spec}` is not a GitHub file: OWNER/REPO, then /PATH and @REF if needed"
            ))
        };
        let text = spec.trim();
        let text = text.strip_prefix(GITHUB).unwrap_or(text);
        let (place, reference) = match text.rsplit_once('@') {
            Some((place, r)) => (place, r),
            None => (text, "HEAD"),
        };
        let word = |s: &str, extra: &str| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c) || extra.contains(c))
        };
        let mut parts = place.split('/');
        let owner = parts.next().filter(|s| word(s, "")).ok_or_else(bad)?;
        let repo = parts.next().filter(|s| word(s, ".")).ok_or_else(bad)?;
        let path: Vec<&str> = parts.collect();
        if !path.iter().all(|s| word(s, ".")) {
            return Err(bad());
        }
        if !reference.split('/').all(|s| word(s, ".")) {
            return Err(bad());
        }
        Ok(Self {
            owner: owner.into(),
            repo: repo.into(),
            path: if path.is_empty() {
                "petri-bot.toml".into()
            } else {
                path.join("/")
            },
            reference: reference.into(),
        })
    }

    /// `OWNER/REPO/PATH@REF`, in full.
    pub fn spec(&self) -> String {
        format!(
            "{}/{}/{}@{}",
            self.owner, self.repo, self.path, self.reference
        )
    }

    /// What an installed copy's `from` says: `github:OWNER/REPO/PATH@REF`.
    pub fn from(&self) -> String {
        format!("{GITHUB}{}", self.spec())
    }

    /// The file's raw text.
    pub fn raw_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            self.owner, self.repo, self.reference, self.path
        )
    }

    /// Fetches the file's text from GitHub.
    pub fn fetch(&self) -> Result<String> {
        let url = self.raw_url();
        let unreachable = |e: reqwest::Error| Error::msg(format!("couldn't fetch {url}: {e}"));
        // No request timeout, as the API client (preflight constraint C35).
        let response = reqwest::blocking::Client::builder()
            .timeout(None::<std::time::Duration>)
            .build()
            .map_err(unreachable)?
            .get(&url)
            .send()
            .map_err(unreachable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::msg(format!(
                "GitHub has no file at {url} (HTTP {})",
                status.as_u16()
            )));
        }
        response.text().map_err(unreachable)
    }
}
