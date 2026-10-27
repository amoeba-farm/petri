//! `strategy/script`: a bot's strategy as a Rhai script (`docs/bots.md`,
//! "`strategy/script`"). Petri compiles it when the bot is written or
//! started and calls its `tick(m)` on every look, with `m` the bot's market
//! and wallet as the host read them. What the script asks for (`long`,
//! `short`, `close`, `cancel`) becomes intents that leave through the bot's
//! sink after the tick, in order, and go out through petri's own order
//! paths and checks (`bots::send`).
//!
//! The sandbox is Rhai's raw engine with only Rhai's pure packages
//! (arithmetic, logic, strings, arrays, maps, math, iterators) and the
//! functions below: no files, no network, no modules, no clock but `m.now`,
//! no `sleep`, no keys. `print` and `debug` go nowhere (the raw engine has
//! no output for them), so a script can't write into `petri bots serve`'s
//! JSON lines.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
};

use ameba_sdk_futures::futures::{
    Side,
    pricing::{sigma_points, survival},
};
use rhai::{
    AST, Array, CallFnOptions, Dynamic, Engine, EvalAltResult, FLOAT, INT, ImmutableString, Map,
    Scope,
    packages::{
        ArithmeticPackage, BasicArrayPackage, BasicBlobPackage, BasicFnPackage,
        BasicIteratorPackage, BasicMapPackage, BasicMathPackage, BasicStringPackage,
        BitFieldPackage, LogicPackage, MoreStringPackage, Package,
    },
};

use super::{
    feeds::{
        BookRead, Clock, Closing, Feed, History, Holding, Index, Intent, Order, Position, Sink,
        Tick,
    },
    host::Run,
    keys::{self, CLOCK, INTENTS, RUN},
};
use crate::{
    compose::{Component, Ctx, Result},
    futures::shape::DEFAULT_LEVERAGE,
};

/// The most operations one call of `tick` may take. A downloaded script
/// that loops forever would otherwise hold the worker every other bot runs
/// on (`docs/bots.md`, "`strategy/script`": "a call to `tick` gets at most
/// 5,000,000 operations"). A tick that reaches it is that look's note; the
/// bot runs on.
pub const MAX_OPERATIONS: u64 = 5_000_000;

/// The largest string, array and map a script may build. A protection, as
/// the operation limit is: within its operations a downloaded script could
/// double a string until petri runs out of memory and aborts with every
/// bot's ladder still up (the user, 2026-10-26: protections against lost
/// funds are the exception to adding limits). They are far above anything a
/// strategy holds: a day of minute samples is 1,440 entries.
pub const MAX_STRING: usize = 1 << 20;
pub const MAX_ITEMS: usize = 100_000;

rhai::def_package! {
    /// Rhai's pure packages: everything of its standard library but the
    /// clock (`BasicTimePackage`) and the language core's `sleep` and
    /// `exit` (`LanguageCorePackage`).
    Pure(lib):
        ArithmeticPackage,
        LogicPackage,
        BasicStringPackage,
        MoreStringPackage,
        BasicIteratorPackage,
        BasicFnPackage,
        BitFieldPackage,
        BasicMathPackage,
        BasicArrayPackage,
        BasicBlobPackage,
        BasicMapPackage
    {}
}

/// The engine every script runs on, before the bot's own functions.
fn sandbox() -> Engine {
    static PURE: OnceLock<Pure> = OnceLock::new();
    let mut engine = Engine::new_raw();
    PURE.get_or_init(Pure::new)
        .register_into_engine(&mut engine);
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_string_size(MAX_STRING);
    engine.set_max_array_size(MAX_ITEMS);
    engine.set_max_map_size(MAX_ITEMS);
    engine
}

/// Compiles `source`: it must define `fn tick(m)`.
pub fn compile(source: &str) -> Result<AST> {
    let ast = sandbox()
        .compile(source)
        .map_err(|e| format!("the script does not compile: {e}"))?;
    if !ast
        .iter_functions()
        .any(|f| f.name == "tick" && f.params.len() == 1)
    {
        return Err("the script needs a fn tick(m)".into());
    }
    Ok(ast)
}

