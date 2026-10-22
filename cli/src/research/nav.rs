//! Research's navigator: the oracle's products, each product's months, a
//! month's categories (its products, by bucket) and one category's tasks,
//! built from the oracle's months (phase.rs) and the workspace's work in
//! them; where each agent is in it; and each agent's feed. research_page.rs
//! draws them. Products and months are the oracle's alone: work naming a
//! month the oracle does not list shows nowhere.

use super::model::{App, Event, Work, safe};
use super::phase::Month;
use serde_json::Value;
use std::collections::BTreeMap;

/// The bucket of work across a whole month (no product has a source yet).
pub const ALL: &str = "*";

/// Where the navigator is: nothing (the products), a product, one of its
/// months, or one of that month's categories.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Path {
    /// "ramx".
    pub market: Option<String>,
    /// "RAMX-202612".
    pub expiry: Option<String>,
    /// A bucket id, or ALL.
    pub bucket: Option<String>,
}

impl Path {
    pub fn product(market: &str) -> Self {
        Self {
            market: Some(market.to_string()),
            ..Default::default()
        }
    }
    pub fn month(market: &str, expiry: &str) -> Self {
        Self {
            expiry: Some(expiry.to_string()),
            ..Self::product(market)
        }
    }
    pub fn category(market: &str, expiry: &str, bucket: &str) -> Self {
        Self {
            bucket: Some(bucket.to_string()),
            ..Self::month(market, expiry)
        }
    }
    /// 0 (products) to 3 (a category).
    pub fn depth(&self) -> usize {
        match (&self.market, &self.expiry, &self.bucket) {
            (None, ..) => 0,
            (Some(_), None, _) => 1,
            (Some(_), Some(_), None) => 2,
            _ => 3,
        }
    }
    /// One level up.
    pub fn up(&self) -> Self {
        let mut up = self.clone();
        match self.depth() {
            3 => up.bucket = None,
            2 => up.expiry = None,
            _ => up.market = None,
        }
        up
    }
}

/// One agent's page: where its navigator is, whether it follows the agent,
/// the task picked in a category, and the feed's scroll (None: the newest
/// at the bottom).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    pub path: Path,
    pub follow: bool,
    pub task: Option<String>,
    pub feed_top: Option<usize>,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            path: Path::default(),
            follow: true,
            task: None,
            feed_top: None,
        }
    }
}

/// Where a piece of work sits: its product, month and category.
pub fn place(objective: &Value) -> Option<Path> {
    let market = objective["marketId"].as_str()?.to_lowercase();
    let expiry = objective["expiryId"].as_str()?;
    let bucket = objective["bucketId"]
        .as_str()
        .filter(|b| !b.is_empty())
        .unwrap_or(ALL);
    Some(Path::category(&market, expiry, bucket))
}

/// Where agent `id` is working right now, and on what (a work id); nowhere
/// on work of a month the oracle does not list.
pub fn place_of(app: &App, id: &str) -> Option<(Path, String)> {
    let run = app.activity_of(id)?;
    let objective = objective_of(app, id, &run.work_id, run.run_id.as_deref())?;
    Some((listed(app, place(&objective)?)?, run.work_id.clone()))
}

/// `path`, if the oracle lists its month.
fn listed(app: &App, path: Path) -> Option<Path> {
    month_of(app, path.market.as_deref()?, path.expiry.as_deref()?)?;
    Some(path)
}

/// A work item's objective: as listed, else as agent `agent`'s run took it.
fn objective_of(app: &App, agent: &str, work: &str, run: Option<&str>) -> Option<Value> {
    if let Some(w) = app.snapshot.work.iter().find(|w| w.id == work) {
        return Some(w.objective.clone());
    }
    app.details
        .get(agent)?
        .runs
        .iter()
        .find(|r| Some(r.id.as_str()) == run || r.work_id == work)
        .map(|r| r.research_context["objective"].clone())
        .filter(|o| o.is_object())
}

/// Whether work is open to agents now.
fn open(work: &Work) -> bool {
    work.availability.is_empty() || work.availability == "available"
}

/// The agents at work somewhere under `path` (a prefix), by id, in the
/// sidebar's order.
fn agents_under(app: &App, at: impl Fn(&Path) -> bool) -> Vec<String> {
    app.snapshot
        .agents
        .iter()
        .filter(|a| a.revoked_at.is_none())
        .filter(|a| place_of(app, &a.id).is_some_and(|(p, _)| at(&p)))
        .map(|a| a.id.clone())
        .collect()
}

