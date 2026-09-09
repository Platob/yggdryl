# Registry

`FixRegistry` owns tagged scalar fields and named message, component, and group definitions, with atomic mutations and indexed borrowed reads.

## Contract

| Category | Native definition | Identity |
| --- | --- | --- |
| `fields` | Tagged scalar `Field`; group counters are `int32` | Canonical tag and branch, plus folded name |
| `messages` | Non-null Struct `Field` owned by an immutable `MsgType` singleton | Name and branch; `fix:msgtype` carries the complete wire code |
| `components` | Named Struct `Field` | Name and branch |
| `groups` | Named List or LargeList of a non-null Struct occurrence | Name and branch; `fix:counter` identifies a separate scalar field |

| Aspect | Rule |
| --- | --- |
| Enums | Each scalar field carries its own canonical `fix:codes` metadata |
| Definition tags | The specification names components, groups and messages rather than tagging them, so each carries a `fix:tag` derived from its name into `[100000, 1100000)`, clear of every published tag; a reference occurrence never restates it |
| References | `fix:field`, `fix:component`, and `fix:group` resolve once at catalog intake; live definitions hold resolved native fields |
| Planning | Message identity, contextual counter lookup, and group layouts are compiled before parsing rows |
| Mutation | A refusal leaves every category and index unchanged; metadata edits refresh referenced occurrences atomically |
| Identity spelling | A case-only replacement preserves the stored canonical name; an identity or referenced datatype change is refused |
| Iteration | Scalar fields iterate tag-major; named categories and message singletons have deterministic native order |
| Ownership | Rust borrows definitions. Python and Node views retain the native registry; mutation refuses while a codec, message, singleton, or active iterator shares it |
| Snapshot | `into_json` / `from_json` preserve all four categories and branch declarations; stable hashes include that complete state |
| Crate fields | `new()` holds this crate's [twenty fields](capture.md#the-crates-own-columns), standard tags from 65000, before anything is inserted, so every registry - loaded, built or left empty - resolves `timestamp` and `sendersessionid`; a [store](store.md) never writes them and reads past a stored copy |

## Use

