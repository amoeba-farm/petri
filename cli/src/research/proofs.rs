//! The Evidence view's own proofs: web proofs you make or import here, by
//! hand (`petri evidence`, crate::evidence, does the work). Sign in to a
//! site in your own browser, prove a page, see exactly what it shows and
//! discloses, and submit it as a source candidate. The agent's finds stay
//! below (render.rs). Proving and submitting run on a worker thread, as the
//! Fund view's writes do; nothing is said while they run.

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};
use serde_json::Value;

use super::render::{ACCENT, ALERT, BORDER, DIM, MUTED, PANEL, TEXT, scrim};
use crate::evidence::{self, store::Entry, view};
use crate::futures::{ctx::Ctx, settings::Settings};

const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(122, 178, 132);

/// Where a proof can go: a product a month takes sources for, or (with
/// `update`) a Game month's source whose registered page the proof is of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub market: String,
    pub expiry: String,
    pub bucket: String,
    /// "RAMX · Dec 2026 · DDR5 16Gb".
    pub title: String,
    pub update: Option<UpdateTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateTarget {
    /// The source's address.
    pub source: String,
    /// The product's minimum update stake, USDC atoms.
    pub stake: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Open the sign-in browser: one field, the site's address.
    SignIn,
    /// Prove a page: its JSON address and the manifest file.
    Prove,
    /// Import a bundle file.
    Import,
    /// One of your proofs (its digest).
    Show(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Edit,
    Working,
    /// A proof's preview; its digest when it can be submitted.
    Preview(Vec<Line<'static>>, Option<String>),
    /// Where to submit it: the months taking sources, by product.
    Choose(String, Vec<Slot>),
    /// The candidate it becomes, before it is sent.
    Review(String, Slot, Vec<String>),
    Done(Vec<String>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub kind: Kind,
    pub fields: Vec<String>,
    pub focus: usize,
    pub stage: Stage,
    pub choice: usize,
}

enum Command {
    Refresh,
    SignIn(String),
    Prove(String, PathBuf),
    Import(PathBuf),
    Show(String),
    Candidate(String, Slot),
    Submit(String, Slot),
    /// The Game months' sources this proof can update.
    UpdateTargets(String, Vec<super::phase::Month>),
    SubmitUpdate(String, Slot),
}

enum Reply {
    List(Vec<Entry>),
    Stage(Stage),
}

#[derive(Default)]
pub struct Proofs {
    pub list: Vec<Entry>,
    pub ticket: Option<Ticket>,
    busy: bool,
    last_refresh: Option<Instant>,
    commands: Option<Sender<Command>>,
    replies: Option<Receiver<Reply>>,
}

/// An error in plain words: what to do, not how it failed.
fn plain(message: &str) -> String {
    let m = message.to_ascii_lowercase();
    if m.contains("notary") && (m.contains("reached") || m.contains("offers no notary")) {
        "The notary is not reachable right now. Try again in a moment.".into()
    } else if m.contains("sign-in browser is not open") {
        "Sign in to the site first (Sign in to a site).".into()
    } else if m.contains("could not be reached") || m.contains("unreachable") {
        "The site or the API is not reachable right now.".into()
    } else {
        crate::research::markets::plain(message.to_string())
    }
}

fn preview_lines(p: &view::Preview) -> Vec<Line<'static>> {
    let muted = Style::default().fg(MUTED);
    let mut lines = vec![Line::styled(
        p.title.clone(),
        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
    )];
    for (label, value) in &p.rows {
        if matches!(label.as_str(), "Bundle" | "Observation") {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{label:<12}"), muted),
            Span::styled(value.clone(), Style::default().fg(TEXT)),
        ]));
    }
    lines.push(Line::raw(""));
    for (text, ok) in &p.steps {
        lines.push(Line::from(vec![
            Span::styled(
                if *ok { "✓ " } else { "· " },
                Style::default().fg(if *ok { GREEN } else { DIM }),
            ),
            Span::styled(
                text.clone(),
                if *ok {
                    Style::default().fg(TEXT)
                } else {
                    muted
                },
            ),
        ]));
    }
    if !p.disclosed.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("What it discloses", muted));
        for d in &p.disclosed {
            lines.push(Line::styled(format!("  {d}"), Style::default().fg(TEXT)));
        }
    }
    if !p.trust.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(p.trust.clone(), muted));
    }
    lines
}

