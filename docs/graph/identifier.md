# Identifier

`Identifier` is one name a source gave a thing - a source, a type and a value, unique by its key `src:type` - and `Identifiers` the sorted map from that key to the identifier, of which a market element states three: its security's (`securityids`), its own (`identifiers`) and its parties' (`partyids`). A source is an `IdSource` and a type an `IdType`: words folded to lower case, one vocabulary for an ISIN, a client order identifier and a trader, so all three are read, written, merged and digested alike. A type may have parents - the values an order identifier held earlier in its chain - which a chain states under types of their own ([Parentage](#parentage)).

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::Identifier` and `yggdryl::Identifiers` (root `identifier.rs`), the vocabularies `yggdryl::IdType` (`idtype.rs`) and `yggdryl::IdSource` (`idsource.rs`), `yggdryl::IdWord` the folded word beside a member; the national number an ISIN embeds is `securityid::embedded`; Python `yggdryl.Identifier`, `yggdryl.Identifiers`; JavaScript `Identifier`, `Identifiers` - a binding takes a source and a type as the text it folds |
| Unique key | `src:type`, `Identifier::key()` - one value per source and type; `src` and `type` are the first two columns of the row `struct<src, type, value>` |
| Words | a source and a type are trimmed and lower-cased, the `_`, `-`, space and `#` a spelling breaks them with dropped - `Executing Trader` and `executing_trader` are `executingtrader` - ASCII letters, digits and `.` only, at most 64 bytes (`IDENTIFIER_KEY_WIDTH`); anything else is refused. A value is never folded to lower case - an ISIN is still `US0378331005` |
| Members | a word the crate names is a member of the enum - `IdType::Isin`, `IdType::ClOrdId`, `IdSource::Fix` - held statically, and every other word is `Other(IdWord)`; `"text".parse::<IdType>()` folds and reads the member a spelling or an alias names (`isinnumber` is `isin`), `as_str()` is the folded word, `is_known()` tells a member from an `Other`, and a word equals a `&str`: `id.kind() == "isin"` |
| Value | trimmed text that states something, at most 64 bytes (`IDENTIFIER_VALUE_WIDTH`): empty or null-like (`null`, `none`, `n/a`) is no identifier and is refused; then [checked by its type](#per-type-value-checks) |
| Named sources | `base` (`IdSource::Base`) where nothing names the source - the base an identifier stands on; `derived` (`IdSource::Derived`) where the crate derived the value rather than read it - the CUSIP, SEDOL, WKN or Valor an ISIN embeds, what a ticker's shape names, a currency pair detected off a symbol, a code a lifecycle learned; `fix` (`IdSource::Fix`) where a FIX field or group states it |
| Display | `src:type=value` - `base:isin=US0378331005`; a map `[a:b=c, ...]` in key order |
| Order, equality | by the key as it is spelled - the bytes of `src:type` - then by value, so `a.b:c` sorts before `a:z`, which a (source, type) pair would not; equality and hash read all three |
| `Identifier::new(src, kind, value)` | takes an `IdSource` and an `IdType`, source first, and validates the value by its type - there is no second constructor for a security; `with_kind(kind)` is the same value under another type of the same source - a parent's base, a base's parent |
| `Identifier::key()` | the unique key as text, `fix:clordid`: what an `Identifiers` map is keyed by in its [Arrow layout](#arrow) |
| `Identifier::from_key(key, value)` | the identifier a key names, inferred ([Reading a key](#reading-a-key)): an explicit `src:type`, a whole security name, or the identifier name the key ends with under the source before it; `None` / `null` where the key names none, the value states nothing or the type refuses it |
| Readers | `src`, `kind` (the `type` column; the property `type` in Python and JavaScript), `value`, `key`; `is_of(src, kind)` compares the key |
| `Identifiers` | a sorted map: one identifier per key `src:type`, held as one vector in key order - an empty map holds no backing; `insert` fills an absent key only, `set` replaces, `remove(src, kind)`, `remove_kind(kind)`, `merge(other)` (fills), `clear`, `len`, `is_empty`, `iter`, `as_slice`; `carry`, `follow_parents` and `fill_parents` move identifiers along a chain ([Parentage](#parentage)); built from an iterator, the first of a key stands |
| Lookups | `get(kind)` the value of the identifier of `kind` the wire stated (`fix`), else the first another named source stated in key order, else the one stated under no source (`base`), else the derived one; `get_identifier(kind)` that identifier; `get_from(src, kind)` that source's alone; `contains_kind`, `of_kind` (one per source); every argument folds |
| Bindings | Python `Identifier(src, type, value)`, `Identifier.from_key`, the readers as properties `src`, `type`, `value`, `key`, `is_of(src, type)`; `Identifiers(ids=[])` with `get(type)`, `get_identifier(type)`, `get_from(src, type)`, `contains_kind(type)`, `of_kind(type)`, iteration, `len`, `str`, equality, hash and pickle. JavaScript `new Identifier(src, type, value)`, `Identifier.fromKey`, getters `src`, `type`, `value`, `key`, `isOf(src, type)`, `equals`, `compare`, `toString`; `new Identifiers(ids?)` with `get`, `getIdentifier`, `getFrom(src, type)`, `containsKind`, `ofKind`, `toArray`, `length`, `equals`, `toString`. A leaf takes a list of `Identifier` or an `Identifiers` for `securityids`, `identifiers` and `partyids`, and answers an `Identifiers`; the map verbs and the vocabulary enums are Rust-only |

## Use

=== "Rust"

    ```rust
    use yggdryl::{IdSource, IdType, Identifier, Identifiers};

    // A value is checked by its type: an ISIN closes on its check digit.
    let isin = Identifier::new(IdSource::Base, IdType::Isin, " us0378331005 ")?;
    assert_eq!(isin.to_string(), "base:isin=US0378331005");
    assert_eq!(isin.key(), "base:isin");
    assert!(Identifier::new(IdSource::Base, IdType::Isin, "US0378331006").is_err(), "a check digit that does not close");

    // Words fold to lower case; a value is trimmed and states something.
    let trader = Identifier::new(
        "Proprietary".parse::<IdSource>()?,
        "Executing Trader".parse::<IdType>()?,
        " T-1 ",
    )?;
    assert_eq!((trader.src(), trader.kind(), trader.value()), (&IdSource::Proprietary, &IdType::ExecutingTrader, "T-1"));
    assert_eq!(trader.kind(), "executingtrader");
    assert!(trader.is_of(&IdSource::Proprietary, &IdType::ExecutingTrader));
    assert!(Identifier::new(IdSource::Fix, IdType::ClOrdId, "n/a").is_err());

    // A key names its source and its type: here the identifier name it ends with.
    let listing = Identifier::from_key("OMS_InstrumentID", "dbi;CH0012214059_XSWX_CHF").expect("a key");
    assert_eq!((listing.src().as_str(), listing.kind()), ("oms", &IdType::InstrumentId));
    assert_eq!(listing.key(), "oms:instrumentid");

    // One value per source and type, in key order; a stated source answers
    // before a derived one.
    let ids: Identifiers = [
        isin,
        Identifier::new(IdSource::Derived, IdType::Cusip, "037833100")?,
        Identifier::new(IdSource::Fix, IdType::Cusip, "037833100")?,
        listing,
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(ids.get_identifier(&IdType::Cusip).map(Identifier::src), Some(&IdSource::Fix));
    let oms: IdSource = "oms".parse()?;
    assert_eq!(ids.get_from(&oms, &IdType::InstrumentId), Some("dbi;CH0012214059_XSWX_CHF"));
    assert_eq!(
        ids.to_string(),
        "[base:isin=US0378331005, derived:cusip=037833100, fix:cusip=037833100, oms:instrumentid=dbi;CH0012214059_XSWX_CHF]"
    );
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Identifier, Identifiers

    # A value is checked by its type: an ISIN closes on its check digit.
    isin = Identifier("base", "isin", " us0378331005 ")
    assert str(isin) == "base:isin=US0378331005"
    assert isin.key == "base:isin"
    with pytest.raises(ValueError):
        Identifier("base", "isin", "US0378331006")

    # Words fold to lower case; a value is trimmed and states something.
    trader = Identifier("Proprietary", "Executing Trader", " T-1 ")
    assert (trader.src, trader.type, trader.value) == ("proprietary", "executingtrader", "T-1")
    assert trader.is_of("proprietary", "executing_trader")
    with pytest.raises(ValueError):
        Identifier("fix", "clordid", "n/a")

    # A key names its source and its type: here the identifier name it ends with.
    listing = Identifier.from_key("OMS_InstrumentID", "dbi;CH0012214059_XSWX_CHF")
    assert listing is not None and (listing.src, listing.type) == ("oms", "instrumentid")
    assert listing.key == "oms:instrumentid"

    # One value per source and type, in key order; a stated source answers
    # before a derived one.
    ids = Identifiers([
        isin,
        Identifier("derived", "cusip", "037833100"),
        Identifier("fix", "cusip", "037833100"),
        listing,
    ])
    assert len(ids) == 4
    cusip = ids.get_identifier("cusip")
    assert cusip is not None and cusip.src == "fix"
    assert ids.get_from("oms", "instrument_id") == "dbi;CH0012214059_XSWX_CHF"
    assert [str(id) for id in ids] == [
        "base:isin=US0378331005",
        "derived:cusip=037833100",
        "fix:cusip=037833100",
        "oms:instrumentid=dbi;CH0012214059_XSWX_CHF",
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Identifiers } = require('yggdryl')

    // A value is checked by its type: an ISIN closes on its check digit.
    const isin = new Identifier('base', 'isin', ' us0378331005 ')
    assert.equal(isin.toString(), 'base:isin=US0378331005')
    assert.equal(isin.key, 'base:isin')
    assert.throws(() => new Identifier('base', 'isin', 'US0378331006'))

    // Words fold to lower case; a value is trimmed and states something.
    const trader = new Identifier('Proprietary', 'Executing Trader', ' T-1 ')
    assert.deepEqual([trader.src, trader.type, trader.value], ['proprietary', 'executingtrader', 'T-1'])
    assert.equal(trader.isOf('proprietary', 'executing_trader'), true)
    assert.throws(() => new Identifier('fix', 'clordid', 'n/a'))

    // A key names its source and its type: here the identifier name it ends with.
    const listing = Identifier.fromKey('OMS_InstrumentID', 'dbi;CH0012214059_XSWX_CHF')
    assert.deepEqual([listing.src, listing.type], ['oms', 'instrumentid'])
    assert.equal(listing.key, 'oms:instrumentid')

    // One value per source and type, in key order; a stated source answers
    // before a derived one.
    const ids = new Identifiers([
      isin,
      new Identifier('derived', 'cusip', '037833100'),
      new Identifier('fix', 'cusip', '037833100'),
      listing,
    ])
    assert.equal(ids.length, 4)
    assert.equal(ids.getIdentifier('cusip').src, 'fix')
    assert.equal(ids.getFrom('oms', 'instrument_id'), 'dbi;CH0012214059_XSWX_CHF')
    assert.equal(
      ids.toString(),
      '[base:isin=US0378331005, derived:cusip=037833100, fix:cusip=037833100, oms:instrumentid=dbi;CH0012214059_XSWX_CHF]',
    )
    ```

## Reading a key

`Identifier::from_key` infers the identifier a key names, which is how a bridge's own spelling - `OMS_InstrumentID`, `firm.x.ParentOrderID` - becomes one ([where a FIX message reads them](#where-identifiers-come-from)). It tries three readings in order and takes the first that answers:

| Reading | Rule |
| --- | --- |
| Explicit | `src:type`, each half read as a word folds: `fix:clordid`, `firm.x:house code` |
| A whole security name | a key a security type is spelled by - `ISINCode`, `security_cusip`, `#ISINCODE`, a leading `#` dropped - is that type from `base` ([`IdType::from_field_name`](#vocabularies)) |
| The name it ends with | the key folds - lower case, no `_`, `-`, space or `#` - and the longest identifier name it ends with is the type: a type the crate names whose spelling ends with `id`, or `account`, `isin`, `cusip`, `sedol`, `figi`. A parentage word spelled right before it - `parent`, `orig`, `origin`, `original` - stays in the type. The source is the rest of the folded key with its `.` trimmed at both ends and kept inside, `base` where nothing is left |

A security type is refused where the key starts with another instrument's prefix - `leg`, `underlying`, `contra`, `related`, `benchmark` - since it names that instrument's code; an operation identifier a leg or a counterparty states (`contraorderid`) is the element's own. A key ending with no identifier name - `transversalkey`, `ticker`, `symbol` - names none.

| Key | Reads as |
| --- | --- |
| `firm.x.ParentOrderID` | `firm.x:parentorderid` |
| `OMS_InstrumentID` | `oms:instrumentid` |
| `OMSUserID` | `oms:userid` |
| `OMSDealerParentOrderID` | `omsdealer:parentorderid` |
| `marketorderid` | `market:orderid` |
| `OrderID` | `base:orderid` |
| `ISINCode` | `base:isin` |
| `fix:clordid` | `fix:clordid` |
| `underlyingisin`, `transversalkey` | none |

=== "Rust"

    ```rust
    use yggdryl::Identifier;

    let read = |key: &str, value: &str| Identifier::from_key(key, value).map(|id| id.to_string());
    assert_eq!(read("firm.x.ParentOrderID", "P-1").as_deref(), Some("firm.x:parentorderid=P-1"));
    assert_eq!(read("OMS_InstrumentID", "dbi;X").as_deref(), Some("oms:instrumentid=dbi;X"));
    assert_eq!(read("OMSUserID", "U-1").as_deref(), Some("oms:userid=U-1"));
    assert_eq!(read("OMSDealerParentOrderID", "P-1").as_deref(), Some("omsdealer:parentorderid=P-1"));
    assert_eq!(read("marketorderid", "O-1").as_deref(), Some("market:orderid=O-1"));
    assert_eq!(read("OrderID", "O-1").as_deref(), Some("base:orderid=O-1"));
    assert_eq!(read("fix:clordid", "C-1").as_deref(), Some("fix:clordid=C-1"));
    // A whole security name is that type from base; another instrument's is none.
    assert_eq!(read("ISINCode", "US0378331005").as_deref(), Some("base:isin=US0378331005"));
    assert_eq!(read("underlyingisin", "US0378331005"), None);
    // A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert_eq!(read("transversalkey", "K-1"), None);
    assert_eq!(read("OrderID", "n/a"), None);
    assert_eq!(read("ISINCode", "US0378331006"), None);
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
    assert read("OrderID", "O-1") == "base:orderid=O-1"
    assert read("fix:clordid", "C-1") == "fix:clordid=C-1"
    # A whole security name is that type from base; another instrument's is none.
    assert read("ISINCode", "US0378331005") == "base:isin=US0378331005"
    assert read("underlyingisin", "US0378331005") is None
    # A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert read("transversalkey", "K-1") is None
    assert read("OrderID", "n/a") is None
    assert read("ISINCode", "US0378331006") is None
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
    assert.equal(read('OrderID', 'O-1'), 'base:orderid=O-1')
    assert.equal(read('fix:clordid', 'C-1'), 'fix:clordid=C-1')
    // A whole security name is that type from base; another instrument's is none.
    assert.equal(read('ISINCode', 'US0378331005'), 'base:isin=US0378331005')
    assert.equal(read('underlyingisin', 'US0378331005'), null)
    // A key naming no identifier, a value stating nothing and a value its type refuses are none.
    assert.equal(read('transversalkey', 'K-1'), null)
    assert.equal(read('OrderID', 'n/a'), null)
    assert.equal(read('ISINCode', 'US0378331006'), null)
    ```

## Vocabularies

`IdType` and `IdSource` are two enums over one fold: a member costs nothing to hold, compare or digest, and any other word is an `Other`. Each member also reads by the longer names FIX gives it (`legalentityidentifier` is `lei`, `bloombergsymbol` is `bloomberg`).

| `IdType` group | Members |
| --- | --- |
| Security types FIX's `SecurityIDSource(22)` names, with the code in parentheses | `cusip` (1), `sedol` (2), `quik` (3), `isin` (4), `ric` (5), `isoccy` (6), `isoctry` (7), `exchsymb` (8), `cta` (9), `bloomberg` (A), `wkn` (B), `dutch` (C), `valor` (D), `sicovam` (E), `belgian` (F), `common` (G), `clearinghouse` (H), `fpmlspec` (I), `opra` (J), `fpmlurl` (K), `loc` (L), `mktassigned` (M), `redentity` (N), `redpair` (P), `cftc` (Q), `isdacommodity` (R), `figi` (S), `lei` (T), `synthetic` (U), `fim` (V), `index` (W), `umtf` (X), `dti` (Y) |
| The crate's own security types | `forex` (a currency pair, no FIX code; also `ccypair`, `currencypair`), `cfi`, `instrumentid` (a venue's or a bridge's own instrument key) |
| Operation identifiers | `orderid`, `clordid`, `origclordid` (`clordid`'s one parent, also spelled `parentclordid`), `secondaryorderid`, `secondaryclordid`, `secondaryexecid`, `secondaryquoteid`, `secondarytradeid`, `secondaryfirmtradeid`, `secondaryallocid`, `secondaryindividualallocid`, `execid`, `quoteid`, `quotereqid`, `mdreqid`, `trdmatchid`, `tradeid`, `tradereportid`, `mdentryid`, `mdentryrefid` |
| Regulatory trade identifiers, by `RegulatoryTradeIDType(1906)` | `regtradeid` (0), `prevregtradeid` (1), `blockregtradeid` (2), `relatedregtradeid` (3), `clearedregtradeid` (4), `tvtic` (5), `reporttrackingnumber` (6) |
| Parties and accounts | `account`, `party` (a role nothing states), `userid`, and the `PartyRole(452)` roles `executingfirm`, `clientid`, `clearingfirm`, `investorid`, `enteringfirm`, `orderoriginationtrader`, `executingtrader`, `orderoriginationfirm`, `executingsystem`, `contrafirm`, `exchange`, `customeraccount`, `enteringtrader`, `contratrader`, `positionaccount`, `orderentryoperatorid`, `executionvenue`, `deskid`, `investmentdecisionmaker`, `algorithm` |

| `IdSource` group | Members |
| --- | --- |
| Where the name came from | `base` nothing names it, `derived` the crate derived it, `fix` a FIX field or group states it |
| `PartyIDSource(447)` and `AcctIDSource(660)` | `bic`, `generalidentifier`, `proprietary`, `isocountrycode`, `settlemententitylocation`, `mic`, `csdparticipant`, `taxid`, `legalentityidentifier`, `shortcodeidentifier`, `nationalidnaturalperson`, `sidcode`, `tfm`, `omgeo`, `dtcccode`, `spsaid` |

Any other source or type is held as a word - `venue`, `firm.x`, `oms`, `housecode` - and works everywhere a member does, but no check runs beyond the generic one.

Two predicates sort a type into the set it belongs to: `IdType::is_security()` is true for the types FIX's `SecurityIDSource(22)` names, `forex`, `cfi` and `instrumentid` - what a market's `securityids` hold - and `IdType::is_party()` for `account`, `party`, `userid` and the `PartyRole(452)` roles - what an operation's `partyids` hold. Every other type is the operation's own and belongs to `identifiers`.

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
    ```

## Per-type value checks

A value is held as its type stores it, and a value its type refuses is no identifier.

| Type | Stored as |
| --- | --- |
| `isin`, `cusip`, `sedol`, `figi` | upper-cased, and the code must close on its own check digit - twelve, nine, seven and twelve bytes |
| `cfi` | upper-cased, a six-letter classification that parses |
| `ric` | one token of printable ASCII, its case kept, at most 32 bytes |
| `bloomberg` | printable ASCII, at most 32 bytes |
| `wkn` | six of `[0-9A-HJ-NP-Z]`, upper-cased |
| `valor` | one to nine digits without a leading zero |
| `forex` | one canonical pair - `eurusd` is `EUR/USD`; `EUR/EUR` is refused |
| `isoccy`, `isoctry` | upper-cased, a currency ISO 4217 names or a country ISO 3166 names |
| every other security type FIX names - an FpML product URL, an index name among them | printable ASCII, at most 64 bytes |
| every other type | any text, at most 64 bytes |

`IdType::max_value_width` answers the bound of each. `IdType::check_security` refuses the one word no security type is, `ticker`: the name a person knows an instrument by lives on `set_ticker`.

`IdType::from_security_source` reads a `SecurityIDSource(22)` or `SecurityAltIDSource(456)` value - the FIX 4 field `IDSource(22)` included - as its one-character code, case-sensitive (`4`, `K`), or as any name the code set writes, its spacing, punctuation and parenthesized remarks passed over: `ISIN number`, `Wertpapier`, `Clearing House / Clearing Organization`, `ISDA/FpML Product Specification (XML in EncodedSecurityDesc <351>)`, `ISDA/FpML Product URL (URL in SecurityID)` and `Letter of Credit` are `isin`, `wkn`, `clearinghouse`, `fpmlspec`, `fpmlurl` and `loc`. A source no member names is kept as it was stated, the word it folds to: a private code `100` is the type `100`, a letter FIX gives nothing (`Z`) the type `z`, a venue's `House Key` the type `housekey`; what no word holds - `House/Key`, a byte past ASCII - is refused, never reshaped, and so is a member naming another kind of identifier - `ClOrdID`, the party role `Exchange`. The reading is the crate's own, over the code set it ships; a dictionary whose `securityidsourcecodeset` was edited does not change it. A FIX message states `SecurityID(48)` under that type and each `SecAltIDGrp(454)` occurrence likewise, from `fix` - except a source spelled `{NAMESPACE}INSTRUMENTID`, a venue's own instrument key, which is an `instrumentid` from that namespace, read as a key's source is read: folded, the dots at its ends dropped, so `ULLINKINSTRUMENTID`, `ULLINK.INSTRUMENTID` and `Ullink Instrument ID` are all `ullink:instrumentid`; a source or a value its type refuses states nothing and is an anomaly, the fields staying on the wire as sent. A lifecycle's learned associations keep the types the crate names; a word of a venue's own is held by the message that states it.

## Parentage

A type can have parents: the values its identifier held earlier in a chain, each kept under a type of its own, so a lookup, a digest and a column read one the way they read any identifier. Parentage is a relation between types, never a part of a value.

| Key | Rule |
| --- | --- |
| `IdType::parents()` | the parent types, nearest first. `clordid`'s is `origclordid` alone - FIX's `OrigClOrdID(41)`, the previous client order identifier. Any other type the crate names, or a word ending in `id`, has `parent{type}` then `orig{type}`: `orderid`'s are `parentorderid`, the value it held before it last changed, and `origorderid`, the value its chain first stated. A parent type has none, so parentage never nests |
| `IdType::parent_of()` | the base a type is a parent of and its place among the base's parents: `origclordid` is `clordid`'s first, `parentorderid` is `orderid`'s first and `origorderid` its second, `origtradeid` `tradeid`'s second. A word spelled `origin` or `original` is no parent: `originalorderid` is a type of its own |
| `FIX:parents` | a dictionary states a field's own list on the field, the identifier type its `FIX:idmap` key or its name names: `ClOrdID(11)` states `["origclordid"]`; `FixRegistry::parents_of` and `parent_of` answer from the stated lists first, then from the name rule ([FIX registry](../fix/registry.md#parents-of-an-identifier)) |
| Following | `Identifiers::follow_parents(previous, parents_of, parent_of)`: for each base the follower states - never a parent type - under a source whose previous statement the chain knows, it fills each parent the follower does not already state. A base that kept its value keeps each parent the previous statement held. A base that changed takes the previous value as its first parent, each middle parent from the previous one a step nearer, and, as the last of two or more, the chain's first value: the previous last parent, else the farthest previous parent stated, else the previous value |
| Filling the base | `Identifiers::fill_parents(parent_of)`: an element stating a parent but not its base takes the base from its nearest stated parent - `parentorderid` before `origorderid` - under the parent's source |
| Carried | `Identifiers::carry(previous, carried)`: an identifier the chain holds and the follower does not is carried as it is where `carried` admits it - every security identifier and every party identifier, and every identifier but `mdentryrefid` ([Operation](operation.md#following-and-merging)); a FIX message the types its `FIX:idmap` follows and the parents of each, so a base and its lineage travel together ([FIX registry](../fix/registry.md#a-field-names-a-message-by-its-identifiers)) |
| The walk | a lifecycle walk joins an element to a live chain by a parent identifier's value under its base too, so an element naming its order as `parentorderid` continues the chain of that `orderid` ([Lifecycle walk](event.md#lifecycle-walk)); every finalize runs `fill_parents` |
| Digest | a parent is an identifier like any other: its source, type and value feed the element's digest |

An order identifier chain `A`, `B`, `C`, `D` ends with `parentorderid` `C` and `origorderid` `A`; a client order identifier chain `C-1` to `C-4` ends with `origclordid` `C-3`.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Operation, OrderEvent};
    use yggdryl::{IdSource, IdType, Identifier};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, orderid: &str, clordid: &str| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        order.insert_identifier(Identifier::new(IdSource::Fix, IdType::OrderId, orderid)?)?;
        order.insert_identifier(Identifier::new(IdSource::Fix, IdType::ClOrdId, clordid)?)?;
        order.finalize();
        Ok(order)
    };
    // What an order holds under each type, `-` where it holds none.
    let held = |order: &OrderEvent, kinds: [&str; 4]| -> [String; 4] {
        kinds.map(|kind| {
            let kind: IdType = kind.parse().expect("a type");
            order
                .get_identifiers()
                .get_from(&IdSource::Fix, &kind)
                .unwrap_or("-")
                .to_owned()
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
            Identifier("fix", "orderid", orderid),
            Identifier("fix", "clordid", clordid),
        ])


    def held(event: graph.OrderEvent) -> tuple[str, ...]:
        # What an order holds under each type, "-" where it holds none.
        return tuple(event.identifiers.get_from("fix", kind) or "-" for kind in KINDS)


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
      identifiers: [new Identifier('fix', 'orderid', orderid), new Identifier('fix', 'clordid', clordid)],
    })
    // What an order holds under each type, '-' where it holds none.
    const held = (event) => KINDS.map((kind) => event.identifiers.getFrom('fix', kind) ?? '-')

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
    use yggdryl::{IdSource, IdType, Identifier, Identifiers};

    // A replacement stating only its parents is the order of its nearest one.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.insert_identifier(Identifier::new(IdSource::Fix, "origorderid".parse()?, "A")?)?;
    order.insert_identifier(Identifier::new(IdSource::Fix, "parentorderid".parse()?, "C")?)?;
    order.finalize();
    assert_eq!(order.get_identifiers().get_from(&IdSource::Fix, &IdType::OrderId), Some("C"));

    // The verbs are Rust-only: the maps they move are the ones a leaf holds.
    let id = |kind: &str, value: &str| Identifier::new(IdSource::Fix, kind.parse().expect("a type"), value);
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
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    # A replacement stating only its parents is the order of its nearest one.
    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        identifiers=[Identifier("fix", "origorderid", "A"), Identifier("fix", "parentorderid", "C")],
    )
    assert order.identifiers.get_from("fix", "orderid") == "C"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    // A replacement stating only its parents is the order of its nearest one.
    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      identifiers: [new Identifier('fix', 'origorderid', 'A'), new Identifier('fix', 'parentorderid', 'C')],
    })
    assert.equal(order.identifiers.getFrom('fix', 'orderid'), 'C')
    ```

## Where identifiers come from

| Holder | Rule |
| --- | --- |
| A graph leaf | what a caller states: the `insert_`/`set_`/`remove_` verbs of [`Market`](market.md#security-identifiers) (`insert_securityid`), [`Operation`](operation.md#identifiers) (`insert_identifier`) and [party ids](operation.md#party-identifiers) (`insert_partyid`), or the `securityids`, `identifiers` and `partyids` facts a binding builds a leaf from; finalizing derives the national code a stated ISIN embeds, from `derived`, and fills a base from its parents ([Parentage](#parentage)) |
| A FIX message | logical facts read off its fields at every settle, the wire kept as sent and never written back ([FIX](../fix/message.md#the-identifier-maps)): an identifier its field's `FIX:idmap` entry names is from `fix` (`fix:clordid`, `fix:orderid`), a regulatory trade identifier by its `RegulatoryTradeIDType(1906)` too; a security identifier is `SecurityID(48)` under its `SecurityIDSource(22)`'s type - a `{NAMESPACE}INSTRUMENTID` source an `instrumentid` from that namespace - each `SecAltIDGrp(454)` occurrence, and the codes an ISIN embeds from `derived`; a caller's write is the message's word and moves no field |
| A FIX entry no dictionary maps | each `metadata` key and each top-level untagged scalar of the message, and the keys its message type declares under `FIX:identifiers`, is read as [`Identifier::from_key`](#reading-a-key) reads a key: a security type is a `securityids` identifier (a value its type refuses is an anomaly), a party type a `partyids` one, any other type an `identifiers` one - `OMS_InstrumentID` is `oms:instrumentid` in `securityids`, `OMS_UserID` `oms:userid` in `partyids`, `firm.x.ParentOrderID` `firm.x:parentorderid` in `identifiers`; the entry stays in the metadata and on the wire as it arrived |
| A FIX party | each `Parties(453)` or `RootParties(1116)` occurrence: its `PartyID(448)` typed by its `PartyRole(452)` code's name folded (`ExecutingFirm` is `executingfirm`, a code the set names nothing for `partyrole{code}`, no role `party`), from its `PartyIDSource(447)` code's name folded (`D` is `proprietary`, `C` `generalidentifier`; a spelling the set resolves nothing for is its own spelling where it is a word; none `base`); `Account(1)` is a party typed `account` from its `AcctIDSource(660)` code's name. A second party of one role and source stays on the wire, no anomaly |
| A leaf a FIX message becomes | the message's sets, a book entry's or a trade side's own party ids leading, plus each unmapped scalar whose key names an identifier - a dictionary-tagged child only by the identifiers its message's type declares (`RefOrderID(1080)`'s `reforderid`), an untagged key by the crate's identifier names too - typed by `Identifier::from_key` and lifted into the set its type belongs to where that set holds its key free or with the same value; otherwise it stays in the leaf's metadata ([What a leaf's metadata holds](../fix/message.md#what-a-leafs-metadata-holds)) |

## Arrow

| Key | Rule |
| --- | --- |
| `Identifier::dtype()` | `struct<src, type, value>`, every child required `utf8`; the accessor `kind` is the column `type` |
| `Identifiers::dtype(item)` | a sorted map `map<entries: struct<key: utf8 not null, {item}: struct<src, type, value> not null>, keys_sorted = true>`: the key is the identifier's own `src:type`, and `item` is `securityid` for the `securityids` column, `identifier` for `identifiers` and `partyid` for `partyids` - the three columns of every generated row ([Row schemas](schemas.md)) |
| Writing | `into_scalar` lays the map out as a `Scalar::SortedMap` of the key text and the identifier row, in key order; a column is null where the map is empty |
| Reading | `from_scalar` reads a `Map` or a `SortedMap` in any order and refuses anything but a map of rows of three text cells, a cell `new` refuses, and a key that is not the `src:type` of the row it keys, each located on its key (`$['fix:clordid']`); the Arrow readers of the [`marketdata` row](schemas.md#the-marketdata-row) and of the [fixed FIX row](schemas.md#the-fix-row) refuse a key that disagrees with its row the same way |
| A lift | a [view](market-data.md#views) reaches one identifier by its key with the path grammar's map segment: `identifiers['fix:clordid'].value as clordid` - the key as stored, lower case; the `isincode` column carries a market row's ISIN |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::{Field, IdSource, IdType, Identifier, Identifiers, Scalar, Serie};

    let mut ids = Identifiers::new();
    assert!(ids.insert(Identifier::new(IdSource::Base, IdType::Isin, "US0378331005")?));
    assert!(ids.insert(Identifier::new(IdSource::Fix, IdType::ClOrdId, "C-2")?));
    assert!(!ids.insert(Identifier::new(IdSource::Fix, IdType::ClOrdId, "C-9")?), "fill only");

    // A map lays out as a sorted map keyed `src:type` and reads back whole.
    let field = Arc::new(Field::new("identifiers", Identifiers::dtype("identifier"), true));
    let column = Serie::from_scalars(field, [ids.into_scalar()])?;
    assert_eq!(Identifiers::from_scalar(&column.scalar(0)?)?, ids);

    // A key that is not its row's `src:type` is refused on that key.
    let row = Identifier::new(IdSource::Fix, IdType::Isin, "US0378331005")?.into_scalar();
    let misfiled = Scalar::from_mapping([(Scalar::from("fix:cusip"), row)])?;
    assert!(Identifiers::from_scalar(&misfiled).unwrap_err().to_string().contains("$['fix:cusip']"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        securityids=[Identifier("base", "isin", "US0378331005")],
    )
    table = graph.MarketData.arrow_reader([order]).read_all()
    securityids = table.schema.field("securityids").type
    assert str(securityids.key_type) == "string"
    assert [child.name for child in securityids.item_type] == ["src", "type", "value"]
    # A map keyed by `src:type`, in key order: the derived CUSIP beside the
    # stated ISIN.
    assert table.column("securityids").to_pylist()[0] == [
        ("base:isin", {"src": "base", "type": "isin", "value": "US0378331005"}),
        ("derived:cusip", {"src": "derived", "type": "cusip", "value": "037833100"}),
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      securityids: [new Identifier('base', 'isin', 'US0378331005')],
    })
    const table = graph.MarketData.arrowReader([order]).intoTable()
    const entries = table.schema.fields.find((field) => field.name === 'securityids').type.children[0].type
    assert.deepEqual(entries.children.map((child) => child.name), ['key', 'value'])
    assert.deepEqual(entries.children[1].type.children.map((child) => child.name), ['src', 'type', 'value'])
    // A map keyed by `src:type`, in key order: the derived CUSIP beside the
    // stated ISIN.
    const [read] = graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order]))
    assert.equal(read.intoLeaf().securityids.toString(), '[base:isin=US0378331005, derived:cusip=037833100]')
    ```

## Edges

- A word is folded, never guessed: `ISIN` and `isin` are one type, `IS/IN` is refused, and `IdType::from_security_source` - not `Identifier::new`, which takes the type already read - is what reads FIX's code `4` or the word `ISINNumber` as `isin`.
- Two sources of one type stand side by side - `fix:isin` and `base:isin` - and `get` answers the wire's (`fix`) whatever sorts before it, then another named source's in key order (`abc:isin` before `venue:isin`), then `base`'s, a `derived` one last; `get_from` names the one wanted.
- `insert` keeps a held value; a market element's [`insert_securityid`](market.md#security-identifiers) also takes back a `derived` identifier of the type it states.
- A value past 64 bytes is no identifier: a leaf a FIX message becomes keeps such a metadata scalar in its metadata rather than lifting it.
- A key is inferred, never trusted: `Identifier::from_key` reads the identifier name a key ends with and nothing else, so a bridge's `transversalkey` is no identifier and stays metadata.
- Folding is intake only: nothing writes an upper-case word back, and a `FIX:idmap` document is stricter than intake - its `key` must be the folded word (`orderid`) and an upper-case one is refused ([FIX registry](../fix/registry.md#a-field-names-a-message-by-its-identifiers)).

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- identifier idtype idsource
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
