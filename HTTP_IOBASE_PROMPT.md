# HTTP over `IOBase`: a session, a request, an answer, a stream, and a byte leaf

Thirteen pieces of work on `claude/rust-http-stream-implementation-oxd1ea`,
each a design change with its own decision in `DECISIONS.md` written before the
code that keeps it, in this order because each one moves what the next one reads:

1. The store-neutral half of `holder/object/client.rs` moves to
   `holder/wire/`, and `object` and `http` both consume it.
2. `HttpMethod` becomes the crate's one spelling of an HTTP verb, and
   `holder/object/request.rs` stops spelling it `&'static str`.
3. `HttpVersion` is the crate's one spelling of a protocol version. This
   build speaks HTTP/1.1, and HTTP/1.0 when pinned; the other two are named,
   refused by name, and their blockers written down.
4. A field name is lowercased once at intake, and the five fields the
   transport owns are refused to a caller.
5. Certificate verification is skipped only when a caller asks, and such a
   session never touches the shared connection pool.
6. A header is a `Metadata` entry under the `http:` keys the crate already
   owns, in both directions, and this module writes no second parser.
7. `HttpSession`, `HttpRequest` and `HttpResponse` are one exchange: the
   `requests`-recognizable door, wide at intake and typed past it.
8. `HttpStream` resumes a cut transfer from `start + delivered` under
   `If-Range`; it never restarts, and it never reports a cut as end of stream.
9. A transport failure replays an idempotent method and refuses a
   non-idempotent one; `holder::object` keeps its present behaviour by name.
10. A redirect is followed and its resolution is the session's;
    `IOBase::url()` never moves.
11. `HttpFile` and `HttpPath` are the two roles. There is no `HttpFolder`,
    because an origin has no listing primitive to promise one with.
12. `http://` and `https://` resolve through `Holder::from_url`, which is
    what `Scheme::is_storage` has been promising since it was written.
13. The cost model is an assertion: request counts, call counts, allocation
    pins, a benchmark, and a docs page that states the same numbers.

The transport extraction (piece 1) is one commit or none. A parallel copy of
the retry loop is forbidden by `AGENTS.md:52-58`, and
`rust/src/holder/object/tests/{accounting,protocol}.rs` must pass
**byte-for-byte unchanged** before a single line of `holder/http/` is written.

The order holds with one addition, and the prompt's own gate is what forces it:
pieces 4, 5 and 6 each name files under `holder/http/` in their **What moves.**,
and piece 1's commit adds `--no-default-features --features http --lib` and
`--features "object http"` to Gate 1 — but the `http` feature and
`holder/http/mod.rs` are not created until piece 7. So piece 1 lands `http = ["wire"]`, the
`#[cfg(feature = "http")] pub mod http;` line and an empty `holder/http/mod.rs`
beside `wire`, or pieces 1 to 6 cannot pass the gate piece 1 itself adds and
pieces 4 to 6 have nowhere to write. Nothing else moves: piece 7 still writes
every type in that module.

## Outcome

- `rust/src/holder/wire/` owns the agent, the pool, the token-budgeted
  jittered retry, the bounded drain, the range arithmetic, `HttpMethod` and
  the request tally. `holder/object/client.rs` owns the endpoint, SigV4,
  OAuth, Shared Key, the region redirect, the XML `<Error>` document and
  multipart, and nothing else. There is no second copy of any moved item and
  no `StatsSnapshot` beside `WireStatsSnapshot`.
- `yggdryl::holder::http::{HttpSession, HttpRequest, HttpResponse, HttpStream,
  HttpFile, HttpPath, HttpAuth, HttpBody, HttpOptions, Validator}` exist;
  `HttpMethod` is re-exported from `holder::wire`. Nothing in the crate spells
  an HTTP verb as a string literal.
- `Holder::from_url("https://…")` answers an `HttpPath`; the
  `Error::unsupported("holding a location of this scheme", …)` arm at
  `holder/mod.rs:234-237` is no longer reached for `http`/`https`, and
  `Scheme::is_http()` exists beside `is_object_store()`.
- A ranged read is one `GET` and never a `HEAD` first. A 64 MiB drain at
  64 KiB batches is one request, not 1024. Construction, `url`, `media_type`,
  `is_container`, `is_atomic`, `is_tabular`, `ls`, `child_by_path`, `parent`
  and `reserve` are zero. Every number in this paragraph is in
  `rust/src/holder/http/tests/accounting.rs` with the count in the test's name.
- A cut transfer re-opens at `start + delivered` with `If-Range`, surfaces a
  `200` answer to that conditional as `Error::Conflict`, and never returns
  `Ok(0)` for a connection that died with bytes outstanding. The worst case for
  one drain in which no byte arrives is `max_attempts + (max_attempts − 1) ×
  max_attempts = 9` requests, and 24 when the open crosses the redirect cap,
  because the initial open runs the same retry ladder and each hop re-runs it;
  with bytes arriving nothing bounds it, and the module doc states that third
  case rather than eliding it.
- An abandoned body returns its connection to the pool, **including when the
  origin answered chunked** — which `Pooled::drop` does not do today.
- Headers go out by iterating one `BTreeMap` range with zero allocations and
  come back through one fold pass and exactly one `Metadata::from_entries`.
  Two disagreeing `Content-Length` headers are `Error::Conflict`, never a
  merge. `Set-Cookie` is reachable and is not in the snapshot.
- `with_tls_verification(false)` is the one spelling, defaults to verifying,
  never reaches the shared agent, logs once, and prints plainly in `Debug`.
  `insecure` is refused by name.
- `HttpVersion` is the crate's one spelling of a protocol version. This build
  speaks HTTP/1.1 and, when pinned, HTTP/1.0 — both of which `ureq` already
  frames and pools differently. `Http2` and `Http3` are named and refused by
  name with `Error::unsupported`, never clamped, and `ureq::http::Version` is
  spelled in exactly one function. No ALPN is offered, because ureq sends none
  and cannot frame what an offer would select.
- Every field name leaves lowercase, `holder/object/aws/credentials.rs:266`
  stops being the crate's one uppercase one, and `connection`, `keep-alive`,
  `proxy-connection`, `transfer-encoding` and `upgrade` are refused to a
  caller because the transport owns framing.
- `docs/holder/backends/http.md` exists, is in `mkdocs.yml`, states the same
  cost table, and carries `Rust-only.` as the whole body of its Python and
  JavaScript tabs.

## Read first

- `AGENTS.md` in full. Gate 1 is §2. The crate denies `unsafe_code`
  (`rust/src/lib.rs:11`) and `holder/local/file.rs:14` is its only unsafe
  site — that stays true. `.api-inventory.txt` and `.api-bindings.txt` are
  hand-maintained and move in the same commit as the name they list.
  §"Storage: IOBase" (`:498-624`) is the contract every piece from 6 on is
  read against; **`:621` ("a 3xx is never followed") is argued with by
  piece 10**, not quietly ignored.
- `rust/src/iobase.rs` in full. Nine methods have no default body
  (`:124, 149, 152, 155, 163, 170, 173, 192, 195`); implementing only those
  compiles and is wrong on cost. The Laziness contract is `:86-110`, the
  Invariants `:111-117`, and `DEFAULT_STREAM_BATCH_SIZE` (`:48`, 64 KiB)
  and `DEFAULT_FETCH_BYTE_SIZE` (`:59`, 1 MiB) are deliberately different
  sizes — the doc at `:51-61` says why, and piece 13 depends on it.
- `rust/src/holder/object/client.rs` in full (2836 lines). This is the thing
  being cut in half. The retry constants are `:35-51`, the pool and agent
  `:2377-2412`, the retry loop `:740-782` and `:884-1029`, `Resuming`,
  `is_resumable` and `is_retryable_transport` in the `:2414-2596` block.
- `rust/src/holder/object/file.rs` in full (834 lines). It is the shape
  `HttpFile` copies: the cost table as the type's own rustdoc (`:16-36`), the
  laziness note (`:38-43`), the stage-then-publish model (`:45-51`), the
  `Mutex<State>` dropped before every wire call (`:426-430`), the
  `clear`-is-a-departure rustdoc (`:363-377`), and `Drop` publishing (`:797-805`).
- `rust/src/metadata.rs:29-46` — seventeen `http:` key constants already
  exist. `:67-73` — `for_each_well_known_protocol!` already emits `as_http`,
  `HttpField`, `HttpFieldMut`, and the comment at `:62-66` says `https` is
  deliberately absent so one namespace owns a header. `:479` —
  `Metadata::protocol(&Scheme)`. `rust/src/metadata/protocol.rs:41,:90` —
  `ProtocolMetadata` and its allocation-free `iter`.
- `rust/src/metadata/validation.rs` — `canonicalize_metadata_key` (`:317`),
  `validate_http_header_name` (`:358`), `is_http_token_byte` (`:370`),
  `validate_http_header_value` (`:391`), `parse_content_length` (`:406`).
  **The HTTP field grammar is already written.** Piece 4 adds no second one.
- `rust/src/media_type.rs:160` (`from_content_headers`),
  `rust/src/mime_type.rs:396,:601` (`from_content_coding`, `content_coding`),
  `rust/src/codec.rs:128,:168` (`as_str`, `has_restarts`),
  `rust/src/types/protocol/http.rs:74,:113,:131,:420`.
- `rust/src/holder/mod.rs` — `Holder` (`:72-127`), `from_url` (`:200-252`)
  and its refusal arm (`:234-237`), `system_time_ns` (`:33-40`).
- `rust/src/holder/zip/mod.rs:15-18` — how a backend whose store does not fit
  the location/container/leaf trio argues the departure in its own module
  doc. Piece 11 argues in that voice.
- `rust/src/holder/object/tests/server.rs:1-14` — the leaf-file `#[path]`
  include shape the in-process origin copies.
- `rust/src/holder/counted.rs:75-140` — `Call`, the canonical list of the
  surface a backend implements, and the spellings piece 13's tallies use.
- `DECISIONS.md` — the last decision is 39. These thirteen are 40 to 52, one per
  piece in order, in the file's current format: `**Rule.**`, the argument in
  bold-led paragraphs, `**What moves.**`, `**Written in:**`, `**Fixtures:**`.
  `**Why.**` is not part of it — no decision since 32 uses it, and 38 and 39 do
  not.
- ureq 3.4.0 is not vendored here. Fetch it before pieces 3 and 5:
  `curl -sSL -o ureq-3.4.0.crate https://static.crates.io/crates/ureq/ureq-3.4.0.crate && tar xzf ureq-3.4.0.crate`.
  The API piece 5 depends on is `src/tls/mod.rs:199`, `:82`, `:265` and
  `src/config.rs:462`. Fetch `ureq-proto` 0.6.1 the same way for piece 3:
  `src/ext.rs:64-65` is where every version but HTTP/1.0 and HTTP/1.1 is
  refused, `src/client/prepare.rs:19` is where HTTP/1.0 loses pooling, and
  `src/chunk.rs:144-172` is where the chunked trailer part is parsed and
  thrown away.
- The code each piece moves is named under it.

## 1. The transport gets a third home

**Today.** `rust/src/holder/object/client.rs` is 2836 lines and owns two
unrelated things. Store-neutral: the `ureq` agent and its pool
(`build_agent`, `:2377-2412`), the six retry constants (`:35-51`), the token
budget and its jittered backoff (`:682-738`, `:2414-2470`), the retry loop
(`send` `:740-782`, `attempt` `:884-932`, `stream` `:934-995`, `open_stream`
`:997-1029`), `Pooled` and the bounded drain, `Resuming`, `is_resumable` and
`is_retryable_transport` (`:2540-2596`), `total_of_content_range`
(`:2620-2624`), the range arithmetic (`:2288-2302`), and `Stats`/`StatsSnapshot`
(`:58-136`). Store-specific: the endpoint, SigV4, the OAuth assertion, Shared
Key, the one-shot region redirect (`:766-774`, `:972-980`), the XML `<Error>`
document, multipart. Everything above is `pub(super)` to `holder::object`, so
nothing outside that module can reach any of it. `StatsSnapshot` is public and
re-exported at `object/mod.rs:122`, reached from `File::stats`, `Folder::stats`
and `Path::stats`, with a doctest at `client.rs:71-76`.

**Rule to write (decision 40).** The store-neutral half is `holder::wire`, and
neither backend owns it. `object` depending on `http` inverts the ownership
story — the signed store backend made downstream of the unsigned one — and
`http` depending on `object` makes a plain HTTP reader compile SigV4, `hmac`,
`md-5`, `ring` and `sha2` for nothing. So a third module, consumed by both:
`Wire`, `WireOptions`, `WireRequest`, `WireAnswer`, `WireBody`,
`WireStatsSnapshot`. `Wire::shared()` is the process-wide agent every
default-configured caller gets; `Wire::for_options` hands it out only when
`WireOptions::is_custom()` is false. `object::StatsSnapshot` is **deleted** and
`holder/object/mod.rs` re-exports `WireStatsSnapshot` under the old name — two
owners of one fact is the thing this piece exists to remove, so keeping both is
refused. The region redirect stays in `object` as a caller-supplied hook, which
leaves the `redirected` bool and one `stats.retries` increment to re-home; do
that in this commit, not later. Every moved item is deleted from `client.rs` in
the same change. Per-phase timeouts stay per-phase and the rationale at
`client.rs:2382-2388` moves with them. `MAX_DOCUMENT` (32 MiB) now bounds every
error document and every non-streamed answer body, which fixes `object`'s
unbounded error-body `read_to_end` in `open_range` (`:1285`), `get_all`
(`:1333`) and `open_reader_range` (`:1417`) as a side effect. The object bytes
`get_all` returns stay bounded only by `try_reserve_exact`.

**Edges.**

- **`MAX_DOCUMENT` bounds a document, never a value.** It bounds a body this
  module reads in order to decide something — the error document in `attempt`
  (`client.rs:917`) and `stream` (`:964`), and a non-streamed `send()` answer.
  `get_all` (`:1320`), `get_range_vec` (`:1215`), `open_range` (`:1252`) and
  `open_reader_range` (`:1398`) keep their unbounded value read, sized by
  `try_reserve_exact` through `crate::iobase::oversized`, and `open_stream`
  keeps `.limit(u64::MAX)` (`:1025`). Capping a value read changes the object
  backend's behaviour, which this piece's own gate forbids and no object test
  catches — the largest body under `object/tests/` is 15 MiB
  (`dialects.rs:305`) — and fails piece 13's 64 MiB drain assertion. One
  spelling for the bound, and it **fails** rather than truncating:
  `.limit(MAX_DOCUMENT).read_to_vec()` (`:917`) moves;
  `.take(MAX_DOCUMENT).read_to_end(..).ok()` (`:964`) is **deleted, not moved**,
  and so are the three unbounded `read_to_end(..).ok()` error-body reads at
  `:1285`, `:1333` and `:1417`. `take` truncates in silence and the `.ok()`
  discards the read failure with it, so an oversized error page is parsed as if
  it were whole; the bound is therefore selected from the status **before**
  `into_body()`, because a `take` layered over `open_stream`'s boxed reader is
  exactly that shape. On a value path never ureq's `read_to_vec`,
  `read_to_string` or `read_json`: each carries ureq's own 10 MiB default
  (`ureq-3.4.0/src/body/mod.rs:30,:298,:332`), and `BodyExceedsLimit` arrives as
  `ErrorKind::Other` (`src/body/limit.rs:24`, `src/error.rs:198-204`), which
  `is_resumable` reads as a cut — so a 64 MiB read re-opens under `If-Range`
  and costs two requests against piece 13's asserted one.
- **`Wire::shared()` shares the `ureq::Agent` and nothing else.** `Agent` is
  three `Arc`s (`ureq-3.4.0/src/agent.rs:90-99`), so every `Wire` clones it and
  sends on the one pool while `WireStats`, the `RetryBudget` and the jitter
  counter are built fresh per `Wire` — exactly where `client.rs:342-344` builds
  them per `Client` today — and `Wire::for_options` returns a new `Wire` around
  a shared or custom agent, never a shared `Wire`. Handing back a shared `Wire`
  fails in two directions and neither is caught: one origin's 5xx storm drains
  the 500 tokens `settle` refuses to refund and stops retries for every
  unrelated session, and `File::stats()`/`Folder::stats()`/`Path::stats()`
  start reporting the whole process for a value the rustdoc moving to
  `wire/stats.rs` calls one client's — `object/tests/accounting.rs:958` is the
  only stats assertion in that suite and it is `>=`. The module doc states the
  remaining scope deliberately: one `Wire` spanning several origins shares one
  budget, which bounds this client's total retry load and is not per-host
  fairness. No per-host map is added.
- **`build_agent` sets the three pool bounds explicitly and inherits none.**
  ureq's defaults are `max_idle_connections` 10, `max_idle_connections_per_host`
  **3** and `max_idle_age` **15 s** measured from last use
  (`ureq-3.4.0/src/config.rs:885-887`, `src/pool.rs:129,:225,:273`), and
  `build_agent` sets none of them today. Inherited, eight `HttpFile` handles
  over one shared `HttpSession` — the parallel shape piece 13 prescribes — keep
  three idle connections and re-handshake the other five on every request, and
  any caller that leaves 15 s between two page reads re-handshakes on each one.
  Piece 13's "1000 sequential ranged reads are 1 TCP connection and 1 TLS
  handshake" then holds only of the in-process origin, which is one host,
  sequential and instant, so the assertion stays green while deployment pays a
  handshake per read. The three are constants in `holder/wire/agent.rs` beside
  `POOLED_DRAIN_LIMIT`, **not** `WireOptions` fields: nothing would read a
  caller-set value, and `is_custom()` would have to grow an arm for each or
  silently drop it. The per-host cap equals the total cap, so one busy origin
  can use the whole pool, and the doc states what the cap is bought with — a
  pooled connection holds a 128 KiB input and a 128 KiB output buffer once used,
  doubled over TLS because the rustls transport keeps its own over the TCP
  transport's (`config.rs:883-884`, `transport/buf.rs:75-101`,
  `tls/rustls.rs:78-84`), ≈512 KiB per idle TLS connection.

**What moves.** New `rust/src/holder/wire/{mod,agent,send,budget,body,range,
method,stats}.rs`. Deletions and call-throughs in `holder/object/client.rs`.
`holder/object/mod.rs` (the re-export). `holder/mod.rs` (`pub mod wire;`).
`rust/Cargo.toml`: `wire = ["dep:ureq"]`, `object = ["wire", "dep:hmac",
"dep:md-5", "dep:ring", "dep:sha2"]`. `.api-inventory.txt` gains a
`### yggdryl::holder::wire` section and loses `object::StatsSnapshot`. The
`client.rs:71-76` doctest moves to `wire/stats.rs`. A `wire`-only build with
neither `object` nor `http` must compile, or the extraction has left a module
nothing consumes.

## 2. One spelling of an HTTP verb

**Today.** `rust/src/holder/object/request.rs:13` is `method: &'static str`,
and every `Request::new` call site passes `"GET"`, `"PUT"`, `"POST"`,
`"DELETE"` or `"HEAD"` as a literal. There is no type that knows a method is
safe, idempotent, or carries a body — which is exactly the fact piece 9 needs.

**Rule to write (decision 41).** `holder::wire::HttpMethod` is the crate's one
spelling: `Get, Head, Post, Put, Patch, Delete, Options`, `ALL` in that order,
`as_str` the uppercase token, `sends_body`, `is_safe`, `is_idempotent`
(`GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS` yes; `POST` and `PATCH` no),
`FromStr` refusing with the accepted vocabulary and the input, `Display`.
`holder/object/request.rs` takes `method: HttpMethod` in the same commit;
no literal survives, and no alias is kept.

