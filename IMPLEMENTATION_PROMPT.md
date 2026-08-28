# Expression implementation brief

Implement one recursive `Expression` tree for typed scalar/vector evaluation,
projection, handle selectors, and pushdown. It replaces equality-pair filters
across `IOBase`, record options, Parquet, Hive paths, and Iceberg. Follow
`AGENTS.md`; this file contains only Expression-specific decisions.

## Outcome

- Parse and display one canonical expression grammar.
- Resolve every node to an output `Field` against a Struct schema.
- Bind once into an executable plan; evaluate over `Scalar` and Arrow batches.
- Select holder attributes such as `&holder.size`, `&holder.url`, and
  `&holder.partition['year']`.
- Push predicates into Hive partitions, Parquet statistics, and Iceberg
  manifests; return a residual for row evaluation.
- Keep `(column, value)` equality pairs only as construction sugar.
- Ship Rust first, then Python/JS parity, benchmarks, and running docs.

## Read first

- `AGENTS.md` in full.
- `rust/src/{datatype,field}/parser.rs` for recursive grammar rules.
- `rust/src/generic/scalar.rs` and `rust/src/generic/typed.rs` for
  `Scalar`/`TypedScalar`.
- `rust/src/field/value.rs` and `rust/src/field/cast/` for validation/casting.
- `rust/src/io/mod.rs`, `rust/src/uri/pattern.rs`, and
  `rust/src/iceberg/scan.rs` for existing filters and pruning.
- `rust/src/arrow/` for batch readers, scalar arrays, kernels, and budgets.
- `docs/{generic,io,iceberg}.md` for documentation shape.

## Rust layout

Create `rust/src/expression/` with real ownership:

| File | Responsibility |
| --- | --- |
| `mod.rs` | public nodes, constructors, traits |
| `parser.rs` | recursive grammar and `FromStr` |
| `display.rs` | canonical round-tripping `Display` |
| `typing.rs` | recursive output-field resolution |
| `bind.rs` | schema/parameter binding and execution plan |
| `eval.rs` | scalar evaluation |
| `arrow.rs` | vectorized batch evaluation |
| `selector.rs` | holder selectors and cost classes |
| `pushdown.rs` | partition/statistics pruning and residual |
| `serde.rs` | structural serialization |
| `tests.rs` | focused module edge cases |

Re-export `Expression` beside `Scalar`, `DataType`, `Field`, and identifiers.

## Data model

Use one `#[non_exhaustive]` enum with categorized nodes:

- Leaves: typed literal, column, nested path, holder attribute, parameter.
- Logical: n-ary `And`/`Or`, unary `Not`; constructors flatten same-kind
  children for stable equality/display/pushdown.
- Comparison: `Eq`, `NotEq`, `Lt`, `LtEq`, `Gt`, `GtEq`,
  `IsDistinctFrom`, `IsNotDistinctFrom`, `In`, `Between`, `IsNull`.
- Arithmetic: add/subtract/multiply/divide/modulo/negate with checked numeric
  promotion and temporal rules.
- String/binary: starts/ends/contains, length, lower/upper, regex.
- Collection: list/map/struct construction, indexing, membership.
- Conditional: `IfElse`, `Coalesce`, `Case` only when one shared typing rule
  can resolve all branches.
- Cast: target `DataType`, safe flag; delegate to Field cast logic.
- Projection: ordered named expressions producing one Struct field.

Literals are `TypedScalar`, never bare Rust primitives. Paths are structured
segments, not reparsed strings. Node enums are closed generic vocabulary in
`generic`; no expression-local copies.

## Typing and binding

- Typing is recursive and total. Every node returns a complete `Field` or an
  error containing expected type, actual type, and expression path.
- Column lookup uses the core Struct-field rules and rejects ambiguous folds.
- Nullability propagates from operands and operation semantics; metadata is
  retained only when the output still represents the same source field.
- Numeric/temporal promotion uses one core matrix shared with scalar and Arrow
  evaluation. No evaluator-specific coercion.
- `bind(schema, parameters)` resolves names, paths, selectors, parameters,
  output fields, casts, kernels, and pushdown fragments once.
- `BoundExpression` stores immutable plans and shareable caches. It is cheap to
  clone, deterministic, thread-safe, and independent of batch contents.
- Parameter values are typed/cast during binding. Missing, extra, or invalid
  parameters fail before evaluation.

## Selectors

- Syntax starts with `&holder`; selectors address handle metadata, not rows.
- Required selectors: `size`, `row_size`, `column_size`, `url`, `media_type`,
  `kind`, `partition[key]`, and protocol metadata access.
- Each selector declares a cost: static, metadata read, listing, or row scan.
  Planning may use only costs allowed by the operation; it never triggers a
  hidden full read.
- Evaluate selectors through object/trait methods on the holder. Do not create
  free helpers or duplicate URI/partition/media parsing.

## Grammar

- Reuse datatype/field parser primitives for byte positions, balanced wrappers,
  quoting, escapes, top-level splitting, and recursion limits.
- Cast targets call `DataType::from_str`; literals call the canonical typed
  scalar parser. Never duplicate either grammar.
