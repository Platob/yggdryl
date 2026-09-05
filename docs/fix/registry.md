# Registry

`FixRegistry` resolves a field in tiers inside one branch, refuses partial writes, iterates branch-major, and is the default a [message](message.md) links.

## Contract

| | |
| --- | --- |
| Owns | `FixRegistry`, `FixKey`, four position indexes over one vector, each split into a primitive half and a nested half, `global()` / `install_global` |
| Tiers | canonical identifier, alternate identifier, canonical name folded, alias folded; a later tier only when every earlier one missed |
| Halves | `field.dtype().is_nested()` picks the half; each tier reads primitive before nested; a partition of each index, never a fifth tier |
| Branch | No lookup crosses a branch; a bare tag or bare name is the standard branch, never whichever dictionaries are loaded |
| String key | A colon-bearing string is a name, never an identifier; `From<&str>` cannot fail, so an identifier is parsed with `FixId::from_str` |
| Folding | ASCII case, once at insert; a probe hashes the query folded beside an inline branch and allocates nothing on a hit |
| Identity | The `FixId`, and separately the branch plus folded canonical name; two fields share neither, nor an alternate identifier, nor an alias |
| Conflict | The same key twice in one tier of one branch -> typed conflict naming both fields and the branch; overlap across tiers or branches is legal |
| Order | `iter` and `next_field_after` walk ascending identifiers, branch-major then by tag, both halves merged |
| Default | `global()` resolves once, on the first call, reading the environment once; every later call answers the same `Arc` |
| Bindings | Python `yggdryl.fix.FixRegistry`, `global_registry`, `install_global_registry`; JavaScript `fix.FixRegistry`, `fix.globalRegistry`, `fix.installGlobalRegistry` |

## Use

