# Optimization contract

This document defines the performance rules for the Rust core and its Python
and JavaScript bindings. Optimize only after preserving behavior with tests and
measuring a representative workload.

## Allocation boundaries

- Scalar `DataType` values, empty `Fields`, empty field metadata, field getters,
  metadata lookup, validation of already-built values, and cloning a
  `Field`, `Fields`, `Metadata`, or nested `DataType` must not allocate.
- Allocation is expected when input strings outgrow compact inline storage,
  when a nested child slice or metadata map is first built, and on the first
  Arrow projection. Keep each of those costs at the construction or conversion
  boundary rather than in a per-record loop.
- A cached Arrow projection may only clone its `Arc`; it must not rebuild the
  Arrow datatype, field metadata map, or child fields.
- Prefer one output allocation with known or estimated capacity. Do not hide
  repeated allocation behind iterator chains when a sized builder or a linear
  merge makes the boundary clearer.
- Normalize a resource identifier once at parse/import. Component getters,
  path-segment iteration, file-name lookup, and suffix lookup borrow canonical
  storage and must not allocate.

## Compact and shared storage

- Keep scalar enum variants inline. Put recursively sized or multi-value state
  behind `Arc`, and use `Arc<[T]>` for immutable ordered collections.
- Represent empty immutable collections without a per-value backing
  allocation. Empty metadata values clone one process-wide `Arc<BTreeMap>`.
  Use `SmolStr` for schema-sized names; metadata uses owned `String` entries so
  a unique consuming Arrow projection can move those allocations directly
  into Arrow's `HashMap`.
- Use the same compact storage for URI schemes and common components. Do not
  retain both the caller's platform path and its normalized `file:` spelling.
- Share complete immutable snapshots. Never clone a child vector or metadata
  slice merely to return a getter, project an unchanged field, or cross an
  internal API boundary.
- Preserve deterministic order for fields and metadata. `BTreeMap` metadata
  provides allocation-free borrowed lookup and lexical iteration while cloned
  snapshots share one `Arc`.
- `Arc<[T]>` deliberately keeps a non-empty immutable child collection to one
  allocation. Stable Rust cannot unwrap an unsized Arc slice, so consuming a
  Struct or Union shallow-clones child Field values into Arrow. Do not switch
  to `Arc<Vec<T>>` (two allocations) without measurements showing a net win.

## Bulk construction and mutation

- Collect fields or metadata once, reserve from an exact-size iterator when
  available, validate once, then publish an immutable shared slice.
- For several metadata changes, use `Field::set_metadata`,
  `Field::update_metadata`, or `Field::try_with_metadata_entries`. Do not chain
  single inserts: each persistent insert may copy the snapshot.
- A bulk overlay must validate its complete input before publishing any change.
  Overlay values win on duplicate keys, and duplicate keys within one input
  remain an error. A uniquely owned map mutates in place; shared state uses
  `Arc::make_mut` once before applying the validated overlay.
- Detect an identical overlay before cloning or mutating the map; retaining the
  current Arc also retains an already-populated Arrow cache.
- Keep builders outside record-processing loops. Records should reference a
  completed schema, not rebuild schema state.

## Arrow cache rules

- Treat a cached Arrow field as a projection of all field state: name,
  datatype, nullability, dictionary ID/order flags, nested children, and
  metadata.
- Every effective mutation of projected state must invalidate the cache. A
  no-op mutation with an equal value should retain it. Cloning an unchanged
  field should share an already-populated cache.
- Validate before publishing a cold projection. Populate the cache atomically
  and make races converge on one stored value; never expose a partially built
  projection.
- Nested state exposed by the public API must remain immutable. If interior
  mutation is ever introduced, it needs an explicit generation/version scheme
  so parent projections cannot become stale.
- Canonical imported shared Arrow FieldRefs seed the exact same projection
  Arc. Borrowed imports clone only that Arc. If reserved metadata is
  canonicalized in the field or any descendant, propagate that fact once and
  rebuild the affected parent projection instead of caching stale foreign
  state.
- Arrow 59 exposes Field metadata as a borrowed private `HashMap`, so it cannot
  transfer that map independently into core storage. Preserve the complete
  imported `FieldRef` for zero-copy reprojection. Borrowed standalone metadata
  projection clones strings; consuming uniquely owned metadata moves strings
  and allocates only Arrow's destination buckets.

## Downstream record hot paths

- Do not allocate a `HashMap`/`BTreeMap` per record. Store record values in
  Arrow arrays/builders or schema-indexed contiguous buffers.
- Resolve field names and positions once per schema or decoder session. A
  shared lookup table at that boundary is acceptable; map construction while
  decoding each record is not.
- Reuse input buffers and Arrow builders across records. Reset lengths while
  retaining capacity, and flush batches at an explicit row or byte threshold.
- Keep the common valid branch straight-line. Attach detailed error context
  only after detecting an invalid value.

## JSON and YAML codec hot paths

- Accept and emit bytes directly. Do not build an intermediate `String` tree;
  YAML text is written to `io::Write`, and JSON/YAML readers decode one framed
  value or document at a time.
- Borrow in-memory slices and keep stream iteration lazy. JSON Lines is one
  complete value per nonempty line; ordinary JSON and YAML streams may read
  ahead, but must not materialize all decoded values before yielding the first.
- Share bytes, sequences, and mappings through `Arc`. Empty
  collections use one process-wide backing allocation, and cloning a nested
  value must not walk or copy its children.
- Keep string-key JSON mappings native. Use typed envelopes only for bytes,
  non-finite floats, wide integers, decimals, unspellable temporals, arbitrary
  mapping keys, and reserved-key collisions; every envelope kind names one
  `Value` variant. YAML class comments are presentation only, and no decode
  path reads one.
