//! Canonical client-side projection of the on-chain monthly oracle lifecycle.
//!
//! The oracle month account owns the phase and effective timestamps. This
//! module only splits the coarse on-chain `Scramble` phase into its fixed
//! placement/challenge/freeze windows and validates that explicit phases agree
//! with those anchors. It never derives a lifecycle from time-to-expiry.

const DAY_SECONDS: i64 = 86_400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OracleLifecyclePhase {
    Unavailable,
    Upcoming,
    SourceSubmission,
    Placement,
    KillChallenge,
    ResolutionFreeze,
    Opening,
    Game,
    Settlement,
}

impl OracleLifecyclePhase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Lifecycle unavailable",
            Self::Upcoming => "Upcoming",
            Self::SourceSubmission => "Source Submission",
            Self::Placement => "Placement",
            Self::KillChallenge => "Kill Challenge",
            Self::ResolutionFreeze => "Resolution Freeze",
            Self::Opening => "Opening",
            Self::Game => "Game",
            Self::Settlement => "Settlement",
        }
    }
}

pub(crate) fn normalize_phase_label(phase: &str) -> String {
    phase.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

pub(crate) fn classify_oracle_lifecycle(
    raw_phase: &str,
    schedule_version: Option<u64>,
    scramble_start_ts: Option<i64>,
    listing_ts: Option<i64>,
    expiry_ts: Option<i64>,
    now_ts: i64,
) -> OracleLifecyclePhase {
    if schedule_version != Some(2) {
        return OracleLifecyclePhase::Unavailable;
    }
    let (Some(scramble_start_ts), Some(listing_ts), Some(expiry_ts)) =
        (scramble_start_ts, listing_ts, expiry_ts)
    else {
        return OracleLifecyclePhase::Unavailable;
    };
    let Some(placement_end) = scramble_start_ts.checked_add(4 * DAY_SECONDS) else {
        return OracleLifecyclePhase::Unavailable;
    };
    let Some(challenge_end) = scramble_start_ts.checked_add(6 * DAY_SECONDS) else {
        return OracleLifecyclePhase::Unavailable;
    };
    let Some(freeze_end) = scramble_start_ts.checked_add(7 * DAY_SECONDS) else {
        return OracleLifecyclePhase::Unavailable;
    };
    let Some(expected_listing_ts) = scramble_start_ts.checked_add(8 * DAY_SECONDS) else {
        return OracleLifecyclePhase::Unavailable;
    };
    if listing_ts != expected_listing_ts || listing_ts >= expiry_ts {
        return OracleLifecyclePhase::Unavailable;
    }

    let scheduled = if now_ts < scramble_start_ts {
        OracleLifecyclePhase::Upcoming
    } else if now_ts < placement_end {
        OracleLifecyclePhase::Placement
    } else if now_ts < challenge_end {
        OracleLifecyclePhase::KillChallenge
    } else if now_ts < freeze_end {
        OracleLifecyclePhase::ResolutionFreeze
    } else if now_ts < listing_ts {
        OracleLifecyclePhase::Opening
    } else if now_ts < expiry_ts {
        OracleLifecyclePhase::Game
    } else {
        OracleLifecyclePhase::Settlement
    };

    match normalize_phase_label(raw_phase).as_str() {
        "upcoming" | "pre_scramble" if scheduled == OracleLifecyclePhase::Upcoming => scheduled,
        "6" | "source_submission" | "sourcesubmission" if schedule_version == Some(2) => {
            // SourceSubmission is explicit on-chain authority. Coverage completion,
            // not either planned anchor, ends it, so it may remain active both
            // before planned Scramble and past planned listing.
            OracleLifecyclePhase::SourceSubmission
        }
        "1" | "scramble"
            if matches!(
                scheduled,
                OracleLifecyclePhase::Upcoming
                    | OracleLifecyclePhase::Placement
                    | OracleLifecyclePhase::KillChallenge
                    | OracleLifecyclePhase::ResolutionFreeze
            ) =>
        {
            scheduled
        }
        "placement" | "source_selection" if scheduled == OracleLifecyclePhase::Placement => {
            scheduled
        }
        "kill_challenge" | "source_challenge"
            if scheduled == OracleLifecyclePhase::KillChallenge =>
        {
            scheduled
        }
        "resolution_freeze" | "source_freeze"
            if scheduled == OracleLifecyclePhase::ResolutionFreeze =>
        {
            scheduled
        }
        "5" | "opening" | "opening_print" if scheduled == OracleLifecyclePhase::Opening => {
            scheduled
        }
        "2" | "game" | "game_mode" | "live_updates" if scheduled == OracleLifecyclePhase::Game => {
            scheduled
        }
        "3" | "settled" | "4" | "closed" if scheduled == OracleLifecyclePhase::Settlement => {
            scheduled
        }
        _ => OracleLifecyclePhase::Unavailable,
    }
}
