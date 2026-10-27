//! Futures on the RAMX and NANDX indexes (`ameba-futures`,
//! `futures/docs/futures.md`), the product the site trades. One market per
//! index and expiry; a position is a claim on a range `[lo, hi)` of the
//! index, bought at an entry on the market's one price line, and it never
//! loses more than it put in.
//!
//! Reads and quotes come from the futures API (`api.rs`). Orders are solved
//! against the market account read from chain and sent by the owner (local
//! route), or prepared by the API and checked before signing (sponsored
//! route, `check.rs`). This module holds what the commands and the TUI's
//! Markets view share: the trusted program, market resolution, the market
//! read and checked from chain, order evaluation and plans.

pub mod api;
pub mod args;
pub mod check;
pub mod ctx;
pub mod error;
pub mod exec;
// DEV-ONLY: REMOVE FOR PROD. `petri faucet`, dev builds only.
#[cfg(feature = "dev-faucet")]
pub mod funds;
pub mod governance;
pub mod light;
pub mod market;
pub mod ops;
pub mod reads;
pub mod rpc;
pub mod settings;
pub mod shape;
pub mod trade;
pub mod txbuild;
pub mod units;
pub mod view;

use ameba_sdk_futures::futures::{
    Book, HEAP_FRAME, MODE_IOC, Market, Side, SubmitArgs, Taker, heap_frame, payoff,
};
use serde_json::Value;
use solana_program::{program_pack::Pack, pubkey::Pubkey};

use crate::{backend::CliError, cli::Cli};

use self::{
    ctx::Ctx,
    error::{Error, Result, bail},
    market::MarketRef,
    shape::Grid,
    txbuild::Plan,
    units::parse_decimal,
};

/// A command's JSON payload and its plain-text rendering.
#[derive(Clone, Debug)]
pub struct Report {
    pub payload: Value,
    pub plain: String,
}

impl Report {
    pub fn new(payload: Value, plain: String) -> Self {
        Self { payload, plain }
    }
}

/// Builds the futures context from the CLI and runs one command. Program
/// logs from a failed transaction go to stderr in plain mode.
pub fn run(
    cli: &Cli,
    command: impl FnOnce(&ctx::Ctx, view::Paint) -> error::Result<Report>,
) -> Result<Report, CliError> {
    let settings = settings::Settings::from_cli(cli).map_err(CliError::new)?;
    let json = settings.json;
    let paint = view::Paint::new(settings.color && !json);
    let ctx = ctx::Ctx::new(settings);
    command(&ctx, paint).map_err(|e| to_cli_error(e, json))
}

pub fn to_cli_error(e: error::Error, json: bool) -> CliError {
    if !json {
        for line in &e.logs {
            eprintln!("  log: {line}");
        }
    }
    let message = e.message.replace("\n  ", " | ").replace('\n', " | ");
    let (code, category, retryable) = if message.contains("unreachable") {
        ("FUTURES_SERVICE_UNAVAILABLE", "unavailable", true)
    } else if message.contains(" failed: ") || message.contains("signature ") {
        ("FUTURES_TRANSACTION_FAILED", "execution_failed", false)
    } else if message.starts_with("refusing") {
        ("FUTURES_REFUSED", "denied", false)
    } else {
        ("FUTURES_INVALID", "validation", false)
    };
    CliError::coded(code, category, message, retryable)
}

/// The trusted futures program.
pub fn program(ctx: &Ctx) -> Result<Pubkey> {
    let dep = ctx.deployment()?;
    dep.futures.ok_or_else(|| {
        Error::msg(format!(
            "the deployment ({}) has no futures program",
            dep.source
        ))
    })
}

impl Grid {
    /// The grid an API market view describes (`stepValue` is index points).
    pub fn of_view(m: &api::MarketView) -> Result<Self> {
        let k =
            m.k.parse::<u64>()
                .ok()
                .filter(|k| *k > 0)
                .ok_or_else(|| Error::msg(format!("{}: the API sent no lot size", m.label)))?;
        let step = parse_decimal(&m.step_value, u32::from(m.value_decimals))
            .ok()
            .and_then(|s| u64::try_from(s).ok())
            .filter(|s| *s > 0)
            .ok_or_else(|| Error::msg(format!("{}: the API sent no grid step", m.label)))?;
        Ok(Self {
            grid: m.grid,
            k,
            step,
            decimals: m.value_decimals,
        })
    }
}

