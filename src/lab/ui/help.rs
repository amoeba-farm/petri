//! GitBook navigation, article, hover preview, glossary, and layout.

use super::super::*;

pub(in super::super) fn draw_home_help_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    if app.home_help_topic == HomeHelpTopic::Agents {
        draw_agent_connection_screen(frame, cli, area, app);
        return;
    }
    draw_gitbook_help_screen(frame, cli, area, app);
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct GitbookHelpLayout {
    pub(in super::super) navigation_area: Rect,
    pub(in super::super) article_area: Rect,
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct GitbookHelpPreviewGeometry {
    pub(in super::super) anchor: Rect,
    pub(in super::super) popup: Rect,
    pub(in super::super) shadow: Option<Rect>,
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct GitbookGlossaryGeometry {
    pub(in super::super) popup: Rect,
    pub(in super::super) shadow: Option<Rect>,
}

pub(in super::super) fn gitbook_help_layout(area: Rect) -> GitbookHelpLayout {
    if area.width >= 72 && area.height >= 10 {
        let navigation_width = (area.width / 2).clamp(40, 52);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(navigation_width), Constraint::Min(32)])
            .split(area);
        return GitbookHelpLayout {
            navigation_area: columns[0],
            article_area: columns[1],
        };
    }

    let navigation_height = (area.height / 3)
        .clamp(5, 9)
        .min(area.height.saturating_sub(4));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(navigation_height), Constraint::Min(4)])
        .split(area);
    GitbookHelpLayout {
        navigation_area: rows[0],
        article_area: rows[1],
    }
}

pub(in super::super) fn gitbook_nav_scroll_start(area: Rect, app: &LabApp) -> usize {
    let visible_height = panel_inner_rect(area)
        .map(|inner| usize::from(inner.height))
        .unwrap_or_default();
    let row_count = gitbook::nav_rows(&app.help.index, &app.help.expanded_categories).len();
    app.help
        .nav_scroll
        .min(row_count.saturating_sub(visible_height))
}

pub(in super::super) fn gitbook_nav_row_rect(
    area: Rect,
    app: &LabApp,
    nav_index: usize,
) -> Option<Rect> {
    let inner = panel_inner_rect(area)?;
    let visible_row = nav_index.checked_sub(gitbook_nav_scroll_start(area, app))?;
    if visible_row >= usize::from(inner.height) {
        return None;
    }
    Some(Rect {
        x: inner.x,
        y: inner.y.saturating_add(visible_row as u16),
        width: inner.width,
        height: 1,
    })
}

pub(in super::super) fn gitbook_help_preview_rect(
    bounds: Rect,
    navigation: Rect,
    anchor: Rect,
) -> Option<Rect> {
    if bounds.width < HELP_PREVIEW_MIN_WIDTH || bounds.height < HELP_PREVIEW_MIN_HEIGHT {
        return None;
    }
    let bounds_right = bounds.x.saturating_add(bounds.width);
    let bounds_bottom = bounds.y.saturating_add(bounds.height);
    let desired_height = ((bounds.height.saturating_mul(4)) / 5)
        .clamp(HELP_PREVIEW_MIN_HEIGHT, HELP_PREVIEW_MAX_HEIGHT)
        .min(bounds.height);
    let side_by_side = navigation.width < bounds.width && navigation.height == bounds.height;

    if side_by_side {
        let desired_width = ((bounds.width.saturating_mul(3)) / 5)
            .clamp(HELP_PREVIEW_MIN_WIDTH.max(40), HELP_PREVIEW_MAX_WIDTH);
        let right_x = navigation
            .x
            .saturating_add(navigation.width)
            .saturating_sub(1);
        let right_width = bounds_right.saturating_sub(right_x).min(desired_width);
        if right_width >= HELP_PREVIEW_MIN_WIDTH {
            let max_y = bounds_bottom.saturating_sub(desired_height);
            let preferred_y = anchor.y.saturating_sub(2);
            return Some(Rect {
                x: right_x,
                y: preferred_y.clamp(bounds.y, max_y),
                width: right_width,
                height: desired_height,
            });
        }

        let left_width = anchor.x.saturating_sub(bounds.x).min(desired_width);
        if left_width >= HELP_PREVIEW_MIN_WIDTH {
            let max_y = bounds_bottom.saturating_sub(desired_height);
            let preferred_y = anchor.y.saturating_sub(2);
            return Some(Rect {
                x: anchor.x.saturating_sub(left_width),
                y: preferred_y.clamp(bounds.y, max_y),
                width: left_width,
                height: desired_height,
            });
        }
    }

    let x = bounds.x;
    let width = bounds.width.saturating_sub(1);
    let anchor_bottom = anchor.y.saturating_add(anchor.height);
    let available_below = bounds_bottom.saturating_sub(anchor_bottom);
    let available_above = anchor.y.saturating_sub(bounds.y);
    let (y, height) = if available_below >= HELP_PREVIEW_MIN_HEIGHT {
        (anchor_bottom, desired_height.min(available_below))
    } else if available_above >= HELP_PREVIEW_MIN_HEIGHT {
        let height = desired_height.min(available_above);
        (anchor.y.saturating_sub(height), height)
    } else {
        (bounds.y, desired_height)
    };
    (width >= HELP_PREVIEW_MIN_WIDTH && height >= HELP_PREVIEW_MIN_HEIGHT).then_some(Rect {
        x,
        y,
        width,
        height,
    })
}

