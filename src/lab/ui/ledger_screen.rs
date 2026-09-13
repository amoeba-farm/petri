//! Integrated wallet, position, writer, and history workspace.

use super::super::*;

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct LedgerScreenLayout {
    pub(in super::super) tabs: Rect,
    pub(in super::super) list: Rect,
    pub(in super::super) detail: Rect,
    pub(in super::super) actions: Option<Rect>,
}

pub(in super::super) fn ledger_screen_layout(area: Rect, view: LedgerView) -> LedgerScreenLayout {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);
    let body = vertical[1];
    match view {
        LedgerView::Account => {
            let columns = Layout::default()
                .direction(if body.width >= 82 {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                })
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(body);
            LedgerScreenLayout {
                tabs: vertical[0],
                list: columns[0],
                detail: columns[1],
                actions: Some(columns[0]),
            }
        }
        LedgerView::Positions => {
            if body.width >= 82 {
                let columns = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .split(body);
                let left = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
                    .split(columns[0]);
                LedgerScreenLayout {
                    tabs: vertical[0],
                    list: left[0],
                    detail: columns[1],
                    actions: Some(left[1]),
                }
            } else {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(28),
                        Constraint::Length(5),
                        Constraint::Min(0),
                    ])
                    .split(body);
                LedgerScreenLayout {
                    tabs: vertical[0],
                    list: rows[0],
                    detail: rows[2],
                    actions: Some(rows[1]),
                }
            }
        }
        LedgerView::History => {
            let columns = Layout::default()
                .direction(if body.width >= 82 {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                })
                .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
                .split(body);
            LedgerScreenLayout {
                tabs: vertical[0],
                list: columns[0],
                detail: columns[1],
                actions: None,
            }
        }
        LedgerView::Writers => {
            if body.width >= 78 {
                let columns = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(28),
                        Constraint::Percentage(44),
                        Constraint::Percentage(28),
                    ])
                    .split(body);
                LedgerScreenLayout {
                    tabs: vertical[0],
                    list: columns[0],
                    detail: columns[1],
                    actions: Some(columns[2]),
                }
            } else {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(30),
                        Constraint::Percentage(42),
                        Constraint::Percentage(28),
                    ])
                    .split(body);
                LedgerScreenLayout {
                    tabs: vertical[0],
                    list: rows[0],
                    detail: rows[1],
                    actions: Some(rows[2]),
                }
            }
        }
    }
}

pub(in super::super) fn draw_ledger_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::Ledger;
    let outer = panel_block(cli, "wallet & positions", Color::Cyan, focused);
    frame.render_widget(outer, area);
    let layout = ledger_screen_layout(area, app.ledger_view);
    draw_ledger_tabs(frame, cli, layout.tabs, app);
    match app.ledger_view {
        LedgerView::Account => draw_account_workspace(frame, cli, layout, app),
        LedgerView::Positions => draw_positions_workspace(frame, cli, layout, app),
        LedgerView::Writers => draw_writers_workspace(frame, cli, layout, app),
        LedgerView::History => draw_history_workspace(frame, cli, layout, app),
    }
}

fn draw_ledger_tabs(frame: &mut Frame<'_>, cli: &Cli, area: Rect, app: &LabApp) {
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 4); 4])
        .split(area);
    for (index, view) in LedgerView::ALL.iter().copied().enumerate() {
        let selected = view == app.ledger_view;
        let color = match view {
            LedgerView::Account => Color::Cyan,
            LedgerView::Positions => Color::Blue,
            LedgerView::Writers => Color::Magenta,
            LedgerView::History => Color::Green,
        };
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                view.label(),
                color,
                selected,
                tabs[index].width,
                tabs[index].height,
            )),
            tabs[index],
        );
    }
}

