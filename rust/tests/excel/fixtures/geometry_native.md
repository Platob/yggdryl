# Native reference geometry scope

Excel 16.0 build 20430.0, French UI 1036/country 33. Seven changed-input
partitions passed with cleanup true. They contain 150 unique observations in
the 1900/1904 systems, each equal before saving, after SaveAs in the same open workbook and in the
saved XML cache. Repeated source controls are stored once. JSON retains the
exact report and manifest hashes, raw COM observations, cached values and
the failed input provenance.

ROW/COLUMN return the first row/column of a genuine reference, including a
reversed rectangle, full axes, empty/error source cells and selected IF/CHOOSE
references. With no argument they return the consuming cell's coordinates.
ROWS/COLUMNS count reference dimensions; scalar typed values have shape 1x1,
errors propagate, and constant arrays supply their parser-proved dimensions.
Every tested 3-D argument returns #VALUE!. Explicit SINGLE/@ narrows a
reference to the host's intersection before its geometry is consumed, or
returns #VALUE! when the host cannot intersect it.

The original 174-case workbook and its ROW/COLUMN scalar/array partitions
failed Workbooks.Open. Those failures provide no cached function answers and
remain separate from this fixture. Direct scalar/array ROW/COLUMN inputs stay
uncomputed pending valid isolated observations. This fixture establishes no
dynamic-array spill, CSE, multi-area union or arbitrary array calculation.

Reference geometry is independent of stored cell values: it must not read a
referenced error, introduce self-dependency, or scan a full row/column. The
constant-array dimensions refer to the existing AST; no array values or blank
cell grid are materialized.

Primary syntax/shape references:
https://support.microsoft.com/en-us/excel/functions/row-function
https://support.microsoft.com/en-us/excel/functions/column-function
https://support.microsoft.com/en-us/excel/functions/rows-function
https://support.microsoft.com/en-us/excel/functions/columns-function
