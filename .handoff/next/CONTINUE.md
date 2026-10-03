# Continue from main

Read `AGENTS.md` first; it is the contract, and its Smoke loop section is
normative. Then do the numbered items below in order, each as its own branch
and PR, squash-merged into `main` once CI is read and green, with `.handoff/`
cleared by the change that consumes it.

## Base

`main` after the struct-pair PR from branch
`claude/iceberg-partition-expressions-oc6r3k` is squash-merged (its title
opens "The struct pair: is_struct, into_struct_type, into_struct_field and
into_struct_scalar"). If `git log --oneline -5 origin/main` does not show it,
that PR is still open: read its CI, fix what is red, squash-merge, and only
then start item 1.

## What is in the tree

- The struct pair on the three roots, Rust only: `DataType::is_struct`,
  `DataType::into_struct_type`, `Field::into_struct_field`,
  `Scalar::into_struct_scalar` (`rust/src/structure.rs`);
  `Scalar::inferred_record_field` (`rust/src/media/inference.rs`);
  `media::DEFAULT_VALUE_NAME` beside `DEFAULT_ROOT_NAME`; the `"row"` and
  `"value"` literals in `rust/src` read the two constants; an
  allocation-free depth probe in `rust/src/default.rs` bounds the wrap at
  `DataType::PARSE_RECURSION_LIMIT`. Tests in `rust/tests/root/structure.rs`,
  `rust/tests/media/inference.rs`, `rust/tests/avro/arrow.rs`; docs on
  `docs/types/datatype.md`, `field.md`, `scalar.md` and the `yggdryl-types`
  skill; `.api-inventory.txt` current.
- Not done, deliberately: no binding has the struct pair; `Serie` and
  `ChunkedSerie` have no `into_struct_serie`; `SerieReader::root_of` and
  `serie/arrow.rs`'s `record_root` are not rerouted; the Python and Node
  `"row"` literals still stand.
- Abandoned by the user, never to be revived: spilled series, memory-mapped
  spill stores, automatic spilling, a `HolderSerie` or any serie accumulated
  on `IOBase`. Nothing of it is in the tree.

## 1. Struct conversions, part 2

Spec: `.handoff/next/STRUCT_SPEC.md` (done, remaining core, bindings, tests,
pins, docs, decisions). In one sentence each:

- Core: `StructSerie::wrap(root, child)` crate-private; `Serie::into_struct_serie`
  (a Struct leaf as is, absent rows kept; a Run refused; else the zero-copy
  one-child wrap under `field.into_struct_field()`); `ChunkedSerie::into_struct_serie`
  with one shared root `Arc`; `SerieReader::root_of` becomes
  `field.into_struct_field()?.with_nullable(false)` while `held_root` keeps
  `validate_bounded` so the three allocation pins the spec names do not move;
  `record_root` deleted, `held_record`'s non-record arm the wrap,
  `batch_under` reading `is_struct`, excel `record_columns` deleted; the Avro
  root is not rerouted (a `["null", record]` root would turn nullable).
- Bindings: Python `DataType.is_struct`, `into_struct_type`,
  `Field.into_struct_field`, `Scalar.into_struct_scalar`,
  `Serie.into_struct_serie`, `ChunkedSerie.into_struct_serie`; Node the
  camelCase same plus `Field.isStruct` for parity. `Scalar.into_struct_field`
  and `intoStructField` keep their names and read both shapes through
  `Scalar::inferred_record_field` (the user's decision). The Python `"row"`
  literal sites read the core constant. Parity tests, boundary benchmarks,
  stubs, `mypy --strict`, regenerated `node/index.*`, the docs tabs, the
  skills, both inventories.

## 2. The castings-first duplicate-logic review

Run `.handoff/next/DEDUP_REVIEW_PROMPT.md` as written: it is the prompt, with
its exclusions re-scoped now that nothing of the spill change lands.

## 3. Clear `.handoff/`

The change that consumes a file deletes it; the last one removes the folder.
