# IsinRegistry

`IsinRegistry` is a table of instruments keyed by ISIN: one row per ISIN of the equivalents it is known by - its detailed CFI code, the market and ticker of its listing, and one code per `SecurityIDSource(22)` type, a RIC, a Bloomberg symbol, a CUSIP, a FIGI among them. A lifecycle learns each market element's statements into it and fills what a later element of the same instrument leaves unsaid - found by its ISIN, its RIC or its ticker on its market; a real value is never downgraded, and between two of one rank the latest statement leads column by column while an older one only fills. The table is an Arrow stream, so it loads from and saves to any holder - an Arrow IPC file, Parquet, a folder of either, an object store - through the record surface every medium shares.

## Contract

| Key | Rule |
| --- | --- |
| Owner | `yggdryl::IsinRegistry` and `yggdryl::IsinEntry` (root `isin_registry.rs`); Python `yggdryl.IsinRegistry`; JavaScript `IsinRegistry`. A binding holds one table behind one lock and crosses a row as a `dict` / plain object of its columns; `IsinEntry` is Rust-only |
| Key | a real ISIN - closing under a listed prefix, `IdType::Isin.is_real` - the instrument's one atomic code; a `ZZ` number, a masked one or a typo keys no row. A RIC or a ticker leads back to its row through an exact inverse index the rows keep, the ticker gated by the market the listing was stated on; a Bloomberg symbol, a FIGI, a CUSIP or a SEDOL is an equivalent the ISIN fills and is never looked up |
| Row | `IsinEntry::field()`, the non-null struct `isinregistry`: `isin` (`isin`, required), `updunix` (`datetime64(ns, UTC)`, when the statement that last moved the row happened, null an undated row and the oldest), `cficode` (`cfi`, detailed only), `miccode` (`mic`, the market its listing columns belong to, never `XXXX`), `ticker` (`utf8`, one to 64 bytes), then one column per `SecurityIDSource(22)` type but the ISIN, in code-set order - `cusip`, `sedol`, `quik`, `ric`, `isoccy`, `isoctry`, `exchsymb`, `cta`, `bloomberg`, `wkn`, `dutch`, `valor`, ... `dti` - each typed by `IdType::value_dtype` (`ric`, `bbg`, `figi`, `utf8` for a type with no datatype of its own): 37 columns, at most `IsinRegistry::MAX_EQUIVALENTS` (12) codes stated per row |
| Listing columns | `IdType::is_listing` names the codes of one venue's listing rather than the instrument - `ric`, `bloomberg`, `exchsymb`, `cta`, `sedol`, `figi`, `mktassigned`, `fim`, `umtf`, `instrumentid` - and they belong, with `ticker`, to the row's `miccode` |
| `merge(entry)` | folds one row into the row of its ISIN by the [update rule](#the-update-rule); whether anything moved. Refuses an ISIN that is not real (`expected an ISIN some agency numbers, got ...`) and a new ISIN past `max_instruments` |
| `learn(event)` | reads what a dated market element states about its instrument - keyed by its stated ISIN where it is real, else by its stated RIC through the inverse index, which only fills - at its `currunix`: its detailed CFI code, its market but `XXXX`, its ticker and each equivalent its map answers with a real value ([`IdType::is_real`](identifier.md#ranks)); never a derived code, a masked number or a typo, an `Other` type or an `instrumentid`. A new ISIN past the bound is skipped with one warning per registry; a known one keeps learning |
| `fill(element)` | from the row its real ISIN - stated or derived - names, else the row its RIC names, else the row its ticker names on its market (`get_by_ticker`), whose ISIN is derived first - over none, or over a number [ranking](identifier.md#ranks) below it, a masked one or a typo: each equivalent of a type it holds nothing of as a `derived` identifier (its base key filled, so `map['valor']` answers), the listing codes and the ticker only where its market - none and `XXXX` unstated - is the row's or either is unstated, and its CFI code where it states none or the row's [refines](../types/codes/cfi.md#two-statements-of-one-instrument) it; the element is finalized where anything moved. Nothing reaches a FIX field or the wire |
| `enrich(event)` | `learn`, then `fill` - what each lifecycle runs on every message, in instant order |
| Reads | `get(isin)` and `get_by_ric(ric)` borrow a row, allocation-free; `get_by_ticker(ticker, market)` the one row listing the ticker whose market is `market`, or where either is unstated - none and `XXXX` unstated - two rows answering being ambiguous and answering none - Python `get_by_ticker(ticker, market=None)`, JavaScript `getByTicker(ticker, market?)`, a market the `mic` datatype refuses refused; `iter()` in ISIN order; `len`, `is_empty`, `max_instruments`; `remove(isin)`, `clear()` |
| A value | `Clone` is an O(1) snapshot sharing the table, and a write copies it only while another clone shares it; an empty registry allocates nothing |
| Persistence | `from_handle(handle)` / `extend_from_handle(handle)` read the handle's own record stream, `from_arrow_reader(reader)` / `extend_from_arrow_reader(reader)` any Arrow stream, and `into_arrow_reader()` is a snapshot stream under `IsinEntry::field()`, written by `IOBase::write_arrow_reader` - an overwrite saves a snapshot, a merge by `isin` upserts. No write verb of its own |
| Sharing | `FixCodec::with_isin_registry(Arc<Mutex<IsinRegistry>>)` shares one table with every lifecycle the codec runs; without it each walk learns into its own, starting empty. Python `FixCodec(..., isin_registry=registry)`, JavaScript `new fix.FixCodec(registry, { isinRegistry })`, each with an `isin_registry` / `isinRegistry` getter answering the caller's own table |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{Event, Market, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier, IsinRegistry, Mic};

    // A golden file's columns are read by any spelling of the fact they name.
    let schema = Arc::new(Schema::new(vec![
        Field::new("ISIN", DataType::Utf8, false),
        Field::new("RIC", DataType::Utf8, true),
        Field::new("BloombergSymbol", DataType::Utf8, true),
        Field::new("CFI", DataType::Utf8, true),
        Field::new("MIC", DataType::Utf8, true),
        Field::new("Ticker", DataType::Utf8, true),
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
        ],
    )?;
    let mut registry = IsinRegistry::from_arrow_reader(batch_reader(schema, vec![golden]))?;
    assert_eq!(registry.len(), 1);

    // An order naming only the RIC is filled with the rest.
    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "HOLN.S")?)?;
    assert!(registry.enrich(&mut order));
    assert_eq!(order.get_isincode(), Some("CH0012214059"));
    assert!(order.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(order.get_securityids().get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert_eq!(order.get_cficode().map(|code| code.as_str()), Some("ESVUFR"));

    // An order naming only its ticker on the listing's market finds the row
    // through the ticker index; another market's listing is another row.
    let mut quoted = OrderEvent::at(1_700_000_000_000_000_000);
    quoted.set_ticker(Some("HOLN".into()), true);
    quoted.set_miccode(Some(Mic::new("XSWX")?), true);
    assert!(registry.enrich(&mut quoted));
    assert_eq!(quoted.get_isincode(), Some("CH0012214059"));
    assert!(registry.get_by_ticker("HOLN", Some(&Mic::new("XLON")?)).is_none());

    // The table streams out as a snapshot, one row per ISIN.
    let back = IsinRegistry::from_arrow_reader(registry.into_arrow_reader()?)?;
    assert_eq!(back.get("CH0012214059"), registry.get("CH0012214059"));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Identifier, IsinRegistry, graph

    # A golden file's columns are read by any spelling of the fact they name.
    golden = pa.table({
        "ISIN": ["CH0012214059"],
        "RIC": ["HOLN.S"],
        "BloombergSymbol": ["HOLN SW Equity"],
        "CFI": ["ESVUFR"],
        "MIC": ["XSWX"],
        "Ticker": ["HOLN"],
    })
    registry = IsinRegistry.from_arrow_reader(golden)
    assert len(registry) == 1
    row = registry.get_by_ric("HOLN.S")
    assert row is not None and (row["isin"], row["bloomberg"], row["cficode"], row["ticker"]) == (
        "CH0012214059",
        "HOLN SW Equity",
        "ESVUFR",
        "HOLN",
    )

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
    })
    const registry = IsinRegistry.fromArrowReader(BatchReader.from(golden))
    assert.equal(registry.length, 1)
    const row = registry.getByRic('HOLN.S')
    assert.deepEqual([row.isin, row.bloomberg, row.cficode, row.ticker], ['CH0012214059', 'HOLN SW Equity', 'ESVUFR', 'HOLN'])

    // The table streams out as a snapshot, one row per ISIN, in ISIN order.
    const back = IsinRegistry.fromArrowReader(registry.intoArrowReader())
    assert.deepEqual(back.get('CH0012214059'), row)
    ```

## The update rule

A statement - a row `merge` folds, or what `learn` reads off an element, dated by its `updunix` - folds into the row of its ISIN column by column:

| Case | What moves |
| --- | --- |
| no row yet | the row is created, below `max_instruments`; past it `merge` refuses naming the bound and `learn` skips the ISIN |
| which is newer | an undated row is the oldest; an undated statement is older than a dated row; a dated statement at or after the row's `updunix` is newer, ties going by read order |
| a column the row lacks | filled, whatever the time |
| a code column holding another value | a value of a higher [rank](identifier.md#ranks) replaces it whatever the time, one of a lower rank never; between two of one rank, replaced by a newer statement and kept against an older one |
| `cficode`, `miccode`, `ticker` holding another value | replaced by a newer statement, kept against an older one - `cficode` by its own rule below |
| `cficode` | a code that refines the held one - fills its `X` positions and contradicts nothing ([`Cfi::refined`](../types/codes/cfi.md#two-statements-of-one-instrument)) - refines it whatever the time; a contradicting code replaces it whole only when newer; a coarse code is no statement |
| the listing | a newer statement stating another market and at least one listing fact switches the listing whole: `miccode` becomes its market and every listing column it does not restate is cleared. An older one's listing facts are dropped; where either states no market, listing facts fold by the rows above under the row's market, which takes the statement's where it had none |
| a RIC | the inverse index stays exact: a row's RIC moving drops its old entry, and a RIC another row holds moves only from a statement at or after that row's `updunix`, which then loses it - a RIC names one listing at a time. A RIC not taken leaves the row's own where its market stayed, and none where the statement switched the listing: one market's RIC never stays on another's |
| `updunix` | the later of the two, only where something moved |
| keyed by its RIC | a statement `learn` keys through the RIC is folded as older than the row: it fills and refines, never replaces or switches |

A statement that moves nothing allocates nothing and never copies a shared table.

=== "Rust"

    ```rust
    use yggdryl::{Cfi, IdType, Isin, IsinEntry, IsinRegistry, Mic};

    let holcim = || -> yggdryl::Result<IsinEntry> { Ok(IsinEntry::new(Isin::new("CH0012214059")?)) };
    let mut registry = IsinRegistry::new();
    assert!(registry.merge(
        holcim()?
            .with_updunix(Some(10))
            .with_miccode(Some(Mic::new("XSWX")?))
            .try_with_code(IdType::Ric, "HOLN.S")?
    )?);
    // An older statement only fills; a newer one replaces.
    assert!(!registry.merge(holcim()?.with_updunix(Some(5)).try_with_code(IdType::Ric, "HOLN.VX")?)?);
    assert!(registry.merge(holcim()?.with_updunix(Some(20)).try_with_code(IdType::Ric, "HOLN.VX")?)?);
    assert!(registry.get_by_ric("HOLN.S").is_none() && registry.get_by_ric("HOLN.VX").is_some());
    // A refining CFI code refines whatever the time.
    assert!(registry.merge(holcim()?.with_updunix(Some(1)).with_cficode(Some(Cfi::new("ESVXXX")?)))?);
    assert!(registry.merge(holcim()?.with_updunix(Some(1)).with_cficode(Some(Cfi::new("ESVUFR")?)))?);
    assert_eq!(registry.get("CH0012214059").and_then(IsinEntry::cficode).map(Cfi::as_str), Some("ESVUFR"));
    ```

=== "Python"

    ```python
    from yggdryl import IsinRegistry

    HOLCIM = "CH0012214059"
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "miccode": "XSWX", "ric": "HOLN.S", "ticker": "HOLN", "updunix": 10})
    # An older statement only fills; a newer one on another market switches the listing whole.
    assert not registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 5})
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ric": "HOLNl.L", "updunix": 20})
    row = registry.get(HOLCIM)
    assert row is not None and (row["miccode"], row["ric"], row["ticker"]) == ("XLON", "HOLNl.L", None)
    assert registry.get_by_ric("HOLN.S") is None, "a RIC names one listing at a time"
    # A refining CFI code refines whatever the time; a contradicting one only when newer.
    assert registry.merge({"isin": HOLCIM, "cficode": "ESVXXX", "updunix": 1})
    assert registry.merge({"isin": HOLCIM, "cficode": "ESVUFR", "updunix": 1})
    assert not registry.merge({"isin": HOLCIM, "cficode": "ESNUFR", "updunix": 1})
    assert registry.get(HOLCIM)["cficode"] == "ESVUFR"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IsinRegistry } = require('yggdryl')

    const HOLCIM = 'CH0012214059'
    const registry = new IsinRegistry()
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XSWX', ric: 'HOLN.S', ticker: 'HOLN', updunix: 10n }))
    // An older statement only fills; a newer one on another market switches the listing whole.
    assert.ok(!registry.merge({ isin: HOLCIM, ric: 'HOLN.VX', updunix: 5n }))
    assert.ok(registry.merge({ isin: HOLCIM, miccode: 'XLON', ric: 'HOLNl.L', updunix: 20n }))
    const row = registry.get(HOLCIM)
    assert.deepEqual([row.miccode, row.ric, row.ticker], ['XLON', 'HOLNl.L', null])
    assert.equal(registry.getByRic('HOLN.S'), null, 'a RIC names one listing at a time')
    // A refining CFI code refines whatever the time; a contradicting one only when newer.
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVXXX', updunix: 1n }))
    assert.ok(registry.merge({ isin: HOLCIM, cficode: 'ESVUFR', updunix: 1n }))
    assert.ok(!registry.merge({ isin: HOLCIM, cficode: 'ESNUFR', updunix: 1n }))
    assert.equal(registry.get(HOLCIM).cficode, 'ESVUFR')
    ```

## Persistence

`extend_from_arrow_reader` resolves each column of the reader's schema once, before a row is read: the registry's own name (`folds_equal`), any spelling or alias of an identifier type (`RIC`, `riccode`, `BloombergSymbol`, `ISINCode`), a field name a type is spelled by (`#ISINCODE`, `cusip_code`), or the registry's own market spellings (`cfi`/`cficode`, `mic`/`miccode`, `ticker`/`symbol`, `updunix`); a column naming nothing is ignored. Two columns naming one fact are refused naming both, and a stream with no `isin` column is refused naming the columns it has - a stream of no columns at all, what a missing store reads as, is the empty registry. One cast plan lands each batch under the resolved subset of the row, a cell a typed column cannot hold landing null; a typed code column's cells are adopted as the landing proved them, and a `utf8` column's through its type's value rule. Each row folds through `merge`, so rows of one ISIN - in one file, or across a folder's parts - fold by `updunix`, and a load past `max_instruments` is refused naming the row rather than truncated.

