# FIX

FIX field definitions as ordinary fields: the `fix:` vocabulary on a field's view, a registry that
resolves a tag or a name to the canonical field, the shards it persists to through any
[`IOBase`](io.md) handle, the process-wide default, and the message value typed against one.

!!! note "Bindings"
    The Python and JavaScript surfaces land in the next change. Every example on this page is
    Rust for now.

## The vocabulary is metadata

A FIX field is a [`Field`](field.md) whose metadata carries the `fix:` namespace. The canonical
name is the field's own `name()`, the datatype its own `dtype()`, and the display name the generic
`display` key; the namespace adds only what FIX states beyond a field. It is read through
`field.as_fix()` (`FixField`) and written through `field.as_fix_mut()` (`FixFieldMut`), so a
caller never spells `fix:` - the property names live in one private place.

| Property | Key | Type | Meaning |
| --- | --- | --- | --- |
| `tag` | `fix:tag` | `i32` | canonical FIX tag, never negative |
| `tags` | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
| `aliases` | `fix:aliases` | ordered name list | alternate names, highest priority first |
| `description` | `fix:description` | text | the specification's own wording |

List-valued properties store as comma-separated text and parse on read. A write rejects an empty
element, a duplicate (aliases compared with ASCII case folded), an alias containing a comma, and a
negative tag; an empty list removes the property. `aliases()` is a lazy iterator over slices of the
stored text, so reading aliases allocates nothing; `tags()` parses integers and answers a `Vec`.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let mut field = DataType::decimal128(20, 8)?.nullable_field("OrderQty");
    field.as_fix_mut().set_tag(38)?;
    field.as_fix_mut().set_aliases(["Qty", "Quantity"])?;
    field.as_fix_mut().set_description("Quantity ordered.")?;
    field.set_display("Order quantity")?;

    assert_eq!(field.as_fix().tag()?, Some(38));
    assert_eq!(field.as_fix().tags()?, Vec::<i32>::new());
    assert_eq!(field.as_fix().aliases().collect::<Vec<_>>(), ["Qty", "Quantity"]);
    assert_eq!(field.as_fix().description(), Some("Quantity ordered."));
    // Stored as ordinary namespaced text, in the one metadata map.
    assert_eq!(field.get_metadata("fix:aliases"), Some("Qty,Quantity"));
    assert_eq!(field.as_fix().len(), 3);

    // A refusal names the full key and leaves the field unchanged.
    let error = field.as_fix_mut().set_tags(&[152, 152]).unwrap_err();
    assert!(error.to_string().contains("fix:tags"), "{error}");
    assert!(!field.has_metadata("fix:tags"));
    let error = field.as_fix_mut().set_tag(-1).unwrap_err();
    assert!(error.to_string().contains("fix:tag"), "{error}");
    assert_eq!(field.as_fix().tag()?, Some(38));
    ```

## Nesting needs no second type

A component is a Struct field whose children are its members; a repeating group is a List field
whose item is that Struct; the group's counter tag is the group field's own `fix:tag`. Every member
carries its own tag, and the one path resolver every [`Field`](field.md) has reaches them:
`NoPartyIDs.PartyID` descends through the list's item because a list is transparent to a dotted
path, and `NoPartyIDs.item.PartyID` spells the same route.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixRegistry};

    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452)?;
    let item = DataType::from_fields([party_id, role])?.required_field("item");
    let mut group = DataType::list(item).nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453)?;

    let registry = FixRegistry::from_fields([group])?;
    assert_eq!(registry.field_by_path("NoPartyIDs")?.as_fix().tag()?, Some(453));
    assert_eq!(registry.field_by_path("NoPartyIDs.PartyID")?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_path("NoPartyIDs.item.PartyRole")?.name(), "PartyRole");
    // A member is reached through its group, not registered on its own.
    assert!(registry.get_field_by_name("PartyID").is_none());
    ```

## The registry resolves in tiers

