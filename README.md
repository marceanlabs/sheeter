# sheeter

Impose PDF pages onto press-ready sheets from the command line: n-up grids,
saddle-stitch booklets with creep, and cut stacks, with crop marks and a hot
folder for unattended jobs.

Built for the jobs a small print shop runs over and over: the same booklet every
week, a stack of business cards, a set of tickets. One binary, no Acrobat, no
GUI, no licence server.

```
sheeter run booklet.pdf --layout booklet --sheet a4l --duplex --creep 0.25mm -o sheets.pdf
```

```
15 pages -> 8 sides on 4 sheets -> sheets.pdf
```

## What it does

- **n-up**: pages in reading order, `--cols` by `--rows`, with duplex and
  long-edge mirroring for work-and-turn.
- **booklet**: two-up saddle stitch, page order worked out for you, `--creep`
  shifts each successive sheet inward to compensate for the fold.
- **step** (cut stack): one block of pages repeated across the whole sheet so
  the stack can be guillotined into identical piles.
- **marks**: crop lines at every trim corner, registration targets at the
  sheet corners. Both refuse to run without a margin to draw into, rather than
  printing marks off the edge of the sheet.
- **hot folder**: `sheeter watch` polls a directory and imposes every PDF that
  lands in it.
- **page selection**: `--pages 1-8,12` imposes part of a file, so a job does not
  need a second tool to strip a cover or a blank back.
- **protected files**: `--password` opens an encrypted input; without it, the
  error says the file is protected instead of pretending there are no pages.
- **true size**: `--fit 100%` places pages at one point per point, centred,
  instead of scaling them to fill the tile.
- **placement report**: `--report job.csv` writes which source page went to
  which sheet, side and tile, so a job can be checked without opening the file.
- Job files in TOML, so a shop can keep one per product and hand it to a
  colleague.

## Verified, not claimed

| Claim | How it is checked |
|---|---|
| Page order is correct | `booklet_orders_the_signature`: the output is reopened and each page's own content is read back, so the signature is verified from the file rather than from the code that wrote it |
| Nothing is dropped | `scripts/check_with_poppler.sh` counts characters with `pdftotext` and requires every source page in the placement report exactly once |
| The file is structurally valid | `qpdf --check` and `pdfinfo` report no complaints; every reference in the model is resolved before the file is written |
| Fonts and images survive | tested on a 194-page RFC (different producer, Type1 fonts) and a LaTeX paper (embedded subset fonts), rendered and read back |
| It is fast enough to run unattended | 194 A4 pages to a 98-side saddle-stitch booklet in under 0.3 s on one core |

Measured on two real files from different producers: a 194-page RFC and a
15-page LaTeX paper. In both, `pdftotext` character counts match the source
exactly.

## Install

```
cargo install sheeter
```

## Job files

```toml
sheet = "a4l"        # A4 landscape
layout = "booklet"
cols = 2
rows = 1
duplex = true
margin = "8mm"       # room for the marks
gutter = "6mm"
creep = "0.25mm"
marks = ["crop"]
```

`sheeter watch --job booklet.toml --in incoming --out imposed` then imposes
every PDF dropped into `incoming`.

Lengths are millimetres unless a unit is given (`mm`, `cm`, `in`, `pt`). Sheet
sizes are `A0` to `A6`, `SRA3`, `SRA4`, `letter`, `legal`, `tabloid`, or measured
like `320x450` or `12x18in`.

## What it does not do

- No GUI and no preview. It is a batch tool, and the report CSV is the proof.
- No bleed-to-trim automation beyond `--bleed`, which extends each page past the
  trim line. It does not add bleed to artwork that has none.
- No spot colours, no ink control, no preflight. It imposes pages; it does not
  judge them.
- No PDF/X or PDF/A output.
- No automatic numbering or variable data. It will not add folios, bates stamps
  or ticket numbers; those come in the artwork.
- No imposition of mixed page sizes on one sheet beyond fitting each page to its
  own tile.
- Does not touch the source: it is read, never modified.

## Licence

Apache-2.0.
