//! Terminal charts for `petri chart`: a market's candle closes as an ASCII
//! plot with volume beneath it.

use crate::{cli::ChartRangeValue, futures::api::Candle};

/// The candle bucket for a range (at most 500 buckets per request).
pub fn resolution_for(range: ChartRangeValue) -> u32 {
    match range {
        ChartRangeValue::OneHour => 60,
        ChartRangeValue::TwentyFourHours => 300,
        ChartRangeValue::SevenDays => 3_600,
        ChartRangeValue::ThirtyDays => 14_400,
        ChartRangeValue::All => 86_400,
    }
}

/// `"5m"`, `"1h"`, `"1d"`.
pub fn bucket_label(seconds: u32) -> String {
    match seconds {
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s => format!("{}m", s / 60),
    }
}

/// Fills empty buckets by carrying the previous close forward (the API
/// omits buckets without trades).
pub fn fill_candles(candles: &[Candle], resolution: i64) -> Vec<Candle> {
    let mut out: Vec<Candle> = Vec::new();
    for c in candles {
        if let Some(prev) = out.last().cloned() {
            let mut t = prev.t + resolution;
            while t < c.t && out.len() < 5_000 {
                out.push(Candle {
                    t,
                    o: prev.c,
                    h: prev.c,
                    l: prev.c,
                    c: prev.c,
                    v: 0.0,
                });
                t += resolution;
            }
        }
        out.push(c.clone());
    }
    out
}

/// A number with trailing zeros trimmed: `100.2`, `7`.
pub fn num(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

fn window_seconds(range: ChartRangeValue) -> Option<i64> {
    range.window_ms().map(|ms| (ms / 1_000) as i64)
}

/// Keeps the points inside the range window and at most `max_points`.
pub fn select_points(
    candles: &[Candle],
    range: ChartRangeValue,
    now: i64,
    max_points: usize,
) -> Vec<Candle> {
    let filled = fill_candles(candles, i64::from(resolution_for(range)));
    let from = window_seconds(range).map(|w| now - w).unwrap_or(i64::MIN);
    let inside: Vec<Candle> = filled.into_iter().filter(|c| c.t >= from).collect();
    if inside.len() <= max_points.max(2) {
        return inside;
    }
    let max_points = max_points.max(2);
    let last = inside.len() - 1;
    (0..max_points)
        .map(|i| inside[i * last / (max_points - 1)].clone())
        .collect()
}

pub fn render_ascii_chart(values: &[f64], requested_height: usize, width: usize) -> Vec<String> {
    let height = requested_height.clamp(6, 30);
    let width = width.clamp(20, 120);
    let mut grid = vec![vec![' '; width]; height];
    let (min_y, max_y) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let (min_y, max_y) = if values.is_empty() {
        (0.0, 1.0)
    } else {
        (min_y, max_y)
    };
    let y_for = |value: f64| -> usize {
        if (max_y - min_y).abs() < f64::EPSILON {
            height / 2
        } else {
            let scaled = ((max_y - value) / (max_y - min_y)) * (height - 1) as f64;
            scaled.round().clamp(0.0, (height - 1) as f64) as usize
        }
    };
    for (index, value) in values.iter().enumerate() {
        let x = if values.len() <= 1 {
            width / 2
        } else {
            index * (width - 1) / (values.len() - 1)
        };
        grid[y_for(*value)][x] = '*';
    }
    grid.into_iter()
        .enumerate()
        .map(|(row, cells)| {
            let y = if height <= 1 {
                max_y
            } else {
                max_y - (max_y - min_y) * (row as f64 / (height - 1) as f64)
            };
            format!(
                "{:>9} | {}",
                format!("{y:.2}"),
                cells.into_iter().collect::<String>()
            )
        })
        .collect()
}

pub fn volume_line(volumes: &[f64], width: usize) -> String {
    const LEVELS: [char; 8] = ['.', ':', '-', '=', '+', '*', '#', '@'];
    let max = volumes.iter().copied().fold(0.0, f64::max);
    let width = width.clamp(20, 120);
    let sampled: Vec<f64> = if volumes.len() <= width {
        volumes.to_vec()
    } else {
        let last = volumes.len() - 1;
        (0..width)
            .map(|i| volumes[i * last / (width - 1)])
            .collect()
    };
    let bars: String = sampled
        .iter()
        .map(|v| {
            if max <= 0.0 || *v <= 0.0 {
                '.'
            } else {
                LEVELS[((v / max) * 7.0).round() as usize]
            }
        })
        .collect();
    format!(
        "{:>9} | {bars}  (lots per bucket, max {})",
        "volume",
        num(max)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: i64, v: f64) -> Candle {
        Candle {
            t,
            o: v,
            h: v,
            l: v,
            c: v,
            v: 1.0,
        }
    }

    #[test]
    fn points_fill_gaps_and_respect_the_window() {
        let candles = [c(0, 100.0), c(600, 101.0), c(90_000, 102.0)];
        let points = select_points(&candles, ChartRangeValue::TwentyFourHours, 90_000, 1_000);
        assert!(points.iter().all(|p| p.t >= 90_000 - 86_400));
        assert_eq!(points.last().unwrap().c, 102.0);
        assert!(points.len() > 2);
        assert!(points.windows(2).all(|w| w[1].t - w[0].t == 300));
        let few = select_points(&candles, ChartRangeValue::TwentyFourHours, 90_000, 5);
        assert_eq!(few.len(), 5);
        assert_eq!(few.last().unwrap().c, 102.0);
    }

    #[test]
    fn candles_carry_the_close_forward() {
        let filled = fill_candles(&[c(0, 1.0), c(180, 2.0)], 60);
        assert_eq!(filled.len(), 4);
        assert_eq!(filled[1].c, 1.0);
        assert_eq!(filled[1].v, 0.0);
        assert_eq!(filled[3].c, 2.0);
        assert_eq!(num(100.20), "100.2");
    }

    #[test]
    fn bucket_labels() {
        assert_eq!(bucket_label(300), "5m");
        assert_eq!(bucket_label(14_400), "4h");
        assert_eq!(bucket_label(86_400), "1d");
    }

    #[test]
    fn ascii_chart_has_one_row_per_height_unit() {
        let rows = render_ascii_chart(&[1.0, 2.0, 3.0], 6, 30);
        assert_eq!(rows.len(), 6);
        assert!(rows[0].contains('*') && rows[5].contains('*'));
        assert!(rows[0].trim_start().starts_with("3.00"));
    }
}