pub(in super::super) fn clipped_preview_shadow(bounds: Rect, popup: Rect) -> Option<Rect> {
    let bounds_right = bounds.x.saturating_add(bounds.width);
    let bounds_bottom = bounds.y.saturating_add(bounds.height);
    let x = popup.x.saturating_add(1);
    let y = popup.y.saturating_add(1);
    let width = popup.width.min(bounds_right.saturating_sub(x));
    let height = popup.height.min(bounds_bottom.saturating_sub(y));
    (width > 0 && height > 0).then_some(Rect {
        x,
        y,
        width,
        height,
    })
}

pub(in super::super) fn gitbook_help_preview_geometry(
    area: Rect,
    app: &LabApp,
) -> Option<GitbookHelpPreviewGeometry> {
    let preview = app.help.preview.as_ref()?;
    if preview.origin == HelpPreviewOrigin::Hover
        && !gitbook_reduced_motion()
        && app.spinner_tick < preview.reveal_tick
    {
        return None;
    }
    let layout = gitbook_help_layout(area);
    let anchor = gitbook_nav_row_rect(layout.navigation_area, app, preview.nav_index)?;
    let popup = gitbook_help_preview_rect(area, layout.navigation_area, anchor)?;
    Some(GitbookHelpPreviewGeometry {
        anchor,
        popup,
        shadow: clipped_preview_shadow(area, popup),
    })
}

pub(in super::super) fn gitbook_help_preview_geometry_for_root(
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) -> Option<GitbookHelpPreviewGeometry> {
    if app.screen != LabScreen::Help || app.home_help_topic != HomeHelpTopic::Overview {
        return None;
    }
    let frame_layout = lab_frame_layout(root, cli, app);
    let body_layout = lab_body_layout(app.screen, frame_layout.body_area);
    gitbook_help_preview_geometry(body_layout.selected_area, app)
}

