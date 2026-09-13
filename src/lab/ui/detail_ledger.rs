//! Market detail, contract activity, and wallet activity presentation.

use super::super::*;

const MAX_LEDGER_DISPLAY_CHARS: usize = 256;

fn ledger_display_text(raw: &str) -> String {
    crate::backend::terminal_safe_text(raw)
        .chars()
        .take(MAX_LEDGER_DISPLAY_CHARS)
        .collect()
}

pub(in super::super) fn draw_detail_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let outer = panel_block(
        cli,
        "market & settlement",
        Color::Cyan,
        app.focus == LabFocus::Detail,
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2); 2])
        .split(vertical[0]);
    for (index, view) in DetailView::ALL.iter().copied().enumerate() {
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                view.label(),
                if view == DetailView::Overview {
                    Color::Cyan
                } else {
                    Color::Magenta
                },
                view == app.trading.detail_view,
                tabs[index].width,
                tabs[index].height,
            )),
            tabs[index],
        );
    }
    let lines = match app.trading.detail_view {
        DetailView::Overview => detail_lines(app),
        DetailView::Settlement => settlement_detail_lines(cli, app),
    };
    frame.render_widget(
        scrolling_panel(
            lines,
            vertical[1],
            cli,
            app.focused_panel_scroll(LabFocus::Detail),
            app.focus == LabFocus::Detail,
        )
        .block(panel_block(
            cli,
            app.trading.detail_view.label(),
            if app.trading.detail_view == DetailView::Settlement {
                Color::Magenta
            } else {
                Color::Cyan
            },
            app.focus == LabFocus::Detail,
        )),
        vertical[1],
    );
}

pub(in super::super) fn detail_tab_hit_at(area: Rect, column: u16, row: u16) -> Option<DetailView> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2); 2])
        .split(vertical[0]);
    tabs.iter()
        .position(|area| rect_contains(*area, column, row))
        .and_then(|index| DetailView::ALL.get(index).copied())
}

pub(in super::super) fn settlement_detail_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    if app.trading.loading_settlement {
        return vec![
            Line::from(format!("{} Loading settlement evidence...", app.spinner())),
            Line::from(Span::styled(
                "Record, readiness, and oracle evidence are checked independently.",
                style(cli, Color::DarkGray),
            )),
        ];
    }
    if let Some(bundle) = app.trading.settlement_bundle.as_ref() {
        return settlement_data::settlement_bundle_lines(bundle)
            .into_iter()
            .map(|line| {
                let color = if line.contains("unavailable") || line.contains("Blockers:") {
                    Color::Yellow
                } else if line.starts_with("Settlement ") {
                    Color::Magenta
                } else {
                    Color::White
                };
                Line::from(Span::styled(line, style(cli, color)))
            })
            .collect();
    }
    vec![
        Line::from(
            app.trading
                .settlement_issue
                .as_deref()
                .map(ledger_display_text)
                .unwrap_or_else(|| {
                    "Settlement evidence has not been loaded for this month.".to_string()
                }),
        ),
        Line::from(Span::styled(
            "Press r to retry. Petri never fills missing settlement facts from catalog dates.",
            style(cli, Color::DarkGray),
        )),
    ]
}

pub(in super::super) fn detail_lines(app: &LabApp) -> Vec<Line<'static>> {
    let Some(detail) = &app.trading.detail else {
        return vec![
            Line::from(if app.trading.loading_detail {
                format!("{} Loading market details...", app.spinner())
            } else {
                "Market details are not available right now.".to_string()
            }),
            Line::from(ledger_display_text(&app.status)),
        ];
    };
    vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", detail.symbol),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(detail.title.clone()),
        ]),
        Line::from(format!(
            "{} | {}",
            market_status_label(&detail.status),
            market_price_context(detail)
        )),
        Line::from(format!(
            "{} | settles {} | {} days",
            detail.expiry_label, detail.settlement, detail.days
        )),
        Line::from(format!(
            "Fixed-risk contracts | no liquidation | cap width {} | selected contract shows max loss",
            detail.cap_width
        )),
        Line::from(format!(
            "{} contracts | listed notional {}",
            detail.rows, detail.listed_notional
        )),
        Line::from(detail.freshness.clone()),
    ]
}

