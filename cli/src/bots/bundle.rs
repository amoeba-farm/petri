//! Bundles: one TOML file a bot, in `bots/` beside petri's config. A bundle
//! is the bot: copying it forks it, sharing the file shares it, and petri
//! picks up an edit to a running bot's file as it is saved.
//!
//! ```toml
//! [bot]
//! id = "ladder-ramx-nov26"
//! market = "ramx-nov26"        # a symbol, label or address petri knows
//! name = "RAMX ladder"         # optional: name, about, author, budget, from
//!
//! [[row]]
//! id = "weights"
//! kind = "weights"
//! budget = 100                 # see weights.rs for every knob
//!
//! [[row]]
//! id = "fair"
//! kind = "signal/normal-fair"
//! ```
//!
//! A bundle's [`hash`] is its strategy: the text without the runner's own
//! lines (`id`, `market`, `budget` and `from` in `[bot]`), so a renamed,
//! moved or refunded copy still counts for the bot it came from
//! (`docs/bots.md`, "The bundle").

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use toml_edit::{DocumentMut, Item, Value};

use crate::compose::{Result, loader::Row};

/// What a bot gets when `[bot]` and its weights name no budget: 100 USDC.
pub const DEFAULT_BUDGET: u64 = 100_000_000;

/// The settings `[bot]` takes.
const BOT_KEYS: [&str; 7] = ["id", "market", "name", "about", "author", "budget", "from"];

/// The `[bot]` settings that are the runner's, not the strategy's: the hash
/// leaves them out.
const RUNNER_KEYS: [&str; 4] = ["id", "market", "budget", "from"];

#[derive(Clone, Debug)]
pub struct Bundle {
    pub id: String,
    /// The market as written.
    pub market: String,
    /// Its shown name, if it gives one ([`Bundle::name`] falls back to the id).
    pub name: Option<String>,
    /// One line on what it does: its thesis.
    pub about: Option<String>,
    /// The publishing wallet's public key (part of the hash).
    pub author: Option<String>,
    /// `[bot] budget`, in USDC atoms.
    pub budget: Option<u64>,
    /// The registry id it was installed or published as.
    pub from: Option<String>,
    pub rows: Vec<Row>,
    /// The file's text as read, byte for byte (what the hash covers).
    raw: String,
    doc: DocumentMut,
}

/// A setting's value as written: a word's text, a number's digits.
fn text(item: &Item, key: &str) -> Result<String> {
    let value = item
        .as_value()
        .ok_or_else(|| format!("{key} should be a number or a word"))?;
    if let Some(s) = value.as_str() {
        return Ok(s.to_string());
    }
    if matches!(value, Value::Array(_) | Value::InlineTable(_)) {
        return Err(format!("{key} should be a number or a word"));
    }
    let mut bare = value.clone();
    bare.decor_mut().clear();
    Ok(bare.to_string())
}

fn word(table: &toml_edit::Table, key: &str, what: &str) -> Result<String> {
    table
        .get(key)
        .and_then(|i| i.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{what} needs a {key}"))
}

/// An optional word in `[bot]`.
fn optional(table: &toml_edit::Table, key: &str) -> Result<Option<String>> {
    match table.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|s| Some(s.trim().to_string()).filter(|s| !s.is_empty()))
            .ok_or_else(|| format!("[bot] {key} should be text in quotes")),
    }
}

/// An amount of USDC as written (`100`, `12.5`), in atoms.
pub fn usdc_atoms(text: &str, key: &str) -> Result<u64> {
    text.trim()
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)
        .map(|usdc| (usdc * 1e6).round() as u64)
        .ok_or_else(|| format!("{key} = {text} is not an amount of USDC"))
}

/// Whether `line` is a table header: its first non-blank character is `[`.
fn header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Whether header `line` opens `[bot]`: exactly `[bot]`, blanks around it
/// aside (`[bot] # ...` does not, as the futures API reads it).
fn bot_header(line: &str) -> bool {
    line.trim() == "[bot]"
}

/// Whether `line` sets one of `keys` (`^\s*(key)\s*=`).
fn sets(line: &str, keys: &[&str]) -> bool {
    let rest = line.trim_start();
    keys.iter().any(|k| {
        rest.strip_prefix(k)
            .is_some_and(|after| after.trim_start().starts_with('='))
    })
}

/// `text` without the lines inside `[bot]` that set one of `keys`, every
/// other line kept with its `\n`.
fn without(text: &str, keys: &[&str]) -> String {
    let mut inside = false;
    let mut kept = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if header(line) {
            inside = bot_header(line);
        } else if inside && sets(line, keys) {
            continue;
        }
        kept.push_str(line);
    }
    kept
}

