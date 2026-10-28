//! Bonds and stakes in the oracle's vault, and how each comes back. Every
//! bond or stake is held in the oracle's vault while its stage runs (a
//! source's listing and support until it opens or is set aside, a challenge
//! until it is ruled, a print until it is accepted or ruled) and moves only
//! when someone settles it (tag 169, permissionless; 167 for an update
//! nobody revealed; the sponsorship's settle and reconcile for a sponsored
//! listing): into its owner's collateral, or to whoever won it. `petri oracle
//! stakes` lists a wallet's, with what each waits for or where it went;
//! `stakes settle --all` settles every one that pays the wallet now.

use std::collections::{HashMap, HashSet};

use ameba_oracle::{
    instruction::{self as ix, PrelistingEscrow, SponsoredAccounts},
    sponsorship::{SourceSponsorship, exposure_address},
    state::{
        OracleChallengeStatus, OracleClaimStatus, OracleEscrowDisposition, OracleEscrowKind,
        OracleOpeningClaimChallenge, OracleOpeningClaimStatus, OracleSourceChallenge,
        OracleSourceStatus, OracleUpdateChallenge, OracleUpdateClaimV2, derive_user_collateral_pda,
    },
};
use serde_json::{Value, json};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use super::{
    args::{EscrowKind, SendArgs},
    chain::{MonthRef, Oracle, SourceRef},
    challenges::{self, iso},
    prints::{self, ChainNow, Opening},
    send::{self, Write},
    writes::month_line,
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::Result,
    units::{short, usdc},
    view::{Paint, Table},
};

/// Where a bond or stake is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// In the vault while its stage runs.
    Locked,
    /// Decided: settling moves it now (to its owner, or to whoever won it).
    Settleable,
    /// Moved out of the vault.
    Settled,
    /// Its stage can never end (an opening print left open at listing):
    /// nothing can move it.
    Stuck,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Settleable => "settleable",
            Self::Settled => "settled",
            Self::Stuck => "stuck",
        }
    }
}

/// Whom an escrow pays once it settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    You,
    Proposer,
    Claimant,
    Challenger,
}

/// What a bond's or stake's stage says about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub status: Status,
    pub words: &'static str,
    /// What settling pays the owner, once it is decided.
    pub to_you: Option<u64>,
    /// Whom it pays, once decided.
    pub pays: Option<Party>,
    /// It settles with the challenge that decided it, not on its own.
    pub via_challenge: bool,
}

const fn locked(words: &'static str) -> Verdict {
    Verdict {
        status: Status::Locked,
        words,
        to_you: None,
        pays: None,
        via_challenge: false,
    }
}

const fn stuck(words: &'static str) -> Verdict {
    Verdict {
        status: Status::Stuck,
        words,
        to_you: None,
        pays: None,
        via_challenge: false,
    }
}

const fn ready(words: &'static str, to_you: u64, pays: Party) -> Verdict {
    Verdict {
        status: Status::Settleable,
        words,
        to_you: Some(to_you),
        pays: Some(pays),
        via_challenge: false,
    }
}

const fn done(words: &'static str, to_you: Option<u64>, pays: Party) -> Verdict {
    Verdict {
        status: Status::Settled,
        words,
        to_you,
        pays: Some(pays),
        via_challenge: false,
    }
}

impl Verdict {
    const fn through_challenge(mut self) -> Self {
        self.via_challenge = true;
        self
    }
}

const NEVER_RESOLVED: &str =
    "Stuck: the source's opening was not resolved before listing, so nothing can return it";

/// Whether a source's stage can still end: one waiting on an opening print
/// past listing (nothing accepts or rules on it then), or frozen without a
/// print past the month's expiry (81 sets one aside only until then), can't.
pub fn stuck_source(status: OracleSourceStatus, opening: Opening, now: u64) -> bool {
    match status {
        OracleSourceStatus::OpeningPending => !opening.is_open(now),
        OracleSourceStatus::Frozen => {
            opening.phase == ameba_oracle::state::OraclePhase::Opening && now >= opening.expiry
        }
        _ => false,
    }
}

/// A listing bond (its proposer's): back once the source opens or is set
/// aside (or timed out unsupported); a challenger who has the source
/// rejected or merged takes it, with the challenge's settle.
pub fn listing_bond(
    status: OracleSourceStatus,
    locked_now: u64,
    bond: u64,
    stuck: bool,
) -> Verdict {
    use OracleSourceStatus as S;
    match status {
        S::Rejected | S::Merged if locked_now == 0 => done(
            "Lost: it went to the challenger who had the source rejected",
            Some(0),
            Party::Challenger,
        ),
        _ if locked_now == 0 => done("Back in your collateral", Some(bond), Party::You),
        _ if stuck => stuck_verdict(),
        S::Active | S::Inactive | S::TimedOut => ready(
            "The source is done: settling returns it to you",
            locked_now,
            Party::You,
        ),
        S::Rejected | S::Merged => ready(
            "Lost to the challenger who had the source rejected: it moves when the challenge settles",
            0,
            Party::Challenger,
        )
        .through_challenge(),
        S::Candidate => locked(
            "Held while the source is a candidate: back once it opens or is set aside; a challenger who has it rejected takes it",
        ),
        _ => locked(
            "Held until the source's opening print is accepted, or it is set aside at listing",
        ),
    }
}

const fn stuck_verdict() -> Verdict {
    stuck(NEVER_RESOLVED)
}

/// A support stake: back once the source is done, whatever became of it
/// (accepted, set aside, rejected or merged).
pub fn support_stake(
    status: OracleSourceStatus,
    stake: u64,
    settled: bool,
    stuck: bool,
) -> Verdict {
    use OracleSourceStatus as S;
    match status {
        _ if settled => done("Back in your collateral", Some(stake), Party::You),
        _ if stuck => stuck_verdict(),
        S::Active | S::Inactive | S::Merged | S::Rejected | S::TimedOut => ready(
            "The source is done: settling returns it to you",
            stake,
            Party::You,
        ),
        _ => locked("Held until the source opens or is set aside"),
    }
}

/// A source challenge's bond (its challenger's): back with the source's
/// listing bond when upheld; to the source's proposer when dismissed or not
/// ruled by the Opening's start.
pub fn source_challenge(
    standing: challenges::Standing,
    settled: bool,
    bond: u64,
    listing: u64,
) -> Verdict {
    use challenges::Standing as W;
    match (standing, settled) {
        (W::Rejected | W::Merged, true) => done(
            "Upheld: your bond came back with the source's listing bond",
            Some(bond.saturating_add(listing)),
            Party::You,
        ),
        (W::Dismissed | W::Lapsed, true) => done(
            "Dismissed: your bond went to the source's proposer",
            Some(0),
            Party::Proposer,
        ),
        (W::Cancelled, true) => done("Cancelled: your bond came back", Some(bond), Party::You),
        (_, true) => done("Settled", None, Party::You),
        (W::Waiting, _) => locked("Waiting for the ruling (Scramble day 7)"),
        (W::Council, _) => locked("With the council"),
        (W::Rejected | W::Merged, _) => ready(
            "Upheld: settling returns your bond with the source's listing bond",
            bond.saturating_add(listing),
            Party::You,
        ),
        (W::Dismissed, _) => ready(
            "Dismissed: settling pays your bond to the source's proposer",
            0,
            Party::Proposer,
        ),
        (W::Lapsed, _) => ready(
            "Not ruled in time, so dismissed: settling pays your bond to the source's proposer",
            0,
            Party::Proposer,
        ),
        (W::Cancelled, _) => ready("Cancelled: settling returns your bond", bond, Party::You),
    }
}

/// A challenge of the owner's source that pays them: one dismissed (or not
/// ruled in time) brings its bond to the proposer.
pub fn source_challenge_owed(
    standing: challenges::Standing,
    settled: bool,
    bond: u64,
) -> Option<Verdict> {
    use challenges::Standing as W;
    match (standing, settled) {
        (W::Dismissed | W::Lapsed, true) => Some(done(
            "A challenge of your source was dismissed: its bond went to you",
            Some(bond),
            Party::You,
        )),
        (W::Dismissed | W::Lapsed, false) => Some(ready(
            "A challenge of your source was dismissed: settling brings its bond to you",
            bond,
            Party::You,
        )),
        _ => None,
    }
}

