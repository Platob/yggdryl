# Capture

A day of session log is a table. This page is the road from one to the other: [`FixCodec`](#a-reader-is-the-whole-parse-surface) turns a captured line into a [message](message.md), [`fix_schema`](#the-columns-are-the-tags) is the one row shape every message answers as, and [`FixMsg::to_row`](#a-column-is-the-name-its-tag-spells) fills it - by the tag each column is named for, so nothing is resolved against the [dictionary](registry.md) per row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec`, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `FixMsg::to_row`, `fix_crate_fields` |
| Columns | named by tag as decimal text - `"35"`, never `"msgtype"`; the spelling stays on the field's `display` |
| Shape | standard header, the fields a consumer reads, three groups, the trailer, this crate's derived facts, then `nofixentries` and `nounmappedfixentries` |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `nofixentries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Refuses | nothing a row's content can do; a line the reader cannot read is a message with nothing in it, and the row count still matches the capture's |
| Found | a column is `index_of("35")` on the schema itself - the name is the tag, so nothing is cached, resolved or invalidated |

## Use

One line in, one row out, with the columns named by tag.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    let schema = fix_schema(&registry, "FixMessage")?;
    let reader = FixCodec::new(Arc::clone(&registry));
    let order = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|", false)?;

    let row = order.to_row(&schema)?;
    let held = row.as_sequence().expect("a row");
    let at = schema.index_of("35").expect("the msgtype column");
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

    order = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")
    row = order.to_row(schema).as_py()

    assert row[schema.index_of("35")] == "D"
    assert row[schema.index_of("55")] == "AAPL"
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

    const order = reader.transformLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|'))
    const row = order.toRow(schema).toJSON()

    assert.equal(row[schema.indexOf('35')], 'D')
    assert.equal(row[schema.indexOf('55')], 'AAPL')
    // A tag no dictionary explains is still in the row, in its own column.
    assert.equal(row[row.length - 1].length, 1)
    ```

## Try it

Every shape a capture holds, read by the real package, is on the [Decode](decode.md) page — beside a reader that takes a frame of your own.

## Find a column

Eighty-nine columns is more than anyone scrolls, and the question a reader actually has is *which column holds this*. The filter matches the tag, the field name and the wording alike.

<div class="ygg-fx" data-fix="row" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## A reader is the whole parse surface

Six entry points, because a capture holds six shapes and guessing between them is what a reader exists to avoid. `text` and `bytes` take a captured line whatever it is wrapped in and read the verb in front of the frame; `fixtext` takes a numeric frame whose separator the caller states; `ultext` takes a bridge frame whose keys are names; `ulconfig` takes a [bridge configuration document](registry.md#a-bridge-configuration-is-a-document-that-names-itself); `pairs` takes what a caller already split.

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
    let held = reader.transform_line(bridge, false)?;

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
    held = reader.transform_line(
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
    const held = reader.transformLine(bridge)

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

- A line the reader refuses is not a line lost: it is a message with nothing in it, so the output row count still equals the input line count.
- A bridge key's `#` drops only where it is the row's sole spelling of that key. `ORDERID=123|#ORDERID=345` states two keys, so there `#ORDERID` stays verbatim beside the dictionary's `OrderID` - its own column, its own entry - rather than two values merging under one name. The twin is matched by the fold every key resolves under, so `OrderId` and `ORDER_ID` keep it too; a bare pair whose value is a stated absence is no twin, because a key that said nothing was sent is not a key that was sent.
- A stated absence - one of `null_values` - produces no field and no entry, because a key that said nothing was sent is not a key that was sent.
- A pinned `version` decides which code spelling a value translates through, never what a field is called: a tag is one column under the name the dictionary holds it by, and what each version called it stays readable through the field's lineage.

## The columns are the tags

`35`, not `msgtype`. A tag is the one name a field has in every version and every dialect: tag 32 is `LastShares` in 4.2 and `LastQty` in a newer one, and a column named either of those changes meaning when a venue upgrades. The tag never moves, so the column never does.

The names are still reachable, because the field under each column carries its own `fix:tag`, its `display`, its lineage and its code set.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixRegistry, fix_schema, fix_schema_tags};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    let schema = fix_schema(&registry, "FixMessage")?;

    let columns: Vec<&str> = schema.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(&columns[..3], ["8", "9", "35"]);
    assert_eq!(&columns[columns.len() - 2..], ["nofixentries", "nounmappedfixentries"]);
    assert_eq!(&fix_schema_tags()[..3], [8, 9, 35]);

    // The spelling stays on the field, so a renderer shows `MsgType` over `35`.
    let held = schema.get_field_by_path("35").expect("the msgtype column");
    assert_eq!(held.display(), Some("MsgType"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixRegistry, fix_schema, fix_schema_tags

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    schema = fix_schema(registry, "FixMessage")

    columns = [child.name for child in schema]
    assert columns[:3] == ["8", "9", "35"]
    assert columns[-2:] == ["nofixentries", "nounmappedfixentries"]
    assert fix_schema_tags()[:3] == [8, 9, 35]

    # The spelling stays on the field, so a renderer shows `MsgType` over `35`.
    assert schema.field("35").display == "MsgType"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const schema = fix.schema(registry, 'FixMessage')

    assert.equal(schema.fieldAt(0).name, '8')
    assert.equal(schema.fieldAt(2).name, '35')
    assert.equal(schema.fieldAt(schema.fieldLen - 2).name, 'nofixentries')
    assert.deepEqual(fix.schemaTags().slice(0, 3), [8, 9, 35])

    // The spelling stays on the field, so a renderer shows `MsgType` over `35`.
    assert.equal(schema.field('35').display, 'MsgType')
    ```

## Nothing is lost at the end

Two lists close every row.

`nofixentries` is the whole arrival record: every pair, in arrival order, untranslated, and a group's members riding under the counter pair that heads them. It is a counter-named list like every FIX group, its occurrence a `fixentry` struct. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

`nounmappedfixentries` is a **view** over that record rather than the rest of it: the pairs no dictionary explained, discovered pre-order at any depth and flattened to one level, in the order they arrived. It holds nothing `nofixentries` does not, and it exists so a venue onboarding a new field finds it by reading one column instead of filtering a million rows. On a well-known dialect it is empty on every row and costs a validity bit.

## The crate's own columns

Seven fields carry six facts a capture states that no dictionary publishes: the last fact needs separate client and venue parent identifiers. Each is an ordinary field on this crate's own branch, so it lifts, columns, serializes and resolves with no special case anywhere. `FixRegistry::with_crate_fields` registers them; nothing in *reading* a message needs them, because every one is a fact about the capture rather than about the wire.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `msghash` | `MsgHash` | 30001 | the xxh3-128 digest of what the message said: the arrival record with the standard header and trailer left out, `MsgType` excepted |
| `version` | `Version` | 30002 | the FIX version it was *read* at, which is not always what `BeginString` claimed |
| `symbolticker` | `SymbolTicker` | 30003 | one instrument symbol that is the same across venues |
| `timestamp` | `Timestamp` | 30004 | the market clock a capture is ordered by |
| `unixpartition` | `UnixPartition` | 30005 | the partition that clock falls in, as whole seconds |
| `parentclordid` | `ParentClOrdID` | 30006 | the client order identifier this order descends from |
| `parentorderid` | `ParentOrderID` | 30007 | the venue order identifier this order descends from |

The envelope `msghash` drops is the standard header and the standard trailer whole, read from the same two tag lists the row shape is ordered by, so a tag either component gains leaves the digest without a second listing learning about it. `MsgType` is the one exception and stays in: a message type is what a message *is* rather than how it travelled, so an order and a report carrying the same tags are not one message. The consequence is the point - two identical orders sent a second apart hash equal, and so do the same order relayed through two sessions or replayed on a resend.

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
    assert_eq!(partition.as_partition().sources()?, Some(vec!["30004".to_owned()]));
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
    assert partition.metadata["partition:sources"] == '["30004"]'
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
    assert.equal(partition.getProperty('partition', 'sources'), '["30004"]')
    assert.equal(partition.getProperty('iceberg', 'transform'), 'truncate[3600]')
    ```

## A bridge configuration is a dictionary of its own

A [bridge configuration document](registry.md#a-bridge-configuration-is-a-document-that-names-itself) states what a session interface *is* — which venue it talks to, over which host and port, at which sequence numbers, in which state — and FIX publishes almost none of it. Four rules read it, and the first is the one the crate's own columns already keep.

**A field the specification has is never given a second tag.** ULBridge spells `SenderCompID`, `TargetCompID` and `BeginString` under FIX's own names, so they resolve to tags 49, 56 and 8 with nothing added.

**Everything else is ULBridge's own dictionary.** The `ulbridge` branch, tags from 20001. Not the standard branch, because the specification publishes no `PrimaryHost`; not [the crate's](#the-crates-own-columns), because the crate did not invent one. Being a branch is also what lets a venue keep its own 20001 without colliding.

**A name resolves in the message's branch and then in the standard one.** That is the tier a built message is read by, so building it under one dictionary alone would drop tag 49 the moment a reader pinned the bridge's.

**One entry or fifty is one shape.** Jolokia answers a single read with one attribute map and a wildcard read with a map keyed by ObjectName; both read as one repeating group, `SessionInterfaces`, whose occurrences are the MBeans the document answered for in canonical ObjectName order.

| the document says | the row holds |
| --- | --- |
| the `request` it echoes, or the request itself | `MBean` 20001, `Operation` 20002 |
| `status`, `error` | `Status` 20003, `Error` 20004 |
| each MBean the `value` answers for | one `SessionInterfaces` 20005 occurrence |
| that entry's ObjectName | `SessionInterface`, `MBeanType`, `PluginType` |
| `SenderCompID`, `TargetCompID`, `BeginString` | FIX's 49, 56, 8 |
| every other attribute | its own tag, typed — a port is a number, `NeedReload` is a boolean |
| an attribute that is an object or an array | the JSON it is, under its own name |

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixBranch, FixCodec, FixRegistry, Scalar, ULBRIDGE_BRANCH};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?.with_ulbridge_fields()?;
    let reader = FixCodec::new(Arc::new(registry)).with_branch(&FixBranch::from_str(ULBRIDGE_BRANCH)?);

    let document = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","CurrentPort":7061,"BackupHost":null,"NeedReload":false},"status":200}"#;
    let held = reader.transform_line(document, false)?;

    // The envelope is what the exchange was, and it types.
    assert_eq!(held.by_tag(yggdryl::OPERATION_TAG)?, &Scalar::from("read"));
    assert_eq!(held.by_tag(yggdryl::STATUS_TAG)?, &Scalar::from(200_i64));

    // The entry is one occurrence, and FIX's own names keep FIX's own tags.
    assert_eq!(held.by_path("SessionInterfaces.0.SenderCompID")?, &Scalar::from("ULB_BKRBDG"));
    assert_eq!(held.by_path("SessionInterfaces.0.MBeanType")?, &Scalar::from("Plugin"));
    // A port is a number and a flag is a boolean, not the text they arrived as.
    assert_eq!(held.by_path("SessionInterfaces.0.CurrentPort")?, &Scalar::from(7061_i64));
    assert_eq!(held.by_path("SessionInterfaces.0.NeedReload")?, &Scalar::from(false));
    // A stated null is an absence: no field, no entry.
    assert!(held.get_by_path("SessionInterfaces.0.BackupHost").is_none());
    ```

Rust only: neither binding registers ULBridge's fields today, and a dictionary that does not have them keeps every key under its own folded spelling rather than dropping it.

### Edges

- A group inside a group — `ExtendedActions[i].parameters[j]` — is one level deeper than a key addresses, so it is retained as the JSON it is. The four arrays a session interface always carries are declared, so each has a name and a tag; anything else keeps its own folded spelling.
- A bulk answer is an array of these and reads as the first response in it: one line is one message, and a document declares one exchange.
- A request document carries no `value`, so it is the envelope alone — which is also what says it went out rather than came back.
- A sentinel is a number the bridge sent: `BackupPort: -1` and `LogLevel: -1` land as `-1`, never as an absence. A stated `null` is the absence.
- Without `with_ulbridge_fields`, every attribute is still kept — as nullable text under its own folded spelling, with no tag.

## A column is the name its tag spells

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [resolution tiers](registry.md#tiers) would be a hash, a verification and a branch walk. None of that runs per row: the schema is fixed, its columns are named `"35"` and `"55"`, and `to_row` fills each one by reading the tag out of the column's own name.

So there is nothing beside the schema to build, hold, or invalidate. A caller finds a column with `index_of` on the schema it already has, and two captures sharing a dictionary share both the schema and every position in it.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `to_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

That is the rule the whole row keeps: what a message said can never fail the batch it arrives in.

## A capture's own columns lead the row

A line was read from somewhere, and where it was read from is what a monitor orders and joins on: the object's URL, the line number in it, the clock the line was stamped with, the thread that wrote it. None of that is FIX and all of it is the row, so it leads the row - and because a line in is a row out, carrying it is a slice rather than a join.

A carried column whose name a FIX column already takes is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema.

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
    assert_eq!(carried.index_of("35"), plain.index_of("35").map(|at| at + 3));
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
    assert carried.index_of("35") == plain.index_of("35") + 3
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
    assert.equal(carried.indexOf('35'), plain.indexOf('35') + 3)
    ```

## Edges

- A tag the dictionary does not have is skipped rather than invented: a column with no field behind it could not be typed.
- A column no tag names is the capture's own, so `to_row` answers null there; whoever read the capture fills it.
- A clock a narrow dictionary types as text is still an instant in the derived `timestamp` column: FIX's own spelling is read there, and text that is not a timestamp is null rather than a refusal.
- `index_of` on a column the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- Switching [deduplication](arrow.md#a-row-in-is-a-row-out) on surrenders the row-in / row-out correspondence, so it is off by default and what went is counted rather than silent.

## Performance

`fix/read`, one line each against the tracked seed dictionary. Release build, one Linux x86_64 container; a capture is read line by line, so the per-line figure is the whole cost.

| shape | bytes | median |
| --- | --- | --- |
| a framed tag stream with prose either side | 85 | 13.6 us |
| a bare tag stream | 64 | 13.4 us |
| the same, read at a pinned 4.2 | 64 | 13.5 us |
| a bridge row keyed by name | 78 | 15.9 us |
| a bridge row with a packed repeating group | 147 | 23.1 us |
| a bridge row of `#` keys, one twinned by its bare spelling | 108 | 18.8 us |
| a wide bridge row, three hundred `#` keys around one twin | 3716 | 659 us |
| a bridge configuration document | 630 | 91.8 us |
| the same, on a dictionary without ULBridge's fields | 630 | 82.9 us |
| the emit that closes the round trip | | 194 ns |

A document costs about seven times a frame at ten times the bytes, and the difference is what it is: a frame is split on a byte and a document is parsed as JSON and walked. Typing it against ULBridge's own dictionary adds 11% over reading it untyped, which is what resolving thirty names costs — and what buys a port that is a number rather than the text it arrived as. The twin scan that decides a `#` costs nothing to see here: a bridge row splits into borrowed slices, the row of `#` keys reads faster per byte than the named one, and the wide row's per-pair cost matches the narrow one's - the scan probes the row's few bare spellings rather than the whole row, so it stays linear.

A dated read costs what an undated one costs, within a code translation per value: a version decides which spellings answer, and no field is renamed or retyped for it, so there is nothing per row to resolve or cache.

Regenerate with:

```bash
cargo bench -p yggdryl --bench fix -- fix/read
```

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix schema::
    cargo test -p yggdryl --test fix reader::a_bridge_frame
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
