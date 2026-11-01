//! `petri oracle heads` grammar: oracle v2's board ("heads on pikes",
//! `oracle/docs/heads.md`). The DeepSeek Harness plugin (`plugins/petri/lib/oracle.mjs`,
//! `HEADS`) builds exactly these argv; keep the two in step.

use clap::{Args, Subcommand, ValueEnum};

use crate::oracle::args::SendArgs;

/// `petri oracle heads [--product P] [COMMAND]`: the boards, or one command.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HeadsArgs {
    #[command(subcommand)]
    pub command: Option<HeadsCommand>,
    #[arg(
        long,
        value_name = "PRODUCT",
        help = "Only this product's board: RAMX, NANDX or BATX"
    )]
    pub product: Option<String>,
}

/// A head on a product's board.
#[derive(Debug, Args, Clone)]
pub struct HeadRef {
    #[arg(
        value_name = "HEAD",
        help = "The head's id on its board (petri oracle heads lists them)"
    )]
    pub head: u32,
    #[arg(
        long,
        value_name = "PRODUCT",
        help = "The product whose board it is on: RAMX, NANDX or BATX"
    )]
    pub product: String,
}

/// One claim of a head.
#[derive(Debug, Args, Clone)]
pub struct ClaimRef {
    #[arg(value_name = "HEAD", help = "The head's id on its board")]
    pub head: u32,
    #[arg(
        value_name = "SEQ",
        help = "The claim's number on its head (petri oracle heads show lists them)"
    )]
    pub seq: u32,
    #[arg(
        long,
        value_name = "PRODUCT",
        help = "The product whose board it is on: RAMX, NANDX or BATX"
    )]
    pub product: String,
}

#[derive(Debug, Subcommand)]
pub enum HeadsCommand {
    #[command(about = "One head: its spec, claims, challenges and delegations")]
    Show(HeadRef),
    #[command(
        about = "Your part: free stake, delegations (active, cooling, in escrow), claims, challenges and rewards, with what to run next"
    )]
    Mine {
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Another wallet; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(
        about = "A contract's listing terms: the frozen sentences, mix and settlement target it settles by"
    )]
    Terms {
        #[arg(
            value_name = "PERIOD|QUARTER|PRODUCT",
            help = "A period address, a quarter such as RAMX-Q1-2027, or a product (its front contract)"
        )]
        target: String,
    },
    #[command(about = "Stake AMBA from your wallet into the oracle; staked AMBA backs heads")]
    Stake {
        #[arg(value_name = "AMBA", help = "How much, e.g. 1500")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Unstake free AMBA back to your wallet (or `all`); delegated, cooling and escrowed AMBA stays"
    )]
    Unstake {
        #[arg(value_name = "AMBA")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Post a head: a source for one component, by name and spec (not a URL), backed at once with your free stake"
    )]
    Post(PostArgs),
    #[command(about = "Delegate free stake to a head: its weight in settlement, and its backing")]
    Delegate {
        #[command(flatten)]
        head: HeadRef,
        #[arg(value_name = "AMBA")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Undelegate from a head (or `all`): it cools, and what the remaining strikes could take stays in escrow"
    )]
    Undelegate {
        #[command(flatten)]
        head: HeadRef,
        #[arg(value_name = "AMBA")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Bring a delegation up to date: strikes, rewards, and what the cooldown and escrow now release to your free stake"
    )]
    Sync {
        #[command(flatten)]
        head: HeadRef,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Whose delegation (anyone may sync it); defaults to yours"
        )]
        owner: Option<String>,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Fill an active head: commit and reveal a claim with its bond; the first fill that stands makes it live"
    )]
    Fill(ClaimArgs),
    #[command(about = "Update a live source: commit and reveal a claim with its bond (anyone may)")]
    Update(ClaimArgs),
    #[command(
        about = "Reveal a committed claim whose reveal did not land (from Petri's private file)"
    )]
    Reveal {
        #[command(flatten)]
        claim: ClaimRef,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(about = "Challenge a claim while it is open to challenge, with a bond and a reason")]
    Challenge(ChallengeArgs),
    #[command(
        about = "Challenge a head itself (it does not exist, its spec can't be filled, it publishes made-up numbers)"
    )]
    ChallengeSource(SourceChallengeArgs),
    #[command(
        about = "Finish what is due on a claim (or, without SEQ, on the head): finalize it once its window ended, settle its challenges, refund an unrevealed commit, expire an open head"
    )]
    Settle {
        #[arg(value_name = "HEAD")]
        head: u32,
        #[arg(
            value_name = "SEQ",
            help = "The claim; without it, the head's own source case"
        )]
        seq: Option<u32>,
        #[arg(long, value_name = "PRODUCT")]
        product: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(about = "Your delegator rewards; --claim moves them into your oracle collateral")]
    Rewards {
        #[arg(long, help = "Claim every delegation's rewards")]
        claim: bool,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Another wallet to show; defaults to the attached wallet"
        )]
        owner: Option<String>,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Claim an accepted claim's fill or update prize once its calendar month has ended"
    )]
    Prize {
        #[command(flatten)]
        claim: ClaimRef,
        #[command(flatten)]
        send: SendArgs,
    },
}

