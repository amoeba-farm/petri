//! `petri oracle` grammar. Reads come from the futures API; writes are built
//! with the oracle program's instruction builders and signed by the attached
//! wallet.

use clap::{Args, Subcommand, ValueEnum};

#[derive(Debug, Subcommand)]
pub enum OracleCommand {
    #[command(about = "Oracle months: phase, sources and settlement status (one market or all)")]
    Months {
        #[arg(
            value_name = "MARKET",
            help = "ramx, nandx or a month such as RAMX-202610"
        )]
        market: Option<String>,
    },
    #[command(about = "The most recently updated oracle month")]
    Latest {
        #[arg(value_name = "MARKET")]
        market: Option<String>,
    },
    #[command(about = "Oracle months by last update, newest first")]
    History {
        #[arg(value_name = "MARKET")]
        market: Option<String>,
    },
    #[command(about = "Every oracle period with its settlement time")]
    Catalog,
    #[command(
        about = "Source inventory of a month, or propose, support and challenge sources",
        args_conflicts_with_subcommands = true
    )]
    Sources {
        #[command(subcommand)]
        command: Option<SourceCommand>,
        #[command(flatten)]
        list: ListArgs,
    },
    #[command(about = "Buckets of a month with their weights and sources")]
    Recipe(ListArgs),
    #[command(
        about = "Source candidates submitted by research agents (no oracle weight)",
        args_conflicts_with_subcommands = true
    )]
    Candidates {
        #[command(subcommand)]
        command: Option<CandidateCommand>,
        #[command(flatten)]
        filter: CandidateFilter,
    },
    #[command(
        about = "USDC bond collateral in the oracle: balance, deposit, withdraw",
        args_conflicts_with_subcommands = true
    )]
    Collateral {
        #[command(subcommand)]
        command: Option<CollateralCommand>,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Wallet to show; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(about = "Write, show or close sealed public evidence objects")]
    Evidence {
        #[command(subcommand)]
        command: EvidenceCommand,
    },
    #[command(about = "Opening prints: claim, challenge, finalize, expire, finalize the phase")]
    Prints {
        #[command(subcommand)]
        command: PrintCommand,
    },
    #[command(about = "Game updates: commit, reveal, challenge, expire")]
    Updates {
        #[command(subcommand)]
        command: UpdateCommand,
    },
    #[command(about = "Settle a finished bond or stake back into your oracle collateral")]
    Stakes {
        #[command(subcommand)]
        command: StakeCommand,
    },
    #[command(
        about = "Sponsor a research agent's candidate: your oracle collateral posts its bond, and the researcher shares its reward"
    )]
    Sponsor {
        #[arg(
            value_name = "CANDIDATE",
            help = "The candidate's content hash (petri oracle candidates)"
        )]
        candidate: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Your rewards: what you can claim, month by month",
        args_conflicts_with_subcommands = true
    )]
    Rewards {
        #[command(subcommand)]
        command: Option<RewardCommand>,
        #[arg(
            value_name = "MARKET",
            help = "ramx (its months), a month such as RAMX-202612; all months when omitted"
        )]
        market: Option<String>,
        #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
        expiry: Option<String>,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Wallet to show; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(about = "Your sponsorships: list them, settle them, claim their bounty")]
    Sponsorships {
        #[command(subcommand)]
        command: Option<SponsorshipCommand>,
    },
    #[command(about = "A researcher's reserve: withdraw what sponsored bounties held back")]
    Reserve {
        #[command(subcommand)]
        command: ReserveCommand,
    },
    #[command(about = "Propose, vote and apply council dispute decisions")]
    Emergency {
        #[command(subcommand)]
        command: CouncilCommand,
    },
}

#[derive(Debug, Args, Clone)]
pub struct ListArgs {
    #[arg(
        value_name = "MARKET",
        help = "ramx, nandx or a month such as RAMX-202610"
    )]
    pub market: Option<String>,
    #[arg(long, help = "Only this month: oct26, 2026-10 or RAMX-202610")]
    pub expiry: Option<String>,
    #[arg(long, help = "Match a bucket, source id or address, status or URL")]
    pub query: Option<String>,
    #[arg(long, default_value_t = 40, help = "Rows per month")]
    pub limit: usize,
}

#[derive(Debug, Args, Clone)]
pub struct CandidateFilter {
    #[arg(long, value_name = "MARKET")]
    pub market: Option<String>,
    #[arg(long, value_name = "MONTH", help = "Month id such as RAMX-202610")]
    pub expiry: Option<String>,
    #[arg(long, value_name = "AGENT_ID", help = "64-hex agent id")]
    pub agent: Option<String>,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}

#[derive(Debug, Subcommand)]
pub enum CandidateCommand {
    #[command(about = "Show one candidate by its content hash")]
    Show {
        #[arg(value_name = "HASH")]
        hash: String,
    },
}