/// What one look's calls left behind, and what `m.change` and
/// `m.prob_above` read.
#[derive(Default)]
struct Look {
    queued: Vec<Intent>,
    /// What `say` set this look, if it was called.
    said: Option<String>,
    index: f64,
    vol: f64,
    years: f64,
    history: Arc<History>,
    /// Wall time now (unix seconds), the history's clock.
    wall: i64,
}

impl Look {
    /// The fractional change of the index over the last `secs`: against the
    /// newest sample at least that old, 0.0 without one.
    fn change(&self, secs: f64) -> f64 {
        if !secs.is_finite() || secs <= 0.0 {
            return 0.0;
        }
        let then = self.wall as f64 - secs;
        match self.history.iter().rev().find(|s| s.t as f64 <= then) {
            Some(s) if s.v != 0.0 => self.index / s.v - 1.0,
            _ => 0.0,
        }
    }

    /// The chance the index settles above `x`: normal around the index,
    /// with `vol` a year (the house maker's model).
    fn prob_above(&self, x: f64) -> f64 {
        survival(
            x,
            self.index,
            sigma_points(self.index, self.vol, self.years),
        )
    }
}

type Shared = Arc<Mutex<Look>>;

fn fail<T>(text: impl Into<String>) -> std::result::Result<T, Box<EvalAltResult>> {
    Err(text.into().into())
}

/// A setting as text: a word, or a number as written.
fn word(v: &Dynamic, what: &str) -> std::result::Result<String, Box<EvalAltResult>> {
    if let Ok(s) = v.as_immutable_string_ref() {
        return Ok(s.trim().to_string());
    }
    if let Ok(n) = v.as_int() {
        return Ok(n.to_string());
    }
    match v.as_float() {
        Ok(x) if x.is_finite() => Ok(format!("{x}")),
        _ => fail(format!("{what} should be a number or text")),
    }
}

/// A whole number of at least 1.
fn count(v: &Dynamic, what: &str) -> std::result::Result<u64, Box<EvalAltResult>> {
    let n = match (v.as_int(), v.as_float()) {
        (Ok(n), _) => n as f64,
        (_, Ok(x)) => x,
        _ => return fail(format!("{what} should be a number")),
    };
    if n.fract() != 0.0 || !(1.0..=1e15).contains(&n) {
        return fail(format!("{what} should be a whole number of at least 1"));
    }
    Ok(n as u64)
}

/// `long(#{...})` / `short(#{...})`: the keys of `petri long` and `petri
/// short`.
fn order(side: Side, o: &Map) -> std::result::Result<Intent, Box<EvalAltResult>> {
    let name = if side == Side::Long { "long" } else { "short" };
    const KEYS: [&str; 8] = [
        "usd", "lots", "leverage", "to", "range", "limit", "post", "slippage",
    ];
    if let Some(key) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return fail(format!("{name} has no setting called {key}"));
    }
    let text = |key: &str| o.get(key).map(|v| word(v, key)).transpose();
    let lots = o.get("lots").map(|v| count(v, "lots")).transpose()?;
    let usd = text("usd")?;
    match (&lots, &usd) {
        (None, None) => return fail(format!("{name} needs usd or lots")),
        (Some(_), Some(_)) => return fail(format!("{name} takes usd or lots, not both")),
        _ => {}
    }
    let leverage = match o.get("leverage") {
        None => DEFAULT_LEVERAGE,
        Some(v) => u32::try_from(count(v, "leverage")?)
            .map_err(|_| Box::<EvalAltResult>::from("leverage is too large"))?,
    };
    let range = text("range")?;
    let to = text("to")?;
    if range.is_some() && (to.is_some() || o.contains_key("leverage")) {
        return fail(format!(
            "{name}: range names the whole range, without leverage or to"
        ));
    }
    let limit = text("limit")?;
    let post = match o.get("post") {
        None => false,
        Some(v) => v
            .as_bool()
            .map_err(|_| Box::<EvalAltResult>::from("post is true or false"))?,
    };
    if post && limit.is_none() {
        return fail(format!("{name}: post needs a limit"));
    }
    Ok(Intent::Open(Order {
        side,
        lots,
        usd,
        leverage,
        to,
        range,
        limit,
        post,
        slippage: text("slippage")?.unwrap_or_else(|| "2".into()),
    }))
}

