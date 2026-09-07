# Registry

`FixRegistry` resolves a field in tiers, refuses partial writes, iterates tag-major, and is the default a [message](message.md) links.

## Contract

| | |
| --- | --- |
| Owns | `FixRegistry`, `FixKey`, one field vector and four hash indexes of positions over it, `global()` / `install_global` |
| Index keys | Canonical and alternate identities use `FixId` directly; canonical names and aliases use independent seeded XXH64 digests over the branch digest and the folded name |
| Collision | A read rechecks the field behind every name digest, so a digest collision is a miss; a mutation refuses it loudly |
| Tiers | canonical identifier, alternate identifier, canonical name folded, alias folded; a later tier only when every earlier one missed |
| Branch | An explicit branch never crosses into another dictionary; with no branch, one deterministic best-match order decides; outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` no named branch may hold a tag |
| Branch aliases | A declaration may name other spellings it answers to; `branch_named` tries the canonical name first and an alias only after, so a dialect is never shadowed by another's second spelling. An alias is a lookup spelling alone - the canonical name is what a field stores and what a `FixId` packs |
| String key | A colon-bearing string is a name, never an identifier; `From<&str>` cannot fail, so an identifier is parsed with `FixId::from_str` |
| Folding | ASCII case, once at insert; a probe hashes the query folded beside an inline branch and allocates nothing on a hit |
| Identity | The `FixId`, and separately the branch plus folded canonical name; two fields share neither, nor an alternate identifier, nor an alias |
| Conflict | The same key twice in one tier of one branch -> typed conflict naming both fields and the branch; overlap across tiers or branches is legal |
| Order | `iter` and `next_field_after` walk ascending packed identifiers, tag-major then by branch digest |
| Versions | `fix:lineage` dates a field; `field_at` / `get_field_at` filter one read by it, `versions` and `newest` are derived from every lineage the dictionary holds |
| Merge | `FixFieldMut::merge_with` folds two definitions of one tag with a rule per key, in one write; `update` calls it |
| Fold | `merge_with` is the one place two dictionaries combine - fields add-or-update, dialects fold beside them, aliases accumulate. `add_fields` is the same fold over a bare field list, `add_cfb_file` a parse in front of it. All three are one mutation: a refusal writes nothing, and all answer the counts added and merged |
| CBlock | `FixRegistry::from_cfb` answers one Ullink CBlock's vocabulary and its message roots; `FixField::from_cfb_file` answers the vocabulary alone; `add_cfb_file` reads one into this dictionary whole - the fold, plus the dialect the root element declares, plus the file's own stem as an alias |
| Codes | `fix:codes` carries a field's vocabulary; any spelling of a member reaches its wire value through three tiers, and an unresolved one falls through |
| Inference | Classifying a line is transport, not FIX: `MimeType`, `MsgType` and `Direction` each answer for themselves, with no dictionary |
| Default | `global()` resolves once, on the first call, reading the environment once; every later call answers the same `Arc` |
| Bindings | Python `yggdryl.fix.FixRegistry`, `global_registry`, `install_global_registry`, `fix_cfb_fields`, and `FixRegistry.merge_with` / `add_fields` / `add_cfb_file`; JavaScript `fix.FixRegistry`, `fix.globalRegistry`, `fix.installGlobalRegistry` |

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

## Browser registry workbench

<div class="ygg-ui" data-fix="registry" markdown="1">
The registry snapshot and editor need JavaScript.
</div>

The workbench has one UI and two authority modes. Read-only use accepts either
a committed manifest generated by the package or full canonical documents. A
connected application supplies callbacks that call the native package; only
those callbacks may create, update, or remove definitions.

| Mode | Input | Authority | Writes |
| --- | --- | --- | --- |
| Static manifest | `model`, optionally `loadDetails` | Package-generated manifest | Disabled |
| Static documents | Canonical `Field.toJSON()` documents | Supplied snapshot | Disabled |
| Connected | Canonical `Field.toJSON()` documents and `actions` | Native `FixRegistry` behind the callbacks | `insert`, `update`, `removeById` |

The browser module projects `fix:tag` and `fix:branch` verbatim into the
callback identifier. It never validates or resolves that identifier, chooses a
lookup tier, merges a definition, or interprets other protocol metadata. It
displays callback results and surfaces thrown native errors without translating
them.

### Native callbacks

Keep the native addon on the Node side. The adapter reconstructs every submitted
field with `Field.fromJSON()` before it reaches the registry and returns
canonical documents after the operation.

```javascript
const assert = require('node:assert/strict')
const { Field, fix } = require('yggdryl')

const symbol = Field.from('Symbol: utf8')
symbol.fix.id = '55:'
const registry = fix.FixRegistry.fromFields([symbol])
const documents = () => registry.toJSON()
const actions = {
  list: async () => documents(),
  insert: async (document) => {
    registry.insert(Field.fromJSON(document))
  },
  update: async (document) => {
    registry.update(Field.fromJSON(document))
  },
  removeById: async (id) => {
    registry.removeById(id)
  },
}