/// How a fetcher reaches a head's number (the spec's access class).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Access {
    PublicPage,
    #[value(name = "login-zktls")]
    LoginZktls,
    OfflineQuote,
    Physical,
}

/// The ground a challenge names: labels only (same bond, resolution and slash).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ReasonCode {
    /// The value or the evidence is wrong, or another ground.
    #[default]
    Value,
    /// The purchase belongs to another channel, or to none.
    OffChannel,
    /// The evidence traces to an earlier live source's origin, undeclared.
    DuplicateOrigin,
    /// An LTA price carried past its own next reset.
    ExpiredObservation,
}

#[derive(Debug, Args, Clone)]
pub struct PostArgs {
    #[arg(long, value_name = "PRODUCT", help = "RAMX, NANDX or BATX")]
    pub product: String,
    #[arg(
        long,
        value_name = "COMPONENT",
        help = "The component: its index, or its label such as \"DDR5 16Gb Contract\" (petri oracle heads lists them)"
    )]
    pub component: String,
    #[arg(
        long,
        value_name = "NAME",
        help = "A short name people recognize (at most 64 bytes)"
    )]
    pub name: String,
    #[arg(
        long,
        value_name = "TEXT",
        help = "What a valid claim must contain: unit, cadence, where it is published; the component's sentence when omitted"
    )]
    pub spec: Option<String>,
    #[arg(
        long,
        value_enum,
        value_name = "ACCESS",
        help = "How a fetcher reaches the number"
    )]
    pub access: Access,
    #[arg(
        long,
        value_name = "AMBA",
        help = "What to delegate on posting; the board's minimum to post when omitted"
    )]
    pub amount: Option<String>,
    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Debug, Args, Clone)]
pub struct ClaimArgs {
    #[arg(value_name = "HEAD")]
    pub head: u32,
    #[arg(long, value_name = "PRODUCT")]
    pub product: String,
    #[arg(
        long,
        value_name = "VALUE",
        help = "The number the head's spec asks for, in its unit (six decimals); a web proof's own when omitted"
    )]
    pub value: Option<String>,
    #[arg(
        long,
        value_name = "TIME",
        help = "When the source published it: unix seconds or a UTC time (2026-10-28T09:00:00Z); the evidence's capture time when omitted"
    )]
    pub time: Option<String>,
    #[arg(
        long,
        value_name = "PUBLISHER/REPORT",
        help = "Who first published the price and in which report (\"TrendForce/DRAM contract price table\"), or its 64-hex origin id"
    )]
    pub origin: String,
    #[command(flatten)]
    pub evidence: EvidenceArg,
    #[command(flatten)]
    pub lta: LtaArgs,
    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Debug, Args, Clone, Default)]
