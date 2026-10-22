//! Authenticated evidence: web proofs (TLSNotary) of what a source showed
//! you, including pages behind your own sign-in (`evidence/README.md`).
//!
//! - `browser`: your own browser with Petri's profile, to sign in yourself.
//! - `prove`: the proof, from this computer, with that session; the bundle
//!   goes to Petri's evidence folder (`store.rs`) with a preview.
//! - `show`, `verify`: the shared verifier (`amoeba_evidence`), every step
//!   but the oracle's.
//! - `import`: a bundle made elsewhere (a remote Petri needs no browser).
//! - `submit`: the API checks it again itself, keeps it for anyone to
//!   re-verify, and takes the candidate with it (`submit.rs`).
//!
//! The TUI's Evidence view calls the same functions.

pub mod args;
pub mod onchain;
pub mod store;
pub mod submit;
pub mod view;

use std::path::{Path, PathBuf};

use amoeba_evidence::{
    Verification,
    bundle::Bundle,
    hash::hex32,
    manifest::Manifest,
    policy::{DEFAULT_LIMITS, Policy},
    verify_bundle,
};
use amoeba_webproof::{browser, prover::ProveRequest};
use serde_json::{Value, json};

use crate::{
    backend::CliError,
    cli::Cli,
    futures::{
        self, Report,
        ctx::Ctx,
        error::{Error, Result},
        settings::Profile,
    },
};
use args::EvidenceCommand;

pub fn run(cli: &Cli, command: &EvidenceCommand) -> std::result::Result<Report, CliError> {
    let settings = futures::settings::Settings::from_cli(cli).map_err(CliError::new)?;
    let ctx = Ctx::new(settings);
    dispatch(&ctx, command).map_err(|e| {
        let message = e.message.replace('\n', " | ");
        let (code, category, retryable) =
            if message.contains("unreachable") || message.contains("could not be reached") {
                ("EVIDENCE_SERVICE_UNAVAILABLE", "unavailable", true)
            } else {
                ("EVIDENCE_INVALID", "validation", false)
            };
        CliError::coded(code, category, message, retryable)
    })
}

/// The verifier policy of this profile: production's, or the dev chain's
/// (its notary is development trust).
pub fn policy(ctx: &Ctx) -> Result<Policy> {
    policy_with(ctx, None, None)
}

/// The profile's policy plus test-only trust (`--dev-root`, `--dev-notary`),
/// which only the dev profile takes; its results say development trust.
pub fn policy_with(ctx: &Ctx, dev_root: Option<&Path>, dev_notary: Option<&str>) -> Result<Policy> {
    let mut p = match ctx.s.profile.0 {
        Profile::Production => Policy::production(),
        Profile::Dev => Policy::dev(),
    };
    if (dev_root.is_some() || dev_notary.is_some()) && ctx.s.profile.0 != Profile::Dev {
        return Err(Error::msg(
            "--dev-root and --dev-notary are for the dev profile only",
        ));
    }
    if let Some(path) = dev_root {
        p.development_roots.push(read_certificate(path)?);
    }
    if let Some(key) = dev_notary {
        let key =
            hex::decode(key.trim()).map_err(|_| Error::msg("--dev-notary is a hex public key"))?;
        p.notaries.push(amoeba_evidence::policy::Notary {
            name: "development notary (--dev-notary)".into(),
            alg: "secp256k1".into(),
            key,
            trust: amoeba_evidence::policy::Trust::Development,
        });
    }
    Ok(p)
}

/// A certificate file, PEM or DER.
fn read_certificate(path: &Path) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let bytes = std::fs::read(path)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display())))?;
    let text = String::from_utf8_lossy(&bytes);
    if let Some(start) = text.find("-----BEGIN CERTIFICATE-----") {
        let body = &text[start + 27..];
        let end = body
            .find("-----END CERTIFICATE-----")
            .ok_or_else(|| Error::msg(format!("{} is not a PEM certificate", path.display())))?;
        let b64: String = body[..end].chars().filter(|c| !c.is_whitespace()).collect();
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| Error::msg(format!("{} is not a PEM certificate", path.display())));
    }
    Ok(bytes)
}