/// An opening print's stake (its claimant's).
pub fn opening_claim(
    standing: prints::Standing,
    escrow: OracleEscrowDisposition,
    stake: u64,
) -> Verdict {
    use OracleEscrowDisposition as E;
    use prints::Standing as P;
    match (standing, escrow) {
        (_, E::Refunded) => done("Back in your collateral", Some(stake), Party::You),
        (_, E::Slashed | E::Transferred) => done(
            "Lost: it went to the challenger who won",
            Some(0),
            Party::Challenger,
        ),
        (P::Challengeable, _) => {
            locked("Open to challenge: it can be accepted once its 300 slots pass")
        }
        (P::Acceptable, _) => locked("Unchallenged: accept it, then its stake can come back"),
        (P::Challenged, _) => locked(
            "Challenged: waiting for the oracle authority's ruling, made by hand on the opening day",
        ),
        (P::Council, _) => locked("With the council"),
        (P::Kept, _) => locked(
            "Kept by the ruling: once the challenge settles (its bond comes to you) the print can be challenged again, then accepted",
        ),
        (P::Replaced, _) => ready(
            "The challenger's value won: it goes to the challenger when the challenge settles",
            0,
            Party::Challenger,
        )
        .through_challenge(),
        (P::Rejected, _) => ready(
            "Rejected: it goes to the challenger when the challenge settles",
            0,
            Party::Challenger,
        )
        .through_challenge(),
        (P::Accepted, _) => ready("Accepted: settling returns it to you", stake, Party::You),
        (P::Cancelled, _) => ready("Called off: settling returns it to you", stake, Party::You),
        (P::Expired, _) => stuck(
            "Stuck: the print was not accepted or ruled on before listing, so nothing can return it",
        ),
    }
}

/// An opening challenge's bond (its challenger's). `stake` is the print's.
pub fn opening_challenge(
    standing: prints::Standing,
    escrow: OracleEscrowDisposition,
    bond: u64,
    stake: u64,
) -> Verdict {
    use OracleEscrowDisposition as E;
    use prints::Standing as P;
    match (standing, escrow) {
        (_, E::Transferred) => done(
            "Upheld: your value became the opening print and your bond its stake (listed as your opening print)",
            None,
            Party::You,
        ),
        (P::Kept, E::Slashed) | (_, E::Slashed) => done(
            "Dismissed: your bond went to the print's claimant",
            Some(0),
            Party::Claimant,
        ),
        (_, E::Refunded) => done("Back in your collateral", None, Party::You),
        (P::Challenged, _) => {
            locked("Waiting for the oracle authority's ruling, made by hand on the opening day")
        }
        (P::Council, _) => locked("With the council"),
        (P::Expired, _) => stuck(
            "Stuck: the challenge was not ruled on before listing, so nothing can return your bond",
        ),
        (P::Kept, _) => ready(
            "Dismissed: settling pays your bond to the print's claimant",
            0,
            Party::Claimant,
        ),
        (P::Replaced, _) => ready(
            "Upheld: settling makes your value the opening print, your bond its stake, and brings you the claimant's stake",
            stake,
            Party::You,
        ),
        (P::Rejected, _) => ready(
            "Upheld: settling returns your bond with the print's stake",
            bond.saturating_add(stake),
            Party::You,
        ),
        (P::Cancelled, _) => ready("Called off: settling returns your bond", bond, Party::You),
        _ => locked("Waiting for the print to be decided"),
    }
}

/// A challenge of the owner's print that pays them: one the ruling dismissed
/// brings its bond to the claimant.
pub fn opening_challenge_owed(
    standing: prints::Standing,
    escrow: OracleEscrowDisposition,
    bond: u64,
) -> Option<Verdict> {
    match (standing, escrow) {
        (prints::Standing::Kept, OracleEscrowDisposition::Unsettled) => Some(ready(
            "A challenge of your print was dismissed: settling brings its bond to you and gives the print a new window",
            bond,
            Party::You,
        )),
        (_, OracleEscrowDisposition::Slashed) => Some(done(
            "A challenge of your print was dismissed: its bond went to you",
            Some(bond),
            Party::You,
        )),
        _ => None,
    }
}

/// An update claim's stake (its claimant's), with the challenge it took, if
/// any, at the chain's slot.
pub fn update_claim(
    claim: &OracleUpdateClaimV2,
    challenge: Option<&OracleUpdateChallenge>,
    slot: u64,
) -> Verdict {
    use OracleChallengeStatus as C;
    use OracleClaimStatus as S;
    use OracleEscrowDisposition as E;
    let c = &claim.claim;
    let stake = c.stake;
    let bond = challenge.map_or(0, |ch| ch.bond);
    match c.escrow_disposition {
        E::Refunded => {
            let won =
                challenge.is_some_and(|ch| ch.status == C::Rejected) && c.status == S::Finalized;
            return done(
                if won {
                    "Back in your collateral, with the challenger's bond"
                } else {
                    "Back in your collateral"
                },
                Some(if won {
                    stake.saturating_add(bond)
                } else {
                    stake
                }),
                Party::You,
            );
        }
        E::Slashed | E::Transferred => {
            return done(
                "Lost: it went to the challenger who won",
                Some(0),
                Party::Challenger,
            );
        }
        E::Unsettled => {}
    }
    if claim.council_review_pending {
        return locked("With the council");
    }
    match (c.status, challenge.map(|ch| ch.status)) {
        (S::Committed, _) if slot <= claim.reveal_deadline_slot => {
            locked("Sealed: reveal it before its deadline, or its stake only comes back unrevealed")
        }
        (S::Committed, _) => Verdict {
            status: Status::Settleable,
            words: "Never revealed: its stake comes back (`petri oracle updates expire`)",
            to_you: Some(stake),
            pays: Some(Party::You),
            via_challenge: false,
        },
        (S::Revealed, None) => {
            locked("Revealed: open to challenge for a day, then the keeper accepts it")
        }
        (S::Revealed, Some(C::RuleReview | C::Open)) => {
            locked("Challenged: waiting for the ruling")
        }
        (S::Revealed, Some(C::RuleReviewUnresolved)) => locked("With the council"),
        (S::Finalized, None) => ready("Accepted: settling returns it to you", stake, Party::You),
        (S::Finalized, Some(C::Rejected)) => ready(
            "Accepted over a challenge: settling returns it with the challenger's bond",
            stake.saturating_add(bond),
            Party::You,
        )
        .through_challenge(),
        (S::Rejected, Some(C::Accepted)) => ready(
            "Rejected on a challenge: it goes to the challenger when the challenge settles",
            0,
            Party::Challenger,
        )
        .through_challenge(),
        (S::Rejected, Some(C::Rejected)) | (S::TimedOut, Some(C::Cancelled)) => ready(
            "Called off: settling returns it to you (and the challenger's bond to them)",
            stake,
            Party::You,
        )
        .through_challenge(),
        (S::TimedOut, None) => ready("Timed out: settling returns it to you", stake, Party::You),
        _ => locked("Waiting for the update to be decided"),
    }
}

/// An update challenge's bond (its challenger's), with the claim it
/// challenged.
pub fn update_challenge(c: &OracleUpdateChallenge, claim: Option<&OracleUpdateClaimV2>) -> Verdict {
    use OracleChallengeStatus as C;
    use OracleClaimStatus as S;
    use OracleEscrowDisposition as E;
    let stake = claim.map_or(0, |k| k.claim.stake);
    match c.escrow_disposition {
        E::Refunded if c.status == C::Accepted => {
            return done(
                "Upheld: your bond came back with the update's stake",
                Some(c.bond.saturating_add(stake)),
                Party::You,
            );
        }
        E::Refunded => return done("Back in your collateral", Some(c.bond), Party::You),
        E::Slashed | E::Transferred => {
            return done(
                "Dismissed: your bond went to the update's claimant",
                Some(0),
                Party::Claimant,
            );
        }
        E::Unsettled => {}
    }
    if claim.is_some_and(|k| k.council_review_pending) {
        return locked("With the council");
    }
    match (c.status, claim.map(|k| k.claim.status)) {
        (C::Open | C::RuleReview, _) => locked("Waiting for the ruling"),
        (C::RuleReviewUnresolved, _) => locked("With the council"),
        (C::Rejected, Some(S::Finalized)) => ready(
            "Dismissed: settling pays your bond to the update's claimant",
            0,
            Party::Claimant,
        ),
        (C::Accepted, Some(S::Rejected)) => ready(
            "Upheld: settling returns your bond with the update's stake",
            c.bond.saturating_add(stake),
            Party::You,
        ),
        (C::Rejected, Some(S::Rejected)) | (C::Cancelled, Some(S::TimedOut)) => {
            ready("Called off: settling returns your bond", c.bond, Party::You)
        }
        _ => locked("Waiting for the update to be decided"),
    }
}

