use std::cmp::Ordering;

use chrono::DateTime;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Sparkline, Wrap},
};
use serde_json::Value;

use crate::{
    backend::{BackendClient, CliError, array_at_key, string_at_key, value_at_key},
    chain_identity,
    cli::{ChartArgs, ChartRangeValue, Cli},
    endpoints,
};

const BACKEND_MAX_CHART_WINDOW_MS: u64 = 90 * 24 * 60 * 60 * 1_000;

pub struct ChartFetch {
    pub options: ChartArgs,
    pub payload: Value,
}

#[derive(Clone, Debug)]
pub struct EmbeddedChart {
    options: ChartArgs,
    view: ChartView,
    status: String,
}

#[derive(Clone, Debug)]
struct ChartPoint {
    as_of: String,
    timestamp_ms: Option<i64>,
    fair_price: f64,
    base_oracle: Option<f64>,
    volume_24h_usd: Option<f64>,
    total_liquidity_usd: Option<f64>,
    listed_notional: Option<f64>,
    best_call_bid: Option<f64>,
    best_call_ask: Option<f64>,
    best_put_bid: Option<f64>,
    best_put_ask: Option<f64>,
}

#[derive(Clone, Debug)]
struct ChartView {
    market_id: String,
    expiry_id: Option<String>,
    expiry_label: Option<String>,
    settlement_utc: Option<String>,
    points: Vec<ChartPoint>,
}

pub fn chart_endpoint(options: &ChartArgs) -> String {
    let window_ms = chart_query_window_ms(options.range);
    let market = options.market.trim().to_ascii_lowercase();
    match options
        .expiry
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(expiry) => endpoints::dlmm_expiry_chart(&market, expiry, window_ms),
        None => endpoints::dlmm_market_chart(&market, window_ms),
    }
}

fn validate_selected_series_scope(options: &ChartArgs, payload: &Value) -> Result<(), CliError> {
    let expiry_id = options
        .expiry
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let data = unwrap_data(payload);
    let market_id = options.market.trim().to_ascii_lowercase();
    if data.get("marketId").and_then(Value::as_str) != Some(market_id.as_str())
        || data.get("expiryId").and_then(Value::as_str) != expiry_id
        || data.get("paperOnly").is_some()
    {
        return Err(CliError::new(
            "Price history does not match the requested market and contract.",
        ));
    }
    let points = array_at_key(data, &["points"]).ok_or_else(|| {
        CliError::new("the current market chart response has no bounded points array")
    })?;
    if points.iter().any(|point| {
        point.get("schemaVersion").and_then(Value::as_u64) != Some(1)
            || point.get("marketId").and_then(Value::as_str) != Some(market_id.as_str())
            || point.get("expiryId").and_then(Value::as_str) != expiry_id
            || parse_chart_point(point).is_none()
    }) {
        return Err(CliError::new(
            "Price history contains invalid observations or belongs to a different contract.",
        ));
    }
    Ok(())
}

pub fn fetch_chart_payload(
    backend: &BackendClient,
    options: &ChartArgs,
) -> Result<ChartFetch, CliError> {
    let primary = fetch_chart_once(backend, options)?;
    if chart_payload_has_points(&primary.options, &primary.payload) {
        return Ok(primary);
    }

    if options.range == ChartRangeValue::All {
        return Ok(primary);
    }

    let wide = fetch_wide_chart(backend, options)?;
    if chart_payload_has_points(&wide.options, &wide.payload) {
        Ok(wide)
    } else {
        Ok(primary)
    }
}

