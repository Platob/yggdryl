# xxHash implementation brief

Implement the whole xxHash family - XXH32, XXH64, XXH3-64, XXH3-128 - behind one
`rust/src/xxhash/` module folder: one-shot digests, resumable streaming states,
digests over any `IOBase` handle through bounded byte streams, a transparent
hashing handle, and a canonical byte feed that hashes any `Scalar` without
allocating. Follow `AGENTS.md`; this file contains only xxhash-specific
decisions.

Do not reimplement the algorithms. The protocol comes from a pinned dependency;
this module owns the vocabulary, the streaming, the handle integration, the
scalar encoding, and the bindings.

## Outcome

- `DigestAlgorithm` names the four algorithms and is the only dispatcher, the
  way `Codec` is for content codings.
- `Digest` is one immutable value: algorithm plus payload, canonical big-endian
  bytes, lowercase hex `Display`/`FromStr`, `Eq`, total `Ord`, `Hash`, serde.
- `xxhash::{xxh32, xxh64, xxh3_64, xxh3_128}` and their seed/secret forms answer
  native widths with no wrapper cost.
- Four resumable streaming states feed bytes, readers, and `Scalar` values, and
  answer a digest without being consumed.
- `IOBase::read_digest` streams any handle in constant memory through
  `pstream_bytes`, so a 64 GiB object hashes in one bounded window. Every
  wrapper - `Coded`, `Gzip`, `Buffered`, `Media`, a table format - inherits it.
- `Hashed<H>` is a transparent handle that hashes bytes as they are written and
  answers without re-reading.
- `Scalar` gains its canonical byte representation: a borrowed allocation-free
  payload view for leaves, and a total prefix-free feed for every variant.
- Arrow batches answer per-row digest columns identical to the scalar feed.
- `stable_hash` becomes XXH3-64 everywhere and the hand-written FNV-1a sink is
  deleted, so the project has exactly one hash contract.
- Python and JavaScript reach the whole surface, verified against an outside
  implementation, with boundary benchmarks.

## Dependency: `twox-hash`, not a reimplementation

Add one dependency to `rust/Cargo.toml`:

```toml
# The xxHash protocol itself: XXH32, XXH64, XXH3-64, and XXH3-128, with the
# runtime AVX2/SSE2/NEON dispatch that keeps XXH3 at reference speed. The
# `parquet` feature already compiles it, so a table build pays nothing new; the
# four algorithm features plus `std` pull no transitive crate, and `random` is
# off because a `RandomState` would drag `rand` in for nothing.
twox-hash = { version = "=2.1.3", default-features = false, features = [
    "xxhash32",
    "xxhash64",
    "xxhash3_64",
    "xxhash3_128",
    "std",
] }
```

Why this crate, decided against the alternatives:

| Candidate | Verdict |
| --- | --- |
| `twox-hash` 2.1.3 | Chosen. All four algorithms, streaming plus one-shot, seeds and custom secrets, SIMD XXH3, MIT, MSRV 1.81, zero transitive deps at these features, and already in `Cargo.lock` because `parquet` and `lz4_flex` depend on it. |
| `xxhash-rust` 0.8.18 | Complete and fast, but a second copy of a protocol the tree already compiles, and its `const` surface buys us nothing. |
| `xxhash-c` / `xxhash-sys` | C `libxxhash` through a build script. A C toolchain requirement for a hash is not a trade this workspace makes anywhere else. |
| Hand-written in `xxhash/` | Rejected. `unsafe_code = "deny"` rules out the SIMD kernels that make XXH3 worth choosing, so a from-scratch port would be slower than the crate it replaced while owning a wire-format bug surface for no gain. |

`std` is required, not decorative: XXH3's `is_x86_feature_detected!` dispatch and
the boxed secret used by the streaming states both need it. Keep the version
pinned with `=` like `smol_str`, `saphyr-parser`, and `iceberg-official`: a
digest is a wire contract, and a silent minor bump must never be able to move a
value.

`twox-hash` is a protocol implementation, not a vocabulary. Its types never
appear in a public signature, a doc example, an error, or a binding. It is
called from `rust/src/xxhash/` and nowhere else, so replacing it later touches
one folder.

