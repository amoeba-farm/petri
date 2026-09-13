//! Options-chain, protected exact-input order-ticket, and trade-risk presentation.

use super::super::*;

pub(in super::super) fn draw_trade_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::Activity;
    let border_style = trade_panel_border_style(cli, app, focused);
    let panel = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(tui_panel_style(cli))
        .title(Span::styled("selected contract", style(cli, Color::Green)));
    frame.render_widget(panel, area);

    if let Some((buy_area, sell_area)) = selected_contract_button_rects(area) {
        draw_trade_action_button(frame, cli, buy_area, app, TradeAction::Buy);
        draw_trade_action_button(frame, cli, sell_area, app, TradeAction::Sell);
    }

    if let Some(command_area) = selected_contract_command_rect(area) {
        let command = selected_contract_preview_command(app);
        let command_line = Paragraph::new(Line::from(vec![
            Span::styled("CLI: ", style(cli, Color::DarkGray)),
            Span::styled(command, style(cli, Color::Cyan)),
        ]));
        frame.render_widget(command_line, command_area);
    }
}

pub(in super::super) fn draw_trade_action_button(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    action: TradeAction,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let active = app.trading.action == action;
    let key = match action {
        TradeAction::Buy => "B",
        TradeAction::Sell => "S",
    };
    let color = match action {
        TradeAction::Buy => Color::Green,
        TradeAction::Sell => Color::Red,
    };
    let label = format!("{} ({key})", action.label().to_ascii_uppercase());
    let lines = raised_button_lines(cli, &label, color, active, area.width, area.height);
    let button = Paragraph::new(lines);
    frame.render_widget(button, area);
}

pub(in super::super) fn panel_inner_rect(area: Rect) -> Option<Rect> {
    if area.width <= 2 || area.height <= 2 {
        return None;
    }
    Some(Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    })
}

pub(in super::super) fn selected_contract_command_rect(area: Rect) -> Option<Rect> {
    let inner = panel_inner_rect(area)?;
    if inner.height == 0 {
        return None;
    }
    Some(Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    })
}

pub(in super::super) fn selected_contract_button_rects(area: Rect) -> Option<(Rect, Rect)> {
    let inner = panel_inner_rect(area)?;
    let max_button_height = inner.height.saturating_sub(1);
    if max_button_height == 0 || inner.width < 12 {
        return None;
    }
    let gap = if inner.width >= 60 { 3 } else { 1 };
    let available = inner.width.saturating_sub(gap);
    let button_width = (available / 2).min(SELECTED_CONTRACT_ACTION_BUTTON_MAX_WIDTH);
    if button_width < 5 {
        return None;
    }
    let pair_width = button_width.saturating_mul(2).saturating_add(gap);
    let button_height = max_button_height.min(3).max(1);
    let x = inner
        .x
        .saturating_add(inner.width.saturating_sub(pair_width) / 2);
    let buy = Rect {
        x,
        y: inner.y,
        width: button_width,
        height: button_height,
    };
    let sell = Rect {
        x: x.saturating_add(button_width).saturating_add(gap),
        y: inner.y,
        width: button_width,
        height: button_height,
    };
    Some((buy, sell))
}

pub(in super::super) fn selected_contract_preview_command(app: &LabApp) -> String {
    let Some(detail) = &app.trading.detail else {
        return "load a market to see the CLI command".to_string();
    };
    let Some(quote) = app.selected_quote() else {
        return "select a contract to see the CLI command".to_string();
    };
    let route_price = trade_route_price(quote, app.trading.action);
    let unavailable_label = trade_risk_unavailable_label(quote, app.trading.action);
    match (app.trading.action, route_price) {
        (_, Some(price)) => {
            trade_plan_command(detail, quote, app.trading.action, &format_decimal(price, 3))
        }
        (TradeAction::Sell, None) => {
            trade_plan_command(detail, quote, app.trading.action, "<price>")
        }
        (TradeAction::Buy, None) => {
            format!("price quote missing: {unavailable_label}; select a quoted contract or refresh")
        }
    }
}

pub(in super::super) fn trade_panel_border_style(cli: &Cli, app: &LabApp, focused: bool) -> Style {
    if app.trading.ticket.is_some() {
        let blink_on = (app.spinner_tick / 2) % 2 == 0;
        if cli.no_color {
            let mut style = Style::default().add_modifier(Modifier::BOLD);
            if blink_on {
                style = style.add_modifier(Modifier::REVERSED);
            }
            return style;
        }
        let color = if blink_on {
            Color::LightYellow
        } else {
            Color::LightRed
        };
        return Style::default().fg(color).add_modifier(Modifier::BOLD);
    }

    panel_border_style(cli, focused)
}

pub(in super::super) fn draw_chain_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let Some(detail) = &app.trading.detail else {
        let label = if app.trading.loading_detail {
            format!(
                "{} Loading {} market...",
                app.spinner(),
                app.selected_id().to_uppercase()
            )
        } else if app.trading.loading_list {
            format!("{} Loading markets...", app.spinner())
        } else {
            "Market is not available right now.".to_string()
        };
        let focused = app.focus == LabFocus::Detail;
        let panel = scrolling_panel(
            vec![
                Line::from(Span::styled(
                    label,
                    style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    app.status.clone(),
                    status_style(cli, &app.status),
                )),
            ],
            area,
            cli,
            app.focused_panel_scroll(LabFocus::Detail),
            focused,
        )
        .block(panel_block(cli, "options", Color::Cyan, focused));
        frame.render_widget(panel, area);
        return;
    };

    if area.height < 7 {
        let focused = app.focus == LabFocus::Detail;
        let panel = scrolling_panel(
            chain_summary_lines(cli, app, detail),
            area,
            cli,
            app.focused_panel_scroll(LabFocus::Detail),
            focused,
        )
        .block(panel_block(cli, "options", Color::Cyan, focused));
        frame.render_widget(panel, area);
        return;
    }

    let Some(chain_layout) = chain_layout(cli, area, app) else {
        return;
    };
    let summary_focused = app.focus == LabFocus::Detail;
    let summary = scrolling_panel(
        chain_summary_lines(cli, app, detail),
        chain_layout.summary,
        cli,
        app.focused_panel_scroll(LabFocus::Detail),
        summary_focused,
    )
    .block(panel_block(cli, "options", Color::Cyan, summary_focused));
    frame.render_widget(summary, chain_layout.summary);

    draw_option_side(
        frame,
        cli,
        chain_layout.calls,
        detail,
        app,
        OptionKind::Call,
    );
    draw_option_side(frame, cli, chain_layout.puts, detail, app, OptionKind::Put);
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct ChainLayout {
    pub(in super::super) summary: Rect,
    pub(in super::super) calls: Rect,
    pub(in super::super) puts: Rect,
}

pub(in super::super) fn chain_layout(cli: &Cli, area: Rect, app: &LabApp) -> Option<ChainLayout> {
    if area.height < 7 {
        return None;
    }
    let summary_height = chain_summary_panel_height(cli, area, app);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(summary_height), Constraint::Min(3)])
        .split(area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    Some(ChainLayout {
        summary: rows[0],
        calls: columns[0],
        puts: columns[1],
    })
}

pub(in super::super) fn chain_focus_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<LabFocus> {
    let layout = chain_layout(cli, area, app)?;
    if rect_contains(layout.summary, column, row) {
        Some(LabFocus::Detail)
    } else if rect_contains(layout.calls, column, row) {
        Some(LabFocus::Calls)
    } else if rect_contains(layout.puts, column, row) {
        Some(LabFocus::Puts)
    } else {
        None
    }
}

