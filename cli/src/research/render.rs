use crate::research::model::{Action, App, Hit, Modal, View, field, safe, usdc};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};

// Neutral surfaces and fixed rules keep the workspace readable as a data feed.
pub(super) const BG: Color = Color::Rgb(17, 19, 21);
pub(super) const BAR: Color = BG;
pub(super) const PANEL: Color = BG;
pub(super) const PLATE: Color = BG;
pub(super) const PLATE_FOCUS: Color = Color::Rgb(46, 50, 54);
pub(super) const FIELD: Color = Color::Rgb(23, 26, 29);
pub(super) const SHADOW: Color = Color::Rgb(9, 10, 11);
pub(super) const BORDER: Color = Color::Rgb(65, 70, 75);
pub(super) const TEXT: Color = Color::Rgb(213, 217, 221);
pub(super) const MUTED: Color = Color::Rgb(146, 153, 160);
pub(super) const DIM: Color = Color::Rgb(99, 107, 115);
pub(super) const ACCENT: Color = Color::Rgb(188, 198, 208);
pub(super) const SELECTED: Color = Color::Rgb(36, 40, 44);
pub(super) const ALERT: Color = Color::Rgb(198, 149, 144);

fn muted() -> Style {
    Style::default().fg(MUTED)
}
fn accent() -> Style {
    Style::default().fg(ACCENT)
}
fn strong() -> Style {
    Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
}
fn active_plate() -> Style {
    Style::default()
        .fg(TEXT)
        .bg(SELECTED)
        .add_modifier(Modifier::BOLD)
}
fn mix(from: Color, to: Color, amount: f32) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) else {
        return to;
    };
    let amount = amount.clamp(0.0, 1.0);
    let channel =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2))
}
fn label(frame: &mut Frame, area: Rect, value: impl Into<String>, style: Style) {
    frame.render_widget(
        Paragraph::new(value.into())
            .style(style)
            .wrap(Wrap { trim: true }),
        area,
    );
}
fn heading(frame: &mut Frame, area: Rect, value: &str) {
    frame.render_widget(Paragraph::new(safe(value)).style(strong()), area);
}
/// Registers a clickable area; returns whether it has keyboard focus or the pointer.
pub(crate) fn register(app: &mut App, area: Rect, action: Action) -> bool {
    let focused = app.modal.is_none() && app.focus == app.hits.len();
    let hovered = app
        .pointer
        .is_some_and(|(x, y)| area.contains((x, y).into()));
    app.hits.push(Hit { area, action });
    focused || hovered
}
fn button(frame: &mut Frame, app: &mut App, area: Rect, title: &str, action: Action, active: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focused = register(app, area, action);
    frame.render_widget(
        Paragraph::new(format!("[{title}]")).style(if active {
            active_plate()
        } else if focused {
            Style::default()
                .fg(ACCENT)
                .bg(PLATE_FOCUS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT).bg(PLATE)
        }),
        area,
    );
}
fn buttons(frame: &mut Frame, app: &mut App, area: Rect, values: Vec<(&str, Action, bool)>) {
    let mut x = area.x;
    for (title, action, active) in values {
        let width = (title.chars().count() as u16 + 2).min(area.right().saturating_sub(x));
        if width < 3 {
            break;
        }
        button(
            frame,
            app,
            Rect::new(x, area.y, width, 1),
            title,
            action,
            active,
        );
        x = x.saturating_add(width + 2);
    }
}
fn paragraph(frame: &mut Frame, app: &mut App, area: Rect, lines: Vec<Line<'static>>) {
    let width = usize::from(area.width.max(1));
    let rows: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum();
    app.scroll = app.scroll.min(
        rows.saturating_sub(usize::from(area.height))
            .min(u16::MAX as usize) as u16,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.scroll, 0)),
        area,
    );
}
fn line(value: impl Into<String>) -> Line<'static> {
    Line::raw(safe(&value.into()))
}
fn subdued(value: impl Into<String>) -> Line<'static> {
    Line::styled(safe(&value.into()), muted())
}
fn within(area: Rect, y: u16, height: u16) -> Rect {
    Rect::new(
        area.x,
        y,
        area.width,
        height.min(area.bottom().saturating_sub(y)),
    )
}
fn pairing_line(name: String, code: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(name, strong()),
        Span::styled(" · ", Style::default().fg(DIM)),
        Span::styled(format!(" {} ", safe(code)), active_plate()),
    ])
}