`FixRegistry` holds its fields in one vector and four indexes of positions over it: canonical tags
and alternate tags in two ordered maps, canonical names and aliases in two maps keyed by
ASCII-case-folded text. A lookup consults a later tier only when every earlier one missed:

1. canonical tag, then alternate tags;
2. canonical name folded, then aliases folded.

A tag query never consults names and a name query never consults tags. Either answers the canonical
field - its own `name()`, never the spelling the query used - and an alias can never take a name
away from a field that claims it canonically. Folding happens once, at insert; a probe hashes the
caller's text folded as it reads it, so a hit allocates nothing.

Every lookup has a specialized form for a key the caller already holds and a failing twin that
raises a typed absence naming the key (`tag 35`, `name "MsgType"`, `path "a.b"`):

| optional | failing | key |
| --- | --- | --- |
| `get_field_by_tag(i32)` | `field_by_tag` | canonical or alternate tag |
| `get_field_by_name(&str)` | `field_by_name` | canonical name or alias, folded |
| `get_field_by_path(&str)` | `field_by_path` | the whole string as a name first, else the first segment here and the rest through `Field::get_field_by_path` |
| `get_field(impl Into<FixKey>)` | `field` | matches `FixKey::Tag` / `FixKey::Name` once and redirects to the row above |

`FixKey` is built from an `i32`, a `&str` or a `&String`, exactly as `FieldKey` is, so
`registry.field(35)` and `registry.field("MsgType")` are one call. `contains` takes the same key,
`iter` walks the fields in ascending canonical-tag order, and `len` / `is_empty` count them.

