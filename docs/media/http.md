# HTTP messages

One whole HTTP/1.1 message - a request or a response, its head and its framed body - as RFC 9112 writes it, under `message/http`; the [HTTP backend](../holder/index.md#http) answers the same values, so a captured exchange and a live one read alike.

## Overview

| | |
| --- | --- |
| Declared by | `message/http`, `.http` |
| Build | the `http` feature |
| Rust | `yggdryl::http`: `Request` and `Response` (`from_bytes`, `into_bytes`, `into_scalar`), and the bare grammar - `parse_request`, `parse_response`, `render_request`, `render_response`, `decode_chunked`, `encode_chunked` |
| JavaScript | `http.Request` and `http.Response`: `fromBytes`, `intoBytes`, `intoScalar` |
| Python | none: the message doors are Rust and JavaScript only |

`Request::from_bytes` and `Response::from_bytes` parse one message and `into_bytes` renders it back; a transfer the [HTTP backend](../holder/index.md#http) made is the same value. A server's trace folder ([Serving a handle](../holder/index.md#serving-a-handle)) is a folder of these documents, one request and one response per exchange.

```text
Response::from_bytes(&[u8]) -> Result<Response>     // status line, headers, framed body; the URL about:blank
Request::from_bytes(&[u8]) -> Result<Request>       // the target joined onto Host
response.into_bytes() / request.into_bytes()        // the message back: names lower case, lexical order
response.into_scalar() -> Result<Scalar>            // {status, reason, version, url, headers, body}
parse_request / parse_response -> (head, body)      // the grammar alone: RequestHead, ResponseHead
render_request / render_response                    // a head and a body back to bytes
decode_chunked(reader) / encode_chunked(writer)     // the chunked framing over std::io
```

## Read

A recipient reads what RFC 9112 lets it read, and refuses the rest as `Error::Parse` with target `http message` at the byte position it stopped:

- a line ends in CRLF, a bare LF tolerated as the terminator and a bare CR anywhere else refused; obs-fold is refused;
- a request, status, field or chunk-size line above `MAX_LINE_BYTES` (8192) and a head of more than `MAX_FIELD_LINES` (256) field lines are refused before they are read further;
- `Content-Length` is a decimal every repetition agrees on and never stands beside `Transfer-Encoding`; the one transfer coding read is `chunked`, its extensions ignored and its trailers folded into the headers;
- a `1xx`, `204` or `304` response has no body whatever its headers state, and a response stating no framing reads to the end of the input; the input must be one message;
- a field value's obs-text is read through `Charset::transcribe`, so a legacy byte is its windows-1252 character rather than a refusal;
- a `Content-Encoding` is kept on the body as sent: `bytes`, `text` and `scalar` decode it, `into_bytes` renders it coded, and a coding this crate cannot decode is refused when the message is parsed;
- `parse_response` reads past every interim `1xx` answer other than `101` to the final one, so `Response::from_bytes` answers the status the exchange ended on and `into_bytes` renders only that last answer; `101` is itself final.

=== "Rust"

    ```rust
    use yggdryl::http::{Method, Request, Response, Status};
    use yggdryl::{MimeType, Scalar, Url};

    // The name declares the medium.
    assert_eq!(Url::from_str("file:///capture.http")?.media_type().base(), &MimeType::HTTP);

    let wire = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Transfer-Encoding: chunked\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n";
    let response = Response::from_bytes(wire)?;
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.text()?, r#"{"a":1}"#);
    // Decoded as RFC 9112 says: the length replaces the transfer coding.
    assert_eq!(response.headers().get("content-length"), Some("7"));
    assert_eq!(response.headers().get("transfer-encoding"), None);
    // As one record: status, reason, version, url, headers and body.
    let record = response.into_scalar()?;
    assert_eq!(record.get_key_str("reason").and_then(Scalar::as_str), Some("OK"));

    // A request's target is joined onto its Host.
    let request = Request::from_bytes(b"GET /v1/orders?limit=2 HTTP/1.1\r\nHost: api.example.com\r\n\r\n")?;
    assert_eq!(request.method(), Method::Get);
    assert_eq!(request.url().to_string(), "http://api.example.com/v1/orders?limit=2");
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { http } = require('yggdryl')

    const wire = Buffer.from(
      'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n' +
        'Transfer-Encoding: chunked\r\n\r\n7\r\n{"a":1}\r\n0\r\n\r\n',
    )
    const response = http.Response.fromBytes(wire)
    assert.equal(response.statusCode, 200)
    assert.equal(response.text(), '{"a":1}')
    // Decoded as RFC 9112 says: the length replaces the transfer coding.
    assert.equal(response.headers.get('content-length'), '7')
    assert.equal(response.headers.get('transfer-encoding'), null)
    // As one record: status, reason, version, url, headers and body.
    assert.equal(response.intoScalar().asJs().reason, 'OK')

    // A request's target is joined onto its Host.
    const request = http.Request.fromBytes(Buffer.from('GET /v1/orders?limit=2 HTTP/1.1\r\nHost: api.example.com\r\n\r\n'))
    assert.equal(request.method, 'GET')
    assert.equal(String(request.url), 'http://api.example.com/v1/orders?limit=2')
    ```

## Write

`into_bytes` renders the head - a request's target from its URL's path and query, with a `host` field from the URL where the request states none - then the field lines, names lower case in lexical order, then a `content-length` where no framing is stated and there is a body to frame (never on a `1xx`, `204` or `304`), then the body under the framing the headers state. `encode_chunked` writes the chunked framing over any writer, and `decode_chunked` reads it back.

=== "Rust"

    ```rust
    use std::io::Write;

    use yggdryl::http::{encode_chunked, Request, Response, Status};

    // Names lower case, then the length that frames the body.
    let response = Response::new(Status::OK)
        .with_header("Content-Type", "application/json")?
        .with_body("{\"a\":1}");
    assert_eq!(
        String::from_utf8(response.into_bytes()?)?,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 7\r\n\r\n{\"a\":1}"
    );

    // A request's target is its URL's path and query, and its `host` the URL's.
    let request = Request::post("https://api.example.com/v1/orders", "{\"symbol\":\"AAPL\"}")?
        .with_header("Accept", "application/json")?;
    assert_eq!(
        String::from_utf8(request.into_bytes()?)?,
        "POST /v1/orders HTTP/1.1\r\naccept: application/json\r\nhost: api.example.com\r\n\
         content-length: 17\r\n\r\n{\"symbol\":\"AAPL\"}"
    );

    // Rendered and parsed again, the message is the same one.
    assert_eq!(Response::from_bytes(&response.into_bytes()?)?.text()?, response.text()?);

    // The chunked framing on its own, over any writer.
    let mut chunked = encode_chunked(Vec::new());
    chunked.write_all(b"{\"a\":1}")?;
    assert_eq!(chunked.finish()?, b"7\r\n{\"a\":1}\r\n0\r\n\r\n");
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { http } = require('yggdryl')

    // A request's target is its URL's path and query, and its `host` the URL's;
    // names are lower case, then the length that frames the body.
    const request = new http.Request('POST', 'https://api.example.com/v1/orders')
      .withHeader('Accept', 'application/json')
      .withBody('{"symbol":"AAPL"}')
    assert.equal(
      request.intoBytes().toString(),
      'POST /v1/orders HTTP/1.1\r\naccept: application/json\r\nhost: api.example.com\r\n' +
        'content-length: 17\r\n\r\n{"symbol":"AAPL"}',
    )

    // A parsed message renders back as the same one, its transfer coding decoded.
    const response = http.Response.fromBytes(
      Buffer.from('HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{"a":1}\r\n0\r\n\r\n'),
    )
    assert.equal(
      response.intoBytes().toString(),
      'HTTP/1.1 200 OK\r\ncontent-length: 7\r\ncontent-type: application/json\r\n\r\n{"a":1}',
    )
    assert.equal(http.Response.fromBytes(response.intoBytes()).text(), '{"a":1}')
    ```