/// A sponsored source's listing bond (its sponsor's): back to the sponsor
/// through the agreement once the source opens or is set aside; a challenger
/// who has it rejected takes it.
pub fn sponsorship(
    a: &SourceSponsorship,
    status: OracleSourceStatus,
    locked_now: u64,
    stuck: bool,
) -> Verdict {
    use OracleSourceStatus as S;
    match a.outcome {
        1 | 3 => return done("Back in your collateral", Some(a.bond), Party::You),
        2 => {
            return done(
                "Lost: it went to the challenger who had the source rejected",
                Some(0),
                Party::Challenger,
            );
        }
        _ => {}
    }
    match status {
        _ if stuck => stuck_verdict(),
        S::Active | S::Inactive | S::TimedOut if locked_now > 0 => ready(
            "The source is done: settling returns the bond to you",
            a.bond,
            Party::You,
        ),
        S::Rejected | S::Merged => ready(
            "Lost to the challenger who had the source rejected: it moves when the challenge settles",
            0,
            Party::Challenger,
        )
        .through_challenge(),
        _ => locked("Held until the sponsored source opens or is set aside"),
    }
}

/// What settles one escrow, its parties resolved.
#[derive(Clone, Debug)]
pub enum Settle {
    /// Tag 169 on a listing bond, support stake or source challenge.
    Prelisting(PrelistingEscrow),
    OpeningClaim {
        claim: Pubkey,
        claimant: Pubkey,
    },
    OpeningChallenge {
        claim: Pubkey,
        challenge: Pubkey,
        claimant: Pubkey,
        challenger: Pubkey,
    },
    UpdateClaim {
        claim: Pubkey,
        claimant: Pubkey,
    },
    UpdateChallenge {
        claim: Pubkey,
        challenge: Pubkey,
        claimant: Pubkey,
        challenger: Pubkey,
    },
    /// Tag 167: an update commitment nobody revealed by its deadline.
    Unrevealed {
        claim: Pubkey,
        claimant: Pubkey,
    },
    /// A sponsored listing bond: settled to its agreement, then reconciled
    /// to the sponsor.
    Sponsorship {
        agreement: Pubkey,
        reserve: Pubkey,
        funder: Pubkey,
    },
}

impl Settle {
    /// The instructions, signed by `cranker`.
    pub fn steps(
        &self,
        program: &Pubkey,
        cranker: &Pubkey,
        month: &MonthRef,
        source: &Pubkey,
    ) -> Vec<(String, Instruction)> {
        let (period, month_key) = (month.period.address, month.address);
        let collateral = |owner: &Pubkey| derive_user_collateral_pda(program, owner).0;
        let escrow = |kind, accounts| ix::settle_oracle_usdc_escrow(program, kind, accounts);
        let head = || {
            vec![
                AccountMeta::new_readonly(*cranker, true),
                AccountMeta::new_readonly(period, false),
                AccountMeta::new_readonly(month_key, false),
            ]
        };
        let one = |ix: Instruction| vec![("settleEscrow".to_string(), ix)];
        match self {
            Self::Prelisting(e) => one(ix::settle_oracle_usdc_prelisting_escrow(
                program, cranker, &period, &month_key, e,
            )),
            Self::OpeningClaim { claim, claimant } => {
                let mut accounts = head();
                accounts.extend([
                    AccountMeta::new_readonly(*source, false),
                    AccountMeta::new(*claim, false),
                    AccountMeta::new(collateral(claimant), false),
                ]);
                one(escrow(OracleEscrowKind::OpeningClaim, accounts))
            }
            Self::OpeningChallenge {
                claim,
                challenge,
                claimant,
                challenger,
            } => {
                let mut accounts = head();
                accounts.extend([
                    AccountMeta::new(*source, false),
                    AccountMeta::new(*claim, false),
                    AccountMeta::new(*challenge, false),
                    AccountMeta::new(collateral(claimant), false),
                    AccountMeta::new(collateral(challenger), false),
                ]);
                one(escrow(OracleEscrowKind::OpeningChallenge, accounts))
            }
            Self::UpdateClaim { claim, claimant } => {
                let mut accounts = head();
                accounts.extend([
                    AccountMeta::new_readonly(*source, false),
                    AccountMeta::new(*claim, false),
                    AccountMeta::new(collateral(claimant), false),
                    AccountMeta::new_readonly(
                        ix::find_oracle_update_guard_address(program, &month_key, claim),
                        false,
                    ),
                ]);
                one(escrow(OracleEscrowKind::UpdateClaim, accounts))
            }
            Self::UpdateChallenge {
                claim,
                challenge,
                claimant,
                challenger,
            } => {
                let mut accounts = head();
                accounts.extend([
                    AccountMeta::new(*claim, false),
                    AccountMeta::new(*challenge, false),
                    AccountMeta::new(collateral(claimant), false),
                    AccountMeta::new(collateral(challenger), false),
                    AccountMeta::new(
                        ix::find_oracle_update_guard_address(program, &month_key, claim),
                        false,
                    ),
                ]);
                one(escrow(OracleEscrowKind::UpdateChallenge, accounts))
            }
            Self::Unrevealed { claim, claimant } => vec![(
                "settleExpiredCommitment".to_string(),
                ix::settle_expired_oracle_update_commitment_v3(
                    program, cranker, &period, &month_key, source, claim, claimant,
                ),
            )],
            Self::Sponsorship {
                agreement,
                reserve,
                funder,
            } => {
                let accounts = SponsoredAccounts {
                    agreement: *agreement,
                    reserve: *reserve,
                    funder: *funder,
                    exposure: exposure_address(program, reserve, funder).0,
                };
                vec![
                    (
                        "settleListingBond".to_string(),
                        ix::settle_oracle_usdc_prelisting_escrow(
                            program,
                            cranker,
                            &period,
                            &month_key,
                            &PrelistingEscrow::ListingBond {
                                source: *source,
                                proposer: *agreement,
                            },
                        ),
                    ),
                    (
                        "reconcileSponsorship".to_string(),
                        ix::sponsored_reconcile(program, cranker, &month_key, source, &accounts),
                    ),
                ]
            }
        }
    }

    /// The petri command that sends it.
    pub fn command(&self, month: &MonthRef, source: &Pubkey) -> String {
        let id = month.period.expiry_id();
        let settle =
            |kind: &str| format!("petri oracle stakes settle {id} --kind {kind} --source {source}");
        match self {
            Self::Prelisting(PrelistingEscrow::ListingBond { proposer, .. }) => {
                format!("{} --owner {proposer}", settle("listing-bond"))
            }
            Self::Prelisting(PrelistingEscrow::SupportStake { supporter, .. }) => {
                format!("{} --owner {supporter}", settle("support-stake"))
            }
            Self::Prelisting(PrelistingEscrow::SourceChallenge { challenge, .. }) => {
                format!("{} --challenge {challenge}", settle("source-challenge"))
            }
            Self::OpeningClaim { .. } => settle("opening-claim"),
            Self::OpeningChallenge { challenge, .. } => {
                format!("{} --challenge {challenge}", settle("opening-challenge"))
            }
            Self::UpdateClaim { claim, .. } => {
                format!("{} --claim {claim}", settle("update-claim"))
            }
            Self::UpdateChallenge {
                claim, challenge, ..
            } => format!(
                "{} --claim {claim} --challenge {challenge}",
                settle("update-challenge")
            ),
            Self::Unrevealed { claim, .. } => {
                format!("petri oracle updates expire {id} --claim {claim}")
            }
            Self::Sponsorship { .. } => format!("petri oracle sponsorships settle {source}"),
        }
    }
}

/// The source challenge's settle, its parties resolved.
fn source_challenge_settle(source: &SourceRef, key: &Pubkey, c: &OracleSourceChallenge) -> Settle {
    Settle::Prelisting(PrelistingEscrow::SourceChallenge {
        source: source.address,
        proposer: source.state.proposer,
        challenge: *key,
        challenger: c.challenger,
        comparison_source: (c.comparison_source != Pubkey::default())
            .then_some(c.comparison_source),
    })
}

