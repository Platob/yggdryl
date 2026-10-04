# IsinRegistry

`IsinRegistry` is a table of instruments keyed by ISIN, one row per instrument: its detailed CFI code, its country of issue, the currency pair an FX or referential number names, its listing's market, ticker and trading currency, and one code per `SecurityIDSource(22)` type - a RIC, a Bloomberg symbol, a CUSIP, a FIGI among them. A lifecycle learns each market element's statements into it and fills what a later element of the instrument leaves unsaid; a parse fills the security identifiers a message leaves unsaid from the table its door fixed. A ticker leads back to the ISIN on its market; every other code is an equivalent the ISIN fills. A valid stated value fills and replaces whatever the time. The registry is bound to the store it was loaded from - an Arrow IPC leaf, a folder of parts, Parquet, an Iceberg table, an object store - and commits back one snapshot only where it moved.

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::IsinRegistry` and `yggdryl::IsinEntry` (root `isin_registry.rs`, with `isin_registry/store.rs` and `isin_registry/env.rs`); Python `yggdryl.IsinRegistry`; JavaScript `IsinRegistry`. A binding holds one table behind one lock and crosses a row as a `dict` / plain object of its columns; `IsinEntry` is Rust-only |
| Key | a real ISIN (`IdType::Isin.is_real`: closing under a listed prefix, `XT` for a referential instrument among the agency prefixes) - the instrument's one atomic code; a `ZZ` number, a masked one or a typo keys no row. A ticker leads back to its row through an exact inverse index, gated by the market its listing was stated on; a RIC, a Bloomberg symbol, a FIGI, a CUSIP or a SEDOL is an equivalent the ISIN fills, never looked up |
| Row | `IsinEntry::field()`, the non-null struct `isinregistry`: `isin` (`isin`, required), `updunix` (`datetime64(ns, UTC)`, when the statement that last moved the row happened - a stamp, deciding nothing), `cficode` (`cfi`, detailed only), `countrycode` (`country`, the stated country of issue, listed only), `forexcode` (`forex`, the pair an FX or referential number names), `miccode` (`mic`, the market its listing columns belong to, never `XXXX`), `ticker` (`utf8`, one to 64 bytes), `currency` (`ccy`, the listing's trading currency, never `XXX`), then one column per `SecurityIDSource(22)` type but the ISIN, in code-set order - `cusip`, `sedol`, `quik`, `ric`, `isoccy`, `isoctry`, `exchsymb`, `cta`, `bloomberg`, `wkn`, `dutch`, `valor`, ... `dti` - each typed by `IdType::value_dtype` (`ric`, `bbg`, `figi`, `utf8` for a type with no datatype of its own): 40 columns, at most `IsinRegistry::MAX_EQUIVALENTS` (12) codes stated per row |
| Instrument and listing columns | `cficode`, `countrycode`, `forexcode` and every code `IdType::is_listing` does not name are the instrument's and fill on any market; `miccode`, `ticker`, `currency` and the listing codes - `ric`, `bloomberg`, `exchsymb`, `cta`, `sedol`, `figi`, `mktassigned`, `fim`, `umtf` - are one listing's, and move together |
| Derived, never stored | the country of issue the ISIN's prefix names where ISO 3166 lists it (`IsinEntry::country()`, the stated country first), and the national number an ISIN embeds ([`securityid::embedded`](../types/codes/isin.md)) |
| `merge(entry)` | folds one row into the row of its ISIN by the [update rule](#the-update-rule); whether anything moved. Refuses an ISIN that is not real (`expected an ISIN some agency numbers, got ...`) and a new ISIN past `max_instruments` |
| `learn(event)` | reads what a dated market element states about its instrument, keyed by its stated real ISIN alone, at its `currunix`: its detailed CFI code, its market but `XXXX`, its ticker, its currency but `XXX` - none where it holds a currency pair, whose `Currency(15)` is the dealt currency - the pair as a `forex` identifier, and each equivalent its map answers with a real value ([`IdType::is_real`](identifier.md#ranks)); never a derived code, a masked number or a typo, an `Other` type or an `instrumentid`. A new ISIN past the bound is skipped with one warning per registry; a known one keeps learning |
| `fill(element)` | finds the row by the element's real ISIN, stated or derived - a miss ends the fill - else by its ticker on its market (`get_by_ticker`), deriving the ISIN first over none or a number [ranking](identifier.md#ranks) below it, a masked one or a typo. It fills each equivalent of a type the element holds nothing of as a `derived` identifier (its base key filled, so `map['valor']` answers); the listing codes only where the two markets - none and `XXXX` unstated - agree or either is unstated; the pair; the ticker on the same market; the CFI code where the element states none or the row's [refines](../types/codes/cfi.md#two-statements-of-one-instrument) it; and the currency only where both markets are stated and equal, the ticker is the row's and the element states none. An element where anything moved is finalized. Nothing reaches a FIX field or the wire |
| `enrich(event)` | `learn`, then `fill` |
| Reads | `get(isin)` borrows a row, allocation-free; `get_by_ticker(ticker, market)` the one row listing the ticker on `market`, or where either market is unstated (none and `XXXX` unstated), none where two rows answer - Python `get_by_ticker(ticker, market=None)`, JavaScript `getByTicker(ticker, market?)`, a market the `mic` datatype refuses refused; `iter()` in ISIN order; `len`, `is_empty`, `max_instruments`, `is_dirty`; `remove(isin)`, `clear()` |
| Persistence | `from_holder(holder)` / `from_url(url, properties)` bind and load - Python `IsinRegistry.from_url(location, max_instruments=..., **properties)`, JavaScript `IsinRegistry.fromUrl(location, maxInstruments?, properties?)`; `set_holder` / `try_with_holder` bind a registry already holding rows; `commit()` writes back only where it moved; `extend_from_handle`, `from_arrow_reader` and `extend_from_arrow_reader` read without binding; `into_arrow_reader()` is the snapshot stream ([Persistence](#persistence)) |
| The process's own | `from_env()` resolves once from `YGGDRYL_ISIN_REGISTRY_URI`, else `~/.config/yggdryl/isin/`, shared behind one lock; `install_env` installs one first; `FixCodec::from_env()` attaches it, `FixCodec::new` attaches none, and nothing commits but the caller ([The process registry](#the-process-registry)) |
| Sharing | `FixCodec::with_isin_registry(Arc<Mutex<IsinRegistry>>)` shares one table with every lifecycle and parse door the codec runs: a parse door fixes the table once as it opens, under one lock on the calling thread, and every worker fills from that; a lifecycle learns and fills under one lock per message. Python `FixCodec(..., isin_registry=registry)`, JavaScript `new fix.FixCodec(registry, { isinRegistry })`, each with an `isin_registry` / `isinRegistry` getter answering the caller's own table |

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

    // An order naming the ISIN is filled with the rest; a RIC alone names nothing.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    assert!(registry.enrich(&mut order));
    assert_eq!(order.get_securityids().get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert!(order.get_securityids().is_derived(&IdType::Ric));
    assert_eq!(order.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));
    let mut by_ric = OrderEvent::at(1_700_000_000_000_000_000);
    by_ric.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
    assert!(!registry.fill(&mut by_ric));

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

    // The table streams out as a snapshot, one row per ISIN.
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

    # The table streams out as a snapshot, one row per ISIN, in ISIN order.
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

    // The table streams out as a snapshot, one row per ISIN, in ISIN order.
    const back = IsinRegistry.fromArrowReader(registry.intoArrowReader())
    assert.deepEqual(back.get('CH0012214059'), row)
    ```

