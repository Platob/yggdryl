# Capture

A day of session log is a table. This page is the road from one to the other: [`FixCodec`](#a-reader-is-the-whole-parse-surface) turns a captured line into the [messages](message.md) it holds, [`fix_schema`](#the-columns-are-the-folded-names) is the one row shape every message answers as, and [`FixMsg::into_row`](#a-column-is-filled-by-the-tag-its-field-carries) fills it - by the tag each column's field carries, so nothing is resolved against the [dictionary](registry.md) per row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec`, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `fix_column_of`, `fix_column_tags`, `FixMsg::into_row`, `fix_crate_fields` |
| Columns | named by the field's folded canonical name - `msgtype`, never `35` and never `msg_type`; the display spelling stays on the field's `display`, the tag on its `fix:tag`, and a named group column's counter on its `fix:counter` |
| Shape | standard header, the fields a consumer reads, three groups, the trailer, this crate's own twenty, `msgdirection`, then `nofixentries` and `nounmappedfixentries` |
| Non-null | `beginstring`, `msghash`, `timestamp`, `unixpartition` - every built message [fills them](#every-message-is-dated-and-versioned); every other column is nullable |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `nofixentries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Expansion | ordinary lines yield one message; a bulk configuration yields one per selected MBean, including every response; empty bulk or wildcard answers yield none |
| Refuses | nothing a row's content can do; a line the reader cannot read is a message with nothing in it, and the row count still matches the capture's |
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
    let mut messages = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|", false)?;
    let order = messages.next().expect("one frame")?;
    assert!(messages.next().is_none());

    let row = order.into_row(&schema)?;
    let held = row.as_sequence().expect("a row");
    let at = schema.index_of("msgtype").expect("the msgtype column");
    assert_eq!(held[at].as_str(), Some("D"));

    // A tag no dictionary explains is still in the row, in its own column.
    let unmapped = held.last().and_then(Scalar::as_sequence).expect("the list");
    assert_eq!(unmapped.len(), 1);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    schema = fix_schema(registry, "FixMessage")
    reader = FixCodec(registry)

    order, = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")
    row = order.into_row(schema).as_py()

    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("symbol")] == "AAPL"
    # A tag no dictionary explains is still in the row, in its own column.
    assert len(row[-1]) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')
    const reader = new fix.FixCodec(registry)

    const [order] = reader.transformLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|'))
    const row = order.intoRow(schema).toJSON()

    assert.equal(row[schema.indexOf('msgtype')], 'D')
    assert.equal(row[schema.indexOf('symbol')], 'AAPL')
    // A tag no dictionary explains is still in the row, in its own column.
    assert.equal(row[row.length - 1].length, 1)
    ```

## Try it

The [Decode](decode.md) page displays native results for the committed frame corpus. Run the examples here to parse other input.

## Find a column

More columns than anyone scrolls, and the question a reader actually has is *which column holds this*. The filter matches the tag, the field name and the wording alike.

<div class="ygg-fx" data-fix="row" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## A reader is the whole parse surface

`transform_line` accepts captured bytes and returns `FixMessages`, a lazy,
fallible iterator. `transform_record` uses the payload and options carried by a
record. `transform_ulconfig_line` splits a bulk or wildcard configuration body.
The specialized `transform_fix_line`, `transform_fixml_line`,
`transform_ullink_line` and `transform_pairs` return one message.

Python exposes native iterators; JavaScript uses `IterableIterator<FixMsg>`.
Errors propagate from the native cursor and fuse it. Schema construction and
group-plan resolution happen before repeated values are processed.

Every one of them ends in the same builder, so a document is typed by the rules that type a frame — one nesting builder, one fold, one code translation, one value contract.

Each is the core's own method under the same name in all three languages.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    // value packs its members behind the two control bytes ULLINK uses.
    let bridge: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|";
    let held = reader.transform_line(bridge, false)?.next().expect("one frame")?;

    assert_eq!(held.by_tag(55)?.as_str(), Some("TTF"));
    assert_eq!(held.by_tag(44)?.as_f64(), Some(41.25));
    // The packed members became three real fields under one nesting.
    let party = held.party("1").expect("the buy-side party");
    assert_eq!(party.id().and_then(yggdryl::Scalar::as_str), Some("BUYSIDE"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    # value packs its members behind the two control bytes ULLINK uses.
    held, = reader.transform_line(
        b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )

    assert held.by_tag(55).as_py() == "TTF"
    assert held.by_tag(44).as_py() == 41.25
    # The packed members became three real fields under one nesting.
    party = held.party("1")
    assert party is not None and party[0].as_py() == "BUYSIDE"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    // value packs its members behind the two control bytes ULLINK uses.
    const bridge = Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    )
    const [held] = reader.transformLine(bridge)

    assert.equal(held.byTag(55).toJSON(), 'TTF')
    assert.equal(held.byTag(44).toJSON(), 41.25)
    // The packed members became three real fields under one nesting.
    assert.equal(held.party('1')[0].toJSON(), 'BUYSIDE')
    ```

### A printed separator is still the separator

A capture that cannot print `0x01` writes what it stands for: `^A`, `\x01`, `<SOH>` or `{SOH}`. The escape happened on the way into the log rather than on the wire, so a numeric frame spelling its separator that way is unescaped once, before it is split, and reaches the same columns the byte itself reaches.

One vocabulary serves both readings, because the spelling a frame is *located* with and the one it is *split* on are the same fact: a capture that escapes its separator is recognized once rather than in each place that reads a frame. A frame carrying a real `0x01` is never scanned for the escapes, so the ordinary path allocates nothing.

### A packed occurrence uses explicit or declared boundaries

ULLINK packs one group occurrence's members behind EOT and ETX (`\x04\x03`); a bridge relaying into a FIX session packs them behind the protocol's own SOH. Both split an occurrence, because inside one neither byte can be part of a value.

The first spelling an occurrence actually carries is the one that splits it, and only that one: reading both at once would let a value that legitimately holds the other byte break into fields nobody wrote.

When neither control spelling is present, the reader scans for the next direct member name declared by the addressed group. The longest declared name wins, so `PartyIDSource` is not shortened to `PartyID`; names from elsewhere in the registry never become nested boundaries. A run with no declared boundary stays whole.

### Edges

- Ordinary unframed text can produce an empty message. Malformed configuration input returns a located error; a fallible message iterator stops after that error.
- A bridge key's `#` drops only where it is the row's sole spelling of that key. `ORDERID=123|#ORDERID=345` states two keys, so there `#ORDERID` stays verbatim beside the dictionary's `OrderID` - its own column, its own entry - rather than two values merging under one name. The twin is matched by the fold every key resolves under, so `OrderId` and `ORDER_ID` keep it too; a bare pair whose value is a stated absence is no twin, because a key that said nothing was sent is not a key that was sent.
- A stated absence - one of `null_values` - produces no field and no entry, because a key that said nothing was sent is not a key that was sent.
- A pinned `version` decides which code spelling a value translates through, never what a field is called: a tag is one column under the name the dictionary holds it by, and what each version called it stays readable through the field's lineage.

## The columns are the folded names

`msgtype`, never `35` and never `msg_type`. A column is spelled the way the dictionary spells the field's canonical name - ASCII case folded once, on the way in - so a row reads the way a message reads, in every binding and every catalog, and a reader spelling `row["msgseqnum"]` finds the sequence number without a dictionary in hand.

The tag is still the identity. Each column carries its field's `fix:tag`, its `display`, its lineage and its code set, and the row is [filled by that tag](#a-column-is-filled-by-the-tag-its-field-carries) rather than by the spelling, so a venue that renames a field between versions changes nothing about where its value lands. `fix_schema_tags` is the same row as tags, in the same order.

A named group column carries `fix:counter` beside the `fix:tag` its definition derives from its own name: the numeric count keeps its own column, and the group column beside it holds the occurrences as a List.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixRegistry, fix_column_of, fix_schema, fix_schema_tags};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    let schema = fix_schema(&registry, "FixMessage")?;

    let columns: Vec<&str> = schema.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(&columns[..3], ["beginstring", "bodylength", "msgtype"]);
    assert_eq!(&columns[columns.len() - 2..], ["nofixentries", "nounmappedfixentries"]);
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
    assert columns[-2:] == ["nofixentries", "nounmappedfixentries"]
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
    assert.equal(schema.fieldAt(schema.fieldLen - 2).name, 'nofixentries')
    assert.deepEqual(fix.schemaTags().slice(0, 3), [8, 9, 35])

    // The spelling stays on the field, so a renderer shows `MsgType` over `msgtype`.
    assert.equal(schema.field('msgtype').display, 'MsgType')
    assert.equal(schema.field('msgtype').fix.tag, 35)
    ```

## Nothing is lost at the end

Two lists close every row.

`nofixentries` is the whole arrival record: every pair, in arrival order, untranslated, and a group's members riding under the counter pair that heads them. It is a list of `fixentry` structs. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

`nounmappedfixentries` is a **view** over that record rather than the rest of it: the pairs no dictionary explained, discovered pre-order at any depth and flattened to one level, in the order they arrived. It holds nothing `nofixentries` does not, and it exists so a venue onboarding a new field finds it by reading one column instead of filtering a million rows. On a well-known dialect it is empty on every row and costs a validity bit.

## The crate's own columns

Twenty fields carry what a capture states, or what a message implies, that no dictionary publishes. Each is an ordinary standard field with a tag from 65000 up - a block no venue claims, above the user-defined range a dialect may take - so it lifts, columns, serializes and resolves with no special case anywhere, and a bridge row spelling `SESSIONID` or `ULFROMSESSIONNAME` reaches it by name like any other field. Every registry holds them from construction: `FixRegistry::new()` inserts them before anything else, so a dictionary loaded from a [store](store.md), built from fields or left empty answers `timestamp` and `sendersessionid` alike, and the store never writes them. `fix_crate_fields` is the listing, in tag order; `SENDERSESSIONID_TAG` and its siblings name the tags, `CRATE_TAG_MIN` the first of them, `is_crate_tag` whether a tag is one, and `TIMESTAMP_NAME` the column the clock takes.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `msghash` | `MsgHash` | 65000 | the xxh3-128 digest of what the message said: the arrival record with the standard header and trailer left out, `MsgType` excepted |
| `version` | `Version` | 65001 | the FIX version it was *read* at, which is not always what `BeginString` claimed |
| `symbolticker` | `SymbolTicker` | 65002 | one instrument symbol that is the same across venues |
| `timestamp` | `Timestamp` | 65003 | the clock a capture is ordered by: the row's own, else the message's, else the epoch |
| `unixpartition` | `UnixPartition` | 65004 | the partition that clock falls in, as whole seconds |
| `parentclordid` | `ParentClOrdID` | 65005 | the client order identifier this order descends from |
| `parentorderid` | `ParentOrderID` | 65006 | the venue order identifier this order descends from |
| `sendersessionid` | `SenderSessionId` | 65007 | the session the message came from, as the message itself states it; a bridge row's own `SESSIONID` reaches it by alias - never the bracket a log writes in front of a line |
| `msgctxid` | `MsgCtxId` | 65008 | the message context a bridge handled the message in, from its log's bracket |
| `senderpluginid` | `SenderPluginId` | 65009 | the plugin the message came from inside a bridge, as the message states it |
| `targetpluginid` | `TargetPluginId` | 65010 | the plugin the message went to inside a bridge, as the message states it |
| `sendersessionname` | `SenderSessionName` | 65011 | the name of the session the message came from: a bridge row's `ULFROMSESSIONNAME`, else the session that logged a line it sent |
| `targetsessionname` | `TargetSessionName` | 65012 | the name of the session the message went to: a bridge row's `ULTOSESSIONNAME`, else the session that logged a line it received |
| `isincode` | `ISINCode` | 65013 | the instrument's ISIN, as an [`isin`](../types/ascii.md): `SecurityID(48)` where `SecurityIDType(22)` says ISIN, else the `SecurityAltID(455)` whose `SecurityAltIDType(456)` does |
| `miccode` | `MICCode` | 65014 | the market the message names, as a `mic`: `SecurityExchange(207)`, else `ExDestination(100)`, else `LastMkt(30)` |
| `state` | `State` | 65015 | the order's state, as a `state`: `OrdStatus(39)`, else `ExecType(150)` |
| `instid` | `InstId` | 65016 | the instrument, the same across venues that spell it alike: the xxh128 digest of its market, classification, ISIN - else symbol - and currency |
| `id` | `Id` | 65017 | the message: the instant closest to the market impact, then the xxh3 digest of what it said, so ids sort by time and never repeat |
| `persistentid` | `PersistentId` | 65018 | the order chain: the instant it was created, then the xxh3 digest of its instrument and first identifier, the same on every later message that shares one of its identifiers |
| `targetsessionid` | `TargetSessionId` | 65019 | the session the message went to, as the message itself states it |

The envelope `msghash` drops is the standard header and the standard trailer whole, read from the same two tag lists the row shape is ordered by, so a tag either component gains leaves the digest without a second listing learning about it. `MsgType` is the one exception and stays in: a message type is what a message *is* rather than how it travelled, so an order and a report carrying the same tags are not one message. The consequence is the point - two identical orders sent a second apart hash equal, and so do the same order relayed through two sessions or replayed on a resend.

The nine after `targetpluginid` are read from two places. A bridge row states its own session and plugin sessions in `SESSIONID`, `ULFROMSESSIONNAME` and `ULTOSESSIONNAME`, which reach the fields by name; and a bridge's log states the message context in the bracket after its clock, and the plugin that wrote the line in front of it, which a [row header](arrow.md#a-bridge-log-names-what-it-fills) captures and the row fills - the plugin landing on the sender's session for a line the plugin sent and on the target's for one it received, never over a value the row stated itself. The three after them are derived when a message becomes a row, from the tags the table names, so a monitor filters an instrument, a market or a lifecycle without knowing which of several tags a venue put it in. The last three are stamped by the [lifecycle](lifecycle.md), which reads a stream in order and is the one place a message learns which order it belongs to. FIX publishes the counterparties in `SenderCompID` and `TargetCompID`; the plugin that carried a message inside a bridge is a fact about the bridge, and one FIX never states.

Two of them declare more than a type, in the protocols the crate already has rather than in a spelling only a FIX reader would know to look for. `msghash` is a digest holder, so it says which algorithm filled it and what it read. `unixpartition` is a derived partition column, so it says which column it derives from and how.

=== "Rust"

    ```rust
    use yggdryl::fix_crate_fields;

    let fields = fix_crate_fields()?;
    let digest = &fields[0];
    assert_eq!(digest.name(), "msghash");
    assert_eq!(digest.display(), Some("MsgHash"));
    assert!(digest.as_digest().is_holder());
    assert_eq!(digest.as_digest().sources()?, Some(vec!["nofixentries".to_owned()]));

    let partition = &fields[4];
    assert_eq!(partition.name(), "unixpartition");
    assert_eq!(partition.display(), Some("UnixPartition"));
    // Which column it derives from, and how: `truncate[3600]`, because the
    // value is seconds floored to a multiple of the width.
    assert_eq!(partition.as_partition().sources()?, Some(vec!["timestamp".to_owned()]));
    assert_eq!(partition.get_metadata("iceberg:transform"), Some("truncate[3600]"));
    ```

=== "Python"

    ```python
    from yggdryl.fix import fix_crate_fields

    fields = {field.name: field for field in fix_crate_fields()}
    digest = fields["msghash"]
    assert digest.metadata["display"] == "MsgHash"
    assert digest.metadata["digest:role"] == "holder"
    assert digest.metadata["digest:algorithm"] == "xxh3-128"
    assert digest.metadata["digest:sources"] == '["nofixentries"]'

    partition = fields["unixpartition"]
    assert partition.metadata["display"] == "UnixPartition"
    # Which column it derives from, and how: `truncate[3600]`, because the
    # value is seconds floored to a multiple of the width.
    assert partition.metadata["partition:sources"] == '["timestamp"]'
    assert partition.metadata["iceberg:transform"] == "truncate[3600]"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fix } = require('yggdryl')

    const held = fix.crateFields()
    const digest = held[0]
    assert.equal(digest.name, 'msghash')
    assert.equal(digest.display, 'MsgHash')
    assert.equal(digest.getProperty('digest', 'role'), 'holder')
    assert.equal(digest.getProperty('digest', 'sources'), '["nofixentries"]')

    const partition = held[4]
    assert.equal(partition.name, 'unixpartition')
    assert.equal(partition.display, 'UnixPartition')
    // Which column it derives from, and how: `truncate[3600]`, because the
    // value is seconds floored to a multiple of the width.
    assert.equal(partition.getProperty('partition', 'sources'), '["timestamp"]')
    assert.equal(partition.getProperty('iceberg', 'transform'), 'truncate[3600]')
    ```

### Every message is dated and versioned

Two of the columns are filled when a message is built, whatever its line carried, and neither becomes an entry unless the wire sent it - so `into_bytes` still re-emits the wire byte for byte.

`beginstring` is the wire's own `BeginString(8)` when stated, else `FIX.<version>` for the version the message was read at: the pinned `version`, else the one `ApplVerID` or `BeginString` implied, else the branch's default, else the dictionary's newest, else 4.4. A bridge row and a configuration document therefore say which FIX they were read as exactly as a frame does, and `FixMsg::version()` always answers for a built message.

`timestamp` closes the message, and is never null. The row's own clock - a `timestamp` column of the record or the batch row, which is what a [row header capture](arrow.md#a-column-is-the-caller-speaking-per-row) becomes - outranks the message's clocks; those are read in decreasing exactness, `TransactTime(60)`, `TrdRegTimestamp(769)`, `SendingTime(52)`, `OrigSendingTime(122)`, a group's from its first occurrence; and a message with neither is stamped with the epoch, `1970-01-01T00:00:00Z`, where a row nobody dated sorts first and visibly rather than among the rows of whatever day it was read on. `market_timestamp()` answers that child, and `unix_partition` floors it to the partition width from its nanoseconds, so a clock stated to the millisecond has a partition.

The root's children are the standard header in its declared order, the body as it arrived, the standard trailer, then `timestamp` last. Four columns are declared non-null because of this - `beginstring`, `msghash`, `timestamp`, `unixpartition` - and every other column is nullable.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, TimeUnit};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(registry).with_version("4.4".parse()?);

    // A frame stating neither its version nor a clock is still dated and versioned.
    let bare = reader.transform_line(b"35=D|55=AAPL|10=0|", false)?.next().expect("one frame")?;
    assert_eq!(bare.by_tag(8)?.as_str(), Some("FIX.4.4"));
    assert_eq!(bare.version().map(|version| version.to_string()), Some("4.4".to_owned()));
    assert_eq!(bare.market_timestamp().temporal_count_at(TimeUnit::Second), Some(0));
    // Neither became an entry, so the wire comes back byte for byte.
    assert_eq!(bare.into_bytes(b'|'), b"35=D|55=AAPL|10=0|");

    // A frame stating both keeps its own, and a sub-second clock has a partition.
    let sent = reader.transform_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|55=AAPL|10=0|", false)?.next().expect("one frame")?;
    assert_eq!(sent.by_tag(8)?.as_str(), Some("FIX.4.2"));
    assert_eq!(sent.market_timestamp().temporal_count_at(TimeUnit::Millisecond), Some(1_787_308_200_415));
    assert_eq!(sent.unix_partition(3_600).as_i64(), Some(1_787_306_400));
    ```