pub(in super::super) fn active_order_ticket_area(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<Rect> {
    if app.screen != LabScreen::Chain || app.trading.ticket.is_none() {
        return None;
    }
    let layout = chain_layout(cli, area, app)?;
    let side_area = match app.trading.chain_focus {
        ChainFocus::Calls => layout.calls,
        ChainFocus::Puts => layout.puts,
        ChainFocus::Markets => return None,
    };
    option_side_layout(side_area, true).order_ticket
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct TradeTicketFieldRects {
    pub(in super::super) premium: Rect,
    pub(in super::super) quantity: Rect,
}

pub(in super::super) fn trade_ticket_field_rects(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<TradeTicketFieldRects> {
    let ticket = app.trading.ticket.as_ref()?;
    let inner = panel_inner_rect(area)?;
    let price_label_width = ticket.action.price_label().chars().count() as u16 + 1;
    let price_input_width = ticket_input_display_width(
        cli,
        &ticket.premium_input,
        "type price",
        ticket.field == TradeTicketField::Premium,
        false,
    );
    let quantity_label_width = "Contracts ".chars().count() as u16;
    let quantity_input_width = ticket_input_display_width(
        cli,
        &ticket.quantity_input,
        "quantity",
        ticket.field == TradeTicketField::Quantity,
        false,
    );
    let compact = inner.width < ORDER_TICKET_COMPACT_WIDTH;
    let (price_y, quantity_x, quantity_y) = if compact {
        if inner.height < 2 {
            return None;
        }
        (inner.y, inner.x, inner.y.saturating_add(1))
    } else {
        if inner.height < 2 {
            return None;
        }
        let quantity_x = inner
            .x
            .saturating_add(price_label_width)
            .saturating_add(price_input_width)
            .saturating_add(" | ".chars().count() as u16);
        (
            inner.y.saturating_add(1),
            quantity_x,
            inner.y.saturating_add(1),
        )
    };
    let price_width = price_label_width
        .saturating_add(price_input_width)
        .min(inner.width);
    let quantity_width = quantity_label_width
        .saturating_add(quantity_input_width)
        .min(
            inner
                .x
                .saturating_add(inner.width)
                .saturating_sub(quantity_x),
        );

    Some(TradeTicketFieldRects {
        premium: Rect {
            x: inner.x,
            y: price_y,
            width: price_width,
            height: 1,
        },
        quantity: Rect {
            x: quantity_x,
            y: quantity_y,
            width: quantity_width,
            height: 1,
        },
    })
}

pub(in super::super) fn trade_ticket_field_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<TradeTicketField> {
    if app.trading.submit_is_running() || app.trading.confirmation_is_open() {
        return None;
    }
    let rects = trade_ticket_field_rects(cli, area, app)?;
    if rect_contains(rects.premium, column, row) {
        return Some(TradeTicketField::Premium);
    }
    if rect_contains(rects.quantity, column, row) {
        return Some(TradeTicketField::Quantity);
    }
    None
}

pub(in super::super) fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

pub(in super::super) fn mouse_source_line_at(
    line_count: usize,
    panel_area: Rect,
    scroll: usize,
    column: u16,
    row: u16,
) -> Option<usize> {
    let inner = panel_inner_rect(panel_area)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible_row = row.saturating_sub(inner.y) as usize;
    visible_source_line_at(line_count, inner.height as usize, scroll, visible_row)
}

pub(in super::super) fn visible_source_line_at(
    line_count: usize,
    height: usize,
    scroll: usize,
    visible_row: usize,
) -> Option<usize> {
    if line_count == 0 || height == 0 || visible_row >= height {
        return None;
    }
    if line_count <= height {
        return (visible_row < line_count).then_some(visible_row);
    }
    if height == 1 {
        return None;
    }

    let max_scroll = line_count.saturating_sub(height.saturating_sub(1).max(1));
    let start = scroll.min(max_scroll);
    let has_top_overflow = start > 0;
    let mut content_capacity = height.saturating_sub(usize::from(has_top_overflow));
    let mut end = (start + content_capacity).min(line_count);
    if end < line_count {
        content_capacity = content_capacity.saturating_sub(1);
        end = (start + content_capacity).min(line_count);
    }

    let content_row = if has_top_overflow {
        if visible_row == 0 {
            return None;
        }
        visible_row - 1
    } else {
        visible_row
    };
    let content_len = end.saturating_sub(start);
    if content_row >= content_len {
        return None;
    }
    Some(start + content_row)
}

pub(in super::super) fn wrapped_mouse_source_line_at(
    lines: &[Line<'static>],
    panel_area: Rect,
    scroll: usize,
    column: u16,
    row: u16,
    trim: bool,
) -> Option<usize> {
    let inner = panel_inner_rect(panel_area)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let mut visible_row = usize::from(row.saturating_sub(inner.y));
    let viewport =
        wrapped_line_viewport(lines, inner.width, usize::from(inner.height), scroll, trim);
    if viewport.hidden_above > 0 {
        if visible_row == 0 {
            return None;
        }
        visible_row = visible_row.saturating_sub(1);
    }
    for source_line in viewport.start..viewport.end {
        let line_height = wrapped_line_height(&lines[source_line], usize::from(inner.width), trim);
        if visible_row < line_height {
            return Some(source_line);
        }
        visible_row = visible_row.saturating_sub(line_height);
    }
    None
}

pub(in super::super) fn market_rail_hit_at(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<MarketRailHit> {
    let lines = dish_list_lines(cli, app);
    let panel_area = wrapped_content_sized_panel_area(area, &lines, true);
    let scroll_focus = if app.focus == LabFocus::MarketSeries {
        LabFocus::MarketSeries
    } else {
        LabFocus::Markets
    };
    let source_line = wrapped_mouse_source_line_at(
        &lines,
        panel_area,
        app.focused_panel_scroll(scroll_focus),
        column,
        row,
        true,
    )?;
    market_rail_source_hit(app, source_line)
}

pub(in super::super) fn external_link_row_at(
    area: Rect,
    column: u16,
    row: u16,
    targets: &[(usize, ExternalLinkTarget)],
) -> Option<ExternalLinkTarget> {
    let inner = panel_inner_rect(area)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible_row = usize::from(row.saturating_sub(inner.y));
    targets
        .iter()
        .find_map(|(target_row, target)| (*target_row == visible_row).then_some(*target))
}

pub(in super::super) fn external_link_hit_at(
    _cli: &Cli,
    root: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<ExternalLinkTarget> {
    let frame = lab_frame_layout(root, _cli, app);
    let body = lab_body_layout(app.screen, frame.body_area);

    if app.screen == LabScreen::Terms && app.focused_panel_scroll(LabFocus::Terms) == 0 {
        return external_link_row_at(
            frame.body_area,
            column,
            row,
            &[(2, ExternalLinkTarget::Terms)],
        );
    }

    if app.screen == LabScreen::Help && app.home_help_topic == HomeHelpTopic::Overview {
        if app.help.article_scroll > 0 {
            return None;
        }
        let help = gitbook_help_layout(body.selected_area);
        return external_link_row_at(
            help.article_area,
            column,
            row,
            &[
                (1, ExternalLinkTarget::Docs),
                (2, ExternalLinkTarget::Terms),
            ],
        );
    }

    if app.screen == LabScreen::Help && app.home_help_topic == HomeHelpTopic::Agents {
        let links = agent_connection_layout(body.selected_area).links_area?;
        return external_link_row_at(
            links,
            column,
            row,
            &[
                (0, ExternalLinkTarget::Docs),
                (1, ExternalLinkTarget::Terms),
            ],
        );
    }

    if app.screen == LabScreen::OracleHelp {
        let links = oracle_help_layout(body.selected_area).links_area?;
        return external_link_row_at(
            links,
            column,
            row,
            &[
                (0, ExternalLinkTarget::Docs),
                (1, ExternalLinkTarget::Terms),
            ],
        );
    }

    None
}

pub(in super::super) fn market_rail_source_hit(
    app: &LabApp,
    source_line: usize,
) -> Option<MarketRailHit> {
    market_rail_dense_source_hit(app, source_line)
}

pub(in super::super) fn market_rail_dense_source_hit(
    app: &LabApp,
    source_line: usize,
) -> Option<MarketRailHit> {
    let mut line_index = 0usize;
    for (market_index, dish) in app.trading.dishes.iter().enumerate() {
        if source_line == line_index {
            return Some(MarketRailHit::Market(market_index));
        }
        line_index += 1;

        if market_index == app.trading.selected && app.trading.market_series_open {
            let series = market_series_labels(app, dish);
            if series.is_empty() {
                if source_line == line_index {
                    return None;
                }
                line_index += 1;
                continue;
            }

            if source_line == line_index {
                return None;
            }
            line_index += 1;
            for series_index in 0..series.len() {
                if source_line == line_index {
                    return Some(MarketRailHit::Series(series_index));
                }
                line_index += 1;
            }
        }
    }
    None
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct HomePanelRects {
    pub(in super::super) summary: Rect,
    pub(in super::super) actions: Rect,
    pub(in super::super) preview: Option<Rect>,
    pub(in super::super) compact: bool,
}

pub(in super::super) fn home_panel_rects(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<HomePanelRects> {
    home_panel_rects_for_focus(cli, area, app, app.focus)
}

pub(in super::super) fn home_panel_rects_for_focus(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    active_focus: LabFocus,
) -> Option<HomePanelRects> {
    if area.height == 0 {
        return None;
    }
    let summary_lines = home_summary_lines(cli, app);
    if area.height < 16 {
        let action_lines = home_action_lines(cli, app, true, app.focus == LabFocus::HomeActions);
        let fallback_lengths = [5, area.height.saturating_sub(5)];
        let rows = focused_stack_rects(
            area,
            active_focus,
            &[
                FocusedStackPanel::new(Some(LabFocus::HomeSummary), summary_lines.len(), 5),
                FocusedStackPanel::new(Some(LabFocus::HomeActions), action_lines.len(), 5),
            ],
            &fallback_lengths,
        );
        return (rows.len() >= 2).then_some(HomePanelRects {
            summary: rows[0],
            actions: rows[1],
            preview: None,
            compact: true,
        });
    }

    let action_lines = home_action_lines(cli, app, false, app.focus == LabFocus::HomeActions);
    let preview_lines = home_preview_lines(cli, app, app.selected_home_action());
    let fallback_lengths = [5, area.height.saturating_sub(10), area.height.min(5)];
    let rows = focused_stack_rects(
        area,
        active_focus,
        &[
            FocusedStackPanel::new(Some(LabFocus::HomeSummary), summary_lines.len(), 5),
            FocusedStackPanel::new(Some(LabFocus::HomeActions), action_lines.len(), 10),
            FocusedStackPanel::new(Some(LabFocus::HomePreview), preview_lines.len(), 5),
        ],
        &fallback_lengths,
    );
    (rows.len() >= 3).then_some(HomePanelRects {
        summary: rows[0],
        actions: rows[1],
        preview: Some(rows[2]),
        compact: false,
    })
}

pub(in super::super) fn home_action_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = home_panel_rects(cli, area, app)?;
    let lines = home_action_lines(cli, app, layout.compact, app.focus == LabFocus::HomeActions);
    let inner_height = panel_inner_height(layout.actions);
    let inner = panel_inner_rect(layout.actions)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible_row = row.saturating_sub(inner.y) as usize;
    let source_line =
        *home_action_visible_line_indices(app, lines.len(), inner_height, layout.compact)
            .get(visible_row)?;
    home_action_source_hit(source_line)
}

pub(in super::super) fn home_action_source_hit(source_line: usize) -> Option<usize> {
    let action_index = source_line / 3;
    (action_index < HomeAction::ALL.len()).then_some(action_index)
}

pub(in super::super) fn home_focus_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<LabFocus> {
    let layout = home_panel_rects(cli, area, app)?;
    if rect_contains(layout.summary, column, row) {
        Some(LabFocus::HomeSummary)
    } else if rect_contains(layout.actions, column, row) {
        Some(LabFocus::HomeActions)
    } else if layout
        .preview
        .is_some_and(|preview| rect_contains(preview, column, row))
    {
        Some(LabFocus::HomePreview)
    } else {
        None
    }
}

pub(in super::super) fn chart_contract_activity_area(area: Rect, app: &LabApp) -> Option<Rect> {
    if app.trading.chart.is_none() || area.height < 12 {
        return None;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(area);
    Some(rows[1])
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct OraclePanelRects {
    pub(in super::super) selected: Option<Rect>,
    pub(in super::super) actions: Option<Rect>,
    pub(in super::super) tree: Rect,
    pub(in super::super) flow: Option<Rect>,
    pub(in super::super) path: Option<Rect>,
}

pub(in super::super) fn oracle_panel_rects(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<OraclePanelRects> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let area = oracle_view_layout(area).content_area;
    if area.width == 0 || area.height == 0 {
        return None;
    }

    if area.width >= 96 && area.height >= 15 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(area);

        if app.oracle_tree().is_none() {
            return Some(OraclePanelRects {
                selected: None,
                actions: None,
                tree: columns[0],
                flow: Some(columns[1]),
                path: None,
            });
        }

        let selected_height = oracle_selected_source_panel_height(area.height);
        let action_line_count = oracle_action_lines(cli, app).len();
        let actions_height = oracle_context_actions_panel_height(
            area.height,
            selected_height + 8,
            action_line_count,
        );
        let main_fallback_lengths = [
            selected_height,
            actions_height,
            columns[0]
                .height
                .saturating_sub(selected_height.saturating_add(actions_height)),
        ];
        let main_rows = focused_stack_rects(
            columns[0],
            app.focus,
            &[
                FocusedStackPanel::new(
                    None,
                    oracle_overview_lines(cli, app).len(),
                    selected_height,
                ),
                FocusedStackPanel::new(
                    Some(LabFocus::OracleActions),
                    action_line_count,
                    actions_height.min(8).max(5),
                ),
                FocusedStackPanel::new(
                    Some(LabFocus::OracleTasks),
                    oracle_tree_lines(cli, app).len(),
                    8,
                ),
            ],
            &main_fallback_lengths,
        );

        let flow_height = oracle_flow_stack_height(area.height);
        let right_fallback_lengths = [flow_height, columns[1].height.saturating_sub(flow_height)];
        let right_rows = focused_stack_rects(
            columns[1],
            app.focus,
            &[
                FocusedStackPanel::new(
                    Some(LabFocus::OracleOverview),
                    oracle_flow_lines(cli, app).len(),
                    flow_height.min(8).max(5),
                ),
                FocusedStackPanel::new(
                    Some(LabFocus::OraclePath),
                    oracle_current_path_lines(cli, app).len(),
                    6,
                ),
            ],
            &right_fallback_lengths,
        );

        return Some(OraclePanelRects {
            selected: main_rows.first().copied(),
            actions: main_rows.get(1).copied(),
            tree: *main_rows.get(2)?,
            flow: right_rows.first().copied(),
            path: right_rows.get(1).copied(),
        });
    }

    if app.oracle_tree().is_none() {
        return Some(OraclePanelRects {
            selected: None,
            actions: None,
            tree: area,
            flow: None,
            path: None,
        });
    }

    let selected_height = oracle_selected_source_panel_height(area.height);
    let lower_height = if area.height >= 24 { 8 } else { 0 };
    let action_line_count = oracle_action_lines(cli, app).len();
    let action_height = oracle_context_actions_panel_height(
        area.height,
        selected_height + lower_height + 6,
        action_line_count,
    );
    let fallback_lengths = [
        selected_height,
        action_height,
        area.height.saturating_sub(
            selected_height
                .saturating_add(action_height)
                .saturating_add(lower_height),
        ),
        lower_height,
    ];
    let mut panels = vec![
        FocusedStackPanel::new(None, oracle_overview_lines(cli, app).len(), selected_height),
        FocusedStackPanel::new(
            Some(LabFocus::OracleActions),
            action_line_count,
            action_height.min(8).max(5),
        ),
        FocusedStackPanel::new(
            Some(LabFocus::OracleTasks),
            oracle_tree_lines(cli, app).len(),
            6,
        ),
    ];
    if lower_height > 0 {
        panels.push(FocusedStackPanel::new(
            Some(LabFocus::OraclePath),
            oracle_current_path_lines(cli, app).len(),
            lower_height.min(8).max(5),
        ));
    }
    let rows = focused_stack_rects(area, app.focus, &panels, &fallback_lengths);

    Some(OraclePanelRects {
        selected: rows.first().copied(),
        actions: rows.get(1).copied(),
        tree: *rows.get(2)?,
        flow: None,
        path: (lower_height > 0).then(|| rows.get(3).copied()).flatten(),
    })
}

pub(in super::super) fn oracle_intro_action_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let lines = oracle_intro_lines(cli, app, app.focus == LabFocus::OracleIntro);
    let source_line = mouse_source_line_at(
        lines.len(),
        area,
        app.focused_panel_scroll(LabFocus::OracleIntro),
        column,
        row,
    )?;
    let action_start = 6usize;
    source_line
        .checked_sub(action_start)
        .filter(|index| *index < OracleIntroAction::ALL.len())
}

pub(in super::super) fn oracle_focus_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<LabFocus> {
    let layout = oracle_panel_rects(cli, area, app)?;
    if layout
        .actions
        .is_some_and(|actions| rect_contains(actions, column, row))
    {
        Some(LabFocus::OracleActions)
    } else if rect_contains(layout.tree, column, row) {
        Some(LabFocus::OracleTasks)
    } else if layout
        .flow
        .is_some_and(|flow| rect_contains(flow, column, row))
    {
        Some(LabFocus::OracleOverview)
    } else if layout
        .path
        .is_some_and(|path| rect_contains(path, column, row))
    {
        Some(LabFocus::OraclePath)
    } else if layout
        .selected
        .is_some_and(|selected| rect_contains(selected, column, row))
    {
        Some(LabFocus::OracleOverview)
    } else {
        None
    }
}

pub(in super::super) fn oracle_tree_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = oracle_panel_rects(cli, area, app)?;
    let lines = oracle_tree_lines(cli, app);
    let source_line = mouse_source_line_at(
        lines.len(),
        layout.tree,
        app.focused_panel_scroll(LabFocus::OracleTasks),
        column,
        row,
    )?;
    oracle_tree_node_at_source_line(app, source_line)
}

pub(in super::super) fn oracle_tree_node_at_source_line(
    app: &LabApp,
    source_line: usize,
) -> Option<usize> {
    let tree = app.oracle_tree()?;
    if source_line <= 1 {
        return Some(app.selected_oracle_node_index());
    }
    let mut cursor = 2usize;
    if app.oracle.tree_issue.is_some() {
        cursor += 1;
    }
    if app.trading.detail.is_none() {
        cursor += 1;
    }
    cursor += 1;

    let matches = tree.search_nodes(&app.oracle.search_input);
    if !matches.is_empty() {
        let match_start = cursor + 1;
        let offset = source_line.checked_sub(match_start)?;
        return matches.into_iter().take(10).nth(offset);
    }

    let tree_start = cursor + 1;
    let offset = source_line.checked_sub(tree_start)?;
    let path_indices = oracle_path_indices(tree, app.selected_oracle_node_index());
    let visible = oracle_visible_tree_node_indices(tree, &path_indices);
    visible.get(offset).copied()
}

pub(in super::super) fn oracle_visible_tree_node_indices(
    tree: &OracleIndexTree,
    path_indices: &[usize],
) -> Vec<usize> {
    let mut indices = Vec::new();
    oracle_push_visible_tree_node_indices(tree, tree.root_index(), path_indices, &mut indices);
    indices
}

pub(in super::super) fn oracle_push_visible_tree_node_indices(
    tree: &OracleIndexTree,
    index: usize,
    path_indices: &[usize],
    indices: &mut Vec<usize>,
) {
    indices.push(index);
    if !path_indices.contains(&index) {
        return;
    }

    for child in tree.child_indices(index) {
        if path_indices.contains(&child) {
            oracle_push_visible_tree_node_indices(tree, child, path_indices, indices);
        } else {
            indices.push(child);
        }
    }
}

pub(in super::super) fn oracle_action_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<OracleAction> {
    if app.oracle.form.is_some() {
        return None;
    }
    let layout = oracle_panel_rects(cli, area, app)?;
    let actions = layout.actions?;
    let lines = oracle_action_lines(cli, app);
    let source_line = mouse_source_line_at(
        lines.len(),
        actions,
        app.focused_panel_scroll(LabFocus::OracleActions),
        column,
        row,
    )?;
    oracle_action_at_source_line(app, source_line)
}

pub(in super::super) fn oracle_action_at_source_line(
    app: &LabApp,
    source_line: usize,
) -> Option<OracleAction> {
    let context = app.selected_oracle_action_context()?;
    let mut locked_actions = Vec::new();
    let mut visible_actions = Vec::new();
    for action in app.visible_oracle_actions() {
        if action.availability(context) == OracleActionAvailability::Locked {
            locked_actions.push(action);
        } else {
            visible_actions.push(action);
        }
    }

    if !locked_actions.is_empty() {
        if source_line == 0 {
            let selected = app.selected_oracle_action();
            return locked_actions
                .iter()
                .copied()
                .find(|action| *action == selected)
                .or_else(|| locked_actions.first().copied());
        }
        return visible_actions.get(source_line - 1).copied();
    }

    visible_actions.get(source_line).copied()
}

pub(in super::super) fn oracle_form_field_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let form = app.oracle.form.as_ref()?;
    let layout = oracle_panel_rects(cli, area, app)?;
    let actions = layout.actions?;
    let inner = panel_inner_rect(actions)?;
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible_row = row.saturating_sub(inner.y) as usize;
    let full_line_count = oracle_form_lines(cli, app, form).len();
    oracle_form_field_at_visible_row(form, inner.height as usize, full_line_count, visible_row)
}

pub(in super::super) fn oracle_form_field_at_visible_row(
    form: &OracleFormDraft,
    inner_height: usize,
    full_line_count: usize,
    visible_row: usize,
) -> Option<usize> {
    let header_count = 3usize;
    let field_count = form.fields.len();
    if inner_height == 0 || field_count == 0 || visible_row >= inner_height {
        return None;
    }

    let selected = form.field_selected.min(field_count.saturating_sub(1));
    if full_line_count <= inner_height || inner_height <= header_count + 2 {
        return (0..field_count).find(|index| {
            header_count + oracle_form_field_line_offset(*index, selected) == visible_row
        });
    }

    let (start, end, spaced) = oracle_form_field_window(form, inner_height)?;
    let mut row = header_count;
    for index in start..end {
        if spaced && index == selected {
            row += 1;
        }
        if row >= inner_height {
            break;
        }
        if row == visible_row {
            return Some(index);
        }
        row += 1;
        if spaced && index == selected {
            row += 1;
        }
    }
    None
}

pub(in super::super) fn oracle_form_field_line_offset(index: usize, selected: usize) -> usize {
    index
        + if index < selected {
            0
        } else if index == selected {
            1
        } else {
            2
        }
}

pub(in super::super) fn oracle_form_field_window(
    form: &OracleFormDraft,
    inner_height: usize,
) -> Option<(usize, usize, bool)> {
    let header_count = 3usize;
    let field_count = form.fields.len();
    if field_count == 0 || inner_height <= header_count + 2 {
        return None;
    }
    let selected = form.field_selected.min(field_count.saturating_sub(1));
    let rows_for_fields = inner_height.saturating_sub(header_count + 1);
    let spaced = rows_for_fields >= 3;
    let spacer_count = usize::from(spaced) * 2;
    let field_budget = rows_for_fields
        .saturating_sub(spacer_count)
        .max(1)
        .min(field_count);
    let mut start = selected.saturating_sub(field_budget / 2);
    if start + field_budget > field_count {
        start = field_count.saturating_sub(field_budget);
    }
    Some((start, (start + field_budget).min(field_count), spaced))
}

pub(in super::super) fn oracle_path_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let tree = app.oracle_tree()?;
    let layout = oracle_panel_rects(cli, area, app)?;
    let path_area = layout.path?;
    let lines = oracle_current_path_lines(cli, app);
    let source_line = mouse_source_line_at(
        lines.len(),
        path_area,
        app.focused_panel_scroll(LabFocus::OraclePath),
        column,
        row,
    )?;
    if source_line == 0 {
        return Some(tree.root_index());
    }
    let path_indices = oracle_path_indices(tree, app.selected_oracle_node_index());
    path_indices.get(source_line - 1).copied()
}

pub(in super::super) fn option_quote_hit_at(
    cli: &Cli,
    area: Rect,
    detail: &DishDetail,
    app: &LabApp,
    kind: OptionKind,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = chain_layout(cli, area, app)?;
    let side_area = match kind {
        OptionKind::Call => layout.calls,
        OptionKind::Put => layout.puts,
    };
    let side_focused = matches!(
        (app.trading.chain_focus, kind),
        (ChainFocus::Calls, OptionKind::Call) | (ChainFocus::Puts, OptionKind::Put)
    );
    let side_layout = option_side_layout(side_area, side_focused && app.trading.ticket.is_some());
    let table_area = side_layout.table;
    let inner = panel_inner_rect(table_area)?;
    if !rect_contains(inner, column, row) || row == inner.y {
        return None;
    }

    let indices = quote_indices_by_kind(detail, kind);
    if indices.is_empty() {
        return None;
    }
    let side_scroll = app.focused_panel_scroll(option_kind_focus(kind));
    let visible_rows = table_area.height.saturating_sub(3).max(1) as usize;
    let visible_row = row.saturating_sub(inner.y + 1) as usize;
    let source_rank =
        visible_source_line_at(indices.len(), visible_rows, side_scroll, visible_row)?;
    indices.get(source_rank).copied()
}

pub(in super::super) fn option_kind_focus(kind: OptionKind) -> LabFocus {
    match kind {
        OptionKind::Call => LabFocus::Calls,
        OptionKind::Put => LabFocus::Puts,
    }
}

pub(in super::super) fn chain_summary_panel_height(cli: &Cli, area: Rect, app: &LabApp) -> u16 {
    let preferred = app.trading.detail.as_ref().map_or(5, |detail| {
        wrapped_content_sized_panel_area(area, &chain_summary_lines(cli, app, detail), true).height
    });
    // Size the summary from its actual wrapped rows, not a fixed three-line
    // assumption. Keep a table header/quote visible and, when open, preserve
    // the minimum side-panel height needed by option_side_layout for the ticket.
    let side_minimum = if app.trading.ticket.is_some() { 12 } else { 4 };
    let summary_budget = area.height.saturating_sub(side_minimum).max(3);
    preferred.max(3).min(summary_budget).min(area.height)
}

pub(in super::super) fn chain_summary_lines(
    cli: &Cli,
    app: &LabApp,
    detail: &DishDetail,
) -> Vec<Line<'static>> {
    let selected_quote = app.selected_quote();
    let risk_preview =
        selected_quote.and_then(|quote| trade_risk_preview(quote, app.trading.action));
    let unavailable_label = selected_quote
        .map(|quote| trade_risk_unavailable_label(quote, app.trading.action))
        .unwrap_or("needs contract");
    let max_loss_label = risk_preview
        .map(|risk| format_usd(risk.max_loss_per_contract))
        .unwrap_or_else(|| unavailable_label.to_string());
    let max_gain_label = risk_preview
        .map(|risk| format_usd(risk.max_gain_per_contract))
        .unwrap_or_else(|| unavailable_label.to_string());

    let market_context = if selected_quote.is_none() && detail.status == "Not ready" {
        let mut message = detail.execution.clone();
        if let Some(issue) = detail.issues.first() {
            message.push_str(" | ");
            message.push_str(issue);
        }
        Line::from(Span::styled(message, style(cli, Color::Yellow)))
    } else {
        option_itm_bar_line(cli, app, detail)
    };

    vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", detail.symbol),
                style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(detail.expiry_label.clone(), style(cli, Color::White)),
            divider_span(cli),
            Span::styled(
                market_price_context(detail),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            Span::styled(
                short_freshness_label(&detail.freshness),
                status_style(cli, &detail.freshness),
            ),
            Span::styled(" | settles ", style(cli, Color::DarkGray)),
            Span::styled(
                short_settlement_label(&detail.settlement),
                style(cli, Color::White),
            ),
        ]),
        market_context,
        Line::from(vec![
            Span::styled("Max loss/contract ", style(cli, Color::DarkGray)),
            Span::styled(
                max_loss_label,
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            Span::styled("Max gain/contract ", style(cli, Color::DarkGray)),
            Span::styled(
                max_gain_label,
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            no_liquidation_span(cli),
        ]),
    ]
}

pub(in super::super) fn option_itm_bar_line(
    cli: &Cli,
    app: &LabApp,
    detail: &DishDetail,
) -> Line<'static> {
    let selected = app.selected_quote();
    let probability = selected.and_then(|quote| quote.probability_itm);
    let contract = selected
        .map(|quote| {
            format!(
                "{} {} {}/{}",
                detail.symbol,
                quote.kind.label(),
                quote.lower_strike,
                quote.upper_strike
            )
        })
        .unwrap_or_else(|| format!("{} selected contract", detail.symbol));
    let (percent, value_style) = match probability {
        Some(value) => (
            format!("{:.0}%", (value * 100.0).round()),
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        None => ("n/a".to_string(), style(cli, Color::DarkGray)),
    };

    let mut spans = vec![Span::styled(
        "ITM odds ".to_string(),
        style(cli, Color::DarkGray),
    )];
    spans.extend(itm_probability_bar_spans(cli, probability, 20));
    spans.extend([
        Span::styled(" ", style(cli, Color::DarkGray)),
        Span::styled(percent, value_style),
        divider_span(cli),
        Span::styled(contract, style(cli, Color::White)),
    ]);
    Line::from(spans)
}

pub(in super::super) fn itm_probability_bar_spans(
    cli: &Cli,
    probability: Option<f64>,
    width: usize,
) -> Vec<Span<'static>> {
    if width == 0 {
        return vec![
            Span::styled("[", style(cli, Color::DarkGray)),
            Span::styled("]", style(cli, Color::DarkGray)),
        ];
    }

    let filled = probability
        .map(|value| ((value.clamp(0.0, 1.0) * width as f64).round() as usize).min(width))
        .unwrap_or(0);
    let mut spans = Vec::with_capacity(width + 2);
    spans.push(Span::styled("[", style(cli, Color::DarkGray)));
    for index in 0..width {
        let active = index < filled;
        let glyph = if active { "█" } else { "░" };
        let segment_style = if active {
            itm_rainbow_segment_style(cli, index, width)
        } else {
            style(cli, Color::DarkGray)
        };
        spans.push(Span::styled(glyph, segment_style));
    }
    spans.push(Span::styled("]", style(cli, Color::DarkGray)));
    spans
}

pub(in super::super) fn itm_rainbow_segment_style(cli: &Cli, index: usize, width: usize) -> Style {
    if cli.no_color {
        return Style::default();
    }

    let ratio = if width <= 1 {
        0.0
    } else {
        index as f64 / (width - 1) as f64
    };
    let color = if ratio < 0.17 {
        Color::Rgb(255, 75, 75)
    } else if ratio < 0.34 {
        Color::Rgb(255, 155, 48)
    } else if ratio < 0.50 {
        Color::Rgb(246, 220, 72)
    } else if ratio < 0.67 {
        Color::Rgb(66, 214, 107)
    } else if ratio < 0.84 {
        Color::Rgb(64, 190, 255)
    } else {
        Color::Rgb(185, 117, 255)
    };

    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub(in super::super) fn draw_option_side(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    detail: &DishDetail,
    app: &LabApp,
    kind: OptionKind,
) {
    let indices = quote_indices_by_kind(detail, kind);
    let focused = matches!(
        (app.trading.chain_focus, kind),
        (ChainFocus::Calls, OptionKind::Call) | (ChainFocus::Puts, OptionKind::Put)
    );
    let side_name = match kind {
        OptionKind::Call => "CALLS",
        OptionKind::Put => "PUTS",
    };
    let title = if focused {
        format!("{side_name} *")
    } else {
        side_name.to_string()
    };
    let title_style = if focused {
        style(cli, option_kind_color(kind)).add_modifier(Modifier::BOLD)
    } else {
        style(cli, Color::Cyan)
    };
    let side_layout = option_side_layout(area, focused && app.trading.ticket.is_some());
    let table_area = side_layout.table;
    let table_width = table_area.width.saturating_sub(2);
    let depth_content_width = indices
        .iter()
        .map(|index| text_width(&compact_depth_label(&detail.option_quotes[*index])))
        .max()
        .unwrap_or_default()
        .max(text_width("DEPTH"));
    let table_layout = option_table_layout(table_width, depth_content_width);
    let mut lines = option_side_header(cli, table_layout);
    if indices.is_empty() {
        let empty_message = match kind {
            OptionKind::Call => "No active call spreads for this month yet.",
            OptionKind::Put => "No active put spreads for this month yet.",
        };
        lines.push(Line::from(Span::styled(
            empty_message,
            style(cli, Color::Yellow),
        )));
    } else {
        let visible_rows = table_area.height.saturating_sub(3).max(1) as usize;
        let quote_lines = indices
            .iter()
            .enumerate()
            .map(|(row_index, index)| {
                let quote = &detail.option_quotes[*index];
                let selected = focused && *index == app.trading.selected_option;
                option_side_row(
                    cli,
                    quote,
                    selected,
                    table_width,
                    table_layout,
                    row_index % 2 == 1,
                )
            })
            .collect::<Vec<_>>();
        let side_scroll = app.focused_panel_scroll(option_kind_focus(kind));
        for line in scroll_lines_to_height(quote_lines, visible_rows, cli, side_scroll, focused) {
            lines.push(line);
        }
    }
    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(panel_border_style(cli, focused))
            .style(tui_panel_style(cli))
            .title(Span::styled(title, title_style)),
    );
    frame.render_widget(panel, table_area);
    if let Some(order_area) = side_layout.order_ticket {
        draw_order_ticket_panel(frame, cli, order_area, app, detail, kind);
    }
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct OptionSideLayout {
    pub(in super::super) table: Rect,
    pub(in super::super) order_ticket: Option<Rect>,
}

pub(in super::super) fn option_side_layout(
    area: Rect,
    show_order_ticket: bool,
) -> OptionSideLayout {
    if !show_order_ticket || area.height < 12 {
        return OptionSideLayout {
            table: area,
            order_ticket: None,
        };
    }
    let ticket_height = (area.height / 2).max(8).min(12);
    let top_height = area.height.saturating_sub(ticket_height);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_height),
            Constraint::Length(ticket_height),
        ])
        .split(area);
    OptionSideLayout {
        table: rows[0],
        order_ticket: Some(rows[1]),
    }
}

