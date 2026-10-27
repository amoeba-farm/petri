//! The Bots view: trading bots people build and run inside petri
//! (`crate::bots`). A background worker owns the bot host. It reads the
//! bundles in petri's `bots/` folder (picking up a file's edit while its bot
//! runs), starts and stops bots, and every two seconds (the house maker's
//! `MAKER_INTERVAL_MS`) reads their markets' feeds, runs them, and checks,
//! signs and sends what they propose. The view lists the bots, starts and
//! stops them and edits their weights; a new bot comes from a template for
//! the market picked.
//!
//! Each bot trades from a wallet of its own (`bots::wallet`): starting it
//! tops that wallet up to its budget from petri's wallet, registers the run
//! with the futures API's registry, and stopping it takes its ladder and
//! orders down and moves its USDC back (`docs/bots.md`).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
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
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use super::{
    model::Action,
    render::{ACCENT, ALERT, BORDER, DIM, MUTED, PANEL, SELECTED, TEXT},
};
use crate::{
    bots::{
        bundle::{self, Bundle},
        feeds::{Holding, Intent, Outcome, Position},
        host::{self, Host, Part},
        registry::{self, Pnl},
        send, wallet,
        weights::{KNOBS, Weights},
    },
    compose::State,
    futures::{
        Chain, Funds,
        api::{self, MarketView},
        ctx::Ctx,
        error::Error,
        exec, money,
        settings::Settings,
        shape::Grid,
        slug,
    },
};

const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(122, 178, 132);

/// How often running bots look at their markets.
const EVERY: Duration = Duration::from_secs(2);

/// How often the registry's P&L of the bots is read, at most
/// (`docs/bots.md`: "read at most every 30 s while the list is served").
const PNL_EVERY: Duration = Duration::from_secs(30);

/// What a bot is quoting, as the view shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteView {
    /// The ladder's range and its full-depth core, as index values.
    pub range: String,
    pub core: String,
    pub lots: u64,
    pub outer: u64,
    pub tiers: u8,
    /// Half the spread, in percent of a step.
    pub spread: f64,
    /// Smaller than its weights ask, to fit the book's room.
    pub fitted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BotView {
    pub id: String,
    pub file: String,
    /// Its shown name (`[bot] name`, else its id) and its one line.
    pub name: String,
    pub about: Option<String>,
    /// The registry id it was installed or published as.
    pub from: Option<String>,
    /// Its own wallet's public key, once it has one.
    pub wallet: Option<String>,
    /// Its rows' kinds (`script`, `ladder`, ...).
    pub kinds: Vec<String>,
    /// Its market as written, and its label once found.
    pub market: String,
    pub label: Option<String>,
    pub running: bool,
    /// Why it can't run as written, in plain words.
    pub problem: Option<String>,
    /// Its market's last read failed: what is shown is the read before.
    pub offline: bool,
    /// Each of [`KNOBS`]' value, as the bot runs with it.
    pub weights: Vec<String>,
    /// The USDC (atoms) its wallet gets on start.
    pub budget: Option<u64>,
    pub parts: Vec<Part>,
    pub quote: Option<QuoteView>,
    pub outcome: Option<(Outcome, Instant)>,
    pub position: Option<Position>,
    /// Its wallet's open positions and resting orders in its market: read
    /// every look while it runs, from the API's account otherwise.
    pub holdings: Vec<Holding>,
    pub orders: u32,
    /// Every month it holds, each holding with its market's label.
    pub held: Vec<(String, Holding)>,
    /// Its run as the registry sees it, once read.
    pub pnl: Option<Pnl>,
    /// Why it does nothing now, if it says.
    pub waiting: Option<String>,
}

enum Command {
    /// Starts a bot, with this budget (USDC) saved to its file first.
    Start {
        id: String,
        budget: Option<f64>,
    },
    Stop(String),
    Set {
        bot: String,
        knob: String,
        value: String,
    },
    /// A new bot on the market with this command-line name.
    New(String),
    /// Saves a bundle's text as its bot's file.
    Write(String),
    /// Stops a bot and deletes its file.
    Remove(String),
    Publish {
        id: String,
        /// The GitHub file that holds it, `OWNER/REPO[/PATH][@REF]`.
        github: Option<String>,
    },
    Install {
        id: String,
        market: Option<String>,
    },
    /// Stops every bot and takes its ladder down.
    Shutdown,
}

enum Reply {
    Bots(Vec<BotView>),
    Note(String),
    Created(String),
    Written(String),
    Removed(String),
    Published(Value),
    Installed(String),
    Down,
}

fn err(e: Error) -> String {
    super::markets::plain(e.message.replace('\n', " · "))
}

/// A knob's value as `w` has it.
fn knob(w: &Weights, name: &str) -> String {
    match name {
        "budget" => w
            .budget
            .map_or("all".into(), |b| format!("{}", b as f64 / 1e6)),
        "fraction" => format!("{}", w.fraction),
        "spread_bps" => w.spread_bps.to_string(),
        "vol" => format!("{}", w.vol),
        "half_width" => w.half_width.to_string(),
        "tiers" => w.tiers.to_string(),
        "tier_bps" => w.tier_bps.to_string(),
        "tier_ratio" => w.tier_ratio.to_string(),
        "skew" => w.skew.to_string(),
        _ => String::new(),
    }
}

/// Every row of `b` that runs builds (scripts compile), or why not.
fn builds(b: &Bundle) -> Result<(), String> {
    for row in b.rows.iter().filter(|r| !r.disabled) {
        host::build(row, &b.market).map_err(|e| format!("{}: {e}", row.id))?;
    }
    Ok(())
}

