//! What the agent does with a research run: the exact commands, in order, by
//! what the month takes now (sources while it forms, opening values in
//! Opening, updates in Game, reward claims once its rewards are frozen).
//! `petri research work next` prints them and `petri_work_next` returns them.

use super::super::workspace::usdc;
use super::Flags;
use serde_json::{Value, json};

/// A value as a JavaScript template literal shows it (the workspace's
/// replies are JSON).
fn js(value: &Value) -> String {
    match value {
        Value::Null => "undefined".into(),
        Value::String(s) => s.clone(),
        Value::Object(_) => "[object Object]".into(),
        other => other.to_string(),
    }
}

/// `value ?? fallback`: the value unless it is missing or null.
fn or(value: &Value, fallback: &str) -> String {
    if value.is_null() {
        fallback.into()
    } else {
        js(value)
    }
}

/// The text of `value` unless it is missing, null or empty (what `||` and a
/// truthy test pass over).
fn present(value: &Value) -> Option<String> {
    Some(js(value)).filter(|s| !value.is_null() && !s.is_empty() && value != &json!(false))
}

fn quote(value: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

/// This command as the agent's shell runs it again (PowerShell on Windows):
/// `own` is how it runs this petri.
pub fn self_command(own: &str, words: &str, flags: &Flags) -> String {
    let agent = flags
        .get("agent")
        .or_else(|| flags.get("name"))
        .unwrap_or("researcher");
    let mut line = format!("{own} research {words} --agent {}", quote(agent));
    if let Some(directory) = flags.get("profile-dir") {
        line.push_str(&format!(" --profile-dir {}", quote(directory)));
    }
    line
}

// A page behind a sign-in (a subscription, a distributor portal): the owner
// signs in to a browser the agent drives (browser use), and the agent proves
// the JSON the page loads with that session (`petri evidence`,
// evidence/README.md). Credentials stay in the browser: petri never sees them.
const MANIFEST: &str = r#"{"format":"amoeba-extraction-manifest-v1","name":"<source>: <product>","evidenceType":"catalog_price","origin":"<host>","path":"<path, no query>","content":"json","records":"/items","match":{"/sku":"<product id>"},"fields":{"price":"/unit_price","currency":"/currency","product":"/sku"}}"#;

fn signed_in(petri: &str, page: &str, json: &str, find: String, submit: String) -> Vec<String> {
    vec![
        format!(
            "- Open it with browser use: {petri} evidence browser {page}. It prints a DevTools address (http://127.0.0.1:<port>); your own browser tool can attach to that window (chrome-devtools-mcp --browserUrl, Playwright connectOverCDP)."
        ),
        "- Ask the owner to sign in there themselves. Never ask for, type or store their password."
            .into(),
        find,
        format!(
            "- Save that response to a file and write an extraction manifest for it, e.g. {MANIFEST}"
        ),
        format!(
            "- Test it: {petri} evidence check-manifest <manifest file> --body <saved response>"
        ),
        format!(
            "- Prove it in the signed-in browser: {petri} evidence prove {json} --manifest <manifest file> --browser --out <bundle file> --json"
        ),
        "- Show the owner the preview it prints (what is disclosed) and go on only if they agree."
            .into(),
        submit,
    ]
}

// An opening or update of a frozen source proves the source's own URL: the
// on-chain evidence record names the source's registered locator.
fn source_proof(petri: &str, url: &str, command: String) -> Vec<String> {
    let mut lines = vec![
        "If the page needs a sign-in, it has no public archive: prove it instead of step 2."
            .to_string(),
    ];
    lines.extend(signed_in(
        petri,
        url,
        url,
        format!("- The proof is of the source's own URL, {url}: its whole JSON response is disclosed. Read the same row and price field the source was found with."),
        command,
    ));
    lines
}

const FIND_JSON: &str = "- Find the JSON the page loads with the price (the browser's network panel). Its whole response is disclosed: pick one with nothing in it the owner would not share.";

fn with(result: &Value, directive: Option<&Value>, next_step: String) -> Value {
    let mut object = result.as_object().cloned().unwrap_or_default();
    if let Some(directive) = directive {
        object.insert("directive".into(), directive.clone());
    }
    object.insert("nextStep".into(), json!(next_step));
    Value::Object(object)
}

/// The workspace's reply to `work next`, with `nextStep`: what to run now.
pub fn assignment_steps(result: Value, flags: &Flags, own: &str) -> Value {
    if result["status"] != "assigned" {
        let next = self_command(own, "work next", flags);
        return with(
            &result,
            None,
            format!("Nothing needs research right now. In a few minutes, run: {next}"),
        );
    }
    let run = js(&result["run"]["id"]);
    let assignment = &result["assignment"];
    let objective = &assignment["objective"];
    let kind = objective["kind"].as_str().unwrap_or_default();
    let directive = present(&assignment["guidance"]["instruction"]);
    let directive_value = json!(directive);
    // How the agent runs petri here (the workspace says, else this petri).
    let petri = present(&result["petri"]).unwrap_or_else(|| own.to_string());
    let petri = petri.as_str();
    let month = present(&objective["expiryId"])
        .unwrap_or_else(|| js(&assignment["candidateScope"]["expiryId"]));
    let title = js(&assignment["title"]);
    let owner = match &directive {
        Some(directive) => format!("The owner's directive for it comes first: {directive}"),
        None => "The owner has given no directive for it.".into(),
    };
    // The owner's live feed in the TUI: one short line per step, as it happens.
    let say = self_command(own, &format!("work report {run}"), flags);
    let wrote = match kind {
        "opening" => "opening value submitted",
        "update" => "update committed",
        _ => "sponsored 30d7e0f9\u{2026}",
    };
    let mut report = vec![
        "Report as you go, one short line each:".to_string(),
        format!(r#"- each search: {say} --kind search --message "<query>""#),
        format!(
            r#"- each page you open: {say} --kind read --source <page URL> --message "<page title>" (a signed-in page too)"#
        ),
        format!(
            r#"- each value you read: {say} --kind value --source <page URL> --message "<value> <unit>, <product>""#
        ),
        format!(
            r#"- each petri write: {say} --kind submit --message "<what it did, e.g. {wrote}>""#
        ),
    ];
    if kind != "opening" && kind != "update" {
        report.push(format!(r#"- each price page worth submitting: {say} --kind source_found --source <page URL> --message "<product, price>""#));
    }
    report.push(format!(
        r#"- anything else: {say} --message "<text>" (--kind blocked if stuck)"#
    ));
    let report = report.join("\n");
    let finish = format!(
        r#"When you are done with it, run: {} --summary "<what you did>""#,
        self_command(own, &format!("work finish {run}"), flags)
    );
    let next = format!(
        "Then run {} again for the next one. Keep going until it says nothing needs research.",
        self_command(own, "work next", flags)
    );
    let page = or(&objective["url"], r#"see "assignment""#);
    let bucket_label = or(&objective["bucketLabel"], "see the brief");
    // A chip month (RAMX and NANDX expiring after 2026-11-01): its sources are
    // comparison series and its prints observations of them, sent through
    // petri's series and observe commands (they check the commodity spec first).
    if objective["chip"] == true && ["opening", "update", "source"].contains(&kind) {
        let steps = chip_steps(
            &result,
            flags,
            own,
            Chip {
                report,
                finish,
                next,
                owner,
                month,
                petri,
            },
        );
        return with(&result, Some(&directive_value), steps);
    }
    let lines: Vec<String> = match kind {
        "opening" => {
            // The oracle's opening rules: the stake is exactly the product's
            // opening bond; the archive is of the source's own URL, captured
            // from the Opening's start up to the chain's clock; the claim is
            // accepted 300 slots after it lands, and only before listing.
            let source = if objective["sourceAddress"].is_null() {
                or(&objective["sourceId"], "<source address>")
            } else {
                js(&objective["sourceAddress"])
            };
            let url = or(&objective["url"], "<page URL>");
            let bond = usdc(&objective["openingBondAtomic"]);
            let start = present(&objective["openingStartUtc"]);
            let until = present(&objective["listingUtc"])
                .map(|l| format!(" ({l})"))
                .unwrap_or_default();
            let opening_bond = bond.clone().unwrap_or_else(|| "<opening bond>".into());
            let since = start
                .as_ref()
                .map(|start| {
                    let digits: String = start
                        .chars()
                        .filter(char::is_ascii_digit)
                        .take(14)
                        .collect();
                    format!(" ({start}, {digits})")
                })
                .unwrap_or_default();
            let exactly = if bond.is_some() {
                "the stake must be exactly this product's opening bond"
            } else {
                "the stake must be exactly the product's opening bond: petri names it if you give another"
            };
            let mut lines = vec![
                format!(
                    "Give this frozen source its opening value now: {title} ({month}). Its page: {page}."
                ),
                owner,
                format!(
                    "The month is in Opening until listing{until}, on the chain's clock: each frozen source needs an opening value, the price its page shows now, where the source starts in Game. Anyone submits it, bonded from their petri wallet with the product's opening bond; a source with none by listing sits out the month."
                ),
                format!(
                    "1. Open the page and read the value it publishes for this exact product ({bucket_label}), in the page's own units."
                ),
                format!(
                    "2. Archive the page: open https://web.archive.org/save/{url} and note the 14-digit UTC capture time it gives. The archive URL is https://web.archive.org/web/<capture time>/{url}, with the page URL exactly as written here. The capture time must be at or after the Opening's start{since} and not after the chain's clock ({petri} status shows it as Clock). If the chain's clock is behind the capture, use an earlier capture of the page from that range (its Wayback calendar lists them) or wait until the chain's clock passes it."
                ),
            ];
            lines.extend(source_proof(petri, &url, format!("- In step 3, give --evidence <bundle file> in place of --archive-url. If {petri} oracle prints opening --help does not list --evidence, report blocked.")));
            lines.extend([
                format!("3. Submit it with the owner's wallet: {petri} oracle prints opening {month} --source {source} --value <value> --archive-url <archive URL> --stake {opening_bond} --yes ({exactly}). Submit it some minutes before listing: it can only be accepted 300 slots after it lands. The bond comes from the owner's oracle collateral; if petri says it is short, deposit the bond first: {petri} oracle collateral deposit {opening_bond} --yes"),
                format!("4. Anyone can challenge it for 300 slots (a minute or two). Then accept it: {petri} oracle prints finalize {month} --source {source} --yes. If petri says it can't be accepted yet, wait a minute and run it again. It must be accepted before listing{until}. If it was challenged, the oracle authority rules on it by hand on the opening day (or leaves it to the council): report that and move on."),
                format!("5. Return the bond to the owner's collateral: {petri} oracle stakes settle {month} --kind opening-claim --source {source} --yes"),
                format!("At listing the month enters Game, once every frozen source has its opening value or is set aside (anyone sets an unopened one aside: {petri} oracle prints expire {month} --source <source address> --yes) and the oracle has weighted its sources. Then anyone can start Game: {petri} oracle prints finalize-phase {month} --yes."),
                report, finish, next,
            ]);
            lines
        }
        "claims" => vec![
            format!(
                "Claim the owner's rewards of {month}. They go to the owner's oracle collateral."
            ),
            owner,
            format!("1. List them: {petri} oracle rewards {month}"),
            format!("2. If any are to claim: {petri} oracle rewards claim {month} --yes"),
            format!(
                "3. For each sponsored source to claim: {petri} oracle sponsorships claim <source> --yes"
            ),
            report,
            finish,
            next,
        ],
        "update" => {
            let url = or(&objective["url"], "<page URL>");
            let source = or(&objective["sourceAddress"], "<source address>");
            let mut lines = vec![
                format!("Update this live source now: {title} ({month}). Its page: {page}."),
                owner,
                "The month is in Game: its sources are set, and each update is a staked new value from the owner's petri wallet.".into(),
                format!("1. Open the page and read the value it publishes for this exact product ({bucket_label})."),
                format!("2. Archive the page: open https://web.archive.org/save/{url} and keep the snapshot URL it gives (https://web.archive.org/web/<14-digit time>/...)."),
            ];
            lines.extend(source_proof(petri, &url, "- In step 3, give --evidence <bundle file> in place of --archive-url <snapshot URL>.".into()));
            lines.extend([
                format!("3. Commit and reveal the update in one step: {petri} oracle updates commit {month} --source {source} --value <value> --archive-url <snapshot URL> --reveal --yes. Petri takes the product's exact update stake and refuses an update the oracle would refuse, saying why."),
                format!("4. If petri says the reveal failed, reveal it before the deadline it names: {petri} oracle updates reveal <CLAIM> --yes. How it stands: {petri} oracle updates {month}"),
                "Skip the update if the value has not changed: petri refuses an unchanged value.".into(),
                report, finish, next,
            ]);
            lines
        }
        _ => {
            let submit = self_command(own, &format!("work submit {run}"), flags);
            let mut lines = vec![
                format!(r#"Research this now: {title}. Its brief and exact scope (candidateScope) are in "assignment". The month takes new sources now."#),
                owner,
                report,
                format!("Submit each page you can verify: write its candidate JSON (marketId, expiryId, bucketId from candidateScope; canonicalUrl, sourceType, productDefinition, priceField, region, currency, evidence) to a file, then run: {submit} --file <file>"),
                "A page behind a sign-in (a subscription, a distributor portal) counts too, with a web proof:".into(),
            ];
            lines.extend(signed_in(
                petri,
                "<page URL>",
                "<JSON URL>",
                FIND_JSON.into(),
                format!("- Submit it with the bundle: {submit} --file <file> --evidence-bundle <bundle file>, with the JSON URL as canonicalUrl. Only a catalog_price or published_benchmark can be a source; a quotation or transaction_record cannot."),
            ));
            lines.extend([sponsoring(petri, &month), finish, next]);
            lines
        }
    };
    with(&result, Some(&directive_value), lines.join("\n"))
}

fn sponsoring(petri: &str, month: &str) -> String {
    format!(
        "A submitted find goes on the oracle once a sponsor posts its bond. If the owner asks you to sponsor finds with their wallet: {petri} oracle sponsor <contentHash> --yes. A product counts as covered once a source of it has a supporter: {petri} oracle sources support {month} --source <source address> --stake 1 --yes. When every product is covered, the owner decides when to close the source window (Scramble's challenge days follow): {petri} oracle sources close {month} --yes."
    )
}

// The fields a comparison series must define (the commodity spec's
// comparison_series_manifest): petri oracle series check names what is missing.
const SERIES_FIELDS: &str = "seriesId and definitionVersion; bucket; evidenceClass (published_assessment, or direct_price for one exact part) and priceBasis (contract); origin.publisher, origin.locator (the page) and origin.row (the exact row or series label on it); physicalScope; nativeUnit; statistic (fixed before any observation); currency; procurementSegment and geography; referencePeriod and cadence; revisionPolicy and missingDataPolicy; the template's product scope block (for RAMX dram: memoryFamily, generation, dieDensity, dieCount, condition, grade, organization, speed; for NANDX nand: architecture, cellType, dieDensity, dieCount, condition, grade, interface; another product's as its template names them); a direct price also direct.manufacturer, mpn, quantityTier, deliveryTerms, pricingPeriod, taxFreight and paymentTerms";

struct Chip<'a> {
    report: String,
    finish: String,
    next: String,
    owner: String,
    month: String,
    petri: &'a str,
}

fn chip_steps(result: &Value, flags: &Flags, own: &str, chip: Chip) -> String {
    let Chip {
        report,
        finish,
        next,
        owner,
        month,
        petri,
    } = chip;
    let run = js(&result["run"]["id"]);
    let assignment = &result["assignment"];
    let objective = &assignment["objective"];
    let title = js(&assignment["title"]);
    let product = if objective["bucketName"].is_null() {
        or(&objective["bucketLabel"], "see the brief")
    } else {
        js(&objective["bucketName"])
    };
    let bucket = if objective["bucketLabel"].is_null() {
        or(&objective["bucketId"], "<bucket>")
    } else {
        js(&objective["bucketLabel"])
    };
    let source = if objective["sourceAddress"].is_null() {
        or(&objective["sourceId"], "<source address>")
    } else {
        js(&objective["sourceAddress"])
    };
    let url = or(&objective["url"], "<page URL>");
    let page = or(&objective["url"], r#"see "assignment""#);
    if objective["kind"] == "source" {
        let submit = self_command(own, &format!("work submit {run}"), flags);
        let mut lines = vec![
            format!(r#"Research this now: {title}. Its brief and exact scope (candidateScope) are in "assignment". The month takes new sources now."#),
            owner,
            format!("This month is on commodity chip buckets: each source is one comparison series of {product}, a repeatable contract price (a published contract assessment, or a direct contract price of one exact part) defined field by field. Never guess a field: leave what the source doesn't say null, and the check says what is missing."),
            report,
            format!("1. Get the series template: {petri} oracle series template --bucket {bucket} --out series.json (add --direct for a direct price of one exact part)."),
            format!("2. Fill it from the source itself. The spec requires: {SERIES_FIELDS}."),
            format!("3. Check it: {petri} oracle series check series.json {month}. Fix what it names until it says Fits the bucket. Wrong product, Not a contract price or Research only means the page doesn't belong in this bucket: move on."),
            format!("4. Submit it: write its candidate JSON (marketId, expiryId, bucketId from candidateScope; evidence: what the page shows; series: the checked series object) to a file, then run: {submit} --file <file>. The page, source type and definition come from the series."),
            "A page behind a sign-in (a subscription assessment, a distributor portal) counts too, with a web proof:".into(),
        ];
        lines.extend(signed_in(
            petri,
            "<page URL>",
            "<JSON URL>",
            FIND_JSON.into(),
            format!("- Put the JSON URL in the series' origin.locator, check it again, and submit it with the bundle: {submit} --file <file> --evidence-bundle <bundle file>."),
        ));
        lines.extend([sponsoring(petri, &month), finish, next]);
        return lines.join("\n");
    }
    let opening = objective["kind"] == "opening";
    let bond = usdc(&objective["openingBondAtomic"]);
    let start = present(&objective["openingStartUtc"]);
    let until = present(&objective["listingUtc"])
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();
    let series = or(&objective["seriesId"], "(see its definition)");
    let mut lines = vec![
        if opening {
            format!("Give this frozen source its opening value now: {title} ({month}). Its page: {page}.")
        } else {
            format!("Update this live source now: {title} ({month}). Its page: {page}.")
        },
        owner,
        format!(
            "This source is comparison series {series} of {product}; its {} is one observation of it: the price, its period and its evidence.",
            if opening { "opening value" } else { "update" }
        ),
        if opening {
            format!("The month is in Opening until listing{until}, on the chain's clock: each frozen source needs an opening value, bonded from the owner's petri wallet with the product's opening bond; a source with none by listing sits out the month.")
        } else {
            "The month is in Game: each update is a staked new observation from the owner's petri wallet.".into()
        },
        format!("1. Get its observation template: {petri} oracle observe template {month} --source {source} --out obs.json"),
        "2. Open the page and read this series' exact row in the page's own units as rawPrice. Fill in the reference period, publishedAt, retrievedAt, publicationVintage and releaseId from the page; leave what it doesn't say null.".into(),
        format!(
            "3. Archive the page: open https://web.archive.org/save/{url} and put the archive URL it gives (https://web.archive.org/web/<14-digit capture time>/{url}, the page URL exactly as written here) in evidenceReference.{}",
            if opening {
                format!(
                    " The capture time must be at or after the Opening's start{} and not after the chain's clock ({petri} status shows it as Clock).",
                    start.as_ref().map(|s| format!(" ({s})")).unwrap_or_default()
                )
            } else {
                String::new()
            }
        ),
        "If the page needs a sign-in, it has no public archive: prove it instead of step 3.".into(),
    ];
    lines.extend(signed_in(
        petri,
        &url,
        &url,
        format!("- The proof is of the source's own URL, {url}: its whole JSON response is disclosed. Read this series' row and its price as rawPrice."),
        "- Put the bundle file in evidenceReference in place of the archive URL: the print cites the proof instead (its price must be rawPrice).".into(),
    ));
    let stake = if opening {
        format!(
            "opening bond{}",
            bond.as_ref()
                .map(|b| format!(" ({b} USDC)"))
                .unwrap_or_default()
        )
    } else {
        "update stake".into()
    };
    let amount = bond.clone().unwrap_or_else(|| "<amount>".into());
    lines.push(format!("4. Send it with the owner's wallet: {petri} oracle observe {month} --source {source} --file obs.json --reveal --yes. Petri checks it against the spec first, takes the product's exact {stake}, and refuses one that would not count, saying why. If petri says the collateral is short: {petri} oracle collateral deposit {amount} --yes"));
    if opening {
        lines.extend([
            format!("5. Anyone can challenge it for 300 slots (a minute or two). Then accept it: {petri} oracle prints finalize {month} --source {source} --yes, before listing{until}. If it was challenged, the oracle authority rules on it by hand on the opening day (or leaves it to the council): report that and move on."),
            format!("6. Return the bond to the owner's collateral: {petri} oracle stakes settle {month} --kind opening-claim --source {source} --yes"),
        ]);
    } else {
        lines.extend([
            format!("5. --reveal reveals it as soon as the commit lands, and its observation is recorded then. If petri says the reveal failed, reveal it before the deadline it names: {petri} oracle updates reveal <CLAIM> --yes"),
            "Skip it if the price has not changed: petri refuses an unchanged value.".into(),
        ]);
    }
    lines.extend([report, finish, next]);
    lines.join("\n")
}
