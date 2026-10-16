//! Current-roster council decisions, using the oracle's exact instruction builder.
use super::{
    args::{CouncilCommand, CouncilKind, CouncilTarget},
    chain::{self, Oracle},
    send::{self, Tx, Write},
    skus,
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result},
    market::pubkey,
    view::Paint,
};
use ameba_oracle::{
    instruction::{self as ix, OracleCouncilActionV1},
    processor::{CouncilCase, CouncilRound, council_decision, decode_council_v3_with_profile},
    state::{OracleEmergencyDisputeKind, derive_oracle_bucket_median_pda},
};
use serde_json::json;
use solana_program::{hash::hashv, instruction::AccountMeta, pubkey::Pubkey};

fn kind(value: CouncilKind) -> OracleEmergencyDisputeKind {
    match value {
        CouncilKind::Source => OracleEmergencyDisputeKind::Source,
        CouncilKind::Opening => OracleEmergencyDisputeKind::Opening,
        CouncilKind::Update => OracleEmergencyDisputeKind::Update,
        CouncilKind::Bucket => OracleEmergencyDisputeKind::BucketMedian,
    }
}

pub fn run(ctx: &Ctx, p: Paint, command: &CouncilCommand) -> Result<Report> {
    let (target, operation, choice) = match command {
        CouncilCommand::Propose { target } => (target, 0, 0),
        CouncilCommand::Vote { target, choice } => (target, 1, *choice),
        CouncilCommand::Apply { target } => (target, 2, 0),
        CouncilCommand::Recover { target } => {
            if !matches!(target.kind, CouncilKind::Bucket) {
                return Err(Error::msg("recover requires a bucket dispute"));
            }
            (target, 2, 0)
        }
    };
    execute(ctx, p, target, operation, choice)
}