fn draw_account_workspace(
    frame: &mut Frame<'_>,
    cli: &Cli,
    layout: LedgerScreenLayout,
    app: &LabApp,
) {
    let mut summary = ledger_header_lines(cli, app);
    summary.push(Line::from(""));
    summary.push(Line::from(Span::styled(
        "Account actions",
        style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    for (index, action) in AccountAction::ALL.iter().copied().enumerate() {
        let selected = index == app.ledger_account_action_selected;
        summary.push(selectable_line(
            cli,
            action.label(),
            selected,
            selected && app.ledger_pane == LedgerPane::Actions,
            Color::Cyan,
        ));
    }
    summary.push(Line::from(""));
    summary.push(Line::from(Span::styled(
        "Wallet and connection changes are human-only. The Guide cannot activate them.",
        style(cli, Color::DarkGray),
    )));
    frame.render_widget(
        scrolling_panel(
            summary,
            layout.list,
            cli,
            app.focused_panel_scroll(LabFocus::Ledger),
            app.ledger_pane == LedgerPane::Actions,
        )
        .block(panel_block(
            cli,
            "Account",
            Color::Cyan,
            app.ledger_pane == LedgerPane::Actions,
        )),
        layout.list,
    );

    let mut details = vec![
        detail_line(cli, "Network", &app.onchain_config.network),
        detail_line(
            cli,
            "Market service",
            &safe_connection_label(&app.onchain_config.backend_url),
        ),
        detail_line(
            cli,
            "Amoeba chain reads",
            &crate::petri_config::rpc_gateway_url(&app.onchain_config.backend_url)
                .map(|url| safe_connection_label(&url))
                .unwrap_or_else(|_| "Amoeba gateway unavailable".to_string()),
        ),
        detail_line(
            cli,
            "Confirmation level",
            &app.onchain_config
                .commitment
                .clone()
                .unwrap_or_else(|| "Solana config default".to_string()),
        ),
        detail_line(cli, "Signing source", app.wallet.path_source.label()),
        detail_line(cli, "Signing-source safety", &app.wallet.keypair_file),
        Line::from(Span::styled(
            "The keypair path is hidden. Private RPC credentials never enter Petri.",
            style(cli, Color::DarkGray),
        )),
        Line::from(""),
    ];
    if app.wallet_switch_editing {
        details.extend([
            Line::from(Span::styled(
                "Switch wallet",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            wallet_switch_input_line(cli, app),
            Line::from(Span::styled(
                "Enter attaches this signing source; Esc cancels.",
                style(cli, Color::DarkGray),
            )),
        ]);
    } else {
        details.extend(ledger_amoeba_activity_lines(cli, app).into_iter().take(8));
    }
    frame.render_widget(
        scrolling_panel(
            details,
            layout.detail,
            cli,
            app.focused_panel_scroll(LabFocus::Ledger),
            app.ledger_pane == LedgerPane::Detail,
        )
        .block(panel_block(
            cli,
            "Account detail & activity",
            Color::Blue,
            app.ledger_pane == LedgerPane::Detail,
        )),
        layout.detail,
    );
}

fn draw_positions_workspace(
    frame: &mut Frame<'_>,
    cli: &Cli,
    layout: LedgerScreenLayout,
    app: &LabApp,
) {
    let rows = app.liquidity_position_rows();
    let mut list = vec![Line::from(Span::styled(
        "Manager-owned positions",
        style(cli, Color::Blue).add_modifier(Modifier::BOLD),
    ))];
    if app.loading_ledger && rows.is_empty() {
        list.push(Line::from(format!(
            "{} Loading positions...",
            app.spinner()
        )));
    } else if rows.is_empty() {
        list.push(Line::from(
            "No current manager-liquidity positions were returned.",
        ));
    } else {
        for (index, row) in rows.iter().enumerate() {
            let market = display_field(row, &["marketId"]);
            let series = display_field(row, &["seriesId"]);
            list.push(selectable_line(
                cli,
                &format!("{market} / {series}"),
                index == app.ledger_position_selected,
                app.ledger_pane == LedgerPane::List,
                Color::Blue,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(list)
            .block(panel_block(
                cli,
                "Liquidity positions",
                Color::Blue,
                app.ledger_pane == LedgerPane::List,
            ))
            .wrap(Wrap { trim: true }),
        layout.list,
    );

    if let Some(actions_area) = layout.actions {
        let loading = app.liquidity_preview_is_running();
        let mut actions = vec![selectable_line(
            cli,
            if loading {
                "Loading unsigned preview..."
            } else {
                "Unsigned liquidity preview"
            },
            true,
            app.ledger_pane == LedgerPane::Actions,
            Color::Green,
        )];
        actions.push(Line::from(Span::styled(
            "Lean preview only · no prepare/sign/submit",
            style(cli, Color::DarkGray),
        )));
        frame.render_widget(
            Paragraph::new(actions)
                .block(panel_block(
                    cli,
                    "Preview",
                    Color::Yellow,
                    app.ledger_pane == LedgerPane::Actions,
                ))
                .wrap(Wrap { trim: true }),
            actions_area,
        );
    }

    let detail = liquidity_detail_lines(cli, app, rows);
    frame.render_widget(
        scrolling_panel(
            detail,
            layout.detail,
            cli,
            app.focused_panel_scroll(LabFocus::Ledger),
            app.ledger_pane == LedgerPane::Detail,
        )
        .block(panel_block(
            cli,
            if app.liquidity_preview_form.is_some() {
                "Unsigned preview form"
            } else if app.liquidity_preview_result.is_some() {
                "Unsigned preview result"
            } else {
                "Position detail"
            },
            Color::Cyan,
            app.ledger_pane == LedgerPane::Detail || app.liquidity_preview_form.is_some(),
        )),
        layout.detail,
    );
}

pub(in super::super) fn liquidity_detail_lines(
    cli: &Cli,
    app: &LabApp,
    rows: &[Value],
) -> Vec<Line<'static>> {
    if let Some(form) = app.liquidity_preview_form.as_ref() {
        let mut lines = vec![
            Line::from(Span::styled(
                "UNSIGNED LEAN PREVIEW",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "This is not independent SDK validation and never prepares transaction bytes, signs, or submits.",
                style(cli, Color::DarkGray),
            )),
            Line::from(""),
        ];
        for (index, field) in liquidity::LiquidityPreviewField::ALL
            .iter()
            .copied()
            .enumerate()
        {
            let selected = index == form.selected_field;
            let raw = form.field_value(field);
            let display = if raw.is_empty() {
                if field == liquidity::LiquidityPreviewField::Entries {
                    "<BIN:A:B:C; repeat with spaces/commas>".to_string()
                } else {
                    "<required>".to_string()
                }
            } else {
                truncate_text(&crate::backend::terminal_safe_text(&raw), 240)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    style(
                        cli,
                        if selected {
                            Color::Yellow
                        } else {
                            Color::DarkGray
                        },
                    ),
                ),
                Span::styled(format!("{}: ", field.label()), style(cli, Color::DarkGray)),
                Span::styled(
                    display,
                    style(cli, if selected { Color::White } else { Color::Gray }).add_modifier(
                        if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        },
                    ),
                ),
            ]));
        }
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Action: Left/Right or A/R/C · fields: Tab/arrows · Enter requests preview · Esc cancels",
                style(cli, Color::DarkGray),
            )),
            Line::from(Span::styled(
                "The position nonce must come from an authoritative source; the current position list does not publish it.",
                style(cli, Color::Yellow),
            )),
        ]);
        return lines;
    }

    if app.liquidity_preview_is_running() {
        return vec![
            Line::from(format!(
                "{} Preparing SDK-validated liquidity review...",
                app.spinner()
            )),
            Line::from(Span::styled(
                "Nothing is being signed or submitted.",
                style(cli, Color::DarkGray),
            )),
        ];
    }

    if let Some(result) = app.liquidity_preview_result.as_ref() {
        let mut lines = vec![
            Line::from(Span::styled(
                if result.ok {
                    "PREVIEW READY"
                } else {
                    "PREVIEW STOPPED"
                },
                style(cli, if result.ok { Color::Green } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(crate::backend::terminal_safe_text(&result.message)),
        ];
        if let Some(payload) = result.payload.as_ref() {
            let request = payload.get("review").unwrap_or(payload);
            lines.extend([
                detail_line(
                    cli,
                    "Market / expiry",
                    &format!(
                        "{} / {}",
                        display_field(request, &["marketId"]),
                        display_field(request, &["expiryId"])
                    ),
                ),
                detail_line(cli, "Owner", &display_field(request, &["ownerPubkey"])),
                detail_line(cli, "Action", &display_field(request, &["action"])),
                detail_line(
                    cli,
                    "Position nonce",
                    &display_field(request, &["positionNonce"]),
                ),
                detail_line(
                    cli,
                    "Entries",
                    &request
                        .get("entries")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default()
                        .to_string(),
                ),
            ]);
        }
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "F9 opens the exact SDK-validated review. Signing requires your explicit approval.",
                style(cli, Color::Yellow),
            )),
        ]);
        return lines;
    }

    rows.get(app.ledger_position_selected)
        .map(|row| {
            vec![
                detail_line(
                    cli,
                    "Market / series",
                    &format!(
                        "{} / {}",
                        display_field(row, &["marketId"]),
                        display_field(row, &["seriesId"])
                    ),
                ),
                detail_line(
                    cli,
                    "Contract",
                    &format!(
                        "{} {}-{}",
                        display_field(row, &["optionKind"]),
                        display_field(row, &["lowerStrike"]),
                        display_field(row, &["upperStrike"])
                    ),
                ),
                detail_line(cli, "Settles", &display_field(row, &["settlementUtc"])),
                detail_line(
                    cli,
                    "Liquidity shares",
                    &display_field(row, &["liquidityShares"]),
                ),
                detail_line(cli, "Fee shares", &display_field(row, &["feeShares"])),
                detail_line(
                    cli,
                    "State",
                    &format!(
                        "{} / {}",
                        display_field(row, &["status"]),
                        display_field(row, &["sourceState"])
                    ),
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "Prepare add, remove, or close from the Preview pane; F9 reviews and approves the exact action.",
                    style(cli, Color::Yellow),
                )),
                Line::from(Span::styled(
                    "The current list omits the required position nonce. Petri accepts it only as an explicit preview input and never infers it.",
                    style(cli, Color::DarkGray),
                )),
            ]
        })
        .unwrap_or_else(|| {
            vec![
                Line::from("Select a manager-liquidity position to inspect it."),
                Line::from(""),
                Line::from(Span::styled(
                    "This is not an option-token or Flat portfolio.",
                    style(cli, Color::DarkGray),
                )),
            ]
        })
}

