# Store

A FIX catalog persists through one [`IOBase`](../holder/index.md) folder as four category directories. What each dialect contributed travels inside the document of the field it contributed to; nothing in the layout is keyed by a dialect.

## Contract

| Aspect | Rule |
| --- | --- |
| Owner | `FixRegistry::from_handle` and `write_into`; bindings redirect to the native loader/writer |
| Fields | `fields/<tag / 100>.json`; each document is an array of tagged scalar fields, tag-major, the holder of a shared tag first |
| Named definitions | `messages/<name>.json`, `components/<name>.json`, `groups/<name>.json`; one native `Field` per document, stating the `fix:tag` derived from the definition's name |
| Enums | Inline `fix:codes` metadata on each scalar field |
| References | Compact native child fields retain reference metadata and use `Null` as the unresolved datatype; intake resolves them to canonical native fields |
| Membership | `fix:branches` metadata inside each field and named definition document: the sorted, lowercase, comma-separated names of the dictionaries that contributed it; that document is the only place a dictionary is recorded |
| Identity | Derived on every read from `fix:tag` and the field's name; no document holds an id |
| Crate fields | The crate's own twenty fields, standard tags from 65000, are never written; every registry holds them from construction, and a stored copy of one is read past |
| Validation | Category shape, shard arithmetic, tag and name identity, references, codes, cycles, and depth are checked before exposing the registry |
| Missing folder | Loads an empty registry and creates nothing |
| Passed over | A directory inside a category, and a file that is not `<n>.json` under `fields/` or `<name>.json` under a named category; a reader ignores them and a writer leaves the directories alone |
| Publication | Writes populated documents, then removes every `.json` document it did not write from the categories it holds and every empty category directory; separate file writes are not a directory-wide transaction |
| Seed | `config/fix`, tracked and written by `write_into`; outside the [default registry](registry.md)'s order |

## Use

