//! Names for the oracle program's custom errors (`OracleError`, codes 6000 and up),
//! bound to the pinned crate's variants so a renamed or removed variant fails to
//! compile.

use ameba_oracle::error::OracleError;

const ERRORS: &[(OracleError, &str)] = &[
    (
        OracleError::InvalidInstructionData,
        "InvalidInstructionData",
    ),
    (OracleError::InvalidAccountList, "InvalidAccountList"),
    (OracleError::InvalidTokenProgram, "InvalidTokenProgram"),
    (OracleError::InvalidSystemProgram, "InvalidSystemProgram"),
    (OracleError::InvalidPda, "InvalidPda"),
    (OracleError::AlreadyInitialized, "AlreadyInitialized"),
    (OracleError::Unauthorized, "Unauthorized"),
    (OracleError::AmountMustBePositive, "AmountMustBePositive"),
    (OracleError::ContractPaused, "ContractPaused"),
    (OracleError::InvalidMint, "InvalidMint"),
    (OracleError::InvalidTokenAccount, "InvalidTokenAccount"),
    (OracleError::InvalidConfigAccount, "InvalidConfigAccount"),
    (OracleError::AccountMismatch, "AccountMismatch"),
    (OracleError::InvalidPeriodAccount, "InvalidPeriodAccount"),
    (
        OracleError::InvalidUserCollateralAccount,
        "InvalidUserCollateralAccount",
    ),
    (
        OracleError::InsufficientAvailableCollateral,
        "InsufficientAvailableCollateral",
    ),
    (OracleError::ArithmeticOverflow, "ArithmeticOverflow"),
    (
        OracleError::InvalidCompressionWitness,
        "InvalidCompressionWitness",
    ),
    (
        OracleError::InvalidSettlementRecord,
        "InvalidSettlementRecord",
    ),
    (
        OracleError::SettlementRecordHashMismatch,
        "SettlementRecordHashMismatch",
    ),
    (
        OracleError::InvalidSettlementAccount,
        "InvalidSettlementAccount",
    ),
    (OracleError::PeriodNotExpired, "PeriodNotExpired"),
    (
        OracleError::InvalidInstructionsSysvar,
        "InvalidInstructionsSysvar",
    ),
    (
        OracleError::MissingSettlementOracleSignatures,
        "MissingSettlementOracleSignatures",
    ),
    (
        OracleError::InvalidSettlementOracleSignature,
        "InvalidSettlementOracleSignature",
    ),
    (
        OracleError::InvalidSettlementOracleSigner,
        "InvalidSettlementOracleSigner",
    ),
    (
        OracleError::DuplicateSettlementOracleSigner,
        "DuplicateSettlementOracleSigner",
    ),
    (
        OracleError::SettlementOraclePayloadMismatch,
        "SettlementOraclePayloadMismatch",
    ),
    (
        OracleError::InvalidOracleMonthAccount,
        "InvalidOracleMonthAccount",
    ),
    (
        OracleError::InvalidOracleSourceAccount,
        "InvalidOracleSourceAccount",
    ),
    (OracleError::InvalidOraclePhase, "InvalidOraclePhase"),
    (
        OracleError::OracleTimingWindowClosed,
        "OracleTimingWindowClosed",
    ),
    (OracleError::InvalidOracleState, "InvalidOracleState"),
    (
        OracleError::OracleOpeningAlreadySubmitted,
        "OracleOpeningAlreadySubmitted",
    ),
    (
        OracleError::InvalidOracleChallengeAccount,
        "InvalidOracleChallengeAccount",
    ),
    (
        OracleError::InvalidOracleUpdateAccount,
        "InvalidOracleUpdateAccount",
    ),
    (
        OracleError::InvalidOracleEmergencyDispute,
        "InvalidOracleEmergencyDispute",
    ),
    (
        OracleError::OracleOpeningClaimPending,
        "OracleOpeningClaimPending",
    ),
    (
        OracleError::OracleOpeningClaimNotFinalizable,
        "OracleOpeningClaimNotFinalizable",
    ),
    (
        OracleError::InvalidOracleOpeningEvidence,
        "InvalidOracleOpeningEvidence",
    ),
    (
        OracleError::InvalidOracleOpeningClaim,
        "InvalidOracleOpeningClaim",
    ),
    (
        OracleError::OracleEscrowAlreadySettled,
        "OracleEscrowAlreadySettled",
    ),
    (
        OracleError::OracleEscrowNotSettleable,
        "OracleEscrowNotSettleable",
    ),
    (
        OracleError::InvalidOracleWeightManifest,
        "InvalidOracleWeightManifest",
    ),
    (
        OracleError::OracleWeightManifestFinalized,
        "OracleWeightManifestFinalized",
    ),
    (
        OracleError::InvalidOracleWeightOrder,
        "InvalidOracleWeightOrder",
    ),
    (
        OracleError::OracleWeightSchemeUnverified,
        "OracleWeightSchemeUnverified",
    ),
    (
        OracleError::OracleWeightManifestHashMismatch,
        "OracleWeightManifestHashMismatch",
    ),
    (
        OracleError::OracleWeightManifestIncomplete,
        "OracleWeightManifestIncomplete",
    ),
    (
        OracleError::InvalidOracleWeightSource,
        "InvalidOracleWeightSource",
    ),
    (
        OracleError::SettlementRecordImmutable,
        "SettlementRecordImmutable",
    ),
    (
        OracleError::CollateralIdentityImmutable,
        "CollateralIdentityImmutable",
    ),
    (
        OracleError::InvalidSettlementSignerRegistry,
        "InvalidSettlementSignerRegistry",
    ),
    (
        OracleError::InvalidSettlementSignerSet,
        "InvalidSettlementSignerSet",
    ),
    (
        OracleError::InvalidSettlementSignerConfiguration,
        "InvalidSettlementSignerConfiguration",
    ),
    (
        OracleError::SettlementSignerRotationPending,
        "SettlementSignerRotationPending",
    ),
    (
        OracleError::SettlementSignerRotationNotReady,
        "SettlementSignerRotationNotReady",
    ),
    (
        OracleError::SettlementSignerGovernanceRequired,
        "SettlementSignerGovernanceRequired",
    ),
    (
        OracleError::SettlementSignerRecoveryRequiresPause,
        "SettlementSignerRecoveryRequiresPause",
    ),
    (
        OracleError::SettlementSignerSetVersionMismatch,
        "SettlementSignerSetVersionMismatch",
    ),
    (
        OracleError::SettlementSignerTimelockTooShort,
        "SettlementSignerTimelockTooShort",
    ),
    (
        OracleError::InvalidOracleActiveWeightManifest,
        "InvalidOracleActiveWeightManifest",
    ),
    (
        OracleError::OracleActiveWeightSchemeUnverified,
        "OracleActiveWeightSchemeUnverified",
    ),
    (
        OracleError::InvalidOracleUpdateRevealTiming,
        "InvalidOracleUpdateRevealTiming",
    ),
    (
        OracleError::OracleUpdateCommitmentMismatch,
        "OracleUpdateCommitmentMismatch",
    ),
    (
        OracleError::SettlementGracePeriodActive,
        "SettlementGracePeriodActive",
    ),
    (
        OracleError::InvalidOracleUsdcRewardVault,
        "InvalidOracleUsdcRewardVault",
    ),
    (
        OracleError::InvalidOracleUsdcRewardSchedule,
        "InvalidOracleUsdcRewardSchedule",
    ),
    (
        OracleError::InvalidOracleUsdcSkuPool,
        "InvalidOracleUsdcSkuPool",
    ),
    (
        OracleError::InvalidOracleUsdcSourceReward,
        "InvalidOracleUsdcSourceReward",
    ),
    (
        OracleError::InvalidOracleUsdcRewardRegistration,
        "InvalidOracleUsdcRewardRegistration",
    ),
    (
        OracleError::InvalidOracleUsdcRewardReceipt,
        "InvalidOracleUsdcRewardReceipt",
    ),
    (
        OracleError::OracleUsdcRewardScheduleUnderfunded,
        "OracleUsdcRewardScheduleUnderfunded",
    ),
    (
        OracleError::OracleUsdcRewardScheduleFrozen,
        "OracleUsdcRewardScheduleFrozen",
    ),
    (
        OracleError::OracleUsdcRewardEntitlementsNotFinalized,
        "OracleUsdcRewardEntitlementsNotFinalized",
    ),
    (OracleError::InvalidOracleUsdcBond, "InvalidOracleUsdcBond"),
    (
        OracleError::OracleUsdcRewardRegistrationIncomplete,
        "OracleUsdcRewardRegistrationIncomplete",
    ),
    (
        OracleError::InsufficientOracleUsdcRewardCustody,
        "InsufficientOracleUsdcRewardCustody",
    ),
    (
        OracleError::InvalidOracleMaturityLadder,
        "InvalidOracleMaturityLadder",
    ),
    (
        OracleError::InvalidOracleSkuCoverageManifest,
        "InvalidOracleSkuCoverageManifest",
    ),
    (
        OracleError::InvalidOracleSkuCoverageRecord,
        "InvalidOracleSkuCoverageRecord",
    ),
    (
        OracleError::InvalidOracleSkuMembershipProof,
        "InvalidOracleSkuMembershipProof",
    ),
    (
        OracleError::OracleSkuCoverageIncomplete,
        "OracleSkuCoverageIncomplete",
    ),
    (
        OracleError::OracleSkuCoverageAlreadyFinalized,
        "OracleSkuCoverageAlreadyFinalized",
    ),
    (
        OracleError::OracleSkuCoverageExtensionTooLate,
        "OracleSkuCoverageExtensionTooLate",
    ),
    (
        OracleError::OracleSkuCoverageInstructionRequired,
        "OracleSkuCoverageInstructionRequired",
    ),
    (
        OracleError::InvalidOracleProductSkuDraft,
        "InvalidOracleProductSkuDraft",
    ),
    (
        OracleError::CompressedStateTransportRequired,
        "CompressedStateTransportRequired",
    ),
    (OracleError::InvalidOracleMedian, "InvalidOracleMedian"),
    (
        OracleError::InvalidOracleObservation,
        "InvalidOracleObservation",
    ),
    (
        OracleError::InvalidOracleSecurityBudget,
        "InvalidOracleSecurityBudget",
    ),
    (
        OracleError::InvalidOracleBountySweep,
        "InvalidOracleBountySweep",
    ),
    (OracleError::MissingGovernanceTail, "MissingGovernanceTail"),
    (OracleError::InvalidGovernanceTail, "InvalidGovernanceTail"),
    (OracleError::MissingGovernanceGate, "MissingGovernanceGate"),
    (
        OracleError::DuplicateGovernanceGate,
        "DuplicateGovernanceGate",
    ),
    (
        OracleError::InvalidGovernanceGatePrivileges,
        "InvalidGovernanceGatePrivileges",
    ),
    (
        OracleError::InvalidGovernanceGateOwner,
        "InvalidGovernanceGateOwner",
    ),
    (
        OracleError::InvalidGovernanceGatePda,
        "InvalidGovernanceGatePda",
    ),
    (
        OracleError::InvalidGovernanceGateData,
        "InvalidGovernanceGateData",
    ),
    (OracleError::GovernanceGateFrozen, "GovernanceGateFrozen"),
    (
        OracleError::GovernanceGateEpochMismatch,
        "GovernanceGateEpochMismatch",
    ),
    (
        OracleError::GovernanceBridgeIdentityUnavailable,
        "GovernanceBridgeIdentityUnavailable",
    ),
    (
        OracleError::NoOracleBountyClaimable,
        "NoOracleBountyClaimable",
    ),
    (OracleError::InstructionTooLarge, "InstructionTooLarge"),
    (OracleError::AddressOccupied, "AddressOccupied"),
    (
        OracleError::InvalidUpgradeAuthority,
        "InvalidUpgradeAuthority",
    ),
    (OracleError::InvalidPeriodSchedule, "InvalidPeriodSchedule"),
    (
        OracleError::InvalidPeriodTransition,
        "InvalidPeriodTransition",
    ),
    (OracleError::PeriodNotFailable, "PeriodNotFailable"),
];

/// `Name: message (oracle error 6xxx)` for an oracle custom error code.
pub fn name(code: u32) -> Option<String> {
    ERRORS
        .iter()
        .find(|(error, _)| *error as u32 == code)
        .map(|(error, name)| format!("{name}: {error} (oracle error {code})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_resolve_to_names_and_messages() {
        assert_eq!(
            name(6000).as_deref(),
            Some("InvalidInstructionData: invalid instruction data (oracle error 6000)")
        );
        assert!(name(6015).unwrap().starts_with("InvalidPeriodAccount"));
        assert!(name(6005).unwrap().starts_with("AlreadyInitialized"));
        assert_eq!(name(5999), None);
        let mut codes: Vec<u32> = ERRORS.iter().map(|(e, _)| *e as u32).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ERRORS.len(), "every code appears once");
    }
}