async function main() {
  const side = Field.from('Side: utf8')
  side.fix.id = '54:'
  await actions.insert(side.toJSON())
  assert.equal((await actions.list()).length, 2)
  await actions.removeById('54:')
  assert.equal((await actions.list()).length, 1)
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
```

Mount the browser half with the separately shipped UI subpath. `actions` may
call an HTTP endpoint, a local bridge, or another transport; the component
awaits it and remains unaware of the transport.

```{ .javascript .ignore }
import { fixRegistryEditor } from 'yggdryl/ui/fix'

const editor = fixRegistryEditor({
  fields: await actions.list(),
  actions,
  pageSize: 60,
})
document.querySelector('[data-fix="registry"]').replaceChildren(editor.element)
console.assert(editor.element.classList.contains('ygg-ui__registry-editor'))
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

## Versions are a filter on the read

A FIX field outlives the version that introduced it and is renamed and retyped on the way, so one registry holds every tag ever defined and a version filters the read. `fix:lineage` is the field's own history, oldest first, and `since`, `until` and deprecation are derived from it rather than stored beside it. Rust only.

| call | answers |
| --- | --- |
| `FixField::lineage()` | every entry, oldest first, as borrowed slices of the stored document |
| `FixField::since()` | the first entry's version |
| `FixField::until()` | the version of the entry that removed the field, when one did |
| `FixField::defined_at(Version)` | whether the field exists then; a field with no lineage exists at every version |
| `FixField::name_at(Version)` | the newest spelling at or before that version |
| `FixField::dtype_at(Version)` | the newest datatype at or before it, resolved through the schema grammar |
| `FixFieldMut::set_lineage(&[FixLineageEntry])` | writes the document, checks it against the field, and rewrites `fix:aliases` from it |
| `FixRegistry::get_field_at(Version, key)` / `field_at` | the key's field, or nothing when the lineage says it did not exist |
| `FixRegistry::versions()` | every version some field is dated at, ascending |
| `FixRegistry::newest()` | the greatest `FixPedigree` any lineage carries |

A `FixPedigree` is a `Version` and an optional extension pack, because the specification dates a change either way. Entries order on the pair, version first, so `5.0SP2` at EP204 precedes `5.0SP2` at EP309 instead of eleven years landing in one bucket.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixLineageEntry, FixPedigree, FixRegistry, Version};

    let mut last_qty = DataType::Float64.nullable_field("LastQty");
    last_qty.as_fix_mut().set_tag(32)?;
    last_qty.as_fix_mut().set_lineage(&[
        FixLineageEntry::new(FixPedigree::new("2.7".parse()?, None))
            .with_name("LastShares")
            .with_dtype("int"),
        FixLineageEntry::new(FixPedigree::new("4.3".parse()?, None))
            .with_name("LastQty")
            .with_dtype("Qty"),
    ])?;

    // The history answers the name and the datatype of any version.
    let view = last_qty.as_fix();
    assert_eq!(view.since(), Some("2.7".parse::<Version>()?));
    assert_eq!(view.name_at("4.2".parse()?), Some("LastShares"));
    assert_eq!(view.name_at("5.0SP2".parse()?), Some("LastQty"));
    assert_eq!(view.dtype_at("4.0".parse()?)?, Some(DataType::Int32));
    // A version older than the first entry is a version the field had not
    // been defined in, which is not the same as having no history at all.
    assert!(!view.defined_at("2.6".parse()?));

    // The writer derives the aliases, so a query by an old spelling resolves.
    assert_eq!(view.aliases().collect::<Vec<_>>(), ["LastShares"]);
    let registry = FixRegistry::from_fields([last_qty])?;
    assert_eq!(registry.field("LastShares")?.name(), "LastQty");

    // The dictionary holds the tag whatever version is asked for; only the
    // read is filtered, and "the newest it holds" is a real pedigree.
    assert_eq!(registry.field_at("4.2".parse()?, 32)?.name(), "LastQty");
    assert!(registry.get_field_at("2.6".parse()?, 32).is_none());
    assert_eq!(registry.newest(), Some(FixPedigree::new("4.3".parse()?, None)));
    assert_ne!(registry.newest().unwrap().version(), Version::MAX);
    ```

### The document

`fix:lineage` is one canonically rendered JSON document: an `entries` array, entries sorted oldest first, keys within an entry in the order below, no whitespace. Every key beyond `since` is optional, because most versions change nothing and an entry stating only a version means "present, unchanged". `since` leads because it is what every read keys on, so a version filter compares it and stops.

| key | holds |
| --- | --- |
| `since` | the version, required |
| `ep` | the extension pack that dated the change |
| `name` | the spelling from that version on |
| `type` | the FIX datatype name from that version on, in the spelling the grammar already resolves |
| `deprecated` | `true` where the specification deprecated the field |
| `removed` | `true` where it removed it, which ends the field's life |
| `doc` | the specification's wording as of that version |

The read is a borrowed scan rather than a parse, so `name_at` over a dated dictionary allocates nothing. It is safe only because the rendering is canonical and checked on the way in: a reader knows which key can come next, so a hand-edited document with reordered or repeated keys is refused with its byte position instead of mis-read.

### Two derivations belong to the writer

`set_lineage` refuses a newest entry that disagrees with the field's own name or datatype, naming both sides, so the lineage is the authority and the field cannot drift from it. It then rewrites `fix:aliases` from the historical spellings, so an old name resolves through the index that already exists. Both are the writer's rather than a caller's, which is what makes them undriftable.

### Edges

- A field with no lineage answers `None` everywhere and `defined_at` is true at every version: no history is no filter, so an undated dictionary resolves as it always has.
- A field whose earliest entry postdates the version asked for is not defined then. A field introduced in 2.7 did not exist in 2.6.
- A malformed document answers nothing rather than something wrong. `lineage()` and `dtype_at` report it with a byte position; `since`, `until`, `name_at` and `defined_at` answer as though the field had no history, and neither path allocates.
- Two entries sharing one pedigree are refused: two statements about one dated point cannot both be the field's.
- An empty slice removes the document and the aliases it derived.
- `set_lineage` is atomic. A refusal leaves the field exactly as it was, aliases included.
- The registry stays version-agnostic. There is no registry-wide default version; a caller who wants one holds a `Version` beside the registry.
- "FIX Latest" is a moving label and is never stored as a version. `newest()` resolves it to the real pedigree the dictionary carries, never `Version::MAX`, which would compare wrongly against a field genuinely dated at the newest version.
- FIXT.1.1 is not modelled: session tags carry the application version that first defined them.
- The lineage carries enough to rename and retype a field between versions. The expression-driven normalization layer — conditions, lookups and value mappings — is not here and needs an evaluator.

## A field carries its code set

Most FIX fields with a vocabulary are `int`, `Boolean` or `String` and carry their members as a code set: a wire value, a symbolic name, usually a sentence of documentation. `fix:codes` is that vocabulary, ordered by wire value, and any spelling of a member reaches its value. Rust only.

It is a second key beside [`AsciiEnum`](../types/ascii.md), not a second copy: that type is name to ASCII value packed through the field's own width, so it accepts only ASCII-width and coded datatypes and carries no description or pedigree. A field may carry both and neither derives from the other.

| call | answers |
| --- | --- |
| `FixField::codes()` | every code, ordered by wire value, borrowed |
| `FixField::code(&str)` | the code one wire value stands for |
| `FixField::code_by_name(&str)` | the code one symbolic name or alias stands for, folded |
| `FixField::code_at(Version, &str)` | the same as `code`, filtered to one version |
| `FixField::code_value(&str)` | any spelling resolved to its wire value, through the three tiers |
| `FixField::code_name(&str)` | the symbolic name one wire value stands for |
| `FixField::code_value_at(Version, &str)` / `code_name_at` | the same, filtered to one version |
| `FixFieldMut::set_codes(&[FixCode])` | writes the set canonically; an empty slice removes it |
| `FixFieldMut::remove_codes()` | removes the set and answers what it held |

### Three tiers, and a fall-through

`code_value` composes three tiers and stops at the first that answers. A spelling that reaches none answers `None` and the caller keeps its own text: a venue sends codes no dictionary lists exactly as it sends fields no dictionary names, and refusing one would drop data.

| tier | key | note |
| ---: | --- | --- |
| 1 | the text as a wire value, exactly | `4` is `4`; a spelling that is already a legal code is never reinterpreted as somebody's name |
| 2 | the folded symbolic name, then any alias | the crate's one fold, so case and `_`, `-`, space all fall away |
| 3 | the leading parenthesized abbreviation of the description | `"Good Till Date (GTD)"` answers `gtd` |

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCode, Version};

    let mut comm_type = DataType::Utf8.nullable_field("CommType");
    comm_type.as_fix_mut().set_tag(13)?;
    comm_type.as_fix_mut().set_codes(&[
        FixCode::new("PerUnit", "1"),
        FixCode::new("PercentageWaivedCashDiscount", "4"),
        FixCode::new("PointsPerBondOrContract", "6")
            .with_description("Good Till Date (GTD) points per bond"),
        FixCode::new("BasisPoints", "7")
            .with_since("5.0SP2".parse::<Version>()?, Some(208)),
    ])?;
    let view = comm_type.as_fix();

    // Tier 1: a legal wire value is never read as somebody's name.
    assert_eq!(view.code_value("4"), Some("4"));
    assert_eq!(view.code_name("4"), Some("PercentageWaivedCashDiscount"));

    // Tier 2: one fold, so four spellings are one.
    assert_eq!(view.code_value("percentage_waived_cash_discount"), Some("4"));
    assert_eq!(view.code_value("PERCENTAGE WAIVED CASH DISCOUNT"), Some("4"));

    // Tier 3: the leading abbreviation of the description.
    assert_eq!(view.code_value("gtd"), Some("6"));

    // A version hides a code deprecated at or before it, and nothing else: a
    // value added later still resolves, because a venue that stated the wrong
    // version is a venue whose values are still worth reading.
    assert_eq!(view.code_value("BasisPoints"), Some("7"));
    assert_eq!(view.code_value_at("4.4".parse::<Version>()?, "BasisPoints"), Some("7"));

    // A venue's own spelling falls through unchanged rather than failing.
    assert_eq!(view.code_value("VenueOwnCommission"), None);
    ```

