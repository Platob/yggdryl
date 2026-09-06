# Store

A registry persists through one [`IOBase`](../holder/index.md) folder handle as two shard trees plus one branch manifest.

## Contract

| key | value |
| --- | --- |
| Owns | `FixRegistry::from_handle` and `write_into` over one folder handle |
| Layout | `<root>/primitive/<shard>.json` and `<root>/primitive/<branch>/<shard>.json`, the same two under `nested/`, and `<root>/branches.json` |
| Branch level | The standard branch writes its shards directly under a tree; a named branch adds one folder |
| Shard | `shard = tag / 100`, inside each tree; an alternate tag fans nothing |
| Tree | `field.dtype().is_nested()`, after unwrapping a dictionary and a run-end encoding; the in-memory indexes still cover both trees together |
| Shard body | JSON array of `Field::into_value`, identifier-ordered, indented; no envelope, no version marker |
| Manifest | `branches.json`, a canonical JSON array ordered by branch name, holding every named branch; absent is valid |
| Load | every shard of both trees on open; both trees optional; other leaves ignored; a missing folder loads empty |
| Authority | the field's own `fix:branch` and datatype, never the folder it sits in; a standard field states no key |
| Write | creates the root, writes populated shards whole, then removes empty shards, branch folders and trees |
| Refused | a root still holding `records/`; no migration, no backward compatibility |
| Seed | `config/fix`, tracked and written by `write_into`; outside the [default registry](registry.md)'s order |

## Use

