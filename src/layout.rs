//! Where each source page goes on each side of each sheet.

use crate::Error;
use crate::sheet::Rect;

/// A position on the sheet, in points, origin bottom left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One page placed in one tile. `dx`/`dy` nudge the content without moving the
/// trim marks, which is what creep compensation does to a saddle-stitched stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub page: Option<usize>,
    pub tile: Tile,
    pub dx: f64,
    pub dy: f64,
}

/// One printed side. A duplex sheet has two, printed back to back.
#[derive(Debug, Clone, PartialEq)]
pub struct Side {
    pub sheet: usize,
    pub back: bool,
    pub placements: Vec<Placement>,
}

/// Which imposition to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Pages in reading order, `cols` by `rows` per side.
    Nup,
    /// Saddle stitch: sheet 1 carries the outside, sheet 2 the next pair inward.
    Booklet,
    /// One block of pages repeated across the whole sheet, so the stack can be
    /// guillotined into identical piles.
    Step,
}

impl Kind {
    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "nup" | "n-up" | "grid" => Ok(Self::Nup),
            "booklet" | "saddle" | "saddle-stitch" => Ok(Self::Booklet),
            "step" | "step-repeat" | "cut-stack" | "cutstack" => Ok(Self::Step),
            other => Err(Error::Layout(format!(
                "{other:?} is not a layout: use nup, booklet or step"
            ))),
        }
    }
}

/// Everything the planner needs that is not the page count.
#[derive(Debug, Clone)]
pub struct Plan {
    pub kind: Kind,
    pub cols: u32,
    pub rows: u32,
    /// Area inside the sheet margins.
    pub area: Rect,
    pub gutter: f64,
    pub duplex: bool,
    /// Mirror each back side left to right, for flipping on the long edge.
    pub mirror_back: bool,
    /// Per-sheet inward shift for saddle stitching, in points.
    pub creep: f64,
    /// How many sheets a `step` job produces.
    pub copies: u32,
}

/// Build the tile grid, row-major with row 0 at the top of the sheet.
fn tiles(area: Rect, cols: u32, rows: u32, gutter: f64) -> Result<Vec<Tile>, Error> {
    if cols == 0 || rows == 0 {
        return Err(Error::Layout(
            "a layout needs at least one column and row".into(),
        ));
    }
    let gaps_x = f64::from(cols - 1);
    let gaps_y = f64::from(rows - 1);
    let tw = (area.w - gutter * gaps_x) / f64::from(cols);
    let th = (area.h - gutter * gaps_y) / f64::from(rows);
    if tw <= 1.0 || th <= 1.0 {
        return Err(Error::Layout(format!(
            "{cols}x{rows} tiles do not fit: the gutter and margins leave {tw:.1}x{th:.1}pt"
        )));
    }
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for row in 0..rows {
        for col in 0..cols {
            let x = area.x + f64::from(col) * (tw + gutter);
            let y = area.y + area.h - f64::from(row + 1) * th - f64::from(row) * gutter;
            out.push(Tile { x, y, w: tw, h: th });
        }
    }
    Ok(out)
}

