---
name: yggdryl-expressions
description: Parse, bind and evaluate yggdryl expressions - Term, Filter, Selector, Plan, Expression, FieldPath - over rows, Arrow batches and streams, and push where/select into record reads. Use when writing a where or select clause or an SQL-like plan (insert into, upsert ... by, delete from, execute), filtering or projecting Arrow data (apply_arrow_reader / applyArrowReader, apply_arrow_batch / applyArrowBatch, apply_records / applyRecords), binding parameters, pruning by statistics (Bounds, statistics_prune), partition_split, user-defined functions, TRANSFORM: columns or nested field paths. Covers Rust, Python and Node.js.
---

# Yggdryl expressions

An expression is text - or a tree built method by method - over one grammar,
and it means nothing until it is **bound** against a non-null Struct `Field`.
Parse once; bind once per stream, which resolves names to indices, converts
each literal into the type of the column it meets, folds constants and orders
`and` cheapest-first; then evaluate as many rows, batches or statistics as you
like from the compiled tree - or hand the unbound clause to a record read and
let the media prune. There is no second engine: one `Bound` answers a row, a
batch, a stream and a file's statistics, and the Arrow tier is asserted equal
to the row tier.

| Layer | Is | Text |
| --- | --- | --- |
| `Term` -> `Bound` | one tree -> that tree compiled against one schema | `price > 100` |
| `Filter` | one predicate, a `where` clause | `where price > 100` (`where` optional) |
| `Selector` -> `BoundSelector` | a projection list, a `select` clause | `id, price * 2 as doubled int32` |
| `Plan` | the sections of one read or write | `upsert into t by (id) select id from s where x > 0` |
| `Expression` | whichever of those the text is, or a `;` sequence | `where a > 1; select b` |
| `Records` | native rows streaming out of any of them | - |
| `FieldPath` | one resolved path into a nested schema or value | `order.line[0].price as price` |

Pipeline: parse -> type -> simplify -> **bind(schema)** -> apply (row, batch,
reader, statistics, or media pushdown).

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| parse a predicate | `"a > 1".parse::<Filter>()?` | `Filter("a > 1")` | `new Filter('a > 1')` |
| parse a projection list | `"a, b * 2 as c".parse::<Selector>()?` | `Selector("a, b * 2 as c")` | `new Selector('a, b * 2 as c')` |
| parse a plan / a sequence | `.parse::<Plan>()?` / `.parse::<Expression>()?` | `Plan(text)` / `Expression(text)` | `new Plan(text)` / `new Expression(text)` |
| build a term without text | `col("p").gt(lit(100_i64))` | `Term.column("p").gt(100)`, `&`, `\|`, `~`, `+ - * / %` | `Term.column('p').gt('100')`, `.and()`, `.add(1)` |
| build a plan by section | `Plan::new().select(s)?.filter(f)?.limit(Some(10))` | `Plan().with_select(s).with_filter(f).with_limit(10)` | `new Plan().withSelect(s).withFilter(f).withLimit(10)` |
| bind once (with parameters) | `f.bind(&root)?`, `f.bind_with(&root, &[("lo", v)])?` | `f.bind(root, {"lo": 1})` | `f.bind(root, { lo: 1 })` |
| answer one row | `bound.matches(&row)?`, `bound.eval(&row)?` | `bound.matches(row)` (sequence or mapping) | `bound.matches(Scalar.from([...]))` |
| shape a stream (primary) | `x.apply_arrow_reader(reader)?` | `x.apply_arrow_reader(reader)` | `x.applyArrowReader(BatchReader.from(t))` |
| shape one batch | `x.apply_arrow_batch(&batch)?` | `x.apply_arrow_batch(batch)`, `apply_arrow(any)` | `x.applyArrowBatch(batch)`, `applyArrow(any)` |
| filter with a held `Bound` | `bound.filter(&b)?`, `filter_mask`, `filter_reader(r)` | `bound.filter_arrow_batch(b)`, `filter_arrow_reader(r)` | `bound.filterArrowBatch(b)`, `filterArrowReader(r)` |
| native rows in and out | `x.apply_records(Some(&root), rows)?` | `x.apply_records(rows, root)` | `x.applyRecords(rows, root)` |
| output schema, no data | `x.apply_field(&root)?`, `apply_datatype`; a plan `plan.field_from(&root)?` | `x.apply_field(root)`; a plan `plan.field_from(root)` | `x.applyField(root)`; a plan `plan.fieldFrom(root)` |
| skip a file by statistics | `bound.statistics_prune(&Bounds::new(..).with_column(..))` | `bound.statistics_prune(Bounds(rows=..).with_column(..))` | not bound |
| split partition / row halves | `bound.partition_split()` -> `Residual` | `bound.partition_split()` -> `(answerable, remaining)` | `bound.partitionSplit()` -> `{ answerable, remaining }` |
| push into a record read | `options.with_filter(f)?.with_select(s)?` + `read_arrow_reader(&options)` | `read_arrow_reader(filter=f, select=s)` | `readArrowReader({ filter: f, select: s })` |
| run a plan on storage | `plan.execute()?` | `plan.execute()` | `plan.execute()` |
| resolve a path | `FieldPath::from_str(p)?`, `apply_scalar(&root, &v)` | `FieldPath(p)` | `new FieldPath(p)` |
| register a function | `register_function(Arc::new(f))?` | `@user_defined_function(namespace=...)` | not bound (parses, bind refuses) |
| store a derivation on a schema | `sel.into_field(&root)?`, `Selector::from_field(&f)` | `sel.into_field(root)`, `Selector.from_field(f)` | `sel.intoField(root)`, `Selector.fromField(f)` |
| recompute stored derivations | `stored.apply_arrow_batch(&b, true, true, true, ArrowCastOptions::new())?`, `apply_arrow_reader(r, ..)` | `stored.apply_arrow_batch(b)`, `apply_arrow_reader(r)` | not bound |
| see what runs | `bound.explain()` | `bound.explain()` | `bound.explain()` |
| canonical text / document | `to_string()`, `into_json()?`, `from_json(s)?` | `str(x)`, `into_json()`, `from_json(s)` | `toString()`, `intoJson()`, `fromJson(s)` |

