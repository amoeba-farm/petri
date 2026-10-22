//! Comparison series of the chip months. RAMX and NANDX months expiring after
//! 2026-11-01 are on commodity chip buckets (the user moved them from modules
//! and SSDs to chips on 2026-10-20), and each of their sources is one
//! comparison series of the spec, `oracle/governance/commodity_submission_spec.v1.json`.
//! A series' source carries [`SeriesDefinition::definition_text`] as its
//! definition, its page (`origin.locator`) as its locator and
//! [`SeriesDefinition::source_type`] as its source type; nothing changes on
//! chain. The spec's routing is the commodity spec crate's: petri runs it
//! before it sends, so a bond never goes on a series the spec disqualifies,
//! and the TUI shows it next to each source.

use std::path::Path;

use ameba_commodity_spec::{
    self as spec, Bucket, MAX_DEFINITION_TEXT_BYTES, MonthContext, MonthSeries, Product, Reason,
    Routing, SeriesDefinition, Stage, Status,
};
use ameba_oracle::state::{OraclePhase, OracleSourceStatus};
use serde_json::{Value, json};

use super::{
    args::Target,
    chain::{MonthRef, Oracle, SourceRef},
    send,
    skus::hex,
    writes::{self, Proposal, month_json, month_line, usdc_amount},
};
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::usdc,
    view::{Paint, Table},
};

/// A source's definition, as routing reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Series(Box<SeriesDefinition>),
    /// It names the series schema but does not parse as one.
    Malformed(String),
    /// Free text: an ordinary (part-number era) definition.
    FreeText,
    /// Not on chain (a dev bootstrap source links only its locator), or not
    /// matching the source's commitment: nothing to route, so no routing.
    Missing,
}

impl Parsed {
    pub fn of(text: Option<&str>) -> Self {
        match text {
            None => Parsed::Missing,
            Some(text) if SeriesDefinition::claims_schema(text) => {
                match SeriesDefinition::parse(text) {
                    Ok(series) => Parsed::Series(Box::new(series)),
                    Err(error) => Parsed::Malformed(error),
                }
            }
            Some(_) => Parsed::FreeText,
        }
    }

    pub fn series(&self) -> Option<&SeriesDefinition> {
        match self {
            Parsed::Series(series) => Some(series),
            _ => None,
        }
    }
}

/// Where a month's series set stands: open while it takes sources
/// (SourceSubmission, Scramble), committed from its freeze on (the freeze
/// sets the recipe and moves the month to Opening).
pub fn stage(phase: OraclePhase) -> Stage {
    match phase {
        OraclePhase::Uninitialized | OraclePhase::SourceSubmission | OraclePhase::Scramble => {
            Stage::Open
        }
        _ => Stage::Committed,
    }
}

/// [`stage`] of a phase as the API names it ("Scramble", "Game").
pub fn stage_named(phase: &str) -> Stage {
    match phase {
        "" | "Uninitialized" | "SourceSubmission" | "Scramble" => Stage::Open,
        _ => Stage::Committed,
    }
}

/// Whether a source with this status is in its month's recipe, the committed
/// series set: frozen, opening, live, or frozen and set aside.
pub fn in_recipe(status: OracleSourceStatus) -> bool {
    matches!(
        status,
        OracleSourceStatus::Frozen
            | OracleSourceStatus::OpeningPending
            | OracleSourceStatus::Active
            | OracleSourceStatus::Inactive
    )
}

/// [`in_recipe`] of a status as the API names it ("Frozen").
pub fn in_recipe_named(status: &str) -> bool {
    matches!(status, "Frozen" | "OpeningPending" | "Active" | "Inactive")
}

/// The routing of a chip month's source whose definition is on chain but
/// is not a series: it gives no settlement contribution until it is one.
pub fn not_a_series(error: Option<&str>) -> Routing {
    let message = match error {
        Some(error) => format!("not a comparison series: {error}"),
        None => "not a comparison series".to_string(),
    };
    Routing {
        status: Status::NeedsMetadata,
        reasons: vec![Reason {
            status: Status::NeedsMetadata,
            field: "definition".into(),
            message,
        }],
    }
}

