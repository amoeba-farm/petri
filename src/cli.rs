use clap::{
    ArgAction, Args, Parser, Subcommand, ValueEnum,
    builder::{
        Styles,
        styling::{AnsiColor, Color, Style},
    },
};

use crate::backend::MAX_SAFE_JSON_INTEGER;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum HistoryTypeValue {
    Trades,
    Claims,
    Oracle,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OracleRewardKindValue {
    SourceProposer,
    SourceSupport,
    Opening,
    Update,
}

impl OracleRewardKindValue {
    pub fn as_draft_value(self) -> &'static str {
        match self {
            Self::SourceProposer => "source_proposer",
            Self::SourceSupport => "source_support",
            Self::Opening => "opening",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OracleStakeKindValue {
    ListingBond,
    SupportStake,
    SourceChallenge,
    OpeningClaim,
    OpeningChallenge,
    UpdateClaim,
    UpdateChallenge,
    EmergencyVote,
}

impl OracleStakeKindValue {
    pub fn as_draft_value(self) -> &'static str {
        match self {
            Self::ListingBond => "listing_bond",
            Self::SupportStake => "support_stake",
            Self::SourceChallenge => "source_challenge",
            Self::OpeningClaim => "opening_claim",
            Self::OpeningChallenge => "opening_challenge",
            Self::UpdateClaim => "update_claim",
            Self::UpdateChallenge => "update_challenge",
            Self::EmergencyVote => "samba_emergency_vote",
        }
    }
}

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

#[derive(Debug, Parser)]
#[command(name = "petri")]
#[command(bin_name = "petri")]
#[command(version)]
#[command(styles = HELP_STYLES)]
#[command(color = clap::ColorChoice::Auto)]
#[command(
    about = "Petri market terminal for hardware markets, wallet positions, and oracle evidence",
    long_about = "Petri is a market terminal for discovering hardware markets, inspecting bounded-risk contracts, viewing wallet exposure, and checking oracle evidence. V3 governed wallet changes require a verified active gate and initialized business state. Frozen or unavailable state stops before preparation or signer access.",
    help_template = "{before-help}{usage-heading}\n  {usage}\n\n{all-args}{after-help}",
    before_help = "Petri\nAmoeba market terminal\n\nInspect bounded hardware markets, wallet exposure, and settlement evidence.\nV3 support: wallet changes require an active gate and initialized markets; unavailable release evidence returns CURRENT_PROGRAM_WRITE_ABI_UNAVAILABLE.\n\nStart here:\n  petri                         Open the market front door\n  petri markets                 Find live hardware markets\n  petri markets show <market>   Open one market\n  petri contracts --market <market>\n  petri writers list            Inspect collective writer sleeves\n  petri liquidity               Request manager-position discovery\n  petri liquidity plan ...      Preview manager-only liquidity\n  petri config show             Show the exact current release boundary",
    after_help = "Help:\n  petri help                    Show all command families\n  petri help <command>          Show flags and examples for one command\n  petri tui                     Open the interactive Lab Bench"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        env = "AMEBA_BACKEND_URL",
        default_value = "https://api.amoeba.farm",
        help_heading = "Connection",
        help = "Hosted Amoeba API origin; loopback is allowed for local development"
    )]
    pub backend_url: String,
    #[arg(
        long,
        global = true,
        env = "SOLANA_CONFIG",
        help_heading = "Wallet And Network",
        help = "Solana CLI config path; defaults to ~/.config/solana/cli/config.yml"
    )]
    pub solana_config: Option<String>,
    #[arg(
        long,
        global = true,
        env = "AMEBA_CLUSTER",
        default_value = "devnet",
        help_heading = "Connection",
        help = "Cluster label shown in CLI output"
    )]
    pub cluster: String,
    #[arg(
        long,
        global = true,
        env = "SOLANA_COMMITMENT",
        help_heading = "Wallet And Network",
        help = "Transaction preflight and confirmation commitment; Solana CLI config is used when omitted"
    )]
    pub commitment: Option<String>,
    #[arg(
        long,
        global = true,
        env = "SOLANA_KEYPAIR",
        help_heading = "Wallet And Network",
        help = "Local keypair or hardware wallet for inspection and explicitly approved operations; never exposed to MCP"
    )]
    pub keypair: Option<String>,
    #[arg(
        long,
        global = true,
        env = "AMEBA_ALLOW_INSECURE_KEYPAIR",
        action = ArgAction::SetTrue,
        help_heading = "Wallet And Network",
        help = "Allow signing with group/world-readable keypair files on Unix"
    )]
    pub allow_insecure_keypair: bool,
    #[arg(
        long,
        global = true,
        env = "AMEBA_OUTPUT",
        value_enum,
        default_value_t = OutputFormat::Plain,
        help_heading = "Display",
        help = "Output format"
    )]
    pub output: OutputFormat,
    #[arg(
        long,
        global = true,
        action = ArgAction::SetTrue,
        help_heading = "Display",
        help = "Shortcut for --output json"
    )]
    pub json: bool,
    #[arg(
        long,
        global = true,
        action = ArgAction::SetTrue,
        help_heading = "Display",
        help = "Suppress plain-text command output; JSON output is still printed"
    )]
    pub quiet: bool,
    #[arg(
        long,
        global = true,
        action = ArgAction::SetTrue,
        help_heading = "Display",
        help = "Retained compatibility flag; current wallet changes remain unavailable"
    )]
    pub yes: bool,
    #[arg(
        long,
        visible_alias = "plain",
        global = true,
        action = ArgAction::SetTrue,
        help_heading = "Display",
        help = "Disable terminal color and use the plain banner"
    )]
    pub no_color: bool,
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
        about = "Prepare wallet funding, staking, and Oracle participation; never signs without separate approval"
    )]
    Participate(crate::participation::ActionArgs),
    #[command(about = "Private Oracle commit/reveal recovery; salts never appear in output")]
    Commitments {
        #[command(subcommand)]
        command: crate::oracle_commitments::Command,
    },
    #[command(about = "Inspect and recover wallet operations without resubmitting")]
    Operations {
        #[command(subcommand)]
        command: OperationsCommand,
    },
    #[command(
        about = "List markets or open a market view",
        long_about = "List markets or open a market view.\n\nUse `petri markets` for the market list, `petri markets show <market>` for the tradeable market summary, `petri markets status <market>` for the compact status view, `petri markets print <market>` for the latest oracle print, and `petri markets chart <market>` for price history."
    )]
    Markets {
        #[command(subcommand)]
        command: Option<MarketsCommand>,
    },
    #[command(about = "Browse listed option contracts")]
    Contracts {
        #[command(flatten)]
        options: ContractsArgs,
    },
    #[command(
        about = "Quote, review, buy and sell exact option series",
        long_about = "Prepare bounded exact-input trades. Buying spends an explicit quote budget for a minimum option quantity; selling transfers owned options for minimum proceeds. Every submission revalidates the reviewed plan and current permission."
    )]
    Trades {
        #[command(subcommand)]
        command: TradesCommand,
    },
    #[command(
        about = "Inspect and operate collective writer sleeves",
        long_about = "Read collective writer sleeves, close previews, status, and policy audits. Supported wallet changes require finalized V3 permission and initialized business state. Missing readiness fails closed."
    )]
    Writers {
        #[command(subcommand)]
        command: WriterCommand,
    },
    #[command(
        about = "Request manager liquidity or an unsigned preview",
        long_about = "Request current manager-liquidity positions or send manager inputs to Lean for an unsigned preview. The current RC44 Lean route fails closed when a non-empty position cannot be joined to its canonical Market, so position discovery is not yet a proven live capability. Petri does not independently SDK-validate, sign, or submit liquidity mutations. This is not an option-token or Flat portfolio."
    )]
    Liquidity {
        #[command(subcommand)]
        command: Option<LiquidityCommand>,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Liquidity-manager wallet to inspect when no subcommand is supplied; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(about = "Read wallet address and balances")]
    Wallet {
        #[command(subcommand)]
        command: Option<WalletCommand>,
    },
    #[command(
        about = "View staking state and prepare exact wallet actions",
        long_about = "View AMBA and sAMBA balances, queues, voting value, and unstaking state. Supported wallet changes require finalized V3 permission and initialized business state. Missing readiness fails closed."
    )]
    Staking {
        #[arg(long, global = true, help = "Exact product for a staking action")]
        market: Option<String>,
        #[arg(long, global = true, help = "Exact listed series for a staking action")]
        expiry: Option<String>,
        #[command(subcommand)]
        command: Option<StakingCommand>,
    },
    #[command(about = "Read indexed wallet history")]
    History {
        #[command(flatten)]
        history: HistoryArgs,
    },
    #[command(about = "Inspect current settlement records")]
    Settlements {
        #[command(subcommand)]
        command: SettlementsCommand,
    },
    #[command(about = "Read current oracle state and review non-submitting semantic drafts")]
    Oracle {
        #[command(subcommand)]
        command: Option<OracleCommand>,
    },
    #[command(about = "Inspect or update Amoeba service configuration")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Connect Petri to supported AI agents")]
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    #[command(about = "Check for and install Petri updates")]
    Update {
        #[command(subcommand)]
        command: Option<UpdateCommand>,
        #[arg(
            long,
            action = ArgAction::SetTrue,
            help = "Skip git fetch and inspect only local refs"
        )]
        no_fetch: bool,
        #[arg(
            long,
            action = ArgAction::SetTrue,
            help = "Update Petri without reinstalling the Windows app"
        )]
        skip_shim: bool,
        #[arg(
            long,
            global = true,
            help = "Approve installing the reviewed preview release without an interactive prompt"
        )]
        yes: bool,
        #[arg(
            long,
            global = true,
            help = "Reopen the terminal interface after a standalone update or recovery"
        )]
        restart: bool,
    },
    #[command(about = "Open the interactive Lab Bench TUI")]
    Tui {
        #[arg(value_name = "MARKET", help = "Optional market id to open first")]
        dish: Option<String>,
        #[arg(
            long,
            action = ArgAction::SetTrue,
            help = "Do not check for Petri updates when the TUI starts"
        )]
        no_update_check: bool,
    },
    #[command(
        about = "Open the current RAMX market",
        long_about = "Open the current RAMX market in the Lab Bench when interactive, or fetch its live snapshot for non-interactive and JSON output. This is not an offline bundled fixture."
    )]
    Demo,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show,
    #[command(about = "Save the Amoeba service origin used by CLI and TUI")]
    Set {
        #[arg(value_enum)]
        key: ConfigKey,
        value: String,
    },
    #[command(about = "Restore a Petri setting to its hosted default")]
    Reset {
        #[arg(value_enum)]
        key: ConfigKey,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ConfigKey {
    #[value(name = "backend-url")]
    BackendUrl,
}

#[derive(Debug, Subcommand)]
pub enum WalletCommand {
    #[command(about = "Print the resolved signer public key")]
    Address,
    #[command(about = "Show wallet SOL, USDC, and AMBA balances")]
    Balance {
        owner_pubkey: Option<String>,
        #[arg(long, env = "AMEBA_QUOTE_MINT")]
        usdc_mint: Option<String>,
        #[arg(long, env = "AMBA_MINT", alias = "major-token-mint")]
        amba_mint: Option<String>,
    },
    #[command(about = "Show current trading collateral and the next safe action")]
    Collateral {
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Wallet public key to inspect; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
}

#[derive(Debug, Args, Clone)]
pub struct StakingStatusArgs {
    #[arg(
        long,
        value_name = "PUBKEY",
        help = "Wallet public key to inspect; defaults to the attached wallet"
    )]
    pub owner: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct StakingStakeArgs {
    #[arg(
        long,
        value_name = "AMOUNT",
        help = "Exact AMBA amount to queue, using a decimal string"
    )]
    pub amount: String,
}

