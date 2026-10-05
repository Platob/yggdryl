# Expression and plan grammar - cheat sheet

The one grammar every language parses (the Rust core owns it). Spellings follow
DuckDB's SQL and Python expression API; where DuckDB and the best-effort rule
disagree, the rule wins. Every alias reads; one canonical text prints, and
`Display`/`str`/`toString` re-parses to the same tree.

## Which type reads which text

| Text | Read as | Notes |
| --- | --- | --- |
| `price > 100` | `Term`, `Filter` | a bare predicate; `where` optional for a `Filter` |
| `where price > 100` | `Filter`, `Expression` (a `where` clause) | |
| `ccy, size * 2 as doubled` | `Selector` | `select` optional for a `Selector` |
| `select ccy` | `Selector`, `Expression` (a `select` clause) | a lone clause is never a one-section plan |
| `select a from t where b > 1 limit 5` | `Plan`, `Expression` (a plan) | |
| `select * from t left join v using (id)` | `Plan`, `Expression` (a plan) | joins are plan sections, never terms |
| `where a > 1; select b` | `Expression` (a sequence) | steps run in order |
| `a > 1` read as `Expression` | refused | an `Expression` must open with `select`, `where`, `create`, `insert`, `upsert`, `delete` or `from` |

## Plan shape

```text
expression  := plan (";" plan)*
plan        := [create] [write] ["select" selector] ["from" source] (join)* ["where" expr]
               ["order" "by" orders] ["limit" n] ["offset" n]   -- limit/offset either order, printed limit first
join        := [kind] "join" source ("using" "(" ident, ... ")" | "on" expr "=" expr ("and" expr "=" expr)*)
kind        := "inner" | "left" ["outer"] | "right" ["outer"] | "full" ["outer"] | "outer" | "semi" | "anti"
                                                                 -- bare `join` is inner, `outer` is full
create      := "create" ["table" | "view"] [target] "(" selector ")" ["with" properties]
write       := verb [target] [("by" | "on") "(" selector ")"]  -- keys only after upsert
target      := location ["with" "(" name "=" "'value'", ... ")"]
location    := "'url'" | raw | part ("." part)*                -- part: ident, "quoted", `quoted`, [bracketed], number
                                                                 -- raw: an unquoted URL or path (<scheme>://, /, ./, ../, ~/, C:\) to the first whitespace , ; or ),
                                                                 --      only after from/into/to/a write verb/create [table|view]/join; printed back as 'url'
source      := target | "(" plan ")"
selector    := "*" [("exclude" | "except") "(" ident, ... ")"] ("," projection)*
             | projection ("," projection)*
projection  := expr ["as" ident] [datatype ["null" | "not null"]] ["with" properties]
orders      := expr ["asc" | "desc"] ["nulls" ("first" | "last")] ("," ...)*
```

## Sections, in the order they run

| Section | Example | Behaviour |
| --- | --- | --- |
| `create` | `create 't.parquet' (id int64 not null, name utf8)` | declares the schema the stream is cast to; `create` alone writes the schema and no rows |
| write verb | `insert into t` | where the shaped stream goes; no target = the handle the plan is given to |
| `select` | `select id, price * 2 as doubled` | `select *` when absent |
| `from` | `from 'file:///lake/t.parquet'`, `from lake.raw.t`, `from (select ...)` | what `execute` reads; a nested plan runs first |
| `join` | `left join v using (id)`, `join v on venue = mic and desk = desk`, `anti join (select id from h) using (id)` | left to right, each source read whole and held, the stream probed batch by batch; `using` keys coalesced, a colliding right name suffixed `_right`; the first join's `key in (<distinct build keys>)` pushed into the left read for one key, `inner`/`right`/`semi`, at most 10,000 keys ([Joins](https://platob.github.io/yggdryl/expression/plans/#joins)) |
| `where` | `where price > 0` | pushed into the read; after a join only its left-only conjuncts, and none under a `right` or `full` join - the rest runs over the joined rows, so it may name a right column |
| `order by` | `order by id desc nulls first, ccy` | the one section that collects the stream; stable sort |
| `limit` / `offset` | `limit 10 offset 5` | slice views; pushed into the read when nothing orders |

## Write verbs

| Canonical | Also read as | Does |
| --- | --- | --- |
| `insert into t` | `insert t`, `append to t`, `append into t`, `append t` | append after stored rows |
| `insert overwrite t` | `insert overwrite into t`, `overwrite [into] t`, `replace [into] t` | replace stored rows |
| `upsert into t by (id)` | `upsert t by (id)`, `merge into t on (id)`, `merge t by (id)` | replace matching rows, append the rest |
| `delete from t where ...` | `delete t where ...` | remove rows the predicate keeps |

