//! Plain-text helpers for futures output: aligned tables, times, sparklines
//! and optional color, in Petri's `A | B | C` line style.

use std::io::IsTerminal;

use chrono::{DateTime, Utc};

/// Aligned columns. Right-aligned columns are listed by index.
pub struct Table {
    headers: Vec<String>,
    right: Vec<usize>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            right: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub fn right(mut self, columns: &[usize]) -> Self {
        self.right = columns.to_vec();
        self
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        let width = visible_width;
        let mut widths: Vec<usize> = self.headers.iter().map(|h| width(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(width(cell));
                }
            }
        }
        let render = |cells: &[String]| {
            let mut line = String::new();
            for (i, cell) in cells.iter().enumerate() {
                let w = widths.get(i).copied().unwrap_or(0);
                let pad = w.saturating_sub(width(cell));
                if i > 0 {
                    line.push_str("  ");
                }
                if self.right.contains(&i) {
                    line.push_str(&" ".repeat(pad));
                    line.push_str(cell);
                } else {
                    line.push_str(cell);
                    if i + 1 < cells.len() {
                        line.push_str(&" ".repeat(pad));
                    }
                }
            }
            line.trim_end().to_string()
        };
        let mut out = vec![render(&self.headers)];
        out.extend(self.rows.iter().map(|r| render(r)));
        out
    }
}

/// Characters a cell shows, not counting ANSI color sequences.
pub fn visible_width(text: &str) -> usize {
    let mut n = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            n += 1;
        }
    }
    n
}

pub fn utc(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

pub fn clock(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|t| t.format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// `"19d 4h"`, `"3h 20m"`, `"45s"`.
pub fn span(seconds: i64) -> String {
    let s = seconds.unsigned_abs();
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3_600, (s % 3_600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

/// `"in 19d 4h"` or `"3h 20m ago"`.
pub fn relative(ts: i64, now: i64) -> String {
    if ts >= now {
        format!("in {}", span(ts - now))
    } else {
        format!("{} ago", span(now - ts))
    }
}

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A one-line sparkline of `values`, at most `width` cells (newest last).
pub fn sparkline(values: &[f64], width: usize) -> String {
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if values.is_empty() || width == 0 {
        return String::new();
    }
    let start = values.len().saturating_sub(width);
    let values = &values[start..];
    let (lo, hi) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    values
        .iter()
        .map(|v| {
            if (hi - lo).abs() < f64::EPSILON {
                BARS[3]
            } else {
                BARS[(((v - lo) / (hi - lo)) * 7.0).round() as usize]
            }
        })
        .collect()
}

/// ANSI color for terminals (off with --no-color, NO_COLOR or a pipe).
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    on: bool,
}

impl Paint {
    pub fn new(enabled: bool) -> Self {
        Self {
            on: enabled
                && std::io::stdout().is_terminal()
                && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }
    pub fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }
    pub fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }
    pub fn red(&self, text: &str) -> String {
        self.wrap("31", text)
    }
    /// Green for gains, red for losses.
    pub fn signed(&self, text: &str) -> String {
        if text.starts_with('-') {
            self.red(text)
        } else if text.starts_with('+') {
            self.green(text)
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_align_columns() {
        let mut t = Table::new(&["MARKET", "LOTS"]).right(&[1]);
        t.row(vec!["RAMX Oct 26".into(), "3".into()]);
        t.row(vec!["NANDX".into(), "120".into()]);
        assert_eq!(
            t.lines(),
            vec![
                "MARKET       LOTS",
                "RAMX Oct 26     3",
                "NANDX         120"
            ]
        );
    }

    #[test]
    fn colored_cells_align() {
        assert_eq!(visible_width("\x1b[32m+0.22\x1b[0m"), 5);
        let mut t = Table::new(&["A", "B"]).right(&[1]);
        t.row(vec!["x".into(), "\x1b[31m-1\x1b[0m".into()]);
        t.row(vec!["y".into(), "100".into()]);
        assert_eq!(t.lines()[1], "x   \x1b[31m-1\x1b[0m");
    }

    #[test]
    fn times_and_sparklines() {
        assert_eq!(span(19 * 86_400 + 4 * 3_600 + 5), "19d 4h");
        assert_eq!(relative(100, 40), "in 1m");
        assert_eq!(relative(40, 100), "1m ago");
        assert_eq!(utc(1_793_491_200), "2026-11-01 00:00 UTC");
        assert_eq!(sparkline(&[1.0, 2.0, 3.0], 10), "▁▅█");
        assert_eq!(sparkline(&[5.0, 5.0], 10), "▄▄");
        assert_eq!(sparkline(&[1.0, 2.0, 3.0], 2).chars().count(), 2);
    }
}