#[derive(Debug, Args, Clone)]
pub struct StakingActivateArgs {
    #[arg(
        long = "min-received",
        value_name = "AMOUNT",
        help = "Minimum exact sAMBA accepted at the current activation-time share rate"
    )]
    pub min_received: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct StakingAmountArgs {
    #[arg(
        long,
        value_name = "AMOUNT",
        help = "Exact token amount, using a decimal string"
    )]
    pub amount: String,
    #[arg(
        long = "min-received",
        value_name = "AMOUNT",
        help = "Minimum exact output accepted if the AMBA-per-sAMBA rate moves"
    )]
    pub min_received: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum StakingCommand {
    #[command(
        about = "Show available and queued AMBA, activation, sAMBA, voting, and unstaking status"
    )]
    Status {
        #[command(flatten)]
        options: StakingStatusArgs,
    },
    #[command(about = "Prepare AMBA staking for review; signing is a separate approval")]
    Stake {
        #[command(flatten)]
        options: StakingStakeArgs,
    },
    #[command(about = "Prepare activation of queued AMBA for review")]
    Activate {
        #[command(flatten)]
        options: StakingActivateArgs,
    },
    #[command(
        about = "Not wired: cancel queued AMBA",
        long_about = "This action has no typed submission route and remains not_wired. No transaction is signed or sent."
    )]
    Cancel,
    #[command(about = "Prepare sAMBA unstaking for review")]
    Unstake {
        #[command(flatten)]
        options: StakingAmountArgs,
    },
    #[command(about = "Prepare completion of a ready AMBA unstake")]
    Claim,
}

