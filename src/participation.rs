//! Shared typed action grammar for CLI and TUI; SDK remains semantic authority.
use crate::{
    backend::{BackendClient, CliError},
    onchain::OnchainConfig,
    portable_operation::{self, Family},
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, clap::ValueEnum, PartialEq)]
pub enum Action {
    InitializeCollateral,
    DepositCollateral,
    WithdrawCollateral,
    Stake,
    ActivateStake,
    Unstake,
    CompleteUnstake,
    ProposeSource,
    SupportSource,
    ChallengeSource,
    SubmitOpening,
    ChallengeOpening,
    FinalizeOpening,
    CommitUpdate,
    RevealUpdate,
    CommitEmergencyVote,
    RevealEmergencyVote,
    ChallengeUpdate,
    ClaimReward,
}
pub const ACTIONS: &[Action] = &[
    Action::InitializeCollateral,
    Action::DepositCollateral,
    Action::WithdrawCollateral,
    Action::Stake,
    Action::ActivateStake,
    Action::Unstake,
    Action::CompleteUnstake,
    Action::ProposeSource,
    Action::SupportSource,
    Action::ChallengeSource,
    Action::SubmitOpening,
    Action::ChallengeOpening,
    Action::FinalizeOpening,
    Action::CommitUpdate,
    Action::RevealUpdate,
    Action::CommitEmergencyVote,
    Action::RevealEmergencyVote,
    Action::ChallengeUpdate,
    Action::ClaimReward,
];
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::InitializeCollateral => "Initialize collateral",
            Self::DepositCollateral => "Deposit collateral",
            Self::WithdrawCollateral => "Withdraw collateral",
            Self::Stake => "Queue AMBA stake",
            Self::ActivateStake => "Activate queued stake",
            Self::Unstake => "Request sAMBA unstake",
            Self::CompleteUnstake => "Complete unstake",
            Self::ProposeSource => "Propose source",
            Self::SupportSource => "Support source",
            Self::ChallengeSource => "Challenge source",
            Self::SubmitOpening => "Submit opening claim",
            Self::ChallengeOpening => "Challenge opening",
            Self::FinalizeOpening => "Finalize opening",
            Self::CommitUpdate => "Commit source update",
            Self::RevealUpdate => "Reveal saved source update",
            Self::CommitEmergencyVote => "Commit emergency vote",
            Self::RevealEmergencyVote => "Reveal saved emergency vote",
            Self::ChallengeUpdate => "Challenge update",
            Self::ClaimReward => "Claim USDC reward",
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::InitializeCollateral => "init_collateral",
            Self::DepositCollateral => "deposit_collateral",
            Self::WithdrawCollateral => "withdraw_collateral",
            Self::Stake => "queue_stake_amba_for_samba",
            Self::ActivateStake => "activate_queued_stake_amba_for_samba",
            Self::Unstake => "request_unstake_samba",
            Self::CompleteUnstake => "complete_unstake_samba",
            Self::ProposeSource => "propose_oracle_source_v3",
            Self::SupportSource => "support_oracle_source_v3",
            Self::ChallengeSource => "challenge_oracle_source_v2",
            Self::SubmitOpening => "submit_oracle_opening_claim_v2",
            Self::ChallengeOpening => "challenge_oracle_opening_claim_v2",
            Self::FinalizeOpening => "finalize_oracle_opening_claim_v2",
            Self::CommitUpdate => "commit_oracle_update_claim_v3",
            Self::RevealUpdate => "reveal_oracle_update_claim_v3",
            Self::CommitEmergencyVote => "commit_oracle_emergency_vote_v3",
            Self::RevealEmergencyVote => "reveal_oracle_emergency_vote_v2",
            Self::ChallengeUpdate => "challenge_oracle_update_claim_v2",
            Self::ClaimReward => "claim_oracle_usdc_reward",
        }
    }
    pub fn family(self) -> Family {
        if matches!(
            self,
            Self::InitializeCollateral | Self::DepositCollateral | Self::WithdrawCollateral
        ) {
            Family::Collateral
        } else {
            Family::Oracle
        }
    }
    /// Exact request field names, human labels, and optionality. No transaction/account override fields.
    pub fn fields(self) -> &'static [(&'static str, &'static str, bool)] {
        match self {
            Self::InitializeCollateral | Self::CompleteUnstake => &[],
            Self::DepositCollateral | Self::WithdrawCollateral => {
                &[("amount", "USDC amount", false)]
            }
            Self::Stake => &[("ambaAmountAtomic", "AMBA amount (atomic)", false)],
            Self::ActivateStake => &[(
                "minSambaOutAtomic",
                "Minimum sAMBA received (atomic)",
                false,
            )],
            Self::Unstake => &[
                ("sambaAmountAtomic", "sAMBA to unstake (atomic)", false),
                ("minAmbaOutAtomic", "Minimum AMBA received (atomic)", false),
            ],
            Self::ProposeSource => &[
                ("skuId", "SKU", false),
                ("sourceId", "Source ID (64 hex)", false),
                ("bucketId", "Bucket (same SKU)", false),
                ("sourceType", "Source type", false),
                ("canonicalLocator", "Source URL / locator", false),
                ("sourceDefinition", "Source definition", false),
                ("listingBondAtomic", "Listing bond (atomic)", false),
            ],
            Self::SupportSource => &[
                ("skuId", "SKU", false),
                ("sourceId", "Source ID (64 hex)", false),
                ("stakeAtomic", "Support stake (atomic)", false),
            ],
            Self::ChallengeSource => &[
                ("sourceId", "Source ID (64 hex)", false),
                ("challengeId", "Challenge ID (64 hex)", false),
                ("reasonCode", "Reason code", false),
                ("bondAtomic", "Challenge bond (atomic)", false),
                ("evidenceHashHex", "Evidence SHA-256", false),
                (
                    "comparisonSourceId",
                    "Comparison source (reason 8 only)",
                    true,
                ),
            ],
            Self::SubmitOpening => &[
                ("sourceId", "Source ID (64 hex)", false),
                ("openingStateAtomic", "Opening value (atomic)", false),
                ("sourceTimeUnix", "Source time (Unix seconds)", false),
                ("stakeAtomic", "Stake (atomic)", false),
                ("archiveUrl", "Evidence archive URL", false),
            ],
            Self::ChallengeOpening => &[
                ("sourceId", "Source ID (64 hex)", false),
                ("challengeId", "Challenge ID (64 hex)", false),
                (
                    "alternativeOpeningStateAtomic",
                    "Alternative opening (atomic)",
                    false,
                ),
                (
                    "alternativeSourceTimeUnix",
                    "Alternative time (Unix seconds)",
                    false,
                ),
                ("bondAtomic", "Challenge bond (atomic)", false),
                ("archiveUrl", "Evidence archive URL", false),
            ],
            Self::FinalizeOpening => &[("sourceId", "Source ID (64 hex)", false)],
            Self::CommitUpdate => &[
                ("sourceId", "Source ID (64 hex)", false),
                ("claimId", "Claim ID (64 hex)", false),
                ("priorStateAtomic", "Previous value (atomic)", false),
                ("newStateAtomic", "New value (atomic)", false),
                (
                    "sourceTimeUnix",
                    "Original source time (Unix seconds)",
                    false,
                ),
                ("evidenceHashHex", "Evidence SHA-256", false),
                ("archiveUrl", "Evidence archive URL", false),
                ("stakeAtomic", "Stake (atomic)", false),
            ],
            Self::RevealUpdate | Self::RevealEmergencyVote => {
                &[("commitmentId", "Saved private commitment ID", false)]
            }
            Self::CommitEmergencyVote => &[
                ("disputeId", "Current dispute ID (64 hex)", false),
                (
                    "disputeKind",
                    "Dispute kind: source / update / opening",
                    false,
                ),
                ("choice", "Decision (current SDK choice)", false),
                ("sambaAmountAtomic", "sAMBA voting stake (atomic)", false),
            ],
            Self::ChallengeUpdate => &[
                ("sourceId", "Source ID (64 hex)", false),
                ("claimantPubkey", "Current claimant wallet", false),
                ("claimId", "Current claim ID (64 hex)", false),
                ("challengeId", "Challenge ID (64 hex)", false),
                (
                    "alternativeStateAtomic",
                    "Alternative value (atomic)",
                    false,
                ),
                (
                    "alternativeSourceTimeUnix",
                    "Alternative time (Unix seconds)",
                    false,
                ),
                ("bondAtomic", "Challenge bond (atomic)", false),
                ("evidenceHashHex", "Evidence SHA-256", false),
                ("archiveUrl", "Evidence archive URL", false),
            ],
            Self::ClaimReward => &[
                (
                    "rewardKind",
                    "Reward: proposer/support/opening/update",
                    false,
                ),
                ("sourceId", "Source ID (64 hex)", false),
                ("claimId", "Claim ID (update reward only)", true),
            ],
        }
    }
}
#[derive(Debug, clap::Args)]
pub struct ActionArgs {
    #[arg(value_enum)]
    pub action: Action,
    #[arg(long, help = "Product; required for staking and Oracle participation")]
    pub market: Option<String>,
    #[arg(
        long,
        help = "Exact listed series; required for staking and Oracle participation"
    )]
    pub expiry: Option<String>,
    #[arg(
        long = "field",
        value_name = "NAME=VALUE",
        help = "Action field; use --describe to see allowed fields"
    )]
    pub fields: Vec<String>,
    #[arg(
        long,
        help = "Show the action's fields without preparing or accessing a wallet"
    )]
    pub describe: bool,
}
pub fn validate_request(request: &Value) -> Result<(), CliError> {
    let action = request["actionType"].as_str().unwrap_or("");
    if !ACTIONS
        .iter()
        .any(|a| matches!(a.family(), Family::Oracle) && a.wire() == action)
    {
        return Err(CliError::new(
            "This is not a public Oracle participation action.",
        ));
    }
    Ok(())
}
pub fn request(
    action: Action,
    owner: &str,
    market: Option<&str>,
    expiry: Option<&str>,
    fields: &[String],
) -> Result<Value, CliError> {
    let mut result = json!({"actionType":action.wire(),"ownerPubkey":owner});
    if matches!(action.family(), Family::Oracle) {
        let market = market
            .ok_or_else(|| CliError::new("Choose an exact market and series for this action."))?
            .to_ascii_lowercase();
        let expiry = expiry
            .ok_or_else(|| CliError::new("Choose an exact market and series for this action."))?;
        crate::market_surface::parse_current_series_id(&market, expiry)?;
        result["marketId"] = json!(market);
        result["expiryId"] = json!(expiry);
    } else if market.is_some() || expiry.is_some() {
        return Err(CliError::new(
            "Collateral is wallet-scoped; omit market and expiry.",
        ));
    }
    for field in fields {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| CliError::new("Action fields use NAME=VALUE."))?;
        if !action.fields().iter().any(|(name, _, _)| *name == key)
            || result.get(key).is_some()
            || value.is_empty()
            || value.len() > 4096
        {
            return Err(CliError::new(
                "Unknown, duplicate, empty, or oversized action field. Use --describe.",
            ));
        }
        result[key] = if key == "reasonCode" {
            json!(
                value
                    .parse::<u8>()
                    .map_err(|_| CliError::new("Reason code must be an integer from 0 to 255"))?
            )
        } else if key == "amount" {
            json!(crate::trade_service::decimal_atoms(value, 6)?.to_string())
        } else {
            json!(value)
        };
    }
    for (key, label, optional) in action.fields() {
        if !optional && result.get(*key).is_none() {
            return Err(CliError::new(format!("{label} is required ({key}).")));
        }
    }
    Ok(result)
}
pub fn run(
    config: &OnchainConfig,
    backend: &BackendClient,
    args: &ActionArgs,
) -> Result<Value, CliError> {
    if args.describe {
        return Ok(
            json!({"action":args.action.label(),"fields":args.action.fields().iter().map(|(name,label,optional)|json!({"name":name,"label":label,"optional":optional})).collect::<Vec<_>>(),"seriesRequired":matches!(args.action.family(),Family::Oracle)}),
        );
    }
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let owner = crate::wallet_signer::signer_pubkey(config)?;
    let request = request(
        args.action,
        &owner,
        args.market.as_deref(),
        args.expiry.as_deref(),
        &args.fields,
    )?;
    portable_operation::prepare(config, backend, args.action.family(), request)
}

