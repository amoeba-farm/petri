//! Lab Bench state and feature composition.
//!
//! This module owns the aggregate state shared by the terminal runtime. Child
//! modules own effects, input routing, and feature-specific presentation while
//! remaining descendants so state does not need crate-wide visibility.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    env,
    hash::{Hash, Hasher},
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::Command as ProcessCommand,
    str::FromStr,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, TableState, Wrap,
    },
};
use serde_json::Value;
use solana_pubkey::Pubkey;

use crate::{
    attached_wallet::{self, AttachedWallet},
    backend::{BackendClient, CliError, array_at_key, string_at_key, value_at_key},
    chart,
    cli::{ChartArgs, ChartRangeValue, Cli},
    endpoints,
    gitbook::{
        self, GitbookGlossaryEntry, GitbookIndex, GitbookNavTarget, GitbookPage, MarkdownBlock,
    },
    guide,
    market_surface::{
        DishDetail, DishSummary, ExpirySummary, OptionKind, OptionQuote, compact_series_labels,
        detail_from_payload, dish_list_payload, extract_dish_summaries, format_decimal, format_usd,
        is_known_value, live_dish_snapshot_payload, market_price_context, market_status_label,
        normalize_probability, number_from_value, positive_quote, render_lab_overview,
        series_label_from_expiry, string_array,
    },
    mcp_setup,
    onchain::OnchainConfig,
    oracle_submissions::{self, OracleSubmissionDraft, OracleSubmissionField},
    oracle_tui::{DEFAULT_ORACLE_NODE_INDEX, OracleIndexTree, OracleNodeKind, RamxOracleNode},
    positions, solana_history, spread_oracle_plan, terminal_brand,
    terminal_keys::is_actionable_key_event,
    wallet_balance,
    wallet_terms::{self, WalletTermsStatus},
    workspace_update::{self, WorkspaceUpdateReport},
    writer_action_mask::WriterActionMask,
};