`extend_from_handle(handle)` reads the handle's own record stream: the media type, a file against a folder, and an object store are the handle's concern. `into_arrow_reader()` clones the table's `Arc` and lays the rows out one bounded batch at a time, holding one batch and never the table; a learn while it streams copies the table once and moves nothing the stream reads.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia, IOMode, IdType, Isin, IsinEntry, IsinRegistry, MimeType};

    let mut registry = IsinRegistry::new();
    registry.merge(IsinEntry::new(Isin::new("CH0012214059")?).try_with_code(IdType::Ric, "HOLN.S")?)?;

    // Any holder takes the snapshot through the record surface: an overwrite here,
    // a merge by `isin` to upsert.
    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?.with_field(IsinEntry::field());
    handle.write_arrow_reader(registry.into_arrow_reader()?, IOMode::Overwrite, &options)?;
    let loaded = IsinRegistry::from_handle(&handle)?;
    assert!(loaded.iter().eq(registry.iter()));
    ```

=== "Python"

    ```python
    import tempfile
    from pathlib import Path

    from yggdryl import IsinRegistry
    from yggdryl.holder import LocalFile

    registry = IsinRegistry()
    registry.merge({"isin": "CH0012214059", "ric": "HOLN.S"})
    with tempfile.TemporaryDirectory() as folder:
        target = Path(folder) / "instruments.arrow"
        LocalFile(target).overwrite_arrow_reader(registry.into_arrow_reader())
        loaded = IsinRegistry.from_handle(target)
        assert loaded.get("CH0012214059") == registry.get("CH0012214059")
        assert len(IsinRegistry.from_handle(Path(folder) / "missing.arrow")) == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, IsinRegistry } = require('yggdryl')

    const registry = new IsinRegistry()
    registry.merge({ isin: 'CH0012214059', ric: 'HOLN.S' })
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'instruments-'))
    try {
      const target = path.join(folder, 'instruments.arrow')
      new IOBase(target).overwriteArrowReader(registry.intoArrowReader())
      const loaded = IsinRegistry.fromHandle(target)
      assert.deepEqual(loaded.get('CH0012214059'), registry.get('CH0012214059'))
      assert.equal(IsinRegistry.fromHandle(path.join(folder, 'missing.arrow')).length, 0)
    } finally {
      fs.rmSync(folder, { recursive: true, force: true })
    }
    ```

## Bounds

| Bound | Value |
| --- | --- |
| `max_instruments` | `IsinRegistry::DEFAULT_MAX_INSTRUMENTS` (16,384) unless `with_max_instruments` says otherwise; no eviction |
| Memory | each instrument is charged its worst case - B-tree slack, twelve codes at their widest, the ticker, one RIC slot in the inverse index and one slot in the ticker index - 3 KiB, so a registry holds at most `max_instruments` × 3 KiB, 48 MiB at the default |
| Per row | `MAX_EQUIVALENTS` (12) codes: a row stating more, or a code in a `utf8` column its type refuses (past its `max_value_width`), is refused naming the row and the column, `$[1].wkn`. A typed code column's cell its datatype cannot hold lands null, and a ticker outside one to 64 bytes is stored as none; the rest of the row loads |
| Past the bound | `learn` skips a new ISIN with one warning per registry, a known ISIN learning on; `merge` and `extend_from_*` refuse, naming the bound |

## Edges

- Learning is the ordered lifecycle's, never a parse's: a parse depends only on its event, and a setter stays pure. A shared registry moved by walks run at once interleaves their learning; share one across walks run one after another.
- A registry's fill is a derivation: it never writes `CFICode(461)` or any FIX field, so a row's `cficode` shows a refined code only where 461 is unstated.
- A filled ISIN moves the [book](book.md) an element stands in: its [book key](market.md#the-book-key) is the ISIN once the registry fills it, so a ticker-only statement joins its instrument's book, and the candles it lands in.
- A ticker leads to a row only on the market its listing was stated on, or where the element or the row states none; a ticker two rows list there leads to none.
- One listing per ISIN: across venues, fills alternate rather than mix - never wrong, by the market gate, only sparse. A market read off `LastMkt(30)` or `ExDestination(100)` can file a routed order's RIC under the routed venue.
- A reused RIC can fill gaps in the old instrument's row through a statement keyed by it, and never replaces a value.

## Performance

`graph/isin_registry` over a registry of 4,096 instruments - each with a CFI code, a market, a ticker, a common code and a RIC - and `fix/pipeline/decoded_lifecycle` with and without a shared registry. One containerized x86_64 Linux run: Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.97.0, release profile with thin LTO. Criterion medians.

| Case | Median | What it does |
| --- | --- | --- |
| `learn_known` | 1.27 µs | a statement of a known ISIN saying nothing new: no row moves, nothing allocates |
| `learn_new` | 6.51 µs | a new ISIN with a CFI code and a common code, into a table no clone shares |
| `fill_by_isin` | 1.86 µs | an order naming only the ISIN, filled with the row's codes, ticker and CFI code, then finalized |
| `fill_by_ric` | 2.11 µs | the same order naming only the RIC: its ISIN derived first through the inverse index |
| `fill_miss` | 40.1 ns | an ISIN no row holds: one lookup, nothing built |
| `into_arrow_reader_4096` | 57.5 ms (71.3 K rows/s) | the snapshot stream drained: each row laid out as its named struct, through the row's value door |
| `extend_from_arrow_reader_4096` | 9.70 ms (422 K rows/s) | the same rows loaded into an empty registry: one cast plan, a code cell adopted as the landing proved it |
| `ipc_roundtrip_4096` | 68.8 ms (59.5 K rows/s) | the snapshot written to an Arrow IPC buffer and read back by `from_handle` |
| `decoded_lifecycle` | 1.11 s (12.0 MiB/s) | the decoded capture walked with the walk-local registry |
| `decoded_lifecycle_shared_registry` | 1.20 s (11.0 MiB/s) | the same walk learning into a registry the codec shares: one uncontended lock per message |

Writing a snapshot costs six times reading it back: a row crosses into Arrow as a named struct the row's value door checks and restates - seven allocations a row, pinned in `rust/tests/allocations.rs` - where a load adopts each landed code. The shared walk took 8.6% longer than the walk-local one over ten samples each, their intervals just apart, each walk starting from an empty registry: what taking the shared table's lock once per message costs.

```bash
cargo bench -p yggdryl --bench graph -- 'graph/isin_registry'
cargo bench -p yggdryl --bench fix -- '^fix/pipeline/decoded_lifecycle(_shared_registry)?$'
```

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root -- isin_registry
    cargo test -p yggdryl --features parquet --test root -- isin_registry
    cargo test -p yggdryl --test allocations -- isin_registry
    cargo test -p yggdryl --test fix -- enrich batch
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