#[derive(Debug, Subcommand)]
pub enum MarketsCommand {
    #[command(about = "Open a market summary")]
    Show {
        #[arg(value_name = "MARKET", help = "Market id")]
        market_id: String,
    },
    #[command(about = "Show current market status")]
    Status {
        #[arg(value_name = "MARKET", help = "Market id")]
        market_id: String,
    },
    #[command(about = "View the latest oracle print for a market")]
    Print {
        #[arg(value_name = "MARKET", help = "Market id")]
        market_id: String,
    },
    #[command(about = "Open a terminal chart with live price history")]
    Chart {
        #[command(flatten)]
        options: ChartArgs,
    },
}

#[derive(Debug, Args, Clone)]
pub struct ContractsArgs {
    #[arg(long, value_name = "MARKET", help = "Market id")]
    pub market: Option<String>,
    #[arg(long, help = "Exact series id to show; omit to list current series")]
    pub expiry: Option<String>,
    #[arg(
        long,
        default_value_t = 17,
        value_parser = parse_contract_row_count,
        help = "Maximum series rows to show when --expiry is omitted"
    )]
    pub rows: usize,
    #[arg(long, help = "Show every current series row")]
    pub all: bool,
}

fn parse_contract_row_count(raw: &str) -> Result<usize, String> {
    let rows = raw
        .parse::<usize>()
        .map_err(|_| "rows must be an integer from 1 through 36".to_string())?;
    if (1..=36).contains(&rows) {
        Ok(rows)
    } else {
        Err("rows must be from 1 through 36".to_string())
    }
}

