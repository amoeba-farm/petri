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
/// Behind an open modal or ticket nothing does: a click there closes it.
pub(crate) fn register(app: &mut App, area: Rect, action: Action) -> bool {
    let focused = app.modal.is_none() && app.focus == app.hits.len();
    let behind = (app.modal.is_some() || app.markets.ticket.is_some()) && !app.drawing_modal;
    let hovered = !behind
        && app
            .pointer
            .is_some_and(|(x, y)| area.contains((x, y).into()));
    app.hits.push(Hit { area, action });
    focused || hovered
}
/// The Fund view: the oracle balance with Add funds and Withdraw, then the
/// finds (each a button that opens it) and your sources (fund.rs).
fn fund_view(frame: &mut Frame, app: &mut App, screen: Rect, body: Rect) {
    use crate::research::fund::{self, Kind};
    let inner = panel(frame, body, None);
    let content = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let balance = fund::balance_line(&app.fund);
    let x = content.x + balance.chars().count() as u16 + 3;
    label(
        frame,
        within(content, content.y, 1),
        balance,
        Style::default().fg(TEXT),
    );
    // Add funds and Withdraw beside the balance, else under it.
    let mut top = content.y + 2;
    if app.fund.snap.owner.is_some() {
        let row = if x + 23 <= content.right() {
            Rect::new(x, content.y, content.right() - x, 1)
        } else {
            top += 1;
            within(content, content.y + 1, 1)
        };
        buttons(
            frame,
            app,
            row,
            vec![
                ("Add funds", Action::FundTicket(Kind::Deposit), false),
                ("Withdraw", Action::FundTicket(Kind::Withdraw), false),
            ],
        );
    }
    let below = Rect::new(
        content.x,
        top,
        content.width,
        content.bottom().saturating_sub(top),
    );
    let drawn = fund::content(frame, &app.fund, below);
    for (area, title, index) in drawn.finds {
        button(
            frame,
            app,
            area,
            &safe(&title),
            Action::ShowFind(index),
            false,
        );
    }
    // A review's Send, as y or Enter.
    if let Some(at) = drawn.send {
        button(frame, app, at, "Send", Action::FundSend, false);
    }
    if let Some(ticket) = drawn.ticket {
        outside(app, screen, ticket, Action::CloseFundTicket);
    }
}