pub(in super::super) fn draw_order_ticket_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    detail: &DishDetail,
    kind: OptionKind,
) {
    let focused = matches!(
        (app.trading.chain_focus, kind),
        (ChainFocus::Calls, OptionKind::Call) | (ChainFocus::Puts, OptionKind::Put)
    );
    let title = format!(
        "{} preview",
        app.trading.action.side_label().to_ascii_lowercase()
    );
    let lines = match (app.trading.ticket.as_ref(), app.selected_quote()) {
        (Some(ticket), Some(quote)) if quote.kind == kind => trade_ticket_lines(
            cli,
            app,
            detail,
            quote,
            ticket,
            area.width.saturating_sub(2),
        ),
        _ => vec![Line::from(Span::styled(
            "Select a contract, then press B or S.".to_string(),
            style(cli, Color::Yellow),
        ))],
    };

    if let (Some(ticket), Some(quote)) = (app.trading.ticket.as_ref(), app.selected_quote())
        && quote.kind == kind
        && let Some(button_area) = order_ticket_place_button_rect(area)
    {
        let block = panel_block(cli, &title, trade_action_color(app.trading.action), focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut content_lines = lines;
        remove_place_order_inline_line(&mut content_lines);
        let content_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: button_area.y.saturating_sub(inner.y),
        };
        if content_area.height > 0 {
            let panel = Paragraph::new(clip_order_ticket_content(
                content_lines,
                content_area.height as usize,
                ticket.result.is_some(),
                cli,
                focused,
            ))
            .style(tui_panel_style(cli))
            .wrap(Wrap { trim: true });
            frame.render_widget(panel, content_area);
        }

        draw_order_ticket_place_button(frame, cli, button_area, inner, app, ticket);
        return;
    }

    let panel = scrolling_panel(lines, area, cli, 0, focused).block(panel_block(
        cli,
        &title,
        trade_action_color(app.trading.action),
        focused,
    ));
    frame.render_widget(panel, area);
}

