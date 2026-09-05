# Registry

`FixRegistry` resolves a field in tiers, refuses partial writes, iterates tag-major, and is the default a [message](message.md) links.

## Contract

| | |
| --- | --- |
| Owns | `FixRegistry`, `FixKey`, one field vector and four hash indexes of positions over it, `global()` / `install_global` |
| Index keys | Canonical and alternate identities use `FixId` directly; canonical names and aliases use independent seeded XXH64 digests over the branch digest and the folded name |
| Collision | A read rechecks the field behind every name digest, so a collision is a miss; a mutation refuses it loudly |
| Tiers | canonical identifier, alternate identifier, canonical name folded, alias folded; a later tier only when every earlier one missed |
| Branch | An explicit branch never crosses into another dictionary; with no branch, one deterministic best-match order decides |
| Tag range | Outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` no named branch may hold a tag |
| String key | A colon-bearing string is a name, never an identifier; `From<&str>` cannot fail, so an identifier is parsed with `FixId::from_str` |
| Folding | ASCII case, once at insert; a probe hashes the query folded beside an inline branch and allocates nothing on a hit |
| Identity | The `FixId`, and separately the branch plus folded canonical name; two fields share neither, nor an alternate identifier, nor an alias |
| Conflict | The same key twice in one tier of one branch -> typed conflict naming both fields and the branch; overlap across tiers or branches is legal |
| Order | `iter` and `next_field_after` walk ascending packed identifiers, tag-major then by branch digest |
| Inference | `infer_bytes_protocol` / `infer_text_protocol` and `infer_bytes_msgtype` / `infer_text_msgtype` classify one line without parsing a message |
| Default | `global()` resolves once, on the first call, reading the environment once; every later call answers the same `Arc` |
| Bindings | Python `yggdryl.fix.FixRegistry`, `global_registry`, `install_global_registry`; JavaScript `fix.FixRegistry`, `fix.globalRegistry`, `fix.installGlobalRegistry` |

## Use

A name or alias in any ASCII case answers the canonical field, and a tag with no branch takes the best match.

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
    venue.as_fix_mut().set_id(&cme, 5055)?;
    let mut registry = FixRegistry::from_fields([symbol, price, venue])?;

    // Any spelling of a name or alias answers the canonical field.
    assert_eq!(registry.field_by_name("TICKER", Some(&standard))?.name(), "Symbol");
    assert_eq!(registry.field("px")?.name(), "Price");
    assert_eq!(registry.get_field(55), registry.get_field("symbol"));
    assert!(registry.contains(FixKey::Tag(44)));
    assert!(!registry.contains("44"), "a tag query never consults names");
    let error = registry.field_by_tag(35).unwrap_err();
    assert!(error.is_absent());

    // Explicit identity/name pin a branch. Omitted tag lookup infers the only
    // matching venue definition, while the standard canonical name wins.
    let venue_id = FixId::from_str("5055:cme")?;
    assert_eq!(registry.field_by_id(venue_id)?.as_fix().branch()?, cme);
    assert_eq!(registry.field(venue_id)?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name("SYMBOL", Some(&cme))?.as_fix().tag()?, Some(5055));
    assert_eq!(registry.field_by_name("symbol", Some(&standard))?.as_fix().tag()?, Some(55));
    assert_eq!(registry.field_by_tag(5055)?.as_fix().branch()?, cme);
    assert!(registry.get_field("5055:cme").is_none(), "a string key is a name");

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

    // Iteration is tag-major, then by branch digest.
    assert_eq!(
        registry.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["Price", "SYMBOL", "Symbol"],
    );
    assert_eq!(registry.remove("sym").map(|field| field.name().to_owned()), Some("SYMBOL".into()));
    assert!(registry.get_field_by_tag(65).is_none());
    assert_eq!(registry.remove(venue_id).map(|field| field.name().to_owned()), Some("Symbol".into()));
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
            fix_field("Symbol", "utf8", "55:", "Ticker"),
            fix_field("Price", "decimal128(20, 8)", "44:", "Px"),
            # The venue dictionary reuses the name `Symbol`, the normal case.
            fix_field("Symbol", "utf8", "5055:cme"),
        ]
    )

    # Any spelling of a name or alias answers the canonical field.
    assert registry.field_by_name("TICKER", STANDARD_BRANCH).name == "Symbol"
    assert registry.field("px").name == "Price"
    assert registry.get_field(55) == registry.get_field("symbol")
    assert 44 in registry
    assert "44" not in registry, "a tag query never consults names"
    with pytest.raises(KeyError, match="tag 35"):
        registry.field_by_tag(35)

    # Explicit identity/name pin a branch. Omitted tag lookup infers the only
    # matching venue definition, while the standard canonical name wins.
    assert registry.field_by_id("5055:cme").fix.branch == "cme"
    assert registry.field_by_name("SYMBOL", "cme").fix.tag == 5055
    assert registry.field_by_name("symbol", "").fix.tag == 55
    assert registry.field_by_tag(5055).fix.branch == "cme"
    assert registry.get_field("5055:cme") is None, "a string key is a name"

    # A key another field holds *in the same branch* is a conflict naming
    # both, and the branch; nothing changes.
    with pytest.raises(ValueError, match="held by Symbol") as conflict:
        registry.insert(fix_field("SymbolSfx", "utf8", "65:", "ticker"))
    assert 'branch \\"\\"' in str(conflict.value)
    assert len(registry) == 3

    # A merge keeps what only the stored field declared and adds the rest.
    incoming = fix_field("SYMBOL", "utf8", "55:", "Sym")
    incoming.fix.tags = [65]
    registry.update(incoming)
    merged = registry.field_by_tag(65)
    assert merged.name == "SYMBOL"
    assert merged.fix.aliases == ["Sym", "Ticker"]
    # A datatype disagreement is refused, never widened.
    with pytest.raises(ValueError):
        registry.update(fix_field("Symbol", "large_utf8", "55:"))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    # Iteration is tag-major, then by branch digest.
    assert [field.fix.id for field in registry] == [
        "44:",
        "55:",
        "5055:cme",
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
      fixField('Symbol', 'utf8', '55:', 'Ticker'),
      fixField('Price', 'decimal128(20, 8)', '44:', 'Px'),
      // The venue dictionary reuses the name `Symbol`, the normal case.
      fixField('Symbol', 'utf8', '5055:cme'),
    ])

    // Any spelling of a name or alias answers the canonical field.
    assert.equal(registry.fieldByName('TICKER', fix.STANDARD_BRANCH).name, 'Symbol')
    assert.equal(registry.field('px').name, 'Price')
    assert.ok(registry.getField(55).equals(registry.getField('symbol')))
    assert.equal(registry.has(44), true)
    assert.equal(registry.has('44'), false, 'a tag query never consults names')
    assert.throws(() => registry.fieldByTag(35), /tag 35/)

    // Explicit identity/name pin a branch. Omitted tag lookup infers the only
    // matching venue definition, while the standard canonical name wins.
    assert.equal(registry.fieldById('5055:cme').fix.branch, 'cme')
    assert.equal(registry.fieldByName('SYMBOL', 'cme').fix.tag, 5055)
    assert.equal(registry.fieldByName('symbol', '').fix.tag, 55)
    assert.equal(registry.fieldByTag(5055).fix.branch, 'cme')
    assert.equal(registry.getField('5055:cme'), null, 'a string key is a name')

    // A key another field holds *in the same branch* is a conflict naming
    // both, and the branch; nothing changes.
    assert.throws(
      () => registry.insert(fixField('SymbolSfx', 'utf8', '65:', 'ticker')),
      /held by Symbol/,
    )
    assert.equal(registry.size, 3)

    // A merge keeps what only the stored field declared and adds the rest.
    const incoming = fixField('SYMBOL', 'utf8', '55:', 'Sym')
    incoming.fix.tags = [65]
    registry.update(incoming)
    const merged = registry.fieldByTag(65)
    assert.equal(merged.name, 'SYMBOL')
    assert.deepEqual(merged.fix.aliases, ['Sym', 'Ticker'])
    // A datatype disagreement is refused, never widened.
    assert.throws(() => registry.update(fixField('Symbol', 'large_utf8', '55:')))
    assert.ok(registry.fieldByTag(55).dtype.equals(DataType.from('utf8')))

    // Iteration is tag-major, then by branch digest.
    assert.deepEqual(
      [...registry].map((field) => field.fix.id),
      ['44:', '55:', '5055:cme'],
    )
    assert.equal(registry.remove('sym').name, 'SYMBOL')
    assert.equal(registry.getFieldByTag(65), null)
    // `remove` reads a string as a standard name, so a vendor field leaves by
    // its identifier.
    assert.equal(registry.removeById('5055:cme').name, 'Symbol')
    assert.equal(registry.size, 1)
    ```