pub struct EvidenceArg {
    #[arg(
        long,
        value_name = "FILE|URL",
        help = "A Wayback capture (https://web.archive.org/web/<14-digit UTC time>/<url>), a web proof (bundle file or digest), or a file such as an offline quote (at most 4,096 bytes)"
    )]
    pub evidence: Option<String>,
}

/// The terms an LTA-channel claim declares (only on LTA heads).
#[derive(Debug, Args, Clone, Default)]
pub struct LtaArgs {
    #[arg(long, value_name = "TIME", help = "LTA: when the price took effect")]
    pub effective_from: Option<String>,
    #[arg(
        long,
        value_name = "TIME",
        help = "LTA: the agreement's next price reset"
    )]
    pub next_reset: Option<String>,
    #[arg(long, value_name = "VALUE", help = "LTA: the agreement's price floor")]
    pub floor: Option<String>,
    #[arg(
        long,
        value_name = "VALUE",
        help = "LTA: the agreement's price ceiling"
    )]
    pub ceiling: Option<String>,
}

impl LtaArgs {
    pub fn is_empty(&self) -> bool {
        self.effective_from.is_none()
            && self.next_reset.is_none()
            && self.floor.is_none()
            && self.ceiling.is_none()
    }
}

#[derive(Debug, Args, Clone)]
pub struct ChallengeArgs {
    #[command(flatten)]
    pub claim: ClaimRef,
    #[arg(long, value_name = "VALUE", help = "The right value, when you know it")]
    pub value: Option<String>,
    #[arg(
        long,
        value_name = "TIME",
        help = "When the right value was published; the evidence's capture time, else the claim's own time"
    )]
    pub time: Option<String>,
    #[command(flatten)]
    pub evidence: EvidenceArg,
    #[arg(
        long,
        value_name = "TEXT",
        help = "What is wrong and what shows it, in plain words; public and permanent"
    )]
    pub reason: String,
    #[arg(long, value_enum, default_value_t = ReasonCode::Value, help = "The ground")]
    pub code: ReasonCode,
    #[arg(
        long,
        value_name = "USDC",
        help = "Bond; the board's challenge bond when omitted"
    )]
    pub bond: Option<String>,
    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Debug, Args, Clone)]
