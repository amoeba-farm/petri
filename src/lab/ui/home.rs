//! Home summary, actions, previews, and selected market context.

use super::super::*;

pub(in super::super) fn draw_home_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let layout_focus = if app.focus == LabFocus::Guide {
        app.guide
            .context_focus
            .filter(|focus| *focus != LabFocus::Guide)
            .unwrap_or(LabFocus::HomeActions)
    } else {
        app.focus
    };
    let Some(layout) = home_panel_rects_for_focus(cli, area, app, layout_focus) else {
        return;
    };

    let summary_focused = app.focus == LabFocus::HomeSummary;
    let summary_lines = home_summary_lines(cli, app);
    let summary_scroll = app.focused_panel_scroll(LabFocus::HomeSummary);
    let summary_lines = if summary_focused {
        scroll_lines_to_panel(summary_lines, layout.summary, cli, summary_scroll, true)
    } else {
        clip_lines_to_panel(summary_lines, layout.summary, cli, summary_scroll)
    };
    let summary = Paragraph::new(summary_lines)
        .block(panel_block(
            cli,
            "selected market",
            Color::Cyan,
            summary_focused,
        ))
        .wrap(Wrap { trim: true });
    frame.render_widget(summary, layout.summary);

    let actions_focused = app.focus == LabFocus::HomeActions;
    let action_lines = home_action_lines(cli, app, layout.compact, actions_focused);
    let actions = Paragraph::new(home_action_lines_for_panel(
        cli,
        app,
        action_lines,
        layout.actions,
        layout.compact,
        actions_focused,
    ))
    .block(panel_block(cli, "home", Color::Yellow, actions_focused));
    frame.render_widget(actions, layout.actions);

    let Some(preview_area) = layout.preview else {
        return;
    };
    let selected_action = app.selected_home_action();
    let preview_focused = app.focus == LabFocus::HomePreview;
    let preview_lines = home_preview_lines(cli, app, selected_action);
    let preview_scroll = app.focused_panel_scroll(LabFocus::HomePreview);
    let preview_lines = if preview_focused {
        scroll_lines_to_panel(preview_lines, preview_area, cli, preview_scroll, true)
    } else {
        clip_lines_to_panel(preview_lines, preview_area, cli, preview_scroll)
    };
    let preview = Paragraph::new(preview_lines)
        .block(panel_block(
            cli,
            home_preview_title(selected_action),
            home_action_accent_color(selected_action),
            preview_focused,
        ))
        .wrap(Wrap { trim: true });
    frame.render_widget(preview, preview_area);
}

pub(in super::super) fn home_summary_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let Some(detail) = &app.trading.detail else {
        return vec![
            Line::from(Span::styled(
                if app.trading.loading_detail {
                    format!("{} Loading selected market...", app.spinner())
                } else {
                    "Select a market from the left.".to_string()
                },
                status_style(cli, &app.status),
            )),
            Line::from(Span::styled(
                app.status.clone(),
                status_style(cli, &app.status),
            )),
        ];
    };

    double_spaced_lines(vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", detail.symbol),
                style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(detail.title.clone(), style(cli, Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                market_price_context(detail),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | month ", style(cli, Color::DarkGray)),
            Span::styled(detail.expiry_label.clone(), style(cli, Color::White)),
            Span::styled(" | settles ", style(cli, Color::DarkGray)),
            Span::styled(
                short_settlement_label(&detail.settlement),
                style(cli, Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("Cap width ", style(cli, Color::DarkGray)),
            Span::styled(
                detail.cap_width.clone(),
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            no_liquidation_span(cli),
            divider_span(cli),
            Span::styled(
                short_freshness_label(&detail.freshness),
                status_style(cli, &detail.freshness),
            ),
        ]),
    ])
}

pub(in super::super) fn home_action_lines(
    cli: &Cli,
    app: &LabApp,
    compact: bool,
    focused: bool,
) -> Vec<Line<'static>> {
    let band_width = HomeAction::ALL
        .iter()
        .copied()
        .map(|action| 29 + home_action_detail(app, action).chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for (index, action) in HomeAction::ALL.iter().copied().enumerate() {
        let selected = index == app.home_selected;
        let active = selected && focused;
        let marker = if selected { ">" } else { " " };
        let detail = home_action_detail(app, action);
        let content_width = 29 + detail.chars().count();
        let band_style = if cli.no_color {
            Style::default()
        } else {
            Style::default().bg(home_action_color(action))
        };
        let blank =
            Line::from(Span::styled("\u{00a0}".repeat(band_width), band_style)).style(band_style);
        lines.push(blank.clone());
        lines.push(
            Line::from(vec![
                Span::styled(marker, style(cli, Color::Yellow)),
                Span::raw(" "),
                Span::styled(
                    format!(" {:<22} ", action.label()),
                    home_action_button_style(cli, action, active, selected),
                ),
                Span::raw(" "),
                Span::styled(detail, style(cli, Color::White)),
                Span::raw(" ".repeat(band_width.saturating_sub(content_width))),
            ])
            .style(band_style),
        );
        lines.push(blank);
    }
    if !compact {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "Enter",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" opens the selected lane. ", style(cli, Color::DarkGray)),
            Span::styled(
                "arrows",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" move. ", style(cli, Color::DarkGray)),
            Span::styled(
                "tab",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" changes panels.", style(cli, Color::DarkGray)),
        ]));
    }
    lines
}