A name or alias in any ASCII case answers the canonical field, and a bare tag stays in the standard branch.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixId, FixKey, FixBranch, FixRegistry};

    let standard = FixBranch::STANDARD;
    let cme = FixBranch::from_str("cme")?;

    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut price = DataType::decimal128(20, 8)?.nullable_field("Price");
    price.as_fix_mut().set_tag(44)?;
    price.as_fix_mut().set_aliases(["Px"])?;
    // The venue dictionary reuses the name `Symbol`, which is the normal case.
    let mut venue = DataType::Utf8.nullable_field("Symbol");
    venue.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5055)?)?;
    let mut registry = FixRegistry::from_fields([symbol, price, venue])?;

    // Any spelling of a name or alias answers the canonical field.
    assert_eq!(registry.field_by_name(&standard, "TICKER")?.name(), "Symbol");
    assert_eq!(registry.field("px")?.name(), "Price");
    assert_eq!(registry.get_field(55), registry.get_field("symbol"));
    assert!(registry.contains(FixKey::Tag(44)));
    assert!(!registry.contains("44"), "a tag query never consults names");
    let error = registry.field_by_tag(35).unwrap_err();
    assert!(error.is_absent());

    // A bare tag and a bare name are the standard branch; the venue field
    // is reached by its identifier or by name inside its own dictionary.
    let venue_id = FixId::from_str("cme:5055")?;
    assert_eq!(registry.field_by_id(&venue_id)?.as_fix().branch()?, cme);
    assert_eq!(registry.field(&venue_id)?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&cme, "SYMBOL")?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name(&standard, "symbol")?.as_fix().tag()?, Some(55));
    assert!(registry.get_field_by_tag(5055).is_none(), "never crosses a branch");
    assert!(registry.get_field("cme:5055").is_none(), "a string key is a name");

    // A key another field holds *in the same branch* is a conflict naming
    // both, and the branch; nothing changes.
    let mut clash = DataType::Utf8.nullable_field("SymbolSfx");
    clash.as_fix_mut().set_tag(65)?;
    clash.as_fix_mut().set_aliases(["ticker"])?;
    let error = registry.insert(clash).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("in branch"), "{error}");
    assert!(error.to_string().contains("held by Symbol"), "{error}");
    assert_eq!(registry.len(), 3);

    // A merge keeps what only the stored field declared and adds the rest.
    let mut incoming = DataType::Utf8.nullable_field("SYMBOL");
    incoming.as_fix_mut().set_tag(55)?;
    incoming.as_fix_mut().set_tags(&[65])?;
    incoming.as_fix_mut().set_aliases(["Sym"])?;
    registry.update(incoming)?;
    let merged = registry.field_by_tag(65)?;
    assert_eq!(merged.name(), "SYMBOL");
    assert_eq!(merged.as_fix().aliases().collect::<Vec<_>>(), ["Sym", "Ticker"]);
    // A datatype disagreement is refused, never widened.
    let mut widened = DataType::LargeUtf8.nullable_field("Symbol");
    widened.as_fix_mut().set_tag(55)?;
    assert!(registry.update(widened).is_err());
    assert_eq!(registry.field_by_tag(55)?.dtype(), &DataType::Utf8);

    // Iteration is branch-major, then by tag.
    assert_eq!(
        registry.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["Symbol", "Price", "SYMBOL"],
    );
    assert_eq!(registry.remove("sym").map(|field| field.name().to_owned()), Some("SYMBOL".into()));
    assert!(registry.get_field_by_tag(65).is_none());
    assert_eq!(registry.remove(&venue_id).map(|field| field.name().to_owned()), Some("Symbol".into()));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field
    from yggdryl.fix import STANDARD_BRANCH, FixRegistry


    def fix_field(name: str, dtype: str, identifier: str, *aliases: str) -> Field:
        field = Field(name, dtype)
        field.fix.id = identifier
        if aliases:
            field.fix.aliases = aliases
        return field


    registry = FixRegistry.from_fields(
        [
            fix_field("Symbol", "utf8", "standard:55", "Ticker"),
            fix_field("Price", "decimal128(20, 8)", "standard:44", "Px"),
            # The venue dictionary reuses the name `Symbol`, the normal case.
            fix_field("Symbol", "utf8", "cme:5055"),
        ]
    )

    # Any spelling of a name or alias answers the canonical field.
    assert registry.field_by_name(STANDARD_BRANCH, "TICKER").name == "Symbol"
    assert registry.field("px").name == "Price"
    assert registry.get_field(55) == registry.get_field("symbol")
    assert 44 in registry
    assert "44" not in registry, "a tag query never consults names"
    with pytest.raises(KeyError, match="tag 35"):
        registry.field_by_tag(35)

    # A bare tag and a bare name are the standard branch; the venue field is
    # reached by its identifier or by name inside its own dictionary.
    assert registry.field_by_id("cme:5055").fix.branch == "cme"
    assert registry.field_by_name("cme", "SYMBOL").fix.tag == 5055
    assert registry.field_by_name("standard", "symbol").fix.tag == 55
    assert registry.get_field_by_tag(5055) is None, "never crosses a branch"
    assert registry.get_field("cme:5055") is None, "a string key is a name"

    # A key another field holds *in the same branch* is a conflict naming
    # both, and the branch; nothing changes.
    with pytest.raises(ValueError, match="held by Symbol") as conflict:
        registry.insert(fix_field("SymbolSfx", "utf8", "standard:65", "ticker"))
    assert 'branch \\"standard\\"' in str(conflict.value)
    assert len(registry) == 3

    # A merge keeps what only the stored field declared and adds the rest.
    incoming = fix_field("SYMBOL", "utf8", "standard:55", "Sym")
    incoming.fix.tags = [65]
    registry.update(incoming)
    merged = registry.field_by_tag(65)
    assert merged.name == "SYMBOL"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(fix_field("Symbol", "large_utf8", "standard:55"))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    # Iteration is branch-major, then by tag.
    assert [field.fix.id for field in registry] == [
        "cme:5055",
        "standard:44",
        "standard:55",
    ]
    assert registry.remove("sym").name == "SYMBOL"
    assert registry.get_field_by_tag(65) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fix } = require('yggdryl')

    function fixField(name, dtype, identifier, ...aliases) {
      const field = Field.from(`${name}: ${dtype}`)
      field.fix.id = identifier
      if (aliases.length !== 0) field.fix.aliases = aliases
      return field
    }

    const registry = fix.FixRegistry.fromFields([
      fixField('Symbol', 'utf8', 'standard:55', 'Ticker'),
      fixField('Price', 'decimal128(20, 8)', 'standard:44', 'Px'),
      // The venue dictionary reuses the name `Symbol`, the normal case.
      fixField('Symbol', 'utf8', 'cme:5055'),
    ])

    // Any spelling of a name or alias answers the canonical field.
    assert.equal(registry.fieldByName(fix.STANDARD_BRANCH, 'TICKER').name, 'Symbol')
    assert.equal(registry.field('px').name, 'Price')
    assert.ok(registry.getField(55).equals(registry.getField('symbol')))
    assert.equal(registry.has(44), true)
    assert.equal(registry.has('44'), false, 'a tag query never consults names')
    assert.throws(() => registry.fieldByTag(35), /tag 35/)

    // A bare tag and a bare name are the standard branch; the venue field is
    // reached by its identifier or by name inside its own dictionary.
    assert.equal(registry.fieldById('cme:5055').fix.branch, 'cme')
    assert.equal(registry.fieldByName('cme', 'SYMBOL').fix.tag, 5055)
    assert.equal(registry.fieldByName('standard', 'symbol').fix.tag, 55)
    assert.equal(registry.getFieldByTag(5055), null, 'never crosses a branch')
    assert.equal(registry.getField('cme:5055'), null, 'a string key is a name')

    // A key another field holds *in the same branch* is a conflict naming
    // both, and the branch; nothing changes.
    assert.throws(
      () => registry.insert(fixField('SymbolSfx', 'utf8', 'standard:65', 'ticker')),
      /held by Symbol/,
    )
    assert.equal(registry.size, 3)

    // A merge keeps what only the stored field declared and adds the rest.
    const incoming = fixField('SYMBOL', 'utf8', 'standard:55', 'Sym')
    incoming.fix.tags = [65]
    registry.update(incoming)
    const merged = registry.fieldByTag(65)
    assert.equal(merged.name, 'SYMBOL')
    assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
    // A datatype disagreement is refused, never widened.
    assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', 'standard:55')))
    assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

    // Iteration is branch-major, then by tag.
    assert.deepEqual(
      [...registry].map((field) => field.fix.id),
      ['cme:5055', 'standard:44', 'standard:55'],
    )
    assert.equal(registry.remove('sym').name, 'SYMBOL')
    assert.equal(registry.getFieldByTag(65), null)
    // `remove` reads a string as a standard name, so a vendor field leaves by
    // its identifier.
    assert.equal(registry.removeById('cme:5055').name, 'Symbol')
    assert.equal(registry.size, 1)
    ```

## Tiers

Inside one branch each tier reads its primitive half before its nested half, and a later tier runs only after every earlier one missed.

| tier | primitive half, then nested half |
| ---: | --- |
| 1 | canonical identifier |
| 2 | alternate identifier |
| 3 | canonical name, folded |
| 4 | alias, folded |

The same `field.dtype().is_nested()` that picks a field's [store tree](store.md) picks its half. A canonical key in the nested half still beats an alternate key in the primitive one.

## Accessors

Every lookup has an optional form and a failing twin. The twin raises a typed absence naming the key (`tag 35`, `identifier cme:5001`, `name "MsgType"`, `path "a.b"`).

| optional | failing | key |
| --- | --- | --- |
| `get_field_by_id(&FixId)` | `field_by_id` | canonical or alternate identifier, in any branch; carries the implementation |
| `get_field_by_tag(i32)` | `field_by_tag` | canonical or alternate tag in the standard branch, which is `get_field_by_id(&FixId::standard(tag))` |
| `get_field_by_name(&FixBranch, &str)` | `field_by_name` | canonical name or alias, folded, inside one branch |
| `get_field_by_path(&FixBranch, &str)` | `field_by_path` | the whole string as a name first, else the first segment here and the rest through `Field::get_field_by_path` |
| `get_field(impl Into<FixKey>)` | `field` | matches `FixKey::Tag` / `FixKey::Id` / `FixKey::Name` once and redirects to the rows above, a bare key meaning the standard branch |

`FixKey` is built from an `i32`, a `&FixId`, a `&str` or a `&String`, exactly as `FieldKey` is, so `registry.field(35)` and `registry.field("MsgType")` are one call.

| call | answers |
| --- | --- |
| `contains(impl Into<FixKey>)` | whether the key resolves, through the same tiers |
| `iter` | every field in ascending identifier order, branch-major then by tag, the two halves merged as it goes |
| `next_field_after` | the cursor a binding advances with; the same merge as `iter` |
| `len` / `is_empty` | both halves counted |

## Insert, update and remove

Both mutations build the result first and check every key it would claim, so a refusal writes nothing.

| call | result |
| --- | --- |
| `insert`, fresh field | `Ok(None)` |
| `insert`, both identity halves match one stored field | `Ok(Some(prior))`, a wholesale replacement |
| `insert`, a key another field holds in the same branch | typed conflict naming both fields and the key; never a silent replacement |
| `update`, same identifier | merge: the incoming field wins the name spelling, nullability and every metadata key both declare; the stored field keeps keys only it declares; `tags` and `aliases` concatenate, incoming first, deduplicated, order kept |
| `update`, branch disagrees | absence, because the branch is half of the identity |
| `update`, datatype disagrees | typed error naming both, never a silent widening |
| `remove` | takes a tag, an identifier or a name, never a path, and answers the field |

## One default registry per process

`FixRegistry::global()` resolves on the first call, on the calling thread, with nothing loaded at module init and no thread spawned. First match wins:

| step | source | when absent |
| ---: | --- | --- |
| 1 | the registry passed to `FixRegistry::install_global` | next step |
| 2 | the folder `YGGDRYL_FIX_REGISTRY` names, a URL or a bare path, through the [local backend](../holder/backends/local.md) | error: a set variable must name an existing directory |
| 3 | `~/.config/fix` through `Folder::config`; skipped with no `HOME` or `USERPROFILE` | next step: a machine with no dictionary is an ordinary first run |
| 4 | the empty registry | |

A malformed shard or a scheme without a backend is an error from `global()`, never the empty registry, and the next call retries. The repository's own `config/fix` is not in the order: nothing walks up from the working directory.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::local::Folder;
    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};

    // Install the tracked seed as this process's default before anything asks for it.
    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;
    FixRegistry::install_global(registry)?;

    let global = FixRegistry::global()?;
    assert_eq!(global.field_by_tag(55)?.name(), "Symbol");
    assert!(Arc::ptr_eq(FixRegistry::global()?, global), "resolved once");

    // A message built without a registry links that same `Arc`.
    let root = DataType::from_fields([global.field_by_tag(55)?.clone()])?.required_field("row");
    let msg = FixMsg::new(root, Scalar::from_record([("Symbol", Scalar::from("AAPL"))])?)?;
    assert!(Arc::ptr_eq(msg.registry(), global));

    // Once resolved, the default is fixed.
    assert!(FixRegistry::install_global(FixRegistry::new()).unwrap_err().is_conflict());
    ```

