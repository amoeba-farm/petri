//! Staking balances, forms, confirmations, and action presentation.

use super::super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct StakingScreenLayout {
    pub(in super::super) account: Rect,
    pub(in super::super) position: Option<Rect>,
    pub(in super::super) primary: Rect,
}

pub(in super::super) fn staking_screen_layout(
    area: Rect,
    workflow_active: bool,
) -> StakingScreenLayout {
    let account_height = if area.height >= 11 { 5 } else { 4 }.min(area.height);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(account_height), Constraint::Min(0)])
        .split(area);
    let account = rows[0];
    let content = rows[1];

    if workflow_active || area.height < 14 {
        return StakingScreenLayout {
            account,
            position: None,
            primary: content,
        };
    }

    if area.width >= 76 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(content);
        return StakingScreenLayout {
            account,
            position: Some(columns[0]),
            primary: columns[1],
        };
    }

    let position_height = content.height.saturating_sub(6).min(8);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(position_height), Constraint::Min(0)])
        .split(content);
    StakingScreenLayout {
        account,
        position: Some(sections[0]),
        primary: sections[1],
    }
}

pub(in super::super) fn draw_staking_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::Staking;
    let workflow_active = app.staking_confirmation.is_some() || app.staking_form.is_some();
    let layout = staking_screen_layout(area, workflow_active);
    let compact_account = layout.position.is_none() && !workflow_active;

    let account = Paragraph::new(staking_account_lines(cli, app, compact_account))
        .block(panel_block(cli, "staking account", Color::Cyan, false))
        .wrap(Wrap { trim: true });
    frame.render_widget(account, layout.account);

    if workflow_active {
        let (title, color, lines) = staking_workflow_lines(cli, app);
        if let Some((cancel, confirm)) = staking_confirmation_button_rects(area, app) {
            let block = panel_block(cli, &title, color, focused);
            let inner = block.inner(layout.primary);
            frame.render_widget(block, layout.primary);
            let content = Rect {
                height: cancel.y.saturating_sub(inner.y).saturating_sub(1),
                ..inner
            };
            let confirmation_lines = app
                .staking_confirmation
                .as_ref()
                .map(|confirmation| staking_confirmation_review_lines(cli, app, confirmation))
                .unwrap_or(lines);
            frame.render_widget(
                Paragraph::new(confirmation_lines)
                    .style(tui_panel_style(cli))
                    .wrap(Wrap { trim: true })
                    .scroll((
                        app.focused_panel_scroll(LabFocus::Staking)
                            .min(u16::MAX as usize) as u16,
                        0,
                    )),
                content,
            );
            let choice = app
                .staking_confirmation
                .as_ref()
                .map(|confirmation| confirmation.choice)
                .unwrap_or(StakingConfirmationChoice::Cancel);
            frame.render_widget(
                Paragraph::new(raised_button_lines(
                    cli,
                    "CANCEL",
                    Color::Blue,
                    choice == StakingConfirmationChoice::Cancel,
                    cancel.width,
                    cancel.height,
                )),
                cancel,
            );
            frame.render_widget(
                Paragraph::new(raised_button_lines(
                    cli,
                    "CONFIRM & SIGN",
                    Color::Green,
                    choice == StakingConfirmationChoice::Confirm,
                    confirm.width,
                    confirm.height,
                )),
                confirm,
            );
        } else {
            let panel = scrolling_panel(
                lines,
                layout.primary,
                cli,
                app.focused_panel_scroll(LabFocus::Staking),
                focused,
            )
            .block(panel_block(cli, &title, color, focused));
            frame.render_widget(panel, layout.primary);
        }
        return;
    }

    if let Some(position_area) = layout.position {
        let compact = panel_inner_height(position_area) < 9;
        let position = Paragraph::new(staking_position_lines(cli, app, compact))
            .block(panel_block(cli, "your position", Color::Cyan, false))
            .wrap(Wrap { trim: true });
        frame.render_widget(position, position_area);
    }

    let action_height = panel_inner_height(layout.primary);
    let actions = Paragraph::new(staking_action_panel_lines(
        cli,
        app,
        action_height,
        layout.position.is_none(),
    ))
    .block(panel_block(cli, "actions", Color::Yellow, focused))
    .wrap(Wrap { trim: true });
    frame.render_widget(actions, layout.primary);
}