pub(in super::super) fn gitbook_help_preview_max_scroll(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<usize> {
    let geometry = gitbook_help_preview_geometry(area, app)?;
    let preview = app.help.preview.as_ref()?;
    let inner = panel_inner_rect(geometry.popup)?;
    let content_height = usize::from(inner.height.saturating_sub(2));
    let text_width = usize::from(inner.width.saturating_sub(3));
    let lines = gitbook_help_preview_lines(cli, app, preview, text_width);
    Some(lines.len().saturating_sub(content_height))
}

pub(in super::super) fn gitbook_help_preview_max_scroll_for_root(
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) -> Option<usize> {
    if app.screen != LabScreen::Help || app.home_help_topic != HomeHelpTopic::Overview {
        return None;
    }
    let frame_layout = lab_frame_layout(root, cli, app);
    let body_layout = lab_body_layout(app.screen, frame_layout.body_area);
    gitbook_help_preview_max_scroll(cli, body_layout.selected_area, app)
}

pub(in super::super) fn gitbook_help_preview_close_rect(popup: Rect) -> Option<Rect> {
    if popup.width < 12 || popup.height < 3 {
        return None;
    }
    Some(Rect {
        x: popup.x.saturating_add(popup.width).saturating_sub(6),
        y: popup.y.saturating_add(1),
        width: 5,
        height: 1,
    })
}

pub(in super::super) fn gitbook_glossary_geometry(
    area: Rect,
    app: &LabApp,
) -> Option<GitbookGlossaryGeometry> {
    let hover = app.help.glossary_hover.as_ref()?;
    if area.width < GLOSSARY_PREVIEW_MIN_WIDTH || area.height < 5 {
        return None;
    }
    let width = area
        .width
        .saturating_sub(2)
        .clamp(GLOSSARY_PREVIEW_MIN_WIDTH, GLOSSARY_PREVIEW_MAX_WIDTH);
    let text_width = usize::from(width.saturating_sub(4)).max(1);
    let definition_lines = wrap_gitbook_text(&hover.definition, text_width, "", "").len();
    let height = (definition_lines as u16)
        .saturating_add(3)
        .max(5)
        .min(area.height);
    let area_right = area.x.saturating_add(area.width);
    let area_bottom = area.y.saturating_add(area.height);
    let max_x = area_right.saturating_sub(width);
    let preferred_x = hover
        .anchor
        .x
        .saturating_add(hover.anchor.width / 2)
        .saturating_sub(width / 3);
    let x = preferred_x.clamp(area.x, max_x);
    let below = hover.anchor.y.saturating_add(hover.anchor.height);
    let y = if area_bottom.saturating_sub(below) >= height {
        below
    } else if hover.anchor.y.saturating_sub(area.y) >= height {
        hover.anchor.y.saturating_sub(height)
    } else {
        area_bottom.saturating_sub(height)
    };
    let popup = Rect::new(x, y, width, height);
    Some(GitbookGlossaryGeometry {
        popup,
        shadow: clipped_preview_shadow(area, popup),
    })
}

pub(in super::super) fn draw_gitbook_help_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let layout = gitbook_help_layout(area);
    let navigation_focused = app.help.pane == HelpPane::Navigation;
    let navigation_scroll = gitbook_nav_scroll_start(layout.navigation_area, app);
    let navigation = Paragraph::new(clip_lines_to_panel(
        gitbook_navigation_lines(cli, app),
        layout.navigation_area,
        cli,
        navigation_scroll,
    ))
    .block(panel_block(
        cli,
        "GitBook topics",
        Color::Cyan,
        navigation_focused,
    ));
    frame.render_widget(navigation, layout.navigation_area);

    let article_focused = app.help.pane == HelpPane::Article;
    let article_title = app
        .current_help_page()
        .map(|page| format!("guide: {}", short_path(&page.title, 42)))
        .unwrap_or_else(|| "guide".to_string());
    let article_block = panel_block(cli, &article_title, Color::Yellow, article_focused);
    let article_inner = article_block.inner(layout.article_area);
    let article = Paragraph::new(scroll_lines_to_panel(
        gitbook_article_lines(cli, app, article_inner.width as usize),
        layout.article_area,
        cli,
        app.help.article_scroll,
        article_focused,
    ))
    .block(article_block);
    frame.render_widget(article, layout.article_area);
    draw_gitbook_help_preview(frame, cli, area, app);
    draw_gitbook_glossary_hover(frame, cli, area, app);
}

pub(in super::super) fn draw_gitbook_help_preview(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let Some(geometry) = gitbook_help_preview_geometry(area, app) else {
        return;
    };
    let Some(preview) = app.help.preview.as_ref() else {
        return;
    };
    let rows = gitbook::nav_rows(&app.help.index, &app.help.expanded_categories);
    let Some(row) = rows.get(preview.nav_index) else {
        return;
    };

    if let Some(shadow) = geometry.shadow {
        frame.render_widget(Clear, shadow);
        fill_tui_area(frame, cli, shadow, Color::Rgb(3, 10, 24));
    }
    frame.render_widget(Clear, geometry.popup);
    let accent = match row.target {
        GitbookNavTarget::Category(_) => Color::Yellow,
        GitbookNavTarget::Page { .. } => Color::Cyan,
    };
    let title_width = usize::from(geometry.popup.width.saturating_sub(24)).max(8);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(style(cli, accent).add_modifier(Modifier::BOLD))
        .style(tui_alt_panel_style(cli));
    let inner = block.inner(geometry.popup);
    frame.render_widget(block, geometry.popup);

    if inner.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("topic preview: {}", short_path(&row.label, title_width)),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            )))
            .style(tui_alt_panel_style(cli)),
            Rect::new(inner.x, inner.y, inner.width.saturating_sub(6), 1),
        );
    }

    let anchor_right = geometry.anchor.x.saturating_add(geometry.anchor.width);
    let popup_bottom = geometry.popup.y.saturating_add(geometry.popup.height);
    if geometry.popup.x >= anchor_right
        && geometry.anchor.y >= geometry.popup.y
        && geometry.anchor.y < popup_bottom
    {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "▶",
                style(cli, accent).add_modifier(Modifier::BOLD),
            )))
            .style(tui_alt_panel_style(cli)),
            Rect {
                x: geometry.popup.x,
                y: geometry.anchor.y,
                width: 1,
                height: 1,
            },
        );
    }

    if let Some(close) = gitbook_help_preview_close_rect(geometry.popup) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [x] ",
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            )))
            .style(tui_alt_panel_style(cli)),
            close,
        );
    }
    if inner.width < 4 || inner.height < 3 {
        return;
    }

    let footer_height: u16 = 1;
    let content_area = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y.saturating_add(1),
        width: inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(footer_height.saturating_add(1)),
    };
    let text_area = Rect {
        width: content_area.width.saturating_sub(1),
        ..content_area
    };
    let lines = gitbook_help_preview_lines(cli, app, preview, text_area.width as usize);
    let max_scroll = lines.len().saturating_sub(usize::from(content_area.height));
    let scroll = preview.scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines.clone())
            .style(tui_alt_panel_style(cli))
            .wrap(Wrap { trim: true })
            .scroll((scroll.min(usize::from(u16::MAX)) as u16, 0)),
        text_area,
    );
    if max_scroll > 0 && content_area.height > 1 {
        let mut scrollbar_state = ScrollbarState::new(lines.len())
            .position(scroll)
            .viewport_content_length(usize::from(content_area.height));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .track_style(style(cli, Color::DarkGray))
            .thumb_style(style(cli, accent));
        frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);
    }

    let footer = gitbook_help_preview_footer(
        cli,
        preview.origin,
        matches!(row.target, GitbookNavTarget::Category(_)),
        inner.width as usize,
    );
    frame.render_widget(
        Paragraph::new(footer)
            .alignment(ratatui::layout::Alignment::Center)
            .style(tui_alt_panel_style(cli)),
        Rect {
            x: inner.x,
            y: inner.y.saturating_add(inner.height).saturating_sub(1),
            width: inner.width,
            height: footer_height,
        },
    );
}

