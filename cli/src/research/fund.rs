//! The Fund view: price pages research agents found (candidates, waiting to be
//! funded onto the oracle), the wallet's oracle balance that bonds and stakes
//! come from (Add funds, Withdraw), and the sources the wallet listed.
//! Reads and writes run on a worker thread, as the Markets view's do.

use std::{
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

use super::render::{ACCENT, ALERT, BORDER, DIM, MUTED, PANEL, TEXT, month_name, scrim};
use crate::{
    futures::{ctx::Ctx, settings::Settings, view::Paint},
    oracle::{chain::Oracle, reads, send, skus, sponsor, writes},
};

/// The Markets view's green: on the oracle, done.
const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(122, 178, 132);

/// A price page a research agent found: a candidate, not yet on the oracle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Find {
    pub content_hash: String,
    /// "RAMX · Nov 2026 · DDR5 16Gb".
    pub title: String,
    pub url: String,
    pub definition: String,
    /// What is priced: field, region and currency.
    pub price: String,
    pub evidence: String,
    pub agent: String,
    pub created_ms: u64,
    /// Its status on the oracle, once a source there has the same page.
    pub on_oracle: Option<String>,
}

/// A source on the oracle that this wallet listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mine {
    pub title: String,
    pub url: String,
    pub status: String,
}

/// A sponsorship this wallet paid for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sponsored {
    pub title: String,
    pub bond: u64,
    /// "waiting", "live, reward to claim", "rejected", …
    pub outcome: String,
    pub reward_paid: u64,
}

/// Everything the view shows, read together.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub finds: Vec<Find>,
    pub mine: Vec<Mine>,
    pub sponsored: Vec<Sponsored>,
    /// Available and locked USDC atoms of the oracle balance; (0, 0) before
    /// its first deposit, None without a wallet.
    pub balance: Option<(u64, u64)>,
    pub owner: Option<String>,
    pub error: Option<String>,
    /// Every oracle month and where it is in its life (phase.rs).
    pub months: Vec<super::phase::Month>,
    /// The chain's clock, unix seconds (a dev chain's runs slow).
    pub chain_now: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Deposit,
    Withdraw,
    /// Sponsor the find with this content hash.
    Sponsor(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Edit,
    Working,
    Review(Vec<String>),
    Done(Vec<String>),
    Failed(String),
}

/// Adding funds or withdrawing: an amount, a review, then the send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub kind: Kind,
    pub amount: String,
    pub stage: Stage,
}

enum Command {
    Refresh,
    Prepare(Kind, String),
    Send,
}

pub(crate) enum Reply {
    Snapshot(Box<Snapshot>),
    Prepared(Result<Vec<String>, String>),
    Sent(Result<Vec<String>, String>),
}

#[derive(Default)]
pub struct Fund {
    pub snap: Snapshot,
    pub loaded: bool,
    /// The last read failed: what is shown is the read before it.
    pub offline: bool,
    pub ticket: Option<Ticket>,
    busy: bool,
    last_refresh: Option<Instant>,
    commands: Option<Sender<Command>>,
    replies: Option<Receiver<Reply>>,
}