fn panel(frame: &mut Frame, area: Rect, title: Option<&str>) -> Rect {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL));
    if let Some(title) = title {
        block = block.title(Line::styled(format!(" {title} "), muted()));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn header(frame: &mut Frame, area: Rect, view: View) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("PETRI", strong()),
            Span::styled(
                if view == View::Markets {
                    "    Capped futures"
                } else {
                    "    Oracle research"
                },
                muted(),
            ),
        ])),
        area,
    );
}

fn tabs(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    let full: u16 = View::ALL
        .iter()
        .map(|v| v.label().len() as u16 + 4)
        .sum::<u16>();
    let narrow = full > area.width + 2;
    for view in View::ALL {
        let title = if narrow {
            view.short_label()
        } else {
            view.label()
        };
        let width = (title.len() as u16 + 2).min(area.right().saturating_sub(x));
        if width < 3 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let focused = register(app, rect, Action::View(view));
        let selected = app.view == view;
        let style = if selected {
            active_plate()
        } else if focused {
            strong().bg(PLATE_FOCUS)
        } else {
            muted()
        };
        let text = if selected {
            format!("[{title}]")
        } else {
            format!(" {title} ")
        };
        frame.render_widget(Paragraph::new(text).style(style), rect);
        x = x.saturating_add(width + 2);
    }
}

fn footer(frame: &mut Frame, app: &App, area: Rect) {
    let content = if app.status.is_empty() && app.view == View::Markets {
        let mut spans = Vec::new();
        for (i, (key, label)) in crate::research::markets::footer_hint()
            .into_iter()
            .enumerate()
        {
            if i > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(DIM)));
            }
            spans.push(Span::styled(key, Style::default().fg(MUTED)));
            spans.push(Span::styled(label, muted()));
        }
        Line::from(spans)
    } else if app.status.is_empty() {
        let key = Style::default().fg(MUTED);
        let separator = Style::default().fg(DIM);
        Line::from(vec![
            Span::styled("Click", key),
            Span::styled(" or ", muted()),
            Span::styled("Tab", key),
            Span::styled(" · ", separator),
            Span::styled("Enter", key),
            Span::styled(" · ", separator),
            Span::styled("m/c/a/e/r", key),
            Span::styled(" · ", separator),
            Span::styled("↑/↓", key),
            Span::styled(" Agents", muted()),
            Span::styled(" · ", separator),
            Span::styled("q", key),
            Span::styled(" Quit", muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("", muted()),
            Span::styled(safe(&app.status), Style::default().fg(TEXT)),
        ])
    };
    frame.render_widget(
        Paragraph::new(content).block(
            Block::default()
                .style(Style::default().bg(BAR))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(TEXT).bg(BG)),
        area,
    );
    if area.width < 45 || area.height < 16 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Petri research", accent().add_modifier(Modifier::BOLD)),
                Line::raw(""),
                Line::raw("Enlarge the terminal to at least 45 × 16."),
                Line::styled("Press q to quit.", muted()),
            ]),
            area,
        );
        return;
    }
    let margin = if area.width >= 60 { 2 } else { 1 };
    let workspace = Rect::new(
        area.x + margin,
        area.y + 1,
        area.width.saturating_sub(margin * 2),
        area.height.saturating_sub(2),
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(workspace);
    header(frame, rows[0], app.view);
    let body = rows[4];
    tabs(frame, app, rows[2]);
    let mut main = body;
    if app.view == View::Markets {
        if body.width >= 72 {
            crate::research::markets::sidebar(
                frame,
                &app.markets,
                Rect::new(body.x, body.y, 22, body.height),
            );
            main = Rect::new(body.x + 25, body.y, body.width - 25, body.height);
        }
        let inner = panel(frame, main, None);
        let content = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        crate::research::markets::content(frame, &app.markets, content);
        footer(frame, app, rows[6]);
        if let Some(modal) = app.modal.clone() {
            let parent = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
            modal_view(frame, app, parent, modal);
        }
        if app.focus >= app.hits.len() {
            app.focus = 0;
        }
        return;
    }
    if body.width >= 72 {
        sidebar(frame, app, Rect::new(body.x, body.y, 22, body.height));
        main = Rect::new(body.x + 25, body.y, body.width - 25, body.height);
    }
    let inner = panel(frame, main, None);
    let vertical_padding = u16::from(inner.height >= 16);
    let mut content = Rect::new(
        inner.x + 2,
        inner.y + vertical_padding,
        inner.width.saturating_sub(4),
        inner.height.saturating_sub(vertical_padding * 2),
    );
    if body.width < 72 {
        let count = app.snapshot.agents.len();
        if count > 0 {
            let index = app
                .snapshot
                .agents
                .iter()
                .position(|a| Some(&a.id) == app.selected_agent.as_ref())
                .unwrap_or(0);
            let a = app.snapshot.agents[index].clone();
            let next = app.snapshot.agents[(index + 1) % count].id.clone();
            button(
                frame,
                app,
                Rect::new(content.x, content.y, content.width, 1),
                &format!("{} · {}  [next agent →]", safe(&a.name), safe(&a.harness)),
                Action::Agent(next),
                true,
            );
            content.y += 2;
            content.height = content.height.saturating_sub(2);
        }
    }
    if let Some(error) = app.research_error.clone() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Oracle research is unavailable here.", strong()),
                Line::raw(""),
                Line::raw(safe(&error)),
                Line::raw(""),
                Line::styled("Press m for futures Markets.", muted()),
            ])
            .wrap(Wrap { trim: true }),
            content,
        );
        footer(frame, app, rows[6]);
        return;
    }
    match app.view {
        View::Markets => {}
        View::Connect => connect(frame, app, content),
        View::Activity => activity(frame, app, content),
        View::Evidence => evidence(frame, app, content),
        View::Rewards => rewards(frame, app, content),
    }
    footer(frame, app, rows[6]);
    if let Some(modal) = app.modal.clone() {
        let parent = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        modal_view(frame, app, parent, modal);
    }
    if app.focus >= app.hits.len() {
        app.focus = 0;
    }
}

