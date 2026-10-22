//! Exact decimals in the oracle's six-decimal atoms.
//!
//! The oracle stores every value with six decimals (`Period.value_decimals`,
//! petri's `value_atoms`). An authenticated value converts only when it is
//! exactly representable: no rounding, no exponent, no thousands separators,
//! no currency symbols. A value the source prints differently needs a
//! manifest that points at an exact field, not a lenient parser.

pub const VALUE_DECIMALS: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecimalError {
    #[error("`{0}` is not a plain decimal number")]
    NotDecimal(String),
    #[error("`{0}` has more than six decimal places")]
    TooPrecise(String),
    #[error("`{0}` is too large")]
    TooLarge(String),
    #[error("`{0}` is not more than zero")]
    NotPositive(String),
}

/// `"12.5"` → `12_500_000`. Accepts `digits[.digits]` only; a leading zero
/// only as the sole whole digit (JSON's own number grammar, minus sign and
/// exponent). Trailing fractional zeros beyond six places are refused too:
/// the source printed more precision than the oracle keeps, and dropping it
/// silently would be a rounding.
pub fn parse_atoms(text: &str) -> Result<u64, DecimalError> {
    let bad = || DecimalError::NotDecimal(text.to_string());
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => {
            if f.is_empty() {
                return Err(bad());
            }
            (w, f)
        }
        None => (text, ""),
    };
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
    {
        return Err(bad());
    }
    if frac.len() > VALUE_DECIMALS as usize {
        return Err(DecimalError::TooPrecise(text.to_string()));
    }
    let scale = 10u128.pow(VALUE_DECIMALS);
    let whole: u128 = whole
        .parse()
        .map_err(|_| DecimalError::TooLarge(text.to_string()))?;
    let frac: u128 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<6}")
            .parse()
            .map_err(|_| DecimalError::NotDecimal(text.to_string()))?
    };
    let atoms = whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .and_then(|a| u64::try_from(a).ok())
        .ok_or_else(|| DecimalError::TooLarge(text.to_string()))?;
    if atoms == 0 {
        return Err(DecimalError::NotPositive(text.to_string()));
    }
    Ok(atoms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_convert() {
        assert_eq!(parse_atoms("12.5"), Ok(12_500_000));
        assert_eq!(parse_atoms("0.000001"), Ok(1));
        assert_eq!(parse_atoms("75"), Ok(75_000_000));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "", ".5", "5.", "-1", "1e3", "1,000", "$5", " 5", "01", "1.2.3", "0x10", "+1",
        ] {
            assert!(parse_atoms(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_atoms("1.0000001"),
            Err(DecimalError::TooPrecise("1.0000001".into()))
        );
        assert_eq!(
            parse_atoms("1.1000000"),
            Err(DecimalError::TooPrecise("1.1000000".into()))
        );
        assert_eq!(parse_atoms("0"), Err(DecimalError::NotPositive("0".into())));
        assert!(matches!(
            parse_atoms("99999999999999999999"),
            Err(DecimalError::TooLarge(_))
        ));
    }
}
