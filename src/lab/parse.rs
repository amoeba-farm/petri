//! Strict adapters from backend JSON into Lab presentation state.

use super::*;

fn integer_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| {
            value.as_str().and_then(|value| {
                value.trim().parse().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc3339(value.trim())
                        .ok()
                        .map(|timestamp| timestamp.timestamp())
                })
            })
        })
}

fn oracle_schedule_value<'a>(month: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    value_at_key(month, keys).or_else(|| {
        value_at_key(
            month,
            &[
                "schedule",
                "lifecycleSchedule",
                "lifecycle_schedule",
                "oracleSchedule",
                "oracle_schedule",
            ],
        )
        .and_then(|schedule| value_at_key(schedule, keys))
    })
}

pub(super) fn spread_oracle_live_state_from_payload(
    market_id: &str,
    payload: &Value,
) -> SpreadOracleLiveState {
    let data = value_at_key(payload, &["data"]).unwrap_or(payload);
    let oracle_month =
        value_at_key(data, &["oracleMonth", "oracle_month"]).filter(|value| !value.is_null());
    let mut issues = string_array(data, &["issues"]);
    let month_label =
        oracle_month.and_then(|month| string_at_key(month, &["oracleMonth", "oracle_month"]));
    let expiry_id = oracle_month
        .and_then(|month| string_at_key(month, &["expiryId", "expiry_id"]))
        .unwrap_or_default();
    let phase = oracle_month
        .and_then(|month| string_at_key(month, &["phase"]))
        .and_then(|phase| oracle_phase_from_spread_label(&phase));
    let backend_scramble_start_ts = oracle_month
        .and_then(|month| {
            oracle_schedule_value(
                month,
                &[
                    "scrambleStartTs",
                    "scramble_start_ts",
                    "scrambleStartUnix",
                    "scramble_start_unix",
                ],
            )
        })
        .and_then(integer_i64);
    let backend_listing_ts = oracle_month
        .and_then(|month| {
            oracle_schedule_value(
                month,
                &["listingTs", "listing_ts", "listingUnix", "listing_unix"],
            )
        })
        .and_then(integer_i64);
    let backend_expiry_ts = oracle_month
        .and_then(|month| {
            oracle_schedule_value(
                month,
                &[
                    "expiryTs",
                    "expiry_ts",
                    "settlementTs",
                    "settlement_ts",
                    "settlementUtc",
                    "settlement_utc",
                ],
            )
        })
        .and_then(integer_i64);
    let (scramble_start_ts, listing_ts, expiry_ts) = (
        backend_scramble_start_ts,
        backend_listing_ts,
        backend_expiry_ts,
    );
    let schedule_version = oracle_month
        .and_then(|month| oracle_schedule_value(month, &["scheduleVersion", "schedule_version"]))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        });
    if matches!(
        phase,
        Some(
            OraclePhase::Upcoming
                | OraclePhase::SourceSubmission
                | OraclePhase::Scramble
                | OraclePhase::Placement
                | OraclePhase::KillChallenge
                | OraclePhase::ResolutionFreeze
                | OraclePhase::OpeningPrint
                | OraclePhase::GameMode
                | OraclePhase::MonthClose
        )
    ) && (scramble_start_ts.is_none()
        || listing_ts.is_none()
        || expiry_ts.is_none()
        || schedule_version != Some(2))
    {
        issues.push(
            "Lifecycle phase is unavailable because a supported on-chain scheduleVersion and valid scrambleStartTs, listingTs, and expiryTs anchors were not all verified."
                .to_string(),
        );
    }
    let pending_resolution_count = oracle_month
        .and_then(|month| {
            value_at_key(
                month,
                &["pendingResolutionCount", "pending_resolution_count"],
            )
        })
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<u64>().ok()))
        });
    let weight_scheme_version = oracle_month
        .and_then(|month| {
            value_at_key(
                month,
                &[
                    "weightSchemeVersion",
                    "weight_scheme_version",
                    "weightVersion",
                    "weight_version",
                ],
            )
        })
        .and_then(number_from_value)
        .and_then(|value| {
            (value.is_finite() && value >= 0.0 && value <= f64::from(u8::MAX))
                .then_some(value.round() as u8)
        })
        .filter(|version| matches!(version, 1 | 255));
    let weight_scheme = oracle_month
        .and_then(|month| string_at_key(month, &["weightScheme", "weight_scheme"]))
        .unwrap_or_else(|| match weight_scheme_version {
            Some(1) => "verified_v1".to_string(),
            Some(255) => "building".to_string(),
            _ => "unknown".to_string(),
        });
    let effective_weight_total_bps = oracle_month
        .and_then(|month| {
            value_at_key(
                month,
                &["effectiveWeightTotalBps", "effective_weight_total_bps"],
            )
        })
        .and_then(number_from_value)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let weight_manifest_hash_hex = oracle_month.and_then(|month| {
        string_at_key(
            month,
            &[
                "weightManifestHashHex",
                "weight_manifest_hash_hex",
                "recipeWeightManifestHashHex",
                "recipe_weight_manifest_hash_hex",
            ],
        )
    });
    let weight_verified = oracle_month
        .and_then(|month| value_at_key(month, &["weightVerified", "weight_verified"]))
        .and_then(Value::as_bool)
        .unwrap_or(weight_scheme_version == Some(1));
    let weight_verification_status = oracle_month
        .and_then(|month| {
            string_at_key(
                month,
                &["weightVerificationStatus", "weight_verification_status"],
            )
        })
        .unwrap_or_else(|| {
            if weight_verified {
                "verified".to_string()
            } else {
                "unverified".to_string()
            }
        });
    let active_weight_scheme_version = oracle_month
        .and_then(|month| {
            value_at_key(
                month,
                &[
                    "activeWeightSchemeVersion",
                    "active_weight_scheme_version",
                    "activeWeightVersion",
                    "active_weight_version",
                ],
            )
        })
        .and_then(number_from_value)
        .and_then(|value| {
            (value.is_finite() && value >= 0.0 && value <= f64::from(u8::MAX))
                .then_some(value.round() as u8)
        })
        .filter(|version| matches!(version, 1 | 255));
    let active_weight_manifest_hash_hex = oracle_month.and_then(|month| {
        string_at_key(
            month,
            &[
                "activeWeightManifestHashHex",
                "active_weight_manifest_hash_hex",
            ],
        )
    });
    let active_weight_verified = oracle_month
        .and_then(|month| value_at_key(month, &["activeWeightVerified", "active_weight_verified"]))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let active_weight_verification_status = oracle_month
        .and_then(|month| {
            string_at_key(
                month,
                &[
                    "activeWeightVerificationStatus",
                    "active_weight_verification_status",
                ],
            )
        })
        .unwrap_or_else(|| {
            if active_weight_verified {
                "finalized".to_string()
            } else if active_weight_scheme_version == Some(255) {
                "manifest build in progress".to_string()
            } else {
                "post-opening active manifest is missing".to_string()
            }
        });
    let sources = oracle_month
        .and_then(|month| array_at_key(month, &["sources"]))
        .cloned()
        .unwrap_or_default();
    let emergencies = oracle_month
        .and_then(|month| array_at_key(month, &["emergencyDisputes", "emergency_disputes"]))
        .cloned()
        .unwrap_or_default();
    let active_emergencies = emergencies
        .iter()
        .filter_map(spread_oracle_emergency_from_value)
        .filter(|emergency| spread_emergency_is_active(&emergency.status))
        .collect::<Vec<_>>();
    let escrows = oracle_month
        .and_then(|month| {
            array_at_key(
                month,
                &[
                    "escrows",
                    "oracleEscrows",
                    "oracle_escrows",
                    "stakeLocks",
                    "stake_locks",
                ],
            )
        })
        .or_else(|| {
            array_at_key(
                data,
                &[
                    "escrows",
                    "oracleEscrows",
                    "oracle_escrows",
                    "stakeLocks",
                    "stake_locks",
                ],
            )
        })
        .map(|items| {
            items
                .iter()
                .filter_map(spread_oracle_escrow_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let observations = sources
        .iter()
        .filter_map(|source| {
            spread_oracle_observation_from_value(
                source,
                &active_emergencies,
                weight_scheme_version,
                weight_verified,
                active_weight_scheme_version,
                active_weight_verified,
            )
        })
        .collect::<Vec<_>>();

    SpreadOracleLiveState {
        market_id: market_id.to_ascii_lowercase(),
        expiry_id,
        oracle_month: month_label,
        phase,
        scramble_start_ts,
        listing_ts,
        expiry_ts,
        schedule_version,
        pending_resolution_count,
        weight_scheme_version,
        weight_scheme,
        effective_weight_total_bps,
        weight_manifest_hash_hex,
        weight_verified,
        weight_verification_status,
        active_weight_scheme_version,
        active_weight_manifest_hash_hex,
        active_weight_verified,
        active_weight_verification_status,
        observations,
        emergencies: active_emergencies,
        escrows,
        issues,
    }
}

pub(super) fn spread_oracle_live_state_for_expiry_from_payload(
    market_id: &str,
    expiry_id: &str,
    payload: &Value,
) -> Result<SpreadOracleLiveState, String> {
    let data = value_at_key(payload, &["oracle"]).unwrap_or(payload);
    let month = array_at_key(data, &["oracleMonths", "oracle_months"])
        .and_then(|months| {
            months.iter().find(|month| {
                string_at_key(month, &["expiryId", "expiry_id"])
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(expiry_id))
            })
        })
        .ok_or_else(|| {
            format!(
                "Oracle evidence for {} {} is not available.",
                market_id.to_uppercase(),
                expiry_id.to_uppercase()
            )
        })?;
    let scoped_payload = serde_json::json!({
        "data": {
            "oracleMonth": month,
            "issues": string_array(month, &["issues"]),
        }
    });
    let mut state = spread_oracle_live_state_from_payload(market_id, &scoped_payload);
    state.expiry_id = expiry_id.to_ascii_uppercase();
    Ok(state)
}

pub(super) fn spread_oracle_reward_state_from_payload(
    market_id: &str,
    expiry_id: &str,
    owner_pubkey: &str,
    payload: &Value,
) -> SpreadOracleRewardState {
    let empty_state = || SpreadOracleRewardState {
        market_id: String::new(),
        expiry_id: String::new(),
        owner_pubkey: String::new(),
        claims: Vec::new(),
    };
    let Some(rewards) = value_at_key(payload, &["data"])
        .and_then(|data| value_at_key(data, &["state"]))
        .and_then(|state| value_at_key(state, &["rewards"]))
    else {
        return empty_state();
    };
    let Some(owner) = string_at_key(rewards, &["ownerPubkey"]) else {
        return empty_state();
    };
    let Some(response_market_id) = string_at_key(rewards, &["marketId"]) else {
        return empty_state();
    };
    let Some(response_expiry_id) = string_at_key(rewards, &["expiryId"]) else {
        return empty_state();
    };
    let response_matches_request = response_market_id.eq_ignore_ascii_case(market_id)
        && response_expiry_id.eq_ignore_ascii_case(expiry_id)
        && owner == owner_pubkey;
    let claims = response_matches_request
        .then(|| {
            array_at_key(rewards, &["claims"])
                .map(|items| {
                    items
                        .iter()
                        .filter(|value| {
                            string_at_key(value, &["marketId"])
                                .is_none_or(|value| value.eq_ignore_ascii_case(market_id))
                                && string_at_key(value, &["expiryId"])
                                    .is_none_or(|value| value.eq_ignore_ascii_case(expiry_id))
                                && string_at_key(value, &["ownerPubkey"])
                                    .is_none_or(|value| value == owner_pubkey)
                        })
                        .filter_map(spread_oracle_reward_claim_from_value)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    SpreadOracleRewardState {
        market_id: response_market_id,
        expiry_id: response_expiry_id,
        owner_pubkey: owner,
        claims,
    }
}

pub(super) fn spread_oracle_reward_claim_from_value(
    value: &Value,
) -> Option<SpreadOracleRewardClaim> {
    let kind = string_at_key(value, &["kind"])?;
    let subject_pda =
        string_at_key(value, &["rewardReceipt"]).or_else(|| string_at_key(value, &["claimPda"]))?;
    let amount_label = string_at_key(value, &["amountLabel"])?;
    Some(SpreadOracleRewardClaim {
        label: string_at_key(value, &["label"]).unwrap_or_else(|| kind.replace('_', " ")),
        kind,
        source_id_hex: string_at_key(value, &["sourceIdHex"])
            .map(|value| value.trim_start_matches("0x").to_ascii_lowercase()),
        claim_id_hex: string_at_key(value, &["claimIdHex"])
            .map(|value| value.trim_start_matches("0x").to_ascii_lowercase()),
        challenge_id_hex: string_at_key(value, &["challengeIdHex"])
            .map(|value| value.trim_start_matches("0x").to_ascii_lowercase()),
        claim_pda: string_at_key(value, &["claimPda"]),
        subject_pda,
        amount_label,
    })
}

pub(super) fn spread_oracle_escrow_from_value(value: &Value) -> Option<SpreadOracleEscrow> {
    let kind = string_at_key(value, &["kind", "escrowKind", "escrow_kind", "stakeKind"])?;
    let subject_pda = string_at_key(
        value,
        &[
            "subjectPda",
            "subject_pda",
            "escrowPda",
            "escrow_pda",
            "pda",
        ],
    )?;
    Some(SpreadOracleEscrow {
        kind: normalized_oracle_escrow_kind(&kind),
        subject_pda,
        owner_pubkey: string_at_key(
            value,
            &["ownerPubkey", "owner_pubkey", "owner", "participant"],
        )
        .unwrap_or_else(|| "-".to_string()),
        amount_label: string_at_key(value, &["amountLabel", "amount_label", "amount"])
            .unwrap_or_else(|| "-".to_string()),
        disposition: string_at_key(
            value,
            &["disposition", "escrowDisposition", "escrow_disposition"],
        )
        .unwrap_or_else(|| "unsettled".to_string())
        .to_ascii_lowercase(),
        terminal_outcome: string_at_key(
            value,
            &["terminalOutcome", "terminal_outcome", "outcome", "status"],
        )
        .unwrap_or_else(|| "pending".to_string())
        .to_ascii_lowercase(),
        settlement_eligible: value_at_key(
            value,
            &[
                "settlementEligible",
                "settlement_eligible",
                "terminalEligible",
                "terminal_eligible",
                "canSettle",
                "can_settle",
                "eligible",
            ],
        )
        .and_then(Value::as_bool)
        .unwrap_or(false),
    })
}

pub(super) fn normalized_oracle_escrow_kind(kind: &str) -> String {
    kind.trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_")
}

pub(super) fn spread_oracle_observation_from_value(
    value: &Value,
    active_emergencies: &[SpreadOracleEmergency],
    weight_scheme_version: Option<u8>,
    month_weight_verified: bool,
    active_weight_scheme_version: Option<u8>,
    month_active_weight_verified: bool,
) -> Option<SpreadOracleObservation> {
    let source = string_at_key(value, &["source"])?;
    let source_id_hex = string_at_key(value, &["sourceIdHex", "source_id_hex"])?;
    let status = string_at_key(value, &["status"]).unwrap_or_else(|| "-".to_string());
    let emergency = active_emergencies
        .iter()
        .find(|emergency| emergency.matches_source(&source_id_hex))
        .cloned();

    let opening_submitted = value_at_key(value, &["openingSubmitted", "opening_submitted"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let opening_claim = value_at_key(value, &["openingClaim", "opening_claim"]);
    let opening_status_label = string_at_key(
        value,
        &[
            "openingClaimStatus",
            "opening_claim_status",
            "openingStatus",
            "opening_status",
        ],
    )
    .or_else(|| opening_claim.and_then(|claim| string_at_key(claim, &["status"])))
    .unwrap_or_default();
    let opening_status =
        OpeningClaimViewStatus::from_label(&opening_status_label, opening_submitted, &status);
    let opening_challenge_deadline_slot = value_at_key(
        value,
        &[
            "openingChallengeDeadlineSlot",
            "opening_challenge_deadline_slot",
            "challengeDeadlineSlot",
            "challenge_deadline_slot",
        ],
    )
    .or_else(|| {
        opening_claim.and_then(|claim| {
            value_at_key(claim, &["challengeDeadlineSlot", "challenge_deadline_slot"])
        })
    })
    .and_then(number_from_value)
    .filter(|value| value.is_finite() && *value >= 0.0)
    .map(|value| value.round() as u64);
    let opening_finalizable = value_at_key(
        value,
        &["openingFinalizable", "opening_finalizable", "finalizable"],
    )
    .or_else(|| opening_claim.and_then(|claim| value_at_key(claim, &["finalizable"])))
    .and_then(Value::as_bool)
    .unwrap_or(false);
    let opening_archive_url = string_at_key(
        value,
        &[
            "openingArchiveUrl",
            "opening_archive_url",
            "archiveUrl",
            "archive_url",
        ],
    )
    .or_else(|| {
        opening_claim.and_then(|claim| string_at_key(claim, &["archiveUrl", "archive_url"]))
    });
    let frozen_weight_bps = value_at_key(value, &["frozenWeightBps", "frozen_weight_bps"])
        .and_then(number_from_value)
        .unwrap_or(0.0);
    let bucket_weight_bps = value_at_key(value, &["bucketWeightBps", "bucket_weight_bps"])
        .and_then(number_from_value)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let source_weight_verified = value_at_key(value, &["weightVerified", "weight_verified"])
        .and_then(Value::as_bool)
        .unwrap_or(month_weight_verified && weight_scheme_version == Some(1));
    let effective_weight_bps = value_at_key(
        value,
        &[
            "effectiveWeightBps",
            "effective_weight_bps",
            "effectiveGlobalWeightBps",
            "effective_global_weight_bps",
        ],
    )
    .and_then(number_from_value)
    .filter(|value| value.is_finite() && *value >= 0.0);
    let active_weight_bps = value_at_key(value, &["activeWeightBps", "active_weight_bps"])
        .and_then(number_from_value)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let active_effective_weight_bps = value_at_key(
        value,
        &[
            "activeEffectiveWeightBps",
            "active_effective_weight_bps",
            "activeEffectiveGlobalWeightBps",
            "active_effective_global_weight_bps",
        ],
    )
    .and_then(number_from_value)
    .filter(|value| value.is_finite() && *value >= 0.0);
    let source_active_weight_verified =
        value_at_key(value, &["activeWeightVerified", "active_weight_verified"])
            .and_then(Value::as_bool)
            .unwrap_or(
                month_active_weight_verified
                    && active_weight_scheme_version == Some(1)
                    && active_weight_bps.is_some()
                    && active_effective_weight_bps.is_some(),
            );

    Some(SpreadOracleObservation {
        source,
        source_id_hex,
        status,
        baseline_state: string_at_key(value, &["baselineState", "baseline_state"])
            .unwrap_or_else(|| "-".to_string()),
        current_state: string_at_key(value, &["currentState", "current_state"])
            .unwrap_or_else(|| "-".to_string()),
        support_stake_total: string_at_key(value, &["supportStakeTotal", "support_stake_total"])
            .unwrap_or_else(|| "-".to_string()),
        frozen_weight_bps,
        bucket_weight_bps,
        effective_weight_bps,
        weight_verified: source_weight_verified,
        active_weight_bps,
        active_effective_weight_bps,
        active_weight_verified: source_active_weight_verified,
        opening_status,
        opening_challenge_deadline_slot,
        opening_finalizable,
        opening_archive_url,
        emergency,
    })
}

pub(super) fn spread_oracle_emergency_from_value(value: &Value) -> Option<SpreadOracleEmergency> {
    Some(SpreadOracleEmergency {
        dispute_id_hex: string_at_key(value, &["disputeIdHex", "dispute_id_hex"])?,
        kind: string_at_key(value, &["kind"])
            .unwrap_or_else(|| "emergency".to_string())
            .to_ascii_lowercase(),
        status: string_at_key(value, &["status"])
            .unwrap_or_else(|| "open".to_string())
            .to_ascii_lowercase(),
        target_id_hex: string_at_key(value, &["targetIdHex", "target_id_hex"])
            .unwrap_or_default()
            .to_ascii_lowercase(),
        source_id_hex: string_at_key(value, &["sourceIdHex", "source_id_hex"])
            .map(|value| value.to_ascii_lowercase()),
        claim_id_hex: string_at_key(value, &["claimIdHex", "claim_id_hex"])
            .map(|value| value.to_ascii_lowercase()),
        challenge_id_hex: string_at_key(value, &["challengeIdHex", "challenge_id_hex"])
            .map(|value| value.to_ascii_lowercase()),
    })
}

pub(super) fn spread_emergency_is_active(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "open" | "committed" | "revealing"
    )
}
