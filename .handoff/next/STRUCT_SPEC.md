# Spec: struct conversions, part 2

Read `AGENTS.md` first; this spec only applies it. Part 1 (the struct pair on
`DataType`, `Field` and `Scalar`, Rust only) is in `main`. Part 2 is the rest:
the pair on `Serie` and `ChunkedSerie`, the reroute of the record-root code in
`serie/arrow.rs` and `excel/sheet.rs` onto it, both bindings, the docs and the
skills. One branch, one commit, one PR, squash-merged once CI is read and green;
that change deletes this file.

**Anchors.** Lines are taken at `091f190`, the struct-pair commit of the
part-1 branch (`claude/iceberg-partition-expressions-oc6r3k`); the hand-off and
docs commits after it change no `rust/src` file. The part-2 files
(`serie/arrow.rs`, `serie/structure.rs`, `serie.rs`, `chunked_serie.rs`,
`excel/sheet.rs`, the binding sources) were not touched by part 1, so their
lines hold in `main`. The part-1 files `structure.rs` and `media/inference.rs`
were gaining rustdoc examples when this was written, so their lines drift:
re-find every anchor by symbol (`grep -n 'fn <name>'`) before scripting an edit,
and never edit by line number.

## 0. Base and what is done

| Landed in part 1 | Where | Pinned by |
| --- | --- | --- |
| `DataType::is_struct(&self) -> bool` - the Struct shape alone | `rust/src/structure.rs`, `impl DataType` beside `as_fields` (≈667) | `rust/tests/root/structure.rs` `struct_pair::is_struct_names_the_struct_shape_alone` |
| `DataType::into_struct_type(&self) -> Result<DataType>` - itself when a struct (a clone sharing its children), else `struct<value: self>` with `value` nullable; refused past the bound naming `$` | `structure.rs` (≈680) | `a_datatype_wraps_as_a_nullable_value_and_a_struct_answers_itself`; refusals `a_wrap_past_the_recursion_limit_is_refused_naming_the_root_and_the_limit`, `a_wrap_over_a_shared_subtree_is_refused_within_the_node_budget` |
| `Field::is_struct` now reads `self.dtype().is_struct()` | `structure.rs`, `impl Field` (≈766) | the existing `Field` suites |
| `Field::into_struct_field(&self) -> Result<Field>` - itself when a struct (name, nullability, metadata kept, so a nullable struct stays nullable), else `DataType::from(StructType::from_fields([self.clone()])?).required_field(DEFAULT_ROOT_NAME)`, which is today's `serie/arrow.rs` `record_root` body exactly; refused past the bound naming the field | `structure.rs` (≈781) | `a_field_wraps_as_the_one_child_of_a_required_row` and the two refusal rows |
| `Scalar::into_struct_scalar(&self) -> Scalar` - a `Scalar::Struct` as is, else the one-entry `Struct({DEFAULT_VALUE_NAME: self})`; infallible, a null becoming `{value: null}` | `structure.rs`, an `impl Scalar` block after `impl fmt::Display for Struct` (≈2082) | `a_scalar_wraps_under_value_and_canonicalizes_under_the_wrapped_type` |
| `Scalar::inferred_record_field(&self) -> Result<Field>` - a sequence value (`as_serie().is_some()`) is `inferred_struct_field()` with its refusals; any other value is `inferred_scalar_field()?.into_struct_field()` renamed `row`: a single named record is its own required struct named `row`, a leaf is `row: struct<value: leaf>` (`value` nullable only for a null) | `rust/src/media/inference.rs`, after `inferred_struct_field` (≈154) | `rust/tests/media/inference.rs` `records::` - `rows_that_name_no_columns_refuse_as_the_struct_inference_does`, `named_rows_infer_the_row_root_the_struct_inference_answers`, `one_named_record_is_its_own_required_row`, `a_leaf_is_the_value_of_a_required_row` |
| `media::DEFAULT_VALUE_NAME: &str = "value"` beside `DEFAULT_ROOT_NAME` | `rust/src/media/mod.rs:59` (`DEFAULT_ROOT_NAME` :57) | read by every site below |
| The allocation-free depth probe: `pub(crate) enum NestingRefusal { Depth, Nodes }` with `text()` (each refusal sentence spelled once) and `refuse(kind, path)`; `pub(crate) fn nesting_exceeds(dtype, depth, visited: &mut usize) -> Option<NestingRefusal>`, recursive, the children enumerated exactly as `preflight_schema_shape` enumerates them, the same `MAX_DEFAULT_NODES` (1,000,000) budget - which is what bounds a subtree shared through an `Arc` - nothing allocated, a wrap starting it at `(1, &mut 1)`; `preflight_schema_shape` and `reserve_pending` read `NestingRefusal` for both sentences | `rust/src/default.rs:152-177` (`NestingRefusal`), `:190` (`nesting_exceeds`), `:283` (`preflight_schema_shape`), `:12` (`MAX_DEFAULT_NODES`) | the two refusal rows above; `cargo test -p yggdryl --test root default` (the preflight's two sentences unchanged) |
| The `rust/src` literal sweep: `typed.rs` `SHARED_NAME` deleted, the shared fields named `crate::media::DEFAULT_VALUE_NAME` (:347, :448); `media/inference.rs` `inferred_scalar_field` (`value`) and `inferred_struct_field` (`row`); `datatype.rs:1669`; `arrow/mod.rs:588,590`; `avro/arrow.rs:40`; `parquet/mod.rs:611`; `iceberg/metadata.rs:1122,1127` | as listed | the suites of those files |
| The Avro root is **not** rerouted: only its `"value"` literal moved. An internals forward `avro_arrow::field_from_schema` reaches the crate-private reader | `rust/src/avro/arrow.rs:29-46`, its `pub mod internals` | `rust/tests/avro/arrow.rs` `mod internal` `a_root_union_reads_as_the_value_of_a_required_row` (a `["null", record]` root and a `[record]` root are the `value` child of a required `row`, nullable and required respectively) |
| Benchmarks: the `typed/struct` group's id `into_struct_field` renamed `from_struct_field` (it times `Field::from(StructField)`), and rows `is_struct`, `into_struct_type`, `into_struct_field` (a leaf field wrapped), `into_struct_scalar` | `rust/benchmarks/types/field/integer.rs:29-57` | `cargo bench -p yggdryl --bench types -- typed/struct --quick` |
| `.api-inventory.txt`: `inferred_record_field` (section `media::inference`), `DEFAULT_VALUE_NAME` (section `media`), the four struct-pair lines (the `structure` section) | `.api-inventory.txt` | `python scripts/check_api_inventory.py` |
| Docs and skill: an "As a struct" section on `docs/types/datatype.md`, `field.md` and `scalar.md`, "Inferred fields" on `scalar.md`, an "As a struct" door table on `docs/types/nested/struct.md`, the edges and commands rows, the `yggdryl-types` skill rows, pitfall and Rust recipe, the `into_struct_<root>` row of AGENTS.md's Public vocabulary - every binding tab reading "Rust only" | §7 | `mkdocs build --strict`, `check_docs_examples.py --lang rust` |

**Not done, and part 2's:** no binding reaches any of it; `Serie` and
`ChunkedSerie` have no `into_struct_serie`; `serie/arrow.rs` `record_root`,
`SerieReader::root_of`, `held_record` and `batch_under` are not rerouted;
`excel/sheet.rs` `record_columns` stands; one `rust/src` value-name literal
was left because its file is part 2's (`serie/arrow.rs:1119`,
`default_dtype_array`); the Python and Node `"row"` and `"value"` literals stand.