### Two traps in tier 3

A *numeric* parenthesization is a tag cross-reference and never a spelling, so `"Broken date; SettlDate (64) is required"` leaves `64` alone. And only the abbreviation on the leading phrase counts, so `"Swap Value Factor (SVP) through a central counterparty (CCP)"` answers `svp` and not `ccp`.

### Ambiguity answers nothing

Two codes folding to one spelling answer `None` rather than whichever the scan met first. So the name tier does **not** stop at its first match: it runs the whole set and answers only on exactly one. That is affordable because tier 1 is the hot path and a spelling lookup comes from human or JSON input.

Two *names* sharing one *value* are an alias, not an ambiguity, and resolve to that value.

### The document

`fix:codes` is one canonical JSON document: a `codes` array ordered by wire value, keys within a code in the order below, no whitespace. Only `value` and `name` are required.

| key | holds |
| --- | --- |
| `value` | the wire value, required, and the key every lookup keys on |
| `name` | the symbolic name, required |
| `since` | the version the specification added this code at |
| `ep` | the extension pack that dated the addition |
| `deprecated` | the version the specification deprecated it at |
| `sort` | the presentation rank the specification gives it |
| `group` | the group the specification files it under |
| `aliases` | venue and per-version spellings that also reach this code |
| `doc` | the specification's own wording |

