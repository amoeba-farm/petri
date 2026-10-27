//! Orders: quote, open (long or short), close, cancel, deposit and
//! withdraw. Every order is quoted first and confirmed unless `--yes`.

use ameba_sdk_futures::futures::{MODE_GTC, MODE_IOC, MODE_POST, Side, SubmitArgs};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;
use solana_signer::Signer;

use super::{
    Chain, Funds, Report, api,
    args::{
        AmountArgs, CancelArgs, CloseArgs, OrderArgs, Pick, QuoteArgs, SideArg, parse_percent_bps,
    },
    atoms,
    check::{self, Intent, Scope},
    ctx::Ctx,
    error::{Error, Result, bail},
    evaluate,
    exec::{self, LOCAL_MIN_LAMPORTS, Outcome, Route},
    group, money, program, resolve,
    shape::{self, Grid, Shape},
    signed_money, slug, stale, submit_plan,
    txbuild::{self, Plan},
    units::{Amount, parse_amount, parse_usdc, short, usdc},
    view::Paint,
    worst_need,
};

pub fn side_of(s: SideArg) -> Side {
    match s {
        SideArg::Long => Side::Long,
        SideArg::Short => Side::Short,
    }
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Long => Side::Short,
        Side::Short => Side::Long,
    }
}

fn side_name(text: &str) -> Side {
    if text == "short" {
        Side::Short
    } else {
        Side::Long
    }
}

fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_IOC => "ioc",
        MODE_GTC => "gtc",
        _ => "post",
    }
}

/// An index value to the cent.
fn grid_price(text: &str) -> String {
    text.parse::<f64>()
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|_| text.to_string())
}

/// USDC a number of lots pays per index point.
fn per_point(grid: &Grid, lots: u64) -> i128 {
    let unit = 10u128.pow(u32::from(grid.decimals));
    (u128::from(lots) * u128::from(grid.k) * unit / u128::from(grid.step.max(1))) as i128
}

/// An order resolved against its market, before pricing.
pub struct Ticket {
    pub m: api::MarketView,
    pub grid: Grid,
    pub side: Side,
    pub shape: Shape,
    pub lots: u64,
    pub mode: u8,
    /// A resting order's limit premium per lot.
    pub limit: Option<u64>,
    /// The entry the range was built around (fractional position).
    pub anchor: f64,
    /// The leverage asked for, when it built the range.
    pub asked: Option<u32>,
    pub bps: u64,
}

impl Ticket {
    fn set(&self) -> u64 {
        self.grid.set(self.shape)
    }

    fn range(&self) -> String {
        format!(
            "{}–{}",
            self.grid.value(self.shape.lo),
            self.grid.value(self.shape.hi)
        )
    }

    fn leverage(&self) -> Option<f64> {
        shape::leverage_at(self.side, self.anchor, self.shape)
    }

    fn head(&self) -> String {
        let word = match self.side {
            Side::Long => "Long",
            Side::Short => "Short",
        };
        let at = self.leverage();
        let how = match (self.limit, self.asked, at) {
            (Some(p), _, _) => format!(
                "limit {}",
                self.grid.entry(u64::from(self.shape.lo) * self.grid.k + p)
            ),
            // Range ends are whole grid steps, so a high leverage can round
            // to a much higher one at this entry: say so.
            (None, Some(x), Some(l)) if (l - f64::from(x)).abs() > f64::from(x) * 0.1 => {
                format!("{x}x ({l:.0}x at this entry)")
            }
            (None, Some(x), _) => format!("{x}x"),
            (None, None, Some(l)) => format!("{l:.0}x"),
            (None, None, None) => "market".into(),
        };
        format!(
            "{word} {} {} · range {} · {how}",
            group(self.lots),
            self.m.label,
            self.range()
        )
    }
}

pub fn ticket(ctx: &Ctx, side: Side, a: &OrderArgs) -> Result<Ticket> {
    let m = resolve(ctx, &a.market.market, a.market.expiry.as_deref())?;
    let grid = Grid::of_view(&m)?;
    let bps = parse_percent_bps(&a.slippage).map_err(Error::msg)?;
    let limit_fine = a
        .limit
        .as_deref()
        .map(|t| grid.fine(t))
        .transpose()
        .map_err(Error::msg)?;
    let anchor = match limit_fine {
        Some(f) if f == 0 || f >= u64::from(grid.grid) * grid.k => bail(format!(
            "a limit must be between {} and {}",
            grid.value(0),
            grid.value(grid.grid)
        ))?,
        Some(f) => f as f64 / grid.k as f64,
        None => {
            let r = m.reference().ok_or_else(|| {
                Error::msg(format!(
                    "{} has no price yet; name one with --limit",
                    m.label
                ))
            })?;
            grid.pos_f64(&r).map_err(Error::msg)?
        }
    };
    let shape = match &a.range {
        Some(text) => {
            let (lo, hi) = shape::parse_range(text).map_err(Error::msg)?;
            shape::explicit(&grid, &lo, &hi).map_err(Error::msg)?
        }
        None => {
            let to =
                a.to.as_deref()
                    .map(|t| grid.end(t))
                    .transpose()
                    .map_err(Error::msg)?;
            let suggested = m.suggestion.as_ref().map(|s| match side {
                Side::Long => s.hi,
                Side::Short => s.lo,
            });
            shape::build(&grid, side, anchor, a.leverage, to, suggested).map_err(Error::msg)?
        }
    };
    let limit = limit_fine
        .map(|f| shape::premium_at(&grid, shape, f))
        .transpose()
        .map_err(Error::msg)?;
    let mode = match (limit, a.post) {
        (None, _) => MODE_IOC,
        (Some(_), true) => MODE_POST,
        (Some(_), false) => MODE_GTC,
    };
    let set = grid.set(shape);
    let lots = match (a.lots, &a.usd) {
        (Some(lots), _) => lots,
        (None, Some(usd)) => {
            let budget = parse_usdc(usd)?;
            let lots = match limit {
                Some(p) => budget / shape::outlay(side, p, set).max(1),
                None => {
                    let d = api::depth(
                        &ctx.api,
                        &m.market,
                        Some((shape.lo, shape.hi)),
                        1_000_000_000,
                    )?;
                    let book = match side {
                        Side::Long => &d.asks,
                        Side::Short => &d.bids,
                    };
                    let levels: Vec<shape::Level> = book
                        .iter()
                        .filter_map(|l| {
                            Some(shape::Level {
                                premium: l.premium.parse().ok()?,
                                lots: l.lots,
                            })
                        })
                        .collect();
                    shape::lots_for_budget(budget, &levels, side, set).0
                }
            };
            if lots == 0 {
                bail(format!(
                    "${usd} does not buy one lot of {}–{}; spend more or lower the leverage",
                    grid.value(shape.lo),
                    grid.value(shape.hi)
                ))?;
            }
            lots
        }
        (None, None) => bail("say how much: a number of lots, or --usd")?,
    };
    Ok(Ticket {
        m,
        grid,
        side,
        shape,
        lots,
        mode,
        limit,
        anchor,
        asked: a.range.is_none().then_some(a.leverage),
        bps,
    })
}