/// The notary to prove through: `--notary`, else `PETRI_NOTARY_URL`, else
/// the futures API's (`/v1/config` → `evidence.webProof.notary`, a path on
/// the API, reached as a WebSocket).
pub fn notary_url(ctx: &Ctx, flag: Option<&str>) -> Result<String> {
    if let Some(u) = flag.map(str::to_string).or_else(|| {
        std::env::var("PETRI_NOTARY_URL")
            .ok()
            .filter(|v| !v.is_empty())
    }) {
        return Ok(u);
    }
    let config = ctx.api.get("/v1/config")?;
    let path = config["evidence"]["webProof"]["notary"]
        .as_str()
        .ok_or_else(|| {
            Error::msg(format!(
                "the futures API at {} offers no notary yet; pass --notary",
                ctx.api.base()
            ))
        })?;
    Ok(join_ws(ctx.api.base(), path))
}

fn join_ws(base: &str, path: &str) -> String {
    if path.starts_with("ws://") || path.starts_with("wss://") || path.starts_with("tcp://") {
        return path.to_string();
    }
    let base = base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}{path}")
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::msg(format!("cannot start the prover: {e}")))
}

/// Petri's own browser profile for signing in to sources.
pub fn browser_profile() -> Result<PathBuf> {
    Ok(store::dir()?.join("browser-profile"))
}

