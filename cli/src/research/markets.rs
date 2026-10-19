//! The Markets view: the futures dashboard inside Petri's terminal UI.
//! A background worker reads the futures API (`/v1/markets`, `/v1/accounts`)
//! and runs tickets through the same paths as `petri long`, `petri close` and
//! `petri cancel` (`futures::trade`); the view draws the market list
//! (sidebar), the book of the market's suggested range, trades, the index
//! and price with a price chart, your positions and orders, and a key-driven
//! ticket.

use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use ameba_sdk_futures::futures::Side;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    chart::fill_candles,
    futures::{
        api::{self, Account, AccountMarket, Depth, MarketView, Trade},
        args::{CancelArgs, CloseArgs, MarketArg, OrderArgs, Pick, SideArg},
        atoms,
        ctx::Ctx,
        money,
        reads::{change, opt_price, price, status},
        settings::Settings,
        shape::{self, Grid, Shape},
        signed_money,
        trade::{self, CancelPlan, ClosePlan, Opening},
        view::{Paint, area_chart},
    },
};

use super::render::{ACCENT, ALERT, BORDER, DIM, MUTED, PANEL, SELECTED, TEXT, scrim};

const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(122, 178, 132);
const RED: ratatui::style::Color = ratatui::style::Color::Rgb(198, 128, 120);

/// How much worse than its quote a ticket may fill, in percent: the CLI's
/// `--slippage` default.
const SLIPPAGE: &str = "2";

/// Everything the dashboard shows, fetched together.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub markets: Vec<MarketView>,
    /// The executable depth of the selected market's suggested range, the
    /// book `petri book` shows: the same whatever the ticket says.
    pub book: Option<Depth>,
    pub trades: Vec<Trade>,
    pub closes: Vec<f64>,
    pub index: Vec<f64>,
    pub account: Option<Account>,
    pub owner: Option<String>,
    pub error: Option<String>,
    /// The market the book, trades, closes and index are of.
    pub detail_for: Option<String>,
}

/// A market's book, trades, closes and index, kept so that showing it
/// again draws at once (Markets::details).
#[derive(Clone, Debug, Default)]
struct Detail {
    book: Option<Depth>,
    trades: Vec<Trade>,
    closes: Vec<f64>,
    index: Vec<f64>,
}