/// The ticket priced by the API (with the attached wallet when there is
/// one, so the quote knows its cash and netting).
fn price(ctx: &Ctx, t: &Ticket) -> Result<api::Quote> {
    let owner = ctx.wallet_pubkey().ok().map(|o| o.to_string());
    let body = api::quote_body(
        &t.m.market,
        owner.as_deref(),
        shape::side_word(t.side),
        t.shape.lo,
        t.shape.hi,
        t.lots,
        t.limit,
        mode_name(t.mode),
        None,
    );
    api::quote(&ctx.api, &body)
}

/// What to send for a priced ticket: a market order at most the quoted
/// fill, with the slippage limit; a limit order as typed.
fn order_args(t: &Ticket, q: &api::Quote) -> Result<SubmitArgs> {
    let set = t.set();
    match t.limit {
        Some(limit) => Ok(SubmitArgs {
            side: t.side,
            lo: t.shape.lo,
            hi: t.shape.hi,
            lots: t.lots,
            limit,
            min_fill: 0,
            mode: t.mode,
            close: None,
        }),
        None => {
            if q.filled == 0 {
                bail(format!(
                    "nothing to {} on {} {} right now",
                    if t.side == Side::Long {
                        "buy"
                    } else {
                        "sell into"
                    },
                    t.m.label,
                    t.range()
                ))?;
            }
            Ok(SubmitArgs {
                side: t.side,
                lo: t.shape.lo,
                hi: t.shape.hi,
                lots: t.lots.min(q.filled),
                limit: shape::open_limit(t.side, atoms(&q.premium), q.filled, set, t.bps),
                min_fill: 1,
                mode: MODE_IOC,
                close: None,
            })
        }
    }
}

fn quote_problem(q: &api::Quote) -> Option<String> {
    let e = q.error.as_deref()?;
    Some(match e {
        "NotTradable" => "the market is not trading now".into(),
        "NoLiquidity" => "nothing on the book fills this order now".into(),
        "WouldCross" => "a post-only order would fill now; leave out --post to trade".into(),
        // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
        "InsufficientFunds" if cfg!(feature = "dev-faucet") => {
            "not enough USDC in your wallet; run `petri faucet` (dev) or add some".into()
        }
        "InsufficientFunds" => "not enough USDC in your wallet; add some".into(),
        other => other.to_string(),
    })
}

fn review_lines(t: &Ticket, q: &api::Quote, args: &SubmitArgs, p: Paint) -> Vec<String> {
    let set = t.set();
    let symbol = &t.m.symbol;
    let mut lines = vec![p.bold(&t.head())];
    let (lo, hi) = (t.grid.value(t.shape.lo), t.grid.value(t.shape.hi));
    let average = q
        .average_entry_value
        .as_deref()
        .map(|v| t.grid.entry(t.grid.fine(v).unwrap_or_default()));
    let filled = q.filled.min(args.lots);
    match t.limit {
        None => {
            lines.push(format!(
                "  Entry       {} average{}",
                average.unwrap_or_else(|| "–".into()),
                if filled < t.lots {
                    format!(
                        " · only {} of {} lots fill now",
                        group(filled),
                        group(t.lots)
                    )
                } else {
                    String::new()
                }
            ));
        }
        Some(limit) => {
            let at = t.grid.entry(u64::from(t.shape.lo) * t.grid.k + limit);
            let rest = t.lots - q.filled.min(t.lots);
            lines.push(match (q.filled, rest) {
                (0, _) => {
                    format!("  Entry       {at}, resting on the book until the price gets there")
                }
                (_, 0) => format!(
                    "  Entry       {} average, all {} lots fill now",
                    average.clone().unwrap_or_else(|| at.clone()),
                    group(t.lots)
                ),
                (f, r) => format!(
                    "  Entry       {} fill now at {}; {} rest at {at}",
                    group(f),
                    average.clone().unwrap_or_else(|| "–".into()),
                    group(r)
                ),
            });
        }
    }
    lines.push(format!(
        "  Per point   {} from {lo} to {hi}",
        money(per_point(&t.grid, args.lots))
    ));
    let pay = match t.limit {
        None => atoms(&q.pay),
        Some(limit) => {
            atoms(&q.pay)
                + i128::from(t.lots - q.filled.min(t.lots))
                    * i128::from(shape::outlay(t.side, limit, set))
        }
    };
    lines.push(format!(
        "  You pay     {}, the most you can lose",
        money(pay.max(0))
    ));
    let redeemed = q.redeemed.as_deref().map(atoms).unwrap_or(0);
    if redeemed > 0 {
        lines.push(format!(
            "  Netting     closes your opposite position on this range and frees {}",
            money(redeemed)
        ));
    }
    lines.push(format!(
        "  Pays up to  {} if {symbol} settles {}",
        money(i128::from(set) * i128::from(args.lots)),
        match t.side {
            Side::Long => format!("at {hi} or above"),
            Side::Short => format!("at {lo} or below"),
        }
    ));
    let deposit = q.deposit.as_deref().map(atoms).unwrap_or(0);
    if deposit > 0 {
        lines.push(format!(
            "  Deposit     {} from your wallet into the market first",
            money(deposit)
        ));
    }
    lines
}