Identity is the pair of canonical tag and folded canonical name, and two fields may share neither -
nor an alternate tag, nor an alias. `insert` answers `Ok(None)` for a fresh field, `Ok(Some(prior))`
when both halves of the identity match one stored field (a wholesale replacement), and a typed
conflict naming both fields and the key otherwise; it never silently replaces a different field.
Overlap *across* tiers is legal and decided by tier order. `update` merges a definition into the
stored field with the same tag: the incoming field wins the name spelling, nullability and every
metadata key both declare; the stored field keeps the keys only it declares; `tags` and `aliases`
concatenate, incoming first, deduplicated, order kept; a datatype disagreement is a typed error
naming both, never a silent widening. Both build the result first and check every key it would
claim, so a refusal leaves the vector and all four indexes untouched. `remove` takes a tag or a name
and answers the field.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixKey, FixRegistry};

    let mut symbol = DataType::Utf8.nullable_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut price = DataType::decimal128(20, 8)?.nullable_field("Price");
    price.as_fix_mut().set_tag(44)?;
    price.as_fix_mut().set_aliases(["Px"])?;
    let mut registry = FixRegistry::from_fields([symbol, price])?;

    // Any spelling of a name or alias answers the canonical field.
    assert_eq!(registry.field_by_name("TICKER")?.name(), "Symbol");
    assert_eq!(registry.field("px")?.name(), "Price");
    assert_eq!(registry.get_field(55), registry.get_field("symbol"));
    assert!(registry.contains(FixKey::Tag(44)));
    assert!(!registry.contains("44"), "a tag query never consults names");
    let error = registry.field_by_tag(35).unwrap_err();
    assert!(error.is_absent());

    // A key another field holds is a conflict naming both; nothing changes.
    let mut clash = DataType::Utf8.nullable_field("SymbolSfx");
    clash.as_fix_mut().set_tag(65)?;
    clash.as_fix_mut().set_aliases(["ticker"])?;
    let error = registry.insert(clash).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("held by Symbol"), "{error}");
    assert_eq!(registry.len(), 2);

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

    assert_eq!(registry.iter().map(|field| field.name()).collect::<Vec<_>>(), ["Price", "SYMBOL"]);
    assert_eq!(registry.remove("sym").map(|field| field.name().to_owned()), Some("SYMBOL".into()));
    assert!(registry.get_field_by_tag(65).is_none());
    ```

## Storage is shards under one handle

A registry reads and writes through one [`IOBase`](io.md) folder handle and nothing else. Shards
live at `<root>/records/<shard>.json` with `shard = tag / 100`, so tags 0-99 are `0.json` and
100-199 are `1.json`: a tag reaches exactly one shard by arithmetic, and an alternate tag is an
index entry that never fans a field across shards. Each shard is a JSON array of the core field
document - what `Field::into_value` projects - ordered by canonical tag and rendered indented, so
the whole `fix:` namespace persists through the path every field already has and the tracked seed
reads in a diff. Nothing else is composed: no envelope, no version marker.

`from_handle` is the one loader. It lists `records/`, reads every `<n>.json` leaf and inserts its
fields; every shard is loaded on open, because a name has no numeric structure to pick a shard
with and a dictionary is small enough that loading it whole costs less than lazy machinery. A
folder that does not exist lists nothing and answers the empty registry, as every handle's
laziness contract says; a shard that exists but does not parse, holds a field without a tag or with
a tag another shard owns, or holds a field the registry refuses, is a typed error naming the
shard's URL. `write_into` writes every populated shard whole - creation is a write consequence -
then removes any `<n>.json` no field populates, so a reload cannot resurrect a removed field.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, FixRegistry};

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
    let mut registry = FixRegistry::from_fields(fields)?;
    registry.write_into(&mut folder)?;

    let mut shards: Vec<String> = std::fs::read_dir(root.join("records"))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    shards.sort();
    // The alternate tag 20 wrote nothing into shard 0 beyond MsgType and StopPx.
    assert_eq!(shards, ["0.json", "1.json"]);

    let reloaded = FixRegistry::from_handle(&folder)?;
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_tag(20)?.name(), "ExecType");

    // Removing the only field of a shard removes the shard on the next write.
    registry.remove(100);
    registry.remove(150);
    registry.write_into(&mut folder)?;
    assert!(!root.join("records").join("1.json").exists());
    assert_eq!(FixRegistry::from_handle(&folder)?.len(), 2);

    // A folder that is not there loads as empty and is not created.
    let absent = Folder::new(root.join("absent"))?;
    assert!(FixRegistry::from_handle(&absent)?.is_empty());
    assert!(!absent.exists());
    let _ = std::fs::remove_dir_all(&root);
    ```

The repository ships a seed dictionary at `config/fix/records/<shard>.json`, written by `write_into`
itself: a small FIX 4.4 subset - the standard header and trailer, the order and execution fields,
the `Parties` component as a repeating group - with the specification's wording as each
description, a display name where FIX has one, declared aliases such as `Ticker` for `Symbol`, and
one alternate tag (`20` for `ExecType`, the pre-4.3 `ExecTransType` whose role it absorbed). It is
what the tests, benchmarks and this page resolve against; it is *not* in the default registry's
resolution order.

=== "Rust"

    ```rust
    use yggdryl::local::Folder;
    use yggdryl::FixRegistry;

    let seed = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config").join("fix");
    let registry = FixRegistry::from_handle(&Folder::new(seed)?)?;

    assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
    assert_eq!(registry.field_by_name("ticker")?.name(), "Symbol");
    assert_eq!(registry.field_by_tag(20)?.name(), "ExecType");
    assert_eq!(registry.field_by_path("NoPartyIDs.PartyID")?.as_fix().tag()?, Some(448));
    assert_eq!(registry.field_by_name("ClOrdID")?.display(), Some("Client order ID"));
    assert!(registry.len() < 40);
    ```

## One default registry per process

`FixRegistry::global()` answers the `Arc<FixRegistry>` every caller gets when it names none. It
resolves on the first call, on the calling thread, reading the environment exactly once - nothing
loads at module init and no thread is spawned - and every later call answers the same `Arc`. First
match wins:

1. a registry installed by `FixRegistry::install_global`, which fails with a typed conflict once
   the default has resolved so the value every caller saw cannot change underneath them;
2. the folder `YGGDRYL_FIX_REGISTRY` names - a URL, or a bare path - through the local backend;
3. `~/.config/fix`, the production default, reached through [`Folder::config`](local.md) when that
   folder exists; skipped when the machine has no `HOME` or `USERPROFILE`;
4. the empty registry.

Step 3 is the one place absence is not a failure: a machine with no dictionary installed is an
ordinary first-run state. A `YGGDRYL_FIX_REGISTRY` that is set but names no directory, a scheme
this crate has no backend for, or a malformed shard under either folder is an error from
`global()`, never the empty registry - and the default stays unresolved, so the next call retries
the load. The repository's own `config/fix` is not in this order: nothing walks up from the working
directory, because behaviour must not depend on where a process was started.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::Folder;
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

## A message carries its registry

`FixMsg` is a value plus the registry that types it: a root Struct [`Field`](field.md) - the only
row schema - and the row as the ordered `Scalar::Sequence` the root declares, validated and
canonicalized through `Field::validate_value` and `Field::canonicalize_value` like every row, so a
`Scalar::Record` input becomes that sequence. `FixMsg::new` links `FixRegistry::global()`;
`FixMsg::with_registry` keeps the `Arc` it is given. `registry()`, `as_field()` and `as_value()`
borrow the three parts.

The value accessors mirror the registry's and resolve through the linked registry, never a private
copy of its rules: `get_by_tag` / `by_tag` resolve the tag to its canonical name and pick the root
child of that name, falling back to a root child named by the tag's decimal text - an unknown tag a
transcriber retained is reachable, never dropped; `get_by_name` / `by_name` fold through the
registry's canonical spelling, then match a root child exactly; `get_by_path` / `by_path` try the
whole string as a name, then descend segment by segment - into a Struct child by name, or into a
List entry by a decimal index; `get` / `value` take a `FixKey` and redirect. A repeating group is a
List of Structs, so one of its members needs the entry's index: `NoPartyIDs.0.PartyID`.