/// One source of a chip month, for routing.
#[derive(Clone, Debug)]
pub struct Entry {
    /// 64 hex.
    pub source_id: String,
    pub parsed: Parsed,
    /// In the month's recipe.
    pub committed: bool,
}

/// The month routing sees: `entries` in source-id order, the order the API's
/// inventory lists them in. (The chain keeps no proposal order; both sides
/// take this one, so an earlier source keeps a repeated series' weight the
/// same way everywhere.)
fn ordered(entries: &[Entry]) -> Vec<MonthSeries<'_>> {
    let mut sources: Vec<MonthSeries<'_>> = entries
        .iter()
        .map(|entry| MonthSeries {
            source_id: &entry.source_id,
            definition: entry.parsed.series(),
            committed: entry.committed,
        })
        .collect();
    sources.sort_by(|a, b| a.source_id.cmp(b.source_id));
    sources
}

/// Every entry's routing, in `entries`' order: a series against its month
/// (as its own source), another definition on chain [`not_a_series`], and
/// none for a source whose definition is not on chain (there is nothing to
/// route; the API gives it `routing: null` too).
pub fn route_entries(product: Product, stage: Stage, entries: &[Entry]) -> Vec<Option<Routing>> {
    let sources = ordered(entries);
    let month = MonthContext {
        product,
        stage,
        sources: &sources,
    };
    entries
        .iter()
        .map(|entry| match &entry.parsed {
            Parsed::Series(series) => Some(spec::route_series_in_month(
                series,
                &month,
                Some(&entry.source_id),
            )),
            Parsed::Malformed(error) => Some(not_a_series(Some(error))),
            Parsed::FreeText => Some(not_a_series(None)),
            Parsed::Missing => None,
        })
        .collect()
}

/// A series against a month's `entries`: as source `this` when it is one of
/// them, else as a new proposal.
pub fn route_against(
    series: &SeriesDefinition,
    product: Product,
    stage: Stage,
    entries: &[Entry],
    this: Option<&str>,
) -> Routing {
    let sources = ordered(entries);
    let month = MonthContext {
        product,
        stage,
        sources: &sources,
    };
    spec::route_series_in_month(series, &month, this)
}

/// "Fits the bucket (CLASSIFIED_CANDIDATE)".
pub fn status_line(routing: &Routing) -> String {
    format!("{} ({})", routing.status.summary(), routing.status)
}

/// Every reason, `field: message`, one per line.
pub fn reason_lines(routing: &Routing) -> Vec<String> {
    routing
        .reasons
        .iter()
        .map(|r| format!("{}: {}", r.field, r.message))
        .collect()
}

/// The refusal when a routing is not worth a bond.
pub(super) fn refuse(routing: &Routing, what: &str) -> Error {
    let mut message = format!(
        "refusing: {what} routes as {}, so it would not count and its bond would be at risk",
        status_line(routing)
    );
    for line in reason_lines(routing) {
        message.push_str("\n  ");
        message.push_str(&line);
    }
    Error::msg(message)
}

fn routing_json(routing: &Routing) -> Value {
    serde_json::to_value(routing).unwrap_or(Value::Null)
}

/// A month's sources with their definitions, read from chain.
pub(super) fn month_entries(oracle: &Oracle, month: &MonthRef) -> Result<Vec<(SourceRef, Parsed)>> {
    let mut sources = oracle.sources(month)?;
    sources.sort_by_key(|s| s.state.source_id);
    let definitions = oracle.definitions(&sources)?;
    Ok(sources
        .into_iter()
        .zip(definitions)
        .map(|(source, text)| (source, Parsed::of(text.as_deref())))
        .collect())
}

pub(super) fn entries_of(read: &[(SourceRef, Parsed)]) -> Vec<Entry> {
    read.iter()
        .map(|(source, parsed)| Entry {
            source_id: hex(&source.state.source_id),
            parsed: parsed.clone(),
            committed: in_recipe(source.state.status),
        })
        .collect()
}

