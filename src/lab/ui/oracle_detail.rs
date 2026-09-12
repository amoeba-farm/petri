//! Oracle recipe, live observation, action, form, and audit detail lines.

use super::super::*;

pub(in super::super) fn oracle_recipe_load_panel_message(app: &LabApp) -> String {
    if app.loading_oracle_tree {
        format!("{} Loading oracle source recipe...", app.spinner())
    } else if let Some(issue) = app.oracle_tree_issue.as_deref() {
        if oracle_tree_issue_is_retryable(issue) {
            format!(
                "{} Could not load oracle source recipe. Retrying...",
                app.spinner()
            )
        } else {
            issue.to_string()
        }
    } else {
        "Oracle source recipe is not loaded yet.".to_string()
    }
}

pub(in super::super) fn oracle_tree_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let Some(tree) = app.oracle_tree() else {
        let symbol = selected_oracle_market_symbol(app);
        let mut lines = vec![Line::from(Span::styled(
            format!("Oracle evidence / {symbol}"),
            style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
        ))];
        if app.loading_oracle_tree {
            lines.push(Line::from(Span::styled(
                oracle_recipe_load_panel_message(app),
                style(cli, Color::Yellow),
            )));
        } else if app.oracle_tree_issue.is_some() {
            lines.push(Line::from(Span::styled(
                oracle_recipe_load_panel_message(app),
                style(cli, Color::Yellow),
            )));
            lines.push(Line::from(Span::styled(
                "Press r to retry now.".to_string(),
                style(cli, Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                oracle_recipe_load_panel_message(app),
                style(cli, Color::DarkGray),
            )));
        }
        return lines;
    };

    let selected_index = app.selected_oracle_node_index();
    let selected = tree.node(selected_index);
    let path_indices = oracle_path_indices(tree, selected_index);
    let mut lines = vec![oracle_breadcrumb_line(cli, tree, &path_indices)];
    lines.push(Line::from(Span::styled(
        format!(
            "{} source recipe: {} product rows / {} source pins",
            tree.display_name, tree.row_bucket_count, tree.terminal_pin_count
        ),
        style(cli, Color::DarkGray),
    )));
    if let Some(issue) = app.oracle_tree_issue.as_deref() {
        lines.push(Line::from(Span::styled(
            format!("Refresh issue: {issue}"),
            style(cli, Color::DarkGray),
        )));
    }

    if app.detail.is_none() {
        let message = if app.loading_detail {
            format!(
                "{} Loading live market detail; source recipe is ready.",
                app.spinner()
            )
        } else {
            "Live market detail is not loaded; source recipe is ready.".to_string()
        };
        lines.push(Line::from(Span::styled(
            message,
            style(cli, Color::DarkGray),
        )));
    }

    lines.push(Line::from(vec![
        Span::styled("/ ", style(cli, Color::Yellow)),
        Span::styled(
            if app.oracle_search_editing {
                format!("search: {}_", app.oracle_search_input)
            } else if app.oracle_search_input.is_empty() {
                "search SKU/source/spec".to_string()
            } else {
                format!("search: {}", app.oracle_search_input)
            },
            style(cli, Color::Gray),
        ),
    ]));

    let matches = tree.search_nodes(&app.oracle_search_input);
    if !matches.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{} matches", matches.len()),
            style(cli, Color::Cyan),
        )));
        for index in matches.into_iter().take(10) {
            lines.push(oracle_tree_node_line(
                cli,
                tree,
                index,
                index == selected_index,
                "match",
            ));
        }
        return lines;
    }

    lines.push(Line::from(Span::styled(
        "Browse oracle tree. The open branch shows where you are drilling.",
        style(cli, Color::Gray),
    )));
    oracle_push_visible_tree_lines(
        cli,
        tree,
        tree.root_index(),
        selected_index,
        &path_indices,
        &mut lines,
    );

    lines.push(Line::from(vec![
        Span::styled("Backspace ", style(cli, Color::Yellow)),
        Span::styled("parent | Enter/Right drill | ", style(cli, Color::Gray)),
        Span::styled("[/] ", style(cli, Color::Yellow)),
        Span::styled("source jump", style(cli, Color::Gray)),
    ]));

    if matches!(
        selected.map(|node| node.kind),
        Some(OracleNodeKind::TerminalPin)
    ) {
        lines.push(Line::from(Span::styled(
            "This source is measured against its own opening value.",
            style(cli, Color::LightGreen),
        )));
    }

    lines
}

pub(in super::super) fn oracle_breadcrumb_line(
    cli: &Cli,
    tree: &OracleIndexTree,
    path_indices: &[usize],
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "Oracle Home".to_string(),
        style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
    )];
    for index in path_indices.iter().skip(1) {
        if let Some(node) = tree.node(*index) {
            spans.push(Span::styled(" > ", style(cli, Color::DarkGray)));
            spans.push(Span::styled(
                node.label.to_string(),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ));
        }
    }
    Line::from(spans)
}

pub(in super::super) fn oracle_path_indices(
    tree: &OracleIndexTree,
    selected_index: usize,
) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut current = Some(selected_index);
    while let Some(index) = current {
        indices.push(index);
        current = tree.parent(index);
    }
    indices.reverse();
    if indices.is_empty() {
        indices.push(tree.root_index());
    }
    indices
}

pub(in super::super) fn oracle_push_visible_tree_lines(
    cli: &Cli,
    tree: &OracleIndexTree,
    index: usize,
    selected_index: usize,
    path_indices: &[usize],
    lines: &mut Vec<Line<'static>>,
) {
    let on_path = path_indices.contains(&index);
    lines.push(oracle_tree_branch_line(
        cli,
        tree,
        index,
        index == selected_index,
        on_path,
    ));
    if !on_path {
        return;
    }

    for child in tree.child_indices(index) {
        if path_indices.contains(&child) {
            oracle_push_visible_tree_lines(cli, tree, child, selected_index, path_indices, lines);
        } else {
            lines.push(oracle_tree_branch_line(cli, tree, child, false, false));
        }
    }
}

pub(in super::super) fn oracle_tree_branch_line(
    cli: &Cli,
    tree: &OracleIndexTree,
    index: usize,
    selected: bool,
    on_path: bool,
) -> Line<'static> {
    let Some(node) = tree.node(index) else {
        return Line::from(Span::styled(
            "  unavailable oracle node".to_string(),
            cell_style(cli, Color::DarkGray, selected),
        ));
    };
    let depth = tree.node_depth(index);
    let indent = if depth == 0 {
        String::new()
    } else {
        format!("{}└─ ", "    ".repeat(depth.saturating_sub(1)))
    };
    let marker = if selected { ">" } else { " " };
    let expand = if !tree.child_indices(index).is_empty() {
        if on_path { "[-]" } else { "[+]" }
    } else {
        "   "
    };
    let weight = if node.kind == OracleNodeKind::TerminalPin {
        format!("{} | 1 source", format_percent(node.row_weight_pct))
    } else if node.pin_count == 1 {
        format!("{} | 1 source", format_percent(node.weight_pct))
    } else {
        format!(
            "{} | {} sources",
            format_percent(node.weight_pct),
            node.pin_count
        )
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} "),
            cell_style(cli, Color::Yellow, selected),
        ),
        Span::styled(indent, style(cli, Color::DarkGray)),
        Span::styled(format!("{expand} "), cell_style(cli, Color::Cyan, selected)),
        Span::styled(
            node.label.to_string(),
            cell_style(cli, Color::White, selected).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {weight}"),
            cell_style(cli, Color::Gray, selected),
        ),
    ])
}

