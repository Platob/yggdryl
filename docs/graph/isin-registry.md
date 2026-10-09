# IsinRegistry

`IsinRegistry` is a table of instruments keyed by ISIN: one row per ISIN and market - a listing - of every fact it is known by - its detailed CFI code, its country of issue, the currency pair an FX or referential number names, the instrument it is written on, the product category of a structured product, the currency it was issued in where one is stated, the market, ticker and trading currency of the listing, and one code per `SecurityIDSource(22)` type, a RIC, a Bloomberg symbol, a CUSIP, a FIGI among them. The instrument's facts are every listing's; a listing's facts are its own row's. A lifecycle learns each market element's statements into it and fills what a later element of the same instrument leaves unsaid; a parse fills the security identifiers a message leaves unsaid from the table its door fixed. The ISIN is the one key: a ticker leads back to it on its market, and so does every code `IsinRegistry::LOOKUP_CODES` names - a CUSIP, a SEDOL, a FIGI, a RIC, a Bloomberg symbol - while the codes it does not name are equivalents the ISIN fills. `resolve` names the row an element means by that waterfall, and, where nothing exact names one, scores its short name in its currency ([Matching](#matching)). A valid stated value fills and replaces whatever the time; no clock gates a merge. The registry is bound to the store it was loaded from - an Arrow IPC leaf, a folder of parts, Parquet, an Iceberg table, an object store - and commits its table back as one snapshot only where it moved.

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::IsinRegistry` and `yggdryl::IsinEntry` (root `isin_registry.rs`, with `isin_registry/store.rs`, `isin_registry/env.rs` and `isin_registry/seed.rs`); Python `yggdryl.IsinRegistry`; JavaScript `IsinRegistry`. A binding holds one table behind one lock and crosses a row as a `dict` / plain object of its columns; `IsinEntry` is Rust-only |
| Key | a real ISIN - closing under a listed prefix, `IdType::Isin.is_real`, `XT` for a referential instrument among the agency prefixes - the instrument's one atomic code; a `ZZ` number, a masked one or a typo keys no row. An instrument holds one listing row per market its listing facts were stated on, in MIC order, or - while no market is known - the unlisted row alone, which the first listing takes over. A ticker leads back to its listing through an exact inverse index the rows keep, by ISIN and market, gated by the market the listing was stated on; every code of `IsinRegistry::LOOKUP_CODES` - twenty types in cascade order, `cusip`, `sedol`, `wkn`, `valor`, `figi`, `bloomberg`, `ric` first - leads back through a code index the rows keep the same way, one slot per ISIN, so the listings of one instrument are one answer and two instruments holding one code are none; a currency, a country, an index name, a synthetic code, an issuer's LEI and a reference many instruments share key nothing ([Matching](#matching)) |
| Row | `IsinEntry::field()`, the non-null struct `isinregistry`: `isin` (`isin`, required), `updunix` (`datetime64(ns, UTC)`, when the statement that last moved a fact of the instrument happened - a stamp, deciding nothing), `firstunix` (`datetime64(ns, UTC)`, the earliest instant of an event the registry learned the instrument from - moved back by an earlier learn, never forward; a stamp), `lastunix` (`datetime64(ns, UTC)`, the latest instant of an event the registry learned the instrument from - moved by every learn, whatever else it taught; a stamp), `cficode` (`cfi`, detailed only), `countrycode` (`country`, the stated country of issue, listed only), `forexcode` (`forex`, the pair an FX or referential number names), `underlyingisin` (`isin`, the instrument it is written on - FIX's underlying - real and never the row's own ISIN), `eusipacode` (`int32`, the [product category](#the-product-category) of a structured product, a four-digit EUSIPA code), `miccode` (`mic`, the market the row is the listing of, never `XXXX`; none for the unlisted row), `ticker` (`utf8`, one to 64 bytes), `fisn` (`fisn`, the ISO 18774 Financial Instrument Short Name the instrument states, FIX's `FinancialInstrumentShortName(2737)`), `currency` (`ccy`, the listing's trading currency, never `XXX`), `origccy` (`ccy`, the currency the instrument was issued in - a depositary receipt's or a share class's listed in another currency - held only where stated, never derived), then one column per `SecurityIDSource(22)` type but the ISIN, in code-set order - `cusip`, `sedol`, `quik`, `ric`, `isoccy`, `isoctry`, `exchsymb`, `cta`, `bloomberg`, `wkn`, `dutch`, `valor`, ... `dti` - each typed by `IdType::value_dtype` (`ric`, `bbg`, `figi`, `lei`, `dti`, `utf8` for a type with no datatype of its own): 46 columns, at most `IsinRegistry::MAX_EQUIVALENTS` (12) codes stated per row. The root declares `PARTITION:by` `["truncate(isin, 2)"]` - the country prefix of the ISIN every row carries, Iceberg's native truncation of the key, which stores no column, so every listing of one ISIN lands in one partition - and `SORT:by` `["isin","miccode"]`, the order the snapshot streams in: an Iceberg table created from the field partitions by country, while a leaf stays flat and a plain folder is laid out by the `column=value` layout it spells ([Persistence](#persistence)) |
| Instrument and listing columns | `updunix`, `firstunix`, `lastunix`, `cficode`, `countrycode`, `forexcode`, `underlyingisin`, `eusipacode`, `fisn`, `origccy` and every code `IdType::is_listing` does not name are the instrument's: a statement of one folds into every listing row of the ISIN, so its rows agree on them, and they fill on any market; `miccode`, `ticker`, `currency` and the listing codes - `ric`, `bloomberg`, `exchsymb` (SIX's symbol on `XSWX` among them), `cta`, `sedol`, `figi`, `mktassigned`, `fim`, `umtf` - are one listing's, held on the row of its market |
| Derived, never stored | the country of issue the ISIN's prefix names where ISO 3166 lists it (`IsinEntry::country()`, the stated country first) |
| Derived facts | wherever the registry folds a row - creates it, or a statement moves it - it derives, once, after the statement has landed, the facts the row's own columns imply into the columns the statement left empty: the national number its ISIN embeds ([`securityid::embedded`](../types/codes/isin.md)) in its equivalent column - `cusip` for `US` and `CA`, `sedol` for `GB`, `IE`, `GG`, `JE` and `IM` behind `00`, `wkn` for `DE` behind `000`, `valor` for `CH` and `LI` - passed over where the row has no room left, and the `currency`, the legal tender ISO 4217 gives the country its market is in (`Mic::country`, `Country::currency`). The currency is a listing's, so it comes from the market alone, never from the ISIN's country: a row of no market, or of a market of no single country such as `XOFF`, takes none, and a market arriving later sets it. A default never displaces a statement: a statement derives nothing - `merge(entry)` folds what the entry states - so a default only fills what the held row leaves empty; a later statement replaces one as it replaces any value; and each listing derives its own, after every fold: its currency from its market, the code the ISIN embeds - a derived SEDOL among them - on every listing. A derived SEDOL is a listing code, filled into an element on the row's market alone. `learn` never reads an element's derived identifiers, so none of this is learned back from what a fill derived |
| Seed | `IsinRegistry::seeded()` - Python and JavaScript `IsinRegistry.seeded()` - is a registry holding the common instruments `config/isin/instruments.json` states: one JSON array of listings sorted by ISIN then market - an instrument listed on two markets, HSBC in London and Hong Kong, one row per market - each row keyed as the registry's columns are - `isin`, `ticker`, `miccode` (absent for an index, which trades on no market), `currency`, `countrycode` (absent where an index of a supranational prefix measures no one country), `cficode` in ISO 10962:2021 (`TIEXXX` an equity index), where FIRDS spells one, `fisn`, and, on the five Irish fund share classes issued in US dollars, `origccy` - and nothing else: 209 listing rows of 208 instruments, 182 of the rows stating a short name. It is the one file maintained by hand; the crate embeds its copy `rust/src/isin_registry/seed.json`, inside the crate's package so a published crate and a source distribution carry it, which `python scripts/check_isin_seed.py --sync` writes byte for byte. The copy is read once per process through the column rule of `extend_from_arrow_reader`, each row folded by `merge`, so a seed row is an ordinary statement and its [derived facts](#derived-facts) are any row's; the registry is clean, bound to no store, and shares the one table until a write moves it. `IsinRegistry::new()` holds none of it; `seeded_from_url(url, properties)` / `seeded_from_holder(holder)` - Python `IsinRegistry.seeded_from_url(location, max_instruments=..., **properties)`, JavaScript `IsinRegistry.seededFromUrl(location, maxInstruments?, properties?)` - lay a store over it ([Persistence](#persistence)). `scripts/check_isin_seed.py` checks the file in the change that edits it - its shape, every ISIN's ISO 6166 check digit, uniqueness on the ISIN and the market and order by both, a row of no market alone under its ISIN, the rows of one ISIN agreeing on the instrument's facts, a country equal to the prefix but under an agency prefix, every MIC in the ISO 10383 table, a 2021 CFI code, a short name's shape, the crate's copy equal to the file - and `rust/tests/isin_registry/seed.rs` pins what the embedded copy holds and that it is the file's bytes |
| `merge(entry)` | folds one row into the listings of its ISIN by the [update rule](#the-update-rule) - the instrument's facts into every listing, the listing facts into the row of the market it names; whether anything moved. Refuses an ISIN that is not real (`expected an ISIN some agency numbers, got ...`) and a new ISIN past `max_instruments` |
| `learn(event)` | reads what a dated market element states about its instrument, keyed by its stated real ISIN alone, its market naming the listing row, at its `transunix` - stated as `firstunix` and `lastunix` on every learn, the earlier and the later kept, so an element stating its ISIN alone is learned, a new instrument's row made and a known one's instant moved: its detailed CFI code, its market but `XXXX`, its ticker, its currency but `XXX` - unless the element holds a currency pair, where `Currency(15)` is the dealt currency and no listing's - its origin currency but `XXX` (`get_origccy`, none for a pair either; on a FIX message the crate field it stated, never what a fill wrote), the pair it states as a `forex` identifier, the short name it states as a `fisn` identifier, and each equivalent its map answers with a real value ([`IdType::is_real`](identifier.md#ranks)); never a derived code, a masked number or a typo, an `Other` type or an `instrumentid`, and never an underlying or a product category - those are the [lifecycle's](../fix/lifecycle.md#instruments-are-learned-in-instant-order) reading of a FIX message ([The underlying](#the-underlying), [The product category](#the-product-category)). A new ISIN past the bound is skipped with one warning per registry; a known one keeps learning |
| `fill(element)` | from the listing row `resolve` names - its real ISIN, a miss ending the fill: the row of its market, else the ISIN's single row, else, its market unstated or listing none of several, the first, for the instrument's facts alone; else the first [lookup code](#matching) it holds, else its ticker on its market (`get_by_ticker`), else, only where `is_economic_match` says so, its short name in its currency, each deriving the ISIN first - over none, or over a number [ranking](identifier.md#ranks) below it, a masked one or a typo: each equivalent of a type it holds nothing of as a `derived` identifier (its base key filled, so `map['valor']` answers), the listing codes only where its market - none and `XXXX` unstated - is the row's or either is unstated, the pair, the short name, the ticker on the same market, its CFI code where it states none or the row's [refines](../types/codes/cfi.md#two-statements-of-one-instrument) it, the currency only where both markets are stated and equal, the ticker is the row's and it states none, and the instrument's `origccy` where it holds none - never over a statement, and never the default `origin_currency()` reads; the element is finalized where anything moved. Nothing reaches a FIX field or the wire |
| `enrich(event)` | `learn`, then `fill` |
| `resolve(element)` | the [matching waterfall](#matching) without a write: `Resolution::Matched { entry, tier, derived, listing }` - the row, the `MatchTier` (`Isin`, `Code(IdType)`, `Symbology`, `Economic { similarity }`), whether the ISIN was derived and whether the row's listing facts are the element's - or `Resolution::Unmatched(Unmatched)`: `NoKey`, `UnknownIsin`, `NoCandidate`, `Ambiguous`, `CfiConflict`, `CurrencyConflict`, `BelowThreshold`, each carrying what it names. Python `resolve(element) -> Resolution`, attributes `matched`, `entry`, `tier`, `kind` (the code type, the identifier's kind), `similarity`, `derived`, `listing`, `unmatched`, `isins`, `stated`, `held`, `best`, `isin`; JavaScript `resolve(element)` a plain `IsinResolution` object of the same fields. Any market leaf, a `MarketData` or a `FixMsg` |
| Economic settings | `economic_threshold` (`DEFAULT_ECONOMIC_THRESHOLD`, `0.85`), `set_economic_threshold` - NaN and anything outside `(0, 1]` refused at `$.economic_threshold`, moving nothing - `try_with_economic_threshold`; `is_economic_match` (`false`), `set_economic_match`, `with_economic_match`: whether a fill takes an economic match. The registry's, never the store's: a load or a commit moves neither. Python `economic_threshold` / `set_economic_threshold`, `is_economic_match` / `set_economic_match`; JavaScript `economicThreshold` / `setEconomicThreshold`, `isEconomicMatch` / `setEconomicMatch` |
| Reads | `get(isin)` borrows the first listing in MIC order - the unlisted row where that is all - allocation-free; `listings(isin)` every listing row in MIC order, empty where unknown; `get_listing(isin, market)` the row on one market; `get_by_ticker(ticker, market)` the one listing stating the ticker on `market`, else the one stating it on no market, and with `market` unstated - none and `XXXX` - the one stating it on any, two rows answering being ambiguous and answering none - Python `get_by_ticker(ticker, market=None)`, JavaScript `getByTicker(ticker, market?)`, a market the `mic` datatype refuses refused; `get_by_code(kind, value, market)` the row a code of `LOOKUP_CODES` names, read as its type stores it - the one ISIN holding it, its row on `market`, else the one row holding the code, else its single row, else its first - two ISINs, a type no lookup reads and a value its type refuses answering none; Python `get_by_code(kind, value, market=None)` and the class attribute `LOOKUP_CODES`, JavaScript `getByCode(kind, value, market?)` and `IsinRegistry.lookupCodes()`; `iter()` every listing row in ISIN then MIC order; `len` the instruments, `rows` the listing rows, `is_empty`, `max_instruments`, `is_dirty`; `remove(isin)` every listing, answered in MIC order, `remove_listing(isin, market)` one, the instrument going with its last; `clear()`; Python spells them alike - `rows` a property - and JavaScript `getListing`, `removeListing` and the `rows` getter ([Listings](#listings)) |
| Persistence | `from_holder(holder)` / `from_url(url, properties)` bind and load the store's rows alone - Python `IsinRegistry.from_url(location, max_instruments=..., **properties)`, JavaScript `IsinRegistry.fromUrl(location, maxInstruments?, properties?)`; `seeded_from_holder(holder)` / `seeded_from_url(url, properties)` - Python `IsinRegistry.seeded_from_url(location, max_instruments=..., **properties)`, JavaScript `IsinRegistry.seededFromUrl(location, maxInstruments?, properties?)` - lay them over the [seed](#seed), the store's rows winning, clean after the load; `set_holder` / `try_with_holder` bind a registry already holding rows, folding them over the store's; `commit()` writes the table back as one snapshot only where it moved; `extend_from_handle(handle)` / `from_arrow_reader(reader)` / `extend_from_arrow_reader(reader)` read any record stream without binding, and `into_arrow_reader()` is the snapshot stream of every listing row under `IsinEntry::field()`, whose `PARTITION:by` `truncate(isin, 2)` partitions an Iceberg table created from it by the ISIN's country prefix ([Persistence](#persistence)) |
| The process's own | `from_env()` resolves once from `YGGDRYL_ISIN_REGISTRY_URI`, else `~/.config/yggdryl/isin/`, else no store, laid over the [seed](#seed) - the store's rows fold over the seed's, so a value the store states wins and a seed row it has none of stands, clean after the load - shared behind one lock; `install_env` installs one first; `FixCodec::from_env()` attaches it, `FixCodec::new` attaches none, and nothing commits but the caller ([The process registry](#the-process-registry)) |
| Sharing | `FixCodec::with_isin_registry(Arc<Mutex<IsinRegistry>>)` shares one table with every lifecycle the codec runs and every parse door it opens: a parse door fixes the table once as it opens, under one lock on the calling thread, and fills derived identifiers from it on every worker; a lifecycle learns and fills under one lock per message. Python `FixCodec(..., isin_registry=registry)`, JavaScript `new fix.FixCodec(registry, { isinRegistry })`, each with an `isin_registry` / `isinRegistry` getter answering the caller's own table |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{Event, Market, OrderEvent};
    use yggdryl::{Country, IdKey, IdType, Identifier, IsinRegistry, Mic};

    // A golden file's columns are read by any spelling of the fact they name.
    let schema = Arc::new(Schema::new(vec![
        Field::new("ISIN", DataType::Utf8, false),
        Field::new("RIC", DataType::Utf8, true),
        Field::new("BloombergSymbol", DataType::Utf8, true),
        Field::new("CFI", DataType::Utf8, true),
        Field::new("MIC", DataType::Utf8, true),
        Field::new("Ticker", DataType::Utf8, true),
        Field::new("Currency", DataType::Utf8, true),
    ]));
    let golden = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec!["CH0012214059"])),
            Arc::new(StringArray::from(vec!["HOLN.S"])),
            Arc::new(StringArray::from(vec!["HOLN SW Equity"])),
            Arc::new(StringArray::from(vec!["ESVUFR"])),
            Arc::new(StringArray::from(vec!["XSWX"])),
            Arc::new(StringArray::from(vec!["HOLN"])),
            Arc::new(StringArray::from(vec!["CHF"])),
        ],
    )?;
    let mut registry = IsinRegistry::from_arrow_reader(batch_reader(schema, vec![golden]))?;
    assert_eq!(registry.len(), 1);
    let row = registry.get("CH0012214059").expect("the row");
    assert_eq!(row.country(), Some(Country::new("CH")?), "the prefix, stated nowhere");
    assert_eq!(row.currency().map(|code| code.as_str()), Some("CHF"));

    // An order naming the ISIN is filled with the rest; a RIC alone leads back to it.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    assert!(registry.enrich(&mut order));
    assert_eq!(order.get_securityids().get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert!(order.get_securityids().is_derived(&IdType::Ric));
    assert_eq!(order.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
    let mut by_ric = OrderEvent::at(1_700_000_000_000_000_000);
    by_ric.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
    assert!(registry.fill(&mut by_ric));
    assert_eq!(by_ric.get_isincode(), Some("CH0012214059"));
    assert!(by_ric.get_securityids().is_derived(&IdType::Isin), "derived from the RIC");

    // An order naming only its ticker on the listing's market finds the row
    // through the ticker index and takes the currency too; another market's
    // listing is another row.
    let mut quoted = OrderEvent::at(1_700_000_000_000_000_000);
    quoted.set_ticker(Some("HOLN".into()), true);
    quoted.set_miccode(Some(Mic::new("XSWX")?), true);
    assert!(registry.enrich(&mut quoted));
    assert_eq!(quoted.get_isincode(), Some("CH0012214059"));
    assert_eq!(quoted.get_currency().as_str(), "CHF");
    assert!(registry.get_by_ticker("HOLN", Some(&Mic::new("XLON")?)).is_none());

    // The table streams out as a snapshot, one row per listing.
    let back = IsinRegistry::from_arrow_reader(registry.into_arrow_reader()?)?;
    assert_eq!(back.get("CH0012214059"), registry.get("CH0012214059"));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import IsinRegistry

    # A golden file's columns are read by any spelling of the fact they name.
    golden = pa.table({
        "ISIN": ["CH0012214059"],
        "RIC": ["HOLN.S"],
        "BloombergSymbol": ["HOLN SW Equity"],
        "CFI": ["ESVUFR"],
        "MIC": ["XSWX"],
        "Ticker": ["HOLN"],
        "Currency": ["CHF"],
    })
    registry = IsinRegistry.from_arrow_reader(golden)
    assert len(registry) == 1
    row = registry.get("CH0012214059")
    assert row is not None and (row["isin"], row["bloomberg"], row["cficode"], row["ticker"], row["currency"]) == (
        "CH0012214059",
        "HOLN SW Equity",
        "ESVUFR",
        "HOLN",
        "CHF",
    )
    assert registry.get_by_ticker("HOLN", "XSWX") == row and registry.get_by_ticker("HOLN", "XLON") is None

    # The table streams out as a snapshot, one row per listing, in ISIN then market order.
    table = registry.into_arrow_reader().read_all()
    assert table.num_rows == 1 and table.column("ric").to_pylist() == ["HOLN.S"]
    assert IsinRegistry.from_arrow_reader(table).get("CH0012214059") == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IsinRegistry } = require('yggdryl')

    // A golden file's columns are read by any spelling of the fact they name.
    const golden = new arrow.Table({
      ISIN: arrow.vectorFromArray(['CH0012214059'], new arrow.Utf8()),
      RIC: arrow.vectorFromArray(['HOLN.S'], new arrow.Utf8()),
      BloombergSymbol: arrow.vectorFromArray(['HOLN SW Equity'], new arrow.Utf8()),
      CFI: arrow.vectorFromArray(['ESVUFR'], new arrow.Utf8()),
      MIC: arrow.vectorFromArray(['XSWX'], new arrow.Utf8()),
      Ticker: arrow.vectorFromArray(['HOLN'], new arrow.Utf8()),
      Currency: arrow.vectorFromArray(['CHF'], new arrow.Utf8()),
    })
    const registry = IsinRegistry.fromArrowReader(BatchReader.from(golden))
    assert.equal(registry.length, 1)
    const row = registry.get('CH0012214059')
    assert.deepEqual([row.isin, row.bloomberg, row.cficode, row.ticker, row.currency], ['CH0012214059', 'HOLN SW Equity', 'ESVUFR', 'HOLN', 'CHF'])
    assert.deepEqual(registry.getByTicker('HOLN', 'XSWX'), row)
    assert.equal(registry.getByTicker('HOLN', 'XLON'), null)

    // The table streams out as a snapshot, one row per listing, in ISIN then market order.
    const back = IsinRegistry.fromArrowReader(registry.intoArrowReader())
    assert.deepEqual(back.get('CH0012214059'), row)
    ```