/// How often the shown market is read again while the view is open, and
/// everything else is, in the background, so any view or market opens with
/// its data already there.
const SHOWN: Duration = Duration::from_secs(2);
const BACKGROUND: Duration = Duration::from_secs(10);
/// The markets not shown, each read this often.
const OTHERS: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Edit,
    /// Quoting or sending: nothing to say until it is back.
    Working,
    Quoted(Vec<String>),
    Done(Vec<String>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A market order: `petri long|short <market> <lots> -x <leverage>`.
    Take {
        side: SideArg,
        lots: u64,
        leverage: u32,
    },
    /// `petri close <market> <slot> --lots <lots>`.
    Close { slot: u32, lots: u64, max: u64 },
    /// `petri cancel <market> <slot>`.
    Cancel { slot: u16, text: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// The market's address.
    pub market: String,
    pub label: String,
    pub kind: Kind,
    pub stage: Stage,
}

enum Command {
    Refresh(Option<String>),
    /// Only this market's book, trades and chart (Markets::details).
    Detail(String),
    Quote(Ticket),
    Send,
}

enum Reply {
    Snapshot(Box<Snapshot>),
    Detail(String, Box<Detail>),
    Quoted(Result<Vec<String>, String>),
    Sent(Result<Vec<String>, String>),
}

#[derive(Default)]
pub struct Markets {
    pub snap: Snapshot,
    pub selected: usize,
    pub position: usize,
    pub order: usize,
    pub ticket: Option<Ticket>,
    pub loaded: bool,
    /// The last read failed: what is shown is the read before it.
    pub offline: bool,
    busy: bool,
    last_refresh: Option<Instant>,
    /// Every market read so far, by address, and when.
    details: HashMap<String, (Detail, Instant)>,
    commands: Option<Sender<Command>>,
    replies: Option<Receiver<Reply>>,
}

fn err(e: crate::futures::error::Error) -> String {
    e.message.replace('\n', " · ")
}

/// An error as a person reads it: an unreachable API or RPC is one plain
/// phrase, not its address and transport error.
pub(crate) fn plain(error: String) -> String {
    if error.contains(" is unreachable") {
        "Couldn't reach Amoeba.".into()
    } else {
        error
    }
}

/// Reads the markets, the selected market's book, trades, candles and
/// index, and the wallet's account.
pub fn fetch(ctx: &Ctx, market: Option<&str>) -> Snapshot {
    let mut snap = Snapshot::default();
    match api::markets(&ctx.api) {
        Ok(mut m) => {
            // Tradable markets first, then by expiry and symbol.
            m.sort_by(|a, b| {
                (a.status != "open", a.expiry_ts, &a.symbol).cmp(&(
                    b.status != "open",
                    b.expiry_ts,
                    &b.symbol,
                ))
            });
            snap.markets = m
        }
        Err(e) => {
            snap.error = Some(err(e));
            return snap;
        }
    }
    let selected = market
        .and_then(|a| snap.markets.iter().find(|m| m.market == a))
        .or_else(|| snap.markets.first())
        .map(|m| m.market.clone());
    let owner = ctx.wallet_pubkey().ok().map(|o| o.to_string());
    // The account alongside the market: each read is a round trip.
    let api = &ctx.api;
    thread::scope(|s| {
        let account = owner
            .as_deref()
            .map(|owner| s.spawn(move || api::account(api, owner).ok()));
        if let Some(market) = selected {
            let d = detail(api, &market);
            snap.book = d.book;
            snap.trades = d.trades;
            snap.closes = d.closes;
            snap.index = d.index;
            snap.detail_for = Some(market);
        }
        snap.account = account.and_then(|a| a.join().ok().flatten());
    });
    snap.owner = owner;
    snap
}

/// A market's book, trades, candles and index, read at once.
fn detail(api: &api::Api, market: &str) -> Detail {
    thread::scope(|s| {
        let book = s.spawn(move || api::depth(api, market, None, 1_000_000_000).ok());
        let trades = s.spawn(move || api::trades(api, market, 30).unwrap_or_default());
        let candles = s.spawn(move || api::candles(api, market, 300, None).unwrap_or_default());
        let index = s.spawn(move || {
            api::index(api, market, 2_000)
                .map(|p| p.iter().map(|p| p.value).collect())
                .unwrap_or_default()
        });
        let candles = candles.join().unwrap_or_default();
        Detail {
            book: book.join().unwrap_or_default(),
            trades: trades.join().unwrap_or_default(),
            closes: fill_candles(&candles, 300).iter().map(|c| c.c).collect(),
            index: index.join().unwrap_or_default(),
        }
    })
}

/// A quoted ticket, waiting for `y`.
enum Pending {
    Open(Box<Opening>),
    Close(Box<ClosePlan>),
    Cancel(Box<CancelPlan>),
}

fn worker(settings: Settings) -> (Sender<Command>, Receiver<Reply>) {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    thread::spawn(move || {
        let ctx = Ctx::new(settings.clone());
        // A ticket is quoted and sent with its own context, so it resolves
        // against the current market list.
        let mut pending: Option<(Ctx, Pending)> = None;
        while let Ok(command) = command_rx.recv() {
            let reply = match command {
                Command::Refresh(market) => {
                    Reply::Snapshot(Box::new(fetch(&ctx, market.as_deref())))
                }
                Command::Detail(market) => {
                    let read = Box::new(detail(&ctx.api, &market));
                    Reply::Detail(market, read)
                }
                Command::Quote(ticket) => {
                    let ticket_ctx = Ctx::new(settings.clone());
                    Reply::Quoted(match quote(&ticket_ctx, &ticket) {
                        Ok((lines, quoted)) => {
                            pending = Some((ticket_ctx, quoted));
                            Ok(lines)
                        }
                        Err(e) => {
                            pending = None;
                            Err(e)
                        }
                    })
                }
                Command::Send => Reply::Sent(match pending.take() {
                    Some((ticket_ctx, quoted)) => send(&ticket_ctx, quoted),
                    None => Err("quote again first".into()),
                }),
            };
            if reply_tx.send(reply).is_err() {
                break;
            }
        }
    });
    (command_tx, reply_rx)
}

fn market_arg(ticket: &Ticket) -> MarketArg {
    MarketArg {
        market: ticket.market.clone(),
        expiry: None,
    }
}

/// The review lines without their bold title (the ticket's own title says
/// it), unindented.
fn body(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .skip(1)
        .map(|l| l.trim_start().to_string())
        .collect()
}

fn quote(ctx: &Ctx, ticket: &Ticket) -> Result<(Vec<String>, Pending), String> {
    let plain = Paint::new(false);
    let owner = ctx.wallet_pubkey().map_err(err)?;
    match &ticket.kind {
        Kind::Take {
            side,
            lots,
            leverage,
        } => {
            let args = OrderArgs {
                market: market_arg(ticket),
                lots: Some(*lots),
                usd: None,
                leverage: *leverage,
                to: None,
                range: None,
                limit: None,
                post: false,
                slippage: SLIPPAGE.into(),
            };
            let o = trade::prepare_open(ctx, trade::side_of(*side), &args).map_err(err)?;
            let review = o.review(plain);
            let mut lines = review.first().cloned().into_iter().collect::<Vec<_>>();
            lines.extend(body(&review));
            Ok((lines, Pending::Open(Box::new(o))))
        }
        Kind::Close { slot, lots, .. } => {
            let args = CloseArgs {
                market: market_arg(ticket),
                position: Pick::One(*slot),
                lots: Some(*lots),
                slippage: SLIPPAGE.into(),
                keep_cash: false,
            };
            let plan = trade::prepare_close(ctx, plain, &owner, &args).map_err(err)?;
            Ok((body(&plan.lines), Pending::Close(Box::new(plan))))
        }
        Kind::Cancel { slot, .. } => {
            let args = CancelArgs {
                market: market_arg(ticket),
                order: Pick::One(u32::from(*slot)),
            };
            let plan = trade::prepare_cancel(ctx, plain, &owner, &args).map_err(err)?;
            Ok((body(&plan.lines), Pending::Cancel(Box::new(plan))))
        }
    }
}

fn send(ctx: &Ctx, pending: Pending) -> Result<Vec<String>, String> {
    let signer = ctx.signer().map_err(err)?;
    let plain = Paint::new(false);
    match pending {
        Pending::Open(o) => {
            let (outcome, took) = trade::send_open(ctx, signer.as_ref(), &o).map_err(err)?;
            Ok(o.sent(plain, &outcome, took.as_ref()))
        }
        Pending::Close(plan) => trade::send_close(ctx, plain, signer.as_ref(), &plan)
            .map(|(lines, _)| lines)
            .map_err(err),
        Pending::Cancel(plan) => trade::send_cancel(ctx, plain, signer.as_ref(), &plan)
            .map(|(lines, _)| lines)
            .map_err(err),
    }
}

impl Markets {
    /// Starts the worker on first use.
    pub fn start(&mut self, settings: Settings) {
        if self.commands.is_none() {
            let (tx, rx) = worker(settings);
            self.commands = Some(tx);
            self.replies = Some(rx);
        }
    }

    pub fn selected_market(&self) -> Option<&MarketView> {
        self.snap
            .markets
            .get(self.selected.min(self.snap.markets.len().saturating_sub(1)))
    }

    fn account_market(&self) -> Option<&AccountMarket> {
        let m = self.selected_market()?;
        self.snap
            .account
            .as_ref()?
            .markets
            .iter()
            .find(|a| a.market == m.market)
    }

    fn send_command(&mut self, command: Command) {
        if let Some(tx) = &self.commands
            && tx.send(command).is_ok()
        {
            self.busy = true;
        }
    }

    /// A new read of the markets. One that failed keeps what is shown,
    /// marked offline, while the reads go on; with nothing shown yet,
    /// content() says plainly that the markets aren't available.
    pub(crate) fn take_snapshot(&mut self, snap: Snapshot) {
        self.offline = snap.error.is_some();
        if self.offline && !self.snap.markets.is_empty() {
            return;
        }
        let market = self.selected_market().map(|m| m.market.clone());
        let mut snap = snap;
        if let Some(read) = snap.detail_for.take() {
            let detail = Detail {
                book: snap.book.take(),
                trades: std::mem::take(&mut snap.trades),
                closes: std::mem::take(&mut snap.closes),
                index: std::mem::take(&mut snap.index),
            };
            self.keep_detail(read, detail);
        }
        self.snap = snap;
        self.loaded = true;
        if let Some(a) = market
            && let Some(at) = self.snap.markets.iter().position(|m| m.market == a)
        {
            self.selected = at;
        }
        self.show_detail();
    }

    /// Keeps a market's read. One whose book failed keeps the read before.
    fn keep_detail(&mut self, market: String, detail: Detail) {
        let detail = match self.details.remove(&market) {
            Some((old, _)) if detail.book.is_none() => old,
            _ => detail,
        };
        self.details.insert(market, (detail, Instant::now()));
    }

    /// The shown market's book, trades and chart, from the last read of it.
    fn show_detail(&mut self) {
        let detail = self
            .selected_market()
            .and_then(|m| self.details.get(&m.market))
            .map(|(d, _)| d.clone())
            .unwrap_or_default();
        self.snap.book = detail.book;
        self.snap.trades = detail.trades;
        self.snap.closes = detail.closes;
        self.snap.index = detail.index;
    }

    /// Applies worker replies and schedules refreshes (`SHOWN`, `BACKGROUND`,
    /// `OTHERS`).
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
                Reply::Snapshot(snap) => self.take_snapshot(*snap),
                Reply::Detail(market, detail) => {
                    self.keep_detail(market, *detail);
                    self.show_detail();
                }
                Reply::Quoted(result) => {
                    if let Some(t) = &mut self.ticket {
                        t.stage = match result {
                            Ok(lines) => Stage::Quoted(lines),
                            Err(e) => Stage::Failed(plain(e)),
                        };
                    }
                }
                Reply::Sent(result) => {
                    if let Some(t) = &mut self.ticket {
                        t.stage = match result {
                            Ok(lines) => Stage::Done(lines),
                            Err(e) => Stage::Failed(plain(e)),
                        };
                    }
                    self.last_refresh = None;
                }
            }
        }
        if self.busy || self.commands.is_none() {
            return;
        }
        let every = if active { SHOWN } else { BACKGROUND };
        if self.last_refresh.is_none_or(|t| t.elapsed() >= every) {
            self.last_refresh = Some(Instant::now());
            let market = self.selected_market().map(|m| m.market.clone());
            self.send_command(Command::Refresh(market));
            return;
        }
        // Between those, the other markets, one at a time, so clicking one
        // shows it at once. Not while a ticket waits on the worker.
        if self.ticket.is_some() {
            return;
        }
        let stale = self
            .snap
            .markets
            .iter()
            .map(|m| m.market.clone())
            .find(|m| {
                self.details
                    .get(m)
                    .is_none_or(|(_, at)| at.elapsed() >= OTHERS)
            });
        if let Some(market) = stale {
            // Marked read now, so a failing one waits its turn.
            let detail = self
                .details
                .remove(&market)
                .map(|(d, _)| d)
                .unwrap_or_default();
            self.details
                .insert(market.clone(), (detail, Instant::now()));
            self.send_command(Command::Detail(market));
        }
    }

    fn select(&mut self, step: isize) {
        let n = self.snap.markets.len();
        if n == 0 {
            return;
        }
        self.show(((self.selected as isize + step).rem_euclid(n as isize)) as usize);
    }

    /// Shows the market at `index` in the list (↑/↓, or a click on it). Not
    /// while a ticket is open: the ticket is for the market it was opened on.
    pub fn show(&mut self, index: usize) {
        if index >= self.snap.markets.len() || self.ticket.is_some() {
            return;
        }
        self.selected = index;
        self.position = 0;
        self.order = 0;
        self.show_detail();
        self.last_refresh = None;
    }

    fn open(&mut self, kind: Kind) {
        if let Some(m) = self.selected_market() {
            self.ticket = Some(Ticket {
                market: m.market.clone(),
                label: m.label.clone(),
                kind,
                stage: Stage::Edit,
            });
        }
    }

    /// The site's leverages that fit a `side` order from the selected
    /// market's price.
    fn leverages(&self, side: SideArg) -> Vec<u32> {
        let Some(m) = self.selected_market() else {
            return shape::LEVERAGES.to_vec();
        };
        let entry = Grid::of_view(m)
            .ok()
            .and_then(|g| g.pos_f64(&m.reference()?).ok());
        shape::leverage_stops(trade::side_of(side), entry, m.grid)
    }

    fn take(&self, side: SideArg) -> Kind {
        Kind::Take {
            side,
            lots: 1,
            leverage: shape::nearest_leverage(shape::DEFAULT_LEVERAGE, &self.leverages(side)),
        }
    }

    /// Keys for the Markets view. Returns false when the key is not ours.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if let Some(ticket) = self.ticket.clone() {
            return self.ticket_key(ticket, key);
        }
        match key.code {
            KeyCode::Up => self.select(-1),
            KeyCode::Down => self.select(1),
            KeyCode::Char('b') => self.open(self.take(SideArg::Long)),
            KeyCode::Char('s') => self.open(self.take(SideArg::Short)),
            KeyCode::Char('x') => {
                if let Some(c) = self
                    .account_market()
                    .and_then(|a| a.claims.get(self.position))
                    .cloned()
                {
                    let free = c.lots.saturating_sub(c.reserved);
                    self.open(Kind::Close {
                        slot: c.slot,
                        lots: free.max(1),
                        max: free.max(1),
                    });
                }
            }
            KeyCode::Char('d') => {
                let order = self
                    .account_market()
                    .and_then(|a| a.orders.get(self.order))
                    .cloned();
                if let Some(o) = order {
                    self.open(Kind::Cancel {
                        slot: o.slot,
                        text: format!(
                            "cancel #{} {}{} {} lots {}–{} at {}",
                            o.slot,
                            o.side,
                            if o.close { " close" } else { "" },
                            o.lots,
                            o.lo_value,
                            o.hi_value,
                            price(&o.entry_value)
                        ),
                    });
                }
            }
            KeyCode::Char('p') => {
                let n = self.account_market().map(|a| a.claims.len()).unwrap_or(0);
                self.position = if n == 0 { 0 } else { (self.position + 1) % n };
            }
            KeyCode::Char('o') => {
                let n = self.account_market().map(|a| a.orders.len()).unwrap_or(0);
                self.order = if n == 0 { 0 } else { (self.order + 1) % n };
            }
            KeyCode::Char('R') => self.last_refresh = None,
            _ => return false,
        }
        true
    }

    fn ticket_key(&mut self, mut ticket: Ticket, key: KeyEvent) -> bool {
        match (&ticket.stage, key.code) {
            (_, KeyCode::Esc) | (Stage::Quoted(_), KeyCode::Char('n')) => {
                self.ticket = None;
                return true;
            }
            (Stage::Done(_) | Stage::Failed(_), _) => {
                self.ticket = None;
                return true;
            }
            (Stage::Working, _) => return true,
            (Stage::Quoted(_), KeyCode::Char('y')) => {
                ticket.stage = Stage::Working;
                self.ticket = Some(ticket);
                self.send_command(Command::Send);
                return true;
            }
            (Stage::Edit, KeyCode::Enter) => {
                ticket.stage = Stage::Working;
                self.ticket = Some(ticket.clone());
                self.send_command(Command::Quote(ticket));
                return true;
            }
            _ => {}
        }
        if !matches!(ticket.stage, Stage::Edit | Stage::Quoted(_)) {
            return true;
        }
        let edited = match &mut ticket.kind {
            Kind::Take {
                lots,
                leverage,
                side,
            } => match key.code {
                KeyCode::Up => {
                    *lots = lots.saturating_add(1);
                    true
                }
                KeyCode::Down => {
                    *lots = lots.saturating_sub(1).max(1);
                    true
                }
                KeyCode::Right | KeyCode::Left => {
                    let stops = self.leverages(*side);
                    let at = stops
                        .iter()
                        .position(|l| *l == shape::nearest_leverage(*leverage, &stops))
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Right {
                        (at + 1).min(stops.len().saturating_sub(1))
                    } else {
                        at.saturating_sub(1)
                    };
                    *leverage = stops.get(next).copied().unwrap_or(*leverage);
                    true
                }
                KeyCode::Tab => {
                    *side = if *side == SideArg::Long {
                        SideArg::Short
                    } else {
                        SideArg::Long
                    };
                    *leverage = shape::nearest_leverage(*leverage, &self.leverages(*side));
                    true
                }
                KeyCode::Backspace => {
                    *lots = (*lots / 10).max(1);
                    true
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    // Any lot count (preflight constraint C33a: no 10,000 cap); a
                    // number past u64 starts again from the digit.
                    let digit = u64::from(c as u8 - b'0');
                    *lots = lots
                        .checked_mul(10)
                        .and_then(|l| l.checked_add(digit))
                        .unwrap_or(digit)
                        .max(1);
                    true
                }
                _ => false,
            },
            Kind::Close { lots, max, .. } => match key.code {
                KeyCode::Up => {
                    *lots = (*lots + 1).min(*max);
                    true
                }
                KeyCode::Down => {
                    *lots = lots.saturating_sub(1).max(1);
                    true
                }
                _ => false,
            },
            Kind::Cancel { .. } => false,
        };
        if edited {
            ticket.stage = Stage::Edit;
        }
        self.ticket = Some(ticket);
        true
    }
}

