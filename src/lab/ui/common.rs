//! Shared terminal formatting, panel overflow, footer help, and styles.

use super::super::*;

/// Existing bordered text-panel overflow and wrapping, with geometry, content,
/// focus and scroll supplied by the screen. Block styling stays at the call site.
pub(in super::super) fn scrolling_panel(
    lines: Vec<Line<'static>>,
    area: Rect,
    cli: &Cli,
    scroll: usize,
    focused: bool,
) -> Paragraph<'static> {
    Paragraph::new(scroll_lines_to_panel(lines, area, cli, scroll, focused))
        .wrap(Wrap { trim: true })
}

pub(in super::super) fn short_pubkey(value: &str) -> String {
    if value.len() <= 16 {
        return value.to_string();
    }
    format!("{}...{}", &value[..8], &value[value.len() - 6..])
}

pub(in super::super) fn short_path(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars || max_chars < 12 {
        return value.to_string();
    }
    let head_len = (max_chars - 3) / 2;
    let tail_len = max_chars - 3 - head_len;
    let head = value.chars().take(head_len).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}...{tail}")
}

pub(in super::super) fn docs_url() -> String {
    env::var("AMEBA_DOCS_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_DOCS_URL.to_string())
}

pub(in super::super) fn normalize_keypair_path_input(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let quoted = (bytes.first() == Some(&b'"') && bytes.last() == Some(&b'"'))
            || (bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\''));
        if quoted {
            return trimmed[1..trimmed.len() - 1].trim().to_string();
        }
    }
    trimmed.to_string()
}

pub(in super::super) fn quote_indices_by_kind(detail: &DishDetail, kind: OptionKind) -> Vec<usize> {
    detail
        .option_quotes
        .iter()
        .enumerate()
        .filter_map(|(index, quote)| (quote.kind == kind).then_some(index))
        .collect()
}

pub(in super::super) fn quote_rank_by_kind(
    detail: &DishDetail,
    selected: usize,
    kind: OptionKind,
) -> Option<usize> {
    quote_indices_by_kind(detail, kind)
        .iter()
        .position(|index| *index == selected)
}

pub(in super::super) fn compact_depth_label(quote: &OptionQuote) -> String {
    quote
        .depth_usd
        .filter(|value| *value > 0.0)
        .map(format_usd)
        .unwrap_or_else(|| quote.status.clone())
}

pub(in super::super) fn selected_contract_label(app: &LabApp) -> String {
    match (&app.trading.detail, app.selected_quote()) {
        (Some(detail), Some(quote)) => format!(
            "{} {} {}-{} / {}",
            detail.symbol,
            quote.kind.label(),
            quote.lower_strike,
            quote.upper_strike,
            detail.expiry_label
        ),
        (Some(detail), None) => format!("{} selected contract", detail.symbol),
        _ => "selected contract".to_string(),
    }
}

pub(in super::super) fn short_freshness_label(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("live") {
        "live".to_string()
    } else if lower.contains("updating")
        || lower.contains("cached")
        || lower.contains("stale")
        || lower.contains("degraded")
    {
        "last known".to_string()
    } else if lower.contains("unavailable") {
        "not available".to_string()
    } else {
        value.to_string()
    }
}

pub(in super::super) fn short_settlement_label(value: &str) -> String {
    value
        .split_once('T')
        .map(|(date, _)| date.to_string())
        .unwrap_or_else(|| value.to_string())
}

pub(in super::super) fn time_to_settle_label(days: &str) -> String {
    let trimmed = days.trim();
    if trimmed.is_empty() || trimmed == "-" {
        "-".to_string()
    } else if trimmed == "1" {
        "1 day".to_string()
    } else if trimmed.to_ascii_lowercase().contains("day") {
        trimmed.to_string()
    } else {
        format!("{trimmed} days")
    }
}

pub(in super::super) fn max_quote_volume(detail: &DishDetail) -> Option<f64> {
    max_quote_value(
        detail
            .option_quotes
            .iter()
            .filter_map(|quote| quote.volume.or(quote.open_interest)),
    )
}

pub(in super::super) fn max_quote_value(values: impl Iterator<Item = f64>) -> Option<f64> {
    values
        .filter(|value| value.is_finite() && *value > 0.0)
        .fold(None, |max: Option<f64>, value| {
            Some(max.map(|current| current.max(value)).unwrap_or(value))
        })
}

