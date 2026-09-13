//! Beginner-facing Oracle reward list.

use super::super::*;

const ORACLE_EARN_ROW_HEIGHT: u16 = 2;

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct OracleEarnLayout {
    pub(in super::super) summary_area: Rect,
    pub(in super::super) table_area: Rect,
    pub(in super::super) action_area: Rect,
}

pub(in super::super) fn oracle_earn_layout(area: Rect) -> OracleEarnLayout {
    let view = oracle_view_layout(area);
    let summary_height = u16::from(view.content_area.height >= 8) * 3;
    let action_height = u16::from(view.content_area.height >= 4) * 3;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(summary_height),
            Constraint::Min(1),
            Constraint::Length(action_height),
        ])
        .split(view.content_area);
    OracleEarnLayout {
        summary_area: rows[0],
        table_area: rows[1],
        action_area: rows[2],
    }
}

pub(in super::super) fn draw_oracle_earn_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let view = oracle_view_layout(area);
    draw_oracle_view_tabs(frame, cli, view.tabs_area, OracleView::Earn);
    let layout = oracle_earn_layout(area);
    let focused = app.focus == LabFocus::OracleEarn;

    if layout.summary_area.height > 0 {
        frame.render_widget(
            Paragraph::new(oracle_earn_summary_lines(cli, app)).style(tui_panel_style(cli)),
            layout.summary_area,
        );
    }

    let claims = app
        .current_oracle_rewards()
        .map(|state| state.claims.as_slice())
        .unwrap_or_default();
    let (start, end) = oracle_earn_visible_claim_window(layout.table_area, app, claims.len());
    let rows = oracle_earn_table_rows(cli, app, start, &claims[start..end]);
    let title = if claims.is_empty() {
        "FUNDED REWARDS".to_string()
    } else {
        format!("FUNDED REWARDS  {}-{} OF {}", start + 1, end, claims.len())
    };
    let header = Row::new([
        Cell::from("#"),
        Cell::from("REWARD / WHAT WAS NEEDED"),
        Cell::from("EARN"),
        Cell::from("STATUS"),
    ])
    .style(style(cli, Color::Cyan).add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(22),
            Constraint::Length(20),
            Constraint::Length(16),
        ],
    )
    .header(header)
    .block(panel_block(cli, &title, Color::Green, focused))
    .column_spacing(1)
    .row_highlight_style(
        style(cli, Color::White)
            .bg(TUI_FIELD_ACTIVE_BACKGROUND)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");
    let selected = (!claims.is_empty()).then_some(app.oracle.earn_selected.saturating_sub(start));
    let mut table_state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, layout.table_area, &mut table_state);

    if layout.action_area.height > 0 {
        let action = Paragraph::new(oracle_earn_action_line(cli, app, focused)).block(panel_block(
            cli,
            "SELECT A REWARD",
            Color::Yellow,
            focused,
        ));
        frame.render_widget(action, layout.action_area);
    }
}

