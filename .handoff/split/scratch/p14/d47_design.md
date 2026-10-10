# P14: design (D47) - the text line's key-values, read from the body

The user's instruction (2026-10-10, `$S/p14/user_instruction.md:1-4`, typos the user's): "add then
also in textline generated schema a metadata map str str which can parse key values handling doubled
keyed values json serialized to list and value default json serialized ... thus first medaillon
layer can accept key value inputs with this metadata field too", amended (`:6`): "rename the text
line metadata to keyvalues instead". Read as decisions 31, 33 and 34 (`$S/user_decisions.md:196-220`):
the plain-text row a `TextLine` generates gains a `keyvalues` column, `map<utf8, utf8>`, filled by a
key-value reading of the line the options ask for; a key stated twice or more holds the JSON array
of its values in order; a value that is not plain text is held as its JSON serialization; the
medallion's bronze layer reads key-value log lines into that column; the column, the option, every
doc and the bronze table spell it `keyvalues`, never `metadata`. `$S` is `.handoff/split/scratch`.

Placement (`user_decisions.md:196-209`): P14 is its own commit, before P15 (D48, `$S/p14/d48_design.md`);
everything in the core's `text/` and `mime_type/line.rs`, one FIX arm redirected onto the core's
rule (D47.4), the Python view, the medallion test, nothing added to Node (AGENTS §4: the request
does not name Node). Read on the working tree of 2026-10-10 (`git status` dirty with other lanes'
work; every line number below is of that tree and is re-read on the commit before P14). No cargo
command was run to write this: every claim names the file and line it was read from, or the run of
`python/.venv/bin/python -I` against the installed extension that produced it (`$S/p14/` holds no
probe; the two scripts are reproduced under "What the tree holds").

**One commit, one design**: the column, the option, the reading, the Python door, the fixture and the
bronze change land together, because a bronze test reading the column needs the option and the option
is nothing without the column.

**Read "Put to the user" first**: its #1 (off by default) and #2 (plain text as itself) lean away
from the literal words, each is one line to reverse, and each names the pins a reversal moves.

## The user's asks, each mapped to a decision

