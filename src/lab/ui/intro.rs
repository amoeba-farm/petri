//! Full-screen Petri startup animation and its bottom Open control.

use super::super::*;

const STARTUP_INTRO_BUTTON_WIDTH: u16 = 18;
const STARTUP_INTRO_BUTTON_HEIGHT: u16 = 3;
pub(in super::super) const STARTUP_INTRO_BUTTON_BACKGROUND: Color = Color::White;
pub(in super::super) const STARTUP_INTRO_BUTTON_TEXT: Color = Color::Rgb(0, 0, 0);
pub(in super::super) const STARTUP_INTRO_DARK_RED: Color = Color::Rgb(224, 18, 54);
pub(in super::super) const STARTUP_INTRO_BRIGHT_RED: Color = Color::Rgb(255, 92, 112);
pub(in super::super) const STARTUP_INTRO_BACKGROUND: Color = Color::Rgb(0, 0, 0);
pub(in super::super) const STARTUP_INTRO_CARVE_BACKGROUND: Color = TUI_BACKGROUND;
pub(in super::super) const STARTUP_INTRO_TITLE_COLOR: Color = Color::White;
pub(in super::super) const STARTUP_INTRO_DENSE_SYMBOLS: [&str; 7] =
    [" ", ".", "=", "+", "*", "#", "@"];
const STARTUP_INTRO_DENSE_SYMBOL_UNITS: [u16; 7] = [
    b' ' as u16,
    b'.' as u16,
    b'=' as u16,
    b'+' as u16,
    b'*' as u16,
    b'#' as u16,
    b'@' as u16,
];
const STARTUP_INTRO_ANIMATION_GAP: u16 = 1;
pub(in super::super) const STARTUP_INTRO_MAX_ANIMATION_HEIGHT: u16 = 36;
const STARTUP_INTRO_MIN_ANIMATION_HEIGHT: u16 = 4;
const STARTUP_INTRO_TITLE_TOP_MARGIN: u16 = 1;

#[derive(Default)]
struct StartupIntroProjectionCache {
    width: u16,
    height: u16,
    projected_frames: Vec<u8>,
    projected_symbols: Vec<u16>,
    projected_motion_mask: Vec<bool>,
    projected_bright_tone_mask: Vec<bool>,
    adjacent_change_indices: Vec<Box<[u32]>>,
}