/// Every market, as the API lists them (cached per command).
pub fn markets(ctx: &Ctx) -> Result<&[api::MarketView]> {
    if let Some(list) = ctx.markets.get() {
        return Ok(list);
    }
    let list = api::markets(&ctx.api)?;
    Ok(ctx.markets.get_or_init(|| list))
}

fn reference(m: &api::MarketView) -> Option<MarketRef> {
    Some(MarketRef {
        address: m.market.parse().ok()?,
        symbol: m.symbol.clone(),
        expiry_ts: m.expiry_ts,
        label: m.label.clone(),
        status: m.status.clone(),
    })
}

/// A market by symbol (`ramx`: the open one expiring first), label (`"RAMX
/// Nov 26"`, `ramx-nov26`) or address.
pub fn resolve(ctx: &Ctx, query: &str, expiry: Option<&str>) -> Result<api::MarketView> {
    let list = markets(ctx)?;
    let refs: Vec<MarketRef> = list.iter().filter_map(reference).collect();
    let found = market::resolve(query, expiry, &refs)?;
    let address = found.address.to_string();
    // The list is a few seconds old at most; read the one market fresh.
    api::market(&ctx.api, &address)
}

/// The command-line name of a market: `ramx-nov26`.
pub fn slug(m: &api::MarketView) -> String {
    slug_of(&m.label, &m.symbol)
}

/// [`slug`] of a market's label and symbol.
pub fn slug_of(label: &str, symbol: &str) -> String {
    let mut parts = label.split_whitespace();
    let symbol = parts.next().unwrap_or(symbol).to_ascii_lowercase();
    let rest: String = parts.collect::<String>().to_ascii_lowercase();
    if rest.is_empty() {
        symbol
    } else {
        format!("{symbol}-{rest}")
    }
}

/// A market read from chain and checked against the trusted deployment.
pub struct Chain {
    pub market: Market,
    pub book: Book,
    pub grid: Grid,
}

impl Chain {
    pub fn load(ctx: &Ctx, address: &Pubkey) -> Result<Self> {
        let dep = ctx.deployment()?;
        let program = program(ctx)?;
        let account = ctx
            .rpc
            .account(address)?
            .ok_or_else(|| Error::msg(format!("{address} has no market account on chain")))?;
        if account.owner != program {
            bail(format!(
                "refusing: {address} is not a market of the trusted futures program {program}"
            ))?;
        }
        let book = Book::decode(&account.data).map_err(|e| {
            Error::msg(format!(
                "{address} does not decode as a futures market: {e}"
            ))
        })?;
        let h = book.header;
        if h.quote_mint != dep.usdc_mint {
            bail(format!(
                "refusing: {address} settles in {} , not the deployment's USDC {}",
                h.quote_mint, dep.usdc_mint
            ))?;
        }
        if h.oracle_program != dep.oracle {
            bail(format!(
                "refusing: {address} reads oracle program {}, not the deployment's {}",
                h.oracle_program, dep.oracle
            ))?;
        }
        let market = Market::derive(
            program,
            *address,
            h.vault,
            h.period,
            h.quote_mint,
            &h.product_id,
            h.expiry_ts,
        );
        Ok(Self {
            market,
            grid: Grid {
                grid: h.grid,
                k: h.k,
                step: h.step_value,
                decimals: h.value_decimals,
            },
            book,
        })
    }
}

/// The owner's USDC token account and SOL.
pub struct Funds {
    pub ata: Pubkey,
    pub exists: bool,
    pub usdc: u64,
    pub sol: u64,
}

impl Funds {
    /// The owner's compressed USDC from the API, in atoms; 0 when it cannot
    /// be read (the token account's USDC is then all there is to spend).
    pub fn compressed(&self, ctx: &Ctx, owner: &Pubkey) -> u64 {
        api::compressed_usdc(&ctx.api, &owner.to_string()).unwrap_or(0)
    }

    pub fn load(ctx: &Ctx, owner: &Pubkey) -> Result<Self> {
        let ata = txbuild::ata(owner, &ctx.deployment()?.usdc_mint);
        let accounts = ctx.rpc.accounts(&[*owner, ata])?;
        Ok(Self {
            ata,
            exists: accounts[1].is_some(),
            usdc: accounts[1]
                .as_ref()
                .and_then(|a| spl_token::state::Account::unpack(&a.data).ok())
                .map(|a| a.amount)
                .unwrap_or(0),
            sol: accounts[0].as_ref().map(|a| a.lamports).unwrap_or(0),
        })
    }
}

