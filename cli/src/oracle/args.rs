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
        about = "Comparison series of the chip months: a template, check one, propose one, a month's series"
    )]
    Series {
        #[command(subcommand)]
        command: SeriesCommand,
    },
    #[command(
        about = "Observe a chip month's series: its opening print in Opening, an update in Game, with the observation's details",
        args_conflicts_with_subcommands = true
    )]
    Observe {
        #[command(subcommand)]
        command: Option<ObserveCommand>,
        #[command(flatten)]
        send: ObserveArgs,
    },
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
    #[command(
        about = "Opening prints: yours and how each stands, the prints open to challenge and the sources waiting for one; or submit, challenge, accept, set aside",
        args_conflicts_with_subcommands = true
    )]
    Prints {
        #[command(subcommand)]
        command: Option<PrintCommand>,
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
            help = "Whose prints and challenges; defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(
        about = "Game updates: yours and how each stands, the updates open to challenge and the sources taking one now; commit, reveal, challenge, expire",
        args_conflicts_with_subcommands = true
    )]
    Updates {
        #[command(subcommand)]
        command: Option<UpdateCommand>,
        #[command(flatten)]
        list: UpdateListArgs,
    },
    #[command(
        about = "Source challenges: yours and how each stands or ended, and the sources open to challenge now"
    )]
    Challenges {
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
            help = "Whose challenges (as challenger or the source's proposer); defaults to the attached wallet"
        )]
        owner: Option<String>,
    },
    #[command(
        about = "Your bonds and stakes: what each waits for or where it went; settle one, or --all that pay you",
        args_conflicts_with_subcommands = true
    )]
    Stakes {
        #[command(subcommand)]
        command: Option<StakeCommand>,
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
            help = "Whose bonds and stakes; defaults to the attached wallet"
        )]
        owner: Option<String>,
        #[arg(
            long,
            help = "Also every settled one still on chain (settled updates, and months that have settled)"
        )]
        settled: bool,
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

#[derive(Debug, Subcommand)]
pub enum SeriesCommand {
    #[command(about = "A blank comparison series of a bucket, every field of the spec present")]
    Template {
        #[arg(
            long,
            value_name = "BUCKET",
            help = "Commodity bucket: DDR5_16GBIT, tlc_1tb, or its name (\"DDR5 16Gb\")"
        )]
        bucket: String,
        #[arg(long, help = "A direct component price, not a published assessment")]
        direct: bool,
        #[arg(
            long,
            value_name = "PATH",
            help = "Write it to a file instead of stdout"
        )]
        out: Option<std::path::PathBuf>,
    },
    #[command(
        about = "Check a series against the spec: on its own, or against a month's sources on chain"
    )]
    Check {
        #[arg(value_name = "FILE", help = "The series JSON")]
        file: std::path::PathBuf,
        #[arg(
            value_name = "MONTH",
            help = "A month such as RAMX-202612 (or ramx with --expiry)"
        )]
        market: Option<String>,
        #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
        expiry: Option<String>,
    },
    #[command(
        about = "Propose a series as a source of a chip month, with its listing bond (only one the spec admits)"
    )]
    Propose {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "PATH", help = "The series JSON")]
        file: std::path::PathBuf,
        #[arg(
            long,
            value_name = "USDC",
            help = "Listing bond; the product's exact bond when omitted"
        )]
        bond: Option<String>,
        #[arg(
            long,
            value_name = "HEX",
            help = "Source id; derived from the month, bucket and page when omitted"
        )]
        source_id: Option<String>,
    },
    #[command(about = "A chip month's sources: bucket, series and whether each counts")]
    Show {
        #[arg(value_name = "MONTH")]
        market: String,
        #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
        expiry: Option<String>,
    },
}

/// `petri oracle updates [MARKET] [--expiry E] [--owner PUBKEY]`.
#[derive(Debug, Args, Clone)]
pub struct UpdateListArgs {
    #[arg(
        value_name = "MARKET",
        help = "ramx (its months), a month such as RAMX-202610; all months when omitted"
    )]
    pub market: Option<String>,
    #[arg(long, help = "Month for a bare symbol: oct26, 2026-10 or unix seconds")]
    pub expiry: Option<String>,
    #[arg(
        long,
        value_name = "PUBKEY",
        help = "Whose updates (as claimant or challenger); defaults to the attached wallet"
    )]
    pub owner: Option<String>,
}