fn ticket_json(t: &Ticket, q: &api::Quote, args: Option<&SubmitArgs>) -> Value {
    json!({
        "market": t.m.market,
        "label": t.m.label,
        "name": slug(&t.m),
        "side": shape::side_word(t.side),
        "lo": t.shape.lo,
        "hi": t.shape.hi,
        "loValue": t.grid.value(t.shape.lo),
        "hiValue": t.grid.value(t.shape.hi),
        "leverage": t.leverage().map(|l| (l * 100.0).round() / 100.0),
        "lots": t.lots,
        "mode": mode_name(t.mode),
        "limit": args.map(|a| a.limit.to_string()),
        "sendLots": args.map(|a| a.lots),
        "slippageBps": t.bps,
        "quote": q,
    })
}

pub fn quote(ctx: &Ctx, p: Paint, a: &QuoteArgs) -> Result<Report> {
    let t = ticket(ctx, side_of(a.side), &a.order)?;
    let q = price(ctx, &t)?;
    let args = order_args(&t, &q).ok();
    let mut lines = match &args {
        Some(args) => review_lines(&t, &q, args, p),
        None => vec![p.bold(&t.head())],
    };
    if let Some(problem) = quote_problem(&q) {
        lines.push(p.red(&format!("  Can't send now: {problem}")));
    } else if args.is_none() {
        lines.push(p.red("  Nothing fills this order right now."));
    }
    let size = match a.order.lots {
        Some(lots) => lots.to_string(),
        None => format!("--usd {}", a.order.usd.clone().unwrap_or_default()),
    };
    lines.push(p.dim(&format!(
        "Send it: petri {} {} {size}{}",
        shape::side_word(t.side),
        slug(&t.m),
        flags(&a.order)
    )));
    Ok(Report::new(
        json!({ "ok": true, "command": "quote", "order": ticket_json(&t, &q, args.as_ref()) }),
        lines.join("\n"),
    ))
}

/// The order's own flags, to repeat them in a hint.
fn flags(a: &OrderArgs) -> String {
    let mut out = String::new();
    if let Some(r) = &a.range {
        out.push_str(&format!(" --range {r}"));
    } else {
        if a.leverage != shape::DEFAULT_LEVERAGE {
            out.push_str(&format!(" -x {}", a.leverage));
        }
        if let Some(to) = &a.to {
            out.push_str(&format!(" --to {to}"));
        }
    }
    if let Some(l) = &a.limit {
        out.push_str(&format!(" --limit {l}"));
    }
    if a.post {
        out.push_str(" --post");
    }
    out
}

/// Splits logs into `Program data` field lists emitted by `program` itself
/// (attributed to the program on top of the invocation stack).
fn program_data(logs: &[String], program: &Pubkey) -> Vec<Vec<Vec<u8>>> {
    use base64::{Engine, engine::general_purpose::STANDARD as B64};
    let program = program.to_string();
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in logs {
        let Some(rest) = line.strip_prefix("Program ") else {
            continue;
        };
        if let Some(data) = rest.strip_prefix("data: ") {
            if stack.last() == Some(&program) {
                out.push(
                    data.split_whitespace()
                        .filter_map(|f| B64.decode(f).ok())
                        .collect(),
                );
            }
            continue;
        }
        let mut words = rest.split_whitespace();
        let invoked = words.next().unwrap_or_default().to_string();
        match words.next() {
            Some("invoke") => stack.push(invoked),
            Some("success") | Some("failed:") => {
                stack.pop();
            }
            _ => {}
        }
    }
    out
}

/// What a landed `Submit` did, from its `ARTAKE01` event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Taken {
    pub filled: u64,
    pub premium: i128,
    pub paid: i128,
    pub rested: Option<u16>,
}

pub fn taken(outcome: &Outcome, program: &Pubkey) -> Option<Taken> {
    let fields = program_data(&outcome.logs, program)
        .into_iter()
        .find(|f| f.first().map(Vec::as_slice) == Some(b"ARTAKE01".as_slice()))?;
    let u64_at = |n: usize| {
        Some(u64::from_le_bytes(
            fields.get(n)?.as_slice().try_into().ok()?,
        ))
    };
    let i128_at = |n: usize| {
        Some(i128::from_le_bytes(
            fields.get(n)?.as_slice().try_into().ok()?,
        ))
    };
    let rested = u16::from_le_bytes(fields.get(10)?.as_slice().try_into().ok()?);
    Some(Taken {
        filled: u64_at(6)?,
        premium: i128_at(7)?,
        paid: i128_at(8)?,
        rested: (rested != u16::MAX).then_some(rested),
    })
}

/// `need`: the USDC the transaction takes from the wallet. When the wallet's
/// token account holds less, only the sponsored route can make up the rest
/// from compressed USDC (the API decompresses it), so `auto` takes it.
fn route(ctx: &Ctx, funds: &Funds, need: u64) -> Route {
    use super::args::RouteArg;
    match ctx.s.route {
        RouteArg::Local => Route::Local,
        RouteArg::Sponsored => Route::Sponsored,
        RouteArg::Auto if funds.sol >= LOCAL_MIN_LAMPORTS && need <= funds.usdc => Route::Local,
        RouteArg::Auto => Route::Sponsored,
    }
}