pub(in super::super) fn bar_for(value: Option<f64>, max: Option<f64>, width: usize) -> String {
    let filled = match (value, max) {
        (Some(value), Some(max))
            if value.is_finite() && max.is_finite() && value > 0.0 && max > 0.0 =>
        {
            ((value / max).clamp(0.0, 1.0) * width as f64).round() as usize
        }
        _ => 0,
    };

    let mut bar = String::with_capacity(width + 2);
    bar.push('[');
    for cell in 0..width {
        bar.push(if cell < filled { '#' } else { '-' });
    }
    bar.push(']');
    bar
}

pub(in super::super) fn loading_spinner(tick: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    FRAMES[tick % FRAMES.len()]
}

pub(in super::super) fn format_optional_decimal(value: Option<f64>, decimals: usize) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format_decimal(value, decimals))
        .unwrap_or_else(|| "n/a".to_string())
}

pub(in super::super) fn format_optional_usd(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(format_usd)
        .unwrap_or_else(|| "n/a".to_string())
}

pub(in super::super) fn style(cli: &Cli, color: Color) -> Style {
    if cli.no_color {
        Style::default()
    } else {
        Style::default().fg(readable_tui_color(color))
    }
}

pub(in super::super) fn divider_span(cli: &Cli) -> Span<'static> {
    Span::styled(" | ", style(cli, Color::DarkGray))
}

pub(in super::super) fn no_liquidation_span(cli: &Cli) -> Span<'static> {
    Span::styled(
        "no liquidation",
        style(cli, Color::Green).add_modifier(Modifier::BOLD),
    )
}

pub(in super::super) fn readable_tui_color(color: Color) -> Color {
    match color {
        Color::Black | Color::DarkGray => Color::Gray,
        Color::Gray => Color::White,
        Color::Red => Color::LightRed,
        Color::Green => Color::LightGreen,
        Color::Yellow => Color::LightYellow,
        Color::Blue | Color::LightBlue => Color::White,
        Color::Magenta => Color::LightMagenta,
        Color::Cyan => Color::LightCyan,
        other => other,
    }
}

pub(in super::super) fn cell_style(cli: &Cli, color: Color, selected: bool) -> Style {
    if selected && !cli.no_color {
        Style::default()
            .fg(readable_tui_color(color))
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED)
    } else if selected {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        style(cli, color)
    }
}

pub(in super::super) fn oracle_form_field_style(
    cli: &Cli,
    color: Color,
    selected: bool,
    flashing: bool,
) -> Style {
    if flashing && !cli.no_color {
        Style::default()
            .fg(Color::Black)
            .bg(TUI_FIELD_FLASH_BACKGROUND)
            .add_modifier(Modifier::BOLD)
    } else if flashing {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED)
    } else {
        cell_style(cli, color, selected)
    }
}

pub(in super::super) fn oracle_emergency_style(cli: &Cli, app: &LabApp) -> Style {
    let flash_on = app.spinner_tick % 2 == 0;
    if cli.no_color {
        let mut style = Style::default().add_modifier(Modifier::BOLD);
        if flash_on {
            style = style.add_modifier(Modifier::REVERSED);
        }
        return style;
    }
    if flash_on {
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    }
}

pub(in super::super) fn oracle_task_style(
    cli: &Cli,
    color: Color,
    selected: bool,
    flashing: bool,
) -> Style {
    if flashing && !cli.no_color {
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else if flashing {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED)
    } else {
        cell_style(cli, color, selected)
    }
}

pub(in super::super) fn optional_value_style(
    cli: &Cli,
    value: Option<f64>,
    color: Color,
    selected: bool,
) -> Style {
    let color = if value
        .filter(|value| value.is_finite() && *value > 0.0)
        .is_some()
    {
        color
    } else {
        Color::DarkGray
    };
    cell_style(cli, color, selected)
}