/// The work under a product (and month, and category).
fn work_at<'a>(
    app: &'a App,
    market: &'a str,
    expiry: Option<&'a str>,
    bucket: Option<&'a str>,
) -> impl Iterator<Item = &'a Work> {
    app.snapshot.work.iter().filter(move |w| {
        place(&w.objective).is_some_and(|p| {
            p.market.as_deref() == Some(market)
                && expiry.is_none_or(|e| p.expiry.as_deref() == Some(e))
                && bucket.is_none_or(|b| p.bucket.as_deref() == Some(b))
        })
    })
}

pub struct ProductRow {
    /// "ramx".
    pub market: String,
    pub months: usize,
    /// Its work open to agents.
    pub tasks: usize,
    pub agents: Vec<String>,
}

/// Every oracle product: those the oracle lists months of.
pub fn products(app: &App) -> Vec<ProductRow> {
    let mut months: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for m in &app.fund.snap.months {
        months
            .entry(m.market.to_lowercase())
            .or_default()
            .insert(m.expiry.clone());
    }
    months
        .into_iter()
        .map(|(market, expiries)| ProductRow {
            tasks: work_at(app, &market, None, None)
                .filter(|w| open(w))
                .count(),
            agents: agents_under(app, |p| p.market.as_deref() == Some(market.as_str())),
            months: expiries.len(),
            market,
        })
        .collect()
}

pub struct MonthRow {
    /// "RAMX-202612".
    pub expiry: String,
    /// Its phase and rewards.
    pub month: Month,
    pub tasks: usize,
    pub agents: Vec<String>,
}

/// A product's months, as the oracle lists them: live ones by date, then
/// settled ones.
pub fn months(app: &App, market: &str) -> Vec<MonthRow> {
    let mut rows: BTreeMap<String, Month> = BTreeMap::new();
    for m in &app.fund.snap.months {
        if m.market.eq_ignore_ascii_case(market) {
            rows.insert(m.expiry.clone(), m.clone());
        }
    }
    let mut rows: Vec<MonthRow> = rows
        .into_iter()
        .map(|(expiry, month)| MonthRow {
            tasks: work_at(app, market, Some(&expiry), None)
                .filter(|w| open(w))
                .count(),
            agents: agents_under(app, |p| {
                p.market.as_deref() == Some(market) && p.expiry.as_deref() == Some(expiry.as_str())
            }),
            month,
            expiry,
        })
        .collect();
    rows.sort_by_key(|r| (r.month.step() == 4, r.expiry.clone()));
    rows
}

pub struct CategoryRow {
    /// A bucket id, or ALL.
    pub bucket: String,
    pub label: String,
    /// Its sources: placed ones of a source gap, else those its month lists.
    pub sources: u64,
    /// A source gap's target.
    pub target: Option<u64>,
    /// Its work, open or not.
    pub tasks: Vec<Work>,
    pub agents: Vec<String>,
}

impl CategoryRow {
    /// Its hottest open task's temperature; none with nothing to do.
    pub fn heat(&self) -> Option<super::render::Heat> {
        self.tasks
            .iter()
            .filter(|w| open(w))
            .map(super::render::Heat::of)
            .min_by_key(|h| *h as u8)
    }
    /// Whether the owner picked any of its tasks ("Research this").
    pub fn focused(&self) -> bool {
        self.tasks.iter().any(|w| w.focused)
    }
}

/// A product's name for a bucket: its label (a chip bucket's name, "DDR5
/// 16Gb", for a chip bucket's label), else the name its id spells, else
/// "Unnamed product b0bf23".
pub fn bucket_label(bucket: &str, label: Option<&str>) -> String {
    use crate::oracle::skus;
    if bucket == ALL {
        return "All products".into();
    }
    label
        .filter(|l| !l.is_empty())
        .map(|l| safe(&skus::name_of(l).unwrap_or_else(|| l.to_string())))
        .or_else(|| skus::id_name(bucket).map(|l| safe(&l)))
        .unwrap_or_else(|| format!("Unnamed product {}", &bucket[..bucket.len().min(6)]))
}