/// The Bots view: the bots beside the shown one (bots.rs).
fn bots_view(frame: &mut Frame, app: &mut App, body: Rect) {
    use crate::research::bots;
    let mut main = body;
    if body.width >= 72 {
        let shown = bots::sidebar(frame, &app.bots, Rect::new(body.x, body.y, 22, body.height));
        for (index, area) in shown.into_iter().enumerate() {
            register(app, area, Action::Bot(index));
        }
        main = Rect::new(body.x + 25, body.y, body.width - 25, body.height);
    }
    let inner = panel(frame, main, None);
    let content = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let markets = app.markets.snap.markets.clone();
    let drawn = bots::content(frame, &app.bots, &markets, content);
    for (area, action) in drawn.rows {
        register(app, area, action);
    }
    for (area, title, action) in drawn.buttons {
        button(frame, app, area, title, action, false);
    }
}
/// Clicks anywhere on `screen` outside a pop-up's `box_` close it, as on a
/// web page: `close` covers the four sides around the box.
fn outside(app: &mut App, screen: Rect, box_: Rect, close: Action) {
    for area in [
        Rect::new(
            screen.x,
            screen.y,
            screen.width,
            box_.y.saturating_sub(screen.y),
        ),
        Rect::new(
            screen.x,
            box_.bottom(),
            screen.width,
            screen.bottom().saturating_sub(box_.bottom()),
        ),
        Rect::new(
            screen.x,
            box_.y,
            box_.x.saturating_sub(screen.x),
            box_.height,
        ),
        Rect::new(
            box_.right(),
            box_.y,
            screen.right().saturating_sub(box_.right()),
            box_.height,
        ),
    ] {
        if area.width > 0 && area.height > 0 {
            app.hits.push(Hit {
                area,
                action: close.clone(),
            });
        }
    }
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
                if matches!(view, View::Markets | View::Bots) {
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

/// The tabs on `area`'s first row; where even the short labels don't fit,
/// the rest go on its second.
fn tabs(frame: &mut Frame, app: &mut App, area: Rect) {
    let (mut x, mut y) = (area.x, area.y);
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
        let mut width = (title.len() as u16 + 2).min(area.right().saturating_sub(x));
        if width < title.len() as u16 + 2 && x > area.x && y + 1 < area.bottom() {
            (x, y) = (area.x, y + 1);
            width = (title.len() as u16 + 2).min(area.width);
        }
        if width < 3 {
            break;
        }
        let rect = Rect::new(x, y, width, 1);
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
    let content = if app.status.is_empty() && matches!(app.view, View::Markets | View::Bots) {
        let mut spans = Vec::new();
        let hint = if app.view == View::Bots {
            crate::research::bots::footer_hint(&app.bots)
        } else {
            crate::research::markets::footer_hint()
        };
        for (i, (key, label)) in hint.into_iter().enumerate() {
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
            Span::styled("Tab", key),
            Span::styled(" · ", separator),
            Span::styled("Enter", key),
            Span::styled(" · ", separator),
            Span::styled("m/o/c/a/e/r/f", key),
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
    app.drawing_modal = false;
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(TEXT).bg(BG)),
        area,
    );
    if area.width < 45 || area.height < 16 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Petri", accent().add_modifier(Modifier::BOLD)),
                Line::styled("45 × 16 minimum · q Quit", muted()),
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
    // The tabs may take the blank row below them on a narrow terminal.
    tabs(
        frame,
        app,
        Rect::new(rows[2].x, rows[2].y, rows[2].width, 2),
    );
    let mut main = body;
    if app.view == View::Fund {
        fund_view(frame, app, area, body);
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
    if app.view == View::Markets {
        if body.width >= 72 {
            let shown = crate::research::markets::sidebar(
                frame,
                &app.markets,
                Rect::new(body.x, body.y, 22, body.height),
            );
            for (index, area) in shown.into_iter().enumerate() {
                register(app, area, Action::Market(index));
            }
            main = Rect::new(body.x + 25, body.y, body.width - 25, body.height);
        }
        let inner = panel(frame, main, None);
        let content = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        let now = chain_now(app);
        let months = app.fund.snap.months.clone();
        if let Some(ticket) =
            crate::research::markets::content(frame, &app.markets, content, &months, now)
        {
            outside(app, area, ticket, Action::CloseTicket);
        }
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
    if app.view == View::Bots {
        bots_view(frame, app, body);
        footer(frame, app, rows[6]);
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
            let title = format!("{} · {}  next →", safe(&a.harness), safe(&a.name));
            let width = (title.chars().count() as u16 + 2).min(content.width);
            button(
                frame,
                app,
                Rect::new(content.x, content.y, width, 1),
                &title,
                Action::Agent(next),
                !app.new_agent,
            );
            let x = content.x + width + 2;
            if x + 13 <= content.right() {
                button(
                    frame,
                    app,
                    Rect::new(x, content.y, 13, 1),
                    "+ New agent",
                    Action::NewAgent,
                    app.new_agent && app.view == View::Connect,
                );
            }
            content.y += 2;
            content.height = content.height.saturating_sub(2);
        }
    }
    // Evidence: your own proofs first (they need no research workspace),
    // then the agent's finds below them.
    if app.view == View::Evidence {
        let used = proofs_section(frame, app, content);
        let rest = Rect::new(
            content.x,
            content.y + used,
            content.width,
            content.height.saturating_sub(used),
        );
        if app.research_error.is_none() && app.loaded && rest.height > 3 {
            evidence(frame, app, rest);
        }
        if let Some(ticket) = app.proofs.ticket.clone() {
            use crate::research::proofs::Click;
            let drawn = crate::research::proofs::draw_ticket(frame, &ticket, body);
            if let Some(r) = drawn.ticket {
                outside(app, area, r, Action::CloseProof);
                register(app, r, Action::ProofClick(Click::Nothing));
            }
            for (at, click) in drawn.buttons {
                register(app, at, Action::ProofClick(click));
            }
            for (at, title, click) in drawn.footer {
                button(frame, app, at, &title, Action::ProofClick(click), false);
            }
            if let Some(t) = &mut app.proofs.ticket {
                t.scroll = t.scroll.min(drawn.max_scroll);
            }
        }
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
    if let Some(error) = app.research_error.clone() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Research unavailable", strong()),
                Line::styled(safe(&error), muted()),
            ])
            .wrap(Wrap { trim: true }),
            content,
        );
        footer(frame, app, rows[6]);
        return;
    }
    // Until the workspace is first read, nothing rather than an empty one.
    if !app.loaded {
        footer(frame, app, rows[6]);
        return;
    }
    match app.view {
        // Drawn earlier, without the agents beside them.
        View::Markets | View::Bots | View::Fund => {}
        View::Connect => connect(frame, app, content),
        View::Activity => activity(frame, app, content),
        // Drawn above, with your proofs.
        View::Evidence => {}
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
    if !app.loaded {
        return;
    }
    // Each agent is one block of two lines, its harness over its name; the
    // selected one has one bar down its side. "+ New agent" follows them.
    let capacity = usize::from(inner.height.saturating_sub(3) / 3).max(1);
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
    let mut y = inner.y + 1;
    if agents.is_empty() {
        label(
            frame,
            Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
            "No agents yet.",
            muted(),
        );
        y += 2;
    }
    for agent in &agents {
        if y + 2 > inner.bottom() {
            break;
        }
        let selected = !app.new_agent && Some(&agent.id) == app.selected_agent.as_ref();
        let block = Rect::new(inner.x, y, inner.width, 2);
        let focused = register(app, block, Action::Agent(agent.id.clone()));
        let background = if selected {
            SELECTED
        } else if focused {
            PLATE_FOCUS
        } else {
            PANEL
        };
        let bar = Span::styled(if selected { "▌" } else { " " }, accent());
        let harness = if agent.revoked_at.is_some() {
            Style::default().fg(DIM).add_modifier(Modifier::BOLD)
        } else if selected || focused {
            accent().add_modifier(Modifier::BOLD)
        } else {
            strong()
        };
        let mut second = vec![bar.clone(), Span::styled(safe(&agent.name), muted())];
        if agent.revoked_at.is_some() {
            second.push(Span::styled(" · off", Style::default().fg(ALERT)));
        }
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![bar, Span::styled(safe(&agent.harness), harness)]),
                Line::from(second),
            ])
            .style(Style::default().bg(background)),
            block,
        );
        // A dot while it is researching.
        if agent.revoked_at.is_none() && app.activity_of(&agent.id).is_some() {
            frame.buffer_mut().set_string(
                inner.right().saturating_sub(2),
                y,
                "●",
                accent().bg(background),
            );
        }
        y += 3;
    }
    let new = app.new_agent && app.view == View::Connect;
    if y < inner.bottom() {
        button(
            frame,
            app,
            Rect::new(inner.x + 1, y, 13.min(inner.width.saturating_sub(1)), 1),
            "+ New agent",
            Action::NewAgent,
            new,
        );
    }
}

