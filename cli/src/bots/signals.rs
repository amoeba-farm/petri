//! Signals: functions of the feeds a strategy quotes from. Each is a row
//! that injects feeds and provides one signal key; a strategy reads the
//! signal and never the feeds behind it, so a different model is a
//! different row.

use std::{collections::BTreeSet, sync::Arc};

use ameba_sdk_futures::futures::pricing::{sigma_points, step_fair};

use super::{
    feeds::{Feed, Index, Position},
    keys::{self, FAIR, SKEW, WEIGHTS},
    weights::Weights,
};
use crate::compose::{Component, Ctx, Result};

pub(super) const YEAR: f64 = 365.0 * 86_400.0;

/// What a strategy prices from at one moment.
pub struct Model {
    /// Where the ladder's core goes: the expected settlement, as a grid
    /// position.
    pub centre: f64,
    /// What the one-step claim `[x, x+1)` is worth, as a fraction of the
    /// full step.
    pub step: Box<dyn Fn(u16) -> f64>,
}

/// `signal:fair`.
pub trait Fair: Send + Sync {
    /// The model now; `None` until its feeds are read.
    fn now(&self) -> Option<Model>;
}

/// `signal:skew`.
pub trait Skew: Send + Sync {
    /// Atoms to lower every mid by, quoting `lots` per step.
    fn lean(&self, lots: u64) -> i64;
}

/// The house maker's model: the index at expiry is normal around today's
/// index, with `vol` a year.
struct NormalFair {
    index: Arc<Feed<Index>>,
    vol: f64,
}

impl Fair for NormalFair {
    fn now(&self) -> Option<Model> {
        let i = self.index.get()?;
        let years = i.expiry_ts.saturating_sub(i.now) as f64 / YEAR;
        let sigma = sigma_points(i.value, self.vol, years) / i.step.max(f64::MIN_POSITIVE);
        let centre = i.centre;
        Some(Model {
            centre,
            step: Box::new(move |x| step_fair(f64::from(x), centre, sigma)),
        })
    }
}

/// `signal/normal-fair`.
pub struct NormalFairRow {
    pub market: String,
}

impl Component for NormalFairRow {
    fn inject(&self) -> BTreeSet<String> {
        [keys::index(&self.market), WEIGHTS.to_string()].into()
    }

    fn provide(&self) -> BTreeSet<String> {
        [FAIR.to_string()].into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let fair: Arc<dyn Fair> = Arc::new(NormalFair {
            index: ctx.need(&keys::index(&self.market))?,
            vol: ctx.need::<Arc<Weights>>(WEIGHTS)?.vol,
        });
        ctx.provide(FAIR, fair)
    }
}

/// The house maker's lean: each lot its ladder holds moves every mid
/// `skew` atoms against it, shared over the lots a step quotes.
struct InventorySkew {
    position: Arc<Feed<Position>>,
    skew: i64,
}

impl Skew for InventorySkew {
    fn lean(&self, lots: u64) -> i64 {
        let held = self.position.get().map_or(0, |p| p.inventory);
        held.saturating_mul(self.skew) / i64::try_from(lots).unwrap_or(i64::MAX).max(1)
    }
}

/// `signal/inventory-skew`.
pub struct InventorySkewRow {
    pub market: String,
}

impl Component for InventorySkewRow {
    fn inject(&self) -> BTreeSet<String> {
        [keys::position(&self.market), WEIGHTS.to_string()].into()
    }

    fn provide(&self) -> BTreeSet<String> {
        [SKEW.to_string()].into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        let skew: Arc<dyn Skew> = Arc::new(InventorySkew {
            position: ctx.need(&keys::position(&self.market))?,
            skew: ctx.need::<Arc<Weights>>(WEIGHTS)?.skew,
        });
        ctx.provide(SKEW, skew)
    }
}
