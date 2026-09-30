# Capture

A day of session log is a table. This page is the road from one to the other: [`FixCodec`](#a-reader-is-the-whole-parse-surface) turns a captured line into the [messages](message.md) it holds, [`fix_schema`](#the-columns-are-the-folded-names) is the one row shape every message answers as, and [`FixMsg::into_row`](#a-column-is-filled-by-the-tag-its-field-carries) fills it - by the tag each column's field carries, so nothing is resolved against the [dictionary](registry.md) per row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec` and its `parse_*` readers, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `fix_column_of`, `fix_column_tags`, `FixMsg::into_row`, `fix_crate_fields` |
| Columns | named by the field's folded canonical name - `msgtype`, never `35` and never `msg_type`; the display spelling stays on the field's `display`, the tag on its `FIX:tag`, and a named group column's counter on its `FIX:counter` |
| Shape | the crate's own clocks, identities, category, normalized instrument codes and strike with the `metadata` Map group; the standard header, with crate `msgcat` immediately after `msgtype`; the fields a consumer reads, four Serie groups, the trailer, FIX's own `msgdirection`, then the `fixentries` map: 128 tags from `fix_schema_tags`, 133 columns with the shipped registry, each Serie group adding its column beside its counter |
| Identifiers | the names a message goes by are no column of the row: its [alternate identifiers](message.md#the-identifier-maps) are read off the FIX fields that state them, and its security identifiers off `SecurityID(48)` under its source, the `secaltids` group and the crated `isincode`, `forexcode`, `bloombergcode` and `figicode` views |
| Non-null | `beginstring`, `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid` - the instants the identity is settled against and the identity it settles to; every other column is nullable, `state` among them - stated on every row a message writes, `UNKNOWN` where nothing states one, while an empty cell entering the column is null as it is for any [enum](../types/enum/state.md#the-code-is-the-rank) - `sendingtime` among them, because the row states tag 52 only where the message did: a clock intake stood in with is not a fact of the message, and the instant it was settled into has a column of its own |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Residual | projected typed columns hold the facts their schema owns; `fixentries`, a sorted map keyed `tag:name`, holds only arrival content no projected column can represent, and `metadata` every key no dictionary resolved. Rebuilding combines them into the semantic message |
| Expansion | a line yields one message per [frame it carries](decode.md#a-line-yields-none-one-or-many-messages) and none where it carries none, each followed by the messages it [reports](message.md#a-parse-splits-what-a-message-reports) - a fill, a traded side, a quoted side; a [JSON document](#a-json-document-is-one-message-stating-nothing) yields exactly one, named `unknown`, whatever the document names |
| Refuses | no row's content: a value that will not type is null, a clock - `SendingTime(52)`, `TransactTime(60)` - naming no instant is left unstated, a payload that was there and would not parse is a message with nothing in it, and a frame that builds no message is left out, so no row ever fails the batch it arrives in; only a source's own failure is an error item, after every message read before it. The row count is the capture's messages rather than its lines |
| Warnings | what a parse defaults or leaves out it [says once](#warnings): a deduplicated `WARN` record on the `log` facade, `logging` in Python, standard error in JavaScript and the CLI |
| Found | a column is `index_of("msgtype")` on the schema itself, and `fix_column_of(&schema, 35)` is the same position read off the column's own `FIX:tag`; nothing is cached, resolved or invalidated |

## Use

One line in, one row per message out, with the columns named as the dictionary names the fields.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);

    let schema = fix_schema(&registry, "FixMessage")?;
    let reader = FixCodec::new(Arc::clone(&registry));
    let mut messages = reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|18=G|9999=x|10=0|")?;
    let order = messages.next().expect("one frame")?;
    assert!(messages.next().is_none());

    let row = order.into_row(&schema)?;
    let held = row.as_sequence().expect("a row");
    let at = schema.index_of("msgtype").expect("the msgtype column");
    assert_eq!(held[at].as_str(), Some("D"));
    assert_eq!(held[schema.index_of("side").expect("the side column")].as_str(), Some("BUYS"));

    // Typed facts belong in their columns. The residual map holds only what
    // a projected column cannot represent, keyed `tag:name`; a key no
    // dictionary resolves is no field, and the metadata holds it instead.
    let residual = held.last().and_then(Scalar::as_mapping).expect("the residual map");
    let keys: Vec<&str> = residual.iter().filter_map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["18:execinst"]);
    let metadata = held[schema.index_of("metadata").expect("the metadata column")]
        .as_mapping()
        .expect("the metadata map");
    let keys: Vec<&str> = metadata.iter().filter_map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["9999"]);
    // In memory the message holds every pair it read as an entry, the
    // unknown key under tag 0 and its own spelling.
    let names: Vec<&str> = order.entries().iter().map(yggdryl::FixEntry::name).collect();
    assert_eq!(names, ["symbol", "side", "execinst", "9999", "timeinforce"]);
    let unresolved: Vec<_> = order.entries().iter().filter(|entry| entry.tag() == 0).collect();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].name(), "9999");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import Side
    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    schema = fix_schema(registry, "FixMessage")
    reader = FixCodec(registry)

    order, = reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|18=G|9999=x|10=0|")
    row = order.into_row(schema).as_py()

    # A column is read by name, never by its place in the row.
    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("side")] == Side.BUYS

    # Typed facts belong in their columns; the residual map holds only what no
    # projected column represents, and a key no dictionary resolves is in the
    # metadata.
    assert row[schema.index_of("fixentries")] == {"18:execinst": "G"}
    assert row[schema.index_of("metadata")] == {"9999": "x"}
    assert [name for _, name, _, _ in order.entries()] == ["symbol", "side", "execinst", "9999", "timeinforce"]
    assert [entry for entry in order.entries() if entry[0] == 0] == [(0, "9999", "x", [])]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')
    const reader = new fix.FixCodec(registry)

    const [order] = reader.parseLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|18=G|9999=x|10=0|'))
    const row = order.intoRow(schema).asJs()

    // A column is read by name, never by its place in the row.
    assert.equal(row[schema.indexOf('msgtype')], 'D')
    assert.equal(row[schema.indexOf('side')], 'BUYS')

    // Typed facts belong in their columns; the residual map holds only what no
    // projected column represents, and a key no dictionary resolves is in the
    // metadata.
    assert.deepEqual(row[schema.indexOf('fixentries')], new Map([['18:execinst', 'G']]))
    assert.deepEqual(row[schema.indexOf('metadata')], new Map([['9999', 'x']]))
    assert.deepEqual(order.entries().map((entry) => entry.name), [
      'symbol', 'side', 'execinst', '9999', 'timeinforce',
    ])
    const unresolved = order.entries().filter((entry) => entry.tag === 0)
    assert.deepEqual(unresolved.map((entry) => [entry.name, entry.value]), [['9999', 'x']])
    ```

## Try it

The [Decode](decode.md) page displays native results for the committed frame corpus. Run the examples here to parse other input.

## Find a column

More columns than anyone scrolls, and the question a reader actually has is *which column holds this*. The filter matches the tag, the field name and the wording alike.

<div class="ygg-fx" data-fix="row" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## A reader is the whole parse surface

The verb is `parse`, and no reader takes a flag. A parse builds the whole message - the [typed facts](message.md#typed-tags) lifted out of the pairs that state them, a nested `XmlData` row exploded into it, the row [restated under the dictionary](message.md#restated-under-the-dictionary), [what it implies filled in](#what-a-message-implied-is-filled-in), the security identifiers and the alternate identifiers, the identity settled, and on every stream door the messages it [reports](message.md#a-parse-splits-what-a-message-reports) split off beside it - and what happens to a message once it is built, [chaining](lifecycle.md) it to the others of its order, is a call over the stream, never an argument to the parse.

| Reader | Takes | Answers |
| --- | --- | --- |
| `parse_line` | one captured line, the verb and prose around the frame included | `FixMessages`, a lazy fallible iterator: [none, one or many](decode.md#a-line-yields-none-one-or-many-messages) - one per frame, each followed by the messages it reports, one for a JSON document, none for a line that states no message |
| `parse_lines` | any iterator of lines | a lazy iterator of `Result<FixMsg>`; a line that is not a row, or a frame that builds no message, is left out with a [warning](#warnings) and the stream reads on; only a source's own failure is an `Err` item |
| `parse_text_line` | one [decoded line](../media/index.md#plain-text), its body, its [row-header captures](arrow.md#a-column-is-the-caller-speaking-per-row) and its clock, the [sending clock](#every-message-is-dated) of a message stating none | `FixMessages` |
| `parse_text_lines` | any iterator of owned or borrowed lines, or `Result`s of them | a lazy iterator of `Result<FixMsg>`; an owned line is never cloned, a borrowed one only to cross to a worker thread, and a source error is moved into the stream unchanged |
| `parse_text_arrow_reader` | a `BatchReader` of text records | a `BatchReader` of [fixed rows](arrow.md) |
| `parse_fix_line`, `parse_fixml_line`, `parse_ullink_line`, `parse_pairs` | one body of that dialect, or pairs already split | one `FixMsg`, as stated: nothing is split off it; a body holding [a second frame](decode.md#a-line-yields-none-one-or-many-messages) is refused |

A stream adapter owns a clone of the codec and borrows nothing, so `codec.arrow_reader(schema, codec.parse_lines(lines))` composes without the codec outliving the stream. Python exposes native iterators; JavaScript uses `IterableIterator<FixMsg>`. Every stream door fuses its source's exhaustion. Schema construction and group-plan resolution happen before repeated values are processed.

Every one of them ends in the same builder, so a document is typed by the rules that type a frame - one nesting builder, one fold, one code translation, one value contract. Each is the core's own method under the same name in all three languages.

### Lines are a stream

`parse_lines` is the line iterator everything else is built on: nothing is collected, and a line answers [every message it carries](decode.md#a-line-yields-none-one-or-many-messages) - two where a relay wrote two frames on one line, none where the line is a sentence, one stating nothing where it carries a JSON document. A line the reader cannot read - an empty one - or a frame that builds no message is left out with a [warning](#warnings) and the stream reads on: one corrupt line must not end a run over ten million, and only the source's own failure is an `Err` item. The example opens with a bridge frame: `#`-prefixed name keys, and one group occurrence whose value packs its members behind the two control bytes ULLINK uses.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    // A bridge row states its fields and not its type, so this reader is told
    // to read the untyped row the [default refusals](decode.md#a-type-nobody-asked-for-is-never-built) drop.
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?))
        .with_exclude_msgtypes::<[&str; 0], &str>([]);

    let bridge: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|";
    let held = reader.parse_line(bridge)?.next().expect("one frame")?;
    assert_eq!(held.by_tag(55)?.as_str(), Some("TTF"));
    assert_eq!(held.by_tag(44)?, Scalar::from(yggdryl::Decimal::parse("41.25")?));
    // The packed members became three real fields under one nesting.
    let party = yggdryl::FieldPath::from_str("Parties[0].PartyID")?;
    assert_eq!(held.by_path(&party)?.as_str(), Some("BUYSIDE"));
    assert_eq!(held.by_tag(453)?.as_i128(), Some(1));

    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
        b"heartbeat emitted seq=7",
        b"",
        b"8=FIX.4.4|35=8|37=O1|11=A|10=0|8=FIX.4.4|35=8|37=O2|11=B|10=0|",
    ];
    let read: Vec<_> = reader.parse_lines(lines).collect();
    // An item per message: the sentence stated none, the empty line is no row
    // - left out with a warning, and the stream went on - and the last line
    // stated two.
    assert_eq!(read.len(), 3);
    assert_eq!(read[0].as_ref().expect("an order").by_tag(55)?.as_str(), Some("AAPL"));
    assert_eq!(read[1].as_ref().expect("a report").by_tag(37)?.as_str(), Some("O1"));
    assert_eq!(read[2].as_ref().expect("a report").by_tag(37)?.as_str(), Some("O2"));
    ```

=== "Python"

    ```python
    import decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    # A bridge row states its fields and not its type, so this reader is told
    # to read the untyped row the default refusals drop.
    reader = FixCodec(
        FixRegistry.from_handle(Path("config/fix").resolve()), exclude_msgtypes=[]
    )

    held, = reader.parse_line(
        b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )
    assert held.by_tag(55).as_py() == "TTF"
    assert held.by_tag(44).as_py() == decimal.Decimal("41.25")
    # The packed members became three real fields under one nesting.
    assert held.by_path("Parties[0].PartyID").as_py() == "BUYSIDE"
    assert held.by_tag(453).as_py() == 1

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
    # An empty line is not a row: it is left out with a warning, not raised.
    assert next(reader.parse_lines([b""]), None) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Scalar, fix } = require('yggdryl')

    // A bridge row states its fields and not its type, so this reader is told
    // to read the untyped row the default refusals drop.
    const reader = new fix.FixCodec(
      fix.FixRegistry.fromHandle(path.resolve('config', 'fix')),
      { excludeMsgtypes: [] },
    )

    const bridge = Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    )
    const [held] = reader.parseLine(bridge)
    assert.equal(held.byTag(55).asJs(), 'TTF')
    assert.ok(held.byTag(44).equals(Scalar.decimal(4125n, 2)))
    // The packed members became three real fields under one nesting.
    assert.equal(held.byPath('Parties[0].PartyID').asJs(), 'BUYSIDE')
    assert.equal(held.byTag(453).asJs(), 1)

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
    // An empty line is not a row: it is left out with a warning, not thrown.
    assert.deepEqual([...reader.parseLines([Buffer.alloc(0)])], [])
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