## Read first

- `AGENTS.md` in full.
- `rust/src/gzip/mod.rs` and `rust/src/generic/codec.rs` for the module and
  dispatcher shape this mirrors exactly.
- `rust/src/io/mod.rs` for `IOBase`, `pstream_bytes`, `delegate_iobase!`, and
  the derived-method rules.
- `rust/src/io/stream.rs` for `ByteStream` and its fuse-after-error contract.
- `rust/src/io/coding.rs` and `rust/src/buffered/` for wrapping handles.
- `rust/src/generic/scalar.rs`, `rust/src/generic/typed.rs`, and
  `rust/src/generic/datatype_id.rs` for the value model and its ids.
- `rust/src/text/display.rs` for the FNV-1a sink this change deletes, and every
  `stable_hash` method that routes through it.
- `rust/src/arrow/` for the scalar/array boundary the row digests reuse.
- `docs/gzip.md` and `docs/io.md` for page shape.

## Rust layout

Create `rust/src/xxhash/` with real ownership:

| File | Responsibility |
| --- | --- |
| `mod.rs` | module docs, one-shot free functions, `reader`/`writer`, re-exports |
| `state.rs` | the four streaming states, `clear`, `std::hash::Hasher`, `BuildHasher` |
| `secret.rs` | secret validation and the seed/secret constructors |
| `stream.rs` | `DigestReader<R>`, `DigestWriter<W>`, and the `IOBase` streaming digest |
| `handle.rs` | `Hashed<H>` |
| `scalar.rs` | the canonical `Scalar` feed and `ValueBytes` |
| `arrow.rs` | per-row digest columns, `arrow` feature only |
| `tests.rs` | vectors, chunk invariance, handle and scalar edge cases |

Add `generic/digest.rs` for `DigestAlgorithm`, `Digest`, and `Digester`, and
re-export all three from `generic` and the crate root, beside `Codec`.

`pub mod xxhash;` is unconditional in `lib.rs`, between `uri` and `yaml`, like
`avro`: the value codec has no Arrow dependency, and only `xxhash/arrow.rs` is
gated. `--no-default-features --lib` must keep compiling.

## Vocabulary

`DigestAlgorithm` variants are `Xxh32`, `Xxh64`, `Xxh3_64`, `Xxh3_128`. Those
spellings pass `non_camel_case_types` under `-D warnings`; they are the names in
Rust, Python, and JavaScript alike, because an algorithm identifier is not a
word sequence and has no camelCase form. Canonical tokens are exactly `xxh32`,
`xxh64`, `xxh3-64`, `xxh3-128`, one spelling each, parsed and displayed by
`DigestAlgorithm`; no aliases, no `xxh3` shorthand.

`DigestAlgorithm` answers `width()` in bytes, `bits()`, `is_seedable()`,
`is_secretable()` (XXH3 only), `ALL` in canonical order, `from_str`, `Display`,
serde, and `digester()`.

`Digest` stores the algorithm and a `u128` payload, is `Copy`, and answers:

- `as_u32`/`as_u64`/`as_u128` - the native value, `None` when the width differs;
- `into_bytes()` - the canonical big-endian representation the reference calls
  `XXH*_canonicalFromHash`, at the algorithm's exact width;
- `from_bytes(algorithm, bytes)` - the inverse, rejecting a wrong length;
- `Display` - lowercase hex, zero-padded to the width, and `FromStr` its exact
  inverse, so `Digest::from_str(&digest.to_string())? == digest`;
- `algorithm()`, `Eq`, total `Ord`, `Hash`, serde, `stable_hash`.

Two digests of different algorithms are never equal, whatever their payload.

`Digester` is the runtime-selected streaming state, an enum over the four
concrete states, exactly as `Encoder` is for `Codec`. `DigestAlgorithm` is the
only place that maps a name to an implementation.

Free functions take the input first and the seed second - `xxh64(input)`,
`xxh64_with_seed(input, seed)` - even though the dependency spells `oneshot`
the other way round; normalize at the call site and never leak the foreign
order. Secret forms are `xxh3_64_with_secret(input, secret)` and
`xxh3_64_with_seed_and_secret(input, seed, secret)`, both `Result`, rejecting a
secret below the reference's 136-byte minimum with a typed error naming the
required and actual length. XXH32 and XXH64 take a seed and never a secret;
`DigestAlgorithm::is_secretable` is what a caller asks first.