pub struct SourceChallengeArgs {
    #[command(flatten)]
    pub head: HeadRef,
    #[command(flatten)]
    pub evidence: EvidenceArg,
    #[arg(
        long,
        value_name = "TEXT",
        help = "What is wrong with the source, in plain words; public and permanent"
    )]
    pub reason: String,
    #[arg(long, value_enum, default_value_t = ReasonCode::Value, help = "The ground")]
    pub code: ReasonCode,
    #[arg(
        long,
        value_name = "USDC",
        help = "Bond; the board's challenge bond when omitted"
    )]
    pub bond: Option<String>,
    #[command(flatten)]
    pub send: SendArgs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Command, oracle::args::OracleCommand};
    use clap::Parser;

    fn heads(args: &[&str]) -> HeadsArgs {
        let mut argv = vec!["petri", "oracle", "heads"];
        argv.extend_from_slice(args);
        match crate::cli::Cli::try_parse_from(argv).unwrap().command {
            Some(Command::Oracle {
                command: Some(OracleCommand::Heads(h)),
            }) => h,
            other => panic!("not an oracle heads command: {other:?}"),
        }
    }

    fn command(args: &[&str]) -> HeadsCommand {
        heads(args).command.expect("a heads subcommand")
    }

    fn refused(args: &[&str]) -> bool {
        let mut argv = vec!["petri", "oracle", "heads"];
        argv.extend_from_slice(args);
        crate::cli::Cli::try_parse_from(argv).is_err()
    }

    /// Every argv `plugins/petri/lib/oracle.mjs` builds (its tests' cases) parses.
    #[test]
    fn the_plugins_argv_parse() {
        let h = heads(&[]);
        assert!(h.command.is_none() && h.product.is_none());
        assert_eq!(
            heads(&["--product", "NANDX"]).product.as_deref(),
            Some("NANDX")
        );
        assert!(
            matches!(command(&["show", "4", "--product", "RAMX"]), HeadsCommand::Show(HeadRef { head: 4, ref product }) if product == "RAMX")
        );
        assert!(matches!(
            command(&["mine"]),
            HeadsCommand::Mine { owner: None }
        ));
        assert!(
            matches!(command(&["stake", "1500", "--yes"]), HeadsCommand::Stake { ref amount, .. } if amount == "1500")
        );
        assert!(
            matches!(command(&["unstake", "250.5"]), HeadsCommand::Unstake { ref amount, .. } if amount == "250.5")
        );
        match command(&[
            "post",
            "--product",
            "RAMX",
            "--component",
            "DDR4_8GBIT",
            "--name",
            "TrendForce DDR4 8Gb contract",
            "--spec",
            "USD per chip, monthly contract price, TrendForce DRAM table",
            "--access",
            "login-zktls",
        ]) {
            HeadsCommand::Post(p) => {
                assert_eq!(
                    (p.product.as_str(), p.component.as_str()),
                    ("RAMX", "DDR4_8GBIT")
                );
                assert_eq!(p.access, Access::LoginZktls);
                assert!(p.amount.is_none() && p.spec.is_some());
            }
            other => panic!("{other:?}"),
        }
        match command(&[
            "post",
            "--product",
            "BATX",
            "--component",
            "0",
            "--name",
            "LFP cell spot",
            "--access",
            "public-page",
            "--amount",
            "1200",
        ]) {
            HeadsCommand::Post(p) => {
                assert_eq!((p.spec, p.amount.as_deref()), (None, Some("1200")))
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(command(&["delegate", "3", "500", "--product", "RAMX"]),
            HeadsCommand::Delegate { head: HeadRef { head: 3, .. }, ref amount, .. } if amount == "500")
        );
        assert!(matches!(
            command(&["undelegate", "2", "1000", "--product", "RAMX"]),
            HeadsCommand::Undelegate { .. }
        ));
        assert!(matches!(
            command(&["sync", "2", "--product", "RAMX"]),
            HeadsCommand::Sync { owner: None, .. }
        ));
        match command(&[
            "fill",
            "4",
            "--product",
            "RAMX",
            "--value",
            "2.31",
            "--time",
            "1793174400",
            "--origin",
            "Kingston/quote desk",
            "--evidence",
            "/home/me/quotes/kingston.json",
        ]) {
            HeadsCommand::Fill(c) => {
                assert_eq!(
                    (c.head, c.value.as_deref(), c.time.as_deref()),
                    (4, Some("2.31"), Some("1793174400"))
                );
                assert_eq!(
                    c.evidence.evidence.as_deref(),
                    Some("/home/me/quotes/kingston.json")
                );
                assert!(c.lta.is_empty());
            }
            other => panic!("{other:?}"),
        }
        match command(&[
            "update",
            "0",
            "--product",
            "RAMX",
            "--value",
            "4.85",
            "--time",
            "1793178000",
            "--origin",
            "TrendForce/DRAM contract",
            "--effective-from",
            "1793000000",
            "--next-reset",
            "1795000000",
            "--floor",
            "4",
            "--ceiling",
            "6",
        ]) {
            HeadsCommand::Update(c) => assert_eq!(c.lta.next_reset.as_deref(), Some("1795000000")),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            command(&["reveal", "4", "2", "--product", "RAMX"]),
            HeadsCommand::Reveal {
                claim: ClaimRef {
                    head: 4,
                    seq: 2,
                    ..
                },
                ..
            }
        ));
        match command(&[
            "challenge",
            "4",
            "0",
            "--product",
            "RAMX",
            "--value",
            "2.05",
            "--evidence",
            "https://web.archive.org/web/20261028080000/https://x.test/q",
            "--reason",
            "The quote is for DDR4 4Gb, not 8Gb.",
        ]) {
            HeadsCommand::Challenge(c) => {
                assert_eq!(
                    (c.claim.head, c.claim.seq, c.code),
                    (4, 0, ReasonCode::Value)
                );
                assert!(c.bond.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            command(&[
                "challenge",
                "4",
                "0",
                "--product",
                "RAMX",
                "--reason",
                "Stale.",
                "--code",
                "expired-observation"
            ]),
            HeadsCommand::Challenge(ChallengeArgs {
                code: ReasonCode::ExpiredObservation,
                ..
            })
        ));
        assert!(matches!(
            command(&[
                "challenge-source",
                "1",
                "--product",
                "RAMX",
                "--reason",
                "A reseller mirror.",
                "--code",
                "duplicate-origin"
            ]),
            HeadsCommand::ChallengeSource(SourceChallengeArgs {
                code: ReasonCode::DuplicateOrigin,
                ..
            })
        ));
        assert!(matches!(
            command(&["settle", "4", "0", "--product", "RAMX"]),
            HeadsCommand::Settle {
                head: 4,
                seq: Some(0),
                ..
            }
        ));
        assert!(matches!(
            command(&["settle", "4", "--product", "RAMX"]),
            HeadsCommand::Settle { seq: None, .. }
        ));
        assert!(matches!(
            command(&["rewards", "--claim"]),
            HeadsCommand::Rewards { claim: true, .. }
        ));
        assert!(matches!(
            command(&["rewards"]),
            HeadsCommand::Rewards { claim: false, .. }
        ));
        assert!(matches!(
            command(&["prize", "0", "3", "--product", "RAMX"]),
            HeadsCommand::Prize {
                claim: ClaimRef {
                    head: 0,
                    seq: 3,
                    ..
                },
                ..
            }
        ));
        assert!(
            matches!(command(&["terms", "RAMX"]), HeadsCommand::Terms { ref target } if target == "RAMX")
        );
        // `--simulate` on every write.
        assert!(matches!(
            command(&["delegate", "3", "500", "--product", "RAMX", "--simulate"]),
            HeadsCommand::Delegate {
                send: SendArgs { simulate: true },
                ..
            }
        ));
    }

    #[test]
    fn malformed_heads_commands_are_refused() {
        assert!(refused(&["show", "4"]), "a head needs its product");
        assert!(
            refused(&["show", "x", "--product", "RAMX"]),
            "a head id is a number"
        );
        assert!(refused(&["delegate", "-1", "5", "--product", "RAMX"]));
        assert!(
            refused(&["fetcher", "stake", "0", "--product", "RAMX"]),
            "no fetcher seats: anyone fills and updates"
        );
        assert!(refused(&[
            "post",
            "--product",
            "RAMX",
            "--component",
            "0",
            "--name",
            "n",
            "--access",
            "website"
        ]));
        assert!(
            refused(&["fill", "4", "--product", "RAMX", "--value", "2"]),
            "a claim declares its origin"
        );
        assert!(
            refused(&["challenge", "4", "0", "--product", "RAMX"]),
            "a challenge says why"
        );
        assert!(refused(&[
            "challenge",
            "4",
            "0",
            "--product",
            "RAMX",
            "--reason",
            "r",
            "--code",
            "stale"
        ]));
        assert!(
            refused(&["--product", "RAMX", "mine"]),
            "the board's product flag is not a subcommand's"
        );
        assert!(
            refused(&["prize", "0", "--product", "RAMX"]),
            "a prize names its claim"
        );
    }
}