/// Plan every side of every sheet.
pub fn plan(spec: &Plan, pages: usize) -> Result<Vec<Side>, Error> {
    if pages == 0 {
        return Err(Error::Layout("the input has no pages".into()));
    }
    let grid = tiles(spec.area, spec.cols, spec.rows, spec.gutter)?;
    let per_side = grid.len();
    let plan = spec;

    match plan.kind {
        Kind::Nup => {
            // Every side consumes its own block of pages; a duplex sheet is two
            // sides, front then back, not the same block twice.
            let total_sides = pages.div_ceil(per_side);
            let mut out = Vec::with_capacity(total_sides);
            for side_index in 0..total_sides {
                let back = plan.duplex && side_index % 2 == 1;
                let sheet = if plan.duplex {
                    side_index / 2
                } else {
                    side_index
                };
                let base = side_index * per_side;
                let placements = grid
                    .iter()
                    .enumerate()
                    .map(|(index, tile)| {
                        let cell = if back && plan.mirror_back {
                            let row = index / plan.cols as usize;
                            let col = index % plan.cols as usize;
                            row * plan.cols as usize + (plan.cols as usize - 1 - col)
                        } else {
                            index
                        };
                        let page = base + cell;
                        Placement {
                            page: (page < pages).then_some(page),
                            tile: *tile,
                            dx: 0.0,
                            dy: 0.0,
                        }
                    })
                    .collect();
                out.push(Side {
                    sheet,
                    back,
                    placements,
                });
            }
            Ok(out)
        }
        Kind::Booklet => {
            if plan.cols != 2 || plan.rows != 1 {
                return Err(Error::Layout(
                    "a saddle-stitch booklet is two pages across one sheet side".into(),
                ));
            }
            let padded = pages.div_ceil(4) * 4;
            let sheets = padded / 4;
            let mut out = Vec::with_capacity(sheets * 2);
            for sheet in 0..sheets {
                let creep = plan.creep * sheet as f64;
                let front = [(padded - 1 - 2 * sheet, creep), (2 * sheet, -creep)];
                let back = [(2 * sheet + 1, creep), (padded - 2 - 2 * sheet, -creep)];
                for (is_back, pair) in [(false, front), (true, back)] {
                    let placements = pair
                        .iter()
                        .zip(grid.iter())
                        .map(|((page, dx), tile)| Placement {
                            page: (*page < pages).then_some(*page),
                            tile: *tile,
                            dx: *dx,
                            dy: 0.0,
                        })
                        .collect();
                    out.push(Side {
                        sheet,
                        back: is_back,
                        placements,
                    });
                }
            }
            Ok(out)
        }
        Kind::Step => {
            let copies = plan.copies.max(1);
            let sides_per_sheet = if plan.duplex { 2 } else { 1 };
            let mut out = Vec::new();
            for sheet in 0..copies as usize {
                for back in [false, true].into_iter().take(sides_per_sheet) {
                    let shift = if back { per_side } else { 0 };
                    let placements = grid
                        .iter()
                        .enumerate()
                        .map(|(index, tile)| Placement {
                            page: Some((index + shift) % pages),
                            tile: *tile,
                            dx: 0.0,
                            dy: 0.0,
                        })
                        .collect();
                    out.push(Side {
                        sheet,
                        back,
                        placements,
                    });
                }
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 842.0, 595.0)
    }

    fn booklet(pages: usize, creep: f64) -> Vec<Side> {
        let spec = Plan {
            kind: Kind::Booklet,
            cols: 2,
            rows: 1,
            area: area(),
            gutter: 0.0,
            duplex: true,
            mirror_back: false,
            creep,
            copies: 1,
        };
        super::plan(&spec, pages).unwrap()
    }

    #[test]
    fn booklet_orders_eight_pages() {
        let sides = booklet(8, 0.0);
        assert_eq!(sides.len(), 4);
        let pages: Vec<Vec<Option<usize>>> = sides
            .iter()
            .map(|side| side.placements.iter().map(|p| p.page).collect())
            .collect();
        assert_eq!(pages[0], vec![Some(7), Some(0)]);
        assert_eq!(pages[1], vec![Some(1), Some(6)]);
        assert_eq!(pages[2], vec![Some(5), Some(2)]);
        assert_eq!(pages[3], vec![Some(3), Some(4)]);
    }

    #[test]
    fn booklet_pads_sparse_input() {
        let sides = booklet(6, 0.0);
        assert_eq!(sides.len(), 4);
        assert_eq!(sides[0].placements[0].page, None);
        assert_eq!(sides[0].placements[1].page, Some(0));
    }

    #[test]
    fn creep_moves_inward_per_sheet() {
        let sides = booklet(16, 1.0);
        assert_eq!(sides[0].placements[0].dx, 0.0);
        assert_eq!(sides[2].placements[0].dx, 1.0);
        assert_eq!(sides[2].placements[1].dx, -1.0);
        assert_eq!(sides[6].placements[0].dx, 3.0);
    }

    #[test]
    fn nup_fills_row_major_from_the_top() {
        let plan = Plan {
            kind: Kind::Nup,
            cols: 2,
            rows: 2,
            area: area(),
            gutter: 10.0,
            duplex: false,
            mirror_back: false,
            creep: 0.0,
            copies: 1,
        };
        let sides = super::plan(&plan, 5).unwrap();
        assert_eq!(sides.len(), 2);
        let first = &sides[0].placements;
        assert_eq!(first[0].page, Some(0));
        assert!(first[0].tile.y > first[2].tile.y, "row 0 sits above row 1");
        assert_eq!(sides[1].placements[1].page, None);
    }

    #[test]
    fn step_repeats_the_block() {
        let plan = Plan {
            kind: Kind::Step,
            cols: 2,
            rows: 2,
            area: area(),
            gutter: 0.0,
            duplex: false,
            mirror_back: false,
            creep: 0.0,
            copies: 1,
        };
        let sides = super::plan(&plan, 2).unwrap();
        let pages: Vec<Option<usize>> = sides[0].placements.iter().map(|p| p.page).collect();
        assert_eq!(pages, vec![Some(0), Some(1), Some(0), Some(1)]);
    }

    #[test]
    fn a_grid_that_cannot_fit_is_refused() {
        let plan = Plan {
            kind: Kind::Nup,
            cols: 8,
            rows: 8,
            area: Rect::new(0.0, 0.0, 100.0, 100.0),
            gutter: 20.0,
            duplex: false,
            mirror_back: false,
            creep: 0.0,
            copies: 1,
        };
        assert!(super::plan(&plan, 8).is_err());
    }
}