Streaming states are `Xxh32`, `Xxh64`, `Xxh3_64`, `Xxh3_128`:

- `new()`, `with_seed(seed)`, `from_secret(secret)`, `from_seed_and_secret(seed, secret)`
  - the fallible pair is `from_*` because it validates a named representation;
- `write_bytes(&mut self, bytes: &[u8])`;
- `write_scalar(&mut self, value: &Scalar)`;
- `write_reader(&mut self, source: &mut impl Read) -> Result<u64>` answering the
  byte count consumed, through one reused bounded buffer;
- `as_digest()`, plus `as_u32`/`as_u64`/`as_u128` at the state's own width -
  borrowed, allocation-free, and resumable: answering never consumes the state;
- `clear()` - back to the constructed seed and secret, not to `new()`;
- `impl std::hash::Hasher`, and a `BuildHasher` per state so they drop into a
  `HashMap`. Document that `Hasher::finish` on `Xxh3_128` answers the low 64
  bits because the trait's return type cannot carry more, and that `as_u128` is
  the full value.

## Streaming over `IOBase`

Add derived methods on `IOBase`. Both have default bodies; adding a required
method here is forbidden, so every backend and every wrapper inherits them
unchanged:

```rust
fn read_digest(&self, algorithm: DigestAlgorithm) -> Result<Digest>;
fn read_range_digest(&self, offset: u64, length: usize, algorithm: DigestAlgorithm)
    -> Result<Digest>;
```

Rules:

- Both drive `pstream_bytes`, retain one bounded chunk, and never call
  `read_all_bytes`. Memory is flat in the object size; prove it in the
  benchmark, not in prose.
- A missing resource digests as empty, matching lazy construction: the digest of
  zero bytes is the algorithm's empty-input constant, never an error.
- A container kind is a typed error naming the kind. Folder and recursive
  digests are out of scope.
- The stream fuses after error like every other byte stream; a partial digest is
  never returned.
- Because the coding wrappers delegate bytes, `Gzip::read_digest` is the digest
  of the decoded payload and `gzip.handle().read_digest(..)` is the digest of the
  compressed form. Test both; they are the property that proves the wrapper
  stack is honest.
- `Buffered<H>` must not be polluted: the digest read goes through
  `pstream_bytes`, which retains no page cache.

`xxhash::reader(source, algorithm)` and `xxhash::writer(target, algorithm)`
mirror the codings' pair: `DigestReader<R>` passes bytes through `Read`
unchanged while hashing them, `DigestWriter<W>` tees to `Write`. Both answer
`as_digest()` at any point and `into_inner()` to give the wrapped value back.
This is how a caller hashes a payload it is already moving, without a second
pass.

## `Hashed<H>`

`rust/src/xxhash/handle.rs` holds one transparent handle, built like `Gzip<H>`:
`delegate_iobase!` and `delegate_iomedia!`, overriding only what changes.

- `Hashed::new(handle, algorithm)`, `with_seed`, `handle()`, `into_handle()`.
- A running state covers writes that are strictly sequential from offset 0 -
  `write_all_bytes`, repeated `append_bytes`, a streamed record write. That is
  the case worth optimizing and the case a lake actually writes.
- Any positional write that is not the running append point, plus `clear` and
  `remove`, marks the state stale. Staleness is not an error and is not silent
  corruption: `read_digest` then re-streams the handle and re-arms the state.
- `read_digest` answers from the running state when it is live and matches the
  handle size, and re-streams otherwise. The answer is identical either way;
  only the cost differs.
- Pending writes are part of the digest only after `flush`, matching
  `pwrite` staging and `flush`/`close` publishing everywhere else.

## `Scalar` byte representations

This is the part every other feature leans on: one canonical byte
representation per `Scalar`, defined once, allocation-free, and reusable by
anything that needs the bytes of a value.

Two deliberate spellings, both on `Scalar`:

