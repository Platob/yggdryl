# Calls

Every derived operation makes the fewest calls to [`IOBase`](bytes.md) it needs, and `Counted` is how that is a number rather than an intention.

## Contract

| Key | Rule |
| --- | --- |
| Why it matters | `IOBase` is the one boundary a layer crosses to reach storage; on a store each crossing is a round trip, so the count decides the wall clock |
| The rule | A whole read is one call. A question the operation already answered is none. A question two branches want is asked once and reused |
| Wrappers | Override the defaults that would ask again - the `size`-then-read pair above all - rather than inheriting them |
| Retention | A handle that resolves a role or a length keeps it for the scope that retains it, and drops it where the answer can change |
| Measured by | `holder::counted::Counted` wraps a handle, forwards every call unchanged, and tallies it by name |
| Asserted by | `rust/tests/iobase_calls.rs`, exactly rather than as a bound; the `holder` benchmark reports the same counts beside the timings |
| Not this | How many *requests* a backend makes of the network to answer one call. That is the backend's own counter - see [Object stores](../backends/object.md) |

## Use

Wrap the storage handle, build the stack on top of it, and read the tally.

```rust
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::holder::counted::{Call, Counted, Group};

let mut source = Buffer::new();
source.write_all_bytes(&vec![7_u8; 4096])?;

let counted = Counted::new(source);
let calls = Arc::clone(counted.calls());

// A whole read is one call, whatever the value's length.
assert_eq!(counted.read_all_bytes()?.len(), 4096);
assert_eq!(calls.get(Call::ReadAllBytes), 1);
assert_eq!(calls.total(), 1);

// The tally names the call, so a read that became two is legible.
assert_eq!(counted.counts().to_string(), "read_all_bytes=1");

// And groups them, so a scan is read as four numbers rather than thirty-one.
calls.reset();
assert_eq!(counted.size(), 4096);
assert_eq!(calls.group(Group::Metadata), 1);
assert_eq!(calls.group(Group::Read), 0);
```

## Where the instrument goes

Under the layer being measured, never over it. A media reader, a coding, or a
page cache built on a `Counted` handle has every call it makes to storage
counted; the same wrapper built the other way round counts one call and hides
what it decomposed into.

```rust
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::counted::Counted;
use yggdryl::IOBase;

let mut source = Buffer::new();
source.write_all_bytes(&vec![7_u8; 64 * 1024])?;
let counted = Counted::new(source);
let calls = Arc::clone(counted.calls());
let cached = counted.buffered(BufferedOptions::default());

// The first read fetches the page holding the window, and learns the length
// while it is there.
assert_eq!(cached.read_range_bytes(0, 16)?.len(), 16);
assert_eq!(calls.snapshot().to_string(), "pread=1 size=1");

// Every later read inside that page reaches the handle for nothing at all -
// including the length bound, which the inherited default would re-ask for.
calls.reset();
assert_eq!(cached.read_range_bytes(0, 16)?.len(), 16);
assert!(calls.snapshot().is_empty());
```

## What it forwards

`Counted` mirrors the surface a *backend* implements: the positional
primitives, the whole and ranged reads and writes, the metadata answers, the
lifecycle pair, and the three navigation methods. Each is tallied and passed
straight through, so the wrapper changes nothing.

It deliberately does not forward the derived defaults - `glob`, `partitions`,
`children_where`, `copy_into`, `read_scalar`, `cursor`, `reader_at`. Those run
against the wrapper, so what the tally records is the calls they *decompose
into*, which is the thing worth knowing: a partition scan costing one listing
and one costing a call per file read the same from outside and differently
here.

## Edges

- A child handle from `parent` or `child_by_path` is the wrapped backend's own, not another `Counted`; the call that produced it is tallied, what a caller then does through it is not.
- The tally is shared behind an `Arc`, so a reading handle survives moving the wrapper several layers down; `reset` is what a benchmark uses between the setup it does not want to count and the operation it does.
- Counting is a relaxed atomic add per call: it is not free, and it is not a thing to leave in a production stack.
- `Counted` counts calls, not bytes. A layer that makes one call and transfers a gigabyte through it reads as one - which is right for the question this answers, and is why the transfer volume of a whole-value read is stated on the page that owns it.
- A [ZIP archive](../backends/zip.md) counts itself instead: it holds a `Holder`, and the enum has no variant a counted handle could arrive as, so `Archive::handle_reads` and `handle_writes` tally the same crossings one layer in. The pins live beside the others in `rust/tests/iobase_calls.rs`.

## Performance

One run of each operation, wall clock beside the calls it makes, over a 4 MiB
value on an in-memory handle. The point of the pairing is that the timings are
the *floor*: memory answers a call in nanoseconds, so a row that costs several
still looks cheap here. Read the count as what the same row would cost in round
trips against a store, where each one is a request.

| Operation | Calls | Time |
| --- | --- | --- |
| whole read | `read_all_bytes=1` | 58 µs |
| footer read | `read_range_bytes=1` | 116 ns |
| stream drain | `pstream_bytes=1` | 39 µs |
| whole digest | `read_digest=1` | 65 µs |
| ranged digest | `read_range_digest=1` | 2.9 µs |
| length | `size=1` | 9.7 ns |
| reader drained to the end | `read_all_bytes=1` | 133 µs |
| coding: whole read | `pstream_bytes=1` | 221 µs |
| coding: decoded length | `pstream_bytes=1` | 116 µs |
| cache: warm ranged read | none | 184 ns |
| listing: recursive over 100 | `ls=1` | 156 µs |
| listing: glob over 100 | `bound_location=2 ls=1` | 221 µs |
| IPC: schema | `pstream_bytes=1 url=1 media_type=1 is_container=1 parent=1` | 214 µs |
| IPC: row count | `pread=8 size=1 media_type=2 is_container=2` | 800 ns |
| Avro: schema | `read_all_bytes=1 media_type=1 is_container=1` | 8.8 µs |
| Avro: row count | `pread=2 size=2 media_type=2 is_container=2` | 5.7 µs |
| Parquet: schema | `read_all_bytes=1 size=1 media_type=1 is_container=1` | 13.8 µs |
| Parquet: row count | `read_range_bytes=2 size=2 media_type=2 is_container=2` | 1.2 µs |
| Parquet: full read | `read_all_bytes=1 size=1 media_type=1 is_container=1` | 1.6 ms |

The IPC row count is the one row whose count grows with the value: it is one
`pread` per message, because the walk exists to skip the bodies a read-ahead
window would transfer. Everything else is flat in the size of the value.

## Commands

```bash
cargo test --test iobase_calls --all-features
cargo bench --bench holder --all-features -- calls/ --noplot
```
