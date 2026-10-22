//! The Research view: a navigator (products › months › categories › one
//! category) beside the selected agent's page (what it is on and its live
//! feed). The navigator follows the agent's work until a click moves it
//! (nav.rs builds what both show).

use super::model::{Action, Agent, App, Work, safe};
use super::nav::{self, ALL, CategoryRow, Path, Tone};
use super::render::{
    ACCENT, ALERT, BG, DIM, Heat, MUTED, PLATE, PLATE_FOCUS, SELECTED, TEXT, chain_now, month_name,
    register,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

fn style(color: ratatui::style::Color) -> Style {
    Style::default().fg(color)
}
fn bold(color: ratatui::style::Color) -> Style {
    style(color).add_modifier(Modifier::BOLD)
}
fn width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// `spans` cut to `room` cells, the last one ending in "…" when cut.
fn fit(spans: Vec<Span<'static>>, room: usize) -> Vec<Span<'static>> {
    if width(&spans) <= room {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.width();
        if used + w < room {
            used += w;
            out.push(span);
            continue;
        }
        let mut text = String::new();
        for c in span.content.chars() {
            let cw = Span::raw(c.to_string()).width();
            if used + cw + 1 > room {
                break;
            }
            used += cw;
            text.push(c);
        }
        if room > 0 {
            text.push('…');
        }
        out.push(Span::styled(text, span.style));
        break;
    }
    out
}

/// A button, as the rest of the TUI draws them.
fn button(frame: &mut Frame, app: &mut App, area: Rect, title: &str, action: Action, on: bool) {
    if area.width < 3 || area.height == 0 {
        return;
    }
    let focused = register(app, area, action);
    let look = if on {
        bold(TEXT).bg(SELECTED)
    } else if focused {
        bold(ACCENT).bg(PLATE_FOCUS)
    } else {
        style(TEXT).bg(PLATE)
    };
    frame.render_widget(Paragraph::new(format!("[{title}]")).style(look), area);
}

/// Buttons in a row from `area`'s left.
fn buttons(frame: &mut Frame, app: &mut App, area: Rect, values: Vec<(&str, Action, bool)>) {
    let mut x = area.x;
    for (title, action, on) in values {
        let w = title.chars().count() as u16 + 2;
        if x + w > area.right() {
            break;
        }
        button(frame, app, Rect::new(x, area.y, w, 1), title, action, on);
        x += w + 2;
    }
}

/// One clickable row: `left` from its start, `right` against its end; `lit`
/// marks the followed agent's place.
fn row(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    action: Action,
    lit: bool,
) {
    let hovered = register(app, area, action);
    let bg = if hovered {
        PLATE_FOCUS
    } else if lit {
        SELECTED
    } else {
        BG
    };
    let room = usize::from(area.width.saturating_sub(1));
    let right = fit(right, room);
    let rw = width(&right);
    let left = fit(left, room.saturating_sub(rw + usize::from(rw > 0)));
    let mut spans = vec![Span::styled(if lit { "▌" } else { " " }, style(ACCENT))];
    let lw = width(&left);
    spans.extend(left);
    spans.push(Span::raw(" ".repeat(room.saturating_sub(lw + rw))));
    spans.extend(right);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        area,
    );
}

/// "Codex", or the agent's name when two share a harness; "+1" for more.
fn who(app: &App, ids: &[String]) -> String {
    let Some(agent) = ids
        .first()
        .and_then(|id| app.snapshot.agents.iter().find(|a| &a.id == id))
    else {
        return String::new();
    };
    let twin = app
        .snapshot
        .agents
        .iter()
        .filter(|a| a.harness == agent.harness)
        .count()
        > 1;
    let name = safe(if twin { &agent.name } else { &agent.harness });
    match ids.len() {
        1 => name,
        n => format!("{name} +{}", n - 1),
    }
}

/// "● Codex" for the agents on it, "○ pick" for your pick nobody is on;
/// `short`, just the dots.
fn marker(app: &App, agents: &[String], focused: bool, short: bool) -> Vec<Span<'static>> {
    if !agents.is_empty() {
        let text = if short {
            "●".repeat(agents.len().min(3))
        } else {
            format!("● {}", who(app, agents))
        };
        vec![Span::styled(text, style(ACCENT))]
    } else if focused {
        vec![Span::styled(
            if short { "○" } else { "○ pick" },
            style(MUTED),
        )]
    } else {
        vec![]
    }
}