=== "Python"

    ```python
    from datetime import datetime, timezone
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()), version="FIX.4.4")

    # A frame stating neither its version nor a clock is still dated and versioned.
    bare = next(reader.transform_line(b"35=D|55=AAPL|10=0|"))
    assert bare.by_tag(8).as_py() == "FIX.4.4"
    assert bare.market_timestamp().as_py() == datetime(1970, 1, 1, tzinfo=timezone.utc)

    # A frame stating both keeps its own, and a sub-second clock has a partition.
    sent = next(reader.transform_line(b"8=FIX.4.2|35=D|52=20260821-10:30:00.415|55=AAPL|10=0|"))
    assert sent.by_tag(8).as_py() == "FIX.4.2"
    assert sent.market_timestamp().as_py() == datetime(2026, 8, 21, 10, 30, 0, 415000, tzinfo=timezone.utc)
    assert sent.unix_partition(3_600).as_py() == 1_787_306_400
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry, { version: 'FIX.4.4' })

    // A frame stating neither its version nor a clock is still dated and versioned.
    const bare = reader.transformLine(Buffer.from('35=D|55=AAPL|10=0|')).next().value
    assert.equal(bare.byTag(8).toJSON(), 'FIX.4.4')
    assert.ok(bare.marketTimestamp() !== null)
    // Neither became an entry, so the wire comes back byte for byte.
    assert.equal(bare.intoBytes('|'.charCodeAt(0)).toString(), '35=D|55=AAPL|10=0|')

    // A frame stating both keeps its own, and a sub-second clock has a partition.
    const sent = reader.transformLine(Buffer.from('8=FIX.4.2|35=D|52=20260821-10:30:00.415|55=AAPL|10=0|')).next().value
    assert.equal(sent.byTag(8).toJSON(), 'FIX.4.2')
    assert.ok(sent.unixPartition(3600) !== null)
    ```

