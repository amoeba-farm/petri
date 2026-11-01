//! Oracle v2, "heads on pikes" (`oracle/docs/heads.md`), with the long-term agreement channel
//! layer (`oracle/docs/lta.md`): tag 210 of the oracle program. Stakers stake AMBA into the
//! oracle and delegate it to sources ("heads") posted by name and spec; a head with enough
//! backing turns active, the first fill that survives its window makes it live, weighted by its
//! delegation; anyone keeps it current with bonded claims (fills and updates), anyone challenges
//! a claim or the source itself, and losses slash.
//!
//! Reads print the futures API's documents verbatim (`/v1/oracle/heads*`, `api/API.md`
//! "Oracle heads"), or the same documents built from chain through `ameba_oracle::heads_view`
//! when the API can't answer (`docs.rs`); `heads mine` is petri's own, from chain, with the
//! follow-up commands that move the user's money along. Writes are built with the oracle crate's
//! tag 210 builders, checked against the accounts as the program will load them (the exact
//! bond and stake, the windows, the state), reviewed and sent like every oracle write
//! (`super::send`: the identity gate, the governance gate, `--simulate`, `--yes`), and print
//! `{ action: "heads.<name>", signature, product, head, … }`.

pub mod accounts;
pub mod args;
pub mod claims;
pub mod docs;
pub mod material;
pub mod reads;
pub mod writes;

use serde_json::{Map, Value, json};

use args::{HeadsArgs, HeadsCommand};

use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result},
    units::{parse_decimal, trim_decimal},
    view::Paint,
};

/// AMBA's decimals (the oracle's heads config records the mint's; the program and the API
/// count AMBA atoms at nine).
pub const AMBA_DECIMALS: u32 = 9;

pub fn run(ctx: &Ctx, p: Paint, a: &HeadsArgs) -> Result<Report> {
    use HeadsCommand as H;
    let Some(command) = &a.command else {
        return reads::boards(ctx, p, a.product.as_deref());
    };
    match command {
        H::Show(h) => reads::show(ctx, p, &h.product, h.head),
        H::Mine { owner } => reads::mine(ctx, p, owner.as_deref()),
        H::Terms { target } => reads::terms(ctx, p, target),
        H::Stake { amount, send } => writes::stake(ctx, p, amount, send.simulate),
        H::Unstake { amount, send } => writes::unstake(ctx, p, amount, send.simulate),
        H::Post(post) => writes::post(ctx, p, post),
        H::Delegate { head, amount, send } => {
            writes::delegate(ctx, p, &head.product, head.head, amount, send.simulate)
        }
        H::Undelegate { head, amount, send } => {
            writes::undelegate(ctx, p, &head.product, head.head, amount, send.simulate)
        }
        H::Sync { head, owner, send } => writes::sync(
            ctx,
            p,
            &head.product,
            head.head,
            owner.as_deref(),
            send.simulate,
        ),
        H::Fill(c) => claims::claim(ctx, p, claims::Kind::Fill, c),
        H::Update(c) => claims::claim(ctx, p, claims::Kind::Update, c),
        H::Reveal { claim, send } => {
            claims::reveal(ctx, p, &claim.product, claim.head, claim.seq, send.simulate)
        }
        H::Challenge(c) => claims::challenge(ctx, p, c),
        H::ChallengeSource(c) => claims::challenge_source(ctx, p, c),
        H::Settle {
            head,
            seq,
            product,
            send,
        } => claims::settle(ctx, p, product, *head, *seq, send.simulate),
        H::Rewards { claim, owner, send } => {
            if *claim {
                writes::claim_rewards(ctx, p, send.simulate)
            } else {
                reads::rewards(ctx, p, owner.as_deref())
            }
        }
        H::Prize { claim, send } => {
            writes::prize(ctx, p, &claim.product, claim.head, claim.seq, send.simulate)
        }
    }
}

// ------------------------------------------------------------------------------------ units