/// `text` squashed to letters and digits, lowercase (as market names are
/// matched).
fn squash(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

struct File {
    path: PathBuf,
    text: String,
    bundle: Result<Bundle, String>,
}

/// A running bot's wallet: what its sends sign with.
struct Wallet {
    ctx: Ctx,
    key: Pubkey,
}

struct Worker {
    settings: Settings,
    ctx: Ctx,
    host: Host,
    folder: Result<PathBuf, String>,
    files: BTreeMap<String, File>,
    /// Each started bot's market: its address, its label and the market as
    /// its file named it.
    markets: BTreeMap<String, (String, String, String)>,
    /// The text each running bot was last started or updated from.
    texts: BTreeMap<String, String>,
    problems: BTreeMap<String, String>,
    outcomes: BTreeMap<String, (Outcome, Instant)>,
    quotes: BTreeMap<String, QuoteView>,
    offline: BTreeSet<String>,
    /// Each market's label and address, by the name a file gives it.
    places: BTreeMap<String, (String, String)>,
    /// A lock on each running bot, so two petris (the TUI and `petri bots
    /// serve`) never run one bot on one wallet at once.
    locks: BTreeMap<String, std::fs::File>,
    /// Each running bot's wallet.
    wallets: BTreeMap<String, Wallet>,
    /// What each running bot's run was registered as (its registry id and
    /// hash); a bot whose registration failed is not here, and is tried
    /// again with the next P&L read.
    registered: BTreeMap<String, (Option<String>, String, String)>,
    /// The markets whose index history was warmed from the API.
    warmed: BTreeSet<String>,
    /// Each bot's run as the registry last said, by bot id.
    pnl: BTreeMap<String, Pnl>,
    /// Each bot's wallet as the API's account last showed it (every look
    /// while it runs, with the P&L otherwise).
    accounts: BTreeMap<String, api::Account>,
    /// The product each running product bot rolls in (`nandx`: the front
    /// open month, the next one when it stops trading).
    products: BTreeMap<String, String>,
    /// Each running bot's last failure to bring its cash home, so a failing
    /// look says it once.
    tidied: BTreeMap<String, String>,
    pnl_read: Option<Instant>,
    /// Each bot's wallet's public key, once it has a wallet (petri never
    /// deletes one, so a key once read stays true).
    keys: BTreeMap<String, Pubkey>,
}

impl Worker {
    fn new(settings: Settings) -> Self {
        Self {
            ctx: Ctx::new(settings.clone()),
            settings,
            host: Host::new(),
            folder: bundle::folder(),
            files: BTreeMap::new(),
            markets: BTreeMap::new(),
            texts: BTreeMap::new(),
            problems: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            quotes: BTreeMap::new(),
            offline: BTreeSet::new(),
            places: BTreeMap::new(),
            locks: BTreeMap::new(),
            wallets: BTreeMap::new(),
            registered: BTreeMap::new(),
            warmed: BTreeSet::new(),
            pnl: BTreeMap::new(),
            accounts: BTreeMap::new(),
            products: BTreeMap::new(),
            tidied: BTreeMap::new(),
            pnl_read: None,
            keys: BTreeMap::new(),
        }
    }

    /// Holds bot `id`'s lock while it runs. Another petri running it (the
    /// TUI, or `petri bots serve` under an agent) would post a second ladder
    /// on the same wallet, so the second start refuses.
    /// Returns whether this call took it (false: this petri already held it).
    fn lock(&mut self, id: &str) -> Result<bool, String> {
        if self.locks.contains_key(id) {
            return Ok(false);
        }
        let folder = self.folder.clone()?.join(".locks");
        crate::petri_config::create_private_dir_all(&folder)
            .map_err(|e| format!("couldn't make {}: {e}", folder.display()))?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder.join(format!("{id}.lock")))
            .map_err(|e| format!("couldn't lock the bot: {e}"))?;
        match file.try_lock() {
            Ok(()) => {
                self.locks.insert(id.into(), file);
                Ok(true)
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                Err("it runs in another petri; stop it there first".into())
            }
            Err(std::fs::TryLockError::Error(e)) => Err(format!("couldn't lock the bot: {e}")),
        }
    }

    /// Reads the folder again. A running bot whose file changed runs the
    /// change (only its changed rows restart); one whose file no longer
    /// builds keeps running as it was, and says why.
    fn scan(&mut self) {
        let Ok(folder) = &self.folder else {
            return;
        };
        let mut files = BTreeMap::new();
        for (path, text, bundle) in bundle::read_all(folder) {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let (id, bundle) = match bundle {
                Ok(b) if files.contains_key(&b.id) => {
                    (stem, Err(format!("another bot is called {}", b.id)))
                }
                Ok(b) => (b.id.clone(), Ok(b)),
                Err(e) => (stem, Err(e)),
            };
            if let Ok(b) = &bundle
                && !self.places.contains_key(&b.market)
                && let Ok(m) = crate::futures::resolve(&self.ctx, &b.market, None)
            {
                self.places.insert(b.market.clone(), (m.label, m.market));
            }
            if !self.keys.contains_key(&id)
                && let Some(key) = wallet::pubkey(folder, &id)
            {
                self.keys.insert(id.clone(), key);
            }
            files.insert(id, File { path, text, bundle });
        }
        self.files = files;
        let running: Vec<String> = self.texts.keys().cloned().collect();
        for id in running {
            let Some(f) = self.files.get(&id) else {
                self.stop(&id);
                continue;
            };
            if self.texts.get(&id) == Some(&f.text) {
                continue;
            }
            let text = f.text.clone();
            match f.bundle.clone() {
                Err(e) => {
                    self.problems.insert(id.clone(), e);
                }
                // Another market: another bot.
                Ok(b) if self.markets.get(&id).is_some_and(|m| m.2 != b.market) => {
                    self.stop(&id);
                    self.start(&id);
                }
                Ok(b) => match self.host.update(&id, &b.rows, b.budget_atoms()) {
                    Ok(()) => {
                        self.problems.remove(&id);
                        // Another strategy (or now published): its run
                        // counts for what it is now.
                        self.register(&id, &b);
                    }
                    Err(e) => {
                        self.problems.insert(id.clone(), e);
                    }
                },
            }
            if self.host.running(&id) {
                self.texts.insert(id, text);
            }
        }
    }

    /// Starts bot `id`: its wallet (made on its first start) is topped up
    /// to its budget from petri's wallet, its run registered, and then it
    /// runs.
    fn start(&mut self, id: &str) {
        self.problems.remove(id);
        let bundle = match self.files.get(id).map(|f| f.bundle.clone()) {
            Some(Ok(b)) => b,
            Some(Err(e)) => {
                self.problems.insert(id.into(), e);
                return;
            }
            None => return,
        };
        if let Err(e) = self.ctx.wallet_pubkey() {
            self.problems.insert(id.into(), err(e));
            return;
        }
        // A product bot starts in its front month that trades now, if one
        // does; a month is the month it names. One running already stays
        // where it runs.
        let product = self.product(&bundle.market);
        let here = self
            .markets
            .get(id)
            .filter(|_| self.host.running(id))
            .map(|m| m.0.clone());
        let front = match (&here, &product) {
            (Some(_), _) | (None, None) => None,
            (None, Some(p)) => self.front(p),
        };
        let named = here.as_deref().unwrap_or(&bundle.market);
        let m = match front.map_or_else(|| crate::futures::resolve(&self.ctx, named, None), Ok) {
            Ok(m) => m,
            Err(e) => {
                self.problems.insert(id.into(), err(e));
                return;
            }
        };
        if let Err(e) = builds(&bundle) {
            self.problems.insert(id.into(), e);
            return;
        }
        self.outcomes.remove(id);
        self.quotes.remove(id);
        let took = match self.lock(id) {
            Ok(took) => took,
            Err(e) => {
                self.problems.insert(id.into(), e);
                return;
            }
        };
        let funded = self
            .wallet(id)
            .and_then(|()| self.fund(id, &m.market, bundle.budget_atoms()));
        if let Err(e) = funded {
            if took {
                self.locks.remove(id);
            }
            if !self.host.running(id) {
                self.wallets.remove(id);
            }
            self.problems.insert(id.into(), e);
            return;
        }
        let was = self.markets.insert(
            id.into(),
            (m.market.clone(), slug(&m), bundle.market.clone()),
        );
        self.register(id, &bundle);
        match self
            .host
            .start(id, &m.market, &bundle.rows, bundle.budget_atoms())
        {
            Ok(()) => {
                if let Some(f) = self.files.get(id) {
                    self.texts.insert(id.into(), f.text.clone());
                }
                match product {
                    Some(p) => self.products.insert(id.into(), p),
                    None => self.products.remove(id),
                };
                self.warm(&m.market);
                if let Ok(folder) = &self.folder {
                    let _ = bundle::set_running(folder, id, true);
                }
            }
            Err(e) => {
                if took {
                    self.locks.remove(id);
                }
                match was {
                    Some(was) => self.markets.insert(id.into(), was),
                    None => self.markets.remove(id),
                };
                if !self.host.running(id) {
                    self.wallets.remove(id);
                    self.registered.remove(id);
                }
                self.problems.insert(id.into(), e);
            }
        }
    }

    /// The product `market` names, when it names one rather than a month
    /// (`nandx`, not `nandx-nov26`).
    fn product(&self, market: &str) -> Option<String> {
        let wanted = squash(market);
        crate::futures::markets(&self.ctx)
            .ok()?
            .iter()
            .find(|m| squash(&m.symbol) == wanted)
            .map(|m| m.symbol.clone())
    }

    /// `product`'s front month that trades now: the tradable one expiring
    /// first, read fresh.
    fn front(&self, product: &str) -> Option<MarketView> {
        api::markets(&self.ctx.api)
            .ok()?
            .into_iter()
            .filter(|m| m.tradable && squash(&m.symbol) == squash(product))
            .min_by_key(|m| m.expiry_ts)
    }

    /// Moves each running product bot whose month stopped trading to the
    /// product's front month, if another trades now. What it holds in the
    /// old month stays there until that month settles ([`Worker::tidy`]
    /// brings its cash home).
    fn roll(&mut self) {
        let feeds = self.host.markets();
        let stopped = |market: &str| {
            feeds
                .iter()
                .find(|(m, _)| m == market)
                .and_then(|(_, f)| f.book.get())
                .is_some_and(|b| !b.tradable)
        };
        let due: Vec<(String, String)> = self
            .products
            .iter()
            .filter(|(id, _)| self.markets.get(*id).is_some_and(|(m, _, _)| stopped(m)))
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect();
        for (id, product) in due {
            let Some(next) = self.front(&product) else {
                continue;
            };
            let Some((was, _, written)) = self.markets.get(&id).cloned() else {
                continue;
            };
            let Some(Ok(b)) = self.files.get(&id).map(|f| f.bundle.clone()) else {
                continue;
            };
            if next.market == was {
                continue;
            }
            if let Err(e) = self.host.roll(&id, &next.market, &b.rows) {
                self.problems.insert(id.clone(), e);
                continue;
            }
            self.markets
                .insert(id.clone(), (next.market.clone(), slug(&next), written));
            self.quotes.remove(&id);
            self.register(&id, &b);
            self.warm(&next.market);
        }
    }

    /// Starts again the bots that ran when petri last quit (`bots/.running`),
    /// each under its lock: one another petri runs is left to it.
    fn resume(&mut self) {
        let Ok(folder) = self.folder.clone() else {
            return;
        };
        for id in bundle::running(&folder) {
            if !self.files.contains_key(&id) {
                let _ = bundle::set_running(&folder, &id, false);
                continue;
            }
            if self.lock(&id).is_ok() {
                self.start(&id);
            }
        }
    }

    /// Brings each running bot's cash home from the markets it no longer
    /// trades: settled where it can be, withdrawn to its wallet
    /// (`docs/bots.md`, "Product bots roll").
    fn tidy(&mut self) {
        let ids: Vec<String> = self.wallets.keys().cloned().collect();
        for id in ids {
            let (Some(w), Some(account)) = (self.wallets.get(&id), self.accounts.get(&id)) else {
                continue;
            };
            let current = self.markets.get(&id).map(|m| m.0.as_str());
            let Ok(signer) = w.ctx.signer() else {
                continue;
            };
            match send::tidy(&w.ctx, signer.as_ref(), account, current) {
                Ok(()) => {
                    self.tidied.remove(&id);
                }
                Err(e) if self.tidied.get(&id) != Some(&e) => {
                    self.outcomes.insert(
                        id.clone(),
                        (
                            Outcome::Refused(format!("Couldn't bring its cash home: {e}")),
                            Instant::now(),
                        ),
                    );
                    self.tidied.insert(id, e);
                }
                Err(_) => {}
            }
        }
    }

    /// Opens bot `id`'s wallet (making it on its first start) for its sends.
    fn wallet(&mut self, id: &str) -> Result<(), String> {
        if self.wallets.contains_key(id) {
            return Ok(());
        }
        let (path, key) = wallet::open(&self.folder()?, id)?;
        self.keys.insert(id.into(), key);
        let ctx = Ctx::new(wallet::settings(&self.settings, &path));
        self.wallets.insert(id.into(), Wallet { ctx, key });
        Ok(())
    }

    /// Tops bot `id`'s wallet up to `budget` (its wallet's USDC and its free
    /// cash in `market` count) and to its SOL float, from petri's wallet.
    fn fund(&self, id: &str, market: &str, budget: u64) -> Result<(), String> {
        let bot = self.wallets.get(id).ok_or("its wallet is not open")?.key;
        let address: Pubkey = market
            .parse()
            .map_err(|_| format!("{market} is not a market address"))?;
        let chain = Chain::load(&self.ctx, &address).map_err(err)?;
        let free = chain.book.trader_of(&bot).map_or(0, |(_, t)| t.free);
        let held = Funds::load(&self.ctx, &bot).map_err(err)?;
        let f = wallet::funding(budget, held.usdc, free, held.sol);
        if f.none() {
            return Ok(());
        }
        let signer = self.ctx.signer().map_err(err)?;
        let main = signer.pubkey();
        let funds = Funds::load(&self.ctx, &main).map_err(err)?;
        if f.usdc > funds.usdc {
            return Err(format!(
                "Its budget needs {} more USDC than your wallet has.",
                money(i128::from(f.usdc - funds.usdc))
            ));
        }
        let plan = wallet::funding_plan(&main, &bot, &chain.market.mint, f, held.exists);
        exec::send_local(&self.ctx, signer.as_ref(), &plan).map_err(err)?;
        Ok(())
    }

    /// Registers bot `id`'s run with the registry, signed by its wallet and
    /// petri's, unless it is registered as it is already (the same registry
    /// id, hash and market). A failure (an API without the registry, a
    /// wallet that can't sign) never stops the bot: it has no P&L until a
    /// later try lands.
    fn register(&mut self, id: &str, b: &Bundle) {
        let Some(w) = self.wallets.get(id) else {
            return;
        };
        let Some(market) = self
            .markets
            .get(id)
            .map(|m| m.0.clone())
            .or_else(|| self.places.get(&b.market).map(|p| p.1.clone()))
        else {
            return;
        };
        // The registry id it came from: a copy from GitHub names none.
        let from = b.from.clone().filter(|f| !f.starts_with(registry::GITHUB));
        let now = (from, b.hash(), market.clone());
        if self.registered.get(id) == Some(&now) {
            return;
        }
        let (Ok(wallet), Ok(owner)) = (w.ctx.signer(), self.ctx.signer()) else {
            return;
        };
        let run = registry::Registration {
            bot: now.0.as_deref(),
            hash: &now.1,
            name: b.name(),
            market: &market,
        };
        if registry::register(&self.ctx.api, wallet.as_ref(), owner.as_ref(), &run).is_ok() {
            self.registered.insert(id.into(), now);
        } else {
            self.registered.remove(id);
        }
    }

    /// Warms `market`'s index history from the API once: its last day of
    /// samples, one a minute.
    fn warm(&mut self, market: &str) {
        if self.warmed.contains(market) {
            return;
        }
        let Some((_, feeds)) = self.host.markets().into_iter().find(|(m, _)| m == market) else {
            return;
        };
        if let Ok(points) = api::index(&self.ctx.api, market, 5_000) {
            let mut history = send::warm(&points, send::wall());
            // Its own reads since, if any, follow.
            for s in feeds.history.get().iter().flat_map(|h| h.iter()) {
                send::record(&mut history, *s);
            }
            feeds.history.set(history);
            self.warmed.insert(market.to_string());
        }
    }

    /// Stops bot `id`: its ladder comes down (the ladder's own cancel) and
    /// then its wallet is swept: resting orders cancelled, free cash
    /// withdrawn, and all its USDC moved back to petri's wallet. Its open
    /// positions stay in its wallet, and its SOL float for the next run.
    fn stop(&mut self, id: &str) {
        let running = self.host.running(id);
        self.host.stop(id);
        self.texts.remove(id);
        self.problems.remove(id);
        if !running {
            return;
        }
        // Its ladder's cancel goes out now, before the sweep.
        self.send_all();
        if let Some(market) = self.markets.get(id).map(|m| m.0.clone())
            && let Err(e) = self.sweep(id, &market)
        {
            self.outcomes.insert(
                id.into(),
                (
                    Outcome::Refused(format!("Couldn't move its cash back: {e}")),
                    Instant::now(),
                ),
            );
        }
        self.wallets.remove(id);
        self.registered.remove(id);
        self.products.remove(id);
        self.tidied.remove(id);
        self.accounts.remove(id);
    }

    fn sweep(&self, id: &str, market: &str) -> Result<(), String> {
        let w = self.wallets.get(id).ok_or("its wallet is not open")?;
        let signer = w.ctx.signer().map_err(err)?;
        send::sweep_market(&w.ctx, signer.as_ref(), market)?;
        // Its cash in the other markets it traded (a product bot's earlier
        // months) comes home too.
        if let Ok(account) = api::account(&w.ctx.api, &w.key.to_string()) {
            send::tidy(&w.ctx, signer.as_ref(), &account, None)?;
        }
        let main = self.ctx.wallet_pubkey().map_err(err)?;
        let held = Funds::load(&w.ctx, &w.key).map_err(err)?;
        let home = Funds::load(&w.ctx, &main).map_err(err)?;
        let mint = w.ctx.deployment().map_err(err)?.usdc_mint;
        if let Some(plan) = wallet::sweep_plan(&w.key, &main, &mint, held.usdc, home.exists) {
            exec::send_local(&w.ctx, signer.as_ref(), &plan).map_err(err)?;
        }
        Ok(())
    }

    /// Lets go of the locks of bots that no longer run, once what they
    /// proposed on their way out (the ladder's cancel) has been sent.
    fn release(&mut self) {
        let host = &self.host;
        self.locks.retain(|id, _| host.running(id));
    }

    /// Sets a weight in the bot's file, if the bot builds with it (and, if
    /// it runs, runs with it).
    fn set(&mut self, id: &str, name: &str, value: &str) -> Result<(), String> {
        let f = self.files.get(id).ok_or("that bot is gone")?;
        let mut b = f.bundle.clone()?;
        b.set(name, value)?;
        if self.host.running(id) {
            self.host.update(id, &b.rows, b.budget_atoms())?;
        } else {
            builds(&b)?;
        }
        let text = b.text();
        std::fs::write(&f.path, &text).map_err(|e| format!("couldn't save the bot: {e}"))?;
        if self.host.running(id) {
            self.texts.insert(id.into(), text.clone());
            self.problems.remove(id);
        }
        let f = self.files.get_mut(id).expect("the bot's file");
        f.text = text;
        f.bundle = Ok(b);
        Ok(())
    }

    /// Bot `id`'s file's text as last read.
    fn text_of(&self, id: &str) -> Result<String, String> {
        self.files
            .get(id)
            .map(|f| f.text.clone())
            .ok_or_else(|| format!("there is no bot called {id}"))
    }

    /// The bots folder, made for the owner alone if it is not there.
    fn folder(&self) -> Result<PathBuf, String> {
        let folder = self.folder.clone()?;
        crate::petri_config::create_private_dir_all(&folder)
            .map_err(|e| format!("couldn't make {}: {e}", folder.display()))?;
        Ok(folder)
    }

    /// Writes a new ladder bot for `market` (a command-line name).
    fn create(&mut self, market: &str) -> Result<String, String> {
        let folder = self.folder()?;
        let (id, path) = bundle::fresh(&folder, &format!("ladder-{market}"));
        std::fs::write(&path, bundle::template(&id, market))
            .map_err(|e| format!("couldn't save the bot: {e}"))?;
        self.scan();
        Ok(id)
    }

    /// Saves bundle `text` as its bot's file (the file that bot has, else
    /// `bots/<id>.toml`) once it parses and every row builds; a running bot
    /// picks it up.
    fn write(&mut self, text: &str) -> Result<String, String> {
        let b = Bundle::parse(text)?;
        builds(&b)?;
        let folder = self.folder()?;
        let path = self
            .files
            .get(&b.id)
            .map(|f| f.path.clone())
            .unwrap_or_else(|| folder.join(format!("{}.toml", b.id)));
        // Never over another bot's file.
        if let Some((other, _)) = self
            .files
            .iter()
            .find(|(id, f)| f.path == path && **id != b.id)
        {
            return Err(format!(
                "{} holds the bot {other}; give this one another id",
                path.display()
            ));
        }
        std::fs::write(&path, text).map_err(|e| format!("couldn't save the bot: {e}"))?;
        self.scan();
        Ok(b.id)
    }

    /// Stops bot `id` (here; a petri running it elsewhere stops it when it
    /// sees the file gone) and deletes its file. Its wallet stays.
    fn remove(&mut self, id: &str) -> Result<(), String> {
        let path = self
            .files
            .get(id)
            .map(|f| f.path.clone())
            .ok_or_else(|| format!("there is no bot called {id}"))?;
        self.stop(id);
        self.release();
        bundle::set_running(&self.folder()?, id, false)?;
        std::fs::remove_file(&path).map_err(|e| format!("couldn't delete the bot: {e}"))?;
        self.scan();
        Ok(())
    }

    /// Publishes bot `id` to the registry, signed by petri's wallet (its
    /// `author`, written into the file first when it names none), naming
    /// the GitHub file that holds it when given; then sets its `from` to
    /// the registry id.
    fn publish(&mut self, id: &str, github: Option<&str>) -> Result<Value, String> {
        let github = github
            .map(registry::Github::parse)
            .transpose()
            .map_err(err)?;
        let f = self
            .files
            .get(id)
            .ok_or_else(|| format!("there is no bot called {id}"))?;
        let mut b = f.bundle.clone()?;
        let path = f.path.clone();
        builds(&b)?;
        let signer = self.ctx.signer().map_err(err)?;
        if b.author.is_none() {
            b.set_bot("author", &signer.pubkey().to_string())?;
            std::fs::write(&path, b.text()).map_err(|e| format!("couldn't save the bot: {e}"))?;
            self.scan();
        }
        let v = registry::publish(
            &self.ctx.api,
            signer.as_ref(),
            &b.publish_text(),
            &b.hash(),
            github.as_ref(),
        )
        .map_err(err)?;
        let reg = v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(&b.id)
            .to_string();
        if b.from.as_deref() != Some(reg.as_str()) {
            b.set_bot("from", &reg)?;
            std::fs::write(&path, b.text()).map_err(|e| format!("couldn't save the bot: {e}"))?;
            self.scan();
        }
        Ok(json!({
            "id": reg,
            "version": v.get("version").cloned().unwrap_or(Value::Null),
            "hash": v.get("hash").cloned().unwrap_or_else(|| b.hash().into()),
        }))
    }

    /// An id no bot here has: `id`, else `id-2`, `id-3`…
    fn unique(&self, folder: &Path, id: &str) -> String {
        let mut n = 1;
        loop {
            let name = if n == 1 {
                id.to_string()
            } else {
                format!("{id}-{n}")
            };
            if !self.files.contains_key(&name) && !folder.join(format!("{name}.toml")).exists() {
                return name;
            }
            n += 1;
        }
    }

    /// Writes published bot `reg`'s latest version as a new bot here (or,
    /// for `github:OWNER/REPO[/PATH][@REF]`, the file on GitHub): `from`
    /// set, on `market` when given, its id made unique.
    fn install(&mut self, reg: &str, market: Option<&str>) -> Result<String, String> {
        let (text, from) = if reg.trim().starts_with(registry::GITHUB) {
            let g = registry::Github::parse(reg).map_err(err)?;
            (g.fetch().map_err(err)?, g.from())
        } else {
            let v = registry::show(&self.ctx.api, reg).map_err(err)?;
            let text = v
                .get("text")
                .and_then(Value::as_str)
                .ok_or("the registry sent no bundle text")?
                .to_string();
            (text, reg.trim().to_string())
        };
        let mut b = Bundle::parse(&text)?;
        b.set_bot("from", &from)?;
        if let Some(market) = market {
            // A product stays a product (it rolls); a month is that month.
            let named = match self.product(market) {
                Some(p) => p.to_ascii_lowercase(),
                None => slug(&crate::futures::resolve(&self.ctx, market, None).map_err(err)?),
            };
            b.set_bot("market", &named)?;
        }
        builds(&b)?;
        let folder = self.folder()?;
        let id = self.unique(&folder, &b.id);
        if id != b.id {
            b.set_bot("id", &id)?;
        }
        std::fs::write(folder.join(format!("{id}.toml")), b.text())
            .map_err(|e| format!("couldn't save the bot: {e}"))?;
        self.scan();
        Ok(id)
    }

    /// Saves `[bot] budget` (USDC) in bot `id`'s file.
    fn budget(&mut self, id: &str, usdc: f64) -> Result<(), String> {
        let f = self
            .files
            .get(id)
            .ok_or_else(|| format!("there is no bot called {id}"))?;
        let mut b = f.bundle.clone()?;
        b.set_budget(usdc)?;
        std::fs::write(&f.path, b.text()).map_err(|e| format!("couldn't save the bot: {e}"))?;
        self.scan();
        Ok(())
    }

    /// Reads the running bots' markets, and each bot's own wallet there;
    /// returns the chain's time.
    fn read(&mut self) -> Option<u64> {
        let mut now = None;
        self.offline.clear();
        for (market, feeds) in self.host.markets() {
            let accounts = &mut self.accounts;
            let read = send::read_market(&self.ctx, &market, &feeds).and_then(|(t, book)| {
                let index = feeds.index.get();
                for (id, feed) in self.host.bots_in(&market) {
                    let Some(w) = self.wallets.get(&id) else {
                        continue;
                    };
                    // Its other months, from the API (none known when it
                    // can't be read).
                    let account = api::account(&self.ctx.api, &w.key.to_string()).ok();
                    let elsewhere = account
                        .as_ref()
                        .map(|a| send::elsewhere(a, &market))
                        .unwrap_or_default();
                    match account {
                        Some(a) => accounts.insert(id.clone(), a),
                        None => accounts.remove(&id),
                    };
                    send::read_bot(&self.ctx, &w.key, &book, index.as_deref(), elsewhere, &feed)?;
                }
                Ok(t)
            });
            match read {
                Ok(t) => now = now.max(Some(t)),
                Err(_) => {
                    self.offline.insert(market);
                }
            }
        }
        now
    }

    /// Reads the registry's P&L of every bot with a wallet, and the
    /// holdings of the stopped ones, at most every [`PNL_EVERY`]; tries
    /// again the registrations that failed. Failures leave what was known.
    fn poll(&mut self) {
        if self.pnl_read.is_some_and(|at| at.elapsed() < PNL_EVERY) {
            return;
        }
        self.pnl_read = Some(Instant::now());
        let unregistered: Vec<(String, Bundle)> = self
            .wallets
            .keys()
            .filter(|id| !self.registered.contains_key(*id))
            .filter_map(|id| Some((id.clone(), self.files.get(id)?.bundle.clone().ok()?)))
            .collect();
        for (id, b) in unregistered {
            self.register(&id, &b);
        }
        let keys: Vec<(String, Pubkey)> = self
            .keys
            .iter()
            .filter(|(id, _)| self.files.contains_key(*id))
            .map(|(id, k)| (id.clone(), *k))
            .collect();
        for (id, key) in keys {
            if let Ok(pnl) = registry::pnl(&self.ctx.api, &key.to_string()) {
                self.pnl.insert(id.clone(), pnl);
            }
            // A running bot's account is read every look.
            if self.host.running(&id) {
                continue;
            }
            if let Ok(account) = api::account(&self.ctx.api, &key.to_string()) {
                self.accounts.insert(id, account);
            }
        }
    }

    /// Sends what the bots proposed, each signed by its bot's wallet, and
    /// tells each how its ladder intents went.
    fn send_all(&mut self) {
        for p in self.host.drain() {
            let outcome = match self.wallets.get(&p.bot) {
                Some(w) => send::send(&w.ctx, &p.market, &p.intent),
                None => Outcome::Refused("Its wallet is not open.".into()),
            };
            match (&p.intent, &outcome) {
                (Intent::Quote { .. }, Outcome::Posted(_)) => {
                    if let Some(q) = self.quote_view(&p.market, &p.intent) {
                        self.quotes.insert(p.bot.clone(), q);
                    }
                }
                (Intent::Cancel, Outcome::Cancelled(_) | Outcome::Clear) => {
                    self.quotes.remove(&p.bot);
                }
                _ => {}
            }
            if p.intent.ladder() {
                self.host.tell(&p.bot, outcome.clone());
            }
            if outcome != Outcome::Clear {
                self.outcomes.insert(p.bot, (outcome, Instant::now()));
            }
        }
    }

    fn quote_view(&self, market: &str, intent: &Intent) -> Option<QuoteView> {
        let Intent::Quote {
            ladder,
            core,
            lots,
            outer,
            fitted,
            ..
        } = intent
        else {
            return None;
        };
        let (_, feeds) = self.host.markets().into_iter().find(|(m, _)| m == market)?;
        let h = feeds.book.get()?.book.header;
        let grid = Grid {
            grid: h.grid,
            k: h.k,
            step: h.step_value,
            decimals: h.value_decimals,
        };
        Some(QuoteView {
            range: format!("{}–{}", grid.value(ladder.lo), grid.value(ladder.hi)),
            core: format!("{}–{}", grid.value(core.lo), grid.value(core.hi)),
            lots: *lots,
            outer: *outer,
            tiers: ladder.tiers.count.max(1),
            spread: f64::from(ladder.bid_spread) / h.k.max(1) as f64 * 100.0,
            fitted: *fitted,
        })
    }

    fn view(&self) -> Vec<BotView> {
        self.files
            .iter()
            .map(|(id, f)| {
                let bundle = f.bundle.as_ref().ok();
                let running = self.host.running(id);
                let config = bundle.and_then(Bundle::weights);
                // A stopped bot that would not build says so before it is
                // started.
                let invalid = bundle.filter(|_| !running).and_then(|b| builds(b).err());
                let market = self.markets.get(id);
                let position = self.host.position(id).map(|p| (*p).clone());
                // Running: its feed. Stopped: the API's account of its wallet.
                let (holdings, orders, held) = match (&position, self.accounts.get(id)) {
                    (Some(p), _) => {
                        let label = market.map(|m| m.1.clone()).unwrap_or_default();
                        let mut held: Vec<(String, Holding)> = p
                            .holdings
                            .iter()
                            .map(|h| (label.clone(), h.clone()))
                            .collect();
                        held.extend(p.elsewhere.iter().cloned());
                        (p.holdings.clone(), p.orders, held)
                    }
                    (None, Some(account)) => {
                        let here = bundle
                            .and_then(|b| self.places.get(&b.market))
                            .map(|p| p.1.clone())
                            .unwrap_or_default();
                        let mine = account.markets.iter().find(|m| m.market == here);
                        (
                            mine.map(send::holdings_in).unwrap_or_default(),
                            mine.map_or(0, |m| m.orders.len() as u32),
                            send::elsewhere(account, ""),
                        )
                    }
                    (None, None) => (Vec::new(), 0, Vec::new()),
                };
                BotView {
                    id: id.clone(),
                    file: f
                        .path
                        .file_name()
                        .map(|n| format!("bots/{}", n.to_string_lossy()))
                        .unwrap_or_default(),
                    name: bundle.map_or_else(|| id.clone(), |b| b.name().to_string()),
                    about: bundle.and_then(|b| b.about.clone()),
                    from: bundle.and_then(|b| b.from.clone()),
                    wallet: self.keys.get(id).map(|k| k.to_string()),
                    kinds: bundle.map(Bundle::kinds).unwrap_or_default(),
                    market: bundle.map(|b| b.market.clone()).unwrap_or_default(),
                    // The market's own label ("RAMX Oct 26") while it is the month the
                    // bot trades; the month's name ("ramx-oct26") if it rolled since.
                    label: bundle
                        .and_then(|b| self.places.get(&b.market))
                        .filter(|p| market.is_none_or(|m| m.0 == p.1))
                        .map(|p| p.0.clone())
                        .or_else(|| market.map(|(_, l, _)| l.clone())),
                    running,
                    problem: self
                        .problems
                        .get(id)
                        .cloned()
                        .or_else(|| f.bundle.as_ref().err().cloned())
                        .or(invalid),
                    offline: market.is_some_and(|(m, _, _)| self.offline.contains(m)),
                    // As written, else the default.
                    weights: KNOBS
                        .iter()
                        .map(|(n, _)| {
                            config
                                .and_then(|c| c.get(*n).cloned())
                                .unwrap_or_else(|| knob(&Weights::default(), n))
                        })
                        .collect(),
                    budget: bundle.map(Bundle::budget_atoms),
                    parts: self.host.parts(id),
                    quote: self.quotes.get(id).cloned(),
                    outcome: self.outcomes.get(id).cloned(),
                    position,
                    holdings,
                    orders,
                    held,
                    pnl: self.pnl.get(id).cloned(),
                    waiting: self.host.note(id),
                }
            })
            .collect()
    }
}