## What a message implied is filled in

A venue sends what its counterparty needs and nothing more, so a row is routinely missing values the message itself already determines: a report stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill stating `LastQty` and `LastPx` has said what it was worth, and a message naming its instrument by an ISIN has said which country issued it. With enrichment on - the flag every `transform_*` entry point takes, `FixOptions.enrich` for a [batch](arrow.md), `enrich_fixmsg` for a message already built - the reader fills them.

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
| `OrdStatus(39)` | `ExecType(150)`; else `LeavesQty(151)` and `CumQty(14)` on a trade | the values the two code sets spell alike - not `D`, Restated in one and AcceptedForBidding in the other; a trade leaving nothing is filled, `2`, and one leaving something after doing something is partially filled, `1` - each landing as the [`state`](../types/ascii.md) column spells it |
| `state` | `OrdStatus(39)`, `ExecType(150)` | the first stated, as the column is defined |
| `LeavesQty(151)`, `OrderQty(38)`, `CumQty(14)` | the other two, on a report | Appendix D: `OrderQty = CumQty + LeavesQty`, and nothing is left once `OrdStatus(39)` is closed |
| `GrossTradeAmt(381)` | `LastQty(32)` × `LastPx(31)` | Appendix D's execution reports |
| `SettlCurrAmt(119)` | `GrossTradeAmt(381)` × `SettlCurrFxRate(155)` | Appendix O |
| `Currency(15)`, `SettlCurrency(120)` | each other | Appendix O: a trade settling in the currency it was dealt in states it once |
| `AvgPx(6)` | `LastPx(31)` | Appendix D, only where `CumQty(14)` says the whole done quantity is this fill |

