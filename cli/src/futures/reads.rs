//! Market reads: the market list, one market, its depth, trades and
//! chart, and a wallet's positions, history and balances.

use serde_json::{Value, json};
use solana_program::{program_pack::Pack, pubkey::Pubkey};

use crate::{chart, cli::ChartRangeValue};

use super::{
    Report, api,
    args::MarketArg,
    atoms,
    ctx::Ctx,
    error::{Error, Result},
    group, money, resolve,
    shape::{self, Grid},
    signed_money, slug, txbuild,
    units::{short, sol, usdc},
    view::{Paint, Table, clock, relative, utc},
};

/// An index value to the cent: `"102.0574"` → `"102.06"`.
pub fn price(text: &str) -> String {
    text.parse::<f64>()
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|_| text.to_string())
}

pub fn opt_price(text: Option<&str>) -> String {
    text.map(price).unwrap_or_else(|| "–".into())
}

/// A 24-hour change: `"+0.22"`, `"–"` without trades.
pub fn change(text: Option<&str>) -> String {
    match text.and_then(|t| t.parse::<f64>().ok()) {
        Some(v) => format!("{v:+.2}"),
        None => "–".into(),
    }
}

fn now(ctx: &Ctx, list: &[api::MarketView]) -> i64 {
    list.iter()
        .map(|m| m.chain_time)
        .find(|t| *t > 0)
        .or_else(|| ctx.rpc.clock().ok().map(|c| c.unix_timestamp))
        .unwrap_or_else(|| chrono::Utc::now().timestamp())
}

/// `open`, `paused` (open, but its oracle period is not tradable now) or
/// `settled at 104.20`.
pub fn status(m: &api::MarketView) -> String {
    match (m.status.as_str(), &m.settlement_value) {
        ("open", _) if m.tradable => "open".into(),
        ("open", _) => "paused".into(),
        (s, Some(v)) => format!("{s} at {}", price(v)),
        (s, None) => s.to_string(),
    }
}

/// The market's day of settlement: `"Nov 1"` (the expiry instant).
fn settles(expiry: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(expiry as i64, 0)
        .map(|t| t.format("%b %-d").to_string())
        .unwrap_or_default()
}

/// What a lot pays, from the markets' own `lotUsdPerPoint` (never a
/// hard-coded lot size: it follows each market's `k`).
fn lot_note<'a>(per_point: impl IntoIterator<Item = &'a str>) -> String {
    let mut values = per_point
        .into_iter()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    let pays = match values.as_slice() {
        [one] => format!("${one}"),
        _ => "a fixed amount (petri market shows it)".to_string(),
    };
    format!(
        "A lot pays {pays} per index point inside its range; a position never loses more than it cost."
    )
}

/// `petri markets`
pub fn markets(ctx: &Ctx, p: Paint) -> Result<Report> {
    let mut list = api::markets(&ctx.api)?;
    list.sort_by_key(|m| (m.status != "open", m.expiry_ts, m.symbol.clone()));
    let now = now(ctx, &list);
    let mut t = Table::new(&[
        "MARKET", "NAME", "PRICE", "24H", "INDEX", "VOL 24H", "SETTLES", "STATUS",
    ])
    .right(&[2, 3, 4, 5]);
    for m in &list {
        t.row(vec![
            m.label.clone(),
            slug(m),
            opt_price(m.last_entry_value.as_deref()),
            p.signed(&change(m.change24h_value.as_deref())),
            opt_price(m.index.as_ref().map(|i| i.value.as_str())),
            group(m.volume24h_lots),
            format!(
                "{} ({})",
                settles(m.expiry_ts),
                relative(m.expiry_ts as i64, now)
            ),
            status(m),
        ]);
    }
    let mut lines = vec![p.bold("Capped futures")];
    if t.is_empty() {
        lines.push("No markets yet.".into());
    } else {
        lines.extend(t.lines());
        lines.push(p.dim(&lot_note(list.iter().map(|m| m.lot_usd_per_point.as_str()))));
    }
    let first = list.first().map(slug).unwrap_or_else(|| "ramx".into());
    lines.push(p.dim(&format!(
        "Next: petri market {first} · petri book {first} · petri long {first} 100"
    )));
    Ok(Report::new(
        json!({ "ok": true, "command": "markets", "markets": list }),
        lines.join("\n"),
    ))
}

