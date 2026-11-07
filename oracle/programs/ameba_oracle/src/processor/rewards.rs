//! Terminal ordinary-USDC escrow settlement and finite cash reward registration and claims.
//!
//! Participant bonds settle back into canonical `UserCollateral` ledgers in the bond vault. A
//! failed month releases its reward reservation once every counted pre-listing escrow is settled.
//! Cash rewards are registered after settlement, frozen as entitlements, and claimed from the
//! reward vault into the recipient's ledger. Council ballots carry neither stake nor payout.

use super::usdc::{
    load_oracle_usdc_reward_schedule, load_oracle_usdc_reward_vault, load_oracle_usdc_sku_pool,
    load_oracle_usdc_source_reward,
};
use super::*;
use crate::{
    constants::{
        MAX_ORACLE_USDC_MERGE_DEPTH, ORACLE_USDC_REWARD_REGISTRATION_PDA_SEED,
        ORACLE_USDC_REWARD_REGISTRATION_VERSION_SEED,
    },
    instruction::SettleOracleEscrowParams,
    state::{
        derive_oracle_usdc_reward_registration_pda, OracleEscrowKind, OracleOpeningClaimStatus,
        OracleUsdcRewardKind, OracleUsdcRewardRegistration, OracleUsdcRewardSchedule,
        OracleUsdcRewardSchedulePhase, OracleUsdcRewardVault, OracleUsdcSourceReward,
    },
};
// The reward claims (174) read these too (`v1-months`).
#[cfg(feature = "v1-months")]
use super::usdc::{
    calculate_oracle_usdc_equal_share, calculate_oracle_usdc_source_reward_allocation,
    calculate_oracle_usdc_supporter_reward,
};
#[cfg(feature = "v1-months")]
use crate::{
    constants::{ORACLE_USDC_REWARD_RECEIPT_PDA_SEED, ORACLE_USDC_REWARD_VAULT_PDA_SEED},
    instruction::ClaimOracleUsdcRewardParams,
    state::{derive_oracle_usdc_reward_receipt_pda, OracleUsdcRewardReceipt, OracleUsdcSkuPool},
};

#[cfg(feature = "v1-months")]
mod claims;
mod escrow;
mod registration;

#[cfg(feature = "v1-months")]
pub(super) use claims::*;
pub(super) use escrow::*;
pub(super) use registration::*;

#[cfg(test)]
mod tests;