/// The oracle month `expiry` of `market`, if read.
pub fn month_of<'a>(app: &'a App, market: &str, expiry: &str) -> Option<&'a Month> {
    app.fund
        .snap
        .months
        .iter()
        .find(|m| m.market.eq_ignore_ascii_case(market) && m.expiry == expiry)
}

/// A month's categories: every product it lists, has sources for or has
/// work in, the month-wide one first, then by name.
pub fn categories(app: &App, market: &str, expiry: &str) -> Vec<CategoryRow> {
    let month = month_of(app, market, expiry);
    let mut rows: BTreeMap<String, CategoryRow> = BTreeMap::new();
    let row = |rows: &mut BTreeMap<String, CategoryRow>, bucket: &str| {
        rows.entry(bucket.to_string())
            .or_insert_with(|| CategoryRow {
                bucket: bucket.to_string(),
                label: String::new(),
                sources: 0,
                target: None,
                tasks: Vec::new(),
                agents: Vec::new(),
            })
            .bucket
            .clone()
    };
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for sku in month.map(|m| m.skus.as_slice()).unwrap_or_default() {
        row(&mut rows, &sku.bucket);
        if let Some(l) = &sku.label {
            labels.insert(sku.bucket.clone(), l.clone());
        }
    }
    for source in month.map(|m| m.sources.as_slice()).unwrap_or_default() {
        let key = row(&mut rows, &source.bucket);
        if let Some(r) = rows.get_mut(&key) {
            r.sources += 1;
        }
        if let Some(l) = &source.label {
            labels.entry(source.bucket.clone()).or_insert(l.clone());
        }
    }
    for w in work_at(app, market, Some(expiry), None) {
        let Some(bucket) = place(&w.objective).and_then(|p| p.bucket) else {
            continue;
        };
        let key = row(&mut rows, &bucket);
        if let Some(l) = w.objective["bucketLabel"]
            .as_str()
            .filter(|l| !l.is_empty())
        {
            labels.entry(bucket.clone()).or_insert(l.to_string());
        }
        let Some(r) = rows.get_mut(&key) else {
            continue;
        };
        if w.objective["kind"].as_str().unwrap_or("source") == "source"
            && let Some(target) = w.objective["targets"]["activeSources"].as_u64()
        {
            r.target = Some(target);
            r.sources = w.objective["activeSources"].as_u64().unwrap_or(0);
        }
        r.tasks.push(w.clone());
    }
    let mut rows: Vec<CategoryRow> = rows
        .into_values()
        .map(|mut r| {
            r.label = bucket_label(&r.bucket, labels.get(&r.bucket).map(String::as_str));
            r.agents = agents_under(app, |p| {
                p.market.as_deref() == Some(market)
                    && p.expiry.as_deref() == Some(expiry)
                    && p.bucket.as_deref() == Some(r.bucket.as_str())
            });
            r
        })
        .collect();
    rows.sort_by_key(|r| (r.bucket != ALL, r.label.to_lowercase()));
    rows
}

/// The task a category shows: the one picked there, else the one the
/// agent is on, else your pick, else its first.
pub fn task<'a>(app: &App, row: &'a CategoryRow) -> Option<&'a Work> {
    let nav = app.nav();
    let on = app
        .selected_agent
        .as_deref()
        .and_then(|id| app.activity_of(id))
        .map(|a| a.work_id.clone());
    row.tasks
        .iter()
        .find(|w| nav.task.as_ref() == Some(&w.id))
        .or_else(|| row.tasks.iter().find(|w| on.as_ref() == Some(&w.id)))
        .or_else(|| row.tasks.iter().find(|w| w.focused))
        .or_else(|| row.tasks.first())
}

/// A page without its scheme and `www.`: "memorymarket.com/price/rdimm".
pub fn short_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    safe(rest.trim_start_matches("www.").trim_end_matches('/'))
}

/// Work as one short line: "RAMX Jan 2027 · DDR5 RDIMM 64GB", an update
/// or opening with its page's host.
pub fn short_title(objective: &Value, fallback: &str) -> String {
    let Some(path) = place(objective) else {
        return safe(fallback);
    };
    let market = path.market.unwrap_or_default().to_uppercase();
    let expiry = path.expiry.unwrap_or_default();
    let month = super::render::month_name(&expiry).unwrap_or(expiry);
    let label = bucket_label(
        &path.bucket.unwrap_or_default(),
        objective["bucketLabel"].as_str(),
    );
    let page = objective["url"]
        .as_str()
        .filter(|_| matches!(objective["kind"].as_str(), Some("update" | "opening")))
        .map(|u| format!(" · {}", short_url(u).split('/').next().unwrap_or_default()))
        .unwrap_or_default();
    format!("{market} {month} · {label}{page}")
}