impl StartupIntroProjectionCache {
    fn refresh(&mut self, width: u16, height: u16) {
        if self.width == width
            && self.height == height
            && !self.projected_frames.is_empty()
            && self.adjacent_change_indices.len() == STARTUP_INTRO_FRAME_COUNT
        {
            return;
        }

        self.width = width;
        self.height = height;
        let source_indices = startup_intro_source_indices_for_target(width, height);
        let target_cells = source_indices.len();
        self.projected_frames.clear();
        self.projected_frames
            .reserve(target_cells.saturating_mul(STARTUP_INTRO_FRAME_COUNT));
        for source_frame in STARTUP_INTRO_TERMINAL_FRAMES
            .chunks_exact(STARTUP_INTRO_TERMINAL_FRAME_CELLS)
            .take(STARTUP_INTRO_FRAME_COUNT)
        {
            self.projected_frames.extend(
                source_indices
                    .iter()
                    .map(|source_index| source_index.map_or(0, |index| source_frame[index])),
            );
        }
        self.projected_symbols.clear();
        self.projected_symbols.reserve(self.projected_frames.len());
        self.projected_symbols
            .extend(self.projected_frames.iter().map(|code| {
                STARTUP_INTRO_DENSE_SYMBOL_UNITS[startup_intro_terminal_symbol_index(*code)
                    .min(STARTUP_INTRO_DENSE_SYMBOL_UNITS.len() - 1)]
            }));

        let source_motion_mask = startup_intro_terminal_motion_mask();
        let source_bright_tone_mask = startup_intro_terminal_bright_tone_mask();
        self.projected_motion_mask.clear();
        self.projected_motion_mask.extend(
            source_indices
                .iter()
                .map(|source_index| source_index.is_some_and(|index| source_motion_mask[index])),
        );
        self.projected_bright_tone_mask.clear();
        self.projected_bright_tone_mask.extend(
            source_indices.iter().map(|source_index| {
                source_index.is_some_and(|index| source_bright_tone_mask[index])
            }),
        );

        self.adjacent_change_indices.clear();
        self.adjacent_change_indices
            .reserve(STARTUP_INTRO_FRAME_COUNT);
        for frame_index in 0..STARTUP_INTRO_FRAME_COUNT {
            let previous_frame_index = frame_index
                .checked_sub(1)
                .unwrap_or(STARTUP_INTRO_FRAME_COUNT - 1);
            let previous_offset = previous_frame_index * target_cells;
            let frame_offset = frame_index * target_cells;
            let changes = self.projected_frames[previous_offset..previous_offset + target_cells]
                .iter()
                .zip(&self.projected_frames[frame_offset..frame_offset + target_cells])
                .enumerate()
                .filter_map(|(target_index, (previous, current))| {
                    (previous != current).then_some(target_index as u32)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            self.adjacent_change_indices.push(changes);
        }
    }

    fn frame(&self, frame_index: usize) -> &[u8] {
        let target_cells = usize::from(self.width).saturating_mul(usize::from(self.height));
        let frame_offset = (frame_index % STARTUP_INTRO_FRAME_COUNT) * target_cells;
        &self.projected_frames[frame_offset..frame_offset + target_cells]
    }

    #[cfg(windows)]
    fn symbol_frame(&self, frame_index: usize) -> &[u16] {
        let target_cells = usize::from(self.width).saturating_mul(usize::from(self.height));
        let frame_offset = (frame_index % STARTUP_INTRO_FRAME_COUNT) * target_cells;
        &self.projected_symbols[frame_offset..frame_offset + target_cells]
    }
}

thread_local! {
    // The contain projection changes only on resize. Keeping it per render thread avoids
    // rebuilding floating-point source coordinates for every cell of every frame.
    static STARTUP_INTRO_PROJECTION_CACHE: RefCell<StartupIntroProjectionCache> =
        RefCell::new(StartupIntroProjectionCache::default());
}

pub(in super::super) fn startup_intro_open_button_rect(root: Rect) -> Rect {
    let width = root.width.min(STARTUP_INTRO_BUTTON_WIDTH);
    let height = if root.height >= STARTUP_INTRO_BUTTON_HEIGHT {
        STARTUP_INTRO_BUTTON_HEIGHT
    } else {
        root.height.min(1)
    };
    let bottom_margin = u16::from(root.height > height);
    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: root.y + root.height.saturating_sub(height + bottom_margin),
        width,
        height,
    }
}

pub(in super::super) fn startup_intro_animation_rect(root: Rect) -> Rect {
    let button = startup_intro_open_button_rect(root);
    let (title_y, available_rows, title_lines) = startup_intro_title_layout(root);
    let title_rows = title_lines.len().min(usize::from(available_rows)) as u16;
    let top = title_y
        .saturating_add(title_rows)
        .saturating_add(STARTUP_INTRO_ANIMATION_GAP);
    let bottom = button.y.saturating_sub(STARTUP_INTRO_ANIMATION_GAP);
    let available_height = bottom.saturating_sub(top);
    if root.width == 0 || available_height < STARTUP_INTRO_MIN_ANIMATION_HEIGHT {
        return Rect {
            x: root.x,
            y: top,
            width: 0,
            height: 0,
        };
    }

    let max_height = available_height.min(STARTUP_INTRO_MAX_ANIMATION_HEIGHT);
    let width_for_height = (u32::from(max_height) * STARTUP_INTRO_SOURCE_VIEW_COLS as u32
        / STARTUP_INTRO_SOURCE_VIEW_ROWS as u32)
        .min(u32::from(u16::MAX)) as u16;
    let (width, height) = if width_for_height <= root.width {
        (width_for_height, max_height)
    } else {
        let height_for_width = (u32::from(root.width) * STARTUP_INTRO_SOURCE_VIEW_ROWS as u32
            / STARTUP_INTRO_SOURCE_VIEW_COLS as u32)
            .min(u32::from(max_height)) as u16;
        (root.width, height_for_width)
    };
    if height < STARTUP_INTRO_MIN_ANIMATION_HEIGHT {
        return Rect {
            x: root.x,
            y: top,
            width: 0,
            height: 0,
        };
    }

    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: top + available_height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(in super::super) fn draw_startup_intro(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    fill_tui_area(frame, cli, root, STARTUP_INTRO_BACKGROUND);
    if root.width == 0 || root.height == 0 {
        return;
    }

    let frame_index = if gitbook_reduced_motion() {
        0
    } else {
        app.startup_intro_frame_index_at(Instant::now())
    };
    let animation = startup_intro_animation_rect(root);
    if animation.width > 0 && animation.height > 0 {
        draw_startup_intro_animation(frame, cli, animation, frame_index);
    }
    draw_startup_intro_title(frame, cli, root);

    let button = startup_intro_open_button_rect(root);
    if button.width == 0 || button.height == 0 {
        return;
    }
    frame.render_widget(Clear, button);
    frame.render_widget(
        Paragraph::new(startup_intro_open_button_lines(
            cli,
            button.width,
            button.height,
        )),
        button,
    );
}

fn startup_intro_open_button_lines(cli: &Cli, width: u16, height: u16) -> Vec<Line<'static>> {
    if cli.no_color {
        return raised_button_lines(cli, "Open", Color::White, true, width, height);
    }
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let style = Style::default()
        .fg(STARTUP_INTRO_BUTTON_TEXT)
        .bg(STARTUP_INTRO_BUTTON_BACKGROUND)
        .add_modifier(Modifier::BOLD);
    let face = centered_text("Open", usize::from(width));
    if height == 1 {
        return vec![Line::from(Span::styled(face, style))];
    }

    let edge = " ".repeat(usize::from(width));
    let mut lines = vec![Line::from(Span::styled(edge.clone(), style))];
    lines.push(Line::from(Span::styled(face, style)));
    if height >= 3 {
        lines.push(Line::from(Span::styled(edge, style)));
    }
    lines
}

fn draw_startup_intro_animation(frame: &mut Frame<'_>, cli: &Cli, root: Rect, frame_index: usize) {
    let target_width = usize::from(root.width);
    let buffer = frame.buffer_mut();
    let dark_style = Style::default()
        .fg(STARTUP_INTRO_DARK_RED)
        .bg(STARTUP_INTRO_CARVE_BACKGROUND)
        .add_modifier(Modifier::BOLD);
    let bright_style = Style::default()
        .fg(STARTUP_INTRO_BRIGHT_RED)
        .bg(STARTUP_INTRO_CARVE_BACKGROUND)
        .add_modifier(Modifier::BOLD);

    STARTUP_INTRO_PROJECTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.refresh(root.width, root.height);
        let projected_frame = cache.frame(frame_index);

        for (target_index, code) in projected_frame.iter().copied().enumerate() {
            let column = root.x + (target_index % target_width) as u16;
            let row = root.y + (target_index / target_width) as u16;
            let cell = &mut buffer[(column, row)];
            if !cli.no_color && cache.projected_motion_mask[target_index] {
                cell.set_symbol(" ")
                    .set_style(if cache.projected_bright_tone_mask[target_index] {
                        bright_style
                    } else {
                        dark_style
                    });
            }
            let symbol_index = startup_intro_terminal_symbol_index(code);
            if symbol_index == 0 {
                continue;
            }
            let symbol = STARTUP_INTRO_DENSE_SYMBOLS
                .get(symbol_index)
                .copied()
                .unwrap_or(" ");
            cell.set_symbol(symbol);
        }
    });
}

