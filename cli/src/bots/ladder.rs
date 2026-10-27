//! `strategy/ladder`: the house maker's whole-grid ladder (`futures-maker`'s
//! quote pass) for one wallet: every depth tier over a core around the
//! fair value, the last tier on every other step, post-only, widening its
//! spread while it would cross the book. What the maker does with the Earn
//! vault's capital and the stakers' credit is left out: a bot quotes with
//! its own wallet's cash, at most its budget.
//!
//! It requotes when the ladder it would post changes (the fair value or its
//! inventory moved) or some of what it quoted traded, and takes its ladder
//! down when it stops.
//!
//! A book holds `MAX_BOOK_LADDER_ARCS` ladder arcs across all its ladders
//! (one per step, side and tier; the program refuses a `Quote` beyond it).
//! A bot takes only the room the other ladders leave when they requote at
//! full depth, so it never keeps the house maker from requoting: it posts
//! the whole-grid ladder its weights ask for if that fits, else the core
//! alone, with fewer tiers, then narrower, and says it was narrowed.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use ameba_sdk_futures::futures::{
    Book, Core, Knot, Ladder, Maker, Step, Tiers,
    pricing::{core_span, fit_lots, ladder_lots, outer_lots, plan_ladder_by, plan_wide_by},
    program::processor::MAX_BOOK_LADDER_ARCS,
};

use super::{
    feeds::{BookRead, Clock, Feed, Intent, Outcome, Position, Sink, Tick},
    host::Run,
    keys::{self, CLOCK, FAIR, INTENTS, RUN, SKEW, WEIGHTS},
    signals::{Fair, Skew},
    weights::Weights,
};

/// The ladder's part name for its notes.
const NOTE: &str = "ladder";
use crate::compose::{Component, Ctx, Result};

/// A ladder's shape: the whole grid around a core, or the core alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Wide { tiers: u8, half_width: u16 },
    Core { tiers: u8, half_width: u16 },
}

impl Shape {
    fn tiers(self) -> u8 {
        match self {
            Self::Wide { tiers, .. } | Self::Core { tiers, .. } => tiers,
        }
    }

    fn half_width(self) -> u16 {
        match self {
            Self::Wide { half_width, .. } | Self::Core { half_width, .. } => half_width,
        }
    }

    /// The arcs it takes posted fresh around `centre`: every tier on both
    /// sides of a core step, one a side elsewhere.
    fn arcs(self, centre: f64, grid: u16) -> usize {
        let (lo, hi) = core_span(centre, grid, self.half_width());
        let core = usize::from(hi - lo) * 2 * usize::from(self.tiers().max(1));
        match self {
            Self::Wide { .. } => core + usize::from(grid - (hi - lo)) * 2,
            Self::Core { .. } => core,
        }
    }

    /// The weights' ladder, then smaller ones, largest first.
    fn all(w: &Weights) -> Vec<Self> {
        let mut all = vec![Self::Wide {
            tiers: w.tiers,
            half_width: w.half_width,
        }];
        all.extend((1..=w.tiers.max(1)).rev().map(|tiers| Self::Core {
            tiers,
            half_width: w.half_width,
        }));
        let mut half_width = w.half_width / 2;
        while half_width > 0 {
            all.push(Self::Core {
                tiers: 1,
                half_width,
            });
            half_width /= 2;
        }
        all
    }
}

/// The arcs a live ladder takes when it requotes at full depth: every tier
/// over its core, one a side elsewhere. Its core is where a step still
/// holds more than its last tier (outside the core a step quotes only
/// that), from the first such step to the last.
fn footprint(mk: &Maker, steps: &[(u16, Step)]) -> usize {
    let n = mk.tiers.n();
    let (bid, ask) = mk.sizes();
    let deep: Vec<u16> = steps
        .iter()
        .filter(|(_, s)| s.bid_lots > bid[n - 1] || s.ask_lots > ask[n - 1])
        .map(|(x, _)| *x)
        .collect();
    let core = deep.first().zip(deep.last());
    (mk.lo..mk.hi)
        .map(|x| match core {
            Some((lo, hi)) if x >= *lo && x <= *hi => 2 * n,
            _ => 2,
        })
        .sum()
}

