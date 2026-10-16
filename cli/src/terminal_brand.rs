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

#[cfg(test)]
const EMPTY_WORD: [&str; 5] = [
    " _____ __  __ ____ _______   __",
    "| ____|  \\/  |  _ \\_   _\\ \\ / /",
    "|  _| | |\\/| | |_) || |  \\ V /",
    "| |___| |  | |  __/ | |   | |",
    "|_____|_|  |_|_|    |_|   |_|",
];

#[cfg(test)]
pub(crate) fn terminal_banner_lines() -> Vec<BrandLine> {
    terminal_banner_lines_for_width(WIDE_BANNER_MIN_COLUMNS)
}

pub(crate) fn terminal_banner_lines_for_width(columns: usize) -> Vec<BrandLine> {
    banner_lines_for_width(columns).to_vec()
}

#[cfg(test)]
pub(crate) fn terminal_title_lines_for_width(columns: usize) -> &'static [BrandLine] {
    if columns >= WIDE_BANNER_MIN_COLUMNS {
        &wide_banner_lines()[1..7]
    } else {
        &compact_banner_lines()[1..2]
    }
}

#[cfg(test)]
pub(crate) fn ascii_header_lines() -> Vec<String> {
    ascii_header_lines_for_width(WIDE_BANNER_MIN_COLUMNS)
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::{
        BrandLine, BrandSpan, BrandTone, WIDE_BANNER_MIN_COLUMNS, ascii_header_lines,
        ascii_header_lines_for_width, empty_word_lines, terminal_banner_lines,
        terminal_banner_lines_for_width, terminal_title_lines_for_width,
    };

    const EXPECTED_PLAIN_BANNER: &str = "  Welcome to Petri -\n  ████┐ ██┐   ██┐ █████┐ ██████┐█████┐  ████┐   ██████┐ ████┐ █████┐ ██┐   ██┐\n ██┌─██┐███┐ ███│██┌──██┐██┌───┘██┌─██┐██┌─██┐  ██┌───┘██┌─██┐██┌─██┐███┐ ███│\n ██████│██┌██┼██│██│  ██│█████┐ █████┌┘██████│  █████┐ ██████│█████┌┘██┌██┼██│\n ██┌─██│██│└─┘██│██│  ██│██┌──┘ ██┌─██┐██┌─██│  ██┌──┘ ██┌─██│██┌██│ ██│└─┘██│\n ██│ ██│██│   ██│└█████┌┘██████┐█████┌┘██│ ██│  ██│    ██│ ██│██│└██┐██│   ██│\n └─┘ └─┘└─┘   └─┘ └────┘ └─────┘└────┘ └─┘ └─┘  └─┘    └─┘ └─┘└─┘ └─┘└─┘   └─┘\n                        Terminal/Command-line interface";

    fn banner_contract_hash(lines: &[BrandLine]) -> u64 {
        let mut hash = 0xcbf29ce484222325_u64;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        };

        for line in lines {
            mix(&[0xfe]);
            for span in line {
                let tone = match span.tone {
                    BrandTone::Title => 1,
                    BrandTone::Outline => 2,
                    BrandTone::Body => 3,
                    BrandTone::Plain => 4,
                };
                mix(&[0xff, tone, u8::from(span.fill)]);
                mix(span.text.as_bytes());
                mix(&[0]);
            }
        }
        hash
    }

    #[test]
    fn wide_ascii_header_matches_the_supplied_banner() {
        assert_eq!(ascii_header_lines().join("\n"), EXPECTED_PLAIN_BANNER);
    }

    #[test]
    fn narrow_ascii_header_uses_the_three_line_fallback() {
        assert_eq!(
            ascii_header_lines_for_width(WIDE_BANNER_MIN_COLUMNS - 1),
            [
                "Welcome to Petri -",
                "AMOEBA FARM",
                "Terminal/Command-line interface"
            ]
        );
    }

    #[test]
    fn title_lines_reuse_only_the_amoeba_farm_part_of_the_banner() {
        let wide: &'static [BrandLine] = terminal_title_lines_for_width(WIDE_BANNER_MIN_COLUMNS);
        let compact: &'static [BrandLine] =
            terminal_title_lines_for_width(WIDE_BANNER_MIN_COLUMNS - 1);

        assert_eq!(wide.len(), 6);
        assert_eq!(compact.len(), 1);
        assert_eq!(
            compact[0]
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "AMOEBA FARM"
        );
    }

    #[test]
    fn terminal_banner_uses_only_title_outline_body_and_plain_regions() {
        let lines = terminal_banner_lines();
        let tones = lines
            .iter()
            .flat_map(|line| line.iter().map(|span| span.tone))
            .collect::<Vec<_>>();

        assert!(tones.contains(&BrandTone::Title));
        assert!(tones.contains(&BrandTone::Outline));
        assert!(tones.contains(&BrandTone::Body));
        assert!(tones.contains(&BrandTone::Plain));
        assert!(lines.iter().flatten().any(|span| span.fill));
        assert_eq!(lines.len(), 8);
    }

    #[test]
    fn banner_rows_preserve_exact_tone_and_fill_roles() {
        let wide = terminal_banner_lines();
        assert_eq!(
            wide.first(),
            Some(&vec![BrandSpan {
                text: "  Welcome to Petri -".to_string(),
                tone: BrandTone::Body,
                fill: false,
            }])
        );
        assert_eq!(
            wide.last(),
            Some(&vec![BrandSpan {
                text: "                        Terminal/Command-line interface".to_string(),
                tone: BrandTone::Body,
                fill: false,
            }])
        );
        for line in &wide[1..7] {
            for span in line {
                let role = if span.text.chars().all(|character| character == '█') {
                    (BrandTone::Title, true)
                } else if span.text.chars().all(|character| character == ' ') {
                    (BrandTone::Plain, false)
                } else {
                    (BrandTone::Outline, false)
                };
                assert_eq!((span.tone, span.fill), role);
            }
        }

        let compact = terminal_banner_lines_for_width(WIDE_BANNER_MIN_COLUMNS - 1);
        for (line, text) in compact.iter().zip([
            "Welcome to Petri -",
            "AMOEBA FARM",
            "Terminal/Command-line interface",
        ]) {
            assert_eq!(
                line,
                &vec![BrandSpan {
                    text: text.to_string(),
                    tone: BrandTone::Body,
                    fill: false,
                }]
            );
        }
    }

    #[test]
    fn empty_word_renders_non_empty_banner() {
        let lines = empty_word_lines();

        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| !line.trim().is_empty()));
    }

    #[test]
    fn banner_span_and_empty_word_contract_is_exact() {
        assert_eq!(
            banner_contract_hash(&terminal_banner_lines()),
            17_298_661_812_663_609_207
        );
        assert_eq!(
            banner_contract_hash(&terminal_banner_lines_for_width(
                WIDE_BANNER_MIN_COLUMNS - 1
            )),
            14_665_893_697_779_234_526
        );
        assert_eq!(
            empty_word_lines(),
            [
                " _____ __  __ ____ _______   __",
                "| ____|  \\/  |  _ \\_   _\\ \\ / /",
                "|  _| | |\\/| | |_) || |  \\ V /",
                "| |___| |  | |  __/ | |   | |",
                "|_____|_|  |_|_|    |_|   |_|",
            ]
        );
    }
}