/// The best price to buy and sell a range, from one lot of depth.
fn best(
    ctx: &Ctx,
    m: &api::MarketView,
    shape: Option<(u16, u16)>,
) -> (Option<String>, Option<String>) {
    match api::depth(&ctx.api, &m.market, shape, 1) {
        Ok(d) => (
            d.asks.first().map(|l| price(&l.entry_value)),
            d.bids.first().map(|l| price(&l.entry_value)),
        ),
        Err(_) => (None, None),
    }
}

/// `petri market <market>`
pub fn market(ctx: &Ctx, p: Paint, arg: &MarketArg) -> Result<Report> {
    let m = resolve(ctx, &arg.market, arg.expiry.as_deref())?;
    let now = if m.chain_time > 0 {
        m.chain_time
    } else {
        now(ctx, &[])
    };
    let mut lines = vec![format!(
        "{} {} · {} · settles {} ({})",
        p.bold(&m.label),
        p.dim(&format!("({})", slug(&m))),
        status(&m),
        utc(m.expiry_ts as i64),
        relative(m.expiry_ts as i64, now)
    )];
    if let Some(v) = &m.settlement_value {
        lines.push(format!("  Settled     {}", price(v)));
    }
    lines.push(format!(
        "  Price       {} last trade · {} in 24h",
        opt_price(m.last_entry_value.as_deref()),
        p.signed(&change(m.change24h_value.as_deref()))
    ));
    if let Some(i) = &m.index {
        lines.push(format!(
            "  Index       {} · {}",
            price(&i.value),
            relative(i.ts, now)
        ));
    }
    let suggestion = m.suggestion.as_ref().map(|s| (s.lo, s.hi));
    if let Some(s) = &m.suggestion {
        lines.push(format!(
            "  Suggested   {}–{}, where the index can reach by expiry (2σ)",
            s.low_value, s.high_value
        ));
        if m.tradable {
            let (ask, bid) = best(ctx, &m, suggestion);
            lines.push(format!(
                "  Book        buy at {} · sell at {} on {}–{}",
                ask.unwrap_or_else(|| "–".into()),
                bid.unwrap_or_else(|| "–".into()),
                s.low_value,
                s.high_value
            ));
        }
    }
    lines.push(format!(
        "  Activity    {} lots in 24h · {} trades · {} open positions",
        group(m.volume24h_lots),
        group(m.trades),
        group(m.claims_open)
    ));
    lines.push(format!(
        "  Settlement  the {} index at expiry, from the oracle{}",
        m.symbol,
        m.period_state
            .as_ref()
            .map(|s| format!(" (period {})", s.status))
            .unwrap_or_default()
    ));
    lines.push(p.dim(&lot_note([m.lot_usd_per_point.as_str()])));
    let name = slug(&m);
    lines.push(p.dim(&format!(
        "Next: petri book {name} · petri chart {name} · petri long {name} 100"
    )));
    Ok(Report::new(
        json!({ "ok": true, "command": "market", "name": name, "market": m }),
        lines.join("\n"),
    ))
}

