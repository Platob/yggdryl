# Encode

Compose a frame from a message type's own layout: required fields first, coded values by name, and the two self-describing tags computed from the bytes as you type.

## Contract

| | |
| --- | --- |
| Offers | From `assets/fix.json`: the fields its layout requires, those plus the ones a frame always carries, or the ones the message declares itself — and any other by tag or by name |
| Values | A field carrying a code set becomes a list of its codes, each shown as the specification words it |
| Writes | Header tags in the standard header's own order, then the body, then the [two self-describing tags](decode.md#what-is-checked) over the bytes it wrote |
| Separator | SOH for the wire, `\|` for reading, `^A` for a log; the arithmetic is always over SOH, whatever is shown |
| Package | [`FixMsg::to_bytes`](message.md) re-emits the wire record from a message's entries; a composed frame is checked against it by round-tripping through [Decode](decode.md) |

## Use

A message re-emits exactly what arrived, so a frame written by hand and read back must match byte for byte.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixReader, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixReader::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let frame = "8=FIX.4.4|9=56|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|10=043|";

    let message = reader.text(frame)?;
    // The emit is the wire record, so a translated code cannot leak into it.
    assert_eq!(message.into_text('|')?, frame);
    // Read back, the round trip is the same message and not merely the same text.
    assert_eq!(reader.text(&message.into_text('|')?)?, message);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixReader, FixRegistry

    reader = FixReader(FixRegistry.from_handle(Path("config/fix").resolve()))
    frame = "8=FIX.4.4|9=56|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|10=043|"

    message = reader.text(frame)
    assert message.to_bytes(ord("|")) == frame.encode()
    assert reader.text(frame) == message
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixReader(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const frame = '8=FIX.4.4|9=56|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|10=043|'

    const message = reader.text(frame)
    assert.equal(Buffer.from(message.toBytes(0x7c)).toString(), frame)
    assert.ok(reader.text(frame).equals(message))
    ```

## Write a frame

Pick a message type. The form is its layout, and the frame under it is rebuilt on every keystroke, with `BodyLength` and `CheckSum` recomputed each time. Read it back in the decoder when it looks right.

<div class="ygg-fx" data-fix="encode" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

### How the frame is ordered

| Position | Holds |
| --- | --- |
| First | `BeginString(8)`, then `BodyLength(9)`, then `MsgType(35)` — the three the specification fixes |
| Then | The rest of the standard header, in the order the header component declares |
| Then | The body, by tag |
| Last | `CheckSum(10)` |

Both are computed here rather than asked for, by the [rule the decoder checks them against](decode.md#what-is-checked).

## Edges

- The composer writes flat tags. A repeating group is written by sending its counter and then its members, which is what a numeric frame does on the wire; the [decoder](decode.md) gathers them back under the counter.
- A field with more than sixty codes stays a text box rather than becoming a list nobody can scroll.
- `MsgType(35)` follows the chosen message type and cannot be edited away from it.
- An empty value is not sent. `54=` is a malformed message, not an absent side, and the package refuses to read one.
- A composed frame is not a validated message. It says what it says; whether a venue accepts it is the venue's answer.

## Commands

```bash
cargo test -p yggdryl --test fix reader::a_message_re_emits
node --test node/tests/fix/fix.test.js
node scripts/build_docs_fix.js --check
```