**What moves.** `holder/wire/method.rs`; `holder/object/request.rs` and every
`Request::new` call site in `aws/`, `google/`, `azure/` and `client.rs`; every
recorded-request assertion in `rust/src/holder/object/tests/` that compares a
method; `.api-inventory.txt`.

## 3. The version is what the answer says, and this build says HTTP/1.1

**Today.** Nothing in the crate names an HTTP version, and the transport already
speaks two. `ureq` frames HTTP/1.0 and HTTP/1.1 and refuses every other version
at the source: `ureq-proto-0.6.1/src/ext.rs:64-65` returns
`Error::UnsupportedVersion` for anything but `HTTP_10`/`HTTP_11`, and
`src/parser.rs:47-49` reads only minor 0 and 1 off a status line. It sends no
ALPN extension at all - `grep -rniE 'alpn|http2|HTTP_2' ureq-3.4.0/src` is
empty - and `ureq::tls::TlsConfig`'s builder exposes `provider`, `client_cert`,
`root_certs`, `use_sni`, `disable_verification` and
`unversioned_rustls_crypto_provider`, and no `alpn_protocols`. Both spoken
versions are live on both sides: `ureq-3.4.0/src/request.rs:301` takes
`version(Version::HTTP_10)`; `ureq-proto-0.6.1/src/client/prepare.rs:19` pushes
`CloseReason::CloseDelimitedBody` for it, so that connection is never pooled;
`client/recvresp.rs:133` reads the answer's version to pick the body mode;
`ureq-3.4.0/src/run.rs:165` carries it onto the response; and
`holder/object/tests/server.rs:1569,1633-1646` already models the 1.0-versus-1.1
keep-alive difference. `StatsSnapshot` (`client.rs:69-101`) counts requests by
method and nothing by version, and `Pooled::drop` (`client.rs:2318-2325`) spends
up to `POOLED_DRAIN_LIMIT` (1 MiB, `:2368`) draining an abandoned body to save a
TLS handshake it cannot save on HTTP/1.0.

**Rule to write (decision 42).** `holder::wire::HttpVersion` is the crate's one
spelling of a protocol version, a closed enum in `Codec`'s shape: `Http10`,
`Http11`, `Http2`, `Http3`, declared oldest first so `Ord` reads as "newer is
greater"; `ALL`; `SPOKEN = [Http10, Http11]`; `as_str` the RFC 9110
`HTTP-version` token; `is_spoken`; `pools_connections`; `FromStr` accepting the
canonical spelling, the ALPN identifier and the bare number (`HTTP/2`,
`HTTP/2.0`, `http2`, `h2`, `2` are one version) and refusing with the accepted
vocabulary and the input; `Display`, `Serialize`, `Deserialize`. `h2c` is
**refused**, not aliased: cleartext HTTP/2 is a different negotiation - prior
knowledge, or the `Upgrade` exchange RFC 9113 removed - and accepting the name
would claim a version the token does not mean.

Naming a version and speaking one are separate, and that is the whole design.
`HttpOptions::with_version(HttpVersion)` **pins**; it is not `max_version`,
because nothing is negotiated and a ceiling would be vocabulary for a mechanism
that does not exist. A version outside `SPOKEN` is
`Error::unsupported("speaking this HTTP version", version.as_str())` -
**refused by name, never clamped**, because clamping downgrades what a caller
asked for and says nothing.

Three points, and no fourth:

1. **Offered: nowhere.** This build sends no ALPN, because ureq sends none and
   cannot frame what an ALPN offer would select. Offering `h2` and then writing
   HTTP/1.1 bytes onto a connection the server selected `h2` for is a protocol
   violation; sending nothing is correct and is what ships.
2. **Decided: one function.** `holder/wire/send.rs` holds
   `fn wire_version(HttpVersion) -> ureq::http::Version` and its inverse, at the
   `ureq::http::Request::builder()` call site that is `client.rs:893` and
   `:1003` today. That is the **only** place `ureq::http::Version` is named, and
   **no `ureq::` type appears in any signature outside `holder/wire/`** - which
   costs nothing, because `client.rs:198` already erases a streamed body to
   `(u16, Vec<(String, String)>, Box<dyn Read + Send>)` and `open_stream`
   (`:997-1029`) is the only function in 2836 lines that names one on the way
   out. That rule is the entire seam.
3. **Recorded and reported.** `WireAnswer` carries the version the answer
   arrived on; `HttpResponse::version()` and `HttpStream::version()` read it;
   `WireStatsSnapshot::requests_by_version: [u64; HttpVersion::ALL.len()]` with
   `requests_on(version)` counts it. The array sums to `requests` and an
   assertion says so, which is what keeps a newly spoken version from going
   uncounted; the fixed length makes a new variant a compile error in
   `WireStats::snapshot` rather than a silently dropped column, and it keeps the
   `Copy`/`Default`/`Eq` reading `StatsSnapshot` has today.

`pools_connections()` has one reader and it is a **fix**, not a seam:
`HttpStream::drop` (piece 8) drains only when the answer's version pools
connections. Draining an abandoned HTTP/1.0 body returns nothing - the
connection closes regardless - so the object backend spends up to a mebibyte
there for nothing.

Written and not written, with the reason, because the reason is the decision:
`HttpRequest` carries **no** version, because a version is a property of the
connection and the connection is chosen per origin - which is why every public
signature in this module names `HttpVersion` at most as a returned value, and
why the day a second version lands only the private transport handle inside
`holder/wire/` grows an arm. There is **no** `multiplexes()`, `allows_0rtt()`,
`alpn_token()`, `requires_lowercase_fields()` or `has_trailers()`: each is
constant over what this build speaks or has no reader, and `AGENTS.md:64`
forbids speculative generality by name. There is no `Wire` enum with one
variant, no transport trait, no `http2.rs`, and no `#[cfg(feature = "http2")]`.
`Provider` (`AGENTS.md`, "Object stores") is the house shape for a dispatcher
and it is written when the second value exists.

The module doc and `docs/holder/backends/http.md` say what is true and name the
blocker, so nobody rediscovers it: this module speaks HTTP/1.1, and HTTP/1.0
when pinned. `ureq` frames neither of the other two
(`ureq-proto-0.6.1/src/ext.rs:64`). HTTP/3 has no path - `quiche` is the only
sans-io HTTP/3 implementation, its only TLS backend is BoringSSL through
`cmake`, against `rust/Cargo.toml:52`'s own standard, and it declares MSRV 1.88
against the 1.85 floor Gate 1 checks; `quinn-proto` 0.11.17 is pure Rust,
sans-io and MSRV exactly 1.85 but is QUIC transport only, so RFC 9114 and RFC
9204 QPACK would be this crate's code. HTTP/2 **is** reachable synchronously -
`h2` pulls tokio only for the `AsyncRead`/`AsyncWrite` traits and can be driven
by a hand-rolled `std::task::Waker` over a blocking socket with no executor and
no `unsafe` - and it is **not taken**: eleven crates and a connection driver
whose failure mode is a silent hang, bought on no measurement. It is revisited
when a profile from the deployment network shows many concurrent small ranges
dominating **and** `HttpFile` issues ranges in batches; neither is true. One
comment records that, and the numbered pieces move on.

**Edges.**

- **An attempt that never got an answer has no version.** `requests` counts an
  attempt where it goes out, so a refusal at the transport — `Io`, `Timeout`,
  `ConnectionFailed`, `HostNotFound`, none of which carries a response
  (`ureq-3.4.0/src/error.rs:31-63`) — advances `requests` while no bucket can
  advance. `WireStatsSnapshot` therefore carries `requests_unanswered: u64`,
  incremented on exactly those four, and the assertion is
  `requests_by_version.iter().sum::<u64>() + requests_unanswered == requests`.
  Never bucket an answerless attempt under the pinned version or a default:
  that publishes a version the wire never spoke, and `HttpVersion` gains no
  `Unknown` variant. Never move the `requests` increment to answer time
  instead: it undercounts the retry and per-hop worst cases this prompt
  asserts. Without the term the sum fails the first time any transport-failure
  or retry test runs, and piece 9's whole test file produces them.
- **`pools_connections()` answers for the version a request is *sent* on, and
  is not the whole drain gate.** ureq marks a connection unpoolable from three
  places and none of them reads the answer's version: the **request's** version
  (`ureq-proto-0.6.1/src/client/prepare.rs:19-22`), `Connection: close` from
  either side (`client/recvresp.rs:73-78`), and a close-delimited body
  (`:190-195`). So `HttpStream::drop` drains when the session's **pinned**
  version pools connections, the answer's `connection` field does not name
  `close`, and the answer's framing states an end — a `content-length`, or
  `transfer-encoding: chunked` — read off the already-folded snapshot and never
  re-parsed. Gate on the answer's version and an `HTTP/1.0` answer carrying a
  `Content-Length` is pooled by ureq (`ureq-proto/src/body.rs:351-361` gates
  only the chunked branch) while the drain is skipped: the connection this fix
  exists to save is burned, the counter shows one more and nothing says why.
  Gate on `keep-alive` instead and it is the same error inverted — a session
  pinned to `Http10` is unpoolable whatever the answer says, so draining it
  copies up to a mebibyte for a connection ureq has already discarded. A body
  with neither framing header is close-delimited and condemned before the drain
  starts, so draining it blocks in `Drop` until the server closes or the cap is
  hit and buys nothing, on every abandoned stream — and
  `hashing/xxhash/stream.rs:50-52`, `media/iceberg/manifest.rs:726-730` and
  `media/ipc/mod.rs:196-206` abandon deliberately. "Stated no length" is not
  the test: chunked is the case the fix is for, so `chunk_next` answers
  `transfer-encoding: chunked` rather than merely omitting `content-length`, or
  piece 13's "six abandoned streams are 1 connection" pins the wrong body.

**What moves.** New `rust/src/holder/wire/version.rs`; `holder/wire/mod.rs`
(`pub use version::HttpVersion;`, re-exported from `holder/http/mod.rs`);
`holder/wire/send.rs` (`wire_version` and its inverse, the crate's only
`ureq::http::Version`); `holder/wire/stats.rs` (`requests_by_version`,
`requests_on`); `holder/wire/body.rs` (the drain gate on `pools_connections`);
`holder/http/options.rs` and `properties.rs` (`with_version`, the
`version`/`http_version`/`http-version` intake spellings in the *shape* of
`object/properties.rs:608` with an empty prefix list — that fn is private to
`holder::object` and peels ten store prefixes — and the refusal);
`holder/http/{response,stream}.rs` (`version()`);
`holder/http/tests/version.rs`; `holder/http/tests/origin.rs` (answer `HTTP/1.0`
and close on demand, as `object/tests/server.rs:1633-1646` already does);
`.api-inventory.txt`; `docs/holder/backends/http.md` (a Versions row stating
what is spoken, what is named, and the blocker for each named one).

## 4. A field name is lowercase, and framing is never a caller's header

**Today.** The crate sends exactly one uppercase field name:
`holder/object/aws/credentials.rs:266`, `.header("Authorization", token)`. Every
other name is spelled lowercase at its call site, and `request.rs:158`
(`header_name`) lowercases whatever a caller spells. The inbound side already
lowercases too - `metadata/validation.rs:317` `canonicalize_metadata_key` - and
lookups compare with `eq_ignore_ascii_case` (`client.rs:214`). So the rule is one
edit from being true and is enforced nowhere: `Request::header`
(`request.rs:79-80`) pushes `name.to_owned()` verbatim. Nothing refuses
`connection`, `keep-alive`, `proxy-connection`, `transfer-encoding` or
`upgrade`; no dialect sends one -
`grep -rniE '"(connection|keep-alive|proxy-connection|transfer-encoding|upgrade)"' rust/src/`
hits only `object/tests/server.rs` - but `holder/http`'s surface is a caller's,
and ureq owns framing.

**Rule to write (decision 43).** A field name is lowercased once, at intake, by
the setter - `HttpRequest::try_with_header` and `Request::header` both, through
one helper in `holder/wire/`, never twice and never at send time. RFC 9110 §5.1
makes field names case-insensitive, so this is the correct HTTP/1.1 behaviour
rather than a concession to anything: it is what makes piece 6's claim true that
two identical requests put byte-identical headers on the wire, it removes the
fold from SigV4's canonical request, which lowercases anyway, and it makes the
outbound side agree with `canonicalize_metadata_key`'s inbound one, so one fact
has one spelling in both directions. `credentials.rs:266` becomes
`"authorization"` in this change; no other call site moves.

Five names are refused outright by the setter - `connection`, `keep-alive`,
`proxy-connection`, `transfer-encoding`, `upgrade` - and `te` is accepted only
with the exact value `trailers`. The refusal is
`Error::unsupported("setting a field the transport owns", name)`, naming the
field. This is a present-day guard: a caller who sets
`transfer-encoding: chunked` through `HttpRequest` puts a second framing
authority on a connection ureq is already framing, which is the
request-smuggling shape `validate_http_header_value` (`validation.rs:391`)
refuses CR, LF, NUL and DEL for. The constant lives in `holder/wire/`, not in
`holder/http/`, because `holder::object` sends through the same door and one
list is the whole point.

Nothing else is owed, and the module doc says so rather than leaving it to be
rediscovered. There is **no** trailers accessor: ureq parses the chunked trailer
part and throws it away (`ureq-proto-0.6.1/src/chunk.rs:144-172`; `grep -rni
trailer ureq-3.4.0/src` returns nothing), so `HttpResponse::trailers()` would
answer empty on every version, which is a lie about the transport rather than a
seam - and `AGENTS.md:52-58` makes adding it later cheap. There is **no**
`requires_lowercase_fields()` predicate: the setter lowercases unconditionally,
so nothing would read it.

**Edges.**

- **A second refused list, `holder/http`'s own, named beside the framing five.**
  `accept-encoding`, `range`, `if-range`, `content-length` and `host` are
  refused by `HttpRequest::try_with_header`/`try_with_headers` with the same
  `Error::unsupported("setting a field the transport owns", name)` — and **only
  there**, never in `holder/wire/field.rs`, because `holder::object`
  legitimately sets `range` (`client.rs:1262,:1407-1408`) and `content-length`
  on four Azure writes whose Shared Key signature reads it back off the request
  (`azure/dialect.rs:53,66,79,100`, `azure/sign.rs:24,:117-124`), so a shared
  refusal breaks the backend piece 1 must leave byte-for-byte unchanged. The
  reason is not framing, it is a second authority over bytes this module
  already computes: `http::request::Builder::header` **appends**
  (`try_append`, `http-1.5.0/src/request.rs:909`) and ureq writes every entry it
  is handed, so a caller's copy is a second field line and not an overwrite. Two
  `Range` lines are one multi-range request per RFC 9110 §5.3 and the
  `multipart/byteranges` 206 is handed back as bytes; a stale `if-range` fails
  the condition and the 200 it produces is misread by `pread` as "the origin
  ignored `Range`", poisoning `accepts_ranges: false`; a caller's
  `accept-encoding: br` is the exact failure piece 7's setter refuses, arriving
  through the door that setter cannot guard — ureq adds no `Accept-Encoding` of
  its own once a caller set one (`ureq-3.4.0/src/run.rs:282,:315-321`) and
  decodes nothing with gzip and brotli off. `content-length` and `host` are the
  smuggling shapes `transfer-encoding` is refused for, one layer apart: ureq
  emits its own framing header only when the caller set none (`run.rs:277-280`),
  so `AmendedRequest::analyze` makes the caller's number the framing authority
  (`ureq-proto-0.6.1/src/client/amended.rs:173-199`) — `content-length: 0`
  beside an `HttpBody` makes `has_body()` false, `run.rs:177` never calls
  `send_body`, and the write publishes an empty resource under a `201`, while a
  value above the body never ends the writer and `run.rs:483-527` spins on
  `Ok(0)` inside a blocking `pwrite` nobody can cancel, since this build sets no
  send-body and no global deadline. `host` is derived from the URI only when the
  caller set none (`client/sendreq.rs:129-139`) while SNI, certificate
  verification and the pool key all come from the URI unconditionally
  (`ureq-3.4.0/src/tls/rustls.rs:62-66`, `src/pool.rs:187-196`): one origin
  gets two spellings, the URL deciding who is connected to and trusted and the
  header deciding which vhost answers, and the session's constant header
  snapshot carries the caller's value onto every redirect hop, where piece 10's
  crossing check reads `url.authority()` and never the header.
  `holder/object/sigv4.rs:28-33,:225` already treats `host` as signer-owned and
  derives it from the URL, and no object call site sends one. The module's own
  writer is not routed through the refusing setter; it writes at the
  `ureq::http::Request::builder()` call site.

**What moves.** New `rust/src/holder/wire/field.rs` (the lowercase helper and
the refused-name constant, named once); `holder/object/request.rs:79-80`
(`header`) and `:149-164` (`header_name`, which calls the helper instead of
folding its own); `holder/object/aws/credentials.rs:266`;
`holder/http/request.rs` (`try_with_header`, `try_with_headers`);
`holder/http/headers.rs` (piece 6 reads the helper rather than writing a
second); any recorded-header assertion in `rust/src/holder/object/tests/` that
spells a name with a capital; `holder/http/tests/headers.rs`;
`.api-inventory.txt`.

## 5. Verification is skipped only when asked, and never on the shared pool

**Today.** `build_agent` (`client.rs:2377-2412`) sets
`http_status_as_error(false)` (`:2392`), `max_redirects(0)` and
`max_redirects_will_error(false)` (`:2398-2400`), the four per-phase timeouts,
the user agent, and an optional proxy. It says nothing about TLS, so every
connection uses ureq's default verification and there is no way to reach an
origin behind a self-signed or corporate certificate.

**Rule to write (decision 44).** `WireOptions::tls_verification()` defaults to
`true`. `with_tls_verification(bool)` is the **only** spelling: no `insecure`,
no `skip_ssl`, no `danger_*`, and the inverted spelling `insecure` is *refused*
by name with `unsupported` naming `verify` as the spelling to use — accepting
two names of opposite polarity is the near-miss that produces a silent
downgrade. `agent.rs::tls_config` builds
`ureq::tls::TlsConfig::builder().provider(TlsProvider::Rustls)` and calls
`.disable_verification(true)` (verified `ureq-3.4.0/src/tls/mod.rs:199`,
reached through `ConfigBuilder::tls_config` at `src/config.rs:462`) only when
the option says so. The `rustls` `dangerous()` call is **inside ureq**
(`src/tls/rustls.rs:166-171`, zero `unsafe` in that file), so this compiles
under `#![deny(unsafe_code)]` with no allow anywhere, needs no new cargo
feature, and needs no rustls feature — rustls 0.23 removed the
`dangerous_configuration` gate. `rustls` does **not** become a direct
dependency; it arrives transitively and naming it would couple the crate to a
pre-1.0 semver for no gain.

Three gating layers, none of them a default:

1. Never the default. Property intake accepts `verify`, `tls_verify`,
   `ssl_verify`, `verify_ssl` and their `-`/`.` variants, canonicalized and
   parsed in the *shape* of `holder/object/properties.rs:608` and `:625` —
   both are private fns in a private module (`object/mod.rs:114`), and
   `:608`'s ten store prefixes would make `s3_verify` configure an HTTP
   session. `from_environment` reads `YGGDRYL_TLS_VERIFY` only, never
   an `HTTP_*` or vendor prefix, so a stray variable cannot disable
   verification for a process that never asked.
