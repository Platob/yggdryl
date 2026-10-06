# Registry

`FixRegistry` owns tagged scalar fields and named message, component and group definitions in one namespace, reached through one family of field doors, with atomic mutations and indexed borrowed reads.

## Contract

| Category | Native definition | Identity |
| --- | --- | --- |
| `fields` | Tagged scalar `Field`; group counters are `int32` | The tag and the folded name together; the id is derived from the pair on every read, never stored |
| `components` | Named Struct `Field`; one carrying `FIX:msgtype` is a message - non-null, owned by an immutable `MsgType` singleton, borrowed through `msgtype` | Folded name; `FIX:msgtype` carries the complete wire code |
| `groups` | Named Serie/LargeSerie of a non-null Struct occurrence, or Map with a non-null entries Struct | Folded name; a Serie/LargeSerie names its separate scalar counter, while a crate Map uses its own tag as `FIX:counter` |

| Aspect | Rule |
| --- | --- |
| Enums | A vocabulary is a named code set the registry owns rather than a property of one field: the members are held once under the set's name and a scalar's `FIX:codeset` states that name. A registry refuses a field naming a set it does not hold, so a set is stated before a field reads by it. Every version's values are in the set: a code an older version declared and the newest dropped is a code of the set like any other, and an older spelling of a surviving code is one of its aliases. The list order is the specification's own rank |
| Code sets | `codesets` is a fourth thing the registry holds beside the three categories, and no `FixCategory`: a set carries no tag, no datatype and no reference, so `FixCategory::ALL` is still the three and the sets have doors of their own - `codeset`, `codeset_of`, `codesets`, `set_codeset`, `merge_codeset`, `remove_codeset`. A [store](store.md) writes them to `codesets/<name>.json` and a snapshot to a `codesets` key |
| History | The dictionary holds one reading of each tag; a spelling an earlier version used is written beside it in the field's `FIX:names`, and a field FIX retired is still in the dictionary under its own tag |
| Bridge names | the generator writes the names a bridge spells a standard field by into that field's `FIX:names`: `omsdealeraccount` on `Account(1)`, `ultraderclordid` on `ClOrdID(11)`, `marketorderid` and `omsdealerorderid` on `OrderID(37)`, `detailedcficode` on `CFICode(461)` - whose two statements in one message fold into one code ([FIX message](message.md#one-namespace)) - `exchangeclientorderid` on `SecondaryClOrdID(526)` and `omsuserid` on `Username(553)` |
| Retirements | What FIX retired and what stands in for it is the crate's own [table](#what-the-specification-retired), applied as a [parse](message.md#restated-under-the-dictionary) restates a message; `FIX:deprecated` marks the field FIX Latest removed, whose value is restated and then nulled |
| Rules | A registry carries no rule of its own. What a parse [fills in](capture.md#what-a-message-implied-is-filled-in) and how it [restates](#retired-fields-are-restated-by-the-crates-table) a retired field are the crate's native code, the same for every registry: no `FIX:` key states a derivation or a replacement, and a registry's field for a target only types the answer - a registry lacking that field fills nothing there |
| Directions | Tag 385's field may carry `FIX:directions`: per code of the set, the `regex::bytes` patterns applied to the prose in front of a payload that name it; a field carrying none reads by the crate's defaults, so a dictionary that ships a table states its own |
| Identifier parents | A scalar may carry `FIX:parents`: the [identifier types](#parents-of-an-identifier) holding the parents of the identifier it states, nearest first; `parent_sources`, `parents_of` and `parent_of` read them |
| Identifier maps | A scalar may carry `FIX:idmap`: the [identifier types](#a-field-names-a-message-by-its-identifiers) its value states - the map, `identifiers`, the lower-case key the type is, whether a following operation carries it, and on `PartyID(448)` the `PartyRole(452)` of the occurrence stating it; `idmap_sources` compiles every field's once |
| Identifiers | `FIX:identifiers` declares a component's direct scalar identifiers, resolved to canonical member names in component order; a `MsgType` compiles their selection once |
| Definition tags | Components and Serie/LargeSerie groups carry a `FIX:tag` derived from their name into `[100000, 1100000)`; a reference occurrence never restates it. A crate Map group instead has one reserved tag, also its counter, with no scalar counterpart |
| Counters | A group's counter is an `int32` field of the dictionary, found by `field_by_tag`; the group it frames carries the counter's tag as `FIX:counter` and is found by `field_by_counter`. The counter frames the group on the wire and nothing else: a message or component definition lists the group alone, its length the count, and one listing the counter beside the group is refused |
| Doors | one family, and every category answers it: `field_by_tag`, `field_by_name`, `field_by_id`, `field_by_path`, `field_by_counter` and the generic `field`, each with its `get_` twin; `insert` files a Struct as a component, a Serie/LargeSerie of a Struct or a Map as a group, anything else as a scalar, and `update`, `add_field`, `merge_with` and `remove` take any of the three |
| References | `FIX:field`, `FIX:component`, and `FIX:group` resolve once at catalog intake; live definitions hold resolved native fields |
| Planning | Message identity, contextual counter lookup, group layouts and identifier selection are compiled before parsing rows |
| Mutation | A refusal leaves every category and index unchanged; metadata edits refresh referenced occurrences atomically |
| Identity spelling | A case-only replacement preserves the stored canonical name; an identity or referenced datatype change is refused |
| Membership | `FIX:sources` lists the ids of the sources that contributed a field, and the registry's [sources catalog](#membership) holds one `FixSource` per id - the file it was read from and its plugin's [`PluginSide`](../types/enum/pluginside.md) - provenance a caller filters on; no lookup consults it, and a message root the codec builds carries none |
| Iteration | Scalar fields iterate tag-major, the tag's holder first, then id; named categories and message singletons have deterministic native order |
| Ownership | Rust borrows definitions. Python and Node views retain the native registry; mutation refuses while a codec, message, singleton, or active iterator shares it |
| Snapshot | `into_json` / `from_json` preserve the vocabularies, the sources catalog and the three categories - `{codesets, sources, fields, components, groups}` and no other key, `sources` stated only where the catalog holds an entry, the sets leading so a reader holds them before it meets a field naming one - with each field's membership inside its metadata; stable hashes include that complete state, the catalog among it |
| Crate definitions | The [crate listing](capture.md#the-crates-own-columns) has 50 definitions, tags 65001 to 65050: 49 scalar fields and the sorted Map group `metadata(65035)`. Eighteen are derived (`is_derived_tag`) and never registered; `new()` registers the other 32 - 31 scalar fields and `metadata` - beside `SendingTime(52)` and `TransactTime(60)`, so an empty registry holds 33 scalar fields and one group: 34 definitions. A [store](store.md) writes these builtins like any other definition, and a stored one can never override the constructed one |
| Standard clocks | `new()` seeds `SendingTime(52)` and `TransactTime(60)` as ordinary nanosecond UTC fields; they account for two of the empty registry's 33 scalar definitions. A loaded dictionary defining either supplies its own matching layout |

## Use

`NoPartyIDs(453)` is a field of the dictionary that frames the group on the wire, and no definition lists it beside the group: `Parties` stores the occurrences, its length the count, and `Party` describes one occurrence.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry, FieldPath, StructType};

    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453)?;
    let mut party_id = DataType::utf8().nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut registry = FixRegistry::from_fields([counter, party_id])?;

    // One door files each by its shape: a Struct is a component, a Serie of
    // one a group, and a scalar a field.
    let mut member = registry.field(448)?.clone();
    member.as_fix_mut().set_field_ref("PartyID")?;
    let party = DataType::from(StructType::from_fields([member])?).required_field("Party");
    registry.insert(party.clone())?;
    let mut parties = DataType::serie(party).nullable_field("Parties");
    parties.as_fix_mut().set_counter(453)?;
    parties.as_fix_mut().set_component("Party")?;
    registry.insert(parties)?;

    let mut group = registry.field_by_name("Parties")?.clone();
    group.as_fix_mut().set_group("Parties")?;
    let mut order = DataType::from(StructType::from_fields([group])?).required_field("Order");
    order.as_fix_mut().set_msgtype("D")?;
    registry.insert(order)?;

    assert_eq!(registry.field(453)?.dtype(), &DataType::Int32);
    assert_eq!(registry.field_by_path(&FieldPath::from_str("Order.Parties.PartyID")?)?.as_fix().tag()?, Some(448));
    let message = registry.msgtype("D")?;
    assert_eq!(message.name(), "Order");
    assert_eq!(message.get_group_by_tag(453).expect("the group the counter opens").name(), "Parties");
    // A counter names the group it opens; the counter itself is a field.
    assert_eq!(registry.field_by_counter(453)?.name(), "Parties");
    assert_eq!(registry.field_by_name("Party")?.fields().len(), 1);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field
    from yggdryl.fix import FixRegistry

    counter = Field("NoPartyIDs", "int32")
    counter.fix.tag = 453
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    registry = FixRegistry.from_fields([counter, party_id])

    # One door files each by its shape: a Struct is a component, a Serie of one a
    # group, and a scalar a field.
    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    party = Field("Party", DataType.from_fields([member]), nullable=False)
    registry.insert(party)
    parties = yggdryl.serie("Parties", party)
    parties.fix.counter = 453
    parties.fix.component = "Party"
    registry.insert(parties)

    group = registry.field_by_name("Parties")
    group.fix.group = "Parties"
    order = Field("Order", DataType.from_fields([group]), nullable=False)
    order.fix.msgtype = "D"
    registry.insert(order)

    assert registry.field(453).dtype == DataType("int32")
    assert registry.field_by_path("Order.Parties.PartyID").fix.tag == 448
    message = registry.msgtype("D")
    assert message.name == "Order"
    # A counter names the group it opens; the counter itself is a field.
    assert registry.field_by_counter(453).name == "Parties"
    assert [field.name for field in registry.field_by_name("Party").dtype.explode_fields()] == ["PartyID"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require('yggdryl')

    const counter = Field.from('NoPartyIDs: int32')
    counter.fix.tag = 453
    const partyId = Field.from('PartyID: utf8')
    partyId.fix.tag = 448
    const registry = fix.FixRegistry.fromFields([counter, partyId])

    // One door files each by its shape: a Struct is a component, a Serie of one a
    // group, and a scalar a field.
    const member = registry.field(448)
    member.fix.fieldRef = 'PartyID'
    const party = fields.struct('Party', [member], { nullable: false })
    registry.insert(party)
    const parties = fields.serie('Parties', party)
    parties.fix.counter = 453
    parties.fix.component = 'Party'
    registry.insert(parties)

    const group = registry.fieldByName('Parties')
    group.fix.group = 'Parties'
    const order = fields.struct('Order', [group], { nullable: false })
    order.fix.msgtype = 'D'
    registry.insert(order)

    assert.equal(registry.field(453).dtype.toString(), 'int32')
    assert.equal(registry.fieldByPath('Order.Parties.PartyID').fix.tag, 448)
    const message = registry.msgtype('D')
    assert.equal(message.name, 'Order')
    // A counter names the group it opens; the counter itself is a field.
    assert.equal(registry.fieldByCounter(453).name, 'Parties')
    assert.deepEqual(registry.fieldByName('Party').dtype.explodeFields().map(field => field.name), ['PartyID'])
    ```

### Group names

The standard calls the repeating block `Parties` and its counter `NoPartyIDs`; Orchestra separately identifies a group's counter and members. See the [FIX Parties description](https://www.fixtrading.org/online-specification/introduction/) and the [pinned Orchestra repository](https://github.com/FIXTradingCommunity/orchestrations/blob/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml).

The generator gives every group a collection display. A unique published plural leads; otherwise a unique free plural from its `No...` counter leads. Thus `AdditionalTermGrp` / `NoAdditionalTerms` becomes `additionalterms` / `AdditionalTerms`, while the explicit alternate-ID collections are `secaltids` / `SecAltIDs` - `SecAltIDGrp(454)`, a message's content rather than a column of the fixed row, whose prefix states the [`securityids`](../graph/identifier.md) identifiers it reads - and `regulatorytradeids` / `RegulatoryTradeIDs`. A singular, shared, or occupied spelling keeps an explicit `Grp`, such as `allocgrp` / `AllocGrp` or `attrbgrp` / `AttrbGrp`. The occurrence component still follows the published group spelling: `Parties` becomes `Party`, `NestedParties2` becomes `NestedParty2`, and a collision adds `Component`. The generated resolver applies the same names and displays to a standard group created from CBlock input.

## Component identifiers

`FIX:identifiers` names only a component's direct scalar members, including a message or a group's occurrence component; the setter accepts names, aliases and decimal tags, then stores canonical names in member order. `MsgType::identifier_values` reads that compiled selection from a message, returning declaration fields beside the values the message's content row holds, skipping absent or null members and never descending into groups; no column carries them - the names a message goes by are its [identifiers](message.md#the-identifier-maps), read off the fields that state them.

=== "Rust"

    ```rust
    use std::sync::Arc;
    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};

    let mut client = DataType::utf8().nullable_field("clordid");
    client.as_fix_mut().set_tag(11)?;
    client.as_fix_mut().set_names(["ClientOrder"])?;
    let mut server = DataType::utf8().nullable_field("orderid");
    server.as_fix_mut().set_tag(37)?;
    let mut order = DataType::from(StructType::from_fields([client.clone(), server.clone()])?).required_field("order");
    order.as_fix_mut().set_msgtype("D")?;
    order.as_fix_mut().set_identifiers(["37", "ClientOrder"])?;
    assert_eq!(order.as_fix().identifiers().collect::<Vec<_>>(), ["clordid", "orderid"]);
    assert_eq!(order.get_metadata("FIX:identifiers"), Some("clordid,orderid"));
    let before = order.clone();
    assert!(order.as_fix_mut().set_identifiers(["clordid", "11"]).is_err());
    assert_eq!(order, before);

    let mut registry = FixRegistry::new();
    registry.insert(order)?;
    let registry = Arc::new(registry);
    // The row is reordered; the result still follows declaration order.
    let row = DataType::from(StructType::from_fields([server, client])?).required_field("row");
    let message = FixMsg::with_registry(
        Arc::clone(&registry), row,
        Scalar::from_sequence([Scalar::from("O-1"), Scalar::from("C-1")]),
    )?;
    // The value is the message's answer rather than a borrow of its row,
    // because an identifier tag is a fact the message lifts and holds.
    let held: Vec<(&str, Scalar)> = registry.msgtype("D")?.identifier_values(&message)
        .map(|(field, value)| (field.name(), value)).collect();
    let values: Vec<(&str, Option<&str>)> = held
        .iter().map(|(name, value)| (*name, value.as_str())).collect();
    assert_eq!(values, [("clordid", Some("C-1")), ("orderid", Some("O-1"))]);
    ```

=== "Python"

    ```python
    import pytest
    from yggdryl import DataType, Field
    from yggdryl.fix import FixMsg, FixRegistry

    client = Field("clordid", "utf8")
    client.fix.tag = 11
    client.fix.names = ["ClientOrder"]
    server = Field("orderid", "utf8")
    server.fix.tag = 37
    order = Field("order", DataType.from_fields([client, server]), nullable=False)
    order.fix.msgtype = "D"
    order.fix.identifiers = ["37", "ClientOrder"]
    assert order.fix.identifiers == ["clordid", "orderid"]
    assert order.metadata["FIX:identifiers"] == "clordid,orderid"
    before = order.into_json()
    with pytest.raises(ValueError, match="FIX:identifiers"):
        order.fix.identifiers = ["clordid", "11"]
    assert order.into_json() == before

    registry = FixRegistry()
    registry.insert(order)
    # The row is reordered; the result still follows declaration order.
    row = Field("row", DataType.from_fields([server, client]), nullable=False)
    message = FixMsg(row, ["O-1", "C-1"], registry)
    values = registry.msgtype("D").identifier_values(message)
    assert [(field.name, value.as_py()) for field, value in values] == [
        ("clordid", "C-1"), ("orderid", "O-1"),
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require('yggdryl')

    const client = Field.from('clordid: utf8')
    client.fix.tag = 11
    client.fix.names = ['ClientOrder']
    const server = Field.from('orderid: utf8')
    server.fix.tag = 37
    const order = fields.struct('order', [client, server], { nullable: false })
    order.fix.msgtype = 'D'
    order.fix.identifiers = ['37', 'ClientOrder']
    assert.deepEqual(order.fix.identifiers, ['clordid', 'orderid'])
    assert.equal(order.get('FIX:identifiers'), 'clordid,orderid')
    const before = order.toJSON()
    assert.throws(() => { order.fix.identifiers = ['clordid', '11'] }, /FIX:identifiers/)
    assert.deepEqual(order.toJSON(), before)

    const registry = new fix.FixRegistry()
    registry.insert(order)
    // The row is reordered; the result still follows declaration order.
    const row = fields.struct('row', [server, client], { nullable: false })
    const message = new fix.FixMsg(row, ['O-1', 'C-1'], registry)
    const values = registry.msgtype('D').identifierValues(message)
    assert.deepEqual(values.map(([field, value]) => [field.name, value.asJs()]), [
      ['clordid', 'C-1'], ['orderid', 'O-1'],
    ])
    ```

| Boundary | Contract |
| --- | --- |
| Declaration | Rust `set_identifiers`, Python `field.fix.identifiers = iterable`, JavaScript `field.fix.identifiers = array`; an empty input removes the property, while a bare string is not a list of spellings |
| Refusal | Empty/comma-bearing spelling, missing or nested member, ambiguous spelling, or two spellings of one member: typed, located, whole-field atomic |
| Stored metadata | The same setter normalizes hand-written declarations after references resolve; reload and merge retain final component order, not the caller's spelling order |
| Selection | Exact canonical member name first; a renamed member's tag only when unique in both the declaration and row; an ambiguous tag selects nothing |
| Ownership | Rust borrows both values without allocation; Python returns read-only declaration Field clones and Scalar wrappers; Node returns independent mutable Field clones and Scalar wrappers, never a mutable registry member |
| Filled | Nothing: no column carries the selected values. A message's names are its [identifiers](message.md#the-identifier-maps), read off the fields that state them, each its type's base key, and the [lifecycle](lifecycle.md#a-chain-is-named-by-its-cross-code) joins a chain by `identifiers` |
| Lifted | A market data leaf [lifts](message.md#what-a-leafs-metadata-holds) every scalar of its metadata whose key names an identifier - an identifier name its message's type selects at the key's end, letters and digits compared whatever the case, and, for a key no dictionary field is, the crate's own identifier names too - into the set its type belongs to (`securityids`, `partyids` or `identifiers`) as the identifier its key names ([`Identifier::from_key`](../graph/identifier.md#reading-a-name)) - an execution report's `marketorderid` a `market:orderid`, a bridge's `venue.x.parentorderid` a `venue.x:parentorderid`; an occurrence's own member is checked against its component's selection |

The generator's one explicit identifier-family table annotates every matching direct member across the 109 shipped components that declare one, message definitions among them; a group member is not flattened into its enclosing message. The [CLI definition flags](cli.md#definition-flags) expose the same native setter through `--identifiers`; category replacement and the [whole-list merge rule](#one-merge-with-a-rule-per-key) remain distinct operations.

## One namespace

A field is its tag and its name, and a lookup asks for one of them: canonical before alternate for tags, canonical name before alias for names, the fold always - ASCII case and the `_`, `-` and space separators dropped - and nothing else. A tag query never searches names; no lookup takes a dialect, and none consults membership.

| Lookup | Meaning |
| --- | --- |
| `field(55)` | The canonical holder of the tag, then a field listing it as an alternate; a tag two fields hold under different names answers the first holder |
| `field_by_id(FixId)` | Exact: the one field whose tag and folded name digest to that id |
| `field_by_name(name)` | The canonical fold, then an alias fold |
| `field_by_path(path)` | Canonical Map name before a scalar alias; otherwise scalar lookup, then a named message/component/group head and nested members |
| `field_by_counter(tag)` | The globally unique repeating group that counter tag opens - `453` the `Parties` Serie, `65035` the `metadata` Map - while the counter itself answers `field(453)` |
| `MsgType::get_group_by_tag(tag)` | Unique group within that message's structure |

The `get_` forms return absence; failing twins return a typed, located error. One spelling addresses a member on both sides: a schema states one item type for a serie, so `Parties[0].PartyID` answers the field every occurrence holds here and the value that occurrence carries in a message. A path through a group may still omit the occurrence - `Parties.PartyID` - because a schema has no positions to skip. A counter shared by multiple contexts is ambiguous globally, so parsing uses the selected message's compiled group index.

A Map group is a native mapping, not a numeric repeating frame: its entries and key stay non-null and its sortedness survives projection and reload. `metadata` is reached by its canonical name or its counter, never by scalar `field_by_tag(65035)`; its key and value gain no wire delimiter or numeric tags, and a canonical scalar name cannot collide with a Map group's name.

Names and aliases use separate indexes; a stored name is rechecked after hashing, so a digest collision never selects an unrelated field. The id is the signed XXH32 of the tag's little-endian bytes followed by the folded name, so `MsgType`, `msgtype` and `Msg_Type` under tag 35 are one id; `FixId::of(tag, name)` refuses a tag that is not positive and displays as its decimal digest - the [fold and its halves](index.md#identity-is-a-tag-and-a-name) are the vocabulary's. An id crosses every boundary as that integer - `FixKey::Id` in Rust, `field_by_id(int)` and `get_by_id(int)` in Python and JavaScript - and a bare integer anywhere else is a tag.

Every name lookup reads four word pairs either way - `offer`/`ask`, `size`/`qty`, `bid`/`demand`, `px`/`price` - wherever one stands in the folded name, after every name a field holds exactly, canonically or as an alias. `AskPx`, `ASK_PX`, `askpx` and `AskPrice` reach `OfferPx(133)`, `DemandQty` reaches `BidSize(134)`, and a bridge's `ULLINK.OFFERPRICE` reaches `OfferPx` under its namespace. A name a dictionary defines as a field of its own is that field; a spelling the words make reaching two different fields reaches none. Nothing is written into the registry: the words are read at the lookup, on the stack, so a lookup allocates nothing, and the codec, the message readers and the setters all resolve through it in every language.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixKey, FixRegistry};

    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35)?;
    let registry = FixRegistry::from_fields([msgtype])?;

    let id = registry.field(35)?.as_fix().id()?.expect("a tagged field");
    assert_eq!(id, FixId::of(35, "msg_type")?);
    assert!(FixId::of(0, "MsgType").is_err());
    assert_eq!(registry.field(FixKey::Id(id))?.name(), "MsgType");
    assert_eq!(registry.field_by_id(id)?.name(), "MsgType");
    // Exact: another name on the held tag is another id, and misses.
    assert!(registry.get_field_by_id(FixId::of(35, "MsgSeqNum")?).is_none());
    assert!(registry.get_field(id.digest()).is_none(), "a bare integer is a tag");
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    msgtype = Field("MsgType", "utf8")
    msgtype.fix.tag = 35
    registry = FixRegistry.from_fields([msgtype])

    held = registry.field(35).fix.id
    assert isinstance(held, int)
    spelled = Field("msg_type", "utf8")
    spelled.fix.tag = 35
    assert spelled.fix.id == held
    assert registry.field_by_id(held).name == "MsgType"
    # Exact: another name on the held tag is another id, and misses.
    assert registry.get_field_by_id(Field("MsgSeqNum", "utf8", metadata={"FIX:tag": "35"}).fix.id) is None
    assert registry.get_field(held) is None, "a bare integer is a tag"
    assert Field("MsgType", "utf8").fix.id is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const msgtype = Field.from('MsgType: utf8')
    msgtype.fix.tag = 35
    const registry = fix.FixRegistry.fromFields([msgtype])

    const held = registry.field(35).fix.id
    assert.equal(typeof held, 'number')
    const spelled = Field.from('msg_type: utf8')
    spelled.fix.tag = 35
    assert.equal(spelled.fix.id, held)
    assert.equal(registry.fieldById(held).name, 'MsgType')
    // Exact: another name on the held tag is another id, and misses.
    const other = Field.from('MsgSeqNum: utf8')
    other.fix.tag = 35
    assert.equal(registry.getFieldById(other.fix.id), null)
    assert.equal(registry.getField(held), null, 'a bare integer is a tag')
    assert.equal(Field.from('MsgType: utf8').fix.id, null)
    ```

### What one namespace means for a field that arrives

The identity is the pair, so `add_field`, `add_fields` and `merge_with` decide by both halves; `insert` replaces only the same identity, and a held tag under another name goes beside the holder exactly as the third row says.

| The registry holds | The arrival | What happens |
| --- | --- | --- |
| The same tag under the same folded name | The same field | Update: `merge_with` unions its tags, aliases, membership and the rest |
| The same folded name under another tag | The same field spelled with another number | Merges into the holder, which gains the tag as an alternate - unless another field already answers that tag, in which case the tag is left out with a debug log - and the incoming name as an alias unless it is the canonical one; no second field |
| The same tag under another name | A new field | Registered under its own id beside the holder; the bare tag keeps answering the first holder, the newcomer is reached by its name or its id, and neither learns the other's name |
| The same tag under a name a third field holds as its canonical name | That third field spelled with another number | Merges into the field the name names, as the second row does; the tag stays with its holder and is not learned as an alternate, and every member reading the arrival reads that field. A name the third field holds only as an alias is not this row: the arrival stands beside the holder of its tag, and the alias stays where it is |
| The same tag, one side named by nothing but its decimal tag | The same field, unnamed | An unnamed arrival merges into the field answering its tag - as its own or as an alternate - whatever that field is called, except the other half of a pair the source itself linked: an arrival naming the holder's own tag among its alternates is two fields. An unnamed holder takes the arrival's name where no field answers that name already, and every member reading it under the digits reads it under the name |
| Neither | A new field | Inserted as it arrived |

A name is what identifies a field to a reader, so a new name on a held tag is a new thing a dialect defined over a tag it reused; a tag is what identifies a field on the wire, so a held name on a new tag is the same thing spelled with another number - and so is a held name on a held tag, a dialect that numbered `MaturityDate2` with `MaturityDate`'s 541: a canonical name reaches one field, so the arrival is that field, never a field beside the holder of 541 that could not be named. A decimal name is no name: a field named `541` - a CBlock tag declaring no `alt` - is a tag a file did not say the name of, so it folds into whatever answers its tag, as its own or as an alternate, and the first file naming the tag names it, rather than standing beside a numbered twin of itself; `insert` stays strict and files it beside the holder. One of this crate's own tags is every dictionary's already and is skipped, counted as neither added nor merged. Two identities digesting to one id is a typed conflict on insert, and every id hit is rechecked against the tag and the fold before it counts.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixRegistry};

    let mut symbol = DataType::utf8().nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    let mut registry = FixRegistry::from_fields([symbol])?;

    // The same folded name under another tag: one field, one more number.
    let mut spelled = DataType::utf8().nullable_field("symbol");
    spelled.as_fix_mut().set_tag(9055)?;
    spelled.as_fix_mut().set_sources(["blp"])?;
    assert!(!registry.add_field(spelled)?, "merged");
    let holder = registry.field_by_tag(9055)?;
    assert_eq!(holder.name(), "Symbol");
    assert_eq!(holder.as_fix().tags()?, [9055]);
    assert_eq!(holder.as_fix().sources().collect::<Vec<_>>(), ["blp"]);

    // The same tag under another name: a second field beside the holder.
    let mut venue = DataType::utf8().nullable_field("VenueSymbol");
    venue.as_fix_mut().set_tag(55)?;
    venue.as_fix_mut().set_sources(["xnas"])?;
    assert!(registry.add_field(venue)?, "added");
    assert_eq!(registry.field_by_tag(55)?.name(), "Symbol", "the bare tag answers the holder");
    assert_eq!(registry.field_by_tag(55)?.as_fix().names().count(), 0, "neither learns the other's name");
    assert!(!registry.field_by_tag(55)?.as_fix().has_source("xnas"));
    let newcomer = registry.field_by_id(FixId::of(55, "venue_symbol")?)?;
    assert_eq!(newcomer.name(), "VenueSymbol");
    assert_eq!(registry.field_by_name("VenueSymbol")?.name(), "VenueSymbol", "reached by its own name");
    assert_eq!(registry.dialects(), ["blp", "xnas"]);

    // A field named by nothing but its tag is unnamed: the first name to
    // arrive on the tag names it, and a later unnamed one merges into it.
    let mut unnamed = DataType::utf8().nullable_field("541");
    unnamed.as_fix_mut().set_tag(541)?;
    assert!(registry.add_field(unnamed.clone())?, "added");
    let mut maturity = DataType::utf8().nullable_field("MaturityDate");
    maturity.as_fix_mut().set_tag(541)?;
    assert!(!registry.add_field(maturity)?, "merged, and named");
    assert!(!registry.add_field(unnamed)?, "merged");
    assert_eq!(registry.field_by_tag(541)?.name(), "MaturityDate");

    // Tag-major, the tag's holder first, then id; the seeded clocks and the
    // crate's own fields sit on their own tags around them.
    let names: Vec<&str> = registry.iter().filter(|field| field.as_fix().tag().ok().flatten() < Some(65_000)).map(|field| field.name()).collect();
    assert_eq!(names, ["sendingtime", "Symbol", "VenueSymbol", "transacttime", "MaturityDate"]);
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    symbol = Field("Symbol", "utf8")
    symbol.fix.tag = 55
    registry = FixRegistry.from_fields([symbol])

    # The same folded name under another tag: one field, one more number.
    spelled = Field("symbol", "utf8")
    spelled.fix.tag = 9055
    spelled.fix.sources = ["blp"]
    assert registry.add_field(spelled) is False, "merged"
    holder = registry.field_by_tag(9055)
    assert holder.name == "Symbol"
    assert holder.fix.tags == [9055]
    assert holder.fix.sources == ["blp"]

    # The same tag under another name: a second field beside the holder.
    venue = Field("VenueSymbol", "utf8")
    venue.fix.tag = 55
    venue.fix.sources = ["xnas"]
    assert registry.add_field(venue) is True, "added"
    assert registry.field_by_tag(55).name == "Symbol", "the bare tag answers the holder"
    assert registry.field_by_tag(55).fix.names == [], "neither learns the other's name"
    assert not registry.field_by_tag(55).fix.has_source("xnas")
    assert registry.field_by_id(venue.fix.id).name == "VenueSymbol"
    assert registry.field_by_name("venue_symbol").name == "VenueSymbol", "reached by its own name"
    assert registry.dialects() == ["blp", "xnas"]

    # A field named by nothing but its tag is unnamed: the first name to
    # arrive on the tag names it, and a later unnamed one merges into it.
    unnamed = Field("541", "utf8")
    unnamed.fix.tag = 541
    assert registry.add_field(unnamed) is True, "added"
    maturity = Field("MaturityDate", "utf8")
    maturity.fix.tag = 541
    assert registry.add_field(maturity) is False, "merged, and named"
    assert registry.add_field(unnamed) is False, "merged"
    assert registry.field_by_tag(541).name == "MaturityDate"

    # Tag-major, the tag's holder first, then id; the seeded clocks and the
    # crate's own fields sit on their own tags around them.
    names = [field.name for field in registry if field.fix.tag < 65000]
    assert names == ["sendingtime", "Symbol", "VenueSymbol", "transacttime", "MaturityDate"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const symbol = Field.from('Symbol: utf8')
    symbol.fix.tag = 55
    const registry = fix.FixRegistry.fromFields([symbol])

    // The same folded name under another tag: one field, one more number.
    const spelled = Field.from('symbol: utf8')
    spelled.fix.tag = 9055
    spelled.fix.sources = ['blp']
    assert.equal(registry.addField(spelled), false, 'merged')
    const holder = registry.fieldByTag(9055)
    assert.equal(holder.name, 'Symbol')
    assert.deepEqual(holder.fix.tags, [9055])
    assert.deepEqual(holder.fix.sources, ['blp'])

    // The same tag under another name: a second field beside the holder.
    const venue = Field.from('VenueSymbol: utf8')
    venue.fix.tag = 55
    venue.fix.sources = ['xnas']
    assert.equal(registry.addField(venue), true, 'added')
    assert.equal(registry.fieldByTag(55).name, 'Symbol', 'the bare tag answers the holder')
    assert.deepEqual(registry.fieldByTag(55).fix.names, [], "neither learns the other's name")
    assert.equal(registry.fieldByTag(55).fix.hasSource('xnas'), false)
    assert.equal(registry.fieldById(venue.fix.id).name, 'VenueSymbol')
    assert.equal(registry.fieldByName('venue_symbol').name, 'VenueSymbol', 'reached by its own name')
    assert.deepEqual(registry.dialects(), ['blp', 'xnas'])

    // A field named by nothing but its tag is unnamed: the first name to
    // arrive on the tag names it, and a later unnamed one merges into it.
    const unnamed = Field.from('541: utf8')
    unnamed.fix.tag = 541
    assert.equal(registry.addField(unnamed), true, 'added')
    const maturity = Field.from('MaturityDate: utf8')
    maturity.fix.tag = 541
    assert.equal(registry.addField(maturity), false, 'merged, and named')
    assert.equal(registry.addField(unnamed), false, 'merged')
    assert.equal(registry.fieldByTag(541).name, 'MaturityDate')

    // Tag-major, the tag's holder first, then id; the seeded clocks and the
    // crate's own fields sit on their own tags around them.
    const names = [...registry].filter(field => field.fix.tag < 65000).map(field => field.name)
    assert.deepEqual(names, ['sendingtime', 'Symbol', 'VenueSymbol', 'transacttime', 'MaturityDate'])
    ```

### Membership

A dictionary is a membership, not a namespace: what a source contributed is recorded on the field it contributed to, as `FIX:sources` - a compact JSON array of source ids, `["blp","xnas"]`, each held to the id grammar (a non-empty word holding no quote, backslash or control character), folded to ASCII lowercase, deduplicated under the fold and kept sorted, so two registries built from the same sources in any order hash alike. An empty list removes the key, which is what every field the specification alone defines states: the shipped `config/fix` carries none. Membership is provenance a caller filters on; resolution never consults it, and a message root the codec builds carries none.

What is known of a source is recorded once, in the registry's sources catalog rather than on every field: one `FixSource` per id - the id, the file it was read from where one is known, and the role of its plugin, a [`PluginSide`](../types/enum/pluginside.md) that is `UKNW` where the source states none. A [store](store.md#membership) writes the catalog as `sources.json`. A field names its sources itself, so a field may name an id the catalog does not hold and the catalog may hold an entry no field names; [`yggdryl fix check`](cli.md#schema-check-and-diff) fails the first and notes the second.

| Rust | Python | JavaScript | Answer |
| --- | --- | --- | --- |
| `FixField::sources()` | `field.fix.sources` | `field.fix.sources` | The ids of the sources that contributed the field, sorted, lowercase; empty when absent |
| `FixField::has_source(id)` | `field.fix.has_source(id)` | `field.fix.hasSource(id)` | Whether one source is among them, under the crate's fold |
| `FixFieldMut::set_sources([..])` | `field.fix.sources = [..]` | `field.fix.sources = [..]` | Replace the list; an id that is empty or holds a quote, a backslash or a control character is refused, naming `FIX:sources` |
| `FixFieldMut::add_source(id)` | `field.fix.add_source(id)` | `field.fix.addSource(id)` | Add one, idempotent under the fold |
| `FixRegistry::dialects()` | `registry.dialects()` | `registry.dialects()` | The distinct ids any field or named definition names, sorted - what the fields state, not the catalog |
| `FixRegistry::sources()` | `registry.sources()` | `registry.sources()` | The catalog in id order: `FixSource` values in Rust, `{"id", "file", "pluginside"}` records in Python (`file` `None` where none is known, `pluginside` a `PluginSide` member), `{ id, file, pluginside }` objects in JavaScript (`file` left out where none is known, `pluginside` the member's stored name) |
| `FixRegistry::get_source(id)` | `registry.get_source(id)` | not bound | The entry `id` names under the fold - `VENUE` and `ve_nue` both reach `venue` - or none |
| `FixRegistry::add_source(FixSource::new(id)?.with_file(..).with_pluginside(..))` | `registry.add_source(id, *, file=None, pluginside=None)` | `registry.addSource(id, { file, pluginside })` | Record one entry, answering whether it arrived; an id already held keeps its entry and takes only what it lacked - a file where it stated none, a role where it stated `UKNW` - and a role disagreeing with a stated one keeps the held one, logged at warn |
| `FixRegistry::remove_source(id)` | `registry.remove_source(id)` | not bound | Remove the entry and answer it; refused as a conflict naming the first field or definition still naming the id |

`merge_with` on a field unions the two lists; `FixRegistry::merge_with`, `add_fields`, `add_cfb_file`, `add_cfb_files` and `add_json_file` carry that union onto whatever the registry already held and fold the other side's catalog through `add_source`, so a merged registry says which sources spoke each field and what is known of each.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry, FixSource, PluginSide};

    let mut registry = FixRegistry::new();
    let mut venue = DataType::utf8().nullable_field("VenueTag");
    venue.as_fix_mut().set_tag(20001)?;
    venue.as_fix_mut().set_sources(["Venue"])?;
    registry.insert(venue)?;
    // The field names its source; the catalog records what is known of it, once.
    assert_eq!(registry.dialects(), ["venue"]);
    assert!(registry.get_source("venue").is_none());

    let entry = FixSource::new("VENUE")?
        .with_file("venue.cfb")
        .with_pluginside(PluginSide::SellSide);
    assert!(registry.add_source(entry), "arrived");
    let held = registry.get_source("Venue").expect("one entry under the fold");
    assert_eq!(
        (held.id(), held.file(), held.pluginside()),
        ("venue", Some("venue.cfb"), PluginSide::SellSide)
    );

    // A held id keeps its entry and takes only what it lacked.
    assert!(!registry.add_source(FixSource::new("venue")?.with_pluginside(PluginSide::BuySide)));
    assert_eq!(registry.get_source("venue").map(FixSource::pluginside), Some(PluginSide::SellSide));
    // An entry a field still names stays.
    assert!(registry.remove_source("venue").is_err());
    assert_eq!(registry.sources().len(), 1);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field, PluginSide
    from yggdryl.fix import FixRegistry

    registry = FixRegistry()
    venue = Field("VenueTag", "utf8")
    venue.fix.tag = 20001
    venue.fix.sources = ["Venue"]
    registry.insert(venue)
    # The field names its source; the catalog records what is known of it, once.
    assert registry.dialects() == ["venue"]
    assert registry.get_source("venue") is None

    assert registry.add_source("VENUE", file="venue.cfb", pluginside="SELL") is True
    entry = {"id": "venue", "file": "venue.cfb", "pluginside": PluginSide.SELL}
    assert registry.get_source("Venue") == entry

    # A held id keeps its entry and takes only what it lacked.
    assert registry.add_source("venue", pluginside=PluginSide.BUYS) is False
    assert registry.sources() == [entry]
    # An entry a field still names stays.
    with pytest.raises(ValueError):
        registry.remove_source("venue")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    const venue = Field.from('VenueTag: utf8')
    venue.fix.tag = 20001
    venue.fix.sources = ['Venue']
    registry.insert(venue)
    // The field names its source; the catalog records what is known of it, once.
    assert.deepEqual(registry.dialects(), ['venue'])
    assert.deepEqual(registry.sources(), [])

    assert.equal(registry.addSource('VENUE', { file: 'venue.cfb', pluginside: 'SELL' }), true)
    const entry = { id: 'venue', file: 'venue.cfb', pluginside: 'SELL' }
    assert.deepEqual(registry.sources(), [entry])

    // A held id keeps its entry and takes only what it lacked.
    assert.equal(registry.addSource('venue', { pluginside: 'BUYS' }), false)
    assert.deepEqual(registry.sources(), [entry])
    ```

## Accessors

| Rust | Python | JavaScript |
| --- | --- | --- |
| `field_by_id(FixId)` | `field_by_id(id: int)` | `fieldById(id: number)` |
| `field_by_tag(i32)` | `field_by_tag(tag: int)` | `fieldByTag(tag: number)` |
| `field_by_name(&str)` | `field_by_name(name)` | `fieldByName(name)` |
| `field_by_path(&FieldPath)` | `field_by_path(path)` | `fieldByPath(path)` |
| `field_by_counter(i32)` | `field_by_counter(tag: int)` | `fieldByCounter(tag: number)` |
| `field(key)` | `field(key)` | `field(key)` |
| `msgtype(spelling)` | `msgtype(spelling)` | `msgtype(spelling)` |
| `dialects()` | `dialects()` | `dialects()` |
| `iter()` | `iter(registry)` | `registry[Symbol.iterator]()` |
| `len()` | `len(registry)` | `registry.size` |

Every one has a `get_` twin answering absence rather than raising it. The size and iteration cover all three categories - the scalar fields in tag-major identity order, then the components and the groups in name order - so a registry that holds one more component is one longer. Python iterators retain their registry until released; Node iterators release it on exhaustion or `return()`.

## Insert, update and remove

| Operation | Contract |
| --- | --- |
| `insert` | Files the field by its shape - a Struct as a component, a Serie/LargeSerie of a Struct or a Map as a group, anything else as a scalar - and replaces only the same identity, answering what it replaced; a held tag under another name is added beside the holder, neither learning the other's name, and a field named by nothing but its tag is added the same way; a canonical name, alias or alternate tag another field holds is a conflict |
| `update` | Merges metadata for the existing identity - same tag and folded name - using the native per-key rules; a definition is replaced whole |
| `add_field` | The lenient twin: `true` where the field arrived, `false` where it folded into one the registry held |
| `remove` | Takes a scalar, a component or a group by any key; returns no field when absent or still referenced |

These mutations preserve stored canonical spelling for case-only input changes. Referenced metadata edits cascade through components, groups, and messages; datatype changes and occurrence-local metadata overrides are refused atomically. A component or Serie/LargeSerie group stating no tag takes the one derived from its name - XXH32 of the name into `[100000, 1100000)`, stepping past a slot already taken - so a document that states a tag keeps it, and an update keeps the tag the stored definition already has. A crate Map group instead declares its own reserved tag and matching counter.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry};

    let mut registry = FixRegistry::new();
    let mut symbol = DataType::utf8().nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    assert!(registry.insert(symbol.clone())?.is_none(), "it arrived");
    assert!(registry.insert(symbol.clone())?.is_some(), "and the second insert replaced it");
    assert!(!registry.add_field(symbol.clone())?, "the lenient twin folds it in");
    symbol.set_name("SYMBOL");
    symbol.as_fix_mut().set_description("Instrument symbol")?;
    registry.update(symbol)?;
    // A case-only rename keeps the stored spelling, and the metadata merged.
    assert_eq!(registry.field(55)?.name(), "Symbol");
    assert_eq!(registry.field(55)?.description(), Some("Instrument symbol"));
    let snapshot = registry.into_json()?;
    assert_eq!(FixRegistry::from_json(&snapshot)?, registry);
    assert!(registry.remove("Symbol").is_some());
    ```

=== "Python"

    ```python
    import copy
    import pickle
    import pytest
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    registry = FixRegistry()
    symbol = Field("Symbol", "utf8")
    symbol.fix.tag = 55
    assert registry.insert(symbol) is None, "it arrived"
    assert registry.insert(symbol) is not None, "and the second insert replaced it"
    assert registry.add_field(symbol) is False, "the lenient twin folds it in"
    symbol.set_name("SYMBOL")
    symbol.fix.description = "Instrument symbol"
    registry.update(symbol)
    # A case-only rename keeps the stored spelling, and the metadata merged.
    assert registry.field(55).name == "Symbol"
    assert registry.field(55).description == "Instrument symbol"
    assert FixRegistry.from_json(registry.into_json()) == registry
    assert pickle.loads(pickle.dumps(registry)) == registry
    assert copy.copy(registry).stable_hash() == registry.stable_hash()
    with pytest.raises(TypeError):
        hash(registry)
    assert registry.remove("Symbol") is not None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    const symbol = Field.from('Symbol: utf8')
    symbol.fix.tag = 55
    assert.equal(registry.insert(symbol), null, 'it arrived')
    assert.ok(registry.insert(symbol), 'and the second insert replaced it')
    assert.equal(registry.addField(symbol), false, 'the lenient twin folds it in')
    symbol.setName('SYMBOL')
    symbol.fix.description = 'Instrument symbol'
    registry.update(symbol)
    // A case-only rename keeps the stored spelling, and the metadata merged.
    assert.equal(registry.field(55).name, 'Symbol')
    assert.equal(registry.field(55).description, 'Instrument symbol')
    assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
    assert.equal(registry.clone().stableHash(), registry.stableHash())
    assert.ok(registry.remove('Symbol'))
    ```

Python registries are mutable and unhashable; `stable_hash()` explicitly computes the native content hash. Python `copy.copy` and Node `clone()` create independently mutable registries, including every category and each field's membership.

## A field names the code set it reads by

A vocabulary belongs to the dictionary rather than to one field. The specification names each set - `SideCodeSet`, `SecurityIDSourceCodeSet` - and names it from as many fields as draw on it, so the registry holds the members once under that name and a scalar's `FIX:codeset` states only which set it reads by. The committed source dictionary holds 735 sets read by 2,027 fields; the registry adds its builtin `msgcatcodeset`, `marketdatatypecodeset`, `statecodeset` and `msgpluginsidecodeset`, so a live default registry holds 739. One time-unit set is read by 103 fields alone; `SecurityIDSource(22)` and `UnderlyingSecurityIDSource(305)` are two of the 36 fields that read `securityidsourcecodeset`, and a code named, aliased or documented once is named for every one of them.

The set is stated first, because a registry refuses a field whose `FIX:codeset` names a set it does not hold - at `insert`, `update`, `from_fields`, `from_json` and a [store](store.md) load alike. Taking one away runs the other way: `remove_codeset`, and `set_codeset` with an empty list, refuse while a held field still reads by that name, naming the field.

`marketdatatypecodeset`, `msgcatcodeset`, `statecodeset` and `msgpluginsidecodeset` are intrinsic rather than ordinary mutable vocabularies: the first names every [MarketDataType](../types/enum/marketdatatype.md) member - its stored name, the `uint16` code a `marketdatatype` column stores and what it means - the second every [MarketDataKind](../types/enum/marketdatakind.md) member - its four-letter category, the `uint8` code a `msgcat` column stores and what it stands for - the third every [State](../types/enum/state.md) member - its stored name, the `uint16` code a `state` column stores as the value, and what it means - the fourth every [PluginSide](../types/enum/pluginside.md) member - `UKNW`, `BUYS` and `SELL`, the `uint8` code a `msgpluginside` column stores and what each role does - and all four are fixed by the crate. Reinstalling the same canonical document is idempotent; replacing, widening, removing, or loading a conflicting document is refused. A custom `MsgType` may still select any symbolic category already in `msgcatcodeset`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCode, FixRegistry};

    let mut registry = FixRegistry::new();
    registry.set_codeset("sidecodeset", &[
        FixCode::new("Buy", "1"),
        FixCode::new("Sell", "2").with_aliases(["Sold"]),
    ])?;

    let mut side = DataType::utf8().nullable_field("Side");
    side.as_fix_mut().set_tag(54)?;
    side.as_fix_mut().set_codeset("sidecodeset")?;
    registry.insert(side)?;

    // The field carries the name; the dictionary answers the members.
    assert_eq!(registry.field(54)?.as_fix().codeset(), Some("sidecodeset"));
    let set = registry.codeset_of(registry.field(54)?).expect("the set the field reads by");
    assert_eq!(set.name(), "sidecodeset");
    assert_eq!(set.code_value("Buy"), Some("1"));
    assert_eq!(set.code_value("sold"), Some("2"));
    assert_eq!(set.code_name("2"), Some("Sell"));
    assert_eq!(set.codes().count(), 2);
    assert_eq!(
        registry.codesets().map(|set| set.name()).collect::<Vec<_>>(),
        ["marketdatatypecodeset", "msgcatcodeset", "msgpluginsidecodeset", "sidecodeset", "statecodeset"],
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    registry = FixRegistry()
    registry.set_codeset("sidecodeset", [
        {"value": "1", "name": "Buy"},
        {"value": "2", "name": "Sell", "aliases": ["Sold"]},
    ])

    side = Field("Side", "utf8")
    side.fix.tag = 54
    side.fix.codeset = "sidecodeset"
    registry.insert(side)

    # The field carries the name; the dictionary answers the members.
    assert registry.field(54).fix.codeset == "sidecodeset"
    assert registry.codeset_names() == [
        "marketdatatypecodeset",
        "msgcatcodeset",
        "msgpluginsidecodeset",
        "sidecodeset",
        "statecodeset",
    ]
    members = registry.codeset_of(registry.field(54))
    assert members == registry.codeset("sidecodeset")
    assert [code["name"] for code in members] == ["Buy", "Sell"]
    assert members[1]["aliases"] == ["Sold"]
    assert FixRegistry.from_json(registry.into_json()) == registry
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    registry.setCodeset('sidecodeset', [
      { value: '1', name: 'Buy' },
      { value: '2', name: 'Sell', aliases: ['Sold'] },
    ])

    const side = Field.from('Side: utf8')
    side.fix.tag = 54
    side.fix.codeset = 'sidecodeset'
    registry.insert(side)

    // The field carries the name; the dictionary answers the members.
    assert.equal(registry.field(54).fix.codeset, 'sidecodeset')
    assert.deepEqual(registry.codesetNames(), [
      'marketdatatypecodeset',
      'msgcatcodeset',
      'msgpluginsidecodeset',
      'sidecodeset',
      'statecodeset',
    ])
    const set = registry.codesetOf(registry.field(54))
    assert.equal(set.name, 'sidecodeset')
    assert.equal(registry.codeValue('sidecodeset', 'sold'), '2')
    assert.equal(registry.codeName('sidecodeset', '1'), 'Buy')
    assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
    ```

`FixCodeSet` is that set borrowed from the dictionary: `name()`, `document()` - the canonical text a store writes - and `codes()`, `code(value)`, `code_by_name(name)`, `code_value(text)` and `code_name(value)`, each read a slice of the stored document rather than a copy of it. `get_codeset` answers nothing where the dictionary holds no such set and `codeset` raises `Error::Absent` over `codesets`; `codeset_of` is the one door between a field and its members, answering nothing for a field that reads by no set; `codesets()` walks every set held, in name order. Where the specification names no set - a dialect's own file, a dictionary built in memory - `FixRegistry::derived_codeset_name(field)` is the name one takes: the folded field name and `codeset`, so `Side` states `sidecodeset`.

`code_value` first accepts an exact wire value, then a folded symbolic name or alias, then an abbreviation from the leading description phrase. Ambiguous spellings answer no value; a malformed document reports a located error through `codes()` and answers no value through the optional lookups. `set_codeset` validates and writes canonical JSON under the folded name; an empty list removes an ordinary set. The intrinsic `msgcatcodeset` exception above refuses removal or change.

| Code key | Meaning |
| --- | --- |
| `value`, `name` | Required wire value and symbolic name; `value` leads the canonical record |
| `aliases`, `doc` | Additional spellings and documentation |
| `group` | The label the specification files the code under |

The rank the specification gives a code is its position in the list, so there
is no key beside the order that states one.

`code`, `code_by_name`, `code_name`, and `code_value` borrow the selected code's data. An unknown spelling returns no match so the codec can retain the wire text. Two codes sharing a name, an empty name or value, a malformed document, and a name no store could file are refused by the writer; `set_codeset` refuses a shared name as `Error::InvalidRecord` at `codes[i]`, the later code's position, naming both names and both values. `none`, `null` and a blank name - trimmed, under the crate's one fold of case, `_`, `-` and space - name nothing, so they claim nothing: two codes may both be called `None`, and such a name resolves only where it reaches exactly one code. A held code named or valued by one is a code like any other - `EncryptMethod(98)` `0` is `None`, `SecurityType(167)` `NONE` is `NoSecurityType`. Description abbreviations ignore numeric tag cross-references and later parenthesizations; two distinct wire values sharing one folded spelling remain ambiguous.

`set_codeset` replaces what an ordinary name held; `merge_codeset` folds into it, keyed by wire value: the reading the dictionary already holds wins a shared value, a placeholder name - a code named after its own wire value, which is what a source that knows the value but not what anyone calls it writes - yields to a real one, and every surviving spelling stays as an alias; a code arriving with a new value under a name another code already claims - by a lookup, or by the rule rendering holds names to, as `eom` folds onto a placeholder code named after its own value `EOM` - keeps its value under no name, the collision logged at warn level naming both values, so a fold never answers a set the writer refuses. A `none`, `null` or blank name claims nothing and is claimed by nothing, whichever side states it, and a code merged in under one arrives under the placeholder its own value is, so a held code a sentinel names - `EncryptMethod`'s `0` `None` - still answers that spelling alone. So a second source widens a vocabulary and never narrows one, which is the same fold [`merge_with`](#one-merge-with-a-rule-per-key) runs over the other dictionary's sets before it folds a single field. `msgcatcodeset` accepts neither replacement nor widening.

=== "Rust"

    ```rust
    use yggdryl::{FixCode, FixRegistry};

    let mut registry = FixRegistry::new();
    registry.set_codeset("sidecodeset", &[
        FixCode::new("Buy", "1").with_description("the long side"),
    ])?;
    // A venue states the set it knows: the value it alone has arrives, and
    // its spelling of a value the dictionary already reads joins as an alias.
    registry.merge_codeset("sidecodeset", &[
        FixCode::new("Bought", "1").with_description("the venue's wording"),
        FixCode::new("Undisclosed", "7"),
    ])?;

    let set = registry.codeset("sidecodeset")?;
    assert_eq!(set.code_name("1"), Some("Buy"));
    assert_eq!(set.code_value("bought"), Some("1"));
    assert_eq!(set.code_name("7"), Some("Undisclosed"));
    assert_eq!(set.code("1").expect("the code").parse_doc()?.as_deref(), Some("the long side"));
    ```

=== "Python"

    ```python
    from yggdryl.fix import FixRegistry

    registry = FixRegistry()
    registry.set_codeset("sidecodeset", [{"value": "1", "name": "Buy"}])
    # A venue states the set it knows: the value it alone has arrives, and its
    # spelling of a value the dictionary already reads joins as an alias.
    registry.merge_codeset("sidecodeset", [
        {"value": "1", "name": "Bought"},
        {"value": "7", "name": "Undisclosed"},
    ])

    held = registry.codeset("sidecodeset")
    assert [code["value"] for code in held] == ["1", "7"]
    assert held[0]["name"] == "Buy" and held[0]["aliases"] == ["Bought"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    registry.setCodeset('sidecodeset', [{ value: '1', name: 'Buy' }])
    // A venue states the set it knows: the value it alone has arrives, and its
    // spelling of a value the dictionary already reads joins as an alias.
    registry.mergeCodeset('sidecodeset', [
      { value: '1', name: 'Bought' },
      { value: '7', name: 'Undisclosed' },
    ])

    const held = registry.codeset('sidecodeset').codes
    assert.deepEqual(held.map((code) => code.value), ['1', '7'])
    assert.equal(held[0].name, 'Buy')
    assert.deepEqual(held[0].aliases, ['Bought'])
    assert.equal(registry.codeValue('sidecodeset', 'Bought'), '1')
    ```

Python takes a set's members as records - `value` and `name` required, `description`, `aliases` and `group` optional - or as the canonical document in one `str`, and answers them as records; JavaScript takes and answers `{value, name, aliases, doc, group}` objects, with `codeset` and `codesetOf` answering `{name, codes}` and `codesetNames()` listing what is held. `FixCodes::parse` is the one text-to-typed-codes boundary underneath both, and what the [CLI](cli.md#code-sets)'s `--codes '<json>'` reads.

### Every version's values are in the set

The committed dictionary folds the FIX 4.0 to 5.0 SP2 listings into each set,
so a value an older version declared and the newest dropped is a code of the
set like any other: `ExecType(150)` `1` and `2`, the partial fill and the fill
FIX 4.3 folded into `Trade`, are `PartiallyFilled` and `Filled`, the names an
element's [state](../types/enum/state.md) reads. The set states one reading of each and dates
none of them.

A legacy name that folds onto a current one takes the suffix `Legacy`, and so
does a retired name the newest version spells beside the one that replaced it -
`BenchmarkCurveName(221)` `Euribor` is `EuriborLegacy` beside `EURIBOR`,
because one spelling cannot reach two codes; an older spelling of a value the
newest version keeps becomes one of its aliases, so a name a 4.2 dictionary
used still reaches the value.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::FixRegistry;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;

    // FIX 4.1 declared ExecType 1; 4.3 folded it into Trade. Both are codes
    // of the one set tag 150 reads by.
    let exectype = registry.codeset_of(registry.field_by_tag(150)?).expect("the ExecType set");
    assert_eq!(exectype.name(), "exectypecodeset");
    assert_eq!(exectype.code("1").expect("a legacy code").name(), "PartiallyFilled");
    // A spelling an older version gave a surviving value is an alias of it.
    let msgtype = registry.codeset_of(registry.field_by_tag(35)?).expect("the MsgType set");
    assert_eq!(msgtype.code_value("ExecutionAcknowledgement"), Some("BN"));
    // A legacy name that folds onto a current one takes the suffix.
    let haltreason = registry.codeset_of(registry.field_by_tag(327)?).expect("the HaltReason set");
    assert_eq!(haltreason.code_name("D"), Some("NewsDisseminationLegacy"));
    ```

## The dictionary holds one reading, and filters by no version

The registry is version-blind: it holds every tag ever defined and filters by
none. A field is the field, under the one name and datatype the dictionary
gives it, and a spelling an earlier version used reaches it as an ordinary
[name](#one-namespace) the generator writes beside it.

A field FIX retired is still in the dictionary: the generator writes every tag
some FIX 4.0 to 5.0 SP2 dictionary declares and the newest lacks -
`ExecTransType(20)`, `Rule80A(47)`, `ExecBroker(76)`, `ClientID(109)` and
thirty-four more - so `field_by_tag` answers for it, because a capture holds
what was sent.

How a retired field or value is restated is
[the crate's table](#retired-fields-are-restated-by-the-crates-table), and a
[code set](#a-field-names-the-code-set-it-reads-by) states one reading of every
value it declares. `FIX:deprecated` is the other half of that fact: it
names the version at which FIX removed the field, and a
[parse](message.md#restated-under-the-dictionary) then restates such a field's
value under what replaced it and leaves the source null, so the row holds the
fact once under its latest name while the entries keep the pair as it arrived.
The shipped dictionary marks 56 fields that way, `MaxFloor(111)` and
`MaxShow(210)` among them.

`FIX:nulls` holds the field's explicit wire spellings for absence. These
metadata documents remain on the field and round-trip through both bindings.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::FixRegistry;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;

    // Rule80A: FIX stopped declaring it after 4.3, and it is still a field.
    let rule80a = registry.field_by_tag(47)?;
    assert_eq!(rule80a.name(), "rule80a");
    // A tag the newest specification kept answers the same way; there is no
    // version to ask either of them about.
    assert!(registry.get_field(687).is_some());
    // The spelling an older version used reaches the field as an alias.
    assert_eq!(registry.field("LastShares")?.name(), "lastqty");
    // A field FIX Latest removed says so, and says at which version.
    assert_eq!(registry.field_by_tag(111)?.as_fix().deprecated(), Some("5.0"));
    assert_eq!(registry.field_by_tag(55)?.as_fix().deprecated(), None);
    ```

### `FIX:nulls`, the spellings that mean nothing was sent

`FIX:nulls` is comma-separated, matched case-insensitively against trimmed input after the field is resolved. A matched spelling becomes null in the typed row while arrival entries retain the original bytes; `FixCodec::with_null_values` is the capture-wide counterpart applied before field lookup.

## Retired fields are restated by the crate's table

The specification retires a field or a value and says what stands in for it: `Rule80A(47)` became `OrderCapacity(528)` beside `OrderRestrictions(529)`, the partial-fill values of `ExecType(150)` folded into `Trade`, `ExecBroker(76)` became one `Parties` occurrence with role `1`. Those retirements are facts about FIX itself, the same for every dictionary that declares the tags, so the crate holds them as one table keyed by the retired tag - [the table below](#what-the-specification-retired) - and a [parse](message.md#restated-under-the-dictionary) applies them as it builds the message, with nothing parsed, bound or evaluated per message. The table is every replacement rule there is: a registry states none of its own and no `FIX:` key carries one, so two registries holding the same fields restate a message alike, and a registry lacking a target's field fills nothing there.

Entries under one tag are in the specification's order, and the order is semantic: the first entry whose condition the held value meets answers, so a catch-all stating no condition comes last, and `ExecInst(18)`'s FIX 4.4 peg rule stands before its FIX 5.0 peg price types because the `R` the first writes is what the second reads. What a condition and a fill can say is [what a held value is, to a rule](message.md#what-a-held-value-is-to-a-rule).

How one entry is applied at one level - the root, or one occurrence of a repeating group:

| Step | Rule |
| --- | --- |
| Source | each child the table lists, in ascending tag order, so `ExecTransType(20)` writes `ExecType(150)` before `ExecType`'s own entries read it |
| Match | the first entry whose condition holds over the level - the held value, the message types it is scoped to, the group it applies inside |
| Plan | every target's value computed and re-typed for the target's field; a value the target cannot hold, or a part a join needs left unstated, ends the entry |
| Check | every target writable: absent, null, or already equal to what would be written; the source field itself is always writable, and a constant written over a multi-valued source replaces the code the condition named |
| Apply | all-or-nothing: one target that cannot take its value blocks the whole entry, and no later entry fills in for it; an occurrence merges into the one whose literal members all equal the planned ones, else appends one, and sets no counter, the group's length being the count |
| Chain | an entry that rewrote the source's own value leaves a new held value, restated in turn, bounded by the entries the tag lists |

### What the specification retired

The crate holds the replaced and deprecated features of FIX 4.3 through 5.0 SP2 - the specification's appendices "Replaced features" (6-F) and "Deprecated features" (6-E) - as one table in `rust/src/fix/retired.rs`: 37 retired fields, 100 entries, keyed by the retired tag and in the order the specification retired them, the appendix that stated each named beside it. An entry states only what the appendix states as a value mapping. The crate's own tests hold the table sorted by tag with each tag once, every entry filling something and a catch-all last.

| FIX | Source | Restated as |
| --- | --- | --- |
| 4.3 | `ExecTransType(20)` `1`, `2`, `3` | `ExecType(150)` `H` TradeCancel, `G` TradeCorrect, `I` OrderStatus |
| 4.3 | `ExecType(150)` `1`, `2` | `ExecType` `F` Trade |
| 4.3 | `Rule80A(47)`, every value | `OrderCapacity(528)`, and `OrderRestrictions(529)` where the appendix states one; the rows it leaves to `Side` fill only 528 |
| 4.3 | `CustomerOrFirm(204)` `0`, `1` | `OrderCapacity(528)` `A`, `P` |
| 4.3 | `ExecBroker(76)`, `BrokerOfCredit(92)`, `ClientID(109)`, `ClearingFirm(439)` | one `Parties` occurrence: `PartyID(448)` the value, `PartyRole(452)` `1`, `2`, `3`, `4` |
| 4.3 | `ClearingAccount(440)` | a `PartySubID(523)` inside the role-4 party, merged into the one `ClearingFirm` made or made alone |
| 4.3 | `SettlLocation(166)` | a `Parties` occurrence of role `10`: `PartyIDSource(447)` `C` for `CED`, `DTC`, `EUR`, `FED`, `PNY`, `PTC`, else `E`, an ISO country code |
| 4.3 | `RelatdSym(46)` | `Symbol(55)` |
| 4.3 | `MaturityDay(205)`, `UnderlyingMaturityDay(314)` | `MaturityDate(541)` joined from `MaturityMonthYear(200)`, `UnderlyingMaturityDate(542)` from `UnderlyingMaturityMonthYear(313)` |
| 4.3 | `OnBehalfOfSendingTime(370)` | one `Hops` occurrence: `HopSendingTime(629)`, `HopCompID(628)` from `OnBehalfOfCompID(115)` |
| 4.3 | `AllocTransType(71)` `0`, `3` on `J` | `AllocType(626)` `1`; `AllocTransType` `0` with `AllocType` `2` |
| 4.4 | `OrdType(40)` `5`, `A`, `B`, `C`, `F`, `H` | `OrdType` `1`, `1`, `2`, `1`, `2`, `D` with `TimeInForce(59)` `7` or `Product(460)` `4` |
| 4.4 | `SettlType(63)` `A` | `SettlType` `2` |
| 4.4 | `SecurityType(167)`, `UnderlyingSecurityType(310)`, `LegSecurityType(609)` `UST`, `USTB` | the same field, `TNOTE`, `TBILL` |
| 4.4 | `ExecInst(18)` `T` | `PegMoveType(835)` `1`, `PegScope(840)` `1`, `ExecInst` `R` |
| 4.4 | `Benchmark(219)` `1` to `9` | `BenchmarkCurveCurrency(220)`, `BenchmarkCurveName(221)`, `BenchmarkCurvePoint(222)` |
| 4.4 | `TotalAccruedInterestAmt(540)` | `AccruedInterestAmt(159)` |
| 4.4 | `SettlCurrAmt(119)`, `SettlCurrency(120)` in `AllocGrp` | `AllocSettlCurrAmt(737)`, `AllocSettlCurrency(736)` |
| 4.4 | `RedemptionDate(240)`, `RepoCollateralSecurityType(239)`, `RepurchaseRate(227)` | `DatedDate(696)`, `UnderlyingSecurityType(310)`, `Price(44)` |
| 4.4 | `RepurchaseTerm(226)` `1`, else | `TerminationType(788)` `1`, `2` |
| 4.4 | `QuantityType(465)` `6`; `1`, `5`, `8` | `QtyType(854)` `1`; `0` |
| 5.0 | `MaxFloor(111)`, `MaxShow(210)` | `DisplayQty(1138)`, `DisplayMinQty(1082)` |
| 5.0 | `OddLot(575)` `Y` | `LotType(1093)` `1` |
| 5.0 | `ExecInst(18)` `L`, `M`, `O`, `P`, `R`, `W`, `a`, `d` | `PegPriceType(1094)` `1` to `5`, `7`, `8`, `9` |
| 5.0 | `LegQty(687)` on `R`, `AJ`, `AG`, `S`, `AI`, `AB`, `8` | `LegOrderQty(685)` |
| 5.0.1 | `LegQty(687)` on `AE`, `AR` | `LegLastQty(1418)` |
| 5.0.1 | `PublishTrdIndicator(852)` `Y`, `N` | `TradePublishIndicator(1390)` `1`, `0` |
| 5.0.1 | `OrderID(37)`, `SecondaryOrderID(198)` on `r` | `MassActionReportID(1369)` |

Deliberately not covered, because the appendix states no value mapping a rule can write: `MDEntryOriginator`, `MDMkt`, `LocationID` and `DeskID` into `PartyRole`; `TargetStrategyParameters` and `ParticipationRate` into `StrategyParameters`; the settlement instruction fields 173 to 187 into `SettlParties`; `SecurityType` `FOR`, which has four candidates; `QuoteType`; `SecondaryTradeReportID` and `SecondaryTradeReportRefID`; `Signature` and the `SecureData` pair; `UnitOfMeasure` `MMbbl`; the `UnderlyingLeg` fields of EP187; `TotalNumPosReports`; `ReceivedDeptID`; `FXBenchmarkRateFix`. `SecurityType` `FUT` and `OPT` and `PutOrCall` into `CFICode` are not rules either, because the native [derivation of `CFICode`](capture.md#what-a-message-implied-is-filled-in) already states them. A field the specification removed and replaced with nothing - `SendingDate(51)`, `WaveNo(105)` - is in the dictionary with its `removed` entry and stays in a restated row as read.

Two entries are listed as the specification states them and do not fire on a parse: `OrderID(37)` and `SecondaryOrderID(198)` are lifted out of the row onto the message's own holders before the restatement reads it. `OddLot(575)` and `PublishTrdIndicator(852)` are boolean fields, and a condition on `Y` or `N` names the flag the [boolean reader](../types/numeric/boolean.md#the-one-text-reader) reads it as: `575=Y` is `LotType(1093)` `1`, `852=Y` and `852=N` are `TradePublishIndicator(1390)` `1` and `0`, and `575=N`, which no rule names, stays as it arrived.

## A registry states no derivation

A message implies values it need not carry: a report stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a `SecurityID` a check digit closes has said what standard numbered it, a CFI has said what security type it is. A [parse](capture.md#what-a-message-implied-is-filled-in) fills them by the crate's native rules - twenty-nine, one per target tag, listed with their edges there. They are facts about FIX, the same for every dictionary, so a registry carries none: no `FIX:` key states one, and a field's only part in a rule is to type the answer landing under its tag. A registry lacking the target's field fills nothing there, and one whose field will not hold the answer fills nothing either. `CountryOfIssue(470)` answers only for the 249 ISO 3166 codes `StringEnum::COUNTRIES` lists, because the `country` datatype validates width alone. No column of this crate's derives at all - what a message says about its market is FIX's own field, and the market traits read it there.

## A field names a message by its identifiers

A message goes by the names its fields state - its order's and its client order's identifiers, the quote's, the execution's - and which field states which is a fact about the field, so it travels on the field as `FIX:idmap`: one canonical document of entries, each the map the value lands in, the lower-case key that is the identifier's type, whether an operation that follows another carries it, and - on `PartyID(448)` - the `PartyRole(452)` of the `Parties` occurrence that states it. A message reads its [identifiers](message.md#the-identifier-maps) off the dictionary's table at every settle, each its type's base key, spelled as the type alone.

```text
OrderID(37)   [{"map":"identifiers","key":"orderid","follow":true}]
ClOrdID(11)   [{"map":"identifiers","key":"clordid"}]
```

| Contract | Rule |
| --- | --- |
| Key | `FIX:idmap`, read with `FixField::idmap() -> FixIdSources` - each item a `Result<FixIdSource>`: `map() -> FixIdMapKind`, `key() -> &IdType`, `follows()`, `role()` - built with `FixIdSource::new(map, key: IdType)`, and written with `FixFieldMut::set_idmap(&[FixIdSource])`, an empty list removing it; Python and JavaScript `field.fix.idmap` cross it as `{map, key, follow, role}` records, `follow` and `role` optional going in |
| Map | `identifiers`, the one map - `FixIdMapKind::Identifiers`, `FixIdMapKind::ALL`, read ASCII case folded |
| Key text | the folded word an [`IdType`](../graph/identifier.md#vocabularies) spells - `orderid`, lower-case ASCII letters, digits and `.`, one to 64 bytes, the width an [`Identifier`](../graph/identifier.md#contract) type holds; a stored document with an upper-case key or an alias of a member (`isinnumber`) is refused, and the Python and JavaScript setters fold the spelling they are given to the word it stores; one map and key once per field |
| Follow | what an entry marks is what `Operation::is_followed_identifier` answers for a message and the lifecycle carries forward, each type's [parents](#parents-of-an-identifier) travelling with it: a message naming no `OrderID(37)` carries the chain's `orderid`, `parentorderid` and `origorderid` alike |
| Role | a `PartyRole(452)` code of ASCII letters and digits, and only on `PartyID(448)` - a registry refuses one on any other tag, naming the field; a stored document whose map, key or role is no such word is refused naming the property, what it holds there and the word it holds - `expected "role" to be a PartyRole code, got "a role"`, as the writer refuses it |
| Compiled | `FixRegistry::idmap_sources() -> &[(i32, FixIdSource)]`, every field's entries beside its tag in tag order, compiled once and forgotten by every change to the fields; Python `idmap_sources()` and JavaScript `idmapSources()` answer `{tag, map, key, follow, role}` records, JavaScript leaving `role` out where the entry states none |
| Shipped | the generator writes the standard fields' from `IDMAP_SOURCES` in `scripts/generate_fix_dictionary.py`, validated against the dictionary at generation - `OrderID(37)`, `SecondaryOrderID(198)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `ExecID(17)`, `TrdMatchID(880)`, `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)`, `TradeID(1003)` and the secondary identifiers `SecondaryClOrdID(526)`, `SecondaryExecID(527)`, `SecondaryAllocID(793)`, `SecondaryIndividualAllocID(989)`, `SecondaryTradeID(1040)`, `SecondaryFirmTradeID(1042)` and `SecondaryQuoteID(1751)`; the crate states none of its own. No shipped field states a role: the parties a message names are no identifier of this map but its [party ids](message.md#parties-and-regulatory-trade-identifiers), which the crate reads itself |

## Parents of an identifier

An identifier a chain changes keeps the values it held under types of their own - `clordid`'s `origclordid`, `orderid`'s `parentorderid` and `origorderid` - and which types those are is a fact about the field, so it travels on the field as `FIX:parents`: a JSON array of identifier types, folded words, nearest first. `ClOrdID(11)` states `["origclordid"]` - FIX's own `OrigClOrdID(41)` - and a message that follows another takes the parents its chain gave each base identifier it states ([Parentage](../graph/identifier.md#parentage)).

```text
ClOrdID(11)    ["origclordid"]
TradeID(1003)  ["origtradeid"]
```

| Contract | Rule |
| --- | --- |
| Key | `FIX:parents`, read with `FixField::parents() -> Words` - each a folded identifier type, borrowed, nothing where absent - and written with `FixFieldMut::set_parents(spellings)`: each spelling folded as an [`IdType`](../graph/identifier.md#vocabularies) folds it, a spelling no type folds from or a type listed twice refused naming the field and leaving it unchanged, an empty list removing the property as `remove_parents()` does; Python and JavaScript `field.fix.parents` cross a list of words, assigning folds each |
| Base | the identifier type the field states: its `FIX:idmap` key, else its folded name |
| Inferred | a dictionary states what its own field names imply, wherever fields arrive - a store loading, a field inserted, updated or added, a dictionary merged: a field named as another's parent - `parent` or `orig` before that field's name, as `IdType::parent_of` reads it - is listed on that field among its `FIX:parents`, a `parent` type before an `orig` one, beside what it already states; a parent field that is its base's only one also answers to its other spelling (`OrigClOrdID` to `parentclordid`) until a field of that name arrives |
| Compiled | `FixRegistry::parent_sources() -> &[(IdType, Box<[IdType]>)]`, every list under its base, compiled once and forgotten by every change to the fields; Python `parent_sources()` answers `{base, parents}` records and JavaScript `parentSources()` the same objects |
| Lookup | `FixRegistry::parents_of(base)` - the stated list, else the ones the name has (`IdType::parents`: `orderid`'s are `parentorderid`, then `origorderid`) - and `parent_of(kind)` - the base `kind` is a parent of and its place among the base's parents, from a stated list first, else the name's reading for a base that states none; Python `parents_of(base)` and `parent_of(kind)` (a `(base, place)` pair or `None`), JavaScript `parentsOf(base)` and `parentOf(kind)` (`{base, at}` or `null`) |
| Shipped | the generator writes them from the dictionary's own field names (`attach_parents` in `scripts/generate_fix_dictionary.py`): `ClOrdID(11)` states `["origclordid"]`, `TradeID(1003)` `["origtradeid"]`, `SecondaryTradeID(1040)` `["origsecondarytradeid"]`, `CrossID(548)` `["origcrossid"]` and `AllocID(70)` `["parentallocid"]`; `OrderID(37)` states none, and has the parents its name has |
| Used by | a message's `Operation::parents_of` and `parent_of` read this registry, so [`follow_parents`](../graph/identifier.md#parentage) and the [lifecycle walk](lifecycle.md#a-chain-is-named-by-its-cross-code) follow the dictionary's lists |

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, FixRegistry, IdType};

    // A field states the parents of its identifier, each folded as a type is.
    let mut orderid = DataType::utf8().nullable_field("OrderID");
    orderid.as_fix_mut().set_tag(37)?;
    orderid.as_fix_mut().set_parents(["ParentOrderID", "origorderid"])?;
    assert_eq!(orderid.get_metadata("FIX:parents"), Some(r#"["parentorderid","origorderid"]"#));
    assert_eq!(orderid.as_fix().parents().collect::<Vec<_>>(), ["parentorderid", "origorderid"]);
    assert!(orderid.as_fix_mut().set_parents(["origorderid", "OrigOrderID"]).is_err(), "a type listed twice");

    // The committed dictionary: ClOrdID(11) states the parent OrigClOrdID(41) is.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root)?)?;
    let parents = |base: IdType| -> Vec<String> {
        registry.parents_of(&base).iter().map(ToString::to_string).collect()
    };
    assert_eq!(parents(IdType::ClOrdId), ["origclordid"]);
    assert_eq!(parents(IdType::TradeId), ["origtradeid"], "OrigTradeID(1126)");
    assert_eq!(parents(IdType::OrderId), ["parentorderid", "origorderid"], "no field: the name's");
    // A stated list places its parents: here the one parent is the first.
    assert_eq!(registry.parent_of(&IdType::OrigClOrdId), Some((IdType::ClOrdId, 0)));
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    # A field states the parents of its identifier, each folded as a type is.
    orderid = Field("OrderID", "utf8")
    orderid.fix.tag = 37
    orderid.fix.parents = ["ParentOrderID", "origorderid"]
    assert orderid.fix.parents == ["parentorderid", "origorderid"]
    assert orderid.metadata["FIX:parents"] == '["parentorderid","origorderid"]'
    with pytest.raises(ValueError, match="FIX:parents"):
        orderid.fix.parents = ["origorderid", "OrigOrderID"]
    assert orderid.fix.parents == ["parentorderid", "origorderid"], "a refusal leaves the field unchanged"

    # The committed dictionary: ClOrdID(11) states the parent OrigClOrdID(41) is.
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert registry.parents_of("clordid") == ["origclordid"]
    assert registry.parents_of("tradeid") == ["origtradeid"]
    assert registry.parents_of("orderid") == ["parentorderid", "origorderid"], "no field: the name's"
    # A stated list places its parents: here the one parent is the first.
    assert registry.parent_of("origclordid") == ("clordid", 0)
    assert {"base": "clordid", "parents": ["origclordid"]} in registry.parent_sources()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Field, fix } = require('yggdryl')

    // A field states the parents of its identifier, each folded as a type is.
    const orderid = Field.from('OrderID: utf8')
    orderid.fix.tag = 37
    orderid.fix.parents = ['ParentOrderID', 'origorderid']
    assert.deepEqual(orderid.fix.parents, ['parentorderid', 'origorderid'])
    assert.equal(orderid.get('FIX:parents'), '["parentorderid","origorderid"]')
    assert.throws(() => {
      orderid.fix.parents = ['origorderid', 'OrigOrderID']
    }, /FIX:parents/)
    assert.deepEqual(orderid.fix.parents, ['parentorderid', 'origorderid'], 'a refusal leaves the field unchanged')

    // The committed dictionary: ClOrdID(11) states the parent OrigClOrdID(41) is.
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    assert.deepEqual(registry.parentsOf('clordid'), ['origclordid'])
    assert.deepEqual(registry.parentsOf('tradeid'), ['origtradeid'])
    assert.deepEqual(registry.parentsOf('orderid'), ['parentorderid', 'origorderid'], 'no field: the name\'s')
    // A stated list places its parents: here the one parent is the first.
    assert.deepEqual(registry.parentOf('origclordid'), { base: 'clordid', at: 0 })
    assert.deepEqual(registry.parentSources().find((source) => source.base === 'clordid'), {
      base: 'clordid',
      parents: ['origclordid'],
    })
    ```

## A field maps its values onto a market data type

A message states what type of its kind it is - a limit order, a tradeable quote, a block trade - through the [`MarketDataType`](../types/enum/marketdatatype.md) its message type's or its kind's typing field reads as: an order's `OrdType(40)`, a quote's `QuoteType(537)`, a trade's `TrdType(828)`, a book entry's `MDEntryType(269)`, a trade capture report's `TradeReportType(856)`, a quote request's `QuoteRequestType(303)`, a mass cancel's `MassCancelRequestType(530)`, a market data request's `SubscriptionRequestType(263)`. The crate reads the standard values of those eight itself (`MarketDataType::from_fix`); a venue's own value - an `OrdType(40)` of `Z` - is a fact about the field, so it travels on the field as `FIX:marketdatatype`: a word list of `wire=MEMBER` pairs, `["Z=ORDPEGGED"]`.

| Contract | Rule |
| --- | --- |
| Key | `FIX:marketdatatype`, read with `FixField::marketdatatypes()` - each item a `(wire, MarketDataType)` pair, a word naming no member passed over - and written with `FixFieldMut::set_marketdatatypes(&[(&str, MarketDataType)])`, an empty list removing it; Python `field.fix.marketdatatypes` crosses `(wire, member)` pairs, a member given as a member, its code or a spelling; JavaScript `field.fix.marketdatatypes` crosses `{wire, marketdatatype}` objects |
| Refuses | a member that names none and one wire value stated twice, naming the field; the field is left as it was |
| Lookup | `FixRegistry::marketdatatype_of(tag, wire)`: this dictionary's mapping first, then the crate's own `MarketDataType::from_fix` - a value no member names reading as its set's catch-all - and `None` for a field that types nothing; Python `marketdatatype_of`, JavaScript `marketdatatypeOf` |
| Compiled | `FixRegistry::marketdatatype_sources() -> &[(i32, SmolStr, MarketDataType)]`, every field's pairs beside its tag, compiled once and forgotten by every change to the fields; Python `marketdatatype_sources()`, Rust and Python only |
| Any field | the mapping is not limited to the eight typing fields: a dialect's own field may state it too, though a parse reads first the fields its message type or kind names ([`fix_tags_of`](../types/enum/marketdatatype.md#fix)) |
| Parse | a [message](message.md) states its `marketdatatype` as it is built, through its registry's `marketdatatype_of` over the first of its `fix_tags_of(msgtype, kind)` it states, then any other field the registry maps, `UKNW` where it states none |
| Book entries | a `W` or `X` entry is a bid, an offer or a trade by the member `marketdatatype_of(269, ..)` reads its `MDEntryType(269)` as - `BOOKBID`, `BOOKOFFER`, `BOOKTRADE` - so a mapping on `MDEntryType` books a venue's own entry type; any other member excludes the entry ([Message](message.md#market-data)) |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Market;
    use yggdryl::{DataType, FixCodec, FixRegistry, MarketDataType};

    let mut ordtype = DataType::utf8().nullable_field("OrdType");
    ordtype.as_fix_mut().set_tag(40)?;
    ordtype
        .as_fix_mut()
        .set_marketdatatypes(&[("Z", MarketDataType::OrdPegged)])?;
    assert_eq!(ordtype.get_metadata("FIX:marketdatatype"), Some(r#"["Z=ORDPEGGED"]"#));

    let mut registry = FixRegistry::new();
    registry.insert(ordtype)?;
    // The dictionary's own value first, then the crate's reading of FIX's.
    assert_eq!(registry.marketdatatype_of(40, "Z"), Some(MarketDataType::OrdPegged));
    assert_eq!(registry.marketdatatype_of(40, "2"), Some(MarketDataType::OrdLimit));
    assert_eq!(registry.marketdatatype_of(40, "Y"), Some(MarketDataType::OrdOther));
    assert_eq!(registry.marketdatatype_of(54, "1"), None);
    assert!(registry.marketdatatype_sources().iter().any(|(tag, wire, member)| {
        *tag == 40 && wire == "Z" && *member == MarketDataType::OrdPegged
    }));

    // A parse types the order by it.
    let codec = FixCodec::new(Arc::new(registry));
    let message = codec
        .parse_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=Z|38=5|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(message.get_marketdatatype(), MarketDataType::OrdPegged);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field, MarketDataType
    from yggdryl.fix import FixCodec, FixRegistry

    ordtype = Field("OrdType", "utf8")
    ordtype.fix.tag = 40
    ordtype.fix.marketdatatypes = [("Z", "ordpegged")]
    assert ordtype.fix.marketdatatypes == [("Z", MarketDataType.ORDPEGGED)]
    with pytest.raises(ValueError):
        ordtype.fix.marketdatatypes = [("Z", "not a type")]

    registry = FixRegistry()
    registry.insert(ordtype)
    assert registry.marketdatatype_of(40, "Z") is MarketDataType.ORDPEGGED
    assert registry.marketdatatype_of(40, "2") is MarketDataType.ORDLIMIT
    assert registry.marketdatatype_of(54, "1") is None
    assert (40, "Z", MarketDataType.ORDPEGGED) in registry.marketdatatype_sources()

    message = FixCodec(registry).parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=Z|38=5|10=0|")
    assert message.marketdatatype is MarketDataType.ORDPEGGED
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const ordtype = Field.from('OrdType: utf8')
    ordtype.fix.tag = 40
    ordtype.fix.marketdatatypes = [{ wire: 'Z', marketdatatype: 'ORDPEGGED' }]
    assert.equal(ordtype.get('FIX:marketdatatype'), '["Z=ORDPEGGED"]')
    assert.throws(() => {
      ordtype.fix.marketdatatypes = [{ wire: 'Z', marketdatatype: 'nope' }]
    }, /marketdatatype/)

    const registry = new fix.FixRegistry()
    registry.insert(ordtype)
    assert.equal(registry.marketdatatypeOf(40, 'Z'), 'ORDPEGGED')
    assert.equal(registry.marketdatatypeOf(40, '2'), 'ORDLIMIT')
    assert.equal(registry.marketdatatypeOf(54, '1'), null)

    const codec = new fix.FixCodec(registry)
    const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=Z|38=5|10=0|'))
    assert.equal(message.marketdatatype, 'ORDPEGGED')
    ```

## A field maps its values onto a time in force

How long an order stands is a [`TimeInForce`](../types/enum/timeinforce.md) member, read the same way: `TimeInForce(59)` through the crate's own `TimeInForce::from_fix`, and a venue's own spelling - of `TimeInForce(59)` or of a field of its own - through `FIX:timeinforce` on that field, a word list of `wire=MEMBER` pairs, `["D=DAY","G=GTC"]`. The contract is the market data type's above, key for key; the examples are on the [TimeInForce page](../types/enum/timeinforce.md#a-dictionary-maps-a-venues-values).

| Contract | Rule |
| --- | --- |
| Key | `FIX:timeinforce`: `FixField::timeinforces()` and `FixFieldMut::set_timeinforces(&[(&str, TimeInForce)])`; Python `field.fix.timeinforces` (`(wire, member)` pairs), JavaScript `field.fix.timeinforces` (`{wire, timeinforce}` objects) |
| Lookup | `FixRegistry::timeinforce_of(tag, wire)`: this dictionary's mapping first, then `TimeInForce::from_fix` for `TimeInForce(59)` alone - a value no member names reading as `OTHER` - and `None` for any other field; Python `timeinforce_of`, JavaScript `timeinforceOf` |
| Compiled | `FixRegistry::timeinforce_sources()`, every field's pairs beside its tag; Python `timeinforce_sources()`, Rust and Python only |
| Parse | a [message](message.md) states its `timeinforce` as it is built: `TimeInForce(59)` through `timeinforce_of`, else the first other field the dictionary maps a stated value of |

## One merge, with a rule per key

`update` on a scalar merges the same identifier: incoming scalar metadata wins, aliases and alternate tags combine under native validation, and canonical spelling is retained. `update` on a component or a group instead replaces the entire supplied definition while preserving its identity.

| Metadata | Merge rule |
| --- | --- |
| `FIX:tag` | Must agree; identity is not merged |
| `FIX:sources` | Union, folded, sorted: every source that contributed either side |
| `FIX:tags` | Combine alternate tags under collision validation |
| `FIX:codeset` | The stored name wins: a field keeps the set it already reads by, and the incoming field's set has already been folded into it, so the field is never moved to a vocabulary holding less than the one it read by |
| `FIX:directions` | Incoming wins whole: a rule table is one statement, and two tables have no order between them |
| `FIX:identifiers` | Incoming wins whole, then resolves against the final component's members into their canonical order; an omitted key preserves the stored declaration |
| Other protocol keys | Incoming wins; preserve keys only the stored field declares |
| Generic description, display, comment, aliases | The generic metadata merge accompanies the protocol merge |

`FixFieldMut::merge_with` owns the protocol half; `FixRegistry::update` also merges generic metadata and updates indexes. A refusal changes nothing, and an update adding nothing leaves the definition unchanged. The members a field reads by are not on the field and so are not in this table: they fold in `codesets`, before the field is looked at, which is what lets the stored name win without losing anything the incoming set declared.

## Folding a second source in

Rust and Python expose `merge_with`, `add_fields`, `add_cfb_file`, `add_cfb_files` and `add_json_file` as one native fold each, staged and adopted whole: nothing lands until the whole call has folded, a datatype the source states at another precision of the stored one folds under it, and a contradiction is passed over rather than ending the fold - see below for which is which. `FixRegistry::from_cfb_file(location, dialect)` in all three languages returns the imported registry and its declared roots, including canonical scalar metadata, named groups/components/messages, and the code sets its fields read by - a CBlock names no set of its own, so each is filed under the name the field supplies, `hedgecurrencycodeset` for `HedgeCurrency` - and stamps every field, group, component and message the file produces - standard tags included - as a member of `dialect` in its `FIX:sources`, recording the catalog entry `dialect` names - the file's name, and the role the root element's `type` attribute names, a [`PluginSide`](../types/enum/pluginside.md) read by `PluginSide::from_plugin_type`; `None` stamps nothing and records no entry, the role read past with it. The root element's `fix-version`, `sendercompid` and `targetcompid` are read past: the version a capture is read at is the row's own `beginstring` where the transport states one, else what the line implies. The [CLI](cli.md) exposes ingestion and synchronization.

A `vocabulary-tag`'s `alt` names its tag where it names only that tag. A dialect that spells one `alt` over two tags - `TRTN_FX_TradeCapture` declares `HedgeCurrency` for the currency a hedge settles in and again for the one it is quoted in - has given a name to neither, and a tag whose `alt` is another tag's own decimal has done the same to that tag's identity. The file's other statement of what a tag is called is read first: a `normalization-binding` spelling one of them by a name of its own names it - `LEGLASTSPOTRATE` for a tag 5190 whose `alt` repeats tag 637's `LegLastPx` - and the contended spelling, then one tag's alone, stays with that tag. A tag the bindings leave unnamed too falls back to its own decimal, the name a tag declaring no `alt` takes as well, and keeps the declared spelling as `display`, so every tag is left named and nothing the file said is lost. A field named by its own decimal is unnamed, and the [fold](#what-one-namespace-means-for-a-field-that-arrives) reads it so: a later file naming that tag names the field, and the members of both files read one field. Contention is decided by the key the spelling's catalog name, below, is indexed under, which folds case and drops `_`, `-` and space, so `Hedge_Currency` and `Hedge Currency` contend with `HedgeCurrency`. Two tags sharing a spelling record each other's tag among their alternate tags and so stay reachable as a pair; three record nothing, because an alternate identifier names one field. A `normalization-binding` cannot spell a contended name back onto one of them, and a `map` naming one decodes neither. The spelling survives where the file made it unambiguous: a `tag-constraint` binds one tag, so the message root, the component and the group each carry it, and a reader resolving a key against the message it arrived in - a bridge row's `MSGTYPE`, and the repeating group the key sits in - reaches the tag the file meant.

A CBlock's `normalization-binding` is read for the names it spells its tags with, and for nothing else. A `tag-normalization` whose mapping is one bare `$602` says its `tag-name` is another spelling of tag 602, so the spelling joins that tag: as its name where the vocabulary left the tag unnamed - declaring no `alt`, or an `alt` another tag also declares - and every binding of the tag agrees on one spelling that no other tag's `alt`, decimal or binding claims, the spelling as written kept as `display` (`LEGCFICODE` names a tag 608 declared with no `alt` `legcficode`); as an alias otherwise, the `vocabulary-tag` keeping the name it gave. A conditional mapping, a `lookup`, and a mapping built from several expressions each name nothing: this layer holds no evaluator. Most of a real binding spells names a tag already answers to - resolution folds ASCII case - so the pass pays where a `vocabulary-tag` declared no `alt`: the binding's spelling is then the tag's name, and only a tag the bindings leave unnamed too is named by its own decimal. No name is refused: one the vocabulary never declared, one another tag already answers to, or one the core could not store drops on its own.

A CBlock spelling - a `vocabulary-tag`'s `alt`, a normalization's `tag-name`, an `rg-name` or a counter's spelling - is filed under its catalog name: lower case, every run of characters outside `[a-z0-9_.-]` one `_`, `_` trimmed at both ends, so `OTC Trade Flags` is `otc_trade_flags` and `(BloombergCustomTag05)` is `bloombergcustomtag05`. The spelling as written stays the `display`; a spelling nothing of which folds leaves a tag named by its decimal, and a group and its occurrence named after their counter (`{counter}grp`, `{counter}component`). Every name a message binds is therefore one a reference can point at, and a message binding such a field keeps it.

A `map` decodes a tag's values into the code set its field reads by, one `entry` per code, oriented the way the map's name says. An entry whose key or value is empty, blank, `none` or `null` - trimmed, under the crate's one fold of case, `_`, `-` and space - states no code: it is skipped, never enters the set and never claims a name, and the skipped entries are named in one warning per file and code set, listing up to 16 of them and then `and N more`. A second name for a value already held is an alias of it; an entry whose name another code already claims - by a lookup, or by the rule rendering holds names to - is dropped, so one entry costs that entry rather than the whole map. A held code named or valued by such a word is kept, because the value is a fact about the wire: `EncryptMethod(98)` `0` is `None`, and `NONE` is a wire value `SecurityType(167)` and `DateRollConvention` state.

`merge_with` combines another registry under the [fold table](#what-one-namespace-means-for-a-field-that-arrives), its code sets folded first, its named definitions folded member by member and each field's membership unioned - the sets lead because a field keeps the set it already reads by, so the members the other dictionary states have to be in that set by the time the field is folded, and a merge therefore widens a vocabulary and never narrows one; `add_fields` folds a scalar field iterable the same way and answers its `(added, merged)` pair. `add_cfb_file(handle, dialect)` parses a CBlock and merges it, stamping the dialect - or, with none supplied, the file's stem where it reads as an id: opening with an ASCII letter and holding no quote, backslash or control character, its percent escapes decoded, so `Morgan Stanley.cfb` stamps `morgan stanley`, while a stem that does not stamps nothing - on everything the file produced and recording its catalog entry; a supplied id that is empty or holds a quote, a backslash or a control character is refused.

`add_cfb_files` is the plural, and it **takes the locations alone**: each is a [`Holder`](../holder/index.md), and what it is decides what it holds. A **glob** - `cblocks/*.cfb`, `cblocks/**/*.cfb`, `**/venue-*.cfb` - holds every file its pattern matches, walked exactly as [`IOBase::glob`](../holder/index.md) walks it: `*` inside one name, `**` across folders, private entries never matched, a container it matches passed by. A **folder** holds the `.cfb` files directly inside it, the suffix in any case. A **file** holds itself, whatever it is named, and a location where nothing is holds nothing. So a folder of counterparty files is `registry.add_cfb_files(&[Holder::local("cblocks")?], None)?`, several locations are one call - one fold, one resolution - and a file reached twice folds once. Python's plural is `add_cfb_files(location, dialect=None)`, `location` a path, a URL or an `IOBase` that is a file, a folder or a glob. **Files fold in ascending URL order** whatever order they were listed in, because the fold's precedence is its input order and a listing's sequence is not a caller's to see: it varies with how a pattern decomposed and with the backend beneath. So where two files disagree about one tag **the first-sorting file's declaration is held**, the later one is passed over, and `cblocks/`, `cblocks/*.cfb` and `**/*.cfb` over the same files all answer the same dictionary. **Files parse on every core, and fold on one:** a parse reads nothing but its own bytes, so the files are read in order and parsed side by side, at most one file per thread in hand; each parsed file then folds into the one staged dictionary in URL order, and the catalog resolves once for all of them - a hundred files cost one resolution, not a hundred. `dialect` is resolved per file: a name supplied here stamps every file with it, and none lets each file's own stem stand in, which is what globbing a folder of counterparty files is for - `cblocks/*.cfb` with none supplied stamps `msfix44` and `blpfix44` from `MSFIX44.cfb` and `BLPFIX44.cfb`'s own stems.

`add_json_file(handle)` is the same door for a [JSON snapshot](store.md) and takes no dialect, because a snapshot is the crate's own format and every field and definition in it already carries the `FIX:sources` its writer meant, and its `sources` array folds into the catalog through `add_source`.

`merge_with`, `add_cfb_file`, `add_cfb_files` and `add_json_file` each answer a `FixMerge`, [below](#what-a-source-says-otherwise-than-the-dictionary-is-passed-over): `sources` counts the dictionaries or files folded - the file count is a fact only the plural calls hold, since an empty match and a match whose files all merged into stored fields both answer zero `added` and `merged` - `added` and `merged` the scalar fields over every source, `restated` among the merged those whose source declared another precision of the stored datatype and folded under it, `dropped` what was passed over in the order the fold met it, `failed` the files `add_cfb_files` left out whole - one `FixFailure` each, its `source` URL and the `reason` it was left out over, displayed `{source}: {reason}` - and `is_clean()` whether `dropped` and `failed` are both empty. `merge_with`, `add_cfb_file` and `add_json_file` are one mutation each: what leaves nothing to keep - a file that will not parse, an incoming dictionary whose own catalog does not validate - leaves the dictionary exactly as it was, and the refusal names the source.

**In `add_cfb_files` one file is one mutation, and one bad file is one file.** Each file folds into the staged dictionary as a mutation of its own: a file that cannot be read, is not a well-formed CBlock, or whose fold refuses rather than passing a declaration over is rolled back alone - the staged dictionary is exactly what the files before it left - and named in `failed`, while every other file still folds. Nothing is adopted until the last file is in and the catalog resolves, so a caller sees every folded file's contribution or none of the call's. The resolution is the one place two files' contributions can refuse each other; where the union does not resolve, the files fold again one at a time, each resolved before the next, so the file the union cannot hold is the one left out by name and the rest are still the dictionary. A listing that fails - a location's or one entry of it - refuses the call before any file is parsed, so a listing that fails part way is a refusal rather than a half-read dictionary. Python answers `failed` as a list of `{"source", "reason"}` mappings.

A CBlock is read for what it says. A real one is megabytes over hundreds of thousands of elements, so an element this reader cannot make sense of - a tag spelled in a way the core cannot store, a mapping to a type nothing listed, a nested grammar with no counter - is dropped and the rest of the file is still a dictionary. A constraint naming a tag the file's own vocabulary never declared is not dropped: it declares the tag as text named by its digits, so the message keeps the member and a file that names the tag names the field when the two fold. Only a document that is not well-formed XML, or that stops with an element open, is refused, because neither leaves anything to keep: `from_cfb_file` and `add_cfb_file` raise it as a native located error wherever they are bound, and `add_cfb_files` leaves that file out, named in `failed`.

A `vocabulary-tag` types itself with one of eight words - `string`, `char`, `integer`, `float`, `boolean`, `utc-date`, `utc-timestamp`, `utc-time-only` - and seven resolve through the schema grammar and its FIX logical names exactly as a datatype expression would, folding case and separators: `string` and `char` are `utf8`, `integer` is `int32`, and `utc-date` is `utcdate`, that day's midnight in UTC. `float` is read for itself as `float64`: the grammar reads that word as SQL does, a 32-bit float, where a CBlock means the FIX family `Qty`, `Price` and `Amt` derive from, which states no width. A word outside the eight resolves through the same grammar - `LocalMktDate`, `MonthYear`, `data` or a datatype name outright reads as what it names - and a word nothing reads types the tag as `utf8` with a warning: the tag is still on the wire, where every FIX datatype is text, and dropping it would take every constraint naming it out of every message. The eight words are coarse, and the fold reads them so - [restated](#what-a-source-says-otherwise-than-the-dictionary-is-passed-over) under the finer declaration a dictionary holds rather than passed over.

The document crosses the charset boundary once, before the XML reader sees a byte, in the charset it states: a byte-order mark, else the declaration's `encoding`, else UTF-8. Real exports declare `US-ASCII`, `UTF-8` and `ISO-8859-1`, so an `é` in a `description` is the character rather than a refusal of the file; an all-ASCII document is borrowed, and a refusal's byte is a position in the decoded text, the document's own wherever it was ASCII. Bytes the declared charset cannot read are transcribed - every valid run kept, a stray byte read as the Windows-1252 character it is - and a charset the crate has no table for is read as UTF-8, each with one warning. An attribute is one attribute: one the reader cannot split into a key and a quoted value is dropped and the element keeps the rest, one whose value will not unescape - `S&P 500`, an HTML entity - keeps the value as the file spelled it, one an element states twice is read once, the first statement standing, and a bare `&` in a description's text is the character.

Each drop is a `log` record at warn level carrying the located sentence a refusal would have, then what the reader did about it: `invalid cfb expression at byte N: line L, column C: expected X, got Y in "<element ...>"; <consequence>` - the byte the reader had reached, the line and column it falls on, what was expected, what arrived and the element quoted as the file spells it, then, after the semicolon, the consequence: `the declaration is dropped`, `the tag is typed string, which every FIX datatype is on the wire`, `the value is kept as the file spelled it`, `the attribute is dropped and the element keeps the rest`, `the constraint is dropped and the message keeps its other members`, `the constraint declares the tag as text, named by its digits`, `the group is dropped and the message keeps the rest`, `the member is dropped and the message keeps the rest`, `the message is dropped and every field it declared kept`, `the tag keeps no code set`, `an entry spelling nothing states no code, so each is skipped`. The last is one record per code set however many maps and entries it covers - `tag 39 "OrdStatus", code set ordstatuscodeset: 1 map entry spelling none, null or nothing: "ORDSTATUS" key "none" value "8"; an entry spelling nothing states no code, so each is skipped` - its list bounded at 16 entries, then `and N more`. A parse reading a located file prefixes the file's name, and one stamping a dialect the dialect - `venue.cfb [venue] invalid cfb expression at byte 204: ...` - which names the file among the many a glob reads side by side. A drop the core raised rather than the grammar - a description or a spelling that cannot be stored, two declarations of one tag - carries the core's own sentence behind the declaration that asked for it. Every quoted span is bounded, so a warning never grows with the file.

A message whose grammar states a NumInGroup counter beside the nested grammar that counter opens is the vendor's contradiction, not a fold's: the message is dropped with every field it declared kept, every other message still binds, and the warning names the line and column of its `grammar-binding`.

What the file states twice is not a drop. A type its listing and one of its bindings both declare, a grammar bound under a wire type another grammar already bound, and a member the held message already carries are each what a dialect looks like: the declarations fold, the members union - the held ones first in their order, then every member only the later declaration states - and the fold is a `log` record at info level rather than a warning. A second grammar-binding of one wire type folds into the first under the [merge rules](#what-a-source-says-otherwise-than-the-dictionary-is-passed-over): two references to one group on one counter fold their members together, a group on another counter stands beside, and each member that still disagrees is dropped with a warning - `message "D", bound again: <reason>; the member is dropped and the message keeps the rest` - while the rest of the binding folds. A group or component the first binding shares with another message is widened for both, as a fold across dialects widens it, and the splits the second binding wrote that nothing reads once it folded are taken back. A definition the second binding widened into the structure of another the file already registered is that definition: the widened one keeps its name and the other folds into it, whichever of the two messages the file binds first. Warn stays reserved for what is actually lost.

A CBlock that binds a group or a component structurally alike to one this parse already registered - the same members in the same order, each its tag and the field it reads, a nested group its counter over the same structure - has declared that definition, whatever it named it and however it stated it: the member names, a `required`, the `rg-name`'s spelling, a description and the sources are no part of the structure, so the held definition's name and spelling hold, a member one grammar requires and another does not is nullable in it - whichever of the two a message states first, every member of the message reading the definition as it is once the message is read - and the later grammar's member reads the held definition under the member name its own grammar gave it. One it binds under a name already registered with other members, alike to nothing held, is naming a second definition rather than restating the first: the split reads under `{name}_{message}`, `message` the name the message is catalogued under - tag 35's own code-set name lower-cased (`underlying_newordersingle`), or, where nothing names the type, `message` and the wire value's own bytes in hex (`underlying_message4145`). A message declaring one name in several shapes - its parties at the root and again inside its legs, or two bindings of the type stating it two ways - takes a split per shape in declaration order, `{name}_{message}`, then `{name}_{message}_2`, `_3`. Two messages that bind one group or component alike share the one definition. The split never touches the member's own name: a message's row still reads `underlyings` whichever definition its `FIX:group` or `FIX:component` reference names. A message the catalog will not hold takes back the groups and components its walk wrote, so no definition is left that nothing reads.

### What a source says otherwise than the dictionary is passed over

A fold of another source is not all-or-nothing the way a single [`insert`](#insert-update-and-remove) is: what the source says otherwise than the dictionary already says is named in `FixMerge::dropped` - one `FixDrop` per declaration: an `incoming` `Field` (the declaration as the source stated it - a scalar, or a member or a definition in the compact shape a [store](store.md) writes), a `reason` naming what the dictionary keeps, and a `source`, the file's URL where the fold read one - while the rest of the source still folds. Seven kinds are passed over rather than raised:

1. a scalar whose datatype contradicts the one stored for the field its identity (tag and folded name) or its folded name reaches - merging never changes a declared datatype, save a group's counter, below;
2. a scalar whose spelling or alternate tag another field already holds;
3. a member a held definition already declares in another shape;
4. a group whose counter this dictionary holds as a contradiction of a count, or whose counter the fold itself passed over, or which disagrees with the group held under its name on that group's own counter - a group on another counter stands beside, below - together with every member that reads that group;
5. a member reading a field the fold passed over, which nothing here now answers to under the identity it named;
6. a code set that does not fold - a document that does not parse, or one that would rewrite a set this crate owns, `msgcatcodeset`, `marketdatatypecodeset`, `statecodeset` or `msgpluginsidecodeset` - named by the set and the field that reads by it: the held set stays as it was;
7. a definition whose fold refuses rather than passing a member over: one definition is one mutation, so it is passed over whole, every write its fold made undone. A definition that arrives new and goes takes the source's members reading it along - a group's always, a component's where nothing held answers its name.

A datatype at another precision of the stored one is no contradiction: the field folds under the stored declaration - membership, aliases and code set included - and is counted in `FixMerge::restated`. Every FIX datatype derives from String and the numeric families state no width, so unbounded text restates anything, whichever side holds it; the integer, floating and decimal families restate each other; an integer restates an enum leaf; the byte layouts restate one another; a date restates a datetime whatever the zone; and one time width restates another. A CBlock's `float` against a stored `decimal128(38, 18)`, its `string` against `ccy`, `mic`, `side`, `binary` or `fixed_ascii(8)`, its `integer` against `int64` and its `utc-date` against a zone-less date each fold under the held declaration; a `boolean` against a stored `int32`, a time of day against an instant, `ccy` against `mic`, or two bounded strings of different bounds contradict.

A time of day against an instant is the contradiction two FIX spellings of one field routinely make - a CBlock has `utc-time-only` and no word for `TZTimeOnly`, so its `MaturityTime(1079)` meets the dictionary's `datetime64(ns,"UTC")` - and it stays passed over, its reason saying why: `expected the datatype datetime64(ns,"UTC") stored for maturitytime (1079), got time64(ns): the stored instant reads a clock only with its offset, a bare time of day as null`; the reverse ends `the stored time of day reads no date and no offset`.

**A counter is the one exception to merging never changing a declared datatype.** A field a source counts a group by - NumInGroup, an `int32` - that the dictionary holds as unbounded text, a float or another integer width is retyped `int32`, logged at warn level - `tag 8070 counts a repeating group, so nocustflds is retyped int32 from float64` - the rule a CBlock's own parse applies to a nested grammar's counter inside one file, so the dictionary is the same whichever file sorts first. A value another dialect sends in that tag that is not an integer reads as null from then on. A decimal, an enum, a code, a flag or a temporal is a contradiction rather than a coarser count: the field stays as held, and the group is passed over and named. Only the field holding the tag - its own, or the one its name reaches on another tag - is retyped. A tag the dictionary reads as another field's alternate is that field spelled with another number: where the field is a count, a group counted by the tag is counted by it, and where it is anything else the group is passed over and named (`expected tag 9001, counting a repeating group, to spell a count, got the alternate tag of text (58), held as utf8`), the field it spells left as it was and the group never moved onto that field's own tag.

`add_field`, `add_fields`, `insert` and `update` are unaffected by any of this and stay strict: met alone, outside a fold, any datatype difference is still refused rather than restated or passed over. A code set's spelling collision never refuses a source: a code arriving with a new value under a name another code already claims keeps its value under no name while the first keeps the spelling, logged at warn level. A fold refuses the whole source, adopting nothing from it, only where nothing is left to keep: malformed XML or JSON, an incoming dictionary whose own catalog does not validate, or a folded catalog that does not resolve.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry};

    let mut price = DataType::Float64.nullable_field("Price");
    price.as_fix_mut().set_tag(44)?;
    let mut held = FixRegistry::from_fields([price])?;

    // One source types tag 44 as text, which every FIX datatype is on the
    // wire: another precision of the price, folded under it.
    let mut text = DataType::utf8().nullable_field("Price");
    text.as_fix_mut().set_tag(44)?;
    let merge = held.merge_with(&FixRegistry::from_fields([text])?)?;
    assert_eq!(merge.restated, 1);
    assert!(merge.is_clean());

    // Another types it as a flag, and brings a field of its own.
    let mut flag = DataType::Boolean.nullable_field("Price");
    flag.as_fix_mut().set_tag(44)?;
    let mut side = DataType::utf8().nullable_field("Side");
    side.as_fix_mut().set_tag(54)?;
    let other = FixRegistry::from_fields([flag, side])?;

    // The held declaration stays: the contradiction is named and passed
    // over, and Side still folds in beside it.
    let merge = held.merge_with(&other)?;
    assert_eq!((merge.added, merge.restated), (1, 0));
    assert!(!merge.is_clean());
    assert_eq!(merge.dropped.len(), 1);
    assert!(merge.dropped[0].reason.contains("float64"));
    assert_eq!(merge.dropped[0].incoming.dtype(), &DataType::Boolean);
    assert_eq!(held.field_by_tag(44)?.dtype(), &DataType::Float64, "a contradiction leaves the held datatype");
    assert_eq!(held.field_by_name("Side")?.dtype(), &DataType::utf8());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field
    from yggdryl.fix import FixRegistry

    price = Field("Price", "float64")
    price.fix.tag = 44
    held = FixRegistry.from_fields([price])

    # One source types tag 44 as text, which every FIX datatype is on the
    # wire: another precision of the price, folded under it.
    text = Field("Price", "utf8")
    text.fix.tag = 44
    report = held.merge_with(FixRegistry.from_fields([text]))
    assert report["restated"] == 1
    assert report["dropped"] == []

    # Another types it as a flag, and brings a field of its own.
    flag = Field("Price", "boolean")
    flag.fix.tag = 44
    side = Field("Side", "utf8")
    side.fix.tag = 54
    other = FixRegistry.from_fields([flag, side])

    # The held declaration stays: the contradiction is named and passed
    # over, and Side still folds in beside it.
    report = held.merge_with(other)
    assert (report["added"], report["restated"]) == (1, 0)
    [dropped] = report["dropped"]
    assert "float64" in dropped["reason"]
    assert dropped["incoming"].dtype == DataType("boolean")
    assert held.field(44).dtype == DataType("float64"), "a contradiction leaves the held datatype"
    assert held.field_by_name("Side").dtype == DataType("utf8")
    ```

Node has no binding over these folds; a JavaScript reader runs [the CLI](cli.md) instead.

Two references under one member name to two groups or two components are one member read two ways rather than a conflict: a dialect that split a group or a component differently for one message - one file drawing `Parties` from one component, another from a second, on the same counter - has still named one member, so the incoming target's members - as the source states that target, whichever of the two folds first - fold into the held target under these same rules, recursively; a group held under its name and counter that a source draws from another component widens the held component the same way. Two groups the fold meets on different counters are not this case: two counters are two tags on the wire, so they are two definitions, and the incoming group stands beside the held one under `{name}_{counter}` - a venue's `dealers` on counter 7101 is `dealers_7101` beside the held `dealers` - while a message member reading it stands beside the held member as `{member}_{counter}` rather than being passed over - once per counter, so a member already standing there reading that group is that member again, and folding one source twice adds nothing the second time. An incoming member no held member answers to by name, reading a group on a counter a held member already reads, is that member read another way - a dialect naming the group after its own `rg-name` - and folds into it rather than standing beside it as a second member of one counter. A member is the field it reads before the name it carries: an incoming member reading another field than the held member of its name - a spelling two tags share, `Urgency` over 61 in one file and over 9252 in another - is the held member reading that field where one does, and a member of its own beside the held ones under `{name}2` otherwise, two tags being two tags on the wire; so nothing is passed over, and the message reads every tag either file states, whichever file folds first.

One structure is one definition. Every definition of the source folds by name first, under the rules above. Once every one has, each pair of definitions of one structure the fold made - the same members in the same order, each its tag and the field it reads, a nested group its counter over the same structure, whatever either is named - folds into one: an arrival stating the structure of a definition the dictionary held before the fold, or a held definition the fold widened until it states another's. The definition kept is one the dictionary held - the first in name order that the fold widened, else the first in name order - so a name a source's merge by name widened keeps answering for its structure, and folding the same source again changes nothing. It is relaxed to the more permissive nullability of each member, its own nullability staying as held, lists both sources, and every member and group that read the other reads it. A split one dialect took for a message folds this way: the fold widens the group that message reads by the split's members, the split then states its structure, and it folds into it - so two dialects stating one group, one requiring a member the other does not, fold to one group with the member nullable. Two definitions the dictionary held as they were stay two, and so do two one source brought unless a definition held before the fold states their structure, which both fold into: a dictionary stating two definitions of one structure - FIX Latest's `InstrmtLegSecList` beside `SecLstUpdRelSymsLeg` - states two, and a dictionary folded into an empty one is that dictionary. Which name a structure is filed under, and the order of its members, are the first source's, as every name a fold keeps is; a component stating no member states no structure and folds into none. A definition folded away is a name the dictionary no longer holds, so a source stating that name with fewer members than the structure it folded into - the source whose fold folded it away included - lands its narrower definition under the name again when it folds again, read by no member already folded, and the fold after that changes nothing: a file reading `RoutingIDs` as 217 alone and `Routes` as 216 and 217, folded where another file held `RoutingIDs` as 216 and 217 and `Routes` as 216, widens `Routes` into `RoutingIDs`' structure and folds `RoutingIDs` into it, and folding that file again lands its one-member `RoutingIDs` once more. Where a dictionary holds `LegA` and `LegB` alike as tag 600 alone and one fold widens both by 601, `LegB` folds into `LegA`, and folding the first dictionary again lands its one-member `LegB`. A fold widening that name again makes the pair anew, and the name kept is then the one that fold widened - `LegB` - so while the two sources alternate, the two names take turns.

A group's counter is read through the same rewrite as its members: a counter that merged by name onto a field held under another tag - `NoRegulatoryTradeIDs` on 20001 folding onto 1907 - counts the held group, which then folds.

A member reading a field that merged into a held field by folded name under another tag is rewritten to the held field's identity as the source folds, so nothing that reads it afterward is passed over for naming a tag that moved. So is a member reading a field passed over on a tag the dictionary holds - an unnamed field, or one whose datatype contradicts: it reads the held field rather than one nothing holds, under the name that field holds once every scalar has folded, so a holder a later declaration of the same source names is read under that name. Two members of one struct that read one field after that rewrite - a custom tag spelled `OrderID` beside an unnamed tag 37, both landing on `OrderID(37)` - keep wire order under `{name}`, `{name}2`, the rule a duplicate constraint already follows. A message whose name another wire code already holds takes the name its own wire value derives - `message` and the wire bytes in hex, so `U8` becomes `message5538` - the same name a CBlock binds when nothing spells the type a store can file. A new definition whose derived tag another definition already holds derives a free one instead.

## Registering a message type

`MsgType` has no public constructor and is not a generic datatype or scalar. It is one immutable registry-owned message Struct; lookup accepts an exact case-sensitive wire code, a folded name, or an alias tag 35's code set gives the code. A wire code is read exactly wherever it is read, and that is the whole of the case rule: tag 35 carries `b` for MassQuoteAcknowledgement beside `B` for News, `c` beside `C`, `d`, `g`, `h`, `j`, `q`, `r` and `s` beside their own capitals, so a dialect stating both cases of a letter states two messages and each keeps its own grammar, its own wording and its own entry. The fold reaches every spelling around a code and never the code itself - `MassQuoteAcknowledgement`, `mass_quote-acknowledgement` and `b Inbound` all reach `b`, while `B` reaches only `B`. Message codes live in one map under the rule fields follow: a definition re-declaring a code under the same folded name folds into the stored one, and under another name it is a second message reached by its name. A dialect that spells a type as a wire value and a qualifier - an Ullink CBlock's `6 Inbound` and `6 Outbound` - has stated tag 35 `6` twice and named it neither time, so the code takes the wire value as its name and every declared spelling as an alias, and that placeholder name yields to a real one the moment the dialect folds into a dictionary that has one. One wire type is one message: the second grammar bound under it folds into the first, keeping its members in order and appending every member only the second declares. The bare code answers the message named as tag 35's code set names the code, else the first in name order - facts of the catalog's content rather than of the order it was built in, so a dictionary folded, stored and loaded answers the same message. An alias spelling that two codes share names nothing.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry};

    let mut field = DataType::utf8().nullable_field("MsgType");
    field.as_fix_mut().set_tag(35)?;
    let mut registry = FixRegistry::from_fields([field])?;
    let message = registry.register_msgtype("P Report Ack", Some("AllocationReportAck"), None)?;
    assert_eq!(message.as_str(), "P Report Ack");
    assert_eq!(message.as_field().as_fix().msgtype(), Some("P Report Ack"));
    assert_eq!(registry.msgtype("allocationreportack")?.as_str(), "P Report Ack");
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    field = Field("MsgType", "utf8")
    field.fix.tag = 35
    registry = FixRegistry.from_fields([field])
    message = registry.register_msgtype("P Report Ack", "AllocationReportAck")
    assert str(message) == "P Report Ack"
    assert message.field.fix.msgtype == "P Report Ack"
    assert registry.msgtype("allocationreportack") == message
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const field = Field.from('MsgType: utf8')
    field.fix.tag = 35
    const registry = fix.FixRegistry.fromFields([field])
    const message = registry.registerMsgtype('P Report Ack', 'AllocationReportAck')
    assert.equal(message.asStr(), 'P Report Ack')
    const projected = message.asField()
    projected.setName('IndependentCopy')
    assert.equal(message.name, 'allocationreportack')
    assert.equal(message.compare(message.clone()), 0)
    ```

Registration states the set tag 35 reads by - the one the field names, else `msgtypecodeset`, [derived](#a-field-names-the-code-set-it-reads-by) from the field and pointed at by it in the same mutation - and, if no message owns that code, creates an empty component carrying it in `components`. Codes may contain spaces and have no artificial width limit; empty text and control characters are refused. `new()` seeds no message type: every code a registry answers is one a dictionary or a caller registered. Python's `message.field` is read-only, while Node `asField()` returns an independent mutable projection that cannot alter the singleton.

## One default registry per process

The first call resolves one shared default: an explicitly installed registry, then `YGGDRYL_FIX_REGISTRY`, then `LocalFolder::config()/fix`, then `FixRegistry::new()`: 39 crate scalar fields and one Map group beside the two seeded clocks, so `len()` is 42. A configured environment location must be valid; explicit codec or message registries take precedence over the process default.

Environment and default-folder resolution happen once, on the first global lookup. `LocalFolder::config` reads `HOME`, then `USERPROFILE`; with neither present the optional default folder is skipped. Installing a default must happen before global resolution, and subsequent reads share the same registry.

| Rust | Python | JavaScript |
| --- | --- | --- |
| `FixRegistry::from_env()` | `FixRegistry.from_env()` | `fix.FixRegistry.fromEnv()` |
| `FixRegistry::install_env(...)` | `FixRegistry.install_env(...)` | `fix.FixRegistry.installEnv(...)` |

## Classifying a captured line

`MimeType` owns protocol inference; `FixCodec` owns shallow raw message-code inference, which requires no registry. `FixRegistry::msgdirection` owns the reading of which way a line moved: FIX's own tag 385, its code set the dictionary's and the [rules that name one](#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) carried on the field as `FIX:directions`.

| Recognized payload | Protocol |
| --- | --- |
| Framed numeric pairs | `text/fix` |
| Name keys with `#` markers or `MSGTYPE=` | `text/ullink` |
| Numeric frame mixed with symbolic keys | `text/fixul` |
| XML in the official `XmlData(213)` payload | `text/fixml` |
| Unframed key/value text | `text/key-value` |
| XML or JSON | The corresponding generic MIME type |
| Unrecognized bytes | `application/octet-stream` |

A JSON document on a line - an object, or an array of objects, opening before
any `=` and balanced to its close, prose allowed in front and behind - is
`application/json`, which is what it is, and names no message type: the codec
reads such a row as [one message stating nothing](capture.md#a-json-document-is-one-message-stating-nothing).

=== "Rust"

    ```rust
    use yggdryl::FixCodec;

    assert_eq!(FixCodec::infer_msgtype_bytes(b"8=FIX.4.4|35=AE|"), Some(b"AE".as_slice()));
    assert_eq!(FixCodec::infer_msgtype_text("MSGTYPE=P Report Ack|"), Some("P Report Ack"));
    ```

=== "Python"

    ```python
    from yggdryl.fix import FixCodec

    assert FixCodec.infer_msgtype_bytes(b"8=FIX.4.4|35=AE|") == b"AE"
    assert FixCodec.infer_msgtype_text("MSGTYPE=P Report Ack|") == "P Report Ack"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fix } = require('yggdryl')

    assert.equal(fix.FixCodec.inferMsgtypeBytes(Buffer.from('8=FIX.4.4|35=AE|')).toString(), 'AE')
    assert.equal(fix.FixCodec.inferMsgtypeText('MSGTYPE=P Report Ack|'), 'P Report Ack')
    ```

### A direction is what the rules on tag 385 read in front of the payload

Which way a message moved is FIX's own fact, tag 385 `MsgDirection`, and nothing else in the crate has one. The dictionary types the field as it types every coded field - text reading by a code set whose members are `R = Receive` and `S = Send`, extendable like any set, and held under the name tag 385's `FIX:codeset` states - and `FixRegistry::msgdirection` answers the registry's reading of it, a `fix::MsgDirection`. The rules that reading applies are the dictionary's too: tag 385's field carries them as `FIX:directions`, one canonical document `[{"code":"S","patterns":["..."]},...]` with an entry per code in the order the dictionary lists them, each pattern a `regex::bytes` expression applied to the prefix - the bytes before the payload, exactly the bound `payload_at` answers, so a verb inside a payload is still the payload's word. A code matches where any of its patterns matches; exactly one matching code names the direction; two or more, or none, name nothing. An entry's code is any spelling of a code of the set - its value, its name, an alias - resolved once through `MsgDirection::code` exactly as a pin is.

Where the field carries no property the defaults answer, keyed by the set's `Send` and `Receive` codes so an extended set still reads `sending >>` as its own `Send`:

| code | pattern | reads |
| --- | --- | --- |
| Send | `(?i)(?:^\|[\s\[(<])(?:sending\|sent\|send\|outbound\|outgoing)(?:[\s\])>:,]\|$)` | the spelled verbs, opened by the start, whitespace or `[`, `(`, `<`, closed by the end, whitespace or `]`, `)`, `>`, `:`, `,` |
| Send | `(?i)(?:^\|[\[(])out(?:[\]):]\|$)` | the bare word, only bracketed: `[OUT]`, `(out)`, `OUT:` |
| Send | `(?i)(?:^\|\s)request:` | the half a Jolokia exchange states in its prose: a request went out |
| Receive | `(?i)(?:^\|[\s\[(<])(?:receiving\|received\|receive\|recv\|inbound\|incoming)(?:[\s\])>:,]\|$)` | the spelled verbs |
| Receive | `(?i)(?:^\|[\[(])in(?:[\]):]\|$)` | the bare word, only bracketed: `(in)`, `[IN]` |
| Receive | `(?i)(?:^\|\s)response:` | an answer came back |

`Request:` and `Response:` prose names the half of an exchange a bridge logged - a request went out, an answer came back - and a bare JSON document states nothing of itself: the half is prose in front of the payload, read by the same rules as every other prose, so a document with no prose in front of it has no tag 385 on the line door and takes the pin on the batch door. The direction is the line's and the document is [one message stating nothing](capture.md#a-json-document-is-one-message-stating-nothing), so the half the prose names is the one thing that message states beyond its clock, its version and its row's captures.

`set_directions` on the field's FIX view refuses what a field can answer alone: a code named twice under any folded spelling (`S` beside `Send`), an empty code or one carrying a quote, backslash or control character, an entry with no pattern, an empty pattern, or a pattern `regex::bytes` refuses, leaving the field unchanged. Whether a code is one of the set is no longer its question - the field carries no members, only the [name](#a-field-names-the-code-set-it-reads-by) of the set it reads by, and the dictionary holds them - so membership is checked where the rules are read: `MsgDirection::from_registry` resolves every code against the set in force and drops a rule naming one outside it, warning `expected a code of the set, one of ..., got ...`. A hand-edited dictionary reaches that reading without the setter at all, and every entry the setter would have refused is dropped there too, with a warning that is its refusal word for word, so the table degrades to fewer rules - down to none, where a property the field carries states nothing readable - rather than to a wrong reading, and never falls back to the defaults, which are the absent property's alone; an empty list, or `remove_directions`, takes the property away; `directions()` walks it borrowed and `FixDirection` is the owned rule; through `update` the incoming table [wins whole](#one-merge-with-a-rule-per-key). The [CLI](cli.md#definition-flags) edits it through `--directions '<json>'` on `fields create` and `fields update`, the vocabulary stated once beside it:

```bash
yggdryl fix codesets write msgdirectioncodeset --codes '[{"value":"R","name":"Receive"},{"value":"S","name":"Send"}]'
yggdryl fix fields update MsgDirection utf8 --tag 385 --codes msgdirectioncodeset --directions '[{"code":"S","patterns":["(?i)^TX\\b"]},{"code":"R","patterns":["(?i)^RX\\b"]}]'
```

`--codes` names the set because an update replaces the definition whole and the name is what the definition carries; the members stay where they are, so restating a field never restates its vocabulary. The bindings read and write the rules as a list on `field.fix.directions`. `MsgDirection::directions` answers the rules in force - the field's, each code resolved to the set's value, or the defaults - as data, so what a dictionary reads by is never hidden in Rust. The codec compiles every pattern once into a `regex::bytes::Regex` when it takes its registry (`FixCodec::new`), so a codec is built after the field is edited, and a row applies the compiled patterns to its prefix allocating nothing. The committed dictionary carries no property and reads by the defaults: they have one owner, the crate, and a dictionary that ships a table states its own.

Every door fills tag 385 from that reading where the wire states none - `parse_line`, `parse_text_line`, the single-dialect doors, the batch reader - and the batch reader's precedence is a stated `msgdirection` column, else the reading, else the codec's pin (`try_with_direction`, the `Send` code by default), which is a code of the set. `MsgDirection::code` resolves any spelling of a code - its value, its name, an alias - and a spelling outside the set is refused naming the set.

## Edges

- A scalar without `FIX:tag`, a nested tagged field, or a nullable message root is refused.
- A Serie/LargeSerie group needs a valid `int32` counter and non-null Struct occurrence; the serie's own nullability is independent. A crate Map group instead requires matching reserved `FIX:tag`/`FIX:counter` values that no scalar canonical or alternate tag occupies; its entries Struct and key remain non-null.
- A component or Serie/LargeSerie group carries the tag derived from its name; its category and folded name identify it, and a stated tag outside `[100000, 1100000)` is refused. The crate Map's own reserved tag is not a derived definition tag.
- A derived tag names a definition this crate derived rather than a tag anyone published; a definition keeps the tag it already has through an update, and one arriving on a tag another definition holds derives afresh.
- Missing, cyclic, contradictory, or over-depth references fail at intake with location; the nesting limit is 64.
- Removing a referenced definition fails atomically; delete dependents before their sources.
- A field-reference occurrence may vary name and nullability, but may not introduce independent metadata overrides.
- Identifier declarations resolve only direct scalar members; an ambiguous spelling, nested selection, duplicate target or malformed list is a located atomic refusal, including raw stored metadata at intake.
- A string lookup is a name or a dotted path, colon included; an id is an integer spelled only through `FixKey::Id` in Rust and `field_by_id` / `get_by_id` in the bindings, and a bare integer anywhere else is a tag.
- `FIX:sources` is never an argument: no lookup, definition or message-type accessor takes a source, and the only filter on membership is the one a caller writes over `sources()`. A codec reads under one source ([`FixCodec::with_source`](capture.md#the-crates-own-columns)) for the plugin role it stamps, and resolves nothing through it.
- Generic scalar iteration and size exclude named definitions. Use the explicit category iterators to walk the catalog.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix
    cargo test --features internals -p yggdryl --test fix -- mod_::internal
    cargo test --features internals -p yggdryl --test fix -- mod_::internal::the_fold_table_holds_through_add_field_and_through_merge_with mod_::internal::one_message_code_namespace_folds_a_restated_name_and_keeps_a_second_one mod_::internal::three_spellings_of_one_name_under_one_tag_are_one_identity mod_::internal::name_indexes_fold_ascii_and_membership_never_resolves
    cargo test -p yggdryl --test fix -- registry::lenient
    cargo test -p yggdryl --test fix -- cfb::
    cargo test -p yggdryl --test fix -- retired:: latest:: enrich::
    cargo test -p yggdryl --test fix -- aliases:: alias_rule::
    ```

=== "Python"

    ```bash
    python -m pytest python/tests/test_fix.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    ```

## Performance

The Rust column is one release run of the Criterion target on one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.94.1 release (thin LTO, one codegen unit), 100 samples, Criterion's default warm-up and measurement window. The Python and Node columns are an earlier release run on Windows, AMD Ryzen 5 150 with 12 logical CPUs, Python 3.12.13 and Node 24.18, 2,000 boundary iterations each; the two hosts differ, so a row compares a language against its own boundary and not against the Rust figure.

| Read | Rust estimate | Python | Node |
| --- | ---: | ---: | ---: |
| Scalar tag hit | 5.62 ns | 184 ns | 346,699 ops/s |
| Folded scalar name hit | 152 ns | 522 ns | 242,181 ops/s |
| Named group lookup | 185 ns | 336 ns | 385,758 ops/s |
| Message singleton lookup | 44.7 ns | 296 ns | 359,589 ops/s |
| Message-scoped group lookup | 22.8 ns | Not isolated | Not isolated |

Rust lookup rows borrow the full seed's native definitions; binding rows include their wrapper boundary. Python's category iterators cover the full seed, while Node's category iterator benchmarks use a small catalog with two fields and one definition in each other category, so their first/drain results are not a cross-language comparison.

| Mutation | Rust estimate | Workload |
| --- | ---: | --- |
| Insert into seed | 113 us | New independent scalar field |
| Referenced metadata update in seed | 854 ms | Atomically refresh the full reference graph |
| Per-field metadata merge | 6.89 us | `FixFieldMut::merge_with` |
| Small-registry merged update | 23.3 us | `FixRegistry::update` |
| Catalog merge with a code-set union | 99.8 us | Two scalar fields plus one component, group and message; imported references refresh against the merged fields |

The catalog merge excludes the setup clone from its timer and includes source validation, the code-set fold, reference resolution and final validation. It uses a small catalog, separate from the seed mutation cases; the figure predates the vocabularies moving from the fields into `codesets` and was not rerun for that change.

### Classifying a capture

The shallow raw FIXML message-code scan measured 963 ns in Rust; Python's Ullink message-code inference measured 639 ns and Node's measured 1,230,618 ops/s. These rows use different wire fixtures and describe their own boundary costs. What the text reader's three classification columns cost over a whole capture is measured where the capture is read, in [`fix/pipeline`](arrow.md#performance).

Borrowed Rust lookups, singleton views, compiled group-plan lookups and identifier selection have counting-allocator coverage. `identifiers()` and `identifier_values()` allocate nothing across the pinned corpus sizes; Python and Node allocate their returned lists and wrappers. Identifier boundary benchmarks are present but were not run for this change. Stable hashing allocates one native digester state, and snapshots/projections allocate by contract; the [store measurements](store.md#performance) cover the full graph separately.

Regenerate from the repository root with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- 'fix/(resolve|mutate|store)'
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