pub(in super::super) fn depth_style(cli: &Cli, quote: &OptionQuote, selected: bool) -> Style {
    let color = if quote
        .depth_usd
        .filter(|value| value.is_finite() && *value > 0.0)
        .is_some()
    {
        Color::Blue
    } else if quote.status.eq_ignore_ascii_case("active")
        || quote.status.eq_ignore_ascii_case("assigned")
        || quote.status.eq_ignore_ascii_case("ready")
    {
        Color::Green
    } else if quote.status.to_ascii_lowercase().contains("queued")
        || quote.status.to_ascii_lowercase().contains("pending")
    {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    cell_style(cli, color, selected)
}

pub(in super::super) fn status_style(cli: &Cli, text: &str) -> Style {
    let lower = text.to_ascii_lowercase();
    let color = if lower.contains("unavailable")
        || lower.contains("not available")
        || lower.contains("not ready")
    {
        Color::Red
    } else if lower.contains("ready") || lower.contains("updated") {
        Color::Green
    } else if lower.contains("live") || lower.contains("available") {
        Color::Green
    } else if lower.contains("updating")
        || lower.contains("loading")
        || lower.contains("opening")
        || lower.contains("refreshing")
        || lower.contains("cached")
        || lower.contains("stale")
        || lower.contains("degraded")
    {
        Color::Yellow
    } else {
        Color::White
    };
    style(cli, color)
}

pub(in super::super) fn trade_action_style(cli: &Cli, action: TradeAction) -> Style {
    style(cli, trade_action_color(action))
}

pub(in super::super) fn trade_action_color(action: TradeAction) -> Color {
    match action {
        TradeAction::Buy => Color::Green,
        TradeAction::Sell => Color::Red,
    }
}

pub(in super::super) fn option_kind_color(kind: OptionKind) -> Color {
    match kind {
        OptionKind::Call => Color::Cyan,
        OptionKind::Put => Color::Magenta,
    }
}

pub(in super::super) fn home_action_color(action: HomeAction) -> Color {
    let index = HomeAction::ALL
        .iter()
        .position(|candidate| *candidate == action)
        .unwrap_or_default();
    option_chain_row_color(index % 2 == 1)
}

pub(in super::super) fn home_action_accent_color(action: HomeAction) -> Color {
    match action {
        HomeAction::Trade | HomeAction::Ledger => Color::Green,
        HomeAction::Staking => Color::Yellow,
        HomeAction::Chart => Color::Blue,
        HomeAction::Oracle | HomeAction::ConnectAgents => Color::Magenta,
        HomeAction::Help => Color::Cyan,
    }
}

pub(in super::super) fn focused_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    selected_area: Rect,
    market_area: Option<Rect>,
    activity_area: Rect,
) -> Option<usize> {
    if app.screen == LabScreen::Home
        && let Some(hidden) = home_missing_lines_below(cli, app, selected_area)
    {
        return Some(hidden);
    }
    if matches!(app.focus, LabFocus::Markets | LabFocus::MarketSeries) {
        let area = market_area?;
        let lines = dish_list_lines(cli, app);
        let panel_area = wrapped_content_sized_panel_area(area, &lines, true);
        let scroll_focus = if app.focus == LabFocus::MarketSeries {
            LabFocus::MarketSeries
        } else {
            LabFocus::Markets
        };
        return wrapped_missing_lines_for_panel(
            &lines,
            panel_area,
            app.focused_panel_scroll(scroll_focus),
            true,
        );
    }

    if app.screen == LabScreen::Chain && app.focus == LabFocus::Activity && activity_area.height > 0
    {
        return missing_lines_for_panel(
            trade_panel_lines(cli, app).len(),
            activity_area,
            app.focused_panel_scroll(LabFocus::Activity),
        );
    }

    match app.screen {
        LabScreen::Terms => missing_lines_for_panel(
            wallet_terms_lines(cli, app).len(),
            selected_area,
            app.focused_panel_scroll(LabFocus::Terms),
        ),
        LabScreen::Home => home_missing_lines_below(cli, app, selected_area),
        LabScreen::Staking => staking_missing_lines_below(cli, app, selected_area),
        LabScreen::Chain => chain_missing_lines_below(cli, app, selected_area),
        LabScreen::Chart => chart_missing_lines_below(app, selected_area),
        LabScreen::OracleIntro => missing_lines_for_panel(
            oracle_intro_lines(cli, app, app.focus == LabFocus::OracleIntro).len(),
            selected_area,
            app.focused_panel_scroll(LabFocus::OracleIntro),
        ),
        LabScreen::Oracle => {
            if app.oracle.view == OracleView::Earn {
                let layout = oracle_earn_layout(selected_area);
                Some(usize::from(4u16.saturating_sub(layout.table_area.height)))
            } else {
                oracle_missing_lines_below(cli, app, selected_area)
            }
        }
        LabScreen::OracleHelp => {
            let layout = oracle_help_layout(selected_area);
            missing_lines_for_panel(
                oracle_help_lines(cli, app).len(),
                layout.help_area,
                app.focused_panel_scroll(LabFocus::OracleHelp),
            )
        }
        LabScreen::Help => {
            if app.home_help_topic == HomeHelpTopic::Agents {
                let help_area =
                    agent_connection_text_panel(agent_connection_layout(selected_area).help_area);
                wrapped_missing_lines_for_panel(
                    &home_help_agent_lines_for_width(cli, app, help_area.width.saturating_sub(2)),
                    help_area,
                    app.focused_panel_scroll(LabFocus::Help),
                    true,
                )
            } else {
                let layout = gitbook_help_layout(selected_area);
                match app.help.pane {
                    HelpPane::Navigation => {
                        let line_count = gitbook_navigation_lines(cli, app).len();
                        let hidden =
                            line_count.saturating_sub(panel_inner_height(layout.navigation_area));
                        (hidden > 0).then_some(hidden)
                    }
                    HelpPane::Article => {
                        let article_width = panel_inner_rect(layout.article_area)
                            .map(|inner| inner.width as usize)
                            .unwrap_or_default();
                        missing_lines_for_panel(
                            gitbook_article_lines(cli, app, article_width).len(),
                            layout.article_area,
                            app.help.article_scroll,
                        )
                    }
                }
            }
        }
        LabScreen::Detail => missing_lines_for_panel(
            detail_lines(app).len(),
            selected_area,
            app.focused_panel_scroll(LabFocus::Detail),
        ),
        LabScreen::Activity => missing_lines_for_panel(
            activity_lines(cli, app).len(),
            selected_area,
            app.focused_panel_scroll(LabFocus::Activity),
        ),
        LabScreen::Ledger => ledger_missing_lines_below(cli, app, selected_area),
    }
}