/// Connect: the selected agent, or a fresh setup for a new one ("+ New
/// agent", or when there are none yet).
fn connect(frame: &mut Frame, app: &mut App, area: Rect) {
    let agent = app
        .snapshot
        .agents
        .iter()
        .find(|a| Some(&a.id) == app.selected_agent.as_ref())
        .cloned();
    // Short terminals drop the blank rows between the parts.
    let requests = app.snapshot.pairings.len() as u16;
    let gap = u16::from(area.height >= 10 + 2 * requests.min(2));
    let y = match agent {
        Some(agent) if !app.new_agent => agent_page(frame, app, area, &agent, gap),
        _ => new_agent_page(frame, app, area, gap),
    };
    connection_requests(frame, app, area, y);
}

/// Buttons from row `y`, left to right, onto the next row when one does
/// not fit; returns the row after the last.
fn button_rows(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    mut y: u16,
    values: Vec<(&str, Action, bool)>,
) -> u16 {
    let mut x = area.x;
    for (title, action, active) in values {
        let width = title.chars().count() as u16 + 2;
        if x > area.x && x + width > area.right() {
            x = area.x;
            y += 1;
        }
        if y >= area.bottom() {
            break;
        }
        let width = width.min(area.right().saturating_sub(x));
        button(frame, app, Rect::new(x, y, width, 1), title, action, active);
        x += width + 2;
    }
    y + 1
}

/// A connected agent: what it is doing, the wallet it earns for, Copy
/// start message and Disconnect.
fn agent_page(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    agent: &crate::research::model::Agent,
    gap: u16,
) -> u16 {
    heading(
        frame,
        within(area, area.y, 1),
        &format!("{} · {}", agent.harness, agent.name),
    );
    let mut y = area.y + 1;
    let activity = app.activity_of(&agent.id).cloned();
    let status = if agent.revoked_at.is_some() {
        "Disconnected".to_string()
    } else {
        match &activity {
            Some(run) => format!("Connected · researching {}", work_title(app, run)),
            None => "Connected · idle".into(),
        }
    };
    label(frame, within(area, y, 1), status, muted());
    y += 1;
    if let Some(message) = activity.as_ref().and_then(|a| a.last_message.clone()) {
        label(
            frame,
            within(area, y, 1),
            format!("Last: {}", safe(&message)),
            muted(),
        );
        y += 1;
    }
    y = wallet(frame, app, area, y + gap, &agent.payout_authority, false) + gap;
    if agent.revoked_at.is_none() {
        let mut actions = Vec::new();
        if activity.is_none() {
            actions.push((
                "Copy start message",
                Action::CopyStart(agent.id.clone()),
                false,
            ));
        }
        actions.push(("Disconnect agent", Action::Revoke(agent.id.clone()), false));
        y = button_rows(frame, app, area, y, actions) + gap;
    }
    y
}

