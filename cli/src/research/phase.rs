//! Where each oracle month is in its life, as a map: taking sources, Scramble
//! (placing, challenging, freezing its sources), Opening, Game, Settled. Read
//! from the futures API's public snapshot (`fund.rs` reads it) with each
//! month's products and sources, drawn in the Markets view for the shown
//! market and as a strip on Research's month rows.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::render::{DIM, MUTED, SELECTED, TEXT};

/// One oracle month, as the snapshot's fact gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Month {
    /// "ramx".
    pub market: String,
    /// "RAMX-202612".
    pub expiry: String,
    /// "SourceSubmission", "Scramble", "Opening", "Game", "Settled", "Closed".
    pub phase: String,
    /// Unix seconds: Scramble's day 1.
    pub scramble_start: Option<i64>,
    /// USDC atoms of rewards, all and still to pay; none without a schedule.
    pub rewards: Option<(u64, u64)>,
    /// The products it takes sources for (while it forms).
    pub skus: Vec<Sku>,
    /// Its sources, as its inventory lists them.
    pub sources: Vec<Source>,
}

/// A product a month takes sources for (its SKU pool).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sku {
    /// Its bucket id, hex.
    pub bucket: String,
    pub label: Option<String>,
}

/// One of a month's sources.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub bucket: String,
    pub label: Option<String>,
    pub url: Option<String>,
    /// "Candidate", "Active", "Frozen", …
    pub status: String,
    /// Its 64-hex id and its address.
    pub id: String,
    pub address: Option<String>,
    /// Whether it counts, on a chip month: the API's routing, else petri's own
    /// of its definition on chain (fund.rs).
    pub checked: Option<Checked>,
}

/// A routing by the commodity spec, as the views show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub status: ameba_commodity_spec::Status,
    /// What decided it, when it does not fit.
    pub reason: Option<String>,
}

impl Checked {
    pub fn of(routing: &ameba_commodity_spec::Routing) -> Self {
        Self {
            status: routing.status,
            reason: (!routing.status.admissible())
                .then(|| routing.deciding().next().map(|r| r.message.clone()))
                .flatten(),
        }
    }

    /// A document's `routing`, as the API gives it.
    pub fn from_api(v: &Value) -> Option<Self> {
        let status = ameba_commodity_spec::Status::parse(v["routing"]["status"].as_str()?)?;
        let reason = (!status.admissible())
            .then(|| {
                v["routing"]["reasons"]
                    .as_array()?
                    .iter()
                    .find(|r| r["status"].as_str() == Some(status.as_str()))?["message"]
                    .as_str()
                    .map(super::model::safe)
            })
            .flatten();
        Some(Self { status, reason })
    }

    /// "fits the bucket", "missing details".
    pub fn words(&self) -> String {
        self.status.summary().to_lowercase()
    }
}

const DAY: i64 = 86_400;

/// A series' expiry id, as the oracle names it: the symbol and the UTC month
/// of the second before expiry ("RAMX", 1798761600: "RAMX-202612").
pub fn expiry_id(symbol: &str, expiry_ts: u64) -> String {
    let at = chrono::DateTime::from_timestamp(expiry_ts as i64 - 1, 0).unwrap_or_default();
    format!("{}-{}", symbol.to_uppercase(), at.format("%Y%m"))
}

fn when(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

fn atoms(v: &Value) -> Option<u64> {
    v.as_str()?.parse().ok()
}

/// A bucket's display label: the API's `bucketName` ("DDR5 16Gb" on a chip
/// month), else its `bucketLabel`.
fn label(v: &Value) -> Option<String> {
    ["bucketName", "bucketLabel"]
        .iter()
        .find_map(|key| v[*key].as_str().filter(|l| !l.is_empty()))
        .map(str::to_string)
}

/// The sources `snapshot`'s `sourceLinks` lists for one month.
fn sources(snapshot: &Value, market: &str, expiry: &str) -> Vec<Source> {
    snapshot["sourceLinks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| {
            l["marketId"].as_str() == Some(market) && l["expiryId"].as_str() == Some(expiry)
        })
        .flat_map(|l| l["sources"].as_array().into_iter().flatten())
        .filter_map(|s| {
            Some(Source {
                bucket: s["bucketId"].as_str()?.to_string(),
                label: label(s),
                url: s["canonicalLocator"]["url"].as_str().map(str::to_string),
                status: s["status"].as_str().unwrap_or_default().to_string(),
                id: s["sourceId"].as_str().unwrap_or_default().to_string(),
                address: s["sourceAddress"].as_str().map(str::to_string),
                checked: Checked::from_api(s),
            })
        })
        .collect()
}