The rules run in one order, laid out so every chain ends in one pass: a `SecurityID`'s validation states the source, under which the ISIN column is read; an ISIN found only among the alternate identifiers becomes the `SecurityID`, whose validation states the source in turn; the country is read after either; a security type read off a CFI places the product; a status read off an execution type decides what is left. The primary identifier is read before the alternate ones, as the column is defined, so a message stating an ISIN in both places states it in `SecurityID`.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    let line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|";
    let held = reader.transform_line(line, true)?.next().expect("one frame")?;
    assert_eq!(held.by_tag(22)?.as_str(), Some("4"));
    assert_eq!(held.by_tag(yggdryl::ISINCODE_TAG)?.as_str(), Some("US0378331005"));
    assert_eq!(held.by_tag(470)?.as_str(), Some("US"));
    assert_eq!(held.by_tag(167)?.as_str(), Some("CS"));
    assert_eq!(held.by_tag(460)?, &Scalar::from(5_i32));
    assert_eq!(held.by_tag(yggdryl::MICCODE_TAG)?.as_str(), Some("XNAS"));
    // A trade leaving nothing is filled, as the `state` column spells it.
    assert_eq!(held.by_tag(39)?.as_str(), Some("80FILLED"));
    assert_eq!(held.by_tag(59)?.as_str(), Some("0"), "a day order");

    // Only the row was filled: the wire comes back byte for byte.
    assert_eq!(held.into_bytes(b'|'), line);
    // And a second pass changes nothing.
    assert_eq!(reader.enrich_fixmsg(held.clone())?, held);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    line = b"8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|"
    held = next(reader.transform_line(line, True))
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
    assert reader.enrich_fixmsg(held) == held
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A fill naming its instrument by an ISIN it never sourced, a CFI and a market.
    const line = '8=FIX.4.4|35=8|37=A|48=US0378331005|461=ESVTFR|207=XNAS|150=F|151=0|14=100|10=0|'
    const held = reader.transformLine(Buffer.from(line), true).next().value
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
    assert.ok(reader.enrichFixmsg(held).equals(held))
    ```

### Edges

- A value the column refuses is silence, not a failure: `SecurityID` under source `4` spelling `XX0000000001`, whose check digit does not close it, leaves `isincode` null, and nothing downstream reads a country off it.
- A value that would not type - `201=abc` in the `PutOrCall` column - is a null the row holds while the entry keeps the text; a rule fills the null in place, so the row has one column for the tag and the entry still says `abc`.
- The rules read codes, and a venue's own word for one is not the code. A bridge row spelling `SECURITYIDSOURCE=isin` beside a `SECURITYID` the check digit closes has stated a source, which stands, and `isin` is not `4` - the dictionary names that code `ISINNumber`, so nothing translated it - so the ISIN column is left null; the same row spelling `SECURITYTYPE=equity` names no code of the `SecurityType` set, so no group files it and no CFI is read off it. The bridge's own `ISINCODE` states the column directly, and the `CFICODE` it spells beside it states the product.
- A trade stating no quantities states no status: `150=F` alone leaves `OrdStatus` absent, and `state` then holds what the report said happened, `F` as the column spells it, `40TRADE`.
- An option stating no `PutOrCall` gets a CFI whose exercise is `X`, and `PutOrCall` is not then read back off it: `X` is the code for an exercise left open.

## A bridge configuration is a dictionary of its own

`UlPlugin` is one selected MBean with its attributes and shared source response.
`UlPlugin::from_json_bytes` and `from_json_scalar` validate the input and return
`UlPlugins`. Bulk responses retain array order; wildcard MBeans use canonical
ObjectName order within each response. The cursor retains the source document
and its current position, with no collected output messages.

Each selected configuration converts to one flat `FixMsg`. `SenderCompID`,
`TargetCompID` and `BeginString` retain standard FIX tags; the `ulbridge`
dictionary types the configuration-specific attributes. Register it with
`with_ulbridge_fields` / `withUlbridgeFields`, then select the codec's
`ulbridge` branch when converting configurations directly.

| Source | Flat message |
| --- | --- |
| selected ObjectName | `MBean`, `SessionInterface`, `MBeanType`, `PluginType` |
| request operation | `Operation` |
| response status or error | `Status`, `Error` |
| selected attribute map | typed fields such as `Name`, `CurrentPort`, `NeedReload` |

### One configuration, out of a bulk body and back

The first response below contains two MBeans. The second is a request with no
value. Parsing produces three messages; no response is discarded.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::{FixBranch, FixCodec, FixRegistry, Scalar, UlPlugin};

    let body = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]"#;
    let registry = FixRegistry::new().with_ulbridge_fields()?;
    let codec = FixCodec::new(Arc::new(registry)).with_branch(&FixBranch::from_str("ulbridge")?);
    let mut configurations = UlPlugin::from_json_bytes(body)?;
    let first = configurations.next().expect("first configuration");
    assert_eq!(first.name(), Some("A"));
    assert_eq!(configurations.count(), 2);

    let message = first.into_fixmsg(&codec, false)?;
    assert_eq!(message.by_name("CurrentPort")?, &Scalar::from(7061_i64));
    assert_eq!(UlPlugin::from_fixmsg(&message)?.name(), Some("A"));
    let mut count = 0;
    for message in codec.transform_line(body, false)? {
        message?;
        count += 1;
    }
    assert_eq!(count, 3);
    ```