/// What agent `id` is on, in short; None while idle.
pub fn doing(app: &App, id: &str) -> Option<String> {
    let run = app.activity_of(id)?;
    Some(
        match objective_of(app, id, &run.work_id, run.run_id.as_deref()) {
            Some(o) => short_title(&o, &run.title),
            None => safe(&run.title),
        },
    )
}

/// How a feed line reads: by whom and how it stands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The agent's own report.
    Agent,
    /// What Petri or the candidate store recorded.
    Petri,
    /// The owner (a directive).
    You,
    /// Something found or received.
    Found,
    /// Blocked.
    Alert,
}

/// One line of an agent's feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub at_ms: u64,
    pub icon: &'static str,
    pub text: String,
    /// Said after the text, quieter.
    pub note: String,
    pub tone: Tone,
    /// Where its run's work is: a click there shows it.
    pub place: Option<Path>,
}

/// One line of text.
fn flat(value: &str) -> String {
    safe(value).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Agent `id`'s feed, oldest first: its events (the owner's directives
/// among them), each a line.
pub fn feed(app: &App, id: &str) -> Vec<Line> {
    let Some(detail) = app.details.get(id) else {
        return Vec::new();
    };
    let mut events: Vec<&Event> = detail.events.iter().collect();
    events.sort_by_key(|e| e.sequence);
    events.into_iter().map(|e| line(app, id, e)).collect()
}

fn line(app: &App, agent: &str, e: &Event) -> Line {
    let url = e.reference_url.as_deref().map(short_url);
    let message = flat(&e.message);
    let kind = if e.actor == "owner" {
        "directive"
    } else {
        e.kind.as_str()
    };
    let (icon, tone, text, note) = match kind {
        "search" => ("⌕", Tone::Agent, message, String::new()),
        "read" => match url {
            Some(url) => ("↗", Tone::Agent, url, message),
            None => ("↗", Tone::Agent, message, String::new()),
        },
        "value" => (
            "$",
            Tone::Agent,
            message,
            url.map(|u| u.split('/').next().unwrap_or_default().to_string())
                .unwrap_or_default(),
        ),
        "source_found" => match url {
            Some(url) => ("✓", Tone::Found, url, message),
            None => ("✓", Tone::Found, message, String::new()),
        },
        "submit" => ("⇪", Tone::Agent, message, String::new()),
        "blocked" => ("!", Tone::Alert, message, String::new()),
        "directive" => (
            "",
            Tone::You,
            if message == "Directive cleared." {
                "directive cleared".into()
            } else {
                message
            },
            String::new(),
        ),
        "run_started" => {
            // The work as the run took it, else its title.
            let run = detail_run(app, agent, e.run_id.as_deref());
            let title = message.strip_prefix("Started: ").unwrap_or(&message);
            let text = run
                .map(|r| {
                    let title = if r.title.is_empty() { title } else { &r.title };
                    short_title(&r.research_context["objective"], title)
                })
                .unwrap_or_else(|| title.to_string());
            ("▸", Tone::Petri, text, String::new())
        }
        "run_finished" => ("■", Tone::Petri, message, String::new()),
        "candidate_received" => ("✓", Tone::Found, "received".into(), url.unwrap_or_default()),
        "candidate_duplicate" => (
            "=",
            Tone::Petri,
            "already known".into(),
            url.unwrap_or_default(),
        ),
        _ => ("·", Tone::Agent, message, url.unwrap_or_default()),
    };
    let note = if note == text { String::new() } else { note };
    Line {
        at_ms: e.created_at,
        icon,
        text,
        note,
        tone,
        place: detail_run(app, agent, e.run_id.as_deref())
            .and_then(|r| listed(app, place(&r.research_context["objective"])?)),
    }
}

fn detail_run<'a>(app: &'a App, agent: &str, run: Option<&str>) -> Option<&'a super::model::Run> {
    let run = run?;
    app.details.get(agent)?.runs.iter().find(|r| r.id == run)
}
