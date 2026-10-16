//! Command grammar for the futures commands. The top-level `Command` enum in
//! `src/cli.rs` embeds these.

use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum RouteArg {
    /// Local when the wallet has SOL for fees, else sponsored
    #[default]
    Auto,
    /// Build locally and send through RPC; the wallet pays the fee
    Local,
    /// Prepared by the futures API, whose sponsor pays the fee; checked and signed here
    Sponsored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SideArg {
    #[value(alias = "buy", alias = "bid")]
    Long,
    #[value(alias = "sell", alias = "ask")]
    Short,
}

/// A market: symbol, name or address, with `--expiry`.
#[derive(Clone, Debug, Args)]
pub struct MarketArg {
    #[arg(
        value_name = "MARKET",
        help = "Symbol (ramx: the open market expiring first), name (ramx-nov26) or address"
    )]
    pub market: String,
    #[arg(
        long,
        value_name = "EXPIRY",
        help = "Pick the expiry for a bare symbol: nov26, 2026-11 or unix seconds"
    )]
    pub expiry: Option<String>,
}

/// An order on a futures market: how much, what range, market or limit.
#[derive(Clone, Debug, Args)]
pub struct OrderArgs {
    #[command(flatten)]
    pub market: MarketArg,
    #[arg(
        value_name = "LOTS",
        value_parser = clap::value_parser!(u64).range(1..),
        required_unless_present = "usd",
        help = "Lots to trade; a lot pays the market's lot value a point inside its range"
    )]
    pub lots: Option<u64>,
    #[arg(
        long,
        value_name = "USD",
        conflicts_with = "lots",
        help = "Spend about this much USDC instead of naming lots"
    )]
    pub usd: Option<String>,
    #[arg(
        long,
        short = 'x',
        value_name = "X",
        default_value_t = super::shape::DEFAULT_LEVERAGE,
        value_parser = parse_leverage,
        help = "Leverage: sets where the range ends on the losing side"
    )]
    pub leverage: u32,
    #[arg(
        long,
        value_name = "PRICE",
        help = "Where the range ends on the winning side [default: the market's suggested range]"
    )]
    pub to: Option<String>,
    #[arg(
        long,
        value_name = "LO-HI",
        conflicts_with_all = ["leverage", "to"],
        help = "Name the whole range instead, e.g. 90-120"
    )]
    pub range: Option<String>,
    #[arg(
        long,
        value_name = "PRICE",
        help = "Limit order: enter at this price; what does not fill now rests on the book"
    )]
    pub limit: Option<String>,
    #[arg(
        long,
        requires = "limit",
        help = "With --limit: only rest, never fill now"
    )]
    pub post: bool,
    #[arg(
        long,
        value_name = "PCT",
        default_value = "2",
        help = "Market orders: how much worse than the quote they may fill, in percent"
    )]
    pub slippage: String,
}

#[derive(Clone, Debug, Args)]
pub struct QuoteArgs {
    #[arg(value_name = "SIDE", help = "long (buy) or short (sell)")]
    pub side: SideArg,
    #[command(flatten)]
    pub order: OrderArgs,
}

/// A position or order number from `petri positions`, or `all`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pick {
    One(u32),
    All,
}

fn parse_pick(text: &str) -> Result<Pick, String> {
    let t = text.trim().trim_start_matches('#');
    if t.eq_ignore_ascii_case("all") {
        return Ok(Pick::All);
    }
    t.parse()
        .map(Pick::One)
        .map_err(|_| format!("`{text}` is not a number from `petri positions` or `all`"))
}

#[derive(Clone, Debug, Args)]
pub struct CloseArgs {
    #[command(flatten)]
    pub market: MarketArg,
    #[arg(
        value_name = "POSITION",
        value_parser = parse_pick,
        help = "Position number from `petri positions`, or `all`"
    )]
    pub position: Pick,
    #[arg(
        long,
        short = 'n',
        value_parser = clap::value_parser!(u64).range(1..),
        help = "Lots to close [default: all of them]"
    )]
    pub lots: Option<u64>,
    #[arg(
        long,
        value_name = "PCT",
        default_value = "2",
        help = "How much worse than the quote the close may fill, in percent"
    )]
    pub slippage: String,
    #[arg(
        long,
        help = "Leave the proceeds in the market instead of sending them to your wallet"
    )]
    pub keep_cash: bool,
}

#[derive(Clone, Debug, Args)]
pub struct CancelArgs {
    #[command(flatten)]
    pub market: MarketArg,
    #[arg(
        value_name = "ORDER",
        value_parser = parse_pick,
        help = "Order number from `petri positions`, or `all`"
    )]
    pub order: Pick,
}

#[derive(Clone, Debug, Args)]
pub struct AmountArgs {
    #[command(flatten)]
    pub market: MarketArg,
    #[arg(value_name = "USD", help = "USDC amount (withdraw also takes `all`)")]
    pub amount: String,
}

pub fn parse_leverage(text: &str) -> Result<u32, String> {
    let raw = text.trim().trim_end_matches(['x', 'X']);
    match raw.parse::<u32>() {
        Ok(l) if l >= 1 => Ok(l),
        _ => Err(format!("`{text}` is not a leverage (e.g. 5 or 5x)")),
    }
}

/// A percentage as basis points: `"2"` or `"2%"` → 200.
pub fn parse_percent_bps(text: &str) -> Result<u64, String> {
    let t = text.trim().trim_end_matches('%');
    super::units::parse_decimal(t, 2)
        .ok()
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v <= 10_000)
        .ok_or_else(|| format!("`{text}` is not a percentage from 0 to 100"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_parsers() {
        assert_eq!(parse_leverage("10x"), Ok(10));
        assert!(parse_leverage("0").is_err());
        assert_eq!(parse_percent_bps("2"), Ok(200));
        assert_eq!(parse_percent_bps("0.5%"), Ok(50));
        assert!(parse_percent_bps("101").is_err());
        assert_eq!(parse_pick("#78"), Ok(Pick::One(78)));
        assert_eq!(parse_pick("ALL"), Ok(Pick::All));
        assert!(parse_pick("x").is_err());
    }
}
