//! AI agent connection state, controls, and user-facing help.

use super::super::*;

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct AgentConnectionLayout {
    pub(in super::super) help_area: Rect,
    pub(in super::super) button_area: Option<Rect>,
    pub(in super::super) links_area: Option<Rect>,
}

pub(in super::super) fn agent_connection_layout(area: Rect) -> AgentConnectionLayout {
    let links_height = if area.height >= 20 { 4 } else { 0 };
    let button_height = if area.height >= 8 {
        3
    } else if area.height >= 4 {
        1
    } else {
        0
    };
    if button_height == 0 {
        return AgentConnectionLayout {
            help_area: area,
            button_area: None,
            links_area: None,
        };
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(button_height),
            Constraint::Length(links_height),
        ])
        .split(area);
    let button_row = rows[1];
    let button_width = button_row.width.min(34);
    let button_x = button_row
        .x
        .saturating_add(button_row.width.saturating_sub(button_width) / 2);
    let button_area = (button_width > 0).then_some(Rect {
        x: button_x,
        y: button_row.y,
        width: button_width,
        height: button_row.height,
    });
    AgentConnectionLayout {
        help_area: rows[0],
        button_area,
        links_area: (links_height > 0).then_some(rows[2]),
    }
}

pub(in super::super) fn agent_connection_button_rect(area: Rect) -> Option<Rect> {
    agent_connection_layout(area).button_area
}

fn agent_connection_padding(area: Rect) -> u16 {
    (area.width.saturating_sub(4) / 2).min(2)
}

// Keep wrapping and overflow geometry identical to the padded Paragraph.
pub(in super::super) fn agent_connection_text_panel(area: Rect) -> Rect {
    let padding = agent_connection_padding(area);
    Rect {
        x: area.x.saturating_add(padding),
        width: area.width.saturating_sub(padding * 2),
        ..area
    }
}

pub(in super::super) fn draw_agent_connection_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::Help;
    let layout = agent_connection_layout(area);
    let text_panel = agent_connection_text_panel(layout.help_area);
    let help = scrolling_panel(
        home_help_agent_lines_for_width(cli, app, text_panel.width.saturating_sub(2)),
        text_panel,
        cli,
        app.focused_panel_scroll(LabFocus::Help),
        focused,
    )
    .block(
        panel_block(cli, "connect your AI agent", Color::Magenta, focused).padding(
            ratatui::widgets::Padding::horizontal(agent_connection_padding(layout.help_area)),
        ),
    );
    frame.render_widget(help, layout.help_area);

    if let Some(button_area) = layout.button_area {
        let (label, mut color) = match app.mcp_connection_action() {
            McpConnectionAction::Enable => ("ENABLE PETRI MCP", Color::Green),
            McpConnectionAction::Disable => ("DISABLE PETRI MCP", Color::Red),
            McpConnectionAction::Repair => ("REPAIR PETRI MCP", Color::Yellow),
            McpConnectionAction::Blocked => ("EXISTING PETRI MCP FOUND", Color::Yellow),
        };
        if app.guide.focused_control.as_deref() == Some("control:mcp:connection") {
            color = Color::LightYellow;
        }
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                label,
                color,
                app.mcp_connection_button_active(),
                button_area.width,
                button_area.height,
            )),
            button_area,
        );
    }

    if let Some(links_area) = layout.links_area {
        let links = scrolling_panel(home_help_link_lines(cli, app), links_area, cli, 0, false)
            .block(panel_block(cli, "learn more", Color::Cyan, false));
        frame.render_widget(links, links_area);
    }
}