`as_value_bytes(&self) -> Option<ValueBytes<'_>>` is the payload alone - no tag,
no length - so hashing `Scalar::from("AAPL")` equals hashing `b"AAPL"` and
matches any other xxHash implementation on the same bytes. `ValueBytes<'_>`
derefs to `&[u8]` and is either borrowed from the value or an inline fixed
array; it never allocates and never copies a string or a byte payload. It
answers `None` for `Null`, `Sequence`, `Mapping`, and `Record`, which have no
payload without a framing.

`write_bytes(&self, sink: &mut impl Hasher)` is the total, prefix-free canonical
feed over every variant. `std::hash::Hasher` is the sink because it already
exists, every state here implements it, and so does the `stable_hash` wrapper -
no new trait, and one feed serving both.

| Variant | Feed |
| --- | --- |
| `Null` | id |
| `Bool` | id, `0x00` or `0x01` |
| `I8`..`U128` | id, value little-endian at its exact width |
| `F16`/`F32`/`F64` | id, canonical IEEE bits little-endian; NaN is already normalized at construction |
| `D128`/`D256` | id, coefficient little-endian, scale as one signed byte |
| `String` | id, length `u64` little-endian, UTF-8 bytes |
| `Bytes`/`Geospatial` | id, length `u64` little-endian, bytes |
| `Enum` | id, length-prefixed enum identity, then the stored integer at its width |
| `Date32`/`Time32`/`Duration32` | id, value `i32` little-endian, `TimeUnit` byte, length-prefixed timezone |
| `Date64`/`Time64`/`DateTime64`/`Duration64` | id, value `i64` little-endian, `TimeUnit` byte, length-prefixed timezone |
| `Sequence` | id, element count `u64` little-endian, each element's feed in order |
| `Mapping` | id, entry count `u64` little-endian, each key feed then value feed in stored order |
| `Record` | id, entry count `u64` little-endian, then per sorted entry a length-prefixed name and the value's feed |

The id is the `DataTypeId` discriminant as one byte. That byte is a wire
contract: pin every value in a test that fails when a variant is inserted, and
say so in the doc comment.

Invariants, each with a test:

- `a == b` implies an identical feed, over a corpus covering every variant.
- Values that differ produce different feeds, including across variant
  boundaries: `Scalar::from("1")`, `Scalar::Bytes(b"1")`, and `Scalar::U8(0x31)`
  all differ, and `Sequence([a, b])` differs from `Sequence([ab])`.
- The result never depends on how the sink batches the writes.
- The feed allocates nothing. Prove it with the counting allocator at several
  corpus sizes, per `AGENTS.md`, not by timing.
- Nesting is depth-bounded by the existing structured-value limit and never
  recurses without one; caller input never reaches `unwrap` or a panic.

Add `Scalar::digest(algorithm) -> Digest` and the same on `TypedScalar<K>` as
the one-call convenience. Both are thin: they build a `Digester`, feed, answer.

`Scalar::stable_hash` becomes XXH3-64 over this feed rather than over the
structural `Hash` sink, so the value and its digest have one definition. See the
next section.

## One hash: `stable_hash` becomes XXH3-64

The project currently carries a second, hand-written hash: an FNV-1a fold in
`rust/src/text/display.rs` behind `stable_hash_bytes`, `stable_hash_chunks`,
`stable_hash_display`, and `stable_hash_of`, which the 44 public `stable_hash`
methods and both bindings route through. Two hash contracts is one too many.
Delete the FNV implementation and re-point every caller at XXH3-64. Per
`AGENTS.md` there is no dual path, no legacy spelling, and no migration reader:
the values change, and every pinned constant, test, and doc sentence changes with
them in the same commit.

What is deleted:

- `FNV_OFFSET_BASIS`, `fnv1a_fold`, and the private `StableHash` FNV sink;
- the pinned FNV expectations in `rust/src/text/display.rs` tests;
- every sentence describing the contract as FNV-1a;
- `hashlib.blake2b` in `python/yggdryl/fields/_defaults.py`, whose generated-class
  identity becomes `field.stable_hash()` - a binding must not carry its own hash
  when the core answers one. Drop the `hashlib` import with it.

Two of the four public spellings go with it, because after the swap they are
aliases rather than implementations:

- `text::stable_hash_bytes(bytes)` would be exactly `xxhash::xxh3_64(bytes)`,
  and an alias with an alternate name is forbidden. Delete it; callers use
  `xxh3_64`. Nothing outside `rust/src/text/` references it today, so this is a
  one-file deletion.
- `text::stable_hash_chunks(chunks)` would be exactly an `Xxh3_64` state fed in
  a loop. Delete it too. Its two documented properties are the state's
  properties now: move the doc example and both assertions to `xxhash`, where
  they must keep passing - chunked and contiguous input agree, and an empty
  chunk contributes nothing wherever it sits.

Remove the `pub use display::{stable_hash_bytes, stable_hash_chunks};`
re-export in `rust/src/text/mod.rs` with them.

The two sinks that are implementations stay, re-pointed at the new hasher:

- `stable_hash_display(value)` feeds the canonical `Display` rendering through
  an `Xxh3_64` state via `fmt::Write`, unchanged in shape.
- `stable_hash_of(value)` keeps a private `Hasher` wrapper around `Xxh3_64`
  that overrides `write_u8` through `write_usize` with explicit little-endian
  bytes, exactly as the FNV sink does today. Do not hand a bare `Xxh3_64` to a
  `Hash` implementation: the default `write_*` bodies use native-endian bytes,
  which would make a stored hash disagree between a big-endian and a
  little-endian machine. This wrapper is the reason the sink stays a named type
  and not an inline call.

Nothing in the surviving surface is `const`. `stable_hash_bytes` is `const`
today and XXH3 cannot be evaluated at compile time; since that function is being
deleted the question resolves itself, and no replacement should be made `const`
by reintroducing a compile-time fold.

`Scalar::stable_hash`, and the `Float16`/`Float32`/`Float64`/`Float`/`Integer`
helpers beside it, answer XXH3-64 over the canonical byte feed rather than over
the structural `Hash` sink, because that feed is now the value's byte
definition. The invariant they are tested against is unchanged: equal values
hash equally across widths.

Everything else - `Field`, `Metadata`, `Uri`, `Url`, `Urn`, `MimeType`,
`MediaType`, `Scheme`, `Timezone`, `DataType`, `Expression`, `RecordOptions`,
`TextOptions`, the Iceberg values - keeps its current sink choice and only
changes hasher. No `stable_hash` method is added, removed, or renamed, and no
signature changes: the contract stays "a deterministic `u64`, equal for equal
values, stable across runs, platforms, and releases", now named as XXH3-64 in
the docs instead of FNV-1a.

## Arrow

`rust/src/xxhash/arrow.rs`, `arrow` feature only:

- `row_digests(batch: &RecordBatch, algorithm) -> Result<ArrayRef>` answers one
  digest per row - `UInt64Array` for the 64-bit algorithms, `FixedSizeBinary(16)`
  for XXH3-128 - which is the dedup key, change-detection column, and hash-join
  key a trading table actually wants.
- `column_digests(array: &dyn Array, field: &Field, algorithm) -> Result<ArrayRef>`
  is the single-column form; the batch form composes it under the batch's
  `Field`.
- The result must equal feeding each row's `Scalar` through `write_bytes`.
  That equality is the contract and the test, on every datatype family, nulls,
  nested structs, lists, maps, dictionaries, and unions.
- Read buffers directly where the layout allows it and fall back to
  `arrow::scalar_value` where it does not, so the path stays exhaustive. The
  buffer path is what the benchmark measures against the fallback.
- Nulls feed the `Null` id, so a null and an empty string never collide.

## Not this change

State these in the docs where a reader would ask, and do not implement them:

- xxHash is not cryptographic. Never use a `Digest` as an integrity check
  against an adversary, and say so on the module page.
- It is not Iceberg's bucket transform. That spec mandates murmur3 x86_32; a
  bucket implemented with xxHash would produce wrong partitions.
- No expression node, no folder or recursive digest, no second hash family, no
  `xxhsum` binary.

## Python

- `python/src/xxhash.rs` plus `python/yggdryl/xxhash.py`, shaped like the
  existing `codec`/`codings` pair.
