# Capture

A day of session log is a table. This page is the road from one to the other: [`FixCodec`](#a-reader-is-the-whole-parse-surface) turns a captured line into the [messages](message.md) it holds, [`fix_schema`](#the-columns-are-the-folded-names) is the one row shape every message answers as, and [`FixMsg::into_row`](#a-column-is-filled-by-the-tag-its-field-carries) fills it - by the tag each column's field carries, so nothing is resolved against the [dictionary](registry.md) per row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec` and its `parse_*` readers, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `fix_column_of`, `fix_column_tags`, `FixMsg::into_row`, `fix_crate_fields` |
| Columns | named by the field's folded canonical name - `msgtype`, never `35` and never `msg_type`; the display spelling stays on the field's `display`, the tag on its `fix:tag`, and a named group column's counter on its `fix:counter` |
| Shape | standard header, the fields a consumer reads, three List groups, the trailer, the crate's 24 scalar fields and the `altids` Map group, FIX's own `msgdirection`, then the one `nofixentries` record: 105 tags from `fix_schema_tags`, 109 columns with the shipped registry, each List group adding its column beside its counter |
| Identifiers | enrichment fills the nullable, sorted `altids` Map from the message's direct `fix:identifiers`; a stated map is preserved, including an empty one |
| Non-null | `beginstring`, `sendingtime`, `updatedat`, `timepartition`, `uuid`, `puuid`, `createdat`, `code`, `snapshotat`; `version` is populated at construction but its column remains nullable |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `nofixentries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Expansion | a line yields one message per [frame it carries](decode.md#a-line-yields-none-one-or-many-messages) and none where it carries none; a bulk configuration yields one per configuration a response named, and none for a response that named none - an error-only answer, a request-only document, an empty bulk or wildcard answer |
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

The verb is `parse`, and no reader takes a flag: what happens to a message once it is built - [filling](#what-a-message-implied-is-filled-in), [restating](message.md#restated-at-the-dictionarys-newest-version), [stamping](lifecycle.md) - is a call over the stream, never an argument to the parse.

| Reader | Takes | Answers |
| --- | --- | --- |
| `parse_line` | one captured line, the verb and prose around the frame included | `FixMessages`, a lazy fallible iterator: [none, one or many](decode.md#a-line-yields-none-one-or-many-messages) - one per frame, one per configuration a bulk answer named, none for a line that states no message |
| `parse_lines` | any iterator of lines | a lazy iterator of `Result<FixMsg>`; a line that is not a row is an `Err` item and the stream continues |
| `parse_text_line` | one [decoded line](../media/text/index.md#row-schema), its body and [row-header captures](arrow.md#a-column-is-the-caller-speaking-per-row) | `FixMessages` |
| `parse_text_lines` | any iterator of owned or borrowed lines, or `Result`s of them | a lazy iterator of `Result<FixMsg>`; lines are borrowed without cloning and a source error is moved into the stream unchanged |
| `parse_text_arrow_reader` | a `BatchReader` of text records | a `BatchReader` of [fixed rows](arrow.md) |
| `parse_plugin_line` | a bulk or wildcard configuration body | `FixMessages`; a body that is not a Jolokia answer names no configuration and answers none, refusing nothing |
| `parse_fix_line`, `parse_fixml_line`, `parse_ullink_line`, `parse_pairs` | one body of that dialect, or pairs already split | one `FixMsg`; a body holding [a second frame](decode.md#a-line-yields-none-one-or-many-messages) is refused |

A stream adapter owns a clone of the codec and borrows nothing, so `codec.arrow_reader(schema, codec.parse_lines(lines))` composes without the codec outliving the stream. Python exposes native iterators; JavaScript uses `IterableIterator<FixMsg>`. Every stream door fuses its source's exhaustion. Schema construction and group-plan resolution happen before repeated values are processed.

Every one of them ends in the same builder, so a document is typed by the rules that type a frame - one nesting builder, one fold, one code translation, one value contract. Each is the core's own method under the same name in all three languages.

### Lines are a stream

`parse_lines` is the line iterator everything else is built on: nothing is collected, and a line answers [every message it carries](decode.md#a-line-yields-none-one-or-many-messages) - two where a relay wrote two frames on one line, none where the line is a sentence, one per configuration where it is a bulk answer. A line the reader refuses is an `Err` item the stream continues past: one corrupt line must not end a run over ten million. The example opens with a bridge frame: `#`-prefixed name keys, and one group occurrence whose value packs its members behind the two control bytes ULLINK uses.

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

### Edges

- Ordinary unframed text produces no message at all - a line that opens no frame, states no bridge pair and carries no document states nothing to read, so it is no row either. A payload that was there and would not parse is a message with nothing in it, so malformed syntax never fails the batch it arrives in; a stated mandatory clock or identity that does not read is a located error item. Malformed configuration input returns a located error; a fallible message iterator stops after that error.
- A bridge key's `#` is judged against the row's bare spellings, in a bridge row and in the name keys a bridge writes into a numeric frame alike. Alone, it drops: `#ORDERID=123` is the dictionary's `OrderID`. Restating a bare pair's bytes, the marked pair is a second spelling of one pair and goes, row and entries alike: `ORDERID=123|#ORDERID=123` is `OrderID` once, and `into_bytes` re-emits the one pair. Beside a bare twin stating other bytes it stays verbatim, because collapsing the two would merge two values under one name: `ORDERID=123|#ORDERID=345` is `OrderID` 123 beside `#ORDERID` 345 - its own column, its own entry - whichever arrived first. The twin is matched by the fold every key resolves under, so `OrderId` and `ORDER_ID` twin it too, and by its stem, so a bare `NOPARTYIDS` group claims every `#NOPARTYIDS[n]` however many the two state - each stays whole under its own name, the count beside them, and none lands in the dictionary's group. A marked group goes only whole: a marked count restating the bare one beside occurrences the bare group never numbered stays with them, and only a marked group restating the bare group pair for pair goes. A bare pair whose value is a stated absence is no twin, because a key that said nothing was sent is not a key that was sent; a value is compared as its bytes, because `abc` is not `ABC`; and the twin is a spelling, never an identity, so a tag and a marked name - `55=AAPL|#SYMBOL=AAPL` - state two values exactly as a tag and a bare name do. In a numeric frame the marks are judged and the keys kept as they are: a packed occurrence there is one value, as a bare one always was. A key marked twice is judged one mark at a time: `##ORDERID` twins `#ORDERID` as `#ORDERID` twins `ORDERID` - restating it goes, beside other bytes it stays, alone it loses one mark.
- A row's message type resolves the way every key does, in the one namespace: a name reaches the message of that name, and a bare code the message tag 35's code set names, else the first in name order. A bridge row calling itself `tradecapturereport` reads against the message of that name, which is what places a counter half the dictionary shares - `NoLegs`, `NoSides` - under the group that message declares.
- A stated absence - one of `null_values` - produces no field and no entry, because a key that said nothing was sent is not a key that was sent.
- A pinned `version` decides which code spelling a value translates through, never what a field is called: a tag is one column under the name the dictionary holds it by, and what each version called it stays readable through the field's lineage.

## The columns are the folded names

`msgtype`, never `35` and never `msg_type`. A column is spelled the way the dictionary spells the field's canonical name - ASCII case folded once, on the way in - so a row reads the way a message reads, in every binding and every catalog, and a reader spelling `row["msgseqnum"]` finds the sequence number without a dictionary in hand.

The tag is still the identity. Each column carries its field's `fix:tag`, its `display`, its lineage and its code set, and the row is [filled by that tag](#a-column-is-filled-by-the-tag-its-field-carries) rather than by the spelling, so a venue that renames a field between versions changes nothing about where its value lands. `fix_schema_tags` is the same row as tags, in the same order.

A List group column carries `fix:counter` beside the `fix:tag` its definition derives from its own name: the numeric count keeps its own column, and the group column beside it holds the occurrences. The built-in `altids` Map instead carries tag and counter 65020 on one group column, with no scalar count field; the Map's entries already determine its cardinality.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixRegistry, fix_column_of, fix_schema, fix_schema_tags};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    let schema = fix_schema(&registry, "FixMessage")?;

    let columns: Vec<&str> = schema.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(&columns[..3], ["beginstring", "bodylength", "msgtype"]);
    assert_eq!(columns.last(), Some(&"nofixentries"));
    assert_eq!(fix_schema_tags().len(), 105);
    assert_eq!(&fix_schema_tags()[..3], [8, 9, 35]);

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
    assert columns[:3] == ["beginstring", "bodylength", "msgtype"]
    assert columns[-1] == "nofixentries"
    assert len(fix_schema_tags()) == 105
    assert fix_schema_tags()[:3] == [8, 9, 35]

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

    assert.equal(schema.fieldAt(0).name, 'beginstring')
    assert.equal(schema.fieldAt(2).name, 'msgtype')
    assert.equal(schema.fieldAt(schema.fieldLen - 1).name, 'nofixentries')
    assert.equal(fix.schemaTags().length, 105)
    assert.deepEqual(fix.schemaTags().slice(0, 3), [8, 9, 35])

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert.equal(schema.field('msgtype').display, 'MsgType')
    assert.equal(schema.field('msgtype').fix.tag, 35)
    ```

## Nothing is lost at the end

One record closes every row.

`nofixentries` is the whole arrival record: every pair the reader read as sent - a stated absence and a bridge's marked restatement of a bare pair are read as never sent, as the [edges](#edges) above state - in arrival order, untranslated, and a group's members riding under the counter pair that heads them. It is a list of `fixentry` structs, each the tag its key resolved to, that key, its value, and what arrived under it. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

An entry says what arrived and only that. A key and a value are ranges of the line the message was read from, so an entry never carries a key that appears nowhere in that line: a bridge packing a whole occurrence into one value - `#NOPARTYIDS[0]=PARTYID=BUYSIDE...PARTYROLE=1` - is recorded as the pair the bridge wrote, and the members read out of it fill `parties[0].partyid` and its siblings in the row. No dialect is there either: a message is not a dictionary member, and which dictionaries a field belongs to is the field's own `fix:branches` in the registry.

A key no dictionary explains is in the same record, with tag 0 beside its exact raw key, value, order and children: an unresolved name, an unresolved numeric wire key and an occurrence key - indexed or packed, which names no field - alike, while a resolved scalar or group key keeps its positive canonical tag. Tag 0 is arrival provenance and never a registry identity: `fix:tag`, `fix:tags`, `fix:counter` and `FixId::of` take positive tags only. So a venue onboarding a new field finds it by filtering one column for tag 0. The column materializes three `fixentry` levels and folds anything deeper into a canonical JSON leaf, which [`from_row`](message.md#a-row-is-a-message-again) validates and reads back.

## The crate's own columns

Twenty-four scalar fields and one Map group carry capture facts that no dictionary publishes. Every registry holds them from construction, and the [store](store.md) omits their definitions: `fix_crate_fields` lists all 25 in tag order, while scalar registry iteration counts the 24 scalars. Their tags run from 65001 - `CRATE_TAG_MIN` (65000) starts the reserved block, whose retired first slot is not reused - `SENDERSESSIONID_TAG_NAME` and its siblings hold each `(tag, name)` pair, and `is_crate_tag` tests ownership. `altids` is a group reached by `group_by_tag(65020)` or group definition name, not by the registry's scalar-field doors.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `version` | `Version` | 65001 | the FIX version it was *read* at, which is not always what `BeginString` claimed |
| `symbolticker` | `SymbolTicker` | 65002 | one instrument symbol that is the same across venues |
| `updatedat` | `UpdatedAt` | 65003 | the settled message instant: `snapshotat` at intake, truncated to the snapshot grid by the [lifecycle](lifecycle.md); non-null |
| `timepartition` | `TimePartition` | 65004 | the partition `updatedat` falls in, as whole seconds; non-null |
| `parentclordid` | `ParentClOrdID` | 65005 | the client order identifier this order descends from |
| `parentorderid` | `ParentOrderID` | 65006 | the venue order identifier this order descends from |
| `sendersessionid` | `SenderSessionId` | 65007 | the session the message came from: its own statement, a bridge row's `SESSIONID` by alias, else the session instance the bracket in front of the line names |
| `msgctxid` | `MsgCtxId` | 65008 | the message context a bridge handled the message in, from its log's bracket |
| `pluginid` | `PluginId` | 65009 | the plugin that logged the line, as a bridge names it: a row's own `pluginid` capture or column fills it, and it selects nothing - no dictionary, no version |
| `prevpluginid` | `PrevPluginId` | 65010 | the plugin a message came through before the one that logged it, as a bridge names it; only ever a capture of that name, never derived |
| `sendersessionname` | `SenderSessionName` | 65011 | the name of the session the message came from: a bridge row's `ULFROMSESSIONNAME`, else the session that logged a line it sent |
| `targetsessionname` | `TargetSessionName` | 65012 | the name of the session the message went to: a bridge row's `ULTOSESSIONNAME`, else the session that logged a line it received |
| `isincode` | `ISINCode` | 65013 | the instrument's ISIN, as an [`isin`](../types/codes.md): `SecurityID(48)` where `SecurityIDType(22)` says ISIN, else the `SecurityAltID(455)` whose `SecurityAltIDType(456)` does |
| `miccode` | `MICCode` | 65014 | the market the message names, as a `mic`: `SecurityExchange(207)`, else `ExDestination(100)`, else `LastMkt(30)` |
| `state` | `State` | 65015 | the order's state, as a `state`: `OrdStatus(39)`, else `ExecType(150)` |
| `instuuid` | `InstUuid` | 65016 | the instrument's version-8 UUID over the xxh128 digest of its market, classification, ISIN - else symbol - and currency, the same across venues that spell it alike; stamped by the lifecycle |
| `uuid` | `Uuid` | 65017 | the message's identity: a version-8 UUID of signed `updatedat` nanoseconds and 58 bits of the XXH64 of its named content; non-null |
| `puuid` | `PUuid` | 65018 | the chain's identity: a version-8 UUID over the XXH3-128 of `code` alone; non-null |
| `targetsessionid` | `TargetSessionId` | 65019 | the session the message went to, as the message itself states it |
| `altids` | `AltIds` | 65020 | a nullable sorted Map of this message's direct declared identifiers, keyed by canonical member name; group members are not flattened |
| `prevupdatedat` | `PrevUpdatedAt` | 65021 | the previous message's `updatedat` in the selected chain; nullable |
| `prevuuid` | `PrevUuid` | 65022 | the previous message's `uuid` in the selected chain; nullable |
| `createdat` | `CreatedAt` | 65023 | the creation instant: `snapshotat` at intake, the first accepted message's in a live chain; non-null |
| `code` | `Code` | 65024 | the exact chain name, empty when unknown; non-null |
| `snapshotat` | `SnapshotAt` | 65025 | the real event instant: a stated one, else `TransactTime(60)`, else `SendingTime(52)`; non-null |

`uuid` is the one stored message identity and `puuid` the chain's; what each hashes, and why a projection that adds or renames columns may move `uuid` while unchanged named content keeps it, is the [message's identity](message.md#clocks-and-identity). `FixMsg::digest` is a separate contract: the XXH3-128 of the arrival record with the standard header, the standard trailer and the crate's own tags left out, `MsgType` excepted, so two identical orders sent a second apart, or relayed through two sessions, digest equal.

Session columns come from bridge pairs or [row-header captures](arrow.md#a-bridge-log-names-what-it-fills), without overwriting a value the message stated. `isincode`, `miccode` and `state` are derived when a message becomes a row; `code`, `instuuid`, `prevupdatedat`, `prevuuid` and the grid `updatedat` are stamped by the [lifecycle](lifecycle.md). `altids` is filled by [enrichment](#a-messages-direct-identifiers-fill-one-map), not by row projection. FIX's `SenderCompID` and `TargetCompID` name counterparties; the plugin carrying a message inside a bridge is a separate fact.

`timepartition` declares more than a type, in the protocols the crate already has rather than in a spelling only a FIX reader would know to look for: `field:partition` marks it as the column a layout is cut on and an Iceberg spec partitions by identity, `partition:sources` says it reads `updatedat`, and `transform:expression` says how - `truncate(updatedat, 'hour')`, the expression layer's own floor to the hour, which is what `time_partition()` answers for a row and what `Field::apply_arrow_batch` fills into a batch that lacks the column.

### Every message is dated and versioned

These columns are filled when a message is built, whatever its line carried, and none of them becomes an entry unless the wire sent it - so `into_bytes` still re-emits the wire byte for byte.

`beginstring` is the wire's own `BeginString(8)` when stated, else `FIX.<version>` for the version the message was read at: the row's own `beginstring` [column or capture](arrow.md#a-column-is-the-caller-speaking-per-row), else the pinned `version`, else the one `ApplVerID(1128)` or `BeginString(8)` implied, else the dictionary's newest, else 4.4. A bridge row and a configuration document therefore say which FIX they were read as exactly as a frame does.

`version` states that same answer outright, on every message the codec generates, because `BeginString` is what the message says about *itself* and a session that mislabels itself - or that carries a row written to a later FIX than it speaks - makes the two differ. `FixMsg::version()` answers the crate's column where a read stamped one and `BeginString` otherwise, so it always answers for a built message.

Seven values close every message and are never null: `SendingTime(52)`, `snapshotat`, `updatedat`, `createdat`, `code`, `uuid` and `puuid`. Initial intake settles them once. `SendingTime` is the message's own, else a carrier row's, else the codec's `default_sending_time`, else one UTC-now read for that undated message; `snapshotat` is a stated one, else `TransactTime(60)`, else that `SendingTime`; `updatedat` and `createdat` default to `snapshotat`, and `code` to empty. A stated clock that is not an instant is a located refusal, never an absent clock a default overwrites. No wall clock is read after intake - enrichment, writes, row exchange and replay carry the settled values - so a read that must be reproducible pins `default_sending_time` or carries the settled rows. `updatedat()`, `createdat()`, `uuid()` and `puuid()` borrow theirs without a lookup, and `time_partition` floors `updatedat` to the partition width from its nanoseconds, so a clock stated to the millisecond has a partition.

The root's children are the standard header in its declared order, the body as it arrived, the standard trailer, then whichever of the seven the message did not state. The fixed schema declares those seven, `beginstring` and `timepartition` non-null, and every other column nullable. The crate's own definitions come first in the example, then two messages the codec dates.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{CODE_TAG_NAME, FixCodec, FixRegistry, SNAPSHOTAT_TAG_NAME, Scalar, TimeUnit, Timezone, fix_crate_fields};

    let fields = fix_crate_fields()?;
    assert_eq!(fields.len(), 25);
    let partition = &fields[3];
    assert_eq!(partition.name(), "timepartition");
    assert_eq!(partition.display(), Some("TimePartition"));
    assert!(partition.is_partition());
    assert_eq!(partition.as_partition().sources()?, Some(vec!["updatedat".to_owned()]));
    assert_eq!(partition.get_metadata("transform:expression"), Some("truncate(updatedat, 'hour')"));

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let default = Scalar::datetime64(1_704_190_530_000_000_000, TimeUnit::Nanosecond, Timezone::UTC)?;
    let reader = FixCodec::new(registry)
        .with_version("4.4".parse()?)
        .try_with_default_sending_time(Some(default.clone()))?;

    // A frame stating neither its version nor a clock is still versioned,
    // and dated by the codec's default sending time.
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
    // sub-second clock has a partition.
    let sent = reader
        .parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(sent.by_tag(8)?.as_str(), Some("FIX.4.2"));
    assert_eq!(sent.by_tag(52)?.temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_200_415));
    assert_eq!(sent.updatedat().temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_199_900));
    assert_eq!(sent.by_tag(SNAPSHOTAT_TAG_NAME.0)?, sent.updatedat());
    assert_eq!(sent.time_partition().temporal_count_at(TimeUnit::Second), Some(1_787_306_400));
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_crate_fields

    CODE, SNAPSHOTAT = 65024, 65025
    fields = list(fix_crate_fields())
    assert len(fields) == 25
    partition = fields[3]
    assert partition.name == "timepartition"
    assert partition.metadata["display"] == "TimePartition"
    assert partition.is_partition
    assert partition.metadata["partition:sources"] == '["updatedat"]'
    assert partition.metadata["transform:expression"] == "truncate(updatedat, 'hour')"

    default = datetime(2024, 1, 2, 10, 15, 30, tzinfo=timezone.utc)
    reader = FixCodec(
        FixRegistry.from_handle(Path("config/fix").resolve()),
        version="FIX.4.4",
        default_sending_time=default,
    )

    # A frame stating neither its version nor a clock is still versioned,
    # and dated by the codec's default sending time.
    bare = next(reader.parse_line(b"35=D|55=AAPL|10=0|"))
    assert bare.by_tag(8).as_py() == "FIX.4.4"
    assert bare.by_tag(52).as_py() == default
    assert bare.updatedat() == bare.createdat() == bare.by_tag(52)
    assert bare.by_tag(CODE).as_py() == ""
    # None of them became an entry, so the wire comes back byte for byte.
    assert bare.into_bytes(ord("|")) == b"35=D|55=AAPL|10=0|"

    # A frame stating its clocks keeps them: TransactTime is the event, and a
    # sub-second clock has a partition.
    sent = next(reader.parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|"))
    assert sent.by_tag(8).as_py() == "FIX.4.2"
    assert sent.by_tag(52).as_py() == datetime(2026, 8, 21, 10, 30, 0, 415000, tzinfo=timezone.utc)
    assert sent.updatedat().as_py() == datetime(2026, 8, 21, 10, 29, 59, 900000, tzinfo=timezone.utc)
    assert sent.by_tag(SNAPSHOTAT) == sent.updatedat()
    assert sent.time_partition().as_py() == datetime(2026, 8, 21, 10, 0, tzinfo=timezone.utc)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, Scalar } = require('yggdryl')

    const [CODE, SNAPSHOTAT] = [65024, 65025]
    const fields = fix.crateFields()
    assert.equal(fields.length, 25)
    const partition = fields[3]
    assert.equal(partition.name, 'timepartition')
    assert.equal(partition.display, 'TimePartition')
    assert.ok(partition.isPartition)
    assert.equal(partition.getProperty('partition', 'sources'), '["updatedat"]')
    assert.equal(partition.getProperty('transform', 'expression'), "truncate(updatedat, 'hour')")

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry, {
      version: 'FIX.4.4',
      defaultSendingTime: new Date(Date.UTC(2024, 0, 2, 10, 15, 30)),
    })

    // A frame stating neither its version nor a clock is still versioned,
    // and dated by the codec's default sending time.
    const bare = reader.parseLine(Buffer.from('35=D|55=AAPL|10=0|')).next().value
    assert.equal(bare.byTag(8).toJSON(), 'FIX.4.4')
    assert.ok(bare.byTag(52).equals(reader.defaultSendingTime))
    assert.ok(bare.updatedat().equals(bare.byTag(52)))
    assert.ok(bare.createdat().equals(bare.byTag(52)))
    assert.equal(bare.byTag(CODE).asJs(), '')
    // None of them became an entry, so the wire comes back byte for byte.
    assert.equal(bare.intoBytes('|'.charCodeAt(0)).toString(), '35=D|55=AAPL|10=0|')

    // A frame stating its clocks keeps them: TransactTime is the event, and a
    // sub-second clock has a partition.
    const sent = reader
      .parseLine(Buffer.from('8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|'))
      .next().value
    assert.equal(sent.byTag(8).toJSON(), 'FIX.4.2')
    assert.ok(sent.updatedat().equals(sent.byTag(60)))
    assert.ok(sent.byTag(SNAPSHOTAT).equals(sent.updatedat()))
    assert.ok(sent.timePartition().equals(Scalar.datetime(1_787_306_400_000_000_000n, 'ns', 'UTC')))
    ```

## What a message implied is filled in

A venue sends what its counterparty needs and nothing more, so a row is routinely missing values the message itself already determines: a report stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill stating `LastQty` and `LastPx` has said what it was worth, and a message naming its instrument by an ISIN has said which country issued it. Enrichment is a call over what the reader built: `enrich_message` fills one message, `enrich_messages` a stream of them, which fills a message from what an earlier one in the stream said too, [remembering the plugin configurations it passed](#a-stream-remembers-the-configurations-it-passed); and [`enrich_messages_arrow_reader`](arrow.md#filled-where-it-sits) a stream of batches, without parsing anything again. Those three are the whole of it: there is one enriching pass, and they are its doors.

The pass is three steps on one message, and their order is the pass's own rather than something a caller composes. It **restates** first - the row [re-expressed at the dictionary's newest version](message.md#restated-at-the-dictionarys-newest-version) - because every rule below reads by tag, and a child a session spelled under an alias with no tag is invisible until it has been canonicalized. It then **completes**, which is a read rather than a write, so that step adds nothing to the row. And it **fills** what the message implies and did not state, from three sources in turn: the composed keys the row carried, then the rules the specification licenses, then the plugin configurations this stream has already passed - in that order, because each may feed the next.

Completion writes nothing because a spelling is a way of asking rather than a thing to store. A consumer addressing `lastshares` finds the `lastqty` the message states: `get_by_name` resolves the spelling through the registry, which answers a field for any alias it holds. No alias twin is added as a second child, and the reason is not economy. Two of the forty-two shipped aliases are another field's canonical name - `quoteackstatus` is an alias of `QuoteStatus(297)` and the name of tag 1865, `tradetype` an alias of `BidTradeType(418)` and the name of tag 3006 - so a message stating both would hold two children of one name, and the root the pass builds would be refused whole. A twin could not cross the batch door either: [`fix_schema`](#the-columns-are-the-folded-names) names a column for a canonical field and for no alias, so `enrich_messages_arrow_reader` would discard on the way out what `enrich_messages` had just added, and the two doors would stop being one pass. And `get_by_tag` answers the earliest child on a tag, so where the twin was placed would silently decide what every column, every rule, every lift and the lifecycle read.

Three things hold whatever the rule. Only the row is filled, never the entries, so `into_bytes` re-emits the wire byte for byte either way. A rule answers only where every input is stated and typed: an identifier no check digit closes, a CFI whose category several security types share and a security type outside every group the specification files answer nothing rather than a guess. And a stated value is never overwritten, which is what makes a second pass change nothing - a value derived once is a stated value the second time.

The rules are the specification's own tables read as the implications they are, and each is one row of the table in `fix/enrich.rs` - a tag, the message types it speaks for, the conditions that must hold and the derivation - never code per field.

| Fills | From | The table it reads |
| --- | --- | --- |
| `SecurityIDSource(22)` | `SecurityID(48)` | the `SecurityIDSource` code set names the standard each code stands for, and each standard closes its identifiers with a check digit: `4` for a number ISO 6166 closes, `1` for a CUSIP, `2` for a SEDOL |
| `isincode` | `SecurityID(48)` under source `4`, else the `SecurityAltID(455)` whose `SecurityAltIDSource(456)` is `4` | ISO 6166; a spelling the check digit does not close is refused by the column and answers nothing |
| `SecurityID(48)` | `isincode` | a bridge row states the crate's column and has thereby stated the primary identifier, whose validation then states the source |
| `Symbol(55)` | `SecurityID(48)` under source `8` or `A`, else the `SecurityAltID(455)` whose source is `8` | the `SecurityIDSource` codes of an exchange symbol and a Bloomberg symbol |
| `CountryOfIssue(470)` | `isincode` | ISO 6166 opens a number with the ISO 3166 code of the numbering agency's country, where it is one: `XS` and `EU` answer nothing |
| `SecurityType(167)` | `CFICode(461)` | Appendix 6-D at its category level - `ES` is `CS`, `F` is `FUT`, an `O?F` is `OOF` and every other `O` is `OPT`, `LR` is `REPO`; a category every kind of bond shares, `DB`, answers nothing |
| `CFICode(461)` | `SecurityType(167)`, `PutOrCall(201)` | Appendix 6-D the other way: `CS` is `ESXXXX`, `CORP` is `DBXXXX`, `FRN` is `DBVXXX`, an option is `OC` or `OP` by its `PutOrCall` and `OX` without one |
| `PutOrCall(201)` | `CFICode(461)` | the second character of a listed (`O`) or unlisted (`H`) option: `C` is a call, `P` a put |
| `Product(460)` | `SecurityType(167)`, else `CFICode(461)` | the group the dictionary's `SecurityType` code set files the value under, as the `Product` code set spells it - `Agency` is `1`, `Corporate` `3`, `Currency` `4`, `Equity` `5`, `Government` `6`, `Loan` `8`, `Money Market` `9`, `Mortgage` `10`, `Municipal` `11`, `Financing` `13`; `Derivatives` and `Other` answer nothing. A CFI in category `E` is `5` and in `L` is `13` |
| `miccode` | `SecurityExchange(207)`, `ExDestination(100)`, `LastMkt(30)` | the first stated, as the [column](#the-crates-own-columns) is defined |
| `TimeInForce(59)` | nothing, on an order, a replace or a report | the field's own definition: absent means `0`, a day order |
| `OrdStatus(39)` | `ExecType(150)`; else `LeavesQty(151)` and `CumQty(14)` on a trade | the values the two code sets spell alike - not `D`, Restated in one and AcceptedForBidding in the other; a trade leaving nothing is filled, `2`, and one leaving something after doing something is partially filled, `1` - each landing as the [`state`](../types/codes.md) column spells it |
| `state` | `OrdStatus(39)`, `ExecType(150)` | the first stated, as the column is defined |
| `LeavesQty(151)`, `OrderQty(38)`, `CumQty(14)` | the other two, on a report | Appendix D: `OrderQty = CumQty + LeavesQty`, and nothing is left once `OrdStatus(39)` is closed |
| `GrossTradeAmt(381)` | `LastQty(32)` × `LastPx(31)` | Appendix D's execution reports |
| `SettlCurrAmt(119)` | `GrossTradeAmt(381)` × `SettlCurrFxRate(155)` | Appendix O |
| `Currency(15)`, `SettlCurrency(120)` | each other | Appendix O: a trade settling in the currency it was dealt in states it once |
| `AvgPx(6)` | `LastPx(31)` | Appendix D, only where `CumQty(14)` says the whole done quantity is this fill |
| `LastPx(31)` | `LastSpotRate(194)` + `LastForwardPoints(195)` | a forward price is quoted as a spot rate and the points away from it, and the points are already in price units, so the two add |
| `BidPx(132)`, `OfferPx(133)` | `BidSpotRate(188)` + `BidForwardPoints(189)`, `OfferSpotRate(190)` + `OfferForwardPoints(191)` | the same quoting, read on each side of a two-sided quote |
| `PeggedPrice(839)` | `PeggedRefPrice(1095)` + `PegOffsetValue(211)` | a pegged order's price is the reference it pegs to plus its own offset, which is signed: a peg below the reference is a negative one |
| `LastMultipliedQty(2368)` | `LastQty(32)` × `ContractMultiplier(231)` | a quantity in contracts times what one contract multiplies to is that quantity in units |
| `TotalTradeMultipliedQty(2370)` | `TotalTradeQty(2367)` × `ContractMultiplier(231)` | the same product, over the whole trade rather than one fill |
| `MinPriceIncrementAmount(1146)` | `MinPriceIncrement(969)` × `ContractMultiplier(231)` | the same product again: an increment in money is the increment in price times the multiplier |
| `TotalTradeQty(2367)` | `LastQty(32)` × `TradingUnitPeriodMultiplier(2353)` | a trade covering several trading unit periods trades its quantity once in each of them |
| `TotalGrossTradeAmt(2369)` | `LastPx(31)` × `TotalTradeQty(2367)` | the whole trade's worth, as `GrossTradeAmt` is the fill's |
| `OrderQty(38)` | `CumQty(14)` + `CxlQty(84)`, on a report | what a canceled order asked for is what it did plus what was canceled |
| `OrigSendingTime(122)` | `SendingTime(52)`, where `PossDupFlag(43)` is `Y` | the session layer's own definition: a possible duplicate carries the clock of the send it repeats |
| `CurrencyCodeSource(2897)` | `Currency(15)` stated | ISO 4217, `6` - the only source FIX's own `Currency` field is written in |

The rules run in one order, laid out so every chain ends in one pass: a `SecurityID`'s validation states the source, under which the ISIN column is read; an ISIN found only among the alternate identifiers becomes the `SecurityID`, whose validation states the source in turn; the country is read after either; a security type read off a CFI places the product; a status read off an execution type decides what is left. The primary identifier is read before the alternate ones, as the column is defined, so a message stating an ISIN in both places states it in `SecurityID`.

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

### A stream remembers the configurations it passed

A bridge states a plugin's two ends once, in the
[configuration](#a-bridge-configuration-is-a-dictionary-of-its-own) it printed
at startup, and then writes ten million lines that name only the plugin. Those
lines are about a session the reader has already read both ends of, so leaving
them null makes every consumer join back to a document it would have to have
kept. `enrich_messages` therefore remembers: every `pluginconfig` it passes is
held under the plugin's `Name` (20013), and every later message whose
[`pluginid`](#the-crates-own-columns) folds equal to a remembered name takes
that configuration's `SenderCompID` (49) and `TargetCompID` (56).

Not `BeginString` (8). Every built message fills it non-null from the version
its row was read at, so a configuration's begin string would never find a
message stating none, and a rule that can never fire is not written down. What
a plugin's configuration says about its version, the row already says about
itself.

The rule the whole pass keeps holds here too: a stated value is never
overwritten, so a line that spelled its own comp ids keeps them and the fill
can only ever add what the capture already implied. A configuration passes
untouched - it is the statement rather than a thing to fill - and one seen
later replaces the one remembered for its name, because a bridge that
reconfigures a session has said the later word. A message naming a plugin no
configuration named is left alone, and so is one naming no plugin at all. The
memory is the iterator's and dies with it: `enrich_message` takes one message,
which is not a stream and has nothing to remember from, and over batches
[`enrich_messages_arrow_reader`](arrow.md#filled-where-it-sits) is one memory
the length of the reader.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?.with_plugin_fields()?;
    let reader = FixCodec::new(Arc::new(registry));

    // What the bridge printed at startup, and a line naming only the plugin.
    let configuration = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"ULMSG_BROKER","SenderCompID":"ULB_BKRBDG","TargetCompID":"ULB_PTBDG"},"status":200}"#;
    let row = b"MSGTYPE=8|ACCOUNT=ACCT-000117|PLUGINID=ULMSG_BROKER|";

    let held: Vec<FixMsg> = reader
        .enrich_messages([
            reader.parse_line(configuration)?.next().expect("the configuration")?,
            reader.parse_line(row)?.next().expect("the row")?,
        ])
        .collect::<yggdryl::Result<_>>()?;
    // The configuration passed untouched; the line behind it took both ends.
    assert_eq!(held[0].as_field().name(), "pluginconfig");
    assert_eq!(held[1].by_tag(49)?.as_str(), Some("ULB_BKRBDG"));
    assert_eq!(held[1].by_tag(56)?.as_str(), Some("ULB_PTBDG"));

    // A line that stated its own sender keeps it; the one it left unsaid fills.
    let stated = b"MSGTYPE=8|PLUGINID=ULMSG_BROKER|SENDERCOMPID=ITS.OWN|";
    let kept = reader
        .enrich_messages([
            reader.parse_line(configuration)?.next().expect("the configuration")?,
            reader.parse_line(stated)?.next().expect("the row")?,
        ])
        .last()
        .expect("the row")?;
    assert_eq!(kept.by_tag(49)?.as_str(), Some("ITS.OWN"));
    assert_eq!(kept.by_tag(56)?.as_str(), Some("ULB_PTBDG"));

    // One message is not a stream, so this door remembers nothing.
    let alone = reader.enrich_message(reader.parse_line(row)?.next().expect("the row")?)?;
    assert_eq!(alone.get_by_tag(49), None);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    registry.with_plugin_fields()
    reader = FixCodec(registry)

    # What the bridge printed at startup, and a line naming only the plugin.
    configuration = b'{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"ULMSG_BROKER","SenderCompID":"ULB_BKRBDG","TargetCompID":"ULB_PTBDG"},"status":200}'
    row = b"MSGTYPE=8|ACCOUNT=ACCT-000117|PLUGINID=ULMSG_BROKER|"

    held = list(reader.enrich_messages([
        next(reader.parse_line(configuration)),
        next(reader.parse_line(row)),
    ]))
    # The configuration passed untouched; the line behind it took both ends.
    assert held[0].field.name == "pluginconfig"
    assert held[1].by_tag(49).as_py() == "ULB_BKRBDG"
    assert held[1].by_tag(56).as_py() == "ULB_PTBDG"

    # A line that stated its own sender keeps it; the one it left unsaid fills.
    stated = b"MSGTYPE=8|PLUGINID=ULMSG_BROKER|SENDERCOMPID=ITS.OWN|"
    kept = list(reader.enrich_messages([
        next(reader.parse_line(configuration)),
        next(reader.parse_line(stated)),
    ]))[-1]
    assert kept.by_tag(49).as_py() == "ITS.OWN"
    assert kept.by_tag(56).as_py() == "ULB_PTBDG"

    # One message is not a stream, so this door remembers nothing.
    assert reader.enrich_message(next(reader.parse_line(row))).get_by_tag(49) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    registry.withPluginFields()
    const reader = new fix.FixCodec(registry)

    // What the bridge printed at startup, and a line naming only the plugin.
    const configuration = Buffer.from('{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"ULMSG_BROKER","SenderCompID":"ULB_BKRBDG","TargetCompID":"ULB_PTBDG"},"status":200}')
    const row = Buffer.from('MSGTYPE=8|ACCOUNT=ACCT-000117|PLUGINID=ULMSG_BROKER|')

    const held = [...reader.enrichMessages([
      reader.parseLine(configuration).next().value,
      reader.parseLine(row).next().value,
    ])]
    // The configuration passed untouched; the line behind it took both ends.
    assert.equal(held[0].field.name, 'pluginconfig')
    assert.equal(held[1].byTag(49).asJs(), 'ULB_BKRBDG')
    assert.equal(held[1].byTag(56).asJs(), 'ULB_PTBDG')

    // A line that stated its own sender keeps it; the one it left unsaid fills.
    const stated = Buffer.from('MSGTYPE=8|PLUGINID=ULMSG_BROKER|SENDERCOMPID=ITS.OWN|')
    const kept = [...reader.enrichMessages([
      reader.parseLine(configuration).next().value,
      reader.parseLine(stated).next().value,
    ])].pop()
    assert.equal(kept.byTag(49).asJs(), 'ITS.OWN')
    assert.equal(kept.byTag(56).asJs(), 'ULB_PTBDG')

    // One message is not a stream, so this door remembers nothing.
    assert.equal(reader.enrichMessage(reader.parseLine(row).next().value).getByTag(49), null)
    ```

### Edges

- A value the column refuses is silence, not a failure: `SecurityID` under source `4` spelling `XX0000000001`, whose check digit does not close it, leaves `isincode` null, and nothing downstream reads a country off it.
- A value that would not type - `201=abc` in the `PutOrCall` column - is a null the row holds while the entry keeps the text; a rule fills the null in place, so the row has one column for the tag and the entry still says `abc`.
- The rules read codes, and a venue's own word for one is not the code. A bridge row spelling `SECURITYIDSOURCE=isin` beside a `SECURITYID` the check digit closes has stated a source, which stands, and `isin` is not `4` - the dictionary names that code `ISINNumber`, so nothing translated it - so the ISIN column is left null; the same row spelling `SECURITYTYPE=equity` names no code of the `SecurityType` set, so no group files it and no CFI is read off it. The bridge's own `ISINCODE` states the column directly, and the `CFICODE` it spells beside it states the product.
- A trade stating no quantities states no status: `150=F` alone leaves `OrdStatus` absent, and `state` then holds what the report said happened, `F` as the column spells it, `40TRADE`.
- An option stating no `PutOrCall` gets a CFI whose exercise is `X`, and `PutOrCall` is not then read back off it: `X` is the code for an exercise left open.

## A bridge configuration is a dictionary of its own

`Plugin` is one configuration: the ObjectName the read named it by and the
attributes it stated, which is all of it. What the Jolokia exchange wrapped
them in - what was asked, and how the asking went - is the transport's and no
part of the configuration, so `MBean`, `Operation`, `Status` and `Error` are
gone with it. `Plugin::from_json_bytes` and `from_json_scalar` answer
`Plugins` and refuse nothing: a body that is not a Jolokia answer names no
configuration, and answering none is what it answers - `{"a":1}` is a row's
own bytes and the row said nothing FIX can read, and bytes that are not JSON
at all are the same silence. Bulk responses retain array order; wildcard
ObjectNames use canonical ObjectName order within each response. The cursor
retains the source document and its current position, with no collected
output messages.

Each selected configuration converts to one flat `FixMsg`. `SenderCompID`,
`TargetCompID` and `BeginString` retain standard FIX tags; the bridge's own
fields type the configuration-specific attributes. `PLUGIN_TAG_MIN` (20001)
is the floor of the range this dictionary claims, not the smallest tag it
defines: 20001 to 20004 held the envelope and are retired rather than reused -
a capture written last year holds `MBean` on 20001, and a dictionary that gave
20001 to something else would read that column as the new field rather than as
the old one - so the smallest tag defined is `SessionInterface` (20010).
`with_plugin_fields` / `withPluginFields` registers them in the
one namespace beside the standard ones, each carrying `fix:branches = plugin`
- the membership `PLUGIN_DIALECT` names, which `dialects()` lists and no
lookup consults - so a document's attributes reach them by name and the codec
needs no pin. Two attributes are held under another name than the document
spells: every registry already holds the crate's `state` (the order's) and
`version` (the FIX version a row was read at), so a plugin's `State` is the
field `PluginState` (20019) and its `Version` is `PluginVersion` (20021). The
row holds them under the dictionary's names; the arrival entry keeps the
document's spelling, exactly as a line keeps what it wrote.