fn muted() -> Style {
    Style::default().fg(MUTED)
}
fn strong() -> Style {
    Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
}

fn panel(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL))
        .title(Line::styled(format!(" {title} "), muted()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn put(frame: &mut Frame, area: Rect, y: u16, line: Line<'_>) {
    if y < area.bottom() {
        frame.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
    }
}

/// The market's price: its last trade on the price line, else the index.
fn last_price(m: &MarketView) -> String {
    opt_price(
        m.last_entry_value
            .as_deref()
            .or(m.index.as_ref().map(|i| i.value.as_str())),
    )
}

/// The market list in the sidebar slot; returns each shown market's area,
/// in order, for clicks.
pub fn sidebar(frame: &mut Frame, m: &Markets, area: Rect) -> Vec<Rect> {
    let inner = panel(frame, area, "MARKETS");
    let mut shown = Vec::new();
    if m.snap.markets.is_empty() {
        // Nothing while the first read is out, or failed (content says so).
        if m.loaded && m.snap.error.is_none() {
            put(
                frame,
                inner,
                inner.y + 1,
                Line::styled(" No markets.", muted()),
            );
        }
        return shown;
    }
    for (i, market) in m.snap.markets.iter().enumerate() {
        let y = inner.y + 1 + i as u16 * 3;
        if y + 2 > inner.bottom() {
            break;
        }
        let selected = i == m.selected;
        let bg = if selected { SELECTED } else { PANEL };
        let marker = if selected { "> " } else { "  " };
        let change = change(market.change24h_value.as_deref());
        let change_style = if change.starts_with('-') {
            Style::default().fg(RED)
        } else {
            Style::default().fg(GREEN)
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(
                        market.label.clone(),
                        if selected {
                            strong().fg(ACCENT)
                        } else {
                            strong()
                        },
                    ),
                ]),
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(last_price(market), Style::default().fg(TEXT)),
                    Span::styled(" ", muted()),
                    Span::styled(change, change_style),
                ]),
            ])
            .style(Style::default().bg(bg)),
            Rect::new(inner.x, y, inner.width, 2),
        );
        shown.push(Rect::new(inner.x, y, inner.width, 2));
    }
    shown
}