/// `petri oracle observe MONTH --source ID --file OBS.json [--stake USDC] [--reveal]`.
#[derive(Debug, Args, Clone)]
pub struct ObserveArgs {
    #[arg(
        value_name = "MONTH",
        help = "A month such as RAMX-202612 (or ramx with --expiry)"
    )]
    pub market: Option<String>,
    #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
    pub expiry: Option<String>,
    #[arg(
        long,
        value_name = "SOURCE",
        help = "64-hex source id or source address"
    )]
    pub source: Option<String>,
    #[arg(long, value_name = "PATH", help = "The observation JSON")]
    pub file: Option<std::path::PathBuf>,
    #[arg(
        long,
        value_name = "USDC",
        help = "Stake; the product's exact opening bond or update stake when omitted"
    )]
    pub stake: Option<String>,
    #[arg(
        long,
        help = "In Game, reveal the update as soon as its commit lands, in one step (the reveal window is 150 slots, well under a minute); an opening print has nothing to reveal, so there it changes nothing"
    )]
    pub reveal: bool,
    #[arg(
        long,
        help = "In Opening, wait out the print's 300 slots (a minute or two) and accept it, unless someone challenges it first; a Game update has nothing to accept here, so there it changes nothing"
    )]
    pub accept: bool,
    #[command(flatten)]
    pub send: SendArgs,
}