pub fn render_chart_static(_cli: &Cli, _backend: &BackendClient, fetch: &ChartFetch) -> String {
    let view = ChartView::from_fetch(fetch);
    let options = &fetch.options;
    let points = view.display_points(options);
    let mut lines = vec![
        format!("{}", chart_title(&view, options)),
        "Observed chart history".to_string(),
        format!(
            "Range {} | points {} of {} | fixed max loss | no liquidation",
            options.range.label(),
            points.len(),
            view.points.len()
        ),
    ];

    if points.is_empty() {
        lines.push(
            "No observed chart history yet. Refresh after live venue observations begin."
                .to_string(),
        );
        return lines.join("\n");
    }

    lines.push(chart_summary_line(&points, view.settlement_utc.as_deref()));
    lines.push("legend: * fair price".to_string());
    lines.extend(render_ascii_chart(&points, options.height));
    if points.iter().any(|point| point.volume_24h_usd.is_some()) {
        lines.push(render_ascii_volume_line(&points));
    }
    if let Some(latest) = points.last() {
        lines.push(format!("latest: {}", point_detail_line(latest)));
    }
    if let Some(first) = points.first() {
        if let Some(last) = points.last() {
            lines.push(format!(
                "window: {} -> {}",
                format_axis_timestamp(first),
                format_axis_timestamp(last)
            ));
        }
    }
    lines.join("\n")
}

pub fn fetch_embedded_chart(
    backend: &BackendClient,
    options: &ChartArgs,
) -> Result<EmbeddedChart, CliError> {
    let fetch = fetch_chart_payload(backend, options)?;
    let mut chart = EmbeddedChart::from_fetch(fetch, "Loaded month chart".to_string());
    if chart.point_count() == 0 {
        chart.status = "No chart history for this month yet".to_string();
    }
    Ok(chart)
}

impl EmbeddedChart {
    pub(crate) fn from_fetch(fetch: ChartFetch, status: String) -> Self {
        let options = fetch.options.clone();
        let view = ChartView::from_fetch(&fetch);
        Self {
            options,
            view,
            status,
        }
    }

    pub fn point_count(&self) -> usize {
        self.view.display_points(&self.options).len()
    }

    pub fn total_point_count(&self) -> usize {
        self.view.points.len()
    }

    pub fn title(&self) -> String {
        chart_title(&self.view, &self.options)
    }

    pub fn range_label(&self) -> &'static str {
        self.options.range.label()
    }

    pub(crate) fn guide_facts(&self) -> Vec<String> {
        let points = self.view.display_points(&self.options);
        if points.is_empty() {
            return vec!["No price history is visible in the selected range.".to_string()];
        }

        let mut facts = vec![chart_summary_line(
            &points,
            self.view.settlement_utc.as_deref(),
        )];
        if let Some(latest) = points.last() {
            facts.push(format!(
                "Latest visible point: {}",
                point_detail_line(latest)
            ));
        }
        facts
    }
}

impl ChartView {
    fn from_fetch(fetch: &ChartFetch) -> Self {
        Self::from_payload(&fetch.options, &fetch.payload)
    }