fn scope_check(
    ctx: &Ctx,
    chain: &Chain,
    owner: &Pubkey,
    funds: &Funds,
    intent: &Intent,
    tx: &solana_transaction::Transaction,
    sponsor: &Pubkey,
) -> Result<()> {
    let program = program(ctx)?;
    check::check(
        tx,
        &Scope {
            owner,
            sponsor,
            program: &program,
            market: &chain.market.market,
            owner_token: &funds.ata,
            mint: &chain.market.mint,
        },
        intent,
    )
}

/// Sends one `Submit`, solving it again once if the book moved before it
/// landed.
fn send_submit(
    ctx: &Ctx,
    signer: &dyn Signer,
    market: &Pubkey,
    args: SubmitArgs,
    withdraw: bool,
) -> Result<Outcome> {
    let owner = signer.pubkey();
    let mut attempt = 0;
    loop {
        let chain = Chain::load(ctx, market)?;
        let funds = Funds::load(ctx, &owner)?;
        let need = evaluate(&chain, &owner, args)
            .map(|e| e.deposit)
            .unwrap_or(0);
        let result = match route(ctx, &funds, need) {
            Route::Local => {
                let e = evaluate(&chain, &owner, args)?;
                if args.mode == MODE_POST && e.filled > 0 {
                    bail(
                        "WouldCross: a post-only order would fill now; leave out --post to trade",
                    )?;
                }
                if e.filled < args.min_fill {
                    bail(format!(
                        "MinFill: nothing fills inside the limit now ({} of {} lots); quote again",
                        e.filled, args.lots
                    ))?;
                }
                if e.deposit > funds.usdc {
                    bail(format!(
                        "InsufficientFunds: the order needs {} more USDC than the market holds for you; your wallet has {}",
                        usdc(e.deposit),
                        usdc(funds.usdc)
                    ))?;
                }
                exec::send_local(
                    ctx,
                    signer,
                    &submit_plan(&chain, &owner, &funds, &e, withdraw),
                )
            }
            Route::Sponsored => {
                let mut params = json!({
                    "market": market.to_string(),
                    "side": shape::side_word(args.side),
                    "lo": args.lo, "hi": args.hi, "lots": args.lots,
                    "limit": args.limit, "minFill": args.min_fill,
                    "mode": mode_name(args.mode), "withdraw": withdraw,
                });
                if let Some((slot, seq)) = args.close {
                    params["close"] = json!({ "slot": slot, "seq": seq.to_string() });
                }
                let intent = Intent::Submit {
                    args,
                    withdraw,
                    max_deposit: worst_need(&args, chain.grid.k),
                };
                exec::send_prepared(ctx, signer, "submit", params, |tx, sponsor, before| {
                    let intent = if before { &Intent::Decompress } else { &intent };
                    scope_check(ctx, &chain, &owner, &funds, intent, tx, sponsor)
                })
            }
        };
        match result {
            Err(e) if attempt == 0 && stale(&e.message) => attempt += 1,
            other => return other,
        }
    }
}

/// What a landed order did. `closing` is the side of the position a close
/// order closed: its proceeds are the premium (a long sold) or the released
/// sets less the premium (a short bought back), not the order's gross outlay.
fn sent_lines(
    p: Paint,
    grid: &Grid,
    shape: Shape,
    closing: Option<Side>,
    outcome: &Outcome,
    taken: Option<&Taken>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(t) = taken {
        if t.filled > 0 {
            let entry = u64::from(shape.lo) * grid.k + (t.premium.max(0) as u64) / t.filled;
            let entry = grid.entry(entry);
            let lots = group(t.filled);
            lines.push(p.green(&match closing {
                None => format!(
                    "Filled {lots} lots at {entry} average · {} {}",
                    if t.paid >= 0 { "paid" } else { "received" },
                    money(t.paid.abs())
                ),
                Some(side) => {
                    let sets = i128::from(grid.set(shape)) * i128::from(t.filled);
                    let got = match side {
                        Side::Long => t.premium,
                        Side::Short => sets - t.premium,
                    };
                    format!("Closed {lots} lots at {entry} average · got {}", money(got))
                }
            }));
        }
        if let Some(slot) = t.rested {
            lines.push(format!("Resting as order #{slot}"));
        }
    }
    for signature in &outcome.before {
        lines.push(p.dim(&format!(
            "{signature} · moved compressed USDC into your wallet"
        )));
    }
    lines.push(p.dim(&format!(
        "{} · {} route",
        outcome.signature,
        outcome.route.word()
    )));
    lines
}

fn taken_json(t: Option<&Taken>) -> Value {
    t.map(|t| {
        json!({
            "filled": t.filled, "premium": t.premium.to_string(),
            "paid": t.paid.to_string(), "restedOrder": t.rested,
        })
    })
    .unwrap_or(Value::Null)
}

/// An order priced by the API, with what to send for it.
pub struct Opening {
    pub ticket: Ticket,
    pub quote: api::Quote,
    pub args: SubmitArgs,
}

impl Opening {
    /// The review the trader confirms.
    pub fn review(&self, p: Paint) -> Vec<String> {
        review_lines(&self.ticket, &self.quote, &self.args, p)
    }

    /// What the landed order did.
    pub fn sent(&self, p: Paint, outcome: &Outcome, took: Option<&Taken>) -> Vec<String> {
        sent_lines(p, &self.ticket.grid, self.ticket.shape, None, outcome, took)
    }
}

