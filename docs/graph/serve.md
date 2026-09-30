# Book display

`yggdryl market serve` hosts the book display over tables of market data: the Node.js components of `node/book/`, embedded in the binary so the page a browser opens and the page the npm package ships are one source, in front of `BookService` - the HTTP face of a [`marketdata`](market-data.md#arrow) table, answering the tickers it holds, the [candles](candle.md) a ticker's [books](book.md) fold into, the book standing at an instant and the audit of every entry, delta and execution as JSON, and that audit as a CSV download. The display draws the bid and ask candles of a range, the mid and the spread, and on a selected bucket shows the last book and its events on each side; it reads dark or light from the system or its toggle, is reachable from the keyboard, and carries the selection in the URL hash so a view is a link.

## Contract

| Type | Owns | Traits | Bindings |
| --- | --- | --- | --- |
| `BookService` | `new(options)`, `with_table(name, holder)` (a name already listed is replaced), `tables()`, `options()`, `events_field()`; `route(self: Arc<Self>, server, prefix)` answers the [routes](#routes) under `{prefix}/api` on an [`http::Server`](../holder/index.md#serving-a-handle) and returns the endpoint; the readings behind them - `tickers(table)`, `candles(&query)`, `book(table, ticker, at)`, `events(&query)` - answer without HTTP exactly what the routes spell | `Debug` | Rust-only: Python and JavaScript reach the display through [the command](#the-command) and `node/book.js` |
| `BookServiceOptions` | `snapshot_millis` (`0`, the grid a capture is folded on before it lands - the service re-folds nothing), `max_event_rows` (`DEFAULT_MAX_EVENT_ROWS`, `5_000`: what `/api/events` answers at most, a stated `0` being one); `with_snapshot_millis`, `with_max_event_rows` | `Clone`, `Debug`, `Default`, `Eq`, `Hash`, `PartialEq` | - |
| `BookTable` | one served table: `name()` (what every route's `table` parameter states) and `holder()` (the location the books are read from) | `Debug` | - |
| `BookQuery` | one question about one ticker's books: `table`, `ticker`, `from`, `to` (nanoseconds UTC, `to` exclusive), `timezone`, `interval` (`None` a minute in `timezone`), `side` (`None` both); `from_parameters(&Parameters)` reads it off a query string, `candle_options()` answers how the candles are bucketed | `Clone`, `Debug`, `Eq`, `Hash`, `PartialEq` | - |

All in `graph::serve`, under the `http` feature, re-exported as `yggdryl::graph::{BookQuery, BookService, BookServiceOptions, BookTable}`. A table is any record location a [`Holder`](../holder/index.md#handles) reads books from - an Iceberg folder (the `iceberg` feature), an Arrow, Parquet, Avro or CSV leaf, a partitioned folder - and every reading is one filtered read of it, `marketdatakind = 'BOOK'`, the ticker and the instants pushed into the location's own [record options](../holder/index.md#column-pushdown), so a store that prunes on them prunes; the `BOOK` rows come back as `BookEvent`s through [`MarketData::from_arrow_reader`](market-data.md#arrow), sorted by their instant. Where the location declares no field, the read declares the [`marketdata` row](market-data.md#columns): a CSV leaf, whose cells state no nested type, reads them as the row types them, and an empty or absent store - a zero-byte leaf, a file removed while serving, a folder holding no leaf or not there yet - is the empty reading, no ticker listed and every ticker a `404`, rather than a failure; a folder whose leaves no record encoding reads keeps that refusal, a `500`. Nothing is cached: every request reads the table as it stands.

## The command

```text
yggdryl market serve [TABLE...] [--bind 127.0.0.1:8080] [--path /] [--snapshot-millis 0]
                     [--capture LOG]... [--registry config/fix] [--rowheader REGEX] [--timezone UTC]
                     [--public-url URL] [--trusted-proxy IP|CIDR]... [--forwarded-header FIELD]...
                     [--path-prefix PREFIX] [--read-timeout 30] [--max-body 16777216] [--trace FOLDER]
```

| Argument | Default | States |
| --- | --- | --- |
| `TABLE` | none | a table to serve, `name=location` or a location alone named after its last segment: an Iceberg table folder, a record leaf (`.arrows`, `.parquet`, `.avro`, `.csv`) or a partitioned folder; a `://` URL goes through `Holder::from_url`. A path where nothing is yet is taken as a folder - the one a capture makes a table of; served with no capture, it is a table holding no book, and nothing is created. Two tables of one name are refused, since a route's `table` names one |
| `--bind` | `127.0.0.1:8080` | the address to listen on; port `0` takes a free one |
| `--path` | `/` | the path the display answers under: its page is `<path>/index.html`, which `<path>` itself sends a browser to, and the routes stand under `<path>/api` |
| `--snapshot-millis` | `0` | the [grid](book.md#book-fold) a capture's books are folded on before they land, milliseconds: a positive one adds the whole live book at every tick the capture crosses, zero only the books its instants touch |
| `--capture` | none, repeatable | a FIX bridge log folded into the *first* table before serving: its lines read under `--rowheader` and `--timezone`, [walked](../fix/lifecycle.md) as the chains they belong to, folded into books through `FixCodec::market_data` and `BookIterator`, and appended as `BOOK` rows through the table's own record options - an Iceberg folder through its table, a leaf under its encoding. Every capture is read before any lands, and they land in one append - one commit on an Iceberg table |
| `--registry` | `config/fix` | the FIX [dictionary](../fix/store.md) a capture is read with, a folder resolved against the working directory: the default is the dictionary a yggdryl checkout commits, and neither the wheel nor the npm package ships one, so a command installed from a registry names the folder it keeps one in. Read only where a capture is |
| `--rowheader` | `yggdryl::ULBRIDGE_ROWHEADER` | the row header every capture line opens with, a regex of named captures ([ULBridge](../fix/capture.md)) |
| `--timezone` | `UTC` | the zone the bridge's clock writes its lines in; an IANA zone this build has rules for, or a fixed offset |
| `--max-body` | `16777216` | the largest request body accepted, bytes |
| `--trace` | none | a folder every exchange is written under, `NNNN-request.http` as read and `NNNN-response.http` as sent ([trace](../holder/index.md#serving-a-handle)) |
| `--public-url`, `--trusted-proxy`, `--forwarded-header`, `--path-prefix`, `--read-timeout` | none, none, `X-Forwarded-For` and `X-Forwarded-Proto`, none, `30` | the server's own options of those names [behind a reverse proxy](../holder/index.md#behind-a-reverse-proxy): `with_public_url`, `with_trusted_proxies`, `with_forwarded_headers`, `with_path_prefix`, `with_read_timeout` |

In run order: every argument is read - `--path` by the server's own path grammar, every table and capture location resolved - then the port is taken and the endpoint spelled on it, then every capture is read and folded, then they land, then the service and the display are routed and the endpoint is printed - a refused argument costs no bind, and a refused bind, endpoint or capture no ingest, because a capture appends and running it twice lands its books twice. The first line printed is the endpoint on the socket, alone, so a script that started the process reads where to connect before anything else: the folder `<path>/` the display stands in, ending in a slash so the page's relative files and its `api/` resolve under it - `http://127.0.0.1:8080/book/` under `--path /book`, `http://127.0.0.1:8080/` at the root; then `· public endpoint <url>` under `--public-url`, then one note per table, `· table <name> over <url>`, ending `, created` for a folder the command made a table of, then one per capture, `· capture <log>: <n> books into <name>`. Under the `iceberg` feature an empty or absent first folder becomes an Iceberg table before a capture lands, its schema the `marketdata` row as Iceberg states it (`MarketData::field().into_scheme_compat(&Scheme::ICEBERG)`: the three `uint64` codes widened to `decimal(20, 0)`, every enum a bare `int32`); the `yggdryl` a wheel ships is built with that feature. Without it nothing is made a table: a leaf takes the rows under its own encoding, and a folder as a partitioned folder does - which, with the `parquet` feature off in that build, refuses them by name; that build's `--help` states no capture into a folder, since every example it states runs in it. The page is `<path>/index.html` under `text/html; charset=utf-8` - the root answers it too - and the seven other files stand beside it under their own types, every one `Cache-Control: no-cache`. A page's relative `theme.css`, `app.js` and `api/` resolve against the folder its URL ends in, and the server sends `<path>/` to the bare `<path>`, so the bare `<path>` answers `308` with the relative `Location` `<last segment>/index.html`, the query kept: a browser opening `<path>` or `<path>/` lands on the page, under whatever prefix a proxy adds.

| Refused before anything is served | At |
| --- | --- |
| `--capture` with no `TABLE` (`a capture needs a table to land in: expected a TABLE beside --capture, got none`) | `$.capture` |
| `--timezone` naming a zone this build has no rules for (`expected an IANA zone this build has rules for, or a fixed offset, got "Mars/Olympus"`) | `$.timezone` |
| `--rowheader` that does not compile | `$.rowheader` |
| `--registry` naming a folder that lists nothing (`expected a FIX dictionary at "file:///...", got nothing`) - a store with nothing in it would load as the crate's own fields alone and read every line as unknown | the folder's URL |
| Two `TABLE`s of one name, by `name=` or by last segment (`expected one table per name, got "books" over both "/data/a/books" and "/data/b/books"`) | `$.tables[<index>]`, the second of them |
| `--path` carrying a query, a fragment or a control byte, or a byte a URL cannot carry - refused before any capture is read | the server's own path grammar, or the URL grammar, by byte |
| `--capture` naming a location the URL grammar refuses, or a capture that cannot be read - refused before any capture lands | the location, or the capture's own refusal |
| `--snapshot-millis` that is no count, `--read-timeout` outside 1 to 86400, `--forwarded-header` naming no forwarded field (`X-Real-IP`), `--bind` on an address that cannot be taken | the argument, or the socket |

A refusal is printed on stdout in place of the endpoint, after the report of anything the core warned about, as one `✗` line - `✗ invalid record value at $.capture: a capture needs a table to land in: expected a TABLE beside --capture, got none` - running on to further lines where its reason does (a `--rowheader` regex error points at the pattern), and the command exits `1`. What the argument parser refuses itself - a value that is no count, out of range or no forwarded field, an argument the command does not take - it writes on stderr instead, clap's `error: invalid value '0' for '--read-timeout <SECONDS>': ...`, and exits `2`. `node/book.js`'s `serve()` rejects with that refusal, then that stderr, as its error's message.

## Routes

Every route is a `GET` under `{prefix}/api` - `HEAD` is answered off it, `POST` is the server's `405`, a leaf that is none of these its `404` - and every answer states `Content-Type` and `Cache-Control: no-store`. A JSON answer is `application/json`; a refusal is JSON `{"error": "<text>"}`, `400` for a parameter the route cannot read (a refusal located at `$.table`, `$.ticker`, `$.from`, `$.to`, `$.at`, `$.tz`, `$.interval`, `$.side` or `$.limit`), `404` for a table, a ticker or a book there is none of, `500` otherwise, whose whole error the service writes to standard error, the server's own log.

No answer carries what authenticates a location: a table's `url` and the text of every refusal are stated without the user information before the host and the query after the path - where a password, an Azure SAS `sig=` or another signature travels - so `https://alice:secret@host/books.arrows?sig=...` is listed, and named by a failed read, as `https://host/books.arrows`. The readings without HTTP are the server's own and keep the whole error.

| Route | Parameters | Answer |
| --- | --- | --- |
| `tables` | - | `[{"name","url"}]`, each location without its user information and its query, `url` null for a location that has none |
| `timezones` | - | `["UTC", ...]`: `UTC`, then every zone this build has rules for (`Timezone::registered()`) by name - the place zones `tz` reads, which reads their aliases and fixed offsets besides. A display offers these rather than its runtime's own list, which names zones this build has no rules for |
| `tickers` | `table` | `[{"ticker","crosscode","from","to","books"}]` ordered by ticker, `from` the whole second the first book stands in and `to` the whole second after the last, both UTC, so `[from, to)` holds every book and a display passes them straight back as a range - the first or the last instant `i64` nanoseconds hold where that second lies past them, `1677-09-21T00:12:43.145224192Z` and `2262-04-11T23:47:16.854775807Z`; a book stating no ticker is not listed, since no query can name it, and an empty or absent table lists none. One projected scan, `select ticker, crosscode, currunix where marketdatakind = 'BOOK'` |
| `candles` | `table`, `ticker`, `from`, `to`, `tz`, `interval` | `{"table","ticker","timezone","interval","from","to","candles":[..]}`, `interval` the effective spelling, each candle `{start,end,bid,ask,mid,spread,bidqty,askqty,books,executions,volume}` with each reading `{open,high,low,close}` or null - [`Candle`](candle.md) cell for cell, the books folded by [`CandleIterator`](candle.md#the-fold) under `interval` aligned to `tz`. A candle's `volume` counts a trade once over it and the candle just before it ([volume](candle.md#volume)): a trade stated again only after a candle that did not name it counts again, so the volumes of candles further apart need not add up to what traded. A known ticker with no book in range answers `"candles": []`; an unknown one is `404` |
| `book` | `table`, `ticker`, `at`, `tz` | the last book at or before `at`: `{currunix,ticker,crosscode,bestbid,bestask,bidqty,askqty,spread,midpoint,imbalance,islocked,iscrossed,alive,deltas,executions,bidlimits,asklimits}` - `imbalance` over the first level, `alive`, `deltas` and `executions` the counts of its entries (the entries are `events`), each side `[{price,quantity,uuids,tradable}]` best first ([`Limit`](book.md#limits)); none there yet is `404` (`expected a book at "books/ACME at or before <instant>", got nothing`) |
| `events` | `table`, `ticker`, `from`, `to`, `tz`, `side`, `limit` | `{"rows":[..],"truncated":bool}`: for every book of the ticker in `[from, to)`, in instant order, one row per alive entry (role `alive`), per delta applied since the book before (`delta`) and per execution at its instant (`execution`), each an object keyed by the fifty-one names of `BookService::events_field()` - `bookunix`, `role`, then `marketdatakind` and every flat column of the [`marketdata` row](market-data.md#columns), the nested `alive`, `deltas`, `executions`, `bidlimits` and `asklimits` left out - at most `limit` rows, `truncated` saying whether more were kept back. The bound is pushed into the read: one projected scan of the range's `currunix` finds the instant by which its earliest `limit + 1` books stand, and only the books up to it are read and built - that count doubled while they state no more than `limit` rows on `side` and the range holds more - so a request holds at most `limit + 1` books and instants whatever its range, a table stored in no instant order included |
| `audit.csv`, `audit.csv.gz`, `audit.csv.zst` | `table`, `ticker`, `from`, `to`, `tz`, `side` | the same rows unbounded, as [the download](#the-audit-download) |

| Parameter | Rule |
| --- | --- |
| `table`, `ticker` | required; a table the service does not hold and a ticker the table holds no book of are `404` |
| `from`, `to`, `at` | ISO 8601 instants with seconds: one stating an offset or `Z` is that instant - an answer's own RFC 9557 text included, its `+` and brackets percent-encoded as any query value is - and a naive one (`2026-08-14T00:00:00`, what a `datetime-local` input spells) a wall clock in `tz`; `to` is exclusive, and `from` not before `to` is refused at `$.to` (``expected an instant after `from` (...), got ...``) |
| `tz` | an IANA zone this build has rules for, default `UTC`; the zone naive instants are read in and every instant of the answer is rendered in |
| `interval` | a [candle spelling](candle.md#buckets), default `1m`, aligned to `tz` |
| `side` | `bid` or `ask`, any case, keeping the entries and deltas resting on that side and the executions stating it; absent keeps both |
| `limit` | the most `events` rows, default and cap `max_event_rows` |

Instants in an answer are the crate's canonical zoned spelling, RFC 9557 - nine fraction digits and, for a place zone, the offset with the bracketed name, `2026-08-14T14:00:00.000000000+02:00[Europe/Zurich]`, UTC as `2026-08-14T12:00:00.000000000Z` - and are what a display sends straight back as the `from`, `to` and `at` of its next question. Decimals are text, so nothing is rounded; UUIDs are their canonical text; the counts - `books`, `executions`, `alive`, `deltas` - and an `events` row's integer columns are JSON integers.

## The audit download

`audit.csv`, `audit.csv.gz` and `audit.csv.zst` answer the rows `events` answers, unbounded, written by the [CSV medium](../media/index.md#csv) into a buffer whose media type is the suffix's - `text/csv` under the coding the name carries - and served under that coding's own `Content-Type`, `text/csv`, `application/gzip` or `application/zstd`, with `Content-Disposition: attachment; filename="audit-<ticker>-<from>-<to>.<suffix>"`, the two instants to the second in UTC, `20260813T220000Z`, and the ticker kept to `A-Z`, `a-z`, `0-9`, `.`, `_` and `-`, any other character `_`. The header line names the fifty-one columns, `bookunix,role,marketdatakind,currunix,...`, and the file reads back through any handle named with the same suffix.

## The components

The display is vanilla ES modules with no framework, no CDN and no build step: eight files under `node/book/`, shipped in the npm package beside `book.js` and embedded by the command.

| File | Owns |
| --- | --- |
| `index.html` | the shell: the table, ticker, from, to, timezone and interval selectors and the theme toggle; the chart with its legend; the point summary; the bid and ask audit tables with the download control; a five-line script stamping the stored theme before the first paint |
| `theme.css` | the design tokens on `:root` (`--bg`, `--fg`, `--muted`, `--panel`, `--border`, `--bid`, `--ask`, `--mid`, `--spread`, `--accent`), redefined under `prefers-color-scheme: dark` and again under `[data-theme="dark"]`; a dense layout that works at phone width, visible focus rings, `prefers-reduced-motion` honoured |
| `theme.js` | `THEMES` (`system`, `light`, `dark`), `resolveTheme`, `applyTheme` (stamps `data-theme` on `<html>`), `toggleTheme`, the `localStorage` key `yggdryl-book-theme` behind `try`/`catch` |
| `api.js` | one function per route - `fetchTables`, `fetchTimezones`, `fetchTickers`, `fetchCandles`, `fetchBook`, `fetchEvents`, `auditUrl` - each building its query string from a plain object, pure over an injectable `fetch`; `ApiError` carries the service's `error` text and status |
| `chart.js` | `drawCandles(canvas, candles, options)` on a 2D canvas at the device pixel ratio: the bid candles left and the ask candles right of every bucket, the mid close as a line, the spread as a lower band, the axes in the chosen zone; hover crosshair and tooltip, click and keyboard selection; `scaleLinear`, `niceTicks`, `layoutCandles`, `nearestCandle` exported pure, so the layout is tested without a DOM |
| `audit.js` | `renderSummary` (best bid and ask, spread, mid, imbalance, the counts), `renderEvents` (a sortable table of `currunix`, `role`, `marketdatakind`, `side`, `price`, `quantity`, `state`, `crosscode`, `curruuid`, `prevuuid`), `downloadLink`; `formatInstant` and `formatDecimal` exported pure |
| `app.js` | the state: the API base off `document.baseURI` (so `--path /book` serves the routes at `/book/api/`), the tables, the zones `timezones` lists (the browser's own the default only where it is listed, else `UTC`), the tickers of the chosen table with `from` and `to` set to the ticker's span, the candles on any selector change (debounced, a reply to a superseded selection dropped), the chart, and on a selected bucket its last book - asked strictly before the candle's end - and the events of its range on each side; the download points at the whole range. The range is held as instants and sent as UTC `Z` text, so a zone change re-renders the inputs rather than moving the span. `HASH_KEYS` - `table`, `ticker`, `from`, `to`, `tz`, `interval`, `at` - are read from and written to the URL hash, so a view is shareable; `INTERVALS` offers `30s`, `1m`, `5m`, `15m`, `1h`, `1d` |
| `favicon.svg` | a two-candle mark |

The chart is focusable: `ArrowLeft`/`ArrowRight` move the hover to the neighbouring bucket, `Home`/`End` to the first and last, `Enter` or `Space` selects it, `Escape` clears the hover, and a live readout beside the canvas states the hovered candle in text; a skip link leads to it, and a `404` on the book route renders as an empty state naming it rather than an error.

`node/book.js`, CommonJS and declared for TypeScript by `node/book.d.ts`, is the package's door to all of it, `require('yggdryl/book')`:

```text
book.assets                    // the absolute `book/` folder
book.assetFiles                // the eight files above, in the order the command embeds them
book.serveArguments(options)   // the argument vector of { tables, bind = '127.0.0.1:0', path = '/', capture, args }
book.serve(options)            // spawns `yggdryl market serve` with it -> Promise<{ endpoint, process, close() }>; `signal` cancels
```

A table is `'name=location'`, a location or `{ name, location }`. `serve` runs `bin` - `YGGDRYL_BIN`, else `yggdryl` on the path - under `env` and resolves once the endpoint line is read from stdout. A process that exits first rejects with what it printed as the error's message: the command's [refusal](#the-command) - its `✗` line and any line the refusal runs on to - then its stderr, where the argument parser refuses; one that printed neither rejects with `yggdryl market serve exited with <code> before printing its endpoint`. `serve` rejects with the spawn error when the process cannot start, with the line when the first one is neither the endpoint, a refusal nor the warning report ahead of one, and with an `AbortError` quoting what it printed once an aborted `signal` has ended the process - `AbortSignal.timeout(ms)` bounds the wait. Until it resolves, the process ends with its parent.

## Examples

### The service over a buffer

Two minutes of `ACME` quotes folded into books and written as `marketdata` rows into a buffer named `books.arrows`, served as the table `books` on a loopback server and asked for its tickers and its minute candles - and asked again without HTTP. Python and JavaScript have no `BookService`: they reach the display through `yggdryl market serve` and, in Node, `book.serve(...)` below.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{
        BookIterator, BookQuery, BookService, BookServiceOptions, Element, Event, Market, MarketData,
        QuoteEvent,
    };
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::http::{Request, Server, Status};
    use yggdryl::{Decimal, IOMedia, Scalar, Side, State, Timezone, Url};

    const SECOND: i64 = 1_000_000_000;
    // 2026-01-05T10:00:00Z.
    const T0: i64 = 1_767_607_200 * SECOND;
    let quote = |unix: i64, code: &str, side: Side, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut quote = QuoteEvent::at(unix);
        quote.set_crosscode(code.to_owned());
        quote.set_ticker(Some("ACME".into()));
        quote.set_side(side);
        quote.set_price(Some(price.parse()?));
        quote.set_quantity(Some(Decimal::from_int(quantity)));
        quote.set_state(State::New);
        quote.finalize();
        Ok(MarketData::from(quote))
    };
    let operations = vec![
        quote(T0 + 5 * SECOND, "B1", Side::Buy, "100", 10)?,
        quote(T0 + 5 * SECOND, "A1", Side::Sell, "101", 5)?,
        quote(T0 + 65 * SECOND, "B2", Side::Buy, "100.5", 4)?,
    ];
    let books = BookIterator::new(operations.into_iter(), 0)?.map(|book| book.map(MarketData::from));
    let mut holder =
        Holder::Buffer(Buffer::new().with_media_type(Url::from_str("file:///books.arrows")?.media_type()));
    let options = holder.record_options()?;
    holder.overwrite_arrow_reader(MarketData::arrow_reader(books, None, None)?, &options)?;

    let service = Arc::new(BookService::new(BookServiceOptions::new()).with_table("books", holder));
    let server = Server::bind("127.0.0.1:0")?;
    let endpoint = Arc::clone(&service).route(&server, "/")?;

    // The zones `tz` reads: UTC, then every zone this build has rules for.
    let zones = Request::get(&format!("{endpoint}api/timezones"))?.send()?.scalar()?;
    let zones = zones.sequence_rows().expect("a list");
    assert_eq!(zones[0].as_str(), Some("UTC"));
    assert!(zones.iter().any(|zone| zone.as_str() == Some("Europe/Zurich")));

    // The tickers a table holds: the span of each, to the second, in UTC.
    let answer = Request::get(&format!("{endpoint}api/tickers?table=books"))?.send()?;
    assert_eq!(answer.status(), Status::OK);
    assert_eq!(answer.headers().get("cache-control"), Some("no-store"));
    let listed = answer.scalar()?;
    let acme = listed.sequence_rows().expect("a list")[0].clone();
    let acme = acme.as_struct().expect("an object");
    assert_eq!(acme["ticker"].as_str(), Some("ACME"));
    assert_eq!(acme["from"].as_str(), Some("2026-01-05T10:00:05.000000000Z"));
    assert_eq!(acme["to"].as_str(), Some("2026-01-05T10:01:06.000000000Z"));
    assert_eq!(acme["books"], Scalar::from(2_u64));

    // The minute candles of the range, `to` exclusive.
    let answer = Request::get(&format!(
        "{endpoint}api/candles?table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T10:02:00Z&interval=1m"
    ))?
    .send()?;
    assert_eq!(answer.status(), Status::OK);
    let answer = answer.scalar()?;
    let answer = answer.as_struct().expect("an object");
    assert_eq!(answer["interval"].as_str(), Some("1m"));
    let candles = answer["candles"].sequence_rows().expect("a list");
    assert_eq!(candles.len(), 2);
    let first = candles[0].as_struct().expect("an object");
    assert_eq!(first["start"].as_str(), Some("2026-01-05T10:00:00.000000000Z"));
    let bid = first["bid"].as_struct().expect("a reading");
    assert_eq!((bid["open"].as_str(), bid["close"].as_str()), (Some("100"), Some("100")));
    assert_eq!(first["ask"].as_struct().and_then(|ask| ask["open"].as_str()), Some("101"));
    assert_eq!(first["books"], Scalar::from(1_u64));

    // The same reading without HTTP answers the same candles.
    let query = BookQuery {
        table: "books".into(),
        ticker: "ACME".into(),
        from: T0,
        to: T0 + 120 * SECOND,
        timezone: Timezone::UTC,
        interval: None,
        side: None,
    };
    let held = service.candles(&query)?;
    assert_eq!(held.len(), 2);
    assert_eq!(held[1].bid.map(|bid| bid.open), Some("100.5".parse()?));
    assert_eq!(held[1].ask.map(|ask| ask.close), Some(Decimal::from_int(101)));
    let book = service.book("books", "ACME", T0 + 90 * SECOND)?.expect("a book at or before 10:01:30");
    assert_eq!(book.get_currunix(), T0 + 65 * SECOND);
    ```

### The display files

What the package ships beside its binding, and the argument vector `serve()` spawns the command with.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const path = require('node:path')

    // The package's `book` entry point, beside its binding.
    const book = require('yggdryl/book')

    assert.equal(path.basename(book.assets), 'book')
    assert.deepEqual([...book.assetFiles], [
      'index.html', 'theme.css', 'theme.js', 'api.js', 'chart.js', 'audit.js', 'app.js', 'favicon.svg',
    ])
    for (const name of book.assetFiles) assert.ok(fs.statSync(path.join(book.assets, name)).size > 0, name)

    // The tables first, then the socket, the path, one --capture per log.
    assert.deepEqual(
      book.serveArguments({
        tables: [{ name: 'books', location: '/data/books' }, 'reference=/data/reference'],
        bind: '0.0.0.0:8080',
        path: '/book',
        capture: 'bridge.log',
      }),
      ['market', 'serve', 'books=/data/books', 'reference=/data/reference', '--bind', '0.0.0.0:8080', '--path', '/book', '--capture', 'bridge.log'],
    )
    assert.deepEqual(book.serveArguments(), ['market', 'serve', '--bind', '127.0.0.1:0', '--path', '/'])
    assert.throws(() => book.serveArguments({ tables: [42] }), TypeError)
    // `serve(options)` spawns the command with that vector and resolves on its endpoint line.
    assert.equal(typeof book.serve, 'function')
    ```

## The ULBridge capture, served

One command, run from the root of a yggdryl checkout, serves the FIX bridge capture the crate's tests read, `rust/tests/fix/ulbridge.log`, read with the dictionary the checkout commits, `config/fix`: an absent folder becomes an Iceberg table, the capture's eight books land in it, and the display answers on a free port. The bridge's clock writes Zurich time, so the lines are read under `--timezone Europe/Zurich`; the routes render in whatever `tz` a question states.

```bash
yggdryl market serve books=/tmp/books --registry config/fix --capture rust/tests/fix/ulbridge.log --timezone Europe/Zurich --bind 127.0.0.1:0
```

```text
http://127.0.0.1:34385/
· table books over file:///tmp/books, created
· capture rust/tests/fix/ulbridge.log: 8 books into books
```

Both paths are the checkout's: neither the wheel nor the npm package ships a FIX dictionary or this capture. The command a registry install puts on the path - `pip install yggdryl`'s, which `book.serve` spawns - passes `--registry` the folder it keeps a dictionary in, a copy of a checkout's `config/fix`, and `--capture` its own bridge log; outside a checkout and without `--registry`, it refuses before it binds, `✗ expected a FIX dictionary at "file:///<working directory>/config/fix", got nothing`. Below, the port is the one this run took, and every answer is one line, wrapped here.

The table it serves, and the tickers the table holds, each with the range that holds its books:

```bash
curl http://127.0.0.1:34385/api/tables
curl 'http://127.0.0.1:34385/api/tickers?table=books'
```

```json
[{"name":"books","url":"file:///tmp/books"}]
```

```json
[{"books":1,"crosscode":"1605","from":"2026-08-14T01:03:17.000000000Z","ticker":"1605","to":"2026-08-14T01:03:18.000000000Z"},
 {"books":1,"crosscode":"2454","from":"2026-08-14T21:59:46.000000000Z","ticker":"2454","to":"2026-08-14T21:59:47.000000000Z"},
 {"books":2,"crosscode":"ABBN.S","from":"2026-08-14T12:46:39.000000000Z","ticker":"ABBN.S","to":"2026-08-14T12:46:40.000000000Z"},
 {"books":1,"crosscode":"EXAMPLECO.S","from":"2026-08-14T12:46:58.000000000Z","ticker":"EXAMPLECO.S","to":"2026-08-14T12:46:59.000000000Z"},
 {"books":2,"crosscode":"HOLN","from":"2026-08-14T12:46:39.000000000Z","ticker":"HOLN","to":"2026-08-14T12:46:41.000000000Z"},
 {"books":1,"crosscode":"XAU/USD","from":"2026-08-14T14:52:55.000000000Z","ticker":"XAU/USD","to":"2026-08-14T14:52:56.000000000Z"}]
```

Holcim's day as hourly candles in Zurich - a naive `from` and `to` are Zurich wall clocks - is one candle, the `14:00` bucket, whose two books read a bid of `72.3` and no ask, and one execution that traded `235` - its last quantity, out of an order of `300` ([volume](candle.md#volume)):

```bash
curl 'http://127.0.0.1:34385/api/candles?table=books&ticker=HOLN&from=2026-08-14T00:00:00&to=2026-08-15T00:00:00&tz=Europe/Zurich&interval=1h'
```

```json
{"candles":[{"ask":null,"askqty":null,"bid":{"close":"72.3","high":"72.3","low":"72.3","open":"72.3"},"bidqty":"50","books":2,
             "end":"2026-08-14T15:00:00.000000000+02:00[Europe/Zurich]","executions":1,"mid":null,"spread":null,
             "start":"2026-08-14T14:00:00.000000000+02:00[Europe/Zurich]","volume":"235"}],
 "from":"2026-08-14T00:00:00.000000000+02:00[Europe/Zurich]","interval":"1h","table":"books","ticker":"HOLN","timezone":"Europe/Zurich",
 "to":"2026-08-15T00:00:00.000000000+02:00[Europe/Zurich]"}
```

The book standing at the end of the day: one entry alive, the bid level it makes, no ask:

```bash
curl 'http://127.0.0.1:34385/api/book?table=books&ticker=HOLN&at=2026-08-15T00:00:00&tz=Europe/Zurich'
```

```json
{"alive":1,"asklimits":[],"askqty":null,"bestask":null,"bestbid":"72.3",
 "bidlimits":[{"price":"72.3","quantity":"50","tradable":true,"uuids":["01a0004f-6b94-7000-bc8d-e2a462f4367e"]}],
 "bidqty":"50","crosscode":"HOLN","currunix":"2026-08-14T14:46:40.020000000+02:00[Europe/Zurich]","deltas":1,"executions":0,
 "imbalance":"1","iscrossed":false,"islocked":false,"midpoint":null,"spread":null,"ticker":"HOLN"}
```

The audit of the day, gzip-coded and named after the ticker and the range in UTC; the header line and one row per entry, delta and execution, cut to their first 120 characters here. Its `content-length` is this checkout's: a row's `srcuuids` are the UUIDs of the capture lines its message was read from, and a line's UUID derives from the URL of the log it was read from, so a checkout standing elsewhere compresses to a few bytes more or fewer:

```bash
curl -s -D - -o audit.csv.gz 'http://127.0.0.1:34385/api/audit.csv.gz?table=books&ticker=HOLN&from=2026-08-14T00:00:00&to=2026-08-15T00:00:00&tz=Europe/Zurich'
gunzip -c audit.csv.gz | head -3 | cut -c1-120
gunzip -c audit.csv.gz | wc -l
```

```text
HTTP/1.1 200 OK
cache-control: no-store
content-disposition: attachment; filename="audit-HOLN-20260813T220000Z-20260814T220000Z.csv.gz"
content-length: 1928
content-type: application/gzip
date: Wed, 30 Sep 2026 04:58:57 GMT
server: yggdryl/0.1.19

bookunix,role,marketdatakind,currunix,creaunix,recdunix,exprunix,prevunix,snapunix,curruuid,crossuuid,crosscode,currhash
2026-08-14T12:46:39.743000000Z,delta,ORDR,2026-08-14T12:46:39.743000000Z,2026-08-14T12:46:39.743000000Z,2026-08-14T12:46
2026-08-14T12:46:39.743000000Z,execution,EXEC,2026-08-14T12:46:39.743000000Z,2026-08-14T12:46:39.743000000Z,2026-08-14T1
5
```

A question the route cannot read is `400` naming the parameter - here a range whose `to` is not after its `from`:

```bash
curl -s -D - 'http://127.0.0.1:34385/api/candles?table=books&ticker=HOLN&from=2026-08-14T00:00:00&to=2026-08-14T00:00:00&tz=Europe/Zurich' | head -1
curl -s 'http://127.0.0.1:34385/api/candles?table=books&ticker=HOLN&from=2026-08-14T00:00:00&to=2026-08-14T00:00:00&tz=Europe/Zurich'
```

```text
HTTP/1.1 400 Bad Request
{"error":"invalid record value at $.to: expected an instant after `from` (2026-08-13T22:00:00.000000000Z), got 2026-08-13T22:00:00.000000000Z"}
```

The display itself answers at the endpoint:

```bash
curl -s -D - -o /dev/null http://127.0.0.1:34385/ | head -5
```

```text
HTTP/1.1 200 OK
cache-control: no-cache
content-length: 4881
content-type: text/html; charset=utf-8
date: Wed, 30 Sep 2026 04:58:57 GMT
```

`/tmp/books` is now an Iceberg table - `metadata/v1.metadata.json`, `v2.metadata.json`, `version-hint.text`, a manifest list, a manifest and one Parquet file of eight rows - that `yggdryl market serve books=/tmp/books` serves again from any folder, since with no capture it reads no dictionary, and that a Node program starts through `book.serve({ tables: 'books=/tmp/books' })`.

## Edges

- An Avro leaf spells no `uint64`: it serves the rows written under the row as Iceberg states it (`MarketData::field().into_scheme_compat(&Scheme::ICEBERG)`, the codes as `decimal(20, 0)`), which the service reads back under the `marketdata` row, while a capture appended to it under its own encoding is refused at `currhashcode` (`expected a datatype Avro can spell, got uint64`).
- An Iceberg table stores `marketdatakind` as a bare `int32` with no extension identity, so the service's category filter compares the member's code rather than its name and prunes there as it does over an IPC leaf carrying the `yggdryl.marketdatakind` extension.
- An `events` row's `uint64` codes - `currhashcode`, `crosshashcode` - are JSON integers up to 2^64, which JavaScript's `JSON.parse` rounds past 2^53; the CSV audit writes them as their digits.
- Routing the same prefix on the same server again replaces the service that answered there; a prefix carrying a query, a fragment or a control byte is refused by `route`.
- `tz` reads only zones this build has rules for: one the bundled registry lacks, such as `Europe/Sofia`, is `400` at `$.tz`, and `timezones` is the list to offer.
- The audit downloads are unbounded by design: the books of the range are held, sorted, while the CSV medium writes their rows a batch at a time, so the range a question states is what bounds it.