#[derive(Debug, Subcommand)]
pub enum WriterCommand {
    #[command(
        hide = true,
        about = "Show live writer-close runtime and custody-mode availability",
        long_about = "Read Amoeba's current writer-close capability projection, including the pinned release identity, lifecycle operations, and separate hot/cold Light-account availability. This is read-only and does not prepare, sign, or submit anything."
    )]
    Capabilities,
    #[command(
        hide = true,
        name = "available-actions",
        about = "Show exact wallet-specific writer action availability",
        long_about = "Read and validate the exact current 23-entry action mask for one canonical owner and writer sleeve. This is read-only availability, not authorization, preparation, signing, or submission."
    )]
    AvailableActions {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
        #[arg(long, value_name = "PUBKEY")]
        owner: String,
    },
    #[command(
        about = "List the current global collective-writer sleeve catalog",
        long_about = "List every current collective-writer sleeve returned by Amoeba. This is a global catalog, not a wallet-owned position view."
    )]
    List {
        #[arg(
            long,
            value_name = "PUBKEY",
            hide = true,
            help = "Deprecated unsupported owner filter"
        )]
        owner: Option<String>,
    },
    #[command(about = "Show one collective writer sleeve and its staged state")]
    Show {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
    },
    #[command(about = "Deposit writer principal through a validated wallet operation")]
    Deposit {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
        #[arg(
            long,
            value_name = "ATOMS",
            help = "Exact USDC atoms as a canonical integer string"
        )]
        amount: String,
    },
    #[command(hide = true, about = "Historical auction bid grammar")]
    Bid {
        #[arg(long, value_name = "AUCTION")]
        auction: String,
        #[arg(long = "series-index", value_name = "INDEX", value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
        #[arg(long, value_name = "ATOMS", help = "Exact price-per-contract atoms")]
        price: String,
        #[arg(long, value_name = "ATOMS", help = "Exact requested contract atoms")]
        amount: String,
    },
    #[command(about = "Withdraw available writer principal")]
    Withdraw {
        #[arg(long)]
        sleeve: String,
        #[arg(long, value_name = "ATOMS")]
        amount: String,
    },
    #[command(
        about = "Discover historical refunds for one owner, independently of active auctions"
    )]
    Refunds {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u8).range(1..33))]
        limit: u8,
    },
    #[command(about = "Refund one exact historical auction bid to its canonical destination")]
    Refund {
        #[arg(long)]
        auction: String,
        #[arg(long)]
        bid: String,
    },
    #[command(about = "Read writer liquidity and frozen buyback limits for one series")]
    Liquidity {
        #[arg(long)]
        sleeve: String,
        #[arg(long)]
        owner: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
    },
    #[command(about = "Initialize the sleeve-owned liquidity position in its canonical pool")]
    LiquidityInitialize {
        #[arg(long)]
        sleeve: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
    },
    #[command(about = "Add writer inventory and bounded buyback bids")]
    LiquidityAdd {
        #[arg(long)]
        sleeve: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
        #[arg(long, value_name = "ATOMS")]
        issue_amount: String,
        #[arg(
            long = "bin",
            required = true,
            value_name = "ID:OPTION_ATOMS:QUOTE_ATOMS",
            help = "Repeat up to eight times in strictly ascending bin order"
        )]
        bins: Vec<String>,
    },
    #[command(about = "Return quote to the sleeve and burn removed unsold writer options")]
    LiquidityRemove {
        #[arg(long)]
        sleeve: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
        #[arg(
            long = "bin",
            required = true,
            value_name = "ID:OPTION_ATOMS:QUOTE_ATOMS",
            help = "Repeat up to eight times in strictly ascending bin order"
        )]
        bins: Vec<String>,
    },
    #[command(about = "Sweep uncommitted writer proceeds into canonical sleeve cash")]
    LiquiditySweep {
        #[arg(long)]
        sleeve: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..20))]
        series_index: u8,
    },
    #[command(
        name = "close-preview",
        about = "Preview the Lean-admitted staged close without submitting"
    )]
    ClosePreview {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
        #[arg(
            long,
            value_name = "ATOMS",
            help = "Exact Flat atoms proposed for close"
        )]
        amount: String,
        #[arg(
            long = "minimum-withdrawal",
            value_name = "ATOMS",
            help = "User's exact minimum USDC withdrawal guard"
        )]
        minimum_withdrawal: String,
    },
    #[command(
        about = "Start or advance one validated close stage",
        group(clap::ArgGroup::new("close_mode").required(true).args(["sleeve", "close_request"]).multiple(true))
    )]
    Close {
        #[arg(
            long,
            value_name = "SLEEVE",
            requires_all = ["amount", "minimum_withdrawal"],
            conflicts_with = "cancel"
        )]
        sleeve: Option<String>,
        #[arg(
            long,
            value_name = "ATOMS",
            help = "Exact Flat atoms proposed for a new close",
            requires_all = ["sleeve", "minimum_withdrawal"],
            conflicts_with = "cancel"
        )]
        amount: Option<String>,
        #[arg(
            long = "minimum-withdrawal",
            value_name = "ATOMS",
            help = "User's exact minimum USDC withdrawal guard for a new close",
            requires_all = ["sleeve", "amount"],
            conflicts_with = "cancel"
        )]
        minimum_withdrawal: Option<String>,
        #[arg(
            long = "close-request",
            value_name = "REQUEST",
            help = "Existing request to advance/cancel, or an optional assertion when starting"
        )]
        close_request: Option<String>,
        #[arg(
            long,
            requires = "close_request",
            help = "Visible lifecycle grammar; unavailable until a reviewed wallet-action ABI defines close cancel"
        )]
        cancel: bool,
    },
    #[command(
        name = "close-status",
        about = "Show staged-close progress and the next permitted action"
    )]
    CloseStatus {
        #[arg(long = "close-request", value_name = "REQUEST")]
        close_request: String,
    },
    #[command(about = "Claim a settled collective long series or Flat residual")]
    Claim {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
        #[arg(long, value_enum)]
        variant: WriterClaimVariant,
        #[arg(
            long = "series-index",
            value_name = "INDEX",
            help = "Required for collective-long (0-19); omit for flat-residual"
        )]
        series_index: Option<u8>,
        #[arg(long, value_name = "ATOMS", help = "Exact positive claim-token atoms")]
        amount: String,
    },
    #[command(name = "transfer-flat", about = "Transfer Flat ownership")]
    TransferFlat {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
        #[arg(long, value_name = "PUBKEY")]
        destination: String,
        #[arg(long, value_name = "ATOMS", help = "Exact Flat atoms")]
        amount: String,
    },
    #[command(
        name = "policy-audit",
        about = "Show the active writer policy and immutable audit hashes"
    )]
    PolicyAudit {
        #[arg(long, value_name = "SLEEVE")]
        sleeve: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CollectiveSwapDirectionValue {
    #[value(name = "quote-for-option")]
    QuoteForOption,
    #[value(name = "option-for-quote")]
    OptionForQuote,
}

impl CollectiveSwapDirectionValue {
    pub const fn as_request_value(self) -> &'static str {
        match self {
            Self::QuoteForOption => "QuoteForOption",
            Self::OptionForQuote => "OptionForQuote",
        }
    }
}

#[derive(Debug, Args, Clone)]
pub struct CollectiveTradeArgs {
    #[arg(long, value_name = "MARKET", help = "Current Market selector")]
    pub market: String,
    #[arg(long, value_enum)]
    pub direction: CollectiveSwapDirectionValue,
    #[arg(
        long = "amount-in",
        value_name = "ATOMS",
        help = "Exact input token atoms"
    )]
    pub amount_in: String,
    #[arg(
        long = "minimum-amount-out",
        value_name = "ATOMS",
        help = "Exact minimum output token atoms"
    )]
    pub minimum_amount_out: String,
    #[arg(long = "limit-bin-id", value_name = "U16", value_parser = clap::value_parser!(u16))]
    pub limit_bin_id: u16,
}

#[derive(Debug, Subcommand)]
pub enum TradesCommand {
    #[command(about = "Prepare a human-unit ticket without signing")]
    Quote {
        #[command(flatten)]
        intent: crate::trade_service::TradeIntent,
        #[arg(long, value_enum, default_value = "buy")]
        side: TradeSide,
    },
    #[command(about = "Buy options using a bounded exact-input budget")]
    Buy {
        #[command(flatten)]
        intent: crate::trade_service::TradeIntent,
        #[arg(
            long,
            help = "Explicitly approve the fresh bounded trade; otherwise only prepare"
        )]
        yes: bool,
    },
    #[command(about = "Sell owned long options; never opens a naked short")]
    Sell {
        #[command(flatten)]
        intent: crate::trade_service::TradeIntent,
        #[arg(
            long,
            help = "Explicitly approve the fresh bounded sale; otherwise only prepare"
        )]
        yes: bool,
    },
    #[command(about = "Execute one previously reviewed, unexpired trade")]
    Execute {
        operation_id: String,
        #[arg(long, required = true, help = "Approve this exact reviewed operation")]
        yes: bool,
    },
    #[command(about = "Prepare an expert exact-input trade without signing")]
    Prepare {
        #[command(flatten)]
        trade: CollectiveTradeArgs,
    },
    #[command(
        about = "Prepare and submit an explicitly specified exact-input trade",
        long_about = "The supplied amounts and limits authorize a fresh exact-input operation. Petri validates current identity, native instruction meaning and permission before local signing."
    )]
    Submit {
        #[command(flatten)]
        trade: CollectiveTradeArgs,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Subcommand)]
