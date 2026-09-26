# yggdryl-arrow: the cast contract

Every cast - `Serie.cast`, `ChunkedSerie.cast`, the `Serie` / `ChunkedSerie`
Arrow doors, `SerieReader`, a held `ArrowCastPlan` - runs one engine and
answers by these rules. Runnable examples are in the three language
references; full detail at https://platob.github.io/yggdryl/types/cast/.

## The two options, and what is not an option

| Knob | Rust | Python | JavaScript | Decides |
| --- | --- | --- | --- | --- |
| `safe` (default `true`) | `ArrowCastOptions::new().with_safe(false)` | `safe=False` | `{ safe: false }` | whether a **present** value that fails to convert becomes null (`true`) or is refused (`false`) - and it can become null only in a nullable column |
| `representation` (default `value`) | `.with_representation(Representation::Bits)` | `representation="bits"` | `{ representation: 'bits' }` | what a **same-width** pair carries: `value` = the number, range-checked; `bits` = the bytes, buffer shared |
| absence | - | - | - | never an option: the **target field's nullability**, one rule at every door |

## Outcome table

| Situation | Nullable target | Required target |
| --- | --- | --- |
| Source null | null | refused at that batch: `required Arrow field $.x holds N null values` |
| Value fails to convert, `safe=true` | null | refused by that value (`not a number`) |
| Value fails to convert, `safe=false` | refused by that value | refused by that value |
| Empty text `""` into a non-text column | null (before `safe` is asked) | refused as a null, by path |
| Whitespace-only `" "` into a non-text column | not empty: a failed conversion, so null under `safe` and refused under `safe=false` | refused by that value |
| Column missing from the source | all-null column | refused when the plan is **compiled**: `required Arrow field $.x is missing from the source` |
| Extra source column | dropped | dropped |
| Null inside a null parent row | stays hidden | stays hidden; only exposed rows are judged |
| `null` datatype, or an encoding whose values are all null | null | kept: null is that datatype's canonical default, not an absence |

A required column **never** writes its canonical default in place of a value
it could not read. The only repair is internal: a column a declaring protocol
fills after the cast (a digest holder, a `TRANSFORM:` or `PARTITION:` column)
may arrive absent, and the finished batch is checked again.

## Targets

| Target given | Means |
| --- | --- |
| A `Field` | that field: name, nullability, metadata, extension identity |
| A field expression (`"price: int64 not null"`) | the field it spells |
| A `DataType` (Python, JavaScript) / `dtype.required_field("value")` (Rust) | the **required** field named `value`; a refusal names `$.value` |
| The column's own field | the column itself, no plan run |
| Any target on a schema-free run | refused: a run lays out no buffers; type its rows with `from_scalars` |

## Record batches and struct roots

| Rule | Behavior |
| --- | --- |
| Root | a bounded, non-null struct `Field`; anything else is refused by name |
| No root given | the batch or stream schema read as the record `row` |
| Child matching | by name, ASCII case-insensitive; an ambiguous fold is refused |
| Child order | the target's order |
| Missing child | nullable: all-null; required: refused at compile time |
| Extra child | dropped |
| Error path | dot/bracket from the cast root: `$.users[].zip`; a landed row as `$[3].bid.live[0].miccode` |
| Out as a batch | `into_arrow_batch` / `into_arrow_reader` refuse a record column holding a null **row** (a batch states no row validity) |

## Representation `bits`

| Pair | Result |
| --- | --- |
| `uint64` <-> `int64` <-> `float64` <-> `fixed_size_binary(8)` (same byte width) | the bytes reinterpreted, value buffer shared; `u64::MAX` reads `-1` and back |
| Different widths (`uint32` -> `int64`) | ordinary conversion, range-checked |
| Rule-governed target (fixed string, code, UUID, version) | the target's rule still runs: four bytes are not a currency |
| Required target over source nulls | refused by path, exactly as under `value` |

## Conversions worth knowing

| Source -> target | Behavior |
| --- | --- |
| text -> number, boolean, decimal, temporal | parsed; a failure follows the outcome table |
| text -> decimal | read at the declared scale; a digit the scale cannot hold is refused, never rounded |
| float -> decimal | the shortest text of the float, rounded half away from zero (`0.125` -> `0.13` at scale 2); `nan`/infinity fail |
| any value with a spelling -> text | that spelling; temporals in the classic form |
| decimal / bigdecimal -> text | trimmed (`1.125`); a parameterized width keeps its declared scale on columns |
| bytes -> code or UUID | read as bytes under every binary framing; non-US-ASCII is a failed value |
| text -> sized / fixed / non-UTF-8 string | every cell validated (length, width, repertoire); a failure follows the outcome table |
| two fixed sizes (serie or binary) | a value change, refused by name |
| dictionary / run-end target | the values' own rule, then the encoding |
| encoded source -> plain target | decoded first |
| any serie layout -> any other; any byte framing -> any other | converted |
| a column of another layout into a compiled plan | refused naming both layouts: a plan is for one source |

## What a landing proves

A column holds only rows its field accepts, so every cast ends where its rows
land in a `Serie`, and the landing proves three things:

| Proof | How |
| --- | --- |
| Layout | the buffers are exactly the field's Arrow projection |
| Absence | validity words counted against nullability, at every level; a record's children judged only where the record row is present |
| Values | **not read** where the layout is the datatype's whole contract - null, boolean, every integer and float, `date32`, datetimes, durations, intervals, plain `utf8` and binary leaves, `uuid`, and nestings of these; **read once** otherwise - codes, ASCII / windows-1252 / sized strings, decimals, `date64`, times, URLs, versions, variants - and the first refused row is named |

An `ARROW:extension:name` label (for example `yggdryl.url`) is **not** a
proof: foreign rows under it are read once, under every option. A plan node
that already read every value under the target's rule is not read again, and
a column the crate laid out itself (`from_scalars`, `from_default`) is never
re-read.

## When failures surface

| Failure | Raised by |
| --- | --- |
| Impossible conversion, ambiguous name, missing required column | `ArrowCastPlan` compile / `preflight`, `SerieReader` construction - before any row |
| A null or bad value in a batch | that batch: at the call for held data, at the **pull** for a stream |
| After a stream failure | the `SerieReader` is fused (yields nothing more) and its source is released |
| Held column / eager drain (`Serie.from_arrow_reader`) | the whole call fails on the first bad batch |

## Cost

| Operation | Cost |
| --- | --- |
| Identity plan (exact layout, nullability fits) | the same buffers handed back; `is_identity` is `true` |
| Equal layout, nullable source under a required target | not the identity: read for nulls |
| `Serie.cast`, a `Serie` Arrow door | one plan compiled per call |
| `SerieReader`, a held `ArrowCastPlan` | one plan for the whole stream; per batch only masks, offsets and uncertified leaves vary |
| `ChunkedSerie.cast`, `plan.apply_chunked` / `apply(chunked)` | one plan over every chunk |
| `ChunkedSerie.from_series` | one plan per run of chunks under one source field |
| `ChunkedSerie.push_chunk` | one plan per call |
| Compile once vs per batch (64-row batches, 1,000 batches) | 3.68 ms vs 6.07 ms, release build |