pub(in super::super) fn home_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    let layout = home_panel_rects(cli, area, app)?;
    let action_lines =
        home_action_lines(cli, app, layout.compact, app.focus == LabFocus::HomeActions);
    let visible_action_lines = home_action_visible_line_indices(
        app,
        action_lines.len(),
        panel_inner_height(layout.actions),
        layout.compact,
    )
    .len();
    let hidden_action_lines = action_lines.len().saturating_sub(visible_action_lines);
    if hidden_action_lines > 0 {
        return Some(hidden_action_lines);
    }

    match app.focus {
        LabFocus::HomeSummary => missing_lines_for_panel(
            home_summary_lines(cli, app).len(),
            layout.summary,
            app.focused_panel_scroll(LabFocus::HomeSummary),
        ),
        LabFocus::HomePreview => layout.preview.and_then(|preview| {
            missing_lines_for_panel(
                home_preview_lines(cli, app, app.selected_home_action()).len(),
                preview,
                app.focused_panel_scroll(LabFocus::HomePreview),
            )
        }),
        _ => None,
    }
}

pub(in super::super) fn chain_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    let Some(detail) = &app.trading.detail else {
        return missing_lines_for_panel(2, area, app.focused_panel_scroll(LabFocus::Detail));
    };

    let layout = chain_layout(cli, area, app);
    if app.focus == LabFocus::Detail || layout.is_none() {
        let summary_area = layout.map_or(area, |layout| layout.summary);
        let hidden = wrapped_line_viewport(
            &chain_summary_lines(cli, app, detail),
            summary_area.width.saturating_sub(2),
            panel_inner_height(summary_area),
            app.focused_panel_scroll(LabFocus::Detail),
            true,
        )
        .hidden_count();
        return (hidden > 0).then_some(hidden);
    }
    let chain_layout = layout?;

    let (kind, column) = match app.focus {
        LabFocus::Calls => (OptionKind::Call, chain_layout.calls),
        LabFocus::Puts => (OptionKind::Put, chain_layout.puts),
        _ => return None,
    };
    let side_layout = option_side_layout(column, app.trading.ticket.is_some());
    let visible_rows = side_layout.table.height.saturating_sub(3).max(1) as usize;
    missing_lines_for_height(
        quote_indices_by_kind(detail, kind).len(),
        visible_rows,
        app.focused_panel_scroll(option_kind_focus(kind)),
    )
}

