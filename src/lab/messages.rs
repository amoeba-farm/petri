//! Typed completion messages sent from bounded background jobs to the Lab runtime.

use super::*;

pub(super) enum LabFetchResult {
    ActionPanel {
        id: u64,
        executed: bool,
        result: Result<Value, String>,
    },
    ReadPanel {
        id: u64,
        result: Result<(String, Vec<String>), String>,
    },
    MarketList {
        request_id: u64,
        result: Result<Value, String>,
    },
    Detail {
        request_id: u64,
        market_id: String,
        result: Result<DishDetail, String>,
    },
    Settlement {
        request_id: u64,
        market_id: String,
        expiry_id: String,
        bundle: settlement_data::SettlementBundle,
    },
    Chart {
        request_id: u64,
        key: String,
        market_id: String,
        month_label: String,
        result: Result<chart::EmbeddedChart, String>,
    },
    Ledger {
        request_id: u64,
        owner_pubkey: String,
        result: Result<Value, String>,
    },
    LiquidityPreview {
        request_id: u64,
        result: Result<Value, String>,
    },
    WriterCommand {
        request_id: u64,
        action: WriterAction,
        result: Result<Value, String>,
    },
    WriterCapabilities {
        request_id: u64,
        result: Result<WriterCloseCapabilityProjection, String>,
    },
    WriterActionMask {
        request_id: u64,
        action: WriterAction,
        owner: String,
        sleeve: String,
        result: Result<WriterActionMask, String>,
    },
    StakingStatus {
        request_id: u64,
        owner_pubkey: String,
        result: Result<Value, String>,
    },
    StakingAction {
        request_id: u64,
        action: StakingAction,
        result: Result<(Value, String), String>,
    },
    TradeSubmit {
        request_id: u64,
        action: TradeAction,
        summary: TradeConfirmationSummary,
        command: String,
        result: Result<String, String>,
    },
    TradePrepare {
        request_id: u64,
        owner: String,
        expiry: String,
        submit: TradeTicketSubmit,
        result: Result<Value, String>,
    },
    OracleTree {
        request_id: u64,
        market_id: String,
        result: Result<OracleTreeFetch, String>,
    },
    OracleLive {
        request_id: u64,
        market_id: String,
        expiry_id: String,
        result: Result<SpreadOracleLiveState, String>,
    },
    OracleRewards {
        request_id: u64,
        market_id: String,
        expiry_id: String,
        owner_pubkey: String,
        result: Result<SpreadOracleRewardState, String>,
    },
    HelpIndex {
        request_id: u64,
        result: Result<GitbookIndex, String>,
    },
    HelpPage {
        request_id: u64,
        page_id: String,
        result: Result<GitbookPage, String>,
    },
    HelpPreviewPage {
        request_id: u64,
        page_id: String,
        result: Result<GitbookPage, String>,
    },
    UpdateCheck {
        request_id: u64,
        result: Result<WorkspaceUpdateReport, String>,
    },
    GuideProbe {
        request_id: u64,
        status: guide::GuideProviderStatus,
    },
    GuideProgress {
        request_id: u64,
        event: guide::GuideStreamEvent,
    },
    GuideReply {
        request_id: u64,
        state_revision: String,
        result: Result<guide::GuideProviderReply, String>,
    },
}