/// The bundle's hash (`docs/bots.md`): SHA-256, lowercase hex, of its text
/// without the runner's lines in `[bot]` (`id`, `market`, `budget`,
/// `from`). The futures API hashes the same bytes the same way.
pub fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(without(text, &RUNNER_KEYS).as_bytes()))
}

impl Bundle {
    pub fn parse(text_: &str) -> Result<Self> {
        let doc: DocumentMut = text_
            .parse()
            .map_err(|e: toml_edit::TomlError| e.message().to_string())?;
        let bot = doc
            .get("bot")
            .and_then(Item::as_table)
            .ok_or("the file needs a [bot] section")?;
        if let Some((key, _)) = bot.iter().find(|(k, _)| !BOT_KEYS.contains(k)) {
            return Err(format!("[bot] has no setting called {key}"));
        }
        let id = word(bot, "id", "[bot]")?;
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("a bot's id is letters, digits, - and _".into());
        }
        let market = word(bot, "market", "[bot]")?;
        let budget = bot
            .get("budget")
            .map(|item| text(item, "budget").and_then(|t| usdc_atoms(&t, "budget")))
            .transpose()?;
        let mut rows = Vec::new();
        if let Some(list) = doc.get("row") {
            let list = list
                .as_array_of_tables()
                .ok_or("rows are [[row]] sections")?;
            for t in list.iter() {
                let id = word(t, "id", "every [[row]]")?;
                let kind = word(t, "kind", &format!("row {id}"))?;
                let disabled = match t.get("disabled") {
                    None => false,
                    Some(i) => i
                        .as_bool()
                        .ok_or_else(|| format!("row {id}: disabled is true or false"))?,
                };
                let mut config = BTreeMap::new();
                for (key, item) in t.iter() {
                    if !["id", "kind", "disabled"].contains(&key) {
                        config.insert(key.to_string(), text(item, key)?);
                    }
                }
                rows.push(Row {
                    id,
                    kind,
                    disabled,
                    config,
                });
            }
        }
        Ok(Self {
            id,
            market,
            name: optional(bot, "name")?,
            about: optional(bot, "about")?,
            author: optional(bot, "author")?,
            budget,
            from: optional(bot, "from")?,
            rows,
            raw: text_.to_string(),
            doc,
        })
    }

    pub fn text(&self) -> String {
        self.raw.clone()
    }

    /// Its shown name: `[bot] name`, else its id.
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// The USDC (atoms) its wallet gets on start: `[bot] budget`, else its
    /// weights row's, else [`DEFAULT_BUDGET`].
    pub fn budget_atoms(&self) -> u64 {
        self.budget
            .or_else(|| {
                self.weights()
                    .and_then(|w| w.get("budget"))
                    .and_then(|t| usdc_atoms(t, "budget").ok())
            })
            .unwrap_or(DEFAULT_BUDGET)
    }

    /// The kinds of its rows that are not disabled, as written
    /// (`strategy/script`, `weights`, ...), once each, in the rows' order
    /// (as the futures API lists them).
    pub fn kinds(&self) -> Vec<String> {
        let mut kinds: Vec<String> = Vec::new();
        for row in self.rows.iter().filter(|r| !r.disabled) {
            if !kinds.contains(&row.kind) {
                kinds.push(row.kind.clone());
            }
        }
        kinds
    }

    /// Its hash ([`hash`]): the strategy, whoever runs it where.
    pub fn hash(&self) -> String {
        hash(&self.raw)
    }

    /// The text a publish sends: the file without `from`.
    pub fn publish_text(&self) -> String {
        without(&self.raw, &["from"])
    }

    /// Sets `[bot] key` to the word `value`, keeping the rest of the file.
    pub fn set_bot(&mut self, key: &str, value: &str) -> Result<()> {
        self.put_bot(key, Value::from(value))
    }

    /// Sets `[bot] budget` to `usdc`, as a number.
    pub fn set_budget(&mut self, usdc: f64) -> Result<()> {
        if !usdc.is_finite() || usdc < 0.0 {
            return Err(format!("budget = {usdc} is not an amount of USDC"));
        }
        let value = if usdc.fract() == 0.0 && usdc < 1e15 {
            Value::from(usdc as i64)
        } else {
            Value::from(usdc)
        };
        self.put_bot("budget", value)
    }

    fn put_bot(&mut self, key: &str, mut value: Value) -> Result<()> {
        if !BOT_KEYS.contains(&key) {
            return Err(format!("[bot] has no setting called {key}"));
        }
        let bot = self
            .doc
            .get_mut("bot")
            .and_then(Item::as_table_mut)
            .ok_or("the file needs a [bot] section")?;
        if let Some(old) = bot.get(key).and_then(Item::as_value) {
            *value.decor_mut() = old.decor().clone();
        }
        bot.insert(key, Item::Value(value));
        *self = Self::parse(&self.doc.to_string())?;
        Ok(())
    }

    /// The weights row's settings, if it has one.
    pub fn weights(&self) -> Option<&BTreeMap<String, String>> {
        self.rows
            .iter()
            .find(|r| r.kind == "weights" && !r.disabled)
            .map(|r| &r.config)
    }

    /// Sets weight `key` to `value` as typed (a number), keeping the rest of
    /// the file as it was.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let typed = value.trim();
        let mut new: Value = match (typed.parse::<i64>(), typed.parse::<f64>()) {
            (Ok(n), _) => n.into(),
            (_, Ok(x)) if x.is_finite() => x.into(),
            _ => return Err(format!("{typed} is not a number")),
        };
        let rows = self
            .doc
            .get_mut("row")
            .and_then(Item::as_array_of_tables_mut)
            .ok_or("the bot has no weights row")?;
        let row = rows
            .iter_mut()
            .find(|t| t.get("kind").and_then(|k| k.as_str()) == Some("weights"))
            .ok_or("the bot has no weights row")?;
        if let Some(old) = row.get(key).and_then(Item::as_value) {
            *new.decor_mut() = old.decor().clone();
        }
        row.insert(key, Item::Value(new));
        *self = Self::parse(&self.doc.to_string())?;
        Ok(())
    }
}

