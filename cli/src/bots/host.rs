//! The host: petri's side of the wall. It mounts the clock and each bot's
//! market feeds, runs every bot as a subtree in a realm of its own, and is
//! the only thing that signs: it drains what the bots propose and hands it
//! to [`super::send`]. It does no I/O itself (the worker in
//! `research/bots.rs` reads the feeds and sends), so it runs as well in
//! tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    feeds::{BookRead, Clock, Feed, History, Index, Intent, Outcome, Position, Sink, Tick},
    keys::{self, CLOCK, INTENTS, RUN},
    ladder::LadderRow,
    script::ScriptRow,
    signals::{InventorySkewRow, NormalFairRow},
    weights::{Weights, WeightsRow},
};
use crate::compose::{
    Component, Ctx, FiberId, Registry, Result, State,
    loader::{Loader, Row},
};

/// Builds a row of a bot in `market`. A part may not provide the host's
/// keys: a bot reads the market, it never stands in for it.
pub fn build(row: &Row, market: &str) -> Result<Box<dyn Component>> {
    let settings = || {
        if row.config.is_empty() {
            Ok(())
        } else {
            Err(format!("{} takes no settings", row.kind))
        }
    };
    let market = market.to_string();
    let part: Box<dyn Component> = match row.kind.as_str() {
        "weights" => Box::new(WeightsRow(Weights::parse(&row.config)?)),
        "signal/normal-fair" => {
            settings()?;
            Box::new(NormalFairRow { market })
        }
        "signal/inventory-skew" => {
            settings()?;
            Box::new(InventorySkewRow { market })
        }
        "strategy/ladder" => {
            settings()?;
            Box::new(LadderRow { market })
        }
        "strategy/script" => Box::new(ScriptRow::build(&row.id, &row.config, market)?),
        other => return Err(format!("petri has no part called {other}")),
    };
    if let Some(key) = part.provide().into_iter().find(|k| keys::host(k)) {
        return Err(format!("a bot can't provide {key}"));
    }
    Ok(part)
}

/// Provides one value under a fixed key.
struct Provider<T> {
    key: String,
    value: T,
}

impl<T: Clone + Send + Sync + 'static> Component for Provider<T> {
    fn provide(&self) -> BTreeSet<String> {
        [self.key.clone()].into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        ctx.provide(&self.key, self.value.clone())
    }
}

/// The yearly volatility a bot assumes without a weights row (the house
/// maker's `MAKER_VOL`, as [`Weights::default`]).
pub const VOL: f64 = 0.40;

/// One run of a bot, from its start to its stop: what its parts read that
/// is the bot's own and may change while it runs, without restarting them.
pub struct Run {
    /// Its budget, USDC atoms (`[bot] budget`, else its weights row's).
    budget: AtomicU64,
    /// Its weights row's `vol`, else [`VOL`].
    vol: Mutex<f64>,
    /// A script's `m.memory`, kept by every tick of the run.
    pub memory: super::script::Memory,
}

impl Run {
    fn new(budget: u64, vol: f64) -> Self {
        Self {
            budget: AtomicU64::new(budget),
            vol: Mutex::new(vol),
            memory: super::script::Memory::default(),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget.load(Ordering::Relaxed)
    }

    pub fn vol(&self) -> f64 {
        *self.vol.lock().expect("run")
    }

    fn set(&self, budget: u64, vol: f64) {
        self.budget.store(budget, Ordering::Relaxed);
        *self.vol.lock().expect("run") = vol;
    }
}

/// The vol a bot's rows assume: its weights row's, else [`VOL`].
fn vol_of(rows: &[Row]) -> f64 {
    rows.iter()
        .find(|r| r.kind == "weights" && !r.disabled)
        .and_then(|r| Weights::parse(&r.config).ok())
        .map_or(VOL, |w| w.vol)
}

/// A bot's root: inside the bot's realm, it provides the bot's way out,
/// its wallet's position in its market and its run.
struct Root {
    sink: Arc<Sink>,
    position: (String, Arc<Feed<Position>>),
    run: Arc<Run>,
}

impl Component for Root {
    fn provide(&self) -> BTreeSet<String> {
        [
            INTENTS.to_string(),
            self.position.0.clone(),
            RUN.to_string(),
        ]
        .into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        ctx.provide(INTENTS, self.sink.clone())?;
        ctx.provide(&self.position.0, self.position.1.clone())?;
        ctx.provide(RUN, self.run.clone())
    }
}

/// One market's feeds, filled by the worker for every bot there.
#[derive(Clone, Default)]
pub struct Feeds {
    pub index: Arc<Feed<Index>>,
    pub book: Arc<Feed<BookRead>>,
    pub history: Arc<Feed<History>>,
}

struct Running {
    market: String,
    root: FiberId,
    loader: Loader,
    sink: Arc<Sink>,
    /// Its wallet in its market, filled by the worker for it alone.
    position: Arc<Feed<Position>>,
    run: Arc<Run>,
}

/// An intent on its way out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposed {
    pub bot: String,
    pub market: String,
    pub intent: Intent,
}

