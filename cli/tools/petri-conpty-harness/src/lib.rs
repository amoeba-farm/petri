use std::{
    env,
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};
use vt100::{Callbacks, Color as VtColor, Parser, Screen};

pub type HarnessResult<T> = Result<T, HarnessError>;

const DEFAULT_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const READ_CHUNK_SIZE: usize = 8 * 1024;
const READ_CHANNEL_DEPTH: usize = 64;
const PUMP_SLICE: Duration = Duration::from_millis(25);
const INPUT_SETTLE: Duration = Duration::from_millis(80);
const HOVER_SETTLE: Duration = Duration::from_millis(1_500);
const CLEAN_EXIT_TIMEOUT: Duration = Duration::from_secs(1);
const SYNCHRONIZED_UPDATE_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNCHRONIZED_UPDATE_END: &[u8] = b"\x1b[?2026l";
const MAX_EXPANDED_INTRO_OUTPUT_BYTES_PER_SECOND: usize = 200_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    fn as_pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl fmt::Display for TerminalSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.cols, self.rows)
    }
}

impl FromStr for TerminalSize {
    type Err = HarnessError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        let mut fields = value.split(['x', 'X']);
        let cols = fields
            .next()
            .filter(|field| !field.is_empty())
            .ok_or_else(|| HarnessError::new("terminal size must use COLSxROWS"))?
            .parse::<u16>()
            .map_err(|_| HarnessError::new("terminal columns must be a positive integer"))?;
        let rows = fields
            .next()
            .filter(|field| !field.is_empty())
            .ok_or_else(|| HarnessError::new("terminal size must use COLSxROWS"))?
            .parse::<u16>()
            .map_err(|_| HarnessError::new("terminal rows must be a positive integer"))?;
        if fields.next().is_some() {
            return Err(HarnessError::new("terminal size must use COLSxROWS"));
        }
        if cols == 0 || rows == 0 {
            return Err(HarnessError::new(
                "terminal columns and rows must both be greater than zero",
            ));
        }
        Ok(Self { cols, rows })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scenario {
    Startup,
    HelpKeyboardPreview,
    HelpMouseMove,
    GuideOff,
    HelpResize,
}

impl Scenario {
    pub const ALL: [Self; 5] = [
        Self::Startup,
        Self::HelpKeyboardPreview,
        Self::HelpMouseMove,
        Self::GuideOff,
        Self::HelpResize,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::HelpKeyboardPreview => "help-keyboard-preview",
            Self::HelpMouseMove => "help-mouse-move",
            Self::GuideOff => "guide-off",
            Self::HelpResize => "help-resize",
        }
    }

    pub const fn default_size(self) -> TerminalSize {
        match self {
            Self::HelpResize => TerminalSize::new(120, 32),
            _ => TerminalSize::new(80, 24),
        }
    }

    const fn reduced_motion_env_value(self) -> &'static str {
        match self {
            Self::Startup => "0",
            _ => "1",
        }
    }
}

