# Decode

Paste a captured line and read it: every tag named, every coded value translated, every repeating group nested, and the two self-describing tags checked against the bytes.

## Contract

| | |
| --- | --- |
| Input | A FIX frame in any separator a log writes it with — SOH, `\|`, `^A`, `;`, or one pair per line — with or without a direction verb in front |
| Reads | Splits the pairs, names each key from `assets/fix.json`, translates each value through its code set, gathers occurrences under their counter, recomputes `BodyLength(9)` and `CheckSum(10)` |
| States | Nothing the package did not answer: the names, types, wording, codes and layouts are the generated manifests, and the typed row, digest, facets and anomalies are shown only where the corpus holds the frame |
| Package | [`FixCodec`](capture.md#a-reader-is-the-whole-parse-surface) is the whole parse surface; six entry points, one per shape a capture holds |
| Pages | [Explorer](explorer.md) explores the dictionary, [Encode](encode.md) writes a frame |

## Use

The reader takes a captured line whatever it is wrapped in, and answers a message typed against the dictionary.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let message = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|", false)?;

    assert_eq!(message.as_field().name(), "D");
    assert_eq!(message.by_tag(55)?.as_str(), Some("AAPL"));
    // The row is typed, so a quantity is a number and not the text it arrived as.
    assert_eq!(message.by_tag(38)?.as_f64(), Some(100.0));
    // And it re-emits exactly what arrived, verb taken off.
    assert_eq!(
        message.into_text('|')?,
        "8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|"
    );
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    message = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|")

    assert message.field.name == "D"
    assert message.by_tag(55).as_py() == "AAPL"
    # The row is typed, so a quantity is a number and not the text it arrived as.
    assert message.by_tag(38).as_py() == 100.0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const message = reader.transformLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|'))

    assert.equal(message.field.name, 'D')
    assert.equal(message.byTag(55).toJSON(), 'AAPL')
    assert.equal(message.byTag(38).toJSON(), 100)
    assert.equal(
      Buffer.from(message.toBytes(0x7c)).toString(),
      '8=FIX.4.4|35=D|55=AAPL|54=1|38=100|10=000|',
    )
    ```

## Read a frame

Type or paste. The reading updates as you go, and the presets are the frames the package itself read at build time — pick one and the package's own answer appears under the reading, on the same line.

<div class="ygg-fx" data-fix="decode" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

The separator is taken from the frame rather than asked for: an SOH where there is one, then `^A`, and otherwise whichever of `|`, a newline, `;` or a space reads the most pairs — because a pipe frame that wrapped across two lines holds a newline, and a frame written one pair to a line holds a pipe inside its `Text(58)`. A direction verb in front of the payload is taken off the same way [the classifier](registry.md#a-direction-is-the-verb-in-front-of-the-payload) takes it off, and whatever else a log emitter printed in front of `8=` is dropped with it.

### What is checked

| Check | Rule |
| --- | --- |
| `BodyLength(9)` | The bytes from the one after its own separator to the one before `10=`, counted with the frame's separator written as one SOH |
| `CheckSum(10)` | Every byte up to and including the separator in front of it, summed, modulo 256, three digits |
| Bytes | Both count UTF-8 bytes, which is what a frame carries: `CAFÉ` is five of them and four characters |
| Required | Every tag the message type's layout marks required and the frame did not send |
| Unexplained | Every key no branch of the dictionary names, kept rather than dropped |
| Occurrences | The count a group's counter states against the occurrences that followed it |

### What is not computed here

The typed row, the message digest, the derived facets and the anomalies are the package's, and this page computes none of them. They are shown for a frame that is in the generated corpus, because the manifest carries what the package answered for it; for anything else the page says so and prints the call that answers it.

## Every shape a capture holds

One line per shape, read by the real package at build time: a numeric frame, a bridge frame with name keys, a frame carrying both, an enriched line with no frame at all, and the lines that do not add up. A [bridge configuration document](capture.md#a-bridge-configuration-is-a-dictionary-of-its-own) is the sixth, and is not in this corpus.

<div class="ygg-fx" data-fix="frames" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Edges

- A numeric frame states its group members flat, so the counter holds no occurrences and the miscount is reported rather than repaired. A bridge frame's indexed keys build the occurrences, and the same counter then holds them.
- A tag that merely arrived twice is two values, not a group of one: only a counter states a count.
- A value that will not type is null in the row and still exactly as it arrived in the entries; the refusal is an anomaly, never an error.
- A key no dictionary names is kept under its own spelling. Nothing is dropped for being unexplained.
- The page reads text; the package reads bytes. A frame whose bytes are not text — a `data` field carrying binary — decodes lossily here and is the package's to read properly.

## Commands

```bash
cargo test -p yggdryl --test fix reader::
node --test node/tests/fix/fix.test.js
node scripts/build_docs_fix.js --check
```