pub(crate) fn read_list() -> Vec<Entry> {
    let Ok(dir) = evidence::store::dir() else {
        return Vec::new();
    };
    let mut list: Vec<Entry> = evidence::store::load_index(&dir)
        .unwrap_or_default()
        .into_values()
        .collect();
    list.sort_by_key(|e| std::cmp::Reverse(e.added_at));
    list
}

fn show(ctx: &Ctx, digest: &str) -> Stage {
    let result = (|| {
        let (_, text) = evidence::load(digest)?;
        let v = amoeba_evidence::verify_bundle(text.as_bytes(), &evidence::policy(ctx)?);
        let entry = evidence::entry_of(digest);
        let submittable = v.authenticated().then(|| digest.to_string());
        Ok::<_, crate::futures::error::Error>(Stage::Preview(
            preview_lines(&view::preview(&v, entry.as_ref())),
            submittable,
        ))
    })();
    result.unwrap_or_else(|e| Stage::Failed(plain(&e.message)))
}

/// The candidate a proof becomes for `slot`: everything but the slot comes
/// from what the proof shows.
fn candidate(
    ctx: &Ctx,
    digest: &str,
    slot: &Slot,
) -> crate::futures::error::Result<serde_json::Map<String, Value>> {
    use crate::futures::error::Error;
    let (_, text) = evidence::load(digest)?;
    let v = amoeba_evidence::verify_bundle(text.as_bytes(), &evidence::policy(ctx)?);
    let env = v
        .envelope
        .ok_or_else(|| Error::msg("this proof does not verify here"))?;
    let o = &env.observation;
    let f = |k: &str| o.fields.get(k).map(|x| x.value.clone());
    let currency = f("currency")
        .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
        .ok_or_else(|| Error::msg("the proof shows no three-letter currency; the manifest needs fields.currency or assume.currency"))?;
    let product = match (f("productName"), f("product")) {
        (Some(n), Some(p)) if n != p => format!("{n} ({p})"),
        (Some(n), _) => n,
        (None, Some(p)) => p,
        (None, None) => env.extraction.manifest_name.clone(),
    };
    let product = if product.chars().count() < 8 {
        format!("{product} — {}", env.extraction.manifest_name)
    } else {
        product
    };
    let region = o
        .context
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase().contains("region"))
        .map(|(_, v)| v.trim_matches('"').to_string())
        .filter(|r| r.len() >= 2)
        .unwrap_or_else(|| "Not stated by the source".into());
    let source_type = match o.evidence_type {
        amoeba_evidence::manifest::EvidenceType::PublishedBenchmark => "Benchmark",
        _ => "Distributor",
    };
    let mut price_field = format!(
        "{} of the record {}",
        o.price.pointer.trim_start_matches('/'),
        env.extraction.record_pointer
    );
    let mut cut = price_field.len().min(128);
    while !price_field.is_char_boundary(cut) {
        cut -= 1;
    }
    price_field.truncate(cut);
    let evidence = format!(
        "Web proof {}: {} {} for {} at {}, seen {} (TLSNotary, notary {}).",
        &env.authentication.bundle_digest[..16],
        o.price.text,
        currency,
        product,
        env.origin.locator,
        env.times.witnessed_at.value,
        env.authentication.notary
    );
    let mut m = serde_json::Map::new();
    for (k, v) in [
        ("marketId", slot.market.clone()),
        ("expiryId", slot.expiry.clone()),
        ("bucketId", slot.bucket.clone()),
        ("canonicalUrl", env.origin.locator.clone()),
        ("sourceType", source_type.to_string()),
        ("productDefinition", product),
        ("priceField", price_field),
        ("region", region),
        ("currency", currency),
        ("evidence", evidence),
    ] {
        m.insert(k.into(), Value::String(v));
    }
    Ok(m)
}

