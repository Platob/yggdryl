# Lifecycle

A message says what happened; it does not say which event it belongs to beyond the identifiers a venue chose. `FixLifecycle` reads a stream once, in arrival order, names the chain each message joins by its `code`, carries the chain's first creation instant and previous message, and lands every message on a deterministic time grid - so a monitor joins an order's whole life on `msgphash` and reads one snapshot per bucket rather than rebuilding the chain from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixLifecycle` (`DEFAULT_INTERVAL_NS`, `interval_ns`, `set_interval_ns`, `try_with_interval_ns`, `fill`, `snapshot`, `snapshots`, `alive`, `clear`), `FixCodec::lifecycle` |
| Columns | the [crate's own](capture.md#the-crates-own-columns) `code` (65024), `updatedat` (65003), `createdat` (65023), `prevupdatedat` (65021), `prevmsghash` (65022) and `instuuid` (65016); `msgphash` (65018) and `msghash` (65017) are recomputed by the message's identity owner after every stamp. The four identity columns are `fixedbinary(16)`, and a stated one of another width, layout or family is a located refusal naming its column |
| Chain name | a non-empty stated `code` selects its live chain globally; else the first identifier reaching a live chain supplies that chain's code; else the first identifier names a new chain `<scope>/<identifier>`, the scope rendered `-` when absent; no identifier leaves `code` empty and opens no chain |
| Identifiers | a stated `altids` Map, else the message type's compiled [`fix:identifiers`](registry.md#component-identifiers) selection, in sorted member-name order; each keyed by the effective instrument scope: a stated `instuuid`, else the sixteen big-endian bytes of the xxh128 digest of market, CFI, ISIN - else symbol - and currency, else absent |
| `msgphash` | the sixteen big-endian bytes of the XXH3-128 over the exact `code` bytes, so a chain's identity is its name; empty code hashes empty bytes and never opens a chain |
| Grid | `updatedat` becomes `floor(t / interval) * interval` of the settled event clock, Euclidean and checked; `snapshotat` keeps the real instant; the interval is positive nanoseconds, `DEFAULT_INTERVAL_NS` (one second) unless set, and changes only while no chain is live |
| Creation | every message joining a live chain carries the `createdat` of that chain's first accepted message |
| History | `prevupdatedat` and `prevmsghash` are the previous accepted message's `updatedat` and `msghash` in the selected chain, null on a first message; a stated non-null value stays |
| Snapshots | `snapshot` answers the full message only for an off-grid arrival that opens a chain or lands above its live chain's highest consumed bucket; an aligned arrival consumes its bucket silently; `snapshots` filters a stream the same way |
| Ends | a terminal [state](../types/codes.md#a-state-sorts-by-its-lifecycle) - the crate's `state`, else `OrdStatus(39)`, else `ExecType(150)` - is stamped, then closes the chain and forgets its identifiers |
| State | live chains only: code, first `createdat`, last `updatedat` and `msghash`, highest bucket, attached identifiers; no pending message, timer or tombstone |
| Atomic | a refusal - an unrepresentable grid instant, a malformed stated `altids`, `instuuid` or previous value, a code-hash collision, a mistyped stamp target - is located and changes no chain, history, bucket or identifier |
| Entries | untouched: entries, wire and arrival digest are what arrived |
| Bindings | Rust; Python `FixLifecycle(registry, *, interval_ns)`, `FixCodec.lifecycle`; JavaScript `new fix.FixLifecycle(registry, { intervalNs })`, `FixCodec.lifecycle` |

## Use

One order's life on one chain: the order, its acknowledgement under the venue's identifier, a replace naming the old client identifier, and the fill under the new one.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{
        CODE_TAG_NAME, FixCodec, FixLifecycle, FixMsg, FixRegistry, PREVUPDATEDAT_TAG_NAME,
        PREVMSGHASH_TAG_NAME, SNAPSHOTAT_TAG_NAME, TimeUnit,
    };

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.100|10=0|",
    ];

    let mut life = FixLifecycle::new(Arc::clone(&registry));
    assert_eq!(life.interval_ns(), FixLifecycle::DEFAULT_INTERVAL_NS);
    let mut stamped: Vec<FixMsg> = Vec::new();
    for line in lines {
        stamped.push(life.fill(reader.parse_fix_line(line)?)?);
    }

    // One chain, named by the first identifier under the instrument scope,
    // whatever identifier each message chose.
    let code = stamped[0].by_tag(CODE_TAG_NAME.0)?.as_str().expect("a named chain");
    assert!(code.ends_with("/A1"), "{code}");
    assert!(stamped.iter().all(|held| held.msgphash() == stamped[0].msgphash()));
    // The first creation instant travels with the chain, and each message
    // names the one before it.
    let created = stamped[0].by_tag(60)?;
    assert!(stamped.iter().all(|held| held.createdat() == created));
    assert!(stamped[0].by_tag(PREVMSGHASH_TAG_NAME.0)?.is_null());
    assert_eq!(stamped[1].by_tag(PREVMSGHASH_TAG_NAME.0)?, stamped[0].msghash());
    assert_eq!(stamped[1].by_tag(PREVUPDATEDAT_TAG_NAME.0)?, stamped[0].updatedat());
    // updatedat lands on the one-second grid; snapshotat keeps the event.
    assert_eq!(stamped[1].updatedat().temporal_count_at(TimeUnit::Millisecond), Some(1_767_348_930_000));
    assert_eq!(stamped[1].by_tag(SNAPSHOTAT_TAG_NAME.0)?, stamped[1].by_tag(60)?);
    // The fill closed the chain, and the wire is untouched.
    assert_eq!(life.alive(), 0);
    assert_eq!(stamped[3].into_bytes(b'|'), lines[3]);

    // A fresh lifecycle replaying the filled stream answers it unchanged.
    let mut replay = FixLifecycle::new(Arc::clone(&registry));
    for held in &stamped {
        assert_eq!(&replay.fill(held.clone())?, held);
    }

    // Snapshots: 30.250 opens bucket 30, 30.500 repeats it, 32.000 is aligned
    // and consumes bucket 32 silently, 33.100 opens bucket 33.
    let snapshots: Vec<FixMsg> = FixLifecycle::new(Arc::clone(&registry))
        .snapshots(reader.parse_lines(lines))
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].updatedat(), stamped[0].updatedat());
    assert_eq!(snapshots[1].updatedat(), stamped[3].updatedat());
    let aligned = reader.parse_fix_line(lines[2])?;
    assert!(FixLifecycle::new(Arc::clone(&registry)).snapshot(aligned)?.is_none());

    // The codec's stream door runs one default-cadence lifecycle.
    let again: Vec<FixMsg> = reader.lifecycle(reader.parse_lines(lines)).collect::<yggdryl::Result<_>>()?;
    assert!(again.iter().all(|held| held.msgphash() == stamped[0].msgphash()));
    // The interval is positive nanoseconds.
    assert_eq!(FixLifecycle::new(Arc::clone(&registry)).try_with_interval_ns(2_000_000_000)?.interval_ns(), 2_000_000_000);
    assert!(FixLifecycle::new(registry).try_with_interval_ns(0).is_err());
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixLifecycle, FixRegistry

    PREVUPDATEDAT, PREVUUID, CODE, SNAPSHOTAT = 65021, 65022, 65024, 65025
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.100|10=0|",
    ]

    life = FixLifecycle(registry)
    assert life.interval_ns == FixLifecycle.DEFAULT_INTERVAL_NS == 1_000_000_000
    stamped = [life.fill(reader.parse_fix_line(line)) for line in lines]

    # One chain, named by the first identifier under the instrument scope,
    # whatever identifier each message chose.
    assert stamped[0].by_tag(CODE).as_py().endswith("/A1")
    assert all(held.msgphash() == stamped[0].msgphash() for held in stamped)
    # The first creation instant travels with the chain, and each message
    # names the one before it.
    assert all(held.createdat() == stamped[0].by_tag(60) for held in stamped)
    assert stamped[0].by_tag(PREVUUID).as_py() is None
    assert stamped[1].by_tag(PREVUUID) == stamped[0].msghash()
    assert stamped[1].by_tag(PREVUPDATEDAT) == stamped[0].updatedat()
    # updatedat lands on the one-second grid; snapshotat keeps the event.
    assert stamped[1].updatedat().as_py() == datetime(2026, 1, 2, 10, 15, 30, tzinfo=timezone.utc)
    assert stamped[1].by_tag(SNAPSHOTAT) == stamped[1].by_tag(60)
    # The fill closed the chain, and the wire is untouched.
    assert life.alive() == 0
    assert stamped[3].into_bytes(ord("|")) == lines[3]

    # A fresh lifecycle replaying the filled stream answers it unchanged.
    replay = FixLifecycle(registry)
    assert [replay.fill(held) for held in stamped] == stamped

    # Snapshots: 30.250 opens bucket 30, 30.500 repeats it, 32.000 is aligned
    # and consumes bucket 32 silently, 33.100 opens bucket 33.
    snapshots = list(FixLifecycle(registry).snapshots(reader.parse_lines(lines)))
    assert [held.updatedat() for held in snapshots] == [stamped[0].updatedat(), stamped[3].updatedat()]
    assert FixLifecycle(registry).snapshot(reader.parse_fix_line(lines[2])) is None

    # The codec's stream door runs one default-cadence lifecycle.
    assert all(held.msgphash() == stamped[0].msgphash() for held in reader.lifecycle(reader.parse_lines(lines)))
    # The interval is positive nanoseconds.
    assert FixLifecycle(registry, interval_ns=2_000_000_000).interval_ns == 2_000_000_000
    with pytest.raises(ValueError):
        FixLifecycle(registry, interval_ns=0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const [PREVUPDATEDAT, PREVUUID, CODE, SNAPSHOTAT] = [65021, 65022, 65024, 65025]
    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|54=1|38=120|60=20260102-10:15:32.000|10=0|',
      '8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|60=20260102-10:15:33.100|10=0|',
    ].map((line) => Buffer.from(line))

    const life = new fix.FixLifecycle(registry)
    assert.equal(life.intervalNs, fix.FixLifecycle.DEFAULT_INTERVAL_NS)
    assert.equal(life.intervalNs, 1_000_000_000n)
    const stamped = lines.map((line) => life.fill(reader.parseFixLine(line)))

    // One chain, named by the first identifier under the instrument scope,
    // whatever identifier each message chose.
    assert.ok(stamped[0].byTag(CODE).asJs().endsWith('/A1'))
    assert.ok(stamped.every((held) => held.msgphash().equals(stamped[0].msgphash())))
    // The first creation instant travels with the chain, and each message
    // names the one before it.
    assert.ok(stamped.every((held) => held.createdat().equals(stamped[0].byTag(60))))
    assert.equal(stamped[0].byTag(PREVUUID).asJs(), null)
    assert.ok(stamped[1].byTag(PREVUUID).equals(stamped[0].msghash()))
    assert.ok(stamped[1].byTag(PREVUPDATEDAT).equals(stamped[0].updatedat()))
    // updatedat lands on the one-second grid; snapshotat keeps the event.
    assert.ok(stamped[1].updatedat().equals(stamped[0].updatedat()), 'bucket 10:15:30')
    assert.ok(stamped[1].byTag(SNAPSHOTAT).equals(stamped[1].byTag(60)))
    // The fill closed the chain, and the wire is untouched.
    assert.equal(life.alive, 0)
    assert.deepEqual(Buffer.from(stamped[3].intoBytes(124)), lines[3])

    // A fresh lifecycle replaying the filled stream answers it unchanged.
    const replay = new fix.FixLifecycle(registry)
    assert.ok(stamped.every((held) => replay.fill(held.clone()).equals(held)))

    // Snapshots: 30.250 opens bucket 30, 30.500 repeats it, 32.000 is aligned
    // and consumes bucket 32 silently, 33.100 opens bucket 33.
    const snapshots = [...new fix.FixLifecycle(registry).snapshots(reader.parseLines(lines))]
    assert.equal(snapshots.length, 2)
    assert.ok(snapshots[0].updatedat().equals(stamped[0].updatedat()))
    assert.ok(snapshots[1].updatedat().equals(stamped[3].updatedat()))
    assert.equal(new fix.FixLifecycle(registry).snapshot(reader.parseFixLine(lines[2])), null)

    // The codec's stream door runs one default-cadence lifecycle.
    assert.ok([...reader.lifecycle(reader.parseLines(lines))].every((held) => held.msgphash().equals(stamped[0].msgphash())))
    // The interval is positive nanoseconds.
    assert.equal(new fix.FixLifecycle(registry, { intervalNs: 2_000_000_000n }).intervalNs, 2_000_000_000n)
    assert.throws(() => new fix.FixLifecycle(registry, { intervalNs: 0n }))
    ```