/// Setting up an agent: which one, its name, the message to paste into
/// its terminal, and the wallet its rewards go to.
fn new_agent_page(frame: &mut Frame, app: &mut App, area: Rect, gap: u16) -> u16 {
    heading(frame, within(area, area.y, 1), "New agent");
    let agent = format!("Agent: {}", App::HARNESSES[app.harness]);
    let name = format!("Name: {}", app.name);
    let mut y = button_rows(
        frame,
        app,
        area,
        area.y + 1 + gap,
        vec![
            (&agent, Action::ChooseHarness, false),
            (&name, Action::Name, false),
            ("Copy agent setup", Action::CopyCommand, false),
        ],
    ) + gap;
    let connected = app.snapshot.settings.payout_authority.clone();
    y = if connected.is_empty() {
        button_rows(
            frame,
            app,
            area,
            y,
            vec![("Connect wallet", Action::Payout, false)],
        )
    } else {
        wallet(frame, app, area, y, &connected, true)
    };
    y + gap
}

/// "Active wallet" and its address, on one row where they fit; `change`
/// adds a button to connect another (only new agents take it: a connected
/// agent keeps its own). Returns the row after it.
fn wallet(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    y: u16,
    address: &str,
    change: bool,
) -> u16 {
    let title = "Active wallet";
    let address = safe(address);
    let x = area.x + title.len() as u16 + 2;
    let width = address.chars().count() as u16;
    let inline = x + width + if change { 10 } else { 0 } <= area.right();
    label(frame, within(area, y, 1), title, muted());
    let (at, next) = if inline {
        (Rect::new(x, y, width, 1), x + width + 2)
    } else {
        (within(area, y + 1, 1), x)
    };
    label(
        frame,
        at.intersection(area),
        address,
        Style::default().fg(TEXT),
    );
    if change {
        button(
            frame,
            app,
            Rect::new(next, y, 8, 1).intersection(area),
            "Change",
            Action::Payout,
            false,
        );
    }
    y + if inline { 1 } else { 2 }
}

/// Agents asking to connect, each with Approve and Reject (beside it where
/// they fit), from row `y`.
fn connection_requests(frame: &mut Frame, app: &mut App, area: Rect, mut y: u16) {
    let pairings = app.snapshot.pairings.clone();
    if pairings.is_empty() {
        return;
    }
    app.scroll = app.scroll.min(pairings.len().saturating_sub(1) as u16);
    for pair in pairings.iter().skip(usize::from(app.scroll)) {
        let request = pairing_line(
            format!("{} / {}", safe(&pair.name), safe(&pair.harness)),
            &pair.code,
        );
        let x = area.x + request.width() as u16 + 2;
        let inline = x + 19 <= area.right();
        let rows = if inline { 1 } else { 2 };
        if y + rows > area.bottom() {
            break;
        }
        frame.render_widget(Paragraph::new(request), within(area, y, 1));
        if pair.status != "approving" {
            buttons(
                frame,
                app,
                if inline {
                    Rect::new(x, y, area.right() - x, 1)
                } else {
                    within(area, y + 1, 1)
                },
                vec![
                    ("Approve", Action::Approve(pair.id.clone()), false),
                    ("Reject", Action::Reject(pair.id.clone()), false),
                ],
            );
        }
        y += rows;
        if let Some(error) = &pair.error {
            label(frame, within(area, y, 1), safe(error), muted());
            y += 1;
        }
    }
}

/// The product an agent is researching, as the list names it.
fn work_title(app: &App, run: &crate::research::model::Activity) -> String {
    app.snapshot
        .work
        .iter()
        .find(|w| w.id == run.work_id)
        .map(gap_title)
        .unwrap_or_else(|| safe(&run.title))
}

/// Research: the navigator beside the selected agent's page
/// (research_page.rs).
fn activity(frame: &mut Frame, app: &mut App, area: Rect) {
    crate::research::research_page::draw(frame, app, area);
}

/// A coverage gap's potential earnings, as a temperature: the share of its
/// target sources (and distinct hosts, once known) still missing. More
/// missing is more to earn; a gap no longer open has nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heat {
    Hot,
    Warm,
    Lukewarm,
    Cold,
}

impl Heat {
    pub(crate) fn of(job: &crate::research::model::Work) -> Self {
        if !(job.availability == "available" || job.availability.is_empty()) {
            return Heat::Cold;
        }
        let objective = &job.objective;
        // An opening or update earns from its rewards, if there are any.
        if let Some(earns) = staked_earns(objective) {
            return if earns { Heat::Warm } else { Heat::Cold };
        }
        let short = |have: &serde_json::Value, want: &serde_json::Value| {
            let want = want.as_f64().unwrap_or(0.0);
            if want <= 0.0 {
                return 0.0;
            }
            ((want - have.as_f64().unwrap_or(0.0)) / want).clamp(0.0, 1.0)
        };
        let mut need = short(
            &objective["activeSources"],
            &objective["targets"]["activeSources"],
        );
        if objective["distinctHosts"].is_number() {
            need = need.max(short(
                &objective["distinctHosts"],
                &objective["targets"]["distinctHosts"],
            ));
        }
        match need {
            n if n >= 0.75 => Heat::Hot,
            n if n >= 0.5 => Heat::Warm,
            n if n >= 0.25 => Heat::Lukewarm,
            _ => Heat::Cold,
        }
    }