/// `close(#{...})`: all its positions, or those with `range` or `side`.
fn closing(o: &Map) -> std::result::Result<Intent, Box<EvalAltResult>> {
    if let Some(key) = o
        .keys()
        .find(|k| !["range", "side", "slippage"].contains(&k.as_str()))
    {
        return fail(format!("close has no setting called {key}"));
    }
    let side = match o.get("side").map(|v| word(v, "side")).transpose()? {
        None => None,
        Some(s) if s == "long" => Some(Side::Long),
        Some(s) if s == "short" => Some(Side::Short),
        Some(s) => return fail(format!("side is long or short, not {s}")),
    };
    Ok(Intent::Close(Closing {
        range: o.get("range").map(|v| word(v, "range")).transpose()?,
        side,
        slippage: o.get("slippage").map(|v| word(v, "slippage")).transpose()?,
    }))
}

/// A script's `m.memory`: a map it writes that lasts the run, read and
/// written as `m.memory.n` or `m.memory["n"]`, with `"n" in m.memory`.
/// Not a shared Rhai map: Rhai's size limits refuse shared values inside the
/// data they measure, so it is a type of its own, held by the run, with its
/// own bound on entries ([`MAX_ITEMS`], the same protection).
#[derive(Clone, Default)]
pub struct Memory(Arc<Mutex<Map>>);

impl Memory {
    fn get(&mut self, key: ImmutableString) -> Dynamic {
        self.0
            .lock()
            .expect("memory")
            .get(key.as_str())
            .cloned()
            .unwrap_or(Dynamic::UNIT)
    }

    fn set(
        &mut self,
        key: ImmutableString,
        value: Dynamic,
    ) -> std::result::Result<(), Box<EvalAltResult>> {
        let mut map = self.0.lock().expect("memory");
        if map.len() >= MAX_ITEMS && !map.contains_key(key.as_str()) {
            return Err(format!(
                "m.memory holds at most {} entries",
                crate::futures::group(MAX_ITEMS as u64)
            )
            .into());
        }
        map.insert(key.into(), value);
        Ok(())
    }
}

/// The sandbox with the bot's functions: its actions, `say`, the methods
/// `m.change` and `m.prob_above`, all reading and writing `look`, and
/// `m.memory`.
fn engine(look: &Shared) -> Engine {
    let mut engine = sandbox();
    engine
        .register_type_with_name::<Memory>("Memory")
        .register_indexer_get(Memory::get)
        .register_indexer_set(Memory::set)
        .register_fn("contains", |m: &mut Memory, key: ImmutableString| {
            m.0.lock().expect("memory").contains_key(key.as_str())
        })
        .register_fn("remove", |m: &mut Memory, key: ImmutableString| {
            m.0.lock()
                .expect("memory")
                .remove(key.as_str())
                .unwrap_or(Dynamic::UNIT)
        })
        .register_fn("len", |m: &mut Memory| {
            m.0.lock().expect("memory").len() as INT
        });
    for (name, side) in [("long", Side::Long), ("short", Side::Short)] {
        let look = look.clone();
        engine.register_fn(
            name,
            move |o: Map| -> std::result::Result<(), Box<EvalAltResult>> {
                let intent = order(side, &o)?;
                look.lock().expect("look").queued.push(intent);
                Ok(())
            },
        );
    }
    let l = look.clone();
    engine.register_fn(
        "close",
        move |o: Map| -> std::result::Result<(), Box<EvalAltResult>> {
            let intent = closing(&o)?;
            l.lock().expect("look").queued.push(intent);
            Ok(())
        },
    );
    let l = look.clone();
    engine.register_fn("cancel", move || {
        l.lock().expect("look").queued.push(Intent::CancelOrders);
    });
    let l = look.clone();
    engine.register_fn("say", move |text: ImmutableString| {
        l.lock().expect("look").said = Some(text.trim().to_string());
    });
    let l = look.clone();
    engine.register_fn("change", move |_: &mut Map, secs: INT| {
        l.lock().expect("look").change(secs as f64)
    });
    let l = look.clone();
    engine.register_fn("change", move |_: &mut Map, secs: FLOAT| {
        l.lock().expect("look").change(secs)
    });
    let l = look.clone();
    engine.register_fn("prob_above", move |_: &mut Map, x: FLOAT| {
        l.lock().expect("look").prob_above(x)
    });
    let l = look.clone();
    engine.register_fn("prob_above", move |_: &mut Map, x: INT| {
        l.lock().expect("look").prob_above(x as f64)
    });
    engine
}