/// The selected market: header, price chart, book, trades, positions, orders.
/// Returns the open ticket's box, if there is one.
/// The shown market, with its oracle month's place in its life (phase.rs)
/// from `months` at `now` (the chain's clock).
pub fn content(
    frame: &mut Frame,
    m: &Markets,
    area: Rect,
    months: &[super::phase::Month],
    now: i64,
) -> Option<Rect> {
    let Some(market) = m.selected_market() else {
        // Nothing while the first read is out; offline if it failed.
        if m.snap.error.is_some() {
            put(frame, area, area.y, Line::styled("offline", muted()));
        }
        return None;
    };
    let mut y = area.y;
    put(
        frame,
        area,
        y,
        Line::from(vec![
            Span::styled(market.label.clone(), strong()),
            Span::styled(format!("  {}", status(market)), muted()),
            Span::styled(if m.offline { "  offline" } else { "" }, muted()),
            Span::styled("  index ", muted()),
            Span::styled(
                opt_price(market.index.as_ref().map(|i| i.value.as_str())),
                Style::default().fg(TEXT),
            ),
            Span::styled("  price ", muted()),
            Span::styled(opt_price(market.last_entry_value.as_deref()), strong()),
            Span::styled("  24h ", muted()),
            Span::styled(
                change(market.change24h_value.as_deref()),
                Style::default().fg(TEXT),
            ),
            Span::styled(format!("  {} open positions", market.claims_open), muted()),
        ]),
    );
    y += 1;
    // Its oracle month: the map, and the day it settles.
    let expiry = super::phase::expiry_id(&market.symbol, market.expiry_ts);
    if let Some(month) = months.iter().find(|o| o.expiry == expiry) {
        let mut line = vec![Span::styled("Oracle  ", muted())];
        line.extend(month.map(now).spans);
        if month.step() < 4
            && let Some(day) = chrono::DateTime::from_timestamp(market.expiry_ts as i64, 0)
        {
            line.push(Span::styled(
                format!("  settles {}", day.format("%b %-d")),
                muted(),
            ));
        }
        put(frame, area, y, Line::from(line));
        y += 1;
    }
    let width = usize::from(area.width.saturating_sub(16)).max(8);
    let (series, name) = if m.snap.closes.len() >= 12 || m.snap.index.len() < 2 {
        (&m.snap.closes, "5m")
    } else {
        (&m.snap.index, "index")
    };
    // The chart's high and low, on its right; taller in a taller window.
    let shown = &series[series.len().saturating_sub(width)..];
    let lo = shown.iter().copied().fold(f64::MAX, f64::min);
    let hi = shown.iter().copied().fold(f64::MIN, f64::max);
    let rows = match area.height {
        h if h >= 30 => 4,
        h if h >= 24 => 3,
        _ => 2,
    };
    for (i, line) in area_chart(series, width, rows).into_iter().enumerate() {
        let label = if i == 0 { name } else { "" };
        let axis = match i {
            _ if shown.is_empty() => String::new(),
            0 => format!(" {hi:.2}"),
            i if i + 1 == rows => format!(" {lo:.2}"),
            _ => String::new(),
        };
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(format!("{label:>5} "), muted()),
                Span::styled(line, Style::default().fg(ACCENT)),
                Span::styled(axis, muted()),
            ]),
        );
        y += 1;
    }
    y += 1;
    let account = m.account_market();
    let positions = account.map(|a| a.claims.len()).unwrap_or(0);
    let orders = account.map(|a| a.orders.len()).unwrap_or(0);
    let lower = (3 + positions.min(6) + orders.min(4) + usize::from(orders > 0)) as u16;
    let upper_height = area.bottom().saturating_sub(y).saturating_sub(lower).max(4);
    let book_width = 30.min(area.width / 2);
    let book_area = Rect::new(area.x, y, book_width, upper_height);
    let trades_area = Rect::new(
        area.x + book_width + 2,
        y,
        area.width.saturating_sub(book_width + 2),
        upper_height,
    );
    draw_book(frame, m, book_area);
    draw_trades(frame, m, trades_area);
    let lower_area = Rect::new(
        area.x,
        y + upper_height + 1,
        area.width,
        area.bottom().saturating_sub(y + upper_height + 1),
    );
    draw_account(frame, m, lower_area);
    m.ticket
        .as_ref()
        .map(|ticket| draw_ticket(frame, m, ticket, area))
}