pub(in super::super) fn chart_missing_lines_below(app: &LabApp, area: Rect) -> Option<usize> {
    if app.trading.chart.is_some() {
        return None;
    }
    missing_lines_for_panel(5, area, app.focused_panel_scroll(LabFocus::Chart))
}

pub(in super::super) fn oracle_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    let layout = oracle_panel_rects(cli, area, app)?;

    if let Some(flow_area) = layout.flow
        && let Some(hidden) = missing_lines_for_panel(
            oracle_flow_lines(cli, app).len(),
            flow_area,
            app.focused_panel_scroll(LabFocus::OracleOverview),
        )
    {
        return Some(hidden);
    }

    let tree_only = layout.selected.is_none()
        && layout.actions.is_none()
        && layout.flow.is_none()
        && layout.path.is_none();
    if tree_only {
        return missing_lines_for_panel(
            oracle_tree_lines(cli, app).len(),
            layout.tree,
            app.focused_panel_scroll(LabFocus::OracleTasks),
        );
    }

    match app.focus {
        LabFocus::OracleActions => layout
            .actions
            .and_then(|area| oracle_action_missing_lines_below(cli, app, area)),
        LabFocus::OracleTasks => missing_lines_for_panel(
            oracle_tree_lines(cli, app).len(),
            layout.tree,
            app.focused_panel_scroll(LabFocus::OracleTasks),
        ),
        LabFocus::OracleOverview => layout.flow.and_then(|area| {
            missing_lines_for_panel(
                oracle_flow_lines(cli, app).len(),
                area,
                app.focused_panel_scroll(LabFocus::OracleOverview),
            )
        }),
        LabFocus::OraclePath => layout.path.and_then(|area| {
            missing_lines_for_panel(
                oracle_current_path_lines(cli, app).len(),
                area,
                app.focused_panel_scroll(LabFocus::OraclePath),
            )
        }),
        _ => None,
    }
}

pub(in super::super) fn oracle_action_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    missing_lines_for_panel(
        oracle_action_lines(cli, app).len(),
        area,
        app.focused_panel_scroll(LabFocus::OracleActions),
    )
}

pub(in super::super) fn ledger_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    let layout = ledger_screen_layout(area, app.ledger_view);
    match app.ledger_pane {
        LedgerPane::Tabs => None,
        LedgerPane::List => {
            let lines = match app.ledger_view {
                LedgerView::Account => ledger_header_lines(cli, app).len() + 7,
                LedgerView::Positions => 1 + app.liquidity_position_rows().len().max(1),
                LedgerView::Writers => 2 + app.writer_sleeve_rows().len().max(1),
                LedgerView::History => 1 + app.ledger_history_rows().len().clamp(1, 50),
            };
            missing_lines_for_panel(lines, layout.list, 0)
        }
        LedgerPane::Actions => {
            let Some(actions) = layout.actions else {
                return None;
            };
            let (lines, scroll) = match app.ledger_view {
                LedgerView::Account => (ledger_header_lines(cli, app).len() + 7, 0),
                LedgerView::Writers => (
                    WriterAction::ALL.len() + 2,
                    writer_action_scroll_offset(actions, app),
                ),
                LedgerView::Positions => (2, 0),
                LedgerView::History => return None,
            };
            missing_lines_for_panel(lines, actions, scroll)
        }
        LedgerPane::Detail => {
            let lines = match app.ledger_view {
                LedgerView::Account => 9 + ledger_amoeba_activity_lines(cli, app).len().min(8),
                LedgerView::Positions => {
                    liquidity_detail_lines(cli, app, app.liquidity_position_rows()).len()
                }
                LedgerView::Writers => writer_detail_lines(cli, app).len(),
                LedgerView::History => app
                    .ledger_history_rows()
                    .get(app.ledger_history_selected)
                    .map(|row| history_detail_lines(cli, row).len())
                    .unwrap_or(3),
            };
            missing_lines_for_panel(
                lines,
                layout.detail,
                app.focused_panel_scroll(LabFocus::Ledger),
            )
        }
    }
}