/// "RAMX · Nov 2026 · DDR5 16Gb": the market, month and product of a bucket.
fn title(market: &str, expiry: &str, bucket: &str, label: Option<&str>) -> String {
    let product = label
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .or_else(|| {
            skus::parse_hex32(bucket, "bucket")
                .ok()
                .and_then(|b| skus::decode_label(&b))
        })
        .unwrap_or_else(|| format!("Unnamed product {}", &bucket[..bucket.len().min(6)]));
    let month = month_name(expiry).unwrap_or_else(|| expiry.to_string());
    format!("{} · {month} · {product}", market.to_uppercase())
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

/// Finds this wallet sponsored are on the oracle, whether or not the API's
/// inventory lists their month.
pub(crate) fn mark_sponsored(finds: &mut [Find], candidates: &[String]) {
    for find in finds.iter_mut() {
        if find.on_oracle.is_none() && candidates.contains(&find.content_hash) {
            find.on_oracle = Some("Sponsored by you".to_string());
        }
    }
}

/// The view's snapshot from the candidates and the oracle's public snapshot.
pub(crate) fn snapshot_from(candidates: &Value, oracle: &Value, owner: Option<&str>) -> Snapshot {
    let mut on_oracle = std::collections::HashMap::new();
    let mut mine = Vec::new();
    for link in oracle["sourceLinks"].as_array().into_iter().flatten() {
        for source in link["sources"].as_array().into_iter().flatten() {
            let status = text(source, "status");
            if let Some(hash) = source["canonicalLocator"]["hash"].as_str() {
                on_oracle.insert(hash.to_lowercase(), status.clone());
            }
            if owner.is_some_and(|o| source["proposer"].as_str() == Some(o)) {
                mine.push(Mine {
                    title: title(
                        &text(link, "marketId"),
                        &text(link, "expiryId"),
                        &text(source, "bucketId"),
                        source["bucketLabel"].as_str(),
                    ),
                    url: source["canonicalLocator"]["url"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    status,
                });
            }
        }
    }
    let finds = candidates["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let d = &c["details"];
            Find {
                content_hash: text(c, "contentHash"),
                title: title(
                    &text(d, "marketId"),
                    &text(d, "expiryId"),
                    &text(d, "bucketId"),
                    None,
                ),
                url: text(d, "canonicalUrl"),
                definition: text(d, "productDefinition"),
                price: [
                    text(d, "priceField"),
                    text(d, "region"),
                    text(d, "currency"),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
                evidence: text(d, "evidence"),
                agent: text(c, "agentId"),
                created_ms: c["createdAt"].as_u64().unwrap_or(0),
                on_oracle: on_oracle
                    .get(&text(c, "canonicalLocatorHash").to_lowercase())
                    .cloned(),
            }
        })
        .collect();
    Snapshot {
        finds,
        mine,
        sponsored: Vec::new(),
        balance: None,
        owner: owner.map(str::to_string),
        error: None,
        months: super::phase::months(oracle),
        chain_now: None,
    }
}

/// Oracle months' market and label month ("RAMX", "RAMX-202612"), by
/// address. Months never change their names, so the worker keeps them.
pub(crate) type Months = std::collections::HashMap<String, (String, String)>;

/// Reads the finds, the oracle's sources and the wallet's oracle balance.
pub fn fetch(ctx: &Ctx) -> Snapshot {
    fetch_with(ctx, &mut Months::new())
}

/// `fetch`, naming months from `known` (and adding to it). The API's two
/// documents and the wallet's chain reads go out at once: each is a round
/// trip, and over a slow link one after another adds up.
pub(crate) fn fetch_with(ctx: &Ctx, known: &mut Months) -> Snapshot {
    let owner = ctx.wallet_pubkey().ok();
    let api = &ctx.api;
    let (read, chain, clock) = thread::scope(|s| {
        let candidates = s
            .spawn(move || reads::document_at(api, &crate::endpoints::oracle_candidates(&[], 100)));
        let oracle = s.spawn(move || reads::document_at(api, crate::endpoints::ORACLE_SNAPSHOT));
        let rpc = &ctx.rpc;
        let clock = s.spawn(move || rpc.clock().ok().map(|c| c.unix_timestamp));
        let chain = owner.map(|owner| {
            let oracle = Oracle::new(ctx).ok();
            let balance = oracle
                .as_ref()
                .and_then(|o| o.collateral(&owner).ok())
                .map(|ledger| {
                    ledger.map_or((0, 0), |l| (l.available_balance, l.position_locked_balance))
                });
            let list = oracle.as_ref().and_then(|o| o.sponsorships(&owner).ok());
            (balance, list)
        });
        let read = (|| -> crate::futures::error::Result<(Value, Value)> {
            let candidates = candidates
                .join()
                .map_err(|_| crate::futures::error::Error::msg("read stopped"))??;
            let oracle = oracle
                .join()
                .map_err(|_| crate::futures::error::Error::msg("read stopped"))??;
            Ok((candidates, oracle))
        })();
        (read, chain, clock.join().ok().flatten())
    });
    let (candidates, oracle) = match read {
        Ok(read) => read,
        Err(e) => {
            return Snapshot {
                error: Some(e.message),
                ..Default::default()
            };
        }
    };
    let owner_text = owner.map(|o| o.to_string());
    let mut snap = snapshot_from(&candidates, &oracle, owner_text.as_deref());
    snap.chain_now = clock;
    if let Some((balance, list)) = chain {
        snap.balance = balance;
        // The months' names, by address, from the oracle's own inventory.
        for link in oracle["sourceLinks"].as_array().into_iter().flatten() {
            if let Some(address) = link["oracleMonthAddress"].as_str() {
                known.insert(
                    address.to_string(),
                    (text(link, "marketId"), text(link, "expiryId")),
                );
            }
        }
        if let Some(list) = list {
            let chain = Oracle::new(ctx).ok();
            let mut held = Vec::new();
            snap.sponsored = list
                .into_iter()
                .map(|(_, a)| {
                    // A month the API does not list is named from the chain.
                    let address = a.month.to_string();
                    if !known.contains_key(&address)
                        && let Some(month) = chain.as_ref().and_then(|o| o.month_at(&a.month).ok())
                    {
                        known.insert(
                            address.clone(),
                            (month.period.symbol.clone(), month.period.expiry_id()),
                        );
                    }
                    let (market, expiry) = known.get(&address).cloned().unwrap_or_default();
                    held.push(skus::hex(&a.candidate_hash));
                    Sponsored {
                        title: title(&market, &expiry, &skus::hex(&a.bucket_id), None),
                        bond: a.bond,
                        outcome: sponsor::outcome(&a).to_string(),
                        reward_paid: a.reward_paid,
                    }
                })
                .collect();
            mark_sponsored(&mut snap.finds, &held);
        }
    }
    snap
}

/// What a sent write did, in words (the signatures stay in `petri history`).
fn done(write: &send::Write) -> Vec<String> {
    let after = match write.action {
        "oracle.sponsor" => "Sponsored.",
        "oracle.collateral.deposit" => "Added.",
        "oracle.collateral.withdraw" => "Withdrawn.",
        _ => "Done.",
    };
    vec![write.title.clone(), after.to_string()]
}

/// A write's review as lines: what it does, then its details.
fn review(write: &send::Write) -> Vec<String> {
    let mut lines = vec![write.title.clone()];
    lines.extend(write.review.iter().cloned());
    lines
}

fn worker(settings: Settings) -> (Sender<Command>, Receiver<Reply>) {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    thread::spawn(move || {
        let ctx = Ctx::new(settings.clone());
        // A write is sent from the TUI: confirmed in the ticket, nothing
        // printed over the screen.
        let mut quiet = settings.clone();
        quiet.yes = true;
        quiet.json = true;
        let mut pending: Option<(Ctx, send::Write)> = None;
        let mut months = Months::new();
        while let Ok(command) = command_rx.recv() {
            let reply = match command {
                Command::Refresh => Reply::Snapshot(Box::new(fetch_with(&ctx, &mut months))),
                Command::Prepare(kind, amount) => {
                    let write_ctx = Ctx::new(quiet.clone());
                    let write = match kind {
                        Kind::Deposit => writes::deposit_write(&write_ctx, &amount),
                        Kind::Withdraw => writes::withdraw_write(&write_ctx, &amount),
                        Kind::Sponsor(hash) => sponsor::sponsor_write(&write_ctx, &hash),
                    };
                    Reply::Prepared(match write {
                        Ok(write) => {
                            let lines = review(&write);
                            pending = Some((write_ctx, write));
                            Ok(lines)
                        }
                        Err(e) => {
                            pending = None;
                            Err(super::markets::plain(e.message))
                        }
                    })
                }
                Command::Send => Reply::Sent(match pending.take() {
                    Some((write_ctx, write)) => {
                        let lines = done(&write);
                        send::run(&write_ctx, Paint::new(false), write, false)
                            .map(|_| lines)
                            .map_err(|e| super::markets::plain(e.message))
                    }
                    None => Err("Review it again first.".into()),
                }),
            };
            if reply_tx.send(reply).is_err() {
                break;
            }
        }
    });
    (command_tx, reply_rx)
}

impl Fund {
    pub fn start(&mut self, settings: Settings) {
        let (commands, replies) = worker(settings);
        self.commands = Some(commands);
        self.replies = Some(replies);
    }

    fn send_command(&mut self, command: Command) {
        if let Some(tx) = &self.commands
            && tx.send(command).is_ok()
        {
            self.busy = true;
        }
    }

    /// A new read. One that failed keeps what is shown, marked offline.
    pub(crate) fn take_snapshot(&mut self, snap: Snapshot) {
        self.offline = snap.error.is_some();
        if self.offline && self.loaded {
            return;
        }
        self.snap = snap;
        self.loaded = true;
    }

    pub(crate) fn reply(&mut self, reply: Reply) {
        self.busy = false;
        match reply {
            Reply::Snapshot(snap) => self.take_snapshot(*snap),
            Reply::Prepared(result) => {
                if let Some(t) = &mut self.ticket {
                    t.stage = match result {
                        Ok(lines) => Stage::Review(lines),
                        Err(e) => Stage::Failed(e),
                    };
                }
            }
            Reply::Sent(result) => {
                if let Some(t) = &mut self.ticket {
                    t.stage = match result {
                        Ok(lines) => Stage::Done(lines),
                        Err(e) => Stage::Failed(e),
                    };
                }
                self.last_refresh = None;
            }
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
            self.reply(reply);
        }
        // Open, every 5 s; elsewhere, every 15 s, so it opens with its data.
        let every = Duration::from_secs(if active { 5 } else { 15 });
        let due = self.last_refresh.is_none_or(|t| t.elapsed() >= every);
        if !self.busy && due && self.commands.is_some() {
            self.last_refresh = Some(Instant::now());
            self.send_command(Command::Refresh);
        }
    }

    /// Add funds or Withdraw: a ticket for the amount.
    pub fn open(&mut self, kind: Kind) {
        // Sponsoring has no amount to type: the product's bond is exact, so
        // the review comes first.
        if let Kind::Sponsor(_) = &kind {
            self.ticket = Some(Ticket {
                kind: kind.clone(),
                amount: String::new(),
                stage: Stage::Working,
            });
            self.send_command(Command::Prepare(kind, String::new()));
            return;
        }
        self.ticket = Some(Ticket {
            kind,
            amount: String::new(),
            stage: Stage::Edit,
        });
    }

    /// A key while the ticket is open.
    pub fn key(&mut self, key: KeyEvent) {
        let Some(mut ticket) = self.ticket.clone() else {
            return;
        };
        match (&ticket.stage, key.code) {
            (_, KeyCode::Esc) | (Stage::Review(_), KeyCode::Char('n')) => {
                self.ticket = None;
                return;
            }
            (Stage::Done(_) | Stage::Failed(_), _) => {
                self.ticket = None;
                return;
            }
            (Stage::Working, _) => return,
            (Stage::Review(_), KeyCode::Char('y') | KeyCode::Enter) => {
                ticket.stage = Stage::Working;
                self.send_command(Command::Send);
            }
            (Stage::Edit, KeyCode::Enter) if !ticket.amount.is_empty() => {
                ticket.stage = Stage::Working;
                self.send_command(Command::Prepare(ticket.kind.clone(), ticket.amount.clone()));
            }
            (Stage::Edit, KeyCode::Backspace) => {
                if ticket.amount == "all" {
                    ticket.amount.clear();
                } else {
                    ticket.amount.pop();
                }
            }
            (Stage::Edit, KeyCode::Char('a')) if ticket.kind == Kind::Withdraw => {
                ticket.amount = "all".into();
            }
            (Stage::Edit, KeyCode::Char(c))
                if (c.is_ascii_digit() || c == '.')
                    && ticket.amount != "all"
                    && ticket.amount.len() < 16 =>
            {
                ticket.amount.push(c);
            }
            _ => {}
        }
        self.ticket = Some(ticket);
    }
}

/// "$12.50" from USDC atoms (six decimals).
pub fn dollars(atoms: u64) -> String {
    format!("${}.{:02}", atoms / 1_000_000, (atoms % 1_000_000) / 10_000)
}

/// "3 d ago", "5 h ago", "12 min ago", "just now".
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(then_ms) / 60_000;
    match minutes {
        0 => "just now".into(),
        m if m < 60 => format!("{m} min ago"),
        m if m < 48 * 60 => format!("{} h ago", m / 60),
        m => format!("{} d ago", m / (24 * 60)),
    }
}

/// "example.com" from a page address.
pub fn host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_else(|| url.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn muted() -> Style {
    Style::default().fg(MUTED)
}

fn put(frame: &mut Frame, area: Rect, y: u16, line: Line<'_>) {
    if y < area.bottom() {
        frame.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
    }
}

/// Where the view put things, for clicks.
#[derive(Default)]
pub struct Drawn {
    /// Each shown find's row: its area and title, by its place in the list.
    pub finds: Vec<(Rect, String, usize)>,
    /// The ticket's box, when one is open.
    pub ticket: Option<Rect>,
}

/// The finds and your sources, below the balance row (render.rs draws that,
/// and the find rows as buttons from `Drawn`).
pub fn content(frame: &mut Frame, f: &Fund, area: Rect) -> Drawn {
    let mut drawn = Drawn::default();
    let mut y = area.y;
    if f.loaded && f.snap.error.is_some() && f.snap.finds.is_empty() {
        put(frame, area, y, Line::styled("offline", muted()));
        return drawn;
    }
    put(
        frame,
        area,
        y,
        Line::from(vec![
            Span::styled(
                "New finds",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(if f.offline { "  offline" } else { "" }, muted()),
        ]),
    );
    let side: u16 = 30;
    let column = area.right().saturating_sub(side);
    if !f.snap.finds.is_empty() {
        put(
            frame,
            Rect::new(column, y, side, 1),
            y,
            Line::styled(format!("{:<12}{}", "found", "on the oracle"), muted()),
        );
    }
    y += 1;
    if f.loaded && f.snap.finds.is_empty() {
        put(frame, area, y, Line::styled("none", muted()));
        y += 1;
    }
    let now = now_ms();
    // Room kept below the finds for both of your lists.
    let mine_rows = 4 + f.snap.mine.len().max(1) as u16 + f.snap.sponsored.len().max(1) as u16;
    for (index, find) in f.snap.finds.iter().enumerate() {
        if y + 1 + mine_rows > area.bottom() {
            break;
        }
        let row = Rect::new(area.x, y, area.width.saturating_sub(side + 1), 1);
        drawn
            .finds
            .push((row, format!("{} · {}", find.title, host(&find.url)), index));
        let oracle = match &find.on_oracle {
            Some(status) => Span::styled(
                format!("● {}", status.to_lowercase()),
                Style::default().fg(GREEN),
            ),
            None => Span::styled("–", muted()),
        };
        put(
            frame,
            Rect::new(column, y, side, 1),
            y,
            Line::from(vec![
                Span::styled(format!("{:<12}", ago(now, find.created_ms)), muted()),
                oracle,
            ]),
        );
        y += 1;
    }
    y += 1;
    put(
        frame,
        area,
        y,
        Line::styled(
            "Your sources",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    );
    y += 1;
    if f.snap.mine.is_empty() {
        put(frame, area, y, Line::styled("none", muted()));
        y += 1;
    }
    for mine in &f.snap.mine {
        if y >= area.bottom() {
            break;
        }
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(
                    format!("{} · {}", mine.title, host(&mine.url)),
                    Style::default().fg(TEXT),
                ),
                Span::styled(format!("  {}", mine.status.to_lowercase()), muted()),
            ]),
        );
        y += 1;
    }
    y += 1;
    put(
        frame,
        area,
        y,
        Line::styled(
            "Your sponsorships",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    );
    y += 1;
    if f.snap.sponsored.is_empty() {
        put(frame, area, y, Line::styled("none", muted()));
    }
    for s in &f.snap.sponsored {
        if y >= area.bottom() {
            break;
        }
        let paid = if s.reward_paid > 0 {
            format!(" · {} reward paid", dollars(s.reward_paid))
        } else {
            String::new()
        };
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(s.title.clone(), Style::default().fg(TEXT)),
                Span::styled(
                    format!("  {} bond · {}{paid}", dollars(s.bond), s.outcome),
                    muted(),
                ),
            ]),
        );
        y += 1;
    }
    if let Some(ticket) = &f.ticket {
        drawn.ticket = Some(draw_ticket(frame, f, ticket, area));
    }
    drawn
}

