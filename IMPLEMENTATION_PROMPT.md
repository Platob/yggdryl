# Prompt: `Expression` — one recursive, typed, vectorized filter and projection tree

Implement the expression layer the workspace has been missing: a single
`Expression` value that parses from text, resolves its own output `Field`
against a schema, evaluates row-at-a-time over `Value`, evaluates vectorized
over Arrow arrays, selects on handle attributes (`&holder.size`,
`&holder.url`, `&holder.partition['year']`), and pushes down into the pruning
each engine already understands (Hive paths, Parquet statistics, Iceberg
manifests). Today the whole project spells a filter as `(&str, &str)`
equality pairs - `IOBase::children_where`, `IORecordOptions::filter_partitions`,
`Table::scan_where` / `overwrite_where` / `merge_where` - and that is the same
question asked five times in the weakest possible language. `Expression`
becomes the one representation those five surfaces share, with the equality
pair kept as sugar over it. Deliver it complete: fully implemented, edge-case
tested on every datatype including nested, benchmarked against implementations
the reader already trusts, and documented with running examples.

Work on branch `claude/generic-expression-impl-w6dyn3`; commit and push there.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** It is the real spec. The sections that govern this
   task: *Order of work* (line 9), *Source layout and scope* (17), *Storage and
   I/O contract* (109), *Table format contract* (228), *Documentation
   organization* (353), *Exact method vocabulary* (409), *Error message
   contract* (578), *Native value behavior* (604), *Parser contract* (626),
   *Arrow and allocation contract* (707), *Binding boundary contract* (841),
   *Python extension* (874), *JavaScript extension* (1064), *Required checks*
   (1128).
2. `rust/src/datatype/parser.rs` and `rust/src/field/parser.rs` — the two
   existing recursive grammars, `PARSE_RECURSION_LIMIT`
   (`datatype/parser.rs:15`), byte-positioned errors, top-level splitting that
   honors quoting, balanced-wrapper stripping. Your grammar is a third sibling
   written the same way; it never re-implements type parsing - a cast target
   goes to `DataType::from_str`.
3. `rust/src/generic/value.rs` — `Value` (line 348), the one native value, and
   `rust/src/generic/typed.rs` — `TypedValue`, a value paired with the datatype
   it belongs to. Literals are `TypedValue`, never bare Rust primitives.
4. `rust/src/field/value.rs` (`validate_value`, `canonicalize_value`) and
   `rust/src/field/cast/` — how a value is checked and converted against a
   `Field`. Expression typing reuses these; it never grows a second cast.
5. `rust/src/io/mod.rs` — `IOBase`, especially `children_where` (line 520),
   `partitions` (492), `glob` (446), and the three record methods
   (1043-1189). `rust/src/uri/pattern.rs` — globs and Hive partitions read off
   a `Url`.
6. `rust/src/iceberg/scan.rs` — `Filter` (line 99) and the residual it already
   computes: partition-column predicates answered by planning, everything else
   applied to rows. That split is the model for pushdown; generalize it, do not
   fork it.
7. `rust/src/arrow/` — `BatchReader`, `scalar_array` / `scalar_value`, the
   allocation and cache rules. Vectorized evaluation lives here-adjacent and
   uses the pinned `arrow-ord` / `arrow-select` / `arrow-cast` kernels.
8. `docs/generic.md`, `docs/io.md`, `docs/iceberg.md` — the documentation
   register to match.

---

## 1. Architecture

### 1.1 Module layout

New top-level module `rust/src/expression/`, categorized the way `field/` is
(`AGENTS.md:17` - modules own real implementation, never empty shells around a
monolith):