pub(in super::super) fn oracle_current_path_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let Some(tree) = app.oracle_tree() else {
        return vec![Line::from(Span::styled(
            oracle_recipe_load_panel_message(app),
            status_style(cli, &app.status),
        ))];
    };

    let selected_index = app.selected_oracle_node_index();
    let path_indices = oracle_path_indices(tree, selected_index);
    let mut lines = vec![Line::from(Span::styled(
        "Oracle Home",
        style(cli, Color::White).add_modifier(Modifier::BOLD),
    ))];
    for (depth, index) in path_indices.iter().copied().enumerate() {
        let Some(node) = tree.node(index) else {
            continue;
        };
        let selected = index == selected_index;
        let indent = if depth == 0 {
            "└─ ".to_string()
        } else {
            format!("{}└─ ", "  ".repeat(depth))
        };
        let marker = if selected { "> " } else { "  " };
        let suffix = if selected { " (current)" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(marker, cell_style(cli, Color::Yellow, selected)),
            Span::styled(indent, style(cli, Color::DarkGray)),
            Span::styled(
                node.label.to_string(),
                cell_style(cli, Color::White, selected).add_modifier(Modifier::BOLD),
            ),
            Span::styled(suffix, cell_style(cli, Color::Yellow, selected)),
        ]));
    }

    let definition = tree
        .node(selected_index)
        .and_then(|node| node.description.as_deref())
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .unwrap_or("No definition is available for this oracle node.");

    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Definition",
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            definition.to_string(),
            style(cli, Color::Gray),
        )),
    ]);

    lines.push(Line::from(""));
    lines.extend(oracle_live_observation_lines(cli, app));

    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Enter/Right = drill down",
            style(cli, Color::LightGreen),
        )),
        Line::from(Span::styled(
            "Backspace/Left = move up",
            style(cli, Color::LightGreen),
        )),
        Line::from(Span::styled(
            "Tab = next panel",
            style(cli, Color::DarkGray),
        )),
    ]);
    lines
}

pub(in super::super) fn current_spread_oracle_live(app: &LabApp) -> Option<&SpreadOracleLiveState> {
    let market_id = app.selected_id();
    app.oracle_live.as_ref().filter(|state| {
        state.market_id.eq_ignore_ascii_case(&market_id)
            && app
                .selected_chart_expiry()
                .is_none_or(|expiry| state.expiry_id.eq_ignore_ascii_case(&expiry.id))
    })
}

pub(in super::super) fn spread_oracle_observation_matches_node(
    observation: &SpreadOracleObservation,
    node: &RamxOracleNode,
) -> bool {
    oracle_identifier_matches(&observation.source, &node.node_id)
        || oracle_identifier_matches(&observation.source, &node.label)
        || oracle_identifier_matches(&observation.source_id_hex, &node.node_id)
        || oracle_identifier_matches(&observation.source_id_hex, &node.label)
}

pub(in super::super) fn oracle_submission_matches_node(
    record: &OracleSubmissionRecord,
    node: &RamxOracleNode,
) -> bool {
    oracle_identifier_matches(&record.node_label, &node.node_id)
        || oracle_identifier_matches(&record.node_label, &node.label)
}

pub(in super::super) fn oracle_source_status_has_unresolved_challenge(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "challenged" | "challenge" | "open_challenge" | "rule_review" | "rule_review_unresolved"
    )
}

pub(in super::super) fn oracle_identifier_matches(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left.eq_ignore_ascii_case(right) || oracle_identifier_key(left) == oracle_identifier_key(right)
}