/// A row's right side: its task count, then its marker, each padded to
/// the list's widest so they line up.
fn tail(tasks: usize, mark: Vec<Span<'static>>, counts: usize, marks: usize) -> Vec<Span<'static>> {
    let count = if tasks == 0 {
        String::new()
    } else {
        plural(tasks, "task", "tasks")
    };
    let mut right = vec![Span::styled(format!("{count:>counts$}"), style(MUTED))];
    if marks > 0 {
        let pad = marks - width(&mark);
        right.push(Span::raw("  "));
        right.extend(mark);
        right.push(Span::raw(" ".repeat(pad)));
    }
    right
}

/// The heat box, flickering by `tick`.
fn heat_box(heat: Option<Heat>, tick: u64, index: usize) -> Span<'static> {
    match heat {
        Some(h) => Span::styled("██", style(h.color(tick, index))),
        None => Span::raw("  "),
    }
}

fn tick() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 140) as u64)
        .unwrap_or(0)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The selected agent, if there is one to show.
fn shown_agent(app: &App) -> Option<Agent> {
    app.snapshot
        .agents
        .iter()
        .find(|a| Some(&a.id) == app.selected_agent.as_ref())
        .cloned()
}

/// The Research view in `area`: the crumbs over the navigator and the
/// agent's page, side by side from about 100 columns, else stacked.
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.follow();
    app.feed_area = Rect::default();
    app.feed_max = 0;
    if area.height < 3 || area.width < 10 {
        return;
    }
    let agent = shown_agent(app);
    crumbs(
        frame,
        app,
        Rect::new(area.x, area.y, area.width, 1),
        agent.is_some(),
    );
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    let Some(agent) = agent else {
        navigator(frame, app, body);
        return;
    };
    if body.width >= 65 {
        let left = ((body.width - 3) / 2).max(30);
        let rule = body.x + left + 1;
        for y in body.y..body.bottom() {
            frame.buffer_mut().set_string(rule, y, "│", style(DIM));
        }
        navigator(frame, app, Rect::new(body.x, body.y, left, body.height));
        let x = rule + 2;
        page(
            frame,
            app,
            Rect::new(x, body.y, body.right().saturating_sub(x), body.height),
            &agent,
        );
    } else {
        // The navigator takes what it needs, up to half; the feed the rest.
        let top = (wanted(app) as u16)
            .clamp(3, (body.height / 2).max(3))
            .min(body.height);
        navigator(frame, app, Rect::new(body.x, body.y, body.width, top));
        if body.height > top + 2 {
            frame.buffer_mut().set_string(
                body.x,
                body.y + top,
                "─".repeat(usize::from(body.width)),
                style(DIM),
            );
            page(
                frame,
                app,
                Rect::new(body.x, body.y + top + 1, body.width, body.height - top - 1),
                &agent,
            );
        }
    }
}

/// About how many rows the navigator shows where it is.
fn wanted(app: &App) -> usize {
    let path = app.nav().path;
    match (&path.market, &path.expiry, &path.bucket) {
        (None, ..) => nav::products(app).len(),
        (Some(market), None, _) => nav::months(app, market).len(),
        (Some(market), Some(expiry), None) => nav::categories(app, market, expiry).len(),
        (Some(market), Some(expiry), Some(bucket)) => nav::categories(app, market, expiry)
            .into_iter()
            .find(|r| &r.bucket == bucket)
            .map_or(1, |r| 4 + r.tasks.len().max(r.sources as usize)),
    }
}

