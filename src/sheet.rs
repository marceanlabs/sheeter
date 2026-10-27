//! Sheet sizes, named or measured.

use crate::Error;
use crate::units::parse_len;

/// A rectangle in points, origin at the bottom left as PDF coordinates do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    #[must_use]
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// Inset every edge.
    #[must_use]
    pub fn inset(self, amount: f64) -> Self {
        Self {
            x: self.x + amount,
            y: self.y + amount,
            w: (self.w - 2.0 * amount).max(0.0),
            h: (self.h - 2.0 * amount).max(0.0),
        }
    }
}

const NAMED: &[(&str, f64, f64)] = &[
    ("a0", 2383.94, 3370.39),
    ("a1", 1683.78, 2383.94),
    ("a2", 1190.55, 1683.78),
    ("a3", 841.89, 1190.55),
    ("a4", 595.28, 841.89),
    ("a5", 419.53, 595.28),
    ("a6", 297.64, 419.53),
    ("sra3", 907.09, 1275.59),
    ("sra4", 637.80, 907.09),
    ("letter", 612.0, 792.0),
    ("legal", 612.0, 1008.0),
    ("tabloid", 792.0, 1224.0),
];

const UNITS: &[&str] = &["mm", "cm", "in", "pt"];

/// Parse a sheet: `A3`, `sra3`, `320x450`, `12x18in`, `210x297mm`, `A4L`.
///
/// A trailing `l` or `-landscape` swaps the sides.
pub fn parse_sheet(raw: &str) -> Result<Rect, Error> {
    let text = raw.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Err(Error::Sheet("empty sheet size".into()));
    }
    if let Ok(size) = parse_one(&text) {
        return Ok(size);
    }
    if let Some(stripped) = text.strip_suffix("-landscape") {
        let mut size = parse_one(stripped)?;
        std::mem::swap(&mut size.w, &mut size.h);
        return Ok(size);
    }
    if let Some(stripped) = text.strip_suffix('l') {
        let mut size = parse_one(stripped)?;
        std::mem::swap(&mut size.w, &mut size.h);
        return Ok(size);
    }
    Err(Error::Sheet(format!("{raw:?} is not a known sheet size")))
}

fn parse_one(body: &str) -> Result<Rect, Error> {
    let Some((left, right)) = body.split_once('x') else {
        let found = NAMED
            .iter()
            .find(|(name, _, _)| *name == body)
            .ok_or_else(|| Error::Sheet(format!("{body:?} is not a known sheet size")))?;
        return Ok(Rect::new(0.0, 0.0, found.1, found.2));
    };
    // A unit on the second side applies to both, so `12x18in` is inches twice.
    let right_unit = UNITS.iter().find(|unit| right.ends_with(*unit)).copied();
    let width_text = if UNITS.iter().any(|unit| left.ends_with(unit)) {
        left.to_string()
    } else {
        format!("{left}{}", right_unit.unwrap_or("mm"))
    };
    let height_text = if right_unit.is_some() {
        right.to_string()
    } else {
        format!("{right}mm")
    };
    let width = parse_len(&width_text)?;
    let height = parse_len(&height_text)?;
    if width <= 0.0 || height <= 0.0 {
        return Err(Error::Sheet(format!("{body:?} has a side of zero")));
    }
    Ok(Rect::new(0.0, 0.0, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_sizes_parse() {
        let a4 = parse_sheet("A4").unwrap();
        assert!((a4.w - 595.28).abs() < 0.01);
        let a4l = parse_sheet("a4l").unwrap();
        assert!((a4l.w - 841.89).abs() < 0.01);
        let legal = parse_sheet("legal").unwrap();
        assert!((legal.h - 1008.0).abs() < 0.01);
    }

    #[test]
    fn measured_sizes_parse() {
        let s = parse_sheet("320x450").unwrap();
        assert!((s.w - 320.0 * 2.834_645_669).abs() < 0.01);
        let s = parse_sheet("12x18in").unwrap();
        assert!((s.w - 864.0).abs() < 0.01, "w={}", s.w);
        assert!((s.h - 1296.0).abs() < 0.01);
        let s = parse_sheet("210x297mm").unwrap();
        assert!((s.w - 595.275).abs() < 0.01);
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(parse_sheet("A99").is_err());
        assert!(parse_sheet("").is_err());
        assert!(parse_sheet("0x10").is_err());
    }
}