pub(in super::super) fn activity_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let (Some(detail), Some(quote)) = (&app.trading.detail, app.selected_quote()) else {
        return vec![Line::from(if app.trading.loading_detail {
            format!("{} Loading selected contract...", app.spinner())
        } else {
            "Select a contract from Options to see recent trades.".to_string()
        })];
    };

    let volume = quote.volume.or(quote.open_interest).unwrap_or(0.0);
    let mut lines = vec![
        Line::from(vec![Span::styled(
            selected_contract_label(app),
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        )]),
        Line::from(format!(
            "Month {} | settles {} | {}",
            detail.expiry_label,
            short_settlement_label(&detail.settlement),
            time_to_settle_label(&detail.days)
        )),
        Line::from(format!(
            "bid {} | ask {} | mid {} | depth {}",
            format_optional_decimal(quote.bid, 3),
            format_optional_decimal(quote.ask, 3),
            format_optional_decimal(quote.mid, 3),
            format_optional_usd(quote.depth_usd)
        )),
        Line::from(format!(
            "reported volume/OI {} {}",
            format_decimal(volume, 1),
            bar_for(Some(volume), max_quote_volume(detail), 18)
        )),
        Line::from(""),
    ];
    if volume <= 0.0 {
        lines.push(Line::from("No recent trades for this contract yet."));
    } else {
        lines.push(Line::from("Recent contract volume is reflected above."));
    }
    lines
}

pub(in super::super) fn ledger_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let mut lines = ledger_header_lines(cli, app);

    if !app.wallet.is_attached() {
        lines.push(Line::from(""));
        lines.extend(ledger_recent_wallet_activity_lines(cli, app));
        return lines;
    }

    if app.loading_ledger {
        lines.push(Line::from(format!(
            "{} Loading account ledger...",
            app.spinner()
        )));
        return lines;
    }

    let Some(payload) = &app.ledger else {
        lines.push(Line::from(""));
        lines.push(Line::from("Wallet activity is temporarily unavailable."));
        lines.push(Line::from("Press r to refresh."));
        return lines;
    };

    let chain_history = value_at_key(payload, &["chainHistory"]).unwrap_or(payload);
    let product_ledger = ledger_product_ledger(payload);
    let summaries = product_ledger.and_then(|ledger| array_at_key(ledger, &["summaries"]));
    let events = product_ledger.and_then(|ledger| array_at_key(ledger, &["events"]));
    let has_product_rows = summaries.map(|items| !items.is_empty()).unwrap_or(false)
        || events.map(|items| !items.is_empty()).unwrap_or(false);
    let amoeba_matches = value_at_key(payload, &["amoebaActivity"])
        .and_then(|activity| array_at_key(activity, &["matches"]));
    let has_amoeba_matches = amoeba_matches
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let current_writer_sleeves = value_at_key(payload, &["currentWriterSleeves"])
        .and_then(|data| array_at_key(data, &["sleeves"]));
    let has_current_writer_sleeves = current_writer_sleeves.is_some_and(|items| !items.is_empty());

    lines.push(Line::from(""));
    if !solana_history::account_history_is_available(chain_history)
        && !has_product_rows
        && !has_amoeba_matches
        && !has_current_writer_sleeves
    {
        lines.push(Line::from("Wallet activity is temporarily unavailable."));
        lines.push(Line::from("Press r to refresh."));
        push_ledger_issues(cli, payload, &mut lines);
        return lines;
    }

    if solana_history::account_history_is_empty(chain_history)
        && !has_product_rows
        && !has_amoeba_matches
        && !has_current_writer_sleeves
    {
        for line in terminal_brand::empty_word_lines() {
            lines.push(Line::from(Span::styled(
                line,
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(
            "No wallet transaction history was found for this account.",
        ));
        push_ledger_issues(cli, payload, &mut lines);
        return lines;
    }

    lines.extend(current_writer_sleeve_lines(cli, payload));
    if has_current_writer_sleeves {
        lines.push(Line::from(""));
    }
    lines.push(Line::from("Amoeba wallet activity"));
    lines.extend(ledger_amoeba_activity_lines(cli, app));
    lines.push(Line::from(""));
    lines.push(Line::from("Recent wallet activity"));
    lines.extend(ledger_recent_wallet_activity_lines(cli, app));
    push_ledger_issues(cli, payload, &mut lines);
    lines
}

pub(in super::super) fn ledger_header_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let wallet = &app.wallet;
    let mut lines = vec![Line::from(vec![
        Span::styled("Account: ", style(cli, Color::DarkGray)),
        Span::styled(
            wallet.account_label().to_string(),
            style(
                cli,
                if wallet.is_attached() {
                    Color::Green
                } else {
                    Color::Yellow
                },
            )
            .add_modifier(Modifier::BOLD),
        ),
    ])];

    if wallet.issue.is_some() {
        lines.push(Line::from(Span::styled(
            "Local signing source unavailable; check your wallet or Solana CLI settings."
                .to_string(),
            style(cli, Color::Yellow),
        )));
    }

    if let Some(line) = wallet_balance_line(cli, app) {
        lines.push(line);
    }
    if let Some(line) = current_collateral_line(cli, app) {
        lines.push(line);
    }

    lines
}

fn current_collateral_line(cli: &Cli, app: &LabApp) -> Option<Line<'static>> {
    let collateral = app
        .ledger
        .as_ref()
        .and_then(|payload| value_at_key(payload, &["currentCollateral"]))
        .and_then(|data| value_at_key(data, &["collateral"]))?;
    let available = string_at_key(collateral, &["availableBalance"])
        .map(|value| ledger_display_text(&value))
        .unwrap_or_else(|| "-".to_string());
    let locked = string_at_key(collateral, &["lockedBalance"])
        .map(|value| ledger_display_text(&value))
        .unwrap_or_else(|| "-".to_string());
    let ready = collateral.get("canSubmitTrade") == Some(&Value::Bool(true));
    Some(Line::from(vec![
        Span::styled("Collateral: ", style(cli, Color::DarkGray)),
        Span::styled(
            format!("available {available} | locked {locked}"),
            style(cli, Color::White),
        ),
        divider_span(cli),
        Span::styled(
            if ready { "trade ready" } else { "waiting" }.to_string(),
            style(cli, if ready { Color::Green } else { Color::Yellow })
                .add_modifier(Modifier::BOLD),
        ),
    ]))
}