| file | owns |
| --- | --- |
| `mod.rs` | the `Expression` enum, constructors, generic state, traits |
| `parser.rs` | the recursive grammar and `FromStr` |
| `display.rs` | canonical `Display` that round-trips through `FromStr` |
| `typing.rs` | recursive output-`Field` resolution against a schema |
| `bind.rs` | `Expression` → `Bound`: the compile step (see 1.6) |
| `eval.rs` | scalar evaluation over `Value` |
| `arrow.rs` | vectorized evaluation over `RecordBatch` (`arrow` feature) |
| `selector.rs` | `&holder.*` attribute selectors and their cost classes |
| `pushdown.rs` | partition pruning, statistics pruning, residual |
| `serde.rs` | structural serialization |
| `tests.rs` | the module's edge cases |

`Expression` is re-exported at the crate root beside `DataType`, `Field`,
`Uri`, `Url`, `Urn`, `Value`.

### 1.2 The enum: categorized nodes, no stringly typing

`Expression` is one `#[non_exhaustive]` enum whose variants are grouped by what
kind of node they are, each group documented as a group:

- **Leaves** — `Literal(TypedValue)`; `Column(SmolStr)`; `Path(Box<Expression>,
  Arc<[Segment]>)` where a `Segment` is a struct field name, a list index, or a
  map key, so `trade.legs[0]['ccy']` is one node resolving recursively through
  nested types; `Attribute(Selector)` (1.4); `Parameter(SmolStr)` for a
  late-bound value.
- **Logical** — `And(Arc<[Expression]>)`, `Or(Arc<[Expression]>)`, `Not(Box<_>)`.
  N-ary, not nested binaries: flattening at construction is what makes
  pushdown, display, and equality stable.
- **Comparison** — `Compare(Box<Expression>, Comparison, Box<Expression>)` with
  `Comparison` a small enum (`Eq`, `NotEq`, `Lt`, `LtEq`, `Gt`, `GtEq`,
  `IsDistinctFrom`, `IsNotDistinctFrom`); `In`, `Between`, `IsNull`,
  `IsNotNull`, `Like { case_insensitive }`, `Glob` (delegating to
  `Url::matches_glob` semantics), `Regex` only if it costs no new dependency -
  otherwise omit it and say so in the docs.
- **Arithmetic** — `Arithmetic(Box<_>, Operator, Box<_>)` (`Add`, `Sub`, `Mul`,
  `Div`, `Rem`, `Neg`) with decimal scale rules that follow `Value::Decimal`
  exactly: a decimal never becomes a float to be added.
- **Scalar functions** — one `Function(Function, Arc<[Expression]>)` variant
  over a closed `Function` enum (string: `lower/upper/length/substring/trim/
  starts_with/ends_with/contains/concat`; temporal: `year/month/day/hour/
  truncate`; null: `coalesce/if_null`; container: `size/element_at`). A closed
  enum, never a name lookup at evaluation time.
- **Shape** — `Cast(Box<Expression>, DataType, Safe)`, `Case` (searched
  `WHEN/THEN/ELSE`), `Struct(Arc<[(SmolStr, Expression)]>)`,
  `List(Arc<[Expression]>)`, `Map(Arc<[(Expression, Expression)]>)` so an
  expression can *build* nested values, not only read them.

Rules for the enum itself: `Arc` for shared nesting (`AGENTS.md:651`), empty
children with no backing allocation, `Clone`/`Debug`/`Display`/`Eq`/`Ord`/
`Hash`/`Serialize`/`Deserialize`, `Display` canonical and round-tripping,
`Debug` diagnostic only, never a panic on caller input, and an explicit node
budget checked before any recursive walk. Construction normalizes: flatten
nested `And`/`And`, fold `Not(Not(x))`, sort nothing (order is data, and
short-circuit order matters).

**`Expression` is not a `Value` variant.** A value is data; an expression is a
plan over data. `Value` stays the codec's lossless value tree, and the two meet
at exactly two points: `Expression::Literal(TypedValue)` going in, and
`Expression::eval(...) -> Value` coming out. Keeping them separate is what lets
`Value` stay serializable structural data (`AGENTS.md:604`) while an expression
carries schema-dependent meaning. Say this in one sentence in the module docs
so the next reader does not re-open it.

### 1.3 Typing is recursive and total

