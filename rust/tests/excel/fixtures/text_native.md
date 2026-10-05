# Native text observations

`text_native.json` retains all 564 cases from Excel 16.0 build 20430.0,
French UI 1036, both workbook date systems. The corrected native collector
observed every case and proved 1,128 exact before/after/saved-cache comparisons.
Cleanup completed. This is native evidence; it makes no Rust-equivalence claim.

The earlier full run remains failed in provenance. Its four collector failures
were the two REPT 32,767-character cells before and after SaveAs. The native
ISERROR transport for these cells was a single-element Boolean tuple. The
corrected collector accepts exactly that shape; it does not change cell values.

Every source formula, native COM observation, saved cache and comparison remains
in the fixture. Native strings containing a lone UTF-16 unit are represented
with `native_utf16_units`, not an invalid JSON surrogate escape, loss, or U+FFFD.
Their saved OOXML spellings remain exact. `utf16_unit_object_paths` enumerates
every such conversion. Ordinary strings stay strings.

The absent workbook compatibility marker is version 1. `unicode_native.json`
separately provides absent/v1/v2 controls; version must come from workbook XML,
never the filename. Microsoft identifies LEN, MID, FIND, SEARCH and REPLACE as
version-sensitive. LEFT/RIGHT preserve whole Unicode scalars in these v1 cases.

Portable reuse requires explicit boundaries: native UPPER retains sharp-s
where Rust Unicode casing expands it; native LOWER of dotted capital I differs
from Rust's full Unicode mapping. Boolean-to-text produced French VRAI/FAUX,
including TEXT(TRUE,"0"). These observations cannot establish an unspecified
workbook locale. They are retained, rather than silently relabeled en-US.

Primary sources read for this slice:
- https://learn.microsoft.com/en-us/openspecs/office_standards/ms-xlsx/294f537a-8085-415a-82ad-979e2912c316
- https://learn.microsoft.com/en-us/openspecs/office_standards/ms-xlsx/d78afeb7-79da-43a4-9282-f3749c27250b
- https://support.microsoft.com/en-gb/excel/compatibility-versions

A separate 40-case LCID1033 native evaluation now proves the Boolean text
spellings used by the en-US core. The JSON retains every original French cache
and adds the independent `en_us` observation; the replay reads that observation
only for the identical formula/source case. These Boolean conversions contain no
date-system operation, so both epoch replays use the same explicit en-US value.
The existing text_en_us_native fixture independently proves TEXT(2.5,TRUE)
returns #VALUE! after TRUE becomes the format text. No native cache was rewritten.