fn draw_writers_workspace(
    frame: &mut Frame<'_>,
    cli: &Cli,
    layout: LedgerScreenLayout,
    app: &LabApp,
) {
    let rows = app.writer_sleeve_rows();
    let mut list = vec![Line::from(Span::styled(
        "Global sleeve catalog",
        style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
    ))];
    list.push(Line::from(Span::styled(
        "Rows are protocol sleeves, not wallet ownership claims.",
        style(cli, Color::DarkGray),
    )));
    if app.loading_ledger && rows.is_empty() {
        list.push(Line::from(format!("{} Loading sleeves...", app.spinner())));
    } else if rows.is_empty() {
        list.push(Line::from(
            "No current collective-writer sleeves were returned.",
        ));
    } else {
        for (index, row) in rows.iter().enumerate() {
            let address = display_field(row, &["address", "sleeve", "sleeveAddress"]);
            let status = display_field(row, &["status", "phase"]);
            list.push(selectable_line(
                cli,
                &format!("{} · {status}", short_pubkey(&address)),
                index == app.ledger_writer_selected,
                app.ledger_pane == LedgerPane::List,
                Color::Magenta,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(list)
            .block(panel_block(
                cli,
                "Collective writers",
                Color::Magenta,
                app.ledger_pane == LedgerPane::List,
            ))
            .wrap(Wrap { trim: true }),
        layout.list,
    );

    frame.render_widget(
        scrolling_panel(
            writer_detail_lines(cli, app),
            layout.detail,
            cli,
            app.focused_panel_scroll(LabFocus::Ledger),
            app.ledger_pane == LedgerPane::Detail,
        )
        .block(panel_block(
            cli,
            if app.writers.form.is_some() {
                "Writer form"
            } else {
                "Sleeve detail"
            },
            Color::Cyan,
            app.ledger_pane == LedgerPane::Detail || app.writers.form.is_some(),
        )),
        layout.detail,
    );

    if let Some(actions_area) = layout.actions {
        let mut actions = Vec::new();
        for (index, action) in WriterAction::ALL.iter().copied().enumerate() {
            let selected = index == app.writers.action_selected;
            actions.push(selectable_line(
                cli,
                action.label(),
                selected,
                app.ledger_pane == LedgerPane::Actions,
                match app.writer_action_availability(action) {
                    WriterActionAvailability::Disabled(_) => Color::DarkGray,
                    WriterActionAvailability::HotOnly(_) => Color::Yellow,
                    WriterActionAvailability::Enabled if action.signs_and_submits() => {
                        Color::Yellow
                    }
                    WriterActionAvailability::Enabled => Color::Green,
                },
            ));
        }
        actions.push(Line::from(""));
        actions.push(Line::from(Span::styled(
            "Signed actions always open review with Cancel selected.",
            style(cli, Color::DarkGray),
        )));
        frame.render_widget(
            // Scrolling and hit testing use one row per action. Wrapping labels
            // breaks that correspondence and can hide the selected action.
            Paragraph::new(actions)
                .scroll((writer_action_scroll_offset(actions_area, app) as u16, 0))
                .block(panel_block(
                    cli,
                    "Actions",
                    Color::Yellow,
                    app.ledger_pane == LedgerPane::Actions,
                )),
            actions_area,
        );
    }
}

pub(in super::super) fn writer_detail_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let mut capability_lines = writer_capability_banner_lines(cli, app);
    if let Some(form) = app.writers.form.as_ref() {
        capability_lines.push(Line::from(Span::styled(
            form.action.label(),
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        let mut lines = capability_lines;
        for (index, field) in form.fields.iter().enumerate() {
            let selected = index == form.selected_field;
            let display = if field.value.is_empty() {
                "<required>".to_string()
            } else if field.secret {
                "•".repeat(field.value.chars().count().min(16))
            } else {
                crate::backend::terminal_safe_text(&field.value)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if selected { "> " } else { "  " },
                    style(
                        cli,
                        if selected {
                            Color::Yellow
                        } else {
                            Color::DarkGray
                        },
                    ),
                ),
                Span::styled(format!("{}: ", field.label), style(cli, Color::DarkGray)),
                Span::styled(
                    display,
                    style(cli, if selected { Color::White } else { Color::Gray }).add_modifier(
                        if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        },
                    ),
                ),
            ]));
        }
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                if form.action.signs_and_submits() {
                    "Enter opens final review · Tab/arrows change field · Esc cancels"
                } else {
                    "Enter runs this read-only request · Tab/arrows change field · Esc cancels"
                },
                style(cli, Color::DarkGray),
            )),
        ]);
        return lines;
    }

    if let Some(result) = app.writers.action_result.as_ref() {
        capability_lines.extend([
            Line::from(Span::styled(
                if result.ok { "READY" } else { "STOPPED" },
                style(cli, if result.ok { Color::Green } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(result.message.clone()),
        ]);
        let mut lines = capability_lines;
        if let Some(payload) = result.payload.as_ref() {
            let rendered = match result.action {
                WriterAction::Show => crate::writer_output::render_writer_sleeve(payload),
                WriterAction::Policy => crate::writer_output::render_writer_policy_audit(payload),
                WriterAction::Liquidity | WriterAction::Refunds => {
                    crate::writer_liquidity::render_inspection(payload)
                        .unwrap_or_else(|issue| format!("Writer read unavailable: {issue}"))
                }
                WriterAction::ClosePreview => {
                    crate::writer_output::render_writer_close_preview(payload)
                }
                WriterAction::CloseStatus => {
                    crate::writer_output::render_writer_close_status(payload)
                }
                _ => writer_receipt_summary(payload),
            };
            lines.extend(
                rendered
                    .lines()
                    .take(
                        if matches!(
                            result.action,
                            WriterAction::Liquidity | WriterAction::Refunds
                        ) {
                            96
                        } else {
                            24
                        },
                    )
                    .map(|line| Line::from(crate::backend::terminal_safe_text(line))),
            );
        }
        return lines;
    }

    let Some(row) = app.selected_writer_sleeve() else {
        capability_lines.extend([
            Line::from("Select a collective-writer sleeve."),
            Line::from(Span::styled(
                "Petri never treats the global catalog as proof that your wallet owns a sleeve.",
                style(cli, Color::DarkGray),
            )),
        ]);
        return capability_lines;
    };
    let active_close_request = writers::writer_close_request_address(row)
        .map(|value| crate::backend::terminal_safe_text(&value))
        .unwrap_or_else(|| "-".to_string());
    capability_lines.extend([
        detail_line(
            cli,
            "Sleeve",
            &display_field(row, &["address", "sleeve", "sleeveAddress"]),
        ),
        detail_line(cli, "Status", &display_field(row, &["status", "phase"])),
        detail_line(cli, "Active close request", &active_close_request),
        detail_line(
            cli,
            "Writer principal",
            &display_field(
                row,
                &["writerPrincipalAtoms", "principalAtoms", "principal"],
            ),
        ),
        detail_line(
            cli,
            "Settlement assets",
            &display_field(row, &["settlementAssetsAtoms", "assetsAtoms", "assets"]),
        ),
        detail_line(
            cli,
            "Exact reserve",
            &display_field(row, &["reserveAtoms", "reserve"]),
        ),
        detail_line(
            cli,
            "Free headroom",
            &display_field(row, &["freeHeadroomAtoms", "headroomAtoms", "headroom"]),
        ),
        detail_line(
            cli,
            "Locked premiums",
            &display_field(row, &["lockedPremiumAtoms", "premiumLedgerAtoms"]),
        ),
        detail_line(
            cli,
            "Flat supply",
            &display_field(
                row,
                &["flatSupplyAtoms", "flatParAtoms", "flatBalanceAtoms"],
            ),
        ),
        detail_line(
            cli,
            "Policy",
            &display_field(row, &["policyVersion", "policy", "policyHash"]),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Flat capital is committed to this dated sleeve. Ordinary redemption is unavailable before settlement; close-to-redeem requires the proportional option basket and can be punitive.",
            style(cli, Color::Yellow),
        )),
    ]);
    capability_lines
}

fn writer_capability_banner_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let color = match app.writer_close_capability_state() {
        WriterCloseCapabilityState::Checking => Color::DarkGray,
        WriterCloseCapabilityState::Unavailable => Color::Red,
        WriterCloseCapabilityState::Ready(projection)
            if projection.implementation_supported
                && projection.runtime_enabled
                && projection.hot_runtime_enabled
                && projection.cold_runtime_enabled =>
        {
            Color::Green
        }
        WriterCloseCapabilityState::Ready(projection)
            if projection.implementation_supported
                && projection.runtime_enabled
                && projection.hot_runtime_enabled =>
        {
            Color::Yellow
        }
        WriterCloseCapabilityState::Ready(_) => Color::Red,
    };
    vec![
        Line::from(Span::styled(
            app.writer_close_capability_cue(),
            style(cli, color).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ]
}

fn draw_history_workspace(
    frame: &mut Frame<'_>,
    cli: &Cli,
    layout: LedgerScreenLayout,
    app: &LabApp,
) {
    let rows = app.ledger_history_rows();
    let mut list = vec![Line::from(Span::styled(
        "Combined authoritative activity",
        style(cli, Color::Green).add_modifier(Modifier::BOLD),
    ))];
    if rows.is_empty() {
        list.push(Line::from(
            "No indexed or chain activity is currently available.",
        ));
    } else {
        for (index, row) in rows.iter().take(50).enumerate() {
            let kind = display_field(row, &["type", "kind", "action", "eventType"]);
            let when = display_field(row, &["timestamp", "time", "blockTime", "createdAt"]);
            list.push(selectable_line(
                cli,
                &format!(
                    "{kind} · {}",
                    solana_history::format_ledger_timestamp(&when)
                ),
                index == app.ledger_history_selected,
                app.ledger_pane == LedgerPane::List,
                Color::Green,
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(list)
            .block(panel_block(
                cli,
                "History",
                Color::Green,
                app.ledger_pane == LedgerPane::List,
            ))
            .wrap(Wrap { trim: true }),
        layout.list,
    );
    let detail = rows
        .get(app.ledger_history_selected)
        .map(|row| history_detail_lines(cli, row))
        .unwrap_or_else(|| {
            vec![
                Line::from("Select an activity row to inspect it."),
                Line::from(""),
                Line::from(Span::styled(
                    "Petri exposes only the combined authoritative history view because typed categories are not yet proven by the current service.",
                    style(cli, Color::DarkGray),
                )),
            ]
        });
    frame.render_widget(
        scrolling_panel(
            detail,
            layout.detail,
            cli,
            app.focused_panel_scroll(LabFocus::Ledger),
            app.ledger_pane == LedgerPane::Detail,
        )
        .block(panel_block(
            cli,
            "Activity detail",
            Color::Cyan,
            app.ledger_pane == LedgerPane::Detail,
        )),
        layout.detail,
    );
}

pub(in super::super) fn history_detail_lines(cli: &Cli, row: &Value) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (label, keys) in [
        ("Type", &["type", "kind", "action", "eventType"][..]),
        ("Status", &["status", "state"][..]),
        ("Market", &["marketId", "market", "symbol"][..]),
        ("Series", &["expiryId", "seriesId", "contractId"][..]),
        ("Amount", &["amount", "amountAtoms", "quantity"][..]),
        (
            "Value",
            &["value", "valueAtoms", "premium", "withdrawal"][..],
        ),
        ("Time", &["timestamp", "time", "blockTime", "createdAt"][..]),
        (
            "Signature",
            &["signature", "transactionSignature", "txid"][..],
        ),
    ] {
        let value = display_field(row, keys);
        if value != "-" {
            lines.push(detail_line(cli, label, &value));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(
            "This row has no supported public detail fields.",
        ));
    }
    lines
}

pub(in super::super) fn ledger_tab_hit_at(area: Rect, column: u16, row: u16) -> Option<LedgerView> {
    let tabs_area = ledger_screen_layout(area, LedgerView::Account).tabs;
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 4); 4])
        .split(tabs_area);
    tabs.iter()
        .position(|rect| rect_contains(*rect, column, row))
        .and_then(|index| LedgerView::ALL.get(index).copied())
}

pub(in super::super) fn ledger_list_row_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let panel = ledger_screen_layout(area, app.ledger_view).list;
    let inner = panel_inner_rect(panel)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let header_rows = match app.ledger_view {
        LedgerView::Writers => 2,
        LedgerView::Positions | LedgerView::History => 1,
        LedgerView::Account => ledger_header_lines(cli, app).len().saturating_add(2),
    }
    .min(u16::MAX as usize) as u16;
    let index = row.saturating_sub(inner.y).saturating_sub(header_rows) as usize;
    let len = match app.ledger_view {
        LedgerView::Account => AccountAction::ALL.len(),
        LedgerView::Positions => app.liquidity_position_rows().len(),
        LedgerView::Writers => app.writer_sleeve_rows().len(),
        LedgerView::History => app.ledger_history_rows().len().min(50),
    };
    (index < len).then_some(index)
}