/// `petri book <market> [--range LO-HI]`
pub fn book(
    ctx: &Ctx,
    p: Paint,
    arg: &MarketArg,
    range: Option<&str>,
    levels: usize,
) -> Result<Report> {
    let m = resolve(ctx, &arg.market, arg.expiry.as_deref())?;
    let grid = Grid::of_view(&m)?;
    let shape = match range {
        Some(text) => {
            let (lo, hi) = shape::parse_range(text).map_err(Error::msg)?;
            let s = shape::explicit(&grid, &lo, &hi).map_err(Error::msg)?;
            Some((s.lo, s.hi))
        }
        None => None,
    };
    let d = api::depth(&ctx.api, &m.market, shape, 1_000_000_000)?;
    let mut lines = vec![format!(
        "{} · range {}–{}{}",
        p.bold(&m.label),
        d.lo_value,
        d.hi_value,
        if range.is_none() { " (suggested)" } else { "" }
    )];
    let mut t = Table::new(&["", "PRICE", "LOTS", "TOTAL"]).right(&[1, 2, 3]);
    let asks: Vec<&api::Level> = d.asks.iter().take(levels).collect();
    for l in asks.iter().rev() {
        t.row(vec![
            "sell".into(),
            p.red(&price(&l.entry_value)),
            group(l.lots),
            group(l.cumulative_lots),
        ]);
    }
    for l in d.bids.iter().take(levels) {
        t.row(vec![
            "buy".into(),
            p.green(&price(&l.entry_value)),
            group(l.lots),
            group(l.cumulative_lots),
        ]);
    }
    let mid = match (d.asks.first(), d.bids.first()) {
        (Some(a), Some(b)) => match (a.entry_value.parse::<f64>(), b.entry_value.parse::<f64>()) {
            (Ok(a), Ok(b)) => format!("{:.2} mid, {:.2} wide", (a + b) / 2.0, a - b),
            _ => String::new(),
        },
        _ => "one-sided".into(),
    };
    let index = m
        .index
        .as_ref()
        .map(|i| format!(" · index {}", price(&i.value)))
        .unwrap_or_default();
    let mut rows = t.lines().into_iter().map(|r| format!("  {r}"));
    lines.extend(rows.by_ref().take(1 + asks.len()));
    lines.push(p.dim(&format!("  ── {mid}{index} ──")));
    lines.extend(rows);
    if d.asks.is_empty() && d.bids.is_empty() {
        lines.push("  Nothing on the book for this range.".into());
    }
    lines.push(p.dim(
        "Sell rows are where you buy this range, buy rows where you sell it; other ranges price differently.",
    ));
    lines.push(p.dim(&format!(
        "Next: petri book {} --range 90-120 · petri quote long {} 100",
        slug(&m),
        slug(&m)
    )));
    Ok(Report::new(
        json!({ "ok": true, "command": "book", "market": m.market, "label": m.label, "depth": d }),
        lines.join("\n"),
    ))
}

/// `petri trades <market>`
pub fn trades(ctx: &Ctx, p: Paint, arg: &MarketArg, limit: u32) -> Result<Report> {
    let m = resolve(ctx, &arg.market, arg.expiry.as_deref())?;
    let list = api::trades(&ctx.api, &m.market, limit)?;
    let mut t = Table::new(&["TIME (UTC)", "SIDE", "LOTS", "PRICE", "RANGE"]).right(&[2, 3]);
    for x in &list {
        let side = if x.side == "sell" { "sell" } else { "buy" };
        t.row(vec![
            clock(x.ts),
            if side == "buy" {
                p.green(side)
            } else {
                p.red(side)
            },
            group(x.lots),
            price(&x.entry_value),
            format!("{}–{}", x.lo_value, x.hi_value),
        ]);
    }
    let mut lines = vec![p.bold(&format!("{} trades", m.label))];
    if t.is_empty() {
        lines.push("No trades yet.".into());
    } else {
        lines.extend(t.lines());
    }
    Ok(Report::new(
        json!({ "ok": true, "command": "trades", "market": m.market, "label": m.label, "trades": list }),
        lines.join("\n"),
    ))
}

