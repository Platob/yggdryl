# Remaining text native observations

All 242 cases were accepted, calculated and saved by Excel16 build20430 under
French UI1036. Cleanup succeeded; all484 before/after-SaveAs versus XML-cache
comparisons agree. This is not an en-US or Rust conformance claim. The fixture
retains the full native observations, input manifest hash and source cells.

PROPER demonstrates letter-based word boundaries: punctuation, digits and the
combining accent separate words; uncased CJK does not. ASCII letter conversion
and uncased characters are supported independently of locale. Non-ASCII cased
characters and Boolean-to-text conversions remain held under the current core.

CHAR/CODE are explicitly computer-code-page dependent in Microsoft’s contract.
ASCII1..127 and errors outside1..255 are portable; extended bytes/characters are
recorded, not silently assigned a workbook-wide Windows-1252 policy. Native
unassigned CP1252 byte positions map to C1 controls; nonrepresentable scalars
map to63 here. The existing charset owner remains the mapping implementation.

VALUE/TEXT include locale-sensitive observations that cannot be equality gates
for the design’s en-US contract. This native VALUE rejects `$42` and interprets
`1/2/2024` as February1; TEXT sees `General`/[Red] as invalid and y/d as literals.
Both successful and refused observations remain in the fixture. No locale is
inferred from the workbook filename and no cache is rewritten.

Primary contracts:
- https://support.microsoft.com/en-us/excel/functions/char-function
- https://support.microsoft.com/en-us/excel/functions/code-function
- https://support.microsoft.com/en-us/excel/functions/proper-function
- https://support.microsoft.com/en-us/excel/functions/value-function
- https://support.microsoft.com/en-us/excel/functions/text-function
