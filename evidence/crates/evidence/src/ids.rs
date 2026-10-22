//! Three identities that must not be collapsed into one hash.
//!
//! 1. **Observation** (`observation_id`): which economic observation this
//!    is. It comes from the authenticated record: server, path, the
//!    manifest's selector, the record's own id and revision when it prints
//!    them, the value with its currency, unit and tier, and the source's own
//!    date. Never from proof bytes, the session time, the wallet or the
//!    account: twenty proofs of one published quote, from twenty sessions
//!    or twenty subscribers, are one observation. A changed value or a new
//!    revision is a new one.
//! 2. **Submission** (`submission_id`): an observation inside one accounting
//!    scope (one candidate slot, one source's update lane). Credit is given
//!    at most once per submission id; the same proof can still be verified,
//!    retried, audited and reused in another scope.
//! 3. **Private account**: not implemented. Hashing an account id with a
//!    public salt lets anyone test guesses; a contributor-chosen salt lets
//!    the contributor mint new identities. A sound scoped nullifier needs a
//!    proof-bound construction this backend does not offer, so no account
//!    identity is claimed (see the evidence README).

use crate::hash::{hex32, tagged};

pub const OBSERVATION_DOMAIN: &str = "amoeba-evidence-observation-v1";
pub const SUBMISSION_DOMAIN: &str = "amoeba-evidence-submission-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationKey<'a> {
    pub server_name: &'a str,
    pub path: &'a str,
    pub selector_digest: [u8; 32],
    pub record_id: &'a str,
    pub revision: &'a str,
    pub value_atoms: u64,
    pub currency: &'a str,
    pub unit: &'a str,
    pub quantity_tier: &'a str,
    pub source_observed_at: &'a str,
}

pub fn observation_id(k: &ObservationKey) -> [u8; 32] {
    tagged(
        OBSERVATION_DOMAIN,
        &[
            k.server_name.as_bytes(),
            k.path.as_bytes(),
            &k.selector_digest,
            k.record_id.as_bytes(),
            k.revision.as_bytes(),
            &k.value_atoms.to_le_bytes(),
            k.currency.as_bytes(),
            k.unit.as_bytes(),
            k.quantity_tier.as_bytes(),
            k.source_observed_at.as_bytes(),
        ],
    )
}

/// An accounting scope, written as text so every component derives the
/// same id: `candidate:<market>:<expiry>:<bucket>` or
/// `update:<oracle program>:<month>:<source id hex>`.
pub fn submission_id(scope: &str, observation: &[u8; 32]) -> [u8; 32] {
    tagged(SUBMISSION_DOMAIN, &[scope.as_bytes(), observation])
}

pub fn candidate_scope(market: &str, expiry: &str, bucket: &str) -> String {
    format!("candidate:{market}:{expiry}:{bucket}")
}

pub fn update_scope(oracle_program: &str, month: &str, source_id: &[u8; 32]) -> String {
    format!("update:{oracle_program}:{month}:{}", hex32(source_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: u64, revision: &'static str) -> ObservationKey<'static> {
        ObservationKey {
            server_name: "portal.fixture.test",
            path: "/api/prices",
            selector_digest: [7; 32],
            record_id: "rec-1",
            revision,
            value_atoms: value,
            currency: "USD",
            unit: "module",
            quantity_tier: "1",
            source_observed_at: "2026-10-19",
        }
    }

    #[test]
    fn same_record_same_observation() {
        assert_eq!(observation_id(&key(1, "3")), observation_id(&key(1, "3")));
        assert_ne!(observation_id(&key(1, "3")), observation_id(&key(2, "3")));
        assert_ne!(observation_id(&key(1, "3")), observation_id(&key(1, "4")));
    }

    #[test]
    fn scopes_separate_credit() {
        let o = observation_id(&key(1, "3"));
        let a = submission_id(&candidate_scope("RAMX", "2026-12", "b1"), &o);
        let b = submission_id(&candidate_scope("RAMX", "2026-12", "b2"), &o);
        assert_ne!(a, b);
        assert_eq!(
            a,
            submission_id(&candidate_scope("RAMX", "2026-12", "b1"), &o)
        );
    }
}