/// The kinds `stakes` lists: the escrow kinds `stakes settle --kind` takes,
/// and a sponsored listing bond.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Escrow(EscrowKind),
    Sponsorship,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Escrow(EscrowKind::ListingBond) => "listing-bond",
            Self::Escrow(EscrowKind::SupportStake) => "support-stake",
            Self::Escrow(EscrowKind::SourceChallenge) => "source-challenge",
            Self::Escrow(EscrowKind::OpeningClaim) => "opening-claim",
            Self::Escrow(EscrowKind::OpeningChallenge) => "opening-challenge",
            Self::Escrow(EscrowKind::UpdateClaim) => "update-claim",
            Self::Escrow(EscrowKind::UpdateChallenge) => "update-challenge",
            Self::Sponsorship => "sponsorship-bond",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Escrow(EscrowKind::ListingBond) => "Listing bond",
            Self::Escrow(EscrowKind::SupportStake) => "Support",
            Self::Escrow(EscrowKind::SourceChallenge) => "Source challenge",
            Self::Escrow(EscrowKind::OpeningClaim) => "Opening print",
            Self::Escrow(EscrowKind::OpeningChallenge) => "Print challenge",
            Self::Escrow(EscrowKind::UpdateClaim) => "Update",
            Self::Escrow(EscrowKind::UpdateChallenge) => "Update challenge",
            Self::Sponsorship => "Sponsorship",
        }
    }
}

/// One bond or stake of the owner's, or someone else's that settles to them
/// (`owed`).
pub struct Stake<'a> {
    pub month: &'a MonthRef,
    pub source: &'a SourceRef,
    pub kind: Kind,
    pub owed: bool,
    pub claim: Option<Pubkey>,
    pub challenge: Option<Pubkey>,
    pub agreement: Option<Pubkey>,
    pub amount: u64,
    pub verdict: Verdict,
    /// Who `Party` names, by address.
    pub parties: Parties,
    pub settle: Option<Settle>,
    /// What moves a locked one along (accept it, reveal it, set the source
    /// aside), when the owner can.
    pub next: Option<String>,
}

/// The wallets an escrow's parties are.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parties {
    pub you: Pubkey,
    pub proposer: Option<Pubkey>,
    pub claimant: Option<Pubkey>,
    pub challenger: Option<Pubkey>,
}

impl Parties {
    fn of(&self, party: Party) -> Option<Pubkey> {
        match party {
            Party::You => Some(self.you),
            Party::Proposer => self.proposer,
            Party::Claimant => self.claimant,
            Party::Challenger => self.challenger,
        }
    }
}

impl Stake<'_> {
    /// Whether `settle --all` settles it: decided, and settling pays the
    /// owner something.
    pub fn pays_owner(&self) -> bool {
        self.verdict.status == Status::Settleable
            && self.verdict.to_you.is_some_and(|a| a > 0)
            && self.settle.is_some()
    }

    fn command(&self) -> Option<String> {
        (self.verdict.status == Status::Settleable)
            .then(|| {
                self.settle
                    .as_ref()
                    .map(|s| s.command(self.month, &self.source.address))
            })
            .flatten()
    }
}

