# Store

A FIX catalog persists through one [`IOBase`](../holder/index.md) folder as three category directories and the `codesets/` folder beside them; a message is a component carrying `FIX:msgtype` and lives among the components. What each dialect contributed travels inside the document of the field it contributed to; nothing in the layout is keyed by a dialect.

## Contract

| Aspect | Rule |
| --- | --- |
| Owner | `FixRegistry::from_handle` and `commit`, with `write_into` the same work for a caller that reads no report; bindings redirect to the native loader/writer |
| Fields | `fields/<tag / 100>.json`, the shard written as nine digits with leading zeros - tag 55 in `fields/000000000.json`, tag 5001 in `fields/000000050.json` - so the shards list in tag order wherever they are listed; each document is an array of tagged scalar fields, tag-major, the holder of a shared tag first |
| Named definitions | `components/<name>.json`, `groups/<name>.json`; a message is a component carrying `FIX:msgtype` and is written beside the others; one native `Field` per document, with a derived tag for a component or Serie/LargeSerie group and an own reserved tag for a Map group |
| Code sets | `codesets/<name>.json`, one document per named [code set](registry.md#a-field-names-the-code-set-it-reads-by), stating the name it is filed under and its members in the set's own order; a scalar's `FIX:codeset` holds that name. Read first, because a field naming a set the dictionary does not hold is refused. Not a `FixCategory`: a set has no tag, no datatype and no reference, so nothing in it resolves against a field |
| Documents | The four `FIX:` properties that hold a canonical document - the entry documents `FIX:replacements` and `FIX:directions`, and the lists `FIX:names` and `FIX:tags` - are written as the JSON arrays they are rather than as one escaped line, so an indented document renders as a document and a person can edit one; a code set's `codes` array is written the same way, which is what makes the tree readable. Reading restates each as the compact canonical text a field's metadata holds, with each entry's keys put back into the order the grammar declares and each list held to its element grammar - a word, a positive tag - so a file may spell them in any order and the field still holds one text. One shape: a file spelling one of these keys as text is refused by name, and so is a field holding text no reader can parse. `yggdryl::into_fix_document`/`from_fix_document` are that pair on one field, which is what `ygg fix read --json` prints and `ygg fix ... --input` takes; `FIX:codeset` is not among them, because a name is one word |
| Snapshots | `FixRegistry::into_json`/`from_json` render and read the whole catalog in that same shape, the `codesets` array leading the three category arrays; `add_json_file(handle)` folds one such file into a dictionary the way `merge_with` folds any, and takes no dialect - a snapshot already carries the `FIX:branches` its writer meant |
| References | Ordinary compact child fields use `Null` as the unresolved datatype; a Map's referenced entries keep the Struct its datatype requires. Both retain reference metadata and resolve to canonical native fields at intake |
| Identifiers | `FIX:identifiers` stays on its component; canonical member names and order resolve through the same owner after references load |
| Membership | `FIX:branches` metadata inside each field and named definition document: the sorted, lowercase, comma-separated names of the dictionaries that contributed it; that document is the only place a dictionary is recorded |
| Identity | Derived on every read from `FIX:tag` and the field's name; no document holds an id |
| Builtins | The crate listing has 31 definitions: 29 scalar fields and the `identifiers(65020)` and `metadata(65049)` Map groups. Every registry constructs them, and a write states them too - `fields/000000650.json`, `groups/identifiers.json`, `groups/metadata.json` - so a store is the whole row rather than the half it declared itself; a stored document never overrides them, because a reader takes the constructed definition over the one it finds |
| The fixed row | `commit` also writes `components/fixmsg.json`: the [row every message answers as](capture.md#the-columns-are-the-folded-names) under the name and tag of `FIXMSG_TAG_NAME` (65050), each column a `FIX:field` or `FIX:group` reference carrying its own `FIX:tag`. It is the crate's rather than the store's, so a read passes it over as it passes the crate's own fields; it is there for a consumer that reads the row's shape without running this crate |
| Standard clocks | `SendingTime(52)` and `TransactTime(60)` are ordinary fields: a stored document defining either is loaded first and keeps its metadata, and only a clock the store does not define is seeded afterwards; a registry writes them like any other field in `fields/000000000.json` |
| Validation | Category shape, shard arithmetic, tag and name identity, references, identifiers, code set names, cycles, and depth are checked before exposing the registry; a set's stem must equal the name its document states, the way a definition's does |
| Missing folder | Loads only the builtins and the seeded standard clocks, and creates nothing |
| Passed over | A directory inside a category or inside `codesets/`, and a file that is not `<n>.json` under `fields/` or `<name>.json` under a named category or `codesets/`; a reader ignores them and a writer leaves the directories alone |
| Publication | Writes populated documents, then removes every `.json` document it did not write from the categories and `codesets/`, and every one of those directories it left empty; separate file writes are not a directory-wide transaction |
| Seed | `config/fix`, tracked and written by `scripts/generate_fix_dictionary.py`, which states the specification's own fields alone; outside the [default registry](registry.md)'s order |

## Use

The counter is a scalar field; a reusable component defines one occurrence and the group references it. This example writes all three categories and one code set, with one field naming the dialect that contributed it, and reloads the complete graph.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, FixCode, FixRegistry, IOBase, FieldPath, StructType};

    let path = LocalFolder::temporary()?.path()?.join(format!("ygg-doc-store-{}", std::process::id()));
    let mut root = LocalFolder::new(&path)?;
    let mut count = DataType::Int32.nullable_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let mut id = DataType::utf8().nullable_field("PartyID");
    id.as_fix_mut().set_tag(448)?;
    id.as_fix_mut().set_branches(["venue"])?;
    let mut registry = FixRegistry::from_fields([count, id])?;
    let mut member = registry.field(448)?.clone();
    member.as_fix_mut().set_field_ref("PartyID")?;
    let mut party = DataType::from(StructType::from_fields([member])?).required_field("Party");
    party.as_fix_mut().set_identifiers(["448"])?;
    registry.insert(party.clone())?;
    let mut group = DataType::serie(party).nullable_field("Parties");
    group.as_fix_mut().set_counter(453)?;
    group.as_fix_mut().set_component("Party")?;
    registry.insert(group)?;
    let mut order = DataType::from(StructType::from_fields([])?).required_field("Order");
    order.as_fix_mut().set_msgtype("D")?;
    registry.insert(order)?;
    // A vocabulary is the dictionary's own, stated before the field that
    // reads by it names it; the store writes it under that name.
    registry.set_codeset("partyidsourcecodeset", &[FixCode::new("Proprietary", "D")])?;
    let mut source = DataType::utf8().nullable_field("PartyIDSource");
    source.as_fix_mut().set_tag(447)?;
    source.as_fix_mut().set_codeset("partyidsourcecodeset")?;
    registry.insert(source)?;

    // A commit writes the documents that moved and leaves the rest where
    // they lie, so it answers what it changed.
    let report = registry.commit(&mut root)?;
    assert!(!report.written.is_empty());
    assert!(report.removed.is_empty());
    assert!(path.join("fields/000000004.json").is_file());
    assert!(path.join("components/Party.json").is_file());
    assert!(path.join("groups/Parties.json").is_file());
    assert!(path.join("components/Order.json").is_file());
    assert!(path.join("codesets/partyidsourcecodeset.json").is_file());
    // The crate's own are written beside them, so a store states the whole
    // row: its tag block is one shard, its two Map groups two documents,
    // and the fixed row itself one more.
    assert!(path.join("fields/000000650.json").is_file());
    assert!(path.join("groups/identifiers.json").is_file());
    assert!(path.join("groups/metadata.json").is_file());
    assert!(path.join("components/fixmsg.json").is_file());
    // The three category directories and `codesets/` are the whole layout.
    assert_eq!(std::fs::read_dir(&path)?.count(), 4);
    let reloaded = FixRegistry::from_handle(&root)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));
    // The vocabulary comes back under its name, and the field by that name.
    let held = reloaded.codeset_of(reloaded.field(447)?).expect("the set the field reads by");
    assert_eq!(held.name(), "partyidsourcecodeset");
    assert_eq!(held.code_name("D"), Some("Proprietary"));
    // Membership travels inside the field's own document.
    assert!(reloaded.field(448)?.as_fix().has_branch("venue"));
    assert_eq!(reloaded.dialects(), ["venue"]);
    assert_eq!(reloaded.field_by_name("Party")?.as_fix().identifiers().collect::<Vec<_>>(), ["PartyID"]);
    assert_eq!(reloaded.field_by_counter(65020)?.name(), "identifiers");
    root.remove(true)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile
    import yggdryl
    from yggdryl import DataType, Field
    from yggdryl.fix import FixRegistry

    count = Field("NoPartyIDs", "int32")
    count.fix.tag = 453
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    party_id.fix.branches = ["venue"]
    registry = FixRegistry.from_fields([count, party_id])
    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    party = Field("Party", DataType.from_fields([member]), nullable=False)
    party.fix.identifiers = ["448"]
    registry.insert(party)
    group = yggdryl.serie("Parties", party)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.insert(group)
    order = Field("Order", DataType.from_fields([]), nullable=False)
    order.fix.msgtype = "D"
    registry.insert(order)
    # A vocabulary is the dictionary's own, stated before the field that reads
    # by it names it; the store writes it under that name.
    registry.set_codeset("partyidsourcecodeset", [{"value": "D", "name": "Proprietary"}])
    source = Field("PartyIDSource", "utf8")
    source.fix.tag = 447
    source.fix.codeset = "partyidsourcecodeset"
    registry.insert(source)

    with tempfile.TemporaryDirectory(prefix="ygg-doc-store-") as temporary:
        root = pathlib.Path(temporary) / "catalog"
        report = registry.commit(root)
        assert report["written"]
        assert report["removed"] == []
        assert (root / "fields/000000004.json").is_file()
        assert (root / "components/Party.json").is_file()
        assert (root / "groups/Parties.json").is_file()
        assert (root / "components/Order.json").is_file()
        assert (root / "codesets/partyidsourcecodeset.json").is_file()
        # The crate's own are written beside them, so a store states the whole
        # row: its tag block is one shard, its two Map groups two documents, and
        # the fixed row itself one more.
        assert (root / "fields/000000650.json").is_file()
        assert (root / "groups/identifiers.json").is_file()
        assert (root / "groups/metadata.json").is_file()
        assert (root / "components/fixmsg.json").is_file()
        # The three category directories and `codesets/` are the whole layout.
        assert sorted(child.name for child in root.iterdir()) == [
            "codesets",
            "components",
            "fields",
            "groups",
        ]
        reloaded = FixRegistry.from_handle(root)
        assert reloaded == registry
        assert reloaded.field_by_path("Parties.PartyID").fix.tag == 448
        # The vocabulary comes back under its name, and the field by that name.
        assert reloaded.field(447).fix.codeset == "partyidsourcecodeset"
        assert reloaded.codeset_of(reloaded.field(447)) == [
            {
                "value": "D",
                "name": "Proprietary",
                "description": None,
                "aliases": [],
                "group": None,
            }
        ]
        # Membership travels inside the field's own document.
        assert reloaded.field(448).fix.branches == ["venue"]
        assert reloaded.dialects() == ["venue"]
        assert reloaded.field_by_name("Party").fix.identifiers == ["PartyID"]
        assert reloaded.field_by_counter(65_020).name == "identifiers"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, fix } = require('yggdryl')

    const count = Field.from('NoPartyIDs: int32')
    count.fix.tag = 453
    const partyId = Field.from('PartyID: utf8')
    partyId.fix.tag = 448
    partyId.fix.branches = ['venue']
    const registry = fix.FixRegistry.fromFields([count, partyId])
    const member = registry.field(448)
    member.fix.fieldRef = 'PartyID'
    const party = fields.struct('Party', [member], { nullable: false })
    party.fix.identifiers = ['448']
    registry.insert(party)
    const group = fields.serie('Parties', party)
    group.fix.counter = 453
    group.fix.component = 'Party'
    registry.insert(group)
    const order = fields.struct('Order', [], { nullable: false })
    order.fix.msgtype = 'D'
    registry.insert(order)
    // A vocabulary is the dictionary's own, stated before the field that reads
    // by it names it; the store writes it under that name.
    registry.setCodeset('partyidsourcecodeset', [{ value: 'D', name: 'Proprietary' }])
    const source = Field.from('PartyIDSource: utf8')
    source.fix.tag = 447
    source.fix.codeset = 'partyidsourcecodeset'
    registry.insert(source)

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-doc-store-'))
    try {
      const report = registry.commit(root)
      assert.ok(report.written.length > 0)
      assert.deepEqual(report.removed, [])
      for (const file of ['fields/000000004.json', 'components/Party.json', 'groups/Parties.json', 'components/Order.json', 'codesets/partyidsourcecodeset.json']) {
        assert.ok(fs.existsSync(path.join(root, file)))
      }
      // The crate's own are written beside them, so a store states the whole row:
      // its tag block is one shard, its two Map groups two documents, and the
      // fixed row itself one more.
      for (const file of ['fields/000000650.json', 'groups/identifiers.json', 'groups/metadata.json', 'components/fixmsg.json']) {
        assert.ok(fs.existsSync(path.join(root, file)))
      }
      // The three category directories and `codesets/` are the whole layout.
      assert.deepEqual(fs.readdirSync(root).sort(), ['codesets', 'components', 'fields', 'groups'])
      const reloaded = fix.FixRegistry.fromHandle(root)
      assert.ok(reloaded.equals(registry))
      assert.equal(reloaded.fieldByPath('Parties.PartyID').fix.tag, 448)
      // The vocabulary comes back under its name, and the field by that name.
      assert.equal(reloaded.field(447).fix.codeset, 'partyidsourcecodeset')
      const held = reloaded.codesetOf(reloaded.field(447))
      assert.equal(held.name, 'partyidsourcecodeset')
      assert.equal(held.codes[0].name, 'Proprietary')
      // Membership travels inside the field's own document.
      assert.deepEqual(reloaded.field(448).fix.branches, ['venue'])
      assert.deepEqual(reloaded.dialects(), ['venue'])
      assert.deepEqual(reloaded.fieldByName('Party').fix.identifiers, ['PartyID'])
      assert.equal(reloaded.fieldByCounter(65020).name, 'identifiers')
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

## Categories and shards

Only scalar fields use numeric shards; alternate tags do not create additional copies. Named files use the stored canonical definition name, so case-only updates retain the existing filename.

```text
<root>/codesets/sidecodeset.json
<root>/fields/000000000.json
<root>/fields/000000050.json
<root>/components/Order.json
<root>/components/Party.json
<root>/groups/Parties.json
```

Tag 55 belongs in `fields/000000000.json`; tag 5001 belongs in `fields/000000050.json`, whichever dictionary defined it. The nine digits are what makes a listing of the shards read in tag order. Every field's canonical tag must agree with its document's shard. Two fields on one tag - a dialect's own name over a tag the specification holds - share the shard, the field the bare tag answers written first; a reader loads a shard in file order, so the holder survives a round trip. A field the specification alone defines states no `FIX:branches`.

## The vocabularies in `codesets/`

A [code set](registry.md#a-field-names-the-code-set-it-reads-by) is a vocabulary rather than a definition, so it is one document of its own rather than a field's property: `codesets/<name>.json` states the name it is filed under and its members in the set's own order, and however many fields read by it, the members are written once.

```json
{
  "name": "advsidecodeset",
  "codes": [
    {"value": "B", "name": "Buy", "doc": "Buy"},
    {"value": "S", "name": "Sell", "doc": "Sell"},
    {"value": "T", "name": "Trade", "doc": "Trade"},
    {"value": "X", "name": "Cross", "doc": "Cross"}
  ]
}
```

The file says what it is without its path: the stem and the stated name have to agree, exactly as a component's do, because the stem is how the set is addressed. The sets are read before the three categories, since a field naming one the dictionary does not hold is refused, and they are written and pruned beside them - a set nothing states any more leaves on the next write, and `codesets/` goes with it when the last one does. A directory inside `codesets/`, and a file that is not `<name>.json`, are passed over the way a category's are.

## Compact references

A persisted child refers to one canonical definition using `FIX:field`, `FIX:component`, or `FIX:group`; its `Null` datatype is replaced at intake. A `FIX:component` or `FIX:group` occurrence states no tag of its own: the derived tag belongs to the canonical definition, and the resolver leaves it there rather than inheriting it, so a catalog compares equal to itself across a round trip. A placeholder states no membership either, because that is the target's to carry. The loaded object is a resolved `Field`, so message readers do not perform filesystem access or resolve schema references per row.

```json
{
  "name": "PartyID",
  "dtype": {"type": "null"},
  "nullable": true,
  "metadata": {"FIX:field": "partyid"}
}
```

A Serie/LargeSerie group stores a non-null item referencing its occurrence component; its root records the separate scalar counter and component relationship beside the group's own derived `FIX:tag`. A Map instead keeps its required entries Struct, non-null key and sortedness; a referenced entries component carries only reference metadata, and reload uses the same occurrence resolver.

The writer compacts resolved references again, keeping each canonical definition in one document; a reference carries its target's `FIX:tag` beside the name, so a reader resolves it by the identity the pair makes, and references to `identifiers` resolve against the registry-owned builtin rather than against the `groups/identifiers.json` a write leaves beside them. Missing targets, conflicting reference kinds, cycles, and nesting beyond 64 are located intake errors; independent occurrence metadata overrides are refused, and canonical updates refresh references atomically.

Identifier declarations normalize through the [component setter](registry.md#component-identifiers) after these references resolve, so stored aliases or decimal tags become canonical direct member names in final component order. A malformed or ambiguous declaration fails the complete load rather than losing a selection silently.

## Membership

A dictionary is a membership, not a namespace: the store has no document for one. Each field and named definition carries the names of the dictionaries that contributed it as `FIX:branches`, written as one comma-separated string, folded to ASCII lowercase, deduplicated and sorted, so two registries built from the same dictionaries in any order write the same bytes and hash alike. `FixRegistry::dialects` (`dialects()` in Python and JavaScript) lists the distinct names across every category, and `has_branch` / `branches` on the field read one field's; none of them takes part in a lookup. A document written with no `FIX:branches` is a field the specification alone defines, which is every field of the tracked seed.

## Complete JSON snapshots

`FixRegistry::into_json` and `from_json` use one object with `codesets`, `fields`, `components`, and `groups` arrays and no other key; a snapshot carrying a `messages` key is refused by name. The `codesets` array leads and is read first, each entry the `{name, codes}` object a `codesets/<name>.json` holds, because a field naming a set the snapshot does not state is refused like any unresolved reference. They reuse the folder store's compact references, builtin omission and resolver, so a snapshot retains named definitions, contextual groups, identifier declarations, the vocabularies and every field's membership; collecting ordinary scalar iteration does not preserve a catalog.

Python pickle and copy preserve this full graph. Node `intoJson` / `fromJson`, `toJSON`, and `clone` do the same; `stable_hash` / `stableHash` derives from native registry state, membership included like any other metadata.

## The tracked seed

The committed `config/fix` catalog contains 6,241 scalar fields in 65 shards, 928 components - 181 messages carrying `FIX:msgtype` and `FIX:msgcat` - and 580 groups. Loading adds 29 crate scalar definitions, including the `srcuuids` serie, six normalized identifier codes, the execution and recording clocks and the session-event key `msgsesseventid`, plus two Map groups: 6,270 scalar fields, 582 groups, 928 components and 181 message types in the live registry, 7,780 definitions total. The generated catalog holds 735 shared code sets in `codesets/`; the builtin `msgcatcodeset` makes 736 live sets.

Beside those 2,308 the tracked tree carries the crate's own dump, which `commit` writes and a read passes over: `fields/000000650.json`, `groups/identifiers.json`, `groups/metadata.json` and the fixed row `components/fixmsg.json`. The generator neither writes nor removes them, and its `--check` ignores them.

It contains 7,751 code records across those 735 generated sets, read by 2,027 fields. The same dictionary held 27,209 of them when every field carried its own vocabulary, which is what naming each one once buys: the 65 field shards are 2,271,320 bytes where they were 5,874,130, against 896,022 for all of `codesets/`. Generated names are canonical lowercase and standard display names remain metadata. Each of the 1,508 persisted named definitions states a unique derived tag - `groups/parties.json` is 209321 - and 109 components declare their matching direct [identifiers](registry.md#component-identifiers), the property omitted where none match.

Fifty-six fields are ones FIX Latest has since removed, [kept under their own tags](registry.md#the-dictionary-holds-one-reading-and-filters-by-no-version) and marked `FIX:deprecated` with the version that removed them; none carries [`FIX:replacements`](registry.md#a-field-carries-what-replaced-it), because what the specification retired - 100 retirements of 37 fields - is [the crate's own table](registry.md#what-the-specification-retired) and not the dictionary's to state; 29 carry [`FIX:derivation`](registry.md#a-field-carries-how-it-is-derived), one term each, and no column of this crate's derives at all. No document states a `FIX:branches` and no document stores the derived `FixId`.

The source is the [pinned FIX Orchestra repository](https://github.com/FIXTradingCommunity/orchestrations/blob/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml), with the [documented naming rules](registry.md#group-names). This is a complete resolved catalog workload, so its load/write timings are not comparable to a scalar-only seed or a small FIX-version subset.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{fix_crate_fields, FixId, FixRegistry, FieldPath};

    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(seed)?)?;

    // Names are folded once, so a caller spells one however they have it and
    // the specification's own spelling stays on `display`.
    assert_eq!(registry.field_by_tag(55)?.name(), "symbol");
    assert_eq!(registry.field_by_name("SYMBOL")?.name(), "symbol");
    assert_eq!(registry.field_by_tag(150)?.display(), Some("ExecType"));
    assert_eq!(registry.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("ClOrdID")?.display(), Some("ClOrdID"));
    // The id is the tag and the folded name, derived on every read and held
    // in no document, so the stored dictionary spells it nowhere.
    let symbol = FixId::of(55, "Symbol")?;
    assert_eq!(registry.field_by_tag(55)?.as_fix().id()?, Some(symbol));
    assert_eq!(registry.field_by_id(symbol)?.name(), "symbol");
    // Every field is a specification field or one of the crate's own, so no
    // field names a dialect that contributed it.
    assert!(registry.dialects().is_empty());
    assert!(registry.iter().all(|field| field.as_fix().branches().next().is_none()));
    // The crate's own definitions are in the store and in the registry alike:
    // 29 scalar fields, including the `srcuuids` serie, and two Map groups.
    assert_eq!(fix_crate_fields()?.len(), 31);
    assert_eq!(registry.iter().count(), 7_780, "the fields and the definitions");
    assert_eq!(registry.len(), 7_780, "the fields, the components and the groups");
    assert_eq!(registry.field_by_counter(65_020)?.name(), "identifiers");
    assert_eq!(registry.msgtype("D")?.name(), "newordersingle");
    // The vocabularies are held beside them, one per name, and a field
    // reaches its own through the name it states.
    assert_eq!(registry.codesets().len(), 736);
    let side = registry.codeset_of(registry.field_by_tag(54)?).expect("the Side set");
    assert_eq!(side.name(), "sidecodeset");
    assert_eq!(side.code_name("1"), Some("Buy"));
    ```

=== "Python"

    ```python
    import pathlib

    from yggdryl.fix import FixRegistry, fix_crate_fields

    # The seed this repository tracks, named from the repository root.
    seed = pathlib.Path("config/fix").resolve()
    registry = FixRegistry.from_handle(seed)

    symbol = registry.field_by_tag(55)
    assert symbol.name == "symbol"
    assert registry.field_by_name("SYMBOL").name == "symbol"
    assert registry.field_by_tag(150).display == "ExecType"
    assert registry.field_by_path("Parties.PartyID").fix.tag == 448
    assert registry.field_by_name("ClOrdID").display == "ClOrdID"
    # The id is an int derived from the tag and the folded name, held in no
    # document; `field_by_id` is the one lookup that reads an int as an id.
    assert isinstance(symbol.fix.id, int)
    assert registry.field_by_id(symbol.fix.id).name == "symbol"
    # Every field is a specification field or one of the crate's own, so no
    # field names a dialect that contributed it.
    assert registry.dialects() == []
    assert all(field.fix.branches == [] for field in registry)
    # The crate's own definitions are in the store and in the registry alike:
    # 29 scalar fields, including the `srcuuids` serie, and two Map groups.
    assert len(fix_crate_fields()) == 31
    assert sum(1 for _ in registry) == 6_270
    assert len(registry) == 7_780
    assert registry.field_by_counter(65_020).name == "identifiers"
    assert registry.msgtype("D").name == "newordersingle"
    # The vocabularies are held beside them, one per name, and a field reaches
    # its own through the name it states.
    assert len(registry.codeset_names()) == 736
    assert registry.field_by_tag(54).fix.codeset == "sidecodeset"
    buy, = (code for code in registry.codeset("sidecodeset") if code["value"] == "1")
    assert buy["name"] == "Buy"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    // The seed this repository tracks, named from the repository root.
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))

    const symbol = registry.fieldByTag(55)
    assert.equal(symbol.name, 'symbol')
    assert.equal(registry.fieldByName('SYMBOL').name, 'symbol')
    assert.equal(registry.fieldByTag(150).display, 'ExecType')
    assert.equal(registry.fieldByPath('Parties.PartyID').fix.tag, 448)
    assert.equal(registry.fieldByName('ClOrdID').display, 'ClOrdID')
    // The id is a number derived from the tag and the folded name, held in no
    // document; `fieldById` is the one lookup that reads a number as an id.
    assert.equal(typeof symbol.fix.id, 'number')
    assert.equal(registry.fieldById(symbol.fix.id).name, 'symbol')
    // Every field is a specification field or one of the crate's own, so no
    // field names a dialect that contributed it.
    assert.deepEqual(registry.dialects(), [])
    assert.ok([...registry].every((field) => field.fix.branches.length === 0))
    // The crate's own definitions are in the store and in the registry alike:
    // 29 scalar fields, including the `srcuuids` serie, and two Map
    // groups. A Node registry sizes and iterates every field, the components
    // and the groups among them.
    assert.equal(fix.crateFields().length, 31)
    assert.equal(registry.size, 7780)
    assert.equal([...registry].length, registry.size)
    assert.equal(registry.fieldByCounter(65020).name, 'identifiers')
    assert.equal(registry.msgtype('D').name, 'newordersingle')
    // The vocabularies are held beside them, one per name, and a field
    // reaches its own through the name it states.
    assert.equal(registry.codesetNames().length, 736)
    assert.equal(registry.fieldByTag(54).fix.codeset, 'sidecodeset')
    assert.equal(registry.codeName('sidecodeset', '1'), 'Buy')
    ```

### Datatypes in the seed

A field document stores the crate's own [datatype document](../types/datatype.md), so a FIX datatype resolves once when the seed is generated and the reader parses no spelling. Every text datatype is the one `string` tag with its parameters, and every byte datatype the one `binary` tag.

| FIX datatype | Stored `dtype` | Reads as |
| --- | --- | --- |
| `String`, `char`, `MultipleCharValue`, `MultipleStringValue`, `XID`, `XIDREF`, `Pattern` | `{"type": "string"}` | `utf8` |
| `MonthYear`, `Tenor` | `{"type": "string", "layout": "fixed_ascii", "fixed": 8}` | `fixed_ascii(8)` |
| `Language` | `{"type": "string", "layout": "fixed_ascii", "fixed": 2}` | `fixed_ascii(2)` |
| `data`, `XMLData` | `{"type": "binary"}` | `binary` |
| `Country`, `Currency`, `Exchange` | `{"type": "country"}`, `{"type": "currency"}`, `{"type": "mic"}` | the [code](../types/codes/index.md) |

Regenerate the seed from the repository root, and check it for drift without a network:

```bash
python scripts/generate_fix_dictionary.py
python scripts/generate_fix_dictionary.py --check
```

## Edges

- A missing category contributes no persisted definitions; a missing root still answers the builtins and the seeded standard clocks, and creates nothing.
- A stored `SendingTime(52)` or `TransactTime(60)` is never replaced by the seed, and two stored documents declaring one of them fail the load like any duplicate identity.
- A stored `components/fixmsg.json`, and any document on a crate tag, is read past: the crate's own definition is the one that types a row, so a dump an older version wrote can never change what a reader builds.
- Two documents declaring one identity - the same tag under the same folded name - or one named declaration twice fail instead of replacing an earlier source record; the same tag under another name is a second field beside the holder, as in memory.
- Scalar arrays and individual named documents have distinct shapes; loading the wrong shape names the document.
- Wrong shard, category datatype, counter type, or reference target is refused before a registry is returned.
- Canonical definition names must form safe single path segments: nonempty ASCII letters, digits, underscore, hyphen, or dot, and never `.` or `..`; a code set's name is held to the same rule, at the store and at the field that names it.
- A `codesets/<name>.json` whose stated name is not its filename stem fails the load naming the set, and two documents stating one name fail as a duplicate; a field naming a set the store does not hold fails the load naming `codesets`.
- A directory inside a category or inside `codesets/` is not a store's layout: what it holds is passed over on read and left alone by publication, so nothing is read as a dialect's own shard.
- A `README` beside the field shards is ignored on read and left alone by publication; only a decimal `<n>.json` is read, whatever width it was written at.
- Builtin scalar and Map group definitions are written like any other, and read past on load in favour of their native owner; an incoming document cannot override a builtin, by restating its name under another tag or by restating the builtin itself.
- A `FIX:branches` value is held to the membership grammar: a name that is empty or carries a comma is refused naming the key.
- Removing the last definition from a shard or category, or the last code set, removes its owned document or directory on the next write.
- A code set a held field still reads by cannot be removed, so a store never writes a field naming a vocabulary the tree does not hold.
- Folder writes publish individual documents; a backend failure can leave already published files visible.
- `config/fix` in the Python and JavaScript seed examples resolves against the working directory, so run them from the repository root.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix store
    cargo test --features internals -p yggdryl --test fix -- mod_::internal::shard_arithmetic
    cargo test -p yggdryl --test iobase_calls fix_catalog_storage_resolves_each_root_path_once
    ```

=== "Python"

    ```bash
    python -m pytest python/tests/test_fix.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/catalog.test.js
    ```

## Performance

The Rust column is one release run of the Criterion target on one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.94.1 release (thin LTO, one codegen unit), 100 samples. The Python and Node columns are an earlier release run on Windows, AMD Ryzen 5 150 with 12 logical CPUs, Python 3.12.13 and Node 24.18, 2,000 boundary iterations each with expensive folder loads reduced to a smaller number of rounds; the two hosts differ, so a row compares a language against its own boundary and not against the Rust figure.

| Folder operation | Rust estimate | Python | Node |
| --- | ---: | ---: | ---: |
| Load full seed | 1.13 s | 2.03 s | 1 op/s, rounded |
| Load 200 scalar fields | Not measured by this Rust fixture | 4.73 ms | 128 ops/s |
| Load 1 / 10 / 100 field shards | 161 us / 945 us / 10.0 ms | Not isolated | Not isolated |
| Write 100 field shards | 30.7 ms | Not isolated | Not isolated |
| Load full catalog with second dialect | 1.07 s | Not isolated | Not isolated |
| Write full catalog with second dialect | 558 ms | Not isolated | Not isolated |

Different processes and sample counts make these observed boundary costs, not a language speed ranking. The full-seed load resolves referenced components and groups and compiles message/group indexes; scalar-shard fixtures measure a smaller operation. The second-dialect rows load and write the seed with a venue's fields merged in, tags from 5000: they land in the store's numeric shards beside the specification's, membership written on each and read back with it.

| Native full-seed snapshot | Rust estimate |
| --- | ---: |
| `into_json` | 110 ms |
| `from_json` | 1.06 s |
| `stable_hash`, one digester state allocation | 191 ms |

| Small catalog boundary | Python | Node |
| --- | ---: | ---: |
| Snapshot encode | 29.2 us | 33,590 ops/s |
| Snapshot decode | 92.2 us | 9,421 ops/s |
| Content hash | 3.02 us | 365,490 ops/s |
| Independent copy | 2.00 us | 177,936 ops/s |

The small boundary fixture adds two fields plus one component, group and message beside the builtins. It is intentionally distinct from the full-seed Rust snapshot fixture; the displayed measurements predate the crate's own definitions becoming what they are now and were not rerun for this change.

Root navigation is asserted with `Counted`: loading resolves `codesets/` and the three category roots, one `child_by_path` each, so a load of a registry holding nothing but the crate's own definitions costs four. A write resolves one path per document it publishes and one per folder it then prunes - nine for that same registry: five documents and four folders. `codesets/` is navigated even by a dictionary holding no set, because a write that holds none has to take away the folder a previous write left, and a folder cannot be removed without being reached. These counts cover the root handle only; document reads and writes happen on child handles and are outside that tally.

Regenerate with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- fix/store
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
