# Reference, array, and intersection native inputs

The manifest contains 38 authored formula cells: 19 identical shapes in each
1900/1904 workbook. `Values!A1:C5` contains exact, distinct source numbers;
`CycleShape!A1=1`, `B1=@A:A`, `A2=B1+1` isolates the false-cycle question.
Cases includes `SUM(range+1)` with and without grouping, direct single-cell
arithmetic controls, explicit `_xlfn.SINGLE` and authored `@` on a full column
and rectangle at intersecting host coordinates, bounded unmarked range and
rectangle formulas, direct range arithmetic, and ABS/ROUND over both ranges
and single cells. Potential five-row/three-column spills have disjoint areas;
the authored sheets have no cached formula answers. The harness is to record
both `Range.Formula` and `Range.Formula2` before and after SaveAs, plus the
saved XML formula attributes/cache. A Formula-only observation cannot decide
whether Excel chose implicit intersection or array evaluation.

Input checks completed without Excel: the existing function-manifest reader
accepted all 38 cases and both workbook hashes; ZIP formula text and empty
caches were checked against every manifest coordinate; openpyxl reopened the
source values, formulas, and date systems. No numeric result is asserted.

From repository root, after applying the Formula2 harness patches:

```powershell
python/.venv/Scripts/python.exe scripts/check_excel_desktop.py functions --cases .scratch/p5-ref-array-inputs/cases.json --output-dir rust/target/excel-desktop/p5-ref-array-observed --active
```

Use a fresh output directory. Input manifest SHA-256:
`a161b403c64dbace2fcbf4266a109ff5ad71b90198c8f382082af95a0e33874e`.

## Native observation (Excel 16.0 build 20430.0)

The owned run `rust/target/excel-desktop/p5-ref-array-observed/results.json`
passed all 38 cases with cleanup. Every before/after COM value agrees with its
saved cache. The durable `reference_array_native.json` retains the input and
saved workbook SHA-256 hashes, raw authored formula, native `Formula` and
`Formula2` before and after SaveAs, typed `Value2`, error identity, saved
formula attributes, and raw cached value. The report SHA-256 is
`d1cc6249b8f7f2c88eaa4d9dab4d809195e6550cba8513ab33face1fbd1847b1`;
the compact fixture SHA-256 is
`8be6b22c1f88532f35e07f33f82606220fa1fedbb3946e84379740725ba10884`.
The paired 1900/1904 cases have identical native results and formula spellings.
This run did not author any expression through the `Formula2` COM setter and
does not establish modern array-evaluation behavior.

The authored plain `SUM(Values!A1:A5+1)` and
`SUM((Values!A1:A5)+1)` return `#VALUE!` at Cases B40/B41. Native
`Formula2` displays `=SUM(@Values!A1:A5+1)` and
`=SUM((@Values!A1:A5)+1)` respectively. In contrast,
`SUM(Values!A1:A5)` retains a range argument in both formula views and
returns 77.5. In the saved OOXML, all three `<f>` texts are unmarked and
their formula attributes are `{}`. These observations show Excel's treatment
of this authored legacy workbook; they do not prove that a Formula2-authored
array expression has the same result.

The bounded unmarked direct range, rectangle, range arithmetic, ABS(range),
and ROUND(range) probes are anchored at row 20, outside Values rows 1–5.
`Formula2` shows inserted `@` at the range operand and each returns `#VALUE!`
with saved `t="e"` cache. No spill was observed. Single-cell arithmetic and
ABS/ROUND controls return numeric caches. There is no unmarked full-column
probe: it could attempt a million-row spill.

At Cases B3, `_xlfn.SINGLE(Values!A:A)` returns Values A3 = 33; at C3,
`_xlfn.SINGLE(Values!A1:C5)` returns Values C3 = 1003. Authored `@` controls
at B4 and C4 return Values A4 = -44 and C4 = 1004. Native `Formula2` has `@`
for each; `Formula` and saved `<f>` omit it. The 2-D controls establish row
*and* column selection at these host coordinates, not a rule for every
nonintersecting shape.

On CycleShape, the authored `A1=1`, `B1=@A:A`, `A2=B1+1` produces B1 = 1
and A2 = 2 in both systems. Thus B1 depends on the intersecting A1, not all
of column A; a dependency edge to A2 would invent a cycle. Native `Formula2`
preserves `=@A:A`; `Formula` and saved `<f>` are `=A:A` and `A:A`, with no
formula attributes. A workbook dependency policy must account for implicit
intersection even where saved formula text lacks the marker.

Regenerate the fixture without COM or Cargo:

```powershell
python .scratch/build-p5-ref-array-evidence.py
```