/// An order priced on the book read from chain, with the cash it needs (the
/// program's rules; the API's `evaluate` mirrors them).
#[derive(Clone, Debug)]
pub struct Eval {
    pub args: SubmitArgs,
    pub filled: u64,
    /// The deposit needed beyond the owner's free cash in the market: what
    /// the fill pays, less the sets its netting releases, plus the escrow of
    /// a resting remainder.
    pub deposit: u64,
    pub proposal: Vec<u8>,
}

pub fn evaluate(chain: &Chain, owner: &Pubkey, args: SubmitArgs) -> Result<Eval> {
    let taker = Taker {
        side: args.side,
        lo: args.lo,
        hi: args.hi,
        limit: args.limit,
        qty: args.lots,
    };
    let proposed = chain.book.propose(&taker);
    let filled = proposed.solution.taker_fill;
    let premium = proposed.solution.premium;
    let set = u64::from(args.hi - args.lo) * chain.grid.k;
    let pay = match args.side {
        Side::Long => premium,
        Side::Short => i128::from(set) * i128::from(filled) - premium,
    };
    let trader = chain.book.trader_of(owner);
    let netted = trader
        .map(|(t, _)| {
            chain
                .book
                .claims_of(t)
                .iter()
                .filter(|(_, c)| c.side != args.side as u8 && c.lo == args.lo && c.hi == args.hi)
                .map(|(_, c)| c.lots.saturating_sub(c.reserved))
                .fold(0u64, u64::saturating_add)
        })
        .unwrap_or(0)
        .min(filled);
    let redeemed = u128::from(netted) * u128::from(set);
    let left = if args.mode == MODE_IOC {
        0
    } else {
        args.lots - filled.min(args.lots)
    };
    let escrow = if args.close.is_some() || left == 0 {
        0
    } else {
        u128::from(left)
            * u128::from(payoff::contribution(
                args.side,
                args.lo,
                args.hi,
                args.limit,
                chain.grid.k,
            ))
    };
    let free = trader.map(|(_, t)| t.free).unwrap_or(0);
    let need = (pay - redeemed as i128 + escrow as i128 - i128::from(free)).max(0);
    let deposit = u64::try_from(need).map_err(|_| Error::msg("the order is too large to fund"))?;
    Ok(Eval {
        args,
        filled,
        deposit,
        proposal: proposed.proposal,
    })
}

/// `[computeBudget, heapFrame, deposit?, submit, createAta?, withdraw?]`.
pub fn submit_plan(chain: &Chain, owner: &Pubkey, funds: &Funds, e: &Eval, withdraw: bool) -> Plan {
    let mut plan = Plan::new();
    plan.push("heapFrame", heap_frame(HEAP_FRAME));
    if e.deposit > 0 {
        plan.push(
            "deposit",
            chain.market.deposit(owner, &funds.ata, e.deposit),
        );
    }
    plan.push(
        "submit",
        chain.market.submit(owner, &e.args, e.proposal.clone()),
    );
    if withdraw {
        if !funds.exists {
            plan.push(
                "createAta",
                txbuild::create_ata_idempotent(owner, owner, &chain.market.mint),
            );
        }
        plan.push(
            "withdraw",
            chain.market.withdraw(owner, &funds.ata, u64::MAX),
        );
    }
    plan
}

/// The worst cash an order can need at its own limit: the bound a
/// sponsor-prepared deposit is checked against.
pub fn worst_need(args: &SubmitArgs, k: u64) -> u128 {
    u128::from(args.lots)
        * u128::from(payoff::contribution(
            args.side, args.lo, args.hi, args.limit, k,
        ))
}

/// Errors from a stale proposal: the book moved between solving and landing.
pub fn stale(message: &str) -> bool {
    [
        "BadProposal",
        "NotBestPrice",
        "NotMaxFill",
        "FifoViolation",
        "Unbalanced",
    ]
    .iter()
    .any(|name| message.contains(name))
}

/// Atoms from an API string.
pub fn atoms(text: &str) -> i128 {
    text.trim().parse().unwrap_or(0)
}

/// USDC atoms as dollars to the cent: `"$14.31"`, `"-$5.00"`.
pub fn money(atoms: i128) -> String {
    let cents = (atoms.unsigned_abs() + 5_000) / 10_000;
    let body = format!("{}.{:02}", group((cents / 100) as u64), cents % 100);
    if atoms < 0 && cents > 0 {
        format!("-${body}")
    } else {
        format!("${body}")
    }
}

/// `money` with a `+` on gains.
pub fn signed_money(atoms: i128) -> String {
    if atoms >= 5_000 {
        format!("+{}", money(atoms))
    } else {
        money(atoms)
    }
}

/// `2709347` → `"2,709,347"`.
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