## Seed

`IsinRegistry::seeded()` - Python `IsinRegistry.seeded()`, JavaScript `IsinRegistry.seeded()` - answers a registry holding the common instruments the crate ships: large equities on their primary listing - HSBC in Hong Kong besides London, one listing row each - and the main equity indices, each with its ticker, market, currency, country, CFI code and, where FIRDS spells one, its short name - 208 instruments in 209 listing rows (`len` 208, `rows` 209), 182 of the rows stating a short name. The source is `config/isin/instruments.json`, the one file maintained by hand; the crate embeds its copy `rust/src/isin_registry/seed.json`, which `python scripts/check_isin_seed.py --sync` writes byte for byte. Each seed row is folded by `merge` as an ordinary statement, so it carries the [derived facts](#derived-facts) any row carries - Apple's CUSIP below is one, stated nowhere in the file. The registry is clean and bound to no store, so `commit` refuses; each call answers a registry of its own, sharing the one parsed table until a write moves it. `IsinRegistry::new()` holds none of it; `seeded_from_url` - Python `IsinRegistry.seeded_from_url`, JavaScript `IsinRegistry.seededFromUrl` - lays a store the caller names over it ([Persistence](#persistence)), and [`from_env`](#the-process-registry) the store the environment names.

=== "Rust"

    ```rust
    use yggdryl::{IdType, IsinRegistry, Mic};

    let registry = IsinRegistry::seeded();
    assert!(!registry.is_empty() && !registry.is_dirty() && registry.holder().is_none());
    assert!(IsinRegistry::new().is_empty(), "a registry built by hand holds none of it");

    let apple = registry.get("US0378331005").expect("seeded");
    assert_eq!(apple.ticker(), Some("AAPL"));
    assert_eq!(apple.miccode(), Some(&Mic::new("XNAS")?));
    assert_eq!(apple.currency().map(|code| code.as_str()), Some("USD"));
    assert_eq!(apple.fisn().map(|name| name.as_str()), Some("APPLE INC/SH SH"));
    assert_eq!(apple.cficode().map(|code| code.as_str()), Some("ESVUFR"));
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"), "the CUSIP its ISIN embeds");
    assert_eq!(registry.get_by_ticker("AAPL", Some(&Mic::new("XNAS")?)), Some(apple));
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    registry = IsinRegistry.seeded()
    assert len(registry) > 0 and not registry.is_dirty
    assert len(IsinRegistry()) == 0, "a registry built by hand holds none of it"

    apple = registry.get("US0378331005")
    assert apple is not None
    assert (apple["ticker"], apple["miccode"], apple["currency"], apple["fisn"], apple["cficode"]) == (
        "AAPL",
        "XNAS",
        "USD",
        "APPLE INC/SH SH",
        "ESVUFR",
    )
    assert apple["cusip"] == "037833100", "the CUSIP its ISIN embeds"
    assert registry.get_by_ticker("AAPL", "XNAS") == apple
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const registry = IsinRegistry.seeded()
    assert.ok(registry.length > 0)
    assert.equal(registry.isDirty, false)
    assert.equal(new IsinRegistry().length, 0, 'a registry built by hand holds none of it')

    const apple = registry.get('US0378331005')
    assert.deepEqual(
      [apple.ticker, apple.miccode, apple.currency, apple.fisn, apple.cficode],
      ['AAPL', 'XNAS', 'USD', 'APPLE INC/SH SH', 'ESVUFR'],
    )
    assert.equal(apple.cusip, '037833100', 'the CUSIP its ISIN embeds')
    assert.deepEqual(registry.getByTicker('AAPL', 'XNAS'), apple)
    ```

## Derived facts

A row the registry folds - created, or moved by a statement - carries the facts its own columns imply, in the columns the statement left empty: the national number its ISIN embeds ([`securityid::embedded`](../types/codes/isin.md)) - a CUSIP for `US` and `CA`, a SEDOL for `GB`, `IE`, `GG`, `JE` and `IM` behind `00`, a WKN for `DE` behind `000`, a Valor for `CH` and `LI` - and the `currency`, the legal tender of the country its market is in ([`Mic::country`](../types/codes/mic.md#operating-mic-and-country), [`Country::currency`](../types/codes/country.md#currency)). The currency is the listing's, so it comes from the market alone, never from the ISIN's prefix: a US ISIN listed on `XETR` defaults to `EUR`, and a row of no market takes none until a market arrives. A default fills only what is empty; a stated value stands, and a later statement replaces a default as it replaces any value. Each listing derives its own: a listing on another market takes that market's currency, and every listing the code its ISIN embeds - a derived SEDOL among them. `learn` never reads an element's derived identifiers, so nothing a fill derived is learned back.

=== "Rust"

    ```rust
    use yggdryl::{Ccy, IdType, Isin, IsinEntry, IsinRegistry, Mic};

    let mut registry = IsinRegistry::new();
    // A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    registry.merge(IsinEntry::new(Isin::new("GB0002634946")?).with_miccode(Some(Mic::new("XLON")?)))?;
    let row = registry.get("GB0002634946").expect("the row");
    assert_eq!(row.get(&IdType::Sedol), Some("0263494"));
    assert_eq!(row.currency().map(|code| code.as_str()), Some("GBP"));

    // The currency is the market's country's, never the ISIN's; no market, none.
    registry.merge(IsinEntry::new(Isin::new("US0378331005")?).with_miccode(Some(Mic::new("XETR")?)))?;
    let apple = registry.get("US0378331005").expect("the row");
    assert_eq!(apple.currency().map(|code| code.as_str()), Some("EUR"));
    assert_eq!(apple.get(&IdType::Cusip), Some("037833100"));
    registry.merge(IsinEntry::new(Isin::new("GB0002374006")?))?;
    let unlisted = registry.get("GB0002374006").expect("the row");
    assert_eq!(unlisted.get(&IdType::Sedol), Some("0237400"));
    assert_eq!(unlisted.currency(), None);

    // A stated value stands over a default.
    registry.merge(IsinEntry::new(Isin::new("GB0002634946")?).with_currency(Some(Ccy::new("USD")?)))?;
    let row = registry.get("GB0002634946").expect("the row");
    assert_eq!(row.currency().map(|code| code.as_str()), Some("USD"));
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    registry = IsinRegistry()
    # A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    assert registry.merge({"isin": "GB0002634946", "miccode": "XLON"})
    row = registry.get("GB0002634946")
    assert row is not None and (row["sedol"], row["currency"]) == ("0263494", "GBP")

    # The currency is the market's country's, never the ISIN's; no market, none.
    assert registry.merge({"isin": "US0378331005", "miccode": "XETR"})
    apple = registry.get("US0378331005")
    assert apple is not None and (apple["cusip"], apple["currency"]) == ("037833100", "EUR")
    registry.merge({"isin": "GB0002374006"})
    unlisted = registry.get("GB0002374006")
    assert unlisted is not None and (unlisted["sedol"], unlisted["currency"]) == ("0237400", None)

    # A stated value stands over a default.
    assert registry.merge({"isin": "GB0002634946", "currency": "USD"})
    assert registry.get("GB0002634946")["currency"] == "USD"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const registry = new IsinRegistry()
    // A GB ISIN behind `00` embeds its SEDOL, and XLON is in GB.
    assert.ok(registry.merge({ isin: 'GB0002634946', miccode: 'XLON' }))
    const row = registry.get('GB0002634946')
    assert.deepEqual([row.sedol, row.currency], ['0263494', 'GBP'])

    // The currency is the market's country's, never the ISIN's; no market, none.
    assert.ok(registry.merge({ isin: 'US0378331005', miccode: 'XETR' }))
    const apple = registry.get('US0378331005')
    assert.deepEqual([apple.cusip, apple.currency], ['037833100', 'EUR'])
    registry.merge({ isin: 'GB0002374006' })
    const unlisted = registry.get('GB0002374006')
    assert.deepEqual([unlisted.sedol, unlisted.currency], ['0237400', null])

    // A stated value stands over a default.
    assert.ok(registry.merge({ isin: 'GB0002634946', currency: 'USD' }))
    assert.equal(registry.get('GB0002634946').currency, 'USD')
    ```

## The update rule

A statement - a row `merge` folds, or what `learn` reads off an element - folds into the listings of its ISIN column by column: the instrument's facts into every listing row, so the rows of one ISIN agree on them at every pause, and the listing facts into the row of the market the statement names. No clock gates it: the statement's `updunix`, `firstunix` and `lastunix` are stamps, and what decides is whether the value is valid and whether it differs.

| Case | What moves |
| --- | --- |
| no row yet | the row is created - the listing of the market the statement names, or the unlisted row where it names none - below `max_instruments`, which counts ISINs; past it `merge` refuses naming the bound and `learn` skips the ISIN |
| a valid value, the row holding none | filled |
| a valid value, the row holding another | replaced - an older or undated statement included |
| the same value | nothing |
| an invalid value | nothing: a typo or a masked number under a checked code is dropped with one deduplicated warning per column, a coarse CFI code, `XXXX`, `XXX`, an unlisted country, a ticker outside one to 64 bytes are no statement - so a row holds only real values, and nothing ranks against anything |
| `cficode` | a code that refines the held one - fills its `X` positions and contradicts nothing ([`Cfi::refined`](../types/codes/cfi.md#two-statements-of-one-instrument)) - refines it; a code the held one refines moves nothing; a contradicting code replaces it whole |
| the listing, on a market the ISIN is listed on | `ticker`, `currency` and each listing code fill or replace by the rows above, on that market's row alone |
| the listing, on a market the ISIN is not listed on | a new listing row: the instrument's facts the ISIN holds, and the listing facts the statement states - a currency alone among them - its currency the market's where it states none; the unlisted row, where that is all the ISIN holds, is taken over instead, its market filled |
| the listing, on no market | the ISIN's single row takes it, listed or not; of several listings none does, with one deduplicated warning per column (`instrument registry listing fact dropped: the statement names no market of the instrument's several listings`), while the statement's instrument facts still fold into every one |
| `underlyingisin` | an instrument fact: a real ISIN other than the row's own fills or replaces on every listing; the row's own ISIN or a typo is no statement |
| `eusipacode` | an instrument fact: a code of a category's shape fills or replaces on every listing; a number of no category's shape is dropped with one warning ([The product category](#the-product-category)) |
| `fisn` | an instrument fact: a short name fills or replaces on every listing |
| `origccy` | an instrument fact: a currency but `XXX` fills or replaces on every listing; nothing derives one - not the ISIN's prefix, which names no issue currency for an Irish or Luxembourg share class, a Cayman domicile or an `XS` Eurobond, and not the listing's currency, which a reload would then read as a statement |
| a derived default | once a statement has landed, the code the ISIN embeds and the currency of the row's market's country fill the columns it left empty, on each row it moved; a default never displaces a statement, and a later statement replaces it ([Derived facts](#contract)) |
| `updunix` | the later of the two, on every listing row, only where a fact moved; a move of `firstunix` or `lastunix` alone leaves it |
| `firstunix` | the earlier of the two, on every listing row, whatever else moved: an event met at an instant before every one learned so far moves it back - the registry dirty - and a later one never moves it; `merge` folds a stated one by the same rule; the seed states none |
| `lastunix` | the later of the two, on every listing row, whatever else moved: `learn` states the element's `transunix` every time, so meeting a known instrument later moves its rows - the registry dirty, the next `commit` writing it - while an earlier or the same instant moves nothing; `merge` folds a stated one by the same rule; the seed states none |

A statement that moves nothing allocates nothing and leaves the registry clean, and a learn that moves only a stamp moves it in place, allocating nothing.

=== "Rust"

    ```rust
    use yggdryl::{Ccy, Cfi, IdType, Isin, IsinEntry, IsinRegistry, Mic};

    let holcim = || -> yggdryl::Result<IsinEntry> { Ok(IsinEntry::new(Isin::new("CH0012214059")?)) };
    let mut registry = IsinRegistry::new();
    assert!(registry.merge(
        holcim()?
            .with_updunix(Some(10))
            .with_miccode(Some(Mic::new("XSWX")?))
            .with_currency(Some(Ccy::new("CHF")?))
            .try_with_code(IdType::Ric, "HOLN.S")?
    )?);
    // A valid value replaces whatever the time; the same one moves nothing.
    assert!(registry.merge(holcim()?.with_updunix(Some(5)).try_with_code(IdType::Ric, "HOLN.VX")?)?);
    assert!(!registry.merge(holcim()?.with_updunix(Some(50)).try_with_code(IdType::Ric, "HOLN.VX")?)?);
    assert_eq!(registry.get("CH0012214059").and_then(|row| row.get(&IdType::Ric)), Some("HOLN.VX"));
    // A typo under a checked code is dropped; a real code replaces.
    assert!(!registry.merge(holcim()?.try_with_code(IdType::Cusip, "037833101")?)?);
    assert!(registry.merge(holcim()?.try_with_code(IdType::Cusip, "037833100")?)?);
    // A refining CFI code refines; a contradicting one replaces.
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESVXXX")?)))?);
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESVUFR")?)))?);
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESNUFR")?)))?);
    assert_eq!(registry.get("CH0012214059").and_then(IsinEntry::cficode).map(Cfi::as_str), Some("ESNUFR"));
    // A listing code on another market is a listing there, holding the instrument's facts;
    // the currency it states none of defaults to the market's, and `get` answers it: XLON sorts first.
    assert!(registry.merge(holcim()?.with_miccode(Some(Mic::new("XLON")?)).try_with_code(IdType::Ric, "HOLN.L")?)?);
    assert_eq!((registry.len(), registry.rows()), (1, 2));
    let london = registry.get_listing("CH0012214059", &Mic::new("XLON")?).expect("the listing");
    assert_eq!((london.get(&IdType::Ric), london.currency().map(Ccy::as_str)), (Some("HOLN.L"), Some("GBP")));
    assert_eq!(london.get(&IdType::Cusip), Some("037833100"), "the instrument's columns are every listing's");
    let swiss = registry.get_listing("CH0012214059", &Mic::new("XSWX")?).expect("the listing");
    assert_eq!((swiss.get(&IdType::Ric), swiss.currency().map(Ccy::as_str)), (Some("HOLN.VX"), Some("CHF")));
    // A listing fact of no market, on two listings, lands on none.
    assert!(!registry.merge(holcim()?.try_with_code(IdType::Ric, "HOLN.Z")?)?);
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    HOLCIM = "CH0012214059"
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "miccode": "XSWX", "ric": "HOLN.S", "ticker": "HOLN", "currency": "CHF", "updunix": 10})
    # A valid value replaces whatever the time; the same one moves nothing.
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 5})
    assert not registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 50})
    # A typo under a checked code is dropped; a real code replaces.
    assert not registry.merge({"isin": HOLCIM, "cusip": "037833101"})
    assert registry.merge({"isin": HOLCIM, "cusip": "037833100", "countrycode": "LI"})
    # A listing code on another market is a listing there, holding the instrument's facts;
    # the currency it states none of defaults to the market's. `get` answers the first in MIC order.
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ric": "HOLNl.L"})
    row = registry.get(HOLCIM)
    assert row is not None and (row["miccode"], row["ric"], row["ticker"], row["currency"]) == ("XLON", "HOLNl.L", None, "GBP")
    assert (row["cusip"], row["countrycode"]) == ("037833100", "LI"), "the instrument's columns are every listing's"
    swiss = registry.get_by_ticker("HOLN", "XSWX")
    assert swiss is not None and (swiss["ric"], swiss["currency"]) == ("HOLN.VX", "CHF")
    # A refining CFI code refines; a contradicting one replaces.
    assert registry.merge({"isin": HOLCIM, "cficode": "ESVXXX"})
    assert registry.merge({"isin": HOLCIM, "cficode": "ESVUFR"})
    assert registry.merge({"isin": HOLCIM, "cficode": "ESNUFR"})
    assert registry.get(HOLCIM)["cficode"] == "ESNUFR"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const HOLCIM = 'CH0012214059'
    const registry = new IsinRegistry()
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XSWX', ric: 'HOLN.S', ticker: 'HOLN', currency: 'CHF', updunix: 10n }))
    // A valid value replaces whatever the time; the same one moves nothing.
    assert.ok(registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 5n }))
    assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 50n }))
    // A typo under a checked code is dropped; a real code replaces.
    assert.ok(!registry.merge({ isin: HOLCIM, cusip: '037833101' }))
    assert.ok(registry.merge({ isin: HOLCIM, cusip: '037833100', countrycode: 'LI' }))
    // A listing code on another market is a listing there, holding the instrument's facts;
    // the currency it states none of defaults to the market's. `get` answers the first in MIC order.
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XLON', ric: 'HOLNl.L' }))
    const row = registry.get(HOLCIM)
    assert.deepEqual([row.miccode, row.ric, row.ticker, row.currency], ['XLON', 'HOLNl.L', null, 'GBP'])
    assert.deepEqual([row.cusip, row.countrycode], ['037833100', 'LI'], 'the instrument\'s columns are every listing\'s')
    const swiss = registry.getByTicker('HOLN', 'XSWX')
    assert.deepEqual([swiss.ric, swiss.currency], ['HOLN.VX', 'CHF'])
    // A refining CFI code refines; a contradicting one replaces.
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVXXX' }))
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVUFR' }))
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESNUFR' }))
    assert.equal(registry.get(HOLCIM).cficode, 'ESNUFR')
    ```

## Listings

An instrument holds one row per market it is listed on: `len` counts ISINs, `rows` the listing rows a commit writes. `get(isin)` answers the first listing in MIC order, `listings(isin)` every one in that order - empty where the ISIN is unknown - and `get_listing(isin, market)` the row of one market, Python `get_listing(isin, market)`, JavaScript `getListing(isin, market)`, a market the `mic` datatype refuses refused. A statement naming a market the ISIN is not listed on makes that listing, holding the instrument's facts; an instrument fact stated on no market folds into every listing, and a listing fact stated on no market of an ISIN listed on several lands on none. `remove_listing(isin, market)` - JavaScript `removeListing` - removes one, the instrument going with its last; `remove(isin)` answers every listing. The seed holds one such pair: HSBC, `GB0005405286`, on `XHKG` and `XLON`.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, IdType, Isin, IsinEntry, IsinRegistry, Mic};

    const HOLCIM: &str = "CH0012214059";
    let holcim = || -> yggdryl::Result<IsinEntry> { Ok(IsinEntry::new(Isin::new(HOLCIM)?)) };
    let mut registry = IsinRegistry::new();
    // Two markets of one ISIN are two listings of one instrument.
    assert!(registry.merge(
        holcim()?
            .with_miccode(Some(Mic::new("XSWX")?))
            .with_ticker(Some("HOLN".into()))
            .try_with_code(IdType::Ric, "HOLN.S")?
    )?);
    assert!(registry.merge(holcim()?.with_miccode(Some(Mic::new("XETR")?)).with_ticker(Some("HLBN".into())))?);
    assert_eq!((registry.len(), registry.rows()), (1, 2));
    let listed: Vec<_> = registry
        .listings(HOLCIM)
        .iter()
        .map(|row| {
            (
                row.miccode().map(|code| code.as_str().to_owned()),
                row.ticker().map(str::to_owned),
                row.currency().map(|code| code.as_str().to_owned()),
            )
        })
        .collect();
    let owned = |mic: &str, ticker: &str, ccy: &str| (Some(mic.to_owned()), Some(ticker.to_owned()), Some(ccy.to_owned()));
    assert_eq!(listed, [owned("XETR", "HLBN", "EUR"), owned("XSWX", "HOLN", "CHF")], "MIC order, each its market's currency");
    assert_eq!(registry.get(HOLCIM), registry.listings(HOLCIM).first(), "the first listing");
    let swiss = registry.get_listing(HOLCIM, &Mic::new("XSWX")?).expect("the listing");
    assert_eq!(swiss.get(&IdType::Ric), Some("HOLN.S"), "a listing code is its market's");
    assert!(registry.get_listing(HOLCIM, &Mic::new("XLON")?).is_none());
    assert!(registry.listings("US0378331005").is_empty());

    // An instrument fact of no market folds into every listing; a listing fact of none, on two, into none.
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESVUFR")?)))?);
    for row in registry.listings(HOLCIM) {
        assert_eq!(row.cficode().map(|code| code.as_str()), Some("ESVUFR"));
        assert_eq!(row.get(&IdType::Valor), Some("1221405"), "the Valor its ISIN embeds, on every listing");
    }
    assert!(!registry.merge(holcim()?.with_ticker(Some("HOLX".into())))?);
    let by_ticker = registry.get_by_ticker("HOLN", None).expect("one listing states it");
    assert_eq!(by_ticker.miccode().map(|code| code.as_str()), Some("XSWX"));

    assert!(registry.remove_listing(HOLCIM, &Mic::new("XETR")?).is_some());
    assert_eq!((registry.len(), registry.rows()), (1, 1));
    assert_eq!(IsinRegistry::seeded().listings("GB0005405286").len(), 2, "HSBC in Hong Kong and London");
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    HOLCIM = "CH0012214059"
    registry = IsinRegistry()
    # Two markets of one ISIN are two listings of one instrument.
    assert registry.merge({"isin": HOLCIM, "miccode": "XSWX", "ticker": "HOLN", "ric": "HOLN.S"})
    assert registry.merge({"isin": HOLCIM, "miccode": "XETR", "ticker": "HLBN"})
    assert (len(registry), registry.rows) == (1, 2)
    assert [(row["miccode"], row["ticker"], row["currency"]) for row in registry.listings(HOLCIM)] == [
        ("XETR", "HLBN", "EUR"),
        ("XSWX", "HOLN", "CHF"),
    ], "MIC order, each its market's currency"
    assert registry.get(HOLCIM) == registry.listings(HOLCIM)[0], "the first listing"
    assert registry.get_listing(HOLCIM, "XSWX")["ric"] == "HOLN.S", "a listing code is its market's"
    assert registry.get_listing(HOLCIM, "XLON") is None and registry.listings("US0378331005") == []

    # An instrument fact of no market folds into every listing; a listing fact of none, on two, into none.
    assert registry.merge({"isin": HOLCIM, "cficode": "ESVUFR"})
    assert [(row["cficode"], row["valor"]) for row in registry.listings(HOLCIM)] == [("ESVUFR", "1221405")] * 2
    assert not registry.merge({"isin": HOLCIM, "ticker": "HOLX"})
    assert registry.get_by_ticker("HOLN")["miccode"] == "XSWX"

    assert registry.remove_listing(HOLCIM, "XETR")["miccode"] == "XETR"
    assert (len(registry), registry.rows) == (1, 1)
    assert [row["miccode"] for row in IsinRegistry.seeded().listings("GB0005405286")] == ["XHKG", "XLON"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const HOLCIM = 'CH0012214059'
    const registry = new IsinRegistry()
    // Two markets of one ISIN are two listings of one instrument.
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XSWX', ticker: 'HOLN', ric: 'HOLN.S' }))
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XETR', ticker: 'HLBN' }))
    assert.deepEqual([registry.length, registry.rows], [1, 2])
    assert.deepEqual(registry.listings(HOLCIM).map((row) => [row.miccode, row.ticker, row.currency]), [
      ['XETR', 'HLBN', 'EUR'],
      ['XSWX', 'HOLN', 'CHF'],
    ], "MIC order, each its market's currency")
    assert.deepEqual(registry.get(HOLCIM), registry.listings(HOLCIM)[0], 'the first listing')
    assert.equal(registry.getListing(HOLCIM, 'XSWX').ric, 'HOLN.S', "a listing code is its market's")
    assert.equal(registry.getListing(HOLCIM, 'XLON'), null)
    assert.deepEqual(registry.listings('US0378331005'), [])

    // An instrument fact of no market folds into every listing; a listing fact of none, on two, into none.
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVUFR' }))
    assert.deepEqual(registry.listings(HOLCIM).map((row) => [row.cficode, row.valor]), [['ESVUFR', '1221405'], ['ESVUFR', '1221405']])
    assert.ok(!registry.merge({ isin: HOLCIM, ticker: 'HOLX' }))
    assert.equal(registry.getByTicker('HOLN').miccode, 'XSWX')

    assert.equal(registry.removeListing(HOLCIM, 'XETR').miccode, 'XETR')
    assert.deepEqual([registry.length, registry.rows], [1, 1])
    assert.deepEqual(IsinRegistry.seeded().listings('GB0005405286').map((row) => row.miccode), ['XHKG', 'XLON'])
    ```

