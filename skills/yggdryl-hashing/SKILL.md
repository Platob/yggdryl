---
name: yggdryl-hashing
description: Digest bytes, handles, values and Arrow rows with yggdryl's xxHash (XXH32, XXH64, XXH3-64, XXH3-128) and couple digests with instants as TxHash. Use when hashing bytes or a file (xxh3, read_digest / readDigest), streaming a resumable state (Xxh3, write_bytes / writeBytes), seeding XXH3 or giving it a secret, computing a cross-language stable_hash / stableHash of a value, declaring DIGEST:role=holder row-digest columns and filling them (apply_arrow_batch / applyArrowBatch, row_digests), building TxHash / TxHasher values, sortable UUIDv7 keys (into_uuid / intoUuid) or DIGEST:time coupled holders. Covers Rust, Python and Node.js.
---

# Yggdryl hashing

Two families over one digest engine. `xxhash` digests bytes, handles, values
and Arrow rows with XXH32, XXH64, XXH3-64 and XXH3-128; `txhash` couples a UTC
instant with one of those digests into one sortable `TxHash`, and defines no
second hash. The fact to hold: **`stable_hash` is XXH3-64 over the canonical
value feed, everywhere** - one value answers one number in Rust (`u64`),
Python (`int`) and JavaScript (`bigint`), whatever width or leaf holds it.
A `Digest` carries its algorithm with the number, so two algorithms never
compare equal. Not cryptographic, and not Iceberg's `bucket[N]` (murmur3).

| Algorithm token | Width | JS answer | Holder storage it fills |
| --- | --- | --- | --- |
| `xxh32` | 4 bytes | `number` | `int32` / `uint32` (the fresh default) |
| `xxh64` | 8 bytes | `bigint` | `int64` / `uint64`, when an `Xxh64` state fills or `DIGEST:algorithm` names it |
| `xxh3-64` (default) | 8 bytes | `bigint` | `int64` / `uint64` |
| `xxh3-128` | 16 bytes | `bigint` | `fixed_size_binary(16)` |

A `TxHash` adds an 8-byte big-endian instant in front: 12, 16 or 24 bytes,
spelled `<unix>@<unit>:<algorithm>:<hex>`.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| digest bytes | `xxhash::xxh3(b)`, `xxh32`, `xxh64`, `xxh128` | `xxhash.xxh3(b)` | `xxhash.xxh3(buf)` |
| digest carrying its algorithm | `DigestAlgorithm::Xxh3.digest(b)`, `xxhash::digest(b, alg)` | `xxhash.digest(b, "xxh3-64")` | `xxhash.digest(buf, 'xxh3-64')` |
| seed | `xxh64_with_seed(b, 42)` | `xxhash.xxh64(b, seed=42)` | `xxhash.xxh64(buf, { seed: 42n })` |
| XXH3 secret | `xxh3_with_secret(b, &secret)?` | `xxhash.xxh3(b, secret=s)` | `xxhash.xxh3(buf, { secret })` |
| resumable state | `Xxh3::new()`, `write_bytes`, `as_u64` / `as_digest` | `xxhash.Xxh3()`, `write_bytes`, `as_int()` / `as_digest()` | `new xxhash.Xxh3()`, `writeBytes`, `asDigest()` |
| algorithm chosen at runtime | `DigestAlgorithm::from_str(s)?.digester()` | `xxhash.Digester(s)` | pick `Xxh32` / `Xxh64` / `Xxh3` / `Xxh128` |
| drain a reader | `state.write_reader(&mut r)?` | `state.write_reader(fileobj)` | not bound |
| digest a stored resource | `handle.read_digest(alg)?`, `read_range_digest(off, len, alg)?` | `IOBase(p).read_digest()`, `read_range_digest(off, len)` | `new IOBase(p).readDigest()`, `readRangeDigest(off, len)` |
| hash while copying | `xxhash::reader(r, alg)`, `writer(w, alg)`, `Hashed::new(h, alg)` | not bound (feed a state) | not bound (feed a state) |
| stable hash of a value | `scalar.stable_hash()` | `Scalar.from_(v).stable_hash()` | `Scalar.from(v).stableHash()` |
| value digest, any algorithm | `scalar.digest(alg)` | `scalar.digest("xxh64")` | `scalar.digest('xxh64')` |
| feed a value into a state | `state.write_scalar(&v)` | `state.write_scalar(v)` | `state.writeScalar(v)` |
| declare a holder column | `f.as_digest_mut().set_holder()?`, `set_sources([..])?` | `f.digest.set_holder()`, `f.digest.sources = [..]` | metadata `'DIGEST:role': 'holder'`, `'DIGEST:sources': '["a"]'` |
| fill holders, seedless | `root.apply_arrow_batch(&b, true, true, true, opts)?`, `root.as_digest().apply_arrow_batch(&b)?` | `root.apply_arrow_batch(b)`, `root.digest.apply_arrow_batch(b)` | `new xxhash.Xxh3().applyArrowBatch(root, b)` |
| fill holders, seeded / forced | `state.apply_arrow_batch(&root, b, force)?` | `state.apply_arrow_batch(root, b, force=True)` | `state.applyArrowBatch(root, b, true)` |
| digest every row / cell | `xxhash::arrow::row_digests(&b, alg)?`, `column_digests(a, &f, alg)?` | `xxhash.row_digests(b)`, `column_digests(a, f)` | not bound |
| couple an instant | `txhash::txh3(b, unix)`, `txhash::digest(b, unix, alg)` | `txhash.txh3(b, unix)` | `txhash.txh3(buf, unix)` |
| configure unit, seed, secret once | `TxHasher::new_in(unit, alg)?.with_seed(7)`, `from_digester` | `txhash.TxHasher("xxh64", unit="s", seed=7)`, `from_state` | `new txhash.TxHasher('xxh64', 's', 7n)`, `fromState` |
| read an instant | `txhash::unix_from_scalar(&v, unit)?` | `txhash.unix_of(v, unit="us")` | `txhash.unixOf(v, 'us')` |
| sortable UUIDv7 | `value.into_uuid()?`, `into_sequenced_uuid(seq, seed)?` | `value.into_uuid()`, `into_sequenced_uuid(seq, seed)` | `value.intoUuid()`, `intoSequencedUuid(seq, seed)` |
| coupled column | `txhash::arrow::row_txhashes(&b, &t, unit, alg)?`, `compose`, `decompose` | `txhash.row_txhashes(b, t)`, `compose`, `decompose` | not bound |
| coupled holder | `set_time("event")?`, `set_unit(TimeUnit::Second)?` | `f.digest.time = "event"`, `f.digest.unit = "s"` | metadata `'DIGEST:time'`, `'DIGEST:unit'` + `TxHasher.applyArrowBatch` |
| parse and render | `Digest::from_str`, `TxHash::from_str`, `from_bytes`, `into_bytes` | `xxhash.Digest(s)`, `TxHash(s)`, `from_bytes`, `bytes(x)` | `Digest.from(s)`, `TxHash.from(s)`, `fromBytes`, `bytes()` |