/// Prices a ticket and fixes the order to send for it: refuses a market
/// that is not trading and a quote that cannot be sent.
pub fn priced(ctx: &Ctx, t: Ticket) -> Result<Opening> {
    if !t.m.tradable {
        bail(format!("{} is not trading now ({})", t.m.label, t.m.status))?;
    }
    let q = price(ctx, &t)?;
    if let Some(problem) = quote_problem(&q) {
        bail(format!("can't send: {problem}"))?;
    }
    let args = order_args(&t, &q)?;
    Ok(Opening {
        ticket: t,
        quote: q,
        args,
    })
}

/// A long or short order, priced (`petri long`, `petri short` and the TUI's
/// ticket).
pub fn prepare_open(ctx: &Ctx, side: Side, a: &OrderArgs) -> Result<Opening> {
    priced(ctx, ticket(ctx, side, a)?)
}

/// Sends a priced order (solving it again once if the book moved).
pub fn send_open(ctx: &Ctx, signer: &dyn Signer, o: &Opening) -> Result<(Outcome, Option<Taken>)> {
    let market: Pubkey = o
        .ticket
        .m
        .market
        .parse()
        .map_err(|_| Error::msg("bad market address"))?;
    let outcome = send_submit(ctx, signer, &market, o.args, false)?;
    let took = taken(&outcome, &program(ctx)?);
    Ok((outcome, took))
}

/// `petri long` / `petri short`.
pub fn open(ctx: &Ctx, p: Paint, side: SideArg, a: &OrderArgs) -> Result<Report> {
    let t = ticket(ctx, side_of(side), a)?;
    let signer = ctx.signer()?;
    let o = priced(ctx, t)?;
    if !ctx.s.json {
        eprintln!("{}", o.review(p).join("\n"));
    }
    exec::confirm(ctx, "Send this order?")?;
    let (outcome, took) = send_open(ctx, signer.as_ref(), &o)?;
    let mut lines = o.sent(p, &outcome, took.as_ref());
    lines.push(p.dim("Next: petri positions"));
    Ok(Report::new(
        json!({
            "ok": true, "command": shape::side_word(o.ticket.side),
            "order": ticket_json(&o.ticket, &o.quote, Some(&o.args)),
            "result": taken_json(took.as_ref()),
            "transaction": outcome.json(),
        }),
        lines.join("\n"),
    ))
}

fn my_market(ctx: &Ctx, owner: &Pubkey, m: &api::MarketView) -> Result<api::AccountMarket> {
    api::account(&ctx.api, &owner.to_string())?
        .markets
        .into_iter()
        .find(|x| x.market == m.market)
        .ok_or_else(|| Error::msg(format!("you have nothing in {}", m.label)))
}

/// One position's close, priced.
struct Closing {
    claim: api::ClaimView,
    args: SubmitArgs,
    quote: api::Quote,
    get: i128,
}

/// A close, priced: one `Submit` per position that can close now.
pub struct ClosePlan {
    pub m: api::MarketView,
    grid: Grid,
    closing: Vec<Closing>,
    keep_cash: bool,
    /// The review: the market, then one line per position (with why one
    /// cannot close now).
    pub lines: Vec<String>,
}

/// Prices closing `owner`'s positions (`petri close` and the TUI's close
/// ticket). Refuses when none of them can close now.
pub fn prepare_close(ctx: &Ctx, p: Paint, owner: &Pubkey, a: &CloseArgs) -> Result<ClosePlan> {
    prepare_close_where(ctx, p, owner, a, |_, _| true)
}