## 1. Semantics

**The pair, one spelling per root (decided by the user).** `into_struct_<root>`
answers the value itself when it already is a struct - a clone, which bumps
pointers - and otherwise the one-child `struct<self>`:

| Root | Already a struct | Otherwise | The borrowed "is it" (unchanged) |
| --- | --- | --- | --- |
| `DataType` (done) | itself | `struct<value: self>`, `value` nullable, bounded, refusal naming `$` | `as_fields()`, `is_struct()` |
| `Field` (done) | itself: name, nullability, metadata kept (a dictionary id exists only on `Field::Dictionary`, inside the variant) | a required `row` (`DEFAULT_ROOT_NAME`), no metadata, one child = `self` unchanged, bounded, refusal naming the field | `StructField::from_field`, `is_struct` |
| `Scalar` (done) | a `Scalar::Struct` itself | `Scalar::Struct({value: self})`, the named input shape `Field::scalar` canonicalizes under the wrapped field; infallible | `Scalar::as_struct` |
| `Serie` (part 2) | the Struct leaf, absent rows kept | `StructSerie::wrap(Arc::new(field.into_struct_field()?), self.clone())`: one child, no row validity, nothing copied or read; a run refused | `Serie::as_struct` |
| `ChunkedSerie` (part 2) | itself | every chunk wrapped under one shared root `Arc` | per chunk `as_struct` |

- **No `as_struct_*` conversion, anywhere (decided by the user):** `as_<noun>`
  stays reserved for dedicated borrowed views. The suffix names the root being
  converted, which keeps `into_struct_serie` apart from the leaf narrowing
  `Serie::as_struct`; the docs put both in one table.
- "Already a struct" is the Struct shape at any nullability, with any absent
  rows, answered losslessly. A wrap never adds an absent row, so a wrapper is
  always required.
- **The bound.** A wrap is refused when the wrapped root would nest past
  `DataType::PARSE_RECURSION_LIMIT` (64) or past the node budget, by the probe;
  the struct arm checks nothing. A field whose own `validate_bounded` passes at
  depth 63 is refused once wrapped.
- **A run is refused** by `Serie::require_field`'s refusal,
  `invalid record value at $: a schema-free run declares no field`.
- **The table rule stays separate.** `SerieReader::root_of` keeps forcing a
  required root (`with_nullable(false)`), and the held doors keep refusing a
  record column holding an absent row: a struct with absent rows has two
  readings as a table (its children as columns, or itself as one struct
  column), so the refusal stands, its text unchanged and carrying no remedy
  clause.