/// The month a write targets, and whether to only simulate it.
#[derive(Debug, Args, Clone)]
pub struct Target {
    #[arg(
        value_name = "MARKET",
        help = "ramx, nandx, a month such as RAMX-202610, or a period address"
    )]
    pub market: String,
    #[arg(long, help = "Month for a bare symbol: oct26, 2026-10 or unix seconds")]
    pub expiry: Option<String>,
    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Debug, Args, Clone, Default)]
pub struct SendArgs {
    #[arg(
        long,
        help = "Simulate the transactions instead of signing and sending them"
    )]
    pub simulate: bool,
}

#[derive(Debug, Subcommand)]
pub enum CollateralCommand {
    #[command(about = "Move USDC from your token account into your oracle collateral")]
    Deposit {
        #[arg(value_name = "USDC")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Move available oracle collateral back to your token account (amount or `all`)"
    )]
    Withdraw {
        #[arg(value_name = "USDC")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum EvidenceKind {
    /// A public source URL (kind 1).
    Locator,
    /// A source definition (kind 2), committed with the oracle's definition domain.
    Definition,
    /// A Wayback archive URL (kind 3).
    Archive,
}

#[derive(Debug, Args, Clone)]
#[group(required = true, multiple = false)]
pub struct EvidenceBody {
    #[arg(long, value_name = "TEXT", help = "The URL or definition text")]
    pub text: Option<String>,
    #[arg(long, value_name = "PATH", help = "Read the text from a file")]
    pub file: Option<std::path::PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum EvidenceCommand {
    #[command(about = "Write and seal an evidence object")]
    Write {
        #[arg(long, value_enum)]
        kind: EvidenceKind,
        #[command(flatten)]
        body: EvidenceBody,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(about = "Show an evidence object: by address, or by kind and text")]
    Show {
        #[arg(value_name = "OBJECT", help = "Evidence object address")]
        object: Option<String>,
        #[arg(long, value_enum, requires = "text")]
        kind: Option<EvidenceKind>,
        #[arg(long, value_name = "TEXT")]
        text: Option<String>,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Payer that wrote it; defaults to the attached wallet"
        )]
        payer: Option<String>,
    },
    #[command(about = "Close an unsealed evidence draft and refund its rent")]
    Close {
        #[arg(value_name = "OBJECT")]
        object: String,
        #[command(flatten)]
        send: SendArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    #[command(about = "Propose a source with a listing bond (writes its evidence first)")]
    Propose {
        #[command(flatten)]
        target: Target,
        #[arg(
            long,
            value_name = "SKU",
            help = "Bucket: governed SKU label or 64-hex bucket id"
        )]
        bucket: String,
        #[arg(
            long,
            value_name = "URL",
            help = "Public HTTPS URL of the price source"
        )]
        url: String,
        #[arg(
            long,
            value_name = "TEXT",
            help = "Source type, for example `retailer listing`"
        )]
        source_type: String,
        #[arg(
            long,
            value_name = "TEXT",
            conflicts_with = "definition_file",
            help = "Source definition text"
        )]
        definition: Option<String>,
        #[arg(
            long,
            value_name = "PATH",
            help = "Read the source definition from a file"
        )]
        definition_file: Option<std::path::PathBuf>,
        #[arg(
            long,
            value_name = "USDC",
            help = "Listing bond, debited from your oracle collateral"
        )]
        bond: String,
        #[arg(
            long,
            value_name = "HEX",
            help = "Source id; derived from the month, bucket and URL when omitted"
        )]
        source_id: Option<String>,
    },
    #[command(
        about = "Close the month's source window once every product has a supported source: Scramble's challenge days follow"
    )]
    Close {
        #[command(flatten)]
        target: Target,
    },
    #[command(about = "Support a source with a stake")]
    Support {
        #[command(flatten)]
        target: Target,
        #[arg(
            long,
            value_name = "SOURCE",
            help = "64-hex source id or source address"
        )]
        source: String,
        #[arg(long, value_name = "USDC")]
        stake: String,
    },
    #[command(about = "Challenge a candidate source with a bond")]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "CODE",
            help = "Reason code (the program's challenge reasons)"
        )]
        reason: u8,
        #[arg(long, value_name = "USDC")]
        bond: String,
        #[arg(
            long,
            value_name = "HEX",
            help = "SHA-256 of the evidence behind the challenge"
        )]
        evidence_hash: String,
        #[arg(
            long,
            value_name = "SOURCE",
            help = "The other source, for a non-independence challenge"
        )]
        comparison: Option<String>,
        #[arg(long, value_name = "HEX", help = "Challenge id; random when omitted")]
        challenge_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PrintCommand {
    #[command(about = "Submit a bonded opening print with its Wayback archive")]
    Opening {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "VALUE",
            help = "Opening value of the source, in its own units"
        )]
        value: String,
        #[arg(
            long,
            value_name = "URL",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: String,
        #[arg(long, value_name = "USDC")]
        stake: String,
    },
    #[command(about = "Challenge an opening print with an alternative")]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(long, value_name = "VALUE")]
        value: String,
        #[arg(long, value_name = "URL")]
        archive_url: String,
        #[arg(long, value_name = "USDC")]
        bond: String,
        #[arg(long, value_name = "HEX")]
        challenge_id: Option<String>,
    },
    #[command(about = "Accept an unchallenged opening print after its window")]
    Finalize {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
    },
    #[command(about = "Mark an unclaimed frozen source inactive")]
    Expire {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
    },
    #[command(
        name = "finalize-phase",
        about = "Close the opening phase: the month enters Game and the period goes Live"
    )]
    FinalizePhase {
        #[command(flatten)]
        target: Target,
    },
}

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    #[command(about = "Commit a sealed source update with a stake; the reveal data stays private")]
    Commit {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "VALUE",
            help = "New value of the source, in its own units"
        )]
        value: String,
        #[arg(
            long,
            value_name = "URL",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: String,
        #[arg(long, value_name = "USDC")]
        stake: String,
    },
    #[command(about = "Reveal a committed update with its Wayback archive")]
    Reveal {
        #[arg(
            value_name = "CLAIM",
            help = "Update claim address printed by `updates commit`"
        )]
        claim: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(about = "Challenge a revealed update with an alternative")]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "CLAIM", help = "Update claim address")]
        claim: String,
        #[arg(long, value_name = "VALUE")]
        value: String,
        #[arg(long, value_name = "URL")]
        archive_url: String,
        #[arg(long, value_name = "USDC")]
        bond: String,
        #[arg(long, value_name = "HEX")]
        challenge_id: Option<String>,
    },
    #[command(about = "Refund a commitment nobody revealed, after its deadline")]
    Expire {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "CLAIM")]
        claim: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum EscrowKind {
    ListingBond,
    SupportStake,
    SourceChallenge,
    OpeningClaim,
    OpeningChallenge,
    UpdateClaim,
    UpdateChallenge,
}