pub(in super::super) fn missing_lines_for_panel(
    line_count: usize,
    area: Rect,
    scroll: usize,
) -> Option<usize> {
    missing_lines_for_height(line_count, panel_inner_height(area), scroll)
}

pub(in super::super) fn missing_lines_for_height(
    line_count: usize,
    height: usize,
    scroll: usize,
) -> Option<usize> {
    let missing = line_viewport(line_count, height, scroll).hidden_count();
    (missing > 0).then_some(missing)
}

pub(in super::super) fn footer_lines_for_app(
    cli: &Cli,
    app: &LabApp,
    missing_lines: Option<usize>,
) -> Vec<Line<'static>> {
    let mut lines = if app.screen == LabScreen::Help && app.home_help_topic == HomeHelpTopic::Agents
    {
        vec![Line::from(vec![
            help_key(cli, "q"),
            help_text(cli, " quit | "),
            help_key(cli, HOME_SHORTCUT_HELP),
            help_text(cli, " home | "),
            help_key(cli, "backspace"),
            help_text(cli, " back home | "),
            help_key(cli, PANEL_SCROLL_HELP),
            help_text(cli, " scroll | "),
            help_key(cli, "enter"),
            help_text(cli, " connection | "),
            help_key(cli, "T"),
            help_text(cli, " wallet access"),
        ])]
    } else if app.screen == LabScreen::Oracle && app.oracle.view == OracleView::Earn {
        vec![Line::from(vec![
            help_key(cli, "q"),
            help_text(cli, " quit | "),
            help_key(cli, HOME_SHORTCUT_HELP),
            help_text(cli, " home | "),
            help_key_with_color(cli, "v", Color::Green),
            help_text(cli, " advanced view | "),
            help_key(cli, "enter/click"),
            help_text(cli, " find task | "),
            help_key(cli, PANEL_SCROLL_HELP),
            help_text(cli, " scroll | "),
            help_key_with_color(cli, "r", Color::Yellow),
            help_text(cli, " refresh"),
        ])]
    } else {
        help_lines(cli, app.screen)
    };
    if let Some(count) = missing_lines.filter(|count| *count > 0) {
        lines.insert(0, footer_hidden_line(cli, count, app.spinner_tick));
    }
    if app.screen != LabScreen::Terms && lines.len() < 3 {
        lines.push(Line::from(vec![
            help_key(cli, "F8"),
            help_text(cli, " operations | "),
            help_key(cli, "F9"),
            help_text(cli, " actions | "),
            help_key_with_color(cli, "g", Color::LightCyan),
            help_text(cli, " ask Guide | "),
            Span::styled(
                app.guide.provider_status.title(),
                style(cli, Color::DarkGray),
            ),
        ]));
    }
    lines
}

pub(in super::super) fn hidden_lines_notice_color(tick: usize) -> Color {
    if (tick / 3).is_multiple_of(2) {
        Color::Yellow
    } else {
        Color::White
    }
}

pub(in super::super) fn footer_hidden_line(cli: &Cli, count: usize, tick: usize) -> Line<'static> {
    let unit = if count == 1 { "line" } else { "lines" };
    let notice_color = hidden_lines_notice_color(tick);
    Line::from(vec![
        Span::styled("↕ ", style(cli, notice_color).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{count} {unit} hidden"),
            style(cli, notice_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" | {PANEL_SCROLL_HELP} scroll"),
            style(cli, Color::DarkGray),
        ),
    ])
}

type HelpPart = (&'static str, Color, &'static str);

const TERMS_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    ("t", Color::LightCyan, " open terms | "),
    ("w", Color::Blue, " switch wallet | "),
    ("enter", Color::Green, " accept for wallet"),
];

const HOME_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    ("arrows", Color::Yellow, " move boxes/rows | "),
    ("tab", Color::Yellow, " next box | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll | "),
    ("enter", Color::Yellow, " open selected | "),
    ("r", Color::Yellow, " refresh"),
];

const STAKING_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("up/down", Color::Yellow, " select action | "),
    ("enter", Color::Yellow, " check action | "),
    ("r", Color::Yellow, " refresh | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll"),
];

const CHART_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("arrows", Color::Yellow, " move boxes/rows | "),
    ("tab", Color::Yellow, " next box | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll | "),
    ("r", Color::Yellow, " refresh | "),
    ("o", Color::Magenta, " options | "),
    ("a", Color::Blue, " trades"),
];

