# Explicit en-US VALUE/TEXT observations

The fixture retains162 LCID1033 Worksheet.Evaluate answers from Excel16 build20430,
both date systems. Both guarded runs passed and restored the attached application.
The original formula strings are unchanged. Native localized before/after-SaveAs
caches and saved XML remain separate fields, not substituted en-US answers.
Rust replay compares the explicit en-US answers only; this does not repair or
upgrade the failed full324 cached-English-TEXT gate.

The fixed en-US contract accepts currency and m/d/yyyy for VALUE; TEXT interprets
General, Red, y/d date tokens and Boolean text under English. General and @ use
the existing eleven-character FormatCode display rule, not formula implicit
number-to-text's twenty-character budget. Empty TEXT format returns empty;
invalid codes/no matching numeric section are #VALUE!, and fill repetition has
no cell width to consume. Negative date behavior follows the workbook epoch.

The current-year abbreviated date `1/2` and clocks with more than three fraction
digits retain explicit held results until the shared Entry owner models them.
Their observed answers remain intact. Boolean format-code coercion stays held;
Boolean input values have explicit English answers. Native French cached values
remain available to verify that this is a scoped LCID oracle, not cache equality.

Source contract: handoff/design.md entry/en-US and TEXT->FormatCode ownership.
https://support.microsoft.com/en-us/excel/functions/value-function
https://support.microsoft.com/en-us/excel/functions/text-function