/// `petri chart <market> [WINDOW]`
pub fn chart(
    ctx: &Ctx,
    p: Paint,
    arg: &MarketArg,
    window: ChartRangeValue,
    height: usize,
) -> Result<Report> {
    let m = resolve(ctx, &arg.market, arg.expiry.as_deref())?;
    let res = chart::resolution_for(window);
    let now = if m.chain_time > 0 {
        m.chain_time
    } else {
        now(ctx, &[])
    };
    let from = window.window_ms().map(|ms| now - (ms / 1_000) as i64);
    let candles = api::candles(&ctx.api, &m.market, res, from)?;
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(100)
        .saturating_sub(14);
    let points = chart::select_points(&candles, window, now, width);
    let mut lines = vec![format!(
        "{} · {} · {} candles",
        p.bold(&m.label),
        window.label(),
        chart::bucket_label(res)
    )];
    if points.is_empty() {
        lines.push(format!(
            "No trades on the price line in this window.{}",
            m.index
                .as_ref()
                .map(|i| format!(" Index now {}.", price(&i.value)))
                .unwrap_or_default()
        ));
    } else {
        let first = &points[0];
        let last = &points[points.len() - 1];
        let high = points.iter().map(|c| c.h).fold(f64::MIN, f64::max);
        let low = points.iter().map(|c| c.l).fold(f64::MAX, f64::min);
        lines.push(format!(
            "open {:.2}  high {:.2}  low {:.2}  last {:.2}  change {}{}",
            first.o,
            high,
            low,
            last.c,
            p.signed(&format!("{:+.2}", last.c - first.o)),
            m.index
                .as_ref()
                .map(|i| format!("  index {}", price(&i.value)))
                .unwrap_or_default()
        ));
        let closes: Vec<f64> = points.iter().map(|c| c.c).collect();
        lines.extend(chart::render_ascii_chart(&closes, height, width));
        lines.push(chart::volume_line(
            &points.iter().map(|c| c.v).collect::<Vec<_>>(),
            width,
        ));
        lines.push(p.dim(&format!("{} → {}", utc(first.t), utc(last.t))));
    }
    Ok(Report::new(
        json!({ "ok": true, "command": "chart", "market": m.market, "label": m.label,
                "window": window.label(), "resolution": res, "candles": points }),
        lines.join("\n"),
    ))
}