## Matching

`resolve(element)` names the row an element means, and how. `fill` reads the same waterfall, the parse door its exact tiers alone. The element is the query: its `securityids` hold its ISIN, its codes and its short name (`FinancialInstrumentShortName(2737)` read into them as `fisn`), and its market, ticker, currency, origin currency and CFI code are its own facts, so a reconciliation job builds a market leaf with the setters rather than a second query shape.

| Tier | Key | Matched | Refused |
| --- | --- | --- | --- |
| `Isin` | a real ISIN the element states | its listing on the element's market, else its single row, else its first with `listing` false; `derived` false | `UnknownIsin { stated }` where the registry lacks it: the cascade ends, since a lower tier would fill another instrument's codes around the one stated |
| `Code(kind)` | each code of `LOOKUP_CODES` it states, in that order | the one ISIN its index slot holds, its row as above; `derived` true | `Ambiguous { tier, isins }` where two ISINs hold it: the cascade ends, a defect to name and never a reason to guess |
| `Symbology` | its ticker on its market (`get_by_ticker`) | as above | `Ambiguous` likewise |
| `Economic { similarity }` | its short name, where every tier above found nothing and it states a currency other than `XXX` | the instrument listed in that currency whose short name is the most similar (`Fisn::similarity`, `1 - levenshtein / longer length`) at or above `economic_threshold`; `derived` true | `BelowThreshold { best, isin }`; `CfiConflict { stated, held, isin }` where the best is of another CFI category (`X` conflicts with nothing); `CurrencyConflict { stated, held, isin }` where it states another origin currency; `Ambiguous` for two equal bests |

