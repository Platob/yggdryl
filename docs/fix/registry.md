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
| Enums | Each scalar field carries its own canonical `fix:codes` metadata, every version's values included: a code an older version declared and the newest dropped is dated `deprecated`, and an older spelling of a surviving code is one of its aliases |
| History | `fix:lineage` dates a field's names and types; the generator writes `deprecated` and `removed` entries, so a field FIX retired is in the dictionary with the version that retired it |
| Replacements | A field FIX retired or whose values it replaced carries `fix:replacements`: how a message's `into_latest` restates it at the newest version; Rust holds no rule table, so a registry edit is a rule edit |
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
| `get_branch_by_digest(i32)` / `branch_by_digest(i32)` | The branch named by a signed branch digest, as `FixBranch::digest_signed` answers it and `branches.json` publishes it |
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

### Every version's values are in the set

The committed dictionary folds the FIX 4.0 to 5.0 SP2 listings into each set. A value an older version declared and the newest dropped is a code of its own, dated `since` the first version listing it and `deprecated` at the version after the last: `ExecType(150)` `1` and `2`, the partial fill and the fill FIX 4.3 folded into `Trade`, are `PartiallyFilled` and `Filled` - the names the crate's `state` column reads - since 4.1 and deprecated 4.4. A legacy name that folds onto a current one takes the suffix `Legacy`, and so does a deprecated name the newest version spells beside the current one that replaced it - `BenchmarkCurveName(221)` `Euribor` is `EuriborLegacy` beside `EURIBOR`, because one spelling cannot reach two codes; an older spelling of a value the newest version keeps becomes one of its aliases, so a name a 4.2 dictionary used still reaches the value. A message's [restatement](message.md#restated-at-the-dictionarys-newest-version) reads these dates: a target holding a code its set no longer declares at the newest version takes the rule's value, one holding a current code stands.

Reading a code's pedigree is Rust only:

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixRegistry, Version};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    // FIX 4.1 declared ExecType 1; 4.3 folded it into Trade and 4.4 stopped listing it.
    let partial = registry.field_by_tag(150)?.as_fix().code("1").expect("a legacy code");
    assert_eq!(partial.name(), "PartiallyFilled");
    assert_eq!(partial.since(), Some("4.1".parse::<Version>()?));
    assert_eq!(partial.deprecated(), Some("4.4".parse::<Version>()?));
    assert!(partial.defined_at("4.2".parse()?));
    assert!(!partial.defined_at("4.4".parse()?));
    // A spelling an older version gave a surviving value is an alias of it.
    assert_eq!(registry.field_by_tag(35)?.as_fix().code_value("ExecutionAcknowledgement"), Some("BN"));
    // A legacy name that folds onto a current one takes the suffix.
    assert_eq!(registry.field_by_tag(327)?.as_fix().code_name("D"), Some("NewsDisseminationLegacy"));
    ```

## Versions are a filter on the read

Rust's `field_at` and `get_field_at` filter scalar lookup through `fix:lineage`; an undated field exists at every version. `FixField::since`, `until`, `defined_at`, `deprecated_at`, `name_at`, and `dtype_at` read the field's own history; `FixRegistry::versions` and `newest` summarize it.

A field FIX retired is in the dictionary: the generator writes every tag some FIX 4.0 to 5.0 SP2 dictionary declares and the newest lacks - `ExecTransType(20)`, `Rule80A(47)`, `ExecBroker(76)`, `ClientID(109)` and thirty-four more - with a lineage entry per version that named it and a final `{"since": v, "removed": true}` at the first version that did not, so `until` answers that version and `get_field_at` stops answering there, while `field_by_tag` answers at every version because a capture holds what was sent. A field the newest version keeps but the specification deprecated - Orchestra's `deprecated` attribute, and the three appendix datings before a removal - carries `{"since": v, "deprecated": true}`, which `deprecated_at` reads and `defined_at` ignores.

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

The committed dictionary's own history, read the same way:

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixPedigree, FixRegistry, Version};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    // Rule80A: declared at 4.0, deprecated at 4.3, gone at 4.4 - and still a field.
    let rule80a = registry.field_by_tag(47)?;
    assert_eq!(rule80a.name(), "rule80a");
    assert_eq!(rule80a.as_fix().since(), Some("4.0".parse::<Version>()?));
    assert!(!rule80a.as_fix().deprecated_at("4.2".parse()?));
    assert!(rule80a.as_fix().deprecated_at("4.3".parse()?));
    assert_eq!(rule80a.as_fix().until(), Some("4.4".parse::<Version>()?));
    assert!(registry.get_field_at("4.2".parse()?, 47).is_some());
    assert!(registry.get_field_at("4.4".parse()?, 47).is_none());
    // LegQty: deprecated at 5.0 SP1 and never removed, so defined at the newest.
    let legqty = registry.field_by_tag(687)?.as_fix();
    assert!(legqty.deprecated_at("5.0.1".parse()?));
    assert_eq!(legqty.until(), None);
    let newest = registry.newest().map(FixPedigree::version);
    assert_eq!(newest, Some("5.0.2".parse::<Version>()?));
    assert!(legqty.defined_at(newest.expect("a dated dictionary")));
    ```

