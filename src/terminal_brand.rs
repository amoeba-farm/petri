use std::env;
use std::sync::OnceLock;

pub(crate) const WIDE_BANNER_MIN_COLUMNS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrandTone {
    Title,
    Outline,
    Body,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BrandSpan {
    pub(crate) text: String,
    pub(crate) tone: BrandTone,
    pub(crate) fill: bool,
}

pub(crate) type BrandLine = Vec<BrandSpan>;

const WIDE_HEADER: [&str; 8] = [
    "  Welcome to Petri -",
    "  ████┐ ██┐   ██┐ █████┐ ██████┐█████┐  ████┐   ██████┐ ████┐ █████┐ ██┐   ██┐",
    " ██┌─██┐███┐ ███│██┌──██┐██┌───┘██┌─██┐██┌─██┐  ██┌───┘██┌─██┐██┌─██┐███┐ ███│",
    " ██████│██┌██┼██│██│  ██│█████┐ █████┌┘██████│  █████┐ ██████│█████┌┘██┌██┼██│",
    " ██┌─██│██│└─┘██│██│  ██│██┌──┘ ██┌─██┐██┌─██│  ██┌──┘ ██┌─██│██┌██│ ██│└─┘██│",
    " ██│ ██│██│   ██│└█████┌┘██████┐█████┌┘██│ ██│  ██│    ██│ ██│██│└██┐██│   ██│",
    " └─┘ └─┘└─┘   └─┘ └────┘ └─────┘└────┘ └─┘ └─┘  └─┘    └─┘ └─┘└─┘ └─┘└─┘   └─┘",
    "                        Terminal/Command-line interface",
];

const COMPACT_HEADER: [&str; 3] = [
    "Welcome to Petri -",
    "AMOEBA FARM",
    "Terminal/Command-line interface",
];

const EMPTY_WORD: [&str; 5] = [
    " _____ __  __ ____ _______   __",
    "| ____|  \\/  |  _ \\_   _\\ \\ / /",
    "|  _| | |\\/| | |_) || |  \\ V /",
    "| |___| |  | |  __/ | |   | |",
    "|_____|_|  |_|_|    |_|   |_|",
];

pub(crate) fn terminal_banner_lines_for_width(columns: usize) -> Vec<BrandLine> {
    banner_lines_for_width(columns).to_vec()
}

pub(crate) fn terminal_title_lines_for_width(columns: usize) -> &'static [BrandLine] {
    if columns >= WIDE_BANNER_MIN_COLUMNS {
        &wide_banner_lines()[1..7]
    } else {
        &compact_banner_lines()[1..2]
    }
}

pub(crate) fn ascii_header_lines_for_width(columns: usize) -> Vec<String> {
    let lines = if columns >= WIDE_BANNER_MIN_COLUMNS {
        &WIDE_HEADER[..]
    } else {
        &COMPACT_HEADER[..]
    };
    lines
        .iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}

pub(crate) fn empty_word_lines() -> Vec<String> {
    EMPTY_WORD.iter().map(|line| (*line).to_string()).collect()
}

pub(crate) fn color_enabled(plain: bool, output_is_tty: bool) -> bool {
    if plain || env::var_os("NO_COLOR").is_some() {
        return false;
    }
    match env::var("FORCE_COLOR") {
        Ok(value) if value == "0" => false,
        Ok(_) => true,
        Err(_) => output_is_tty,
    }
}

fn banner_lines_for_width(columns: usize) -> &'static [BrandLine] {
    if columns >= WIDE_BANNER_MIN_COLUMNS {
        wide_banner_lines()
    } else {
        compact_banner_lines()
    }
}

fn wide_banner_lines() -> &'static [BrandLine] {
    static LINES: OnceLock<Vec<BrandLine>> = OnceLock::new();
    LINES
        .get_or_init(|| {
            WIDE_HEADER
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    if index == 0 || index == WIDE_HEADER.len() - 1 {
                        body_line(line)
                    } else {
                        graphical_line(line)
                    }
                })
                .collect()
        })
        .as_slice()
}

fn compact_banner_lines() -> &'static [BrandLine] {
    static LINES: OnceLock<Vec<BrandLine>> = OnceLock::new();
    LINES
        .get_or_init(|| COMPACT_HEADER.iter().map(|line| body_line(line)).collect())
        .as_slice()
}

fn body_line(text: &str) -> BrandLine {
    vec![BrandSpan {
        text: text.to_string(),
        tone: BrandTone::Body,
        fill: false,
    }]
}

fn graphical_line(text: &str) -> BrandLine {
    let mut spans = Vec::new();
    let mut current_role: Option<(BrandTone, bool)> = None;
    let mut current_text = String::new();

    for character in text.chars() {
        let role = match character {
            '█' => (BrandTone::Title, true),
            ' ' => (BrandTone::Plain, false),
            _ => (BrandTone::Outline, false),
        };
        if current_role.is_some_and(|existing| existing != role) {
            let (tone, fill) = current_role.expect("role is set before flushing");
            spans.push(BrandSpan {
                text: current_text,
                tone,
                fill,
            });
            current_text = String::new();
        }
        current_role = Some(role);
        current_text.push(character);
    }

    if let Some((tone, fill)) = current_role {
        spans.push(BrandSpan {
            text: current_text,
            tone,
            fill,
        });
    }

    spans
}