A configuration is a message type of the crate's own: `pluginconfig`, whose
wire code is `UCFG`. FIX reserves every code opening with `U` for the messages
it does not define, so this one collides with no dictionary's own and no
dictionary has to be edited for a configuration to read as what it is.
`PLUGINCONFIG_CODE_NAME` is the pair, and `fix_plugin_message` the component
it names: FIX's `MsgType` beside the plugin attributes and the three fields
FIX publishes, which is the whole of what a configuration states.
`FixRegistry::new` registers it, so every registry answers `UCFG` exactly as
every registry holds the crate's own fields - a codec meeting a configuration
cannot write a registry it shares, so the type is there before the first
document arrives. A component holds its members by value, so a registry that
never called `with_plugin_fields` still types a configuration's attributes
from the message while still answering no `plugin` dialect and no
`CurrentPort` by name: those are two questions and they keep two answers.

The code is a built child rather than a pair. The exchange sent no `35=`, the
arrival record is what the document stated, and the wire therefore re-emits
byte for byte without one. What the message reads as is `pluginconfig`, the
name the crate registered the code under, where a message typed off the wire
keeps whatever the wire spelled - `D`, `8`, `ExecutionReport`. The two are
different facts: a spelling a document wrote is the document's word and is
kept verbatim, and a code the crate supplies is the crate's, which knows the
name it registered it under.

