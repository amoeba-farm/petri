use std::{str::FromStr, time::Duration};

use ameba_sdk::{
    constants::{
        CURRENT_STATE_NAMESPACE_SEED, ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED,
        ORACLE_PLAYER_LEDGER_PDA_SEED, ORACLE_REWARD_FUNNEL_PDA_SEED, ORACLE_SAMBA_MINT_PDA_SEED,
        ORACLE_SAMBA_STAKE_ACTIVATION_SECONDS, ORACLE_SAMBA_UNBONDING_SECONDS,
        ORACLE_SAMBA_VOTE_VAULT_PDA_SEED, ORACLE_STAKE_ACTIVATION_PDA_SEED,
        ORACLE_STAKING_POOL_PDA_SEED, ORACLE_UNSTAKE_REQUEST_PDA_SEED, VAULT_PDA_SEED,
    },
    encode_current_vault_instruction,
    instruction::{
        ActivateQueuedStakeAmbaForSambaParams, CancelQueuedStakeAmbaParams,
        CompleteUnstakeSambaParams, QueueStakeAmbaForSambaParams, RequestUnstakeSambaParams,
        VaultInstruction, VaultInstructionTag,
    },
    state::{
        OracleMajorTokenConfig, OraclePlayerLedger, OracleRewardFunnel, OracleStakeActivation,
        OracleStakingPool, OracleUnstakeRequest, VaultConfig,
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use borsh::BorshDeserialize;
use serde::Serialize;
use serde_json::{Value, json};
use solana_instruction::{AccountMeta, Instruction};
use solana_program::{program_option::COption, program_pack::Pack};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::program as system_program;
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

use crate::{
    backend::{CliError, string_at_key, value_at_key},
    onchain::{self, OnchainConfig},
    wallet_signer,
};

// Kept for exact decoder regression coverage while every staking mutation is
// blocked by the static current-release gate.
#[allow(dead_code)]
const REQUEST_UNSTAKE_SAMBA_TAG: u8 = VaultInstructionTag::RequestUnstakeSamba as u8;
#[allow(dead_code)]
const COMPLETE_UNSTAKE_SAMBA_TAG: u8 = VaultInstructionTag::CompleteUnstakeSamba as u8;
#[allow(dead_code)]
const QUEUE_STAKE_AMBA_FOR_SAMBA_TAG: u8 = VaultInstructionTag::QueueStakeAmbaForSamba as u8;
#[allow(dead_code)]
const ACTIVATE_QUEUED_STAKE_AMBA_FOR_SAMBA_TAG: u8 =
    VaultInstructionTag::ActivateQueuedStakeAmbaForSamba as u8;
#[allow(dead_code)]
const CANCEL_QUEUED_STAKE_AMBA_TAG: u8 = VaultInstructionTag::CancelQueuedStakeAmba as u8;
const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DISPLAY_DECIMALS: u8 = 18;
const MAX_CONSOLIDATION_TRANSFERS: usize = 8;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StakingStatus {
    pub available: bool,
    pub owner: String,
    pub availability_note: Option<String>,
    pub decimals: Option<u8>,
    pub available_amba: Option<String>,
    pub available_amba_atoms: Option<String>,
    pub staked_samba: Option<String>,
    pub staked_samba_atoms: Option<String>,
    pub redeemable_amba: Option<String>,
    pub redeemable_amba_atoms: Option<String>,
    pub voting_power_samba: Option<String>,
    pub queued_amba: Option<String>,
    pub queued_amba_atoms: Option<String>,
    pub activate_after_unix: Option<u64>,
    pub seconds_until_activation: Option<u64>,
    pub activation_ready: bool,
    pub can_queue_stake: bool,
    pub can_activate: bool,
    pub can_cancel_queued_stake: bool,
    pub activation_blocked_reason: Option<String>,
    pub reward_funnel_ready: bool,
    pub reward_funnel_empty: bool,
    pub unbonding_amba: Option<String>,
    pub unbonding_amba_atoms: Option<String>,
    pub claimable_at_unix: Option<u64>,
    pub seconds_until_claimable: Option<u64>,
    pub unstake_ready: bool,
    pub can_claim: bool,
    pub exchange_rate: Option<String>,
    pub rewards_added_to_pool_amba: Option<String>,
    pub rewards_added_to_pool_amba_atoms: Option<String>,
    pub rewards_included: bool,
    pub protocol_paused: bool,
    pub supply_changes_locked: bool,
    pub staking_state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StakingTransactionResult {
    pub action: String,
    pub status: String,
    pub owner: String,
    pub signature: String,
    pub amount: Option<String>,
    pub amount_asset: Option<String>,
    pub minimum_received: Option<String>,
    pub received_asset: Option<String>,
    pub activation_after_seconds: Option<u64>,
    pub claimable_after_seconds: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug)]
struct RawAccount {
    owner: Pubkey,
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct TokenHolding {
    address: Pubkey,
    amount: u64,
}

#[derive(Clone, Debug)]
struct StakingContext {
    status: StakingStatus,
    owner: Pubkey,
    decimals: u8,
    pool: OracleStakingPool,
    holdings: Vec<TokenHolding>,
    vault_config: Pubkey,
    major_token_config: Pubkey,
    player_ledger: Pubkey,
    staking_pool: Pubkey,
    samba_mint: Pubkey,
    amba_mint: Pubkey,
    stake_activation: Pubkey,
    reward_funnel: Pubkey,
    reward_funnel_token: Pubkey,
    unstake_request: Pubkey,
    owner_samba_ata: Pubkey,
}

fn load_signing_context(
    config: &OnchainConfig,
) -> Result<(Box<dyn Signer>, StakingContext), CliError> {
    crate::chain_identity::verify_onchain_config(config)?;
    let signer = wallet_signer::load_signer(config)?;
    let owner = signer
        .try_pubkey()
        .map_err(|_| CliError::new("Could not read the attached wallet address."))?;
    let context = require_available_context(load_context_after_identity(config, owner)?)?;
    Ok((signer, context))
}

fn atoms_or_zero(value: &Option<String>) -> u64 {
    value
        .as_deref()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn require_typed_staking_submission() -> Result<(), CliError> {
    crate::current_release::require_current_write_release()?;
    Err(CliError::new(
        "Current typed staking submission is not_wired. Nothing was prepared, signed, or sent.",
    ))
}

pub fn status(
    config: &OnchainConfig,
    owner_override: Option<&str>,
) -> Result<StakingStatus, CliError> {
    crate::chain_identity::verify_onchain_config(config)?;
    let owner = match owner_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(owner) => parse_pubkey("wallet address", owner)?,
        None => {
            let signer = wallet_signer::load_signer(config)?;
            signer
                .try_pubkey()
                .map_err(|_| CliError::new("Could not read the attached wallet address."))?
        }
    };
    Ok(load_context_after_identity(config, owner)?.status)
}

pub fn stake(config: &OnchainConfig, amount: &str) -> Result<StakingTransactionResult, CliError> {
    require_typed_staking_submission()?;
    let (signer, context) = load_signing_context(config)?;
    let owner = context.owner;
    ensure_protocol_active(&context.status)?;
    if atoms_or_zero(&context.status.queued_amba_atoms) != 0 {
        return Err(CliError::new(
            "This wallet already has AMBA queued. Activate or cancel it before queuing another stake.",
        ));
    }

    let amba_amount = parse_token_amount(amount, context.decimals, "AMBA amount")?;
    let available = atoms_or_zero(&context.status.available_amba_atoms);
    if amba_amount > available {
        return Err(CliError::new(format!(
            "Only {} AMBA is available to stake.",
            format_token_amount(available, context.decimals)
        )));
    }

    let stake_instruction = Instruction {
        program_id: ameba_sdk::ID,
        accounts: vec![
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(context.vault_config, false),
            AccountMeta::new_readonly(context.major_token_config, false),
            AccountMeta::new(context.player_ledger, false),
            AccountMeta::new_readonly(context.staking_pool, false),
            AccountMeta::new(context.stake_activation, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: serialize_staking_instruction(VaultInstruction::QueueStakeAmbaForSamba {
            params: QueueStakeAmbaForSambaParams { amba_amount },
        })?,
    };
    let signature = sign_submit_exact(
        config,
        signer.as_ref(),
        owner,
        context.amba_mint,
        vec![stake_instruction],
    )?;

    Ok(StakingTransactionResult {
        action: "stake".to_string(),
        status: "submitted".to_string(),
        owner: owner.to_string(),
        signature,
        amount: Some(format_token_amount(amba_amount, context.decimals)),
        amount_asset: Some("AMBA".to_string()),
        minimum_received: None,
        received_asset: None,
        activation_after_seconds: Some(ORACLE_SAMBA_STAKE_ACTIVATION_SECONDS),
        claimable_after_seconds: None,
        message: "AMBA queued for staking. It can be activated after seven days; queued AMBA earns no rewards and provides no voting power.".to_string(),
    })
}

pub fn activate(
    config: &OnchainConfig,
    min_received: Option<&str>,
) -> Result<StakingTransactionResult, CliError> {
    require_typed_staking_submission()?;
    let (signer, context) = load_signing_context(config)?;
    let owner = context.owner;
    let queued_amba = atoms_or_zero(&context.status.queued_amba_atoms);
    if queued_amba == 0 {
        return Err(CliError::new("There is no queued AMBA to activate."));
    }
    ensure_protocol_active(&context.status)?;
    ensure_supply_changes_open(&context.status)?;
    if !context.status.activation_ready {
        return Err(CliError::new(format!(
            "This AMBA is still in its seven-day staking activation wait. Try again in {}.",
            format_duration(context.status.seconds_until_activation.unwrap_or(0))
        )));
    }
    if !context.status.reward_funnel_ready {
        return Err(CliError::new(
            "Staking activation is waiting for the canonical reward intake to be initialized.",
        ));
    }
    if !context.status.reward_funnel_empty {
        return Err(CliError::new(
            "Staking activation is waiting for pending AMBA rewards to be included in the share rate.",
        ));
    }

    let quoted_samba = calculate_samba_for_amba(
        queued_amba,
        context.pool.active_amba_backing,
        context.pool.samba_supply,
    )?;
    let minimum_samba = parse_minimum(
        min_received,
        context.decimals,
        quoted_samba,
        "minimum sAMBA received",
    )?;
    let ata_instruction =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &owner,
            &owner,
            &context.samba_mint,
            &spl_token::id(),
        );
    let activation_instruction = Instruction {
        program_id: ameba_sdk::ID,
        accounts: vec![
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new_readonly(context.vault_config, false),
            AccountMeta::new_readonly(context.major_token_config, false),
            AccountMeta::new(context.staking_pool, false),
            AccountMeta::new(context.stake_activation, false),
            AccountMeta::new_readonly(context.reward_funnel, false),
            AccountMeta::new_readonly(context.reward_funnel_token, false),
            AccountMeta::new(context.samba_mint, false),
            AccountMeta::new(context.owner_samba_ata, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: serialize_staking_instruction(VaultInstruction::ActivateQueuedStakeAmbaForSamba {
            params: ActivateQueuedStakeAmbaForSambaParams {
                min_samba_out: minimum_samba,
            },
        })?,
    };
    let signature = sign_submit_exact(
        config,
        signer.as_ref(),
        owner,
        context.amba_mint,
        vec![ata_instruction, activation_instruction],
    )?;

    Ok(StakingTransactionResult {
        action: "activate".to_string(),
        status: "submitted".to_string(),
        owner: owner.to_string(),
        signature,
        amount: Some(format_token_amount(queued_amba, context.decimals)),
        amount_asset: Some("AMBA".to_string()),
        minimum_received: Some(format_token_amount(minimum_samba, context.decimals)),
        received_asset: Some("sAMBA".to_string()),
        activation_after_seconds: None,
        claimable_after_seconds: None,
        message: "Queued AMBA activated and sAMBA minted at the current share rate. Future rewards are included in redeemable AMBA.".to_string(),
    })
}

pub fn cancel(config: &OnchainConfig) -> Result<StakingTransactionResult, CliError> {
    require_typed_staking_submission()?;
    let (signer, context) = load_signing_context(config)?;
    let owner = context.owner;
    let queued_amba = atoms_or_zero(&context.status.queued_amba_atoms);
    if queued_amba == 0 {
        return Err(CliError::new("There is no queued AMBA to cancel."));
    }
    let instruction = Instruction {
        program_id: ameba_sdk::ID,
        accounts: vec![
            AccountMeta::new_readonly(owner, true),
            AccountMeta::new_readonly(context.vault_config, false),
            AccountMeta::new(context.player_ledger, false),
            AccountMeta::new(context.stake_activation, false),
        ],
        data: serialize_staking_instruction(VaultInstruction::CancelQueuedStakeAmba {
            params: CancelQueuedStakeAmbaParams {},
        })?,
    };
    let signature = sign_submit_exact(
        config,
        signer.as_ref(),
        owner,
        context.amba_mint,
        vec![instruction],
    )?;

    Ok(StakingTransactionResult {
        action: "cancel".to_string(),
        status: "submitted".to_string(),
        owner: owner.to_string(),
        signature,
        amount: Some(format_token_amount(queued_amba, context.decimals)),
        amount_asset: Some("AMBA".to_string()),
        minimum_received: None,
        received_asset: Some("AMBA".to_string()),
        activation_after_seconds: None,
        claimable_after_seconds: None,
        message:
            "Queued staking cancelled. The AMBA was returned to the wallet's available balance."
                .to_string(),
    })
}

pub fn unstake(
    config: &OnchainConfig,
    amount: &str,
    min_received: Option<&str>,
) -> Result<StakingTransactionResult, CliError> {
    require_typed_staking_submission()?;
    let (signer, context) = load_signing_context(config)?;
    let owner = context.owner;
    ensure_protocol_active(&context.status)?;
    ensure_supply_changes_open(&context.status)?;
    if atoms_or_zero(&context.status.unbonding_amba_atoms) != 0 {
        return Err(CliError::new(
            "This wallet already has AMBA unbonding. Claim it when ready before starting another unstake.",
        ));
    }

    let samba_amount = parse_token_amount(amount, context.decimals, "sAMBA amount")?;
    let total_samba = context
        .holdings
        .iter()
        .try_fold(0_u64, |total, holding| total.checked_add(holding.amount))
        .ok_or_else(|| CliError::new("The sAMBA balance is too large to display safely."))?;
    if samba_amount > total_samba {
        return Err(CliError::new(format!(
            "Only {} sAMBA is available to unstake.",
            format_token_amount(total_samba, context.decimals)
        )));
    }
    let quoted_amba = calculate_amba_for_samba(
        samba_amount,
        context.pool.active_amba_backing,
        context.pool.samba_supply,
    )?;
    let minimum_amba = parse_minimum(
        min_received,
        context.decimals,
        quoted_amba,
        "minimum AMBA received",
    )?;

    let mut instructions = vec![
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &owner,
            &owner,
            &context.samba_mint,
            &spl_token::id(),
        ),
    ];
    append_consolidation_instructions(&context, samba_amount, &mut instructions)?;
    instructions.push(Instruction {
        program_id: ameba_sdk::ID,
        accounts: vec![
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(context.vault_config, false),
            AccountMeta::new_readonly(context.major_token_config, false),
            AccountMeta::new(context.staking_pool, false),
            AccountMeta::new(context.unstake_request, false),
            AccountMeta::new(context.samba_mint, false),
            AccountMeta::new(context.owner_samba_ata, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: serialize_staking_instruction(VaultInstruction::RequestUnstakeSamba {
            params: RequestUnstakeSambaParams {
                samba_amount,
                min_amba_out: minimum_amba,
            },
        })?,
    });
    let signature = sign_submit_exact(
        config,
        signer.as_ref(),
        owner,
        context.amba_mint,
        instructions,
    )?;

    Ok(StakingTransactionResult {
        action: "unstake".to_string(),
        status: "submitted".to_string(),
        owner: owner.to_string(),
        signature,
        amount: Some(format_token_amount(samba_amount, context.decimals)),
        amount_asset: Some("sAMBA".to_string()),
        minimum_received: Some(format_token_amount(minimum_amba, context.decimals)),
        received_asset: Some("AMBA".to_string()),
        activation_after_seconds: None,
        claimable_after_seconds: Some(ORACLE_SAMBA_UNBONDING_SECONDS),
        message: "Unstaking started. The AMBA amount, including embedded staking rewards, can be claimed after seven days.".to_string(),
    })
}

pub fn claim(config: &OnchainConfig) -> Result<StakingTransactionResult, CliError> {
    require_typed_staking_submission()?;
    let (signer, context) = load_signing_context(config)?;
    let owner = context.owner;
    ensure_protocol_active(&context.status)?;
    let pending = atoms_or_zero(&context.status.unbonding_amba_atoms);
    if pending == 0 {
        return Err(CliError::new("There is no unbonding AMBA to claim."));
    }
    if !context.status.can_claim {
        let remaining = context.status.seconds_until_claimable.unwrap_or(0);
        return Err(CliError::new(format!(
            "This AMBA is still unbonding. Try again in {}.",
            format_duration(remaining)
        )));
    }

    let instruction = Instruction {
        program_id: ameba_sdk::ID,
        accounts: vec![
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(context.vault_config, false),
            AccountMeta::new(context.player_ledger, false),
            AccountMeta::new(context.staking_pool, false),
            AccountMeta::new(context.unstake_request, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: serialize_staking_instruction(VaultInstruction::CompleteUnstakeSamba {
            params: CompleteUnstakeSambaParams {},
        })?,
    };
    let signature = sign_submit_exact(
        config,
        signer.as_ref(),
        owner,
        context.amba_mint,
        vec![instruction],
    )?;

    Ok(StakingTransactionResult {
        action: "claim".to_string(),
        status: "submitted".to_string(),
        owner: owner.to_string(),
        signature,
        amount: Some(format_token_amount(pending, context.decimals)),
        amount_asset: Some("AMBA".to_string()),
        minimum_received: None,
        received_asset: None,
        activation_after_seconds: None,
        claimable_after_seconds: None,
        message: "Unstaked AMBA claimed and returned to the wallet's available AMBA balance."
            .to_string(),
    })
}

pub fn render_status(status: &StakingStatus) -> String {
    if !status.available {
        return [
            "staking=not available".to_string(),
            status.availability_note.clone().unwrap_or_else(|| {
                "Staking is not available on this Amoeba deployment yet.".to_string()
            }),
        ]
        .join("\n");
    }
    let available = status.available_amba.as_deref().unwrap_or("-");
    let staked = status.staked_samba.as_deref().unwrap_or("-");
    let redeemable = status.redeemable_amba.as_deref().unwrap_or("-");
    let queued = status.queued_amba.as_deref().unwrap_or("-");
    let unbonding = status.unbonding_amba.as_deref().unwrap_or("-");
    let activation_state = if status.can_activate {
        "ready to activate".to_string()
    } else if let Some(seconds) = status.seconds_until_activation.filter(|value| *value > 0) {
        format!("activation in {}", format_duration(seconds))
    } else if status
        .queued_amba_atoms
        .as_deref()
        .is_some_and(|value| value != "0")
    {
        status
            .activation_blocked_reason
            .clone()
            .unwrap_or_else(|| "waiting to activate".to_string())
    } else {
        "none".to_string()
    };
    let claim_state = if status.unstake_ready && status.protocol_paused {
        "ready after staking resumes".to_string()
    } else if status.can_claim {
        "ready to claim".to_string()
    } else if let Some(seconds) = status.seconds_until_claimable.filter(|value| *value > 0) {
        format!("claimable in {}", format_duration(seconds))
    } else {
        "none".to_string()
    };
    let staking_state = if status.protocol_paused {
        "temporarily paused"
    } else if status.supply_changes_locked {
        "paused while a vote is resolved"
    } else {
        "open"
    };
    [
        format!("staking={staking_state} | wallet={}", status.owner),
        format!("available AMBA={available} | sAMBA={staked} | redeemable AMBA={redeemable}"),
        format!("queued AMBA={queued} | {activation_state}"),
        format!("unbonding AMBA={unbonding} | {claim_state}"),
        format!(
            "exchange rate=1 sAMBA = {} AMBA | rewards are included in redeemable AMBA; there is no separate staking-reward claim",
            status.exchange_rate.as_deref().unwrap_or("-")
        ),
    ]
    .join("\n")
}

pub fn render_transaction(result: &StakingTransactionResult) -> String {
    [
        format!("staking {}={}", result.action, result.status),
        result.message.clone(),
        format!("signature={}", result.signature),
    ]
    .join("\n")
}

fn require_available_context(context: StakingContext) -> Result<StakingContext, CliError> {
    if context.status.available {
        Ok(context)
    } else {
        Err(CliError::new(
            context.status.availability_note.unwrap_or_else(|| {
                "Staking is not available on this Amoeba deployment yet.".to_string()
            }),
        ))
    }
}

fn ensure_supply_changes_open(status: &StakingStatus) -> Result<(), CliError> {
    if status.supply_changes_locked {
        return Err(CliError::new(
            "Staking and unstaking are paused while an emergency vote is being resolved.",
        ));
    }
    Ok(())
}

fn ensure_protocol_active(status: &StakingStatus) -> Result<(), CliError> {
    if status.protocol_paused {
        return Err(CliError::new(
            "Staking actions are temporarily paused for this Amoeba deployment.",
        ));
    }
    Ok(())
}

fn derive_current_pda(program_id: &Pubkey, seeds: &[&[u8]]) -> (Pubkey, u8) {
    let mut namespaced = Vec::with_capacity(seeds.len() + 1);
    namespaced.push(CURRENT_STATE_NAMESPACE_SEED);
    namespaced.extend_from_slice(seeds);
    Pubkey::find_program_address(&namespaced, program_id)
}

fn load_context_after_identity(
    config: &OnchainConfig,
    owner: Pubkey,
) -> Result<StakingContext, CliError> {
    let program_id = ameba_sdk::ID;
    let (vault_config, vault_config_bump) = derive_current_pda(&program_id, &[VAULT_PDA_SEED]);
    let (major_token_config, major_config_bump) =
        derive_current_pda(&program_id, &[ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED]);
    let (player_ledger, player_bump) = derive_current_pda(
        &program_id,
        &[ORACLE_PLAYER_LEDGER_PDA_SEED, owner.as_ref()],
    );
    let (staking_pool, pool_bump) =
        derive_current_pda(&program_id, &[ORACLE_STAKING_POOL_PDA_SEED]);
    let (samba_mint, _) = derive_current_pda(&program_id, &[ORACLE_SAMBA_MINT_PDA_SEED]);
    let (samba_vote_vault, _) =
        derive_current_pda(&program_id, &[ORACLE_SAMBA_VOTE_VAULT_PDA_SEED]);
    let (stake_activation, activation_bump) = derive_current_pda(
        &program_id,
        &[ORACLE_STAKE_ACTIVATION_PDA_SEED, owner.as_ref()],
    );
    let (reward_funnel, reward_funnel_bump) =
        derive_current_pda(&program_id, &[ORACLE_REWARD_FUNNEL_PDA_SEED]);
    let (unstake_request, unstake_bump) = derive_current_pda(
        &program_id,
        &[ORACLE_UNSTAKE_REQUEST_PDA_SEED, owner.as_ref()],
    );
    let owner_samba_ata =
        spl_associated_token_account::get_associated_token_address(&owner, &samba_mint);

    let accounts = fetch_accounts(
        config,
        &[
            staking_pool,
            samba_mint,
            player_ledger,
            unstake_request,
            stake_activation,
            major_token_config,
            solana_program::sysvar::clock::ID,
            vault_config,
        ],
    )?;
    let Some(pool_account) = accounts[0].as_ref() else {
        return Ok(StakingContext {
            status: StakingStatus {
                available: false,
                owner: owner.to_string(),
                availability_note: Some(
                    "Staking is not available on this Amoeba deployment yet.".to_string(),
                ),
                decimals: None,
                available_amba: None,
                available_amba_atoms: None,
                staked_samba: None,
                staked_samba_atoms: None,
                redeemable_amba: None,
                redeemable_amba_atoms: None,
                voting_power_samba: None,
                queued_amba: None,
                queued_amba_atoms: None,
                activate_after_unix: None,
                seconds_until_activation: None,
                activation_ready: false,
                can_queue_stake: false,
                can_activate: false,
                can_cancel_queued_stake: false,
                activation_blocked_reason: None,
                reward_funnel_ready: false,
                reward_funnel_empty: false,
                unbonding_amba: None,
                unbonding_amba_atoms: None,
                claimable_at_unix: None,
                seconds_until_claimable: None,
                unstake_ready: false,
                can_claim: false,
                exchange_rate: None,
                rewards_added_to_pool_amba: None,
                rewards_added_to_pool_amba_atoms: None,
                rewards_included: true,
                protocol_paused: false,
                supply_changes_locked: false,
                staking_state: "not_available".to_string(),
            },
            owner,
            decimals: 0,
            pool: OracleStakingPool::default(),
            holdings: Vec::new(),
            vault_config,
            major_token_config,
            player_ledger,
            staking_pool,
            samba_mint,
            amba_mint: Pubkey::default(),
            stake_activation,
            reward_funnel,
            reward_funnel_token: Pubkey::default(),
            unstake_request,
            owner_samba_ata,
        });
    };
    require_program_account(pool_account, "staking pool")?;
    let mut pool = decode_staking_pool(
        &pool_account.data,
        pool_bump,
        major_token_config,
        samba_mint,
        samba_vote_vault,
    )?;

    let vault_config_account = accounts[7]
        .as_ref()
        .ok_or_else(|| CliError::new("The verified Amoeba vault configuration is missing."))?;
    require_program_account(vault_config_account, "Amoeba vault configuration")?;
    let protocol_paused = decode_vault_config(&vault_config_account.data, vault_config_bump)?;

    let mint_account = accounts[1]
        .as_ref()
        .ok_or_else(|| CliError::new("The sAMBA mint is missing from this deployment."))?;
    if mint_account.owner != spl_token::id() {
        return Err(CliError::new(
            "The sAMBA mint does not belong to the classic token program.",
        ));
    }
    let mint = Mint::unpack(&mint_account.data)
        .map_err(|_| CliError::new("The sAMBA mint has an invalid account layout."))?;
    if !mint.is_initialized
        || mint.mint_authority != COption::Some(vault_config)
        || mint.freeze_authority != COption::None
    {
        return Err(CliError::new(
            "The sAMBA mint does not match the verified Amoeba staking pool.",
        ));
    }
    if mint.decimals > MAX_DISPLAY_DECIMALS {
        return Err(CliError::new(
            "The sAMBA mint uses an unsupported display precision.",
        ));
    }

    let major_config_account = accounts[5]
        .as_ref()
        .ok_or_else(|| CliError::new("The AMBA staking configuration is missing."))?;
    require_program_account(major_config_account, "AMBA staking configuration")?;
    let amba_mint = decode_major_token_config(&major_config_account.data, major_config_bump)?;
    let amba_mint_account = fetch_accounts(config, &[amba_mint])?
        .into_iter()
        .next()
        .flatten()
        .ok_or_else(|| CliError::new("The AMBA mint is missing from this deployment."))?;
    if amba_mint_account.owner != spl_token::id() {
        return Err(CliError::new(
            "The AMBA mint does not belong to the classic token program.",
        ));
    }
    let amba_mint_state = Mint::unpack(&amba_mint_account.data)
        .map_err(|_| CliError::new("The AMBA mint has an invalid account layout."))?;
    if !amba_mint_state.is_initialized || amba_mint_state.decimals != mint.decimals {
        return Err(CliError::new(
            "AMBA and sAMBA do not use the same token precision.",
        ));
    }
    let reward_funnel_token =
        spl_associated_token_account::get_associated_token_address(&reward_funnel, &amba_mint);
    let reward_accounts = fetch_accounts(config, &[reward_funnel, reward_funnel_token])?;
    let reward_funnel_token_state = reward_accounts[1]
        .as_ref()
        .map(|account| {
            decode_reward_funnel_token_account(
                account,
                reward_funnel,
                amba_mint,
                "staking reward intake",
            )
        })
        .transpose()?;
    let reward_funnel_ready = match reward_accounts[0].as_ref() {
        Some(funnel_account) => {
            require_program_account(funnel_account, "staking reward intake")?;
            let token_state = reward_funnel_token_state.as_ref().ok_or_else(|| {
                CliError::new("The staking reward intake token account is missing.")
            })?;
            decode_reward_funnel(
                &funnel_account.data,
                reward_funnel_bump,
                major_token_config,
                amba_mint,
                reward_funnel_token,
            )?;
            token_state.state == AccountState::Initialized
        }
        None => false,
    };
    let reward_funnel_empty = reward_funnel_token_state
        .as_ref()
        .is_some_and(|account| account.amount == 0);
    if mint.supply > pool.samba_supply {
        return Err(CliError::new(
            "The live sAMBA supply is larger than the staking pool record.",
        ));
    }
    // A holder can burn classic SPL shares directly. The program projects that lower live supply
    // into the pool before its next mint or burn; use the same projected supply for status/quotes.
    if mint.supply < pool.samba_supply {
        if mint.supply == 0 {
            pool.active_amba_backing = 0;
        }
        pool.samba_supply = mint.supply;
    }

    let available_amba = if let Some(ledger_account) = accounts[2].as_ref() {
        require_program_account(ledger_account, "available AMBA balance")?;
        decode_player_ledger(&ledger_account.data, player_bump, owner)?
    } else {
        0
    };
    let request = if let Some(request_account) = accounts[3].as_ref() {
        require_program_account(request_account, "unstaking record")?;
        Some(decode_unstake_request(
            &request_account.data,
            unstake_bump,
            owner,
        )?)
    } else {
        None
    };
    let activation = if let Some(activation_account) = accounts[4].as_ref() {
        require_program_account(activation_account, "queued staking record")?;
        Some(decode_stake_activation(
            &activation_account.data,
            activation_bump,
            owner,
        )?)
    } else {
        None
    };
    let clock_account = accounts[6]
        .as_ref()
        .ok_or_else(|| CliError::new("The network clock is unavailable right now."))?;
    if clock_account.owner != solana_program::sysvar::ID {
        return Err(CliError::new(
            "The network clock does not match the selected Solana deployment.",
        ));
    }
    let chain_timestamp = read_i64(&clock_account.data, 32)?;
    if chain_timestamp < 0 {
        return Err(CliError::new("The network clock returned an invalid time."));
    }
    let holdings = fetch_token_holdings(config, owner, samba_mint)?;
    let samba_balance = holdings
        .iter()
        .try_fold(0_u64, |total, holding| total.checked_add(holding.amount))
        .ok_or_else(|| CliError::new("The sAMBA balance is too large to display safely."))?;
    if samba_balance > pool.samba_supply {
        return Err(CliError::new(
            "The wallet sAMBA balance is larger than the live sAMBA supply.",
        ));
    }
    let redeemable_amba = if samba_balance == 0 {
        0
    } else {
        calculate_amba_for_samba(samba_balance, pool.active_amba_backing, pool.samba_supply)?
    };
    let pending_amba = request
        .as_ref()
        .map(|value| value.pending_amba)
        .unwrap_or(0);
    if pending_amba > pool.pending_unstake_amba {
        return Err(CliError::new(
            "The wallet's unbonding AMBA is larger than the staking pool reserve.",
        ));
    }
    let claimable_at = request
        .as_ref()
        .filter(|value| value.pending_amba != 0)
        .map(|value| value.claimable_at_ts);
    let now = chain_timestamp as u64;
    let queued_amba = activation
        .as_ref()
        .map(|value| value.queued_amba)
        .unwrap_or(0);
    let activate_after = activation
        .as_ref()
        .filter(|value| value.queued_amba != 0)
        .map(|value| value.activate_after_ts);
    let activation_ready = activate_after.is_some_and(|timestamp| now >= timestamp);
    let seconds_until_activation = activate_after.map(|timestamp| timestamp.saturating_sub(now));
    let unstake_ready = claimable_at.is_some_and(|timestamp| now >= timestamp);
    let can_claim = unstake_ready && !protocol_paused;
    let seconds_until_claimable = claimable_at.map(|timestamp| timestamp.saturating_sub(now));
    let decimals = mint.decimals;
    let exchange_rate = if pool.samba_supply == 0 {
        "1".to_string()
    } else {
        format_ratio(pool.active_amba_backing, pool.samba_supply)
    };
    let supply_changes_locked = pool.governance_lock_count != 0;
    let can_queue_stake = queued_amba == 0 && available_amba != 0 && !protocol_paused;
    let can_activate = queued_amba != 0
        && activation_ready
        && !protocol_paused
        && !supply_changes_locked
        && reward_funnel_ready
        && reward_funnel_empty;
    let can_cancel_queued_stake = queued_amba != 0;
    let activation_blocked_reason = if queued_amba == 0 {
        None
    } else if protocol_paused {
        Some("Activation is paused for this Amoeba deployment.".to_string())
    } else if !activation_ready {
        Some("Queued AMBA is still in its seven-day activation wait.".to_string())
    } else if supply_changes_locked {
        Some("Activation is paused while an emergency vote is unresolved.".to_string())
    } else if !reward_funnel_ready {
        Some("Activation is waiting for the staking reward intake to be initialized.".to_string())
    } else if !reward_funnel_empty {
        Some(
            "Activation is waiting for pending rewards to be included in the share rate."
                .to_string(),
        )
    } else {
        None
    };
    let status = StakingStatus {
        available: true,
        owner: owner.to_string(),
        availability_note: None,
        decimals: Some(decimals),
        available_amba: Some(format_token_amount(available_amba, decimals)),
        available_amba_atoms: Some(available_amba.to_string()),
        staked_samba: Some(format_token_amount(samba_balance, decimals)),
        staked_samba_atoms: Some(samba_balance.to_string()),
        redeemable_amba: Some(format_token_amount(redeemable_amba, decimals)),
        redeemable_amba_atoms: Some(redeemable_amba.to_string()),
        voting_power_samba: Some(format_token_amount(samba_balance, decimals)),
        queued_amba: Some(format_token_amount(queued_amba, decimals)),
        queued_amba_atoms: Some(queued_amba.to_string()),
        activate_after_unix: activate_after,
        seconds_until_activation,
        activation_ready,
        can_queue_stake,
        can_activate,
        can_cancel_queued_stake,
        activation_blocked_reason,
        reward_funnel_ready,
        reward_funnel_empty,
        unbonding_amba: Some(format_token_amount(pending_amba, decimals)),
        unbonding_amba_atoms: Some(pending_amba.to_string()),
        claimable_at_unix: claimable_at,
        seconds_until_claimable,
        unstake_ready,
        can_claim,
        exchange_rate: Some(exchange_rate),
        rewards_added_to_pool_amba: Some(format_token_amount(pool.total_rewards_funded, decimals)),
        rewards_added_to_pool_amba_atoms: Some(pool.total_rewards_funded.to_string()),
        rewards_included: true,
        protocol_paused,
        supply_changes_locked,
        staking_state: if protocol_paused {
            "protocol_paused".to_string()
        } else if can_activate {
            "activation_ready".to_string()
        } else if queued_amba != 0 {
            "activation_wait".to_string()
        } else if supply_changes_locked {
            "paused_for_vote".to_string()
        } else {
            "open".to_string()
        },
    };

    Ok(StakingContext {
        status,
        owner,
        decimals,
        pool,
        holdings,
        vault_config,
        major_token_config,
        player_ledger,
        staking_pool,
        samba_mint,
        amba_mint,
        stake_activation,
        reward_funnel,
        reward_funnel_token,
        unstake_request,
        owner_samba_ata,
    })
}

fn require_program_account(account: &RawAccount, label: &str) -> Result<(), CliError> {
    if account.owner != ameba_sdk::ID {
        return Err(CliError::new(format!(
            "The {label} does not belong to the verified Amoeba program."
        )));
    }
    Ok(())
}

fn decode_sdk_state<T: BorshDeserialize>(
    data: &[u8],
    expected_len: usize,
    invalid_message: &'static str,
) -> Result<T, CliError> {
    if data.len() != expected_len {
        return Err(CliError::new(invalid_message));
    }
    let mut remaining = data;
    let value = T::deserialize(&mut remaining).map_err(|_| CliError::new(invalid_message))?;
    if remaining.iter().any(|byte| *byte != 0) {
        return Err(CliError::new(invalid_message));
    }
    Ok(value)
}

fn decode_vault_config(data: &[u8], expected_bump: u8) -> Result<bool, CliError> {
    let config: VaultConfig = decode_sdk_state(
        data,
        VaultConfig::LEN,
        "The Amoeba vault configuration has an invalid account layout.",
    )?;
    if !config.is_initialized || config.bump != expected_bump || !config.has_current_layout() {
        return Err(CliError::new(
            "The Amoeba vault configuration has an invalid account layout.",
        ));
    }
    Ok(config.paused)
}

fn decode_staking_pool(
    data: &[u8],
    expected_bump: u8,
    expected_major_config: Pubkey,
    expected_mint: Pubkey,
    expected_vote_vault: Pubkey,
) -> Result<OracleStakingPool, CliError> {
    let pool: OracleStakingPool = decode_sdk_state(
        data,
        OracleStakingPool::LEN,
        "The staking pool has an invalid account layout.",
    )?;
    if !pool.is_initialized
        || pool.bump != expected_bump
        || pool.account_discriminator != OracleStakingPool::ACCOUNT_DISCRIMINATOR
        || pool.account_version != OracleStakingPool::ACCOUNT_VERSION
    {
        return Err(CliError::new(
            "The staking pool has an invalid account layout.",
        ));
    }
    if pool.major_token_config != expected_major_config
        || pool.samba_mint != expected_mint
        || pool.samba_vote_vault != expected_vote_vault
    {
        return Err(CliError::new(
            "The staking pool does not match the verified Amoeba deployment.",
        ));
    }
    if (pool.active_amba_backing == 0) != (pool.samba_supply == 0) {
        return Err(CliError::new(
            "The staking pool has an invalid AMBA/sAMBA exchange rate.",
        ));
    }
    Ok(pool)
}

fn decode_reward_funnel(
    data: &[u8],
    expected_bump: u8,
    expected_major_config: Pubkey,
    expected_amba_mint: Pubkey,
    expected_token_account: Pubkey,
) -> Result<(), CliError> {
    let funnel: OracleRewardFunnel = decode_sdk_state(
        data,
        OracleRewardFunnel::LEN,
        "The staking reward intake has an invalid account layout.",
    )?;
    if !funnel.is_initialized
        || funnel.bump != expected_bump
        || funnel.account_discriminator != OracleRewardFunnel::ACCOUNT_DISCRIMINATOR
        || funnel.account_version != OracleRewardFunnel::ACCOUNT_VERSION
        || funnel.major_token_config != expected_major_config
        || funnel.amba_mint != expected_amba_mint
        || funnel.funnel_token_account != expected_token_account
    {
        return Err(CliError::new(
            "The staking reward intake has an invalid account layout.",
        ));
    }
    let allocated = [
        funnel.total_game_funded,
        funnel.total_scramble_funded,
        funnel.total_challenge_funded,
        funnel.total_staking_funded,
        funnel.total_reserve_funded,
    ]
    .into_iter()
    .try_fold(0_u128, |total, amount| total.checked_add(amount))
    .ok_or_else(|| CliError::new("The staking reward intake totals are too large."))?;
    if allocated != funnel.total_swept {
        return Err(CliError::new(
            "The staking reward intake totals do not match.",
        ));
    }
    Ok(())
}

fn decode_reward_funnel_token_account(
    account: &RawAccount,
    expected_owner: Pubkey,
    expected_mint: Pubkey,
    label: &str,
) -> Result<TokenAccount, CliError> {
    if account.owner != spl_token::id() {
        return Err(CliError::new(format!(
            "The {label} token account does not belong to the classic token program."
        )));
    }
    let token = TokenAccount::unpack(&account.data)
        .map_err(|_| CliError::new(format!("The {label} token account is invalid.")))?;
    if token.state != AccountState::Initialized
        || token.owner != expected_owner
        || token.mint != expected_mint
        || token.delegate != COption::None
        || token.delegated_amount != 0
        || token.is_native != COption::None
        || token.close_authority != COption::None
    {
        return Err(CliError::new(format!(
            "The {label} token account does not match the verified staking deployment."
        )));
    }
    Ok(token)
}

fn decode_player_ledger(
    data: &[u8],
    expected_bump: u8,
    expected_owner: Pubkey,
) -> Result<u64, CliError> {
    let ledger: OraclePlayerLedger = decode_sdk_state(
        data,
        OraclePlayerLedger::LEN,
        "The wallet's available AMBA balance has an invalid account layout.",
    )?;
    if !ledger.is_initialized || ledger.bump != expected_bump || ledger.owner != expected_owner {
        return Err(CliError::new(
            "The wallet's available AMBA balance has an invalid account layout.",
        ));
    }
    Ok(ledger.major_tokens)
}

fn decode_major_token_config(data: &[u8], expected_bump: u8) -> Result<Pubkey, CliError> {
    let config: OracleMajorTokenConfig = decode_sdk_state(
        data,
        OracleMajorTokenConfig::LEN,
        "The AMBA staking configuration has an invalid account layout.",
    )?;
    if !config.is_initialized || config.bump != expected_bump {
        return Err(CliError::new(
            "The AMBA staking configuration has an invalid account layout.",
        ));
    }
    if config.mint == Pubkey::default() || config.vault_token_account == Pubkey::default() {
        return Err(CliError::new(
            "The AMBA staking configuration is incomplete.",
        ));
    }
    Ok(config.mint)
}

fn decode_unstake_request(
    data: &[u8],
    expected_bump: u8,
    expected_owner: Pubkey,
) -> Result<OracleUnstakeRequest, CliError> {
    let request: OracleUnstakeRequest = decode_sdk_state(
        data,
        OracleUnstakeRequest::LEN,
        "The wallet's unstaking record has an invalid account layout.",
    )?;
    if !request.is_initialized
        || request.bump != expected_bump
        || request.account_discriminator != OracleUnstakeRequest::ACCOUNT_DISCRIMINATOR
        || request.account_version != OracleUnstakeRequest::ACCOUNT_VERSION
        || request.owner != expected_owner
    {
        return Err(CliError::new(
            "The wallet's unstaking record has an invalid account layout.",
        ));
    }
    if (request.pending_amba == 0) != (request.claimable_at_ts == 0) {
        return Err(CliError::new(
            "The wallet's unstaking amount and claim time do not match.",
        ));
    }
    Ok(request)
}

fn decode_stake_activation(
    data: &[u8],
    expected_bump: u8,
    expected_owner: Pubkey,
) -> Result<OracleStakeActivation, CliError> {
    let activation: OracleStakeActivation = decode_sdk_state(
        data,
        OracleStakeActivation::LEN,
        "The wallet's queued staking record has an invalid account layout.",
    )?;
    if !activation.is_initialized
        || activation.bump != expected_bump
        || activation.account_discriminator != OracleStakeActivation::ACCOUNT_DISCRIMINATOR
        || activation.account_version != OracleStakeActivation::ACCOUNT_VERSION
        || activation.owner != expected_owner
    {
        return Err(CliError::new(
            "The wallet's queued staking record has an invalid account layout.",
        ));
    }
    if (activation.queued_amba == 0) != (activation.activate_after_ts == 0) {
        return Err(CliError::new(
            "The wallet's queued AMBA amount and activation time do not match.",
        ));
    }
    Ok(activation)
}

fn fetch_accounts(
    config: &OnchainConfig,
    addresses: &[Pubkey],
) -> Result<Vec<Option<RawAccount>>, CliError> {
    let response = rpc_call(
        config,
        "getMultipleAccounts",
        json!([
            addresses.iter().map(ToString::to_string).collect::<Vec<_>>(),
            {"encoding": "base64", "commitment": "confirmed"}
        ]),
    )?;
    let values = response
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("Staking balances are unavailable from this connection."))?;
    if values.len() != addresses.len() {
        return Err(CliError::new(
            "Staking balances are incomplete from this connection.",
        ));
    }
    values
        .iter()
        .map(|value| {
            if value.is_null() {
                return Ok(None);
            }
            let owner = string_at_key(value, &["owner"])
                .ok_or_else(|| CliError::new("A staking account is missing its owner."))?;
            let owner = parse_pubkey("staking account owner", &owner)?;
            let encoded = value
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
                .ok_or_else(|| CliError::new("A staking account has invalid data."))?;
            let data = BASE64_STANDARD
                .decode(encoded)
                .map_err(|_| CliError::new("A staking account has invalid data."))?;
            Ok(Some(RawAccount { owner, data }))
        })
        .collect()
}

fn fetch_token_holdings(
    config: &OnchainConfig,
    owner: Pubkey,
    mint: Pubkey,
) -> Result<Vec<TokenHolding>, CliError> {
    let response = rpc_call(
        config,
        "getTokenAccountsByOwner",
        json!([
            owner.to_string(),
            {"mint": mint.to_string()},
            {"encoding": "jsonParsed", "commitment": "confirmed"}
        ]),
    )?;
    let values = response
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("The sAMBA balance is unavailable right now."))?;
    let mut holdings = Vec::with_capacity(values.len());
    for value in values {
        holdings.push(decode_token_holding(value, owner, mint)?);
    }
    holdings.sort_by_key(|holding| holding.address.to_bytes());
    Ok(holdings)
}

fn decode_token_holding(
    value: &Value,
    expected_owner: Pubkey,
    expected_mint: Pubkey,
) -> Result<TokenHolding, CliError> {
    let address = value
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("An sAMBA account is missing its address."))?;
    let account_owner = value
        .pointer("/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("An sAMBA account is missing its token program."))?;
    let parsed_owner = value
        .pointer("/account/data/parsed/info/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("An sAMBA account is missing its wallet owner."))?;
    let parsed_mint = value
        .pointer("/account/data/parsed/info/mint")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::new("An sAMBA account is missing its mint."))?;
    let amount = value
        .pointer("/account/data/parsed/info/tokenAmount/amount")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| CliError::new("An sAMBA account has an invalid balance."))?;
    if account_owner != spl_token::id().to_string()
        || parsed_owner != expected_owner.to_string()
        || parsed_mint != expected_mint.to_string()
    {
        return Err(CliError::new(
            "An sAMBA account does not match the attached wallet and verified mint.",
        ));
    }
    Ok(TokenHolding {
        address: parse_pubkey("sAMBA account", address)?,
        amount,
    })
}

fn append_consolidation_instructions(
    context: &StakingContext,
    amount: u64,
    instructions: &mut Vec<Instruction>,
) -> Result<(), CliError> {
    let ata_amount = context
        .holdings
        .iter()
        .find(|holding| holding.address == context.owner_samba_ata)
        .map(|holding| holding.amount)
        .unwrap_or(0);
    let mut needed = amount.saturating_sub(ata_amount);
    let mut transfer_count = 0_usize;
    for holding in &context.holdings {
        if needed == 0 {
            break;
        }
        if holding.address == context.owner_samba_ata || holding.amount == 0 {
            continue;
        }
        if transfer_count >= MAX_CONSOLIDATION_TRANSFERS {
            return Err(CliError::new(
                "sAMBA is spread across too many token accounts. Consolidate it into the wallet's sAMBA account and try again.",
            ));
        }
        let moved = holding.amount.min(needed);
        instructions.push(
            spl_token::instruction::transfer_checked(
                &spl_token::id(),
                &holding.address,
                &context.samba_mint,
                &context.owner_samba_ata,
                &context.owner,
                &[],
                moved,
                context.decimals,
            )
            .map_err(|_| CliError::new("Could not prepare the sAMBA transfer for unstaking."))?,
        );
        needed -= moved;
        transfer_count += 1;
    }
    if needed != 0 {
        return Err(CliError::new(
            "The wallet does not have enough sAMBA in usable token accounts.",
        ));
    }
    Ok(())
}

fn sign_submit_exact(
    _config: &OnchainConfig,
    _signer: &dyn Signer,
    _owner: Pubkey,
    _amba_mint: Pubkey,
    _instructions: Vec<Instruction>,
) -> Result<String, CliError> {
    crate::current_release::require_current_write_release()?;
    Err(CliError::new(
        "Current staking transaction admission is not_wired. Nothing was submitted.",
    ))
}

// Retained for exact decoder regression coverage while the static release gate
// prevents the corresponding prepare path from running.
#[allow(dead_code)]
fn validate_exact_staking_instructions(
    owner: Pubkey,
    amba_mint: Pubkey,
    instructions: &[Instruction],
) -> Result<(), CliError> {
    let spread = instructions
        .last()
        .ok_or_else(|| CliError::new("The staking transaction is empty."))?;
    if spread.program_id != ameba_sdk::ID {
        return Err(CliError::new(
            "The staking transaction does not match the requested action.",
        ));
    }
    let program_id = ameba_sdk::ID;
    let (vault_config, _) = derive_current_pda(&program_id, &[VAULT_PDA_SEED]);
    let (major_config, _) = derive_current_pda(&program_id, &[ORACLE_MAJOR_TOKEN_CONFIG_PDA_SEED]);
    let (ledger, _) = derive_current_pda(
        &program_id,
        &[ORACLE_PLAYER_LEDGER_PDA_SEED, owner.as_ref()],
    );
    let (pool, _) = derive_current_pda(&program_id, &[ORACLE_STAKING_POOL_PDA_SEED]);
    let (mint, _) = derive_current_pda(&program_id, &[ORACLE_SAMBA_MINT_PDA_SEED]);
    let (activation, _) = derive_current_pda(
        &program_id,
        &[ORACLE_STAKE_ACTIVATION_PDA_SEED, owner.as_ref()],
    );
    let (reward_funnel, _) = derive_current_pda(&program_id, &[ORACLE_REWARD_FUNNEL_PDA_SEED]);
    let reward_funnel_token =
        spl_associated_token_account::get_associated_token_address(&reward_funnel, &amba_mint);
    let (request, _) = derive_current_pda(
        &program_id,
        &[ORACLE_UNSTAKE_REQUEST_PDA_SEED, owner.as_ref()],
    );
    let ata = spl_associated_token_account::get_associated_token_address(&owner, &mint);
    let tag =
        spread.data.first().copied().ok_or_else(|| {
            CliError::new("The staking transaction has unexpected instruction data.")
        })?;
    let (expected_data_len, expected_accounts, setup_kind) = match tag {
        QUEUE_STAKE_AMBA_FOR_SAMBA_TAG => (
            9,
            vec![
                AccountMeta::new(owner, true),
                AccountMeta::new_readonly(vault_config, false),
                AccountMeta::new_readonly(major_config, false),
                AccountMeta::new(ledger, false),
                AccountMeta::new_readonly(pool, false),
                AccountMeta::new(activation, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            "none",
        ),
        ACTIVATE_QUEUED_STAKE_AMBA_FOR_SAMBA_TAG => (
            9,
            vec![
                AccountMeta::new_readonly(owner, true),
                AccountMeta::new_readonly(vault_config, false),
                AccountMeta::new_readonly(major_config, false),
                AccountMeta::new(pool, false),
                AccountMeta::new(activation, false),
                AccountMeta::new_readonly(reward_funnel, false),
                AccountMeta::new_readonly(reward_funnel_token, false),
                AccountMeta::new(mint, false),
                AccountMeta::new(ata, false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            "ata",
        ),
        CANCEL_QUEUED_STAKE_AMBA_TAG => (
            1,
            vec![
                AccountMeta::new_readonly(owner, true),
                AccountMeta::new_readonly(vault_config, false),
                AccountMeta::new(ledger, false),
                AccountMeta::new(activation, false),
            ],
            "none",
        ),
        REQUEST_UNSTAKE_SAMBA_TAG => (
            17,
            vec![
                AccountMeta::new(owner, true),
                AccountMeta::new_readonly(vault_config, false),
                AccountMeta::new_readonly(major_config, false),
                AccountMeta::new(pool, false),
                AccountMeta::new(request, false),
                AccountMeta::new(mint, false),
                AccountMeta::new(ata, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            "unstake",
        ),
        COMPLETE_UNSTAKE_SAMBA_TAG => (
            1,
            vec![
                AccountMeta::new(owner, true),
                AccountMeta::new_readonly(vault_config, false),
                AccountMeta::new(ledger, false),
                AccountMeta::new(pool, false),
                AccountMeta::new(request, false),
                AccountMeta::new_readonly(system_program::ID, false),
            ],
            "none",
        ),
        _ => {
            return Err(CliError::new(
                "The staking transaction does not match the requested action.",
            ));
        }
    };
    if spread.data.len() != expected_data_len || spread.accounts != expected_accounts {
        return Err(CliError::new(
            "The staking transaction does not match the requested accounts and amounts.",
        ));
    }

    let setup = &instructions[..instructions.len() - 1];
    if setup_kind == "none" && !setup.is_empty() {
        return Err(CliError::new(
            "The staking transaction contains an unexpected setup instruction.",
        ));
    }
    if matches!(setup_kind, "ata" | "unstake") {
        let expected_ata =
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &owner,
                &owner,
                &mint,
                &spl_token::id(),
            );
        if setup.first() != Some(&expected_ata) {
            return Err(CliError::new(
                "The staking transaction does not contain the canonical sAMBA account setup.",
            ));
        }
    }
    if setup_kind == "ata" && setup.len() != 1 {
        return Err(CliError::new(
            "The staking transaction contains an unexpected setup instruction.",
        ));
    }
    if setup_kind == "unstake" {
        if setup.len() > MAX_CONSOLIDATION_TRANSFERS + 1 {
            return Err(CliError::new(
                "The unstaking transaction contains too many setup transfers.",
            ));
        }
        let mut seen_sources = std::collections::BTreeSet::new();
        for transfer in setup.iter().skip(1) {
            if transfer.program_id != spl_token::id()
                || transfer.accounts.len() != 4
                || transfer.accounts[0].is_signer
                || !transfer.accounts[0].is_writable
                || transfer.accounts[0].pubkey == ata
                || transfer.accounts[1] != AccountMeta::new_readonly(mint, false)
                || transfer.accounts[2] != AccountMeta::new(ata, false)
                || transfer.accounts[3] != AccountMeta::new_readonly(owner, true)
                || transfer.data.len() != 10
                || transfer.data[0] != 12
                || !seen_sources.insert(transfer.accounts[0].pubkey)
            {
                return Err(CliError::new(
                    "The unstaking transaction contains an unexpected sAMBA transfer.",
                ));
            }
        }
    }
    Ok(())
}

fn rpc_call(config: &OnchainConfig, method: &str, params: Value) -> Result<Value, CliError> {
    let rpc_url = onchain::resolve_rpc_url(config)?;
    let client = crate::backend::pinned_blocking_http_client_builder()
        .timeout(RPC_TIMEOUT)
        .build()
        .map_err(|_| CliError::new("Could not open the selected connection."))?;
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": format!("petri-staking:{method}"),
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|_| CliError::new("Staking is temporarily unavailable from this connection."))?;
    let status = response.status();
    let payload = response
        .json::<Value>()
        .map_err(|_| CliError::new("The selected connection returned an invalid response."))?;
    if !status.is_success() {
        return Err(CliError::new(format!(
            "Staking is temporarily unavailable (connection status {status})."
        )));
    }
    if value_at_key(&payload, &["error"]).is_some() {
        return Err(CliError::new(
            "The selected connection could not complete the staking request.",
        ));
    }
    Ok(payload)
}

fn parse_pubkey(label: &str, value: &str) -> Result<Pubkey, CliError> {
    Pubkey::from_str(value.trim())
        .map_err(|_| CliError::new(format!("{label} is not a valid Solana address.")))
}

fn parse_token_amount(value: &str, decimals: u8, label: &str) -> Result<u64, CliError> {
    parse_token_amount_inner(value, decimals, label, false)
}

fn parse_token_amount_inner(
    value: &str,
    decimals: u8,
    label: &str,
    allow_zero: bool,
) -> Result<u64, CliError> {
    let value = value.trim();
    if value.is_empty() || value.starts_with(['+', '-']) || value.contains(['e', 'E']) {
        return Err(CliError::new(format!(
            "{label} must be a positive decimal amount."
        )));
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || !fraction.chars().all(|character| character.is_ascii_digit())
    {
        return Err(CliError::new(format!(
            "{label} must be a positive decimal amount."
        )));
    }
    if fraction.len() > usize::from(decimals) {
        let extra = &fraction[usize::from(decimals)..];
        if extra.chars().any(|character| character != '0') {
            return Err(CliError::new(format!(
                "{label} supports at most {decimals} decimal places."
            )));
        }
    }
    let scale = 10_u128
        .checked_pow(u32::from(decimals))
        .ok_or_else(|| CliError::new(format!("{label} uses unsupported precision.")))?;
    let whole = whole
        .parse::<u128>()
        .map_err(|_| CliError::new(format!("{label} is too large.")))?;
    let kept_fraction = fraction
        .chars()
        .take(usize::from(decimals))
        .collect::<String>();
    let fraction_value = if kept_fraction.is_empty() {
        0
    } else {
        kept_fraction
            .parse::<u128>()
            .map_err(|_| CliError::new(format!("{label} is invalid.")))?
            * 10_u128.pow(u32::from(decimals) - kept_fraction.len() as u32)
    };
    let atoms = whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction_value))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| CliError::new(format!("{label} is too large.")))?;
    if atoms == 0 && !allow_zero {
        return Err(CliError::new(format!("{label} must be greater than zero.")));
    }
    Ok(atoms)
}

fn parse_minimum(
    value: Option<&str>,
    decimals: u8,
    quote: u64,
    label: &str,
) -> Result<u64, CliError> {
    let minimum = match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => parse_token_amount_inner(value, decimals, label, true)?,
        None => quote,
    };
    if minimum > quote {
        return Err(CliError::new(format!(
            "The current quote is {}, below the requested {} of {}.",
            format_token_amount(quote, decimals),
            label,
            format_token_amount(minimum, decimals)
        )));
    }
    Ok(minimum)
}

fn calculate_samba_for_amba(amba: u64, active_backing: u64, supply: u64) -> Result<u64, CliError> {
    if active_backing == 0 && supply == 0 {
        return Ok(amba);
    }
    if active_backing == 0 || supply == 0 {
        return Err(CliError::new(
            "The staking pool has an invalid AMBA/sAMBA exchange rate.",
        ));
    }
    let output = (u128::from(amba) * u128::from(supply)) / u128::from(active_backing);
    let output =
        u64::try_from(output).map_err(|_| CliError::new("The sAMBA quote is too large."))?;
    if output == 0 {
        return Err(CliError::new(
            "That AMBA amount is too small to receive sAMBA.",
        ));
    }
    Ok(output)
}

fn calculate_amba_for_samba(samba: u64, active_backing: u64, supply: u64) -> Result<u64, CliError> {
    if active_backing == 0 || supply == 0 || samba > supply {
        return Err(CliError::new(
            "The staking pool has an invalid AMBA/sAMBA exchange rate.",
        ));
    }
    let output = (u128::from(samba) * u128::from(active_backing)) / u128::from(supply);
    let output =
        u64::try_from(output).map_err(|_| CliError::new("The AMBA quote is too large."))?;
    if output == 0 {
        return Err(CliError::new(
            "That sAMBA amount is too small to redeem AMBA.",
        ));
    }
    Ok(output)
}

fn serialize_staking_instruction(instruction: VaultInstruction) -> Result<Vec<u8>, CliError> {
    encode_current_vault_instruction(&instruction)
        .map_err(|_| CliError::new("The staking instruction could not be encoded safely."))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, CliError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| CliError::new("A staking account is truncated."))?;
    Ok(i64::from_le_bytes(bytes))
}

fn format_token_amount(atoms: u64, decimals: u8) -> String {
    if decimals == 0 {
        return atoms.to_string();
    }
    let scale = 10_u64.pow(u32::from(decimals));
    let whole = atoms / scale;
    let fraction = atoms % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let mut fraction = format!("{fraction:0width$}", width = usize::from(decimals));
    while fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{whole}.{fraction}")
}

fn format_ratio(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        return "-".to_string();
    }
    let scaled = (u128::from(numerator) * 1_000_000_u128) / u128::from(denominator);
    let whole = scaled / 1_000_000;
    let fraction = scaled % 1_000_000;
    if fraction == 0 {
        whole.to_string()
    } else {
        let mut fraction = format!("{fraction:06}");
        while fraction.ends_with('0') {
            fraction.pop();
        }
        format!("{whole}.{fraction}")
    }
}

fn format_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}