pub(in super::super) fn home_action_lines_for_panel(
    cli: &Cli,
    app: &LabApp,
    lines: Vec<Line<'static>>,
    area: Rect,
    compact: bool,
    focused: bool,
) -> Vec<Line<'static>> {
    let inner_height = panel_inner_height(area);
    let visible_indices = home_action_visible_line_indices(app, lines.len(), inner_height, compact);
    if visible_indices.len() == lines.len() {
        return lines;
    }
    if inner_height == 0 {
        return Vec::new();
    }

    let hidden = lines.len().saturating_sub(visible_indices.len());
    let direction = hidden_line_direction(&visible_indices, lines.len());
    let mut visible = visible_indices
        .into_iter()
        .filter_map(|index| lines.get(index).cloned())
        .collect::<Vec<_>>();
    visible.push(hidden_lines_notice(cli, direction, hidden, focused));
    visible
}

pub(in super::super) fn hidden_line_direction(
    visible_indices: &[usize],
    line_count: usize,
) -> &'static str {
    if visible_indices.is_empty() {
        return "↓";
    }
    let contiguous = visible_indices
        .windows(2)
        .all(|pair| pair[1] == pair[0].saturating_add(1));
    if !contiguous {
        return "↕";
    }
    match (visible_indices.first(), visible_indices.last()) {
        (Some(0), _) => "↓",
        (_, Some(last)) if last.saturating_add(1) == line_count => "↑",
        _ => "↕",
    }
}

pub(in super::super) fn home_action_visible_line_indices(
    app: &LabApp,
    line_count: usize,
    inner_height: usize,
    compact: bool,
) -> Vec<usize> {
    if inner_height == 0 || line_count == 0 {
        return Vec::new();
    }
    if line_count <= inner_height {
        return (0..line_count).collect();
    }

    let inner_height = inner_height.saturating_sub(1);
    if inner_height == 0 {
        return Vec::new();
    }

    let action_line_count = (HomeAction::ALL.len() * 3).min(line_count);
    let footer_count = if compact {
        0
    } else {
        line_count.saturating_sub(action_line_count)
    };
    let footer_capacity = if inner_height > footer_count {
        footer_count
    } else {
        0
    };
    let action_capacity = inner_height.saturating_sub(footer_capacity);
    if action_capacity == 0 || action_line_count == 0 {
        return Vec::new();
    }

    let selected = app
        .home_selected
        .min(HomeAction::ALL.len().saturating_sub(1));
    let action_start = if action_capacity >= 3 {
        let visible_bands = (action_capacity / 3).max(1).min(HomeAction::ALL.len());
        selected
            .saturating_sub(visible_bands / 2)
            .min(HomeAction::ALL.len().saturating_sub(visible_bands))
            * 3
    } else {
        let selected_line = selected * 3 + 1;
        selected_line
            .saturating_sub(action_capacity / 2)
            .min(action_line_count.saturating_sub(action_capacity))
    };
    let action_end = (action_start + action_capacity).min(action_line_count);
    let mut indices = (action_start..action_end).collect::<Vec<_>>();
    indices.extend(action_line_count..action_line_count + footer_capacity);
    indices
}

pub(in super::super) fn home_action_button_style(
    cli: &Cli,
    action: HomeAction,
    active: bool,
    selected: bool,
) -> Style {
    if cli.no_color {
        let mut style = Style::default();
        if selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        if active {
            style = style.add_modifier(Modifier::REVERSED);
        }
        return style;
    }
    terminal_button_surface_style(cli, home_action_color(action), active || selected)
}

pub(in super::super) fn home_action_detail(app: &LabApp, action: HomeAction) -> String {
    if action != HomeAction::Ledger {
        return action.detail().to_string();
    }
    if let Some(pubkey) = app.wallet.pubkey.as_deref() {
        format!("{} attached; view recent activity.", short_pubkey(pubkey))
    } else {
        "No wallet attached; attach a wallet to view history.".to_string()
    }
}