const HOME_SHORTCUT_HELP: &str = "Home/Alt+h";
const PANEL_SCROLL_HELP: &str = "PgUp/PgDn";
const PANEL_SCROLL_STEP: usize = 5;
const HELP_REFRESH_TTL_SECONDS: u64 = 10 * 60;
const HELP_PAGE_CACHE_LIMIT: usize = 12;
const HELP_PREVIEW_DWELL_TICKS: usize = 3;
const HELP_HOVER_EXIT_GRACE_TICKS: usize = 5;
const HELP_PREVIEW_MIN_WIDTH: u16 = 28;
const HELP_PREVIEW_MAX_WIDTH: u16 = 64;
const HELP_PREVIEW_MIN_HEIGHT: u16 = 7;
const HELP_PREVIEW_MAX_HEIGHT: u16 = 18;
const GLOSSARY_PREVIEW_MIN_WIDTH: u16 = 30;
const GLOSSARY_PREVIEW_MAX_WIDTH: u16 = 56;
const PETRI_TUI_REDUCED_MOTION_ENV: &str = "PETRI_TUI_REDUCED_MOTION";
const ORACLE_TREE_RETRY_TICKS: usize = 30;
const DEFAULT_DOCS_URL: &str = "https://docs.amoeba.farm";
const PETRI_UPDATE_CHECK_ENV: &str = "PETRI_UPDATE_CHECK";
pub const TUI_UPDATE_REQUESTED_EXIT_CODE: i32 = 42;
pub const TUI_REBUILD_REQUESTED_EXIT_CODE: i32 = 43;
const HEADER_TICKER_LINES: u16 = 1;
const HEADER_TICKER_GUTTER: &str = "  ";
const HEADER_MIN_BODY_WITH_BRAND: u16 = 15;
const HEADER_MIN_BODY_WITH_COMPACT_BRAND: u16 = 8;
const HEADER_COMPACT_WALLET_LINES_MAX: u16 = 3;
const LAB_VISUAL_TICK_INTERVAL_MS: u64 = 200;
const LAB_MAX_VISUAL_TICKS_PER_FRAME: u128 = 8;
const TERMINAL_SIZE_WARNING_ATTENTION_TICKS: usize = 8;
const TRADE_RESULT_MODAL_SECONDS: u64 = 5;
const TICKER_SCROLL_TICKS_PER_COLUMN: usize = 1;
const TICKER_SEPARATOR: &str = "   |   ";
const TUI_BACKGROUND: Color = Color::Rgb(7, 21, 48);
const TUI_TICKER_BACKGROUND: Color = Color::Rgb(10, 27, 50);
const TUI_PANEL_BACKGROUND: Color = Color::Rgb(13, 30, 52);
const TUI_PANEL_BACKGROUND_ALT: Color = Color::Rgb(17, 39, 68);
const TUI_BORDER_BLUE: Color = Color::Rgb(45, 93, 218);
const TUI_BORDER_MUTED: Color = Color::Rgb(55, 75, 102);
const TUI_FIELD_BACKGROUND: Color = Color::Rgb(16, 36, 64);
const TUI_FIELD_ACTIVE_BACKGROUND: Color = Color::Rgb(38, 76, 136);
const TUI_FIELD_FLASH_BACKGROUND: Color = Color::LightYellow;
const TUI_NAV_BUTTON_BROWN: Color = Color::Rgb(126, 78, 38);
const HEADER_BACK_BUTTON_LABEL: &str = " < back ";
const HEADER_HOME_BUTTON_LABEL: &str = " home ";
const SELECTED_CONTRACT_ACTION_BUTTON_MAX_WIDTH: u16 = 20;
const ORDER_TICKET_COMPACT_WIDTH: u16 = 60;
const SCREEN_HISTORY_LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LabScreen {
    Terms,
    Home,
    Staking,
    Chain,
    Chart,
    OracleIntro,
    Oracle,
    OracleHelp,
    Help,
    Detail,
    Activity,
    Ledger,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OracleView {
    Earn,
    #[default]
    Advanced,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LedgerView {
    #[default]
    Account,
    Positions,
    Writers,
    History,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LedgerPane {
    #[default]
    Tabs,
    List,
    Actions,
    Detail,
}

impl LedgerPane {
    const ALL: [Self; 4] = [Self::Tabs, Self::List, Self::Actions, Self::Detail];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccountAction {
    SwitchWallet,
}

impl AccountAction {
    const ALL: [Self; 1] = [Self::SwitchWallet];

    fn label(self) -> &'static str {
        match self {
            Self::SwitchWallet => "Switch wallet",
        }
    }
}

impl LedgerView {
    const ALL: [Self; 4] = [Self::Account, Self::Positions, Self::Writers, Self::History];

    fn label(self) -> &'static str {
        match self {
            Self::Account => "Account",
            Self::Positions => "Liquidity",
            Self::Writers => "Writers",
            Self::History => "History",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DetailView {
    #[default]
    Overview,
    Settlement,
}

impl DetailView {
    const ALL: [Self; 2] = [Self::Overview, Self::Settlement];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Settlement => "Settlement",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeAction {
    Trade,
    Chart,
    Oracle,
    Ledger,
    Staking,
    Help,
    ConnectAgents,
}

impl HomeAction {
    const ALL: [Self; 7] = [
        Self::Trade,
        Self::Chart,
        Self::Oracle,
        Self::Ledger,
        Self::Staking,
        Self::Help,
        Self::ConnectAgents,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Trade => "Trade options",
            Self::Chart => "View chart",
            Self::Oracle => "Oracle work & evidence",
            Self::Ledger => "Wallet ledger",
            Self::Staking => "Staking",
            Self::Help => "Help",
            Self::ConnectAgents => "Connect your AI agent",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Trade => "Browse fixed-risk monthly contracts.",
            Self::Chart => "Fair price and liquidity history.",
            Self::Oracle => "Earn from verifiable work or inspect settlement evidence.",
            Self::Ledger => "Recent wallet activity and Amoeba trades.",
            Self::Staking => "Queue or activate AMBA, hold sAMBA, or finish unstaking.",
            Self::Help => "Amoeba overview, docs, terms, and product map.",
            Self::ConnectAgents => {
                "Use Petri with Claude Code, Codex, Gemini, and other supported agents."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StakingAction {
    Stake,
    Activate,
    CancelQueue,
    Unstake,
    Claim,
    Refresh,
}

impl StakingAction {
    const ALL: [Self; 6] = [
        Self::Stake,
        Self::Activate,
        Self::CancelQueue,
        Self::Unstake,
        Self::Claim,
        Self::Refresh,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Stake => "Queue AMBA",
            Self::Activate => "Activate queued AMBA",
            Self::CancelQueue => "Cancel queued AMBA",
            Self::Unstake => "Unstake sAMBA",
            Self::Claim => "Claim ready AMBA",
            Self::Refresh => "Refresh",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Stake => "Begin the seven-day wait before sAMBA can be minted.",
            Self::Activate => "Mint sAMBA at the current share rate after the queue matures.",
            Self::CancelQueue => "Return queued AMBA to the available balance without minting.",
            Self::Unstake => "Burn sAMBA and start the seven-day unstaking wait.",
            Self::Claim => "Move ready unstaked AMBA back into your available balance.",
            Self::Refresh => "Read the latest staking balances and exchange rate.",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Stake => Color::Green,
            Self::Activate => Color::Cyan,
            Self::CancelQueue => Color::Yellow,
            Self::Unstake => Color::Magenta,
            Self::Claim => Color::Cyan,
            Self::Refresh => Color::Yellow,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StakingFormField {
    Amount,
    MinimumReceived,
}

impl StakingFormField {
    fn label(self, action: StakingAction) -> &'static str {
        match (self, action) {
            (Self::Amount, StakingAction::Stake) => "AMBA to queue",
            (Self::Amount, StakingAction::Unstake) => "sAMBA to unstake",
            (Self::MinimumReceived, StakingAction::Activate) => "Minimum sAMBA received",
            (Self::MinimumReceived, StakingAction::Unstake) => "Minimum AMBA received",
            _ => "Amount",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StakingForm {
    action: StakingAction,
    field: StakingFormField,
    amount_input: String,
    minimum_received_input: String,
}

impl StakingForm {
    fn new(action: StakingAction) -> Self {
        let field = if action == StakingAction::Activate {
            StakingFormField::MinimumReceived
        } else {
            StakingFormField::Amount
        };
        Self {
            action,
            field,
            amount_input: String::new(),
            minimum_received_input: String::new(),
        }
    }

    fn fields(&self) -> &'static [StakingFormField] {
        match self.action {
            StakingAction::Stake => &[StakingFormField::Amount],
            StakingAction::Activate => &[StakingFormField::MinimumReceived],
            StakingAction::Unstake => {
                &[StakingFormField::Amount, StakingFormField::MinimumReceived]
            }
            StakingAction::CancelQueue | StakingAction::Claim | StakingAction::Refresh => &[],
        }
    }

    fn selected_input_mut(&mut self) -> &mut String {
        match self.field {
            StakingFormField::Amount => &mut self.amount_input,
            StakingFormField::MinimumReceived => &mut self.minimum_received_input,
        }
    }

    fn move_field(&mut self) {
        let fields = self.fields();
        if fields.len() < 2 {
            return;
        }
        let index = fields
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        self.field = fields[(index + 1) % fields.len()];
    }

    fn validate(&self) -> Result<(), String> {
        if matches!(self.action, StakingAction::Stake | StakingAction::Unstake) {
            validate_staking_decimal("Amount", &self.amount_input, false)?;
        }
        if matches!(
            self.action,
            StakingAction::Activate | StakingAction::Unstake
        ) && !self.minimum_received_input.trim().is_empty()
        {
            validate_staking_decimal("Minimum received", &self.minimum_received_input, true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StakingConfirmationChoice {
    Cancel,
    Confirm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StakingConfirmation {
    action: StakingAction,
    amount: Option<String>,
    minimum_received: Option<String>,
    choice: StakingConfirmationChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StakingActionResult {
    action: StakingAction,
    ok: bool,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterAction {
    Show,
    Policy,
    Deposit,
    // Historical bid handling remains available without a new-bid menu entry.
    #[allow(dead_code)]
    Bid,
    Withdraw,
    Refunds,
    Refund,
    Liquidity,
    LiquidityInitialize,
    LiquidityAdd,
    LiquidityRemove,
    LiquiditySweep,
    ClosePreview,
    BeginClose,
    CloseStatus,
    AdvanceClose,
    CancelClose,
    ClaimLong,
    ClaimFlat,
    TransferFlat,
    Refresh,
}

impl WriterAction {
    const ALL: [Self; 20] = [
        Self::Show,
        Self::Policy,
        Self::Deposit,
        Self::Withdraw,
        Self::Liquidity,
        Self::LiquidityInitialize,
        Self::LiquidityAdd,
        Self::LiquidityRemove,
        Self::LiquiditySweep,
        Self::Refunds,
        Self::Refund,
        Self::ClosePreview,
        Self::BeginClose,
        Self::CloseStatus,
        Self::AdvanceClose,
        Self::CancelClose,
        Self::ClaimLong,
        Self::ClaimFlat,
        Self::TransferFlat,
        Self::Refresh,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Show => "Sleeve detail",
            Self::Policy => "Policy audit",
            Self::Deposit => "Deposit principal",
            Self::Bid => "Auction bid",
            Self::Withdraw => "Withdraw principal",
            Self::Refunds => "Historical refunds",
            Self::Refund => "Refund a historical bid",
            Self::Liquidity => "Writer liquidity",
            Self::LiquidityInitialize => "Initialize writer liquidity",
            Self::LiquidityAdd => "Add writer liquidity",
            Self::LiquidityRemove => "Remove writer liquidity",
            Self::LiquiditySweep => "Sweep writer proceeds",
            Self::ClosePreview => "Preview close",
            Self::BeginClose => "Start close",
            Self::CloseStatus => "Close status",
            Self::AdvanceClose => "Advance close",
            Self::CancelClose => "Cancel close",
            Self::ClaimLong => "Claim long",
            Self::ClaimFlat => "Claim Flat residual",
            Self::TransferFlat => "Transfer Flat",
            Self::Refresh => "Refresh catalog",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Show => "Inspect the selected global sleeve and its current staged state.",
            Self::Policy => "Inspect the frozen policy identity and audit hashes.",
            Self::Deposit => "Fund writer principal and receive the current Flat issuance.",
            Self::Bid => "Submit a funded bid to one of the 20 current writer series.",
            Self::Withdraw => "Withdraw eligible principal from the canonical sleeve cash vault.",
            Self::Refunds => {
                "Discover one owner's historical bids independently of active auctions."
            }
            Self::Refund => "Refund the exact historical bid to its canonical refund account.",
            Self::Liquidity => {
                "Read custody, inventory, reserves, and frozen buyback budgets for one series."
            }
            Self::LiquidityInitialize => {
                "The frozen manager initializes the sleeve-owned lane in the canonical pool."
            }
            Self::LiquidityAdd => {
                "The frozen manager places backed options and bounded quote bids; Flat ownership grants no management permission."
            }
            Self::LiquidityRemove => {
                "Return quote to sleeve cash and burn removed unsold options. No assets go to the manager."
            }
            Self::LiquiditySweep => {
                "Return uncommitted writer proceeds to the canonical sleeve cash vault."
            }
            Self::ClosePreview => "Read the verified basket, reserve change, and safe withdrawal.",
            Self::BeginClose => "Start only the first stage of a writer close.",
            Self::CloseStatus => "Inspect progress and the next permitted close stage.",
            Self::AdvanceClose => {
                "Submit the one basket-deposit or finalization stage Lean selects now."
            }
            Self::CancelClose => {
                "Unavailable: the current wallet-action ABI has no close-cancel kind."
            }
            Self::ClaimLong => {
                "Claim settled collective long tokens for the exact selected series."
            }
            Self::ClaimFlat => "Claim the settled residual belonging to Flat holders.",
            Self::TransferFlat => "Transfer exact Flat atoms to another wallet.",
            Self::Refresh => "Reload the current global sleeve catalog.",
        }
    }

    fn signs_and_submits(self) -> bool {
        matches!(
            self,
            Self::Deposit
                | Self::Withdraw
                | Self::Refund
                | Self::LiquidityInitialize
                | Self::LiquidityAdd
                | Self::LiquidityRemove
                | Self::LiquiditySweep
                | Self::Bid
                | Self::BeginClose
                | Self::AdvanceClose
                | Self::CancelClose
                | Self::ClaimLong
                | Self::ClaimFlat
                | Self::TransferFlat
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WriterCloseCapabilityRoute {
    method: String,
    path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WriterCloseCapabilityProjection {
    implementation_supported: bool,
    runtime_enabled: bool,
    hot_runtime_enabled: bool,
    cold_runtime_enabled: bool,
    operations: Vec<String>,
    routes: Vec<WriterCloseCapabilityRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WriterCloseCapabilityState {
    Checking,
    Ready(WriterCloseCapabilityProjection),
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WriterActionAvailability {
    Enabled,
    HotOnly(String),
    Disabled(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActionFormField {
    key: &'static str,
    label: &'static str,
    value: String,
    required: bool,
    secret: bool,
}

impl ActionFormField {
    fn new(key: &'static str, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            key,
            label,
            value: value.into(),
            required: true,
            secret: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WriterActionForm {
    action: WriterAction,
    fields: Vec<ActionFormField>,
    selected_field: usize,
}

impl WriterActionForm {
    fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.trim())
    }

    fn selected_field_mut(&mut self) -> Option<&mut ActionFormField> {
        self.fields.get_mut(self.selected_field)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UserActionConfirmationChoice {
    Cancel,
    Confirm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfirmationMousePress {
    Trade(TradeConfirmationChoice),
    Writer(UserActionConfirmationChoice),
    Staking(StakingConfirmationChoice),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WriterActionConfirmation {
    action: WriterAction,
    args: Vec<String>,
    summary: Vec<String>,
    choice: UserActionConfirmationChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingWriterReview {
    owner: String,
    sleeve: String,
    form: WriterActionForm,
    confirmation: WriterActionConfirmation,
}

#[derive(Clone, Debug)]
struct WriterActionResult {
    action: WriterAction,
    ok: bool,
    payload: Option<Value>,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeHelpTopic {
    Overview,
    Agents,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum HelpPane {
    #[default]
    Navigation,
    Article,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HelpPreviewOrigin {
    Hover,
    Keyboard,
}

#[derive(Clone, Debug)]
struct GitbookHelpPreview {
    nav_index: usize,
    origin: HelpPreviewOrigin,
    scroll: usize,
    reveal_tick: usize,
    page: Option<GitbookPage>,
}

#[derive(Clone, Debug)]
struct GitbookGlossaryHover {
    term: String,
    definition: String,
    anchor: Rect,
}

#[derive(Clone, Copy, Debug)]
struct GitbookGlossaryMatch<'a> {
    start: usize,
    end: usize,
    entry: &'a GitbookGlossaryEntry,
}

#[derive(Clone, Debug)]
struct GitbookGlossaryHit {
    entry: GitbookGlossaryEntry,
    anchor: Rect,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum McpConnectionState {
    #[default]
    Disabled,
    Enabled,
    NeedsRepair,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpConnectionAction {
    Enable,
    Disable,
    Repair,
    Blocked,
}

impl McpConnectionState {
    fn from_setup_status(status: &mcp_setup::McpSetupStatus) -> Self {
        if status.has_conflict() {
            Self::Conflict
        } else if status.is_fully_enabled() {
            Self::Enabled
        } else if status.needs_repair() || status.is_partially_enabled() {
            Self::NeedsRepair
        } else {
            Self::Disabled
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderNavAction {
    Back,
    Home,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TermsAction {
    OpenTerms,
    SwitchWallet,
    Accept,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalLinkTarget {
    Docs,
    Terms,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleIntroAction {
    Earn,
    Advanced,
    ReadMore,
    BackHome,
}

impl OracleIntroAction {
    const ALL: [Self; 4] = [Self::Earn, Self::Advanced, Self::ReadMore, Self::BackHome];

    fn label(self) -> &'static str {
        match self {
            Self::Earn => "Earn — funded rewards",
            Self::Advanced => "Advanced view",
            Self::ReadMore => "How it works",
            Self::BackHome => "Exit to home",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Earn => "See exact rewards for the selected market and month.",
            Self::Advanced => {
                "Open the source tree, phase timeline, evidence tools, and manual actions."
            }
            Self::ReadMore => {
                "Learn the phases, settlement rule, evidence rules, and contributor actions."
            }
            Self::BackHome => "Return to the market home without opening the oracle.",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Earn => Color::Green,
            Self::Advanced => Color::Magenta,
            Self::ReadMore => Color::Cyan,
            Self::BackHome => Color::Yellow,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleAction {
    ProposeSource,
    EditDefinition,
    BackSource,
    OpeningPrint,
    SubmitUpdate,
    Challenge,
    ClaimReward,
    SettleStake,
    DepositAmba,
    WithdrawAmba,
    ReviewQueue,
}

impl OracleAction {
    const ALL: [Self; 9] = [
        Self::ProposeSource,
        Self::EditDefinition,
        Self::BackSource,
        Self::OpeningPrint,
        Self::SubmitUpdate,
        Self::Challenge,
        Self::DepositAmba,
        Self::WithdrawAmba,
        Self::ReviewQueue,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ProposeSource => "Propose source",
            Self::EditDefinition => "Edit definition",
            Self::BackSource => "Back source",
            Self::OpeningPrint => "Opening print",
            Self::SubmitUpdate => "Commit update",
            Self::Challenge => "Challenge",
            Self::ClaimReward => "Claim reward",
            Self::SettleStake => "Settle stake",
            Self::DepositAmba => "Deposit AMBA",
            Self::WithdrawAmba => "Withdraw AMBA",
            Self::ReviewQueue => "Review work queue",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::ProposeSource => "Propose source",
            Self::EditDefinition => "Edit definition (local draft)",
            Self::BackSource => "Back source",
            Self::OpeningPrint => "Opening print",
            Self::SubmitUpdate => "Commit update",
            Self::Challenge => "Challenge",
            Self::ClaimReward => "Claim reward",
            Self::SettleStake => "Settle stake (local draft)",
            Self::DepositAmba => "Deposit AMBA (local draft)",
            Self::WithdrawAmba => "Withdraw AMBA (local draft)",
            Self::ReviewQueue => "Review work queue (local drafts)",
        }
    }

    fn allowed(self, phase: OraclePhase) -> bool {
        if matches!(
            self,
            Self::ClaimReward | Self::SettleStake | Self::DepositAmba | Self::WithdrawAmba
        ) {
            return true;
        }
        match phase {
            OraclePhase::Unavailable
            | OraclePhase::Upcoming
            | OraclePhase::SourceSubmission
            | OraclePhase::Scramble => {
                matches!(self, Self::ReviewQueue)
            }
            OraclePhase::Placement => matches!(
                self,
                Self::ProposeSource | Self::EditDefinition | Self::BackSource | Self::ReviewQueue
            ),
            OraclePhase::KillChallenge => matches!(self, Self::Challenge | Self::ReviewQueue),
            OraclePhase::ResolutionFreeze => matches!(self, Self::ReviewQueue),
            OraclePhase::OpeningPrint => {
                matches!(
                    self,
                    Self::OpeningPrint | Self::Challenge | Self::ReviewQueue
                )
            }
            OraclePhase::GameMode => matches!(
                self,
                Self::SubmitUpdate | Self::Challenge | Self::ReviewQueue
            ),
            OraclePhase::MonthClose => matches!(self, Self::ReviewQueue),
        }
    }

    fn base_availability(
        self,
        phase: OraclePhase,
        node_kind: OracleNodeKind,
    ) -> OracleActionAvailability {
        if !self.allowed(phase) {
            return OracleActionAvailability::Locked;
        }
        match self {
            Self::ClaimReward
            | Self::SettleStake
            | Self::DepositAmba
            | Self::WithdrawAmba
            | Self::ReviewQueue => OracleActionAvailability::Active,
            Self::BackSource
                if matches!(
                    node_kind,
                    OracleNodeKind::RowBucket | OracleNodeKind::TerminalPin
                ) =>
            {
                OracleActionAvailability::Active
            }
            Self::BackSource => OracleActionAvailability::ChooseSource,
            Self::EditDefinition
            | Self::ProposeSource
            | Self::OpeningPrint
            | Self::SubmitUpdate
            | Self::Challenge
                if node_kind == OracleNodeKind::TerminalPin =>
            {
                OracleActionAvailability::Active
            }
            Self::EditDefinition
            | Self::ProposeSource
            | Self::OpeningPrint
            | Self::SubmitUpdate
            | Self::Challenge => OracleActionAvailability::ChooseSource,
        }
    }

    fn availability(self, context: OracleActionContext) -> OracleActionAvailability {
        let base = self.base_availability(context.phase, context.node_kind);
        if base != OracleActionAvailability::Active {
            return base;
        }
        match self {
            Self::OpeningPrint if !context.source.opening_status.can_submit() => {
                OracleActionAvailability::Locked
            }
            Self::SubmitUpdate
                if context.source.opening_status != OpeningClaimViewStatus::Accepted =>
            {
                OracleActionAvailability::Locked
            }
            Self::Challenge
                if context.phase == OraclePhase::OpeningPrint
                    && context.source.opening_status != OpeningClaimViewStatus::Pending =>
            {
                OracleActionAvailability::Locked
            }
            Self::Challenge
                if context.source.has_unresolved_challenge
                    || context.source.has_active_emergency =>
            {
                OracleActionAvailability::Locked
            }
            _ => OracleActionAvailability::Active,
        }
    }

    fn detail(self, phase: OraclePhase) -> &'static str {
        match (self, phase) {
            (Self::ProposeSource, OraclePhase::Placement) => {
                "source category, canonical locator, source definition, row target, stake/support"
            }
            (Self::ProposeSource, _) => "source proposals belong to placement",
            (Self::EditDefinition, OraclePhase::Placement) => {
                "edit the definition before the placement snapshot"
            }
            (Self::EditDefinition, _) => "definition edits close after placement snapshot",
            (Self::BackSource, OraclePhase::Placement) => {
                "add support to a candidate source before weights freeze"
            }
            (Self::BackSource, _) => "support changes close after placement",
            (Self::OpeningPrint, OraclePhase::OpeningPrint) => {
                "raw value, timestamp, source definition, archive evidence, and stake"
            }
            (Self::OpeningPrint, _) => "opening prints wait for the frozen source map",
            (Self::SubmitUpdate, OraclePhase::GameMode) => {
                "submit only when a source value changes; include archive evidence"
            }
            (Self::SubmitUpdate, OraclePhase::MonthClose) => {
                "closed after the live update window; review final output"
            }
            (Self::SubmitUpdate, _) => "updates open during Game Mode after opening prints",
            (Self::Challenge, OraclePhase::KillChallenge) => {
                "invalid, duplicate, wrong bucket, wrong definition, or disallowed source"
            }
            (Self::Challenge, OraclePhase::OpeningPrint) => {
                "challenge a wrong opening print with corrected archive evidence"
            }
            (Self::Challenge, OraclePhase::GameMode) => {
                "challenge a bad update before finalization"
            }
            (Self::Challenge, _) => "challenge window is closed for this phase",
            (Self::ClaimReward, _) => "claim an earned treasury reward",
            (Self::SettleStake, _) => {
                "settle one terminal stake or bond; refund or slash is derived on-chain"
            }
            (Self::DepositAmba, _) => "move AMBA into oracle voting custody",
            (Self::WithdrawAmba, _) => "move available AMBA back to your wallet",
            (Self::ReviewQueue, OraclePhase::MonthClose) => {
                "final accepted states and oracle output"
            }
            (Self::ReviewQueue, _) => "phase status, current node, and pending work",
        }
    }

    fn contextual_detail(self, context: OracleActionContext) -> &'static str {
        match self {
            Self::OpeningPrint if !context.source.opening_status.can_submit() => {
                context.source.opening_status.display_label()
            }
            Self::SubmitUpdate
                if context.source.opening_status != OpeningClaimViewStatus::Accepted =>
            {
                "live updates require an accepted opening claim"
            }
            Self::Challenge
                if context.phase == OraclePhase::OpeningPrint
                    && context.source.opening_status != OpeningClaimViewStatus::Pending =>
            {
                "requires one pending opening claim"
            }
            Self::Challenge if context.source.has_active_emergency => {
                "emergency voting is already active for this source"
            }
            Self::Challenge if context.source.has_unresolved_challenge => {
                "challenge already queued; awaiting resolution"
            }
            _ => self.detail(context.phase),
        }
    }

    fn state(self, context: OracleActionContext) -> &'static str {
        match self {
            Self::ReviewQueue if self.allowed(context.phase) => "view",
            _ => self.availability(context).label(),
        }
    }

    fn color(self) -> Color {
        match self {
            Self::ProposeSource => Color::Yellow,
            Self::EditDefinition => Color::Cyan,
            Self::BackSource => Color::Blue,
            Self::OpeningPrint => Color::Green,
            Self::SubmitUpdate => Color::Green,
            Self::Challenge => Color::Red,
            Self::ClaimReward => Color::Green,
            Self::SettleStake => Color::LightGreen,
            Self::DepositAmba => Color::Magenta,
            Self::WithdrawAmba => Color::Blue,
            Self::ReviewQueue => Color::Magenta,
        }
    }
}

const ORACLE_LOCK_FLASH_TICKS: u8 = 6;
const ORACLE_FORM_FIELD_FLASH_TICKS: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OracleLockedFlash {
    action: OracleAction,
    ticks_remaining: u8,
    visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OracleFormFieldFlash {
    field_index: usize,
    ticks_remaining: u8,
    visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OraclePhase {
    Unavailable,
    Upcoming,
    SourceSubmission,
    Scramble,
    Placement,
    KillChallenge,
    ResolutionFreeze,
    OpeningPrint,
    GameMode,
    MonthClose,
}

impl OraclePhase {
    const ALL: [Self; 7] = [
        Self::SourceSubmission,
        Self::Placement,
        Self::KillChallenge,
        Self::ResolutionFreeze,
        Self::OpeningPrint,
        Self::GameMode,
        Self::MonthClose,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Lifecycle unavailable",
            Self::Upcoming => "Upcoming",
            Self::SourceSubmission => "Source Submission",
            Self::Scramble => "Scramble (pre-listing)",
            Self::Placement => "Placement",
            Self::KillChallenge => "Source Challenges",
            Self::ResolutionFreeze => "Source Freeze",
            Self::OpeningPrint => "Opening Prints",
            Self::GameMode => "Game Mode",
            Self::MonthClose => "Settlement",
        }
    }

    fn spread_label(self) -> &'static str {
        match self {
            Self::Unavailable => "Lifecycle unavailable",
            Self::Upcoming => "Upcoming",
            Self::SourceSubmission => "Source Submission",
            Self::Scramble => "Scramble",
            Self::Placement => "Placement",
            Self::KillChallenge => "Kill Challenge",
            Self::ResolutionFreeze => "Source Freeze",
            Self::OpeningPrint => "Opening Print",
            Self::GameMode => "Game Mode",
            Self::MonthClose => "Settlement",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Unavailable => "on-chain lifecycle has not been verified for this month",
            Self::Upcoming => "Scramble has not started for this future month",
            Self::SourceSubmission => {
                "complete terminal-SKU source coverage before the placement schedule can begin"
            }
            Self::Scramble => "build and freeze the source map before contracts are listed",
            Self::Placement => "add or back public sources before the month starts",
            Self::KillChallenge => "challenge bad source definitions before freeze",
            Self::ResolutionFreeze => "resolve challenges and freeze this month's source map",
            Self::OpeningPrint => "review and accept starting values with archive evidence",
            Self::GameMode => "live source updates after opening claims are accepted",
            Self::MonthClose => "final accepted values settle the month",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Unavailable => Color::DarkGray,
            Self::Upcoming => Color::Gray,
            Self::SourceSubmission => Color::LightCyan,
            Self::Scramble => Color::LightYellow,
            Self::Placement => Color::Cyan,
            Self::KillChallenge => Color::Red,
            Self::ResolutionFreeze => Color::Yellow,
            Self::OpeningPrint => Color::Green,
            Self::GameMode => Color::Blue,
            Self::MonthClose => Color::Magenta,
        }
    }

    fn active_color(self) -> Color {
        match self {
            Self::Unavailable => Color::Gray,
            Self::Upcoming => Color::White,
            Self::SourceSubmission => Color::Cyan,
            Self::Scramble => Color::Yellow,
            Self::Placement => Color::LightCyan,
            Self::KillChallenge => Color::LightRed,
            Self::ResolutionFreeze => Color::LightYellow,
            Self::OpeningPrint => Color::LightGreen,
            Self::GameMode => Color::LightBlue,
            Self::MonthClose => Color::LightMagenta,
        }
    }
}

fn oracle_phase_from_spread_label(label: &str) -> Option<OraclePhase> {
    match label.trim().to_ascii_lowercase().as_str() {
        "0" | "uninitialized" => Some(OraclePhase::Unavailable),
        "upcoming" | "pre_scramble" | "pre-scramble" => Some(OraclePhase::Upcoming),
        "6" | "source_submission" | "source-submission" => Some(OraclePhase::SourceSubmission),
        "1" | "scramble" => Some(OraclePhase::Scramble),
        "placement" | "source_selection" => Some(OraclePhase::Placement),
        "kill_challenge" | "source_challenge" => Some(OraclePhase::KillChallenge),
        "resolution_freeze" | "source_freeze" => Some(OraclePhase::ResolutionFreeze),
        "5" | "opening" | "opening_print" => Some(OraclePhase::OpeningPrint),
        "2" | "game" | "game_mode" | "live_updates" => Some(OraclePhase::GameMode),
        "3" | "settled" | "4" | "closed" => Some(OraclePhase::MonthClose),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleSourceState {
    Placed,
    Snapshotted,
    Challenged,
    Frozen,
    OpeningPending,
    Active,
    MonthClosed,
}

impl OracleSourceState {
    fn label(self) -> &'static str {
        match self {
            Self::Placed => "PLACED",
            Self::Snapshotted => "SNAPSHOTTED",
            Self::Challenged => "CHALLENGED",
            Self::Frozen => "FROZEN",
            Self::OpeningPending => "OPENING_PENDING",
            Self::Active => "ACTIVE",
            Self::MonthClosed => "MONTH_CLOSED",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Placed => "candidate source",
            Self::Snapshotted => "snapshot locked",
            Self::Challenged => "challenged",
            Self::Frozen => "frozen for month",
            Self::OpeningPending => "needs opening value",
            Self::Active => "live updates",
            Self::MonthClosed => "month closed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleUpdateState {
    Submitted,
    Challenged,
    Finalized,
    Corrected,
    Rejected,
    Court,
}

impl OracleUpdateState {
    fn label(self) -> &'static str {
        match self {
            Self::Submitted => "SUBMITTED",
            Self::Challenged => "CHALLENGED",
            Self::Finalized => "FINALIZED",
            Self::Corrected => "CORRECTED",
            Self::Rejected => "REJECTED",
            Self::Court => "COURT",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Challenged => "challenged",
            Self::Finalized => "finalized",
            Self::Corrected => "corrected",
            Self::Rejected => "rejected",
            Self::Court => "court review",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OpeningClaimViewStatus {
    #[default]
    Missing,
    Pending,
    Challenged,
    Accepted,
    RejectedRetryable,
    Inactive,
    SubmittedUnknown,
}

impl OpeningClaimViewStatus {
    fn from_label(label: &str, opening_submitted: bool, source_status: &str) -> Self {
        let normalized = label.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "1" | "pending" | "opening_pending" => Self::Pending,
            "2" | "challenged" | "opening_challenged" => Self::Challenged,
            "3" | "accepted" | "active" | "opening_accepted" => Self::Accepted,
            "4" | "rejected" | "rejected_retryable" | "retry" => Self::RejectedRetryable,
            "inactive" | "source_inactive" => Self::Inactive,
            "" | "0" | "empty" | "missing" => {
                match source_status.trim().to_ascii_lowercase().as_str() {
                    "2" | "inactive" => Self::Inactive,
                    "4" | "opening_pending" => Self::Pending,
                    "5" | "active" => Self::Accepted,
                    _ if opening_submitted => Self::SubmittedUnknown,
                    _ => Self::Missing,
                }
            }
            _ if opening_submitted => Self::SubmittedUnknown,
            _ => Self::Missing,
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Missing => "opening missing",
            Self::Pending => "opening pending; challenge window open",
            Self::Challenged => "opening challenged; awaiting resolution",
            Self::Accepted => "opening accepted",
            Self::RejectedRetryable => "opening rejected; replacement allowed",
            Self::Inactive => "inactive for this month",
            Self::SubmittedUnknown => "opening submitted; acceptance not confirmed",
        }
    }

    fn can_submit(self) -> bool {
        matches!(self, Self::Missing | Self::RejectedRetryable)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OracleSourceActionState {
    has_unresolved_challenge: bool,
    has_active_emergency: bool,
    opening_status: OpeningClaimViewStatus,
    opening_finalizable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OracleActionContext {
    phase: OraclePhase,
    node_kind: OracleNodeKind,
    source: OracleSourceActionState,
    market_expired: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleActionAvailability {
    Active,
    ChooseSource,
    Locked,
}

impl OracleActionAvailability {
    fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ChooseSource => "choose source",
            Self::Locked => "locked",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OracleFormMode {
    SourceProposal,
    DefinitionEdit,
    SourceSupport,
    OpeningPrint,
    UpdateClaim,
    Challenge,
    RewardClaim,
    StakeSettlement,
    AmbaDeposit,
    AmbaWithdraw,
}

impl OracleFormMode {
    fn from_action(action: OracleAction) -> Option<Self> {
        match action {
            OracleAction::ProposeSource => Some(Self::SourceProposal),
            OracleAction::EditDefinition => Some(Self::DefinitionEdit),
            OracleAction::BackSource => Some(Self::SourceSupport),
            OracleAction::OpeningPrint => Some(Self::OpeningPrint),
            OracleAction::SubmitUpdate => Some(Self::UpdateClaim),
            OracleAction::Challenge => Some(Self::Challenge),
            OracleAction::ClaimReward => Some(Self::RewardClaim),
            OracleAction::SettleStake => Some(Self::StakeSettlement),
            OracleAction::DepositAmba => Some(Self::AmbaDeposit),
            OracleAction::WithdrawAmba => Some(Self::AmbaWithdraw),
            OracleAction::ReviewQueue => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::SourceProposal => "Source proposal",
            Self::DefinitionEdit => "Edit source definition",
            Self::SourceSupport => "Back source",
            Self::OpeningPrint => "Opening print",
            Self::UpdateClaim => "Commit update claim",
            Self::Challenge => "Challenge",
            Self::RewardClaim => "Claim reward",
            Self::StakeSettlement => "Settle stake",
            Self::AmbaDeposit => "Deposit AMBA",
            Self::AmbaWithdraw => "Withdraw AMBA",
        }
    }

    fn modules(self) -> &'static str {
        match self {
            Self::SourceProposal | Self::DefinitionEdit => "source definition review",
            Self::SourceSupport => "source support",
            Self::OpeningPrint => "opening print evidence",
            Self::UpdateClaim => "hidden update commitment",
            Self::Challenge => "source challenge",
            Self::RewardClaim => "reward claim receipt",
            Self::StakeSettlement => "terminal stake settlement",
            Self::AmbaDeposit | Self::AmbaWithdraw => "AMBA voting custody",
        }
    }

    fn user_hint(self) -> &'static str {
        match self {
            Self::SourceProposal => {
                "Save a local semantic draft to add a public source; no instruction is prepared."
            }
            Self::DefinitionEdit => {
                "Save a local semantic draft for a source-definition fix before freeze."
            }
            Self::SourceSupport => {
                "Save a local semantic draft to back this source with stake/support."
            }
            Self::OpeningPrint => {
                "Save a local semantic draft of this source's opening value and evidence."
            }
            Self::UpdateClaim => "Save a local semantic update-commitment draft for later review.",
            Self::Challenge => "Save a local semantic challenge draft with exact evidence.",
            Self::RewardClaim => {
                "Save a local semantic reward-claim draft; nothing is claimed yet."
            }
            Self::StakeSettlement => {
                "Save a local semantic stake-settlement draft; nothing is settled yet."
            }
            Self::AmbaDeposit => "Save a local semantic AMBA-deposit draft; no tokens move yet.",
            Self::AmbaWithdraw => {
                "Save a local semantic AMBA-withdrawal draft; no tokens move yet."
            }
        }
    }

    fn spread_action(self) -> Option<&'static str> {
        match self {
            Self::SourceProposal => Some("Propose source"),
            Self::DefinitionEdit => None,
            Self::SourceSupport => Some("Back source"),
            Self::OpeningPrint => Some("Opening print"),
            Self::UpdateClaim => Some("Commit update claim v2"),
            Self::Challenge => Some("Challenge"),
            Self::RewardClaim => Some("Claim oracle reward"),
            Self::StakeSettlement => Some("Settle oracle stake"),
            Self::AmbaDeposit => Some("Deposit AMBA tokens"),
            Self::AmbaWithdraw => Some("Withdraw AMBA tokens"),
        }
    }

    fn source_state(self) -> OracleSourceState {
        match self {
            Self::SourceProposal | Self::DefinitionEdit | Self::SourceSupport => {
                OracleSourceState::Placed
            }
            Self::OpeningPrint => OracleSourceState::OpeningPending,
            Self::UpdateClaim => OracleSourceState::Active,
            Self::Challenge => OracleSourceState::Challenged,
            Self::RewardClaim | Self::StakeSettlement | Self::AmbaDeposit | Self::AmbaWithdraw => {
                OracleSourceState::Active
            }
        }
    }

    fn update_state(self) -> Option<OracleUpdateState> {
        match self {
            Self::UpdateClaim => Some(OracleUpdateState::Submitted),
            Self::Challenge => Some(OracleUpdateState::Challenged),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct OracleFormField {
    label: &'static str,
    value: String,
    required: bool,
    editable: bool,
}

impl OracleFormField {
    fn editable(label: &'static str, value: impl Into<String>, required: bool) -> Self {
        Self {
            label,
            value: value.into(),
            required,
            editable: true,
        }
    }

    fn readonly(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            required: false,
            editable: false,
        }
    }
}

#[derive(Clone, Debug)]
struct OracleFormDraft {
    mode: OracleFormMode,
    node_index: usize,
    field_selected: usize,
    fields: Vec<OracleFormField>,
}

impl OracleFormDraft {
    fn new(
        mode: OracleFormMode,
        node_index: usize,
        phase: OraclePhase,
        tree: &OracleIndexTree,
    ) -> Option<Self> {
        let row_label = tree
            .row_bucket_for(node_index)
            .and_then(|index| tree.node(index).map(|node| node.label.to_string()))
            .unwrap_or_else(|| tree.display_name.clone());
        let node = tree.node(node_index)?;
        let fields = match mode {
            OracleFormMode::SourceProposal => vec![
                OracleFormField::editable("Source category", "Retailer Product Page", true),
                OracleFormField::editable("Canonical locator", "", true),
                OracleFormField::editable("Source definition", "", true),
                OracleFormField::readonly("Product row", row_label),
                OracleFormField::editable("Stake/support", "1", true),
            ],
            OracleFormMode::DefinitionEdit => vec![
                OracleFormField::readonly("Source", node.label.as_str()),
                OracleFormField::editable("Canonical locator", "", true),
                OracleFormField::editable("Source definition", "", true),
                OracleFormField::editable("Edit note", "", false),
            ],
            OracleFormMode::SourceSupport => vec![
                OracleFormField::readonly("Product row/source", row_label),
                OracleFormField::editable("Stake/support", "1", true),
                OracleFormField::editable("Support note", "", false),
            ],
            OracleFormMode::OpeningPrint => vec![
                OracleFormField::readonly("Source", node.label.as_str()),
                OracleFormField::editable("Raw value", "", true),
                OracleFormField::editable("Timestamp", "", true),
                OracleFormField::editable("Canonical locator", "", true),
                OracleFormField::editable("Source definition", "", true),
                OracleFormField::editable("Wayback archive URL", "", true),
                OracleFormField::editable("Stake", "1", true),
            ],
            OracleFormMode::UpdateClaim => vec![
                OracleFormField::readonly("Source id", node.label.as_str()),
                OracleFormField::editable("Claim id", "", true),
                OracleFormField::editable("Commit hash", "", true),
                OracleFormField::editable("Stake", "1", true),
            ],
            OracleFormMode::Challenge => {
                let mut fields = vec![
                    OracleFormField::readonly("Target source", node.label.as_str()),
                    OracleFormField::editable("Challenge reason", "invalid source", true),
                    OracleFormField::editable(
                        "Corrected value",
                        "",
                        phase == OraclePhase::OpeningPrint,
                    ),
                ];
                if phase == OraclePhase::OpeningPrint {
                    fields.extend([
                        OracleFormField::editable("Timestamp", "", true),
                        OracleFormField::editable("Canonical locator", "", true),
                        OracleFormField::editable("Source definition", "", true),
                        OracleFormField::editable("Wayback archive URL", "", true),
                        OracleFormField::editable("Stake/bond", "1", true),
                    ]);
                } else if phase == OraclePhase::GameMode {
                    fields.extend([
                        OracleFormField::editable("Claim id", "", true),
                        OracleFormField::editable("Claimant", "", true),
                        OracleFormField::editable("Evidence / Archive Link", "", true),
                        OracleFormField::editable("Wayback archive URL", "", true),
                        OracleFormField::editable("Stake/bond", "1", true),
                    ]);
                } else {
                    fields.extend([
                        OracleFormField::editable("Comparison source", "", false),
                        OracleFormField::editable("Evidence / Archive Link", "", true),
                        OracleFormField::editable("Wayback archive URL", "", true),
                        OracleFormField::editable("Stake/bond", "1", true),
                    ]);
                }
                fields
            }
            OracleFormMode::RewardClaim => vec![
                OracleFormField::editable("Reward kind", "game_update", true),
                OracleFormField::readonly("Source id", node.label.as_str()),
                OracleFormField::editable("Claim id", "", false),
                OracleFormField::editable("Challenge id", "", false),
                OracleFormField::editable("Claim PDA", "", false),
                OracleFormField::editable("Subject PDA", "", false),
            ],
            OracleFormMode::StakeSettlement => vec![
                OracleFormField::readonly("Stake kind", ""),
                OracleFormField::readonly("Subject PDA", ""),
                OracleFormField::readonly("Owner", ""),
                OracleFormField::readonly("Amount", ""),
                OracleFormField::readonly("Terminal outcome", ""),
                OracleFormField::readonly("Disposition", ""),
                OracleFormField::readonly("Settlement eligibility", ""),
            ],
            OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw => vec![
                OracleFormField::editable("Amount", "1000000", true),
                OracleFormField::readonly("Mint", wallet_balance::resolve_wallet_amba_mint(None)),
                OracleFormField::editable("User token account", "", false),
                OracleFormField::readonly(
                    "Vault token account",
                    wallet_balance::resolve_amba_vault_token_account(None),
                ),
            ],
        };

        let field_selected = fields.iter().position(|field| field.editable).unwrap_or(0);
        Some(Self {
            mode,
            node_index,
            field_selected,
            fields,
        })
    }

    fn selected_field_mut(&mut self) -> Option<&mut OracleFormField> {
        self.fields.get_mut(self.field_selected)
    }

    fn set_field_value(&mut self, label: &str, value: impl Into<String>) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.label == label) {
            field.value = value.into();
        }
    }

    fn move_field(&mut self, offset: isize) {
        if self.fields.is_empty() {
            self.field_selected = 0;
            return;
        }
        let len = self.fields.len() as isize;
        let current = self.field_selected.min(self.fields.len() - 1) as isize;
        self.field_selected = (current + offset).clamp(0, len - 1) as usize;
    }

    fn validate(&self) -> Result<(), String> {
        let opening_evidence_form = self.mode == OracleFormMode::OpeningPrint
            || (self.mode == OracleFormMode::Challenge
                && self
                    .fields
                    .iter()
                    .any(|field| field.label == "Canonical locator"));
        for field in &self.fields {
            if field.required && field.value.trim().is_empty() {
                return Err(format!("{} is required.", field.label));
            }
            if field.label == "Wayback archive URL" && !field.value.trim().is_empty() {
                let archive_url = field.value.trim();
                if opening_evidence_form {
                    let canonical_locator = self
                        .fields
                        .iter()
                        .find(|candidate| candidate.label == "Canonical locator")
                        .map(|candidate| candidate.value.trim())
                        .unwrap_or("");
                    let source_time = self
                        .fields
                        .iter()
                        .find(|candidate| candidate.label == "Timestamp")
                        .map(|candidate| candidate.value.trim())
                        .unwrap_or("");
                    spread_oracle_plan::validate_opening_archive_url(
                        archive_url,
                        canonical_locator,
                        source_time,
                    )
                    .map_err(|error| format!("{error}."))?;
                } else if !archive_url.starts_with("http") {
                    return Err("Wayback archive URL must be a public URL.".to_string());
                }
            }
        }
        if self.mode == OracleFormMode::SourceProposal {
            let category = self
                .fields
                .iter()
                .find(|field| field.label == "Source category")
                .map(|field| field.value.trim())
                .unwrap_or("");
            if !is_allowed_v1_source_category(category) {
                return Err("Source category must be public retailer, distributor, manufacturer/store, benchmark/assessment, or public API.".to_string());
            }
        }
        if self.mode == OracleFormMode::Challenge {
            if let Some(claimant) = self
                .fields
                .iter()
                .find(|field| field.label == "Claimant")
                .map(|field| field.value.trim())
            {
                let parsed = Pubkey::from_str(claimant)
                    .map_err(|_| "Claimant must be a valid Solana address.".to_string())?;
                if parsed == Pubkey::default() || parsed.to_string() != claimant {
                    return Err("Claimant must be a canonical Solana address.".to_string());
                }
            }
            if let Some(corrected) = self
                .fields
                .iter()
                .find(|field| field.label == "Corrected value")
                .map(|field| field.value.trim())
                .filter(|value| !value.is_empty())
            {
                let parsed = corrected
                    .parse::<f64>()
                    .map_err(|_| "Corrected value must be a finite number.".to_string())?;
                if !parsed.is_finite() {
                    return Err("Corrected value must be a finite number.".to_string());
                }
                if self.fields.iter().any(|field| field.label == "Claimant")
                    && (parsed <= 0.0 || parsed.fract() != 0.0)
                {
                    return Err(
                        "An update challenge needs a positive whole-number corrected state."
                            .to_string(),
                    );
                }
            }
        }
        for label in ["Stake", "Stake/support", "Stake/bond"] {
            if let Some(value) = self
                .fields
                .iter()
                .find(|field| field.label == label)
                .map(|field| field.value.trim())
            {
                let parsed = value
                    .parse::<f64>()
                    .map_err(|_| format!("{label} must be a positive amount."))?;
                if !parsed.is_finite() || parsed <= 0.0 {
                    return Err(format!("{label} must be a positive amount."));
                }
            }
        }
        if matches!(
            self.mode,
            OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw
        ) {
            let amount = self
                .fields
                .iter()
                .find(|field| field.label == "Amount")
                .map(|field| field.value.trim())
                .unwrap_or("");
            if amount
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .is_none()
            {
                return Err("Amount must be a positive AMBA amount.".to_string());
            }
        }
        if self.mode == OracleFormMode::RewardClaim {
            let kind = self
                .fields
                .iter()
                .find(|field| field.label == "Reward kind")
                .map(|field| field.value.trim())
                .unwrap_or("");
            let valid = matches!(
                kind,
                "source_discovery"
                    | "source_challenge"
                    | "opening_challenge"
                    | "game_update"
                    | "update_challenge"
            );
            if !valid {
                return Err("Reward kind must be source_discovery, source_challenge, opening_challenge, game_update, or update_challenge.".to_string());
            }
        }
        if self.mode == OracleFormMode::StakeSettlement {
            let kind = self
                .fields
                .iter()
                .find(|field| field.label == "Stake kind")
                .map(|field| field.value.trim())
                .unwrap_or("");
            if !matches!(
                kind,
                "listing_bond"
                    | "support_stake"
                    | "source_challenge"
                    | "opening_claim"
                    | "opening_challenge"
                    | "update_claim"
                    | "update_challenge"
                    | "samba_emergency_vote"
            ) {
                return Err("Stake kind is not supported by terminal settlement.".to_string());
            }
            let subject = self
                .fields
                .iter()
                .find(|field| field.label == "Subject PDA")
                .map(|field| field.value.trim())
                .unwrap_or("");
            if subject.is_empty() {
                return Err("Terminal stake record is required.".to_string());
            }
            let eligibility = self
                .fields
                .iter()
                .find(|field| field.label == "Settlement eligibility")
                .map(|field| field.value.trim())
                .unwrap_or("");
            if eligibility != "ready to settle" {
                return Err("This stake or bond is not ready to settle.".to_string());
            }
        }
        Ok(())
    }

    fn summary(&self) -> String {
        let key_fields = self
            .fields
            .iter()
            .filter(|field| field.editable && !field.value.trim().is_empty())
            .take(2)
            .map(|field| format!("{}={}", field.label, field.value.trim()))
            .collect::<Vec<_>>();
        if key_fields.is_empty() {
            self.mode.title().to_string()
        } else {
            key_fields.join("; ")
        }
    }
}

#[derive(Clone, Debug)]
struct OracleSubmissionRecord {
    stored_id: Option<String>,
    title: String,
    node_label: String,
    row_label: String,
    phase: String,
    modules: String,
    source_state: OracleSourceState,
    update_state: Option<OracleUpdateState>,
    summary: String,
    backend_status: String,
    fields: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct OracleAccumulatorPreview {
    covered_rows: usize,
    covered_weight_pct: u32,
    total_rows: usize,
    final_index: Option<f64>,
    benchmark_delta_pct: Option<f64>,
    row_lines: Vec<String>,
    missing_rows: Vec<String>,
}

#[derive(Clone, Debug)]
struct OraclePinObservation {
    opening: Option<f64>,
    latest: Option<f64>,
}

#[derive(Clone, Debug)]
struct SpreadOracleLiveState {
    market_id: String,
    expiry_id: String,
    oracle_month: Option<String>,
    phase: Option<OraclePhase>,
    scramble_start_ts: Option<i64>,
    listing_ts: Option<i64>,
    expiry_ts: Option<i64>,
    schedule_version: Option<u64>,
    pending_resolution_count: Option<u64>,
    weight_scheme_version: Option<u8>,
    weight_scheme: String,
    effective_weight_total_bps: Option<f64>,
    weight_manifest_hash_hex: Option<String>,
    weight_verified: bool,
    weight_verification_status: String,
    active_weight_scheme_version: Option<u8>,
    active_weight_manifest_hash_hex: Option<String>,
    active_weight_verified: bool,
    active_weight_verification_status: String,
    observations: Vec<SpreadOracleObservation>,
    emergencies: Vec<SpreadOracleEmergency>,
    escrows: Vec<SpreadOracleEscrow>,
    issues: Vec<String>,
}

#[derive(Clone, Debug)]
struct SpreadOracleObservation {
    source: String,
    source_id_hex: String,
    status: String,
    baseline_state: String,
    current_state: String,
    support_stake_total: String,
    frozen_weight_bps: f64,
    bucket_weight_bps: Option<f64>,
    effective_weight_bps: Option<f64>,
    weight_verified: bool,
    active_weight_bps: Option<f64>,
    active_effective_weight_bps: Option<f64>,
    active_weight_verified: bool,
    opening_status: OpeningClaimViewStatus,
    opening_challenge_deadline_slot: Option<u64>,
    opening_finalizable: bool,
    opening_archive_url: Option<String>,
    emergency: Option<SpreadOracleEmergency>,
}

#[derive(Clone, Debug)]
struct SpreadOracleEmergency {
    dispute_id_hex: String,
    kind: String,
    status: String,
    target_id_hex: String,
    source_id_hex: Option<String>,
    claim_id_hex: Option<String>,
    challenge_id_hex: Option<String>,
}

#[derive(Clone, Debug)]
struct SpreadOracleEscrow {
    kind: String,
    subject_pda: String,
    owner_pubkey: String,
    amount_label: String,
    disposition: String,
    terminal_outcome: String,
    settlement_eligible: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct SpreadOracleRewardState {
    market_id: String,
    expiry_id: String,
    owner_pubkey: String,
    claims: Vec<SpreadOracleRewardClaim>,
}

#[derive(Clone, Debug)]
struct SpreadOracleRewardClaim {
    kind: String,
    label: String,
    source_id_hex: Option<String>,
    claim_id_hex: Option<String>,
    challenge_id_hex: Option<String>,
    claim_pda: Option<String>,
    subject_pda: String,
    amount_label: String,
}

impl SpreadOracleLiveState {
    fn scheduled_phase_at(&self, now_ts: i64) -> Option<OraclePhase> {
        let reported_phase = self.phase?;
        Some(
            match crate::oracle_lifecycle::classify_oracle_lifecycle(
                reported_phase.spread_label(),
                self.schedule_version,
                self.scramble_start_ts,
                self.listing_ts,
                self.expiry_ts,
                now_ts,
            ) {
                crate::oracle_lifecycle::OracleLifecyclePhase::Unavailable => {
                    OraclePhase::Unavailable
                }
                crate::oracle_lifecycle::OracleLifecyclePhase::Upcoming => OraclePhase::Upcoming,
                crate::oracle_lifecycle::OracleLifecyclePhase::SourceSubmission => {
                    OraclePhase::SourceSubmission
                }
                crate::oracle_lifecycle::OracleLifecyclePhase::Placement => OraclePhase::Placement,
                crate::oracle_lifecycle::OracleLifecyclePhase::KillChallenge => {
                    OraclePhase::KillChallenge
                }
                crate::oracle_lifecycle::OracleLifecyclePhase::ResolutionFreeze => {
                    OraclePhase::ResolutionFreeze
                }
                crate::oracle_lifecycle::OracleLifecyclePhase::Opening => OraclePhase::OpeningPrint,
                crate::oracle_lifecycle::OracleLifecyclePhase::Game => OraclePhase::GameMode,
                crate::oracle_lifecycle::OracleLifecyclePhase::Settlement => {
                    OraclePhase::MonthClose
                }
            },
        )
    }

    fn active_emergency_count(&self) -> usize {
        self.emergencies.len()
    }

    fn first_settlement_eligible_escrow(&self) -> Option<&SpreadOracleEscrow> {
        self.escrows
            .iter()
            .find(|escrow| escrow.settlement_eligible)
    }
}

impl SpreadOracleEmergency {
    fn matches_source(&self, source_id_hex: &str) -> bool {
        let source_id = source_id_hex.trim();
        [
            Some(self.target_id_hex.as_str()),
            self.source_id_hex.as_deref(),
            self.claim_id_hex.as_deref(),
            self.challenge_id_hex.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|candidate| candidate.eq_ignore_ascii_case(source_id))
    }
}

impl OracleSubmissionRecord {
    fn from_draft(draft: &OracleFormDraft, phase: OraclePhase, tree: &OracleIndexTree) -> Self {
        let node = tree.node(draft.node_index);
        let is_amba_custody = matches!(
            draft.mode,
            OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw
        );
        let is_treasury_action = draft.mode == OracleFormMode::RewardClaim;
        let is_stake_settlement = draft.mode == OracleFormMode::StakeSettlement;
        let row_label = if is_amba_custody || is_treasury_action || is_stake_settlement {
            "wallet".to_string()
        } else {
            tree.row_bucket_for(draft.node_index)
                .and_then(|index| tree.node(index).map(|node| node.label.clone()))
                .unwrap_or_else(|| "-".to_string())
        };
        Self {
            stored_id: None,
            title: draft.mode.title().to_string(),
            node_label: if is_amba_custody {
                "AMBA".to_string()
            } else if is_treasury_action {
                "Reward treasury".to_string()
            } else if is_stake_settlement {
                "Oracle stake".to_string()
            } else {
                node.map(|node| node.label.to_string())
                    .unwrap_or_else(|| "-".to_string())
            },
            row_label,
            phase: phase.label().to_string(),
            modules: draft.mode.modules().to_string(),
            source_state: draft.mode.source_state(),
            update_state: draft.mode.update_state(),
            summary: draft.summary(),
            backend_status: "queued in this TUI session".to_string(),
            fields: draft
                .fields
                .iter()
                .map(|field| (field.label.to_string(), field.value.clone()))
                .collect(),
        }
    }

    fn from_stored(stored: OracleSubmissionDraft) -> Self {
        Self {
            stored_id: Some(stored.id),
            title: stored.action,
            node_label: stored.node_label,
            row_label: stored.row_label,
            phase: stored.phase,
            modules: stored.modules,
            source_state: source_state_from_label(&stored.source_state),
            update_state: stored.update_state.as_deref().map(update_state_from_label),
            summary: stored.summary,
            backend_status: stored.backend_status,
            fields: stored
                .fields
                .into_iter()
                .map(|field| (field.label, field.value))
                .collect(),
        }
    }

    fn to_stored_draft(
        &self,
        form: &OracleFormDraft,
        market_id: &str,
        month_label: &str,
        expiry_id: Option<&str>,
        phase: OraclePhase,
        tree: &OracleIndexTree,
    ) -> Result<OracleSubmissionDraft, String> {
        let action = form.mode.spread_action().ok_or_else(|| {
            format!(
                "{} is not a spread-submittable oracle draft yet.",
                form.mode.title()
            )
        })?;
        let node = tree.node(form.node_index);
        Ok(OracleSubmissionDraft {
            id: String::new(),
            created_at_unix_seconds: 0,
            market_id: market_id.to_string(),
            month_label: month_label.to_string(),
            expiry_id: expiry_id
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "-")
                .map(str::to_string),
            oracle_month: None,
            action: action.to_string(),
            phase: phase.spread_label().to_string(),
            breadcrumb: tree
                .breadcrumb(form.node_index)
                .into_iter()
                .map(str::to_string)
                .collect(),
            node_label: self.node_label.clone(),
            node_kind: if matches!(
                form.mode,
                OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw
            ) {
                "wallet".to_string()
            } else if form.mode == OracleFormMode::RewardClaim {
                "oracle treasury".to_string()
            } else if form.mode == OracleFormMode::StakeSettlement {
                "oracle stake".to_string()
            } else {
                node.map(|node| node.kind.label().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            },
            row_label: self.row_label.clone(),
            source_state: self.source_state.label().to_string(),
            update_state: self.update_state.map(|state| state.label().to_string()),
            modules: self.modules.clone(),
            summary: self.summary.clone(),
            fields: self
                .fields
                .iter()
                .map(|(label, value)| OracleSubmissionField {
                    label: label.clone(),
                    value: value.clone(),
                })
                .collect(),
            backend_status: self.backend_status.clone(),
        })
    }

    fn raw_value(&self) -> Option<f64> {
        self.fields
            .iter()
            .find(|(label, _)| label == "Raw value")
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
    }
}

fn source_state_from_label(label: &str) -> OracleSourceState {
    match label {
        "PLACED" => OracleSourceState::Placed,
        "SNAPSHOTTED" => OracleSourceState::Snapshotted,
        "CHALLENGED" => OracleSourceState::Challenged,
        "FROZEN" => OracleSourceState::Frozen,
        "OPENING_PENDING" => OracleSourceState::OpeningPending,
        "ACTIVE" => OracleSourceState::Active,
        "MONTH_CLOSED" => OracleSourceState::MonthClosed,
        _ => OracleSourceState::Placed,
    }
}

fn update_state_from_label(label: &str) -> OracleUpdateState {
    match label {
        "SUBMITTED" => OracleUpdateState::Submitted,
        "CHALLENGED" => OracleUpdateState::Challenged,
        "FINALIZED" => OracleUpdateState::Finalized,
        "CORRECTED" => OracleUpdateState::Corrected,
        "REJECTED" => OracleUpdateState::Rejected,
        "COURT" => OracleUpdateState::Court,
        _ => OracleUpdateState::Submitted,
    }
}

fn load_oracle_submission_records(
    path: Option<&PathBuf>,
) -> (Vec<OracleSubmissionRecord>, Option<String>) {
    let Some(path) = path else {
        return (Vec::new(), None);
    };
    match oracle_submissions::load_recent_at_path(path, 16) {
        Ok(records) => (
            records
                .into_iter()
                .map(OracleSubmissionRecord::from_stored)
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error.to_string())),
    }
}

fn oracle_accumulator_preview(
    records: &[OracleSubmissionRecord],
    tree: Option<&OracleIndexTree>,
) -> OracleAccumulatorPreview {
    let Some(tree) = tree else {
        return OracleAccumulatorPreview {
            covered_rows: 0,
            covered_weight_pct: 0,
            total_rows: 0,
            final_index: None,
            benchmark_delta_pct: None,
            row_lines: Vec::new(),
            missing_rows: Vec::new(),
        };
    };
    let mut observations: HashMap<String, HashMap<String, OraclePinObservation>> = HashMap::new();
    for record in records {
        let Some(raw_value) = record.raw_value() else {
            continue;
        };
        let row = record.row_label.clone();
        if row == "-" {
            continue;
        }
        let pin = record.node_label.clone();
        let observation =
            observations
                .entry(row)
                .or_default()
                .entry(pin)
                .or_insert(OraclePinObservation {
                    opening: None,
                    latest: None,
                });
        match record.title.as_str() {
            "Opening print"
                if matches!(
                    record.source_state,
                    OracleSourceState::Active | OracleSourceState::MonthClosed
                ) =>
            {
                observation.opening = Some(raw_value);
                observation.latest.get_or_insert(raw_value);
            }
            "Submit update" => {
                observation.latest = Some(raw_value);
            }
            _ => {}
        }
    }

    let rows = tree.row_bucket_indices();
    let mut covered_rows = 0;
    let mut covered_weight_bps = 0_u32;
    let mut weighted_index_sum = 0.0;
    let mut row_lines = Vec::new();
    let mut missing_rows = Vec::new();

    for row_index in rows.iter().copied() {
        let Some(row_node) = tree.node(row_index) else {
            continue;
        };
        let Some(row_observations) = observations.get(row_node.label.as_str()) else {
            missing_rows.push(format!(
                "{} ({})",
                row_node.label.as_str(),
                format_percent(row_node.weight_pct)
            ));
            continue;
        };
        let mut source_deltas = Vec::new();
        for observation in row_observations.values() {
            let (Some(opening), Some(latest)) = (observation.opening, observation.latest) else {
                continue;
            };
            if opening > 0.0 && latest.is_finite() && opening.is_finite() {
                source_deltas.push((latest / opening) - 1.0);
            }
        }
        if source_deltas.is_empty() {
            missing_rows.push(format!(
                "{} ({})",
                row_node.label.as_str(),
                format_percent(row_node.weight_pct)
            ));
            continue;
        }
        let row_delta = source_deltas.iter().sum::<f64>() / source_deltas.len() as f64;
        let row_index_value = 100.0 * (1.0 + row_delta);
        weighted_index_sum += f64::from(row_node.weight_bps) * row_index_value;
        covered_rows += 1;
        covered_weight_bps += row_node.weight_bps;
        row_lines.push(format!(
            "{}: {} sources, row delta {}, row weight {}",
            row_node.label.as_str(),
            source_deltas.len(),
            format_signed_pct(row_delta * 100.0),
            format_percent(row_node.weight_pct)
        ));
    }

    let final_index = (covered_weight_bps == 10_000).then_some(weighted_index_sum / 10_000.0);
    OracleAccumulatorPreview {
        covered_rows,
        covered_weight_pct: covered_weight_bps / 100,
        total_rows: rows.len(),
        final_index,
        benchmark_delta_pct: final_index.map(|index| index - 100.0),
        row_lines,
        missing_rows,
    }
}

fn format_signed_pct(value: f64) -> String {
    if value.is_finite() {
        format!("{value:+.2}%")
    } else {
        "n/a".to_string()
    }
}

fn format_percent(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    if (value.fract()).abs() < 0.000_001 {
        format!("{}%", value as i64)
    } else {
        format!("{value:.2}%")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TradeAction {
    Buy,
    Sell,
}

impl TradeAction {
    fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    fn side_label(self) -> &'static str {
        match self {
            Self::Buy => "Bid",
            Self::Sell => "Ask",
        }
    }

    fn price_label(self) -> &'static str {
        match self {
            Self::Buy => "Price",
            Self::Sell => "Premium",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChainFocus {
    Markets,
    Calls,
    Puts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum LabFocus {
    Terms,
    Staking,
    Markets,
    MarketSeries,
    HomeSummary,
    HomeActions,
    HomePreview,
    OracleIntro,
    OracleEarn,
    OracleHelp,
    Chart,
    OracleOverview,
    OracleTasks,
    OracleActions,
    OraclePath,
    Help,
    Detail,
    Activity,
    Ledger,
    Calls,
    Puts,
    Guide,
}

#[derive(Clone, Copy, Debug)]
struct FocusedStackPanel {
    focus: Option<LabFocus>,
    line_count: usize,
    focused_min_height: u16,
}

impl FocusedStackPanel {
    fn new(focus: Option<LabFocus>, line_count: usize, focused_min_height: u16) -> Self {
        Self {
            focus,
            line_count,
            focused_min_height,
        }
    }
}

fn line_count_panel_height(line_count: usize) -> u16 {
    line_count.saturating_add(2).min(u16::MAX as usize) as u16
}

fn focused_stack_lengths(
    available_height: u16,
    active_focus: LabFocus,
    panels: &[FocusedStackPanel],
) -> Vec<u16> {
    const COLLAPSED_HEIGHT: u16 = 3;

    if panels.is_empty() {
        return Vec::new();
    }
    if available_height == 0 {
        return vec![0; panels.len()];
    }

    let full_heights = panels
        .iter()
        .map(|panel| line_count_panel_height(panel.line_count))
        .collect::<Vec<_>>();
    let full_total = full_heights.iter().copied().fold(0u16, u16::saturating_add);
    if full_total <= available_height {
        return full_heights;
    }
    let Some(focused_index) = panels
        .iter()
        .position(|panel| panel.focus == Some(active_focus))
    else {
        let mut remaining = available_height;
        let mut lengths = Vec::with_capacity(panels.len());
        for full_height in full_heights {
            let height = full_height.min(remaining);
            lengths.push(height);
            remaining = remaining.saturating_sub(height);
        }
        return lengths;
    };

    let mut lengths = vec![0; panels.len()];
    let mut remaining = available_height;
    let focused_min = panels[focused_index]
        .focused_min_height
        .max(COLLAPSED_HEIGHT)
        .min(full_heights[focused_index])
        .min(remaining);
    lengths[focused_index] = focused_min;
    remaining = remaining.saturating_sub(focused_min);

    for (index, full_height) in full_heights.iter().enumerate() {
        if index != focused_index {
            let allocated = COLLAPSED_HEIGHT.min(*full_height).min(remaining);
            lengths[index] = allocated;
            remaining = remaining.saturating_sub(allocated);
        }
    }

    let focused_growth = full_heights[focused_index]
        .saturating_sub(lengths[focused_index])
        .min(remaining);
    lengths[focused_index] = lengths[focused_index].saturating_add(focused_growth);
    remaining = remaining.saturating_sub(focused_growth);

    for (index, full_height) in full_heights.iter().copied().enumerate() {
        if index == focused_index {
            continue;
        }
        let growth = full_height.saturating_sub(lengths[index]).min(remaining);
        lengths[index] = lengths[index].saturating_add(growth);
        remaining = remaining.saturating_sub(growth);
        if remaining == 0 {
            break;
        }
    }
    lengths
}

fn vertical_rects_from_lengths(area: Rect, lengths: &[u16]) -> Vec<Rect> {
    let mut y = area.y;
    let mut remaining = area.height;
    lengths
        .iter()
        .map(|height| {
            let height = (*height).min(remaining);
            let rect = Rect {
                x: area.x,
                y,
                width: area.width,
                height,
            };
            y = y.saturating_add(height);
            remaining = remaining.saturating_sub(height);
            rect
        })
        .collect()
}

fn focused_stack_rects(
    area: Rect,
    active_focus: LabFocus,
    panels: &[FocusedStackPanel],
    fallback_lengths: &[u16],
) -> Vec<Rect> {
    let lengths = if panels.iter().any(|panel| panel.focus == Some(active_focus)) {
        focused_stack_lengths(area.height, active_focus, panels)
    } else {
        fallback_lengths.to_vec()
    };
    vertical_rects_from_lengths(area, &lengths)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FocusEdge {
    screen: LabScreen,
    from: LabFocus,
    direction: FocusDirection,
    to: LabFocus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarketRailHit {
    Market(usize),
    Series(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TradeTicketField {
    Premium,
    Quantity,
}

const TRADE_TICKET_FIELD_FLASH_TICKS: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TradeTicketFieldFlash {
    field: TradeTicketField,
    ticks_remaining: u8,
    visible: bool,
}

#[derive(Clone, Debug)]
struct TradeTicketResult {
    ok: bool,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TradeConfirmationChoice {
    Cancel,
    Confirm,
}

#[derive(Clone, Debug, PartialEq)]
struct TradeConfirmationSummary {
    action: TradeAction,
    symbol: String,
    expiry: String,
    kind: OptionKind,
    lower_strike: String,
    upper_strike: String,
    price: f64,
    qty: u64,
    entry_total: f64,
    total_max_loss: f64,
    total_max_gain: f64,
    total_max_payout: f64,
    probability_itm: Option<f64>,
    probability_cap_hit: Option<f64>,
    account: String,
}

#[derive(Clone, Debug)]
struct TradeConfirmation {
    prepared: TradeTicketSubmit,
    choice: TradeConfirmationChoice,
}

#[derive(Clone, Debug)]
struct TradeResultModal {
    ok: bool,
    waiting: bool,
    action: TradeAction,
    summary: Option<TradeConfirmationSummary>,
    details_in_ticket: bool,
    failure_reason: Option<String>,
    expires_at: Instant,
}

impl TradeResultModal {
    fn new(
        ok: bool,
        action: TradeAction,
        summary: Option<TradeConfirmationSummary>,
        details_in_ticket: bool,
        now: Instant,
    ) -> Self {
        Self {
            ok,
            waiting: false,
            action,
            summary,
            details_in_ticket,
            failure_reason: None,
            expires_at: now + Duration::from_secs(TRADE_RESULT_MODAL_SECONDS),
        }
    }

    fn remaining_seconds_at(&self, now: Instant) -> u64 {
        let remaining_millis = self.expires_at.saturating_duration_since(now).as_millis();
        if remaining_millis == 0 {
            0
        } else {
            ((remaining_millis + 999) / 1_000) as u64
        }
    }

    fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[derive(Clone, Debug)]
struct TradeTicket {
    action: TradeAction,
    premium_input: String,
    quantity_input: String,
    field: TradeTicketField,
    confirmation: Option<TradeConfirmation>,
    submitting: bool,
    submit_request_id: Option<u64>,
    last_command: Option<String>,
    result: Option<TradeTicketResult>,
}

impl TradeTicket {
    fn new(action: TradeAction, premium: Option<f64>) -> Self {
        Self {
            action,
            premium_input: premium
                .map(|value| format_decimal(value, 3))
                .unwrap_or_default(),
            quantity_input: String::new(),
            field: TradeTicketField::Premium,
            confirmation: None,
            submitting: false,
            submit_request_id: None,
            last_command: None,
            result: None,
        }
    }

    fn clear_review(&mut self) {
        self.confirmation = None;
        self.result = None;
    }
}

#[derive(Debug)]
struct OracleTreeFetch {
    tree: OracleIndexTree,
    notice: Option<String>,
}

struct GitbookRenderCache {
    page_id: String,
    page_revision: u64,
    page_count: usize,
    width: usize,
    no_color: bool,
    motion_tick: usize,
    lines: Vec<Line<'static>>,
}

#[derive(Clone, Debug)]
struct GuidePendingContinuation {
    expected_screen: LabScreen,
    market_id: Option<String>,
    tool_result: guide::GuideToolResult,
}

#[derive(Clone, Debug)]
struct GuidePanelState {
    config: guide::GuideConfig,
    provider_status: guide::GuideProviderStatus,
    selected_provider: usize,
    input: String,
    composing: bool,
    context_focus: Option<LabFocus>,
    messages: VecDeque<guide::GuideConversationTurn>,
    scroll: usize,
    request_id: u64,
    loading: bool,
    progress: Option<String>,
    suggested_actions: Vec<String>,
    selected_suggestion: Option<usize>,
    action_preview: Option<guide::GuideActionPreview>,
    highlighted_targets: Vec<String>,
    comparison_targets: Vec<String>,
    focused_control: Option<String>,
    request_target_ids: HashSet<String>,
    request_ui_target_ids: HashSet<String>,
    request_challenge_ids: HashSet<String>,
    active_question: Option<String>,
    allow_continuation: bool,
    tool_step: u8,
    pending_continuation: Option<GuidePendingContinuation>,
}

impl GuidePanelState {
    fn new(config: guide::GuideConfig) -> Self {
        let provider_status = if let Some(issue) = config.issue.clone() {
            guide::GuideProviderStatus::SetupRequired { message: issue }
        } else if config.provider == guide::GuideProviderPreference::Off {
            guide::GuideProviderStatus::Off
        } else {
            guide::GuideProviderStatus::Checking
        };
        Self {
            config,
            provider_status,
            selected_provider: 0,
            input: String::new(),
            composing: false,
            context_focus: None,
            messages: VecDeque::new(),
            scroll: 0,
            request_id: 0,
            loading: false,
            progress: None,
            suggested_actions: Vec::new(),
            selected_suggestion: None,
            action_preview: None,
            highlighted_targets: Vec::new(),
            comparison_targets: Vec::new(),
            focused_control: None,
            request_target_ids: HashSet::new(),
            request_ui_target_ids: HashSet::new(),
            request_challenge_ids: HashSet::new(),
            active_question: None,
            allow_continuation: false,
            tool_step: 0,
            pending_continuation: None,
        }
    }

    fn push_message(&mut self, role: guide::GuideConversationRole, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.messages
            .push_back(guide::GuideConversationTurn { role, text });
        while self.messages.len() > 16 {
            self.messages.pop_front();
        }
        self.scroll = 0;
    }

    fn conversation_context(&self) -> Vec<guide::GuideConversationTurn> {
        self.messages
            .iter()
            .rev()
            .take(4)
            .cloned()
            .map(|mut turn| {
                turn.text = turn.text.chars().take(600).collect();
                turn
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

struct LabApp {
    dishes: Vec<DishSummary>,
    selected: usize,
    pending_initial_dish: Option<String>,
    home_selected: usize,
    home_help_topic: HomeHelpTopic,
    help_index: GitbookIndex,
    help_pages: crate::cache::DisplayCache<String, GitbookPage>,
    help_page_revision: u64,
    help_render_cache: RefCell<Option<GitbookRenderCache>>,
    help_selected_page_id: String,
    help_selected_nav: usize,
    help_expanded_categories: Vec<bool>,
    help_pane: HelpPane,
    help_nav_scroll: usize,
    help_article_scroll: usize,
    help_preview: Option<GitbookHelpPreview>,
    help_glossary_hover: Option<GitbookGlossaryHover>,
    help_hover_grace_ticks: Option<usize>,
    help_index_request: u64,
    help_page_request: u64,
    help_page_request_id: Option<String>,
    help_preview_page_request: u64,
    help_preview_page_request_id: Option<String>,
    loading_help_index: bool,
    loading_help_page: bool,
    loading_help_preview_page: bool,
    help_preview_failed_page_id: Option<String>,
    help_issue: Option<String>,
    help_transition_tick: Option<usize>,
    help_last_checked_at: Option<Instant>,
    mcp_connection_state: McpConnectionState,
    mcp_managed_entry_enabled: bool,
    mcp_connection_issue: Option<String>,
    mcp_repair_failed: bool,
    guide: GuidePanelState,
    oracle_intro_selected: usize,
    oracle_view: OracleView,
    oracle_earn_selected: usize,
    oracle_selected: usize,
    oracle_node_selected: usize,
    oracle_search_input: String,
    oracle_search_editing: bool,
    oracle_form: Option<OracleFormDraft>,
    oracle_form_field_flash: Option<OracleFormFieldFlash>,
    oracle_locked_flash: Option<OracleLockedFlash>,
    oracle_tree: Option<OracleIndexTree>,
    oracle_tree_issue: Option<String>,
    oracle_tree_retry_after_tick: Option<usize>,
    oracle_submissions: Vec<OracleSubmissionRecord>,
    oracle_submission_store_path: Option<PathBuf>,
    oracle_submission_issue: Option<String>,
    oracle_live: Option<SpreadOracleLiveState>,
    oracle_live_issue: Option<String>,
    oracle_rewards: Option<SpreadOracleRewardState>,
    oracle_reward_issue: Option<String>,
    update_check_enabled: bool,
    update_check_request: u64,
    update_report: Option<WorkspaceUpdateReport>,
    update_issue: Option<String>,
    update_check_forced: bool,
    loading_update_check: bool,
    update_mouse_requested: bool,
    selected_option: usize,
    active_option_kind: OptionKind,
    chain_focus: ChainFocus,
    focus: LabFocus,
    market_series_open: bool,
    chart_expiry: usize,
    initial_chart_expiry: Option<String>,
    chart_range: ChartRangeValue,
    chart_launch_options: Option<ChartArgs>,
    chart_last_refresh_at: Option<Instant>,
    pending_initial_chart: bool,
    trade_action: TradeAction,
    trade_ticket: Option<TradeTicket>,
    read_panel: Option<read_panel::ReadPanel>,
    read_panel_request: u64,
    action_panel: Option<actions::ActionPanel>,
    action_panel_request: u64,
    trade_review_scroll: u16,
    trade_ticket_field_flash: Option<TradeTicketFieldFlash>,
    trade_result_modal: Option<TradeResultModal>,
    suppress_trade_result_escape_repeat: bool,
    left_mouse_down: bool,
    confirmation_mouse_press: Option<ConfirmationMousePress>,
    detail: Option<DishDetail>,
    detail_view: DetailView,
    settlement_bundle: Option<settlement_data::SettlementBundle>,
    settlement_issue: Option<String>,
    chart: Option<chart::EmbeddedChart>,
    ledger: Option<Value>,
    ledger_view: LedgerView,
    ledger_pane: LedgerPane,
    ledger_account_action_selected: usize,
    ledger_position_selected: usize,
    ledger_writer_selected: usize,
    ledger_history_selected: usize,
    liquidity_preview_form: Option<liquidity::LiquidityPreviewForm>,
    liquidity_preview_result: Option<liquidity::LiquidityPreviewResult>,
    writer_action_selected: usize,
    writer_form: Option<WriterActionForm>,
    writer_confirmation: Option<WriterActionConfirmation>,
    writer_action_result: Option<WriterActionResult>,
    staking_status: Option<Value>,
    staking_issue: Option<String>,
    staking_selected: usize,
    staking_form: Option<StakingForm>,
    staking_confirmation: Option<StakingConfirmation>,
    staking_action_result: Option<StakingActionResult>,
    wallet: AttachedWallet,
    wallet_terms: WalletTermsStatus,
    wallet_switch_input: String,
    wallet_switch_editing: bool,
    onchain_config: OnchainConfig,
    screen: LabScreen,
    startup_intro_started_at: Option<Instant>,
    screen_history: Vec<LabScreen>,
    status: String,
    issues: Vec<String>,
    read_cache_scope: read_cache::ReadScope,
    detail_cache: crate::cache::DisplayCache<String, DishDetail>,
    chart_cache: crate::cache::DisplayCache<String, chart::EmbeddedChart>,
    settlement_cache:
        crate::cache::DisplayCache<(String, String), settlement_data::SettlementBundle>,
    panel_scrolls: HashMap<LabFocus, usize>,
    list_request: u64,
    detail_request: u64,
    settlement_request: u64,
    chart_request: u64,
    ledger_request: u64,
    liquidity_preview_request: u64,
    liquidity_preview_inflight: Option<u64>,
    writer_action_request: u64,
    writer_action_inflight: Option<u64>,
    writer_action_mask_request: u64,
    writer_action_mask_inflight: Option<u64>,
    pending_writer_review: Option<PendingWriterReview>,
    staking_status_request: u64,
    staking_action_request: u64,
    staking_action_inflight: Option<u64>,
    trade_submit_request: u64,
    trade_submit_inflight: Option<u64>,
    oracle_tree_request: u64,
    oracle_live_request: u64,
    oracle_reward_request: u64,
    loading_list: bool,
    loading_detail: bool,
    loading_settlement: bool,
    loading_chart: bool,
    loading_ledger: bool,
    loading_staking: bool,
    loading_oracle_tree: bool,
    loading_oracle_live: bool,
    loading_oracle_rewards: bool,
    spinner_tick: usize,
    terminal_size_warning_started_tick: Option<usize>,
}

#[path = "actions.rs"]
mod actions;
#[path = "core.rs"]
mod core;
#[path = "guide_snapshot.rs"]
mod guide_snapshot;
#[path = "guide.rs"]
mod guide_state;
#[path = "help.rs"]
mod help;
#[path = "input.rs"]
mod input;
#[path = "intro.rs"]
mod intro;
#[path = "jobs.rs"]
mod jobs;
#[path = "liquidity.rs"]
mod liquidity;
#[path = "market.rs"]
mod market;
#[path = "messages.rs"]
mod messages;
#[path = "navigation.rs"]
mod navigation;
#[path = "oracle.rs"]
mod oracle;
#[path = "parse.rs"]
mod parse;
#[path = "cache.rs"]
mod read_cache;
#[path = "read_panel.rs"]
mod read_panel;
#[path = "requests.rs"]
mod requests;
#[path = "routing.rs"]
mod routing;
#[path = "runtime.rs"]
mod runtime;
#[path = "settlement_data.rs"]
mod settlement_data;
#[path = "staking.rs"]
mod staking;
#[path = "trade.rs"]
mod trade;
#[path = "ui/mod.rs"]
mod ui;
#[path = "writers.rs"]
mod writers;

use guide_snapshot::*;
use intro::*;
use jobs::*;
use messages::*;
use navigation::*;
use parse::*;
pub use runtime::{LabExitAction, run_lab_bench, run_lab_chart_bench};
use runtime::{TerminalPointerShape, staking_form_input_character, validate_staking_decimal};
use ui::*;