`Expression::field(&self, schema: &Field) -> Result<Field>` returns the output
field - name, datatype, nullability - resolved recursively, and it is the only
place output types are decided:

- a `Column`/`Path` resolves through the struct root by `index_of` /
  `get_field_by_name`, ASCII case-insensitively the way every cast selects,
  descending into struct children, list values, and map values for each
  `Segment`; an unknown name is an error naming the fields that do exist;
- a comparison is `Bool`, nullable when either side is;
- arithmetic follows a documented promotion matrix over the integer, floating,
  decimal, and temporal families - never "promote to f64";
- `Cast` is the declared target, `safe` deciding nullability;
- `Struct`/`List`/`Map` build the nested datatype from their children, so
  `Expression::field` on a nested expression is itself recursive and hits the
  same recursion limit;
- `Attribute` types come from the fixed table in 1.4.

`Expression::is_predicate(schema)` is `field(schema)?.data_type() == Boolean`.
Every entry point that wants a filter checks that once and says
`expected a boolean predicate, got <datatype>` when it does not.

### 1.4 `&holder.<attribute>` — selectors over the handle, not the rows

A filter over a listing asks about the *location*, not the columns, and today
nothing can express it. `&` opens the attribute namespace, `holder` is the
namespace this task implements (`&field.*` and `&schema.*` are reserved,
unimplemented, and rejected with that word). Each selector has one fixed
datatype and one **cost class**, and the cost class is the point:

| selector | datatype | cost |
| --- | --- | --- |
| `&holder.url` | Utf8 | free |
| `&holder.scheme`, `&holder.authority`, `&holder.path` | Utf8 | free |
| `&holder.name`, `&holder.stem` | Utf8 | free |
| `&holder.suffixes` | List(Utf8) | free |
| `&holder.query['k']`, `&holder.fragment` | Utf8 nullable | free |
| `&holder.partition['year']` | Utf8 nullable | free (read off the path) |
| `&holder.depth` | UInt32 | free |
| `&holder.media_type`, `&holder.mime_type`, `&holder.codec` | Utf8 | free |
| `&holder.kind`, `&holder.is_container` | Utf8 / Bool | one stat |
| `&holder.size`, `&holder.is_empty` | UInt64 / Bool | one stat |

"Free" means derived from the `Url` alone - no backend call. `bind` orders
conjunct evaluation cheap-first and short-circuits, so
`&holder.name like '%.parquet' and &holder.size > 0` never stats a file whose
name already disqualified it. Prove that with a counting mock in tests: the
number of backend calls is a behavior, not an implementation detail.

Then generalize the one surface that wants this:
`IOBase::children_where(&[(&str, &str)], bool)` gains a sibling
`children_matching(&Expression, bool)` (naming per `AGENTS.md:409`), the
existing method becomes a thin call into it building an `And` of equalities,
and glob descent still prunes: a predicate that constrains
`&holder.partition['year']` prunes directories exactly as the equality pairs do
today - nothing under a losing prefix is listed or decoded.

### 1.5 One grammar, recursive at every level

`Expression::from_str` is the single entry point, and every nested construct
re-enters it. A `select` list parses each item through it; a `where` clause
parses through it; a `case` arm, a function argument, a cast operand, a list
element, a `Path` base - all of them. There is no second parser and no
per-construct sub-parser that could accept a different language than the top
level. Concretely:

- `Expression::from_str("price > 100")`, and the statement-shaped
  `Statement::from_str("select ccy, price * qty as notional where price > 100
  order by notional desc limit 10")` where `Statement` is a small struct of
  `Vec<Expression>` projections, `Option<Expression>` predicate, ordering, and
  limit - each field parsed by recursing into the expression grammar. The
  statement form is what a binding hands over in one string, and it is how
  "select parses, and when it detects `where` it recurses into the same
  parser" is satisfied literally.