/// The owner's bonds and stakes in the months `market` names, read from
/// chain: one program scan per account type (sources, source challenges,
/// opening challenges when there are prints, the owner's update claims and
/// update challenges, the owner's sponsorships), the rest read by address.
/// Settled ones of months whose period has settled or failed, and settled
/// updates and update challenges, are left out unless `all_settled`; how
/// many is the second value.
pub fn read<'a>(
    oracle: &Oracle,
    owner: &Pubkey,
    months: &'a [MonthRef],
    sources: &'a [SourceRef],
    now: &ChainNow,
    all_settled: bool,
) -> Result<(Vec<Stake<'a>>, usize)> {
    let month_of: HashMap<Pubkey, &MonthRef> = months.iter().map(|m| (m.address, m)).collect();
    let source_of: HashMap<Pubkey, &SourceRef> = sources.iter().map(|s| (s.address, s)).collect();
    let single = match months {
        [only] => Some(only.address),
        _ => None,
    };
    let opening_of = |m: &MonthRef| Opening::of(m);
    let stuck_of = |s: &SourceRef| {
        month_of
            .get(&s.state.month)
            .is_some_and(|m| stuck_source(s.state.status, opening_of(m), now.unix))
    };
    let mut out: Vec<Stake> = Vec::new();
    let you = *owner;
    let base = Parties {
        you,
        ..Default::default()
    };

    // The SKU pools the listing bonds come from (a settled one keeps no
    // amount of its own).
    let proposed: Vec<&SourceRef> = sources.iter().filter(|s| s.state.proposer == you).collect();
    let challenges = oracle.source_challenges(single.as_ref())?;
    let pairs: Vec<(Pubkey, [u8; 32])> = {
        let mut seen = HashSet::new();
        proposed
            .iter()
            .copied()
            .chain(
                challenges
                    .iter()
                    .filter(|(_, c)| c.challenger == you)
                    .filter_map(|(_, c)| source_of.get(&c.source).copied()),
            )
            .map(|s| (s.state.month, s.state.bucket_id))
            .filter(|p| seen.insert(*p))
            .collect()
    };
    let pools: HashMap<(Pubkey, [u8; 32]), u64> = pairs
        .iter()
        .copied()
        .zip(oracle.sku_pools(&pairs)?)
        .filter_map(|(pair, pool)| Some((pair, pool?.listing_bond)))
        .collect();
    let listing_of = |s: &SourceRef| {
        pools
            .get(&(s.state.month, s.state.bucket_id))
            .copied()
            .unwrap_or(0)
    };
    // The challenge that had a source rejected or merged, by source.
    let upheld: HashMap<Pubkey, (&Pubkey, &OracleSourceChallenge)> = challenges
        .iter()
        .filter(|(_, c)| c.status == OracleChallengeStatus::Accepted)
        .map(|(k, c)| (c.source, (k, c)))
        .collect();

    // Listing bonds of the sources the owner proposed.
    for source in &proposed {
        let Some(&month) = month_of.get(&source.state.month) else {
            continue;
        };
        let s = &source.state;
        let verdict = listing_bond(
            s.status,
            s.listing_bond_locked,
            listing_of(source),
            stuck_of(source),
        );
        let by = upheld.get(&source.address);
        let settle = if verdict.via_challenge {
            by.map(|(k, c)| source_challenge_settle(source, k, c))
        } else {
            Some(Settle::Prelisting(PrelistingEscrow::ListingBond {
                source: source.address,
                proposer: you,
            }))
        };
        out.push(Stake {
            month,
            source,
            kind: Kind::Escrow(EscrowKind::ListingBond),
            owed: false,
            claim: None,
            challenge: by.map(|(k, _)| **k),
            agreement: None,
            amount: if s.listing_bond_locked > 0 {
                s.listing_bond_locked
            } else {
                listing_of(source)
            },
            verdict,
            parties: Parties {
                challenger: by.map(|(_, c)| c.challenger),
                ..base
            },
            settle,
            next: expire_next(month, source, now.unix),
        });
    }

    // Support stakes.
    for (source, position) in sources.iter().zip(oracle.support_positions(sources, &you)?) {
        let (Some(position), Some(&month)) = (position, month_of.get(&source.state.month)) else {
            continue;
        };
        let settled =
            position.released || position.escrow_disposition != OracleEscrowDisposition::Unsettled;
        out.push(Stake {
            month,
            source,
            kind: Kind::Escrow(EscrowKind::SupportStake),
            owed: false,
            claim: None,
            challenge: None,
            agreement: None,
            amount: position.support_stake,
            verdict: support_stake(
                source.state.status,
                position.support_stake,
                settled,
                stuck_of(source),
            ),
            parties: base,
            settle: Some(Settle::Prelisting(PrelistingEscrow::SupportStake {
                source: source.address,
                supporter: you,
            })),
            next: None,
        });
    }

    // Source challenges: the owner's bonds, and dismissed challenges of the
    // owner's sources (their bonds come to the owner).
    for (key, c) in &challenges {
        let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
        else {
            continue;
        };
        let window = challenges::Window::of(&month.state);
        let standing = challenges::Standing::of(
            c.status,
            Some(source.state.status),
            now.unix,
            window.ruled_by,
        );
        let settled = c.escrow_disposition != OracleEscrowDisposition::Unsettled;
        let verdict = if c.challenger == you {
            Some((
                source_challenge(standing, settled, c.bond, listing_of(source)),
                false,
            ))
        } else if source.state.proposer == you {
            source_challenge_owed(standing, settled, c.bond).map(|v| (v, true))
        } else {
            None
        };
        let Some((verdict, owed)) = verdict else {
            continue;
        };
        out.push(Stake {
            month,
            source,
            kind: Kind::Escrow(EscrowKind::SourceChallenge),
            owed,
            claim: None,
            challenge: Some(*key),
            agreement: None,
            amount: c.bond,
            verdict,
            parties: Parties {
                proposer: Some(source.state.proposer),
                challenger: Some(c.challenger),
                ..base
            },
            settle: Some(source_challenge_settle(source, key, c)),
            next: None,
        });
    }

    // Opening prints and their challenges.
    let claims = oracle.opening_claims(sources)?;
    let opening_challenges = if claims.is_empty() {
        Vec::new()
    } else {
        oracle.opening_challenges(single.as_ref())?
    };
    let mut of_claim: HashMap<Pubkey, Vec<&(Pubkey, OracleOpeningClaimChallenge)>> = HashMap::new();
    for c in &opening_challenges {
        of_claim.entry(c.1.claim).or_default().push(c);
    }
    for claim in &claims {
        let c = &claim.state;
        let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
        else {
            continue;
        };
        let open = Opening::of(month).is_open(now.unix);
        let of = of_claim
            .get(&claim.address)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let active = of.iter().rev().copied().find(|(_, ch)| {
            ch.claim_attempt == c.attempt
                && ch.escrow_disposition == OracleEscrowDisposition::Unsettled
        });
        let standing = prints::Standing::of_print(c, active.map(|(_, a)| a), open, now.slot);
        if c.claimant == you {
            let verdict = opening_claim(standing, c.escrow_disposition, c.stake);
            let settle = if verdict.via_challenge {
                active.map(|(k, ch)| Settle::OpeningChallenge {
                    claim: claim.address,
                    challenge: *k,
                    claimant: c.claimant,
                    challenger: ch.challenger,
                })
            } else {
                Some(Settle::OpeningClaim {
                    claim: claim.address,
                    claimant: c.claimant,
                })
            };
            out.push(Stake {
                month,
                source,
                kind: Kind::Escrow(EscrowKind::OpeningClaim),
                owed: false,
                claim: Some(claim.address),
                challenge: active.map(|(k, _)| *k),
                agreement: None,
                amount: c.stake,
                verdict,
                parties: Parties {
                    claimant: Some(c.claimant),
                    challenger: active.map(|(_, ch)| ch.challenger),
                    ..base
                },
                settle,
                next: (standing == prints::Standing::Acceptable).then(|| {
                    format!(
                        "petri oracle prints finalize {} --source {}",
                        month.period.expiry_id(),
                        source.address
                    )
                }),
            });
        }
        for (key, ch) in of.iter().map(|e| (&e.0, &e.1)) {
            let mine = ch.challenger == you;
            let against = c.claimant == you && ch.claim_attempt == c.attempt;
            if !mine && !against {
                continue;
            }
            let claim_status = if ch.claim_attempt == c.attempt {
                c.status
            } else {
                OracleOpeningClaimStatus::Rejected
            };
            let ch_standing = prints::Standing::of_challenge(ch, claim_status, open);
            let verdict = if mine {
                Some(opening_challenge(
                    ch_standing,
                    ch.escrow_disposition,
                    ch.bond,
                    c.stake,
                ))
            } else {
                opening_challenge_owed(ch_standing, ch.escrow_disposition, ch.bond)
            };
            let Some(verdict) = verdict else { continue };
            out.push(Stake {
                month,
                source,
                kind: Kind::Escrow(EscrowKind::OpeningChallenge),
                owed: !mine,
                claim: Some(claim.address),
                challenge: Some(*key),
                agreement: None,
                amount: ch.bond,
                verdict,
                parties: Parties {
                    claimant: Some(c.claimant),
                    challenger: Some(ch.challenger),
                    ..base
                },
                settle: Some(Settle::OpeningChallenge {
                    claim: claim.address,
                    challenge: *key,
                    claimant: c.claimant,
                    challenger: ch.challenger,
                }),
                next: None,
            });
        }
    }

    // Update claims (with the challenge each took) and the owner's update
    // challenges (with the claim each challenged).
    let updates = oracle.update_claims_of(&you, single.as_ref())?;
    // Only an unsettled claim's challenge decides anything: a settled one's
    // stake has moved already (and a busy wallet has thousands).
    let pairs: Vec<(Pubkey, &OracleUpdateClaimV2)> = updates
        .iter()
        .filter(|(_, c)| c.claim.escrow_disposition == OracleEscrowDisposition::Unsettled)
        .map(|(k, c)| (*k, c))
        .collect();
    let found: HashMap<Pubkey, (Pubkey, OracleUpdateChallenge)> = pairs
        .iter()
        .zip(oracle.update_challenges_on(&pairs)?)
        .filter_map(|((claim, _), challenge)| Some((*claim, challenge?)))
        .collect();
    let taken: Vec<Option<(Pubkey, OracleUpdateChallenge)>> =
        updates.iter().map(|(k, _)| found.get(k).cloned()).collect();
    for ((key, claim), challenge) in updates.iter().zip(&taken) {
        let c = &claim.claim;
        let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
        else {
            continue;
        };
        let verdict = update_claim(claim, challenge.as_ref().map(|(_, ch)| ch), now.slot);
        let settle = match (c.status, challenge) {
            (OracleClaimStatus::Committed, _) => Some(Settle::Unrevealed {
                claim: *key,
                claimant: c.claimant,
            }),
            (_, Some((ck, ch))) => Some(Settle::UpdateChallenge {
                claim: *key,
                challenge: *ck,
                claimant: c.claimant,
                challenger: ch.challenger,
            }),
            (_, None) => Some(Settle::UpdateClaim {
                claim: *key,
                claimant: c.claimant,
            }),
        };
        out.push(Stake {
            month,
            source,
            kind: Kind::Escrow(EscrowKind::UpdateClaim),
            owed: false,
            claim: Some(*key),
            challenge: challenge.as_ref().map(|(k, _)| *k),
            agreement: None,
            amount: c.stake,
            verdict,
            parties: Parties {
                claimant: Some(c.claimant),
                challenger: challenge.as_ref().map(|(_, ch)| ch.challenger),
                ..base
            },
            settle,
            next: (c.status == OracleClaimStatus::Committed
                && now.slot <= claim.reveal_deadline_slot)
                .then(|| format!("petri oracle updates reveal {key}")),
        });
    }
    let mine = oracle.update_challenges_of(&you, single.as_ref())?;
    let claim_keys: Vec<Pubkey> = mine.iter().map(|(_, c)| c.claim).collect();
    for ((key, ch), claim) in mine.iter().zip(oracle.update_claims_at(&claim_keys)?) {
        let Some(claim) = claim else { continue };
        let c = &claim.claim;
        let (Some(&month), Some(&source)) = (month_of.get(&c.month), source_of.get(&c.source))
        else {
            continue;
        };
        out.push(Stake {
            month,
            source,
            kind: Kind::Escrow(EscrowKind::UpdateChallenge),
            owed: false,
            claim: Some(ch.claim),
            challenge: Some(*key),
            agreement: None,
            amount: ch.bond,
            verdict: update_challenge(ch, Some(&claim)),
            parties: Parties {
                claimant: Some(c.claimant),
                challenger: Some(ch.challenger),
                ..base
            },
            settle: Some(Settle::UpdateChallenge {
                claim: ch.claim,
                challenge: *key,
                claimant: c.claimant,
                challenger: ch.challenger,
            }),
            next: None,
        });
    }

    // Listing bonds the owner sponsored.
    for (agreement, a) in oracle.sponsorships(&you)? {
        let (Some(&month), Some(&source)) = (month_of.get(&a.month), source_of.get(&a.source))
        else {
            continue;
        };
        let verdict = sponsorship(
            &a,
            source.state.status,
            source.state.listing_bond_locked,
            stuck_of(source),
        );
        let by = upheld.get(&source.address);
        let settle = if verdict.via_challenge {
            by.map(|(k, c)| source_challenge_settle(source, k, c))
        } else {
            Some(Settle::Sponsorship {
                agreement,
                reserve: a.reserve,
                funder: a.funder,
            })
        };
        out.push(Stake {
            month,
            source,
            kind: Kind::Sponsorship,
            owed: false,
            claim: None,
            challenge: by.map(|(k, _)| **k),
            agreement: Some(agreement),
            amount: a.bond,
            verdict,
            parties: Parties {
                challenger: by.map(|(_, c)| c.challenger),
                ..base
            },
            settle,
            next: None,
        });
    }
    // Settled ones are history: those of a month whose period has settled
    // (or failed), and settled updates and update challenges (a running Game
    // settles them by the thousand), are left out unless asked for, and
    // counted.
    let before = out.len();
    out.retain(|s| {
        all_settled
            || s.verdict.status != Status::Settled
            || !(finished(s.month)
                || matches!(
                    s.kind,
                    Kind::Escrow(EscrowKind::UpdateClaim | EscrowKind::UpdateChallenge)
                ))
    });
    let earlier = before - out.len();
    out.sort_by_key(|s| {
        (
            s.month.period.period.expiry_ts,
            s.month.address,
            s.source.state.bucket_id,
            s.source.address,
            s.kind.name(),
        )
    });
    Ok((out, earlier))
}

