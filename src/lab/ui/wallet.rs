//! Wallet Terms, switching controls, and their terminal geometry.

use super::super::*;

pub(in super::super) fn draw_terms_screen(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    let focused = app.focus == LabFocus::Terms;
    let panel = scrolling_panel(
        wallet_terms_lines(cli, app),
        area,
        cli,
        app.focused_panel_scroll(LabFocus::Terms),
        focused,
    )
    .block(panel_block(cli, "wallet terms", Color::Yellow, focused));
    frame.render_widget(panel, area);
}

pub(in super::super) fn wallet_terms_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let account = app
        .wallet
        .pubkey
        .as_deref()
        .map(short_pubkey)
        .unwrap_or_else(|| "not attached".to_string());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Review Terms to continue",
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" for wallet ", style(cli, Color::DarkGray)),
            Span::styled(
                account,
                style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Terms of Use: ", style(cli, Color::DarkGray)),
            Span::styled(
                app.wallet_terms.terms_url.clone(),
                style(cli, Color::Cyan).add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        Line::from(""),
        terms_action_button_line(cli, app),
        Line::from(""),
        if app.wallet_switch_editing {
            wallet_switch_input_line(cli, app)
        } else {
            wallet_switch_prompt_line(cli)
        },
        Line::from(""),
        Line::from(Span::styled(
            "You will only be asked once for this wallet on this computer.",
            style(cli, Color::DarkGray),
        )),
    ];

    if let Some(issue) = app.wallet_terms.issue.as_deref() {
        lines.push(Line::from(Span::styled(
            issue.to_string(),
            status_style(cli, issue),
        )));
    }
    lines
}

pub(in super::super) fn terms_action_button_line(cli: &Cli, app: &LabApp) -> Line<'static> {
    let focused = app.guide.focused_control.as_deref();
    let button_styles = |target: &str, color| {
        if focused == Some(target) {
            (
                terminal_button_key_style(cli, Color::LightYellow),
                terminal_button_label_style(cli)
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        } else {
            (
                terminal_button_key_style(cli, color),
                terminal_button_label_style(cli),
            )
        }
    };
    let (terms_key, terms_label) = button_styles("control:terms:open", Color::Cyan);
    let (wallet_key, wallet_label) = button_styles("control:terms:switch-wallet", Color::Blue);
    let (accept_key, accept_label) = button_styles("control:terms:accept", Color::Green);
    Line::from(vec![
        Span::styled(" T ", terms_key),
        Span::styled(" Open Terms ", terms_label),
        Span::raw("   "),
        Span::styled(" W ", wallet_key),
        Span::styled(" Switch Wallet ", wallet_label),
        Span::raw("   "),
        Span::styled(" Enter ", accept_key),
        Span::styled(" Accept ", accept_label),
    ])
}

pub(in super::super) fn terms_action_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    scroll: usize,
    column: u16,
    row: u16,
) -> Option<TermsAction> {
    let source_line = mouse_source_line_at(
        wallet_terms_lines(cli, app).len(),
        area,
        scroll,
        column,
        row,
    )?;
    if source_line != 4 {
        return None;
    }
    let inner = panel_inner_rect(area)?;
    let relative_column = column.saturating_sub(inner.x) as usize;
    terms_action_at_column(relative_column)
}

pub(in super::super) fn terms_action_at_column(column: usize) -> Option<TermsAction> {
    let buttons = [
        (
            TermsAction::OpenTerms,
            text_width(" T ") + text_width(" Open Terms "),
        ),
        (
            TermsAction::SwitchWallet,
            text_width(" W ") + text_width(" Switch Wallet "),
        ),
        (
            TermsAction::Accept,
            text_width(" Enter ") + text_width(" Accept "),
        ),
    ];
    let button_count = buttons.len();
    let mut cursor = 0usize;
    for (index, (action, width)) in buttons.into_iter().enumerate() {
        if column >= cursor && column < cursor.saturating_add(width) {
            return Some(action);
        }
        cursor = cursor.saturating_add(width);
        if index + 1 < button_count {
            cursor = cursor.saturating_add(text_width("   "));
        }
    }
    None
}

pub(in super::super) fn wallet_switch_prompt_line(cli: &Cli) -> Line<'static> {
    Line::from(Span::styled(
        "Wrong wallet? Press W to attach a different local keypair.",
        style(cli, Color::DarkGray),
    ))
}

pub(in super::super) fn wallet_switch_input_line(cli: &Cli, app: &LabApp) -> Line<'static> {
    let input = if app.wallet_switch_input.is_empty() {
        "path to keypair.json".to_string()
    } else {
        format!(
            "local signing path entered ({} characters)",
            app.wallet_switch_input.chars().count()
        )
    };
    let input_color = if app.wallet_switch_input.is_empty() {
        Color::DarkGray
    } else {
        Color::White
    };
    Line::from(vec![
        Span::styled("Wallet path ", style(cli, Color::DarkGray)),
        Span::styled(input, style(cli, input_color).add_modifier(Modifier::BOLD)),
        Span::styled("  Enter loads | Esc cancels", style(cli, Color::DarkGray)),
    ])
}
