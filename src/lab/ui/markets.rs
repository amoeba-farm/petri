//! Market rail, selected market, and listed-month presentation.

use super::super::*;

pub(in super::super) fn draw_dish_list(frame: &mut Frame<'_>, cli: &Cli, area: Rect, app: &LabApp) {
    let focused = matches!(app.focus, LabFocus::Markets | LabFocus::MarketSeries);
    let lines = dish_list_lines(cli, app);
    let area = wrapped_content_sized_panel_area(area, &lines, true);
    let title = match app.focus {
        LabFocus::Markets => "markets *",
        LabFocus::MarketSeries => "markets: months *",
        _ => "markets",
    };
    let title_style = if focused {
        style(cli, Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        style(cli, Color::Cyan)
    };
    let scroll_focus = if app.focus == LabFocus::MarketSeries {
        LabFocus::MarketSeries
    } else {
        LabFocus::Markets
    };
    let panel = scrolling_panel(
        lines,
        area,
        cli,
        app.focused_panel_scroll(scroll_focus),
        focused,
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(panel_border_style(cli, focused))
            .style(tui_panel_style(cli))
            .title(Span::styled(title, title_style)),
    );
    frame.render_widget(panel, area);
}

pub(in super::super) fn dish_list_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    if app.trading.dishes.is_empty() {
        if app.trading.loading_list {
            return vec![Line::from(Span::styled(
                format!("{} Loading markets...", app.spinner()),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ))];
        }
        return vec![Line::from("No markets available.")];
    }

    let mut lines = Vec::new();
    for (index, dish) in app.trading.dishes.iter().enumerate() {
        let selected = index == app.trading.selected;
        let active = selected && app.focus == LabFocus::Markets;
        let marker = if active {
            ">"
        } else if selected {
            "*"
        } else {
            " "
        };
        lines.push(Line::from(vec![
            Span::styled(marker, cell_style(cli, Color::Yellow, active)),
            Span::raw(" "),
            Span::styled(
                market_button_label(dish),
                cell_style(cli, Color::Cyan, active),
            ),
            Span::styled(
                format!(" {}", market_description(dish)),
                cell_style(cli, Color::Gray, active),
            ),
        ]));
        if selected && app.trading.market_series_open {
            let series = market_series_labels(app, dish);
            if series.is_empty() {
                let message = if app.trading.loading_detail {
                    format!("  {} Loading open months...", app.spinner())
                } else {
                    "  No open months loaded. Press r to refresh.".to_string()
                };
                lines.push(Line::from(Span::styled(
                    message,
                    style(cli, Color::DarkGray),
                )));
            } else {
                let series_focused = app.focus == LabFocus::MarketSeries;
                let selected_series_index =
                    app.trading.chart_expiry.min(series.len().saturating_sub(1));
                lines.push(Line::from(Span::styled(
                    "  Monthly Series",
                    market_series_heading_style(cli),
                )));
                for (series_index, label) in series.iter().enumerate() {
                    let active_series = series_focused && series_index == selected_series_index;
                    let selected_series = series_index == selected_series_index;
                    let month_marker = if active_series {
                        ">"
                    } else if selected_series {
                        "*"
                    } else {
                        " "
                    };
                    let branch = if series_index + 1 == series.len() {
                        "`-"
                    } else {
                        "|-"
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {month_marker} "),
                            cell_style(cli, Color::Yellow, active_series),
                        ),
                        Span::styled(format!("{branch} "), market_series_branch_style(cli)),
                        Span::styled(
                            label.clone(),
                            market_series_label_style(cli, active_series, selected_series),
                        ),
                    ]));
                }
            }
        }
    }
    lines
}

pub(in super::super) fn market_description(dish: &DishSummary) -> std::borrow::Cow<'_, str> {
    match dish.id.as_str() {
        "ramx" => "DRAM monthly".into(),
        "nandx" => "NAND memory monthly".into(),
        _ => {
            let title = dish.title.trim();
            if title.is_empty() {
                "monthly contracts".into()
            } else if title.to_ascii_lowercase().contains("monthly") {
                title.into()
            } else {
                format!("{title} monthly contracts").into()
            }
        }
    }
}