fn worker(settings: Settings) -> (Sender<Command>, Receiver<Reply>) {
    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    thread::spawn(move || {
        let mut w = Worker::new(settings);
        w.scan();
        let _ = reply_tx.send(Reply::Bots(w.view()));
        // The bots that ran when petri last quit run again.
        w.resume();
        w.release();
        let _ = reply_tx.send(Reply::Bots(w.view()));
        let mut next = Instant::now() + EVERY;
        loop {
            let wait = next.saturating_duration_since(Instant::now());
            match command_rx.recv_timeout(wait) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    let running: Vec<String> = w.texts.keys().cloned().collect();
                    for id in running {
                        w.stop(&id);
                    }
                    w.send_all();
                    w.release();
                    let _ = reply_tx.send(Reply::Down);
                    return;
                }
                Ok(command) => {
                    let reply = match command {
                        Command::Start { id, budget } => {
                            match budget.map(|b| w.budget(&id, b)).transpose() {
                                Ok(_) => {
                                    w.start(&id);
                                    None
                                }
                                Err(e) => Some(Reply::Note(e)),
                            }
                        }
                        Command::Stop(id) => {
                            // Its ladder comes down now, not at the next look.
                            w.stop(&id);
                            w.release();
                            w.folder()
                                .and_then(|f| bundle::set_running(&f, &id, false))
                                .err()
                                .map(Reply::Note)
                        }
                        Command::Set { bot, knob, value } => {
                            w.set(&bot, &knob, &value).err().map(Reply::Note)
                        }
                        Command::New(market) => {
                            Some(w.create(&market).map_or_else(Reply::Note, Reply::Created))
                        }
                        Command::Write(text) => {
                            Some(w.write(&text).map_or_else(Reply::Note, Reply::Written))
                        }
                        Command::Remove(id) => Some(
                            w.remove(&id)
                                .map_or_else(Reply::Note, |()| Reply::Removed(id)),
                        ),
                        Command::Publish { id, github } => Some(
                            w.publish(&id, github.as_deref())
                                .map_or_else(Reply::Note, Reply::Published),
                        ),
                        Command::Install { id, market } => Some(
                            w.install(&id, market.as_deref())
                                .map_or_else(Reply::Note, Reply::Installed),
                        ),
                        Command::Shutdown => unreachable!(),
                    };
                    if let Some(reply) = reply {
                        let _ = reply_tx.send(reply);
                    }
                    if reply_tx.send(Reply::Bots(w.view())).is_err() {
                        return;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            next = Instant::now() + EVERY;
            w.scan();
            if w.host.any()
                && let Some(now) = w.read()
            {
                // A product bot whose month stopped trading moves on first.
                w.roll();
                w.host.tick(now);
                // What it read shows while what the bots propose is sent.
                if reply_tx.send(Reply::Bots(w.view())).is_err() {
                    return;
                }
            }
            w.send_all();
            w.tidy();
            w.release();
            w.poll();
            if reply_tx.send(Reply::Bots(w.view())).is_err() {
                return;
            }
        }
    });
    (command_tx, reply_rx)
}

// --- Headless: `petri bots` and `petri bots serve` -----------------------------
//
// The same worker without a terminal, for agents and other front ends (the
// DeepSeek Harness plugin, plugins/petri): they read the bots as JSON and
// drive them with the commands the view sends.

fn holding_json(h: &Holding) -> Value {
    let number = |t: &str| t.parse::<f64>().map_or(Value::Null, |v| json!(v));
    json!({
        "side": if h.side == Side::Long { "long" } else { "short" },
        "low": number(&h.low),
        "high": number(&h.high),
        "lots": h.lots,
        "costAtoms": h.cost.to_string(),
        "valueAtoms": h.value.to_string(),
    })
}

/// A bot as JSON: everything the Bots view shows.
pub fn bot_json(b: &BotView) -> Value {
    let outcome = b.outcome.as_ref().map(|(o, at)| {
        let (kind, text) = match o {
            Outcome::Posted(t) => ("posted", t.clone()),
            Outcome::Cancelled(t) => ("cancelled", t.clone()),
            Outcome::Sent(t) => ("sent", t.clone()),
            Outcome::Clear => ("clear", String::new()),
            Outcome::Refused(t) => ("refused", t.clone()),
        };
        json!({ "kind": kind, "text": text, "secondsAgo": at.elapsed().as_secs() })
    });
    json!({
        "id": b.id,
        "name": b.name,
        "about": b.about,
        "from": b.from,
        "wallet": b.wallet,
        "file": b.file,
        "market": b.market,
        "label": b.label,
        "kinds": b.kinds,
        "running": b.running,
        "problem": b.problem,
        "offline": b.offline,
        "weights": KNOBS
            .iter()
            .zip(&b.weights)
            .map(|((name, about), value)| json!({ "name": name, "about": about, "value": value }))
            .collect::<Vec<_>>(),
        "budgetAtoms": b.budget.map(|v| v.to_string()),
        "parts": b.parts.iter().map(|p| json!({
            "id": p.id,
            "state": format!("{:?}", p.state).to_lowercase(),
            "error": p.error,
            "missing": p.missing,
        })).collect::<Vec<_>>(),
        "quote": b.quote.as_ref().map(|q| json!({
            "range": q.range, "core": q.core, "lots": q.lots, "outer": q.outer,
            "tiers": q.tiers, "spreadPctOfStep": q.spread, "fitted": q.fitted,
        })),
        "outcome": outcome,
        "position": b.position.as_ref().map(|p| json!({
            "cashAtoms": p.cash.to_string(),
            "freeAtoms": p.free.to_string(),
            "walletAtoms": p.wallet.to_string(),
            "inventory": p.inventory,
        })),
        "holdings": b.holdings.iter().map(holding_json).collect::<Vec<_>>(),
        "held": b.held.iter().map(|(market, h)| {
            let mut one = holding_json(h);
            one["market"] = market.clone().into();
            one
        }).collect::<Vec<_>>(),
        "orders": b.orders,
        "pnlAtoms": b.pnl.as_ref().and_then(|p| p.pnl_atoms.clone()),
        "points": b.pnl.as_ref().map_or(Value::Null, |p| p.points.clone()),
        "counted": b.pnl.as_ref().and_then(|p| p.counted),
        "waiting": b.waiting,
    })
}

/// A stopped worker on the bots on disk, with the registry's P&L read once.
fn snapshot(settings: Settings) -> Worker {
    let mut w = Worker::new(settings);
    w.scan();
    w.poll();
    w
}

/// The bots on disk, as a stopped worker sees them (`petri bots`).
pub fn list(settings: Settings) -> Vec<BotView> {
    snapshot(settings).view()
}

/// The folder bots live in.
pub fn folder() -> Result<PathBuf, String> {
    bundle::folder()
}

/// Writes a new ladder bot for `market` (any name petri knows) and returns
/// its id (`petri bots new`). It trades only once started.
pub fn create(settings: Settings, market: &str) -> Result<String, String> {
    let mut w = Worker::new(settings);
    let m = crate::futures::resolve(&w.ctx, market, None).map_err(err)?;
    w.scan();
    w.create(&slug(&m))
}

/// Sets weight `knob` of bot `id` in its file, if the bot builds with it
/// (`petri bots set`). A petri running the bot picks the change up.
pub fn set(settings: Settings, id: &str, knob: &str, value: &str) -> Result<(), String> {
    if !KNOBS.iter().any(|(n, _)| *n == knob) {
        let names: Vec<_> = KNOBS.iter().map(|(n, _)| *n).collect();
        return Err(format!(
            "bots have no weight called {knob} (one of: {})",
            names.join(", ")
        ));
    }
    let mut w = Worker::new(settings);
    w.scan();
    w.set(id, knob, value)
}

/// The bot `id` as the list shows it.
fn one(w: &Worker, id: &str) -> Result<Value, String> {
    w.view()
        .iter()
        .find(|b| b.id == id)
        .map(bot_json)
        .ok_or_else(|| format!("there is no bot called {id}"))
}

/// Validates bundle `text` and saves it as its bot's file (`petri bots
/// write`): the bot's id and its JSON.
pub fn write(settings: Settings, text: &str) -> Result<(String, Value), String> {
    let mut w = Worker::new(settings);
    w.scan();
    let id = w.write(text)?;
    let bot = one(&w, &id)?;
    Ok((id, bot))
}

/// Bot `id`'s JSON and its file's text as it is now (`petri bots read`),
/// for an agent to change and `write` back. It only reads, so it works
/// while another petri runs the bot.
pub fn read(settings: Settings, id: &str) -> Result<(Value, String), String> {
    let w = snapshot(settings);
    let text = w.text_of(id)?;
    Ok((one(&w, id)?, text))
}

/// Deletes bot `id`'s file (`petri bots remove`); a petri running it stops
/// it, ladder down and cash back, as it sees the file gone. Its wallet
/// stays.
pub fn remove(settings: Settings, id: &str) -> Result<(), String> {
    let mut w = Worker::new(settings);
    w.scan();
    w.remove(id)
}

/// Publishes bot `id` (`petri bots publish`), naming the GitHub file that
/// holds it when given: `{ id, version, hash }`.
pub fn publish(settings: Settings, id: &str, github: Option<&str>) -> Result<Value, String> {
    let mut w = Worker::new(settings);
    w.scan();
    w.publish(id, github)
}

/// Installs published bot `reg` (`petri bots install`): the new bot's id
/// and its JSON.
pub fn install(
    settings: Settings,
    reg: &str,
    market: Option<&str>,
) -> Result<(String, Value), String> {
    let mut w = Worker::new(settings);
    w.scan();
    let id = w.install(reg, market)?;
    let bot = one(&w, &id)?;
    Ok((id, bot))
}

/// The registry's published bots (`petri bots discover`).
pub fn discover(
    settings: Settings,
    market: Option<&str>,
    sort: Option<&str>,
    days: Option<u32>,
) -> Result<Value, String> {
    registry::discover(&Ctx::new(settings).api, market, sort, days).map_err(err)
}

/// One published bot, with its text (`petri bots show`).
pub fn show(settings: Settings, reg: &str) -> Result<Value, String> {
    registry::show(&Ctx::new(settings).api, reg).map_err(err)
}

/// The registry's board (`petri bots leaderboard`).
pub fn leaderboard(settings: Settings, days: Option<u32>) -> Result<Value, String> {
    registry::leaderboard(&Ctx::new(settings).api, days).map_err(err)
}

/// One `petri bots serve` command, as a line of JSON.
fn parse_command(line: &str) -> Result<Command, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not a command: {e}"))?;
    let text = |key: &str| v.get(key).and_then(|x| x.as_str()).map(str::to_string);
    if let Some(id) = text("start") {
        let budget = match v.get("budget") {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => Some(
                s.trim()
                    .parse()
                    .map_err(|_| format!("budget {s} is not an amount of USDC"))?,
            ),
            Some(_) => return Err("budget is an amount of USDC".into()),
        };
        return Ok(Command::Start { id, budget });
    }
    if let Some(id) = text("stop") {
        return Ok(Command::Stop(id));
    }
    if let Some(market) = text("new") {
        return Ok(Command::New(market));
    }
    if let Some(t) = text("write") {
        return Ok(Command::Write(t));
    }
    if let Some(id) = text("remove") {
        return Ok(Command::Remove(id));
    }
    if let Some(p) = v.get("publish") {
        // `{"publish":ID}`, or `{"publish":{"id":ID,"github":"OWNER/REPO..."}}`.
        let (id, github) = match p {
            Value::String(id) => (Some(id.clone()), None),
            Value::Object(o) => (
                o.get("id").and_then(Value::as_str).map(str::to_string),
                o.get("github").and_then(Value::as_str).map(str::to_string),
            ),
            _ => (None, None),
        };
        let id = id.ok_or("publish needs an id")?;
        return Ok(Command::Publish { id, github });
    }
    if let Some(i) = v.get("install") {
        let id = i
            .get("id")
            .and_then(Value::as_str)
            .ok_or("install needs an id")?;
        return Ok(Command::Install {
            id: id.to_string(),
            market: i.get("market").and_then(Value::as_str).map(str::to_string),
        });
    }
    if let Some(s) = v.get("set") {
        let field = |key: &str| {
            s.get(key)
                .and_then(|x| match x {
                    Value::String(t) => Some(t.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .ok_or_else(|| format!("set needs {key}"))
        };
        return Ok(Command::Set {
            bot: field("bot")?,
            knob: field("knob")?,
            value: field("value")?,
        });
    }
    if v.get("shutdown").is_some() {
        return Ok(Command::Shutdown);
    }
    Err("commands: start, stop, set, new, write, remove, publish, install, shutdown".into())
}

/// Runs bots without a terminal (`petri bots serve`). Commands arrive on
/// stdin, one JSON object a line: `{"start":ID,"budget":N}` (budget
/// optional), `{"stop":ID}`, `{"set":{"bot":ID,"knob":NAME,"value":N}}`,
/// `{"new":MARKET}`, `{"write":TEXT}`, `{"remove":ID}`, `{"publish":ID}`,
/// `{"install":{"id":REGID,"market":M}}`, `{"shutdown":true}`. Replies
/// leave on stdout, one a line: `{"bots":[...]}` after every command and
/// every look (two seconds), so the next lists show a command's effect;
/// `{"note":TEXT}`, `{"created":ID}`, `{"written":ID}`, `{"removed":ID}`,
/// `{"published":{...}}`, `{"installed":ID}`, and `{"down":true}` last.
/// Closing stdin is a shutdown: every bot stops, its ladder comes down and
/// its cash goes back to the wallet before `down`.
pub fn serve(settings: Settings) -> Result<(), String> {
    use std::io::{BufRead, Write};
    let (commands, replies) = worker(settings);
    {
        let commands = commands.clone();
        thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match parse_command(&line) {
                    Ok(command) => {
                        if commands.send(command).is_err() {
                            return;
                        }
                    }
                    Err(e) => println!("{}", json!({ "note": e })),
                }
            }
            let _ = commands.send(Command::Shutdown);
        });
    }
    let mut out = std::io::stdout();
    let mut reading = true;
    for reply in &replies {
        let line = match reply {
            Reply::Bots(list) => {
                let bots: Vec<_> = list.iter().map(bot_json).collect();
                json!({ "bots": bots }).to_string()
            }
            Reply::Note(note) => json!({ "note": note }).to_string(),
            Reply::Created(id) => json!({ "created": id }).to_string(),
            Reply::Written(id) => json!({ "written": id }).to_string(),
            Reply::Removed(id) => json!({ "removed": id }).to_string(),
            Reply::Published(p) => json!({ "published": p }).to_string(),
            Reply::Installed(id) => json!({ "installed": id }).to_string(),
            Reply::Down => {
                let _ = writeln!(out, "{}", json!({ "down": true }));
                let _ = out.flush();
                return Ok(());
            }
        };
        if reading && writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            // Nobody reads any more: stop the bots as a closed stdin would,
            // and wait for their ladders to come down.
            reading = false;
            let _ = commands.send(Command::Shutdown);
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Bots {
    pub list: Vec<BotView>,
    /// The bot shown; `list.len()` is a new bot.
    pub selected: usize,
    pub weight: usize,
    /// A weight being typed.
    pub editing: Option<String>,
    /// The shown bot's start, waiting for a yes.
    pub confirm: bool,
    /// The market picked for a new bot.
    pub pick: usize,
    pub note: Option<String>,
    pub loaded: bool,
    /// The bot to show once the list has it (one just made).
    show: Option<String>,
    commands: Option<Sender<Command>>,
    replies: Option<Receiver<Reply>>,
}

/// The markets a new bot can quote: the tradable ones.
pub fn open(markets: &[MarketView]) -> Vec<&MarketView> {
    markets.iter().filter(|m| m.tradable).collect()
}

impl Bots {
    /// The bots as they are on disk, read here and now (`petri tui --render
    /// ... --view bots`).
    pub fn take_snapshot(&mut self, settings: Settings) {
        let mut w = Worker::new(settings);
        w.scan();
        self.list = w.view();
        self.loaded = true;
    }

    /// Starts the worker on first use.
    pub fn start(&mut self, settings: Settings) {
        if self.commands.is_none() {
            let (tx, rx) = worker(settings);
            self.commands = Some(tx);
            self.replies = Some(rx);
        }
    }

    fn send(&self, command: Command) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(command);
        }
    }

    pub fn tick(&mut self) {
        let mut replies = Vec::new();
        if let Some(rx) = &self.replies {
            while let Ok(reply) = rx.try_recv() {
                replies.push(reply);
            }
        }
        for reply in replies {
            match reply {
                Reply::Bots(list) => {
                    let shown = self.shown().map(|b| b.id.clone());
                    // The first read opens on the first bot.
                    let new = self.loaded && self.selected >= self.list.len();
                    self.list = list;
                    self.loaded = true;
                    let want = self.show.clone().or(shown);
                    if let Some(at) = want.and_then(|id| self.list.iter().position(|b| b.id == id))
                    {
                        self.selected = at;
                        self.show = None;
                    } else if new {
                        self.selected = self.list.len();
                    }
                    self.selected = self.selected.min(self.list.len());
                }
                Reply::Note(note) => self.note = Some(note),
                Reply::Created(id) | Reply::Written(id) | Reply::Installed(id) => {
                    self.show = Some(id)
                }
                Reply::Removed(_) | Reply::Published(_) | Reply::Down => {}
            }
        }
    }

    /// Stops every bot and waits for their ladders to come down (also one
    /// started too lately to show as running).
    pub fn shutdown(&mut self) {
        if self.commands.is_none() {
            return;
        }
        self.send(Command::Shutdown);
        if let Some(rx) = &self.replies {
            while let Ok(reply) = rx.recv() {
                if matches!(reply, Reply::Down) {
                    break;
                }
            }
        }
    }

    pub fn shown(&self) -> Option<&BotView> {
        self.list.get(self.selected)
    }

    /// Shows the bot at `index` (`list.len()`: a new bot).
    pub fn select(&mut self, index: usize) {
        if self.editing.is_some() || self.confirm {
            return;
        }
        self.selected = index.min(self.list.len());
        self.note = None;
    }

    fn step(&mut self, by: isize) {
        let n = self.list.len() as isize + 1;
        self.select((self.selected as isize + by).rem_euclid(n) as usize);
    }

    /// Start (after a yes) or stop the shown bot.
    pub fn toggle(&mut self) {
        let Some(bot) = self.shown() else {
            return;
        };
        if bot.running {
            let id = bot.id.clone();
            self.send(Command::Stop(id));
        } else {
            self.confirm = true;
        }
        self.note = None;
    }

    pub fn answer(&mut self, start: bool) {
        if start && let Some(bot) = self.shown() {
            self.send(Command::Start {
                id: bot.id.clone(),
                budget: None,
            });
        }
        self.confirm = false;
    }

    /// Starts typing weight `index` of the shown bot.
    pub fn edit(&mut self, index: usize) {
        if self.confirm {
            return;
        }
        let Some(bot) = self.shown() else {
            return;
        };
        if index >= KNOBS.len() {
            return;
        }
        let value = bot.weights.get(index).cloned().unwrap_or_default();
        self.weight = index;
        self.editing = Some(if value == "all" { String::new() } else { value });
        self.note = None;
    }

    fn save(&mut self) {
        let (Some(text), Some(bot)) = (self.editing.take(), self.shown()) else {
            return;
        };
        let knob = KNOBS[self.weight].0.to_string();
        let bot = bot.id.clone();
        self.send(Command::Set {
            bot,
            knob,
            value: text,
        });
    }

    /// A new bot on the open market at `index` of [`open`].
    pub fn create(&mut self, markets: &[MarketView], index: usize) {
        if let Some(m) = open(markets).get(index) {
            self.send(Command::New(slug(m)));
            self.note = None;
        }
    }

    /// Keys for the Bots view. Returns false when the key is not ours.
    pub fn key(&mut self, key: KeyEvent, markets: &[MarketView]) -> bool {
        if let Some(text) = &mut self.editing {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter => self.save(),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == '-' => text.push(c),
                _ => {}
            }
            return true;
        }
        if self.confirm {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.answer(true),
                KeyCode::Char('n') | KeyCode::Esc => self.answer(false),
                _ => {}
            }
            return true;
        }
        let new = self.selected >= self.list.len();
        let n = if new {
            open(markets).len()
        } else {
            KNOBS.len()
        };
        match key.code {
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Left if n > 0 && new => self.pick = (self.pick + n - 1) % n,
            KeyCode::Right if n > 0 && new => self.pick = (self.pick + 1) % n,
            KeyCode::Left => self.weight = (self.weight + n - 1) % n,
            KeyCode::Right => self.weight = (self.weight + 1) % n,
            KeyCode::Enter if new => self.create(markets, self.pick),
            KeyCode::Enter => self.edit(self.weight),
            KeyCode::Char(' ') if !new => self.toggle(),
            KeyCode::Char('n') => self.select(self.list.len()),
            _ => return false,
        }
        true
    }
}