pub(in super::super) fn draw_startup_intro_animation_delta<W: Write>(
    backend: &mut CrosstermBackend<W>,
    cli: &Cli,
    root: Rect,
    previous_frame_index: usize,
    frame_index: usize,
) -> io::Result<()> {
    if root.width == 0 || root.height == 0 || previous_frame_index == frame_index {
        return Ok(());
    }

    let target_width = usize::from(root.width);
    let target_cells = target_width.saturating_mul(usize::from(root.height));
    let mut output = Vec::new();
    let mut last_position: Option<(u16, u16)> = None;
    let mut last_style = None;

    STARTUP_INTRO_PROJECTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.refresh(root.width, root.height);
        let previous_frame_index = previous_frame_index % STARTUP_INTRO_FRAME_COUNT;
        let frame_index = frame_index % STARTUP_INTRO_FRAME_COUNT;
        let previous_frame = cache.frame(previous_frame_index);
        let projected_frame = cache.frame(frame_index);
        let adjacent_previous = frame_index
            .checked_sub(1)
            .unwrap_or(STARTUP_INTRO_FRAME_COUNT - 1);
        if previous_frame_index == adjacent_previous {
            let changes = &cache.adjacent_change_indices[frame_index];
            output.reserve(changes.len().saturating_mul(12));
            for target_index in changes.iter().map(|index| *index as usize) {
                push_startup_intro_projected_change(
                    &mut output,
                    &mut last_position,
                    &mut last_style,
                    cli,
                    root,
                    target_width,
                    target_index,
                    projected_frame[target_index],
                    cache.projected_bright_tone_mask[target_index],
                );
            }
        } else {
            output.reserve(target_cells);
            for (target_index, (previous_code, code)) in previous_frame
                .iter()
                .copied()
                .zip(projected_frame.iter().copied())
                .enumerate()
            {
                if previous_code != code {
                    push_startup_intro_projected_change(
                        &mut output,
                        &mut last_position,
                        &mut last_style,
                        cli,
                        root,
                        target_width,
                        target_index,
                        code,
                        cache.projected_bright_tone_mask[target_index],
                    );
                }
            }
        }
    });

    if !cli.no_color && last_style.is_some() {
        output.extend_from_slice(b"\x1b[0m");
    }
    backend.write_all(&output)
}

