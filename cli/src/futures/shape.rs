//! An order's shape and limits, built the way the site's ticket builds them
//! (`web/src/lib/futures/market.ts`), so `petri buy` and the web send the same
//! order for the same inputs.
//!
//! A claim on `[lo, hi)` (grid positions) pays a long `clamp(S - lo, 0, hi -
//! lo)` steps of `k` atoms per lot at settlement `S`, and a short the rest of
//! the set. Leverage puts the loss-side end at `entry ∓ entry / leverage` (a
//! long's `lo`, a short's `hi`); the profit-side end is the trader's, else the
//! market's suggested range. Entries are fractional positions.

use ameba_sdk_futures::futures::Side;

use super::units::{parse_decimal, trim_decimal};

pub const DEFAULT_LEVERAGE: u32 = 10;

/// The leverages the site's ticket offers (`FUTURES_LEVERAGES`); a market shows
/// those whose loss-side end lands on its grid (`leverage_stops`). The TUI's
/// ticket steps through them; `-x` takes any leverage.
pub const LEVERAGES: [u32; 9] = [1, 2, 3, 5, 10, 20, 25, 50, 100];

/// A market's grid: positions `0..=grid`, `k` atoms per lot per step, one
/// step worth `step` index units at `decimals`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub grid: u16,
    pub k: u64,
    pub step: u64,
    pub decimals: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub lo: u16,
    pub hi: u16,
}

impl Grid {
    /// One complete set of `shape`, per lot.
    pub fn set(&self, shape: Shape) -> u64 {
        u64::from(shape.hi - shape.lo) * self.k
    }

    /// A position's index value: `"98"`, or `"98.5"` on a fractional grid.
    pub fn value(&self, pos: u16) -> String {
        trim_decimal(
            u128::from(pos) * u128::from(self.step),
            u32::from(self.decimals),
        )
    }

    /// An entry in fine units (`1/k` step) as an index value to the cent.
    pub fn entry(&self, fine: u64) -> String {
        let raw = u128::from(fine) * u128::from(self.step) / u128::from(self.k.max(1));
        let unit = 10u128.pow(u32::from(self.decimals));
        let cents = (raw * 100 + unit / 2) / unit;
        format!("{}.{:02}", cents / 100, cents % 100)
    }

    /// An index value as fine units, rounded to the nearest.
    pub fn fine(&self, text: &str) -> Result<u64, String> {
        let raw = parse_decimal(text, u32::from(self.decimals)).map_err(|e| e.0)?;
        let step = u128::from(self.step.max(1));
        let fine = (raw * u128::from(self.k) + step / 2) / step;
        u64::try_from(fine).map_err(|_| format!("`{text}` is too large"))
    }

    /// An index value as a fractional position.
    pub fn pos_f64(&self, text: &str) -> Result<f64, String> {
        Ok(self.fine(text)? as f64 / self.k.max(1) as f64)
    }

    /// An index value that must be a grid position (a range end).
    pub fn end(&self, text: &str) -> Result<u16, String> {
        let raw = parse_decimal(text, u32::from(self.decimals)).map_err(|e| e.0)?;
        let step = u128::from(self.step.max(1));
        if raw % step != 0 {
            return Err(format!(
                "range ends are whole steps of {}; `{text}` is not",
                self.value(1)
            ));
        }
        match u16::try_from(raw / step) {
            Ok(pos) if pos <= self.grid => Ok(pos),
            _ => Err(format!(
                "`{text}` is off the grid; ends run from {} to {}",
                self.value(0),
                self.value(self.grid)
            )),
        }
    }
}

/// `LO-HI` (also `LO:HI` or `LO..HI`) as index values.
pub fn parse_range(text: &str) -> Result<(String, String), String> {
    let t = text.trim();
    let split = t
        .split_once("..")
        .or_else(|| t.split_once(':'))
        .or_else(|| t.split_once('–'))
        .or_else(|| {
            t.char_indices()
                .skip(1)
                .find(|(_, c)| *c == '-')
                .map(|(i, _)| (&t[..i], &t[i + 1..]))
        });
    match split {
        Some((lo, hi)) if !lo.trim().is_empty() && !hi.trim().is_empty() => {
            Ok((lo.trim().to_string(), hi.trim().to_string()))
        }
        _ => Err(format!("`{text}` is not a range like 90-120")),
    }
}

/// An explicit range from its index values.
pub fn explicit(grid: &Grid, lo: &str, hi: &str) -> Result<Shape, String> {
    let (lo, hi) = (grid.end(lo)?, grid.end(hi)?);
    if lo >= hi {
        return Err("the range's low end must be below its high end".into());
    }
    Ok(Shape { lo, hi })
}