/// The suggested range's book: sell offers (where you buy the range) above,
/// bids (where you sell it) below, the range and spread between.
fn draw_book(frame: &mut Frame, m: &Markets, area: Rect) {
    put(
        frame,
        area,
        area.y,
        Line::styled(
            format!("{:<6}{:>9}{:>7}{:>7}", "BOOK", "PRICE", "LOTS", "TOTAL"),
            muted(),
        ),
    );
    // Nothing until it is read.
    let Some(book) = &m.snap.book else {
        return;
    };
    let rows = usize::from(area.height.saturating_sub(2));
    let per_side = (rows / 2).max(1);
    let asks: Vec<_> = book.asks.iter().take(per_side).collect();
    let mut y = area.y + 1 + (per_side - asks.len()) as u16;
    for l in asks.iter().rev() {
        put(
            frame,
            area,
            y,
            Line::styled(
                format!(
                    "{:<6}{:>9}{:>7}{:>7}",
                    "ask",
                    price(&l.entry_value),
                    l.lots,
                    l.cumulative_lots
                ),
                Style::default().fg(RED),
            ),
        );
        y += 1;
    }
    let spread = match (book.asks.first(), book.bids.first()) {
        (Some(a), Some(b)) => match (a.entry_value.parse::<f64>(), b.entry_value.parse::<f64>()) {
            (Ok(a), Ok(b)) => format!(" · {:.2} wide", a - b),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let gap = format!(
        "{:─^29}",
        format!(" {}–{}{spread} ", book.lo_value, book.hi_value)
    );
    put(frame, area, y, Line::styled(gap, Style::default().fg(DIM)));
    y += 1;
    for l in book.bids.iter().take(per_side) {
        put(
            frame,
            area,
            y,
            Line::styled(
                format!(
                    "{:<6}{:>9}{:>7}{:>7}",
                    "bid",
                    price(&l.entry_value),
                    l.lots,
                    l.cumulative_lots
                ),
                Style::default().fg(GREEN),
            ),
        );
        y += 1;
    }
}

fn draw_trades(frame: &mut Frame, m: &Markets, area: Rect) {
    put(
        frame,
        area,
        area.y,
        Line::styled(
            format!(
                "{:<10}{:<6}{:>9}{:>6}  {}",
                "TRADES", "SIDE", "PRICE", "LOTS", "RANGE"
            ),
            muted(),
        ),
    );
    if m.snap.trades.is_empty() {
        put(frame, area, area.y + 1, Line::styled("none", muted()));
    }
    for (i, t) in m
        .snap
        .trades
        .iter()
        .take(usize::from(area.height.saturating_sub(1)))
        .enumerate()
    {
        let time = chrono::DateTime::<chrono::Utc>::from_timestamp(t.ts, 0)
            .map(|d| d.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        let side = if t.side == "sell" { "sell" } else { "buy" };
        let color = if side == "buy" { GREEN } else { RED };
        put(
            frame,
            area,
            area.y + 1 + i as u16,
            Line::from(vec![
                Span::styled(format!("{time:<10}"), muted()),
                Span::styled(
                    format!("{side:<6}{:>9}{:>6}", price(&t.entry_value), t.lots),
                    Style::default().fg(color),
                ),
                Span::styled(format!("  {}–{}", t.lo_value, t.hi_value), muted()),
            ]),
        );
    }
}

/// A position's leverage at its entry, as the site shows it.
fn leverage(grid: Option<&Grid>, side: &str, entry: &str, lo: u16, hi: u16) -> String {
    let side = if side == "short" {
        Side::Short
    } else {
        Side::Long
    };
    grid.and_then(|g| g.pos_f64(entry).ok())
        .and_then(|at| shape::leverage_at(side, at, Shape { lo, hi }))
        .map(|l| format!("{l:.0}x"))
        .unwrap_or_else(|| "–".into())
}

fn draw_account(frame: &mut Frame, m: &Markets, area: Rect) {
    let mut y = area.y;
    let Some(account) = m.account_market() else {
        let text = match &m.snap.owner {
            None => "No wallet",
            Some(_) => "No positions",
        };
        put(frame, area, y, Line::styled(text, muted()));
        return;
    };
    let grid = m.selected_market().and_then(|v| Grid::of_view(v).ok());
    put(
        frame,
        area,
        y,
        Line::from(vec![
            Span::styled("POSITIONS", muted()),
            Span::styled(
                format!(
                    "   free {} · reserved {} · wallet {}",
                    money(atoms(&account.trader.free)),
                    money(atoms(&account.trader.reserved)),
                    m.snap
                        .account
                        .as_ref()
                        .map(|a| money(atoms(&a.usdc)))
                        .unwrap_or_else(|| "–".into()),
                ),
                muted(),
            ),
        ]),
    );
    y += 1;
    if account.claims.is_empty() {
        put(frame, area, y, Line::styled("  none", muted()));
        y += 1;
    }
    for (i, c) in account.claims.iter().take(6).enumerate() {
        let pnl = c
            .pnl_at_index
            .as_deref()
            .map(|v| signed_money(atoms(v)))
            .unwrap_or_else(|| "–".into());
        let color = if pnl.starts_with('-') { RED } else { GREEN };
        let selected = i == m.position;
        put(
            frame,
            area,
            y,
            Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    format!(
                        "#{:<4}{:<6}{:>4} {:>5} lots @ {:<7} range {}–{}  ",
                        c.slot,
                        c.side,
                        leverage(grid.as_ref(), &c.side, &c.entry_value, c.lo, c.hi),
                        c.lots,
                        price(&c.entry_value),
                        c.lo_value,
                        c.hi_value
                    ),
                    if selected {
                        strong()
                    } else {
                        Style::default().fg(TEXT)
                    },
                ),
                Span::styled(format!("P&L {pnl}"), Style::default().fg(color)),
                Span::styled(
                    c.exit
                        .as_ref()
                        .map(|e| format!("  close {}", money(atoms(&e.net))))
                        .unwrap_or_default(),
                    muted(),
                ),
            ])
            .style(Style::default().bg(if selected { SELECTED } else { PANEL })),
        );
        y += 1;
    }
    if !account.orders.is_empty() {
        put(frame, area, y, Line::styled("ORDERS", muted()));
        y += 1;
        for (i, o) in account.orders.iter().take(4).enumerate() {
            let selected = i == m.order;
            put(
                frame,
                area,
                y,
                Line::from(vec![
                    Span::styled(
                        if selected { "> " } else { "  " },
                        Style::default().fg(ACCENT),
                    ),
                    Span::styled(
                        format!(
                            "#{:<4}{:<12}{:>5} lots @ {:<7} range {}–{}  {}",
                            o.slot,
                            format!("{}{}", o.side, if o.close { " close" } else { "" }),
                            o.lots,
                            price(&o.entry_value),
                            o.lo_value,
                            o.hi_value,
                            if o.close {
                                format!(
                                    "holds position #{}",
                                    o.claim.as_ref().map(|c| c.slot).unwrap_or_default()
                                )
                            } else {
                                format!("holds {}", money(atoms(&o.escrow)))
                            }
                        ),
                        if selected {
                            strong()
                        } else {
                            Style::default().fg(TEXT)
                        },
                    ),
                ]),
            );
            y += 1;
        }
    }
}