    fn from_payload(options: &ChartArgs, payload: &Value) -> Self {
        let data = unwrap_data(payload);
        let history = value_at_key(data, &["history"]).unwrap_or(data);
        let market_id = string_at_key(history, &["marketId", "market_id", "id"])
            .unwrap_or_else(|| options.market.trim().to_string());
        let expiry_id = string_at_key(history, &["expiryId", "expiry_id"])
            .or_else(|| {
                options
                    .expiry
                    .as_ref()
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty());
        let latest = array_at_key(history, &["points"]).and_then(|points| points.last());
        let expiry_label = string_at_key(history, &["expiryLabel", "expiry_label", "label"])
            .or_else(|| latest.and_then(|point| string_at_key(point, &["expiryLabel"])));
        let settlement_utc = string_at_key(
            history,
            &["settlementUtc", "settlement_utc", "settlementTime"],
        )
        .or_else(|| latest.and_then(|point| string_at_key(point, &["settlementUtc"])));
        let mut points = array_at_key(history, &["points", "history"])
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(parse_chart_point)
            .collect::<Vec<_>>();

        sort_and_dedup_points(&mut points);

        Self {
            market_id,
            expiry_id,
            expiry_label,
            settlement_utc,
            points,
        }
    }

    fn display_points(&self, options: &ChartArgs) -> Vec<ChartPoint> {
        let ranged = filter_points_for_range(&self.points, options.range);
        downsample_points(&ranged, options.points.clamp(2, 2_000))
    }
}

pub fn draw_embedded_chart(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    chart: &EmbeddedChart,
    focused: bool,
) {
    let points = chart.view.display_points(&chart.options);
    let has_volume = points.iter().any(|point| point.volume_24h_usd.is_some());
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(7),
            Constraint::Length(if has_volume { 3 } else { 0 }),
            Constraint::Length(6),
        ])
        .split(area);
    let header_title = if focused {
        "month chart *"
    } else {
        "month chart"
    };
    let header_style = if focused {
        chart_style(cli, Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        chart_style(cli, Color::Cyan)
    };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                chart.title(),
                chart_style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  range ".to_string(), chart_style(cli, Color::DarkGray)),
            Span::styled(
                chart.range_label().to_string(),
                chart_style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  points ".to_string(), chart_style(cli, Color::DarkGray)),
            Span::styled(
                format!("{}/{}", chart.point_count(), chart.total_point_count()),
                chart_style(cli, Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(chart.status.clone(), status_style(cli, &chart.status)),
            Span::styled(
                " | fair price from selected month",
                chart_style(cli, Color::DarkGray),
            ),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(Span::styled(header_title, header_style)),
    );
    frame.render_widget(header, areas[0]);

    if points.is_empty() {
        let empty_area = Rect {
            x: area.x,
            y: areas[1].y,
            width: area.width,
            height: area
                .y
                .saturating_add(area.height)
                .saturating_sub(areas[1].y),
        };
        let empty = Paragraph::new(vec![
            Line::from(Span::styled(
                "No chart history for this month yet.",
                chart_style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Try a wider range or another listed month.",
                chart_style(cli, Color::Gray),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("fair price"))
        .wrap(Wrap { trim: true });
        frame.render_widget(empty, empty_area);
        return;
    }

    draw_price_chart(frame, cli, areas[1], &points);

    if has_volume {
        draw_volume_sparkline(frame, cli, areas[2], &points);
    }

    let details = Paragraph::new(detail_lines(
        &points,
        points.last(),
        chart.view.settlement_utc.as_deref(),
    ))
    .block(Block::default().borders(Borders::ALL).title("month detail"))
    .wrap(Wrap { trim: true });
    frame.render_widget(details, areas[3]);
}

fn price_chart_marker_for_platform(is_windows: bool) -> symbols::Marker {
    if is_windows {
        symbols::Marker::Block
    } else {
        symbols::Marker::Braille
    }
}

fn draw_price_chart(frame: &mut Frame<'_>, cli: &Cli, area: Rect, points: &[ChartPoint]) {
    let fair_data = points
        .iter()
        .enumerate()
        .map(|(index, point)| (point_x(point, index), point.fair_price))
        .collect::<Vec<_>>();
    let values = points
        .iter()
        .map(|point| point.fair_price)
        .collect::<Vec<_>>();
    let (mut min_y, mut max_y) = min_max(&values).unwrap_or((0.0, 1.0));
    if (max_y - min_y).abs() < f64::EPSILON {
        let pad = (min_y.abs() * 0.08).max(0.000_001);
        min_y = (min_y - pad).max(0.0);
        max_y += pad;
    } else {
        let pad = (max_y - min_y) * 0.08;
        min_y -= pad;
        max_y += pad;
    }

    let (mut min_x, mut max_x) =
        min_max(&fair_data.iter().map(|point| point.0).collect::<Vec<_>>()).unwrap_or((0.0, 1.0));
    if (max_x - min_x).abs() < f64::EPSILON {
        min_x -= 1.0;
        max_x += 1.0;
    }

    let datasets = vec![
        Dataset::default()
            .name("fair")
            .marker(price_chart_marker_for_platform(cfg!(windows)))
            .graph_type(if points.len() == 1 {
                GraphType::Scatter
            } else {
                GraphType::Line
            })
            .style(chart_style(cli, Color::Yellow))
            .data(&fair_data),
    ];

    let first_label = points
        .first()
        .map(format_axis_timestamp)
        .unwrap_or_default();
    let last_label = points.last().map(format_axis_timestamp).unwrap_or_default();
    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title("fair price"))
        .x_axis(
            Axis::default()
                .style(chart_style(cli, Color::Gray))
                .bounds([min_x, max_x])
                .labels(vec![Span::raw(first_label), Span::raw(last_label)]),
        )
        .y_axis(
            Axis::default()
                .style(chart_style(cli, Color::Gray))
                .bounds([min_y, max_y])
                .labels(vec![
                    Span::raw(format_decimal(min_y, 2)),
                    Span::raw(format_decimal(max_y, 2)),
                ]),
        );
    frame.render_widget(chart, area);
}

fn draw_volume_sparkline(frame: &mut Frame<'_>, cli: &Cli, area: Rect, points: &[ChartPoint]) {
    let volumes = points
        .iter()
        .map(|point| {
            point
                .volume_24h_usd
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(0.0)
        })
        .collect::<Vec<_>>();
    let max_volume = volumes
        .iter()
        .copied()
        .filter(|value| *value > 0.0)
        .reduce(f64::max)
        .unwrap_or(0.0);

    let latest_volume = volumes.last().copied().unwrap_or(0.0);
    let data = if max_volume > 0.0 {
        volumes
            .iter()
            .map(|value| ((*value / max_volume).clamp(0.0, 1.0) * 1_000.0).round() as u64)
            .collect::<Vec<_>>()
    } else {
        vec![0; volumes.len().max(1)]
    };
    let title = format!(
        "volume 24h | latest {} | max {}",
        format_optional_usd(Some(latest_volume)),
        format_optional_usd(Some(max_volume))
    );
    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(&data)
        .max(1_000)
        .style(chart_style(cli, Color::Blue));
    frame.render_widget(sparkline, area);
}

fn chart_style(cli: &Cli, color: Color) -> Style {
    if cli.no_color {
        Style::default()
    } else {
        Style::default().fg(readable_chart_color(color))
    }
}

fn readable_chart_color(color: Color) -> Color {
    match color {
        Color::Blue | Color::LightBlue => Color::White,
        other => other,
    }
}

fn status_style(cli: &Cli, value: &str) -> Style {
    let lower = value.to_ascii_lowercase();
    let color = if lower.contains("failed")
        || lower.contains("unavailable")
        || lower.contains("not available")
    {
        Color::Red
    } else if lower.contains("ready") || lower.contains("updated") {
        Color::Green
    } else {
        Color::Yellow
    };
    chart_style(cli, color)
}

fn detail_lines(
    points: &[ChartPoint],
    selected: Option<&ChartPoint>,
    settlement_utc: Option<&str>,
) -> Vec<Line<'static>> {
    let latest = points.last();
    let summary = chart_summary_line(points, settlement_utc);
    let selected_line = selected
        .map(point_detail_line)
        .unwrap_or_else(|| "selected: n/a".to_string());
    let latest_line = latest
        .map(|point| format!("latest: {}", point_detail_line(point)))
        .unwrap_or_else(|| "latest: n/a".to_string());
    vec![
        Line::from(summary),
        Line::from(selected_line),
        Line::from(latest_line),
        Line::from("Oracle reference is shown separately from the contract price."),
        Line::from("Amoeba options have fixed maximum loss and no liquidation."),
    ]
}

fn render_ascii_chart(points: &[ChartPoint], requested_height: usize) -> Vec<String> {
    let height = requested_height.clamp(6, 30);
    let terminal_width = crossterm::terminal::size()
        .map(|(width, _)| usize::from(width))
        .unwrap_or(100);
    let width = terminal_width.saturating_sub(14).clamp(40, 120);
    let mut grid = vec![vec![' '; width]; height];
    let values = points
        .iter()
        .map(|point| point.fair_price)
        .collect::<Vec<_>>();
    let (min_y, max_y) = min_max(&values).unwrap_or((0.0, 1.0));
    let y_for = |value: f64| -> usize {
        if (max_y - min_y).abs() < f64::EPSILON {
            height / 2
        } else {
            let scaled = ((max_y - value) / (max_y - min_y)) * (height.saturating_sub(1) as f64);
            scaled.round().clamp(0.0, height.saturating_sub(1) as f64) as usize
        }
    };
    let x_for = |index: usize| -> usize {
        if points.len() <= 1 {
            width / 2
        } else {
            index * (width - 1) / (points.len() - 1)
        }
    };

    for (index, point) in points.iter().enumerate() {
        let x = x_for(index);
        let y = y_for(point.fair_price);
        grid[y][x] = '*';
    }

    grid.into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let y_value = if height <= 1 {
                max_y
            } else {
                max_y - ((max_y - min_y) * (row_index as f64 / (height - 1) as f64))
            };
            format!(
                "{:>9} | {}",
                format_decimal(y_value, 2),
                row.into_iter().collect::<String>()
            )
        })
        .collect()
}

fn render_ascii_volume_line(points: &[ChartPoint]) -> String {
    let volumes = points
        .iter()
        .map(|point| {
            point
                .volume_24h_usd
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(0.0)
        })
        .collect::<Vec<_>>();
    let max_volume = volumes
        .iter()
        .copied()
        .filter(|value| *value > 0.0)
        .reduce(f64::max)
        .unwrap_or(0.0);

    let width = crossterm::terminal::size()
        .map(|(width, _)| usize::from(width))
        .unwrap_or(100)
        .saturating_sub(16)
        .clamp(24, 80);
    let sampled = if volumes.is_empty() {
        vec![0.0; width]
    } else {
        downsample_scalar_values(&volumes, width)
    };
    let bars = sampled
        .iter()
        .map(|value| volume_ascii_level(*value, max_volume))
        .collect::<String>();
    format!(
        "volume 24h: {} latest={} max={}",
        bars,
        format_optional_usd(volumes.last().copied()),
        format_optional_usd(Some(max_volume))
    )
}

fn downsample_scalar_values(values: &[f64], max_points: usize) -> Vec<f64> {
    if values.len() <= max_points {
        return values.to_vec();
    }
    if max_points <= 1 {
        return values.last().copied().into_iter().collect();
    }
    let last = values.len() - 1;
    (0..max_points)
        .map(|index| values[index * last / (max_points - 1)])
        .collect()
}

fn volume_ascii_level(value: f64, max_value: f64) -> char {
    if !value.is_finite() || value <= 0.0 || !max_value.is_finite() || max_value <= 0.0 {
        return '.';
    }
    const LEVELS: [char; 8] = ['.', ':', '-', '=', '+', '*', '#', '@'];
    let index = ((value / max_value).clamp(0.0, 1.0) * (LEVELS.len() - 1) as f64).round() as usize;
    LEVELS[index]
}

fn parse_chart_point(value: &Value) -> Option<ChartPoint> {
    // Use the same current exact-series, micro-unit history as the site's
    // trading chart. Do not reinterpret oracle/index prices as option prices.
    let as_of = value.get("asOf")?.as_str()?.to_string();
    let timestamp_ms = parse_timestamp_ms(&as_of)?;
    if timestamp_ms <= 0
        || u128::try_from(timestamp_ms).ok()? != chart_atomic(value, "timestampMs")?
    {
        return None;
    }
    let fair_price = chart_atomic(value, "fairPriceMicros")? as f64 / 1_000_000.0;
    if !fair_price.is_finite() || fair_price <= 0.0 {
        return None;
    }
    let oracle = chart_atomic(value, "oraclePriceMicros")?;
    let liquidity = chart_atomic(value, "liquidityQuoteAtomic")?;
    let notional = chart_atomic(value, "notionalQuoteAtomic")?;
    Some(ChartPoint {
        timestamp_ms: Some(timestamp_ms),
        as_of,
        fair_price,
        base_oracle: (oracle > 0).then_some(oracle as f64 / 1_000_000.0),
        volume_24h_usd: None,
        total_liquidity_usd: Some(liquidity as f64 / 1_000_000.0),
        listed_notional: Some(notional as f64 / 1_000_000.0),
        best_call_bid: None,
        best_call_ask: None,
        best_put_bid: None,
        best_put_ask: None,
    })
}

fn chart_atomic(value: &Value, key: &str) -> Option<u128> {
    let raw = value.get(key)?.as_str()?;
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    raw.parse().ok()
}

fn chart_query_window_ms(range: ChartRangeValue) -> Option<u64> {
    range.window_ms().or(Some(BACKEND_MAX_CHART_WINDOW_MS))
}

fn fetch_chart_once(backend: &BackendClient, options: &ChartArgs) -> Result<ChartFetch, CliError> {
    let endpoint = chart_endpoint(options);
    let payload = backend.get(&endpoint)?;
    chain_identity::validate_current_backend_envelope(&payload)?;
    validate_selected_series_scope(options, &payload)?;
    Ok(ChartFetch {
        options: options.clone(),
        payload,
    })
}

fn fetch_wide_chart(backend: &BackendClient, options: &ChartArgs) -> Result<ChartFetch, CliError> {
    if options.range == ChartRangeValue::All {
        return fetch_chart_once(backend, options);
    }
    let mut wide_options = options.clone();
    wide_options.range = ChartRangeValue::All;
    fetch_chart_once(backend, &wide_options)
}

fn chart_payload_has_points(options: &ChartArgs, payload: &Value) -> bool {
    !ChartView::from_payload(options, payload).points.is_empty()
}

fn unwrap_data(payload: &Value) -> &Value {
    value_at_key(payload, &["data"]).unwrap_or(payload)
}

fn parse_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.timestamp_millis())
}