## Rules for fast, correct use

1. **Use `stable_hash` for cross-language identity.** It is the same number in
   all three languages for equal values; a hand-rolled hash of a text or JSON
   rendering is not, and drifts with formatting.
2. **The value feed is framed.** `stable_hash` hashes a tag byte then the
   canonical form (lengths included), so it is not `xxh3` of the value's
   bytes; the bare payload is `as_value_bytes` (Rust, Python).
3. **Equal values, one digest; different values, apart.** `int8` 1 and
   `int64` 1, `float32`/`float64` 1.5, `decimal(1.00)`/`decimal(1)`, and every
   string leaf of the same characters hash equal; `null` vs `""`, `"1"` vs
   `b"1"`, and a `ccy` vs a `country` with the same text stay apart.
4. **Hash a stored resource where it lives.** `read_digest` streams the handle
   through `pstream_bytes` one bounded chunk at a time on every backend - the
   memory high-water mark does not move and no byte crosses into the host.
   Never `read_all_bytes` then hash.
5. **Hash in the pass you already make.** Rust `xxhash::reader` / `writer`
   hash a copy in flight; `Hashed<H>` answers `read_digest` from its running
   state while writes are sequential from offset 0, and re-streams once after
   a positional write. In Python and JavaScript feed the chunks you copy to a
   state.
6. **States are resumable.** Any split answers the one-shot digest; reading a
   digest never consumes the state; `clear()` returns to the constructed seed
   and secret; JavaScript `clone()` forks.
7. **Carry the algorithm.** Store or send a `Digest` (`xxh3-64:<hex>`) or a
   `TxHash` rather than a bare integer: `xxh64` and `xxh3-64` are both 64 bits
   and differ.
8. **Mark one holder, leave its sources ordinary columns.** `DIGEST:role =
   holder` on the digest column; `DIGEST:sources` (a JSON array of paths
   relative to its own Struct) narrows the input, absent or `["*"]` meaning
   every non-holder field. The schema pipeline fills holders last, after cast
   and transform, and a written (non-default) cell is preserved unless forced -
   so re-filling is idempotent and cheap.
9. **Seedless or seeded, decide once.** `Field.apply_arrow_batch` and the
   `digest` view fill with the seedless `stable_hash`; a state's
   `apply_arrow_batch` applies its seed and secret. A seed changes every
   digest, so a seeded key is only comparable with the same seed.
10. **Secrets are XXH3 only**: at least 136 bytes, refused below that, and
    consulted only for payloads longer than 240 bytes.