`NoKey` is an element stating nothing any tier reads, and `NoCandidate` one every tier looked for and found none of. `listing` is whether the row's listing facts are the element's - its market is the row's, or either is unstated - so a match by a code on another market fills the instrument's facts and none of the listing's. An economic match is a judgement: `resolve` always weighs it, and `fill` takes it only where `is_economic_match` is set, because a derived ISIN becomes the [book key](market.md#the-book-key) the element's book and chain live under; a parse never takes one.

### By ISIN

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, MatchTier, Mic, Resolution, Unmatched};

    let mut registry = IsinRegistry::new();
    registry.merge(IsinEntry::new(Isin::new("US0378331005")?).with_miccode(Some(Mic::new("XNAS")?)))?;
    registry.merge(IsinEntry::new(Isin::new("DE0007164600")?).with_miccode(Some(Mic::new("XETR")?)))?;
    let order = |codes: &[(IdType, &str)]| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(1);
        for (kind, value) in codes {
            order.insert_securityid(Identifier::new(IdKey::base(kind.clone()), value)?)?;
        }
        Ok(order)
    };

    // The ISIN decides alone, over a CUSIP naming Apple.
    let Resolution::Matched { entry, tier, derived, listing } =
        registry.resolve(&order(&[(IdType::Isin, "DE0007164600"), (IdType::Cusip, "037833100")])?)
    else {
        panic!("SAP by its ISIN");
    };
    assert_eq!((entry.isin().as_str(), tier, derived, listing), ("DE0007164600", MatchTier::Isin, false, true));

    // An ISIN the registry lacks ends the cascade: the CUSIP is never read.
    let unknown = registry.resolve(&order(&[(IdType::Isin, "CH0012005267"), (IdType::Cusip, "037833100")])?);
    assert!(matches!(unknown, Resolution::Unmatched(Unmatched::UnknownIsin { stated }) if stated.as_str() == "CH0012005267"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, IsinRegistry, graph

    registry = IsinRegistry()
    registry.merge({"isin": "US0378331005", "miccode": "XNAS"})
    registry.merge({"isin": "DE0007164600", "miccode": "XETR"})


    def order(*codes: tuple[str, str], **facts: object) -> graph.OrderEvent:
        return graph.OrderEvent(1, securityids=[Identifier(kind, value) for kind, value in codes], **facts)


    # The ISIN decides alone, over a CUSIP naming Apple.
    sap = registry.resolve(order(("isin", "DE0007164600"), ("cusip", "037833100")))
    assert sap and sap.entry is not None and sap.entry["isin"] == "DE0007164600"
    assert (sap.tier, sap.derived, sap.listing) == ("isin", False, True)

    # An ISIN the registry lacks ends the cascade: the CUSIP is never read.
    unknown = registry.resolve(order(("isin", "CH0012005267"), ("cusip", "037833100")))
    assert not unknown and (unknown.unmatched, unknown.stated) == ("UnknownIsin", "CH0012005267")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, IsinRegistry, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'US0378331005', miccode: 'XNAS' })
    registry.merge({ isin: 'DE0007164600', miccode: 'XETR' })
    const order = (codes, facts = {}) =>
      new graph.OrderEvent(1n, { securityids: codes.map(([kind, value]) => new Identifier(kind, value)), ...facts })

    // The ISIN decides alone, over a CUSIP naming Apple.
    const sap = registry.resolve(order([['isin', 'DE0007164600'], ['cusip', '037833100']]))
    assert.deepEqual([sap.matched, sap.entry.isin, sap.tier, sap.derived, sap.listing], [true, 'DE0007164600', 'isin', false, true])

    // An ISIN the registry lacks ends the cascade: the CUSIP is never read.
    const unknown = registry.resolve(order([['isin', 'CH0012005267'], ['cusip', '037833100']]))
    assert.deepEqual([unknown.matched, unknown.unmatched, unknown.stated], [false, 'UnknownIsin', 'CH0012005267'])
    ```

### By code

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, MatchTier, Mic, Resolution, Unmatched};

    let mut registry = IsinRegistry::new();
    registry.merge(IsinEntry::new(Isin::new("US0378331005")?).with_miccode(Some(Mic::new("XNAS")?)))?;
    registry.merge(IsinEntry::new(Isin::new("DE0007164600")?).try_with_code(IdType::Common, "C-1")?)?;
    registry.merge(IsinEntry::new(Isin::new("GB0002374006")?).try_with_code(IdType::Common, "C-1")?)?;
    let by = |kind: IdType, value: &str| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(1);
        order.insert_securityid(Identifier::new(IdKey::base(kind), value)?)?;
        order.set_miccode(Some(Mic::new("XLON")?), true);
        Ok(order)
    };

    // The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    let Resolution::Matched { entry, tier, derived, listing } = registry.resolve(&by(IdType::Cusip, "037833100")?) else {
        panic!("Apple by its CUSIP");
    };
    assert_eq!((entry.isin().as_str(), tier, derived, listing), ("US0378331005", MatchTier::Code(IdType::Cusip), true, false));
    assert_eq!(registry.get_by_code(&IdType::Cusip, "037833100", None).map(|row| row.isin().as_str()), Some("US0378331005"));

    // A code two instruments hold names neither, and stops the cascade.
    let Resolution::Unmatched(Unmatched::Ambiguous { isins, .. }) = registry.resolve(&by(IdType::Common, "C-1")?) else {
        panic!("two instruments");
    };
    assert_eq!(isins.iter().map(Isin::as_str).collect::<Vec<_>>(), ["DE0007164600", "GB0002374006"]);
    // One instrument listed twice is one answer: HSBC in London and Hong Kong.
    let hsbc = IsinRegistry::seeded();
    assert_eq!(hsbc.get_by_code(&IdType::Sedol, "0540528", None).map(|row| row.isin().as_str()), Some("GB0005405286"));
    // A currency keys nothing.
    assert!(hsbc.get_by_code(&IdType::IsoCcy, "USD", None).is_none());
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, IsinRegistry, graph

    registry = IsinRegistry()
    registry.merge({"isin": "US0378331005", "miccode": "XNAS"})
    registry.merge({"isin": "DE0007164600", "common": "C-1"})
    registry.merge({"isin": "GB0002374006", "common": "C-1"})


    def by(kind: str, value: str) -> graph.OrderEvent:
        return graph.OrderEvent(1, securityids=[Identifier(kind, value)], miccode="XLON")


    # The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    apple = registry.resolve(by("cusip", "037833100"))
    assert apple.entry is not None and apple.entry["isin"] == "US0378331005"
    assert (apple.tier, apple.kind, apple.derived, apple.listing) == ("code", "cusip", True, False)
    row = registry.get_by_code("cusip", "037833100")
    assert row is not None and row["isin"] == "US0378331005"

    # A code two instruments hold names neither, and stops the cascade.
    twins = registry.resolve(by("common", "C-1"))
    assert (twins.unmatched, twins.tier, twins.kind, twins.isins) == ("Ambiguous", "code", "common", ["DE0007164600", "GB0002374006"])
    # One instrument listed twice is one answer: HSBC in London and Hong Kong.
    hsbc = IsinRegistry.seeded().get_by_code("sedol", "0540528")
    assert hsbc is not None and hsbc["isin"] == "GB0005405286"
    assert "isoccy" not in IsinRegistry.LOOKUP_CODES and IsinRegistry.LOOKUP_CODES[0] == "cusip"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, IsinRegistry, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'US0378331005', miccode: 'XNAS' })
    registry.merge({ isin: 'DE0007164600', common: 'C-1' })
    registry.merge({ isin: 'GB0002374006', common: 'C-1' })
    const by = (kind, value) => new graph.OrderEvent(1n, { securityids: [new Identifier(kind, value)], miccode: 'XLON' })

    // The CUSIP Apple's ISIN embeds leads back to it; Apple is not listed on XLON.
    const apple = registry.resolve(by('cusip', '037833100'))
    assert.deepEqual([apple.entry.isin, apple.tier, apple.kind, apple.derived, apple.listing], ['US0378331005', 'code', 'cusip', true, false])
    assert.equal(registry.getByCode('cusip', '037833100').isin, 'US0378331005')

    // A code two instruments hold names neither, and stops the cascade.
    const twins = registry.resolve(by('common', 'C-1'))
    assert.deepEqual([twins.unmatched, twins.tier, twins.kind, twins.isins], ['Ambiguous', 'code', 'common', ['DE0007164600', 'GB0002374006']])
    // One instrument listed twice is one answer: HSBC in London and Hong Kong.
    assert.equal(IsinRegistry.seeded().getByCode('sedol', '0540528').isin, 'GB0005405286')
    assert.equal(IsinRegistry.lookupCodes()[0], 'cusip')
    ```

