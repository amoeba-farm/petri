//! Oracle entry and Oracle help presentation.

use super::super::*;

pub(in super::super) fn draw_oracle_intro_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OracleIntro;
    let intro = scrolling_panel(
        oracle_intro_lines(cli, app, focused),
        area,
        cli,
        app.focused_panel_scroll(LabFocus::OracleIntro),
        focused,
    )
    .block(panel_block(cli, "oracle entry", Color::Magenta, focused));
    frame.render_widget(intro, area);
}

pub(in super::super) fn oracle_intro_lines(
    cli: &Cli,
    app: &LabApp,
    focused: bool,
) -> Vec<Line<'static>> {
    let market = selected_market_context(app);
    let symbol = selected_oracle_market_symbol(app);
    let month = selected_oracle_month_label(app);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Oracle evidence",
                style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" for ", style(cli, Color::DarkGray)),
            Span::styled(
                market,
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "Choose a simple work queue or inspect the complete evidence system.",
            style(cli, Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Market ", style(cli, Color::DarkGray)),
            Span::styled(symbol, style(cli, Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" | month ", style(cli, Color::DarkGray)),
            Span::styled(month, style(cli, Color::White)),
            Span::styled(" | settles ", style(cli, Color::DarkGray)),
            Span::styled(
                selected_oracle_settlement_label(app),
                style(cli, Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Choose your Oracle view:",
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )),
    ];

    for (index, action) in OracleIntroAction::ALL.iter().copied().enumerate() {
        lines.push(oracle_intro_action_line(
            cli,
            action,
            index == app.oracle.intro_selected,
            focused,
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Earn is the normal view. Advanced keeps the complete source and phase workbench.",
        style(cli, Color::DarkGray),
    )));
    lines
}

pub(in super::super) fn oracle_intro_action_line(
    cli: &Cli,
    action: OracleIntroAction,
    selected: bool,
    focused: bool,
) -> Line<'static> {
    let active = selected && focused;
    let marker = if selected { ">" } else { " " };
    Line::from(vec![
        Span::styled(
            format!("{marker} "),
            cell_style(cli, action.color(), active),
        ),
        Span::styled(
            format!(" {} ", action.label()),
            terminal_button_key_style(cli, action.color()),
        ),
        Span::raw("  "),
        Span::styled(action.detail(), cell_style(cli, Color::White, active)),
    ])
}

pub(in super::super) fn draw_oracle_help_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OracleHelp;
    let layout = oracle_help_layout(area);
    let help = scrolling_panel(
        oracle_help_lines(cli, app),
        layout.help_area,
        cli,
        app.focused_panel_scroll(LabFocus::OracleHelp),
        focused,
    )
    .block(panel_block(cli, "oracle help", Color::Cyan, focused));
    frame.render_widget(help, layout.help_area);

    if let Some(links_area) = layout.links_area {
        let links = scrolling_panel(oracle_help_link_lines(cli, app), links_area, cli, 0, false)
            .block(panel_block(cli, "docs and terms", Color::Yellow, false));
        frame.render_widget(links, links_area);
    }
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct OracleHelpLayout {
    pub(in super::super) help_area: Rect,
    pub(in super::super) links_area: Option<Rect>,
}

pub(in super::super) fn oracle_help_layout(area: Rect) -> OracleHelpLayout {
    let links_height = if area.height >= 10 {
        5
    } else if area.height >= 7 {
        4
    } else {
        0
    };
    if links_height == 0 {
        return OracleHelpLayout {
            help_area: area,
            links_area: None,
        };
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(links_height)])
        .split(area);
    OracleHelpLayout {
        help_area: rows[0],
        links_area: Some(rows[1]),
    }
}

pub(in super::super) fn oracle_help_link_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                "Read more on our docs",
                style(cli, Color::Cyan)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(" (GitBook): ", style(cli, Color::DarkGray)),
            Span::styled(
                docs_url(),
                style(cli, Color::Cyan).add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Terms of Service: ",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.wallet_terms.terms_url.clone(),
                style(cli, Color::Yellow).add_modifier(Modifier::UNDERLINED),
            ),
        ]),
    ]
}