/// The Game months' Active sources whose registered page this proof is of.
fn update_targets(ctx: &Ctx, digest: &str, months: &[super::phase::Month]) -> Stage {
    use solana_program::hash::hashv;
    let result = (|| {
        let (_, text) = evidence::load(digest)?;
        let v = amoeba_evidence::verify_bundle(text.as_bytes(), &evidence::policy(ctx)?);
        let env = v
            .envelope
            .ok_or_else(|| crate::futures::error::Error::msg("this proof does not verify here"))?;
        let want = hashv(&[b"locator", env.origin.locator.as_bytes()]).to_bytes();
        let oracle = crate::oracle::chain::Oracle::new(ctx)?;
        let mut slots = Vec::new();
        for m in months.iter().filter(|m| m.phase == "Game") {
            let Ok(month) = oracle.month(&m.market, Some(&m.expiry)) else {
                continue;
            };
            for s in oracle.sources(&month).unwrap_or_default() {
                if s.state.canonical_locator_hash != want
                    || s.state.status != ameba_oracle::state::OracleSourceStatus::Active
                {
                    continue;
                }
                // Without the pool's update stake there's no stake to offer.
                let Ok(stake) = oracle
                    .sku_pool(&month, &s.state.bucket_id)
                    .map(|p| p.update_min_bond)
                else {
                    continue;
                };
                slots.push(Slot {
                    market: m.market.clone(),
                    expiry: m.expiry.clone(),
                    bucket: crate::oracle::skus::hex(&s.state.bucket_id),
                    title: format!(
                        "{} · {} · {} (source {}…)",
                        m.market.to_uppercase(),
                        super::render::month_name(&m.expiry).unwrap_or_else(|| m.expiry.clone()),
                        s.bucket_label(),
                        &s.address.to_string()[..8]
                    ),
                    update: Some(UpdateTarget {
                        source: s.address.to_string(),
                        stake,
                    }),
                });
            }
        }
        Ok::<_, crate::futures::error::Error>(slots)
    })();
    match result {
        Ok(slots) if slots.is_empty() => Stage::Failed(
            "No source in a Game month has this page. Submit it as a source instead, while a month takes sources.".into(),
        ),
        Ok(slots) => Stage::Choose(digest.to_string(), slots),
        Err(e) => Stage::Failed(plain(&e.message)),
    }
}

fn update_review(ctx: &Ctx, digest: &str, slot: Slot) -> Stage {
    let result = (|| {
        let (_, text) = evidence::load(digest)?;
        let v = amoeba_evidence::verify_bundle(text.as_bytes(), &evidence::policy(ctx)?);
        let env = v
            .envelope
            .ok_or_else(|| crate::futures::error::Error::msg("this proof does not verify here"))?;
        let stake = slot.update.as_ref().map(|u| u.stake).unwrap_or(0);
        Ok::<_, crate::futures::error::Error>(vec![
            format!(
                "New value: {} (the proven price)",
                env.observation.price.text
            ),
            format!(
                "Seen: {} (the notary's session time)",
                env.times.witnessed_at.value
            ),
            format!(
                "Stake: {} USDC from your oracle collateral",
                crate::futures::units::usdc(stake)
            ),
            "Petri commits it sealed, reveals it, and publishes the proof so anyone can check it."
                .into(),
            "Anyone can challenge it for a day; unchallenged, it counts.".into(),
        ])
    })();
    match result {
        Ok(lines) => Stage::Review(digest.to_string(), slot, lines),
        Err(e) => Stage::Failed(plain(&e.message)),
    }
}