- One-shot functions accept anything with the buffer protocol - `bytes`,
  `bytearray`, `memoryview` - without copying, plus `str` encoded as UTF-8.
  Answer `int`.
- Streaming classes `Xxh32`, `Xxh64`, `Xxh3_64`, `Xxh3_128` with
  `write_bytes`, `write_scalar`, `as_digest`, `clear`, and the same seed and
  secret constructors.
- `Digest` is immutable with `__int__`, `__bytes__`, `__str__`, `__eq__`,
  `__hash__`, `__repr__`, ordering, pickle, and copy.
- `IOBase.read_digest(algorithm)` and `Scalar.digest(algorithm)` redirect
  natively. `DigestAlgorithm` is exposed through the existing enums package.
- Errors map exception type only and keep the native message.

## JavaScript

- `node/src/xxhash.rs` plus declarations, camelCase for multi-word names only;
  the algorithm identifiers stay `xxh32`, `xxh64`, `xxh3_64`, `xxh3_128` in
  every language.
- Accept `Buffer`, `Uint8Array`, `ArrayBuffer`, and `string`; answer `number`
  for XXH32 and `bigint` for the 64- and 128-bit results.
- Streaming classes mirror Python's; `Digest` answers `toString`, `toJSON`,
  equality, hash, and clone.
- `readDigest` on the handle and `digest` on the scalar redirect to the same
  native path. No JS-side hashing.

## Tests

Pin these constants; they were produced with the pinned dependency and match the
published reference values:

| Input | XXH32 | XXH64 | XXH3-64 | XXH3-128 |
| --- | --- | --- | --- | --- |
| `""` | `0x02CC5D05` | `0xEF46DB3751D8E999` | `0x2D06800538D394C2` | `0x99AA06D3014798D86001C324468D497F` |
| `"abc"` | `0x32D153FF` | `0x44BC2CF5AD770999` | `0x78AF5F94892F3950` | `0x06B05AB6733A618578AF5F94892F3950` |

Note that XXH3-128's low 64 bits equal XXH3-64 on the same input; assert it.

Cover:

- every XXH3 size branch - 0, 1-3, 4-8, 9-16, 17-128, 129-240, and past 240 -
  seeded, unseeded, and with a custom secret, plus a secret one byte short;
- chunk invariance: for a fixed payload, every split - 1, 7, 64, 240, 1024,
  whole - yields the identical digest, as a property test over random splits;
- `IOBase::read_digest` equals the one-shot digest of `read_all_bytes` for
  `Buffer`, a memory-mapped local file, a `Gzip` handle and its wrapped handle,
  a `Buffered` handle, and an `arrowfs` handle; empty and missing resources; a
  container kind erroring by name;
- `read_range_digest` bounds, including a range past the end;
- `Hashed<H>`: sequential writes answer without re-reading, an out-of-order
  positional write re-streams to the same value, `clear` and `remove` re-arm,
  and pending writes only count after flush;
- `Digest`: hex round trip, canonical byte round trip, wrong-width rejection,
  cross-algorithm inequality, ordering, serde;
- the `Scalar` feed invariants and the `DataTypeId` byte pin;
- Arrow row digests equal the scalar feed on every datatype family;
- the `stable_hash` swap: `stable_hash_display` over a `&str` equals
  `xxh3_64` of its UTF-8 bytes, the chunked-equals-contiguous and empty-chunk
  properties still hold on their new home, the little-endian `write_*`
  overrides make `stable_hash_of` answer the same value on any target, every
  existing `stable_hash` equality and stability test passes with its constants
  regenerated, and neither `grep -ri fnv` nor `grep -rn stable_hash_bytes` finds
  anything left in the tree;
- outside validation: the Python suite compares every one-shot and streaming
  result against the `xxhash` PyPI package, which binds C `libxxhash`. Both
  directions, and a skipped half is not a pass.

## Benchmarks

`rust/benchmarks/xxhash.rs` plus `rust/benchmarks/xxhash/`, registered in
`rust/Cargo.toml` with `harness = false`, compiled in a default build.

Measure release builds for:

- each algorithm at 1, 4, 16, 64, 128, 240 B and 1, 64 KiB, 1, 64 MiB, reported
  in GB/s;