| Source | Flat message |
| --- | --- |
| the crate | `MsgType` (35) holding `UCFG`, built rather than arrived |
| selected ObjectName | `SessionInterface` (20010), `MBeanType`, `PluginType` |
| selected attribute map | typed fields such as `Name`, `CurrentPort`, `NeedReload`; `State` and `Version` land in `PluginState` and `PluginVersion` |

The ObjectName is the one part of the exchange that names the configuration
itself, and it was always the `SessionInterface` attribute, so the envelope was
restating it.

### One configuration, out of a bulk body and back

The first response below names two configurations, the first of them stating
its `State` and `Version`. The second member is a request with no value: it
names no configuration and therefore states no message. Parsing produces two
messages, one per configuration a response named, each reading as
`pluginconfig` and each re-emitting its own bytes with no `35=` among them.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::{FixCodec, FixRegistry, Scalar, PLUGIN_DIALECT, PLUGIN_TAG_MIN, Plugin};

    let body = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061,"State":"logged","Version":"4.7.0"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]"#;
    let registry = FixRegistry::new().with_plugin_fields()?;
    // The bridge's fields sit in the one namespace, each a member of `plugin`.
    assert_eq!(registry.dialects(), [PLUGIN_DIALECT]);
    // 20001 is the floor of the range, not a tag: the four the envelope held
    // are retired, and the smallest one defined is `SessionInterface`.
    assert!(registry.get_field_by_tag(PLUGIN_TAG_MIN).is_none());
    assert!(registry.field_by_tag(20_010)?.as_fix().has_branch(PLUGIN_DIALECT));
    let codec = FixCodec::new(Arc::new(registry));
    let mut configurations = Plugin::from_json_bytes(body);
    let first = configurations.next().expect("first configuration");
    assert_eq!(first.name(), Some("A"));
    assert_eq!(first.state(), Some("logged"));
    assert_eq!(configurations.count(), 1, "the request-only member named no configuration");

    let message = first.into_fixmsg(&codec)?;
    assert_eq!(message.by_name("CurrentPort")?, &Scalar::from(7061_i64));
    // The row holds the plugin's State and Version under the dictionary's names;
    // the entry keeps the document's spelling.
    assert_eq!(message.by_name("PluginState")?, &Scalar::from("logged"));
    assert_eq!(message.by_tag(20_021)?, &Scalar::from("4.7.0"));
    assert!(message.get_by_name("state").is_none(), "the order's state is another field");
    let entry = message.entries().iter().find(|entry| entry.tag() == 20_019).expect("the State entry");
    assert_eq!(entry.key().as_bytes(), b"State");
    assert_eq!(Plugin::from_fixmsg(&message)?.name(), Some("A"));
    assert_eq!(Plugin::from_fixmsg(&message)?.state(), Some("logged"));
    // The type is the crate's: every registry answers `UCFG`, and the code is
    // a built child, so the wire the document sent still holds no `35=`.
    assert_eq!(message.as_field().name(), "pluginconfig");
    assert_eq!(message.by_tag(35)?.as_str(), Some("UCFG"));
    assert_eq!(FixRegistry::new().msgtype("UCFG")?.name(), "pluginconfig");
    assert!(!message.into_text('|')?.contains("35="));
    let mut count = 0;
    for message in codec.parse_line(body)? {
        message?;
        count += 1;
    }
    assert_eq!(count, 2, "the request-only member named no configuration");
    // A body that is not a Jolokia answer names none, and refuses nothing.
    assert_eq!(Plugin::from_json_bytes(br#"{"a":1}"#).count(), 0);
    ```

=== "Python"

    ```python
    from yggdryl.fix import PLUGIN_DIALECT, FixCodec, FixRegistry, Plugin

    body = b'[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061,"State":"logged","Version":"4.7.0"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]'
    registry = FixRegistry()
    registry.with_plugin_fields()
    # The bridge's fields sit in the one namespace, each a member of `plugin`.
    assert registry.dialects() == [PLUGIN_DIALECT]
    # 20001 is the floor of the range, not a tag: the four the envelope held
    # are retired, and the smallest one defined is `SessionInterface`.
    assert registry.get_field_by_tag(20_001) is None
    assert registry.field_by_tag(20_010).fix.branches == [PLUGIN_DIALECT]
    codec = FixCodec(registry)
    configurations = Plugin.from_json_bytes(body)
    first = next(configurations)
    assert first.name == "A"
    assert first.state == "logged"
    assert sum(1 for _ in configurations) == 1, "the request-only member named no configuration"

    message = first.into_fixmsg(codec)
    assert message.by_name("CurrentPort").as_py() == 7061
    # The row holds the plugin's State and Version under the dictionary's names;
    # the entry keeps the document's spelling.
    assert message.by_name("PluginState").as_py() == "logged"
    assert message.by_tag(20_021).as_py() == "4.7.0"
    assert message.get_by_name("state") is None, "the order's state is another field"
    assert (20_019, "State", "logged") in message.entries()
    assert Plugin.from_fixmsg(message).name == "A"
    assert Plugin.from_fixmsg(message).state == "logged"
    # The type is the crate's: every registry answers `UCFG`, and the code is
    # a built child, so the wire the document sent still holds no `35=`.
    assert message.field.name == "pluginconfig"
    assert message.by_tag(35).as_py() == "UCFG"
    assert FixRegistry().msgtype("UCFG").name == "pluginconfig"
    assert b"35=" not in message.into_bytes(ord("|"))
    assert sum(1 for _ in codec.parse_line(body)) == 2, "the request-only member named no configuration"
    # A body that is not a Jolokia answer names none, and refuses nothing.
    assert list(Plugin.from_json_bytes(b'{"a":1}')) == []
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fix } = require('yggdryl')

    const body = Buffer.from('[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061,"State":"logged","Version":"4.7.0"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]')
    const registry = new fix.FixRegistry()
    registry.withPluginFields()
    // The bridge's fields sit in the one namespace, each a member of `plugin`.
    assert.deepEqual(registry.dialects(), ['plugin'])
    // 20001 is the floor of the range, not a tag: the four the envelope held
    // are retired, and the smallest one defined is `SessionInterface`.
    assert.equal(registry.getFieldByTag(20_001), null)
    assert.deepEqual(registry.fieldByTag(20_010).fix.branches, ['plugin'])
    const codec = new fix.FixCodec(registry)
    const configurations = fix.Plugin.fromJsonBytes(body)[Symbol.iterator]()
    const first = configurations.next().value
    assert.equal(first.name, 'A')
    assert.equal(first.state, 'logged')
    assert.equal([...configurations].length, 1, 'the request-only member named no configuration')

    const message = first.intoFixmsg(codec)
    assert.equal(message.byName('CurrentPort').asJs(), 7061)
    // The row holds the plugin's State and Version under the dictionary's names;
    // the entry keeps the document's spelling.
    assert.equal(message.byName('PluginState').asJs(), 'logged')
    assert.equal(message.byTag(20_021).asJs(), '4.7.0')
    assert.equal(message.getByName('state'), null, "the order's state is another field")
    assert.ok(message.arrivals().some(([tag, key]) => tag === 20_019 && key === 'State'))
    assert.equal(fix.Plugin.fromFixmsg(message).name, 'A')
    assert.equal(fix.Plugin.fromFixmsg(message).state, 'logged')
    // The type is the crate's: every registry answers `UCFG`, and the code is
    // a built child, so the wire the document sent still holds no `35=`.
    assert.equal(message.field.name, 'pluginconfig')
    assert.equal(message.byTag(35).asJs(), 'UCFG')
    assert.equal(new fix.FixRegistry().msgtype('UCFG').name, 'pluginconfig')
    assert.ok(!Buffer.from(message.intoBytes(124)).toString().includes('35='))
    assert.equal([...codec.parseLine(body)].length, 2, 'the request-only member named no configuration')
    // A body that is not a Jolokia answer names none, and refuses nothing.
    assert.equal([...fix.Plugin.fromJsonBytes(Buffer.from('{"a":1}'))].length, 0)
    ```

### Edges

- An empty bulk array or empty wildcard response yields zero configurations.
- A request-only document, an error-only response and any other body that is
  not a Jolokia answer name no configuration and yield no message: a read that
  answers no plugin answers none, and the row is silent rather than carrying an
  envelope with nothing inside it.
- Null attributes are absent. Numeric sentinels such as `-1` remain numbers.
- The original response may contain sibling configurations; equality and
  hashing of a selected `Plugin` are its ObjectName and its attributes and
  nothing else, so sibling attributes do not reach it and neither does the
  exchange - the same plugin answered under `"status":503` is the same value.
- A bulk member that is not a Jolokia answer names no configuration and the
  walk continues past it, reporting nothing: being unable to read a body is not
  an error in the codec. Conversion errors stop and fuse `FixMessages`; the
  iterator does not skip a failed message and continue.
- [Arrow parsing](arrow.md#one-row-per-message) repeats each source row's carried
  columns for all configurations produced from its body.

## A column is filled by the tag its field carries

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [lookup](registry.md) would be a hash and a verification. None of that runs per row: the schema is fixed, its columns are named `msgtype` and `symbol`, each carries its field's `fix:tag`, and `into_row` fills each one by that tag. `fix_column_tags` reads the tags off a schema once, so a batch of a million rows reads them once rather than once per row; a caller-declared root that spells a column by its tag's digits is read the same way, the digits answering where the field carries no tag.

So there is nothing beside the schema to build, hold, or invalidate. A caller finds a column with `index_of` on the schema it already has - or with `fix_column_of` and the tag - and two captures sharing a dictionary share both the schema and every position in it. A List group column is filled by its `fix:counter`, while the numeric count stays in its own column; the self-counting `altids` Map occupies only its own column.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `into_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

Map groups use the same row and Arrow doors, preserving key/value fields, non-null keys and `keys_sorted`; a null map, an empty map and a map with a null value remain distinct.

## A capture's own columns lead the row

A line's URL, line number, timestamp and other capture fields lead its FIX columns. A source row produces one output row per message - one per frame a line carried, one per configuration a bulk body selected - and each of them receives the same carried values from that row; a row that carried no message produces none.

A carried column whose folded name a FIX column already takes - a `msgCtxId` capture beside `msgctxid`, a text reader's `msgtype` beside the FIX one - is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema. What it stated is not lost, because the row [fills that column from it](arrow.md#a-column-is-the-caller-speaking-per-row). A `msgdirection` column is the row's stated direction, read as a parameter and carried nowhere else.

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
- `timepartition` is floored from `updatedat`'s nanoseconds, so a clock stated to the microsecond has a partition rather than a null for not being a whole second.
- A column of the crate's own is typed by the crate's definition, on a tag from 65001 that no dictionary publishes: `updatedat` is an instant, `uuid` a UUID, `miccode` a `mic`, `state` a `state`, whatever text a venue spelled them in.
- A capture's own `timestamp` is context, not a FIX clock: it leads the row as a carried column and never overrides the message's `TransactTime`, then `SendingTime`, reading.
- `altids` is filled by enrichment, never derived by `into_row` alone. A known message with no stated identifiers gets an empty map; an unknown message remains null in the fixed row. Invalid UTF-8 in a selected binary identifier raises the core's located conversion error.
- Typed text drops the replacement character and every control character but tab, so a byte a transport mangled does not become a mangled column; the entry keeps the bytes exactly as they arrived.
- `index_of` on a column the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- Nothing drops a row unasked: `FixDedup`, Rust only, drops adjacent republications over any message stream, and is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) the caller composes.

