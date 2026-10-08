# Identifier

`Identifier` is one name a source gave a thing - a value under a key, an `IdKey`: the source that gave it and the type of name it is - and `Identifiers` the sorted map from the key to the value, of which a market element states three: its security's (`securityids`), its own (`identifiers`) and its parties' (`partyids`). A source is an `IdSource` and a type an `IdType`: words folded to lower case, one vocabulary for an ISIN, a client order identifier and a trader, so all three are read, written, merged and digested alike. The key from the base source - what FIX's own fields state - is spelled as its type alone (`isin`), and it is the type's answer: a named source stating a type fills it ([The base key](#the-base-key)). A type may have parents - the values an order identifier held earlier in its chain - which a chain states under types of their own ([Parentage](#parentage)).

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::IdKey` (root `idkey.rs`), `yggdryl::Identifier` and `yggdryl::Identifiers` (root `identifier.rs`), the vocabularies `yggdryl::IdType` (`idtype.rs`) and `yggdryl::IdSource` (`idsource.rs`), `yggdryl::IdWord` the folded word beside a member; the national number an ISIN embeds is `securityid::embedded`; Python `yggdryl.Identifier`, `yggdryl.Identifiers`; JavaScript `Identifier`, `Identifiers` - a binding takes a key as its text, and `IdKey` is Rust-only |
| `IdKey` | a source and a type, `IdKey::new(src, kind)`; `IdKey::base(kind)` the type's base key; `src()`, `kind()`, `is_base()`, `with_kind(kind)`. It displays as `src:type`, a base key as its type alone - `isin`, `ullink:isin` - and orders by that spelling's bytes, so `cusip` < `derived:cusip` < `isin` < `oms:instrumentid` < `ullink:isin`; a key of two member words spells as a static string |
| Reading a key | `"text".parse::<IdKey>()` reads exactly: `src:type` is that source and that type, each folded, and a bare word is that type from the base source - `ISIN`, `ISIN_Number`, `base:isin`, `BASE:ISIN` and `fix:isin` are all `isin`, `marketorderid` is the type `marketorderid`. An empty half (`fix:`, `:isin`), a second `:` and a word no fold reads are refused, `expected an identifier key src:type or type`. Nothing is inferred: an inferred reading is [`Identifier::from_key`](#reading-a-name)'s |
| Words | a source and a type are trimmed and lower-cased, the `_`, `-`, space and `#` a spelling breaks them with dropped - `Executing Trader` and `executing_trader` are `executingtrader` - ASCII letters, digits and `.` only, at most 64 bytes (`IDENTIFIER_WORD_WIDTH`); anything else is refused. A value is never folded to lower case - an ISIN is still `US0378331005` |
| Members | a word the crate names is a member of the enum - `IdType::Isin`, `IdType::ClOrdId`, `IdSource::Proprietary` - held statically, and every other word is `Other(IdWord)`; `"text".parse::<IdType>()` folds and reads the member a spelling or an alias names (`isinnumber` and `isincode` are `isin`), `as_str()` is the folded word, `is_known()` tells a member from an `Other`, and a word equals a `&str`: `id.kind() == "isin"` |
| Value | trimmed text that states something, at most 64 bytes (`IDENTIFIER_VALUE_WIDTH`): empty or null-like (`null`, `none`, `n/a`) is no identifier and is refused; then [checked by its type](#per-type-value-checks) - the shape of a code, never whether its check digit closes, which is its [rank](#ranks) - and, under the `bic` and `legalentityidentifier` sources, by the code that source gives ([Under a source](#under-a-source)) |
| Named sources | `base` (`IdSource::Base`) where nothing names the source - what a FIX field or group states, the standard being the base, so the word `fix` reads as `base` and is never written; `derived` (`IdSource::Derived`) where the crate derived the value rather than read it - the CUSIP, SEDOL, WKN or Valor an ISIN embeds, what a ticker's shape names, a currency pair detected off a symbol, a code an [`IsinRegistry`](isin-registry.md) filled; any other source is the one a message names - a bridge namespace (`oms`, `ullink`), a `PartyIDSource(447)` (`proprietary`) or an `AcctIDSource(660)` (`bic`) |
| Display | `key=value` - `isin=US0378331005`, `ullink:isin=US0378331005`; a map `[a=b, ...]` in key order |
| Order, equality | by the key as it is spelled, then by value, so `a.b:c` sorts before `a:z`, which a (source, type) pair would not; equality and hash read the key and the value |
| `Identifier::new(key, value)` | takes an `IdKey` and checks the value's shape by its type - there is no second constructor for a security; `with_kind(kind)` is the same value under another type of the same source - a parent's own type, a type's parent |
| `Identifier::from_key(name, value)` | the identifier a name no key spells names, inferred ([Reading a name](#reading-a-name)): an explicit `src:type`, a whole security name, or the identifier name the name ends with under the source before it; `None` / `null` where the name names none, the value states nothing or the type refuses it |
| Readers | `key`, `src`, `kind` (the property `type` in Python and JavaScript), `value` |
| `Identifiers` | a sorted map from a key to its value, held as one vector in key order - an empty map holds no backing - kept by [the base rule](#the-base-key) and [the rank](#ranks): `insert` fills an absent key or one holding a lower-ranked value, `set` replaces, `remove(key)`, `merge(other, later)`, `clear`, `len`, `is_empty`, `iter`, `as_slice`; `carry`, `follow_parents` and `fill_parents` move identifiers along a chain ([Parentage](#parentage)); built from an iterator through `insert`, the first of a key and rank standing |
| Lookups | `get(kind)` the value of `kind`'s base key - the type's answer, one binary search; `get_from(key)` the value under exactly that key; `contains_kind(kind)` whether anything of the type is held; `of_kind(kind)` every key of the type, the base key included; `is_derived(kind)` whether the base key holds only a derivation |
| Bindings | Python `Identifier(key, value)`, `Identifier.from_key`, the readers as properties `key`, `src`, `type`, `value`; `Identifiers(ids=[])`, `Identifiers.from_dict(d)`, `into_dict()`, `get(type)`, `get_from(key)`, `contains_kind(type)`, `of_kind(type)`, `is_derived(type)`, iteration, `len`, `str`, equality, hash and pickle. JavaScript `new Identifier(key, value)`, `Identifier.fromKey`, getters `key`, `src`, `type`, `value`, `equals`, `compare`, `toString`; `new Identifiers(ids?)`, `Identifiers.fromObject(o)`, `intoObject()`, `get`, `getFrom(key)`, `containsKind`, `ofKind`, `isDerived`, `toArray`, `length`, `equals`, `toString`. A leaf takes an `Identifiers`, a `dict` / `Map` of key text to value, or a list of `Identifier` for `securityids`, `identifiers` and `partyids`, and answers an `Identifiers`; `IdKey`, the map verbs and the vocabulary enums are Rust-only |

## Use

=== "Rust"

    ```rust
    use yggdryl::{IdKey, IdSource, IdType, Identifier, Identifiers};

    // A key is a source and a type; the base source's is the type alone.
    let isin = Identifier::new(IdKey::base(IdType::Isin), " us0378331005 ")?;
    assert_eq!(isin.to_string(), "isin=US0378331005");
    assert_eq!(isin.key(), "isin");
    // A check digit that does not close is a rank, not a refusal; the shape is.
    assert_eq!(IdType::Isin.rank("US0378331006"), 1);
    assert!(Identifier::new(IdKey::base(IdType::Isin), "US037833100").is_err(), "eleven characters");

    // A key's text reads exactly; words fold, a value is trimmed and states something.
    let trader = Identifier::new("Proprietary:Executing Trader".parse()?, " T-1 ")?;
    assert_eq!((trader.src(), trader.kind(), trader.value()), (&IdSource::Proprietary, &IdType::ExecutingTrader, "T-1"));
    assert_eq!(trader.key(), "proprietary:executingtrader");
    assert_eq!("fix:ClOrdID".parse::<IdKey>()?, IdKey::base(IdType::ClOrdId), "the standard is the base");
    assert!(Identifier::new(IdKey::base(IdType::ClOrdId), "n/a").is_err());

    // A name no key spells is read for the identifier name it ends with.
    let listing = Identifier::from_key("OMS_InstrumentID", "dbi;CH0012214059_XSWX_CHF").expect("a key");
    assert_eq!(listing.key(), "oms:instrumentid");

    // One value per key, in key order; a named source fills its type's base key.
    let ids: Identifiers = [
        isin,
        Identifier::new("derived:cusip".parse()?, "037833100")?,
        listing,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.get(&IdType::InstrumentId), Some("dbi;CH0012214059_XSWX_CHF"));
    assert_eq!(ids.get_from(&"oms:instrumentid".parse()?), Some("dbi;CH0012214059_XSWX_CHF"));
    assert!(ids.is_derived(&IdType::Cusip));
    assert_eq!(
        ids.to_string(),
        "[cusip=037833100, derived:cusip=037833100, instrumentid=dbi;CH0012214059_XSWX_CHF, \
         isin=US0378331005, oms:instrumentid=dbi;CH0012214059_XSWX_CHF]"
    );
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Identifier, Identifiers

    # A key is a source and a type; the base source's is the type alone.
    isin = Identifier("isin", " us0378331005 ")
    assert str(isin) == "isin=US0378331005" and isin.key == "isin"
    # A check digit that does not close is a rank, not a refusal; the shape is.
    assert Identifier("isin", "US0378331006").value == "US0378331006"
    with pytest.raises(ValueError):
        Identifier("isin", "US037833100")

    # A key's text reads exactly; words fold, a value is trimmed and states something.
    trader = Identifier("Proprietary:Executing Trader", " T-1 ")
    assert (trader.src, trader.type, trader.value) == ("proprietary", "executingtrader", "T-1")
    assert Identifier("fix:ClOrdID", "C-1").key == "clordid", "the standard is the base"
    with pytest.raises(ValueError):
        Identifier("clordid", "n/a")

    # A name no key spells is read for the identifier name it ends with.
    listing = Identifier.from_key("OMS_InstrumentID", "dbi;CH0012214059_XSWX_CHF")
    assert listing is not None and listing.key == "oms:instrumentid"

    # One value per key, in key order; a named source fills its type's base key.
    ids = Identifiers([isin, Identifier("derived:cusip", "037833100"), listing])
    assert ids.get("instrumentid") == "dbi;CH0012214059_XSWX_CHF"
    assert ids.get_from("oms:instrumentid") == "dbi;CH0012214059_XSWX_CHF"
    assert ids.is_derived("cusip")
    assert ids.into_dict() == {
        "cusip": "037833100",
        "derived:cusip": "037833100",
        "instrumentid": "dbi;CH0012214059_XSWX_CHF",
        "isin": "US0378331005",
        "oms:instrumentid": "dbi;CH0012214059_XSWX_CHF",
    }
    assert Identifiers.from_dict(ids.into_dict()) == ids
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Identifiers } = require('yggdryl')

    // A key is a source and a type; the base source's is the type alone.
    const isin = new Identifier('isin', ' us0378331005 ')
    assert.equal(isin.toString(), 'isin=US0378331005')
    assert.equal(isin.key, 'isin')
    // A check digit that does not close is a rank, not a refusal; the shape is.
    assert.equal(new Identifier('isin', 'US0378331006').value, 'US0378331006')
    assert.throws(() => new Identifier('isin', 'US037833100'))

    // A key's text reads exactly; words fold, a value is trimmed and states something.
    const trader = new Identifier('Proprietary:Executing Trader', ' T-1 ')
    assert.deepEqual([trader.src, trader.type, trader.value], ['proprietary', 'executingtrader', 'T-1'])
    assert.equal(new Identifier('fix:ClOrdID', 'C-1').key, 'clordid', 'the standard is the base')
    assert.throws(() => new Identifier('clordid', 'n/a'))

    // A name no key spells is read for the identifier name it ends with.
    const listing = Identifier.fromKey('OMS_InstrumentID', 'dbi;CH0012214059_XSWX_CHF')
    assert.equal(listing.key, 'oms:instrumentid')

    // One value per key, in key order; a named source fills its type's base key.
    const ids = new Identifiers([isin, new Identifier('derived:cusip', '037833100'), listing])
    assert.equal(ids.get('instrumentid'), 'dbi;CH0012214059_XSWX_CHF')
    assert.equal(ids.getFrom('oms:instrumentid'), 'dbi;CH0012214059_XSWX_CHF')
    assert.ok(ids.isDerived('cusip'))
    assert.deepEqual(ids.intoObject(), {
      cusip: '037833100',
      'derived:cusip': '037833100',
      instrumentid: 'dbi;CH0012214059_XSWX_CHF',
      isin: 'US0378331005',
      'oms:instrumentid': 'dbi;CH0012214059_XSWX_CHF',
    })
    assert.ok(Identifiers.fromObject(ids.intoObject()).equals(ids))
    ```

## The base key

Every type a map holds has its base key, and its value is the type's answer: `get(kind)` reads it, a view's `isincode` reads it, `map['isin']` lifts it. One rule keeps it, owned by `Identifiers`, so `securityids`, `identifiers` and `partyids` - and every holder, setter, door and binding - keep it alike.

| A statement | What moves |
| --- | --- |
| a named source states a type | the base key is filled where it is empty or [ranks](#ranks) below: `ullink:isin=X` alone is also `isin=X` |
| anything but a derivation states a type | the derivation of it, `derived:T`, is taken back where it does not outrank the statement, the base key it filled leaving with it; one that outranks it stands - a named statement landing under its own key as evidence, a base one nowhere |
| a derivation, `derived:T` | it lands only where nothing of its type is held or the base key ranks below it, and fills the base key; replaced or removed, the base key follows it while that is all the base key holds |
| the base key itself | it moves only through its own key: `insert` fills it or replaces what ranks below, `set` replaces it, and `remove(&IdKey::base(T))` removes every key of the type |
| `merge(other, later)` | a union by key: where both hold a key with two values, the higher-ranked, and of one rank `other`'s when `later`, else this map's - a later placeholder never replaces a real value; a derivation of `other`'s lands only where this map holds nothing of its type or a base key ranking below it |
| a map read back - `from_scalar`, `from_dict`, `fromObject`, an Arrow row | its entries are read as they are, then closed: a type with no base key takes its highest-ranked named source's value, the first in key order among equals, unless its derivation outranks them all, else its derivation's; a map the crate wrote comes back unchanged |

Replacing or removing a named source of one rank leaves the base key as it was. The wire states base keys, so a bridge restating the wire's code under its own name (`ullink:isin` beside `isin`) is the common case, and following it would let replacing or removing the bridge's copy overwrite or erase the wire's code; `remove(&IdKey::base(T))` is how a caller drops a type.

=== "Rust"

    ```rust
    use yggdryl::{IdKey, IdType, Identifier, Identifiers};

    let id = |key: &str, value: &str| Identifier::new(key.parse().expect("a key"), value);
    let mut ids = Identifiers::new();
    assert!(ids.insert(id("ullink:isin", "US0378331005")?));
    assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"), "the source fills the base key");
    assert!(!ids.insert(id("isin", "CH0012214059")?), "insert fills only");

    // A derivation lands only where nothing of its type is held; a statement takes it back.
    assert!(ids.insert(id("derived:cusip", "037833100")?));
    assert!(ids.is_derived(&IdType::Cusip));
    assert!(ids.insert(id("oms:cusip", "037833100")?));
    assert!(!ids.is_derived(&IdType::Cusip) && ids.get_from(&"derived:cusip".parse()?).is_none());

    // Removing a named source leaves the base key; removing the base key removes the type.
    assert!(ids.remove(&"ullink:isin".parse()?).is_some());
    assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));
    assert!(ids.remove(&IdKey::base(IdType::Cusip)).is_some());
    assert!(!ids.contains_kind(&IdType::Cusip));
    assert_eq!(ids.to_string(), "[isin=US0378331005]");

    // A merge keeps this map's value unless the other is later.
    let other: Identifiers = [id("isin", "CH0012214059")?].into_iter().collect();
    let mut earlier = ids.clone();
    assert!(!earlier.merge(&other, false));
    assert!(ids.merge(&other, true));
    assert_eq!(ids.get(&IdType::Isin), Some("CH0012214059"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Identifiers

    # A named source fills its type's base key, the first statement standing.
    ids = Identifiers([Identifier("ullink:isin", "US0378331005"), Identifier("isin", "CH0012214059")])
    assert ids.get("isin") == "US0378331005"
    # A derivation lands only where nothing of its type is held; a statement takes it back.
    derived = Identifiers([Identifier("derived:cusip", "037833100")])
    assert derived.is_derived("cusip")
    stated = Identifiers([Identifier("derived:cusip", "037833100"), Identifier("oms:cusip", "037833100")])
    assert stated.into_dict() == {"cusip": "037833100", "oms:cusip": "037833100"}
    # A map read back is closed: a type with no base key takes its first named source's value.
    assert Identifiers.from_dict({"oms:isin": "CH0012214059", "ullink:isin": "US0378331005"}).get("isin") == "CH0012214059"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Identifiers } = require('yggdryl')

    // A named source fills its type's base key, the first statement standing.
    const ids = new Identifiers([new Identifier('ullink:isin', 'US0378331005'), new Identifier('isin', 'CH0012214059')])
    assert.equal(ids.get('isin'), 'US0378331005')
    // A derivation lands only where nothing of its type is held; a statement takes it back.
    assert.ok(new Identifiers([new Identifier('derived:cusip', '037833100')]).isDerived('cusip'))
    const stated = new Identifiers([new Identifier('derived:cusip', '037833100'), new Identifier('oms:cusip', '037833100')])
    assert.deepEqual(stated.intoObject(), { cusip: '037833100', 'oms:cusip': '037833100' })
    // A map read back is closed: a type with no base key takes its first named source's value.
    assert.equal(Identifiers.fromObject({ 'oms:isin': 'CH0012214059', 'ullink:isin': 'US0378331005' }).get('isin'), 'CH0012214059')
    ```

## Ranks

A value's rank is how real it is as a value of its type: `IdType::rank(value)` answers from zero to `IdType::max_rank()`, and `IdType::is_real(value)` whether it reaches it. A registered [code](../types/codes/index.md#rank) ranks by its own `CodeValue::rank` - an ISIN two where its check digit closes under a listed prefix, one for a typo under a listed prefix or a closing `ZZ` number, zero for a masked number such as `XX0000000001` or the number that states none, `XX0000000000`; a CUSIP, a SEDOL or a FIGI one where its check digit closes, an LEI or a DTI one where its check closes; a CFI code two where detailed and one where it only classifies; an ISO country one where ISO 3166 lists it; an ISO currency zero for `XXX` alone - and every other type one for any value it holds; a value the type refuses is zero. Under the `bic` and `legalentityidentifier` sources an identifier ranks by the lower of its type's rank and the code's own - a BIC one where ISO 3166 lists its country, an LEI one where its check digits close - so a map decides a restated `bic:executingfirm` by the BIC. A base key ranks from the statements of the same value - the highest rank among its type's nonbase keys, or the type's rank where there are none; a same-valued statement from a source without a registered code gives it the type's rank. So a base answer filled by a BIC or LEI typo is ranked by that named code source, and a listed-country BIC or closing LEI under the same named key replaces it whichever arrives first ([Under a source](#under-a-source)). A lower-case spelling ranks as the upper-case one it folds to, and nothing allocates.

| Where | Rule |
| --- | --- |
| `insert` | a value lands where nothing is held under its key, or where what is held ranks below it, whatever the order the two were stated in: a real number replaces a masked one or a typo, never the other way; two values of one rank keep the first |
| A derivation | lands only where nothing of its type is held or the base key ranks below it - a registry's real number over a masked one - and a statement takes it back only where it does not outrank the statement |
| `merge` | the higher-ranked value wins where two meet under one key; of one rank, `other`'s when `later` |
| `carry`, a read map's close | fill only what is not held - a key the follower lacks, a type's missing base key - choosing among candidates by the same rank |
| `set` | the explicit statement: replaces whatever rank is held |

So a real value replaces a placeholder wherever the two meet - a lifecycle, a merge of two statements, a registry fill, a FIX message's fields read in order - and a later placeholder never takes it back.

=== "Rust"

    ```rust
    use yggdryl::{IdKey, IdType, Identifier, Identifiers};

    let isin = |value: &str| Identifier::new(IdKey::base(IdType::Isin), value);
    assert_eq!((IdType::Isin.rank("US0378331005"), IdType::Isin.max_rank()), (2, 2));
    assert_eq!(IdType::Isin.rank("US0378331006"), 1, "a typo under a listed prefix");
    assert!(!IdType::Isin.is_real("XX0000000001"), "a masked number");

    let mut ids = Identifiers::new();
    assert!(ids.insert(isin("XX0000000001")?));
    assert!(ids.insert(isin("US0378331005")?), "a real number replaces a masked one");
    assert!(!ids.insert(isin("XX0000000001")?), "and is never replaced by it");
    assert!(!ids.insert(isin("US5949181045")?), "two real numbers keep the first");
    assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));

    // A later placeholder never replaces a real value, whatever leads.
    let masked: Identifiers = [isin("XX0000000001")?].into_iter().collect();
    assert!(!ids.merge(&masked, true));
    let mut placeholder = masked.clone();
    assert!(placeholder.merge(&ids, false));
    assert_eq!(placeholder.get(&IdType::Isin), Some("US0378331005"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Identifiers

    # A real number replaces a masked one whichever comes first, and two real
    # numbers keep the first.
    ids = Identifiers([Identifier("isin", "XX0000000001"), Identifier("isin", "US0378331005")])
    assert ids.get("isin") == "US0378331005"
    ids = Identifiers([Identifier("isin", "US0378331005"), Identifier("isin", "XX0000000001")])
    assert ids.get("isin") == "US0378331005"
    ids = Identifiers([Identifier("isin", "US0378331005"), Identifier("isin", "US5949181045")])
    assert ids.get("isin") == "US0378331005"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Identifiers } = require('yggdryl')

    // A real number replaces a masked one whichever comes first, and two real
    // numbers keep the first.
    const ids = (...values) => new Identifiers(values.map((value) => new Identifier('isin', value)))
    assert.equal(ids('XX0000000001', 'US0378331005').get('isin'), 'US0378331005')
    assert.equal(ids('US0378331005', 'XX0000000001').get('isin'), 'US0378331005')
    assert.equal(ids('US0378331005', 'US5949181045').get('isin'), 'US0378331005')
    ```

`rank`, `max_rank` and `is_real` are Rust-only.

## Reading a name

`Identifier::from_key` infers the identifier a name no key spells names, which is how a bridge's own spelling - `OMS_InstrumentID`, `firm.x.ParentOrderID` - becomes one ([where a FIX message reads them](#where-identifiers-come-from)); a key's own text is read exactly by `IdKey` ([Contract](#contract)). It tries three readings in order and takes the first that answers:

| Reading | Rule |
| --- | --- |
| Explicit | `src:type`, each half read as a word folds: `oms:clordid`, `firm.x:house code`, `fix:clordid` the base key `clordid` |
| A whole security name | a key a security type is spelled by - `ISINCode`, `security_cusip`, `#ISINCODE`, a leading `#` dropped ([`IdType::from_field_name`](#vocabularies)), or any spelling of a security type FIX gives no source code, `FISN`, `FinancialInstrumentShortName` and `CFI` as a bare `ISIN` is - is that type from the base source |
| The name it ends with | the key folds - lower case, no `_`, `-`, space or `#` - and the longest identifier name it ends with is the type: a spelling the crate reads a type by, an [alias](#vocabularies) included, that ends with `id`; `account`, `isin`, `cusip`, `sedol`, `figi`; or a security type's spelling ending with `code`, `symbol`, `number` or `ticker` - `riccode`, `isincode`, `bbgsymbol`, `bloombergticker` - so a bridge's `OMS_RICCODE` names its `ric`. A bare `ric` or `cfi` ends no key - `GENERIC` names nothing - and `ticker` alone is no type's spelling. A parentage word spelled right before it - `parent`, `orig`, `origin`, `original` - stays in the type. The source is the rest of the folded key with its `.` trimmed at both ends and kept inside, the base source where nothing is left or where it folds to a source the crate reserves - `base`, `fix`, `derived` - which names no namespace: `Derived_ISIN` and `FIX.ISIN` are `isin`, so a namespace spelled before the name never files a code under `derived`, while an explicit `src:type` keeps the source it spells, `derived:isin` included; the source and the type are each a word of at most 64 bytes, never the key they spell together |

A security type is refused where another instrument's word - `leg`, `underlying`, `contra`, `related`, `benchmark` - opens the key or ends what the folded key spells before the type, after any namespace, since it names that instrument's code: `UnderlyingISIN`, `OMS_UnderlyingISIN`, `FIX.LegISIN` and `firm.x.ContraCUSIP` name no security identifier. An operation identifier a leg or a counterparty states (`contraorderid`) is the element's own. A key ending with no identifier name - `transversalkey`, `ticker`, `symbol` - names none. What a derivative is written on is not lost: an [`IsinRegistry`](isin-registry.md#the-underlying) learns an underlying's ISIN as the row's `underlyingisin`. A structured product's category - `EUSIPACode`, `OMS_SSPACategory` - names no identifier either: the registry learns it as the row's [`eusipacode`](isin-registry.md#the-product-category).

| Key | Reads as |
| --- | --- |
| `firm.x.ParentOrderID` | `firm.x:parentorderid` |
| `OMS_InstrumentID` | `oms:instrumentid` |
| `OMSUserID` | `oms:userid` |
| `OMSDealerParentOrderID` | `omsdealer:parentorderid` |
| `marketorderid` | `market:orderid` |
| `OrderID` | `orderid` |
| `ISINCode` | `isin` |
| `OMS_RICCODE`, `ULLINK.ISINCODE`, `OMS_BloombergTicker` | `oms:ric`, `ullink:isin`, `oms:bloomberg` |
| `Derived_ISIN`, `FIX.ISIN` | `isin` |
| `FISN`, `FISNCode`, `OMS_FISNCODE` | `fisn`, `fisn`, `oms:fisn` |
| `fix:clordid` | `clordid` |
| `underlyingisin`, `OMS_UnderlyingISIN`, `FIX.LegISIN`, `transversalkey`, `OMS_RIC`, `OMS_FISN`, `TICKERCODE`, `EUSIPACode` | none |

=== "Rust"

    ```rust
    use yggdryl::Identifier;

    let read = |key: &str, value: &str| Identifier::from_key(key, value).map(|id| id.to_string());
    assert_eq!(read("firm.x.ParentOrderID", "P-1").as_deref(), Some("firm.x:parentorderid=P-1"));
    assert_eq!(read("OMS_InstrumentID", "dbi;X").as_deref(), Some("oms:instrumentid=dbi;X"));
    assert_eq!(read("OMSUserID", "U-1").as_deref(), Some("oms:userid=U-1"));
    assert_eq!(read("OMSDealerParentOrderID", "P-1").as_deref(), Some("omsdealer:parentorderid=P-1"));
    assert_eq!(read("marketorderid", "O-1").as_deref(), Some("market:orderid=O-1"));
    assert_eq!(read("OrderID", "O-1").as_deref(), Some("orderid=O-1"));
    assert_eq!(read("fix:clordid", "C-1").as_deref(), Some("clordid=C-1"));
    // A whole security name is that type from the base source; another instrument's is none.
    assert_eq!(read("ISINCode", "US0378331005").as_deref(), Some("isin=US0378331005"));
    assert_eq!(read("underlyingisin", "US0378331005"), None);
    // A security type's code spelling names it at the end of a key.
    assert_eq!(read("OMS_RICCODE", "AAPL.O").as_deref(), Some("oms:ric=AAPL.O"));
    // A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert_eq!(read("transversalkey", "K-1"), None);
    assert_eq!(read("OrderID", "n/a"), None);
    assert_eq!(read("ISINCode", "US037833100"), None);
    ```

=== "Python"

    ```python
    from yggdryl import Identifier


    def read(key: str, value: str) -> str | None:
        found = Identifier.from_key(key, value)
        return None if found is None else str(found)


    assert read("firm.x.ParentOrderID", "P-1") == "firm.x:parentorderid=P-1"
    assert read("OMS_InstrumentID", "dbi;X") == "oms:instrumentid=dbi;X"
    assert read("OMSUserID", "U-1") == "oms:userid=U-1"
    assert read("OMSDealerParentOrderID", "P-1") == "omsdealer:parentorderid=P-1"
    assert read("marketorderid", "O-1") == "market:orderid=O-1"
    assert read("OrderID", "O-1") == "orderid=O-1"
    assert read("fix:clordid", "C-1") == "clordid=C-1"
    # A whole security name is that type from the base source; another instrument's is none.
    assert read("ISINCode", "US0378331005") == "isin=US0378331005"
    assert read("underlyingisin", "US0378331005") is None
    # A security type's code spelling names it at the end of a key.
    assert read("OMS_RICCODE", "AAPL.O") == "oms:ric=AAPL.O"
    # A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert read("transversalkey", "K-1") is None
    assert read("OrderID", "n/a") is None
    assert read("ISINCode", "US037833100") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier } = require('yggdryl')

    const read = (key, value) => Identifier.fromKey(key, value)?.toString() ?? null

    assert.equal(read('firm.x.ParentOrderID', 'P-1'), 'firm.x:parentorderid=P-1')
    assert.equal(read('OMS_InstrumentID', 'dbi;X'), 'oms:instrumentid=dbi;X')
    assert.equal(read('OMSUserID', 'U-1'), 'oms:userid=U-1')
    assert.equal(read('OMSDealerParentOrderID', 'P-1'), 'omsdealer:parentorderid=P-1')
    assert.equal(read('marketorderid', 'O-1'), 'market:orderid=O-1')
    assert.equal(read('OrderID', 'O-1'), 'orderid=O-1')
    assert.equal(read('fix:clordid', 'C-1'), 'clordid=C-1')
    // A whole security name is that type from the base source; another instrument's is none.
    assert.equal(read('ISINCode', 'US0378331005'), 'isin=US0378331005')
    assert.equal(read('underlyingisin', 'US0378331005'), null)
    // A security type's code spelling names it at the end of a key.
    assert.equal(read('OMS_RICCODE', 'AAPL.O'), 'oms:ric=AAPL.O')
    // A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert.equal(read('transversalkey', 'K-1'), null)
    assert.equal(read('OrderID', 'n/a'), null)
    assert.equal(read('ISINCode', 'US037833100'), null)
    ```

## Vocabularies

`IdType` and `IdSource` are two enums over one fold: a member costs nothing to hold, compare or digest, and any other word is an `Other`. Each member also reads by the longer names FIX gives it (`legalentityidentifier` is `lei`, `bloombergsymbol` is `bloomberg`).

| `IdType` group | Members |
| --- | --- |
| Security types FIX's `SecurityIDSource(22)` names, with the code in parentheses | `cusip` (1), `sedol` (2), `quik` (3), `isin` (4), `ric` (5), `isoccy` (6), `isoctry` (7), `exchsymb` (8), `cta` (9), `bloomberg` (A), `wkn` (B), `dutch` (C), `valor` (D), `sicovam` (E), `belgian` (F), `common` (G), `clearinghouse` (H, also its full name `Clearing House / Clearing Organization`; `ClearingOrganization` alone is the party role `PartyRole(452)` `21`), `fpmlspec` (I), `opra` (J), `fpmlurl` (K), `loc` (L), `mktassigned` (M), `redentity` (N), `redpair` (P), `cftc` (Q), `isdacommodity` (R), `figi` (S), `lei` (T), `synthetic` (U), `fim` (V), `index` (W), `umtf` (X), `dti` (Y) |
| The crate's own security types | `forex` (a currency pair, no FIX code; also `ccypair`, `currencypair`), `cfi`, `fisn` (an ISO 18774 [financial instrument short name](../types/codes/fisn.md), no FIX code - FIX states it in `FinancialInstrumentShortName(2737)`), `instrumentid` (a venue's or a bridge's own instrument key) |
| Reference data | `elf` (an ISO 20275 [entity legal form](../types/codes/elf.md) - what kind of entity a party is - also `elfcode`, `entitylegalform`, `entitylegalformcode`): neither a security nor a party, so an operation's `identifiers` hold it |
| Operation identifiers | `orderid`, `clordid`, `origclordid` (`clordid`'s one parent), `secondaryorderid`, `secondaryclordid`, `secondaryexecid`, `secondaryquoteid`, `secondarytradeid`, `secondaryfirmtradeid`, `secondaryallocid`, `secondaryindividualallocid`, `execid`, `quoteid`, `quotereqid`, `mdreqid`, `trdmatchid`, `tradeid`, `tradereportid`, `tradereportrefid` (`tradereportid`'s one parent, FIX's `TradeReportRefID(572)`), `mdentryid`, `mdentryrefid` |
| Regulatory trade identifiers, by `RegulatoryTradeIDType(1906)` | `regtradeid` (0), `prevregtradeid` (1), `blockregtradeid` (2), `relatedregtradeid` (3), `clearedregtradeid` (4), `tvtic` (5), `reporttrackingnumber` (6) |
| Parties and accounts | `account`, `party` (a role nothing states), `userid`, and the `PartyRole(452)` roles `executingfirm`, `clientid`, `clearingfirm`, `investorid`, `enteringfirm`, `orderoriginationtrader`, `executingtrader`, `orderoriginationfirm`, `executingsystem`, `contrafirm`, `clearingorganization`, `exchange`, `customeraccount`, `enteringtrader`, `contratrader`, `positionaccount`, `orderentryoperatorid`, `executionvenue`, `deskid`, `investmentdecisionmaker`, `algorithm` |

| `IdSource` group | Members |
| --- | --- |
| Where the name came from | `base` nothing names it - a FIX field or group, the standard's own word `fix` reading as it; `derived` the crate derived it |
| `PartyIDSource(447)` and `AcctIDSource(660)` | `bic`, `generalidentifier`, `proprietary`, `isocountrycode`, `settlemententitylocation`, `mic`, `csdparticipant`, `taxid`, `legalentityidentifier`, `shortcodeidentifier`, `nationalidnaturalperson`, `sidcode`, `tfm`, `omgeo`, `dtcccode`, `spsaid` |

A security type also reads by the spellings a bridge writes its code under, each folded first - `BBG_Code`, `Bloomberg Ticker` and `ISIN_ID` are `bbgcode`, `bloombergticker` and `isinid`:

| Type | Also read as |
| --- | --- |
| `isin` | `isinnumber`, `isincode`, `isinid` |
| `cusip` | `cusipcode`, `cusipnumber`, `cusipid` |
| `sedol` | `sedolcode`, `sedolnumber`, `sedolid` |
| `ric` | `riccode`, `ricsymbol`, `reuterscode`, `reuterssymbol`, `reuters` |
| `bloomberg` | `bbgsymb`, `bloombergsymbol`, `bloombergcode`, `bbg`, `bbgcode`, `bbgsymbol`, `bloombergid`, `bloombergticker` |
| `figi` | `financialinstrumentglobalidentifier`, `figicode`, `figiid`, `openfigi` |
| `wkn` | `wertpapier`, `wkncode`, `wknnumber` |
| `valor` | `valoren`, `valorcode`, `valornumber`, `valorid`, `xswxvalor` (SIX's source spelling `X-SWX-VALOR`), `valorennummer`, `valorennumber` |
| `exchsymb` | `exchangesymbol`, `exchsymbol`, `sixsymbol`, `valorsymbol`, `valorensymbol` (SIX's symbol, a listing code on the market it is stated on) |
| `cta` | `consolidatedtapeassociation`, `ctasymbol`, `consolidatedtapeassociationsymbol` |
| `isoccy`, `isoctry` | `isocurrencycode`, `isocountrycode` |
| `common` | `commoncode` |
| `forex` | `forexcode`, `ccypair`, `currencypair` |
| `cfi` | `cficode` |
| `fisn` | `fisncode`, `financialinstrumentshortname` |
| `instrumentid` | `instrumentcode` |

Every one ending with `id`, `code`, `symbol`, `number` or `ticker` also names its type at the end of a longer key - `OMS_RICCODE` is `oms:ric` ([Reading a name](#reading-a-name)) - which is how a FIX message captures a bridge's own security keys ([Where identifiers come from](#where-identifiers-come-from)).

Any other source or type is held as a word - `venue`, `firm.x`, `oms`, `housecode` - and works everywhere a member does, but no check runs beyond the generic one.

Two predicates sort a type into the set it belongs to: `IdType::is_security()` is true for the types FIX's `SecurityIDSource(22)` names, `forex`, `cfi`, `fisn` and `instrumentid` - what a market's `securityids` hold - and `IdType::is_party()` for `account`, `party`, `userid` and the `PartyRole(452)` roles - what an operation's `partyids` hold. Every other type is the operation's own and belongs to `identifiers`.

=== "Rust"

    ```rust
    use yggdryl::{IdSource, IdType};

    // A spelling folds to the member it names, an alias included.
    assert_eq!("ISIN_Number".parse::<IdType>()?, IdType::Isin);
    assert_eq!("Executing Trader".parse::<IdType>()?, IdType::ExecutingTrader);
    assert_eq!("BASE".parse::<IdSource>()?, IdSource::Base);
    assert_eq!(IdType::ClOrdId.as_str(), "clordid");

    // Any other word is held folded, and is no member.
    let house = "House Code".parse::<IdType>()?;
    assert!(!house.is_known());
    assert_eq!(house.as_str(), "housecode");
    assert!(!"oms".parse::<IdSource>()?.is_known(), "a bridge is a word, not a member");

    // A `SecurityIDSource(22)` value is a code or a name; one no member
    // names is kept as stated, and a ticker is no type.
    assert_eq!(IdType::from_security_source("4")?, IdType::Isin);
    assert_eq!(IdType::Isin.fix_security_source(), Some('4'));
    assert_eq!(
        IdType::from_security_source("ISDA/FpML Product URL (URL in SecurityID)")?,
        IdType::FpmlUrl
    );
    assert_eq!(IdType::from_security_source("100")?.as_str(), "100");
    assert!(IdType::from_security_source("ticker").is_err());
    // A field's name states the type it holds.
    assert_eq!(IdType::from_field_name("#ISINCODE"), Some(IdType::Isin));
    assert_eq!(IdType::from_field_name("legisin"), None);

    // A type belongs to the set its kind names.
    assert!(IdType::Isin.is_security() && IdType::InstrumentId.is_security());
    assert!(IdType::ExecutingTrader.is_party() && IdType::Account.is_party());
    assert!(!IdType::ClOrdId.is_security() && !IdType::ClOrdId.is_party());
    // A short name is a security's; a legal form is neither a security's nor a party's.
    assert_eq!("FinancialInstrumentShortName".parse::<IdType>()?, IdType::Fisn);
    assert!(IdType::Fisn.is_security() && IdType::Fisn.fix_security_source().is_none());
    assert_eq!("EntityLegalForm".parse::<IdType>()?, IdType::Elf);
    assert!(!IdType::Elf.is_security() && !IdType::Elf.is_party());
    ```

## Per-type value checks

A value is held as its type stores it, and a value its type refuses is no identifier. A code is checked for its shape alone; how real it is - a check digit that closes, a listed prefix or country - is its [rank](#ranks), never a refusal.

| Type | Stored as |
| --- | --- |
| `isin`, `cusip`, `sedol`, `figi` | upper-cased, of the code's shape - twelve, nine, seven and twelve bytes, the check digit in its place - whether that digit closes the code being its rank |
| `lei` | upper-cased, of the [LEI](../types/codes/lei.md)'s shape - twenty bytes, eighteen letters or digits and two check digits - whether ISO 7064 MOD 97-10 closes it being its rank |
| `dti` | upper-cased, of the [DTI](../types/codes/dti.md)'s shape - nine bytes of its thirty-symbol alphabet, the first not `0` - whether hybrid MOD 31,30 closes it being its rank |
| `cfi` | upper-cased, six letters; how much it classifies is its rank |
| `fisn` | upper-cased, of the [FISN](../types/codes/fisn.md)'s shape - an issuer and a description either side of the first `/`, at most 35 printable ASCII bytes; every such value of one rank |
| `elf` | upper-cased, of the [ELF](../types/codes/elf.md)'s shape - four letters or digits; every such value of one rank |
| `ric` | one token of printable ASCII, its case kept, at most 32 bytes |
| `bloomberg` | printable ASCII, at most 32 bytes |
| `wkn` | six of `[0-9A-HJ-NP-Z]`, upper-cased |
| `valor` | one to nine digits without a leading zero |
| `forex` | one canonical pair - `eurusd` is `EUR/USD`; `EUR/EUR` is refused |
| `isoccy`, `isoctry` | upper-cased, of a currency's or a country's width; a country ISO 3166 lists ranks above one it does not, and the currency `XXX` below every other |
| every other security type FIX names - an FpML product URL, an index name among them | printable ASCII, at most 64 bytes |
| every other type | any text, at most 64 bytes |

`IdType::max_value_width` answers the bound of each - `fisn` 35, `elf` 4. `IdType::check_security` refuses the one word no security type is, `ticker`: the name a person knows an instrument by lives on `set_ticker`.

`IdType::from_security_source` reads a `SecurityIDSource(22)` or `SecurityAltIDSource(456)` value - the FIX 4 field `IDSource(22)` included - as its one-character code, case-sensitive (`4`, `K`), or as any name the code set writes, its spacing, punctuation and parenthesized remarks passed over: `ISIN number`, `Wertpapier`, `X-SWX-VALOR`, `Clearing House / Clearing Organization`, `ISDA/FpML Product Specification (XML in EncodedSecurityDesc <351>)`, `ISDA/FpML Product URL (URL in SecurityID)` and `Letter of Credit` are `isin`, `wkn`, `valor`, `clearinghouse`, `fpmlspec`, `fpmlurl` and `loc`. A source no member names is kept as it was stated, the word it folds to: a private code `100` is the type `100`, a letter FIX gives nothing (`Z`) the type `z`, a venue's `House Key` the type `housekey`; what no word holds - `House/Key`, a byte past ASCII - is refused, never reshaped, and so is a member naming another kind of identifier - `ClOrdID`, the party role `Exchange`. The reading is the crate's own, over the code set it ships; a dictionary whose `securityidsourcecodeset` was edited does not change it. A FIX message states `SecurityID(48)` under that type's base key and each `SecAltIDGrp(454)` occurrence likewise - except a source spelled `{NAMESPACE}INSTRUMENTID`, a venue's own instrument key, which is an `instrumentid` from that namespace, read as a key's source is read: folded, the dots at its ends dropped, a word of at most 64 bytes before the `INSTRUMENTID` it is spelled with, so `ULLINKINSTRUMENTID`, `ULLINK.INSTRUMENTID` and `Ullink Instrument ID` are all `ullink:instrumentid`, and a namespace folding to a source the crate reserves - `DERIVEDINSTRUMENTID`, `Base Instrument ID`, `FIX.INSTRUMENTID` - names no venue and is read as none, the base `instrumentid`, by the rule a [name's source](#reading-a-name) is read by, so neither field states a code under `derived`; a source or a value its type refuses states nothing and is an anomaly, the fields staying on the wire as sent. An [`IsinRegistry`](isin-registry.md) learns the types the crate names; a word of a venue's own is held by the message that states it.

### Under a source

Two sources are standards whose every value is a registered code: a value under `bic` - `PartyIDSource(447)` `B`, `AcctIDSource(660)` `1` - is an ISO 9362 [BIC](../types/codes/bic.md), and one under `legalentityidentifier` - `PartyIDSource(447)` `N` - an ISO 17442 [LEI](../types/codes/lei.md), whatever type of name it is: a party's role, the account, a word no member names. `Identifier::new`, and every door that builds an identifier through it, holds such a value to that code's shape beside its type's rule, upper-cased, and refuses another on its key: `invalid record value at bic:executingtrader: a value under the bic source is a BIC: expected eight or eleven characters, got "T-1"`. Every other source leaves a value to its type's rule alone, so `proprietary:executingtrader=T-1` stands.

The code's rank is read beside the type's, and the identifier ranks by the lower of the two: a BIC whose country ISO 3166 lists ranks above one whose country it does not, and an LEI whose check digits close above one whose digits do not. A base key ranks from the statements of the same value - the highest rank among its type's nonbase keys, or the type's rank where there are none. So a base answer filled by a BIC or LEI typo is ranked by that named code source, and a listed-country BIC or closing LEI under the same named key replaces it whichever arrives first. A same-valued statement from a source without a registered code gives the base key the type's rank. A value from another source makes no claim to be a code and ranks as its type, so of it and a real code - or of two real codes - the first stated stays the answer. A FIX party the rule refuses stays on the wire as an [anomaly](../fix/message.md#parties-and-regulatory-trade-identifiers).

=== "Rust"

    ```rust
    use yggdryl::{IdKey, IdSource, IdType, Identifier, Identifiers};

    let firm = |value: &str| Identifier::new(IdKey::new(IdSource::Bic, IdType::ExecutingFirm), value);
    // A value under `bic` is a BIC whatever its role: upper-cased, or refused on its key.
    assert_eq!(firm("deutdeff500")?.value(), "DEUTDEFF500");
    let refused = firm("T-1").unwrap_err().to_string();
    assert!(refused.contains("bic:executingfirm") && refused.contains("is a BIC"), "{refused}");
    // The same value from another source follows its type's rule alone.
    let house = IdKey::new(IdSource::Proprietary, IdType::ExecutingFirm);
    assert_eq!(Identifier::new(house, "T-1")?.value(), "T-1");
    // Under `legalentityidentifier`, an LEI.
    let client = IdKey::new(IdSource::LegalEntityIdentifier, IdType::ClientId);
    assert_eq!(Identifier::new(client.clone(), "hwupkr0mpou8fgxbt394")?.value(), "HWUPKR0MPOU8FGXBT394");
    assert!(Identifier::new(client, "CL").is_err());

    // A BIC of a listed country outranks one of an unlisted country under its
    // key and against the base answer, whichever value arrived first.
    let ids: Identifiers = [firm("ABCDXX22")?, firm("DEUTDEFF")?].into_iter().collect();
    assert_eq!(ids.get_from(&IdKey::new(IdSource::Bic, IdType::ExecutingFirm)), Some("DEUTDEFF"));
    assert_eq!(ids.get(&IdType::ExecutingFirm), Some("DEUTDEFF"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Identifiers

    # A value under `bic` is a BIC whatever its role: upper-cased, or refused on its key.
    assert Identifier("bic:executingfirm", "deutdeff500").value == "DEUTDEFF500"
    try:
        Identifier("bic:executingfirm", "T-1")
    except ValueError as error:
        assert "bic:executingfirm" in str(error) and "is a BIC" in str(error)
    else:
        raise AssertionError("a value under bic that is no BIC")
    # The same value from another source follows its type's rule alone.
    assert Identifier("proprietary:executingfirm", "T-1").value == "T-1"
    # Under `legalentityidentifier`, an LEI.
    assert Identifier("legalentityidentifier:clientid", "hwupkr0mpou8fgxbt394").value == "HWUPKR0MPOU8FGXBT394"

    # A BIC of a listed country outranks one of an unlisted country under its
    # key and against the base answer, whichever value arrived first.
    ids = Identifiers([Identifier("bic:executingfirm", "ABCDXX22"), Identifier("bic:executingfirm", "DEUTDEFF")])
    assert ids.get_from("bic:executingfirm") == "DEUTDEFF"
    assert ids.get("executingfirm") == "DEUTDEFF"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Identifiers } = require('yggdryl')

    // A value under `bic` is a BIC whatever its role: upper-cased, or refused on its key.
    assert.equal(new Identifier('bic:executingfirm', 'deutdeff500').value, 'DEUTDEFF500')
    assert.throws(() => new Identifier('bic:executingfirm', 'T-1'), /bic:executingfirm.*is a BIC/)
    // The same value from another source follows its type's rule alone.
    assert.equal(new Identifier('proprietary:executingfirm', 'T-1').value, 'T-1')
    // Under `legalentityidentifier`, an LEI.
    assert.equal(new Identifier('legalentityidentifier:clientid', 'hwupkr0mpou8fgxbt394').value, 'HWUPKR0MPOU8FGXBT394')

    // A BIC of a listed country outranks one of an unlisted country under its
    // key and against the base answer, whichever value arrived first.
    const ids = new Identifiers([new Identifier('bic:executingfirm', 'ABCDXX22'), new Identifier('bic:executingfirm', 'DEUTDEFF')])
    assert.equal(ids.getFrom('bic:executingfirm'), 'DEUTDEFF')
    assert.equal(ids.get('executingfirm'), 'DEUTDEFF')
    ```

## Parentage

A type can have parents: the values its identifier held earlier in a chain, each kept under a type of its own, so a lookup, a digest and a column read one the way they read any identifier. Parentage is a relation between types, never a part of a value, and only a chain identity has parents: a type naming one lifecycle chain by a value the chain states as its own.

| Key | Rule |
| --- | --- |
| `IdType::parents()` | the parent types of a chain identity, nearest first. `clordid`'s is `origclordid` alone - FIX's `OrigClOrdID(41)`, the previous client order identifier - and `tradereportid`'s `tradereportrefid` alone - FIX's `TradeReportRefID(572)`, the report a cancel or a replace refers to. Any other chain identity has `parent{type}` then `orig{type}`: `orderid`'s are `parentorderid`, the value it held before it last changed, and `origorderid`, the value its chain first stated. Every other type - a per-report reference such as `execid`, a security, a party, a parent type, any other word - has none, so parentage never nests |
| `IdType::is_chain_identity()` | whether a type names one lifecycle chain - an order, a quote, a trade or a trade report - by a value the chain states as its own: `orderid`, `clordid`, `secondaryorderid`, `secondaryclordid`, `quoteid`, `secondaryquoteid`, `tradeid`, `secondarytradeid`, `secondaryfirmtradeid` and `tradereportid`, and nothing else. A reference an event states about itself or about a request many chains answer - `execid`, `trdmatchid`, `tvtic`, `quotereqid`, `mdreqid`, `mdentryid`, the regulatory trade identifiers - names no chain, and neither does a security, a party, the account, a parent type or any other word (`venueorderid`, `reforderid`). Rust-only: a binding names types as text, and reads parentage through a registry's `parents_of` |
| `IdType::parent_of()` | the type a type is a parent of - the parent's own type - and its place among that type's parents: `origclordid` is `clordid`'s first, `tradereportrefid` `tradereportid`'s, `parentorderid` is `orderid`'s first and `origorderid` its second, `origtradeid` `tradeid`'s second. Only a chain identity's parent names one: `parentexecid` and `parentisin` are words of their own, and so are `parentclordid` and `parenttradereportid`, each base's one parent being spelled as FIX names it. A word spelled `origin` or `original` is no parent: `originalorderid` is a type of its own |
| `FIX:parents` | a dictionary states a field's own list on the field, the identifier type its `FIX:idmap` key or its name names: `ClOrdID(11)` states `["origclordid"]`; `FixRegistry::parents_of` and `parent_of` answer from the stated lists first, then from the type's ([FIX registry](../fix/registry.md#parents-of-an-identifier)). A dictionary may state a list on any field; a lifecycle reads the lists of chain identities alone |
| Following | `Identifiers::follow_parents(previous, parents_of, parent_of)`: for each type with parents the follower states - never a parent type - under a source whose previous statement the chain knows, it fills each parent the follower does not already state. A type that kept its value keeps each parent the previous statement held. A type that changed takes the previous value as its first parent, each middle parent from the previous one a step nearer, and, as the last of two or more, the chain's first value: the previous last parent, else the farthest previous parent stated, else the previous value |
| Filling the parent's own type | `Identifiers::fill_parents(parent_of)`: an element stating a parent but not the type it is a parent of takes that type from its nearest stated parent - `parentorderid` before `origorderid` - under the parent's source, base-source parents first |
| Carried | `Identifiers::carry(previous, carried)`: an identifier the chain holds and the follower does not is carried as it is where `carried` admits it - every security identifier and every party identifier, and every identifier but `mdentryrefid` ([Operation](operation.md#following-and-merging)); a FIX message the types its `FIX:idmap` follows and the parents of each, so a type and its lineage travel together ([FIX registry](../fix/registry.md#a-field-names-a-message-by-its-identifiers)) |
| The walk | a [lifecycle walk](event.md#lifecycle-walk) names a live chain by each chain identity its statements stated - the old value beside the new after a change - and by the chain's first value a parent names where it stands last among its base's parents (`origclordid`, `origorderid`, `origtradeid`, `tradereportrefid`), filed and read under that base, so a replace naming the order it replaced as its `origclordid` continues that order's chain. The previous-value slot `parentorderid` names no chain: the walk writes it from a value the chain already holds, and a bridge spells a hierarchy parent by it. A name has one live holder, the first chain of its kind and side that stated it, until that chain ends; every finalize runs `fill_parents` |
| Digest | a parent is an identifier like any other: its source, type and value feed the element's digest |

An order identifier chain `A`, `B`, `C`, `D` ends with `parentorderid` `C` and `origorderid` `A`; a client order identifier chain `C-1` to `C-4` ends with `origclordid` `C-3`.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Operation, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, orderid: &str, clordid: &str| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        order.insert_identifier(Identifier::new(IdKey::base(IdType::OrderId), orderid)?)?;
        order.insert_identifier(Identifier::new(IdKey::base(IdType::ClOrdId), clordid)?)?;
        order.finalize();
        Ok(order)
    };
    // What an order holds under each type, `-` where it holds none.
    let held = |order: &OrderEvent, kinds: [&str; 4]| -> [String; 4] {
        kinds.map(|kind| {
            let kind: IdType = kind.parse().expect("a type");
            order.get_identifiers().get(&kind).unwrap_or("-").to_owned()
        })
    };
    let kinds = ["parentorderid", "origorderid", "clordid", "origclordid"];

    // A first statement has no parent.
    let placed = order(T, "A", "C-1")?;
    assert_eq!(held(&placed, kinds), ["-", "-", "C-1", "-"]);
    // Each change names the value before it and, where there are two parents,
    // the chain's first.
    let second = order(T + 1, "B", "C-2")?.with_previous(&placed).expect("a later event follows");
    assert_eq!(held(&second, kinds), ["A", "A", "C-2", "C-1"]);
    let third = order(T + 2, "C", "C-3")?.with_previous(&second).expect("a later event follows");
    assert_eq!(held(&third, kinds), ["B", "A", "C-3", "C-2"]);
    let fourth = order(T + 3, "D", "C-4")?.with_previous(&third).expect("a later event follows");
    assert_eq!(held(&fourth, kinds), ["C", "A", "C-4", "C-3"]);
    // Stating the same values again keeps the parents of the step before.
    let acked = order(T + 4, "D", "C-4")?.with_previous(&fourth).expect("a later event follows");
    assert_eq!(held(&acked, kinds), ["C", "A", "C-4", "C-3"]);
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    T = 1_700_000_000_000_000_000
    KINDS = ("parentorderid", "origorderid", "clordid", "origclordid")


    def order(unix: int, orderid: str, clordid: str) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode="O-1001", identifiers=[
            Identifier("orderid", orderid),
            Identifier("clordid", clordid),
        ])


    def held(event: graph.OrderEvent) -> tuple[str, ...]:
        # What an order holds under each type, "-" where it holds none.
        return tuple(event.identifiers.get(kind) or "-" for kind in KINDS)


    # A first statement has no parent.
    placed = order(T, "A", "C-1")
    assert held(placed) == ("-", "-", "C-1", "-")
    # Each change names the value before it and, where there are two parents,
    # the chain's first.
    second = order(T + 1, "B", "C-2").with_previous(placed)
    assert second is not None and held(second) == ("A", "A", "C-2", "C-1")
    third = order(T + 2, "C", "C-3").with_previous(second)
    assert third is not None and held(third) == ("B", "A", "C-3", "C-2")
    fourth = order(T + 3, "D", "C-4").with_previous(third)
    assert fourth is not None and held(fourth) == ("C", "A", "C-4", "C-3")
    # Stating the same values again keeps the parents of the step before.
    acked = order(T + 4, "D", "C-4").with_previous(fourth)
    assert acked is not None and held(acked) == ("C", "A", "C-4", "C-3")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const KINDS = ['parentorderid', 'origorderid', 'clordid', 'origclordid']
    const order = (unix, orderid, clordid) => new graph.OrderEvent(unix, {
      crosscode: 'O-1001',
      identifiers: [new Identifier('orderid', orderid), new Identifier('clordid', clordid)],
    })
    // What an order holds under each type, '-' where it holds none.
    const held = (event) => KINDS.map((kind) => event.identifiers.get(kind) ?? '-')

    // A first statement has no parent.
    const placed = order(T, 'A', 'C-1')
    assert.deepEqual(held(placed), ['-', '-', 'C-1', '-'])
    // Each change names the value before it and, where there are two parents,
    // the chain's first.
    const second = order(T + 1n, 'B', 'C-2').withPrevious(placed)
    assert.deepEqual(held(second), ['A', 'A', 'C-2', 'C-1'])
    const third = order(T + 2n, 'C', 'C-3').withPrevious(second)
    assert.deepEqual(held(third), ['B', 'A', 'C-3', 'C-2'])
    const fourth = order(T + 3n, 'D', 'C-4').withPrevious(third)
    assert.deepEqual(held(fourth), ['C', 'A', 'C-4', 'C-3'])
    // Stating the same values again keeps the parents of the step before.
    const acked = order(T + 4n, 'D', 'C-4').withPrevious(fourth)
    assert.deepEqual(held(acked), ['C', 'A', 'C-4', 'C-3'])
    ```

An element that states where it came from but not what it is now is what it came from: finalizing fills the base from the nearest parent it states.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Operation, OrderEvent};
    use yggdryl::{IdType, Identifier, Identifiers};

    // A replacement stating only its parents is the order of its nearest one.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.insert_identifier(Identifier::new("origorderid".parse()?, "A")?)?;
    order.insert_identifier(Identifier::new("parentorderid".parse()?, "C")?)?;
    order.finalize();
    assert_eq!(order.get_identifiers().get(&IdType::OrderId), Some("C"));

    // The verbs are Rust-only: the maps they move are the ones a leaf holds.
    let id = |key: &str, value: &str| Identifier::new(key.parse().expect("a key"), value);
    let mut chain: Identifiers = [id("orderid", "A")?].into_iter().collect();
    for value in ["B", "C", "D"] {
        let mut next: Identifiers = [id("orderid", value)?].into_iter().collect();
        next.follow_parents(&chain, IdType::parents, IdType::parent_of);
        chain = next;
    }
    assert_eq!(chain.get(&"parentorderid".parse()?), Some("C"));
    assert_eq!(chain.get(&"origorderid".parse()?), Some("A"));
    let mut orphan: Identifiers = [id("origorderid", "A")?, id("parentorderid", "C")?].into_iter().collect();
    assert!(orphan.fill_parents(IdType::parent_of));
    assert_eq!(orphan.get(&IdType::OrderId), Some("C"), "the nearest parent");

    // Parents belong to chain identities alone.
    assert!(IdType::ClOrdId.is_chain_identity() && !IdType::ExecId.is_chain_identity());
    assert!(IdType::ExecId.parents().is_empty(), "a per-report reference");
    assert_eq!(IdType::TradeReportId.parents().as_ref(), [IdType::TradeReportRefId]);
    assert_eq!("parentclordid".parse::<IdType>()?.parent_of(), None, "a word of its own");
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    # A replacement stating only its parents is the order of its nearest one.
    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        identifiers=[Identifier("origorderid", "A"), Identifier("parentorderid", "C")],
    )
    assert order.identifiers.get("orderid") == "C"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    // A replacement stating only its parents is the order of its nearest one.
    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      identifiers: [new Identifier('origorderid', 'A'), new Identifier('parentorderid', 'C')],
    })
    assert.equal(order.identifiers.get('orderid'), 'C')
    ```

## Where identifiers come from

| Holder | Rule |
| --- | --- |
| A graph leaf | what a caller states: the `insert_`/`set_`/`remove_` verbs of [`Market`](market.md#security-identifiers) (`insert_securityid`), [`Operation`](operation.md#identifiers) (`insert_identifier`) and [party ids](operation.md#party-identifiers) (`insert_partyid`), or the `securityids`, `identifiers` and `partyids` facts a binding builds a leaf from; finalizing derives the national code a stated ISIN embeds, from `derived`, and fills a type from its parents ([Parentage](#parentage)); an [`IsinRegistry`](isin-registry.md) fills what a lifecycle learned, from `derived` |
| A FIX message | logical facts read off its fields at every settle, the wire kept as sent and never written back ([FIX](../fix/message.md#the-identifier-maps)): an identifier its field's `FIX:idmap` entry names is the base key of its type (`clordid`, `orderid`), a regulatory trade identifier by its `RegulatoryTradeIDType(1906)` too; a security identifier is `SecurityID(48)` under its `SecurityIDSource(22)`'s type - a `{NAMESPACE}INSTRUMENTID` source an `instrumentid` from that namespace - each `SecAltIDGrp(454)` occurrence, `FinancialInstrumentShortName(2737)` under the base `fisn` key, and the codes an ISIN embeds from `derived`. The first value stated under a key fills it, in reading order - the fields, the groups, then the unmapped entries - a later value that [outranks](#ranks) it replaces it, recorded as an anomaly naming what it replaced, and a later different value of no higher rank is dropped as a `dropped_identifier` anomaly, staying on the wire; a caller's write is the message's word and moves no field |
| A FIX entry no dictionary maps | each `metadata` key and each top-level untagged scalar of the message, and the keys its message type declares under `FIX:identifiers`, is read as [`Identifier::from_key`](#reading-a-name) reads a name: a security type is a `securityids` identifier (a value its type refuses is an anomaly), a party type a `partyids` one, any other type an `identifiers` one (a value either type refuses is no identifier and no anomaly) - `OMS_InstrumentID` is `oms:instrumentid` in `securityids`, `OMS_RICCODE` `oms:ric` and `ULLINK.ISINCODE` `ullink:isin` there too - every key whose folded name ends with a security type's spelling - `OMS_UserID` `oms:userid` in `partyids`, `firm.x.ParentOrderID` `firm.x:parentorderid` in `identifiers`, each filling its type's base key where nothing states it, and a whole security name such as `#ISINCODE` the base key itself. A captured entry leaves the fixed row's `metadata` cell and rides `fixentries` under `0:<key>`, the key as it arrived; it stays on the wire as it arrived |
| A FIX party | each `Parties(453)` or `RootParties(1116)` occurrence: its `PartyID(448)` typed by its `PartyRole(452)` code's name folded (`ExecutingFirm` is `executingfirm`, a code the set names nothing for `partyrole{code}`, no role `party`), from its `PartyIDSource(447)` code's name folded (`D` is `proprietary`, `C` `generalidentifier`; a spelling the set resolves nothing for is its own spelling where it is a word, `MyVenue` `myvenue`, and a bare code the set names nothing for - one character, or digits - `partyidsource{code}`, `W` `partyidsourcew`; none the base source), so `proprietary:executingtrader` also fills `executingtrader`; `Account(1)` is a party typed `account` from its `AcctIDSource(660)` code's name, by the same rule (`1` is `bic`, a code the set names nothing for `acctidsource{code}`). A party under `bic` or `legalentityidentifier` is held to that code's shape ([Under a source](#under-a-source)), one of another shape an anomaly that stays on the wire. A second party of one role and source stays on the wire, no anomaly |
| A leaf a FIX message becomes | the message's sets, a book entry's or a trade side's own party ids leading, plus each unmapped scalar whose key names an identifier - a dictionary-tagged child only by the identifiers its message's type declares (`RefOrderID(1080)`'s `reforderid`), an untagged key by the crate's identifier names too - typed by `Identifier::from_key` and lifted into the set its type belongs to where that set holds its key free or with the same value; otherwise it stays in the leaf's metadata ([What a leaf's metadata holds](../fix/message.md#what-a-leafs-metadata-holds)) |

## Arrow

| Key | Rule |
| --- | --- |
| `Identifiers::dtype()` | a sorted map `map<entries: struct<key: utf8 not null, value: utf8 not null>, keys_sorted = true>`: the key is its `IdKey`'s text - `src:type`, the type alone for the base source - and the value the identifier's; one datatype for the `securityids`, `identifiers` and `partyids` columns of every generated row ([Row schemas](schemas.md)) - a market-data row's cell holding the base keys alone, its other keys [side information](market-data.md#side-information) in `metadata` under the map's name, `securityids.ullink:isin` |
| Writing | `into_scalar` lays the map out as a `Scalar::SortedMap` of the key text and the value, in key order; a key of two member words is a static string, so a row writes no key text it has to build; a column is null where the map is empty |
| Reading | `from_scalar` reads a `Map` or a `SortedMap` in any order, or a sequence of such maps - their union - each key read exactly as `IdKey` reads one, then closes the map ([The base key](#the-base-key)); it refuses, located on the key (`$['fix:']`, `$[1]['account']`), a key that reads as none, a value that states nothing or that its type refuses, two spellings of one key with two values (`isin` and `BASE:ISIN`), and a key or a value that is not text, a struct included. The Arrow readers of the [`marketdata` row](schemas.md#the-marketdata-row) and of the [fixed FIX row](schemas.md#the-fix-row) read and refuse the same way, located on the column (`$.identifiers['fix:']`), caching each key text they read - `FixMsg::from_row` refuses the row and `FixCodec::messages` leaves it out with a warning |
| A binding | an `Identifier` crosses the scalar boundary as the one-entry map of its key, an `Identifiers` as its sorted map; Python `from_dict`/`into_dict` and JavaScript `fromObject`/`intoObject` are the map as a `dict` or a plain object of key text to value |
| A lift | a [view](market-data.md#views) reaches one value by its key with the path grammar's map segment: `identifiers['clordid'] as clordid`, and a source's statement where the row files it, under its map's name, `metadata['securityids.ullink:isin'] as ullinkisin` - the key as stored, lower case; the `isincode` column carries a market row's ISIN |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::{Field, IdKey, IdType, Identifier, Identifiers, Scalar, Serie};

    let mut ids = Identifiers::new();
    assert!(ids.insert(Identifier::new(IdKey::base(IdType::Isin), "US0378331005")?));
    assert!(ids.insert(Identifier::new("oms:clordid".parse()?, "C-2")?));
    assert!(!ids.insert(Identifier::new("oms:clordid".parse()?, "C-9")?), "fill only");

    // A map lays out as a sorted map of key text to value and reads back whole.
    let field = Arc::new(Field::new("identifiers", Identifiers::dtype(), true));
    let column = Serie::from_scalars(field, [ids.into_scalar()])?;
    assert_eq!(Identifiers::from_scalar(&column.scalar(0)?)?, ids);

    // A key that reads as none is refused on that key.
    let unread = Scalar::from_mapping([(Scalar::from("fix:"), Scalar::from("X"))])?;
    assert!(Identifiers::from_scalar(&unread).unwrap_err().to_string().contains("$['fix:']"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        securityids=[Identifier("isin", "US0378331005")],
    )
    table = graph.MarketData.arrow_reader([order]).read_all()
    securityids = table.schema.field("securityids").type
    assert str(securityids.key_type) == "string" and str(securityids.item_type) == "string"
    # A map from the key's text to its value, in key order: the stated ISIN
    # and the CUSIP it derives, each under its base key alone; the derivation
    # itself is side information, filed in the metadata cell.
    assert table.column("securityids").to_pylist()[0] == [
        ("cusip", "037833100"),
        ("isin", "US0378331005"),
    ]
    assert table.column("metadata").to_pylist()[0] == [("securityids.derived:cusip", "037833100")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      securityids: [new Identifier('isin', 'US0378331005')],
    })
    const table = graph.MarketData.arrowReader([order]).intoTable()
    const entries = table.schema.fields.find((field) => field.name === 'securityids').type.children[0].type
    assert.deepEqual(entries.children.map((child) => child.name), ['key', 'value'])
    // Read back, the derivation the metadata cell filed rejoins the map of
    // its type: the leaf is the leaf it was.
    const [read] = graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order]))
    assert.equal(read.intoLeaf().securityids.toString(), '[cusip=037833100, derived:cusip=037833100, isin=US0378331005]')
    ```

## Edges

- A word is folded, never guessed: `ISIN` and `isin` are one type, `IS/IN` is refused, and `IdType::from_security_source` - not `Identifier::new`, which takes the type already read - is what reads FIX's code `4` or the word `ISINNumber` as `isin`.
- A key's text is read exactly, a name is inferred: `"marketorderid".parse::<IdKey>()` is the base type `marketorderid`, `Identifier::from_key("marketorderid", ..)` is `market:orderid`. A map read from text or Arrow never infers, so what the crate wrote reads back as itself.
- A named source and the base key of its type stand side by side - `ullink:isin` and `isin` - and `get` answers the base key whatever sorts around it; `get_from` names the one wanted.
- A stale base key is by design: replacing or removing a named source leaves the base key it filled, because the wire's own code is indistinguishable from a bridge's copy of it. `remove(&IdKey::base(kind))` removes the type.
- `insert` keeps a held value of the same rank or higher, and a statement of a type takes back a derivation it is not outranked by: a market element's [`insert_securityid`](market.md#security-identifiers) is `Identifiers::insert` after the security check.
- A value past 64 bytes is no identifier: a leaf a FIX message becomes keeps such a metadata scalar in its metadata rather than lifting it.
- A name is inferred, never trusted: `Identifier::from_key` reads the identifier name a name ends with and nothing else, so a bridge's `transversalkey` is no identifier and stays metadata.
- Folding is intake only: nothing writes an upper-case word back, and a `FIX:idmap` document is stricter than intake - its `key` must be the folded word (`orderid`) and an upper-case one is refused ([FIX registry](../fix/registry.md#a-field-names-a-message-by-its-identifiers)).

## Performance

`graph/identifier`: an `IdKey` spelled and read, a sixteen-entry `Identifiers` map - eight securities under a bridge's source, each filling its base key - crossing a `Scalar` and built entry by entry, and 4,096 orders, each stating three securities, two identifiers and a party, written to and read from `marketdata` batches. One containerized x86_64 Linux run: Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.97.0, release profile with thin LTO. Criterion medians.

| Case | Median | What it does |
| --- | --- | --- |
| `idkey/spelled_known` | 13.9 ns | `SmolStr::from(&key)` for a member pair, `derived:cusip`: the static text, no allocation |
| `idkey/spelled_other` | 57.8 ns | the same for a bridge's own word, `omsbridge:instrumentid` |
| `idkey/read_known` | 171 ns | `"derived:cusip".parse::<IdKey>()` |
| `idkey/read_other` | 298 ns | `"omsbridge:instrumentid".parse::<IdKey>()` |
| `identifiers/into_scalar_16` | 803 ns | the map as its sorted `map<utf8, utf8>` value |
| `identifiers/from_scalar_16` | 7.12 µs | the map read back from that value, every key read and every value checked by its type |
| `identifiers/insert_sourced_16` | 2.86 µs | eight sourced securities inserted into an empty map, sixteen entries with their base keys |
| `identifiers/remove_derived` | 427 ns | one derived identifier removed from a seventeen-entry map |
| `marketdata/ids_write_4096` | 12.8 ms (321 K rows/s) | the orders laid out as `marketdata` batches |
| `marketdata/ids_read_4096` | 39.4 ms (104 K rows/s) | those batches read back into orders |

```bash
cargo bench -p yggdryl --bench graph -- 'graph/identifier'
```

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- idkey identifier idtype idsource
    cargo test -p yggdryl --test graph -- operation
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_identifier.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/identifier.test.js node/tests/graph/operation.test.js
    ```