pub(in super::super) fn home_preview_title(action: HomeAction) -> &'static str {
    match action {
        HomeAction::Trade => "trade preview",
        HomeAction::Chart => "chart preview",
        HomeAction::Oracle => "oracle preview",
        HomeAction::Ledger => "ledger preview",
        HomeAction::Staking => "staking preview",
        HomeAction::Help => "help preview",
        HomeAction::ConnectAgents => "agent preview",
    }
}

use std::borrow::Cow;

pub(in super::super) fn home_preview_lines(
    cli: &Cli,
    app: &LabApp,
    action: HomeAction,
) -> Vec<Line<'static>> {
    let [primary, secondary] = home_preview_body(app, action);
    vec![
        Line::from(vec![
            Span::styled(
                action.label(),
                style(cli, home_action_accent_color(action)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" - ", style(cli, Color::DarkGray)),
            Span::styled(primary, style(cli, Color::White)),
        ]),
        Line::from(Span::styled(secondary, style(cli, Color::Gray))),
    ]
}

pub(in super::super) fn home_preview_body(
    app: &LabApp,
    action: HomeAction,
) -> [Cow<'static, str>; 2] {
    let market = || selected_market_context(app);
    match action {
        HomeAction::Trade => [
            format!("Open fixed-risk contracts for {}.", market()).into(),
            "Choose a call spread or put spread, then preview max loss and max payout before trading.".into(),
        ],
        HomeAction::Chart => [
            format!("See fair price, move, volume, and liquidity history for {}.", market()).into(),
            "Use range keys inside the chart to compare active windows.".into(),
        ],
        HomeAction::Oracle => [
            format!("Inspect the source trail for {} settlement.", market()).into(),
            "Each source is measured against its own opening value, then combined through frozen monthly weights.".into(),
        ],
        HomeAction::Ledger => [
            "Review wallet activity and Amoeba trade history.".into(),
            match app.wallet.pubkey.as_deref() {
                Some(pubkey) => format!("Attached account: {}", short_pubkey(pubkey)).into(),
                None => "Attach a wallet to use account history.".into(),
            },
        ],
        HomeAction::Staking => [
            "Queue AMBA for seven days, then activate it into transferable sAMBA shares.".into(),
            "Queued AMBA earns no rewards; activated sAMBA gains redeemable AMBA automatically.".into(),
        ],
        HomeAction::Help => [
            "Open Amoeba Farm overview, GitBook, Terms of Service, and product map.".into(),
            "A quick orientation for markets, charts, oracle evidence, and wallet history.".into(),
        ],
        HomeAction::ConnectAgents => [
            "Use Petri with Claude Code, Codex, Gemini, and other supported AI agents.".into(),
            "Connect once, repair in place if needed, or turn the connection off here.".into(),
        ],
    }
}

pub(in super::super) fn selected_market_context(app: &LabApp) -> String {
    app.trading
        .detail
        .as_ref()
        .map(|detail| {
            if detail.expiry_label.trim().is_empty() || detail.expiry_label == "-" {
                detail.symbol.clone()
            } else {
                format!("{} {}", detail.symbol, detail.expiry_label)
            }
        })
        .or_else(|| {
            app.trading
                .dishes
                .get(app.trading.selected)
                .map(|dish| dish.symbol.clone())
        })
        .unwrap_or_else(|| "the selected market".to_string())
}

pub(in super::super) fn selected_oracle_market_id(app: &LabApp) -> String {
    app.trading
        .detail
        .as_ref()
        .map(|detail| detail.id.clone())
        .or_else(|| {
            app.trading
                .dishes
                .get(app.trading.selected)
                .map(|dish| dish.id.clone())
        })
        .unwrap_or_else(|| app.selected_id())
}

pub(in super::super) fn selected_oracle_market_symbol(app: &LabApp) -> String {
    app.trading
        .detail
        .as_ref()
        .map(|detail| detail.symbol.clone())
        .or_else(|| {
            app.trading
                .dishes
                .get(app.trading.selected)
                .map(|dish| dish.symbol.clone())
        })
        .unwrap_or_else(|| selected_oracle_market_id(app).to_uppercase())
}

pub(in super::super) fn selected_oracle_month_label(app: &LabApp) -> String {
    app.trading
        .detail
        .as_ref()
        .map(|detail| detail.expiry_label.trim())
        .filter(|label| !label.is_empty() && *label != "-")
        .map(str::to_string)
        .unwrap_or_else(|| "selected month".to_string())
}

pub(in super::super) fn selected_oracle_settlement_label(app: &LabApp) -> String {
    app.trading
        .detail
        .as_ref()
        .map(|detail| detail.settlement.trim())
        .filter(|settlement| !settlement.is_empty() && *settlement != "-")
        .map(short_settlement_label)
        .unwrap_or_else(|| "loading".to_string())
}