### By ticker

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{Isin, IsinEntry, IsinRegistry, MatchTier, Mic, Resolution};

    let mut registry = IsinRegistry::new();
    registry.merge(IsinEntry::new(Isin::new("GB0002374006")?).with_miccode(Some(Mic::new("XLON")?)).with_ticker(Some("DGE".into())))?;
    let mut order = OrderEvent::at(1);
    order.set_ticker(Some("DGE".into()), true);
    order.set_miccode(Some(Mic::new("XLON")?), true);
    let Resolution::Matched { entry, tier, derived, listing } = registry.resolve(&order) else {
        panic!("Diageo by its ticker");
    };
    assert_eq!((entry.isin().as_str(), tier, derived, listing), ("GB0002374006", MatchTier::Symbology, true, true));
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry, graph

    registry = IsinRegistry()
    registry.merge({"isin": "GB0002374006", "miccode": "XLON", "ticker": "DGE"})
    diageo = registry.resolve(graph.OrderEvent(1, ticker="DGE", miccode="XLON"))
    assert diageo.entry is not None and diageo.entry["isin"] == "GB0002374006"
    assert (diageo.tier, diageo.derived, diageo.listing) == ("symbology", True, True)
    assert registry.resolve(graph.OrderEvent(1, ticker="ZZZZ")).unmatched == "NoCandidate"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'GB0002374006', miccode: 'XLON', ticker: 'DGE' })
    const diageo = registry.resolve(new graph.OrderEvent(1n, { ticker: 'DGE', miccode: 'XLON' }))
    assert.deepEqual([diageo.entry.isin, diageo.tier, diageo.derived, diageo.listing], ['GB0002374006', 'symbology', true, true])
    assert.equal(registry.resolve(new graph.OrderEvent(1n, { ticker: 'ZZZZ' })).unmatched, 'NoCandidate')
    ```

### By short name

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{Ccy, Fisn, IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, MatchTier, Mic, Resolution, Unmatched};

    let mut registry = IsinRegistry::new();
    registry.merge(
        IsinEntry::new(Isin::new("US0378331005")?)
            .with_miccode(Some(Mic::new("XNAS")?))
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
        registry.resolve(&named("APPLE INC/SH")?)
    else {
        panic!("Apple by its short name");
    };
    assert_eq!((entry.isin().as_str(), derived), ("US0378331005", true));
    assert!((similarity - 12.0 / 13.0).abs() < 1e-12);
    // Too far apart is named with the best score.
    let far = registry.resolve(&named("APPLE INC/SH USD CL A")?);
    assert!(matches!(far, Resolution::Unmatched(Unmatched::BelowThreshold { .. })));

    // A fill takes the match only where told to.
    let mut order = named("APPLE INC/SH")?;
    assert!(!registry.fill(&mut order), "a judgement no fill takes unasked");
    registry.set_economic_match(true);
    assert!(registry.fill(&mut order));
    assert_eq!(order.get_isincode(), Some("US0378331005"));
    assert!(order.get_securityids().is_derived(&IdType::Isin));
    assert!(registry.set_economic_threshold(1.5).is_err());
    ```

=== "Python"

    ```python
    import math
    from pathlib import Path

    from yggdryl import Identifier, IsinRegistry, graph
    from yggdryl.fix import FixCodec, FixRegistry

    registry = IsinRegistry()
    registry.merge({"isin": "US0378331005", "miccode": "XNAS", "fisn": "APPLE INC./SH"})


    def named(name: str) -> graph.OrderEvent:
        return graph.OrderEvent(1, securityids=[Identifier("fisn", name)], currency="USD")


    # One character in thirteen apart, in the currency Apple is listed in.
    apple = registry.resolve(named("APPLE INC/SH"))
    assert (apple.tier, apple.derived) == ("economic", True)
    assert apple.similarity is not None and math.isclose(apple.similarity, 12 / 13)
    # Too far apart is named with the best score.
    far = registry.resolve(named("APPLE INC/SH USD CL A"))
    assert (far.unmatched, far.isin) == ("BelowThreshold", "US0378331005")

    # A fill takes the match only where told to.
    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
    line = b"8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|"
    assert registry.economic_threshold == IsinRegistry.DEFAULT_ECONOMIC_THRESHOLD == 0.85
    assert not registry.is_economic_match and not registry.fill(codec.parse_fix_line(line))
    registry.set_economic_match(True)
    message = codec.parse_fix_line(line)
    assert registry.fill(message) and message.isincode == "US0378331005"
    assert message.securityids.is_derived("isin")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Identifier, IsinRegistry, fix, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'US0378331005', miccode: 'XNAS', fisn: 'APPLE INC./SH' })
    const named = (name) => new graph.OrderEvent(1n, { securityids: [new Identifier('fisn', name)], currency: 'USD' })

    // One character in thirteen apart, in the currency Apple is listed in.
    const apple = registry.resolve(named('APPLE INC/SH'))
    assert.deepEqual([apple.tier, apple.derived], ['economic', true])
    assert.ok(Math.abs(apple.similarity - 12 / 13) < 1e-12)
    // Too far apart is named with the best score.
    const far = registry.resolve(named('APPLE INC/SH USD CL A'))
    assert.deepEqual([far.unmatched, far.isin], ['BelowThreshold', 'US0378331005'])

    // A fill takes the match only where told to.
    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const line = () => codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|'))
    assert.equal(registry.economicThreshold, 0.85)
    assert.ok(!registry.isEconomicMatch && !registry.fill(line()))
    registry.setEconomicMatch(true)
    const message = line()
    assert.ok(registry.fill(message))
    assert.equal(message.isincode, 'US0378331005')
    assert.ok(message.securityids.isDerived('isin'))
    ```

### A conflict is named

