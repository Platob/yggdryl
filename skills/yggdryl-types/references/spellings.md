# Type expression spellings

One grammar reads every datatype expression - `DataType::from_str` in Rust,
`DataType("...")` / `DataType.from_str` in Python, `DataType.from('...')` /
`new DataType('...')` in JavaScript - and `Field::from_str` / `Field.from_str`
/ `Field.from` wrap it with a name and nullability. Every accepted spelling
resolves to one datatype and **displays under one canonical name** (the left
column below), so compare `DataType` values, never the text you wrote.

## Grammar rules

| Rule | Detail |
| --- | --- |
| Folding | keywords ignore case, `_` and `-`: `large_utf8`, `largeutf8`, `LARGE-UTF8` are one type. A space ends a keyword, so `large utf8` and `unsigned bigint` are refused (write `ubigint`/`unsignedbigint`); only the fixed SQL phrases `double precision`, `character varying`, `timestamp with[out] time zone`, `interval day`/`interval year` contain one |
| Round trip | `str(t)` / `t.toString()` / `to_string()` re-parses to the same value; `repr` in Python is `DataType.from_str("...")` |
| Nesting limit | 64 levels (`DataType::PARSE_RECURSION_LIMIT`), in parsing, defaults and compatibility walks alike |
| Errors | refusal names the byte position and what was expected: `invalid datatype expression at byte 5: ...` |
| Child nullability | `struct<a:int32 not null>`, `array<int64 not null>`; a child stated without it is nullable |
| Quoted child names | `struct<"a.b":int32>` or ``struct<`a.b`:int32>`` - one child named `a.b` |
| Field text | `price decimal(18, 6) NOT NULL`, `id: int64`, `id int64 not null`, or the canonical `field("id",int64,nullable=false,metadata={})`; a bare field is nullable |
| Logical names | the FIX vocabulary resolves to an ordinary datatype and displays as it (`Price` -> `float64`); never a variant of its own. `DataType.logical_names()` / `DataType.logicalNames()` lists them |

## Null, boolean, integers, floats

| Canonical | Also parsed as |
| --- | --- |
| `null` | `void` |
| `boolean` | `bool` |
| `int8` | `tinyint`, `byte` |
| `int16` | `smallint`, `short` |
| `int32` | `int`, `integer` |
| `int64` | `bigint`, `long` |
| `uint8` | `utinyint`, `unsignedtinyint` |
| `uint16` | `usmallint`, `unsignedsmallint` |
| `uint32` | `uint`, `unsignedint`, `unsignedinteger` |
| `uint64` | `ubigint`, `unsignedbigint` |
| `float16` | `half` |
| `float32` | `float`, `real` |
| `float64` | `double`, `double precision` |

No `int128`/`uint128` datatype exists: a 128-bit integer value answers the
narrowest `decimal(p, 0)` holding its digits.

## Decimals

| Canonical | Also parsed as | Note |
| --- | --- | --- |
| `decimal32(p,s)` | `decimal(1..=9, s)`, `numeric(p,s)` | the selector picks the narrowest backing width |
| `decimal64(p,s)` | `decimal(10..=18, s)` | |
| `decimal128(p,s)` | `decimal(19..=38, s)`; bare `numeric` is `decimal128(38,0)` | |
| `decimal256(p,s)` | `decimal(39..=76, s)`, `bignumeric(p,s)` | precision 77 and above refused |
| `decimal` | - | the **fixed leaf**: 38 digits at scale 18, Arrow `Decimal128(38,18)` under `yggdryl.decimal`. Not the same datatype as `decimal(38,18)` |
| `bigdecimal` | - | the fixed leaf over `Decimal256(76,18)` under `yggdryl.bigdecimal` |

Scale may be negative (`decimal256(39,-4)`, a multiplier); a positive scale
may not exceed the precision (`decimal(2,3)` is refused).

## Temporal

| Canonical | Also parsed as |
| --- | --- |
| `date32` | `date` |
| `date64` | `date_millisecond` |
| `time32(s)`, `time32(ms)` | `time(s)`, `time(ms)`, `time(0)`, `time(3)` |
| `time64(us)`, `time64(ns)` | `time`, `time(us)`, `time(6)`, `time(9)`; `time(p)` picks the unit from the precision |
| `datetime64(us)` | `datetime64`, `timestamp`, `timestamp_ntz`, `timestamp(6)`, `timestamp without time zone`, `datetime64(us, None)` - a naive wall clock |
| `datetime64(us,"UTC")` | `datetime64(us, UTC)`, `timestamp_ltz`, `timestamp with time zone`, `datetime64(us, Some(UTC))` |
| `datetime64(ns,"Europe/Paris")` | `datetime64(9, Europe/Paris)`; any zone spelling `Timezone` canonicalizes (`Asia/Calcutta` -> `Asia/Kolkata`) |
| `duration32(unit)`, `duration64(unit)` | Arrow's `Duration(ns)` reads as `duration64(ns)`; bare `duration` and `duration(ms)` are **refused** (no width) |
| `interval(month_day_nano)` | `interval` |
| `interval(day_time)` | SQL `interval day` |
| `interval(year_month)` | SQL `interval year`, `interval(years)` |
| `timezone` | `tz`, `timezone_name` - a column of zones, not a temporal |