`value` leads every code for a reason that is measurable rather than tidy: it makes a record's opening a literal byte sequence, and one that cannot occur inside a stored string, since a quote inside one is escaped. So `code(value)` addresses the record it wants in a single pass over the bytes and reads only that one, instead of parsing every code it passes.

A code's pedigree is stored as real numbers. Many codes are dated by extension pack alone — `BasisPoints` is "Added EP208" rather than added in a version — so a moving label never becomes a stored one.

### Edges

- An unresolved spelling is never an error. `code_value` answers `None`, and the caller keeps its text.
- Two codes may share a value; two codes may not share a name, folded. `set_codes` refuses the second, naming it, and leaves the field unchanged.
- A code stating an empty value or an empty name is refused.
- An empty slice removes the property rather than storing an empty set.
- A malformed document answers nothing rather than something wrong: `codes()` reports it with a byte position, while `code`, `code_by_name` and `code_value` answer `None`. Neither path allocates.
- Keys follow the document's declared order, so a hand-edited document with reordered or repeated keys is refused rather than mis-scanned. A reordered one also stops leading with `value`, so the addressed search misses it and falls back to the walk, which is what reports it.
- `remove_codes` removes the property before it parses what it held, because a document a reader refuses is one a caller asked to take away.

## One merge, with a rule per key

Several sources describe one tag — FIX Latest, a QuickFIX dictionary, a vendor orchestration, a `.cfb` — and `FixFieldMut::merge_with` folds one into another in a single pass. The receiver is the incoming definition and wins every shared key; the other keeps only what it alone declares. Rust only.

| key | rule |
| --- | --- |
| `fix:branch`, `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
| `fix:tags` | union, incoming first, order kept, deduplicated |
| `fix:aliases` | union, folded, incoming first — then rewritten from the merged lineage |
| `description` | not FIX's key, so this half leaves it alone in both directions; `FixRegistry::update` carries it through the generic merge |
| `fix:lineage` | merged by pedigree, incoming winning an equal pair, re-sorted oldest first |
| `fix:codes` | merged by wire value, incoming winning a shared value |
| any other `fix:` key | incoming wins; stored keeps what only it has |

Precedence is the caller's ordering, not a field on the merge. A generator folds its lowest-priority source first, so the highest-priority one is the last merged and wins — one concept, in the one place that knows about sources.

A description is not merged here at all: what a column holds is a fact about the column rather than a FIX fact, so it lives on the generic `description` key beside `alias`, `comment` and `display`, and the generic half of `update` is what folds it.

The whole merged namespace is written once. `FixRegistry::update` calls it for the `fix:` half and the shared metadata merge for the generic half, because the protocol view reaches only its own namespace by design.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FixCode, FixLineageEntry, FixPedigree, Version};

    let mut stored = DataType::Utf8.nullable_field("LastQty");
    stored.as_fix_mut().set_tag(32)?;
    stored.as_fix_mut().set_tags(&[65])?;
    stored.as_fix_mut().set_description("the stored wording")?;
    stored.as_fix_mut().set_codes(&[FixCode::new("StoredOnly", "9")])?;

    let mut incoming = DataType::Utf8.nullable_field("LastQty");
    incoming.as_fix_mut().set_tag(32)?;
    incoming.as_fix_mut().set_tags(&[67, 65])?;
    incoming.as_fix_mut().set_lineage(&[
        FixLineageEntry::new(FixPedigree::new("2.7".parse::<Version>()?, None))
            .with_name("LastShares"),
        FixLineageEntry::new(FixPedigree::new("4.3".parse::<Version>()?, None))
            .with_name("LastQty"),
    ])?;

    incoming.as_fix_mut().merge_with(&stored.as_fix())?;
    let merged = incoming.as_fix();

    assert_eq!(merged.tags()?, [67, 65]);
    // The stored side keeps what only it declared.
    assert_eq!(merged.code_name("9"), Some("StoredOnly"));
    // A description is not this half's key, so merging two FIX views leaves it
    // alone in both directions; `FixRegistry::update` is what carries it.
    assert_eq!(merged.description(), None);
    // The aliases come from the merged lineage, never from the union.
    assert_eq!(merged.aliases().collect::<Vec<_>>(), ["LastShares"]);
    ```

### Edges

- Identity is checked before anything is built, so a refusal costs neither a render nor a write.
- A merge is atomic: the replacement is validated whole before it is applied, so a refusal leaves the field exactly as it was.
- A merge that adds nothing leaves the field byte-identical.
- A `fix:` key this vocabulary does not name still travels: it is one side's statement, not something a replace may drop.

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
| `add_fields`, absent identity | inserted, and counted as added |
| `add_fields`, stored identity | merged through `update`, and counted as merged |
| `add_fields`, a refusal partway | nothing is written: the whole fold is staged and only then adopted, so a refusal on the last field of a thousand leaves the dictionary as it was |
| `remove` | takes a tag, an identifier or a name, never a path, and answers the field |

## Folding a second source in

Several sources describe one dictionary, and `FixRegistry::merge_with` is how a later one enters an earlier one. It is the single place that knows how two dictionaries combine, and the other two entry points are it with something in front:

| call | what it takes |
| --- | --- |
| `merge_with(other)` | another dictionary: its fields *and* its dialects |
| `add_fields(fields)` | a bare field list, folded by the same rule |
| `add_cfb_file(handle, branch, aliases)` | one Ullink CBlock, parsed and then merged |

The field rule is the same in all three: a field whose canonical identity is absent is inserted, one already stored is merged, and the counts say which was which. A bare `insert` loop would replace the stored definition wholesale and drop what only it declared; a bare `update` loop would refuse everything new.