/// `petri positions [owner]`
pub fn positions(ctx: &Ctx, p: Paint, owner: Option<&str>) -> Result<Report> {
    let owner = ctx.owner(owner)?;
    let a = api::account(&ctx.api, &owner.to_string())?;
    let compressed = api::compressed_usdc(&ctx.api, &owner.to_string()).unwrap_or(0);
    let mut lines = vec![format!(
        "{} {} · wallet {} USDC{}",
        p.bold("Positions"),
        short(&owner.to_string()),
        money(atoms(&a.usdc)),
        if compressed > 0 {
            format!(" + {} compressed", money(i128::from(compressed)))
        } else {
            String::new()
        }
    )];
    let mut hints = Vec::new();
    let mut any = false;
    for am in &a.markets {
        let name = slug_of(&am.label, &am.symbol);
        let free = atoms(&am.trader.free);
        let reserved = atoms(&am.trader.reserved);
        if am.claims.is_empty() && am.orders.is_empty() && free == 0 && reserved == 0 {
            continue;
        }
        any = true;
        lines.push(String::new());
        lines.push(format!(
            "{} {} · {} · cash {} free{}",
            p.bold(&am.label),
            p.dim(&format!("({name})")),
            match (&am.status[..], &am.mark_value) {
                ("open", Some(v)) => format!("index {}", price(v)),
                (s, Some(v)) => format!("{s} at {}", price(v)),
                (s, None) => s.to_string(),
            },
            money(free),
            if reserved > 0 {
                format!(", {} in orders", money(reserved))
            } else {
                String::new()
            }
        ));
        if !am.claims.is_empty() {
            let mut t = Table::new(&[
                "#",
                "SIDE",
                "RANGE",
                "LOTS",
                "ENTRY",
                "PAID",
                "MAX",
                "P&L",
                "CLOSE NOW",
            ])
            .right(&[3, 4, 5, 6, 7, 8]);
            for c in &am.claims {
                t.row(vec![
                    c.slot.to_string(),
                    if c.side == "long" {
                        p.green("long")
                    } else {
                        p.red("short")
                    },
                    format!("{}–{}", c.lo_value, c.hi_value),
                    group(c.lots),
                    price(&c.entry_value),
                    money(atoms(&c.cost)),
                    money(atoms(&c.max_payout)),
                    c.pnl_at_index
                        .as_deref()
                        .map(|v| p.signed(&signed_money(atoms(v))))
                        .unwrap_or_else(|| "–".into()),
                    c.exit
                        .as_ref()
                        .map(|e| {
                            let mut s = money(atoms(&e.net));
                            if e.lots < c.lots {
                                s.push_str(&format!(" ({} lots)", group(e.lots)));
                            }
                            s
                        })
                        .unwrap_or_else(|| "–".into()),
                ]);
            }
            lines.extend(t.lines().into_iter().map(|l| format!("  {l}")));
            hints.push(format!("petri close {name} {}", am.claims[0].slot));
        }
        if !am.orders.is_empty() {
            lines.push("  Orders".into());
            let mut t =
                Table::new(&["#", "SIDE", "RANGE", "LOTS", "PRICE", "HOLDS"]).right(&[3, 4, 5]);
            for o in &am.orders {
                t.row(vec![
                    o.slot.to_string(),
                    format!("{}{}", o.side, if o.close { " close" } else { "" }),
                    format!("{}–{}", o.lo_value, o.hi_value),
                    group(o.lots),
                    price(&o.entry_value),
                    if o.close {
                        format!(
                            "position #{}",
                            o.claim.as_ref().map(|c| c.slot).unwrap_or_default()
                        )
                    } else {
                        money(atoms(&o.escrow))
                    },
                ]);
            }
            lines.extend(t.lines().into_iter().map(|l| format!("  {l}")));
            hints.push(format!("petri cancel {name} {}", am.orders[0].slot));
        }
        if free > 0 {
            hints.push(format!("petri withdraw {name} all"));
        }
    }
    if any {
        lines.push(p.dim(
            "P&L values each position at the index (the settlement once settled); CLOSE NOW is what selling it on the book returns.",
        ));
    } else {
        lines.push("No positions yet.".into());
        hints.push("petri long ramx 100".into());
        // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
        if cfg!(feature = "dev-faucet") && atoms(&a.usdc) == 0 {
            hints.insert(0, "petri faucet".into());
        }
    }
    hints.truncate(3);
    lines.push(p.dim(&format!("Next: {}", hints.join(" · "))));
    Ok(Report::new(
        json!({ "ok": true, "command": "positions", "account": a }),
        lines.join("\n"),
    ))
}

