//! Petri's error type and terminal-safe output helpers shared by every
//! command: typed CLI errors, control-character neutralization and JSON
//! output that cannot inject terminal escapes.

use serde_json::Value;
use std::error::Error;
use std::fmt::{Display, Formatter, Write as _};

const MAX_ERROR_DISPLAY_CHARS: usize = 4 * 1024;

#[derive(Debug)]
pub struct CliError {
    message: String,
    code: String,
    category: String,
    retryable: bool,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::coded("PETRI_VALIDATION_FAILED", "validation", message, false)
    }

    pub fn coded(
        code: impl Into<String>,
        category: &str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            category: category.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn json(&self) -> Value {
        serde_json::json!({"ok": false, "error": {
            "code": self.code,
            "category": self.category,
            "message": terminal_safe_text(&self.message),
            "retryable": self.retryable,
        }})
    }

    pub fn exit_code(&self) -> i32 {
        match self.category.as_str() {
            "denied" => 77,
            "unavailable" | "not_available" => 69,
            "execution_failed" => 70,
            _ => 1,
        }
    }
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", terminal_safe_text(&self.message))
    }
}

impl Error for CliError {}

pub(crate) fn terminal_safe_text(raw: &str) -> String {
    raw.chars()
        .take(MAX_ERROR_DISPLAY_CHARS)
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        // Unicode format controls (General_Category=Cf) are invisible or can
        // alter neighboring text. Replace the complete Unicode 15.1 set rather
        // than maintaining only the bidi subset.
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

pub fn json_string(value: &Value) -> Result<String, CliError> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::new(format!("failed to format JSON output: {error}")))?;
    let mut escaped = String::with_capacity(serialized.len());
    for character in serialized.chars() {
        if character >= '\u{007f}' && (character.is_control() || is_bidi_control(character)) {
            write_json_unicode_escape(&mut escaped, character);
        } else {
            escaped.push(character);
        }
    }
    Ok(escaped)
}

fn write_json_unicode_escape(output: &mut String, character: char) {
    let codepoint = character as u32;
    if codepoint <= 0xffff {
        write!(output, "\\u{codepoint:04x}").expect("writing to a String cannot fail");
        return;
    }

    let supplementary = codepoint - 0x1_0000;
    let high_surrogate = 0xd800 + (supplementary >> 10);
    let low_surrogate = 0xdc00 + (supplementary & 0x3ff);
    write!(output, "\\u{high_surrogate:04x}\\u{low_surrogate:04x}")
        .expect("writing to a String cannot fail");
}

pub fn value_at_key<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        if let Some(found) = value.get(*key) {
            return Some(found);
        }
    }
    None
}

pub fn string_at_key(value: &Value, keys: &[&str]) -> Option<String> {
    value_at_key(value, keys).and_then(|item| match item {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_errors_replace_terminal_and_bidi_control_characters() {
        let unsafe_text = "unsafe\u{1b}]8;;https://example.test\u{7}link\u{1b}]8;;\u{7}\u{0085}\u{00ad}\u{0600}\u{061c}\u{200b}\u{200c}\u{200d}\u{2028}\u{2029}\u{202e}\u{2060}\u{2064}\u{206a}\u{206f}\u{feff}\u{e0001}text";
        let rendered = CliError::new(unsafe_text).to_string();
        for control in [
            '\u{1b}',
            '\u{7}',
            '\u{0085}',
            '\u{00ad}',
            '\u{0600}',
            '\u{061c}',
            '\u{200b}',
            '\u{200c}',
            '\u{200d}',
            '\u{2028}',
            '\u{2029}',
            '\u{202e}',
            '\u{2060}',
            '\u{2064}',
            '\u{206a}',
            '\u{206f}',
            '\u{feff}',
            '\u{e0001}',
        ] {
            assert!(
                !rendered.contains(control),
                "terminal-safe output retained U+{:04X}",
                control as u32
            );
        }
        assert!(rendered.contains("unsafe"));
        assert!(rendered.contains("text"));
        assert!(rendered.contains("link"));
    }

    #[test]
    fn cli_errors_cap_attacker_controlled_display_text() {
        let rendered = CliError::new("x".repeat(MAX_ERROR_DISPLAY_CHARS + 512)).to_string();
        assert_eq!(rendered.chars().count(), MAX_ERROR_DISPLAY_CHARS);
    }

    #[test]
    fn json_output_escapes_terminal_controls_without_changing_parsed_values() {
        let unsafe_text =
            "safe\u{007f}\u{0085}\u{061c}\u{200b}\u{2028}\u{202e}\u{2066}\u{feff}\u{e0001}tail";
        let payload = serde_json::json!({"backendField": unsafe_text});
        let rendered = json_string(&payload).expect("format terminal-safe JSON");

        for control in [
            '\u{007f}',
            '\u{0085}',
            '\u{061c}',
            '\u{200b}',
            '\u{2028}',
            '\u{202e}',
            '\u{2066}',
            '\u{feff}',
            '\u{e0001}',
        ] {
            assert!(
                !rendered.contains(control),
                "JSON terminal output retained U+{:04X}",
                control as u32
            );
        }
        assert!(rendered.contains("\\u202e"));
        assert!(rendered.contains("\\udb40\\udc01"));
        assert_eq!(
            serde_json::from_str::<Value>(&rendered).expect("escaped output remains valid JSON"),
            payload,
            "escaping must preserve machine-readable JSON values"
        );
    }

    #[test]
    fn large_json_output_is_printed_whole() {
        // No size ceiling (preflight constraint C24 removed the old 4 MiB refusal).
        let payload = serde_json::json!({ "backendField": "x".repeat(5 * 1024 * 1024) });
        let rendered = json_string(&payload).expect("large JSON output is printed");
        assert_eq!(serde_json::from_str::<Value>(&rendered).unwrap(), payload);
    }
}