/// A part of a running bot as the Bots view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub id: String,
    pub state: State,
    pub error: Option<String>,
    pub missing: Vec<String>,
}

pub struct Host {
    reg: Registry,
    clock: Arc<Clock>,
    markets: BTreeMap<String, Feeds>,
    bots: BTreeMap<String, Running>,
    /// What stopped bots proposed on their way out, still to send.
    leftover: Vec<Proposed>,
    ticks: u64,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    pub fn new() -> Self {
        let mut reg = Registry::default();
        let clock = Arc::new(Clock::default());
        reg.mount(
            None,
            CLOCK,
            Box::new(Provider {
                key: CLOCK.into(),
                value: clock.clone(),
            }),
            Vec::new(),
        );
        Self {
            reg,
            clock,
            markets: BTreeMap::new(),
            bots: BTreeMap::new(),
            leftover: Vec::new(),
            ticks: 0,
        }
    }

    /// `market`'s feeds, mounted on first use.
    fn feeds(&mut self, market: &str) -> Feeds {
        if let Some(f) = self.markets.get(market) {
            return f.clone();
        }
        let f = Feeds::default();
        let mut provide = |key: String, value: Box<dyn Component>| {
            self.reg.mount(None, &key, value, Vec::new());
        };
        let index = keys::index(market);
        provide(
            index.clone(),
            Box::new(Provider {
                key: index,
                value: f.index.clone(),
            }),
        );
        let book = keys::book(market);
        provide(
            book.clone(),
            Box::new(Provider {
                key: book,
                value: f.book.clone(),
            }),
        );
        let history = keys::history(market);
        provide(
            history.clone(),
            Box::new(Provider {
                key: history,
                value: f.history.clone(),
            }),
        );
        self.markets.insert(market.to_string(), f.clone());
        f
    }

    /// Starts bot `id` in `market` from its rows, with `budget` (USDC
    /// atoms). Nothing runs if a row does not build.
    pub fn start(&mut self, id: &str, market: &str, rows: &[Row], budget: u64) -> Result<()> {
        if self.bots.contains_key(id) {
            return self.update(id, rows, budget);
        }
        self.mount(id, market, rows, Arc::new(Run::new(budget, vol_of(rows))))
    }

    /// Moves running bot `id` to `market` (a product bot's next month): it
    /// stops where it was (its ladder's cancel goes out with the next
    /// drain, for the market it leaves) and starts there with the same run,
    /// so its budget and its script's memory go with it.
    pub fn roll(&mut self, id: &str, market: &str, rows: &[Row]) -> Result<()> {
        let Some(run) = self.bots.get(id).map(|b| b.run.clone()) else {
            return Ok(());
        };
        self.stop(id);
        self.mount(id, market, rows, run)
    }

    fn mount(&mut self, id: &str, market: &str, rows: &[Row], run: Arc<Run>) -> Result<()> {
        self.feeds(market);
        let sink = Arc::new(Sink::default());
        let position = Arc::new(Feed::default());
        let root = self.reg.mount(
            None,
            id,
            Box::new(Root {
                sink: sink.clone(),
                position: (keys::position(market), position.clone()),
                run: run.clone(),
            }),
            keys::realm(id),
        );
        let mut loader = Loader::default();
        let built = loader.reconcile(&mut self.reg, root, rows, &|r| build(r, market));
        if let Err(e) = built {
            self.reg.dispose(root);
            sink.take();
            return Err(e);
        }
        self.bots.insert(
            id.to_string(),
            Running {
                market: market.to_string(),
                root,
                loader,
                sink,
                position,
                run,
            },
        );
        Ok(())
    }

