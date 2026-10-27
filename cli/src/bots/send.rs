//! The network side of the wall: reading a market's feeds for the bots
//! there, and sending what they propose. Every send is checked against the
//! market read fresh from chain (the trusted program, its USDC and oracle:
//! `futures::Chain::load`), built with the futures client crate, signed by
//! the bot's own wallet and sent with the identity checks of every petri
//! send (`exec::send_local`). A bot's orders go through petri's own order
//! paths (`futures::trade`'s `prepare_open`/`send_open`, `prepare_close`/
//! `send_close`, `prepare_cancel`/`send_cancel`), so every check of `petri
//! long`, `close` and `cancel` applies to them.

use ameba_sdk_futures::futures::{
    Book, HEAP_FRAME, STATUS_OPEN, Side, heap_frame, payoff,
    program::processor::MAX_BOOK_LADDER_ARCS,
};
use solana_program::pubkey::Pubkey;
use solana_signer::Signer;

use super::{
    feeds::{BookRead, Closing, Feed, Holding, Index, Intent, Order, Outcome, Position, Sample},
    host::Feeds,
};
use crate::futures::{
    Chain, Funds, api,
    args::{CancelArgs, CloseArgs, MarketArg, OrderArgs, Pick},
    ctx::Ctx,
    error::Error,
    exec, money,
    shape::{self, Grid},
    trade,
    txbuild::{self, Plan},
    view::Paint,
};

fn err(e: Error) -> String {
    e.message.replace('\n', " · ")
}

fn address(market: &str) -> Result<Pubkey, String> {
    market
        .parse()
        .map_err(|_| format!("{market} is not a market address"))
}

/// `owner`'s ladder in `book`: its trader slot, and the ladder's escrow when
/// it is live.
fn ladder_of(
    book: &Book,
    owner: &Pubkey,
) -> (Option<ameba_sdk_futures::futures::Trader>, i64, Option<u64>) {
    let Some((t, trader)) = book.trader_of(owner) else {
        return (None, 0, None);
    };
    let ladder = book.makers().into_iter().find(|(_, mk, _)| mk.trader == t);
    let inventory = ladder
        .as_ref()
        .map_or(0, |(_, _, steps)| steps.iter().map(|(_, s)| s.inv).sum());
    let live = ladder
        .filter(|(_, mk, _)| mk.seq != 0)
        .map(|(_, mk, _)| mk.escrow);
    (Some(trader), inventory, live)
}

/// The grid a market account describes.
fn grid_of(book: &Book) -> Grid {
    let h = &book.header;
    Grid {
        grid: h.grid,
        k: h.k,
        step: h.step_value,
        decimals: h.value_decimals,
    }
}

/// Wall time, unix seconds (the clock of the API's index points).
pub fn wall() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The history keeps a day of samples, one a minute.
const DAY: i64 = 86_400;
const MINUTE: i64 = 60;

/// Adds `sample` to `history` if a minute has passed since its newest, and
/// drops what is older than a day.
pub fn record(history: &mut Vec<Sample>, sample: Sample) {
    if history.last().is_none_or(|s| sample.t >= s.t + MINUTE) {
        history.push(sample);
    }
    let since = sample.t - DAY;
    history.retain(|s| s.t >= since);
}

/// The API's index points of the last day before `now`, one a minute,
/// oldest first.
pub fn warm(points: &[api::IndexPoint], now: i64) -> Vec<Sample> {
    let mut sorted: Vec<&api::IndexPoint> = points.iter().collect();
    sorted.sort_by_key(|p| p.t);
    let mut history = Vec::new();
    for p in sorted {
        if p.t >= now - DAY && p.t <= now && p.value.is_finite() {
            record(&mut history, Sample { t: p.t, v: p.value });
        }
    }
    history
}