=== "Python"

    ```python
    import pathlib

    import pytest

    from yggdryl import DataType, Field
    from yggdryl.fix import FixMsg, FixRegistry, global_registry, install_global_registry

    # Install the tracked seed as this process's default before anything asks for it.
    seed = FixRegistry.from_handle(pathlib.Path("config/fix").resolve())
    install_global_registry(seed)

    default = global_registry()
    assert default.field_by_tag(55).name == "Symbol"
    assert default == global_registry(), "resolved once"

    # A message built without a registry links that same dictionary.
    root = Field("row", DataType.from_fields([default.field_by_tag(55)]), nullable=False)
    assert FixMsg(root, {"Symbol": "AAPL"}).registry == default

    # Once resolved, the default is fixed.
    with pytest.raises(ValueError, match="already resolved"):
        install_global_registry(FixRegistry())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fields, fix } = require('yggdryl')

    // Install the tracked seed as this process's default before anything asks for it.
    const seed = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    fix.installGlobalRegistry(seed)

    const global = fix.globalRegistry()
    assert.equal(global.fieldByTag(55).name, 'Symbol')
    assert.ok(global.equals(fix.globalRegistry()), 'resolved once')

    // A message built without a registry links that same dictionary.
    const root = fields.struct('row', [global.fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { Symbol: 'AAPL' }).registry.equals(global))

    // Once resolved, the default is fixed.
    assert.throws(() => fix.installGlobalRegistry(new fix.FixRegistry()), /already resolved/)
    ```