#[allow(clippy::too_many_arguments)]
fn push_startup_intro_projected_change(
    output: &mut Vec<u8>,
    last_position: &mut Option<(u16, u16)>,
    last_style: &mut Option<StartupIntroTerminalStyle>,
    cli: &Cli,
    root: Rect,
    target_width: usize,
    target_index: usize,
    code: u8,
    bright_tone: bool,
) {
    let column = root.x + (target_index % target_width) as u16;
    let row = root.y + (target_index / target_width) as u16;
    if !matches!(*last_position, Some((last_column, last_row)) if last_row == row && last_column.checked_add(1) == Some(column))
    {
        push_startup_intro_cursor_position(output, column, row);
    }

    let symbol_index =
        startup_intro_terminal_symbol_index(code).min(STARTUP_INTRO_DENSE_SYMBOLS.len() - 1);
    if !cli.no_color {
        let style = startup_intro_terminal_style(bright_tone);
        if *last_style != Some(style) {
            output.extend_from_slice(style.escape());
            *last_style = Some(style);
        }
    }
    output.extend_from_slice(STARTUP_INTRO_DENSE_SYMBOLS[symbol_index].as_bytes());
    *last_position = Some((column, row));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupIntroTerminalStyle {
    Dark,
    Bright,
}

impl StartupIntroTerminalStyle {
    const fn escape(self) -> &'static [u8] {
        match self {
            Self::Dark => b"\x1b[1;38;2;224;18;54;48;2;7;21;48m",
            Self::Bright => b"\x1b[1;38;2;255;92;112;48;2;7;21;48m",
        }
    }
}

fn startup_intro_terminal_style(bright_tone: bool) -> StartupIntroTerminalStyle {
    if bright_tone {
        StartupIntroTerminalStyle::Bright
    } else {
        StartupIntroTerminalStyle::Dark
    }
}

#[cfg(windows)]
pub(in super::super) fn startup_intro_native_animation_available(root: Rect) -> bool {
    startup_intro_console_output(root).is_some()
}

#[cfg(not(windows))]
pub(in super::super) fn startup_intro_native_animation_available(_root: Rect) -> bool {
    false
}