- Define explicit precedence for path/index, unary, multiplicative, additive,
  comparison, `not`, `and`, `or`, then projection.
- Canonical display is unambiguous and parses to an equal tree. Preserve names,
  strings, Unicode, and structured path segments exactly.
- Reject trailing tokens, empty n-ary nodes, invalid arity, duplicate projection
  names, unsupported operator/type pairs, and over-limit nesting with byte
  position and expression path.

## Evaluation

- Scalar evaluation takes a canonical row `Scalar` plus bound parameters and
  returns a validated `Scalar` under the resolved field.
- Arrow evaluation uses pinned Arrow kernels and returns arrays/batches without
  row materialization. Exact arrays pass through where semantics allow.
- Scalar and Arrow paths have identical null, NaN, overflow, division,
  temporal/timezone, decimal, enum, nested, and error behavior.
- Projection emits target order. Missing nullable/defaultable output follows
  Field casting; unknown or ambiguous input is rejected.
- No JSON bridge, per-row schema/map, unbounded cache, or panic on input.

## Pushdown

Compile an expression into `Pushdown { partition, statistics, residual }`:

- Partition clauses are those fully answerable from Hive/Iceberg partition
  values.
- Statistics clauses are conservative proofs from min/max/null counts. Unknown
  means read; a false negative is forbidden.
- Residual contains every clause not fully answered earlier and is evaluated on
  rows after decode.
- Preserve `And`/`Or`/`Not` semantics; partial `Or` generally remains residual
  unless every branch is safely answerable.
- Parquet and Iceberg consume the same pushdown plan. Equality-pair APIs call
  the Expression constructor and immediately enter this path.
- Report planned/read/skipped files and residual presence for tests/benchmarks.

## Core integration

- Replace filter tuple storage in record options with `Option<Expression>` or a
  bounded expression list using one canonical combination rule.
- Update `children_where`, scans, overwrite/merge filters, partition routing,
  and generic media dispatch to accept Expression. Keep equality construction
  sugar only where it reduces boundary friction.
- Expressions are generic `Scalar` values where serialization requires it; do
  not add a second tagged codec format.
- `IOMode` remains the only operation mode. Expression filters never select
  overwrite/append/merge intent.

## Python

- Native `Expression` wrapper with idiomatic operators that build nodes; no
  Python evaluator or parser.
- Accept native literals, enums, datetimes, decimals, collections, Field/
  DataType wrappers, and PyArrow scalars through existing `Scalar` inference.
- Batch evaluation accepts/returns PyArrow through the C Data Interface.
- Table/media methods accept an Expression or equality mapping and redirect to
  the same native plan.
- Implement stable repr/equality/hash/pickle and native errors.

## JavaScript

- Native `Expression` wrapper plus named builders; JS operators cannot be
  overloaded. Use camelCase only at the boundary.
- Infer native literals, bigint, Date, enums, collections, Field/DataType, and
  Arrow JS values through existing `Scalar` conversion.
- Batch evaluation uses the copied IPC boundary. Media/table methods redirect
  to the same native plan.
- Implement stable string/JSON/equality/hash/clone behavior and declarations.

## Tests

Cover:

- parse/display/serde round trips and adversarial grammar;
- every datatype family, widths, nulls, NaN, overflow, decimals, enums,
  timezone-naive/aware temporals, nested structs/lists/maps/unions;
- missing/ambiguous columns, invalid paths, parameter errors, recursion/budget
  limits, and atomic failure;
- scalar/Arrow parity for every operator;
- partition/statistics pruning truth tables, especially `Or`, `Not`, null, and
  unknown statistics;
- Hive, Parquet, and Iceberg end-to-end results and read/skipped counts;
- Python/JS inference, casting, hashing, serialization, Arrow interop, and
  record I/O parity.

## Benchmarks

Measure release builds for:

- cold parse/display and bind-once/evaluate-many;
- scalar evaluation against a direct Rust baseline;
- Arrow evaluation against equivalent Arrow kernels;
- exact-schema pass-through and cast-required batches;
- narrow/wide/deep projections, null-heavy data, large batches;
- Hive/Parquet/Iceberg pruning with time-to-first-result, bytes/files skipped,
  and residual cost;
- Python and JS boundary overhead without hiding conversion cost.

Keep fixtures outside measured loops. Publish generated results beside the
documented methods, with machine/runtime/build and trusted baseline.

## Documentation

- Update `docs/expression.md`, `docs/generic.md`, `docs/io.md`, media pages,
  extension pages, nav, and API registers.
- Use compact Rust/Python/JavaScript tabs with assertions. Show parse, bind,
  scalar evaluation, Arrow batch evaluation, holder selectors, and pushdown.
- Embed read/overwrite/append/merge benchmark results on the relevant media
  pages.
- Document only the current filter, options, and method contracts.

## Completion

Run the checks required by `AGENTS.md`, including default and feature-enabled
Rust builds, both extension suites/types, docs examples/strict build, relevant
benchmarks, and external Parquet/Iceberg comparisons. Handoff only the outcome,
changed surfaces, verification, remaining caveats, and exact next action.