fn sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    app.sidebar = area;
    let inner = panel(frame, area, Some("AGENTS"));
    if app.snapshot.agents.is_empty() {
        label(
            frame,
            Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 3),
            "No agents yet.",
            muted(),
        );
        return;
    }
    let capacity = usize::from(inner.height.saturating_sub(1) / 3).max(1);
    app.agent_scroll = app
        .agent_scroll
        .min(app.snapshot.agents.len().saturating_sub(capacity));
    let agents = app
        .snapshot
        .agents
        .iter()
        .skip(app.agent_scroll)
        .take(capacity)
        .cloned()
        .collect::<Vec<_>>();
    for (index, agent) in agents.iter().enumerate() {
        let y = inner.y + 1 + index as u16 * 3;
        if y + 2 > inner.bottom() {
            break;
        }
        let selected = Some(&agent.id) == app.selected_agent.as_ref();
        let focused = register(
            app,
            Rect::new(inner.x, y, inner.width, 1),
            Action::Agent(agent.id.clone()),
        );
        app.hits.push(Hit {
            area: Rect::new(inner.x, y + 1, inner.width, 1),
            action: Action::Agent(agent.id.clone()),
        });
        let background = if selected {
            SELECTED
        } else if focused {
            PLATE_FOCUS
        } else {
            PANEL
        };
        let marker = if selected { "> " } else { "  " };
        let name = if selected || focused {
            accent().add_modifier(Modifier::BOLD)
        } else {
            strong()
        };
        let subtitle = if agent.revoked_at.is_some() {
            Span::styled("Disconnected", Style::default().fg(ALERT))
        } else {
            Span::styled(safe(&agent.harness), muted())
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(marker, accent()),
                    Span::styled(safe(&agent.name), name),
                ]),
                Line::from(vec![Span::styled(marker, accent()), subtitle]),
            ])
            .style(Style::default().bg(background)),
            Rect::new(inner.x, y, inner.width, 2),
        );
    }
}