/// Commit, then reveal (the program takes a reveal one slot after the
/// commit, `ORACLE_UPDATE_REVEAL_DELAY_SLOTS`), citing the proof.
fn submit_update(ctx: &Ctx, digest: &str, slot: &Slot) -> Stage {
    use crate::oracle::{args::Target, writes};
    let Some(update) = &slot.update else {
        return Stage::Failed("Not an update.".into());
    };
    let result = (|| {
        let target = Target {
            market: slot.expiry.clone(),
            expiry: None,
            send: Default::default(),
        };
        let p = crate::futures::view::Paint::new(false);
        let stake = crate::futures::units::trim_decimal(u128::from(update.stake.max(1)), 6);
        let committed = writes::update_commit(
            ctx,
            p,
            &target,
            &update.source,
            None,
            writes::Cite::Proof(digest),
            &stake,
        )?;
        let claim = committed.payload["details"]["claim"]
            .as_str()
            .ok_or_else(|| crate::futures::error::Error::msg("the commit named no claim"))?
            .to_string();
        let mut last = None;
        for _ in 0..10 {
            match writes::update_reveal(ctx, p, &claim, &Default::default()) {
                Ok(_) => return Ok(claim),
                Err(e) => last = Some(e),
            }
            // The next slot (about 0.3-0.4 s on the dev chain).
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(last.unwrap_or_else(|| crate::futures::error::Error::msg("the reveal did not land")))
    })();
    match result {
        Ok(claim) => Stage::Done(vec![
            format!(
                "Update committed and revealed: claim {}…",
                &claim[..8.min(claim.len())]
            ),
            "The proof is published; anyone can check it during the challenge day.".into(),
        ]),
        Err(e) => Stage::Failed(plain(&e.message)),
    }
}

fn worker(settings: Settings) -> (Sender<Command>, Receiver<Reply>) {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    thread::spawn(move || {
        let mut quiet = settings;
        quiet.json = true;
        quiet.yes = true;
        let ctx = Ctx::new(quiet);
        while let Ok(command) = command_rx.recv() {
            let reply = match command {
                Command::Refresh => Reply::List(read_list()),
                Command::SignIn(url) => Reply::Stage(match evidence::open_browser(Some(&url)) {
                    Ok(_) => Stage::Done(vec![
                        format!("Opened your browser on {url}."),
                        "Sign in there yourself: Petri never asks for your password.".into(),
                        "Then Prove a page, with the address of the JSON the page shows.".into(),
                    ]),
                    Err(e) => Stage::Failed(plain(&e.message)),
                }),
                Command::Prove(url, manifest) => Reply::Stage({
                    let signed_in = evidence::browser_profile()
                        .ok()
                        .and_then(|p| amoeba_webproof::browser::devtools_of(&p))
                        .is_some();
                    match std::fs::read(&manifest) {
                        Err(_) => Stage::Failed(format!("No manifest at {}.", manifest.display())),
                        Ok(bytes) => {
                            let args = evidence::ProveArgs {
                                url: &url,
                                manifest: &bytes,
                                browser: signed_in,
                                cdp: None,
                                headers: &[],
                                note: None,
                                out: None,
                                notary: None,
                                no_bind: false,
                                max_response: None,
                                connect_to: None,
                                dev_root: None,
                                dev_notary: None,
                            };
                            match evidence::prove(&ctx, &args) {
                                Ok(p) => {
                                    let digest = p.entry.as_ref().map(|e| e.digest.clone());
                                    let mut lines = preview_lines(&view::preview(
                                        &p.verification,
                                        p.entry.as_ref(),
                                    ));
                                    if p.entry.is_none() {
                                        lines.push(Line::raw(""));
                                        lines.push(Line::styled(
                                            if p.status == 401 || p.status == 403 {
                                                "The site refused: sign in first, then prove again."
                                            } else {
                                                "Nothing was kept: the manifest reads no record in it."
                                            },
                                            Style::default().fg(ALERT),
                                        ));
                                    }
                                    Stage::Preview(lines, digest)
                                }
                                Err(e) => Stage::Failed(plain(&e.message)),
                            }
                        }
                    }
                }),
                Command::Import(path) => Reply::Stage({
                    let result = (|| {
                        let text = std::fs::read_to_string(&path).map_err(|_| {
                            crate::futures::error::Error::msg(format!(
                                "No bundle at {}.",
                                path.display()
                            ))
                        })?;
                        let v = amoeba_evidence::verify_bundle(
                            text.as_bytes(),
                            &evidence::policy(&ctx)?,
                        );
                        if !v.authenticated() {
                            return Ok(Stage::Preview(
                                preview_lines(&view::preview(&v, None)),
                                None,
                            ));
                        }
                        let report = crate::evidence::import_text(&text, &v)?;
                        Ok::<_, crate::futures::error::Error>(Stage::Preview(
                            preview_lines(&view::preview(&v, Some(&report))),
                            Some(report.digest),
                        ))
                    })();
                    result.unwrap_or_else(|e| Stage::Failed(plain(&e.message)))
                }),
                Command::Show(digest) => Reply::Stage(show(&ctx, &digest)),
                Command::UpdateTargets(digest, months) => {
                    Reply::Stage(update_targets(&ctx, &digest, &months))
                }
                Command::SubmitUpdate(digest, slot) => {
                    Reply::Stage(submit_update(&ctx, &digest, &slot))
                }
                Command::Candidate(digest, slot) if slot.update.is_some() => {
                    Reply::Stage(update_review(&ctx, &digest, slot))
                }
                Command::Candidate(digest, slot) => {
                    Reply::Stage(match candidate(&ctx, &digest, &slot) {
                        Ok(m) => Stage::Review(
                            digest,
                            slot,
                            m.iter()
                                .filter(|(k, _)| {
                                    !matches!(k.as_str(), "marketId" | "expiryId" | "bucketId")
                                })
                                .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or("")))
                                .collect(),
                        ),
                        Err(e) => Stage::Failed(plain(&e.message)),
                    })
                }
                Command::Submit(digest, slot) => Reply::Stage({
                    let result = (|| {
                        let m = candidate(&ctx, &digest, &slot)?;
                        let (_, text) = evidence::load(&digest)?;
                        let dir = evidence::store::dir()?;
                        evidence::submit::upload(&ctx, &dir, &digest, &text)?;
                        let reply = evidence::submit::candidate_map(&ctx, &dir, &digest, m)?;
                        Ok::<_, crate::futures::error::Error>(
                            if reply["duplicate"].as_bool() == Some(true) {
                                Stage::Done(vec![
                                    "This observation was already submitted for this product."
                                        .into(),
                                    "The first submission keeps the credit.".into(),
                                ])
                            } else {
                                Stage::Done(vec![
                                    format!("Submitted as a source for {}.", slot.title),
                                    "Sponsors see it in Fund; the oracle decides on chain.".into(),
                                ])
                            },
                        )
                    })();
                    result.unwrap_or_else(|e| Stage::Failed(plain(&e.message)))
                }),
            };
            if reply_tx.send(reply).is_err() {
                break;
            }
        }
    });
    (command_tx, reply_rx)
}