    pub(crate) fn word(self) -> &'static str {
        match self {
            Heat::Hot => "hot",
            Heat::Warm => "warm",
            Heat::Lukewarm => "lukewarm",
            Heat::Cold => "cold",
        }
    }

    /// The box's color now: red through blue. Hot and warm flicker like a
    /// flame (a jump between shades); lukewarm and cold shimmer like ice (a
    /// drift through them). `tick` moves about seven times a second.
    pub(crate) fn color(self, tick: u64, row: usize) -> Color {
        let shades: &[(u8, u8, u8)] = match self {
            Heat::Hot => &[(255, 69, 0), (255, 40, 25), (255, 120, 20), (230, 30, 30)],
            Heat::Warm => &[(255, 140, 0), (255, 165, 40), (240, 120, 20)],
            Heat::Lukewarm => &[(120, 200, 190), (150, 215, 205), (100, 185, 180)],
            Heat::Cold => &[
                (70, 140, 255),
                (120, 180, 255),
                (190, 225, 255),
                (95, 160, 255),
            ],
        };
        let step = match self {
            Heat::Hot | Heat::Warm => {
                (tick.wrapping_mul(2_654_435_761) ^ (row as u64).wrapping_mul(40_503)) >> 5
            }
            Heat::Lukewarm | Heat::Cold => tick / 2 + row as u64,
        };
        let (r, g, b) = shades[(step % shades.len() as u64) as usize];
        Color::Rgb(r, g, b)
    }
}

/// Whether staked work earns: an update from its month's remaining rewards,
/// an opening from its product's opening budget (none when not known). None
/// for other work.
pub(crate) fn staked_earns(objective: &serde_json::Value) -> Option<bool> {
    let field = match objective["kind"].as_str()? {
        "update" => "rewardsRemainingAtomic",
        "opening" => "openingBudgetAtomic",
        _ => return None,
    };
    Some(
        objective[field]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|left| left > 0),
    )
}

/// A gap as a person reads it: market, month and product, like
/// "RAMX · Dec 2026 · DDR5 16Gb". Work from before objectives keeps its title.
pub(crate) fn gap_title(job: &crate::research::model::Work) -> String {
    let objective = &job.objective;
    let (Some(market), Some(month)) = (
        objective["marketId"].as_str(),
        objective["expiryId"].as_str().and_then(month_name),
    ) else {
        return safe(&job.title);
    };
    let product = match (
        objective["bucketLabel"].as_str().filter(|l| !l.is_empty()),
        objective["bucketId"].as_str(),
    ) {
        (Some(label), _) => label.to_string(),
        (None, Some(id)) => format!("Unnamed product {}", &id[..id.len().min(6)]),
        // A gap across the whole month: no product has a source yet.
        (None, None) => "All products".into(),
    };
    // An opening or update names its source's page.
    let page = objective["kind"]
        .as_str()
        .filter(|kind| matches!(*kind, "opening" | "update"))
        .and_then(|kind| Some((kind, objective["url"].as_str().and_then(host)?)))
        .map(|(kind, h)| format!(" · {kind} {h}"))
        .unwrap_or_default();
    safe(&format!(
        "{} · {month} · {product}{page}",
        market.to_uppercase()
    ))
}

/// A page's host without `www.`.
fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(host.trim_start_matches("www.").to_string())
}

/// "RAMX-202612" is "Dec 2026".
pub(super) fn month_name(expiry: &str) -> Option<String> {
    let digits = expiry.rsplit('-').next()?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month: usize = digits[4..].parse().ok()?;
    Some(format!(
        "{} {}",
        MONTHS.get(month.checked_sub(1)?)?,
        &digits[..4]
    ))
}

/// The chain's clock (a dev chain's runs slow), else this computer's.
pub(crate) fn chain_now(app: &App) -> i64 {
    app.fund.snap.chain_now.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    })
}