pub(in super::super) fn ledger_writer_action_hit_at(
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.ledger_view != LedgerView::Writers {
        return None;
    }
    let panel = ledger_screen_layout(area, app.ledger_view).actions?;
    let inner = panel_inner_rect(panel)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let index = row.saturating_sub(inner.y) as usize + writer_action_scroll_offset(panel, app);
    (index < WriterAction::ALL.len()).then_some(index)
}

pub(in super::super) fn ledger_liquidity_action_hit_at(
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> bool {
    if app.ledger_view != LedgerView::Positions || app.liquidity_preview_is_running() {
        return false;
    }
    ledger_screen_layout(area, app.ledger_view)
        .actions
        .and_then(panel_inner_rect)
        .is_some_and(|inner| {
            rect_contains(
                Rect {
                    height: inner.height.min(1),
                    ..inner
                },
                column,
                row,
            )
        })
}

pub(in super::super) fn writer_action_scroll_offset(area: Rect, app: &LabApp) -> usize {
    let visible = panel_inner_height(area).max(1);
    app.writers
        .action_selected
        .saturating_sub(visible.saturating_sub(1))
}

pub(in super::super) fn writer_confirmation_modal_rect(root: Rect) -> Rect {
    let width = root.width.saturating_sub(4).min(72);
    let height = root.height.saturating_sub(4).min(20);
    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: root.y + root.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(in super::super) fn writer_confirmation_button_rects(root: Rect) -> Option<(Rect, Rect)> {
    let modal = writer_confirmation_modal_rect(root);
    let inner = panel_inner_rect(modal)?;
    if inner.width < 24 || inner.height < 5 {
        return None;
    }
    let gap = if inner.width >= 50 { 3 } else { 1 };
    let available = inner.width.saturating_sub(2).saturating_sub(gap);
    let cancel_width = available / 2;
    let confirm_width = available.saturating_sub(cancel_width);
    let y = inner.y + inner.height.saturating_sub(3);
    Some((
        Rect {
            x: inner.x + 1,
            y,
            width: cancel_width,
            height: 3,
        },
        Rect {
            x: inner.x + 1 + cancel_width + gap,
            y,
            width: confirm_width,
            height: 3,
        },
    ))
}

pub(in super::super) fn writer_confirmation_button_at(
    root: Rect,
    column: u16,
    row: u16,
) -> Option<UserActionConfirmationChoice> {
    let (cancel, confirm) = writer_confirmation_button_rects(root)?;
    if rect_contains(cancel, column, row) {
        Some(UserActionConfirmationChoice::Cancel)
    } else if rect_contains(confirm, column, row) {
        Some(UserActionConfirmationChoice::Confirm)
    } else {
        None
    }
}

pub(in super::super) fn draw_writer_confirmation_modal(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    let Some(confirmation) = app.writers.confirmation.as_ref() else {
        return;
    };
    dim_tui_for_modal(frame, cli, root);
    let modal = writer_confirmation_modal_rect(root);
    frame.render_widget(Clear, modal);
    let block = panel_block(
        cli,
        &format!(
            "confirm {}",
            confirmation.action.label().to_ascii_lowercase()
        ),
        Color::Yellow,
        true,
    )
    .style(tui_alt_panel_style(cli));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    let mut lines = vec![Line::from(Span::styled(
        confirmation.action.label(),
        style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
    ))];
    lines.extend(
        confirmation
            .summary
            .iter()
            .map(|line| Line::from(crate::backend::terminal_safe_text(line))),
    );
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Petri re-fetches current state, validates the pinned SDK plan, signs locally, submits through the typed Amoeba route, and checks status.",
            style(cli, Color::White),
        )),
        Line::from(Span::styled(
            "The Guide and external MCP cannot choose Confirm.",
            style(cli, Color::DarkGray),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .style(tui_alt_panel_style(cli))
            .wrap(Wrap { trim: true }),
        Rect {
            x: inner.x.saturating_add(1),
            y: inner.y,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(4),
        },
    );
    if let Some((cancel, confirm)) = writer_confirmation_button_rects(root) {
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                "CANCEL",
                Color::Blue,
                confirmation.choice == UserActionConfirmationChoice::Cancel,
                cancel.width,
                cancel.height,
            )),
            cancel,
        );
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                "CONFIRM & SEND",
                Color::Yellow,
                confirmation.choice == UserActionConfirmationChoice::Confirm,
                confirm.width,
                confirm.height,
            )),
            confirm,
        );
    }
}

