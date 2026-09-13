//! Embedded chart screen and contract-activity control.

use super::super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum ChartControlHit {
    Range(ChartRangeValue),
    Refresh,
}

const CHART_RANGES: [ChartRangeValue; 5] = [
    ChartRangeValue::OneHour,
    ChartRangeValue::TwentyFourHours,
    ChartRangeValue::SevenDays,
    ChartRangeValue::ThirtyDays,
    ChartRangeValue::All,
];

fn chart_controls_area(area: Rect) -> Option<Rect> {
    (area.width >= 30 && area.height >= 7).then_some(Rect {
        height: if area.height >= 18 { 3 } else { 1 },
        ..area
    })
}

fn chart_content_area(area: Rect) -> Rect {
    let Some(controls) = chart_controls_area(area) else {
        return area;
    };
    Rect {
        y: controls.y.saturating_add(controls.height),
        height: area.height.saturating_sub(controls.height),
        ..area
    }
}

pub(in super::super) fn chart_activity_area(area: Rect, app: &LabApp) -> Option<Rect> {
    let content = chart_content_area(area);
    (content.height >= 12)
        .then(|| chart_contract_activity_area(area, app))
        .flatten()
}

fn chart_control_rects(area: Rect) -> Option<Vec<Rect>> {
    let controls = chart_controls_area(area)?;
    Some(
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 6); 6])
            .split(controls)
            .to_vec(),
    )
}

pub(in super::super) fn chart_control_hit_at(
    area: Rect,
    column: u16,
    row: u16,
) -> Option<ChartControlHit> {
    let index = chart_control_rects(area)?
        .iter()
        .position(|rect| rect_contains(*rect, column, row))?;
    CHART_RANGES
        .get(index)
        .copied()
        .map(ChartControlHit::Range)
        .or_else(|| (index == CHART_RANGES.len()).then_some(ChartControlHit::Refresh))
}

fn draw_chart_controls(frame: &mut Frame<'_>, cli: &Cli, area: Rect, app: &LabApp) {
    let Some(rects) = chart_control_rects(area) else {
        return;
    };
    for (index, range) in CHART_RANGES.iter().copied().enumerate() {
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                range.label(),
                Color::Cyan,
                app.trading.chart_range == range,
                rects[index].width,
                rects[index].height,
            )),
            rects[index],
        );
    }
    let refresh = rects[CHART_RANGES.len()];
    frame.render_widget(
        Paragraph::new(raised_button_lines(
            cli,
            if app.trading.loading_chart {
                "Loading"
            } else {
                "Refresh"
            },
            Color::Yellow,
            app.trading.loading_chart,
            refresh.width,
            refresh.height,
        )),
        refresh,
    );
}

pub(in super::super) fn draw_chart_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    draw_chart_controls(frame, cli, area, app);
    let area = chart_content_area(area);
    if let Some(chart) = &app.trading.chart {
        if area.height >= 12 {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(3)])
                .split(area);
            chart::draw_embedded_chart(frame, cli, rows[0], chart, app.focus == LabFocus::Chart);
            draw_contract_activity_button(frame, cli, rows[1], app);
        } else {
            chart::draw_embedded_chart(frame, cli, area, chart, app.focus == LabFocus::Chart);
        }
        return;
    }

    let month = app
        .selected_chart_expiry()
        .map(|expiry| {
            format!(
                "{} | settles {} | {} days",
                expiry.label, expiry.settlement, expiry.days
            )
        })
        .unwrap_or_else(|| "No month selected.".to_string());
    let title = if app.trading.loading_chart {
        format!("{} Loading chart...", app.spinner())
    } else {
        "Chart is not available right now.".to_string()
    };
    let panel = scrolling_panel(
        vec![
            Line::from(Span::styled(
                title,
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(month),
            Line::from(Span::styled(
                app.status.clone(),
                status_style(cli, &app.status),
            )),
            contract_activity_hint_line(cli, app),
            Line::from(
                "Use Left to choose a market, Enter to open months, then Enter for options.",
            ),
        ],
        area,
        cli,
        app.focused_panel_scroll(LabFocus::Chart),
        app.focus == LabFocus::Chart,
    )
    .block(panel_block(
        cli,
        "month chart",
        Color::Cyan,
        app.focus == LabFocus::Chart,
    ));
    frame.render_widget(panel, area);
}

pub(in super::super) fn draw_contract_activity_button(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let panel = scrolling_panel(
        vec![contract_activity_hint_line(cli, app)],
        area,
        cli,
        app.focused_panel_scroll(LabFocus::Activity),
        false,
    )
    .block(panel_block(cli, "contract activity", Color::Blue, false));
    frame.render_widget(panel, area);
}

pub(in super::super) fn contract_activity_hint_line(cli: &Cli, app: &LabApp) -> Line<'static> {
    let label = selected_contract_label(app);
    Line::from(vec![
        Span::styled("[a] ", style(cli, Color::Blue).add_modifier(Modifier::BOLD)),
        Span::styled(
            "recent trades",
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" for ", style(cli, Color::DarkGray)),
        Span::styled(label, style(cli, Color::Cyan)),
    ])
}