## A chain is named by its code

A stated non-empty `code` is the chain's name and selects it globally, across instrument scopes. A message stating none joins through its identifiers: the first one - in the sorted member-name order of `altids` - that a live chain already owns under the same instrument scope supplies that chain's code, so the replace's `OrigClOrdID` reaches the order its new `ClOrdID` does not, and every identifier the message carries then attaches to that chain unless another live chain already owns it. A message whose identifiers reach no chain names a new one after its first identifier, `<scope>/<identifier>`, the scope spelled as the thirty-two lowercase hex digits of its sixteen bytes; one with neither a code nor an identifier - a heartbeat, a logon - keeps an empty code, still gets its `msghash` and a `msgphash` over empty bytes, and opens nothing.

Two explicit codes never merge and never steal each other's identifiers, and a generated code whose hash meets a live chain of another name is a located `$.msgphash` refusal. These deterministic hashes are not collision-free; they are what makes two reads of one capture agree without a wall clock.

## A chain carries its creation and its history

The first accepted message of a live chain fixes its `createdat`: first by arrival, not the minimum or the grid, and a later statement does not replace it. Every later message selecting the chain - late, aligned, suppressed or terminal - carries it. `prevupdatedat` and `prevmsghash` are the previous accepted message's `updatedat` and `msghash`, filled independently where null and kept where stated; they follow every accepted message, so a previous identity may name a message the snapshot stream filtered out. A late arrival moves history backward without lowering the chain's highest bucket.

