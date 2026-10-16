//! Unit conversions: decimals, USDC atoms and the amount grammar the
//! commands accept. Index values and grid positions are the market's
//! (`shape.rs`), read from the market account.

use std::fmt;

/// USDC has 6 decimals.
pub const USDC_DECIMALS: u32 = 6;
pub const USDC_ATOMS: u64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitError(pub String);

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnitError {}

fn err(message: impl Into<String>) -> UnitError {
    UnitError(message.into())
}

/// Formats `units / 10^decimals` with trailing zeros trimmed (`"100"`, `"100.2"`).
pub fn trim_decimal(units: u128, decimals: u32) -> String {
    let scale = 10u128.pow(decimals);
    let whole = units / scale;
    let frac = units % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let digits = format!("{frac:0width$}", width = decimals as usize);
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

/// Parses a non-negative decimal (`"12"`, `"12.5"`, `".5"`) into units with
/// `decimals` fractional digits. More fractional digits than that are refused.
pub fn parse_decimal(text: &str, decimals: u32) -> Result<u128, UnitError> {
    let raw = text.trim().replace(['_', ','], "");
    let raw = raw.strip_prefix('$').unwrap_or(&raw);
    if raw.is_empty() {
        return Err(err("expected a number"));
    }
    if raw.starts_with('-') {
        return Err(err(format!("`{text}` must not be negative")));
    }
    let (whole, frac) = match raw.split_once('.') {
        Some((w, f)) => (w, f),
        None => (raw, ""),
    };
    if (whole.is_empty() && frac.is_empty())
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return Err(err(format!("`{text}` is not a number")));
    }
    if frac.len() > decimals as usize {
        return Err(err(format!(
            "`{text}` has more than {decimals} decimal places"
        )));
    }
    let scale = 10u128.pow(decimals);
    let whole: u128 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| err(format!("`{text}` is too large")))?
    };
    let frac_units: u128 = if frac.is_empty() {
        0
    } else {
        let padded = format!("{frac:0<width$}", width = decimals as usize);
        padded
            .parse()
            .map_err(|_| err(format!("`{text}` is not a number")))?
    };
    whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac_units))
        .ok_or_else(|| err(format!("`{text}` is too large")))
}

/// `"12.5"` USDC → `12_500_000` atoms.
pub fn parse_usdc(text: &str) -> Result<u64, UnitError> {
    let atoms = parse_decimal(text, USDC_DECIMALS)?;
    let atoms = u64::try_from(atoms).map_err(|_| err(format!("`{text}` is too large")))?;
    if atoms == 0 {
        return Err(err("the amount must be more than zero"));
    }
    Ok(atoms)
}

/// An amount argument that may also be `all`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    All,
    Atoms(u64),
}

pub fn parse_amount(text: &str) -> Result<Amount, UnitError> {
    if text.trim().eq_ignore_ascii_case("all") || text.trim().eq_ignore_ascii_case("max") {
        Ok(Amount::All)
    } else {
        parse_usdc(text).map(Amount::Atoms)
    }
}

/// `12_500_000` atoms → `"12.50"`: at least two decimals, at most six.
pub fn usdc(atoms: u64) -> String {
    usdc_unsigned(u128::from(atoms))
}

fn usdc_unsigned(atoms: u128) -> String {
    let whole = atoms / u128::from(USDC_ATOMS);
    let frac = atoms % u128::from(USDC_ATOMS);
    let digits = format!("{frac:06}");
    let trimmed = digits.trim_end_matches('0');
    let frac = if trimmed.len() < 2 {
        &digits[..2]
    } else {
        trimmed
    };
    format!("{}.{frac}", group(whole))
}

/// Signed atoms with an explicit sign: `"+1.20"`, `"-0.40"`, `"0.00"`.
pub fn usdc_signed(atoms: i128) -> String {
    let body = usdc_unsigned(atoms.unsigned_abs());
    match atoms.signum() {
        1 => format!("+{body}"),
        -1 => format!("-{body}"),
        _ => body,
    }
}

/// Parses an API atom string (`"-1200"`) into signed atoms.
pub fn atoms_i128(text: &str) -> Option<i128> {
    text.trim().parse().ok()
}

/// Formats an API atom string as USDC; unparsable text is returned as is.
// Only the dev faucet (a `dev-faucet` build) uses it today.
#[cfg_attr(not(feature = "dev-faucet"), allow(dead_code))]
pub fn usdc_str(text: &str) -> String {
    match atoms_i128(text) {
        Some(v) if v < 0 => usdc_signed(v),
        Some(v) => usdc_unsigned(v as u128),
        None => text.to_string(),
    }
}

fn group(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Lamports → SOL text.
pub fn sol(lamports: u64) -> String {
    let s = trim_decimal(u128::from(lamports), 9);
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Short form of a base58 key: `7gvW…cXC2`.
pub fn short(key: &str) -> String {
    if key.len() <= 12 {
        key.to_string()
    } else {
        format!("{}…{}", &key[..4], &key[key.len() - 4..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc_atoms_round_trip() {
        assert_eq!(parse_usdc("12.5"), Ok(12_500_000));
        assert_eq!(parse_usdc("1,000"), Ok(1_000_000_000));
        assert_eq!(parse_usdc("$0.000001"), Ok(1));
        assert_eq!(parse_usdc(".25"), Ok(250_000));
        assert!(parse_usdc("0").is_err());
        assert!(parse_usdc("0.0000001").is_err());
        assert!(parse_usdc("-1").is_err());
        assert!(parse_usdc("1e3").is_err());
        assert!(parse_usdc("99999999999999999999").is_err());
        assert_eq!(parse_amount("all"), Ok(Amount::All));
        assert_eq!(parse_amount("5"), Ok(Amount::Atoms(5_000_000)));
        assert_eq!(usdc(12_500_000), "12.50");
        assert_eq!(usdc(1), "0.000001");
        assert_eq!(usdc(1_234_567_890_000), "1,234,567.89");
        assert_eq!(usdc(0), "0.00");
        assert_eq!(usdc_signed(-400_000), "-0.40");
        assert_eq!(usdc_signed(1_200_000), "+1.20");
        assert_eq!(usdc_str("-1200000"), "-1.20");
        assert_eq!(usdc_str("10000000000"), "10,000.00");
    }

    #[test]
    fn decimal_trimming() {
        assert_eq!(trim_decimal(100_200_000, 6), "100.2");
        assert_eq!(trim_decimal(5, 0), "5");
        assert_eq!(sol(1_500_000_000), "1.5");
        assert_eq!(sol(2_000_000_000), "2.0");
        assert_eq!(
            short("7gvWoo3QyURfyc3wCrTkUmbsuWF18ZY8mwV67eyfXcC2"),
            "7gvW…XcC2"
        );
    }
}