=== "Python"

    ```python
    from yggdryl.fix import FixCodec, FixRegistry, UlPlugin

    body = b'[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]'
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    codec = FixCodec(registry, branch="ulbridge")
    configurations = UlPlugin.from_json_bytes(body)
    first = next(configurations)
    assert first.name == "A"
    assert sum(1 for _ in configurations) == 2

    message = first.into_fixmsg(codec)
    assert message.by_name("CurrentPort").as_py() == 7061
    assert UlPlugin.from_fixmsg(message).name == "A"
    assert sum(1 for _ in codec.transform_line(body)) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fix } = require('yggdryl')

    const body = Buffer.from('[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]')
    const registry = new fix.FixRegistry()
    registry.withUlbridgeFields()
    const codec = new fix.FixCodec(registry, { branch: 'ulbridge' })
    const configurations = fix.UlPlugin.fromJsonBytes(body)[Symbol.iterator]()
    const first = configurations.next().value
    assert.equal(first.name, 'A')
    assert.equal([...configurations].length, 2)

    const message = first.intoFixmsg(codec)
    assert.equal(message.byName('CurrentPort').asJs(), 7061)
    assert.equal(fix.UlPlugin.fromFixmsg(message).name, 'A')
    assert.equal([...codec.transformLine(body)].length, 3)
    ```