fn execute(
    ctx: &Ctx,
    p: Paint,
    input: &CouncilTarget,
    operation: u8,
    choice: u8,
) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(&input.month.market, input.month.expiry.as_deref())?;
    let owner = ctx.owner(None)?;
    let target = pubkey(&input.target, "council target")?;
    let target_id = skus::parse_hex32(&input.target_id, "council target ID")?;
    let kind = kind(input.kind);
    if target_id == [0; 32] {
        return Err(Error::msg("council target ID must be nonzero"));
    }
    if kind == OracleEmergencyDisputeKind::BucketMedian
        && (target
            != derive_oracle_bucket_median_pda(&oracle.program, &month.address, &target_id).0
            || !input.account.is_empty())
    {
        return Err(Error::msg(
            "bucket recovery requires its canonical bucket account and no extra accounts",
        ));
    }
    let dep = ctx.deployment()?;
    let controller = if let Some(g) = &dep.governance_controller {
        pubkey(&g.program, "governance controller")?
    } else if dep.oracle_release_tag.as_deref() == Some("localnet-dev") && dep.dev_chain() {
        // The dev chain's ungated oracle, in the dev profile only.
        oracle.program
    } else {
        return Err(Error::msg(
            "council commands require a trusted governance controller",
        ));
    };
    let config = Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], &controller).0;
    let account = ctx
        .rpc
        .account(&config)?
        .ok_or_else(|| Error::msg("controller council config is missing"))?;
    if account.executable {
        return Err(Error::msg(
            "controller council config must be a state account",
        ));
    }
    // The council's development profile: the dev chain's own controller, in
    // the dev profile only.
    let development = controller
        != ameba_sdk_futures::futures::governance::PRODUCTION_CONTROLLER_PROGRAM_ID
        && dep.dev_chain();
    let view = decode_council_v3_with_profile(
        &controller,
        &config,
        &account.owner,
        &account.data,
        development,
    )
    .map_err(|e| Error::msg(format!("invalid council config: {e}")))?;
    if operation == 1 && !view.seats.contains(&owner) {
        return Err(Error::msg(
            "the attached wallet is not a current council seat",
        ));
    }
    let case_key = ix::find_council_case_address(&oracle.program, &month.address, &target);
    let target_account = ctx
        .rpc
        .account(&target)?
        .ok_or_else(|| Error::msg("council target is missing"))?;
    if target_account.owner != oracle.program || target_account.executable {
        return Err(Error::msg("council target belongs to another program"));
    }
    let mut decision = None;
    let case_hash = if operation == 0 {
        [0; 32]
    } else {
        let account = ctx.rpc.account(&case_key)?;
        let case: CouncilCase = chain::decode(
            &oracle.program,
            &case_key,
            account.as_ref(),
            CouncilCase::LEN,
            "council case",
        )?;
        validate_case(&case, &month.address, &target, &target_id, kind)?;
        if case.evidence_hash
            != hashv(&[
                b"amoeba-council-evidence-v1",
                target.as_ref(),
                &target_account.data,
            ])
            .to_bytes()
        {
            return Err(Error::msg("the dispute evidence changed; refresh the case"));
        }
        if operation == 1 && choice >= case.choices {
            return Err(Error::msg("choice is outside this case's alternatives"));
        }
        let round_key = ix::find_council_round_address(&oracle.program, &case_key, view.epoch);
        let round_account = ctx.rpc.account(&round_key)?;
        let ballots = if round_account.is_some() {
            let round: CouncilRound = chain::decode(
                &oracle.program,
                &round_key,
                round_account.as_ref(),
                CouncilRound::LEN,
                "council round",
            )?;
            if !round.initialized
                || round.discriminator != *b"OCR"
                || round.version != 1
                || round.case != case_key
                || round.epoch != view.epoch
                || round.seats_hash != view.digest
            {
                return Err(Error::msg(
                    "council round does not match the current roster",
                ));
            }
            round.ballots
        } else {
            [255; 5]
        };
        if operation == 1 {
            let seat = view
                .seats
                .iter()
                .position(|s| *s == owner)
                .ok_or_else(|| Error::msg("wallet is not a current council seat"))?;
            if ballots[seat] != 255 {
                return Err(Error::msg(
                    "this council seat has already voted in the current round",
                ));
            }
        }
        let (outcome, mask, majority) = council_decision(&ballots, case.choices, case.fallback)
            .map_err(|e| Error::msg(e.to_string()))?;
        decision = Some(
            json!({"choice":outcome,"votes":mask.count_ones(),"majority":majority,"fallback":case.fallback,"deadline":case.deadline}),
        );
        case.case_hash
    };
    let remaining = input
        .account
        .iter()
        .map(|text| {
            let (readonly, text) = text
                .strip_prefix("ro:")
                .map_or((false, text.as_str()), |s| (true, s));
            let key = pubkey(text, "council target account")?;
            Ok(if readonly {
                AccountMeta::new_readonly(key, false)
            } else {
                AccountMeta::new(key, false)
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let action = OracleCouncilActionV1 {
        operation,
        kind,
        target_id,
        expected_case_hash: case_hash,
        expected_epoch: view.epoch,
        expected_seats_hash: view.digest,
        choice,
    };
    let instruction = ix::council_action(
        &oracle.program,
        &owner,
        &month.period.address,
        &month.address,
        &target,
        &config,
        action,
        (operation == 1).then_some(&owner),
        remaining,
    );
    send::run(
        ctx,
        p,
        Write {
            action: "council",
            title: format!("Council decision for {}", month.period.label()),
            review: vec![
                format!("Target: {target}"),
                format!(
                    "Council epoch: {}; three of five current seats decide",
                    view.epoch
                ),
                format!("Operation: {operation}; choice: {choice}"),
            ],
            details: json!({"case":case_key.to_string(),"target":target.to_string(),"epoch":view.epoch,"caseHash":skus::hex(&case_hash),"choice":choice,"currentDecision":decision}),
            txs: vec![Tx::one("council", instruction)],
        },
        input.month.send.simulate,
    )
}

fn validate_case(
    case: &CouncilCase,
    month: &Pubkey,
    target: &Pubkey,
    id: &[u8; 32],
    kind: OracleEmergencyDisputeKind,
) -> Result<()> {
    if !case.initialized
        || case.discriminator != *b"OCC"
        || case.version != 1
        || case.finalized
        || case.month != *month
        || case.target != *target
        || case.target_id != *id
        || case.kind != kind
        || case.case_hash == [0; 32]
        || !(2..=3).contains(&case.choices)
        || case.fallback >= case.choices
    {
        return Err(Error::msg("case does not match this unresolved dispute"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_validation_refuses_replaced_targets_and_finalized_decisions() {
        let month = Pubkey::new_unique();
        let target = Pubkey::new_unique();
        let id = [1; 32];
        let mut case = CouncilCase {
            discriminator: *b"OCC",
            version: 1,
            initialized: true,
            bump: 1,
            month,
            target,
            target_id: id,
            case_hash: [2; 32],
            evidence_hash: [3; 32],
            kind: OracleEmergencyDisputeKind::BucketMedian,
            choices: 2,
            fallback: 0,
            snapshot_slot: 1,
            opened_slot: 2,
            deadline: 3,
            finalized: false,
            outcome: 0,
            reason: 0,
            decision_epoch: 0,
            seat_mask: 0,
            resolved_slot: 0,
        };
        assert!(validate_case(&case, &month, &target, &id, case.kind).is_ok());
        assert!(validate_case(&case, &month, &Pubkey::new_unique(), &id, case.kind).is_err());
        assert!(validate_case(&case, &month, &target, &[9; 32], case.kind).is_err());
        case.finalized = true;
        assert!(validate_case(&case, &month, &target, &id, case.kind).is_err());
    }

    #[test]
    fn a_rotated_or_split_round_cannot_supply_three_current_votes() {
        let decision = council_decision(&[1, 1, 1, 255, 255], 2, 0).unwrap();
        assert_eq!(decision, (1, 7, true));
        assert_eq!(
            council_decision(&[1, 1, 0, 0, 255], 2, 0).unwrap(),
            (0, 0, false)
        );
        assert_eq!(council_decision(&[255; 5], 2, 0).unwrap(), (0, 0, false));
    }
}