fn connect(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height < 21 || area.width < 62 {
        compact_connect(frame, app, area);
        return;
    }
    heading(frame, within(area, area.y, 1), "Connect your agent");
    let mut x = area.x;
    let mut y = area.y + 2;
    for (index, harness) in App::HARNESSES.iter().enumerate() {
        let width = harness.len() as u16 + 2;
        if x + width > area.right() {
            y += 2;
            x = area.x;
        }
        if y >= area.bottom() {
            return;
        }
        button(
            frame,
            app,
            Rect::new(x, y, width, 1),
            harness,
            Action::Harness(index),
            app.harness == index,
        );
        x += width + 2;
    }
    y += 3;
    let name = format!("Name: {}", app.name);
    buttons(
        frame,
        app,
        within(area, y, 1),
        vec![(&name, Action::Name, false)],
    );
    y += 2;
    buttons(
        frame,
        app,
        within(area, y, 1),
        vec![
            ("Copy login", Action::CopyCommand, false),
            ("Copy MCP setup", Action::CopyMcp, false),
        ],
    );
    y += 2;
    label(
        frame,
        within(area, y, 2),
        "Run the login command in your agent's harness. Approve its code here.",
        muted(),
    );
    y += 3;
    buttons(
        frame,
        app,
        within(area, y, 1),
        vec![("Reward address", Action::Payout, false)],
    );
    y += 2;
    let address = if app.snapshot.settings.payout_authority.is_empty() {
        "Set your public Solana address before approving.".into()
    } else {
        app.snapshot.settings.payout_authority.clone()
    };
    label(frame, within(area, y, 2), address, muted());
    y += 3;
    let pairings = app.snapshot.pairings.clone();
    if pairings.is_empty() {
        label(
            frame,
            within(area, y, 2),
            "No connection requests.",
            muted(),
        );
        y += 3;
        if let Some(agent) = app
            .snapshot
            .agents
            .iter()
            .find(|a| Some(&a.id) == app.selected_agent.as_ref() && a.revoked_at.is_none())
            .cloned()
        {
            buttons(
                frame,
                app,
                within(area, y, 1),
                vec![("Disconnect agent", Action::Revoke(agent.id), false)],
            );
        }
        return;
    }
    app.scroll = app.scroll.min(pairings.len().saturating_sub(1) as u16);
    for pair in pairings.iter().skip(usize::from(app.scroll)) {
        if y + 4 > area.bottom() {
            break;
        }
        frame.render_widget(
            Paragraph::new(pairing_line(
                format!("{} / {}", safe(&pair.name), safe(&pair.harness)),
                &pair.code,
            )),
            within(area, y, 1),
        );
        y += 1;
        if let Some(error) = &pair.error {
            label(frame, within(area, y, 1), safe(error), muted());
            y += 1;
        }
        if pair.status == "approving" {
            label(frame, within(area, y, 1), "Connecting…", muted());
        } else {
            buttons(
                frame,
                app,
                within(area, y, 1),
                vec![
                    ("Approve", Action::Approve(pair.id.clone()), false),
                    ("Reject", Action::Reject(pair.id.clone()), false),
                ],
            );
        }
        y += 3;
    }
}

fn compact_connect(frame: &mut Frame, app: &mut App, area: Rect) {
    let name = format!("Name: {}", app.name);
    buttons(
        frame,
        app,
        within(area, area.y, 1),
        vec![
            (&name, Action::Name, false),
            ("Reward address", Action::Payout, false),
        ],
    );
    let harness = App::HARNESSES[app.harness];
    buttons(
        frame,
        app,
        within(area, area.y + 2, 1),
        vec![
            (
                harness,
                Action::Harness((app.harness + 1) % App::HARNESSES.len()),
                false,
            ),
            ("Copy login", Action::CopyCommand, false),
            ("Copy MCP", Action::CopyMcp, false),
        ],
    );
    let pairs = app.snapshot.pairings.clone();
    app.scroll = app.scroll.min(pairs.len().saturating_sub(1) as u16);
    let mut y = area.y + 4;
    if pairs.is_empty() {
        label(
            frame,
            within(area, y, 1),
            "No connection requests.",
            muted(),
        );
        if let Some(agent) = app
            .snapshot
            .agents
            .iter()
            .find(|a| Some(&a.id) == app.selected_agent.as_ref() && a.revoked_at.is_none())
            .cloned()
        {
            buttons(
                frame,
                app,
                within(area, y + 2, 1),
                vec![("Disconnect agent", Action::Revoke(agent.id), false)],
            );
        }
        return;
    }
    for pair in pairs.iter().skip(usize::from(app.scroll)) {
        if y + 3 > area.bottom() {
            break;
        }
        frame.render_widget(
            Paragraph::new(pairing_line(safe(&pair.name), &pair.code)),
            within(area, y, 1),
        );
        y += 1;
        if pair.status == "approving" {
            label(frame, within(area, y, 1), "Connecting…", muted());
        } else {
            buttons(
                frame,
                app,
                within(area, y, 1),
                vec![
                    ("Approve", Action::Approve(pair.id.clone()), false),
                    ("Reject", Action::Reject(pair.id.clone()), false),
                ],
            );
        }
        y += 3;
    }
}