/// A month whose period settled or failed: its money's stages are over.
fn finished(month: &MonthRef) -> bool {
    matches!(
        month.period.period.status,
        ameba_oracle::interface::PeriodStatus::Settled
            | ameba_oracle::interface::PeriodStatus::Failed
    )
}

/// `prints expire`, for the proposer of a frozen source left without a print
/// once listing passed (its listing bond waits on that).
fn expire_next(month: &MonthRef, source: &SourceRef, now: u64) -> Option<String> {
    let opening = Opening::of(month);
    (source.state.status == OracleSourceStatus::Frozen
        && !source.state.opening_submitted
        && opening.phase == ameba_oracle::state::OraclePhase::Opening
        && now >= opening.listing
        && now < opening.expiry)
        .then(|| {
            format!(
                "petri oracle prints expire {} --source {}",
                month.period.expiry_id(),
                source.address
            )
        })
}

/// The months `market` names, their sources (one scan) and the chain's time.
fn scope(
    oracle: &Oracle,
    market: Option<&str>,
    expiry: Option<&str>,
) -> Result<(Vec<MonthRef>, Vec<SourceRef>, ChainNow)> {
    let months = oracle.months_in(&oracle.periods_of(market, expiry)?)?;
    let now = ChainNow::read(oracle.ctx)?;
    let sources = if months.is_empty() {
        Vec::new()
    } else {
        let single = match months.as_slice() {
            [only] => Some(only.address),
            _ => None,
        };
        let known: HashSet<Pubkey> = months.iter().map(|m| m.address).collect();
        oracle
            .sources_in(single.as_ref())?
            .into_iter()
            .filter(|s| known.contains(&s.state.month))
            .collect()
    };
    Ok((months, sources, now))
}

fn to_json(stake: &Stake, url: Option<&String>) -> Value {
    let v = &stake.verdict;
    json!({
        "month": stake.month.period.expiry_id(),
        "monthAddress": stake.month.address.to_string(),
        "kind": stake.kind.name(),
        "role": if stake.owed { "owed" } else { "yours" },
        "source": stake.source.address.to_string(),
        "url": url,
        "bucket": stake.source.bucket_label(),
        "bucketName": stake.source.bucket_name(),
        "claim": stake.claim.map(|k| k.to_string()),
        "challenge": stake.challenge.map(|k| k.to_string()),
        "agreement": stake.agreement.map(|k| k.to_string()),
        "amountAtoms": stake.amount.to_string(),
        "toYouAtoms": v.to_you.map(|a| a.to_string()),
        "paysTo": v.pays.and_then(|p| stake.parties.of(p)).map(|k| k.to_string()),
        "status": v.status.code(),
        "statusText": v.words,
        "settleCommand": stake.command(),
        "nextCommand": stake.next,
    })
}

/// The money in `stakes`: the owner's own still held while their stage runs,
/// what settling pays them now, and their own that nothing can move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub locked: u64,
    pub settleable: u64,
    pub stuck: u64,
}

impl Totals {
    pub fn of(stakes: &[Stake]) -> Self {
        let mut t = Self::default();
        for s in stakes {
            match s.verdict.status {
                Status::Locked if !s.owed => t.locked = t.locked.saturating_add(s.amount),
                Status::Stuck if !s.owed => t.stuck = t.stuck.saturating_add(s.amount),
                Status::Settleable if s.pays_owner() => {
                    t.settleable = t.settleable.saturating_add(s.verdict.to_you.unwrap_or(0))
                }
                _ => {}
            }
        }
        t
    }
}

/// `petri oracle stakes [MARKET] [--owner]`: the owner's bonds and stakes,
/// read from chain, with what each waits for or where it went.
pub fn list(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
    all_settled: bool,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let owner = ctx.owner(owner)?;
    let (months, sources, now) = scope(&oracle, market, expiry)?;
    let (stakes, earlier) = read(&oracle, &owner, &months, &sources, &now, all_settled)?;
    let listed: Vec<SourceRef> = {
        let mut seen = HashSet::new();
        stakes
            .iter()
            .map(|s| s.source)
            .filter(|s| seen.insert(s.address))
            .cloned()
            .collect()
    };
    let urls: HashMap<Pubkey, String> = listed
        .iter()
        .zip(oracle.locators(&listed)?)
        .filter_map(|(s, url)| Some((s.address, url?)))
        .collect();
    let totals = Totals::of(&stakes);
    let mut lines = vec![p.bold(&format!("Bonds and stakes of {owner}"))];
    let mut table = Table::new(&[
        "Month",
        "Kind",
        "Source",
        "Bucket",
        "Amount",
        "Status",
        "To you",
        "Where it stands",
    ])
    .right(&[4, 6]);
    let mut notes = Vec::new();
    for s in &stakes {
        table.row(vec![
            s.month.period.expiry_id(),
            format!("{}{}", s.kind.label(), if s.owed { " (owed)" } else { "" }),
            short(&s.source.address.to_string()),
            s.source.bucket_name(),
            usdc(s.amount),
            s.verdict.status.code().to_string(),
            s.verdict.to_you.map_or("-".into(), usdc),
            s.verdict.words.to_string(),
        ]);
        for command in s.command().iter().chain(s.next.iter()) {
            notes.push(format!("  {command}"));
        }
    }
    if table.is_empty() {
        lines.push("  none".to_string());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        lines.extend(notes.iter().map(|n| p.dim(n)));
        lines.push(String::new());
        lines.push(format!(
            "  Held while their stage runs: {} USDC · settles to you now: {} USDC{}",
            usdc(totals.locked),
            usdc(totals.settleable),
            if totals.stuck > 0 {
                format!(" · stuck: {} USDC", usdc(totals.stuck))
            } else {
                String::new()
            }
        ));
        if totals.settleable > 0 {
            lines.push(p.dim(&format!(
                "  petri oracle stakes settle --all{}   (settles each one that pays you)",
                market.map(|m| format!(" {m}")).unwrap_or_default()
            )));
        }
    }
    if earlier > 0 {
        lines.push(p.dim(&format!(
            "  ({earlier} settled ones not shown, settled updates and those of months that have settled: --settled lists them)"
        )));
    }
    let items: Vec<Value> = stakes
        .iter()
        .map(|s| to_json(s, urls.get(&s.source.address)))
        .collect();
    Ok(Report::new(
        json!({
            "ok": true,
            "owner": owner.to_string(),
            "chainTimeUtc": iso(now.unix),
            "stakes": items,
            "totals": {
                "lockedAtoms": totals.locked.to_string(),
                "settleableAtoms": totals.settleable.to_string(),
                "stuckAtoms": totals.stuck.to_string(),
            },
            "settledNotShown": earlier,
        }),
        lines.join("\n"),
    ))
}