/// Quitting petri (however it ends) takes the bots' ladders down first.
impl Drop for Bots {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn footer_hint(b: &Bots) -> Vec<(&'static str, &'static str)> {
    if b.editing.is_some() {
        return vec![("Enter", " Save"), ("Esc", " Cancel")];
    }
    if b.confirm {
        return vec![("y", " Start"), ("Esc", " Cancel")];
    }
    if b.selected >= b.list.len() {
        return vec![
            ("↑/↓", " Bots"),
            ("←/→", " Market"),
            ("Enter", " Make it"),
            ("q", " Quit"),
        ];
    }
    vec![
        ("↑/↓", " Bots"),
        ("←/→", " Weight"),
        ("Enter", " Edit"),
        ("Space", " Start/Stop"),
        ("n", " New bot"),
        ("q", " Quit"),
    ]
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

/// A bot's state in a word, and its colour.
fn state(bot: &BotView) -> (&'static str, ratatui::style::Color) {
    if bot.running && bot.parts.iter().any(|p| p.state == State::Failed) {
        ("Needs a look", ALERT)
    } else if bot.running {
        ("Running", GREEN)
    } else if bot.problem.is_some() {
        ("Can't start", ALERT)
    } else {
        ("Stopped", MUTED)
    }
}

/// The bot list in the sidebar slot, then "+ New bot"; returns each entry's
/// area, in order, for clicks.
pub fn sidebar(frame: &mut Frame, b: &Bots, area: Rect) -> Vec<Rect> {
    let inner = panel(frame, area, "BOTS");
    let mut shown = Vec::new();
    if !b.loaded {
        return shown;
    }
    let mut y = inner.y + 1;
    for (i, bot) in b.list.iter().enumerate() {
        if y + 2 > inner.bottom() {
            return shown;
        }
        let selected = i == b.selected;
        let marker = if selected { "> " } else { "  " };
        let (word, colour) = state(bot);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(
                        bot.id.clone(),
                        if selected {
                            strong().fg(ACCENT)
                        } else {
                            strong()
                        },
                    ),
                ]),
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(ACCENT)),
                    Span::styled(word, Style::default().fg(colour)),
                ]),
            ])
            .style(Style::default().bg(if selected { SELECTED } else { PANEL })),
            Rect::new(inner.x, y, inner.width, 2),
        );
        shown.push(Rect::new(inner.x, y, inner.width, 2));
        y += 3;
    }
    if y < inner.bottom() {
        let selected = b.selected >= b.list.len();
        let area = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    "+ New bot",
                    if selected {
                        strong().fg(ACCENT)
                    } else {
                        muted()
                    },
                ),
            ]))
            .style(Style::default().bg(if selected { SELECTED } else { PANEL })),
            area,
        );
        shown.push(area);
    }
    shown
}