/// "Products › RAMX › Dec 2026 › DDR5 RDIMM 64GB", each crumb a way back
/// up, and Follow at the end while there is an agent to follow.
fn crumbs(frame: &mut Frame, app: &mut App, area: Rect, follow: bool) {
    let nav = app.nav();
    let path = nav.path.clone();
    let mut parts = vec![("Products".to_string(), Path::default())];
    if let Some(market) = &path.market {
        parts.push((market.to_uppercase(), Path::product(market)));
        if let Some(expiry) = &path.expiry {
            parts.push((
                month_name(expiry).unwrap_or_else(|| safe(expiry)),
                Path::month(market, expiry),
            ));
            if let Some(bucket) = &path.bucket {
                let label = nav::categories(app, market, expiry)
                    .into_iter()
                    .find(|r| &r.bucket == bucket)
                    .map(|r| r.label)
                    .unwrap_or_else(|| nav::bucket_label(bucket, None));
                parts.push((label, path.clone()));
            }
        }
    }
    let end = if follow {
        let w = 8;
        let x = area.right().saturating_sub(w);
        button(
            frame,
            app,
            Rect::new(x, area.y, w, 1),
            "Follow",
            Action::Follow,
            nav.follow,
        );
        x.saturating_sub(1)
    } else {
        area.right()
    };
    let last = parts.len() - 1;
    let mut x = area.x;
    for (i, (title, to)) in parts.into_iter().enumerate() {
        if i > 0 {
            if x + 3 >= end {
                break;
            }
            frame.buffer_mut().set_string(x, area.y, " › ", style(DIM));
            x += 3;
        }
        let room = usize::from(end.saturating_sub(x));
        if room < 2 {
            break;
        }
        let text = fit(vec![Span::raw(title)], room)
            .pop()
            .map(|s| s.content.to_string())
            .unwrap_or_default();
        let w = Span::raw(text.clone()).width() as u16;
        let at = Rect::new(x, area.y, w, 1);
        let hovered = register(app, at, Action::Nav(to));
        let look = if i == last {
            bold(TEXT)
        } else if hovered {
            bold(ACCENT).bg(PLATE_FOCUS)
        } else {
            style(MUTED)
        };
        frame.render_widget(Paragraph::new(text).style(look), at);
        x += w;
    }
}

/// One row of a navigator list.
struct Item {
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    action: Action,
    lit: bool,
}