/// `petri oracle stakes settle --all [MARKET]`: every one of the owner's that
/// pays them now, a few to a transaction. Those that pay someone else (a
/// dismissed challenge, a lost print) are left to whoever they pay.
pub fn settle_all(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    owner: Option<&str>,
    send_args: &SendArgs,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let cranker = ctx.wallet_pubkey()?;
    let owner = match owner {
        Some(text) => ctx.owner(Some(text))?,
        None => cranker,
    };
    let (months, sources, now) = scope(&oracle, market, expiry)?;
    let (stakes, _) = read(&oracle, &owner, &months, &sources, &now, false)?;
    let mut seen = HashSet::new();
    let mut steps = Vec::new();
    let mut settled = Vec::new();
    let mut review = Vec::new();
    for s in stakes.iter().filter(|s| s.pays_owner()) {
        let Some(settle) = &s.settle else { continue };
        let command = settle.command(s.month, &s.source.address);
        // Two of the owner's can settle through one challenge.
        if !seen.insert(command.clone()) {
            continue;
        }
        steps.extend(settle.steps(&oracle.program, &cranker, s.month, &s.source.address));
        let to_you = s.verdict.to_you.unwrap_or(0);
        review.push(format!(
            "{} · {} · {}: {} USDC to you ({})",
            s.month.period.expiry_id(),
            s.kind.label(),
            s.source.bucket_name(),
            usdc(to_you),
            s.verdict.words
        ));
        settled.push(json!({
            "month": s.month.period.expiry_id(),
            "monthAddress": s.month.address.to_string(),
            "kind": s.kind.name(),
            "source": s.source.address.to_string(),
            "bucketName": s.source.bucket_name(),
            "claim": s.claim.map(|k| k.to_string()),
            "challenge": s.challenge.map(|k| k.to_string()),
            "amountAtoms": s.amount.to_string(),
            "toYouAtoms": to_you.to_string(),
            "command": command,
        }));
    }
    let skipped: Vec<Value> = stakes
        .iter()
        .filter(|s| s.verdict.status == Status::Settleable && !s.pays_owner())
        .map(|s| {
            json!({
                "month": s.month.period.expiry_id(),
                "kind": s.kind.name(),
                "source": s.source.address.to_string(),
                "amountAtoms": s.amount.to_string(),
                "paysTo": s.verdict.pays.and_then(|p| s.parties.of(p)).map(|k| k.to_string()),
                "why": s.verdict.words,
                "command": s.command(),
            })
        })
        .collect();
    let total: u64 = stakes
        .iter()
        .filter(|s| s.pays_owner())
        .filter_map(|s| s.verdict.to_you)
        .sum();
    if steps.is_empty() {
        return Ok(Report::new(
            json!({
                "ok": true,
                "action": "oracle.stakes.settle-all",
                "owner": owner.to_string(),
                "settled": [],
                "skipped": skipped,
                "totalToYouAtoms": "0",
            }),
            format!(
                "Nothing of {owner}'s settles to them now (`petri oracle stakes` shows each and what it waits for)."
            ),
        ));
    }
    let count = settled.len();
    if owner != cranker {
        review.push(format!(
            "They settle into {owner}'s collateral; your wallet only pays the fees"
        ));
    }
    if let Some(m) = months.first().filter(|_| months.len() == 1) {
        review.insert(0, month_line(m));
    }
    let write = Write {
        action: "oracle.stakes.settle-all",
        title: format!(
            "Settle {count} bond{} and stake{} paying {} USDC into {}'s oracle collateral",
            if count == 1 { "" } else { "s" },
            if count == 1 { "" } else { "s" },
            usdc(total),
            short(&owner.to_string())
        ),
        review,
        details: json!({
            "owner": owner.to_string(),
            "settled": settled,
            "skipped": skipped,
            "totalToYouAtoms": total.to_string(),
        }),
        txs: send::pack(&oracle.program, &cranker, steps),
    };
    send::run(ctx, p, write, send_args.simulate)
}

