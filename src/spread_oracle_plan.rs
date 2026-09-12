//! Semantic oracle evidence validation only.
//!
//! Current instruction planning, account manifests, packet construction, signing,
//! and receipts are owned by `ameba-sdk/operator`. Petri intentionally does
//! not reproduce that transaction logic in Rust while the public operator
//! contract is not frozen.

use chrono::{DateTime, Datelike, NaiveDateTime};
use solana_program::hash::hashv;

use crate::backend::CliError;

const ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES: usize = 384;
const ORACLE_OPENING_ARCHIVE_URL_PREFIX: &str = "https://web.archive.org/web/";

pub(crate) fn validate_opening_archive_url(
    archive_url: &str,
    canonical_locator: &str,
    source_time: &str,
) -> Result<(), CliError> {
    let archive_url = archive_url.trim();
    let canonical_locator = canonical_locator.trim();
    let bytes = archive_url.as_bytes();
    if bytes.len() <= ORACLE_OPENING_ARCHIVE_URL_PREFIX.len()
        || bytes.len() > ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES
        || !archive_url.starts_with(ORACLE_OPENING_ARCHIVE_URL_PREFIX)
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(invalid_opening_archive_url_error());
    }

    let suffix = archive_url
        .strip_prefix(ORACLE_OPENING_ARCHIVE_URL_PREFIX)
        .unwrap_or_default();
    let Some((capture, archived_target)) = suffix.split_once('/') else {
        return Err(invalid_opening_archive_url_error());
    };
    if capture.len() != 14
        || !capture.bytes().all(|byte| byte.is_ascii_digit())
        || !(archived_target.starts_with("https://") || archived_target.starts_with("http://"))
        || archived_target.is_empty()
        || !has_nonempty_raw_url_authority(archived_target)
    {
        return Err(invalid_opening_archive_url_error());
    }

    let capture_time = NaiveDateTime::parse_from_str(capture, "%Y%m%d%H%M%S")
        .ok()
        .filter(|value| (1970..=9999).contains(&value.year()))
        .and_then(|value| u64::try_from(value.and_utc().timestamp()).ok())
        .ok_or_else(invalid_opening_archive_url_error)?;
    let source_time = parse_source_time(source_time)?;
    if capture_time != source_time {
        return Err(CliError::new(
            "Wayback capture timestamp must exactly equal the opening source timestamp in UTC",
        ));
    }

    if canonical_locator.is_empty()
        || hash32_hex(&["locator", archived_target]) != hash32_hex(&["locator", canonical_locator])
    {
        return Err(CliError::new(
            "Wayback archive URL must archive the exact Canonical locator entered for this source",
        ));
    }
    Ok(())
}

fn has_nonempty_raw_url_authority(target: &str) -> bool {
    let Some(after_scheme) = target
        .strip_prefix("https://")
        .or_else(|| target.strip_prefix("http://"))
    else {
        return false;
    };
    let authority_end = after_scheme
        .find(|character| matches!(character, '/' | '?' | '#'))
        .unwrap_or(after_scheme.len());
    authority_end > 0
}

fn invalid_opening_archive_url_error() -> CliError {
    CliError::new(format!(
        "Wayback archive URL must be {ORACLE_OPENING_ARCHIVE_URL_PREFIX}<14 ASCII digits>/<absolute http(s) Canonical locator>, contain no whitespace, and be at most {ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES} UTF-8 bytes"
    ))
}

fn parse_source_time(raw: &str) -> Result<u64, CliError> {
    let raw = raw.trim();
    if let Ok(value) = raw.parse::<u64>() {
        return Ok(value);
    }
    let parsed = DateTime::parse_from_rfc3339(raw).map_err(|_| {
        CliError::new(format!(
            "invalid source timestamp '{raw}'; use RFC 3339 or Unix seconds"
        ))
    })?;
    u64::try_from(parsed.timestamp())
        .map_err(|_| CliError::new("source timestamp must not be before Unix epoch"))
}

fn hash32_hex(parts: &[&str]) -> String {
    let byte_parts = parts.iter().map(|part| part.as_bytes()).collect::<Vec<_>>();
    format!("0x{}", bytes_to_hex(&hashv(&byte_parts).to_bytes()))
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use core::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