fn usdc(atoms: u64) -> Dynamic {
    Dynamic::from_float(atoms as f64 / 1e6)
}

fn number(text: &str) -> Dynamic {
    Dynamic::from_float(text.parse().unwrap_or(0.0))
}

/// `m`: what the script reads this look (`docs/bots.md`, the `m` table).
pub fn m(index: &Index, book: &BookRead, position: &Position, history: &History, run: &Run) -> Map {
    let mut m = Map::new();
    let mut put = |key: &str, value: Dynamic| {
        m.insert(key.into(), value);
    };
    put("market", index.label.clone().into());
    put("symbol", index.symbol.clone().into());
    put("index", Dynamic::from_float(index.value));
    put("step", Dynamic::from_float(index.step));
    put("low", Dynamic::from_float(0.0));
    put(
        "high",
        Dynamic::from_float(f64::from(book.book.header.grid) * index.step),
    );
    put(
        "expiry_in",
        Dynamic::from_int(index.expiry_ts.saturating_sub(index.now) as INT),
    );
    put("now", Dynamic::from_int(index.now as INT));
    put("tradable", Dynamic::from_bool(book.tradable));
    put("cash", usdc(position.wallet.saturating_add(position.free)));
    put("budget", usdc(run.budget()));
    let holding = |h: &Holding| {
        let mut one = Map::new();
        let side = if h.side == Side::Long {
            "long"
        } else {
            "short"
        };
        one.insert("side".into(), side.into());
        one.insert("low".into(), number(&h.low));
        one.insert("high".into(), number(&h.high));
        one.insert("lots".into(), Dynamic::from_int(h.lots as INT));
        one.insert("cost".into(), usdc(h.cost));
        one.insert("value".into(), usdc(h.value));
        one
    };
    let holdings: Array = position
        .holdings
        .iter()
        .map(|h| Dynamic::from_map(holding(h)))
        .collect();
    put("holdings", holdings.into());
    // Every month it holds: this one's, then the others', with `market`.
    let held: Array = position
        .holdings
        .iter()
        .map(|h| (index.label.as_str(), h))
        .chain(position.elsewhere.iter().map(|(m, h)| (m.as_str(), h)))
        .map(|(market, h)| {
            let mut one = holding(h);
            one.insert("market".into(), market.into());
            Dynamic::from_map(one)
        })
        .collect();
    put("held", held.into());
    put("orders", Dynamic::from_int(INT::from(position.orders)));
    put("ladder", Dynamic::from_bool(position.ladder));
    let samples: Array = history
        .iter()
        .map(|s| {
            let mut one = Map::new();
            one.insert("t".into(), Dynamic::from_int(s.t));
            one.insert("v".into(), Dynamic::from_float(s.v));
            Dynamic::from_map(one)
        })
        .collect();
    put("history", samples.into());
    put("vol", Dynamic::from_float(run.vol()));
    // The run's own: what the script writes into it is there next look.
    put("memory", Dynamic::from(run.memory.clone()));
    m
}