pub(in super::super) fn draw_gitbook_glossary_hover(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let Some(hover) = app.help.glossary_hover.as_ref() else {
        return;
    };
    let Some(geometry) = gitbook_glossary_geometry(area, app) else {
        return;
    };
    if let Some(shadow) = geometry.shadow {
        frame.render_widget(Clear, shadow);
        fill_tui_area(frame, cli, shadow, Color::Rgb(3, 10, 24));
    }
    frame.render_widget(Clear, geometry.popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(style(cli, Color::Magenta).add_modifier(Modifier::BOLD))
        .style(tui_alt_panel_style(cli));
    let inner = block.inner(geometry.popup);
    frame.render_widget(block, geometry.popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "glossary: {}",
                short_path(
                    &hover.term,
                    usize::from(inner.width.saturating_sub(10)).max(4),
                )
            ),
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        )))
        .style(tui_alt_panel_style(cli)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if inner.height > 1 {
        frame.render_widget(
            Paragraph::new(hover.definition.clone())
                .style(tui_alt_panel_style(cli))
                .wrap(Wrap { trim: true }),
            Rect::new(
                inner.x,
                inner.y.saturating_add(1),
                inner.width,
                inner.height.saturating_sub(1),
            ),
        );
    }
}

pub(in super::super) fn gitbook_help_preview_footer(
    cli: &Cli,
    origin: HelpPreviewOrigin,
    category: bool,
    width: usize,
) -> Line<'static> {
    let action = if category { "expand" } else { "open" };
    let candidates = if origin == HelpPreviewOrigin::Keyboard {
        vec![
            format!("wheel/Pg scroll | Enter/click {action} | Space/Esc close"),
            format!("wheel | Enter {action} | Space close | Esc"),
            format!("Enter {action} | Space/Esc"),
            format!("Enter {action} | Esc"),
            "Esc".to_string(),
        ]
    } else {
        vec![
            format!("wheel/Pg scroll | Enter/click {action} | Space pin | Esc"),
            format!("wheel | click {action} | Space pin | Esc"),
            format!("click {action} | Space pin | Esc"),
            "Space pin | Esc".to_string(),
            "Esc".to_string(),
        ]
    };
    let text = candidates
        .into_iter()
        .find(|candidate| text_width(candidate) <= width)
        .unwrap_or_default();
    Line::from(Span::styled(text, style(cli, Color::DarkGray)))
}

