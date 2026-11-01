//! The heads reads: the boards (`petri oracle heads [--product P]`), one head (`heads show`),
//! the user's part (`heads mine`), the delegator rewards (`heads rewards`) and a contract's
//! listing terms (`heads terms`). The boards and heads are the futures API's documents verbatim,
//! or the same documents from chain when the API can't answer them (`docs.rs`).

use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    accounts::{self, Chain},
    amba, at,
    docs::{self, Mine, Snapshot},
    value,
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result},
    units::{short, usdc},
    view::Paint,
};

const BOARDS: &str = "/v1/oracle/heads";

fn atoms_of(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_u64())
        .unwrap_or(0)
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The API's document at `path`, else the one `fallback` builds from chain. When both fail, the
/// chain's refusal is the answer (it says what is missing in plain words).
fn api_or_chain(
    ctx: &Ctx,
    path: &str,
    fallback: impl FnOnce() -> Result<Value>,
) -> Result<(Value, &'static str)> {
    match ctx.api.get(path) {
        Ok(doc) if doc.is_object() && doc.get("error").is_none() => Ok((doc, "api")),
        Ok(_) | Err(_) => fallback().map(|doc| (doc, "chain")),
    }
}

// ---------------------------------------------------------------------------------- boards

/// `petri oracle heads [--product P]`: every board (`{ boards, updatedAt }`), or one.
pub fn boards(ctx: &Ctx, p: Paint, product: Option<&str>) -> Result<Report> {
    let product = product.map(accounts::product).transpose()?;
    let path = match product {
        Some(pr) => format!("{BOARDS}/{}", pr.symbol()),
        None => BOARDS.to_string(),
    };
    let (doc, _) = api_or_chain(ctx, &path, || {
        let chain = Chain::reads(ctx)?;
        let snap = Snapshot::read(&chain, &product.into_iter().collect::<Vec<_>>(), None)?;
        Ok(match product {
            Some(_) => snap
                .boards
                .first()
                .map(|b| docs::board_json(&snap, b))
                .unwrap_or(Value::Null),
            None => docs::document(&snap),
        })
    })?;
    let boards: Vec<&Value> = match doc.get("boards").and_then(Value::as_array) {
        Some(list) => list.iter().collect(),
        None => vec![&doc],
    };
    let mut lines = Vec::new();
    if boards.is_empty() {
        lines.push("No heads boards on this network yet.".to_string());
    }
    for b in boards {
        lines.extend(board_lines(p, b));
    }
    Ok(Report::new(doc, lines.join("\n")))
}

fn share(bps: &Value) -> String {
    bps.as_u64()
        .map(|b| format!("{}%", (b as f64 / 100.0).round()))
        .unwrap_or_default()
}

/// What a contract settles on, in words.
fn target_words(target: &str) -> String {
    match target {
        "fixed-mix" | "fixed_mix_reference" => {
            "settles on each item's fixed-mix blend of Spot, Contract and LTA".to_string()
        }
        "contract-channel" | "contract_channel" => {
            "settles on each item's Contract channel".to_string()
        }
        other => format!("settles on {other}"),
    }
}

/// A change in ppm as a percentage (`+1.25%`), `-` without one.
fn pct(ppm: &Value) -> String {
    ppm.as_i64()
        .map(|v| format!("{:+.2}%", v as f64 / 10_000.0))
        .unwrap_or_else(|| "-".to_string())
}

/// One component (an item in one channel): its mix share, its change or why it has none, and
/// its live sources with their shares.
fn component_lines(c: &Value, heads: &[&Value], mix_bps: Option<u64>) -> Vec<String> {
    let index = c["index"].as_u64().unwrap_or(0);
    let sources: Vec<&&Value> = heads
        .iter()
        .filter(|h| h["status"] == "live" && h["component"].as_u64() == Some(index))
        .collect();
    let reading = &c["reading"];
    let change = match text(&reading["status"]) {
        "degraded" => format!(
            "on its last valid value ({})",
            text(&reading["reason"]).replace('-', " ")
        ),
        "no-contract" | "" => "no contract yet".to_string(),
        _ => pct(&c["changePpm"]),
    };
    let mut out = vec![format!(
        "    {index:>2} {}{} · {change} · {} live source{}",
        text(&c["label"]),
        mix_bps
            .map(|m| format!(" ({}% of the mix)", m / 100))
            .unwrap_or_default(),
        sources.len(),
        if sources.len() == 1 { "" } else { "s" }
    )];
    for h in sources {
        out.push(format!(
            "         #{} {} · {} · {}{}",
            h["id"],
            text(&h["name"]),
            amba(atoms_of(&h["delegation"])),
            share(&h["shareBps"]),
            h["value"]
                .as_str()
                .and_then(|v| v.parse().ok())
                .map(|v| format!(" · now {}", value(v)))
                .unwrap_or_default()
        ));
    }
    out
}

