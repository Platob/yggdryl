# Contract addendum (decisions taken by the foreground; binding on the core, service and UI)

These refine the design where later implementation and native evidence resolve an ambiguity. The affected core phase, service and UI implement the same contract.

1. **`activeSheet`** in `GET workbook` is a sheet **key** (the `key` of an entry in `sheets`), never an index.
2. **Widths.** Every width the API states or takes (layout `defaults.columnWidth`, column runs, `columnWidth` op `size`) is in the file's `<col width>` units, padding included. When a sheet states no `defaultColWidth`, the service answers `9.140625` (64 px at `maxDigitWidth` 7). Pixel conversion is design §4.2's formula.
3. **Fill character.** A tile cell's `fill` is `[char, offset]`: `offset` is the character index in `t` at which the repeated character is inserted (so `"$"* #,##0.00` pads after the `$`). `Rendered::fill` in the core is `Option<(char, usize)>` accordingly (P3).
4. **Indices.** `at`, `start`, `first`, `last` and every run index are **0-based** integers; `ref`/`range`/`ranges` are A1 text (`"B2"`, `"A1:C3"`), `sheet` is a sheet key.
5. **`setStyle` patch spellings** (the enum names of ECMA-376, camelCase as the schema spells them):
   - `underline`: `none | single | double | singleAccounting | doubleAccounting`
   - `borders.preset`: `bottom | top | left | right | all | outside | thickOutside | none`; `borders.style`: `thin | medium | thick | dashed | dotted | double | hair | mediumDashed | dashDot | mediumDashDot | dashDotDot | mediumDashDotDot | slantDashDot`
   - `horizontal`: `general | left | center | right | fill | justify | centerContinuous | distributed`; `vertical`: `top | center | bottom | justify | distributed`
   - colours (`fontColor`, `fill`, `borders.color`) are `"#RRGGBB"`; `null` means automatic (font), no fill (fill), automatic (border)
   - `numberFormat` is a format **code** (`"0.00%"`); a code equal to an ECMA-376 §18.8.30 built-in's en-US code (ids 0–22, 37–40, 45–49) interns to that built-in id, anything else to a declared custom id
   - `decimals`: an integer delta (−15…15) applied per distinct source style through `FormatCode::with_decimals`; a patch carries `decimals` or `numberFormat`, not both (400 otherwise)