impl Proofs {
    pub fn start(&mut self, settings: Settings) {
        let (c, r) = worker(settings);
        self.commands = Some(c);
        self.replies = Some(r);
    }

    fn send(&mut self, command: Command) {
        if let Some(tx) = &self.commands
            && tx.send(command).is_ok()
        {
            self.busy = true;
        }
    }

    pub fn tick(&mut self, active: bool) {
        let mut replies = Vec::new();
        if let Some(rx) = &self.replies {
            while let Ok(reply) = rx.try_recv() {
                replies.push(reply);
            }
        }
        for reply in replies {
            self.busy = false;
            match reply {
                Reply::List(list) => self.list = list,
                Reply::Stage(stage) => {
                    if let Some(t) = &mut self.ticket {
                        t.stage = stage;
                        t.choice = 0;
                    }
                    self.last_refresh = None;
                }
            }
        }
        let every = Duration::from_secs(if active { 3 } else { 30 });
        if !self.busy
            && self.commands.is_some()
            && self.last_refresh.is_none_or(|t| t.elapsed() >= every)
        {
            self.last_refresh = Some(Instant::now());
            self.send(Command::Refresh);
        }
    }

    pub fn open(&mut self, kind: Kind) {
        let fields = match &kind {
            Kind::SignIn | Kind::Import => vec![String::new()],
            Kind::Prove => vec![String::new(), String::new()],
            Kind::Show(_) => vec![],
        };
        let stage = if let Kind::Show(d) = &kind {
            self.send(Command::Show(d.clone()));
            Stage::Working
        } else {
            Stage::Edit
        };
        self.ticket = Some(Ticket {
            kind,
            fields,
            focus: 0,
            stage,
            choice: 0,
        });
    }

