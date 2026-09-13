//! Oracle draft models, fields and local validation, independent of screen rendering.
use super::oracle_model::{OracleAction, OraclePhase, OracleSourceState, OracleUpdateState};
use crate::{oracle_tui::OracleIndexTree, spread_oracle_plan, wallet_balance};
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OracleFormMode {
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
    pub(super) fn from_action(action: OracleAction) -> Option<Self> {
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

    pub(super) fn title(self) -> &'static str {
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

    pub(super) fn modules(self) -> &'static str {
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

    pub(super) fn user_hint(self) -> &'static str {
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

    pub(super) fn spread_action(self) -> Option<&'static str> {
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

    pub(super) fn source_state(self) -> OracleSourceState {
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

    pub(super) fn update_state(self) -> Option<OracleUpdateState> {
        match self {
            Self::UpdateClaim => Some(OracleUpdateState::Submitted),
            Self::Challenge => Some(OracleUpdateState::Challenged),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct OracleFormField {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) required: bool,
    pub(super) editable: bool,
}

impl OracleFormField {
    pub(super) fn editable(label: &'static str, value: impl Into<String>, required: bool) -> Self {
        Self {
            label,
            value: value.into(),
            required,
            editable: true,
        }
    }

    pub(super) fn readonly(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
            required: false,
            editable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct OracleFormDraft {
    pub(super) mode: OracleFormMode,
    pub(super) node_index: usize,
    pub(super) field_selected: usize,
    pub(super) fields: Vec<OracleFormField>,
}

impl OracleFormDraft {
    pub(super) fn new(
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

    pub(super) fn selected_field_mut(&mut self) -> Option<&mut OracleFormField> {
        self.fields.get_mut(self.field_selected)
    }

    pub(super) fn set_field_value(&mut self, label: &str, value: impl Into<String>) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.label == label) {
            field.value = value.into();
        }
    }

    pub(super) fn move_field(&mut self, offset: isize) {
        if self.fields.is_empty() {
            self.field_selected = 0;
            return;
        }
        let len = self.fields.len() as isize;
        let current = self.field_selected.min(self.fields.len() - 1) as isize;
        self.field_selected = (current + offset).clamp(0, len - 1) as usize;
    }

    pub(super) fn validate(&self) -> Result<(), String> {
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

    pub(super) fn summary(&self) -> String {
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

fn is_allowed_v1_source_category(category: &str) -> bool {
    let normalized = category.trim().to_ascii_lowercase();
    [
        "retailer product page",
        "distributor catalog page",
        "manufacturer product or store page",
        "manufacturer page",
        "benchmark / assessment",
        "market price assessment",
        "public api",
        "public api endpoint",
    ]
    .iter()
    .any(|allowed| normalized.contains(allowed))
}