2. Never the shared pool. `WireOptions::is_custom()` is true whenever
   `!tls_verification()` or a root/client certificate is set, so such a
   session always builds its own agent. This is correctness, not hygiene:
   `Wire::for_options` hands out `Wire::shared()` whenever `is_custom()` is
   false, so without it the setting is silently dropped.
3. Never silent. `HttpSession::is_insecure()` reports it; `HttpOptions`'
   hand-written `Debug` prints `verify: false` plainly while continuing to
   mask every credential; one `log::warn!` at session construction names the
   host set; the docs Contract table carries a `TLS` row in ureq's own words.

`with_root_certificates(pem)` over `RootCerts::Specific` is the knob for a
corporate or proxy CA and is the one to reach for first.
`RootCerts::PlatformVerifier` is **never offered**: without ureq's
`platform-verifier` feature, which this build does not enable, it panics at
`src/tls/rustls.rs:183`. There is no automatic downgrade — a certificate
failure is `Error::Io` and stops.

**Edges.**

- **`is_custom()` is derived from every field `build_agent` reads, not from the
  TLS pair alone.** `timeout`, `connect_timeout`, `proxy`, `tls_verification`
  and the root/client certificates, true when any differs from the default;
  `ObjectOptions::has_custom_transport` (`options.rs:484-488`) folds into it and
  is deleted, so `object` keeps handing a proxied or long-timeout client its own
  agent. The user agent is **not** in the set: `build_agent` sends a
  compile-time constant (`client.rs:2411`) and ureq applies its config value
  only when the request carries no `user-agent` header
  (`ureq-3.4.0/src/run.rs:283,:340-343`), so piece 7's `with_user_agent` is a
  per-request header and forking the pool for it would give one fact two owners.
  A field `build_agent` reads that `is_custom()` does not name puts that session
  on the shared default agent and drops the setting with no error — on a
  corporate network, straight past the egress proxy. One test flips each field
  in turn and asserts `is_custom()`, with the default asserted false.
- **A custom session owns its agent, so it is built once and `Clone`d to fan
  out, never rebuilt per handle.** An agent owns its pool and its rustls
  `ClientConfig`, and therefore that config's TLS session-resumption store
  (`ureq-3.4.0/src/tls/rustls.rs:26-29,:95,:137-239`; `rustls`'s
  `ClientConfig::resumption` defaults to a per-config in-memory store). A
  default-configured session is free to rebuild because `Wire::for_options`
  folds it onto `Wire::shared()`, and that asymmetry is the trap: a custom one
  rebuilt per file buys a fresh pool and a cold full handshake every file,
  against this module's headline claim of 1000 reads on one connection, and
  construction is contractually 0 requests so nothing warns. `is_custom()`'s
  rustdoc and the docs Contract TLS row state the cost beside the correctness,
  and `holder/http/tests/tls.rs` asserts it over plain `http` from the origin's
  `connection_count`, where `is_custom()` still forces a private agent: 100
  `HttpFile`s over one cloned custom session are **1** connection, 100
  separately built custom sessions are **100**. That is also the only observable
  test of gating layer 2, because the std-only origin speaks no TLS. There is no
  pre-warm request and no `warm()`.
- **Property intake peels no store prefix.** `object/properties.rs:608`
  (`canonical`) and `:625` (`flag`) are private fns in a private module
  (`object/mod.rs:114`), so they are unreachable from `holder::http` and cannot
  be called; write this module's own in the *shape* of `:608` — trim,
  `to_ascii_lowercase`, `['-', '.'] -> '_'` — with an **empty** prefix list.
  Copying `:603-605`'s ten-prefix table makes `s3_verify=false` disable an HTTP
  session's TLS verification and `aws_version=1.0` pin its protocol version,
  because `Holder::from_url` (`holder/mod.rs:200-252`) hands one property bag to
  every scheme: a store's own knob silently configuring a store-neutral handle,
  with no diagnostic. Every accepted spelling is listed literally — `version`,
  `http_version`, `verify`, `tls_verify`, `ssl_verify`, `verify_ssl` and their
  `-`/`.` variants — and a name the list does not hold is **ignored**, as
  `object/properties.rs:471` and `holder/mod.rs:239-250` already ignore one, not
  refused: a shared bag carrying S3 knobs would otherwise fail every `https://`
  handle.

**What moves.** `holder/wire/agent.rs` (`tls_config`, the extended
`build_agent`), `holder/wire/mod.rs` (`WireOptions`, `is_custom`),
`holder/http/options.rs` and `properties.rs` (intake), `holder/http/tests/tls.rs`,
`docs/holder/backends/http.md` (the Contract table row).

## 6. A header is a `Metadata` entry, and there is no second parser

**Today.** `rust/src/metadata/validation.rs` already owns the whole RFC 9110
field grammar: `canonicalize_metadata_key` (`:317`) folds an `https:`/`HTTPS:`
prefix to the literal `"http"` and lowercases the key;
`validate_http_header_name` (`:358`) with `is_http_token_byte` (`:370`)
enforces the token set; `validate_http_header_value` (`:391`) refuses CR, LF,
NUL, DEL and every control but HTAB, which is the header-injection guard;
`validate_entry`'s `_` arm (`:299-312`) accepts any unknown `http:` name
unchanged, so there is no allowlist; and `http:content-length` is the one key
whose value is rewritten, through `parse_content_length` (`:406`), so `"00042"`
stores as `"42"`. `Metadata::protocol(&Scheme::HTTP).iter()`
(`metadata.rs:479`, `metadata/protocol.rs:90`) walks one contiguous `BTreeMap`
range and yields the bare lowercase name and the stored value. None of this is
reachable from a transport, because there is no transport that speaks HTTP in
its own name.

**Rule to write (decision 45).** `Metadata` is not converted into the header
map — it **is** the header map. `holder/http/headers.rs` is only the list↔map
adapter and the disposition rule, and writes no parser and no validator.

Outbound is `headers.protocol(&Scheme::HTTP).iter()`: nothing allocated,
nothing re-validated, nothing lowercased at send time, and the snapshot's own
lexical order, so two identical requests put byte-identical headers on the
wire. A session's defaults are one `Metadata`
(`Arc<BTreeMap<String, String>>`), so cloning into a request is one atomic
increment, and `Metadata::new()` re-seats onto a process-wide shared empty map
(`metadata.rs:268-272`), so a request that adds no header of its own builds
**zero** header allocations. `overlay(request, session)` takes the **request**
as the receiver, because `Metadata::merge_with` resolves every clash in the
receiver's favour (`metadata.rs:451-459`) and a per-call header exists to beat
the session default; the argument order backwards silently keeps the stale side.

Inbound folds **once, before `Metadata::from_entries` is called even once**,
because that constructor refuses a duplicate canonical key with
`Error::DuplicateMetadataKey` rather than taking last-wins, and a response
legitimately carrying `Vary` twice must not fail construction. Three
dispositions:

- **Fold** — RFC 9110's `#list` join on `", "`. Everything not named below.
- **Singleton** — `content-length`, `content-location`, `content-range`,
  `content-type`, `etag`, `expires`, `last-modified`, `location`. Repeated with
  identical values it collapses silently; repeated with **disagreeing** values
  it is `Error::Conflict` naming the URL. That is a request-smuggling shape and
  never a merge and never a last-wins.
- **Never** — `proxy-authenticate`, `set-cookie`, `www-authenticate`. RFC 6265
  forbids folding `set-cookie`; the two challenge headers carry parameters a
  comma cannot be told apart from. These are kept out of the `Metadata`
  entirely and reachable only through `HttpResponse::set_cookies()` and
  `header_all`, so the snapshot never claims a value it cannot represent. The
  docs Contract table states the exception rather than letting a caller
  discover that `as_metadata()` omits cookies.

`header_all` answers the folded value when the snapshot holds it and every
occurrence from the side-channel otherwise, and **never splits a stored value
back apart** — splitting would be re-parsing past the boundary. A header value that is
not UTF-8 is unrepresentable: dropped under a `Fold` name, `Error::Conflict`
under a `Singleton` one. It is read with `str::from_utf8(value.as_bytes())`,
not `HeaderValue::to_str` as `client.rs:905-910` does — that call also refuses
every byte above `0x7e`, which the outbound validator accepts.

`Content-Type` and `Content-Encoding` are read by
`MediaType::from_content_headers` (`media_type.rs:160`) and nowhere else. The
reverse projection already exists at `HttpFieldMut::set_media_type`
(`types/protocol/http.rs:420`) and a write's headers are built by that rule
rather than a second one.

**Edges.**

- **The fold runs once at answer construction inside `send`, before the body
  reaches any caller or `HttpStream`** — never lazily behind `as_metadata()` or
  `header()`. ureq frames the body from the **first** `Content-Length`: the
  duplicate check in `ureq-proto-0.6.1/src/body.rs:333-341` sits inside one
  `HeaderMap::get` and can never trip, and `TooManyContentLengthHeaders` guards
  only the outbound request (`client/amended.rs:155-158`), so a lazy fold hands
  a caller who reads only bytes a body framed under a length a second header
  contradicts. A conflicting answer's body is dropped **undrained**, as the one
  exception to piece 8's drain-on-drop: draining reads exactly the first
  `Content-Length`, which is what drives ureq to `reuse()` the connection, and
  the injected remainder is already sitting unconsumed in `LazyBuffers.input`
  where ureq's reuse probe (`pool.rs:125-133` through `tcp.rs:242-258`, a
  non-blocking read of the raw socket) cannot see it — so the next request on
  that session parses the attacker's bytes as its own answer. `origin.rs` grows
  a `two_content_lengths_next` injection: the answer is `Error::Conflict` with
  zero bytes delivered, and `connection_count` rises on the following request.
- **An inbound key is always `http:`-prefixed, on reads and removals as much as
  on writes.** It is built with `crate::metadata::property_key(&Scheme::HTTP,
  name)` or one of the seventeen `HTTP_*_KEY` constants, and read with
  `headers.as_http().get(name)`, `get_property(&Scheme::HTTP, name)` or those
  same constants. Never the bare wire name and never
  `canonicalize_metadata_key`, which namespaces nothing — it acts only on a key
  that already carries an `http:`/`https:` prefix (`validation.rs:340-343`) and
  is `pub(super)` to `metadata` anyway. A bare name is silently accepted by
  `validate_entry`'s `_` arm as a top-level key, which costs twice: a bare
  `content-type` never matches `HTTP_CONTENT_TYPE_KEY`, so `media_type()` and
  piece 10's `http:content-location` read nothing off an answer that carried
  the header; and five reserved keys — `alias`, `comment`, `description`,
  `display`, `location` (`metadata.rs:24-27,:46`) — become writable from the
  wire, so `Location: https://evil/x` becomes the snapshot's own
  `Field::location()` while a relative `Location: /next` fails `Url::from_str`
  and takes the whole answer down. On the read side
  `canonical_http_lookup_key` returns a prefixless key untouched
  (`validation.rs:328-330`), so `get("etag")` and `remove("authorization")`
  match the literal keys and answer `None`: `None` is also the honest answer for
  a header the origin never sent, so nothing at the call site tells the two
  apart. The uncaught cost is piece 10's credential strip, where a bare
  `remove` is a no-op that carries a bearer token across an origin.
- **A value is `str::from_utf8(value.as_bytes())`, never
  `HeaderValue::to_str`.** `to_str` rejects every byte above 0x7e
  (`http-1.5.0/src/header/value.rs:243-256,:558-560`), so copying
  `client.rs:905-910` drops *valid UTF-8* values that
  `validate_http_header_value` (`validation.rs:391-404`) accepts on the way out
  — the module could send a UTF-8 `Content-Disposition` filename it can never
  read back. Only a value that is not UTF-8 is unrepresentable in `Metadata`'s
  `String`: under a **Fold** name it is dropped, under a **Singleton** it is
  `Error::Conflict` naming the field and the URL, because absent and
  unrepresentable are different facts — a dropped `etag` silently means no
  validator, no `If-Range`, and under `Relaxed` an undetectable mid-read splice;
  a dropped `content-length` means a probe the cost table says does not happen.
- **An answer carrying both `content-length` and `transfer-encoding` is
  `Error::Conflict` naming the URL**, raised in the fold in the same arm as two
  disagreeing `Content-Length` headers, never "chunked wins and the length is
  dropped". ureq frames the body from the chunked encoding and returns before it
  reads the parsed length (`ureq-proto-0.6.1/src/body.rs:351-356`) but leaves
  `content-length` in the map — stripping is gated on decompression, which this
  build never does (`ureq-3.4.0/src/run.rs:110-119`, `rust/Cargo.toml:113-115`)
  — so the value folds into the open scope as the resource's length, `size()`
  reports it, and every `pread` past it answers `Ok(0)` with zero requests:
  truncation with no request to observe. It is the response-splitting shape RFC
  9112 §6.3 says to handle as an error. Only the combination fails:
  `transfer-encoding: chunked` alone is an ordinary answer this module must
  accept, since the drain fix and the chunked accounting assertions depend on
  it, so piece 4's refused-name list is outbound-only and is not consulted here.
- **A served `Content-Encoding` is filtered per token before
  `MediaType::from_content_headers` sees it.** An `identity` token and an empty
  token are dropped — both mean "no coding" — and a header with nothing left is
  passed as `None`, never `Some("")`; `gzip, identity` is one coding, not a
  refusal. Without the filter a real origin's `Content-Encoding: identity`
  makes `from_content_headers` refuse (`mime_type.rs:408-413`, the all-empty
  case at `media_type.rs:177-183`) and turns a readable 200 into a parse error,
  taking the answer's `Content-Type` and charset with it. A pair the filter did
  not save still falls to the next rung — `Url::media_type()`, then
  `MimeType::FILE` — and is **never** absorbed by `.unwrap_or_default()`:
  `media_type()` returns a borrow with nowhere to report a failure, so the
  default is the tempting repair and the corrupting one —
  `MediaType::default()` is `application/octet-stream` (`media_type.rs:416-419`),
  the `OnceLock` never re-seats, and a `.parquet` URL answers
  `is_tabular() == false` at zero requests for the handle's life. Where the pair
  does have an error channel — `read_text()`, `into_holder()` — the refusal is
  surfaced. This is the same token classifier piece 7's ranged-answer conflict
  reads, so there is one and not two, and it is a token drop, not a parser.
- **`overlay` is a branch, not a call.** It probes both sides with
  `Metadata::protocol(&Scheme::HTTP).is_empty()` (`metadata/protocol.rs:85-87`,
  one range lookup, no allocation) and calls `Metadata::merge_with` **only when
  both sides hold an `http:` entry**; otherwise it clones the non-empty side's
  snapshot, which is an `Arc` bump. `merge_with` (`metadata.rs:451-459`) has no
  empty-side short-circuit — it collects both sides into a `BTreeMap<&str,
  &str>` and runs the result back through `from_entries`, which calls
  `validate_entry` on every entry — so calling it unconditionally costs two maps,
  2N string allocations and N re-canonicalizations per request for headers that
  were validated when the session was built, and breaks piece 13's "a request
  with N session headers and no request headers allocates 0 times for headers".
  When both sides do hold headers the request stays the receiver, unchanged.

**What moves.** `holder/http/headers.rs`, `holder/http/tests/headers.rs`,
`.api-inventory.txt`, `rust/tests/allocations.rs` (the zero-allocation pin).

## 7. `HttpSession`, `HttpRequest`, `HttpResponse`

**Today.** Nothing in the crate makes an HTTP request in its own name. Every
byte that leaves goes out as an S3, GCS or Azure operation.

**Rule to write (decision 46).** One reusable client, one call before it is
sent, one answer that is 2xx by construction.

`HttpSession` holds the transport (and therefore the pool), the base location,
the constant header snapshot, the default parameters, the auth spelling, the
redirect resolutions and the options. `Clone` is an `Arc` bump, it is
`Send + Sync`, and constructing one touches no network.
`HttpSession::request(method, target)` is the one owner; `get`, `head`, `post`,
`put`, `patch`, `delete` and `options_for` are one-line redirects. **Nothing
named after a method sends anything** — all seven return `HttpRequest<'_>`,
and the wire is touched only by `send`, `open`, `read_all_bytes`, `read_text`
or `read_scalar`. `get` collides with the verb table's `get*` = borrowed
lookup, and the collision is resolved by `AGENTS.md:388-390`: an HTTP method
name is foreign-protocol vocabulary and is recorded as such.
`options` is already an accessor, so `OPTIONS` is `options_for` — the one name
in this surface that is not the bare method, stated in the module doc rather
than left as a surprise.

A setter is fallible **exactly when the vocabulary underneath it refuses**.
Headers refuse, so `try_with_header`/`try_with_headers`/`try_with_base`/
`try_with_json`. `Parameters` encodes rather than refusing, so `with_params`
is infallible. That rule is mechanical; do not decide `try_` case by case.

`HttpBody` is wide (`From` for `&[u8]`, `Vec<u8>`, `&str`, `String`,
`&Scalar`, `&dyn IOBase`, `Option<HttpBody>`); `Payload` is typed and has two
states, because the only question downstream is replayable-or-not.

There is **no `raise_for_status`**. `send` maps every non-2xx at the boundary,
so a caller holding an `HttpResponse` holds a 2xx by construction and never
checks. The status is not lost — it is a field on the error — and
`Error::is_absent()` answers the one branch most callers want. The
404-as-emptiness rule and the redirect statuses live *below* this boundary, in
`file.rs` and `redirect.rs`, where a status is still a value.

`HttpResponse` redirects into facts the crate already owns rather than
restating them: `read_text()` through `Charset`/`MediaType`, `read_scalar()`
through `crate::text::from_io`, `pstream_bytes` through `ByteStream`,
`headers()` through `ProtocolMetadata`, `into_holder()` so a content coding
decodes through `crate::coding` and a record encoding reads through `IOMedia`
with nothing re-inferred.

What goes out, and why:

- **`Range`** only when the window is not the whole value from zero, rendered
  into a 48-byte stack buffer with manual digit emission — no `format!`, no
  `String`. `offset == 0` with no bound emits **no header at all**.
- **`Accept-Encoding: identity` on every ranged request, always.** `Range`
  addresses the selected representation and a coded one has no stable byte
  offsets; `Codec::has_restarts()` (`codec.rs:168`) is false for gzip and
  zlib. A ranged answer that nonetheless carries a non-identity
  `Content-Encoding` is `Error::conflict`. This is the reasoning
  `rust/Cargo.toml:113-115` already records for keeping ureq's compression
  off, extended to the header the client does control.
- **`Accept-Encoding: <codings>` on whole-value reads only**, derived and
  never written down: `Codec::ALL` through `crate::iobase::coding_mime`
  (`iobase/lifecycle.rs:60-67`) chained through `MimeType::content_coding()`
  (`mime_type.rs:601`). Today `"zstd, gzip, deflate"`. **Never
  `Codec::as_str()`**, which emits the unregistered `"zlib"` and the forbidden
  `"identity"`. `with_accept_encoding` **refuses `br` and `compress`**: both
  parse cleanly as encodings but `Codec::from_mime_type` falls through to
  `Codec::Identity` for them, so a brotli body would be handed back undecoded
  and *labelled decoded*. The setter catches what goes out; an answered coding
  this build cannot decode is refused where the body is handed over, because an
  origin may serve a coding no request asked for.
