//! Packed full-screen startup animation and input intent.

use super::*;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_LEFT: usize = 15;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_RIGHT: usize = 226;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_TOP: usize = 8;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_BOTTOM: usize = 75;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_COLS: usize =
    STARTUP_INTRO_SOURCE_VIEW_RIGHT - STARTUP_INTRO_SOURCE_VIEW_LEFT;
pub(super) const STARTUP_INTRO_SOURCE_VIEW_ROWS: usize =
    STARTUP_INTRO_SOURCE_VIEW_BOTTOM - STARTUP_INTRO_SOURCE_VIEW_TOP;
pub(super) const STARTUP_INTRO_FRAME_COUNT: usize = 207;
pub(super) const STARTUP_INTRO_FRAME_INTERVAL: Duration = Duration::from_millis(30);
pub(super) const STARTUP_INTRO_TERMINAL_FRAME_CELLS: usize =
    STARTUP_INTRO_SOURCE_VIEW_COLS * STARTUP_INTRO_SOURCE_VIEW_ROWS;
pub(super) const STARTUP_INTRO_TERMINAL_BRIGHT_MASK: u8 = 0x80;
pub(super) const STARTUP_INTRO_TERMINAL_SYMBOL_MASK: u8 = 0x0f;
pub(super) static STARTUP_INTRO_TERMINAL_FRAMES: std::sync::LazyLock<Box<[u8]>> =
    std::sync::LazyLock::new(|| {
        let bytes = decode_intro_asset(
            include_bytes!("../../assets/tui/intro/terminal-frames.bin.zlib"),
            STARTUP_INTRO_TERMINAL_FRAME_CELLS * STARTUP_INTRO_FRAME_COUNT,
            "2feff30af371f34989edfb7a18114840bf498d42a43cc8beca66a8c0058b472f",
        )
        .expect("embedded terminal animation must match its authored bytes");
        assert!(bytes.iter().all(|code| code & 0x7f <= 6));
        bytes
    });

// One zlib stream, exact expanded length, no trailing bytes. Decode once before
// the playback clock starts; rendering continues to consume ordinary cell bytes.
fn decode_intro_asset(
    packed: &[u8],
    expected_bytes: usize,
    expected_sha256: &str,
) -> Result<Box<[u8]>, &'static str> {
    let mut decoder = flate2::Decompress::new(true);
    let mut bytes = vec![0; expected_bytes + 1];
    let status = decoder
        .decompress(packed, &mut bytes, flate2::FlushDecompress::Finish)
        .map_err(|_| "invalid intro compression")?;
    if status != flate2::Status::StreamEnd
        || decoder.total_out() != expected_bytes as u64
        || decoder.total_in() != packed.len() as u64
    {
        return Err("invalid intro length or trailing data");
    }
    bytes.truncate(expected_bytes);
    let digest = crate::content_hash::sha256_hex(&bytes);
    if digest != expected_sha256 {
        return Err("intro content digest mismatch");
    }
    Ok(bytes.into_boxed_slice())
}
static STARTUP_INTRO_TERMINAL_MOTION_MASK: std::sync::OnceLock<Box<[bool]>> =
    std::sync::OnceLock::new();
static STARTUP_INTRO_TERMINAL_BRIGHT_TONE_MASK: std::sync::OnceLock<Box<[bool]>> =
    std::sync::OnceLock::new();

pub(super) fn startup_intro_terminal_motion_mask() -> &'static [bool] {
    STARTUP_INTRO_TERMINAL_MOTION_MASK.get_or_init(|| {
        let mut mask = vec![false; STARTUP_INTRO_TERMINAL_FRAME_CELLS];
        for frame in STARTUP_INTRO_TERMINAL_FRAMES
            .chunks_exact(STARTUP_INTRO_TERMINAL_FRAME_CELLS)
            .take(STARTUP_INTRO_FRAME_COUNT)
        {
            for (active, code) in mask.iter_mut().zip(frame) {
                *active |= startup_intro_terminal_symbol_index(*code) != 0;
            }
        }
        mask.into_boxed_slice()
    })
}