- Apply byte, depth, node, alias, and document limits while reading. Depth and
  node limits are per document; exact-limit acceptance and one-over rejection
  belong in the benchmark-adjacent adversarial tests.
- Measure representative encode/decode, one-frame-at-a-time streams, deep
  nesting, wide mapping validation, shared clones, exotic enveloped values, and
  reserved-envelope collisions before changing the wire representation.

## Python field-class hot paths

- Compile annotations once per dataclass. Cache the native root `Field`, the
  ordered child-Field tuple, and resolved hints together. Cached access through
  the static `Class.field()` accessor and pure `field(Class)` builder must use a
  class-local fast path without acquiring the schema-construction lock.
- A scalar-decorated dataclass builds its native field on the first `field()`
  call. Keep that lazy construction synchronized; remove
  pending namespace state immediately after success and do not let a
  weak-cache value retain its own class key.
- Keep Python typing introspection in Python, but construct only native
  `DataType`/`Field` values. Scalar inference helpers and imported modules may
  be cached because native datatype wrappers are immutable; never cache a
  mutable standalone Field as a global annotation result.
- Freeze decorated-class-owned root and child Fields before publishing their
  singleton references. Reject mutation at the native boundary; never rebuild
  a root Struct merely to reconcile a mutated detached child.
- Retain resolved annotations, not captured frames. Deferred schemas keep only
  bindings reachable from their annotation graph; nested generic schemas copy
  only the reachable resolved-cache subtree.

## Validation rules

- Validate untrusted input at constructors, FFI boundaries, and before a cold
  Arrow projection. Do not repeat schema validation for every record.
- String and Arrow recursion share `DataType::PARSE_RECURSION_LIMIT`; keep
  near-limit success and one-over-limit rejection covered on the smallest
  supported thread stack.
- Typed constructors and field mutation methods must preserve invariants for
  decimal precision/scale, list sizes, map shape, union IDs, dictionary keys,
  run-end types, field IDs, and non-null primary keys.
- Bulk operations validate the completed candidate before replacing live state.
  On error, callers retain the previous valid value.
- URI constructors validate schemes, delimiters, percent escapes, and URN
  namespace rules before publishing a value. Windows and UNC path detection is
  textual and host-independent; it must not depend on `cfg!(windows)`.
- Add unit tests for the minimum, maximum, empty, duplicate, conflicting, and
  invalid-sign cases of each invariant. Add a regression test before changing a
  path that previously accepted or rejected an edge case incorrectly.

## Binding rules

- Python and JavaScript wrappers own or clone native shared handles; they must
  not maintain a second independent schema model.
- Keep metadata lookup as a keyed native call. Materialize a Python `dict` or
  JavaScript object only when the caller explicitly requests the full snapshot.
  Send bulk metadata across FFI once rather than issuing one native call per
  entry.
- Route all binding constructors and mutations through core validation. Convert
  Rust errors to stable language exceptions and never use `unwrap` or panic on
  user input.
- Use the Arrow C Data Interface or the runtime's native Arrow bridge where it
  can preserve shared buffers. Document any unavoidable copy at the binding
  boundary.
- Release the Python GIL or avoid blocking the JavaScript event loop only for
  work large enough to amortize scheduling; cloning a field or reading metadata
  stays synchronous.

## Measurement requirements

Run the focused suite in release mode:

```text
cargo bench --manifest-path rust/Cargo.toml --bench datatype
cargo bench --manifest-path rust/Cargo.toml --bench field
cargo bench --manifest-path rust/Cargo.toml --bench uri
cargo bench --manifest-path rust/Cargo.toml --bench text
cargo bench --manifest-path rust/Cargo.toml --bench json
cargo bench --manifest-path rust/Cargo.toml --bench yaml
python python/benchmarks/fields.py --iterations 100000
```

The suite measures scalar and deeply nested parsing, canonical field parsing,
shared nested clones, allocation-free nested validation, stable hashes,
metadata hit/miss lookup and bulk overlay, datatype projection, and cold,
cached, and consuming field projections. Resource identifier benchmarks cover
canonical parsing, Windows/UNC normalization, display round trips, cloning,
hashing, and borrowed component/suffix lookup.
Codec benchmarks cover byte-slice encode/decode, strict JSON Lines and YAML
document streaming, depth-64 values, 8- and 1024-entry mappings, shared nested
clones, and representative round trips. Collision and resource-limit cases remain
correctness baselines because malformed-input timing is not an optimization
target.
`BatchSize::SmallInput` keeps construction of cold fixtures outside their timed
projection routines.
The Python script separately measures cached `Class.field()` and `field(Class)`
calls, annotation inference, nested dataclass fields, and cold `@scalar`
decoration. Use the same interpreter/build mode; never compare debug and release
extension results.

- Compare on the same machine, power mode, Rust toolchain, target features, and
  benchmark fixture. Record the command and commit IDs for both baseline and
  candidate.
- Inspect Criterion's estimate and 95% confidence interval; investigate a
  repeatable regression above 5%, even if it is not yet statistically
  significant. Do not claim an improvement from a single sample.
- Measure allocations separately with a platform allocator profiler. Hot-path
  allocation counts are an invariant: do not accept a new allocation in clone,
  lookup, or cached projection without a documented design change.
- Use `std::hint::black_box`, separate setup from the timed routine, and keep
  cold and cached cases distinct. A benchmark that mixes cache population with
  cache hits cannot justify either path.
- Re-run unit tests and the benchmark compile check on the declared Rust 1.85
  MSRV. Benchmark-only dependencies stay under `[dev-dependencies]`; production
  and extension dependency graphs must not include Criterion.