The most similar instrument is refused, never passed over for the next, where it is another asset class or states another origin currency: the element and the row then describe two instruments, and the answer names both sides.

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{Ccy, Cfi, Fisn, IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, Mic, Resolution, Unmatched};

    let mut registry = IsinRegistry::new();
    registry.merge(
        IsinEntry::new(Isin::new("US0378331005")?)
            .with_miccode(Some(Mic::new("XNAS")?))
            .with_fisn(Some(Fisn::new("APPLE INC/SH")?))
            .with_cficode(Some(Cfi::new("DBFTFR")?)),
    )?;
    let mut order = OrderEvent::at(1);
    order.insert_securityid(Identifier::new(IdKey::base(IdType::Fisn), "APPLE INC/SH")?)?;
    order.set_currency(Ccy::new("USD")?, true);
    order.set_cficode(Some(Cfi::new("ESVUFR")?), true);
    let Resolution::Unmatched(Unmatched::CfiConflict { stated, held, isin }) = registry.resolve(&order) else {
        panic!("a share against a bond");
    };
    assert_eq!((stated, held, isin.as_str()), ('E', 'D', "US0378331005"));
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, IsinRegistry, graph

    registry = IsinRegistry()
    registry.merge({"isin": "US0378331005", "miccode": "XNAS", "fisn": "APPLE INC/SH", "cficode": "DBFTFR"})
    share = graph.OrderEvent(1, securityids=[Identifier("fisn", "APPLE INC/SH")], currency="USD", cficode="ESVUFR")
    conflict = registry.resolve(share)
    assert (conflict.unmatched, conflict.stated, conflict.held, conflict.isin) == ("CfiConflict", "E", "D", "US0378331005")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, IsinRegistry, graph } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'US0378331005', miccode: 'XNAS', fisn: 'APPLE INC/SH', cficode: 'DBFTFR' })
    const share = new graph.OrderEvent(1n, { securityids: [new Identifier('fisn', 'APPLE INC/SH')], currency: 'USD', cficode: 'ESVUFR' })
    const conflict = registry.resolve(share)
    assert.deepEqual([conflict.unmatched, conflict.stated, conflict.held, conflict.isin], ['CfiConflict', 'E', 'D', 'US0378331005'])
    ```

## `firstunix` and `lastunix`

`firstunix` is the earliest instant an event stated the ISIN and `lastunix` the latest: every `learn` states the element's `transunix` as both, so an element stating its ISIN alone is learned, meeting a known instrument at a later instant moves `lastunix` and meeting it at an earlier one - a capture replayed out of order - moves `firstunix` back, each on every listing, the registry dirty and the next `commit` writing the snapshot, while `updunix` stays at the statement that last moved a fact. An instant between the two moves nothing. A golden file states `firstunix` under `firstunix`, `firstseen` or `firstseenunix`; the seed states neither.

=== "Rust"

    ```rust
    use yggdryl::graph::{Market, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier, IsinRegistry, Mic};

    const TEN: i64 = 1_767_348_000_000_000_000; // 2026-01-02T10:00:00Z
    const HOUR: i64 = 3_600_000_000_000;
    let order = |unix: i64| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(unix);
        order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
        Ok(order)
    };
    let mut listed = order(TEN)?;
    listed.set_miccode(Some(Mic::new("XSWX")?), true);
    listed.set_ticker(Some("HOLN".into()), true);
    let mut registry = IsinRegistry::new();
    assert!(registry.learn(&listed));
    let row = registry.get("CH0012214059").expect("learned");
    assert_eq!((row.firstunix(), row.lastunix(), row.updunix()), (Some(TEN), Some(TEN), Some(TEN)));

    // A clean copy meets the instrument an hour later: nothing new is taught, yet the instant moves.
    let mut clean = IsinRegistry::from_arrow_reader(registry.into_arrow_reader()?)?;
    assert!(!clean.is_dirty());
    assert!(clean.learn(&order(TEN + HOUR)?), "the ISIN alone");
    assert!(clean.is_dirty());
    let row = clean.get("CH0012214059").expect("learned");
    assert_eq!((row.lastunix(), row.updunix()), (Some(TEN + HOUR), Some(TEN)), "no fact moved");
    // An earlier instant moves `firstunix` back; one between the two moves nothing.
    assert!(clean.learn(&order(TEN - HOUR)?));
    let row = clean.get("CH0012214059").expect("learned");
    assert_eq!((row.firstunix(), row.lastunix(), row.updunix()), (Some(TEN - HOUR), Some(TEN + HOUR), Some(TEN)));
    assert!(!clean.learn(&order(TEN)?), "between the two");
    ```

=== "Python"

    ```python
    import datetime
    from pathlib import Path

    from yggdryl import IsinRegistry
    from yggdryl.fix import FixCodec, FixRegistry

    HOLCIM = "CH0012214059"
    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))


    def order(stamp: str, listing: str = "") -> object:
        return codec.parse_fix_line(f"8=FIX.4.4|35=D|52={stamp}|11=A|22=4|48={HOLCIM}{listing}|10=0|".encode())


    nine, ten, eleven = (datetime.datetime(2026, 1, 2, hour, tzinfo=datetime.timezone.utc) for hour in (9, 10, 11))
    registry = IsinRegistry()
    assert registry.learn(order("20260102-10:00:00", "|55=HOLN|207=XSWX|15=CHF"))
    row = registry.get(HOLCIM)
    assert (row["firstunix"], row["lastunix"], row["updunix"]) == (ten, ten, ten)

    # A clean copy meets the instrument an hour later: nothing new is taught, yet the instant moves.
    clean = IsinRegistry.from_arrow_reader(registry.into_arrow_reader())
    assert not clean.is_dirty
    assert clean.learn(order("20260102-11:00:00")), "the ISIN alone"
    assert clean.is_dirty
    row = clean.get(HOLCIM)
    assert (row["lastunix"], row["updunix"]) == (eleven, ten), "no fact moved"
    # An earlier instant moves `firstunix` back; one between the two moves nothing.
    assert clean.learn(order("20260102-09:00:00"))
    row = clean.get(HOLCIM)
    assert (row["firstunix"], row["lastunix"], row["updunix"]) == (nine, eleven, ten)
    assert not clean.learn(order("20260102-10:00:00")), "between the two"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { IsinRegistry, fix } = require('yggdryl')

    const HOLCIM = 'CH0012214059'
    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
    const order = (stamp, listing = '') => codec.parseFixLine(Buffer.from(`8=FIX.4.4|35=D|52=${stamp}|11=A|22=4|48=${HOLCIM}${listing}|10=0|`))
    const [nine, ten, eleven] = [9, 10, 11].map((hour) => new Date(Date.UTC(2026, 0, 2, hour)))

    const registry = new IsinRegistry()
    assert.ok(registry.learn(order('20260102-10:00:00', '|55=HOLN|207=XSWX|15=CHF')))
    const row = registry.get(HOLCIM)
    assert.deepEqual([row.firstunix, row.lastunix, row.updunix], [ten, ten, ten])

    // A clean copy meets the instrument an hour later: nothing new is taught, yet the instant moves.
    const clean = IsinRegistry.fromArrowReader(registry.intoArrowReader())
    assert.equal(clean.isDirty, false)
    assert.ok(clean.learn(order('20260102-11:00:00')), 'the ISIN alone')
    assert.equal(clean.isDirty, true)
    const met = clean.get(HOLCIM)
    assert.deepEqual([met.lastunix, met.updunix], [eleven, ten], 'no fact moved')
    // An earlier instant moves `firstunix` back; one between the two moves nothing.
    assert.ok(clean.learn(order('20260102-09:00:00')))
    const back = clean.get(HOLCIM)
    assert.deepEqual([back.firstunix, back.lastunix, back.updunix], [nine, eleven, ten])
    assert.ok(!clean.learn(order('20260102-10:00:00')), 'between the two')
    ```

## The underlying

A row's `underlyingisin` is the ISIN of the instrument this one is written on - a warrant's share, an option's index future - FIX's underlying. It is an instrument fact under the update rule: a real ISIN other than the row's own fills or replaces on every listing, a typo is dropped with one warning, the row's own ISIN is stored as none, and a new listing holds it. Nothing lifts it: an underlying's ISIN never enters `securityids`, because the [book key](market.md#the-book-key) is the instrument's own, so `Identifier::from_key` keeps refusing `UnderlyingISIN` ([Reading a name](identifier.md#reading-a-name)).

`learn` never states one. What states it is the FIX [lifecycle](../fix/lifecycle.md#instruments-are-learned-in-instant-order), reading each message beside its ISIN - a real one of its own, or nothing is learned - from the first of: `UnderlyingSecurityID(309)` under an ISIN `UnderlyingSecurityIDSource(305)` or under none, else an `UnderlyingSymbol(311)` shaped as an ISIN or an instrument key, at the root or in each `NoUnderlyings(711)` occurrence; a `RelatedSecurityID(1650)` of a `NoRelatedInstruments(1647)` occurrence typed `Underlier` (`RelatedInstrumentType(1648)` `2`); and an unmapped entry a bridge keys as an underlying's ISIN (`UnderlyingISIN`, `OMS_Underlying_ISIN_Code`). Two different underlyings - a basket - name none, and so do the message's own ISIN and a number that does not close. Every code stays where it arrived, on the wire and in the message's metadata. A golden file states it under `underlyingisin` or any key naming an underlying's ISIN ([Persistence](#persistence)).

=== "Rust"

    ```rust
    use yggdryl::{Isin, IsinEntry, IsinRegistry, Mic};

    let novartis = || -> yggdryl::Result<IsinEntry> { Ok(IsinEntry::new(Isin::new("CH0012005267")?)) };
    let mut registry = IsinRegistry::new();
    assert!(registry.merge(novartis()?.with_miccode(Some(Mic::new("XSWX")?)).with_underlyingisin(Some(Isin::new("CH0012214059")?)))?);
    let underlying = |registry: &IsinRegistry| registry.get("CH0012005267").and_then(IsinEntry::underlyingisin).map(Isin::as_str).map(str::to_owned);
    assert_eq!(underlying(&registry).as_deref(), Some("CH0012214059"));
    // The row's own ISIN is no underlying, a typo is dropped, and a new listing holds it.
    assert!(!registry.merge(novartis()?.with_underlyingisin(Some(Isin::new("CH0012005267")?)))?);
    assert!(!registry.merge(novartis()?.with_underlyingisin(Some(Isin::new("CH0012214058")?)))?);
    assert!(registry.merge(novartis()?.with_miccode(Some(Mic::new("XLON")?)).with_ticker(Some("NOVNL".into())))?);
    assert_eq!(underlying(&registry).as_deref(), Some("CH0012214059"));
    assert_eq!(IsinEntry::new(Isin::new("CH0012005267")?).with_underlyingisin(Some(Isin::new("CH0012005267")?)).underlyingisin(), None);
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    NOVARTIS, HOLCIM = "CH0012005267", "CH0012214059"
    registry = IsinRegistry()
    assert registry.merge({"isin": NOVARTIS, "miccode": "XSWX", "underlyingisin": HOLCIM})
    assert registry.get(NOVARTIS)["underlyingisin"] == HOLCIM
    # The row's own ISIN is no underlying, a typo is dropped, and a new listing holds it.
    assert not registry.merge({"isin": NOVARTIS, "underlyingisin": NOVARTIS})
    assert not registry.merge({"isin": NOVARTIS, "underlyingisin": "CH0012214058"})
    assert registry.merge({"isin": NOVARTIS, "miccode": "XLON", "ticker": "NOVNL"})
    assert registry.get(NOVARTIS)["underlyingisin"] == HOLCIM
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const [NOVARTIS, HOLCIM] = ['CH0012005267', 'CH0012214059']
    const registry = new IsinRegistry()
    assert.ok(registry.merge({ isin: NOVARTIS, miccode: 'XSWX', underlyingisin: HOLCIM }))
    assert.equal(registry.get(NOVARTIS).underlyingisin, HOLCIM)
    // The row's own ISIN is no underlying, a typo is dropped, and a new listing holds it.
    assert.ok(!registry.merge({ isin: NOVARTIS, underlyingisin: NOVARTIS }))
    assert.ok(!registry.merge({ isin: NOVARTIS, underlyingisin: 'CH0012214058' }))
    assert.ok(registry.merge({ isin: NOVARTIS, miccode: 'XLON', ticker: 'NOVNL' }))
    assert.equal(registry.get(NOVARTIS).underlyingisin, HOLCIM)
    ```

## The product category

A row's `eusipacode` is the product category of a structured product: one four-digit code of EUSIPA's European Derivative Map, which the SSPA's Swiss Derivative Map - the Swiss column of the European one - numbers the same way. The first digit is the level - `1` an investment product, `2` a leverage product - the first two the group - `11` capital protection, `12` yield enhancement, `13` participation, `14` credit linked notes, `21` and `22` leverage without and with a knock-out, `23` constant leverage - and the last two the member, `99` a group's miscellaneous one. It is an instrument fact under the [update rule](#the-update-rule): a code of a category's shape fills or replaces on every listing, a number of no category's shape is dropped with one warning, and a new listing holds it. Nothing lifts it: no FIX field names it and it is no identifier, so it enters no map ([Reading a name](identifier.md#reading-a-name)).

| Key | Rule |
| --- | --- |
| Value | `yggdryl::Eusipa` (root `eusipa.rs`), a value of its own rather than a datatype, as [`Limit`](book.md#limits) is: `Eusipa::new(code)` holds a `u16` of the shape - `1000` to `2999` - and refuses another as a value, at the value itself - no datatype - (`invalid record value at $: expected a four-digit EUSIPA product category opening with 1, an investment product, or 2, a leverage product, got 3100`); `from_text` and `FromStr` read four ASCII digits once trimmed - no sign, no leading zero, no fraction - (`invalid record value at $: expected a four-digit EUSIPA product category, got "23x0"`); `code()`, `group()` (the first two digits), `level()` (the first); `Display` the four digits; serde the number, read back through `new`; `TryFrom<u16>` and `From<Eusipa> for u16`; equality, order and hash by the code |
| Names | `name()` the English name EUSIPA's map of February 2024 gives the code, `sspa_name()` the one the SSPA's gives - its 2023 map (v.23/2) and its 2026 map (v.26/1) list the same categories - each `None` where that map lists no such member; `is_listed()` whether either does |
| Held by its shape | a code is held by its shape, never by a list: the maps are snapshots of lists that move - EUSIPA retired `1110` and added its credit linked notes, the SSPA added `1135`, `1255` and its `14xx` - and a feed may state a member either list has not caught up with, so `Eusipa::new(2301)` holds a code no map lists, `is_listed()` false |
| One code, two names | the maps share their numbering and name one code apart: `1260` is Express Certificates in the European map and a Conditional Coupon Barrier Reverse Convertible in the Swiss one, so the code, never a name, is the fact a row holds; every other code both maps list names one category, worded each map's way (`2200` Knock-Out Warrants, Warrant with Knock-Out) |
| The column | `eusipacode`, `int32` - the code, in a width every table format stores, Iceberg having no unsigned integer - the eighth column, after `underlyingisin`; Rust `IsinEntry::eusipacode()` and `with_eusipacode(Option<Eusipa>)`; a Python or JavaScript row crosses it as the number, which `merge` takes as a number or its text |
| Learned | by the FIX [lifecycle](../fix/lifecycle.md#instruments-are-learned-in-instant-order), beside a message whose real ISIN keys the row, off an unmapped entry a bridge keys: a key whose folded name ends with `eusipa`, `eusipacode`, `eusipacategory`, `sspa`, `sspacode` or `sspacategory`, a namespace before it passed over - `EUSIPACode`, `OMS_SSPACategory`, `X-SWX-SSPA` - whose text is four digits once trimmed. Two different categories state none, text of no category's shape states none, a key naming a name (`EUSIPA_Name`) is none, since the maps name one code apart, and so is a key naming another instrument's category - `leg`, `underlying`, `contra`, `related` or `benchmark` opening the key or spelled just before the category word, after any namespace: `UnderlyingEUSIPA`, `LegSSPACategory`, `OMS_ContraEUSIPA` - by the rule a [security type](identifier.md#reading-a-name) is refused by. `learn` never states one, and the entry stays where it arrived |
| Read | from a golden file's column spelled the same six ways, Euronext's `EUSIPA_Code` among them, its cells numbers or the text of one ([Persistence](#persistence)) |
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
    use yggdryl::{Eusipa, Isin, IsinEntry, IsinRegistry, Mic};

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

    // An instrument fact: it fills on any market, and a new listing holds it.
    let product = || -> yggdryl::Result<IsinEntry> { Ok(IsinEntry::new(Isin::new("CH0123456789")?)) };
    let mut registry = IsinRegistry::new();
    assert!(registry.merge(product()?.with_miccode(Some(Mic::new("XSWX")?)).with_eusipacode(Some(constant)))?);
    assert!(registry.merge(product()?.with_miccode(Some(Mic::new("XLON")?)).with_ticker(Some("ACMEL".into())))?);
    assert_eq!(registry.get("CH0123456789").and_then(IsinEntry::eusipacode), Some(constant));
    ```