    /// Submit the previewed proof: choose the month and product.
    pub fn choose(&mut self, months: &[super::phase::Month]) {
        let Some(t) = &mut self.ticket else { return };
        let Stage::Preview(_, Some(digest)) = &t.stage else {
            return;
        };
        let slots: Vec<Slot> = months
            .iter()
            .filter(|m| m.phase == "SourceSubmission")
            .flat_map(|m| {
                m.skus.iter().map(move |s| Slot {
                    market: m.market.clone(),
                    expiry: m.expiry.clone(),
                    bucket: s.bucket.clone(),
                    title: format!(
                        "{} · {} · {}",
                        m.market.to_uppercase(),
                        super::render::month_name(&m.expiry).unwrap_or_else(|| m.expiry.clone()),
                        s.label
                            .clone()
                            .unwrap_or_else(|| format!("{}…", &s.bucket[..8.min(s.bucket.len())]))
                    ),
                    update: None,
                })
            })
            .collect();
        t.stage = if slots.is_empty() {
            Stage::Failed("No month is taking sources right now.".into())
        } else {
            Stage::Choose(digest.clone(), slots)
        };
        t.choice = 0;
    }

    /// A slot picked (click or Enter): the candidate to review.
    pub fn pick(&mut self, index: usize) {
        let Some(t) = &mut self.ticket else { return };
        let Stage::Choose(digest, slots) = &t.stage else {
            return;
        };
        if let Some(slot) = slots.get(index).cloned() {
            let digest = digest.clone();
            t.stage = Stage::Working;
            self.send(Command::Candidate(digest, slot));
        }
    }

    pub fn confirm(&mut self) {
        let Some(t) = &mut self.ticket else { return };
        if let Stage::Review(digest, slot, _) = &t.stage {
            let (d, s) = (digest.clone(), slot.clone());
            t.stage = Stage::Working;
            let command = if s.update.is_some() {
                Command::SubmitUpdate(d, s)
            } else {
                Command::Submit(d, s)
            };
            self.send(command);
        }
    }

    /// Submit the previewed proof as an update of a Game month's source.
    pub fn choose_update(&mut self, months: &[super::phase::Month]) {
        let Some(t) = &mut self.ticket else { return };
        let Stage::Preview(_, Some(digest)) = &t.stage else {
            return;
        };
        let digest = digest.clone();
        t.stage = Stage::Working;
        self.send(Command::UpdateTargets(digest, months.to_vec()));
    }

    /// The pop-up's own button: open, prove or import with what is typed.
    pub fn go(&mut self) {
        self.submit_fields();
    }

    fn submit_fields(&mut self) {
        let Some(t) = &mut self.ticket else { return };
        let f: Vec<String> = t.fields.iter().map(|s| s.trim().to_string()).collect();
        let command = match t.kind {
            Kind::SignIn if !f[0].is_empty() => Command::SignIn(f[0].clone()),
            Kind::Prove if !f[0].is_empty() && !f[1].is_empty() => {
                Command::Prove(f[0].clone(), PathBuf::from(&f[1]))
            }
            Kind::Import if !f[0].is_empty() => Command::Import(PathBuf::from(&f[0])),
            _ => return,
        };
        t.stage = Stage::Working;
        self.send(command);
    }

    /// Pasted text goes into the field being edited.
    pub fn paste(&mut self, text: &str) {
        if let Some(t) = &mut self.ticket
            && t.stage == Stage::Edit
            && let Some(field) = t.fields.get_mut(t.focus)
        {
            field.extend(text.chars().filter(|c| !c.is_control()).take(1024));
        }
    }

