# Mapped array native observations

The retained 210 literal/wrapped observations are from literal_arrays_native.json.
The additional 48 observations are the successful owned-workbook mapper run
on Excel 16.0 build 20430.0, both date systems; cleanup completed. The JSON
retains the report hash, before/after SaveAs observations and saved XML caches.
No reopen observation is claimed.

The scalar-operation stage selects 54 observations and replays authored and saved
formula text (108 exact comparisons). It covers scalar capture, singleton-axis
broadcast, missing-axis #N/A, signs, percent, arithmetic, equality, ABS and ROUND.
The remaining observations stay recorded without being called proven by this
stage. Unprefixed IFNA/CONCAT input controls produced #NAME?; they do not establish
the registered functions' array semantics. Literal arrays and their ordinary
operations are separate from held CSE/data-table/cm formula anchors.
