# sheeter notes

Impose PDF pages onto press-ready sheets. Rust, lopdf, no unsafe.

## Layout

- `src/sheet.rs` named and measured sheet sizes, the `Rect` type.
- `src/units.rs` length parsing (millimetres by default) and PDF number formatting.
- `src/layout.rs` the planner: which source page goes in which tile on which side.
- `src/pdf.rs` the renderer: source pages become Form XObjects placed by a matrix.
- `src/marks.rs` crop lines and registration targets, as content stream fragments.
- `tests/impose.rs` end to end, reads the output back to prove ordering.

## Decisions worth keeping

- **Rebuild, do not edit.** The output is a new document. Only objects a placed
  page can reach are copied in, once each, so nothing points into a document that
  no longer exists and no dead objects are carried over.
- **One form per source page.** The page's content becomes a Form XObject with
  its crop box normalised to the origin. Rotation is applied in the placement
  matrix, never by rewriting content.
- **Marks live in the margin.** If the margin is too small for a 3 mm mark the
  job fails with the number to use, because marks drawn at margin 0 disappear off
  the sheet edge and look like a bug.
- **The report is the proof.** `--report` writes sheet/side/slot/source, which is
  how `scripts/check_with_poppler.sh` proves every page landed exactly once.

## Known limits

- Booklet is two-up only. Four-up booklets are a different imposition and are
  refused rather than approximated.
- No preflight, no spot colour, no PDF/X.
- Source pages with a `/Rotate` are handled in the matrix; a page whose content
  relies on `/UserUnit` is not.

## Next, if it earns it

- `sheeter watch` has no retry queue: a file that fails is reported and skipped.
- Price and packaging are unproven. See `/root/work/rust-tool-hunt/shortlist.md`.