6. **`fillEntry`** carries `ref`, the active cell: `{op:"fillEntry", sheet, ranges, text, ref}`. The text is entered at `ref` and every other cell of the ranges receives the same formula shape, so relative references translate from `ref` exactly as Excel's Ctrl+Enter does.
7. **Change feed.** `GET changes?since=N&wait=25` (design §8.3/§8.6) is consumed by `js/sync.js`: one Web Locks leader per browser holds the long poll and relays over `BroadcastChannel("yggdryl-excel")`; followers never poll.
8. **Copy marker names the service instance.** `GET workbook` answers `"instance"`: a UUIDv7 the `Service` draws once when it is constructed (the crate's own UUID generation; no new dependency). The `data-yggdryl-copy` marker is `{instance}:{generation}:{revision}:{key}:{range}`; a paste whose instance differs from the current one is a `pasteText`.
9. **Number-format preview.** `GET format?code=<code>&sheet=<key>&ref=<A1>` answers `{"text":"$1,234.50","color":"#FF0000"|null,"fill":[char,offset]|null}`: the code rendered through `FormatCode::render` against that cell's value (a blank cell renders `1234.5` as the sample). An unparsable code is 400 naming the byte. The Format Cells dialog previews through it.
10. **Find next.** `GET find?text=<text>&scope=sheet|workbook&sheet=<key>&from=<A1>&in=values|formulas&matchCase=true|false&entireCell=true|false` answers the next match `{"sheet":1,"ref":"C9"}`, or `null` when there is none. It replaces §8.3's `find` row (`q`, `after`, `case`, `whole` and the `{"match":…}` wrapper are gone).
   - The options are the `replace` op's (§8.4), spelled as query parameters: `text` is the op's `find`, and `scope`, `sheet`, `in`, `matchCase` and `entireCell` mean what they mean there. Both go through `Workbook::find`'s `FindOptions` and the wildcard matcher of §4.3 (`* ? ~`), so Find Next stops exactly on the cells Replace All would change.
   - `text` is required and non-empty. `scope` defaults to `sheet`, `in` to `values`, `matchCase` and `entireCell` to `false`; `entireCell=false` matches anywhere in the cell's text.
   - `sheet` is required: the worksheet the search starts on. `scope=sheet` searches that worksheet alone; `scope=workbook` continues through the following sheets in workbook order and wraps to the first, skipping chartsheets and every sheet whose `state` is not `visible`.
   - `from` is exclusive: the search starts at the cell after it, by rows (left to right, then down), and wraps round to end at `from` itself, so a lone match answers again. Without `from` the search starts at A1 of `sheet`, A1 included.
   - `in=values` matches the text the cell shows (its tile `text`); `in=formulas` matches its entry (`cells/{ref}`'s `entry`). A blank cell never matches; a merged range matches only at its anchor. Hidden rows and columns are searched like any others.
   - Refusals: 400 `invalid_record` naming the parameter for an empty or missing `text`, an unknown `scope` or `in`, a `matchCase`/`entireCell` other than `true`/`false`, or a `from` that is not a cell reference; 404 for a `sheet` that names no worksheet.
11. **One spelling for the searched text.** The `replace` op spells the searched text `text` (not design §8.4's `find`): `{op:"replace", scope, sheet, text, replacement, in, matchCase, entireCell}`, the same names and meanings as `GET find`'s query (item 10).
12. **Automatic colours stay automatic in the styles payload.** In `GET styles`, a font or border whose colour is automatic answers `"color": null`, and a cell with no fill answers `"fill": null`; only explicit colours are resolved to `"#RRGGBB"` (theme and indexed colours with tint resolved through `StyleSheet::resolve`). The UI maps `null` onto the theme at render time (light/dark mode, "Dark cells"), so the server never guesses the viewer's scheme.
13. **Formula assist.** `POST formula/assist` (design §8.3) with `action: tokens | cycle | reference` is the one assist endpoint (no `formula/cycle-reference`). `caret`, `start` and `end` are **UTF-16 code-unit** indexes into `text`, as a browser caret is. For `reference`, `caret` is the start of the span being replaced and `text` is the text at the moment of pointing (it may hold the previously pointed reference); the answer is only the new reference's spelling. A token whose sheet cannot be resolved answers `sheet: null`: coloured in the text, not framed on the grid. `GET functions` answers 159 entries with Excel-style signatures (the dev fixture `handoff/ui-dev/fixtures/functions.json` is the expected shape).

14. **Package-fragment relocation and MCE.** When an edit moves an opaque metadata registration between package owners, its namespace bindings and inherited markup interpretation travel with it. The supported inheritance rules are **ECMA-376 Part 3, fifth edition (2015), §§7.2, 7.3, 9.2 and 9.4**: `Ignorable` namespace names and `ProcessContent` expanded names accumulate from ancestors; prefixes are resolved where declared; `MustUnderstand` is examined where authored and is not copied from an ancestor. Authored vendor markup and other-edition hints remain byte-exact. A changed inherited directive chain outside this edition, including inherited `PreserveAttributes`/`PreserveElements`, refuses with a located `Unsupported` error. Additional destination directives cannot be canceled; relocation must prove that they preserve the fragment's interpretation or fail atomically with a located refusal. This preserves carried markup without claiming complete MCE processing, choosing `AlternateContent` branches, or guessing an application's understood namespaces. Primary specification: [ECMA-376](https://ecma-international.org/publications-and-standards/standards/ecma-376/), [Part 3 (2015)](https://ecma-international.org/wp-content/uploads/ECMA-376-3_5th_edition_december_2015.zip).

15. **Legacy VML structural edits.** A standard Note with an explicit eight-integer cell anchor follows its owning cell: both corner indices receive the owner's delta, retaining pixel offsets. Duplicate, empty or malformed coordinate metadata fails atomically, naming the part. Moving a surviving CSS-only note without an anchor refuses with `Unsupported`; synthesizing exact cell geometry would require host font/display metrics absent from the package. Genuine VML controls also block structural edits. Foreign namespace lookalikes remain byte-exact. The original openpyxl CSS-only fixture remains a mandatory atomic-refusal exchange case; a separate explicitly anchored fixture exercises the positive structural path. Excel can quantize offsets again during SaveAs: the desktop oracle compares native edited state with the opened Rust result, then compares both after one equal native SaveAs/reopen cycle, retaining all four snapshots and using no tolerance. `rust/tests/excel/fixtures/vml_excel.json` records Excel 16.0 build 20430.0 answers; other control/placement variants are not covered by that native evidence.

16. **One record header policy after the main merge.** Main now owns a CSV implementation, so the shared vocabulary is root `RecordHeader::{Source, None, Rows(u32), Infer}`. It replaces `ExcelHeader`; `RecordOptions::{header,set_header}` is the only generic header property. Boolean intake resolves once (`true` to `Source`, `false` to `None`), explicit null clears to `None`, and positive integer intake becomes `Rows(n)`. CSV resolves `Rows(1)` to its existing one-record header state (`Source` on readback); deeper rows and `Infer` refuse atomically at `$.header`. Excel retains physical row counts, conservative read-only inference and authoritative table metadata: `Rows(1)` remains distinct from `Source` and every `Rows(n)` with a named table refuses. The Excel grid bound belongs to Excel options rather than the shared enum. CSV's dialect-specific Boolean state remains owned by `CsvOptions`; it has no multiline header inference. Quoted line breaks within a CSV field remain ordinary field content.

17. **Explicit recalculation preserves lazy workbook intake.** `Workbook::open` and `from_bytes` remain lazy and retain their source-call and file-fidelity contracts. `calculate_all` is the explicit core operation; P8 service startup performs `parse_all` then `calculate_all`. Record/media reads do not evaluate formulas. This supersedes the eager reading of design §5.5's “runs on open”. A failed outer edit must preserve cells, revisions and pending recalculation state; nested batches and rollback never publish intermediate formula values.

18. **Native numeric evidence replaces the proposed cancellation shortcut.** Excel 16.0 build 20430.0 contradicts design §5.4's symmetric `2^-48` cutoff. Among 176 native calibration cases, `1-(1-2^-50)` is zero while `(1-2^-50)-1` is nonzero; wrapping the positive expression in parentheses preserves its binary result. The AST therefore retains explicit grouping. Ordinary arithmetic retains binary tails; 15-digit normalization belongs to comparison and the specified rounding operations. The precise replacement compensation policy remains a P5 investigation, and uncertain results must remain uncomputed instead of using the disproven shortcut. Raw COM values and native saved cell types are separate evidence: an Excel error can cross Value2 as a NaN Double, while its saved cache explicitly states `t="e"`. A collector must retain both without inventing a numeric value or discarding the error. Native execution success is not Rust evaluator equivalence.

19. **Native function signatures.** The function-list response retains its 159-entry shape, but its signature text must reflect actual accepted arguments. Excel 16.0 build 20430.0 refuses isolated `IF(TRUE)` and `IF(FALSE)` files and opens two-argument IF and explicitly omitted arms without repair: IF requires two through three argument slots. The same native run opens four-argument reference-form INDEX and returns the selected area's value: INDEX accepts two through four slots. `SUM()` and `COUNT()` are independently refused on file open. These controls correct the earlier UI fixture rather than perpetuating its incorrect IF minimum or incomplete INDEX form.

20. **Cancellation is a root-operation policy.** Further native probes refine item 18: for opposite effective signs in root binary addition/subtraction, a nonzero normal residual strictly smaller than eight ULPs of the first operand becomes positive zero. At exactly eight ULPs it remains nonzero; reversing operands across a binade can therefore change the answer. An outer authored group or arithmetic parent preserves the raw binary result. The 1,380-case binary fixture candidate contains 960 root observations and 420 grouped/nested controls with separately observed inputs; all match this rule at the tested scales. This is Excel 16.0 build 20430.0 evidence, not the disproven symmetric cutoff. SUM is a separate aggregate policy: the 288-case order probe agrees with final-pair correction for 54 ordered sums and contradicts correction after every accumulated pair. Its implementation and merge behavior must be pinned independently; it must not silently inherit root arithmetic at each step.

21. **Share and enrich project expressions (user instruction).** Excel functions must reuse the project's optimized typed expression operations and their root value kernels where semantics agree. Missing reusable operations are implemented once in that shared owner and used by both consumers, with expression parity and cost pins before the Excel integration. Formula text, workbook references, Excel error values, blank/coercion rules, serial dates, UTF-16 behavior and the native-confirmed numeric policies remain explicit workbook adapters. They must not change existing expression semantics implicitly. The registry resolves names once; evaluation never reparses an expression or constructs/binds a generic term per cell. This refines design §5.1: the distinct workbook grammar is necessary, but is not permission to duplicate generic computation. The exact reusable surface is being audited before function implementation.

22. **Native numeric-literal intake is distinct from computed numbers.** Excel 16.0 build 20430.0 truncates an authored decimal lexeme to its first fifteen significant digits before binary conversion. The 14 literal observations in `rust/tests/excel/fixtures/literal_precision_excel.json` include ordinary decimals, large integers, exponent forms, and `9.99999999999999999E307`: the latter becomes the largest entered value, `9.99999999999999E307`, although parsing the full lexeme directly as f64 would round it to `1E308`. Isolated positive and negative `1E308` formulas fail native Open; computed positive and negative `2^1023*(2-2^-52)` open and retain the largest finite f64 magnitude. Reuse the stack decimal owner in `format::Digits`; do not narrow generic floating-point or computed-result arithmetic. The unary sign remains a parser operation, file spelling remains exact, and invalid entered literals fail at their byte. This refines the undifferentiated fifteen-digit rule in design section 5.4. Provenance is the hashed native input/output reports under `p5-parser-boundary-observed` and `p5-parser-boundary-followup-v2-observed-1`; the fixture records Excel version, wire formulas, cached XML and native binary bits. Microsoft [Excel specifications and limits](https://support.microsoft.com/en-us/excel/excel-specifications-and-limits) documents distinct entered and computed maxima.

23. **Native call depth includes implicit intersection.** The 12 isolated controls in `rust/tests/excel/fixtures/formula_nesting_excel.json` establish 65 simultaneously nested calls (64 beneath the outermost), including file `_xlfn.SINGLE` and its entered `@` form. Excel 16.0 build 20430.0 opens ABS65, SINGLE65, SINGLE around ABS64, and nonconsecutive SINGLE-sum65; ABS66, SINGLE66, SINGLE around ABS65 and SINGLE-sum66 fail native Open. Every input hash, successful saved package hash, before/after value and saved numeric cache was checked; cleanup succeeded for every run. Provenance adds `p5-parser-depth-final-observed` to the two runs in item 22. Entry and lowered file compilation use this same call budget. The existing separate 64-level parenthesis/array resource bound remains; these call probes do not establish a native grouping limit. Explicit Group nodes remain semantically significant for cancellation. This is parser eligibility evidence, not a claim that the unfinished P5 evaluator matches every cached result.

24. **Circular status identifies actual cycles.** Design section 5.5's Kahn residue also contains acyclic consumers blocked behind a cycle. Refine circular reporting to actual strongly connected components: components with more than one node, or a self-loop. Only their members contribute to circular_count and its first 256 listed coordinates. Their downstream consumers retain caches and are uncomputed, without a circular label; unrelated acyclic nodes still evaluate once. Unsupported upstream formulas likewise block their consumers. This is the agreed graph-status contract, not evidence that static potential references in unchosen lazy branches are active cycles; those conditional/dynamic dependency semantics remain uncomputed until resolved. The graph remains lazy and explicit recalculation preserves item 17's workbook intake contract.

    The reverse index stores each typed rectangle once and indexes its canonical axis segments, replacing section 5.5's duplication across individual columns. Buckets contain registration IDs; retained capacity is bounded by peak precedents times axis depth. A point query opens at most 37 buckets and allocates no result collection. Its work includes candidates rejected by the orthogonal coordinate test, so it makes no output-only spatial complexity claim. Scheduling streams these reverse neighbors rather than retaining a potentially quadratic formula-edge list. Full `calculate_all` forces every eligible formula; unchanged incremental `recalculate` evaluates no nonvolatile formula.


25. **Ordered aggregate state, not merged partial sums.** Excel 16.0 build
20430.0's verified 288-case SUM-order run contains 54 distinct ordered
three-number SUM results. For those results, a left-to-right binary64 fold
with the calibrated root cancellation correction only on its final pair
matches 54/54 exact IEEE-754 bits. A raw fold matches 36/54; correcting
every pair matches 40/54. Direct arguments, range and array spellings agree
on the 18 canonical source triples tested. This is evidence for the tested
numeric cases, not a blanket claim about every SUM input or grouped call.
The shared `Accumulator` admits already-resolved values in source order,
retains only its raw prefix and last numeric operand, and applies the
correction at `finish_sum`. The follow-up 40-case Excel run
(`sum_edges_native.json`, input SHA-256
`e8cde983212d3129fc47d6464dc4fecf4bf02cfc7ab80e4b8551a281178b5b83`)
confirms that trailing numeric zeros can change cancellation, blank
references do not act like numeric zeros, and a later explicit argument
error takes precedence over aggregate overflow. Four native subnormal
results have nonzero in-memory IEEE-754 bits but saved numeric cache zero.
Until a distinct raw-computed/cache representation is settled, any
subnormal SUM input or intermediate returns named uncomputed rather than
publishing a plausible zero or normal result. This limits the implementation;
it does not establish full numeric conformance. The accumulator has no
`merge`: binary64 folds are not
associative, and collapsing a child subtotal loses the ordered final
operand. SUM/SUBTOTAL, pivot aggregates and status summaries use this one
owner but feed original ordered source rows; P6 must verify native pivot
grand-total ordering before choosing its intake traversal. Origin-sensitive
Boolean/text/blank coercion and nonnumeric aggregate functions remain at
the workbook value boundary, not in the numeric primitive.

26. **Subnormal runtime values and saved caches differ by operator.** The 16 native controls in `subnormal_native.json` (Excel 16.0 build 20430.0) show that `3E-308-2.5E-308` retains a subnormal COM value while Excel writes a zero XML cache. Adding `2.3E-308` then produces the normal result `2.8000000000000003E-308`, including grouped, referenced and SUM forms. Flushing each addition to zero is therefore incorrect. Multiplication, division and percent controls instead return positive zero both in memory and in the cache. Until the evaluator and incremental dependencies retain a separate raw computed value, addition/subtraction with subnormal inputs or intermediates and SUM with such values remain explicitly uncomputed and preserve their previous cache. This is a temporary coverage limitation, not numeric conformance; generic expression arithmetic retains its IEEE semantics. The durable fixture pins both native representations and the public regression pins the named refusal alongside computed multiplication/division/percent controls.

27. **Signed 1904 date and time formats.** The 64 native observations in `negative_date_native.json` (Excel 16.0 build 20430.0, explicit en-US TEXT evaluation) establish that a negative 1904 serial formats its magnitude with the selected section's automatic or authored sign. For example, `-1` with `m/d/yyyy` displays `-1/2/1904`; a conditional negative section can suppress the sign. This also applies to elapsed time. The 1900 system still refuses negative serials, and each system's magnitude stops at its own last day of year 9999. This changes rendering only: strict typed date intake and the stored value remain unchanged. UI-localized physical cell text is retained separately in the fixture.

28. **MOD has a native quotient boundary.** Three owned Excel 16.0 build 20430.0 runs yield 252 MOD observations in `math_mod_boundary_native.json`, with independently observed operand bits. The current bounded adapter uses shared `Arithmetic::Rem` plus divisor-sign adjustment for absolute quotient at most 2^40, and reports the observed `#NUM!` behavior at quotient at least 2^41 and for a nonzero subnormal remainder. The intervening quotient region has both numeric and error observations; its exact cutoff remains uncomputed rather than guessed. Subnormal operands also remain uncomputed until the raw-value policy is settled; a zero divisor is `#DIV/0!`. This is a partial numeric contract, not full MOD conformance. It does not cap the numerator: a huge numerator with a comparable divisor can yield zero. All 252 fixture cases must be exercised by the numeric primitive test, with 31 explicit holds and 221 exact numeric/error matches. Microsoft's MOD documentation supplies the divisor-sign and zero-divisor rules; the bounded quotient behavior comes from these native observations.

    The verified partition is 148 numeric results, 71 `#NUM!` errors and two `#DIV/0!` errors, with thirty uncertain-quotient cases and one subnormal-input case held. Excel rewrites formula literals; the primitive fixture therefore uses independently observed operand bits and retains original and native formula spellings separately.

29. **Recalculation reports retained workbook status.** `Recalculation.evaluated` counts computed formula candidates visited in the current pass, including computed Excel errors. `uncomputed` counts all currently Held or Circular formula nodes, including acyclic consumers blocked by uncomputed predecessors; `circular_count` counts actual current cycle members, and `circular` lists at most the first 256 in stable sheet-key/coordinate order. An unchanged incremental pass evaluates zero nonvolatile formulas but does not erase existing uncomputed/circular status. Full `calculate_all` still forces every formula candidate. The lazy Calculation owner reuses evaluator, graph scheduling, name-index and publication scratch; retaining the evaluator's two buffers intentionally improves the existing warm constant-formula allocation pin from 2 to 0, with a red check before integration. This defines the implementation target, not a claim that graph/Workbook integration or P5 is complete.

30. **Workbook text compatibility is a source-level setting.** Excel 16.0 build
20430.0 produced 102 verified text-function observations in
`unicode_native.json`: absent and explicit `setVersion="1"` inputs agree on
all 34 formulas across the two date systems; explicit `setVersion="2"`
changes nine of the 17 formulas in each system. `LEN("A😀Z")` is 4 in the
first two inputs and 3 in v2; `MID("A😀Z",2,1)` is a lone high UTF-16 unit
in the first two and `😀` in v2. Excel removes an explicit v1 extension on
SaveAs but retains v2 with `warnBelowVersion="2"`. The workbook reader resolves the authored marker once into a typed
calculation mode; the existing carried XML remains the source owner for
round trips. Evidence fixtures distinguish authored and saved markers.
Microsoft documents pre-feature workbooks as v1, agreeing with the absent
marker observations here. The text adapter selects this mode once; generic
Rust expressions keep their existing UTF-8 semantics. The marker namespace
and attributes follow [MS-XLSX CT_Version](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-xlsx/294f537a-8085-415a-82ad-979e2912c316),
and the function-specific changes follow [Microsoft's workbook setting](https://support.microsoft.com/en-gb/excel/compatibility-versions). The tested `LEFT` and `RIGHT`
cases keep whole emoji even in v1, so do not apply one code-unit slicing rule
to every function. Native COM `Value2` can contain an unpaired UTF-16 unit,
which saved OOXML transports as `_xD83D_` or `_xDE00_` in a `t="str"`
cache. The oracle decodes that transport for equality while retaining raw
XML. Because Rust `String` cannot contain a lone surrogate, any formula
whose result needs one remains explicitly uncomputed with its source cache
preserved until a distinct raw text/cache representation is settled; no
replacement character or silently changed text is published. This evidence
does not establish all Excel text functions, other builds/locales, or saved
workbook reopen semantics.

31. **Exact Excel temporal serials are sparse source facts.** A numeric OOXML
    cell with a temporal number format is decoded once through `DateSystem` to
    its typed `Scalar`. Where serializing that scalar under the same epoch
    yields different IEEE-754 bits, the held `Sheet` retains the original bits
    in its existing sparse `CellExtra` at that coordinate. This includes 1900
    serial 60, which has the same typed day as 59, and fractions below the
    millisecond resolution of the temporal leaves. Canonical cells retain no
    extra fact, `Cell` stays at most 80 bytes, and record/Arrow reads keep their
    established typed millisecond projection. The wire numeric decoder owns
    the one parse and date conversion; strict streamed readers reject invalid
    temporal serials, while held sheets retain their existing General numeric
    fallback for such a serial without reparsing it.

    Sheet save, formatted display, entry spelling and formula references use
    the proved original serial when present; they never infer 60 from a typed
    date or revalidate each read. Style changes that preserve the numeric
    interpretation may retain it. A changed date system, numeric meaning or
    replacement cell clears it before publication. `CellMut` removes the
    exceptional fact before lending `&mut Cell`, so forgetting the guard
    cannot leave stale bits; a normal drop may restore it only when source
    kind, value, format and formula are unchanged. Formula calculation stages
    the resulting `Cell` and exceptional raw bits together and publishes both
    atomically, including when typed 59 and 60 compare equal. A dependent in
    the same pass observes the staged numeric serial, while a later pass reads
    the retained sheet fact. This extends item 17's atomic recalculation rule
    and item 26's distinction between raw results and serialized caches; it
    does not change generic temporal or numeric arithmetic.

    The twelve native Excel 16.0 build 20430.0 observations in
    `rust/tests/excel/fixtures/temporal_serial_native.json` retain authored
    1900 serials 59, 60 and 60.5 and a submillisecond modern serial through
    calculation and SaveAs, in both date systems. 1900 serial 60 displays the
    phantom 29 February and `=A3-A2` evaluates to 1. The fixture records
    authored and saved XML, COM `Value2`, exact numeric bits, package hashes
    and cleanup. The run did not reopen the saved packages. It is native
    evidence for these cases, not an assertion that Rust passes the staged
    fidelity tests.