/// Opens the sign-in browser on `url`; returns its DevTools address once
/// the browser has written it (it does so within moments of starting).
pub fn open_browser(url: Option<&str>) -> Result<Option<String>> {
    let exe = browser::find().ok_or_else(|| {
        Error::msg("no Chrome, Edge, Brave or Chromium found; install one, or pass --cdp to `prove` for a browser you started yourself")
    })?;
    let profile = browser_profile()?;
    crate::petri_config::create_private_dir_all(&profile).map_err(|e| Error::msg(e.to_string()))?;
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    browser::open(&exe, &profile, url.unwrap_or("about:blank"))
        .map_err(|e| Error::msg(format!("cannot open {}: {e}", exe.display())))?;
    // Only for the message: the address appears once the browser is up.
    for _ in 0..50 {
        if let Some(addr) = browser::devtools_of(&profile) {
            return Ok(Some(addr));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(None)
}

pub struct Proved {
    pub entry: Option<store::Entry>,
    pub verification: Verification,
    pub status: u16,
    pub timings: amoeba_webproof::prover::Timings,
    pub cookie_names: Vec<String>,
}

pub struct ProveArgs<'a> {
    pub url: &'a str,
    pub manifest: &'a [u8],
    pub browser: bool,
    pub cdp: Option<&'a str>,
    pub headers: &'a [String],
    pub note: Option<&'a str>,
    pub out: Option<&'a Path>,
    pub notary: Option<&'a str>,
    pub no_bind: bool,
    pub max_response: Option<usize>,
    pub connect_to: Option<&'a str>,
    pub dev_root: Option<&'a Path>,
    pub dev_notary: Option<&'a str>,
}

/// Proves one page and keeps the bundle when it reads a record.
pub fn prove(ctx: &Ctx, a: &ProveArgs) -> Result<Proved> {
    Manifest::parse(a.manifest).map_err(|e| Error::msg(e.to_string()))?;
    // A proof shows the server's name (its certificate), not the port the
    // page came from, and an oracle locator has no port.
    let target =
        amoeba_webproof::prover::Target::parse(a.url).map_err(|e| Error::msg(e.to_string()))?;
    if target.port != 443 && ctx.s.profile.0 != Profile::Dev {
        return Err(Error::msg(format!(
            "{} uses port {}: a proof shows the server's name but not its port, so prove the page on https's usual port (443)",
            a.url, target.port
        )));
    }
    let mut headers = Vec::new();
    for h in a.headers {
        let (k, v) = h
            .split_once(':')
            .ok_or_else(|| Error::msg(format!("`{h}` is not NAME: VALUE")))?;
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }
    let rt = runtime()?;
    let devtools = match (a.browser, a.cdp) {
        (_, Some(addr)) => Some(addr.to_string()),
        (true, None) => Some(browser::devtools_of(&browser_profile()?).ok_or_else(|| {
            Error::msg("the sign-in browser is not open: run `petri evidence browser <url>` and sign in first")
        })?),
        (false, None) => None,
    };
    let mut cookie_names = Vec::new();
    if let Some(dt) = devtools {
        let signed = rt
            .block_on(browser::signed_in(&dt, a.url))
            .map_err(|e| Error::msg(format!("could not read the browser's session: {e}")))?;
        cookie_names = signed.cookie_names.clone();
        headers.extend(signed.headers());
    }
    let binding = if a.no_bind {
        None
    } else {
        ctx.wallet_pubkey().ok().map(|p| p.to_bytes())
    };
    let mut req = ProveRequest::new(a.url);
    req.headers = headers;
    req.binding = binding;
    req.connect_to = a.connect_to.map(str::to_string);
    if let Some(root) = a.dev_root {
        req.extra_roots.push(read_certificate(root)?);
    }
    if let Some(max) = a.max_response {
        if max > DEFAULT_LIMITS.max_recv {
            return Err(Error::msg(format!(
                "--max-response is at most {} bytes (what verifiers accept)",
                DEFAULT_LIMITS.max_recv
            )));
        }
        req.max_recv = max;
    }
    let notary = notary_url(ctx, a.notary)?;
    let notes = a
        .note
        .map(|n| vec![("note".to_string(), n.to_string())])
        .unwrap_or_default();
    let (bundle, timings, status) = rt
        .block_on(amoeba_webproof::prove(&notary, &req, a.manifest, notes))
        .map_err(|e| Error::msg(e.to_string()))?;
    let json = bundle.to_json();
    let verification = verify_bundle(
        json.as_bytes(),
        &policy_with(ctx, a.dev_root, a.dev_notary)?,
    );
    let entry = if verification.authenticated() {
        Some(keep_verified(&json, &verification, "proved", a.out)?)
    } else {
        None
    };
    Ok(Proved {
        entry,
        verification,
        status,
        timings,
        cookie_names,
    })
}

/// Keeps a bundle whose verification ran (any method), under its digest.
fn keep_verified(
    json: &str,
    v: &Verification,
    origin: &str,
    out: Option<&Path>,
) -> Result<store::Entry> {
    let env = v.envelope.as_ref();
    let entry = store::Entry {
        digest: v
            .bundle_digest
            .clone()
            .ok_or_else(|| Error::msg("the bundle is unreadable"))?,
        file: PathBuf::new(),
        origin: origin.into(),
        added_at: store::now(),
        locator: env.map(|e| e.origin.locator.clone()).unwrap_or_default(),
        price: env
            .map(|e| {
                let cur = e
                    .observation
                    .fields
                    .get("currency")
                    .map(|f| f.value.as_str())
                    .unwrap_or("");
                format!("{} {cur}", e.observation.price.text)
                    .trim()
                    .to_string()
            })
            .unwrap_or_default(),
        product: env
            .and_then(|e| e.observation.fields.get("product"))
            .map(|f| f.value.clone())
            .unwrap_or_default(),
        events: vec![],
    };
    store::save_in(&store::dir()?, entry, json, out)
}

/// Keeps a verified bundle made elsewhere in Petri's evidence folder.
pub fn import_text(text: &str, v: &Verification) -> Result<store::Entry> {
    keep_verified(text, v, "imported", None)
}

/// The DKIM key for `selector._domainkey.domain` over DNS-over-HTTPS, as
/// this computer sees it now: development trust, and said so.
fn dns_key(domain: &str, selector: &str) -> Result<amoeba_evidence::email::PinnedKey> {
    let name = format!("{selector}._domainkey.{domain}");
    let http = reqwest::blocking::Client::new();
    let reply: Value = http
        .get("https://cloudflare-dns.com/dns-query")
        .query(&[("name", name.as_str()), ("type", "TXT")])
        .header("accept", "application/dns-json")
        .send()
        .and_then(|r| r.json())
        .map_err(|e| Error::msg(format!("DNS lookup of {name} failed: {e}")))?;
    let record = reply["Answer"]
        .as_array()
        .and_then(|a| a.iter().find_map(|r| r["data"].as_str()))
        .map(|d| {
            // TXT data comes as quoted strings, maybe several.
            d.split('"')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, s)| s)
                .collect::<String>()
        })
        .filter(|r| !r.is_empty())
        .ok_or_else(|| Error::msg(format!("{name} publishes no DKIM key")))?;
    Ok(amoeba_evidence::email::PinnedKey {
        domain: domain.to_ascii_lowercase(),
        selector: selector.to_ascii_lowercase(),
        record,
        trust: amoeba_evidence::policy::Trust::Development,
        provenance: format!(
            "DNS TXT resolved over DNS-over-HTTPS (cloudflare-dns.com) by this computer at {}",
            store::now()
        ),
    })
}