/// What the content drew that takes clicks: buttons (drawn by the caller,
/// which highlights them) and rows.
#[derive(Default)]
pub struct Drawn {
    pub buttons: Vec<(Rect, &'static str, Action)>,
    pub rows: Vec<(Rect, Action)>,
}

fn ago(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3_600 {
        format!("{}m ago", s / 60)
    } else {
        format!("{}h ago", s / 3_600)
    }
}

/// The shown bot, or the new-bot picker.
pub fn content(frame: &mut Frame, b: &Bots, markets: &[MarketView], area: Rect) -> Drawn {
    let mut drawn = Drawn::default();
    if !b.loaded {
        return drawn;
    }
    let Some(bot) = b.shown() else {
        picker(frame, b, markets, area, &mut drawn);
        return drawn;
    };
    let mut y = area.y;
    let (word, colour) = state(bot);
    put(
        frame,
        area,
        y,
        Line::from(vec![
            Span::styled(bot.id.clone(), strong()),
            Span::styled("  ", muted()),
            Span::styled(word, Style::default().fg(colour)),
            Span::styled(if bot.offline { "  offline" } else { "" }, muted()),
        ]),
    );
    y += 1;
    let market = bot.label.clone().unwrap_or_else(|| bot.market.clone());
    put(
        frame,
        area,
        y,
        Line::styled(format!("{market} · {}", bot.file), muted()),
    );
    y += 2;
    let note = b.note.as_ref().or(bot.problem.as_ref());
    if let Some(note) = note {
        let height = 2.min(area.bottom().saturating_sub(y));
        frame.render_widget(
            Paragraph::new(note.clone())
                .style(Style::default().fg(ALERT))
                .wrap(Wrap { trim: true }),
            Rect::new(area.x, y, area.width, height),
        );
        y += height + 1;
    }
    if b.confirm {
        let budget = bot
            .budget
            .map_or("all".to_string(), |a| money(i128::from(a)));
        let text = format!(
            "Start {}? Petri tops its own wallet up to {budget} from yours, and it trades {market} from there, requoting on its own as prices move. Stop it, or quit petri, and its ladder comes down and its cash comes back to your wallet.",
            bot.id
        );
        let height = 4.min(area.bottom().saturating_sub(y));
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(TEXT))
                .wrap(Wrap { trim: true }),
            Rect::new(area.x, y, area.width, height),
        );
        y += height + 1;
        if y < area.bottom() {
            drawn
                .buttons
                .push((Rect::new(area.x, y, 7, 1), "Start", Action::BotAnswer(true)));
            drawn.buttons.push((
                Rect::new(area.x + 9, y, 8, 1),
                "Cancel",
                Action::BotAnswer(false),
            ));
        }
        return drawn;
    }
    if y < area.bottom() {
        let (title, width) = if bot.running {
            ("Stop", 6)
        } else {
            ("Start", 7)
        };
        drawn
            .buttons
            .push((Rect::new(area.x, y, width, 1), title, Action::BotToggle));
    }
    y += 2;
    let wide = area.width >= 96;
    let left = Rect::new(
        area.x,
        y,
        if wide { 56 } else { area.width },
        area.bottom().saturating_sub(y),
    );
    weights(frame, b, bot, left, &mut drawn);
    let right = if wide {
        Rect::new(area.x + 60, y, area.width - 60, left.height)
    } else {
        let top = y + KNOBS.len() as u16 + 2;
        Rect::new(area.x, top, area.width, area.bottom().saturating_sub(top))
    };
    now(frame, bot, right);
    drawn
}