pub(in super::super) fn order_ticket_place_button_rect(area: Rect) -> Option<Rect> {
    let inner = panel_inner_rect(area)?;
    if inner.height < 6 || inner.width < 18 {
        return None;
    }
    let margin = if inner.width >= 24 { 1 } else { 0 };
    let width = inner.width.saturating_sub(margin).min(32);
    Some(Rect {
        x: inner.x.saturating_add(margin),
        y: inner.y + inner.height.saturating_sub(3),
        width,
        height: 3,
    })
}

pub(in super::super) fn draw_order_ticket_place_button(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    inner: Rect,
    app: &LabApp,
    ticket: &TradeTicket,
) {
    let (label, color, active) = trade_order_button_state(app, ticket);
    let lines = raised_button_lines(cli, &label, color, active, area.width, area.height);
    frame.render_widget(Paragraph::new(lines), area);

    let hint_x = area.x.saturating_add(area.width).saturating_add(1);
    if area.height < 2 || hint_x == 0 {
        return;
    }
    let hint_width = inner
        .width
        .saturating_add(inner.x)
        .saturating_sub(hint_x)
        .min(inner.width);
    if hint_width == 0 {
        return;
    }
    let hint = format!(
        "{} | Tab contracts/price | Esc cancel",
        trade_ticket_next_label(app, ticket)
    );
    let hint_area = Rect {
        x: hint_x,
        y: area.y + 1,
        width: hint_width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            fit_text_to_width(&hint, hint_width as usize),
            style(cli, Color::DarkGray),
        )))
        .style(tui_panel_style(cli)),
        hint_area,
    );
}