A dialect folds beside the fields. One the dictionary does not hold arrives whole; one it holds takes the incoming record - version and session pair - while keeping every spelling it already answered to, because reading a second source is not a statement that the first one's names were wrong.

`FixRegistry::add_cfb_file` is the whole ingest in one call: it folds the vocabulary exactly as `add_fields` does, and records the dialect the root element declares - the FIX version and the session `CompID` pair - which reading the fields alone loses, because a field carries its branch's *name* and nothing else of it. The location's own stem also becomes a branch alias whenever it is not already the name, so a dictionary read from `MSFIX44.cfb` under the branch `morgan` still answers to `msfix44`; a branch the dictionary already holds keeps the spellings it already answered to, because reading a second file is not a statement that the first one's names were wrong.

`FixField::from_cfb_file` is the source that made the fold worth having. It answers one Ullink CBlock's vocabulary alone - keyed, in declaration order, code sets attached - where `FixRegistry::from_cfb` answers a whole registry plus the message roots its grammar bindings describe. Both build the dictionary, so both refuse the same files; the vocabulary door just drops the one it built. A CBlock never names itself, so with no branch supplied the handle's own stem does: `bloomberg.cfb` reads into the branch `bloomberg`. Rust and Python only.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder};
    use yggdryl::{DataType, FixField, FixRegistry};

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-fix-cfb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let written = |name: &str, body: &str| -> yggdryl::Result<File> {
        let mut file = File::new(root.join(name))?;
        file.write_all_bytes(body.as_bytes())?;
        Ok(file)
    };

    // One counterparty's file, named by the file it arrives as.
    let bloomberg = written("bloomberg.cfb", r#"<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4" sendercompid="OURDESK" targetcompid="BLPFIX">
        <vocabulary>
            <vocabulary-tag name="55" alt="Symbol" type="string">
                <description>Ticker symbol.</description>
            </vocabulary-tag>
            <vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />
        </vocabulary>
    </cplugin-configuration>"#)?;

    // No branch supplied, so the stem names the dialect. A dialect claims only
    // the user-defined range, so tag 55 stays FIX's and 10001 is Bloomberg's.
    let fields = FixField::from_cfb_file(&bloomberg, None)?;
    assert_eq!(fields.iter().map(yggdryl::Field::name).collect::<Vec<_>>(), ["symbol", "excludeddealers"]);
    assert!(fields[0].as_fix().branch()?.is_standard());
    assert_eq!(fields[1].as_fix().branch()?.name(), "bloomberg");
    let mut registry = FixRegistry::from_fields(fields)?;

    // A second counterparty's file folds in: the tag both declare is merged,
    // the one only this file declares is added.
    let morgan = written("morgan.cfb", r#"<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4" sendercompid="OURDESK" targetcompid="MSFIX">
        <vocabulary>
            <vocabulary-tag name="55" alt="Symbol" type="string" />
            <vocabulary-tag name="44" alt="Price" type="float" />
        </vocabulary>
    </cplugin-configuration>"#)?;
    let (added, merged) = registry.add_fields(FixField::from_cfb_file(&morgan, None)?)?;
    assert_eq!((added, merged), (1, 1));
    // The fold kept the description only the first file declared.
    assert_eq!(registry.field_by_tag(55)?.description(), Some("Ticker symbol."));
    assert_eq!(registry.field_by_tag(44)?.dtype(), &DataType::Float32);

    // A datatype that disagrees with the stored definition is refused, never
    // widened - which is what a CBlock's generic `integer` meets.
    let retyped = written("retyped.cfb", r#"<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4">
        <vocabulary><vocabulary-tag name="55" alt="Symbol" type="integer" /></vocabulary>
    </cplugin-configuration>"#)?;
    let before = registry.clone();
    let error = registry.add_fields(FixField::from_cfb_file(&retyped, None)?).unwrap_err();
    assert!(error.to_string().contains("utf8"), "{error}");
    // The fold is one mutation, so a refusal leaves the dictionary as it was.
    assert_eq!(registry, before);
    assert_eq!(registry.len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pytest

    from yggdryl.fix import FixRegistry, fix_cfb_fields

    workspace = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-fix-cfb-"))


    def written(name: str, body: str) -> pathlib.Path:
        path = workspace / name
        path.write_text(body, encoding="utf-8")
        return path


    # One counterparty's file, named by the file it arrives as.
    bloomberg = written("bloomberg.cfb", """<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4" sendercompid="OURDESK" targetcompid="BLPFIX">
        <vocabulary>
            <vocabulary-tag name="55" alt="Symbol" type="string">
                <description>Ticker symbol.</description>
            </vocabulary-tag>
            <vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />
        </vocabulary>
    </cplugin-configuration>""")

    # No branch supplied, so the stem names the dialect. A dialect claims only
    # the user-defined range, so tag 55 stays FIX's and 10001 is Bloomberg's.
    fields = fix_cfb_fields(bloomberg)
    assert [field.name for field in fields] == ["symbol", "excludeddealers"]
    assert fields[0].fix.branch == ""
    assert fields[1].fix.branch == "bloomberg"
    registry = FixRegistry.from_fields(fields)

    # A second counterparty's file folds in: the tag both declare is merged,
    # the one only this file declares is added.
    morgan = written("morgan.cfb", """<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4" sendercompid="OURDESK" targetcompid="MSFIX">
        <vocabulary>
            <vocabulary-tag name="55" alt="Symbol" type="string" />
            <vocabulary-tag name="44" alt="Price" type="float" />
        </vocabulary>
    </cplugin-configuration>""")
    assert registry.add_fields(fix_cfb_fields(morgan)) == (1, 1)
    # The fold kept the description only the first file declared.
    assert registry.field_by_tag(55).description == "Ticker symbol."
    assert registry.field_by_tag(44).dtype.id == "float32"

    # A datatype that disagrees with the stored definition is refused, never
    # widened - which is what a CBlock's generic `integer` meets.
    retyped = written("retyped.cfb", """<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration version="1.2" fix-version="4.4">
        <vocabulary><vocabulary-tag name="55" alt="Symbol" type="integer" /></vocabulary>
    </cplugin-configuration>""")
    with pytest.raises(ValueError, match="utf8"):
        registry.add_fields(fix_cfb_fields(retyped))
    # The fold is one mutation, so a refusal leaves the dictionary as it was.
    assert registry.field_by_tag(55).dtype.id == "utf8"
    assert len(registry) == 3
    ```

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
    // Names are folded, and the specification's own spelling stays on `display`.
    assert_eq!(global.field_by_tag(55)?.name(), "symbol");
    assert_eq!(global.field_by_tag(55)?.display(), Some("Symbol"));
    assert!(Arc::ptr_eq(FixRegistry::global()?, global), "resolved once");

    // A message built without a registry links that same `Arc`.
    let root = DataType::from_fields([global.field_by_tag(55)?.clone()])?.required_field("row");
    let msg = FixMsg::new(root, Scalar::from_record([("symbol", Scalar::from("AAPL"))])?)?;
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
    # Names are folded, and the specification's own spelling stays on `display`.
    assert default.field_by_tag(55).name == "symbol"
    assert default.field_by_tag(55).display == "Symbol"
    assert default == global_registry(), "resolved once"

    # A message built without a registry links that same dictionary.
    root = Field("row", DataType.from_fields([default.field_by_tag(55)]), nullable=False)
    assert FixMsg(root, {"symbol": "AAPL"}).registry == default

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
    // Names are folded, and the specification's own spelling stays on `display`.
    assert.equal(global.fieldByTag(55).name, 'symbol')
    assert.equal(global.fieldByTag(55).display, 'Symbol')
    assert.ok(global.equals(fix.globalRegistry()), 'resolved once')

    // A message built without a registry links that same dictionary.
    const root = fields.struct('row', [global.fieldByTag(55)], { nullable: false })
    assert.ok(new fix.FixMsg(root, { symbol: 'AAPL' }).registry.equals(global))

    // Once resolved, the default is fixed.
    assert.throws(() => fix.installGlobalRegistry(new fix.FixRegistry()), /already resolved/)
    ```

## Classifying a captured line

Deciding what one captured line is, what message type it declares and which way it moved is **transport rather than FIX** — every captured line has a shape whatever protocol it carried — so it needs no dictionary and lives with the types that name each answer.

| call | answers |
| --- | --- |
| `MimeType::infer_bytes` / `infer_text` | what the line is |
| `MsgType::infer_bytes` / `infer_text` | the message type it declares, borrowed |
| `MsgDirection::infer_bytes` / `infer_text` | which way it moved |
| `MsgDirection::split_bytes` / `split_text` | the same, with the marker taken off the line |

One shallow scan answers all three. It reads no message and allocates nothing, and every answer is a slice of the caller's bytes.

| line | answer |
| --- | --- |
| numeric pairs in a frame | `text/fix` |
| `#`-marked keys, or a `MSGTYPE=` key | `text/ullink` |
| a numeric frame also carrying symbolic keys | `text/fixul` |
| official `XmlData(213)` whose payload begins with XML | `text/fixml` |
| no frame, but `key=value` throughout | `text/key-value` |
| a document opening `<` or `{`/`[` | `application/xml`, `application/json` |
| anything else | `application/octet-stream` |

The scan locates `8=` first, then `35=`, then the first pair-shaped run. It holds one separator, stops at checksum tag 10, and reads no prefix, suffix, XML attribute or `#A=1` inside a value as a field. A frame beats a document, because an `XmlData` payload is part of a frame; a document beats the bare pair rules, because an attribute inside a tag is not a field.

=== "Rust"

    ```rust
    use yggdryl::types::{MsgDirection, MsgType};
    use yggdryl::MimeType;

    let line = b"sending >> 8=FIX.4.4|35=D|55=AAPL|10=001|";
    assert_eq!(MimeType::infer_bytes(line), MimeType::FIX);
    assert_eq!(MsgType::infer_bytes(line), Some(&b"D"[..]));
    assert_eq!(MsgDirection::infer_bytes(line), Some(MsgDirection::SENT));

    // No frame, but pairs throughout.
    assert_eq!(
        MimeType::infer_bytes(b"level=INFO worker=3 took=12ms"),
        MimeType::KEYVALUE
    );
    // A document is what it opens as.
    assert_eq!(MimeType::infer_bytes(b"<Order id='1'/>"), MimeType::XML);

    // Reading the verb takes it off the line, and takes nothing else.
    let (direction, body) = MsgDirection::split_bytes(line);
    assert_eq!(direction, Some(MsgDirection::SENT));
    assert_eq!(body, b">> 8=FIX.4.4|35=D|55=AAPL|10=001|");
    ```

=== "Python"

    ```python
    from yggdryl import MimeType

    line = "ACCOUNT=A1|MSGTYPE=D|SYMBOL=AAPL"
    assert MimeType.infer_text(line) == MimeType.ULLINK
    assert MimeType.infer_text_msgtype(line) == "D"
    assert MimeType.infer_text_direction("recv " + line) == "RECV"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MimeType } = require('yggdryl')

    const line = '8=FIX.4.4|35=D|11=ORDER-1|213=SYMBOL=AAPL|SIDE=1|10=000|'
    assert.ok(MimeType.inferText(line).equals(MimeType.FIXUL))
    assert.equal(MimeType.inferTextMsgtype(line), 'D')
    ```

A raw `MSGTYPE=` anywhere in the line wins over tag 35, because a bridge writes its own type in front of a frame it relays, and `U` plus an alphanumeric suffix routes to the canonical `UDF` root.

### A direction is the verb in front of the payload

The verb counts only where it stands before the message starts, so a `sent` inside a FIX `Text(58)`, a bridge value spelled `OUT=1`, or an XML payload's own wording never becomes a direction.

| direction | opens with |
| --- | --- |
| `SENT` | `send`, `sending`, `sent`; and `out`, `outbound`, `outgoing` |
| `RECV` | `receive`, `receiving`, `received`, `recv`; and `in`, `inbound`, `incoming` |

The bare `in` and `out` forms are **chosen** only where a bracket opens them and a delimiter closes them, because that is the one shape a marker has and none of the shapes the same letters have otherwise: `direct:out` is a route endpoint and `MCFID-IN-XPAR` is a session name. They still count against an opposite verb, which is what makes `sending in session 3` and `received out of order` answer nothing rather than a wrong answer.

A prefix carrying both verbs, and one carrying neither, both answer nothing.

## Edges

- `registry.get_field("5055:cme")` -> `None`; a string key is a name, and only `FixId::from_str` parses an identifier.
- `registry.get_field_by_tag(5055)` with no branch -> the deterministic best match, so a named branch's own tag resolves; an explicit branch never crosses.
- Two names whose seeded XXH64 digests collide -> a read rechecks the field behind the digest and misses; a mutation refuses loudly.
- `MsgType::infer_bytes` -> a borrowed slice of the input line, so the Rust byte path allocates nothing.
- A line the scan cannot place -> `application/octet-stream`; a checksum tag 10 stops the scan.
- `contains("44")` -> `false`; a tag query never consults names, and a name query never consults tags.
- A path -> the whole string as a name first, keeping a dotted name reachable; then the first segment here, the rest through `Field::get_field_by_path` exactly.
- An alternate tag equal to another field's canonical tag, or an alias equal to another's canonical name -> legal, and it never wins.
- The same key twice in the same tier of one branch -> conflict.
- `insert` of a field whose key another field holds in the same branch -> conflict naming both fields and the branch; `len` unchanged.
- `insert` of a field matching one stored field on both identity halves -> `Ok(Some(prior))`, a replacement, never a conflict.
- `update` with a different datatype -> typed error; the stored datatype stays.
- `add_fields` of a field with no `fix:tag` -> absence naming `fix:tag`; there is no identity to add or fold under.
- `add_fields` of the same tag in another branch -> added, never folded; the identity is the whole probe, and a tag alone is not.
- `add_fields` refusing on any field -> the dictionary is unchanged, `len` included; one copy of it is staged per call, not per field.
- `merge_with` or `add_cfb_file` refusing -> the branch records and the fields are staged together and adopted together, so neither arrives.
- `add_cfb_file` where the stem already is the branch name -> no alias is invented; a name is not an alias of itself.
- `FixField::from_cfb_file` with no branch, on a handle whose stem is not a branch -> refused, never folded into one; a `Buffer`'s URL is an identity rather than a location, so bytes in memory are named by the caller.
- `FixField::from_cfb_file` on a file naming one field twice -> the same refusal `FixRegistry::from_cfb` gives, because the vocabulary door builds the dictionary too and drops it; a tag declared twice *identically* is the one difference, arriving twice there and once here.
- A CBlock's `float` or `string` meeting a committed `float64` or `msgtype` -> the datatype refusal above; a CBlock says nothing about which tag is money or which is a MsgType.
- `remove` with a path -> never a match; it takes a tag, an identifier or a name, and a bare one means the standard branch.
- Primitive and nested fields share one identity space; a repeating group claiming a scalar's tag, name, alternate tag or alias -> the same conflict as between two scalars.
- `install_global` after `global()` has resolved -> typed conflict (`already resolved` in the bindings); the value every caller saw cannot change.
- `YGGDRYL_FIX_REGISTRY` set to a missing directory -> error from `global()`, where an absent `~/.config/fix` is the empty registry.
- A tag outside `[FixId::USER_TAG_MIN, FixId::USER_TAG_MAX)` in a named branch -> refused ([FIX](index.md)); inside it a vendor field is also reachable by its `FixId` or a branch-qualified name.
- `branch_named` with a spelling one branch declares as an alias and another holds as its canonical name -> the canonical one; the exact spelling is answered before any alias is tried.
- An alias equal to the branch's own name, or repeated -> refused; a spelling that already reaches a dictionary is not a second way to reach it.
- A field whose `fix:branch` holds an alias -> a different branch, not that one; an alias is resolved by `branch_named` and never stored on a field.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib fix::tests
    cargo test -p yggdryl --lib -- fix::tests::a_field_without_a_tag fix::tests::a_name_or_alias fix::tests::tier_order fix::tests::a_tag_query fix::tests::an_insert_conflict fix::tests::reinserting fix::tests::a_merge_follows fix::tests::a_rejected_merge fix::tests::removal_keeps fix::tests::specialized_and_generic fix::tests::iteration_follows fix::tests::iteration_and_the_cursor fix::tests::nestedness_routes fix::tests::an_omitted_branch_infers fix::tests::protocol_and_msgtype_inference fix::tests::a_nested_field_can_never fix::tests::two_branches_may_hold fix::tests::the_default_resolves
    cargo test -p yggdryl --test fix global
    cargo test -p yggdryl --lib -- fix::tests::a_lineage fix::tests::a_version_filters fix::tests::a_removed_entry fix::tests::two_entries
    cargo test -p yggdryl --lib -- fix::tests::a_branch_answers_to_its_aliases fix::tests::a_branch_alias_is_held fix::tests::merge_with_folds_the_fields
    cargo test -p yggdryl --lib -- fix::tests::a_code_set fix::tests::an_alias_shares fix::tests::an_ambiguous_spelling fix::tests::tier_three fix::tests::a_version_hides fix::tests::two_codes_may
    cargo test -p yggdryl --test allocations a_fix_lineage_read a_fix_code_lookup
    cargo bench -p yggdryl --bench fix -- fix/resolve
    cargo bench -p yggdryl --bench fix -- fix/lineage
    cargo test -p yggdryl --lib -- fix::tests::a_field_merge fix::tests::a_merge_keeps fix::tests::a_merge_of_disagreeing fix::tests::a_merge_adding_nothing
    cargo test -p yggdryl --lib -- fix::tests::add_fields_adds_what_is_absent fix::tests::add_fields_refuses_the_way
    cargo test -p yggdryl --test fix -- cfb::a_file_answers_its_vocabulary cfb::an_unnamed_file_takes_its_branch cfb::a_stem_that_is_not_a_branch cfb::a_cblock_vocabulary_folds cfb::folding_a_cblock_into_the_committed
    cargo test -p yggdryl --test fix -- cfb::a_cblock_reads_in_whole cfb::reading_a_cblock_in_whole_is_one_mutation
    cargo bench -p yggdryl --bench fix -- fix/mutate
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix
    python/.venv/bin/python -m pytest python/tests/fix -k "registry_resolves or explicit_branch or inference or registry_absence or registry_keys or registry_coerces or registry_iterates or seed_iterates or registry_insert or registry_mutation or install_global"
    python/.venv/bin/python -m pytest python/tests/fix -k "add_fields or cblock or merge_with or alias"
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

The timing runs report release builds on one Windows x86_64 host, so they are boundary-scale comparisons, not promises. The allocation test is the stronger hot-path assertion.

| assertion | scope |
| --- | --- |
| zero allocations in Rust | canonical tag, identifier, folded name, alias, miss, path, protocol inference, MsgType inference, iteration, every lineage read and every code lookup including a refused document |

### A merge

`fix/mutate`, folding two realistic definitions of tag 32 — each dated, coded, described and aliased.

| case | median |
| --- | --- |
| `merge_with`, the `fix:` half | 9.38 us |
| `update`, both halves plus reindexing | 15.0 us |

Both are dominated by re-rendering the merged lineage and code documents, which is what a generator pays once per tag rather than per read.

### A code set

`fix/codes`, over generated sets of 10, 60 and 300 members against the same document read as a `HashMap`. The map is shown twice on purpose: built once and read forever it wins, and that is the caller's option; built to answer one lookup it loses at every size, which is the case this scan exists for.

| case | 10 codes | 60 codes | 300 codes |
| --- | --- | --- | --- |
| `code` on the first value | 347 ns | 346 ns | 351 ns |
| `code` on the last value | 416 ns | 549 ns | 2.45 us |
| `code` on a miss | 295 ns | 568 ns | 2.20 us |
| `code_value_at`, last value | | | 868 ns |
| `code_by_name`, folded (tier 2) | 2.13 us | 10.4 us | 50.7 us |
| a `HashMap` hit, map already built | 35.7 ns | 37.4 ns | 35.5 ns |
| a `HashMap` built, then hit | 2.41 us | 15.3 us | 71.7 us |
| `set_codes` | | | 917 us |

Tier 1 addresses the record it wants rather than parsing every code it passes, which is why it barely moves with the set's size until the value it seeks is at the end. Tier 2 cannot: it must run the whole set, because two codes folding to one spelling have to answer nothing rather than the first. That is the cost the ambiguity rule buys, and it is still under building a map to answer one question.

### A version-filtered read

`fix/lineage`, over a field carrying three entries in a dictionary of 400 dated ones. Release build, one Windows x86_64 host, twenty samples; the host's own baselines moved by up to 1.8x between runs, so the ratios are the measurement and the absolute figures are a scale.

| case | median | against |
| --- | --- | --- |
| `get_field` on an undated key | 8.2 ns | the unfiltered read |
| `get_field_at` on a dated key | 519 ns | 63x the unfiltered read |
| `since` | 208 ns | one entry |
| `defined_at` | 458 ns | three entries |
| `name_at` | 494 ns | three entries |
| `dtype_at` | 964 ns | three entries plus building a `DataType` |
| one `fix:` property lookup | 103 ns | the floor every `fix:` accessor pays |
| one `Version` parse | 36 ns | paid per entry the scan reaches |
| `newest` / `versions` | 204 us / 218 us | every lineage in the dictionary, walked once |

Two facts the table is for. A single-entry read is about half fixed cost - one `fix:` metadata lookup plus one version parse - and the scan itself is roughly 110 ns per entry, so a lineage is cheap to hold and not free to ask. And `newest` and `versions` walk every field, so they are answered once and held, never per row.

Criterion takes ten samples with short warm-up and measurement windows in this phase.

```bash
cargo bench -p yggdryl --bench fix -- fix/resolve --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
cargo bench -p yggdryl --bench fix -- fix/lineage --warm-up-time 0.2 --measurement-time 0.5 --sample-size 20
python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
YGGDRYL_BENCH_ITERATIONS=5000 npm run --prefix node bench:fix
```

<!-- PHASE2_BENCHMARK_RESULTS -->
