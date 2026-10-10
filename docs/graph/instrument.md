# Instrument

An `Instrument` is one [element](element.md) per instrument, keyed by its cross code: a security an agency numbered by its real ISIN alone, and everything no agency numbers - an FX pair, a forward, a swap, an option, a future, a strategy - by `class:body`, its CFI class and the characteristics that make it one instrument. It holds every mapping it is known by - its ISIN, its CFI code, its short name, the pair, the national numbers, an issuer's LEI, every source's own statement of them - its listings nested, one per market, and the complementary facts no typed field holds. `Instruments` is the collection a process learns into: a lifecycle reads what each market element or FIX message states about its instrument, and fills what a later element leaves unsaid - among it `instcode`, the instrument's cross code, on every market row. The collection is bound to the store it was loaded from - an Arrow IPC leaf, a folder of parts, Parquet, an Iceberg table, an object store - and commits its table back as one snapshot only where it moved.

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl_market::Instrument` and `yggdryl_market::Instruments` (root `instrument.rs`, with `instrument/store.rs`, `instrument/env.rs` and `instrument/seed.rs`), `yggdryl_market::Listing` (`listing.rs`), `yggdryl_market::Characteristics`, `Settle`, `Expiry` and `Exercise` (`characteristics.rs`); Python `yggdryl.Instruments` and `yggdryl.instrument.Resolution`; JavaScript `Instruments`. A binding holds one collection behind one lock and crosses an instrument as a `dict` / plain object of its columns, a listing, a leg and the characteristics as nested ones; `Instrument`, `Listing` and `Characteristics` are Rust-only |
| Key | `crosscode`, at most `MAX_CODE_WIDTH` (128) bytes, written once by the instrument from its typed facts ([The cross code](#the-cross-code)): a real ISIN - closing under a listed prefix, `IdType::Isin.is_real` - for a security, `class:body` for everything else. Two constructors write a code, `Instrument::for_security(isin)` and `Instrument::for_body(class, forex, &characteristics, underlying, legs)`; there is no constructor from the text, which would give the key a second owner |
| Identity | an `Element` and no `Event`: `crossuuid` and `crosshashcode` the graph's [cross identity](element.md) of the code, `uuid = crossuuid`, so the code alone moves the identity; `hashcode` the digest of every fact through its typed accessors - the identifiers, the CFI, the country, the currencies, the underlying, the legs, the characteristics, the product category, the listings, the metadata, the aliases - the three stamps and `srcuuids` never fed. A multiplier, an exercise style, a listing or a metadata entry moves `hashcode` alone |
| Minted number | an instrument keyed `class:body` mints a `QY` ISIN from its code (`Instrument::minted_number(code)`, `Isin::minted` over the XXH3-128 of the code), held under `yggdryl:isin` and filled as the base `isin` where no real ISIN is held ([The minted number](#the-minted-number)); Python `Instruments.mint(crosscode)` |
| Row | `Instrument::field()` - Python and JavaScript `Instruments.field()` - the non-null struct `instrument` of twenty-five columns: the six element columns (`uuid`, `crossuuid`, `crosscode`, `hashcode`, `crosshashcode`, `srcuuids`), `aliascodes` (`serie<utf8>`, the codes it had before its re-keys), `placeholder` (`boolean`), `isin`, `cficode`, `forexcode`, `fisn` (projections of `securityids`, holding nothing of their own), `countrycode`, `currency`, `origccy`, `securityids` (a sorted `map<utf8, utf8>`), `underlying` (`utf8`, a code), `legs` (`serie<struct<code: utf8, ratio: int32>>`), `eusipacode` (`int32`), `characteristics` (`struct<settle, settle2, expiry: utf8, strikepx, multiplier: decimal, exercise: utf8>`), `listings` (`serie<struct<miccode: mic, ticker: utf8, currency: ccy, codes: map<utf8, utf8>>>`), `metadata` (a sorted `map<utf8, utf8>`), `updunix`, `firstunix`, `lastunix` (`datetime64(ns, UTC)`). The root declares `PARTITION:by` `["truncate(crosscode, 2)"]` - an ISIN's country prefix, a body's class letters, stored as no column - and `SORT:by` `["crosscode"]`, the order the snapshot streams in |
| `instcode` | the market fact naming the instrument a market element is about: [`Market::get_instcode`](market.md), the instrument's cross code as `utf8`, the 37th market column of every `marketdata` row and crate tag `65_054` of the [FIX row](../fix/capture.md). The parse writes it where the message alone spells the code - a stated real ISIN, a detected FX pair - and a fill writes it from the instrument it resolved, never over a held one; it is held, followed along a chain and written, and fed to no digest, as `crossuuid` is not. A filled `instcode` shares the instrument's one allocation of its code. A reader joins a market table to the instruments on `instcode = crosscode`, or `instcode` among `aliascodes` for a row filled before a [re-key](#identity-moves) |
| `merge(entry)` | folds one instrument into the instrument of its cross code by the [update rule](#the-update-rule), an entry spelling a placeholder's body re-keying it; whether anything moved. A new instrument past `max_instruments` refused |
| `learn(event)` | keyed by the code the event's facts spell: a stated real ISIN, never a derivation; a `forex` identifier beside an `I*` class keys the FX spot and mints its number; a derivative stating a real ISIN and no body is a placeholder under it. Its market names the listing its listing facts land on, at its `transunix`. A `J*` or `S*` element states no settle and learns nothing, a ticker-only security has no instrument, and a new instrument past the bound is passed over with one warning per collection. The underlying, the legs, a derivative's characteristics, the product category and the descriptive metadata are the FIX [lifecycle's](../fix/lifecycle.md#instruments-are-learned-in-instant-order) reading of a message ([Derivatives and strategies](#derivatives-and-strategies)) |
| `fill(element)` | from the instrument `resolve` names: each identifier of a type it holds none of, as a `derived` one - the minted number of a pair among them - the listing codes and the ticker where its market is listed or it states none and the instrument has one listing, the CFI where it states none or the instrument's [refines](../types/codes/cfi.md#two-statements-of-one-instrument) it, the currency only on the same market under the listing's ticker, the origin currency, and the cross code as `instcode` where it holds none; finalized where anything moved. Nothing reaches a FIX field or the wire |
| `enrich(event)` | `learn`, then `fill` |
| `resolve(element)` | the [matching waterfall](#matching) without a write: `Resolution::Matched { entry, tier, derived, listing }` - the instrument, the `MatchTier` (`Isin`, `CrossCode`, `Code(IdType)`, `Symbology`, `Economic { similarity }`), whether the key was derived, whether the instrument's listing on the element's market is the element's - or `Resolution::Unmatched(Unmatched)`: `NoKey`, `UnknownIsin`, `UnknownCode`, `NoCandidate`, `Ambiguous`, `CfiConflict`, `CurrencyConflict`, `BelowThreshold`, each carrying what it names. Python `resolve(element) -> Resolution`, attributes `matched`, `entry`, `tier`, `kind`, `similarity`, `derived`, `listing`, `unmatched`, `codes`, `stated`, `held`, `best`, `code`; JavaScript `resolve(element)` a plain object of the same fields. Any market leaf, a `MarketData` or a `FixMsg` |
| Reads | `get(key)` borrows the instrument a cross code, an alias or an ISIN it holds - real or minted - names, allocation-free; Rust and Python `get_by_uuid(uuid)` the one a cross identity names, a former one included; `listings(key)` its listings in MIC order, empty where unknown; `get_listing(key, market)` one; `get_by_ticker(ticker, market)` the one instrument listing the ticker on `market`, else on no market, and with `market` unstated - none and `XXXX` - on any, two answering none; `get_by_code(kind, value)` the one instrument holding a code of `LOOKUP_CODES`, two answering none; `iter()` every instrument in code order; `len` the instruments, `rows` the rows a commit writes - one per instrument - `is_empty`, `max_instruments`, `is_dirty`; `remove(key)` answers the instrument, `remove_listing(key, market)` one listing, the instrument staying; `clear()`. Python spells them alike - `rows`, `is_dirty` and `max_instruments` properties, `LOOKUP_CODES` a class attribute; JavaScript `getListing`, `getByTicker`, `getByCode`, `removeListing`, `Instruments.lookupCodes()` and the `length`, `rows`, `isDirty` getters |
| Economic settings | `economic_threshold` (`DEFAULT_ECONOMIC_THRESHOLD`, `0.85`), `set_economic_threshold` - NaN and anything outside `(0, 1]` refused at `$.economic_threshold`, moving nothing - `try_with_economic_threshold`; `is_economic_match` (`false`), `set_economic_match`, `with_economic_match`: whether a fill takes an economic match. The collection's, never the store's |
| Persistence | `from_holder(holder)` / `from_url(url, properties)` bind and load the store's rows alone; `seeded_from_holder` / `seeded_from_url` lay them over the [seed](#seed); `set_holder` / `try_with_holder` bind a collection already holding instruments; `commit()` writes the snapshot back only where its content differs from the store's; `from_arrow_reader`, `extend_from_arrow_reader`, `extend_from_handle` read any record stream without binding; `into_arrow_reader()` is the snapshot stream ([Persistence](#persistence)). Python `Instruments.from_url(location, max_instruments=..., **properties)`, `seeded_from_url`; JavaScript `Instruments.fromUrl(location, maxInstruments?, properties?)`, `seededFromUrl` |
| The process's own | `from_env()` resolves once from `YGGDRYL_INSTRUMENTS_URI`, else `~/.config/yggdryl/instruments/`, else no store, laid over the seed, shared behind one lock; `install_env` installs one first; `FixCodec::from_env()` attaches it ([The process instruments](#the-process-instruments)) |
| Sharing | `FixCodec::with_instruments(Arc<Mutex<Instruments>>)` shares one collection with every lifecycle the codec runs and every parse door it opens: a door fixes the table once as it opens and fills derived identifiers from it on every worker; a lifecycle learns and fills under one lock per message. Python `FixCodec(..., instruments=held)`, JavaScript `new fix.FixCodec(registry, { instruments })`, each with an `instruments` getter answering the caller's own collection |

## The cross code

The code is written once, by the instrument, from its typed facts - `crosscode := isin | class ':' body`:

| Class | Body | Example |
| --- | --- | --- |
| a security an agency numbered - an equity, a fund, a bond, an index | none: the real ISIN alone, the CFI a fact beside it | `US0378331005` |
| `I*` FX spot or index, `IT` a commodity's | the pair | `IF:EUR/USD`, `IT:XAU/USD` |
| `J*` forward | the pair and its settle - a tenor (`SettlType(63)`) or a date | `JF:EUR/USD:M3`, `JF:EUR/USD:2027-01-15` |
| `S*` swap | the pair, the near settle and the far one | `SF:USD/JPY:0:M3` |
| `O*` option, `H*` | the underlying's code, the expiry day and the strike | `OC:US0378331005:2026-12-18:200` |
| `F*` future | the underlying's code and the contract month (a week as `yyyy-mmwN`) | `FF:EU0009658145:2026-12` |
| `K*` strategy | the legs' codes joined `+` in code order, a ratio other than one as an `n*` prefix | `KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03` |

Never in the code: a ticker, a market, an LEI, a RIC, the multiplier, the exercise style, a uuid, the minted number. A strike is written as its shortest decimal, so `200`, `200.0` and `2.0E2` are one code; legs stated in either order are one code; a future stated by its last trading day keys by the month it falls in. A body the facts do not spell whole - an option with no strike, a class keying by no body - is refused naming the class, and a 129th byte is refused by name. A derivative stating a real ISIN and no body is a placeholder keyed by that ISIN until its body arrives ([Identity moves](#identity-moves)).

The pins every release keeps, `rust/market/tests/root/instrument.rs`:

| `crosscode` | `crosshashcode` (XXH3-64) | Minted `isin` |
| --- | --- | --- |
| `US0378331005` | `27e376388c8738fd` | - |
| `EU0009658145` | `fe3a11ff3d28a176` | - |
| `IF:EUR/USD` | `f4bd070ccad3d90f` | `QYLTVIRYHNX5` |
| `JF:EUR/USD:M3` | `817c2b867e12a51a` | `QYIJ9KBCDDV1` |
| `JF:EUR/USD:2027-01-15` | `b1e8c6896a1c7a77` | `QYI2FZFJUNE8` |
| `SF:USD/JPY:0:M3` | `7fffc4bbae889ca0` | `QYKXOCJXPVF9` |
| `IT:XAU/USD` | `f0083480b6c87b8d` | `QY900CSWCFZ9` |
| `OC:US0378331005:2026-12-18:200` | `6f50390c1bd8f746` | `QYK7DLZMSYS1` |
| `OC:US0378331005:2026-12-18:210` | `3b15a30b31569400` | `QYG1U5ULBQ73` |
| `OP:US0378331005:2026-12-18:200` | `abb15048d5817f7b` | `QY6TB00ZO934` |
| `FF:EU0009658145:2026-12` | `4de3b6fc251f4c55` | `QY4NFU6XFYI7` |
| `FF:EU0009658145:2027-03` | `1024ff3991480a91` | `QY3KUS57QPX3` |
| `KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03` | `69efa5da470620f0` | `QY9THOC9IUF9` |

`crossuuid` and `uuid` are the graph's cross identity of the code, today `Uuid::from_v8` over the XXH3-64; they move once with every other cross identity of the tree, the code, its hash code and the minted number never.

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Cfi, Decimal, Forex, Isin, Uuid};
    use yggdryl_market::{Characteristics, Exercise, Instrument, Leg, MAX_CODE_WIDTH};
    yggdryl_market::install()?;

    // A security is keyed by its real ISIN alone; its identity is the code's.
    let apple = Instrument::for_security(Isin::new("US0378331005")?)?;
    assert_eq!(apple.get_crosscode(), "US0378331005");
    assert_eq!(format!("{:016x}", apple.get_crosshashcode()), "27e376388c8738fd");
    assert_eq!(apple.get_uuid(), apple.get_crossuuid());
    assert_eq!(apple.get_crossuuid(), Uuid::from_v8(u128::from(apple.get_crosshashcode())));
    assert!(Instrument::for_security(Isin::new("US0378331006")?).is_err(), "no real number");

    // A spot is keyed by its class and its pair, and mints its number.
    let spot = Instrument::for_body(Cfi::new("IFXXXP")?, Some(&Forex::new("EUR/USD")?), &Characteristics::default(), None, &[])?;
    assert_eq!((spot.get_crosscode(), spot.minted_isin()), ("IF:EUR/USD", Some("QYLTVIRYHNX5")));

    // An option by its underlying's code, its expiry and its strike; the
    // multiplier and the exercise style move its content code alone.
    let call = |strike: &str| -> yggdryl::Result<Instrument> {
        let terms = Characteristics::default()
            .with_expiry(Some("2026-12-18".parse()?))
            .with_strikepx(Some(Decimal::parse(strike)?));
        Instrument::for_body(Cfi::new("OCXXXX")?, None, &terms, Some("US0378331005"), &[])
    };
    assert_eq!(call("200")?.get_crosscode(), "OC:US0378331005:2026-12-18:200");
    assert_eq!(call("2.0E2")?.get_crosscode(), call("200.0")?.get_crosscode());
    let american = Instrument::for_body(
        Cfi::new("OCXXXX")?,
        None,
        &Characteristics::default()
            .with_expiry(Some("2026-12-18".parse()?))
            .with_strikepx(Some(Decimal::parse("200")?))
            .with_exercise(Some(Exercise::American)),
        Some("US0378331005"),
        &[],
    )?;
    assert_eq!(american.get_crossuuid(), call("200")?.get_crossuuid());
    assert_ne!(american.get_hashcode(), call("200")?.get_hashcode());

    // A strategy by its legs in code order, whatever order they were stated in.
    let legs = [Leg::new("FF:EU0009658145:2027-03", 1)?, Leg::new("FF:EU0009658145:2026-12", 1)?];
    let spread = Instrument::for_body(Cfi::new("KEXXXX")?, None, &Characteristics::default(), None, &legs)?;
    assert_eq!(spread.get_crosscode(), "KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03");
    assert!(spread.get_crosscode().len() <= MAX_CODE_WIDTH);
    // A body the facts do not spell whole is refused naming the class.
    assert!(Instrument::for_body(Cfi::new("OCXXXX")?, None, &Characteristics::default(), Some("US0378331005"), &[]).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    held = Instruments()
    # A security is keyed by its real ISIN alone; its identity is the code's.
    assert held.merge({"isin": "US0378331005", "cficode": "ESVUFR"})
    apple = held.get("US0378331005")
    assert apple["crosscode"] == "US0378331005" and apple["crosshashcode"] == 0x27E376388C8738FD
    assert apple["uuid"] == apple["crossuuid"] == "00000000-0000-8000-a7e3-76388c8738fd"

    # An option by its underlying's code, its expiry and its strike, minted
    # its number; the multiplier and the exercise style are facts beside it.
    assert held.merge({
        "cficode": "OCXXXX",
        "underlying": "US0378331005",
        "characteristics": {"expiry": "2026-12-18", "strikepx": "2.0E2", "multiplier": "100", "exercise": "american"},
    })
    option = held.get("OC:US0378331005:2026-12-18:200")
    assert option["isin"] == "QYK7DLZMSYS1" == Instruments.mint("OC:US0378331005:2026-12-18:200")
    assert option["characteristics"]["exercise"] == "American"

    # A strategy by its legs in code order, a ratio other than one a prefix.
    assert held.merge({"cficode": "KEXXXX", "legs": [{"code": "US0378331005", "ratio": 1}, {"code": "OC:US0378331005:2026-12-18:200", "ratio": 2}]})
    assert held.get("KE:2*OC:US0378331005:2026-12-18:200+US0378331005") is not None
    assert [row["crosscode"] for row in held.into_arrow_reader().read_all().to_pylist()] == [
        "KE:2*OC:US0378331005:2026-12-18:200+US0378331005",
        "OC:US0378331005:2026-12-18:200",
        "US0378331005",
    ], "in code order"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    // A row merges with the element columns it requires; the key and the
    // identity are written again from the facts.
    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })

    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: 'US0378331005', cficode: 'ESVUFR' })))
    const apple = held.get('US0378331005')
    assert.deepEqual([apple.crosscode, apple.crosshashcode], ['US0378331005', 0x27e376388c8738fdn])
    assert.equal(apple.uuid, apple.crossuuid)

    // An option by its underlying's code, its expiry and its strike, minted its number.
    assert.ok(held.merge(statement({
      cficode: 'OCXXXX',
      underlying: 'US0378331005',
      characteristics: { settle: null, settle2: null, expiry: '2026-12-18', strikepx: '200', multiplier: null, exercise: null },
    })))
    assert.equal(held.get('OC:US0378331005:2026-12-18:200').isin, 'QYK7DLZMSYS1')
    ```