fn email(
    ctx: &Ctx,
    message: &Path,
    manifest: &Path,
    disclose: bool,
    keys: EmailKeys,
    out: Option<&Path>,
) -> Result<Report> {
    use amoeba_evidence::{bundle::EmailBundle, email::EmailManifest};
    let bytes = std::fs::read(message)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", message.display())))?;
    let m_bytes = std::fs::read(manifest)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", manifest.display())))?;
    let m = EmailManifest::parse(&m_bytes).map_err(|e| Error::msg(e.to_string()))?;
    let policy = email_policy(ctx, &bytes, &m, keys)?;
    let bundle = EmailBundle {
        message: bytes,
        manifest: m,
        annotations: vec![],
    };
    let json = bundle.to_json();
    let v = verify_bundle(json.as_bytes(), &policy);
    let entry = if disclose && v.authenticated() {
        Some(keep_verified(&json, &v, "email", out)?)
    } else {
        None
    };
    let mut plain = view::plain(&view::preview(&v, entry.as_ref()));
    plain.push_str(match (&entry, disclose) {
        (Some(_), _) => "\n  Saved. It discloses the whole message to whoever checks it.\n",
        (None, false) if v.authenticated() => {
            "\n  Nothing was written: the bundle would disclose the whole message. Add --disclose to write it.\n"
        }
        _ => "\n  Nothing was written.\n",
    });
    Ok(Report::new(
        json!({ "bundle": entry.map(|e| e.file), "verification": v }),
        plain,
    ))
}

/// Where an email command may find the signer's key beyond the policy's
/// pinned ones.
#[derive(Clone, Copy)]
struct EmailKeys<'a> {
    /// DNS-over-HTTPS from this computer (development trust).
    dns: bool,
    /// A test key record from a file (dev profile only).
    dev_key: Option<&'a Path>,
}

/// The verifier policy for an email, with the signer's key from DNS or a
/// test file added (development trust) when asked.
fn email_policy(
    ctx: &Ctx,
    message: &[u8],
    m: &amoeba_evidence::email::EmailManifest,
    keys: EmailKeys,
) -> Result<amoeba_evidence::policy::Policy> {
    let mut policy = policy(ctx)?;
    if keys.dev_key.is_some() && ctx.s.profile.0 != Profile::Dev {
        return Err(Error::msg("--dev-dkim-key is for the dev profile only"));
    }
    if !keys.dns && keys.dev_key.is_none() {
        return Ok(policy);
    }
    let msg =
        amoeba_evidence::dkim::parse_message(message).map_err(|e| Error::msg(e.to_string()))?;
    for sig in amoeba_evidence::dkim::signatures(&msg)
        .into_iter()
        .flatten()
    {
        if sig.domain != m.signing_domain {
            continue;
        }
        if let Some(path) = keys.dev_key {
            let record = std::fs::read_to_string(path)
                .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display())))?;
            policy.dkim_keys.push(amoeba_evidence::email::PinnedKey {
                domain: sig.domain.clone(),
                selector: sig.selector.clone(),
                record: record.trim().to_string(),
                trust: amoeba_evidence::policy::Trust::Development,
                provenance: "a test key given with --dev-dkim-key".into(),
            });
        } else {
            policy.dkim_keys.push(dns_key(&sig.domain, &sig.selector)?);
        }
    }
    Ok(policy)
}