fn activity(frame: &mut Frame, app: &mut App, area: Rect) {
    buttons(
        frame,
        app,
        within(area, area.y, 1),
        vec![
            (
                "Work",
                Action::ActivitySection(0),
                app.activity_section == 0,
            ),
            (
                "Runs",
                Action::ActivitySection(1),
                app.activity_section == 1,
            ),
            (
                "Directives",
                Action::ActivitySection(2),
                app.activity_section == 2,
            ),
            ("Scan", Action::Discover, false),
        ],
    );
    let content = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );
    if app.activity_section == 0 {
        research_work(frame, app, content);
        return;
    }
    if app.activity_section == 2 {
        directives(frame, app, content);
        return;
    }
    let Some(detail) = app.selected().cloned() else {
        label(
            frame,
            content,
            "Select an agent to follow its research runs.",
            muted(),
        );
        return;
    };
    if detail.events.is_empty() {
        label(
            frame,
            content,
            "This agent has not started a research run yet.",
            muted(),
        );
        return;
    }
    let mut lines = vec![
        Line::styled(safe(&format!("{} / Activity", detail.agent.name)), strong()),
        line(""),
    ];
    let columns = content.width >= 62;
    let prefix_width = if columns { 38 } else { 0 };
    if columns {
        lines.push(subdued(format!(
            "{:<10}{:<10}{:<18}DETAIL",
            "TIME UTC", "FROM", "EVENT"
        )));
        lines.push(subdued("─".repeat(usize::from(content.width))));
    }
    for event in detail.events.iter().rev() {
        let seconds = event.created_at / 1000 % 60;
        let minutes = event.created_at / 60000 % 60;
        let hours = event.created_at / 3600000 % 24;
        let time = format!("{hours:02}:{minutes:02}:{seconds:02}");
        let kind = match event.kind.as_str() {
            "note" => "Progress".to_string(),
            "blocked" => "Blocked".to_string(),
            "source_found" => "Source found".to_string(),
            "run_finished" => "Run finished".to_string(),
            "run_started" => "Run started".to_string(),
            "candidate_received" => "Received".to_string(),
            "candidate_duplicate" => "Duplicate".to_string(),
            other => safe(&other.replace('_', " ")),
        };
        let actor = safe(&event.actor);
        let prefix = if columns {
            format!(
                "{time:<10}{actor:<10}{:<18}",
                kind.chars().take(16).collect::<String>()
            )
        } else {
            lines.push(subdued(format!("{time} UTC    {actor}    {kind}")));
            String::new()
        };
        let width = usize::from(content.width)
            .saturating_sub(prefix_width)
            .max(1);
        for (index, text) in wrap_feed(&safe(&event.message), width)
            .into_iter()
            .enumerate()
        {
            lines.push(Line::from(vec![
                Span::styled(
                    if index == 0 {
                        prefix.clone()
                    } else {
                        " ".repeat(prefix_width)
                    },
                    muted(),
                ),
                Span::raw(text),
            ]));
        }
        if let Some(reference) = &event.reference_url {
            for text in wrap_feed(&safe(reference), width) {
                lines.push(subdued(format!("{}{text}", " ".repeat(prefix_width))));
            }
        }
        lines.push(line(""));
    }
    paragraph(frame, app, content, lines);
}
fn wrap_feed(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in value.split('\n') {
        let mut row = String::new();
        let mut cells = 0;
        for word in paragraph.split_whitespace() {
            let size = Span::raw(word).width();
            if !row.is_empty() && cells + 1 + size > width {
                lines.push(std::mem::take(&mut row));
                cells = 0;
            }
            if !row.is_empty() {
                row.push(' ');
                cells += 1;
            }
            for symbol in word.chars() {
                let size = Span::raw(symbol.to_string()).width();
                if cells + size > width && !row.is_empty() {
                    lines.push(std::mem::take(&mut row));
                    cells = 0;
                }
                row.push(symbol);
                cells += size;
            }
        }
        lines.push(row);
    }
    lines
}