/// The month's commodity product, or why it has no series.
pub(super) fn chip_product(oracle: &Oracle, month: &MonthRef) -> Result<Product> {
    oracle.chip_product(month)?.ok_or_else(|| {
        Error::msg(format!(
            "{} is not on commodity buckets (it keeps its part numbers or categories), so its sources are not comparison series",
            month.period.expiry_id()
        ))
    })
}

/// A commodity bucket by its id ("DDR5_16GBIT"), name ("DDR5 16Gb") or 64-hex id, in any product.
/// A product's research candidate (RAMX's DDR5 32Gb since 2026-10-21) counts
/// as found: its series is evidence for a later decision.
pub fn find_bucket(text: &str) -> Result<&'static Bucket> {
    let text = text.trim();
    let id = super::skus::parse_bucket(text).ok();
    Product::ALL
        .iter()
        .flat_map(|product| {
            product
                .buckets()
                .iter()
                .chain(product.research_candidates())
        })
        .find(|bucket| Some(bucket.id32()) == id)
        .ok_or_else(|| {
            let all: Vec<String> = Product::ALL
                .iter()
                .map(|product| {
                    format!(
                        "{} {}",
                        product.symbol(),
                        product
                            .buckets()
                            .iter()
                            .map(|b| b.id)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect();
            Error::msg(format!(
                "`{text}` is not a commodity bucket; the buckets are {}",
                all.join("; ")
            ))
        })
}

fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display())))
}

pub(super) fn read_series(path: &Path) -> Result<SeriesDefinition> {
    SeriesDefinition::parse(&read_file(path)?)
        .map_err(|e| Error::msg(format!("{}: {e}", path.display())))
}

/// Whether `bucket` is a research candidate of its product: one the spec
/// defines that the product doesn't register, so a series of it routes as
/// RESEARCH_ONLY.
fn research_candidate(bucket: &Bucket) -> bool {
    bucket.product.research_candidate(bucket.id).is_some()
}

/// Writes `value` as pretty JSON to `out`, or returns it as the plain output.
pub(super) fn json_out(value: &Value, out: Option<&Path>, done: String) -> Result<String> {
    let text = serde_json::to_string_pretty(value).expect("JSON encodes");
    match out {
        Some(path) => {
            std::fs::write(path, format!("{text}\n"))
                .map_err(|e| Error::msg(format!("cannot write {}: {e}", path.display())))?;
            Ok(done)
        }
        None => Ok(text),
    }
}

/// `petri oracle series template --bucket ID [--direct] [--out FILE]`.
pub fn template(bucket: &str, direct: bool, out: Option<&Path>) -> Result<Report> {
    let bucket = find_bucket(bucket)?;
    let template = spec::series_template(bucket, direct);
    let file = out.map(|p| p.display().to_string());
    // A research candidate's template is written too, with what it is for.
    let note = research_candidate(bucket).then(|| {
        format!(
            "{} is a research candidate of {}, not one of its buckets: a series of it routes as Research only, carries no settlement weight, and series propose refuses it.",
            bucket.name,
            bucket.product.symbol()
        )
    });
    let plain = json_out(
        &template,
        out,
        format!(
            "Wrote a {} series template to {}. Fill in every null, then check it: petri oracle series check {} <MONTH>{}",
            bucket.title(),
            file.as_deref().unwrap_or_default(),
            file.as_deref().unwrap_or_default(),
            note.as_ref().map(|n| format!("\n{n}")).unwrap_or_default(),
        ),
    )?;
    // On stdout the template stays plain JSON; the note goes beside it.
    if let (Some(note), None) = (&note, out) {
        eprintln!("{note}");
    }
    let payload = json!({
        "ok": true,
        "bucket": {
            "id": bucket.id,
            "name": bucket.name,
            "product": bucket.product.symbol(),
            "weightBps": bucket.weight_bps,
            "researchCandidate": research_candidate(bucket),
        },
        "note": note,
        "template": template,
        "file": file,
    });
    Ok(Report::new(payload, plain))
}