fn board_lines(p: Paint, b: &Value) -> Vec<String> {
    let product = text(&b["product"]);
    let heads: Vec<&Value> = b["heads"]
        .as_array()
        .map(|h| h.iter().collect())
        .unwrap_or_default();
    let live = heads.iter().filter(|h| h["status"] == "live").count();
    let mut out = vec![p.bold(&format!(
        "{product} heads board · {live} live source{} · {} delegated · prize pool {} USDC",
        if live == 1 { "" } else { "s" },
        amba(atoms_of(&b["totals"]["delegation"])),
        usdc(atoms_of(&b["totals"]["prizePool"]))
    ))];
    let need = b["params"]["minDelegators"].as_u64().unwrap_or(0);
    out.push(format!(
        "  Posting a head takes at least {}; it turns active at {} with {need} delegator{}.",
        amba(atoms_of(&b["thresholds"]["minPost"])),
        amba(atoms_of(&b["thresholds"]["activation"])),
        if need == 1 { "" } else { "s" }
    ));
    let index = &b["index"];
    if index.is_object() {
        let contract = b["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["period"] == index["contract"])
            .and_then(|c| c["expiry"].as_u64())
            .map(|e| format!("{}, ", quarter_name(product, e)))
            .unwrap_or_default();
        out.push(format!(
            "  Index ({contract}{}): {}",
            target_words(text(&b["settlementTarget"])),
            match text(&index["status"]) {
                "unavailable" => format!("unavailable: {}", text(&index["reason"])),
                status => format!(
                    "{status} {}{}",
                    pct(&index["changePpm"]),
                    index["value"]
                        .as_str()
                        .and_then(|v| v.parse().ok())
                        .map(|v| format!(" → {}", value(v)))
                        .unwrap_or_default()
                ),
            }
        ));
    }
    let items: Vec<&Value> = b["items"]
        .as_array()
        .map(|i| i.iter().collect())
        .unwrap_or_default();
    if items.is_empty() {
        // An older document: its components alone.
        for c in b["components"].as_array().into_iter().flatten() {
            out.extend(component_lines(c, &heads, None));
        }
    }
    for item in items {
        let r = &item["reference"];
        out.push(format!(
            "  {} · {}% of {product} · mix {} · blend {}",
            text(&item["name"]),
            item["basketWeightBps"].as_u64().unwrap_or(0) / 100,
            text(&item["mixBasis"]),
            if r["available"] == true {
                pct(&r["changePpm"])
            } else {
                format!("unavailable ({})", text(&r["reason"]))
            }
        ));
        for c in item["components"].as_array().into_iter().flatten() {
            let mix = item["mixBps"][text(&c["channel"])].as_u64();
            out.extend(component_lines(c, &heads, mix));
        }
    }
    let gathering: Vec<&&Value> = heads
        .iter()
        .filter(|h| h["status"] == "open" || h["status"] == "active")
        .collect();
    if !gathering.is_empty() {
        out.push("  Gathering backing:".to_string());
        for h in gathering {
            out.push(format!(
                "    #{} {} · {} · {} · {} · {} delegator{}",
                h["id"],
                text(&h["name"]),
                text(&h["componentLabel"]),
                text(&h["status"]),
                amba(atoms_of(&h["delegation"])),
                h["delegators"],
                if h["delegators"] == 1 { "" } else { "s" }
            ));
        }
    }
    out.push(format!(
        "  One head: petri oracle heads show <HEAD> --product {product}"
    ));
    out
}

// ---------------------------------------------------------------------------------- one head

/// `petri oracle heads show <head> --product P`.
pub fn show(ctx: &Ctx, p: Paint, product: &str, head: u32) -> Result<Report> {
    let product = accounts::product(product)?;
    let path = format!("{BOARDS}/{}/{head}", product.symbol());
    let (doc, _) = api_or_chain(ctx, &path, || {
        let chain = Chain::reads(ctx)?;
        chain.config()?;
        let board = chain.board(product)?;
        let (key, _) = chain.head(&board, head)?;
        let snap = Snapshot::read(&chain, &[product], Some(&key))?;
        let b = snap
            .boards
            .first()
            .ok_or_else(|| Error::msg("the board did not read"))?;
        let (_, h) = b
            .accounts
            .heads
            .iter()
            .find(|(k, _)| *k == key)
            .ok_or_else(|| Error::msg(format!("{} has no head {head}.", product.symbol())))?;
        Ok(docs::head_detail_json(&snap, b, &key, h))
    })?;
    Ok(Report::new(doc.clone(), head_lines(p, &doc).join("\n")))
}

fn head_lines(p: Paint, h: &Value) -> Vec<String> {
    let mut out = vec![p.bold(&format!(
        "{} #{} {} · {}",
        text(&h["product"]),
        h["id"],
        text(&h["name"]),
        text(&h["status"])
    ))];
    out.push(format!(
        "  {} · {} · {} delegated by {} · {} strike{}",
        text(&h["componentLabel"]),
        text(&h["accessClass"]),
        amba(atoms_of(&h["delegation"])),
        h["delegators"],
        h["strikes"],
        if h["strikes"] == 1 { "" } else { "s" }
    ));
    if let Some(v) = h["value"].as_str().and_then(|v| v.parse().ok()) {
        out.push(format!(
            "  Now {} (published {})",
            value(v),
            h["sourceTs"].as_u64().map(at).unwrap_or_default()
        ));
    }
    if !text(&h["spec"]).is_empty() {
        out.push(format!(
            "  Spec: {}",
            crate::backend::terminal_safe_text(text(&h["spec"]))
        ));
    }
    for c in h["claims"].as_array().into_iter().flatten() {
        out.push(format!(
            "  claim {} {} {} by {} · {} · bond {} USDC{}",
            c["seq"],
            text(&c["kind"]),
            c["value"]
                .as_str()
                .and_then(|v| v.parse().ok())
                .map(value)
                .unwrap_or_else(|| "(sealed)".into()),
            short(text(&c["fetcher"])),
            text(&c["status"]),
            usdc(atoms_of(&c["bond"])),
            c["windowEndsAt"]
                .as_u64()
                .filter(|_| c["status"] == "open" || c["status"] == "challenged")
                .map(|t| format!(" · open to challenge until {}", at(t)))
                .unwrap_or_default()
        ));
    }
    for c in h["challenges"].as_array().into_iter().flatten() {
        out.push(format!(
            "  challenge of {} by {} · {} · bond {} USDC · {}",
            if c["target"] == "source" {
                "the source".to_string()
            } else {
                format!("claim {}", c["seq"])
            },
            short(text(&c["challenger"])),
            text(&c["reason"]),
            usdc(atoms_of(&c["bond"])),
            text(&c["status"])
        ));
    }
    out
}

// ------------------------------------------------------------------------------- heads mine

/// `petri oracle heads mine [--owner]`: from chain, with the follow-up commands.
pub fn mine(ctx: &Ctx, p: Paint, owner: Option<&str>) -> Result<Report> {
    let owner = ctx.owner(owner)?;
    let chain = Chain::reads(ctx)?;
    let m = Mine::read(&chain, &owner)?;
    let doc = docs::mine_json(&m);
    Ok(Report::new(doc.clone(), mine_lines(p, &doc).join("\n")))
}

fn mine_lines(p: Paint, d: &Value) -> Vec<String> {
    let mut out = vec![p.bold(&format!(
        "Oracle stake of {}: {} free",
        text(&d["owner"]),
        amba(atoms_of(&d["stake"]["free"]))
    ))];
    let row = |v: &Value| {
        format!(
            "{} #{} {}",
            text(&v["product"]),
            v["head"],
            text(&v["name"])
        )
    };
    for v in d["delegations"].as_array().into_iter().flatten() {
        let mut parts = vec![format!("delegated {}", amba(atoms_of(&v["amount"])))];
        if atoms_of(&v["cooling"]) > 0 {
            parts.push(format!(
                "{} cooling{}",
                amba(atoms_of(&v["cooling"])),
                v["cooldownEndsAt"]
                    .as_u64()
                    .map(|t| format!(" until {}", at(t)))
                    .unwrap_or_default()
            ));
        }
        if atoms_of(&v["escrow"]) > 0 {
            parts.push(format!("{} in escrow", amba(atoms_of(&v["escrow"]))));
        }
        if atoms_of(&v["rewards"]) > 0 {
            parts.push(format!("{} USDC rewards", usdc(atoms_of(&v["rewards"]))));
        }
        out.push(format!(
            "  {} ({}) · {}",
            row(v),
            text(&v["headStatus"]),
            parts.join(" · ")
        ));
        if let Some(c) = v["syncCommand"].as_str() {
            out.push(format!("      release: {c}"));
        }
    }
    for v in d["claims"].as_array().into_iter().flatten() {
        out.push(format!(
            "  claim {} on {} · {} · {}",
            v["seq"],
            row(v),
            text(&v["kind"]),
            text(&v["status"])
        ));
        if let Some(c) = v["prizeCommand"].as_str() {
            out.push(format!(
                "      prize {} USDC: {c}",
                usdc(atoms_of(&v["prize"]))
            ));
        }
    }
    for v in d["challenges"].as_array().into_iter().flatten() {
        out.push(format!(
            "  challenge of {} on {} · bond {} USDC · {}",
            if v["target"] == "source" {
                "the source".to_string()
            } else {
                format!("claim {}", v["seq"])
            },
            row(v),
            usdc(atoms_of(&v["bond"])),
            text(&v["status"])
        ));
    }
    let claimable = atoms_of(&d["rewards"]["claimable"]);
    if claimable > 0 {
        out.push(format!(
            "  Rewards to claim: {} USDC: {}",
            usdc(claimable),
            text(&d["rewards"]["claimCommand"])
        ));
    }
    if out.len() == 1 {
        out.push(
            "  No delegations, claims or challenges. Stake first: petri oracle heads stake <AMBA>"
                .to_string(),
        );
    }
    out
}

/// `petri oracle heads rewards [--owner]`: the delegator rewards to claim, per delegation.
pub fn rewards(ctx: &Ctx, p: Paint, owner: Option<&str>) -> Result<Report> {
    let owner = ctx.owner(owner)?;
    let chain = Chain::reads(ctx)?;
    let m = Mine::read(&chain, &owner)?;
    let doc = docs::mine_json(&m);
    let delegations: Vec<Value> = doc["delegations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| atoms_of(&d["rewards"]) > 0)
        .map(|d| json!({ "product": d["product"], "head": d["head"], "name": d["name"], "rewards": d["rewards"] }))
        .collect();
    let mut lines = vec![p.bold(&format!(
        "Delegator rewards of {owner}: {} USDC",
        usdc(atoms_of(&doc["rewards"]["claimable"]))
    ))];
    for d in &delegations {
        lines.push(format!(
            "  {} #{} {} · {} USDC",
            text(&d["product"]),
            d["head"],
            text(&d["name"]),
            usdc(atoms_of(&d["rewards"]))
        ));
    }
    if let Some(c) = doc["rewards"]["claimCommand"].as_str() {
        lines.push(format!("  Claim into your oracle collateral: {c}"));
    }
    let mut out = doc["rewards"].clone();
    out["owner"] = json!(owner.to_string());
    out["delegations"] = json!(delegations);
    Ok(Report::new(out, lines.join("\n")))
}

// ------------------------------------------------------------------------------------ terms

/// `RAMX Q4 2026`: the quarter a contract expiring at `expiry` is named after (the quarter that
/// ends at its expiry: Q4 2026 expires on 2027-01-01).
pub fn quarter_name(symbol: &str, expiry: u64) -> String {
    let (year, quarter) = quarter_of(expiry);
    format!("{symbol} Q{quarter} {year}")
}

/// The quarter holding a contract's last second (`commodity_spec::molds::Quarter`, whose mix it
/// lists with).
fn quarter_of(expiry: u64) -> (i32, u32) {
    let q =
        ameba_commodity_spec::molds::Quarter::of_expiry(i64::try_from(expiry).unwrap_or(i64::MAX));
    (q.year, u32::from(q.quarter))
}

/// A quarter as `heads terms` takes it: `Q1-2027`, `Q1 2027`, `2027Q1`, `2027-Q1`.
fn parse_quarter(text: &str) -> Option<(i32, u32)> {
    let t: String = text
        .to_ascii_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let (year, quarter) = if let Some(rest) = t.strip_prefix('Q') {
        (rest.get(1..)?, rest.get(..1)?)
    } else {
        let (y, q) = t.split_once('Q')?;
        (y, q)
    };
    let quarter: u32 = quarter.parse().ok().filter(|q| (1..=4).contains(q))?;
    Some((year.parse().ok()?, quarter))
}

/// `petri oracle heads terms <period|month|product>`: the listing terms document.
pub fn terms(ctx: &Ctx, p: Paint, target: &str) -> Result<Report> {
    let period = match target.trim().parse::<Pubkey>() {
        Ok(period) => period,
        Err(_) => contract_period(ctx, target)?,
    };
    let path = format!("{BOARDS}/contracts/{period}/terms");
    let (doc, _) = api_or_chain(ctx, &path, || {
        let chain = Chain::reads(ctx)?;
        docs::terms_json(&chain, &period)
    })?;
    // The listed document names its quarter and commits to that quarter's mix.
    let listed_doc: Value = text(&doc["document"])
        .split_once('\n')
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    let mut lines = vec![p.bold(&format!(
        "Listing terms of {} ({}, {})",
        quarter_name(text(&doc["product"]), doc["expiry"].as_u64().unwrap_or(0)),
        period,
        target_words(text(&doc["settlementTarget"]))
    ))];
    if let Some(c) = listed_doc["mixCommitment"].as_str() {
        lines.push(format!(
            "  The {} mix, committed as {c}:",
            listed_doc["quarter"].as_str().unwrap_or("quarter's")
        ));
    }
    lines.push(format!(
        "  Document SHA-256 {} · {}",
        text(&doc["documentSha256"]),
        if doc["matches"] == true {
            "matches what the contract listed"
        } else {
            "does NOT match what the contract listed"
        }
    ));
    // The listed items: basket weight and the channel mix the blend weighs them by.
    let listed: Value = text(&doc["document"])
        .split_once('\n')
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(Value::Null);
    if let Some(d) = listed["disclosure"].as_str() {
        lines.push(format!("  {}", crate::backend::terminal_safe_text(d)));
    }
    for item in listed["items"].as_array().into_iter().flatten() {
        let mix = &item["mixBps"];
        // `[spot, contract, lta]` in the document; an object in a board's items.
        let share = |c: &str| {
            let at = ["spot", "contract", "lta"]
                .iter()
                .position(|x| *x == c)
                .unwrap_or(0);
            mix.get(at)
                .or_else(|| mix.get(c))
                .and_then(Value::as_u64)
                .map(|m| format!("{}%", m as f64 / 100.0))
                .unwrap_or_else(|| "-".into())
        };
        lines.push(format!(
            "  {} · {}% of the basket · Spot {} · Contract {} · LTA {} ({})",
            text(&item["name"]),
            item["basketWeightBps"].as_u64().unwrap_or(0) as f64 / 100.0,
            share("spot"),
            share("contract"),
            share("lta"),
            text(&item["mixBasis"])
        ));
    }
    for s in doc["sentences"].as_array().into_iter().flatten() {
        lines.push(format!(
            "  {:>2} {} {}: {}",
            s["index"],
            text(&s["bucketId"]),
            text(&s["channel"]),
            crate::backend::terminal_safe_text(text(&s["sentence"]))
        ));
    }
    Ok(Report::new(doc, lines.join("\n")))
}

/// The period of a product's front contract (`RAMX`) or of a month (`RAMX-202701`).
fn contract_period(ctx: &Ctx, target: &str) -> Result<Pubkey> {
    let t = target.trim().to_ascii_uppercase();
    let (symbol, month) = match t.split_once(['-', ' ']) {
        Some((s, m)) => (s.to_string(), Some(m.trim().to_string())),
        None => (t.clone(), None),
    };
    let product = accounts::product(&symbol)?;
    let (board, _) = api_or_chain(ctx, &format!("{BOARDS}/{}", product.symbol()), || {
        let chain = Chain::reads(ctx)?;
        let snap = Snapshot::read(&chain, &[product], None)?;
        Ok(snap
            .boards
            .first()
            .map(|b| docs::board_json(&snap, b))
            .unwrap_or(Value::Null))
    })?;
    let contracts: Vec<&Value> = board["contracts"]
        .as_array()
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let found = match &month {
        Some(m) => {
            let wanted = parse_quarter(m).ok_or_else(|| {
                Error::msg(format!(
                    "`{m}` is not a quarter: name it like {} Q1 2027 (or RAMX-Q1-2027).",
                    product.symbol()
                ))
            })?;
            contracts
                .iter()
                .find(|c| quarter_of(c["expiry"].as_u64().unwrap_or(0)) == wanted)
        }
        None => board["index"]["contract"]
            .as_str()
            .and_then(|period| contracts.iter().find(|c| c["period"] == period))
            .or_else(|| contracts.iter().find(|c| c["status"] != "settled")),
    };
    let period = found.and_then(|c| c["period"].as_str()).ok_or_else(|| {
        Error::msg(match &month {
            Some(m) => format!("{} has no contract listed for {m}.", product.symbol()),
            None => format!(
                "{} has no contract listed yet: its terms are fixed when one lists.",
                product.symbol()
            ),
        })
    })?;
    period
        .parse()
        .map_err(|_| Error::msg(format!("`{period}` is not a period address")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/api/fixtures/oracle-heads.json"
        ))
        .expect("the API's heads fixture");
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn contracts_are_named_by_quarter() {
        // 2027-02-01 00:00 UTC ends January 2027.
        // 2027-01-01 00:00 UTC ends Q4 2026; 2027-04-01 ends Q1 2027.
        assert_eq!(quarter_name("RAMX", 1_798_761_600), "RAMX Q4 2026");
        assert_eq!(quarter_name("RAMX", 1_806_537_600), "RAMX Q1 2027");
        for text in ["Q1-2027", "Q1 2027", "2027Q1", "2027-Q1", "q1 2027"] {
            assert_eq!(parse_quarter(text), Some((2027, 1)), "{text}");
        }
        assert_eq!(parse_quarter("202701"), None);
        assert_eq!(parse_quarter("Q5 2027"), None);
    }

    /// The plain views read the API's documents (the fixture's boards and head) without panicking
    /// and say what matters.
    #[test]
    fn plain_views_read_the_api_documents() {
        let f = fixture();
        let p = Paint::new(false);
        for board in f["channels"]["heads"]["boards"].as_array().unwrap() {
            let lines = board_lines(p, board).join("\n");
            assert!(lines.contains("heads board"), "{lines}");
            assert!(lines.contains("Posting a head takes at least"));
        }
        let ramx = &f["channels"]["heads"]["boards"][0];
        let lines = board_lines(p, ramx).join("\n");
        assert!(
            lines.contains("DDR5 16Gb · 40% of RAMX · mix assumed"),
            "{lines}"
        );
        assert!(
            lines.contains("DDR5 16Gb Contract (50% of the mix)"),
            "{lines}"
        );
        assert!(
            lines.contains(&format!(
                "Index ({}, settles on each item's",
                ramx["contracts"][0]["label"].as_str().unwrap()
            )),
            "{lines}"
        );
        let head = &f["channels"]["head"];
        let lines = head_lines(p, head).join("\n");
        assert!(lines.contains(head["name"].as_str().unwrap()), "{lines}");
    }
}