pub enum OperationsCommand {
    #[command(
        about = "Approve one exact reviewed collateral, staking, Oracle, or liquidity operation"
    )]
    Execute {
        operation_id: String,
        #[arg(long)]
        yes: bool,
    },
    #[command(about = "List local operation references, optionally for one explicit owner")]
    List {
        #[arg(long)]
        owner: Option<String>,
    },
    #[command(about = "Show one local recovery reference; does not contact a signer")]
    Show { operation_id: String },
    #[command(about = "Reconcile the original operation; --watch polls for at most 30 refreshes")]
    Status {
        operation_id: String,
        #[arg(long)]
        watch: bool,
    },
    #[command(about = "Recover authoritative status only; never replays a transaction")]
    Resume { operation_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum WriterClaimVariant {
    #[value(name = "collective-long")]
    CollectiveLong,
    #[value(name = "flat-residual")]
    FlatResidual,
}

impl WriterClaimVariant {
    pub const fn as_request_value(self) -> &'static str {
        match self {
            Self::CollectiveLong => "collective_long",
            Self::FlatResidual => "flat_residual",
        }
    }
}

#[derive(Debug, Args, Clone)]
pub struct OptionsChainArgs {
    #[arg(long, value_name = "MARKET", help = "Market id")]
    pub market: String,
    #[arg(long, help = "Exact series id to show; omit to list current series")]
    pub expiry: Option<String>,
    #[arg(
        long,
        default_value_t = 17,
        value_parser = parse_contract_row_count,
        help = "Maximum series rows to show when --expiry is omitted"
    )]
    pub rows: usize,
    #[arg(long, help = "Show every current series row")]
    pub all: bool,
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

#[derive(Debug, Args, Clone)]
pub struct ChartArgs {
    #[arg(value_name = "MARKET", default_value = "ramx", help = "Market id")]
    pub market: String,
    #[arg(
        long,
        help = "Expiry id, for example <CURRENT_EXPIRY_ID>. Omit when a market-level chart is sufficient"
    )]
    pub expiry: Option<String>,
    #[arg(
        long,
        value_enum,
        default_value_t = ChartRangeValue::TwentyFourHours,
        help = "History window to fetch from the backend"
    )]
    pub range: ChartRangeValue,
    #[arg(
        long,
        default_value_t = 20,
        help = "Auto-refresh interval in seconds for the interactive TUI; 0 disables auto-refresh"
    )]
    pub refresh_seconds: u64,
    #[arg(
        long = "static",
        action = ArgAction::SetTrue,
        help = "Print a non-interactive terminal chart instead of entering full-screen TUI mode"
    )]
    pub static_view: bool,
    #[arg(
        long,
        default_value_t = 240,
        help = "Maximum chart points to render after range filtering"
    )]
    pub points: usize,
    #[arg(
        long,
        default_value_t = 14,
        help = "Static chart plot height in terminal rows"
    )]
    pub height: usize,
}

#[derive(Debug, Args, Clone)]
pub struct HistoryArgs {
    #[arg(help = "Owner public key; defaults to the attached local keypair pubkey")]
    pub owner_pubkey: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub limit: u16,
    #[arg(
        long = "type",
        value_enum,
        default_value_t = HistoryTypeValue::All,
        help = "Activity category; only all is authoritative in the current release"
    )]
    pub activity_type: HistoryTypeValue,
}

#[derive(Debug, Subcommand)]
pub enum SettlementsCommand {
    Show {
        market_id: String,
        expiry_id: String,
    },
    #[command(
        about = "Check whether oracle settlement is ready",
        alias = "preflight"
    )]
    Check {
        market_id: String,
        expiry_id: String,
    },
    Oracle {
        market_id: String,
        expiry_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleCommand {
    #[command(
        about = "Read exact-series carry lineage, selection progress and original opening provenance"
    )]
    Carry(crate::oracle_carry::Args),
    #[command(about = "Read spread-owned oracle state from Amoeba")]
    State,
    #[command(about = "List spread-owned oracle markets, or inspect one market")]
    Markets {
        #[arg(value_name = "MARKET", help = "Optional market id such as ramx")]
        market: Option<String>,
    },
    #[command(about = "Read the latest spread-owned oracle month/settlement")]
    Latest {
        #[arg(value_name = "MARKET", help = "Optional market id such as ramx")]
        market: Option<String>,
    },
    #[command(about = "Read spread-owned oracle settlement history")]
    History {
        #[arg(value_name = "MARKET", help = "Optional market id such as ramx")]
        market: Option<String>,
    },
    #[command(about = "Read the RAMX-MOD source recipe and evidence sources")]
    Recipe {
        #[command(flatten)]
        args: OracleRecipeArgs,
    },
    #[command(about = "Save local source proposal, support, opening, or challenge drafts")]
    Sources {
        #[command(subcommand)]
        command: Option<OracleSourceCommand>,
    },
    #[command(about = "Save local oracle opening-print drafts")]
    Prints {
        #[command(subcommand)]
        command: OraclePrintsCommand,
    },
    #[command(about = "Save local source-update or challenge drafts")]
    Updates {
        #[command(subcommand)]
        command: OracleUpdatesCommand,
    },
    #[command(about = "Save local emergency-dispute and vote drafts")]
    Emergency {
        #[command(subcommand)]
        command: OracleEmergencyCommand,
    },
    #[command(about = "Save a local draft for a USDC oracle reward claim")]
    Rewards {
        #[command(subcommand)]
        command: OracleRewardsCommand,
    },
    #[command(about = "Save a local draft to settle an oracle stake or bond")]
    Stakes {
        #[command(subcommand)]
        command: OracleStakesCommand,
    },
    #[command(about = "Save local AMBA-custody drafts for oracle voting power")]
    Amba {
        #[command(subcommand)]
        command: OracleAmbaCommand,
    },
    #[command(about = "List, validate, or inspect locally queued oracle drafts")]
    Drafts {
        #[command(subcommand)]
        command: Option<OracleDraftsCommand>,
    },
}