pub(in super::super) fn market_button_label(dish: &DishSummary) -> String {
    format!("[ {:<5} ]", dish.symbol.trim())
}

pub(in super::super) fn market_series_labels(app: &LabApp, dish: &DishSummary) -> Vec<String> {
    let detail = app
        .trading
        .detail
        .as_ref()
        .filter(|detail| detail.id.eq_ignore_ascii_case(&dish.id));
    let labels = detail
        .map(|detail| {
            detail
                .expiries
                .iter()
                .filter_map(series_label_from_expiry)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    compact_series_labels(labels).into_iter().take(6).collect()
}

pub(in super::super) fn market_series_heading_style(cli: &Cli) -> Style {
    style(cli, Color::Blue).add_modifier(Modifier::BOLD)
}

pub(in super::super) fn market_series_branch_style(cli: &Cli) -> Style {
    style(cli, Color::DarkGray)
}

pub(in super::super) fn market_series_label_style(
    cli: &Cli,
    active: bool,
    selected: bool,
) -> Style {
    if active {
        cell_style(cli, Color::Yellow, true)
    } else if selected {
        style(cli, Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        style(cli, Color::Cyan)
    }
}

pub(in super::super) fn draw_selected_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    if app.screen == LabScreen::Terms {
        draw_terms_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Home {
        draw_home_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Staking {
        draw_staking_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Chart {
        draw_chart_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::OracleIntro {
        draw_oracle_intro_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Oracle {
        match app.oracle.view {
            OracleView::Earn => draw_oracle_earn_screen(frame, cli, area, app),
            OracleView::Advanced => draw_oracle_screen(frame, cli, area, app),
        }
        return;
    }
    if app.screen == LabScreen::OracleHelp {
        draw_oracle_help_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Help {
        draw_home_help_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Chain {
        draw_chain_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Ledger {
        draw_ledger_screen(frame, cli, area, app);
        return;
    }
    if app.screen == LabScreen::Detail {
        draw_detail_screen(frame, cli, area, app);
        return;
    }

    let title = screen_title(app.screen);
    let lines = match app.screen {
        LabScreen::Terms => unreachable!("terms screen is rendered before line selection"),
        LabScreen::Home => unreachable!("home screen is rendered before line selection"),
        LabScreen::Staking => unreachable!("staking screen is rendered before line selection"),
        LabScreen::Chain => unreachable!("chain screen is rendered before line selection"),
        LabScreen::OracleIntro => {
            unreachable!("oracle intro screen is rendered before line selection")
        }
        LabScreen::Oracle => unreachable!("oracle screen is rendered before line selection"),
        LabScreen::OracleHelp => {
            unreachable!("oracle help screen is rendered before line selection")
        }
        LabScreen::Help => unreachable!("home help screen is rendered before line selection"),
        LabScreen::Detail => unreachable!("detail screen is rendered before line selection"),
        LabScreen::Activity => activity_lines(cli, app),
        LabScreen::Ledger => ledger_lines(cli, app),
        LabScreen::Chart => unreachable!("chart screen is rendered before line selection"),
    };
    let focused = match app.screen {
        LabScreen::Detail => app.focus == LabFocus::Detail,
        LabScreen::Activity => app.focus == LabFocus::Activity,
        LabScreen::Ledger => app.focus == LabFocus::Ledger,
        LabScreen::Staking => app.focus == LabFocus::Staking,
        LabScreen::Help => app.focus == LabFocus::Help,
        _ => false,
    };
    let panel = scrolling_panel(
        lines,
        area,
        cli,
        app.focused_panel_scroll(app.focus),
        focused,
    )
    .block(panel_block(cli, title, Color::Cyan, focused));
    frame.render_widget(panel, area);
}