/// The balance row's text: "Oracle balance $12.00 · $1.00 locked" (in
/// bonds and stakes).
pub fn balance_line(f: &Fund) -> String {
    match f.snap.balance {
        Some((available, 0)) => format!("Oracle balance {}", dollars(available)),
        Some((available, locked)) => format!(
            "Oracle balance {} · {} locked",
            dollars(available),
            dollars(locked)
        ),
        None if f.loaded && f.snap.owner.is_none() => "No wallet".into(),
        None => "Oracle balance".into(),
    }
}

/// Rows `text` takes when word-wrapped at `width`.
fn wrapped_rows(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let (mut rows, mut used) = (1, 0);
    for word in text.split_whitespace() {
        let n = word.chars().count();
        if used > 0 && used + 1 + n > width {
            rows += 1;
            used = 0;
        }
        used += if used > 0 { 1 + n } else { n };
        while used > width {
            rows += 1;
            used -= width;
        }
    }
    rows
}

fn draw_ticket(frame: &mut Frame, f: &Fund, ticket: &Ticket, area: Rect) -> Rect {
    let (title, verb) = match ticket.kind {
        Kind::Deposit => ("ADD FUNDS", "Add"),
        Kind::Withdraw => ("WITHDRAW", "Withdraw"),
        Kind::Sponsor(_) => ("SPONSOR", "Sponsor"),
    };
    let mut lines: Vec<Line> = Vec::new();
    let hint = match &ticket.stage {
        Stage::Edit => {
            let shown = if ticket.amount.is_empty() {
                "$".to_string()
            } else if ticket.amount == "all" {
                "all".to_string()
            } else {
                format!("${}", ticket.amount)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{verb} "), muted()),
                Span::styled(
                    shown,
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
            ]));
            if let Some((available, _)) = f.snap.balance
                && ticket.kind == Kind::Withdraw
            {
                lines.push(Line::styled(
                    format!("{} available", dollars(available)),
                    muted(),
                ));
            }
            match ticket.kind {
                Kind::Deposit => "amount · Enter review · Esc",
                Kind::Withdraw => "amount · a all · Enter review · Esc",
                Kind::Sponsor(_) => "Esc",
            }
        }
        // Waiting on the chain: nothing to say yet.
        Stage::Working => "",
        Stage::Review(review) => {
            lines.extend(
                review
                    .iter()
                    .map(|l| Line::styled(l.clone(), Style::default().fg(TEXT))),
            );
            "y/Enter send · n/Esc cancel"
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
    let width = area.width.min(64);
    // Borders, the one-cell margins and the hint row around the wrapped text.
    let rows: usize = lines
        .iter()
        .map(|l| wrapped_rows(&l.to_string(), usize::from(width.saturating_sub(4))))
        .sum();
    let height = (rows as u16 + 3).max(5).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.bottom().saturating_sub(height),
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
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height.saturating_sub(1),
        ),
    );
    put(
        frame,
        inner,
        inner.bottom().saturating_sub(1),
        Line::styled(format!(" {hint}"), Style::default().fg(DIM)),
    );
    rect
}