`x` is any clause: `Filter`, `Selector`, `Plan` or `Expression`, with two
exceptions: a `Plan`'s output schema is `plan.field_from(root)` /
`plan.fieldFrom(root)` (Rust also `apply_datatype`), never `apply_field`; and
a Rust `Plan` answers only `apply_arrow_reader` and `execute` - convert with
`Expression::from(plan)` for `apply_arrow_batch`, `apply_records` or
`apply_field`.

## Rules for fast, correct use

1. **Parse once, bind once, evaluate many.** Binding is the expensive,
   schema-dependent step (0.5-4.5 us); hoist it out of every row and batch
   loop and keep the `Bound` / `BoundSelector`.
2. **Take the reader door.** `apply_arrow_reader` binds once and wraps the
   stream lazily, with the output schema known before the first batch. The
   unbound clause's `apply_arrow_batch` and `apply_scalar` bind on *every*
   call - right for one batch, a defect in a loop.
3. **Push `where` and `select` into the read** instead of filtering after it.
   Record options carry them as sections: hive leaves prune by
   `partition_pairs`, Parquet row groups and Iceberg manifests by statistics,
   only `read_columns()` is decoded, and only surviving rows reach the host
   (see `yggdryl-records`).
4. **Identity costs nothing.** Shape the clause so the cheap rows of this
   table apply:

   | Shape | Buffers |
   | --- | --- |
   | `select *`, a self alias, a cast to the type a column has | skipped at bind: the reader or batch that came in |
   | a mask keeping every row | the input batch handed back (Rust, Python: the same object) |
   | a mask keeping some rows | copied, because a batch is dense |
   | a projection of bare columns | arrays reordered, no buffer touched |
   | `offset`, `limit` | slice views over the batches they cross |
   | `order by` | the stream collected once, one sort, one `take` |
   | any of the above from JavaScript | copied through Arrow IPC in and out |

5. **`order by` is the only section that collects.** `limit` and `offset` are
   slices, pushed into the read only when nothing orders. Filter first and
   order only when the answer needs it.
6. **Statistics answer `false` only when no row can match.** `true` means
   "must read", never "matches"; a user function is unknown to statistics, so
   it forces a row read and never a wrong skip.
7. **Null is unknown.** A comparison with a null is unknown and a `where`
   keeps only exactly-true rows; test absence with `is null`, `is distinct
   from` or `coalesce`.
8. **Best effort, then a named refusal.** A constant coerces into the operand
   it meets (`i = '1'` on `int64` binds as `i = 1`); operands with no common
   type compare as text (`s > 1` on `utf8` is `s > '1'`); a declared column
   casts safely unless it is `not null`, which refuses naming the column and
   value. Decimals never become floats - pass exact values, and write a
   fractional literal against a decimal typed (`decimal(9,2) '9.50'`): an
   untyped `9.5` is `float64`, shares no type with a decimal, and so compares
   as text.
9. **Paths are resolved, never split.** A `FieldPath` is parsed once at the
   boundary; quote a dotted name (`"a.b"`); never split a path string on `.`.
10. **Parameters bind once.** `:name` values go to `bind_with` / `bind(root,
    params)`. A missing one is refused naming it; an extra one is silently
    ignored and a repeated one (Rust slice) takes its first value, so compare
    against `parameters()` yourself when a misspelt key must fail.
11. **Canonical text is identity.** Every layer's text re-parses to the same
    tree and `stable_hash` / `stableHash` hashes it - a safe key for a cache of
    parsed clauses; a cache of *bound* plans keys on the clause hash together
    with the schema's `Field` `stable_hash`, because a `Bound` is resolved
    against one schema.