fn weights(frame: &mut Frame, b: &Bots, bot: &BotView, area: Rect, drawn: &mut Drawn) {
    put(frame, area, area.y, Line::styled("Weights", strong()));
    for (i, (name, what)) in KNOBS.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.bottom() {
            return;
        }
        let selected = i == b.weight;
        let value = match (&b.editing, selected) {
            (Some(text), true) => format!("{text}▌"),
            _ => bot.weights.get(i).cloned().unwrap_or_default(),
        };
        let row = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(format!("{name:<11}"), Style::default().fg(TEXT)),
                Span::styled(
                    format!("{value:<9}"),
                    if selected {
                        strong().fg(ACCENT)
                    } else {
                        strong()
                    },
                ),
                Span::styled(*what, muted()),
            ]))
            .style(Style::default().bg(if selected { SELECTED } else { PANEL })),
            row,
        );
        drawn.rows.push((row, Action::BotWeight(i)));
    }
}

/// What the bot is doing: its ladder, cash, holdings, last send and parts.
fn now(frame: &mut Frame, bot: &BotView, area: Rect) {
    let mut lines = vec![Line::styled("Now", strong())];
    let row = |label: &str, value: String, style: Style| {
        Line::from(vec![
            Span::styled(format!("{label:<8}"), muted()),
            Span::styled(value, style),
        ])
    };
    let text = Style::default().fg(TEXT);
    if let Some(q) = &bot.quote {
        let ladder = if q.range == q.core {
            q.range.clone()
        } else {
            format!("{}, full depth {}", q.range, q.core)
        };
        lines.push(row("Ladder", ladder, text));
        let tiers = if q.tiers == 1 {
            "1 tier".to_string()
        } else {
            format!("{} tiers", q.tiers)
        };
        let lots = if q.outer > 0 {
            format!(
                "{} a step in {tiers}, {} outside",
                crate::futures::group(q.lots),
                crate::futures::group(q.outer)
            )
        } else {
            format!("{} a step in {tiers}", crate::futures::group(q.lots))
        };
        lines.push(row("Lots", lots, text));
        if q.fitted {
            lines.push(row("", "Narrowed to fit the book".into(), muted()));
        }
        lines.push(row("Spread", format!("±{:.2}% of a step", q.spread), text));
    }
    if let Some(p) = &bot.position {
        lines.push(row(
            "Cash",
            format!(
                "{} here · {} in its wallet",
                money(i128::from(p.cash)),
                money(i128::from(p.wallet))
            ),
            text,
        ));
        if p.inventory != 0 {
            lines.push(row("Holds", format!("{:+} lots", p.inventory), text));
        }
    }
    if let Some((outcome, at)) = &bot.outcome {
        let (said, style) = match outcome {
            Outcome::Posted(_) => (format!("Posted its ladder {}", ago(*at)), text),
            Outcome::Cancelled(_) => (
                format!(
                    "Took its ladder down, cash back in your wallet, {}",
                    ago(*at)
                ),
                text,
            ),
            Outcome::Sent(what) => (format!("{what}, {}", ago(*at)), text),
            Outcome::Clear => (String::new(), text),
            Outcome::Refused(why) => (why.clone(), Style::default().fg(ALERT)),
        };
        lines.push(row("Last", said, style));
    }
    if let Some(waiting) = &bot.waiting {
        lines.push(row("Waiting", waiting.clone(), muted()));
    }
    if bot.running {
        let mut parts = vec![Span::styled(format!("{:<8}", "Parts"), muted())];
        for (i, p) in bot.parts.iter().enumerate() {
            if i > 0 {
                parts.push(Span::styled(" · ", Style::default().fg(DIM)));
            }
            let colour = match p.state {
                State::Active => TEXT,
                State::Failed => ALERT,
                _ => DIM,
            };
            parts.push(Span::styled(p.id.clone(), Style::default().fg(colour)));
        }
        lines.push(Line::from(parts));
        for p in &bot.parts {
            if let Some(e) = &p.error {
                lines.push(row(
                    "",
                    format!("{}: {e}", p.id),
                    Style::default().fg(ALERT),
                ));
            } else if p.state != State::Active && !p.missing.is_empty() {
                lines.push(row(
                    "",
                    format!("{} waits for {}", p.id, p.missing.join(", ")),
                    muted(),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// A new bot: pick its market.
fn picker(frame: &mut Frame, b: &Bots, markets: &[MarketView], area: Rect, drawn: &mut Drawn) {
    put(frame, area, area.y, Line::styled("New bot", strong()));
    let intro = "Pick its market. Petri writes the bot to a file you can edit here or by hand, and it trades only once you start it.";
    let height = 2.min(area.bottom().saturating_sub(area.y + 1));
    frame.render_widget(
        Paragraph::new(intro)
            .style(muted())
            .wrap(Wrap { trim: true }),
        Rect::new(area.x, area.y + 1, area.width, height),
    );
    if let Some(note) = &b.note {
        put(
            frame,
            area,
            area.y + 3,
            Line::styled(note.clone(), Style::default().fg(ALERT)),
        );
    }
    for (i, m) in open(markets).into_iter().enumerate() {
        let y = area.y + 5 + i as u16;
        if y >= area.bottom() {
            return;
        }
        let selected = i == b.pick;
        let row = Rect::new(area.x, y, area.width.min(40), 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    m.label.clone(),
                    if selected {
                        strong().fg(ACCENT)
                    } else {
                        strong()
                    },
                ),
            ]))
            .style(Style::default().bg(if selected { SELECTED } else { PANEL })),
            row,
        );
        drawn.rows.push((row, Action::NewBot(i)));
    }
}

#[cfg(test)]
mod headless_tests {
    use super::*;
    use clap::Parser;

    /// A worker on `folder`, its API and RPC unreachable (nothing here reads
    /// the network).
    fn worker_in(folder: &std::path::Path) -> Worker {
        let cli = crate::cli::Cli::parse_from([
            "petri",
            "--profile",
            "dev",
            "--api",
            "http://127.0.0.1:9",
            "--rpc",
            "http://127.0.0.1:9",
        ]);
        let settings = Settings::merge(&cli, &Default::default());
        let mut w = Worker::new(settings);
        w.folder = Ok(folder.to_path_buf());
        w
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("petri-bots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    const SCRIPT: &str = r#"[bot]
id = "dip"
market = "ramx-nov26"
name = "Dip buyer"

[[row]]
id = "strategy"
kind = "strategy/script"
source = '''
fn tick(m) {
  if m.change(3600) < -0.03 { long(#{ usd: 10 }); }
}
'''
"#;

    #[test]
    fn serve_reads_each_command_and_refuses_the_rest() {
        assert!(matches!(
            parse_command(r#"{"start":"a"}"#),
            Ok(Command::Start { id, budget: None }) if id == "a"
        ));
        assert!(matches!(
            parse_command(r#"{"start":"a","budget":250}"#),
            Ok(Command::Start { id, budget: Some(b) }) if id == "a" && b == 250.0
        ));
        assert!(matches!(
            parse_command(r#"{"start":"a","budget":"12.5"}"#),
            Ok(Command::Start { budget: Some(b), .. }) if b == 12.5
        ));
        assert!(parse_command(r#"{"start":"a","budget":true}"#).is_err());
        assert!(matches!(parse_command(r#"{"stop":"a"}"#), Ok(Command::Stop(id)) if id == "a"));
        assert!(matches!(parse_command(r#"{"new":"ramx"}"#), Ok(Command::New(m)) if m == "ramx"));
        assert!(matches!(
            parse_command(r#"{"write":"[bot]\nid = \"a\""}"#),
            Ok(Command::Write(t)) if t == "[bot]\nid = \"a\""
        ));
        assert!(matches!(parse_command(r#"{"remove":"a"}"#), Ok(Command::Remove(id)) if id == "a"));
        assert!(matches!(
            parse_command(r#"{"publish":"a"}"#),
            Ok(Command::Publish { id, github: None }) if id == "a"
        ));
        assert!(matches!(
            parse_command(r#"{"publish":{"id":"a","github":"me/bots"}}"#),
            Ok(Command::Publish { id, github: Some(g) }) if id == "a" && g == "me/bots"
        ));
        assert!(parse_command(r#"{"publish":{"github":"me/bots"}}"#).is_err());
        assert!(matches!(
            parse_command(r#"{"install":{"id":"github:me/bots@v1"}}"#),
            Ok(Command::Install { id, market: None }) if id == "github:me/bots@v1"
        ));
        assert!(matches!(
            parse_command(r#"{"install":{"id":"dip-buyer"}}"#),
            Ok(Command::Install { id, market: None }) if id == "dip-buyer"
        ));
        assert!(matches!(
            parse_command(r#"{"install":{"id":"dip-buyer","market":"nandx"}}"#),
            Ok(Command::Install { market: Some(m), .. }) if m == "nandx"
        ));
        assert!(parse_command(r#"{"install":{"market":"nandx"}}"#).is_err());
        assert!(matches!(
            parse_command(r#"{"shutdown":true}"#),
            Ok(Command::Shutdown)
        ));
        match parse_command(r#"{"set":{"bot":"a","knob":"budget","value":250}}"#) {
            Ok(Command::Set { bot, knob, value }) => {
                assert_eq!(
                    (bot.as_str(), knob.as_str(), value.as_str()),
                    ("a", "budget", "250")
                );
            }
            _ => panic!("set"),
        }
        assert!(parse_command(r#"{"set":{"bot":"a","knob":"budget"}}"#).is_err());
        assert!(parse_command(r#"{"sign":"anything"}"#).is_err());
        assert!(parse_command("start a").is_err());
    }

    /// Two petris (the TUI and `petri bots serve`) never run one bot at once:
    /// the second start is refused until the first lets go.
    #[test]
    fn a_bot_runs_in_one_petri_at_a_time() {
        let folder = scratch("lock");
        let mut first = worker_in(&folder);
        let mut second = worker_in(&folder);
        assert_eq!(first.lock("ladder-ramx"), Ok(true));
        // The same petri restarting its own bot keeps its lock.
        assert_eq!(first.lock("ladder-ramx"), Ok(false));
        let refused = second.lock("ladder-ramx").unwrap_err();
        assert!(refused.contains("another petri"), "{refused}");
        assert_eq!(second.lock("ladder-nandx"), Ok(true));
        // A stopped bot lets go once its way out has been sent.
        first.release();
        assert!(first.locks.is_empty());
        assert_eq!(second.lock("ladder-ramx"), Ok(true));
        drop(second);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// `write` saves what builds, as `bots/<id>.toml`; `read` gives the file
    /// back, also while another petri holds the bot's lock (runs it);
    /// `remove` deletes the file and leaves the wallet.
    #[test]
    fn write_read_and_remove_a_bot() {
        let folder = scratch("write");
        let mut w = worker_in(&folder);
        w.scan();
        assert_eq!(w.write(SCRIPT), Ok("dip".to_string()));
        assert_eq!(
            std::fs::read_to_string(folder.join("dip.toml")).unwrap(),
            SCRIPT
        );
        let v = one(&w, "dip").unwrap();
        assert_eq!(v["name"], "Dip buyer");
        assert_eq!(v["kinds"], json!(["strategy/script"]));
        assert_eq!(v["budgetAtoms"], "100000000");
        assert_eq!(v["wallet"], Value::Null);
        assert_eq!(v["problem"], Value::Null);
        // A script that doesn't compile is refused, and nothing changes.
        let broken = SCRIPT.replace("long(#{ usd: 10 });", "long(#{ usd: 10 ");
        let e = w.write(&broken).unwrap_err();
        assert!(
            e.starts_with("strategy: the script does not compile"),
            "{e}"
        );
        assert_eq!(w.text_of("dip"), Ok(SCRIPT.to_string()));
        // An edit replaces the bot's own file, under whatever name it has.
        std::fs::rename(folder.join("dip.toml"), folder.join("mine.toml")).unwrap();
        w.scan();
        let edited = SCRIPT.replace("usd: 10", "usd: 20");
        assert_eq!(w.write(&edited), Ok("dip".to_string()));
        assert!(!folder.join("dip.toml").exists());
        assert_eq!(w.text_of("dip"), Ok(edited.clone()));
        // Never over another bot's file.
        let other = edited.replace("id = \"dip\"", "id = \"mine\"");
        let e = w.write(&other).unwrap_err();
        assert!(e.contains("holds the bot dip"), "{e}");
        // Another petri running it doesn't keep a read out.
        let mut serving = worker_in(&folder);
        assert_eq!(serving.lock("dip"), Ok(true));
        let mut reader = worker_in(&folder);
        reader.scan();
        assert_eq!(reader.text_of("dip"), Ok(edited));
        drop(serving);
        // Its wallet stays when the bot goes.
        let (path, key) = wallet::open(&folder, "dip").unwrap();
        w.remove("dip").unwrap();
        assert!(!folder.join("mine.toml").exists());
        assert!(path.exists());
        assert_eq!(wallet::pubkey(&folder, "dip"), Some(key));
        assert!(w.remove("dip").is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_start_budget_is_saved_to_the_file_first() {
        let folder = scratch("budget");
        let mut w = worker_in(&folder);
        w.scan();
        w.write(SCRIPT).unwrap();
        w.budget("dip", 250.0).unwrap();
        let text = w.text_of("dip").unwrap();
        assert!(text.contains("budget = 250\n"), "{text}");
        assert_eq!(one(&w, "dip").unwrap()["budgetAtoms"], "250000000");
        w.budget("dip", 12.5).unwrap();
        assert_eq!(one(&w, "dip").unwrap()["budgetAtoms"], "12500000");
        assert!(w.budget("dip", -1.0).is_err());
        assert!(w.budget("nobody", 1.0).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_installed_id_is_made_unique() {
        let folder = scratch("unique");
        let mut w = worker_in(&folder);
        w.scan();
        assert_eq!(w.unique(&folder, "dip"), "dip");
        w.write(SCRIPT).unwrap();
        assert_eq!(w.unique(&folder, "dip"), "dip-2");
        std::fs::write(folder.join("dip-2.toml"), "not a bot").unwrap();
        w.scan();
        assert_eq!(w.unique(&folder, "dip"), "dip-3");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_bot_as_json_names_every_weight() {
        let view = BotView {
            id: "ladder-ramx".into(),
            name: "RAMX ladder".into(),
            market: "ramx-nov26".into(),
            kinds: vec!["weights".into(), "strategy/ladder".into()],
            weights: KNOBS.iter().map(|_| "1".to_string()).collect(),
            outcome: Some((Outcome::Refused("no SOL for fees".into()), Instant::now())),
            holdings: vec![Holding {
                side: Side::Short,
                lo: 90,
                hi: 120,
                low: "90".into(),
                high: "120.5".into(),
                lots: 7,
                cost: 1_500_000,
                value: 2_000_000,
            }],
            orders: 2,
            ..Default::default()
        };
        let v = bot_json(&view);
        assert_eq!(v["id"], "ladder-ramx");
        assert_eq!(v["name"], "RAMX ladder");
        assert_eq!(v["weights"].as_array().map(Vec::len), Some(KNOBS.len()));
        assert_eq!(v["weights"][0]["name"], "budget");
        assert_eq!(v["outcome"]["kind"], "refused");
        assert_eq!(v["running"], false);
        assert_eq!(v["kinds"], json!(["weights", "strategy/ladder"]));
        assert_eq!(
            v["holdings"][0],
            json!({ "side": "short", "low": 90.0, "high": 120.5, "lots": 7,
                    "costAtoms": "1500000", "valueAtoms": "2000000" })
        );
        assert_eq!(v["orders"], 2);
        // Unknown until the registry answers.
        assert_eq!(v["pnlAtoms"], Value::Null);
        assert_eq!(v["points"], Value::Null);
        assert_eq!(v["counted"], Value::Null);
        let v = bot_json(&BotView {
            pnl: Some(Pnl {
                pnl_atoms: Some("-1250000".into()),
                points: json!([{ "t": 1, "pnlAtoms": "0" }]),
                counted: Some(true),
            }),
            ..Default::default()
        });
        assert_eq!(v["pnlAtoms"], "-1250000");
        assert_eq!(v["points"][0]["t"], 1);
        assert_eq!(v["counted"], true);
    }
}