pub(in super::super) fn oracle_identifier_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(in super::super) fn oracle_live_observation_lines(
    cli: &Cli,
    app: &LabApp,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Live observations",
        style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
    ))];

    if app.loading_oracle_live {
        lines.push(Line::from(Span::styled(
            format!("{} Loading source observations...", app.spinner()),
            style(cli, Color::Yellow),
        )));
        return lines;
    }

    let Some(live) = current_spread_oracle_live(app) else {
        let message = if app.oracle_live_issue.is_some() {
            "Live source observations are temporarily unavailable."
        } else {
            "No live source observations are loaded yet."
        };
        lines.push(Line::from(Span::styled(message, style(cli, Color::Yellow))));
        return lines;
    };

    let emergency_count = live.active_emergency_count();
    let summary_style = if emergency_count > 0 {
        oracle_emergency_style(cli, app)
    } else {
        style(cli, Color::Gray)
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} source observation", live.observations.len()),
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if live.observations.len() == 1 {
                "".to_string()
            } else {
                "s".to_string()
            },
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | emergency ", style(cli, Color::DarkGray)),
        Span::styled(
            if emergency_count > 0 {
                format!("{emergency_count} active")
            } else {
                "clear".to_string()
            },
            summary_style,
        ),
    ]));

    let scheme_version = live
        .weight_scheme_version
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| "version unknown".to_string());
    let manifest = live
        .weight_manifest_hash_hex
        .as_deref()
        .filter(|hash| !hash.trim().is_empty())
        .map(|hash| short_path(hash, 18))
        .unwrap_or_else(|| "none".to_string());
    let effective_total = live
        .effective_weight_total_bps
        .map(|weight| format!("{:.2}%", weight / 100.0))
        .unwrap_or_else(|| "unverified".to_string());
    lines.push(Line::from(Span::styled(
        format!(
            "Weight scheme: {} ({scheme_version}) | effective total {effective_total} | manifest {manifest} | {}",
            live.weight_scheme,
            if live.weight_verified {
                "VERIFIED"
            } else {
                "UNVERIFIED"
            }
        ),
        style(
            cli,
            if live.weight_verified {
                Color::Green
            } else {
                Color::Yellow
            },
        ),
    )));
    if !live.weight_verified {
        lines.push(Line::from(Span::styled(
            format!(
                "Weight verification: {}. Source weights are not verified global settlement weights.",
                live.weight_verification_status
            ),
            style(cli, Color::Yellow),
        )));
    }
    let active_scheme_version = live
        .active_weight_scheme_version
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| "version unknown".to_string());
    let active_manifest = live
        .active_weight_manifest_hash_hex
        .as_deref()
        .filter(|hash| !hash.trim().is_empty())
        .map(|hash| short_path(hash, 18))
        .unwrap_or_else(|| "none".to_string());
    lines.push(Line::from(Span::styled(
        format!(
            "Post-opening active weights: {active_scheme_version} | manifest {active_manifest} | {}",
            if live.active_weight_verified {
                "FINALIZED"
            } else {
                "NOT FINALIZED"
            }
        ),
        style(
            cli,
            if live.active_weight_verified {
                Color::Green
            } else {
                Color::Yellow
            },
        ),
    )));
    if !live.active_weight_verified {
        lines.push(Line::from(Span::styled(
            format!(
                "Active-weight readiness: {}. Updates, DLMM eligibility, settlement, and closeout remain blocked.",
                live.active_weight_verification_status
            ),
            style(cli, Color::Yellow),
        )));
    }

    match live.pending_resolution_count {
        Some(0) => lines.push(Line::from(Span::styled(
            "Day-7 freeze barrier clear: no pending source dispute resolutions.",
            style(cli, Color::Green),
        ))),
        Some(count) => lines.push(Line::from(Span::styled(
            format!(
                "Day-7 freeze blocked: {count} pending source dispute resolutions must reach ordinary or emergency terminal resolution."
            ),
            style(cli, Color::Yellow),
        ))),
        None => lines.push(Line::from(Span::styled(
            "Day-7 freeze readiness unknown: verify pendingResolutionCount is zero on-chain.",
            style(cli, Color::DarkGray),
        ))),
    }

    if live.observations.is_empty() {
        lines.push(Line::from(Span::styled(
            "This month has no source rows from the spread oracle read yet.",
            style(cli, Color::DarkGray),
        )));
    } else {
        for observation in live.observations.iter().take(10) {
            lines.push(spread_oracle_observation_line(cli, app, observation));
        }
    }

    if !live.escrows.is_empty() {
        let eligible = live
            .escrows
            .iter()
            .filter(|escrow| escrow.settlement_eligible)
            .count();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "Stake and bond status | {} record{} | {eligible} ready to settle",
                live.escrows.len(),
                if live.escrows.len() == 1 { "" } else { "s" }
            ),
            style(
                cli,
                if eligible > 0 {
                    Color::LightGreen
                } else {
                    Color::Gray
                },
            ),
        )));
        for escrow in live.escrows.iter().take(6) {
            lines.push(spread_oracle_escrow_line(cli, escrow));
        }
    }

    for issue in live.issues.iter().take(2) {
        lines.push(Line::from(Span::styled(
            format!("note: {issue}"),
            style(cli, Color::Yellow),
        )));
    }

    lines
}

pub(in super::super) fn spread_oracle_escrow_line(
    cli: &Cli,
    escrow: &SpreadOracleEscrow,
) -> Line<'static> {
    let kind = escrow.kind.replace('_', " ");
    let state = if escrow.settlement_eligible {
        "ready to settle".to_string()
    } else if escrow.disposition == "unsettled" {
        format!("waiting ({})", escrow.terminal_outcome)
    } else {
        escrow.disposition.clone()
    };
    Line::from(vec![
        Span::styled("   ", style(cli, Color::DarkGray)),
        Span::styled(
            format!("{kind} {}", escrow.amount_label),
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " | {state} | owner {} | record {}",
                short_pubkey(&escrow.owner_pubkey),
                short_pubkey(&escrow.subject_pda)
            ),
            style(
                cli,
                if escrow.settlement_eligible {
                    Color::LightGreen
                } else {
                    Color::Gray
                },
            ),
        ),
    ])
}

pub(in super::super) fn spread_oracle_observation_line(
    cli: &Cli,
    app: &LabApp,
    observation: &SpreadOracleObservation,
) -> Line<'static> {
    let has_emergency = observation.emergency.is_some();
    let marker_style = if has_emergency {
        oracle_emergency_style(cli, app)
    } else {
        style(cli, Color::DarkGray)
    };
    let text_style = if has_emergency {
        oracle_emergency_style(cli, app)
    } else {
        style(cli, Color::White)
    };
    let deadline = observation
        .opening_challenge_deadline_slot
        .map(|slot| format!("; challenge deadline slot {slot}"))
        .unwrap_or_default();
    let finalize = opening_challenge_status_suffix(observation.opening_finalizable);
    let opening = format!(
        "{}{}{}",
        observation.opening_status.display_label(),
        deadline,
        finalize
    );
    let emergency_label = observation
        .emergency
        .as_ref()
        .map(|emergency| format!(" | EMERGENCY {} {}", emergency.kind, emergency.status))
        .unwrap_or_default();
    let archive_label = observation
        .opening_archive_url
        .as_deref()
        .map(|archive_url| format!(" | archive {}", short_wayback_archive_url(archive_url)))
        .unwrap_or_default();
    let bucket_weight = observation
        .bucket_weight_bps
        .map(|weight| format!("{:.2}%", weight / 100.0))
        .unwrap_or_else(|| "unknown".to_string());
    let frozen_label = if observation.weight_verified {
        format!(
            "frozen {:.2}% / effective {:.2}% VERIFIED AUDIT",
            observation.frozen_weight_bps / 100.0,
            observation.effective_weight_bps.unwrap_or(0.0) / 100.0
        )
    } else {
        format!(
            "frozen {:.2}% / effective UNVERIFIED",
            observation.frozen_weight_bps / 100.0
        )
    };
    let active_label = if observation.active_weight_verified {
        format!(
            "active SKU {:.2}% / effective {:.2}% VERIFIED",
            observation.active_weight_bps.unwrap_or(0.0) / 100.0,
            observation.active_effective_weight_bps.unwrap_or(0.0) / 100.0
        )
    } else {
        "active weights NOT FINALIZED".to_string()
    };

    Line::from(vec![
        Span::styled(
            if has_emergency { "!! " } else { "   " }.to_string(),
            marker_style,
        ),
        Span::styled(
            short_path(&observation.source, 18),
            text_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" / {}", short_path(&observation.source_id_hex, 14)),
            style(cli, Color::DarkGray),
        ),
        Span::styled(
            format!(
                " | {} | current {} | bucket {bucket_weight} | {frozen_label} | {active_label} | support {}{}{}",
                opening,
                observation.current_state,
                observation.support_stake_total,
                archive_label,
                emergency_label
            ),
            if has_emergency {
                oracle_emergency_style(cli, app)
            } else {
                style(cli, Color::Gray)
            },
        ),
    ])
}

pub(in super::super) fn opening_challenge_status_suffix(finalizable: bool) -> &'static str {
    if finalizable {
        "; challenge window closed"
    } else {
        ""
    }
}