fn research_work(frame: &mut Frame, app: &mut App, area: Rect) {
    heading(frame, within(area, area.y, 1), "Oracle coverage gaps");
    if app.snapshot.work.is_empty() {
        let mut lines = vec![line("No research gaps confirmed."), line("")];
        if let Some(error) = app.snapshot.discovery["error"].as_str() {
            lines.push(line(error));
        } else if app.snapshot.discovery["assessedAt"].is_null() {
            lines.push(subdued("Reading oracle coverage against your directives…"));
        } else if let Some(assessments) = app.snapshot.discovery["assessments"].as_array() {
            for assessment in assessments
                .iter()
                .filter(|a| a["status"] == "unknown")
                .take(32)
            {
                lines.push(line(field(assessment, "expiryId")));
                lines.push(subdued(field(assessment, "reason")));
                lines.push(line(""));
            }
            if lines.len() == 2 {
                lines.push(subdued("Your enabled directives have no open coverage gaps in the available inventories."));
            }
        }
        paragraph(
            frame,
            app,
            Rect::new(
                area.x,
                area.y + 2,
                area.width,
                area.height.saturating_sub(2),
            ),
            lines,
        );
        return;
    }
    let jobs = app.snapshot.work.clone();
    app.scroll = app.scroll.min(jobs.len().saturating_sub(1) as u16);
    let mut y = area.y + 2;
    for (index, job) in jobs.iter().enumerate().skip(usize::from(app.scroll)) {
        if y + 2 > area.bottom() {
            break;
        }
        button(
            frame,
            app,
            Rect::new(area.x, y, area.width, 1),
            &safe(&format!(
                "{}{}",
                if job.availability == "available" || job.availability.is_empty() {
                    String::new()
                } else {
                    format!("{} · ", job.availability)
                },
                job.title
            )),
            Action::ShowWork(index),
            false,
        );
        y += 2;
    }
}

fn directives(frame: &mut Frame, app: &mut App, area: Rect) {
    buttons(
        frame,
        app,
        within(area, area.y, 1),
        vec![("Add directive", Action::NewDirective, false)],
    );
    let policies = app.snapshot.directives.clone();
    app.scroll = app.scroll.min(policies.len().saturating_sub(1) as u16);
    let mut y = area.y + 2;
    for policy in policies.iter().skip(usize::from(app.scroll)) {
        if y + 4 > area.bottom() {
            break;
        }
        let markets = if policy.market_ids.is_empty() {
            "All markets".into()
        } else {
            policy.market_ids.join(", ")
        };
        label(
            frame,
            within(area, y, 1),
            markets,
            accent().add_modifier(Modifier::BOLD),
        );
        label(
            frame,
            within(area, y + 1, 1),
            format!(
                "{} active sources · {} hosts per product",
                policy.min_active_sources, policy.min_distinct_hosts
            ),
            muted(),
        );
        buttons(
            frame,
            app,
            within(area, y + 2, 1),
            vec![
                ("Edit", Action::EditDirective(policy.id.clone()), false),
                (
                    if policy.enabled { "Pause" } else { "Resume" },
                    Action::ToggleDirective(policy.id.clone()),
                    false,
                ),
            ],
        );
        y += 4;
    }
}

fn evidence(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(detail) = app.selected().cloned() else {
        label(
            frame,
            area,
            "Connect an agent to follow its evidence.",
            muted(),
        );
        return;
    };
    let sources = app.sources();
    if sources.is_empty() {
        label(
            frame,
            area,
            detail
                .validation_error
                .unwrap_or_else(|| "No source evidence submitted yet.".into()),
            muted(),
        );
        return;
    }
    app.source = app.source.min(sources.len() - 1);
    let source = &sources[app.source];
    let data = &source["details"];
    let previous = if app.source == 0 {
        sources.len() - 1
    } else {
        app.source - 1
    };
    buttons(
        frame,
        app,
        within(area, area.y, 1),
        vec![
            ("← Previous", Action::Source(previous), false),
            (
                "Next →",
                Action::Source((app.source + 1) % sources.len()),
                false,
            ),
        ],
    );
    let mut lines = vec![
        subdued(format!("Source {} of {}", app.source + 1, sources.len())),
        line(""),
    ];
    for (title, key) in [
        ("Source URL", "canonicalUrl"),
        ("Product", "productDefinition"),
        ("Price field", "priceField"),
        ("Region", "region"),
        ("Currency", "currency"),
        ("Submitted evidence", "evidence"),
    ] {
        lines.push(subdued(title));
        lines.push(line(field(data, key)));
        lines.push(line(""));
    }
    if let Some(error) = detail.validation_error {
        lines.push(subdued(error));
    }
    paragraph(
        frame,
        app,
        Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        ),
        lines,
    );
}