Target properties: `media_type`, `codec`, `safe`, `batch_row_size`,
`batch_byte_size`, `commit_batch_num`, `num_threads`, `max_row_size`,
`row_offset`, `max_byte_size`, plus what a holder reads - `t with (media_type = 'text/csv', batch_row_size = '1024')`.

## Terms

| Shape | Spelling |
| --- | --- |
| column | `name`, `"odd name"`, `` `odd name` `` - resolved ASCII case-insensitively |
| literal | `1` (int64), `1.5` (float64), `'text'` (utf8), `true`, `null` |
| typed literal | `decimal128(9,2) '1.50'`, `date32 '2024-01-01'`, `int32 '5'`, `utf8 null` |
| parameter | `:since` - supplied at bind, never later; a missing one is refused, an extra one ignored |
| comparison | `=`, `<>` / `!=`, `<`, `<=`, `>`, `>=` |
| distinctness | `is distinct from`, `is not distinct from` (two-valued: never unknown) |
| null test | `is null`, `is not null` |
| membership | `x in (a, b)`, `x not in (a, b)` |
| range | `x between lo and hi`, `x not between lo and hi` (inclusive) |
| pattern | `like 'a%'`, `ilike 'A%'`, `like 'a!%' escape '!'`, `glob '**/*.parquet'` |
| logic | `and`, `or`, `not` (Kleene three-valued) |
| arithmetic | `+`, `-`, `*`, `/`, `%`, unary `-`; `/` over two integers is integer division truncating toward zero (`7 / 2` = 3, `-7 / 2` = -3) - cast one operand (`cast(a as float64) / b`, or a decimal) for a fractional quotient |
| conditional | `case when c then v [when ...] else w end` |
| conversion | `cast(x as int32)` (refuses), `try_cast(x as int32)` (null on failure) |
| constructor | `[1, 2]` serie, `{'k': 1}` map, `struct(1 as a)` struct |
| comment | `-- to end of line` |

Precedence, loosest first: `or`, `and`, `not`, predicate, `+ -`, `* / %`,
unary `-`, accessor.

## Paths (`FieldPath` and term accessors)

| Step | Spelling | Reaches |
| --- | --- | --- |
| child | `a.b`, or bare `a` at the start (`.a` same) | a struct child |
| position | `a[0]`, `a[-1]` | one serie element, 0-based, negative from the end; past the end is null |
| key | `a['k']` | one map entry; missing key is null; `['7']` is a key, `[7]` a position |
| slice | `a[1:3]`, `a[:-1]` | `[start:end)` run, either bound optional, clipped at the ends (`FieldSegment` `Range`) |
| predicate segment | `legs[ccy = 'EUR']`, `legs[active]`, `legs[ccy = 'EUR'][-1].size` | elements of a serie of structs the predicate answers exactly true for (`FieldSegment` `Where`, `FieldSegment::filter`) |
| quoted child | `"a.b"` | ONE child named `a.b`; `a.b` is two levels |
| alias | `order.line[0].price as price` | what to call what the path reached (`column_name`) |

A doubled quote inside a quoted name is that quote: `"say ""hi"""` is `say "hi"`.
`order.as` is a child named `as`; `assets` is one name.

## Selectors

| Projection | Publishes |
| --- | --- |
| `price` | the column, unchanged (a bare-column projection touches no buffer) |
| `price as amount` | renamed |
| `price * 2 as doubled` | computed, typed by the term |
| `price decimal(9,2)` | cast; a value that does not fit becomes null |
| `price decimal(9,2) not null` | cast; a null or a misfit is refused naming `price` |
| `id int64 with (comment = 'key')` | metadata on the published field |
| `*` | every column, handed back untouched |
| `* exclude (secret)` / `* except (secret)` | every column but those |
| `*, upper(name) as loud` | star first, appended projections after the kept columns |
| `* exclude (name), upper(name) as name` | replace a column: exclude it and re-project it (it moves to the end); `*, upper(name) as name` is refused as a duplicate name - there is no `* replace (...)` |
| `unnest(legs) as leg` / `explode(legs)` | one row per element; a struct item becomes `leg.<child>` columns; whole projection only, at most one per select |

Refused: `a, *`, a trailing `*,`, `* exclude ()`, `select` inside a projection
list, `unnest` inside a term, a `where`, an `order by`, a key or a `create`.

## Functions (closed set of 28)

Aliases in parentheses read and print as the canonical name.