pub(in super::super) fn short_wayback_archive_url(archive_url: &str) -> String {
    const PREFIX: &str = "https://web.archive.org/web/";
    let Some((capture, target)) = archive_url
        .strip_prefix(PREFIX)
        .and_then(|suffix| suffix.split_once('/'))
    else {
        return short_path(archive_url, 36);
    };
    format!("web.archive.org/web/{capture}/{}", short_path(target, 18))
}

pub(in super::super) fn panel_inner_height(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

// Shared overflow attachment for bordered text panels. This measures wrapped terminal rows before
// rendering and always reserves notice rows instead of letting Paragraph clip content silently.
pub(in super::super) fn scroll_lines_to_panel(
    lines: Vec<Line<'static>>,
    area: Rect,
    cli: &Cli,
    scroll: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    scroll_wrapped_lines_to_panel(lines, area, cli, scroll, focused, true)
}

pub(in super::super) fn scroll_wrapped_lines_to_panel(
    lines: Vec<Line<'static>>,
    area: Rect,
    cli: &Cli,
    scroll: usize,
    focused: bool,
    trim: bool,
) -> Vec<Line<'static>> {
    let height = panel_inner_height(area);
    let width = area.width.saturating_sub(2);
    if height == 0 || width == 0 {
        return Vec::new();
    }

    let viewport = wrapped_line_viewport(&lines, width, height, scroll, trim);
    if viewport.hidden_count() == 0 {
        return lines;
    }
    if height == 1 {
        return vec![hidden_lines_notice(
            cli,
            "↓",
            viewport.hidden_count(),
            focused,
        )];
    }

    let mut visible = Vec::new();
    if viewport.hidden_above > 0 {
        visible.push(hidden_lines_notice(
            cli,
            "↑",
            viewport.hidden_above,
            focused,
        ));
    }
    visible.extend(lines[viewport.start..viewport.end].iter().cloned());
    if viewport.hidden_below > 0 {
        visible.push(hidden_lines_notice(
            cli,
            "↓",
            viewport.hidden_below,
            focused,
        ));
    }
    visible
}