- Accept the forms the ecosystem writes: SQL comparison and boolean keywords
  (`and/or/not/in/between/is null/like/ilike`), C-style operators
  (`&&`, `||`, `!`, `==`, `!=`), quoted identifiers with case and Unicode
  preserved, single- or double-quoted string literals with escapes, typed
  literals (`date '2024-01-01'`, `timestamp '...'`, `decimal '1.50'`,
  `x'00ff'`), `cast(x as decimal(18,2))` delegating the type text to
  `DataType::from_str`, nested paths `a.b[0]['k']`, and the `&holder.*`
  namespace.
- Follow the parser contract exactly (`AGENTS.md:626`): split only at top-level
  separators honoring quoting and escapes; reject trailing tokens, duplicate
  projection aliases, malformed numbers, unbalanced delimiters; never strip
  unmatched or interior delimiters heuristically; enforce
  `DataType::PARSE_RECURSION_LIMIT`; every error carries a byte position and
  context; every grammar branch gets a round-trip test and an adversarial test.
- `Display` emits canonical text that re-parses to an equal expression,
  parenthesized by precedence and no more.

### 1.6 `bind` once, evaluate many — the execution plan

`Expression` is unbound and schema-free; `Bound` is what runs:

```rust
let bound = expression.bind(&schema)?;   // once per stream
let mask  = bound.filter_mask(&batch)?;  // per batch, no schema work
```

`bind` resolves column names to indices, casts each literal to its comparison
column's type once, folds constants, orders conjuncts by cost class, and
returns a typed error if anything does not line up. Per `AGENTS.md:707` there
are **no per-record maps or schemas**: a `Bound` holds the resolved plan and a
batch evaluation touches no name lookup. Three evaluation surfaces, one plan:

1. **Scalar** — `Bound::eval(&Value) -> Result<Value>` and
   `Bound::matches(&Value) -> Result<bool>` over a `Value::Record`/`Sequence`
   row. This is the tier that works in a build without the `arrow` feature; the
   whole module below `arrow.rs` compiles with `--no-default-features`.
2. **Vectorized** — `Bound::evaluate(&RecordBatch) -> Result<ArrayRef>` and
   `filter_mask(&RecordBatch) -> Result<BooleanArray>`, built from the pinned
   `arrow-ord` comparison kernels, `arrow-select` (`filter`, `take`, `zip`,
   `nullif`), and `arrow-cast`. Write no elementwise loop a kernel already
   owns. Null semantics are three-valued SQL and match Arrow's kernels exactly
   - test that against Arrow, do not assert it.
3. **Streaming, zero-copy** — `Bound::filter_reader(BatchReader) -> BatchReader`
   and `project_reader`, transforms that hold at most one batch, never
   materialize a table, and avoid copying where the shape allows: an all-true
   mask returns the input batch untouched (no `filter` call at all), an
   all-false mask yields an empty slice, a projection reorders `ArrayRef`s
   without touching buffers, and a selective mask goes through the shared
   selection kernel rather than a hand-rolled compaction. Wire it into the
   record methods so a caller can filter what a handle decodes without reading
   the whole thing, and into raw byte access so a line-oriented handle
   (`read_lines_matching`) and a record handle answer the same predicate the
   same way.

### 1.7 Pushdown: the same expression, answered as early as possible

`pushdown.rs` splits a bound predicate into what a layer can answer and what is
left over - the *residual* - exactly as `iceberg/scan.rs` already does for
partition columns, generalized:

- `partition_split()` → the part answerable from Hive path partitions plus the
  residual, so `IOBase` folder reads prune directories and
  `IORecordOptions::filter_partitions` is re-expressed as one `Expression`
  without changing its behavior;
- `statistics_prune(min, max, null_count, row_count)` → `true` when a container
  *may* contain a matching row, used for Parquet row groups and Iceberg
  manifest entries. Bounds prune a container; they never select a row - keep
  that sentence in the code.
- `Table::scan_where(&[(&str,&str)])` keeps its signature and gains
  `scan_matching(&Expression)`; the internal `Filter` becomes a `Bound`
  predicate, and the residual it computes today is this module's residual. Same
  for `overwrite_where` / `merge_where`. One filter representation in the
  workspace after this task, not two.

