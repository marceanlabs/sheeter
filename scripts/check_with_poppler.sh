#!/usr/bin/env bash
# Independent check of an imposed file, with tools that did not write it.
#
#   scripts/check_with_poppler.sh <source.pdf> <imposed.pdf> [report.csv]
#
# Fails on: a poppler syntax warning, a qpdf complaint, text that did not carry,
# or (when a placement report is given) a source page not placed exactly once.
set -euo pipefail

source_pdf=${1:?usage: check_with_poppler.sh <source.pdf> <imposed.pdf> [report.csv]}
imposed=${2:?usage: check_with_poppler.sh <source.pdf> <imposed.pdf> [report.csv]}
report=${3:-}

fail=0

for file in "$source_pdf" "$imposed"; do
  if pdfinfo "$file" 2>&1 | grep -qi 'Syntax Warning'; then
    echo "FAIL syntax: poppler warns about $file"
    pdfinfo "$file" 2>&1 | grep -i 'Syntax Warning' | head -5
    fail=1
  fi
done

if ! qpdf --check "$imposed" >/dev/null 2>&1; then
  echo "FAIL structure: qpdf rejects $imposed"
  qpdf --check "$imposed" | tail -5
  fail=1
fi

# Characters, not words: scaling changes where pdftotext decides a word ended, so
# a word count moves by a few on any imposition. A dropped page moves far more.
src_chars=$(pdftotext "$source_pdf" - | tr -d '[:space:]' | wc -c)
out_chars=$(pdftotext "$imposed" - | tr -d '[:space:]' | wc -c)
tolerance=$(( src_chars / 1000 ))
delta=$(( src_chars - out_chars ))
if [ "${delta#-}" -gt "$tolerance" ]; then
  echo "FAIL content: source has $src_chars characters, imposed has $out_chars"
  fail=1
fi

if [ -n "$report" ]; then
  pages=$(pdfinfo "$source_pdf" | awk '/^Pages:/ {print $2}')
  placed=$(awk -F, 'NR>1 && $4 != "blank" {print $4}' "$report" | sort -n)
  unique=$(echo "$placed" | sort -nu | wc -l)
  dupes=$(echo "$placed" | sort -n | uniq -d | head -3 | tr '\n' ' ')
  if [ "$unique" -ne "$pages" ]; then
    echo "FAIL coverage: source has $pages pages, the report places $unique distinct"
    fail=1
  fi
  if [ -n "$dupes" ]; then
    echo "FAIL coverage: pages placed more than once: $dupes"
    fail=1
  fi
fi

src_pages=$(pdfinfo "$source_pdf" | awk '/^Pages:/ {print $2}')
out_pages=$(pdfinfo "$imposed" | awk '/^Pages:/ {print $2}')

if [ "$fail" -eq 0 ]; then
  echo "ok: $src_pages pages in, $out_pages sides out, $src_chars characters carried, no complaints"
fi
exit "$fail"