## Tiers

A later tier runs only after every earlier one missed, and a tag query never consults names.

| tier | key |
| ---: | --- |
| 1 | canonical identifier |
| 2 | alternate identifier |
| 3 | canonical name, folded |
| 4 | alias, folded |

An explicit branch never crosses into another dictionary. With no branch, one deterministic order decides the answer.

| step | key |
| ---: | --- |
| 1 | standard canonical key |
| 2 | named-branch canonical keys, in branch-name order |
| 3 | standard alternate key |
| 4 | named-branch alternate keys, in branch-name order |

## Accessors

Every lookup has an optional form and a failing twin. The twin raises a typed absence naming the key (`tag 35`, `identifier 5001:cme`, `name "MsgType"`, `path "a.b"`).

| optional | failing | key |
| --- | --- | --- |
| `get_field_by_id(FixId)` | `field_by_id` | canonical or alternate identifier, in any branch; carries the implementation |
| `get_field_by_tag(i32)` | `field_by_tag` | canonical or alternate tag, through the deterministic best-match order |
| `get_field_by_name(&str, Option<&FixBranch>)` | `field_by_name` | canonical name or alias, folded; an omitted branch takes the best match |
| `get_field_by_path(&str, Option<&FixBranch>)` | `field_by_path` | the whole string as a name first, else the first segment here and the rest through `Field::get_field_by_path` |
| `get_field(impl Into<FixKey>)` | `field` | matches `FixKey::Tag` / `FixKey::Id` / `FixKey::Name` once and redirects to the rows above |