A bridge logs what it exchanged over JMX beside what it exchanged over FIX, so a line may carry a JSON document - an object, or an array of objects, opening before any `=` and balanced to its close, with prose allowed in front of it and behind it - and the [classifier](registry.md#classifying-a-captured-line) names it `application/json` with no message type. The codec reads no document. Such a row is exactly one message named `unknown` with no entries - `entries()` is empty, `into_bytes` re-emits nothing and no tag 35 is built - carrying only what the row stated around the document: its clock, the version it was read at, the direction the prose in front of it spells under the [default rules](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) - `Response:` is `R`, `Request:` is `S` - and what the row itself stated - a `msgpluginid` filling the crate's own column, and the capture's own columns carried beside them. A Jolokia answer, a wildcard or bulk answer, an error-only answer and a bare `{"a":1}` are each one such message, never one per configuration the document names, and nothing the document says reaches a field or a later message.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    // A document states no message type, so this reader reads the untyped row
    // the [default refusals](decode.md#a-type-nobody-asked-for-is-never-built) drop.
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?))
        .with_exclude_msgtypes::<[&str; 0], &str>([]);

    // A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    let line = br#"2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)"#;
    let mut messages = reader.parse_line(line)?;
    let message = messages.next().expect("one message")?;
    assert!(messages.next().is_none());
    // Named `unknown`, with nothing the document said in it.
    assert_eq!(message.as_field().name(), "unknown");
    assert!(message.entries().is_empty());
    // Nothing but the version it was read at: no type, nobody sending it.
    assert_eq!(message.into_text('|')?, "8=FIX.4.4|");
    assert_eq!(message.get_by_tag(35), None);
    assert_eq!(message.get_by_tag(49), None);
    // The prose in front of it is the row's, read as ever.
    assert_eq!(message.by_tag(385)?.as_str(), Some("R"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    # A document states no message type, so this reader reads the untyped row
    # the default refusals drop.
    reader = FixCodec(
        FixRegistry.from_handle(Path("config/fix").resolve()), exclude_msgtypes=[]
    )

    # A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    line = b'2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)'
    message, = reader.parse_line(line)
    # Named `unknown`, with nothing the document said in it.
    assert message.field.name == "unknown"
    assert message.entries() == []
    # Nothing but the version it was read at: no type, nobody sending it.
    assert message.into_text("|") == "8=FIX.4.4|"
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

    // A document states no message type, so this reader reads the untyped row
    // the default refusals drop.
    const reader = new fix.FixCodec(
      fix.FixRegistry.fromHandle(path.resolve('config', 'fix')),
      { excludeMsgtypes: [] },
    )

    // A Jolokia answer as a bridge logs it: prose in front, a duration behind.
    const line = Buffer.from('2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"},"value":{"Name":"Router_OrderRouting","SenderCompID":"CLI.PROD.TRD","TargetCompID":"ST.PROD"},"status":200} (12 ms)')
    const [message, ...rest] = reader.parseLine(line)
    assert.equal(rest.length, 0)
    // Named `unknown`, with nothing the document said in it.
    assert.equal(message.field.name, 'unknown')
    assert.deepEqual(message.entries(), [])
    // Nothing but the version it was read at: no type, nobody sending it.
    assert.equal(message.intoText('|'), '8=FIX.4.4|')
    assert.equal(message.getByTag(35), null)
    assert.equal(message.getByTag(49), null)
    // The prose in front of it is the row's, read as ever.
    assert.equal(message.byTag(385).asJs(), 'R')
    ```

### Edges

- Ordinary unframed text produces no message at all - a line that opens no frame, states no bridge pair and carries no document states nothing to read, so it is no row either. A payload that was there and would not parse is a message with nothing in it, so malformed syntax never fails the batch it arrives in; a clock that names no instant is left unstated and a frame that builds no message is left out, each beside a [warning](#warnings) - only a source's own failure is an error item.
- A bridge key's `#` is judged against the row's bare spellings, in a bridge row and in the name keys a bridge writes into a numeric frame alike. Alone, it drops: `#ORDERID=123` is the dictionary's `OrderID`. Restating a bare pair's bytes, the marked pair is a second spelling of one pair and goes, row and entries alike: `ORDERID=123|#ORDERID=123` is `OrderID` once, and `into_bytes` re-emits the one pair. Beside a bare twin stating other bytes it stays verbatim, because collapsing the two would merge two values under one name: `ORDERID=123|#ORDERID=345` is `OrderID` 123 beside `#ORDERID` 345 - its own column, its own entry - whichever arrived first. The twin is matched by the fold every key resolves under, so `OrderId` and `ORDER_ID` twin it too, and by its stem, so a bare `NOPARTYIDS` group claims every `#NOPARTYIDS[n]` however many the two state - each stays whole under its own name, the count beside them, and none lands in the dictionary's group. A marked group goes only whole: a marked count restating the bare one beside occurrences the bare group never numbered stays with them, and only a marked group restating the bare group pair for pair goes. A bare pair whose value is a stated absence is no twin, because a key that said nothing was sent is not a key that was sent; a value is compared as its bytes, because `abc` is not `ABC`; and the twin is a spelling, never an identity, so a tag and a marked name - `55=AAPL|#SYMBOL=AAPL` - state two values exactly as a tag and a bare name do. In a numeric frame the marks are judged and the keys kept as they are: a packed occurrence there is one value, as a bare one always was. A key marked twice is judged one mark at a time: `##ORDERID` twins `#ORDERID` as `#ORDERID` twins `ORDERID` - restating it goes, beside other bytes it stays, alone it loses one mark.
- A row's message type resolves the way every key does, in the one namespace: a name reaches the message of that name, and a bare code the message tag 35's code set names, else the first in name order. A bridge row calling itself `tradecapturereport` reads against the message of that name, which is what places a counter half the dictionary shares - `NoLegs`, `NoSides` - under the group that message declares.
- A stated absence produces no field and no entry, because a key that said nothing was sent is not a key that was sent. Unless `null_values` is pinned, matching trims ASCII whitespace and folds case over exactly `""`, `null`, `<null>`, `none`, `n/a` and `[n/a]`.

## Warnings

Data is never an error. A parse, a market projection, a [lifecycle](lifecycle.md) walk and a [book](../graph/book.md#book-fold) walk read what a capture states at the smallest part that can stand: a fact that will not read takes its default, an item that cannot stand is left out, and either is said once as a warning. Only a source failure - a reader, a store or a runtime that could not answer, `Error::is_source_failure` - is an `Err` item, yielded after every message read before it. It ends the batch, lifecycle, market and book doors, which fuse; `parse_text_lines` hands a failing item through in its place and reads the lines behind it, the source being the caller's to stop.

| Aspect | Rule |
| --- | --- |
| Defaults | a value that will not type is null beside a [`FixAnomaly`](message.md#anomalies); a clock - `SendingTime(52)`, `TransactTime(60)` - naming no instant is left unstated, and the message is dated [as one stating none is](#every-message-is-dated); a market fact that reads as nothing is null - a size or an FX part no exact decimal reads - or the message's, for an entry clock naming no instant; a trade side no side reads is an execution of side `UNKN` |
| Left out | a line that is not a row, and a frame that builds no message, the frames after it still read; a batch row whose cell the Arrow landing refuses - a `state` code no member takes - the rest of its batch still read; a batch of another schema than the first; a [book entry that cannot stand](message.md#market-data); in a book walk, an order or a quote stating neither the bid nor the ask, an operation dated before its book, and a group the book refuses |
| Kept as stated | an observation whose content merge is refused is walked unmerged, its clocks and provenance folded; an order link or a side the rebuild refuses leaves the message as it stated itself ([lifecycle](lifecycle.md#edges)) |
| Deduplication | keyed by where it is raised, what went wrong and the column, tag or kind it is about - never the value or the row: logged the first time with its detail, then again as `seen N times` at the tenfold counts 10, 100, 1000; process-wide, and bounded at 4096 distinct warnings, past which one line says so and new kinds are counted in total only |
| Level | `WARN` on Rust's `log` facade, its target the module path that raised it - `yggdryl::fix::messages` |
| Surfaces | Rust: any `log` backend, `env_logger` for one. Python: the standard `logging` module, logger `yggdryl.<module path>` - `yggdryl.fix.messages` - at `WARNING`; `yggdryl.refresh_logging()` after a level was changed once records had flowed. JavaScript: standard error as `yggdryl: <message>` when the addon loads, unless the process installed a Rust logger first. CLI: standard error, prefixed `!` |

```text
<what went wrong, and what was done instead> (<subject>): <detail>; later occurrences are counted rather than repeated
<what went wrong, and what was done instead> (<subject>): seen 10 times
```

The clock below names no instant, so it is data: the message is read, dated as one stating none is, and the text is kept as an anomaly. The record is logged once for the process however many lines state the same clock.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // `52=bad` names no instant. The message is read, its clock is unstated,
    // and a warning goes to whatever `log` backend the process installed.
    let message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|52=bad|10=0|")?;
    assert_eq!(message.by_tag(11)?.as_str(), Some("A"));
    assert!(!message.header().stated_sendingtime());
    let [anomaly] = message.anomalies() else { panic!("one anomaly") };
    assert_eq!(anomaly.field(), "sendingtime");
    assert!(anomaly.reason().ends_with(", got \"bad\""), "{}", anomaly.reason());
    ```

=== "Python"

    ```python
    import logging
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    said: list[logging.LogRecord] = []


    class Collect(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            said.append(record)


    # Records travel under `yggdryl.<module path>`, so the package's own logger
    # is the one switch; its effective level is WARNING unless set otherwise.
    logging.getLogger("yggdryl").addHandler(Collect())

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    # `52=bad` names no instant. The message is read, its clock is unstated,
    # and the warning reaches the handler.
    (message,) = codec.parse_line(b"8=FIX.4.4|35=D|11=A|52=bad|10=0|")
    assert message.by_tag(11).as_py() == "A"
    assert [field for field, _ in message.anomalies] == ["sendingtime"]
    assert any(
        record.name.startswith("yggdryl.fix") and "sendingtime" in record.getMessage()
        for record in said
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // `52=bad` names no instant. The message is read and its clock is
    // unstated; the warning goes to standard error as `yggdryl: FIX clock left
    // unstated: ...`.
    const [message] = codec.parseLine(Buffer.from('8=FIX.4.4|35=D|11=A|52=bad|10=0|'))
    assert.equal(message.byTag(11).asJs(), 'A')
    assert.deepEqual(message.anomalies.map((anomaly) => anomaly.field), ['sendingtime'])
    ```

## The columns are the folded names

`msgtype`, never `35` and never `msg_type`. A column is spelled the way the dictionary spells the field's canonical name - ASCII case folded once, on the way in - so a row reads the way a message reads, in every binding and every catalog, and a reader spelling `row["msgseqnum"]` finds the sequence number without a dictionary in hand.

The tag is still the identity. Each column carries its field's `FIX:tag`, its `display` and the name of the [code set](registry.md#a-field-names-the-code-set-it-reads-by) it reads its values by - the members live once in the dictionary, reached through `codeset_of` - and the row is [filled by that tag](#a-column-is-filled-by-the-tag-its-field-carries) rather than by the spelling, so a venue that renames a field between versions changes nothing about where its value lands. `fix_schema_tags` is the same row as tags, in the same order.

The order is nine bands, and each answers one question a reader has, so a row reads left to right the way a person asks about a message rather than in the order a dictionary happens to file its fields:

| Band | Columns |
| --- | --- |
| when it happened | `currunix`, `execunix`, `recdunix`, `creaunix`, `prevunix`, `snapunix`, `exprunix`, then `SendingTime`, `OrigSendingTime`, `TransactTime`, `SettlDate`, `TradeDate`, `ExpireTime`, `ValidUntilTime`, `ExpireDate` |
| which event it is | `curruuid`, `crossuuid`, `crosscode`, `currhashcode`, `crosshashcode`, `prevuuid`, `seqnum`, `srcuuids` |
| which message and session carried it | `BeginString`, `MsgType`, crate `MsgCat`, `MsgSeqNum`, `SenderCompID`, `TargetCompID`, `PossDupFlag`, `MsgDirection`, `msgpluginid`, `msgoriginator`, `msgctxid`, `msgsessionid`, `msgsesseventid`, `conversationid`. `MsgCat` follows `MsgType` immediately; it is the registry's category of that type, not another wire field, `msgsesseventid` is the [session event](#the-crates-own-columns) `MsgType`, `MsgSeqNum`, `msgsessionid` and `msgctxid` join to, and `conversationid` closes the band. Not where the capture read it: that is [the capture's own column](#the-crates-own-columns) and no column of this row |
| which instrument | `Symbol`, `SecurityID`, `SecurityIDSource`, the crate's `isincode`, `forexcode`, `bloombergcode`, `figicode`, `miccode` and `strikepx`, `SecurityType`, `SecuritySubType`, `SecurityExchange`, `ExDestination`, `LastMkt`, `CFICode`, `MaturityDate`, `Product`, then what the market said about trading it |
| which order | `Account`, `ClOrdID`, `OrigClOrdID`, `SecondaryClOrdID`, `OrderID`, `SecondaryOrderID`, `ExecID`, `TradeID`, `QuoteReqID`, `QuoteID`, `MDReqID`, `QuoteRespID` |
| what values it states | `Side`, `Price`, `PrevClosePx`, `LastPx`, `AvgPx`, `OrderQty`, `Quantity`, `LastQty`, `CumQty`, `LeavesQty`, `UnitOfMeasure`, `Currency`, `SettlCurrency`, `QtyType`, `OrdType`, `TimeInForce`, then the quote's `BidPx`, `BidSize`, `OfferPx` and `OfferSize`, then the FX parts `LastSpotRate`, `LastForwardPoints`, `BidSpotRate`, `BidForwardPoints`, `OfferSpotRate` and `OfferForwardPoints` |
| how it went | `state`, then `OrdStatus`, `ExecType`, `QuoteStatus`, `QuoteResponseLevel`, `QuoteEntryRejectReason`, `OrdRejReason`, `CxlRejReason`, `Text` |
| what it carried whole | the four repeating groups, each beside its counter: `Parties`, `secaltids` (display `SecAltIDs`), `TrdRegTimestamps`, `regulatorytradeids` (display `RegulatoryTradeIDs`) - `TrdRegTimestamps` is read rather than merely carried, since its `TrdRegTimestamp` / `TrdRegTimestampType` pair can [date the message](#the-official-clock-dates-the-message) |
| everything else | the standard header, body and trailer fields no band above claims, then `metadata`, then the `fixentries` map |

`cargo run --example fix_schema --features arrow` prints that row as the Arrow batch schema a consumer reads, one column a line: its tag, its name, its Arrow type, and whether it is required.

A Serie group column carries `FIX:counter` beside the `FIX:tag` its definition derives from its own name: the numeric count keeps its own column, and the group column beside it holds the occurrences. The semantic collections are `secaltids` / `SecAltIDs` and `regulatorytradeids` / `RegulatoryTradeIDs`, while `NoSecurityAltID(454)` and `NoRegulatoryTradeIDs(1907)` remain the counters beside them. The crate's `metadata` Map instead carries tag and counter on one group column, 65030, with no scalar count field; a Map's entries already determine its cardinality.

A proprietary group that reuses a standard counter but maps none of that standard group's members remains whole in `fixentries`; its fixed Serie and scalar counter stay null instead of asserting a misleading empty standard group.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixRegistry, fix_column_of, fix_schema, fix_schema_tags};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;
    let schema = fix_schema(&registry, "FixMessage")?;

    let columns: Vec<&str> = schema.fields().iter().map(yggdryl::Field::name).collect();
    // The crate's own lead the row - a table is read by time and joined by
    // identity - and the protocol's own follow them.
    assert_eq!(&columns[..4], ["currunix", "execunix", "recdunix", "creaunix"]);
    // The band that says which message and session carried it, in its order.
    let header = schema.index_of("beginstring").expect("the band opens");
    assert_eq!(&columns[header..header + 4], ["beginstring", "msgtype", "msgcat", "msgseqnum"]);
    assert_eq!(columns.last(), Some(&"fixentries"));
    // A bridge's originating plugin and conversation are columns of the
    // message band; its identifiers and instrument names stay content.
    assert_eq!(fix_schema_tags().len(), 128);
    assert_eq!(columns.len(), 133);
    assert_eq!(&fix_schema_tags()[header..header + 4], [8, 35, 65016, 34]);

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    let held = schema.get_field_by_path("msgtype").expect("the msgtype column");
    assert_eq!(held.display(), Some("MsgType"));
    assert_eq!(held.as_fix().tag()?, Some(35));
    // The tag rides on the column, so a tag still finds it. Rust only.
    assert_eq!(fix_column_of(&schema, 35), schema.index_of("msgtype"));
    // A field no band claims is still a column, further along.
    assert!(schema.index_of("bodylength").is_some_and(|at| at > header));
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
    assert columns[:4] == ["currunix", "execunix", "recdunix", "creaunix"]
    # The band that says which message and session carried it, in its order.
    header = columns.index("beginstring")
    assert columns[header:header + 4] == ["beginstring", "msgtype", "msgcat", "msgseqnum"]
    assert columns[-1] == "fixentries"
    # A bridge's originating plugin and conversation are columns of the
    # message band; its identifiers and instrument names stay content.
    assert len(fix_schema_tags()) == 128
    assert len(columns) == 133
    assert fix_schema_tags()[header:header + 4] == [8, 35, 65016, 34]

    # The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert schema.field("msgtype").display == "MsgType"
    assert schema.field("msgtype").fix.tag == 35
    # A field no band claims is still a column, further along.
    assert columns.index("bodylength") > header
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')

    const columns = [...Array(schema.fieldLen).keys()].map((at) => schema.fieldAt(at).name)
    // The crate's own lead the row - a table is read by time and joined by
    // identity - and the protocol's own follow them.
    assert.deepEqual(columns.slice(0, 4), ['currunix', 'execunix', 'recdunix', 'creaunix'])
    // The band that says which message and session carried it, in its order.
    const header = schema.indexOf('beginstring')
    assert.deepEqual(columns.slice(header, header + 4), ['beginstring', 'msgtype', 'msgcat', 'msgseqnum'])
    assert.equal(columns[columns.length - 1], 'fixentries')
    // A bridge's originating plugin and conversation are columns of the
    // message band; its identifiers and instrument names stay content.
    assert.equal(fix.schemaTags().length, 128)
    assert.equal(columns.length, 133)
    assert.deepEqual(fix.schemaTags().slice(header, header + 4), [8, 35, 65016, 34])

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert.equal(schema.field('msgtype').display, 'MsgType')
    assert.equal(schema.field('msgtype').fix.tag, 35)
    // A field no band claims is still a column, further along.
    assert.ok(schema.indexOf('bodylength') > header)
    ```

## Nothing is lost at the end

Two cells close every row: `metadata`, then `fixentries`. Together with the projected columns they make a row lossless: [`from_row`](message.md#a-row-is-a-message-again) rebuilds content from all three, then writes the canonical row again.

`fixentries` is the residual message content: every field the message states that no projected column represents, as a sorted `map<utf8, utf8>` keyed `tag:name` - the field's tag and its dictionary name, the two its [identity](index.md#identity-is-a-tag-and-a-name) is made of, `18:execinst`. A column that represents a field whole takes it out of the map, so a message whose every field has a column states an empty map. What a key holds is written by the crate's one JSON codec, deterministically:

| The entry | Its value |
| --- | --- |
| a scalar | its canonical wire text, `18:execinst` = `G` |
| a scalar whose text opens the way JSON does - `[`, `{` or `"` | its JSON string, so a value opening that way is JSON and every other value is text |
| a group | the JSON array of its occurrences, each the object of its members |
| a component | the JSON object of its members |
| a member | keyed `tag:name` the same way, leaves as text, nested groups and components recursing |
| a key stated more than once at one level | one key, the JSON array of its values in arrival order |

The typed entries are the message's in memory - [`entries()`](message.md#contract) - and the map is only how a row holds them: `from_row` parses it back into those entries through the dictionary, a declared group's members in the dictionary's order so a re-emitted group still starts on its delimiter. A member no dictionary names inside a group it does name stays inside that group's JSON under `0:name`; at the top level a `0:` key is refused, because a key no dictionary resolved is no field and has no `tag:name`.

Such a key lands in `metadata` instead, beside the namespaced keys a bridge states: under its own spelling, its value the raw text it arrived as, the JSON array of its values where it was stated more than once - `7001=a|7001=b` is `["a","b"]` - the JSON of what it holds where it nests, and `{}` where it was stated with no value. In memory it stays the message's own entry, tag `0` under that spelling, so the wire re-emits it; a row read back restores it there. A venue onboarding a new field finds it as a `metadata` key.

An entry says what the message states and only that. A group holding no occurrence states nothing - `NoPartySubIDs(802)=0` is the group absent - so it is no entry, no `fixentries` key, nothing the wire re-emits and nothing `currhashcode` digests, while its column still holds the empty list and the counter beside it the count. A table storing an absent list as an empty one, as PyIceberg does, therefore reads back the message that was written. A bridge packing a whole occurrence into one value - `#NOPARTYIDS[0]=PARTYID=BUYSIDE...PARTYROLE=1` - is read into the members of the occurrence it names, so the record holds the occurrence under its counter's entry as it holds one a numeric frame spelled member by member. No dialect is there either: a message is not a dictionary member, and which dictionaries a field belongs to is the field's own `FIX:branches` in the registry.

## The crate's own columns

Forty scalar definitions and one Map group carry the facts no dictionary publishes: what the [event](../graph/event.md) a message is states, in `execunix` the [market](../graph/market.md#contract) it stands in, what the capture stated about its line, the message category, five normalized instrument codes whose standard FIX representation is contextual and the option strike, the plugin and conversation a bridge's log line names, and the account, the user, the six identifiers and the two instruments a bridge names in words of its own. Every registry holds them from construction and the [store](store.md) writes them, so a dump is the whole row and a stored copy is read past in favour of the constructed one: `fix_crate_fields` lists all 41 in tag order. Their tags run contiguously from 65001 in the fixed row's band order - `currunix` 65001 through `metadata` 65030, then `sourceurl`, the ten a bridge names and, at 65042, the fixed row's own document `fixmsg` - a retired definition leaving no gap; `CRATE_TAG_MIN` (65000) opens the reserved block and `CRATE_TAG_MAX` (65100) closes it, `CURRUNIX_TAG_NAME` and its siblings hold each `(tag, name)` pair, and `is_crate_tag` tests ownership. `metadata` is the one group, reached by `field_by_counter(65030)` or by name. On a message every definition but `sourceurl` and the ten a bridge names in its own words - 65032 to 65041 - is a [typed fact](message.md#typed-tags): held by the event, the message or the capture, reached by its tag, and never in the content row. Those ten stay content, kept among a fixed row's residual entries rather than as columns: the six identifiers each state their own [`FIX:idmap`](registry.md#a-field-names-a-message-by-its-identifiers) and land in the [alternate identifiers](message.md#the-identifier-maps) under their upper-cased names, the dealer's account and user are content a leaf's [metadata](message.md#what-a-leafs-metadata-holds) carries, and the two instrument names the row holds each as it arrived and each folds into a `secaltids` occurrence under its own source - `OMSINSTRUMENTID`, `ULLINKINSTRUMENTID` - never over an occurrence the row states under that source and only where the source can hold it, a value past its 32 bytes an [anomaly](message.md#anomalies). `sourceurl` is the capture's own column - what a *reader* said about the line rather than what the line said - and neither a message nor the fixed row holds it: it leads the row as [one of the capture's own](#a-captures-own-columns-lead-the-row), beside the body the line was cut from and its place in the object.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `currunix` | `CurrUnix` | 65001 | when the message happened, a nanosecond UTC clock: a stated `currunix`, else the official transaction clock standing within the codec's `official_time_delay_ms` of `SendingTime(52)`, else that `SendingTime`; non-null |
| `execunix` | `ExecUnix` | 65002 | a [market fact](../graph/market.md#contract), not an event's - a text line states none: the latest precise execution instant this lifecycle has reached, nanoseconds UTC: initially a stated crate value, else `ExecutionTimestamp(2749)`, else the first `TrdRegTimestamps(768)` occurrence whose `TrdRegTimestampType(770)` is `ExecutionTime`, else a bridge's `eventtimestamp`, else `TransactTime(60)` only where the message reports an execution - an initial `35=AE` with `TradeReportTransType(487)` absent/New and `ExecType(150)` absent or execution-like among them; non-New/cancel/correct/reverse/status AE and `AD`, `AQ` or `AR` invent none. A report `FixMsg::is_execution` accepts that states no precise clock takes its `currunix` at intake, where it follows nothing yet, and later lifecycle events carry the clock without moving it backward; a lifecycle output read back with none is not refilled |
| `recdunix` | `RecdUnix` | 65003 | the precise recording instant, nanoseconds UTC: a stated crate value, else the enclosing text line's `mtime`; nullable, absent for raw bytes or pairs, and never inferred from FIX sending or hop clocks |
| `creaunix` | `CreaUnix` | 65004 | when the message was created: a stated one, else `currunix`; `CreationTime` is its indexed alias, so a bridge's case-folded `CREATIONTIME` and compact UTC timestamp fill this typed instant directly; the earliest its chain knows once walked; non-null |
| `prevunix` | `PrevUnix` | 65005 | when the message this one follows happened; stamped by the [lifecycle](lifecycle.md), nullable |
| `snapunix` | `SnapUnix` | 65006 | the epoch-aligned tick of a separate owned [grid view](lifecycle.md#snapshots-are-a-grid), which is dated at it - its `currunix` the same tick, its `curruuid` the identity that instant derives; nullable, and unchanged on source rows |
| `exprunix` | `ExprUnix` | 65007 | when the current generation stops being good, a nanosecond UTC clock: `ExpireTime(126)`, else `ValidUntilTime(62)`, `ExpireDate(432)` or `MaturityDate(541)`; a newer message's explicit deadline wins even where earlier, else it inherits the predecessor's; nullable |
| `curruuid` | `CurrUuid` | 65008 | the message's identity, a `uuid`: its millisecond `currunix` leads, `seqnum` occupies UUIDv7's ordered 12-bit lane up to 4095, and `rand_b` carries a 62-bit XXH3 payload over `currhashcode` plus the whole sequence seeded by `crosshashcode`; non-null |
| `crossuuid` | `CrossUuid` | 65009 | the identity every message of one lifecycle shares, a `uuid`: the UUIDv8 of `crosshashcode`, or the message's own `curruuid` where it names no cross code; non-null |
| `crosscode` | `CrossCode` | 65010 | the identifier every message of one lifecycle shares: an explicit nonempty value, else the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`, stored under the side the message takes where its `msgcat` is `ORDR`, `QUOT` or `EXEC` - `BUYS:A1` - and as spelled for `UNKN` and for every other category ([`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches)); empty where none |
| `currhashcode` | `CurrHashCode` | 65011 | the code the message's content digests to, `uint64`: the XXH3-64 of what the message states but the standard header and trailer, less `MsgType(35)` - the event's own facts, the text, the metadata, `MsgType`, the FIX fields it lifted, then the [entry tree](#nothing-is-lost-at-the-end) - never the frame a hop carried it in, never the chain it is in and never the row's columns; non-null |
| `crosshashcode` | `CrossHashCode` | 65012 | the XXH3-64 of `crosscode` as stored, a side prefix included, zero where the message names none; non-null |
| `prevuuid` | `PrevUuid` | 65013 | the identity of the message this one follows; stamped by the lifecycle, nullable |
| `seqnum` | `SeqNum` | 65014 | the message's [place](lifecycle.md#a-place-counts-one-instant) among the messages of its instant, `uint64`: 0 for the first its stream hands over there, one more for each next; null at 0 |
| `srcuuids` | `SrcUuids` | 65015 | the sorted unique identities of the elements this message was read from, a `serie<uuid>`: the [text line](../media/index.md#plain-text) it was parsed out of, stated by the line doors as the line's own `curruuid` before the message settles, and for a message a parse [split off](message.md#a-parse-splits-what-a-message-reports) its source's `curruuid` beside its source's own sources; none for bytes; provenance, never its chain - no walk moves it - and outside the `currhashcode`, so the same bytes read from two lines are one message; never on the wire; nullable |
| `msgcat` | `MsgCat` | 65016 | the message type's business category, the [`MarketDataKind`](../types/enum/marketdatakind.md) member its `int32` code stores - `ORDR` `10`, `QUOT` `14`, `EXEC` `8`, `TRAD` `21`, `BOOK` `3`, and the batches `ORDB` `22`, `QUOB` `23`, `TRDB` `25` - read by the crate's `msgcatcodeset` off the dictionary's `FIX:msgcat` for the type ([filing](index.md#a-message-type-is-filed-under-one-category)), `UNKN` where it files none; an execution report a parse splits a fill off is its order's `ORDR`, or its quote's `QUOT`, and the fill `EXEC`, and a batch's entries a parse splits off each its item's - `ORDR`, `QUOT`, `EXEC` or `TRAD`. Placed immediately after `msgtype`, and the category a [market data leaf](message.md#market-data) states as its `marketdatakind`; a row stating one is the row's word; nullable |
| `msgpluginid` | `MsgPluginId` | 65017 | the plugin that logged the line, as a bridge names it: a row's own `msgpluginid` capture or column fills it, and it selects nothing - no dictionary, no version; a capture fact |
| `msgoriginator` | `MsgOriginator` | 65018 | the plugin a message came into a bridge through, as the bridge's own log line names it: `Message received: ... from (X as ...)`, `Execution report from X type ...`, or the `msgpluginid` that logged a `Receiving :` line; null where the line names none - `PLUGINORIGINATOR=OMSDealer` is content and stays so. Placed after `msgpluginid` in the message band, outside the `currhashcode`, and when two observations of one delivery merge the earlier one's stands, a differing later one an [anomaly](message.md#anomalies); a capture fact |
| `msgctxid` | `MsgCtxId` | 65019 | the message context a bridge handled the message in, from its log's bracket; a capture fact |
| `msgsessionid` | `MsgSessionId` | 65020 | the session *instance* a bridge handled the line on, as its own row header brackets it - never what the message states about itself; a capture fact |
| `msgsesseventid` | `MsgSessEventId` | 65021 | the session event a bridge delivered the message as, `utf8`: `MsgType(35)`, `msgsessionid`, `msgctxid` and `MsgSeqNum(34)` joined by `:` as they are stated - `8:e7256476:9effef3e6a:1094` - where the three texts are nonempty and the sequence present, null otherwise; derived by the message whenever it settles and never read off a row, settled to one message and never carried along a chain, outside the `currhashcode`, and the key the lifecycle's [delivery merge](lifecycle.md#a-twin-is-not-a-successor) matches observations on; a capture fact |
| `conversationid` | `ConversationId` | 65022 | the conversation a bridge filed the message under: a `CONVERSATIONID` the message stated, else the `{conversationId: ...}` of its log line, a line naming another an anomaly; closes the message band after `msgsesseventid`, outside the `currhashcode`, and merges as `msgoriginator` does; a capture fact |
| `isincode` | `IsinCode` | 65023 | the `ISIN` entry of the message's [security identifiers](message.md#typed-tags), stated directly by a bridge or through `SecurityID(48)` under source `4` or a `secaltids` occurrence: a view of `get_securityids()`, row-stated when written; nullable |
| `forexcode` | `ForexCode` | 65024 | the `FOREX` entry of the message's security identifiers - the [currency pair](../types/codes/forex.md) it is about, canonical `CCY/CCY` - stated by a row, else [detected](#a-currency-pair-is-read-off-the-symbol) off `Symbol(55)` where the message states no other class: a view of `get_securityids()`, row-stated when written; nullable |
| `bloombergcode` | `BloombergCode` | 65025 | the `BLOOMBERG` entry of the message's security identifiers, stated directly or under source `A`: a view of `get_securityids()`, row-stated when written; nullable |
| `figicode` | `FIGICode` | 65026 | the `FIGI` entry of the message's security identifiers, stated directly or under source `S`: a view of `get_securityids()`, row-stated when written; nullable |
| `miccode` | `MicCode` | 65027 | the MIC the message names for its market: a stated one, else `LastMkt(30)`, `ExDestination(100)`, the market a bridge's [instrument key](message.md#typed-tags) names, then `SecurityExchange(207)`, the first an ISO 10383 MIC - `Mic::is_iso`, four upper-case letters or digits - or a Reuters mnemonic resolving to one; a currency pair naming none of them is ISO 10383's `XXXX`. A bridge's `INSTRUMENT[EXCHANGE]` is its alias and states it; a short code there, `S` or `TW`, is an [anomaly](message.md#anomalies) and stays a child of its own spelling while the ladder answers; nullable |
| `strikepx` | `StrikePx` | 65028 | the option strike price the message identifies: `StrikePrice(202)` as the `decimal` leaf; a row stating one is the row's word; nullable |
| `state` | `State` | 65029 | the state the message reached, stored as the `int32` code of its [member](../types/enum/state.md#the-code-is-the-rank) and read by the crate's `statecodeset`: the first status field the message states - `OrdStatus(39)`, `ExecType(150)`, `ExecAckStatus(1036)`, `TrdRptStatus(939)`, `QuoteStatus(297)`, `AllocStatus(87)`, `ConfirmStatus(665)`, `AffirmStatus(940)`, `MassActionResponse(1375)`, `MassCancelResponse(531)` - else what its message type asks for, `UNKNOWN` where neither states one; `FILLED` on an execution a parse split off; the furthest its chain knows once the [lifecycle](lifecycle.md) followed it, and a row stating one is the row's word; nullable, and never null on a row a message wrote |
| `metadata` | `Metadata` | 65030 | a nullable sorted Map of what a message stated that is no field: a bridge's namespaced keys - `TECH.CLIENTID`, `firm.acronym`, `ullink.clientid` - and every key no dictionary resolved, each under the key as it was spelled, folded ([above](#nothing-is-lost-at-the-end)); a namespaced spelling the dictionary holds whole, `ULLINK.INSTRUMENTID`, is that field instead |
| `sourceurl` | `SourceUrl` | 65031 | the object the line was read from, typed as a `url`: the capture's own column and no column of the fixed row, declared by whoever read the line and carried in front of the row, carried by every message parsed out of or read back from that row as [what it carries](message.md#a-row-is-a-message-again) and stated again at its column by `into_row`, and held as no fact, so outside the `currhashcode`, the entries and the wire; null where nobody stated one, because the same message read from a second copy of one day's log is the same message |
| `omsdealeraccount` | `OmsDealerAccount` | 65032 | the account an OMS dealer books the order to: content, which a leaf's metadata carries; nullable |
| `omsuserid` | `OmsUserId` | 65033 | the OMS user who entered the order: content, which a leaf's metadata carries; nullable |
| `parentorderid` | `ParentOrderId` | 65034 | the parent order's identifier: content, `altids` under `PARENTORDERID`, carried by a following operation; nullable |
| `parentclordid` | `ParentClOrdId` | 65035 | the parent order's client identifier: content, `altids` under `PARENTCLORDID`, followed; nullable |
| `omsdealerparentorderid` | `OmsDealerParentOrderId` | 65036 | the OMS dealer's parent order identifier: content, `altids` under `OMSDEALERPARENTORDERID`, followed; nullable |
| `exchangeclientorderid` | `ExchangeClientOrderId` | 65037 | the exchange's client order identifier: content, `altids` under `EXCHANGECLIENTORDERID`, followed; nullable |
| `transversalkey` | `TransversalKey` | 65038 | the key a bridge threads one order through every system with: content, `altids` under `TRANSVERSALKEY`, followed; nullable |
| `ultraderclordid` | `UlTraderClOrdId` | 65039 | a trading screen's own client order identifier: content, `altids` under `ULTRADERCLORDID`, never carried by a following operation; nullable |
| `omsinstrumentid` | `OmsInstrumentId` | 65040 | the instrument an OMS bridge names, as it stated it - `dbi;CH0012214059_XSWX_CHF` - content, never a column of the fixed row, and a `secaltids` occurrence under the `OMSINSTRUMENTID` source; nullable |
| `ullinkinstrumentid` | `UllinkInstrumentId` | 65041 | the instrument a ULLINK bridge names, its alias `ullink.instrumentid` the spelling the bridge writes: content, never a column of the fixed row, and a `secaltids` occurrence under the `ULLINKINSTRUMENTID` source; nullable |

FIX keeps the market facts it names directly: the prices, quantities, currency, side, unit and the quote's bid and offer remain under their standard tags, as does the classification `CFICode(461)`. CFI therefore has no crate duplicate. The five crate code columns give typed locations for normalized identifiers without discarding the standard fields that stated them: `isincode`, `forexcode`, `bloombergcode` and `figicode` are views of the message's security identifiers - `get_securityids()`, built at every settle as the [message states them](message.md#typed-tags) - and `miccode` is the market; `strikepx` types the strike `StrikePrice(202)` states. A CUSIP or a SEDOL is one more security identifier under its own key, `ids.get("CUSIP")`, never a column of its own; the national number a canonical ISIN embeds is derived into a key the message does not state and reaches no field and no wire. `msgcat` similarly projects the symbolic category attached to `MsgType(35)` through the registry's code set into the `MarketDataKind` member, while `msgtype` remains the standard field.

`currhashcode` is the one stored content identity and `crossuuid` the chain's; what each digests, and why a projection that adds or renames columns leaves them alone, is the [message's identity](../hashing.md). `FixMsg::digest` is a separate contract: the XXH3-128 of what the wire emits, so two identical orders read with two separators digest equal, and [`FixDedup`](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) drops the second.

The three capture facts a read states - `msgpluginid`, `msgctxid`, `msgsessionid` - come from [row-header captures](arrow.md#a-bridge-log-names-what-it-fills) or columns of the read, without overwriting a value the message stated. A complete nonempty `msgtype`, `msgsessionid` and `msgctxid` with a present `msgseqnum` join to the [delivery identity](lifecycle.md#a-twin-is-not-a-successor) `msgsesseventid`, the fourth fact the capture holds and a column of the fixed row of its own: the four values joined by `:` exactly as stated, `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>` - `8:e7256476:9effef3e6a:1094` - the sequence in its canonical `u64` spelling. The message derives it again whenever it settles, so any missing part removes it, a cell a row states for it is replaced by the join rather than read. A session or context holding a `:` of its own could join two splits to one key; a bridge names none that way. It is capture provenance and excluded from the FIX content identity. Equal `msgsesseventid` values force a full merge before ordinary with-previous following: the observation with the later `recdunix` is the reference - a stated one leads an absent one, and the later `currunix` closes a tie - its scalar conflicts win, older facts fill absences, and equal-index group occurrences merge recursively in FIX tag order. The merged `recdunix` and `execunix` are the earliest observations, so a merged statement ranks by its earliest recording against a third. A capture or column named `sourceurl` fills nothing at all: the cell is carried by every message read out of that row and stated again at its own column by `into_row`. Market values remain FIX fields; the normalized identifier columns, plus category, state, expiry, execution and recording instants, are typed readings built from them. A lifecycle carries the furthest state and the current generation's deadline: a newer explicit `exprunix` can shorten it, while an absent one inherits. The parse fills an absent `execunix` from `currunix` only where `FixMsg::is_execution` says that observation reports an execution - including a qualifying initial AE even without an execution lifecycle state, and excluding cancel/correct/status reports even where their carried state is filled - and the lifecycle carries the later of that clock and the predecessor's through successors; `recdunix` remains a fact of the observation. Duplicate statements merge execution and recording facts to their earliest values. `prevuuid`, `prevunix` and the folded `creaunix` are stamped by the [lifecycle](lifecycle.md), `seqnum` is the message's [place at its instant](lifecycle.md#a-place-counts-one-instant), which the parse gives by order, the walk by content and a market leaf takes from its message, and `snapunix` only on a separate grid view; `srcuuids` is stated where a message is parsed out of a `TextLine` - the line's own `curruuid` - and where a parse [splits a message off](message.md#a-parse-splits-what-a-message-reports) another, and by nothing else, because a walk carries no source along a chain. FIX's `SenderCompID` and `TargetCompID` name counterparties; the plugin carrying a message inside a bridge is a separate fact.

There is no partition column. How a layout is cut is the target's to decide: an Iceberg table takes an `hour` transform over `currunix` and reads the instant the row already carries, so a materialized copy of it was a second owner of one fact. A reader that wants the hour asks the target for it.

### Every message is dated

These facts are settled when a message is built, whatever its line carried, and none of them becomes a pair on the wire unless the wire sent it.

`beginstring` is the wire's own `BeginString(8)` when stated, else `FIX.4.4`, the crate's own, filled by the builder so that every row states one: a bridge row, and the row a JSON document is, say which FIX they were read as exactly as a frame does. A codec pins none - a version is what a line said, never a caller's statement about a whole run - and the dictionary states none to lend: it holds every tag ever defined and filters by none.

Six values close every message and are never null: `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` and `crossuuid`; `state` is written on every one of them too, `UNKNOWN` where nothing states one, and stays a nullable column because a state has no neutral member for an empty cell to read as. Initial intake settles the clocks once. `SendingTime(52)` is the message's own, else a row cell or line capture reaching tag 52, else the `currunix` of the [text line](../media/index.md#plain-text) the message was read out of - its `mtime` capture, else its handle's modification time; on the [Arrow text door](arrow.md), the batch's `currunix` column; a line at the epoch is a line nothing dated, on either door - else the codec's `default_sending_time`, else one UTC-now read for that undated message. The line's clock leads the pin because the instant a line was recorded at is nearer the send than any clock a caller pins for a whole run; bytes read with no line behind them - `parse_line`, `parse_fix_line`, `parse_lines`, `parse_pairs` - go straight to the default, and a root built by `FixMsg::new` to now. Only a stated one is a fact of the message, so only a stated one goes back on the wire and into the row's tag-52 column, which `FixHeader::stated_sendingtime` answers. That clock is the **reference** every parse dates against, and `currunix` is a stated `currunix`, else [the official clock](#the-official-clock-dates-the-message) standing within the codec's `official_time_delay_ms` of it, else the sending clock itself; `creaunix` is a stated one, else `currunix`. What a resend's `OrigSendingTime(122)` says stays the [lifecycle](lifecycle.md)'s to read off the structured message. `execunix` reads the most precise direct execution clock the message gives, and a report `FixMsg::is_execution` accepts that states none and follows nothing takes its settled `currunix` right at intake, because an execution report stating no other clock executed when it happened; `recdunix` is the enclosing line's recording clock - the same line clock - and no sending clock ever stands in for a missing one. A stated clock that names no instant is left unstated - beside an anomaly naming the text and a [warning](#warnings) - and the message is dated as one stating none is: the message states no sending clock, and the row's tag-52 column none. No wall clock is read after intake - writes, row exchange, the lifecycle and replay carry the settled values - so a read that must be reproducible reads lines that carry their clock, pins `default_sending_time`, or carries the settled rows. `header().sendingtime()` and the event's clock getters answer them without a lookup, in nanoseconds since the epoch, and `by_tag` answers the same clocks under their standard or crate tags.

Dated, each message takes its [place](lifecycle.md#a-place-counts-one-instant) by order: the next of its instant's run, within a row and across every row the door reads, so a report and the execution the parse [split off](message.md#a-parse-splits-what-a-message-reports) it are places 0 and 1, and the split message's `srcuuids` name the identity its source was placed under.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl::{CREAUNIX_TAG_NAME, FixCodec, FixRegistry, Scalar, TimeUnit, Timezone, CURRUNIX_TAG_NAME, fix_crate_fields};

    let fields = fix_crate_fields()?;
    assert_eq!(fields.len(), 41);
    // No partition column: how a layout is cut is the target's to decide.
    assert!(fields.iter().all(|field| !field.is_partition()));
    // The two identities are the crate's own uuid, the codes plain integers.
    let by_name = |name: &str| fields.iter().find(|field| field.name() == name).expect("a crate column");
    assert_eq!(by_name("curruuid").dtype(), &yggdryl::DataType::uuid());
    assert_eq!(by_name("crossuuid").dtype(), &yggdryl::DataType::uuid());
    assert_eq!(by_name("currhashcode").dtype(), &yggdryl::DataType::UInt64);
    // And no market column: a price is `Price(44)`, the dictionary's own.
    assert!(fields.iter().all(|field| field.name() != "price" && field.name() != "quantity"));

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let default = Scalar::datetime64(1_704_190_530_000_000_000, TimeUnit::Nanosecond, Timezone::UTC)?;
    let reader = FixCodec::new(Arc::clone(&registry))
        .try_with_default_sending_time(Some(default.clone()))?;
    // A frame stating neither its version nor a clock still states a version
    // - the crate's own 4.4 - and is dated by the codec's default sending time.
    let bare = reader.parse_line(b"35=D|55=AAPL|10=0|")?.next().expect("one frame")?;
    assert_eq!(bare.header().beginstring(), "FIX.4.4");
    assert_eq!(bare.by_tag(8)?.as_str(), Some("FIX.4.4"));
    assert_eq!(bare.by_tag(52)?, default);
    assert!(!bare.header().stated_sendingtime());
    assert_eq!(bare.by_tag(CURRUNIX_TAG_NAME.0)?, default);
    assert_eq!(bare.by_tag(CREAUNIX_TAG_NAME.0)?, default);
    assert_eq!(bare.get_currunix(), 1_704_190_530_000_000_000);
    assert_eq!(bare.get_curruuid(), bare.time_uuid()?);
    // None of them became a pair on the wire: the header re-emits the
    // version, and the content what the frame stated, a day order derived.
    assert_eq!(bare.into_text('|')?, "8=FIX.4.4|35=D|55=AAPL|59=0|10=0|");

    // A frame stating its clocks keeps them: the stated SendingTime goes
    // back on the wire, and the transaction half a second in front of it is
    // inside the default one-second delay, so it is the same event said
    // twice and the more exact saying of it dates the message.
    let sent = reader
        .parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(sent.header().beginstring(), "FIX.4.2");
    assert!(sent.header().stated_sendingtime());
    assert_eq!(sent.by_tag(52)?.temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_200_415));
    assert_eq!(sent.get_currunix(), 1_787_308_199_900_000_000);
    assert_eq!(sent.get_creaunix(), Some(sent.get_currunix()));
    assert_eq!(sent.by_tag(60)?.temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_199_900));
    assert!(sent.get_snapunix().is_none(), "a read is not a snapshot");
    assert!(sent.into_text('|')?.starts_with("8=FIX.4.2|35=D|52=20260821-10:30:00.415|"));
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    from yggdryl import DataType
    from yggdryl.fix import FixCodec, FixRegistry, fix_crate_fields

    UNIX, CREAUNIX = 65001, 65004
    fields = list(fix_crate_fields())
    assert len(fields) == 41
    # No partition column: how a layout is cut is the target's to decide.
    assert not any(field.is_partition for field in fields)
    # The two identities are the crate's own uuid, the codes plain integers.
    by_name = {field.name: field for field in fields}
    assert by_name["curruuid"].dtype == DataType("uuid")
    assert by_name["crossuuid"].dtype == DataType("uuid")
    assert by_name["currhashcode"].dtype == DataType("uint64")
    assert "price" not in by_name and "quantity" not in by_name

    default = datetime(2024, 1, 2, 10, 15, 30, tzinfo=timezone.utc)
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry, default_sending_time=default)

    # A frame stating neither its version nor a clock still states a version - the
    # crate's own 4.4 - and is dated by the codec's default sending time.
    bare = next(reader.parse_line(b"35=D|55=AAPL|10=0|"))
    assert bare.header().beginstring == "FIX.4.4"
    assert bare.by_tag(8).as_py() == "FIX.4.4"
    assert bare.by_tag(52).as_py() == default
    assert not bare.header().stated_sendingtime
    assert bare.by_tag(UNIX).as_py() == default
    assert bare.by_tag(CREAUNIX).as_py() == default
    assert bare.currunix == 1_704_190_530_000_000_000
    # None of them became a pair on the wire: the header re-emits the version, and
    # the content what the frame stated, a day order derived.
    assert bare.into_text("|") == "8=FIX.4.4|35=D|55=AAPL|59=0|10=0|"

    # A frame stating its clocks keeps them: the stated SendingTime goes back on
    # the wire, and the transaction half a second in front of it is inside the
    # default one-second delay, so it is the same event said twice and the more
    # exact saying of it dates the message.
    sent = next(reader.parse_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|"))
    assert sent.header().beginstring == "FIX.4.2"
    assert sent.header().stated_sendingtime
    assert sent.by_tag(52).as_py() == datetime(2026, 8, 21, 10, 30, 0, 415000, tzinfo=timezone.utc)
    assert sent.currunix == 1_787_308_199_900_000_000
    assert sent.creaunix == sent.currunix
    assert sent.by_tag(60).as_py() == datetime(2026, 8, 21, 10, 29, 59, 900000, tzinfo=timezone.utc)
    assert sent.snapunix is None, "a read is not a snapshot"
    assert sent.into_text("|").startswith("8=FIX.4.2|35=D|52=20260821-10:30:00.415|")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { DataType, fix } = require('yggdryl')

    const [UNIX, CREAUNIX] = [65001, 65004]
    const fields = fix.crateFields()
    assert.equal(fields.length, 41)
    // No partition column: how a layout is cut is the target's to decide.
    assert.ok(fields.every((field) => !field.isPartition))
    // The two identities are the crate's own uuid, the codes plain integers.
    const byName = Object.fromEntries(fields.map((field) => [field.name, field]))
    assert.equal(byName.curruuid.dtype.toString(), 'uuid')
    assert.equal(byName.crossuuid.dtype.toString(), 'uuid')
    assert.equal(byName.currhashcode.dtype.toString(), 'uint64')
    assert.equal(byName.price, undefined)
    assert.equal(byName.quantity, undefined)

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry, {
      defaultSendingTime: new Date(Date.UTC(2024, 0, 2, 10, 15, 30)),
    })

    // A frame stating neither its version nor a clock still states a version - the
    // crate's own 4.4 - and is dated by the codec's default sending time.
    const bare = reader.parseLine(Buffer.from('35=D|55=AAPL|10=0|')).next().value
    assert.equal(bare.header().beginstring, 'FIX.4.4')
    assert.equal(bare.byTag(8).asJs(), 'FIX.4.4')
    assert.ok(bare.byTag(52).equals(reader.defaultSendingTime))
    assert.ok(bare.byTag(UNIX).equals(bare.byTag(52)))
    assert.ok(bare.byTag(CREAUNIX).equals(bare.byTag(52)))
    assert.equal(bare.currunix, 1_704_190_530_000_000_000n)
    // None of them became a pair on the wire - only a stated sending clock is a
    // fact of the message - so the header re-emits the version and the content
    // what the frame stated, a day order derived.
    assert.equal(bare.intoText('|'), '8=FIX.4.4|35=D|55=AAPL|59=0|10=0|')

    // A frame stating its clocks keeps them: the stated SendingTime goes back on
    // the wire, and the transaction half a second in front of it is inside the
    // default one-second delay, so it is the same event said twice and the more
    // exact saying of it dates the message.
    const sent = reader
      .parseLine(Buffer.from('8=FIX.4.2|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|55=AAPL|10=0|'))
      .next().value
    assert.equal(sent.header().beginstring, 'FIX.4.2')
    assert.equal(sent.currunix, 1_787_308_199_900_000_000n)
    assert.equal(sent.creaunix, sent.currunix)
    assert.equal(sent.byTag(60).asJs().getTime(), Date.UTC(2026, 7, 21, 10, 29, 59, 900))
    assert.equal(sent.snapunix, null, 'a read is not a snapshot')
    assert.ok(sent.intoText('|').startsWith('8=FIX.4.2|35=D|52=20260821-10:30:00.415|'))
    ```

### The official clock dates the message

`SendingTime(52)` is when a session put the message on the wire, which is not when the thing it reports happened. A venue that stamps its own clock says that more exactly, and the distance between the two is the only evidence a parse has that the two clocks are saying the same thing: inside a hop they are one event said twice and the venue's saying is the better one; a whole second apart they are two events - a resend of an older order, a report batched behind the trades it covers, a clock nobody disciplined - and dating the message by the far one would move it out of the order it was sent in. `official_time_delay_ms` / `officialTimeDelayMs` is that distance, one second by default, and it is a pin on the codec for the whole run.

The parse ranks what the message says about its own event and takes the best clock standing inside the delay, on either side of the sending clock:

| Rank | Clock | Read as |
| --- | --- | --- |
| 1 | `TransactTime(60)` | the message's own statement of when its transaction happened. A day-only value - `60=20260821`, which the parse restates as that day's midnight - dates nothing: what it says is the day |
| 2 | `TrdRegTimestamp(769)` whose `TrdRegTimestampType(770)` is `ExecutionTime(1)`, `BrokerExecution(5)`, `TimePriority(8)`, `OrderbookEntryTime(9)`, `OrderSubmissionTime(10)`, `OrderCancellationTime(29)`, `OrderModificationTime(30)`, `TradeCancellationTime(32)` or `TradeModificationTime(33)` | the venue's own stamp of the event itself |
| 3 | `TrdRegTimestamp(769)` whose type is `TimeIn(2)`, `TimeOut(3)`, `BrokerReceipt(4)`, `DeskReceipt(6)` or `OrderRoutingTime(31)` | a hop the message crossed on its way here: nearer the event than the sending clock, further from it than the stamps above |
| - | every other `TrdRegTimestampType`, and a code no set names | never a clock. Submission to clearing, public and non-public reporting and their updates, confirmation, clearing, allocation, submission to a repository, continuation events, valuation, an identifier's assignment, affirmation and a bare update time all happen *after* the event; a previous time priority and a previous identifier describe the state this one replaced; a reference time for the BBO describes the market it was measured against. An unranked stamp is silence, never a guess |

Two clocks of one rank are decided by the nearer of them, and the earlier instant closes the last tie, so one row reads one way. The `TrdRegTimestamps(768)` group is read **as a group**: `TrdRegTimestamp` says nothing on its own - the same tag carries an execution's instant, a desk's receipt and the moment a report reached a repository - and what tells them apart is the `TrdRegTimestampType` standing beside it in the same occurrence, which only a dictionary declaring the group pairs. Where nothing inside the delay qualifies, the sending clock dates the message, as it always did.

This is parse behavior and depends on the one message being parsed, so it composes with everything downstream untouched. What a message whose sending clock was a *stand-in* does is the [lifecycle](lifecycle.md)'s separate reading: `FixMsg::dated_by_transaction` re-dates it by `TransactTime` with no delay at all, because a clock nobody stated is no reference to measure a distance from.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::graph::Event;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(registry);
    assert_eq!(FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_MS, 1_000);
    assert_eq!(codec.official_time_delay_ms(), 1_000);

    // A report stating no TransactTime and two regulatory stamps. The nearer
    // one is when the report reached a repository, which is not when the
    // trade happened; the execution half a second earlier is.
    let report = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|\
          769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|",
    )?;
    assert_eq!(report.get_currunix(), 1_787_308_199_900_000_000);

    // Five seconds out is a different event of the session's day, whatever
    // its type says, so the one clock every message carries keeps it.
    let apart = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|\
          769=20260821-10:29:55|770=1|10=0|",
    )?;
    assert_eq!(apart.get_currunix(), 1_787_308_200_415_000_000);

    // The delay is the caller's to widen, and a nonpositive one admits only a
    // clock equal to the sending clock.
    let wide = codec.clone().with_official_time_delay_ms(10_000);
    assert_eq!(wide.official_time_delay_ms(), 10_000);
    let widened = wide.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|\
          769=20260821-10:29:55|770=1|10=0|",
    )?;
    assert_eq!(widened.get_currunix(), 1_787_308_195_000_000_000);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)
    assert codec.official_time_delay_ms == 1_000

    # A report stating no TransactTime and two regulatory stamps. The nearer one
    # is when the report reached a repository, which is not when the trade
    # happened; the execution half a second earlier is.
    report = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|"
        b"769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|"
    )
    assert report.currunix == 1_787_308_199_900_000_000

    # Five seconds out is a different event of the session's day, whatever its
    # type says, so the one clock every message carries keeps it.
    apart = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|"
        b"769=20260821-10:29:55|770=1|10=0|"
    )
    assert apart.currunix == 1_787_308_200_415_000_000

    # The delay is the caller's to widen.
    wide = FixCodec(registry, official_time_delay_ms=10_000)
    assert wide.official_time_delay_ms == 10_000
    widened = wide.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|"
        b"769=20260821-10:29:55|770=1|10=0|"
    )
    assert widened.currunix == 1_787_308_195_000_000_000
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)
    assert.equal(codec.officialTimeDelayMs, 1_000)

    // A report stating no TransactTime and two regulatory stamps. The nearer one
    // is when the report reached a repository, which is not when the trade
    // happened; the execution half a second earlier is.
    const report = codec.parseFixLine(
      Buffer.from(
        '8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|' +
          '769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|',
      ),
    )
    assert.equal(report.currunix, 1_787_308_199_900_000_000n)

    // Five seconds out is a different event of the session's day, whatever its
    // type says, so the one clock every message carries keeps it.
    const apart = codec.parseFixLine(
      Buffer.from(
        '8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|769=20260821-10:29:55|770=1|10=0|',
      ),
    )
    assert.equal(apart.currunix, 1_787_308_200_415_000_000n)

    // The delay is the caller's to widen.
    const wide = new fix.FixCodec(registry, { officialTimeDelayMs: 10_000 })
    assert.equal(wide.officialTimeDelayMs, 10_000)
    const widened = wide.parseFixLine(
      Buffer.from(
        '8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|769=20260821-10:29:55|770=1|10=0|',
      ),
    )
    assert.equal(widened.currunix, 1_787_308_195_000_000_000n)
    ```

## What a message implied is filled in

A venue sends what its counterparty needs and nothing more, so a row is routinely missing values the message itself already determines: a report stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill stating `LastQty` and `LastPx` has said what it was worth, and a message naming its instrument by an ISIN has said which country issued it. A parse fills them: there is no `enrich` door, because a message a reader answers is a message already filled, and a second pass over it would be a second reading of one line.

The fill is part of building the message, and its steps are the parse's own rather than something a caller composes. The message is **restated** - the row [re-expressed under the dictionary](message.md#restated-under-the-dictionary) - because every rule below reads a child by its tag or its canonical name, and a child a session spelled under an alias is invisible until it has been canonicalized. A currency pair its symbol names is then [detected](#a-currency-pair-is-read-off-the-symbol), so the rules read the cells it fills. The parse then **derives** what the message implies and did not state, by the crate's native rules - twenty-nine, one per target tag, read off the message's own children and settled to a fixpoint. A registry states no rule of its own: its field for a target types the answer, and a registry lacking that field fills nothing there, so two registries holding the same fields fill a message alike. Then the security identifiers and the [alternate identifiers](message.md#the-identifier-maps) are read off the fields that state them, and the identity settles once, after everything the pass wrote.

No alias twin is added as a second child, because a spelling is a way of asking rather than a thing to store: a consumer addressing `lastshares` finds the `lastqty` the message states, since `get_by_name` resolves the spelling through the registry, which answers a field for any alias it holds. Two of the forty-two shipped aliases are another field's canonical name - `quoteackstatus` is an alias of `QuoteStatus(297)` and the name of tag 1865, `tradetype` an alias of `BidTradeType(418)` and the name of tag 3006 - so a message stating both would hold two children of one name, and the root would be refused whole. A twin could not cross the batch door either: [`fix_schema`](#the-columns-are-the-folded-names) names a column for a canonical field and for no alias. And `get_by_tag` answers the earliest child on a tag, so where the twin was placed would silently decide what every column, every rule and the lifecycle read.

Three things hold whatever the rule. A rule writes the row, which the entries and the wire read, so a derived value is emitted behind what arrived - `381=50` after a fill of five at ten - and a message read back from its own bytes states the same content again. A rule answers only where the answer is certain: an absent input is silence, so an identifier no check digit closes, a CFI whose category several security types share and a security type outside every group the specification files answer nothing rather than a guess. And a stated value is never overwritten, which is what makes a second pass change nothing - a value derived once is a stated value the second time.

| Contract | Rule |
| --- | --- |
| Owner | the crate's native code, one rule per target tag, the same for every registry; no `FIX:` key states a rule, and the only registry fact a rule reads is the field that types its answer |
| Reads | the message's own children by tag - `secaltids` by its canonical name - and each answer already landed in the pass, so a rule reads what an earlier one filled |
| Fixpoint | evaluation continues until one pass writes nothing, bounded by the twenty-nine targets, so a chain settles in whatever order its fields fall: `securityidsource` from `securityid` and `securityid` from the alternate identifiers, `product` after `securitytype` after `cficode`, `leavesqty` after `ordstatus` after `exectype` |
| Landed | everything that derived reaches the row through one `set_each`, one rebuild for the whole pass, each value under the dictionary's own field for the tag; a row holding a stated null takes the answer in that child's place, anything else is appended |
| Silence | an absent input, a condition that does not hold, arithmetic that overflows, a number the stated scale cannot hold exactly and an answer the field refuses are all a null column and never a failure - the answer the field refuses is dropped beside a [warning](#warnings) naming its tag |

What each rule reads, in words:

| Fills | From | The table it reads |
| --- | --- | --- |
| `SecurityIDSource(22)` | `SecurityID(48)` | the `SecurityIDSource` code set names the standard each code stands for, and each standard closes its identifiers with a check digit: `4` for a number ISO 6166 closes, `1` for a CUSIP, `2` for a SEDOL |
| `SecurityID(48)` | the first `SecurityAltID(455)` whose `SecurityAltIDSource(456)` is `4` | ISO 6166: a message naming an ISIN only as an alternate has stated its primary identifier, and a value the check digit does not close answers nothing |
| `Symbol(55)` | `SecurityID(48)` under source `8` or `A`, else the `SecurityAltID(455)` whose source is `8` | the `SecurityIDSource` codes of an exchange symbol and a Bloomberg symbol |
| `CountryOfIssue(470)` | the ISIN, wherever the message stated it | ISO 6166 opens a number with the ISO 3166 code of the numbering agency's country, where it is one: the prefix answers exactly where the crate's own registry of countries, `StringEnum::COUNTRIES`, lists it - 249 codes - so `XS`, `EU` and every unassigned pair answer nothing |
| `SecurityType(167)` | `CFICode(461)` | Appendix 6-D at its category level, case not counting - `ES` is `CS`, `F` is `FUT`, an `O?F` is `OOF` and every other `O` is `OPT`, `LR` is `REPO`, `IF` is `FXSPOT`, `JF` is `FXFWD` and `FXNDF` where ISO 10962's attributes say non-deliverable, `SF` is `FXSWAP` and `FXNDS` where non-deliverable; a category every kind of bond shares, `DB`, answers nothing |
| `CFICode(461)` | `SecurityType(167)`, `PutOrCall(201)` | Appendix 6-D the other way: `CS` is `ESXXXX`, `CORP` is `DBXXXX`, `FRN` is `DBVXXX`, an option is `OC` or `OP` by its `PutOrCall` and `OX` without one, `FXSPOT` is `IFXXXP`, `FXFWD` `JFTXFP`, `FXNDF` `JFTXFN`, `FXSWAP` `SFXXXP`, `FXNDS` `SFXXXN` |
| `PutOrCall(201)` | `CFICode(461)` | the second character of a listed (`O`) or unlisted (`H`) option: `C` is a call, `P` a put |
| `Product(460)` | `SecurityType(167)`, else `CFICode(461)` | the group the dictionary's `SecurityType` code set files the value under, as the `Product` code set spells it - `Agency` is `1`, `Corporate` `3`, `Currency` `4`, `Equity` `5`, `Government` `6`, `Loan` `8`, `Money Market` `9`, `Mortgage` `10`, `Municipal` `11`, `Financing` `13`; `Derivatives` and `Other` answer nothing. A CFI in category `E` is `5` and in `L` is `13` |
| `TimeInForce(59)` | nothing, on an order `D`, a replace `G` or a report `8` | the field's own definition: absent means `0`, a day order |
| `OrdStatus(39)` | `ExecType(150)`; else `LeavesQty(151)` and `CumQty(14)` on a trade | the values the two code sets spell alike - not `D`, Restated in one and AcceptedForBidding in the other; a trade leaving nothing is filled, `2`, and one leaving something after doing something is partially filled, `1` - each landing under `OrdStatus`'s own code set, which is where `get_state` reads the element's [state](../types/enum/state.md) |
| `LeavesQty(151)`, `OrderQty(38)`, `CumQty(14)` | the other two, on a report | Appendix D: `OrderQty = CumQty + LeavesQty`, nothing is left once `OrdStatus(39)` is closed, and what a canceled order asked for is what it did plus `CxlQty(84)`: a report with nothing left - stated or unstated - that states a canceled quantity answers `CumQty + CxlQty`, one that states what is left answers `CumQty + LeavesQty` whatever it canceled, and `CxlQty` is read last where nothing says what is left |
| `GrossTradeAmt(381)` | `LastQty(32)` x `LastPx(31)` | Appendix D's execution reports; each operand is stated at scale nine, half the target scale, so the product lands back at eighteen digits |
| `SettlCurrAmt(119)` | `GrossTradeAmt(381)` x `SettlCurrFxRate(155)` | Appendix O, at the same scales |
| `Currency(15)`, `SettlCurrency(120)` | each other | Appendix O: a trade settling in the currency it was dealt in states it once - except on a currency product, `Product(460)` `4`, whose two currencies are the pair's two legs |
| `AvgPx(6)` | `LastPx(31)` | Appendix D, only where `CumQty(14)` equals a positive `LastQty(32)`, the whole done quantity being this fill |
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

A report is `MsgType` `8` or `9`. No caller supplies an order: the rules keep each answer visible to the ones after them and continue until one writes nothing, so a `SecurityID`'s validation states the source it was issued under, an ISIN found only among the alternate identifiers becomes the `SecurityID` whose validation states the source in turn, the country is read after either, and a status read off an execution type decides what is left. The fixpoint reaches what one evaluation round could not: a forward price derived from spot and points is the price the worth and the average read, a quantity ordered off what was canceled is the one the remainder reads, and a trade over several periods multiplies and prices through the quantity it derived.

What the pass leaves null it leaves null on purpose, and a reader needs to be able to tell that from a gap. No amount is filled whose scale depends on a convention the message does not state: `GrossTradeAmt(381)` from a percent-of-par price needs a division by one hundred that the specification writes in price units and leaves to the reader; an FX gross amount is a product or a quotient depending on `SettlPriceFxRateCalc(2366)`, and absent that tag the quoting convention decides; `NetMoney(118)` needs `Commission(12)` resolved through `CommType(13)` and every `MiscFeeAmt(137)` through `MiscFeeBasis(891)`. A capture reader that guesses a notional is worse than one that leaves it null. Nor is anything filled that needs a second message - `OrigClOrdID(41)` from the request a report answers, `ListID(66)` from the list an order belonged to, a bust's effect on `CumQty(14)` - because each is a fact about a chain rather than about a message, and chains are the [lifecycle's](lifecycle.md). Nor a rate a message's amounts divide by: the market fact [`fxrates`](../graph/market.md#fx-rates) is filled by no FIX field.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::graph::{Event, Market};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|";
    let held = reader.parse_line(line)?.next().expect("one frame")?;
    assert_eq!(held.by_tag(22)?.as_str(), Some("4"));
    assert_eq!(held.by_tag(470)?.as_str(), Some("US"));
    assert_eq!(held.by_tag(167)?.as_str(), Some("CS"));
    assert_eq!(held.by_tag(460)?, Scalar::from(5_i32));
    // The instrument's own identifiers and its market are no columns of
    // this crate's: the traits read them off FIX's own fields, the
    // identifiers as one set keyed by source.
    assert_eq!(held.get_securityids().get("ISIN"), Some("US0378331005"));
    assert_eq!(held.get_miccode().map(|held| held.as_str()), Some("XNAS"));
    // A trade leaving nothing is filled: FIX's own code in `OrdStatus`, and
    // the crate's lifecycle-sorted state beside it.
    assert_eq!(held.by_tag(39)?.as_str(), Some("2"));
    assert_eq!(held.get_state().as_str(), "FILLED");
    assert_eq!(held.by_tag(59)?.as_str(), Some("0"), "a day order");

    // The wire carries what arrived and then what the message implied, so
    // reading it back states the same content again - a second read dates
    // an undated line at its own now, which is the one thing that moves.
    assert_eq!(reader.parse_fix_line(&held.into_bytes(b'|'))?.entries(), held.entries());
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import State
    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|"
    held = next(reader.parse_line(line))
    assert held.by_tag(22).as_py() == "4"
    assert held.by_tag(470).as_py() == "US"
    assert held.by_tag(167).as_py() == "CS"
    assert held.by_tag(460).as_py() == 5
    # The instrument's own identifiers and its market are no columns of this
    # crate's: the properties read them off FIX's own fields, the identifiers
    # as one dict keyed by source.
    assert held.securityids["ISIN"] == "US0378331005"
    assert held.miccode.as_py() == "XNAS"
    # A trade leaving nothing is filled: FIX's own code in `OrdStatus`, and the
    # crate's lifecycle-sorted state beside it.
    assert held.by_tag(39).as_py() == "2"
    assert held.state == State.FILLED
    assert held.by_tag(59).as_py() == "0", "a day order"

    # The wire carries what arrived and then what the message implied, so reading
    # it back states the same content again - a second read dates an undated line
    # at its own now, which is the one thing that moves.
    assert reader.parse_fix_line(held.into_bytes(ord("|"))).entries() == held.entries()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    const line = '8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|'
    const held = reader.parseLine(Buffer.from(line)).next().value
    assert.equal(held.byTag(22).asJs(), '4')
    assert.equal(held.byTag(470).asJs(), 'US')
    assert.equal(held.byTag(167).asJs(), 'CS')
    assert.equal(held.byTag(460).asJs(), 5)
    // The instrument's own identifiers and its market are no columns of this
    // crate's: the getters read them off FIX's own fields, the identifiers as
    // one record keyed by source.
    assert.equal(held.securityids.ISIN, 'US0378331005')
    assert.equal(held.miccode, 'XNAS')
    // A trade leaving nothing is filled: FIX's own code in `OrdStatus`, and the
    // crate's lifecycle-sorted state beside it.
    assert.equal(held.byTag(39).asJs(), '2')
    assert.equal(held.state, 'FILLED')
    assert.equal(held.byTag(59).asJs(), '0', 'a day order')

    // The wire carries what arrived and then what the message implied, so reading
    // it back states the same content again - a second read dates an undated line
    // at its own now, which is the one thing that moves.
    const again = reader.parseFixLine(Buffer.from(held.intoBytes(124)))
    assert.deepEqual(again.entries(), held.entries())
    ```

### A currency pair is read off the symbol

A venue quoting a currency pair routinely states nothing but its symbol - `55=EUR/USD` and a price, with no `SecurityType(167)`, no `Product(460)`, no classification and no currency. The symbol already says all of them, so a parse reads it through [`FxSymbol::from_symbol`](../types/codes/forex.md#a-symbol-names-a-pair-and-a-tenor), the one detector, before the rules run. A pair it names enters the message's derived security identifiers under [`FOREX`](../types/codes/forex.md#the-forex-security-identifier), so `forexcode` answers it, a stated identifier replaces it and a changed symbol derives it again; a pair trades on no one market, so `miccode` is `XXXX` where nothing names one. Detection then fills, only where the cell is absent or detection wrote it before:

| Cell | Filled with |
| --- | --- |
| `SecurityType(167)` | `FXSPOT` for a spot or unstated tenor, `FXFWD` for a forward; nothing for a metal |
| `Product(460)` | `4`, a metal `2` |
| `CFICode(461)` | `IFXXXP` for `FXSPOT`, `JFTXFP` for `FXFWD`, `ITKXXX` for a metal; also where the stated code is all `X` |
| `Currency(15)` | the base leg; a stated currency that is neither leg leaves both currencies alone |
| `SettlCurrency(120)` | the other leg, where `Currency(15)` is one |
| `SettlType(63)` | what the symbol's suffix spells, where neither it nor `SettlDate(64)` is stated |

A symbol naming no pair detects nothing, and so does a message stating a class that is not foreign exchange - a `SecurityType(167)` outside the FX codes (`FOR`, `FXNDF`, `FXSPOT`, `FXFWD`, `FXSWAP`, `FXNDS`, `FXBN`, `FXDN`), a `Product(460)` other than `4` (or `2` for a metal), a `CFICode(461)` neither all `X` nor in an FX group; a later write that makes it so takes back what detection derived. A row stating its `forexcode` is the row's word, and detection is skipped. Each registry remembers what every distinct symbol read as, pair and no pair alike, for 4,096 symbols - past that the answer is still given and not remembered - so every codec and door reading one registry reads a symbol once. A detected pair is a derived identifier, stated nowhere; a `FOREX` identifier a row or a bridge key states is stated.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Market;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // A spot order stating nothing but its symbol.
    let order = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=FX1|55=EURUSD|54=1|38=1000000|10=0|")?;
    assert_eq!(order.get_securityids().get("FOREX"), Some("EUR/USD"));
    assert_eq!(order.get_miccode().map(|mic| mic.as_str()), Some("XXXX"));
    // What the pair implies is written into the row.
    assert_eq!(order.by_tag(167)?.as_str(), Some("FXSPOT"));
    assert_eq!(order.by_tag(15)?.as_str(), Some("EUR"));
    assert_eq!(order.by_tag(120)?.as_str(), Some("USD"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A spot order stating nothing but its symbol.
    order = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=FX1|55=EURUSD|54=1|38=1000000|10=0|")
    assert order.securityids["FOREX"] == "EUR/USD"
    assert order.miccode.as_py() == "XXXX"
    # What the pair implies is written into the row.
    assert order.by_tag(167).as_py() == "FXSPOT"
    assert order.by_tag(15).as_py() == "EUR"
    assert order.by_tag(120).as_py() == "USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A spot order stating nothing but its symbol.
    const order = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=FX1|55=EURUSD|54=1|38=1000000|10=0|'))
    assert.equal(order.securityids.FOREX, 'EUR/USD')
    assert.equal(order.miccode, 'XXXX')
    // What the pair implies is written into the row.
    assert.equal(order.byTag(167).asJs(), 'FXSPOT')
    assert.equal(order.byTag(15).asJs(), 'EUR')
    assert.equal(order.byTag(120).asJs(), 'USD')
    ```

### The identifier maps are read off the row

The [alternate identifiers](message.md#the-identifier-maps) `Operation` answers - `get_altids()` - are rebuilt at every settle from the fields that state them: `OrderID(37)`, `SecondaryOrderID(198)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `ExecID(17)`, `TrdMatchID(880)`, `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)` and `TradeID(1003)`, and a bridge's own. Each entry sits under the upper-cased name of the field that stated it, and no column carries them: the fields do, so the fill changes neither the entries, the emitted bytes nor the digest, and a write to the map is a write to its field. It is what the [lifecycle](lifecycle.md#a-chain-is-named-by-its-cross-code) joins a chain by when a message spells no code the live one shares. The account a message is booked to, `Account(1)`, names no identifier: it is the message's `ACCOUNT` [account](message.md#accounts-and-regulatory-trade-identifiers), which a leaf holds in its `accountids` rather than its metadata. The user who entered it, `SenderSubID(50)`, names none either.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Operation};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let wire = b"8=FIX.4.4|35=8|1=ACCT-7|50=TRD1|37=O-1|11=C-1|17=E-1|10=0|";
    let mut filled = codec.parse_fix_line(wire)?;

    // The three alternate identifiers the execution report states, each
    // under the upper-cased name of the field that stated it, in key order;
    // the account and the sender's sub-identifier are none of them.
    let names: Vec<(&str, &str)> = filled.get_altids().iter().collect();
    assert_eq!(names, [("CLORDID", "C-1"), ("EXECID", "E-1"), ("ORDERID", "O-1")]);
    // The chain is named by the strongest identifier the message spells.
    assert_eq!(filled.get_crosscode(), "O-1");

    // The map is a view of the row: a write lands on the field the key is
    // read from, so the row and the wire carry the pair, and a held key
    // is filled by nothing.
    assert!(filled.insert_altid("TRDMATCHID", "M-1")?);
    assert_eq!(filled.by_tag(880)?.as_str(), Some("M-1"));
    assert!(!filled.insert_altid("ORDERID", "O-2")?);
    assert_eq!(filled.get_altids().get("ORDERID"), Some("O-1"));
    // A key no field of the dictionary states cannot be written, and the
    // refusal names it.
    assert!(filled.insert_altid("VENUEREF", "X").is_err());

    // A message stating none goes by no name.
    let heartbeat = codec.parse_fix_line(b"8=FIX.4.4|35=0|10=0|")?;
    assert!(heartbeat.get_altids().is_empty());
    ```

### A composed key fills the field its last segment names

A bridge writes a field under its own namespace, so one row carries `TECH.CLIENTID`, `ULLINK.INSTRUMENTID`, `FIRM.ORIG.ULFROMSESSIONNAME` and `OMSVENDOR.CALC.EXECBROKER` beside its plain keys. The fact is the field's however the writer spelled the key: where the last dotted segment of a key resolves to a dictionary field and that field is absent, the composed key fills it, and the filled child takes the field's tag, so every rule after it reads the value like any other.

The key itself is the bridge's own statement and not a FIX field, so it lands in the [`metadata`](#the-crates-own-columns) Map - tag 65030, one sorted `map<utf8, utf8>` under the key as the bridge spelled it, folded - rather than as a child of the content row: `metadata()` answers them, `by_tag(METADATA_TAG_NAME.0)` is the map, and one column of the fixed row carries it. A key whose last segment names no field of this dictionary is in the map and nowhere else.

One voice or silence. Where a row names one absent field under several composed keys and they do not agree, none of them fills it. That is not a precaution: on nine lines of the committed corpus `FIRM.ORIG.CLIENTID` is `2540498.003` and `ULLINK.CLIENTID` is `trader1` with `CLIENTID` absent - a firm account number and a trader login, and nothing in the row says which one the field means. Filling from either would invent a fact; filling from neither states what the row actually settled, which is nothing.

Never overwriting is load-bearing here too, and the same corpus proves it: `CLIENT.SYMBOL` is `XAU` where `SYMBOL` is `XAU/USD`, and `OMSVENDOR.CALC.EXECBROKER` is `SWXCCP` where `EXECBROKER` is `1743045.007`. A namespace's spelling of a fact is not the fact.

The cancel reject the corpus ends on shows the fill and its bound side by side: `OMSVENDOR.ORDERQTY=10000` fills the `OrderQty` the row never states and `OMSVENDOR.TIMEINFORCE=day` its `TimeInForce`, while `FIRM.ACRONYM`, `ULLINK.INSTRUMENTID` and `ULLINK.BYPASSRISK` name no field of the dictionary; every one of the seven keys is in the message's metadata under its own dotted spelling, read back as `metadata()["firm.acronym"]` and never as `acronym`.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let read = |row: &[u8]| -> yggdryl::Result<FixMsg> {
        reader.parse_line(row)?.next().expect("one row")
    };

    // The namespace is the writer's; the fact is the field's, and the key
    // the bridge wrote is the message's metadata.
    let filled = read(b"MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|FIRM.ACRONYM=XYZ|")?;
    assert_eq!(filled.by_name("Account")?.as_str(), Some("ACCT-000117"));
    assert_eq!(filled.metadata()["tech.account"], "ACCT-000117");
    assert_eq!(filled.metadata()["firm.acronym"], "XYZ", "no field, so the map alone");
    assert_eq!(filled.get_by_name("firm.acronym"), None);

    // Two namespaces naming one absent field, disagreeing: nothing fills it,
    // and both statements are still in the metadata.
    let split = read(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=2540498.003|ULLINK.CLIENTID=trader1|")?;
    assert_eq!(split.get_by_name("ClientID"), None);
    assert_eq!(split.metadata().len(), 2);

    // Agreeing, they fill; and a namespace never lands over a stated value.
    let agreed = read(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|")?;
    assert_eq!(agreed.by_name("ClientID")?.as_str(), Some("trader1"));
    let stated = read(b"MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|")?;
    assert_eq!(stated.by_name("Symbol")?.as_str(), Some("XAU/USD"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    def read(row):
        held, = reader.parse_line(row)
        return held

    # The namespace is the writer's; the fact is the field's, and the key the
    # bridge wrote is the message's metadata.
    filled = read(b"MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|FIRM.ACRONYM=XYZ|")
    assert filled.by_name("Account").as_py() == "ACCT-000117"
    assert filled.metadata["tech.account"] == "ACCT-000117"
    assert filled.metadata["firm.acronym"] == "XYZ", "no field, so the map alone"
    assert filled.get_by_name("firm.acronym") is None

    # Two namespaces naming one absent field, disagreeing: nothing fills it, and
    # both statements are still in the metadata.
    split = read(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=2540498.003|ULLINK.CLIENTID=trader1|")
    assert split.get_by_name("ClientID") is None
    assert len(split.metadata) == 2

    # Agreeing, they fill; and a namespace never lands over a stated value.
    agreed = read(b"MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|")
    assert agreed.by_name("ClientID").as_py() == "trader1"
    stated = read(b"MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|")
    assert stated.by_name("Symbol").as_py() == "XAU/USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const read = (row) => reader.parseLine(Buffer.from(row)).next().value

    // The namespace is the writer's; the fact is the field's, and the key the
    // bridge wrote is the message's metadata.
    const filled = read('MSGTYPE=8|TECH.ACCOUNT=ACCT-000117|SIDE=1|FIRM.ACRONYM=XYZ|')
    assert.equal(filled.byName('Account').asJs(), 'ACCT-000117')
    assert.equal(filled.metadata['tech.account'], 'ACCT-000117')
    assert.equal(filled.metadata['firm.acronym'], 'XYZ', 'no field, so the map alone')
    assert.equal(filled.getByName('firm.acronym'), null)

    // Two namespaces naming one absent field, disagreeing: nothing fills it, and
    // both statements are still in the metadata.
    const split = read('MSGTYPE=8|FIRM.ORIG.CLIENTID=2540498.003|ULLINK.CLIENTID=trader1|')
    assert.equal(split.getByName('ClientID'), null)
    assert.equal(Object.keys(split.metadata).length, 2)

    // Agreeing, they fill; and a namespace never lands over a stated value.
    const agreed = read('MSGTYPE=8|FIRM.ORIG.CLIENTID=trader1|ULLINK.CLIENTID=trader1|')
    assert.equal(agreed.byName('ClientID').asJs(), 'trader1')
    const stated = read('MSGTYPE=8|CLIENT.SYMBOL=XAU|SYMBOL=XAU/USD|')
    assert.equal(stated.byName('Symbol').asJs(), 'XAU/USD')
    ```

### Edges

- A value no standard closes is silence, not a failure: `SecurityID` under source `4` spelling `XX0000000001`, whose check digit does not close it, is no ISIN - the trait answers none - and nothing downstream reads a country off it.
- A value that would not type - `201=abc` in the `PutOrCall` column - is a null the row holds while the entry keeps the text; a rule fills the null in place, so the row has one column for the tag and the entry still says `abc`.
- An absent input is silence: a report stating `LastQty(32)` and no `LastPx(31)` derives no `GrossTradeAmt(381)`, and no rule guesses.
- The rules read codes, and a venue's own word for one is not the code. A bridge row spelling `SECURITYIDSOURCE=isin` beside a `SECURITYID` the check digit closes has stated a source, which stands, and `isin` is not `4` - the dictionary names that code `ISINNumber`, so nothing translated it - so the normalized ISIN stays null; the same row spelling `SECURITYTYPE=equity` names no code of the `SecurityType` set, so no group files it and no CFI is read off it. Direct bridge keys are different: `ISINCODE` resolves the crate's normalized `isincode`, while `CFICODE` resolves FIX's standard `CFICode(461)`; neither becomes unknown metadata.
- A trade stating no quantities states no status: `150=F` alone leaves `OrdStatus` absent, and `get_state` then answers what the report said happened, `F` as the code set spells it, `TRADE`.
- An option stating no `PutOrCall` gets a CFI whose exercise is `X`, and `PutOrCall` is not then read back off it: `X` is the code for an exercise left open.
- A registry built from a handful of fields still fills: the rules and the market traits read the standard's fields by tag and by name, and a rule whose target field the registry lacks fills nothing there rather than refusing.
- A `SecurityID` under source `4` that no check digit closes falls through to the alternate identifier whose source is `4`: `22=4|48=NOTANISIN00` beside `455=CH0012221716|456=4` answers the `ISIN` of `get_securityids()` with the alternate and fills `CountryOfIssue` with `CH`; a primary the digit closes answers itself whatever the alternate says; a message neither closes answers nothing.
- `CountryOfIssue` answers for exactly the 249 prefixes `StringEnum::COUNTRIES` lists, and for no other of the 676 pairs: `AB`, `UK`, `TP` and `XS` are silence, `GB` and `US` answer.
- A report with nothing left that states a canceled quantity ordered what it did plus what it canceled: `39=4|14=40|84=60` derives `OrderQty` 100 and `LeavesQty` 0, and `151=0` stated beside them changes nothing - Appendix D's cancel, where the deleted hand-laid order answered `CumQty` alone. A working report stating what is left answers done plus left, whatever it canceled.
- The fixpoint fills what one hand-laid pass could not: `32=10|194=1.25|195=0.25` on a report derives `LastPx` 1.5 and then `GrossTradeAmt` 15, and `AvgPx` where `CumQty` is the fill; `150=0|14=0|84=100` derives `OrderQty` 100 and then `LeavesQty` 100; `32=10|2353=2|231=5|31=3` derives `TotalTradeQty` 20 and then the multiplied and the gross quantities off it.
- A rule reads a field as what it is: `PossDupFlag(43)` is a boolean, so `43=Y` fills `OrigSendingTime` from `SendingTime`, and `TradingUnitPeriodMultiplier(2353)` is an integer, so `32=10|2353=2` fills `TotalTradeQty` 20.
- A code is compared exactly, because FIX codes are case-sensitive: `22=A` is Bloomberg's source and fills `Symbol` from `SecurityID`, and `22=a` is no code of the set and fills nothing.

## A column is filled by the tag its field carries

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [lookup](registry.md) would be a hash and a verification. None of that runs per row: the schema is fixed, its columns are named `msgtype` and `symbol`, each carries its field's `FIX:tag`, and `into_row` fills each one by that tag. `fix_column_tags` reads the tags off a schema once, so a batch of a million rows reads them once rather than once per row; a caller-declared root that spells a column by its tag's digits is read the same way, the digits answering where the field carries no tag.

So there is nothing beside the schema to build, hold, or invalidate. A caller finds a column with `index_of` on the schema it already has - or with `fix_column_of` and the tag - and two captures sharing a dictionary share both the schema and every position in it. A Serie group column is filled by its `FIX:counter`, while the numeric count stays in its own column; the self-counting `metadata` Map occupies only its own column.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `into_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

Map groups use the same row and Arrow doors, preserving key/value fields, non-null keys and `keys_sorted`; a null map, an empty map and a map with a null value remain distinct.

## A capture's own columns lead the row

A line's URL, line number, timestamp and other capture fields lead its FIX columns. A source row produces one output row per message - one per frame a line carried, one for a JSON document - and each of them receives the same carried values from that row; a row that carried no message produces none.

A carried column whose folded name a FIX column already takes - a `MsgCtxId` capture beside `msgctxid`, a text reader's `msgtype` beside the FIX one - is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema. What it stated is not lost. A clashing column whose FIX field is fillable [fills it](arrow.md#a-column-is-the-caller-speaking-per-row) - a `MsgCtxId` capture does. A column naming `sourceurl` clashes with nothing, because no column of the fixed row takes that name: it leads the row like any other of the capture's own, is carried by every message read out of the row, and is stated again at that column by `into_row`. A `msgdirection` column is the row's stated direction, read as a parameter and carried nowhere else. The fifteen [event columns](../graph/market-data.md#columns) a [text line's batch](../media/index.md#plain-text) opens with are the line's own facts and fill no field: the carrier's `curruuid` is each message's one source, its `srcuuids`, exactly as the line door states it, its `currunix` is the line's clock - each message's `recdunix`, and the [sending clock](#every-message-is-dated) of one stating no `SendingTime(52)` - and the other thirteen say nothing about the message; all are dropped as the row's own twins take their names.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, FixRegistry, StructType, fix_schema, fix_schema_carrying};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;

    let capture = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])?)
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
- A column whose field carries neither a `FIX:tag` nor a `FIX:counter`, and whose name spells no tag, is the capture's own, so `into_row` answers null there; whoever read the capture states it. `sourceurl` carries a `FIX:tag` and answers null just the same, because no holder answers it either.
- A column whose name holds a `.` is a bridge's own statement rather than a capture's, so `from_row` carries it into the message's [metadata](message.md#typed-tags) as a parsed line's namespaced key goes there.
- `SendingTime(52)` and `TransactTime(60)` are typed by the registry's own declarations - `FixRegistry::new` seeds both where a dictionary defines neither - and a declaration that is not a nanosecond UTC instant is a defect of the registry, refused at intake; a stated clock that does not read as an instant is left unstated beside an anomaly and a [warning](#warnings), and the message is dated as one stating none is. `TrdRegTimestamp(769)` is seeded by nothing and is the dictionary's to type, so a registry that leaves it untyped reads no regulatory clock and the message is dated without one.
- A column of the crate's own is typed by the crate's definition, on a tag from 65001 that no dictionary publishes: `currunix` is an instant, `currhashcode` a `uint64`, `curruuid` a `uuid`, `msgcat` a `marketdatakind`, `state` a `state`, `forexcode` a `forex`, `sourceurl` a `url`, whatever text a venue spelled them in - and a FIX column is typed by the dictionary, so `SecurityExchange(207)` is a `mic` and `Price(44)` a `decimal128(38, 18)`.
- A capture's own `timestamp` is context, not a FIX clock: it leads the row as a carried column, dates nothing, and never overrides the message's own [reading](#the-official-clock-dates-the-message) of `currunix`. The line's own clock - an `mtime` capture, else its handle's time - is the one the message reads: its `recdunix`, and the [sending clock](#every-message-is-dated) of a message stating no `SendingTime(52)`.
- `metadata` is the message's own fact, filled as it is built: a message no bridge namespaced leaves the column null. The alternate identifiers are no column: a row states the fields they are read from, and a message read back out of a row answers them again.
- Typed text drops the replacement character and every control character but tab, so a byte a transport mangled does not become a mangled column; the entry keeps the bytes exactly as they arrived.
- `index_of` on a column the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- A parse drops no row: `FixDedup`, Rust only, drops a message whose `digest` equals the one before it - an adjacent republication - over any message stream, and is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) the caller composes; the lifecycle, a stage too, [yields each identity once](lifecycle.md#an-identity-is-yielded-once) within the codec's `dedup_window_ms`.

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
    python/.venv/bin/python -m pytest python/tests/test_fix.py
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k "reader or row or crate"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    node scripts/build_docs_fix.js --check
    ```