pub(in super::super) fn staking_missing_lines_below(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
) -> Option<usize> {
    let workflow_active = app.staking_confirmation.is_some() || app.staking_form.is_some();
    if !workflow_active {
        return None;
    }
    let layout = staking_screen_layout(area, true);
    let (_, _, lines) = staking_workflow_lines(cli, app);
    wrapped_missing_lines_for_panel(
        &lines,
        layout.primary,
        app.focused_panel_scroll(LabFocus::Staking),
        true,
    )
}

pub(in super::super) fn staking_confirmation_button_rects(
    area: Rect,
    app: &LabApp,
) -> Option<(Rect, Rect)> {
    if app.staking_confirmation.is_none() || app.staking_action_is_running() {
        return None;
    }
    let primary = staking_screen_layout(area, true).primary;
    let inner = panel_inner_rect(primary)?;
    if inner.width < 24 || inner.height < 7 {
        return None;
    }
    let gap = if inner.width >= 50 { 3 } else { 1 };
    let available = inner.width.saturating_sub(gap);
    let cancel_width = available / 2;
    let confirm_width = available.saturating_sub(cancel_width);
    let y = inner.y.saturating_add(inner.height.saturating_sub(3));
    Some((
        Rect {
            x: inner.x,
            y,
            width: cancel_width,
            height: 3,
        },
        Rect {
            x: inner.x.saturating_add(cancel_width).saturating_add(gap),
            y,
            width: confirm_width,
            height: 3,
        },
    ))
}

pub(in super::super) fn staking_confirmation_button_at(
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<StakingConfirmationChoice> {
    let (cancel, confirm) = staking_confirmation_button_rects(area, app)?;
    if rect_contains(cancel, column, row) {
        Some(StakingConfirmationChoice::Cancel)
    } else if rect_contains(confirm, column, row) {
        Some(StakingConfirmationChoice::Confirm)
    } else {
        None
    }
}

pub(in super::super) fn staking_account_lines(
    cli: &Cli,
    app: &LabApp,
    compact: bool,
) -> Vec<Line<'static>> {
    let account = app
        .wallet
        .pubkey
        .as_deref()
        .map(short_pubkey)
        .unwrap_or_else(|| "not attached".to_string());
    let mut lines = vec![Line::from(vec![
        Span::styled("Attached account  ", style(cli, Color::DarkGray)),
        Span::styled(
            account,
            style(
                cli,
                if app.wallet.is_attached() {
                    Color::Green
                } else {
                    Color::Yellow
                },
            )
            .add_modifier(Modifier::BOLD),
        ),
    ])];

    if compact {
        lines.push(staking_compact_balance_line(cli, app));
        return lines;
    }

    lines.extend([
        Line::from(Span::styled(
            "sAMBA is your transferable share of staked AMBA.",
            style(cli, Color::White),
        )),
        Line::from(Span::styled(
            "Staking rewards are already included in Redeemable AMBA.",
            style(cli, Color::Green).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "New AMBA waits seven days before activation; queued AMBA earns no rewards.",
            style(cli, Color::DarkGray),
        )),
    ]);
    lines
}

pub(in super::super) fn staking_position_lines(
    cli: &Cli,
    app: &LabApp,
    compact: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.loading_staking {
        lines.push(Line::from(Span::styled(
            format!("{} Reading staking balances...", app.spinner()),
            style(cli, Color::Cyan),
        )));
    }
    if let Some(issue) = app.staking_issue.as_deref() {
        lines.push(Line::from(Span::styled(
            issue.to_string(),
            style(cli, Color::Yellow),
        )));
    }

    if let Some(status) = app.staking_status.as_ref() {
        lines.extend(if compact {
            staking_compact_balance_lines(cli, status)
        } else {
            staking_balance_lines(cli, status)
        });
    } else if !app.loading_staking && app.staking_issue.is_none() {
        lines.push(Line::from("Staking balances are not loaded."));
        lines.push(Line::from("Press r to refresh."));
    }
    lines
}