/// `petri evidence email --zk`: the labelled lines, proven on this computer
/// without the rest of the message. The message reaches only the circuit's
/// working folder (its inputs are removed after proving) and the two local
/// programs; the bundle holds the proof and the claims the preview lists.
fn email_zk(
    ctx: &Ctx,
    message: &Path,
    manifest: &Path,
    nargo: Option<PathBuf>,
    bb: Option<PathBuf>,
    keys: EmailKeys,
    out: Option<&Path>,
) -> Result<Report> {
    use amoeba_evidence::{bundle::EmailZkBundle, email::EmailManifest, email_zk};
    let bytes = std::fs::read(message)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", message.display())))?;
    let m_bytes = std::fs::read(manifest)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", manifest.display())))?;
    let m = EmailManifest::parse(&m_bytes).map_err(|e| Error::msg(e.to_string()))?;
    let policy = email_policy(ctx, &bytes, &m, keys)?;
    // The message's own checks first, so a refusal says why in plain words
    // before a minute of proving.
    let prepared = email_zk::prepare(&bytes, &m, &policy.dkim_keys)
        .map_err(|e| Error::msg(format!("This message cannot be proven: {e}.")))?;
    let tools = email_zk::Tools {
        nargo: nargo.unwrap_or_else(|| "nargo".into()),
        bb: bb.unwrap_or_else(|| "bb".into()),
    };
    tools.check().map_err(|e| {
        Error::msg(format!(
            "{e}. The zero-knowledge proof runs nargo {} and bb {} on this computer (noirup -v {}; bbup -v {}).",
            email_zk::NARGO_VERSION,
            email_zk::BB_VERSION,
            email_zk::NARGO_VERSION,
            email_zk::BB_VERSION
        ))
    })?;
    // Petri's own private folder: the circuit's inputs and witness hold the
    // message while they exist.
    let work = store::dir()?.join("circuits").join(email_zk::CIRCUIT_NAME);
    crate::petri_config::create_private_dir_all(&work).map_err(|e| Error::msg(e.to_string()))?;
    let started = std::time::Instant::now();
    let proof = email_zk::prove(&prepared, &work, &tools).map_err(Error::msg)?;
    let seconds = started.elapsed().as_secs_f64();
    let bundle = EmailZkBundle {
        circuit: email_zk::circuit_id(),
        proof,
        claims: prepared.claims,
        manifest: m,
        annotations: vec![],
    };
    let json = bundle.to_json();
    let v = verify_bundle(json.as_bytes(), &policy);
    let entry = if v.authenticated() {
        Some(keep_verified(&json, &v, "email-zk", out)?)
    } else {
        None
    };
    let mut plain = view::plain(&view::preview(&v, entry.as_ref()));
    plain.push_str(&match &entry {
        Some(_) => format!(
            "\n  Saved. Proven in {seconds:.0} s; it discloses only what is listed above.\n"
        ),
        None => "\n  Nothing was written.\n".to_string(),
    });
    Ok(Report::new(
        json!({ "bundle": entry.map(|e| e.file), "seconds": seconds, "verification": v }),
        plain,
    ))
}

pub fn load(arg: &str) -> Result<(PathBuf, String)> {
    store::resolve_in(&store::dir()?, arg)
}

pub fn entry_of(digest: &str) -> Option<store::Entry> {
    store::load_index(&store::dir().ok()?)
        .ok()?
        .get(digest)
        .cloned()
}