/// Wall time, unix seconds.
/// A failed tick, as the bot's note says it.
fn why(e: &EvalAltResult) -> String {
    match e {
        EvalAltResult::ErrorTooManyOperations(_) => format!(
            "Its script took more than {} operations this look; it tries again next look.",
            crate::futures::group(MAX_OPERATIONS)
        ),
        EvalAltResult::ErrorRuntime(value, pos) => format!("Its script threw {value} ({pos})."),
        other => format!("Its script stopped: {other}."),
    }
}

struct Script {
    /// Its row's id: the name its notes go by.
    id: String,
    ast: Arc<AST>,
    engine: Engine,
    look: Shared,
    index: Arc<Feed<Index>>,
    book: Arc<Feed<BookRead>>,
    position: Arc<Feed<Position>>,
    history: Arc<Feed<History>>,
    run: Arc<Run>,
    sink: Arc<Sink>,
    /// Its last `say`, shown until the next.
    said: Option<String>,
}

impl Script {
    fn tick(&mut self, _: &Tick) {
        let (Some(index), Some(book), Some(position)) =
            (self.index.get(), self.book.get(), self.position.get())
        else {
            return;
        };
        let history = self.history.get().unwrap_or_default();
        let m = m(&index, &book, &position, &history, &self.run);
        *self.look.lock().expect("look") = Look {
            index: index.value,
            vol: self.run.vol(),
            years: index.expiry_ts.saturating_sub(index.now) as f64 / super::signals::YEAR,
            history,
            wall: super::send::wall(),
            ..Look::default()
        };
        let ran = self.engine.call_fn_with_options::<Dynamic>(
            CallFnOptions::new(),
            &mut Scope::new(),
            &self.ast,
            "tick",
            (m,),
        );
        let look = std::mem::take(&mut *self.look.lock().expect("look"));
        match ran {
            Ok(_) => {
                if let Some(said) = look.said {
                    self.said = Some(said).filter(|s| !s.is_empty());
                }
                for intent in look.queued {
                    self.sink.propose(intent);
                }
                self.sink.say(&self.id, self.said.clone());
            }
            // Nothing it queued goes out; the note says why, for this look.
            Err(e) => self.sink.say(&self.id, Some(why(&e))),
        }
    }
}

/// The `strategy/script` row: its source, compiled.
pub struct ScriptRow {
    id: String,
    market: String,
    ast: Arc<AST>,
}

impl ScriptRow {
    /// Compiles row `id`'s `source` (its only setting).
    pub fn build(id: &str, config: &BTreeMap<String, String>, market: String) -> Result<Self> {
        if let Some(key) = config.keys().find(|k| *k != "source") {
            return Err(format!("strategy/script has no setting called {key}"));
        }
        let source = config
            .get("source")
            .ok_or("strategy/script needs a source")?;
        Ok(Self {
            id: id.to_string(),
            market,
            ast: Arc::new(compile(source)?),
        })
    }
}

impl Component for ScriptRow {
    fn inject(&self) -> BTreeSet<String> {
        [
            keys::index(&self.market),
            keys::book(&self.market),
            keys::position(&self.market),
            keys::history(&self.market),
            RUN.to_string(),
            INTENTS.to_string(),
            CLOCK.to_string(),
        ]
        .into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let sink: Arc<Sink> = ctx.need(INTENTS)?;
        let clock: Arc<Clock> = ctx.need(CLOCK)?;
        let look = Shared::default();
        let script = Arc::new(Mutex::new(Script {
            id: self.id.clone(),
            ast: self.ast.clone(),
            engine: engine(&look),
            look,
            index: ctx.need(&keys::index(&self.market))?,
            book: ctx.need(&keys::book(&self.market))?,
            position: ctx.need(&keys::position(&self.market))?,
            history: ctx.need(&keys::history(&self.market))?,
            run: ctx.need(RUN)?,
            sink: sink.clone(),
            said: None,
        }));
        let id = self.id.clone();
        ctx.effect(move || Ok(Box::new(move || sink.say(&id, None))))?;
        ctx.effect(move || {
            let id = clock.subscribe(Box::new(move |tick| {
                script.lock().expect("script").tick(tick)
            }));
            Ok(Box::new(move || clock.unsubscribe(id)))
        })
    }
}