pub(in super::super) fn trade_order_button_state(
    app: &LabApp,
    ticket: &TradeTicket,
) -> (String, Color, bool) {
    let focused = matches!(
        app.guide.focused_control.as_deref(),
        Some("control:trade:review" | "control:trade:confirm")
    );
    if ticket.result.as_ref().is_some_and(|result| !result.ok) {
        ("CHECK QUOTE".to_string(), Color::Red, true)
    } else {
        (
            "QUOTE ONLY · NO SUBMIT".to_string(),
            if focused {
                Color::LightYellow
            } else {
                Color::Blue
            },
            focused,
        )
    }
}

pub(in super::super) fn remove_place_order_inline_line(lines: &mut Vec<Line<'static>>) {
    if let Some(index) = lines.iter().position(|line| {
        line.spans.iter().any(|span| {
            let content = span.content.as_ref();
            content.contains("PLACE ORDER")
                || content.contains("CONFIRM ORDER")
                || content.contains("CHECK ORDER")
                || content.contains("CHECK QUOTE")
                || content.contains("SUBMITTING ORDER")
                || content.contains("QUOTE ONLY")
        })
    }) {
        lines.remove(index);
    }
}

pub(in super::super) fn clip_order_ticket_content(
    mut lines: Vec<Line<'static>>,
    height: usize,
    keep_result: bool,
    cli: &Cli,
    focused: bool,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    if lines.len() <= height {
        return lines;
    }
    if height == 1 {
        return vec![hidden_lines_notice(cli, "↓", lines.len(), focused)];
    }

    let line_count = lines.len();
    let content_height = height.saturating_sub(1);
    let mut visible = if keep_result && content_height > 1 {
        let result = lines.pop().expect("result line");
        let mut visible = Vec::with_capacity(content_height);
        visible.extend(lines.into_iter().take(content_height - 1));
        visible.push(result);
        visible
    } else {
        lines.into_iter().take(content_height).collect()
    };
    let hidden = line_count.saturating_sub(visible.len());
    let direction = if keep_result { "↕" } else { "↓" };
    visible.push(hidden_lines_notice(cli, direction, hidden, focused));
    visible
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct OptionTableLayout {
    pub(in super::super) range_width: usize,
    pub(in super::super) buy_width: usize,
    pub(in super::super) sell_width: usize,
    pub(in super::super) mid_width: usize,
    pub(in super::super) depth_width: usize,
    pub(in super::super) show_mid: bool,
    pub(in super::super) show_depth: bool,
    pub(in super::super) full_sell_label: bool,
}

pub(in super::super) fn option_table_layout(
    width: u16,
    depth_content_width: usize,
) -> OptionTableLayout {
    const RANGE_WIDTH: usize = 9;
    const BUY_WIDTH: usize = 6;
    const SELL_WIDTH: usize = 6;
    const MID_WIDTH: usize = 5;
    const DEPTH_MIN_WIDTH: usize = 5;
    const REQUIRED_GAPS_AND_MARKER: usize = 4;

    let available = usize::from(width);
    let required_width = REQUIRED_GAPS_AND_MARKER + RANGE_WIDTH + BUY_WIDTH + SELL_WIDTH;
    if available < required_width {
        let cell_width = available.saturating_sub(REQUIRED_GAPS_AND_MARKER);
        let range_width = cell_width.min(RANGE_WIDTH);
        let buy_width = cell_width.saturating_sub(range_width).min(BUY_WIDTH);
        let sell_width = cell_width
            .saturating_sub(range_width)
            .saturating_sub(buy_width)
            .min(SELL_WIDTH);
        return OptionTableLayout {
            range_width,
            buy_width,
            sell_width,
            mid_width: 0,
            depth_width: 0,
            show_mid: false,
            show_depth: false,
            full_sell_label: false,
        };
    }

    let depth_content_width = depth_content_width.max(DEPTH_MIN_WIDTH);
    let remaining_after_required = available - required_width;
    let show_depth = remaining_after_required > DEPTH_MIN_WIDTH;
    if !show_depth {
        return OptionTableLayout {
            range_width: RANGE_WIDTH,
            buy_width: BUY_WIDTH,
            sell_width: SELL_WIDTH,
            mid_width: 0,
            depth_width: 0,
            show_mid: false,
            show_depth: false,
            full_sell_label: false,
        };
    }

    let depth_only_width = remaining_after_required - 1;
    let compact_with_mid_width = required_width + 1 + MID_WIDTH + 1 + depth_content_width;
    let show_mid = available >= compact_with_mid_width;
    if !show_mid {
        return OptionTableLayout {
            range_width: RANGE_WIDTH,
            buy_width: BUY_WIDTH,
            sell_width: SELL_WIDTH,
            mid_width: 0,
            depth_width: depth_only_width,
            show_mid: false,
            show_depth: true,
            full_sell_label: false,
        };
    }

    let mut layout = OptionTableLayout {
        range_width: RANGE_WIDTH,
        buy_width: BUY_WIDTH,
        sell_width: SELL_WIDTH,
        mid_width: MID_WIDTH,
        depth_width: depth_content_width,
        show_mid: true,
        show_depth: true,
        full_sell_label: false,
    };
    let mut extra = available - compact_with_mid_width;
    let range_growth = extra.min(2);
    layout.range_width += range_growth;
    extra -= range_growth;
    let buy_growth = extra.min(1);
    layout.buy_width += buy_growth;
    extra -= buy_growth;
    let sell_growth = extra.min(4);
    layout.sell_width += sell_growth;
    extra -= sell_growth;
    let mid_growth = extra.min(2);
    layout.mid_width += mid_growth;
    extra -= mid_growth;
    layout.depth_width += extra;
    layout.full_sell_label = layout.sell_width >= 10;
    layout
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in super::super) struct TradeRiskPreview {
    pub(in super::super) max_loss_per_contract: f64,
    pub(in super::super) max_gain_per_contract: f64,
    pub(in super::super) max_payout_per_contract: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in super::super) struct TradeTicketSizing {
    pub(in super::super) qty: u64,
    pub(in super::super) total_max_loss: f64,
}

pub(in super::super) fn trade_risk_preview(
    quote: &OptionQuote,
    action: TradeAction,
) -> Option<TradeRiskPreview> {
    trade_route_price(quote, action)
        .and_then(|premium| trade_risk_preview_for_price(quote, action, premium))
}

pub(in super::super) fn trade_risk_preview_for_price(
    quote: &OptionQuote,
    action: TradeAction,
    premium: f64,
) -> Option<TradeRiskPreview> {
    let lower = parse_contract_number(&quote.lower_strike)?;
    let upper = parse_contract_number(&quote.upper_strike)?;
    let width = upper - lower;
    if !width.is_finite() || width <= 0.0 || !premium.is_finite() || premium <= 0.0 {
        return None;
    }

    let buyer_max_loss = premium;
    let buyer_max_gain = (width - premium).max(0.0);
    let (max_loss_per_contract, max_gain_per_contract) = match action {
        TradeAction::Buy => (buyer_max_loss, buyer_max_gain),
        // An owned-long sale releases inventory; it never creates short exposure.
        TradeAction::Sell => (0.0, buyer_max_loss),
    };

    Some(TradeRiskPreview {
        max_loss_per_contract,
        max_gain_per_contract,
        max_payout_per_contract: width,
    })
}

pub(in super::super) fn trade_ticket_sizing(
    ticket: &TradeTicket,
    risk: TradeRiskPreview,
) -> Result<TradeTicketSizing, String> {
    if !risk.max_loss_per_contract.is_finite() || risk.max_loss_per_contract < 0.0 {
        return Err("Could not calculate risk for this ticket.".to_string());
    }
    let qty = ticket
        .quantity_input
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|qty| *qty > 0)
        .ok_or_else(|| "Enter a whole contract quantity greater than 0.".to_string())?;
    let total_max_loss = risk.max_loss_per_contract * qty as f64;
    Ok(TradeTicketSizing {
        qty,
        total_max_loss,
    })
}