pub(in super::super) fn staking_compact_balance_line(cli: &Cli, app: &LabApp) -> Line<'static> {
    if app.loading_staking {
        return Line::from(Span::styled(
            format!("{} Reading balances...", app.spinner()),
            style(cli, Color::Cyan),
        ));
    }
    if let Some(issue) = app.staking_issue.as_deref() {
        return Line::from(Span::styled(
            truncate_text(issue, 72),
            style(cli, Color::Yellow),
        ));
    }
    let Some(status) = app.staking_status.as_ref() else {
        return Line::from(Span::styled(
            "Balances not loaded • r refreshes",
            style(cli, Color::DarkGray),
        ));
    };
    if !staking_status_is_available(status) {
        return Line::from(Span::styled(
            truncate_text(&staking_availability_note(status), 72),
            style(cli, Color::Yellow),
        ));
    }

    let available = staking_display_value(
        status,
        &["availableAmbaBalance", "availableAmba", "available_amba"],
    );
    let samba = staking_display_value(
        status,
        &[
            "stakedSamba",
            "staked_samba",
            "sambaBalance",
            "samba_balance",
            "sAMBA",
            "samba",
        ],
    );
    let pending = staking_display_value(
        status,
        &[
            "ownerPendingUnstakeAmba",
            "owner_pending_unstake_amba",
            "pendingUnstakeAmba",
            "pending_unstake_amba",
            "unbondingAmba",
            "unbonding_amba",
        ],
    );
    Line::from(vec![
        Span::styled(
            format!("{available} AMBA"),
            style(cli, Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ready • ", style(cli, Color::DarkGray)),
        Span::styled(
            format!("{samba} sAMBA"),
            style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" • ", style(cli, Color::DarkGray)),
        Span::styled(format!("{pending} unbonding"), style(cli, Color::White)),
    ])
}

pub(in super::super) fn staking_workflow_lines(
    cli: &Cli,
    app: &LabApp,
) -> (String, Color, Vec<Line<'static>>) {
    if let Some(confirmation) = app.staking_confirmation.as_ref() {
        return (
            format!(
                "review {}",
                confirmation.action.label().to_ascii_lowercase()
            ),
            Color::Yellow,
            staking_confirmation_lines(cli, app, confirmation),
        );
    }
    if let Some(form) = app.staking_form.as_ref() {
        return (
            form.action.label().to_ascii_lowercase(),
            form.action.color(),
            staking_form_lines(cli, form),
        );
    }
    ("actions".to_string(), Color::Yellow, Vec::new())
}

pub(in super::super) fn staking_balance_lines(cli: &Cli, payload: &Value) -> Vec<Line<'static>> {
    if !staking_status_is_available(payload) {
        let state = staking_status_value(Some(payload), &["stakingState", "staking_state"])
            .unwrap_or_else(|| "unavailable".to_string());
        return vec![
            Line::from(vec![
                Span::styled("Staking state  ", style(cli, Color::DarkGray)),
                Span::styled(
                    state,
                    style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                staking_availability_note(payload),
                style(cli, Color::Yellow),
            )),
            Line::from(Span::styled(
                "Refresh remains available; queue, activate, cancel, unstake, and claim are disabled.",
                style(cli, Color::DarkGray),
            )),
        ];
    }
    let available = staking_display_value(
        payload,
        &["availableAmbaBalance", "availableAmba", "available_amba"],
    );
    let samba = staking_display_value(
        payload,
        &[
            "stakedSamba",
            "staked_samba",
            "sambaBalance",
            "samba_balance",
            "sAMBA",
            "samba",
        ],
    );
    let redeemable = staking_display_value(
        payload,
        &["redeemableAmba", "redeemable_amba", "redeemableAmbaBalance"],
    );
    let voting = staking_display_value(
        payload,
        &[
            "votingPowerSamba",
            "voting_power_samba",
            "votingPower",
            "voting_power",
            "sambaVotingPower",
        ],
    );
    let queued = staking_display_value(payload, &["queuedAmba", "queued_amba"]);
    let queued_positive = staking_status_value_is_positive(
        payload,
        &[
            "queuedAmba",
            "queued_amba",
            "queuedAmbaAtoms",
            "queued_amba_atoms",
        ],
    );
    let pending = staking_display_value(
        payload,
        &[
            "ownerPendingUnstakeAmba",
            "owner_pending_unstake_amba",
            "pendingUnstakeAmba",
            "pending_unstake_amba",
            "unbondingAmba",
            "unbonding_amba",
        ],
    );
    let pending_positive = staking_status_value_is_positive(
        payload,
        &[
            "ownerPendingUnstakeAmba",
            "owner_pending_unstake_amba",
            "pendingUnstakeAmba",
            "pending_unstake_amba",
            "unbondingAmba",
            "unbonding_amba",
        ],
    );
    let protocol_paused = staking_status_flag(payload, &["protocolPaused", "protocol_paused"]);
    let supply_locked =
        staking_status_flag(payload, &["supplyChangesLocked", "supply_changes_locked"])
            || staking_status_number(payload, &["governanceLockCount", "governance_lock_count"])
                .is_some_and(|count| count > 0.0);

    let mut lines = vec![
        staking_balance_line(cli, "Available AMBA", &available, Color::Green),
        staking_balance_line(cli, "sAMBA", &samba, Color::Cyan),
        staking_balance_line(cli, "Redeemable AMBA", &redeemable, Color::Magenta),
        staking_balance_line(cli, "Voting power", &voting, Color::Yellow),
        staking_balance_line(cli, "Queued AMBA", &queued, Color::Blue),
        staking_balance_line(cli, "Unbonding AMBA", &pending, Color::White),
    ];

    if let Some(rate) = staking_status_value(
        Some(payload),
        &["exchangeRate", "exchange_rate", "ambaPerSamba"],
    ) {
        lines.push(Line::from(vec![
            Span::styled("Share rate        ", style(cli, Color::DarkGray)),
            Span::styled(
                format!("1 sAMBA = {} AMBA", truncate_text(&rate, 30)),
                style(cli, Color::Magenta),
            ),
        ]));
    }

    let activation_ready =
        staking_status_flag(payload, &["canActivate", "can_activate", "activationReady"]);
    let activation_wait = if !queued_positive {
        "No AMBA is queued for activation.".to_string()
    } else if activation_ready {
        "Matured, but wallet changes are unavailable in the current release.".to_string()
    } else if let Some(seconds) = staking_status_u64(
        payload,
        &["secondsUntilActivation", "seconds_until_activation"],
    )
    .filter(|seconds| *seconds > 0)
    {
        format!(
            "Seven-day activation wait: {} remaining.",
            format_staking_countdown(seconds)
        )
    } else {
        staking_status_value(
            Some(payload),
            &["activationBlockedReason", "activation_blocked_reason"],
        )
        .unwrap_or_else(|| "Queued AMBA is waiting to activate.".to_string())
    };
    lines.push(Line::from(vec![
        Span::styled("Activation  ", style(cli, Color::DarkGray)),
        Span::styled(
            activation_wait,
            style(
                cli,
                if activation_ready {
                    Color::Green
                } else {
                    Color::White
                },
            ),
        ),
    ]));

    let unstake_ready = staking_status_flag(
        payload,
        &["unstakeReady", "unstake_ready", "canClaim", "can_claim"],
    );
    let wait = if !pending_positive {
        "No AMBA is waiting to finish unstaking.".to_string()
    } else if unstake_ready && protocol_paused {
        "Ready when staking actions resume.".to_string()
    } else if unstake_ready {
        "Matured, but wallet changes are unavailable in the current release.".to_string()
    } else if let Some(seconds) = staking_status_u64(
        payload,
        &[
            "secondsUntilUnstakeReady",
            "seconds_until_unstake_ready",
            "secondsUntilClaimable",
            "seconds_until_claimable",
            "unstakeSecondsRemaining",
        ],
    ) {
        format!(
            "Seven-day wait: {} remaining.",
            format_staking_countdown(seconds)
        )
    } else if let Some(claimable_at) = staking_status_value(
        Some(payload),
        &[
            "unstakeClaimableAt",
            "unstakeClaimableAtTs",
            "unstake_claimable_at",
            "unstake_claimable_at_ts",
            "claimableAtUnix",
            "claimable_at_unix",
        ],
    ) {
        format!(
            "Seven-day wait: ready at {}.",
            format_staking_claimable_at(&claimable_at)
        )
    } else {
        "Seven-day unstaking wait is in progress.".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("Unstaking  ", style(cli, Color::DarkGray)),
        Span::styled(
            wait,
            style(
                cli,
                if unstake_ready && !protocol_paused {
                    Color::Green
                } else {
                    Color::White
                },
            ),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Activate / unstake  ", style(cli, Color::DarkGray)),
        Span::styled(
            if protocol_paused {
                "temporarily paused for this deployment"
            } else if supply_locked {
                "paused for an unresolved emergency vote"
            } else {
                "available"
            },
            style(
                cli,
                if protocol_paused || supply_locked {
                    Color::Yellow
                } else {
                    Color::Green
                },
            )
            .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines
}

pub(in super::super) fn staking_compact_balance_lines(
    cli: &Cli,
    payload: &Value,
) -> Vec<Line<'static>> {
    let lines = staking_balance_lines(cli, payload);
    if !staking_status_is_available(payload) || lines.len() <= 6 {
        return lines;
    }

    let mut compact = lines.iter().take(3).cloned().collect::<Vec<_>>();
    if let Some(unbonding) = lines.get(4) {
        compact.push(unbonding.clone());
    }
    compact.extend(lines.iter().rev().take(2).cloned().rev());
    compact
}

pub(in super::super) fn staking_display_value(payload: &Value, keys: &[&str]) -> String {
    staking_status_value(Some(payload), keys)
        .map(|value| truncate_text(value.trim(), 36))
        .unwrap_or_else(|| "not reported".to_string())
}

pub(in super::super) fn format_staking_claimable_at(value: &str) -> String {
    let trimmed = value.trim();
    if let Ok(timestamp) = trimmed.parse::<i64>()
        && let Some(datetime) = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
    {
        return datetime.format("%Y-%m-%d %H:%M UTC").to_string();
    }
    truncate_text(trimmed, 48)
}

pub(in super::super) fn staking_balance_line(
    cli: &Cli,
    label: &'static str,
    amount: &str,
    color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<18}"), style(cli, Color::DarkGray)),
        Span::styled(
            amount.to_string(),
            style(cli, color).add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(in super::super) fn staking_action_panel_lines(
    cli: &Cli,
    app: &LabApp,
    height: usize,
    compact: bool,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }

    let show_detail = height >= 5;
    let show_keys = !compact && height >= 7;
    let mut lines = Vec::new();
    if show_keys {
        lines.push(Line::from(vec![
            Span::styled(
                "↑/↓",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" choose  ", style(cli, Color::DarkGray)),
            Span::styled(
                "Enter",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" checks  ", style(cli, Color::DarkGray)),
            Span::styled("r", style(cli, Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" refreshes", style(cli, Color::DarkGray)),
        ]));
    }
    if compact && height < StakingAction::ALL.len() {
        lines.extend(staking_compact_action_rows(cli, app));
    } else {
        lines.extend(staking_action_rows(cli, app, show_detail));
    }

    if let Some(result) = app.staking_action_result.as_ref()
        && lines.len() < height
    {
        lines.push(staking_action_result_line(cli, result));
    }
    if height >= 10 && lines.len() < height {
        lines.push(Line::from(""));
        if lines.len() < height {
            lines.push(Line::from(Span::styled(
                "Current release is read-only; staking wallet changes are unavailable.",
                style(cli, Color::DarkGray),
            )));
        }
    }
    lines.truncate(height);
    lines
}

pub(in super::super) fn staking_compact_action_rows(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    StakingAction::ALL
        .chunks(2)
        .map(|actions| {
            let mut spans = Vec::new();
            for (column, action) in actions.iter().copied().enumerate() {
                if column > 0 {
                    spans.push(Span::raw("  "));
                }
                let index = StakingAction::ALL
                    .iter()
                    .position(|candidate| *candidate == action)
                    .unwrap_or(0);
                let selected = index == app.staking_selected;
                let available = app.staking_action_availability(action).is_ok();
                let running = app.staking_action_is_running()
                    && app
                        .staking_confirmation
                        .as_ref()
                        .is_some_and(|confirmation| confirmation.action == action);
                let marker = if running {
                    "…"
                } else if available {
                    "✓"
                } else {
                    "–"
                };
                let action_style = if selected {
                    cell_style(cli, action.color(), true).add_modifier(Modifier::BOLD)
                } else if available {
                    style(cli, action.color()).add_modifier(Modifier::BOLD)
                } else {
                    style(cli, Color::DarkGray)
                };
                spans.push(Span::styled(
                    format!(
                        "{}  {:<20} {marker}",
                        if selected { ">" } else { " " },
                        action.label()
                    ),
                    action_style,
                ));
            }
            Line::from(spans)
        })
        .collect()
}

pub(in super::super) fn staking_action_rows(
    cli: &Cli,
    app: &LabApp,
    show_selected_detail: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, action) in StakingAction::ALL.iter().copied().enumerate() {
        let selected = index == app.staking_selected;
        let availability = app.staking_action_availability(action);
        let available = availability.is_ok();
        let running = app.staking_action_is_running()
            && app
                .staking_confirmation
                .as_ref()
                .is_some_and(|confirmation| confirmation.action == action);
        let state = if running {
            "running"
        } else if available {
            "ready"
        } else {
            "unavailable"
        };
        let action_style = if selected {
            cell_style(cli, action.color(), true).add_modifier(Modifier::BOLD)
        } else if available {
            style(cli, action.color()).add_modifier(Modifier::BOLD)
        } else {
            style(cli, Color::DarkGray)
        };
        let state_style = if running {
            style(cli, Color::Cyan).add_modifier(Modifier::BOLD)
        } else if available {
            style(cli, Color::Green)
        } else {
            style(cli, Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                style(
                    cli,
                    if selected {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    },
                )
                .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {:<20} ", action.label()), action_style),
            Span::raw(" "),
            Span::styled(state, state_style),
        ]));
        if selected && show_selected_detail {
            let detail = availability
                .err()
                .unwrap_or_else(|| action.detail().to_string());
            let detail = if detail.contains(crate::current_release::WRITE_ERROR_CODE) {
                "Read-only release: wallet changes are unavailable.".to_string()
            } else {
                detail
            };
            lines.push(Line::from(vec![
                Span::styled("  ↳ ", style(cli, Color::DarkGray)),
                Span::styled(
                    detail,
                    style(
                        cli,
                        if available {
                            Color::Gray
                        } else {
                            Color::Yellow
                        },
                    ),
                ),
            ]));
        }
    }
    lines
}

pub(in super::super) fn staking_action_result_line(
    cli: &Cli,
    result: &StakingActionResult,
) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "{} — {}: {}",
            if result.ok {
                "Complete"
            } else {
                "Not completed"
            },
            result.action.label(),
            result.message
        ),
        style(
            cli,
            if result.ok {
                Color::Green
            } else {
                Color::Yellow
            },
        )
        .add_modifier(Modifier::BOLD),
    ))
}