#[derive(Debug, Subcommand)]
pub enum ObserveCommand {
    #[command(about = "A blank observation of a source's series")]
    Template {
        #[arg(value_name = "MONTH")]
        market: String,
        #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
        expiry: Option<String>,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "PATH",
            help = "Write it to a file instead of stdout"
        )]
        out: Option<std::path::PathBuf>,
    },
    #[command(
        about = "Store the observation record of a print already on chain with the API (a retry)"
    )]
    Record {
        #[arg(value_name = "MONTH")]
        market: String,
        #[arg(long, help = "Month for a bare symbol: dec26, 2026-12 or unix seconds")]
        expiry: Option<String>,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "CLAIM",
            help = "The opening claim, or the revealed update claim"
        )]
        claim: String,
        #[arg(
            long,
            value_name = "PATH",
            help = "The observation JSON; an update `observe` committed keeps its own"
        )]
        file: Option<std::path::PathBuf>,
    },
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
    /// An authenticated evidence record (kind 4): a web proof's
    /// `amoeba-evidence-v1;…` text (`petri evidence`).
    Record,
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
    #[command(
        about = "Support someone's candidate source with the product's support bond (checked before anything is sent)"
    )]
    Support {
        #[command(flatten)]
        target: Target,
        #[arg(
            long,
            value_name = "SOURCE",
            help = "64-hex source id or source address"
        )]
        source: String,
        #[arg(
            long,
            value_name = "USDC",
            help = "Stake, debited from your oracle collateral; the product's exact support bond when omitted"
        )]
        stake: Option<String>,
    },
    #[command(
        about = "Challenge a candidate source with a bond, on Scramble days 5 and 6 (checked before anything is sent)",
        group(
            clap::ArgGroup::new("evidence")
                .required(true)
                .args(["why", "why_file", "evidence_hash"])
        )
    )]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(
            long,
            value_name = "SOURCE",
            help = "64-hex source id or source address"
        )]
        source: String,
        #[arg(
            long,
            value_name = "REASON",
            value_parser = super::challenges::parse_reason,
            help = super::challenges::REASON_HELP
        )]
        reason: u8,
        #[arg(
            long,
            value_name = "USDC",
            help = "Bond, debited from your oracle collateral; the exact bond the program requires when omitted"
        )]
        bond: Option<String>,
        #[arg(
            long,
            value_name = "TEXT",
            help = "Why the source is wrong: petri seals it as a public evidence object and commits the challenge to it, so whoever rules reads it (`petri oracle evidence show`). Public and permanent"
        )]
        why: Option<String>,
        #[arg(
            long,
            value_name = "PATH",
            help = "Read the --why text from a file (public and permanent)"
        )]
        why_file: Option<std::path::PathBuf>,
        #[arg(
            long,
            value_name = "HEX",
            help = "SHA-256 commitment of evidence you published yourself, instead of --why"
        )]
        evidence_hash: Option<String>,
        #[arg(
            long,
            value_name = "SOURCE",
            help = "The other source, for a `duplicate` challenge: a candidate of the same bucket"
        )]
        comparison: Option<String>,
        #[arg(long, value_name = "HEX", help = "Challenge id; random when omitted")]
        challenge_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PrintCommand {
    #[command(
        about = "Submit a bonded opening print with its Wayback archive (checked before anything is sent)"
    )]
    Opening {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "VALUE",
            required_unless_present = "evidence",
            help = "Opening value of the source, in its own units [default with --evidence: the proven price]"
        )]
        value: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            required_unless_present = "evidence",
            conflicts_with = "evidence",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: Option<String>,
        #[arg(
            long,
            value_name = "BUNDLE",
            help = "A web proof of the source's page instead of an archive (petri evidence prove)"
        )]
        evidence: Option<String>,
        #[arg(
            long,
            value_name = "USDC",
            help = "Stake, debited from your oracle collateral; the product's exact opening bond when omitted"
        )]
        stake: Option<String>,
        #[arg(
            long,
            help = "Wait out its 300 slots (a minute or two) and accept it, unless someone challenges it first"
        )]
        accept: bool,
    },
    #[command(
        about = "Challenge someone's opening print with an alternative, inside its 300 slots (checked before anything is sent)"
    )]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "VALUE",
            required_unless_present = "evidence",
            help = "The source's value as you read it, in its own units [default with --evidence: the proven price]"
        )]
        value: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            required_unless_present = "evidence",
            conflicts_with = "evidence",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: Option<String>,
        #[arg(
            long,
            value_name = "BUNDLE",
            help = "A web proof of the source's page instead of an archive (petri evidence prove)"
        )]
        evidence: Option<String>,
        #[arg(
            long,
            value_name = "USDC",
            help = "Bond, debited from your oracle collateral; the exact bond the program requires when omitted"
        )]
        bond: Option<String>,
        #[arg(long, value_name = "HEX", help = "Challenge id; random when omitted")]
        challenge_id: Option<String>,
    },
    #[command(
        about = "Accept an unchallenged opening print once its 300 slots passed, before listing"
    )]
    Finalize {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
    },
    #[command(about = "Set aside a frozen source left without an opening print, from listing")]
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
    #[command(
        about = "Commit a sealed source update with the product's exact stake (checked before anything is sent); the reveal data stays private, and --reveal reveals it at once"
    )]
    Commit {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "SOURCE")]
        source: String,
        #[arg(
            long,
            value_name = "VALUE",
            required_unless_present = "evidence",
            help = "New value of the source, in its own units [default with --evidence: the proven price]"
        )]
        value: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            required_unless_present = "evidence",
            conflicts_with = "evidence",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: Option<String>,
        #[arg(
            long,
            value_name = "BUNDLE",
            help = "A web proof of the source's page instead of an archive (petri evidence prove); uploaded when you reveal"
        )]
        evidence: Option<String>,
        #[arg(
            long,
            value_name = "USDC",
            help = "Stake, debited from your oracle collateral; the product's exact update stake when omitted"
        )]
        stake: Option<String>,
        #[arg(
            long,
            help = "Reveal it as soon as the commit lands, in one step (the reveal window is 150 slots, well under a minute)"
        )]
        reveal: bool,
    },
    #[command(
        about = "Reveal a committed update with its Wayback archive, within 150 slots of the commit"
    )]
    Reveal {
        #[arg(
            value_name = "CLAIM",
            help = "Update claim address printed by `updates commit`"
        )]
        claim: String,
        #[command(flatten)]
        send: SendArgs,
    },
    #[command(
        about = "Challenge a revealed update with another value, until it is accepted (checked before anything is sent)"
    )]
    Challenge {
        #[command(flatten)]
        target: Target,
        #[arg(long, value_name = "CLAIM", help = "Update claim address")]
        claim: String,
        #[arg(
            long,
            value_name = "VALUE",
            required_unless_present = "evidence",
            help = "The source's value, in its own units [default with --evidence: the proven price]"
        )]
        value: Option<String>,
        #[arg(
            long,
            value_name = "URL",
            required_unless_present = "evidence",
            conflicts_with = "evidence",
            help = "https://web.archive.org/web/<14-digit UTC time>/<source URL>"
        )]
        archive_url: Option<String>,
        #[arg(
            long,
            value_name = "BUNDLE",
            help = "A web proof of the source's page instead of an archive (petri evidence prove)"
        )]
        evidence: Option<String>,
        #[arg(
            long,
            value_name = "USDC",
            help = "Bond, debited from your oracle collateral; the exact bond the program requires when omitted"
        )]
        bond: Option<String>,
        #[arg(long, value_name = "HEX", help = "Challenge id; random when omitted")]
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
    #[command(
        about = "Settle one bond or stake back into oracle collateral, or --all of the owner's that pay them now"
    )]
    Settle {
        #[arg(
            value_name = "MARKET",
            help = "ramx, nandx, a month such as RAMX-202610, or a period address (with --all: all months when omitted)"
        )]
        market: Option<String>,
        #[arg(long, help = "Month for a bare symbol: oct26, 2026-10 or unix seconds")]
        expiry: Option<String>,
        #[command(flatten)]
        send: SendArgs,
        #[arg(
            long,
            conflicts_with_all = ["kind", "source", "claim", "challenge"],
            help = "Every one of the owner's that settles to them now (petri oracle stakes lists them), a few to a transaction"
        )]
        all: bool,
        #[arg(long, value_enum, required_unless_present = "all")]
        kind: Option<EscrowKind>,
        #[arg(long, value_name = "SOURCE", required_unless_present = "all")]
        source: Option<String>,
        #[arg(
            long,
            value_name = "PUBKEY",
            help = "Proposer or supporter (with --all: whose); defaults to the attached wallet"
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
