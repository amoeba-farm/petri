//! Shared raised-button styling and geometry.

use super::super::*;

pub(in super::super) fn terminal_button_key_style(cli: &Cli, color: Color) -> Style {
    if cli.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        terminal_button_surface_style(cli, color, true)
    }
}

pub(in super::super) fn terminal_button_label_style(cli: &Cli) -> Style {
    if cli.no_color {
        Style::default()
    } else {
        tui_alt_panel_style(cli).fg(Color::White)
    }
}

pub(in super::super) fn terminal_button_surface_style(
    cli: &Cli,
    color: Color,
    active: bool,
) -> Style {
    if cli.no_color {
        let mut style = Style::default().add_modifier(Modifier::BOLD);
        if active {
            style = style.add_modifier(Modifier::REVERSED);
        }
        return style;
    }
    let bg = if active {
        button_active_color(color)
    } else {
        button_fill_color(color)
    };
    Style::default()
        .fg(Color::White)
        .bg(bg)
        .add_modifier(Modifier::BOLD)
}

pub(in super::super) fn raised_button_lines(
    cli: &Cli,
    label: &str,
    color: Color,
    active: bool,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let label = centered_text(label, width as usize);
    if height == 1 {
        return vec![Line::from(Span::styled(
            label,
            terminal_button_surface_style(cli, color, active),
        ))];
    }

    if cli.no_color {
        let inner_width = width.saturating_sub(2) as usize;
        let top = if width >= 2 {
            format!("┌{}┐", "─".repeat(inner_width))
        } else {
            " ".repeat(width as usize)
        };
        let face = if width >= 2 {
            format!("│{}│", centered_text(label.trim(), inner_width))
        } else {
            label
        };
        let bottom = if width >= 2 {
            format!("└{}┘", "─".repeat(inner_width))
        } else {
            " ".repeat(width as usize)
        };
        let button_style = terminal_button_surface_style(cli, color, active);
        let mut lines = vec![Line::from(Span::styled(top, button_style))];
        if height >= 2 {
            lines.push(Line::from(Span::styled(face, button_style)));
        }
        if height >= 3 {
            lines.push(Line::from(Span::styled(bottom, button_style)));
        }
        return lines;
    }

    let edge = " ".repeat(width as usize);
    let mut lines = Vec::with_capacity(height.min(3) as usize);
    lines.push(Line::from(Span::styled(
        edge.clone(),
        tui_surface_style(cli, button_top_edge_color(color, active)),
    )));
    lines.push(Line::from(Span::styled(
        label,
        terminal_button_surface_style(cli, color, active),
    )));
    if height >= 3 {
        lines.push(Line::from(Span::styled(
            edge,
            tui_surface_style(cli, button_shadow_color(color)),
        )));
    }
    lines
}

pub(in super::super) fn centered_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let label_width = text_width(text);
    if label_width >= width {
        return fit_text_to_width(text, width);
    }
    let left = (width - label_width) / 2;
    let right = width - label_width - left;
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

// Fill, active fill, top edge, active top edge, shadow. Unknown colors
// retain their exact value in every role; no-color styling bypasses this table.
fn button_palette(color: Color) -> [Color; 5] {
    let rgb = match color {
        Color::Rgb(126, 78, 38) => [
            (111, 67, 31),
            (147, 88, 41),
            (164, 99, 48),
            (196, 128, 68),
            (67, 39, 22),
        ],
        Color::Red | Color::LightRed => [
            (160, 28, 34),
            (192, 37, 44),
            (203, 42, 49),
            (238, 61, 68),
            (92, 18, 25),
        ],
        Color::Green | Color::LightGreen => [
            (26, 113, 61),
            (33, 139, 75),
            (42, 159, 84),
            (62, 204, 109),
            (13, 71, 40),
        ],
        Color::Yellow | Color::LightYellow => [
            (172, 132, 18),
            (207, 160, 26),
            (212, 169, 33),
            (245, 204, 60),
            (111, 83, 13),
        ],
        Color::Blue | Color::LightBlue => [
            (41, 74, 184),
            (55, 96, 220),
            (59, 102, 223),
            (88, 133, 255),
            (19, 42, 112),
        ],
        Color::Magenta | Color::LightMagenta => [
            (114, 54, 176),
            (140, 69, 214),
            (145, 72, 219),
            (177, 99, 255),
            (66, 31, 111),
        ],
        Color::Cyan | Color::LightCyan => [
            (0, 120, 148),
            (0, 149, 181),
            (0, 164, 191),
            (43, 213, 232),
            (0, 73, 94),
        ],
        other => return [other; 5],
    };
    rgb.map(|(r, g, b)| Color::Rgb(r, g, b))
}

pub(in super::super) fn button_fill_color(color: Color) -> Color {
    button_palette(color)[0]
}

pub(in super::super) fn button_active_color(color: Color) -> Color {
    button_palette(color)[1]
}

pub(in super::super) fn button_top_edge_color(color: Color, active: bool) -> Color {
    button_palette(color)[if active { 3 } else { 2 }]
}

pub(in super::super) fn button_shadow_color(color: Color) -> Color {
    button_palette(color)[4]
}
