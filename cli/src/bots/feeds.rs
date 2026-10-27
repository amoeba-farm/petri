//! What passes the bot's wall: the feeds the host reads for it, the clock it
//! runs on, and the sink its intents leave by. A bot never holds a key, a
//! signer or a client; it reads feeds and proposes.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use ameba_sdk_futures::futures::{Book, Core, Knot, Ladder, Side};
use solana_program::pubkey::Pubkey;

/// One value, replaced on every read; empty until the first.
pub struct Feed<T> {
    now: Mutex<Option<Arc<T>>>,
}

impl<T> Default for Feed<T> {
    fn default() -> Self {
        Self {
            now: Mutex::new(None),
        }
    }
}

impl<T> Feed<T> {
    pub fn get(&self) -> Option<Arc<T>> {
        self.now.lock().expect("feed").clone()
    }

    pub fn set(&self, value: T) {
        *self.now.lock().expect("feed") = Some(Arc::new(value));
    }
}

/// A market's index (`/v1/markets/:market`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    /// The market's label (`ramx-nov26`) and product (`RAMX`).
    pub label: String,
    pub symbol: String,
    /// The index as a grid position.
    pub centre: f64,
    /// The index, in index points.
    pub value: f64,
    /// Index points per grid step.
    pub step: f64,
    pub expiry_ts: u64,
    /// The chain's time when it was read.
    pub now: u64,
}

/// One index sample: wall time (unix seconds, as the API's index points)
/// and the index in index points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub t: i64,
    pub v: f64,
}

/// Index samples, one a minute for the last day, oldest first: warmed from
/// the API's `/v1/markets/:market/index` when a bot starts there, then the
/// worker's own reads.
pub type History = Vec<Sample>;

/// The market account read from chain, the same for every bot there.
#[derive(Clone, Debug)]
pub struct BookRead {
    pub book: Book,
    /// Whether the market trades now (open, its oracle period live).
    pub tradable: bool,
}

/// One open position of a bot's wallet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub side: Side,
    /// Its range: grid positions, and the index values (text) they are.
    pub lo: u16,
    pub hi: u16,
    pub low: String,
    pub high: String,
    pub lots: u64,
    /// What it cost, and what it pays at the index now, in USDC atoms.
    pub cost: u64,
    pub value: u64,
}

/// A bot's wallet in its market, read for that wallet alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Position {
    /// The wallet it is read for: the bot's own.
    pub owner: Pubkey,
    /// Its cash in the market, free and committed, in USDC atoms.
    pub cash: u64,
    /// The free part of it.
    pub free: u64,
    /// USDC in the wallet, ready to go in.
    pub wallet: u64,
    /// Net lots its ladder holds (long is positive).
    pub inventory: i64,
    /// Its open positions here, and its resting orders.
    pub holdings: Vec<Holding>,
    pub orders: u32,
    /// Its positions in other markets (a product bot's earlier months),
    /// each with its market's label, from the API's account of the wallet.
    pub elsewhere: Vec<(String, Holding)>,
    /// Whether its ladder is up.
    pub ladder: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    pub n: u64,
    /// The chain's time.
    pub now: u64,
}

type Subscriber = Box<dyn FnMut(&Tick) + Send>;

/// Ticks for every bot at once, once the host has read their feeds.
#[derive(Default)]
pub struct Clock {
    subscribers: Mutex<BTreeMap<u64, Subscriber>>,
    next: AtomicU64,
}

impl Clock {
    /// Calls `f` on every tick until [`Clock::unsubscribe`]. `f` must not
    /// subscribe or unsubscribe itself.
    pub fn subscribe(&self, f: Subscriber) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.subscribers.lock().expect("clock").insert(id, f);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.subscribers.lock().expect("clock").remove(&id);
    }

    pub fn tick(&self, tick: Tick) {
        for f in self.subscribers.lock().expect("clock").values_mut() {
            f(&tick);
        }
    }

    #[cfg(test)]
    pub fn subscribers(&self) -> usize {
        self.subscribers.lock().expect("clock").len()
    }
}

/// An order a bot opens: the keys of `petri long` and `petri short`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub side: Side,
    pub lots: Option<u64>,
    /// USDC to spend instead of naming lots, as written.
    pub usd: Option<String>,
    pub leverage: u32,
    pub to: Option<String>,
    /// The whole range, `90-120`.
    pub range: Option<String>,
    pub limit: Option<String>,
    pub post: bool,
    /// Percent, as written.
    pub slippage: String,
}

/// Which of its positions a bot closes: all, or those with this range or
/// side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Closing {
    pub range: Option<String>,
    pub side: Option<Side>,
    pub slippage: Option<String>,
}

/// What a bot asks petri to do in its market.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Post this ladder, replacing the bot's ladder there (post-only: the
    /// host refuses one that would trade at once).
    Quote {
        ladder: Ladder,
        core: Core,
        knots: Vec<Knot>,
        /// Lots a core step quotes, and a step outside it.
        lots: u64,
        outer: u64,
        /// Smaller than its weights ask, to fit the book's room.
        fitted: bool,
    },
    /// Take the bot's ladder down.
    Cancel,
    /// Open a position (`petri long` / `petri short`).
    Open(Order),
    /// Close positions (`petri close`).
    Close(Closing),
    /// Cancel its resting orders (`petri cancel <market> all`).
    CancelOrders,
}

impl Intent {
    /// Whether it says what the ladder should be (only the last of those
    /// counts), rather than being an order of its own.
    pub fn ladder(&self) -> bool {
        matches!(self, Self::Quote { .. } | Self::Cancel)
    }
}

/// What became of the last intent the host sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Posted(String),
    Cancelled(String),
    /// An order went out: what it did, in plain words.
    Sent(String),
    /// Nothing to take down.
    Clear,
    /// Not sent, or failed: why, in plain words.
    Refused(String),
}

/// A bot's way out: it proposes, the host drains, checks, signs and sends,
/// and tells it how that went.
#[derive(Default)]
pub struct Sink {
    proposed: Mutex<Vec<Intent>>,
    /// How its last ladder intent went.
    last: Mutex<Option<Outcome>>,
    /// Why the bot does nothing now, if a part says, by part.
    notes: Mutex<BTreeMap<String, String>>,
}

impl Sink {
    pub fn propose(&self, intent: Intent) {
        self.proposed.lock().expect("sink").push(intent);
    }

    pub fn take(&self) -> Vec<Intent> {
        std::mem::take(&mut *self.proposed.lock().expect("sink"))
    }

    pub fn tell(&self, outcome: Outcome) {
        *self.last.lock().expect("sink") = Some(outcome);
    }

    pub fn last(&self) -> Option<Outcome> {
        self.last.lock().expect("sink").clone()
    }

    /// Part `who`'s note (none: it has nothing to say).
    pub fn say(&self, who: &str, note: Option<String>) {
        let mut notes = self.notes.lock().expect("sink");
        match note.filter(|n| !n.is_empty()) {
            Some(n) => notes.insert(who.to_string(), n),
            None => notes.remove(who),
        };
    }

    /// What its parts say, together.
    pub fn note(&self) -> Option<String> {
        let notes = self.notes.lock().expect("sink");
        (!notes.is_empty()).then(|| notes.values().cloned().collect::<Vec<_>>().join(" · "))
    }
}