pub(in super::super) fn home_help_link_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled(
                "GitBook: ",
                style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
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

pub(in super::super) fn home_help_agent_lines_for_width(
    cli: &Cli,
    app: &LabApp,
    width: u16,
) -> Vec<Line<'static>> {
    let (status_text, status_color) = match app.mcp_connection_state {
        McpConnectionState::Disabled => ("Not connected", Color::Gray),
        McpConnectionState::Enabled => ("Connected", Color::Green),
        McpConnectionState::NeedsRepair => ("Needs repair", Color::Yellow),
        McpConnectionState::Conflict => ("Existing connection found", Color::Yellow),
    };
    let action_lines: &[&str] = match app.mcp_connection_action() {
        McpConnectionAction::Enable => &[
            "Select Enable Petri MCP below, or press Enter.",
            "Then restart or reload your open assistant.",
        ],
        McpConnectionAction::Disable => &[
            "Petri is enabled for your assistant.",
            "Use the button below to disconnect.",
        ],
        McpConnectionAction::Repair => &[
            "Select Repair Petri MCP below, or press Enter.",
            "Petri repairs only the connection it manages.",
        ],
        McpConnectionAction::Blocked => &[
            "This connection is not managed by Petri.",
            "Your existing settings have been left unchanged.",
        ],
    };
    let heading = |label: &'static str| {
        Line::from(Span::styled(
            label,
            style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
        ))
    };
    let body = |text: &'static str| Line::from(Span::styled(text, style(cli, Color::Gray)));
    let divider = || {
        Line::from(Span::styled(
            "─".repeat(usize::from(width.min(72))),
            style(cli, Color::DarkGray).add_modifier(Modifier::DIM),
        ))
    };
    let mut lines = vec![
        Line::from(""),
        heading("CONNECTION"),
        Line::from(Span::styled(
            status_text,
            style(cli, status_color).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(action_lines.iter().copied().map(body));
    lines.extend([
        Line::from(""),
        divider(),
        heading("SUPPORTED AGENTS"),
        body("Codex / Claude Code / Gemini / other MCP clients"),
        Line::from(""),
        divider(),
        heading("HOW IT WORKS"),
        body("Starts with your assistant."),
        body("Stays enabled after restarts."),
        body("Nothing needs to run while your assistant is closed."),
        Line::from(""),
        divider(),
        heading("YOUR WALLET"),
        body("Agents prepare actions for your review."),
        body("Execution requires your approval of the exact operation."),
        body("Your wallet signs locally. Keys and recovery material stay private."),
    ]);
    if let Some(issue) = app.mcp_connection_issue.as_deref() {
        lines.extend([Line::from(""), divider(), heading("CONNECTION ISSUE")]);
        lines.push(Line::from(Span::styled(
            issue.to_string(),
            style(cli, Color::LightRed).add_modifier(Modifier::BOLD),
        )));
        if app.mcp_repair_failed {
            lines.push(Line::from(Span::styled(
                "If repair still cannot complete, disconnect only Petri's managed connection, then connect it again.",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )));
        }
    }
    lines.push(Line::from(""));
    lines
}

pub(in super::super) fn oracle_help_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "How to read the oracle",
            style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "The main page is for navigation. This page explains the rules behind it.",
            style(cli, Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Settlement rule",
            style(cli, Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Each source is compared with its own opening value. The oracle combines those source-local moves through frozen monthly weights and fixed basket weights.",
            style(cli, Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Phase timeline",
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )),
    ];

    let active = app.oracle_phase();
    for phase in OraclePhase::ALL {
        let marker = if phase == active { "ACTIVE" } else { "      " };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), style(cli, phase.active_color())),
            Span::styled(
                format!("{}: ", phase.label()),
                style(cli, phase.color()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(phase.detail(), style(cli, Color::Gray)),
        ]));
    }

    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Context actions",
            style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Propose source: source category, canonical locator, source definition, product row, stake/support.",
            style(cli, Color::Gray),
        )),
        Line::from(Span::styled(
            "Opening: value, source time, exact canonical URL, matching 14-digit UTC Wayback capture, stake. Updates keep their existing archive-evidence fields.",
            style(cli, Color::Gray),
        )),
        Line::from(Span::styled(
            "Challenge: bad source definition, wrong row, stale opening print, or bad live update evidence.",
            style(cli, Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Evidence rules",
            style(cli, Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Allowed V1 sources: public retailer, distributor, manufacturer/store, benchmark/assessment, or public API.",
            style(cli, Color::Gray),
        )),
        Line::from(Span::styled(
            "Not allowed in V1: OTC trades, private invoices, broker DMs, login-only prices, search snippets, auctions, unstable seller listings.",
            style(cli, Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Controls",
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Enter on the intro opens the tree. Backspace/Left returns from help to the intro. Home/Alt+h returns home.",
            style(cli, Color::Gray),
        )),
    ]);

    lines
}
