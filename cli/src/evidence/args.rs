//! `petri evidence …`: web proofs of what a source showed you, including
//! pages behind your own sign-in.

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum EvidenceCommand {
    #[command(
        about = "Open your browser to sign in to a source",
        long_about = "Opens your own Chrome, Edge, Brave or Chromium with Petri's own profile, on URL. Sign in there yourself: Petri never asks for a password. `petri evidence prove --browser` then uses that browser's session for the one page it proves. An agent with browser tools can drive the same window through the DevTools address this prints (on this computer only). Close the window when you are done."
    )]
    Browser {
        #[arg(value_name = "URL", help = "The page to open")]
        url: Option<String>,
    },
    #[command(
        about = "Prove what a page shows, as a web proof",
        long_about = "Connects to the source from this computer, with your signed-in session, through a notary (TLSNotary, MPC mode): the notary helps run the TLS connection so the page can't be forged, without seeing it. Writes a proof bundle and shows exactly what it discloses: the request line and Host, the names of the other headers (never their values: your cookies stay hidden) and the whole response. Nothing is submitted."
    )]
    Prove {
        #[arg(
            value_name = "URL",
            help = "The https address of the JSON the page shows"
        )]
        url: String,
        #[arg(
            long,
            value_name = "FILE",
            help = "Extraction manifest: which record and fields to read"
        )]
        manifest: PathBuf,
        #[arg(
            long,
            help = "Use the session of the browser `petri evidence browser` opened"
        )]
        browser: bool,
        #[arg(
            long,
            value_name = "ADDR",
            conflicts_with = "browser",
            help = "Use another browser's session through its DevTools address (http://127.0.0.1:PORT)"
        )]
        cdp: Option<String>,
        #[arg(
            long = "header",
            value_name = "NAME: VALUE",
            help = "Send this header too (its value stays hidden in the proof)"
        )]
        headers: Vec<String>,
        #[arg(
            long,
            value_name = "TEXT",
            help = "Your note, kept with the bundle (not verified)"
        )]
        note: Option<String>,
        #[arg(
            long,
            value_name = "FILE",
            help = "Where to write the bundle [default: Petri's evidence folder]"
        )]
        out: Option<PathBuf>,
        #[arg(
            long,
            value_name = "URL",
            help = "The notary [default: the futures API's]"
        )]
        notary: Option<String>,
        #[arg(long, help = "Don't bind the proof to your wallet")]
        no_bind: bool,
        #[arg(
            long,
            value_name = "BYTES",
            help = "Largest response to prove, up to 65536 [default: 16384]"
        )]
        max_response: Option<usize>,
        /// Test only: connect here instead of the URL's host.
        #[arg(long, hide = true, value_name = "HOST:PORT")]
        connect_to: Option<String>,
        /// Test only: trust this extra root certificate (PEM or DER) for the
        /// source; results say "development root".
        #[arg(long, hide = true, value_name = "FILE")]
        dev_root: Option<PathBuf>,
        /// Test only: also trust this notary key (hex); results say
        /// "development trust". Dev profile only.
        #[arg(long, hide = true, value_name = "KEY")]
        dev_notary: Option<String>,
    },
    #[command(
        about = "A DKIM-signed quote from a saved email, as evidence",
        long_about = "Checks the email's DKIM signature and reads the quote's labelled lines (an email manifest: signingDomain and fields like \"price\": \"Unit price:\") from its signed text. On its own this method discloses the whole message to whoever checks the bundle, so nothing is written without --disclose. With --zk, Petri proves the labelled lines on this computer with a zero-knowledge proof instead: the bundle discloses the signing domain and selector, the signature time, the signed Content-Type, whether the From address is in the signing domain, and the labelled lines, nothing else (about a minute; needs nargo 1.0.0-beta.18 and bb 3.0.3). A quote stays a quotation: it is not a source under the evidence rules."
    )]
    Email {
        #[arg(value_name = "FILE", help = "The message, as saved (.eml)")]
        message: PathBuf,
        #[arg(
            long,
            value_name = "FILE",
            help = "Email manifest: the signing domain and the line labels"
        )]
        manifest: PathBuf,
        #[arg(
            long,
            conflicts_with = "zk",
            help = "Write the bundle, which discloses the whole message"
        )]
        disclose: bool,
        #[arg(
            long,
            help = "Prove only the labelled lines (zero-knowledge) and write that bundle"
        )]
        zk: bool,
        #[arg(
            long,
            value_name = "PATH",
            requires = "zk",
            help = "The nargo to run [default: nargo on PATH]"
        )]
        nargo: Option<PathBuf>,
        #[arg(
            long,
            value_name = "PATH",
            requires = "zk",
            help = "The bb to run [default: bb on PATH]"
        )]
        bb: Option<PathBuf>,
        #[arg(
            long,
            help = "Look the signer's key up over DNS-over-HTTPS (cloudflare-dns.com) from this computer; results say development trust"
        )]
        dns: bool,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Test only: accept this DKIM key record (a file holding the TXT
        /// value) for the manifest's domain, as development trust. Dev
        /// profile only.
        #[arg(long, hide = true, value_name = "FILE")]
        dev_dkim_key: Option<PathBuf>,
    },
    #[command(about = "Try a manifest on a saved response before proving")]
    CheckManifest {
        #[arg(value_name = "FILE")]
        manifest: PathBuf,
        #[arg(long, value_name = "FILE", help = "A saved JSON response of the page")]
        body: PathBuf,
        #[arg(long, value_name = "URL", help = "The address it came from")]
        url: Option<String>,
    },
    #[command(about = "What a proof discloses and what it shows")]
    Show {
        #[arg(
            value_name = "BUNDLE",
            help = "A bundle file, or a digest in Petri's evidence folder"
        )]
        bundle: String,
        /// Test only: also trust this root (PEM or DER).
        #[arg(long, hide = true, value_name = "FILE")]
        dev_root: Option<PathBuf>,
        /// Test only: also trust this notary key (hex); results say
        /// "development trust". Dev profile only.
        #[arg(long, hide = true, value_name = "KEY")]
        dev_notary: Option<String>,
    },
    #[command(about = "Check a proof here, every step but the oracle's")]
    Verify {
        #[arg(value_name = "BUNDLE")]
        bundle: String,
        #[arg(long, hide = true, value_name = "FILE")]
        dev_root: Option<PathBuf>,
        /// Test only: also trust this notary key (hex); results say
        /// "development trust". Dev profile only.
        #[arg(long, hide = true, value_name = "KEY")]
        dev_notary: Option<String>,
    },
    #[command(about = "Add a bundle made on another computer")]
    Import {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    #[command(about = "Your proofs, with each step and what the oracle did")]
    List,
    #[command(
        about = "Upload a proof, and submit it with a source candidate",
        long_about = "Uploads the bundle to the futures API, which checks it itself. With --candidate, also submits that source candidate (the research workspace's candidate JSON: marketId, expiryId, bucketId, canonicalUrl, sourceType, productDefinition, priceField, region, currency, evidence; or a find's comparison series with marketId, expiryId and evidence, the API deriving the rest) with the proof attached, as you: rewards go to your wallet. The proof must be for the candidate's canonicalUrl."
    )]
    Submit {
        #[arg(value_name = "BUNDLE")]
        bundle: String,
        #[arg(
            long,
            value_name = "FILE",
            help = "The source candidate to submit with it"
        )]
        candidate: Option<PathBuf>,
    },
}
