//! Feature-owned writers state and local transitions. Network and signing effects stay in the coordinator.
use super::{ActionFormField, UserActionConfirmationChoice};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WriterAction {
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
    pub(super) const ALL: [Self; 20] = [
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

    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn detail(self) -> &'static str {
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

    pub(super) fn signs_and_submits(self) -> bool {
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
pub(super) struct WriterCloseCapabilityRoute {
    pub(super) method: String,
    pub(super) path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WriterCloseCapabilityProjection {
    pub(super) implementation_supported: bool,
    pub(super) runtime_enabled: bool,
    pub(super) hot_runtime_enabled: bool,
    pub(super) cold_runtime_enabled: bool,
    pub(super) operations: Vec<String>,
    pub(super) routes: Vec<WriterCloseCapabilityRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WriterCloseCapabilityState {
    Checking,
    Ready(WriterCloseCapabilityProjection),
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WriterActionAvailability {
    Enabled,
    HotOnly(String),
    Disabled(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WriterActionForm {
    pub(super) action: WriterAction,
    pub(super) fields: Vec<ActionFormField>,
    pub(super) selected_field: usize,
}

impl WriterActionForm {
    pub(super) fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.trim())
    }

    pub(super) fn selected_field_mut(&mut self) -> Option<&mut ActionFormField> {
        self.fields.get_mut(self.selected_field)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WriterActionConfirmation {
    pub(super) action: WriterAction,
    pub(super) args: Vec<String>,
    pub(super) summary: Vec<String>,
    pub(super) choice: UserActionConfirmationChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingWriterReview {
    pub(super) owner: String,
    pub(super) sleeve: String,
    pub(super) form: WriterActionForm,
    pub(super) confirmation: WriterActionConfirmation,
}

#[derive(Clone, Debug)]
pub(super) struct WriterActionResult {
    pub(super) action: WriterAction,
    pub(super) ok: bool,
    pub(super) payload: Option<Value>,
    pub(super) message: String,
}

pub(super) struct WriterState {
    pub(super) action_selected: usize,
    pub(super) form: Option<WriterActionForm>,
    pub(super) confirmation: Option<WriterActionConfirmation>,
    pub(super) action_result: Option<WriterActionResult>,
    pub(super) action_request: u64,
    pub(super) action_inflight: Option<u64>,
    pub(super) action_mask_request: u64,
    pub(super) action_mask_inflight: Option<u64>,
    pub(super) pending_review: Option<PendingWriterReview>,
}

impl WriterState {
    pub(super) fn new() -> Self {
        Self {
            action_selected: 0,
            form: None,
            confirmation: None,
            action_result: None,
            action_request: 0,
            action_inflight: None,
            action_mask_request: 0,
            action_mask_inflight: None,
            pending_review: None,
        }
    }

    pub(super) fn selected_action(&self) -> WriterAction {
        WriterAction::ALL
            .get(self.action_selected)
            .copied()
            .unwrap_or(WriterAction::Show)
    }

    pub(super) fn push_form_char(&mut self, character: char) {
        if self.action_mask_is_loading() {
            return;
        }
        if character.is_control() {
            return;
        }
        let Some(field) = self
            .form
            .as_mut()
            .and_then(WriterActionForm::selected_field_mut)
        else {
            return;
        };
        let maximum = match field.key {
            "bins" => 512,
            "cursor" => 768,
            _ => 160,
        };
        if field.value.chars().count() < maximum {
            field.value.push(character);
        }
    }

    pub(super) fn backspace_form_input(&mut self) {
        if self.action_mask_is_loading() {
            return;
        }
        if let Some(field) = self
            .form
            .as_mut()
            .and_then(WriterActionForm::selected_field_mut)
        {
            field.value.pop();
        }
    }

    pub(super) fn move_confirmation_choice(&mut self, direction: isize) {
        if self.action_is_running() {
            return;
        }
        if let Some(confirmation) = self.confirmation.as_mut() {
            confirmation.choice = if direction < 0 {
                UserActionConfirmationChoice::Cancel
            } else {
                UserActionConfirmationChoice::Confirm
            };
        }
    }

    pub(super) fn action_is_running(&self) -> bool {
        self.action_inflight.is_some()
    }

    pub(super) fn action_mask_is_loading(&self) -> bool {
        self.action_mask_inflight.is_some()
    }

    pub(super) fn interaction_is_locked(&self) -> bool {
        self.action_is_running() || self.action_mask_is_loading()
    }

    pub(super) fn clear_action_mask_check(&mut self) {
        self.action_mask_request = self.action_mask_request.wrapping_add(1);
        self.action_mask_inflight = None;
        self.pending_review = None;
        self.confirmation = None;
    }
}