/// As [`prepare_close`], of the positions `a` names that `keep` keeps (a
/// bot's `close` with a range or side).
pub fn prepare_close_where(
    ctx: &Ctx,
    p: Paint,
    owner: &Pubkey,
    a: &CloseArgs,
    keep: impl Fn(&Grid, &api::ClaimView) -> bool,
) -> Result<ClosePlan> {
    let m = resolve(ctx, &a.market.market, a.market.expiry.as_deref())?;
    let grid = Grid::of_view(&m)?;
    let bps = parse_percent_bps(&a.slippage).map_err(Error::msg)?;
    let mine = my_market(ctx, owner, &m)?;
    let claims: Vec<api::ClaimView> = match a.position {
        Pick::One(n) => vec![
            mine.claims
                .iter()
                .find(|c| c.slot == n)
                .cloned()
                .ok_or_else(|| {
                    Error::msg(format!(
                        "no position #{n} in {}; see `petri positions`",
                        m.label
                    ))
                })?,
        ],
        Pick::All => {
            if a.lots.is_some() {
                bail("--lots closes part of one position; name it instead of `all`")?;
            }
            mine.claims.clone()
        }
    };
    let claims: Vec<api::ClaimView> = claims.into_iter().filter(|c| keep(&grid, c)).collect();
    if claims.is_empty() {
        bail(format!("you have no positions in {}", m.label))?;
    }
    let mut lines = vec![p.bold(&format!("Close in {}", m.label))];
    let mut closing: Vec<Closing> = Vec::new();
    for c in claims {
        let side = side_name(&c.side);
        let shape = Shape { lo: c.lo, hi: c.hi };
        let set = grid.set(shape);
        let free = c.lots.saturating_sub(c.reserved);
        let name = format!("#{} {} {}–{}", c.slot, c.side, c.lo_value, c.hi_value);
        if free == 0 {
            lines.push(format!(
                "  {name}: held by a resting close order; cancel it first"
            ));
            continue;
        }
        let lots = a.lots.unwrap_or(free);
        if lots > free {
            bail(format!("{name} has {free} lots to close"))?;
        }
        let owner_text = owner.to_string();
        let body = api::quote_body(
            &m.market,
            Some(&owner_text),
            shape::side_word(opposite(side)),
            c.lo,
            c.hi,
            lots,
            None,
            "ioc",
            Some((c.slot, &c.seq)),
        );
        let q = api::quote(&ctx.api, &body)?;
        if q.filled == 0 || q.error.is_some() {
            lines.push(format!(
                "  {name}: {}",
                quote_problem(&q)
                    .unwrap_or_else(|| "nothing on the book takes it right now".into())
            ));
            continue;
        }
        let total = atoms(&q.premium).max(0) as u128;
        let average = match side {
            Side::Long => total / u128::from(q.filled),
            Side::Short => total.div_ceil(u128::from(q.filled)),
        }
        .min(u128::from(set)) as u64;
        let seq: u64 = c
            .seq
            .parse()
            .map_err(|_| Error::msg("bad position sequence"))?;
        let args = SubmitArgs {
            side: opposite(side),
            lo: c.lo,
            hi: c.hi,
            lots: lots.min(q.filled),
            limit: shape::close_limit(side, average, set, bps),
            min_fill: 1,
            mode: MODE_IOC,
            close: Some((c.slot, seq)),
        };
        let get = -q
            .net_pay
            .as_deref()
            .map(atoms)
            .unwrap_or_else(|| match side {
                Side::Long => -atoms(&q.premium),
                Side::Short => atoms(&q.premium) - i128::from(set) * i128::from(q.filled),
            });
        let cost_share = atoms(&c.cost) * i128::from(args.lots) / i128::from(c.lots.max(1));
        lines.push(format!(
            "  {name}: {} {} lots at {} · you get {} · paid {} · P&L {}",
            if side == Side::Long {
                "sell"
            } else {
                "buy back"
            },
            group(args.lots),
            q.average_entry_value
                .as_deref()
                .map(|v| grid.entry(grid.fine(v).unwrap_or_default()))
                .unwrap_or_else(|| "–".into()),
            money(get),
            money(cost_share),
            p.signed(&signed_money(get - cost_share))
        ));
        closing.push(Closing {
            claim: c,
            args,
            quote: q,
            get,
        });
    }
    if closing.is_empty() {
        return Err(Error::msg(lines.join("\n")));
    }
    if !a.keep_cash {
        lines.push("  Then your cash in the market goes back to your wallet.".into());
    }
    Ok(ClosePlan {
        m,
        grid,
        closing,
        keep_cash: a.keep_cash,
        lines,
    })
}

impl ClosePlan {
    /// How many positions it closes.
    pub fn count(&self) -> usize {
        self.closing.len()
    }
}

/// Sends a priced close, one position at a time: the result lines and one
/// JSON entry per position. A failure before anything landed is an error;
/// a later one is reported in its position's entry.
pub fn send_close(
    ctx: &Ctx,
    p: Paint,
    signer: &dyn Signer,
    plan: &ClosePlan,
) -> Result<(Vec<String>, Vec<Value>)> {
    let market: Pubkey = plan
        .m
        .market
        .parse()
        .map_err(|_| Error::msg("bad market address"))?;
    let program = program(ctx)?;
    let mut out = Vec::new();
    let mut done = Vec::new();
    for c in &plan.closing {
        match send_submit(ctx, signer, &market, c.args, !plan.keep_cash) {
            Ok(outcome) => {
                let took = taken(&outcome, &program);
                out.push(format!("#{}:", c.claim.slot));
                out.extend(
                    sent_lines(
                        p,
                        &plan.grid,
                        Shape {
                            lo: c.claim.lo,
                            hi: c.claim.hi,
                        },
                        Some(side_name(&c.claim.side)),
                        &outcome,
                        took.as_ref(),
                    )
                    .into_iter()
                    .map(|l| format!("  {l}")),
                );
                done.push(json!({
                    "position": c.claim.slot, "lots": c.args.lots, "limit": c.args.limit.to_string(),
                    "expectedProceeds": c.get.to_string(), "quote": c.quote,
                    "result": taken_json(took.as_ref()), "transaction": outcome.json(),
                }));
            }
            Err(e) if done.is_empty() => return Err(e),
            Err(e) => {
                out.push(p.red(&format!("#{}: {}", c.claim.slot, e.message)));
                done.push(json!({ "position": c.claim.slot, "error": e.message }));
            }
        }
    }
    Ok((out, done))
}

/// `petri close`.
pub fn close(ctx: &Ctx, p: Paint, a: &CloseArgs) -> Result<Report> {
    let signer = ctx.signer()?;
    let plan = prepare_close(ctx, p, &signer.pubkey(), a)?;
    if !ctx.s.json {
        eprintln!("{}", plan.lines.join("\n"));
    }
    exec::confirm(
        ctx,
        if plan.count() == 1 {
            "Close it?"
        } else {
            "Close them?"
        },
    )?;
    let (mut out, done) = send_close(ctx, p, signer.as_ref(), &plan)?;
    out.push(p.dim("Next: petri positions"));
    Ok(Report::new(
        json!({ "ok": true, "command": "close", "market": plan.m.market, "label": plan.m.label, "closed": done }),
        out.join("\n"),
    ))
}

/// Resting orders to cancel, with the review.
pub struct CancelPlan {
    pub m: api::MarketView,
    orders: Vec<api::OrderView>,
    /// The review: the market, then one line per order.
    pub lines: Vec<String>,
}

