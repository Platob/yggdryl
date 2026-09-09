# Store

A FIX catalog persists through one [`IOBase`](../holder/index.md) folder as four category directories and an optional branch manifest.

## Contract

| Aspect | Rule |
| --- | --- |
| Owner | `FixRegistry::from_handle` and `write_into`; bindings redirect to the native loader/writer |
| Fields | `fields/<tag / 100>.json`, or `fields/<branch>/<tag / 100>.json`; each document is an array of tagged scalar fields |
| Named definitions | `messages/<name>.json`, `components/<name>.json`, `groups/<name>.json`, with a branch directory when needed; one native `Field` per document, stating the `fix:tag` derived from the definition's name |
| Enums | Inline `fix:codes` metadata on each scalar field |
| References | Compact native child fields retain reference metadata and use `Null` as the unresolved datatype; intake resolves them to canonical native fields |
| Branch manifest | Optional `branches.json`, containing named branch declarations and aliases |
| Crate fields | The crate's own twenty fields, standard tags from 65000, are never written; every registry holds them from construction, and a stored copy of one is read past |
| Validation | Category shape, shard arithmetic, name/branch identity, references, codes, cycles, and depth are checked before exposing the registry |
| Missing folder | Loads an empty registry and creates nothing |
| Refused | A root still holding `records/`; no migration, no backward compatibility |
| Publication | Writes populated documents, then removes stale owned documents and empty category directories; separate file writes are not a directory-wide transaction |
| Seed | `config/fix`, tracked and written by `write_into`; outside the [default registry](registry.md)'s order |

## Use

The counter is a scalar field; a reusable component defines one occurrence and the group references it. This example writes all four categories and reloads their complete graph.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixCategory, FixRegistry, IOBase};

    let path = Folder::temporary()?.path()?.join(format!("ygg-doc-store-{}", std::process::id()));
    let mut root = Folder::new(&path)?;
    let mut count = DataType::Int32.nullable_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let mut id = DataType::Utf8.nullable_field("PartyID");
    id.as_fix_mut().set_tag(448)?;
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
    let reloaded = FixRegistry::from_handle(&root)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_path("Parties.PartyID", None)?.as_fix().tag()?, Some(448));
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
        reloaded = FixRegistry.from_handle(root)
        assert reloaded == registry
        assert reloaded.field_by_path("Parties.PartyID").fix.tag == 448
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
      const reloaded = fix.FixRegistry.fromHandle(root)
      assert.ok(reloaded.equals(registry))
      assert.equal(reloaded.fieldByPath('Parties.PartyID').fix.tag, 448)
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

## Categories and shards

Only scalar fields use numeric shards; alternate tags do not create additional copies. Named files use the stored canonical definition name, so case-only updates retain the existing filename.

```text
<root>/fields/0.json
<root>/fields/cme/50.json
<root>/components/Party.json
<root>/groups/Parties.json
<root>/messages/Order.json
<root>/messages/cme/VenueOrder.json
<root>/branches.json
```

Tag `55:` belongs in `fields/0.json`; `5001:cme` belongs in `fields/cme/50.json`. Every field's own identity must agree with its directory and document location. A standard definition omits `fix:branch`.

## Compact references

A persisted child refers to one canonical definition using `fix:field`, `fix:component`, or `fix:group`; its `Null` datatype is replaced at intake. A `fix:component` or `fix:group` occurrence states no tag of its own: the derived tag belongs to the canonical definition, and the resolver leaves it there rather than inheriting it, so a catalog compares equal to itself across a round trip. The loaded object is a resolved `Field`, so message readers do not perform filesystem access or resolve schema references per row.

```json
{
  "name": "PartyID",
  "dtype": {"type": "null"},
  "nullable": true,
  "metadata": {"fix:field": "partyid"}
}
```

A group stores a List or LargeList whose non-null item references the occurrence component; its root records the counter tag and component relationship, beside its own `fix:tag`, which is derived from the group's name and is never the counter's. The writer compacts resolved references again, keeping each canonical definition in one document. Missing targets, conflicting reference kinds, cycles, and nesting beyond 64 are located intake errors. Independent occurrence metadata overrides are refused; canonical metadata updates refresh their references atomically.