11. **Small inputs pay the binding.** A one-byte call costs about 166 ns in
    Python and 496 ns in Node over the hash itself; batch small values through
    a holder fill or `row_digests` instead of a call per value. A Python
    `bytes` is borrowed; `bytearray`/`memoryview` are copied (1.7x slower); a
    JavaScript string is UTF-8 encoded (7.5x slower than a `Buffer`).
12. **A TxHash sorts by time only within one unit, one algorithm, one sign.**
    Its bytes are the big-endian instant then the digest; value order is unit,
    then count, then digest; instants are never normalized across units.
13. **Instants are UTC, microseconds by default.** A zoned datetime already
    counts from the epoch, a naive one reads as UTC, a date is its midnight; a
    coarser restatement floors, overflow is refused. Set the resolution once
    on a `TxHasher`.
14. **UUIDv7 needs a 64-bit digest and a non-negative instant.** `into_uuid`
    keeps the microsecond and all 64 digest bits (nothing hashed again, both
    read back); `into_sequenced_uuid(sequence, seed)` is the graph-event
    layout; Rust `into_ordered_bytes` is the 16-byte chronological key with no
    UUID bits.
15. **JavaScript Arrow crosses by copy.** A state's or `TxHasher`'s
    `applyArrowBatch` copies the batch through Arrow IPC both ways; row and
    coupled column functions are Rust and Python only.

## Pitfalls

| Wrong | Right |
| --- | --- |
| `xxh3(json.dumps(row))` as a row key | `Scalar.from_(row).stable_hash()` / `Scalar.from(row).stableHash()` - one number in every language |
| expecting `Scalar.from_("AAPL").stable_hash() == xxhash.xxh3(b"AAPL")` | they differ by design; compare `stable_hash` with `stable_hash`, or use `as_value_bytes` for the payload |
| `xxh3(handle.read_all_bytes())` / `fs.readFileSync` then hash | `handle.read_digest()` / `readDigest()` - streamed, constant memory |
| a column mixing bare `xxh64` and `xxh3-64` integers | keep the `Digest` (it carries its algorithm) or one declared `DIGEST:algorithm` |
| JS `xxhash.xxh3(buf) === 123` or `+ 1` | it is a `bigint`: compare with `123n`; only `xxh32` is a `number` |
| marking the source columns as holders too | mark only the digest column; a holder never feeds itself or another holder |
| `DIGEST:sources` naming a path through a serie or map | sources descend Structs only; a serie, map or union is selected whole |
| `TxHasher::new(..).with_seed(7)` after giving a secret | `with_seed` drops a secret: build `Xxh3::from_seed_and_secret` / `Xxh3(seed=, secret=)` and pass it to `from_digester` / `from_state` |
| sorting raw `TxHash` bytes across units, algorithms or pre-1970 instants | sort values (`<`, `compare`), or keep one unit and algorithm; a negative instant's bytes sort last |
| `txh128(...).into_uuid()` | refused: UUIDv7 carries 64 digest bits - use `xxh3-64` or `xxh64` |
| Python `unix_of(True)` / JS `unixOf(true)` | refused as `TypeError`; pass an `int`/`bigint`, a datetime, text or a `Scalar` |
| treating a digest as tamper-proof | xxHash detects accidental change only; use a cryptographic hash for adversarial input |

## Language references

- `references/rust.md` - read when writing Rust.
- `references/python.md` - read when writing Python.
- `references/javascript.md` - read when writing JavaScript / TypeScript.

## Deeper

- Contract, algorithms and bindings: https://platob.github.io/yggdryl/hashing/
- Streaming, seeds and secrets: https://platob.github.io/yggdryl/hashing/#streaming,
  https://platob.github.io/yggdryl/hashing/#seeds-and-secrets
- Handles and write-through: https://platob.github.io/yggdryl/hashing/#handles
- The value feed and its encoding: https://platob.github.io/yggdryl/hashing/#values,
  https://platob.github.io/yggdryl/hashing/#encoding
- Holders and row digests: https://platob.github.io/yggdryl/hashing/#digest-holders-and-row-digests
- TxHash, UUIDv7, instants, TxHasher: https://platob.github.io/yggdryl/hashing/#txhash-values,
  https://platob.github.io/yggdryl/hashing/#order-and-uuidv7-projection
- Coupled columns and holders: https://platob.github.io/yggdryl/hashing/#coupled-columns,
  https://platob.github.io/yggdryl/hashing/#coupled-holders
- Every edge: https://platob.github.io/yggdryl/hashing/#edges; measured costs:
  https://platob.github.io/yggdryl/hashing/#performance
- The `digest` protocol view on a field: https://platob.github.io/yggdryl/types/protocol/
- Sibling skills: `yggdryl-storage` (the `IOBase` handles `read_digest`
  streams), `yggdryl-types` (`Scalar`, `Field` metadata, `apply_arrow_batch`),
  `yggdryl-arrow` (batches and readers), `yggdryl-market-data` (event
  identities built on `TxHash` and UUIDv7), `yggdryl-expressions`
  (`stable_hash` of a plan as a cache key).