#[derive(Debug, Args, Clone)]
pub struct OracleRecipeArgs {
    #[arg(value_name = "MARKET", help = "Oracle market id")]
    pub market: Option<String>,
    #[arg(long, help = "Optional SKU, row, generation, or source search")]
    pub query: Option<String>,
    #[arg(long, default_value_t = 24, help = "Maximum matched nodes to return")]
    pub limit: usize,
}

#[derive(Debug, Args, Clone)]
pub struct OracleEmergencyCommitArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        long = "commit-hash",
        value_name = "HEX",
        help = "32-byte commit hash for the hidden vote"
    )]
    pub commit_hash: String,
    #[arg(
        long = "samba-amount",
        value_name = "AMOUNT",
        help = "sAMBA base units to transfer into the emergency pot"
    )]
    pub samba_amount: u64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleEmergencyRevealArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(long, value_name = "CHOICE", help = "Ballot choice to reveal")]
    pub choice: String,
    #[arg(
        long,
        value_name = "HEX",
        help = "32-byte salt used in the commit hash"
    )]
    pub salt: String,
}

#[derive(Debug, Args, Clone)]
pub struct OracleAmbaCustodyArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(long, value_name = "AMOUNT", help = "AMBA base units to move")]
    pub amount: u64,
    #[arg(
        long,
        value_name = "MINT",
        default_value = crate::wallet_balance::DEFAULT_AMBA_MINT,
        help = "Classic SPL AMBA mint public key"
    )]
    pub mint: String,
    #[arg(
        long = "user-token-account",
        value_name = "TOKEN_ACCOUNT",
        help = "User-owned AMBA token account; backend prepare derives the signer ATA when omitted"
    )]
    pub user_token_account: Option<String>,
    #[arg(
        long = "vault-token-account",
        value_name = "TOKEN_ACCOUNT",
        default_value = "CawSd1hKG9fBbBnNDQRnhtP5oHMWwy6quWFj4EWxxxMz",
        help = "AMBA token account owned by the spread vault config PDA"
    )]
    pub vault_token_account: String,
}

#[derive(Debug, Args, Clone)]
pub struct OracleRewardClaimArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(long, value_enum, help = "Current USDC reward kind to claim")]
    pub kind: OracleRewardKindValue,
    #[arg(
        long = "source-id",
        value_name = "SOURCE_ID",
        help = "Canonical source id from the current oracle read projection"
    )]
    pub source_id: Option<String>,
    #[arg(
        long = "claim-id",
        value_name = "CLAIM_ID",
        help = "Canonical current update-claim id; required for update rewards"
    )]
    pub claim_id: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct OracleStakeSettleArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        long,
        value_enum,
        help = "Stake or bond kind; the program derives refund or slash from terminal state"
    )]
    pub kind: OracleStakeKindValue,
    #[arg(
        long = "source-id",
        value_name = "SOURCE_ID",
        help = "Listing-bond source id; valid only for listing-bond settlement"
    )]
    pub source_id: Option<String>,
    #[arg(
        long = "subject-pda",
        value_name = "PUBKEY",
        help = "Stake/bond record public key from oracle latest or the TUI"
    )]
    pub subject_pda: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum OracleDraftsCommand {
    #[command(about = "List locally queued semantic drafts")]
    List {
        #[arg(long, default_value_t = 16, help = "Maximum drafts to show")]
        limit: usize,
        #[arg(
            long,
            value_name = "PATH",
            help = "Override the local oracle draft store"
        )]
        path: Option<String>,
    },
    #[command(
        about = "Show the complete reviewable intent for queued drafts",
        long_about = "Show market/month/series identity, oracle context, evidence/value/stake fields, and local status. Secret-labelled fields stay redacted, and no transaction is prepared, signed, or submitted."
    )]
    Show {
        #[arg(
            value_name = "SELECTOR",
            default_value = "latest",
            help = "Draft id to show, or latest/all"
        )]
        selector: String,
        #[arg(
            long,
            value_name = "PATH",
            help = "Override the local oracle draft store"
        )]
        path: Option<String>,
    },
    #[command(about = "Validate queued semantic drafts without preparing a transaction")]
    Validate {
        #[arg(
            value_name = "SELECTOR",
            default_value = "latest",
            help = "Draft id to validate, or latest/all"
        )]
        selector: String,
        #[arg(
            long,
            value_name = "PATH",
            help = "Override the local oracle draft store"
        )]
        path: Option<String>,
    },
    #[command(
        hide = true,
        about = "Compatibility command that reports oracle transaction preparation as not_wired"
    )]
    Submit {
        #[arg(
            value_name = "SELECTOR",
            default_value = "latest",
            help = "Semantic draft id, latest, or all (reported only; no transaction is prepared)"
        )]
        selector: String,
    },
}

#[derive(Debug, Args, Clone)]
pub struct OracleDraftCommonArgs {
    #[arg(long, default_value = "ramx", help = "Oracle market id")]
    pub market: String,
    #[arg(
        long,
        default_value = "unbound",
        help = "Local review label for the oracle month; this is not chain identity"
    )]
    pub month: String,
    #[arg(
        long,
        value_name = "EXPIRY",
        help = "Spread/DLMM expiry id this oracle draft targets, such as <CURRENT_EXPIRY_ID>"
    )]
    pub expiry: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Override the local oracle draft store"
    )]
    pub path: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub struct OracleSourceProposeArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Terminal source pin SKU or node index"
    )]
    pub node: String,
    #[arg(
        long = "category",
        alias = "source-category",
        help = "Public source category"
    )]
    pub source_category: String,
    #[arg(
        long = "locator",
        alias = "canonical-locator",
        help = "Canonical public source URL or locator"
    )]
    pub canonical_locator: String,
    #[arg(
        long = "definition",
        alias = "source-definition",
        help = "Exact source definition"
    )]
    pub source_definition: String,
    #[arg(
        long = "stake",
        alias = "stake-support",
        default_value_t = 1.0,
        help = "Cash/stablecoin support amount"
    )]
    pub stake_support: f64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleSourceSupportArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Source pin, row, or node index to support"
    )]
    pub node: String,
    #[arg(long, default_value_t = 1.0, help = "Cash/stablecoin support amount")]
    pub stake: f64,
    #[arg(long, default_value = "", help = "Optional support note")]
    pub note: String,
}