pub(super) fn startup_intro_terminal_bright_tone_mask() -> &'static [bool] {
    STARTUP_INTRO_TERMINAL_BRIGHT_TONE_MASK.get_or_init(|| {
        let mut tone_balance = vec![0_i16; STARTUP_INTRO_TERMINAL_FRAME_CELLS];
        for frame in STARTUP_INTRO_TERMINAL_FRAMES
            .chunks_exact(STARTUP_INTRO_TERMINAL_FRAME_CELLS)
            .take(STARTUP_INTRO_FRAME_COUNT)
        {
            for (balance, code) in tone_balance.iter_mut().zip(frame) {
                if startup_intro_terminal_symbol_index(*code) == 0 {
                    continue;
                }
                *balance += if startup_intro_terminal_is_bright(*code) {
                    1
                } else {
                    -1
                };
            }
        }
        tone_balance
            .into_iter()
            .map(|balance| balance > 0)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StartupIntroKeyAction {
    Open,
    Quit,
    Ignore,
}

pub(super) fn startup_intro_key_action(code: &KeyCode) -> StartupIntroKeyAction {
    match code {
        KeyCode::Enter => StartupIntroKeyAction::Open,
        KeyCode::Char('q' | 'Q') | KeyCode::Esc => StartupIntroKeyAction::Quit,
        _ => StartupIntroKeyAction::Ignore,
    }
}

pub(super) fn startup_intro_frame_index(elapsed: Duration) -> usize {
    if STARTUP_INTRO_FRAME_COUNT == 0 {
        return 0;
    }

    let elapsed_step = elapsed.as_nanos() / STARTUP_INTRO_FRAME_INTERVAL.as_nanos();
    (elapsed_step % STARTUP_INTRO_FRAME_COUNT as u128) as usize
}

pub(super) fn startup_intro_next_frame_timeout(elapsed: Duration) -> Duration {
    let interval_nanos = STARTUP_INTRO_FRAME_INTERVAL.as_nanos();
    let elapsed_in_frame = elapsed.as_nanos() % interval_nanos;
    Duration::from_nanos((interval_nanos - elapsed_in_frame) as u64)
}

pub(super) fn startup_intro_source_indices_for_target(
    target_width: u16,
    target_height: u16,
) -> Vec<Option<usize>> {
    if target_width == 0 || target_height == 0 {
        return Vec::new();
    }

    let mut source_indices =
        Vec::with_capacity(usize::from(target_width).saturating_mul(usize::from(target_height)));
    for row in 0..target_height {
        for column in 0..target_width {
            source_indices.push(
                startup_intro_source_cell_for_target(column, row, target_width, target_height).map(
                    |(source_column, source_row)| {
                        (source_row - STARTUP_INTRO_SOURCE_VIEW_TOP)
                            * STARTUP_INTRO_SOURCE_VIEW_COLS
                            + source_column
                            - STARTUP_INTRO_SOURCE_VIEW_LEFT
                    },
                ),
            );
        }
    }
    source_indices
}

pub(super) fn startup_intro_source_cell_for_target(
    column: u16,
    row: u16,
    target_width: u16,
    target_height: u16,
) -> Option<(usize, usize)> {
    if target_width == 0 || target_height == 0 {
        return None;
    }

    let target_width = f64::from(target_width);
    let target_height = f64::from(target_height);
    let source_width = STARTUP_INTRO_SOURCE_VIEW_COLS as f64;
    let source_height = STARTUP_INTRO_SOURCE_VIEW_ROWS as f64;
    let scale = (target_width / source_width).min(target_height / source_height);
    let rendered_width = source_width * scale;
    let rendered_height = source_height * scale;
    let offset_x = (target_width - rendered_width) / 2.0;
    let offset_y = (target_height - rendered_height) / 2.0;
    let source_x = (f64::from(column) + 0.5 - offset_x) / scale;
    let source_y = (f64::from(row) + 0.5 - offset_y) / scale;
    if !(0.0..source_width).contains(&source_x) || !(0.0..source_height).contains(&source_y) {
        return None;
    }
    Some((
        STARTUP_INTRO_SOURCE_VIEW_LEFT + source_x.floor() as usize,
        STARTUP_INTRO_SOURCE_VIEW_TOP + source_y.floor() as usize,
    ))
}

pub(super) const fn startup_intro_terminal_symbol_index(code: u8) -> usize {
    (code & STARTUP_INTRO_TERMINAL_SYMBOL_MASK) as usize
}

pub(super) const fn startup_intro_terminal_is_bright(code: u8) -> bool {
    code & STARTUP_INTRO_TERMINAL_BRIGHT_MASK != 0
}