=== "Python"

    ```python
    from yggdryl import Eusipa, IsinRegistry

    # A category is held by its shape; each map names the codes it lists.
    constant = Eusipa("2300")
    assert (constant.code, constant.group, constant.level) == (2300, 23, 2)
    assert constant.name == "Constant Leverage Certificate" and int(constant) == 2300 and str(constant) == "2300"
    express = Eusipa(1260)
    assert express.name == "Express Certificates"
    assert express.sspa_name == "Conditional Coupon Barrier Reverse Convertible"
    assert Eusipa(1135).name is None, "a code only the Swiss map lists"
    assert not Eusipa(2301).is_listed, "a code of the shape no map lists is held"
    for refused in (3100, "23x0"):
        try:
            Eusipa(refused)
        except ValueError:
            pass
        else:
            raise AssertionError(refused)

    # An instrument fact: it fills on any market, and a new listing holds it.
    PRODUCT = "CH0123456789"
    registry = IsinRegistry()
    assert registry.merge({"isin": PRODUCT, "miccode": "XSWX", "eusipacode": 2300})
    assert registry.merge({"isin": PRODUCT, "miccode": "XLON", "ticker": "ACMEL"})
    assert Eusipa(registry.get(PRODUCT)["eusipacode"]) == constant
    # A number of no category's shape is dropped, with a warning.
    assert not registry.merge({"isin": PRODUCT, "eusipacode": 3100})
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    // JavaScript has no Eusipa: a row crosses the category as its number.
    const PRODUCT = 'CH0123456789'
    const registry = new IsinRegistry()
    assert.ok(registry.merge({ isin: PRODUCT, miccode: 'XSWX', eusipacode: 2300 }))
    assert.ok(registry.merge({ isin: PRODUCT, miccode: 'XLON', ticker: 'ACMEL' }))
    assert.equal(registry.get(PRODUCT).eusipacode, 2300)
    // A number of no category's shape is dropped, with a warning.
    assert.ok(!registry.merge({ isin: PRODUCT, eusipacode: 3100 }))
    ```

A walk learns the category off a bridge's key beside the message's ISIN, and nothing else moves: the entry stays where it arrived.

=== "Rust"

    ```rust
    use std::sync::{Arc, Mutex};

    use yggdryl::local::LocalFolder;
    use yggdryl::{Eusipa, FixCodec, FixMsg, FixRegistry, IsinEntry, IsinRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let instruments = Arc::new(Mutex::new(IsinRegistry::new()));
    let codec = FixCodec::new(registry).with_isin_registry(Arc::clone(&instruments));
    let parsed: Vec<FixMsg> = codec
        .parse_lines([
            "8=FIX.4.4|35=D|11=A|22=4|48=CH0123456789|55=ACMEX|207=XSWX|EUSIPACode=2300|10=0|",
            // Two different categories state none.
            "8=FIX.4.4|35=D|11=B|22=4|48=CH0012214059|EUSIPA=2300|SSPA=1260|10=0|",
        ])
        .collect::<yggdryl::Result<_>>()?;
    let walked: Vec<FixMsg> = codec.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    assert_eq!(walked.len(), 2);
    let learned = instruments.lock().expect("the instrument registry");
    assert_eq!(learned.get("CH0123456789").and_then(IsinEntry::eusipacode), Some(Eusipa::new(2300)?));
    assert_eq!(learned.get("CH0012214059").and_then(IsinEntry::eusipacode), None);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import IsinRegistry
    from yggdryl.fix import FixCodec, FixRegistry

    instruments = IsinRegistry()
    codec = FixCodec(FixRegistry.from_handle(Path("config/fix")), isin_registry=instruments)
    lines = [
        b"8=FIX.4.4|35=D|11=A|22=4|48=CH0123456789|55=ACMEX|207=XSWX|OMS_SSPACategory=1260|10=0|",
        # Two different categories state none.
        b"8=FIX.4.4|35=D|11=B|22=4|48=CH0012214059|EUSIPA=2300|SSPA=1260|10=0|",
    ]
    assert len(list(codec.lifecycle(codec.parse_lines(lines)))) == 2
    assert instruments.get("CH0123456789")["eusipacode"] == 1260
    assert (instruments.get("CH0012214059") or {}).get("eusipacode") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { IsinRegistry, fix } = require('yggdryl')

    const instruments = new IsinRegistry()
    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')), { isinRegistry: instruments })
    const lines = [
      '8=FIX.4.4|35=D|11=A|22=4|48=CH0123456789|55=ACMEX|207=XSWX|X-SWX-SSPA=1260|10=0|',
      // Two different categories state none.
      '8=FIX.4.4|35=D|11=B|22=4|48=CH0012214059|EUSIPA=2300|SSPA=1260|10=0|',
    ]
    assert.equal([...codec.lifecycle([...codec.parseLines(lines.map((line) => Buffer.from(line)))])].length, 2)
    assert.equal(instruments.get('CH0123456789').eusipacode, 1260)
    assert.equal((instruments.get('CH0012214059') ?? {}).eusipacode ?? null, null)
    ```

## Persistence

A registry is bound to one store and loaded from it once: `from_holder(holder)` and `from_url(url, properties)` build an empty registry, bind it and load - the holder's own record stream, an Arrow IPC leaf, Parquet, a folder of parts, an Iceberg table or an object store, under the holder's own record options resolved once at the binding - Arrow IPC for a folder listing no record leaf, or plain text alone - the same spellings `extend_from_handle` reads; a store holding nothing is an empty first run, and the registry is clean after the load. Those two load the store's rows alone; `seeded_from_holder(holder)` and `seeded_from_url(url, properties)` - Python `IsinRegistry.seeded_from_url`, JavaScript `IsinRegistry.seededFromUrl` - lay them over the [seed](#seed) as [`from_env`](#the-process-registry) does: the store's rows fold over the seed's by the update rule, so a value the store states wins and a fact only the seed states stands beside it, a seed row the store has none of stands, and a store holding nothing loads as the seed bound to it; the store is read once and the seed costs it no call, and the registry is clean after the load, so the seed reaches the store with the first commit after something moved, as part of its snapshot. `set_holder` and `try_with_holder` bind a registry already holding rows: the store's rows are loaded and the held rows fold over them, so the registry is dirty exactly where a held row moved something.

`commit()` writes the table back only where it moved since it was loaded or last committed, so the store holds exactly the snapshot (`into_arrow_reader`) - every listing row, in ISIN then market order - whatever its layout: a leaf is rewritten in one overwrite, and truncated by a registry emptied; an Iceberg table is replaced in one atomic snapshot, every row of every partition, an emptied registry one empty snapshot that keeps the table a table; a plain folder has its record parts of the store's encoding removed - a leaf of another encoding or a file that is no record part, a README beside the parts, is never touched - then the snapshot laid out as one `part-0.arrows` under the layout the folder spells, none where the registry is empty. A clean registry touches the store with no call and answers no rows; a registry bound to no store refuses. A leaf names its encoding by its name, so `instruments.parquet` in a build without Parquet is refused at the binding rather than laid out as something else, and a location inside an Iceberg table - one partition of it - is refused too. Nothing commits implicitly: a lifecycle learns into the table, and the caller commits. The store's own rules hold: two processes committing whole snapshots to one file lose each other's rows, and an Iceberg table at the location is replaced atomically.

In a medallion pipeline the registry's commit is the stage right after the FIX-message parse: the lifecycle walk writes `silver.fix_messages` and teaches the codec's registry as it goes, and the next stage, `silver.instruments`, commits that registry once the refined write has drained the walk, so the commit holds every instrument the window taught before anything reads the refined messages, and a window that taught nothing writes nothing. The pipeline binds the seeded registry (`seeded_from_url`) to an Iceberg table created from `IsinEntry::field()`, so partitioned by the ISIN's country prefix through `truncate(isin, 2)`; each commit is one snapshot replacing every row of every partition, and the first one carries the seed rows into the lake's instruments table, which is then the whole reference.

`extend_from_arrow_reader` resolves each column of the reader's schema once, before a row is read: the registry's own name, any spelling or alias of an identifier type (`RIC`, `riccode`, `BloombergSymbol`, `ISINCode`, `ccypair`), a field name a type is spelled by (`#ISINCODE`, `cusip_code`), or the registry's own spellings (`cfi`/`cficode`, `country`/`countrycode`/`countryofissue`, `mic`/`miccode`, `ticker`/`symbol`, `ccy`/`currency`/`currencycode`, `origccy`/`origcurrency`/`originalcurrency`/`issuecurrency`, `updunix`, `firstunix`/`firstseen`/`firstseenunix`, `lastunix`/`lastseen`/`lastseenunix`, `underlyingisin` or any key naming an underlying's ISIN - `UnderlyingISIN`, `OMS_Underlying_ISIN_Code` - and `eusipa`, `eusipacode`, `eusipacategory`, `sspa`, `sspacode` or `sspacategory` for the product category - Euronext's `EUSIPA_Code` - its cells numbers or the text of one); a column naming nothing is ignored. Two columns naming one fact are refused naming both, and a stream with no `isin` column is refused naming the columns it has - a stream of no columns at all, what a missing store reads as, is the empty registry. One cast plan lands each batch under the resolved subset of the row, a cell a typed column cannot hold landing null; a typed code column's cells are adopted as the landing proved them, and a `utf8` column's through its type's value rule. Each row folds through `merge` as a statement, so rows of one ISIN - in one file, or across a folder's parts - fold in the order they are read, its rows on several markets loading as several listings, and a load past `max_instruments` is refused naming the row rather than truncated. `extend_from_handle(handle)` reads a handle's own record stream the same way without binding to it, and `into_arrow_reader()` clones the table's `Arc` and lays the rows out one bounded batch at a time, holding one batch and never the table; a learn while it streams copies the table once and moves nothing the stream reads.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, IdType, Isin, IsinEntry, IsinRegistry, Url};

    // A folder that is not there yet is an empty first run, laid out by the
    // first commit; a trailing slash is what makes it a folder.
    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-isin-doc-{}", std::process::id()));
    let url = Url::from_location(&format!("{}/", root.display()))?;
    let none: [(&str, &str); 0] = [];
    let mut registry = IsinRegistry::from_url(&url, none)?;
    assert!(registry.is_empty() && !registry.is_dirty());
    assert_eq!(registry.commit()?.written_rows, 0, "a clean registry writes nothing");
    registry.merge(IsinEntry::new(Isin::new("CH0012214059")?).try_with_code(IdType::Ric, "HOLN.S")?)?;
    assert!(registry.is_dirty());
    assert_eq!(registry.commit()?.written_rows, 1);
    assert!(!registry.is_dirty());
    assert!(root.join("part-0.arrows").is_file());
    let loaded = IsinRegistry::from_url(&url, none)?;
    assert!(loaded.iter().eq(registry.iter()));
    assert!(registry.holder().is_some_and(IOBase::is_container));
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import os
    import tempfile
    from pathlib import Path

    from yggdryl import IsinRegistry

    with tempfile.TemporaryDirectory() as folder:
        # A folder that is not there yet is an empty first run, laid out by
        # the first commit; a trailing separator is what makes it a folder.
        store = str(Path(folder) / "isin") + os.sep
        registry = IsinRegistry.from_url(store)
        assert len(registry) == 0 and not registry.is_dirty
        assert registry.commit().written_rows == 0, "a clean registry writes nothing"
        registry.merge({"isin": "CH0012214059", "ric": "HOLN.S"})
        assert registry.is_dirty and registry.commit().written_rows == 1 and not registry.is_dirty
        assert (Path(folder) / "isin" / "part-0.arrows").is_file()
        loaded = IsinRegistry.from_url(store)
        assert loaded.get("CH0012214059") == registry.get("CH0012214059")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IsinRegistry } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'instruments-'))
    try {
      // A folder that is not there yet is an empty first run, laid out by
      // the first commit; a trailing separator is what makes it a folder.
      const store = path.join(folder, 'isin') + path.sep
      const registry = IsinRegistry.fromUrl(store)
      assert.equal(registry.length, 0)
      assert.equal(registry.isDirty, false)
      assert.equal(registry.commit().writtenRows, 0, 'a clean registry writes nothing')
      registry.merge({ isin: 'CH0012214059', ric: 'HOLN.S' })
      assert.equal(registry.isDirty, true)
      assert.equal(registry.commit().writtenRows, 1)
      assert.equal(registry.isDirty, false)
      assert.ok(fs.existsSync(path.join(folder, 'isin', 'part-0.arrows')))
      const loaded = IsinRegistry.fromUrl(store)
      assert.deepEqual(loaded.get('CH0012214059'), registry.get('CH0012214059'))
    } finally {
      fs.rmSync(folder, { recursive: true, force: true })
    }
    ```

An Iceberg table is the store a pipeline keeps its instruments in: created from the registry's own row (`IsinEntry::field()`, Python and JavaScript `IsinRegistry.field()`), the registry bound to it by its location, every lifecycle learning into it and one `commit` after the walk replacing the table's rows as one snapshot - nothing where the walk learned nothing new.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOResult, IdType, Isin, IsinEntry, IsinRegistry, Url};

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-isin-lake-{}", std::process::id()));
    IcebergTable::create(
        LocalFolder::new(&root)?,
        FormatVersion::V3,
        IsinEntry::field(),
        PartitionSpec::unpartitioned(),
    )?;
    let url = Url::from_location(&format!("{}/", root.display()))?;
    let none: [(&str, &str); 0] = [];
    let mut registry = IsinRegistry::from_url(&url, none)?;
    registry.merge(IsinEntry::new(Isin::new("CH0012214059")?).try_with_code(IdType::Ric, "HOLN.S")?)?;
    assert_eq!(registry.commit()?.written_rows, 1, "one snapshot holds the registry");
    assert_eq!(registry.commit()?, IOResult::default(), "a clean registry commits nothing");
    let loaded = IsinRegistry::from_url(&url, none)?;
    assert!(loaded.iter().eq(registry.iter()));
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import tempfile

    from yggdryl import IsinRegistry
    from yggdryl.iceberg import IcebergCatalog

    with tempfile.TemporaryDirectory() as folder:
        catalog = IcebergCatalog.open_or_create("lake", folder)
        namespace = catalog.namespaces.open_or_create("reference")
        table = namespace.tables.open_or_create("instruments", IsinRegistry.field())
        registry = IsinRegistry.from_url(table.url)
        registry.merge({"isin": "CH0012214059", "ric": "HOLN.S"})
        assert registry.commit().written_rows == 1, "one snapshot holds the registry"
        assert registry.commit().written_rows == 0, "a clean registry commits nothing"
        assert catalog.table("reference.instruments").row_size() == 1
        loaded = IsinRegistry.from_url(table.url)
        assert loaded.get("CH0012214059") == registry.get("CH0012214059")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IsinRegistry, iceberg } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'instruments-lake-'))
    try {
      const catalog = iceberg.IcebergCatalog.openOrCreate('lake', folder)
      const namespace = catalog.namespaces().openOrCreate('reference')
      const table = namespace.tables().openOrCreate('instruments', IsinRegistry.field())
      const registry = IsinRegistry.fromUrl(String(table.url))
      registry.merge({ isin: 'CH0012214059', ric: 'HOLN.S' })
      assert.equal(registry.commit().writtenRows, 1, 'one snapshot holds the registry')
      assert.equal(registry.commit().writtenRows, 0, 'a clean registry commits nothing')
      const loaded = IsinRegistry.fromUrl(String(table.url))
      assert.deepEqual(loaded.get('CH0012214059'), registry.get('CH0012214059'))
    } finally {
      fs.rmSync(folder, { recursive: true, force: true })
    }
    ```

