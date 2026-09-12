# Lifecycle

A message says what happened; it does not say which order it happened to, beyond the identifiers a venue chose. `FixLifecycle` reads a stream once, in order, and stamps every message with the three identities the stream implies: the instrument, the message itself, and the order chain it belongs to - so a monitor joins an order's whole life on one column rather than rebuilding the chain from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixLifecycle`, `FixCodec::lifecycle`, `INSTID_TAG`, `ID_TAG`, `PERSISTENTID_TAG` |
| Columns | `instid` (65016), `id` (65017), `persistentid` (65018): three of the [crate's own](capture.md#the-crates-own-columns), sixteen bytes each, big-endian |
| `instid` | the xxh128 digest of the instrument's market, classification, ISIN - else symbol - and currency, upper-cased; null where the message names none of them |
| `id` | the instant closest to the market impact, in microseconds, then the xxh3 digest of what the message said: every message has one, and ids sort by time |
| `persistentid` | the instant the chain was created, then the xxh3 digest of its instrument and first identifier; the same on every later message sharing one of the chain's identifiers, null on a message naming no order |
| Chain | joined on `OrigClOrdID(41)`, `ClOrdID(11)`, `OrderID(37)`, `SecondaryClOrdID(526)`, `SecondaryOrderID(198)`, in that order; every identifier a message carries then reaches the chain it joined |
| Ends | a terminal [state](../types/codes.md#a-state-sorts-by-its-lifecycle) - filled, done for day, cancelled, rejected, expired - closes the chain and forgets its identifiers |
| Clock | `TransactTime(60)`, else `SendingTime(52)`, else the row's `timestamp`, else the epoch |
| Stated | a value the message already carries is never overwritten, so a stamped stream read again is a no-op |
| Entries | untouched: the wire re-emits byte for byte |
| Bindings | Rust; Python (`FixLifecycle`, `FixCodec.lifecycle`); JavaScript (`FixLifecycle`, `FixCodec.lifecycle`) |

## Use

One order's life, six messages long, on one persistent identity.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixLifecycle, FixRegistry, ID_TAG, INSTID_TAG, PERSISTENTID_TAG};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut life = FixLifecycle::new(Arc::clone(&registry));

    // The order, its acknowledgement under the venue's own identifier, a
    // replace naming the old identifier, and the fill under the new one.
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:33.000|10=0|",
    ];
    let mut stamped = Vec::new();
    for line in lines {
        stamped.push(life.fill(reader.parse_line(line)?.next().expect("one frame")?)?);
    }

    // One chain from the order to the fill, whatever identifier each
    // message chose, and one instrument.
    let chain = stamped[0].by_tag(PERSISTENTID_TAG)?;
    assert!(stamped.iter().all(|held| held.get_by_tag(PERSISTENTID_TAG) == Some(chain)));
    let instrument = stamped[0].by_tag(INSTID_TAG)?;
    assert!(stamped.iter().all(|held| held.get_by_tag(INSTID_TAG) == Some(instrument)));

    // Ids sort by the market's own clock.
    let ids: Vec<&[u8]> = stamped.iter().map(|held| held.by_tag(ID_TAG).unwrap().as_bytes().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    // The fill ended the chain: nothing is alive, and the wire is untouched.
    assert_eq!(life.alive(), 0);
    assert_eq!(stamped[3].into_bytes(b'|'), lines[3]);

    // Over an iterator, the codec runs one lifecycle for the whole stream.
    let again: Vec<_> = reader
        .lifecycle(lines.iter().map(|line| {
            reader.parse_line(line).unwrap().next().expect("one frame").unwrap()
        }))
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(again[3].by_tag(PERSISTENTID_TAG)?, chain);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixLifecycle, FixRegistry

    INSTID, ID, PERSISTENTID = 65016, 65017, 65018
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    life = FixLifecycle(registry)

    # The order, its acknowledgement under the venue's own identifier, a
    # replace naming the old identifier, and the fill under the new one.
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|60=20260102-10:15:32.000|10=0|",
        b"8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:33.000|10=0|",
    ]
    stamped = [life.fill(next(reader.parse_line(line))) for line in lines]

    # One chain from the order to the fill, whatever identifier each
    # message chose, and one instrument.
    chain = stamped[0].by_tag(PERSISTENTID)
    assert all(held.get_by_tag(PERSISTENTID) == chain for held in stamped)
    instrument = stamped[0].by_tag(INSTID)
    assert all(held.get_by_tag(INSTID) == instrument for held in stamped)

    # Ids sort by the market's own clock.
    ids = [held.by_tag(ID).as_py() for held in stamped]
    assert all(earlier < later for earlier, later in zip(ids, ids[1:]))

    # The fill ended the chain: nothing is alive, and the wire is untouched.
    assert life.alive() == 0
    assert stamped[3].into_bytes(ord("|")) == lines[3]

    # Over an iterable, the codec runs one lifecycle for the whole stream,
    # answering it lazily.
    again = list(reader.lifecycle(next(reader.parse_line(line)) for line in lines))
    assert again[3].by_tag(PERSISTENTID) == chain
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const life = new fix.FixLifecycle(registry)

    // The order, its acknowledgement under the venue's own identifier, a
    // replace naming the old identifier, and the fill under the new one.
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|60=20260102-10:15:30.000|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|207=XNAS|15=USD|54=1|38=120|60=20260102-10:15:32.000|10=0|',
      '8=FIX.4.4|35=8|11=A2|150=F|39=2|14=120|151=0|55=AAPL|207=XNAS|15=USD|60=20260102-10:15:33.000|10=0|',
    ]
    const stamped = lines.map((line) => life.fill(reader.parseLine(Buffer.from(line)).next().value))

    // One chain from the order to the fill, whatever identifier each
    // message chose, and one instrument.
    const chain = stamped[0].byTag(65018)
    assert.ok(stamped.every((held) => held.byTag(65018).equals(chain)))
    const instrument = stamped[0].byTag(65016)
    assert.ok(stamped.every((held) => held.byTag(65016).equals(instrument)))

    // Ids sort by the market's own clock.
    const ids = stamped.map((held) => Buffer.from(held.byTag(65017).asJs()))
    assert.ok(ids.every((id, at) => at === 0 || Buffer.compare(ids[at - 1], id) < 0))

    // The fill ended the chain: nothing is alive, and the wire is untouched.
    assert.equal(life.alive, 0)
    assert.equal(stamped[3].intoBytes('|'.charCodeAt(0)).toString(), lines[3])

    // Over any iterable, the codec runs one lifecycle for the whole stream,
    // answering it lazily.
    const again = [...reader.lifecycle(lines.map((line) => reader.parseLine(Buffer.from(line)).next().value))]
    assert.ok(again[3].byTag(65018).equals(chain))
    ```

## The chain is the identifiers, joined

An order is created under one `ClOrdID`, acknowledged under an `OrderID`, replaced under a new `ClOrdID` that names the old one as `OrigClOrdID`, and filled under whichever of them the venue chose to echo. A message joins a chain through any identifier it carries - `OrigClOrdID` first, because a replace or a cancel names the order it acts on there and the new `ClOrdID` it carries is not yet anyone's - and every identifier it carries then reaches that chain, so the replace's new `ClOrdID` joins the order the old one opened. A message carrying an identifier no chain holds opens one, dated by its own impact clock; a message carrying none, a heartbeat or a logon, gets an `id` and no chain.

Two tags spelling one identifier are one key: an order acknowledged under the client's own identifier joins nothing to itself.

## A chain ends when its state does

The state a message reports - the crate's own `state`, else `OrdStatus`, else `ExecType` - ranks its lifecycle, and a rank past the live ones closes the chain: its identifiers are forgotten, so a venue reusing a `ClOrdID` tomorrow opens a new chain rather than joining yesterday's. What is held is therefore the orders still alive, `alive()` says how many, and `clear()` forgets them all, as a new session or a new day would. A closed chain's slot is reused, so the state stays the size of the busiest moment rather than the whole capture.

## The impact clock

`TransactTime(60)` is when the venue says it happened; `SendingTime(52)` when the message left; the row's own `timestamp` when the capture saw it. The first stated is the instant an `id` and a `persistentid` open with, in microseconds, so a consumer ordering by id orders by the market's own clock where one was stated, and two reads of one capture agree on every identity, because nothing here reads a wall clock.

## In a batch read

The lifecycle is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call), and a stage is a call: `codec.arrow_reader(schema, codec.lifecycle(codec.messages(reader)))` runs one `FixLifecycle` over the whole read, so the `persistentid` a row carries depends on the rows before it - which is what a chain is. Nothing stamps unasked, for the reason nothing enriches unasked: a stamped value is indistinguishable from a stated one. The example [there](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) lands one order's life in a batch on one chain.

## Edges

- A message stating its own `instid`, `id` or `persistentid` keeps it; a terminal message still closes the chain its identifiers reach, so a stamped stream read again ends where the first read ended.
- The same line at the same instant is the same `persistentid` and the same `id`: the identities are digests, not sequence numbers, and never depend on when the pass ran.
- An ISIN outranks a symbol in the instrument's identity, and case does not tell two instruments apart; another market does.
- A bridge row names the same facts under its own keys - `#ISINCODE`, `#LASTMKT`, `#CURRENCY`, `CLORDID` - and reaches the same identities.
- A state a venue spells outside the vocabulary is not a state and ends nothing.
- A registry without the crate's three columns - none this crate builds - stamps nothing and passes the message through.