fn loss_raw(side: Side, entry: f64, leverage: u32) -> f64 {
    let risk = entry / f64::from(leverage.max(1));
    match side {
        Side::Long => (entry - risk).round(),
        Side::Short => (entry + risk).round(),
    }
}

fn loss_fits(side: Side, entry: f64, leverage: u32, grid: u16) -> bool {
    let raw = loss_raw(side, entry, leverage);
    match side {
        Side::Long => raw >= 0.0 && raw < entry,
        Side::Short => raw <= f64::from(grid) && raw > entry,
    }
}

/// The leverages whose loss-side end lands on the grid, strictly on the loss
/// side of `entry`: `(lowest, highest)`.
pub fn leverage_span(side: Side, entry: f64, grid: u16) -> Option<(u32, u32)> {
    let fits: Vec<u32> = (1..=1_000)
        .filter(|&l| loss_fits(side, entry, l, grid))
        .collect();
    Some((*fits.first()?, *fits.last()?))
}

/// The offered [`LEVERAGES`] whose loss-side end at `entry` lands on the
/// grid, strictly on the loss side of it (the site's `leverageStops`);
/// all of them while there is no entry.
pub fn leverage_stops(side: Side, entry: Option<f64>, grid: u16) -> Vec<u32> {
    match entry.filter(|e| e.is_finite() && *e > 0.0) {
        None => LEVERAGES.to_vec(),
        Some(entry) => LEVERAGES
            .into_iter()
            .filter(|l| loss_fits(side, entry, *l, grid))
            .collect(),
    }
}

/// The stop nearest `wanted` (the site's `nearestLeverage`).
pub fn nearest_leverage(wanted: u32, stops: &[u32]) -> u32 {
    if stops.is_empty() || stops.contains(&wanted) {
        return wanted;
    }
    stops.iter().copied().fold(stops[0], |best, stop| {
        if stop.abs_diff(wanted) < best.abs_diff(wanted) {
            stop
        } else {
            best
        }
    })
}

/// The shape for a ticket: leverage fixes the loss-side end around `entry`;
/// the profit-side end is `profit`, else `suggested`, kept on the profit side
/// of the entry and on the grid.
pub fn build(
    grid: &Grid,
    side: Side,
    entry: f64,
    leverage: u32,
    profit: Option<u16>,
    suggested: Option<u16>,
) -> Result<Shape, String> {
    let g = f64::from(grid.grid);
    if !(entry > 0.0 && entry < g) {
        return Err("there is no price to build the range around yet".into());
    }
    if !loss_fits(side, entry, leverage, grid.grid) {
        let at = grid.entry((entry * grid.k as f64).round() as u64);
        return Err(match leverage_span(side, entry, grid.grid) {
            Some((low, high)) => format!(
                "{leverage}x does not fit a {} from {at}: use {low}x to {high}x",
                side_word(side)
            ),
            None => format!("no leverage fits a {} from {at}", side_word(side)),
        });
    }
    let loss = loss_raw(side, entry, leverage) as u16;
    let long = side == Side::Long;
    let nearest = if long {
        (entry.floor() + 1.0).min(g)
    } else {
        (entry.ceil() - 1.0).max(0.0)
    };
    let profit = match profit {
        Some(p) => {
            let p = f64::from(p);
            if long && p <= entry {
                return Err(format!(
                    "--to must be above {}",
                    grid.entry((entry * grid.k as f64) as u64)
                ));
            }
            if !long && p >= entry {
                return Err(format!(
                    "--to must be below {}",
                    grid.entry((entry * grid.k as f64) as u64)
                ));
            }
            p
        }
        None => {
            let s = suggested.map(f64::from).unwrap_or(if long {
                (entry + entry / 5.0).round()
            } else {
                (entry - entry / 5.0).round()
            });
            if long {
                s.max(nearest).min(g)
            } else {
                s.min(nearest).max(0.0)
            }
        }
    } as u16;
    let shape = if long {
        Shape {
            lo: loss,
            hi: profit,
        }
    } else {
        Shape {
            lo: profit,
            hi: loss,
        }
    };
    if !(f64::from(shape.lo) < entry && entry < f64::from(shape.hi)) {
        return Err("the range must contain the entry".into());
    }
    Ok(shape)
}

pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Long => "long",
        Side::Short => "short",
    }
}