fn current_writer_sleeve_lines(cli: &Cli, payload: &Value) -> Vec<Line<'static>> {
    let Some(sleeves) = value_at_key(payload, &["currentWriterSleeves"])
        .and_then(|data| array_at_key(data, &["sleeves"]))
        .filter(|items| !items.is_empty())
    else {
        return Vec::new();
    };
    let mut lines = vec![Line::from(Span::styled(
        format!("Collective writer sleeves ({})", sleeves.len()),
        style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
    ))];
    for sleeve in sleeves.iter().take(12) {
        let address = string_at_key(sleeve, &["sleeve"])
            .or_else(|| string_at_key(sleeve, &["address"]))
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        let status = string_at_key(sleeve, &["status"])
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        let flat = string_at_key(sleeve, &["flatBalanceAtoms"])
            .or_else(|| string_at_key(sleeve, &["flatParAtoms"]))
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        lines.push(Line::from(format!(
            "{address} | {status} | Flat {flat} atoms"
        )));
    }
    if sleeves.len() > 12 {
        lines.push(Line::from(format!(
            "{} more collective sleeves; use `petri writers list`.",
            sleeves.len() - 12
        )));
    }
    lines
}

pub(in super::super) fn wallet_balance_line(cli: &Cli, app: &LabApp) -> Option<Line<'static>> {
    let balance = app
        .ledger
        .as_ref()
        .and_then(|payload| value_at_key(payload, &["walletBalance"]))?;
    let sol = value_at_key(balance, &["solAmount"])
        .and_then(Value::as_f64)
        .map(wallet_balance::format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    let usdc = value_at_key(balance, &["usdcAmount"])
        .and_then(Value::as_f64)
        .map(wallet_balance::format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    let amba = value_at_key(balance, &["ambaAmount"])
        .and_then(Value::as_f64)
        .map(wallet_balance::format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    Some(Line::from(vec![
        Span::styled("Balances: ", style(cli, Color::DarkGray)),
        Span::styled(format!("SOL {sol}"), style(cli, Color::White)),
        divider_span(cli),
        Span::styled(format!("USDC {usdc}"), style(cli, Color::Green)),
        divider_span(cli),
        Span::styled(
            format!("AMBA {amba}"),
            style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
        ),
    ]))
}

pub(in super::super) fn ledger_recent_wallet_activity_lines(
    _cli: &Cli,
    app: &LabApp,
) -> Vec<Line<'static>> {
    let wallet = &app.wallet;
    if !wallet.is_attached() {
        return vec![
            Line::from("Attach a wallet to view wallet activity and Amoeba trade history."),
            Line::from("Tip: launch with --keypair or use your Solana CLI keypair path."),
        ];
    }

    if app.loading_ledger {
        return vec![Line::from(format!(
            "{} Loading account ledger...",
            app.spinner()
        ))];
    }

    let Some(payload) = &app.ledger else {
        return vec![
            Line::from("Wallet activity is temporarily unavailable."),
            Line::from("Press r to refresh."),
        ];
    };

    let chain_history = value_at_key(payload, &["chainHistory"]).unwrap_or(payload);
    if !solana_history::account_history_is_available(chain_history) {
        return vec![Line::from("Wallet activity is temporarily unavailable.")];
    }
    let signatures = solana_history::account_history_signatures(chain_history);
    let mut lines = vec![Line::from(format!(
        "{} transactions found",
        signatures.len()
    ))];
    if signatures.is_empty() {
        lines.push(Line::from("No recent wallet transactions found."));
        return lines;
    }
    for (index, item) in signatures.iter().take(8).enumerate() {
        let signature = string_at_key(item, &["signature"])
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        let block_time = string_at_key(item, &["blockTime", "block_time"])
            .map(|value| solana_history::format_ledger_timestamp(&value))
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        let status = string_at_key(item, &["confirmationStatus", "confirmation_status"])
            .map(|value| ledger_display_text(&value))
            .unwrap_or_else(|| "-".to_string());
        let err = value_at_key(item, &["err"])
            .filter(|value| !value.is_null())
            .map(|value| value.to_string())
            .filter(|value| value != "ok")
            .map(|value| ledger_display_text(&value))
            .map(|value| format!(" | issue {}", short_path(&value, 24)))
            .unwrap_or_default();
        let label = if index == 0 { "Latest" } else { "Recent" };
        lines.push(Line::from(format!(
            "{label}: {block_time} | {status} | {}{err}",
            short_path(&signature, 28)
        )));
    }

    lines
}

pub(in super::super) fn ledger_amoeba_activity_lines(
    _cli: &Cli,
    app: &LabApp,
) -> Vec<Line<'static>> {
    let wallet = &app.wallet;
    if !wallet.is_attached() {
        return vec![Line::from(
            "Attach a wallet to scan for Amoeba-related transactions.",
        )];
    }

    if app.loading_ledger {
        return vec![Line::from(format!(
            "{} Loading account ledger...",
            app.spinner()
        ))];
    }

    let Some(payload) = &app.ledger else {
        return vec![Line::from("Amoeba activity is temporarily unavailable.")];
    };

    let activity = value_at_key(payload, &["amoebaActivity"]);
    let matches = activity.and_then(|value| array_at_key(value, &["matches"]));
    let match_count = matches.map(|items| items.len()).unwrap_or(0);
    let scanned = activity
        .and_then(|value| string_at_key(value, &["scannedTransactionCount"]))
        .map(|value| ledger_display_text(&value))
        .unwrap_or_else(|| "0".to_string());
    let program_count = activity
        .and_then(|value| string_at_key(value, &["programCount"]))
        .map(|value| ledger_display_text(&value))
        .unwrap_or_else(|| "1".to_string());
    let mut lines = Vec::new();
    if match_count == 0 {
        lines.push(Line::from("No matching recent wallet transactions found."));
        lines.push(Line::from(format!(
            "Checked {scanned} recent transactions against {program_count} Amoeba programs."
        )));
    } else {
        lines.push(Line::from(format!(
            "{} matching transaction{} found",
            match_count,
            if match_count == 1 { "" } else { "s" }
        )));
        if let Some(items) = matches {
            for (index, item) in items.iter().take(5).enumerate() {
                let signature = string_at_key(item, &["signature"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let block_time = string_at_key(item, &["blockTime", "block_time"])
                    .map(|value| solana_history::format_ledger_timestamp(&value))
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let status = string_at_key(item, &["confirmationStatus", "confirmation_status"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let label = if index == 0 { "Latest" } else { "Recent" };
                lines.push(Line::from(format!(
                    "{label}: {block_time} | {status} | {} | {}",
                    short_path(&signature, 28),
                    matched_program_labels(item)
                )));
            }
        }
    }

    if let Some(ledger) = ledger_product_ledger(payload) {
        let summaries = array_at_key(ledger, &["summaries"]);
        let events = array_at_key(ledger, &["events"]);
        let summary_count = summaries.map(|items| items.len()).unwrap_or(0);
        let event_count = events.map(|items| items.len()).unwrap_or(0);
        if summary_count > 0 || event_count > 0 {
            let generated_at = string_at_key(ledger, &["generatedAt", "generated_at"])
                .map(|value| solana_history::format_ledger_timestamp(&value))
                .map(|value| ledger_display_text(&value))
                .unwrap_or_else(|| "-".to_string());
            lines.push(Line::from(""));
            lines.push(Line::from("Indexed Amoeba history"));
            lines.push(Line::from(format!("Last updated {generated_at}")));
            lines.push(Line::from(format!(
                "Recent markets {summary_count} | recent events {event_count}"
            )));
        }

        if let Some(totals) = value_at_key(ledger, &["totals"]) {
            if !totals.is_null() {
                lines.push(Line::from(
                    "Totals are available in diagnostics only because token units could not be converted safely.",
                ));
            }
        }

        if let Some(items) = summaries.filter(|items| !items.is_empty()) {
            lines.push(Line::from("Recent markets:"));
            for summary in items.iter().take(5) {
                let market = string_at_key(summary, &["marketId", "market_id"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let expiry = string_at_key(summary, &["expiryId", "expiry_id"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let event_count = string_at_key(summary, &["eventCount", "event_count"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "0".to_string());
                lines.push(Line::from(format!(
                    "{market}/{expiry} | {event_count} events"
                )));
            }
        }

        if let Some(items) = events.filter(|items| !items.is_empty()) {
            lines.push(Line::from("Recent events:"));
            for event in items.iter().take(6) {
                let occurred = string_at_key(
                    event,
                    &["occurredAt", "occurred_at", "blockTime", "block_time"],
                )
                .map(|value| solana_history::format_ledger_timestamp(&value))
                .map(|value| ledger_display_text(&value))
                .unwrap_or_else(|| "-".to_string());
                let event_type = string_at_key(event, &["eventType", "event_type"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let market = string_at_key(event, &["marketId", "market_id"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                let expiry = string_at_key(event, &["expiryId", "expiry_id"])
                    .map(|value| ledger_display_text(&value))
                    .unwrap_or_else(|| "-".to_string());
                lines.push(Line::from(format!(
                    "{occurred} | {event_type} | {market}/{expiry}"
                )));
            }
        }
    }

    lines
}

pub(in super::super) fn ledger_product_ledger(payload: &Value) -> Option<&Value> {
    let product_payload =
        value_at_key(payload, &["productLedger"]).filter(|value| !value.is_null());
    product_payload
        .map(|payload| value_at_key(payload, &["data"]).unwrap_or(payload))
        .map(|root| value_at_key(root, &["ledger"]).unwrap_or(root))
}

pub(in super::super) fn matched_program_labels(item: &Value) -> String {
    array_at_key(item, &["matchedPrograms", "matched_programs"])
        .map(|programs| {
            programs
                .iter()
                .filter_map(|program| string_at_key(program, &["label", "name", "programId"]))
                .map(|label| ledger_display_text(&label))
                .collect::<Vec<_>>()
        })
        .filter(|labels| !labels.is_empty())
        .map(|labels| ledger_display_text(&labels.join(", ")))
        .unwrap_or_else(|| "Amoeba".to_string())
}

pub(in super::super) fn current_detail_status(
    market_id: &str,
    detail: Option<&DishDetail>,
) -> String {
    let symbol = market_id.to_uppercase();
    match detail {
        Some(detail) if !detail.issues.is_empty() && detail.status != "Active" => {
            format!("{symbol} market metadata updated; contracts unavailable")
        }
        Some(_) => format!("{symbol} market updated"),
        None => format!("{symbol} market updated"),
    }
}

pub(in super::super) fn push_ledger_issues(
    cli: &Cli,
    payload: &Value,
    lines: &mut Vec<Line<'static>>,
) {
    let Some(issues) = array_at_key(payload, &["issues"]).filter(|items| !items.is_empty()) else {
        return;
    };

    lines.push(Line::from(""));
    for issue in issues {
        let label = ledger_display_text(&solana_history::user_facing_ledger_issue(
            issue.as_str().unwrap_or("ledger issue"),
        ));
        lines.push(Line::from(Span::styled(
            format!("note: {label}"),
            style(cli, Color::Yellow),
        )));
    }
}
