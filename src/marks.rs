//! Printer's marks: crop lines at the trim corners and registration targets.

use crate::layout::Side;
use crate::sheet::Rect;
use crate::units::{MM, num};

/// How the marks are drawn.
#[derive(Debug, Clone, Copy)]
pub struct MarkStyle {
    pub crop: bool,
    pub registration: bool,
    /// Length of each crop line.
    pub length: f64,
    /// Gap between the trim corner and the start of the line.
    pub gap: f64,
    /// Stroke width.
    pub width: f64,
}

impl Default for MarkStyle {
    fn default() -> Self {
        Self {
            crop: true,
            registration: false,
            length: 3.0 * MM,
            gap: 0.0,
            width: 0.25,
        }
    }
}

impl MarkStyle {
    /// Nothing at all, for a proof without furniture.
    #[must_use]
    pub fn none() -> Self {
        Self {
            crop: false,
            registration: false,
            ..Self::default()
        }
    }
}

/// The marks for one side, as a content stream fragment.
pub fn content(side: &Side, area: Rect, style: &MarkStyle) -> String {
    if !style.crop && !style.registration {
        return String::new();
    }
    let mut out = String::from("q\n1 1 1 1 K\n");
    out.push_str(&format!("{} w\n", num(style.width)));
    out.push_str("[] 0 d\n");

    if style.crop {
        for placement in &side.placements {
            if placement.page.is_none() {
                continue;
            }
            let t = placement.tile;
            let len = style.length;
            let gap = style.gap;
            // Left edge of the tile: two horizontal lines, one at each corner.
            for y in [t.y, t.y + t.h] {
                out.push_str(&line(t.x - gap - len, y, t.x - gap, y));
            }
            // Right edge.
            for y in [t.y, t.y + t.h] {
                out.push_str(&line(t.x + t.w + gap, y, t.x + t.w + gap + len, y));
            }
            // Bottom edge.
            for x in [t.x, t.x + t.w] {
                out.push_str(&line(x, t.y - gap - len, x, t.y - gap));
            }
            // Top edge.
            for x in [t.x, t.x + t.w] {
                out.push_str(&line(x, t.y + t.h + gap, x, t.y + t.h + gap + len));
            }
        }
    }

    if style.registration {
        let radius = 2.0 * MM;
        let arm = 4.0 * MM;
        for (cx, cy) in [
            (area.x, area.y),
            (area.x + area.w, area.y),
            (area.x, area.y + area.h),
            (area.x + area.w, area.y + area.h),
        ] {
            out.push_str(&circle(cx, cy, radius));
            out.push_str(&circle(cx, cy, radius * 0.55));
            out.push_str(&line(cx - arm, cy, cx - radius * 1.15, cy));
            out.push_str(&line(cx + radius * 1.15, cy, cx + arm, cy));
            out.push_str(&line(cx, cy - arm, cx, cy - radius * 1.15));
            out.push_str(&line(cx, cy + radius * 1.15, cx, cy + arm));
        }
    }

    out.push_str("Q\n");
    out
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> String {
    format!("{} {} m {} {} l S\n", num(x1), num(y1), num(x2), num(y2))
}

/// A circle from four cubic Béziers, because PDF has no circle operator.
fn circle(cx: f64, cy: f64, r: f64) -> String {
    let k = 0.552_284_749_8 * r;
    let mut out = String::new();
    out.push_str(&format!("{} {} m\n", num(cx + r), num(cy)));
    for (c1, c2, end) in [
        ((cx + r, cy + k), (cx + k, cy + r), (cx, cy + r)),
        ((cx - k, cy + r), (cx - r, cy + k), (cx - r, cy)),
        ((cx - r, cy - k), (cx - k, cy - r), (cx, cy - r)),
        ((cx + k, cy - r), (cx + r, cy - k), (cx + r, cy)),
    ] {
        out.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            num(c1.0),
            num(c1.1),
            num(c2.0),
            num(c2.1),
            num(end.0),
            num(end.1)
        ));
    }
    out.push_str("S\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Placement, Tile};

    fn side() -> Side {
        Side {
            sheet: 0,
            back: false,
            placements: vec![Placement {
                page: Some(0),
                tile: Tile {
                    x: 100.0,
                    y: 100.0,
                    w: 200.0,
                    h: 300.0,
                },
                dx: 0.0,
                dy: 0.0,
            }],
        }
    }

    #[test]
    fn crop_marks_draw_eight_lines_per_tile() {
        let style = MarkStyle {
            crop: true,
            registration: false,
            ..MarkStyle::default()
        };
        let text = content(&side(), Rect::new(0.0, 0.0, 600.0, 800.0), &style);
        assert_eq!(text.matches(" l S").count(), 8);
        assert!(text.starts_with("q\n"));
        assert!(text.ends_with("Q\n"));
    }

    #[test]
    fn blank_tiles_get_no_marks() {
        let mut side = side();
        side.placements[0].page = None;
        let text = content(
            &side,
            Rect::new(0.0, 0.0, 600.0, 800.0),
            &MarkStyle::default(),
        );
        assert_eq!(text.matches(" l S").count(), 0);
    }

    #[test]
    fn nothing_requested_draws_nothing() {
        let text = content(
            &side(),
            Rect::new(0.0, 0.0, 600.0, 800.0),
            &MarkStyle::none(),
        );
        assert!(text.is_empty());
    }
}
