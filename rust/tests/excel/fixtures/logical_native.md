# NOT text: native locale boundary

`logical_text_native.json` preserves all 68 observations from
`rust/target/excel-desktop/p5-logical-text-boundary-observed/results.json`
(SHA-256 `39f9f36d093871d1c71f9f34ede11430a22a7c8e242aba4f35f81178231887e1`).
The inputs are the two hashed workbooks in
`.scratch/p5-logical-text-boundary-inputs/cases.json` (manifest SHA-256
`581a8c114becfc9987f71bb5e71b8645b9a6e44251e98055427451f5944266a3`).
The runner reports Excel 16.0 build 20430.0, UI language 1036 (French),
`passed=true`, `cleanup_completed=true`, all 68 cases observed, and both
pre-save and post-save COM values equal to saved XML cache type/value.
The fixture preserves each Formula, Formula2, Value2, and XML cache separately.
It is native execution evidence, not a Rust equivalence claim.

For each date system, the same 17 source texts were tested as a literal
argument and a text-cell reference to `NOT`. `VRAI` yields saved Boolean false
and `FAUX` saved Boolean true in both origins and epochs: 8 Boolean caches.
The other 60 caches are typed error `#VALUE!`: English TRUE/FALSE in upper,
lower, mixed case and selected whitespace forms, text `1`/`0`, `abc`, and
the empty string from a source formula. These observations establish the
current French-host behavior for those spellings only. They do not establish
an en-US workbook formula rule, nor a rule for untested text.

The Rust logical intake holds `Operand::Text`, retaining the previous cache
with `Unevaluated::Coercion`. The public test checks all 68 text cases remain
held. `Operand::logical` computes Blank=false, Number `!=0`, Boolean identity,
and propagates Error. The 72-case `logical_native.json` run separately records the
numeric/Boolean/blank/reference/error subset as 40 native caches;
18 text cases need locale context, and 14 IF/AND/OR controls belong to later
implementation. The Rust TRUE/FALSE/NOT fixture test now matches all 40
numeric/Boolean/blank/reference/error cases and proves the 18 locale-dependent
text cases retain their caches. Two additional controls prove incremental
updates and wrong-arity refusal. Generic expression `Term::Not` remains unchanged.

## Narrow secondary oracle

The existing `scripts/check_excel_desktop.py::evaluate_function` invokes
**one owned worksheet's** `Evaluate` dispatcher with LCID 1033. Its existing
ERROR.TYPE test proves that this dispatch spelling succeeds on the French
host where LCID 0 returned `#NAME?`; it does not prove that LCID 1033 changes
Boolean text coercion in authored workbook formulas. A secondary probe can
call that already-resolved dispatcher on literal-only expressions
`NOT("TRUE")`, `NOT("FALSE")`, `NOT("VRAI")`, `NOT("FAUX")`, and `NOT(0)`,
recording raw COM variants/errors under LCID 1033 and LCID 0 in the same
owned worksheet. Direct literals avoid source-cell or cached-result state.
This probe needs no `Application.Calculate`, application preference change,
new Excel process, or workbook write. The owner should retain the active
Excel guard, before/after preferences, and cleanup report.

If the two LCIDs disagree, that proves an `Evaluate` dispatch boundary only;
saved OOXML formulas still require a separate en-US-host workbook observation
before assigning their text semantics. If they agree, LCID 1033 does not
resolve this locale gap. Either outcome leaves the initial Rust Text hold
unchanged until formula-locale context has a typed owner.