## Use

A lifecycle learns each statement and fills the next element naming the same instrument. A market element of any kind is learned through `Market` facts alone; a FIX message through a codec sharing the collection adds what only a message spells ([Derivatives and strategies](#derivatives-and-strategies)).

=== "Rust"

    ```rust
    use yggdryl::Mic;
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IdKey, IdType, Identifier, Instruments};
    yggdryl_market::install()?;

    // An order stating Holcim's ISIN, its RIC, its market, its ticker and its currency.
    let mut stated = OrderEvent::at(1_767_348_000_000_000_000);
    stated.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    stated.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
    stated.set_miccode(Some(Mic::new("XSWX")?), true);
    stated.set_ticker(Some("HOLN".into()), true);
    let mut instruments = Instruments::new();
    assert!(instruments.learn(&stated));
    let holcim = instruments.get("CH0012214059").expect("learned");
    assert_eq!(holcim.ticker(Some(&Mic::new("XSWX")?)), Some("HOLN"));
    assert_eq!(holcim.get(&IdType::Valor), Some("1221405"), "the Valor its ISIN embeds");
    let listing = holcim.listing(Some(&Mic::new("XSWX")?)).expect("the market stated");
    assert_eq!(listing.get(&IdType::Ric), Some("HOLN.S"), "a listing code is its market's");

    // A later order naming only its ticker on that market is filled, its code among the rest.
    let mut later = OrderEvent::at(1_767_348_060_000_000_000);
    later.set_ticker(Some("HOLN".into()), true);
    later.set_miccode(Some(Mic::new("XSWX")?), true);
    assert!(instruments.fill(&mut later));
    assert_eq!(later.get_instcode(), Some("CH0012214059"));
    assert_eq!(later.get_isincode(), Some("CH0012214059"));
    assert!(later.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(later.get_currency().as_str(), "CHF", "the market's currency, on the same market");
    assert!(!instruments.fill(&mut later), "nothing left to fill");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import Instruments
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
    held = Instruments()
    stated = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|")
    assert stated.instcode == "CH0012214059", "a stated real ISIN is the code from the parse"
    assert held.learn(stated)
    holcim = held.get("CH0012214059")
    assert holcim["securityids"] == {"cfi": "ESVUFR", "isin": "CH0012214059", "valor": "1221405"}
    assert holcim["listings"] == [{"miccode": "XSWX", "ticker": "HOLN", "currency": "CHF", "codes": {"ric": "HOLN.S"}}]

    # A later message naming only its ticker on that market is filled, never on the wire.
    later = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|")
    assert (later.isincode, later.instcode) == (None, None)
    assert held.fill(later)
    assert (later.isincode, later.instcode) == ("CH0012214059", "CH0012214059")
    assert later.securityids.is_derived("isin") and later.securityids.get("ric") == "HOLN.S"
    assert b"48=" not in later.into_bytes(ord("|"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Instruments, fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const held = new Instruments()
    const stated = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|'))
    assert.equal(stated.instcode, 'CH0012214059', 'a stated real ISIN is the code from the parse')
    assert.ok(held.learn(stated))
    assert.deepEqual(held.listings('CH0012214059'), [
      { codes: new Map([['ric', 'HOLN.S']]), currency: 'CHF', miccode: 'XSWX', ticker: 'HOLN' },
    ])

    // A later message naming only its ticker on that market is filled, never on the wire.
    const later = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|'))
    assert.ok(held.fill(later))
    assert.deepEqual([later.isincode, later.instcode], ['CH0012214059', 'CH0012214059'])
    assert.ok(later.securityids.isDerived('isin'))
    assert.ok(!later.intoText('|').includes('48='))
    ```

## Seed

`Instruments::seeded()` - Python and JavaScript `Instruments.seeded()` - answers a collection holding the common instruments the crate ships: large equities on their primary listing - HSBC in Hong Kong besides London - and the main equity indices, each with its ticker, market, currency, country, CFI code and, where FIRDS spells one, its short name: 208 instruments holding 209 listings. The source is `config/instruments/instruments.json`, one object per instrument - `isin`, `cficode`, `countrycode`, `fisn`, `origccy` and its `listings` of `miccode`, `ticker` and `currency` - and the one file maintained by hand; any other key of an object is a [metadata](#metadata-and-sourced-identifiers) entry. The crate embeds its copy `rust/market/src/instrument/seed.json`, inside the crate's package, which `python scripts/check_instruments_seed.py --sync` writes byte for byte; the checker holds the file to its shape, every ISIN's check digit, uniqueness and order, the MICs to the ISO 10383 table, the CFI codes and the short names. Each object is folded by `merge` as an ordinary statement, so it carries the [derived facts](#derived-facts) any instrument carries - Apple's CUSIP below is one, stated nowhere in the file. The collection is clean and bound to no store, so `commit` refuses; each call answers a collection of its own, sharing the one parsed table until a write moves it. `Instruments::new()` holds none of it.

=== "Rust"

    ```rust
    use yggdryl::Mic;
    use yggdryl_market::{IdType, Instruments};
    yggdryl_market::install()?;

    let seeded = Instruments::seeded();
    assert_eq!((seeded.len(), seeded.rows()), (208, 208));
    assert!(!seeded.is_dirty() && seeded.holder().is_none());
    assert!(Instruments::new().is_empty(), "a collection built by hand holds none of it");
    assert_eq!(seeded.listings("GB0005405286").len(), 2, "HSBC in Hong Kong and London");

    let apple = seeded.get("US0378331005").expect("seeded");
    assert_eq!(apple.ticker(Some(&Mic::new("XNAS")?)), Some("AAPL"));
    assert_eq!(apple.fisn().map(|name| name.as_str().to_owned()).as_deref(), Some("APPLE INC/SH SH"));
    assert_eq!(apple.cficode().map(|code| code.as_str().to_owned()).as_deref(), Some("ESVUFR"));
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"), "the CUSIP its ISIN embeds");
    assert_eq!(seeded.get_by_ticker("AAPL", Some(&Mic::new("XNAS")?)), Some(apple));
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    seeded = Instruments.seeded()
    assert (len(seeded), seeded.rows) == (208, 208) and not seeded.is_dirty
    assert len(Instruments()) == 0, "a collection built by hand holds none of it"
    assert [listing["miccode"] for listing in seeded.listings("GB0005405286")] == ["XHKG", "XLON"]

    apple = seeded.get("US0378331005")
    assert (apple["fisn"], apple["cficode"]) == ("APPLE INC/SH SH", "ESVUFR")
    assert apple["listings"] == [{"miccode": "XNAS", "ticker": "AAPL", "currency": "USD", "codes": None}]
    assert apple["securityids"]["cusip"] == "037833100", "the CUSIP its ISIN embeds"
    assert seeded.get_by_ticker("AAPL", "XNAS") == apple
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const seeded = Instruments.seeded()
    assert.deepEqual([seeded.length, seeded.rows, seeded.isDirty], [208, 208, false])
    assert.equal(new Instruments().length, 0, 'a collection built by hand holds none of it')
    assert.deepEqual(seeded.listings('GB0005405286').map((listing) => listing.miccode), ['XHKG', 'XLON'])

    const apple = seeded.get('US0378331005')
    assert.deepEqual([apple.fisn, apple.cficode], ['APPLE INC/SH SH', 'ESVUFR'])
    assert.equal(apple.securityids.get('cusip'), '037833100', 'the CUSIP its ISIN embeds')
    assert.equal(seeded.getByTicker('AAPL', 'XNAS').crosscode, 'US0378331005')
    ```

## The minted number

An instrument no agency numbers still needs an ISIN wherever a reader joins on `isin` or keys a [book](market.md#the-book-key) by it, so the crate mints one: the prefix `QY`, a user-assigned code ISO 3166 leaves to private use, then nine base-36 digits of the low 46 bits of the code's XXH3-128, closed by the ISIN check digit (`Isin::minted`, `Isin::is_minted`). It is a pure function of the code: every process and every parse door mints the same number for the same instrument, and the number is never in the key. It ranks below every agency's number ([ranks](identifier.md#ranks)): an instrument holds it under `yggdryl:isin` and as its base `isin` only while no real ISIN is held, a stated real number wins, and a number stated that is not this instrument's own mint (`is_own_mint`) is kept as stated and minted over by nothing. Two codes minting one number are refused at the second learn by name, the refused instrument keeping its identity with no number.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, Forex, Isin};
    use yggdryl_market::{Characteristics, Instrument};
    yggdryl_market::install()?;

    let number = Instrument::minted_number("IF:EUR/USD");
    assert_eq!(number.as_str(), "QYLTVIRYHNX5");
    assert!(Isin::is_minted(number.as_str(), yggdryl::xxhash::xxh128(b"IF:EUR/USD")));
    let spot = Instrument::for_body(Cfi::new("IFXXXP")?, Some(&Forex::new("EUR/USD")?), &Characteristics::default(), None, &[])?;
    assert_eq!((spot.isin(), spot.minted_isin()), (Some("QYLTVIRYHNX5"), Some("QYLTVIRYHNX5")));
    assert!(spot.is_own_mint(&number));
    assert!(!spot.is_own_mint(&Isin::new("QY2QX016JGV0")?), "another number of the shape");
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    assert Instruments.mint("IF:EUR/USD") == "QYLTVIRYHNX5"
    assert Instruments.mint("OC:US0378331005:2026-12-18:200") == "QYK7DLZMSYS1"
    held = Instruments()
    assert held.merge({"cficode": "IFXXXP", "forexcode": "EUR/USD"})
    pair = held.get("QYLTVIRYHNX5")
    assert pair["crosscode"] == "IF:EUR/USD" and pair["securityids"]["yggdryl:isin"] == "QYLTVIRYHNX5"
    ```

Rust and Python; JavaScript reads the number off an instrument's `isin`.

## Forex

An FX pair has no agency's number, so it is keyed by its class and its body. The parse detects a pair off `Symbol(55)` ([FX symbols](../types/codes/forex.md)) and settles its class from `CFICode(461)`, else `SecurityType(167)`, else `IFXXXP` for a spot; the message then spells its code from that alone - `IF:EUR/USD` for a spot, `JF:EUR/USD:M3` off a forward's `SettlType(63)` or `JF:EUR/USD:2027-01-15` off its `SettlDate(64)`, `SF:USD/JPY:0:M3` off a swap's two settles, its far one read from `SettlDate2(193)` - writes it as `instcode`, and derives the minted number under `derived:isin` beside `derived:forex`, so a stated ISIN always wins and the pair's [book](market.md#the-book-key) is keyed by its number (`3:0:QYLTVIRYHNX5`) from the first message. Every parse door derives the same number with no table. The lifecycle learns the instrument: its currency is the quote leg - EUR/USD is dollars per euro - its country none. A listing code stated on no market - a RIC `EUR=`, which every FX pair is stated under with no `SecurityExchange(207)` - stays among `securityids` until a market is learned for it ([Listings](#listings)). A bare market element states no settle, so it learns a spot (`I*`) from a `forex` identifier and its class alone, and a forward or a swap only through a FIX message.

=== "Rust"

    ```rust
    use std::sync::{Arc, Mutex};

    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::graph::Market;
    use yggdryl_market::{IdType, Instruments};
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec = FixCodec::new(registry).with_instruments(Arc::clone(&instruments));
    let parsed: Vec<FixMsg> = codec
        .parse_lines(["8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:01|11=1|55=EUR/USD|54=1|38=1000000|44=1.0850|10=0|"])
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(parsed[0].get_instcode(), Some("IF:EUR/USD"), "the message alone spells it");
    assert_eq!(parsed[0].get_isincode(), Some("QYLTVIRYHNX5"));
    assert!(parsed[0].get_securityids().is_derived(&IdType::Isin));
    let walked: Vec<FixMsg> = codec.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    assert_eq!(walked[0].get_instcode(), Some("IF:EUR/USD"));
    let held = instruments.lock().expect("the instruments");
    let pair = held.get("IF:EUR/USD").expect("learned by the walk");
    assert_eq!(pair.currency().map(|code| code.as_str()), Some("USD"), "the quote leg");
    assert_eq!(pair.country(), None);
    assert_eq!(held.get("QYLTVIRYHNX5").map(|found| found.minted_isin()), Some(Some("QYLTVIRYHNX5")));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import Instruments
    from yggdryl.fix import FixCodec, FixRegistry

    held = Instruments()
    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")), instruments=held)
    parsed = codec.parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:01|11=1|55=EUR/USD|54=1|38=1000000|44=1.0850|10=0|")
    assert (parsed.instcode, parsed.isincode) == ("IF:EUR/USD", "QYLTVIRYHNX5"), "the message alone spells it"
    [walked] = codec.lifecycle([parsed])
    pair = held.get("IF:EUR/USD")
    assert (pair["forexcode"], pair["currency"], pair["countrycode"]) == ("EUR/USD", "USD", None), "the quote leg"
    # Another spelling of the pair is the same instrument.
    list(codec.lifecycle([codec.parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:02|11=2|55=EURUSD CURNCY|54=2|38=500000|10=0|")]))
    assert len(held) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Instruments, fix } = require('yggdryl')

    const held = new Instruments()
    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')), { instruments: held })
    const parsed = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:01|11=1|55=EUR/USD|54=1|38=1000000|44=1.0850|10=0|'))
    assert.deepEqual([parsed.instcode, parsed.isincode], ['IF:EUR/USD', 'QYLTVIRYHNX5'], 'the message alone spells it')
    const [walked] = [...codec.lifecycle([parsed])]
    assert.equal(walked.instcode, 'IF:EUR/USD')
    const pair = held.get('IF:EUR/USD')
    assert.deepEqual([pair.forexcode, pair.currency, pair.countrycode], ['EUR/USD', 'USD', null], 'the quote leg')
    ```

## Derivatives and strategies

A derivative's code reads its underlying's code, which only the collection resolves, so the parse writes no `instcode` for one and the lifecycle learns it: `FixMsg::stated_characteristics()` answers the class - `CFICode(461)` as settled, `PutOrCall(201)` refining an option - the pair, and the characteristics a body is written from: the expiry from `MaturityDate(541)` with `MaturityDay` folded, else `MaturityMonthYear(200)` as a month or a week; `StrikePrice(202)`; the settles from `SettlDate(64)`/`SettlDate2(193)` else `SettlType(63)`; `ContractMultiplier(231)` and `ExerciseStyle(1194)` as facts. The underlying is the instrument `UnderlyingSecurityID(309)` names under an ISIN source, an `UnderlyingSymbol(311)` shaped as an ISIN, a related `Underlier` or a bridge's `UnderlyingISIN` resolves to in the collection; a strategy's legs are the `NoLegs(555)` group's `LegSecurityID(602)` under `LegSecurityIDSource(603)` with `LegRatioQty(623)`, each resolved to its code, sorted and deduplicated. Every value is read by its field's type. A derivative whose underlying no instrument keys is learned as a placeholder under its real ISIN; a message stating neither a real ISIN nor a body learns nothing. The real ISIN a derivative states stays a fact beside its key, and no number is minted over it.

The lifecycle also learns what the instrument's FIX component describes and no typed fact holds, into its [metadata](#metadata-and-sourced-identifiers): `Issuer(106)`, `SecurityDesc(107)`, `SecurityType(167)`, `SecuritySubType(762)`, `Product(460)`, `ProductComplex(1227)`, `SecurityGroup(1151)`, `SecurityStatus(965)`, `UnitOfMeasure(996)`, `StateOrProvinceOfIssue(471)` and `LocaleOfIssue(472)`, each under the dictionary's name for the field, its text trimmed.

=== "Rust"

    ```rust
    use std::sync::{Arc, Mutex};

    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::graph::Market;
    use yggdryl_market::{Exercise, Instruments};
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec = FixCodec::new(registry).with_instruments(Arc::clone(&instruments));
    let walk = |seq: u32, body: &str| -> yggdryl::Result<Vec<FixMsg>> {
        let line = format!("8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|");
        let parsed: Vec<FixMsg> = codec.parse_lines([line]).collect::<yggdryl::Result<_>>()?;
        codec.lifecycle(parsed).collect()
    };
    // The underlying first, then the call on it, numbered by its exchange.
    walk(1, "22=4|48=US0378331005|461=ESVUFR")?;
    let call = walk(2, "22=4|48=DE000C000000|461=OCASPS|201=1|202=200|541=20261218|1194=1|231=100|711=1|309=US0378331005|305=4")?;
    assert_eq!(call[0].get_instcode(), Some("OC:US0378331005:2026-12-18:200"), "filled by the lifecycle");
    assert_eq!(call[0].get_isincode(), Some("DE000C000000"), "the real number stays the row's");
    let held = instruments.lock().expect("the instruments");
    let option = held.get("DE000C000000").expect("the number leads to the option");
    assert_eq!(option.underlying(), Some("US0378331005"));
    assert_eq!(option.characteristics().exercise(), Some(Exercise::American));
    assert_eq!(option.minted_isin(), None, "a numbered instrument mints none");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import Instruments
    from yggdryl.fix import FixCodec, FixRegistry

    held = Instruments()
    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")), instruments=held)


    def walk(seq: int, body: str) -> list:
        line = f"8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|{body}|10=0|"
        return list(codec.lifecycle(codec.parse_lines([line.encode()])))


    walk(1, "22=4|48=EU0009658145|461=TIXXXX")
    for seq, (number, month) in enumerate((("DE000F000007", "202612"), ("DE000F000015", "202703")), start=2):
        walk(seq, f"22=4|48={number}|461=FFICSX|200={month}|711=1|309=EU0009658145|305=4")
    # A calendar spread over the two futures, its legs in code order.
    spread = "8=FIX.4.4|35=AB|49=S|56=T|34=4|52=20260102-10:15:04|11=4|55=FESX-SPREAD|461=KEXXXX|555=2|600=FESX|602=DE000F000015|603=4|623=1|600=FESX|602=DE000F000007|603=4|623=1|10=0|"
    [walked] = codec.lifecycle(codec.parse_lines([spread.encode()]))
    assert walked.instcode == "KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03"
    strategy = held.get(walked.instcode)
    assert [(leg["code"], leg["ratio"]) for leg in strategy["legs"]] == [("FF:EU0009658145:2026-12", 1), ("FF:EU0009658145:2027-03", 1)]
    assert strategy["isin"] == Instruments.mint(walked.instcode)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Instruments, fix } = require('yggdryl')

    const held = new Instruments()
    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')), { instruments: held })
    const walk = (seq, body) => {
      const line = `8=FIX.4.4|35=D|49=S|56=T|34=${seq}|52=20260102-10:15:0${seq}|11=${seq}|${body}|10=0|`
      return [...codec.lifecycle([...codec.parseLines([Buffer.from(line)])])]
    }
    walk(1, '22=4|48=EU0009658145|461=TIXXXX')
    const [future] = walk(2, '22=4|48=DE000F000007|461=FFICSX|200=202612|711=1|309=EU0009658145|305=4')
    assert.equal(future.instcode, 'FF:EU0009658145:2026-12', 'keyed by its contract month')
    assert.equal(held.get('DE000F000007').underlying, 'EU0009658145')
    ```

## Identity moves

A derivative first met stating only its venue's number - a Eurex option's ISIN, before any message spells its underlying, expiry and strike - is a placeholder keyed by that real ISIN. When a statement spells its body, the instrument is re-keyed in place, once: its code becomes `class:body`, the old code joins `aliascodes` (at most `MAX_ALIASES`, sorted), its identity becomes the new code's, the listings and every fact stay, and every instrument holding the old code - a future written on it, a strategy holding it as a leg - is re-keyed in the same fold, each keeping its own old code among its aliases. `get` and `get_by_uuid` answer the instrument by an alias and by the identity it had before, so a store, a market row's `instcode` or a uuid written before the re-key still finds it; the gold join is `instcode = crosscode or instcode in aliascodes`. Nothing else moves a key: any other call that would move one in place is refused at `$.crosscode` naming both codes.

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Cfi, Decimal, Isin, Mic};
    use yggdryl_market::{Characteristics, Instrument, Instruments, Listing};
    yggdryl_market::install()?;

    let eurex = "DE0000000009";
    let mut instruments = Instruments::new();
    instruments.merge(Instrument::for_security(Isin::new("US0378331005")?)?)?;
    let placeholder = Instrument::for_security(Isin::new(eurex)?)?
        .try_with_cficode(Some(Cfi::new("OCXXXX")?))?
        .with_listing(Listing::new(Some(Mic::new("XEUR")?)).with_ticker(Some("ODAX".into())))?;
    assert!(instruments.merge(placeholder)?);
    let before = instruments.get(eurex).expect("held").clone();
    assert!(before.is_placeholder());

    // The body arrives on the option's number.
    let terms = Characteristics::default()
        .with_expiry(Some("2026-12-18".parse()?))
        .with_strikepx(Some(Decimal::parse("200")?));
    let body = Instrument::for_security(Isin::new(eurex)?)?
        .try_with_cficode(Some(Cfi::new("OCXXXX")?))?
        .try_with_underlying(Some("US0378331005"))?
        .try_with_characteristics(terms)?;
    assert!(instruments.merge(body)?);
    assert_eq!(instruments.len(), 2, "re-keyed, not added");
    let option = instruments.get("OC:US0378331005:2026-12-18:200").expect("the new code");
    assert!(!option.is_placeholder());
    assert_eq!(option.aliascodes(), [eurex]);
    assert_eq!(option.isin(), Some(eurex), "the number stays a fact");
    assert_eq!(option.ticker(Some(&Mic::new("XEUR")?)), Some("ODAX"), "the listing stays");
    assert_eq!(instruments.get(eurex).map(Element::get_crosscode), Some(option.get_crosscode()));
    assert_eq!(instruments.get_by_uuid(before.get_uuid()).map(Element::get_crosscode), Some(option.get_crosscode()));
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    EUREX = "DE0000000009"
    held = Instruments()
    assert held.merge({"isin": "US0378331005"})
    assert held.merge({"isin": EUREX, "cficode": "OCXXXX", "listings": [{"miccode": "XEUR", "ticker": "ODAX"}]})
    placeholder = held.get(EUREX)
    assert placeholder["placeholder"] and placeholder["crosscode"] == EUREX

    # The body arrives on the option's number.
    assert held.merge({"isin": EUREX, "cficode": "OCXXXX", "underlying": "US0378331005", "characteristics": {"expiry": "2026-12-18", "strikepx": "200"}})
    assert len(held) == 2, "re-keyed, not added"
    option = held.get("OC:US0378331005:2026-12-18:200")
    assert not option["placeholder"] and option["aliascodes"] == [EUREX]
    assert option["isin"] == EUREX and "yggdryl:isin" not in option["securityids"], "a number stated, none minted"
    assert held.get(EUREX) == option and held.get_by_uuid(placeholder["uuid"]) == option
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const EUREX = 'DE0000000009'
    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: 'US0378331005' })))
    assert.ok(held.merge(statement({ isin: EUREX, cficode: 'OCXXXX' })))
    assert.equal(held.get(EUREX).placeholder, true)

    // The body arrives on the option's number.
    const characteristics = { settle: null, settle2: null, expiry: '2026-12-18', strikepx: '200', multiplier: null, exercise: null }
    assert.ok(held.merge(statement({ isin: EUREX, cficode: 'OCXXXX', underlying: 'US0378331005', characteristics })))
    const option = held.get(EUREX)
    assert.deepEqual([option.crosscode, option.aliascodes, option.placeholder], ['OC:US0378331005:2026-12-18:200', [EUREX], false])
    ```

## The underlying and the legs

An option's or a future's `underlying` and a strategy's `legs` are held as **codes**, because the key is written from them verbatim; the uuids are provided readings over them - `underlying_uuid()` and `leg_uuids()`, each the graph's cross identity of the held code - so the user's "underlying pointing to another instrument" is one read and no lookup. A leg is `Leg::new(code, ratio)`, its ratio a key byte (`2*` before its code), refused at `$.legs` for an empty, lower-case or strategy code or a ratio of zero; at most `MAX_LEGS` fit the 128-byte code. A held code is never stale: a re-key rewrites every dependent in the same fold ([Identity moves](#identity-moves)). Nothing walks the underlying: a chain of underlyings is one hop per instrument. An underlying never enters `securityids`, since the [book key](market.md#the-book-key) is the instrument's own, so `Identifier::from_key` keeps refusing `UnderlyingISIN` ([Reading a name](identifier.md#reading-a-name)).

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Cfi, Decimal};
    use yggdryl_market::{Characteristics, Instrument, Leg};
    yggdryl_market::install()?;

    let apple = Instrument::for_security(yggdryl::Isin::new("US0378331005")?)?;
    let terms = Characteristics::default()
        .with_expiry(Some("2026-12-18".parse()?))
        .with_strikepx(Some(Decimal::parse("200")?));
    let call = Instrument::for_body(Cfi::new("OCXXXX")?, None, &terms, Some(apple.get_crosscode()), &[])?;
    assert_eq!(call.underlying(), Some("US0378331005"));
    assert_eq!(call.underlying_uuid(), Some(apple.get_crossuuid()), "the code's identity, read");

    let legs = [Leg::new(call.get_crosscode(), 2)?, Leg::new(apple.get_crosscode(), 1)?];
    let covered = Instrument::for_body(Cfi::new("KEXXXX")?, None, &Characteristics::default(), None, &legs)?;
    assert_eq!(covered.get_crosscode(), "KE:2*OC:US0378331005:2026-12-18:200+US0378331005");
    assert_eq!(covered.leg_uuids().collect::<Vec<_>>(), [call.get_crossuuid(), apple.get_crossuuid()]);
    assert!(Leg::new("oc:lower", 1).is_err() && Leg::new("US0378331005", 0).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    held = Instruments()
    assert held.merge({"isin": "US0378331005"})
    assert held.merge({"cficode": "OCXXXX", "underlying": "US0378331005", "characteristics": {"expiry": "2026-12-18", "strikepx": "200"}})
    call = held.get("OC:US0378331005:2026-12-18:200")
    assert call["underlying"] == "US0378331005"
    assert held.merge({"cficode": "KEXXXX", "legs": [{"code": call["crosscode"], "ratio": 2}, {"code": "US0378331005", "ratio": 1}]})
    covered = held.get("KE:2*OC:US0378331005:2026-12-18:200+US0378331005")
    assert [leg["ratio"] for leg in covered["legs"]] == [2, 1]
    ```

Rust and Python; JavaScript reads the codes off a row's `underlying` and `legs`.

## Derived facts

An instrument the collection folds - created, or moved by a statement - carries the facts its own facts imply, where the statement left them empty: the national number its ISIN embeds ([`securityid::embedded`](../types/codes/isin.md)) - a CUSIP for `US` and `CA`, a Valor for `CH` and `LI`, a WKN for `DE` behind `000` among the instrument's identifiers, a SEDOL for `GB`, `IE`, `GG`, `JE` and `IM` behind `00` on its listings, since a SEDOL is a listing code - none for a `QY` number; each listing's `currency`, the legal tender of the country its market is in ([`Mic::country`](../types/codes/mic.md#operating-mic-and-country), [`Country::currency`](../types/codes/country.md#currency)), from the market alone - a US ISIN listed on `XETR` defaults to `EUR`, and an unlisted listing takes none; an FX pair's `currency`, its quote leg, and a derivative's, its underlying's where that states one; the country of issue, the ISIN's prefix where ISO 3166 lists it (`Instrument::country()`, the stated country first; none for an agency prefix, a minted number or a pair). A default fills only what is empty; a stated value stands, and a later statement replaces a default as it replaces any value. The origin currency is never derived. `learn` never reads an element's derived identifiers, so nothing a fill derived is learned back.

=== "Rust"

    ```rust
    use yggdryl::{Ccy, Isin, Mic};
    use yggdryl_market::{IdType, Instrument, Instruments, Listing};
    yggdryl_market::install()?;

    let mut instruments = Instruments::new();
    // A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    instruments.merge(Instrument::for_security(Isin::new("GB0002634946")?)?.with_listing(Listing::new(Some(Mic::new("XLON")?)))?)?;
    let bae = instruments.get_listing("GB0002634946", &Mic::new("XLON")?).expect("listed");
    assert_eq!(bae.get(&IdType::Sedol), Some("0263494"));
    assert_eq!(bae.currency().map(Ccy::as_str), Some("GBP"));

    // The currency is the market's country's, never the ISIN's; a stated one stands.
    instruments.merge(
        Instrument::for_security(Isin::new("US0378331005")?)?
            .with_listing(Listing::new(Some(Mic::new("XETR")?)))?
            .with_listing(Listing::new(Some(Mic::new("XNAS")?)).with_currency(Some(Ccy::new("CHF")?)))?,
    )?;
    let apple = instruments.get("US0378331005").expect("held");
    assert_eq!(apple.listing(Some(&Mic::new("XETR")?)).and_then(Listing::currency).map(Ccy::as_str), Some("EUR"));
    assert_eq!(apple.listing(Some(&Mic::new("XNAS")?)).and_then(Listing::currency).map(Ccy::as_str), Some("CHF"));
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"));
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    held = Instruments()
    # A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    assert held.merge({"isin": "GB0002634946", "listings": [{"miccode": "XLON"}]})
    assert held.get_listing("GB0002634946", "XLON") == {"miccode": "XLON", "ticker": None, "currency": "GBP", "codes": {"sedol": "0263494"}}
    # The currency is the market's country's, never the ISIN's; a stated one stands.
    assert held.merge({"isin": "US0378331005", "listings": [{"miccode": "XETR"}, {"miccode": "XNAS", "currency": "CHF"}]})
    assert [(each["miccode"], each["currency"]) for each in held.listings("US0378331005")] == [("XETR", "EUR"), ("XNAS", "CHF")]
    assert held.get("US0378331005")["securityids"]["cusip"] == "037833100"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const listing = (facts) => ({ miccode: null, ticker: null, currency: null, codes: null, ...facts })
    const held = new Instruments()
    // A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    assert.ok(held.merge(statement({ isin: 'GB0002634946', listings: [listing({ miccode: 'XLON' })] })))
    assert.deepEqual(held.getListing('GB0002634946', 'XLON'), { codes: new Map([['sedol', '0263494']]), currency: 'GBP', miccode: 'XLON', ticker: null })
    // A stated currency stands.
    assert.ok(held.merge(statement({ isin: 'US0378331005', listings: [listing({ miccode: 'XETR', currency: 'USD' })] })))
    assert.equal(held.getListing('US0378331005', 'XETR').currency, 'USD')
    ```

## The update rule

A statement - an instrument `merge` folds, or what `learn` reads off an element - folds into the instrument of its cross code fact by fact. No clock gates it: the statement's `updunix`, `firstunix` and `lastunix` are stamps, and what decides is whether the value is valid and whether it differs.

| Case | What moves |
| --- | --- |
| no instrument yet | it is created, below `max_instruments`; past it `merge` refuses naming the bound and `learn` passes the instrument over |
| a valid value, none held | filled |
| a valid value, another held | replaced - an older or undated statement included |
| the same value | nothing |
| an invalid value | nothing: a typo or a masked number under a checked code is dropped with one deduplicated warning per column, a coarse CFI code, `XXXX`, `XXX`, an unlisted country, a ticker outside one to 64 bytes are no statement |
| `cficode` | a code that [refines](../types/codes/cfi.md#two-statements-of-one-instrument) the held one refines it; a code the held one refines moves nothing; a contradicting code replaces it whole |
| an ISIN | a real one replaces a minted one, never the reverse ([ranks](identifier.md#ranks)) |
| a placeholder's body | the instrument re-keyed ([Identity moves](#identity-moves)) |
| the listing facts | `ticker`, `currency` and each listing code fill or replace on the listing of the market the statement names, a new market making a listing; a listing fact stated on no market lands on the single listing, or, of several, on none with one deduplicated warning; a listing code stated on no market of an instrument listed nowhere stays among `securityids` ([Listings](#listings)) |
| `metadata` | by key: a stated value fills a key the instrument lacks and replaces a differing one; a new key past `MAX_METADATA` passed over with one warning ([Metadata](#metadata-and-sourced-identifiers)) |
| `underlying`, `eusipacode`, `fisn`, `origccy` | instrument facts: a valid value fills or replaces; an instrument's own code is no underlying |
| a derived default | the national number and the listing currency fill what the statement left empty ([Derived facts](#derived-facts)) |
| `updunix` | the later of the two, only where a fact moved |
| `firstunix`, `lastunix` | the earlier and the later of the two, whatever else moved: `learn` states the element's `transunix` as both, so meeting a known instrument later, or replayed earlier, dirties the collection and the next `commit` writes it, while an instant between the two moves nothing |

A statement that moves nothing allocates nothing and leaves the collection clean, and a learn that moves only a stamp moves it in place.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, Isin};
    use yggdryl_market::{IdType, Instrument, Instruments};
    yggdryl_market::install()?;

    let holcim = || -> yggdryl::Result<Instrument> { Instrument::for_security(Isin::new("CH0012214059")?) };
    let mut instruments = Instruments::new();
    assert!(instruments.merge(holcim()?.with_updunix(Some(10)).try_with_code(IdType::Common, "C-1")?)?);
    // A valid value replaces whatever the time; the same one moves nothing.
    assert!(instruments.merge(holcim()?.with_updunix(Some(5)).try_with_code(IdType::Common, "C-2")?)?);
    assert!(!instruments.merge(holcim()?.with_updunix(Some(50)).try_with_code(IdType::Common, "C-2")?)?);
    // A typo under a checked code is dropped; a real code fills.
    assert!(!instruments.merge(holcim()?.try_with_code(IdType::Cusip, "037833101")?)?);
    assert!(instruments.merge(holcim()?.try_with_code(IdType::Cusip, "037833100")?)?);
    // A refining CFI code refines; a contradicting one replaces.
    assert!(instruments.merge(holcim()?.try_with_cficode(Some(Cfi::new("ESVXXX")?))?)?);
    assert!(instruments.merge(holcim()?.try_with_cficode(Some(Cfi::new("ESVUFR")?))?)?);
    assert!(instruments.merge(holcim()?.try_with_cficode(Some(Cfi::new("ESNUFR")?))?)?);
    let held = instruments.get("CH0012214059").expect("held");
    assert_eq!(held.cficode().map(|code| code.as_str().to_owned()).as_deref(), Some("ESNUFR"));
    assert_eq!(held.get(&IdType::Common), Some("C-2"));
    assert_eq!(held.updunix(), Some(10), "the later stamp, moved with a fact");
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    HOLCIM = "CH0012214059"
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "listings": [{"miccode": "XSWX", "ticker": "HOLN", "codes": {"ric": "HOLN.S"}}], "updunix": 10})
    # A valid value replaces whatever the time; the same one moves nothing.
    assert held.merge({"isin": HOLCIM, "listings": [{"miccode": "XSWX", "codes": {"ric": "HOLN.VX"}}], "updunix": 5})
    assert not held.merge({"isin": HOLCIM, "listings": [{"miccode": "XSWX", "codes": {"ric": "HOLN.VX"}}]})
    # A typo under a checked code is dropped; a real code fills.
    assert not held.merge({"isin": HOLCIM, "securityids": {"cusip": "037833101"}})
    assert held.merge({"isin": HOLCIM, "securityids": {"cusip": "037833100"}, "countrycode": "LI"})
    # A refining CFI code refines; a contradicting one replaces.
    for code in ("ESVXXX", "ESVUFR", "ESNUFR"):
        assert held.merge({"isin": HOLCIM, "cficode": code})
    row = held.get(HOLCIM)
    assert (row["cficode"], row["countrycode"], row["listings"][0]["codes"]) == ("ESNUFR", "LI", {"ric": "HOLN.VX"})
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const HOLCIM = 'CH0012214059'
    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: HOLCIM, updunix: 10n, securityids: new Map([['cusip', '037833100']]) })))
    assert.ok(!held.merge(statement({ isin: HOLCIM, securityids: new Map([['cusip', '037833100']]) })), 'the same value moves nothing')
    assert.ok(!held.merge(statement({ isin: HOLCIM, securityids: new Map([['cusip', '037833101']]) })), 'a typo moves nothing')
    for (const cficode of ['ESVXXX', 'ESVUFR', 'ESNUFR']) {
      assert.ok(held.merge(statement({ isin: HOLCIM, cficode })))
    }
    assert.equal(held.get(HOLCIM).cficode, 'ESNUFR')
    ```

## Listings

An instrument holds one listing per market it was stated on, at most `MAX_LISTINGS` (6), in MIC order, nested in its one row: `len` and `rows` both count instruments. A `Listing` is its market (`miccode`), its `ticker`, its trading `currency` and its listing `codes` - the codes `IdType::is_listing` names, a RIC, a Bloomberg symbol, an exchange symbol, a SEDOL, a FIGI, an `instrumentid` - at most `MAX_LISTING_CODES` (2) types, each its base key beside one named source. `listings(key)` answers every one, `get_listing(key, market)` one - a market the `mic` datatype refuses refused - and `remove_listing(key, market)` removes one, the instrument staying. A listing code stated on a market lives on that listing alone. One stated on no market - a RIC on a message with no `SecurityExchange(207)`, every FX pair's - lives among `securityids` under its key while the instrument is listed nowhere, and moves onto the first listing a statement then makes, so a code has one holder at every instant and `Instrument::get(&IdType)` reads `securityids` then the listings in MIC order. An index or a pair may hold the unlisted listing (`Listing::new(None)`), which the first market takes over. The seed holds one instrument listed twice: HSBC, `GB0005405286`, on `XHKG` and `XLON`.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, Forex, Mic};
    use yggdryl_market::{Characteristics, IdType, Instrument, Instruments, Listing};
    yggdryl_market::install()?;

    // A RIC stated on no market lives among the identifiers of a pair listed nowhere ...
    let spot = Instrument::for_body(Cfi::new("IFXXXP")?, Some(&Forex::new("EUR/USD")?), &Characteristics::default(), None, &[])?
        .try_with_code(IdType::Ric, "EUR=")?;
    assert_eq!(spot.securityids().get(&IdType::Ric), Some("EUR="));
    assert!(spot.listings().is_empty());
    // ... and moves onto the first listing a statement makes.
    let traded = spot.with_listing(Listing::new(Some(Mic::new("XOFF")?)))?;
    assert_eq!(traded.securityids().get(&IdType::Ric), None);
    assert_eq!(traded.listing(Some(&Mic::new("XOFF")?)).and_then(|listing| listing.get(&IdType::Ric)), Some("EUR="));
    assert_eq!(traded.get(&IdType::Ric), Some("EUR="));

    let seeded = Instruments::seeded();
    let hsbc: Vec<_> = seeded.listings("GB0005405286").iter().filter_map(Listing::miccode).map(Mic::as_str).collect();
    assert_eq!(hsbc, ["XHKG", "XLON"]);
    ```

=== "Python"

    ```python
    from yggdryl import Instruments

    HOLCIM = "CH0012214059"
    held = Instruments()
    # Two markets of one instrument are two listings of its one row.
    assert held.merge({"isin": HOLCIM, "listings": [{"miccode": "XSWX", "ticker": "HOLN", "codes": {"ric": "HOLN.S"}}]})
    assert held.merge({"isin": HOLCIM, "listings": [{"miccode": "XLON", "ticker": "HOLNL"}]})
    assert (len(held), held.rows) == (1, 1)
    assert [(each["miccode"], each["ticker"], each["currency"]) for each in held.listings(HOLCIM)] == [
        ("XLON", "HOLNL", "GBP"),
        ("XSWX", "HOLN", "CHF"),
    ], "MIC order, each its market's currency"
    assert held.get_listing(HOLCIM, "XSWX")["codes"] == {"ric": "HOLN.S"}, "a listing code is its market's"
    assert held.get_listing(HOLCIM, "XNAS") is None and held.listings("US0378331005") == []
    assert held.remove_listing(HOLCIM, "XLON")["miccode"] == "XLON"
    assert len(held) == 1 and len(held.listings(HOLCIM)) == 1, "the instrument stays"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const listing = (facts) => ({ miccode: null, ticker: null, currency: null, codes: null, ...facts })
    const HOLCIM = 'CH0012214059'
    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: HOLCIM, listings: [listing({ miccode: 'XSWX', ticker: 'HOLN' })] })))
    assert.ok(held.merge(statement({ isin: HOLCIM, listings: [listing({ miccode: 'XLON', ticker: 'HOLNL' })] })))
    assert.deepEqual([held.length, held.rows], [1, 1])
    assert.deepEqual(held.listings(HOLCIM).map((each) => [each.miccode, each.ticker, each.currency]), [
      ['XLON', 'HOLNL', 'GBP'],
      ['XSWX', 'HOLN', 'CHF'],
    ])
    assert.equal(held.removeListing(HOLCIM, 'XLON').miccode, 'XLON')
    assert.equal(held.listings(HOLCIM).length, 1, 'the instrument stays')
    ```

## Metadata and sourced identifiers

`metadata` holds the complementary facts no typed field holds, by key - the market crate's own `Metadata` map, the 25th column, a sorted `map<utf8, utf8>`: `Instrument::metadata()`, `set_metadata(key, value)`, `try_with_metadata`, `remove_metadata`, an empty key or value, one past `MAX_METADATA_WIDTH` (128 bytes) and a new key past `MAX_METADATA` (16) refused at `$.metadata`. It is merged by key - a stated value fills a key the instrument lacks and replaces a differing one, an equal one moves nothing, a key past the bound is passed over with one warning - and fed to the content code, never the key: a metadata move moves `hashcode` alone. The FIX lifecycle fills it from the descriptive fields of a message's instrument component ([Derivatives and strategies](#derivatives-and-strategies)); a seed object's extra keys and a golden file's columns no field reads land in it under their names, a text cell as it is and any other as its JSON text. The generic `learn` carries none: a market element's own metadata describes the event, not the instrument. Python and JavaScript read and merge it as the row's `metadata` mapping.

`securityids` holds every source's statement under its own `src:type` key - `ullink:isin`, `bloomberg:figi` - beside the base key the [base rule](identifier.md#ranks) fills, and the learn path, the store and a reload carry them: at most `MAX_SECURITYIDS` (32) entries, each type's base key beside one named source, a new named key past it passed over by name. A listing code under its source lands on its listing.

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Isin, Mic};
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IdKey, IdSource, IdType, Identifier, Instrument, Instruments};
    yggdryl_market::install()?;

    let sourced = |src: &str, kind: IdType| -> yggdryl::Result<IdKey> { Ok(IdKey::new(src.parse::<IdSource>()?, kind)) };
    // A bridge's statement: the ISIN under its own source, a FIGI under Bloomberg's.
    let mut stated = OrderEvent::at(1);
    stated.insert_securityid(Identifier::new(sourced("ullink", IdType::Isin)?, "US0378331005")?)?;
    stated.insert_securityid(Identifier::new(sourced("bloomberg", IdType::Figi)?, "BBG000B9XRY4")?)?;
    stated.set_miccode(Some(Mic::new("XNAS")?), true);
    let mut instruments = Instruments::new();
    assert!(instruments.learn(&stated));
    let apple = instruments.get("US0378331005").expect("keyed by the ISIN its source filled");
    assert_eq!(apple.securityids().get_from(&sourced("ullink", IdType::Isin)?), Some("US0378331005"));
    let listing = apple.listing(Some(&Mic::new("XNAS")?)).expect("the market stated");
    assert_eq!(listing.codes().get_from(&sourced("bloomberg", IdType::Figi)?), Some("BBG000B9XRY4"));

    // Complementary facts move the content code, never the key.
    let bare = Instrument::for_security(Isin::new("US0378331005")?)?;
    let described = bare.clone().try_with_metadata("issuer", "Apple Inc.")?;
    assert_eq!(described.metadata().get("issuer").map(|value| value.as_str()), Some("Apple Inc."));
    assert_eq!(described.get_crossuuid(), bare.get_crossuuid());
    assert_ne!(described.get_hashcode(), bare.get_hashcode());
    assert!(bare.clone().try_with_metadata("issuer", &"x".repeat(Instrument::MAX_METADATA_WIDTH + 1)).is_err());
    assert!(instruments.merge(described)?);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Instruments

    HOLCIM = "CH0012214059"
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim Ltd"}, "securityids": {"ullink:isin": HOLCIM}})
    row = held.get(HOLCIM)
    assert row["metadata"] == {"issuer": "Holcim Ltd"}
    assert row["securityids"]["ullink:isin"] == row["securityids"]["isin"] == HOLCIM
    assert not held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim Ltd"}}), "an equal value moves nothing"
    assert held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim AG", "sector": "materials"}})
    moved = held.get(HOLCIM)
    assert moved["hashcode"] != row["hashcode"] and moved["crossuuid"] == row["crossuuid"], "a fact, never the key"

    # A golden file's column no field reads lands in each row's metadata.
    golden = held.into_arrow_reader().read_all().append_column("desk", pa.array(["equities"]))
    assert Instruments.from_arrow_reader(golden).get(HOLCIM)["metadata"] == {"desk": "equities", "issuer": "Holcim AG", "sector": "materials"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const HOLCIM = 'CH0012214059'
    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: HOLCIM, metadata: new Map([['issuer', 'Holcim Ltd']]), securityids: new Map([['ullink:isin', HOLCIM]]) })))
    const row = held.get(HOLCIM)
    assert.deepEqual(row.metadata, new Map([['issuer', 'Holcim Ltd']]))
    assert.equal(row.securityids.get('ullink:isin'), HOLCIM)
    ```

## Matching

`resolve(element)` names the instrument an element means, and how; `fill` reads the same waterfall, a parse door its exact tiers alone. The element is the query: its `securityids` hold its ISIN, its pair, its codes and its short name (`FinancialInstrumentShortName(2737)` read as `fisn`), and its market, ticker, currency, origin currency and CFI code are its own facts, so a reconciliation job builds a market leaf with the setters rather than a second query shape.

| Tier | Key | Matched | Refused |
| --- | --- | --- | --- |
| `Isin` | a real ISIN the element states | the instrument holding it; `derived` false | `UnknownIsin { stated }` where none holds it: the cascade ends, since a lower tier would fill another instrument's codes around the one stated |
| `CrossCode` | the code the element's own facts spell - an FX spot's, from its `forex` identifier and its CFI class | the instrument of that code | `UnknownCode { stated }` where none holds it: the cascade ends |
| `Isin` | a minted number the element holds | the instrument holding it | |
| `Code(kind)` | each code of `LOOKUP_CODES` it states, in that order - `cusip`, `sedol`, `wkn`, `valor`, `figi`, `bloomberg`, `ric` first; never a currency, a country, an index name, an issuer's LEI | the one instrument holding it, its listings one answer; `derived` true | `Ambiguous { tier, codes }` where two instruments hold it: the cascade ends, a defect to name and never a reason to guess |
| `Symbology` | its ticker on its market (`get_by_ticker`) | as above | `Ambiguous` likewise |
| `Economic { similarity }` | its short name, where every tier above found nothing and it states a currency other than `XXX` | the instrument listed in that currency whose short name is the most similar (`Fisn::similarity`, `1 - levenshtein / longer length`) at or above `economic_threshold`; `derived` true | `BelowThreshold { best, code }`; `CfiConflict { stated, held, code }` where the best is of another CFI category (`X` conflicts with nothing); `CurrencyConflict { stated, held, code }` where it states another origin currency; `Ambiguous` for two equal bests |

`NoKey` is an element stating nothing any tier reads - a forward on a bare market element, whose class keys a body it cannot spell, among them - and `NoCandidate` one every tier looked for and found none of. `listing` is whether the instrument's listing on the element's market is the element's - its market is listed, or it states none and the instrument has one listing - so a match by a code on another market fills the instrument's facts and none of the listing's. An economic match is a judgement: `resolve` always weighs it, and `fill` takes it only where `is_economic_match` is set, because a derived ISIN becomes the [book key](market.md#the-book-key) the element's book and chain live under; a parse never takes one.

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Isin, Mic};
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IdKey, IdType, Identifier, Instrument, Instruments, Listing, MatchTier, Resolution, Unmatched};
    yggdryl_market::install()?;

    let mut instruments = Instruments::new();
    let listed = |isin: &str, mic: &str, ticker: Option<&str>| -> yggdryl::Result<Instrument> {
        Instrument::for_security(Isin::new(isin)?)?.with_listing(Listing::new(Some(Mic::new(mic)?)).with_ticker(ticker.map(Into::into)))
    };
    instruments.merge(listed("US0378331005", "XNAS", Some("AAPL"))?)?;
    instruments.merge(listed("GB0002374006", "XLON", Some("DGE"))?)?;
    instruments.merge(listed("DE0007164600", "XETR", None)?)?;
    let order = |codes: &[(IdType, &str)]| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(1);
        for (kind, value) in codes {
            order.insert_securityid(Identifier::new(IdKey::base(kind.clone()), value)?)?;
        }
        Ok(order)
    };

    // The ISIN decides alone, over a CUSIP naming Apple.
    let Resolution::Matched { entry, tier, derived, listing } =
        instruments.resolve(&order(&[(IdType::Isin, "DE0007164600"), (IdType::Cusip, "037833100")])?)
    else {
        panic!("SAP by its ISIN");
    };
    assert_eq!((entry.get_crosscode(), tier, derived, listing), ("DE0007164600", MatchTier::Isin, false, true));
    // An ISIN the collection lacks ends the cascade: the CUSIP is never read.
    let unknown = instruments.resolve(&order(&[(IdType::Isin, "CH0012005267"), (IdType::Cusip, "037833100")])?);
    assert!(matches!(unknown, Resolution::Unmatched(Unmatched::UnknownIsin { .. })));
    // The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    let mut by_code = order(&[(IdType::Cusip, "037833100")])?;
    by_code.set_miccode(Some(Mic::new("XLON")?), true);
    let Resolution::Matched { entry, tier, listing, .. } = instruments.resolve(&by_code) else {
        panic!("Apple by its CUSIP");
    };
    assert_eq!((entry.get_crosscode(), tier, listing), ("US0378331005", MatchTier::Code(IdType::Cusip), false));
    // The ticker on its market last.
    let mut by_ticker = OrderEvent::at(1);
    by_ticker.set_ticker(Some("DGE".into()), true);
    by_ticker.set_miccode(Some(Mic::new("XLON")?), true);
    assert!(matches!(instruments.resolve(&by_ticker), Resolution::Matched { tier: MatchTier::Symbology, .. }));
    assert_eq!(instruments.resolve(&OrderEvent::at(1)), Resolution::Unmatched(Unmatched::NoKey));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, Instruments, graph

    held = Instruments()
    held.merge({"isin": "US0378331005", "listings": [{"miccode": "XNAS", "ticker": "AAPL"}]})
    held.merge({"isin": "GB0002374006", "listings": [{"miccode": "XLON", "ticker": "DGE"}]})
    held.merge({"isin": "DE0007164600", "listings": [{"miccode": "XETR"}]})


    def order(*codes: tuple[str, str], **facts: object) -> graph.OrderEvent:
        return graph.OrderEvent(1, securityids=[Identifier(kind, value) for kind, value in codes], **facts)


    # The ISIN decides alone, over a CUSIP naming Apple.
    sap = held.resolve(order(("isin", "DE0007164600"), ("cusip", "037833100")))
    assert sap and sap.entry["crosscode"] == "DE0007164600" and (sap.tier, sap.derived, sap.listing) == ("isin", False, True)
    # An ISIN the collection lacks ends the cascade: the CUSIP is never read.
    unknown = held.resolve(order(("isin", "CH0012005267"), ("cusip", "037833100")))
    assert not unknown and (unknown.unmatched, unknown.stated) == ("UnknownIsin", "CH0012005267")
    # The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    apple = held.resolve(order(("cusip", "037833100"), miccode="XLON"))
    assert (apple.entry["crosscode"], apple.tier, apple.kind, apple.derived, apple.listing) == ("US0378331005", "code", "cusip", True, False)
    # The ticker on its market last.
    diageo = held.resolve(order(ticker="DGE", miccode="XLON"))
    assert (diageo.entry["crosscode"], diageo.tier) == ("GB0002374006", "symbology")
    assert held.resolve(order()).unmatched == "NoKey" and held.resolve(order(ticker="ZZZZ")).unmatched == "NoCandidate"
    assert Instruments.LOOKUP_CODES[:3] == ("cusip", "sedol", "wkn") and "isoccy" not in Instruments.LOOKUP_CODES
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Instruments, graph } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const listing = (facts) => ({ miccode: null, ticker: null, currency: null, codes: null, ...facts })
    const held = new Instruments()
    held.merge(statement({ isin: 'US0378331005', listings: [listing({ miccode: 'XNAS', ticker: 'AAPL' })] }))
    held.merge(statement({ isin: 'GB0002374006', listings: [listing({ miccode: 'XLON', ticker: 'DGE' })] }))
    held.merge(statement({ isin: 'DE0007164600', listings: [listing({ miccode: 'XETR' })] }))
    const order = (codes, facts = {}) => new graph.OrderEvent(1, { securityids: codes.map(([kind, value]) => new Identifier(kind, value)), ...facts })

    // The ISIN decides alone, over a CUSIP naming Apple.
    const sap = held.resolve(order([['isin', 'DE0007164600'], ['cusip', '037833100']]))
    assert.deepEqual([sap.matched, sap.entry.crosscode, sap.tier, sap.derived, sap.listing], [true, 'DE0007164600', 'isin', false, true])
    // An ISIN the collection lacks ends the cascade.
    const unknown = held.resolve(order([['isin', 'CH0012005267'], ['cusip', '037833100']]))
    assert.deepEqual([unknown.matched, unknown.unmatched, unknown.stated], [false, 'UnknownIsin', 'CH0012005267'])
    // The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    const apple = held.resolve(order([['cusip', '037833100']], { miccode: 'XLON' }))
    assert.deepEqual([apple.entry.crosscode, apple.tier, apple.kind, apple.listing], ['US0378331005', 'code', 'cusip', false])
    assert.equal(held.resolve(order([], { ticker: 'DGE', miccode: 'XLON' })).tier, 'symbology')
    ```

### By short name

=== "Rust"

    ```rust
    use yggdryl::graph::Element;
    use yggdryl::{Ccy, Fisn, Isin, Mic};
    use yggdryl_market::graph::{Market, OrderEvent};
    use yggdryl_market::{IdKey, IdType, Identifier, Instrument, Instruments, Listing, MatchTier, Resolution, Unmatched};
    yggdryl_market::install()?;

    let mut instruments = Instruments::new();
    instruments.merge(
        Instrument::for_security(Isin::new("US0378331005")?)?
            .with_listing(Listing::new(Some(Mic::new("XNAS")?)))?
            .with_fisn(Some(Fisn::new("APPLE INC./SH")?)),
    )?;
    let named = |name: &str| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(1);
        order.insert_securityid(Identifier::new(IdKey::base(IdType::Fisn), name)?)?;
        order.set_currency(Ccy::new("USD")?, true);
        Ok(order)
    };

    // One character in thirteen apart, in the currency Apple is listed in.
    let Resolution::Matched { entry, tier: MatchTier::Economic { similarity }, derived, .. } =
        instruments.resolve(&named("APPLE INC/SH")?)
    else {
        panic!("Apple by its short name");
    };
    assert_eq!((entry.get_crosscode(), derived), ("US0378331005", true));
    assert!((similarity - 12.0 / 13.0).abs() < 1e-12);
    assert!(matches!(instruments.resolve(&named("APPLE INC/SH USD CL A")?), Resolution::Unmatched(Unmatched::BelowThreshold { .. })));

    // A fill takes the match only where told to.
    let mut order = named("APPLE INC/SH")?;
    assert!(!instruments.fill(&mut order), "a judgement no fill takes unasked");
    instruments.set_economic_match(true);
    assert!(instruments.fill(&mut order));
    assert_eq!((order.get_isincode(), order.get_instcode()), (Some("US0378331005"), Some("US0378331005")));
    assert!(instruments.set_economic_threshold(1.5).is_err());
    ```

=== "Python"

    ```python
    import math

    from yggdryl import Identifier, Instruments, graph

    held = Instruments()
    held.merge({"isin": "US0378331005", "listings": [{"miccode": "XNAS"}], "fisn": "APPLE INC./SH", "cficode": "ESVUFR"})


    def named(name: str, **facts: object) -> graph.OrderEvent:
        return graph.OrderEvent(1, securityids=[Identifier("fisn", name)], **{"currency": "USD", **facts})


    # One character in thirteen apart, in the currency Apple is listed in.
    apple = held.resolve(named("APPLE INC/SH"))
    assert (apple.tier, apple.derived) == ("economic", True) and math.isclose(apple.similarity, 12 / 13)
    far = held.resolve(named("APPLE INC/SH USD CL A"))
    assert (far.unmatched, far.code) == ("BelowThreshold", "US0378331005")
    # The most similar is refused, never passed over, where it is another asset class.
    bond = held.resolve(named("APPLE INC./SH", cficode="DBFTFR"))
    assert (bond.unmatched, bond.stated, bond.held, bond.code) == ("CfiConflict", "D", "E", "US0378331005")
    assert held.economic_threshold == Instruments.DEFAULT_ECONOMIC_THRESHOLD == 0.85 and not held.is_economic_match
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, Instruments, graph } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const held = new Instruments()
    held.merge({
      uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false,
      isin: 'US0378331005', fisn: 'APPLE INC./SH', listings: [{ miccode: 'XNAS', ticker: null, currency: null, codes: null }],
    })
    const named = (name) => new graph.OrderEvent(1, { securityids: [new Identifier('fisn', name)], currency: 'USD' })
    const apple = held.resolve(named('APPLE INC/SH'))
    assert.deepEqual([apple.entry.crosscode, apple.tier, apple.derived], ['US0378331005', 'economic', true])
    assert.ok(Math.abs(apple.similarity - 12 / 13) < 1e-12)
    const far = held.resolve(named('APPLE INC/SH USD CL A'))
    assert.deepEqual([far.unmatched, far.code], ['BelowThreshold', 'US0378331005'])
    assert.deepEqual([held.economicThreshold, held.isEconomicMatch], [0.85, false])
    ```

## The product category

An instrument's `eusipacode` is the product category of a structured product: one four-digit code of EUSIPA's European Derivative Map, which the SSPA's Swiss Derivative Map - the Swiss column of the European one - numbers the same way. The first digit is the level - `1` an investment product, `2` a leverage product - the first two the group - `11` capital protection, `12` yield enhancement, `13` participation, `14` credit linked notes, `21` and `22` leverage without and with a knock-out, `23` constant leverage - and the last two the member, `99` a group's miscellaneous one. It is an instrument fact under the [update rule](#the-update-rule): a code of a category's shape fills or replaces, a number of no category's shape is dropped with one warning. Nothing lifts it: no FIX field names it and it is no identifier, so it enters no map ([Reading a name](identifier.md#reading-a-name)).

| Key | Rule |
| --- | --- |
| Value | `yggdryl_market::Eusipa` (root `eusipa.rs`), a value of its own rather than a datatype, as [`Limit`](book.md#limits) is: `Eusipa::new(code)` holds a `u16` of the shape - `1000` to `2999` - and refuses another as a value, at the value itself - no datatype - (`invalid record value at $: expected a four-digit EUSIPA product category opening with 1, an investment product, or 2, a leverage product, got 3100`); `from_text` and `FromStr` read four ASCII digits once trimmed - no sign, no leading zero, no fraction - (`invalid record value at $: expected a four-digit EUSIPA product category, got "23x0"`); `code()`, `group()` (the first two digits), `level()` (the first); `Display` the four digits; serde the number, read back through `new`; `TryFrom<u16>` and `From<Eusipa> for u16`; equality, order and hash by the code |
| Names | `name()` the English name EUSIPA's map of February 2024 gives the code, `sspa_name()` the one the SSPA's gives - its 2023 map (v.23/2) and its 2026 map (v.26/1) list the same categories - each `None` where that map lists no such member; `is_listed()` whether either does |
| Held by its shape | a code is held by its shape, never by a list: the maps are snapshots of lists that move - EUSIPA retired `1110` and added its credit linked notes, the SSPA added `1135`, `1255` and its `14xx` - and a feed may state a member either list has not caught up with, so `Eusipa::new(2301)` holds a code no map lists, `is_listed()` false |
| One code, two names | the maps share their numbering and name one code apart: `1260` is Express Certificates in the European map and a Conditional Coupon Barrier Reverse Convertible in the Swiss one, so the code, never a name, is the fact an instrument holds; every other code both maps list names one category, worded each map's way (`2200` Knock-Out Warrants, Warrant with Knock-Out) |
| The column | `eusipacode`, `int32` - the code, in a width every table format stores, Iceberg having no unsigned integer - after `legs`; Rust `Instrument::eusipacode()` and `with_eusipacode(Option<Eusipa>)`; a Python or JavaScript row crosses it as the number |
| Learned | by the FIX [lifecycle](../fix/lifecycle.md#instruments-are-learned-in-instant-order), beside a message whose code keys an instrument, off an unmapped entry a bridge keys: a key whose folded name ends with `eusipa`, `eusipacode`, `eusipacategory`, `sspa`, `sspacode` or `sspacategory`, a namespace before it passed over - `EUSIPACode`, `OMS_SSPACategory`, `X-SWX-SSPA` - whose text is four digits once trimmed. Two different categories state none, text of no category's shape states none, a key naming a name (`EUSIPA_Name`) is none, since the maps name one code apart, and so is a key naming another instrument's category - `leg`, `underlying`, `contra`, `related` or `benchmark` opening the key or spelled just before the category word, after any namespace: `UnderlyingEUSIPA`, `LegSSPACategory`, `OMS_ContraEUSIPA` - by the rule a [security type](identifier.md#reading-a-name) is refused by. `learn` never states one, and the entry stays where it arrived |
| Bindings | Python `yggdryl.Eusipa(code)` - an `int` or its text - with the properties `code`, `group`, `level`, `name`, `sspa_name` and `is_listed`, `int()` the code, `str()` the four digits, `repr` `Eusipa(2300)`, equality, order and hash by the code, and pickle; a code of no category's shape is `ValueError`, a number past `uint16` `OverflowError`, a `bool` or another type `TypeError`. JavaScript has no `Eusipa`: a row crosses the code as a number |

What each map names a code, `-` where it lists no such member - `name()` and `sspa_name()` answer `None` there:

| Code | EUSIPA, February 2024 (`name`) | SSPA, 2023 and 2026 (`sspa_name`) |
| --- | --- | --- |
| `1100` | Uncapped Capital Protection | Capital Protection Note with Participation |
| `1120` | Capped Capital Protection | - |
| `1130` | Capital Protection with Knock-Out | Capital Protection Note with Barrier |
| `1135` | - | Capital Protection Note with Twin Win |
| `1140` | Capital Protection with Coupon | Capital Protection Note with Coupon |
| `1199` | Miscellaneous Capital Protection | - |
| `1200` | Discount Certificates | Discount Certificate |
| `1210` | Barrier Discount Certificates | Barrier Discount Certificate |
| `1220` | Reverse Convertibles | Reverse Convertible |
| `1230` | Barrier Reverse Convertibles | Barrier Reverse Convertible |
| `1240` | Capped Outperformance Certificates | - |
| `1250` | Capped Bonus Certificates | - |
| `1255` | - | Conditional Coupon Reverse Convertible |
| `1260` | Express Certificates | Conditional Coupon Barrier Reverse Convertible |
| `1299` | Miscellaneous Yield Enhancement | - |
| `1300` | Tracker Certificates | Tracker Certificate |
| `1310` | Outperformance Certificates | Outperformance Certificate |
| `1320` | Bonus Certificates | Bonus Certificate |
| `1330` | Outperformance Bonus Certificates | Bonus Outperformance Certificate |
| `1340` | Twin-Win Certificates | Twin Win Certificate |
| `1399` | Miscellaneous Participation | - |
| `1400` | - | Credit Linked Notes |
| `1410` | - | Conditional Capital Protection Note with add. credit risk |
| `1420` | - | Yield Enhancement Certificate with add. credit risk |
| `1430` | - | Participation Certificate with add. credit risk |
| `1440` | Credit Linked Note - Linear | - |
| `1450` | Credit Linked Note - Equity Tranche | - |
| `1460` | Credit Linked Note - Mezz./Senior Tranche | - |
| `1499` | Miscellaneous Credit Linked Notes | - |
| `2100` | Warrants | Warrant |
| `2110` | Spread Warrants | Spread Warrant |
| `2199` | Miscellaneous | - |
| `2200` | Knock-Out Warrants | Warrant with Knock-Out |
| `2205` | Open-end Knock-Out Warrants | - |
| `2210` | Mini-Futures | Mini-Future |
| `2230` | Double Knock-Out Warrants | - |
| `2299` | Miscellaneous | - |
| `2300` | Constant Leverage Certificate | Constant Leverage Certificate |
| `2399` | Miscellaneous Constant Leverage Products | - |

=== "Rust"

    ```rust
    use yggdryl::Isin;
    use yggdryl_market::{Eusipa, Instrument, Instruments};
    yggdryl_market::install()?;

    // A category is held by its shape; each map names the codes it lists.
    let constant: Eusipa = "2300".parse()?;
    assert_eq!((constant.code(), constant.group(), constant.level()), (2300, 23, 2));
    assert_eq!(constant.name(), Some("Constant Leverage Certificate"));
    let express = Eusipa::new(1260)?;
    assert_eq!(express.name(), Some("Express Certificates"));
    assert_eq!(express.sspa_name(), Some("Conditional Coupon Barrier Reverse Convertible"));
    assert_eq!(Eusipa::new(1135)?.name(), None, "a code only the Swiss map lists");
    assert!(!Eusipa::new(2301)?.is_listed(), "a code of the shape no map lists is held");
    assert!(Eusipa::new(3100).is_err() && Eusipa::from_text("23x0").is_err());

    // An instrument fact, merged by the update rule.
    let mut instruments = Instruments::new();
    assert!(instruments.merge(Instrument::for_security(Isin::new("CH0012214059")?)?.with_eusipacode(Some(constant)))?);
    assert_eq!(instruments.get("CH0012214059").and_then(Instrument::eusipacode), Some(constant));
    ```

=== "Python"

    ```python
    from yggdryl import Eusipa, Instruments

    # A category is held by its shape; each map names the codes it lists.
    constant = Eusipa("2300")
    assert (constant.code, constant.group, constant.level) == (2300, 23, 2)
    assert constant.name == "Constant Leverage Certificate" and int(constant) == 2300 and str(constant) == "2300"
    express = Eusipa(1260)
    assert (express.name, express.sspa_name) == ("Express Certificates", "Conditional Coupon Barrier Reverse Convertible")
    assert Eusipa(1135).name is None and not Eusipa(2301).is_listed

    # An instrument fact, merged by the update rule; no category's shape is dropped.
    held = Instruments()
    assert held.merge({"isin": "CH0012214059", "eusipacode": 2300})
    assert Eusipa(held.get("CH0012214059")["eusipacode"]) == constant
    assert not held.merge({"isin": "CH0012214059", "eusipacode": 3100})
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Instruments } = require('yggdryl')

    // JavaScript has no Eusipa: a row crosses the category as its number.
    const ZERO = '00000000-0000-0000-0000-000000000000'
    const statement = (facts) => ({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, ...facts })
    const held = new Instruments()
    assert.ok(held.merge(statement({ isin: 'CH0012214059', eusipacode: 2300 })))
    assert.equal(held.get('CH0012214059').eusipacode, 2300)
    assert.ok(!held.merge(statement({ isin: 'CH0012214059', eusipacode: 3100 })))
    ```

## Persistence

A collection is bound to one store and loaded from it once: `from_holder(holder)` and `from_url(url, properties)` build an empty collection, bind it and load - the holder's own record stream, an Arrow IPC leaf, Parquet, a folder of parts, an Iceberg table or an object store, under the holder's own record options resolved once at the binding - Arrow IPC for a folder listing no record leaf, or plain text alone; a store holding nothing is an empty first run, and the collection is clean after the load. `seeded_from_holder(holder)` and `seeded_from_url(url, properties)` lay the store's rows over the [seed](#seed): a value the store states wins, a fact only the seed states stands beside it, a seed instrument the store has none of stands, and a store holding nothing loads as the seed bound to it - clean after the load, so the seed reaches the store with the first commit after something moved. `set_holder` and `try_with_holder` bind a collection already holding instruments: the store's rows are loaded and the held ones fold over them, so the collection is dirty exactly where a held one left the table differing from the store's rows.

`commit()` writes the table back only where its content differs from what the store holds - as it was loaded or last committed: an instrument added or removed, or one whose content code (`hashcode`) or window (`firstunix`, `lastunix`) moved, which `is_dirty` answers. The comparison is of content, never of writes: a fact that moves and moves back since the load - two sources disagreeing on one `metadata` key within a run, each statement replacing the other, the run ending where the store stands - is no change, so a run replayed over the same input commits nothing, and `updunix`, the instant a fact last moved, dirties nothing on its own. A dirty commit leaves the store holding exactly the snapshot (`into_arrow_reader`) - one row per instrument, in code order - whatever its layout: a leaf is rewritten in one overwrite, and truncated by a collection emptied; an Iceberg table is replaced in one atomic snapshot across every partition, an emptied collection one empty snapshot; a plain folder has its record parts of the store's encoding removed - a file that is no record part is never touched - then the snapshot laid out as one `part-0.arrows`. A clean collection touches the store with no call and answers no rows; one bound to no store refuses. A leaf names its encoding by its name, so `instruments.parquet` in a build without Parquet is refused at the binding, and so is a location inside an Iceberg table. Nothing commits implicitly: a lifecycle learns into the collection, and the caller commits. The commit is a whole overwrite and never a keyed merge, which could not delete an instrument removed or folded by a re-key.

`extend_from_arrow_reader` casts the stream once into `Instrument::field()`, a nullable column it lacks null. A row whose `crosscode` and `hashcode` are a held instrument's states exactly what is held, so only its stamps move, read off the landed leaves and never built; any other row's key is written again from its typed columns - a stored `crosscode` they do not spell refused at `$.crosscode` naming both - and folded by `merge`. A column no field reads lands in each row's `metadata` under its name. A stream lacking `crosscode` - a table laid out before the instrument row - is refused at `$.crosscode`, to be dropped and laid out afresh, and a row a column refuses is located `$[row].column`. `extend_from_handle(handle)` reads a handle's own record stream the same way without binding to it, and `into_arrow_reader()` lays the rows out one bounded batch at a time from a snapshot taken under the lock, which a learn while it streams does not move.

In a medallion pipeline the collection's commit is the stage right after the FIX-message parse: the lifecycle walk writes `silver.fix_messages` and teaches the codec's collection as it goes, and the next stage, `silver.instruments`, commits it once the refined write has drained the walk, so the commit holds every instrument the window taught, and a window that taught nothing writes nothing. The pipeline binds the seeded collection (`seeded_from_url`) to the Iceberg table `silver.record_keeping.instruments`, created from `Instrument::field()` laid out under `doris` - as Apache Doris's Iceberg catalog reads an Iceberg table, every instant at microseconds ([Apache Doris](../types/datatype.md#apache-doris)) - partitioned through `truncate(crosscode, 2)`; a table found under an older row is dropped and created afresh, since silver is replayed from bronze. Every market table - the FIX messages, the books, the orders, the quotes, the executions - carries `instcode`, which joins it to the instruments.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, Isin, Url};
    use yggdryl_market::{IdType, Instrument, Instruments};
    yggdryl_market::install()?;

    // A folder that is not there yet is an empty first run, laid out by the
    // first commit; a trailing slash is what makes it a folder.
    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-instruments-doc-{}", std::process::id()));
    let url = Url::from_location(&format!("{}/", root.display()))?;
    let none: [(&str, &str); 0] = [];
    let mut instruments = Instruments::from_url(&url, none)?;
    assert!(instruments.is_empty() && !instruments.is_dirty());
    assert_eq!(instruments.commit()?.written_rows, 0, "a clean collection writes nothing");
    instruments.merge(Instrument::for_security(Isin::new("CH0012214059")?)?.try_with_code(IdType::Common, "C-1")?)?;
    assert!(instruments.is_dirty());
    assert_eq!(instruments.commit()?.written_rows, 1);
    assert!(!instruments.is_dirty());
    assert!(root.join("part-0.arrows").is_file());
    let loaded = Instruments::from_url(&url, none)?;
    assert!(loaded.iter().eq(instruments.iter()));
    assert!(instruments.holder().is_some_and(IOBase::is_container));
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import os
    import tempfile
    from pathlib import Path

    from yggdryl import Instruments

    with tempfile.TemporaryDirectory() as folder:
        # A folder that is not there yet is an empty first run, laid out by
        # the first commit; a trailing separator is what makes it a folder.
        store = str(Path(folder) / "instruments") + os.sep
        held = Instruments.from_url(store)
        assert len(held) == 0 and not held.is_dirty
        assert held.commit().written_rows == 0, "a clean collection writes nothing"
        held.merge({"isin": "CH0012214059", "securityids": {"common": "C-1"}})
        assert held.is_dirty and held.commit().written_rows == 1 and not held.is_dirty
        assert (Path(folder) / "instruments" / "part-0.arrows").is_file()
        assert Instruments.from_url(store).get("CH0012214059") == held.get("CH0012214059")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Instruments } = require('yggdryl')

    const ZERO = '00000000-0000-0000-0000-000000000000'
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'instruments-'))
    try {
      // A folder that is not there yet is an empty first run, laid out by
      // the first commit; a trailing separator is what makes it a folder.
      const store = path.join(folder, 'instruments') + path.sep
      const held = Instruments.fromUrl(store)
      assert.deepEqual([held.length, held.isDirty, held.commit().writtenRows], [0, false, 0])
      held.merge({ uuid: ZERO, crossuuid: ZERO, crosscode: '', hashcode: 0n, crosshashcode: 0n, placeholder: false, isin: 'CH0012214059' })
      assert.equal(held.commit().writtenRows, 1)
      assert.ok(fs.existsSync(path.join(folder, 'instruments', 'part-0.arrows')))
      assert.equal(Instruments.fromUrl(store).get('CH0012214059').crosscode, 'CH0012214059')
    } finally {
      fs.rmSync(folder, { recursive: true, force: true })
    }
    ```