---

## 2. Order of work (`AGENTS.md:9` — Rust first, fully)

**Phase 0** research note → **Phase 1** Rust core complete (enum, parser,
typing, bind, scalar eval, vectorized eval, selectors, pushdown, tests, benches,
docs) → **Phase 2** Python → **Phase 3** JavaScript → **Phase 4** docs and
benchmark tables → **Phase 5** required checks. Phase 1 stopping on its own is
complete work.

---

## 3. Phase 0: what the outside world already settled

Before writing the enum, spend one pass reading how existing systems spell this,
and write `docs/expression.md`'s design section from it (short, cited, and
opinionated - not a survey):

- **Apache Iceberg expressions** — unbound vs bound expressions, `ref`,
  residual evaluation, `NaN` handling, and why partition predicates are
  answered by planning. The closest prior art to this design; deviate only with
  a stated reason.
- **Substrait** — the cross-engine serialized plan format: check whether the
  node categories here map onto its `Expression` message, and record the
  mapping even if no translation ships in this task.
- **Arrow Acero / `arrow::compute::Expression` and pyarrow.dataset filters** —
  the vectorized surface and the pushdown protocol pyarrow itself uses.
- **DataFusion `Expr`, Polars expressions, DuckDB, Spark Catalyst, Calcite
  `RexNode`** — for the operator set, null semantics, and decimal promotion
  rules that users will expect.
- **JMESPath / JSONPath and S3 Select** — for nested path syntax over
  semi-structured values; borrow the spelling users already know.
- **sqlglot** — an outside parser to check the grammar against in the Python
  tests (`AGENTS.md:12` wants an outside check for anything exchange-shaped),
  if it can be a test-only dependency; otherwise state the check performed
  instead.

The deliverable of this phase is a *decision list* in the docs: which forms are
accepted, which were deliberately refused, and which are reserved.

---

## 4. Phase 1 details: tests

`rust/src/expression/tests.rs`, plus per-file test modules where the existing
modules keep them. Cover, at minimum:

- **Parsing**: every operator, precedence and associativity (against a hand
  table), quoted identifiers with Unicode, every literal form, `select ... where
  ... order by ... limit`, nesting to the recursion limit and one past it,
  adversarial inputs (unbalanced quotes, trailing tokens, `a in ()`, `1 +`,
  deeply nested `not`), byte position correctness in every error, and a
  round-trip property test: `parse → display → parse` is stable for every
  constructed expression.
- **Typing**: output field for every node over every datatype family, including
  nested struct/list/map/dictionary/run-end/union columns; unknown column names
  error naming what exists; a non-boolean `where` is refused; the promotion
  matrix is asserted case by case, including decimal scale and temporal units.
- **Scalar evaluation**: three-valued logic truth tables; null propagation;
  `IsDistinctFrom` against null; nested path reads on struct-in-list-in-map;
  building nested values with `Struct`/`List`/`Map`; decimal arithmetic exact
  to the scale; every `Function` on empty, null, and boundary input.
- **Vectorized equivalence** (the important one): a property test asserting
  that for a batch of random values, `Bound::evaluate` over the batch equals
  the scalar `Bound::eval` applied per row, for every node kind and every
  datatype - including nulls and nested types. Scalar and vectorized answering
  differently is the bug this module can most easily hide.
- **Selectors**: a counting mock backend proving the cost ordering and that a
  free-attribute predicate performs zero backend calls; partition pruning skips
  unlisted directories; glob and predicate compose.
- **Zero copy**: an all-true mask returns a batch whose buffers are pointer-
  equal to the input's; a projection does not touch child buffers; a stream over
  a large synthetic table holds at most one batch (assert peak, do not claim it).
- **Pushdown**: `partition_split` residual correctness (the union of pruned and
  residual answers exactly what the full predicate answers) over Hive layouts
  and Iceberg tables; statistics pruning never prunes a container that holds a
  match (false negatives are a correctness bug, false positives are fine).