fn oracle_earn_summary_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let market = selected_oracle_market_symbol(app);
    let month = selected_oracle_month_label(app);
    let count = app
        .current_oracle_rewards()
        .map(|state| state.claims.len())
        .unwrap_or_default();
    vec![
        Line::from(Span::styled(
            "EARN FROM ORACLE WORK",
            style(cli, Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(market, style(cli, Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("  •  ", style(cli, Color::DarkGray)),
            Span::styled(month, style(cli, Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("  •  settles ", style(cli, Color::DarkGray)),
            Span::styled(
                selected_oracle_settlement_label(app),
                style(cli, Color::White),
            ),
            Span::styled("  •  ", style(cli, Color::DarkGray)),
            Span::styled(
                format!("{count} funded reward{}", if count == 1 { "" } else { "s" }),
                style(cli, if count > 0 { Color::Green } else { Color::Gray }),
            ),
        ]),
    ]
}

fn oracle_earn_table_rows(
    cli: &Cli,
    app: &LabApp,
    start: usize,
    claims: &[SpreadOracleRewardClaim],
) -> Vec<Row<'static>> {
    if !claims.is_empty() {
        return claims
            .iter()
            .enumerate()
            .map(|(offset, claim)| {
                let rank = start + offset + 1;
                Row::new([
                    Cell::from(rank.to_string()),
                    Cell::from(Text::from(vec![
                        Line::from(Span::styled(
                            claim.label.clone(),
                            style(cli, Color::White).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            oracle_reward_need_label(&claim.kind),
                            style(cli, Color::Gray),
                        )),
                    ])),
                    Cell::from(Text::from(vec![
                        Line::from(Span::styled(
                            claim.amount_label.clone(),
                            style(cli, Color::Green).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled("funded reward", style(cli, Color::DarkGray))),
                    ])),
                    Cell::from(Text::from(vec![
                        Line::from(Span::styled(
                            "READY",
                            style(cli, Color::Green).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled("to review", style(cli, Color::Gray))),
                    ])),
                ])
                .height(ORACLE_EARN_ROW_HEIGHT)
            })
            .collect();
    }

    let (headline, detail, earn, status, color) = if app.wallet.pubkey.is_none() {
        (
            "Connect a wallet",
            "See rewards available to this wallet",
            "—",
            "CONNECT",
            Color::Yellow,
        )
    } else if app.oracle.loading_rewards {
        (
            "Checking funded rewards...",
            "Only the selected market and month are included",
            "—",
            "LOADING",
            Color::Cyan,
        )
    } else if app.oracle.reward_issue.is_some() {
        (
            "Rewards could not be verified",
            "Press R to try this market and month again",
            "—",
            "RETRY",
            Color::Red,
        )
    } else {
        (
            "No funded reward is ready",
            "Try another market or month, or refresh",
            "—",
            "NONE",
            Color::Yellow,
        )
    };
    vec![
        Row::new([
            Cell::from(""),
            Cell::from(Text::from(vec![
                Line::from(Span::styled(
                    headline,
                    style(cli, color).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(detail, style(cli, Color::Gray))),
            ])),
            Cell::from(Span::styled(earn, style(cli, Color::DarkGray))),
            Cell::from(Span::styled(
                status,
                style(cli, color).add_modifier(Modifier::BOLD),
            )),
        ])
        .height(ORACLE_EARN_ROW_HEIGHT),
    ]
}

fn oracle_reward_need_label(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "source_discovery" => "New repeatable source".to_string(),
        "source_challenge" => "Incorrect source evidence".to_string(),
        "opening_challenge" => "Incorrect opening value".to_string(),
        "game_update" => "Verified market update".to_string(),
        "update_challenge" => "Incorrect market update".to_string(),
        other => other.replace('_', " ").replace('-', " "),
    }
}

fn oracle_earn_visible_claim_window(
    table_area: Rect,
    app: &LabApp,
    claim_count: usize,
) -> (usize, usize) {
    if claim_count == 0 {
        return (0, 0);
    }
    let inner_height = table_area.height.saturating_sub(2);
    let row_height = usize::from(ORACLE_EARN_ROW_HEIGHT);
    let capacity = usize::from(inner_height.saturating_sub(1))
        .checked_div(row_height)
        .unwrap_or_default()
        .max(1);
    let selected = app.oracle.earn_selected.min(claim_count - 1);
    let start = selected
        .saturating_sub(capacity.saturating_sub(1))
        .min(claim_count.saturating_sub(capacity));
    (start, (start + capacity).min(claim_count))
}

pub(in super::super) fn oracle_earn_action_line(
    cli: &Cli,
    app: &LabApp,
    focused: bool,
) -> Line<'static> {
    let (label, detail, color) = if let Some(claim) = app.selected_oracle_reward_claim() {
        (
            format!(" REVIEW {} ", claim.amount_label),
            format!(
                "{} of {}  •  Enter to open",
                app.oracle.earn_selected + 1,
                app.current_oracle_rewards()
                    .map(|state| state.claims.len())
                    .unwrap_or_default()
            ),
            Color::Green,
        )
    } else if app.oracle.loading_rewards {
        (
            " CHECKING... ".to_string(),
            "Selected market + month".to_string(),
            Color::Cyan,
        )
    } else {
        (
            " REFRESH REWARDS ".to_string(),
            "Enter or R".to_string(),
            Color::Yellow,
        )
    };
    Line::from(vec![
        Span::styled("> ", cell_style(cli, color, focused)),
        Span::styled(label, terminal_button_key_style(cli, color)),
        Span::styled(
            format!("  {detail}"),
            cell_style(cli, Color::White, focused),
        ),
    ])
}

pub(in super::super) fn oracle_earn_claim_hit_at(
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = oracle_earn_layout(area);
    if !rect_contains(layout.table_area, column, row) {
        return None;
    }
    let claim_count = app.current_oracle_rewards()?.claims.len();
    let (start, end) = oracle_earn_visible_claim_window(layout.table_area, app, claim_count);
    let data_y = layout.table_area.y.saturating_add(2);
    let offset = usize::from(row.checked_sub(data_y)? / ORACLE_EARN_ROW_HEIGHT);
    let index = start + offset;
    (index < end).then_some(index)
}

pub(in super::super) fn oracle_earn_action_hit_at(area: Rect, column: u16, row: u16) -> bool {
    let layout = oracle_earn_layout(area);
    rect_contains(layout.action_area, column, row)
}