An Iceberg table is the store a pipeline keeps its instruments in: created from the row as Iceberg expresses it (`Instrument::field().into_scheme_compat(&Scheme::ICEBERG)`, Python `Instruments.field().into_scheme_compat("iceberg")`), the collection bound to it by its location, every lifecycle learning into it and one `commit` replacing the table's rows as one snapshot - nothing where the walk learned nothing new.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOResult, Isin, Scheme, Url};
    use yggdryl_market::{Instrument, Instruments};
    yggdryl_market::install()?;

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-instruments-lake-{}", std::process::id()));
    IcebergTable::create(
        LocalFolder::new(&root)?,
        FormatVersion::V3,
        Instrument::field().into_scheme_compat(&Scheme::ICEBERG)?,
        PartitionSpec::unpartitioned(),
    )?;
    let url = Url::from_location(&format!("{}/", root.display()))?;
    let none: [(&str, &str); 0] = [];
    let mut instruments = Instruments::from_url(&url, none)?;
    instruments.merge(Instrument::for_security(Isin::new("CH0012214059")?)?)?;
    assert_eq!(instruments.commit()?.written_rows, 1, "one snapshot holds the collection");
    assert_eq!(instruments.commit()?, IOResult::default(), "a clean collection commits nothing");
    let loaded = Instruments::from_url(&url, none)?;
    assert!(loaded.iter().eq(instruments.iter()));
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import tempfile

    from yggdryl import Instruments
    from yggdryl.iceberg import IcebergCatalog

    with tempfile.TemporaryDirectory() as folder:
        catalog = IcebergCatalog.open_or_create("lake", folder)
        namespace = catalog.namespaces.open_or_create("reference")
        table = namespace.tables.open_or_create("instruments", Instruments.field().into_scheme_compat("iceberg"))
        held = Instruments.from_url(table.url)
        held.merge({"isin": "CH0012214059"})
        assert held.commit().written_rows == 1, "one snapshot holds the collection"
        assert held.commit().written_rows == 0, "a clean collection commits nothing"
        assert catalog.table("reference.instruments").row_size() == 1
        assert Instruments.from_url(table.url).get("CH0012214059") == held.get("CH0012214059")
    ```

Rust and Python; JavaScript has no door to express a row as Iceberg does.

## The process instruments

`Instruments::from_env()` is the collection the process environment names, resolved once on the first call and shared behind one lock - Python `Instruments.from_env()`, JavaScript `Instruments.fromEnv()`. The order is fixed, first match wins:

1. a collection installed by `install_env` - Python `Instruments.install_env(held)`, JavaScript `Instruments.installEnv(held)` - before anything resolves one; installing after a resolution is a typed conflict;
2. the location `YGGDRYL_INSTRUMENTS_URI` names, trimmed: a URL of any scheme this build holds - a local folder or leaf, an Iceberg table's folder, an object store - or a bare path, a leading `~/` read under the home directory, the home itself refused; an empty value reads as unset, and a store holding nothing yet is an empty first run;
3. `~/.config/yggdryl/instruments/`, a folder of Arrow IPC parts the first `commit` lays out;
4. with no home directory, the [seed](#seed) bound to nothing.

Every store is laid over the seed: its rows fold over the seed's by the [update rule](#the-update-rule), so a value the store states wins and a seed instrument it has none of stands, clean after the load. A location naming a scheme this build has no backend for, a store that cannot be read, a table laid out before the instrument row or a row the collection refuses is an error, never the seed alone, and the default stays unresolved so the next call retries. `FixCodec::from_env()` - Python `FixCodec.from_env()`, JavaScript `FixCodec.fromEnv()` - is the one codec constructor that attaches it; `FixCodec::new` and the bindings' constructors attach none unless handed one. Nothing commits implicitly.

```bash
YGGDRYL_INSTRUMENTS_URI=s3://bucket/instruments/   # an object store folder of IPC parts
YGGDRYL_INSTRUMENTS_URI=~/warehouse/instruments/   # an Iceberg table's folder, replaced in one snapshot
YGGDRYL_INSTRUMENTS_URI=/data/instruments.arrows   # one IPC leaf
```

## Bounds

| Bound | Value |
| --- | --- |
| `max_instruments` | `Instruments::DEFAULT_MAX_INSTRUMENTS` (16,384) unless `with_max_instruments` says otherwise; no eviction |
| Memory | each instrument is charged its worst case - every bound below at its widest, one listing inline - at most 32 KiB, so 512 MiB at the default bound |
| Per instrument | `MAX_CODE_WIDTH` (128) bytes of code; `MAX_SECURITYIDS` (32) security identifiers, `Instruments::MAX_EQUIVALENTS` (12) base types among them; `MAX_LISTINGS` (6) listings, each `MAX_LISTING_CODES` (2) code types and a ticker of one to `Listing::MAX_TICKER_WIDTH` (64) bytes; `MAX_ALIASES` (4) former codes; `MAX_LEGS` legs, what the code holds; `MAX_METADATA` (16) metadata entries of `MAX_METADATA_WIDTH` (128) bytes |
| Past a bound | a statement past an entry bound is passed over by name, never a refusal of the instrument; a new instrument past `max_instruments` is passed over by `learn` with one warning per collection and refused by `merge`, `extend_from_*` and a load, naming the bound - which fails `from_env` on a store holding more |

## Edges

- Learning is the ordered lifecycle's, never a parse's: a parse fills derived identifiers from the table its door fixed - the ISIN a ticker names on its market, every equivalent, the pair, the minted number - and nothing else, and writes `instcode` only where the message alone spells it, so the message's identity, its wire and its row are the same with and without a table. A learn while a door reads reaches no message of that reading. Share one collection across walks run one after another; walks run at once interleave their learning.
- A fill is a derivation: it never writes `CFICode(461)`, `Currency(15)` or any FIX field, and a held `instcode` stands.
- A filled ISIN moves the [book](book.md) an element stands in: its [book key](market.md#the-book-key) is the ISIN once a fill derives it, so a ticker-only statement joins its instrument's book from the first message the parse filled - as long as the table the door fixed already knew it. A ticker-only security with no instrument holds no `instcode`.
- A ticker leads to an instrument only on the market it was listed on, or, where none is, to the one instrument listing it on no market; where the element states no market, to the one listing it anywhere, so one ticker on two markets of two instruments leads to none. A code of `LOOKUP_CODES` leads to its instrument whatever listing holds it, and to none where two instruments hold it.
- Every learn states `firstunix` and `lastunix`, so a walk that meets a known instrument outside the instants already learned dirties the collection and the next `commit` writes the snapshot.
- `origccy` fills an element only where it holds none and is never derived.
- An economic match is weighed over the instruments listed in the element's currency, linear in them; a parse door never runs it.
- A placeholder's re-key moves its identity once, and the identity it had answers through `get_by_uuid`; a market row filled before the re-key carries the old code, an alias of the instrument.
- A minted number's collision is refused only in the lifecycle, which holds the table: the parse mints unconditionally, so two bronze rows of two colliding instruments - about `1.9e-6` for 16,384 instruments - share a number the silver rows lose.
- A whole overwrite is the one commit: last writer wins on a file store, and a crash mid-write leaves a torn leaf that fails the next load.

## Performance

`graph/instrument` in the market bench over a collection of 4,096 instruments - a known and a new learn, a fill by ISIN, by ticker and a miss, the snapshot stream drained, a load and an IPC round trip - and `fix/pipeline/decoded_lifecycle` with and without a collection the codec shares (`decoded_lifecycle_shared_registry`). No number is stated here until the release run regenerates them; the allocation and call-count claims are pinned in `rust/market/tests/allocations.rs` and `rust/market/tests/iobase_calls.rs` (`mod instrument`).

```bash
cargo bench -p yggdryl-market --bench graph -- 'graph/instrument'
cargo bench -p yggdryl-fix --bench fix -- '^fix/pipeline/decoded_lifecycle(_shared_registry)?$'
```

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl-market --test root -- instrument characteristics listing
    cargo test -p yggdryl-market --test instrument
    cargo test -p yggdryl-market --test instrument --features "internals parquet iceberg"
    cargo test -p yggdryl-market --test allocations -- instrument
    cargo test -p yggdryl-fix --test root -- enrich forex batch
    cargo bench -p yggdryl-market --bench graph -- 'graph/instrument'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_instrument.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/instrument.test.js
    ```
