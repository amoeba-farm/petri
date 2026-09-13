//! Lab Bench state and feature composition.
//!
//! This module owns the aggregate state shared by the terminal runtime. Child
//! modules own effects, input routing, and feature-specific presentation while
//! remaining descendants so state does not need crate-wide visibility.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    env,
    hash::{Hash, Hasher},
    io::{self, IsTerminal, Write},
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
    oracle_submissions,
    oracle_tui::{DEFAULT_ORACLE_NODE_INDEX, OracleIndexTree, OracleNodeKind, RamxOracleNode},
    positions, solana_history, terminal_brand,
    terminal_keys::is_actionable_key_event,
    wallet_balance,
    wallet_terms::{self, WalletTermsStatus},
    workspace_update::WorkspaceUpdateReport,
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

#[derive(Debug)]
struct OracleTreeFetch {
    tree: OracleIndexTree,
    notice: Option<String>,
}

struct LabApp {
    trading: trade_state::TradeState,
    home_selected: usize,
    home_help_topic: HomeHelpTopic,
    help: help_state::HelpState,
    mcp_connection_state: McpConnectionState,
    mcp_managed_entry_enabled: bool,
    mcp_connection_issue: Option<String>,
    mcp_repair_failed: bool,
    guide: GuidePanelState,
    oracle: oracle_state::OracleState,
    updates: updates::UpdateState,
    focus: LabFocus,
    read_panel: Option<read_panel::ReadPanel>,
    read_panel_request: u64,
    action_panel: Option<actions::ActionPanel>,
    action_panel_request: u64,
    left_mouse_down: bool,
    confirmation_mouse_press: Option<ConfirmationMousePress>,
    ledger: Option<Value>,
    ledger_view: LedgerView,
    ledger_pane: LedgerPane,
    ledger_account_action_selected: usize,
    ledger_position_selected: usize,
    ledger_writer_selected: usize,
    ledger_history_selected: usize,
    liquidity_preview_form: Option<liquidity::LiquidityPreviewForm>,
    liquidity_preview_result: Option<liquidity::LiquidityPreviewResult>,
    writers: writer_state::WriterState,
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
    cache: read_cache::TuiCache,
    panel_scrolls: HashMap<LabFocus, usize>,
    ledger_request: u64,
    liquidity_preview_request: u64,
    liquidity_preview_inflight: Option<u64>,
    staking_status_request: u64,
    staking_action_request: u64,
    staking_action_inflight: Option<u64>,
    loading_ledger: bool,
    loading_staking: bool,
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
#[path = "help_state.rs"]
mod help_state;
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
#[path = "oracle_forms.rs"]
mod oracle_forms;
#[path = "oracle_model.rs"]
mod oracle_model;
#[path = "oracle_state.rs"]
mod oracle_state;
use oracle_forms::{OracleFormDraft, OracleFormField, OracleFormMode};
use oracle_model::{
    OpeningClaimViewStatus, OracleAction, OracleActionAvailability, OracleActionContext,
    OracleIntroAction, OraclePhase, OracleSourceActionState, OracleSourceState,
    OracleSubmissionRecord, OracleUpdateState, SpreadOracleEmergency, SpreadOracleEscrow,
    SpreadOracleLiveState, SpreadOracleObservation, SpreadOracleRewardClaim,
    SpreadOracleRewardState, format_percent, format_signed_pct, load_oracle_submission_records,
    oracle_accumulator_preview, oracle_phase_from_spread_label,
};
use oracle_state::OracleView;
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
#[path = "trade_state.rs"]
mod trade_state;
use trade_state::{
    ChainFocus, DetailView, TradeAction, TradeConfirmation, TradeConfirmationChoice,
    TradeConfirmationSummary, TradeResultModal, TradeSubmitLaunch, TradeTicket, TradeTicketField,
    TradeTicketResult, TradeTicketSubmit,
};
#[path = "ui/mod.rs"]
mod ui;
#[path = "updates.rs"]
mod updates;
#[path = "writer_state.rs"]
mod writer_state;
#[path = "writers.rs"]
mod writers;
use writer_state::{
    PendingWriterReview, WriterAction, WriterActionAvailability, WriterActionConfirmation,
    WriterActionForm, WriterActionResult, WriterCloseCapabilityProjection,
    WriterCloseCapabilityRoute, WriterCloseCapabilityState,
};

use guide_snapshot::*;
use guide_state::GuidePanelState;
use intro::*;
use jobs::*;
use messages::*;
use navigation::*;
use parse::*;
use read_cache::GitbookRenderCache;
pub use runtime::{LabExitAction, run_lab_bench, run_lab_chart_bench};
use runtime::{TerminalPointerShape, staking_form_input_character, validate_staking_decimal};
use ui::*;
