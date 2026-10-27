//! Impose PDF pages onto press-ready sheets.
//!
//! One job description drives everything: a sheet size, a layout, and the marks.
//! The binary is a thin caller so the same code can run from a hot folder.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod layout;
pub mod marks;
pub mod pdf;
pub mod sheet;
pub mod units;

use std::path::{Path, PathBuf};

use serde::Deserialize;

use layout::{Kind, Plan};
use marks::MarkStyle;
use sheet::Rect;

/// Everything that can go wrong, in words a print shop can act on.
#[derive(Debug)]
pub enum Error {
    Unit(String),
    Sheet(String),
    Layout(String),
    Pdf(String),
    Config(String),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unit(message) | Self::Sheet(message) | Self::Layout(message) => {
                f.write_str(message)
            }
            Self::Pdf(message) => write!(f, "pdf: {message}"),
            Self::Config(message) => f.write_str(message),
            Self::Io(err) => write!(f, "io: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<lopdf::Error> for Error {
    fn from(err: lopdf::Error) -> Self {
        Self::Pdf(err.to_string())
    }
}

/// A job, as read from a TOML file or built from the command line.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Job {
    /// Sheet size: `A4`, `A3L`, `SRA3`, `320x450`, `12x18in`.
    pub sheet: String,
    /// `nup`, `booklet` or `step`.
    pub layout: String,
    pub cols: u32,
    pub rows: u32,
    /// Space kept clear at every sheet edge.
    pub margin: String,
    /// Space between tiles.
    pub gutter: String,
    /// Content overhang past the trim line.
    pub bleed: String,
    /// Per-sheet inward shift for saddle stitching.
    pub creep: String,
    /// Any of `crop`, `reg`, `none`.
    pub marks: Vec<String>,
    pub duplex: bool,
    /// Mirror back sides left to right for long-edge flipping.
    pub mirror_back: bool,
    /// `fit` keeps the aspect ratio, `fill` stretches to the tile.
    pub fit: String,
    /// Sheets produced by a `step` job.
    pub copies: u32,
}

impl Default for Job {
    fn default() -> Self {
        Self {
            sheet: "A4".into(),
            layout: "nup".into(),
            cols: 2,
            rows: 1,
            margin: "5".into(),
            gutter: "0".into(),
            bleed: "0".into(),
            creep: "0".into(),
            marks: vec!["crop".into()],
            duplex: false,
            mirror_back: false,
            fit: "fit".into(),
            copies: 1,
        }
    }
}

impl Job {
    /// Read a job from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, Error> {
        toml::from_str(text).map_err(|err| Error::Config(format!("job file: {err}")))
    }

    /// Read a job from a file.
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)?;
        Self::from_toml(&text)
    }

    /// Resolve the strings into numbers and a plan.
    pub fn prepare(&self) -> Result<Prepared, Error> {
        let sheet = sheet::parse_sheet(&self.sheet)?;
        let margin = units::parse_len(&self.margin)?;
        let area = sheet.inset(margin);
        if area.w < 10.0 || area.h < 10.0 {
            return Err(Error::Layout(format!(
                "a {:.0}mm margin leaves no printable area on a {} sheet",
                margin / units::MM,
                self.sheet
            )));
        }
        let spec = Plan {
            kind: Kind::parse(&self.layout)?,
            cols: self.cols,
            rows: self.rows,
            area,
            gutter: units::parse_len(&self.gutter)?,
            duplex: self.duplex,
            mirror_back: self.mirror_back,
            creep: units::parse_len(&self.creep)?,
            copies: self.copies,
        };
        let style = marks_from(&self.marks)?;
        // Marks live in the margin. Without room they would be drawn off the
        // sheet and silently vanish, so say so instead.
        if style.crop || style.registration {
            let needed = style.length + style.gap;
            if margin + 0.01 < needed {
                return Err(Error::Layout(format!(
                    "marks are {:.1}mm long but the margin is {:.1}mm; set a margin of at least {:.1}mm or pass --marks none",
                    needed / units::MM,
                    margin / units::MM,
                    needed / units::MM
                )));
            }
        }
        Ok(Prepared {
            sheet,
            area,
            spec,
            style,
            bleed: units::parse_len(&self.bleed)?,
            fill: match self.fit.to_ascii_lowercase().as_str() {
                "fit" | "contain" => false,
                "fill" | "stretch" => true,
                other => {
                    return Err(Error::Layout(format!(
                        "{other:?} is not a fit mode: use fit or fill"
                    )));
                }
            },
        })
    }
}

fn marks_from(names: &[String]) -> Result<MarkStyle, Error> {
    let mut style = MarkStyle::none();
    for name in names {
        match name.trim().to_ascii_lowercase().as_str() {
            "crop" | "cropmarks" | "crop-marks" => style.crop = true,
            "reg" | "registration" => style.registration = true,
            "none" => {}
            other => {
                return Err(Error::Layout(format!(
                    "{other:?} is not a mark: use crop, reg or none"
                )));
            }
        }
    }
    Ok(style)
}

/// A job with its strings resolved.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub sheet: Rect,
    pub area: Rect,
    pub spec: Plan,
    pub style: MarkStyle,
    pub bleed: f64,
    pub fill: bool,
}

/// What a run produced.
#[derive(Debug, Clone)]
pub struct Summary {
    pub pages_in: usize,
    pub sides: usize,
    pub sheets: usize,
    pub written: PathBuf,
}

/// Impose `input` into `output`.
pub fn impose(input: &Path, output: &Path, job: &Job) -> Result<Summary, Error> {
    impose_reporting(input, output, job, None)
}

/// Impose, and optionally write the placement map as CSV so the job can be
/// checked without opening the PDF.
pub fn impose_reporting(
    input: &Path,
    output: &Path,
    job: &Job,
    report: Option<&Path>,
) -> Result<Summary, Error> {
    let prepared = job.prepare()?;
    let source = lopdf::Document::load(input)
        .map_err(|err| Error::Pdf(format!("could not read {}: {err}", input.display())))?;

    let sides;
    let pages_in;
    {
        let imposer = pdf::Imposer::new(&source);
        pages_in = imposer.page_count();
        sides = layout::plan(&prepared.spec, pages_in)?;
        imposer.write(&sides, &prepared, output)?;
    }

    if let Some(path) = report {
        let mut text = String::from("sheet,side,slot,source_page,x_pt,y_pt,w_pt,h_pt\n");
        for side in &sides {
            for (slot, placement) in side.placements.iter().enumerate() {
                let page = placement
                    .page
                    .map_or_else(|| "blank".to_string(), |index| (index + 1).to_string());
                text.push_str(&format!(
                    "{},{},{},{},{:.2},{:.2},{:.2},{:.2}\n",
                    side.sheet + 1,
                    if side.back { "back" } else { "front" },
                    slot + 1,
                    page,
                    placement.tile.x,
                    placement.tile.y,
                    placement.tile.w,
                    placement.tile.h
                ));
            }
        }
        std::fs::write(path, text)?;
    }

    let sheets = sides
        .iter()
        .map(|side| side.sheet)
        .max()
        .map_or(0, |v| v + 1);
    Ok(Summary {
        pages_in,
        sides: sides.len(),
        sheets,
        written: output.to_path_buf(),
    })
}

/// Default output name beside the input: `job.pdf` becomes `job-sheets.pdf`.
#[must_use]
pub fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".into());
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}-sheets.pdf"))
}