`FixKey` is built from an `i32`, a `FixId`, a `&str` or a `&String`, exactly as `FieldKey` is, so `registry.field(35)` and `registry.field("MsgType")` are one call.

| call | answers |
| --- | --- |
| `contains(impl Into<FixKey>)` | whether the key resolves, through the same tiers |
| `iter` | every field in ascending packed-identifier order, tag-major then by branch digest |
| `next_field_after` | the cursor each binding advances with; the same order as `iter` |
| `len` / `is_empty` | the one field vector counted |

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

## Protocol and MsgType inference

`infer_bytes_protocol` and `infer_text_protocol` classify one arbitrary log line without parsing a message. `infer_bytes_msgtype` and `infer_text_msgtype` borrow the value of numeric tag 35 or symbolic `MSGTYPE`.

| line | answer |
| --- | --- |
| numeric pairs | `text/fix` |
| known symbolic pairs | `text/ullink` |
| a numeric frame also carrying key/value names | `text/fixul` |
| official `XmlData(213)` whose payload begins with XML | `text/fixml` |
| unrelated text | `application/octet-stream` |

The scan locates `8=` first, then `35=`, then the first pair-shaped run. It holds one separator, stops at checksum tag 10, and reads no prefix, suffix, XML attribute or `#A=1` inside a value as a field.

=== "Rust"

    ```rust
    use yggdryl::{FixRegistry, MimeType};

    let registry = FixRegistry::new();
    let line = b"sending 8=FIX.4.4|35=D|55=AAPL|10=001| queued seq=7";
    assert_eq!(registry.infer_bytes_protocol(line), MimeType::FIX);
    assert_eq!(registry.infer_bytes_msgtype(line), Some(&b"D"[..]));
    ```

=== "Python"

    ```python
    from yggdryl import MimeType
    from yggdryl.fix import FixRegistry

    registry = FixRegistry()
    line = "ACCOUNT=A1|MSGTYPE=D|SYMBOL=AAPL"
    assert registry.infer_text_protocol(line) == MimeType.ULLINK
    assert registry.infer_text_msgtype(line) == "D"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MimeType, fix } = require('yggdryl')

    const registry = new fix.FixRegistry()
    const line = '8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|'
    assert.ok(registry.inferTextProtocol(line).equals(MimeType.FIXUL))
    assert.equal(registry.inferTextMsgtype(line), 'D')
    ```