#[derive(Debug, Subcommand)]
pub enum SponsorshipCommand {
    #[command(
        about = "Once its bond can settle: settle it and record the outcome (the bond returns to you)"
    )]
    Settle {
        #[arg(value_name = "SOURCE")]
        source: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(about = "Once the month's rewards are frozen: split its bounty with the researcher")]
    Claim {
        #[arg(value_name = "SOURCE")]
        source: String,
        #[command(flatten)]
        send: SendArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum RewardCommand {
    #[command(about = "Claim a month's frozen rewards into your oracle collateral")]
    Claim {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_enum, help = "Only this kind")]
        kind: Option<RewardKind>,
    },
}

/// Who a reward pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RewardKind {
    /// A source's proposer.
    Proposer,
    /// A source's supporter.
    Supporter,
    /// The claimant of an accepted opening value.
    Opening,
    /// The claimant of a finalized update.
    Update,
}

#[derive(Debug, Subcommand)]
pub enum ReserveCommand {
    #[command(about = "Move USDC from an agent's reserve to your oracle collateral")]
    Withdraw {
        #[arg(long, value_name = "AGENT_ID", help = "The research agent's 64-hex id")]
        agent: String,
        #[arg(value_name = "USDC")]
        amount: String,
        #[command(flatten)]
        send: SendArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum StakeCommand {
    #[command(about = "Settle one bond or stake back into oracle collateral")]
    Settle {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_enum)]
        kind: EscrowKind,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Proposer or supporter; defaults to the attached wallet"
        )]
        owner: Option<String>,
        #[arg(
            long,
            value_name = "CLAIM",
            help = "Update claim address (update kinds)"
        )]
        claim: Option<String>,
        #[arg(
            long,
            value_name = "ADDRESS",
            help = "Challenge address (challenge kinds)"
        )]
        challenge: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CouncilKind {
    Source,
    Opening,
    Update,
    Bucket,
}

#[derive(Clone, Debug, Args)]
pub struct CouncilTarget {
    #[command(flatten)]
    pub month: Target,
    #[arg(long, value_enum)]
    pub kind: CouncilKind,
    #[arg(long, help = "Disputed challenge or bucket account")]
    pub target: String,
    #[arg(long, help = "32-byte challenge or bucket ID in hex")]
    pub target_id: String,
    #[arg(
        long,
        help = "Target-specific account, in the oracle builder's order; prefix readonly accounts with ro:"
    )]
    pub account: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum CouncilCommand {
    #[command(about = "Open a case against the current immutable dispute evidence")]
    Propose {
        #[command(flatten)]
        target: CouncilTarget,
    },
    #[command(about = "Vote with the attached wallet's current council seat")]
    Vote {
        #[command(flatten)]
        target: CouncilTarget,
        #[arg(long)]
        choice: u8,
    },
    #[command(about = "Apply the current round's decision or its defined fallback")]
    Apply {
        #[command(flatten)]
        target: CouncilTarget,
    },
    #[command(about = "Apply an approved thin-bucket recovery decision")]
    Recover {
        #[command(flatten)]
        target: CouncilTarget,
    },
}