## Snapshots are a grid, not a timer

`updatedat` is truncated to `floor(t / interval) * interval` of the settled event clock and exact boundaries open their bucket; `snapshotat` keeps the real instant, so truncation can put `updatedat` before `createdat`. `fill` answers every message; `snapshot` answers the same transition's message only when it arrived off-grid and opened its chain or landed in a bucket above the chain's highest consumed one, and `None` otherwise - an aligned arrival consumes its bucket without emitting, and an unnamed message never emits. `snapshots` owns a configured lifecycle over a stream of `Result` messages and drops only those successful `None` answers; nothing is pending, no timer fires and no bucket is backfilled.

The interval is settled before the stream: `set_interval_ns` refuses zero, a negative value, or a change while a chain is live, and repeating the current interval is a no-op. `clear` forgets chains and keeps the interval.

## A chain ends when its state does

A terminal state - filled, done for day, cancelled, rejected, expired - is stamped with the chain's creation and history, then closes the chain and forgets its identifiers, suppressed or not, so a venue reusing a `ClOrdID` tomorrow opens a new chain. Reopening a code keeps its `msgphash` and starts a fresh incarnation, which may emit again in the same bucket. What is held is therefore the live chains; `alive()` counts them and `clear()` forgets them, as a new session or a new day would.