pub(in super::super) fn staking_form_lines(cli: &Cli, form: &StakingForm) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} details", form.action.label()),
            style(cli, form.action.color()).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(form.action.detail(), style(cli, Color::Gray))),
        Line::from(""),
    ];
    for field in form.fields() {
        let field = *field;
        let selected = form.field == field;
        let value = match field {
            StakingFormField::Amount => form.amount_input.as_str(),
            StakingFormField::MinimumReceived => form.minimum_received_input.as_str(),
        };
        let shown = if value.is_empty() { "_" } else { value };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                style(
                    cli,
                    if selected {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    },
                ),
            ),
            Span::styled(
                format!("{:<25}", field.label(form.action)),
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                shown.to_string(),
                if selected {
                    cell_style(cli, Color::White, true).add_modifier(Modifier::BOLD)
                } else {
                    style(cli, Color::White)
                },
            ),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Review comes next; nothing is signed or sent from this form.",
        style(cli, Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        if form.fields().len() > 1 {
            "Tab/Up/Down changes field • Enter reviews • Esc cancels • q quits Petri"
        } else {
            "Enter reviews • Esc cancels • q quits Petri"
        },
        style(cli, Color::DarkGray),
    )));
    lines
}

fn staking_confirmation_review_lines(
    cli: &Cli,
    app: &LabApp,
    confirmation: &StakingConfirmation,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Final review",
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Check the amount and outcome before signing.",
            style(cli, Color::Gray),
        )),
        Line::from(""),
    ];
    match confirmation.action {
        StakingAction::Stake => {
            lines.push(Line::from(format!(
                "Queue {} AMBA for seven days.",
                confirmation.amount.as_deref().unwrap_or("not reported")
            )));
            lines.push(Line::from(
                "Queued AMBA earns no rewards and provides no voting power.",
            ));
        }
        StakingAction::Activate => {
            lines.push(Line::from(format!(
                "Minimum received: {} sAMBA",
                confirmation
                    .minimum_received
                    .as_deref()
                    .unwrap_or("current verified quote (used as minimum)")
            )));
            lines.push(Line::from(
                "The complete queued AMBA amount activates at the current share rate.",
            ));
        }
        StakingAction::CancelQueue => {
            lines.push(Line::from(format!(
                "Return {} queued AMBA to the available balance.",
                confirmation.amount.as_deref().unwrap_or("the")
            )));
            lines.push(Line::from("No sAMBA will be minted."));
        }
        StakingAction::Unstake => {
            lines.push(Line::from(format!(
                "Unstake {} sAMBA",
                confirmation.amount.as_deref().unwrap_or("not reported")
            )));
            lines.push(Line::from(format!(
                "Minimum fixed amount: {} AMBA",
                confirmation
                    .minimum_received
                    .as_deref()
                    .unwrap_or("current verified quote (used as minimum)")
            )));
            lines.push(Line::from(
                "The sAMBA is burned now; the fixed AMBA amount waits seven days.",
            ));
        }
        StakingAction::Claim => {
            lines.push(Line::from(format!(
                "Claim {} ready AMBA into your available balance.",
                confirmation.amount.as_deref().unwrap_or("the")
            )));
        }
        StakingAction::Refresh => {}
    }
    lines.push(Line::from(""));
    if app.staking_action_is_running() {
        lines.push(Line::from(Span::styled(
            format!(
                "{} Signing and waiting for network confirmation...",
                app.spinner()
            ),
            style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
        )));
    }
    lines
}