/// The product a series' bucket belongs to.
fn product_of(series: &SeriesDefinition) -> Result<Product> {
    let bucket = series.bucket.as_deref().map(str::trim).unwrap_or_default();
    Product::ALL
        .into_iter()
        .find(|product| {
            product.bucket(bucket).is_some() || product.research_candidate(bucket).is_some()
        })
        .ok_or_else(|| {
            Error::msg(format!(
                "its bucket {bucket:?} is not a commodity bucket: name the month to check it against (petri oracle series check FILE RAMX-202612)"
            ))
        })
}

/// The source of `entries` this series already is: the one whose definition
/// is exactly its text.
fn on_chain<'a>(
    series: &SeriesDefinition,
    read: &'a [(SourceRef, Parsed)],
) -> Option<&'a SourceRef> {
    let hash = writes::evidence_preimage(
        super::args::EvidenceKind::Definition,
        &series.definition_text(),
    )
    .ok()?
    .2;
    read.iter()
        .map(|(source, _)| source)
        .find(|source| source.state.source_definition_hash == hash)
}

/// `petri oracle series check FILE [MONTH]`.
pub fn check(
    ctx: &Ctx,
    p: Paint,
    file: &Path,
    market: Option<&str>,
    expiry: Option<&str>,
) -> Result<Report> {
    let series = read_series(file)?;
    let text = series.definition_text();
    let (routing, against, month_value, source) = match market {
        None => {
            let product = product_of(&series)?;
            (
                spec::route_series(&series, product),
                format!("{} on its own (a month decides the rest)", product.symbol()),
                Value::Null,
                None,
            )
        }
        Some(market) => {
            let oracle = Oracle::new(ctx)?;
            let month = oracle.month(market, expiry)?;
            let product = chip_product(&oracle, &month)?;
            let read = month_entries(&oracle, &month)?;
            let this = on_chain(&series, &read).map(|s| hex(&s.state.source_id));
            let routing = route_against(
                &series,
                product,
                stage(month.state.phase),
                &entries_of(&read),
                this.as_deref(),
            );
            (
                routing,
                format!(
                    "{} ({}, {} sources){}",
                    month.period.expiry_id(),
                    month.phase_name(),
                    read.len(),
                    if this.is_some() {
                        ", as its source there"
                    } else {
                        ", as a new source"
                    }
                ),
                month_json(&month),
                this,
            )
        }
    };
    let bucket = Product::ALL
        .into_iter()
        .find_map(|product| series.bucket_in(product));
    let mut lines = vec![
        p.bold(&status_line(&routing)),
        format!("  Checked against {against}"),
        format!(
            "  Series {} · {}",
            series.series_id.as_deref().unwrap_or("(no id)"),
            bucket.map_or_else(
                || series
                    .bucket
                    .clone()
                    .unwrap_or_else(|| "(no bucket)".into()),
                Bucket::title
            )
        ),
        format!("  Source type {}", series.source_type()),
        format!(
            "  Definition {} of {MAX_DEFINITION_TEXT_BYTES} bytes",
            text.len()
        ),
    ];
    if text.len() > MAX_DEFINITION_TEXT_BYTES {
        lines.push(p.red("  Too long for a source's definition: shorten its notes or policies"));
    }
    for line in reason_lines(&routing) {
        lines.push(format!("  {line}"));
    }
    if routing.status.admissible() && market.is_some() && source.is_none() {
        lines.push(p.dim(
            "Propose it with `petri oracle series propose <MONTH> --file <FILE>` while the month takes sources.",
        ));
    }
    let payload = json!({
        "ok": true,
        "file": file.display().to_string(),
        "month": month_value,
        "sourceId": source,
        "routing": routing_json(&routing),
        "summary": routing.status.summary(),
        "admissible": routing.status.admissible(),
        "sourceType": series.source_type(),
        "definitionBytes": text.len(),
        "maxDefinitionBytes": MAX_DEFINITION_TEXT_BYTES,
    });
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle series propose MONTH --file FILE [--bond USDC]`.
pub fn propose(
    ctx: &Ctx,
    p: Paint,
    target: &Target,
    file: &Path,
    bond: Option<&str>,
    source_id: Option<&str>,
) -> Result<Report> {
    let series = read_series(file)?;
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(&target.market, target.expiry.as_deref())?;
    let product = chip_product(&oracle, &month)?;
    let read = month_entries(&oracle, &month)?;
    let routing = route_against(
        &series,
        product,
        stage(month.state.phase),
        &entries_of(&read),
        None,
    );
    // A series the spec does not admit never counts: refusing it here keeps
    // the listing bond off a disqualified submission (as petri refuses an
    // opening the oracle's rules would reject). Routing itself refuses nothing.
    if !routing.status.admissible() {
        return Err(refuse(&routing, "this series"));
    }
    let text = series.definition_text();
    if text.len() > MAX_DEFINITION_TEXT_BYTES {
        bail(format!(
            "the series' definition is {} bytes, over the {MAX_DEFINITION_TEXT_BYTES} a source holds: shorten its notes or policies",
            text.len()
        ))?;
    }
    let bucket = series
        .bucket_in(product)
        .ok_or_else(|| Error::msg("the series names no bucket of this product"))?;
    let url = series
        .origin
        .locator
        .as_deref()
        .map(str::trim)
        .ok_or_else(|| Error::msg("the series names no page (origin.locator)"))?;
    let bucket_id = bucket.id32();
    let listing_bond = oracle.sku_pool(&month, &bucket_id)?.listing_bond;
    // The program takes exactly the product's listing bond.
    if let Some(bond) = bond
        && usdc_amount(bond, "the listing bond")? != listing_bond
    {
        let bond = crate::futures::units::trim_decimal(u128::from(listing_bond), 6);
        bail(format!(
            "the listing bond of this product is {bond} USDC: pass --bond {bond}, or leave it out"
        ))?;
    }
    let owner = ctx.wallet_pubkey()?;
    let available = oracle
        .collateral(&owner)?
        .map_or(0, |l| l.available_balance);
    if available < listing_bond {
        bail(format!(
            "the listing bond is {} USDC and your oracle collateral has {} USDC available: deposit first (petri oracle collateral deposit {})",
            usdc(listing_bond),
            usdc(available),
            usdc(listing_bond - available)
        ))?;
    }
    let source_id = source_id
        .map(|id| super::skus::parse_hex32(id, "source id"))
        .transpose()?;
    let source_type = series.source_type();
    let mut write = writes::propose_write(
        &oracle,
        &month,
        &Proposal {
            bucket_id,
            url,
            source_type: &source_type,
            definition: &text,
            listing_bond,
            source_id,
        },
    )?;
    write.action = "oracle.series.propose";
    write.title = format!(
        "Propose the {} series {} for {} with a {} USDC listing bond",
        bucket.name,
        series.series_id.as_deref().unwrap_or_default(),
        month.period.expiry_id(),
        usdc(listing_bond)
    );
    write
        .review
        .insert(1, format!("Checked: {}", status_line(&routing)));
    write.review.insert(2, format!("Source type {source_type}"));
    write.details["routing"] = routing_json(&routing);
    send::run(ctx, p, write, target.send.simulate)
}

/// "Example Research · DDR5 16Gb (2Gx8) 4800/5600", or what the definition is.
fn series_words(parsed: &Parsed) -> String {
    match parsed {
        Parsed::Series(series) => [
            series.origin.publisher.as_deref(),
            series.origin.row.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · "),
        Parsed::Malformed(_) => "(not a valid series)".into(),
        Parsed::FreeText => "(free text)".into(),
        Parsed::Missing => "(definition not on chain)".into(),
    }
}

/// `petri oracle series show MONTH`.
pub fn show(ctx: &Ctx, p: Paint, market: &str, expiry: Option<&str>) -> Result<Report> {
    let oracle = Oracle::new(ctx)?;
    let month = oracle.month(market, expiry)?;
    let product = chip_product(&oracle, &month)?;
    let read = month_entries(&oracle, &month)?;
    let stage = stage(month.state.phase);
    let routings = route_entries(product, stage, &entries_of(&read));
    let mut table = Table::new(&["Source id", "Bucket", "Status", "Series", "Checked"]);
    let mut items = Vec::new();
    let mut notes = Vec::new();
    for ((source, parsed), routing) in read.iter().zip(&routings) {
        let id = hex(&source.state.source_id);
        table.row(vec![
            format!("{}…", &id[..16]),
            source.bucket_name(),
            format!("{:?}", source.state.status),
            crate::backend::terminal_safe_text(&series_words(parsed)),
            routing
                .as_ref()
                .map_or("-", |r| r.status.summary())
                .to_string(),
        ]);
        if let Some(routing) = routing.as_ref().filter(|r| !r.status.admissible()) {
            for reason in routing.deciding() {
                notes.push(format!(
                    "  {}…  {}: {}",
                    &id[..8],
                    reason.field,
                    crate::backend::terminal_safe_text(&reason.message)
                ));
            }
        }
        items.push(json!({
            "source": source.address.to_string(),
            "sourceId": id,
            "bucket": source.bucket_label(),
            "bucketName": source.bucket_name(),
            "status": format!("{:?}", source.state.status),
            "series": parsed.series(),
            "routing": routing.as_ref().map_or(Value::Null, routing_json),
        }));
    }
    let mut lines = vec![p.bold(&format!(
        "{} · {} chip buckets · series set {}",
        month_line(&month),
        product.symbol(),
        match stage {
            Stage::Open => "open (it commits at the freeze)",
            Stage::Committed => "committed",
        }
    ))];
    if table.is_empty() {
        lines.push("  No sources yet.".into());
    } else {
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
    }
    if !notes.is_empty() {
        lines.push(p.dim("Why some don't count:"));
        lines.extend(notes);
    }
    let payload = json!({
        "ok": true,
        "month": month_json(&month),
        "product": product.symbol(),
        "stage": match stage { Stage::Open => "open", Stage::Committed => "committed" },
        "sources": items,
    });
    Ok(Report::new(payload, lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(id: &str, row: &str) -> SeriesDefinition {
        let template = spec::series_template(Product::Ramx.bucket("DDR5_16GBIT").unwrap(), false);
        let mut series: SeriesDefinition = serde_json::from_value(template).unwrap();
        series.series_id = Some(id.into());
        series.origin.publisher = Some("Example Research".into());
        series.origin.locator = Some("https://example.com/price/dram".into());
        series.origin.row = Some(row.into());
        series
    }

    #[test]
    fn definitions_parse_as_series_free_text_or_neither() {
        let text = series("a", "DDR5 16Gb").definition_text();
        assert!(matches!(Parsed::of(Some(&text)), Parsed::Series(_)));
        assert_eq!(
            Parsed::of(Some("Product: DDR5 RDIMM 64GB")),
            Parsed::FreeText
        );
        assert_eq!(Parsed::of(None), Parsed::Missing);
        let broken = r#"{"schema":"amoeba.comparison-series.v1","colour":"red"}"#;
        assert!(matches!(Parsed::of(Some(broken)), Parsed::Malformed(_)));
        for error in [None, Some("bad")] {
            let routing = not_a_series(error);
            assert_eq!(routing.status, Status::NeedsMetadata);
            assert_eq!(routing.reasons[0].field, "definition");
        }
        assert_eq!(
            not_a_series(None).reasons[0].message,
            "not a comparison series"
        );
    }

    #[test]
    fn a_month_routes_each_source_and_a_repeat_keeps_no_weight() {
        let first = series("ex-ddr5-16", "DDR5 16Gb");
        let entries = vec![
            Entry {
                source_id: "bb".repeat(32),
                parsed: Parsed::Series(Box::new(first.clone())),
                committed: true,
            },
            Entry {
                source_id: "aa".repeat(32),
                parsed: Parsed::FreeText,
                committed: true,
            },
            Entry {
                source_id: "cc".repeat(32),
                parsed: Parsed::Series(Box::new(first.clone())),
                committed: false,
            },
            // A dev bootstrap source: its definition is not on chain.
            Entry {
                source_id: "dd".repeat(32),
                parsed: Parsed::Missing,
                committed: true,
            },
        ];
        let routings = route_entries(Product::Ramx, Stage::Committed, &entries);
        // Nothing to route without a definition: no routing at all.
        assert_eq!(routings[3], None);
        let routings: Vec<Routing> = routings.into_iter().take(3).map(Option::unwrap).collect();
        assert_eq!(routings[1].reasons[0].message, "not a comparison series");
        // The template leaves its measurement unset: missing details, yet the
        // later copy of the same series is a duplicate by source-id order.
        assert_eq!(routings[1].status, Status::NeedsMetadata);
        assert!(
            routings[2]
                .reasons
                .iter()
                .any(|r| r.status == Status::DuplicateNoExtraWeight)
        );
        assert!(
            !routings[0]
                .reasons
                .iter()
                .any(|r| r.status == Status::DuplicateNoExtraWeight)
        );
        // A new proposal of the same series repeats the one on chain.
        let routing = route_against(&first, Product::Ramx, Stage::Open, &entries, None);
        assert!(
            routing
                .reasons
                .iter()
                .any(|r| r.status == Status::DuplicateNoExtraWeight)
        );
        assert!(
            refuse(&routing, "this series")
                .message
                .starts_with("refusing: this series routes as")
        );
    }

    #[test]
    fn stages_and_recipes_follow_the_freeze() {
        assert_eq!(stage(OraclePhase::SourceSubmission), Stage::Open);
        assert_eq!(stage(OraclePhase::Scramble), Stage::Open);
        assert_eq!(stage(OraclePhase::Opening), Stage::Committed);
        assert_eq!(stage_named("Game"), Stage::Committed);
        assert_eq!(stage_named("Scramble"), Stage::Open);
        assert!(in_recipe(OracleSourceStatus::OpeningPending));
        assert!(!in_recipe(OracleSourceStatus::Candidate));
        assert!(in_recipe_named("Inactive") && !in_recipe_named("Merged"));
    }

    #[test]
    fn buckets_are_found_in_every_product() {
        assert_eq!(find_bucket("DDR5_16GBIT").unwrap().name, "DDR5 16Gb");
        assert_eq!(find_bucket("1tb tlc").unwrap().id, "tlc_1tb");
        let error = find_bucket("DDR5 RDIMM 64GB").unwrap_err().message;
        assert!(
            error.contains("RAMX DDR5_16GBIT") && error.contains("NANDX tlc_1tb"),
            "{error}"
        );
        // RAMX's research candidate: its template is written, and says so.
        let candidate = template("DDR5 32Gb", false, None).unwrap();
        assert_eq!(candidate.payload["bucket"]["researchCandidate"], true);
        assert_eq!(candidate.payload["template"]["bucket"], "DDR5_32GBIT");
        assert!(
            candidate.payload["note"]
                .as_str()
                .unwrap()
                .contains("routes as Research only")
        );
        assert!(candidate.plain.starts_with('{'), "stdout stays JSON");
        let mut series: SeriesDefinition =
            serde_json::from_value(candidate.payload["template"].clone()).unwrap();
        series.series_id = Some("ddr5-32gb".into());
        assert_eq!(product_of(&series).unwrap(), Product::Ramx);
        assert_eq!(
            spec::route_series(&series, Product::Ramx).status,
            Status::ResearchOnly
        );
        let report = template("tlc_512gb", false, None).unwrap();
        assert_eq!(report.payload["template"]["bucket"], "tlc_512gb");
        assert_eq!(report.payload["bucket"]["name"], "512Gb TLC");
        assert!(
            report
                .plain
                .contains("\"schema\": \"amoeba.comparison-series.v1\"")
        );
    }
}