| Group | Functions |
| --- | --- |
| text | `lower` (`lcase`), `upper` (`ucase`), `length` (`len`, `char_length`, `character_length`), `substring` (`substr`; 1-based), `trim` (`btrim`), `starts_with` (`startswith`), `ends_with` (`endswith`), `contains` (`strpos_contains`), `concat` |
| calendar parts | `year`, `month`, `day` (`dayofmonth`), `hour` - a field read off the date |
| epoch periods | `years`, `quarters`, `months`, `weeks` (Monday-start), `days` (a `date32`), `hours`, `minutes(x, n)` - `int32` whole periods since 1970, floored (`years('1969-12-31')` is -1); `minutes` always writes its step `n` (1 to 4294967295) |
| floors | `truncate` (`trunc`, `date_trunc`; fixed units only - no calendar months), `time_bucket(width, x)` - `x` floored to the width, its datatype, unit and zone kept |
| null handling | `coalesce`, `if_null` (`ifnull`, `nvl`, `isnull`) |
| nested | `size` (`cardinality`), `get`, `slice` (`array_slice`), `unnest` (`explode`) |
| user-defined | `namespace.name(args)` - registered with a signature; never pushed to statistics |

`time_bucket` width is a constant: text - `'15 minutes'`, `'30s'`, `'1.5h'`,
`'900s'`, `'1 week'`, ISO 8601 `'PT15M'`, a clock `'00:15:00'` - or a
`duration` literal; units `ns`, `us`, `ms`, `s`/`sec`, `min`, `h`/`hr`, `d`,
`w` and their long forms. `'15m'` is refused (a minute or a month), as are a
calendar, zero, negative, column-held or finer-than-`x` width and a sub-day
width over a date. Buckets start at DuckDB's origin, Monday 2000-01-03 (UTC
for a zoned value, the wall clock for a naive one), so a width dividing a day
lines up with the epoch. Epoch periods and `time_bucket` are monotone: a range
on `x` prunes a predicate on them, and they are the keys `PARTITION:by` and
`window_by` take. A sub-day period over a date is refused; a period past
`int32` is null.

`FUNCTIONS`, `COMPARISONS`, `VERBS` (Python `yggdryl.expression`) and
`expressionVocabularies()` (JavaScript) list the live vocabulary.

## Semantics that decide answers

| Rule | Effect |
| --- | --- |
| null is unknown | a null operand makes a comparison unknown; `where` keeps a row only when the answer is exactly `true` |
| `and` / `or` | `false and unknown` = false, `true or unknown` = true, `not unknown` = unknown |
| constants coerce | `i = '1'` on `int64` binds as `i = 1`; `price > 100` on `decimal(9,2)` binds as `price > decimal32(9,2) '100'` |
| no common type | compares as text: `s > 1` on `utf8` binds as `s > '1'` |
| decimals | never implicitly a float; division keeps at least six fractional places; an untyped fractional literal (`price > 9.5`) is `float64`, shares no type with a decimal and compares as text - write `decimal(9,2) '9.50'` or an integer |
| floats | IEEE totalOrder: `nan = nan`, `nan` sorts above everything |
| text order | code point, no collation |
| names | ASCII case-insensitive; one name in two cases is an ambiguity error |
| join keys | `on` takes `=` conjuncts only, its left term bound against the rows so far and its right against the source; a path names a column, never a table alias (`t.id` is column `t`'s child `id`) |
| limits | nesting at most 32 levels (`expression::RECURSION_LIMIT`, its own - the schema grammar's is 64), a `from (plan)` counting against it; at most 100,000 nodes (`NODE_LIMIT`) |

## Refused and reserved

| Refused | Because |
| --- | --- |
| subqueries in `where`, aggregates, window functions, `group by`, `having`, `union` | a second relation enters only as `from (plan)` or a [`join`](https://platob.github.io/yggdryl/expression/plans/#joins) clause |
| a join inside a term | a join is a plan section |
| regex (`~`, `rlike`, `similar to`) | no regex engine |
| `element_at` | ambiguous base; use `get` |
| per-row `like` pattern | refused at bind |
| `\|\|` as or, `&&` as and | one operator, one meaning |
| a session timezone | meaning would depend on the evaluator |
| `//`, `->`, `->>`, `date_diff`, `concat_ws`, `strip_prefix`, `strip_suffix`, hex literals | reserved: parse errors today |

Depth: https://platob.github.io/yggdryl/expression/grammar/,
https://platob.github.io/yggdryl/expression/plans/#joins,
https://platob.github.io/yggdryl/expression/functions/#calendar-parts-and-epoch-periods and
https://platob.github.io/yggdryl/types/paths/
