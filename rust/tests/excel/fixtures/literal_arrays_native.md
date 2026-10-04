# Literal-array native observations

Excel 16.0 build 20430.0, French UI 1036, both date systems. The two guarded
runs observed 162 literal/aggregate and 48 wrapped-array cases. All 210 native
before/after-SaveAs observations agree with the saved XML caches; cleanup
completed. This is a same-open-workbook SaveAs observation, not a reopened
comparison. Report hashes, input workbook proofs, Formula/Formula2, and all
original caches are retained in the JSON.

The first Rust slice selects 134 leaf/publication/aggregate observations and
replays their authored and saved spellings. Literal arrays borrow constants
from the one parsed expression arena, retain array origin through names and
selectors, and enter the existing aggregate visitor in row-major order.
Array text and Boolean coercion is distinct from worksheet-cell coercion:
AVERAGEA counts array text as zero but ignores array Booleans; MINA/MAXA ignore
both. GCD/LCM accept numeric array text and reject array Booleans.

The original leaf slice left the following observations for later stages: eight numeric variance modes per epoch exceed the already documented
exact variance domain; transformed arrays, legacy CONCATENATE scalarization,
SUMPRODUCT arrays, and reference union/intersection need their own execution
stages. Native wrapping distinguishes SUM({1,2}+1)=5 from
SUM(IF(TRUE,{1,2},{3,4})+1)=2, so intermediate arrays must not be flattened
indiscriminately or lifted across every selector boundary.

Primary grammar and publication contract:
https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/c45b0396-bc38-4fd6-abf7-9782b7d6f926
https://learn.microsoft.com/en-us/office/vba/excel/concepts/cells-and-ranges/range-formula-vs-formula2

Subsequent mapped/selector/SUMPRODUCT tests use mapped_arrays_native.json,
which retains these210 observations and48 further native mapper controls.
Those separately named tests prove their selected subsets; the original
initial_leaf_slice flag remains historical rather than silently widened.