A dictionary of only scalars writes no `nested/` folder, and one of only groups writes no `primitive/`.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixId, FixBranch, FixRegistry};

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-fix-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut folder = Folder::new(&root)?;

    let mut fields = Vec::new();
    for (tag, name) in [(35, "MsgType"), (99, "StopPx"), (100, "NoAllocs"), (150, "ExecType")] {
        let mut field = DataType::Utf8.nullable_field(name);
        field.as_fix_mut().set_tag(tag)?;
        fields.push(field);
    }
    fields[3].as_fix_mut().set_tags(&[20])?;
    // One venue field, which lands in its own branch folder.
    let cme = FixBranch::from_str("cme")?;
    let mut trade = DataType::Utf8.nullable_field("TradeID");
    trade.as_fix_mut().set_id(&cme, 5001)?;
    fields.push(trade);
    // One repeating group, which is the only field of the nested tree.
    let item = DataType::from_fields([DataType::Utf8.nullable_field("PartyID")])?
        .required_field("item");
    let mut parties = DataType::list(item).nullable_field("NoPartyIDs");
    parties.as_fix_mut().set_tag(453)?;
    fields.push(parties);
    let mut registry = FixRegistry::from_fields(fields)?;
    registry.write_into(&mut folder)?;

    let shards = |tree: &str, branch: &str| -> yggdryl::Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(root.join(tree).join(branch))?
            .filter(|entry| entry.as_ref().is_ok_and(|entry| entry.path().is_file()))
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        names.sort();
        Ok(names)
    };
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert_eq!(shards("primitive", "")?, ["0.json", "1.json"]);
    // Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert_eq!(shards("primitive", "cme")?, ["50.json"]);
    // The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert_eq!(shards("nested", "")?, ["4.json"]);

    let reloaded = FixRegistry::from_handle(&folder)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(reloaded.field_by_tag(453)?.name(), "NoPartyIDs");
    assert_eq!(reloaded.field_by_id(FixId::from_str("5001:cme")?)?.name(), "TradeID");

    // Removing the only field of a shard removes the shard on the next write,
    // emptying a branch removes its folder whole, and emptying a tree removes
    // the tree.
    registry.remove(100);
    registry.remove(150);
    registry.remove(453);
    registry.remove(FixId::from_str("5001:cme")?);
    registry.write_into(&mut folder)?;
    assert!(!root.join("primitive").join("").join("1.json").exists());
    assert!(!root.join("primitive").join("cme").exists());
    assert!(!root.join("nested").exists());
    assert_eq!(FixRegistry::from_handle(&folder)?.len(), 2);

    // A folder that is not there loads as empty and is not created.
    let absent = Folder::new(root.join("absent"))?;
    assert!(FixRegistry::from_handle(&absent)?.is_empty());
    assert!(!absent.exists());
    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pytest

    from yggdryl import DataType, Field, types as field_builders
    from yggdryl.fix import FixRegistry

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-fix-"))
    root = workspace / "dictionary"

    declared = []
    for tag, name in ((35, "MsgType"), (99, "StopPx"), (100, "NoAllocs"), (150, "ExecType")):
        field = Field(name, "utf8")
        field.fix.tag = tag
        declared.append(field)
    declared[3].fix.tags = [20]
    # One venue field, which lands in its own branch folder.
    trade = Field("TradeID", "utf8")
    trade.fix.id = "5001:cme"
    declared.append(trade)
    # One repeating group, which is the only field of the nested tree.
    item = Field("item", DataType.from_fields([Field("PartyID", "utf8")]), nullable=False)
    parties = field_builders.list("NoPartyIDs", item)
    parties.fix.tag = 453
    declared.append(parties)
    registry = FixRegistry.from_fields(declared)
    registry.write_into(root)


    def shards(tree: str, branch: str) -> list[str]:
        return sorted(path.name for path in (root / tree / branch).iterdir() if path.is_file())


    # The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert shards("primitive", "") == ["0.json", "1.json"]
    # Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert shards("primitive", "cme") == ["50.json"]
    # The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert shards("nested", "") == ["4.json"]

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_tag(20).name == "ExecType"
    assert reloaded.field_by_tag(453).name == "NoPartyIDs"
    assert reloaded.field_by_id("5001:cme").name == "TradeID"

    # Removing the only field of a shard removes the shard on the next write,
    # emptying a branch removes its folder whole, and emptying a tree removes
    # the tree. `remove` reads a str key as a standard name, so the venue
    # field leaves by rebuilding the dictionary without it.
    registry.remove(100)
    registry.remove(150)
    registry.remove(453)
    kept = FixRegistry.from_fields(
        [field for field in registry if field.fix.branch == ""]
    )
    kept.write_into(root)
    assert not (root / "primitive" / "1.json").exists()
    assert not (root / "primitive" / "cme").exists()
    assert not (root / "nested").exists()
    assert len(FixRegistry.from_handle(root)) == 2

    # A root left in the retired `records/` layout is refused, not read empty.
    retired = workspace / "retired"
    (retired / "records" / "old").mkdir(parents=True)
    with pytest.raises(ValueError, match="records"):
        FixRegistry.from_handle(retired)

    # A folder that is not there loads as empty and is not created.
    absent = root / "absent"
    assert not FixRegistry.from_handle(absent)
    assert not absent.exists()

    shutil.rmtree(workspace)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, fix } = require('yggdryl')

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-fix-'))
    const root = path.join(workspace, 'dictionary')

    const declared = []
    for (const [tag, name] of [[35, 'MsgType'], [99, 'StopPx'], [100, 'NoAllocs'], [150, 'ExecType']]) {
      const field = Field.from(`${name}: utf8`)
      field.fix.tag = tag
      declared.push(field)
    }
    declared[3].fix.tags = [20]
    // One venue field, which lands in its own branch folder.
    const trade = Field.from('TradeID: utf8')
    trade.fix.id = '5001:cme'
    declared.push(trade)
    // One repeating group, which is the only field of the nested tree.
    const item = fields.struct('item', [Field.from('PartyID: utf8')], { nullable: false })
    const parties = fields.list('NoPartyIDs', item)
    parties.fix.tag = 453
    declared.push(parties)
    const registry = fix.FixRegistry.fromFields(declared)
    registry.writeInto(root)

    const shards = (tree, branch) => fs.readdirSync(path.join(root, tree, branch))
      .filter((name) => fs.statSync(path.join(root, tree, branch, name)).isFile())
      .sort()
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert.deepEqual(shards('primitive', ''), ['0.json', '1.json'])
    // Each branch owns its own shard arithmetic: 5001 / 100 is 50.
    assert.deepEqual(shards('primitive', 'cme'), ['50.json'])
    // The group is nested, so it is the nested tree's only shard: 453 / 100.
    assert.deepEqual(shards('nested', ''), ['4.json'])

    const reloaded = fix.FixRegistry.fromHandle(root)
    assert.ok(reloaded.equals(registry))
    assert.equal(reloaded.fieldByTag(20).name, 'ExecType')
    assert.equal(reloaded.fieldByTag(453).name, 'NoPartyIDs')
    assert.equal(reloaded.fieldById('5001:cme').name, 'TradeID')

    // Removing the only field of a shard removes the shard on the next write,
    // emptying a branch removes its folder whole, and emptying a tree removes
    // the tree. A vendor field leaves by its identifier, because `remove`
    // reads a string as a standard name.
    registry.remove(100)
    registry.remove(150)
    registry.remove(453)
    registry.removeById('5001:cme')
    registry.writeInto(root)
    assert.equal(fs.existsSync(path.join(root, 'primitive', '1.json')), false)
    assert.equal(fs.existsSync(path.join(root, 'primitive', 'cme')), false)
    assert.equal(fs.existsSync(path.join(root, 'nested')), false)
    assert.equal(fix.FixRegistry.fromHandle(root).size, 2)

    // A root left in the retired `records/` layout is refused, not read empty.
    const retired = path.join(workspace, 'retired')
    fs.mkdirSync(path.join(retired, 'records', 'old'), { recursive: true })
    assert.throws(() => fix.FixRegistry.fromHandle(retired), /records/)

    // A folder that is not there loads as empty and is not created.
    const absent = path.join(root, 'absent')
    assert.equal(fix.FixRegistry.fromHandle(absent).size, 0)
    assert.equal(fs.existsSync(absent), false)

    fs.rmSync(workspace, { recursive: true, force: true })
    ```

## Trees and shards

`primitive` holds the fields whose datatype is one scalar value and `nested` the ones carrying a subtree. In FIX terms the nested fields are exactly the components, a Struct of members, and the repeating groups, a List of that Struct.

The split keeps an authored dictionary legible, not the lookup fast: one identity space and one field vector still cover both trees.

```text
<root>/primitive/<shard>.json
<root>/primitive/<branch>/<shard>.json
<root>/nested/<shard>.json
<root>/nested/<branch>/<shard>.json
<root>/branches.json
```

`shard = tag / 100` inside each tree, so `55:` is `primitive/0.json` and `5001:cme` is `primitive/cme/50.json`. A named branch segment is the canonical lowercase text, one safe path segment.

## Branch manifest

`branches.json` is a canonically rendered JSON array ordered by branch name, and the standard branch is omitted from it. Each named `FixBranch` stores `name`, derived `digest`, `version`, `targetcompid` and `sendercompid`.

| rule | behaviour |
| --- | --- |
| absent value in an entry | defaulted to the branch defaults |
| declared `digest` | verified against the derived one |
| absent manifest | valid; default branch values are reconstructed from the shards |
| entry with no field in either tree | typed error, never an invented dictionary |

A registry answers the stored branch values through five calls.

| call | answers |
| --- | --- |
| `branch_of(FixId)` | the borrowed `FixBranch` that identifier names |
| `branch_named(&str)` | the borrowed branch of that name |
| `branch_for_session(sender, target)` | the borrowed branch of that component pair, matched with ASCII case folded |
| `branches()` | every stored branch |
| `set_branch(FixBranch)` | installs one atomically |

Component IDs keep the spelling they were written with.

## The tracked seed

`config/fix` holds a small FIX 4.4 subset in `config/fix/primitive/<shard>.json` and `config/fix/nested/4.json`: the header and trailer, the order and execution fields, `Parties` as a repeating group.
Each field carries the specification's wording as its description and a display name where FIX has one.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

    assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
    assert_eq!(registry.field_by_name("ticker", Some(&standard))?.name(), "Symbol");
    assert_eq!(registry.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(registry.field_by_path("NoPartyIDs.PartyID", Some(&standard))?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("ClOrdID", Some(&standard))?.display(), Some("Client order ID"));
    // Every seed field is a specification field, so none states a branch.
    assert!(registry.iter().all(|field| !field.has_metadata("fix:branch")));
    assert!(registry.len() < 40);
    ```