/// Finds `owner`'s orders to cancel (`petri cancel` and the TUI's cancel
/// ticket).
pub fn prepare_cancel(ctx: &Ctx, p: Paint, owner: &Pubkey, a: &CancelArgs) -> Result<CancelPlan> {
    let m = resolve(ctx, &a.market.market, a.market.expiry.as_deref())?;
    let mine = my_market(ctx, owner, &m)?;
    let orders: Vec<api::OrderView> = match a.order {
        Pick::One(n) => vec![
            mine.orders
                .iter()
                .find(|o| u32::from(o.slot) == n)
                .cloned()
                .ok_or_else(|| {
                    Error::msg(format!(
                        "no order #{n} in {}; see `petri positions`",
                        m.label
                    ))
                })?,
        ],
        Pick::All => mine.orders.clone(),
    };
    if orders.is_empty() {
        bail(format!("you have no orders in {}", m.label))?;
    }
    let mut lines = vec![p.bold(&format!("Cancel in {}", m.label))];
    for o in &orders {
        lines.push(format!(
            "  #{} {} {} lots {}–{} at {}{}",
            o.slot,
            o.side,
            group(o.lots),
            o.lo_value,
            o.hi_value,
            grid_price(&o.entry_value),
            if o.close { " (close)" } else { "" }
        ));
    }
    Ok(CancelPlan { m, orders, lines })
}

impl CancelPlan {
    /// How many orders it cancels.
    pub fn count(&self) -> usize {
        self.orders.len()
    }
}

/// Cancels the planned orders one at a time: the result lines and one JSON
/// entry per order. A failure before anything landed is an error.
pub fn send_cancel(
    ctx: &Ctx,
    p: Paint,
    signer: &dyn Signer,
    plan: &CancelPlan,
) -> Result<(Vec<String>, Vec<Value>)> {
    let market: Pubkey = plan
        .m
        .market
        .parse()
        .map_err(|_| Error::msg("bad market address"))?;
    let mut out = Vec::new();
    let mut done = Vec::new();
    for o in &plan.orders {
        let seq: u64 = o
            .seq
            .parse()
            .map_err(|_| Error::msg("bad order sequence"))?;
        let result = send_simple(
            ctx,
            signer,
            &market,
            Intent::Cancel {
                slot: o.slot,
                seq,
                withdraw: false,
            },
        );
        match result {
            Ok(outcome) => {
                out.push(format!(
                    "Cancelled #{} · {}",
                    o.slot,
                    p.dim(&outcome.signature)
                ));
                done.push(json!({ "order": o.slot, "transaction": outcome.json() }));
            }
            Err(e) if done.is_empty() => return Err(e),
            Err(e) => {
                out.push(p.red(&format!("#{}: {}", o.slot, e.message)));
                done.push(json!({ "order": o.slot, "error": e.message }));
            }
        }
    }
    Ok((out, done))
}

/// `petri cancel`.
pub fn cancel(ctx: &Ctx, p: Paint, a: &CancelArgs) -> Result<Report> {
    let signer = ctx.signer()?;
    let plan = prepare_cancel(ctx, p, &signer.pubkey(), a)?;
    if !ctx.s.json {
        eprintln!("{}", plan.lines.join("\n"));
    }
    exec::confirm(
        ctx,
        if plan.count() == 1 {
            "Cancel it?"
        } else {
            "Cancel them?"
        },
    )?;
    let (out, done) = send_cancel(ctx, p, signer.as_ref(), &plan)?;
    Ok(Report::new(
        json!({ "ok": true, "command": "cancel", "market": plan.m.market, "label": plan.m.label, "cancelled": done }),
        out.join("\n"),
    ))
}

/// Deposit, withdraw or cancel: one step, local or sponsored.
fn send_simple(ctx: &Ctx, signer: &dyn Signer, market: &Pubkey, intent: Intent) -> Result<Outcome> {
    let owner = signer.pubkey();
    let chain = Chain::load(ctx, market)?;
    let funds = Funds::load(ctx, &owner)?;
    match route(ctx, &funds, 0) {
        Route::Local => {
            let mut plan = Plan::new();
            match &intent {
                Intent::Deposit { amount } => {
                    plan.push("deposit", chain.market.deposit(&owner, &funds.ata, *amount));
                }
                Intent::Withdraw { amount } => {
                    if !funds.exists {
                        plan.push(
                            "createAta",
                            txbuild::create_ata_idempotent(&owner, &owner, &chain.market.mint),
                        );
                    }
                    plan.push(
                        "withdraw",
                        chain.market.withdraw(&owner, &funds.ata, *amount),
                    );
                }
                Intent::Cancel { slot, seq, .. } => {
                    plan.push("cancel", chain.market.cancel(&owner, *slot, *seq));
                }
                Intent::Submit { .. } => unreachable!("orders go through send_submit"),
                Intent::Decompress => unreachable!("only the API prepares decompressions"),
            }
            exec::send_local(ctx, signer, &plan)
        }
        Route::Sponsored => {
            let m = market.to_string();
            let (action, params) = match &intent {
                Intent::Deposit { amount } => (
                    "deposit",
                    json!({ "market": m, "amount": amount.to_string() }),
                ),
                Intent::Withdraw { amount } => (
                    "withdraw",
                    json!({ "market": m, "amount": if *amount == u64::MAX { "all".to_string() } else { amount.to_string() } }),
                ),
                Intent::Cancel {
                    slot,
                    seq,
                    withdraw,
                } => (
                    "cancel",
                    json!({ "market": m, "slot": slot, "seq": seq.to_string(), "withdraw": withdraw }),
                ),
                Intent::Submit { .. } => unreachable!("orders go through send_submit"),
                Intent::Decompress => unreachable!("only the API prepares decompressions"),
            };
            exec::send_prepared(ctx, signer, action, params, |tx, sponsor, before| {
                let intent = if before { &Intent::Decompress } else { &intent };
                scope_check(ctx, &chain, &owner, &funds, intent, tx, sponsor)
            })
        }
    }
}

