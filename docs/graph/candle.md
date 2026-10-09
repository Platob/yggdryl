# Candle

A candle is one OHLC of one book over one bucket: `Candle` what the books of one cross code whose instants fell in `[start, end)` read at their best bid, their best ask, their midpoint and their spread - each an `Ohlc` of open, high, low and close - with the quantities at the touch when the bucket closed and how many books it folded; `CandleOptions` the interval and the zone whose wall clock the buckets align to; `CandleIterator` the fold of a sorted stream of [books](book.md) into candles.

## Contract

| Type | Owns | Traits | Bindings |
| --- | --- | --- | --- |
| `Candle` | the cross code, the ticker, the bucket's `start` and `end` (nanoseconds UTC, `end` exclusive), the four readings `bid`, `ask`, `mid`, `spread`, the touch `bidqty`/`askqty` and the count `books`; `field()`, `into_scalar`, `from_scalar`, `arrow_reader` | `Clone`, `Debug`, `Eq`, `Hash`, `PartialEq` | Python `graph.Candle`, JavaScript `graph.Candle` - built by the walk or read back, never constructed |
| `Ohlc` | one reading's `open`, `high`, `low`, `close`; `at(value)` opens one, `fold(value)` moves the close and the high or low | `Clone`, `Copy`, `Debug`, `Eq`, `Hash`, `PartialEq` | Python a `dict` of four decimal `Scalar`s keyed `open`, `high`, `low`, `close`; JavaScript a `{ open, high, low, close }` of decimal text |
| `CandleOptions` | `new(interval)`, `from_spelling(text)`, `with_timezone(zone)`; `interval()`, `timezone()`, `spelling()` | `Clone`, `Debug`, `Eq`, `Hash`, `PartialEq` | Python `graph.CandleOptions(interval, timezone=None)`, JavaScript `new graph.CandleOptions(interval, timezone)` - a spelling, or a count of nanoseconds (`int`; `bigint` or a whole `number`) |
| `CandleIterator<I>` | the [fold](#the-fold): `new(books, options)`, `options()` | `Iterator<Item = Result<Candle>>`, `FusedIterator` | Python `graph.CandleIterator(books, options)` and `graph.candles(books, interval, timezone=None)`; JavaScript `new graph.CandleIterator(books, options)` and `graph.candles(books, options, timezone)` |

All in `graph::candle`, re-exported as `yggdryl::graph::{Candle, CandleIterator, CandleOptions, Ohlc}`. A candle is a value of its own rather than a datatype: its row is the struct [`Candle::field()`](#arrow-row) declares.

## Buckets

A bucket is `interval` nanoseconds of the zone's wall clock, so a daily candle opens at local midnight and an hourly one on the local hour.

| Key | Rule |
| --- | --- |
| Interval | `CandleOptions::new(nanoseconds)`, positive; else `InvalidRecord` at `$.interval` (`expected a positive count of nanoseconds, got 0`) |
| Spellings | `from_spelling` reads `<count><unit>` with the units `w`, `d`, `h`, `m`, `s`, `ms`, `us`, `ns`: `30s`, `1m`, `5m`, `1h`, `1d`, `1w`, `250ms`; `spelling()` writes the count and the widest unit dividing exactly - `90s`, `2m`, `1500ms`, `2w` - which `from_spelling` reads back. Refused at `$.interval`: an empty text, a bare unit (`m`), a zero (`0s`), a fraction (`1.5m`), a blank (`1 m`), a sign, another case (`1M`) or word (`1min`), and a count past `i64` nanoseconds |
| Zone | `with_timezone`, default `Timezone::UTC`; the bucket of an instant is found from its wall clock in that zone, `local.div_euclid(interval)`, so under `Asia/Kolkata` (+05:30) the hour holding `10:45Z` (`16:15` local) opens at `16:00`, `10:30Z`, where the UTC hour opens at `10:00Z`. A zone this build has no rules for is held by the options and refused when the walk first buckets an instant |
| Edges | an edge is the earliest instant whose wall clock reads at or after the local edge, so the edges rise with the instants and a sorted stream never re-enters a bucket it left. An instant's bucket is the last whose start is at or before it, searched from the index of its own wall clock by doubling steps and then bisection, so finding it costs `O(log n)` edge solves for a gap of `n` intervals - a fall-back's repeated hour at `1ns` included |
| Spring forward | `Europe/Zurich`, 2026-03-29, `01:00Z`: the wall clock skips `02:00`-`03:00`. Hourly candles open at the local hours `01`, `03`, `04` - the skipped hour yields no candle - and abut in UTC: `[00:00Z, 01:00Z)`, `[01:00Z, 02:00Z)`, `[02:00Z, 03:00Z)`. The daily candle is `[2026-03-28T23:00Z, 2026-03-29T22:00Z)`, twenty-three hours, both edges local midnight |
| Fall back | `Europe/Zurich`, 2026-10-25, `01:00Z`: the wall clock reads `02:00`-`03:00` twice. The hourly bucket `02` is one two-hour bucket, `[00:00Z, 02:00Z)`; with `30m` the `02:00` bucket opens once at `00:00Z` and the `02:30` bucket holds `[00:30Z, 02:00Z)` - every instant until the wall clock first reads `03:00` - so no bucket is entered twice, where a plain conversion of each local edge would reopen `02:00` an hour later. The daily candle is twenty-five hours |
| Range | an instant the zone cannot read, or a bucket that cannot be held in `i64` nanoseconds, is refused at `$.book.transunix` |

## The fold

`CandleIterator::new(books, options)` folds books sorted by their instant into one candle per cross code and bucket. A candle reads a book's top of book alone, which a [delta book](book.md#complete-books-and-delta-books) states as a complete one does, so a walk's books fold as they are yielded, none rebuilt.

| Key | Rule |
| --- | --- |
| Input | an `Iterator<Item = Result<BookEvent>>` - a [`BookIterator`](book.md#book-fold) is one - sorted by `get_transunix`; a regression is refused at `$.book.transunix` (`expected an instant at or after 2000, got 1000`), a `ValueError` in Python and an `Error` in JavaScript. The bindings take `BookEvent`s or `MarketData` holding one, so a table's rows fold as they arrive; another leaf is refused by the core's own narrowing, `$.kind: expected book_event, got order`, a `TypeError` in Python |
| Buckets | the candles of a bucket are emitted, in cross-code order, when the stream moves past the bucket and at the stream's end; an empty bucket yields no candle. The open bucket holds one fold per cross code until it closes |
| Failure | a source error, a regression or a range refusal ends the walk: the candles of every bucket the stream moved past - the refused book's own step included - are emitted before it, the open bucket is dropped rather than emitted incomplete, the error is the last item, and the iterator fuses |
| `crosscode`, `ticker` | the book's stored [`get_crosscode`](market.md#sides-and-cross-codes) - `3:0:` then its [book key](market.md#the-book-key), the ISIN, else the ticker, else `XX0000000000` (`3:0:ACME`, `3:0:CH0012214059`) - and the first book's `get_ticker`, null where it states none |
| `bid`, `ask` | over [`best_price(Side::Buy)`](book.md#limits) and `best_price(Side::Sell)` of every book that states one: the first opens the reading, each later one moves the close and the high or low; `None` where no book of the bucket stated one |
| `mid`, `spread` | over `bbo_midpoint()` and `spread()` the same way, so a one-sided book contributes to neither, and a bucket whose ask side empties keeps the ask, mid and spread the earlier books read |
| `bidqty`, `askqty` | the last book's `best_quantity` on each side - `None` when the last book has none, whatever an earlier one had |
| `books` | how many books folded |

## Arrow row

`Candle::field()` is the required struct `candle` of twenty-three cells, in this order:

| Cells | Datatype |
| --- | --- |
| `crosscode` | `utf8 not null` |
| `ticker` | `utf8` |
| `start`, `end` | `datetime64(ns, "UTC") not null` |
| `bidopen`, `bidhigh`, `bidlow`, `bidclose`, `askopen`, `askhigh`, `asklow`, `askclose`, `midopen`, `midhigh`, `midlow`, `midclose`, `spreadopen`, `spreadhigh`, `spreadlow`, `spreadclose` | `decimal`, nullable - a reading states all four or none |
| `bidqty`, `askqty` | `decimal` |
| `books` | `uint64 not null` |

| Key | Rule |
| --- | --- |
| `into_scalar()` | the named `Scalar::Struct` of the twenty-three cells, an absent ticker, reading or quantity a null |
| `from_scalar(value)` | reads that struct, or the ordered row the field's value door canonicalizes it to, through `Candle::field()`'s [`Field::scalar`](../types/field.md): a text price or a bare count lands as the column would hold it, a name the struct lacks is a null. Refused under `$.candle`: a missing cross code, instant or count, a cell of another datatype, a name the struct does not hold, and a reading stating some of its four cells and not the others |
| `arrow_reader(candles, batch_row_size)` | lays `Result<Candle>`s out as bounded batches under `field()`, lazily; `None` is the crate's default row bound; a source error follows the completed prefix and fuses the reader |
| Bindings | Python `Candle.field()`, `into_scalar()`, `from_scalar(value)` (a `Scalar`, a mapping of the same names or the ordered row) and `as_py()` (the twenty-three cells as native values: `Decimal`s, aware `datetime`s); JavaScript `Candle.field()`, `intoScalar()`, `fromScalar(value)` (a `Scalar` or a plain object), `toJSON()`/`fromJSON(value)` (the flat row as the JSON codec writes it - instants ISO 8601 text, decimals text, counts numbers - so `JSON.stringify` is exact), `equals`, `clone`. A batch is laid out through [`Serie`](../types/serie.md) from the rows `into_scalar` answers |

## Examples

### Minute candles

Four books of one minute: one quote a side, restated at each book, and three fills, which reach no book. The bid read `100`, `102`, `99`, `101` across them, so its candle opens at `100`, tops at `102`, bottoms at `99` and closes at `101`.

=== "Rust"

    ```rust
    use yggdryl::graph::{
        BookIterator, CandleIterator, CandleOptions, Element, Event, ExecutionEvent, Market, MarketData, Ohlc,
        QuoteEvent,
    };
    use yggdryl::{Decimal, Side, State};

    const SECOND: i64 = 1_000_000_000;
    let quote = |unix: i64, code: &str, side: Side, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut quote = QuoteEvent::at(unix);
        quote.set_crosscode(code.to_owned());
        quote.set_ticker(Some("ACME".into()), true);
        quote.set_side(side, true);
        quote.set_price(Some(price.parse()?), true);
        quote.set_quantity(Some(Decimal::from_int(quantity)), true);
        quote.set_state(State::New);
        quote.finalize();
        Ok(MarketData::from(quote))
    };
    let fill = |unix: i64, code: &str, side: Side, lastqty: Option<i64>| -> yggdryl::Result<MarketData> {
        let mut fill = ExecutionEvent::at(unix);
        fill.set_crosscode(code.to_owned());
        fill.set_ticker(Some("ACME".into()), true);
        fill.set_side(side, true);
        fill.set_price(Some("100".parse()?), true);
        // What the fill traded.
        fill.set_lastqty(lastqty.map(Decimal::from_int), true);
        fill.set_state(State::Filled);
        fill.finalize();
        Ok(MarketData::from(fill))
    };
    let operations = vec![
        quote(10 * SECOND, "B", Side::Buy, "100", 5)?,
        quote(10 * SECOND, "A", Side::Sell, "103", 7)?,
        quote(20 * SECOND, "B", Side::Buy, "102", 5)?,
        fill(20 * SECOND, "E1", Side::Buy, Some(4))?,
        quote(30 * SECOND, "A", Side::Sell, "102.5", 7)?,
        quote(30 * SECOND, "B", Side::Buy, "99", 5)?,
        quote(40 * SECOND, "B", Side::Buy, "101", 8)?,
        quote(40 * SECOND, "A", Side::Sell, "103.5", 9)?,
        fill(40 * SECOND, "E2", Side::Sell, Some(6))?,
        fill(40 * SECOND, "E3", Side::Sell, None)?,
    ];

    let books = BookIterator::new(operations.into_iter(), 0)?;
    let candles = CandleIterator::new(books, CandleOptions::from_spelling("1m")?)
        .collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(candles.len(), 1, "four books, one minute");
    let candle = &candles[0];
    let ohlc = |open: &str, high: &str, low: &str, close: &str| -> yggdryl::Result<Ohlc> {
        Ok(Ohlc { open: open.parse()?, high: high.parse()?, low: low.parse()?, close: close.parse()? })
    };
    assert_eq!((candle.crosscode.as_str(), candle.ticker.as_deref()), ("3:0:ACME", Some("ACME")));
    assert_eq!((candle.start, candle.end), (0, 60 * SECOND));
    assert_eq!(candle.bid, Some(ohlc("100", "102", "99", "101")?));
    assert_eq!(candle.ask, Some(ohlc("103", "103.5", "102.5", "103.5")?));
    assert_eq!(candle.mid, Some(ohlc("101.5", "102.5", "100.75", "102.25")?));
    assert_eq!(candle.spread, Some(ohlc("3", "3.5", "1", "2.5")?));
    // The touch when the bucket closed, and the four books the quotes made:
    // a fill moves no book.
    assert_eq!((candle.bidqty, candle.askqty), (Some(Decimal::from_int(8)), Some(Decimal::from_int(9))));
    assert_eq!(candle.books, 4);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    SECOND = 1_000_000_000

    def quote(unix: int, code: str, side: str, price: str, quantity: int) -> graph.QuoteEvent:
        return graph.QuoteEvent(
            unix, crosscode=code, ticker="ACME", side=side, price=Decimal(price), quantity=quantity, state="NEW"
        )

    def fill(unix: int, code: str, side: str, lastqty: int | None) -> graph.ExecutionEvent:
        # What the fill traded is its last quantity.
        return graph.ExecutionEvent(
            unix, crosscode=code, ticker="ACME", side=side, price=Decimal("100"), lastqty=lastqty, state="FILLED"
        )

    operations = [
        quote(10 * SECOND, "B", "BUYS", "100", 5),
        quote(10 * SECOND, "A", "SELL", "103", 7),
        quote(20 * SECOND, "B", "BUYS", "102", 5),
        fill(20 * SECOND, "E1", "BUYS", 4),
        quote(30 * SECOND, "A", "SELL", "102.5", 7),
        quote(30 * SECOND, "B", "BUYS", "99", 5),
        quote(40 * SECOND, "B", "BUYS", "101", 8),
        quote(40 * SECOND, "A", "SELL", "103.5", 9),
        fill(40 * SECOND, "E2", "SELL", 6),
        fill(40 * SECOND, "E3", "SELL", None),
    ]

    books = list(graph.BookIterator(operations))
    [candle] = graph.candles(books, "1m")
    assert candle == next(iter(graph.CandleIterator(books, graph.CandleOptions("1m"))))

    def reading(value: dict | None) -> tuple | None:
        return None if value is None else tuple(value[cell].as_py() for cell in ("open", "high", "low", "close"))

    assert (candle.crosscode, candle.ticker, candle.start, candle.end) == ("3:0:ACME", "ACME", 0, 60 * SECOND)
    assert reading(candle.bid) == (Decimal("100"), Decimal("102"), Decimal("99"), Decimal("101"))
    assert reading(candle.ask) == (Decimal("103"), Decimal("103.5"), Decimal("102.5"), Decimal("103.5"))
    assert reading(candle.mid) == (Decimal("101.5"), Decimal("102.5"), Decimal("100.75"), Decimal("102.25"))
    assert reading(candle.spread) == (Decimal("3"), Decimal("3.5"), Decimal("1"), Decimal("2.5"))
    # The touch when the bucket closed, and the four books the quotes made:
    # a fill moves no book.
    assert candle.bidqty is not None and candle.bidqty.as_py() == Decimal(8)
    assert candle.askqty is not None and candle.askqty.as_py() == Decimal(9)
    assert candle.books == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const SECOND = 1_000_000_000n
    const quote = (unix, code, side, price, quantity) => new graph.QuoteEvent(unix, {
      crosscode: code, ticker: 'ACME', side, price, quantity, state: 'NEW',
    })
    // What the fill traded is its last quantity.
    const fill = (unix, code, side, lastqty) => new graph.ExecutionEvent(unix, {
      crosscode: code, ticker: 'ACME', side, price: '100', lastqty, state: 'FILLED',
    })
    const operations = [
      quote(10n * SECOND, 'B', 'BUYS', '100', 5),
      quote(10n * SECOND, 'A', 'SELL', '103', 7),
      quote(20n * SECOND, 'B', 'BUYS', '102', 5),
      fill(20n * SECOND, 'E1', 'BUYS', 4),
      quote(30n * SECOND, 'A', 'SELL', '102.5', 7),
      quote(30n * SECOND, 'B', 'BUYS', '99', 5),
      quote(40n * SECOND, 'B', 'BUYS', '101', 8),
      quote(40n * SECOND, 'A', 'SELL', '103.5', 9),
      fill(40n * SECOND, 'E2', 'SELL', 6),
      fill(40n * SECOND, 'E3', 'SELL', undefined),
    ]

    const books = [...new graph.BookIterator(operations)]
    const [candle] = graph.candles(books, '1m')
    const [walked] = new graph.CandleIterator(books, new graph.CandleOptions('1m'))
    assert.ok(walked.equals(candle))

    assert.deepEqual([candle.crosscode, candle.ticker, candle.start, candle.end], ['3:0:ACME', 'ACME', 0n, 60n * SECOND])
    assert.deepEqual(candle.bid, { open: '100', high: '102', low: '99', close: '101' })
    assert.deepEqual(candle.ask, { open: '103', high: '103.5', low: '102.5', close: '103.5' })
    assert.deepEqual(candle.mid, { open: '101.5', high: '102.5', low: '100.75', close: '102.25' })
    assert.deepEqual(candle.spread, { open: '3', high: '3.5', low: '1', close: '2.5' })
    // The touch when the bucket closed, and the four books the quotes made:
    // a fill moves no book.
    assert.deepEqual([candle.bidqty, candle.askqty], ['8', '9'])
    assert.equal(candle.books, 4)
    ```

### A Zurich day

Empty books at `00:30Z`, `01:30Z` and `02:30Z` on 2026-03-29, the day Europe/Zurich springs forward: hourly candles open at the local hours `01`, `03` and `04` and abut in UTC, and the daily candle runs from local midnight to local midnight, twenty-three hours.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, CandleIterator, CandleOptions};
    use yggdryl::Timezone;

    const SECOND: i64 = 1_000_000_000;
    const HOUR: i64 = 3_600 * SECOND;
    // 2026-03-29T00:00:00Z: Europe/Zurich springs forward at 01:00Z.
    const DAY: i64 = 1_774_742_400 * SECOND;
    let zurich = Timezone::from_str("Europe/Zurich")?;
    let local_hour = |unix: i64| -> yggdryl::Result<i64> {
        Ok(zurich.into_local(unix / SECOND)?.rem_euclid(86_400) / 3_600)
    };
    let books = |instants: &[i64]| -> Vec<yggdryl::Result<BookEvent>> {
        instants.iter().map(|unix| Ok(BookEvent::new(*unix, "ACME"))).collect()
    };
    let instants = [DAY + 30 * 60 * SECOND, DAY + 90 * 60 * SECOND, DAY + 150 * 60 * SECOND];

    let hourly = CandleOptions::from_spelling("1h")?.with_timezone(zurich);
    let candles = CandleIterator::new(books(&instants).into_iter(), hourly).collect::<yggdryl::Result<Vec<_>>>()?;
    let edges: Vec<(i64, i64)> = candles.iter().map(|candle| (candle.start, candle.end)).collect();
    assert_eq!(edges, [(DAY, DAY + HOUR), (DAY + HOUR, DAY + 2 * HOUR), (DAY + 2 * HOUR, DAY + 3 * HOUR)]);
    let hours = candles.iter().map(|candle| local_hour(candle.start)).collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(hours, [1, 3, 4], "no candle opens at the hour the zone skipped");

    let daily = CandleOptions::from_spelling("1d")?.with_timezone(zurich);
    let candles = CandleIterator::new(books(&instants).into_iter(), daily).collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(candles.len(), 1);
    let candle = &candles[0];
    assert_eq!((candle.start, candle.end), (DAY - HOUR, DAY + 22 * HOUR));
    assert_eq!(candle.end - candle.start, 23 * HOUR);
    assert_eq!((local_hour(candle.start)?, local_hour(candle.end)?, candle.books), (0, 0, 3));

    // The same instants bucketed in UTC open on the UTC hour.
    let utc = CandleIterator::new(books(&instants).into_iter(), CandleOptions::from_spelling("1h")?)
        .collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(utc.iter().map(|candle| candle.start).collect::<Vec<_>>(), [DAY, DAY + HOUR, DAY + 2 * HOUR]);
    ```

=== "Python"

    ```python
    from yggdryl import Timezone, graph

    SECOND = 1_000_000_000
    HOUR = 3_600 * SECOND
    # 2026-03-29T00:00:00Z: Europe/Zurich springs forward at 01:00Z.
    DAY = 1_774_742_400 * SECOND
    zurich = Timezone("Europe/Zurich")

    def local_hour(unix: int) -> int:
        return zurich.into_local(unix // SECOND) % 86_400 // 3_600

    def books() -> list[graph.BookEvent]:
        return [graph.BookEvent(unix, "ACME") for unix in (DAY + 30 * 60 * SECOND, DAY + 90 * 60 * SECOND, DAY + 150 * 60 * SECOND)]

    hourly = graph.candles(books(), "1h", zurich)
    assert [(candle.start, candle.end) for candle in hourly] == [
        (DAY, DAY + HOUR),
        (DAY + HOUR, DAY + 2 * HOUR),
        (DAY + 2 * HOUR, DAY + 3 * HOUR),
    ]
    assert [local_hour(candle.start) for candle in hourly] == [1, 3, 4], "no candle opens at the hour the zone skipped"

    [daily] = graph.candles(books(), graph.CandleOptions("1d", "Europe/Zurich"))
    assert (daily.start, daily.end) == (DAY - HOUR, DAY + 22 * HOUR)
    assert daily.end - daily.start == 23 * HOUR
    assert (local_hour(daily.start), local_hour(daily.end), daily.books) == (0, 0, 3)

    # The same instants bucketed in UTC open on the UTC hour.
    assert [candle.start for candle in graph.candles(books(), "1h")] == [DAY, DAY + HOUR, DAY + 2 * HOUR]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Timezone, graph } = require('yggdryl')

    const SECOND = 1_000_000_000n
    const HOUR = 3_600n * SECOND
    // 2026-03-29T00:00:00Z: Europe/Zurich springs forward at 01:00Z.
    const DAY = 1_774_742_400n * SECOND
    const zurich = new Timezone('Europe/Zurich')
    const localHour = (unix) => Math.floor((zurich.intoLocal(Number(unix / SECOND)) % 86_400) / 3_600)
    const books = () => [DAY + 30n * 60n * SECOND, DAY + 90n * 60n * SECOND, DAY + 150n * 60n * SECOND]
      .map((unix) => new graph.BookEvent(unix, 'ACME'))

    const hourly = graph.candles(books(), '1h', zurich)
    assert.deepEqual(hourly.map((candle) => [candle.start, candle.end]), [
      [DAY, DAY + HOUR],
      [DAY + HOUR, DAY + 2n * HOUR],
      [DAY + 2n * HOUR, DAY + 3n * HOUR],
    ])
    assert.deepEqual(hourly.map((candle) => localHour(candle.start)), [1, 3, 4], 'no candle opens at the hour the zone skipped')

    const [daily] = graph.candles(books(), new graph.CandleOptions('1d', 'Europe/Zurich'))
    assert.deepEqual([daily.start, daily.end], [DAY - HOUR, DAY + 22n * HOUR])
    assert.equal(daily.end - daily.start, 23n * HOUR)
    assert.deepEqual([localHour(daily.start), localHour(daily.end), daily.books], [0, 0, 3])

    // The same instants bucketed in UTC open on the UTC hour.
    assert.deepEqual(graph.candles(books(), '1h').map((candle) => candle.start), [DAY, DAY + HOUR, DAY + 2n * HOUR])
    ```

### The Arrow round trip

A candle laid out as one row under `Candle::field()` and read back as the same value.

=== "Rust"

    ```rust
    use yggdryl::graph::{Candle, Ohlc};
    use yggdryl::{ArrowCastOptions, Decimal, Serie};

    let field = Candle::field()?;
    assert_eq!((field.name(), field.field_len(), field.is_nullable()), ("candle", 23, false));
    let names: Vec<&str> = field.fields().iter().map(|child| child.name()).collect();
    assert_eq!(&names[..5], ["crosscode", "ticker", "start", "end", "bidopen"]);
    assert_eq!(&names[20..], ["bidqty", "askqty", "books"]);
    assert_eq!(field.fields()[2].dtype().to_string(), "datetime64(ns,\"UTC\")");

    let candle = Candle {
        crosscode: "3:0:ACME".into(),
        ticker: Some("ACME".into()),
        start: 60_000_000_000,
        end: 120_000_000_000,
        bid: Some(Ohlc::at("99.5".parse()?)),
        ask: None,
        mid: None,
        spread: None,
        bidqty: Some(Decimal::from_int(300)),
        askqty: None,
        books: 1,
    };
    // One row out, one value back: through Arrow, and through the scalar alone.
    let mut batches = Candle::arrow_reader([Ok(candle.clone())], None)?;
    let batch = batches.next().expect("one batch")?;
    assert_eq!((batch.num_rows(), batch.num_columns()), (1, 23));
    let rows = Serie::from_arrow_batch(Some(&field), &batch, ArrowCastOptions::default())?;
    assert_eq!(Candle::from_scalar(&rows.scalar(0)?)?, candle);
    assert_eq!(Candle::from_scalar(&candle.into_scalar())?, candle);
    // A reading states its four cells or none.
    let mut half = candle.into_scalar();
    half = half.with_field("bidhigh", yggdryl::Scalar::Null)?;
    assert!(Candle::from_scalar(&half).unwrap_err().to_string().contains("$.candle"));
    ```

=== "Python"

    ```python
    import datetime
    from decimal import Decimal

    from yggdryl import Serie, graph

    field = graph.Candle.field()
    names = [child.name for child in field.dtype]
    assert (field.name, len(names), field.nullable) == ("candle", 23, False)
    assert names[:5] == ["crosscode", "ticker", "start", "end", "bidopen"]
    assert names[20:] == ["bidqty", "askqty", "books"]
    assert str(field.dtype[2].dtype) == 'datetime64(ns,"UTC")'

    # A mapping of the cells reads as the column would hold them; a name left
    # out is a null.
    candle = graph.Candle.from_scalar(
        {"crosscode": "3:0:ACME", "ticker": "ACME", "start": 60 * 10**9, "end": 120 * 10**9, "bidopen": "99.5",
         "bidhigh": "99.5", "bidlow": "99.5", "bidclose": "99.5", "bidqty": 300, "books": 1}
    )
    assert candle.ask is None and candle.bid is not None and candle.bid["open"].as_py() == Decimal("99.5")

    # One row out, one value back: through Arrow, and through the scalar alone.
    rows = Serie.from_scalars(field, [candle.into_scalar()])
    batch = rows.into_arrow_batch()
    assert (batch.num_rows, batch.num_columns) == (1, 23)
    assert graph.Candle.from_scalar(rows.scalar(0)) == candle
    assert graph.Candle.from_scalar(candle.into_scalar()) == candle
    native = candle.as_py()
    assert native["start"] == datetime.datetime(1970, 1, 1, 0, 1, tzinfo=datetime.timezone.utc)
    assert (native["bidclose"], native["askopen"], native["books"]) == (Decimal("99.5"), None, 1)
    # A reading states its four cells or none.
    try:
        graph.Candle.from_scalar({**native, "bidhigh": None})
    except ValueError as error:
        assert "$.candle" in str(error)
    else:
        raise AssertionError("a half reading was read")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Serie, graph } = require('yggdryl')

    const field = graph.Candle.field()
    const names = Array.from({ length: field.fieldLen }, (_, at) => field.fieldAt(at).name)
    assert.deepEqual([field.name, names.length, field.nullable], ['candle', 23, false])
    assert.deepEqual(names.slice(0, 5), ['crosscode', 'ticker', 'start', 'end', 'bidopen'])
    assert.deepEqual(names.slice(20), ['bidqty', 'askqty', 'books'])
    assert.equal(String(field.fieldAt(2).dtype), 'datetime64(ns,"UTC")')

    // A plain object spelling the cells reads as the column would hold them;
    // a name left out is a null.
    const candle = graph.Candle.fromScalar({
      crosscode: '3:0:ACME', ticker: 'ACME', start: 60_000_000_000n, end: 120_000_000_000n,
      bidopen: '99.5', bidhigh: '99.5', bidlow: '99.5', bidclose: '99.5', bidqty: 300, books: 1,
    })
    assert.equal(candle.ask, null)
    assert.deepEqual(candle.bid, { open: '99.5', high: '99.5', low: '99.5', close: '99.5' })

    // One row out, one value back: through a Serie, and through the scalar alone.
    const rows = Serie.fromScalars(field, [candle.intoScalar()])
    assert.equal(rows.length, 1)
    assert.ok(graph.Candle.fromScalar(rows.scalar(0)).equals(candle))
    assert.ok(graph.Candle.fromScalar(candle.intoScalar()).equals(candle))
    // The JSON is the flat row: ISO 8601 instants, decimal text, counts as numbers.
    const json = candle.toJSON()
    assert.deepEqual([json.start, json.bidclose, json.askopen, json.bidqty, json.books],
      ['1970-01-01T00:01:00Z', '99.5', null, '300', 1])
    assert.ok(graph.Candle.fromJSON(JSON.stringify(candle)).equals(candle))
    // A reading states its four cells or none.
    assert.throws(() => graph.Candle.fromScalar({ ...json, bidhigh: null }), /\$\.candle/)
    ```