fn dispatch(ctx: &Ctx, command: &EvidenceCommand) -> Result<Report> {
    use EvidenceCommand as E;
    match command {
        E::Browser { url } => {
            let addr = open_browser(url.as_deref())?;
            let plain = format!(
                "Opened your browser with Petri's own profile{}.\nSign in there yourself; Petri never asks for your password.\nThen: petri evidence prove <url> --manifest <file> --browser\n{}",
                url.as_deref()
                    .map(|u| format!(" on {u}"))
                    .unwrap_or_default(),
                addr.as_deref()
                    .map(|a| format!("DevTools for browser tools (this computer only): {a}"))
                    .unwrap_or_default()
            );
            Ok(Report::new(
                json!({ "opened": true, "url": url, "devtools": addr, "profile": browser_profile()? }),
                plain,
            ))
        }
        E::Prove {
            url,
            manifest,
            browser,
            cdp,
            headers,
            note,
            out,
            notary,
            no_bind,
            max_response,
            connect_to,
            dev_root,
            dev_notary,
        } => {
            let manifest_bytes = std::fs::read(manifest)
                .map_err(|e| Error::msg(format!("cannot read {}: {e}", manifest.display())))?;
            let p = prove(
                ctx,
                &ProveArgs {
                    url,
                    manifest: &manifest_bytes,
                    browser: *browser,
                    cdp: cdp.as_deref(),
                    headers,
                    note: note.as_deref(),
                    out: out.as_deref(),
                    notary: notary.as_deref(),
                    no_bind: *no_bind,
                    max_response: *max_response,
                    connect_to: connect_to.as_deref(),
                    dev_root: dev_root.as_deref(),
                    dev_notary: dev_notary.as_deref(),
                },
            )?;
            let preview = view::preview(&p.verification, p.entry.as_ref());
            let mut plain = view::plain(&preview);
            match &p.entry {
                Some(e) => plain.push_str(&format!(
                    "\n  Saved {}\n  Next: petri evidence submit {} --candidate <candidate.json>\n",
                    e.file.display(),
                    &e.digest[..16]
                )),
                None if p.status == 401 || p.status == 403 => plain.push_str(
                    "\n  The source refused the request: sign in in the browser (`petri evidence browser <url>`) and prove again.\n",
                ),
                None if !p.verification.stages.provenance.ok() => {
                    plain.push_str("\n  Nothing was saved: the proof does not verify here.\n")
                }
                None => plain.push_str("\n  Nothing was saved: the proof shows no record the manifest can read.\n"),
            }
            Ok(Report::new(
                json!({
                    "bundle": p.entry.as_ref().map(|e| e.file.clone()),
                    "bundleDigest": p.verification.bundle_digest,
                    "status": p.status,
                    "cookiesUsed": p.cookie_names,
                    "timings": p.timings,
                    "verification": p.verification,
                }),
                plain,
            ))
        }
        E::Email {
            message,
            manifest,
            disclose,
            zk,
            nargo,
            bb,
            dns,
            out,
            dev_dkim_key,
        } => {
            let keys = EmailKeys {
                dns: *dns,
                dev_key: dev_dkim_key.as_deref(),
            };
            if *zk {
                email_zk(
                    ctx,
                    message,
                    manifest,
                    nargo.clone(),
                    bb.clone(),
                    keys,
                    out.as_deref(),
                )
            } else {
                email(ctx, message, manifest, *disclose, keys, out.as_deref())
            }
        }
        E::CheckManifest {
            manifest,
            body,
            url,
        } => check_manifest(manifest, body, url.as_deref()),
        E::Show {
            bundle,
            dev_root,
            dev_notary,
        }
        | E::Verify {
            bundle,
            dev_root,
            dev_notary,
        } => {
            let (path, text) = load(bundle)?;
            let v = verify_bundle(
                text.as_bytes(),
                &policy_with(ctx, dev_root.as_deref(), dev_notary.as_deref())?,
            );
            let entry = v.bundle_digest.as_deref().and_then(entry_of);
            if matches!(command, E::Verify { .. }) && !v.authenticated() {
                let reason = [
                    &v.stages.received,
                    &v.stages.provenance,
                    &v.stages.extraction,
                ]
                .iter()
                .find_map(|s| s.reason.clone())
                .unwrap_or_else(|| "it does not verify".into());
                return Err(Error::msg(format!(
                    "{} does not verify: {reason}",
                    path.display()
                )));
            }
            Ok(Report::new(
                json!({ "bundle": path, "verification": v, "events": entry.map(|e| e.events) }),
                view::plain(&view::preview(
                    &v,
                    entry_of(v.bundle_digest.as_deref().unwrap_or("")).as_ref(),
                )),
            ))
        }
        E::Import { file } => {
            let text = std::fs::read_to_string(file)
                .map_err(|e| Error::msg(format!("cannot read {}: {e}", file.display())))?;
            let v = verify_bundle(text.as_bytes(), &policy(ctx)?);
            if !v.authenticated() {
                let reason = [
                    &v.stages.received,
                    &v.stages.provenance,
                    &v.stages.extraction,
                ]
                .iter()
                .find_map(|s| s.reason.clone())
                .unwrap_or_default();
                return Err(Error::msg(format!("not imported: {reason}")));
            }
            // Web proofs only: an email bundle is refused here.
            Bundle::parse(text.as_bytes(), DEFAULT_LIMITS.max_bundle_bytes)
                .map_err(|e| Error::msg(e.to_string()))?;
            let entry = keep_verified(&text, &v, "imported", None)?;
            let mut plain = view::plain(&view::preview(&v, Some(&entry)));
            plain.push_str(&format!("\n  Imported as {}\n", entry.file.display()));
            Ok(Report::new(
                json!({ "entry": entry, "verification": v }),
                plain,
            ))
        }
        E::List => {
            let index = store::load_index(&store::dir()?)?;
            let mut entries: Vec<&store::Entry> = index.values().collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.added_at));
            let mut plain = String::new();
            if entries.is_empty() {
                plain.push_str("No proofs yet. Make one with `petri evidence prove`.\n");
            }
            for e in &entries {
                let last = e
                    .events
                    .last()
                    .map(|ev| ev.summary.as_str())
                    .unwrap_or("not submitted");
                plain.push_str(&format!(
                    "{}  {:<14} {:<40} {}\n",
                    &e.digest[..12],
                    e.price,
                    e.locator.trim_start_matches("https://"),
                    last
                ));
            }
            Ok(Report::new(json!({ "proofs": entries }), plain))
        }
        E::Submit { bundle, candidate } => {
            let (_, text) = load(bundle)?;
            let v = verify_bundle(text.as_bytes(), &policy(ctx)?);
            let digest = v
                .bundle_digest
                .clone()
                .ok_or_else(|| Error::msg("the bundle is unreadable"))?;
            if !v.authenticated() {
                return Err(Error::msg(
                    "this proof does not verify here; see `petri evidence show`",
                ));
            }
            if entry_of(&digest).is_none() {
                // Web proofs only: an email bundle is refused here.
                Bundle::parse(text.as_bytes(), DEFAULT_LIMITS.max_bundle_bytes)
                    .map_err(|e| Error::msg(e.to_string()))?;
                keep_verified(&text, &v, "imported", None)?;
            }
            let dir = store::dir()?;
            let uploaded = submit::upload(ctx, &dir, &digest, &text)?;
            let mut plain = format!(
                "Uploaded {}: the API verified it and keeps it for anyone to re-check.\n",
                &digest[..16]
            );
            let mut payload = json!({ "bundleDigest": digest, "upload": uploaded });
            if let Some(file) = candidate {
                let reply = submit::candidate(ctx, &dir, &digest, file)?;
                plain.push_str(if reply["duplicate"].as_bool() == Some(true) {
                    "Candidate: this observation was already submitted for this slot; the first submission keeps the credit.\n"
                } else {
                    "Candidate submitted with the proof. Sponsors see it in the Fund view; the oracle decides on chain.\n"
                });
                payload["candidate"] = reply;
            }
            Ok(Report::new(payload, plain))
        }
    }
}

