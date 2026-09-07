# Capture

A day of session log is a table. This page is the road from one to the other: [`FixReader`](#a-reader-is-the-whole-parse-surface) turns a captured line into a [message](message.md), [`fix_schema`](#the-columns-are-the-tags) is the one row shape every message answers as, and [`FixProjection`](#a-projection-resolves-the-columns-once) is what makes filling it an indexed read rather than six thousand [dictionary](registry.md) lookups a row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixReader`, `fix_schema`, `fix_schema_tags`, `FixProjection`, `FixMsg::to_row`, `fix_crate_fields` |
| Columns | named by tag as decimal text - `"35"`, never `"msgtype"`; the spelling stays on the field's `display` |
| Shape | standard header, the fields a consumer reads, three groups, the trailer, this crate's derived facts, then `entries` and `unmapped` |
| Decided | before the first row is read, from the dictionary alone; never inferred from the data |
| Lossless | `entries` is the whole arrival record, so the wire is rebuilt from it and never from the columns |
| Refuses | nothing a row's content can do; a line the reader cannot read is a message with nothing in it, and the row count still matches the capture's |
| Cached | a reader holds one version's projections; a clone gets a cache of its own |

## Use

One line in, one row out, with the columns named by tag.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixProjection, FixReader, FixRegistry, Scalar};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);

    let projection = FixProjection::new(&registry, "FixMessage")?;
    let reader = FixReader::new(Arc::clone(&registry));
    let order = reader.text("sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")?;

    let row = order.to_row(&projection);
    let held = row.as_sequence().expect("a row");
    let at = projection.position_of(35).expect("the msgtype column");
    assert_eq!(held[at].as_str(), Some("D"));

    // A tag no dictionary explains is still in the row, in its own column.
    let unmapped = held.last().and_then(Scalar::as_sequence).expect("the list");
    assert_eq!(unmapped.len(), 1);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixProjection, FixReader, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    projection = FixProjection(registry, "FixMessage")
    reader = FixReader(registry)

    order = reader.text("sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|")
    row = order.to_row(projection).as_py()

    assert row[projection.position_of(35)] == "D"
    assert row[projection.position_of(55)] == "AAPL"
    # A tag no dictionary explains is still in the row, in its own column.
    assert len(row[-1]) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const projection = new fix.FixProjection(registry, 'FixMessage')
    const reader = new fix.FixReader(registry)

    const order = reader.text('sending >> 8=FIX.4.4|35=D|55=AAPL|54=1|44=10.5|9999=x|10=0|')
    const row = order.toRow(projection).toJSON()

    assert.equal(row[projection.positionOf(35)], 'D')
    assert.equal(row[projection.positionOf(55)], 'AAPL')
    // A tag no dictionary explains is still in the row, in its own column.
    assert.equal(row[row.length - 1].length, 1)
    ```

## Try it

Every shape a capture holds, read by the real package, is on the [Decode](decode.md) page — beside a reader that takes a frame of your own.

## Find a column

Eighty-nine columns is more than anyone scrolls, and the question a reader actually has is *which column holds this*. The filter matches the tag, the field name and the wording alike.

<div class="ygg-fx" data-fix="projection" markdown="1">
This section renders `assets/fix.json` and needs JavaScript.
</div>

## A reader is the whole parse surface

Five entry points, because a capture holds five shapes and guessing between them is what a reader exists to avoid. `text` and `bytes` take a captured line whatever it is wrapped in and read the verb in front of the frame; `fixtext` takes a numeric frame whose separator the caller states; `ultext` takes a bridge frame whose keys are names; `pairs` takes what a caller already split.

Each is the core's own method under the same name in all three languages.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixReader, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixReader::new(Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?));

    // A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    // value packs its members behind the two control bytes ULLINK uses.
    let bridge: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|";
    let held = reader.bytes(bridge)?;

    assert_eq!(held.by_tag(55)?.as_str(), Some("TTF"));
    assert_eq!(held.by_tag(44)?.as_f64(), Some(41.25));
    // The packed members became three real fields under one nesting.
    let party = held.party("1").expect("the buy-side party");
    assert_eq!(party.id().and_then(yggdryl::Scalar::as_str), Some("BUYSIDE"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixReader, FixRegistry

    reader = FixReader(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    # value packs its members behind the two control bytes ULLINK uses.
    held = reader.bytes(
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

    const reader = new fix.FixReader(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A bridge frame: `#`-prefixed name keys, and one group occurrence whose
    // value packs its members behind the two control bytes ULLINK uses.
    const bridge = Buffer.from(
      '|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1' +
        '|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|',
      'binary',
    )
    const held = reader.bytes(bridge)

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
- A stated absence - one of `null_values` - produces no field and no entry, because a key that said nothing was sent is not a key that was sent.
- A pinned `source_version` decides which name a tag answers to: tag 32 is `lastshares` at 4.2 and `lastqty` at a newer one.
- Cloning a reader gives it a projection cache of its own, because two readers differing in version would otherwise clear each other's every row.

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
    assert_eq!(&columns[columns.len() - 2..], ["entries", "unmapped"]);
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
    assert columns[-2:] == ["entries", "unmapped"]
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
    assert.equal(schema.fieldAt(schema.fieldLen - 2).name, 'entries')
    assert.deepEqual(fix.schemaTags().slice(0, 3), [8, 9, 35])

    // The spelling stays on the field, so a renderer shows `MsgType` over `35`.
    assert.equal(schema.field('35').display, 'MsgType')
    ```

## Nothing is lost at the end

Two lists close every row.

`entries` is the whole arrival record: every pair, in arrival order, untranslated. It is what makes a row lossless - the fixed columns are a *reading* of the message and the entries *are* the message, so the wire is rebuilt from them and never from the columns.

`unmapped` is a **view** over that record rather than the rest of it: the pairs no dictionary explained, in the order they arrived. It holds nothing `entries` does not, and it exists so a venue onboarding a new field finds it by reading one column instead of filtering a million rows. On a well-known dialect it is empty on every row and costs a validity bit.

## The crate's own columns

Seven fields carry six facts a capture states that no dictionary publishes: the last fact needs separate client and venue parent identifiers. Each is an ordinary field on this crate's own branch, so it lifts, columns, serializes and resolves with no special case anywhere. `FixRegistry::with_crate_fields` registers them; nothing in *reading* a message needs them, because every one is a fact about the capture rather than about the wire.

| Column | Display | Tag | Holds |
| --- | --- | --- | --- |
| `msghash` | `MsgHash` | 30001 | the xxh3-128 digest of what the message said, envelope tags excluded |
| `version` | `Version` | 30002 | the FIX version it was *read* at, which is not always what `BeginString` claimed |
| `symbolticker` | `SymbolTicker` | 30003 | one instrument symbol that is the same across venues |
| `timestamp` | `Timestamp` | 30004 | the market clock a capture is ordered by |
| `unixpartition` | `UnixPartition` | 30005 | the partition that clock falls in, as whole seconds |
| `parentclordid` | `ParentClOrdID` | 30006 | the client order identifier this order descends from |
| `parentorderid` | `ParentOrderID` | 30007 | the venue order identifier this order descends from |

Two of them declare more than a type, in the protocols the crate already has rather than in a spelling only a FIX reader would know to look for. `msghash` is a digest holder, so it says which algorithm filled it and what it read. `unixpartition` is a derived partition column, so it says which column it derives from and how.

=== "Rust"

    ```rust
    use yggdryl::fix_crate_fields;

    let fields = fix_crate_fields()?;
    let digest = &fields[0];
    assert_eq!(digest.name(), "msghash");
    assert_eq!(digest.display(), Some("MsgHash"));
    assert!(digest.as_digest().is_holder());
    assert_eq!(digest.as_digest().sources()?, Some(vec!["entries".to_owned()]));

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
    assert digest.metadata["digest:sources"] == '["entries"]'

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
    assert.equal(digest.getProperty('digest', 'sources'), '["entries"]')

    const partition = held[4]
    assert.equal(partition.name, 'unixpartition')
    assert.equal(partition.display, 'UnixPartition')
    // Which column it derives from, and how: `truncate[3600]`, because the
    // value is seconds floored to a multiple of the width.
    assert.equal(partition.getProperty('partition', 'sources'), '["30004"]')
    assert.equal(partition.getProperty('iceberg', 'transform'), 'truncate[3600]')
    ```

## A projection resolves the columns once

Every message in a capture asks for the same tags in the same order, and each ask through the ordinary [resolution tiers](registry.md#tiers) is a hash, a verification and a branch walk. A projection resolves them once and turns the per-row cost into an indexed read, which is the whole reason a fixed schema is worth having.

It is held beside a dictionary rather than inside one: a projection is a reader's concern, and a dictionary carrying one would have to invalidate it on every edit.

### A group is laid out the way the column declares it

A message's group holds the members that occurrence stated, in the order it stated them; the fixed column declares the dictionary's. `to_row` places them by name and leaves the rest null, so an occurrence a bridge packed into one member lands in the same columns as one that spelled every member out - and an occurrence shorter than the dictionary declares is a row rather than a refusal.

That is the rule the whole row keeps: what a message said can never fail the batch it arrives in.

## A capture's own columns lead the row

A line was read from somewhere, and where it was read from is what a monitor orders and joins on: the object's URL, the line number in it, the clock the line was stamped with, the thread that wrote it. None of that is FIX and all of it is the row, so it leads the row - and because a line in is a row out, carrying it is a slice rather than a join.

A carried column whose name a FIX column already takes is dropped rather than renamed or duplicated: the FIX column is the one a reader spelling it means, and two columns of one name is not a schema.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixProjection, FixRegistry, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    let capture = DataType::from_fields([
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Binary.required_field("body"),
    ])?
    .required_field("line");

    let plain = FixProjection::new(&registry, "FixMessage")?;
    let carried = FixProjection::carrying(&capture, fix_schema(&registry, "FixMessage")?)?;

    assert_eq!(carried.carried(), 3);
    assert_eq!(carried.column(0).map(yggdryl::Field::name), Some("url"));
    // The FIX columns keep their order; they only start further along.
    assert_eq!(
        carried.position_of(35),
        plain.position_of(35).map(|at| at + 3),
    );
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import DataType, Field
    from yggdryl.fix import FixProjection, FixRegistry

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

    plain = FixProjection(registry, "FixMessage")
    carried = FixProjection(registry, "FixMessage", capture)

    assert carried.carried == 3
    assert carried.column(0).name == "url"
    # The FIX columns keep their order; they only start further along.
    assert carried.position_of(35) == plain.position_of(35) + 3
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

    const plain = new fix.FixProjection(registry, 'FixMessage')
    const carried = new fix.FixProjection(registry, 'FixMessage', capture)

    assert.equal(carried.carried, 3)
    assert.equal(carried.column(0).name, 'url')
    // The FIX columns keep their order; they only start further along.
    assert.equal(carried.positionOf(35), plain.positionOf(35) + 3)
    ```

## Edges

- A tag the dictionary does not have is skipped rather than invented: a column with no field behind it could not be typed.
- A carried column carries no tag, so `to_row` answers null there; whoever read the capture fills it.
- A clock a narrow dictionary types as text is still an instant in the derived `timestamp` column: FIX's own spelling is read there, and text that is not a timestamp is null rather than a refusal.
- `position_of` on a tag the schema does not carry -> `None`, never a wrong column.
- Two captures sharing a dictionary share a schema exactly, because the shape is built without reading a single message.
- Switching [deduplication](arrow.md#a-row-in-is-a-row-out) on surrenders the row-in / row-out correspondence, so it is off by default and what went is counted rather than silent.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix schema::
    cargo test -p yggdryl --test fix reader::a_bridge_frame
    cargo run -p yggdryl-cli -- schema --root config/fix
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