/// The arcs the book leaves for trader `me`'s ladder: what every other
/// ladder takes at full depth, out of the book's limit.
fn room(b: &Book, me: Option<u16>) -> usize {
    let others: usize = b
        .makers()
        .iter()
        .filter(|(_, mk, _)| mk.seq != 0 && Some(mk.trader) != me)
        .map(|(_, mk, steps)| footprint(mk, steps))
        .sum();
    MAX_BOOK_LADDER_ARCS.saturating_sub(others)
}

/// The ladder last proposed, as planned at the base spread.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Quoted {
    lots: u64,
    outer: u64,
    core: Core,
    plan: (Ladder, Core, Vec<Knot>),
}

pub struct Quoter {
    fair: Arc<dyn Fair>,
    skew: Arc<dyn Skew>,
    book: Arc<Feed<BookRead>>,
    position: Arc<Feed<Position>>,
    weights: Arc<Weights>,
    run: Arc<Run>,
    sink: Arc<Sink>,
    last: Option<Quoted>,
}

impl Quoter {
    fn tick(&mut self, _: &Tick) {
        if matches!(self.sink.last(), Some(Outcome::Refused(_))) {
            // What it last asked for did not land: ask again.
            self.last = None;
        }
        if let Some(intent) = self.plan() {
            self.sink.propose(intent);
        }
    }