### Edges

- An empty bulk array or empty wildcard response yields zero configurations.
- A request-only document or an error response without a value yields one
  envelope message. Status and error belong to each selected response.
- Null attributes are absent. Numeric sentinels such as `-1` remain numbers.
- The original response may contain sibling MBeans; equality and hashing of a
  selected `UlPlugin` depend on that configuration and its exchange metadata,
  not unrelated sibling attributes.
- A malformed bulk member reports its location. Conversion errors stop and fuse
  `FixMessages`; the iterator does not skip a failed message and continue.
- [Arrow parsing](arrow.md#one-row-per-message) repeats each source row's carried
  columns for all configurations produced from its body.

## A column is filled by the tag its field carries

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [resolution tiers](registry.md#tiers) would be a hash, a verification and a branch walk. None of that runs per row: the schema is fixed, its columns are named `msgtype` and `symbol`, each carries its field's `fix:tag`, and `into_row` fills each one by that tag. `fix_column_tags` reads the tags off a schema once, so a batch of a million rows reads them once rather than once per row; a caller-declared root that spells a column by its tag's digits is read the same way, the digits answering where the field carries no tag.

So there is nothing beside the schema to build, hold, or invalidate. A caller finds a column with `index_of` on the schema it already has - or with `fix_column_of` and the tag - and two captures sharing a dictionary share both the schema and every position in it. A named group column is filled by the counter its `fix:counter` states, which is read ahead of the tag the group derives from its own name, while the numeric count stays in its own column.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `into_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

That is the rule the whole row keeps: what a message said can never fail the batch it arrives in.

## A capture's own columns lead the row

A line's URL, line number, timestamp and other capture fields lead its FIX columns. A bulk configuration can produce several message rows; each receives the same carried values from its source row.

A carried column whose folded name a FIX column already takes - a `msgCtxId` capture beside `msgctxid`, a text reader's `msgtype` beside the FIX one - is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema. What it stated is not lost, because the row [fills that column from it](arrow.md#a-column-is-the-caller-speaking-per-row). `direction` and `msgdirection` are two names, so both are present.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixRegistry, fix_schema, fix_schema_carrying};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    let capture = DataType::from_fields([
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Binary.required_field("body"),
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
                Field("body", DataType("binary"), nullable=False),
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
        fields.binary('body', { nullable: false }),
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
- A clock a narrow dictionary types as text is still an instant in the derived `timestamp` column: FIX's own spelling is read there, and text that is not a clock leaves the message to its next clock, else the epoch - never a refusal.
- `unixpartition` is floored from the clock's nanoseconds, so a clock stated to the microsecond has a partition rather than a null for not being a whole second.
- A column of the crate's own is typed by the crate's definition, on a standard tag from 65000 that no dialect claims: `timestamp` is an instant, `miccode` a `mic`, `state` a `state`, whatever text a venue spelled them in.
- Typed text drops the replacement character and every control character but tab, so a byte a transport mangled does not become a mangled column; the entry keeps the bytes exactly as they arrived.
- `index_of` on a column the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- [Deduplication](arrow.md#one-row-per-message) removes adjacent duplicate messages after expansion and is off by default.

## Performance

Measured on Windows, AMD Ryzen 5 150 (12 logical CPUs), Rust 1.96 release,
10 Criterion samples with 0.1 s warm-up and 0.2 s measurement. The tracked
registry now loads all four categories; these results do not compare against
the previous field-only seed workload.

| Native case | Central estimate |
| --- | --- |
| `fix/read/bare` | 28.654 µs |
| `fix/read/named` | 32.335 µs |
| `fix/read/grouped` (packed bridge occurrence) | 43.127 µs |
| `fix/read/numeric_grouped` | 31.721 µs |
| `fix/read/numeric_grouped_pinned_branch` | 9.8660 µs |
| `fix/read/ulconfig` | 168.07 µs |
| `fix/read/emit` | 522.60 ns |
| `fix/ulconfig/first/256` (already parsed source) | 344.42 ns |
| `fix/ulconfig/drain/256` (already parsed source) | 112.45 µs |
| `fix/ulconfig/messages/256` | 8.1357 ms |

The pinned-branch case parses `6000=1|6001=42|6002=7|` with `beta` selected while `alpha` and `beta` share those tags and declare different member and tail datatypes. Registry construction and group-plan compilation are outside the timer; this small fixture is separate from the full-seed read cases. The same final run refreshed the `bare` and `numeric_grouped` rows above.

A document costs about six times a frame, and the difference is what it is: a frame is split on a byte and a document is parsed as JSON and walked. A dated read costs what an undated one costs, within a code translation per value: a version decides which spellings answer, and no field is renamed or retyped for it, so there is nothing per row to resolve or cache.

The first-item and full-drain cases distinguish cursor cost from conversion of
every selected configuration. Allocation tests assert zero allocations for
borrowed configuration iteration; each stable hash owns one Xxh3 state buffer.
Parsing bytes and building message values allocate.

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/read --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
cargo bench --locked -p yggdryl --bench fix -- fix/read/numeric_grouped_pinned_branch --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
cargo bench -p yggdryl --bench fix -- fix/ulconfig --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
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
