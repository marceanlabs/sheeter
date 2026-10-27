//! Length parsing. Millimetres unless a unit is given, because the people who
//! impose sheets measure in millimetres.

use crate::Error;

/// Points per millimetre, from the definition of the international inch.
pub const MM: f64 = 72.0 / 25.4;

/// Parse a length into points.
///
/// A bare number is millimetres. Suffixes: `mm`, `cm`, `in`, `pt`.
pub fn parse_len(raw: &str) -> Result<f64, Error> {
    let text = raw.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Err(Error::Unit("empty length".into()));
    }
    let (number, factor) = if let Some(v) = text.strip_suffix("mm") {
        (v, MM)
    } else if let Some(v) = text.strip_suffix("cm") {
        (v, 10.0 * MM)
    } else if let Some(v) = text.strip_suffix("in") {
        (v, 72.0)
    } else if let Some(v) = text.strip_suffix("pt") {
        (v, 1.0)
    } else {
        (text.as_str(), MM)
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| Error::Unit(format!("{raw:?} is not a length")))?;
    if !value.is_finite() || value < 0.0 {
        return Err(Error::Unit(format!("{raw:?} is not a usable length")));
    }
    Ok(value * factor)
}

/// Format a number for a PDF content stream: fixed point, no exponent, no
/// trailing zeros, and never `-0`.
pub fn num(value: f64) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let mut text = format!("{rounded:.4}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() || text == "-" {
        text.push('0');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_carry_units() {
        assert!((parse_len("25.4mm").unwrap() - 72.0).abs() < 1e-9);
        assert!((parse_len("72pt").unwrap() - 72.0).abs() < 1e-9);
        assert!((parse_len("1in").unwrap() - 72.0).abs() < 1e-9);
        assert!((parse_len("2.54cm").unwrap() - 72.0).abs() < 1e-9);
        assert!((parse_len("10").unwrap() - 10.0 * MM).abs() < 1e-9);
    }

    #[test]
    fn numbers_print_cleanly() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1.0 / 3.0), "0.3333");
    }
}