fn compare_points(left: &ChartPoint, right: &ChartPoint) -> Ordering {
    match (left.timestamp_ms, right.timestamp_ms) {
        (Some(left_ts), Some(right_ts)) => left_ts.cmp(&right_ts),
        _ => left.as_of.cmp(&right.as_of),
    }
}

fn sort_and_dedup_points(points: &mut Vec<ChartPoint>) {
    points.sort_by(compare_points);
    points.dedup_by(|left, right| {
        left.as_of == right.as_of && (left.fair_price - right.fair_price).abs() < f64::EPSILON
    });
}

fn filter_points_for_range(points: &[ChartPoint], range: ChartRangeValue) -> Vec<ChartPoint> {
    let Some(window_ms) = range.window_ms() else {
        return points.to_vec();
    };
    let Some(latest_ms) = points.iter().rev().find_map(|point| point.timestamp_ms) else {
        return points.to_vec();
    };
    let cutoff = latest_ms.saturating_sub(window_ms as i64);
    let ranged = points
        .iter()
        .filter(|point| {
            point
                .timestamp_ms
                .map(|timestamp| timestamp >= cutoff)
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    if ranged.is_empty() {
        points.last().cloned().into_iter().collect()
    } else {
        ranged
    }
}

fn downsample_points(points: &[ChartPoint], max_points: usize) -> Vec<ChartPoint> {
    if points.len() <= max_points {
        return points.to_vec();
    }
    if max_points <= 1 {
        return points.last().cloned().into_iter().collect();
    }
    let last = points.len() - 1;
    (0..max_points)
        .map(|index| {
            let source_index = index * last / (max_points - 1);
            points[source_index].clone()
        })
        .collect()
}

fn point_x(point: &ChartPoint, index: usize) -> f64 {
    point
        .timestamp_ms
        .map(|value| value as f64)
        .unwrap_or(index as f64)
}

fn min_max(values: &[f64]) -> Option<(f64, f64)> {
    let mut finite = values.iter().copied().filter(|value| value.is_finite());
    let first = finite.next()?;
    let mut min_value = first;
    let mut max_value = first;
    for value in finite {
        min_value = min_value.min(value);
        max_value = max_value.max(value);
    }
    Some((min_value, max_value))
}

fn chart_title(view: &ChartView, options: &ChartArgs) -> String {
    let market = if view.market_id.trim().is_empty() {
        options.market.trim()
    } else {
        view.market_id.trim()
    };
    match (view.expiry_label.as_deref(), view.expiry_id.as_deref()) {
        (Some(label), Some(id)) if !label.eq_ignore_ascii_case(id) => {
            format!("{} chart / {}", market.to_uppercase(), label)
        }
        (Some(label), _) => format!("{} chart / {}", market.to_uppercase(), label),
        (_, Some(id)) => format!("{} chart / {}", market.to_uppercase(), id),
        _ => format!("{} chart", market.to_uppercase()),
    }
}

fn chart_summary_line(points: &[ChartPoint], settlement_utc: Option<&str>) -> String {
    let Some(first) = points.first() else {
        return "summary: n/a".to_string();
    };
    let latest = points.last().unwrap_or(first);
    let delta = latest.fair_price - first.fair_price;
    let pct = if first.fair_price > 0.0 {
        Some((delta / first.fair_price) * 100.0)
    } else {
        None
    };
    let settlement = settlement_utc
        .map(|value| format!(" | settles {}", format_timestamp_label(value, false)))
        .unwrap_or_default();
    format!(
        "fair {} | move {} ({}) | from {} | to {}{}",
        format_decimal(latest.fair_price, 3),
        format_signed_decimal(delta, 3),
        format_optional_percent(pct),
        format_axis_timestamp(first),
        format_axis_timestamp(latest),
        settlement
    )
}

fn point_detail_line(point: &ChartPoint) -> String {
    format!(
        "{} | fair {} | starting index {} | volume 24h {} | liquidity {} | call {}/{} | put {}/{} | notional {}",
        format_axis_timestamp(point),
        format_decimal(point.fair_price, 3),
        format_optional_decimal(point.base_oracle, 3),
        format_optional_usd(point.volume_24h_usd),
        format_optional_usd(point.total_liquidity_usd),
        format_optional_decimal(point.best_call_bid, 3),
        format_optional_decimal(point.best_call_ask, 3),
        format_optional_decimal(point.best_put_bid, 3),
        format_optional_decimal(point.best_put_ask, 3),
        format_optional_usd(point.listed_notional)
    )
}

fn format_axis_timestamp(point: &ChartPoint) -> String {
    format_timestamp_label(&point.as_of, true)
}

fn format_timestamp_label(value: &str, prefer_time: bool) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|date| {
            if prefer_time {
                date.format("%b %d %H:%M").to_string()
            } else {
                date.format("%b %d").to_string()
            }
        })
        .unwrap_or_else(|_| value.to_string())
}

fn format_optional_decimal(value: Option<f64>, digits: usize) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format_decimal(value, digits))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_decimal(value: f64, digits: usize) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    let formatted = format!("{value:.digits$}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn format_signed_decimal(value: f64, digits: usize) -> String {
    if value > 0.0 {
        format!("+{}", format_decimal(value, digits))
    } else {
        format_decimal(value, digits)
    }
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| {
            if value > 0.0 {
                format!("+{}%", format_decimal(value, 2))
            } else {
                format!("{}%", format_decimal(value, 2))
            }
        })
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_usd(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(format_compact_usd)
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_compact_usd(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    let abs = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    if abs >= 1_000_000_000.0 {
        format!("{sign}${}", format_decimal(abs / 1_000_000_000.0, 2)) + "B"
    } else if abs >= 1_000_000.0 {
        format!("{sign}${}", format_decimal(abs / 1_000_000.0, 2)) + "M"
    } else if abs >= 1_000.0 {
        format!("{sign}${}", format_decimal(abs / 1_000.0, 2)) + "K"
    } else {
        format!("{sign}${}", format_decimal(abs, 2))
    }
}