    /// Brings running bot `id` in line with its changed rows and budget:
    /// only the rows that changed restart, with what relied on them. A row
    /// that does not build changes nothing.
    pub fn update(&mut self, id: &str, rows: &[Row], budget: u64) -> Result<()> {
        let Some(bot) = self.bots.get_mut(id) else {
            return Ok(());
        };
        let market = bot.market.clone();
        bot.loader
            .reconcile(&mut self.reg, bot.root, rows, &|r| build(r, &market))?;
        bot.run.set(budget, vol_of(rows));
        Ok(())
    }

    /// Stops bot `id`, undoing everything it did; what it proposed on its
    /// way out (its ladder's cancel) goes out with the next [`Host::drain`].
    pub fn stop(&mut self, id: &str) {
        let Some(bot) = self.bots.remove(id) else {
            return;
        };
        self.reg.dispose(bot.root);
        self.leftover
            .extend(bot.sink.take().into_iter().map(|intent| Proposed {
                bot: id.to_string(),
                market: bot.market.clone(),
                intent,
            }));
    }

    pub fn running(&self, id: &str) -> bool {
        self.bots.contains_key(id)
    }

    pub fn any(&self) -> bool {
        !self.bots.is_empty()
    }

    /// The bots running in `market`, with their wallets' feeds.
    pub fn bots_in(&self, market: &str) -> Vec<(String, Arc<Feed<Position>>)> {
        self.bots
            .iter()
            .filter(|(_, b)| b.market == market)
            .map(|(id, b)| (id.clone(), b.position.clone()))
            .collect()
    }

    /// Running bot `id`'s wallet in its market, as last read.
    pub fn position(&self, id: &str) -> Option<Arc<Position>> {
        self.bots.get(id)?.position.get()
    }

    /// The markets bots run in, with their feeds.
    pub fn markets(&self) -> Vec<(String, Feeds)> {
        let wanted: BTreeSet<&String> = self.bots.values().map(|b| &b.market).collect();
        wanted
            .into_iter()
            .filter_map(|m| Some((m.clone(), self.markets.get(m)?.clone())))
            .collect()
    }

    /// Runs every bot once, on feeds just read.
    pub fn tick(&mut self, now: u64) {
        self.ticks += 1;
        self.clock.tick(Tick { n: self.ticks, now });
    }

    /// Everything proposed since the last drain, in the order proposed (the
    /// stopped bots' first, then each running bot's): every order, and one
    /// ladder intent per bot and market. A ladder intent says what the
    /// bot's ladder there should be, so its last one stands for the rest (a
    /// cancel then a new ladder is the new ladder, which replaces the old
    /// one whole); a rolled bot's cancel in the month it left still goes
    /// out. Each order is one of its own.
    pub fn drain(&mut self) -> Vec<Proposed> {
        let mut all = std::mem::take(&mut self.leftover);
        for (id, bot) in &self.bots {
            all.extend(bot.sink.take().into_iter().map(|intent| Proposed {
                bot: id.clone(),
                market: bot.market.clone(),
                intent,
            }));
        }
        let mut last: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for (at, p) in all.iter().enumerate() {
            if p.intent.ladder() {
                last.insert((&p.bot, &p.market), at);
            }
        }
        let keep: BTreeSet<usize> = last.into_values().collect();
        all.into_iter()
            .enumerate()
            .filter(|(at, p)| !p.intent.ladder() || keep.contains(at))
            .map(|(_, p)| p)
            .collect()
    }

    /// Tells bot `id` how its last ladder intent went.
    pub fn tell(&mut self, id: &str, outcome: Outcome) {
        if let Some(bot) = self.bots.get(id) {
            bot.sink.tell(outcome);
        }
    }

    /// What bot `id` says about doing nothing now, if anything.
    pub fn note(&self, id: &str) -> Option<String> {
        self.bots.get(id)?.sink.note()
    }

    /// Bot `id`'s parts, in its rows' order.
    pub fn parts(&self, id: &str) -> Vec<Part> {
        let Some(bot) = self.bots.get(id) else {
            return Vec::new();
        };
        bot.loader
            .rows()
            .filter_map(|(row, fiber)| {
                let s = self.reg.status(fiber)?;
                Some(Part {
                    id: row.to_string(),
                    state: s.state,
                    error: s.error,
                    missing: s.missing,
                })
            })
            .collect()
    }

    /// How many keys are provided, for the tests.
    #[cfg(test)]
    pub fn provided(&self) -> usize {
        self.reg.provided()
    }

    /// How many bots tick, for the tests.
    #[cfg(test)]
    pub fn ticking(&self) -> usize {
        self.clock.subscribers()
    }
}