/// Applies a manifest to a saved response body, as the verifier would to
/// a proven one.
fn check_manifest(manifest: &Path, body: &Path, url: Option<&str>) -> Result<Report> {
    use amoeba_evidence::{extract, http};
    let m_bytes = std::fs::read(manifest)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", manifest.display())))?;
    let m = Manifest::parse(&m_bytes).map_err(|e| Error::msg(e.to_string()))?;
    let body = std::fs::read(body)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", body.display())))?;
    let target = url
        .and_then(|u| u.strip_prefix("https://"))
        .and_then(|r| r.find('/').map(|i| r[i..].to_string()))
        .unwrap_or_else(|| m.path.clone());
    let host = url
        .and_then(|u| u.strip_prefix("https://"))
        .map(|r| r.split(['/', '?']).next().unwrap_or("").to_string())
        .unwrap_or_else(|| m.origin.clone());
    let request = http::Request {
        method: "GET".into(),
        target,
        host,
        headers: vec![],
    };
    let response = http::Response {
        status: 200,
        headers: vec![],
        content_type: Some("application/json".into()),
        body,
    };
    let x = extract::extract(&m, &request, &response)
        .map_err(|e| Error::msg(format!("the manifest does not read this body: {e}")))?;
    let mut plain = format!(
        "The manifest reads {} → price {} ({} atoms)\n",
        x.record_pointer, x.price.value, x.price_atoms
    );
    for (k, f) in &x.fields {
        plain.push_str(&format!("  {k:<12} {}\n", f.value));
    }
    plain.push_str(&format!(
        "  selector {}\n  manifest {}\n",
        hex32(&m.selector_digest()),
        hex32(&m.manifest_hash())
    ));
    let fields: Value = x
        .fields
        .iter()
        .map(|(k, f)| (k.clone(), json!(f.value)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    Ok(Report::new(
        json!({
            "record": x.record_pointer,
            "price": x.price.value,
            "priceAtoms": x.price_atoms,
            "fields": fields,
            "context": x.context,
            "selectorDigest": hex32(&m.selector_digest()),
            "manifestHash": hex32(&m.manifest_hash()),
        }),
        plain,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notary_addresses() {
        assert_eq!(
            join_ws("http://127.0.0.1:4500", "/oracle/notary"),
            "ws://127.0.0.1:4500/oracle/notary"
        );
        assert_eq!(
            join_ws("https://api.amoeba.farm/", "/oracle/notary"),
            "wss://api.amoeba.farm/oracle/notary"
        );
        assert_eq!(
            join_ws("https://x", "tcp://127.0.0.1:4520"),
            "tcp://127.0.0.1:4520"
        );
    }
}