- **Round trips**: serde structural round trip; `Eq`/`Ord`/`Hash` consistency;
  no panic on any caller input (fuzz the parser with a bounded random corpus in
  a test, not a new dependency).

Errors follow `AGENTS.md:578`: typed, located, `expected X, got Y`, byte
position for parse errors, column name plus the available names for resolution
errors.

---

## 5. Phase 1 benchmarks

`rust/benchmarks/expression.rs` with the dispatcher pattern
(`#[path = "expression/mod.rs"] mod benchmarks;`, stable criterion group IDs),
measuring **parse and apply separately** - the user asks for both, and they
have nothing to do with each other:

- **Parse**: a short predicate, a wide `select` list, a deeply nested boolean
  tree, a nested-path expression; plus `parse → display → parse`. Baseline: the
  existing `DataType::from_str` benchmarks, so the reader can see the grammar
  costs the same order as the schema grammar.
- **Bind**: binding cost against a 10-column and a 200-column schema, to prove
  binding is once-per-stream and not once-per-batch.
- **Apply, vectorized**: filter selectivity sweep (1%, 50%, 99%, 100%) over
  1e6-row batches; comparison, boolean tree, string predicate, nested path
  read, arithmetic projection. **Baseline: the raw `arrow-ord` /
  `arrow-select` kernel call doing the same thing by hand** - that is the
  number that says whether this layer costs anything. Report the overhead.
- **Apply, scalar**: the same predicates over `Value` rows, to size the
  no-`arrow` tier honestly.
- **End to end**: read a Parquet/IPC file through `filter_reader` versus
  reading it whole and filtering after, showing what pushdown saves.

Python and JavaScript benchmarks compare against implementations the reader
trusts on the same payload: `pyarrow.compute` / `pyarrow.dataset` filters,
and - if available in the bench environment without becoming a package
dependency - DuckDB and Polars on the identical data. Numbers are regenerated
into `docs/benchmarks.md`, never edited (`AGENTS.md:392`), naming machine,
interpreter, and build profile.

---

## 6. Phase 2: Python binding

`python/src/expression.rs` exposing `yggdryl.Expression` over the native tree -
no Python-side parsing, no Python-side evaluation (`AGENTS.md:841`):

- `Expression.parse("price > 100")`, `Expression.column("price")`,
  `Expression.literal(value)`, `.field(schema)`, `.matches(row)`,
  `.evaluate(batch)`, `.filter(reader_or_table)`, `str(expr)` canonical.
- A builder DSL users already know from polars/pyarrow: `col("price") > 100`,
  `&`, `|`, `~`, `.is_null()`, `.isin([...])`, `.cast("decimal(18,2)")`,
  `.like("%.parquet")`, `holder.size`, `holder.partition["year"]`. Operator
  overloads construct native nodes directly.
- Every existing filter argument accepts `str | Expression` beside the pairs it
  accepts today: `IOBase.children_where`, `read_arrow_batch_reader` options,
  `Table.scan_where`. A `str` is parsed natively; a `pyarrow.compute.Expression`
  is refused naming what was expected (do not silently reinterpret a foreign
  expression object).
- Zero-copy across the boundary: a filtered read returns Arrow data through the
  existing record path with no Python-level row loop anywhere.
- `_native.pyi` and `__init__.pyi` updated; `mypy --strict` green.
- `python/tests/test_expression.py` in house style: parsing parity against
  `sqlglot`-normalized SQL where applicable, evaluation parity against
  `pyarrow.compute` on the same batch for every operator and datatype
  (**this is the outside-implementation check**), dataset-level filter parity
  against `pyarrow.dataset` pushdown, nested types, null semantics, and error
  messages surfacing unchanged.

---

## 7. Phase 3: JavaScript binding

