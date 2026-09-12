//! Oracle workbench frame, panels, geometry, and hit testing.

use super::super::*;

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct OracleViewLayout {
    pub(in super::super) tabs_area: Rect,
    pub(in super::super) content_area: Rect,
}

pub(in super::super) fn oracle_view_layout(area: Rect) -> OracleViewLayout {
    if area.height < 4 {
        return OracleViewLayout {
            tabs_area: Rect { height: 0, ..area },
            content_area: area,
        };
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    OracleViewLayout {
        tabs_area: rows[0],
        content_area: rows[1],
    }
}

pub(in super::super) fn draw_oracle_view_tabs(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    active: OracleView,
) {
    if area.height == 0 {
        return;
    }
    let earn_active = active == OracleView::Earn;
    let advanced_active = active == OracleView::Advanced;
    let tabs = Paragraph::new(Line::from(vec![
        Span::styled(
            " Earn ",
            if earn_active {
                terminal_button_key_style(cli, Color::Green)
            } else {
                style(cli, Color::Gray)
            },
        ),
        Span::raw("  "),
        Span::styled(
            " Advanced view ",
            if advanced_active {
                terminal_button_key_style(cli, Color::Magenta)
            } else {
                style(cli, Color::Gray)
            },
        ),
        Span::styled("   V switches views", style(cli, Color::DarkGray)),
    ]))
    .block(panel_block(cli, "oracle view", Color::Cyan, false));
    frame.render_widget(tabs, area);
}

pub(in super::super) fn oracle_view_tab_rects(area: Rect) -> Option<(Rect, Rect)> {
    let layout = oracle_view_layout(area);
    let inner = panel_inner_rect(layout.tabs_area)?;
    let earn = Rect {
        x: inner.x,
        y: inner.y,
        width: 6.min(inner.width),
        height: 1.min(inner.height),
    };
    let advanced_x = inner.x.saturating_add(8);
    let advanced = Rect {
        x: advanced_x,
        y: inner.y,
        width: 15.min(inner.right().saturating_sub(advanced_x)),
        height: 1.min(inner.height),
    };
    Some((earn, advanced))
}

pub(in super::super) fn oracle_view_tab_hit_at(
    area: Rect,
    column: u16,
    row: u16,
) -> Option<OracleView> {
    let (earn, advanced) = oracle_view_tab_rects(area)?;
    if rect_contains(earn, column, row) {
        Some(OracleView::Earn)
    } else if rect_contains(advanced, column, row) {
        Some(OracleView::Advanced)
    } else {
        None
    }
}

pub(in super::super) fn draw_oracle_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let view_layout = oracle_view_layout(area);
    draw_oracle_view_tabs(frame, cli, view_layout.tabs_area, OracleView::Advanced);

    let Some(layout) = oracle_panel_rects(cli, area, app) else {
        return;
    };

    if let Some(selected) = layout.selected {
        draw_oracle_selected_source_panel(frame, cli, selected, app);
    }
    if let Some(actions) = layout.actions {
        draw_oracle_actions_panel(frame, cli, actions, app);
    }
    draw_oracle_tree_panel(frame, cli, layout.tree, app);
    if let Some(flow) = layout.flow {
        draw_oracle_flow_panel(frame, cli, flow, app);
    }
    if let Some(path) = layout.path {
        draw_oracle_path_panel(frame, cli, path, app);
    }
}

pub(in super::super) fn draw_oracle_tree_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OracleTasks;
    let tree = Paragraph::new(scroll_wrapped_lines_to_panel(
        oracle_tree_lines(cli, app),
        area,
        cli,
        app.focused_panel_scroll(LabFocus::OracleTasks),
        focused,
        false,
    ))
    .block(panel_block(cli, "oracle tree", Color::Magenta, focused))
    .wrap(Wrap { trim: false });
    frame.render_widget(tree, area);
}

pub(in super::super) fn draw_oracle_flow_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OracleOverview;
    let flow = scrolling_panel(
        oracle_flow_lines(cli, app),
        area,
        cli,
        app.focused_panel_scroll(LabFocus::OracleOverview),
        focused,
    )
    .block(panel_block(cli, "phase timeline", Color::Cyan, focused));
    frame.render_widget(flow, area);
}

pub(in super::super) fn draw_oracle_selected_source_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = false;
    let selected = scrolling_panel(oracle_overview_lines(cli, app), area, cli, 0, focused)
        .block(panel_block(cli, "selected source", Color::Yellow, focused));
    frame.render_widget(selected, area);
}

pub(in super::super) fn draw_oracle_actions_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OracleActions;
    let actions = Paragraph::new(oracle_action_lines_for_panel(
        cli,
        app,
        area,
        app.focused_panel_scroll(LabFocus::OracleActions),
        focused,
    ))
    .block(panel_block(cli, "context actions", Color::Yellow, focused))
    .wrap(Wrap { trim: true });
    frame.render_widget(actions, area);
}

pub(in super::super) fn draw_oracle_path_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::OraclePath;
    let path = Paragraph::new(scroll_wrapped_lines_to_panel(
        oracle_current_path_lines(cli, app),
        area,
        cli,
        app.focused_panel_scroll(LabFocus::OraclePath),
        focused,
        false,
    ))
    .block(panel_block(cli, "current path", Color::Yellow, focused))
    .wrap(Wrap { trim: false });
    frame.render_widget(path, area);
}

pub(in super::super) fn oracle_selected_source_panel_height(height: u16) -> u16 {
    if height >= 22 {
        7
    } else if height >= 16 {
        6
    } else {
        4
    }
}

pub(in super::super) fn oracle_context_actions_panel_height(
    height: u16,
    reserved_height: u16,
    line_count: usize,
) -> u16 {
    let base_height = if height >= 30 {
        7
    } else if height >= 22 {
        6
    } else {
        4
    };
    let desired_height = line_count.saturating_add(2).clamp(4, u16::MAX as usize) as u16;
    let cap = if height >= 42 {
        16
    } else if height >= 34 {
        14
    } else if height >= 30 {
        12
    } else if height >= 22 {
        8
    } else {
        4
    };
    let max_height = height.saturating_sub(reserved_height).max(4);
    desired_height.max(base_height).min(cap).min(max_height)
}

pub(in super::super) fn oracle_flow_stack_height(height: u16) -> u16 {
    if height >= 34 {
        20
    } else if height >= 28 {
        18
    } else if height >= 22 {
        15
    } else if height >= 18 {
        12
    } else if height >= 14 {
        9
    } else {
        0
    }
}
