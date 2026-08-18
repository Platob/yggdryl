# Prompt: `expressions` — one generic expression engine for select, filter, cast, and transform

Implement `rust/src/expressions/`: a **self-contained, dependency-free expression
value** for the whole workspace — an `Expr` tree over the project's own
`DataType`, `Field`, and `Value`, parsed from a SQL-like text grammar
(`Expr::from_str`), **bound once** against a root struct `Field` into a resolved,
simplified, cheap-to-evaluate plan, then evaluated three ways from that one
plan: row-at-a-time over `Value`, vectorized over an Arrow `RecordBatch`, and
**three-valued over column statistics** so a file, a manifest, or a partition
directory is skipped without being read. The grammar isolates real names behind
encapsulators (`"total amount"`) and reaches inside values by child, key, index,
and range (`payload['id']`, `tags[0]`, `path[1:3]`), and one `apply` verb carries
the whole thing onto every type that holds values — a row, a `TypedValue`, an
array, a batch, a streaming `BatchReader` — or onto a `Field` alone, when the
caller only wants the schema the result would have.

Then make it the filtering and selection vocabulary the record surface already
wanted: today a filter is a `(column, value)` text pair
(`AGENTS.md:237`) — equality against a hard-coded string, nothing else — and a
selection is a list of column names. After this change the same call sites take
an expression: `venue = 'XNAS' AND price BETWEEN 10 AND 20 AND ts >= DATE
'2024-01-01'` prunes Iceberg manifests, data files, and `column=value`
directories through the statistics evaluator, filters the surviving rows through
the vectorized evaluator, and the pair vocabulary survives as sugar that builds
the same expression. Deliver it complete: fully implemented, edge-case tested,
benchmarked against baselines the reader trusts, checked against an outside
implementation, and documented with running examples.

Work on branch `claude/generic-expression-filtering-fm76gl`; commit and push
there. Do not open a pull request.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** It is the real spec. The sections that govern this
   task: *Order of work* (line 9), *Source layout and scope* (17), *Table format
   contract* (220 — especially 234–247, how a scan prunes and what a filter is
   today), *Documentation organization* (345), *Exact method vocabulary* (397),
   *Error message contract* (566), *Native value behavior* (592), *Parser
   contract* (614), *Arrow and allocation contract* (695), *Binding boundary
   contract* (829), *Python extension* (862), *JavaScript extension* (1052),
   *Required checks* (1116).
2. `rust/src/generic/value.rs` — `Value`, the one native value: every variant
   this engine evaluates over, its total `Ord` (line 672), and
   `Value::data_type` in `generic/inference.rs:62`.
3. `rust/src/generic/typed.rs` — `TypedValue<K>`, one value paired with the
   datatype it belongs to, and its Arrow projection (`to_arrow_array`,
   `from_arrow_array`). A literal in a bound expression is this pairing, never a
   loose `Value` beside a remembered name.
4. `rust/src/field/mod.rs` — `Field` as the schema: `get_field_by_name`,
   `index_of`, `validate_value`, `canonicalize_value`, `parquet_field_id`,
   `is_partition`/`partition_fields`.
5. `rust/src/datatype/parser.rs` and `rust/src/field/parser.rs` — the two
   existing recursive grammars. Your parser matches their shape exactly: byte
   positions in every error, an explicit recursion limit, top-level splitting
   that honors quoting, no heuristic delimiter stripping. **`CAST(x AS <type>)`
   reuses `DataType::from_str` for the type half** — never a second type
   grammar.
6. `rust/src/iceberg/scan.rs` — `Filter` (line 99) and the three pruning levels
   (`manifest_matches`:199, `file_matches`:251). This is the code the new engine
   replaces; read what it promises before you delete it, especially the
   partition-text-versus-cast-value split documented at lines 19–25.
7. `rust/src/io/partition.rs` — `partition_text` (line 79), `NULL_PARTITION`
   (41), `filtered_reader` (347) and `filter_rows` (385): the folder-side row
   filter, today a string comparison per row per column.
8. `rust/src/generic/options.rs` — `IORecordOptions` (line 46):
   `select_by_names` (96), `filter_partitions` (110), and the
   `record_options_fields!` macro (279) every encoding's settings struct uses.
9. `docs/io.md` §"Partition pruning and filtering" (line 2134) and
   `docs/iceberg.md` (lines 936–943, 1253–1318) — the documentation register to
   match and to update.

---

## 1. Architecture

### 1.1 The module

New module folder `rust/src/expressions/`, `pub mod expressions;` in
`rust/src/lib.rs`, **unconditional** (the core carries no Arrow dependency, so a
`--no-default-features` build gets the value, the parser, the binder, the row
evaluator and the statistics evaluator in full). Files, each owning real
implementation and never an empty shell:

| file | owns |
| --- | --- |
| `mod.rs` | `Expr`, its constructors, `Display`, `FromStr`, Serde, `columns`, `conjuncts`, `simplify`, `bind` |
| `parser.rs` | the SQL-like grammar and its byte-positioned errors |
| `bound.rs` | `Bound` — the resolved, typed, simplified plan; the one thing every evaluator reads |
| `eval.rs` | row evaluation over `Value` (unconditional) |
| `stats.rs` | `ColumnStats`, `Certainty`, bounds evaluation and residual computation (unconditional) |
| `select.rs` | `Selection` — the ordered projection of named expressions, and the root `Field` it produces |
| `arrow.rs` | vectorized evaluation over `RecordBatch` → `BooleanArray`, projection and filtering (`#[cfg(feature = "arrow")]`) |
| `apply.rs` | the `Apply` / `ArrowApply` extension traits and their type redirections (§1.9) |
| `tests.rs` | the module's edge cases |

Re-export exactly `Expr` and `Selection` from `lib.rs` beside `Value` (they
cross every boundary the way `Value` does); everything else is reached as
`yggdryl::expressions::*`.

### 1.2 `Expr` — the unbound value

One `#[non_exhaustive]`-free, closed, recursive enum. It is a *value*, not a
plan: `Clone`, `Debug`, `Display`, `Eq`, `Ord`, `Hash`, `Serialize`,
`Deserialize`, `Display` round-tripping through `FromStr`
(`AGENTS.md:592`). Nesting is shared through `Arc` so cloning a large predicate
does not copy it, and an empty child list carries no allocation.

The vocabulary, and nothing beyond it without a written reason:

- **`Column`** — a name path: a root column plus a chain of accessors — `a.b`
  into a struct, `a[0]` into a list, `m['k']` into a map, `a[1:3]` a range of
  either. Names keep case and Unicode, are isolated by encapsulators when they
  carry whitespace or punctuation (§1.3.1), and the chain is spelled by
  §1.3.2. Matching is ASCII case-insensitive, the way every cast and selection
  in this project already resolves names.
- **`Literal`** — a `Value`. `NULL` is `Value::Null`.
- **`Cast { expr, data_type, safe }`** — the schema-directed cast this project
  already owns; `safe` matches `IORecordOptions::safe` (a value the target
  cannot hold becomes null rather than an error).