fn rewards(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(detail) = app.selected().cloned() else {
        label(
            frame,
            area,
            "Connect an agent to follow its rewards.",
            muted(),
        );
        return;
    };
    if let Some(error) = detail.validation_error {
        label(frame, area, error, muted());
        return;
    }
    let mut lines = vec![
        Line::styled(safe(&format!("{} / Rewards", detail.agent.name)), strong()),
        subdued("Recorded payouts · USDC"),
        subdued(format!("Reward address: {}", detail.agent.payout_authority)),
        line(""),
    ];
    let candidates = detail.candidates.unwrap_or_default();
    if candidates.is_empty() {
        lines.push(subdued("No confirmed rewards yet."));
    }
    for candidate in &candidates {
        lines.push(line(field(&candidate["details"], "productDefinition")));
        let observation = &candidate["observation"];
        if observation.is_null() {
            lines.push(subdued("Awaiting oracle outcome."));
        } else if observation["rewardSettled"] != true {
            lines.push(subdued("No settled reward recorded."));
        } else {
            for (title, key) in [
                ("Researcher cash paid", "researcherCashAtomic"),
                ("Retained from reward", "researcherReserveAtomic"),
                ("Sponsor share", "sponsorAtomic"),
            ] {
                if let Some(value) = usdc(&observation["allocation"][key]) {
                    lines.push(line(format!("{title}: {value}")));
                }
            }
        }
        lines.push(line(""));
    }
    if !detail.complete {
        lines.push(subdued(
            "More source history is available from the candidate API.",
        ));
    }
    paragraph(frame, app, area, lines);
}

/// Dims everything already drawn so a dialog stands out.
pub(super) fn scrim(buf: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            let (fg, bg) = (cell.fg, cell.bg);
            cell.set_fg(mix(fg, SHADOW, 0.6))
                .set_bg(mix(bg, SHADOW, 0.5));
        }
    }
}