/// Rows `lines` take wrapped at `width`.
fn rows(lines: &[Line], width: u16) -> u16 {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|l| l.width().div_ceil(width).max(1) as u16)
        .sum()
}

/// Draws the ticket; returns its box.
fn draw_ticket(frame: &mut Frame, m: &Markets, ticket: &Ticket, area: Rect) -> Rect {
    let title = match &ticket.kind {
        Kind::Take {
            side: SideArg::Long,
            ..
        } => "LONG",
        Kind::Take { .. } => "SHORT",
        Kind::Close { .. } => "CLOSE",
        Kind::Cancel { .. } => "CANCEL",
    };
    let mut lines: Vec<Line> = Vec::new();
    match &ticket.kind {
        Kind::Take {
            side,
            lots,
            leverage,
        } => {
            let levs = m.leverages(*side);
            lines.push(Line::from(vec![
                Span::styled("side ", muted()),
                Span::styled(
                    shape::side_word(trade::side_of(*side)),
                    strong().fg(if *side == SideArg::Long { GREEN } else { RED }),
                ),
                Span::styled("   lots ", muted()),
                Span::styled(lots.to_string(), strong()),
                Span::styled("   leverage ", muted()),
                Span::styled(format!("{leverage}x"), strong()),
                Span::styled(
                    format!(
                        "  of {}",
                        levs.iter()
                            .map(|l| format!("{l}x"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    muted(),
                ),
            ]));
        }
        Kind::Close { slot, lots, max } => lines.push(Line::from(vec![
            Span::styled(format!("position #{slot}  lots "), muted()),
            Span::styled(format!("{lots}"), strong()),
            Span::styled(format!(" of {max}"), muted()),
        ])),
        Kind::Cancel { text, .. } => lines.push(Line::styled(text.clone(), strong())),
    }
    let hint = match &ticket.stage {
        Stage::Edit => match ticket.kind {
            Kind::Take { .. } => "↑/↓ 0-9 lots · ←/→ leverage · Tab side · Enter quote · Esc",
            Kind::Close { .. } => "↑/↓ lots · Enter quote · Esc",
            Kind::Cancel { .. } => "Enter review · Esc",
        },
        Stage::Working => "",
        Stage::Quoted(q) => {
            lines.push(Line::raw(""));
            lines.extend(
                q.iter()
                    .map(|l| Line::styled(l.clone(), Style::default().fg(TEXT))),
            );
            "y send · n/Esc close"
        }
        Stage::Done(d) => {
            lines.push(Line::raw(""));
            lines.extend(
                d.iter()
                    .map(|l| Line::styled(l.clone(), Style::default().fg(GREEN))),
            );
            "any key closes"
        }
        Stage::Failed(e) => {
            lines.push(Line::raw(""));
            lines.push(Line::styled(e.clone(), Style::default().fg(ALERT)));
            "any key closes"
        }
    };
    let width = area.width.min(64);
    // Borders, the hint row, and the text wrapped inside the borders.
    let height = (rows(&lines, width.saturating_sub(4)) + 3)
        .max(5)
        .min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.bottom().saturating_sub(height),
        width,
        height,
    );
    scrim(frame.buffer_mut(), rect);
    frame.render_widget(Clear, rect);
    let inner = panel(frame, rect, &format!("{title} · {}", ticket.label));
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

/// Footer key hints for the Markets view.
pub fn footer_hint() -> Vec<(&'static str, &'static str)> {
    vec![
        ("↑/↓", " Markets"),
        ("b/s", " Long/Short"),
        ("x", " Close"),
        ("d", " Cancel"),
        ("p/o", " Select"),
        ("q", " Quit"),
    ]
}