/// Every month of a public snapshot's `state.months`, with its products
/// (`skus`) and sources (`sourceLinks`).
pub fn months(snapshot: &Value) -> Vec<Month> {
    snapshot["state"]["months"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            Some(Month {
                market: f["marketId"].as_str()?.to_string(),
                expiry: f["expiryId"].as_str()?.to_string(),
                phase: f["phase"].as_str()?.to_string(),
                scramble_start: when(&f["schedule"]["scrambleStartUtc"]),
                rewards: f["rewards"].is_object().then(|| {
                    (
                        atoms(&f["rewards"]["totalAtomic"]).unwrap_or(0),
                        atoms(&f["rewards"]["remainingAtomic"]).unwrap_or(0),
                    )
                }),
                skus: f["skus"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|k| {
                        Some(Sku {
                            bucket: k["bucketId"].as_str()?.to_string(),
                            label: label(k),
                        })
                    })
                    .collect(),
                sources: sources(snapshot, f["marketId"].as_str()?, f["expiryId"].as_str()?),
            })
        })
        .collect()
}

impl Month {
    /// The commodity product of a chip month: one whose products (or
    /// sources) are commodity chip buckets. None on part numbers.
    pub fn chip_product(&self) -> Option<ameba_commodity_spec::Product> {
        let chips = self
            .skus
            .iter()
            .map(|k| k.bucket.as_str())
            .chain(self.sources.iter().map(|s| s.bucket.as_str()))
            .any(|bucket| {
                crate::oracle::skus::parse_hex32(bucket, "bucket")
                    .ok()
                    .and_then(|id| crate::oracle::skus::chip_bucket(&id))
                    .is_some()
            });
        chips
            .then(|| ameba_commodity_spec::Product::from_symbol(&self.market))
            .flatten()
    }
}

/// The steps of a month's life, in order.
pub const STEPS: [&str; 5] = ["Taking sources", "Scramble", "Opening", "Game", "Settled"];

impl Month {
    /// Which of `STEPS` it is in.
    pub fn step(&self) -> usize {
        match self.phase.as_str() {
            "SourceSubmission" => 0,
            "Scramble" => 1,
            "Opening" => 2,
            "Game" => 3,
            _ => 4,
        }
    }

    /// Scramble's day (1 to 7) at `now`, unix seconds.
    pub fn scramble_day(&self, now: i64) -> Option<i64> {
        let start = self.scramble_start?;
        (now >= start).then(|| ((now - start) / DAY + 1).min(7))
    }

    /// The step's name, Scramble's with what its day is for.
    fn step_name(&self, step: usize, now: i64) -> String {
        if step == 1 && self.step() == 1 {
            return match self.scramble_day(now) {
                Some(d @ 1..=4) => format!("Scramble: placing, day {d} of 7"),
                Some(d @ 5..=6) => format!("Scramble: challenges, day {d} of 7"),
                Some(_) => "Scramble: freezing, day 7".into(),
                None => "Scramble".into(),
            };
        }
        STEPS[step].to_string()
    }

    /// The map: every step, the month's own highlighted.
    pub fn map(&self, now: i64) -> Line<'static> {
        let here = self.step();
        let mut spans = Vec::new();
        for step in 0..STEPS.len() {
            if step > 0 {
                spans.push(Span::styled(" ─ ", Style::default().fg(DIM)));
            }
            let name = self.step_name(step, now);
            spans.push(if step == here {
                Span::styled(
                    format!(" {name} "),
                    Style::default()
                        .fg(TEXT)
                        .bg(SELECTED)
                        .add_modifier(Modifier::BOLD),
                )
            } else if step < here {
                Span::styled(name, Style::default().fg(MUTED))
            } else {
                Span::styled(name, Style::default().fg(DIM))
            });
        }
        Line::from(spans)
    }

    /// The step in a word or two: "Sources", "Scramble day 2", "Game".
    pub fn short_step(&self, now: i64) -> String {
        match self.step() {
            0 => "Sources".into(),
            1 => match self.scramble_day(now) {
                Some(d) => format!("Scramble day {d}"),
                None => "Scramble".into(),
            },
            step => STEPS[step].into(),
        }
    }

    /// The map in five dots (done ●, now ◉, to come ○), then its step's
    /// name (Research's month rows).
    pub fn strip(&self, now: i64) -> Vec<Span<'static>> {
        let here = self.step();
        let mut dots = String::new();
        let mut spans = Vec::new();
        for step in 0..STEPS.len() {
            if step == here {
                spans.push(Span::styled(
                    std::mem::take(&mut dots),
                    Style::default().fg(MUTED),
                ));
                spans.push(Span::styled(
                    "◉",
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ));
            } else {
                dots.push(if step < here { '●' } else { '○' });
            }
        }
        spans.push(Span::styled(dots, Style::default().fg(DIM)));
        spans.push(Span::styled(
            format!(" {}", self.short_step(now)),
            Style::default().fg(TEXT),
        ));
        spans.retain(|s| !s.content.is_empty());
        spans
    }

    /// Its rewards still to pay, "$520" (USDC); none without any.
    pub fn rewards_left(&self) -> Option<String> {
        match self.rewards {
            Some((_, left)) if left > 0 => Some(format!("${}", usdc(left))),
            _ => None,
        }
    }
}

/// Whole USDC, or two decimals.
fn usdc(atoms: u64) -> String {
    if atoms.is_multiple_of(1_000_000) {
        (atoms / 1_000_000).to_string()
    } else {
        format!("{:.2}", atoms as f64 / 1e6)
    }
}