- **Comparison** — `Eq`, `NotEq`, `Lt`, `LtEq`, `Gt`, `GtEq`.
- **Logical** — `And`, `Or`, `Not`.
- **Null tests** — `IsNull`, `IsNotNull`. These are the only operators that
  answer `true`/`false` about a null.
- **Membership** — `In { expr, list }`, `Between { expr, low, high }` (sugar the
  simplifier lowers to two comparisons).
- **Text** — `Like { expr, pattern, escape, negated, case_insensitive }` and
  `StartsWith` (`LIKE 'x%'` folds to it — it is the one text predicate a
  statistics range can prune).
- **Arithmetic** — `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg` over integers,
  floats, and decimals (decimal arithmetic keeps the exact coefficient/scale
  model `Value::Decimal` already carries; never route a decimal through `f64`).
- **`Function`** — a small closed set with SQL semantics only:
  `coalesce`, `length`, `lower`, `upper`, `trim`, `substring`, `abs`,
  `truncate(x, width)`, and calendar extraction
  `year|month|day|hour|minute|second` (also spelled `EXTRACT(YEAR FROM x)`),
  computed through `rust/src/generic/iso.rs` — **never a second calendar
  implementation, never a date crate**. Calendar functions return the *calendar
  field* (`year(DATE '2024-03-01')` is `2024`), which is SQL's meaning and
  deliberately **not** Iceberg's "years since 1970" transform; §3.3 says how the
  two are related without conflating them.
- **`Case { branches, otherwise }`** — the one conditional.

Constructors follow the vocabulary (`AGENTS.md:397`): `Expr::column("venue")`,
`Expr::literal(Value::from(3))`, and consuming combinators `and`, `or`, `not`,
`eq`, `lt_eq`, `is_null`, `cast_to`, `alias` — plus `From`/`FromStr`/`TryFrom`
alongside them. `Display` emits canonical SQL: minimal parentheses driven by
precedence, identifiers quoted with `"` only when they need it, strings with
doubled `'`, temporals as `DATE '…'` / `TIMESTAMP '…'` / `TIME '…'`, decimals
exactly as `Value` spells them.

Free of any schema, `Expr` answers:

- `columns()` — every column path it reads, deduplicated in first-seen order.
  This is what drives projection pushdown: a read decodes only the columns the
  filter and the selection actually need.
- `conjuncts()` — the top-level `AND` operands, flattened. Pruning is per
  conjunct, and a residual is the conjuncts a partition tuple did not settle.
- `simplify()` — pure rewriting, no schema: constant folding of literal-only
  subtrees; `AND`/`OR` absorption with `TRUE`/`FALSE`; `NOT` pushed through
  comparisons and De Morgan; `IN` of one element to `Eq`, of zero to `FALSE`;
  `BETWEEN` to two comparisons; `LIKE 'x%'` with no other wildcard to
  `StartsWith`; duplicate conjuncts dropped. Simplification is
  **semantics-preserving under three-valued logic** — that is the property the
  tests assert, not a list of rewrites.

### 1.3 The grammar — `Expr::from_str`

One recursive-descent parser in `parser.rs`, following the parser contract
(`AGENTS.md:614`) to the letter.

- Precedence, loosest first: `OR`, `AND`, `NOT`, comparison /
  `IS [NOT] NULL` / `[NOT] IN` / `[NOT] BETWEEN` / `[NOT] LIKE` / `ILIKE`,
  additive, multiplicative, unary `-`, postfix `::type`, primary.
- Primaries: literals (integers, decimals — a fractional literal is
  `Value::Decimal` with the scale as written, never an `f64`; floats only in
  exponent form or when the datatype demands it; `TRUE`/`FALSE`/`NULL`;
  single-quoted strings with `''` escaping; `DATE`/`TIME`/`TIMESTAMP`/`INTERVAL`
  typed literals parsed through `generic/iso.rs` and `TimeUnit::from_str`),
  identifiers (bare, `"quoted"`, dotted, `[index]`, `['key']`), `( … )`,
  function calls, `CAST(expr AS <datatype>)` and `expr::<datatype>` — the
  datatype half is `DataType::from_str`, so every type the schema grammar
  accepts is accepted here, including nested ones.
- Keywords are ASCII case-insensitive; identifiers and quoted values keep case
  and Unicode.
- Errors are `Error::Parse { target: "expression", position, reason }` with the
  byte offset and `expected X, got Y` (`AGENTS.md:566`): a trailing token, an
  unbalanced parenthesis, an unterminated string, an unknown function named
  beside the vocabulary it is not in, a datatype the type grammar rejects
  (position relative to the whole expression, not to the type fragment).
- An explicit recursion limit, matching the schema parser's, refused as a typed
  error rather than a stack overflow.
- `Display`→`from_str`→`Display` is a fixed point for every expression the
  tests build; adversarial inputs (deep nesting, unbalanced quotes, `IN ()`,
  `BETWEEN` with a missing `AND`, a 10 000-element `IN` list) are refused
  without panic (`AGENTS.md:636`).

### 1.3.1 Encapsulators — isolating a real name

A column called `total amount`, `order.id`, `select`, or `prix (€)` must be
addressable, and the only thing that can make it addressable is a delimiter pair
around it. The parser therefore has a **lexical layer that isolates a token
before the grammar ever sees it**, and the rules are the parser contract
(`AGENTS.md:614`) applied to names:

- **Identifier encapsulators, all three accepted**: `"double quoted"` (SQL
  standard), `` `backticked` `` (Hive/Spark/MySQL), `[bracketed]` (T-SQL,
  Databricks). Inside one, *everything* is part of the name: whitespace, `.`,
  `(`, `-`, `*`, operators, keywords, digits, leading digits, and Unicode. The
  closing delimiter is doubled to embed itself — `"say ""hi"" now"`,
  `` `back``tick` ``, `[bra]]cket]` — which is each dialect's own escape and
  keeps the token self-delimiting.
- **A double-quoted token is always an identifier, never a string.** That is the
  SQL rule and it removes the one real ambiguity in this grammar:
  `WHERE "venue" = 'XNAS'` compares a column to a string, `WHERE 'venue' =
  'XNAS'` compares two strings and folds to `FALSE`. Say it in the docs; test
  both.
- **String literals** are single-quoted with `''` doubling. A backslash is *not*
  an escape inside a string (SQL, not C) — the only place `\` is special is
  `LIKE … ESCAPE '\'`, where the escape character is whatever the clause names.
- **Whitespace is data inside an encapsulator and a separator outside it.** A
  quoted name is never trimmed: `"  a  "` names a column with two leading and
  two trailing spaces, and it must survive `Display` → `from_str` → `Display`
  unchanged. Outside, any run of Unicode whitespace separates tokens, and
  whitespace is never required between a token and a delimiter (`a=1`,
  `a IN(1,2)`, `f (x)` all parse).