- **`Accept`** only when named, from `MimeType::as_str()` on concrete
  constants — never from `MediaType`'s `Display`, which emits the crate's own
  `;encodings=a,b` form and is not a legal `Accept` value.
- **`If-Match`** on every publishing write from a handle that knows a
  validator, so a lost update is a 412 → `Error::Conflict` instead of a silent
  overwrite; **`If-None-Match: *`** on a create the metadata says is
  known-absent. Both cost zero extra requests — a header on a request already
  made.
- **`If-None-Match`** on a re-read inside an open scope that already holds
  bytes; a 304 transfers no body and increments `stats.not_modified`. Not sent
  when there is nothing to serve from, because a 304 with nothing behind it
  wastes the round trip it was meant to save.
- **`Vary`** is recorded, and when it names a header the request varies on the
  open scope is invalidated rather than reused.
- **`User-Agent`** defaults to `concat!("yggdryl/", env!("CARGO_PKG_VERSION"))`
  and is the one header `with_user_agent` replaces rather than merges.

**Edges.**

- **A setter guards what goes out; a server answers what it likes.** A 2xx that
  is not a ranged answer and whose `Content-Encoding` names a coding this build
  cannot decode is `Error::unsupported("decoding this content coding", token)`,
  refused where the body is handed over. `br` and `compress` are the whole set:
  `MimeType::from_content_coding` (`mime_type.rs:396-410`) already refuses every
  token it cannot name, and `Codec::from_mime_type` (`codec.rs:231-241`) maps
  only those two to `Identity`. The check runs over **every** coding in the
  list, not the outermost, because `Codec::from_media_type` reads only `.last()`
  (`codec.rs:247-251`), so `gzip, br` would otherwise pass. Let one through and
  the caller holds coded bytes on a handle whose `IOBase::codec()` answers
  `Identity` and over which `Holder::into_media_as` composes no decoder, so
  `read_text`, `read_scalar` and `into_holder` all read brotli as plain — the
  exact "undecoded and labelled decoded" failure `with_accept_encoding` exists
  to prevent, arriving by the one door that setter cannot guard, since ureq
  decodes nothing here and only `debug!`s an unknown coding
  (`ureq-3.4.0/src/body/mod.rs:894-901`). On a **ranged** answer the conflict is
  decided on a remaining non-`identity` token after piece 6's filter — never on
  the header's presence and never on a `from_content_headers` error, both of
  which refuse the one value that is safe. A 3xx, an error answer and a body
  that cannot exist — `HEAD`, `304`, zero length — are not checked.
- **Exactly one line per field name leaves this module.** Each generated header
  — `Range`, `If-Range`, `Accept-Encoding`, `User-Agent`, `Authorization`,
  `If-Match`, `If-None-Match`, `Accept` — is emitted at the
  `ureq::http::Request::builder()` call site only after the overlaid `Metadata`
  has been consulted for that name, with two dispositions and no third. Where
  the module's value **is** the operation — `range` and `if-range` on a `pread`,
  a resume or a ranged read, `accept-encoding: identity` on any ranged request,
  and the `if-match`/`if-none-match` a handle's validator generates — a caller
  entry of that name is `Error::conflict` naming both values, because it is
  disagreeing input about what is being read or written and picking one silently
  returns a window nobody asked for under a 206 that looks valid. Everywhere
  else — `authorization`, `user-agent`, `accept`, `accept-encoding` on a
  whole-value read — the caller's entry wins and the generated header is **not
  emitted at all**, the direction `overlay(request, session)` and
  `with_user_agent` already resolve in. `HttpAuth` is resolved into `Prepared`'s
  header map when `Prepared` is built, as one `authorization` entry, **before**
  piece 10's cross-origin drop — never onto the ureq builder at send time, which
  puts a second `authorization` line beside the caller's (`Builder::header`
  appends; ureq-proto counts only duplicate `host` and `content-length`) and
  regenerates the credential after the drop ran, undoing it on every hop. A
  generated header is never written into the `Metadata`: `insert` calls
  `Arc::make_mut` (`metadata.rs:544,:555`) and would clone the whole map per
  request.
- **A window with no upper bound is the remainder, not a window ending at
  `u64::MAX`.** `length == usize::MAX`, or `offset + length` past `u64::MAX`,
  emits `bytes={offset}-` and is carried as `last: None` through `HttpStream`'s
  resume. `offset.saturating_add(length - 1)` (`client.rs:1259`,
  `file.rs:551`) is the shape not to copy: it puts a last-byte-pos past
  `i64::MAX` on the wire, which nginx answers `416` to (it parses the value into
  `off_t` and refuses the overflow), and a `416` is `Ok(0)` — so every
  `read_to_end` from a non-zero offset, which is exactly
  `read_range_bytes(offset, usize::MAX)` (`iobase/bytes.rs:368-377`), comes back
  empty with no error. `client.rs:1408` already spells the open-ended form; that
  is the one to reuse. A saturated bound also disarms piece 8's rule 4, so a
  body that dies at true EOF re-opens past the end and the 416 becomes a
  conflict.
- **A `304` is served only from bytes the open scope already holds, and never
  becomes an empty read.** ureq frames it `NoBody` by construction
  (`ureq-proto-0.6.1/src/body.rs:495-505`), does not count it a redirect
  (`client/mod.rs:311-318`) and raises nothing under
  `http_status_as_error(false)`, so it arrives as an ordinary answer whose body
  is zero bytes. It is a hit only when its `ETag` weak-matches the validator
  that conditioned it, or it carries none — weak comparison is `If-None-Match`'s
  (RFC 9110 §13.1.2), so `W/"v1"` and `"v1"` are one tag. A different tag means
  the origin's current representation is not the one the scope holds: the
  scope's bytes and validator are dropped, one **unconditional** re-read
  replaces them, and `stats.not_modified` is **not** incremented — counting it
  would report a hit on bytes just proven stale. A `304` answering a request
  that went out carrying neither `If-None-Match` nor `If-Modified-Since` — the
  test is the request as sent, so a caller's own conditional counts — and a
  `304` with nothing in the scope to serve from are both `Error::conflict`
  naming the URL, never `Ok(0)` and never an empty `Vec`. That is what bounds
  the re-read at one.
- **`Vary: *` invalidates the open scope unconditionally, with no comparison.**
  Nothing that answer cached — size, validator, media type, `Accept-Ranges` — is
  reused, because `*` says the selection turns on something the request does not
  name. Any other value is read as the `#list` piece 6 folded it into: split on
  `,`, trimmed, compared ASCII-case-insensitively against the names this request
  actually sent, since the origin spells the value and piece 4 lowercases only
  what leaves. That is one read of one stored value, not the occurrence
  reconstruction `header_all` refuses. Comparing field names alone never matches
  `*`, so the scope survives the one answer that forbids reuse and a later
  `pread` past a stale cached length returns `Ok(0)` with zero requests; and
  this module's own requests do vary in a header origins vary on, since
  `Accept-Encoding` differs between a ranged and a whole read.
- **The boundary normalizes exactly two families, and only because the target
  variants carry no status.** `404 | 410` → `Error::absent`, `409 | 412` →
  `Error::conflict` — the closed match `Client::failure` already makes
  (`client.rs:1067-1069`), plus 410. Every other status stays
  `Error::remote("http", operation, status, code, message, url)` with the number
  intact. **`405`, `416`, `403` and `501` are never widened into
  `Error::Unsupported`**: that variant means this backend declined a capability
  (`error.rs:111-116`), not that this origin refused this method, and `pread`'s
  `416 → Ok(0)`, `remove`'s `405` and the resume's `416` each have to read the
  number to act. "Every non-2xx" also overstates the boundary in two places the
  design depends on: a 3xx is a value `redirect.rs` reads, and the `304`
  answering `If-None-Match` is a hit. `Error::is_absent()` and
  `Error::is_conflict()` (`error.rs:417,:430`) are the two branches where the
  status genuinely is lost; losing it anywhere else costs a caller the only fact
  the origin gave it.

**What moves.** `holder/http/{mod,session,request,response,body,auth,options,
properties,location}.rs`, `holder/http/tests/{mod,origin}.rs`,
`rust/Cargo.toml` (`http = ["wire"]`), `holder/mod.rs`
(`#[cfg(feature = "http")] pub mod http;`), `.api-inventory.txt`.

## 8. A stream resumes; it never restarts, and a cut is never end of stream

**Today.** `object`'s `Resuming` (`client.rs`, the `:2414-2596` block) re-opens a
cut transfer but sends **no** conditional: `grep -rn 'if-range' rust/src/`
returns nothing. `open_reader_range` maps `404 | 416` to `std::io::empty()`
(`client.rs:1411-1413`). So an object rewritten mid-read silently splices two
generations, and one deleted mid-transfer yields a clean EOF with truncated
data and no error. `Pooled::drop` returns immediately when
`left > POOLED_DRAIN_LIMIT`, and `left` is `u64::MAX` whenever the origin
stated no `Content-Length` — so **every abandoned chunked body burns its
connection**, and several core consumers abandon deliberately
(`hashing/xxhash/stream.rs:50-52`, `media/iceberg/manifest.rs:726-730`,
`media/ipc/mod.rs:196-206`).

**Rule to write (decision 47).** `HttpStream` is a plain `std::io::Read` and
nothing else. It does not chunk, does not implement `Iterator`, and caches
nothing: `ByteStream` already owns batching, the never-empty rule, the
short-final rule, the allocation guard and the fuse. What `HttpStream` owns is
the two things only the transport can do — resume, and give the connection
back.

Once `into_stream` hands the caller the body, the exchange is settled and
`send` retries nothing further. On a resumable read error `HttpStream` repairs
**in place**: it re-opens at `start + delivered` with `bytes={from}-{last}` or
`bytes={from}-`, so `ByteStream` never sees the cut and its fuse is never
tripped. `is_resumable` is the verified set `ConnectionReset |
ConnectionAborted | BrokenPipe | UnexpectedEof | TimedOut | Interrupted |
Other` (`client.rs:2546-2561`); the generic kind is included deliberately,
because mistaking a severed connection for a decoding failure costs the whole
transfer where mistaking it the other way costs one bounded re-open.

Six properties, each load-bearing and each pinned:

1. The re-seek is a **new ranged request from `start + delivered`**, never a
   re-read-and-discard. Building it as a skip over a fresh from-zero request
   would re-transfer the whole prefix and, worse, silently truncate:
   `bytestream.rs:343-346` sets `remaining = 0` and returns `Ok(0)` forever if
   the source ends during the skip.
2. The failure counter resets on **any** byte arriving, so the budget is on
   *consecutive* failures. A flaky link survives unbounded interruptions; a
   link that accepts and immediately drops stops after `max_attempts - 1`.
3. A cut connection **never** surfaces as `Ok(0)`. `ByteStream::read_filled`
   reads that as permanent end of stream and sets `done`
   (`bytestream.rs:107-110`).
4. A fully-delivered bounded window whose body then dies returns `Ok(0)`:
   everything asked for arrived. Dropping this makes a satisfied range
   re-request a zero-length window and surface a 416 as a failure.
5. A failed re-open surfaces the **original** error, not the re-open's.
6. A resume is **not** a retry: it goes out at `attempt = 1`, increments
   `requests`, `gets` and `resumes`, and spends no budget token. Its own
   exchange may still hit a 503 and spend from budget one, so the worst case
   for one drain in which no byte arrives is `max_attempts + (max_attempts − 1)
   × max_attempts = 9` requests — the initial open is an ordinary `send` and
   runs its own full ladder — and `(max_redirects + 1) × max_attempts` = 18 for
   one exchange that hops, 24 for a drain whose open does. **Those numbers are
   in the module doc and in assertions**, on a drain where no byte arrives
   between cuts; with progress the re-open count is deliberately unbounded and
   the module doc says so. The object backend leaves all of it unstated.

`If-Range` is the gap this closes, and this module refuses to port the object
backend's silence. The first answer's `ETag` — or, absent one, its
`Last-Modified` — is the stream's validator. Every resume sends
`If-Range: <validator>` beside the `Range`. **206** means it held. **200**
means it did not and the origin is sending the whole changed resource from
byte zero: that is the unreadable-answer spelling — `Error::remote("http",
"resume", 200, "EntityChanged", <the validator asked for and the one that
came back>, mask_uri(url))` — surfaced through `Read::read` as
`std::io::Error::other`, not a skip-and-continue and not silence.
`Error::conflict` cannot carry it: both its nouns are `&'static str` and its
`Display` says "expected to create" (`error.rs:347`, `:231-237`), so the two
validators — the facts that make it a conflict — would be dropped while the
test still passed. **404, 410
or 416 on a resume** is the same conflict (410/404 as `Error::absent`). This is
the sharpest departure: on an **initial** open those statuses still read as
emptiness, because absence is emptiness per the laziness contract; on a
**resume** they mean the resource vanished under a transfer that already
delivered bytes, and reporting that as a clean end hands the caller a truncated
value it cannot detect. A **weak** validator cannot back `If-Range`, so it is
treated as absent: a `Range` with no conditional is honoured unconditionally
and the answer is a 206, so the 200-is-a-conflict rule guards nothing here and
the module doc must not claim it does. With **no**
validator at all, `Consistency::Strict` makes a cut a failure rather than an
unguarded re-open; `Relaxed` resumes unconditional and the module doc states
that a mid-read rewrite is then undetectable.

`HttpStream::drop` drains up to `POOLED_DRAIN_LIMIT` (1 MiB) so an abandoned
body returns its connection, and — the one fix over `Pooled` — drains up to
`DRAIN_LIMIT` (64 KiB) when the origin stated no length, instead of giving up.
A chunked body of unknown size is usually short. The drain stays bounded; an
unbounded drain would be worse than burning the connection.

**Edges.**

- **A resume re-opens only what a `GET` produced, against a pinned target.** The
  resume plan — the absolute URL the body arrived from after any hops in the
  opening exchange, the exact header set that hop went out with (already
  stripped by `Prepared::redirected`), the validator and the window — is decided
  once at `into_stream` and held as `Option<Resume>`: `Some` for `GET`, `None`
  for every other method, never re-derived and never a stored method whose only
  legal value is `Get`. With no plan a cut body surfaces the transport error
  through `Read::read` and is never re-opened. Both re-openings an implementer
  reaches for are wrong and both are silent: re-sending the `POST` or `PATCH`
  duplicates the effect — a second order, a second charge, a second append —
  past piece 9's gate, which lives in `Prepared`/`send` and is not consulted
  once the body is handed over, with no token spent and no counter showing it;
  and re-opening the same URL as a `GET` splices a different value, because the
  `GET` representation of a `POST` target is not the body that was cut.
  Rebuilding the request from the original `HttpRequest` re-attaches the
  `authorization`/`cookie` the origin crossing dropped, leaking the credential
  on every resume; re-reading the session's resolution map lets a sibling's
  permanent redirect — or point 4's validator drop — move the transfer under a
  body already in flight, and an origin that honours `Range` while ignoring the
  unrecognized `If-Range` answers 206, splicing two generations with no error.
  Piece 10 point 6's "the resolved URL" means this pinned target.
- **`HttpStream` is this module's only body reader.** `pread`,
  `read_range_bytes`, `read_all_bytes` and `read_range_digest` read through it,
  never off the `WireAnswer` — a short bounded exchange is not a different kind
  of transfer — so a cut inside any of them resumes in place and property 4
  makes a fully-delivered bounded window whose body then dies `Ok(0)` rather
  than a failure. Building a second raw reader for "the short path" is
  `object`'s shape (`open_range` raw at `client.rs:1252`, `Resuming` attached
  only at `:1373`) and puts every resume behind `pstream_bytes`, the one method
  `Buffered` delegates untouched (`buffered/mod.rs:503`): its `read_all_bytes`,
  `read_range_bytes`, `pread_exact` and `pread` all funnel into the inner
  `pread` through `fetch_run` (`:404-411`), where an `Err` propagates with no
  repair. Under the recommended pairing a cut inside a coalesced page run then
  fails the whole read, and piece 8's six pinned properties test a path that
  composition never takes. It also saves a second bounded drain: a
  buffer-filling path that stops before the body is exhausted would need its own
  copy of what `HttpStream::drop` owns, or it burns the connection "1000
  sequential ranged reads are 1 TCP connection" asserts.
- **A validator is what the origin can be held to, and nothing else is sent.** A
  **weak** `ETag` is treated as absent — never sent, never stripped to look
  strong, which would assert on the wire a validator the origin never issued.
  RFC 9110 §13.1.5 forbids a weak entity tag in `If-Range`, and a `Range` with
  no conditional is honoured unconditionally, so the answer is **206** and the
  200-is-a-conflict branch can never fire: that branch is not a guard for a
  request that omits `If-Range`, and the module doc must not claim it is. A
  `Last-Modified` backs `If-Range` only when the same answer also carries a
  `Date` that parses and is at least one second after it — RFC 9110 §8.8.2.2's
  own test, and the only strength a client can deduce, since both values came
  off one clock in one answer; `date` is read off the same folded snapshot
  through `validate_entry`'s `_` arm, compared with the one HTTP-date conversion
  `mtime` owns, no second parser and no extra request. No `Date`, an unparsable
  one, or one inside the same second means the stream has **no** validator and
  the no-validator rule applies. `Consistency::Strict` is the **default**: with
  no validator a cut is a failure, never an unguarded re-open, and `Relaxed` is
  asked for by name with its own rustdoc stating that a mid-read rewrite is then
  undetectable. Get either wrong and a resource rewritten mid-transfer splices
  two generations on exactly the origins this piece exists to protect against —
  CDNs and compressing proxies, which serve weak tags, and a resource rewritten
  inside the second its `Last-Modified` names, which keeps that exact string.
  The origin answers a weak `ETag`, and a `drop_validator` answer carrying
  neither field, or `Strict` is unassertable.
- **End of stream and a changed resource are told apart by what the answer
  states.** The satisfied-window check reads the window the **answer**
  described, not only the caller's bound: `HttpStream` records the last byte the
  first answer's `Content-Range` names, and the effective last is that or the
  caller's bound when it is smaller. On a window with no upper bound, a resume
  issued at `from == start + delivered` and answered **416** is `Ok(0)` if and
  only if its `Content-Range: bytes */N` has `N == from`; a 416 whose `N`
  disagrees, a `bytes */*`, a 416 with no `Content-Range`, and every 404/410 on
  a resume stay `Error::Conflict` — the harvested total is the only thing that
  distinguishes a finished transfer from a rewritten resource, and `If-Range`
  is what makes the arm safe to trust. This is not theoretical: a chunked body
  whose terminating `0\r\n` never arrives, and a `RecvBody` timeout after the
  last data chunk, both surface as resumable errors at exactly
  `delivered == total`, and `read_range_bytes(offset, usize::MAX)` bounds the
  window past the end, so `from > last` never fires there either. A handle that
  already recorded `accepts_ranges: false` attempts no resume at all: a cut body
  is `Error::unsupported("resuming a transfer from an origin that ignores
  Range", url)`, never a re-open and never a client-side skip. When a resume is
  attempted and the answer is `200`, the answer's own validator decides which
  cause it is — a different validator or none is the changed resource, an equal
  one means `Range` was ignored and takes the `unsupported` refusal above.
  Reporting "a changed resource" for an origin that never served one costs a
  request per cut and sends whoever reads the error hunting a consistency bug
  that does not exist. Property 5 is narrowed with it: a re-open that **reached
  the origin** surfaces that answer's error, and only a re-open that failed in
  transport falls back to the original — otherwise every conflict rule above is
  unreachable and a resource replaced mid-transfer is reported as
  `ConnectionReset`.
