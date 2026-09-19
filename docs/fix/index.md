# FIX

FIX field definitions are ordinary fields: a `FIX:` vocabulary on a [`Field`](../types/field.md), a [registry](registry.md) resolving them, [shards](store.md) persisting them, a [message](message.md) typed against one, an [Arrow boundary](arrow.md) streaming a whole capture through it, a [capture](capture.md) landing in one fixed row, and a [tool](cli.md) to manage all of it.

The dictionary is also open in the browser: [explore](explorer.md) it, [decode](decode.md) a frame against it, or inspect its native [wire emission](encode.md).

## Pages

| Page | Purpose |
| --- | --- |
| [FIX](index.md) | This page: vocabulary, `FixId`, membership, nesting |
| [Explorer](explorer.md) | The whole dictionary live: counts, field search, message layouts, provenance |
| [Decode](decode.md) | A line in, every message it holds out; every shape a capture holds, read by the package |
| [Encode](encode.md) | Native wire emission from captured message entries |
| [Registry](registry.md) | `FixRegistry`: one-namespace resolution, `FixKey`, mutation, protocol inference, the process-wide default |
| [Store](store.md) | Shard trees under one `IOBase` folder, `from_handle`, `write_into`, the tracked seed |
| [Message](message.md) | `FixMsg`: a market event over a content row - the typed holders, the accessors, `set`/`remove`, `from_row` reading a fixed row back, and what restating a message under the dictionary decides |
| [Arrow](arrow.md) | `FixCodec::parse_text_arrow_reader`, `lifecycle_arrow_reader`, `messages`, `arrow_reader`, `write_arrow_reader`: a capture already in Arrow, streamed through a dictionary and back to the wire, batched by raw bytes |
| [Capture](capture.md) | `FixCodec` and its `parse_*` readers, `fix_schema`, `FixMsg::into_row`, and what a parse fills in for a message: a day of session log as one table |
| [Lifecycle](lifecycle.md) | `FixCodec::lifecycle` and the [graph](../graph.md)'s one walk: chains named by the cross code, their creation and history, twins folded, and grid snapshots across a stream |
| [CLI](cli.md) | `ygg`: dictionary CRUD, `.cfb` ingest, schema dump, quality and drift, from a terminal |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixField` / `FixFieldMut` (`as_fix()` / `as_fix_mut()`), `FixId`; no second field class |
| Keys | `FIX:tag`, `FIX:tags`, `FIX:names`, `FIX:branches`, `FIX:identifiers`; name, datatype, `display` and `description` stay the field's own |
| Identity | A field is its tag and its name, and nothing else. `FixId` is one `i32`: the signed XXH32 of the tag's four little-endian bytes followed by the folded name; `FixId::of(tag, name)` builds it and refuses a tag that is not positive; `Copy`, four bytes, its own hash key |
| Spelling | Rendered as its decimal digest wherever it crosses a boundary - `FixKey::Id`, `FixMsg::get_by_id`, Python `int`, JavaScript `number`, a row column; `FixId::from_digest` reads that integer back; a bare integer anywhere else (`FixKey::from(i32)`, `registry.field(55)`, `msg.get(55)`) is a tag |
| Fold | ASCII case, `_`, `-` and space are not part of the name, so `Msg_Type`, `msgtype` and `MsgType` under tag 35 are one id, the one every name lookup already answers |
| Derived | Computed on every read from `FIX:tag` and the field's name, never stored (no `FIX:id` key), so a rename is never stale; `None` without a tag; read-only in every binding |
| Membership | `FIX:branches` lists the dictionaries that contributed a field: each name non-empty and without a comma, folded to ASCII lowercase, deduplicated under the fold, kept sorted and comma-joined, so registries built from the same dictionaries in any order hash alike; empty input removes the key |
| Dialect | The name `from_cfb_file(handle, Some("cme"))` stamps on every field, group, component and message the file produces, standard tags included; `FixRegistry::dialects()` lists the distinct names; provenance a caller filters on, never consulted by a lookup, and never part of the identity |
| Tag range | Any positive tag holds an identity; nothing gates a tag on its dictionary. `set_tag`, `set_tags` and `set_counter` refuse zero and negative tags, naming their key: tag 0 marks only an [unresolved arrival entry](capture.md#nothing-is-lost-at-the-end), never a definition. Derived definition tags take `FixId::DEFINITION_TAG_MIN..FixId::DEFINITION_TAG_MAX`, `[100000, 1100000)` |
| Order | Tag-major, the tag's holder first, then id: `FixFieldIter`, `next_field_after`, the bindings' iteration and the store all follow it, so the bare tag comes back to the field that held it across a round trip |
| List properties | `FIX:names` and `FIX:tags` are compact JSON arrays, `["Qty","Quantity"]` and `[1088]`, crossed by a store as the arrays they are; `names()` walks the array lazily and `tags()` parses it to a `Vec`. `FIX:branches`, `FIX:identifiers` and `FIX:nulls` stay comma-separated text, `branches()`, `identifiers()` and `nulls()` lazy slices of it. An empty list removes the key |
| Identifiers | A component declares its own direct scalar members through `FIX:identifiers`; names, aliases and decimal tags resolve once to canonical names in component order, never by flattening a group |
| Errors | `InvalidMetadataValue` naming the full key; the field stays unchanged |
| Categories | `fields/` stores tagged scalar fields; `components/` named Structs, a message being the one that carries `FIX:msgtype`; `groups/` List/LargeList occurrences and Map entries. Every one is reached through the registry's [field doors](registry.md#accessors) |
| Bindings | Python `field.fix` and `yggdryl.fix`; JavaScript `field.fix` and its `fix` namespace; the id crosses as an integer, membership as a list of strings |

## Use

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let mut field = DataType::decimal128(20, 8)?.nullable_field("OrderQty");
    field.as_fix_mut().set_tag(38)?;
    field.as_fix_mut().set_names(["Qty", "Quantity"])?;
    field.as_fix_mut().set_description("Quantity ordered.")?;
    field.set_display("Order quantity")?;

    assert_eq!(field.as_fix().tag()?, Some(38));
    assert_eq!(field.as_fix().tags()?, Vec::<i32>::new());
    assert_eq!(field.as_fix().names().collect::<Vec<_>>(), ["Qty", "Quantity"]);
    assert_eq!(field.as_fix().description(), Some("Quantity ordered."));
    // Stored as ordinary namespaced text, in the one metadata map: a list is
    // the compact JSON array it is.
    assert_eq!(field.get_metadata("FIX:names"), Some("[\"Qty\",\"Quantity\"]"));
    // Two, not three: a description is a fact about the column rather than a
    // FIX fact, so it lives on the generic key beside `display`.
    assert_eq!(field.get_metadata("description"), Some("Quantity ordered."));
    assert_eq!(field.as_fix().len(), 2);

    // A refusal names the full key and leaves the field unchanged.
    let error = field.as_fix_mut().set_tags(&[152, 152]).unwrap_err();
    assert!(error.to_string().contains("FIX:tags"), "{error}");
    assert!(!field.has_metadata("FIX:tags"));
    for tag in [0, -1] {
        let error = field.as_fix_mut().set_tag(tag).unwrap_err();
        assert!(error.to_string().contains("FIX:tag"), "{error}");
    }
    assert_eq!(field.as_fix().tag()?, Some(38));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field

    field = Field("OrderQty", "decimal128(20, 8)")
    field.fix.tag = 38
    field.fix.names = ["Qty", "Quantity"]
    field.fix.description = "Quantity ordered."
    field.set_display("Order quantity")

    assert field.fix.tag == 38
    assert field.fix.tags == []
    assert field.fix.names == ["Qty", "Quantity"]
    assert field.fix.description == "Quantity ordered."
    # Stored as ordinary namespaced text, in the one metadata map: a list is
    # the compact JSON array it is.
    assert field.metadata["FIX:names"] == '["Qty","Quantity"]'
    # Two, not three: a description is a fact about the column rather than a
    # FIX fact, so it lives on the generic key beside `display`.
    assert field.metadata["description"] == "Quantity ordered."
    assert len(field.fix) == 2

    # A refusal names the full key and leaves the field unchanged.
    with pytest.raises(ValueError, match="FIX:tags"):
        field.fix.tags = [152, 152]
    assert "FIX:tags" not in field.metadata
    for tag in (0, -1):
        with pytest.raises(ValueError, match="FIX:tag"):
            field.fix.tag = tag
    assert field.fix.tag == 38

    # A bool is never a tag, and one outside i32 is never narrowed.
    with pytest.raises(TypeError, match="not bool"):
        field.fix.tag = True
    with pytest.raises(OverflowError):
        field.fix.tag = 2**31

    # An empty list removes a list property; `del` removes any of them.
    field.fix.names = []
    assert field.fix.names == []
    del field.fix["tag"]
    assert field.fix.tag is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = Field.from('OrderQty: decimal128(20, 8)')
    field.fix.tag = 38
    field.fix.names = ['Qty', 'Quantity']
    field.fix.description = 'Quantity ordered.'
    field.setDisplay('Order quantity')

    assert.equal(field.fix.tag, 38)
    assert.deepEqual(field.fix.tags, [])
    assert.deepEqual(field.fix.names, ['Qty', 'Quantity'])
    assert.equal(field.fix.description, 'Quantity ordered.')
    // Stored as ordinary namespaced text, in the one metadata map: a list is
    // the compact JSON array it is.
    assert.equal(field.get('FIX:names'), '["Qty","Quantity"]')
    // Two, not three: a description is a fact about the column rather than a
    // FIX fact, so it lives on the generic key beside `display`.
    assert.equal(field.get('description'), 'Quantity ordered.')
    assert.equal(field.fix.size, 2)

    // A refusal names the full key and leaves the field unchanged.
    assert.throws(() => {
      field.fix.tags = [152, 152]
    }, /FIX:tags/)
    assert.equal(field.has('FIX:tags'), false)
    for (const tag of [0, -1]) {
      assert.throws(() => {
        field.fix.tag = tag
      }, /FIX:tag/)
    }
    assert.equal(field.fix.tag, 38)

    // A tag crosses as a number and is never narrowed, and the vocabulary is
    // answered only by the fix view.
    assert.throws(() => {
      field.fix.tag = 2 ** 31
    }, /signed 32-bit integer/)
    assert.throws(() => field.iceberg.tag, TypeError)

    // An empty array removes a list property; `delete` removes any of them.
    field.fix.names = []
    assert.deepEqual(field.fix.names, [])
    field.fix.delete('tag')
    assert.equal(field.fix.tag, null)
    ```