- this module's one-shot against a direct `twox-hash` call on the same payload:
  the number being measured is wrapper overhead, so state it as such. Expect it
  inside noise from 4 KiB up; report the small-payload dispatch cost honestly
  rather than hiding it behind a large-payload average;
- streaming in `pstream_bytes`-sized chunks against one-shot;
- `IOBase::read_digest` against `read_all_bytes` plus one-shot, over a large
  local file, reporting peak resident bytes for both - that gap is the reason
  this method exists;
- `Hashed<H>` write-through against an unwrapped write plus a separate digest
  pass;
- the `Scalar` feed: leaf, wide record, deep nesting, with the allocation
  baseline;
- `stable_hash_display` and `stable_hash_of` before and after the swap, on the
  short canonical renderings they actually see - a field name, a URI, a datatype
  expression. FNV-1a is competitive at a handful of bytes; if XXH3-64 is slower
  there, report the number and keep the single contract rather than tuning the
  benchmark or reintroducing a second hash;
- Arrow row digests: buffer path against the `scalar_value` fallback, and
  against a naive per-row `Scalar` materialization;
- Python against the `xxhash` package and JavaScript against its own boundary,
  with conversion cost visible, not hidden.

Keep fixtures outside the measured loops. Publish generated results on
`docs/xxhash.md` with machine, runtime, and build named. Regenerate; never edit
numbers.

## Documentation

- New `docs/xxhash.md`: purpose sentence, one-shot digests, streaming states,
  handle digests, `Hashed<H>`, the scalar feed with its encoding table, Arrow
  row digests, the not-cryptographic and not-murmur3 notes, then the benchmark
  results.
- Add it to `mkdocs.yml` nav under `Storage`, after `zstd`.
- Update `docs/generic.md` for `DigestAlgorithm`/`Digest`/`Digester`,
  `docs/io.md` for `read_digest`, `docs/benchmarks.md` for the new commands and
  the result link, and the two extension pages.
- Retire every "FNV-1a" sentence: `rust/src/text/display.rs`, the six napi doc
  comments in `node/src/{datatype,field,timezone,uri}.rs` - `node/index.d.ts`
  regenerates from them, do not hand-edit it - and the `stable_hash` prose in
  `docs/{generic,field,uri,avro,iceberg}.md` and `docs/extensions/python.md`.
  The replacement names XXH3-64 and links to `docs/xxhash.md`.
- Rust/Python/JavaScript tabs in that order, every block self-contained with an
  assertion, passing `scripts/check_docs_examples.py`.
- Update `AGENTS.md`: name `Digest`, `DigestAlgorithm`, and `Digester` in the
  generic enum inventory, and add the `rust/src/xxhash/` ownership line beside
  the `{gzip,zlib,zstd}` one, saying `DigestAlgorithm` is the only dispatcher.
- Update `.api-inventory.txt` and `.api-bindings.txt` by hand; both are
  maintained, not generated.

## Phases

Land each as its own commit, green before the next starts.

1. `twox-hash` pinned, `generic/digest.rs`, `xxhash/` one-shot plus streaming
   states, vectors and chunk invariance.
2. `stream.rs`, the two `IOBase` derived methods, `reader`/`writer`.
3. `Hashed<H>`.
4. The `Scalar` feed and `ValueBytes`, with the allocation proof.
5. The `stable_hash` swap and the FNV deletion, including the Python
   `blake2b` identity. One commit, values regenerated, tree grep clean.
6. `xxhash/arrow.rs` row digests.
7. Python and JavaScript, with the outside-implementation comparison.
8. Docs, benchmarks, contract files.

## Completion

Run the checks `AGENTS.md` requires for the touched surface: formatting,
warning-free Clippy, workspace tests with default features and with
`parquet iceberg`, Rust 1.85 default and `--no-default-features --lib`, rustdoc
with warnings denied, the new and neighbouring benchmarks, both extension suites
and their boundary benchmarks, docs examples and `python -m mkdocs build
--strict`, and the dead-code, duplicate-logic, and Rust-only binding sweeps.
Hand off only the outcome, the changed surfaces, the verification results, any
remaining caveat, and the exact next action.
