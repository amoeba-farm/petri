use clap::{
    ArgAction, Parser, Subcommand, ValueEnum,
    builder::{
        Styles,
        styling::{AnsiColor, Color, Style},
    },
};

use crate::futures::{
    args::{AmountArgs, CancelArgs, CloseArgs, MarketArg, OrderArgs, QuoteArgs, RouteArg},
    settings::Profile,
};

const HELP_STYLES: Styles = Styles::styled()
    .header(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Cyan)))
            .bold(),
    )
    .usage(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Cyan)))
            .bold(),
    )
    .literal(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Green)))
            .bold(),
    )
    .placeholder(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
            .bold(),
    )
    .valid(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Green)))
            .bold(),
    )
    .invalid(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Red)))
            .bold(),
    )
    .error(
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Red)))
            .bold(),
    );

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Plain,
    Json,
}

/// A chart's time window.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ChartRangeValue {
    #[value(name = "1h", alias = "1H")]
    OneHour,
    #[value(name = "24h", alias = "1d", alias = "24H", alias = "1D")]
    TwentyFourHours,
    #[value(name = "7d", alias = "1w", alias = "7D", alias = "1W")]
    SevenDays,
    #[value(name = "30d", alias = "1m", alias = "30D", alias = "1M")]
    ThirtyDays,
    #[value(name = "all", alias = "ALL")]
    All,
}

impl ChartRangeValue {
    pub fn label(self) -> &'static str {
        match self {
            Self::OneHour => "1h",
            Self::TwentyFourHours => "24h",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
            Self::All => "all",
        }
    }

    pub fn window_ms(self) -> Option<u64> {
        match self {
            Self::OneHour => Some(60 * 60 * 1_000),
            Self::TwentyFourHours => Some(24 * 60 * 60 * 1_000),
            Self::SevenDays => Some(7 * 24 * 60 * 60 * 1_000),
            Self::ThirtyDays => Some(30 * 24 * 60 * 60 * 1_000),
            Self::All => None,
        }
    }
}

const GLOBAL: &str = "Global options";

/// `--profile`, `--rpc` and `--api` help: their defaults follow the profile,
/// and the build's profile (`dev-chain` builds default to dev).
pub const PROFILE_HELP: &str = if cfg!(feature = "dev-chain") {
    "Deployment: dev (the dev chain; this build's default) or production (mainnet)"
} else {
    "Deployment: production (mainnet; the default) or dev (the dev chain)"
};
pub const RPC_HELP: &str =
    "Solana RPC [default: https://api.mainnet-beta.solana.com; dev: http://127.0.0.1:8899]";
pub const API_HELP: &str =
    "Futures API [default: https://api.amoeba.farm; dev: http://127.0.0.1:4500]";