/// A list of rows from `app.scroll`, with ▴/▾ rows that scroll it when it
/// does not fit. `build` makes them, with short markers when the full ones
/// would cut a row's name.
fn list(frame: &mut Frame, app: &mut App, area: Rect, build: impl Fn(&App, bool) -> Vec<Item>) {
    let h = usize::from(area.height);
    if h == 0 {
        return;
    }
    let room = usize::from(area.width.saturating_sub(1));
    let mut items = build(app, false);
    if items
        .iter()
        .any(|i| width(&i.left) + width(&i.right) + 2 > room)
    {
        items = build(app, true);
    }
    let n = items.len();
    let mut start = usize::from(app.scroll);
    if n <= h {
        start = 0;
    } else {
        start = start.min(n - h.saturating_sub(1).max(1));
    }
    app.scroll = start as u16;
    let above = start > 0;
    let mut rows = h - usize::from(above);
    let below = n - start > rows;
    if below {
        rows = rows.saturating_sub(1);
    }
    let mut y = area.y;
    let more = |frame: &mut Frame, app: &mut App, y: u16, text: String, to: usize| {
        row(
            frame,
            app,
            Rect::new(area.x, y, area.width, 1),
            vec![Span::styled(text, style(DIM))],
            vec![],
            Action::Scroll(to as u16),
            false,
        );
    };
    if above {
        let to = start.saturating_sub(rows.max(1));
        more(frame, app, y, format!("▴ {start} more"), to);
        y += 1;
    }
    for item in items.into_iter().skip(start).take(rows) {
        row(
            frame,
            app,
            Rect::new(area.x, y, area.width, 1),
            item.left,
            item.right,
            item.action,
            item.lit,
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
    }
}

/// Where the followed agent is, to light its rows.
fn followed(app: &App) -> Option<Path> {
    let id = app.selected_agent.as_deref()?;
    nav::place_of(app, id).map(|(p, _)| p)
}

fn navigator(frame: &mut Frame, app: &mut App, area: Rect) {
    let path = app.nav().path;
    let here = followed(app);
    match (&path.market, &path.expiry, &path.bucket) {
        (None, ..) => products(frame, app, area, here),
        (Some(market), None, _) => months(frame, app, area, market, here),
        (Some(market), Some(expiry), None) => categories(frame, app, area, market, expiry, here),
        (Some(market), Some(expiry), Some(bucket)) => {
            category(frame, app, area, market, expiry, bucket)
        }
    }
}

fn products(frame: &mut Frame, app: &mut App, area: Rect, here: Option<Path>) {
    if nav::products(app).is_empty() {
        // The products are the oracle's months: nothing until they are read.
        let text = if app.fund.offline || app.snapshot.discovery["error"].is_string() {
            "offline"
        } else if !app.fund.loaded {
            ""
        } else {
            "No products"
        };
        frame.render_widget(
            Paragraph::new(format!(" {text}")).style(style(MUTED)),
            Rect::new(area.x, area.y, area.width, 1.min(area.height)),
        );
        return;
    }
    list(frame, app, area, |app, short| {
        let rows = nav::products(app);
        let name = rows.iter().map(|r| r.market.len()).max().unwrap_or(0) + 2;
        let counts = rows
            .iter()
            .map(|r| plural(r.tasks, "task", "tasks").len())
            .max()
            .unwrap_or(0);
        let marks = rows
            .iter()
            .map(|r| width(&marker(app, &r.agents, false, short)))
            .max()
            .unwrap_or(0);
        rows.into_iter()
            .map(|r| {
                let quiet = r.tasks == 0;
                let right = tail(r.tasks, marker(app, &r.agents, false, short), counts, marks);
                Item {
                    left: vec![
                        Span::styled(
                            format!("{:<name$}", r.market.to_uppercase()),
                            if quiet { style(MUTED) } else { bold(TEXT) },
                        ),
                        Span::styled(plural(r.months, "month", "months"), style(MUTED)),
                    ],
                    right,
                    lit: here.as_ref().and_then(|p| p.market.as_ref()) == Some(&r.market),
                    action: Action::Nav(Path::product(&r.market)),
                }
            })
            .collect()
    });
}

fn months(frame: &mut Frame, app: &mut App, area: Rect, market: &str, here: Option<Path>) {
    let now = chain_now(app);
    list(frame, app, area, |app, short| {
        let rows = nav::months(app, market);
        let counts = rows
            .iter()
            .map(|r| plural(r.tasks, "task", "tasks").len())
            .max()
            .unwrap_or(0);
        let marks = rows
            .iter()
            .map(|r| width(&marker(app, &r.agents, false, short)))
            .max()
            .unwrap_or(0);
        rows.into_iter()
            .map(|r| {
                let quiet = r.tasks == 0;
                let mut left = vec![Span::styled(
                    format!(
                        "{:<10}",
                        month_name(&r.expiry).unwrap_or_else(|| safe(&r.expiry))
                    ),
                    if quiet { style(MUTED) } else { bold(TEXT) },
                )];
                left.extend(r.month.strip(now));
                if let Some(rewards) = r.month.rewards_left() {
                    left.push(Span::styled(format!("  {rewards}"), style(ACCENT)));
                }
                let right = tail(r.tasks, marker(app, &r.agents, false, short), counts, marks);
                Item {
                    left,
                    right,
                    lit: here.as_ref().is_some_and(|p| {
                        p.market.as_deref() == Some(market) && p.expiry.as_ref() == Some(&r.expiry)
                    }),
                    action: Action::Nav(Path::month(market, &r.expiry)),
                }
            })
            .collect()
    });
}

/// "1/3 src" for a source gap, "2 src" otherwise; nothing without any.
fn count(row: &CategoryRow) -> String {
    match row.target {
        Some(target) => format!("{}/{target} src", row.sources),
        None if row.sources > 0 => format!("{} src", row.sources),
        None => String::new(),
    }
}

fn categories(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    market: &str,
    expiry: &str,
    here: Option<Path>,
) {
    let tick = tick();
    list(frame, app, area, |app, short| {
        let rows = nav::categories(app, market, expiry);
        let counts = rows.iter().map(|r| count(r).len()).max().unwrap_or(0);
        let marks = rows
            .iter()
            .map(|r| width(&marker(app, &r.agents, r.focused(), short)))
            .max()
            .unwrap_or(0);
        rows.iter()
            .enumerate()
            .map(|(i, r)| {
                let heat = r.heat();
                let mut right = vec![
                    Span::styled(format!("{:>counts$}", count(r)), style(MUTED)),
                    Span::raw(" "),
                    heat_box(heat, tick, i),
                ];
                if marks > 0 {
                    let mark = marker(app, &r.agents, r.focused(), short);
                    let pad = marks - width(&mark);
                    right.push(Span::raw(" "));
                    right.extend(mark);
                    right.push(Span::raw(" ".repeat(pad)));
                }
                Item {
                    left: vec![Span::styled(
                        r.label.clone(),
                        if heat.is_none() {
                            style(MUTED)
                        } else {
                            style(TEXT)
                        },
                    )],
                    right,
                    lit: here.as_ref() == Some(&Path::category(market, expiry, &r.bucket)),
                    action: Action::Nav(Path::category(market, expiry, &r.bucket)),
                }
            })
            .collect()
    });
}

/// USDC atoms as "$30" or "$2.50"; none for nothing.
fn money(value: &serde_json::Value) -> Option<String> {
    let atoms = value.as_str()?.parse::<u64>().ok().filter(|a| *a > 0)?;
    Some(if atoms.is_multiple_of(1_000_000) {
        format!("${}", atoms / 1_000_000)
    } else {
        format!("${:.2}", atoms as f64 / 1e6)
    })
}

/// What a category's work is, in numbers, in parts: "sources 1/3",
/// "sites 1/2"; "updates", "2 sources"; "openings", "bond 2.5 USDC".
fn summary(row: &CategoryRow, task: Option<&Work>) -> Vec<String> {
    let o = task.map(|w| w.objective.clone()).unwrap_or_default();
    let sources = plural(row.tasks.len(), "source", "sources");
    match o["kind"].as_str().unwrap_or("source") {
        "update" => vec!["updates".into(), sources],
        "opening" => {
            let mut parts = vec!["openings".into(), sources];
            parts.extend(money(&o["openingBondAtomic"]).map(|b| format!("bond {b}")));
            parts.extend(money(&o["openingBudgetAtomic"]).map(|b| format!("budget {b}")));
            parts
        }
        _ if task.is_none() => vec![plural(row.sources as usize, "source", "sources")],
        _ => {
            let mut parts = vec![format!("sources {}", count(row).trim_end_matches(" src"))];
            if let (Some(have), Some(want)) = (
                o["distinctHosts"].as_u64(),
                o["targets"]["distinctHosts"].as_u64(),
            ) {
                parts.push(format!("sites {have}/{want}"));
            }
            parts
        }
    }
}

/// One category, in place of the list: what its work needs, who is on it,
/// your directive with Research this and Directive, then its tasks (an
/// update or opening per source) or the sources it has.
fn category(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    market: &str,
    expiry: &str,
    bucket: &str,
) {
    let Some(row) = nav::categories(app, market, expiry)
        .into_iter()
        .find(|r| r.bucket == bucket)
    else {
        return;
    };
    let task = nav::task(app, &row).cloned();
    let bottom = area.bottom();
    let at = |y: u16| Rect::new(area.x, y, area.width, bottom.saturating_sub(y));
    // What it needs, and its temperature.
    let mut parts: Vec<(&'static str, Vec<Span<'static>>)> = summary(&row, task.as_ref())
        .into_iter()
        .map(|p| (" · ", vec![Span::styled(p, style(MUTED))]))
        .collect();
    if let Some(h) = row.heat() {
        parts.push((
            "  ",
            vec![
                heat_box(Some(h), tick(), 0),
                Span::styled(format!(" {}", h.word()), style(MUTED)),
            ],
        ));
    }
    let mut y = area.y + flow(frame, at(area.y), parts);
    // Who is on it.
    let on: Vec<(&'static str, Vec<Span<'static>>)> = app
        .snapshot
        .agents
        .iter()
        .filter(|a| row.agents.contains(&a.id))
        .map(|a| {
            (
                ", ",
                vec![Span::styled(
                    format!("● {} · {}", safe(&a.harness), safe(&a.name)),
                    style(ACCENT),
                )],
            )
        })
        .collect();
    let nobody = on.is_empty();
    if y < bottom && !nobody {
        y += flow(frame, at(y), on);
    }
    if let Some(task) = &task {
        // Your directive, and what you can do about it.
        let directive = task.guidance["instruction"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| safe(&s.split_whitespace().collect::<Vec<_>>().join(" ")));
        if let (true, Some(text)) = (y < bottom, directive) {
            let mut spans = vec![Span::raw(" "), Span::styled("you: ", bold(ACCENT))];
            spans.extend(fit(
                vec![Span::styled(text, style(TEXT))],
                usize::from(area.width.saturating_sub(6)),
            ));
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(area.x, y, area.width, 1),
            );
            y += 1;
        } else if y < bottom && task.focused && nobody {
            flow(
                frame,
                at(y),
                vec![("", vec![Span::styled("○ pick", style(MUTED))])],
            );
            y += 1;
        }
        if y < bottom {
            buttons(
                frame,
                app,
                Rect::new(area.x + 1, y, area.width.saturating_sub(1), 1),
                vec![
                    (
                        if task.focused {
                            "Stop researching"
                        } else {
                            "Research this"
                        },
                        Action::Focus(task.id.clone(), !task.focused),
                        false,
                    ),
                    ("Directive", Action::Refine(task.id.clone()), false),
                ],
            );
            y += 1;
        }
    }
    if y >= bottom {
        return;
    }
    let rest = at(y);
    let staked = row
        .tasks
        .iter()
        .any(|w| matches!(w.objective["kind"].as_str(), Some("update" | "opening")));
    if staked {
        // A task per source; with more than one, the buttons act on the lit one.
        let tick = tick();
        let picked = task.map(|t| t.id).filter(|_| row.tasks.len() > 1);
        list(frame, app, rest, |app, short| {
            // One task's agent is named above already.
            let marks = row
                .tasks
                .iter()
                .filter(|_| row.tasks.len() > 1)
                .map(|w| width(&marker(app, &w.researchers, w.focused, short)))
                .max()
                .unwrap_or(0);
            row.tasks
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let page = w.objective["url"]
                        .as_str()
                        .map(nav::short_url)
                        .unwrap_or_else(|| safe(&w.title));
                    let mut right = vec![heat_box(Some(Heat::of(w)), tick, i)];
                    if marks > 0 {
                        let mark = marker(app, &w.researchers, w.focused, short);
                        let pad = marks - width(&mark);
                        right.push(Span::raw(" "));
                        right.extend(mark);
                        right.push(Span::raw(" ".repeat(pad)));
                    }
                    Item {
                        left: vec![Span::styled(page, style(TEXT))],
                        right,
                        lit: picked.as_ref() == Some(&w.id),
                        action: Action::Task(w.id.clone()),
                    }
                })
                .collect()
        });
        return;
    }
    // The sources it has: its month's, else its gap's; on a chip month, with
    // what the spec makes of each ("active · counts this month").
    let mut found: Vec<(String, String)> = nav::month_of(app, market, expiry)
        .map(|m| {
            m.sources
                .iter()
                .filter(|s| s.bucket == bucket || bucket == ALL)
                .map(|s| {
                    let status = s.status.to_lowercase();
                    (
                        s.url.as_deref().map(nav::short_url).unwrap_or_default(),
                        match &s.checked {
                            Some(checked) => format!("{status} · {}", checked.words()),
                            None => status,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if found.is_empty()
        && let Some(task) = &task
    {
        found = task.objective["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                (
                    s["url"].as_str().map(nav::short_url).unwrap_or_default(),
                    s["status"].as_str().unwrap_or_default().to_lowercase(),
                )
            })
            .collect();
    }
    let skip = usize::from(app.scroll).min(found.len().saturating_sub(1));
    app.scroll = skip as u16;
    let room = usize::from(area.width.saturating_sub(1));
    for (i, (url, status)) in found.into_iter().skip(skip).enumerate() {
        let y = rest.y + i as u16;
        if y >= bottom {
            break;
        }
        let status = Span::styled(status, style(DIM));
        let sw = status.width();
        let left = fit(
            vec![Span::styled(url, style(TEXT))],
            room.saturating_sub(sw + 1),
        );
        let lw = width(&left);
        let mut spans = vec![Span::raw(" ")];
        spans.extend(left);
        spans.push(Span::raw(" ".repeat(room.saturating_sub(lw + sw))));
        spans.push(status);
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

/// "14:03", this computer's time.
fn clock(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The agent's page: who it is and what it is on, then its feed, newest at
/// the bottom unless scrolled up.
fn page(frame: &mut Frame, app: &mut App, area: Rect, agent: &Agent) {
    if area.height == 0 || area.width < 4 {
        return;
    }
    let room = usize::from(area.width);
    // Who it is and what it is on: one line, or two when they do not fit.
    let name = vec![
        Span::styled(safe(&agent.harness), bold(TEXT)),
        Span::styled(" · ", style(DIM)),
        Span::styled(safe(&agent.name), style(MUTED)),
    ];
    let on = if agent.revoked_at.is_some() {
        Span::styled("off", style(ALERT))
    } else {
        match nav::doing(app, &agent.id) {
            Some(what) => Span::styled(what, style(ACCENT)),
            None => Span::styled("idle", style(DIM)),
        }
    };
    let split = width(&name) + 2 + on.width() > room && area.height > 3;
    let mut head = vec![Line::from(fit(name.clone(), room))];
    if split {
        head.push(Line::from(fit(vec![on], room)));
    } else {
        let mut one = name;
        one.push(Span::raw("  "));
        one.push(on);
        head[0] = Line::from(fit(one, room));
    }
    let rows = head.len() as u16;
    frame.render_widget(
        Paragraph::new(head),
        Rect::new(area.x, area.y, area.width, rows),
    );
    let feed = Rect::new(area.x, area.y + rows, area.width, area.height - rows);
    if feed.height == 0 {
        return;
    }
    let lines = nav::feed(app, &agent.id);
    let h = usize::from(feed.height);
    let max = lines.len().saturating_sub(h);
    app.feed_area = feed;
    app.feed_max = max;
    let pinned = app.nav().feed_top.is_none_or(|t| t >= max);
    let top = app.nav().feed_top.map_or(max, |t| t.min(max));
    let shown = if pinned { h } else { h - 1 };
    for (i, l) in lines.iter().skip(top).take(shown).enumerate() {
        let mut spans = vec![Span::styled(format!("{} ", clock(l.at_ms)), style(DIM))];
        let text = style(match l.tone {
            Tone::Agent | Tone::You => TEXT,
            Tone::Petri => MUTED,
            Tone::Found => ACCENT,
            Tone::Alert => ALERT,
        });
        if l.tone == Tone::You {
            spans.push(Span::styled("you: ", bold(ACCENT)));
        } else {
            let icon = match l.tone {
                Tone::Found => style(ACCENT),
                Tone::Alert => bold(ALERT),
                _ => style(MUTED),
            };
            spans.push(Span::styled(format!("{} ", l.icon), icon));
        }
        spans.push(Span::styled(l.text.clone(), text));
        // Its note only when it fits whole.
        let note = Span::styled(format!("  {}", l.note), style(DIM));
        if !l.note.is_empty() && width(&spans) + note.width() <= room {
            spans.push(note);
        }
        // A line of a run shows that run's work in the navigator.
        let at = Rect::new(feed.x, feed.y + i as u16, feed.width, 1);
        let hovered = l
            .place
            .clone()
            .is_some_and(|place| register(app, at, Action::Nav(place)));
        frame.render_widget(
            Paragraph::new(Line::from(fit(spans, room))).style(if hovered {
                Style::default().bg(PLATE_FOCUS)
            } else {
                Style::default()
            }),
            at,
        );
    }
    if !pinned {
        let y = feed.bottom() - 1;
        let at = Rect::new(feed.x, y, feed.width, 1);
        let hovered = register(app, at, Action::Newest);
        frame.render_widget(
            Paragraph::new(format!("▾ {} newer", lines.len() - top - shown)).style(if hovered {
                bold(ACCENT).bg(PLATE_FOCUS)
            } else {
                style(DIM)
            }),
            at,
        );
    }
}

/// `parts` (each with what goes before it) from `area`'s left, a part that
/// does not fit going on the next row, up to `area`'s height. Returns the
/// rows used.
fn flow(frame: &mut Frame, area: Rect, parts: Vec<(&'static str, Vec<Span<'static>>)>) -> u16 {
    let room = usize::from(area.width.saturating_sub(1));
    let mut lines: Vec<Vec<Span<'static>>> = vec![vec![]];
    for (before, part) in parts {
        let full = lines.len() >= usize::from(area.height);
        let line = lines.last_mut().expect("a line");
        let gap = if line.is_empty() { 0 } else { before.len() };
        if !line.is_empty() && width(line) + gap + width(&part) > room && !full {
            lines.push(part);
            continue;
        }
        if !line.is_empty() {
            line.push(Span::styled(before, style(DIM)));
        }
        line.extend(part);
    }
    for (i, spans) in lines.iter().enumerate() {
        let mut all = vec![Span::raw(" ")];
        all.extend(fit(spans.clone(), room));
        frame.render_widget(
            Paragraph::new(Line::from(all)),
            Rect::new(area.x, area.y + i as u16, area.width, 1),
        );
    }
    lines.len() as u16
}