pub(in super::super) fn wrapped_line_count(
    lines: &[Line<'static>],
    width: u16,
    trim: bool,
) -> usize {
    if lines.is_empty() || width == 0 {
        return 0;
    }
    lines
        .iter()
        .map(|line| wrapped_line_height(line, usize::from(width), trim))
        .sum()
}

pub(in super::super) fn wrapped_line_height(
    line: &Line<'static>,
    width: usize,
    trim: bool,
) -> usize {
    if width == 0 {
        return 0;
    }
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if text.is_empty() {
        return 1;
    }
    if !trim {
        return text_width(&text).div_ceil(width).max(1);
    }

    let mut rows = 1usize;
    let mut column = 0usize;
    for word in text.split_whitespace() {
        let mut word_width = text_width(word);
        if word_width == 0 {
            continue;
        }
        let separator = usize::from(column > 0);
        if column.saturating_add(separator).saturating_add(word_width) <= width {
            column = column.saturating_add(separator).saturating_add(word_width);
            continue;
        }
        if column > 0 {
            rows = rows.saturating_add(1);
        }
        if word_width > width {
            rows = rows.saturating_add(word_width.saturating_sub(1) / width);
            word_width %= width;
        }
        column = word_width;
    }
    rows
}

pub(in super::super) fn wrapped_lines_end(
    lines: &[Line<'static>],
    start: usize,
    capacity: usize,
    width: u16,
    trim: bool,
) -> usize {
    if capacity == 0 {
        return start;
    }
    let mut end = start;
    while end < lines.len() && wrapped_line_count(&lines[start..=end], width, trim) <= capacity {
        end += 1;
    }
    end
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in super::super) struct WrappedLineViewport {
    pub(in super::super) start: usize,
    pub(in super::super) end: usize,
    pub(in super::super) hidden_above: usize,
    pub(in super::super) hidden_below: usize,
}

impl WrappedLineViewport {
    pub(in super::super) fn hidden_count(self) -> usize {
        self.hidden_above.saturating_add(self.hidden_below)
    }
}

pub(in super::super) fn wrapped_line_viewport(
    lines: &[Line<'static>],
    width: u16,
    height: usize,
    scroll: usize,
    trim: bool,
) -> WrappedLineViewport {
    let total_rows = wrapped_line_count(lines, width, trim);
    if total_rows == 0 {
        return WrappedLineViewport::default();
    }
    if height == 0 {
        return WrappedLineViewport {
            hidden_below: total_rows,
            ..WrappedLineViewport::default()
        };
    }
    if total_rows <= height {
        return WrappedLineViewport {
            end: lines.len(),
            ..WrappedLineViewport::default()
        };
    }
    if height == 1 {
        return WrappedLineViewport {
            hidden_below: total_rows,
            ..WrappedLineViewport::default()
        };
    }

    let start = scroll.min(lines.len().saturating_sub(1));
    let hidden_above = wrapped_line_count(&lines[..start], width, trim);
    let top_notice_rows = usize::from(hidden_above > 0);
    let capacity_without_bottom = height.saturating_sub(top_notice_rows);
    let mut end = wrapped_lines_end(lines, start, capacity_without_bottom, width, trim);
    if end < lines.len() {
        end = wrapped_lines_end(
            lines,
            start,
            capacity_without_bottom.saturating_sub(1),
            width,
            trim,
        );
    }
    let hidden_below = wrapped_line_count(&lines[end..], width, trim);

    WrappedLineViewport {
        start,
        end,
        hidden_above,
        hidden_below,
    }
}

pub(in super::super) fn wrapped_missing_lines_for_panel(
    lines: &[Line<'static>],
    area: Rect,
    scroll: usize,
    trim: bool,
) -> Option<usize> {
    let hidden = wrapped_line_viewport(
        lines,
        area.width.saturating_sub(2),
        panel_inner_height(area),
        scroll,
        trim,
    )
    .hidden_count();
    (hidden > 0).then_some(hidden)
}

pub(in super::super) fn clip_lines_to_panel(
    lines: Vec<Line<'static>>,
    area: Rect,
    cli: &Cli,
    scroll: usize,
) -> Vec<Line<'static>> {
    scroll_lines_to_height(lines, panel_inner_height(area), cli, scroll, false)
}

pub(in super::super) fn scroll_lines_to_height(
    lines: Vec<Line<'static>>,
    height: usize,
    cli: &Cli,
    scroll: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let viewport = line_viewport(lines.len(), height, scroll);
    if viewport.hidden_count() == 0 {
        return lines;
    }
    if height == 1 {
        return vec![hidden_lines_notice(cli, "↓", lines.len(), focused)];
    }

    let mut visible = Vec::new();
    if viewport.hidden_above > 0 {
        visible.push(hidden_lines_notice(
            cli,
            "↑",
            viewport.hidden_above,
            focused,
        ));
    }
    visible.extend(lines[viewport.start..viewport.end].iter().cloned());
    if viewport.hidden_below > 0 {
        visible.push(hidden_lines_notice(
            cli,
            "↓",
            viewport.hidden_below,
            focused,
        ));
    }
    visible
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in super::super) struct LineViewport {
    pub(in super::super) start: usize,
    pub(in super::super) end: usize,
    pub(in super::super) hidden_above: usize,
    pub(in super::super) hidden_below: usize,
}

impl LineViewport {
    pub(in super::super) fn hidden_count(self) -> usize {
        self.hidden_above.saturating_add(self.hidden_below)
    }
}

pub(in super::super) fn line_viewport(
    line_count: usize,
    height: usize,
    scroll: usize,
) -> LineViewport {
    if line_count == 0 {
        return LineViewport::default();
    }
    if height == 0 {
        return LineViewport {
            hidden_below: line_count,
            ..LineViewport::default()
        };
    }
    if line_count <= height {
        return LineViewport {
            end: line_count,
            ..LineViewport::default()
        };
    }
    if height == 1 {
        return LineViewport {
            hidden_below: line_count,
            ..LineViewport::default()
        };
    }

    let max_scroll = line_count.saturating_sub(height.saturating_sub(1).max(1));
    let start = scroll.min(max_scroll);
    let hidden_above = start;
    let mut content_capacity = height.saturating_sub(usize::from(hidden_above > 0));
    let mut end = (start + content_capacity).min(line_count);
    if end < line_count {
        content_capacity = content_capacity.saturating_sub(1);
        end = (start + content_capacity).min(line_count);
    }

    LineViewport {
        start,
        end,
        hidden_above,
        hidden_below: line_count.saturating_sub(end),
    }
}

pub(in super::super) fn hidden_lines_notice(
    cli: &Cli,
    direction: &'static str,
    hidden: usize,
    focused: bool,
) -> Line<'static> {
    let hint = if focused { "; PgUp/PgDn scroll" } else { "" };
    Line::from(Span::styled(
        format!("{direction} {hidden} lines hidden{hint}"),
        style(cli, Color::DarkGray),
    ))
}

pub(in super::super) fn oracle_tree_node_line(
    cli: &Cli,
    tree: &OracleIndexTree,
    index: usize,
    selected: bool,
    prefix: &'static str,
) -> Line<'static> {
    let Some(node) = tree.node(index) else {
        return Line::from(Span::styled(
            format!("  {prefix} unavailable oracle node"),
            cell_style(cli, Color::DarkGray, selected),
        ));
    };
    let marker = if selected { ">" } else { " " };
    let depth = tree.node_depth(index);
    let indent = "    ".repeat(depth.min(4));
    let weight = if node.kind == OracleNodeKind::TerminalPin {
        format!("source weight {}", format_percent(node.row_weight_pct))
    } else {
        format!("basket weight {}", format_percent(node.weight_pct))
    };
    let source_count_label = if node.kind == OracleNodeKind::TerminalPin {
        "1 evidence trail".to_string()
    } else if node.pin_count == 1 {
        "1 source".to_string()
    } else {
        format!("{} sources", node.pin_count)
    };
    Line::from(vec![
        Span::styled(format!("{marker} "), cell_style(cli, Color::Cyan, selected)),
        Span::styled(
            format!("{prefix} "),
            cell_style(cli, Color::DarkGray, selected),
        ),
        Span::styled(
            format!("{indent}{} ", node.label.as_str()),
            cell_style(cli, Color::White, selected).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{weight} | {source_count_label}"),
            cell_style(cli, Color::Gray, selected),
        ),
    ])
}

pub(in super::super) fn oracle_overview_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let Some(tree) = app.oracle_tree() else {
        return vec![Line::from(Span::styled(
            oracle_recipe_load_panel_message(app),
            status_style(cli, &app.status),
        ))];
    };

    let phase = app.oracle_phase();
    let Some(node) = app.selected_oracle_node() else {
        return vec![Line::from(Span::styled(
            oracle_recipe_load_panel_message(app),
            status_style(cli, &app.status),
        ))];
    };
    let selected_index = app.selected_oracle_node_index();
    let row = tree
        .row_bucket_for(selected_index)
        .and_then(|index| tree.node(index).map(|node| node.label.as_str()))
        .unwrap_or("-");
    let weight_label = oracle_weight_label(node);
    let symbol = selected_oracle_market_symbol(app);
    let month_label = selected_oracle_month_label(app);
    let settlement_label = selected_oracle_settlement_label(app);
    let live_observation = current_spread_oracle_live(app).and_then(|live| {
        live.observations
            .iter()
            .find(|observation| spread_oracle_observation_matches_node(observation, node))
    });
    let source_status = live_observation
        .filter(|_| {
            matches!(
                phase,
                OraclePhase::OpeningPrint | OraclePhase::GameMode | OraclePhase::MonthClose
            )
        })
        .map(|observation| observation.opening_status.display_label())
        .unwrap_or_else(|| oracle_source_state_label(phase));
    let challenge_status = live_observation
        .map(|observation| match observation.opening_status {
            OpeningClaimViewStatus::Pending => "opening challenge window open",
            OpeningClaimViewStatus::Challenged => "opening challenge unresolved",
            OpeningClaimViewStatus::Accepted => "opening challenge window closed",
            OpeningClaimViewStatus::RejectedRetryable => "replacement opening allowed",
            _ => oracle_challenge_state_label(phase),
        })
        .unwrap_or_else(|| oracle_challenge_state_label(phase));
    let last_accepted = live_observation
        .filter(|observation| observation.opening_status == OpeningClaimViewStatus::Accepted)
        .map(|observation| observation.baseline_state.as_str())
        .unwrap_or("not accepted");
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{symbol} oracle evidence"),
                style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | month ", style(cli, Color::DarkGray)),
            Span::styled(month_label, style(cli, Color::White)),
            Span::styled(" | settles ", style(cli, Color::DarkGray)),
            Span::styled(settlement_label, style(cli, Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Source ", style(cli, Color::Green)),
            Span::styled(
                node.label.to_string(),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            divider_span(cli),
            Span::styled(node.kind.label().to_string(), style(cli, Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Weight ", style(cli, Color::Green)),
            Span::styled(
                weight_label,
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | row ", style(cli, Color::DarkGray)),
            Span::styled(row.to_string(), style(cli, Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Source status ", style(cli, Color::Green)),
            Span::styled(
                source_status.to_string(),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | challenge ", style(cli, Color::DarkGray)),
            Span::styled(challenge_status.to_string(), style(cli, Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Last accepted value ", style(cli, Color::Green)),
            Span::styled(last_accepted.to_string(), style(cli, Color::White)),
            Span::styled(" | move ", style(cli, Color::DarkGray)),
            Span::styled(oracle_delta_label(phase), style(cli, Color::White)),
        ]),
    ];

    if let Some(live) = current_spread_oracle_live(app) {
        let emergency_count = live.active_emergency_count();
        let month = live
            .oracle_month
            .clone()
            .unwrap_or_else(|| selected_oracle_month_label(app));
        lines.push(Line::from(vec![
            Span::styled("Live observations ", style(cli, Color::Green)),
            Span::styled(
                format!("{} in {month}", live.observations.len()),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | emergency ", style(cli, Color::DarkGray)),
            Span::styled(
                if emergency_count > 0 {
                    format!("{emergency_count} active")
                } else {
                    "clear".to_string()
                },
                if emergency_count > 0 {
                    oracle_emergency_style(cli, app)
                } else {
                    style(cli, Color::White)
                },
            ),
        ]));
    } else if app.loading_oracle_live {
        lines.push(Line::from(Span::styled(
            format!("{} Loading live observations...", app.spinner()),
            style(cli, Color::Yellow),
        )));
    } else if app.oracle_live_issue.is_some() {
        lines.push(Line::from(Span::styled(
            "Live observations are temporarily unavailable.",
            style(cli, Color::Yellow),
        )));
    }

    lines
}

pub(in super::super) fn oracle_weight_label(node: &RamxOracleNode) -> String {
    if node.kind == OracleNodeKind::TerminalPin {
        format!(
            "source pin; source weight {}",
            format_percent(node.row_weight_pct)
        )
    } else {
        format!("basket weight {}", format_percent(node.weight_pct))
    }
}

pub(in super::super) fn is_allowed_v1_source_category(category: &str) -> bool {
    let normalized = category.trim().to_ascii_lowercase();
    [
        "retailer product page",
        "distributor catalog page",
        "manufacturer product or store page",
        "manufacturer page",
        "benchmark / assessment",
        "market price assessment",
        "public api",
        "public api endpoint",
    ]
    .iter()
    .any(|allowed| normalized.contains(allowed))
}

pub(in super::super) fn oracle_source_state_label(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable => "lifecycle unverified",
        OraclePhase::Upcoming => "not started",
        OraclePhase::SourceSubmission => "terminal-SKU coverage incomplete",
        OraclePhase::Scramble => "pre-listing source work",
        _ => oracle_source_state_for_phase(phase).display_label(),
    }
}

pub(in super::super) fn oracle_source_state_for_phase(phase: OraclePhase) -> OracleSourceState {
    match phase {
        OraclePhase::Unavailable
        | OraclePhase::Upcoming
        | OraclePhase::SourceSubmission
        | OraclePhase::Scramble => OracleSourceState::Placed,
        OraclePhase::Placement => OracleSourceState::Placed,
        OraclePhase::KillChallenge => OracleSourceState::Snapshotted,
        OraclePhase::ResolutionFreeze => OracleSourceState::Frozen,
        OraclePhase::OpeningPrint => OracleSourceState::OpeningPending,
        OraclePhase::GameMode => OracleSourceState::Active,
        OraclePhase::MonthClose => OracleSourceState::MonthClosed,
    }
}

pub(in super::super) fn oracle_challenge_state_label(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable => "unverified",
        OraclePhase::Upcoming => "not started",
        OraclePhase::SourceSubmission => "waiting for complete terminal-SKU coverage",
        OraclePhase::Scramble => "governed by on-chain pre-listing state",
        OraclePhase::Placement => "not open yet",
        OraclePhase::KillChallenge => "source challenge open",
        OraclePhase::ResolutionFreeze => "resolving",
        OraclePhase::OpeningPrint => "opening challenge allowed",
        OraclePhase::GameMode => "update challenge allowed",
        OraclePhase::MonthClose => "final unless disputed",
    }
}

pub(in super::super) fn oracle_delta_label(phase: OraclePhase) -> &'static str {
    match phase {
        OraclePhase::Unavailable
        | OraclePhase::Upcoming
        | OraclePhase::SourceSubmission
        | OraclePhase::Scramble
        | OraclePhase::Placement
        | OraclePhase::KillChallenge
        | OraclePhase::ResolutionFreeze => "unopened",
        OraclePhase::OpeningPrint => "0.00% after accepted opening print",
        OraclePhase::GameMode => "movement from opening value",
        OraclePhase::MonthClose => "final movement from opening value",
    }
}

pub(in super::super) fn oracle_output_preview_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let preview = oracle_accumulator_preview(&app.oracle_submissions, app.oracle_tree());
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Settlement preview",
            style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("Coverage ", style(cli, Color::DarkGray)),
            Span::styled(
                format!(
                    "{}/{} rows | {}% basket weight",
                    preview.covered_rows, preview.total_rows, preview.covered_weight_pct
                ),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    match (preview.final_index, preview.benchmark_delta_pct) {
        (Some(index), Some(delta)) => {
            lines.push(Line::from(vec![
                Span::styled("Current oracle index ", style(cli, Color::DarkGray)),
                Span::styled(
                    format!("index {index:.3} | delta {}", format_signed_pct(delta)),
                    style(cli, Color::Green).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        _ => {
            lines.push(Line::from(Span::styled(
                "Incomplete until every active source has an accepted opening claim and latest accepted state.",
                style(cli, Color::Yellow),
            )));
        }
    }

    lines.push(Line::from(Span::styled(
        "Method: source-local moves -> source weights -> fixed basket weights.",
        style(cli, Color::Gray),
    )));
    for row in preview.row_lines.iter().take(4) {
        lines.push(Line::from(Span::styled(
            row.clone(),
            style(cli, Color::Gray),
        )));
    }
    if !preview.missing_rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "Missing rows: {}",
                preview
                    .missing_rows
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            style(cli, Color::DarkGray),
        )));
    }
    lines
}

pub(in super::super) fn oracle_flow_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let active = app.oracle_phase();
    let mut lines = Vec::new();
    for (index, phase) in OraclePhase::ALL.iter().copied().enumerate() {
        lines.extend(oracle_flow_phase_lines(
            cli,
            phase,
            active,
            app.spinner_tick,
        ));
        if index + 1 < OraclePhase::ALL.len() {
            lines.push(Line::from(Span::styled(
                "   ↓".to_string(),
                style(cli, Color::DarkGray),
            )));
        }
    }
    lines.push(Line::from(""));
    let game_mode_on = active == OraclePhase::GameMode;
    lines.push(Line::from(vec![
        Span::styled("Game Mode: ", style(cli, Color::Gray)),
        Span::styled(
            if game_mode_on { "ON" } else { "OFF" }.to_string(),
            style(
                cli,
                if game_mode_on {
                    Color::LightBlue
                } else {
                    Color::Magenta
                },
            )
            .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines
}

pub(in super::super) fn oracle_flow_phase_lines(
    cli: &Cli,
    phase: OraclePhase,
    active: OraclePhase,
    tick: usize,
) -> Vec<Line<'static>> {
    let selected = phase == active;
    let marker = if selected {
        oracle_active_timeline_badge(tick)
    } else {
        "○         "
    };
    let color = if selected {
        phase.active_color()
    } else {
        phase.color()
    };
    vec![
        Line::from(vec![
            Span::styled(format!("{marker} "), cell_style(cli, color, selected)),
            Span::styled(
                phase.label(),
                cell_style(cli, Color::White, selected).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("   ", style(cli, Color::DarkGray)),
            Span::styled(phase.detail(), cell_style(cli, Color::Gray, selected)),
        ]),
    ]
}

pub(in super::super) fn oracle_active_timeline_badge(tick: usize) -> &'static str {
    match tick % 4 {
        0 => ". ACTIVE .",
        1 => ": ACTIVE :",
        2 => "* ACTIVE *",
        _ => ": ACTIVE :",
    }
}

pub(in super::super) fn oracle_action_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    if let Some(form) = &app.oracle_form {
        return oracle_form_lines(cli, app, form);
    }

    let Some(context) = app.selected_oracle_action_context() else {
        return vec![Line::from(Span::styled(
            oracle_recipe_load_panel_message(app),
            status_style(cli, &app.status),
        ))];
    };
    let mut locked_actions = Vec::new();
    let mut locked_selected = false;
    let mut locked_flashing = false;
    let mut lines = Vec::new();
    for (index, action) in app.visible_oracle_actions().iter().copied().enumerate() {
        let selected = app.focus == LabFocus::OracleActions && index == app.oracle_selected;
        if action.availability(context) == OracleActionAvailability::Locked {
            locked_actions.push(action);
            locked_selected |= selected;
            locked_flashing |= app.oracle_action_flash_visible(action);
            continue;
        }
        lines.push(oracle_task_line(
            cli,
            action,
            context,
            selected,
            app.oracle_action_flash_visible(action),
        ));
    }
    if !locked_actions.is_empty() {
        lines.insert(
            0,
            oracle_locked_task_line(cli, &locked_actions, locked_selected, locked_flashing),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "All actions save local drafts only; Petri never prepares, signs, or sends them.",
        style(cli, Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "Enter opens an active action. Read more explains evidence rules and settlement math.",
        style(cli, Color::DarkGray),
    )));
    if app.selected_oracle_action() == OracleAction::ReviewQueue {
        lines.extend(oracle_submission_review_lines(cli, app));
    } else if app.oracle_submission_issue.is_some() || !app.oracle_submissions.is_empty() {
        lines.extend(oracle_submission_lines(cli, app));
    }
    lines
}

pub(in super::super) fn oracle_action_lines_for_panel(
    cli: &Cli,
    app: &LabApp,
    area: Rect,
    scroll: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    if let Some(form) = &app.oracle_form {
        return oracle_form_lines_for_height(cli, app, form, panel_inner_height(area), focused);
    }
    scroll_lines_to_panel(oracle_action_lines(cli, app), area, cli, scroll, focused)
}

pub(in super::super) fn oracle_form_lines(
    cli: &Cli,
    app: &LabApp,
    form: &OracleFormDraft,
) -> Vec<Line<'static>> {
    let node_label = if matches!(
        form.mode,
        OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw
    ) {
        "AMBA"
    } else {
        app.oracle_tree()
            .and_then(|tree| tree.node(form.node_index))
            .map(|node| node.label.as_str())
            .unwrap_or("selected source")
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} — LOCAL DRAFT ", form.mode.title()),
                style(cli, oracle_form_mode_color(form.mode)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("for {node_label}"), style(cli, Color::White)),
        ]),
        Line::from(Span::styled(
            form.mode.user_hint().to_string(),
            style(cli, Color::Gray),
        )),
        Line::from(Span::styled(
            "Enter queues local draft | Up/Down fields | PgUp/PgDn jumps | Esc cancels",
            if app.guide.focused_control.as_deref() == Some("control:oracle:queue-draft") {
                style(cli, Color::LightYellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                style(cli, Color::DarkGray)
            },
        )),
    ];

    for (index, field) in form.fields.iter().enumerate() {
        let selected = index == form.field_selected;
        let flashing = app.oracle_form_field_flash_visible(index);
        if selected {
            lines.push(Line::from(""));
        }
        let marker = if selected { ">" } else { " " };
        let required = if field.required { " required" } else { "" };
        let autofill = if field.editable { "" } else { " autofilled" };
        let value = if field.value.is_empty() {
            "-".to_string()
        } else {
            field.value.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} "),
                oracle_form_field_style(cli, Color::Yellow, selected, flashing),
            ),
            Span::styled(
                format!("{}: ", field.label),
                oracle_form_field_style(cli, Color::White, selected, flashing)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                value,
                oracle_form_field_style(cli, Color::Gray, selected, flashing),
            ),
            Span::styled(
                format!("{required}{autofill}"),
                oracle_form_field_style(cli, Color::DarkGray, selected, flashing),
            ),
        ]));
        if selected {
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(Span::styled(
        "Opening evidence is one public Wayback URL whose UTC capture time and archived target exactly match the entered source time and Canonical locator; the spread program stores it and derives the commitment.",
        style(cli, Color::Gray),
    )));
    let queued_state = if matches!(
        form.mode,
        OracleFormMode::AmbaDeposit | OracleFormMode::AmbaWithdraw
    ) {
        "Queued draft state: AMBA voting custody".to_string()
    } else {
        format!(
            "Queued draft state: source {}{}",
            form.mode.source_state().display_label(),
            form.mode
                .update_state()
                .map(|state| format!(" | update state {}", state.display_label()))
                .unwrap_or_default()
        )
    };
    lines.push(Line::from(Span::styled(
        queued_state,
        style(cli, Color::Cyan),
    )));
    lines.extend(oracle_submission_lines(cli, app));
    lines
}

pub(in super::super) fn oracle_form_lines_for_height(
    cli: &Cli,
    app: &LabApp,
    form: &OracleFormDraft,
    inner_height: usize,
    focused: bool,
) -> Vec<Line<'static>> {
    let lines = oracle_form_lines(cli, app, form);
    if inner_height == 0 || lines.len() <= inner_height {
        return scroll_lines_to_height(lines, inner_height, cli, 0, focused);
    }

    let header_count = 3usize.min(lines.len());
    let field_count = form.fields.len();
    if inner_height <= header_count + 2 || field_count == 0 {
        return scroll_lines_to_height(lines, inner_height, cli, 0, focused);
    }

    let mut visible = lines[..header_count].to_vec();
    let selected = form.field_selected.min(field_count.saturating_sub(1));
    let Some((start, end, spaced)) = oracle_form_field_window(form, inner_height) else {
        return scroll_lines_to_height(lines, inner_height, cli, 0, focused);
    };
    for index in start..end {
        if visible.len() >= inner_height {
            break;
        }
        if spaced && index == selected {
            visible.push(Line::from(""));
        }
        visible.push(lines[header_count + oracle_form_field_line_offset(index, selected)].clone());
        if spaced && index == selected && visible.len() < inner_height {
            visible.push(Line::from(""));
        }
    }

    let footer_start = header_count + field_count + 2;
    if let Some(footer) = lines.get(footer_start) {
        if visible.len() < inner_height {
            visible.push(footer.clone());
        }
    }

    scroll_lines_to_height(visible, inner_height, cli, 0, focused)
}

pub(in super::super) fn oracle_form_mode_color(mode: OracleFormMode) -> Color {
    match mode {
        OracleFormMode::SourceProposal => Color::Yellow,
        OracleFormMode::DefinitionEdit => Color::Cyan,
        OracleFormMode::SourceSupport => Color::Blue,
        OracleFormMode::OpeningPrint => Color::Green,
        OracleFormMode::UpdateClaim => Color::Green,
        OracleFormMode::Challenge => Color::Red,
        OracleFormMode::RewardClaim => Color::Green,
        OracleFormMode::StakeSettlement => Color::LightGreen,
        OracleFormMode::AmbaDeposit => Color::Magenta,
        OracleFormMode::AmbaWithdraw => Color::Blue,
    }
}

pub(in super::super) fn oracle_submission_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    if let Some(issue) = app.oracle_submission_issue.as_deref() {
        lines.push(Line::from(Span::styled(
            format!("Oracle draft store issue: {issue}"),
            style(cli, Color::Yellow),
        )));
    }
    if app.oracle_submissions.is_empty() {
        lines.push(Line::from(Span::styled(
            "No oracle submissions queued from this TUI session.",
            style(cli, Color::DarkGray),
        )));
        return lines;
    }

    lines.push(Line::from(Span::styled(
        "Queued oracle submissions",
        style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
    )));
    for record in app.oracle_submissions.iter().rev().take(4) {
        let update_state = record
            .update_state
            .map(|state| format!(" | {}", state.display_label()))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", record.title),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{} / {} | {}{} | {}",
                    record.row_label,
                    record.node_label,
                    record.source_state.display_label(),
                    update_state,
                    record.phase
                ),
                style(cli, Color::Gray),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", record.summary),
            style(cli, Color::DarkGray),
        )));
        if record.stored_id.is_some() {
            lines.push(Line::from(Span::styled(
                "  Draft saved locally.".to_string(),
                style(cli, Color::DarkGray),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {}", record.backend_status),
                style(cli, Color::DarkGray),
            )));
        }
    }
    lines.extend(oracle_output_preview_lines(cli, app));
    lines
}

