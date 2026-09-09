# Decode

Inspect captured frames and the messages produced by the native FIX codec.
The browser displays committed package results; use the code examples to parse
your own bytes.

## Contract

| Aspect | Rule |
| --- | --- |
| Native intake | `FixCodec::transform_line` accepts captured bytes and returns a lazy `FixMessages` iterator |
| Scalar fields | Values resolve through the field catalog; inline `fix:codes` supplies enum names |
| Repeating groups | The count remains an `int32` field; a named List holds its component occurrences |
| Wire record | Original entries retain order and raw values, including values that fail typed conversion |
| Browser | Shows native sample fields, values, arrivals, emissions and anomalies from `assets/fix.json` |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let frame = b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|";
    let mut messages = codec.transform_line(frame, false)?;
    let message = messages.next().expect("one frame")?;
    assert!(messages.next().is_none());
    assert_eq!(message.by_tag(453)?, &yggdryl::Scalar::from(1_i32));
    assert_eq!(message.by_path("Parties.0.PartyID")?.as_str(), Some("BROKER"));
    assert_eq!(message.into_bytes(b'|'), b"8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|");
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    frame = b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|"
    message, = codec.transform_line(frame)
    assert message.by_tag(453).as_py() == 1
    assert message.by_path("Parties.0.PartyID").as_py() == "BROKER"
    assert message.into_bytes(ord("|")) == frame.removeprefix(b"recv ")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const frame = 'recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|'
    const [message] = codec.transformLine(Buffer.from(frame))
    assert.equal(message.byTag(453).asJs(), 1)
    assert.equal(message.byPath('Parties.0.PartyID').asJs(), 'BROKER')
    assert.equal(Buffer.from(message.intoBytes(124)).toString(), frame.slice(5))
    ```

## Read a frame

Select a sample to inspect the native result. Searches select existing manifest
entries; this page does not parse arbitrary FIX text or reconstruct its schema.

<div class="ygg-fx" data-fix="decode" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

### What is checked

The displayed anomalies are exactly those returned by the package for each
sample. The viewer does not calculate additional checksum, body-length,
required-field or group-count findings. An empty anomaly list is not a venue's
acceptance of an order.

## Every shape a capture holds

The corpus contains numeric and named frames, packed and numeric groups,
derived values and malformed inputs. [Configuration bodies](capture.md#a-bridge-configuration-is-a-dictionary-of-its-own)
use the same native parsing pipeline and can expand to multiple messages.

<div class="ygg-fx" data-fix="frames" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Edges

- A numeric frame states its group members flat, and the dictionary's declaration is what folds them back: the group's first declared member opens an occurrence, a member the occurrence already holds opens the next, and a tag the group does not declare closes it. A bridge frame's indexed keys state the occurrences outright, and the same counter holds them either way. A count the members do not meet is reported rather than repaired, and an ambiguous group context needs a message definition to select the layout.
- A tag that merely arrived twice is two values, not a group of one: only a counter states a count.
- A value that will not type is null in the row and still exactly as it arrived in the entries, ready for emission; the refusal is an anomaly, never an error.
- A key the dictionary does not name is looked for in the message it arrived in before it is kept unexplained: the message root's own children for a flat key, the occurrence's declared members for a packed one. A dialect that spelled one name over two tags has named neither of them in the dictionary, and this is where its own grammar says which of them a key means.
- A row a data field carries is a message of its own type, at its own version, and is read against both. A bridge writes a whole trade capture into a `35=UL` frame's `XmlData`, and `UL` says nothing about the groups that row nests or the spellings its dialect gave two tags; the frame's `BeginString` is the envelope's version and says nothing about which FIX the row was written to, which is routinely a later one than the session speaks. The row states neither, so its type falls to what it declares and its version to the dictionary's newest. A version the caller pinned is the caller speaking for the whole run and answers for the row too, and the frame's own statements stay the frame's.
- A key nothing names at all is kept under its own spelling and its arrival value. Nothing is dropped for being unexplained.
- Bulk and wildcard configuration input yields all selected configurations; empty answers yield none. A conversion error propagates and fuses the cursor.
- The page reads text; the package reads bytes. A frame whose bytes are not text — a `data` field carrying binary — decodes lossily here and is the package's to read properly.

## Commands

```bash
cargo test -p yggdryl --test fix
node --test "node/tests/fix/*.test.js"
node scripts/build_docs_fix.js --check
```