pub(in super::super) fn trade_ticket_validation_field(message: &str) -> Option<TradeTicketField> {
    let message = message.to_ascii_lowercase();
    if message.contains("contract quantity") {
        Some(TradeTicketField::Quantity)
    } else if message.starts_with("enter a price")
        || message.starts_with("enter a premium")
        || message.starts_with("cap width:")
        || message.contains("must be below")
        || message.contains("calculate risk for this ticket")
    {
        Some(TradeTicketField::Premium)
    } else {
        None
    }
}

pub(in super::super) fn build_trade_ticket_submit(
    cli: &Cli,
    app: &LabApp,
) -> Result<TradeTicketSubmit, String> {
    let Some(ticket) = &app.trading.ticket else {
        return Err("Open an order ticket first.".to_string());
    };
    let Some(detail) = &app.trading.detail else {
        return Err("Load a market before placing an order.".to_string());
    };
    let Some(quote) = app.selected_quote() else {
        return Err("Select a contract before placing an order.".to_string());
    };
    app.require_selected_contract_tradeable()?;
    let Some(owner) = app.wallet.pubkey.as_deref() else {
        return Err("Attach a wallet before placing an order.".to_string());
    };
    let premium = parse_contract_number(&ticket.premium_input)
        .filter(|value| *value > 0.0)
        .ok_or_else(|| {
            format!(
                "Enter a {} greater than 0.",
                ticket.action.price_label().to_ascii_lowercase()
            )
        })?;
    let lower_value = parse_contract_number(&quote.lower_strike).unwrap_or(0.0);
    let upper_value = parse_contract_number(&quote.upper_strike).unwrap_or(0.0);
    let width = upper_value - lower_value;
    if !width.is_finite() || width <= 0.0 {
        return Err("Selected contract has an invalid strike width.".to_string());
    }
    if premium >= width {
        let width_label = format_decimal(width, 3);
        return Err(format!(
            "Cap width: max ${width_label} each; {} - {} = {width_label}. Enter {} below ${width_label}.",
            quote.upper_strike,
            quote.lower_strike,
            ticket.action.price_label().to_ascii_lowercase(),
        ));
    }
    let risk = trade_risk_preview_for_price(quote, ticket.action, premium)
        .ok_or_else(|| "Could not calculate risk for this ticket.".to_string())?;
    let sizing = trade_ticket_sizing(ticket, risk)?;
    let mut args = Vec::new();
    let mut envs = Vec::new();
    push_child_global_args(&mut args, &mut envs, cli, app, owner);
    args.extend([
        "--json".to_string(),
        "trades".to_string(),
        "quote".to_string(),
        "--market".to_string(),
        detail.id.clone(),
        "--expiry".to_string(),
        app.selected_chart_expiry()
            .ok_or_else(|| "Select an exact series.".to_string())?
            .id
            .clone(),
        "--side".to_string(),
        match ticket.action {
            TradeAction::Buy => "buy".to_string(),
            TradeAction::Sell => "sell".to_string(),
        },
        "--quantity".to_string(),
        ticket.quantity_input.clone(),
        "--limit-price".to_string(),
        ticket.premium_input.clone(),
    ]);
    let command = display_command(&args);
    Ok(TradeTicketSubmit {
        args,
        envs,
        command,
        summary: TradeConfirmationSummary {
            action: ticket.action,
            symbol: detail.symbol.clone(),
            expiry: app
                .selected_chart_expiry()
                .map(|e| e.id.clone())
                .unwrap_or_default(),
            kind: quote.kind,
            lower_strike: quote.lower_strike.clone(),
            upper_strike: quote.upper_strike.clone(),
            price: premium,
            qty: sizing.qty,
            entry_total: premium * sizing.qty as f64,
            total_max_loss: sizing.total_max_loss,
            total_max_gain: risk.max_gain_per_contract * sizing.qty as f64,
            total_max_payout: risk.max_payout_per_contract * sizing.qty as f64,
            probability_itm: quote.probability_itm,
            probability_cap_hit: quote.probability_cap_hit,
            account: short_pubkey(owner),
        },
    })
}

