# Added scope: record I/O and spreadsheet table discovery

User requests, 2026-09-30: enrich the existing Rust media readers/writers while
finishing Excel serve; detect several tables on one worksheet; infer multiline
headers; research outside examples and refinements.

This work belongs in the Rust core before the Python and Node phases. Get Data
must reuse Holder, RecordOptions and the existing Arrow dispatch, including the
same resolved table/header selection as direct record reads. No UI-only parser,
second importer or binding-side inference. P4 remains in progress. Named-table
record selection and unstructured region discovery are implemented locally
and narrowly tested; explicit multirow headers and opt-in evidence-based Infer now pass their narrow tests.

## Primary references and implications

- [Microsoft Power Query Excel connector](https://learn.microsoft.com/en-us/power-query/connectors/excel)
  demonstrates three suggested regions on one sheet (A1:C5, D8:E10, C13:F16).
  The navigator exposes those regions and the entire sheet. Its guidance warns
  that changed layouts can invalidate suggested tables, and that incorrect
  worksheet dimensions can omit data or slow reads. Discovery should therefore
  expose evidence-backed suggestions and inspect actual cells instead of
  treating the declared dimension as an authoritative data boundary.
- [pandas read_excel](https://pandas.pydata.org/docs/reference/api/pandas.read_excel.html)
  accepts several header row indices and retains the levels as a MultiIndex.
  Preserve each level until the existing field-naming boundary; do not confuse
  hierarchy with line breaks inside one label or silently discard a first data
  row that happens to contain strings.
- [openpyxl worksheet tables](https://openpyxl.readthedocs.io/en/stable/worksheet_tables.html)
  exposes multiple named tables and their ranges on a worksheet. Explicit OOXML
  table identity/range/header metadata is stronger evidence than an inferred
  rectangular region. A structured table's single column-name row and a
  visually merged, multilevel report header are different source facts.

## Required cases and limits to settle in the core design

- Several vertically stacked and side-by-side regions, declared tables mixed
  with unstructured blocks, title/footer notes, blank gaps and internal nulls.
- Explicit sheet/range/table selection takes precedence. More than one viable
  implicit selection is reported with candidate locations rather than joined.
- Multirow headers with merged parents, unmerged parents, blank corners,
  repeated labels, numeric/date labels, and embedded LF/CRLF text.
- Expand a merged label only inside its actual merge; never blindly forward
  fill unrelated blanks. Preserve source positions and label levels.
- Blank/duplicate/ambiguous labels and ambiguous header depth require a typed,
  located result or explicit override; no silent suffixes or guessed schema.
- Discovery may inspect bounded metadata/cell evidence; selected record reads
  remain streaming. Pin reads/allocations at two corpus sizes and benchmark the
  discovery path separately from selected reading and writing.
- Exchange fixtures in both directions, including openpyxl-authored multiple
  tables and merged headers. Real Excel acceptance remains a separate oracle.

Shared IOMedia default-option forwarding was reproduced and repaired: all 71
IOMedia tests, one read/write call pin, one allocation pin and eight benchmark
smoke cases pass. Projected-schema inconsistency was then reproduced and
repaired through one `IORecordOptions::result_field` binder. Generic declared
reads, IPC, Excel, Avro, text, XMLA, Parquet and Iceberg regressions pass.
Identity result schemas allocate zero times at 64/4,096 columns; declared schema
resolution performs no I/O. Review found a separate Iceberg predicate-pruning
mismatch after a declared datatype cast; its regression and repair now pass.
Unchanged declarations retain file pruning; changed filter-column declarations
use the existing post-cast expression path.
Schema caches retain the source shape, and canonical dimensions do not use
projected fields. See the local phase report for exact commands and counts.


## Further researched edge cases

The [pandas development reader documentation](https://pandas.pydata.org/docs/dev/reference/api/pandas.read_excel.html)
and [its writer source](https://github.com/pandas-dev/pandas/blob/main/pandas/io/formats/excel.py)
describe index-name/separator rows around hierarchical headers. These are
producer conventions, not a general rule for interpreting an arbitrary
workbook. Add a fixture with a real blank gap, and another whose first data row
contains strings: no implicit extra-row consumption or blind forward filling.
Discovery components separated by blanks remain suggestions; a caller's exact
range and header selection resolve their meaning.

Scratch reviews cover atomic selection intake, conservative header evidence
and preserving hierarchy through existing nested Fields. Those pieces remain
design proposals. The implemented region scanner recycles the existing
parser's cell buffer and compacts its connected-component frontier after every
row. The frontier is bounded by worksheet width; results are capped at 1,024
with a located refusal. Allocation counts are checked separately from retained
bytes; no peak-memory measurement is implied.

The [Excel.Workbook reference](https://learn.microsoft.com/en-us/powerquery-m/excel-workbook)
warns that automatic number/date header conversion changes with the host culture.
[Table.PromoteHeaders](https://learn.microsoft.com/en-us/powerquery-m/table-promoteheaders)
makes that choice explicit with a Culture option. Refinement: inferred header
meaning must not depend on this machine's French Office/Windows locale. Explicit
numeric/date header handling needs one documented deterministic core conversion;
header inference must not discard a numeric first data row merely because the
remaining body can be typed. These are design constraints, not a claim that the
new header API has been implemented or Excel-verified.

## Named-table implementation and next boundaries

The local core now has one `ExcelSelection` discriminant for worksheet/range
or named table and an `ExcelHeader::{Source,None}` policy. A table's actual
worksheet membership, relationship target, range, column metadata and header/
totals bands resolve before schema inference or record reading. A table body
includes absent physical rows as null records; ordinary worksheet selection
keeps its existing sparse-row behavior. Source uses the table's column names;
None uses physical column letters without including the table's header/totals.
Named-table writes still refuse before input consumption or I/O; range-aware
metadata maintenance remains required, not an implemented writer.

Initial named-table behavior: 20 tests passed. Source/None table-band regression
was reproduced red, then its three focused tests passed. Metadata-observer
allocations initially grew from 458 to 697 with unrelated long cells increasing
from 16 to 256; the existing parser now skips cell construction during this
metadata pass. Its warmed allocation pin is 433 at all four combinations of
2/32 selected rows and 16/256 unrelated rows. Field/read source calls each use
one package stream; a declared result schema uses no I/O. Eight debug benchmark
cases passed. These pins measure reader construction, not total inference work.
Inference costs are pinned separately: selected 2/32 rows cost 428/490
allocations for schema and 458/583 for full read, unchanged at 16/256 unrelated
rows. Header-only tables cost 415 in either corpus. The openpyxl driver now
passes eight reported checks, including two named tables, a missing body row,
header/totals exclusion and exact LF/CRLF column names. openpyxl leaves OOXML
inline-string escapes unread on the reverse path, so its check pins that exact
wire text while Rust also pins the decoded CRLF after reopening.

[Microsoft's table schema reference](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.table?view=openxml-3.0.1)
defines displayName and name as ST_Xstring and distinguishes header/totals
bands from the containing range. The
[openpyxl table implementation](https://openpyxl.readthedocs.io/en/latest/_modules/openpyxl/worksheet/table.html)
escapes tableColumn names. A local openpyxl fixture consequently exposed encoded
LF/CRLF names; the repair uses the existing OOXML string decoder before
identity/duplicate checks, including protected literal escape spellings. Its
focused runtime regression and 92 table tests pass.

Next core slices remain separate: one atomic combined-selection intake; a
shared physical leaf/header plan consumed by streaming and in-memory readers
and both writers; then explicit multirow headers and conservative inference. Actual merges alone expand a
label. Nested Field hierarchy owns meaning; geometry is canonical, not a second
schema or a promise of byte-exact original merge recovery. Nullable group
identity is not inferred from blank leaves. No automatic region choice is
published until its ambiguity rules have executable tests.

## Region discovery implementation

`excel::regions(handle, sheet)` returns authoritative registered tables and
eight-neighbour occupied-cell component bounds in worksheet/range order. It
uses actual cells, including uncached formulas and literal whitespace, ignoring
styled blanks and empty text. Table extents are excluded by compiled row events
and column intervals; they are not checked per cell against every table.
Suggested rectangles can contain holes or enclose excluded tables; they are
not automatically selected or asserted to be viable record tables.

The seven initial regressions were reproduced at runtime, then all seven
passed. Three frontier/exclusion controls also pass, and the separate
1,025-live-fragment bridge control confirms the result cap is not a premature
frontier cap. Source calls stay at one stream over four corpora. With ZIP
restart metadata fixed, 64/4,096 occupied rows allocate 157 times for one
region and 159 for 32 regions. The default ZIP writer adds a restart map
above 64 KiB, adding exactly a Vec and an Arc; a separate control pins that
two-allocation difference. Four debug benchmark cases passed. The complete
default Excel suite passed 725 tests before the additional bridge control;
named-table allocation and source-call pins were unchanged. Microsoft Excel is now available through the active session (16.0 build20430.0).
Native checks are in progress; synthetic rich-package and totals fixtures still
need repairs before they can serve as fidelity oracles.


## Shared owned-source intake

Borrowed anonymous media sources now retain the first bounded native stream
chunk directly and grow that private result until EOF. This removes private
atomic-copy staging while keeping public `copy_into` unchanged. Direct intake
costs 2 allocations for 64/4,096 bytes and 6 for 131,089 bytes; source-location
probes fall from three to one. Late read failures retain their typed location.

The downstream audit found exactly the explained reductions: named-table
metadata428, inferred field429/491 and fullread461/586 at2/32selectedrows,
header-only414; discovery148/150 for1/32regions atboth64/4,096rows. Every
compressed package is below a stream batch, removing21allocations. The text
corpus removes21allocations below64KiB and22 at114KiB. Changed pins state
these causes; public atomic-copy23/26 controls and all other audited pins stay
unchanged. Focused repaired pins and the plain-text benchmark smoke pass.

## Explicit hierarchical headers and physical record rows

The core now accepts `ExcelHeader::Rows(n)` for worksheet reads and writes.
Only real merged ranges expand labels; LF and CRLF remain inside one literal
name. `Field` owns the nested schema, and the shared record layout binds its
physical leaf columns once per batch. Named tables keep their authoritative
column metadata and reject `Rows(n)`. Opt-in `Infer` now passes 27 tests, including merged hierarchy, ambiguity refusals and trailing all-null physical records.

Held writes stage conversions and validate merge conflicts before mutation.
Streamed writes emit label rows and merge spans without expanding tall merged
headers into empty rows. Both paths retain actual empty body records as
physical rows, including the single empty Source header of a zero-column
record. The existing row-layout map owns that occupancy; cell count and cell
dimension retain their existing meanings. Required missing values name the
physical worksheet cell. Header/grid arithmetic rejects platform-maximum
record counts without overflow. Raw-batch preflight now checks the worksheet
limit before Arrow landing: huge null batches and cumulative batch overflow
both refuse promptly. The shared reader's fusion and ordinary-reader controls
pass; invalid direct anchors now fail an early grid check before subtracting the remaining row budget.

The nine-check openpyxl exchange passes both directions for merged headers,
literal multiline names, dates and an all-null body row. Excel 16.0 build
20430.0 opens both exchange files without repair. Separate allocation checks
show no growth from unrelated cells in header observation or held writes, and
no per-record growth between 64 and 640 streamed records. All eight debug
read/write benchmark cases pass; release timing has not been measured.

The native rich fixture and durable tint exporter now replace
scratch-only reproduction steps. Rust pins every untouched compressed member
and the closed normalization list for an edited worksheet/package; Excel opens
the edited result. Current style checks pass all 110 cases. The full desktop
fidelity run opened all ten inputs without repair and passed the edited-file
comparison. The note-anchor defect is repaired; native structural comparisons pass both before save and after equal one-save/reopen cycles. The original raw rich fixture needs its separate native-valid source repair and final proof.

Native Excel 16.0 build 20430.0 also completed seven worksheet filter/sort cut
probes. A full cross-sheet cut removes the source worksheet filter and does
not create one on the destination. Its hidden `_FilterDatabase` name keeps
its source-sheet scope while its formula follows the moved range. A same-sheet
cut moves the filter. An unaffected destination filter survives; a fully
covered one disappears. Partial body cuts keep the worksheet filter span,
while the hidden-name formula changes. Standalone sort metadata remains on
the source sheet. The generic filter-transfer proposal was replaced by native-observed ownership rules; eight Rust controls and the filtered cost/IO pins pass. All probes restored the
active Excel session successfully.

## Current table-write bound and resize refinement

Named-table overwrite now changes only the existing body while preserving the
header, totals, adjacent tables and unrelated worksheet XML. Seventeen tests
pass, including implicit coordinates, row/column style inheritance, nulls,
split/empty input batches, malformed input refusal and file-backed replacement.
The allocation measurements are 837/837 for 64/4096 unrelated rows and
837/6689 for 2/256 selected rows. Source I/O is one read, publication one write;
late refusal performs no publication. The changed XML plan remains eager,
though the ZIP reader now avoids a second complete worksheet buffer.

Positive body growth and shrink pass Rust, openpyxl and desktop Excel checks
for tables without totals and for one totals row whose cells are physically
present and representable. The three-mode native comparison opened, calculated,
saved and reopened Rust and Excel results: table/filter extents and visible
`SUBTOTAL(109,[qty])` values 6/13/21 agreed for shrink/equal/grow. An equal-height
write leaves the existing totals band unchanged. A moved standalone totals
formula retains its text and discards a stale cached value so Excel recalculates.

Moving a totals formula with A1 references or grouped/array metadata, or a cell
whose namespace scope cannot be retained, refuses atomically with a location.
A physically absent totals cell also refuses on a height change, naming the
source, destination and table: Excel may synthesize its label, function and
style from authoritative table-column metadata. The writer cannot replace that
missing cell with a blank without losing meaning. Existing allocated totals
cells with no `s` or explicit `s=0` stay plain even when the source row is
styled; Excel 16.0 confirmed this separately from the absent-cell case.

[Microsoft ListObject.Resize](https://learn.microsoft.com/en-us/office/vba/api/excel.listobject.resize)
changes the table range without inserting or moving worksheet cells. Whole-row
insert/remove would move adjacent tables and is therefore the wrong owner for
isolated record overwrite. Any expansion must prove that its corridor does
not consume another table or unrelated occupied cells before publication.

[Table.PromoteHeaders](https://learn.microsoft.com/en-us/powerquery-m/table-promoteheaders)
accepts an explicit culture when converting scalar labels. This reinforces
keeping header meaning at the resolved intake boundary rather than depending
on the machine's display locale. These references were checked 2026-09-30.

On 2026-10-01, the active Excel 16.0 build 20430.0 probes confirmed that
conditional-format and validation rules distinguish disjoint original ranges
from ranges intersected by a cut. Microsoft describes conditional-format
references relative to the top-left cell of the applied range in
[Conditional Formatting Rules Simplified](https://www.microsoft.com/en-us/microsoft-365/blog/2012/02/27/conditional-formatting-rules-simplified/).
The native saved packages, rather than a per-cell approximation, pin how cuts
repartition those ranges. The 18 partial-owner area-reference probes expose
additional behavior that must be represented exactly or refused before mutation.
