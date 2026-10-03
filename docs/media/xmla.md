# XML for Analysis

An XML for Analysis 1.1 rowset document as a record medium - the `xsd:schema` naming its columns, then one `<row>` element per row - and, in Rust, the provider that serves folders of record media to Excel and Power Query as catalogs over HTTP.

## Overview

| | |
| --- | --- |
| Declared by | `application/xmla+xml`, `.xmla` |
| Build | default; the provider's HTTP route needs the `http` feature |
| Rust | `yggdryl::xmla`: `Xmla<H>` over any handle with `XmlaOptions`, the free `read_field`, `read_batch_reader` and `overwrite_arrow_reader`, `Rowset` and `XsdType` for the document, and the [provider](#provider): `Service`, `Catalog`, `Request`, `Response` |
| Python, JavaScript | any `IOBase` whose name declares the document, through the [calls every medium answers](index.md#read); the provider is Rust-only, and `yggdryl xmla serve` its terminal |
| Settings | the shared [`RecordOptions`](index.md#options); in Rust, `XmlaOptions` also says which envelope a write carries and which of the schema and the rows it holds |

A `.xmla` handle holds one document and reads and writes it through the same calls as every other medium. Every value is spelled as the XML Schema type its datatype maps to (`xsd:long`, `xsd:double`, `xsd:dateTime`, `uuid`, `xsd:base64Binary`), so a client reads the document without this crate.

## Read

A read takes the rowset inside a SOAP 1.1 `DiscoverResponse` or `ExecuteResponse`, or as the bare rowset `root` a client saved off the wire. Without a declaration, the document's own schema is what a read answers, and a `dateTime` column whose values spell a zone lands as `datetime64(us, UTC)`; a declared field types a document written without its schema, which a read without one refuses by name. An absent element is a null cell. The document is parsed whole - XML has no frame to read a prefix of - and answered as one batch, whatever `batch_row_size` says.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOMedia, MimeType, Scalar, StructType};

    // A rowset with no schema - the bare `root` a client saved - has nothing
    // that says what its cells are, so a read declares the field.
    let bare = Buffer::from_bytes(
        b"<root><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row></root>".to_vec(),
    )
    .with_media_type(MimeType::XMLA.into());
    let options = bare.record_options()?;
    let refused = bare.read_arrow_field(&options).unwrap_err();
    assert!(refused.to_string().contains("carries no schema"), "{refused}");

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let mut symbols = Vec::new();
    for records in bare.read_serie(Some(&options.with_field(field)))? {
        let records = records?;
        let symbol = records.child("symbol").expect("a symbol column");
        for row in 0..symbol.len() {
            symbols.push(symbol.scalar(row)?);
        }
    }
    // An absent element is a null cell.
    assert_eq!(symbols, [Scalar::from("AAPL"), Scalar::Null]);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # A document carrying its schema answers it, with no declaration.
    written = IOBase(root / "trades.xmla")
    written.overwrite_records([{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}])
    assert [(child.name, str(child.dtype)) for child in written.read_arrow_field().dtype] == [
        ("id", "int64"),
        ("symbol", "utf8"),
    ]

    # A rowset with no schema - the bare `root` a client saved - has nothing
    # that says what its cells are, so a read declares the field.
    bare = IOBase(root / "saved.xmla")
    bare.write_bytes(b"<root><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row></root>")
    with pytest.raises(ValueError, match="carries no schema"):
        bare.read_arrow_field()
    schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("symbol", pa.string())])
    # An absent element is a null cell.
    assert list(bare.read_records(field=schema)) == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": None},
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // A rowset with no schema - the bare `root` a client saved - has nothing
    // that says what its cells are, so a read declares the field.
    const bare = new IOBase(path.join(root, 'saved.xmla'))
    bare.writeBytes(Buffer.from('<root><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row></root>'))
    assert.throws(() => bare.readArrowField(), /carries no schema/)
    const field = fields.struct('row', [new Field('id', 'int64', false), Field.from('symbol: utf8')], {
      nullable: false,
    })
    // An absent element is a null cell.
    assert.deepEqual([...bare.readRecords({ field })], [
      { id: 1n, symbol: 'AAPL' },
      { id: 2n, symbol: null },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Write

A write streams each batch into the document as it arrives and hands the handle the whole once: the `xsd:schema`, then one `<row>` per row, inside the SOAP 1.1 `ExecuteResponse` a provider answers a statement with. The schema travels with the rows: a nullable column is `minOccurs="0"` and absent where its cell is null, a sequence column is `maxOccurs="unbounded"` and one element per item, a struct is the element's children, and a column whose name is not an XML name is written under its `_xHHHH_` escape with `sql:field` keeping the original. In Rust, `XmlaOptions` writes a `DiscoverResponse` instead (`with_method`), the bare rowset `root` (`without_envelope`), or the schema or the rows alone (`with_content`).

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let mut handle = Buffer::new().with_media_type(MimeType::XMLA.into());
    let options = handle.record_options()?.with_field(field.clone());
    handle.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        ],
        &options,
    )?;

    // The document carries its schema and one `<row>` per row; a null cell
    // is an absent element.
    let document = String::from_utf8(handle.read_all_bytes()?)?;
    assert!(document.contains("<xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\" minOccurs=\"0\"/>"));
    assert!(document.contains("<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>"));

    // It reads back through the calls every medium answers.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&handle.record_options()?)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    assert_eq!(handle.read_arrow_field(&handle.record_options()?)?.fields().len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.xmla")

    # The `.xmla` suffix picks the rowset document; a None makes the column nullable.
    handle.overwrite_records([{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}])

    document = handle.read_bytes().decode()
    assert "<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>" in document
    assert list(handle.read_records()) == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": None},
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.xmla'))

    // The `.xmla` suffix picks the rowset document; a null makes the column nullable.
    handle.overwriteRecords([{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }])

    const document = handle.readBytes().toString()
    assert.ok(document.includes('<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>'))
    assert.deepEqual([...handle.readRecords()], [
      { id: 1n, symbol: 'AAPL' },
      { id: 2n, symbol: null },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Provider

`yggdryl::xmla` is also the provider side of the protocol, Rust-only: a [`Service`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.Service.html) serves the catalogs of a [warehouse](../warehouse/index.md) - any `Catalog`; a `FolderCatalog` reads a folder of record media as one, each file the folder holds a table, each folder inside it a namespace (a schema, to XMLA) of tables, a folder laid out as an Iceberg table a table wherever it sits (refused by name in a build without the `iceberg` feature), and a ZIP archive a catalog of its members - answering `Discover` with the XMLA schema rowsets (`DISCOVER_DATASOURCES`, `DISCOVER_SCHEMA_ROWSETS`, `DBSCHEMA_CATALOGS`, `DBSCHEMA_SCHEMATA` - every namespace under a catalog - `DBSCHEMA_TABLES` - every table, `TABLE_SCHEMA` the namespace parts between the catalog and the table as a path and null at the root, `DESCRIPTION` the table's description else what holds its rows - `DBSCHEMA_COLUMNS` from each table's `field()` and the rest, restrictions applied, and one multidimensional rowset, `MDSCHEMA_CUBES`, each catalog its one cube - what MSOLAP asks for between the catalog list and the tables, and the only `MDSCHEMA_*` rowset a tabular provider answers) and `Execute` by running the statement through the [expression grammar](../expression/index.md) against the table it names - `table` under the `Catalog` property, `catalog.table`, `schema.table`, `catalog.schema.table`, as many namespace parts as the catalog's `namespace_levels` allow - resolved to its whole path and run with [`Plan::execute_in`](../expression/plans.md#sources) against the service's warehouse, so no target is rewritten to a URL; a URL target outside every served catalog's location is refused, and a write unless the service was made writable. `with_catalog` serves one more catalog in place of one of the same name, `with_warehouse` a whole warehouse, and `warehouse()`, `catalogs()` and `catalog(name)` read them back. Every refusal is a SOAP fault carrying the XMLA `<Error>` with a code, a description and the source, answered at HTTP `200` the way the reference providers answer and XMLA clients read one; a header block that demands to be understood and is not a session block earns a `MustUnderstand` fault, and a failure once a streamed answer has begun is reported inside the rowset as `<Messages><Error/></Messages>`. [`Service::route`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.Service.html#method.route) puts the service on the crate's [`http::Server`](../holder/index.md#serving-a-handle) at a path: a `GET` answers a short text description of the endpoint and a `HEAD` its head; a `POST` is always answered `200` under `text/xml; charset=utf-8` and `X-Transport-Caps-Negotiation-Flags: 0,0,0,0,0` - a body whose declared content type is not XML earns an immediate `Client` fault under those same headers, and any other body, empty included, is handed to `Service::handle`, its answer written as it is sent so a large `Execute` streams; any other method is the server's own `405` naming `GET, HEAD, POST`. `yggdryl xmla serve` does the same from a terminal, printing the endpoint first.

The binding speaks what the reference clients - MSOLAP, which Excel's Data Connection Wizard and PivotTables use, and ADOMD.NET, which Power Query uses - send: a `Content-Length` or a chunked request body, `Expect: 100-continue`, and MS-SSAS content negotiation. The connection side of that - framing, keep-alive, the interim `100 Continue` answered once the head has passed every check a body is refused on (so a .NET client does not wait its 350 ms), timeouts and bounds, HTTP/2 and HTTP/3, the exchange trace - is the crate's [HTTP server](../holder/index.md#serving-a-handle); what stays XMLA's is the negotiation header itself, stamped on every SOAP answer and never on the `GET` description, plain text XML both ways. A session opens the way those clients open one - an `Execute` carrying `BeginSession` and an empty `<Statement/>`, answered empty under a `Session` block that every later answer carries back, a fault included. `DISCOVER_SCHEMA_ROWSETS` states each rowset's `SchemaGuid` and `RestrictionsMask`, a restriction sent with no value restricts nothing, and `DISCOVER_PROPERTIES` answers the names those clients read before they drive a provider, each with what is true of this one: `ProviderType` 1 (a tabular data provider), `MDXSupport`, `ServerName`, `SQLSupport` 512, `DBMSVersion` `10.50.1600.1` - the SQL Server 2008 R2 RTM build, the release whose XML for Analysis is spoken here and the oldest ADOMD.NET agrees to talk to, `ProviderVersion` staying the crate's own - the `Mdprop*` MDX capability masks - all zero, the statement language being the expression grammar - and the `Dbprop*`/`Ssprop*` properties a client states about itself, echoed back as it set them, a number's own default where it set none, and no cell at all where there is no default - never an empty cell under an `int`. [`ServerOptions::with_trace`](https://docs.rs/yggdryl/latest/yggdryl/http/struct.ServerOptions.html#method.with_trace) - `yggdryl xmla serve --trace <folder>` - writes every exchange as it went over the wire, `NNNN-request.http` as read and `NNNN-response.http` as sent, the interim status and the chunked framing included, numbered from `0000` across every connection: what a client asked is read from the folder, and a request file replays through `Service::handle` with its body.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::xmla::{Discover, Execute, Request, RequestType, Response, Service, ServiceOptions};
    use yggdryl::{DataType, FolderCatalog, IOBase, IOMedia, Scalar, Serie, StructType};

    let root = std::env::temp_dir().join(format!("yggdryl-xmla-docs-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let field = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
    ])?)
    .required_field("row");
    let mut trades = Holder::folder(&root)?.child_by_path("trades.arrows")?;
    let options = trades.record_options()?.with_field(field);
    trades.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(187.5_f64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(410.25_f64)]),
        ],
        &options,
    )?;

    // A folder is a catalog; the service answers a request's bytes with a
    // response's bytes, which is what the server puts on the socket.
    let service = Service::new(ServiceOptions::new())
        .with_catalog(FolderCatalog::bound("market", Holder::folder(&root)?));
    let discover = Request::from(Discover::new(RequestType::DbschemaTables));
    let answer = service.handle(&discover.into_bytes()?, Vec::new())?;
    let tables = Response::from_bytes(&answer, None)?;
    let names = tables.rows().and_then(|rows| rows.child("TABLE_NAME").cloned());
    assert_eq!(names.map(|column| column.scalar(0)).transpose()?, Some(Scalar::from("trades")));

    // A statement runs through the expression grammar against the table it names.
    let execute = Request::from(Execute::statement(
        "select symbol from market.trades where price > 200",
    ));
    let answer = service.handle(&execute.into_bytes()?, Vec::new())?;
    let selected = Response::from_bytes(&answer, None)?;
    assert_eq!(selected.rows().map(Serie::len), Some(1));
    std::fs::remove_dir_all(&root)?;
    ```

The server is bound the same way in Rust - `let server = http::Server::bind_with("127.0.0.1:8080", ServerOptions::default())?; Arc::new(service).route(&server, "/xmla")?` - and from the command line, where `name=location` builds the catalog `Catalog::from_url` picks for the location - a folder path or a URL a holder resolves is a folder catalog - tracing each exchange when asked:

```bash
yggdryl xmla serve market=/data/market reference=s3://bucket/reference --bind 0.0.0.0:8080 --path /xmla
yggdryl xmla serve market=C:\data\market --trace C:\data\trace
```

The `yggdryl` a wheel ships is built with the CLI crate's `iceberg` feature, which `scripts/stage_cli.py` passes, so it reads an Iceberg folder; `cargo build -p yggdryl-cli` alone builds the schema-only core, which lists such a folder as a table and refuses its rows by name.

## Excel as a client

Excel reaches the provider through MSOLAP and through Power Query's ADOMD.NET, and three doors open on a folder catalog `yggdryl xmla serve` puts on a socket. Each was driven from Excel (Microsoft 365, 16.0.20430) against `market=C:\data\market` - Iceberg tables of ten thousand, a hundred thousand and a million trades, a table of every datatype, a `reference` schema - with the exchanges kept under `rust/tests/xmla/fixtures/excel/<door>/` as they went over the wire and replayed through `Service::handle` by `rust/tests/xmla/service.rs`, each answer checked against the captured one by what a client reads: the kind of answer, the columns, the number of rows, the fault code.

- **Power Query, with a query** (`pq-query`, `refresh`). *Données > Nouvelle requête > À partir d'une base de données > SQL Server Analysis Services*: server `http://127.0.0.1:8080/xmla`, database `market`, and the statement in the *Requête MDX ou DAX* box - `select * from market.trades limit 100`. The text crosses unchanged as the `Execute` statement under `Format=Tabular`, the preview types the columns and *Charger* lands the rows; a refresh runs the query again over the pooled connection. The same in M:

    ```text
    AnalysisServices.Database("http://127.0.0.1:8080/xmla", "market",
        [Query = "select * from market.trades limit 100"])
    ```

- **An `.odc` with a command text** (`odc-query`). `Provider=MSOLAP;Data Source=http://127.0.0.1:8080/xmla;Initial Catalog=market;` with `<odc:CommandType>Query</odc:CommandType>` and the statement as `<odc:CommandText>`: opened, Excel lands the rows as an ordinary table in one session of five requests - the leanest path there is, and no Power Query.
- **The Data Connection Wizard** (`wizard`). *Données externes > À partir d'autres sources > À partir d'Analysis Services*: MSOLAP asks `DISCOVER_PROPERTIES`, opens a session, then `DISCOVER_SCHEMA_ROWSETS`, `DBSCHEMA_CATALOGS`, `MDSCHEMA_CUBES` and `DBSCHEMA_TABLES`, lists the catalog as its one cube and saves the `.odc`.

What stays closed, and why. The PivotTable the wizard offers next asks the `MDSCHEMA_*` set and then MDX (`pivottable`); Power Query's navigator - the connector with no query - runs a DMV query, `select [CUBE_NAME], [BASE_CUBE_NAME], [CUBE_CAPTION] from $system.mdschema_cubes where [CUBE_SOURCE] = 1`, and then browses as an MDX or a DAX client (`pq-navigator`); a DAX text, `EVALUATE 'trades'`, is refused by the grammar at byte 0 and Power Query shows the refusal (`pq-dax`). MDX and DAX are not spoken here: the statement language is the [expression grammar](../expression/index.md), and a query is the door.

Two facts the clients read before anything else are stated once, in [`ServiceOptions`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.ServiceOptions.html): `DBMSVersion` is `10.50.1600.1` - SQL Server 2008 R2 RTM, the release whose XML for Analysis is spoken here and the oldest ADOMD.NET agrees to talk to - and `ProviderVersion` is the crate's. ADOMD.NET sends every request with `Expect: 100-continue` and chunked, the body opening with a byte-order mark, and a `<Cancel/>` before it reuses a pooled connection, answered empty: no command is ever left running here.

## Behind a reverse proxy

The server speaks cleartext HTTP/1.1 on the socket it binds, and has no TLS on that port. MSOLAP and ADOMD.NET reach an `https` endpoint, and a shared host reaches the provider under a path of its own, through a reverse proxy that terminates TLS and forwards each request over plain HTTP to the socket. What the proxy did to the request is stated to the provider by five flags, each a method of the same name on [`ServerOptions`](../holder/index.md#behind-a-reverse-proxy), which is where the rule for every URL the server states is spelled out:

| flag | `ServerOptions` | states |
| --- | --- | --- |
| `--public-url https://data.example.com/olap` | `with_public_url` | the scheme, host, port and prefix clients use, whatever a request says: the base of every URL the server states and the `URL` `DISCOVER_DATASOURCES` answers |
| `--trusted-proxy 10.0.0.0/8`, repeatable | `with_trusted_proxies` | the peers - IP addresses or CIDR networks - whose forwarded fields are believed; none by default, because any client can write them |
| `--forwarded-header X-Forwarded-Prefix`, repeatable | `with_forwarded_headers` | the forwarded fields read from a trusted peer: `X-Forwarded-For` and `X-Forwarded-Proto` by default, which every proxy below sets on every request. Given, the list replaces the default; name only a field the proxy sets or overwrites, because one it passes through is whatever the client wrote. `Forwarded`, `X-Forwarded-Host`, `-Port` and `-Prefix` are read only when named |
| `--path-prefix /olap` | `with_path_prefix` | the path the proxy leaves in front of `--path`, stripped before routing and carried back on the URLs the server states; a proxy that strips it itself needs none |
| `--read-timeout 75` | `with_read_timeout` | seconds, 1 to 86400, a connection may stay quiet before the server closes it, 30 by default; keep it above the proxy's upstream keep-alive timeout |

```bash
yggdryl xmla serve bronze=/lake/bronze silver=/lake/silver gold=/lake/gold --bind 127.0.0.1:8080 --path /xmla \
  --public-url https://data.example.com/olap --trusted-proxy 10.0.0.0/8 --path-prefix /olap
```

nginx leaves `Forwarded`, `X-Forwarded-Host`, `-Port` and `-Prefix` as the client sent them, which is why none is read unless named: behind this configuration the server needs no `--forwarded-header` at all.

`--public-url` alone is enough when the public endpoint is known: the description a `GET` answers, the `URL` of `DISCOVER_DATASOURCES` and the note printed after the endpoint all state `https://data.example.com/olap/xmla`, and the first line printed stays the socket's own `http://127.0.0.1:8080/xmla`, which is what a script that started the process connects to. Without it, the forwarded fields of a trusted proxy give the description its URL and `DISCOVER_DATASOURCES` states none - the captured Excel exchanges show both clients working without one - so trust the proxy's own address or network and nothing wider, never `0.0.0.0/0`. What holds whatever the proxy is:

- **TLS ends at the proxy.** `https` is the proxy's; it forwards plain HTTP and says so in `X-Forwarded-Proto` (or `Forwarded: proto=https`, once named), which is where the server learns the scheme it is reached under. No certificate is configured on the server.
- **The public host is the proxy's `Host`.** Every proxy below either passes the client's `Host` on or sets it, and that is the host the server states; a host in `X-Forwarded-Host` is read only when `--forwarded-header X-Forwarded-Host` says the proxy sets it.
- **A `POST` is never redirected.** A `GET` or `HEAD` of `/olap/xmla/` - the trailing slash a `location /olap/` block or a copied URL adds - is a `308` to the relative `../xmla`, the query kept; a `POST` there is served by the route. MSOLAP and ADOMD.NET only ever `POST`, and whether either re-sends a body after a `308` is unverified, so a stray slash costs them nothing.
- **Keep the proxy's upstream keep-alive under the server's read timeout.** A proxy reuses an idle connection to the server; the server closes one that has been quiet for `--read-timeout`. When the proxy's idle timeout is the longer of the two it reuses a connection the server has closed and the client sees a `502`, so the proxy's timeout goes below `--read-timeout` (25 s under the default 30), or `--read-timeout` above a timeout the proxy does not let you set (a load balancer's 60 s is `--read-timeout 75`).
- **Speak HTTP/1.1 upstream.** An HTTP/1.0 request - nginx's default upstream version - is answered with a close-delimited body, at one connection per request, and such a body has no way to say it ended short: an `Execute` whose rows fail part way reads as a whole, shorter answer. `proxy_http_version 1.1` with an empty `Connection` header keeps the connections pooled and the answers chunked, where a missing last chunk says so.
- **Stream, and size the body.** A large `Execute` is written as it is sent, so turn response buffering off at the proxy or the first row waits for the last; the largest request body the proxy accepts is `--max-body` (16 MiB by default).

nginx, forwarding the path whole so the server strips the prefix:

```nginx
upstream yggdryl_xmla {
    server 127.0.0.1:8080;
    keepalive 16;
    keepalive_timeout 25s;   # under the server's --read-timeout, 30 s by default
}

server {
    listen 443 ssl;
    server_name data.example.com;
    # ssl_certificate and ssl_certificate_key as usual

    location /olap/ {
        proxy_pass http://yggdryl_xmla;   # no URI part: /olap/xmla crosses whole, hence --path-prefix /olap
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_set_header Host $host;                                   # the public host the server states
        proxy_set_header X-Forwarded-Proto $scheme;                    # overwritten, read by default
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;   # appended, walked from the right
        proxy_request_buffering off;
        proxy_buffering off;
        proxy_read_timeout 300s;
        client_max_body_size 16m;         # = --max-body
    }
}
```

Caddy, which terminates TLS on its own, passes the client's `Host` on, sets `X-Forwarded-For`, `-Proto` and `-Host` itself and strips the prefix, so the server is told the prefix rather than asked to strip it. `header_up` overwrites whatever the client sent in that field, so it is safe to name - `--trusted-proxy 127.0.0.1 --forwarded-header X-Forwarded-For --forwarded-header X-Forwarded-Proto --forwarded-header X-Forwarded-Prefix`:

```caddyfile
data.example.com {
    handle_path /olap/* {
        reverse_proxy 127.0.0.1:8080 {
            header_up X-Forwarded-Prefix /olap
            flush_interval -1
            transport http {
                keepalive 25s
                keepalive_idle_conns 16
            }
        }
    }
}
```

IIS with Application Request Routing and URL Rewrite, the proxy enabled in the ARR module and the forwarded variables allowed in `applicationHost.config` or the site's `web.config`; ARR adds `X-Forwarded-For` on its own, forwards HTTP/1.1 with keep-alive, sends the rewritten `Host`, and buffers answers unless told not to. The rule below sets `X-Forwarded-Proto` and `X-Forwarded-Host` on every request, overwriting the client's, so the host is named too - `--forwarded-header X-Forwarded-For --forwarded-header X-Forwarded-Proto --forwarded-header X-Forwarded-Host`:

```xml
<configuration>
  <system.webServer>
    <rewrite>
      <rules>
        <rule name="yggdryl xmla" stopProcessing="true">
          <match url="^olap/(.*)" />
          <action type="Rewrite" url="http://127.0.0.1:8080/olap/{R:1}" />
          <serverVariables>
            <set name="HTTP_X_FORWARDED_PROTO" value="https" />
            <set name="HTTP_X_FORWARDED_HOST" value="{HTTP_HOST}" />
          </serverVariables>
        </rule>
      </rules>
      <allowedServerVariables>
        <add name="HTTP_X_FORWARDED_PROTO" />
        <add name="HTTP_X_FORWARDED_HOST" />
      </allowedServerVariables>
    </rewrite>
  </system.webServer>
</configuration>
```

```powershell
& "$env:windir\system32\inetsrv\appcmd.exe" set config -section:system.webServer/proxy -responseBufferLimit:0
```

A cloud load balancer - an AWS Application Load Balancer, a Google external HTTP(S) load balancer, an Azure Application Gateway - terminates TLS, passes the client's `Host` on, and sets `X-Forwarded-For` and `X-Forwarded-Proto` on every request it forwards, which are the two read by default; it passes an `X-Forwarded-Host` through as the client wrote it, so name none. Trust the address range the balancer sends from (the subnets it lives in, or the ranges the cloud publishes for it) with `--trusted-proxy`, and raise `--read-timeout` above its idle timeout (60 s on an ALB by default, so `--read-timeout 75`), or state `--public-url` and trust nothing. A balancer that forwards the path whole takes `--path-prefix`; one that rewrites it to `/xmla` takes none.

## Serve Iceberg catalogs as cubes

A [PyIceberg](https://py.iceberg.apache.org/) SQL catalog keeps its table pointers in SQLite and its tables under a warehouse folder, laid out as `<warehouse>/<namespace>/<table>/metadata/*.metadata.json` - which is exactly the catalog, schema and table a folder catalog reads, so a lake of one such catalog per layer is served to Excel with nothing between PyIceberg and the provider but the folders. Three layers - bronze, silver and gold, each a catalog of its own over a warehouse of its own - are three cubes: `MDSCHEMA_CUBES` answers one row per catalog, each namespace is a schema, each table folder a table.

Land the tables through PyIceberg, one SQL catalog per layer with its warehouse a folder beside its database - `sqlite:///lake/silver.db` over `lake/silver`. The block is tagged `ignore` because it needs `pyiceberg`, which the example runner's environment does not carry; it runs as written under an environment that has it:

```{ .python .ignore }
import pathlib
import tempfile

import pyarrow as pa
from pyiceberg.catalog.sql import SqlCatalog

lake = pathlib.Path(tempfile.mkdtemp())


def catalog(layer: str) -> SqlCatalog:
    # The pointers in `<layer>.db`, the tables under `<layer>/<namespace>/<table>/`.
    (lake / layer).mkdir()
    return SqlCatalog(
        layer,
        uri=f"sqlite:///{(lake / f'{layer}.db').as_posix()}",
        warehouse=(lake / layer).as_posix(),
    )


bronze, silver, gold = (catalog(layer) for layer in ("bronze", "silver", "gold"))
for layer in (bronze, silver, gold):
    layer.create_namespace("record_keeping")

log_messages = pa.table(
    {
        "seqnum": pa.array([1, 2, 3], pa.int64()),
        "body": ["8=FIX.4.4|35=D|55=AAPL", "8=FIX.4.4|35=8|55=AAPL", "8=FIX.4.4|35=D|55=MSFT"],
    }
)
bronze.create_table("record_keeping.log_messages", schema=log_messages.schema).append(log_messages)
orders = pa.table(
    {
        "symbol": ["AAPL", "MSFT", "GOOG", "AAPL"],
        "price": pa.array([187.5, 410.25, 141.0, 188.0], pa.float64()),
        "quantity": pa.array([100, 250, 40, 60], pa.int64()),
    }
)
silver.create_table("record_keeping.orders", schema=orders.schema).append(orders)

# What the provider reads: the table folder under the namespace folder, and
# its metadata documents, the highest-numbered one being the current table.
metadata = lake / "silver" / "record_keeping" / "orders" / "metadata"
assert len(list(metadata.glob("0000*.metadata.json"))) == 2
assert gold.list_tables("record_keeping") == []
```

Serve the three warehouses, one catalog each and read-only - never `--writable` over a table PyIceberg manages, because a commit made here would write a new metadata document the SQLite pointer does not name, and the two would fork:

```bash
yggdryl xmla serve bronze=/lake/bronze silver=/lake/silver gold=/lake/gold --bind 0.0.0.0:8080
```

```text
http://127.0.0.1:8080/xmla
· catalog bronze over file:///lake/bronze
· catalog silver over file:///lake/silver
· catalog gold over file:///lake/gold
```

Each layer's warehouse is its own catalog. Serving `/lake` as one catalog would make the layers schemas and read `record_keeping` as a folder table, its Iceberg folders the files of that table. A layer with no table yet - gold above, an empty folder, or a folder that does not exist - is still a cube of no tables, so a dashboard can name it before the first table lands.

What a client then sees is what the [provider](#provider) answers over the same folders, here in one process with no socket between, over tables the crate's own `IcebergTable::create` lays out the way PyIceberg does:

```rust
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, IcebergTable, assign_field_ids};
use yggdryl::local::LocalFolder;
use yggdryl::xmla::{
    Discover, Execute, PropertyList, Request, RequestType, Response, Service, ServiceOptions,
};
use yggdryl::{DataType, FolderCatalog, Scalar, Serie, StructType, arrow};

let lake = std::env::temp_dir().join(format!("yggdryl-xmla-lake-{}", std::process::id()));
let _ = std::fs::remove_dir_all(&lake);

// `<layer>/record_keeping/<table>`: a PyIceberg SQL catalog's warehouse layout.
let mut orders = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("symbol"),
    DataType::Float64.required_field("price"),
    DataType::Int64.required_field("quantity"),
])?)
.required_field("row");
assign_field_ids(&mut orders, 1)?;
let folder = LocalFolder::new(lake.join("silver/record_keeping/orders"))?;
let spec = PartitionSpec::identity(0, &orders, &[])?;
let mut table = IcebergTable::create(folder, FormatVersion::V2, orders.clone(), spec)?;
let batch = RecordBatch::try_new(
    orders.into_arrow_schema()?,
    vec![
        Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
        Arc::new(Float64Array::from(vec![187.5, 410.25, 141.0])),
        Arc::new(Int64Array::from(vec![100_i64, 250, 40])),
    ],
)?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
let mut log_messages = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("seqnum"),
    DataType::utf8().required_field("body"),
])?)
.required_field("row");
assign_field_ids(&mut log_messages, 1)?;
let folder = LocalFolder::new(lake.join("bronze/record_keeping/log_messages"))?;
let spec = PartitionSpec::identity(0, &log_messages, &[])?;
IcebergTable::create(folder, FormatVersion::V2, log_messages, spec)?;
std::fs::create_dir_all(lake.join("gold"))?;

// One catalog per layer, as `yggdryl xmla serve bronze=... silver=... gold=...`.
let mut service = Service::new(ServiceOptions::new());
for layer in ["bronze", "silver", "gold"] {
    service = service.with_catalog(FolderCatalog::bound(layer, Holder::folder(lake.join(layer))?));
}

// One cube per catalog, in the order they were named.
let cubes = Request::from(Discover::new(RequestType::MdschemaCubes));
let answer = service.handle(&cubes.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
let cubes = response.rows().expect("a rowset");
let names = cubes.child("CUBE_NAME").expect("a column");
let names: Vec<Scalar> = (0..cubes.len()).map(|row| names.scalar(row)).collect::<Result<_, _>>()?;
assert_eq!(names, [Scalar::from("bronze"), Scalar::from("silver"), Scalar::from("gold")]);

// Under a catalog - Excel's database - the tables are the namespace's, and a
// namespace is a schema.
let tables = Request::from(
    Discover::new(RequestType::DbschemaTables)
        .with_properties(PropertyList::new().with("Catalog", "silver")),
);
let answer = service.handle(&tables.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
let tables = response.rows().expect("a rowset");
assert_eq!(tables.len(), 1);
let schema = tables.child("TABLE_SCHEMA").expect("a column").scalar(0)?;
let name = tables.child("TABLE_NAME").expect("a column").scalar(0)?;
assert_eq!((schema, name), (Scalar::from("record_keeping"), Scalar::from("orders")));

// A statement names its table `catalog.schema.table`.
let execute = Request::from(Execute::statement(
    "select symbol, price from silver.record_keeping.orders where price > 150",
));
let answer = service.handle(&execute.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
assert_eq!(response.rows().map(Serie::len), Some(2));
std::fs::remove_dir_all(&lake)?;
```

Excel then opens the [doors above](#excel-as-a-client) on it: Power Query with the server `http://<host>:8080/xmla` - `https://data.example.com/olap/xmla` [behind a proxy](#behind-a-reverse-proxy) - the database `silver` and the statement `select * from silver.record_keeping.orders limit 100`; an `.odc` with `Initial Catalog=silver` and that statement as its command text; the Data Connection Wizard listing `bronze`, `silver` and `gold` as the three cubes. A statement names its table as `catalog.schema.table`, or as `schema.table` under the `Catalog` the connection set - Excel's database - and `record_keeping.orders` with no catalog set is refused by name when several catalogs are served, since each has a `record_keeping`.

What the provider reads, and does not. It opens a table at its highest-numbered `*.metadata.json` and never reads the SQLite pointer, so it agrees with PyIceberg exactly as long as the folder holds one line of history and the pointer names its last document. A table PyIceberg drops stays in the cube until its folder is removed: `drop_table` deletes the row and leaves every file, so the table is still listed and read; `purge_table` deletes the files but leaves the table folder holding an empty `data/` and an empty `metadata/`, which `DBSCHEMA_TABLES` still lists as a table and a `select` of it answers with a fault (`expected a record encoding this build implements ..., got inode/directory`) - so after either call, remove `<warehouse>/<namespace>/<table>/` as well; a table re-created at the same location starts its numbering below the old documents, so the old table is what is served until those are removed; `write.metadata.path` or `write.data.path` pointing outside the table folder is not detected; a nested namespace `a.b` is a folder named `a.b`, a schema name with a dot, and `catalog.a.b.table` is four parts, refused. And writing goes one way: PyIceberg commits land in the folder and are served at the next request, while `--writable` over a PyIceberg-managed table would fork the pointer, so the provider stays read-only over such a lake.

## Performance

Criterion point estimates from a Windows 11 x86_64 release run on an AMD Ryzen 5 150 (6 cores, 23 GiB) with rustc 1.96.1 (2026-09-26). The documents are the ten-thousand-row table of the `media` bench; the provider rows are `Service::handle` over a folder catalog of three IPC tables - the bytes a client sent in, the bytes the server puts on the socket out, with no socket between.

| operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| write a rowset document | 10,000 | 7.56 ms | 1.32M rows/s |
| read a rowset document | 10,000 | 96.9 ms | 103k rows/s |
| write a `Discover` request | - | 1.59 us | - |
| read a `Discover` request | - | 18.5 us | - |
| `DISCOVER_PROPERTIES` | 53 | 233 us | - |
| `DBSCHEMA_CATALOGS` | 1 | 139 us | - |
| `DBSCHEMA_TABLES` | 3 | 1.20 ms | - |
| `DBSCHEMA_COLUMNS` | every column of the 3 | 5.84 ms | - |
| `select * from market.trades_10k` | 10,000 | 10.8 ms | 926k rows/s |
| `select * from market.trades_100k` | 100,000 | 81.4 ms | 1.23M rows/s |
| `select * from market.trades_1m` | 1,000,000 | 822 ms | 1.22M rows/s |

```bash
cargo bench -p yggdryl --bench media -- media/xmla
```

End to end, on the same machine: `yggdryl xmla serve market=C:\data\market` built in release with the `iceberg` feature and no trace, serving the nine-column Iceberg `trades` tables the Excel captures read, and ADOMD.NET 19.84.1 - the client library Power Query drives - executing `select * from market.<table>` over loopback and reading every cell in compiled .NET, warm. Beside it, pyarrow 25.0.1 reading the same tables' Parquet files straight off the disk.

| table | rows | first row, ADOMD.NET | every cell, ADOMD.NET over XML for Analysis | the Parquet files, pyarrow |
| --- | ---: | ---: | ---: | ---: |
| `trades` | 10,000 | 0.02 s | 0.23 s | 0.01 s |
| `trades_100k` | 100,000 | 0.04 s | 2.1 s | 0.01 s |
| `trades_1m` | 1,000,000 | 0.07 s | 19.7 s | 0.06 s |

The provider writes the million rows in 0.82 s; the rest is the client reading them as XML text, which is what the protocol carries. Regenerated by hand: serve the folder as above and time `AdomdCommand.ExecuteReader` draining every row, and `pyarrow.dataset.dataset("<table>/data", format="parquet", partitioning="hive").to_table()` for the baseline.
