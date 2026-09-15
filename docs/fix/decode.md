# Decode

Inspect captured frames and the messages produced by the native FIX codec.
The browser displays committed package results; use the code examples to parse
your own bytes.

## Contract

| Aspect | Rule |
| --- | --- |
| Native intake | `FixCodec::parse_line` accepts captured bytes and returns a lazy `FixMessages` iterator - [none, one or many](#a-line-yields-none-one-or-many-messages) of them a line; `parse_lines` streams a whole capture |
| Single frames | `parse_fix_line`, `parse_ullink_line`, `parse_fixml_line` and `parse_pairs` each answer one message and [refuse a body holding a second](#a-line-yields-none-one-or-many-messages) |
| Scalar fields | Values resolve through the field catalog; inline `fix:codes` supplies enum names |
| Repeating groups | The count remains an `int32` field; a named List holds its component occurrences |
| Wire record | Original entries retain order and raw values, including values that fail typed conversion |
| Browser | Shows native sample fields, values, arrivals, emissions and anomalies from `assets/fix.json` |

## Use

One frame, read against the dictionary. A line can carry more than one, and
[what it carries is what it answers](#a-line-yields-none-one-or-many-messages).

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, FieldPath};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let frame = b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|";
    let mut messages = codec.parse_line(frame)?;
    let message = messages.next().expect("one frame")?;
    assert!(messages.next().is_none());
    assert_eq!(message.by_tag(453)?, &yggdryl::Scalar::from(1_i32));
    assert_eq!(message.by_path(&FieldPath::from_str("Parties[0].PartyID")?)?.as_str(), Some("BROKER"));
    assert_eq!(message.into_bytes(b'|'), b"8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|");
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    frame = b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|"
    message, = codec.parse_line(frame)
    assert message.by_tag(453).as_py() == 1
    assert message.by_path("Parties[0].PartyID").as_py() == "BROKER"
    assert message.into_bytes(ord("|")) == frame.removeprefix(b"recv ")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const frame = 'recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|'
    const [message] = codec.parseLine(Buffer.from(frame))
    assert.equal(message.byTag(453).asJs(), 1)
    assert.equal(message.byPath('Parties[0].PartyID').asJs(), 'BROKER')
    assert.equal(Buffer.from(message.intoBytes(124)).toString(), frame.slice(5))
    ```

## A line yields none, one or many messages

A captured line is not a message. It is whatever a relay wrote - a frame, two
frames a batching relay put on one write, a bridge row, a document, or a
sentence with an `=` in it - and `parse_line` answers one message for each the
line carries and nothing for a line that carries none.

| the line holds | the messages |
| --- | --- |
| one frame | one: its first pair to its checksum, the transport's prose in front of it and its remark behind it dropped |
| two frames | two, each re-emitting only its own bytes |
| a frame stating no checksum | one, ending where the next unmarked `8=` opens |
| a bridge row, then a frame | two: the row is a message and the frame the next; a tag run behind a bridge row that opens no frame stays part of it, which is the mixed form a bridge writes |
| a FIXML document | one, whatever prose a transport wrote in front of it |
| a bulk [configuration](capture.md#a-bridge-configuration-is-a-dictionary-of-its-own) | one per configuration a response named, each reading as `pluginconfig` |
| a document naming no configuration | none at all: an error-only answer, a request with no value, a wildcard that selected nothing, or a JSON body that is not a Jolokia answer |
| a sentence | none at all |

What opens a frame is the rule the scanner locates a line's first frame by, read
over the line's own pairs: an unmarked `8=`, and where the rest of the run
states none, an unmarked `35=`. Only an unmarked `8=` closes an open frame,
because a frame's own `35=` stands behind its `8=` and a second `35=` inside it
is a duplicate tag rather than a new message. A key the bridge marked is the
bridge's own spelling, so a `#8=` opens nothing and a `#10=` closes nothing.

A run of named pairs is a bridge row rather than prose carrying an `=` where the
line [named a separator](../media/text/index.md#where-a-pair-ends) for it - a pipe, a
`SOH`, or [a spelling a log escaped one with](capture.md#a-printed-separator-is-still-the-separator),
never whitespace - or where the bridge marked one of its keys with `#`. A
numeric frame needs neither: a run of tag-keyed pairs is FIX whatever separated
it, so a space-separated frame still reads. `ACCOUNT=A1|SIDE=1` is a message and
`After Enrichment -> ACCOUNT=A1 SIDE=1` is a sentence.

`unknown` names a frame, a bridge row or a document that stated no type - never
a line that stated no frame, and never a
[configuration](capture.md#a-bridge-configuration-is-a-dictionary-of-its-own),
whose type the crate states where the document stated none.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // Two frames on one line are two messages, each re-emitting its own bytes.
    let both = b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|";
    let read: Vec<Vec<u8>> = codec
        .parse_line(both)?
        .map(|message| message.map(|message| message.into_bytes(b'|')))
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], b"8=FIX.4.4|35=D|11=A|10=001|");
    assert_eq!(read[1], b"8=FIX.4.4|35=8|37=O1|10=002|");

    // A sentence states no message, whatever `=` it happens to hold.
    assert!(codec.parse_line(b"After Enrichment -> ACCOUNT=A1 SIDE=1")?.next().is_none());

    // The same pairs behind a separator the line named are a bridge row, and a
    // row that stated no type is named `unknown`.
    let row = codec.parse_line(b"ACCOUNT=A1|SIDE=1")?.next().expect("a bridge row")?;
    assert_eq!(row.as_field().name(), "unknown");

    // The single-frame door refuses the body holding both.
    let refused = codec.parse_fix_line(both).unwrap_err();
    assert!(refused.to_string().contains("expected one frame, got a second"), "{refused}");
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # Two frames on one line are two messages, each re-emitting its own bytes.
    both = b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|"
    assert [message.into_bytes(ord("|")) for message in codec.parse_line(both)] == [
        b"8=FIX.4.4|35=D|11=A|10=001|",
        b"8=FIX.4.4|35=8|37=O1|10=002|",
    ]
    # A sentence states no message, whatever `=` it happens to hold.
    assert list(codec.parse_line(b"After Enrichment -> ACCOUNT=A1 SIDE=1")) == []
    # The same pairs behind a separator the line named are a bridge row, and a
    # row that stated no type is named `unknown`.
    row, = codec.parse_line(b"ACCOUNT=A1|SIDE=1")
    assert row.field.name == "unknown"
    # The single-frame door refuses the body holding both.
    with pytest.raises(ValueError, match="expected one frame"):
        codec.parse_fix_line(both)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))

    // Two frames on one line are two messages, each re-emitting its own bytes.
    const both = Buffer.from('8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|')
    const read = [...codec.parseLine(both)].map((message) => Buffer.from(message.intoBytes(124)).toString())
    assert.deepEqual(read, ['8=FIX.4.4|35=D|11=A|10=001|', '8=FIX.4.4|35=8|37=O1|10=002|'])
    // A sentence states no message, whatever `=` it happens to hold.
    assert.equal([...codec.parseLine(Buffer.from('After Enrichment -> ACCOUNT=A1 SIDE=1'))].length, 0)
    // The same pairs behind a separator the line named are a bridge row, and a
    // row that stated no type is named `unknown`.
    const [row] = codec.parseLine(Buffer.from('ACCOUNT=A1|SIDE=1'))
    assert.equal(row.field.name, 'unknown')
    // The single-frame door refuses the body holding both.
    assert.throws(() => codec.parseFixLine(both), /expected one frame/)
    ```

The single-dialect doors take one frame each, so a body holding a second is
refused at the byte that second opens at: `expected one frame, got a second`.
A caller holding two frames holds a row, and a row is what `parse_line` reads.

A payload that was there and would not parse - a malformed document - is the one
case that still answers a message with nothing in it, named `unknown`:
`parse_text_line` and the batch reader answer it rather than fail, because
malformed syntax must never fail the batch it arrives in, while `parse_line` hands
the refusal back. A payload that was not there at all answers no message, and no
bytes at all is the one typed refusal, [not a row](capture.md#lines-are-a-stream).

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
use the same native parsing pipeline and expand to one message for each
configuration they select, exactly as a line expands to one for
[each frame it carries](#a-line-yields-none-one-or-many-messages).

<div class="ygg-fx" data-fix="frames" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## Edges

- A numeric frame states its group members flat, and the dictionary's declaration is what folds them back: the group's first declared member opens an occurrence, a member the occurrence already holds opens the next, and a tag the group does not declare closes it. A bridge frame's indexed keys state the occurrences outright, and the same counter holds them either way. A count the members do not meet is reported rather than repaired - the committed capture's cancel reject states `#NOTRDREGTIMESTAMPS=4` and indexes five occurrences, and reads as five occurrences under a counter of four with the disagreement in `anomalies()` - and an ambiguous group context needs a message definition to select the layout.
- A tag that merely arrived twice is two values, not a group of one: only a counter states a count.
- A value that will not type is null in the row and still exactly as it arrived in the entries, ready for emission; the refusal is an anomaly, never an error - except a settled clock or identity (`SendingTime(52)`, `TransactTime(60)`, `updatedat`, `createdat`, `msghash`, `msgphash`, `snapshotat`, and `code` bytes that are not text), whose unreadable value is a located error item.
- Every generated message carries the version the read used - the codec's target where one is pinned - in the crate's own `version` column, beside the `BeginString` the line itself stated. A read never rewrites what arrived, so the two disagree exactly where the session does.
- A key the dictionary does not name is looked for in the message it arrived in before it is kept unexplained: the message root's own children for a flat key, the occurrence's declared members for a packed one. A dialect that spelled one name over two tags has named neither of them in the dictionary, and this is where its own grammar says which of them a key means.
- `XmlData(213)` is read into the line that carried it, whichever of the two things a bridge writes into it: a row of its own pairs, or the FIXML the tag is named for. Either becomes real fields resolved to real tags rather than one opaque value, a nested element's attributes flattening the way a packed occurrence already does. The field still holds the bytes it arrived as and the wire re-emits them exactly, because a reading of a value is not a second arrival; a document that will not parse fills nothing and the value stays whole.
- A row a data field carries is a message of its own type, at its own version, and is read against both. A bridge writes a whole trade capture into a `35=UL` frame's `XmlData`, and `UL` says nothing about the groups that row nests or the spellings its dialect gave two tags; the frame's `BeginString` is the envelope's version and says nothing about which FIX the row was written to, which is routinely a later one than the session speaks. The row states neither, so its type falls to what it declares and its version to the dictionary's newest. A version the caller pinned is the caller speaking for the whole run and answers for the row too, and the frame's own statements stay the frame's.
- A key nothing names at all is kept under its own spelling and its arrival value, its arrival entry carrying tag 0 - a name or a numeric key alike, since tag 0 is never a registry identity. Nothing is dropped for being unexplained.
- Bulk and wildcard configuration input yields all selected configurations; empty answers yield none. A conversion error propagates and fuses the cursor.
- The page reads text; the package's byte doors - `parse_line`, `parse_lines`, `parse_fix_line`, `parse_ullink_line`, `parse_pairs` - read bytes as given. A frame whose bytes are not text — a `data` field carrying binary — decodes lossily here and is those doors' to read properly. A line the [text reader](../media/text/index.md#a-line-is-text) made was decoded before the codec read it, so through `parse_text_line` and `parse_text_arrow_reader` the codec reads text, and the line's `decoded_byte_size` says whether any byte was decoded.

## Commands

```bash
cargo test -p yggdryl --test fix
node --test "node/tests/fix/*.test.js"
node scripts/build_docs_fix.js --check
```