`NoPartyIDs(453)` stores a count, while `Parties` stores the occurrences and `Party` describes one occurrence.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCategory, FixId, FixRegistry};

    let mut counter = DataType::Int32.nullable_field("NoPartyIDs");
    counter.as_fix_mut().set_tag(453)?;
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut registry = FixRegistry::from_fields([counter, party_id])?;

    let mut member = registry.field(448)?.clone();
    member.as_fix_mut().set_field_ref("PartyID")?;
    let party = DataType::from_fields([member])?.required_field("Party");
    registry.create_definition(FixCategory::Components, party.clone())?;
    let mut parties = DataType::list(party).nullable_field("Parties");
    parties.as_fix_mut().set_counter(453)?;
    parties.as_fix_mut().set_component("Party")?;
    registry.create_definition(FixCategory::Groups, parties)?;

    let mut group = registry.definition(FixCategory::Groups, "Parties", None)?.clone();
    group.as_fix_mut().set_group("Parties")?;
    let mut count = registry.field(453)?.clone();
    count.as_fix_mut().set_field_ref("NoPartyIDs")?;
    let mut order = DataType::from_fields([count, group])?.required_field("Order");
    order.as_fix_mut().set_msgtype("D")?;
    registry.create_definition(FixCategory::Messages, order)?;

    assert_eq!(registry.field(453)?.dtype(), &DataType::Int32);
    assert_eq!(registry.field_by_path("Order.Parties.PartyID", None)?.as_fix().tag()?, Some(448));
    let message = registry.msgtype("D", None)?;
    assert_eq!(message.name(), "Order");
    assert_eq!(message.get_group_by_counter(FixId::from_str("453:")?).unwrap().name(), "Parties");
    assert_eq!(registry.definitions(FixCategory::Groups).count(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types
    from yggdryl.fix import FixRegistry

    counter = Field("NoPartyIDs", "int32")
    counter.fix.tag = 453
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    registry = FixRegistry.from_fields([counter, party_id])

    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    party = Field("Party", DataType.from_fields([member]), nullable=False)
    registry.create_definition("components", party)
    parties = types.list("Parties", party)
    parties.fix.counter = 453
    parties.fix.component = "Party"
    registry.create_definition("groups", parties)

    group = registry.definition("groups", "Parties")
    group.fix.group = "Parties"
    count = registry.field(453)
    count.fix.field_ref = "NoPartyIDs"
    order = Field("Order", DataType.from_fields([count, group]), nullable=False)
    order.fix.msgtype = "D"
    registry.create_definition("messages", order)

    assert registry.field(453).dtype == DataType("int32")
    assert registry.field_by_path("Order.Parties.PartyID").fix.tag == 448
    message = registry.msgtype("D")
    assert message.name == "Order"
    assert message.get_group_by_counter("453:").name == "Parties"
    assert [field.name for field in registry.definitions("groups")] == ["Parties"]
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

    const member = registry.field(448)
    member.fix.fieldRef = 'PartyID'
    const party = fields.struct('Party', [member], { nullable: false })
    registry.createDefinition('components', party)
    const parties = fields.list('Parties', party)
    parties.fix.counter = 453
    parties.fix.component = 'Party'
    registry.createDefinition('groups', parties)

    const group = registry.definition('groups', 'Parties')
    group.fix.group = 'Parties'
    const count = registry.field(453)
    count.fix.fieldRef = 'NoPartyIDs'
    const order = fields.struct('Order', [count, group], { nullable: false })
    order.fix.msgtype = 'D'
    registry.createDefinition('messages', order)

    assert.equal(registry.field(453).dtype.toString(), 'int32')
    assert.equal(registry.fieldByPath('Order.Parties.PartyID').fix.tag, 448)
    const message = registry.msgtype('D')
    assert.equal(message.name, 'Order')
    assert.equal(message.getGroupByCounter('453:').name, 'Parties')
    assert.deepEqual([...registry.definitions('groups')].map(field => field.name), ['Parties'])
    ```

### Group names

The standard calls the repeating block `Parties` and its counter `NoPartyIDs`; Orchestra separately identifies a group's counter and members. See the [FIX Parties description](https://www.fixtrading.org/online-specification/introduction/) and the [pinned Orchestra repository](https://github.com/FIXTradingCommunity/orchestrations/blob/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml).

The generator preserves official group names, including `Grp` suffixes. It derives an occurrence name deterministically: `Parties` becomes `Party`, and `NestedParties2` becomes `NestedParty2`; these singular occurrence names are local naming choices. A collision with an existing field produces an explicit suffix, such as `RateSourceGrp`, `LegRateSourceGrp`, or an occurrence's `Component` suffix. Source display names remain metadata.

## Tiers

Scalar lookups try canonical identifiers, alternate identifiers, folded names, and aliases; a tag query never searches names. An explicit branch pins the lookup, while omission uses the standard branch and then named branches in deterministic order.

| Lookup | Meaning |
| --- | --- |
| `field(55)` | Tagged scalar field |
| `field_by_id(FixId)` | Exact canonical or alternate identifier |
| `field_by_name(name, branch)` | Scalar name or alias |
| `field_by_path(path, branch)` | Scalar first, then a named message/component/group head and nested members |
| `definition(category, name, branch)` | One explicit category |
| `group_by_counter(FixId)` | Globally unique group for that counter |
| `MsgType::get_group_by_counter(FixId)` | Unique group within that message's structure |

The `get_` forms return absence; failing twins return a typed, located error. A path through a group omits the occurrence type: `Parties.PartyID`; a message value adds an occurrence index, such as `Parties.0.PartyID`. A counter shared by multiple contexts is ambiguous globally, so parsing uses the selected message's compiled group index.

Within a scalar lookup kind, omission of a branch tries standard canonical keys, named-branch canonical keys, standard alternates, then named-branch alternates. Names and aliases use separate indexes; a stored name is rechecked after hashing, so a digest collision never selects an unrelated field.

### Branch declarations

`branch_named` tries canonical branch identity before aliases; an alias does not change the canonical branch stored on fields. Rust and Python expose complete branch values, while Node accepts branch text and exposes reverse digest lookup plus complete snapshot preservation.

| Rust accessor | Answer |
| --- | --- |
| `branch_of(FixId)` | Borrowed declaration for an identifier's branch |
| `branch_named(name)` | Canonical name first, then an alias |
| `branches()` | Lazy branch declarations |
| `get_branch_by_digest(i32)` / `branch_by_digest(i32)` | The branch named by an arrival's signed digest |
| `set_branch(FixBranch)` | Install or replace a declaration atomically |

## Accessors

| Rust | Python | JavaScript |
| --- | --- | --- |
| `definitions(category)` | `definitions(category)` | `definitions(category)` |
| `definition(category, name, branch)` | `definition(category, name, branch=None)` | `definition(category, name, branch?)` |
| `msgtype(spelling, branch)` | `msgtype(spelling, branch=None)` | `msgtype(spelling, branch?)` |
| `msgtypes()` | `msgtypes()` | `msgtypes()` |
| `iter()` | `iter(registry)` | `registry[Symbol.iterator]()` |
| `len()` | `len(registry)` | `registry.size` |

The size and ordinary iteration count scalar fields only. Named iterators hold a native position, and singleton iterators preserve exact identity even when one message's name equals another's wire code. Python iterators retain their registry until released; Node category iterators release it on exhaustion or `return()`.

## Insert, update and remove

| Operation | Contract |
| --- | --- |
| `create_definition` | Refuses an existing canonical name or field identifier |
| `insert_definition` | Inserts or replaces one complete definition; returns the replaced field |
| `update_definition` | Replaces an existing definition in full; absence is an error and omitted metadata is removed |
| `remove_definition` | Removes one definition; refuses live references; absence returns no field |
| Scalar `insert` | Inserts or replaces a tagged scalar field |
| Scalar `update` | Merges metadata for the existing identity using the native per-key rules |
| Scalar `remove` | Returns no field when absent or still referenced |

These mutations preserve stored canonical spelling for case-only input changes. Referenced metadata edits cascade through components, groups, and messages; datatype changes and occurrence-local metadata overrides are refused atomically. A named definition stating no tag takes the one derived from its name - XXH32 of the name into `[100000, 1100000)`, stepping past a slot already taken - so a document that states a tag keeps it, and an update keeps the tag the stored definition already has.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCategory, FixRegistry};

    let mut registry = FixRegistry::new();
    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    registry.create_definition(FixCategory::Fields, symbol.clone())?;
    assert!(registry.create_definition(FixCategory::Fields, symbol.clone()).is_err());
    symbol.set_name("SYMBOL");
    symbol.as_fix_mut().set_description("Instrument symbol")?;
    registry.update_definition(FixCategory::Fields, symbol)?;
    assert_eq!(registry.field(55)?.name(), "Symbol");
    let snapshot = registry.into_json()?;
    assert_eq!(FixRegistry::from_json(&snapshot)?, registry);
    assert!(registry.remove_definition(FixCategory::Fields, "Symbol", None)?.is_some());
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
    registry.create_definition("fields", symbol)
    with pytest.raises(ValueError):
        registry.create_definition("fields", symbol)
    symbol.set_name("SYMBOL")
    symbol.fix.description = "Instrument symbol"
    registry.update_definition("fields", symbol)
    assert registry.field(55).name == "Symbol"
    assert FixRegistry.from_json(registry.into_json()) == registry
    assert pickle.loads(pickle.dumps(registry)) == registry
    assert copy.copy(registry).stable_hash() == registry.stable_hash()
    with pytest.raises(TypeError):
        hash(registry)
    assert registry.remove_definition("fields", "Symbol") is not None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    const symbol = Field.from('Symbol: utf8')
    symbol.fix.tag = 55
    registry.createDefinition('fields', symbol)
    assert.throws(() => registry.createDefinition('fields', symbol))
    symbol.setName('SYMBOL')
    symbol.fix.description = 'Instrument symbol'
    registry.updateDefinition('fields', symbol)
    assert.equal(registry.field(55).name, 'Symbol')
    assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
    assert.equal(registry.clone().stableHash(), registry.stableHash())
    assert.ok(registry.removeDefinition('fields', 'Symbol'))
    ```

Python registries are mutable and unhashable; `stable_hash()` explicitly computes the native content hash. Python `copy.copy` and Node `clone()` create independently mutable registries, including every category and branch declaration.

## A field carries its code set

A scalar's enum vocabulary remains inline in `fix:codes`, with required `value` and `name`, plus optional aliases, documentation, and pedigree. Typed borrowed code lookups and `FixCode` construction are Rust-only; both bindings preserve the same metadata through native validation and snapshots.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCode, FixRegistry};

    let mut side = DataType::Utf8.nullable_field("Side");
    side.as_fix_mut().set_tag(54)?;
    side.as_fix_mut().set_codes(&[
        FixCode::new("Buy", "1"), FixCode::new("Sell", "2"),
    ])?;
    let registry = FixRegistry::from_fields([side])?;
    assert_eq!(registry.field(54)?.as_fix().code_value("Buy"), Some("1"));
    assert_eq!(registry.field(54)?.as_fix().code_name("2"), Some("Sell"));
    ```

=== "Python"

    ```python
    from yggdryl import Field
    from yggdryl.fix import FixRegistry

    side = Field("Side", "utf8")
    side.fix.tag = 54
    side.metadata["fix:codes"] = '{"codes":[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]}'
    registry = FixRegistry.from_fields([side])
    assert '"name":"Buy"' in registry.field(54).metadata["fix:codes"]
    assert FixRegistry.from_json(registry.into_json()) == registry
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const side = Field.from('Side: utf8')
    side.fix.tag = 54
    side.set('fix:codes', '{"codes":[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]}')
    const registry = fix.FixRegistry.fromFields([side])
    assert.match(registry.field(54).get('fix:codes'), /"name":"Buy"/)
    assert.ok(fix.FixRegistry.fromJson(registry.intoJson()).equals(registry))
    ```

`code_value` first accepts an exact wire value, then a folded symbolic name or alias, then an abbreviation from the leading description phrase. Ambiguous spellings answer no value; malformed documents report a located error through `codes()` and answer no value through optional lookup methods. `set_codes` validates and writes canonical JSON; an empty set removes the metadata.

| Inline code key | Meaning |
| --- | --- |
| `value`, `name` | Required wire value and symbolic name; `value` leads the canonical record |
| `aliases`, `doc` | Additional spellings and documentation |
| `since`, `ep`, `deprecated` | Numeric dotted version and extension-pack pedigree |
| `sort`, `group` | Source presentation rank and grouping |

`code`, `code_by_name`, `code_name`, and `code_value` borrow the selected code's data; their `_at` variants filter by version. An unknown spelling returns no match so the codec can retain the wire text. Duplicate names, empty names/values, and malformed documents are refused by the writer. Description abbreviations ignore numeric tag cross-references and later parenthesizations; two distinct wire values sharing one folded spelling remain ambiguous.

## Versions are a filter on the read

Rust's `field_at` and `get_field_at` filter scalar lookup through `fix:lineage`; an undated field exists at every version. `FixField::since`, `until`, `defined_at`, `name_at`, and `dtype_at` read the field's own history; `FixRegistry::versions` and `newest` summarize it.

A `Version` has numeric major, minor, and patch parts, such as `5.0.2`; an optional extension-pack number belongs to `FixPedigree`. `fix:nulls` holds the field's explicit wire spellings for absence. These metadata documents remain on the field and round-trip through both bindings.

Typed lineage construction and filtered registry reads are Rust only:

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixLineageEntry, FixPedigree, FixRegistry, Version};

    let mut quantity = DataType::Float64.nullable_field("LastQty");
    quantity.as_fix_mut().set_tag(32)?;
    quantity.as_fix_mut().set_lineage(&[
        FixLineageEntry::new(FixPedigree::new("2.7".parse()?, None))
            .with_name("LastShares").with_dtype("int"),
        FixLineageEntry::new(FixPedigree::new("4.3".parse()?, None))
            .with_name("LastQty").with_dtype("Qty"),
    ])?;
    assert_eq!(quantity.as_fix().name_at("4.2".parse()?), Some("LastShares"));
    assert_eq!(quantity.as_fix().name_at("5.0.2".parse()?), Some("LastQty"));
    assert_eq!(quantity.as_fix().since(), Some("2.7".parse::<Version>()?));
    let registry = FixRegistry::from_fields([quantity])?;
    assert_eq!(registry.field("LastShares")?.name(), "LastQty");
    assert!(registry.get_field_at("2.6".parse()?, 32).is_none());
    ```

| Lineage entry key | Meaning |
| --- | --- |
| `since` | Required numeric dotted version; canonical entries are oldest first |
| `ep` | Optional extension pack |
| `name`, `type`, `doc` | Name, resolved datatype document, and description from that point |
| `deprecated`, `removed` | Deprecation or end of the field's lifetime |

`set_lineage` checks that the newest entry agrees with the field, resolves datatype spellings, derives historical aliases, and collapses unchanged entries. Earlier string entries immediately preceding a temporal type adopt that temporal interpretation; other retypes remain distinct. Duplicate pedigrees and unresolved datatypes fail atomically. An empty lineage removes its derived aliases; an undated field has no version filter, and `newest()` returns a real pedigree rather than a moving label or `Version::MAX`.

### `fix:nulls`, the spellings that mean nothing was sent

`fix:nulls` is comma-separated, matched case-insensitively against trimmed input after the field is resolved. A matched spelling becomes null in the typed row while arrival entries retain the original bytes; `FixCodec::with_null_values` is the capture-wide counterpart applied before field lookup.

## One merge, with a rule per key

Scalar `update` merges the same identifier: incoming scalar metadata wins, aliases and alternate tags combine under native validation, and canonical spelling is retained. Category `update_definition` instead replaces the entire supplied definition while preserving its identity.

| Metadata | Merge rule |
| --- | --- |
| `fix:tag`, `fix:branch` | Must agree |
| `fix:tags` | Combine alternate tags under collision validation |
| `fix:lineage` | Merge by pedigree, incoming entry winning a shared point |
| `fix:codes` | Merge by wire value, incoming code winning a shared value |
| Other protocol keys | Incoming wins; preserve keys only the stored field declares |
| Generic description, display, comment, aliases | The generic metadata merge accompanies the protocol merge |

`FixFieldMut::merge_with` owns the protocol half; `FixRegistry::update` also merges generic metadata and updates indexes. A refusal changes nothing, and an update adding nothing leaves the definition unchanged.

## Folding a second source in

Rust and Python expose `merge_with`, `add_fields`, and `add_cfb_file` as atomic native folds. `from_cfb_file` in all three languages returns the imported registry and its declared roots, including canonical scalar metadata, named groups/components/messages, and inline enum codes; the [CLI](cli.md) exposes ingestion and synchronization.

A `vocabulary-tag`'s `alt` names its tag where it names only that tag. A dialect that spells one `alt` over two tags - `TRTN_FX_TradeCapture` declares `HedgeCurrency` for the currency a hedge settles in and again for the one it is quoted in - has given a name to neither, and a tag whose `alt` is another tag's own decimal has done the same to that tag's identity. Both fall back to their own decimal, the name a tag declaring no `alt` already takes, and keep the declared spelling as `display`, so every tag is left named and nothing the file said is lost. Contention is decided by the key a name is indexed under, which folds case and drops `_`, `-` and space, so `Hedge_Currency` contends with `HedgeCurrency`. Two tags sharing a spelling record each other's tag among their alternate tags and so stay reachable as a pair; three record nothing, because an alternate identifier names one field. A `normalization-binding` cannot spell a contended name back onto one of them, and a `map` naming one decodes neither. The spelling survives where the file made it unambiguous: a `tag-constraint` binds one tag, so the message root, the component and the group each carry it, and a reader resolving a key against the message it arrived in - a bridge row's `MSGTYPE`, and the repeating group the key sits in - reaches the tag the file meant.

A CBlock's `normalization-binding` is read for the names it spells its tags with, and for nothing else. A `tag-normalization` whose mapping is one bare `$602` says its `tag-name` is another spelling of tag 602, so that spelling joins the field as an alias while the `vocabulary-tag` keeps the name. A conditional mapping, a `lookup`, and a mapping built from several expressions each name nothing: this layer holds no evaluator. Most of a real binding spells names a tag already answers to - resolution folds ASCII case - so the pass pays where a `vocabulary-tag` declared no `alt` and the tag is otherwise reachable only by its own number. No name is refused: one the vocabulary never declared, one another tag in the same branch already answers to, or one the core could not store drops on its own.

`merge_with` combines another registry and its dialect declarations; `add_fields` folds a scalar field iterable; `add_cfb_file` parses a CBlock, folds its vocabulary, and records the declared numeric FIX version and source branch aliases. These operations report added/merged counts only after the entire staged fold succeeds. CBlock byte, element, and content failures cross each binding as native located errors.

## Registering a message type

`MsgType` has no public constructor and is not a generic datatype or scalar. It is one immutable registry-owned message Struct; lookup accepts an exact case-sensitive wire code or a folded name/alias, and ambiguous wire codes answer absence.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry};

    let mut field = DataType::Utf8.nullable_field("MsgType");
    field.as_fix_mut().set_tag(35)?;
    let mut registry = FixRegistry::from_fields([field])?;
    let message = registry.register_msgtype("P Report Ack", Some("AllocationReportAck"), None)?;
    assert_eq!(message.as_str(), "P Report Ack");
    assert_eq!(message.as_field().as_fix().msgtype(), Some("P Report Ack"));
    assert_eq!(registry.msgtype("allocationreportack", None)?.as_str(), "P Report Ack");
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

Registration updates tag 35's inline vocabulary and creates an empty message Struct if no message owns that code. Codes may contain spaces and have no artificial width limit; empty text and control characters are refused. Python's `message.field` is read-only, while Node `asField()` returns an independent mutable projection that cannot alter the singleton.

## One default registry per process

The first call resolves one shared default: an explicitly installed registry, then `YGGDRYL_FIX_REGISTRY`, then `Folder::config()/fix`, then `FixRegistry::new()`: the crate's own fields and nothing else. A configured environment location must be valid; explicit codec or message registries take precedence over the process default.

Environment and default-folder resolution happen once, on the first global lookup. `Folder::config` reads `HOME`, then `USERPROFILE`; with neither present the optional default folder is skipped. Installing a default must happen before global resolution, and subsequent reads share the same registry.

| Rust | Python | JavaScript |
| --- | --- | --- |
| `FixRegistry::global()` | `global_registry()` | `fix.globalRegistry()` |
| `FixRegistry::install_global(...)` | `install_global_registry(...)` | `fix.installGlobalRegistry(...)` |

## Classifying a captured line

`MimeType` owns protocol inference; `FixCodec` owns shallow raw message-code inference, which requires no registry. `MsgDirection` owns direction inference and prefix splitting.

| Recognized payload | Protocol |
| --- | --- |
| Framed numeric pairs | `text/fix` |
| Name keys with `#` markers or `MSGTYPE=` | `text/ullink` |
| Numeric frame mixed with symbolic keys | `text/fixul` |
| XML in the official `XmlData(213)` payload | `text/fixml` |
| A JSON document identifying an ULBridge MBean | `text/ulconfig` |
| Unframed key/value text | `text/key-value` |
| Other XML or JSON | The corresponding generic MIME type |
| Unrecognized bytes | `application/octet-stream` |

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

### A bridge configuration is a document that names itself

An ObjectName's `type=` property supplies its raw configuration type; otherwise the request operation supplies it. Bulk and wildcard documents expand lazily through `UlPlugins` and `FixMessages`; each selected configuration becomes one flat typed message, as described in [Capture](capture.md).

### A direction is the verb in front of the payload

`send`, `sending`, `sent`, and outbound markers identify `SENT`; receive and inbound markers identify `RECV`. A configuration response echoes `request` and is received; a request without an echo is sent.

## Edges

- A scalar without `fix:tag`, a nested tagged field, or a nullable message root is refused.
- A group needs a valid `int32` counter and non-null Struct occurrence; the list's own nullability is independent.
- A named definition carries the tag derived from its name; its category, name and branch identify it, and a stated tag outside `[100000, 1100000)` is refused.
- A derived tag is admissible on any branch: it names a definition this crate derived rather than a tag anyone published, and the branch digest keeps two derivations of one name apart.
- Missing, cyclic, contradictory, or over-depth references fail at intake with location; the nesting limit is 64.
- Removing a referenced definition fails atomically; delete dependents before their sources.
- A field-reference occurrence may vary name and nullability, but may not introduce independent metadata overrides.
- A colon-bearing scalar string lookup is a name; parse a `FixId` explicitly or use the binding's identifier method.
- Generic scalar iteration and size exclude named definitions. Use the explicit category iterators to walk the catalog.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix
    cargo test -p yggdryl --lib fix::tests::
    ```

=== "Python"

    ```bash
    python -m pytest python/tests/fix
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/*.test.js
    ```

## Performance

Measured in release mode on Windows, AMD Ryzen 5 150 with 12 logical CPUs, Rust 1.96, Python 3.12.13, and Node 24.18. Python and Node ran 2,000 boundary iterations; Rust Criterion used 10 samples, 100 ms warm-up, and a 200 ms target measurement window.

| Read | Rust estimate | Python | Node |
| --- | ---: | ---: | ---: |
| Scalar tag hit | 9.16 ns | 184 ns | 346,699 ops/s |
| Folded scalar name hit | 346 ns | 522 ns | 242,181 ops/s |
| Named group lookup | 138 ns | 336 ns | 385,758 ops/s |
| Message singleton lookup | 61.1 ns | 296 ns | 359,589 ops/s |
| Message-scoped group lookup | 35.9 ns | Not isolated | Not isolated |

Rust lookup rows borrow the full seed's native definitions; binding rows include their wrapper boundary. Python's category iterators cover the full seed, while Node's category iterator benchmarks use a small catalog with two fields and one definition in each other category, so their first/drain results are not a cross-language comparison.

| Mutation | Rust estimate | Workload |
| --- | ---: | --- |
| Insert into seed | 217 us | New independent scalar field |
| Referenced metadata update in seed | 1.13 s | Atomically refresh the full reference graph |
| Per-field metadata merge | 11.3 us | `FixFieldMut::merge_with` |
| Small-registry merged update | 21.7 us | `FixRegistry::update` |
| Catalog merge with inline-code union | 112.89 us | Two scalar fields plus one component, group and message; imported references refresh against the merged fields |

The catalog merge excludes the setup clone from its timer and includes source validation, code union, reference resolution and final validation. It uses a small catalog, separate from the seed mutation cases.

### Classifying a capture

The shallow raw FIXML message-code scan measured 963 ns in Rust; Python's Ullink message-code inference measured 639 ns and Node's measured 1,230,618 ops/s. These rows use different wire fixtures and describe their own boundary costs.

`fix/classify`, over a `.log` handle read as records - 4,000 lines cycling the five shapes, of which the bridge configuration documents are most of the bytes. Release build, one Linux x86_64 container; the baseline is the same read with the three classification columns off, which is the only honest comparison because it is the same work minus the readings.

| case | median | per row | against the plain read |
| --- | --- | --- | --- |
| `read_arrow_reader`, no classification | 4.89 ms | 1.22 us | - |
| the same with `mimetype`, `msgtype` and `direction` | 15.2 ms | 3.8 us | 3.1x |

The three readings on their own, one line each:

| shape | bytes | `mimetype` | `msgtype` | `direction` |
| --- | --- | --- | --- | --- |
| framed FIX with prose either side | 85 | 609 ns | 575 ns | 367 ns |
| a bare tag stream | 64 | 574 ns | 566 ns | 235 ns |
| a bridge row keyed by name | 78 | 432 ns | 421 ns | 205 ns |
| a sentence nothing matches | 52 | 102 ns | 76.8 ns | 1.15 us |
| a bridge configuration document | 840 | 1.38 us | 1.37 us | 1.11 us |

The scan is linear in the line, so a document is a long line rather than a different kind of work. The one asymmetry is the sentence: with no frame to bound the prose, a direction is read against the whole of it - which is exactly what a document does *not* pay, because its bound is where the object opens.

Classification is opt-in per column for that reason. A capture that only needs rows pays the 1.22 us; one that needs to know what each line is pays the reading over the bytes it has.

Borrowed Rust lookups, singleton views, and compiled group-plan lookups have counting-allocator coverage. Stable hashing allocates one native digester state, and snapshots/projections allocate by contract; the [store measurements](store.md#performance) cover the full graph separately.

Regenerate from the repository root with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
cargo bench --locked -p yggdryl --bench fix -- fix/mutate/merge_catalog_inline_codes --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