## The vocabulary is metadata

The namespace adds only what FIX states beyond a field, and a caller never spells `FIX:`.

| Property | Key | Type | Meaning |
| --- | --- | --- | --- |
| `branches` | `FIX:branches` | sorted lowercase name list | the dictionaries that contributed this field; absent for a field the specification alone defines |
| `tag` | `FIX:tag` | `i32` | canonical tag, always positive |
| `tags` | `FIX:tags` | JSON array of positive `i32` | alternate tags, highest priority first |
| `names` | `FIX:names` | JSON array of names | alternate names, highest priority first |
| `identifiers` | `FIX:identifiers` | canonical member names, in component order | the component's direct scalar identifiers; [declaration and compiled selection](registry.md#component-identifiers) |
| `description` | `description` | text | the specification's wording, on the generic key every catalog reads |
| `codes` | `FIX:codes` | canonical JSON, by wire value | the inline enum values declared by this field; see [Registry](registry.md#a-field-carries-its-code-set) |
| `counter` | `FIX:counter` | `i32` | on a List/LargeList group, the separate scalar count field's tag; on a crate Map group, its own tag, without a scalar counter |
| `component` | `FIX:component` | name | component reference, including a group's occurrence |
| `field_ref` / `fieldRef` | `FIX:field` | name | scalar field reference in a definition |
| `group` | `FIX:group` | name | group reference in a definition |
| `msgtype` | `FIX:msgtype` | text | complete case-sensitive wire code on a message Struct |
| `replacements` | `FIX:replacements` | canonical JSON, in order | how a value of this field is restated at a later version: the fields it fills and the values they take; see [Registry](registry.md#a-field-carries-what-replaced-it) |
| `directions` | `FIX:directions` | canonical JSON, in stated order | on tag 385: per code of the set, the `regex::bytes` patterns that name it from the prose in front of a payload; absent reads by the built-in defaults; see [Registry](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) |

## Identity is a tag and a name

A tag is what identifies a field on the wire and a name is what identifies it to a reader, so the identity is both: `FixId::of(tag, name)` digests the tag's four little-endian bytes and the folded name into one signed 32-bit integer. Nothing writes it; every read derives it from `FIX:tag` and the field's current name, so a retag or a rename is another identity and a spelling under the fold is the same one. Membership is provenance kept beside it and is no half of it. How a registry answers an id, and why a bare integer there is a tag, is [one namespace](registry.md#one-namespace).

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId};

    let mut trade = DataType::utf8().nullable_field("TradeID");
    // No membership means the specification alone, and there is no
    // identity without a tag.
    assert_eq!(trade.as_fix().branches().count(), 0);
    assert_eq!(trade.as_fix().id()?, None);

    trade.as_fix_mut().set_tag(5001)?;
    let id = trade.as_fix().id()?.expect("a tagged field has an identity");
    assert_eq!(id, FixId::of(5001, "TradeID")?);
    assert_eq!(std::mem::size_of::<FixId>(), 4);
    // Rendered as its decimal digest wherever it crosses a boundary, and
    // read back from that integer.
    assert_eq!(id.to_string(), id.digest().to_string());
    assert_eq!(FixId::from_digest(id.digest()), id);
    // Derived on every read from `FIX:tag` and the name; nothing stores it.
    assert!(!trade.has_metadata("FIX:id"));
    assert_eq!(trade.as_fix().len(), 1);

    // One fold: ASCII case, `_`, `-` and space are not part of the name.
    let msgtype = FixId::of(35, "MsgType")?;
    assert_eq!(FixId::of(35, "msg_type")?, msgtype);
    assert_eq!(FixId::of(35, "MSG-TYPE")?, msgtype);
    assert_eq!(FixId::of(35, "Msg Type")?, msgtype);
    // Both halves count: another tag or another name is another identity.
    assert_ne!(FixId::of(36, "MsgType")?, msgtype);
    assert_ne!(FixId::of(35, "MsgSeqNum")?, msgtype);
    // The tag half is positive: 0 marks an unresolved arrival, never a field.
    assert!(FixId::of(-1, "MsgType").is_err());
    assert!(FixId::of(0, "").is_err());
    assert!(FixId::of(1, "").is_ok());
    assert!(FixId::of(i32::MAX, "MsgType").is_ok());

    // Membership is provenance and no half of the identity: folded to
    // ASCII lowercase, deduplicated, sorted, stored comma-joined.
    trade.as_fix_mut().set_branches(["Globex", "CME", "cme"])?;
    assert_eq!(trade.get_metadata("FIX:branches"), Some("cme,globex"));
    assert!(trade.as_fix().has_branch("CME"));
    assert_eq!(trade.as_fix().id()?, Some(id));
    trade.as_fix_mut().add_branch("blp")?;
    assert_eq!(trade.as_fix().branches().collect::<Vec<_>>(), ["blp", "cme", "globex"]);
    // Held to the membership grammar: non-empty, no comma; a refusal names the key.
    let error = trade.as_fix_mut().set_branches(["c,me"]).unwrap_err();
    assert!(error.to_string().contains("FIX:branches"), "{error}");
    assert_eq!(trade.get_metadata("FIX:branches"), Some("blp,cme,globex"));

    // A rename under the fold is the same identity; another name is another.
    trade.set_name("Trade_ID");
    assert_eq!(trade.as_fix().id()?, Some(id));
    trade.set_name("TradeReportID");
    assert_eq!(trade.as_fix().id()?, Some(FixId::of(5001, "TradeReportID")?));
    assert_ne!(trade.as_fix().id()?, Some(id));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field

    trade = Field("TradeID", "utf8")
    # No membership means the specification alone, and there is no identity
    # without a tag.
    assert trade.fix.branches == []
    assert trade.fix.id is None

    trade.fix.tag = 5001
    held = trade.fix.id
    # One signed 32-bit integer, derived on every read from `FIX:tag` and the
    # name; nothing stores it, and nothing else can say it.
    assert isinstance(held, int) and -(2**31) <= held < 2**31
    assert "FIX:id" not in trade.metadata
    assert set(trade.fix) == {"tag"}
    with pytest.raises(AttributeError):
        trade.fix.id = 7

    # One fold: ASCII case, `_`, `-` and space are not part of the name.
    msgtype = Field("MsgType", "utf8", metadata={"FIX:tag": "35"}).fix.id
    for spelling in ("msgtype", "MSG_TYPE", "Msg-Type", "Msg Type"):
        assert Field(spelling, "utf8", metadata={"FIX:tag": "35"}).fix.id == msgtype, spelling
    # Both halves count: another tag or another name is another identity.
    assert Field("MsgType", "utf8", metadata={"FIX:tag": "36"}).fix.id != msgtype
    assert Field("MsgSeqNum", "utf8", metadata={"FIX:tag": "35"}).fix.id != msgtype

    # Membership is provenance and no half of the identity: folded to ASCII
    # lowercase, deduplicated, sorted, stored comma-joined.
    trade.fix.branches = ["Globex", "CME", "cme"]
    assert trade.fix.branches == ["cme", "globex"]
    assert trade.metadata["FIX:branches"] == "cme,globex"
    assert trade.fix.has_branch("CME")
    trade.fix.add_branch("blp")
    assert trade.fix.branches == ["blp", "cme", "globex"]
    assert trade.fix.id == held
    # Held to the membership grammar: non-empty, no comma; a refusal names the key.
    with pytest.raises(ValueError, match="FIX:branches"):
        trade.fix.branches = ["c,me"]
    assert trade.fix.branches == ["blp", "cme", "globex"]

    # A rename under the fold is the same identity; another name is another.
    trade.set_name("Trade_ID")
    assert trade.fix.id == held
    trade.set_name("TradeReportID")
    assert trade.fix.id != held
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const trade = Field.from('TradeID: utf8')
    // No membership means the specification alone, and there is no identity
    // without a tag.
    assert.deepEqual(trade.fix.branches, [])
    assert.equal(trade.fix.id, null)

    trade.fix.tag = 5001
    const held = trade.fix.id
    // One signed 32-bit integer, derived on every read from `FIX:tag` and the
    // name; nothing stores it, and the property has no setter.
    assert.ok(Number.isInteger(held) && held >= -(2 ** 31) && held < 2 ** 31)
    assert.equal(trade.has('FIX:id'), false)
    assert.equal(trade.fix.size, 1)

    // One fold: ASCII case, `_`, `-` and space are not part of the name.
    const idOf = (name, tag) => {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = tag
      return field.fix.id
    }
    const msgtype = idOf('MsgType', 35)
    for (const spelling of ['msgtype', 'MSG_TYPE', 'Msg-Type', 'Msg Type']) {
      assert.equal(idOf(spelling, 35), msgtype, spelling)
    }
    // Both halves count: another tag or another name is another identity.
    assert.notEqual(idOf('MsgType', 36), msgtype)
    assert.notEqual(idOf('MsgSeqNum', 35), msgtype)

    // Membership is provenance and no half of the identity: folded to ASCII
    // lowercase, deduplicated, sorted, stored comma-joined.
    trade.fix.branches = ['Globex', 'CME', 'cme']
    assert.deepEqual(trade.fix.branches, ['cme', 'globex'])
    assert.equal(trade.get('FIX:branches'), 'cme,globex')
    assert.equal(trade.fix.hasBranch('CME'), true)
    trade.fix.addBranch('blp')
    assert.deepEqual(trade.fix.branches, ['blp', 'cme', 'globex'])
    assert.equal(trade.fix.id, held)
    // Held to the membership grammar: non-empty, no comma; a refusal names the key.
    assert.throws(() => {
      trade.fix.branches = ['c,me']
    }, /FIX:branches/)
    assert.deepEqual(trade.fix.branches, ['blp', 'cme', 'globex'])

    // A rename under the fold is the same identity; another name is another.
    trade.setName('Trade_ID')
    assert.equal(trade.fix.id, held)
    trade.setName('TradeReportID')
    assert.notEqual(trade.fix.id, held)
    ```

## Nesting needs no second type

`NoPartyIDs` is an `int32` field at tag 453. `Parties` is a separate List of the
`Party` Struct, linked to that count through `FIX:counter`. Fields, components
and groups are the three registry categories, a message being a component that
carries `FIX:msgtype`.

The crate's `identifiers(65020)` and `metadata(65049)` are also groups: a
nullable, sorted-key `map<utf8, utf8>` each, whose occurrence is its non-null
entries Struct, with no separate scalar counter and no invented numeric tags
for its key or value. A parse fills the first from the message's
[declared identifiers](registry.md#component-identifiers) and the second from
the [namespaced keys](capture.md#a-composed-key-fills-the-field-its-last-segment-names)
a bridge wrote, while ordinary List/LargeList groups keep their existing
counter rules.

The published FIX component names guide the catalog: [FIX message structures](https://fixtrading.org/concepts-part1-messagestructures/)
and [FIX Orchestra](https://github.com/FIXTradingCommunity/fix-orchestra-spec/blob/master/v1-0-STANDARD/orchestra_spec.md)
distinguish fields, components, messages and repeating groups. The generator
uses a standard name when it fits the role, then a deterministic descriptive
name checked against existing field and definition names. The native canonical
names are folded; `display` keeps the specification's spelling.

=== "Rust"

    ```rust
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixRegistry, FieldPath};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;
    assert_eq!(registry.field_by_tag(453)?.dtype(), &DataType::Int32);
    // The counter names the group it opens, and one door answers all three
    // categories: a scalar, a component, a group.
    let parties = registry.field_by_counter(453)?;
    assert_eq!(parties.name(), "parties");
    assert_eq!(parties.as_fix().counter()?, Some(453));
    assert!(!registry.field_by_name("Party")?.fields().is_empty());
    assert_eq!(registry.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("PartyID")?.as_fix().tag()?, Some(448));
    let identifiers = registry.field_by_counter(65_020)?;
    assert_eq!(identifiers.name(), "identifiers");
    assert_eq!(identifiers.as_fix().counter()?, Some(65_020));
    assert!(registry.get_field_by_tag(65_020).is_none(), "a Map group is no scalar");
    ```

=== "Python"

    ```python
    from pathlib import Path
    from yggdryl.fix import FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert str(registry.field_by_tag(453).dtype) == "int32"
    # The counter names the group it opens, and one door answers all three
    # categories: a scalar, a component, a group.
    parties = registry.field_by_counter(453)
    assert parties.name == "parties"
    assert parties.fix.counter == 453
    assert registry.field_by_name("Party").is_struct
    assert registry.field_by_path("Parties.PartyID").fix.tag == 448
    assert registry.field_by_name("PartyID").fix.tag == 448
    identifiers = registry.field_by_counter(65_020)
    assert identifiers.name == "identifiers"
    assert identifiers.fix.counter == 65_020
    assert registry.get_field_by_tag(65_020) is None, "a Map group is no scalar"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    assert.equal(registry.fieldByTag(453).dtype.toString(), 'int32')
    // The counter names the group it opens, and one door answers all three
    // categories: a scalar, a component, a group.
    const parties = registry.fieldByCounter(453)
    assert.equal(parties.name, 'parties')
    assert.equal(parties.fix.counter, 453)
    assert.ok(registry.fieldByName('Party').fieldLen > 0)
    assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByName('PartyID').fix.tag, 448)
    const identifiers = registry.fieldByCounter(65020)
    assert.equal(identifiers.name, 'identifiers')
    assert.equal(identifiers.fix.counter, 65020)
    assert.equal(registry.getFieldByTag(65020), null, 'a Map group is no scalar')
    ```

## Edges

- An empty name, a duplicate (names ASCII-folded, tags exact), a name holding a quote, a backslash or a control character, or a zero or negative tag -> refused naming `FIX:tags` / `FIX:names` / `FIX:tag`; field unchanged. A name may hold a comma: the array frames it.
- A stored `FIX:tags` that is not the compact array the setter writes - comma text, a spaced or signed element, an unclosed bracket - is refused where it is read; a stored `FIX:names` that is not one reads as nothing rather than as part of a list.
- An identifier spelling that is empty, contains a comma, names no member, names a nested member, is ambiguous, or repeats a selected member -> a located `FIX:identifiers` refusal; the whole field stays unchanged. Empty input removes the declaration.
- Folding is ASCII only: `Größe` and `GRÖSSE` are two names.
- A tag is decimal `1` to `i32::MAX`; readers refuse stored `0`, `+35`, `-35`, `3x`.
- Python `tag = True` -> `TypeError`; `2**31` -> `OverflowError`. JavaScript `2 ** 31` -> "signed 32-bit integer"; `field.iceberg.tag` -> `TypeError`.
- Any positive tag holds an identity, whatever dictionary spoke it; the only tag refusal is a zero or negative one, from `set_tag`, `set_tags`, `set_counter` or `FixId::of`, naming `FIX:tag` / `FIX:tags` / `FIX:counter`. Tag 0 appears only on an arrival entry no dictionary resolved.
- The id is read, never assigned: Python `trade.fix.id = 7` -> `AttributeError`; the JavaScript property has no setter; there is no `FIX:id` key and no id key spelling in the CLI.
- A membership name that is empty or carries a comma -> refused naming `FIX:branches`; field unchanged. Names fold to ASCII lowercase on the way in and `has_branch` folds its argument the same way; a number is never a name (`TypeError` in Python, a `String` conversion failure in JavaScript).
- `FixKey::Id` is the one spelling of an id in Rust; a bare `i32` is a tag through `FixKey::from`, `registry.field(55)` and `msg.get(55)`. Python `field_by_id(int)` / `get_by_id(int)` and JavaScript `fieldById(number)` / `getById(number)` are exact: no alias, alternate tag or fold is consulted on the way.
- Two identities digesting to one 32-bit id are a typed conflict on insert, and every id hit is rechecked against the tag and the folded name before it counts.
- A message root the codec builds carries no membership: a message is not a dictionary member.
- `get_field_by_path` traverses a declared group with or without an occurrence: a schema states one item type, so `Parties[0].PartyID` and `Parties.PartyID` reach the same field, and the first is the spelling a message value takes.
- A shared count tag can describe different group layouts. A message singleton selects its own group context; an ambiguous registry-wide counter lookup fails.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::name_indexes_fold_ascii fix::tests::three_spellings_of_one_name fix::tests::an_identifier_is_one_integer fix::tests::membership_folds_once fix::tests::membership_round_trips fix::tests::properties_round_trip fix::tests::a_property_write fix::tests::the_fold_table_holds fix::tests::iteration_and_the_cursor_are_tag_major fix::tests::a_corrupt_stored fix::tests::a_path_reaches
    cargo bench -p yggdryl --bench fix -- fix/mutate/set_
    cargo bench -p yggdryl --bench fix -- fix/mutate/add_branch
    cargo bench -p yggdryl --bench fix -- fix/resolve/id_
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "vocabulary or tag_rejects or id_is_the_tag or membership"
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="typed fix vocabulary|answers only on the fix view|never narrowed|identifier is a number|membership is a sorted list" node/tests/fix/fix.test.js
    npm run --prefix node bench:fix
    ```
