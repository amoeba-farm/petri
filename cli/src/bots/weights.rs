//! A bot's weights: the house maker's knobs (`futures-maker`'s `MAKER_VOL`,
//! `MAKER_HALF_WIDTH`, `MAKER_SPREAD_BPS`, `MAKER_TIERS`, `MAKER_TIER_BPS`,
//! `MAKER_TIER_RATIO`, `MAKER_SKEW`, `MAKER_QUOTE_FRACTION` and
//! `MAKER_BUDGET_USDC`), with its defaults. The `weights` row provides them
//! to the bot's other rows; editing one restarts the rows that use it.

use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc};

use ameba_sdk_futures::futures::math::MAX_TIERS;

use super::keys::WEIGHTS;
use crate::compose::{Component, Ctx, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Weights {
    /// The index's yearly volatility the fair values assume.
    pub vol: f64,
    /// Steps either side of the index that quote every tier.
    pub half_width: u16,
    /// Half the spread, in basis points of a full step.
    pub spread_bps: u32,
    /// Depth tiers per step and side.
    pub tiers: u8,
    /// Each tier's extra spread, in basis points of a full step.
    pub tier_bps: u64,
    /// Each tier's lots, as a percentage of the tier before.
    pub tier_ratio: u16,
    /// Atoms each mid leans per lot held, against the inventory.
    pub skew: i64,
    /// The share of its budget the ladder quotes.
    pub fraction: f64,
    /// The most USDC (atoms) the bot uses, in the market and from the
    /// wallet; none: all of it.
    pub budget: Option<u64>,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            vol: 0.40,
            half_width: 32,
            spread_bps: 100,
            tiers: 5,
            tier_bps: 40,
            tier_ratio: 120,
            skew: 20,
            fraction: 1.0,
            budget: None,
        }
    }
}

/// Every knob, in the order the Bots view lists them, with what it does.
pub const KNOBS: [(&str, &str); 9] = [
    ("budget", "Most USDC it uses"),
    ("fraction", "Share of the budget it quotes"),
    ("spread_bps", "Half spread, bps of a step"),
    ("vol", "Yearly volatility it assumes"),
    ("half_width", "Steps each side quoting full depth"),
    ("tiers", "Depth tiers per step"),
    ("tier_bps", "Extra spread per tier, bps"),
    ("tier_ratio", "Lots per tier, % of the last"),
    ("skew", "Lean per lot held, atoms"),
];

fn number<T: std::str::FromStr>(config: &BTreeMap<String, String>, key: &str, or: T) -> Result<T> {
    match config.get(key) {
        Some(text) => text
            .trim()
            .parse()
            .map_err(|_| format!("{key} = {text} is not a number it takes")),
        None => Ok(or),
    }
}

impl Weights {
    /// The weights a row sets, the defaults for the rest.
    pub fn parse(config: &BTreeMap<String, String>) -> Result<Self> {
        if let Some(key) = config.keys().find(|k| !KNOBS.iter().any(|(n, _)| n == k)) {
            return Err(format!("there is no weight called {key}"));
        }
        let d = Self::default();
        let budget = match config.get("budget") {
            None => None,
            Some(text) => {
                let usdc: f64 = text
                    .trim()
                    .parse()
                    .ok()
                    .filter(|v: &f64| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| format!("budget = {text} is not an amount of USDC"))?;
                Some((usdc * 1e6).round() as u64)
            }
        };
        let w = Self {
            vol: number(config, "vol", d.vol)?,
            half_width: number(config, "half_width", d.half_width)?,
            spread_bps: number(config, "spread_bps", d.spread_bps)?,
            tiers: number(config, "tiers", d.tiers)?,
            tier_bps: number(config, "tier_bps", d.tier_bps)?,
            tier_ratio: number(config, "tier_ratio", d.tier_ratio)?,
            skew: number(config, "skew", d.skew)?,
            fraction: number(config, "fraction", d.fraction)?,
            budget,
        };
        // The program's own rules for a ladder's tiers (`Quote`).
        if w.tiers > MAX_TIERS {
            return Err(format!("tiers can be at most {MAX_TIERS}"));
        }
        if w.tiers > 1 && w.tier_ratio == 0 {
            return Err("with more than one tier, tier_ratio can't be 0".into());
        }
        Ok(w)
    }
}

/// The `weights` row.
pub struct WeightsRow(pub Weights);

impl Component for WeightsRow {
    fn provide(&self) -> BTreeSet<String> {
        [WEIGHTS.to_string()].into()
    }

    fn apply(&mut self, ctx: &mut Ctx) -> Result<()> {
        ctx.provide(WEIGHTS, Arc::new(self.0.clone()))
    }
}