pub(in super::super) fn push_child_global_args(
    args: &mut Vec<String>,
    envs: &mut Vec<(String, String)>,
    cli: &Cli,
    app: &LabApp,
    _owner: &str,
) {
    args.extend(["--backend-url".to_string(), cli.backend_url.clone()]);
    args.extend(["--cluster".to_string(), cli.cluster.clone()]);
    push_optional_child_env(envs, "SOLANA_CONFIG", cli.solana_config.as_deref());
    push_optional_child_env(envs, "SOLANA_COMMITMENT", cli.commitment.as_deref());
    if app.wallet.is_attached() {
        envs.push((
            "SOLANA_KEYPAIR".to_string(),
            app.wallet.keypair_path.clone(),
        ));
    }
    if cli.allow_insecure_keypair {
        args.push("--allow-insecure-keypair".to_string());
    }
    args.push("--no-color".to_string());
}

pub(in super::super) fn push_optional_child_env(
    envs: &mut Vec<(String, String)>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        envs.push((key.to_string(), value.to_string()));
    }
}

pub(in super::super) fn display_command(args: &[String]) -> String {
    std::iter::once("petri".to_string())
        .chain(args.iter().map(|word| redact_command_word(word)))
        .map(|word| shell_word(&word))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(in super::super) fn redact_command_word(word: &str) -> String {
    let mut redacted = word.to_string();
    for marker in ["api-key=", "apikey=", "token=", "access_token=", "key="] {
        if let Some(index) = redacted.to_ascii_lowercase().find(marker) {
            let value_start = index + marker.len();
            let value_end = redacted[value_start..]
                .find(['&', ' '])
                .map(|offset| value_start + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(value_start..value_end, "<redacted>");
        }
    }
    redacted
}

pub(in super::super) fn shell_word(word: &str) -> String {
    if word.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '=' | '<' | '>')
    }) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

pub(in super::super) fn trade_route_price(quote: &OptionQuote, action: TradeAction) -> Option<f64> {
    match action {
        TradeAction::Buy => positive_quote(quote.ask),
        TradeAction::Sell => positive_quote(quote.bid),
    }
}

pub(in super::super) fn trade_risk_unavailable_label(
    quote: &OptionQuote,
    action: TradeAction,
) -> &'static str {
    let lower = parse_contract_number(&quote.lower_strike);
    let upper = parse_contract_number(&quote.upper_strike);
    if lower.zip(upper).is_none_or(|(lower, upper)| upper <= lower) {
        return "needs strikes";
    }
    match action {
        TradeAction::Buy => "needs ask",
        TradeAction::Sell => "enter price",
    }
}

pub(in super::super) fn trade_plan_command(
    _detail: &DishDetail,
    _quote: &OptionQuote,
    action: TradeAction,
    _tick: &str,
) -> String {
    format!(
        "petri trades prepare --market <current-market-pubkey> --direction {} --amount-in <exact-input-atoms> --minimum-amount-out <minimum-output-atoms> --limit-bin-id <limit-bin-id>",
        match action {
            TradeAction::Buy => "quote-for-option",
            TradeAction::Sell => "option-for-quote",
        },
    )
}

pub(in super::super) fn parse_contract_number(value: &str) -> Option<f64> {
    value
        .trim()
        .replace(',', "")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

pub(in super::super) fn option_side_header(
    cli: &Cli,
    layout: OptionTableLayout,
) -> Vec<Line<'static>> {
    let sell_label = if layout.full_sell_label { "ASK" } else { "ASK" };
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            option_cell("RANGE", layout.range_width),
            style(cli, Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(
            option_cell("BID", layout.buy_width),
            style(cli, Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            option_cell(sell_label, layout.sell_width),
            style(cli, Color::Red),
        ),
    ];

    if layout.show_mid {
        spans.extend([
            Span::raw(" "),
            Span::styled(
                option_cell("MID", layout.mid_width),
                style(cli, Color::Yellow),
            ),
        ]);
    }
    if layout.show_depth {
        spans.extend([
            Span::raw(" "),
            Span::styled(
                option_cell("DEPTH", layout.depth_width),
                style(cli, Color::Blue),
            ),
        ]);
    }

    vec![Line::from(spans)]
}

pub(in super::super) fn option_side_row(
    cli: &Cli,
    quote: &OptionQuote,
    selected: bool,
    width: u16,
    layout: OptionTableLayout,
    shaded: bool,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let row_bg = option_row_background(cli, shaded);
    let mut visible_width = 0usize;
    let mut spans = vec![
        option_span(
            marker.to_string(),
            cell_style(cli, Color::Yellow, selected),
            row_bg,
            &mut visible_width,
        ),
        option_gap_span(row_bg, &mut visible_width),
        option_span(
            option_cell(
                &format!("{}/{}", quote.lower_strike, quote.upper_strike),
                layout.range_width,
            ),
            cell_style(cli, Color::White, selected),
            row_bg,
            &mut visible_width,
        ),
        option_gap_span(row_bg, &mut visible_width),
        option_span(
            option_cell(
                &format_optional_decimal(quote.bid, if layout.show_mid { 3 } else { 2 }),
                layout.buy_width,
            ),
            optional_value_style(cli, quote.bid, Color::Green, selected),
            row_bg,
            &mut visible_width,
        ),
        option_gap_span(row_bg, &mut visible_width),
        option_span(
            option_cell(
                &format_optional_decimal(quote.ask, if layout.show_mid { 3 } else { 2 }),
                layout.sell_width,
            ),
            optional_value_style(cli, quote.ask, Color::Red, selected),
            row_bg,
            &mut visible_width,
        ),
    ];

    if layout.show_mid {
        spans.extend([
            option_gap_span(row_bg, &mut visible_width),
            option_span(
                option_cell(&format_optional_decimal(quote.mid, 3), layout.mid_width),
                optional_value_style(cli, quote.mid, Color::Yellow, selected),
                row_bg,
                &mut visible_width,
            ),
        ]);
    }
    if layout.show_depth {
        spans.extend([
            option_gap_span(row_bg, &mut visible_width),
            option_span(
                option_cell(&compact_depth_label(quote), layout.depth_width),
                depth_style(cli, quote, selected),
                row_bg,
                &mut visible_width,
            ),
        ]);
    }
    let target_width = usize::from(width);
    if let Some(bg) = row_bg
        && visible_width < target_width
    {
        spans.push(Span::styled(
            " ".repeat(target_width - visible_width),
            Style::default().bg(bg),
        ));
    }

    Line::from(spans)
}

pub(in super::super) fn option_span(
    content: String,
    style: Style,
    row_bg: Option<Color>,
    visible_width: &mut usize,
) -> Span<'static> {
    *visible_width += content.chars().count();
    Span::styled(content, option_row_style(style, row_bg))
}

pub(in super::super) fn option_gap_span(
    row_bg: Option<Color>,
    visible_width: &mut usize,
) -> Span<'static> {
    option_span(" ".to_string(), Style::default(), row_bg, visible_width)
}

pub(in super::super) fn option_row_style(style: Style, row_bg: Option<Color>) -> Style {
    if let Some(bg) = row_bg {
        style.bg(bg)
    } else {
        style
    }
}

pub(in super::super) fn option_row_background(cli: &Cli, shaded: bool) -> Option<Color> {
    if cli.no_color || !shaded {
        None
    } else {
        Some(option_chain_row_color(shaded))
    }
}

pub(in super::super) fn option_chain_row_color(shaded: bool) -> Color {
    if shaded {
        TUI_PANEL_BACKGROUND_ALT
    } else {
        TUI_PANEL_BACKGROUND
    }
}

pub(in super::super) fn option_cell(value: &str, width: usize) -> String {
    let value = truncate_cell(value.trim(), width);
    format!("{value:<width$}")
}

pub(in super::super) fn truncate_cell(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let mut out = String::new();
    for c in chars.by_ref().take(width) {
        out.push(c);
    }
    if width > 3 && chars.next().is_some() {
        for _ in 0..3 {
            out.pop();
        }
        out.push_str("...");
    }
    out
}

pub(in super::super) fn trade_panel_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                "BUY (B)",
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            Span::styled(
                "SELL (S)",
                style(cli, Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("CLI: ", style(cli, Color::DarkGray)),
            Span::styled(
                selected_contract_preview_command(app),
                style(cli, Color::Cyan),
            ),
        ]),
    ]
}