pub(in super::super) fn oracle_submission_review_lines(
    cli: &Cli,
    app: &LabApp,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Local Oracle draft review",
            style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "These are semantic drafts only. Petri cannot prepare, sign, or submit them yet.",
            style(cli, Color::Yellow),
        )),
    ];
    if let Some(issue) = app.oracle_submission_issue.as_deref() {
        lines.push(Line::from(Span::styled(
            format!(
                "Draft store issue: {}",
                crate::backend::terminal_safe_text(issue)
            ),
            style(cli, Color::Yellow),
        )));
    }
    if app.oracle_submissions.is_empty() {
        lines.push(Line::from(Span::styled(
            "No local Oracle drafts are queued.",
            style(cli, Color::DarkGray),
        )));
        return lines;
    }

    for (offset, record) in app.oracle_submissions.iter().rev().take(8).enumerate() {
        if offset > 0 {
            lines.push(Line::from(""));
        }
        let title = crate::backend::terminal_safe_text(&record.title);
        let node = crate::backend::terminal_safe_text(&record.node_label);
        let row = crate::backend::terminal_safe_text(&record.row_label);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", title),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{row} / {node}"), style(cli, Color::Gray)),
        ]));
        lines.push(Line::from(Span::styled(
            format!(
                "Phase: {} | State: {}{}",
                crate::backend::terminal_safe_text(&record.phase),
                record.source_state.display_label(),
                record
                    .update_state
                    .map(|state| format!(" / {}", state.display_label()))
                    .unwrap_or_default()
            ),
            style(cli, Color::Cyan),
        )));
        for (label, raw_value) in &record.fields {
            let safe_label = crate::backend::terminal_safe_text(label);
            let secret = safe_label.to_ascii_lowercase().contains("secret")
                || safe_label.to_ascii_lowercase() == "salt";
            let safe_value = if secret {
                "[secret omitted]".to_string()
            } else if raw_value.trim().is_empty() {
                "-".to_string()
            } else {
                crate::backend::terminal_safe_text(raw_value)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {safe_label}: "), style(cli, Color::Gray)),
                Span::styled(safe_value, style(cli, Color::White)),
            ]));
        }
        if let Some(id) = record.stored_id.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("  Local id: {}", crate::backend::terminal_safe_text(id)),
                style(cli, Color::DarkGray),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!(
                "  Status: {}",
                crate::backend::terminal_safe_text(&record.backend_status)
            ),
            style(cli, Color::DarkGray),
        )));
    }
    lines
}