=== "Python"

    ```python
    import pathlib

    from yggdryl.fix import STANDARD_BRANCH, FixRegistry

    # The seed this repository tracks, named from the repository root.
    seed = pathlib.Path("config/fix").resolve()
    registry = FixRegistry.from_handle(seed)

    assert registry.field_by_tag(55).name == "Symbol"
    assert registry.field_by_id("55:").name == "Symbol"
    assert registry.field_by_name("ticker", STANDARD_BRANCH).name == "Symbol"
    assert registry.field_by_tag(20).name == "ExecType"
    assert registry.field_by_path("NoPartyIDs.PartyID", STANDARD_BRANCH).fix.tag == 448
    assert registry.field_by_name("ClOrdID", STANDARD_BRANCH).display == "Client order ID"
    # Every seed field is a specification field, so none states a branch.
    assert all("fix:branch" not in field.metadata for field in registry)
    assert len(registry) < 40
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    // The seed this repository tracks, named from the repository root.
    const standard = fix.STANDARD_BRANCH
    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))

    assert.equal(registry.fieldByTag(55).name, 'Symbol')
    assert.equal(registry.fieldById('55:').name, 'Symbol')
    assert.equal(registry.fieldByName('ticker', standard).name, 'Symbol')
    assert.equal(registry.fieldByTag(20).name, 'ExecType')
    assert.equal(registry.fieldByPath('NoPartyIDs.PartyID', standard).fix.tag, 448)
    assert.equal(registry.fieldByName('ClOrdID', standard).display, 'Client order ID')
    // Every seed field is a specification field, so none states a branch.
    assert.ok([...registry].every((field) => field.has('fix:branch') === false))
    assert.ok(registry.size < 40)
    ```