#[derive(Debug, Args, Clone)]
pub struct OracleOpeningClaimArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Frozen source pin SKU or node index"
    )]
    pub node: String,
    #[arg(long, help = "Observed opening value")]
    pub raw_value: f64,
    #[arg(long, help = "Source observation time as RFC 3339 or Unix seconds")]
    pub timestamp: String,
    #[arg(
        long,
        help = "Wayback URL https://web.archive.org/web/<14-digit UTC>/<exact canonical URL> (max 384 UTF-8 bytes)"
    )]
    pub archive_url: String,
    #[arg(
        long = "canonical-locator",
        alias = "locator",
        help = "Canonical public source URL or locator"
    )]
    pub canonical_locator: String,
    #[arg(
        long = "source-definition",
        alias = "definition",
        help = "Frozen source definition shown by the evidence"
    )]
    pub source_definition: String,
    #[arg(long, default_value_t = 1.0, help = "Opening claim stake amount")]
    pub stake: f64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleOpeningChallengeArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Frozen source pin SKU or node index"
    )]
    pub node: String,
    #[arg(long, help = "Why the pending opening claim is wrong")]
    pub reason: String,
    #[arg(long, help = "Corrected opening value")]
    pub corrected_value: f64,
    #[arg(long, help = "Corrected source time as RFC 3339 or Unix seconds")]
    pub timestamp: String,
    #[arg(
        long,
        help = "Wayback URL https://web.archive.org/web/<14-digit UTC>/<exact canonical URL> (max 384 UTF-8 bytes)"
    )]
    pub archive_url: String,
    #[arg(
        long = "canonical-locator",
        alias = "locator",
        help = "Canonical public source URL or locator"
    )]
    pub canonical_locator: String,
    #[arg(
        long = "source-definition",
        alias = "definition",
        help = "Frozen source definition shown by the evidence"
    )]
    pub source_definition: String,
    #[arg(
        long = "stake",
        alias = "stake-bond",
        default_value_t = 1.0,
        help = "Opening challenge bond amount"
    )]
    pub stake_bond: f64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleChallengeDraftArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Terminal source pin SKU or node index"
    )]
    pub node: String,
    #[arg(long, help = "Challenge reason")]
    pub reason: String,
    #[arg(
        long = "comparison-source",
        alias = "comparison-source-id",
        value_name = "SOURCE_ID",
        help = "Existing candidate source id or label for insufficient differentiation challenges"
    )]
    pub comparison_source_id: Option<String>,
    #[arg(long, help = "Wayback/archive URL for challenge evidence")]
    pub archive_url: String,
    #[arg(
        long = "stake",
        alias = "stake-bond",
        default_value_t = 1.0,
        help = "Challenge bond amount"
    )]
    pub stake_bond: f64,
    #[arg(long, help = "Corrected value for source challenges")]
    pub corrected_value: Option<f64>,
}

#[derive(Debug, Args, Clone)]
pub struct OracleUpdateChallengeArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        value_name = "SOURCE_OR_NODE",
        help = "Terminal source pin SKU or node index"
    )]
    pub node: String,
    #[arg(
        long = "claim-id",
        value_name = "HEX_OR_DRAFT_ID",
        help = "Exact target update-claim id; pass a 32-byte hex id or the update draft id"
    )]
    pub claim_id: String,
    #[arg(
        long,
        value_name = "PUBKEY",
        help = "Original claimant address bound to the claimant-scoped update claim PDA"
    )]
    pub claimant: String,
    #[arg(long, help = "Why the revealed update is wrong")]
    pub reason: String,
    #[arg(long, help = "Wayback/archive URL for challenge evidence")]
    pub archive_url: String,
    #[arg(
        long = "corrected-value",
        value_name = "STATE",
        value_parser = clap::value_parser!(u64).range(1..=MAX_SAFE_JSON_INTEGER),
        help = "Positive backend-safe integer state; must differ from the revealed and current source states"
    )]
    pub corrected_value: u64,
    #[arg(
        long = "stake",
        alias = "stake-bond",
        value_name = "AMOUNT",
        default_value_t = 1,
        value_parser = clap::value_parser!(u64).range(1..=MAX_SAFE_JSON_INTEGER),
        help = "Positive backend-safe integer challenge bond amount"
    )]
    pub stake_bond: u64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleUpdateCommitArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        long = "source",
        alias = "source-id",
        value_name = "SOURCE_ID",
        help = "Oracle source id for the update commitment"
    )]
    pub source_id: String,
    #[arg(
        long = "claim-id",
        value_name = "HEX_OR_DRAFT_ID",
        help = "Claimant-scoped update claim id; pass a 32-byte hex id or local draft id"
    )]
    pub claim_id: String,
    #[arg(
        long = "commit-hash",
        value_name = "HEX32",
        help = "32-byte hidden update commit hash"
    )]
    pub commit_hash: String,
    #[arg(long, default_value_t = 1.0, help = "Update commitment stake amount")]
    pub stake: f64,
}

#[derive(Debug, Args, Clone)]
pub struct OracleUpdateRevealArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(
        long = "source",
        alias = "source-id",
        value_name = "SOURCE_ID",
        help = "Oracle source id for the update reveal"
    )]
    pub source_id: String,
    #[arg(
        long = "claim-id",
        value_name = "HEX_OR_DRAFT_ID",
        help = "Target update-claim id; pass a 32-byte hex id or a local update draft id"
    )]
    pub claim_id: String,
    #[arg(long = "prior-state", help = "Exact source state committed against")]
    pub prior_state: u64,
    #[arg(long, help = "Revealed source-local value")]
    pub raw_value: f64,
    #[arg(long, help = "Source observation timestamp")]
    pub timestamp: String,
    #[arg(long, help = "Wayback/archive URL for the revealed update")]
    pub archive_url: String,
    #[arg(
        long = "evidence-hash",
        value_name = "HEX32",
        help = "Expert override for the 32-byte evidence hash; omitted values are derived from the archive URL"
    )]
    pub evidence_hash: Option<String>,
    #[arg(
        long = "secret-salt",
        value_name = "HEX32",
        help = "Secret 32-byte salt committed by the claimant"
    )]
    pub secret_salt: String,
}