pub fn prepare_staking(
    config: &OnchainConfig,
    backend: &BackendClient,
    action: Action,
    market: Option<&str>,
    expiry: Option<&str>,
    amount: Option<&str>,
    minimum: Option<&str>,
) -> Result<Value, CliError> {
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let owner = crate::wallet_signer::signer_pubkey(config)?;
    let mut fields = Vec::new();
    if !matches!(action, Action::CompleteUnstake) {
        let decimals = crate::staking::status(config, Some(&owner))?
            .decimals
            .ok_or_else(|| CliError::new("Current staking token precision is unavailable."))?;
        let atoms = |value: &str| {
            if value == "0" {
                Ok("0".to_string())
            } else {
                crate::trade_service::decimal_atoms(value, decimals).map(|v| v.to_string())
            }
        };
        match action {
            Action::Stake => fields.push(format!(
                "ambaAmountAtomic={}",
                atoms(amount.ok_or_else(|| CliError::new("AMBA amount is required"))?)?
            )),
            Action::ActivateStake => fields.push(format!(
                "minSambaOutAtomic={}",
                atoms(minimum.ok_or_else(|| CliError::new(
                    "Set --min-received explicitly before preparing activation."
                ))?)?
            )),
            Action::Unstake => {
                fields.push(format!(
                    "sambaAmountAtomic={}",
                    atoms(amount.ok_or_else(|| CliError::new("sAMBA amount is required"))?)?
                ));
                fields.push(format!(
                    "minAmbaOutAtomic={}",
                    atoms(minimum.ok_or_else(|| CliError::new(
                        "Set --min-received explicitly before preparing unstaking."
                    ))?)?
                ));
            }
            _ => return Err(CliError::new("Unsupported staking action")),
        }
    }
    let request = request(action, &owner, market, expiry, &fields)?;
    portable_operation::prepare(config, backend, Family::Oracle, request)
}