## Edges

- `registry.get_field("cme:5055")` -> `None`; a string key is a name, and only `FixId::from_str` parses an identifier.
- `registry.get_field_by_tag(5055)` for a vendor tag -> `None`; a bare tag never crosses the standard branch.
- `contains("44")` -> `false`; a tag query never consults names, and a name query never consults tags.
- A path -> the whole string as a name first, keeping a dotted name reachable; then the first segment here, the rest through `Field::get_field_by_path` exactly.
- An alternate tag equal to another field's canonical tag, or an alias equal to another's canonical name -> legal, and it never wins.
- The same key twice in the same tier of one branch -> conflict.
- `insert` of a field whose key another field holds in the same branch -> conflict naming both fields and the branch; `len` unchanged.
- `insert` of a field matching one stored field on both identity halves -> `Ok(Some(prior))`, a replacement, never a conflict.
- `update` with a different datatype -> typed error; the stored datatype stays.
- `remove` with a path -> never a match; it takes a tag, an identifier or a name, and a bare one means the standard branch.
- A nested field claiming a primitive's tag, name, alternate tag or alias -> the same conflict as between two primitives, checked in both halves first.
- `install_global` after `global()` has resolved -> typed conflict (`already resolved` in the bindings); the value every caller saw cannot change.
- `YGGDRYL_FIX_REGISTRY` set to a missing directory -> error from `global()`, where an absent `~/.config/fix` is the empty registry.
- A tag below `FixId::STANDARD_TAG_LIMIT` in a vendor branch -> refused ([FIX](index.md)); at or above it a vendor field needs its `FixId` or a branch-qualified name.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::a_field_without_a_tag fix::tests::a_name_or_alias fix::tests::tier_order fix::tests::a_tag_query fix::tests::an_insert_conflict fix::tests::reinserting fix::tests::a_merge_follows fix::tests::a_rejected_merge fix::tests::removal_keeps fix::tests::specialized_and_generic fix::tests::iteration_follows fix::tests::iteration_and_the_cursor fix::tests::nestedness_routes fix::tests::the_primitive_half fix::tests::a_nested_field_can_never fix::tests::two_branches_may_hold fix::tests::the_default_resolves
    cargo test -p yggdryl --test fix global
    cargo bench -p yggdryl --bench fix -- fix/resolve
    cargo bench -p yggdryl --bench fix -- fix/mutate
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "registry_resolves or no_lookup or registry_absence or registry_keys or registry_coerces or registry_iterates or seed_iterates or registry_insert or registry_mutation or install_global"
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="resolves every key|never crosses a branch|removeById|absence throws|number tag or a string name|coerced at the boundary|iterates lazily|seed iterates|insert, update and remove|shared registry|registry is a value|fix namespace is frozen|installing the process default" node/tests/fix/fix.test.js
    npm run --prefix node bench:fix
    ```

## Performance

### Native

One local Windows x86_64 release run of the Criterion target, point estimates, over the tracked seed of 34 fields unless a row says otherwise.

| resolution | estimate |
| --- | ---: |
| `get_field_by_tag` primitive hit | 32.3 ns |
| `get_field_by_tag` nested hit (`NoPartyIDs`, tag 453) | 93.1 ns |
| `get_field_by_tag` alternate-tag hit | 65.8 ns |
| `get_field_by_tag` miss | 72.2 ns |
| `get_field_by_id` vendor hit, over 1034 fields in two branches | 128.1 ns |
| `get_field_by_name` hit | 81.8 ns |
| `get_field_by_name` hit, query differently cased | 81.2 ns |
| `get_field_by_name` alias hit | 191.5 ns |
| `get_field_by_name` miss | 175.4 ns |
| `get_field_by_name` vendor-branch hit, over 1034 fields | 89.7 ns |
| `get_field_by_name` vendor-branch alias hit, over 1034 fields | 175.2 ns |
| `get_field_by_tag` cross-branch miss, over 1034 fields | 222.1 ns |
| `get_field_by_tag` standard hit, over 1034 fields in two branches | 137.8 ns |
| `get_field(FixKey::Tag)` generic tag hit | 32.4 ns |
| `get_field("Symbol")` generic name hit | 89.2 ns |
| `field(55)` failing-half tag hit | 31.4 ns |
| `get_field_by_path`, one segment | 142.6 ns |
| `get_field_by_path`, two segments (`NoPartyIDs.PartyID`) | 389.4 ns |
| `get_field_by_path`, three segments (`NoPartyIDs.item.PartyRole`) | 389.2 ns |
| baseline `HashMap<FixId, Field>` hit | 39.4 ns |
| baseline `HashMap<i32, Field>` tag hit | 19.3 ns |
| baseline `HashMap<String, Field>` hit after lowercasing the query | 97.0 ns |
| tag hit over 4034 fields, all primitive | 179.5 ns |
| name hit over 4034 fields, all primitive | 106.4 ns |
| alias hit over 4034 fields, all primitive | 188.0 ns |
| primitive tag hit over 4034 fields, one in fifty nested | 201.3 ns |
| nested tag hit over 4034 fields, one in fifty nested | 268.2 ns |

Mutation clones the dictionary in the batch setup and returns it as the routine's output, so neither clone nor drop is inside the timer.

| mutation | estimate |
| --- | ---: |
| `insert` into the seed | 5.13 us |
| `insert` into 4034 fields | 115 us |
| `from_fields` over 4034 fields | 14.1 ms |
| `update` merging an alias and an alternate tag into the seed | 9.93 us |
| the same `update` over 4034 fields | 17.9 us |
| `remove` from 4034 fields | 10.6 us |

- The generic accessor is the specialized one plus its dispatch, within the noise on a tag: 32.4 ns against 32.3 ns.
- The specialized pair exists so a caller that already knows its key pays no dispatch.
- A folded name hit costs what `HashMap<String, Field>` costs *before* lowercasing its query: the fold happens inside the hash, so no folded copy is built.
- The `FixId` key moved the tag hit from 4.5 ns to 32.3 ns, while the `HashMap<i32, Field>` baseline went from 18.1 ns to 19.3 ns.
- Every index level compares an inline branch string before an `i32`, and a 32-byte key makes a node span six cache lines, not one.
- Against `HashMap<FixId, Field>` at 39.4 ns the ordered index is faster, and it alone answers `next_field_after`, `iter` and branch-major grouping.
- `HashMap<i32, Field>` is faster only because it cannot hold two branches; it answers the ambiguous question the identity carries.

The split is for locality, so the hot half a transcriber probes per wire tag holds only scalars. On this dictionary shape the numbers show a small loss, not a win.

| probe over the seed | single index | split index |
| --- | ---: | ---: |
| primitive tag hit | 27.4 ns | 32.3 ns |
| tag miss | 48.4 ns | 72.2 ns |
| alias hit | 136.5 ns | 191.5 ns |
| one-segment path | 86.5 ns | 142.6 ns |

- The seed's nested half holds one field of 34, so the primitive map is one entry smaller than the undivided one was.
- One entry changes no B-tree depth, and the extra structure costs more than it saves.
- Over 4034 fields, one in fifty nested, the primitive tag hit is 201.3 ns against 179.5 ns all primitive: 2% fewer entries buys nothing measurable.
- Every probe that misses its first map now reads two maps per tier.
- A nested hit pays that too: 93.1 ns over the seed against 32.3 ns for a primitive one.
- The split earns its place on the [layout](store.md): the nested definitions are a contiguous half, read, written and skipped as a unit.
- A minority nested share buys no faster primitive hit, and this page claims none; a large nested half is the unmeasured case that would pay.

```bash
cargo bench -p yggdryl --bench fix -- --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```

### Python boundary

One local Windows x86_64 run of the release wheel (`maturin build --release`) under CPython 3.12, median time per call. Rows run over the 34-field seed, vendor rows beside a generated `cme` dictionary of 1000 fields; sub-microsecond rows move by a third between runs.

| Python operation | estimate |
| --- | ---: |
| `get_field_by_tag(55)` hit | 196 ns |
| `get_field_by_tag(20)` alternate-tag hit | 235 ns |
| `get_field_by_id("standard:55")` hit | 408 ns |
| `get_field_by_name("standard", "Symbol")` hit | 303 ns |
| `get_field_by_name("standard", "symbol")` hit, folded query | 298 ns |
| `get_field_by_name("standard", "ticker")` alias hit | 404 ns |
| `get_field_by_tag(9999)` miss | 186 ns |
| `get_field_by_name("standard", "Nope")` miss | 264 ns |
| `get_field_by_id("cme:5001")` miss | 308 ns |
| `get_field(55)` generic tag hit | 205 ns |
| `field_by_path`, one segment | 357 ns |
| `field_by_path`, two segments (`NoPartyIDs.PartyID`) | 568 ns |
| `get_field_by_id("cme:5001")` vendor hit, two branches | 519 ns |
| `get_field_by_name("cme", ...)` vendor hit, two branches | 304 ns |
| `get_field_by_name("cme", ...)` vendor alias hit, two branches | 493 ns |
| `get_field_by_tag(5001)` cross-branch miss, two branches | 288 ns |
| `get_field_by_tag(55)` standard hit, two branches | 304 ns |

- A hit is the native lookup plus one crossing: the key is read once and the answer is wrapped as a `Field`, cloned, not borrowed.
- That wrapping is most of the number: the 32.3 ns native tag hit sits an order of magnitude below, so the Rust tiers are indistinguishable.
- A miss wraps nothing and is the one reliably cheaper case; a caller resolving the same field repeatedly should hold the answer.

```bash
python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
```

### JavaScript boundary

One local Windows x86_64 run (AMD Ryzen 5 150) of the release addon (`npm run --prefix node build`) under Node.js v24.18.0, whole-loop rate. Rows run over the 34-field seed, vendor rows beside a generated `cme` dictionary of 1000 fields; sub-microsecond rows move by a third between runs.

| JavaScript operation | rate | per call |
| --- | ---: | ---: |
| `getFieldByTag(55)` hit | 320k/s | 3.12 us |
| `getFieldByTag(20)` alternate-tag hit | 308k/s | 3.25 us |
| `getFieldById('standard:55')` hit | 268k/s | 3.73 us |
| `getFieldByName('standard', 'Symbol')` hit | 287k/s | 3.48 us |
| `getFieldByName('standard', 'symbol')` hit, folded query | 265k/s | 3.78 us |
| `getFieldByName('standard', 'ticker')` alias hit | 242k/s | 4.14 us |
| `getFieldByTag(9999)` miss | 1.23M/s | 815 ns |
| `getFieldByName('standard', 'Nope')` miss | 884k/s | 1.13 us |
| `getFieldById('cme:5001')` miss | 929k/s | 1.08 us |
| `getField(55)` generic tag hit | 319k/s | 3.14 us |
| `getField('Symbol')` generic name hit | 283k/s | 3.53 us |
| `fieldByPath`, one segment | 273k/s | 3.66 us |
| `fieldByPath`, two segments (`NoPartyIDs.PartyID`) | 223k/s | 4.48 us |
| `getFieldById('cme:5001')` vendor hit, two branches | 219k/s | 4.56 us |
| `getFieldByName('cme', ...)` vendor hit, two branches | 269k/s | 3.72 us |
| `getFieldByName('cme', ...)` vendor alias hit, two branches | 264k/s | 3.79 us |
| `getFieldByTag(5001)` cross-branch miss, two branches | 1.13M/s | 886 ns |
| `getFieldByTag(55)` standard hit, two branches | 302k/s | 3.31 us |
| `removeById('cme:9999')` miss, two branches | 848k/s | 1.18 us |

- A miss is the crossing itself: 815 ns for key coercion, the native probe and `null` back, beside 641 ns for a bare `registry.size`.
- A hit adds the wrapper: `field.clone()` on an already-held native `Field` costs 2.99 us, so a 3.12 us tag hit is nearly one `Field` materialization.
- The native tag hit is 32.3 ns, two orders of magnitude below the crossing, so the Rust tiers are indistinguishable; hold the answer.
- `removeById`'s miss is 1.18 us: the same identifier parse as `getFieldById`'s 1.08 us miss, plus the mutation's uniqueness check.

```bash
npm run --prefix node bench:fix
```