/// Reads `market`'s shared feeds: the API's market view (its index and
/// whether it trades), the market account from chain, and an index sample
/// for the history. Returns the chain's time and the account, for the
/// bots' own reads ([`read_bot`]).
pub fn read_market(ctx: &Ctx, market: &str, feeds: &Feeds) -> Result<(u64, Book), String> {
    let view = api::market(&ctx.api, market).map_err(err)?;
    let chain = Chain::load(ctx, &address(market)?).map_err(err)?;
    feeds.book.set(BookRead {
        book: chain.book.clone(),
        tradable: view.tradable,
    });
    if let Some(index) = &view.index {
        let grid = Grid::of_view(&view).map_err(err)?;
        let value: f64 = index.value.parse().map_err(|_| "the API sent no index")?;
        let step: f64 = view
            .step_value
            .parse()
            .map_err(|_| "the API sent no step")?;
        feeds.index.set(Index {
            label: crate::futures::slug(&view),
            symbol: view.symbol.clone(),
            centre: grid.pos_f64(&index.value)?,
            value,
            step,
            expiry_ts: view.expiry_ts,
            now: u64::try_from(view.chain_time).unwrap_or(0),
        });
        let mut history = feeds
            .history
            .get()
            .map(|h| (*h).clone())
            .unwrap_or_default();
        record(
            &mut history,
            Sample {
                t: wall(),
                v: value,
            },
        );
        feeds.history.set(history);
    }
    Ok((u64::try_from(view.chain_time).unwrap_or(0), chain.book))
}

/// `owner`'s wallet in `book`, with `usdc` in the wallet; its positions
/// valued at `index` (a grid position) when there is one.
pub fn position_of(book: &Book, owner: &Pubkey, usdc: u64, index: Option<f64>) -> Position {
    let (trader, inventory, live) = ladder_of(book, owner);
    let slot = book.trader_of(owner).map(|(t, _)| t);
    let grid = grid_of(book);
    let k = book.header.k;
    let at = index.map(|c| (c.max(0.0) * k as f64) as u64);
    let holdings = slot
        .map(|t| book.claims_of(t))
        .unwrap_or_default()
        .into_iter()
        .map(|(_, c)| {
            let side = Side::from_u8(c.side).unwrap_or(Side::Long);
            let per = at.map_or(0, |s| match side {
                Side::Long => payoff::long_per_lot(c.lo, c.hi, s, k),
                Side::Short => payoff::short_per_lot(c.lo, c.hi, s, k),
            });
            Holding {
                side,
                lo: c.lo,
                hi: c.hi,
                low: grid.value(c.lo),
                high: grid.value(c.hi),
                lots: c.lots,
                cost: c.cost,
                value: per.saturating_mul(c.lots),
            }
        })
        .collect();
    let orders = slot.map_or(0, |t| {
        book.orders().iter().filter(|(_, o)| o.trader == t).count() as u32
    });
    Position {
        owner: *owner,
        cash: trader.map_or(0, |t| t.free.saturating_add(t.reserved)),
        free: trader.map_or(0, |t| t.free),
        wallet: usdc,
        inventory,
        holdings,
        orders,
        elsewhere: Vec::new(),
        ladder: live.is_some(),
    }
}

/// Reads bot wallet `owner`'s own feed in the market `book` is: its USDC
/// from chain, its cash, ladder, positions and orders from `book`.
/// Its positions in other markets come with it (`elsewhere`).
pub fn read_bot(
    ctx: &Ctx,
    owner: &Pubkey,
    book: &Book,
    index: Option<&Index>,
    elsewhere: Vec<(String, Holding)>,
    feed: &Feed<Position>,
) -> Result<(), String> {
    let funds = Funds::load(ctx, owner).map_err(err)?;
    feed.set(Position {
        elsewhere,
        ..position_of(book, owner, funds.usdc, index.map(|i| i.centre))
    });
    Ok(())
}

/// An error from sending, as a person reads it.
fn plain(error: String) -> String {
    if error.contains("MakerCapacity") {
        "This market has no room for another ladder right now.".into()
    } else if error.contains("WouldCross") {
        "Its ladder would trade with the book at once; it tries again as prices move.".into()
    } else if error.contains("InsufficientFunds") || error.contains("insufficient funds") {
        "Its wallet doesn't have enough for this.".into()
    } else {
        crate::research::markets::plain(error)
    }
}