pub(in super::super) fn gitbook_help_preview_lines(
    cli: &Cli,
    app: &LabApp,
    preview: &GitbookHelpPreview,
    width: usize,
) -> Vec<Line<'static>> {
    let width = width.max(8);
    let rows = gitbook::nav_rows(&app.help.index, &app.help.expanded_categories);
    let Some(row) = rows.get(preview.nav_index) else {
        return vec![Line::from(Span::styled(
            "This topic is no longer in the guide index.",
            style(cli, Color::Yellow),
        ))];
    };
    match row.target {
        GitbookNavTarget::Category(category_index) => {
            let Some(category) = app.help.index.categories.get(category_index) else {
                return Vec::new();
            };
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        "SECTION",
                        style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {} topics", category.pages.len()),
                        style(cli, Color::Gray),
                    ),
                ]),
                Line::from(""),
            ];
            for page in &category.pages {
                lines.extend(
                    wrap_gitbook_text(&page.title, width, "• ", "  ")
                        .into_iter()
                        .map(|line| {
                            Line::from(Span::styled(
                                line,
                                style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
                            ))
                        }),
                );
                if !page.description.trim().is_empty() {
                    lines.extend(
                        wrap_gitbook_text(&page.description, width, "  ", "  ")
                            .into_iter()
                            .map(|line| Line::from(Span::styled(line, style(cli, Color::Gray)))),
                    );
                }
                lines.push(Line::from(""));
            }
            lines
        }
        GitbookNavTarget::Page { category, page } => {
            let Some(category) = app.help.index.categories.get(category) else {
                return Vec::new();
            };
            let Some(link) = category.pages.get(page) else {
                return Vec::new();
            };
            let source = preview
                .page
                .as_ref()
                .map(|page| page.source.label())
                .unwrap_or("GitBook summary");
            let mut lines = vec![
                Line::from(vec![
                    Span::styled(
                        category.title.clone(),
                        style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  •  ", style(cli, Color::DarkGray)),
                    Span::styled(source.to_string(), style(cli, Color::Green)),
                ]),
                Line::from(""),
            ];
            if let Some(page) = preview.page.as_ref() {
                append_gitbook_preview_blocks(cli, &mut lines, &page.blocks, width);
            } else {
                if !link.description.trim().is_empty() {
                    lines.extend(
                        wrap_gitbook_text(&link.description, width, "", "")
                            .into_iter()
                            .map(|line| Line::from(Span::styled(line, style(cli, Color::White)))),
                    );
                    lines.push(Line::from(""));
                }
                let (message, color) = if app.help.loading_preview_page
                    && app.help.preview_page_request_id.as_deref() == Some(link.id.as_str())
                {
                    (
                        format!("{} Loading the published topic preview...", app.spinner()),
                        Color::Cyan,
                    )
                } else if app.help.preview_failed_page_id.as_deref() == Some(link.id.as_str()) {
                    (
                        "The published topic preview is temporarily unavailable.".to_string(),
                        Color::Yellow,
                    )
                } else {
                    (
                        "Preparing the published topic preview...".to_string(),
                        Color::DarkGray,
                    )
                };
                lines.extend(
                    wrap_gitbook_text(&message, width, "", "")
                        .into_iter()
                        .map(|line| Line::from(Span::styled(line, style(cli, color)))),
                );
            }
            lines
        }
    }
}

pub(in super::super) fn append_gitbook_preview_blocks(
    cli: &Cli,
    lines: &mut Vec<Line<'static>>,
    blocks: &[MarkdownBlock],
    width: usize,
) {
    for block in blocks {
        match block {
            MarkdownBlock::Heading { level, text } => lines.extend(
                wrap_gitbook_text(text, width, "", "")
                    .into_iter()
                    .map(|line| {
                        Line::from(Span::styled(
                            line,
                            style(
                                cli,
                                if *level <= 2 {
                                    Color::Yellow
                                } else {
                                    Color::Cyan
                                },
                            )
                            .add_modifier(Modifier::BOLD),
                        ))
                    }),
            ),
            MarkdownBlock::Paragraph(text) => lines.extend(
                wrap_gitbook_text(text, width, "", "")
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, style(cli, Color::White)))),
            ),
            MarkdownBlock::Bullet { depth, text } => {
                let first = format!("{}• ", "  ".repeat(*depth));
                let rest = " ".repeat(first.chars().count());
                lines.extend(
                    wrap_gitbook_text(text, width, &first, &rest)
                        .into_iter()
                        .map(|line| Line::from(Span::styled(line, style(cli, Color::Gray)))),
                );
            }
            MarkdownBlock::Numbered {
                depth,
                number,
                text,
            } => {
                let first = format!("{}{}. ", "  ".repeat(*depth), number);
                let rest = " ".repeat(first.chars().count());
                lines.extend(
                    wrap_gitbook_text(text, width, &first, &rest)
                        .into_iter()
                        .map(|line| Line::from(Span::styled(line, style(cli, Color::Gray)))),
                );
            }
            MarkdownBlock::Quote(text) => lines.extend(
                wrap_gitbook_text(text, width, "│ ", "│ ")
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, style(cli, Color::Magenta)))),
            ),
            MarkdownBlock::Code(text) => lines.extend(
                wrap_gitbook_text(text, width, "  ", "  ")
                    .into_iter()
                    .map(|line| {
                        Line::from(Span::styled(
                            line,
                            style(cli, Color::Green).bg(TUI_FIELD_BACKGROUND),
                        ))
                    }),
            ),
            MarkdownBlock::Rule => lines.push(Line::from(Span::styled(
                "─".repeat(width.min(36)),
                style(cli, Color::DarkGray),
            ))),
            MarkdownBlock::Blank => lines.push(Line::from("")),
        }
    }
}