/// "Your proofs": Sign in to a site, Prove a page, Import a proof, then
/// each proof here as a button that opens it (proofs.rs). Returns the rows
/// it took.
fn proofs_section(frame: &mut Frame, app: &mut App, area: Rect) -> u16 {
    use crate::research::proofs::{Kind, row_text};
    label(frame, within(area, area.y, 1), "Your proofs", strong());
    let actions = vec![
        (
            "Sign in to a site",
            Action::ProofTicket(Kind::SignIn),
            false,
        ),
        ("Prove a page", Action::ProofTicket(Kind::Prove), false),
        ("Import a proof", Action::ProofTicket(Kind::Import), false),
    ];
    let mut y = area.y;
    let x = area.x + 13;
    if x + 54 <= area.right() {
        buttons(frame, app, Rect::new(x, y, area.right() - x, 1), actions);
    } else {
        y += 1;
        buttons(frame, app, within(area, y, 1), actions);
    }
    y += 2;
    let list = app.proofs.list.clone();
    if list.is_empty() {
        label(
            frame,
            within(area, y, 1),
            "None yet. A web proof shows what a site showed you, signed-in pages too.",
            muted(),
        );
        y += 1;
    }
    // Six rows of yours, leaving room for the agent's finds below: ▴/▾ rows
    // (a click, or Tab and Enter) and the wheel over the list scroll through
    // the rest, as the navigator's list does.
    let h = usize::from(area.bottom().saturating_sub(y).min(6));
    let n = list.len();
    let start = if n <= h {
        0
    } else {
        app.proofs.scroll.min(n - h.saturating_sub(1).max(1))
    };
    app.proofs.scroll = start;
    app.proofs.area = Rect::new(area.x, y, area.width, h.min(n) as u16);
    let above = start > 0;
    let mut rows = h.saturating_sub(usize::from(above));
    let below = n - start > rows;
    if below {
        rows = rows.saturating_sub(1);
    }
    let more = |frame: &mut Frame, app: &mut App, y: u16, text: String, to: usize| {
        if y < area.bottom() {
            let width = (text.chars().count() as u16 + 2).min(area.width);
            button(
                frame,
                app,
                Rect::new(area.x, y, width, 1),
                &text,
                Action::ProofScroll(to),
                false,
            );
        }
    };
    if above {
        more(
            frame,
            app,
            y,
            format!("▴ {start} more"),
            start.saturating_sub(rows.max(1)),
        );
        y += 1;
    }
    for (index, entry) in list.iter().enumerate().skip(start).take(rows) {
        let text = safe(&row_text(entry));
        let width = (text.chars().count() as u16 + 2).min(area.width);
        button(
            frame,
            app,
            Rect::new(area.x, y, width, 1),
            &text,
            Action::ShowProof(index),
            false,
        );
        y += 1;
    }
    if below {
        more(
            frame,
            app,
            y,
            format!("▾ {} more", n - start - rows),
            start + rows,
        );
        y += 1;
    }
    y += 1;
    if !app.snapshot.agents.is_empty() && y < area.bottom() {
        label(frame, within(area, y, 1), "Your agent's finds", strong());
        y += 2;
    }
    y.saturating_sub(area.y).min(area.height)
}