pub(in super::super) fn trade_ticket_lines(
    cli: &Cli,
    app: &LabApp,
    detail: &DishDetail,
    quote: &OptionQuote,
    ticket: &TradeTicket,
    width: u16,
) -> Vec<Line<'static>> {
    let submitting = app.trading.submit_is_running();
    let strategy_label = match quote.kind {
        OptionKind::Call => "call spread",
        OptionKind::Put => "put spread",
    };
    let premium = parse_contract_number(&ticket.premium_input).filter(|value| *value > 0.0);
    let cap_width = parse_contract_number(&quote.lower_strike)
        .zip(parse_contract_number(&quote.upper_strike))
        .map(|(lower, upper)| upper - lower)
        .filter(|width| width.is_finite() && *width > 0.0);
    let compact_cap_error = premium.zip(cap_width).and_then(|(price, width)| {
        (price >= width).then(|| format!("Cap width: max ${} each", format_decimal(width, 3)))
    });
    let risk = premium.and_then(|price| trade_risk_preview_for_price(quote, ticket.action, price));
    let price_word = ticket.action.price_label().to_ascii_lowercase();
    let enter_price_label = format!("enter {price_word}");
    let sizing = risk.and_then(|risk| trade_ticket_sizing(ticket, risk).ok());
    let actual_max_loss_label = match (risk, sizing) {
        (Some(_), Some(sizing)) => format_usd(sizing.total_max_loss),
        (Some(_), None) => "enter contracts".to_string(),
        (None, _) => enter_price_label.clone(),
    };
    let actual_max_loss_color = if risk.is_some() && sizing.is_none() {
        Color::Red
    } else {
        Color::Green
    };
    let max_gain_label = match (risk, sizing) {
        (Some(risk), Some(sizing)) => format_usd(risk.max_gain_per_contract * sizing.qty as f64),
        (Some(_), None) => "enter contracts".to_string(),
        (None, _) => enter_price_label.clone(),
    };
    let range_or_collateral_label = match (risk, sizing) {
        _ if ticket.action == TradeAction::Sell => "none".to_string(),
        (Some(risk), Some(sizing)) => format_usd(risk.max_payout_per_contract * sizing.qty as f64),
        (Some(_), None) => "enter contracts".to_string(),
        (None, _) => enter_price_label.clone(),
    };
    let entry_total_label = match (premium, sizing) {
        (Some(premium), Some(sizing)) => format_usd(premium * sizing.qty as f64),
        (Some(_), None) => "enter contracts".to_string(),
        (None, _) => enter_price_label.clone(),
    };
    let command = match build_trade_ticket_submit(cli, app) {
        Ok(submit) => submit.command,
        Err(message) => format!("Before review: {message}"),
    };
    let next_label = trade_ticket_next_label(app, ticket);
    let (button_label, button_color, button_active) = trade_order_button_state(app, ticket);

    let compact = width < ORDER_TICKET_COMPACT_WIDTH;
    let price_line = || {
        Line::from(vec![
            Span::styled(
                format!("{} ", ticket.action.price_label()),
                style(cli, Color::DarkGray),
            ),
            ticket_input_span(
                cli,
                &ticket.premium_input,
                "type price",
                ticket.field == TradeTicketField::Premium,
                submitting,
                app.trading
                    .ticket_field_flash_visible(TradeTicketField::Premium),
                None,
            ),
        ])
    };
    let quantity_line = || {
        Line::from(vec![
            Span::styled("Contracts ", style(cli, Color::DarkGray)),
            ticket_input_span(
                cli,
                &ticket.quantity_input,
                "quantity",
                ticket.field == TradeTicketField::Quantity,
                submitting,
                app.trading
                    .ticket_field_flash_visible(TradeTicketField::Quantity),
                Some(Color::Red),
            ),
        ])
    };
    let compact_risk_line = || {
        if let Some(label) = &compact_cap_error {
            return Line::from(Span::styled(
                label.clone(),
                style(cli, Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        Line::from(vec![
            Span::styled(
                if ticket.action == TradeAction::Buy {
                    "Est. debit "
                } else {
                    "New exposure "
                },
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                actual_max_loss_label.clone(),
                style(cli, actual_max_loss_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if ticket.action == TradeAction::Buy {
                    " | Est. max gain "
                } else {
                    " | Est. proceeds "
                },
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                max_gain_label.clone(),
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    let payout_line = || {
        Line::from(vec![
            Span::styled(
                match ticket.action {
                    TradeAction::Buy => "Gross payout ",
                    TradeAction::Sell => "New collateral ",
                }
                .to_string(),
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                range_or_collateral_label.clone(),
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                match ticket.action {
                    TradeAction::Buy => " | Debit ",
                    TradeAction::Sell => " | Credit ",
                }
                .to_string(),
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                entry_total_label.clone(),
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            no_liquidation_span(cli),
        ])
    };

    let mut lines = if compact {
        vec![
            price_line(),
            quantity_line(),
            compact_risk_line(),
            payout_line(),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(
                    format!("{} ", ticket.action.label()),
                    trade_action_style(cli, ticket.action).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} {} {}-{} / {} ",
                        detail.symbol,
                        strategy_label,
                        quote.lower_strike,
                        quote.upper_strike,
                        detail.expiry_label
                    ),
                    style(cli, option_kind_color(quote.kind)).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "| {} ticket",
                        ticket.action.side_label().to_ascii_lowercase()
                    ),
                    style(cli, Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{} ", ticket.action.price_label()),
                    style(cli, Color::DarkGray),
                ),
                ticket_input_span(
                    cli,
                    &ticket.premium_input,
                    "type price",
                    ticket.field == TradeTicketField::Premium,
                    submitting,
                    app.trading
                        .ticket_field_flash_visible(TradeTicketField::Premium),
                    None,
                ),
                Span::styled(" | Contracts ", style(cli, Color::DarkGray)),
                ticket_input_span(
                    cli,
                    &ticket.quantity_input,
                    "quantity",
                    ticket.field == TradeTicketField::Quantity,
                    submitting,
                    app.trading
                        .ticket_field_flash_visible(TradeTicketField::Quantity),
                    Some(Color::Red),
                ),
                Span::styled(
                    if ticket.action == TradeAction::Buy {
                        " | Est. debit "
                    } else {
                        " | New exposure "
                    },
                    style(cli, Color::DarkGray),
                ),
                Span::styled(
                    actual_max_loss_label.clone(),
                    style(cli, actual_max_loss_color).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    if ticket.action == TradeAction::Buy {
                        "Est. max gain "
                    } else {
                        "Est. proceeds "
                    },
                    style(cli, Color::DarkGray),
                ),
                Span::styled(
                    max_gain_label.clone(),
                    style(cli, Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    match ticket.action {
                        TradeAction::Buy => " | Gross payout ",
                        TradeAction::Sell => " | New collateral ",
                    }
                    .to_string(),
                    style(cli, Color::DarkGray),
                ),
                Span::styled(
                    range_or_collateral_label.clone(),
                    style(cli, Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    match ticket.action {
                        TradeAction::Buy => " | Debit ",
                        TradeAction::Sell => " | Credit ",
                    }
                    .to_string(),
                    style(cli, Color::DarkGray),
                ),
                Span::styled(
                    entry_total_label.clone(),
                    style(cli, Color::Green).add_modifier(Modifier::BOLD),
                ),
                divider_span(cli),
                no_liquidation_span(cli),
            ]),
        ]
    };
    lines.extend([
        Line::from(vec![
            Span::styled(
                format!(" {button_label} "),
                terminal_button_surface_style(cli, button_color, button_active),
            ),
            Span::styled(
                format!(" {next_label} | Tab contracts/price | Esc cancel"),
                style(cli, Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Quote command: ", style(cli, Color::DarkGray)),
            Span::styled(command, style(cli, Color::Cyan)),
        ]),
    ]);

    if let Some(result) = &ticket.result {
        let color = if result.ok { Color::Green } else { Color::Red };
        let label = if result.ok {
            "Quote only: "
        } else if result.message.starts_with("Cap width:") {
            ""
        } else {
            "Error: "
        };
        lines.push(Line::from(vec![
            Span::styled(
                label.to_string(),
                style(cli, color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(result.message.clone(), style(cli, Color::White)),
        ]));
    }

    lines
}

pub(in super::super) fn trade_ticket_next_label(
    app: &LabApp,
    ticket: &TradeTicket,
) -> &'static str {
    if app.trading.submit_is_running() {
        "Submitting..."
    } else if ticket.confirmation.is_some() {
        "Confirm in popup"
    } else {
        "Enter quote"
    }
}

pub(in super::super) fn ticket_input_span(
    cli: &Cli,
    value: &str,
    placeholder: &str,
    active: bool,
    disabled: bool,
    flashing: bool,
    empty_color: Option<Color>,
) -> Span<'static> {
    let mut text = if value.trim().is_empty() {
        placeholder.to_string()
    } else {
        value.to_string()
    };
    if active && !disabled {
        text.push('_');
    }
    let color = if disabled {
        Color::DarkGray
    } else if value.trim().is_empty() {
        empty_color.unwrap_or(if active {
            Color::Yellow
        } else {
            Color::DarkGray
        })
    } else if active {
        Color::Yellow
    } else {
        Color::White
    };
    if cli.no_color {
        let mut style = style(cli, color).add_modifier(Modifier::BOLD);
        if flashing && !disabled {
            style = style.add_modifier(Modifier::REVERSED);
        }
        return Span::styled(format!("[{text}]"), style);
    }
    let bg = if flashing && !disabled {
        TUI_FIELD_FLASH_BACKGROUND
    } else if active && !disabled {
        TUI_FIELD_ACTIVE_BACKGROUND
    } else {
        TUI_FIELD_BACKGROUND
    };
    let fg = if flashing && !disabled {
        Color::Black
    } else {
        color
    };
    let style = style(cli, fg).add_modifier(Modifier::BOLD).bg(bg);
    Span::styled(format!(" {text} "), style)
}

pub(in super::super) fn ticket_input_display_width(
    _cli: &Cli,
    value: &str,
    placeholder: &str,
    active: bool,
    disabled: bool,
) -> u16 {
    let mut text = if value.trim().is_empty() {
        placeholder.to_string()
    } else {
        value.to_string()
    };
    if active && !disabled {
        text.push('_');
    }
    text.chars().count().saturating_add(2) as u16
}