pub(in super::super) fn gitbook_navigation_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let rows = gitbook::nav_rows(&app.help.index, &app.help.expanded_categories);
    if rows.is_empty() {
        return vec![Line::from(Span::styled(
            "No guide topics are available.",
            style(cli, Color::Gray),
        ))];
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let selected = index == app.help.selected_nav;
            let active = selected && app.help.pane == HelpPane::Navigation;
            let marker = if selected { ">" } else { " " };
            let (prefix, color, bold) = match row.target {
                GitbookNavTarget::Category(_) => (
                    if row.expanded { "[-]" } else { "[+]" },
                    Color::Yellow,
                    true,
                ),
                GitbookNavTarget::Page { .. } => ("  -", Color::Cyan, false),
            };
            let mut row_style = cell_style(cli, color, active);
            if bold || selected {
                row_style = row_style.add_modifier(Modifier::BOLD);
            }
            Line::from(Span::styled(
                format!("{marker} {prefix} {}", row.label),
                row_style,
            ))
        })
        .collect()
}

pub(in super::super) fn gitbook_nav_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !rect_contains(area, column, row) {
        return None;
    }
    let inner = panel_block(cli, "GitBook topics", Color::Cyan, true).inner(area);
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible_row = usize::from(row.saturating_sub(inner.y));
    let index = gitbook_nav_scroll_start(area, app).saturating_add(visible_row);
    (index < gitbook::nav_rows(&app.help.index, &app.help.expanded_categories).len())
        .then_some(index)
}

pub(in super::super) fn gitbook_glossary_matches<'a>(
    text: &str,
    entries: &'a [GitbookGlossaryEntry],
) -> Vec<GitbookGlossaryMatch<'a>> {
    let lowercase = text.to_ascii_lowercase();
    let mut ordered = entries.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|entry| std::cmp::Reverse(entry.term.len()));
    let mut matches = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(character) = text[cursor..].chars().next() else {
            break;
        };
        let matched = ordered.iter().copied().find_map(|entry| {
            let term = entry.term.to_ascii_lowercase();
            let end = cursor.saturating_add(term.len());
            if end > lowercase.len()
                || !lowercase[cursor..].starts_with(&term)
                || !text.is_char_boundary(end)
            {
                return None;
            }
            let starts_at_boundary = cursor == 0
                || text[..cursor].chars().next_back().is_none_or(|before| {
                    !before.is_alphanumeric() && before != '_' && before != '-'
                });
            let ends_at_boundary = end == text.len()
                || text[end..]
                    .chars()
                    .next()
                    .is_none_or(|after| !after.is_alphanumeric() && after != '_' && after != '-');
            (starts_at_boundary && ends_at_boundary).then_some((entry, end))
        });
        if let Some((entry, end)) = matched {
            matches.push(GitbookGlossaryMatch {
                start: cursor,
                end,
                entry,
            });
            cursor = end;
        } else {
            cursor = cursor.saturating_add(character.len_utf8());
        }
    }
    matches
}