/// Sends `intent` for `market`, signed by the wallet `ctx` signs with (the
/// bot's own).
pub fn send(ctx: &Ctx, market: &str, intent: &Intent) -> Outcome {
    match try_send(ctx, market, intent) {
        Ok(outcome) => outcome,
        Err(e) => Outcome::Refused(plain(e)),
    }
}

fn market_arg(market: &str) -> MarketArg {
    MarketArg {
        market: market.to_string(),
        expiry: None,
    }
}

/// A landed order's result lines in one, without the signatures.
fn said(lines: &[String]) -> String {
    let kept: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.ends_with(" route") && !l.ends_with(':'))
        .collect();
    kept.join(" · ")
}

/// Whether `claim` is one `c` closes: of its range and side, when named.
pub fn closes(c: &Closing, grid: &Grid, claim: &api::ClaimView) -> bool {
    let side = c.side.is_none_or(|s| shape::side_word(s) == claim.side);
    let range = c.range.as_deref().is_none_or(|text| {
        shape::parse_range(text)
            .and_then(|(lo, hi)| Ok((grid.end(&lo)?, grid.end(&hi)?)))
            .is_ok_and(|(lo, hi)| lo == claim.lo && hi == claim.hi)
    });
    side && range
}

fn open(ctx: &Ctx, signer: &dyn Signer, market: &str, o: &Order) -> Result<Outcome, String> {
    let args = OrderArgs {
        market: market_arg(market),
        lots: o.lots,
        usd: o.usd.clone(),
        leverage: o.leverage,
        to: o.to.clone(),
        range: o.range.clone(),
        limit: o.limit.clone(),
        post: o.post,
        slippage: o.slippage.clone(),
    };
    let opening = trade::prepare_open(ctx, o.side, &args).map_err(err)?;
    let (outcome, took) = trade::send_open(ctx, signer, &opening).map_err(err)?;
    let word = if o.side == Side::Long {
        "Long"
    } else {
        "Short"
    };
    let lines = opening.sent(Paint::new(false), &outcome, took.as_ref());
    Ok(Outcome::Sent(match said(&lines) {
        s if s.is_empty() => format!("{word} sent"),
        s => format!("{word}: {s}"),
    }))
}

fn close(ctx: &Ctx, signer: &dyn Signer, market: &str, c: &Closing) -> Result<Outcome, String> {
    let p = Paint::new(false);
    let args = CloseArgs {
        market: market_arg(market),
        position: Pick::All,
        lots: None,
        slippage: c.slippage.clone().unwrap_or_else(|| "2".into()),
        keep_cash: false,
    };
    let plan = trade::prepare_close_where(ctx, p, &signer.pubkey(), &args, |grid, claim| {
        closes(c, grid, claim)
    })
    .map_err(err)?;
    let (lines, _) = trade::send_close(ctx, p, signer, &plan).map_err(err)?;
    Ok(Outcome::Sent(format!("Closed: {}", said(&lines))))
}

fn cancel_orders(ctx: &Ctx, signer: &dyn Signer, market: &str) -> Result<Outcome, String> {
    let p = Paint::new(false);
    let args = CancelArgs {
        market: market_arg(market),
        order: Pick::All,
    };
    let plan = match trade::prepare_cancel(ctx, p, &signer.pubkey(), &args) {
        Ok(plan) => plan,
        // Nothing resting: nothing to cancel.
        Err(e)
            if e.message.starts_with("you have no orders")
                || e.message.starts_with("you have nothing in") =>
        {
            return Ok(Outcome::Clear);
        }
        Err(e) => return Err(err(e)),
    };
    let (lines, _) = trade::send_cancel(ctx, p, signer, &plan).map_err(err)?;
    Ok(Outcome::Sent(said(&lines)))
}