impl FromStr for Scenario {
    type Err = HarnessError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "startup" | "home" => Ok(Self::Startup),
            "help-keyboard-preview" | "help-keyboard" => Ok(Self::HelpKeyboardPreview),
            "help-mouse-move" | "help-hover" | "mouse" => Ok(Self::HelpMouseMove),
            "guide-off" | "guide" => Ok(Self::GuideOff),
            "help-resize" | "resize" => Ok(Self::HelpResize),
            _ => Err(HarnessError::new(format!(
                "unknown scenario {value:?}; expected startup, help-keyboard-preview, help-mouse-move, guide-off, or help-resize"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub binary: PathBuf,
    pub repo_dir: PathBuf,
    pub timeout: Duration,
    pub output_limit: usize,
    pub artifact_dir: Option<PathBuf>,
}

impl RunOptions {
    pub fn new(binary: PathBuf, repo_dir: PathBuf, timeout: Duration) -> Self {
        Self {
            binary,
            repo_dir,
            timeout,
            output_limit: DEFAULT_OUTPUT_LIMIT,
            artifact_dir: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScenarioReport {
    pub scenario: Scenario,
    pub size: TerminalSize,
    pub elapsed: Duration,
    pub detail: String,
    pub screen: String,
    pub artifact: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessError {
    message: String,
}

impl HarnessError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn with_screen(message: impl AsRef<str>, screen: &str) -> Self {
        let screen = normalize_screen(screen);
        if screen.is_empty() {
            Self::new(message.as_ref())
        } else {
            Self::new(format!(
                "{}\n\ncurrent screen:\n{}",
                message.as_ref(),
                screen
            ))
        }
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HarnessError {}

impl From<std::io::Error> for HarnessError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

pub fn normalize_screen(contents: &str) -> String {
    let mut lines = contents
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

pub fn sgr_mouse_move(column: u16, row: u16) -> Vec<u8> {
    format!("\x1b[<35;{};{}M", column.max(1), row.max(1)).into_bytes()
}

pub fn sgr_mouse_button(column: u16, row: u16, pressed: bool) -> Vec<u8> {
    format!(
        "\x1b[<0;{};{}{}",
        column.max(1),
        row.max(1),
        if pressed { 'M' } else { 'm' }
    )
    .into_bytes()
}

pub fn run_scenario(
    options: &RunOptions,
    scenario: Scenario,
    size: TerminalSize,
) -> HarnessResult<ScenarioReport> {
    if !cfg!(windows) {
        return Err(HarnessError::new(
            "the Petri ConPTY harness requires native Windows",
        ));
    }
    if size.cols < 20 || size.rows < 10 {
        return Err(HarnessError::new(
            "test terminals must be at least 20 columns by 10 rows",
        ));
    }

    let binary = options.binary.canonicalize().map_err(|error| {
        HarnessError::new(format!(
            "could not resolve Petri binary {}: {error}",
            options.binary.display()
        ))
    })?;
    if !binary.is_file() {
        return Err(HarnessError::new(format!(
            "Petri binary is not a file: {}",
            binary.display()
        )));
    }
    let binary_name = binary
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if !binary_name.eq_ignore_ascii_case("petri.exe") {
        return Err(HarnessError::new(format!(
            "refusing to launch non-Petri binary {binary_name:?}; pass target\\release\\petri.exe or target\\debug\\petri.exe"
        )));
    }
    let repo_dir = options.repo_dir.canonicalize().map_err(|error| {
        HarnessError::new(format!(
            "could not resolve Petri repository {}: {error}",
            options.repo_dir.display()
        ))
    })?;

    let started = Instant::now();
    let sandbox = Sandbox::create()?;
    let stub = LoopbackStub::start()?;
    let launch = LaunchContext::prepare(&sandbox, stub.url(), scenario)?;
    let mut session = PtySession::spawn(
        &binary,
        &repo_dir,
        &launch,
        size,
        options.timeout,
        options.output_limit,
    )?;

    let (screen, detail) = match scenario {
        Scenario::Startup => scenario_startup(&mut session)?,
        Scenario::HelpKeyboardPreview => scenario_help_keyboard_preview(&mut session)?,
        Scenario::HelpMouseMove => scenario_help_mouse_move(&mut session)?,
        Scenario::GuideOff => scenario_guide_off(&mut session)?,
        Scenario::HelpResize => scenario_help_resize(&mut session)?,
    };
    session.assert_no_terminal_failure()?;
    session.quit(Duration::from_secs(3))?;
    let detail = format!("{detail}; q exited cleanly and restored the primary screen");

    let artifact = options
        .artifact_dir
        .as_ref()
        .map(|dir| write_artifact(dir, scenario, size, &screen))
        .transpose()?;
    drop(session);
    drop(stub);
    drop(sandbox);

    Ok(ScenarioReport {
        scenario,
        size,
        elapsed: started.elapsed(),
        detail,
        screen,
        artifact,
    })
}

fn scenario_startup(session: &mut PtySession) -> HarnessResult<(String, String)> {
    let (detail, screen) = if session.size == TerminalSize::new(120, 32) {
        session.wait_for_intro()?;
        require_intro_theme(session, "startup intro")?;
        require_intro_safe_margins(session, "startup intro")?;
        let expanded = TerminalSize::new(320, 90);
        session.observe_presented_nonblank_cells = true;
        let expanded_presentations_from = session.presented_nonblank_cells.len();
        let expanded_sync_from = session.presentation_times.len();
        session.resize(expanded)?;
        session.require_screen(
            &format!("expanded startup intro at {expanded}"),
            Duration::from_secs(4),
            |_, text| {
                let title_marks = text
                    .lines()
                    .take(9)
                    .flat_map(str::chars)
                    .filter(|character| "┌┐└┘─│├┤┬┴┼".contains(*character))
                    .count();
                text.contains("Open") && !text.contains("Trade options") && title_marks >= 24
            },
        )?;
        require_intro_theme(session, "expanded startup intro")?;
        require_intro_safe_margins(session, "expanded startup intro")?;
        session.wait_for_synchronized_presentation(
            expanded_sync_from,
            "expanded startup intro",
            Duration::from_secs(2),
        )?;
        session.finish_presented_nonblank_observation();
        require_no_blank_presentations(
            &session.presented_nonblank_cells[expanded_presentations_from..],
            "expanded startup intro",
        )?;

        let (distinct_frames, presented_frames, max_frame_gap, sampled_bytes, sample_elapsed) =
            require_intro_motion(
                session,
                "expanded native-cadence startup intro",
                12,
                Duration::from_millis(60),
                10,
            )?;
        let sampled_bytes_per_second = intro_output_bytes_per_second(sampled_bytes, sample_elapsed);
        require_intro_output_budget(
            session,
            "expanded native-cadence startup intro",
            sampled_bytes_per_second,
            MAX_EXPANDED_INTRO_OUTPUT_BYTES_PER_SECOND,
        )?;
        let compact = TerminalSize::new(80, 24);
        session.observe_presented_nonblank_cells = true;
        let compact_presentations_from = session.presented_nonblank_cells.len();
        let compact_sync_from = session.presentation_times.len();
        session.resize(compact)?;
        session.require_screen(
            &format!("shrunk startup intro at {compact}"),
            Duration::from_secs(4),
            |_, text| {
                let title_marks = text
                    .lines()
                    .take(9)
                    .flat_map(str::chars)
                    .filter(|character| "┌┐└┘─│├┤┬┴┼".contains(*character))
                    .count();
                text.contains("Open") && !text.contains("Trade options") && title_marks >= 24
            },
        )?;
        require_intro_theme(session, "shrunk startup intro")?;
        require_intro_safe_margins(session, "shrunk startup intro")?;
        require_no_intro_animation_below_open(session, "shrunk startup intro")?;
        session.wait_for_synchronized_presentation(
            compact_sync_from,
            "shrunk startup intro",
            Duration::from_secs(2),
        )?;
        session.finish_presented_nonblank_observation();
        require_no_blank_presentations(
            &session.presented_nonblank_cells[compact_presentations_from..],
            "shrunk startup intro",
        )?;
        let screen = session.screen_text();
        session.observe_presented_nonblank_cells = true;
        let home_presentations_from = session.presented_nonblank_cells.len();
        let home_sync_from = session.presentation_times.len();
        session.enter_home()?;
        session.wait_for_synchronized_presentation(
            home_sync_from,
            "startup intro to Home transition",
            Duration::from_secs(2),
        )?;
        session.finish_presented_nonblank_observation();
        require_no_blank_presentations(
            &session.presented_nonblank_cells[home_presentations_from..],
            "startup intro to Home transition",
        )?;
        (
            format!(
                "the titled ASCII startup intro stayed animated after a native resize to {expanded} ({} sampled frames, {presented_frames} synchronized frames, {} ms maximum frame gap, {sampled_bytes} terminal bytes in {} ms, {sampled_bytes_per_second} bytes/s), then shrank cleanly to {compact} before Enter opened Home without a blank intermediate frame",
                distinct_frames,
                max_frame_gap.as_millis(),
                sample_elapsed.as_millis()
            ),
            screen,
        )
    } else {
        session.wait_for_intro()?;
        require_intro_theme(session, "startup intro")?;
        require_intro_safe_margins(session, "startup intro")?;
        let (distinct_frames, presented_frames, max_frame_gap, sampled_bytes, sample_elapsed) =
            require_intro_motion(
                session,
                "native-cadence startup intro",
                8,
                Duration::from_millis(60),
                6,
            )?;
        let screen = session.screen_text();
        session.observe_presented_nonblank_cells = true;
        let home_presentations_from = session.presented_nonblank_cells.len();
        let home_sync_from = session.presentation_times.len();
        session.enter_home()?;
        session.wait_for_synchronized_presentation(
            home_sync_from,
            "startup intro to Home transition",
            Duration::from_secs(2),
        )?;
        session.finish_presented_nonblank_observation();
        require_no_blank_presentations(
            &session.presented_nonblank_cells[home_presentations_from..],
            "startup intro to Home transition",
        )?;
        (
            format!(
                "the titled ASCII startup intro produced {distinct_frames} sampled frames and {presented_frames} synchronized frames with a {} ms maximum frame gap; {sampled_bytes} terminal bytes in {} ms before Enter opened Home without a blank intermediate frame",
                max_frame_gap.as_millis(),
                sample_elapsed.as_millis()
            ),
            screen,
        )
    };
    if session.size == TerminalSize::new(60, 20) {
        require_hidden_lines_footer(session, "short Home hidden-lines footer")?;
    }
    Ok((screen, detail))
}

fn require_intro_theme(session: &PtySession, label: &str) -> HarnessResult<()> {
    const CARVE_NAVY: VtColor = VtColor::Rgb(7, 21, 48);
    const DARK_RED: VtColor = VtColor::Rgb(224, 18, 54);
    const BRIGHT_RED: VtColor = VtColor::Rgb(255, 92, 112);

    let screen = session.parser.screen();
    let (rows, cols) = screen.size();
    let mut black_canvas_cells = 0;
    let mut navy_carve_cells = 0;
    let mut red_animation_cells = 0;
    let mut white_title_cells = 0;
    let mut background_counts = std::collections::BTreeMap::<String, usize>::new();
    for row in 0..rows {
        for column in 0..cols {
            let Some(cell) = screen.cell(row, column) else {
                continue;
            };
            *background_counts
                .entry(format!("{:?}", cell.bgcolor()))
                .or_default() += 1;
            if intro_color_is_black(cell.bgcolor()) {
                black_canvas_cells += 1;
            } else if cell.bgcolor() == CARVE_NAVY {
                navy_carve_cells += 1;
            }
            if cell.bgcolor() == CARVE_NAVY
                && matches!(cell.fgcolor(), DARK_RED | BRIGHT_RED)
                && !cell.contents().trim().is_empty()
            {
                red_animation_cells += 1;
            }
            if row < 9
                && (intro_color_is_white(cell.fgcolor()) || intro_color_is_white(cell.bgcolor()))
            {
                white_title_cells += 1;
            }
        }
    }

    let open = screen
        .rows(0, cols)
        .enumerate()
        .find_map(|(row, line)| {
            let byte_column = line.find("Open")?;
            Some((row as u16, line[..byte_column].chars().count() as u16))
        })
        .ok_or_else(|| {
            HarnessError::with_screen(format!("{label} has no Open button"), &screen.contents())
        })?;
    let (open_row, open_column) = open;
    let button_left = open_column.saturating_sub(7);
    let button_top = open_row.saturating_sub(1);
    let button_is_solid_white = (button_top..button_top.saturating_add(3)).all(|row| {
        (button_left..button_left.saturating_add(18)).all(|column| {
            screen
                .cell(row, column)
                .is_some_and(|cell| intro_color_is_white(cell.bgcolor()))
        })
    });
    let open_is_black_on_white = "Open".chars().enumerate().all(|(offset, expected)| {
        screen
            .cell(open_row, open_column.saturating_add(offset as u16))
            .is_some_and(|cell| {
                cell.contents() == expected.to_string()
                    && intro_color_is_black(cell.fgcolor())
                    && intro_color_is_white(cell.bgcolor())
            })
    });

    if black_canvas_cells == 0
        || navy_carve_cells == 0
        || red_animation_cells < 8
        || white_title_cells < 24
        || !button_is_solid_white
        || !open_is_black_on_white
    {
        return Err(HarnessError::with_screen(
            format!(
                "{label} did not preserve the black/white/navy/red intro theme: {black_canvas_cells} black canvas cells, {white_title_cells} white title cells, {navy_carve_cells} navy carve cells, {red_animation_cells} red animation cells, solid white button={button_is_solid_white}, black Open label={open_is_black_on_white}; backgrounds={background_counts:?}"
            ),
            &screen.contents(),
        ));
    }
    Ok(())
}

fn intro_color_is_black(color: VtColor) -> bool {
    matches!(color, VtColor::Idx(0) | VtColor::Rgb(0, 0, 0))
}

fn intro_color_is_white(color: VtColor) -> bool {
    matches!(
        color,
        VtColor::Idx(7) | VtColor::Idx(15) | VtColor::Rgb(255, 255, 255)
    )
}

fn require_intro_motion(
    session: &mut PtySession,
    label: &str,
    sample_count: usize,
    sample_interval: Duration,
    minimum_distinct_frames: usize,
) -> HarnessResult<(usize, usize, Duration, usize, Duration)> {
    // A native resize can leave one large synchronized frame in the ConPTY pipe.
    // Drain that one-time redraw before measuring the immutable animation's steady cadence.
    session.settle(Duration::from_millis(180))?;
    let mut distinct_frames = Vec::new();
    let mut sampled_output_bytes = Vec::new();
    let sample_started = Instant::now();
    let output_bytes_before_samples = session.raw_output.len();
    let presentations_before_samples = session.presentation_times.len();
    for _ in 0..sample_count {
        session.settle(sample_interval)?;
        sampled_output_bytes.push(session.raw_output.len());
        let screen = session.screen_text();
        if !distinct_frames.iter().any(|frame| frame == &screen) {
            distinct_frames.push(screen);
        }
    }
    let elapsed = sample_started.elapsed();
    if distinct_frames.len() < minimum_distinct_frames {
        return Err(HarnessError::with_screen(
            format!(
                "{label} produced only {} distinct frame(s) in {} ms; output bytes by sample: {:?}",
                distinct_frames.len(),
                elapsed.as_millis(),
                sampled_output_bytes
            ),
            &session.parser.screen().contents(),
        ));
    }
    let sampled_bytes = session
        .raw_output
        .len()
        .saturating_sub(output_bytes_before_samples);
    let presentation_times = &session.presentation_times[presentations_before_samples..];
    let frame_gaps = presentation_times
        .windows(2)
        .map(|pair| pair[1].saturating_duration_since(pair[0]))
        .collect::<Vec<_>>();
    let max_frame_gap = frame_gaps.iter().copied().max().unwrap_or_default();
    let expected_presented_frames = elapsed.as_nanos() / Duration::from_millis(30).as_nanos();
    let minimum_presented_frames = ((expected_presented_frames * 4) / 5).max(1) as usize;
    if presentation_times.len() < minimum_presented_frames
        || max_frame_gap > Duration::from_millis(120)
    {
        return Err(HarnessError::with_screen(
            format!(
                "{label} presented {} synchronized frame(s) in {} ms with a {} ms maximum gap; gaps: {:?}",
                presentation_times.len(),
                elapsed.as_millis(),
                max_frame_gap.as_millis(),
                frame_gaps
                    .iter()
                    .map(Duration::as_millis)
                    .collect::<Vec<_>>()
            ),
            &session.parser.screen().contents(),
        ));
    }
    Ok((
        distinct_frames.len(),
        presentation_times.len(),
        max_frame_gap,
        sampled_bytes,
        elapsed,
    ))
}

fn intro_output_bytes_per_second(sampled_bytes: usize, elapsed: Duration) -> usize {
    let elapsed_nanos = elapsed.as_nanos().max(1);
    ((sampled_bytes as u128 * 1_000_000_000) / elapsed_nanos).min(usize::MAX as u128) as usize
}

fn require_intro_output_budget(
    session: &PtySession,
    label: &str,
    bytes_per_second: usize,
    maximum_bytes_per_second: usize,
) -> HarnessResult<()> {
    if bytes_per_second > maximum_bytes_per_second {
        return Err(HarnessError::with_screen(
            format!(
                "{label} emitted {bytes_per_second} terminal bytes/s, above the {maximum_bytes_per_second} bytes/s regression budget"
            ),
            &session.parser.screen().contents(),
        ));
    }
    Ok(())
}

fn require_intro_safe_margins(session: &PtySession, label: &str) -> HarnessResult<()> {
    let text = session.screen_text();
    let lines = text.lines().collect::<Vec<_>>();
    let open_row = lines
        .iter()
        .position(|line| line.contains("Open"))
        .ok_or_else(|| HarnessError::with_screen(format!("{label} has no Open button"), &text))?;
    let title_last_row = lines
        .iter()
        .enumerate()
        .take(open_row.min(10))
        .filter(|(_, line)| {
            line.contains("AMOEBA FARM")
                || line
                    .chars()
                    .any(|character| "█┌┐└┘─│├┤┬┴┼".contains(character))
        })
        .map(|(row, _)| row)
        .last()
        .ok_or_else(|| HarnessError::with_screen(format!("{label} has no title"), &text))?;
    let animation_rows = lines
        .iter()
        .enumerate()
        .skip(title_last_row + 1)
        .take(open_row.saturating_sub(title_last_row + 1))
        .filter(|(_, line)| {
            line.chars()
                .any(|character| ".:-=+*#%@".contains(character))
        })
        .map(|(row, _)| row)
        .collect::<Vec<_>>();
    let Some(first_animation_row) = animation_rows.first().copied() else {
        return Err(HarnessError::with_screen(
            format!("{label} has no visible animation"),
            &text,
        ));
    };
    let last_animation_row = animation_rows
        .last()
        .copied()
        .unwrap_or(first_animation_row);
    let blank_rows_below_title = first_animation_row.saturating_sub(title_last_row + 1);
    let rows_before_open_label = open_row.saturating_sub(last_animation_row + 1);
    if blank_rows_below_title < 1 || rows_before_open_label < 2 {
        return Err(HarnessError::with_screen(
            format!(
                "{label} clips the animation into its chrome: {blank_rows_below_title} blank row(s) below the title and {rows_before_open_label} row(s) before the Open label"
            ),
            &text,
        ));
    }
    Ok(())
}

fn require_no_intro_animation_below_open(session: &PtySession, label: &str) -> HarnessResult<()> {
    let text = session.screen_text();
    let lines = text.lines().collect::<Vec<_>>();
    let open_row = lines
        .iter()
        .position(|line| line.contains("Open"))
        .ok_or_else(|| HarnessError::with_screen(format!("{label} has no Open button"), &text))?;
    if lines[open_row..].iter().any(|line| {
        line.chars()
            .any(|character| ".:-=+*#%@".contains(character))
    }) {
        return Err(HarnessError::with_screen(
            format!("{label} left animation cells at or below the Open button"),
            &text,
        ));
    }
    Ok(())
}

fn require_no_blank_presentations(nonblank_cells: &[usize], label: &str) -> HarnessResult<()> {
    let minimum = nonblank_cells.iter().copied().min().unwrap_or_default();
    if nonblank_cells.is_empty() || minimum == 0 {
        return Err(HarnessError::new(format!(
            "{label} exposed a fully blank intermediate presentation; nonblank cells by presentation: {nonblank_cells:?}"
        )));
    }
    Ok(())
}

fn scenario_help_keyboard_preview(session: &mut PtySession) -> HarnessResult<(String, String)> {
    session.wait_for_home()?;
    open_help(session)?;
    session.settle(INPUT_SETTLE)?;
    session.send(b" ")?;
    session.require_screen("keyboard-pinned Help preview", HOVER_SETTLE, |_, text| {
        text.contains("topic preview:") && text.contains("[x]")
    })?;
    let screen = session.screen_text();
    session.send(b"\x1b")?;
    session.require_screen("Help preview dismissal", HOVER_SETTLE, |_, text| {
        !text.contains("topic preview:")
    })?;
    Ok((
        screen,
        "Space pinned the selected topic preview and Esc dismissed it".to_string(),
    ))
}

fn scenario_help_mouse_move(session: &mut PtySession) -> HarnessResult<(String, String)> {
    session.wait_for_home()?;
    open_help(session)?;
    let (column, row) = session.find_label_cell("Welcome", true).unwrap_or((7, 8));
    session.send_native_mouse_move(session.size.cols.saturating_sub(2), session.size.rows)?;
    session.settle(Duration::from_millis(120))?;
    session.send_native_mouse_move(column, row)?;
    session.settle(Duration::from_millis(120))?;
    session.send_native_mouse_move(column.saturating_add(1), row)?;
    session.settle(Duration::from_millis(120))?;
    session.send_native_mouse_move(column, row)?;
    session.require_screen(
        &format!("real native mouse-move Help preview at {column},{row}"),
        Duration::from_millis(1_500),
        |_, text| text.contains("topic preview:") && text.contains("[x]"),
    )?;
    let screen = session.screen_text();

    let outside_column = session.size.cols;
    let outside_row = session.size.rows;
    session.send_native_mouse_move(outside_column, outside_row)?;
    session.require_screen(
        "mouse-away Help preview dismissal",
        HOVER_SETTLE,
        |_, text| !text.contains("topic preview:"),
    )?;
    Ok((
        screen,
        format!(
            "native Windows no-button mouse move opened the preview at {column},{row} and moving away dismissed it"
        ),
    ))
}

fn scenario_guide_off(session: &mut PtySession) -> HarnessResult<(String, String)> {
    session.wait_for_home()?;
    session.send(b"g")?;
    session.require_screen(
        "Guide provider-off panel",
        Duration::from_secs(2),
        |_, text| {
            text.contains("Guide: off")
                && text.contains("The AI Guide is off. Petri remains fully usable.")
                && text.contains("Ask a question_")
        },
    )?;
    session.send(b"status\r")?;
    session.require_screen(
        "provider-off Guide response",
        Duration::from_secs(2),
        |_, text| {
            text.contains("Guide: off")
                && text.contains("The AI Guide is off. Petri remains fully usable.")
                && !text.contains("Guide thinking")
        },
    )?;
    let screen = session.screen_text();
    session.send(b"\x1b")?;
    Ok((
        screen,
        "Guide opened and remained responsive without probing or spawning a provider".to_string(),
    ))
}

fn scenario_help_resize(session: &mut PtySession) -> HarnessResult<(String, String)> {
    session.wait_for_home()?;
    open_help(session)?;
    let mut checkpoints = Vec::new();
    for size in [
        TerminalSize::new(60, 20),
        TerminalSize::new(80, 24),
        TerminalSize::new(120, 32),
    ] {
        session.resize(size)?;
        session.require_screen(
            &format!("Help redraw at {size}"),
            Duration::from_secs(2),
            |_, text| {
                text.contains("GitBook topics")
                    && text.contains("guide: Welcome")
                    && text.contains("lines hidden")
            },
        )?;
        if size == TerminalSize::new(60, 20) {
            require_hidden_lines_footer(session, "short Help hidden-lines footer")?;
            session.send(b"\t")?;
            session.send(b"\x1b[6~")?;
            session.settle(INPUT_SETTLE)?;
            require_hidden_lines_footer(session, "scrolled Help hidden-lines footer")?;
        }
        checkpoints.push(size.to_string());
    }
    Ok((
        session.screen_text(),
        format!(
            "Help redrew after native ConPTY resize events at {}",
            checkpoints.join(", ")
        ),
    ))
}

fn require_hidden_lines_footer(session: &mut PtySession, label: &str) -> HarnessResult<()> {
    session.require_screen(label, Duration::from_secs(2), |_, text| {
        footer_has_hidden_lines(text)
    })
}

fn footer_has_hidden_lines(text: &str) -> bool {
    let mut footer = text.lines().rev().take(4).collect::<Vec<_>>();
    footer.reverse();
    let hidden_row = footer
        .iter()
        .position(|line| line.contains("lines hidden") && line.contains("PgUp/PgDn scroll"));
    let hotkey_row = footer.iter().position(|line| line.contains("q quit"));
    matches!((hidden_row, hotkey_row), (Some(hidden), Some(hotkeys)) if hidden < hotkeys)
}

fn open_help(session: &mut PtySession) -> HarnessResult<()> {
    for _ in 0..5 {
        session.send(b"\x1b[B")?;
        session.settle(INPUT_SETTLE)?;
    }
    session.send(b"\r")?;
    session.require_screen("bundled Help reader", Duration::from_secs(3), |_, text| {
        text.contains("GitBook topics")
            && text.contains("Welcome to Amoeba Farm")
            && text.contains("bundled guide")
    })
}

fn write_artifact(
    directory: &Path,
    scenario: Scenario,
    size: TerminalSize,
    screen: &str,
) -> HarnessResult<PathBuf> {
    fs::create_dir_all(directory).map_err(|error| {
        HarnessError::new(format!(
            "could not create artifact directory {}: {error}",
            directory.display()
        ))
    })?;
    let path = directory.join(format!("{}-{}.txt", scenario.name(), size));
    fs::write(
        &path,
        format!(
            "terminal_size: {size}\nscenario: {}\n\n{screen}\n",
            scenario.name()
        ),
    )
    .map_err(|error| {
        HarnessError::new(format!(
            "could not write artifact {}: {error}",
            path.display()
        ))
    })?;
    Ok(path)
}

struct LaunchContext {
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
}

impl LaunchContext {
    fn prepare(sandbox: &Sandbox, stub_url: String, scenario: Scenario) -> HarnessResult<Self> {
        let config_dir = sandbox.path.join("config");
        let appdata = sandbox.path.join("appdata");
        let local_appdata = sandbox.path.join("local-appdata");
        let temp = sandbox.path.join("temp");
        for directory in [&config_dir, &appdata, &local_appdata, &temp] {
            fs::create_dir_all(directory)?;
        }
        let solana_config = config_dir.join("solana.yml");
        let missing_keypair = config_dir.join("missing-wallet.json");
        fs::write(
            &solana_config,
            format!(
                "json_rpc_url: {stub_url}\nkeypair_path: {}\ncommitment: confirmed\n",
                missing_keypair.display()
            ),
        )?;

        let mut child_env = Vec::new();
        for key in [
            "SystemRoot",
            "WINDIR",
            "ComSpec",
            "PATH",
            "PATHEXT",
            "NUMBER_OF_PROCESSORS",
            "PROCESSOR_ARCHITECTURE",
        ] {
            if let Some(value) = env::var_os(key) {
                child_env.push((OsString::from(key), value));
            }
        }
        child_env.extend([
            (OsString::from("HOME"), sandbox.path.as_os_str().to_owned()),
            (
                OsString::from("USERPROFILE"),
                sandbox.path.as_os_str().to_owned(),
            ),
            (OsString::from("APPDATA"), appdata.as_os_str().to_owned()),
            (
                OsString::from("LOCALAPPDATA"),
                local_appdata.as_os_str().to_owned(),
            ),
            (
                OsString::from("XDG_CONFIG_HOME"),
                config_dir.as_os_str().to_owned(),
            ),
            (OsString::from("TEMP"), temp.as_os_str().to_owned()),
            (OsString::from("TMP"), temp.as_os_str().to_owned()),
            (OsString::from("TERM"), OsString::from("xterm-256color")),
            (OsString::from("COLORTERM"), OsString::from("truecolor")),
            (OsString::from("PETRI_API_URL"), OsString::from(&stub_url)),
            (OsString::from("PETRI_RPC_URL"), OsString::from(&stub_url)),
            (OsString::from("AMEBA_DOCS_URL"), OsString::from(&stub_url)),
            (
                OsString::from("PETRI_BACKEND_TIMEOUT_MS"),
                OsString::from("500"),
            ),
            (
                OsString::from("PETRI_CONFIG_PATH"),
                config_dir.join("petri.toml").into_os_string(),
            ),
            (
                OsString::from("AMEBA_WALLET_TERMS_CACHE"),
                config_dir.join("wallet-terms.json").into_os_string(),
            ),
            (
                OsString::from("PETRI_GUIDE_PROVIDER"),
                OsString::from("off"),
            ),
            (
                OsString::from("PETRI_TUI_REDUCED_MOTION"),
                OsString::from(scenario.reduced_motion_env_value()),
            ),
            (OsString::from("PETRI_UPDATE_CHECK"), OsString::from("0")),
            (
                OsString::from("NO_PROXY"),
                OsString::from("127.0.0.1,localhost"),
            ),
        ]);

        let args = vec![
            OsString::from("--output"),
            OsString::from("plain"),
            OsString::from("--api"),
            OsString::from(&stub_url),
            OsString::from("--solana-config"),
            solana_config.into_os_string(),
            OsString::from("--keypair"),
            missing_keypair.into_os_string(),
            OsString::from("tui"),
            OsString::from("--no-update-check"),
        ];
        Ok(Self {
            args,
            env: child_env,
        })
    }
}

struct Sandbox {
    base: PathBuf,
    path: PathBuf,
}

impl Sandbox {
    fn create() -> HarnessResult<Self> {
        let base = env::temp_dir().join("petri-conpty-harness");
        fs::create_dir_all(&base)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = base.join(format!("{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { base, path })
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if self.path.parent() == Some(self.base.as_path()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct LoopbackStub {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LoopbackStub {
    fn start() -> HarnessResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => respond_not_found(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for LoopbackStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn respond_not_found(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let mut request = [0_u8; 2048];
    let _ = stream.read(&mut request);
    let _ = stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.flush();
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

#[derive(Clone)]
struct HostCallbacks {
    writer: Weak<Mutex<Box<dyn Write + Send>>>,
    cursor_requests: usize,
    cursor_reply_error: Option<String>,
}

impl Callbacks for HostCallbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut Screen,
        intermediate_one: Option<u8>,
        intermediate_two: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        let cursor_report_requested = intermediate_one.is_none()
            && intermediate_two.is_none()
            && command == 'n'
            && params.len() == 1
            && params[0] == [6];
        if !cursor_report_requested {
            return;
        }
        self.cursor_requests += 1;
        let Some(writer) = self.writer.upgrade() else {
            self.cursor_reply_error = Some("input writer was unavailable".to_string());
            return;
        };
        let (row, column) = screen.cursor_position();
        match writer.lock() {
            Ok(mut writer) => {
                if let Err(error) =
                    write!(writer, "\x1b[{};{}R", row + 1, column + 1).and_then(|_| writer.flush())
                {
                    self.cursor_reply_error = Some(error.to_string());
                }
            }
            Err(_) => {
                self.cursor_reply_error = Some("input writer lock was poisoned".to_string());
            }
        }
    }
}

enum ReadEvent {
    Data {
        bytes: Vec<u8>,
        received_at: Instant,
    },
    Error(String),
    Eof,
}

#[cfg(windows)]
struct ConsoleInputAttachment {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ConsoleInputAttachment {
    fn attach(process_id: u32) -> HarnessResult<Self> {
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
            System::Console::{AttachConsole, FreeConsole},
        };

        unsafe {
            // The scenario worker is disposable and has piped stdout/stderr,
            // so detaching it does not affect the suite's captured output.
            FreeConsole();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let attached = unsafe { AttachConsole(process_id) };
            if attached != 0 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(HarnessError::new(format!(
                    "could not attach to Petri pseudoconsole input: {}",
                    std::io::Error::last_os_error()
                )));
            }
            thread::sleep(Duration::from_millis(20));
        }

        let name = "CONIN$\0".encode_utf16().collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            unsafe {
                FreeConsole();
            }
            return Err(HarnessError::new(format!(
                "could not open Petri CONIN$: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(Self { handle })
    }

    fn mouse_move(&self, column: u16, row: u16) -> HarnessResult<()> {
        use windows_sys::Win32::System::Console::{
            COORD, INPUT_RECORD, INPUT_RECORD_0, MOUSE_EVENT, MOUSE_EVENT_RECORD, MOUSE_MOVED,
            WriteConsoleInputW,
        };

        let record = INPUT_RECORD {
            EventType: MOUSE_EVENT as u16,
            Event: INPUT_RECORD_0 {
                MouseEvent: MOUSE_EVENT_RECORD {
                    dwMousePosition: COORD {
                        X: column.saturating_sub(1) as i16,
                        Y: row.saturating_sub(1) as i16,
                    },
                    dwButtonState: 0,
                    dwControlKeyState: 0,
                    dwEventFlags: MOUSE_MOVED,
                },
            },
        };
        let mut written = 0_u32;
        let result = unsafe { WriteConsoleInputW(self.handle, &record, 1, &mut written) };
        if result == 0 || written != 1 {
            return Err(HarnessError::new(format!(
                "could not write native mouse record to Petri: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ConsoleInputAttachment {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
            windows_sys::Win32::System::Console::FreeConsole();
        }
    }
}

struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn Child + Send + Sync>,
    writer: Option<SharedWriter>,
    rx: Receiver<ReadEvent>,
    parser: Parser<HostCallbacks>,
    raw_output: Vec<u8>,
    presentation_times: Vec<Instant>,
    presentation_scan_from: usize,
    synchronized_scan_from: usize,
    synchronized_update_active: bool,
    observe_presented_nonblank_cells: bool,
    presented_nonblank_cells: Vec<usize>,
    output_limit: usize,
    hard_deadline: Instant,
    exit_status: Option<ExitStatus>,
    size: TerminalSize,
    #[cfg(windows)]
    console_input: Option<ConsoleInputAttachment>,
}

impl PtySession {
    fn spawn(
        binary: &Path,
        repo_dir: &Path,
        launch: &LaunchContext,
        size: TerminalSize,
        timeout: Duration,
        output_limit: usize,
    ) -> HarnessResult<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(size.as_pty()).map_err(|error| {
            HarnessError::new(format!("could not create native ConPTY session: {error}"))
        })?;
        let reader = pair.master.try_clone_reader().map_err(|error| {
            HarnessError::new(format!("could not clone ConPTY output reader: {error}"))
        })?;
        let writer = pair.master.take_writer().map_err(|error| {
            HarnessError::new(format!("could not take ConPTY input writer: {error}"))
        })?;
        let writer = Arc::new(Mutex::new(writer));

        let (tx, rx) = mpsc::sync_channel(READ_CHANNEL_DEPTH);
        spawn_reader(reader, tx);
        let callbacks = HostCallbacks {
            writer: Arc::downgrade(&writer),
            cursor_requests: 0,
            cursor_reply_error: None,
        };
        let parser = Parser::new_with_callbacks(size.rows, size.cols, 0, callbacks);

        // portable-pty 0.9 enables PSEUDOCONSOLE_INHERIT_CURSOR while
        // CreatePseudoConsole is opening. Reply before the first child request
        // so older Windows builds cannot leave the child waiting on ESC[6n.
        {
            let mut input = writer
                .lock()
                .map_err(|_| HarnessError::new("ConPTY input writer lock was poisoned"))?;
            input
                .write_all(b"\x1b[1;1R")
                .and_then(|_| input.flush())
                .map_err(|error| {
                    HarnessError::new(format!(
                        "could not complete initial ConPTY cursor handshake: {error}"
                    ))
                })?;
        }

        let mut command = CommandBuilder::new(binary.as_os_str());
        command.args(&launch.args);
        command.cwd(repo_dir.as_os_str());
        command.env_clear();
        for (key, value) in &launch.env {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command).map_err(|error| {
            HarnessError::new(format!("could not launch Petri in ConPTY: {error}"))
        })?;
        drop(pair.slave);
        Ok(Self {
            master: Some(pair.master),
            child,
            writer: Some(writer),
            rx,
            parser,
            raw_output: Vec::new(),
            presentation_times: Vec::new(),
            presentation_scan_from: 0,
            synchronized_scan_from: 0,
            synchronized_update_active: false,
            observe_presented_nonblank_cells: false,
            presented_nonblank_cells: Vec::new(),
            output_limit: output_limit.max(64 * 1024),
            hard_deadline: Instant::now() + timeout,
            exit_status: None,
            size,
            #[cfg(windows)]
            console_input: None,
        })
    }

    fn screen_text(&self) -> String {
        normalize_screen(&self.parser.screen().contents())
    }

    fn wait_for_home(&mut self) -> HarnessResult<()> {
        self.wait_for_intro()?;
        self.enter_home()
    }

    fn wait_for_intro(&mut self) -> HarnessResult<()> {
        self.require_screen(
            "Petri ASCII startup intro",
            Duration::from_secs(4),
            |_, text| {
                text.contains("Open")
                    && !text.contains("Trade options")
                    && text
                        .chars()
                        .filter(|character| ".:-=+*#%@".contains(*character))
                        .count()
                        >= 8
            },
        )
    }

    fn enter_home(&mut self) -> HarnessResult<()> {
        self.send(b"\r")?;
        self.require_screen(
            "interactive Petri Home",
            Duration::from_secs(4),
            |_, text| {
                text.contains("home") && text.contains("Trade options") && text.contains("q quit")
            },
        )
    }

    fn require_screen(
        &mut self,
        label: &str,
        timeout: Duration,
        predicate: impl Fn(&Screen, &str) -> bool,
    ) -> HarnessResult<()> {
        if self.wait_screen(timeout, predicate)? {
            return Ok(());
        }
        Err(HarnessError::with_screen(
            format!(
                "timed out waiting for {label} ({})",
                self.diagnostic_state()
            ),
            &self.parser.screen().contents(),
        ))
    }

    fn wait_screen(
        &mut self,
        timeout: Duration,
        predicate: impl Fn(&Screen, &str) -> bool,
    ) -> HarnessResult<bool> {
        let deadline = (Instant::now() + timeout).min(self.hard_deadline);
        loop {
            let text = self.screen_text();
            if predicate(self.parser.screen(), &text) {
                return Ok(true);
            }
            if let Some(status) = &self.exit_status {
                return Err(HarnessError::with_screen(
                    format!("Petri exited before the expected screen: {status:?}"),
                    &text,
                ));
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            self.pump(deadline)?;
        }
    }

    fn settle(&mut self, duration: Duration) -> HarnessResult<()> {
        let deadline = (Instant::now() + duration).min(self.hard_deadline);
        while Instant::now() < deadline {
            self.pump(deadline)?;
            if self.exit_status.is_some() {
                return Err(HarnessError::with_screen(
                    "Petri exited while settling terminal input",
                    &self.parser.screen().contents(),
                ));
            }
        }
        Ok(())
    }

    fn wait_for_synchronized_presentation(
        &mut self,
        previous_presentations: usize,
        label: &str,
        timeout: Duration,
    ) -> HarnessResult<()> {
        let deadline = (Instant::now() + timeout).min(self.hard_deadline);
        while Instant::now() < deadline {
            if self.presentation_times.len() > previous_presentations
                && !self.synchronized_update_active
            {
                return Ok(());
            }
            self.pump(deadline)?;
            if self.exit_status.is_some() {
                break;
            }
        }
        Err(HarnessError::with_screen(
            format!(
                "timed out waiting for {label} synchronized presentation ({} total presentations, active={})",
                self.presentation_times.len(),
                self.synchronized_update_active
            ),
            &self.parser.screen().contents(),
        ))
    }

    fn pump(&mut self, deadline: Instant) -> HarnessResult<()> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.check_child()?;
            return Ok(());
        }
        match self.rx.recv_timeout(PUMP_SLICE.min(remaining)) {
            Ok(event) => self.apply_read_event(event)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {}
        }
        while let Ok(event) = self.rx.try_recv() {
            self.apply_read_event(event)?;
        }
        self.check_child()
    }

    fn apply_read_event(&mut self, event: ReadEvent) -> HarnessResult<()> {
        match event {
            ReadEvent::Data { bytes, received_at } => {
                if self.raw_output.len().saturating_add(bytes.len()) > self.output_limit {
                    return Err(HarnessError::new(format!(
                        "Petri exceeded the {} byte ConPTY output limit",
                        self.output_limit
                    )));
                }
                self.parser.process(&bytes);
                self.raw_output.extend_from_slice(&bytes);
                let presentations_before_read = self.presentation_times.len();
                record_sequence_times(
                    &self.raw_output,
                    SYNCHRONIZED_UPDATE_END,
                    &mut self.presentation_scan_from,
                    &mut self.presentation_times,
                    received_at,
                );
                record_synchronized_update_state(
                    &self.raw_output,
                    &mut self.synchronized_scan_from,
                    &mut self.synchronized_update_active,
                );
                if self.observe_presented_nonblank_cells
                    && self.presentation_times.len() > presentations_before_read
                    && !self.synchronized_update_active
                {
                    self.presented_nonblank_cells.push(
                        self.parser
                            .screen()
                            .contents()
                            .chars()
                            .filter(|character| !character.is_whitespace())
                            .count(),
                    );
                }
            }
            ReadEvent::Error(error) => {
                return Err(HarnessError::new(format!(
                    "ConPTY output reader failed: {error}"
                )));
            }
            ReadEvent::Eof => {}
        }
        Ok(())
    }

    fn finish_presented_nonblank_observation(&mut self) {
        self.observe_presented_nonblank_cells = false;
        let nonblank_cells = self
            .parser
            .screen()
            .contents()
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        self.presented_nonblank_cells.push(nonblank_cells);
    }

    fn check_child(&mut self) -> HarnessResult<()> {
        if self.exit_status.is_none() {
            self.exit_status = self.child.try_wait().map_err(|error| {
                HarnessError::new(format!("could not poll Petri process: {error}"))
            })?;
        }
        Ok(())
    }

    fn send(&mut self, bytes: &[u8]) -> HarnessResult<()> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| HarnessError::new("ConPTY input is already closed"))?;
        let mut writer = writer
            .lock()
            .map_err(|_| HarnessError::new("ConPTY input writer lock was poisoned"))?;
        writer
            .write_all(bytes)
            .map_err(|error| HarnessError::new(format!("could not send ConPTY input: {error}")))?;
        writer
            .flush()
            .map_err(|error| HarnessError::new(format!("could not flush ConPTY input: {error}")))
    }

    fn resize(&mut self, size: TerminalSize) -> HarnessResult<()> {
        self.master
            .as_ref()
            .ok_or_else(|| HarnessError::new("ConPTY master is already closed"))?
            .resize(size.as_pty())
            .map_err(|error| HarnessError::new(format!("could not resize ConPTY: {error}")))?;
        self.parser.screen_mut().set_size(size.rows, size.cols);
        self.size = size;
        Ok(())
    }

    #[cfg(windows)]
    fn send_native_mouse_move(&mut self, column: u16, row: u16) -> HarnessResult<()> {
        if self.console_input.is_none() {
            let process_id = self.child.process_id().ok_or_else(|| {
                HarnessError::new("Petri process id was unavailable for native mouse input")
            })?;
            self.console_input = Some(ConsoleInputAttachment::attach(process_id)?);
        }
        self.console_input
            .as_ref()
            .expect("console input attachment")
            .mouse_move(column, row)
    }

    #[cfg(not(windows))]
    fn send_native_mouse_move(&mut self, _column: u16, _row: u16) -> HarnessResult<()> {
        Err(HarnessError::new("native mouse records require Windows"))
    }

    fn find_label_cell(&self, label: &str, left_half_only: bool) -> Option<(u16, u16)> {
        let (_, cols) = self.parser.screen().size();
        self.parser
            .screen()
            .rows(0, cols)
            .enumerate()
            .find_map(|(row, line)| {
                let byte_column = line.find(label)?;
                let column = line[..byte_column].chars().count() as u16;
                if left_half_only && column >= cols / 3 {
                    return None;
                }
                Some((column.saturating_add(2), row as u16 + 1))
            })
    }

    fn assert_no_terminal_failure(&self) -> HarnessResult<()> {
        let raw = String::from_utf8_lossy(&self.raw_output).to_ascii_lowercase();
        for marker in [
            "panicked at",
            "fatal runtime error",
            "failed to enter raw mode",
            "failed to enter alternate screen",
        ] {
            if raw.contains(marker) {
                return Err(HarnessError::with_screen(
                    format!("Petri emitted terminal failure marker {marker:?}"),
                    &self.parser.screen().contents(),
                ));
            }
        }
        Ok(())
    }

    fn diagnostic_state(&self) -> String {
        let screen = self.parser.screen();
        let tail_start = self.raw_output.len().saturating_sub(256);
        let raw_tail = String::from_utf8_lossy(&self.raw_output[tail_start..]);
        let raw_text = String::from_utf8_lossy(&self.raw_output);
        let callbacks = self.parser.callbacks();
        format!(
            "raw_bytes={}, alternate_screen={}, mouse={:?}/{:?}, raw_modes={{alt:{}, any_mouse:{}, sgr_mouse:{}, win32_input:{}}}, child_status={:?}, cursor_requests={}, cursor_reply_error={:?}, raw_tail={:?}",
            self.raw_output.len(),
            screen.alternate_screen(),
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
            raw_text.contains("\x1b[?1049h"),
            raw_text.contains("\x1b[?1003h"),
            raw_text.contains("\x1b[?1006h"),
            raw_text.contains("\x1b[?9001h"),
            self.exit_status,
            callbacks.cursor_requests,
            callbacks.cursor_reply_error,
            raw_tail
        )
    }

    fn quit(&mut self, timeout: Duration) -> HarnessResult<()> {
        let cleanup_output_start = self.raw_output.len();
        self.send(b"q")?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump(deadline)?;
            if let Some(status) = &self.exit_status {
                if status.success() {
                    return self
                        .wait_for_clean_terminal_exit(cleanup_output_start, CLEAN_EXIT_TIMEOUT);
                }
                return Err(HarnessError::new(format!(
                    "Petri exited with nonzero status {status:?}"
                )));
            }
        }

        let _ = self.child.kill();
        let kill_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < kill_deadline {
            self.pump(kill_deadline)?;
            if self.exit_status.is_some() {
                break;
            }
        }
        Err(HarnessError::with_screen(
            "Petri did not exit after q before the shutdown deadline",
            &self.parser.screen().contents(),
        ))
    }

    fn wait_for_clean_terminal_exit(
        &mut self,
        cleanup_output_start: usize,
        timeout: Duration,
    ) -> HarnessResult<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.clean_terminal_exit_observed(cleanup_output_start) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(PUMP_SLICE.min(remaining)) {
                Ok(event) => self.apply_read_event(event)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(event) = self.rx.try_recv() {
                self.apply_read_event(event)?;
            }
        }
        self.assert_clean_terminal_exit(cleanup_output_start)
    }

    fn clean_terminal_exit_observed(&self, cleanup_output_start: usize) -> bool {
        let cleanup_output = self
            .raw_output
            .get(cleanup_output_start..)
            .unwrap_or_default();
        let raw = String::from_utf8_lossy(cleanup_output);
        raw.contains("\x1b[?25h")
            && raw.contains("\x1b]22;\x1b\\")
            && raw.contains("\x1b[?1004l")
            && !self.parser.screen().alternate_screen()
            && !self.parser.screen().hide_cursor()
            && self.screen_text().is_empty()
    }

    fn assert_clean_terminal_exit(&self, cleanup_output_start: usize) -> HarnessResult<()> {
        if self.clean_terminal_exit_observed(cleanup_output_start) {
            return Ok(());
        }

        Err(HarnessError::with_screen(
            format!(
                "Petri exited after q without fully restoring the terminal ({})",
                self.diagnostic_state()
            ),
            &self.parser.screen().contents(),
        ))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if self.exit_status.is_none() {
            let _ = self.child.kill();
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                if self.child.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.writer.take();
        if let Some(master) = self.master.take() {
            // ClosePseudoConsole may block on older Windows builds. Keep the
            // reader draining and isolate final close from the scenario worker.
            thread::spawn(move || drop(master));
        }
    }
}

fn spawn_reader(mut reader: Box<dyn Read + Send>, tx: SyncSender<ReadEvent>) {
    thread::spawn(move || {
        let mut receiver_connected = true;
        loop {
            let mut buffer = vec![0_u8; READ_CHUNK_SIZE];
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if receiver_connected {
                        let _ = tx.send(ReadEvent::Eof);
                    }
                    break;
                }
                Ok(read) => {
                    buffer.truncate(read);
                    if receiver_connected
                        && tx
                            .send(ReadEvent::Data {
                                bytes: buffer,
                                received_at: Instant::now(),
                            })
                            .is_err()
                    {
                        receiver_connected = false;
                    }
                }
                Err(error) => {
                    if receiver_connected {
                        let _ = tx.send(ReadEvent::Error(error.to_string()));
                    }
                    break;
                }
            }
        }
    });
}

fn record_sequence_times(
    haystack: &[u8],
    needle: &[u8],
    scan_from: &mut usize,
    times: &mut Vec<Instant>,
    received_at: Instant,
) {
    if needle.is_empty() {
        return;
    }
    while *scan_from + needle.len() <= haystack.len() {
        let Some(relative) = haystack[*scan_from..]
            .windows(needle.len())
            .position(|candidate| candidate == needle)
        else {
            break;
        };
        *scan_from += relative + needle.len();
        times.push(received_at);
    }
    *scan_from = (*scan_from).max(haystack.len().saturating_sub(needle.len() - 1));
}

fn record_synchronized_update_state(
    haystack: &[u8],
    scan_from: &mut usize,
    synchronized: &mut bool,
) {
    let overlap = SYNCHRONIZED_UPDATE_BEGIN
        .len()
        .max(SYNCHRONIZED_UPDATE_END.len())
        .saturating_sub(1);
    loop {
        let begin = haystack[*scan_from..]
            .windows(SYNCHRONIZED_UPDATE_BEGIN.len())
            .position(|candidate| candidate == SYNCHRONIZED_UPDATE_BEGIN)
            .map(|relative| *scan_from + relative);
        let end = haystack[*scan_from..]
            .windows(SYNCHRONIZED_UPDATE_END.len())
            .position(|candidate| candidate == SYNCHRONIZED_UPDATE_END)
            .map(|relative| *scan_from + relative);
        match (begin, end) {
            (Some(begin), Some(end)) if begin < end => {
                *synchronized = true;
                *scan_from = begin + SYNCHRONIZED_UPDATE_BEGIN.len();
            }
            (_, Some(end)) => {
                *synchronized = false;
                *scan_from = end + SYNCHRONIZED_UPDATE_END.len();
            }
            (Some(begin), None) => {
                *synchronized = true;
                *scan_from = begin + SYNCHRONIZED_UPDATE_BEGIN.len();
            }
            (None, None) => break,
        }
    }
    *scan_from = (*scan_from).max(haystack.len().saturating_sub(overlap));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_uses_columns_by_rows() {
        assert_eq!(
            "80x24".parse::<TerminalSize>().expect("80x24"),
            TerminalSize { cols: 80, rows: 24 }
        );
        assert_eq!(
            "120X32".parse::<TerminalSize>().expect("120X32"),
            TerminalSize {
                cols: 120,
                rows: 32,
            }
        );
    }

    #[test]
    fn terminal_size_rejects_zero_and_malformed_values() {
        for value in ["", "80", "0x24", "80x0", "wide x tall", "80x24x1"] {
            assert!(value.parse::<TerminalSize>().is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn screen_normalization_removes_only_layout_noise() {
        let input = "Amoeba Farm   \nHelp      \n          \n";
        assert_eq!(normalize_screen(input), "Amoeba Farm\nHelp");
    }

    #[test]
    fn sgr_mouse_move_is_one_based_and_has_no_button_pressed() {
        assert_eq!(sgr_mouse_move(7, 8), b"\x1b[<35;7;8M");
    }

    #[test]
    fn sgr_mouse_click_uses_press_and_release_suffixes() {
        assert_eq!(sgr_mouse_button(60, 10, true), b"\x1b[<0;60;10M");
        assert_eq!(sgr_mouse_button(60, 10, false), b"\x1b[<0;60;10m");
    }

    #[test]
    fn scenario_names_are_stable() {
        for scenario in Scenario::ALL {
            assert_eq!(scenario.name().parse::<Scenario>(), Ok(scenario));
        }
    }

    #[test]
    fn startup_scenario_keeps_animation_enabled() {
        assert_eq!(Scenario::Startup.reduced_motion_env_value(), "0");
        for scenario in Scenario::ALL
            .into_iter()
            .filter(|scenario| *scenario != Scenario::Startup)
        {
            assert_eq!(scenario.reduced_motion_env_value(), "1");
        }
    }

    #[test]
    fn synchronized_frame_counter_handles_split_reader_chunks() {
        let received_at = Instant::now();
        let mut output = b"prefix\x1b[?202".to_vec();
        let mut scan_from = 0;
        let mut times = Vec::new();
        record_sequence_times(
            &output,
            SYNCHRONIZED_UPDATE_END,
            &mut scan_from,
            &mut times,
            received_at,
        );
        assert!(times.is_empty());

        output.extend_from_slice(b"6lsuffix\x1b[?2026l");
        record_sequence_times(
            &output,
            SYNCHRONIZED_UPDATE_END,
            &mut scan_from,
            &mut times,
            received_at,
        );
        assert_eq!(times, [received_at, received_at]);
    }

    #[test]
    fn synchronized_update_tracker_handles_split_begin_and_end_sequences() {
        let mut output = b"prefix\x1b[?202".to_vec();
        let mut scan_from = 0;
        let mut synchronized = false;
        record_synchronized_update_state(&output, &mut scan_from, &mut synchronized);
        assert!(!synchronized);

        output.extend_from_slice(b"6hframe\x1b[?20");
        record_synchronized_update_state(&output, &mut scan_from, &mut synchronized);
        assert!(synchronized);

        output.extend_from_slice(b"26lsuffix");
        record_synchronized_update_state(&output, &mut scan_from, &mut synchronized);
        assert!(!synchronized);
    }

    #[test]
    fn intro_output_rate_uses_the_measured_sample_duration() {
        assert_eq!(
            intro_output_bytes_per_second(72_000, Duration::from_millis(800)),
            90_000
        );
        assert_eq!(intro_output_bytes_per_second(0, Duration::ZERO), 0);
    }

    #[test]
    fn hidden_lines_notice_must_be_in_the_visible_footer_band() {
        assert!(footer_has_hidden_lines(
            "body\nbody\n↕ 9 lines hidden | PgUp/PgDn scroll\nq quit\ng ask Guide"
        ));
        assert!(!footer_has_hidden_lines(
            "↕ 9 lines hidden | PgUp/PgDn scroll\nbody\nbody\nbody\nq quit\ng ask Guide"
        ));
        assert!(!footer_has_hidden_lines(
            "body\n↓ 9 missing lines below | PgUp/PgDn scroll\nq quit\ng ask Guide"
        ));
        assert!(!footer_has_hidden_lines(
            "body\nq quit | arrows move\n↕ 9 lines hidden | PgUp/PgDn scroll\ng ask Guide"
        ));
    }
}