| # | The ask (the user's words) | Decision |
| --- | --- | --- |
| 1 | "add then also in textline generated schema a metadata map str str" / "rename ... to keyvalues" | D47.1 (the column: `keyvalues`, `map<utf8, utf8>` keys sorted, nullable, after `body`) |
| 2 | "which can parse key values" | D47.2 (the option that asks for the reading, `parse_keyvalues`, off by default) and D47.3 (the grammar: the crate's one key-value scanner, `TextEntries::from_bytes`; what it reads today, the one gap closed) |
| 3 | "handling doubled keyed values json serialized to list" | D47.4 (the hold rule: a key stated once its text, stated twice or more the JSON array of its values in arrival order, one owner in the core, written by the core's JSON string escaper) |
| 4 | "and value default json serialized" | D47.4 (a value that is not plain text - nested pairs, or text opening the way JSON does - held as its JSON; the rule the FIX row already applies, moved into the core so the FIX row reads it from there) |
| 5 | the row header, the identity, the writer, the read-back | D47.5 (the reading is of the body past the header; `hashcode` the body's alone and unmoved; the writer consumes `body` alone; a batch read back never lands the cell) |
| 6 | Python, Node, docs, skills | D47.6 (a Python property and a `TextLine.keyvalues` reading; Node nothing; the pages) |
| 7 | "thus first medaillon layer can accept key value inputs" | D47.7 (bronze opts in through `Lake.text_options()`; the column stays in bronze's text table and never reaches a FIX table; a key-value fixture beside the capture; the pins that move) |
| 8 | the cost | D47.8 (nothing per row with the option off; with it on, the tree once, one owned text per value, one JSON render per value that is not plain text; the pins) |

## What the tree holds today (the facts the design moves)

| Fact | Where |
| --- | --- |
| One plan builds the schema and the rows: `ElementColumn::ALL` (6), `EventColumn::ALL` (9), `mimetype` under `parse_mimetype`, `body` (utf8, required), `dropped_byte_size` under `max_record_byte_size`, one nullable column per capture no event fact consumes; renames, then the duplicate-name refusal; the columns' `Vec` sized `+ 3` for the three optional columns | `rust/src/text/plan.rs:27-40` (`TextSource`), `:93-177` (`compile`: the capacity `:95-96`, element `:103`, event `:114`, `BodyType` `:125`, `Body` `:135`, `DroppedByteSize` `:143`, captures `:153`, `rename` `:174`, `refuse_duplicates` `:175`); `options.rs:539-541` (`line_plan` compiles on every call), `:621-623` (`source_field` = the plan rendered) |
| `TextPlan::projected` keeps only the columns the `select` and `filter` read, so an unselected column costs nothing per row | `plan.rs:73-84` |
| Each line is one ordered `Scalar` sequence, one `value_of` arm per `TextSource`, laid out by `arrow::rows::result_reader`, which canonicalizes every row | `rust/src/text/batch.rs:32-58`, `:70-100`, `:107-131` |
| The reverse direction: `Intake::resolve` locates each planned column by exact name, folded name, then `ALIASES`, into `positions` (`None` where absent); `Intake::land` lands every located column; `line_of` skips `body_at` alone and reads every other located column through `cell_of` into a `Scalar` before `apply` runs its arm - `Body` ignored (`:740`), `BodyType` restated (`:726-736`), a capture restated (`:752`) | `batch.rs:159-198`, `:201-210`, `:216-240`, `:247-331` (`resolve`, `positions` `:252`, `body_at` `:319-322`), `:451-470` (`land`), `:490-525` (`line_of`, `:521` the one skip), `:605` (`cell_of`), `:654-755` (`apply`) |
| A capture spelled as `body`, `dropped_byte_size` or a derived event column is refused at `$.rowheader`; the message still lists `identifiers`, which the array no longer holds | `rust/src/text/options.rs:15` (`BASE_COLUMNS`), `:28-37`, `:359-373` |
| `TextOptions` is a flat `derive(Eq, Hash, Ord)` struct; `parse_mimetype: bool` (default `false`) is the shape of a flag that adds a column | `options.rs:118-226`, `:193`, `:250` |
| A line holds what the reader cut; every other fact is a reading resolved once into a `Resolved` slot, a `set_*` stating a `Stated` fact that wins; equality, order and hash read the stated facts alone | `rust/src/text/line.rs:18-70`, `:192-210`, `:226-245`, `:284-301` |
| `TextLine::entries()`: the stated tree, else the body's own resolved once by `TextEntries::from_bytes`; `None` where the body states no pair ("an absence, never an empty tree") | `line.rs:772-788`; `rust/src/text/entry.rs:193-195`, `:212-214` |
| The tree: insertion order, a repeated key two entries; `TextEntry` holds `key`, `value`, `marked`, `entries` and nothing else, each key and value a range of the page (`body.slice`), `marked` whether the line wrote `#` in front of the key; `collect_entries` descends any value where `PairSpan::nested` finds an `=`, to `MAX_ENTRY_DEPTH` 8 | `entry.rs:18`, `:20-37`, `:151-159`, `:445-470`; `rust/src/mime_type/line.rs:772-788` (`PairSpan`, the key range excludes the mark) |
| The scanner: `pairs()` runs `memchr_iter(b'=')` over the whole line and reads a pair at each `=` through `pair_ending_at` then `pair_at`, which refuses a pair whose value opens `'` or `"` or stands at a field end and one not at a field start; a frame the line separated with SOH, an escaped SOH or `\|` is read segment by segment, each cut at its first `=`; outside a frame the loose rule - the value running to a field end (SOH, `\|`, blank, `\r`, `\n`, `]`, `)`, `}`, `,`, `;`, an escaped SOH). `located_entry_spans` is the entries walk: `pairs()` taken while before the frame (`outside`), then each segment's own pair or, where a segment is prose, `pairs()` over it. `pairs()`/`pair_at` are also what `locate_frame` and `inspect`'s `has_any_pair` read: relaxing them moves frame location and line classification for every reader | `mime_type/line.rs:584-620` (`pair_at`, `:585` the field start, `:613-616` the quote refusal), `:825-836` (`loose_span`), `:643-652` (`is_field_end`), `:856-871` (`segment_span`), `:1010-1059` (`entry_spans`, `located_entry_spans`, `outside` `:1033-1035`), `:1098-1100` (`pairs`), `:1117-1133` (`pair_ending_at`); `:178`, `:197-207` (`locate_frame` reading `pair_at`), `:669`, `:1084` (`inspect`, `has_any_pair`) |
| What the scanner answers, run through the installed extension (`TextLine(0, body).entries`): `a=1 b=2 a=3` → `a=1, b=2, a=3`; `ACCOUNT=X\|#NOPARTYIDS=3\|NOPARTYIDS=2\|NOPARTYIDS[0]=PARTYID=1••PARTYROLE=7••` → `ACCOUNT=X`, `NOPARTYIDS=3`, `NOPARTYIDS=2`, `NOPARTYIDS[0]` nesting one level; `ExecId=[00064703461GBYZ0]. OrderId=[00079132558GLXC0]` → `ExecId=[00064703461GBYZ0` (the opener kept, the closer trimmed); `level=info msg="hello world" n=3` → `level=info`, `n=3` (**the quoted value is lost**); `k="quoted v"` → none, and so is `k="v"` with no blank (`pair_at` refuses the quote; nothing reads it as `"v"`); `a=1,b=2`, `a=1;b=2` → `a=1` alone; `a=1&a=2&b` → `a=1&a=2&b`; `a=` → none; `a==b` → `a==b`; `k={"x":1} j=[1,2]` → `k={"x":1`, `j=[1`; `INFO k=v` → `k=v`; `x = 5`, prose → none; inside a frame `58=` between two separators is a pair with the empty value (`:1003-1009`) | the two probes of 2026-10-10 (`scratchpad/probe.py`, `probe2.py`; Python's `TextEntry` exposes no `marked`, `python/src/text/line.rs:206-260`) |
| The FIX row already holds the rule the user describes, for `fixentries` and `metadata`: a scalar stated once its wire text; one opening `[`, `{` or `"` its JSON string (`opens_json`); a group or component its JSON (`entry_json`, `occurrence_json`, `members_json`, which need the registry's `states_occurrences`); a key stated more than once the JSON array of its values in arrival order (`entries_map` sorting a `SmallVec` of `(key, arrival, entry)` by key then arrival, `keyed_text` rendering through `yggdryl::into_json_scalar`); read back, a value is decoded as JSON only where it opens that way | `rust/fix/src/schema.rs:33-66`, `:1245-1249` (`opens_json`), `:1286-1354` (`entry_json`, `occurrence_json`, `members_json`), `:1365-1414` (`entries_map`, `keyed_text`), `:1436-1478` (`split_unresolved`), `:3030-3081` (`row_metadata`) |
| The core's JSON writer: `into_json_scalar(&Scalar) -> Result<String>`; `Scalar.from_(["a","b"]).into_json()` prints `["a","b"]` | `rust/src/json/mod.rs:105`; the probe |
| The map datatype the market row uses: `DataType::map_of(utf8, utf8, true)` - key required, value nullable, `keys_sorted` - and `map_of` allocates on every call (the two fields' `Vec`, the struct, the `Arc<MapType>`); `Scalar::from_mapping` answers `Scalar::Map` in insertion order, one shared `Arc<[_]>`, a duplicate key refused; `declared_layout` turns a `Map` under a `SortedMap` field into `SortedMap`, and the canonical walk reports a value whose discriminant moved as rewritten | `rust/market/src/graph/market_column.rs:342-343`; `rust/src/mapping.rs:215-230`; `rust/src/scalar.rs:2007-2016`, `:1220-1222`; `rust/src/serie/datatype.rs:253-256`; `rust/src/value/canonical.rs:1424-1427` |
| `hashcode` is the XXH3-64 of the body alone; the writer consumes `body` alone (`write_arrow_reader` → `encoded_bodies`) | `line.rs:43`, `:49-56`, `:1368`; `rust/src/text/arrow.rs:1167-1176`, `:1225` |
| The FIX row already owns `metadata` (crate tag 65 037); `fix_schema_carrying` carries a text column only where no FIX column folds equal to its name (`carried`); the FIX stage reads a carried column cell by cell into `Cells` (`Vec<(SmolStr, Scalar)>`) per bronze row and clones it onto every message the row answered | `rust/fix/src/crated.rs:260`; `rust/fix/src/schema.rs:612-650`; `rust/fix/src/batch.rs:974-1003`, `:1156-1172` (`Cells`, `carried_messages`), `:1281-1286` (the cell read per row); `rust/fix/src/codec.rs:1480-1485` |
| Bronze: `parse_log_messages` is one `read_serie(options=lake.text_options(), filter=window)` keyed-appended into `bronze.record_keeping.log_messages`; `text_options()` sets `rowheader`, `timezone`, `start_rownum` and nothing else; `log_row()` is `source_field()`, `fix_row()` is `fix_schema_carrying(log_row(), fix_schema(registry))` | `python/tests/medallion.py:223-243`, `:331-351` |
| The medallion pins: `IOResult(144, 144)` for bronze, `parsed > 144`, the capture's six captures last (`names[-6:]`), the capture read from `rust/tests/support/ulbridge.log`, which the CI planner names in the `fix` leaf's `paths` (its tests `include_bytes!` it) and in `[rows.libs]` | `python/tests/test_fix.py:5729`, `:5796`, `:5235-5247`; `.github/ci/rows.toml:101`, `:159`; `scripts/tests/test_ci_plan.py:249`, `:435`; `rust/fix/tests/root/{batch,build,cfb,ulbridge,codec}.rs` |
| The cost pins over text: `read_text_lines` a constant `TEXT_LINES_ONCE` (10) over the owned copy and nothing a line - the constant counts the plan `line_plan()` compiles (`arrow.rs:162`, `:178`); the tree paid on the first ask and free on the second; a batch read back at most 8 allocations per later row, under `TextOptions::new()` | `rust/tests/allocations.rs:5355`, `:5389-5406`, `:5096-5107`, `:5111-5160` |
| The pins naming the text row's columns: Rust `with_event(&[...])`, Python `EVENT_COLUMNS + [...]`, the docs' `fields().len() == 18` | `rust/tests/text/options.rs:28-49`, `plan.rs:62,258,509,536,579,626`, `line.rs:770,1078,1643`, `arrow.rs:55`; `python/tests/text/test_init.py:35-51`, `:183,327,360,377,387,405,424,584,614,644`; `docs/media/text.md:55-56` |
| Python's `TextOptions` sets a property by name through one `set_property` match, pickles its state, and the stub lists the properties in `TextProperties`; Python's `TextLine` has `entries`, `body`, `bodytype` getters | `python/src/iomedia.rs:2709-2744` (`set_property`), `:1561-1575`, `:1891-1935`; `python/yggdryl/_native.pyi:5067-5096`, `:5334`; `python/src/text/line.rs:341`, `:468-474`, `:560` |
| Node's `TextOptions` exposes neither `parse_mimetype` nor `dedup_adjacent`; its `TextLine` exposes `entries` | `node/src/text/options.rs` (grep: no `parseMimetype`); `node/src/text/line.rs:489-491` |
| The FIX equivalence snapshot, the gate every scanner change is read against | `rust/fix/tests/root/codec.rs:3578` (`YGGDRYL_FIX_EQUIVALENCE_WRITE`), `:3769` |

## D47.1 - the column: `keyvalues`, `map<utf8, utf8>` keys sorted, nullable, after `body`

- **Name** `keyvalues` (decision 34), its display `KeyValues`, description "The key-value pairs the
  body states, each key once: a key stated once its value as text, a key stated more than once the
  JSON array of its values in order, a value that is not plain text its JSON." A capture named
  `keyvalues` is refused at `$.rowheader` as `body` is: `keyvalues` joins `BASE_COLUMNS`
  (`options.rs:15`), and the refusal's sentence (`:368`) is re-spelled to list `keyvalues` and to drop
  the stale `identifiers`.
- **Datatype** `DataType::map_of(DataType::utf8(), DataType::utf8(), true)` - key required, value
  nullable, keys sorted - the shape the market row's `metadata` (`market_column.rs:342`) and the FIX
  row's `fixentries`/`metadata` already have, so a bronze reader joins one map shape across the text,
  FIX and market rows. **Held once**: a `static KEYVALUES: OnceLock<DataType>` in `plan.rs`, cloned
  (a refcount of the `Arc<MapType>`) on every compile, because `map_of` allocates on every call
  and `line_plan()` compiles on every read, count and `source_field()` ask - which is what keeps
  `TEXT_LINES_ONCE` exact with the option on (D47.8). **Sorted, not in arrival order**, for three
  reasons: the fold of D47.4 groups the entries by key anyway, which is a sort, so sorting is free;
  a sorted Arrow map (`keys_sorted = true`) is what a reader binary-searches; and the order that
  carries meaning - which value came first - is kept inside the value, in the JSON array, where the
  key order does not. A value is `nullable` only because `map_of` makes it so; nothing writes a
  null value.
- **Nullable, never the empty map**: `None` from `TextEntries::from_bytes` is "an absence, never an
  empty tree" (`entry.rs:193-195`), and the cell is null exactly there - a line stating no pair - and a
  map of at least one entry everywhere else. The 55 of the capture's 144 lines that state no pair read
  null (the evidence lane's count over `ulbridge.log`).
- **Place**: after `body` and after `dropped_byte_size`, before the captures - a `TextSource::KeyValues`
  pushed in `compile` between `plan.rs:143-152` and the capture loop at `:153`, the capacity `+ 3`
  re-spelled `+ 4` so the `Vec` never grows. The captures stay the open-ended tail, so the pins
  that read them from the end hold (`test_fix.py:5235-5247`, `names[-6:]`), and the event block
  stays contiguous before `body`. The row is then, under the option: the six element columns, the
  nine event columns, `mimetype` where asked, `body`, `dropped_byte_size` where asked, `keyvalues`,
  the captures.
- `default_name_of` answers `"keyvalues"` for the source (`batch.rs:201-210`). **No `ALIASES` row**: a
  batch read back never lands the cell (D47.5), so an alias would locate a column only to leave it
  unread; `locate` by the default name is what lets a batch carrying `keyvalues` land without a
  refusal, and `rename_columns` renames it as it renames `body`.

## D47.2 - the option: `TextOptions.parse_keyvalues`, off by default

- One owner: `pub parse_keyvalues: bool` beside `parse_mimetype` (`options.rs:193`), default `false`
  in `new()` (`:250`), inside the options' identity (derived `Eq`/`Hash`/`Ord`, `:118`), stated and
  pickled by Rust and Python as `parse_mtime` is (Node carries neither flag, D47.6). Its doc: "Whether
  to read each line's key-value pairs into a `keyvalues` column. Off by default: on, a column is
  planned for every row and a line stating no pair reads null." The column exists exactly where the
  flag is on (`plan.rs` `compile`), so `source_field()` knows the row before a byte is read (AGENTS
  "`TextOptions::autotype` is the shape to copy").
- **Why off** (put to the user, #1): the user's "add then also in textline generated schema" reads
  as always; decision 31 reads it as "the options ask for". Off moves no existing pin - the
  `with_event` lists, the `EVENT_COLUMNS + [...]` asserts, the docs' 18, the `schemas.md` table,
  `TEXT_LINES_ONCE`, the `text_batch build/plain` benchmark - and keeps every text read that never
  asked for pairs at its cost; the capture pipeline's FIX pins (`rust/fix/tests/root/ulbridge.rs:2418-2424`
  composes the carried root from `source_field()`) stay as they are, and bronze opts in by one line
  (D47.7). Always-on is one line of `new()` and moves: every `with_event`/`EVENT_COLUMNS` list
  (`rust/tests/text/{options,plan,line,arrow}.rs`, `python/tests/text/test_init.py`), `docs/media/text.md:56`'s
  `18` to `19`, `docs/graph/schemas.md`'s default table, `TEXT_LINES_ONCE` by nothing (the
  datatype is a refcount) but the `<= 8` read-back pin by nothing either (the cell is never landed),
  the `build/plain` number by the second pass over every body, and the FIX capture's carried root
  (`ulbridge.rs:2418-2424`) by one column - which D47.7's exclusion then takes back off.
- No pair-separator or dialect option in P14: the grammar is the scanner's (D47.3), and a dialect
  knob would be a second grammar's door. A `pairs_separator` is named under "Put to the user" (#8) as
  the one addition the research (Logstash `field_split`, Splunk `DELIMS`, a query string's `&`) would
  justify, left out until a line shape asks for it.

## D47.3 - the grammar: the crate's one scanner, and the one gap closed

The reading is `TextLine::entries()` projected: the tree `TextEntries::from_bytes` resolves once
(`line.rs:780-788`), which is `mime_type::line::entry_spans` (`:1010-1059`) - the frame walk for a
line that named a separator (SOH raw or escaped, `|`), the loose rule everywhere else, `#` read as
the mark, a pair-shaped value descended to eight levels. **No second parser** (AGENTS "no second
schema, dispatcher, parser"): what the column reads is what `entries` answers, so a FIX bridge row,
a FIX frame, a bare `k=v` run and a sentence carrying one pair all read as they already read, and a
value a caller asks `entry_by_path` for is byte for byte the text the column holds (the one reading
of escapes, below).

What the scanner reads today, from the probe: a bridge row's every field; a FIX frame's every tag
(group tags repeated: `448`, `447`, `452` four times each on line 6 of the capture, the evidence
lane's count); a whitespace-separated run (`a=1 b=2 a=3`); the pairs inside a sentence. What it does
not read: a double-quoted value (`msg="hello world"` is **dropped**, `k="quoted v"` and `k="v"` are
no pair), a run separated by `,`, `;` or `&` (the first pair alone), an empty loose value (`a=` is
no pair, by design: `mime_type/line.rs:1003-1009`), a bracketed value's closer (`ExecId=[0006...]`
keeps the opener and loses the `]`, `trimmed_segment_end`/`is_field_end`).

**The one gap closed in P14: the double-quoted loose value.** The reason: a "key-value log line" in
the user's sense (decision 33) is logfmt-shaped - Go's `slog`, Heroku, Logstash's `kv` - and logfmt
quotes any value holding a blank (`go-logfmt/encode.go:173`, the research's fact); a reading that
drops `msg="hello world"` reads the one field such a line is written for. **The rule lives in the
entries walk alone** - the `outside` iterator of `located_entry_spans` (`:1033-1035`) and the loose
fallback of a prose segment (`:1050-1054`) - as a quoted arm those two call beside `loose_span`;
`pair_at`, `pairs()` and `pair_ending_at` are **not touched**, so `locate_frame`, `inspect`,
`has_any_pair`, `bodytype`/`parse_mimetype` and the codec's frame location read exactly what they
read today (a line whose only pair is quoted still classifies as it does now; put to the user, #6).
The arm: at an `=` `pairs()` refused because the next byte is `"`, and only where the `=` closes a
key `pair_at` would otherwise take, the value runs from the byte after the quote to the closing
quote, where **a backslash consumes the next byte** (`\"` and `\\` do not close, and `"a\\"` closes
at its last quote); the quotes excluded; the escapes **kept as written** - reading (a): the value is
a range of the page as every entry is (`entry.rs:20-24`), `TextEntry` carries no `quoted` fact, and
there is no decode anywhere, so `TextEntry::value`, `entry_by_path` and the column answer one text
and no second escape reader stands beside JSON's. An unterminated quote is **no pair**, today's
answer. **Two consequences the walk states**: every `=` inside a quoted span it consumed is skipped
(the outside walk advances past the span's end, so `msg="a b=c" n=3` is `msg` → `a b=c`, `n` → `3`),
and a quoted value is a leaf - `PairSpan::nested` is not asked of it, so `msg="user=bob logged in"`
holds that text and never the tree `{"user":"bob"}`. A frame's segment (`segment_span`, `:856-871`)
is not touched: inside a `|`- or SOH-separated frame a value runs to the separator and a quote is
content, as a FIX `58=` text is.

**The cost of finding the closing quote**: one bounded `memchr2(b'"', b'\\')` walk from the opener,
and a per-line watermark "no unescaped closing quote at or after X" kept by the walk, so a line of
many unterminated `k="` openers scans its tail once, not once per opener - linear in the line,
pinned by an adversarial row (D47.8). With no `="` on the line the arm costs one byte compare per
refused `=`, which is the comparison `pair_at` already makes.

**What moves, whatever the option says** - the quoted arm is in the walk every reader of entries
shares, so these are public behaviour changes of `TextLine::entries()`, `entry_by_path`, Python's
`TextLine.entries` and the FIX codec's prose walk (`text_entries_from_bytes_direct_located`,
`codec.rs:1495-1497`, `implementer.rs:818-822`), stated on the page and in AGENTS: `msg="a b"` gains
the pair `msg` → `a b` (none before); `k="v"` gains `k` → `v` (none before); and a pair that stood
inside a quoted value - `msg="a b=c"` read `b` → `c` today, because a blank is a field start - is
now text of `msg` and no pair. Nothing else moves: an unquoted pair reads as it did. **The pin
that decides whether the rule lands**: the FIX equivalence snapshot (`rust/fix/tests/root/codec.rs:3578`,
regenerated under `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`), `rust/fix/tests/root/ulbridge.rs`, the
`rust/tests/mime_type/line.rs` classification pins and the text `bodytype` pins must not move. If
one cell moves, the quoted rule is withdrawn from P14 and put to the user with the cell, never
landed over a moved pin. The three readings above are added to `rust/tests/mime_type/line.rs` and
`rust/tests/text/entry.rs` red first.

**Not closed in P14** (put to the user, #7): the `,`/`;`/`&` pair separators (a query string is a
document, not a log line; Splunk's `DELIMS` is a configured dialect), the bracket trim
(`[0006...]` → `0006...`, Logstash's `include_brackets`), the empty loose value. Each is a change to
the scanner the FIX codec reads by, each moves the snapshot's reading of prose, and none is asked for
by the capture or by a logfmt line.

## D47.4 - the hold rule: text once, a JSON array twice, JSON where it is not plain text - one owner

The rule the FIX row applies to `fixentries` and `metadata` (`schema.rs:33-66`) is the rule the
user describes, and after P14 it has **one owner in the core**, `rust/src/text/entry.rs`, which the
FIX row reads through `implementer` - never a second copy that "happens to agree":

- `KeyValueFold` (crate-private, `entry.rs`, exported to the FIX crate as
  `implementer::keyvalue_fold`): takes the pairs of one level as `(key: &str, arrival, Rendered)`,
  `Rendered::Text(&str)` a leaf's text or `Rendered::Json(&str)` a value already rendered as JSON by
  its owner; sorts them by key then arrival in one `SmallVec<[_; 64]>` of **borrowed** keys (the
  `entries_map` shape, `schema.rs:1369-1373`, the owned `SmolStr` per entry it built before sorting
  gone - a key is owned once per distinct key, after the sort); files each run of one key once;
  and answers the map as **`Scalar::SortedMap`** (the `Map` holder `from_mapping` fills, under the
  sorted variant), so the declared layout and the value agree and the per-row canonical walk
  answers "unchanged" - built as `Scalar::Map`, `declared_layout` would move the discriminant and
  `result_reader` would rebuild every row with pairs (`serie/datatype.rs:253-256`,
  `canonical.rs:1424-1427`).
- `opens_json(text)` moves here from `schema.rs:1247-1249`, deleted there.
- The renders are written **straight into one reusable `String` per read** through the JSON
  writer's string escaper (`json/`'s, reached crate-privately), never through a `Scalar` sequence
  and `into_json_scalar`: a key stated twice is `[` + each value as a JSON string or as the JSON it
  already is + `]`, a JSON-opening leaf is its JSON string, and the result is copied once into the
  value `Str` (inline under `Str`'s inline capacity, one allocation past it). The FIX non-group
  arm (`keyed_text`'s scalar and once/twice logic, `schema.rs:1396-1414`) calls the fold with its
  leaves as `Text` and its group/component values as `Json` rendered by `entry_json` exactly as
  today; `entry_json`, `occurrence_json`, `members_json` and `states_occurrences` stay FIX's, since
  they need the dictionary. FIX's copy of the leaf/repeat/opens-JSON logic and `opens_json` are
  deleted in the same commit, and the equivalence snapshot is the gate that proves no FIX cell moved.

`TextEntries::into_keyvalues(&self) -> Result<Scalar>` (public, `entry.rs`, beside `from_bytes`;
`into_*` because it builds another representation and allocates by contract - a bare noun is a
borrowed lookup, AGENTS vocabulary) folds the tree's **top level** through `KeyValueFold`:

1. **The key** is the key as the line wrote it, **byte-exact, never folded**: `#`-marked keys keep
   their mark - `#NOPARTYIDS` and `NOPARTYIDS` are two keys - and `Level=info level=warn` is two
   keys, although the column's `locate` and the FIX side fold names. The mark "is a fact about what
   the line wrote and not a rendering of it" (`entry.rs:26-30`), and the capture shows why it must
   stay: line 11 writes `#NOPARTYIDS=3` beside `NOPARTYIDS=2` (the evidence lane's grep), a
   restatement and a value, not one key said twice; folding them would make the bridge's two
   numbers an array `["3","2"]` under a key that never carried both. Twenty-five of the capture's
   bridge rows restate keys this way. (Put to the user, #5: the alternative merges the two under the
   bare key.)
2. **A key stated once**, whose value is a leaf (no nested tree) and whose text does not open the
   way JSON does (`[`, `{`, `"`): the value text as read - a quoted loose value without its quotes
   and with its escapes as written (D47.3).
3. **Any other value** is its JSON: a leaf whose text opens like JSON as its JSON **string**
   (`"[00064703461GBYZ0"`, quotes and all - which is what makes the map unambiguous, below); a
   nested tree as the JSON object of its entries, each member under its key, a member stated twice
   the array of its values, recursively - the nested level sorted by the same stack `SmallVec`,
   no `BTreeMap`, no `Scalar::from_struct`, written into the same `String`.
4. **A key stated two or more times**: the JSON array of its values in arrival order, each value
   rendered as 2 or 3 renders it as a JSON value - a plain text a JSON string, a nested tree an
   object - so `a=1 b=2 a=3` holds `a` → `["1","3"]`, `b` → `2`.
5. A line stating no pair is the null cell before any of this runs.

| Value shape | The cell holds |
| --- | --- |
| `a=1` once, plain | `a` → `1` |
| `a=1 ... a=3` | `a` → `["1","3"]` |
| `ExecId=[0006` (opens `[`) | `ExecId` → `"[0006"` |
| `NOPARTYIDS[0]=PARTYID=1••PARTYROLE=7••` (nested) | `NOPARTYIDS[0]` → `{"PARTYID":"1","PARTYROLE":"7"}` |
| `#A=1 A=2` | `#A` → `1`, `A` → `2` |
| `msg="hello world"` | `msg` → `hello world` |
| `msg="a \"b\""` | `msg` → `a \"b\"` (escapes as written) |
| `58=` inside a frame, once / twice | `58` → `""` / `58` → `["",""]` |
| `a=` loose, bare `a` | absent (no pair) |
| `Level=info level=warn` | two keys, as written |

**Why the map is unambiguous.** A map value is read by its first byte, exactly as the FIX row reads
its `metadata` back (`split_unresolved`, `schema.rs:1436-1478`): `"` a JSON string, `[` an array (a key
stated twice), `{` an object (a nested value), anything else plain text. A value stated once that
*looks* like an array - the research's open question - cannot collide, because a plain text opening
`[` is written as its JSON string and therefore opens with `"`. The bracketed-prose quirk reads under
the same rule: `ExecId=[00064703461GBYZ0` lands as `"[00064703461GBYZ0"`, the scanner's own reading
written honestly (put to the user, #3 and #4).

**"value default json serialized"**: read as FIX reads it - a value that is not plain text is held as
its JSON, plain text as itself - rather than every value JSON-encoded (`"1"` for `1`). One rule, one
owner, for the text, FIX and market maps; the alternative is put to the user (#2) and is one arm
of the fold's leaf case.

**Nested trees** are folded to JSON objects rather than held raw (put to the user, #9): a bridge's
`NOPARTYIDS[0]=PARTYID=1743045.007••PARTYIDSOURCE=proprietary/customcode••PARTYROLE=executingfirm••`
(capture line 11; 63 of the 89 pair-bearing lines nest, the evidence lane's count) reads as
`{"PARTYID":"1743045.007","PARTYIDSOURCE":"proprietary/customcode","PARTYROLE":"executingfirm"}`,
the user's "json serialized" for a value that is not plain text; the raw text is the alternative
and is `TextEntries::from_bytes_direct`'s reading, one level.

## D47.5 - the row header, the identity, the writer, the read-back

- **The reading is of the body past the header.** `entries()` reads `self.body` (`line.rs:786`), and
  the body is the line past what the row header matched (the header comes off where the line is made,
  AGENTS "Plain-text rows"); the captures have their own columns and enter no pair. A `k=v` inside
  the header's match is therefore not in the map - a header that captures `level` and a body saying
  `level=info` give the capture column and the map entry, two columns, two facts.
- **`hashcode` and `uuid` do not move**: the code is the body's XXH3-64 (`line.rs:1368`) and the
  reading adds nothing stated, so a line read with and without the option has one `hashcode`, one
  `uuid`, one `crossuuid` - pinned in `rust/tests/text/line.rs`. The bronze key
  `(transunix, crosshashcode, seqnum, hashcode)` (`medallion.py:96`) therefore keys the same rows
  with the column on.
- **The writer** consumes `body` alone (`arrow.rs:1167-1176`, `encoded_bodies` `:1225`); the column
  is ignored on write exactly as the captures are, since the body holds the pairs it was read from.
- **The read-back never lands the cell**: `Intake::resolve` (`batch.rs:278-331`) leaves
  `TextSource::KeyValues` at position `None` whatever the batch carries, so `Intake::land` never
  lands the column and `line_of` never reads it - the one skip beside `body_at` is not enough,
  because every located column goes through `cell_of` into a `Scalar` before `apply` is reached
  (`:490-525`), and a landed map would cost an `Arc<[_]>` and one `Str` per key and value above
  the inline size per row, thrown away by an ignoring arm. The cell is a reading of the body and the
  line re-derives it on its first ask; restating it would mean parsing JSON back into a
  `TextEntries` (a value the map cannot name a page range for). `locate` (`:216-240`) still finds the
  column by name, so a batch carrying it lands without a refusal. Pinned: a batch carrying
  `keyvalues` read back with `parse_keyvalues` on costs the same `<= 8` per later row
  (`allocations.rs:5154-5158`, which today runs under `TextOptions::new()` and so cannot see it).
- `TextLine::into_keyvalues(&self) -> Result<Option<Scalar>>` (public, `line.rs`, beside
  `named_captures` `:1291-1306`): `entries().map(TextEntries::into_keyvalues)` transposed - **built on
  each ask, no slot of its own**: the tree it reads is the resolved slot, the map is what a batch
  asks once per row and a caller holding a `TextLine` asks rarely, and a `OnceLock<Option<Scalar>>`
  on every line would widen every line of every read for that one caller. `.api-inventory.txt`
  gains the three rows (`TextOptions.parse_keyvalues`, `TextEntries::into_keyvalues`,
  `TextLine::into_keyvalues`).

## D47.6 - Python, Node, docs

- **Python** (`python/src/iomedia.rs`): `parse_keyvalues` getter/setter on `TextOptions` beside
  `parse_mtime` (`:3074`), a `"parse_keyvalues"` arm in `set_property` (`:2709-2744`) so
  `read_serie(parse_keyvalues=True)` and `TextOptions(parse_keyvalues=True)` work by name (AGENTS
  "Defaults in the signature"), the pickle state (`:1561-1575` written, `:1891-1935` read) carrying
  it, `TextProperties` (`_native.pyi:5067-5096`) and `class TextOptions` (`:5334`) stating it;
  `TextLine.keyvalues -> dict[str, str] | None` (`python/src/text/line.rs` beside `entries` `:560`,
  the property over `into_keyvalues` as the other `into_*` readings cross), the map crossed as a
  `dict` through the one `Scalar.as_py` boundary; `.api-bindings.txt:24,64` re-spelled. Tests:
  `python/tests/text/test_init.py` (the schema names under the option, `EVENT_COLUMNS + ["body",
  "keyvalues", ...]`), `test_line.py` (the readings of D47.4's table on their bodies, `None` on
  prose), `python/tests/test_iomedia.py` (the pickle round trip carries the flag).
- **Node**: no door (AGENTS §4; the request names Node nowhere). Node's `TextOptions` carries neither
  `parse_mimetype` nor `dedup_adjacent` today and gains nothing; `node/index.js`/`index.d.ts` do not
  move unless a listed signature did (none does); `npm run --prefix node build:debug` and
  `node --test node/tests/text/line.test.js` prove the doors still build - and `entries` there
  reads the quoted arm, so a Node test stating a quoted pair's absence is re-pinned where one
  exists (none found by grep; verified at phase 5). The two docs manifests (`docs/assets/fix.json`,
  `playground.json`) are unchanged for the same reason and `--check`ed.
- **Docs**: `docs/media/text.md` - the Overview's Settings row (`parse_keyvalues`, "Rust and Python;
  JavaScript has no `parse_keyvalues`") and Columns row (`keyvalues` after `dropped_byte_size`, under
  `parse_keyvalues`), a "Key-values" section after Read stating D47.3's grammar in one paragraph and
  D47.4's rule as the table above, one Rust and one Python tab over a four-line buffer (`a=1 b=2
  a=3`, a bridge row with `#` under the loose walk, a logfmt line with a quoted value, prose)
  asserting the four cells, Edges (null where no pair; the column ignored on write and never landed
  on read-back; `hashcode` unmoved; **the quoted value now read by `entries`, `entry_by_path` and
  Python's `TextLine.entries` whatever the option, a pair inside a quoted value no longer a pair of
  its own**); `docs/graph/schemas.md:85-113` - a row `16 | keyvalues | map<utf8, utf8> | KeyValues |
  under parse_keyvalues` and the captures row `17...`, with the bullets re-spelled; `AGENTS.md`
  "Plain-text rows" ("then `keyvalues: map<utf8, utf8>` under `parse_keyvalues`, a key stated once
  its text, stated twice the JSON array of its values, a value that is not plain text its JSON, the
  rule `text/entry.rs` owns and the FIX row's maps read through `implementer`") and the
  `mime_type/line.rs` scanner sentence (a double-quoted loose value read to its closing quote, a
  backslash consuming the next byte, escapes kept); `skills/yggdryl-records/SKILL.md:169` item 14
  and `references/rust.md:449`, `python.md:347`; `docs/fix/arrow.md:528-548` unchanged (the option
  off, and the FIX tables never hold the column). The Performance table of `text.md` is not edited
  by hand: a `build/keyvalues` row is added only by a release run of the bench D47.8 adds.

## D47.7 - bronze: the option on in `Lake.text_options()`, the column bronze's alone, a fixture beside the capture

- **One line on**: `options.parse_keyvalues = True` in `Lake.text_options()` (`medallion.py:223-229`),
  so `log_row()` (`:232`) gains the column and `bronze.log_messages` is created from it (`table_of`,
  `:251-262`).
- **The column never reaches a FIX table** (one owner per fact; the user asked for the first layer
  alone). For a bridge row or a FIX frame every pair the map holds is already that row's
  `fixentries` (a resolved field) or `metadata` (an unresolved key, under the same rule), so a
  carried `keyvalues` would hold each arrival twice under two columns on `bronze.fix_messages` and
  `silver.fix_messages`, and would cost what no "one refcount" claim covers: the FIX stage reads a
  carried column cell by cell into `Cells` per bronze row (`batch.rs:1281-1286`, a `Scalar::Map`
  built per row), and each of the k messages a frame line splits into lays the whole map out again
  in the FIX row, k times. So `fix_row()` (`:237`) is `fix_schema_carrying(carried_root,
  fix_schema(registry))` with `carried_root` being `log_row()` **less `keyvalues`** (one exclusion in
  `medallion.py`), and the FIX stage's read of bronze selects it out (`select` with the star-exclude
  the plan grammar carries, `* exclude (keyvalues)`), so the cell is neither landed nor read there.
  Pinned in `test_fix.py`: `"keyvalues" in bronze.log_messages`' root and in **neither** FIX table's.
  `FIX_PIPELINE_COSTS` therefore does not move. Carrying it into silver is put to the user (#11) as
  its own reading. In Rust, `fix_schema_carrying` would carry a `keyvalues` column a caller hands
  it - the committed dictionary has no field named `keyvalues`, so `Columns::resolve` makes no fill
  of it (`batch.rs:988-1003`) - pinned in `rust/fix/tests/root/batch.rs` ("a carried `keyvalues`
  cell rides the row and fills no field"), so a dictionary that one day names one is the pin going
  red and not a silent fill. `declared()` (`:155-181`) needs no change.
- **The fixture**: `rust/tests/support/keyvalues.log`, beside `ulbridge.log`, never inside it (every
  144/151/41 pin, the scale and benchmark corpora read the capture by line count). Eight to twelve
  lines in the capture's row-header shape (`ULBRIDGE_ROWHEADER`, `rust/fix/src/ulbridge.rs:119`:
  clock, `[thread-session:ctx:seq]`, `[plugin]`, `(LEVEL)`, body), clocks inside the test's
  2026-08-14 window on a quarter-hour boundary, bodies stating: a run of unique pairs; a `#`
  restatement **under the loose walk** (`#A=1 A=2`, blank-separated - `#A=1|A=2` would be a
  bridge-classified frame, since `inspect` reads a `#`-marked key as a bridge's own marker,
  `mime_type/line.rs:658-662`, and `|` is a named separator); a key stated three times; a nested
  value; a logfmt line with a quoted value holding a blank; a value opening `[`; a line of prose
  with no pair. **No frame a codec reads as a message**: no `8=FIX`, no `MSGTYPE=`, no `#`-marked
  key inside a `|`- or SOH-separated run, so the FIX stage answers no message for them and `parsed >
  144` (`test_fix.py:5796`), `written["silver.fix_messages"].written_rows == 41` (`:5804`) hold -
  verified at phase 4 by the run; a fixture line that yields a message is re-spelled, never the pin.
  The planner learns the path in **`[rows.libs]` alone** (`.github/ci/rows.toml:159`, with its row in
  `scripts/tests/test_ci_plan.py` beside `:249`/`:435`): no `yggdryl-fix` source includes the
  fixture, so the `fix` leaf's `paths` (`:101`) does not name it.
- **The test**: `test_the_medallion_pipeline_lands_every_stage_over_two_catalogs`
  (`test_fix.py:5649-5903`) writes the fixture as a third object, `logs/kv-0.log`, beside
  `bridge-0.log` and `bridge-1.log` (`:5666-5668`), and pins: `written["bronze.log_messages"] ==
  IOResult(144 + N, 144 + N)` (`:5729`), the rerun `IOResult(144 + N, 0, 144 + N)` and `row_size ==
  144 + N` (`:5882-5884`, `:5898-5901`), the roots above, and - read back from bronze through
  `rows_of` - the cells of the fixture's lines: the triple key's `["a","b","c"]`, the `#` key kept,
  the nested object, the quoted value's text, the `"[`-opening string, `None` on the prose line and
  on every capture line that states no pair. An existing bronze table written before P14 lacks the
  column; P14 evolves no stored schema (the test's catalogs are fresh folders), and a lake holding
  one re-creates it - put to the user (#12).

## D47.8 - the cost

- **The option off: nothing but the quoted arm.** The plan has no `KeyValues` column (`compile`),
  `value_of` has no arm to run, `source_field()` is one column shorter, and no slot was added to
  `TextLine`. `TEXT_LINES_ONCE` (10, `allocations.rs:5355`), the `<= 8` read-back pin (`:5154-5158`),
  `text_costs` (`iobase_calls.rs:1111-1158`) and `text_batch build/plain` do not move; the plan's
  `Vec` is one allocation whatever its length. The quoted arm is the one part of P14 every reader
  of entries pays, and it allocates nothing: the watermark is two `usize` on the stack.
- **The option on, per row**: the tree, once (`first_ask > 0`, free after, `allocations.rs:5102-5107`;
  one `Vec` per level, `entry.rs:178-184`); the fold's `SmallVec` of borrowed keys on the stack up to
  64; one `Vec<(Scalar, Scalar)>`; one owned key per **distinct** key (`SmolStr`, inline to 23
  bytes - the capture's keys are) and one `Str` per value held as text (copied off the page - the
  map must own what Arrow copies next, inline under `Str`'s capacity); one copy into a `Str` per
  value rendered as JSON (a key stated twice, a nested tree, a JSON-opening text) from the read's
  one reusable `String`, which grows once per read and never per row; the map's one shared
  `Arc<[_]>`. A line stating no pair: zero allocations (the tree answers `None`, the cell is
  `Scalar::Null`) - **and one more pass over its body** (`entry_spans`: `locate_frame` plus the `=`
  walk), so a pairless line costs time it did not cost off. Nothing in the decode: `read_text_lines`
  never asks the tree, and the datatype is a refcount of the `OnceLock`, so
  `reading_text_lines_costs_a_constant_and_nothing_a_line` holds **exactly** with the option on.
- **Under a `select` that does not read it**: nothing (`TextPlan::projected`, `plan.rs:73-84`);
  `row_size` builds no row (`arrow.rs:183-212`, the counting rows keep no body).
- **New pins** (`rust/tests/allocations.rs`, each red first, two corpus sizes): (a) a text read with
  the option on costs `read_text_lines` exactly `TEXT_LINES_ONCE` as off; (b) `into_arrow_batch`
  with the option on over lines stating no pair costs **a constant per batch and nothing per row**
  past what off costs - an allocation claim only, the planned column's own arrays per batch being
  that constant; (c) over lines of N once-stated plain pairs it costs the tree's one `Vec`, N `Str`
  values and D distinct `SmolStr` keys above the inline size, one `Vec<(Scalar, Scalar)>`, one
  `Arc<[_]>`, and nothing else - the number counted at phase 1 and written per value shape with its
  sentence; (d) a key stated twice, a nested tree and a `[`-opening text each cost one `Str` over
  (c) and the reusable `String` nothing after the first row; (e) a batch carrying `keyvalues` read
  back with the option on holds the `<= 8` per later row; (f) a line of 10 000 unterminated `k="`
  openers is read by `entries()` in time linear in its length (a wall-clock bound beside the
  allocation count, the adversarial row of D47.3). **Must not move**: `TEXT_LINES_ONCE`,
  `OWNED_COPY_COSTS`, the `<= 8` pin, the tree's first-ask/second-free pair, `text_costs`,
  `FIX_PIPELINE_COSTS` (`rust/fix/tests/allocations.rs`, the capture read with the option off and
  the column never carried), `a_real_line_costs_the_same_at_every_stage_every_time`, the FIX
  landing rows.
- **Benches**: `text_batch build/keyvalues` over `pair_corpus()` (`rust/benchmarks/text/line.rs:366`)
  with `parse_keyvalues` on, beside `build/plain`, and `build/keyvalues_pairless` over a pairless
  corpus, the time gate for pin (b) - `cargo bench -p yggdryl --bench text -- text_batch --quick`
  before and after, direction only; `build/plain` must read the same; `cargo bench -p yggdryl-fix
  --bench fix -- --quick` level (the prose walk's quoted arm).
- **FIX side**: nothing carried (D47.7), so no FIX allocation row moves; the equivalence snapshot
  and `--test root store` (the hash) prove the fold's move changed no cell.

## Tests and the pins that move, each with its sentence

| Pin | Moves to | Sentence |
| --- | --- | --- |
| `rust/tests/text/options.rs` the reserved-name refusal (`:359-373` of `options.rs`) | `keyvalues` refused as a capture name; the message lists it and no longer `identifiers` | "keyvalues is the row's column, D47.1" |
| `rust/tests/text/plan.rs` new | the column after `body`/`dropped_byte_size` and before the captures under the option, absent off, projected away under a `select`, renamed by `rename_columns` | "D47.1, D47.2" |
| `rust/tests/text/batch.rs` new | every row of D47.4's table as a cell, null on prose; a batch read back never lands the cell and the row is not rebuilt (the value answers `SortedMap`) | "D47.4, D47.5" |
| `rust/tests/text/entry.rs` new | `TextEntries::into_keyvalues` on a hand-built tree; `msg="a b=c" n=3` two entries; `msg="user=bob"` a leaf; `"a\\"` closing at its last quote; an unterminated quote no pair | "D47.3, D47.4" |
| `rust/tests/text/line.rs` new | `TextLine::into_keyvalues`; `hashcode`/`uuid` equal with the option on and off | "the identity is the body's, D47.5" |
| `rust/tests/mime_type/line.rs` (the mirror of `mime_type/line.rs`) | the quoted loose value in `entry_spans`; the three moved readings of D47.3; `inspect`/`locate_frame` unchanged on the same lines | "a logfmt value holds a blank, D47.3" |
| `rust/tests/allocations.rs` new rows (a)-(f) | counted at phase 1 | "D47.8" |
| `rust/fix/tests/root/schema.rs` | the non-group arm through the core fold: the same `fixentries`/`metadata` cells (the snapshot the gate) | "one owner of the hold rule, D47.4" |
| `rust/fix/tests/root/batch.rs` new | a carried `keyvalues` cell fills no field | "D47.7" |
| `python/tests/text/test_init.py`, `test_line.py`, `test_iomedia.py` new | the names under the option, the dict reading, the pickle | "D47.6" |
| `python/tests/test_fix.py:5729,5882-5884,5898-5901` | `144 + N`, N the fixture's lines; the column in bronze's root and in neither FIX table's | "the key-value object joins the capture, D47.7" |
| `.github/ci/rows.toml:159`, `scripts/tests/test_ci_plan.py` | the fixture's path in `[rows.libs]` | "D47.7" |
| `.api-inventory.txt`, `.api-bindings.txt:24,64` | the three names, the property and the reading | "D47.5, D47.6" |

**Must not move**: every `with_event` list and `EVENT_COLUMNS + [...]` assert (the option is off);
`docs/media/text.md:56` (`18`); `docs/graph/schemas.md`'s default table; `TEXT_LINES_ONCE`,
`OWNED_COPY_COSTS`, the `<= 8` pin; `text_costs`; `FIX_PIPELINE_COSTS`; the FIX equivalence snapshot
and `rust/fix/tests/root/ulbridge.rs` (D47.3's and D47.4's gate); the classification pins of
`rust/tests/mime_type/line.rs` and the text `bodytype` pins; the dictionary hash and the crate dump
(no FIX field, no dump change); `parsed > 144` and `== 41`; `names[-6:]`; every book, instrument and
market pin; Node's loader and declarations.

## Implementation plan

One commit, one push, CI read to `CI result`; phases in AGENTS order, disjoint files per worker, the
whole run leading the chain once phase 2 settles. `cargo check -p yggdryl --all-targets --keep-going
--message-format=short` drives the sweep; nobody runs `cargo fmt --all` while a worker edits. A
rustdoc filter names the module that defines the item (AGENTS' `graph::book::BookEvent::keyed`), and
the passed count is read: a filter matching nothing reports `ok` over zero tests.

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move |
| --- | --- | --- | --- |
| 1 core | `rust/src/text/options.rs` (the flag, `BASE_COLUMNS`, the refusal's sentence), `plan.rs` (`TextSource::KeyValues`, its push, the `OnceLock` datatype, `+ 4`), `batch.rs` (`value_of` arm, `default_name_of`, `resolve` leaving it unlocated), `entry.rs` (`KeyValueFold`, `opens_json`, `TextEntries::into_keyvalues`, the renders), `line.rs` (`TextLine::into_keyvalues`, the doc table row), `rust/src/implementer.rs` (`keyvalue_fold`), `rust/src/mime_type/line.rs` (the quoted arm of the entries walk); tests `rust/tests/text/{options,plan,batch,entry,line}.rs`, `rust/tests/mime_type/line.rs`, `rust/tests/allocations.rs` | `cargo check -p yggdryl --all-targets`; `cargo test -p yggdryl --test text options`; `--test text plan`; `--test text batch`; `--test text entry`; `--test text line`; `cargo test -p yggdryl --test mime_type line`; `cargo test -p yggdryl --test allocations text`; `--test iobase_calls text_costs` (must not move); `cargo test -p yggdryl --doc text::line::TextLine::into_keyvalues`; `--doc text::entry::TextEntries::into_keyvalues` (each with its passed count read); `cargo bench -p yggdryl --bench text -- text_batch --quick` | the refusal's sentence; the new rows |
| 2 FIX | `rust/fix/src/schema.rs` (`keyed_text` over `implementer::keyvalue_fold`; `opens_json` and the leaf/repeat copy deleted), `rust/fix/tests/root/{schema,batch}.rs` | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --test root schema`; `--test root batch`; `cargo test --locked -p yggdryl-fix --test root equivalence` (**must not move**; if a cell does, the quoted arm is reverted in phase 1 or the fold's move is read for the defect, and the cell put to the user); `--test root ulbridge`; `cargo test -p yggdryl-fix --test allocations a_real_line_costs_the_same_at_every_stage_every_time` (must not move); `--test root store` (the hash must not move); `cargo bench -p yggdryl-fix --bench fix -- --quick` (level) | none |
| 3 Python | `python/src/iomedia.rs`, `python/src/text/line.rs`, `python/yggdryl/_native.pyi`, `python/tests/text/{test_init,test_line}.py`, `python/tests/test_iomedia.py`, `python/tests/typing_bindings.py` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/text -x -q`; `... python/tests/test_iomedia.py -x -q`; §3's `mypy --strict` line | none |
| 4 bronze | `rust/tests/support/keyvalues.log` (new), `.github/ci/rows.toml` (`[rows.libs]`), `scripts/tests/test_ci_plan.py`, `python/tests/medallion.py`, `python/tests/test_fix.py` | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py`; `python/.venv/bin/python -m pytest python/tests/test_fix.py -k medallion -x -q`; `... -k capture_pipeline -x -q` (must not move: it reads its own objects) | `test_fix.py:5729,5882-5884,5898-5901` |
| 5 Node | nothing edited unless a `line.test.js` assertion names a quoted pair's absence | `npm run --prefix node build:debug`; `node --test node/tests/text/line.test.js`; `git diff --exit-code -- node/index.js node/index.d.ts`; `node scripts/build_docs_playground.js --check`; `node scripts/build_docs_fix.js --check` | none expected |
| 6 docs, skills, inventories, contract | `docs/media/text.md`, `docs/graph/schemas.md`, `AGENTS.md`, `skills/yggdryl-records/{SKILL.md,references/rust.md,references/python.md}`, `.api-inventory.txt`, `.api-bindings.txt` | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_api_inventory.py`; `python scripts/check_docs_examples.py --lang rust` and `--lang python` as chain steps | - |
| 7 the chain | the whole runs `--all-features --no-fail-fast` of the three crates, clippy both lanes, `cargo doc -D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests, mkdocs, the inventories, the docs runners | one background script, one log, read once | phases 1 and 4's alone |

Then `cargo fmt --all` once; the commit (the attribution lines the harness states); one push; the run
read to `CI result`; the handoff (`Goal`, `State`, `Checks`, `Next`), the result recorded where the
lane records it.

## Interplay with P15 and the order

P14's Rust files are `text/{options,plan,batch,entry,line}.rs`, `mime_type/line.rs`,
`implementer.rs` and `rust/fix/src/schema.rs`; P15 (D48) edits `text/{sep,reader,mod}.rs`,
`text/arrow.rs` (`:659` and `Held`'s visibility), `csv/`, `media/options.rs` and `excel/options.rs`.
Those sets are disjoint. **The two share**: `python/src/iomedia.rs`, `python/yggdryl/_native.pyi`,
`python/tests/test_iomedia.py`, `python/tests/typing_bindings.py`, `AGENTS.md`,
`skills/yggdryl-records/*`, `.api-inventory.txt` and `.api-bindings.txt`, so P15's anchors in them
are re-read on P14's commit as its `arrow.rs:659` already is. P14 first as decided (31 before 32),
each its own commit, P15 rebased on P14's tree.

## Put to the user (interpretations taken; say if another was meant)

1. **`parse_keyvalues` is off by default** and bronze turns it on in `Lake.text_options()`; "add then
   also in textline generated schema" could mean always-on. Reversing is one line of `new()` and
   moves the pins D47.2 lists (every text column list, the docs' `18`, the `build/plain` number, the
   FIX capture's carried root).
2. **"value default json serialized" is read as FIX reads it**: plain text as itself, a nested or
   JSON-opening or repeated value as its JSON. The alternative JSON-encodes every value (`"1"`), one
   arm of the fold's leaf case, and moves the FIX `fixentries`/`metadata` cells too, since the rule
   has one owner - the equivalence snapshot and `docs/fix/arrow.md`'s examples with it.
3. **The column is `map<utf8, utf8>` with keys sorted**, the FIX and market maps' shape; arrival order
   lives inside a repeated key's array. The alternative is an insertion-ordered map.
4. **A bracketed prose value is stored as the scanner reads it** (`"[00064703461GBYZ0"`, a JSON string
   because it opens with `[`); trimming the bracket pair is a scanner change the FIX codec reads by.
5. **A `#`-marked key keeps its `#`**: `#NOPARTYIDS` and `NOPARTYIDS` are two keys, not one doubled
   key, because the capture's values differ (`3` and `2`). The alternative folds them into one array.
6. **The one scanner gap closed is the double-quoted loose value** (logfmt's `msg="..."`), in the
   entries walk alone, escapes kept as written, gated on the FIX equivalence snapshot and the
   classification pins not moving. It moves three public readings whatever the option (D47.3): a
   quoted pair now read, a pair inside a quoted value no longer one. Nothing in the user's words
   asked for a scanner change, so this is its own decision: say if the walk should stay as it is.
7. `,`/`;`/`&` pair separators, the bracket trim and the empty loose value are not closed.
8. **No pair-separator or dialect option** in P14; a `pairs_separator` would be the one addition, for
   a query-string or Splunk `DELIMS` shape, and is not written.
9. **A nested value is its JSON object** (the bridge's party entries), not its raw text.
10. **A batch read back never lands the `keyvalues` cell** (the body re-derives it), as `body` itself
    is not restated; `TextLine::into_keyvalues` is built on each ask over the tree's one slot, no
    slot of its own.
11. **The column stays in `bronze.log_messages`**: `fix_row()` excludes it and the FIX stage selects
    it out, so no FIX table holds it. The alternative carries it onto every message a line splits
    into, each arrival then held twice (as `fixentries`/`metadata` and as `keyvalues`), at a per-row
    map build and a per-message layout.
12. **The fixture is a third log object in the medallion test**, `rust/tests/support/keyvalues.log`,
    `144 + N` re-pinned; `ulbridge.log` is untouched; an existing bronze table lacking the column is
    not evolved by P14.
13. **Node gains nothing**: no `parseKeyvalues`, no `keyvalues` reading; the page says "Rust and
    Python; JavaScript has no `parse_keyvalues`".
14. **The reading is the depth-8 tree's top level**, so a nested value is read as a tree (and written
    as JSON); `from_bytes_direct` (one level) is the alternative and would hold the bridge's party
    text raw.

## Taken (decision 36, 2026-10-10)

The user answered: `parse_keyvalues` off by default (bronze opts in); values plain unless a
repeated key, a nested tree or a JSON-opening text needs JSON; the double-quoted loose value read
as one pair (escapes raw); CSV header and separator inferred by default. Every other question here
is taken as recommended (`user_decisions.md` 36). Implement these, not the alternatives.