/// `petri deposit`.
pub fn deposit(ctx: &Ctx, p: Paint, a: &AmountArgs) -> Result<Report> {
    let signer = ctx.signer()?;
    let owner = signer.pubkey();
    let m = resolve(ctx, &a.market.market, a.market.expiry.as_deref())?;
    let amount = parse_usdc(&a.amount)?;
    if amount == 0 {
        bail("deposit more than 0")?;
    }
    let funds = Funds::load(ctx, &owner)?;
    if amount > funds.usdc {
        // A deposit moves only the token account's USDC; orders spend
        // compressed USDC themselves (the API decompresses it).
        let compressed = funds.compressed(ctx, &owner);
        if compressed > 0 {
            bail(format!(
                "your wallet has {} USDC, plus {} compressed that orders spend directly; deposit less, or place the order",
                usdc(funds.usdc),
                usdc(compressed)
            ))?;
        }
        bail(format!(
            // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
            "your wallet has {} USDC; {}deposit less",
            usdc(funds.usdc),
            if cfg!(feature = "dev-faucet") {
                "run `petri faucet` (dev) or "
            } else {
                ""
            }
        ))?;
    }
    if !ctx.s.json {
        eprintln!(
            "Deposit {} from your wallet into {}",
            money(i128::from(amount)),
            m.label
        );
    }
    exec::confirm(ctx, "Send?")?;
    let market: Pubkey = m
        .market
        .parse()
        .map_err(|_| Error::msg("bad market address"))?;
    let outcome = send_simple(ctx, signer.as_ref(), &market, Intent::Deposit { amount })?;
    Ok(Report::new(
        json!({ "ok": true, "command": "deposit", "market": m.market, "label": m.label,
                "amount": amount.to_string(), "transaction": outcome.json() }),
        format!(
            "Deposited {} into {} · {}",
            money(i128::from(amount)),
            m.label,
            p.dim(&outcome.signature)
        ),
    ))
}

/// `petri withdraw`.
pub fn withdraw(ctx: &Ctx, p: Paint, a: &AmountArgs) -> Result<Report> {
    let signer = ctx.signer()?;
    let owner = signer.pubkey();
    let m = resolve(ctx, &a.market.market, a.market.expiry.as_deref())?;
    let market: Pubkey = m
        .market
        .parse()
        .map_err(|_| Error::msg("bad market address"))?;
    let chain = Chain::load(ctx, &market)?;
    let free = chain
        .book
        .trader_of(&owner)
        .map(|(_, t)| t.free)
        .filter(|f| *f > 0)
        .ok_or_else(|| Error::msg(format!("you have no free cash in {}", m.label)))?;
    let amount = match parse_amount(&a.amount)? {
        Amount::All => u64::MAX,
        Amount::Atoms(0) => bail("withdraw more than 0")?,
        Amount::Atoms(n) if n > free => bail(format!(
            "you have ${} free in {}; withdraw that or less",
            usdc(free),
            m.label
        ))?,
        Amount::Atoms(n) => n,
    };
    let shown = money(i128::from(if amount == u64::MAX { free } else { amount }));
    if !ctx.s.json {
        eprintln!("Withdraw {shown} from {} to your wallet", m.label);
    }
    exec::confirm(ctx, "Send?")?;
    let outcome = send_simple(ctx, signer.as_ref(), &market, Intent::Withdraw { amount })?;
    Ok(Report::new(
        json!({ "ok": true, "command": "withdraw", "market": m.market, "label": m.label,
                "amount": if amount == u64::MAX { "all".to_string() } else { amount.to_string() },
                "free": free.to_string(), "transaction": outcome.json() }),
        format!(
            "Withdrew {shown} from {} to {} · {}",
            m.label,
            short(&owner.to_string()),
            p.dim(&outcome.signature)
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_points() {
        assert_eq!(money(14_310_200), "$14.31");
        assert_eq!(money(-5_000_000), "-$5.00");
        assert_eq!(money(134_955_015), "$134.96");
        assert_eq!(money(-1_000), "$0.00");
        assert_eq!(signed_money(-2_127_090_000), "-$2,127.09");
        assert_eq!(signed_money(1_000_000), "+$1.00");
        let dev = Grid {
            grid: 200,
            k: 10_000,
            step: 1_000_000,
            decimals: 6,
        };
        // 100 lots at $0.01 a point.
        assert_eq!(per_point(&dev, 100), 1_000_000);
        assert_eq!(group(2_709_347), "2,709,347");
    }

    #[test]
    fn take_events_are_read_from_the_futures_program_only() {
        use base64::{Engine, engine::general_purpose::STANDARD as B64};
        let futures = Pubkey::new_from_array([1; 32]);
        let other = Pubkey::new_from_array([2; 32]);
        let mut fields: Vec<Vec<u8>> = vec![b"ARTAKE01".to_vec()];
        for _ in 0..6 {
            fields.push(vec![0; 8]);
        }
        fields[6] = 7u64.to_le_bytes().to_vec();
        fields.push(1_234i128.to_le_bytes().to_vec());
        fields.push(1_500i128.to_le_bytes().to_vec());
        fields.push(vec![0; 8]);
        fields.push(3u16.to_le_bytes().to_vec());
        let line = |f: &[Vec<u8>]| {
            format!(
                "Program data: {}",
                f.iter()
                    .map(|x| B64.encode(x))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        let logs = vec![
            format!("Program {futures} invoke [1]"),
            format!("Program {other} invoke [2]"),
            line(&fields),
            format!("Program {other} success"),
            line(&fields),
            format!("Program {futures} success"),
        ];
        assert_eq!(program_data(&logs, &futures).len(), 1);
        let outcome = Outcome {
            signature: "sig".into(),
            route: Route::Local,
            steps: vec![],
            logs,
            slot: 1,
            before: vec![],
        };
        assert_eq!(
            taken(&outcome, &futures),
            Some(Taken {
                filled: 7,
                premium: 1_234,
                paid: 1_500,
                rested: Some(3),
            })
        );
    }
}