const CHART_RANGE_HELP: &[HelpPart] = &[
    ("1", Color::Yellow, " 1h "),
    ("2", Color::Yellow, " 24h "),
    ("3", Color::Yellow, " 7d "),
    ("4", Color::Yellow, " 30d "),
    ("5", Color::Yellow, " all"),
];

const HELP_NAV_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("tab/left/right", Color::Yellow, " pane | "),
    ("arrows", Color::Yellow, " select/scroll"),
];

const HELP_OPEN_HELP: &[HelpPart] = &[
    ("hover/space", Color::LightCyan, " preview | "),
    ("enter/click", Color::Yellow, " open | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " page | "),
    ("r", Color::Yellow, " refresh"),
];

const ORACLE_INTRO_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("arrows", Color::Yellow, " choose button | "),
    ("enter", Color::Yellow, " open selected | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll"),
];

const ORACLE_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("v", Color::Green, " switch view | "),
    ("arrows", Color::Yellow, " select | "),
    ("enter/right", Color::Yellow, " drill | "),
    ("backspace/left", Color::Yellow, " parent | "),
    ("tab", Color::Yellow, " next box | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll | "),
    ("[/]", Color::Yellow, " sources | "),
    ("r", Color::Yellow, " refresh | "),
    ("o", Color::Magenta, " options"),
];

const ORACLE_HELP_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("backspace/left", Color::Yellow, " oracle entry | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll | "),
    ("enter", Color::Yellow, " back"),
];

const MARKET_HELP: &[HelpPart] = &[
    ("q", Color::Yellow, " quit | "),
    (HOME_SHORTCUT_HELP, Color::Yellow, " home | "),
    ("arrows", Color::Yellow, " move boxes/rows | "),
    ("tab", Color::Yellow, " next pane | "),
    (PANEL_SCROLL_HELP, Color::Yellow, " scroll | "),
    ("b", Color::Green, " buy | "),
    ("s", Color::Red, " sell | "),
    ("r", Color::Yellow, " refresh | "),
    ("a", Color::Blue, " trades"),
];

const MARKET_NAV_HELP: &[HelpPart] = &[
    ("c", Color::Blue, " chart | "),
    ("o", Color::Magenta, " options | "),
];

pub(in super::super) fn help_lines(cli: &Cli, screen: LabScreen) -> Vec<Line<'static>> {
    let line_sets: &[&[HelpPart]] = match screen {
        LabScreen::Terms => &[TERMS_HELP],
        LabScreen::Home => &[HOME_HELP],
        LabScreen::Staking => &[STAKING_HELP],
        LabScreen::Chart => &[CHART_HELP, CHART_RANGE_HELP],
        LabScreen::OracleIntro => &[ORACLE_INTRO_HELP],
        LabScreen::Oracle => &[ORACLE_HELP],
        LabScreen::OracleHelp => &[ORACLE_HELP_HELP],
        LabScreen::Help => &[HELP_NAV_HELP, HELP_OPEN_HELP],
        LabScreen::Chain | LabScreen::Detail | LabScreen::Activity | LabScreen::Ledger => {
            &[MARKET_HELP, MARKET_NAV_HELP]
        }
    };
    line_sets
        .iter()
        .map(|parts| help_line_from_parts(cli, parts))
        .collect()
}

fn help_line_from_parts(cli: &Cli, parts: &[HelpPart]) -> Line<'static> {
    let mut spans = Vec::with_capacity(parts.len() * 2);
    for &(key, color, text) in parts {
        spans.push(help_key_with_color(cli, key, color));
        spans.push(help_text(cli, text));
    }
    Line::from(spans)
}

pub(in super::super) fn help_key(cli: &Cli, text: &'static str) -> Span<'static> {
    help_key_with_color(cli, text, Color::Yellow)
}

pub(in super::super) fn help_key_with_color(
    cli: &Cli,
    text: &'static str,
    color: Color,
) -> Span<'static> {
    Span::styled(text, style(cli, color).add_modifier(Modifier::BOLD))
}

pub(in super::super) fn help_text(cli: &Cli, text: &'static str) -> Span<'static> {
    Span::styled(text, style(cli, Color::DarkGray))
}