/// An AMBA amount in atoms: `1500`, `0.5` (at most nine decimals), more than zero.
pub fn parse_amba(text: &str) -> Result<u64> {
    let atoms = parse_decimal(text, AMBA_DECIMALS)
        .map_err(|e| Error::msg(format!("{e} (an amount of AMBA such as 1500 or 0.5)")))?;
    let atoms = u64::try_from(atoms).map_err(|_| Error::msg(format!("`{text}` is too large")))?;
    if atoms == 0 {
        return Err(Error::msg("The amount must be more than zero."));
    }
    Ok(atoms)
}

/// An AMBA amount, or `all` (`max`) for `limit`.
pub fn parse_amba_or_all(text: &str, limit: u64) -> Result<u64> {
    if matches!(text.trim().to_ascii_lowercase().as_str(), "all" | "max") {
        return Ok(limit);
    }
    parse_amba(text)
}

/// A value on the oracle's six-decimal scale (`4.85` → 4,850,000), more than zero.
pub fn parse_value(text: &str) -> Result<u64> {
    let atoms = parse_decimal(text, 6)
        .map_err(|e| Error::msg(format!("{e} (a value such as 4.85, at most six decimals)")))?;
    u64::try_from(atoms)
        .ok()
        .filter(|a| *a > 0)
        .ok_or_else(|| Error::msg(format!("`{text}` is not a positive value")))
}

/// A time: unix seconds, or a UTC time such as `2026-10-28T09:00:00Z` (seconds optional).
pub fn parse_time(text: &str) -> Result<u64> {
    let t = text.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        return t
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| Error::msg(format!("`{text}` is not a time")));
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(t)
        .map(|d| d.timestamp())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%MZ").map(|d| d.and_utc().timestamp())
        })
        .map_err(|_| {
            Error::msg(format!(
                "`{text}` is not a time: give unix seconds or a UTC time such as 2026-10-28T09:00:00Z"
            ))
        })?;
    u64::try_from(parsed)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::msg(format!("`{text}` is before 1970")))
}

fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1,500 AMBA`, `0.25 AMBA`.
pub fn amba(atoms: u64) -> String {
    let text = trim_decimal(u128::from(atoms), AMBA_DECIMALS);
    let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
    let whole = group(whole);
    if frac.is_empty() {
        format!("{whole} AMBA")
    } else {
        format!("{whole}.{frac} AMBA")
    }
}

/// A six-decimal value as text (`4850000` → `4.85`).
pub fn value(atoms: u64) -> String {
    trim_decimal(u128::from(atoms), 6)
}

/// `"1500000000000"`: an atom amount as the documents carry it.
pub fn atoms(v: u64) -> Value {
    json!(v.to_string())
}

/// A unix time, `null` for 0 (has not happened).
pub fn time_or_null(ts: u64) -> Value {
    if ts == 0 { Value::Null } else { json!(ts) }
}

/// `2026-10-28 09:00 UTC`.
pub fn at(ts: u64) -> String {
    crate::futures::view::utc(i64::try_from(ts).unwrap_or(i64::MAX))
}

// ---------------------------------------------------------------------------- results

/// A sent (or simulated) heads write as the plugin reads it: `details` lifted to the top beside
/// `ok`, `action` and what was sent, and `signature`, the last transaction's.
pub fn flatten(mut report: Report) -> Report {
    let details = report
        .payload
        .as_object_mut()
        .and_then(|o| o.remove("details"))
        .unwrap_or(Value::Null);
    let signature = report.payload["transactions"]
        .as_array()
        .and_then(|t| t.iter().rev().find_map(|t| t["signature"].as_str()))
        .map(str::to_string);
    if let Some(top) = report.payload.as_object_mut() {
        if let Value::Object(details) = details {
            for (key, value) in details {
                top.entry(key).or_insert(value);
            }
        }
        if let Some(signature) = signature {
            top.insert("signature".into(), json!(signature));
        }
    }
    report
}

/// Sets `fields` on a result's top level (what a write reads back once it landed).
pub fn set(report: &mut Report, fields: Value) {
    if let (Some(top), Value::Object(fields)) = (report.payload.as_object_mut(), fields) {
        let fields: Map<String, Value> = fields;
        top.extend(fields);
    }
}

/// Whether a result is a send that landed (not a simulation).
pub fn landed(report: &Report) -> bool {
    report.payload.get("simulated") != Some(&Value::Bool(true))
}

/// `petri oracle heads …`: the follow-up commands `heads mine` and the results list. The plugin
/// runs these forms (`plugins/petri/lib/oracle.mjs`, `STEPS`), word for word; its
/// `heads fetcher withdraw` has no command since fetchers hold no seats.
pub mod commands {
    pub fn sync(product: &str, head: u32) -> String {
        format!("petri oracle heads sync {head} --product {product}")
    }
    pub fn prize(product: &str, head: u32, seq: u32) -> String {
        format!("petri oracle heads prize {head} {seq} --product {product}")
    }
    pub fn rewards() -> String {
        "petri oracle heads rewards --claim".to_string()
    }
    /// Not one of the three: printed where a fill or update's reveal failed.
    pub fn reveal(product: &str, head: u32, seq: u32) -> String {
        format!("petri oracle heads reveal {head} {seq} --product {product}")
    }
    pub fn settle(product: &str, head: u32, seq: Option<u32>) -> String {
        match seq {
            Some(seq) => format!("petri oracle heads settle {head} {seq} --product {product}"),
            None => format!("petri oracle heads settle {head} --product {product}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_values_and_times_parse() {
        assert_eq!(parse_amba("1500").unwrap(), 1_500_000_000_000);
        assert_eq!(parse_amba("250.5").unwrap(), 250_500_000_000);
        assert_eq!(parse_amba("0.000000001").unwrap(), 1);
        assert!(parse_amba("0.0000000001").is_err());
        assert!(parse_amba("0").is_err());
        assert!(parse_amba("1e3").is_err());
        assert_eq!(parse_amba_or_all("all", 7).unwrap(), 7);
        assert_eq!(parse_value("4.85").unwrap(), 4_850_000);
        assert!(parse_value("2.1234567").is_err());
        assert!(parse_value("0").is_err());
        assert_eq!(parse_time("1793178000").unwrap(), 1_793_178_000);
        assert_eq!(parse_time("2026-10-28T09:00:00Z").unwrap(), 1_793_178_000);
        assert_eq!(parse_time("2026-10-28T09:00Z").unwrap(), 1_793_178_000);
        assert!(parse_time("yesterday").is_err());
        assert!(parse_time("2026-10-28 09:00").is_err());
        assert_eq!(amba(1_500_000_000_000), "1,500 AMBA");
        assert_eq!(amba(250_500_000_000), "250.5 AMBA");
        assert_eq!(value(4_850_000), "4.85");
    }

    /// The three follow-ups the plugin runs, exactly as its `STEPS` reads them.
    #[test]
    fn follow_ups_are_the_plugins_forms() {
        assert_eq!(
            commands::sync("RAMX", 2),
            "petri oracle heads sync 2 --product RAMX"
        );
        assert_eq!(
            commands::prize("RAMX", 0, 3),
            "petri oracle heads prize 0 3 --product RAMX"
        );
        assert_eq!(commands::rewards(), "petri oracle heads rewards --claim");
    }

    #[test]
    fn a_result_lifts_its_details() {
        let report = Report::new(
            json!({ "ok": true, "action": "heads.delegate", "details": { "product": "RAMX", "head": 3, "action": "x" },
                    "transactions": [{ "signature": "a" }, { "signature": "b" }] }),
            String::new(),
        );
        let out = flatten(report).payload;
        assert_eq!(
            out["action"], "heads.delegate",
            "details never overwrite the result's own fields"
        );
        assert_eq!(
            (out["product"].as_str(), out["head"].as_u64()),
            (Some("RAMX"), Some(3))
        );
        assert_eq!(out["signature"], "b");
        assert!(out.get("details").is_none());
    }
}