/// The ids of the bots that run, one a line, in `bots/.running`: a worker
/// (the TUI's, or `petri bots serve`) starts them again when it starts. A
/// stop or remove takes an id out; quitting petri keeps it.
pub fn running(folder: &Path) -> Vec<String> {
    std::fs::read_to_string(folder.join(".running"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Puts bot `id` in the running list, or takes it out. The TUI and `serve`
/// may both change the list, so each change holds `.running.lock` while it
/// reads and writes it.
pub fn set_running(folder: &Path, id: &str, on: bool) -> Result<()> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(".running.lock"))
        .map_err(|e| format!("couldn't lock the running list: {e}"))?;
    lock.lock()
        .map_err(|e| format!("couldn't lock the running list: {e}"))?;
    let mut ids = running(folder);
    let had = ids.iter().any(|i| i == id);
    if had == on {
        return Ok(());
    }
    if on {
        ids.push(id.to_string());
    } else {
        ids.retain(|i| i != id);
    }
    let text: String = ids.iter().map(|i| format!("{i}\n")).collect();
    crate::petri_config::write_private_file(&folder.join(".running"), text.as_bytes())
}

/// Where bundles live: `bots/` beside petri's config.
pub fn folder() -> Result<PathBuf> {
    let config = crate::petri_config::config_path()?;
    Ok(config
        .parent()
        .ok_or("petri's config has no folder")?
        .join("bots"))
}

/// A new ladder bot on `market`: the house maker's weights, with a budget
/// of 100 USDC to start from.
pub fn template(id: &str, market: &str) -> String {
    format!(
        r#"# A petri trading bot. Start, stop and edit it in petri's Bots tab, or
# edit this file: petri picks up the change while the bot runs.

[bot]
id = "{id}"
market = "{market}"

# The numbers the bot quotes with (the house maker's knobs).
[[row]]
id = "weights"
kind = "weights"
budget = 100       # most USDC it uses
fraction = 1.0     # share of the budget it quotes
spread_bps = 100   # half spread, basis points of a step
vol = 0.4          # yearly volatility the fair values assume
half_width = 32    # steps each side that quote full depth
tiers = 5          # depth tiers per step
tier_bps = 40      # extra spread per tier
tier_ratio = 120   # lots per tier, % of the tier before
skew = 20          # lean per lot held, atoms

# Fair value of every step: normal around the index.
[[row]]
id = "fair"
kind = "signal/normal-fair"

# Lean against what the ladder holds.
[[row]]
id = "skew"
kind = "signal/inventory-skew"

# The ladder: posts, requotes and takes itself down.
[[row]]
id = "quote"
kind = "strategy/ladder"
"#
    )
}

/// Every bundle in `folder`, by file name: each parsed, or why not.
pub fn read_all(folder: &Path) -> Vec<(PathBuf, String, Result<Bundle>)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let bundle = Bundle::parse(&text);
            Some((path, text, bundle))
        })
        .collect()
}

/// A file name for a new bot called `id` in `folder` that no bot has yet.
pub fn fresh(folder: &Path, id: &str) -> (String, PathBuf) {
    let mut n = 1;
    loop {
        let name = if n == 1 {
            id.to_string()
        } else {
            format!("{id}-{n}")
        };
        let path = folder.join(format!("{name}.toml"));
        if !path.exists() {
            return (name, path);
        }
        n += 1;
    }
}