pub(in super::super) fn staking_confirmation_lines(
    cli: &Cli,
    app: &LabApp,
    confirmation: &StakingConfirmation,
) -> Vec<Line<'static>> {
    let mut lines = staking_confirmation_review_lines(cli, app, confirmation);
    if app.staking_action_is_running() {
        return lines;
    }
    for choice in [
        StakingConfirmationChoice::Cancel,
        StakingConfirmationChoice::Confirm,
    ] {
        let selected = confirmation.choice == choice;
        let label = match choice {
            StakingConfirmationChoice::Cancel => "Cancel",
            StakingConfirmationChoice::Confirm => "Confirm and sign",
        };
        lines.push(Line::from(Span::styled(
            format!("{} [ {label} ]", if selected { ">" } else { " " }),
            if selected {
                cell_style(
                    cli,
                    if choice == StakingConfirmationChoice::Cancel {
                        Color::Yellow
                    } else {
                        Color::Green
                    },
                    true,
                )
                .add_modifier(Modifier::BOLD)
            } else {
                style(cli, Color::DarkGray)
            },
        )));
    }
    lines.push(Line::from(Span::styled(
        "Cancel is selected by default. Nothing is signed until you confirm.",
        style(cli, Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "Left/Right selects • Enter chooses • Esc cancels • q quits Petri",
        style(cli, Color::DarkGray),
    )));
    lines
}