- **A read failure is classified by downcasting, not by `ErrorKind` alone.**
  `error.get_ref().and_then(|e| e.downcast_ref::<ureq::Error>())`, in
  `holder/wire/` beside `is_resumable`, so `holder/http` still names no `ureq::`
  type. `BodyHandler::read` sends every non-`Io` ureq error through `into_io()`,
  which is `io::Error::other` (`ureq-3.4.0/src/run.rs:768-770`,
  `src/error.rs:198-204`), so `ErrorKind::Other` is the only kind a mid-body
  `timeout_recv_body` ever has — `TimedOut` never fires on this transport and is
  dead weight in the set, `Other` is what makes a timeout resumable, and
  deleting it as "too broad" silently disables resume for every timeout.
  `Timeout` resumes; `Protocol` (ureq-proto's chunk-framing errors) and
  `BodyExceedsLimit` do **not**, both being permanent — and a resume hands the
  re-opened body a *fresh* limit, so applying `MAX_DOCUMENT` to the streamed
  path would silently defeat it and deliver a 10 GiB resource in 32 MiB
  installments. `Interrupted` leaves the set entirely: `std::io::Read` defines
  it as "call `read` again", ureq preserves the kind and leaves the handler
  usable (`run.rs:678-686`), so it is retried **in place** — no pause, no
  re-open, no increment — and the crate already reads it that way at
  `bytestream.rs:255` and `media/ipc/mod.rs:433`. `failures` resets only when at
  least `DEFAULT_STREAM_BATCH_SIZE` (`iobase.rs:48`, 64 KiB) has been delivered
  since the previous re-open, because ureq's body timeout is an absolute
  deadline anchored at the `RecvResponse` mark (`src/timings.rs:55,:153-170`)
  and every re-open restarts it: without the floor a trickling origin resets on
  the first byte of every segment and drives re-opens without bound inside one
  uncancellable `Read::read` — at 1 B/s a 64 MiB read is ~5×10⁵ requests and
  never an error. The origin needs `trickle_next(bytes_per_tick)` beside
  `cut_next_body(after)`; `holder::object` inherits the narrowed set and
  `object/tests/{accounting,protocol}.rs` must still pass unchanged.
- **A destructor opens no request and holds no clock it does not own.**
  `HttpStream::drop` drains `self.reader` directly, never through
  `HttpStream::read`: draining through the resume wrapper swallows the error
  `drain_upto` stops on (`client.rs:2352`), so a cut mid-drain issues up to
  `max_attempts − 1` fresh `GET`s, each re-entering the full retry ladder with
  its jittered pause, from a `Drop` that cannot fail or be cancelled. The drain
  is bounded in wall clock as well as in bytes: `holder/wire/body.rs` checks its
  own `DRAIN_DEADLINE` (1 s, the order of a connect plus a handshake — the whole
  of what draining buys) between reads. ureq's `timeout_recv_body` cannot serve
  as that deadline — `run.rs:674` re-anchors it at `now` on every await, a spent
  budget saturates to zero and `NextTimeout::not_zero` (`timings.rs:198`) turns
  that into one second, and it is applied as a per-syscall `SO_RCVTIMEO`
  (`tcp.rs:184`) — so a server yielding one byte before each expiry never times
  out and a 1 MiB drain becomes ~1 Mi reads in a destructor. And the drain pays
  only at EOF: ureq returns a connection to the pool only from
  `BodyHandler::ended()` (`run.rs:711-745`), and `reuse` refuses a connection
  whose probe reads a byte (`pool.rs:125-133`), so a drain that stops at
  `DRAIN_LIMIT` without reaching the end saves nothing and the bytes are pulled
  off the wire for nothing. The known-length arm never starts a doomed drain —
  `left > POOLED_DRAIN_LIMIT` is checked before the first read (`client.rs:2320`)
  — and the unknown-length arm cannot, so the claim is written conditionally
  wherever it appears: an abandoned chunked body returns its connection **when
  its remainder fits in `DRAIN_LIMIT`**, and past that burns it exactly as
  `Pooled` does today. The accounting test asserts both arms by name, or the
  fixture's short bodies certify a claim nothing tested where it is false.

**What moves.** `holder/http/stream.rs`, `holder/http/validator.rs`,
`holder/wire/body.rs` (the `Pooled` drain fix), `holder/http/tests/resume.rs`,
`rust/benchmarks/holder/http/resume.rs`.

## 9. A transport failure does not replay a non-idempotent method

**Today.** `send` (`client.rs:740-782`) retries `PUT`, `POST` and `DELETE` on a
transport failure and on 5xx exactly as it retries `GET`. There is no method
check anywhere. Against a store whose API is known that is defensible; against
an arbitrary origin a re-sent `POST` duplicates an effect.

**Rule to write (decision 48).** The budget itself is unchanged and moves
verbatim: `RETRY_TOKENS = 500`, `RETRY_COST = 5`, `RETRY_REFUND = 1`,
`RETRY_BACKOFF = 50ms`, `RETRY_BACKOFF_CAP = 20s`, `RETRY_AFTER_CAP = 30s`;
`may_retry` is an attempt cap **and** a token bucket, both; `withdraw` is a
`fetch_update` CAS succeeding only at `held >= RETRY_COST`; `settle` refuses to
refund for `status >= 500 || status == 429`, so a healthy client never drains
and a 5xx storm is bounded in total rather than per-caller. The pause is
`Retry-After` when the origin sent bare integer seconds within the cap —
obeyed verbatim and without jitter, because it is an instruction — and
otherwise a uniform draw from `[0, backoff(attempt)]`. Full jitter, not the
window itself: doubling alone puts every client that failed at the same instant
back on the wire at the same instant. The draw stays
`xxh3(counter.fetch_add(1, Relaxed))` over a per-transport `AtomicU64` — no
`rand` crate, no global RNG, and a sequence a test can predict.

What changes is the trigger. Three, and no others: `is_retryable_transport`
(`Io | Timeout | ConnectionFailed | HostNotFound`, with **no wildcard arm**, so
a new ureq 3.x variant defaults to not-retryable), `status == 429`, and
`status >= 500`. No other 4xx. Then:

- `429` and `503` retry for every method, because they mean the request was
  refused before it was applied; `500`, `502` and `504` retry idempotent
  methods only, because an intermediary's report about an unreadable upstream
  exchange, and a 500 after a committed write, both say nothing about whether
  the origin ran the request;
- a **transport** trigger retries `GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS`;
- for `POST` and `PATCH` a transport trigger retries **only** on
  `ConnectionFailed` and `HostNotFound`, where nothing reached the origin.

The gate lives in `holder::http`'s `Prepared`, **never in `budget.rs`**, or it
changes how `object` retries a `PUT`. `holder::object` sets
`WireRequest::retry_regardless = true` and keeps its behaviour and its
accounting assertions byte-for-byte. A `Payload::Streamed` is not replayable,
so its attempt count is forced to 1 and `may_retry` is never consulted.

**Edges.**

- **A status trigger is not one trigger, and the method gate applies to it too.**
  `429` and `503` mean the request was refused before it was applied — the case
  `Retry-After` exists for — so they retry every method. Every other retryable
  status retries idempotent methods only: a `502` or `504` is an intermediary's
  report about an upstream exchange whose answer it could not read, and a `500`
  can follow a write the store already committed, so none of them says the
  origin did nothing. Replaying a `POST` or `PATCH` on one duplicates the effect
  with the design's own blessing, and the transport gate never sees it, because
  that gate is keyed on `is_retryable_transport`. `holder::object` is unchanged:
  `WireRequest::retry_regardless = true` bypasses the status gate as well as the
  transport one, or `object/tests/accounting.rs` stops passing unmodified.
- **`Retry-After: 0` must not remove the pause.** `delay-seconds` is `1*DIGIT`
  (RFC 9110 §10.2.3), so `"0"` parses, `asked <= RETRY_AFTER_CAP` passes
  (`client.rs:2577-2582`) and `pause` sleeps `Duration::ZERO`
  (`:691-703`): all three attempts then leave back-to-back at an origin that
  just said it is overloaded, which is the herd the jitter exists to prevent,
  and every one costs `RETRY_COST` with no interval in which it could have
  started succeeding, because `settle` never refunds a 429 (`:723-731`). One
  host answering `Retry-After: 0` empties a 500-token budget in about a hundred
  round trips. The pause is therefore `max(asked, draw)`: the instruction may
  lengthen the jittered pause, never remove it, and `RETRY_AFTER_CAP` still
  bounds the other end. At defaults the draw is at most 100 ms and the smallest
  non-zero `Retry-After` is one second, so the floor fires only at zero — one
  `max` in `wire/budget.rs`, changing no request count in either backend.

**What moves.** `holder/wire/budget.rs`, `holder/wire/send.rs`,
`holder/http/request.rs` (`Prepared`), `holder/object/client.rs` (the
`retry_regardless` call sites), `holder/http/tests/accounting.rs`.

## 10. A redirect is followed, and the handle's URL does not move

**Today.** `AGENTS.md:621` says "a 3xx is never followed", and `build_agent`
enforces it with `.max_redirects(0).max_redirects_will_error(false)`
(`client.rs:2398-2400`). The sentence sits in the object-store section and
reasons from S3's signing-region correction and Google's reuse of 308 for a
chunk that landed.

**Rule to write (decision 49).** Neither of those reasons applies to a general
origin, so this module follows redirects, and the decision **argues with that
sentence** rather than ignoring it. The agent stays at `max_redirects(0)`: ureq
still follows nothing, and the loop is made one level up, on `HttpSession`,
where the session can record what it learns.

`Redirect::of(status)` is `301 | 308 => Permanent`, `302 | 303 | 307 =>
Temporary`, anything else `Unusable`. `303` is the one status that rewrites the
request — anything but `HEAD` becomes `GET` and the body is dropped. `301` and
`302` preserve for `GET` and `HEAD` and become `GET` otherwise, which is what
every deployed origin expects. `307` and `308` preserve both. `target` resolves
per RFC 3986 and **clears the base's query** for a relative location:
`Url::joinpath` (`uri/mod.rs:640`) preserves a query, which is right for a
signed object location and wrong here.

Bounded by `max_redirects()` (default 5). Each hop increments `redirects` and
`requests`, spends **no** retry token, and re-runs the whole retry ladder at
the new location. Exceeding the cap, or returning to a URL already visited in
this operation, is `Error::remote(… "TooManyRedirects" …)`. A `Payload::Streamed`
that a 307/308 would have to re-send is refused with `Error::unsupported`
rather than sent empty. **Crossing an origin** — scheme, host, or effective
port, where the effective port is
`url.authority().port().or_else(|| url.default_port())` (`scheme.rs:186-194`
gives 80/443) — **drops** `authorization`, `cookie` and `proxy-authorization`
before the next request; `Prepared::redirected` is the one place that happens.
Following `https` → `http` is `Error::conflict` unless the session allowed it.

The "IOBase redirections" half is a rebinding, not a renaming:

1. **`IOBase::url()` never changes.** It is the caller's name: what
   `Error::absent`/`conflict` report, what a `Holder` repr renders, what
   `copy_into`'s staging is named after. A location that drifted under a
   *temporary* redirect would make every diagnostic lie.
2. A `Permanent` redirect is recorded in the **session's** map. Every later
   request on that session — from this handle *and from every sibling handle
   under the same moved prefix* — starts at the resolved location. That is the
   whole cost claim: a thousand-file scan behind a moved prefix costs one extra
   request **for the session**, not one per file. The map is bounded at
   `MAX_RESOLUTIONS = 1024`, least recently used dropped, behind an `RwLock`
   read on the fast path; losing an entry costs one request, never correctness.
3. `HttpFile::resolved_url()` answers the current binding, `http:content-location`
   carries it (the key already exists, `metadata.rs:35`, already typed at
   `types/protocol/http.rs:74`), and `HttpResponse::history()` shows the chain
   with credentials stripped.
4. Recording a resolution **drops any cached validator**: an `ETag` from the
   old location is not a validator for the new one, and carrying it across a
   move is exactly how two bodies get spliced.
5. A `Temporary` redirect records nothing and rebinds nothing.
6. **A redirect mid-stream is never followed.** Once `HttpStream` owns the body
   the status is settled; a resume's re-open goes to the URL the stream pinned
   when its body arrived — not the map's current answer — and a 3xx there is a
   failure, not another hop. A redirect between range requests would
   silently change what is being read.
7. `with_max_redirects(0)` restores the object backend's behaviour exactly.

`object`'s one-shot region redirect stays in `object`: it is not a `Location`
to follow, it corrects a signing region, and `redirect.rs` knows nothing about it.

**Edges.**

- **A `Location` is classified syntactically, before any parse, because the
  crate's own parser cannot be the classifier.** `Uri::from_str` with no usable
  scheme falls to `Uri::from_path` (`uri/parser.rs:195-203`), so `/x` parses as
  `file:///x` and `//evil.example/x` is read as a UNC path and parses as
  `file://evil.example/x` keeping the attacker's authority
  (`uri/mod.rs:148-160`), and `Url::from_uri` exempts `file:` from its host
  check (`uri/url.rs:34-46`) — so `Url::from_str(location).or_else(|_|
  base.joinpath(location))` never reaches its fallback for the two commonest
  relative forms, while surviving a casual test because a bare `x` does fall
  through. Three forms: a valid `Scheme` token before the first `:` with no `/`,
  `?` or `#` before it is absolute and is used as written; a leading `//` is a
  network-path reference resolved by prefixing the base's scheme and re-parsing
  whole — it must never reach `joinpath`, which takes the leading-`/` branch and
  drops the empty segment (`uri/path.rs:362-363,:474`), turning
  `//cdn.example/x` into the path `/cdn.example/x` on the base's own host, where
  the crossing check sees no change and `pread`'s `404 → Ok(0)` reports the miss
  as an empty file; anything else is relative. A relative reference is **split
  before `joinpath` sees it**, in the parser's own order
  (`uri/parser.rs:210-226`): the fragment at the first `#`, then the query at
  the first `?` of what remains — first-`?`-first misparses `../x#a?b`, since a
  fragment admits `?`. Only the path part reaches `Url::joinpath`, whose
  `is_path_byte` admits neither character, so handing it the whole reference is
  a `uri path` parse error and **every** relative redirect carrying a query
  fails. The target's query is the reference's own, cleared when it carries
  none; its fragment is the reference's, or the base's when it has none (RFC
  9110 §10.2.2), carried as a formed component through `Uri::from_parts` and
  never through `set_fragment`, which percent-encodes `%` unconditionally and
  would rewrite `#a%20b` as `a%2520b` in `resolved_url()`. A 3xx classified
  `Permanent` or `Temporary` whose `Location` is absent, empty or unparseable is
  `Error::remote` naming the status and the header as received — never
  resolved, never followed, never recorded: ureq's own `NoLocationHeader` is
  unreachable at `max_redirects(0)` (`run.rs:224-235`), and
  `Uri::joinpath("")` returns the base path, so the empty one follows a hop to
  itself and records a self-resolution that rebinds every sibling handle under
  the prefix. The resolved target is `http` or `https` (`Scheme::is_http()`) or
  the hop is `Error::conflict` naming the scheme; and a `Location` whose
  authority carries user information is never followed, because ureq-proto turns
  URI userinfo into `Authorization: Basic` whenever the request carries no auth
  header (`client/sendreq.rs:140-152`) — which is exactly the state the
  origin-crossing strip leaves it in, so the origin would choose the client's
  credentials one layer below anything this module sees.
- **The origin comparison is `Scheme`, the effective port, and
  `authority().host()` under `eq_ignore_ascii_case`** — never `authority()` or
  `authority().as_str()` (`uri/authority.rs:9,:20`), which carry userinfo and an
  explicit `:443` into a byte compare, and nothing in `uri/` lowercases a host.
  A same-host `Location` that merely drops `user:pass@`, or spells the host in
  another case, otherwise reads as a cross-origin move: `authorization` is
  dropped and the hop answers 401 with nothing naming the cause. Host case is
  the only thing normalized beyond the elided default port; a trailing-dot host
  stays a different origin, because the origin distinguishes them.
- **The credential drop is keyed on the URL the request actually goes to.** It
  lives where a `Prepared`'s final target is composed — which covers a live hop
  **and** a request that starts at a recorded resolution — not only in
  `Prepared::redirected`, which never runs for a map-sourced start: the one-time
  leak the hop guard prevents would otherwise be permanent for the session's
  lifetime. The names are removed by their stored keys, `http:authorization`,
  `http:cookie` and `http:proxy-authorization`: `Metadata::remove` canonicalizes
  through `canonical_http_lookup_key`, which adds no prefix, so a bare name
  matches nothing, answers `None`, mutates nothing, and the call compiles, runs
  and returns while the credential travels. `Metadata::get` misses the same way,
  so the test asserts on the **origin's recorded request headers for the next
  hop**, never on the drop's return. Userinfo does not survive intake either:
  `HttpSession`/`HttpPath`/`HttpFile` construction moves the whole userinfo —
  percent-decoded, as `holder/fs/uri.rs:229-234` already does — into `HttpAuth`
  and stores the stripped URL, because `Uri`'s `Display` writes the authority
  verbatim (`uri/mod.rs:683`, `uri/authority.rs:258-261`) and would print the
  password in `IOBase::url()`, in every `Error::absent`/`conflict` message, in
  `Holder`'s derived `Debug`, in `copy_into`'s staging name and in
  `http:content-location`. `object::without_credentials` moves up to
  `holder/mod.rs` as the one owner and gains one arm rather than being reused
  verbatim — it returns early when `password()` is `None`, which keeps an Azure
  container name and would keep `https://<api-key>@host`, and it rebuilds from
  scheme + host + path, dropping the query and the fragment. "`IOBase::url()`
  never moves" means no redirect rebinds it, not that it is byte-identical to
  what the caller handed in.
- **The map is keyed on whole segments, records only what a `Location` proved,
  and never changes an answer.** A resolution is recorded only for a request
  that carried **no query and no fragment** — the origin's answer to `?sig=X`
  says nothing about `?sig=Y`, and such a key can never hit again — and the
  stored target keeps neither. The prefix is derived from the hop itself: when
  the new location ends with the same whole trailing segments as the old, the
  entry is old-prefix → new-prefix; otherwise it is that one path → that one
  location. Lookup compares whole segments — `Url::segments_under`
  (`uri/glob.rs:156`), never a string `starts_with`, which rewrites
  `/database/x` when `/data/` moved — and the **longest** match wins, so two
  overlapping moves have one answer rather than a `HashMap` iteration order; the
  rewrite re-joins the request's remaining segments and then restores the
  request's own query and fragment, because `joinpath` preserves the target's.
  Only the unbroken leading run of `Permanent` hops rewrites the map, and it
  binds the URL the operation started from **and every intermediate in that run**
  to where the run ends: never `P → Q` beside `Q → R`, because resolution is one
  lookup and not a walk, so a stale intermediate pays the extra hop forever
  instead of once. No status ever evicts an entry; LRU is the only removal. And
  a resolution is a cost optimization that never changes an answer: a `404` or
  `410` on a request the session re-pointed is re-issued **once**, at the URL the
  caller named, with the resolution bypassed, and only that second answer leaves
  the session — so `file.rs` keeps `404 | 410 | 416 → Ok(0)` in one place, the
  re-issue never recurses and is refused for a consumed `Payload::Streamed`, and
  the second answer decides the entry: a permanent redirect back to the resolved
  prefix means the entry was right, anything else means it was over-broad and it
  is dropped. Cost is +1 per absent resource behind a moved prefix and 0 for
  every resource that is there. Without the guard one sibling's 301 makes every
  unmoved sibling read `Ok(0)`: a thousand-file scan silently returns a thousand
  empty files, a write overwrites a resource it believed empty, and a `DELETE`
  at a location the caller never named reports a removal that never happened.
- **A rewrite that drops a body is a refusal, not a rewrite.** The
  `301`/`302`/`303` → `GET` rewrite applies only to a request with no body and
  to `POST`, the one method RFC 9110 §15.4.2's note names. A `301` or `302`
  answering a body-carrying `PUT` or `PATCH` re-sends it unchanged at the
  resolved location exactly as `307`/`308` do — so the `Payload::Streamed`
  refusal is no longer scoped to 307/308, and it is the only thing that stops
  one. A `303` answering a publishing write is `Error::conflict("the write this
  handle staged", "a redirect to a result document", url)`: a `303` `Location`
  names a result to `GET`, never a new write target, so no following request can
  carry the staged bytes and the `200` it answers is not evidence the write
  landed — from `Drop`, where the result is unreportable, the `PUT` would become
  a bodyless `GET` and `flush()` would return `Ok(())` with the caller's bytes
  never stored. `DELETE` keeps its method on `301`/`302` for the same reason:
  rewritten to `GET` it makes `remove()` answer `Ok(())` having deleted nothing.
  A hop that does legitimately drop the payload drops the fields that described
  it in the same step — `content-type`, `content-encoding` and above all
  `content-length`, which ureq reads back out of the map rather than from the
  body (`ureq-proto/src/client/amended.rs:173-199`), so a surviving
  `content-length: N` makes `run.rs:482-528` spin on `Ok(0)` forever with no
  send-body and no global deadline set; ureq's own redirect code removes it for
  exactly this reason (`client/redirect.rs:88`), and at `max_redirects(0)` this
  module inherits none of that. A hop also never re-sends a precondition derived
  from a cached validator — `If-Match`, or an `If-None-Match` carrying a
  concrete entity-tag — because the tag names a representation of a resource the
  request is no longer addressed to and **both** readings are wrong: carried, it
   412s at the new location and surfaces as a lost update that never happened;
  stripped, piece 7's lost-update guard silently disappears and the write
  overwrites blind. So the hop is refused with `Error::conflict` naming both
  URLs in the `path` argument, **after** a `Permanent` status has recorded its
  resolution and dropped the cached validator, so the caller's retry opens at
  the resolved location, re-derives the validator there and publishes once.
  `If-None-Match: *` is carried: it asks whether the target has a
  representation, whatever the target is.

**What moves.** `holder/http/redirect.rs`, `holder/http/session.rs`,
`holder/http/request.rs`, `holder/http/tests/redirects.rs`, `DECISIONS.md`
(the entry that argues with `AGENTS.md:621`), and `AGENTS.md:620-623` itself,
which gains the sentence naming the object-store scope of its own rule.

## 11. Two roles, and no folder

**Today.** `AGENTS.md:47` says a storage backend is a location/container/leaf
trio. `holder/zip/mod.rs:15-18` already argues a departure from it for a store
whose shape does not fit. An HTTP origin has no listing primitive at all.

**Rule to write (decision 50).** `HttpFile` (`IOFile`) and `HttpPath`
(`IOPath`). There is no `HttpFolder`: a role that scraped a directory-index
page would promise a tree the protocol does not have, which is what zip's own
module doc argues against for a store that at least *has* an index. The
departure is argued in the module doc, in that voice. `HttpPath` exists because
`Holder::from_url` must answer with something for a location whose role is
undecided, and because `IOPath::path_kind` (`iopath.rs:32-42`) already answers
`Directory` for a glob or a trailing slash **without touching the store**; its
`is_folder()` is a constant `false` and `as_file()` refuses a trailing-slash or
glob URL with `Error::conflict`.

`HttpFile` implements the nine methods with no default body, satisfies `IOMedia`
with the one line `crate::impl_default_iomedia!();`, implements `IOFile`'s four
members, and **overrides exactly the set below**. Implementing only the nine
compiles and is wrong on cost; the request-count test is the only thing that
catches it.

- `pread` — **one ranged `GET`, never a `HEAD` first.** Checks the stage, then
  an open-scope length so a read entirely past the end is `Ok(0)` with zero
  requests, then **drops the state lock before the wire call**
  (`object/file.rs:426-430`; it matters doubly here because `Buffered::read_at`
  already holds its page lock across the inner `pread`). Fills the caller's
  slice directly — no intermediate `Vec` — and loops until full or `Ok(0)`, so
  a short socket read is not a short `pread`. Folds `Content-Range`'s total
  into the open scope. `404 | 410 | 416` → `Ok(0)`. A **200 answer to a ranged
  request** means the origin ignored `Range`: `offset` bytes are discarded
  client-side and the handle records `accepts_ranges: false`.
- `pwrite` — **stages, never publishes.** One `GET` materializes the stored
  value on the first positional write; later ones mutate the staged buffer.
  Publication is `flush`/`close`/`Drop`. This is not optional:
  `write_all_bytes` (`iobase.rs:850`), `compress_into_with_level` (`:1155`) and
  `decompress_into_with` (`:1199`) each call `flush()` themselves, so a `PUT`
  per `pwrite` turns one logical write into N round trips and breaks the append
  and overwrite paths.
- `size` — zero requests when the open scope, the stage or a prior
  `Content-Range`/`Content-Length` already said; otherwise one probe. It
  returns a bare `u64`, so there is nowhere to report a failure: **a failed
  probe records the length as unknown, not as zero**, and the next call
  re-asks. Degrading a transient 503 into a permanent "empty" would let a
  caller overwrite a resource it thought was empty.
- `media_type` — returns a **borrow**, so inference lives in a
  `OnceLock<MediaType>` and `declared` is checked *before* the `OnceLock` is
  touched (`object/file.rs:691-703`). **Zero requests**: `declared`, then a
  `Content-Type`/`Content-Encoding` pair a completed answer already folded in,
  then `Url::media_type()`, then `MimeType::FILE`. A served type is a windfall
  from a read that already happened, never a round trip taken to answer this.
- `pstream_bytes` — the default is one `pread` per batch, ≈16 000 requests per
  GiB. Override: reject `batch_size == 0` **before opening anything**
  (`object/file.rs:443-448` restates the check verbatim for the same reason); a
  staged value branches to `ByteStream::from_handle`; otherwise **one**
  `HttpStream` handed to `ByteStream::from_reader`.
- `read_all_bytes` — the default is `size()` then `pread_exact`: a probe and a
  transfer. One `GET` answers both. Allocates from `Content-Length` with
  `try_reserve_exact` mapped through `crate::iobase::oversized`
  (`iobase/lifecycle.rs:53`). 404 → `Ok(Vec::new())`.
- `read_range_bytes` — the default clamps against `size()` (a second round
  trip) **and** allocates `length` on the caller's word. `iobase/bytes.rs:368-377`
  calls it with `usize::MAX` for every `read_to_end` from a non-zero offset, so
  this is a *normal* call. Allocate from what the **answer** says is coming.
- `read_range_digest` — the default streams from `offset` to the end of the
  resource and breaks early, which over HTTP asks for a gigabyte to hash
  sixteen bytes. One bounded `GET`.
- `is_container` — a constant `false`; the default asks `kind()`, which here is
  a request. `is_atomic`/`is_tabular` route to `IOFile`, so both answer from
  the media type at zero cost. `kind` is `IOFile::file_kind`.
- `mtime` — from the open-scope `Last-Modified` through
  `crate::holder::system_time_ns` (`holder/mod.rs:33-40`), the one owner of
  that conversion. `None` while unknown.
- `open`/`opened`/`close`/`flush` — `open` issues the one probe and caches
  size, validator, media type and `Accept-Ranges` for the scope; it **never
  caches bytes** unless `with_prefetch(n)` was set, which is the one documented
  departure, and it succeeds on a resource that does not exist without creating
  it. `close` publishes any stage and drops the cache, so closed reads are
  fresh. **`closed()` is never overridden** — it is exactly `!opened()`.
- `parent` builds from `Url::parent` into another `HttpPath`, zero requests;
  `child_by_path` routes to `IOFile::file_child_by_path`, which is
  `NotADirectory` naming the URL (`iofile.rs:87-95`); `ls` routes to
  `IOFile::file_ls`, which is `Listing::empty()` — a resource that cannot
  contain others lists nothing rather than failing.
- `clear` and `remove` **must** be overridden or the trait defaults truncate
  rather than reach the origin (`iobase.rs:975-977`).

`clear` is a documented departure: HTTP, like S3, has no way to empty a value
without writing one, and the contract (`iobase.rs:860-864`) says clearing is
not a write. Under `Writes::None` — the default — it is
`Error::unsupported("clear", "http")`; with writes enabled it sends a zero-byte
write and **says so in its own rustdoc in the same words `object/file.rs:363-377`
uses**.

`remove` takes **no pre-probe, ever.** One `DELETE`. No `HEAD`, no `kind()`, no
`size()`, no exists check, no `ls()` first: that is an explicit contract
violation (`iobase.rs:920-936`) and on an origin each probe is a full round
trip. `404` and `410` map to `Ok(())`; `401`, `403`, `405`, `429` and `5xx`
stay typed failures — a blanket `let _ = …` is named in the docs as not an
implementation of this rule (`iobase.rs:941-942`). It returns `Result<()>`,
never a bool and never a count, because reporting "did something exist" would
force the probe the design refuses.

**Every read path consults the stage first** — `pread`, `read_all_bytes`,
`read_range_bytes`, `pstream_bytes` **and** `read_range_digest`
(`object/file.rs:400-413, 450-455, 470-472, 488-495, 540-546`). Missing one
means a caller cannot read what it just wrote.

Kept at the default, deliberately: `bound_location` (returning `Some` reroutes
`copy_into`/`move_into` into `holder::fs::copy_bound` and `glob` into
`hierarchy::bound_glob`, both of which assume a real filesystem), `closed`,
`pread_exact`, `pwrite_all`, `append_bytes`, `write_all_bytes`, `read_digest`,
`read_scalar`, `write_scalar`, `copy_into`, `move_into`, `compress_into*`,
`decompress_into*`, `codec`, `is_empty`, `is_io`, `partitions`, `glob`,
`children_matching`, `children_where`, and every `Self: Sized` adapter.
**`IOCursor` is not implemented**: `Cursor<H>::stream_bytes`
(`iocursor.rs:175-179`) already keeps one native stream alive, and the
`IOCursor` default is a `read_next`/`pread` ladder, so hand-rolling it would
silently reintroduce one request per batch.

**Edges.**

- **A probe is one `GET` carrying `Range: bytes=0-0` and `Accept-Encoding:
  identity`, and `HttpFile` sends no `HEAD` anywhere** — not in `size`, `open`,
  `mtime`, `kind` or `file_exists`. One 206 answers everything `open` caches:
  `Content-Range`'s total is the length of the identity representation `pread`'s
  offsets address, the answer carries the validator, the `Last-Modified` and the
  media type, and the 206 is itself proof of range support where
  `Accept-Ranges: bytes` is only a claim. A `HEAD` buys strictly less for the
  same round trip: it reports `Content-Length` for whatever coding the origin
  chose for it, so against a compressing origin `size()` is silently the
  compressed length and every offset derived from it is wrong; `head_object`'s
  `.unwrap_or(0)` (`client.rs:1153-1157`) is what a `HEAD` without a length
  costs; and a `HEAD` refused with 405 costs a second request, so its worst case
  is two where this is one. Four answers, each with a total: **206**, the total
  from `Content-Range`, `*` reading as unknown; **416**, where `bytes */N` is
  the length and `N == 0` is a present, empty resource — 416 is the *normal*
  answer for a zero-length resource, so this is the one place it is not the
  `Ok(0)` `pread` maps it to, and dropping it makes every `size()` on an empty
  resource re-ask forever; **200**, where the origin ignored `Range`, the total
  is `Content-Length`, the handle records `accepts_ranges: false` and the body
  is abandoned to the bounded drain and **never read** — a `size()` that
  downloads a gigabyte is the failure this probe exists to avoid; **404/410**, a
  known zero. Anything else is a failed probe and records unknown. `HEAD` stays
  a caller's verb through `HttpSession::head`; `refuse_head` (405) is the
  tripwire asserting no `HttpFile` path sends one.
- **`Content-Range` is parsed in one place and checked against the request that
  produced it.** `holder/wire/range.rs` replaces `total_of_content_range`
  (`client.rs:2620-2624`, which reads only the text after the final `/`) and
  yields `(first, last, total)`: the unit is the literal `bytes`,
  `first <= last`, `total` is a number or `*`, and the unsatisfied `bytes
  */total` form is legal on a **416** only. The refusal is `holder::http`'s,
  never the shared parser's and never `object`'s, or piece 1's byte-for-byte
  gate fails. A 206 whose `first` is not the offset the request asked for, or
  which carries no parseable `Content-Range` — which is what a
  `multipart/byteranges` answer is, and this module writes no MIME parser and
  never copies such a body — is `Error::conflict` before a byte is copied; the
  same check runs on a resume's re-open, where a 206 positioned anywhere but
  `start + delivered` splices bytes `ByteStream` cannot detect. `skip = if
  status == 200 { offset } else { 0 }` (`client.rs:1301,:1434`) is the shape
  that ignores all of this: a proxy that re-ranged or coalesced lands bytes from
  another window at the caller's offset with no error anywhere, and `chars
  0-9/1000` folds a count of something else into the scope as a byte length.
  Which status supplies the total is equally fixed: on a **206** it is
  `Content-Range`'s complete-length and nothing else — a 206's `Content-Length`
  is the *window*, so folding it makes `size()` answer 1000 for a 10 GiB
  resource and every later read takes the zero-request past-the-end `Ok(0)`; on
  a **200** it is `Content-Length`, whatever `Range` the request carried and
  even when a `Content-Range` is echoed beside it, because the whole
  representation is arriving — `open_range`'s present precedence
  (`client.rs:1299-1300`) is backwards and must not move into `holder/wire/`
  unchanged; on a **416** `bytes */N` **re-seats** the recorded length, so a
  resource that shrank stops being read against a stale size; `bytes 0-99/*`
  records **unknown**, never 0 and never 100. `ignore_range_next` echoes a
  `Content-Range` on the 200 it injects, or the precedence is untestable.
- **A 206 covering less than was asked for is not the end of the value.** When
  the served `last` is below the requested last-byte-pos — or, for an open-ended
  `bytes={offset}-`, below `total - 1` — the next ranged `GET` goes out from
  `offset + filled`; short is otherwise returned only on `404 | 410 | 416` or at
  a known total, and a continuation delivering zero new bytes is
  `Error::conflict` naming the URL, so a one-byte-per-answer origin stops rather
  than turning one read into a request per byte. Reading the socket until it
  ends and returning `filled` breaks `iobase.rs:113` for `pread` and `:717` for
  `read_range_bytes` — and `rest_of` calls the latter with `usize::MAX` for
  every `Reader::read_to_end` and `Cursor::read_to_end` from a non-zero offset
  (`iobase/bytes.rs:368-377`, `iocursor.rs:193-199`), which takes whatever comes
  back as the whole remainder. Continuations increment `requests` and `gets`,
  spend no retry token, and carry their own `+1 per continuation` row beside the
  resume row, or this rule and "a ranged read is 1 request" disagree.
- **A zero-width window is answered before any arithmetic and before any
  request.** An empty `buffer` is `Ok(0)`, `length == 0` is `Ok(Vec::new())`, a
  zero-length digest is `algorithm.digester().as_digest()` — the three answers
  `client.rs:1181-1183`, `:1222-1224` and `file.rs:548-550` already give — and
  **no `Range` is ever constructed from a length of zero**. The object backend's
  guard lives in the caller, not in the arithmetic (`client.rs:1259`), so
  folding those two layers into one `pread` drops it: `length - 1` underflows,
  a debug build panics inside a read, and a release build — the workspace sets
  no `overflow-checks` — sends `bytes={offset}-18446744073709551615` and
  transfers the whole tail to answer a request for nothing, still returning
  `Ok(0)`. `Reader::read` and `HandleInput::read` hand `std::io::Read`'s buffer
  straight through, so an empty one arrives from outside. Zero requests for all
  three is an assertion.
- **A stated length is a claim, and the stage has a ceiling.** The
  pre-reservation on a whole or unbounded read is `min(stated, asked,
  DEFAULT_FETCH_BYTE_SIZE)` (`iobase.rs:59`) and the buffer grows from the bytes
  that actually arrived; `client.rs:1231-1238` and `:1341-1349`, which reserve
  the stated number outright, are the shape not to copy, and a body ending short
  of its stated length is a transport cut (`ureq` answers
  `Error::disconnected`) handled by piece 8, never an allocation error —
  `crate::iobase::oversized` names only a value past `usize`, so on a host that
  overcommits `Content-Length: 17179869184` reserves 16 GiB per concurrent read
  to receive ten bytes, and on one that does not the ten-byte read is refused as
  "not addressable by usize". On the write side every path that grows the stage
  — `pwrite`, `truncate`, `reserve` — checks the requested end offset against
  `MAX_DOCUMENT` **before `try_reserve` and before `resize`** and refuses with
  `Error::unsupported("staging a value larger than one request body", length)`:
  the stage is published as one body and this module has no multipart, so
  `object/file.rs:817-827`'s `resize` is not the guard — `try_reserve` refuses
  only what the address space refuses and `Vec::resize(size, 0)` then commits
  every reserved page by writing zeros into it, so `pwrite(1 << 33, b"x")` from
  a corrupt index zero-fills 8 GiB of real memory and, if it survives, puts all
  8 GiB on the wire as one body.
- **What an answer does not say is unknown, and unknown is not zero.** A probe
  that **succeeds** but states no length — chunked, HTTP/1.0 close-delimited, or
  a `HEAD` a caller made — records unknown: `ureq::Body::content_length()` is
  already `Option<u64>` and answers `None` for both
  (`ureq-3.4.0/src/body/mod.rs:195-206`), so `.unwrap_or(0)` is the bug. Unlike
  a failed probe this is **not re-asked** — the same question gets the same
  non-answer, and re-probing inside an open scope breaks the `size` row's zero —
  and it is resolved only by a transfer that states a total or one that ends;
  unknown is never read as a length, so `pread` may not take the past-the-end
  short-circuit and `pwrite` may not skip the materializing `GET`. `file_exists()`
  answers `false` only for a definitive `404` or `410`: a `503`, a timeout or a
  `405` leaves existence unknown, is not cached as absence, and answers `true`,
  because `object/file.rs:358`'s `is_ok_and(..)` carried over makes a caller's
  `if !exists { write }` clobber a resource that was merely unreachable, and
  `iofile.rs:60-66` maps that `false` to `IOKind::Unknown`. On a read path only
  `200` and `206` are readable: `201`, `202`, `203`, `204` and `205` are the
  origin answering something that is not a representation and raise the same
  status-carrying `Error::remote`, never `Ok(0)` — ureq frames a 204 `NoBody`
  unconditionally, so zero bytes otherwise read as emptiness at that offset and
  `pwrite`'s first-write `GET` stages an empty value that publication writes
  over a resource with bytes. That materializing `GET` maps **only 404 and 410**
  to an empty stage; every other status and every transport failure that
  survives the retry budget refuses the `pwrite` and leaves the stage `None`, so
  the next write re-asks (`materialize`'s `if state.stage.is_none()` guard makes
  that free). Unlike `size`, `pwrite` returns a `Result` and has somewhere to
  report it — and an empty stage is not a neutral start: `pwrite` zero-fills the
  gap below `offset` and publication replaces the whole resource, so a transient
  503 stages zeros and `flush` publishes them while both calls answer `Ok`. This
  is the only path in the backend that destroys data it never read.
- **A scope is one snapshot of one representation.** Size, mtime,
  `Accept-Ranges`, served media type and validator are learned together and
  dropped together: a ranged answer's total folds into the open scope only when
  that answer's validator equals the scope's, and a differing validator drops
  the **whole** scope to unknown — the same landing a failed probe has, never
  zero — so the next call re-asks and one field is never rewritten under the
  others. `learn_size`'s unconditional `meta.size = size` (`object/file.rs:185`)
  is what not to copy: the lock is released across the wire call so two `pread`s
  overlap, their totals land out of order, the older and smaller wins, and the
  open-scope shortcut then answers `Ok(0)` with zero requests on a scope whose
  length came from one generation and whose `ETag` came from another. `open`
  records the scope only **after** the probe answers — a `404` or `410` is an
  answer and opens an empty scope; a transport failure, a `503` or an exhausted
  budget leaves `opened()` false and caches nothing — because
  `object/file.rs:753-754` sets the flag and *then* probes, so `open` returns
  `Err` while `opened()` answers `true` and every later read writes into a scope
  the caller never entered and will never `close`. `close` **takes** the
  media-type cell (`OnceLock::take`, sound on `&mut self`), so a served
  `Content-Type` learned in one scope can never answer `media_type()` in the
  next; one cell, not two, because the URL inference it discards with it is a
  pure function of a `url()` that never moves. And the cell is seated **only
  from the URL** — `Url::media_type()`, else `MimeType::FILE` — with the served
  pair held in its own `Option<MediaType>` field beside `declared`, written at
  the `&mut` boundaries from an answer already in hand and read in field order
  `declared` → served → cell: a `OnceLock` seats once and `media_type()` takes
  `&self`, so seating the ladder inside the initializer lets the first caller
  freeze the name-derived guess forever — `Coded::infer` asks `codec()` before
  any request (`coding/coded.rs:62-65`), freezes `Identity`, and hands a gzip
  body back undecoded and labelled decoded.
- **A mutation refuses at the point of call, and a destructor does the least it
  can.** Under `Writes::None` — the default — `pwrite`, `truncate`, `clear` and
  `remove` each return `Error::unsupported(<verb>, "http")` at the call, so
  nothing is staged that cannot be published and no materializing `GET` is spent
  on a write that can never land; the defaulted composites inherit it through
  those four, there is no second gate in `publish`, and under `Writes::None`
  every write row of the cost table is 0 requests. Putting the gate where the
  wire write happens is the reading "publication is `flush`/`close`/`Drop`"
  invites and is the silent one: `write_all_bytes` calls `flush()` itself
  (`iobase.rs:850`) so that path fails, but a positional `pwrite` then a drop
  returns `Ok(n)`, spends one `GET`, publishes nothing and reports nothing.
  `HttpFile::drop` publishes nothing at all while `std::thread::panicking()` —
  it discards the stage and emits one `log::warn!` naming the URL and the staged
  length — because an interrupted writer's stage is a torn value that a `PUT`
  would make the resource's whole value with no `Result` anywhere, and because a
  panic inside the wire call while already unwinding is a double panic and
  aborts the process (ureq asserts on its own invariants at
  `timings.rs:125-130`, `run.rs:714,:745`); the poisoned-lock arm copied from
  `object/file.rs:797-805` does not cover this, since the state lock is
  deliberately released across every wire call and a panic between two `pwrite`s
  leaves it unpoisoned. Otherwise a `Drop` publish sends the same conditional a
  `flush` would — never a blind overwrite to make `Drop` succeed — **forces its
  attempt count to 1**, the knob piece 9 already forces for a
  `Payload::Streamed`, so no token is spent and `pause` never sleeps; three
  attempts at three 120 s per-phase timeouts with a verbatim `Retry-After` of up
  to 30 s between them blocks a scope exit for minutes for a result `let _ =`
  discards. A failure is `log::warn!`-ed once naming the URL, the method and the
  status or transport error — the 412 that piece 7's `If-Match` exists to
  produce is otherwise detected and then thrown away with the bytes
  (`media/iceberg/staging.rs:213-223` is the house shape) — and the stage lock
  itself is `self.state.lock().map_err(|_| poisoned())`, never
  `PoisonError::into_inner`, which is right for `buffered`'s recomputable pages
  and wrong for the only copy of bytes nothing else holds.
- **`HttpPath` never probes, and answers three questions from the URL.** Its
  role comes from the URL alone and no byte method consults `path_kind`,
  `is_file()` or any other existence question — absence is settled by the
  `GET`'s own `404 → Ok(0)`. Do not port `with_resolved` (`object/path.rs:222-235`):
  its probe-then-dispatch costs a round trip before the first read on the very
  type `Holder::from_url` answers, its `absent` default returns `Ok(0)` *without
  sending the `GET`*, and it holds the resolution mutex across the dispatched
  wire call, undoing `pread`'s lock-drop rule from a layer that rule does not
  reach. Every byte method forwards to **one retained** `HttpFile` in a
  `OnceLock`, never a fresh `as_file()?` per call (`object/path.rs:474,:500`):
  `HttpFile` stages and `Drop` publishes, so a per-call handle re-`GET`s and
  `PUT`s on every `pwrite` and loses the stage between them. Three `IOBase`
  defaults are overridden because `kind()` on this type is `path_kind()`, which
  falls through to `is_file()`: `is_container` is `url.is_glob() ||
  url.has_trailing_slash()` — the probe can never change it, since `is_folder()`
  is constant `false` and neither `File` nor `Unknown` is a container;
  `is_atomic` is `path_is_atomic`, media type alone; `is_tabular` is
  `media_type().is_tabular()` and nothing further — **not** `path_is_tabular`,
  which for the `DIRECTORY` media type a glob or trailing slash produces
  descends into `container_is_tabular` (`iobase/hierarchy.rs:104-134`) and pays
  an `ls` plus, under `iceberg`, a `child_by_path` and a metadata listing to
  learn what an origin with no listing primitive can only answer `false`. These
  are the first three questions every generic caller asks, so the accounting
  test asks all three of an `HttpPath` and not only of an `HttpFile`.

In the `open`/`opened`/`close`/`flush` bullet, replace "unless `with_prefetch(n)` was set, which is the one documented departure" with: "unless `with_prefetch(Tail | Head, n)` was set, which is the one documented departure. The direction is named because the probe fuses only with a fetch that does not need the size it is asking for: a head window renders `bytes=0-{n-1}`, a tail window renders the **suffix** form `bytes=-{n}` (RFC 9110 §14.1.2), whose 206 carries the complete length in `Content-Range`, so the probe and the prefetch are one request. Never `bytes={size-n}-`: it needs the size and so costs the request it exists to save. The cached window's start is read off the answer and **never computed as `size - n`** — a suffix request against a shorter representation is answered whole, and a `200` means `Range` was ignored — so subtracting caches the head under the name of the tail and the 8-byte read at `size-8` then returns the wrong bytes with no error; a complete length of `*` leaves the size unknown, so the prefetch was not a probe. A tail window is what a footer-first format needs: `media/parquet/mod.rs`'s `load_metadata` (`:567,:574,:599`) is 1 request over an opened `with_prefetch(Tail, 64 KiB)` handle, 2 over `Buffered<HttpFile>` at 1 MiB pages, and 3 over a bare closed handle. `Buffered` cannot substitute — `buffered/mod.rs:58` says pinning is a retention guarantee, never a prefetch."

After the `pread` bullet's "the handle records `accepts_ranges: false`", add: "That flag has a reader, or it is write-only and every later `pread` repeats the whole-resource transfer with a longer discarded prefix — a 1024-page scan of a 1 GiB resource moves 1 TiB in 1024 full-body round trips and errors nowhere. A 200 to a ranged request is a **mode**: the same body is read whole into the stage, **clean**, so `flush`/`close`/`Drop` still publish nothing, and every later read on the handle is served from it at zero requests. `Accept-Ranges: none` on the probe sets the same flag, which lives with the stage and not only in the open scope, or a closed handle re-learns it on every `pread`. A stated length above `MAX_DOCUMENT`, or a chunked body that reaches it, is `Error::unsupported(\"ranged reads against an origin that ignores Range\", url)` rather than a whole-resource transfer repeated per page. Asserted: ten `pread`s after one `ignore_range_next` are one request."

Append to the "Kept at the default, deliberately" paragraph: "A `Coding` wrapper is not covered by the table above and does not preserve it. `Coding` has no decoded seek — `decoded_stream` (`coding/mod.rs:127-148`) reads only `handle.pstream_bytes(0, window)` — so on a **closed** handle every `Coding::pread` and `Coding::read_range_bytes` is a fresh whole-resource `GET` from byte zero whose body is abandoned, and `Coding::size` (`:439-443`) is a whole decode reported as `streamed_size().unwrap_or(0)`, so a 503 makes a coded handle read as empty and a caller overwrites a resource it thought was zero-length — the wrongness `HttpFile::size` refuses below it, reintroduced above it. `Coding::open` (`:500-504`) materializes the decoded value once and every later read answers from memory at zero requests, so **open is the fix**, and the module doc and the cost table say so where they name `Holder::from_url(url, [(\"codec\", \"gzip\")])`. Buffering is not the fix: `Buffered` delegates `pstream_bytes` untouched, so pages under a coding are never consulted, and `into_coded_with` (`holder/mod.rs:547-551`) lifts an existing cache back outside the coding by design — the pairing stays `Buffered(Coded(HttpFile))`."

**What moves.** `holder/http/{file,path}.rs`, `holder/http/tests/roles.rs`,
`holder/http/mod.rs` (the cost table as the module's own rustdoc, in the shape
of `object/file.rs:16-36`).

## 12. `http://` resolves like every other location

**Today.** `Scheme::is_storage()` (`scheme.rs:249-263`) already lists `Http` and
`Https` as "the schemes a filesystem abstraction can open", and has since it
was written. `Holder::from_url` (`holder/mod.rs:200-252`) has no arm for them,
so every such URL falls to
`Error::unsupported("holding a location of this scheme", …)` at `:234-237`.
The crate promises a handle it does not have.

**Rule to write (decision 51).** `Scheme::is_http()` exists beside
`is_object_store()`, spelled the same way and for the same reason — one
predicate, read everywhere the scheme is branched on, so the three never drift.
`Holder::from_url` gains one arm answering `HttpPath`, and the two universal
properties it already reads at `:239-250` (`media_type|mime_type|content_type`
and `codec|content_encoding`) apply unchanged. Two `Holder` variants, and the
four matches at `:564, :591, :619, :647` grow with them. Without the `http`
feature the arm is a refusal naming the feature, exactly as the object arm at
`:228-231` does today.

**What moves.** `rust/src/scheme.rs`, `rust/src/holder/mod.rs`,
`rust/tests/holder/`, `.api-inventory.txt`, `docs/holder/index.md` (the Pages
and Variants tables), `docs/holder/backends/http.md`, `mkdocs.yml` nav
(`- HTTP: holder/backends/http.md` after the ZIP row at `:162`).

## 13. The cost model is an assertion, not a claim

**Today.** `AGENTS.md:503-510` requires every derived surface to state its cost
in call counts, pin it in `rust/tests/iobase_calls.rs`, and report it in the
`holder` benchmark beside the timing. `holder/object/tests/accounting.rs` is
1154 lines of exactly that for the object backend, against the in-process
`FakeS3` at `holder/object/tests/server.rs`.

**Rule to write (decision 52).** The same, twice: what a layer asks of storage
in `rust/tests/iobase_calls.rs` through `Counted`, and what those calls become
on the wire in `holder/http/tests/accounting.rs` against an in-process origin.
The split is the repo's own (`iobase_calls.rs:1-14`). The table is in the
module's rustdoc and in `docs/holder/backends/http.md` in the same numbers:

| operation | requests, closed | requests, open |
| --- | --- | --- |
| construction, `url`, `media_type`, `set_media_type`, `is_container`, `is_atomic`, `is_tabular`, `ls`, `child_by_path`, `parent`, `partitions`, `reserve` | 0 | 0 |
| `truncate(n)` where `n == size()` | 0 | 0 |
| `size`, `mtime`, `kind`, `is_empty`, `file_exists`, `capacity` (unstaged) | 1 probe | 0 |
| `open` | 1 probe | — |
| `pread`, `read_range_bytes`, `read_range_digest` | 1 ranged `GET` | 1 |
| `read_all_bytes`, `read_digest`, `read_scalar`, `pstream_bytes` (whole drain) | 1 `GET` | 1 |
| `pwrite` (first positional write on a resource with bytes) | 1 `GET` | 1 |
| `pwrite` (subsequent), `truncate(0)` | 0 | 0 |
| `flush` / `close` / `Drop` with a dirty stage | 1 write | 1 |
| `write_all_bytes`, `clear` | 1 write | 1 |
| `remove` | 1 `DELETE` | 1 |
| `close`/`flush` with nothing staged | 0 | 0 |
| a permanently redirected URL, first operation on the session | +1, once per session per prefix | — |
| a 206 covering less than was asked for | +1 per continuation; never a retry token | — |
| a resumed transfer | +1 per resume; never a retry token | — |

`Counted` tallies use the verified `Call::name()` spellings
(`counted.rs:196-231`) and `CallCounts`'s own `Display` (`name=count` in
`Call::ALL` order, or the literal `none`): a whole read is `read_all_bytes=1`;
a ranged read `read_range_bytes=1`; a positional read `pread=1`; a drain
`pstream_bytes=1`; a whole write `write_all_bytes=1`; a cold `Buffered` ranged
read `pread=1 size=1`; a warm one `none`.

Each of these is one assertion, named in the test's own name:

- construction is 0 requests;
- a ranged read is 1 request and transfers the range, asserted by the recorded
  `Range` header;
- **no read path calls `size()`**;
- a 64 MiB drain at 64 KiB batches is 1 request, not 1024;
- 1000 sequential ranged reads are 1 TCP connection and 1 TLS handshake;
- six abandoned streams are 1 connection, **including when the origin answers
  chunked**;
- a request with N session headers and no request headers allocates 0 times
  for headers;
- a full drain holds `O(batch_size)`, never `O(N)`;
- `read_range_bytes(offset, usize::MAX)` on a sixteen-byte resource completes
  rather than aborting on allocation;
- every error body and every non-streamed `send()` answer is bounded at
  `MAX_DOCUMENT`, asserted beside a 64 MiB whole read that succeeds.

`Buffered<HttpFile>` is the recommended pairing, **stated in the module doc and
never applied silently** — composition is the caller's:
`BufferedOptions::default().with_page_size(DEFAULT_FETCH_BYTE_SIZE).with_max_bytes(64 << 20)`,
**in that order**, because `with_page_size` re-applies the `max_bytes` clamp
(`buffered/options.rs:82-87`) and a budget set first is silently overridden. On
HTTP a page *is* a request, so the page takes `DEFAULT_FETCH_BYTE_SIZE` (1 MiB),
not `DEFAULT_STREAM_BATCH_SIZE` (64 KiB) — `iobase.rs:51-61` says why the two
constants differ. The coalescing cap is `max_bytes / page_size`
(`buffered/mod.rs:390`), so 1 MiB pages under the default 8 MiB budget cap one
miss at 8 pages; raising the budget is what makes coalescing work at this page
size, not optional tuning. `Buffered` delegates `pstream_bytes` untouched
(`buffered/mod.rs:503`), so a full scan goes straight to the one-`GET` path;
and it holds its page-table mutex across the inner `pread` (guard at `:257`,
call at `:313-314`), so two threads reading one buffered handle serialize —
**parallel range reads need separate `HttpFile` handles over one shared
`HttpSession`**, and the docs say so.

The benchmark is `rust/benchmarks/holder/http/{bytes,access,resume}.rs`,
registered in `holder.rs`'s single `criterion_group!` with a
`#[cfg(not(feature = "http"))]` stub block so the list compiles in every
feature state. Groups `http_bytes`, `http_access`, `http_resume`, against the
in-process origin over a real socket, with **raw ureq** as the baseline —
`object_store` does not speak plain HTTP, and adding `reqwest` would measure a
second stack. That choice is stated in the docs Performance section rather than
left implicit. Every corpus size goes through `crate::bench_profile::corpus`,
every fixture is outside the timed loop, and `origin.set_recording(false)` keeps
the harness log out of the measurement.

**Edges.**

- **The table is the bare handle's, and `Buffered` changes two rows of it.**
  `Buffered` delegates `pstream_bytes` untouched (`buffered/mod.rs:503`) but
  **overrides `read_all_bytes` into `read_range_bytes(0, usize::MAX)`**
  (`:521-523,:532-543`), so on the recommended pairing a whole-value read is not
  one `GET`: it is `ceil(size / max_bytes)` ranged `GET`s — one per coalesced
  miss run, capped at `max_bytes / page_size` (`:390`) — plus one `size()` probe
  when the handle is closed, and at its peak it holds `max_bytes` **twice**
  above the returned value, because `fetch_run` allocates one contiguous
  `vec![0_u8; pages * page_size]` (`:404-406`) and keeps it alive while every
  page it covers is copied out and inserted. At 1 MiB pages under a 64 MiB
  budget that is 128 MiB over the value and a 1 GiB whole read is 17 requests,
  against "a full drain holds `O(batch_size)`, never `O(N)`".
  `read_digest`, `read_scalar`, `read_range_digest` and `pstream_bytes` all
  reach the handle through the delegated `pstream_bytes`, so those stay one
  `GET` and one batch, and the module doc says whole-value reads on a buffered
  handle go through them. `Buffered`'s `IOMedia` impl (`:451-492`) likewise
  delegates `row_size`, `column_size`, `record_options`, `read_arrow_field`,
  `read_arrow_reader` and both `read_parquet_*` to the inner handle, so the page
  cache serves byte reads only: the pairing recommendation is scoped to byte
  reads in the same sentence, and `read_parquet_statistics()` through
  `Buffered<HttpFile>` is pinned at its true request count rather than at one
  page fetch.
- **The redirect row's assertion pins prefix keying, not repeat visits.** Two
  handles under one moved prefix on one session cost **one** extra request in
  total — the second handle, never touched before, starts at the resolved
  location. A one-handle/two-operation test passes under exact-URL keying and
  therefore pins nothing, while a thousand-file scan pays a thousand extra
  requests against a green suite. Two boundaries are pinned with it, because
  both fail silently rather than slowly: `/a/f1 → /b/renamed` teaches that one
  URL and no prefix, and a handle at `/database/x` takes **zero** rewrites under
  a moved `/data/` — a mis-keyed rewrite reaches a URL that does not exist and
  `pread` maps `404` to `Ok(0)`, so a whole directory reads as empty with no
  error anywhere.

**What moves.** `rust/tests/iobase_calls.rs` (a `mod http` block),
`rust/tests/allocations.rs`, `holder/http/tests/accounting.rs`,
`rust/benchmarks/holder.rs`, `rust/benchmarks/holder/http/{mod,bytes,access,resume}.rs`,
`docs/holder/backends/http.md`, `docs/benchmarks.md` (one row).

## What a server answers, and what the module does

One table, because the same answer reaches `send`, `pread`, `redirect.rs` and
`HttpStream` and they must not disagree. It is the module doc's, the docs page's,
and the shape `holder/http/tests/origin.rs` injects against.

| the answer | this module |
| --- | --- |
| `1xx` but `101` | never seen — ureq consumes them (`ureq-proto-0.6.1/src/client/recvresp.rs:49-62`). No `Expect: 100-continue` is owed |
| `200` to an unranged read | the value; the total is `Content-Length` |
| `200` to a ranged read | `Range` was ignored: `accepts_ranges: false`, total from `Content-Length`, an echoed `Content-Range` read by nothing |
| `200` to an `If-Range` resume | the validator did not hold — `Error::remote`, unless the answer's validator equals the stream's, which is `Range` ignored (piece 8) |
| `201`–`203`, `204`, `205` on a read path | `Error::remote` carrying the status; never `Ok(0)` |
| `206` | one part, `Content-Range` checked against the request (piece 11); `multipart/byteranges` is refused |
| `301`, `308` | `Permanent`: followed, recorded, validator dropped |
| `302`, `303`, `307` | `Temporary`: followed, nothing recorded |
| `304` | a hit only against the validator that conditioned it (piece 7); otherwise `Error::conflict` |
| `404`, `410` | `Error::absent` at the boundary; emptiness below it on an initial read; a conflict on a resume; `Ok(())` for `remove` |
| `405`, `501` | `Error::remote` carrying the status; never widened to `Unsupported` |
| `409`, `412` | `Error::conflict` |
| `416` | `Ok(0)` on a read, a length on a probe, a conflict on a resume unless `bytes */N` equals the resume point |
| `429`, `503` | retried for every method |
| `500`, `502`, `504` | retried for idempotent methods only |
| `600`–`999` | reachable: `Error::remote` carrying the number. The catch-all is a status, never `unreachable!()` and never clamped into 5xx |
| below `100` | never arrives as a status — see the lockfile note below |
| `Content-Length` + `Transfer-Encoding` | `Error::Conflict` at the fold |
| two disagreeing `Content-Length` | `Error::Conflict` at answer construction; the body is dropped undrained |
| chunked, or no framing header at all | framed by ureq; the length is **unknown**, never 0 |
| `Content-Encoding: identity` or empty | dropped per token before the media type is read |
| `Content-Encoding: br`, `compress` | `Error::unsupported` naming the coding |
| a value carrying obs-text | kept — read with `str::from_utf8`, not `HeaderValue::to_str` |

Three rules the table depends on and no single piece owns:

**A status below 100 is a panic in the pinned transport.** `httparse` accepts any
three digits with no range check (`httparse-1.10.1/src/lib.rs:933-941`) and
`ureq-proto` 0.6.1 then does `StatusCode::from_u16(v).unwrap()`
(`src/parser.rs:57`), so `HTTP/1.1 099 x` unwinds out of `pread` on bytes a
hostile origin chooses. 0.6.3 returns `Error::HttpParse` there and satisfies
ureq 3.4.0's `^0.6.1`, so piece 1 moves `Cargo.lock` to ureq-proto 0.6.3 — no
other dependency moves — and re-anchors the 0.6.1 line citations in pieces 3
and 4. After the bump `099` reaches this module as a transport failure with no
status, and `fail_next(99)` asserts exactly that.

**`Error::conflict` is a create-collision, not a general disagreement.** Both
nouns are `&'static str` (`error.rs:347`) and its `Display` renders `expected to
create a {expected} at {path:?}, got an existing {actual}` (`:231-237`). A
disagreement whose two sides are runtime values cannot be spelled with it, and
substituting a pair of fixed phrases so it compiles drops the two facts that
make it a conflict while the test still passes. Those are the crate's existing
unreadable-answer spelling — `malformed()` (`object/client.rs:2651`), whose
`code`/`message` are `impl AsRef<str>`: `Error::remote("http", method.as_str(),
status, "<Code>", format!(…), mask_uri(url))`. `holder/wire/` owns that one
helper and `holder/http/` calls it. `Error::conflict` survives where both sides
are fixed nouns and the create framing is true — the 412 lost update, `as_file()`
on a glob or trailing-slash URL, the 303 over a staged write.

**Every `Error::remote` this module builds masks its URL at the call site.**
`remote` is the one constructor that does not mask (`error.rs:388`, against
`:339` and `:348`), and an HTTP URL is the only path in this crate carrying
`user:pass@` or a presigned query, so unmasked it lands verbatim in the message,
in `Display`, and in every log line that renders it — including the
`"TooManyRedirects"` call, whose status is the **last hop's**, the only status a
server actually answered.

## The origin to build, in `holder/http/tests/origin.rs`

A std-only loopback HTTP/1.1 origin, a leaf file `#[path]`-included from the
tests and from the benchmark, shaped like `holder/object/tests/server.rs:1-14`:
it depends on `std` alone and names nothing of the crate. Deterministic
answers, as `FakeS3` is — a quoted FNV-1a `ETag`, one fixed `Last-Modified`.

It must be able to inject, by name: `redirect_next(status, location)`,
`fail_next(status)`, `fail_after(bytes)`, `cut_next_body(after)`,
`ignore_range_next` (answer 200 to a ranged request), `rewrite_body` (change
the entity mid-transfer so `If-Range` fails), `refuse_head` (405), `chunk_next`
(answer without `Content-Length`), and `oversized_error`. It records
`Recorded { method, path, headers, range }`, `connection_count` and
`request_count`, because every assertion in piece 13 is a count or a recorded
header.

Without `rewrite_body` and `chunk_next` the two sharpest rules in this prompt —
the `If-Range` conflict and the chunked-drain fix — cannot be tested at all.
Build them first.

## Refuted before, still standing

- Following a 3xx inside `holder::object`. Piece 10 changes `holder::http` only;
  the object backend keeps `max_redirects(0)` and its region correction, which
  is not a `Location` to follow.
- Retrying a 4xx other than 429. The origin answered; the answer is the result.
- Parsing the HTTP-date spelling of `Retry-After`. Deliberately not parsed, and
  a value past `RETRY_AFTER_CAP` falls back to the jittered backoff: waiting
  minutes inside a call nobody can cancel is worse than failing.
- A `rand` dependency for jitter. The xxh3-over-a-counter draw is already in
  the crate and is predictable in a test.
- Letting ureq decode `Content-Encoding`. `rust/Cargo.toml:113-115` records why
  it is off, and a decoded body corrupts a range.
- Caching bytes in `open`. `open` caches metadata, never bytes — that is
  `object/mod.rs`'s rule and `Buffered`'s job.
- Speaking HTTP/2 now. It is reachable synchronously — `h2` pulls tokio only
  for the `AsyncRead`/`AsyncWrite` traits and can be driven by a hand-rolled
  `std::task::Waker` over a blocking socket, with no executor and no `unsafe`,
  and all eleven net-new crates are already in this repo's `Cargo.lock` through
  the `object_store` dev-dependency. It is not taken, because no measurement
  justifies eleven crates and a connection driver whose failure mode is a
  silent hang. Multiplexing is the only win and it is unreachable while
  `IOBase`'s contract is one range per blocking call. Revisit when **both** are
  true: a profile from the deployment network, not a proxied sandbox, shows
  many concurrent small ranges dominating; and `HttpFile` issues ranges in
  batches. Then it is `h2` behind an `http2` feature, and never hyper, reqwest
  or a tokio runtime.
- Speaking HTTP/3 now. `quiche` is the only sans-io HTTP/3 implementation and
  its only TLS backend is BoringSSL through `cmake`, against
  `rust/Cargo.toml:52`'s own "still no C toolchain needed", at MSRV 1.88
  against the 1.85 floor Gate 1 checks. `quinn-proto` 0.11.17 is pure Rust,
  sans-io, MSRV exactly 1.85 and already in the lockfile — and is QUIC
  transport only, so RFC 9114 and RFC 9204 QPACK would be this crate's code
  forever. One comment records that door; the pieces move on.

## How to prove it

Per piece: the decision written first, then Gate 1 whole from the repository
root — `cargo fmt --all -- --check`; `cargo clippy --locked -p yggdryl
--all-targets --no-deps -- -D warnings`; `cargo clippy --locked --workspace
--all-targets --all-features --no-deps -- -D warnings`; `cargo test --locked
-p yggdryl --all-targets`; `cargo test --locked -p yggdryl --all-targets
--features "parquet iceberg"`; `cargo test --locked -p yggdryl --doc`;
`RUSTDOCFLAGS="-D warnings" cargo doc --locked -p yggdryl --no-deps`;
`cargo check --locked -p yggdryl --profile bench --benches`; and the MSRV block
`cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl`
at `--all-targets`, at `--no-default-features --lib`, at
`--no-default-features --features object --lib`, plus
`cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl
--all-targets --features iceberg`.

Three new feature-state checks join that block, and Gate 1 in `AGENTS.md:1035-1038`
gains them in piece 1's commit:

```bash
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --features wire --lib
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --features http --lib
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features "object http"
```

A `wire`-only build must compile with neither `object` nor `http`, or the
extraction left a module nothing consumes.

The cost and allocation pins re-run on every piece from 7 on:
`cargo test --locked -p yggdryl --test iobase_calls --test allocations`.

**Piece 1 has one extra gate, and it is the piece's whole point:**
`cargo test --locked -p yggdryl --features object holder::object::tests` must
pass with `rust/src/holder/object/tests/accounting.rs` and `protocol.rs`
**unmodified**. If either file needs a change, the extraction changed the
object backend's behaviour and is wrong.

Benchmarks for the surfaces touched, release build, numbers regenerated:
`cargo bench -p yggdryl --bench holder --features http -- http_ --noplot`.

Bindings are **not** rebuilt and Gates 2 and 3 do not run: this is Rust-only
work per `AGENTS.md:32-34`. Confirm that `python/Cargo.toml:23-26` and
`node/Cargo.toml:23-26` — both pinned to `features = ["iceberg", "object"]` —
still build after the feature graph changes in piece 1, and that the two new
`Holder` variants fall into Python's existing `Role::Held` arm
(`python/src/iobase.rs:150-153`) rather than failing a match. That is a build
check, not a binding.

Gate 4 runs for the docs page: `python scripts/check_docs_examples.py` for
`rust`, `python` and `javascript`, then `python -m mkdocs build --strict`.

## Do not

- Do not copy the retry loop, the pool, `Pooled`, `Resuming`, `is_resumable`,
  `is_retryable_transport` or the stats into `holder/http/`. Piece 1 moves
  them once; a second copy is the thing `AGENTS.md:52-58` forbids by name.
- Do not keep `object::StatsSnapshot` beside `WireStatsSnapshot`, and do not
  keep an alias for it. The bindings and the doctest lose the old name.
- Do not write a header parser, a header validator, or an allowlist of header
  names. `rust/src/metadata/validation.rs` owns the RFC 9110 grammar; calling
  it is the whole of piece 6.
- Do not call `Metadata::from_entries` before folding. It refuses a duplicate
  canonical key rather than taking last-wins, so a response carrying `Vary`
  twice would fail construction.
- Do not merge two disagreeing `Content-Length` headers, and do not take the
  last one. That is a request-smuggling shape; it is `Error::Conflict`.
- Do not send `Accept-Encoding` with anything but `identity` on a ranged
  request, and do not build the whole-read list from `Codec::as_str()`, which
  emits the unregistered `"zlib"` and the forbidden `"identity"`.
- Do not `HEAD` before a read. Not in `pread`, not in `read_all_bytes`, not in
  `read_range_bytes`, not in `pstream_bytes`. A ranged `GET` answers the length
  and the bytes in one exchange.
- Do not pre-probe in `remove`, `clear` or any write. `iobase.rs:920-936` names
  that as a contract violation, and a blanket `let _ = …` is named at `:941-942`
  as not an implementation of the rule.
- Do not publish from `pwrite`. `write_all_bytes`, `compress_into_with_level`
  and `decompress_into_with` each call `flush()` themselves.
- Do not let `size()` answer `0` for a failed probe. Record it as unknown.
- Do not return `Ok(0)` for a cut connection. `ByteStream` reads that as
  permanent end of stream.
- Do not resume by re-reading from zero and discarding a prefix.
  `bytestream.rs:343-346` returns `Ok(0)` forever if the source ends during the
  skip, so that shape truncates silently.
- Do not treat a `200` answer to an `If-Range` resume as a fresh start, and do
  not treat a `404` on a resume as emptiness. Both are `Error::Conflict`.
- Do not spend a retry token on a resume or on a redirect hop, and do not count
  either as a retry in the stats.
- Do not move `IOBase::url()` when a redirect resolves. `resolved_url()` and
  `http:content-location` carry the new binding.
- Do not carry a validator across a recorded redirect.
- Do not follow a redirect mid-stream.
- Do not put the idempotence gate in `budget.rs`; it would change how
  `holder::object` retries a `PUT`.
- Do not implement `IOCursor`, and do not add an `HttpFolder`.
- Do not reach for `unsafe`; `holder/local/file.rs:14` stays the crate's only
  unsafe site. Do not add `rustls` as a direct dependency.
- Do not make `with_tls_verification(false)` the default, do not accept
  `insecure` as an alias, and do not read the setting from any environment
  variable but `YGGDRYL_TLS_VERIFY`.
- Do not offer `RootCerts::PlatformVerifier`: without ureq's
  `platform-verifier` feature it panics at `src/tls/rustls.rs:183`.
- Do not apply `Buffered` inside the handle. Composition is the caller's, and
  a wrapper that keeps its own copy of what the store said hides the call
  (`AGENTS.md:508-510`).
- Do not advertise `h2` or `h3` in ALPN, and do not reach into
  `ureq::unversioned::transport` to do it. ureq sends no ALPN extension and
  cannot frame either version, so a server that selected one would receive
  HTTP/1.1 bytes on an HTTP/2 connection; and that module is documented as
  exempt from semver, so a minor bump breaks the build.
- Do not add `h2`, `hyper`, `reqwest`, `h3`, `quinn`, `quinn-proto`, `quiche`,
  `h2-sans-io`, `fluke-hpack` or `tokio`, and do not write an `http2.rs` stub
  or a `#[cfg(feature = "http2")]` gate over code nothing compiles.
  `rust/Cargo.toml:37` says "no SDK, runtime, or async executor rides in".
- Do not add a transport trait, a boxed transport, or a `Wire` enum with one
  variant. `Provider` is the house shape for a dispatcher and it is written in
  the commit that adds the second value, not before.
- Do not add `multiplexes()`, `allows_0rtt()`, `alpn_token()`,
  `requires_lowercase_fields()` or `has_trailers()` to `HttpVersion`, and do
  not put a version on `HttpRequest`. Each is constant over what this build
  speaks or has no reader, and a version is a property of the connection.
- Do not clamp a version, and do not spell the option `max_version`. Nothing
  is negotiated; a version this build cannot speak is refused by name.
- Do not accept `h2c`. RFC 9113 removed the `Upgrade` path and says a client
  MUST NOT send the token.
- Do not add a trailers accessor. ureq parses the chunked trailer part and
  discards it, so it would answer empty on every version — a lie about the
  transport rather than a seam.
- Do not drain an abandoned body without consulting `pools_connections()`. On
  HTTP/1.0 the connection closes regardless.
- **Do not write anywhere — rustdoc, module doc, `DECISIONS.md`, or the docs
  page — that this module speaks HTTP/2 or HTTP/3, or that Amazon S3 does.**
  The ALPN probes behind any such claim in the research that produced this
  prompt measured a proxying egress gateway, which answers `ALPN: h2` for
  every host including `neverssl.com`. Any version claim about a real origin
  is re-measured from an unproxied network, checking the certificate issuer,
  before it is written down. The same applies to any h2-versus-h1.1 timing.
- Do not bound a caller's value read with `MAX_DOCUMENT`, and do not spell that bound with `Read::take`. `take` truncates in silence; a bound that does not fail is not a bound.
- Do not hand back a shared `Wire`, and do not inherit ureq's idle-pool defaults. `Wire::shared()` shares the agent; the stats, the budget and the jitter counter are per `Wire`, and three idle connections per host at a 15 s idle age make piece 13's connection counts true only of the in-process origin.
- Do not gate the abandoned-body drain on the *answer's* version. ureq decides poolability from the request's version, `Connection: close`, and whether the body's framing states an end.
- Do not add `range`, `if-range`, `accept-encoding`, `content-length` or `host` to the shared refused-name constant. `holder::object` sets two of them, and piece 1's byte-for-byte gate is what stops you.
- Do not let a caller's header and a generated one both reach the wire, and do not apply `HttpAuth` at send time. `Builder::header` appends rather than replaces, and a credential resolved after `Prepared::redirected` undoes the drop on every hop.
- Do not look a header up, or remove one, by its bare field name. `canonical_http_lookup_key` adds no prefix, so the call answers `None`, mutates nothing, and compiles.
- Do not read an inbound value with `HeaderValue::to_str`. It refuses every byte above 0x7e, which `validate_http_header_value` accepts on the way out.
- Do not use `Error::conflict` for a disagreement whose two sides are runtime values, and do not let an `Error::remote` carry an unmasked URL. Both nouns are `&'static str`, its `Display` says "expected to create", and `remote` is the one constructor that does not mask.
- Do not treat a `HEAD` as the probe. `HttpFile` sends none; a probe is one `GET` with `Range: bytes=0-0`, and `refuse_head` is the tripwire that proves it.
- Do not read a 206's `Content-Length` as the resource's length, and do not copy a 206 body before checking where the answer says it starts.
- Do not build a `Range` from a zero-length window, and do not saturate an unbounded one. `length - 1` underflows; the remainder is `bytes={offset}-`.
- Do not reserve a peer's stated `Content-Length`, and do not grow a stage past `MAX_DOCUMENT`. `Vec::resize` commits every page `try_reserve` only reserved.
- Do not resume a body a `GET` did not produce, and do not re-derive a resume's target from the session's map. The plan is pinned at `into_stream`.
- Do not treat `ErrorKind::Interrupted` as a cut — retry the read in place — and do not reset the failure counter on a single byte. A byte is not progress.
- Do not send an `If-Range` a weak `ETag` or a same-second `Last-Modified` backs, and do not claim the 200-is-a-conflict rule guards a request that carried no conditional.
- Do not do wire I/O in a destructor while `std::thread::panicking()`, do not run the retry ladder from `Drop`, and do not discard a `Drop` publish's failure. Log it.
- Do not use `PoisonError::into_inner` on the stage lock. The staged bytes are the only copy of a value nothing else holds.
- Do not infer a moved prefix from a `Location` that does not preserve whole trailing segments, do not match one with `starts_with`, and do not let a session resolution change an answer: a `404` or `410` on a re-pointed request is re-issued once at the URL the caller named.
- Do not resolve a `Location` with `Url::from_str` and a `joinpath` fallback. A scheme-less reference parses as a `file:` URL, so the fallback never runs and `//evil.example/x` keeps the attacker's authority.
- Do not rewrite a body-carrying `PUT` or `PATCH` to `GET` on a 303, and do not carry a cached validator, a `content-length` or a `content-type` across a hop that dropped the body.
- Do not answer `capacity()` with `0` on an unstaged handle, and do not let `file_exists()` answer `false` for a probe that failed rather than answered.
- Do not probe in `HttpPath`. Its role is the URL's, and every byte method forwards to one retained `HttpFile`.
- Do not write a binding. Rust-only is complete work here; document it as such.
- Do not land two pieces in one commit.