| Lineage entry key | Meaning |
| --- | --- |
| `since` | Required numeric dotted version; canonical entries are oldest first |
| `ep` | Optional extension pack |
| `name`, `type`, `doc` | Name, resolved datatype document, and description from that point |
| `deprecated`, `removed` | The specification deprecated the field from this version, or stopped declaring it; a `removed` entry states no name or type, and `defined_at` is false from it on |

`set_lineage` checks that the newest entry agrees with the field, resolves datatype spellings, derives historical aliases, and collapses unchanged entries. Earlier string entries immediately preceding a temporal type adopt that temporal interpretation; other retypes remain distinct. Duplicate pedigrees and unresolved datatypes fail atomically. An empty lineage removes its derived aliases; an undated field has no version filter, and `newest()` returns a real pedigree rather than a moving label or `Version::MAX`.

### `fix:nulls`, the spellings that mean nothing was sent

`fix:nulls` is comma-separated, matched case-insensitively against trimmed input after the field is resolved. A matched spelling becomes null in the typed row while arrival entries retain the original bytes; `FixCodec::with_null_values` is the capture-wide counterpart applied before field lookup.

## A field carries what replaced it

The specification retires a field or a value and says what stands in for it: `Rule80A(47)` became `OrderCapacity(528)` beside `OrderRestrictions(529)`, the partial-fill values of `ExecType(150)` folded into `Trade`, `ExecBroker(76)` became one `Parties` occurrence with role `1`. Those rules are facts about the field being restated, so they travel on it as `fix:replacements`: one canonical document, read borrowed, that a [message's `into_latest`](message.md#restated-at-the-dictionarys-newest-version) applies. A registry adds or edits a rule by editing metadata; nothing in Rust holds a table of them.

Entries are in **document order**, and the order is semantic: the first entry whose conditions a held value meets answers, so a catch-all entry stating no `when` comes last.

```json
{"replacements":[{"since":"4.3","when":"A","fills":[{"tag":528,"value":"A"}],"doc":"Rule80A A is OrderCapacity A (FIX 4.3 Appendix 6-F)"}]}
```

| Entry key | Required | Value | Meaning |
| --- | --- | --- | --- |
| `since` | yes | dotted version | the version the specification replaced the feature at; `into_latest` applies every entry whatever its date |
| `ep` | no | integer | the extension pack that dated the replacement |
| `msgtypes` | no | wire codes, exact case | the entry applies only when the root's `MsgType(35)` is one of them; absent is every message |
| `in` | no | folded group names | the entry applies only when the source sits inside an occurrence of one of these repeating groups; absent is wherever the field sits |
| `when` | no | wire text | the held value the entry applies to; absent is any stated value |
| `fills` | yes, one or more | fills | the fields that take a value, and what value |
| `doc` | no | text | the specification's wording, where the mapping is not the plain "same value in the replacement field" |

| Fill | Keys | The target takes |
| --- | --- | --- |
| `{"tag":1138}` | `tag` | the source field's own value, re-typed for the target |
| `{"tag":528,"value":"A"}` | `tag`, `value` | that constant, as wire text; a `MultipleCharValue` target takes its tokens space-separated, `"1 3"` |
| `{"tag":628,"from":115}` | `tag`, `from` | the stated value of that tag at the same level |
| `{"tag":541,"join":[200,205]}` | `tag`, `join` | the wire texts of those tags concatenated, two at least, every one stated; an integer part is spelled with two digits, which is how a day completes a month-year |
| `{"group":"parties","members":[...]}` | `group`, `members` | one occurrence of that repeating group at this level, whose members are fills of their own - a member may itself be a group |

Keys are read in the order the tables list them; exactly one of `tag` and `group` is stated, `value`, `from` and `join` travel only with `tag` and at most one of them, `members` only with `group`. The scanner refuses an unknown key, a key out of order, and a missing `since` or `fills` at its byte; the writer refuses an entry with no fill, a group fill with no member, a `join` of fewer than two tags, a negative tag, and any message type, group name, held value or constant that is not a word the reader reads back.

How one entry is applied at one level - the root, or one occurrence of a repeating group:

| Step | Rule |
| --- | --- |
| Source | each child whose field carries `fix:replacements`, in ascending tag order, so `ExecTransType(20)` writes `ExecType(150)` before `ExecType`'s own rule reads it |
| Match | the first entry whose `msgtypes`, `in` and `when` hold; a held value meets `when` when its wire text equals it or one space-separated token does, and a `state` when `when` names the state held |
| Plan | every fill computed: the source re-typed, a constant read as the target's field reads wire text, a `from` or `join` from the stated values at this level; a value the target cannot hold, or a `join` part unstated, ends the entry |
| Check | every target writable: absent, null, already equal to what would be written, or holding a code its set no longer declares at `newest()`; the source field itself is always writable |
| Apply | all-or-nothing: one target that cannot take its value blocks the whole entry, and no later entry fills in for it; a group fill merges into the occurrence whose constant members all equal the fill's constants, else appends one, and sets the counter to the count the group then has |
| Chain | an entry that rewrote the source's own value leaves a new held value, restated in turn, bounded by the rules the field states |

### Configuring a rule

A rule is metadata on the field, so it is configured the way any field fact is: edit the field, `update` the registry, and every reader linked to that registry restates by it. The typed builders - `FixReplacement`, `FixFill`, `FixFillSource` - and the borrowed read, `replacements()`, are Rust only; Python and JavaScript write the canonical text on the `fix:replacements` key. An empty list, or `remove_replacements`, takes the rules away from the field in hand; through `update` the incoming document replaces the stored one whole, because two documents have no order between them, and a stored document the incoming field does not state is kept, as every other protocol key is. So a rule is replaced through `update` by writing the document that should stand, and silenced by a registry built without it - never by omitting the key.

The committed rule reads `Rule80A(47)` `A` as an agency order. A desk that knows its 4.2 counterparty meant a principal one edits the field, and nothing else:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::fix::{FixFill, FixFillSource, FixReplacement};
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixCodec, FixRegistry, Version};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let mut registry = FixRegistry::from_handle(&Folder::new(root)?)?;

    let mut rule80a = registry.field_by_tag(47)?.clone();
    rule80a.as_fix_mut().set_replacements(&[FixReplacement::new("4.3".parse::<Version>()?)
        .with_when("A")
        .with_fills([FixFill::Field { tag: 528, value: FixFillSource::Constant("P".into()) }])
        .with_doc("Rule80A A on this venue was a principal order")])?;
    // The builder writes the one canonical text the reader reads back.
    assert_eq!(
        rule80a.get_metadata("fix:replacements"),
        Some(concat!(
            r#"{"replacements":[{"since":"4.3","when":"A","fills":[{"tag":528,"value":"P"}],"#,
            r#""doc":"Rule80A A on this venue was a principal order"}]}"#,
        ))
    );
    registry.update(rule80a)?;

    // Read back borrowed, in document order.
    let entry = registry.field_by_tag(47)?.as_fix().replacements().next().expect("one rule")?;
    assert_eq!(entry.since(), "4.3".parse::<Version>()?);
    assert_eq!(entry.when(), Some("A"));

    // Every reader linked to the registry restates by the edited rule.
    let reader = FixCodec::new(Arc::new(registry));
    let read = reader.parse_line(b"8=FIX.4.2|35=D|11=A|47=A|10=0|")?.next().expect("one frame")?;
    let latest = read.into_latest()?;
    assert_eq!(latest.by_tag(528)?.as_str(), Some("P"));
    assert_eq!(latest.by_tag(47)?.as_str(), Some("A"), "the source stays as read");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())

    rule80a = registry.field_by_tag(47)
    rule80a.metadata["fix:replacements"] = (
        '{"replacements":[{"since":"4.3","when":"A","fills":[{"tag":528,"value":"P"}]}]}'
    )
    registry.update(rule80a)
    assert '"value":"P"' in registry.field_by_tag(47).metadata["fix:replacements"]

    # Every reader linked to the registry restates by the edited rule.
    reader = FixCodec(registry)
    read = next(reader.parse_line(b"8=FIX.4.2|35=D|11=A|47=A|10=0|"))
    latest = read.into_latest()
    assert latest.by_tag(528).as_py() == "P"
    assert latest.by_tag(47).as_py() == "A", "the source stays as read"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))

    const rule80a = registry.fieldByTag(47)
    rule80a.set(
      'fix:replacements',
      '{"replacements":[{"since":"4.3","when":"A","fills":[{"tag":528,"value":"P"}]}]}',
    )
    registry.update(rule80a)
    assert.match(registry.fieldByTag(47).get('fix:replacements'), /"value":"P"/)

    // Every reader linked to the registry restates by the edited rule.
    const reader = new fix.FixCodec(registry)
    const read = reader.parseLine(Buffer.from('8=FIX.4.2|35=D|11=A|47=A|10=0|')).next().value
    const latest = read.intoLatest()
    assert.equal(latest.byTag(528).toJSON(), 'P')
    assert.equal(latest.byTag(47).toJSON(), 'A', 'the source stays as read')
    ```

### The rules the dictionary carries

The generator writes the replaced and deprecated features of FIX 4.3 through 5.0 SP2 - the specification's appendices "Replaced features" (6-F) and "Deprecated features" (6-E) - onto 37 fields as 100 entries, each validated at generation against the dictionary: the source and every target tag exist, a `when` and a constant are codes of their field's set where it has one, group and member names exist. A rule states only what the appendix states as a value mapping.

| Since | Source | Restated as |
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
| 4.3 | `OnBehalfOfSendingTime(370)` | one `HopGrp` occurrence: `HopSendingTime(629)`, `HopCompID(628)` from `OnBehalfOfCompID(115)` |
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

Deliberately not covered, because the appendix states no value mapping a rule can write: `MDEntryOriginator`, `MDMkt`, `LocationID` and `DeskID` into `PartyRole`; `TargetStrategyParameters` and `ParticipationRate` into `StrategyParametersGrp`; the settlement instruction fields 173 to 187 into `SettlParties`; `SecurityType` `FOR`, which has four candidates; `QuoteType`; `SecondaryTradeReportID` and `SecondaryTradeReportRefID`; `Signature` and the `SecureData` pair; `UnitOfMeasure` `MMbbl`; the `UnderlyingLeg` fields of EP187; `TotalNumPosReports`; `ReceivedDeptID`; `FXBenchmarkRateFix`. `SecurityType` `FUT` and `OPT` and `PutOrCall` into `CFICode` are not rules either, because [enrichment](capture.md#what-a-message-implied-is-filled-in) already derives them. A field the specification removed and replaced with nothing - `SendingDate(51)`, `WaveNo(105)` - is in the dictionary with its `removed` entry and stays in a restated row as read.

## One merge, with a rule per key

Scalar `update` merges the same identifier: incoming scalar metadata wins, aliases and alternate tags combine under native validation, and canonical spelling is retained. Category `update_definition` instead replaces the entire supplied definition while preserving its identity.

| Metadata | Merge rule |
| --- | --- |
| `fix:tag`, `fix:branch` | Must agree |
| `fix:tags` | Combine alternate tags under collision validation |
| `fix:lineage` | Merge by pedigree, incoming entry winning a shared point |
| `fix:codes` | Merge by wire value, incoming code winning a shared value |
| `fix:replacements` | Incoming wins whole: the order of its entries is the rule, and two documents have no order between them |
| Other protocol keys | Incoming wins; preserve keys only the stored field declares |
| Generic description, display, comment, aliases | The generic metadata merge accompanies the protocol merge |

`FixFieldMut::merge_with` owns the protocol half; `FixRegistry::update` also merges generic metadata and updates indexes. A refusal changes nothing, and an update adding nothing leaves the definition unchanged.

## Folding a second source in

Rust and Python expose `merge_with`, `add_fields`, and `add_cfb_file` as atomic native folds. `from_cfb_file` in all three languages returns the imported registry and its declared roots, including canonical scalar metadata, named groups/components/messages, and inline enum codes; the [CLI](cli.md) exposes ingestion and synchronization.

A `vocabulary-tag`'s `alt` names its tag where it names only that tag. A dialect that spells one `alt` over two tags - `TRTN_FX_TradeCapture` declares `HedgeCurrency` for the currency a hedge settles in and again for the one it is quoted in - has given a name to neither, and a tag whose `alt` is another tag's own decimal has done the same to that tag's identity. Both fall back to their own decimal, the name a tag declaring no `alt` already takes, and keep the declared spelling as `display`, so every tag is left named and nothing the file said is lost. Contention is decided by the key a name is indexed under, which folds case and drops `_`, `-` and space, so `Hedge_Currency` contends with `HedgeCurrency`. Two tags sharing a spelling record each other's tag among their alternate tags and so stay reachable as a pair; three record nothing, because an alternate identifier names one field. A `normalization-binding` cannot spell a contended name back onto one of them, and a `map` naming one decodes neither. The spelling survives where the file made it unambiguous: a `tag-constraint` binds one tag, so the message root, the component and the group each carry it, and a reader resolving a key against the message it arrived in - a bridge row's `MSGTYPE`, and the repeating group the key sits in - reaches the tag the file meant.

A CBlock's `normalization-binding` is read for the names it spells its tags with, and for nothing else. A `tag-normalization` whose mapping is one bare `$602` says its `tag-name` is another spelling of tag 602, so that spelling joins the field as an alias while the `vocabulary-tag` keeps the name. A conditional mapping, a `lookup`, and a mapping built from several expressions each name nothing: this layer holds no evaluator. Most of a real binding spells names a tag already answers to - resolution folds ASCII case - so the pass pays where a `vocabulary-tag` declared no `alt` and the tag is otherwise reachable only by its own number. No name is refused: one the vocabulary never declared, one another tag in the same branch already answers to, or one the core could not store drops on its own.

`merge_with` combines another registry and its dialect declarations; `add_fields` folds a scalar field iterable; `add_cfb_file` parses a CBlock, folds its vocabulary, and records the declared numeric FIX version and source branch aliases. These operations report added/merged counts only after the entire staged fold succeeds.

A CBlock is read for what it says. A real one is megabytes over hundreds of thousands of elements, so an element this reader cannot make sense of - a tag spelled in a way the core cannot store, a constraint naming a tag the file's own vocabulary never declared, a mapping to a type nothing listed, a `fix-version` the version grammar cannot read - is dropped and the rest of the file is still a dictionary. Each drop is a `log` record at warn level carrying the located sentence a refusal would have: the byte, what was expected, what arrived, and the element the file spells it in. Only a document that is not well-formed XML, or that stops with an element open, is refused across each binding as a native located error, because neither leaves anything to keep.

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
    cargo test -p yggdryl --lib -- fix::tests::a_replacement_document_round_trips_canonically_and_in_order fix::tests::the_replacement_writer_refuses_what_the_document_cannot_state fix::tests::a_merge_lets_the_incoming_replacements_win_whole fix::tests::every_committed_replacement_is_the_document_the_rust_writer_renders
    cargo test -p yggdryl --test fix latest::the_dictionary_carries_the_rules_the_engine_reads
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
| Catalog merge with inline-code union | 99.8 us | Two scalar fields plus one component, group and message; imported references refresh against the merged fields |

The catalog merge excludes the setup clone from its timer and includes source validation, code union, reference resolution and final validation. It uses a small catalog, separate from the seed mutation cases.

### Classifying a capture

The shallow raw FIXML message-code scan measured 963 ns in Rust; Python's Ullink message-code inference measured 639 ns and Node's measured 1,230,618 ops/s. These rows use different wire fixtures and describe their own boundary costs. What the text reader's three classification columns cost over a whole capture is measured where the capture is read, in [`fix/pipeline`](arrow.md#performance).

Borrowed Rust lookups, singleton views, and compiled group-plan lookups have counting-allocator coverage. Stable hashing allocates one native digester state, and snapshots/projections allocate by contract; the [store measurements](store.md#performance) cover the full graph separately.

Regenerate from the repository root with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- 'fix/(resolve|mutate|store)'
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