fn selectable_line(
    cli: &Cli,
    label: &str,
    selected: bool,
    pane_focused: bool,
    color: Color,
) -> Line<'static> {
    let active = selected && pane_focused;
    Line::from(vec![
        Span::styled(
            if selected { "> " } else { "  " },
            style(
                cli,
                if selected {
                    Color::Yellow
                } else {
                    Color::DarkGray
                },
            ),
        ),
        Span::styled(
            crate::backend::terminal_safe_text(label),
            style(cli, if active { Color::White } else { color }).add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
    ])
}

fn detail_line(cli: &Cli, label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), style(cli, Color::DarkGray)),
        Span::styled(
            crate::backend::terminal_safe_text(value),
            style(cli, Color::White),
        ),
    ])
}

fn display_field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        })
        .map(|text| crate::backend::terminal_safe_text(&text))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "-".to_string())
}

fn writer_receipt_summary(payload: &Value) -> String {
    let signature = value_at_key(payload, &["receipt"])
        .and_then(|receipt| string_at_key(receipt, &["signature"]))
        .or_else(|| string_at_key(payload, &["signature"]))
        .unwrap_or_else(|| "confirmed".to_string());
    format!("Writer action confirmed\nSignature: {signature}")
}

fn safe_connection_label(raw: &str) -> String {
    reqwest::Url::parse(raw)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            let port = url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            Some(format!("{}://{host}{port}", url.scheme()))
        })
        .unwrap_or_else(|| "configured endpoint".to_string())
}
