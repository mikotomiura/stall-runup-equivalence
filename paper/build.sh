#!/usr/bin/env bash
# Build the manuscript with pdfLaTeX and BibTeX.
#
#   bash build.sh <template-dir> <out-dir>
#
# <template-dir> holds jedm.cls and acmtrans.bst of the journal's template (BUILD.md says where to obtain it).
# <out-dir> receives main.pdf and the logs; it must lie outside the directory of this article.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: bash build.sh <template-dir> <out-dir>" >&2
  exit 2
fi
here=$(cd "$(dirname "$0")" && pwd -P)
root=$(cd "$here/.." && pwd -P)
template=$(cd "$1" && pwd -P)
if [ ! -f "$template/jedm.cls" ] || [ ! -f "$template/acmtrans.bst" ]; then
  echo "build.sh: ${template} does not hold jedm.cls and acmtrans.bst" >&2
  exit 2
fi
mkdir -p "$2"
out=$(cd "$2" && pwd -P)
case "$out/" in
  "$root"/*)
    echo "build.sh: ${out} lies inside ${root}; give an output directory outside it" >&2
    exit 2 ;;
esac

export TEXINPUTS="${template}:"
export BSTINPUTS="${template}:"
export BIBINPUTS="${here}:"

latex() {
  if ! pdflatex -no-shell-escape -interaction=nonstopmode -halt-on-error -output-directory="$out" main.tex \
      > "$out/main.run.log"; then
    echo "build.sh: pdflatex failed; see ${out}/main.run.log" >&2
    exit 1
  fi
}

cd "$here"
latex
# BibTeX exits with 1 when it only warns; the warnings are counted below.
(cd "$out" && bibtex main > main.bibtex.log) || [ $? -eq 1 ] || {
  echo "build.sh: bibtex failed; see ${out}/main.blg" >&2
  exit 1
}
latex
latex

log="$out/main.log"
echo "main: $(grep -c '^!' "$log" || true) errors, $(grep -c 'LaTeX Warning: Reference' "$log" || true) undefined references, $(grep -c 'Warning: Citation' "$log" || true) undefined citations, $(grep -c '^Overfull' "$log" || true) overfull boxes"
echo "bibtex: $(grep -c '^Warning--' "$out/main.blg" || true) warnings"