A raw `MSGTYPE=` anywhere in the line wins over tag 35, and `U` followed by an alphanumeric suffix routes to the canonical `UDF` root. Canonical spellings work in an empty registry, loaded aliases and alternate tags extend the same lookup, and the Rust byte path returns a slice of the input without allocating.

## Edges

- `registry.get_field("5055:cme")` -> `None`; a string key is a name, and only `FixId::from_str` parses an identifier.
- `registry.get_field_by_tag(5055)` with no branch -> the deterministic best match, so a named branch's own tag resolves; an explicit branch never crosses.
- Two names whose seeded XXH64 digests collide -> a read rechecks the field behind the digest and misses; a mutation refuses loudly.
- `contains("44")` -> `false`; a tag query never consults names, and a name query never consults tags.
- A path -> the whole string as a name first, keeping a dotted name reachable; then the first segment here, the rest through `Field::get_field_by_path` exactly.
- An alternate tag equal to another field's canonical tag, or an alias equal to another's canonical name -> legal, and it never wins.
- The same key twice in the same tier of one branch -> conflict.
- `insert` of a field whose key another field holds in the same branch -> conflict naming both fields and the branch; `len` unchanged.
- `insert` of a field matching one stored field on both identity halves -> `Ok(Some(prior))`, a replacement, never a conflict.
- `update` with a different datatype -> typed error; the stored datatype stays.
- `remove` with a path -> never a match; it takes a tag, an identifier or a name, and a bare one means the standard branch.
- Primitive and nested fields share one identity space; a repeating group claiming a scalar's tag, name, alternate tag or alias -> the same conflict as between two scalars.
- `install_global` after `global()` has resolved -> typed conflict (`already resolved` in the bindings); the value every caller saw cannot change.
- `YGGDRYL_FIX_REGISTRY` set to a missing directory -> error from `global()`, where an absent `~/.config/fix` is the empty registry.
- A tag outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` in a named branch -> refused ([FIX](index.md)); inside it a vendor field is also reachable by its `FixId` or a branch-qualified name.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::a_field_without_a_tag fix::tests::a_name_or_alias fix::tests::tier_order fix::tests::a_tag_query fix::tests::an_insert_conflict fix::tests::reinserting fix::tests::a_merge_follows fix::tests::a_rejected_merge fix::tests::removal_keeps fix::tests::specialized_and_generic fix::tests::iteration_follows fix::tests::iteration_and_the_cursor fix::tests::nestedness_routes fix::tests::an_omitted_branch_infers fix::tests::protocol_and_msgtype_inference fix::tests::a_nested_field_can_never fix::tests::two_branches_may_hold fix::tests::the_default_resolves
    cargo test -p yggdryl --test fix global
    cargo bench -p yggdryl --bench fix -- fix/resolve
    cargo bench -p yggdryl --bench fix -- fix/mutate
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "registry_resolves or explicit_branch or inference or registry_absence or registry_keys or registry_coerces or registry_iterates or seed_iterates or registry_insert or registry_mutation or install_global"
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix/fix.test.js
    node --test --test-name-pattern="resolves every key|explicit branch pins lookup|inference stays native|removeById|absence throws|number tag or a string name|coerced at the boundary|iterates lazily|seed iterates|insert, update and remove|shared registry|registry is a value|fix namespace is frozen|installing the process default" node/tests/fix/fix.test.js
    npm run --prefix node bench:fix
    ```

## Performance

Performance is a guardrail, not a second contract. The release target uses 400 generated fields and 100 fields in the second branch, while Python and JavaScript use 200 of each.

The timing runs report release builds on one Windows x86_64 host, so they are boundary-scale comparisons, not promises. The allocation test is the stronger hot-path assertion: canonical tag, identifier, folded name, alias, miss, path, protocol inference, MsgType inference and iteration allocate nothing in Rust.

Criterion takes ten samples with short warm-up and measurement windows in this phase.

```bash
cargo bench -p yggdryl --bench fix -- fix/resolve --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
YGGDRYL_BENCH_ITERATIONS=5000 npm run --prefix node bench:fix
```

<!-- PHASE2_BENCHMARK_RESULTS -->