    /// A key while the pop-up is open; `months` for Submit.
    pub fn key(&mut self, key: KeyEvent, months: &[super::phase::Month]) {
        let Some(t) = self.ticket.clone() else { return };
        match (&t.stage, key.code) {
            (_, KeyCode::Esc) => self.ticket = None,
            (Stage::Working, _) => {}
            (Stage::Done(_) | Stage::Failed(_), _) => self.ticket = None,
            (Stage::Edit, KeyCode::Tab | KeyCode::Down) => {
                if let Some(t) = &mut self.ticket {
                    t.focus = (t.focus + 1) % t.fields.len().max(1);
                }
            }
            (Stage::Edit, KeyCode::BackTab | KeyCode::Up) => {
                if let Some(t) = &mut self.ticket {
                    let n = t.fields.len().max(1);
                    t.focus = (t.focus + n - 1) % n;
                }
            }
            (Stage::Edit, KeyCode::Enter) => {
                if t.focus + 1 < t.fields.len() {
                    if let Some(t) = &mut self.ticket {
                        t.focus += 1;
                    }
                } else {
                    self.submit_fields();
                }
            }
            (Stage::Edit, KeyCode::Backspace) => {
                if let Some(t) = &mut self.ticket
                    && let Some(f) = t.fields.get_mut(t.focus)
                {
                    f.pop();
                }
            }
            (Stage::Edit, KeyCode::Char(c)) => {
                if let Some(t) = &mut self.ticket
                    && let Some(f) = t.fields.get_mut(t.focus)
                    && f.len() < 1024
                {
                    f.push(c);
                }
            }
            (Stage::Preview(_, Some(_)), KeyCode::Char('s') | KeyCode::Enter) => {
                self.choose(months)
            }
            (Stage::Preview(_, Some(_)), KeyCode::Char('u')) => self.choose_update(months),
            (Stage::Preview(..), _) => self.ticket = None,
            (Stage::Choose(_, slots), KeyCode::Down | KeyCode::Tab) => {
                let n = slots.len();
                if let Some(t) = &mut self.ticket {
                    t.choice = (t.choice + 1) % n;
                }
            }
            (Stage::Choose(_, slots), KeyCode::Up | KeyCode::BackTab) => {
                let n = slots.len();
                if let Some(t) = &mut self.ticket {
                    t.choice = (t.choice + n - 1) % n;
                }
            }
            (Stage::Choose(..), KeyCode::Enter) => self.pick(t.choice),
            (Stage::Review(..), KeyCode::Char('y') | KeyCode::Enter) => self.confirm(),
            (Stage::Review(..), KeyCode::Char('n')) => self.ticket = None,
            _ => {}
        }
    }
}

