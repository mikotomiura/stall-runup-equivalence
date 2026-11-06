# Building the manuscript

The manuscript uses the LaTeX template of the Journal of Educational Data Mining (class `jedm`, bibliography style
`acmtrans`). The template is not part of this directory. Obtain the archive `2024_08_02_jedm-latex.zip` from the
journal's Information for Authors page (<https://jedm.educationaldatamining.org/index.php/JEDM/information/authors>)
and unpack it; the directory must hold `jedm.cls` and `acmtrans.bst`.

```
bash build.sh <template-dir> <out-dir>
```

`<out-dir>` receives `main.pdf` and the logs and must lie outside this directory. The script runs pdfLaTeX without shell
escape, then BibTeX, then pdfLaTeX twice, and reports the number of errors, undefined references and citations, and
overfull boxes. Tested with TeX Live 2025 (MacTeX).

The word count of the main text (from the introduction to the end of the conclusion, with headings, captions and
footnotes; without the contents of tables and figures, formulas, the abstract, the declaration, the references and the
appendix) is measured with

```
texcount -inc -sum -merge sections/introduction.tex sections/related.tex sections/data.tex sections/design.tex \
  sections/results.tex sections/discussion.tex sections/limitations.tex sections/conclusion.tex
```