## In a batch read

The lifecycle is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call), and a stage is a call: `codec.arrow_reader(schema, codec.lifecycle(codec.messages(reader)))` fills a whole read, and `codec.arrow_reader(schema, life.snapshots(codec.messages(reader)))` lands only its snapshots. `lifecycle` takes owned messages or their `Result`s and `snapshots` takes a stream of `Result`s (Python and JavaScript accept any iterable of messages); both yield a source or transition error as an item without advancing state, and fuse only exhaustion. Nothing stamps unasked, for the reason nothing enriches unasked: a stamped value is indistinguishable from a stated one.

## Edges

- The same line at the same instant is the same `code` and `msgphash`: the identities are digests of settled values, never sequence numbers, and a fresh or cleared lifecycle replaying a raw or an already-filled stream answers the same messages.
- Feeding an earlier message into an advanced lifecycle is a new arrival, not a rewind.
- An ISIN outranks a symbol in the instrument scope, and case does not tell two instruments apart; another market does. A bridge row naming the same facts under its own keys reaches the same scope.
- A stated `altids` is authoritative, an empty one included; its keys must be unique and ascending and its values text or null, else a located refusal. A null or empty identifier contributes nothing; text is neither trimmed nor case-folded.
- A state a venue spells outside the vocabulary is not a state and ends nothing. A terminal message opening no live chain keeps its own `createdat`, may emit its off-grid snapshot, and leaves no chain behind.
- A stated `msghash` or `msgphash` must match what the finalized message computes; `code` is what a caller states to name a chain.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix lifecycle
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix -k lifecycle
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/fix/*.test.js"
    ```
