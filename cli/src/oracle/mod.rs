//! The standalone oracle program. Reads come from the futures API's oracle
//! documents (`/v1/oracle/public-snapshot`, `/v1/oracle/catalog`) and its
//! source-candidate store (`/oracle/source-candidates`); writes are built with
//! the pinned oracle crate's instruction builders against the trusted
//! deployment's oracle program and signed by the attached wallet. Sponsorship
//! (tag 30/22: sponsor, settle, claim, reserve withdraw) is in `sponsor.rs`,
//! the other reward claims (tag 174) in `rewards.rs`, source challenges (tag
//! 161) and how they end in `challenges.rs`, opening prints (tags 162-165,
//! 81) and how they end in `prints.rs`, Game updates (166-169, 198, 199,
//! 201) and how they end in `updates.rs`, a wallet's bonds and stakes and
//! their settlement (tag 169) in `stakes.rs`. Cleanup (30/24) has no command.

pub mod args;
pub mod chain;
pub mod challenges;
pub mod claims;
pub mod council;
pub mod errors;
pub mod observe;
pub mod prints;
pub mod reads;
pub mod rewards;
pub mod send;
pub mod series;
pub mod skus;
pub mod sponsor;
pub mod stakes;
pub mod updates;
pub mod writes;

use crate::{
    backend::CliError,
    cli::Cli,
    futures::{self, Report, ctx::Ctx, error::Result, view::Paint},
};
use args::{
    CandidateCommand, CollateralCommand, EvidenceCommand, ObserveCommand, OracleCommand,
    PrintCommand, ReserveCommand, RewardCommand, SeriesCommand, SourceCommand, SponsorshipCommand,
    StakeCommand, UpdateCommand,
};

/// Runs one oracle command with the futures connection settings (`--rpc`,
/// `--api`, `--deployment`, `--keypair`). Program logs from a failed
/// transaction go to stderr in plain mode.
pub fn run(cli: &Cli, command: Option<&OracleCommand>) -> std::result::Result<Report, CliError> {
    let settings = futures::settings::Settings::from_cli(cli).map_err(CliError::new)?;
    let json = settings.json;
    let paint = Paint::new(settings.color && !json);
    let ctx = Ctx::new(settings);
    dispatch(&ctx, paint, command).map_err(|e| {
        if !json {
            for line in &e.logs {
                eprintln!("  log: {line}");
            }
        }
        let message = e.message.replace("\n  ", " | ").replace('\n', " | ");
        let (code, category, retryable) =
            if message.contains("unreachable") || message.contains("ORACLE_SNAPSHOT_PREPARING") {
                ("ORACLE_SERVICE_UNAVAILABLE", "unavailable", true)
            } else if message.contains(" failed: ") || message.contains("signature ") {
                ("ORACLE_TRANSACTION_FAILED", "execution_failed", false)
            } else if message.starts_with("refusing") {
                ("ORACLE_REFUSED", "denied", false)
            } else {
                ("ORACLE_INVALID", "validation", false)
            };
        CliError::coded(code, category, message, retryable)
    })
}