Not spellings: `datetime`, `timestamp_tz`. Units: `s`/`second(s)`,
`ms`/`milli(s)`/`millisecond(s)`, `us`/`µs`/`micro(s)`/`microsecond(s)`,
`ns`/`nano(s)`/`nanosecond(s)`, and the interval layouts `year_month`,
`day_time`, `month_day_nano`. `d`/`day(s)` is a value unit only
(`Scalar.duration(n, 'd')`, a date's `unit`): no datatype takes it, so
`duration64(d)` is refused. `time32` takes `s`/`ms`, `time64` takes `us`/`ns`,
`datetime64` and `duration32`/`duration64` take `s`..`ns`.

## Strings: eighteen leaves, one number rule

Six shapes in each of three charsets. The leaf **is** the declaration.

| Shape | UTF-8 | US-ASCII | windows-1252 | Number |
| --- | --- | --- | --- | --- |
| 32-bit offsets | `utf8` | `ascii` | `cp1252` | none |
| 64-bit offsets | `large_utf8` | `large_ascii` | `large_cp1252` | none |
| view | `utf8_view` | `ascii_view` | `cp1252_view` | none |
| view, 64-bit | `large_utf8_view` | `large_ascii_view` | `large_cp1252_view` | none |
| fixed | `fixed_utf8(n)` | `fixed_ascii(n)` | `fixed_cp1252(n)` | exact width in bytes, NUL-padded, required |
| bounded | `sized_utf8(n)` | `sized_ascii(n)` | `sized_cp1252(n)` | maximum in bytes, required |

| Spelling | Is |
| --- | --- |
| `string`, `str`, `text`, `varchar`, `nvarchar`, `char`, `character varying` | `utf8` |
| `varchar(n)`, `string(n)`, `utf8(n)`, `sized_string(n)` | `sized_utf8(n)` |
| `char(n)`, `character(n)`, `nchar(n)`, `fixed_string(n)` | `fixed_utf8(n)` |
| `large_string`, `string_view`, `large_string_view` | `large_utf8`, `utf8_view`, `large_utf8_view` |
| `us_ascii`, `string(us-ascii)`; `ascii(n)` | `ascii`; `sized_ascii(n)` |
| `windows_1252`, `string(windows-1252)`, `string(cp1252)`; `cp1252(n)` | `cp1252`; `sized_cp1252(n)` |
| `string(windows-1252,32)`, `fixed_string(us-ascii,4)` | `sized_cp1252(32)`, `fixed_ascii(4)` |

**One number, one leaf.** A number on a plain leaf is a *maximum* and
answers the sized leaf (`utf8(32)` = `sized_utf8(32)`, `ascii(4)` =
`sized_ascii(4)`); on `fixed_*` it is the exact *width*; `fixed_*` and
`sized_*` never stand without one; a large or view leaf refuses one
(`large_utf8(64)`, `utf8_view(8)` are errors, not silently narrowed); a bound
of `0` is refused. Only the charset-free spellings (`string`,
`fixed_string`, ...) take a charset argument: `utf8(windows-1252)` is refused
because the name already said it, and `string(iso-8859-1)` is refused because
only UTF-8, US-ASCII and windows-1252 have leaves. Bounds count **stored
bytes**: `sized_utf8(4)` refuses `Grüß` (6 bytes) where `sized_cp1252(4)`
holds it.

## Bytes: six leaves

| Canonical | Also parsed as | Number |
| --- | --- | --- |
| `binary` | `bytes`, `blob`, `bytea`, `varbinary` | none |
| `large_binary` | - | none |
| `binary_view` | - | none |
| `large_binary_view` | - | none |
| `fixed_binary(n)` | `fixed_size_binary(n)` | exact width, never padded |
| `sized_binary(n)` | `binary(n)`, `varbinary(n)`, `varbinary_bounded(n)` | maximum |

The same number rule holds: `large_binary(16)` is refused.

## Codes, identifiers and canonical text

| Canonical | Also parsed as | Note |
| --- | --- | --- |
| `ccy`, `country`, `mic`, `cfi`, `isin`, `cusip`, `sedol`, `bbg`, `figi`, `ric`, `side`, `state`, `timeinforce`, `unit` | FIX `Ccy`, `Country`, `Exchange` (= `mic`) | fourteen registered codes, kind `code`; widths 3, 2, 4, 6, 12, 9, 7, 32, 12, 32, 8, 10, 8, 32 |
| `uuid` | - | 16 bytes under `arrow.uuid` |
| `version` | - | `major.minor.patch`, numerically ordered |
| `mimetype` | `mime` | one `type/subtype` |
| `mediatype` | `content_type` | MIME type + charset + content codings |
| `url`, `urn` | - | locations and names (see `yggdryl-uri`) |
| `geometry`, `geography` | `geometry("EPSG:3857")`, `geography("OGC:CRS84","vincenty")` | WKB payload; the default CRS `OGC:CRS84` and edges `spherical` display as nothing |
| `variant` | - | the Parquet Variant datatype; **not** a union |

Not spellings: `json`, `jsonb`.

## Nested

| Canonical | Also parsed as |
| --- | --- |
| `serie(field("item",T,...))` | `serie<T>`, `serie(T)`, `array<T>`, `list<T>`, `ARRAY<T>` |
| `large_serie(...)` | `large_serie<T>`, `largearray<T>`, `large_list<T>` |
| `serie_view(...)` | `arrayview<T>`, `list_view<T>` |
| `large_serie_view(...)` | `largearrayview<T>`, `large_list_view<T>` |
| `fixed_size_serie(...,n)` | `fixed_size_serie(T,n)`, `fixedarray<T,n>`, `fixed_size_list<T,n>`, `fixed_size_list(T,n)` |
| `struct(field(...),...)` | `struct<a:T,b:U>`, `STRUCT<a: INT, b: STRING>`, `row(a T, b U)` |
| `map(field("entries",...),keys_sorted=false)` | `map<K,V>`, `MAP<K, V>`; sorted keys: `map<K,V,keys_sorted=true>` (there is no `sorted_map<...>` spelling) |
| `union(dense,0=field(...),...)` | `variant(a:T,b:U)` (dense, ids from 0), `dense_union(a:T)`, `sparse_union(a:T)`, `union(sparse,0=a:T)` |
| `dictionary(K,V)` | `dict<K,V>`, `dictionary<K,V>`; `K` one of the eight integers |
| `run_end_encoded(field("run_ends",...),field("values",...))` | `run_end_encoded(int32, utf8)`; run ends `int16`/`int32`/`int64` only |

The `list` words (`list`, `list_view`, `large_list`, `large_list_view`,
`fixed_size_list`) are `DataTypeId::LEGACY_NAMES`: every door still reads
them and nothing writes them - the display, `id`, `kind()` and every
serialized tag say `serie`. A serie item written without a field is named
`item` and is nullable.

## FIX logical names

| Name | Resolves to |
| --- | --- |
| `Ccy`, `Country`, `Exchange`/`mic`, `cfi`, `isin`, `cusip`, `sedol`, `bbg`, `ric`, `figi` | the code of that name |
| `Language` | `fixed_ascii(2)` |
| `MonthYear`, `Tenor` | `fixed_ascii(8)` |
| `Length`, `TagNum`, `NumInGroup`, `Reserved100Plus`, `Reserved1000Plus`, `Reserved4000Plus` | `int32` |
| `SeqNum` | `int64` |
| `DayOfMonth` | `int8` |
| `Qty`, `Price`, `PriceOffset`, `Percentage`, `Amt` | `float64` |
| `UTCTimestamp`, `TZTimestamp`, `TZTimeOnly`, `UTCDateOnly`/`utcdate` | `datetime64(ns,"UTC")` |
| `LocalMktDate`, `LocalMktDatetime` | `datetime64(ns)` |
| `UTCTimeOnly`, `LocalMktTime` | `time64(ns)` |
| `Pattern`, `MultipleCharValue`, `MultipleStringValue`, `XID`, `XIDREF` | `utf8` |
| `data`, `XMLData` | `binary` |

`int`, `float`, `char`, `String`, `Boolean` keep their grammar meaning
(`int32`, `float32`, `utf8`, `utf8`, `boolean`), not FIX's. Names fold like
every keyword: `utc_date_only` is `UTCDateOnly`.

## Engine compatibility rewrites

`into_scheme_compat(target)` / `intoSchemeCompat(target)` applies only the
layout rewrites a target needs, and refuses one that would reinterpret values.

| Target | Rewrite |
| --- | --- |
| `arrow` | validated clone |
| `spark` | `uint8` -> `int16`, `uint64` -> `decimal128(20,0)`, `fixed_size_serie` -> `serie`; `datetime64(ns)` refused |
| `polars`, `pandas` | no map (refused naming the key/value struct); Polars keeps unsigned and `fixed_size_serie` |
| `iceberg` | `int8`, `int16`, `uint8`, `uint16` -> `int32`; no duration or interval |

Anything else (`duckdb`, ...) is refused, listing the accepted targets.