/// The guard an update claim's settle proves absent, or its challenge's
/// settle takes (the program's `derive_oracle_update_challenge_guard_pda`).
#[cfg(test)]
fn guard_of(program: &Pubkey, month: &Pubkey, claim: &Pubkey) -> Pubkey {
    ameba_oracle::state::derive_oracle_update_challenge_guard_pda(program, month, claim).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use OracleEscrowDisposition as E;
    use OracleSourceStatus as S;

    #[test]
    fn a_listing_bond_comes_back_once_its_source_is_done() {
        let v = |status, locked_now, stuck| listing_bond(status, locked_now, 2_000_000, stuck);
        assert_eq!(v(S::Candidate, 2_000_000, false).status, Status::Locked);
        assert_eq!(v(S::Frozen, 2_000_000, false).status, Status::Locked);
        let back = v(S::Active, 2_000_000, false);
        assert_eq!(
            (back.status, back.to_you, back.pays),
            (Status::Settleable, Some(2_000_000), Some(Party::You))
        );
        assert!(!back.via_challenge);
        for done in [S::Inactive, S::TimedOut] {
            assert_eq!(v(done, 2_000_000, false).status, Status::Settleable);
        }
        // Rejected: the challenger takes it, through the challenge.
        let lost = v(S::Rejected, 2_000_000, false);
        assert_eq!(
            (lost.status, lost.to_you, lost.pays, lost.via_challenge),
            (Status::Settleable, Some(0), Some(Party::Challenger), true)
        );
        assert_eq!(v(S::Rejected, 0, false).status, Status::Settled);
        assert_eq!(v(S::Rejected, 0, false).to_you, Some(0));
        let settled = v(S::Active, 0, false);
        assert_eq!(
            (settled.status, settled.to_you),
            (Status::Settled, Some(2_000_000))
        );
        assert_eq!(v(S::OpeningPending, 2_000_000, true).status, Status::Stuck);
    }

    #[test]
    fn a_support_stake_comes_back_whatever_became_of_the_source() {
        for done in [S::Active, S::Inactive, S::Merged, S::Rejected, S::TimedOut] {
            let v = support_stake(done, 1_000_000, false, false);
            assert_eq!(
                (v.status, v.to_you),
                (Status::Settleable, Some(1_000_000)),
                "{done:?}"
            );
        }
        assert_eq!(
            support_stake(S::Candidate, 1, false, false).status,
            Status::Locked
        );
        assert_eq!(
            support_stake(S::Active, 1, true, false).status,
            Status::Settled
        );
        assert_eq!(
            support_stake(S::OpeningPending, 1, false, true).status,
            Status::Stuck
        );
    }

    fn opening() -> Opening {
        Opening {
            phase: ameba_oracle::state::OraclePhase::Opening,
            start: 1_000,
            listing: 2_000,
            expiry: 9_000,
        }
    }

    #[test]
    fn a_source_is_stuck_when_its_opening_can_no_longer_end() {
        assert!(!stuck_source(S::OpeningPending, opening(), 1_500));
        assert!(stuck_source(S::OpeningPending, opening(), 2_000));
        assert!(!stuck_source(S::Frozen, opening(), 2_000));
        assert!(stuck_source(S::Frozen, opening(), 9_000));
        assert!(!stuck_source(S::Active, opening(), 9_000));
        let game = Opening {
            phase: ameba_oracle::state::OraclePhase::Game,
            ..opening()
        };
        assert!(!stuck_source(S::Frozen, game, 9_000));
        assert!(stuck_source(S::OpeningPending, game, 1_500));
    }

    #[test]
    fn a_source_challenge_pays_by_its_ruling() {
        use challenges::Standing as W;
        let v = |standing, settled| source_challenge(standing, settled, 1_000_000, 2_000_000);
        assert_eq!(v(W::Waiting, false).status, Status::Locked);
        assert_eq!(v(W::Council, false).status, Status::Locked);
        let upheld = v(W::Rejected, false);
        assert_eq!(
            (upheld.status, upheld.to_you),
            (Status::Settleable, Some(3_000_000))
        );
        let dismissed = v(W::Dismissed, false);
        assert_eq!(
            (dismissed.to_you, dismissed.pays),
            (Some(0), Some(Party::Proposer))
        );
        assert_eq!(v(W::Lapsed, false).pays, Some(Party::Proposer));
        assert_eq!(v(W::Cancelled, false).to_you, Some(1_000_000));
        assert_eq!(v(W::Merged, true).status, Status::Settled);
        // The proposer of a source whose challenge was dismissed is owed its bond.
        assert_eq!(
            source_challenge_owed(W::Dismissed, false, 1_000_000)
                .unwrap()
                .to_you,
            Some(1_000_000)
        );
        assert_eq!(source_challenge_owed(W::Rejected, false, 1_000_000), None);
    }

    #[test]
    fn a_prints_stake_and_its_challenges_pay_by_the_ruling() {
        use prints::Standing as P;
        assert_eq!(
            opening_claim(P::Challengeable, E::Unsettled, 5).status,
            Status::Locked
        );
        assert_eq!(
            opening_claim(P::Acceptable, E::Unsettled, 5).status,
            Status::Locked
        );
        let accepted = opening_claim(P::Accepted, E::Unsettled, 5);
        assert_eq!(
            (accepted.status, accepted.to_you, accepted.via_challenge),
            (Status::Settleable, Some(5), false)
        );
        let lost = opening_claim(P::Rejected, E::Unsettled, 5);
        assert_eq!(
            (lost.to_you, lost.pays, lost.via_challenge),
            (Some(0), Some(Party::Challenger), true)
        );
        assert!(opening_claim(P::Replaced, E::Unsettled, 5).via_challenge);
        assert_eq!(
            opening_claim(P::Kept, E::Unsettled, 5).status,
            Status::Locked
        );
        assert_eq!(
            opening_claim(P::Expired, E::Unsettled, 5).status,
            Status::Stuck
        );
        assert_eq!(
            opening_claim(P::Accepted, E::Refunded, 5).status,
            Status::Settled
        );
        // The challenger's side.
        let kept = opening_challenge(P::Kept, E::Unsettled, 1, 5);
        assert_eq!((kept.to_you, kept.pays), (Some(0), Some(Party::Claimant)));
        assert_eq!(
            opening_challenge(P::Replaced, E::Unsettled, 1, 5).to_you,
            Some(5)
        );
        assert_eq!(
            opening_challenge(P::Rejected, E::Unsettled, 1, 5).to_you,
            Some(6)
        );
        assert_eq!(
            opening_challenge(P::Challenged, E::Unsettled, 1, 5).status,
            Status::Locked
        );
        assert_eq!(
            opening_challenge(P::Expired, E::Unsettled, 1, 5).status,
            Status::Stuck
        );
        assert_eq!(
            opening_challenge(P::Replaced, E::Transferred, 1, 5).status,
            Status::Settled
        );
        // The claimant is owed a dismissed challenge's bond.
        assert_eq!(
            opening_challenge_owed(P::Kept, E::Unsettled, 1)
                .unwrap()
                .to_you,
            Some(1)
        );
        assert_eq!(opening_challenge_owed(P::Replaced, E::Unsettled, 1), None);
    }

    fn update(status: OracleClaimStatus, escrow: OracleEscrowDisposition) -> OracleUpdateClaimV2 {
        OracleUpdateClaimV2 {
            claim: ameba_oracle::state::OracleUpdateClaimData {
                stake: 5,
                status,
                escrow_disposition: escrow,
                ..Default::default()
            },
            reveal_deadline_slot: 100,
            ..Default::default()
        }
    }

    fn update_challenge_of(
        status: OracleChallengeStatus,
        escrow: OracleEscrowDisposition,
    ) -> OracleUpdateChallenge {
        OracleUpdateChallenge {
            bond: 1,
            status,
            escrow_disposition: escrow,
            ..Default::default()
        }
    }

    #[test]
    fn an_updates_stake_comes_back_once_it_is_decided() {
        use OracleChallengeStatus as C;
        use OracleClaimStatus as U;
        let v = |claim: &OracleUpdateClaimV2, c: Option<&OracleUpdateChallenge>, slot| {
            update_claim(claim, c, slot)
        };
        // Sealed, then unrevealed past its deadline: `updates expire` returns it.
        assert_eq!(
            v(&update(U::Committed, E::Unsettled), None, 100).status,
            Status::Locked
        );
        let late = v(&update(U::Committed, E::Unsettled), None, 101);
        assert_eq!((late.status, late.to_you), (Status::Settleable, Some(5)));
        assert_eq!(
            v(&update(U::Revealed, E::Unsettled), None, 200).status,
            Status::Locked
        );
        assert_eq!(
            v(&update(U::Finalized, E::Unsettled), None, 200).to_you,
            Some(5)
        );
        let won = v(
            &update(U::Finalized, E::Unsettled),
            Some(&update_challenge_of(C::Rejected, E::Unsettled)),
            200,
        );
        assert_eq!((won.to_you, won.via_challenge), (Some(6), true));
        let lost = v(
            &update(U::Rejected, E::Unsettled),
            Some(&update_challenge_of(C::Accepted, E::Unsettled)),
            200,
        );
        assert_eq!((lost.to_you, lost.pays), (Some(0), Some(Party::Challenger)));
        let mut council = update(U::Revealed, E::Unsettled);
        council.council_review_pending = true;
        assert_eq!(v(&council, None, 200).words, "With the council");
        assert_eq!(
            v(&update(U::Finalized, E::Refunded), None, 200).status,
            Status::Settled
        );
        // The challenger's side.
        let claim = update(U::Rejected, E::Unsettled);
        let upheld = update_challenge(
            &update_challenge_of(C::Accepted, E::Unsettled),
            Some(&claim),
        );
        assert_eq!(upheld.to_you, Some(6));
        let claim = update(U::Finalized, E::Unsettled);
        let dismissed = update_challenge(
            &update_challenge_of(C::Rejected, E::Unsettled),
            Some(&claim),
        );
        assert_eq!(
            (dismissed.to_you, dismissed.pays),
            (Some(0), Some(Party::Claimant))
        );
        assert_eq!(
            update_challenge(
                &update_challenge_of(C::RuleReview, E::Unsettled),
                Some(&claim)
            )
            .status,
            Status::Locked
        );
    }

    #[test]
    fn settle_commands_name_the_escrow_exactly() {
        let month = MonthRef {
            period: super::super::chain::PeriodRef {
                address: Pubkey::new_unique(),
                period: ameba_oracle::interface::Period::new_forming(
                    255,
                    super::super::chain::product_id("RAMX").unwrap(),
                    1_790_000_000,
                    1_798_761_600,
                    6,
                ),
                symbol: "RAMX".into(),
            },
            address: Pubkey::new_unique(),
            state: Default::default(),
        };
        let (source, who, claim, challenge) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let base = "petri oracle stakes settle RAMX-202612";
        assert_eq!(
            Settle::Prelisting(PrelistingEscrow::SupportStake {
                source,
                supporter: who
            })
            .command(&month, &source),
            format!("{base} --kind support-stake --source {source} --owner {who}")
        );
        assert_eq!(
            Settle::UpdateChallenge {
                claim,
                challenge,
                claimant: who,
                challenger: who
            }
            .command(&month, &source),
            format!(
                "{base} --kind update-challenge --source {source} --claim {claim} --challenge {challenge}"
            )
        );
        assert_eq!(
            Settle::Unrevealed {
                claim,
                claimant: who
            }
            .command(&month, &source),
            format!("petri oracle updates expire RAMX-202612 --claim {claim}")
        );
        // The update claim's settle proves the absent guard its challenge would hold.
        let program = Pubkey::new_unique();
        let steps = Settle::UpdateClaim {
            claim,
            claimant: who,
        }
        .steps(&program, &who, &month, &source);
        assert_eq!(steps.len(), 1);
        let guard = guard_of(&program, &month.address, &claim);
        assert_eq!(steps[0].1.accounts.last().unwrap().pubkey, guard);
        // A sponsorship settles its listing bond, then reconciles.
        let steps = Settle::Sponsorship {
            agreement: who,
            reserve: claim,
            funder: challenge,
        }
        .steps(&program, &who, &month, &source);
        assert_eq!(
            steps.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["settleListingBond", "reconcileSponsorship"]
        );
    }

    #[test]
    fn totals_count_your_own_held_money_and_what_settles_to_you() {
        let month = MonthRef {
            period: super::super::chain::PeriodRef {
                address: Pubkey::new_unique(),
                period: ameba_oracle::interface::Period::new_forming(255, [1; 32], 1, 2, 6),
                symbol: "RAMX".into(),
            },
            address: Pubkey::new_unique(),
            state: Default::default(),
        };
        let source = SourceRef {
            address: Pubkey::new_unique(),
            state: Default::default(),
        };
        let stake = |verdict: Verdict, amount, owed| Stake {
            month: &month,
            source: &source,
            kind: Kind::Escrow(EscrowKind::SupportStake),
            owed,
            claim: None,
            challenge: None,
            agreement: None,
            amount,
            verdict,
            parties: Parties::default(),
            settle: Some(Settle::OpeningClaim {
                claim: source.address,
                claimant: source.address,
            }),
            next: None,
        };
        let stakes = [
            stake(locked("held"), 3, false),
            stake(ready("back", 2, Party::You), 2, false),
            stake(ready("lost", 0, Party::Challenger), 4, false),
            stake(ready("owed", 1, Party::You), 1, true),
            stake(stuck("stuck"), 7, false),
        ];
        assert_eq!(
            Totals::of(&stakes),
            Totals {
                locked: 3,
                settleable: 3,
                stuck: 7
            }
        );
        assert!(stakes[1].pays_owner() && !stakes[2].pays_owner());
    }
}