fn try_send(ctx: &Ctx, market: &str, intent: &Intent) -> Result<Outcome, String> {
    let signer = ctx.signer().map_err(err)?;
    let (ladder, core, knots) = match intent {
        Intent::Open(o) => return open(ctx, signer.as_ref(), market, o),
        Intent::Close(c) => return close(ctx, signer.as_ref(), market, c),
        Intent::CancelOrders => return cancel_orders(ctx, signer.as_ref(), market),
        Intent::Cancel => return cancel_ladder(ctx, signer.as_ref(), market),
        Intent::Quote {
            ladder,
            core,
            knots,
            ..
        } => (ladder, core, knots),
    };
    let owner = signer.pubkey();
    let chain = Chain::load(ctx, &address(market)?).map_err(err)?;
    let b = &chain.book;
    let (trader, _, live) = ladder_of(b, &owner);
    // The program's count, before any cash moves.
    let arcs = b.quote_arcs(&owner, ladder, *core, knots);
    if arcs.is_none_or(|n| n > MAX_BOOK_LADDER_ARCS) {
        return Err("MakerCapacity".into());
    }
    // Post-only, against the market as it is now.
    let proposal = b
        .quote_proposal_core(&owner, ladder, *core, knots)
        .ok_or("WouldCross")?;
    let escrow = b
        .ladder_escrow(&owner, ladder, *core, knots)
        .ok_or("Its ladder is too large to fund.")?;
    let free = trader.map_or(0, |t| t.free);
    let need = escrow.saturating_sub(free.saturating_add(live.unwrap_or(0)));
    let funds = Funds::load(ctx, &owner).map_err(err)?;
    if funds.sol == 0 {
        return Err("Its wallet needs a little SOL to post a ladder.".into());
    }
    // The cash it needs beyond what it has in the market goes in first, on
    // its own: a ladder's transaction is near the size limit already.
    if need > 0 {
        if need > funds.usdc {
            return Err(format!(
                "Its ladder needs {} more USDC than its wallet has.",
                money(i128::from(need - funds.usdc))
            ));
        }
        let mut plan = Plan::new();
        plan.push("deposit", chain.market.deposit(&owner, &funds.ata, need));
        exec::send_local(ctx, signer.as_ref(), &plan).map_err(err)?;
    }
    let mut plan = Plan::new();
    plan.push("heapFrame", heap_frame(HEAP_FRAME));
    plan.push(
        "quote",
        chain
            .market
            .quote_core(&owner, ladder, *core, knots.clone(), proposal),
    );
    let sent = exec::send_local(ctx, signer.as_ref(), &plan).map_err(err)?;
    Ok(Outcome::Posted(sent.signature))
}

/// The ladder comes down and the cash it frees goes back to the wallet, as
/// `petri close` does unless asked to keep it.
fn cancel_ladder(ctx: &Ctx, signer: &dyn Signer, market: &str) -> Result<Outcome, String> {
    let owner = signer.pubkey();
    let chain = Chain::load(ctx, &address(market)?).map_err(err)?;
    let (trader, _, live) = ladder_of(&chain.book, &owner);
    if live.is_none() && trader.is_none_or(|t| t.free == 0) {
        return Ok(Outcome::Clear);
    }
    let funds = Funds::load(ctx, &owner).map_err(err)?;
    let mut plan = Plan::new();
    if live.is_some() {
        plan.push("cancelQuote", chain.market.cancel_quote(&owner));
    }
    if !funds.exists {
        plan.push(
            "createAta",
            txbuild::create_ata_idempotent(&owner, &owner, &chain.market.mint),
        );
    }
    plan.push(
        "withdraw",
        chain.market.withdraw(&owner, &funds.ata, u64::MAX),
    );
    let sent = exec::send_local(ctx, signer, &plan).map_err(err)?;
    Ok(Outcome::Cancelled(sent.signature))
}

/// A stopped bot's market sweep: its resting orders cancelled, then its
/// free cash withdrawn to its wallet (its ladder came down already). Its
/// positions stay.
pub fn sweep_market(ctx: &Ctx, signer: &dyn Signer, market: &str) -> Result<(), String> {
    if let Outcome::Refused(why) =
        cancel_orders(ctx, signer, market).unwrap_or_else(Outcome::Refused)
    {
        return Err(why);
    }
    // Whatever is free now (the ladder's cancel withdrew its own), with the
    // cash the cancelled orders freed.
    match cancel_ladder(ctx, signer, market) {
        Ok(_) => Ok(()),
        Err(e) => Err(plain(e)),
    }
}