#[derive(Debug, Parser)]
#[command(name = "petri")]
#[command(bin_name = "petri")]
#[command(version)]
#[command(styles = HELP_STYLES)]
#[command(color = clap::ColorChoice::Auto)]
#[command(
    about = "Trade RAMX and NANDX capped futures from the terminal",
    long_about = "Petri trades Amoeba's capped futures on the RAMX and NANDX memory-chip indexes. A position is a range of the index: it pays a fixed amount per point the index settles inside the range, so it never loses more than it cost and is never liquidated. Petri also reads the index oracle and submits oracle evidence."
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        env = "SOLANA_KEYPAIR",
        hide_env = true,
        value_name = "PATH",
        help_heading = GLOBAL,
        help = "The wallet that signs (a keypair file or usb:// hardware wallet)"
    )]
    pub keypair: Option<String>,
    #[arg(long, global = true, help_heading = GLOBAL, help = "Print JSON")]
    pub json: bool,
    #[arg(
        long,
        global = true,
        help_heading = GLOBAL,
        help = "Send without asking for confirmation"
    )]
    pub yes: bool,
    #[arg(
        long,
        global = true,
        env = "PETRI_RPC_URL",
        hide_env = true,
        value_name = "URL",
        help_heading = GLOBAL,
        help = RPC_HELP
    )]
    pub rpc: Option<String>,
    #[arg(
        long = "api",
        global = true,
        env = "PETRI_API_URL",
        hide_env = true,
        value_name = "URL",
        help_heading = GLOBAL,
        help = API_HELP
    )]
    pub futures_api: Option<String>,
    #[arg(
        long,
        global = true,
        env = "PETRI_PROFILE",
        hide_env = true,
        value_enum,
        hide_possible_values = true,
        value_name = "PROFILE",
        help_heading = GLOBAL,
        help = PROFILE_HELP
    )]
    pub profile: Option<Profile>,
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t = RouteArg::Auto,
        hide_default_value = true,
        hide_possible_values = true,
        value_name = "ROUTE",
        help_heading = GLOBAL,
        help = "Who pays the fee: local (your wallet), sponsored (the API), or auto"
    )]
    pub route: RouteArg,
    #[arg(
        long,
        visible_alias = "plain",
        global = true,
        action = ArgAction::SetTrue,
        help_heading = GLOBAL,
        help = "No color"
    )]
    pub no_color: bool,
    #[arg(
        long,
        global = true,
        env = "PETRI_DEPLOYMENT",
        value_name = "PATH",
        hide = true,
        help = "Deployment manifest with the program ids to trust; defaults to the API's /v1/config"
    )]
    pub deployment: Option<std::path::PathBuf>,
    #[arg(
        long,
        global = true,
        env = "SOLANA_CONFIG",
        hide = true,
        help = "Solana CLI config to find the keypair in"
    )]
    pub solana_config: Option<String>,
    #[arg(
        long,
        global = true,
        env = "AMEBA_ALLOW_INSECURE_KEYPAIR",
        action = ArgAction::SetTrue,
        hide = true,
        help = "Allow signing with group- or world-readable keypair files"
    )]
    pub allow_insecure_keypair: bool,
    #[arg(
        long,
        global = true,
        env = "AMEBA_OUTPUT",
        value_enum,
        default_value_t = OutputFormat::Plain,
        hide = true
    )]
    pub output: OutputFormat,
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn resolved_output(&self) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else {
            self.output
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(
        visible_alias = "market",
        about = "Every market, or one market's price, index and suggested range",
        long_about = "Without MARKET: every market with its price, 24h change, index, volume and settlement date. With MARKET: that market's price, index, suggested range, best prices and activity.\n\nName a market by symbol (ramx: the open market expiring first), by name (ramx-nov26, \"RAMX Nov 26\") or by address."
    )]
    Markets {
        #[arg(
            value_name = "MARKET",
            help = "Symbol (ramx), name (ramx-nov26) or address"
        )]
        market: Option<String>,
        #[arg(
            long,
            value_name = "EXPIRY",
            requires = "market",
            help = "Pick the expiry for a bare symbol: nov26, 2026-11 or unix seconds"
        )]
        expiry: Option<String>,
    },
    #[command(
        about = "What a range costs to buy or sell, level by level",
        long_about = "The executable depth of one range on the book: every price you could buy it at (sell rows) and sell it at (buy rows), with the lots there. Different ranges trade against each other, so each range has its own depth. Without --range this is the market's suggested range."
    )]
    Book {
        #[command(flatten)]
        market: MarketArg,
        #[arg(
            long,
            value_name = "LO-HI",
            help = "The range, e.g. 90-120 [default: the suggested range]"
        )]
        range: Option<String>,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..), help = "Price levels per side")]
        depth: u32,
    },
    #[command(about = "A market's recent trades")]
    Trades {
        #[command(flatten)]
        market: MarketArg,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
    },
    #[command(about = "A market's price chart")]
    Chart {
        #[command(flatten)]
        market: MarketArg,
        #[arg(
            value_name = "WINDOW",
            value_enum,
            default_value_t = ChartRangeValue::TwentyFourHours,
            help = "1h, 24h, 7d, 30d or all"
        )]
        window: ChartRangeValue,
        #[arg(long, default_value_t = 14, help = "Chart height in rows")]
        height: usize,
    },
    #[command(
        about = "Price an order without sending it",
        long_about = "Price an order exactly as `petri long` or `petri short` would send it, without signing anything.",
        after_help = "Examples:\n  petri quote long ramx 100\n  petri quote short nandx --usd 50 -x 5"
    )]
    Quote(QuoteArgs),
    #[command(
        visible_alias = "buy",
        about = "Go long: buy a range of the index",
        long_about = "Go long: buy a range of the index. Each lot pays the market's lot value (petri market shows it) for every point the index settles above the range's low end, up to its high end. You pay for it up front, and that is the most you can lose.\n\nLeverage sets the low end (entry minus entry/leverage); --to sets the high end (default: the market's suggested range). Or name both ends with --range. A market order fills now within --slippage of its quote; --limit rests the rest of the order on the book at that entry. Petri quotes first and asks before sending unless --yes.",
        after_help = "Examples:\n  petri long ramx 100                  100 lots at 10x on the suggested range\n  petri long ramx 100 -x 5 --to 130    5x, the range ending at 130\n  petri long ramx --usd 50             spend about $50\n  petri long ramx 100 --range 90-120   name the range\n  petri long ramx 100 --limit 99       rest a bid at 99.00"
    )]
    Long(OrderArgs),
    #[command(
        visible_alias = "sell",
        about = "Go short: sell a range of the index",
        long_about = "Go short: sell a range of the index. Each lot pays the market's lot value (petri market shows it) for every point the index settles below the range's high end, down to its low end. You put in the rest of the range's value up front, and that is the most you can lose.\n\nLeverage sets the high end (entry plus entry/leverage); --to sets the low end (default: the market's suggested range). Or name both ends with --range. To close a long, use `petri close`.",
        after_help = "Examples:\n  petri short ramx 100\n  petri short nandx 50 -x 20 --to 80\n  petri short ramx 100 --limit 104 --post"
    )]
    Short(OrderArgs),
    #[command(
        visible_aliases = ["account", "pos"],
        about = "Your positions, orders and cash in each market, with P&L"
    )]
    Positions {
        #[arg(
            value_name = "OWNER",
            help = "Another wallet's address [default: yours]"
        )]
        owner: Option<String>,
    },
    #[command(
        about = "Close a position at the book",
        long_about = "Close a position at the book: a long is sold, a short bought back, on the position's own range. Petri quotes it, asks, and by default sends the cash back to your wallet in the same transaction.",
        after_help = "Examples:\n  petri close ramx 78\n  petri close ramx-nov26 all\n  petri close ramx 78 --lots 10 --keep-cash"
    )]
    Close(CloseArgs),
    #[command(about = "Cancel a resting order")]
    Cancel(CancelArgs),
    #[command(about = "Move USDC from your wallet into a market")]
    Deposit(AmountArgs),
    #[command(about = "Move free USDC from a market back to your wallet (an amount or `all`)")]
    Withdraw(AmountArgs),
    #[command(about = "Your trades, orders, deposits and settlements")]
    History {
        #[arg(
            value_name = "OWNER",
            help = "Another wallet's address [default: yours]"
        )]
        owner: Option<String>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
    },
    #[command(
        about = "Your wallet: address, SOL, USDC and cash in markets",
        args_conflicts_with_subcommands = true
    )]
    Wallet {
        #[command(subcommand)]
        command: Option<WalletCommand>,
        #[arg(
            value_name = "OWNER",
            help = "Another wallet's address [default: yours]"
        )]
        owner: Option<String>,
    },
    /// DEV-ONLY: REMOVE FOR PROD. Dev builds only (`dev-faucet`).
    #[cfg(feature = "dev-faucet")]
    #[command(about = "Get dev USDC (and SOL with --sol) for a wallet")]
    Faucet {
        #[arg(
            value_name = "OWNER",
            help = "Another wallet's address [default: yours]"
        )]
        owner: Option<String>,
        #[arg(
            long,
            value_name = "SOL",
            help = "Also airdrop this much SOL (local validators)"
        )]
        sol: Option<String>,
    },
    #[command(about = "Check the cluster, programs, API and wallet")]
    Status,
    #[command(
        about = "The index oracle: months, sources, evidence and bonds",
        long_about = "Read the index oracle's months, sources and candidates from the futures API, and write to the oracle program: bond collateral, evidence, source proposals and supports, opening prints and updates. Writes are reviewed and confirmed (or --yes), then signed by your wallet; --simulate runs them without signing."
    )]
    Oracle {
        #[command(subcommand)]
        command: Option<crate::oracle::args::OracleCommand>,
    },
    #[command(
        about = "Connect research agents and submit oracle evidence",
        disable_help_flag = true
    )]
    Research {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(about = "Open the terminal interface: markets and the oracle research workspace")]
    Tui {
        #[arg(long, help = "Don't check for updates in this session")]
        no_update_check: bool,
        #[arg(long, hide = true, value_name = "WIDTHxHEIGHT")]
        render: Option<String>,
        #[arg(long, hide = true, value_name = "VIEW")]
        view: Option<String>,
    },
    #[command(about = "Show or change saved settings")]
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },
    #[command(about = "Connect Petri to AI agents (read-only)")]
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    #[command(about = "Check for and install Petri updates")]
    Update {
        #[command(subcommand)]
        command: Option<UpdateCommand>,
        #[arg(long, help = "Source checkouts: inspect local refs only, no git fetch")]
        no_fetch: bool,
        #[arg(
            long,
            help = "Source checkouts: update without reinstalling the Windows app"
        )]
        skip_shim: bool,
        #[arg(long, help = "Reopen the terminal interface after a standalone update")]
        restart: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum WalletCommand {
    #[command(about = "Print your wallet's address")]
    Address,
    #[command(about = "Create a new signing wallet (prints only its address)")]
    New {
        #[arg(
            long,
            value_name = "PATH",
            help = "Write it here instead, without making it your signing wallet"
        )]
        out: Option<std::path::PathBuf>,
        #[arg(long, help = "Replace an existing file (its key is lost)")]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    #[command(about = "Show every setting and where it comes from")]
    Show,
    #[command(about = "Save a setting")]
    Set {
        #[arg(value_enum)]
        key: ConfigKey,
        value: String,
    },
    #[command(about = "Restore a setting to its default")]
    Reset {
        #[arg(value_enum)]
        key: ConfigKey,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ConfigKey {
    #[value(name = "rpc", alias = "rpc-url", help = "Solana RPC")]
    RpcUrl,
    #[value(name = "api", alias = "futures-api-url", help = "Futures API")]
    FuturesApiUrl,
    #[value(name = "keypair", help = "Signing wallet")]
    Keypair,
    #[value(name = "deployment", help = "Deployment manifest")]
    Deployment,
    #[value(name = "profile", help = "production or dev")]
    Profile,
}

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    #[command(about = "Check whether Petri needs an update")]
    Check,
    #[command(about = "Show this installation's update channel and version")]
    Info,
    #[command(about = "Restore the previous app files after a standalone update")]
    Recover,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    #[command(hide = true, about = "Serve bounded typed public agent actions")]
    Serve,
    #[command(hide = true, about = "Describe typed public agent actions")]
    Actions,
    #[command(hide = true, about = "Invoke one typed public agent action")]
    Invoke {
        action: String,
        #[arg(
            long,
            required = true,
            help = "Read one bounded typed JSON request from stdin"
        )]
        request_stdin: bool,
    },
    #[command(about = "Show whether the Petri agent connection is healthy")]
    Status,
    #[command(about = "Connect Petri to Claude Code and Codex")]
    Enable,
    #[command(about = "Diagnose and repair the agent connection")]
    Repair,
    #[command(about = "Disconnect Petri from AI agents")]
    Disable,
    #[command(about = "Print the MCP tool manifest for agent wrappers")]
    Manifest,
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{ChartRangeValue, Cli, Command, ConfigCommand, ConfigKey, Profile, WalletCommand};
    use crate::futures::args::{Pick, RouteArg, SideArg};

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn refused(args: &[&str]) -> bool {
        Cli::try_parse_from(args).is_err()
    }

    #[test]
    fn command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_go_anywhere() {
        let cli = parse(&["petri", "markets"]);
        assert!(matches!(
            cli.command,
            Some(Command::Markets { market: None, .. })
        ));
        assert_eq!(cli.route, RouteArg::Auto);
        let cli = parse(&[
            "petri",
            "--json",
            "--rpc",
            "http://127.0.0.1:8899",
            "--api",
            "http://127.0.0.1:4500",
            "positions",
            "--keypair",
            "/tmp/k.json",
            "--route",
            "sponsored",
        ]);
        assert!(cli.json);
        assert_eq!(cli.rpc.as_deref(), Some("http://127.0.0.1:8899"));
        assert_eq!(cli.futures_api.as_deref(), Some("http://127.0.0.1:4500"));
        for (name, profile) in [
            ("dev", Profile::Dev),
            ("localnet", Profile::Dev),
            ("production", Profile::Production),
            ("mainnet", Profile::Production),
        ] {
            assert_eq!(
                parse(&["petri", "markets", "--profile", name]).profile,
                Some(profile)
            );
        }
        assert!(refused(&["petri", "--profile", "devnet", "markets"]));
        assert_eq!(cli.keypair.as_deref(), Some("/tmp/k.json"));
        assert_eq!(cli.route, RouteArg::Sponsored);
    }

    #[test]
    fn reads_parse() {
        let cli = parse(&["petri", "market", "ramx", "--expiry", "nov26"]);
        assert!(
            matches!(cli.command, Some(Command::Markets { market: Some(ref m), expiry: Some(_) }) if m == "ramx")
        );
        assert!(refused(&["petri", "markets", "--expiry", "nov26"]));
        let cli = parse(&[
            "petri", "book", "nandx", "--range", "90-120", "--depth", "5",
        ]);
        assert!(
            matches!(cli.command, Some(Command::Book { depth: 5, range: Some(ref r), .. }) if r == "90-120")
        );
        assert!(matches!(
            parse(&["petri", "trades", "ramx"]).command,
            Some(Command::Trades { limit: 20, .. })
        ));
        assert!(matches!(
            parse(&["petri", "chart", "ramx", "7d"]).command,
            Some(Command::Chart {
                window: ChartRangeValue::SevenDays,
                ..
            })
        ));
        assert!(matches!(
            parse(&["petri", "chart", "ramx"]).command,
            Some(Command::Chart {
                window: ChartRangeValue::TwentyFourHours,
                ..
            })
        ));
        for alias in ["positions", "account", "pos"] {
            assert!(matches!(
                parse(&["petri", alias]).command,
                Some(Command::Positions { owner: None })
            ));
        }
        assert!(matches!(
            parse(&["petri", "history", "--limit", "5"]).command,
            Some(Command::History { limit: 5, .. })
        ));
    }

    #[test]
    fn orders_parse() {
        let cli = parse(&["petri", "long", "ramx", "100"]);
        let Some(Command::Long(o)) = cli.command else {
            panic!()
        };
        assert_eq!(
            (o.lots, o.leverage, o.slippage.as_str()),
            (Some(100), 10, "2")
        );
        let cli = parse(&["petri", "buy", "ramx", "100", "-x", "5x", "--to", "130"]);
        let Some(Command::Long(o)) = cli.command else {
            panic!()
        };
        assert_eq!((o.leverage, o.to.as_deref()), (5, Some("130")));
        let cli = parse(&[
            "petri", "sell", "nandx", "--usd", "50", "--limit", "104", "--post",
        ]);
        let Some(Command::Short(o)) = cli.command else {
            panic!()
        };
        assert_eq!((o.lots, o.usd.as_deref(), o.post), (None, Some("50"), true));
        let cli = parse(&[
            "petri",
            "quote",
            "short",
            "ramx-nov26",
            "3",
            "--range",
            "90-120",
        ]);
        let Some(Command::Quote(q)) = cli.command else {
            panic!()
        };
        assert_eq!(q.side, SideArg::Short);
        assert_eq!(q.order.range.as_deref(), Some("90-120"));
        assert!(matches!(
            parse(&["petri", "quote", "buy", "ramx", "1"]).command,
            Some(Command::Quote(ref q)) if q.side == SideArg::Long
        ));
    }

    #[test]
    fn bad_orders_are_refused() {
        for args in [
            &["petri", "long", "ramx"][..],
            &["petri", "long", "ramx", "0"],
            &["petri", "long", "ramx", "10", "--usd", "5"],
            &["petri", "long", "ramx", "10", "-x", "0"],
            &[
                "petri", "long", "ramx", "10", "--range", "90-120", "-x", "5",
            ],
            &[
                "petri", "long", "ramx", "10", "--range", "90-120", "--to", "130",
            ],
            &["petri", "long", "ramx", "10", "--post"],
            &["petri", "quote", "hold", "ramx", "1"],
            &["petri", "long"],
        ] {
            assert!(refused(args), "{args:?}");
        }
    }

    #[test]
    fn positions_commands_parse() {
        let cli = parse(&[
            "petri",
            "close",
            "ramx",
            "78",
            "--lots",
            "10",
            "--keep-cash",
        ]);
        let Some(Command::Close(c)) = cli.command else {
            panic!()
        };
        assert_eq!(
            (c.position, c.lots, c.keep_cash),
            (Pick::One(78), Some(10), true)
        );
        let cli = parse(&["petri", "close", "ramx-nov26", "all"]);
        assert!(matches!(cli.command, Some(Command::Close(ref c)) if c.position == Pick::All));
        let cli = parse(&["petri", "cancel", "ramx", "#3"]);
        assert!(matches!(cli.command, Some(Command::Cancel(ref c)) if c.order == Pick::One(3)));
        let cli = parse(&["petri", "deposit", "ramx", "100"]);
        assert!(matches!(cli.command, Some(Command::Deposit(ref a)) if a.amount == "100"));
        let cli = parse(&["petri", "withdraw", "ramx", "all", "--expiry", "nov26"]);
        assert!(
            matches!(cli.command, Some(Command::Withdraw(ref a)) if a.amount == "all" && a.market.expiry.is_some())
        );
    }

    #[test]
    fn wallet_and_setup_parse() {
        assert!(matches!(
            parse(&["petri", "wallet"]).command,
            Some(Command::Wallet {
                command: None,
                owner: None
            })
        ));
        assert!(matches!(
            parse(&[
                "petri",
                "wallet",
                "C6B4a71wjn8nF4rBa3VWGppoi9EXsgvx1JGVZdj8VAK3"
            ])
            .command,
            Some(Command::Wallet {
                command: None,
                owner: Some(_)
            })
        ));
        assert!(matches!(
            parse(&["petri", "wallet", "address"]).command,
            Some(Command::Wallet {
                command: Some(WalletCommand::Address),
                ..
            })
        ));
        assert!(matches!(
            parse(&["petri", "wallet", "new", "--out", "/tmp/a.json"]).command,
            Some(Command::Wallet {
                command: Some(WalletCommand::New { force: false, .. }),
                ..
            })
        ));
        // DEV-ONLY: REMOVE FOR PROD.
        #[cfg(feature = "dev-faucet")]
        assert!(matches!(
            parse(&["petri", "faucet", "--sol", "2"]).command,
            Some(Command::Faucet {
                sol: Some(_),
                owner: None
            })
        ));
        #[cfg(not(feature = "dev-faucet"))]
        assert!(Cli::try_parse_from(["petri", "faucet"]).is_err());
        assert!(matches!(
            parse(&["petri", "status"]).command,
            Some(Command::Status)
        ));
        assert!(matches!(
            parse(&["petri", "config"]).command,
            Some(Command::Config { command: None })
        ));
        for (name, key) in [
            ("rpc", ConfigKey::RpcUrl),
            ("rpc-url", ConfigKey::RpcUrl),
            ("api", ConfigKey::FuturesApiUrl),
            ("keypair", ConfigKey::Keypair),
            ("deployment", ConfigKey::Deployment),
            ("profile", ConfigKey::Profile),
        ] {
            let cli = parse(&["petri", "config", "reset", name]);
            assert!(
                matches!(cli.command, Some(Command::Config { command: Some(ConfigCommand::Reset { key: k }) }) if k == key)
            );
        }
    }

    #[test]
    fn retired_commands_are_gone() {
        for args in [
            &["petri", "contracts"][..],
            &["petri", "lp"],
            &["petri", "exit", "ramx", "1"],
            &["petri", "place", "buy", "ramx"],
            &["petri", "cancel-all"],
            &["petri", "transfer", "ramx", "1", "x"],
            &["petri", "settlements", "show", "ramx"],
            &["petri", "candles", "ramx"],
            &["petri", "index", "ramx"],
            &["petri", "keys", "show"],
            &["petri", "demo"],
            &["petri", "markets", "show", "ramx", "extra"],
            &["petri", "--quiet", "markets"],
            &["petri", "--commitment", "confirmed", "markets"],
        ] {
            assert!(refused(args), "{args:?}");
        }
    }

    #[test]
    fn oracle_parses() {
        use crate::oracle::args::{
            CollateralCommand, EvidenceCommand, EvidenceKind, OracleCommand as O, SourceCommand,
        };
        let oracle = |args: &[&str]| match parse(args).command {
            Some(Command::Oracle { command }) => command,
            other => panic!("{args:?}: {other:?}"),
        };
        assert!(oracle(&["petri", "oracle"]).is_none());
        assert!(matches!(
            oracle(&["petri", "oracle", "months", "ramx"]),
            Some(O::Months { market: Some(_) })
        ));
        assert!(matches!(
            oracle(&["petri", "oracle", "sources", "ramx", "--expiry", "oct26", "--query", "ddr5"]),
            Some(O::Sources { command: None, ref list }) if list.expiry.as_deref() == Some("oct26")
        ));
        assert!(matches!(
            oracle(&[
                "petri", "oracle", "sources", "support", "ramx", "--source", "ab", "--stake", "5"
            ]),
            Some(O::Sources {
                command: Some(SourceCommand::Support { .. }),
                ..
            })
        ));
        assert!(matches!(
            oracle(&["petri", "oracle", "evidence", "write", "--kind", "locator", "--text", "https://x.com", "--simulate"]),
            Some(O::Evidence { command: EvidenceCommand::Write { kind: EvidenceKind::Locator, ref send, .. } }) if send.simulate
        ));
        assert!(matches!(
            oracle(&["petri", "oracle", "collateral", "deposit", "25"]),
            Some(O::Collateral {
                command: Some(CollateralCommand::Deposit { .. }),
                ..
            })
        ));
        for retired in [
            &["petri", "oracle", "rewards", "claim", "--kind", "opening"][..],
            &["petri", "oracle", "cleanup"],
            &["petri", "oracle", "drafts"],
        ] {
            assert!(refused(retired), "{retired:?}");
        }
    }

    #[test]
    fn research_forwards_agent_arguments() {
        assert!(matches!(
            parse(&["petri", "tui", "--no-update-check"]).command,
            Some(Command::Tui {
                no_update_check: true,
                ..
            })
        ));
        assert!(refused(&["petri", "tui", "ramx"]));
        let parsed = parse(&[
            "petri",
            "research",
            "mcp",
            "--agent",
            "cedar",
            "--harness",
            "Codex",
        ]);
        assert!(
            matches!(parsed.command, Some(Command::Research { args }) if args == ["mcp", "--agent", "cedar", "--harness", "Codex"])
        );
        let parsed = parse(&["petri", "research", "--help"]);
        assert!(matches!(parsed.command, Some(Command::Research { args }) if args == ["--help"]));
    }
}