pub(in super::super) fn oracle_task_line(
    cli: &Cli,
    action: OracleAction,
    context: OracleActionContext,
    selected: bool,
    flashing: bool,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let active = action.availability(context) == OracleActionAvailability::Active;
    let action_color = if active {
        action.color()
    } else {
        Color::DarkGray
    };
    let marker_style = oracle_task_style(cli, action_color, selected, flashing);
    let state_style = oracle_task_style(cli, action_color, selected, flashing);
    let label_style = oracle_task_style(
        cli,
        if active {
            Color::White
        } else {
            Color::DarkGray
        },
        selected,
        flashing,
    )
    .add_modifier(Modifier::BOLD);
    let detail_style = oracle_task_style(
        cli,
        if active { Color::Gray } else { Color::DarkGray },
        selected,
        flashing,
    );
    Line::from(vec![
        Span::styled(format!("{marker} "), marker_style),
        Span::styled(
            format!("[{}] ", action.state(context)),
            state_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{}: ", action.display_label()), label_style),
        Span::styled(action.contextual_detail(context), detail_style),
    ])
}

pub(in super::super) fn oracle_locked_task_line(
    cli: &Cli,
    actions: &[OracleAction],
    selected: bool,
    flashing: bool,
) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let labels = actions
        .iter()
        .map(|action| action.label())
        .collect::<Vec<_>>()
        .join(", ");
    let row_style = oracle_task_style(cli, Color::DarkGray, selected, flashing);
    Line::from(vec![
        Span::styled(format!("{marker} "), row_style),
        Span::styled("[locked] ", row_style.add_modifier(Modifier::BOLD)),
        Span::styled(labels, row_style),
    ])
}