## Edges

- `primitive/0.json`, a leaf directly under a tree -> a standard-branch shard, read like any other; other leaves are ignored.
- A folder under a tree whose name is not a branch -> `FixBranch::from_str`'s parse failure, its byte position and the folder URL.
- `branches.json` absent -> valid; every branch value is reconstructed from the shards with the branch defaults.
- A `branches.json` entry no field in either tree claims -> typed error, never an invented dictionary.
- A branch folder's name -> the canonical lowercase branch text: one path segment, no separators, no `.` or `..`.
- A `README` beside the shards -> ignored on read, left alone by `write_into`'s cleanup; only `<n>.json` with a decimal `n` is read.
- A field in the wrong shard, in a folder its `fix:branch` contradicts, or in the tree its datatype contradicts -> refused with both sides named.
- A dictionary-encoded or run-end-encoded field -> placed by its unwrapped value type, so a dictionary of Struct is nested and one of Utf8 is not.
- A shard that does not parse, holds a tagless field, duplicates another shard's tag, or holds a field the registry refuses -> typed error naming the shard's URL.
- A folder that does not exist -> the empty registry, and the folder is not created.
- A root still holding `records/`, nested or flat -> refused naming the folder, never read as empty.
- The last field of a shard removed -> that shard, its branch folder and its tree disappear on the next `write_into`.
- A field whose datatype moves it between trees -> the old tree's copy is removed by the next `write_into`, never resurrected on reload.
- `config/fix` in the Python and JavaScript seed examples -> resolved against the working directory, so run them from the repository root.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test fix store
    cargo test -p yggdryl --lib fix::tests::shard_arithmetic
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "storage_location or retired_layout or written_trees or own_folder"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="storage location|retired layout|two trees|own folder" node/tests/fix/fix.test.js
    ```