- **Never strip an unmatched or interior delimiter heuristically.** An
  unterminated `"`, `` ` ``, `[`, or `'` is `Error::Parse` naming the byte
  offset **of the opener** plus what would close it — the position a caller can
  actually fix — not the offset of end-of-input. An interior delimiter that is
  not doubled ends the token; whatever follows is judged by the grammar.
- **`[…]` is two things, and position decides which** — this is the one real
  collision in the grammar, so decide it in the lexer and say so: a bracket in
  *primary* position (the start of an expression, or immediately after a `.`) is
  a **quoted identifier**; a bracket immediately following a completed primary
  is a **subscript** (§1.3.2), whose contents are literals and ranges only,
  never a bare name. `[my col] = 1` reads a column; `a[0]` reads an element;
  `a [0]` is the same subscript, because whitespace never changes what a token
  is. A bare identifier inside a subscript (`a[b]`) is a byte-positioned error
  naming both readings, never a guess.
- **Dotted paths encapsulate per segment**: `"my schema"."my column"`,
  `` `db`.`table`.`col` ``, `a."b.c"` — a `.` *inside* an encapsulator is part
  of the name, a `.` between two segments is the path separator. There is no
  splitting pass over the raw text; the path is built by the same postfix loop
  that reads accessors (§1.3.2).
- **Comments** — `-- to end of line` and `/* … */` — are skipped by the lexer,
  and neither can start inside an encapsulator.
- **Canonical `Display` re-quotes minimally and re-parses identically**: a name
  is emitted bare when it matches `[A-Za-z_][A-Za-z0-9_]*` and is not a reserved
  word, and in double quotes with doubling otherwise. The backtick and bracket
  forms are input spellings only — accepted, never emitted, which is how a
  grammar has three dialects on the way in and one canonical form on the way
  out.
- **Quoting isolates characters; it does not change matching.** An encapsulated
  identifier still resolves ASCII case-insensitively, because that is how every
  cast, selection, and struct reconciliation in this project resolves a name,
  and a second rule here would make `select "A"` and `select A` disagree. Two
  columns that fold together are an **ambiguity error naming both**, exactly as
  struct reconciliation already refuses an ambiguous fold — never a silent
  first-wins.
- The pair sugar of §3.1 builds its expression through the same quoter, so
  `filter_partitions([("total amount", "3")])` yields `"total amount" = '3'` and
  round-trips; a partition column with a space cannot become an unparseable
  filter.

### 1.3.2 Accessors — key, index, and range

One postfix chain, left-associative, binding tighter than everything else
including `::` (so `a.b[0]::int` casts the element). Written once in the
grammar, resolved once at bind time, applied by all three evaluators.

| spelling | means | on |
| --- | --- | --- |
| `a.b` | child by name | `Struct` (and a `Map` with string keys, as sugar for `a['b']`) |
| `a['b']` | item by key | `Map`; a `Struct` child when the key is a string literal |
| `a[0]`, `a[-1]` | item by position | `List`, `LargeList`, `ListView`, `FixedSizeList`, `Utf8*`, `Binary*` |
| `a[1:3]`, `a[1:]`, `a[:3]`, `a[:]` | a range of items | the same list, string, and binary types |

Inside brackets a double-quoted token is a **string literal key**, not an
identifier — the one place the §1.3.1 rule is relaxed, because there is no
column position inside a subscript. Say so where it is implemented, and test it.

Semantics, chosen to match what this repository already does rather than any one
SQL dialect, and documented as such:

- **Indices are 0-based**, matching `Value::get(index)` and Arrow/Spark `[]`. A
  negative index counts from the end (`a[-1]` is the last element). Spell that
  out beside the table; a reader expecting 1-based must be told once, clearly.
- **Ranges are half-open** — `a[1:3]` is elements 1 and 2 — matching Rust and
  Python, said next to the 0-based note. An omitted bound is the start or the
  end; a negative bound counts from the end; an inverted or empty range yields
  an empty list or string, never an error.
- **Out of range is null, not an error**: `a[99]` on a three-element list is
  null, `a[1:99]` clamps. Absence is not a failure on the read path anywhere
  else in this project, and a predicate over a ragged list must not abort a
  scan. A *range* clamps; an *index* nulls.
- **A range over text slices Unicode scalar values; over binary it slices
  bytes**, and never splits a character — the one place the two families differ.
  State it in the docs and test it with multi-byte input.
- **Resolution happens at bind time, from the datatype.** The chain resolves
  against the container's `DataType` — `Struct` to a child ordinal, `Map` to a
  key lookup with the key cast to the map's key type once, list to index
  arithmetic — so what `Bound` carries is a fixed slot chain and evaluation
  costs offset arithmetic with no name lookup. An accessor the datatype cannot
  answer (`a[0]` on a `Struct`, `a.b` on an `Int64`, a key the map's key type
  cannot hold) is a bind error naming the datatype, the accessor, and the path
  to it (`AGENTS.md:566`).
- **Arrow evaluation stays columnar where the layout allows it**: a struct child
  is a zero-copy column slice, a `FixedSizeList` index is a strided slice, a
  variable-length list index or range is offsets arithmetic plus
  `arrow_select::take`. A case that cannot be expressed with the crates the
  `arrow` feature already links falls back to the row path for that node only,
  with a comment naming the cost (§1.6).
- **Pruning through an accessor is conservative.** A `Struct` child usually has
  its own leaf statistics (Parquet and Iceberg both key bounds per leaf), so
  `a.b > 3` can prune; a list element, a map key, and every range answer
  `Maybe`, because no statistic bounds them. Getting this wrong loses rows, so
  the rule sits in `stats.rs` beside the code and is tested with a deliberately
  misleading list column.
- `Display` re-emits the chain canonically (`a.b[0]['k'][1:3]`) and round-trips.
- **A range is not `BETWEEN`.** `a[1:3]` selects items; `a BETWEEN 1 AND 3` is a
  predicate. Both exist, neither parses as the other, and the docs put them side
  by side once so no reader conflates them.

### 1.4 `Bound` — where the optimization lives

```rust
impl Expr {
    /// Resolve this expression against the schema it will be evaluated over.
    pub fn bind(&self, schema: &Field) -> Result<Bound>;
}
```

`bind` is the **one** place a name becomes a position and a text becomes a
typed value, and it happens once per read — never per batch, never per row
(`AGENTS.md:722`: no per-record maps or schemas). It:

1. resolves every `Column` to its ordinal path in the root struct `Field` plus
   the leaf `Field` itself, erroring with the columns the schema *does* have
   when a name is absent (the message shape `iceberg::scan::Filter::resolve`
   already uses at `scan.rs:124`);
2. computes each node's result `DataType`, refusing a comparison between types
   that have no common comparison type, naming both sides;
3. **folds every literal into the column's own type once** — a `TypedValue`
   built through `Field::canonicalize_value`, so `price > '10.5'` against
   `decimal(10,2)` compares two decimals, not a string and a decimal, and the
   conversion is not repeated per row. A literal the type cannot hold is a bind
   error naming both, **except** through the tolerant constructors of §3.1,
   which fold it to a statically-`FALSE` predicate exactly as the folder route
   tolerates an unmatched filter today;
4. runs `simplify()` before and after folding, so a bound plan is already
   minimal;
5. orders conjuncts cheapest-first (column comparison before arithmetic before
   `LIKE` before `CAST`), which is legal because evaluation is side-effect free
   and three-valued `AND` is commutative here;
6. returns a `Bound` that is `Send + Sync`, cheaply clonable (shared `Arc`
   nodes), and carries the schema it was bound to so an evaluator can never be
   handed a batch that disagrees — a mismatch is a typed error naming the
   differing column.

`Bound` answers `columns()`, `conjuncts()`, `data_type()`, `is_always_true()`,
`is_always_false()`, `to_expr()` (back to the unbound value, for `Display` and
serialization), and the three evaluations below. **There is exactly one
evaluation engine**: the row path, the vectorized path, and the statistics path
are three readings of the same `Bound`, and a behavior that differs between them
is a bug the tests catch.

### 1.5 Row evaluation over `Value` (unconditional)

```rust
impl Bound {
    pub fn evaluate(&self, row: &Value) -> Result<Value>;
    pub fn matches(&self, row: &Value) -> Result<bool>;
}
```

A row is one `Value::Record` (or a `Value::Sequence` in field order, or a
`Value::Mapping` by name — all three, since all three are what this project
calls a row). Semantics are **SQL three-valued logic**, stated in the module
docs and pinned by tests: a comparison with a null operand is unknown, `matches`
keeps a row only on `true`, `AND`/`OR` follow the standard tables, `IS NULL` is
the only way to select absence. Getting a column costs a slot index and a borrow
— no name lookup, no allocation, no intermediate `Vec` per row.

### 1.6 Vectorized evaluation over Arrow (`arrow` feature)

```rust
impl Bound {
    pub fn evaluate_batch(&self, batch: &RecordBatch) -> crate::arrow::Result<ArrayRef>;
    pub fn mask(&self, batch: &RecordBatch) -> crate::arrow::Result<BooleanArray>;
    pub fn filter_batch(&self, batch: &RecordBatch) -> crate::arrow::Result<RecordBatch>;
}
```

- Comparisons go through `arrow_ord::cmp` against a `Scalar` built **once** at
  bind time from the folded literal (`TypedValue::to_arrow_array` gives the
  one-row array; `arrow::scalar_array` is the Field-directed spelling) — never a
  per-batch cast of the literal, never `ArrayFormatter` per row, which is what
  `io/partition.rs:400` does today and what this replaces.
- Casts go through `ArrowCast` (`rust/src/field/cast/`), never a second cast.
- All conjuncts are AND-ed into **one** `BooleanArray` and
  `arrow_select::filter::filter_record_batch` runs **once** per batch — never a
  filter per predicate.
- Null handling matches the row path exactly: the mask's nulls are dropped by
  the filter, which is three-valued logic by construction.
- **No new dependency.** The `arrow` feature already pulls `arrow-array`,
  `arrow-buffer`, `arrow-cast`, `arrow-data`, `arrow-ord`, `arrow-row`,
  `arrow-select` (`rust/Cargo.toml:14`). Express every kernel with those. Where
  one genuinely cannot be (element-wise arithmetic, `LIKE`), evaluate that node
  through the row path over the batch's values and say so in a comment naming
  the cost — do **not** add `arrow-arith`/`arrow-string` without stating in the
  commit message why the fallback is unacceptable, and if you do add one it is
  pinned in the workspace table at the same `59.2.0` and gated by `arrow`.
- Measure before claiming an optimization (`AGENTS.md:722`); §5's benchmark is
  where the claim is settled.

### 1.7 Three-valued evaluation over statistics (unconditional)

This is what makes a filter *prune* instead of merely *filter*.

```rust
/// What one column's statistics say about a group of rows.
pub struct ColumnStats {
    pub lower: Option<Value>,
    pub upper: Option<Value>,
    pub null_count: Option<u64>,
    pub value_count: Option<u64>,
}

/// What the statistics let us conclude about a predicate over that group.
pub enum Certainty { AlwaysTrue, Maybe, AlwaysFalse }

impl Bound {
    /// Answer from statistics alone, reading columns through `stats`.
    pub fn evaluate_stats(&self, stats: &dyn StatsSource) -> Certainty;
    /// The conjuncts these statistics did not settle.
    pub fn residual(&self, stats: &dyn StatsSource) -> Bound;
}
```

Rules, and they are the correctness heart of the feature:

- **`Maybe` is always safe; `AlwaysFalse` must be provable.** A missing
  statistic, an unsupported node, an incomparable type: `Maybe`. A wrong
  `AlwaysFalse` loses rows silently, which is the one failure this module must
  never have — every rule gets a test that feeds it deliberately coarse
  statistics.
- Ranges: `Eq` is false when the literal is outside `[lower, upper]`;
  `Lt`/`Gt`/`LtEq`/`GtEq` compare against the relevant bound; `In` is false when
  every element is outside; `IsNull` is false when `null_count == 0`;
  `IsNotNull` is false when `null_count == value_count`; `StartsWith` prunes on
  the truncated string bounds (a prefix comparison, correct for truncated
  bounds only in the direction that widens — comment why).
- `AlwaysTrue` is claimed only where it is provable (e.g. `IsNotNull` with
  `null_count == 0`), because it is what lets `residual` drop a conjunct.
- Composition follows Kleene logic over `Certainty`.
- `StatsSource` is a tiny trait (`fn stats(&self, column: &BoundColumn) ->
  Option<ColumnStats>`) so the Iceberg manifest's `FieldSummary`, its data-file
  bounds, and a `column=value` directory each implement it without this module
  knowing any of them exists. A partition directory implements it as
  `lower == upper == the value`, which makes directory pruning a *special case
  of statistics pruning* rather than a second code path — that unification is
  the point of the whole design.

### 1.8 `Selection` — select, cast, transform

```rust
pub struct Selection { /* ordered (alias, Expr) */ }

impl Selection {
    pub fn from_str(text: &str) -> Result<Self>;         // "id, upper(venue) AS venue, price::decimal(10,2)"
    pub fn from_names<I>(names: I) -> Self;              // the existing select_by_names, verbatim
    pub fn bind(&self, schema: &Field) -> Result<BoundSelection>;
    pub fn is_empty(&self) -> bool;                      // empty selects everything
}

impl BoundSelection {
    pub fn schema(&self) -> &Field;                      // the root Field the projection produces
    pub fn evaluate(&self, row: &Value) -> Result<Value>;
    #[cfg(feature = "arrow")]
    pub fn project_batch(&self, batch: &RecordBatch) -> crate::arrow::Result<RecordBatch>;
}
```

An alias defaults to the expression's canonical `Display` when the caller gives
none, and a bare column selection produces the *same* root `Field` the current
`select_by_names` produces — that identity is a test, because it is what lets the
existing surface become sugar without changing an answer. A projection built
only of bare columns must still push down as a projection (Parquet mask, IPC
projection) exactly as today: computed columns are evaluated *after* the
encoding's own projection of the columns they read, never instead of it.

---

### 1.9 `Apply` and `ArrowApply` — one verb over every carrier

An expression is only useful where the values are, and the values are in a
dozen different shapes: a `Value` row, a `TypedValue`, an `ArrayRef` beside its
`Field`, a `RecordBatch`, a `StructArray`, a streaming `BatchReader` — and
sometimes there are no values at all and the caller only wants the `Field` or
the `DataType` the result *would* have. `rust/src/expressions/apply.rs` gives
all of that one verb, modeled directly on the precedent this repository already
has for exactly this problem: `ArrowCast` (`rust/src/field/cast/plan.rs:35`),
one trait implemented for `DataType` and `Field`, with a method per carrier.

Two traits, split on the feature boundary:

```rust
/// Applying an expression to whatever carries values. Unconditional.
pub trait Apply {
    /// Resolve against the schema the carrier will be read under, once.
    fn bind_to(&self, schema: &Field) -> Result<Bound>;

    /// The Field the result has - schema only, no data, nothing opened.
    fn apply_field(&self, schema: &Field) -> Result<Field>;
    /// The DataType the result has, for a carrier that is one value.
    fn apply_data_type(&self, data_type: &DataType) -> Result<DataType>;

    fn apply_value(&self, schema: &Field, row: &Value) -> Result<Value>;
    fn apply_values<'a, I>(&self, schema: &Field, rows: I) -> Result<ValueApply<'a>>
    where I: IntoIterator<Item = Value> + 'a;      // lazy, one row held
    fn apply_typed_value(&self, value: &TypedValue) -> Result<TypedValue>;

    /// The type-directed entry: whatever `target` is, do that carrier's thing.
    fn apply<T: Applicable>(&self, target: T) -> Result<T::Output>;
}

/// The Arrow carriers. Behind the default `arrow` feature.
#[cfg(feature = "arrow")]
pub trait ArrowApply: Apply {
    fn apply_arrow_array(&self, field: &Field, array: ArrayRef) -> crate::arrow::Result<ArrayRef>;
    fn apply_arrow_scalar(&self, field: &Field, array: ArrayRef) -> crate::arrow::Result<Scalar<ArrayRef>>;
    fn apply_arrow_batch(&self, batch: RecordBatch) -> crate::arrow::Result<RecordBatch>;
    fn apply_arrow_batch_reader(&self, batches: BatchReader) -> crate::arrow::Result<BatchReader>;
    fn apply_arrow<T: ArrowApplicable>(&self, target: T) -> crate::arrow::Result<T::Output>;
}
```

**Redirection on the subject side.** Both traits are implemented for every value
that *is* or *names* an expression, so the caller never converts by hand:
`Expr`, `Bound`, `Selection`, `BoundSelection`, `&str` / `String` (parsed by the
core parser, never by the caller — `AGENTS.md:843`, infer at the boundary and
compute in core), and `&[(&str, &str)]` (the partition-pair sugar of §3.1). A
subject that is already `Bound` skips binding; a `&str` subject parses and binds
per call, which the docs tell the reader to hoist out of a loop and the
benchmark of §5 puts a number on.

**Redirection on the carrier side.** `Applicable` / `ArrowApplicable` are small
traits with one associated `Output`, implemented once per carrier:

| carrier | `Output` | note |
| --- | --- | --- |
| `&Value` | `Value` | one row; `Value::Record`, `Sequence`, or `Mapping` |
| `&TypedValue` | `TypedValue` | value and datatype together |
| `&Field` | `Field` | the result schema, no data |
| `&DataType` | `DataType` | the result type of one value |
| `(&Field, ArrayRef)` | `ArrayRef` | one column with the field that describes it |
| `Scalar<ArrayRef>` | `Scalar<ArrayRef>` | the one-row pinned form |
| `RecordBatch` | `RecordBatch` | consumed, like `cast_arrow_batch` |
| `StructArray` | `StructArray` | rows as one struct column |
| `BatchReader` | `BatchReader` | consumed, streaming, lazy |

**What "apply" means is decided by the bound result type, defined once and
documented once:**

- a **boolean** expression *filters* a collection carrier (batch, reader, row
  iterator) and *evaluates* a single-value carrier (a row yields `Value::Bool`);
- a **non-boolean** expression *computes* one column, named by its alias or by
  its canonical `Display` when it has none;
- a **`Selection`** *projects* to its root `Field`.

`apply_*` dispatches to the explicit operations already on `Bound` and
`BoundSelection` (§1.5–1.8: `matches`, `mask`, `filter_batch`, `evaluate`,
`project_batch`) — it adds one verb, not a second implementation, and no
existing verb gains an alias (`AGENTS.md:397`).

Requirements that make this surface worth having rather than sugar:

- **`apply_field` opens nothing and allocates no data.** It is how
  `read_arrow_field` answers under a selection without touching a file, how a
  binding shows a user the output schema before a read, and how a write
  validates a projection up front. Its answer must equal the schema of the batch
  `apply_arrow_batch` produces from the same input — that equality is a test,
  not a comment.
- **`apply_arrow_batch_reader` is lazy and streaming**: it binds once against
  the reader's schema, and the returned reader answers `schema()` with the
  *result* schema immediately, before the first batch is pulled; it holds at
  most one batch, propagates a per-batch failure through the existing
  `arrow::Error` channel, and never materializes a vector. `filtered_reader` and
  `select_reader` in `rust/src/io/partition.rs` become one call to it each.
- **Binding happens once per apply, never per batch or per row.** A carrier that
  is a stream binds when the stream is built. This is the whole optimization
  claim of §1.4 and the benchmark of §5 is where it is settled.
- **The generic entries are zero-cost**: `apply` / `apply_arrow` monomorphize to
  the same call the explicit method makes — no `dyn`, no boxing on the value
  path, and the only trait object anywhere is the `BatchReader` this project
  already boxes.
- **A schema disagreement is a typed error naming the differing column**, raised
  at bind time for a reader (before the first batch) and at apply time for a
  loose batch, never a silent mismatch or a null column.
- **`IOBase` gets no `apply` method.** The record surface stays exactly the
  three methods it has; an expression reaches a handle through `RecordOptions`
  (§3.1) and nowhere else. `apply_*` operates on what a caller already holds.

---

## 2. Order of work

`AGENTS.md:9` — Rust first, fully. Each phase is complete work on its own.

- **Phase 1 — the module.** `Expr`, parser (encapsulators and accessors
  included), `Bound`, row evaluation, statistics evaluation, `Selection`, Arrow
  evaluation, the `Apply`/`ArrowApply` surface, edge-case tests, benchmarks,
  `docs/expressions.md` with runnable Rust examples (Python/JS tabs marked
  `!!! note "Rust first"` until Phase 4/5 land).
- **Phase 2 — the record surface** (`generic/options.rs`, `io/partition.rs`):
  options take an expression; the pair vocabulary becomes sugar; the folder row
  filter and directory pruning run on the engine.
- **Phase 3 — Iceberg**: `scan.rs` prunes manifests, files, and partition tuples
  through `evaluate_stats`; `Filter` is deleted; residuals come from `residual`.
- **Phase 4 — Python binding.** **Phase 5 — JavaScript binding.**
- **Phase 6 — docs, notebooks, benchmark tables, interop check.**
- **Phase 7 — required checks.**

Commit at each phase boundary (and inside Phase 1 per file group) with
descriptive messages.

---

## 3. Integration — the reason this exists

### 3.1 Record options (`rust/src/generic/options.rs`)

Add to `IORecordOptions`, beside the existing settings, and to
`record_options_fields!` so every encoding gets them mechanically:

- `filter(&self) -> Option<&Expr>` / `set_filter(&mut self, filter: Expr)` /
  `with_filter(self, filter: impl TryInto<Expr>) -> Result<Self>` — a `&str`
  is accepted and parsed by the core, so a caller writes
  `.with_filter("venue = 'XNAS' AND price > 10")?`.
- `selection(&self) -> Option<&Selection>` / `set_selection` / `with_selection`.

Keep `filter_partitions`, `with_filter_partitions`, `select_by_names`, and
`with_select_by_names` **exactly as they are spelled** — they are published API
in both bindings and the docs — and reimplement them as constructors over the
new value: `with_filter_partitions([("venue", "XNAS")])` builds
`venue = 'XNAS'` (and `venue IS NULL` for the text `null`, which is what
`NULL_PARTITION` means today), conjoined with any existing filter;
`filter_partitions()` continues to answer the pairs the caller set. Two settings
that mean the same thing must not be able to disagree: store the expression,
derive the pairs, and pin that with a test.

**Preserve today's tolerance exactly.** `filter_partitions` on a *folder* route
tolerates a column the rows do not carry (`io/partition.rs:396` skips it) and a
value the type cannot read (`scan.rs:144` makes it match nothing); on a *table*
route an undeclared column is an error (`AGENTS.md:245`). Those three behaviors
are the pair sugar's, not the expression's: `Expr::bind` is strict and says so,
and the sugar's constructor selects the tolerant binding mode. Both modes get
tests naming which surface they belong to.

### 3.2 Folders (`rust/src/io/partition.rs`)

- `filter_rows` (line 385) is deleted; `filtered_reader` (347) binds the
  options' filter against the reader's schema **once** and calls
  `Bound::filter_batch` per batch. The `ArrayFormatter`-per-row-per-column loop
  goes away; the benchmark in §5 reports what that was worth.
- Directory pruning: a `column=value` directory becomes a `StatsSource` whose
  single column has `lower == upper == partition_text`'s value, so
  `evaluate_stats` returning `AlwaysFalse` is what skips a subtree — and the
  subtree is skipped **before** it is listed, which is what `children_where`
  already promises. `children_where` itself keeps its `(&str, &str)` signature
  and gains a sibling `children_matching(&Bound)`; the pair form calls the
  expression form.
- `partition_text` stays the one renderer (`AGENTS.md`'s "the manifest is the
  authority on a partition value; the path is layout" is unchanged): text
  comparison is what an *identity* partition directory answers, typed comparison
  is what a manifest tuple answers, and the engine is told which by the
  `StatsSource` it is handed — never by a branch inside `expressions/`.

### 3.3 Iceberg (`rust/src/iceberg/scan.rs`, `table.rs`)

- Delete `Filter` (`scan.rs:99`) and its bespoke comparisons. `manifest_matches`
  (199) evaluates the bound filter against a `StatsSource` over the manifest
  list row's `FieldSummary`s; `file_matches` (251) evaluates it against one over
  the data file's `lower_bounds`/`upper_bounds`/`null_counts`/`value_counts`,
  decoded through the existing `iceberg::value::single_value`/`compare_single`
  (that module stays the authority on Iceberg's single-value encoding — the
  expression engine never learns it); `ScanTask::residual` becomes the residual
  `Bound` the plan carries forward.
- `Table::plan(&[(&str, &str)])`, `plan_at`, `scan`, `read`, `delete` keep their
  signatures and gain expression siblings following the vocabulary:
  `plan_where(&Bound)` / `scan_where` / `read_where`, with the pair forms
  building the expression. Same for the `iceberg::mod.rs:146` filter plumbing.
- **Transform projection — the one genuinely new pruning rule.** A predicate on
  a *source* column can prune a partition field the spec produced by a
  transform, by projecting the predicate onto the transform's value space:
  `Transform::project(&Bound, source: &Field, partition: &Field) ->
  Option<Bound>` in `rust/src/iceberg/partition.rs` (it belongs to the format,
  not to `expressions/`). It must be **inclusive** — it may only ever widen, so
  a file that could match is never pruned:
  - `Identity` — the predicate itself;
  - `Void` — `None` (nothing to say);
  - `Year`/`Month`/`Day`/`Hour` — a range predicate on a date/timestamp column
    projects to the same comparison on the transformed value, computed by
    applying the transform to the literal bound (with the inclusive rounding
    each direction needs — a test per direction per unit);
  - `Truncate(w)` — comparisons project by truncating the literal, `Eq` projects
    to `Eq` on the truncated value, `StartsWith` projects when the prefix is at
    least `w` long;
  - `Bucket(n)` — `None`. The hash is Iceberg's Murmur3 and this repository does
    not implement it (`partition.rs:87` says exactly that about writes). Refuse
    it by name in the same voice, do not emulate it, and note in the docs that a
    bucket partition still prunes through the *stored* partition value when a
    caller filters on the partition column itself.
- Everything the scan already reports stays reportable: `ScanPlan` must still
  say how many manifests and files the metadata let it skip, and the pruning
  numbers in `docs/iceberg.md` must be re-verified, not assumed.

---

## 4. Tests

`rust/src/expressions/tests.rs` (module edge cases) plus dispatcher
`rust/tests/expressions.rs` over `rust/tests/expressions/{parser, eval, stats,
select, arrow}.rs`, mirroring how `rust/tests/field.rs` dispatches
(`AGENTS.md:366`).

- **Parser**: every operator and precedence pairing; `Display`↔`FromStr` round
  trips including nested `CAST` to a nested datatype; typed temporal literals in
  every unit and zone; decimal literals keeping their written scale; adversarial
  refusals with the right byte position (unbalanced parens, trailing token,
  `IN ()`, missing `AND` in `BETWEEN`, unknown function, bad datatype,
  over-limit nesting).
- **Encapsulators** (§1.3.1): a name with spaces, a dot, a `(`, an operator, a
  reserved word, a leading digit, and non-ASCII text, each through all three
  encapsulators and each round-tripping to the one canonical double-quoted
  spelling; doubled closers (`""`, ` `` `, `]]`) embedding the delimiter;
  `"  a  "` keeping its padding byte for byte; `"x" = 'x'` parsing as
  column-versus-string while `'x' = 'x'` folds to `TRUE`; an unterminated
  delimiter of each kind reporting the **opener's** byte offset; a `.` inside a
  quoted segment staying part of the name; comments skipped outside and inert
  inside a quoted name; a case-insensitive fold that hits two columns refused as
  ambiguous naming both; the §3.1 pair sugar round-tripping a column named
  `total amount`.
- **Accessors** (§1.3.2): struct child, map key, list index, negative index, and
  every range spelling, on every container type in the table, including chains
  (`a.b[0]['k'][1:3]`); 0-based and half-open semantics asserted explicitly;
  out-of-range index yielding null and out-of-range range clamping; an inverted
  range yielding empty; a text range never splitting a multi-byte character
  while a binary range slices bytes; a double-quoted subscript read as a key,
  not an identifier; every bind-time refusal (`a[0]` on a struct, `a.b` on a
  scalar, a key the map's key type cannot hold) naming datatype, accessor, and
  path; accessor results identical between the row path and the Arrow path.
- **Bind**: name resolution case-insensitively; unknown column names the columns
  that exist; literal folded once into the column type (assert the bound literal
  is the column's datatype); incompatible comparison refused naming both types;
  strict mode versus the tolerant pair mode; schema mismatch at evaluation time.
- **Simplify**: each rewrite, plus a property-style check that simplification
  never changes an answer over a table of rows including nulls.
- **Row evaluation**: the full three-valued truth tables for `AND`/`OR`/`NOT`;
  null comparisons; every scalar type including decimals across scales,
  temporals across units and zones, and bytes; nested column paths; `CASE`;
  arithmetic overflow refused rather than wrapped.
- **Statistics**: every rule with exact bounds, coarse bounds, missing bounds,
  all-null and no-null columns; **the safety property** — for a set of rows and
  their true statistics, `evaluate_stats` never answers `AlwaysFalse` when any
  row matches, and never `AlwaysTrue` when any row does not; residual conjunct
  computation.
- **Arrow parity**: the *same* `Bound` over the *same* data must produce the
  same selection through `matches` (row) and `mask` (vectorized) — a table-driven
  test across every supported type and operator. This is the single most
  valuable test in the change; write it first.
- **Selection**: alias defaulting; bare-column selection produces byte-identical
  results and the identical root `Field` to today's `select_by_names`; computed
  columns after projection pushdown; a name the rows lack is an error, as today.
- **Apply surface** (§1.9): the same expression through `apply_value`,
  `apply_arrow_batch`, and `apply_arrow_batch_reader` selects the same rows and
  produces the same columns; `apply_field` equals the schema of the batch
  `apply_arrow_batch` produces, with no data allocated and no file opened; a
  `&str` subject, an `Expr` subject, a `Bound` subject, and a pair-slice subject
  give identical answers; the streamed reader answers `schema()` before its
  first batch and holds at most one batch; a batch whose schema disagrees errors
  naming the column, and a reader whose schema disagrees errors before the first
  batch; every carrier in the redirection table is exercised at least once.
- **Integration**: `rust/src/io/tests.rs` — a folder-partitioned lake filtered
  by an expression prunes directories (assert nothing under the excluded prefix
  is listed, with a counting handle) and filters rows; `rust/src/iceberg/tests.rs`
  — manifests, files, and rows pruned by an expression, the counts asserted, the
  pair-form and expression-form answers identical on the same table, transform
  projection pruning a `day`-partitioned table on a timestamp predicate, a
  bucket-partitioned table pruning nothing but answering correctly.
- No feature-gated behavior difference: the whole module minus `arrow.rs` is
  tested under `--no-default-features --lib`.

---

## 5. Benchmarks and the outside baseline

`rust/benchmarks/expressions.rs` with the dispatcher pattern
(`#[path = "expressions/mod.rs"] mod benchmarks;`, stable criterion group IDs),
over `rust/benchmarks/expressions/{parse, bind, eval, prune}.rs`:

- cold `from_str` of a small, a medium, and a deeply nested predicate;
- `bind` + `simplify` cost, so the "bind once" claim is a number;
- row evaluation per million rows versus a hand-written Rust closure — the
  honest upper bound;
- vectorized `mask`+`filter_batch` versus (a) the hand-written `arrow_ord`
  comparison it compiles to and (b) **the current `filter_rows` string
  comparison it replaces**, on the same batches. That second baseline is the
  claim this change makes; report it.
- statistics pruning: files skipped per second over a synthetic manifest;
- the encapsulated-name and accessor-chain legs: parsing and evaluating
  `"total amount"`, `a.b[0]`, and `a[1:3]` beside their unencapsulated,
  unaccessored equivalents, so the grammar's convenience carries a known cost;
- the apply surface: `apply_arrow_batch` from a `&str` subject (parse + bind per
  call) versus a hoisted `Bound` subject, which is what the docs tell readers to
  do and must therefore be a number rather than advice.

Extend `rust/benchmarks/iceberg.rs` with a filtered-plan leg so the pruning
change is visible where it matters.

**Outside implementation** (`AGENTS.md:339`): `scripts/check_expression_interop.py`
plus `rust/tests/expression_interop.rs`, following
`scripts/check_iceberg_interop.py` exactly, including the rule that the Rust
half prints `SKIPPED` when the external side is absent and the driver fails on
that word — a skipped half can never read as a pass. Check both:

- **row semantics** against `pyarrow.compute` / `pyarrow.dataset` — the same
  predicate text over the same table must select the same rows, nulls included;
- **pruning semantics** against PyIceberg — `pyiceberg.expressions.parser.parse`
  on the same text, and its inclusive metrics evaluator on the same table, must
  agree with `evaluate_stats` about which files are skipped. Disagreement in the
  conservative direction (we read a file PyIceberg skipped) is reported, not
  failed; disagreement in the other direction is a hard failure.

---

## 6. Bindings

**Python** — `python/src/expression.rs` (mirroring core domains,
`AGENTS.md:96`), exported as `yggdryl.Expr` and `yggdryl.Selection`:

- `Expr.from_str(text)`, `Expr.column(name)`, `Expr.literal(value)`; a plain
  `str` is accepted wherever an `Expr` is expected and redirects immediately to
  the core parser (`AGENTS.md:843`, infer at the boundary, compute in Rust).
- Idiomatic operators building expressions without evaluating anything:
  `__eq__`/`__ne__`/`__lt__`/`__le__`/`__gt__`/`__ge__`, `&`, `|`, `~`,
  `is_null()`, `isin([...])`, `like(...)`, `cast(dtype)`, `alias(name)`.
  `__eq__` returning an `Expr` means `__hash__` must be defined explicitly (the
  canonical text's stable hash) and the docs must say the class is not a value
  you put in a set expecting equality semantics.
- `__str__`/`__repr__`/pickle/JSON per house style; `equals`, `show_diff`.
- `RecordOptions` gains `filter` and `selection` properties plus
  `with_filter` / `with_selection`, accepting `str` or `Expr`;
  `filter_partitions` and `select_by_names` keep working unchanged.
- **`Expr.apply(target)` and `Selection.apply(target)`** — the §1.9 redirection
  at the boundary: a `pyarrow.RecordBatch`, `Table`, `RecordBatchReader`, or
  `Array` (with a `Field`), a `yggdryl.Field` (schema only), a `dict` or
  `Record` row. The dispatch happens in Rust after one coercion at the boundary
  (`AGENTS.md:843`); Python never inspects the expression to choose a path.
  `Expr.bind(field)` exposes the hoistable bound form, and `Expr.field(schema)`
  the result schema.
- `Table.plan/scan/read`, `IOBase.children_where`, `IOBase.glob` accept an
  expression wherever they accept the pairs today.
- `python/yggdryl/_native.pyi` and `__init__.pyi` updated; `mypy --strict`
  green; tests in `python/tests/test_expressions.py` in house style (fixtures,
  plain-English test classes with docstrings), covering parse, operators,
  filtering a Buffer, filtering a partitioned folder, filtering an Iceberg
  table, error surfacing (the native message unchanged), and the PyArrow
  agreement check;
  benchmark `python/benchmarks/expressions.py` against `pyarrow.compute` on the
  same payload, release build only.

**JavaScript** — `node/src/expression.rs`, camelCase boundary:
`Expr.from(text)`, `Expr.column`, `Expr.literal`, and method combinators
(`eq`, `lt`, `and`, `or`, `not`, `isNull`, `isIn`, `like`, `cast`, `alias`)
since JS has no operator overloading; `toString`, `toJSON`, `equals`,
`stableHash`. `RecordOptions.withFilter` / `withSelection` accept a string or an
`Expr`; `expr.apply(target)` redirects over a `BatchReader`, an Arrow JS
`Table`/`RecordBatch`, IPC bytes, a native `Field`, or a row object, and
`expr.bind(field)` / `expr.field(schema)` expose the bound form and the result
schema; the `iceberg` namespace keeps its shape (`AGENTS.md:1092`) and its table
methods take the same argument in the same position as Python. Tests
`node/tests/expressions.test.js` + `expressions.types.ts`; benchmark
`node/benchmarks/expressions.js` wired as `npm run bench:expressions`.

Update `.api-bindings.txt` / `.api-inventory.txt` by their generator, never by
hand.

---

## 7. Documentation

- New page `docs/expressions.md`: one H1, exactly one opening sentence, then
  example-first sections — write a predicate; parse one from SQL text; bind it to
  a schema and see the folded literal; filter rows; filter a batch; select and
  compute columns; prune a partitioned folder; prune an Iceberg table; the
  three-valued null rules stated plainly in a short table; **naming things that
  need quoting** (the three encapsulators in, one canonical spelling out, and
  the whitespace-is-data rule); **reaching inside a value** (child, key, index,
  range — with the 0-based and half-open conventions stated once, loudly, beside
  a `BETWEEN` counter-example); **applying an expression to what you already
  hold** (the §1.9 carrier table as runnable examples: a row, a batch, a
  reader, a field); what the grammar accepts (a compact operator/precedence
  table and the literal forms); what it deliberately does not (no subqueries, no
  joins, no aggregates, no `bucket` — each with its one-line reason).
- Every example in **Rust → Python → JavaScript tabs, in that order**, each
  idiomatic and self-contained with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. Check `.api-bindings.txt` before
  showing a language do anything.
- Add the page to `mkdocs.yml` nav beside `field`/`datatype` (it is a core
  value, not a storage concern); update `docs/io.md` §"Partition pruning and
  filtering" (line 2134) and `docs/iceberg.md` (936–943, 1253–1318) so the "a
  filter is a column name and a value as text" sentences become "a filter is an
  expression; the pair form is sugar for one equality"; update
  `docs/architecture.md`, the README layout table, and
  `docs/extensions/{python,javascript}.md` for their boundary only.
- Regenerate notebooks with `python scripts/build_docs_notebooks.py` (edit
  blocks, never notebooks); regenerate `docs/benchmarks.md` tables, naming
  machine, interpreter, and build profile.
- Update `AGENTS.md` itself: the *Source layout and scope* bullet list gains
  `rust/src/expressions/`, and the *Table format contract* sentence at line 237
  ("A filter is a `(column, value)` text pair") is rewritten to describe the
  expression and its sugar. A contract that no longer matches the code is worse
  than no contract.
- `python -m mkdocs build --strict` stays green.

---

## 8. Required checks (all must pass before handoff)

Per `AGENTS.md:1116`: `cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` **twice**
(default features and `--features "parquet iceberg"`); workspace tests twice the
same way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; the Rust 1.85 core
check (default features and `--no-default-features --lib` — the whole expression
value, parser, binder, row evaluator and statistics evaluator must compile and
be tested without `arrow`); `cargo bench --benches --no-run`; `maturin develop` +
`pytest` + `mypy --strict`; `npm run test:package` + `npm test`;
`python scripts/check_docs_examples.py`; `python scripts/check_expression_interop.py`;
`python scripts/check_iceberg_interop.py` (unchanged answers);
`python -m mkdocs build --strict`. Clean generated targets, `site/`, venvs,
native binaries, caches, and `node_modules` after validation.

---

## 9. Hard constraints, restated

- **One engine.** Row, vectorized, and statistics evaluation read the same
  `Bound`. No second comparison implementation survives this change:
  `iceberg::scan::Filter` and `io::partition::filter_rows` are deleted, not
  wrapped.
- **No new dependency** in any of the three manifests; no parser generator, no
  date crate, no expression crate. The type grammar is `DataType::from_str`, the
  calendar is `generic/iso.rs`, the casts are `field::cast`, the Iceberg
  single-value encoding stays in `iceberg::value`.
- **The record surface stays exactly three methods.** `apply_*` is a surface
  over values, batches, and readers a caller already holds; `IOBase` gains
  nothing, and an expression reaches a handle only through `RecordOptions`.
- **One lexer, one accessor resolver.** Encapsulator stripping and accessor
  resolution exist once, in `expressions/parser.rs` and `expressions/bound.rs`;
  no call site, sugar constructor, or binding splits a dotted name, trims a
  quote, or parses a subscript on its own.
- **The core module knows nothing about storage or table formats.**
  `expressions/` may not mention Iceberg, partitions, manifests, or `IOBase`;
  those modules implement `StatsSource` and call in. Dependencies point one way.
- **Published spellings do not change.** `filter_partitions`,
  `select_by_names`, `children_where`, `Table::plan`, and every binding name in
  `.api-bindings.txt` keep working with identical answers; new capability
  arrives as new names following the vocabulary (`AGENTS.md:397`), never as an
  alias with a different verb.
- **Pruning may never lose a row.** `Maybe` is the safe answer everywhere; every
  `AlwaysFalse` rule has a test with adversarially coarse statistics; transform
  projection is inclusive by construction.
- **Three-valued logic is the semantics**, stated in the module docs, identical
  in all three evaluators, and pinned by the parity test.
- Errors are typed, located (byte position for the parser, dotted path for
  binding and evaluation), `expected X, got Y`, values quoted with `{value:?}`
  and truncated through the shared limits; no panic, unwrap, or unsafe on
  caller-controlled input; bindings surface the native message unchanged.
- Allocation discipline: bind once, never per batch or per row; shared nesting
  through `Arc`; empty collections without backing allocation; no per-record
  maps or schemas.
- Anything held in memory carries a comment saying why.

**Definition of done**: a caller writes
`options.with_filter(r#""trading venue" = 'XNAS' AND payload['ts'] >= TIMESTAMP '2024-01-01T00:00:00Z'"#)?`
and the same one sentence skips Iceberg manifests, skips data files, skips
`trading venue=XNYS/` directories without listing them, and filters the rows
that survive; the same expression hands to `apply_arrow_batch` a batch already
in hand, to `apply_arrow_batch_reader` a stream, and to `apply_field` the
schema its result would have without opening anything — in Rust, Python, and
JavaScript, with one implementation of the comparison behind all of it.