/// A wallet's positions in one market of its API account.
pub fn holdings_in(m: &api::AccountMarket) -> Vec<Holding> {
    let atoms = |t: &str| t.parse::<u64>().unwrap_or(0);
    m.claims
        .iter()
        .map(|c| Holding {
            side: if c.side == "short" {
                Side::Short
            } else {
                Side::Long
            },
            lo: c.lo,
            hi: c.hi,
            low: c.lo_value.clone(),
            high: c.hi_value.clone(),
            lots: c.lots,
            cost: atoms(&c.cost),
            value: c.payout_at_index.as_deref().map_or(0, atoms),
        })
        .collect()
}

/// A wallet's positions in the markets of `account` other than `current`,
/// each with its market's label (a product bot's earlier months).
pub fn elsewhere(account: &api::Account, current: &str) -> Vec<(String, Holding)> {
    account
        .markets
        .iter()
        .filter(|m| m.market != current)
        .flat_map(|m| {
            let label = crate::futures::slug_of(&m.label, &m.symbol);
            holdings_in(m).into_iter().map(move |h| (label.clone(), h))
        })
        .collect()
}

/// Whether a market of `account` may hold cash to bring back: free cash,
/// or claims in a market no longer open.
fn to_tidy(m: &api::AccountMarket) -> bool {
    m.trader.free.parse::<u64>().unwrap_or(0) > 0 || (m.status != "open" && !m.claims.is_empty())
}

/// Brings bot wallet `signer`'s cash home from every market `account`
/// lists but the one it trades (`current`, while that is open): in a final
/// market its claims (and its ladder's inventory) are settled first (the
/// program lets anyone settle), then its free cash is withdrawn to its
/// wallet. Each market is read fresh from chain first; one the trusted
/// program does not own is left alone. Returns the first failure, after
/// trying them all.
pub fn tidy(
    ctx: &Ctx,
    signer: &dyn Signer,
    account: &api::Account,
    current: Option<&str>,
) -> Result<(), String> {
    let owner = signer.pubkey();
    let mut failed = None;
    for m in account.markets.iter().filter(|m| to_tidy(m)) {
        if Some(m.market.as_str()) == current && m.status == "open" {
            continue;
        }
        let Ok(address) = address(&m.market) else {
            continue;
        };
        let Ok(chain) = Chain::load(ctx, &address) else {
            continue;
        };
        let Some((t, _)) = chain.book.trader_of(&owner) else {
            continue;
        };
        let run = || -> Result<(), String> {
            if chain.book.header.status != STATUS_OPEN {
                let mut plan = Plan::new();
                let claims: Vec<u32> = chain.book.claims_of(t).iter().map(|(i, _)| *i).collect();
                if !claims.is_empty() {
                    plan.push("settle", chain.market.settle(claims));
                }
                for (i, mk, steps) in chain.book.makers() {
                    if mk.trader == t && steps.iter().any(|(_, s)| s.inv != 0) {
                        plan.push("settleMaker", chain.market.settle_maker(i));
                    }
                }
                if plan.names().len() > 1 {
                    exec::send_local(ctx, signer, &plan).map_err(err)?;
                }
            }
            withdraw_free(ctx, signer, &address).map_err(plain)
        };
        if let Err(e) = run() {
            failed.get_or_insert(format!("{}: {e}", m.label));
        }
    }
    failed.map_or(Ok(()), Err)
}

/// Withdraws all of `signer`'s free cash in `market` to its wallet, if it
/// has any.
fn withdraw_free(ctx: &Ctx, signer: &dyn Signer, market: &Pubkey) -> Result<(), String> {
    let owner = signer.pubkey();
    let chain = Chain::load(ctx, market).map_err(err)?;
    if chain
        .book
        .trader_of(&owner)
        .is_none_or(|(_, t)| t.free == 0)
    {
        return Ok(());
    }
    let funds = Funds::load(ctx, &owner).map_err(err)?;
    let mut plan = Plan::new();
    if !funds.exists {
        plan.push(
            "createAta",
            txbuild::create_ata_idempotent(&owner, &owner, &chain.market.mint),
        );
    }
    plan.push(
        "withdraw",
        chain.market.withdraw(&owner, &funds.ata, u64::MAX),
    );
    exec::send_local(ctx, signer, &plan).map_err(err)?;
    Ok(())
}