pub(in super::super) fn gitbook_glossary_style(cli: &Cli) -> Style {
    style(cli, Color::Magenta).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

pub(in super::super) fn gitbook_glossary_line(
    cli: &Cli,
    text: String,
    base_style: Style,
    entries: &[GitbookGlossaryEntry],
    linked_terms: &mut HashSet<String>,
) -> Line<'static> {
    let matches = gitbook_glossary_matches(&text, entries)
        .into_iter()
        .filter(|matched| !linked_terms.contains(&matched.entry.term.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Line::from(Span::styled(text, base_style));
    }
    let mut spans = Vec::new();
    let mut cursor = 0;
    for matched in matches {
        if cursor < matched.start {
            spans.push(Span::styled(
                text[cursor..matched.start].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            text[matched.start..matched.end].to_string(),
            gitbook_glossary_style(cli),
        ));
        linked_terms.insert(matched.entry.term.to_ascii_lowercase());
        cursor = matched.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }
    Line::from(spans)
}

pub(in super::super) fn is_gitbook_glossary_span(span: &Span<'_>) -> bool {
    span.style.add_modifier.contains(Modifier::BOLD)
        && span.style.add_modifier.contains(Modifier::UNDERLINED)
}

pub(in super::super) fn gitbook_glossary_hit_at(
    cli: &Cli,
    article_area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<GitbookGlossaryHit> {
    let article_title = app
        .current_help_page()
        .map(|page| format!("guide: {}", short_path(&page.title, 42)))
        .unwrap_or_else(|| "guide".to_string());
    let inner = panel_block(
        cli,
        &article_title,
        Color::Yellow,
        app.help.pane == HelpPane::Article,
    )
    .inner(article_area);
    if !rect_contains(inner, column, row) {
        return None;
    }
    let visible = scroll_lines_to_panel(
        gitbook_article_lines(cli, app, inner.width as usize),
        article_area,
        cli,
        app.help.article_scroll,
        app.help.pane == HelpPane::Article,
    );
    let line = visible.get(usize::from(row.saturating_sub(inner.y)))?;
    let mut x = inner.x;
    for span in &line.spans {
        let width = text_width(span.content.as_ref()).min(usize::from(u16::MAX)) as u16;
        let span_area = Rect::new(x, row, width, 1);
        if width > 0 && rect_contains(span_area, column, row) && is_gitbook_glossary_span(span) {
            let entry = gitbook::bundled_glossary_entries()
                .iter()
                .find(|entry| entry.term.eq_ignore_ascii_case(span.content.as_ref()))?
                .clone();
            return Some(GitbookGlossaryHit {
                entry,
                anchor: span_area,
            });
        }
        x = x.saturating_add(width);
    }
    None
}

pub(in super::super) fn gitbook_article_lines(
    cli: &Cli,
    app: &LabApp,
    width: usize,
) -> Vec<Line<'static>> {
    let width = width.max(8);
    let motion_tick = gitbook_motion_tick(app);
    let cacheable = !app.help.loading_index
        && !app.help.loading_page
        && app.help.issue.is_none()
        && motion_tick == 0;
    if cacheable
        && app.current_help_page().is_some()
        && let Some(cache) = app.cache.help_render().as_ref()
        && cache.page_id == app.help.selected_page_id
        && cache.page_revision == app.cache.help_revision()
        && cache.page_count == app.help.index.page_count()
        && cache.width == width
        && cache.no_color == cli.no_color
        && cache.motion_tick == motion_tick
    {
        return cache.lines.clone();
    }
    let mut lines = Vec::new();
    let source = app
        .current_help_page()
        .map(|page| page.source)
        .unwrap_or(app.help.index.source);
    lines.push(Line::from(vec![
        Span::styled("Source ", style(cli, Color::DarkGray)),
        Span::styled(
            source.label().to_string(),
            style(
                cli,
                if source == gitbook::GitbookSource::Live {
                    Color::Green
                } else {
                    Color::Yellow
                },
            )
            .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", style(cli, Color::DarkGray)),
        Span::styled(
            if app.help.loading_index {
                format!("{} checking for updates", app.spinner())
            } else {
                format!("{} pages", app.help.index.page_count())
            },
            style(cli, Color::Gray),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(
            "Published docs: {}",
            short_path(&docs_url(), width.saturating_sub(16))
        ),
        style(cli, Color::Cyan).add_modifier(Modifier::UNDERLINED),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "Terms: {}",
            short_path(
                &app.wallet_terms.terms_url,
                width.saturating_sub("Terms: ".len()),
            )
        ),
        style(cli, Color::Yellow).add_modifier(Modifier::UNDERLINED),
    )));
    if let Some(issue) = app.help.issue.as_deref() {
        lines.extend(
            wrap_gitbook_text(&format!("Live update unavailable: {issue}"), width, "", "")
                .into_iter()
                .map(|line| Line::from(Span::styled(line, style(cli, Color::Yellow)))),
        );
    }
    lines.push(Line::from(""));

    if app.help.loading_page {
        for art in gitbook::loading_frame(gitbook_animation_tick(app)) {
            lines.push(Line::from(Span::styled(
                art.to_string(),
                style(cli, Color::Magenta).add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(Span::styled(
            "Updating this page from GitBook...",
            style(cli, Color::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    let Some(page) = app.current_help_page() else {
        lines.push(Line::from(Span::styled(
            "Select a topic on the left and press Enter.",
            style(cli, Color::Gray),
        )));
        return lines;
    };
    lines.push(Line::from(Span::styled(
        page.title.clone(),
        style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    let glossary_entries: &[GitbookGlossaryEntry] =
        if page.url.to_ascii_lowercase().contains("glossary") {
            &[]
        } else {
            gitbook::bundled_glossary_entries()
        };
    let mut linked_glossary_terms = HashSet::new();

    for (block_index, block) in page.blocks.iter().enumerate() {
        match block {
            MarkdownBlock::Heading { level, text } => {
                if *level <= 2 {
                    lines.push(Line::from(Span::styled(
                        gitbook::section_divider_frame(motion_tick.wrapping_add(block_index))
                            .to_string(),
                        style(cli, Color::Magenta),
                    )));
                }
                lines.extend(
                    wrap_gitbook_text(text, width, "", "")
                        .into_iter()
                        .map(|line| {
                            Line::from(Span::styled(
                                line,
                                style(
                                    cli,
                                    if *level <= 2 {
                                        Color::Yellow
                                    } else {
                                        Color::Cyan
                                    },
                                )
                                .add_modifier(Modifier::BOLD),
                            ))
                        }),
                );
            }
            MarkdownBlock::Paragraph(text) => {
                lines.extend(
                    wrap_gitbook_text(text, width, "", "")
                        .into_iter()
                        .map(|line| {
                            gitbook_glossary_line(
                                cli,
                                line,
                                style(cli, Color::White),
                                glossary_entries,
                                &mut linked_glossary_terms,
                            )
                        }),
                );
            }
            MarkdownBlock::Bullet { depth, text } => {
                let first = format!("{}- ", "  ".repeat(*depth));
                let rest = " ".repeat(first.chars().count());
                lines.extend(
                    wrap_gitbook_text(text, width, &first, &rest)
                        .into_iter()
                        .map(|line| {
                            gitbook_glossary_line(
                                cli,
                                line,
                                style(cli, Color::Gray),
                                glossary_entries,
                                &mut linked_glossary_terms,
                            )
                        }),
                );
            }
            MarkdownBlock::Numbered {
                depth,
                number,
                text,
            } => {
                let first = format!("{}{}. ", "  ".repeat(*depth), number);
                let rest = " ".repeat(first.chars().count());
                lines.extend(
                    wrap_gitbook_text(text, width, &first, &rest)
                        .into_iter()
                        .map(|line| {
                            gitbook_glossary_line(
                                cli,
                                line,
                                style(cli, Color::Gray),
                                glossary_entries,
                                &mut linked_glossary_terms,
                            )
                        }),
                );
            }
            MarkdownBlock::Quote(text) => {
                lines.extend(
                    wrap_gitbook_text(text, width, "| ", "| ")
                        .into_iter()
                        .map(|line| {
                            gitbook_glossary_line(
                                cli,
                                line,
                                style(cli, Color::Magenta),
                                glossary_entries,
                                &mut linked_glossary_terms,
                            )
                        }),
                );
            }
            MarkdownBlock::Code(text) => {
                lines.extend(
                    wrap_gitbook_text(text, width, "  ", "  ")
                        .into_iter()
                        .map(|line| {
                            Line::from(Span::styled(
                                line,
                                style(cli, Color::Green).bg(TUI_FIELD_BACKGROUND),
                            ))
                        }),
                );
            }
            MarkdownBlock::Rule => lines.push(Line::from(Span::styled(
                "-".repeat(width.min(42)),
                style(cli, Color::DarkGray),
            ))),
            MarkdownBlock::Blank => lines.push(Line::from("")),
        }
    }
    if cacheable && app.current_help_page().is_some() {
        app.cache.store_help_render(GitbookRenderCache {
            page_id: app.help.selected_page_id.clone(),
            page_revision: app.cache.help_revision(),
            page_count: app.help.index.page_count(),
            width,
            no_color: cli.no_color,
            motion_tick,
            lines: lines.clone(),
        });
    }
    lines
}

pub(in super::super) fn gitbook_motion_tick(app: &LabApp) -> usize {
    if gitbook_reduced_motion() {
        return 0;
    }
    app.help
        .transition_tick
        .filter(|started| app.spinner_tick.saturating_sub(*started) <= 8)
        .map(|_| app.spinner_tick)
        .unwrap_or(0)
}

pub(in super::super) fn gitbook_animation_tick(app: &LabApp) -> usize {
    if gitbook_reduced_motion() {
        0
    } else {
        app.spinner_tick
    }
}

pub(in super::super) fn gitbook_reduced_motion() -> bool {
    env::var(PETRI_TUI_REDUCED_MOTION_ENV)
        .ok()
        .is_some_and(|value| !env_flag_is_false(&value))
}

pub(in super::super) fn wrap_gitbook_text(
    text: &str,
    width: usize,
    first_prefix: &str,
    continuation_prefix: &str,
) -> Vec<String> {
    let width = width.max(1);
    let mut output = Vec::new();
    let mut current = first_prefix.to_string();
    let mut prefix = first_prefix;
    for word in text.split_whitespace() {
        let separator = usize::from(current.chars().count() > prefix.chars().count());
        if current.chars().count() + separator + word.chars().count() <= width {
            if separator > 0 {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if current.chars().count() > prefix.chars().count() {
            output.push(current);
            prefix = continuation_prefix;
            current = prefix.to_string();
        }
        let available = width.saturating_sub(current.chars().count()).max(1);
        let chars = word.chars().collect::<Vec<_>>();
        for chunk in chars.chunks(available) {
            if current.chars().count() > prefix.chars().count() {
                output.push(current);
                prefix = continuation_prefix;
                current = prefix.to_string();
            }
            current.extend(chunk);
            if chunk.len() == available {
                output.push(current);
                prefix = continuation_prefix;
                current = prefix.to_string();
            }
        }
    }
    if current.chars().count() > prefix.chars().count() || output.is_empty() {
        output.push(current);
    }
    output
}