The counter is a scalar field; a reusable component defines one occurrence and the group references it. This example writes all four categories, with one field naming the dialect that contributed it, and reloads the complete graph.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixCategory, FixRegistry, IOBase, FieldPath};

    let path = Folder::temporary()?.path()?.join(format!("ygg-doc-store-{}", std::process::id()));
    let mut root = Folder::new(&path)?;
    let mut count = DataType::Int32.nullable_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let mut id = DataType::utf8().nullable_field("PartyID");
    id.as_fix_mut().set_tag(448)?;
    id.as_fix_mut().set_branches(["venue"])?;
    let mut registry = FixRegistry::from_fields([count, id])?;
    let mut member = registry.field(448)?.clone();
    member.as_fix_mut().set_field_ref("PartyID")?;
    let party = DataType::from_fields([member])?.required_field("Party");
    registry.create_definition(FixCategory::Components, party.clone())?;
    let mut group = DataType::list(party).nullable_field("Parties");
    group.as_fix_mut().set_counter(453)?;
    group.as_fix_mut().set_component("Party")?;
    registry.create_definition(FixCategory::Groups, group)?;
    let mut order = DataType::from_fields([])?.required_field("Order");
    order.as_fix_mut().set_msgtype("D")?;
    registry.create_definition(FixCategory::Messages, order)?;

    registry.write_into(&mut root)?;
    assert!(path.join("fields/4.json").is_file());
    assert!(path.join("components/Party.json").is_file());
    assert!(path.join("groups/Parties.json").is_file());
    assert!(path.join("messages/Order.json").is_file());
    // The four category directories are the whole layout.
    assert_eq!(std::fs::read_dir(&path)?.count(), 4);
    let reloaded = FixRegistry::from_handle(&root)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_path(&FieldPath::from_str("Parties.PartyID")?)?.as_fix().tag()?, Some(448));
    // Membership travels inside the field's own document.
    assert!(reloaded.field(448)?.as_fix().has_branch("venue"));
    assert_eq!(reloaded.dialects(), ["venue"]);
    root.remove(true)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile
    from yggdryl import DataType, Field, types
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
    registry.create_definition("components", party)
    group = types.list("Parties", party)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.create_definition("groups", group)
    order = Field("Order", DataType.from_fields([]), nullable=False)
    order.fix.msgtype = "D"
    registry.create_definition("messages", order)

    with tempfile.TemporaryDirectory(prefix="ygg-doc-store-") as temporary:
        root = pathlib.Path(temporary) / "catalog"
        registry.write_into(root)
        assert (root / "fields/4.json").is_file()
        assert (root / "components/Party.json").is_file()
        assert (root / "groups/Parties.json").is_file()
        assert (root / "messages/Order.json").is_file()
        # The four category directories are the whole layout.
        assert sorted(child.name for child in root.iterdir()) == ["components", "fields", "groups", "messages"]
        reloaded = FixRegistry.from_handle(root)
        assert reloaded == registry
        assert reloaded.field_by_path("Parties.PartyID").fix.tag == 448
        # Membership travels inside the field's own document.
        assert reloaded.field(448).fix.branches == ["venue"]
        assert reloaded.dialects() == ["venue"]
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
    registry.createDefinition('components', party)
    const group = fields.list('Parties', party)
    group.fix.counter = 453
    group.fix.component = 'Party'
    registry.createDefinition('groups', group)
    const order = fields.struct('Order', [], { nullable: false })
    order.fix.msgtype = 'D'
    registry.createDefinition('messages', order)

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-doc-store-'))
    try {
      registry.writeInto(root)
      for (const file of ['fields/4.json', 'components/Party.json', 'groups/Parties.json', 'messages/Order.json']) {
        assert.ok(fs.existsSync(path.join(root, file)))
      }
      // The four category directories are the whole layout.
      assert.deepEqual(fs.readdirSync(root).sort(), ['components', 'fields', 'groups', 'messages'])
      const reloaded = fix.FixRegistry.fromHandle(root)
      assert.ok(reloaded.equals(registry))
      assert.equal(reloaded.fieldByPath('Parties.PartyID').fix.tag, 448)
      // Membership travels inside the field's own document.
      assert.deepEqual(reloaded.field(448).fix.branches, ['venue'])
      assert.deepEqual(reloaded.dialects(), ['venue'])
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

## Categories and shards

Only scalar fields use numeric shards; alternate tags do not create additional copies. Named files use the stored canonical definition name, so case-only updates retain the existing filename.

```text
<root>/fields/0.json
<root>/fields/50.json
<root>/components/Party.json
<root>/groups/Parties.json
<root>/messages/Order.json
```

Tag 55 belongs in `fields/0.json`; tag 5001 belongs in `fields/50.json`, whichever dictionary defined it. Every field's canonical tag must agree with its document's shard. Two fields on one tag - a dialect's own name over a tag the specification holds - share the shard, the field the bare tag answers written first; a reader loads a shard in file order, so the holder survives a round trip. A field the specification alone defines states no `fix:branches`.

## Compact references

A persisted child refers to one canonical definition using `fix:field`, `fix:component`, or `fix:group`; its `Null` datatype is replaced at intake. A `fix:component` or `fix:group` occurrence states no tag of its own: the derived tag belongs to the canonical definition, and the resolver leaves it there rather than inheriting it, so a catalog compares equal to itself across a round trip. A placeholder states no membership either, because that is the target's to carry. The loaded object is a resolved `Field`, so message readers do not perform filesystem access or resolve schema references per row.

```json
{
  "name": "PartyID",
  "dtype": {"type": "null"},
  "nullable": true,
  "metadata": {"fix:field": "partyid"}
}
```

A group stores a List or LargeList whose non-null item references the occurrence component; its root records the counter tag and component relationship, beside its own `fix:tag`, which is derived from the group's name and is never the counter's. The writer compacts resolved references again, keeping each canonical definition in one document. Missing targets, conflicting reference kinds, cycles, and nesting beyond 64 are located intake errors. Independent occurrence metadata overrides are refused; canonical metadata updates refresh their references atomically.

## Membership

A dictionary is a membership, not a namespace: the store has no document for one. Each field and named definition carries the names of the dictionaries that contributed it as `fix:branches`, written as one comma-separated string, folded to ASCII lowercase, deduplicated and sorted, so two registries built from the same dictionaries in any order write the same bytes and hash alike. `FixRegistry::dialects` (`dialects()` in Python and JavaScript) lists the distinct names across every category, and `has_branch` / `branches` on the field read one field's; none of them takes part in a lookup. A document written with no `fix:branches` is a field the specification alone defines, which is every field of the tracked seed.

## Complete JSON snapshots

`FixRegistry::into_json` and `from_json` use one object with `fields`, `components`, `groups`, and `messages` arrays and no other key. They reuse the folder store's compact references and resolver, so a snapshot retains named definitions, contextual groups, inline enums, and every field's membership; collecting ordinary scalar iteration does not preserve a catalog.

Python pickle and copy preserve this full graph. Node `intoJson` / `fromJson`, `toJSON`, and `clone` do the same; `stable_hash` / `stableHash` derives from native registry state, membership included like any other metadata.

## The tracked seed

The committed `config/fix` catalog contains 6,241 scalar fields in 65 shards, 747 components, 580 groups, and 181 messages: 1,573 JSON documents totaling 9,271,670 bytes. It contains 27,209 inline code records on 2,026 fields; generated names are canonical lowercase and standard display names remain metadata. Each of the 1,508 named definitions states the tag derived from its name - `groups/parties.json` is 209321 - and no two share one. Thirty-eight of the fields are ones FIX has since removed, kept with the version that [removed them](registry.md#versions-are-a-filter-on-the-read); 37 carry [`fix:replacements`](registry.md#a-field-carries-what-replaced-it), 100 entries in all. No document states a `fix:branches` and no document states an id.

The source is the [pinned FIX Orchestra repository](https://github.com/FIXTradingCommunity/orchestrations/blob/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml), with the [documented naming rules](registry.md#group-names). This is a complete resolved catalog workload, so its load/write timings are not comparable to a scalar-only seed or a small FIX-version subset.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixId, FixRegistry, FieldPath};

    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

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
    // The whole published dictionary, not a sample of it.
    assert!(registry.len() > 6_000);
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
    assert len(fix_crate_fields()) == 20
    # The whole published dictionary, not a sample of it.
    assert len(registry) > 6_000
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
    assert.equal(fix.crateFields().length, 20)
    // The whole published dictionary, not a sample of it.
    assert.ok(registry.size > 6_000)
    ```

### Datatypes in the seed

A field document stores the crate's own [datatype document](../types/datatype.md), so a FIX datatype resolves once when the seed is generated and the reader parses no spelling. Every text datatype is the one `string` tag with its parameters, and every byte datatype the one `binary` tag.

| FIX datatype | Stored `dtype` | Reads as |
| --- | --- | --- |
| `String`, `char`, `MultipleCharValue`, `MultipleStringValue`, `XID`, `XIDREF`, `Pattern` | `{"type": "string"}` | `utf8` |
| `MonthYear`, `Tenor` | `{"type": "string", "layout": "fixed_string", "charset": "us-ascii", "fixed": 8}` | `fixed_ascii(8)` |
| `Language` | `{"type": "string", "layout": "fixed_string", "charset": "us-ascii", "fixed": 2}` | `fixed_ascii(2)` |
| `data`, `XMLData` | `{"type": "binary"}` | `binary` |
| `Country`, `Currency`, `Exchange` | `{"type": "country"}`, `{"type": "currency"}`, `{"type": "mic"}` | the [code](../types/codes.md) |

The same document is what a `fix:lineage` entry's `type` holds. Regenerate the seed from the repository root, and check it for drift without a network:

```bash
python scripts/generate_fix_dictionary.py
python scripts/generate_fix_dictionary.py --check
```

## Edges

- A missing category is empty; reading a missing root creates nothing.
- Two documents declaring one identity - the same tag under the same folded name - or one named declaration twice fail instead of replacing an earlier source record; the same tag under another name is a second field beside the holder, as in memory.
- Scalar arrays and individual named documents have distinct shapes; loading the wrong shape names the document.
- Wrong shard, category datatype, counter type, or reference target is refused before a registry is returned.
- Canonical definition names must form safe single path segments: nonempty ASCII letters, digits, underscore, hyphen, or dot, and never `.` or `..`.
- A directory inside a category is not a store's layout: what it holds is passed over on read and left alone by publication, so nothing is read as a dialect's own shard.
- A `README` beside the field shards is ignored on read and left alone by publication; only `<n>.json` with a decimal `n` is read.
- A stored document holding one of the crate's own fields, a standard tag from 65000, is read past, and `write_into` writes none of them, so a store never holds a copy that could drift from the crate's.
- A `fix:branches` value is validated as an alias is: a name that is empty or carries a comma is refused naming the key.
- Removing the last definition from a shard or category removes its owned document or directory on the next write.
- Folder writes publish individual documents; a backend failure can leave already published files visible.
- `config/fix` in the Python and JavaScript seed examples resolves against the working directory, so run them from the repository root.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix store
    cargo test -p yggdryl --lib fix::tests::shard_arithmetic
    cargo test -p yggdryl --test iobase_calls fix_catalog_storage_resolves_each_root_path_once
    ```

=== "Python"

    ```bash
    python -m pytest python/tests/fix/test_catalog.py
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

The small boundary fixture contains two fields plus one component, group, and message. It is intentionally distinct from the full-seed Rust snapshot fixture.

Root navigation is asserted with `Counted`: loading resolves the four category roots (`child_by_path=4`); writing a one-shard catalog resolves those four paths plus its shard (`child_by_path=5`). These counts cover the root handle only; document reads/writes occur on child handles and are outside that tally.

Regenerate with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- fix/store
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