`node/src/expression.rs`, mirroring the Python surface with camelCase names:
`Expression.parse`, `Expression.column`, `expr.field(schema)`,
`expr.matches(row)`, `expr.evaluate(batch)`, and a builder
(`col('price').gt(100).and(col('ccy').eq('EUR'))`) since JS has no operator
overloading - say that in the docs rather than emulating it. 64-bit values
cross as `bigint`; errors surface the native message unchanged.
`node/tests/expression.test.js` + `expression.types.ts` (node:test +
`tsc --noEmit`): parse/display round trips, evaluation parity with the Rust
tier via fixtures, nested types, and type-level checks for the builder.

---

## 8. Phase 4: documentation

- New page `docs/expression.md` — one H1, exactly one opening sentence, then
  example-first sections: parse a predicate; the grammar (a compact table);
  typing against a schema; scalar vs vectorized vs streaming; `&holder.*`
  selectors with the cost table; filtering a folder listing; filtering a record
  read; pushdown and residual; building nested values; the decision list from
  Phase 0.
- Every example in **Rust → Python → JavaScript tabs, in that order**, each
  idiomatic, self-contained, with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. Check `.api-bindings.txt` before
  showing a language do anything.
- Add the page to `mkdocs.yml` (a `Values:` entry beside `uri`, or its own
  slot if the nav reads better that way - state which and why in the commit).
  Update `docs/io.md` (`children_matching`), `docs/generic.md` (the
  `Value`/`Expression` boundary), `docs/iceberg.md` (`scan_matching`), and
  `docs/architecture.md`. Regenerate notebooks with
  `python scripts/build_docs_notebooks.py` (edit blocks, never notebooks).
  Update the README layout table for `rust/src/expression/`.
- `docs/benchmarks.md` regenerated from real runs, release builds only.
- `python -m mkdocs build --strict` stays green.

---

## 9. Phase 5: required checks (all must pass before handoff)

Per `AGENTS.md:1128`: `cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` **twice**
(default features and `--features "parquet iceberg"`); workspace tests twice the
same way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; the Rust 1.85 core
check (default features and `--no-default-features --lib` — the scalar tier must
compile without `arrow`); `cargo bench --benches --no-run`; maturin develop +
pytest + `mypy --strict`; `npm run test:package` + `npm test`;
`python scripts/check_docs_examples.py`; `python -m mkdocs build --strict`.
Clean generated targets, venvs, and `node_modules` after validation.

---

## 10. Hard constraints, restated

- **No new dependency** in any of the three manifests. The grammar is
  hand-written like the two existing ones; evaluation uses the already-pinned
  Arrow crates; no regex crate, no parser generator, no runtime.
- **One representation.** After this task the workspace has exactly one filter
  type. The `(&str, &str)` pairs stay as public sugar that builds an
  `Expression`; they never keep a parallel implementation.
- **Never a second parser, a second cast, or a second error enum.** Type text
  goes to `DataType::from_str`; value conversion goes through `field/cast`;
  failures are `crate::Error`.
- **Bind once.** No name lookup, schema walk, or allocation of a per-record map
  inside a batch loop; measure before claiming any optimization
  (`AGENTS.md:707`).
- **Scalar and vectorized must agree**, including nulls, NaN ordering, decimal
  scale, and nested types. This is asserted by property test, not by review.
- **Zero copy where the shape allows**, and where it does not, a comment saying
  why a copy is unavoidable.
- Method names follow the exact vocabulary (`AGENTS.md:409`); Rust
  `children_matching`/`scan_matching` ↔ Python `children_matching`/
  `scan_matching` ↔ JS `childrenMatching`/`scanMatching`; argument names and
  order identical across languages.
- Commit in coherent steps (enum, parser, typing, bind, scalar eval, vectorized
  eval, selectors, pushdown, benches, python, node, docs) with descriptive
  messages; push the branch; do not open a PR.

**Definition of done**: a user writes
`table.scan_matching("ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'")`
in any of the three languages; the year predicate prunes manifests before a
single byte is read, the rest evaluates vectorized on Arrow batches, the
answer is identical to evaluating it row by row, and the benchmark table shows
what the layer costs over calling the Arrow kernel by hand.