fn dispatch(ctx: &Ctx, p: Paint, command: Option<&OracleCommand>) -> Result<Report> {
    use OracleCommand as O;
    let Some(command) = command else {
        return reads::months(ctx, p, None);
    };
    match command {
        O::Months { market } => reads::months(ctx, p, market.as_deref()),
        O::Latest { market } => reads::latest(ctx, p, market.as_deref()),
        O::History { market } => reads::history(ctx, p, market.as_deref()),
        O::Catalog => reads::catalog(ctx, p),
        O::Recipe(list) => reads::recipe(
            ctx,
            p,
            list.market.as_deref(),
            list.expiry.as_deref(),
            list.query.as_deref(),
            list.limit,
        ),
        O::Sources {
            command: None,
            list,
        } => reads::sources(
            ctx,
            p,
            list.market.as_deref(),
            list.expiry.as_deref(),
            list.query.as_deref(),
            list.limit,
        ),
        O::Sources {
            command: Some(command),
            ..
        } => match command {
            SourceCommand::Propose {
                target,
                bucket,
                url,
                source_type,
                definition,
                definition_file,
                bond,
                source_id,
            } => writes::source_propose(
                ctx,
                p,
                target,
                bucket,
                url,
                source_type,
                definition.as_deref(),
                definition_file.as_deref(),
                bond,
                source_id.as_deref(),
            ),
            SourceCommand::Support {
                target,
                source,
                stake,
            } => writes::source_support(ctx, p, target, source, stake.as_deref()),
            SourceCommand::Close { target } => writes::sources_close(ctx, p, target),
            SourceCommand::Challenge {
                target,
                source,
                reason,
                bond,
                why,
                why_file,
                evidence_hash,
                comparison,
                challenge_id,
            } => challenges::challenge(
                ctx,
                p,
                target,
                &challenges::Request {
                    source,
                    reason: *reason,
                    bond: bond.as_deref(),
                    evidence: challenges::Evidence::of(
                        why.as_deref(),
                        why_file.as_deref(),
                        evidence_hash.as_deref(),
                    ),
                    comparison: comparison.as_deref(),
                    challenge_id: challenge_id.as_deref(),
                },
            ),
        },
        O::Series { command } => match command {
            SeriesCommand::Template {
                bucket,
                direct,
                out,
            } => series::template(bucket, *direct, out.as_deref()),
            SeriesCommand::Check {
                file,
                market,
                expiry,
            } => series::check(ctx, p, file, market.as_deref(), expiry.as_deref()),
            SeriesCommand::Propose {
                target,
                file,
                bond,
                source_id,
            } => series::propose(ctx, p, target, file, bond.as_deref(), source_id.as_deref()),
            SeriesCommand::Show { market, expiry } => {
                series::show(ctx, p, market, expiry.as_deref())
            }
        },
        O::Observe {
            command: Some(command),
            ..
        } => match command {
            ObserveCommand::Template {
                market,
                expiry,
                source,
                out,
            } => observe::template(ctx, market, expiry.as_deref(), source, out.as_deref()),
            ObserveCommand::Record {
                market,
                expiry,
                source,
                claim,
                file,
            } => observe::record(
                ctx,
                p,
                market,
                expiry.as_deref(),
                source,
                claim,
                file.as_deref(),
            ),
        },
        O::Observe {
            command: None,
            send: a,
        } => {
            let (Some(market), Some(source), Some(file)) = (&a.market, &a.source, &a.file) else {
                return futures::error::bail(
                    "give the month, --source and --file: petri oracle observe RAMX-202612 --source <SOURCE> --file obs.json",
                );
            };
            observe::observe(
                ctx,
                p,
                market,
                a.expiry.as_deref(),
                source,
                file,
                a.stake.as_deref(),
                a.reveal,
                a.accept,
                a.send.simulate,
            )
        }
        O::Candidates {
            command: None,
            filter,
        } => reads::candidates(
            ctx,
            p,
            filter.market.as_deref(),
            filter.expiry.as_deref(),
            filter.agent.as_deref(),
            filter.limit,
        ),
        O::Candidates {
            command: Some(CandidateCommand::Show { hash }),
            ..
        } => reads::candidate(ctx, p, hash),
        O::Collateral {
            command: None,
            owner,
        } => writes::collateral_show(ctx, p, owner.as_deref()),
        O::Collateral {
            command: Some(CollateralCommand::Deposit { amount, send }),
            ..
        } => writes::collateral_deposit(ctx, p, amount, send),
        O::Collateral {
            command: Some(CollateralCommand::Withdraw { amount, send }),
            ..
        } => writes::collateral_withdraw(ctx, p, amount, send),
        O::Evidence { command } => match command {
            EvidenceCommand::Write { kind, body, send } => writes::evidence_write(
                ctx,
                p,
                *kind,
                body.text.as_deref(),
                body.file.as_deref(),
                send,
            ),
            EvidenceCommand::Show {
                object,
                kind,
                text,
                payer,
            } => writes::evidence_show(
                ctx,
                p,
                object.as_deref(),
                *kind,
                text.as_deref(),
                payer.as_deref(),
            ),
            EvidenceCommand::Close { object, send } => writes::evidence_close(ctx, p, object, send),
        },
        O::Prints {
            command: None,
            market,
            expiry,
            owner,
        } => prints::list(
            ctx,
            p,
            market.as_deref(),
            expiry.as_deref(),
            owner.as_deref(),
        ),
        O::Prints {
            command: Some(command),
            ..
        } => match command {
            PrintCommand::Opening {
                target,
                source,
                value,
                archive_url,
                evidence,
                stake,
                accept,
            } => prints::print_opening(
                ctx,
                p,
                target,
                source,
                value.as_deref(),
                writes::Cite::of(archive_url, evidence),
                stake.as_deref(),
                *accept,
            ),
            PrintCommand::Challenge {
                target,
                source,
                value,
                archive_url,
                evidence,
                bond,
                challenge_id,
            } => prints::challenge(
                ctx,
                p,
                target,
                &prints::ChallengeRequest {
                    source,
                    value: value.as_deref(),
                    cited: writes::Cite::of(archive_url, evidence),
                    bond: bond.as_deref(),
                    challenge_id: challenge_id.as_deref(),
                },
            ),
            PrintCommand::Finalize { target, source } => prints::finalize(ctx, p, target, source),
            PrintCommand::Expire { target, source } => prints::expire(ctx, p, target, source),
            PrintCommand::FinalizePhase { target } => writes::print_finalize_phase(ctx, p, target),
        },
        O::Updates {
            command: None,
            list,
        } => updates::list(
            ctx,
            p,
            list.market.as_deref(),
            list.expiry.as_deref(),
            list.owner.as_deref(),
        ),
        O::Updates {
            command: Some(command),
            ..
        } => match command {
            UpdateCommand::Commit {
                target,
                source,
                value,
                archive_url,
                evidence,
                stake,
                reveal,
            } => updates::commit(
                ctx,
                p,
                target,
                &updates::CommitRequest {
                    source,
                    value: value.as_deref(),
                    cited: writes::Cite::of(archive_url, evidence),
                    stake: stake.as_deref(),
                    reveal: *reveal,
                },
            ),
            UpdateCommand::Reveal { claim, send } => updates::reveal(ctx, p, claim, send.simulate),
            UpdateCommand::Challenge {
                target,
                claim,
                value,
                archive_url,
                evidence,
                bond,
                challenge_id,
            } => updates::challenge(
                ctx,
                p,
                target,
                &updates::ChallengeRequest {
                    claim,
                    value: value.as_deref(),
                    cited: writes::Cite::of(archive_url, evidence),
                    bond: bond.as_deref(),
                    challenge_id: challenge_id.as_deref(),
                },
            ),
            UpdateCommand::Expire { target, claim } => updates::expire(ctx, p, target, claim),
        },
        O::Sponsor { candidate, send } => sponsor::sponsor(ctx, p, candidate, send),
        O::Rewards {
            command: None,
            market,
            expiry,
            owner,
        } => rewards::show(
            ctx,
            p,
            market.as_deref(),
            expiry.as_deref(),
            owner.as_deref(),
        ),
        O::Rewards {
            command: Some(RewardCommand::Claim { target, kind }),
            ..
        } => send::run(
            ctx,
            p,
            rewards::claim_write(ctx, target, *kind)?,
            target.send.simulate,
        ),
        O::Sponsorships { command: None } => sponsor::sponsorships(ctx, p),
        O::Sponsorships {
            command: Some(SponsorshipCommand::Settle { source, send: s }),
        } => send::run(ctx, p, sponsor::settle_write(ctx, source)?, s.simulate),
        O::Sponsorships {
            command: Some(SponsorshipCommand::Claim { source, send: s }),
        } => send::run(ctx, p, sponsor::claim_write(ctx, source)?, s.simulate),
        O::Reserve {
            command:
                ReserveCommand::Withdraw {
                    agent,
                    amount,
                    send: s,
                },
        } => send::run(
            ctx,
            p,
            sponsor::reserve_withdraw_write(ctx, agent, amount)?,
            s.simulate,
        ),
        O::Challenges {
            market,
            expiry,
            owner,
        } => challenges::list(
            ctx,
            p,
            market.as_deref(),
            expiry.as_deref(),
            owner.as_deref(),
        ),
        O::Stakes {
            command: None,
            market,
            expiry,
            owner,
            settled,
        } => stakes::list(
            ctx,
            p,
            market.as_deref(),
            expiry.as_deref(),
            owner.as_deref(),
            *settled,
        ),
        O::Stakes {
            command:
                Some(StakeCommand::Settle {
                    market,
                    expiry,
                    send,
                    all,
                    kind,
                    source,
                    owner,
                    claim,
                    challenge,
                }),
            ..
        } => {
            if *all {
                return stakes::settle_all(
                    ctx,
                    p,
                    market.as_deref(),
                    expiry.as_deref(),
                    owner.as_deref(),
                    send,
                );
            }
            let (Some(market), Some(kind), Some(source)) = (market, kind, source) else {
                return futures::error::bail(
                    "give the month, --kind and --source: petri oracle stakes settle RAMX-202612 --kind opening-claim --source <SOURCE> (or --all)",
                );
            };
            writes::stake_settle(
                ctx,
                p,
                &args::Target {
                    market: market.clone(),
                    expiry: expiry.clone(),
                    send: send.clone(),
                },
                *kind,
                source,
                owner.as_deref(),
                claim.as_deref(),
                challenge.as_deref(),
            )
        }
        O::Emergency { command } => council::run(ctx, p, command),
    }
}