## The update rule

A statement - a row `merge` folds, or what `learn` reads off an element - folds into its ISIN's row column by column. No clock gates it: `updunix` is a stamp, and validity and difference decide.

| Case | What moves |
| --- | --- |
| no row yet | the row is created, below `max_instruments`; past it `merge` refuses naming the bound and `learn` skips the ISIN |
| a valid value, the row holding none | filled |
| a valid value, the row holding another | replaced - an older or undated statement included |
| the same value | nothing |
| an invalid value | nothing: a typo or a masked number under a checked code is dropped with one deduplicated warning per column; a coarse CFI code, `XXXX`, `XXX`, an unlisted country and a ticker outside one to 64 bytes are no statement - so a row holds only real values, and nothing ranks against anything |
| `cficode` | a code that refines the held one - fills its `X` positions and contradicts nothing ([`Cfi::refined`](../types/codes/cfi.md#two-statements-of-one-instrument)) - refines it; a code the held one refines moves nothing; a contradicting code replaces it whole |
| the listing, on the same market or where either market is unstated | `ticker`, `currency` and each listing code fill or replace by the rows above, under the row's market, which takes the statement's where it had none |
| the listing, on another market | a statement stating a ticker or a listing code switches the listing whole: `miccode`, `ticker`, `currency` and the listing codes become what it states, every listing column it does not state cleared; a currency alone names no listing and moves nothing |
| `updunix` | the later of the two, only where something moved |

A statement that moves nothing allocates nothing and leaves the registry clean.

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
    // A typo under a checked code is dropped; a real code replaces.
    assert!(!registry.merge(holcim()?.try_with_code(IdType::Cusip, "037833101")?)?);
    assert!(registry.merge(holcim()?.try_with_code(IdType::Cusip, "037833100")?)?);
    // A currency alone on another market names no listing; a listing code there switches it whole.
    assert!(!registry.merge(holcim()?.with_miccode(Some(Mic::new("XLON")?)).with_currency(Some(Ccy::new("GBP")?)))?);
    assert!(registry.merge(holcim()?.with_miccode(Some(Mic::new("XLON")?)).try_with_code(IdType::Ric, "HOLN.L")?)?);
    let row = registry.get("CH0012214059").expect("the row");
    assert_eq!((row.miccode().map(Mic::as_str), row.get(&IdType::Ric), row.currency()), (Some("XLON"), Some("HOLN.L"), None));
    assert_eq!(row.get(&IdType::Cusip), Some("037833100"), "the instrument's own columns stay");
    // A refining CFI code refines; a contradicting one replaces.
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESVXXX")?)))?);
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESVUFR")?)))?);
    assert!(registry.merge(holcim()?.with_cficode(Some(Cfi::new("ESNUFR")?)))?);
    assert_eq!(registry.get("CH0012214059").and_then(IsinEntry::cficode).map(Cfi::as_str), Some("ESNUFR"));
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
    assert registry.merge({"isin": HOLCIM, "cusip": "037833100"})
    # A currency alone on another market names no listing; a listing code there switches it whole.
    assert not registry.merge({"isin": HOLCIM, "miccode": "XLON", "currency": "GBP"})
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ric": "HOLNl.L"})
    row = registry.get(HOLCIM)
    assert row is not None and (row["miccode"], row["ric"], row["ticker"], row["currency"]) == ("XLON", "HOLNl.L", None, None)
    assert row["cusip"] == "037833100", "the instrument's own columns stay"
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
    assert.ok(registry.merge({ isin: HOLCIM, cusip: '037833100' }))
    // A currency alone on another market names no listing; a listing code there switches it whole.
    assert.ok(!registry.merge({ isin: HOLCIM, miccode: 'XLON', currency: 'GBP' }))
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XLON', ric: 'HOLNl.L' }))
    const row = registry.get(HOLCIM)
    assert.deepEqual([row.miccode, row.ric, row.ticker, row.currency], ['XLON', 'HOLNl.L', null, null])
    assert.equal(row.cusip, '037833100', 'the instrument\'s own columns stay')
    // A refining CFI code refines; a contradicting one replaces.
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVXXX' }))
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVUFR' }))
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESNUFR' }))
    assert.equal(registry.get(HOLCIM).cficode, 'ESNUFR')
    ```

## Persistence

`from_holder(holder)` and `from_url(url, properties)` bind an empty registry to one store and load it once - the holder's own record stream: an Arrow IPC leaf, Parquet, a folder of parts, an Iceberg table or an object store - under the holder's record options, resolved once at the binding: Arrow IPC for a folder listing no record leaf, or plain text alone. A store holding nothing is an empty first run, and the registry is clean after the load. `set_holder` and `try_with_holder` bind a registry already holding rows: the store's rows load and the held rows fold over them, so it is dirty exactly where a held row moved something.

`commit()` writes the table back only when it moved since the load or the last commit, leaving the store holding exactly the snapshot (`into_arrow_reader`):

- a leaf is rewritten in one overwrite, truncated by an emptied registry;
- an Iceberg table is replaced in one atomic snapshot, every row of every partition; an emptied registry commits one empty snapshot, keeping the table a table;
- a plain folder has its record parts of the store's encoding removed - a leaf of another encoding or a file that is no record part, a README beside them, is never touched - and the snapshot laid out as one `part-0.arrows` under the folder's layout, none for an empty registry.

A clean registry makes no call and answers no rows; one bound to no store refuses. A leaf's name states its encoding, so `instruments.parquet` in a build without Parquet is refused at the binding, as is a location inside an Iceberg table - one of its partitions. Nothing commits implicitly: a lifecycle learns, and the caller commits. The store's own rules hold: two processes committing whole snapshots to one file lose each other's rows.

`extend_from_arrow_reader` resolves each column of the reader's schema once, before a row is read: the registry's own name, any spelling or alias of an identifier type (`RIC`, `riccode`, `BloombergSymbol`, `ISINCode`, `ccypair`), a field name a type is spelled by (`#ISINCODE`, `cusip_code`), or the registry's own spellings (`cfi`/`cficode`, `country`/`countrycode`/`countryofissue`, `mic`/`miccode`, `ticker`/`symbol`, `ccy`/`currency`/`currencycode`, `updunix`); a column naming nothing is ignored. Two columns naming one fact are refused naming both, and a stream with no `isin` column is refused naming the columns it has - a stream of no columns at all, what a missing store reads as, is the empty registry. One cast plan lands each batch under the resolved subset of the row, a cell a typed column cannot hold landing null; a typed code column's cells are adopted as the landing proved them, and a `utf8` column's through its type's value rule. Each row folds through `merge`, so rows of one ISIN - in one file, or across a folder's parts - fold in read order, and a load past `max_instruments` is refused naming the row, never truncated. `extend_from_handle(handle)` reads a handle's record stream the same way without binding to it. `into_arrow_reader()` clones the table's `Arc` and lays the rows out one bounded batch at a time, never holding the table; a learn while it streams copies the table once and moves nothing the stream reads.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{IdType, Isin, IsinEntry, IsinRegistry, Url};

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

## The process registry

`IsinRegistry::from_env()` is the registry the process environment names, resolved once on the first call and shared behind one lock - Python `IsinRegistry.from_env()`, JavaScript `IsinRegistry.fromEnv()`. The order is fixed, first match wins:

1. a registry installed by `install_env` - Python `IsinRegistry.install_env(registry)`, JavaScript `IsinRegistry.installEnv(registry)` - before anything resolves one; installing after a resolution is a typed conflict;
2. the location `YGGDRYL_ISIN_REGISTRY_URI` names: a URL of any scheme this build holds - a local folder or leaf, an Iceberg table's folder, an object store - or a bare path, a leading `~` the home directory; an empty value reads as unset, and a store holding nothing yet is an empty first run;
3. `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first `commit` lays out;
4. with no home directory, an empty registry bound to nothing.

A location that names a scheme this build has no backend for, a store that cannot be read or a row the registry refuses is an error, never the empty registry, and the default stays unresolved so the next call retries. `FixCodec::from_env()` - Python `FixCodec.from_env()`, JavaScript `FixCodec.fromEnv()` - is the one codec constructor that attaches it, so every parse through that codec fills from it and every lifecycle learns into it; `FixCodec::new` and the bindings' constructors attach none unless handed one. What the walks learn reaches the store when the caller commits.

```bash
YGGDRYL_ISIN_REGISTRY_URI=s3://bucket/instruments/   # an object store folder of IPC parts
YGGDRYL_ISIN_REGISTRY_URI=~/warehouse/instruments/   # an Iceberg table's folder, replaced in one snapshot
YGGDRYL_ISIN_REGISTRY_URI=/data/instruments.arrows   # one IPC leaf
```

## Bounds

| Bound | Value |
| --- | --- |
| `max_instruments` | `IsinRegistry::DEFAULT_MAX_INSTRUMENTS` (16,384) unless `with_max_instruments` says otherwise; no eviction |
| Memory | each instrument is charged its worst case - B-tree slack, twelve codes at their widest, the ticker, one slot in the ticker index - 3 KiB, so a registry holds at most `max_instruments` × 3 KiB, 48 MiB at the default |
| Per row | `MAX_EQUIVALENTS` (12) codes: a row stating more, or a code in a `utf8` column its type refuses (past its `max_value_width`), is refused naming the row and the column, `$[1].wkn`. A typed code column's cell its datatype cannot hold lands null, and a ticker outside one to 64 bytes, an unlisted country or `XXX` is stored as none; the rest of the row loads |
| Past the bound | `learn` skips a new ISIN with one warning per registry, a known ISIN learning on; `merge`, `extend_from_*` and a load refuse, naming the bound - which fails `from_env` on a store holding more rows than the bound |

## Edges

- Learning is the ordered lifecycle's, never a parse's. A parse only fills derived identifiers from the table its door fixed - the ISIN a ticker names on its market, every equivalent, the pair - so a message's identity, wire and row are the same with or without a table, and nothing a parse derived is learned back. A learn during a door's read reaches no message of that read. Walks run at once interleave their learning; share one registry across walks run one after another.
- A registry's fill is a derivation: it never writes `CFICode(461)`, `Currency(15)` or any FIX field, so a row's `cficode` shows a refined code only where 461 is unstated.
- A filled ISIN moves the [book](book.md) an element stands in: its [book key](market.md#the-book-key) is the ISIN once the table fills it at the parse, so a ticker-only statement joins its instrument's book, and the candles it lands in, from the first message - as long as the table the door fixed already knew the pair.
- A ticker leads to a row only on the market its listing was stated on, or where the element or the row states none; a ticker two rows list there leads to none. A RIC leads nowhere: two rows may hold one RIC, each on its own listing.
- One listing per ISIN: across venues, fills alternate rather than mix - never wrong, by the market gate, only sparse - and a multi-venue instrument stated from two venues in turn switches its listing on each statement from the other venue, keeping the registry dirty.
- The currency is the listing's: it fills only on the same stated market under the row's ticker, and an FX trade's `Currency(15)` - the currency dealt, not a listing's - is never learned.
- The country of issue is learned only where `CountryOfIssue(470)` differs from the ISIN's prefix, since the parse lands the prefix on every message stating none; any other row answers its prefix. A wrong held country is taken back by an explicit `merge` stating the prefix's, which a row never stores beside its key.
- A ticker is looked up trimmed, as it is learned, and a statement whose only new code would be a thirteenth type moves nothing.
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