fn evidence(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(detail) = app.selected().cloned() else {
        label(frame, area, "No agents yet.", muted());
        return;
    };
    let sources = app.sources();
    if sources.is_empty() {
        label(
            frame,
            area,
            detail
                .validation_error
                .unwrap_or_else(|| "No evidence yet.".into()),
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
    // "1 of 3" after the two buttons.
    let count = format!("{} of {}", app.source + 1, sources.len());
    frame.buffer_mut().set_stringn(
        area.x + 24,
        area.y,
        &count,
        usize::from(area.right().saturating_sub(area.x + 24)),
        muted(),
    );
    let mut lines = Vec::new();
    for (title, key) in [
        ("URL", "canonicalUrl"),
        ("Product", "productDefinition"),
        ("Price field", "priceField"),
        ("Region", "region"),
        ("Currency", "currency"),
        ("Evidence", "evidence"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("{title:<13}"), muted()),
            Span::raw(field(data, key)),
        ]));
    }
    if let Some(error) = detail.validation_error {
        lines.push(line(""));
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

/// Rewards: what the agent's finds earned it, month by month as a chart,
/// then each sponsored find and what became of it (rewards.rs).
fn rewards(frame: &mut Frame, app: &mut App, area: Rect) {
    use crate::research::rewards::by_month;
    use ratatui::widgets::{Bar, BarChart, BarGroup};
    let Some(agent) = app
        .snapshot
        .agents
        .iter()
        .find(|a| Some(&a.id) == app.selected_agent.as_ref())
        .cloned()
    else {
        label(frame, area, "No agents yet.", muted());
        return;
    };
    // Nothing is said about earnings until they are read.
    let read = app.rewards.by_agent.get(&agent.id).cloned();
    let earnings = read.clone().unwrap_or_default();
    let total: u64 = earnings.iter().map(|e| e.earned).sum();
    let atoms =
        |value: u64| usdc(&serde_json::Value::String(value.to_string())).unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                safe(&format!("{} · {}", agent.harness, agent.name)),
                strong(),
            ),
            Span::styled(if app.rewards.offline { "  offline" } else { "" }, muted()),
        ])),
        within(area, area.y, 1),
    );
    // "Earned  3.5 USDC   4 finds · 2 sponsored".
    let submitted = app.sources().len();
    let finds = if submitted == 1 { "find" } else { "finds" };
    let mut figures = Vec::new();
    if read.is_some() {
        figures.push(Span::styled("Earned  ", muted()));
        figures.push(Span::styled(atoms(total), strong()));
        figures.push(Span::raw("   "));
    }
    figures.push(Span::styled(format!("{submitted} {finds}"), muted()));
    if read.is_some() {
        figures.push(Span::styled(
            format!(" · {} sponsored", earnings.len()),
            muted(),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(figures)),
        within(area, area.y + 1, 1),
    );
    // The chart, once something earns: a bar per oracle month, its height
    // what that month paid.
    let months = by_month(&earnings);
    let mut below = area.y + 3;
    // Room kept under it for the list's heading and a few of its rows.
    let list = 2 + earnings.len().min(3) as u16;
    let chart = Rect::new(
        area.x,
        below,
        area.width,
        9.min(area.bottom().saturating_sub(below + list)),
    );
    if !months.is_empty() && chart.height >= 4 {
        let bars: Vec<Bar> = months
            .iter()
            .map(|(month, value)| {
                Bar::default()
                    .value(*value)
                    .text_value(atoms(*value).trim_end_matches(" USDC").to_string())
                    .label(Line::from(month.clone()))
                    .style(accent())
                    .value_style(Style::default().fg(BG).bg(ACCENT))
            })
            .collect();
        frame.render_widget(
            BarChart::default()
                .block(
                    Block::default()
                        .borders(Borders::LEFT | Borders::BOTTOM)
                        .border_style(Style::default().fg(BORDER))
                        .title(Line::styled("USDC per month", muted())),
                )
                .data(BarGroup::default().bars(&bars))
                .bar_width(8)
                .bar_gap(3)
                .max(months.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1))
                .label_style(muted()),
            chart,
        );
        below = chart.bottom() + 1;
    }
    if earnings.is_empty() {
        return;
    }
    let mut lines = vec![subdued("Sponsored finds")];
    for e in &earnings {
        lines.push(Line::from(vec![
            Span::styled(safe(&e.title), Style::default().fg(TEXT)),
            Span::styled(format!("  · {}", e.outcome), muted()),
            Span::styled(
                if e.earned > 0 {
                    format!("  · {}", atoms(e.earned))
                } else {
                    String::new()
                },
                accent(),
            ),
        ]));
    }
    paragraph(
        frame,
        app,
        Rect::new(
            area.x,
            below,
            area.width,
            area.bottom().saturating_sub(below),
        ),
        lines,
    );
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
    app.drawing_modal = true;
    let screen = frame.area();
    scrim(frame.buffer_mut(), screen);
    let width = parent.width.saturating_sub(4).min(80);
    let height = parent.height.saturating_sub(2).min(match modal {
        Modal::Refine { .. } | Modal::Find(_) => 21,
        Modal::Paste(_) => 14,
        _ => 13,
    });
    let area = Rect::new(
        parent.x + (parent.width - width) / 2,
        parent.y + (parent.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    outside(app, screen, area, Action::Cancel);
    let title = match &modal {
        Modal::Payout(_) => " CONNECT WALLET ",
        Modal::Name(_) => " AGENT NAME ",
        Modal::Harness => " YOUR AGENT ",
        Modal::Find(_) => " FOUND BY A RESEARCHER ",
        Modal::Refine { .. } => " DIRECTIVE FOR YOUR AGENT ",
        Modal::Revoke(_) => " DISCONNECT AGENT ",
        Modal::Paste(_) => " PASTE INTO YOUR AGENT ",
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
        Modal::Payout(value) => {
            input(
                frame,
                app,
                Rect::new(body.x, body.y + 1, body.width, 3),
                &value,
                0,
            );
            // This Petri's own wallet, offered first.
            if app.snapshot.settings.payout_authority.is_empty()
                && app.markets.snap.owner.as_deref() == Some(value.as_str())
            {
                label(frame, within(body, body.y + 4, 1), "Petri wallet", muted());
            }
        }
        Modal::Name(value) => {
            input(
                frame,
                app,
                Rect::new(body.x, body.y + 1, body.width, 3),
                &value,
                0,
            );
        }
        Modal::Find(find) => {
            let field = |name: &str, value: &str| {
                Line::from(vec![
                    Span::styled(format!("{name:<11}"), muted()),
                    Span::styled(safe(value), Style::default().fg(TEXT)),
                ])
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mine = app
                .snapshot
                .agents
                .iter()
                .find(|a| a.id == find.agent)
                .map(|a| format!("your agent {}", a.name));
            let by = mine.unwrap_or_else(|| {
                format!("a researcher ({})", &find.agent[..find.agent.len().min(8)])
            });
            let lines = vec![
                field("Price page", &find.url),
                field("Product", &find.definition),
                field("Priced as", &find.price),
                field("Evidence", &find.evidence),
                field(
                    "Found by",
                    &format!(
                        "{by} · {}",
                        crate::research::fund::ago(now, find.created_ms)
                    ),
                ),
                field(
                    "Oracle",
                    &find
                        .on_oracle
                        .as_deref()
                        .map_or("not yet".into(), str::to_lowercase),
                ),
            ];
            // On a chip month: what the spec makes of its series.
            let mut lines = lines;
            if let Some(checked) = &find.checked {
                let words = match &checked.reason {
                    Some(reason) => format!("{}: {reason}", checked.status.summary()),
                    None => checked.status.summary().to_string(),
                };
                lines.push(field("Checked", &words));
            }
            label(
                frame,
                within(body, body.y, 1),
                safe(&find.title),
                accent().add_modifier(Modifier::BOLD),
            );
            paragraph(
                frame,
                app,
                within(body, body.y + 2, body.height.saturating_sub(2)),
                lines,
            );
        }
        Modal::Harness => {
            for (index, harness) in App::HARNESSES.iter().enumerate() {
                let y = body.y + index as u16;
                if y >= body.bottom() {
                    break;
                }
                let mark = if index == app.harness { "●" } else { "○" };
                button(
                    frame,
                    app,
                    Rect::new(body.x, y, body.width.min(32), 1),
                    &format!("{mark} {harness}"),
                    Action::Harness(index),
                    index == app.harness,
                );
            }
            label(
                frame,
                within(body, body.bottom().saturating_sub(1), 1),
                "↑/↓ · Enter",
                muted(),
            );
        }
        Modal::Refine {
            instruction,
            work_id,
        } => {
            // Which work it is for; the agent reads it before it starts.
            let title = app
                .snapshot
                .work
                .iter()
                .find(|w| w.id == work_id)
                .map(|w| crate::research::nav::short_title(&w.objective, &w.title))
                .unwrap_or_default();
            label(frame, within(body, body.y, 1), title, muted());
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
        Modal::Paste(start) => {
            // For a connected agent (its start message) or the new one.
            let agent = start
                .as_ref()
                .and_then(|id| app.snapshot.agents.iter().find(|a| &a.id == id))
                .cloned();
            let harness = safe(
                agent
                    .as_ref()
                    .map_or(App::HARNESSES[app.harness], |a| a.harness.as_str()),
            );
            let keys = if cfg!(target_os = "macos") {
                "⌘V"
            } else if cfg!(windows) {
                "Ctrl+V"
            } else {
                "Ctrl+Shift+V"
            };
            let steps = match &agent {
                Some(agent) => [
                    format!("Open your chat with {} in {harness}.", safe(&agent.name)),
                    format!("Paste it ({keys}) and send it."),
                    "It starts researching. Its page here shows what it does.".into(),
                ],
                None => [
                    format!("Open a chat with {harness}."),
                    format!("Paste it ({keys}) and send it."),
                    format!("{harness} shows a pairing code. Approve the same code here."),
                ],
            };
            label(
                frame,
                within(body, body.y, 1),
                "Copied to your clipboard.",
                strong(),
            );
            // Each step's number, its words beside it (wrapped under them).
            let text = body.width.saturating_sub(3);
            let mut y = body.y + 2;
            for (index, step) in steps.into_iter().enumerate() {
                if y >= body.bottom() {
                    break;
                }
                label(
                    frame,
                    within(body, y, 1),
                    format!("{}", index + 1),
                    accent().add_modifier(Modifier::BOLD),
                );
                let rows = crate::research::fund::wrapped_rows(&step, usize::from(text)) as u16;
                let area = Rect::new(body.x + 3, y, text, rows).intersection(body);
                label(frame, area, step, Style::default().fg(TEXT));
                y += rows;
            }
        }
        Modal::Revoke(agent) => {
            frame.render_widget(
                Paragraph::new(format!("Disconnect {}?", safe(&agent.name))),
                body,
            );
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
    let values = if let Some(Modal::Find(find)) = &app.modal
        && app.find_sponsorable(find)
    {
        vec![
            (
                "Sponsor it",
                Action::Sponsor(find.content_hash.clone()),
                app.form_focus == 0,
            ),
            ("Close", Action::Cancel, app.form_focus == 1),
        ]
    } else if let Some(Modal::Paste(start)) = &app.modal {
        let again = match start {
            Some(id) => Action::CopyStart(id.clone()),
            None => Action::CopyCommand,
        };
        vec![
            ("Done", Action::Cancel, app.form_focus == 0),
            ("Copy again", again, app.form_focus == 1),
        ]
    } else if matches!(app.modal, Some(Modal::Find(_) | Modal::Harness)) {
        vec![("Close", Action::Cancel, true)]
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
