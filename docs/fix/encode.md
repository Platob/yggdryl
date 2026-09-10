# Encode

`FixMsg::into_bytes` emits the message's original arrival record with the chosen
separator. The typed row is a projection; emission uses the retained raw values
and their wire order.

## Contract

| Aspect | Rule |
| --- | --- |
| Source | Original entries held by a parsed `FixMsg` |
| Order | A group's members follow its scalar counter in arrival order |
| Separator | One byte; SOH by default in Python and JavaScript |
| Values | Original wire values, including values represented as null in the typed row |
| Browser | Displays bytes already emitted by the native package for each stored sample |

## Use

The specialized numeric-frame reader returns one message. Generic captured-line
and bulk readers return [message iterators](capture.md#a-reader-is-the-whole-parse-surface).

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let frame = b"8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|";
    let message = codec.parse_fix_line(frame)?;
    let emitted = message.into_bytes(b'|');
    assert_eq!(emitted, frame);
    assert_eq!(codec.parse_fix_line(&emitted)?, message);
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    frame = b"8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|"
    message = codec.parse_fix_line(frame, separator=ord("|"))
    emitted = message.into_bytes(ord("|"))
    assert emitted == frame
    assert codec.parse_fix_line(emitted, separator=ord("|")) == message
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const frame = Buffer.from('8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|')
    const message = codec.parseFixLine(frame, 124)
    const emitted = Buffer.from(message.intoBytes(124))
    assert.deepEqual(emitted, frame)
    assert.ok(codec.parseFixLine(emitted, 124).equals(message))
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
- Enum display names in a typed row do not replace the original wire codes.
- Direction verbs and surrounding capture prose are outside the emitted frame.
- For streamed Arrow output, [`write_arrow_reader`](arrow.md#back-to-the-wire)
  writes each row's `nofixentries` back as one line, with the codec's separator.

## Commands

```bash
cargo test -p yggdryl --test fix
node --test "node/tests/fix/*.test.js"
node scripts/build_docs_fix.js --check
```