/// The leverage a shape has at an entry: `entry / (entry - lo)` for a long,
/// `entry / (hi - entry)` for a short.
pub fn leverage_at(side: Side, entry: f64, shape: Shape) -> Option<f64> {
    let risk = match side {
        Side::Long => entry - f64::from(shape.lo),
        Side::Short => f64::from(shape.hi) - entry,
    };
    (risk > 0.0).then(|| entry / risk)
}

/// Premium per lot of an entry (fine units) on `shape`: `entry - lo * k`.
pub fn premium_at(grid: &Grid, shape: Shape, entry_fine: u64) -> Result<u64, String> {
    let lo = u64::from(shape.lo) * grid.k;
    let hi = u64::from(shape.hi) * grid.k;
    if entry_fine <= lo || entry_fine >= hi {
        return Err(format!(
            "the limit must be inside the range {}–{}",
            grid.value(shape.lo),
            grid.value(shape.hi)
        ));
    }
    Ok(entry_fine - lo)
}

/// What one lot puts in at a premium: a long the premium, a short the rest
/// of the set.
pub fn outlay(side: Side, premium: u64, set: u64) -> u64 {
    match side {
        Side::Long => premium,
        Side::Short => set.saturating_sub(premium),
    }
}

fn ceil_div(n: u128, d: u128) -> u128 {
    n.div_ceil(d.max(1))
}

/// The limit premium per lot for an order expected at `premium` that may fill
/// up to `bps` worse, worse measured on `stake` (what a lot puts in or takes
/// out). A long's limit is a most it pays, a short's a least it receives;
/// either stays inside `[0, set]`.
pub fn slippage_limit(order: Side, premium: u64, stake: u64, bps: u64, set: u64) -> u64 {
    let slip = ceil_div(u128::from(stake) * u128::from(bps), 10_000) as u64;
    match order {
        Side::Long => premium.saturating_add(slip).min(set),
        Side::Short => premium.saturating_sub(slip),
    }
}

/// An opening market order's limit from its quote: the quoted average
/// premium per lot, worse by the slippage of what a lot puts in.
pub fn open_limit(side: Side, quoted_premium: i128, filled: u64, set: u64, bps: u64) -> u64 {
    if filled == 0 {
        return match side {
            Side::Long => set,
            Side::Short => 0,
        };
    }
    let total = quoted_premium.max(0) as u128;
    let average = match side {
        Side::Long => ceil_div(total, u128::from(filled)),
        Side::Short => total / u128::from(filled),
    }
    .min(u128::from(set)) as u64;
    slippage_limit(side, average, outlay(side, average, set), bps, set)
}

/// A close's limit: closing a long is a short order on its shape (a least
/// premium), closing a short a long order (a most premium), each worse by the
/// slippage of what the lot returns.
pub fn close_limit(claim: Side, premium: u64, set: u64, bps: u64) -> u64 {
    let order = match claim {
        Side::Long => Side::Short,
        Side::Short => Side::Long,
    };
    slippage_limit(order, premium, outlay(claim, premium, set), bps, set)
}

/// One executable depth level: a premium per lot and the lots there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub premium: i128,
    pub lots: u64,
}