    /// What to propose now, if anything (`futures-maker`'s `quote`).
    fn plan(&mut self) -> Option<Intent> {
        let read = self.book.get()?;
        let position = self.position.get()?;
        let model = self.fair.now()?;
        if !read.tradable {
            return None;
        }
        let (b, owner, w) = (&read.book, position.owner, &*self.weights);
        let (k, grid) = (b.header.k, b.header.grid);
        let trader = b.trader_of(&owner).map(|(t, _)| t);
        let ladder = trader.and_then(|t| b.makers().into_iter().find(|(_, mk, _)| mk.trader == t));
        let live = ladder.as_ref().is_some_and(|(_, mk, _)| mk.seq != 0);
        // What it quotes with: its cash here and in its wallet, at most its
        // budget (`[bot] budget`, else its weights'), times its fraction.
        let capital = position.cash.saturating_add(position.wallet);
        let budget = (self.run.budget().min(capital) as f64 * w.fraction) as u64;
        // The largest shape the book has room for.
        let room = room(b, trader);
        let Some(shape) = Shape::all(w)
            .into_iter()
            .find(|s| s.arcs(model.centre, grid) <= room)
        else {
            self.sink.say(
                NOTE,
                Some("No room on this market's book for its ladder now.".into()),
            );
            self.last = None;
            return live.then_some(Intent::Cancel);
        };
        self.sink.say(NOTE, None);
        let fitted = shape != Shape::all(w)[0];
        let (core_lo, core_hi) = core_span(model.centre, grid, shape.half_width());
        let outside = match shape {
            Shape::Wide { .. } => grid - (core_hi - core_lo),
            Shape::Core { .. } => 0,
        };
        let tiers = Tiers {
            count: shape.tiers(),
            ratio: w.tier_ratio,
            spread: ((k * w.tier_bps) / 10_000).max(1) as u32,
        };
        let base = ((k * u64::from(w.spread_bps)) / 10_000).max(1) as u32;
        let plan = |spread: u32, lots: u64, lean: i64| match shape {
            Shape::Wide { half_width, .. } => plan_wide_by(
                &*model.step,
                model.centre,
                k,
                grid,
                half_width,
                spread,
                lean,
                (lots, lots),
                tiers,
            ),
            Shape::Core { half_width, .. } => {
                let (l, knots) = plan_ladder_by(
                    &*model.step,
                    model.centre,
                    k,
                    grid,
                    half_width,
                    spread,
                    lean,
                    (lots, lots),
                    tiers,
                );
                (l, l.full(), knots)
            }
        };
        // The most lots the budget funds as the program escrows them.
        let lots = fit_lots(
            ladder_lots(
                budget,
                k,
                u64::from(core_hi - core_lo),
                u64::from(outside),
                tiers,
            ),
            |n| {
                let (l, core, knots) = plan(base, n, self.skew.lean(n));
                b.ladder_escrow(&owner, &l, core, &knots)
                    .is_some_and(|e| e <= budget)
            },
        );
        if lots == 0 {
            self.last = None;
            return live.then_some(Intent::Cancel);
        }
        let lean = self.skew.lean(lots);
        let planned = plan(base, lots, lean);
        // Some of what a step quoted has traded.
        let traded = match (&ladder, &self.last) {
            (Some((_, mk, steps)), Some(q)) => steps.iter().any(|(x, s)| {
                let quoted = if *x >= q.core.lo && *x < q.core.hi {
                    q.lots
                } else {
                    q.outer
                };
                *x >= mk.lo
                    && *x < mk.hi
                    && ((s.bid_lots > 0 && s.bid_lots < quoted)
                        || (s.ask_lots > 0 && s.ask_lots < quoted))
            }),
            _ => false,
        };
        let due = !live || self.last.as_ref().is_none_or(|q| q.plan != planned) || traded;
        if !due {
            return None;
        }
        // The tightest spread that does not cross the book, from the base,
        // doubling up to half a step; past that the host says why it can't.
        let widest = u32::try_from(k / 2).unwrap_or(u32::MAX);
        let mut spread = base;
        let (ladder, core, knots) = loop {
            let (l, core, knots) = plan(spread, lots, lean);
            if spread >= widest || b.quote_proposal_core(&owner, &l, core, &knots).is_some() {
                break (l, core, knots);
            }
            spread = spread.saturating_mul(2).min(widest);
        };
        let outer = if outside > 0 {
            outer_lots(tiers, lots)
        } else {
            0
        };
        self.last = Some(Quoted {
            lots,
            outer,
            core: planned.1,
            plan: planned,
        });
        Some(Intent::Quote {
            ladder,
            core,
            knots,
            lots,
            outer,
            fitted,
        })
    }
}

/// The `strategy/ladder` row.
pub struct LadderRow {
    pub market: String,
}

impl Component for LadderRow {
    fn inject(&self) -> BTreeSet<String> {
        [
            FAIR.to_string(),
            SKEW.to_string(),
            keys::book(&self.market),
            keys::position(&self.market),
            WEIGHTS.to_string(),
            RUN.to_string(),
            INTENTS.to_string(),
            CLOCK.to_string(),
        ]
        .into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let sink: Arc<Sink> = ctx.need(INTENTS)?;
        let clock: Arc<Clock> = ctx.need(CLOCK)?;
        let quoter = Arc::new(Mutex::new(Quoter {
            fair: ctx.need(FAIR)?,
            skew: ctx.need(SKEW)?,
            book: ctx.need(&keys::book(&self.market))?,
            position: ctx.need(&keys::position(&self.market))?,
            weights: ctx.need(WEIGHTS)?,
            run: ctx.need(RUN)?,
            sink: sink.clone(),
            last: None,
        }));
        // An open ladder is an emission, with no inverse of its own: the
        // cleanup is registered first, so it runs last, after the ticks stop.
        ctx.effect(move || {
            Ok(Box::new(move || {
                sink.say(NOTE, None);
                sink.propose(Intent::Cancel)
            }))
        })?;
        ctx.effect(move || {
            let id = clock.subscribe(Box::new(move |tick| {
                quoter.lock().expect("quoter").tick(tick)
            }));
            Ok(Box::new(move || clock.unsubscribe(id)))
        })
    }
}
