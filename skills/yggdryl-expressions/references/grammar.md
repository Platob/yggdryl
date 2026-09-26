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
| `where a > 1; select b` | `Expression` (a sequence) | steps run in order |
| `a > 1` read as `Expression` | refused | an `Expression` must open with `select`, `where`, `create`, `insert`, `upsert`, `delete` or `from` |

## Plan shape

```text
expression  := plan (";" plan)*
plan        := [create] [write] ["select" selector] ["from" source] ["where" expr]
               ["order" "by" orders] ["limit" n] ["offset" n]   -- limit/offset either order, printed limit first
create      := "create" ["table" | "view"] [target] "(" selector ")" ["with" properties]
write       := verb [target] [("by" | "on") "(" selector ")"]  -- keys only after upsert
target      := location ["with" "(" name "=" "'value'", ... ")"]
location    := "'url'" | part ("." part)*                      -- part: ident, "quoted", `quoted`, [bracketed], number
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
| `where` | `where price > 0` | pushed into the read |
| `order by` | `order by id desc nulls first, ccy` | the one section that collects; stable sort |
| `limit` / `offset` | `limit 10 offset 5` | slice views; pushed into the read when nothing orders |

## Write verbs

| Canonical | Also read as | Does |
| --- | --- | --- |
| `insert into t` | `insert t`, `append to t`, `append into t`, `append t` | append after stored rows |
| `insert overwrite t` | `insert overwrite into t`, `overwrite [into] t`, `replace [into] t` | replace stored rows |
| `upsert into t by (id)` | `upsert t by (id)`, `merge into t on (id)`, `merge t by (id)` | replace matching rows, append the rest |
| `delete from t where ...` | `delete t where ...` | remove rows the predicate keeps |

Target properties: `media_type`, `codec`, `safe`, `batch_row_size`,
`batch_byte_size`, `commit_row_size`, `max_row_size`, `max_byte_size`, plus
what a holder reads - `t with (media_type = 'text/csv', batch_row_size = '1024')`.

## Terms

| Shape | Spelling |
| --- | --- |
| column | `name`, `"odd name"`, `` `odd name` `` - resolved ASCII case-insensitively |
| literal | `1` (int64), `1.5` (float64), `'text'` (utf8), `true`, `null` |
| typed literal | `decimal128(9,2) '1.50'`, `date32 '2024-01-01'`, `int32 '5'`, `utf8 null` |
| parameter | `:since` - supplied at bind, never later |
| comparison | `=`, `<>` / `!=`, `<`, `<=`, `>`, `>=` |
| distinctness | `is distinct from`, `is not distinct from` (two-valued: never unknown) |
| null test | `is null`, `is not null` |
| membership | `x in (a, b)`, `x not in (a, b)` |
| range | `x between lo and hi`, `x not between lo and hi` (inclusive) |
| pattern | `like 'a%'`, `ilike 'A%'`, `like 'a!%' escape '!'`, `glob '**/*.parquet'` |
| logic | `and`, `or`, `not` (Kleene three-valued) |
| arithmetic | `+`, `-`, `*`, `/`, `%`, unary `-` |
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
| slice | `a[1:3]`, `a[:-1]` | `[start:end)` run, either bound optional (terms only) |
| predicate segment | `legs[ccy = 'EUR']`, `legs[active]`, `legs[ccy = 'EUR'][-1].size` | elements of a serie of structs the predicate answers exactly true for (terms only) |
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
| `unnest(legs) as leg` / `explode(legs)` | one row per element; a struct item becomes `leg.<child>` columns; whole projection only, at most one per select |

Refused: `a, *`, a trailing `*,`, `* exclude ()`, `select` inside a projection
list, `unnest` inside a term, a `where`, an `order by`, a key or a `create`.

## Functions (closed set of 20)

| Group | Functions |
| --- | --- |
| text | `lower`, `upper`, `length`, `substring` (1-based), `trim`, `starts_with`, `ends_with`, `contains`, `concat` |
| temporal | `year`, `month`, `day`, `hour`, `truncate` (fixed units only - no calendar months) |
| null handling | `coalesce`, `if_null` |
| nested | `size`, `get`, `slice`, `unnest` (alias `explode`) |
| user-defined | `namespace.name(args)` - registered with a signature; never pushed to statistics |

`FUNCTIONS`, `COMPARISONS`, `VERBS` (Python `yggdryl.expression`) and
`expressionVocabularies()` (JavaScript) list the live vocabulary.

## Semantics that decide answers

| Rule | Effect |
| --- | --- |
| null is unknown | a null operand makes a comparison unknown; `where` keeps a row only when the answer is exactly `true` |
| `and` / `or` | `false and unknown` = false, `true or unknown` = true, `not unknown` = unknown |
| constants coerce | `i = '1'` on `int64` binds as `i = 1`; `price > 100` on `decimal(9,2)` binds as `price > decimal32(9,2) '100.00'` |
| no common type | compares as text: `s > 1` on `utf8` binds as `s > '1'` |
| decimals | never implicitly a float; division keeps at least six fractional places |
| floats | IEEE totalOrder: `nan = nan`, `nan` sorts above everything |
| text order | code point, no collation |
| names | ASCII case-insensitive; one name in two cases is an ambiguity error |
| limits | nesting shares the schema grammar's limit (32); at most 100,000 nodes |

## Refused and reserved

| Refused | Because |
| --- | --- |
| subqueries in `where`, joins, aggregates, windows, `group by`, `having`, `union` | one relation; `from (plan)` is the only nesting |
| regex (`~`, `rlike`, `similar to`) | no regex engine |
| `element_at` | ambiguous base; use `get` |
| per-row `like` pattern | refused at bind |
| `\|\|` as or, `&&` as and | one operator, one meaning |
| a session timezone | meaning would depend on the evaluator |
| `//`, `->`, `->>`, `date_diff`, `concat_ws`, hex literals | reserved: parse errors today |

Depth: https://platob.github.io/yggdryl/expression/grammar/ and
https://platob.github.io/yggdryl/types/paths/