/// Whole lots `budget` opens taking `levels` best first, so the order costs
/// no more than the budget: `(lots, total outlay)`.
pub fn lots_for_budget(budget: u64, levels: &[Level], side: Side, set: u64) -> (u64, u64) {
    let (mut left, mut lots, mut cost) = (budget, 0u64, 0u64);
    for level in levels {
        if level.lots == 0 || level.premium < 0 {
            continue;
        }
        let per = outlay(side, level.premium as u64, set);
        if per == 0 {
            continue;
        }
        let take = level.lots.min(left / per);
        lots += take;
        cost += per * take;
        left -= per * take;
        if take < level.lots {
            break;
        }
    }
    (lots, cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: Grid = Grid {
        grid: 200,
        k: 10_000,
        step: 1_000_000,
        decimals: 6,
    };

    #[test]
    fn values_and_entries() {
        assert_eq!(DEV.value(98), "98");
        assert_eq!(DEV.entry(1_020_574), "102.06");
        assert_eq!(DEV.fine("102.0574").unwrap(), 1_020_574);
        assert_eq!(DEV.end("120").unwrap(), 120);
        assert!(DEV.end("120.5").is_err());
        assert!(DEV.end("201").is_err());
        assert_eq!(
            parse_range("90-120").unwrap(),
            ("90".to_string(), "120".to_string())
        );
        assert_eq!(parse_range("90..120").unwrap().1, "120");
        assert!(parse_range("90").is_err());
        assert!(explicit(&DEV, "120", "90").is_err());
    }

    #[test]
    fn shapes_follow_the_site_ticket() {
        // A 10x long from 102.06 loses below round(102.06 - 10.206) = 92 and
        // runs to the suggestion's high end.
        let s = build(&DEV, Side::Long, 102.06, 10, None, Some(142)).unwrap();
        assert_eq!(s, Shape { lo: 92, hi: 142 });
        let s = build(&DEV, Side::Short, 102.06, 10, None, Some(62)).unwrap();
        assert_eq!(s, Shape { lo: 62, hi: 112 });
        // The profit end the trader types wins; it must be on the profit side.
        let s = build(&DEV, Side::Long, 102.06, 5, Some(120), Some(142)).unwrap();
        assert_eq!(s, Shape { lo: 82, hi: 120 });
        assert!(build(&DEV, Side::Long, 102.06, 5, Some(100), None).is_err());
        // A suggestion inside the entry is pushed one step past it.
        let s = build(&DEV, Side::Long, 102.06, 10, None, Some(90)).unwrap();
        assert_eq!(s.hi, 103);
        // 1x long loses everything at 0. A short's loss end that rounds back
        // onto the entry does not fit: from 102.06 the most is 231x.
        assert_eq!(
            build(&DEV, Side::Long, 102.06, 1, None, Some(142))
                .unwrap()
                .lo,
            0
        );
        let e = build(&DEV, Side::Short, 102.06, 500, None, Some(62)).unwrap_err();
        assert!(e.contains("use 2x to 231x"), "{e}");
        // A short at 1x from 120 would end above the grid.
        assert!(build(&DEV, Side::Short, 120.0, 1, None, None).is_err());
        assert_eq!(leverage_span(Side::Short, 120.0, 200).unwrap().0, 2);
    }

    #[test]
    fn leverage_stops_follow_the_site() {
        assert_eq!(leverage_stops(Side::Long, None, 200), LEVERAGES.to_vec());
        // From 102.06 every offered long fits; a 1x short would end above
        // the grid's 200.
        assert_eq!(
            leverage_stops(Side::Long, Some(102.06), 200),
            LEVERAGES.to_vec()
        );
        assert_eq!(
            leverage_stops(Side::Short, Some(120.0), 200),
            vec![2, 3, 5, 10, 20, 25, 50, 100]
        );
        // Near the bottom of the grid a high-leverage long rounds back onto
        // its entry.
        assert!(!leverage_stops(Side::Long, Some(3.0), 200).contains(&100));
        assert_eq!(nearest_leverage(10, &[2, 3, 5, 20]), 5);
        assert_eq!(nearest_leverage(10, &[2, 10]), 10);
        assert_eq!(nearest_leverage(10, &[]), 10);
    }

    #[test]
    fn limits_match_the_site() {
        let shape = Shape { lo: 89, hi: 120 };
        let set = DEV.set(shape);
        assert_eq!(set, 310_000);
        // 100 lots quoted at 14,310,200 atoms: 143,102 a lot, +2% of it.
        assert_eq!(
            open_limit(Side::Long, 14_310_200, 100, set, 200),
            143_102 + 2_863
        );
        // A short pays the rest of the set; its limit drops by 2% of that.
        assert_eq!(
            open_limit(Side::Short, 14_310_200, 100, set, 200),
            143_102 - 3_338
        );
        assert_eq!(open_limit(Side::Long, 0, 0, set, 200), set);
        assert_eq!(close_limit(Side::Long, 100_000, set, 200), 98_000);
        assert_eq!(close_limit(Side::Short, 100_000, set, 200), 104_200);
        assert_eq!(premium_at(&DEV, shape, 1_000_000).unwrap(), 110_000);
        assert!(premium_at(&DEV, shape, 1_300_000).is_err());
        assert_eq!(
            leverage_at(Side::Long, 100.0, Shape { lo: 90, hi: 120 }),
            Some(10.0)
        );
    }

    #[test]
    fn budgets_walk_the_depth() {
        let levels = [
            Level {
                premium: 100_000,
                lots: 3,
            },
            Level {
                premium: 120_000,
                lots: 10,
            },
        ];
        // $1 buys 3 at 0.10 and 5 at 0.12.
        assert_eq!(
            lots_for_budget(1_000_000, &levels, Side::Long, 310_000),
            (8, 900_000)
        );
        assert_eq!(
            lots_for_budget(50_000, &levels, Side::Long, 310_000),
            (0, 0)
        );
    }
}