- **The bindings' inference door keeps its name (decided by the user).**
  Python `Scalar.into_struct_field()` and JavaScript `Scalar.intoStructField()`
  read both shapes through the one core reading, `Scalar::inferred_record_field`:
  rows of named records answer the inferred struct root exactly as today
  (refusals included: `[]`, `[[1]]`); a single named record answers its own
  required struct named `row`; any other value answers `row: struct<value: ..>`.
  `into_array_field` / `intoArrayField` and `into_field` / `intoField` are
  unchanged, and so is the JavaScript record-class static getter
  `intoStructField`, which is a different thing (a class's declared root).
- **What is not widened:** `Scalar::inferred_struct_field` and its other
  callers (`http/pages.rs:155`, `media/structured.rs:59`, Python
  `Scalar.into_arrow_batch` at `python/src/scalar.rs:1164`) stay rows-only.

## 2. Remaining core

Files per AGENTS' Layout: a `Serie` verb in `serie.rs`, a `ChunkedSerie` verb in
`chunked_serie.rs`, the record leaf's constructor in `serie/structure.rs`.

```rust
// rust/src/serie/structure.rs, after `new` (:51-62); crate-private
impl StructSerie {
    pub(crate) fn wrap(root: Arc<Field>, child: Serie) -> Self {
        let rows = child.len();
        Self::new(root, vec![child], None, rows)
    }
}

// rust/src/serie.rs, after `as_struct` (:3489-3492)
impl Serie {
    /// This column as a struct column: itself when it is one, absent rows
    /// kept, else the one child of a required `row` record - nothing copied
    /// or read.
    pub fn into_struct_serie(&self) -> crate::Result<Serie> {
        let field = self.require_field()?;                 // a run refused
        if self.as_struct().is_some() {
            return Ok(self.clone());
        }
        Ok(crate::SerieValue::into_serie(StructSerie::wrap(
            Arc::new(field.into_struct_field()?),
            self.clone(),
        )))
    }
}

// rust/src/chunked_serie.rs, after `into_serie` (:611)
impl ChunkedSerie {
    /// Every chunk as a struct column under one shared root: itself when
    /// its field is a struct, else each chunk the one child of the root
    /// `Field::into_struct_field` answers, built once.
    pub fn into_struct_serie(&self) -> crate::Result<Self> {
        if self.field.is_struct() {
            return Ok(self.clone());
        }
        let root = Arc::new(self.field.into_struct_field()?);
        let chunks = self.chunks.iter()
            .map(|chunk| crate::SerieValue::into_serie(
                StructSerie::wrap(Arc::clone(&root), chunk.clone())))
            .collect();
        Ok(Self::from_landed(root, chunks))               // :104; ends recomputed, same values
    }
}
```

`chunked_serie.rs` names the leaf as `crate::StructSerie` (the root re-exports
`serie::*`, `lib.rs:301`; `serie.rs:255` re-exports the leaf) or adds it to its
`use crate::serie::{..}` list (:70).
`from_landed` is the crate's door for chunks already landed under their field,
which a wrap's chunks are: every chunk's `field_ref()` is `Arc::ptr_eq` with
the root.

**The reroute, `rust/src/serie/arrow.rs`** - each exactly equivalent, the
replaced code deleted:

1. `record_root` (:1124-1131) is deleted; it is the wrap arm of
   `Field::into_struct_field`.
2. `SerieReader::root_of` (:1715-1721) becomes
   `Ok(field.into_struct_field()?.with_nullable(false))`. Its outputs are
   identical; its wrap arm now runs the probe. Every caller inherits it:
   `batch_schema` (:1137), `SerieReader::cast` (:1650), `serie/order.rs:1404`
   (`window_by`'s key bind), `chunked_serie.rs:1118` (`window_by`), and the
   bindings' `python/src/serie.rs:620` (`batch_root`),
   `python/src/chunked_serie.rs:680`, `node/src/serie.rs:700`,
   `node/src/chunked_serie.rs:382`. Its rustdoc keeps naming the rule and adds
   that it is `Field::into_struct_field` forced required.
3. `held_root` (:1517-1521) is **unchanged**: it keeps `root.validate_bounded()?`
   after `root_of`. The struct arm of `into_struct_field` checks nothing and the
   probe allocates nothing, so `root_of`'s wrap arm costs exactly what
   `record_root` cost (`WINDOW_BY_VALUES_KEY`, `CHUNKED_WINDOW_BY_BIND`) and the
   record arm keeps its one bounded walk in `held_root` (`held == [7, 9]` counts
   that walk's pending vector). Running `validate_bounded` inside the conversion
   instead would move all three pins (§5).
4. `held_record` (:1525-1543): the non-record arm becomes
   `structure::StructSerie::wrap(Arc::clone(root), serie)`; the record arm
   stays, its refusal through the one constructor below.
5. `batch_under` (:1147): `if field.dtype().as_fields().is_none()` becomes
   `if !field.is_struct()`; its refusal through the one constructor below.
6. The absent-row refusal is written once: one crate-private
   `fn absent_rows_refusal(name: &str, absent: usize) -> Error` in this file,
   writing exactly today's
   `record column {name:?} holds {absent} absent rows, which a table cannot state`
   as `Error::IncompatibleSchema`, called from `batch_under` (:1156) and
   `held_record` (:1532).
7. `default_dtype_array` (:1119): `Field::new("value", ..)` reads
   `crate::media::DEFAULT_VALUE_NAME` (still required).

**`rust/src/excel/sheet.rs`.** `record_columns` (:907-923) is deleted.
`write_serie` (:507-508) reads
`let root = crate::SerieReader::root_of(serie.require_field()?)?;` and
`let columns = serie.as_struct().map_or_else(|| vec![serie.clone()], |records| records.children().to_vec());`.
The record arm keeps not refusing absent rows (a sheet writes cells, not a
batch). `crate::Error` has no `Internal` variant, so no unreachable arm is
spelled. `empty_root` (:926-932) stays, and with it the `StructType` import.

**`rust/src/text/typed.rs:161`** (`prepare_dtype`): the bare datatype's field
`Field::new("value", dtype.clone(), true)` reads `DEFAULT_VALUE_NAME` - the
same fact as `default_dtype_array` (§9, decision 12).

**Behaviour that moves, on purpose, and is pinned (§4):**

- A held door (`SerieReader::from_serie`, `from_chunked`, `Serie::into_arrow_batch`,
  `into_arrow_reader`, `ChunkedSerie::into_arrow_reader`, `window_by`) over a
  non-record column whose field nests at the ceiling is refused by `root_of`
  naming the column (`{name}: schema nesting exceeds the hard limit of 64`),
  where today the wrapped root is refused naming `$` - by `held_root`'s
  `validate_bounded`, or by `arrow_schema_from_field`'s (`arrow/mod.rs:313`)
  at schema export. No test pins the old text.
- `Sheet::write_serie` of such a column is refused the same way, where
  `record_columns` built the root unbounded.

**Deliberately not rerouted** (the duplicate-logic review's, which excludes
only what this spec does):

- the Avro root (`avro/arrow.rs:29-46`): a root `["null", record]`, `[record]`
  or a `Ref` to a record answers a nullable struct, which `into_struct_field`
  would answer as itself - a nullable root the record surface refuses;
- the expression roots `Field::new(DEFAULT_ROOT_NAME, dtype, false)`
  (`expression/plan.rs:1047`, `selector.rs:784`, `filter.rs:191`,
  `records.rs:148`), which stay record-only, and `DataType::into_arrow_schema` /
  `Field::into_arrow_schema`, `arrow_schema_from_field` (`arrow/mod.rs:313-322`)
  and `ArrowCastPlan::compile_schema` (`cast.rs:599`), which keep refusing a
  non-struct;
- the multi-child roots (`serie/order.rs:1483` `record_of`,
  `xxhash/arrow.rs:505` `level_root`, `excel/sheet.rs:680`,
  `xmla/definitions.rs:140`, `xmla/rowset.rs:758`, `python/src/iceberg.rs`,
  `node/src/iceberg.rs:89`) and the zero-child `empty_root` builders
  (`excel/sheet.rs:926`, `excel/reader.rs:538`), which wrap nothing;
- the `as_fields().is_some()` / `is_none()` reads that could say `is_struct()`
  (`cast.rs:501,599`, `arrow/mod.rs:320`, `iceberg/statistics.rs:231`,
  `serie/arrow.rs:1362`, `python/src/serie.rs:629`,
  `python/src/chunked_serie.rs:699`), beyond the two this spec rewrites anyway
  (`batch_under`, `root_of`).

**Public names added** (`.api-inventory.txt`): `Serie::into_struct_serie`
(section `serie`, :5619), `ChunkedSerie::into_struct_serie` (section
`chunked_serie`, :5714); the `root_of` line (:5638) gains "Field::into_struct_field
forced required". Nothing for `pub(crate)` names. **Deleted:**
`serie::arrow::record_root`, `excel::sheet::record_columns`.

## 3. Bindings

Both bindings only coerce and redirect.

### Python (§3 of AGENTS)

| Class | Adds | Redirects to |
| --- | --- | --- |
| `DataType` (`python/src/datatype.rs`) | `is_struct` - a `#[getter]` property, as every `DataType` predicate there is and as `Field.is_struct` already is; `into_struct_type()` | `DataType::is_struct`, `into_struct_type` |
| `Field` (`python/src/field.rs`; `is_struct` exists, :705) | `into_struct_field()` | `Field::into_struct_field` |
| `Scalar` (`python/src/scalar.rs`) | `into_struct_scalar()`; `into_struct_field()` (:1117-1122) redirects to `inferred_record_field`, its docstring widened | `Scalar::into_struct_scalar`, `inferred_record_field` |
| `Serie` (`python/src/serie.rs`) | `into_struct_serie()`, answered through `described` so the leaf class (`StructSerie`) is kept | `Serie::into_struct_serie` |
| `ChunkedSerie` (`python/src/chunked_serie.rs`) | `into_struct_serie()` | `ChunkedSerie::into_struct_serie` |

- The wraps run on the GIL: a wrap reads no row and copies nothing (§9,
  decision 15).
- **Literals** read the core constants:
  - `"row"` → `yggdryl::media::DEFAULT_ROOT_NAME`: `python/src/serie.rs:615`
    (the `DEFAULT_ROOT` const is deleted; its uses :304, :308, :526, :585),
    `iomedia.rs:1188, 1304, 1334`, `scalar.rs:1165`, `field.rs:104`, the
    `field.rs:262` signature default (`name = yggdryl::media::DEFAULT_ROOT_NAME`;
    the stub keeps spelling `"row"`), `cast.rs:28`, `iobase.rs:2635`.
  - `"value"` → `yggdryl::media::DEFAULT_VALUE_NAME`: `datatype.rs:418, 459`,
    `serie.rs:267` (`target_of`), `serie.rs:751` (a pinned row named as a
    value). The `representation = "value"` signature defaults are a
    `Representation` spelling, not a name, and stay.
- **Stubs** `python/yggdryl/_native.pyi`: `Serie` (:445), `ChunkedSerie` (:862),
  `Scalar` (:1035; `into_struct_field` :1070), `DataType` (:1368, `is_struct`
  under `@property` beside `is_nested`), `Field` (:1924). `StructSerie` (:680)
  inherits `into_struct_serie`.
- `.api-bindings.txt` (Python): `DataType` (:25) `into_struct_type`,
  `is_struct`; `Field` (:32) `into_struct_field`; `Scalar` (:66)
  `into_struct_scalar`; `Serie` (:70) and `ChunkedSerie` (:74)
  `into_struct_serie`.

### Node (§4 of AGENTS)

| Class | Adds | Shape |
| --- | --- | --- |
| `DataType` (`node/src/datatype.rs`) | `isStruct` getter; `intoStructType()` | plain `#[napi]` (`#[napi(getter, js_name = "isStruct")]`) |
| `Field` (`node/src/field.rs`) | `isStruct` getter (parity with Python's `Field.is_struct`); `intoStructField()` | plain `#[napi]` |
| `Scalar` (`node/src/text/codec.rs`) | `intoStructScalar()`; the instance `intoStructField()` (:513-520) redirects to `inferred_record_field` | plain `#[napi]` |
| `Serie` (`node/src/serie.rs`) | `intoStructSerie()` | `#[napi(js_name = "_intoStructSerieNative", skip_typescript)]`, listed in the `answering` group of `node/binding.js` (:1504-1512) so `describedSerie` hands it out as its leaf's class, added to the delete list that follows it (≈1519-1572), declared in `node/binding.d.ts` beside `intoSorted` |
| `ChunkedSerie` (`node/src/chunked_serie.rs`) | `intoStructSerie()` | plain `#[napi]`, as `intoReversed` (:503) |

- **Literals:** `"row"` → `DEFAULT_ROOT_NAME` at `node/src/serie.rs:692`;
  `"value"` → `DEFAULT_VALUE_NAME` at `node/src/serie.rs:662, 1277`,
  `node/src/chunked_serie.rs:360` (a bare `DataType` as a cast target) and
  `node/src/datatype.rs:890`.
- **The record-class getter does not collide.** A native `Field` instance
  gaining an `intoStructField()` method is not a record class:
  `node/records.js` (`isStructRecord` :205-213, `structFieldGetter` :716-735,
  :951) reads the constructor's own properties, never the prototype's. Pinned
  (§4).
- **Regenerate** `node/index.js` / `index.d.ts` with
  `npm run --prefix node build:debug`, then the two docs manifests
  (`node scripts/build_docs_playground.js`, `node scripts/build_docs_fix.js`).
- `.api-bindings.txt` (JavaScript): `DataType` (:414) `isStruct`,
  `intoStructType`; `Field` (:454) `isStruct`, `intoStructField`; `Scalar`
  (:532) `intoStructScalar`; `Serie` (:536) and `ChunkedSerie` (:542)
  `intoStructSerie`. The field-class protocol entry (:401) is unchanged.

## 4. Tests

Mirror files (AGENTS "Where a test lives"); every refusal written and run red
before its happy path; a test reaches the crate through `yggdryl::` only.

- **`rust/tests/root/serie.rs`** (mirrors `rust/src/serie.rs`, where the verb
  lives). Refusal first: `into_struct_serie` of a run is refused with
  `invalid record value at $: a schema-free run declares no field`. Then: a
  leaf column wraps as a required `row` whose one child is the column's field
  unchanged; zero copy - the child's `into_arrow_array()` buffers keep their
  `data_ptr()` through the wrap; a struct column holding absent rows answers
  itself, its validity kept, and its nullable field stays nullable; a zero-row
  column wraps to zero rows; the wrap's rows equal the source's rows wrapped
  (`scalar(i)` is the one-cell row).
- **`rust/tests/root/chunked_serie.rs`**: `into_struct_serie` of a three-chunk
  leaf column shares one root - every chunk's `field_ref()` `Arc::ptr_eq` with
  the answer's `field_ref()` - and keeps the chunk count and ends; a struct
  field answers itself; a chunked serie of no chunk answers the root alone;
  `SerieReader::from_chunked` of the wrap names the same root as of the
  source.
- **`rust/tests/serie/arrow.rs`**: refusal first - a held door over a
  non-record column at the ceiling (a `serie<..>` field whose own walk reaches
  depth 63, built as `rust/tests/root/structure.rs`'s `chain` builds it) is
  refused by `SerieReader::from_serie`, by `Serie::into_arrow_batch` and by
  `SerieReader::root_of` naming the column and `hard limit of 64`. Then the
  absent-row refusal reads one text from both doors (the three absent-row
  tests named under "Unchanged suites" assert `1 absent rows`; add one row
  asserting the whole sentence from `batch_under` and from `held_record`).
- **`rust/tests/excel/sheet.rs`**: refusal first - `write_serie` of a column at
  the ceiling is refused naming it; the existing rows below unchanged.
- **Unchanged suites prove the reroute** - every line of them passes untouched:
  `rust/tests/serie/arrow.rs` `the_held_root_is_one_rule_every_held_door_names_its_field_by`
  (:1592), `a_record_column_holding_an_absent_row_is_not_a_table` (:749),
  `a_run_and_a_record_column_holding_an_absent_row_are_no_stream` (:1381),
  `a_chunked_record_column_holding_an_absent_row_is_no_stream` (:1503) and
  every `from_serie` / `from_chunked` / `into_arrow_batch` pin;
  `rust/tests/excel/sheet.rs`
  `a_serie_that_is_not_a_record_is_the_one_column_named_as_it_is_and_a_run_is_refused`
  (:661), `write_serie_lays_rows_out_from_an_anchor_with_or_without_the_header_replacing_cells_there`
  (:683), `write_serie_refuses_rows_or_columns_that_would_leave_the_grid_before_writing_any`
  (:715); the window suites (`rust/tests/root/window_serie.rs`,
  `rust/tests/serie/order.rs`, `rust/tests/root/chunked_serie.rs`); the avro
  suite; `rust/tests/iobase_calls.rs -- records::`.
- **Python** (`python/tests/test_datatype.py`, `test_field.py`,
  `test_scalar.py`, `test_serie.py`, `test_chunked_serie.py`,
  `typing_bindings.py`): refusals first - `Serie.into_struct_serie` of a run
  raises `ValueError` with the core text; `Scalar.from_([[1]]).into_struct_field()`
  still raises (`match="field names"`, `test_scalar.py` ≈751) and
  `Scalar.from_([])` raises the empty-rows text. Then every verb against the
  core's answer; the widened door - `Scalar.from_({"id": 1}).into_struct_field()`
  is a required struct named `row` with child `id`, `Scalar.from_(5)` is
  `row: struct<value: int64>`, `Scalar.from_(None)` has a nullable `value`;
  `DataType.is_struct` is a property; a `StructSerie` comes back from
  `Serie.into_struct_serie`; `mypy --strict` over the stubs and
  `typing_bindings.py`.
- **Node** (`node/tests/datatype.test.js`, `field.test.js`,
  `text/codec.test.js`, `serie.test.js`, `chunked_serie.test.js` and their
  `.types.ts`): the same cases; the refusals at `text/codec.test.js` ≈1286 and
  ≈1311-1314 kept; a plain object's `intoStructField()` is a required `row`
  struct, a leaf a `row` over `value`; `Serie.intoStructSerie()` answers a
  `StructSerie`; in `field.test.js`, a native `Field` passed where a record
  class is read is still not one (`intoField(field)` answers the field, a
  `Field` instance in a records write is refused as before); `tsc --noEmit`.

## 5. Pins

**Must not move** (`rust/tests/allocations.rs`; a moved count is a design
answer back to its phase, never a re-pin):

| Line | Pin | Why it is at risk |
| --- | --- | --- |
| :3341 (test :3344) | `WINDOW_BY_VALUES_KEY` - "two for the `row` root the column binds under" | `window_by` binds against `root_of`'s wrap arm, which now runs the probe; the probe allocates nothing and the build is `record_root`'s |
| :3547 (test :3559) | `CHUNKED_WINDOW_BY_BIND` - the `row` root (two) | the same, through `chunked_serie.rs:1118` |
| :7901-7936 | `a_held_reader_costs_its_root_and_an_exact_batch_lands_for_less_than_an_imported_one`, `held == [7, 9]` - "the preflight walk's pending vector" | a record column through `held_root`; the struct arm checks nothing and `held_root` keeps `validate_bounded`, so the walk is still counted once |

And unchanged with them: :2468 `every_other_serie_verb_costs_its_rung_and_never_a_row`,
:2684 `a_window_verb_costs_the_window_s_serie_and_the_serie_s_own_verb`, the
window rows :3213, :3817, :3870, :3944, :4052, :4126, :4483
(`static_values_are_lent_free`) and :4522, and the chunked rows :4703 and
:4847 (the latter counting `SerieReader::from_chunked` at :4893).

**Added** (each at two corpus sizes, 4,096 and 262,144 rows, through `counted`):

| Row | Claim |
| --- | --- |
| `into_struct_on_a_struct_costs_a_clone` | `into_struct_type`, `into_struct_field`, `into_struct_serie` and `ChunkedSerie::into_struct_serie` on a struct cost exactly what `clone` costs (a `Serie`: nothing; a `ChunkedSerie`: its two vectors) |
| `a_struct_wrap_costs_one_root_and_never_a_row` | `Serie::into_struct_serie` of a leaf column costs the same at both sizes: the root `into_struct_field` builds, its `Arc` and the record leaf around the one child, never a row - the count read off the first run and every part of it named in the row's rustdoc, as the neighbouring pins name theirs |
| `a_chunked_struct_wrap_shares_one_root` | `allocations(100 chunks) - allocations(1 chunk) = 99 x` the per-chunk constant: the root is built once |

**`rust/tests/iobase_calls.rs`:** nothing added; `-- records::` re-run
unchanged.

## 6. Benchmarks

- `rust/benchmarks/arrow.rs`: `into_struct_serie` rows in
  `arrow_serie_construct` (a leaf column and a record column, `Serie`) and
  `arrow_chunked_serie` (`ChunkedSerie`), at the `bench_profile` corpus. The
  `arrow_serie_into_reader` group (`SerieReader::from_serie`, the reroute's
  hot path) is the direction check, unchanged. Smoke:
  `cargo bench -p yggdryl --bench arrow -- into_struct_serie --quick` and
  `-- arrow_serie_into_reader --quick`.
- The `typed/struct` rows are part 1's; nothing changes there.
- Python boundary rows: `python/benchmarks/types/serie.py` (`Serie` and
  `ChunkedSerie.into_struct_serie`, against `pa.StructArray.from_arrays` over
  the same array), `python/benchmarks/datatypes.py` (`DataType.into_struct_type`,
  `Field.into_struct_field`, against `pa.struct` / `pa.field`),
  `python/benchmarks/types/scalars.py` (`Scalar.into_struct_scalar`, the
  widened `into_struct_field`). `python/tests/test_benchmarks.py` runs every
  script.
- Node: rows in `node/benchmarks/types.js` (`npm run --prefix node bench:types`).
- No page states a number until the release run on the machine its table
  names (AGENTS §2, "What CI never runs").

## 7. Docs and skills

**Already documented by part 1** (in the working tree when this was written,
and still being edited, so described by heading, not line): on
`docs/types/datatype.md`, `field.md` and `scalar.md` an `## As a struct`
section each (contract, a Rust example, and Python and JavaScript tabs that say
"Rust only: no binding reaches ..."), `scalar.md`'s `## Inferred fields`
covering `inferred_record_field` (its binding tabs also "Rust only"), the
`## Edges` and `## Commands` rows of the three pages (the probe's refusal, the
`typed/struct` bench filters), `field.md`'s "Converting to one native field"
table naming `Field::into_struct_field`, and on `docs/types/nested/struct.md`
an `## As a struct` door table pointing at the three pages and saying the doors
are Rust only. `skills/yggdryl-types/SKILL.md` has the door rows "a non-record
as a record" and "the record root of any value, no schema" (Python and
JavaScript columns "Rust only") and a pitfall saying Python's
`Scalar.into_struct_field()` and JavaScript's `intoStructField()` are the
rows-only `inferred_struct_field`; `references/rust.md` has the recipe.
AGENTS.md's Public vocabulary has the `into_struct_<root>` row naming the three
done verbs.

**Part 2 adds:**

- `docs/types/serie.md`: an `## As a struct` section - the §1 table, the
  relation to `Serie::as_struct` (a narrowing) and to `SerieReader::root_of`
  (the table rule, which forces required and refuses absent rows), a Rust /
  Python / JavaScript example asserting the wrap and the zero copy, the run
  refusal; an `## Edges` line; a `## Commands` bench line. The `window_by` Key
  row and the `from_serie` edge already name `root_of` and stay true.
- `docs/types/chunked-serie.md`: `into_struct_serie` - one shared root, chunks
  kept apart - beside `## Streams`.
- `datatype.md`, `field.md`, `scalar.md`: the "Rust only" tabs become Python and
  JavaScript examples (`DataType.is_struct` as a property and `isStruct` as a
  getter); `scalar.md`'s Inferred fields tabs show the widened
  `Scalar.into_struct_field()` / `intoStructField()`, the refusals included.
- `docs/types/nested/struct.md`: the door table gains `Serie::into_struct_serie`
  and `ChunkedSerie::into_struct_serie` (owned by `serie.md` and
  `chunked-serie.md`) and drops "Rust only".
- Skills: `skills/yggdryl-types` - the two door rows' Python and JavaScript
  columns, the pitfall rewritten to the widened contract, recipes in
  `references/python.md` and `references/javascript.md`;
  `skills/yggdryl-arrow` - a door row "a column as a record"
  (`into_struct_serie`) and recipes in its three references; `skills/yggdryl`
  if its entry table lists `Serie` doors.
- AGENTS.md: the `serie.rs` and `chunked_serie.rs` Layout rows name
  `into_struct_serie`; the Public vocabulary `into_struct_<root>` row gains
  `Serie::into_struct_serie` and `ChunkedSerie::into_struct_serie`; "Serie is
  the collection" gains the row "any column as a record | `serie.into_struct_serie()?`,
  `chunked.into_struct_serie()?`: itself when a struct, else the one child of a
  required `row`".
- Inventories: `.api-inventory.txt` (§2), `.api-bindings.txt` (§3), then
  `python scripts/check_api_inventory.py`.

## 8. Phases

One branch from `main`, phases in the AGENTS order, one commit at the end.
Nobody runs `cargo fmt --all` until Ship; every edit is a scripted exact-string
edit whose anchor matches once.

| Phase | Files | Settle |
| --- | --- | --- |
| C1 serie | `rust/src/serie/structure.rs`, `serie/arrow.rs`, `serie.rs`; tests `rust/tests/root/serie.rs`, `rust/tests/serie/arrow.rs` | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl --test root serie`; `cargo test -p yggdryl --test serie -- structure arrow order`; `cargo test -p yggdryl --test expression selector` |
| C2 chunked | `rust/src/chunked_serie.rs`; test `rust/tests/root/chunked_serie.rs` | `cargo test -p yggdryl --test root chunked_serie` |
| C3 excel, text | `rust/src/excel/sheet.rs`, `rust/src/text/typed.rs`; test `rust/tests/excel/sheet.rs` | `cargo test -p yggdryl --test excel`; `cargo test -p yggdryl --test text typed` |
| Pins and bench | `rust/tests/allocations.rs` (the three rows), `rust/benchmarks/arrow.rs` | `cargo test -p yggdryl --test allocations -- window a_held_reader_costs_its_root static_values into_struct struct_wrap`, then once unfiltered `cargo test -p yggdryl --test allocations`; `cargo test -p yggdryl --test iobase_calls -- records::`; `cargo bench -p yggdryl --bench arrow -- into_struct_serie --quick` |
| W whole run | - (one background chain, one log, a marker per step) | `cargo test -p yggdryl --all-targets --all-features --no-fail-fast`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings`; `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo test -p yggdryl --doc`; `cargo check --workspace --all-targets --keep-going --message-format=short` (lists the bindings' call sites). Foreground meanwhile: `.api-inventory.txt`, the Rust tabs of §7 |
| P Python | `python/src/{datatype,field,scalar,serie,chunked_serie,iomedia,cast,iobase}.rs`, `python/yggdryl/_native.pyi`, `python/tests/{test_datatype,test_field,test_scalar,test_serie,test_chunked_serie}.py`, `python/tests/typing_bindings.py`, `python/benchmarks/{datatypes.py,types/serie.py,types/scalars.py}` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/<file> -x -q` per file; `python/.venv/bin/python -m pytest python/tests/test_benchmarks.py -q`; `python/.venv/bin/python -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py` |
| N Node | `node/src/{datatype,field,serie,chunked_serie}.rs`, `node/src/text/codec.rs`, `node/binding.js`, `node/binding.d.ts`, regenerated `node/index.js` / `index.d.ts`, `node/tests/{datatype,field,serie,chunked_serie}.test.js`, `node/tests/text/codec.test.js` and their `.types.ts`, `node/benchmarks/types.js`, regenerated `docs/assets/fix.json` and `playground.json` | `npm run --prefix node build:debug`; `node --test node/tests/<file>.test.js` per file; `npm run --prefix node test:package:debug`; `git diff --exit-code -- node/index.js node/index.d.ts`; `cargo build --locked -p yggdryl-cli`; `npm test --prefix node`; `node scripts/build_docs_playground.js --check`; `node scripts/build_docs_fix.js --check` |
| D docs | §7's pages, skills, AGENTS.md rows, `.api-bindings.txt` | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript`; `python scripts/check_api_inventory.py` |
| Ship | `cargo fmt --all` once; delete `.handoff/next/STRUCT_SPEC.md` | one commit, push, open the PR, read CI per job, fix what is red (AGENTS §2 "A red run"), squash-merge. Report the skipped local-only checks: the release benchmarks behind any page number |

## 9. Decisions (defaults stated)

1. **Names - DECIDED by the user.** `into_struct_*` only; no `as_struct_*`
   conversion in Rust or a binding.
2. **The bindings' inference door - DECIDED by the user.** No rename:
   `Scalar.into_struct_field` / `intoStructField` read both shapes through
   `Scalar::inferred_record_field`.
3. **Borrowed "is it a struct".** Default: no new narrowing on `DataType` or
   `Field`; `DataType::is_struct` is the one new predicate (done).
4. **Already-struct nullability.** Default: answered as itself, nullable and
   absent rows kept; `root_of` alone forces required.
5. **Wrap child names.** `value`, nullable, for `DataType` and `Scalar`; the
   field's own name for `Field`, `Serie` and `ChunkedSerie`, under a required
   `row` root (done for the first three).
6. **Return type.** Default: `crate::Result` for both `into_struct_serie`
   verbs, as `into_struct_field` - no Arrow type crosses and no plan runs
   (AGENTS "Errors by side").
7. **Where the verbs and their tests live.** Default: `serie.rs` beside
   `as_struct` and `chunked_serie.rs` beside `into_serie`; tests in
   `rust/tests/root/serie.rs` and `rust/tests/root/chunked_serie.rs`, the
   mirrors; `StructSerie::wrap` is crate-private and pinned through the verbs
   and the held doors.
8. **The bound.** Default: the probe bounds the wrap arm; `held_root` keeps
   `validate_bounded`; the three pins of §5 do not move.
9. **The absent-row refusal.** Default: one constructor, today's text, no
   remedy clause.
10. **The ceiling refusal moves to `root_of`** and names the column, for every
    held door, `window_by`'s bind and `Sheet::write_serie`. Default: accepted
    and pinned (§4); a pinned text that moves is re-pinned with that reason.
11. **The Avro root.** Default: not rerouted (a root union would answer a
    nullable struct the record surface refuses).
12. **The `text/typed.rs:161` literal.** Default: swept here, the bare
    datatype's value field being the same fact as `default_dtype_array`'s.
    Alternative: leave it to the duplicate-logic review.
13. **Predicates in the bindings.** Default: Python `DataType.is_struct` a
    property; Node `isStruct` a getter on `DataType` and `Field`, as Python's
    are properties. Alternative: a Node method `isStruct()`.
14. **Binding literals.** Default: both constants in both bindings, the
    pyo3 signature default spelled with the constant and the stub keeping
    `"row"`.
15. **The GIL.** Default: the Python wraps run on the GIL; they read no row.
16. **Expression roots, multi-child roots, the `as_fields()` reads,
    `into_arrow_schema` widening.** Default: unchanged here, still refusing
    where they refuse; the duplicate-logic review decides them.
17. **Numbers on pages.** Default: none until the release run; the Commands
    sections carry the filters.
