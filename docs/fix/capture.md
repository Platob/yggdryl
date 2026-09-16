# Capture

A day of session log is a table. This page is the road from one to the other: [`FixCodec`](#a-reader-is-the-whole-parse-surface) turns a captured line into the [messages](message.md) it holds, [`fix_schema`](#the-columns-are-the-folded-names) is the one row shape every message answers as, and [`FixMsg::into_row`](#a-column-is-filled-by-the-tag-its-field-carries) fills it - by the tag each column's field carries, so nothing is resolved against the [dictionary](registry.md) per row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec` and its `parse_*` readers, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `fix_column_of`, `fix_column_tags`, `FixMsg::into_row`, `fix_crate_fields` |
| Columns | named by the field's folded canonical name - `msgtype`, never `35` and never `msg_type`; the display spelling stays on the field's `display`, the tag on its `fix:tag`, and a named group column's counter on its `fix:counter` |
| Shape | the crate's own clocks and identities, standard header, the fields a consumer reads, three List groups, the trailer, the crate's other scalar fields with the `altids` Map group and the `instids` Struct, FIX's own `msgdirection`, then the one `fixentries` group under the `nofixentries` that counts it: 116 tags from `fix_schema_tags`, 120 columns with the shipped registry, each List group adding its column beside its counter |
| Identifiers | enrichment fills the nullable, sorted `altids` Map from the message's direct `fix:identifiers`; a stated map is preserved, including an empty one |
| Non-null | `beginstring`, `sendingtime`, `updatedat`, `msghash`, `msgphash`, `createdat`, `code`; `snapshotat` is held by the replay bundle but nullable, because only a snapshot stamps it, and `version` is populated at construction while its column remains nullable |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `fixentries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Expansion | a line yields one message per [frame it carries](decode.md#a-line-yields-none-one-or-many-messages) and none where it carries none; a [JSON document](#a-json-document-is-one-message-stating-nothing) yields exactly one, named `unknown`, whatever the document names |
| Refuses | no parseable row's content: a payload that was there and would not parse is a message with nothing in it, so it never fails the batch it arrives in; a stated mandatory clock that is not an instant is a located error item. The row count is the capture's messages rather than its lines |
| Found | a column is `index_of("msgtype")` on the schema itself, and `fix_column_of(&schema, 35)` is the same position read off the column's own `fix:tag`; nothing is cached, resolved or invalidated |

## Use

One line in, one row per message out, with the columns named as the dictionary names the fields.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    let schema = fix_schema(&registry, "FixMessage")?;
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut messages = reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")?;
    let order = messages.next().expect("one frame")?;
    assert!(messages.next().is_none());

    let row = order.into_row(&schema)?;
    let held = row.as_sequence().expect("a row");
    let at = schema.index_of("msgtype").expect("the msgtype column");
    assert_eq!(held[at].as_str(), Some("D"));

    // The last column is every pair that arrived; a key no dictionary
    // explains is an entry of tag 0 under its raw key.
    let entries = held.last().and_then(Scalar::as_sequence).expect("the arrival record");
    assert_eq!(entries.len(), 7);
    let unresolved: Vec<_> = order.entries().iter().filter(|entry| entry.tag() == 0).collect();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].key().as_str(), Some("9999"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    schema = fix_schema(registry, "FixMessage")
    reader = FixCodec(registry)

    order, = reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")
    row = order.into_row(schema).as_py()

    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("symbol")] == "AAPL"
    # The last column is every pair that arrived; a key no dictionary
    # explains is an entry of tag 0 under its raw key.
    assert len(row[-1]) == 7
    assert [entry for entry in order.entries() if entry[0] == 0] == [(0, "9999", "x")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')
    const reader = new fix.FixCodec(registry)

    const [order] = reader.parseLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|'))
    const row = order.intoRow(schema).toJSON()

    assert.equal(row[schema.indexOf('msgtype')], 'D')
    assert.equal(row[schema.indexOf('symbol')], 'AAPL')
    // The last column is every pair that arrived; a key no dictionary
    // explains is an entry of tag 0 under its raw key.
    assert.equal(row[row.length - 1].length, 7)
    assert.deepEqual(order.arrivals().filter(([tag]) => tag === 0), [[0, '9999', 'x']])
    ```

## Try it

The [Decode](decode.md) page displays native results for the committed frame corpus. Run the examples here to parse other input.

## Find a column

More columns than anyone scrolls, and the question a reader actually has is *which column holds this*. The filter matches the tag, the field name and the wording alike.

<div class="ygg-fx" data-fix="row" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## A reader is the whole parse surface

The verb is `parse`, and no reader takes a flag: what happens to a message once it is built - [filling](#what-a-message-implied-is-filled-in), [restating](message.md#restated-under-the-dictionary), [stamping](lifecycle.md) - is a call over the stream, never an argument to the parse.

| Reader | Takes | Answers |
| --- | --- | --- |
| `parse_line` | one captured line, the verb and prose around the frame included | `FixMessages`, a lazy fallible iterator: [none, one or many](decode.md#a-line-yields-none-one-or-many-messages) - one per frame, one for a JSON document, none for a line that states no message |
| `parse_lines` | any iterator of lines | a lazy iterator of `Result<FixMsg>`; a line that is not a row is an `Err` item and the stream continues |
| `parse_text_line` | one [decoded line](../media/text/index.md#row-schema), its body and [row-header captures](arrow.md#a-column-is-the-caller-speaking-per-row) | `FixMessages` |
| `parse_text_lines` | any iterator of owned or borrowed lines, or `Result`s of them | a lazy iterator of `Result<FixMsg>`; lines are borrowed without cloning and a source error is moved into the stream unchanged |
| `parse_text_arrow_reader` | a `BatchReader` of text records | a `BatchReader` of [fixed rows](arrow.md) |
| `parse_fix_line`, `parse_fixml_line`, `parse_ullink_line`, `parse_pairs` | one body of that dialect, or pairs already split | one `FixMsg`; a body holding [a second frame](decode.md#a-line-yields-none-one-or-many-messages) is refused |

A stream adapter owns a clone of the codec and borrows nothing, so `codec.arrow_reader(schema, codec.parse_lines(lines))` composes without the codec outliving the stream. Python exposes native iterators; JavaScript uses `IterableIterator<FixMsg>`. Every stream door fuses its source's exhaustion. Schema construction and group-plan resolution happen before repeated values are processed.

Every one of them ends in the same builder, so a document is typed by the rules that type a frame - one nesting builder, one fold, one code translation, one value contract. Each is the core's own method under the same name in all three languages.

### Lines are a stream

`parse_lines` is the line iterator everything else is built on: nothing is collected, and a line answers [every message it carries](decode.md#a-line-yields-none-one-or-many-messages) - two where a relay wrote two frames on one line, none where the line is a sentence, one stating nothing where it carries a JSON document. A line the reader refuses is an `Err` item the stream continues past: one corrupt line must not end a run over ten million. The example opens with a bridge frame: `#`-prefixed name keys, and one group occurrence whose value packs its members behind the two control bytes ULLINK uses.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    let bridge: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|";
    let held = reader.parse_line(bridge)?.next().expect("one frame")?;
    assert_eq!(held.by_tag(55)?.as_str(), Some("TTF"));
    assert_eq!(held.by_tag(44)?.as_f64(), Some(41.25));
    // The packed members became three real fields under one nesting.
    let party = held.party("1").expect("the buy-side party");
    assert_eq!(party.id().and_then(yggdryl::Scalar::as_str), Some("BUYSIDE"));

    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
        b"heartbeat emitted seq=7",
        b"",
        b"8=FIX.4.4|35=8|37=O1|11=A|10=0|8=FIX.4.4|35=8|37=O2|11=B|10=0|",
    ];
    let read: Vec<_> = reader.parse_lines(lines).collect();
    // An item per message: the sentence stated none, the last line two.
    assert_eq!(read.len(), 4);
    assert_eq!(read[0].as_ref().expect("an order").by_tag(55)?.as_str(), Some("AAPL"));
    assert!(read[1].is_err(), "an empty line is not a row, and the stream went on");
    assert_eq!(read[2].as_ref().expect("a report").by_tag(37)?.as_str(), Some("O1"));
    assert_eq!(read[3].as_ref().expect("a report").by_tag(37)?.as_str(), Some("O2"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    held, = reader.parse_line(
        b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )
    assert held.by_tag(55).as_py() == "TTF"
    assert held.by_tag(44).as_py() == 41.25
    # The packed members became three real fields under one nesting.
    party = held.party("1")
    assert party is not None and party[0].as_py() == "BUYSIDE"

    lines = [
        b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
        b"heartbeat emitted seq=7",
        b"8=FIX.4.4|35=8|37=O1|11=A|10=0|8=FIX.4.4|35=8|37=O2|11=B|10=0|",
    ]
    read = reader.parse_lines(lines)
    assert next(read).by_tag(55).as_py() == "AAPL"
    # The sentence stated no message; the last line stated both of its frames.
    assert next(read).by_tag(37).as_py() == "O1"
    assert next(read).by_tag(37).as_py() == "O2"
    assert next(read, None) is None
    # An empty line is not a row: the item raises where it is reached.
    with pytest.raises(ValueError):
        next(reader.parse_lines([b""]))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    const bridge = Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    )
    const [held] = reader.parseLine(bridge)
    assert.equal(held.byTag(55).toJSON(), 'TTF')
    assert.equal(held.byTag(44).toJSON(), 41.25)
    // The packed members became three real fields under one nesting.
    assert.equal(held.party('1')[0].toJSON(), 'BUYSIDE')

    const lines = [
      '8=FIX.4.4|35=D|11=A|55=AAPL|10=0|',
      'heartbeat emitted seq=7',
      '8=FIX.4.4|35=8|37=O1|11=A|10=0|8=FIX.4.4|35=8|37=O2|11=B|10=0|',
    ].map((line) => Buffer.from(line))
    const read = [...reader.parseLines(lines)]
    // Three lines, three messages: the sentence stated none, the last line two.
    assert.equal(read.length, 3)
    assert.equal(read[0].byTag(55).asJs(), 'AAPL')
    assert.equal(read[1].byTag(37).asJs(), 'O1')
    assert.equal(read[2].byTag(37).asJs(), 'O2')
    // An empty line is not a row: the item throws where it is reached.
    assert.throws(() => [...reader.parseLines([Buffer.alloc(0)])])
    ```

### A printed separator is still the separator

A capture that cannot print `0x01` writes what it stands for: `^A`, `\x01`, `<SOH>` or `{SOH}`. The escape happened on the way into the log rather than on the wire, so a numeric frame spelling its separator that way is unescaped once, before it is split, and reaches the same columns the byte itself reaches.

One vocabulary serves both readings, because the spelling a frame is *located* with and the one it is *split* on are the same fact: a capture that escapes its separator is recognized once rather than in each place that reads a frame. A frame carrying a real `0x01` is never scanned for the escapes, so the ordinary path allocates nothing.

### A packed occurrence uses explicit or declared boundaries

ULLINK packs one group occurrence's members behind EOT and ETX (`\x04\x03`); a bridge relaying into a FIX session packs them behind the protocol's own SOH. Both split an occurrence, because inside one neither byte can be part of a value.

The first spelling an occurrence actually carries is the one that splits it, and only that one: reading both at once would let a value that legitimately holds the other byte break into fields nobody wrote.

When neither control spelling is present, the reader scans for the next direct member name declared by the addressed group. The longest declared name wins, so `PartyIDSource` is not shortened to `PartyID`; names from elsewhere in the registry never become nested boundaries. A run with no declared boundary stays whole.

A group packed inside an occurrence is packed behind the same separator, at the same level as the occurrence's own members: `NOPARTYSUBIDS=1`, then `NOPARTYSUBIDS[0]=PARTYSUBID=a`, then `PARTYSUBIDTYPE=b`, then the party's own `PARTYID=c`. Every packed value ends with its separator, so where the inner occurrence ends the bridge writes two in a row, and the empty segment between them is its close. A value carrying a close anywhere but at its own end is bounded by its closes alone - a venue's own key packed inside a sub-identifier stays inside it - and a value carrying none is bounded by the dictionary: the pairs after the sub-occurrence belong to it while its group declares them, an occurrence of a group it declares is skipped whole, and the first pair it does not declare ends it. That pair ends every enclosing occurrence that does not declare it either, so it lands on the nearest one that does, or on the packed value's own occurrence, and what follows lands there with it. Either way the sub-occurrence renders as `NOPARTYIDS[0].NOPARTYSUBIDS[0].PARTYSUBID`, the key the builder nests by, at any depth a bridge packs - up to the sixty-four levels a message schema may nest, past which an opener is one more member and its packed value its value, so a run of openers nothing closed reads as a wide row rather than exhausting the reader. That rendering builds the row and nothing else: the [arrival record](#nothing-is-lost-at-the-end) keeps the one pair the bridge wrote, because a key no range of the line spells is a reading and not an arrival.

### A JSON document is one message stating nothing

A bridge logs what it exchanged over JMX beside what it exchanged over FIX, so a line may carry a JSON document - an object, or an array of objects, opening before any `=` and balanced to its close, with prose allowed in front of it and behind it - and the [classifier](registry.md#classifying-a-captured-line) names it `application/json` with no message type. The codec reads no document. Such a row is exactly one message named `unknown` with no entries - `entries()` is empty, `into_bytes` re-emits nothing and no tag 35 is built - carrying only what the row stated around the document: its clock, the version it was read at, the direction the prose in front of it spells under the [default rules](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) - `Response:` is `R`, `Request:` is `S` - and the row's own captured columns, a `pluginid` among them. A Jolokia answer, a wildcard or bulk answer, an error-only answer and a bare `{"a":1}` are each one such message, never one per configuration the document names, and nothing the document says reaches a field or a later message.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    let line = br#"2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)"#;
    let mut messages = reader.parse_line(line)?;
    let message = messages.next().expect("one message")?;
    assert!(messages.next().is_none());
    // Named `unknown`, with nothing the document said in it.
    assert_eq!(message.as_field().name(), "unknown");
    assert!(message.entries().is_empty());
    assert!(message.into_bytes(b'|').is_empty());
    assert_eq!(message.get_by_tag(35), None);
    assert_eq!(message.get_by_tag(49), None);
    // The prose in front of it is the row's, read as ever.
    assert_eq!(message.by_tag(385)?.as_str(), Some("R"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    line = b'2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)'
    message, = reader.parse_line(line)
    # Named `unknown`, with nothing the document said in it.
    assert message.field.name == "unknown"
    assert message.entries() == []
    assert message.into_bytes(ord("|")) == b""
    assert message.get_by_tag(35) is None
    assert message.get_by_tag(49) is None
    # The prose in front of it is the row's, read as ever.
    assert message.by_tag(385).as_py() == "R"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    const line = Buffer.from('2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)')
    const [message, ...rest] = reader.parseLine(line)
    assert.equal(rest.length, 0)
    // Named `unknown`, with nothing the document said in it.
    assert.equal(message.field.name, 'unknown')
    assert.deepEqual(message.arrivals(), [])
    assert.equal(message.intoBytes('|'.charCodeAt(0)).length, 0)
    assert.equal(message.getByTag(35), null)
    assert.equal(message.getByTag(49), null)
    // The prose in front of it is the row's, read as ever.
    assert.equal(message.byTag(385).asJs(), 'R')
    ```

### Edges

- Ordinary unframed text produces no message at all - a line that opens no frame, states no bridge pair and carries no document states nothing to read, so it is no row either. A payload that was there and would not parse is a message with nothing in it, so malformed syntax never fails the batch it arrives in; a stated mandatory clock or identity that does not read is a located error item.
- A bridge key's `#` is judged against the row's bare spellings, in a bridge row and in the name keys a bridge writes into a numeric frame alike. Alone, it drops: `#ORDERID=123` is the dictionary's `OrderID`. Restating a bare pair's bytes, the marked pair is a second spelling of one pair and goes, row and entries alike: `ORDERID=123|#ORDERID=123` is `OrderID` once, and `into_bytes` re-emits the one pair. Beside a bare twin stating other bytes it stays verbatim, because collapsing the two would merge two values under one name: `ORDERID=123|#ORDERID=345` is `OrderID` 123 beside `#ORDERID` 345 - its own column, its own entry - whichever arrived first. The twin is matched by the fold every key resolves under, so `OrderId` and `ORDER_ID` twin it too, and by its stem, so a bare `NOPARTYIDS` group claims every `#NOPARTYIDS[n]` however many the two state - each stays whole under its own name, the count beside them, and none lands in the dictionary's group. A marked group goes only whole: a marked count restating the bare one beside occurrences the bare group never numbered stays with them, and only a marked group restating the bare group pair for pair goes. A bare pair whose value is a stated absence is no twin, because a key that said nothing was sent is not a key that was sent; a value is compared as its bytes, because `abc` is not `ABC`; and the twin is a spelling, never an identity, so a tag and a marked name - `55=AAPL|#SYMBOL=AAPL` - state two values exactly as a tag and a bare name do. In a numeric frame the marks are judged and the keys kept as they are: a packed occurrence there is one value, as a bare one always was. A key marked twice is judged one mark at a time: `##ORDERID` twins `#ORDERID` as `#ORDERID` twins `ORDERID` - restating it goes, beside other bytes it stays, alone it loses one mark.
- A row's message type resolves the way every key does, in the one namespace: a name reaches the message of that name, and a bare code the message tag 35's code set names, else the first in name order. A bridge row calling itself `tradecapturereport` reads against the message of that name, which is what places a counter half the dictionary shares - `NoLegs`, `NoSides` - under the group that message declares.
- A stated absence - one of `null_values` - produces no field and no entry, because a key that said nothing was sent is not a key that was sent.
- The `version` a row was read at decides which code spelling a value translates through, never what a field is called: a tag is one column under the name the dictionary holds it by, and what each version called it stays readable as an alias beside that name.

## The columns are the folded names

`msgtype`, never `35` and never `msg_type`. A column is spelled the way the dictionary spells the field's canonical name - ASCII case folded once, on the way in - so a row reads the way a message reads, in every binding and every catalog, and a reader spelling `row["msgseqnum"]` finds the sequence number without a dictionary in hand.

The tag is still the identity. Each column carries its field's `fix:tag`, its `display` and its code set, and the row is [filled by that tag](#a-column-is-filled-by-the-tag-its-field-carries) rather than by the spelling, so a venue that renames a field between versions changes nothing about where its value lands. `fix_schema_tags` is the same row as tags, in the same order.

A List group column carries `fix:counter` beside the `fix:tag` its definition derives from its own name: the numeric count keeps its own column, and the group column beside it holds the occurrences. The built-in `altids` Map instead carries tag and counter 65020 on one group column, with no scalar count field; the Map's entries already determine its cardinality.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixRegistry, fix_column_of, fix_schema, fix_schema_tags};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    let schema = fix_schema(&registry, "FixMessage")?;

    let columns: Vec<&str> = schema.fields().iter().map(yggdryl::Field::name).collect();
    // The crate's own lead the row - a table is read by time and joined by
    // identity - and the protocol's own follow them.
    assert_eq!(&columns[..3], ["updatedat", "prevupdatedat", "createdat"]);
    let header = schema.index_of("beginstring").expect("the header opens");
    assert_eq!(&columns[header..header + 3], ["beginstring", "bodylength", "msgtype"]);
    assert_eq!(columns.last(), Some(&"fixentries"));
    assert_eq!(fix_schema_tags().len(), 116);
    assert_eq!(&fix_schema_tags()[header..header + 3], [8, 9, 35]);

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    let held = schema.get_field_by_path("msgtype").expect("the msgtype column");
    assert_eq!(held.display(), Some("MsgType"));
    assert_eq!(held.as_fix().tag()?, Some(35));
    // The tag rides on the column, so a tag still finds it. Rust only.
    assert_eq!(fix_column_of(&schema, 35), schema.index_of("msgtype"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixRegistry, fix_schema, fix_schema_tags

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    schema = fix_schema(registry, "FixMessage")

    columns = [child.name for child in schema]
    # The crate's own lead the row - a table is read by time and joined by
    # identity - and the protocol's own follow them.
    assert columns[:3] == ["updatedat", "prevupdatedat", "createdat"]
    header = columns.index("beginstring")
    assert columns[header:header + 3] == ["beginstring", "bodylength", "msgtype"]
    assert columns[-1] == "fixentries"
    assert len(fix_schema_tags()) == 116
    assert fix_schema_tags()[header:header + 3] == [8, 9, 35]

    # The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert schema.field("msgtype").display == "MsgType"
    assert schema.field("msgtype").fix.tag == 35
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')

    // The crate's own lead the row - a table is read by time and joined by
    // identity - and the protocol's own follow them.
    assert.equal(schema.fieldAt(0).name, 'updatedat')
    const header = schema.indexOf('beginstring')
    assert.equal(schema.fieldAt(header + 2).name, 'msgtype')
    assert.equal(schema.fieldAt(schema.fieldLen - 1).name, 'fixentries')
    assert.equal(fix.schemaTags().length, 116)
    assert.deepEqual(fix.schemaTags().slice(header, header + 3), [8, 9, 35])

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert.equal(schema.field('msgtype').display, 'MsgType')
    assert.equal(schema.field('msgtype').fix.tag, 35)
    ```

## Nothing is lost at the end

One record closes every row.

`fixentries` is the whole arrival record: every pair the reader read as sent - a stated absence and a bridge's marked restatement of a bare pair are read as never sent, as the [edges](#edges) above state - in arrival order and a group's members riding under the counter pair that heads them. It is a list of `fixentry` structs, each carrying five members: `tagnum` the tag its key resolved to, `tagname` the dictionary's name for that tag, `tagvalue` and `tagkey` the value and the key exactly as they arrived, and `fixentries` what arrived under it. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

| Member | Type | Holds |
| --- | --- | --- |
| `tagnum` | `int32` | the canonical tag the key resolved to, `0` where none did |
| `tagname` | `utf8` **not null** | the dictionary's name for that tag; the arrival key itself where no dictionary explains it; `UNNAMED_ENTRY` (`yggdryl:unnamed`) where the arrival carried no key at all |
| `tagvalue` | `utf8` | the value exactly as it arrived |
| `tagkey` | `utf8` | the key exactly as it arrived |
| `fixentries` | `list<fixentry>`, `utf8` at the leaf | what arrived under it; the folded JSON at the third level, null where nothing folded |

`tagkey` and `tagvalue` are the arrival and `tagnum` and `tagname` are what a dictionary made of it, resolved once at parse. Keeping the arrival key is what keeps re-emission byte for byte: a venue that wrote `Side=1` wrote `Side`, and a row spelling only the canonical `side` would rebuild a line nobody sent. `tagname` is the one member that cannot be null, because its job is to be the column a consumer groups and filters a wire name by without resolving every tag against a dictionary of its own - so it falls back to a name rather than to nothing.

An entry says what arrived and only that. A key and a value are ranges of the line the message was read from, so an entry never carries a key that appears nowhere in that line: a bridge packing a whole occurrence into one value - `#NOPARTYIDS[0]=PARTYID=BUYSIDE...PARTYROLE=1` - is recorded as the pair the bridge wrote, and the members read out of it fill `parties[0].partyid` and its siblings in the row. No dialect is there either: a message is not a dictionary member, and which dictionaries a field belongs to is the field's own `fix:branches` in the registry.

A key no dictionary explains is in the same record, with tag 0 beside its exact raw key, value, order and children - and `tagname` spelling that raw key rather than nothing: an unresolved name, an unresolved numeric wire key and an occurrence key - indexed or packed, which names no field - alike, while a resolved scalar or group key keeps its positive canonical tag. Tag 0 is arrival provenance and never a registry identity: `fix:tag`, `fix:tags`, `fix:counter` and `FixId::of` take positive tags only. So a venue onboarding a new field finds it by filtering one column for tag 0. The column materializes three `fixentry` levels and folds anything deeper into a canonical JSON leaf, which [`from_row`](message.md#a-row-is-a-message-again) validates and reads back; a level that folded nothing holds null there, which is what tells it from one that folded an empty subtree.

## The crate's own columns

Thirty-four scalar fields, one Map group and one Struct carry capture facts that no dictionary publishes. Every registry holds them from construction and the [store](store.md) writes them, so a dump is the whole row and a stored copy is read past in favour of the constructed one: `fix_crate_fields` lists all 36 in tag order, while scalar registry iteration counts the 34 scalars. Their tags run from 65001 - `CRATE_TAG_MIN` (65000) starts the reserved block, whose retired slots 65000, 65004 and 65016 are not reused - `SENDERSESSIONID_TAG_NAME` and its siblings hold each `(tag, name)` pair, and `is_crate_tag` tests ownership. `altids` is a group reached by `group_by_tag(65020)` or group definition name, and `instids` a component reached by definition name, not by the registry's scalar-field doors.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `version` | `Version` | 65001 | the FIX version it was *read* at, which is not always what `BeginString` claimed |
| `symbolticker` | `SymbolTicker` | 65002 | one instrument symbol that is the same across venues |
| `updatedat` | `UpdatedAt` | 65003 | the settled message instant: `snapshotat` at intake, truncated to the snapshot grid by the [lifecycle](lifecycle.md); non-null |
| `parentclordid` | `ParentClOrdID` | 65005 | the client order identifier this order descends from |
| `parentorderid` | `ParentOrderID` | 65006 | the venue order identifier this order descends from |
| `sendersessionid` | `SenderSessionId` | 65007 | the session the message came from, as the message states it: a bridge row's `SESSIONID` reaches it by alias, and the instance a bracket names is `bridgesessionid` instead |
| `msgctxid` | `MsgCtxId` | 65008 | the message context a bridge handled the message in, from its log's bracket |
| `pluginid` | `PluginId` | 65009 | the plugin that logged the line, as a bridge names it: a row's own `pluginid` capture or column fills it, and it selects nothing - no dictionary, no version |
| `prevpluginid` | `PrevPluginId` | 65010 | the plugin a message came through before the one that logged it, as a bridge names it; only ever a capture of that name, never derived |
| `sendersessionname` | `SenderSessionName` | 65011 | the name of the session the message came from: a bridge row's `ULFROMSESSIONNAME`, else the session that logged a line it sent |
| `targetsessionname` | `TargetSessionName` | 65012 | the name of the session the message went to: a bridge row's `ULTOSESSIONNAME`, else the session that logged a line it received |
| `isincode` | `ISINCode` | 65013 | the instrument's ISIN, as an [`isin`](../types/codes.md): `SecurityID(48)` where `SecurityIDType(22)` says ISIN, else the `SecurityAltID(455)` whose `SecurityAltIDType(456)` does |
| `miccode` | `MICCode` | 65014 | the market the message names, as a `mic`: `SecurityExchange(207)`, else `ExDestination(100)`, else `LastMkt(30)` |
| `state` | `State` | 65015 | the order's state, as a `state`: `OrdStatus(39)`, else `ExecType(150)` |
| `msghash` | `MsgHash` | 65017 | the message's identity, `fixedbinary(16)`: signed `updatedat` nanoseconds with the sign bit flipped in bytes 0..8, then all 64 bits of the XXH64 of its named content in bytes 8..16; non-null |
| `msgphash` | `MsgPHash` | 65018 | the chain's identity, `fixedbinary(16)`: the big-endian XXH3-128 of `code` alone; non-null |
| `targetsessionid` | `TargetSessionId` | 65019 | the session the message went to, as the message itself states it |
| `altids` | `AltIds` | 65020 | a nullable sorted Map of this message's direct declared identifiers, keyed by canonical member name; group members are not flattened |
| `prevupdatedat` | `PrevUpdatedAt` | 65021 | the previous message's `updatedat` in the selected chain; nullable |
| `prevmsghash` | `PrevMsgHash` | 65022 | the previous message's `msghash` in the selected chain, `fixedbinary(16)`; nullable |
| `createdat` | `CreatedAt` | 65023 | the creation instant: `snapshotat` at intake, the first accepted message's in a live chain; non-null |
| `code` | `Code` | 65024 | the exact chain name, empty when unknown; non-null |
| `snapshotat` | `SnapshotAt` | 65025 | the instant a reading of this chain was taken at, ungridded: `FixLifecycle::snapshot` stamps it and nothing else does, so it is empty on every row that is not a snapshot |
| `sourceurl` | `SourceUrl` | 65026 | the object the line was read from, typed as a `url`: filled from the text read's own `sourceurl` column, and left out of the content `msghash` hashes, because the same message read from a second copy of one day's log is the same message |
| `nofixentries` | `NoFixEntries` | 65027 | the counter of the `fixentries` arrival record, read off the record rather than derived; left out of the content `msghash` hashes, as the record it counts is |
| `recordedat` | `RecordedAt` | 65028 | when the capture wrote the line down, which is a fact about the capture and so is outside the content `msghash` hashes |
| `expiredat` | `ExpiredAt` | 65029 | when the order stops being live, from what the message said about its own expiry |
| `bidcurrency` | `BidCurrency` | 65030 | the currency the bid lane is denominated in: the message's own `Currency(15)`, else `SettlCurrency(120)` |
| `offercurrency` | `OfferCurrency` | 65031 | the same for the offer lane |
| `bridgesessionid` | `BridgeSessionId` | 65032 | the session *instance* a bridge handled the line on, as its own row header brackets it - never what the message states about itself, because the joined names below must be the same string on every leg of one message |
| `bloombergcode` | `BloombergCode` | 65033 | the instrument's Bloomberg identifier, as a `bloomberg`: the message's own, else a `SecurityID(48)` or `SecurityAltID(455)` whose source is Bloomberg (`A`) or FIGI (`S`) |
| `cusipcode` | `CUSIPCode` | 65034 | the instrument's CUSIP, as a `cusip`, read the same way from source `1` |
| `sedolcode` | `SEDOLCode` | 65035 | the instrument's SEDOL, as a `sedol`, read the same way from source `2` |
| `instids` | `InstIds` | 65036 | every identifier the instrument is known by as one Struct - `cficode`, `isincode`, `bloombergcode`, `cusipcode`, `sedolcode` - filled from the columns beside it; null where the message names the instrument in no way at all |
| `sessionmsgid` | `SessionMsgId` | 65037 | `bridgesessionid` and `msgctxid` joined by a colon, which is what the legs of one routed message share; null where either part is missing |
| `sessionmsgseqid` | `SessionMsgSeqId` | 65038 | the same with `MsgSeqNum(34)` appended, which names one occurrence and matches exactly |

The three identity columns - `msghash`, `msgphash`, `prevmsghash` - are `fixedbinary(16)`, sixteen plain big-endian bytes with no version or variant bit and no Arrow extension over them, because every lake engine reads `fixed[16]` and none reads `uuid` the same way twice.

The instrument is not among them, and is not a column at all. The [lifecycle](lifecycle.md) digests what the message says the instrument is - its market, its classification, its ISIN else its symbol, and its currency - to scope the identifiers a chain hangs under, and spells those sixteen bytes as the thirty-two hex digits opening a generated `code`. Nothing stamps the digest into the row, and no message states one to scope itself by: 65016 carried it and is retired unreused.

`msghash` is the one stored message identity and `msgphash` the chain's; what each hashes, and why a projection that adds or renames columns may move `msghash` while unchanged named content keeps it, is the [message's identity](message.md#clocks-and-identity). `FixMsg::digest` is a separate contract: the XXH3-128 of the arrival record with the standard header, the standard trailer and the crate's own tags left out, `MsgType` excepted, so two identical orders sent a second apart, or relayed through two sessions, digest equal.

Session columns come from bridge pairs or [row-header captures](arrow.md#a-bridge-log-names-what-it-fills), without overwriting a value the message stated. The four identifier columns, `miccode` and `state` are derived when a message becomes a row; `code`, `prevupdatedat`, `prevmsghash` and the grid `updatedat` are stamped by the [lifecycle](lifecycle.md), and `snapshotat` only where it takes a snapshot. `altids` is filled by [enrichment](#a-messages-direct-identifiers-fill-one-map), not by row projection. FIX's `SenderCompID` and `TargetCompID` name counterparties; the plugin carrying a message inside a bridge is a separate fact.

There is no partition column. How a layout is cut is the target's to decide: an Iceberg table takes an `hour` transform over `updatedat` and reads the instant the row already carries, so a materialized copy of it was a second owner of one fact. A reader that wants the hour asks the target for it.

### Every message is dated and versioned

These columns are filled when a message is built, whatever its line carried, and none of them becomes an entry unless the wire sent it - so `into_bytes` still re-emits the wire byte for byte.

`beginstring` is the wire's own `BeginString(8)` when stated, else `FIX.<version>` for the version the message was read at: the row's own `beginstring` [column or capture](arrow.md#a-column-is-the-caller-speaking-per-row), else the one `ApplVerID(1128)` or `BeginString(8)` implied, else the crate's own 4.4. The dictionary states no version to lend: it holds every tag ever defined and filters by none. A codec pins none - a version is what a line or the transport around it said, never a caller's statement about a whole run. A bridge row, and the row a JSON document is, therefore say which FIX they were read as exactly as a frame does.

`version` states that same answer outright, on every message the codec generates, because `BeginString` is what the message says about *itself* and a session that mislabels itself - or that carries a row written to a later FIX than it speaks - makes the two differ. `FixMsg::version()` answers the crate's column where a read stamped one and `BeginString` otherwise, so it always answers for a built message.

Six values close every message and are never null: `SendingTime(52)`, `updatedat`, `createdat`, `code`, `msghash` and `msgphash`. Initial intake settles them once. `SendingTime` is the message's own, else a carrier row's, else the codec's `default_sending_time`, else one UTC-now read for that undated message; the event instant is a stated `snapshotat`, else `TransactTime(60)`, else that `SendingTime`; `updatedat` defaults to it, `createdat` to a stated one, else `OrigSendingTime(122)`, else it, and `code` to empty. `snapshotat` itself is the seventh column of the bundle and the one that is nullable: only `FixLifecycle::snapshot` stamps it, so a row that no snapshot was taken of leaves it empty and "is this row a snapshot" is answerable from the row. A stated clock that is not an instant is a located refusal, never an absent clock a default overwrites. No wall clock is read after intake - enrichment, writes, row exchange and replay carry the settled values - so a read that must be reproducible pins `default_sending_time` or carries the settled rows. `updatedat()`, `createdat()`, `msghash()` and `msgphash()` borrow theirs without a lookup.

The root's children are the standard header in its declared order, the body as it arrived, the standard trailer, then whichever of the bundle the message did not state. The fixed schema declares those six and `beginstring` non-null, and every other column - `snapshotat` among them - nullable. The crate's own definitions come first in the example, then two messages the codec dates.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{CODE_TAG_NAME, FixCodec, FixRegistry, SNAPSHOTAT_TAG_NAME, Scalar, TimeUnit, Timezone, fix_crate_fields};

    let fields = fix_crate_fields()?;
    assert_eq!(fields.len(), 36);
    // No partition column: how a layout is cut is the target's to decide.
    assert!(fields.iter().all(|field| !field.is_partition()));
    let ids = fields.iter().find(|field| field.name() == "instids").expect("the joined identifiers");
    let members: Vec<&str> = ids.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(members, ["cficode", "isincode", "bloombergcode", "cusipcode", "sedolcode"]);

    // The three identity columns are sixteen plain bytes, not a UUID.
    let identity = yggdryl::DataType::fixed_size_binary(16)?;
    for name in ["msghash", "msgphash", "prevmsghash"] {
        let column = fields.iter().find(|field| field.name() == name).expect("a crate column");
        assert_eq!(column.dtype(), &identity);
        assert_eq!(column.dtype().to_string(), "fixed_size_binary(16)");
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let default = Scalar::datetime64(1_704_190_530_000_000_000, TimeUnit::Nanosecond, Timezone::UTC)?;
    let reader = FixCodec::new(Arc::clone(&registry))
        .try_with_default_sending_time(Some(default.clone()))?;
    // A frame stating neither its version nor a clock is still versioned - at
    // the crate's own 4.4 - and dated by the codec's default sending time.
    let bare = reader.parse_line(b"35=D|55=AAPL|10=0|")?.next().expect("one frame")?;
    assert_eq!(bare.by_tag(8)?.as_str(), Some("FIX.4.4"));
    assert_eq!(bare.version().map(|version| version.to_string()), Some("4.4".to_owned()));
    assert_eq!(bare.by_tag(52)?, &default);
    assert_eq!(bare.updatedat(), &default);
    assert_eq!(bare.createdat(), &default);
    assert_eq!(bare.by_tag(CODE_TAG_NAME.0)?.as_str(), Some(""));
    // None of them became an entry, so the wire comes back byte for byte.
    assert_eq!(bare.into_bytes(b'|'), b"35=D|55=AAPL|10=0|");

    // A frame stating its clocks keeps them: TransactTime is the event, and a
    // read is not a snapshot, so that column stays empty.
    let sent = reader
        .parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(sent.by_tag(8)?.as_str(), Some("FIX.4.2"));
    assert_eq!(sent.by_tag(52)?.temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_200_415));
    assert_eq!(sent.updatedat().temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_199_900));
    assert_eq!(sent.createdat(), sent.updatedat());
    assert!(sent.by_tag(SNAPSHOTAT_TAG_NAME.0)?.is_null());
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    from yggdryl import DataType
    from yggdryl.fix import FixCodec, FixRegistry, fix_crate_fields

    CODE, SNAPSHOTAT = 65024, 65025
    fields = list(fix_crate_fields())
    assert len(fields) == 36
    # No partition column: how a layout is cut is the target's to decide.
    assert not any(field.is_partition for field in fields)
    ids = next(field for field in fields if field.name == "instids")
    assert [member.name for member in ids] == [
        "cficode",
        "isincode",
        "bloombergcode",
        "cusipcode",
        "sedolcode",
    ]

    # The three identity columns are sixteen plain bytes, not a UUID.
    identity = DataType("fixedbinary(16)")
    by_name = {field.name: field for field in fields}
    for name in ("msghash", "msgphash", "prevmsghash"):
        assert by_name[name].dtype == identity

    default = datetime(2024, 1, 2, 10, 15, 30, tzinfo=timezone.utc)
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry, default_sending_time=default)

    # A frame stating neither its version nor a clock is still versioned - at
    # the crate's own 4.4 - and dated by the codec's default sending time.
    bare = next(reader.parse_line(b"35=D|55=AAPL|10=0|"))
    assert bare.by_tag(8).as_py() == f"FIX.{bare.by_name('version').as_py()}"
    assert bare.by_tag(52).as_py() == default
    assert bare.updatedat() == bare.createdat() == bare.by_tag(52)
    assert bare.by_tag(CODE).as_py() == ""
    # None of them became an entry, so the wire comes back byte for byte.
    assert bare.into_bytes(ord("|")) == b"35=D|55=AAPL|10=0|"

    # A frame stating its clocks keeps them: TransactTime is the event, and a
    # read is not a snapshot, so that column stays empty.
    sent = next(reader.parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|"))
    assert sent.by_tag(8).as_py() == "FIX.4.2"
    assert sent.by_tag(52).as_py() == datetime(2026, 8, 21, 10, 30, 0, 415000, tzinfo=timezone.utc)
    assert sent.updatedat().as_py() == datetime(2026, 8, 21, 10, 29, 59, 900000, tzinfo=timezone.utc)
    assert sent.createdat() == sent.updatedat()
    assert sent.by_tag(SNAPSHOTAT).is_null
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { DataType, fix, Scalar } = require('yggdryl')

    const [CODE, SNAPSHOTAT] = [65024, 65025]
    const fields = fix.crateFields()
    assert.equal(fields.length, 36)
    // No partition column: how a layout is cut is the target's to decide.
    assert.ok(fields.every((field) => !field.isPartition))
    const ids = fields.find((field) => field.name === 'instids')
    assert.deepEqual(
      Array.from({ length: ids.fieldLen }, (_, at) => ids.getFieldAt(at).name),
      ['cficode', 'isincode', 'bloombergcode', 'cusipcode', 'sedolcode'],
    )

    // The three identity columns are sixteen plain bytes, not a UUID.
    const identity = DataType.fixedSizeBinary(16)
    for (const name of ['msghash', 'msgphash', 'prevmsghash']) {
      const column = fields.find((field) => field.name === name)
      assert.ok(column.dtype.equals(identity))
    }

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry, {
      defaultSendingTime: new Date(Date.UTC(2024, 0, 2, 10, 15, 30)),
    })

    // A frame stating neither its version nor a clock is still versioned - at
    // the crate's own 4.4 - and dated by the codec's default sending time.
    const bare = reader.parseLine(Buffer.from('35=D|55=AAPL|10=0|')).next().value
    assert.equal(bare.byTag(8).toJSON(), `FIX.${bare.byName('version').toJSON()}`)
    assert.ok(bare.byTag(52).equals(reader.defaultSendingTime))
    assert.ok(bare.updatedat().equals(bare.byTag(52)))
    assert.ok(bare.createdat().equals(bare.byTag(52)))
    assert.equal(bare.byTag(CODE).asJs(), '')
    // None of them became an entry, so the wire comes back byte for byte.
    assert.equal(bare.intoBytes('|'.charCodeAt(0)).toString(), '35=D|55=AAPL|10=0|')

    // A frame stating its clocks keeps them: TransactTime is the event, and a
    // read is not a snapshot, so that column stays empty.
    const sent = reader
      .parseLine(Buffer.from('8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|'))
      .next().value
    assert.equal(sent.byTag(8).toJSON(), 'FIX.4.2')
    assert.ok(sent.updatedat().equals(sent.byTag(60)))
    assert.ok(sent.createdat().equals(sent.updatedat()))
    assert.equal(sent.byTag(SNAPSHOTAT).kind, 'null')
    ```

## What a message implied is filled in

A venue sends what its counterparty needs and nothing more, so a row is routinely missing values the message itself already determines: a report stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill stating `LastQty` and `LastPx` has said what it was worth, and a message naming its instrument by an ISIN has said which country issued it. Enrichment is a call over what the reader built: `enrich_message` fills one message, `enrich_messages` a stream of them, each from itself alone - the pass carries nothing from one message to the next; and [`enrich_messages_arrow_reader`](arrow.md#filled-where-it-sits) a stream of batches, without parsing anything again. Those three are the whole of it: there is one enriching pass, and they are its doors.

The pass is four steps on one message, and their order is the pass's own rather than something a caller composes. What the row's projection dropped comes back off the arrival record first. It then **restates** - the row [re-expressed under the dictionary](message.md#restated-under-the-dictionary) - because every derivation below reads by canonical name, and a child a session spelled under an alias is invisible until it has been canonicalized. It then **derives** what the message implies and did not state, from what its fields declare: every field carrying a [`fix:derivation`](registry.md#a-field-carries-how-it-is-derived) is a column the pass fills, by evaluating that field's own term over the message, and the derivations are swept to a fixpoint. Then the component's identifier declaration fills the `altids` Map - in that order, because each may feed the next.

Completion writes nothing because a spelling is a way of asking rather than a thing to store. A consumer addressing `lastshares` finds the `lastqty` the message states: `get_by_name` resolves the spelling through the registry, which answers a field for any alias it holds. No alias twin is added as a second child, and the reason is not economy. Two of the forty-two shipped aliases are another field's canonical name - `quoteackstatus` is an alias of `QuoteStatus(297)` and the name of tag 1865, `tradetype` an alias of `BidTradeType(418)` and the name of tag 3006 - so a message stating both would hold two children of one name, and the root the pass builds would be refused whole. A twin could not cross the batch door either: [`fix_schema`](#the-columns-are-the-folded-names) names a column for a canonical field and for no alias, so `enrich_messages_arrow_reader` would discard on the way out what `enrich_messages` had just added, and the two doors would stop being one pass. And `get_by_tag` answers the earliest child on a tag, so where the twin was placed would silently decide what every column, every derivation, every lift and the lifecycle read.

Three things hold whatever the derivation. Only the row is filled, never the entries, so `into_bytes` re-emits the wire byte for byte either way. A derivation answers only where the answer is certain: an absent input is a null the term answers null over, so an identifier no check digit closes, a CFI whose category several security types share and a security type outside every group the specification files answer nothing rather than a guess. And a stated value is never overwritten, which is what makes a second pass change nothing - a value derived once is a stated value the second time.

The rules are the registry's, not the crate's. Each is one term of the [expression grammar](../expression/grammar.md) over the message's fields, carried as `fix:derivation` by the field it fills and read with `FixField::derivation` - never code per field, and never a table in Rust. A desk that derives a column differently edits the field and updates the registry, and every reader linked to it fills by the edit - [configuring a derivation](registry.md#configuring-a-derivation) shows the edit and the reader filling by it in Rust, Python and JavaScript; [the dictionary's own derivations](registry.md#the-derivations-the-dictionary-carries) are the specification's tables spelled as terms, and the crate's three derived columns declare theirs in `fix/crated.rs`.

| Contract | Rule |
| --- | --- |
| Rule | `fix:derivation` on the target field: one term over the message's fields by their canonical folded names (`orderqty`, `cumqty`, `secaltidgrp`), a group reached by its name and a member of it through a path (`secaltidgrp[securityaltidsource = '4'][0].securityaltid`), the crate's own columns by theirs |
| Compiled | once per registry, on the first enrichment or row fill, and kept until a field or a definition changes: every term parsed, every name it reads proven a field or a group of the registry, every term bound against the working schema; a name the dictionary lacks or a term that does not bind refuses at that first call, naming the field, never per message - and the refusal is kept as a compiled list is, so every door answers it, `enrich_message`, `enrich_messages`, `into_row` and the batch reader alike, and the registry compiles once until a field changes |
| Bound | once per registry, at compile, against the working schema: the ordered union of every column any derivation reads or fills, each typed by the registry's field for it, a group by its group definition; no term is bound against a message, so no shape is recognized and nothing is kept per shape - a stream of a million messages of a thousand shapes binds nothing |
| Swept | in tag order over a working row gathered off the message by tag - a stated field as its value, a stated group laid out as the registry declares its occurrence, an absent column as null: a target the row holds non-null is skipped; else the term is evaluated, a null answer is silence, a non-null one is typed by the target's field exactly as `set` types a value - a value the field refuses is silence - and written where the next derivation reads it |
| Fixpoint | sweeps repeat until one writes nothing, bounded by the number of derivations, so a chain settles in whatever order its fields fall: `securityid` from `isincode` and `isincode` from `securityid`, `product` after `securitytype` after `cficode`, `leavesqty` after `ordstatus` after `exectype` |
| Landed | everything that derived reaches the row through one `set_each`, one rebuild for the whole pass, each value under the dictionary's own field for the tag; a row holding a stated null takes the answer in that child's place, anything else is appended |
| Crate columns | `isincode`, `miccode` and `state` derive by the same compiled term and the same gather - on the message when enriched, on the row when `into_row` is asked for a column the message does not state, each answer typed by the column's field so a refused value is a null column on the row as it is silence on the message - and their terms read the standard's fields by name, so a registry built from a handful of fields holds what it lacks as null columns of the working schema rather than refusing to enrich |
| Silence | an absent input, a condition that does not hold, an answer the field refuses, an evaluation the grammar refuses, and a crate column's term that does not bind over a registry lacking what it reads are all a null column and never a failure |

What each shipped derivation reads, in words; the exact terms are [in the registry's table](registry.md#the-derivations-the-dictionary-carries).

| Fills | From | The table it reads |
| --- | --- | --- |
| `SecurityIDSource(22)` | `SecurityID(48)` | the `SecurityIDSource` code set names the standard each code stands for, and each standard closes its identifiers with a check digit: `4` for a number ISO 6166 closes, `1` for a CUSIP, `2` for a SEDOL - `try_cast(securityid as isin)`, `cusip`, `sedol` |
| `isincode` | `SecurityID(48)` under source `4`, else the `SecurityAltID(455)` whose `SecurityAltIDSource(456)` is `4` | ISO 6166, each read through `try_cast(... as isin)`: a primary the check digit does not close is null rather than an answer, so the alternate is consulted behind it, and a spelling neither closes answers nothing |
| `SecurityID(48)` | `isincode` | a bridge row states the crate's column and has thereby stated the primary identifier, whose validation then states the source |
| `Symbol(55)` | `SecurityID(48)` under source `8` or `A`, else the `SecurityAltID(455)` whose source is `8` | the `SecurityIDSource` codes of an exchange symbol and a Bloomberg symbol |
| `CountryOfIssue(470)` | `isincode` | ISO 6166 opens a number with the ISO 3166 code of the numbering agency's country, where it is one: the prefix answers exactly where the crate's own registry of countries, `StringEnum::COUNTRIES`, lists it - the generator writes those 249 codes into the term's `in (...)` off the registry's source - so `XS`, `EU` and every unassigned pair answer nothing |
| `SecurityType(167)` | `CFICode(461)` | Appendix 6-D at its category level, case not counting - `ES` is `CS`, `F` is `FUT`, an `O?F` is `OOF` and every other `O` is `OPT`, `LR` is `REPO`; a category every kind of bond shares, `DB`, answers nothing |
| `CFICode(461)` | `SecurityType(167)`, `PutOrCall(201)` | Appendix 6-D the other way: `CS` is `ESXXXX`, `CORP` is `DBXXXX`, `FRN` is `DBVXXX`, an option is `OC` or `OP` by its `PutOrCall` and `OX` without one |
| `PutOrCall(201)` | `CFICode(461)` | the second character of a listed (`O`) or unlisted (`H`) option: `C` is a call, `P` a put |
| `Product(460)` | `SecurityType(167)`, else `CFICode(461)` | the group the dictionary's `SecurityType` code set files the value under, as the `Product` code set spells it - `Agency` is `1`, `Corporate` `3`, `Currency` `4`, `Equity` `5`, `Government` `6`, `Loan` `8`, `Money Market` `9`, `Mortgage` `10`, `Municipal` `11`, `Financing` `13`; `Derivatives` and `Other` answer nothing. A CFI in category `E` is `5` and in `L` is `13` |
| `miccode` | `SecurityExchange(207)`, `ExDestination(100)`, `LastMkt(30)` | the first stated, as the [column](#the-crates-own-columns) is defined |
| `TimeInForce(59)` | nothing, on an order, a replace or a report | the field's own definition: absent means `0`, a day order |
| `OrdStatus(39)` | `ExecType(150)`; else `LeavesQty(151)` and `CumQty(14)` on a trade | the values the two code sets spell alike - not `D`, Restated in one and AcceptedForBidding in the other; a trade leaving nothing is filled, `2`, and one leaving something after doing something is partially filled, `1` - each landing as the [`state`](../types/codes.md) column spells it |
| `state` | `OrdStatus(39)`, `ExecType(150)` | the first stated, as the column is defined |
| `LeavesQty(151)`, `OrderQty(38)`, `CumQty(14)` | the other two, on a report | Appendix D: `OrderQty = CumQty + LeavesQty`, nothing is left once `OrdStatus(39)` is closed, and what a canceled order asked for is what it did plus `CxlQty(84)`: a report with nothing left - stated or unstated - that states a canceled quantity answers `CumQty + CxlQty`, one that states what is left answers `CumQty + LeavesQty` whatever it canceled, and `CxlQty` is read last where nothing says what is left |
| `GrossTradeAmt(381)` | `LastQty(32)` x `LastPx(31)` | Appendix D's execution reports |
| `SettlCurrAmt(119)` | `GrossTradeAmt(381)` x `SettlCurrFxRate(155)` | Appendix O |
| `Currency(15)`, `SettlCurrency(120)` | each other | Appendix O: a trade settling in the currency it was dealt in states it once |
| `AvgPx(6)` | `LastPx(31)` | Appendix D, only where `CumQty(14)` says the whole done quantity is this fill |
| `LastPx(31)` | `LastSpotRate(194)` + `LastForwardPoints(195)` | a forward price is quoted as a spot rate and the points away from it, and the points are already in price units, so the two add |
| `BidPx(132)`, `OfferPx(133)` | `BidSpotRate(188)` + `BidForwardPoints(189)`, `OfferSpotRate(190)` + `OfferForwardPoints(191)` | the same quoting, read on each side of a two-sided quote |
| `PeggedPrice(839)` | `PeggedRefPrice(1095)` + `PegOffsetValue(211)` | a pegged order's price is the reference it pegs to plus its own offset, which is signed: a peg below the reference is a negative one |
| `LastMultipliedQty(2368)` | `LastQty(32)` x `ContractMultiplier(231)` | a quantity in contracts times what one contract multiplies to is that quantity in units |
| `TotalTradeMultipliedQty(2370)` | `TotalTradeQty(2367)` x `ContractMultiplier(231)` | the same product, over the whole trade rather than one fill |
| `MinPriceIncrementAmount(1146)` | `MinPriceIncrement(969)` x `ContractMultiplier(231)` | the same product again: an increment in money is the increment in price times the multiplier |
| `TotalTradeQty(2367)` | `LastQty(32)` x `TradingUnitPeriodMultiplier(2353)` | a trade covering several trading unit periods trades its quantity once in each of them |
| `TotalGrossTradeAmt(2369)` | `LastPx(31)` x `TotalTradeQty(2367)` | the whole trade's worth, as `GrossTradeAmt` is the fill's |
| `OrigSendingTime(122)` | `SendingTime(52)`, where `PossDupFlag(43)` is `Y` | the session layer's own definition: a possible duplicate carries the clock of the send it repeats |
| `CurrencyCodeSource(2897)` | `Currency(15)` stated | ISO 4217, `6` - the only source FIX's own `Currency` field is written in |

There is no hand-laid order. A sweep evaluates the derivations in tag order, each answer visible to the derivations after it, and the sweeps repeat until one writes nothing, so a `SecurityID`'s validation states the source under which the ISIN column is read, an ISIN found only among the alternate identifiers becomes the `SecurityID` whose validation states the source in turn, the country is read after either, and a status read off an execution type decides what is left - in one pass, whichever way round the chain runs. The primary identifier is read before the alternate ones because the `isincode` term says so (`coalesce` of the sourced `securityid`, then the alternate), not because a rule comes first. The fixpoint reaches what one pass in a hand-laid order could not: a forward price derived from spot and points is the price the worth and the average read, a quantity ordered off what was canceled is the one the remainder reads, and a trade over several periods multiplies and prices through the quantity it derived - each a value the old pass left absent and added on its second run.

What the pass leaves null it leaves null on purpose, and a reader needs to be able to tell that from a gap. No amount is filled whose scale depends on a convention the message does not state: `GrossTradeAmt(381)` from a percent-of-par price needs a division by one hundred that the specification writes in price units and leaves to the reader; an FX gross amount is a product or a quotient depending on `SettlPriceFxRateCalc(2366)`, and absent that tag the quoting convention decides; `NetMoney(118)` needs `Commission(12)` resolved through `CommType(13)` and every `MiscFeeAmt(137)` through `MiscFeeBasis(891)`. A capture reader that guesses a notional is worse than one that leaves it null. Nor is anything filled that needs a second message - `OrigClOrdID(41)` from the request a report answers, `ListID(66)` from the list an order belonged to, a bust's effect on `CumQty(14)` - because each is a fact about a chain rather than about a message, and chains are the [lifecycle's](lifecycle.md).

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|";
    let held = reader.enrich_message(reader.parse_line(line)?.next().expect("one frame")?)?;
    assert_eq!(held.by_tag(22)?.as_str(), Some("4"));
    assert_eq!(held.by_tag(yggdryl::ISINCODE_TAG_NAME.0)?.as_str(), Some("US0378331005"));
    assert_eq!(held.by_tag(470)?.as_str(), Some("US"));
    assert_eq!(held.by_tag(167)?.as_str(), Some("CS"));
    assert_eq!(held.by_tag(460)?, &Scalar::from(5_i32));
    assert_eq!(held.by_tag(yggdryl::MICCODE_TAG_NAME.0)?.as_str(), Some("XNAS"));
    // A trade leaving nothing is filled, as the `state` column spells it.
    assert_eq!(held.by_tag(39)?.as_str(), Some("80FILLED"));
    assert_eq!(held.by_tag(59)?.as_str(), Some("0"), "a day order");

    // Only the row was filled: the wire comes back byte for byte.
    assert_eq!(held.into_bytes(b'|'), line);
    // And a second pass changes nothing.
    assert_eq!(reader.enrich_message(held.clone())?, held);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|"
    held = reader.enrich_message(next(reader.parse_line(line)))
    assert held.by_tag(22).as_py() == "4"
    assert held.by_tag(65013).as_py() == "US0378331005"  # isincode
    assert held.by_tag(470).as_py() == "US"
    assert held.by_tag(167).as_py() == "CS"
    assert held.by_tag(460).as_py() == 5
    assert held.by_tag(65014).as_py() == "XNAS"  # miccode
    # A trade leaving nothing is filled, as the `state` column spells it.
    assert held.by_tag(39).as_py() == "80FILLED"
    assert held.by_tag(59).as_py() == "0"  # a day order

    # Only the row was filled: the wire comes back byte for byte.
    assert held.into_bytes(ord("|")) == line
    # And a second pass changes nothing.
    assert reader.enrich_message(held) == held
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    const line = '8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|'
    const held = reader.enrichMessage(reader.parseLine(Buffer.from(line)).next().value)
    assert.equal(held.byTag(22).toJSON(), '4')
    assert.equal(held.byTag(65013).toJSON(), 'US0378331005') // isincode
    assert.equal(held.byTag(470).toJSON(), 'US')
    assert.equal(held.byTag(167).toJSON(), 'CS')
    assert.equal(held.byTag(460).toJSON(), 5)
    assert.equal(held.byTag(65014).toJSON(), 'XNAS') // miccode
    // A trade leaving nothing is filled, as the `state` column spells it.
    assert.equal(held.byTag(39).toJSON(), '80FILLED')
    assert.equal(held.byTag(59).toJSON(), '0') // a day order

    // Only the row was filled: the wire comes back byte for byte.
    assert.equal(held.intoBytes('|'.charCodeAt(0)).toString(), line)
    // And a second pass changes nothing.
    assert.ok(reader.enrichMessage(held).equals(held))
    ```

### A message's direct identifiers fill one Map

`fix:identifiers` on the message component selects direct scalar members; its compiled `MsgType` supplies their values in declaration order, and enrichment writes their canonical names and UTF-8 values into the sorted `altids` Map. Nested groups are not flattened, non-null keys are required, and nullable values remain nullable across the native row and Arrow doors.

A known message with no selected values gets `{}`; an unknown message leaves `altids` absent, so its fixed column is null. A stated map, including an empty one, is preserved; enrichment is idempotent and changes neither arrival entries, emitted bytes nor the digest. This Map has no separate scalar count and introduces no numeric FIX group wire syntax.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let wire = b"8=FIX.4.4|35=8|37=O-1|11=C-1|17=E-1|10=0|";
    let original = codec.parse_fix_line(wire)?;
    let filled = codec.enrich_message(original.clone())?;
    assert_eq!(filled.by_tag(65_020)?.get_key_str("clordid"), Some(&Scalar::from("C-1")));
    assert_eq!(filled.entries(), original.entries());
    assert_eq!(filled.into_bytes(b'|'), wire);
    assert_eq!(filled.digest(), original.digest());
    assert_eq!(codec.enrich_message(filled.clone())?, filled);

    let schema = fix_schema(&registry, "FixMessage")?;
    let batches = codec.arrow_reader(schema, [filled.clone()])?;
    let mut restored = codec.messages(batches);
    assert_eq!(restored.next().expect("one row")?.by_tag(65_020)?, filled.by_tag(65_020)?);
    assert!(restored.next().is_none());
    let mut stated = filled.clone();
    stated.set("altids", Scalar::from_mapping([])?)?;
    assert!(codec.enrich_message(stated)?.by_tag(65_020)?.as_mapping().expect("a map").is_empty());
    let empty = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|")?)?;
    assert!(empty.by_tag(65_020)?.as_mapping().expect("a map").is_empty());
    let unknown = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=ZZ|10=0|")?)?;
    assert!(unknown.get_by_tag(65_020).is_none());
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)
    wire = b"8=FIX.4.4|35=8|37=O-1|11=C-1|17=E-1|10=0|"
    original = codec.parse_fix_line(wire)
    filled = codec.enrich_message(original)
    expected = {"clordid": "C-1", "execid": "E-1", "orderid": "O-1"}
    assert filled.by_tag(65_020).as_py() == expected
    assert filled.entries() == original.entries()
    assert filled.into_bytes(124) == wire and filled.digest() == original.digest()
    assert codec.enrich_message(filled) == filled

    table = codec.arrow_reader(fix_schema(registry), [filled]).read_all()
    assert table.schema.field("altids").type.keys_sorted
    restored, = codec.messages(table)
    assert restored.by_name("altids").as_py() == expected
    filled.set("altids", {})
    assert codec.enrich_message(filled).by_name("altids").as_py() == {}
    empty = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|"))
    assert empty.by_name("altids").as_py() == {}
    unknown = codec.enrich_message(codec.parse_fix_line(b"8=FIX.4.4|35=ZZ|10=0|"))
    assert unknown.get_by_name("altids") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)
    const wire = Buffer.from('8=FIX.4.4|35=8|37=O-1|11=C-1|17=E-1|10=0|')
    const original = codec.parseFixLine(wire)
    const filled = codec.enrichMessage(original)
    const expected = new Map([['clordid', 'C-1'], ['execid', 'E-1'], ['orderid', 'O-1']])
    assert.ok(filled.byTag(65020).asJs() instanceof Map)
    assert.deepEqual(filled.byTag(65020).asJs(), expected)
    assert.deepEqual(filled.arrivals(), original.arrivals())
    assert.deepEqual(filled.intoBytes(124), wire)
    assert.deepEqual(filled.digest(), original.digest())
    assert.ok(codec.enrichMessage(filled).equals(filled))

    const table = codec.arrowReader(fix.schema(registry), [filled]).intoTable()
    assert.equal(table.schema.fields.find((field) => field.name === 'altids').type.keysSorted, true)
    assert.deepEqual([...table.getChild('altids').get(0)], [...expected])
    const restored = [...codec.messages(table)]
    assert.equal(restored.length, 1)
    assert.deepEqual(restored[0].byName('altids').asJs(), expected)
    filled.set('altids', new Map())
    assert.deepEqual(codec.enrichMessage(filled).byName('altids').asJs(), new Map())
    const empty = codec.enrichMessage(codec.parseFixLine(Buffer.from('8=FIX.4.4|35=0|10=0|')))
    assert.deepEqual(empty.byName('altids').asJs(), new Map())
    const unknown = codec.enrichMessage(codec.parseFixLine(Buffer.from('8=FIX.4.4|35=ZZ|10=0|')))
    assert.equal(unknown.getByName('altids'), null)
    ```

### A composed key fills the field its last segment names

A bridge writes a field under its own namespace, so one row carries `TECH.CLIENTID`, `ULLINK.INSTRUMENTID`, `FIRM.ORIG.ULFROMSESSIONNAME` and `OMSVENDOR.CALC.EXECBROKER` beside its plain keys. The fact is the field's however the writer spelled the key: where the last dotted segment of a child's name resolves to a dictionary field and that field is absent, the composed key fills it, and the filled child takes the field's tag, so every rule after it reads the value like any other. A segment naming no field of this dictionary names nothing, and the composed child stays in the row exactly as it arrived.

One voice or silence. Where a row names one absent field under several composed keys and they do not agree, none of them fills it. That is not a precaution: on nine lines of the committed corpus `FIRM.ORIG.CLIENTID` is `3000090.006` and `ULLINK.CLIENTID` is `trader1` with `CLIENTID` absent - a firm account number and a trader login, and nothing in the row says which one the field means. Filling from either would invent a fact; filling from neither states what the row actually settled, which is nothing.

Never overwriting is load-bearing here too, and the same corpus proves it: `CLIENT.SYMBOL` is `XAU` where `SYMBOL` is `XAU/USD`, and `OMSVENDOR.CALC.EXECBROKER` is `SWXCCP` where `EXECBROKER` is `2003103.001`. A namespace's spelling of a fact is not the fact.

The cancel reject the corpus ends on shows the fill and its bound side by side: `OMSVENDOR.ORDERQTY=10000` fills the `OrderQty` the row never states and `OMSVENDOR.TIMEINFORCE=day` its `TimeInForce`, each composed pair staying the arrival it was under its own dotted name and tag 0, while `FIRM.ACRONYM`, `ULLINK.INSTRUMENTID` and `ULLINK.BYPASSRISK` name no field of the dictionary and stay under their namespaces, read back by `by_name("firm.acronym")` and never as `acronym`.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));
    let enriched = |row: &[u8]| -> yggdryl::Result<FixMsg> {
        reader.enrich_message(reader.parse_line(row)?.next().expect("one row")?)
    };

    // The namespace is the writer's; the fact is the field's.
    let filled = enriched(b"MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|")?;
    assert_eq!(filled.by_name("Account")?.as_str(), Some("ACCT-000117"));

    // Two namespaces naming one absent field, disagreeing: nothing fills it.
    let split = enriched(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=3000090.006|ULLINK.CLIENTID=trader1|")?;
    assert_eq!(split.get_by_name("ClientID"), None);

    // Agreeing, they fill; and a namespace never lands over a stated value.
    let agreed = enriched(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|")?;
    assert_eq!(agreed.by_name("ClientID")?.as_str(), Some("trader1"));
    let stated = enriched(b"MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|")?;
    assert_eq!(stated.by_name("Symbol")?.as_str(), Some("XAU/USD"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    def enriched(row):
        return reader.enrich_message(next(reader.parse_line(row)))

    # The namespace is the writer's; the fact is the field's.
    filled = enriched(b"MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|")
    assert filled.by_name("Account").as_py() == "ACCT-000117"

    # Two namespaces naming one absent field, disagreeing: nothing fills it.
    split = enriched(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=3000090.006|ULLINK.CLIENTID=trader1|")
    assert split.get_by_name("ClientID") is None

    # Agreeing, they fill; and a namespace never lands over a stated value.
    agreed = enriched(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|")
    assert agreed.by_name("ClientID").as_py() == "trader1"
    stated = enriched(b"MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|")
    assert stated.by_name("Symbol").as_py() == "XAU/USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const enriched = (row) => reader.enrichMessage(reader.parseLine(Buffer.from(row)).next().value)

    // The namespace is the writer's; the fact is the field's.
    const filled = enriched('MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|')
    assert.equal(filled.byName('Account').asJs(), 'ACCT-000117')

    // Two namespaces naming one absent field, disagreeing: nothing fills it.
    const split = enriched('MSGTYPE=8|FIRM.ORIG.CLIENTID=3000090.006|ULLINK.CLIENTID=trader1|')
    assert.equal(split.getByName('ClientID'), null)

    // Agreeing, they fill; and a namespace never lands over a stated value.
    const agreed = enriched('MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|')
    assert.equal(agreed.byName('ClientID').asJs(), 'trader1')
    const stated = enriched('MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|')
    assert.equal(stated.byName('Symbol').asJs(), 'XAU/USD')
    ```

### Edges

- A value the column refuses is silence, not a failure: `SecurityID` under source `4` spelling `XX0000000001`, whose check digit does not close it, leaves `isincode` null, and nothing downstream reads a country off it.
- A value that would not type - `201=abc` in the `PutOrCall` column - is a null the row holds while the entry keeps the text; a derivation fills the null in place, so the row has one column for the tag and the entry still says `abc`.
- An absent input is silence: a report stating `LastQty(32)` and no `LastPx(31)` derives no `GrossTradeAmt(381)`, because `lastqty * lastpx` over a null is null, and no term guesses.
- The derivations read codes, and a venue's own word for one is not the code. A bridge row spelling `SECURITYIDSOURCE=isin` beside a `SECURITYID` the check digit closes has stated a source, which stands, and `isin` is not `4` - the dictionary names that code `ISINNumber`, so nothing translated it - so the ISIN column is left null; the same row spelling `SECURITYTYPE=equity` names no code of the `SecurityType` set, so no group files it and no CFI is read off it. The bridge's own `ISINCODE` states the column directly, and the `CFICODE` it spells beside it states the product.
- A trade stating no quantities states no status: `150=F` alone leaves `OrdStatus` absent, and `state` then holds what the report said happened, `F` as the column spells it, `40TRADE`.
- An option stating no `PutOrCall` gets a CFI whose exercise is `X`, and `PutOrCall` is not then read back off it: `X` is the code for an exercise left open.
- A derivation the dictionary cannot compile refuses the first enrichment, naming the field, and reads no message for it: `lastqty * nosuchfield` on `GrossTradeAmt` names a column no field or group of the registry answers to, and `lastqty * symbol` does not bind. The refusal is every door's: `enrich_message`, `enrich_messages`, `into_row` and the batch reader answer it alike, and the registry keeps it as it keeps a compiled list, so a refused registry compiles once and refuses every ask until a field changes. A text that is not a term is refused earlier still, by the registry, at insert, update and load - a store whose shard spells `lastqty *` on a field does not load, and neither does a JSON snapshot.
- A registry built from a handful of fields enriches: the crate columns' own derivations read the standard's fields by name, and a name such a registry lacks is a column no message states, null in the working schema, rather than a refusal.
- A `SecurityID` under source `4` that no check digit closes falls through to the alternate identifier whose source is `4`: `22=4|48=NOTANISIN00` beside `455=CH0012221716|456=4` fills `isincode` with the alternate and `CountryOfIssue` with `CH`; a primary the digit closes answers itself whatever the alternate says; a message neither closes answers nothing.
- `CountryOfIssue` answers for exactly the 249 prefixes `StringEnum::COUNTRIES` lists, and for no other of the 676 pairs: `AB`, `UK`, `TP` and `XS` are silence, `GB` and `US` answer.
- A report with nothing left that states a canceled quantity ordered what it did plus what it canceled: `39=4|14=40|84=60` derives `OrderQty` 100 and `LeavesQty` 0, and `151=0` stated beside them changes nothing - Appendix D's cancel, where the deleted hand-laid order answered `CumQty` alone. A working report stating what is left answers done plus left, whatever it canceled.
- The fixpoint fills what one hand-laid pass could not: `32=10|194=1.25|195=0.25` on a report derives `LastPx` 1.5 and then `GrossTradeAmt` 15, and `AvgPx` where `CumQty` is the fill; `150=0|14=0|84=100` derives `OrderQty` 100 and then `LeavesQty` 100; `32=10|2353=2|231=5|31=3` derives `TotalTradeQty` 20 and then the multiplied and the gross quantities off it.
- A term reads a field as what it is: `PossDupFlag(43)` is a boolean, so `43=Y` fills `OrigSendingTime` from `SendingTime`, and `TradingUnitPeriodMultiplier(2353)` is an integer, so `32=10|2353=2` fills `TotalTradeQty` 20 - both where the deleted rules, reading text or a float, never fired.
- A code is compared exactly, because FIX codes are case-sensitive: `22=A` is Bloomberg's source and fills `Symbol` from `SecurityID`, and `22=a` is no code of the set and fills nothing, where the deleted rule folded case.

## A column is filled by the tag its field carries

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [lookup](registry.md) would be a hash and a verification. None of that runs per row: the schema is fixed, its columns are named `msgtype` and `symbol`, each carries its field's `fix:tag`, and `into_row` fills each one by that tag. `fix_column_tags` reads the tags off a schema once, so a batch of a million rows reads them once rather than once per row; a caller-declared root that spells a column by its tag's digits is read the same way, the digits answering where the field carries no tag.

So there is nothing beside the schema to build, hold, or invalidate. A caller finds a column with `index_of` on the schema it already has - or with `fix_column_of` and the tag - and two captures sharing a dictionary share both the schema and every position in it. A List group column is filled by its `fix:counter`, while the numeric count stays in its own column; the self-counting `altids` Map occupies only its own column.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `into_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

Map groups use the same row and Arrow doors, preserving key/value fields, non-null keys and `keys_sorted`; a null map, an empty map and a map with a null value remain distinct.

## A capture's own columns lead the row

A line's URL, line number, timestamp and other capture fields lead its FIX columns. A source row produces one output row per message - one per frame a line carried, one for a JSON document - and each of them receives the same carried values from that row; a row that carried no message produces none.

A carried column whose folded name a FIX column already takes - a `MsgCtxId` capture beside `msgctxid`, a text reader's `msgtype` beside the FIX one - is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema. What it stated is not lost, because the row [fills that column from it](arrow.md#a-column-is-the-caller-speaking-per-row). A `msgdirection` column is the row's stated direction, read as a parameter and carried nowhere else.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixRegistry, fix_schema, fix_schema_carrying};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    let capture = DataType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?
    .required_field("line");

    let plain = fix_schema(&registry, "FixMessage")?;
    let carried = fix_schema_carrying(&capture, &plain)?;

    assert_eq!(carried.fields()[0].name(), "url");
    // The FIX columns keep their order; they only start further along.
    assert_eq!(carried.index_of("msgtype"), plain.index_of("msgtype").map(|at| at + 3));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import DataType, Field
    from yggdryl.fix import FixRegistry, fix_schema, fix_schema_carrying

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    capture = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", DataType("utf8"), nullable=False),
                Field("rownum", DataType("int64"), nullable=False),
                Field("body", DataType("utf8"), nullable=False),
            ]
        ),
        nullable=False,
    )

    plain = fix_schema(registry, "FixMessage")
    carried = fix_schema_carrying(capture, plain)

    assert carried.get_field_at(0).name == "url"
    # The FIX columns keep their order; they only start further along.
    assert carried.index_of("msgtype") == plain.index_of("msgtype") + 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fields, fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const capture = fields.struct(
      'line',
      [
        fields.utf8('url', { nullable: false }),
        fields.int64('rownum', { nullable: false }),
        fields.utf8('body', { nullable: false }),
      ],
      { nullable: false },
    )

    const plain = fix.schema(registry, 'FixMessage')
    const carried = fix.schemaCarrying(capture, plain)

    assert.equal(carried.fieldAt(0).name, 'url')
    // The FIX columns keep their order; they only start further along.
    assert.equal(carried.indexOf('msgtype'), plain.indexOf('msgtype') + 3)
    ```

## Edges

- A tag the dictionary does not have is skipped rather than invented: a column with no field behind it could not be typed.
- A column whose field carries neither a `fix:tag` nor a `fix:counter`, and whose name spells no tag, is the capture's own, so `into_row` answers null there; whoever read the capture fills it.
- `SendingTime(52)` and `TransactTime(60)` are typed by the registry's own declarations - `FixRegistry::new` seeds both where a dictionary defines neither - and a declaration that is not a nanosecond UTC instant is refused at intake; a stated clock that does not read as an instant is a located error item, never a default.
- A column of the crate's own is typed by the crate's definition, on a tag from 65001 that no dictionary publishes: `updatedat` is an instant, `msghash` sixteen fixed bytes, `miccode` a `mic`, `state` a `state`, whatever text a venue spelled them in.
- A capture's own `timestamp` is context, not a FIX clock: it leads the row as a carried column and never overrides the message's `TransactTime`, then `SendingTime`, reading.
- `altids` is filled by enrichment, never derived by `into_row` alone. A known message with no stated identifiers gets an empty map; an unknown message remains null in the fixed row. Invalid UTF-8 in a selected binary identifier raises the core's located conversion error.
- Typed text drops the replacement character and every control character but tab, so a byte a transport mangled does not become a mangled column; the entry keeps the bytes exactly as they arrived.
- `index_of` on a column the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- Nothing drops a row unasked: `FixDedup`, Rust only, drops adjacent republications over any message stream, and is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) the caller composes.

## Performance

What one line of each shape costs the codec - a framed tag stream, a bare one, a bridge row keyed by name, a packed occurrence, a document - is measured where the shapes are read together, in [`fix/pipeline`](arrow.md#performance): the `parse_lines` row is the codec over every body of a real capture, and a bridge row of a hundred named keys costs it a hundred dictionary lookups where a frame of twenty tags costs twenty.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix schema::
    cargo test -p yggdryl --test fix codec::a_bridge_frame
    cargo run -p yggdryl-cli -- fix schema --root config/fix
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "reader or row or crate"
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/fix/*.test.js"
    node scripts/build_docs_fix.js --check
    ```
