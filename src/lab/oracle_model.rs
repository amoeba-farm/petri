//! Oracle action/lifecycle projections and submission records. No LabApp dependency.
use super::oracle_forms::{OracleFormDraft, OracleFormMode};
use crate::{
    oracle_submissions::{self, OracleSubmissionDraft, OracleSubmissionField},
    oracle_tui::{OracleIndexTree, OracleNodeKind},
};
use ratatui::style::Color;
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OracleIntroAction {
    Earn,
    Advanced,
    ReadMore,
    BackHome,
}

impl OracleIntroAction {
    pub(super) const ALL: [Self; 4] = [Self::Earn, Self::Advanced, Self::ReadMore, Self::BackHome];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Earn => "Earn — funded rewards",
            Self::Advanced => "Advanced view",
            Self::ReadMore => "How it works",
            Self::BackHome => "Exit to home",
        }
    }

    pub(super) fn detail(self) -> &'static str {
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

    pub(super) fn color(self) -> Color {
        match self {
            Self::Earn => Color::Green,
            Self::Advanced => Color::Magenta,
            Self::ReadMore => Color::Cyan,
            Self::BackHome => Color::Yellow,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OracleAction {
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
    pub(super) const ALL: [Self; 9] = [
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

    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn display_label(self) -> &'static str {
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

    pub(super) fn allowed(self, phase: OraclePhase) -> bool {
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

    pub(super) fn base_availability(
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

    pub(super) fn availability(self, context: OracleActionContext) -> OracleActionAvailability {
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

    pub(super) fn detail(self, phase: OraclePhase) -> &'static str {
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

    pub(super) fn contextual_detail(self, context: OracleActionContext) -> &'static str {
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

    pub(super) fn state(self, context: OracleActionContext) -> &'static str {
        match self {
            Self::ReviewQueue if self.allowed(context.phase) => "view",
            _ => self.availability(context).label(),
        }
    }

    pub(super) fn color(self) -> Color {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OraclePhase {
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
    pub(super) const ALL: [Self; 7] = [
        Self::SourceSubmission,
        Self::Placement,
        Self::KillChallenge,
        Self::ResolutionFreeze,
        Self::OpeningPrint,
        Self::GameMode,
        Self::MonthClose,
    ];

    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn spread_label(self) -> &'static str {
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

    pub(super) fn detail(self) -> &'static str {
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

    pub(super) fn color(self) -> Color {
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

    pub(super) fn active_color(self) -> Color {
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

pub(super) fn oracle_phase_from_spread_label(label: &str) -> Option<OraclePhase> {
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
pub(super) enum OracleSourceState {
    Placed,
    Snapshotted,
    Challenged,
    Frozen,
    OpeningPending,
    Active,
    MonthClosed,
}

impl OracleSourceState {
    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn display_label(self) -> &'static str {
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
pub(super) enum OracleUpdateState {
    Submitted,
    Challenged,
    Finalized,
    Corrected,
    Rejected,
    Court,
}

impl OracleUpdateState {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Submitted => "SUBMITTED",
            Self::Challenged => "CHALLENGED",
            Self::Finalized => "FINALIZED",
            Self::Corrected => "CORRECTED",
            Self::Rejected => "REJECTED",
            Self::Court => "COURT",
        }
    }

    pub(super) fn display_label(self) -> &'static str {
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
pub(super) enum OpeningClaimViewStatus {
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
    pub(super) fn from_label(label: &str, opening_submitted: bool, source_status: &str) -> Self {
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

    pub(super) fn display_label(self) -> &'static str {
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

    pub(super) fn can_submit(self) -> bool {
        matches!(self, Self::Missing | Self::RejectedRetryable)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct OracleSourceActionState {
    pub(super) has_unresolved_challenge: bool,
    pub(super) has_active_emergency: bool,
    pub(super) opening_status: OpeningClaimViewStatus,
    pub(super) opening_finalizable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OracleActionContext {
    pub(super) phase: OraclePhase,
    pub(super) node_kind: OracleNodeKind,
    pub(super) source: OracleSourceActionState,
    pub(super) market_expired: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OracleActionAvailability {
    Active,
    ChooseSource,
    Locked,
}

impl OracleActionAvailability {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ChooseSource => "choose source",
            Self::Locked => "locked",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct OracleSubmissionRecord {
    pub(super) stored_id: Option<String>,
    pub(super) title: String,
    pub(super) node_label: String,
    pub(super) row_label: String,
    pub(super) phase: String,
    pub(super) modules: String,
    pub(super) source_state: OracleSourceState,
    pub(super) update_state: Option<OracleUpdateState>,
    pub(super) summary: String,
    pub(super) backend_status: String,
    pub(super) fields: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub(super) struct OracleAccumulatorPreview {
    pub(super) covered_rows: usize,
    pub(super) covered_weight_pct: u32,
    pub(super) total_rows: usize,
    pub(super) final_index: Option<f64>,
    pub(super) benchmark_delta_pct: Option<f64>,
    pub(super) row_lines: Vec<String>,
    pub(super) missing_rows: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct OraclePinObservation {
    pub(super) opening: Option<f64>,
    pub(super) latest: Option<f64>,
}

#[derive(Clone, Debug)]
pub(super) struct SpreadOracleLiveState {
    pub(super) market_id: String,
    pub(super) expiry_id: String,
    pub(super) oracle_month: Option<String>,
    pub(super) phase: Option<OraclePhase>,
    pub(super) scramble_start_ts: Option<i64>,
    pub(super) listing_ts: Option<i64>,
    pub(super) expiry_ts: Option<i64>,
    pub(super) schedule_version: Option<u64>,
    pub(super) pending_resolution_count: Option<u64>,
    pub(super) weight_scheme_version: Option<u8>,
    pub(super) weight_scheme: String,
    pub(super) effective_weight_total_bps: Option<f64>,
    pub(super) weight_manifest_hash_hex: Option<String>,
    pub(super) weight_verified: bool,
    pub(super) weight_verification_status: String,
    pub(super) active_weight_scheme_version: Option<u8>,
    pub(super) active_weight_manifest_hash_hex: Option<String>,
    pub(super) active_weight_verified: bool,
    pub(super) active_weight_verification_status: String,
    pub(super) observations: Vec<SpreadOracleObservation>,
    pub(super) emergencies: Vec<SpreadOracleEmergency>,
    pub(super) escrows: Vec<SpreadOracleEscrow>,
    pub(super) issues: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SpreadOracleObservation {
    pub(super) source: String,
    pub(super) source_id_hex: String,
    pub(super) status: String,
    pub(super) baseline_state: String,
    pub(super) current_state: String,
    pub(super) support_stake_total: String,
    pub(super) frozen_weight_bps: f64,
    pub(super) bucket_weight_bps: Option<f64>,
    pub(super) effective_weight_bps: Option<f64>,
    pub(super) weight_verified: bool,
    pub(super) active_weight_bps: Option<f64>,
    pub(super) active_effective_weight_bps: Option<f64>,
    pub(super) active_weight_verified: bool,
    pub(super) opening_status: OpeningClaimViewStatus,
    pub(super) opening_challenge_deadline_slot: Option<u64>,
    pub(super) opening_finalizable: bool,
    pub(super) opening_archive_url: Option<String>,
    pub(super) emergency: Option<SpreadOracleEmergency>,
}

#[derive(Clone, Debug)]
pub(super) struct SpreadOracleEmergency {
    pub(super) dispute_id_hex: String,
    pub(super) kind: String,
    pub(super) status: String,
    pub(super) target_id_hex: String,
    pub(super) source_id_hex: Option<String>,
    pub(super) claim_id_hex: Option<String>,
    pub(super) challenge_id_hex: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SpreadOracleEscrow {
    pub(super) kind: String,
    pub(super) subject_pda: String,
    pub(super) owner_pubkey: String,
    pub(super) amount_label: String,
    pub(super) disposition: String,
    pub(super) terminal_outcome: String,
    pub(super) settlement_eligible: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(super) struct SpreadOracleRewardState {
    pub(super) market_id: String,
    pub(super) expiry_id: String,
    pub(super) owner_pubkey: String,
    pub(super) claims: Vec<SpreadOracleRewardClaim>,
}

#[derive(Clone, Debug)]
pub(super) struct SpreadOracleRewardClaim {
    pub(super) kind: String,
    pub(super) label: String,
    pub(super) source_id_hex: Option<String>,
    pub(super) claim_id_hex: Option<String>,
    pub(super) challenge_id_hex: Option<String>,
    pub(super) claim_pda: Option<String>,
    pub(super) subject_pda: String,
    pub(super) amount_label: String,
}

impl SpreadOracleLiveState {
    pub(super) fn scheduled_phase_at(&self, now_ts: i64) -> Option<OraclePhase> {
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

    pub(super) fn active_emergency_count(&self) -> usize {
        self.emergencies.len()
    }

    pub(super) fn first_settlement_eligible_escrow(&self) -> Option<&SpreadOracleEscrow> {
        self.escrows
            .iter()
            .find(|escrow| escrow.settlement_eligible)
    }
}

impl SpreadOracleEmergency {
    pub(super) fn matches_source(&self, source_id_hex: &str) -> bool {
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
    pub(super) fn from_draft(
        draft: &OracleFormDraft,
        phase: OraclePhase,
        tree: &OracleIndexTree,
    ) -> Self {
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

    pub(super) fn from_stored(stored: OracleSubmissionDraft) -> Self {
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

    pub(super) fn to_stored_draft(
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

    pub(super) fn raw_value(&self) -> Option<f64> {
        self.fields
            .iter()
            .find(|(label, _)| label == "Raw value")
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
    }
}

pub(super) fn source_state_from_label(label: &str) -> OracleSourceState {
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

pub(super) fn update_state_from_label(label: &str) -> OracleUpdateState {
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

pub(super) fn load_oracle_submission_records(
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

pub(super) fn oracle_accumulator_preview(
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

pub(super) fn format_signed_pct(value: f64) -> String {
    if value.is_finite() {
        format!("{value:+.2}%")
    } else {
        "n/a".to_string()
    }
}

pub(super) fn format_percent(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    if (value.fract()).abs() < 0.000_001 {
        format!("{}%", value as i64)
    } else {
        format!("{value:.2}%")
    }
}
