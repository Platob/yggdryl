# Capture

[`FixCodec`](#a-reader-is-the-whole-parse-surface) turns captured lines into [messages](message.md), and [`FixMsg::into_row`](#a-column-is-the-name-its-tag-spells) projects each message under the scalar and group columns declared by [`fix_schema`](#the-columns-are-the-tags).

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec`, `fix_schema`, `fix_schema_carrying`, `fix_schema_tags`, `FixMsg::into_row`, `fix_crate_fields` |
| Columns | scalar columns use decimal tags; named group columns hold Lists; `display` carries the readable spelling |
| Shape | standard header, the fields a consumer reads, three groups, the trailer, this crate's derived facts, then `nofixentries` and `nounmappedfixentries` |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `nofixentries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Expansion | ordinary lines yield one message; a bulk configuration yields one per selected MBean, including every response; empty bulk or wildcard answers yield none |
| Found | `index_of("35")` finds a scalar column's position; row projection reads message tag indexes and uses registry lookup when needed |

## Use

Iterate the messages from a line and project each under one declared row field.

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

    order, = reader.transform_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")
    row = order.into_row(schema).as_py()

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

    const [order] = reader.transformLine(Buffer.from('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|'))
    const row = order.intoRow(schema).toJSON()

    assert.equal(row[schema.indexOf('35')], 'D')
    assert.equal(row[schema.indexOf('55')], 'AAPL')
    // A tag no dictionary explains is still in the row, in its own column.
    assert.equal(row[row.length - 1].length, 1)
    ```

## Try it

The [Decode](decode.md) page displays native results for the committed frame corpus. Run the examples here to parse other input.

## Find a column

The column viewer searches the native fixed row field by tag, name and description.

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

## The columns are the tags

`35` identifies the scalar message-code column. A tag is the one name a field has in every version and every dialect: tag 32 is `LastShares` in 4.2 and `LastQty` in a newer one, and a column named either of those changes meaning when a venue upgrades. The tag never moves, so the column never does.

Scalar columns carry `fix:tag`, `display`, lineage and inline `fix:codes`. A named group column carries `fix:counter`: the numeric count remains in its own tag column, while the group column holds the occurrences.

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

`nofixentries` is the whole arrival record: every pair, in arrival order, untranslated, and a group's members riding under the counter pair that heads them. It is a list of `fixentry` structs. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

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

`Ulconfig` is one selected MBean with its attributes and shared source response.
`Ulconfig::from_json_bytes` and `from_json_scalar` validate the input and return
`Ulconfigs`. Bulk responses retain array order; wildcard MBeans use canonical
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
    use yggdryl::{FixBranch, FixCodec, FixRegistry, Scalar, Ulconfig};

    let body = br#"[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]"#;
    let registry = FixRegistry::new().with_ulbridge_fields()?;
    let codec = FixCodec::new(Arc::new(registry)).with_branch(&FixBranch::from_str("ulbridge")?);
    let mut configurations = Ulconfig::from_json_bytes(body)?;
    let first = configurations.next().expect("first configuration");
    assert_eq!(first.name(), Some("A"));
    assert_eq!(configurations.count(), 2);

    let message = first.into_fixmsg(&codec, false)?;
    assert_eq!(message.by_name("CurrentPort")?, &Scalar::from(7061_i64));
    assert_eq!(Ulconfig::from_fixmsg(&message)?.name(), Some("A"));
    let mut count = 0;
    for message in codec.transform_line(body, false)? {
        message?;
        count += 1;
    }
    assert_eq!(count, 3);
    ```

=== "Python"

    ```python
    from yggdryl.fix import FixCodec, FixRegistry, Ulconfig

    body = b'[{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,type=Plugin":{"Name":"A","CurrentPort":7061},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,type=Plugin":{"Name":"B","CurrentPort":7062}},"status":200},{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}]'
    registry = FixRegistry()
    registry.with_ulbridge_fields()
    codec = FixCodec(registry, branch="ulbridge")
    configurations = Ulconfig.from_json_bytes(body)
    first = next(configurations)
    assert first.name == "A"
    assert sum(1 for _ in configurations) == 2

    message = first.into_fixmsg(codec)
    assert message.by_name("CurrentPort").as_py() == 7061
    assert Ulconfig.from_fixmsg(message).name == "A"
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
    const configurations = fix.Ulconfig.fromJsonBytes(body)[Symbol.iterator]()
    const first = configurations.next().value
    assert.equal(first.name, 'A')
    assert.equal([...configurations].length, 2)

    const message = first.intoFixmsg(codec)
    assert.equal(message.byName('CurrentPort').asJs(), 7061)
    assert.equal(fix.Ulconfig.fromFixmsg(message).name, 'A')
    assert.equal([...codec.transformLine(body)].length, 3)
    ```

### Edges

- An empty bulk array or empty wildcard response yields zero configurations.
- A request-only document or an error response without a value yields one
  envelope message. Status and error belong to each selected response.
- Null attributes are absent. Numeric sentinels such as `-1` remain numbers.
- The original response may contain sibling MBeans; equality and hashing of a
  selected `Ulconfig` depend on that configuration and its exchange metadata,
  not unrelated sibling attributes.
- A malformed bulk member reports its location. Conversion errors stop and fuse
  `FixMessages`; the iterator does not skip a failed message and continue.
- [Arrow parsing](arrow.md#one-row-per-message) repeats each source row's carried
  columns for all configurations produced from its body.

## A column is the name its tag spells

The row field fixes column order before messages arrive. For scalar columns such as `"35"` and `"55"`, `into_row` reads the tag from the column name and calls the message's tag lookup: the message's own index answers first, followed by the [registry resolution tiers](registry.md#tiers) and the retained numeric spelling when needed.

A caller finds an output position with `index_of` on that row field. Group columns use `fix:counter` to select the message's indexed group value; the numeric count remains a separate scalar column. Projection performs these lookups for each row while reusing the declared output shape.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `into_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

That is the rule the whole row keeps: what a message said can never fail the batch it arrives in.

## A capture's own columns lead the row

A line's URL, line number, timestamp and other capture fields lead its FIX columns. A bulk configuration can produce several message rows; each receives the same carried values from its source row.

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
- A column with neither a scalar tag nor a declared group is carried input; `into_row` leaves it null for the capture reader to fill.
- A clock a narrow dictionary types as text is still an instant in the derived `timestamp` column: FIX's own spelling is read there, and text that is not a timestamp is null rather than a refusal.
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