Serialization is inherited, not written: `field.clone().into_json()` renders the schema,
[`into_json_scalar`](json.md) the value, and `from_json_scalar_with_field` reads it back typed,
ordered and canonicalized against the same root.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, from_json_scalar_with_field, into_json_scalar};

    let mut symbol = DataType::Utf8.required_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_aliases(["Ticker"])?;
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38)?;
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut parties = DataType::list(DataType::from_fields([party_id])?.required_field("item"))
        .nullable_field("NoPartyIDs");
    parties.as_fix_mut().set_tag(453)?;
    let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone(), parties.clone()])?);

    // The root carries a tag no dictionary explains, under its rendered name.
    let root = DataType::from_fields([qty, symbol, parties, DataType::Utf8.nullable_field("9999")])?
        .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::I64(100)),
        ("NoPartyIDs", Scalar::from_sequence([
            Scalar::from_record([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value)?;

    // The record became the ordered row the root declares.
    assert_eq!(msg.as_value().as_sequence().map(|row| row.len()), Some(4));
    assert_eq!(msg.by_tag(38)?, &Scalar::I64(100));
    assert_eq!(msg.by_name("ticker")?, &Scalar::from("AAPL"));
    assert_eq!(msg.by_path("NoPartyIDs.0.PartyID")?, &Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("NoPartyIDs.PartyID").is_err(), "a group member needs its index");

    // Schema and value serialize through the paths every field and value share.
    let schema = root.clone().into_json()?;
    assert!(schema.contains("\"fix:tag\":\"55\""), "{schema}");
    let text = into_json_scalar(msg.as_value())?;
    let read = from_json_scalar_with_field(&text, &root)?;
    assert_eq!(&read, msg.as_value());
    assert_eq!(FixMsg::with_registry(registry, root, read)?, msg);
    ```

## Edge cases

- Folding is ASCII only: `Größe` and `GRÖSSE` are two names. FIX spellings are ASCII, and a
  non-ASCII byte compares as it is.
- A tag is a decimal integer from `0` to `i32::MAX`. The setters refuse a negative one and the
  readers refuse stored text that is not exactly decimal digits (`+35`, `-35`, `3x`), so the shard
  arithmetic is total.
- A path tries the whole string as a name first, so a field whose name contains a dot stays
  reachable; only then is the first segment resolved here and the rest handed to
  `Field::get_field_by_path`, where the remainder matches child names exactly.
- An alternate tag equal to another field's canonical tag, or an alias equal to another field's
  canonical name, is legal and simply never wins. The same key twice in the *same* tier across two
  fields is a conflict.
- `remove` takes a tag or a name, never a path: a component's member is not a registry entry.
- `from_handle` reads only leaves named `<n>.json` with a decimal `n`; a README beside them is
  ignored and left alone by `write_into`'s cleanup. A field stored in the wrong shard is refused
  with the range that shard holds.
- `YGGDRYL_FIX_REGISTRY` must name an existing directory: a configured location that is not there
  is a misconfiguration, not a first run, so it is an error where `~/.config/fix` would be empty.
- `FixMsg::get_by_tag` renders an unknown tag on the stack, so a miss allocates nothing. The
  fallback matches the root child's name exactly - `9999`, never `09999`.
- `Field::get_field_by_path` is transparent to a list on a read; a write through
  `set_field_by_path` or `remove_field_by_path` still spells the item (`NoPartyIDs.item.PartyID`).

## Measured resolution cost

One local Windows x86_64 release run of the Criterion target (point estimates), over the tracked
seed of 34 fields unless a name says otherwise:

```console
cargo bench -p yggdryl --bench fix -- --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```

| operation | estimate |
| --- | ---: |
| `get_field_by_tag` hit | 4.5 ns |
| `get_field_by_tag` alternate-tag hit | 9.3 ns |
| `get_field_by_tag` miss | 8.0 ns |
| `get_field_by_name` hit | 64.9 ns |
| `get_field_by_name` hit, query differently cased | 65.9 ns |
| `get_field_by_name` alias hit | 111.6 ns |
| `get_field_by_name` miss | 94.4 ns |
| `get_field(FixKey::Tag)` generic tag hit | 12.8 ns |
| `get_field("Symbol")` generic name hit | 60.1 ns |
| `field(55)` failing-half tag hit | 6.6 ns |
| `get_field_by_path`, one segment | 62.4 ns |
| `get_field_by_path`, two segments (`NoPartyIDs.PartyID`) | 190.0 ns |
| `get_field_by_path`, three segments (`NoPartyIDs.item.PartyRole`) | 212.6 ns |
| baseline `HashMap<i32, Field>` tag hit | 17.7 ns |
| baseline `HashMap<String, Field>` hit after lowercasing the query | 96.3 ns |
| tag hit over 4034 fields | 29.0 ns |
| name hit over 4034 fields | 79.0 ns |
| alias hit over 4034 fields | 107.9 ns |
| `from_handle`, 1 shard of 10 fields | 632 us |
| `from_handle`, 10 shards of 10 fields | 4.56 ms |
| `from_handle`, 100 shards of 10 fields | 57.4 ms |
| `from_handle`, the seed (3 shards, 34 fields) | 1.67 ms |
| `write_into`, 100 shards | 300 ms |
| explicit-location autoload of the seed (URL parse, folder, load) | 1.75 ms |

The specialized accessor and the generic one it redirects to cost the same: the `FixKey` match is
one branch. A folded name hit costs what the plain `HashMap<String, Field>` baseline costs *before*
that baseline lowercases its query - the fold happens inside the hash, so no folded copy is built.
`from_handle` scales with the number of shards, because every shard is read on open; the seed
loads in the time a single shard folder of ten fields does.
