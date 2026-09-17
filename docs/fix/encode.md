# Encode

`FixMsg::into_bytes` emits the message as it now stands, with the chosen
separator: the standard header from its [typed holder](message.md#typed-tags),
the event's own FIX tags, the text, then the [entries](message.md) - the content
row read as a tree, in its order, each value as the wire spells it.

## Contract

| Aspect | Rule |
| --- | --- |
| Source | the typed holders and the content row's [entries](message.md), never the fixed columns |
| Order | tags 8, 35, 49, 56, 34, 43 and 52 - the last only where the message stated it - then the event's own 15, 54, 461, 132, 133, 134, 135, then 58, then the entries pre-order, so a group's members follow the counter that heads them |
| Separator | One byte; SOH by default in Python and JavaScript |
| Values | the value as the wire spells it, a coded fact as its wire code (`54=1`, never `BUY`) and a lane number at the decimal's scale; a derived value is emitted like any other, because the message states it |
| Digest | `digest()` is the XXH3-128 of exactly these bytes, so two messages that re-emit alike digest alike whatever separator either was read with |
| Browser | Displays bytes already emitted by the native package for each stored sample |

## Use

The specialized numeric-frame reader returns one message. Generic captured-line
and bulk readers return [message iterators](capture.md#a-reader-is-the-whole-parse-surface).
A parse fills what the message implies, so the emitted line carries the pairs the
dictionary derived for it behind the ones that arrived; the frame states its
`SendingTime`, so reading the emitted bytes again settles the
[same clocks](capture.md#every-message-is-dated) and emits the same line.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let frame = b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|";
    let message = codec.parse_fix_line(frame)?;
    let emitted = message.into_text('|')?;
    // The header from its holder, then the event's own tags - the side as
    // its wire code, the bid size the order's quantity filled - then the
    // content, and the day order the dictionary derived for it.
    assert_eq!(
        emitted,
        "8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|54=1|59=0|134=100|55=AAPL|10=000|",
    );
    // Emission is idempotent: reading those bytes back emits them again.
    let again = codec.parse_fix_line(emitted.as_bytes())?;
    assert_eq!(again.into_text('|')?, emitted);
    assert_eq!(again.entries(), message.entries());
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    frame = b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|"
    message = codec.parse_fix_line(frame)
    emitted = message.into_text("|")
    # The header from its holder, then the event's own tags - the side as its
    # wire code, the bid size the order's quantity filled - then the content,
    # and the day order the dictionary derived for it.
    assert emitted == "8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|54=1|59=0|134=100|55=AAPL|10=000|"
    # Emission is idempotent: reading those bytes back emits them again.
    again = codec.parse_fix_line(emitted.encode())
    assert again.into_text("|") == emitted
    assert again.entries() == message.entries()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const frame = Buffer.from('8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|')
    const message = codec.parseFixLine(frame)
    const emitted = message.intoText('|')
    // The header from its holder, then the event's own tags - the side as its
    // wire code, the bid size the order's quantity filled - then the content,
    // and the day order the dictionary derived for it.
    assert.equal(emitted, '8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|54=1|59=0|134=100|55=AAPL|10=000|')
    // Emission is idempotent: reading those bytes back emits them again.
    const again = codec.parseFixLine(Buffer.from(emitted))
    assert.equal(again.intoText('|'), emitted)
    assert.deepEqual(again.entries(), message.entries())
    ```

## Inspect emitted bytes

Select a native sample and copy its emitted text. The viewer does not synthesize
new orders, recalculate framing tags or run a separate FIX encoder.

<div class="ygg-fx" data-fix="encode" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Edges

- `BodyLength` and `CheckSum` retain the values that arrived; emission does not
  repair an invalid frame.
- A coded value is emitted as its wire code, never as the name a reader sees:
  `by_tag(54)` answers `BUY` and the wire carries `54=1`.
- A group is emitted as the counter pair valued its occurrence count, then the
  members of each occurrence; the counter column beside the group states the
  same count once and is not a second pair.
- A settled value the codec supplied is the message's own fact rather than a pair it received: a `SendingTime` nothing stated is not emitted, and the identity - `currhashcode`, `curruuid`, `crossuuid` - is emitted nowhere.
- An arrival entry no dictionary resolved carries tag 0 and is emitted under its raw key, exactly where it arrived.
- Direction verbs and surrounding capture prose are outside the emitted frame.
- For streamed Arrow output, [`write_arrow_reader`](arrow.md#back-to-the-wire)
  writes each row's `fixentries` back as one line, with the codec's separator.

## Commands

```bash
cargo test -p yggdryl --test fix
node --test "node/tests/fix/*.test.js"
node scripts/build_docs_fix.js --check
```