## Performance

`fix/plugin`, a bulk configuration document read as the dictionary of its own it is: one Jolokia wildcard answer holding 1, 32 and 256 configurations, walked into them and each read into a message. Release build, one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.94.1 release (thin LTO, one codegen unit). The run is from before a message stopped carrying the Jolokia envelope (decision 17) - a configuration is built from four fewer pairs than it was, and its stable hash is taken over two parts rather than three - so the `messages` and `stable_hash` rows are that reading's and the table is due the regeneration below.

| case | estimate |
| --- | --- |
| `fix/plugin/first/256`, the first configuration out of the parsed document | 260 ns |
| `fix/plugin/drain/256`, every configuration out of it | 76.6 us |
| `fix/plugin/messages/256`, every configuration read into a message | 2.56 ms |
| `fix/plugin/messages/32` | 318 us |
| `fix/plugin/messages/1` | 9.84 us |
| `fix/plugin/stable_hash_one_state_allocation/256`, one configuration's stable hash | 284 ns |

The first-item and full-drain cases distinguish cursor cost from conversion of every selected configuration: a configuration costs its walk, and a message a build over its attributes, so the messages row is the drain row plus one build per configuration. Allocation tests assert zero allocations for borrowed configuration iteration; each stable hash owns one Xxh3 state buffer. Parsing bytes and building message values allocate.

What one line of each shape costs the codec - a framed tag stream, a bare one, a bridge row keyed by name, a packed occurrence, a document - is measured where the shapes are read together, in [`fix/pipeline`](arrow.md#performance): the `parse_lines` row is the codec over every body of a real capture, and a bridge row of a hundred named keys costs it a hundred dictionary lookups where a frame of twenty tags costs twenty.

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/plugin
```

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