## Branch manifest

`branches.json` is an array ordered by branch name; each named branch record carries `name`, signed `branch` digest, numeric `version`, and optional `aliases`. The digest is checked against the canonical name, and each folder manifest entry must belong to a definition in the catalog.

An absent manifest is valid: branches are reconstructed from the definitions with their defaults. Rust and Python expose branch declarations and mutation directly; Node accepts branch text and preserves complete declarations through native catalog snapshots.

## Complete JSON snapshots

`FixRegistry::into_json` and `from_json` use one object with `fields`, `components`, `groups`, `messages`, and `branches` arrays. They reuse the folder store's compact references and resolver, so a snapshot retains named definitions, contextual groups, inline enums, and branch aliases; collecting ordinary scalar iteration does not preserve a catalog.

Python pickle and copy preserve this full graph. Node `intoJson` / `fromJson`, `toJSON`, and `clone` do the same; `stable_hash` / `stableHash` derives from native registry state.

## The tracked seed

The committed `config/fix` catalog contains 6,203 scalar fields in 65 shards, 747 components, 580 groups, and 181 messages: 1,573 JSON documents totaling 9,370,670 bytes. It contains 27,103 inline code records on 2,016 fields; generated names are canonical lowercase and standard display names remain metadata. Each of the 1,508 named definitions states the tag derived from its name - `groups/parties.json` is 209321 - and no two share one.