/// What the pop-up put where, for clicks (render.rs registers them).
#[derive(Default)]
pub struct Drawn {
    pub ticket: Option<Rect>,
    /// Clickable rows inside it: (area, what a click does).
    pub buttons: Vec<(Rect, Click)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Click {
    /// Inside the pop-up, on nothing clickable (keeps the click from the
    /// view behind it).
    Nothing,
    Field(usize),
    Go,
    Submit,
    Pick(usize),
    Confirm,
    SubmitUpdate,
}

fn rows_of(lines: &[Line], width: u16) -> u16 {
    lines
        .iter()
        .map(|l| {
            super::fund::wrapped_rows(&l.to_string(), usize::from(width.saturating_sub(4))).max(1)
                as u16
        })
        .sum()
}

/// The pop-up over `area` (the Evidence view's content).
pub fn draw_ticket(frame: &mut Frame, t: &Ticket, area: Rect) -> Drawn {
    let mut drawn = Drawn::default();
    let muted = Style::default().fg(MUTED);
    let title = match t.kind {
        Kind::SignIn => "SIGN IN TO A SITE",
        Kind::Prove => "PROVE A PAGE",
        Kind::Import => "IMPORT A PROOF",
        Kind::Show(_) => "WEB PROOF",
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    // (line index, click) for rows that are buttons.
    let mut clicks: Vec<(usize, Click)> = Vec::new();
    let hint = match &t.stage {
        Stage::Edit => {
            let labels: &[&str] = match t.kind {
                Kind::SignIn => &["Site address"],
                Kind::Prove => &["JSON address", "Manifest file"],
                Kind::Import => &["Bundle file"],
                Kind::Show(_) => &[],
            };
            for (i, label) in labels.iter().enumerate() {
                let value = t.fields.get(i).cloned().unwrap_or_default();
                let focused = i == t.focus;
                clicks.push((lines.len(), Click::Field(i)));
                lines.push(Line::from(vec![
                    Span::styled(format!("{label:<14}"), muted),
                    Span::styled(
                        format!("{value}{}", if focused { "▏" } else { "" }),
                        Style::default().fg(if focused { ACCENT } else { TEXT }),
                    ),
                ]));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                match t.kind {
                    Kind::SignIn => "Opens your own browser with Petri's profile. You sign in there; Petri never sees your password.",
                    Kind::Prove => "Proves the page from this computer with your signed-in session. Nothing is sent until you submit.",
                    _ => "A bundle made with petri evidence prove, on any computer.",
                },
                muted,
            ));
            clicks.push((lines.len(), Click::Go));
            lines.push(Line::styled(
                match t.kind {
                    Kind::SignIn => "[ Open ]",
                    Kind::Prove => "[ Prove ]",
                    _ => "[ Import ]",
                },
                Style::default().fg(ACCENT),
            ));
            "Tab next field · Enter · Esc"
        }
        Stage::Working => "",
        Stage::Preview(preview, digest) => {
            lines.extend(preview.iter().cloned());
            lines.push(Line::raw(""));
            if digest.is_some() {
                clicks.push((lines.len(), Click::Submit));
                lines.push(Line::styled(
                    "[ Submit as a source ]",
                    Style::default().fg(ACCENT),
                ));
                clicks.push((lines.len(), Click::SubmitUpdate));
                lines.push(Line::styled(
                    "[ Submit as an update ]",
                    Style::default().fg(ACCENT),
                ));
                "s source · u update · Esc close"
            } else {
                "any key closes"
            }
        }
        Stage::Choose(_, slots) => {
            lines.push(Line::styled(
                "Which product is this a source for?",
                Style::default().fg(TEXT),
            ));
            lines.push(Line::raw(""));
            for (i, slot) in slots.iter().enumerate() {
                clicks.push((lines.len(), Click::Pick(i)));
                let style = if i == t.choice {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(TEXT)
                };
                lines.push(Line::styled(
                    format!("{} {}", if i == t.choice { "›" } else { " " }, slot.title),
                    style,
                ));
            }
            "↑↓ choose · Enter · Esc"
        }
        Stage::Review(_, slot, rows) => {
            lines.push(Line::styled(
                format!("Source candidate for {}", slot.title),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ));
            for r in rows {
                lines.push(Line::styled(r.clone(), muted));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                if slot.update.is_some() {
                    "Signed by your wallet, with the proof as its evidence."
                } else {
                    "Submitted as you, with the proof attached; rewards go to your wallet."
                },
                muted,
            ));
            clicks.push((lines.len(), Click::Confirm));
            lines.push(Line::styled("[ Submit ]", Style::default().fg(ACCENT)));
            "y/Enter submit · Esc cancel"
        }
        Stage::Done(done) => {
            lines.extend(
                done.iter()
                    .map(|l| Line::styled(l.clone(), Style::default().fg(GREEN))),
            );
            "any key closes"
        }
        Stage::Failed(e) => {
            lines.push(Line::styled(e.clone(), Style::default().fg(ALERT)));
            "any key closes"
        }
    };
    let width = area.width.min(84);
    let text_rows = rows_of(&lines, width);
    let height = (text_rows + 3).max(5).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    scrim(frame.buffer_mut(), rect);
    frame.render_widget(Clear, rect);
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().fg(TEXT).bg(PANEL))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let body = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(1),
    );
    // Where each line lands once wrapped, for the clickable ones.
    let mut y = body.y;
    for (i, line) in lines.iter().enumerate() {
        let h = super::fund::wrapped_rows(&line.to_string(), usize::from(body.width)).max(1) as u16;
        if let Some((_, click)) = clicks.iter().find(|(at, _)| *at == i)
            && y < body.bottom()
        {
            drawn
                .buttons
                .push((Rect::new(body.x, y, body.width, 1), click.clone()));
        }
        y += h;
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    if !hint.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {hint}"), Style::default().fg(DIM))),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }
    drawn.ticket = Some(rect);
    drawn
}

/// One row of the list: "41.25 USD · portal.example.com/api/prices · not submitted".
pub fn row_text(e: &Entry) -> String {
    let last = e
        .events
        .last()
        .map(|ev| ev.summary.clone())
        .unwrap_or_else(|| "not submitted".into());
    format!(
        "{} · {} · {}",
        if e.price.is_empty() { "–" } else { &e.price },
        e.locator.trim_start_matches("https://"),
        last
    )
}