fn modal_view(frame: &mut Frame, app: &mut App, parent: Rect, modal: Modal) {
    app.hits.clear();
    let screen = frame.area();
    scrim(frame.buffer_mut(), screen);
    let width = parent.width.saturating_sub(4).min(80);
    let height = parent.height.saturating_sub(2).min(
        if matches!(
            modal,
            Modal::Directive { .. } | Modal::Refine { .. } | Modal::Assignment(_)
        ) {
            21
        } else {
            13
        },
    );
    let area = Rect::new(
        parent.x + (parent.width - width) / 2,
        parent.y + (parent.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let title = match &modal {
        Modal::Payout(_) => " REWARD ADDRESS ",
        Modal::Name(_) => " AGENT NAME ",
        Modal::Directive { .. } => " STANDING DIRECTIVE ",
        Modal::Refine { .. } => " RESEARCH GUIDANCE ",
        Modal::Assignment(_) => " COVERAGE OBJECTIVE ",
        Modal::Revoke(_) => " DISCONNECT AGENT ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().fg(TEXT).bg(PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.buffer_mut().set_stringn(
        area.x + 2,
        area.y,
        title,
        usize::from(area.width.saturating_sub(4)),
        active_plate(),
    );
    let body = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(4),
    );
    match modal {
        Modal::Payout(value) | Modal::Name(value) => {
            input(
                frame,
                app,
                Rect::new(body.x, body.y + 1, body.width, 3),
                &value,
                0,
            );
            label(
                frame,
                within(body, body.y + 5, 1),
                "Enter to save · Esc to cancel",
                muted(),
            );
        }
        Modal::Directive {
            markets,
            sources,
            hosts,
            ..
        } => {
            label(
                frame,
                within(body, body.y, 1),
                "Markets (empty means all)",
                muted(),
            );
            input(
                frame,
                app,
                Rect::new(body.x, body.y + 1, body.width, 3),
                &markets,
                0,
            );
            label(
                frame,
                within(body, body.y + 4, 1),
                "Active sources per product",
                muted(),
            );
            let half = body.width / 2;
            input(
                frame,
                app,
                Rect::new(body.x, body.y + 5, half.saturating_sub(1), 3),
                &sources,
                1,
            );
            label(
                frame,
                Rect::new(body.x + half, body.y + 4, half, 1),
                "Distinct hosts",
                muted(),
            );
            input(
                frame,
                app,
                Rect::new(body.x + half, body.y + 5, half, 3),
                &hosts,
                2,
            );
            label(
                frame,
                within(body, body.y + 9, 2),
                "Coverage targets guide research. Oracle acceptance remains separate.",
                muted(),
            );
        }
        Modal::Refine { instruction, .. } => {
            label(
                frame,
                within(body, body.y, 1),
                "Lower-level research direction",
                muted(),
            );
            input(
                frame,
                app,
                Rect::new(
                    body.x,
                    body.y + 1,
                    body.width,
                    body.height.saturating_sub(1),
                ),
                &instruction,
                0,
            );
        }
        Modal::Assignment(work) => {
            label(
                frame,
                within(body, body.y, 2),
                safe(&work.title),
                accent().add_modifier(Modifier::BOLD),
            );
            let mut lines: Vec<_> = work
                .brief
                .lines()
                .map(|value| line(value.to_string()))
                .collect();
            if let Some(instruction) = work.guidance["instruction"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                lines.push(line(""));
                lines.push(subdued("Human guidance"));
                lines.extend(instruction.lines().map(|v| line(v.to_string())));
            }
            if let Some(sources) = work.objective["sources"].as_array() {
                lines.push(line(""));
                lines.push(subdued("Existing source URLs"));
                for source in sources {
                    if let Some(url) = source["url"].as_str() {
                        lines.push(line(url.to_string()));
                    }
                }
            }
            paragraph(
                frame,
                app,
                within(body, body.y + 2, body.height.saturating_sub(2)),
                lines,
            );
        }
        Modal::Revoke(agent) => {
            frame.render_widget(Paragraph::new(format!("Disconnect {}?\n\nFuture requests will be denied. Research history stays available.", safe(&agent.name))).wrap(Wrap { trim: false }), body);
            buttons(
                frame,
                app,
                Rect::new(inner.x + 1, inner.bottom() - 2, inner.width - 2, 1),
                vec![
                    (
                        "Disconnect",
                        Action::ConfirmRevoke(agent.id),
                        app.form_focus == 0,
                    ),
                    ("Cancel", Action::Cancel, app.form_focus == 1),
                ],
            );
            return;
        }
    }
    let fields = app.form_fields();
    let values = if let Some(Modal::Assignment(work)) = &app.modal {
        if work.objective.is_null() {
            vec![("Close", Action::Cancel, true)]
        } else {
            vec![
                (
                    "Refine",
                    Action::Refine(work.id.clone()),
                    app.form_focus == 0,
                ),
                ("Close", Action::Cancel, app.form_focus == 1),
            ]
        }
    } else {
        vec![
            ("Save", Action::Save, app.form_focus == fields),
            ("Cancel", Action::Cancel, app.form_focus == fields + 1),
        ]
    };
    buttons(
        frame,
        app,
        Rect::new(inner.x + 1, inner.bottom() - 2, inner.width - 2, 1),
        values,
    );
    if !app.status.is_empty() {
        label(
            frame,
            Rect::new(inner.x + 1, inner.bottom() - 1, inner.width - 2, 1),
            safe(&app.status),
            muted(),
        );
    }
}
fn input(frame: &mut Frame, app: &mut App, area: Rect, value: &str, index: usize) {
    if area.height < 2 {
        return;
    }
    let focused = app.form_focus == index;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused { ACCENT } else { DIM }))
        .style(Style::default().fg(TEXT).bg(FIELD));
    let width = area.width.saturating_sub(2).max(1) as usize;
    let visible = if area.height <= 3 {
        value
            .chars()
            .rev()
            .take(width.saturating_sub(1))
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
    } else {
        value.to_string()
    };
    let mut lines: Vec<Line> = safe(&visible)
        .split('\n')
        .map(|value| Line::raw(value.to_string()))
        .collect();
    if focused && let Some(last) = lines.last_mut() {
        last.push_span(Span::styled("▌", accent()));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
    app.hits.push(Hit {
        area,
        action: Action::FormFocus(index),
    });
}