The source is the [pinned FIX Orchestra repository](https://github.com/FIXTradingCommunity/orchestrations/blob/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml), with the [documented naming rules](registry.md#group-names). This is a complete resolved catalog workload, so its load/write timings are not comparable to a scalar-only seed or a small FIX-version subset.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

    // Names are folded once, so a caller spells one however they have it and
    // the specification's own spelling stays on `display`.
    assert_eq!(registry.field_by_tag(55)?.name(), "symbol");
    assert_eq!(registry.field_by_name("SYMBOL", Some(&standard))?.name(), "symbol");
    assert_eq!(registry.field_by_tag(150)?.display(), Some("ExecType"));
    assert_eq!(registry.field_by_path("Parties.PartyID", Some(&standard))?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("ClOrdID", Some(&standard))?.display(), Some("ClOrdID"));
    // Every field is a specification field or one of the crate's own, and
    // both are standard, so none states a branch.
    let branched = registry.iter().filter(|field| field.has_metadata("fix:branch")).count();
    assert_eq!(branched, 0);
    // The whole published dictionary, not a sample of it.
    assert!(registry.len() > 6_000);
    ```

=== "Python"

    ```python
    import pathlib

    from yggdryl.fix import STANDARD_BRANCH, FixRegistry, fix_crate_fields

    # The seed this repository tracks, named from the repository root.
    seed = pathlib.Path("config/fix").resolve()
    registry = FixRegistry.from_handle(seed)

    assert registry.field_by_tag(55).name == "symbol"
    assert registry.field_by_id("55:").name == "symbol"
    assert registry.field_by_name("SYMBOL", STANDARD_BRANCH).name == "symbol"
    assert registry.field_by_tag(150).display == "ExecType"
    assert registry.field_by_path("Parties.PartyID", STANDARD_BRANCH).fix.tag == 448
    assert registry.field_by_name("ClOrdID", STANDARD_BRANCH).display == "ClOrdID"
    # Every field is a specification field or one of the crate's own, and
    # both are standard, so none states a branch.
    assert sum("fix:branch" in field.metadata for field in registry) == 0
    assert len(fix_crate_fields()) == 19
    # The whole published dictionary, not a sample of it.
    assert len(registry) > 6_000
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    // The seed this repository tracks, named from the repository root.
    const standard = fix.STANDARD_BRANCH
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))

    assert.equal(registry.fieldByTag(55).name, 'symbol')
    assert.equal(registry.fieldById('55:').name, 'symbol')
    assert.equal(registry.fieldByName('SYMBOL', standard).name, 'symbol')
    assert.equal(registry.fieldByTag(150).display, 'ExecType')
    assert.equal(registry.fieldByPath('Parties.PartyID', standard).fix.tag, 448)
    assert.equal(registry.fieldByName('ClOrdID', standard).display, 'ClOrdID')
    // Every field is a specification field or one of the crate's own, and
    // both are standard, so none states a branch.
    assert.equal([...registry].filter((field) => field.has('fix:branch')).length, 0)
    assert.equal(fix.crateFields().length, 19)
    // The whole published dictionary, not a sample of it.
    assert.ok(registry.size > 6_000)
    ```

## Edges

- A missing category is empty; reading a missing root creates nothing.
- Duplicate field identifiers or named declarations fail instead of replacing an earlier source record.
- Scalar arrays and individual named documents have distinct shapes; loading the wrong shape names the document.
- Wrong shard, branch, category datatype, counter type, or reference target is refused before a registry is returned.
- Standard fields outside the user range cannot acquire a named branch.
- Canonical definition names must form safe single path segments; separators and traversal names are refused.
- A directory under a category whose name is not a branch is refused with `FixBranch::from_str`'s parse failure, its byte position, and the directory URL; a branch directory is named by the canonical lowercase branch text.
- A `branches.json` entry holding `targetcompid` or `sendercompid` is refused naming the key: a branch is a dictionary, and the session that spoke it is a fact about a run.
- A `README` beside the field shards is ignored on read and left alone by publication; only `<n>.json` with a decimal `n` is read.
- A stored document holding one of the crate's own fields, a standard tag from 65000, is read past, and `write_into` writes none of them, so a store never holds a copy that could drift from the crate's.
- A root still holding `records/`, nested or flat, is refused naming the directory, never read as empty.
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

Release measurements on Windows, AMD Ryzen 5 150 with 12 logical CPUs, Rust 1.96, Python 3.12.13, and Node 24.18. Rust uses 10 Criterion samples; Python and Node use 2,000 boundary iterations, with expensive folder loads reduced to a smaller number of rounds.

| Folder operation | Rust estimate | Python | Node |
| --- | ---: | ---: | ---: |
| Load full seed | 3.98 s | 2.03 s | 1 op/s, rounded |
| Load 200 scalar fields | Not measured by this Rust fixture | 4.73 ms | 128 ops/s |
| Load 1 / 10 / 100 field shards | 1.15 / 7.05 / 84.7 ms | Not isolated | Not isolated |
| Write 100 field shards | 551 ms | Not isolated | Not isolated |
| Load full catalog with second branch | 3.41 s | Not isolated | Not isolated |
| Write full catalog with second branch | 12.1 s | Not isolated | Not isolated |

Different processes and sample counts make these observed boundary costs, not a language speed ranking. The full-seed load resolves referenced components and groups and compiles message/group indexes; scalar-shard fixtures measure a smaller operation.

| Native full-seed snapshot | Rust estimate |
| --- | ---: |
| `into_json` | 177 ms |
| `from_json` | 1.44 s |
| `stable_hash`, one digester state allocation | 396 ms |

| Small catalog boundary | Python | Node |
| --- | ---: | ---: |
| Snapshot encode | 29.2 us | 33,590 ops/s |
| Snapshot decode | 92.2 us | 9,421 ops/s |
| Content hash | 3.02 us | 365,490 ops/s |
| Independent copy | 2.00 us | 177,936 ops/s |

The small boundary fixture contains two fields plus one component, group, and message. It is intentionally distinct from the full-seed Rust snapshot fixture.

Root navigation is asserted with `Counted`: loading resolves four category roots plus the manifest (`child_by_path=5`); writing a one-shard catalog resolves those five paths plus its shard (`child_by_path=6`). These counts cover the root handle only; document reads/writes occur on child handles and are outside that tally.

Regenerate with release bindings installed:

```bash
cargo bench -p yggdryl --bench fix -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
python python/benchmarks/fix.py --iterations 2000
```

```powershell
$env:YGGDRYL_BENCH_ITERATIONS = '2000'
node node/benchmarks/fix.js
```