#[cfg(windows)]
fn startup_intro_console_output(root: Rect) -> Option<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Console::{
            CONSOLE_SCREEN_BUFFER_INFO, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
            STD_OUTPUT_HANDLE,
        },
    };

    if !startup_intro_has_visible_classic_console() {
        return None;
    }
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut mode = 0;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return None;
    }
    let mut info = unsafe { std::mem::zeroed::<CONSOLE_SCREEN_BUFFER_INFO>() };
    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return None;
    }
    let right = i32::from(root.x) + i32::from(root.width);
    let bottom = i32::from(root.y) + i32::from(root.height);
    if right > i32::from(info.dwSize.X) || bottom > i32::from(info.dwSize.Y) {
        return None;
    }
    Some(handle)
}

#[cfg(windows)]
fn startup_intro_has_visible_classic_console() -> bool {
    use std::sync::OnceLock;
    use windows_sys::Win32::{
        System::Console::GetConsoleWindow,
        UI::WindowsAndMessaging::{GA_PARENT, GetAncestor, HWND_MESSAGE, IsWindowVisible},
    };

    static VISIBLE_CLASSIC_CONSOLE: OnceLock<bool> = OnceLock::new();
    *VISIBLE_CLASSIC_CONSOLE.get_or_init(|| {
        let virtual_terminal_environment = [
            "WT_SESSION",
            "TERM_PROGRAM",
            "COLORTERM",
            "ConEmuANSI",
            "ANSICON",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
            || std::env::var_os("TERM")
                .is_some_and(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dumb"));
        if virtual_terminal_environment {
            return false;
        }

        let console_window = unsafe { GetConsoleWindow() };
        !console_window.is_null()
            && unsafe { IsWindowVisible(console_window) } != 0
            && unsafe { GetAncestor(console_window, GA_PARENT) } != HWND_MESSAGE
    })
}

#[cfg(windows)]
pub(in super::super) fn draw_startup_intro_animation_native(
    root: Rect,
    previous_frame_index: usize,
    frame_index: usize,
) -> io::Result<bool> {
    use windows_sys::Win32::System::Console::{COORD, WriteConsoleOutputCharacterW};

    if root.width == 0 || root.height == 0 || previous_frame_index == frame_index {
        return Ok(true);
    }
    let Some(handle) = startup_intro_console_output(root) else {
        return Ok(false);
    };
    let target_width = usize::from(root.width);
    STARTUP_INTRO_PROJECTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.refresh(root.width, root.height);
        let previous_frame_index = previous_frame_index % STARTUP_INTRO_FRAME_COUNT;
        let frame_index = frame_index % STARTUP_INTRO_FRAME_COUNT;
        let adjacent_previous = frame_index
            .checked_sub(1)
            .unwrap_or(STARTUP_INTRO_FRAME_COUNT - 1);
        let owned_changes;
        let changes: &[u32] = if previous_frame_index == adjacent_previous {
            &cache.adjacent_change_indices[frame_index]
        } else {
            let previous_frame = cache.frame(previous_frame_index);
            let projected_frame = cache.frame(frame_index);
            owned_changes = previous_frame
                .iter()
                .zip(projected_frame)
                .enumerate()
                .filter_map(|(target_index, (previous, current))| {
                    (previous != current).then_some(target_index as u32)
                })
                .collect::<Vec<_>>();
            &owned_changes
        };
        let symbols = cache.symbol_frame(frame_index);
        let mut cursor = 0;
        while cursor < changes.len() {
            let first_index = changes[cursor] as usize;
            let row = first_index / target_width;
            let start_column = first_index % target_width;
            let mut end_column = start_column + 1;
            cursor += 1;
            while cursor < changes.len() {
                let target_index = changes[cursor] as usize;
                if target_index / target_width != row {
                    break;
                }
                end_column = target_index % target_width + 1;
                cursor += 1;
            }
            let start_index = row * target_width + start_column;
            let end_index = row * target_width + end_column;
            let units = &symbols[start_index..end_index];
            let mut written = 0;
            let Ok(x) = i16::try_from(u32::from(root.x) + start_column as u32) else {
                return Ok(false);
            };
            let Ok(y) = i16::try_from(u32::from(root.y) + row as u32) else {
                return Ok(false);
            };
            let coordinate = COORD { X: x, Y: y };
            let success = unsafe {
                WriteConsoleOutputCharacterW(
                    handle,
                    units.as_ptr(),
                    units.len() as u32,
                    coordinate,
                    &mut written,
                )
            };
            if success == 0 {
                // ConPTY can resize its screen buffer between the availability check and
                // this write. Fall back to the regular ANSI delta for this frame instead
                // of turning that transient race into a fatal TUI error.
                return Ok(false);
            }
            if written as usize != units.len() {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

#[cfg(not(windows))]
pub(in super::super) fn draw_startup_intro_animation_native(
    _root: Rect,
    _previous_frame_index: usize,
    _frame_index: usize,
) -> io::Result<bool> {
    Ok(false)
}

fn push_startup_intro_cursor_position(output: &mut Vec<u8>, column: u16, row: u16) {
    output.extend_from_slice(b"\x1b[");
    push_startup_intro_decimal(output, row.saturating_add(1));
    output.push(b';');
    push_startup_intro_decimal(output, column.saturating_add(1));
    output.push(b'H');
}

fn push_startup_intro_decimal(output: &mut Vec<u8>, value: u16) {
    let mut digits = [0_u8; 5];
    let mut cursor = digits.len();
    let mut remaining = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    output.extend_from_slice(&digits[cursor..]);
}

fn draw_startup_intro_title(frame: &mut Frame<'_>, cli: &Cli, root: Rect) {
    let (title_y, available_rows, title_lines) = startup_intro_title_layout(root);
    if available_rows == 0 {
        return;
    }

    let color_enabled = terminal_brand::color_enabled(cli.no_color, true);
    let buffer = frame.buffer_mut();

    for (line_index, line) in title_lines
        .iter()
        .take(usize::from(available_rows))
        .enumerate()
    {
        let line_width = line
            .iter()
            .map(|span| text_width(&span.text))
            .sum::<usize>()
            .min(usize::from(root.width));
        let mut column = root.x + root.width.saturating_sub(line_width as u16) / 2;
        let row = title_y + line_index as u16;
        let mut remaining = root.right().saturating_sub(column);

        for span in line {
            if remaining == 0 {
                break;
            }
            let span_style = startup_intro_title_span_style(span, color_enabled);
            if color_enabled && span.fill {
                let width = text_width(&span.text).min(usize::from(remaining)) as u16;
                for offset in 0..width {
                    buffer[(column + offset, row)]
                        .set_symbol(" ")
                        .set_style(span_style);
                }
                column += width;
                remaining = remaining.saturating_sub(width);
            } else {
                let (next_column, _) =
                    buffer.set_stringn(column, row, &span.text, usize::from(remaining), span_style);
                let rendered = next_column.saturating_sub(column);
                column = next_column;
                remaining = remaining.saturating_sub(rendered);
            }
        }
    }
}

fn startup_intro_title_layout(root: Rect) -> (u16, u16, &'static [terminal_brand::BrandLine]) {
    let button = startup_intro_open_button_rect(root);
    let title_y = root.y + STARTUP_INTRO_TITLE_TOP_MARGIN.min(root.height.saturating_sub(1));
    let available_rows = button.y.saturating_sub(title_y);
    if available_rows == 0 {
        return (title_y, 0, &[]);
    }

    let wide_title_rows =
        terminal_brand::terminal_title_lines_for_width(terminal_brand::WIDE_BANNER_MIN_COLUMNS)
            .len();
    let use_wide_title = usize::from(root.width) >= terminal_brand::WIDE_BANNER_MIN_COLUMNS
        && usize::from(available_rows) > wide_title_rows;
    let title_width = if use_wide_title {
        usize::from(root.width)
    } else {
        terminal_brand::WIDE_BANNER_MIN_COLUMNS.saturating_sub(1)
    };
    (
        title_y,
        available_rows,
        terminal_brand::terminal_title_lines_for_width(title_width),
    )
}

fn startup_intro_title_span_style(span: &terminal_brand::BrandSpan, color_enabled: bool) -> Style {
    if color_enabled && span.tone != terminal_brand::BrandTone::Plain {
        let style = Style::default().fg(STARTUP_INTRO_TITLE_COLOR);
        if span.fill {
            style.bg(STARTUP_INTRO_TITLE_COLOR)
        } else {
            style
        }
    } else {
        Style::default()
    }
}