#[derive(Debug, Args, Clone)]
pub struct OracleUpdateExpireArgs {
    #[command(flatten)]
    pub common: OracleDraftCommonArgs,
    #[arg(long = "source", alias = "source-id", value_name = "SOURCE_ID")]
    pub source_id: String,
    #[arg(long = "claim-id", value_name = "HEX_OR_DRAFT_ID")]
    pub claim_id: String,
    #[arg(
        long,
        value_name = "PUBKEY",
        help = "Original claimant address bound to the claim PDA"
    )]
    pub claimant: String,
}

#[derive(Debug, Subcommand)]
pub enum OracleSourceCommand {
    #[command(about = "Draft a source proposal in the local oracle queue")]
    Propose {
        #[command(flatten)]
        draft: OracleSourceProposeArgs,
    },
    #[command(about = "Draft source backing/support in the local oracle queue")]
    Support {
        #[command(flatten)]
        draft: OracleSourceSupportArgs,
    },
    #[command(about = "Draft a source kill/challenge in the local oracle queue")]
    Challenge {
        #[command(flatten)]
        draft: OracleChallengeDraftArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OraclePrintsCommand {
    #[command(about = "Draft an evidence-backed opening claim for a frozen source")]
    Opening {
        #[command(flatten)]
        draft: OracleOpeningClaimArgs,
    },
    #[command(about = "Draft a challenge to one pending opening claim")]
    Challenge {
        #[command(flatten)]
        draft: OracleOpeningChallengeArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleUpdatesCommand {
    #[command(about = "Draft a hidden source-local update commitment")]
    Commit {
        #[command(flatten)]
        draft: OracleUpdateCommitArgs,
    },
    #[command(about = "Draft the reveal for a hidden source-local update commitment")]
    Reveal {
        #[command(flatten)]
        draft: OracleUpdateRevealArgs,
    },
    #[command(about = "Settle the bond of an unrevealed claim after its reveal deadline")]
    Expire {
        #[command(flatten)]
        draft: OracleUpdateExpireArgs,
    },
    #[command(about = "Draft a challenge to a source-local update claim")]
    Challenge {
        #[command(flatten)]
        draft: OracleUpdateChallengeArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleEmergencyCommand {
    #[command(about = "Draft a hidden emergency vote commit")]
    Commit {
        #[command(flatten)]
        draft: OracleEmergencyCommitArgs,
    },
    #[command(about = "Draft an emergency vote reveal")]
    Reveal {
        #[command(flatten)]
        draft: OracleEmergencyRevealArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleRewardsCommand {
    #[command(about = "Draft claiming one earned current USDC oracle reward")]
    Claim {
        #[command(flatten)]
        draft: OracleRewardClaimArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleStakesCommand {
    #[command(
        about = "Permissionlessly settle a terminal stake or bond; refund/slash is program-derived"
    )]
    Settle {
        #[command(flatten)]
        draft: OracleStakeSettleArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum OracleAmbaCommand {
    #[command(about = "Draft depositing AMBA into oracle voting custody")]
    Deposit {
        #[command(flatten)]
        draft: OracleAmbaCustodyArgs,
    },
    #[command(about = "Draft withdrawing available AMBA from oracle voting custody")]
    Withdraw {
        #[command(flatten)]
        draft: OracleAmbaCustodyArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
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
    #[command(about = "Show whether the managed Petri agent connection is healthy")]
    Status,
    #[command(about = "Enable the managed Petri agent connection")]
    Enable,
    #[command(about = "Diagnose and repair the managed Petri agent connection")]
    Repair,
    #[command(about = "Disable the managed Petri agent connection")]
    Disable,
    #[command(about = "Print the v0 MCP/tool manifest for agent wrappers")]
    Manifest,
}

#[derive(Debug, Args, Clone)]
pub struct LiquidityArgs {
    #[arg(long, value_name = "MARKET", help = "Current market id")]
    pub market: String,
    #[arg(long, value_name = "EXPIRY_ID", help = "Current expiry id")]
    pub expiry: String,
    #[arg(
        long = "position-nonce",
        value_name = "NONCE",
        value_parser = clap::value_parser!(u64),
        help = "Existing manager position nonce"
    )]
    pub position_nonce: u64,
    #[arg(
        long = "entry",
        value_name = "BIN:AMOUNT_A:AMOUNT_B:AMOUNT_C",
        required = true,
        action = ArgAction::Append,
        help = "Liquidity entry; repeat in strictly ascending bin order"
    )]
    pub entries: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum LiquidityActionValue {
    #[default]
    Add,
    Remove,
    #[value(name = "close-position")]
    ClosePosition,
}

impl LiquidityActionValue {
    pub fn cli_value(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::ClosePosition => "close-position",
        }
    }

    pub fn as_request_value(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::ClosePosition => "close_position",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum LiquidityCommand {
    #[command(
        about = "Prepare and review SDK-validated manager liquidity",
        long_about = "Prepare an exact manager-liquidity action through Lean and independently validate it with the pinned SDK. Review the operation, then approve it with 'petri operations execute <id> --yes'."
    )]
    Plan {
        #[arg(long, value_enum, help = "Manager action to preview")]
        action: LiquidityActionValue,
        #[command(flatten)]
        liquidity: LiquidityArgs,
    },
    #[command(about = "Prepare an add-liquidity action for separate approval")]
    Add {
        #[command(flatten)]
        liquidity: LiquidityArgs,
    },
    #[command(about = "Prepare a remove-liquidity action for separate approval")]
    Remove {
        #[command(flatten)]
        liquidity: LiquidityArgs,
    },
    #[command(
        name = "close-position",
        about = "Prepare a manager position close for separate approval"
    )]
    ClosePosition {
        #[command(flatten)]
        liquidity: LiquidityArgs,
    },
}