## The process registry

`IsinRegistry::from_env()` is the registry the process environment names, resolved once on the first call and shared behind one lock - Python `IsinRegistry.from_env()`, JavaScript `IsinRegistry.fromEnv()`. The order is fixed, first match wins:

1. a registry installed by `install_env` - Python `IsinRegistry.install_env(registry)`, JavaScript `IsinRegistry.installEnv(registry)` - before anything resolves one; installing after a resolution is a typed conflict;
2. the location `YGGDRYL_ISIN_REGISTRY_URI` names: a URL of any scheme this build holds - a local folder or leaf, an Iceberg table's folder, an object store - or a bare path, a leading `~` the home directory; an empty value reads as unset, and a store holding nothing yet is an empty first run;
3. `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first `commit` lays out;
4. with no home directory, the [seed](#seed) bound to nothing.

Every store is laid over the seed: its rows fold over the seed's by the [update rule](#the-update-rule), so a value the store states wins and a seed row it has none of stands, clean after the load; a store holding nothing loads as the seed bound to it, and the seed reaches the store with the first commit that moves anything, as part of its snapshot. A location that names a scheme this build has no backend for, a store that cannot be read or a row the registry refuses is an error, never the seed alone, and the default stays unresolved so the next call retries. `FixCodec::from_env()` - Python `FixCodec.from_env()`, JavaScript `FixCodec.fromEnv()` - is the one codec constructor that attaches it, so every parse through that codec fills from it and every lifecycle learns into it; `FixCodec::new` and the bindings' constructors attach none unless handed a registry. Nothing commits implicitly: what the walks learned reaches the store when the caller commits.

```bash
YGGDRYL_ISIN_REGISTRY_URI=s3://bucket/instruments/   # an object store folder of IPC parts
YGGDRYL_ISIN_REGISTRY_URI=~/warehouse/instruments/   # an Iceberg table's folder, replaced in one snapshot
YGGDRYL_ISIN_REGISTRY_URI=/data/instruments.arrows   # one IPC leaf
```

## Bounds

| Bound | Value |
| --- | --- |
| `max_instruments` | `IsinRegistry::DEFAULT_MAX_INSTRUMENTS` (16,384) ISINs unless `with_max_instruments` says otherwise; the listings of a known ISIN are not counted against it; no eviction |
| Memory | each listing row is charged its worst case - B-tree slack, twelve codes at their widest, the ticker and the short name, one slot in the ticker index and one in the code index per code - 5,387 bytes on a 64-bit target, rounded up to 8 KiB, so a registry holds `rows()` × 8 KiB at most, 128 MiB at the default bound with one listing an instrument; one listing sits inline in its ISIN's slot, so an instrument of one market costs nothing beside its row |
| Per row | `MAX_EQUIVALENTS` (12) codes: a row stating more, or a code in a `utf8` column its type refuses (past its `max_value_width`), is refused naming the row and the column, `$[1].wkn`. A typed code column's cell its datatype cannot hold lands null, and a ticker outside one to 64 bytes, an unlisted country or `XXX` is stored as none; the rest of the row loads |
| Past the bound | `learn` skips a new ISIN with one warning per registry, a known ISIN learning on; `merge`, `extend_from_*` and a load refuse, naming the bound - which fails `from_env` on a store holding more rows than the bound |

## Edges

- Learning is the ordered lifecycle's, never a parse's: a parse fills derived identifiers from the table its door fixed - the ISIN a ticker names on its market, every equivalent, the pair - and nothing else, so the message's identity, its wire and its row are the same with and without a table, and what a parse derived is learned back as nothing. A learn while a door reads reaches no message of that reading. Share one registry across walks run one after another; walks run at once interleave their learning.
- A registry's fill is a derivation: it never writes `CFICode(461)`, `Currency(15)` or any FIX field, so a row's `cficode` shows a refined code only where 461 is unstated.
- A filled ISIN moves the [book](book.md) an element stands in: its [book key](market.md#the-book-key) is the ISIN once the table fills it at the parse, so a ticker-only statement joins its instrument's book, and the candles it lands in, from the first message - as long as the table the door fixed already knew the pair.
- A ticker leads to a listing only on the market it was stated on, or, where none is, to the one row stating it on no market; where the element states no market, to the one row stating it anywhere, so one ticker on two markets - the same instrument's or two - leads to none. A code of `LOOKUP_CODES` leads to its instrument whatever market holds it - two listings of one ISIN holding one RIC or one SEDOL are one answer - and to none where two ISINs hold it; its row is the element's market's, else the one row holding it, and its listing facts fill only on the same market.
- One row per listing: a multi-venue instrument stated from each venue holds one row per venue, each filled into an element on its own market, and an element of no market, or of a market the instrument is not listed on, takes the instrument's facts alone. Every venue a lifecycle meets the ISIN on - an MTF, `XOFF` - is a listing of it.
- Every learn states `firstunix` and `lastunix`, so a walk that meets a known instrument outside the instants already learned dirties the registry and the next `commit` writes the snapshot, though no other fact moved.
- `origccy` fills an element only where it holds none and is never derived into a column: an element of a USD-issued share listed in EUR stating only `currency = EUR` teaches the registry no origin currency, and one the registry held stands.
- An economic match is weighed per instrument - the short name is the instrument's - over the instruments listed in the element's currency, linear in them and below `max_instruments`; the parse door never runs it.
- The currency is the listing's: it fills only on the same stated market under the row's ticker, and an FX trade's `Currency(15)` - the currency dealt, not a listing's - is never learned.
- The country of issue is learned only where a message states one its ISIN's prefix does not already say (`CountryOfIssue(470)` differing from the prefix), because the parse lands the prefix on every message stating none; every other row answers its prefix, and a wrong held country is taken back by an explicit `merge` stating the prefix, which a folded row never holds beside its key.
- A ticker is looked up trimmed, as it is learned, and a statement whose only new code would be a thirteenth type moves nothing.
- A store laid out before the product category was a column - a leaf or an Iceberg table whose row has no `eusipacode`, `fisn`, `firstunix`, `lastunix` or `origccy` - loads it as null and keeps its own row on commit, as every overwrite of a leaf or a table does: no migration rewrites it, the category stays in the registry, and every commit holding one warns once, naming the column and the store (`instrument registry column not stored: the store's row lacks it (eusipacode at <store>)`), deduplicated as every [data warning](../logging.md#warnings) is. An emptied leaf, a new store and a plain folder - whose parts every commit lays out afresh - hold the row as it is now.
- Nothing walks the underlying: a chain of underlyings - a warrant on an option on a share - is one hop per row, and a cycle stated by mistake is held as stated.
- A whole overwrite is the one commit: last writer wins on a file store, and a crash mid-write leaves a torn leaf that fails the next load.

## Performance

`graph/isin_registry` over a registry of 4,096 instruments - each with a CFI code, a market, a ticker, a common code and a RIC - and `fix/pipeline/decoded_lifecycle` with and without a shared registry. One containerized x86_64 Linux run: Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile with thin LTO. Criterion medians.

| Case | Median | What it does |
| --- | --- | --- |
| `learn_known` | 1.23 µs | a statement of a known ISIN saying nothing new: no row moves, nothing allocates |
| `learn_new` | 11.7 µs | a new ISIN with a CFI code and a common code, into a table no clone shares: a forty-column row built and its ticker indexed |
| `fill_by_isin` | 1.87 µs | an order naming only the ISIN, filled with the row's codes, ticker and CFI code, then finalized |
| `fill_by_ticker` | 2.14 µs | the same order naming only its ticker on its market: its ISIN derived first through the ticker index |
| `fill_miss` | 185 ns | an ISIN no row holds: the key checked real, one lookup, nothing built |
| `into_arrow_reader_4096` | 50.9 ms (80.5 K rows/s) | the snapshot stream drained: each row laid out as its named struct, through the row's value door |
| `extend_from_arrow_reader_4096` | 9.52 ms (430 K rows/s) | the same rows loaded into an empty registry: one cast plan, a code cell adopted as the landing proved it |
| `ipc_roundtrip_4096` | 65.4 ms (62.7 K rows/s) | the snapshot written to an Arrow IPC buffer and read back by `extend_from_handle` |
| `decoded_lifecycle` | 420 ms (31.5 MiB/s) | the decoded capture walked with the walk-local registry |
| `decoded_lifecycle_shared_registry` | 419 ms (31.6 MiB/s) | the same walk learning into a registry the codec shares: one lock per message across the learn and the fill |

Writing a snapshot costs five times reading it back: a row crosses into Arrow as a named struct the row's value door checks and restates - eight allocations a row, pinned in `rust/tests/allocations.rs` - where a load adopts each landed code. The shared walk and the walk-local one measure the same within Criterion's interval over ten samples each, each walk starting from an empty registry: the one uncontended lock a message takes across its learn and its fill is below what the walk's own work hides.

```bash
cargo bench -p yggdryl --bench graph -- 'graph/isin_registry'
cargo bench -p yggdryl --bench fix -- '^fix/pipeline/decoded_lifecycle(_shared_registry)?$'
```

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- isin_registry
    cargo test -p yggdryl --test isin_registry
    cargo test -p yggdryl --test isin_registry --features "internals parquet iceberg"
    cargo test -p yggdryl --test allocations -- isin_registry
    cargo test -p yggdryl --test fix -- enrich batch threads
    cargo bench -p yggdryl --bench graph -- 'graph/isin_registry'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_isin_registry.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/isin_registry.test.js
    ```