fn slug_of(label: &str, symbol: &str) -> String {
    slug(&api::MarketView {
        label: label.to_string(),
        symbol: symbol.to_string(),
        ..Default::default()
    })
}

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn num_at(v: &Value, key: &str) -> u64 {
    v.get(key)
        .and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

/// One history item as `(event, detail)`, or `None` for an item that only
/// repeats another (the taker's side of a fill repeats its take).
fn history_row(item: &Value) -> Option<(String, String)> {
    let d = item.get("data").unwrap_or(&Value::Null);
    let range = format!("{}–{}", str_at(d, "loValue"), str_at(d, "hiValue"));
    let role = item.get("role").and_then(Value::as_str);
    Some(match str_at(item, "type") {
        "take" => {
            let filled = num_at(d, "filled");
            if filled == 0 {
                return None;
            }
            (
                str_at(d, "side").to_string(),
                format!(
                    "{} lots · {range} at {} · {} {}",
                    group(filled),
                    price(str_at(d, "entryValue")),
                    if atoms(str_at(d, "paid")) >= 0 {
                        "paid"
                    } else {
                        "received"
                    },
                    money(atoms(str_at(d, "paid")).abs())
                ),
            )
        }
        "fill" | "ladderFill" if role == Some("taker") => return None,
        "fill" => (
            "filled".into(),
            format!(
                "your {} order: {} lots · {range} at {}",
                str_at(d, "side"),
                group(num_at(d, "lots")),
                price(str_at(d, "entryValue"))
            ),
        ),
        "ladderFill" => ("filled".into(), format!("your ladder on {range}")),
        "place" => (
            "order".into(),
            format!(
                "#{} {} {} lots · {range} at {}",
                num_at(d, "slot"),
                str_at(d, "side"),
                group(num_at(d, "lots")),
                price(str_at(d, "entryValue"))
            ),
        ),
        "cancel" => (
            "cancel".into(),
            format!(
                "order #{} · released {}",
                num_at(d, "slot"),
                money(atoms(str_at(d, "released")))
            ),
        ),
        "deposit" => ("deposit".into(), money(atoms(str_at(d, "amount")))),
        "withdraw" => ("withdraw".into(), money(atoms(str_at(d, "amount")))),
        "redeem" => (
            "netted".into(),
            format!(
                "{} lots · {range} · freed {}",
                group(num_at(d, "lots")),
                money(atoms(str_at(d, "cash")))
            ),
        ),
        "settle" => (
            "settled".into(),
            format!(
                "{} {} lots · {range} · paid out {}",
                str_at(d, "side"),
                group(num_at(d, "lots")),
                money(atoms(str_at(d, "payout")))
            ),
        ),
        "transfer" => (
            "transfer".into(),
            format!(
                "{} {} lots · {range} {}",
                str_at(d, "side"),
                group(num_at(d, "lots")),
                if role == Some("in") { "in" } else { "out" }
            ),
        ),
        other => (other.to_string(), String::new()),
    })
}

/// `petri history [owner]`
pub fn history(ctx: &Ctx, p: Paint, owner: Option<&str>, limit: u32) -> Result<Report> {
    let owner = ctx.owner(owner)?;
    let v = api::history(&ctx.api, &owner.to_string(), limit)?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut t = Table::new(&["TIME (UTC)", "MARKET", "EVENT", "DETAIL"]);
    for item in &items {
        if let Some((event, detail)) = history_row(item) {
            t.row(vec![
                clock(item.get("ts").and_then(Value::as_i64).unwrap_or(0)),
                str_at(item, "label").to_string(),
                event,
                detail,
            ]);
        }
    }
    let mut lines = vec![format!(
        "{} {}",
        p.bold("History"),
        short(&owner.to_string())
    )];
    if t.is_empty() {
        lines.push("Nothing yet.".into());
    } else {
        lines.extend(t.lines());
    }
    Ok(Report::new(
        json!({ "ok": true, "command": "history", "owner": owner.to_string(), "items": items }),
        lines.join("\n"),
    ))
}

/// `petri wallet [owner]`
pub fn wallet(ctx: &Ctx, p: Paint, owner: Option<&str>) -> Result<Report> {
    let mine = owner.is_none();
    let owner: Pubkey = ctx.owner(owner)?;
    let mint = ctx.deployment()?.usdc_mint;
    let token = txbuild::ata(&owner, &mint);
    let accounts = ctx.rpc.accounts(&[owner, token])?;
    let lamports = accounts[0].as_ref().map(|a| a.lamports).unwrap_or(0);
    let usdc_atoms = accounts[1]
        .as_ref()
        .and_then(|a| spl_token::state::Account::unpack(&a.data).ok())
        .map(|a| a.amount);
    let markets = api::account(&ctx.api, &owner.to_string())
        .map(|a| a.markets)
        .unwrap_or_default();
    // USDC left compressed by Amoeba's earlier product; orders spend it once
    // the token account runs short.
    let compressed = api::compressed_usdc(&ctx.api, &owner.to_string()).ok();
    let bonds = crate::oracle::chain::Oracle::new(ctx)
        .ok()
        .and_then(|o| o.collateral(&owner).ok().flatten());
    let mut lines = vec![format!("{} {owner}", p.bold("Wallet"))];
    if mine {
        lines.push(p.dim(&format!(
            "  from {} ({})",
            ctx.s.keypair.0,
            ctx.s.keypair.1.word()
        )));
    }
    lines.push(format!("  SOL            {}", sol(lamports)));
    lines.push(format!(
        "  USDC           ${}{}",
        usdc(usdc_atoms.unwrap_or(0)),
        if usdc_atoms.is_none() {
            " (no token account yet)"
        } else {
            ""
        }
    ));
    if let Some(atoms) = compressed.filter(|a| *a > 0) {
        lines.push(format!("  Compressed     ${} USDC", usdc(atoms)));
    }
    let mut cash = Vec::new();
    for am in &markets {
        let (free, reserved) = (atoms(&am.trader.free), atoms(&am.trader.reserved));
        if free > 0 || reserved > 0 {
            lines.push(format!(
                "  {:<14} {} free{}",
                am.label,
                money(free),
                if reserved > 0 {
                    format!(", {} in orders", money(reserved))
                } else {
                    String::new()
                }
            ));
            cash.push(json!({ "market": am.market, "label": am.label, "free": am.trader.free, "reserved": am.trader.reserved }));
        }
    }
    if let Some(b) = &bonds {
        lines.push(format!(
            "  Oracle bonds   ${} available, ${} locked",
            usdc(b.available_balance),
            usdc(b.position_locked_balance)
        ));
    }
    // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
    if cfg!(feature = "dev-faucet") && usdc_atoms.unwrap_or(0) == 0 && lamports == 0 {
        lines.push(p.dim("Next: petri faucet --sol 1"));
    }
    Ok(Report::new(
        json!({
            "ok": true, "command": "wallet", "owner": owner.to_string(),
            "keypair": mine.then(|| ctx.s.keypair.0.clone()),
            "lamports": lamports.to_string(), "usdcMint": mint.to_string(),
            "usdcTokenAccount": token.to_string(), "usdcAtoms": usdc_atoms.map(|a| a.to_string()),
            "compressedUsdcAtoms": compressed.map(|a| a.to_string()),
            "marketCash": cash,
            "oracleCollateral": bonds.as_ref().map(|b| json!({
                "availableAtoms": b.available_balance.to_string(),
                "lockedAtoms": b.position_locked_balance.to_string(),
            })),
        }),
        lines.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lot_note_follows_the_markets_lot_value() {
        assert!(lot_note(["0.01", "0.01"]).starts_with("A lot pays $0.01 per index point"));
        assert!(lot_note(["0.05"]).starts_with("A lot pays $0.05 per index point"));
        assert!(lot_note(["0.01", "0.05"]).starts_with("A lot pays a fixed amount"));
        assert!(lot_note([]).starts_with("A lot pays a fixed amount"));
    }

    #[test]
    fn small_formats() {
        assert_eq!(price("102.0574"), "102.06");
        assert_eq!(change(Some("0.2239")), "+0.22");
        assert_eq!(change(None), "–");
        assert_eq!(signed_money(1_500_000), "+$1.50");
        assert_eq!(signed_money(-1_500_000), "-$1.50");
        assert_eq!(settles(1_793_491_200), "Nov 1");
        assert_eq!(slug_of("RAMX Nov 26", "RAMX"), "ramx-nov26");
    }

    #[test]
    fn history_rows_read_the_documented_events() {
        let take = json!({ "type": "take", "role": null, "data": {
            "side": "short", "filled": 613, "loValue": "96", "hiValue": "135",
            "entryValue": "112.9845", "paid": "134955015" } });
        assert_eq!(
            history_row(&take).unwrap(),
            (
                "short".into(),
                "613 lots · 96–135 at 112.98 · paid $134.96".into()
            )
        );
        let echo = json!({ "type": "ladderFill", "role": "taker", "data": {} });
        assert!(history_row(&echo).is_none());
        let dep = json!({ "type": "deposit", "data": { "amount": "5000000" } });
        assert_eq!(history_row(&dep).unwrap().1, "$5.00");
    }
}
