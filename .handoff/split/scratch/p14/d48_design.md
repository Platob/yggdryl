# P15: design (D48) - delimited text over the text line

The user's instruction (2026-10-10, `$S/p14/user_instruction.md:2`, typos the user's): "then make
the csv like media internally use the existing text line implementations but auto handling headers
or infer headers with separator etc optimizely". Read as decision 32 (`$S/user_decisions.md:203-209`):
the delimited-text media (`text/csv`, `text/tab-separated-values`) read and write through the text
medium's line machinery - the bounded physical-line splitting, the transport's decoding, `TextLine` -
instead of a second tokenizer of their own, a header row handled or inferred and the separator
inferred, at no worse cost than today's reader, its benchmark and allocation pins the bar. `$S` is
`.handoff/split/scratch`.

Placement (`user_decisions.md:203-209`): P15 is its own commit after P14 (D47,
`$S/p14/d47_design.md`); everything in the core's `text/` and `csv/`, the two bindings re-spelled
where a core signature moved, nothing added to Node (AGENTS §4). Read on the working tree of
2026-10-10; every line number below is of that tree and is re-read on P14's commit (the shared files
are listed under "Interplay with P14"). No cargo command was run to write this: every claim names
the file and line it was read from, the run of `python/.venv/bin/python -I` against the installed
extension that produced it, or the URL the research lane fetched (every one of them
`raw.githubusercontent.com`; the RFC, IANA, DuckDB and Arrow hosts were refused by the egress proxy,
so the RFC 4180 grammar is CSVW's quoted copy and the DuckDB rules its source files).

**One commit, one design**: the splitter's second mode, the CSV reader over it, the two inferences
and the dialect's tri-state settings land together, because the inference is what makes the
splitter's mode worth having and the mode is what makes the inference cost one window.

## The user's asks, each mapped to a decision

| # | The ask (the user's words) | Decision |
| --- | --- | --- |
| 1 | "make the csv like media internally use the existing text line implementations" | D48.1 (what is reused - the one window and its refill, `Held`'s part assembly, `TextBytes` - and at what level: the page and the splitter, not `TextLine`; the transport was already shared), D48.2 (the splitter's delimited mode: one quote-aware pass answering the record and its cells, a quoted newline inside it) |
| 2 | "but auto handling headers or infer headers" | D48.3 (the header ladder: stated, a declared field, inferred DuckDB's way over a bounded prefix, one rule for every door) |
| 3 | "with separator etc" | D48.4 (the separator inferred by consistency in one pass over a bounded prefix; the quote never inferred; TSV fixing the tab; the resolved dialect what the cache keys) |
| 4 | "optimizely" | D48.5 (what is deleted; the one pass, no copy of an unescaped record), D48.6 (the cost bar and how it is proven) |
| 5 | the exchange, the pages | D48.7 (an exchange check against Python's `csv` both directions and DuckDB's sniffer, always run; `docs/media/csv.md`) |

## What the tree holds today (the facts the design moves)

| Fact | Where |
| --- | --- |
| CSV is its own tokenizer: a 64 KiB window of its own (`WINDOW = DEFAULT_STREAM_BATCH_SIZE`), its own `fill` (looping `read` until `needed` bytes are in hand), BOM strip, `peek`, `skip_line`, `next_record`, `next_cell`, `quoted_content`, `unquoted_content`; every record **copied** into one reused `record: Vec<u8>` with the quotes removed and doubled quotes folded, each cell a `Cell { start, end, quoted }` range of that copy; the tokenizer consumes incrementally and never rescans a byte | `rust/src/csv/reader.rs:21-34`, `:36-60` (`Dialect`), `:78-114`, `:131-157`, `:200-233`, `:244-284`, `:292-332`, `:354-386` |
| Its rules: a UTF-8 BOM stripped by a byte compare of its own (`:147-155`); a blank record skipped (`:208-218`); a record opening with the comment byte skipped to the next `\n` whatever it quotes (`:219-222`); `trim` drops blanks around a cell unless the blank is the separator (`:235-241`, `:247-251`, `:270-277`); a quote opens a quoted cell only as the cell's first byte (`:252-258`); inside, a doubled quote is one with no escape, escape+byte is content with one (`:313-329`); bytes after a closing quote are content (`:267`); a record ends at `\n` or `\r\n`, a lone `\r` is content (`:372-384`); the stream ending inside quotes is refused at `$[n]`/`$.header` naming the row the quote opened in (`:336-348`); the unquoted scan is `memchr3(separator, '\n', '\r')` (`:360`), the quoted one `memchr2(quote, escape)`/`memchr(quote)` (`:299-301`) | `reader.rs` as cited |
| What that gives, run through the installed extension: `a,b\n1,"x\ny"\n2,"p\r\nq"\n3,z\r4\n` reads 3 rows with `x\ny`, `p\r\nq` (CRLF verbatim) and `z\r4`; the same bytes as `text/plain` split into `1,"x`, `y"`, `3,z`, `4`; `a;b\n1;2\n` under the defaults is one column named `a;b`; `1,2\n3,4\n` under the defaults names its columns `1` and `2` and has one row | the evidence lane's `probe.py`/`probe2.py` of 2026-10-10 |
| No separator or header inference anywhere: `separator: u8` (`,`), `CsvOptions::tsv()` the one tab, chosen by media type alone (`CsvCodec::default_options`, `Csv::new`); `header: bool` default `true`; `quote: Option<u8>` (`None` quotes nothing, default `Some(b'"')`), `escape: Option<u8>` (`None` the RFC doubling), `comment: Option<u8>`; `set_separator` refuses the byte the quote, the escape or the comment holds (`require_role`) | `rust/src/csv/options.rs:87-96`, `:105-141`, `:185-199`, `:304-318`; `rust/src/csv/media.rs:327-335`, `:461-466` |
| The row builder: `CellReader` resolved once per column (`Text`, `Zoned`, `Json`, `Value`), the row width checked, each cell read through `Charset::Utf8.transcribe` - the rule a text line decodes by - and the column's value door; the ladder `[Boolean, Int64, Float64, Date32]` then `datetime64(ns, UTC)`, a boolean proven only by what one prints, over the first `infer_row_size` (1 024) records copied out (`into_owned`) into a `VecDeque`; the names from the header, `column_<i>` where none, a name twice refused at `$.header`; the sampled rows emitted first; `Candidates::narrow` allocates per column and rung, never per cell | `reader.rs:450-486`, `:524-579`, `:664-738`, `:742-772`, `:774-809`, `:841-981`; `rust/src/text/line.rs:1659-1673` (`decoded`) |
| `count` walks the records reading no cell and subtracts the header by `options.header()`; `stated_columns` reads the first record alone and names it by `options.header()`, for `write_target` (every append and merge) and `column_size` | `reader.rs:991-1008`, `:1016-1027`; `media.rs:93-167` |
| The transport is **already** the text medium's: `csv/media.rs` imports `text::transport`'s `borrowed_decoded`, `decoded_over`, `encoded_terminator`, `ends_with`, `fetched`, `update_suffix`; `decoded_over` an owned stream or a cursor over the owned handle, `borrowed_decoded` one `pstream_bytes` at `DEFAULT_FETCH_BYTE_SIZE` (1 MiB) for a count or a schema; a declared charset other than UTF-8/US-ASCII laid over the coded bytes; no BOM looked for under UTF-8 | `rust/src/csv/media.rs:20-22`; `rust/src/text/transport.rs:32-66`, `:99-114`, `:116-178`; `rust/src/iobase.rs:49`, `:60` |
| The text splitter: one shared window (`Arc<Vec<u8>>`), `rewind` moving the open tail to 0 (`copy_within`, or a fresh page where a line still points into the old one), `refill` making **one** `source.read` per call, `next_part` yielding `LinePart { range, end }` - a line longer than the window in parts - the trailing `\r` held back as `overlap` until the next fill; when no break is found and `cursor > 0 \|\| filled < page.len()` it refills and `continue`s, so `next_break` **re-runs from the line's start over bytes it already scanned** (harmless for a stateless break); a line that merely straddles a window end but fits one is rewound and returned whole, and only a line longer than the window comes out in parts; `next_break` flexible (`\n`, `\r\n`, a lone `\r`, mixed) or pinned; no quote state | `rust/src/text/reader.rs:21-37`, `:39-62` (`rewind`), `:96-134` (`refill`, the one `read` at `:114`), `:137-210` (`next_part`; `:187-190` the overlap); `rust/src/text/sep.rs:220-259`, `:279-292` |
| `Held`: a line is a range of the page (`Span`) while it fits one window and a vector of its own (`Owned`) once it spans two (`detach`) or a header matched in its middle; `narrow`, `truncate`, `push`, `slice` (a range of the same page, `TextBytes::from_page`), `into_text_bytes`; **private** to `text/arrow.rs` (`enum Held`, no `pub(crate)`); `sep` is a **private** module of `text/` (`mod sep;`) | `rust/src/text/arrow.rs:261-370` (`enum Held` `:271`, `push` `:323`, `detach` `:347`, `slice` `:354`); `rust/src/text/mod.rs:25` |
| `RawRows::next_line` assembles one physical line from parts (`Held::push`/`detach`); a line cutting to nothing is a separator, not a row (`finish`, `decoded_size == 0`); the only multi-line record is `framing`, which joins continuation lines after a synthetic `\n` (the terminator lost) by a header match, not by quote state | `arrow.rs:625-782`, `:784-829`, `:858-874` (`:869` the pushed `\n`), `:884-899` |
| `sep.rs` reserves the word *separator* "for the field separation delimited formats will need" | `rust/src/text/sep.rs:1-7` |
| `TextLine::from_bytes` validates the whole line as UTF-8 (a pass), holds `Arc<TextOptions>`, the stated and resolved slots of the fifteen element and event columns | `line.rs:192-210`, `:346`, `:1659-1673` |
| The byte-order mark's one owner: `Charset::from_bom(input) -> Option<(Charset, usize)>` over `charset/bom.rs`; "a mark is framing rather than content, so `from_bom` answers its length and the caller decides" (AGENTS Charsets) | `rust/src/charset.rs:329`; `rust/src/charset/bom.rs` |
| The one prior art for a separator inference is the frame classifier's `LineSeparator::for_line`: candidates SOH, `\|`, the escaped markers, whitespace, ranked by the earliest that separated a field; no `,`, `;`, tab, no quotes | `rust/src/mime_type/line.rs:270-340` |
| The cache: `Csv<H>` keys its entry by the `CsvOptions` it read under, `reads_as` comparing the dialect roles, `header`, `null_values`, `trim`, `infer_row_size`; every write invalidates; the docs already say the entry holds "a CSV's inferred dialect and field", which nothing infers today | `media.rs:426-455`, `:525-549`, `:561-590`; `options.rs:443-452`; `docs/media/index.md:391` |
| The shared `header`: `MediumSettings::header() -> Option<bool>` (`None` on a medium with no header) and `set_header(bool) -> bool`, the object-safe `MediumOptions` twin and `RecordOptions`' forwarders, implemented by CSV and Excel; `RecordOptions::set_header(bool)` refused elsewhere at `$.header`; `csv_separator() -> Option<u8>` (`None` another encoding); `csv_quote()`/`csv_escape() -> Option<Option<u8>>` the shape of a CSV role that is itself optional; Python and Node set `header` and `separator` by name | `rust/src/media/options.rs:158-168`, `:202-208`, `:314-322`, `:2082-2095`, `:2103`, `:2121`, `:2155-2185`; `csv/options.rs:532-536`; `excel/options.rs:190-197`; `python/src/iomedia.rs:1589,1657,1950`; `node/src/media/options.rs:689-790` |
| The cost bar: `CSV_COUNT_COSTS = [(8, 12), (64, 18)]`, constant from 16 to 1 024 records - "seven for a one-cell record ... five for the transport and the cutter ... one each for the record buffer and the cell index on the first record. Every count past that is those two vectors doubling up to the first record's width ... nothing after it"; `csv_costs`: schema `pstream_bytes=1 url=1 media_type=1 is_container=1`, full read `pstream_bytes=1 url=1 bound_location=3 media_type=2 is_container=1 parent=1`, column count `pstream_bytes=1 size=1 url=1 media_type=2 is_container=2`, row count `pstream_bytes=1 media_type=2 is_container=2`; `a_record_wrapper_forwards_the_tail_read` (`read_tail_bytes=1`); the `media/csv/{plain,quoted,gzip}` bench (`write`, `read_declared`, `read_inferred`, `row_size` over 10 000 rows); the page's numbers (plain declared read 12.38 ms, inferred 13.55 ms, `row_size` 1.092 ms, 2026-10-02) | `rust/tests/allocations.rs:6085-6124` (the sentence `:6095-6106`); `rust/tests/iobase_calls.rs:1015-1036`, `:125-145`; `rust/benchmarks/media/csv.rs:75-120`; `docs/media/csv.md:292-308` |
| The CSV tests: `rust/tests/csv/{reader (32), media (44), writer (15), options (9), mod_ (2)}.rs`; `rust/tests/media/options.rs` mirrors `media/options.rs`; the pages `docs/media/csv.md` (Overview, Read, Write, Dialect, Edges, Performance), `docs/media/index.md:15`; AGENTS.md's `csv/` row ("one streaming RFC 4180 tokenizer ... `reader.rs`"); the skills' CSV rows; no exchange script and no CSV job - CSV bytes are fixtures of the ZIP, Azure and GCS scripts alone | `rust/tests/csv.rs`; `rust/tests/media/options.rs`; `docs/media/csv.md:1-60,180-308`; `AGENTS.md` Layout row for `ipc/, parquet/, avro/, csv/`; `skills/yggdryl-records/SKILL.md:52-55`; `scripts/check_{zip,azure,gcs}_interop.py` |
| The "one column under the default dialect" assertion, **seven sites**: `docs/media/csv.md` Rust `:44-45`, Python `:89-90`, JavaScript `:123-124`; `skills/yggdryl-records/references/rust.md:525`, `python.md:405`, `javascript.md:429` (each a block `check_docs_examples.py` runs); `skills/yggdryl-records/references/formats.md:69` in prose | as cited |
| The research (`raw.githubusercontent.com`): RFC 4180's `escaped = DQUOTE *(TEXTDATA / COMMA / CR / LF / 2DQUOTE) DQUOTE` (CSVW's copy, `w3c/csvw/gh-pages/syntax/index.html:2175-2195`); CSVW's dialect defaults and `text/tab-separated-values` → TAB, `header=absent` → no header (`:625-633,1105-1162`); Arrow's reader sniffing nothing, newlines in values opt-in (`apache/arrow/.../csv/options.h:45-67,152-172`); DuckDB's candidates `,` `\|` `;` `\t`, nine quote/escape pairs, the dialect with the most rows of one column count then the most columns, a user-set option fixed, a declared column count restricting the candidates (`duckdb/.../sniffer/dialect_detection.cpp:17-28,78-105,233-370`); DuckDB's header rule - types from the rows after the first, the first row a header iff some cell fails a cast to a non-text type, every column text or the first row null → a header (`header_detection.cpp:11-17,259-340`); Python's `Sniffer` scoring by the share of the modal field count with the preference `,` `\t` `;` space `:` (`cpython/main/Lib/csv.py:247-404,503-547`) and its `has_header` false negative on all-text data (`:635-697`, shown locally on 3.13) | the research lane's facts, each with its URL |

## D48.1 - what is reused, and at what level

Three readings of "use the existing text line implementations" were weighed:

- **A - a CSV record is a `TextLine`**: refused. A `TextLine` is an event of the graph - a `uuid`, a
  `hashcode` over its body, `transunix`, `seqnum`, the fifteen columns and their stated and resolved
  slots (`line.rs:192-210`) - and a CSV record is a row of the table its header names, with no
  identity of its own; the FIX capture is the line-as-event, a trades table is not. It would also
  pay a UTF-8 validation pass per record (`:1659-1673`) that the cell reader re-does through
  `transcribe`, an `Arc<TextOptions>` clone per record, and the row would still have to be cut into
  cells by a second scan - the cost bar refuses it before the semantics do.
- **B - the page, the splitter, the part assembly, `TextBytes`** (**recommended**), stated plainly:
  the **transport is already shared** (`csv/media.rs:20-22` imports `text::transport`), so P15 adds
  no sharing there. What is newly shared is the text line *implementation*'s window - one shared
  page, a record a range of it, a refill that never writes over a page something still names
  (`reader.rs:39-134`) - the part-to-unit assembly `RawRows::next_line` runs over `Held::push`/`detach`
  (`arrow.rs:323-350`, `:625-782`), factored into **one helper both callers use** so joining parts
  into a held unit has one owner, and `TextBytes` as the page type a kept record is of
  (`rust/src/text/bytes.rs:85,156`). What the tokenizer does that no line does - the quote-aware
  cut and the cell ranges - is **not replaced by an existing implementation**: it is CSV's own
  grammar, moved out of `csv/reader.rs` into `text/sep.rs` as the splitter's second mode, beside
  the one file that already says where a line ends. CSV deletes its own window, fill and record
  copy; what stays CSV's own is the typing ladder and the row builder.
- **C - the text medium's `RawRows` with a dialect** (the record as a framed `RawRow`): refused; a
  `RawRow` is one body with a header match (`arrow.rs:213-229`) and the framing join writes a
  synthetic `\n` (`:869`), which is not the CRLF a quoted cell holds verbatim (the probe:
  `p\r\nq`).

Decided: **B**. A CSV record on the row path is a **borrowed** range of `Lines::window()` (or of one
per-read buffer where it was longer than the window), its cells `&[u8]` ranges of it; a record kept
in the inference sample is a `TextBytes` of the page (one `Arc` clone), never the row path's. The
row builder retains nothing, so it pays no `Arc` increment per record or per cell. `TextLine` stays
the plain-text medium's.

## D48.2 - the splitter's delimited mode: one quote-aware pass, the record and its cells

`Lines::next_part` becomes generic over a sealed crate-private `Cutter` trait with two
implementations, monomorphized so the text path compiles to today's code (no enum match per line):

```text
pub(crate) trait Cutter { .. }               // sealed: where a unit ends, what to hold back
pub(crate) struct LineCut<'a>(Option<&'a LineSep>);   // today's: flexible or pinned terminator
pub(crate) struct RecordCut<'a>(&'a mut RecordScan);  // delimited: a break only outside quotes
```

`RecordScan` (`text/sep.rs`, crate-private - `text/mod.rs:25` becomes `pub(crate) mod sep;` so
`csv/reader.rs` reaches it) carries the `Delimiter` (separator, quote, escape, comment, trim - five
bytes and a flag, built once per read from the resolved dialect by `csv/reader.rs`) and the state
that survives a refill and a part boundary:

- the cell state (`CellStart`, `Unquoted`, `Quoted`, `QuoteSeen`, `Escaped`), the 1-based physical
  line and the line the record opened on (today's `line`/`record_line`, `reader.rs:86-89`);
- `scanned`, the record-relative offset the scan has consumed up to - **this is what makes the pass
  one pass across refills**: `next_part` refills and `continue`s after a search that found no break
  (`reader.rs:195-200`), and `rewind` moves the record's start to 0, so a record-relative offset
  needs no shift; `next_record_break` resumes at `record_start + scanned`, never re-reading a byte,
  so a quote opened in one fill and closed in the next is counted once, no cell is pushed twice and
  the line count is right. The two cases - a quote across a refill, a quote across an emitted part -
  are pinned in `rust/tests/csv/reader.rs`;
- `cells: Vec<Cell>` - `Cell { start, end, quoted, escaped }`, offsets record-relative, cleared per
  record, one vector per read (today's `cells`, `:85`) - **or nothing**, in the scan's **count-only
  mode**, which counts the cells of a record and pushes none: what `count` (which reads no cell,
  and today records them all the same) and the sniff run under.

`next_record_break(window, scan, complete)` is the state machine today's `next_cell`,
`quoted_content` and `unquoted_content` run, over the window instead of a copy:

- in `Unquoted`, `memchr2(separator, b'\n')`, a `\r` before the `\n` read by looking back - the quote
  is content inside an unquoted cell and is tested **only on the first byte at `CellStart`** (after
  the blanks `trim` drops, `:247-258`), so the scan makes no stop today's `memchr3(sep, '\n', '\r')`
  does not; in `Quoted`, `memchr2(quote, escape)` or `memchr(quote)` (today's, `:299-301`), the `\n`
  count inside quotes kept for the line number (`:303,309`);
- in `Quoted` a quote moves to `QuoteSeen`, from which a second quote with no escape is content and
  anything else closes the cell (`:325-329`), escape+byte content (`:313-324`); bytes after the close
  are content to the separator (`:267`);
- a break is `\n` or `\r\n` in every state but `Quoted`, and nothing in `Quoted`; a lone `\r` is
  content (`:377-384`); a comment record is skipped to its `\n` with no quote state (`:219-222`); a
  blank record is skipped (`:208-218`);
- **the byte-order mark** at offset 0 of the stream is read by the one owner: `Charset::from_bom`
  (`charset.rs:329`) is asked once at the first cut, and the length it answers is stepped over where
  it names UTF-8; `csv/reader.rs`'s own byte compare (`:147-155`) is deleted, not re-homed. The text
  mode keeps the mark (`transport.rs:175-178`), and the two modes keep their divergence, each the
  contract its page states;
- **the overlap**: a window's last byte is undecided where the next byte may complete it, so it is
  held back exactly as today's trailing `\r` is (`reader.rs:187-190`, `overlap` by cutter): a
  trailing `\r` **in every state except `Quoted`** (`a,\r|\n` at `CellStart`, `"x"\r|\n` after a
  close, `x\r|\n` in `Unquoted` - the `\r` is content in `Quoted` and is not held), and a trailing
  quote in `Quoted` (the next byte may double it);
- a record longer than the window comes out in parts, `end = false`, and the CSV path - which
  retains no record - assembles them into **one reused per-read buffer** through the shared
  assembly helper, never a `Held::Owned` vector per record: the buffer grows once per read to the
  longest record and is the one copy such a record costs, in any design. The cells' offsets are
  record-relative for that reason: a part boundary moves no cell;
- the stream ending in `Quoted` is the refusal today's `unterminated` raises (`:336-348`), the
  record's index and the opened line carried by the scan.

**`Lines::prime()`** (new, `text/reader.rs`): fills the window by reading until it is full or the
source is drained, cutting nothing - today's tokenizer `fill(needed)` loops the same way
(`:131-145`), while `Lines::refill` makes one `read` per call (`:114`) and so a first fill holds
whatever one read answers (a gzip decoder's chunk, a charset `Reader`'s transcribed chunk, one HTTP
chunk). The sniff and the header rule run over the primed window, so the sample is the first
min(64 KiB, document) bytes whatever the coding or chunking, and a `.csv`, its `.csv.gz` copy and
a UTF-16LE copy resolve one dialect - pinned. A BOM split by a short first read cannot occur after
priming.

**One pass.** The quote-aware break has to stop at every separator to know a cell's state, so
recording the cell ranges in the same scan costs nothing a second scan would not pay twice; a design
that breaks on `memchr2(\n, \r)` and then cuts cells reads every record twice. And the pass copies
nothing: `Record::cell(i) -> &[u8]` borrows an unquoted cell from the page, a quoted cell with no
doubled quote and no escape as the range between its quotes, and a cell holding a doubled quote or
an escape (`escaped: true`) from **one per-read scratch `Vec<u8>`** into which every escaped cell of
the record is unescaped once when the record is read, its range recorded - never a `Cow::Owned` per
cell, which would be one allocation per escaped cell where today's record copy charges none. Today
every record is copied whole (`:304,310,365`); after, the common record moves no byte - the
"optimizely".

**What the physical-line mode keeps**: everything. `LineCut` is today's `next_part` body
(`reader.rs:145-209`) behind the monomorphized trait, `next_break` (`sep.rs:240-259`) untouched, so
the text pins - `TEXT_LINES_ONCE`, the retention-per-window pin, the `text_lines` benches at
`--quick` level - do not move; `text/arrow.rs:659` is re-spelled `next_part(LineCut(options.linesep()))`
and `Held`'s `push`/`detach`/`slice` become `pub(crate)` for the assembly helper.

## D48.3 - the header: stated, declared, inferred over a bounded prefix, one rule for every door

`CsvOptions.header: Option<bool>` - `Some(true)`/`Some(false)` stated, **`None` the default and
inferred**. The ladder, one rung answering and the rest never asked:

1. **Stated** (`Some`): as today - the first record names the columns, or every record is a row.
2. **A declared field**: the first record is a header iff its cells spell `declared.fields()`'
   names, in order, exactly - one comparison, nothing typed. A first record that does **not** spell
   them is a **row** (rung 3 never runs under a declaration: a declared all-text field would
   otherwise answer "header" and drop the first data row of a headerless document).
3. **Inferred, DuckDB's rule** (`header_detection.cpp:259-290`), with no declaration, over **the
   first 32 records the sniff's prefix holds** (`HEADER_SAMPLE`, a constant beside
   `DEFAULT_CSV_INFER_ROW_SIZE`): the records after the first are typed by the ladder
   (`Candidates::narrow`, `reader.rs:694-719`), and the first record is a header iff some non-empty
   cell of it does not read under its column's non-text datatype (a cell `symbol` over an `int64`
   column), **or every column is text**, or the first record is the only one. Every column text → a
   header keeps today's `header: true` default on a text table (`name,city\nalice,paris`), which
   Python's length vote gets wrong (the research's local run), and a one-record document stays what
   it is today: a header and no row. Bounded to 32 so that `count`, `column_size` and `write_target`
   - which read no cell today - pay one typing of at most 32 records and never `infer_row_size`
   (1 024): the alternative, typing the whole sample on the count path, roughly doubles `row_size`
   (the bench's `read_inferred` less `read_declared` is 1.2 ms against `row_size`'s 1.09 ms).

**One rule, every door**: `read`, `count`, `stated_columns` (`column_size`, `write_target`) resolve
the header through the same `resolve_dialect`, so they agree on a document; `Resolved` is kept in
the open cache entry (`media.rs:437-445`), so a count after a read on an open handle, or within
`cache_ttl`, runs no inference again. The names stay the header's, an empty name `column_<i>`, a
name twice refused at `$.header` (`:742-772`; DuckDB suffixes `_1`, a second spelling of one name,
not adopted - put to the user, #5); without a header `column_1..n`.

**On write**: `None` writes a header, as `Some(true)` does - a document a caller wrote is one a
reader infers a header on - and an append or a merge onto a stored document follows the **stored**
document's answer: `write_target` (`media.rs:93-153`) resolves the dialect (D48.4) as a read does,
so an append onto a document inferred headerless is positional and onto one with a header completes
onto its names.

**The shared accessor does not move** (put to the user, #6): `MediumSettings::header() ->
Option<bool>`/`set_header(bool)`, their `MediumOptions` twins and `RecordOptions::header()`/
`set_header(bool)` (`media/options.rs:158-168`, `:202-208`, `:314-322`, `:2155-2185`) stay as they are
on CSV and Excel - `set_header(bool)` states it, `header()` answers the statement (`None` where
unstated, as where another encoding) - and Excel takes no refusal it did not have. The tri-state is
CSV's own door, in the shape `csv_quote()` already has (`:2103`): `RecordOptions::csv_header() ->
Option<Option<bool>>` (`None` another encoding, `Some(None)` unstated) and `set_csv_header(Option<bool>)`
(`None` clearing the statement, refused at `$.header` for another encoding), over
`CsvOptions::header() -> Option<bool>`, `set_header(bool)` and `clear_header()`. Python's `header`
property reads `bool | None` through `csv_header` where the options are CSV's and takes `bool | None`,
`None` routed to `set_csv_header(None)` (AGENTS "`None` and `null` are values, and clear"); Node's
`header`/`withHeader` keep their `boolean` type, since no signature they call moved. The `header`
MIME parameter (`header=absent`, RFC 4180/CSVW) is not read: `MediaType` holds no parameter but the
charset (`rust/src/media_type.rs:32`), and the explicit option and the inference answer the same
question (put to the user, #7).

## D48.4 - the separator inferred in one pass; the quote never; TSV fixed; the resolved dialect

`CsvOptions.separator: Option<u8>` - `Some` stated, **`None` the default and inferred**;
`CsvOptions::tsv()` states `Some(b'\t')` and `default_options`/`Csv::new` under
`text/tab-separated-values` keep choosing it (`media.rs:327-335,461-466`), so a `.tsv` name fixes
the tab and never sniffs. **The quote is never sniffed**: `quote: Option<u8>` already means "quote
nothing" by `None` and a caller's `Some(b'"')` cannot be told from the default, so a sniffed quote
would either override a stated one (against DuckDB's "a user-set option fixed", which this design
cites) or fail `reads_as`; the quote, the escape and the comment are the stated roles under which
the sniff cuts, and a `'`-quoted document is one a caller states. `RecordOptions::csv_separator()`
becomes `Option<Option<u8>>` (the `csv_quote` shape: `None` another encoding, `Some(None)` unstated)
and `set_csv_separator(Option<u8>)` takes the tri-state, over `CsvOptions::separator()`,
`set_separator(u8)` and `clear_separator()`; `rust/tests/media/options.rs` pins both shapes for CSV
and for another encoding.

**The sniff**, run once per read, count, column size or write where the separator or the header is
unstated, over **the primed window's first 16 KiB cut at its last complete record** (`SNIFF_BYTES`),
and never where a `.tsv` name or a stated separator fixes the axis:

1. Candidates `,` `\t` `;` `|` (DuckDB's set, `dialect_detection.cpp:17-28`), **less any byte the
   stated quote, escape or comment holds** (the byte `set_separator`'s role check would refuse,
   `csv/options.rs:192`). Under a declared field, a candidate is kept only where its modal count
   below equals the declared width (DuckDB's declared-column rule): the declaration already says how
   many cells a record holds, so a `;` document with stray commas under a three-column declaration
   cannot sniff `,` and then be refused by width.
2. **One pass**: one `RecordScan` in count-only mode under the stated quote walks the prefix once,
   counting, per record, the occurrences of all four candidate bytes outside quotes at once (a
   record's cell count under candidate c is its unquoted c count plus one), each candidate's counts
   into a fixed `[u16; 65]` histogram on the stack (counts over 64 in the overflow bucket) - no
   second cut per candidate, since the quote is stated; four histograms and the stack is all it
   allocates: **0**. A comment or blank record is skipped as the reader skips it.
3. Score: (a) the share of records holding the modal cell count - **no preamble allowance**: a
   leading record that disagrees lowers the share like any other, because nothing skips it (the
   header rung and the row builder read record 0, so an allowed preamble would be read as the
   header or refused by width; a preamble skip is put to the user, #14); (b) the modal count, at
   least 2, **the higher count winning at equal share**; (c) the preference `,` `\t` `;` `|`. No
   candidate reaching a modal count of 2 → **the separator `,`** under the stated quote: a one-column
   document is a document, never a refusal (AGENTS "Best effort, then a named refusal"). A window
   holding **no complete record** (a first record over 64 KiB) is read as the start of one record:
   the candidate with the most unquoted occurrences in it wins, `,` at a tie, and the header rung
   then reads the first 32 records as the cutter yields them.
4. The winner is the **resolved** dialect: `Resolved { separator, header }` beside the stated
   options, what the reader cuts by, what `write_target` renders an append by, what the cache entry
   holds as its state (`keyed`, `media.rs:437-445`) and what `reads_as` compares - a stated role must
   equal the resolved one, an unstated role always does - so a document read under an inferred `;`
   answers options stating `;` and options stating nothing, and `docs/media/index.md:391`'s "a CSV's
   inferred dialect" becomes true.

**The cost**: one count-mode pass over at most 16 KiB stopping at every candidate byte and quote -
the per-byte work of a count pass over 16 of the bench document's 230 KiB, so about 7 % of the
count's work on `row_size` and under 1 % of `read_declared`, plus rung 3's typing of at most 32
records where the header is unstated. **Measured, not estimated**: `cargo bench -p yggdryl --bench
media -- media/csv --quick` before phase 2 and after, `row_size` and `read_declared` written here
with their numbers; "under a tenth of a millisecond" is withdrawn until the run says it. Under a
stated separator *and* a stated header nothing runs; under a stated separator alone rung 3 runs.

## D48.5 - what is deleted, what owns each fact

| Fact | Owner after P15 | Deleted |
| --- | --- | --- |
| where a record ends, a quoted newline inside it, the overlap at a window's end, the comment skip, the count-only mode | `text/sep.rs` `RecordScan`/`next_record_break`, `text/reader.rs` `Lines::next_part::<RecordCut>` | `csv/reader.rs` `Tokenizer` whole: its window, `fill`, `peek`, `peek_second`, `skip_line`, `next_record`, `next_cell`, `quoted_content`, `unquoted_content`, `unterminated` (`:21-34,62-387`) and the `record: Vec<u8>` copy |
| the BOM at offset 0 | `Charset::from_bom`, asked once by `next_record_break` | the tokenizer's byte compare (`:147-155`) |
| the window, the refill, the page a record is a range of, the prime | `text/reader.rs` `Lines` (`prime` new, the rest unchanged) | the tokenizer's `window: Vec<u8>` |
| joining parts into one held unit | one crate-private helper in `text/arrow.rs` over `Held::push`/`detach`, called by `RawRows::next_line` and by the CSV record assembly | the loop's second copy |
| a cell's bytes | `csv/reader.rs` `Record<'_>` over the window or the per-read buffer, the scan's `cells` and the per-read scratch; `cell(i) -> (&[u8], quoted)` | the per-record copy |
| the five dialect roles the cut reads by | `csv/reader.rs` builds `Delimiter` from the resolved dialect (today's `Dialect::of`, `:49-60`, moved) | `Dialect` in `csv/reader.rs` |
| the typing ladder, the row builder, the names, the count | `csv/reader.rs` (`CellReader`, `Columns`, `Candidates`, `LADDER`, `header_names`, `Rows`, `open_columns`, `stated_columns`, `count` - unchanged but for the cutter they call and the resolved header they read) | - |
| the sniff and the header rule | `csv/reader.rs` `resolve_dialect(primed, options, declared) -> Resolved` (new), `SNIFF_BYTES`, `HEADER_SAMPLE` | - |
| the stated dialect, the resolved one | `csv/options.rs` (`separator`, `header` tri-state with `clear_*`; `Resolved`; `reads_as` over it) | - |
| the CSV doors of `RecordOptions` | `media/options.rs` `csv_separator() -> Option<Option<u8>>`, `set_csv_separator(Option<u8>)`, `csv_header()`, `set_csv_header(Option<bool>)`; the shared `header`/`set_header` unchanged | - |
| the record terminator vocabulary | `text/sep.rs` (its module doc re-spelled: the word *separator* now delimits fields, as it reserved) | - |
| the writer | `csv/writer.rs`, unchanged but for an unstated separator (`,`, `\t` under TSV) and an unstated header (written) | - |

`TextOptions` gains nothing: the dialect is `CsvOptions`' and the splitter's mode is chosen by the
caller, so the plain-text medium names no separator. Nothing of `TextLine` moves.

## D48.6 - the cost bar, and how it is proven

- **`CSV_COUNT_COSTS` (12, 18)**: the exact expected deltas are written **before** counting, from the
  pin's own attribution (`allocations.rs:6095-6106`): the record buffer gone, −1 and **minus every
  doubling the sentence attributes to it** (not −1 alone); the cell index gone from the count path
  too (count-only mode pushes nothing), −1 and minus its doublings; the tokenizer's window gone, −1;
  the splitter's page +2 (the `Vec` and its `Arc` box); the sniff 0; rung 3's typing of at most 32
  records what `Candidates::narrow` allocates per column and rung (the header is unstated under
  `CsvOptions::new()`, so `csv_count_cost`'s corpus takes that path), counted at phase 2 and written
  as its own term. The new constant is the sum of those terms and nothing else; a number that does
  not drop by the record and index deltas points to a hidden copy, and a term that grows with the
  records is a defect, never a re-pin. The test keeps its 16-versus-1 024 equality as the invariant,
  which now also rests on rung 3 allocating nothing per cell - stated in its sentence.
- **`csv_costs`**: the four strings must not move - the sniff reads the window the one
  `pstream_bytes` filled, `write_target` reads the prefix it already read, the row count walks the
  same borrowed transport; `a_record_wrapper_forwards_the_tail_read` holds (the wrapper is
  untouched).
- **The bench**: `cargo bench -p yggdryl --bench media -- media/csv --quick` before phase 2 and after,
  direction only, and a new `column_size` row beside `row_size` - `read_declared` and `read_inferred`
  not slower, `write` unmoved, `row_size` and `column_size` **not slower than the record copy they
  shed buys** (the sniff and rung 3 cost time they did not; the saving is the copy; the
  expectation that `row_size` gets faster is **withdrawn until measured**); the page's table is
  regenerated by a release run on the machine it names or left at 2026-10-02 with the sentence that
  the release run owes it. `media/csv/gzip` gates the prime (one `read` loop against today's).
- **A call-count pin**: `stated_columns` and `count` read one `pstream_bytes` and prime one window,
  whatever `infer_row_size` says (`iobase_calls.rs` `csv_costs` already holds `pstream_bytes=1`;
  the primed window is one fetch of the 1 MiB `borrowed_decoded` buffer).
- **Text's pins must not move**: `TEXT_LINES_ONCE`, `OWNED_COPY_COSTS`,
  `keeping_every_line_costs_its_windows_and_not_its_lines`, the `text_lines` benches - `LineCut` is
  today's path.
- **New pins** (`rust/tests/allocations.rs`, red first): a record straddling byte 65 536 but shorter
  than the window costs no allocation (it is rewound and read whole); a record longer than the
  window costs the per-read buffer's growth once per read and reads verbatim across a quoted
  newline; a document of 1 024 records of 8 cells with every fourth cell quoted (today's
  `csv_records`) costs the same whether the quotes double or not, and the same whether every cell
  doubles a quote (one scratch per read, its growth once); the sniff over any prefix allocates 0;
  rung 3 over 32 records costs what `narrow` costs per column and nothing per record (16 versus
  1 024 records equal).

## D48.7 - the exchange, the bindings, the pages

- **Exchange** (AGENTS: "exchange formats checked both directions against an outside
  implementation" - CSV has none today): `scripts/check_csv_interop.py`, Python's standard `csv`
  module writing documents the crate reads - quoted newlines, CRLF inside quotes, doubled quotes,
  `;` and tab dialects, a headerless numeric table, a BOM - and reading documents the crate wrote,
  cell for cell; and the sniffer's separator and header answers compared to `duckdb`'s
  `read_csv_auto` on the same documents. **DuckDB is installed in the job** (a `pip install duckdb`
  step, as the other exchanges install their peers), and the script **refuses** where `duckdb` does
  not import rather than printing `SKIPPED`: a skipped half of an exchange check is not a pass
  (AGENTS Media; `interop/`'s precedent). Its CI job is a row of `.github/ci/rows.toml` beside the
  Excel exchange, which is a workflow-table change and runs everything once (put to the user, #8).
- **Bindings, re-spelled where the signature moved** (AGENTS §3/§4): Python's `RecordOptions.separator`
  reads and takes `str | None` and `.header` `bool | None` (`python/src/iomedia.rs:2418-2551`, the
  pickle at `:1589,1950`, `_native.pyi`); Node's `separator`/`withSeparator` nullable because
  `csv_separator()`'s shape moved (`node/src/media/options.rs:689-790`, `index.d.ts` regenerated),
  `header`/`withHeader` unchanged (`boolean`) because the shared accessor did not move; **no new Node
  test and no new Node door** (AGENTS §4: a change that does not name Node "adds no door, parity
  test, benchmark, `.api-bindings.txt` entry or JavaScript tab") - an existing `node/tests/records.test.js`
  assertion is re-pinned only where the moved signature breaks it; Python's tri-state round trip is
  one new case in `python/tests/test_iomedia.py` (`TestCsvOptions`, `:293`). `.api-bindings.txt:61`
  re-spelled.
- **Pages**: `docs/media/csv.md` - the intro ("a header named or inferred, the separator stated or
  inferred"), the Read paragraph and **all three of its tabs** (the `;` document now reads as two
  columns under the default dialect - the assertion `== 1` moves to `2` in the Rust tab `:44-45`, the
  Python tab `:89-90` and the JavaScript tab `:123-124`, each existing tab re-pinned, none added), a
  **Dialect inference** section stating D48.3's ladder and D48.4's candidates and score as two short
  tables with a **Rust and Python** example (a `;` document and a headerless numeric one read with no
  option set; no JavaScript tab is added for a change that does not name Node - the one line "Rust
  and Python shown; the JavaScript door reads the same inference" where the page needs it), the
  Dialect table's `separator` and `header` rows (unstated/inferred the default), Edges (the cache
  keyed by the resolved dialect; a one-column document; a preamble not skipped; the one copy a
  record longer than the window costs), Performance by the release run; the same `== 1` assertion
  moved to `2` in `skills/yggdryl-records/references/rust.md:525`, `python.md:405`, `javascript.md:429`
  (existing blocks re-pinned) and the prose of `formats.md:69`; `docs/media/index.md:391` unchanged
  and true; `AGENTS.md`'s `csv/` row ("read as the text splitter's delimited mode - one quote-aware
  pass answering each record and its cells as ranges of the page - under a dialect stated or
  inferred ..."); `skills/yggdryl-records/SKILL.md:52-55` (the `;` row: "or nothing, the separator
  inferred"); `.api-inventory.txt:3361-3367,4165-4180` (`separator()`/`header()` answering `Option`,
  `clear_separator`, `clear_header`, `csv_header`, `set_csv_header`, `Resolved`; the crate-private
  `Cutter`, `RecordScan` and `prime` omitted).

## Tests and the pins that move, each with its sentence

| Pin | Moves to | Sentence |
| --- | --- | --- |
| `rust/tests/csv/reader.rs:350` `a_tsv_name_reads_tabs_and_an_option_reads_any_other_separator` | the `;` document reads as `;` with no option; a stated `,` keeps one column | "the separator is inferred where unstated, D48.4" |
| `:428` `without_a_header_the_columns_are_numbered_and_every_record_is_a_row` | `1,2\n3,4` under no statement infers no header (two int rows, `column_1`, `column_2`); `Some(false)` as today; `Some(true)` names `1`, `2` | "the header is inferred where unstated, D48.3" |
| `:124` `a_header_naming_a_column_twice_is_refused` | the same refusal, the header inferred (text cells over typed rows) or stated | "D48.3" |
| `:344` `a_byte_order_mark_is_framing_and_not_the_first_name` | unchanged, now `Charset::from_bom`'s answer | "D48.2" |
| `rust/tests/csv/reader.rs` new | the ladder of D48.3 (a declared field's names; a declared field the first record does not spell → a row; every-column-text → header; a numeric first row → no header; one record → header; the rule bounded to 32 records); the sniff of D48.4 (each candidate; a quoted `,` inside a `;` document; a candidate the stated comment holds excluded; a declared width restricting the candidates; equal shares → the higher count; a disagreeing lead record lowering the share; no candidate → `,`; a window with no complete record); the window-boundary records (a quoted newline across 64 KiB, a quote as the window's last byte, a `\r` there at `CellStart`, in `Unquoted` and after a close; a quote opened in one fill and closed after a refill, and across an emitted part); a `.csv`, `.csv.gz` and UTF-16LE copy of one document resolving one dialect | "D48.2-4" |
| `rust/tests/text/sep.rs`, `rust/tests/text/reader.rs` new | `next_record_break`'s state machine on the tokenizer's old rules (`:247-320` of `reader.rs` re-homed), the count-only mode, `scanned` surviving a rewind, `prime` filling to full or drained; `LineCut` byte-equal to today | "D48.2" |
| `rust/tests/csv/options.rs`, `media.rs:1115` (`an_open_handle_answers_its_cached_schema_only_for_the_options_it_was_read_under`), `:479`, `:1154` | the tri-state setters and `clear_*`; `reads_as` over the resolved dialect; a count after a read on an open handle running no inference; `media_open_as` binding the tab | "the cache keys the resolved dialect, D48.4" |
| `rust/tests/media/options.rs` | `csv_separator()`/`set_csv_separator(None)`, `csv_header()`/`set_csv_header(None)` on CSV and on another encoding; `header()`/`set_header(bool)` unchanged on CSV and Excel | "the CSV doors beside the shared accessor, D48.3, D48.4" |
| `rust/tests/csv/writer.rs`, `media.rs:930-1072` (the append/overwrite/merge onto a stored document) | an unstated separator writes `,`; an append follows the stored document's resolved dialect | "D48.3, D48.4" |
| `rust/tests/allocations.rs:6107` `CSV_COUNT_COSTS` | counted at phase 2 against the written deltas, the constant alone | "the splitter's page replaces the cutter's window, the record copy and the count's cell index; rung 3 typing 32 records once, D48.6" |
| `rust/tests/allocations.rs` new rows of D48.6 | counted at phase 2 | "D48.6" |
| `rust/tests/iobase_calls.rs` `csv_costs` | unchanged (the gate) plus the one-window pin of `stated_columns`/`count` | "D48.6" |
| `python/tests/test_iomedia.py:293` `TestCsvOptions`, `python/tests/media/test_init.py:513-564` | `header`/`separator` as `None`; the `typed.csv` read unchanged (its header is text over typed rows) | "D48.3, D48.4" |
| `node/tests/records.test.js` | only where an existing assertion names `separator`'s type | "re-spelled, D48.7" |
| `docs/media/csv.md:44-45,89-90,123-124`; `skills/.../rust.md:525`, `python.md:405`, `javascript.md:429`; `formats.md:69` | `1` → `2` in the six blocks, the prose re-spelled | "D48.4" |
| `.api-inventory.txt`, `.api-bindings.txt:61` | the `Option` signatures, the new doors | "D48.3, D48.4" |

**Must not move**: `csv_costs`' four strings; `a_record_wrapper_forwards_the_tail_read`; every text
pin (`TEXT_LINES_ONCE`, `OWNED_COPY_COSTS`, the retention pin, the `<= 8` read-back pin,
`text_costs`); every `rust/tests/csv/reader.rs` rule test but the three named (quoting, CRLF, lone
CR, EOF in quotes, blank and comment records, escape, nulls, trim, declared reads, the ladder, the
past-sample refusal, value-door parity, NaN/inf); the warehouse tests over `.csv` leaves
(`iobase_calls.rs:1600-1740`); every Excel `header` test (`rust/tests/excel/options.rs:427,455`,
`media.rs:374`: the shared accessor did not move); the FIX and market pins, which read no CSV;
Node's `header` declarations.

## Implementation plan

One commit, one push, CI read to `CI result`; phases in AGENTS order, disjoint files per worker, the
whole run leading the chain once phase 2 settles. `cargo check -p yggdryl --all-targets --keep-going
--message-format=short` drives the sweep; `cargo fmt --all` once after the last worker. A rustdoc
filter names the module that defines the item and its passed count is read.

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move |
| --- | --- | --- | --- |
| 1 the splitter's mode | `rust/src/text/sep.rs` (`Delimiter`, `RecordScan`, `Cell`, `next_record_break`, the count-only mode, the module doc), `rust/src/text/reader.rs` (`Cutter`, `LineCut`, `RecordCut`, `next_part::<C>`, `prime`, the overlap by cutter, `Charset::from_bom` at the first record cut), `rust/src/text/mod.rs:25` (`pub(crate) mod sep;`), `rust/src/text/arrow.rs` (`:659` re-spelled `LineCut(options.linesep())`; `Held`'s `push`/`detach`/`slice` `pub(crate)`; the part-assembly helper factored out of `RawRows::next_line`); tests `rust/tests/text/{sep,reader}.rs` | `cargo check -p yggdryl --all-targets`; `cargo test -p yggdryl --test text sep`; `--test text reader`; `--test text arrow` (must not move); `cargo test -p yggdryl --test allocations text` (must not move); `--test iobase_calls text_costs`; `cargo bench -p yggdryl --bench text -- text_lines --quick` (level) | none |
| 2 the CSV reader | `rust/src/csv/reader.rs` (the `Tokenizer` deleted, `Records` over `Lines`, the per-read buffer and scratch, `Record::cell -> &[u8]`, `resolve_dialect`, `SNIFF_BYTES`, `HEADER_SAMPLE`), `rust/src/csv/options.rs` (`separator`/`header` tri-state, `clear_*`, `Resolved`, `reads_as`), `rust/src/csv/media.rs` (the entry's state, `write_target` and `count` resolving), `rust/src/csv/writer.rs` (the unstated roles), `rust/src/media/options.rs:2082-2095` (`csv_separator() -> Option<Option<u8>>`, `set_csv_separator(Option<u8>)`, `csv_header`, `set_csv_header`); tests `rust/tests/csv/{reader,options,media,writer}.rs`, `rust/tests/media/options.rs`, `rust/tests/allocations.rs`, `rust/tests/iobase_calls.rs` | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl --test csv reader`; `--test csv options`; `--test csv media`; `--test csv writer`; `--test media options`; `--test excel options` (must not move); `cargo test -p yggdryl --test allocations csv`; `--test iobase_calls csv` (must not move); `cargo test -p yggdryl --doc csv::options::CsvOptions` (the passed count read); `cargo bench -p yggdryl --bench media -- media/csv --quick` before and after, the numbers written into D48.4 | `CSV_COUNT_COSTS`; the three reader tests; the options/media/writer tests named above |
| 3 Python | `python/src/iomedia.rs` (`header`, `separator` tri-state, the pickle), `python/yggdryl/_native.pyi`, `python/tests/test_iomedia.py`, `python/tests/media/test_init.py`, `python/tests/typing_bindings.py` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_iomedia.py -x -q`; `... python/tests/media/test_init.py -x -q`; §3's `mypy --strict` line | `TestCsvOptions` |
| 4 Node (re-spelled) | `node/src/media/options.rs` (`separator` nullable; `header` untouched), `node/tests/records.test.js` only where an assertion breaks; then `node/index.js`, `node/index.d.ts` | `npm run --prefix node build:debug`; `node --test node/tests/records.test.js`; `npm run --prefix node test:package:debug`; `git diff --exit-code -- node/index.js node/index.d.ts` (expected to move: the `separator` declarations) | the declarations |
| 5 exchange | `scripts/check_csv_interop.py` (new), `.github/ci/rows.toml` (its job row), `.github/workflows/ci.yml` (the job, `duckdb` installed), `scripts/tests/test_ci_plan.py` | `python scripts/check_csv_interop.py` under `python/.venv` with `duckdb` installed; `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | - |
| 6 docs manifests | `docs/assets/{fix,playground}.json` | `node scripts/build_docs_fix.js && node scripts/build_docs_playground.js`, then each `--check` | both, if the `separator` signature is in the manifest |
| 7 docs, skills, inventories, contract | `docs/media/csv.md`, `docs/media/index.md`, `AGENTS.md`, `skills/yggdryl-records/{SKILL.md,references/{rust,python,javascript,formats}.md}`, `.api-inventory.txt`, `.api-bindings.txt` | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_api_inventory.py`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps | the seven `1` → `2` sites |
| 8 the chain | the whole runs `--all-features --no-fail-fast` of the three crates, clippy both lanes, `cargo doc -D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests, mkdocs, the inventories, the docs runners | one background script, one log, read once | phase 2's alone |

Then the commit (the attribution lines the harness states); one push; the run read to `CI result`;
the handoff.

## Interplay with P14 and the order

P15's Rust files are `text/{sep,reader,mod}.rs`, `text/arrow.rs` (`:659`, `Held`'s visibility, the
assembly helper), `csv/` and `media/options.rs`; P14's are `text/{options,plan,batch,entry,line}.rs`,
`mime_type/line.rs`, `implementer.rs` and `rust/fix/src/schema.rs`. Those sets are disjoint. **The
two share**: `python/src/iomedia.rs`, `python/yggdryl/_native.pyi`, `python/tests/test_iomedia.py`,
`python/tests/typing_bindings.py`, `AGENTS.md`, `skills/yggdryl-records/*`, `.api-inventory.txt` and
`.api-bindings.txt`, so every P15 anchor in them - and `arrow.rs:659`, which P14 does not touch - is
re-read on P14's tree. P14 lands first as decided; P15 is rebased on it.

## Put to the user (interpretations taken; say if another was meant)

1. **The reuse is the page and the splitter, not `TextLine`**: a CSV record is a borrowed range of
   the shared window with its cells ranges of it, cut by the splitter's new quote-aware mode. Stated
   plainly: the transport was already shared before P15; the cutter is CSV's own grammar moved into
   `text/sep.rs`, not an existing implementation replacing it; what is newly shared is the window,
   its refill and the part assembly. The literal "a CSV record is a `TextLine`" was refused for the
   semantics (a line is an event, a row is not) and for the cost bar.
2. **An unstated separator is inferred, for `.csv` too**: the default was `,` and is now "inferred,
   `,` preferred", so a `;` document reads as `;` with nothing stated - the one visible behaviour
   change for a caller, which moves the seven "one column" sites and one reader test. The
   alternative infers only when `,` is inconsistent over the sample.
3. **The candidates are DuckDB's four** (`,` `\t` `;` `|`) under the stated quote; the quote is never
   inferred; Python's space and `:` are not candidates (a space-separated table is a text medium's
   line).
4. **An unstated header is inferred DuckDB's way over the first 32 records** - a first row some cell
   of which does not read under its column's non-text type, or every column text, or the only row -
   which keeps today's answer on a text table and on a one-record document; under a declared field
   the first record is a header only where it spells the declared names. The alternative keeps
   `header: true` the default and infers nothing, which spares `row_size`, `column_size` and every
   append the sample typing.
5. **A header naming a column twice stays refused** at `$.header` whether stated or inferred; DuckDB
   suffixes `_1`, `_2`, which is not adopted.
6. **The tri-state is `CsvOptions`' own** - `separator`/`header` `Option`, `clear_separator`/
   `clear_header`, reached through `RecordOptions::csv_separator`/`csv_header` in `csv_quote`'s
   `Option<Option<_>>` shape - and the shared `header()`/`set_header(bool)` does not move, so Excel
   takes no new refusal and Node's `header` type stays. The alternative moves the shared accessor to
   `Option<bool>` and refuses `None` on a workbook.
7. **The `header=absent`/`present` MIME parameter is not read** (`MediaType` carries no parameter but
   the charset); the option and the inference answer it.
8. **An exchange script against Python's `csv` module, both directions, and DuckDB's sniffer, with
   its own CI job that installs `duckdb`** (a `rows.toml` and `ci.yml` change, so the push that adds
   it runs everything once); the script refuses where `duckdb` is missing rather than skipping. The
   alternative drops the DuckDB half and keeps Python `csv` alone.
9. **`CSV_COUNT_COSTS` is re-pinned once, against deltas written before counting**; a per-record
   term is a defect. The page's Performance table is regenerated by a release run or left dated
   2026-10-02 with the sentence that it predates P15; the `row_size` direction is measured, not
   assumed.
10. **The two modes keep their divergences on purpose**: a lone `\r` is content and a UTF-8 BOM is
    stepped over under the record cutter (RFC 4180, today's reader, the mark read by
    `Charset::from_bom`), a line break and the first three bytes of the first line under the line
    cutter (today's text medium).
11. **An append or merge onto a stored document follows the stored document's inferred dialect**
    (`write_target` resolves it as a read does); the alternative renders by the caller's options and
    lets a `;` document gain `,` rows.
12. **Nothing is added to Node**: `separator` is re-spelled nullable because `csv_separator()`'s
    shape moved; `header` is untouched; no new Node test and no JavaScript tab for the new
    Dialect-inference example, which is Rust and Python (AGENTS §4). The existing JavaScript tabs
    asserting one column are re-pinned to two where they already stand.
13. **The medallion's bronze layer gains no CSV intake in P15**: a CSV read produces table rows
    with no event identity (`uuid`, `transunix`, `seqnum`, `hashcode`), so bronze's keep-key
    `(transunix, crosshashcode, seqnum, hashcode)` could not take one; the user's "thus first
    medaillon layer can accept key value inputs with this metadata field too" is read as decision 33
    confines it, to key-value log lines (P14). The alternative - a CSV row read as a line of its
    own text, each row an event - is a reading D48.1's option A refused and is named here, not taken.
14. **No preamble is skipped**: a leading record that disagrees with the modal cell count lowers the
    sniff's score, and a document opening with a title line is refused by width or reads that line
    as its header, as today. The alternative is a `skip` count on `Resolved` that every door
    honours, with the header the first record after it (DuckDB's dirty rows).

## Taken (decision 36, 2026-10-10)

The user answered: `parse_keyvalues` off by default (bronze opts in); values plain unless a
repeated key, a nested tree or a JSON-opening text needs JSON; the double-quoted loose value read
as one pair (escapes raw); CSV header and separator inferred by default. Every other question here
is taken as recommended (`user_decisions.md` 36). Implement these, not the alternatives.