12. **A `Field` is a plan holder.** `into_field` stores each derivation as
    `TRANSFORM:function` + `TRANSFORM:sources` (a call over plain columns) or
    `TRANSFORM:expression`, and `Field::apply_arrow_batch` /
    `apply_arrow_reader` (Python `field.apply_arrow_batch(batch)`; not bound
    in JavaScript) recomputes them on a batch - see `yggdryl-arrow` for the
    cast it runs first. Keep the source columns in the selector: the stored
    field is what the recompute reads.
13. **DuckDB names the vocabulary**: `* exclude (...)`, `unnest`/`explode`,
    `in`, `between`, `is null`, `case when`, `asc`/`desc nulls first`. Joins,
    aggregates, windows and regexes are refused, not emulated.

## Pitfalls

| Wrong | Right |
| --- | --- |
| `Term.column("ccy").eq("EUR")` - a string is term *text*, so this compares two columns | Python `eq(Term.literal("EUR"))` or `eq("'EUR'")`; JS `eq(Term.literal(Scalar.from('EUR')))` or `eq("'EUR'")` |
| Python `Term.column("p") > 100` or `== ...` | `.gt(100)`, `.eq(...)`: comparison operators are Python ordering and equality, only `&`, `\|`, `~` and arithmetic build terms |
| JS `Term.column('p').gt(100)` (throws) / `Term.literal('x')` (throws) | `gt('100')`; `Term.literal(Scalar.from('x'))` |
| `Expression("a > 1")` (refused: an expression names its clause) | `Filter("a > 1")`, `Term("a > 1")`, or `Expression("where a > 1")` |
| `for b in reader: Filter(text).apply_arrow_batch(b)` | `Filter(text).apply_arrow_reader(reader)`, or `bound = f.bind(root)` once and `bound.filter_arrow_batch(b)` |
| `read_arrow_reader()` then filtering in pyarrow / Arrow JS | `read_arrow_reader(filter="...", select="...")` / `readArrowReader({ filter, select })` |
| a Python `float` or JS number against `decimal(9,2)` | `decimal.Decimal("1.50")`, `Scalar.decimal(150n, 2)` |
| `where price > 9.5` on a `decimal` column - `9.5` is a `float64` literal, which shares no type with a decimal, so it silently compares as text (`cast(price as utf8) > '9.5'`, and 10.00 fails) | `price > decimal(9,2) '9.50'`, an integer literal (`price > 9`), or a `:param` bound to `decimal.Decimal` / `Scalar.decimal` |
| `a / b` on two integers expecting a fraction (`7 / 2` is 3, truncated toward zero) | `cast(a as float64) / b`, or a decimal operand |
| `*, upper(name) as name` to replace a column (refused: `name` twice) | `* exclude (name), upper(name) as name` - the column moves to the end |
| a bound-plan cache keyed on the clause's `stable_hash` alone | key on the clause hash and the schema `Field`'s `stable_hash` |
| `bind(root, {"lo": 1, "hi": 2})` expecting the unused `hi` to fail | extras are ignored; check the keys against `parameters()` |
| `where x = null` | `where x is null` (`= null` is unknown for every row) |
| `a.b` meaning one column named `a.b` | `"a.b"` (quoted); unquoted it is two levels |
| JS `bound.matches(Scalar.from({ a: 1 }))` | `bound.matches(Scalar.from([1]))` - a row is a sequence in schema order |
| reading `statistics_prune(...) == true` as "the file matches" | it means "cannot rule out"; only `false` skips |
| registering a function from JavaScript | register in Rust or Python; JS parses `ns.fn(x)` and refuses it at bind |

## Language references

- `references/rust.md` - read when writing Rust.
- `references/python.md` - read when writing Python.
- `references/javascript.md` - read when writing JavaScript / TypeScript.
- `references/grammar.md` - the grammar cheat sheet: every clause, verb,
  operator, path step and function, and the semantics that decide answers.

## Deeper

- Overview and the best-effort rule: https://platob.github.io/yggdryl/expression/
- Grammar, functions, refusals: https://platob.github.io/yggdryl/expression/grammar/
- Terms, literals, predicate segments: https://platob.github.io/yggdryl/expression/terms/
- Selectors, `* exclude`, declared columns: https://platob.github.io/yggdryl/expression/selectors/
- Filters and pushdown levels: https://platob.github.io/yggdryl/expression/filters/
- User functions: https://platob.github.io/yggdryl/expression/functions/
- Plans, verbs, locations, `execute`: https://platob.github.io/yggdryl/expression/plans/
- Evaluation tiers, statistics, Iceberg scans: https://platob.github.io/yggdryl/expression/evaluate/
- Paths: https://platob.github.io/yggdryl/types/paths/
- Record options and partition pruning: https://platob.github.io/yggdryl/media/#options,
  https://platob.github.io/yggdryl/holder/#pruning-and-filtering
- Sibling skills: `yggdryl-records` (where the pushed-down read runs),
  `yggdryl-types` (the `Field` a clause binds against),
  `yggdryl-arrow` (`BatchReader`, `Serie`, casts, the cast
  `Field::apply_arrow_batch` runs before `TRANSFORM:`), `yggdryl-hashing`
  (`stable_hash`).
