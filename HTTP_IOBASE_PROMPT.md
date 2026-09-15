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
  `Ok(0)` for a connection that died with bytes outstanding. The worst case
  for one drain at defaults is `1 + (max_attempts − 1) × max_attempts = 7`
  requests, stated in the module doc and asserted.
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
  and `DEFAULT_FETCH_BYTE_SIZE` (`:62`, 1 MiB) are deliberately different
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
  doc. Piece 9 argues in that voice.
- `rust/src/holder/object/tests/server.rs:1-14` — the leaf-file `#[path]`
  include shape the in-process origin copies.
- `rust/src/holder/counted.rs:75-140` — `Call`, the canonical list of the
  surface a backend implements, and the spellings piece 13's tallies use.
- `DECISIONS.md` — the last decision is 39. These eleven are 40 to 50, in the
  file's own format (`**Rule.**` / `**Why.**` / `**Written in:**` /
  `**Fixtures:**`).
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
whole-body read, which fixes `object`'s unbounded `read_to_end` in
`open_range`, `get_all` and `open_reader_range` as a side effect.

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

**What moves.** New `rust/src/holder/wire/version.rs`; `holder/wire/mod.rs`
(`pub use version::HttpVersion;`, re-exported from `holder/http/mod.rs`);
`holder/wire/send.rs` (`wire_version` and its inverse, the crate's only
`ureq::http::Version`); `holder/wire/stats.rs` (`requests_by_version`,
`requests_on`); `holder/wire/body.rs` (the drain gate on `pools_connections`);
`holder/http/options.rs` and `properties.rs` (`with_version`, the
`version`/`http_version`/`http-version` intake spellings through the existing
`canonical()` recipe at `object/properties.rs:608`, and the refusal);
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
   `ssl_verify`, `verify_ssl` and their `-`/`.` variants through the existing
   `canonical()` recipe (`holder/object/properties.rs:608`) and `flag()`
   parser (`:625`). `from_environment` reads `YGGDRYL_TLS_VERIFY` only, never
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
back apart** — splitting would be re-parsing past the boundary. A non-UTF-8
header value is skipped, the way `client.rs:905-910` skips it.

`Content-Type` and `Content-Encoding` are read by
`MediaType::from_content_headers` (`media_type.rs:160`) and nowhere else. The
reverse projection already exists at `HttpFieldMut::set_media_type`
(`types/protocol/http.rs:420`) and a write's headers are built by that rule
rather than a second one.

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
  and *labelled decoded*. The setter is the only place that catches it.
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
   for one drain at defaults is `1 + (max_attempts − 1) × max_attempts = 7`
   requests. **That number is in the module doc and in an assertion**; the
   object backend leaves it unstated.

`If-Range` is the gap this closes, and this module refuses to port the object
backend's silence. The first answer's `ETag` — or, absent one, its
`Last-Modified` — is the stream's validator. Every resume sends
`If-Range: <validator>` beside the `Range`. **206** means it held. **200**
means it did not and the origin is sending the whole changed resource from
byte zero: that is `Error::conflict("the generation the transfer started
from", "a changed resource", url)` surfaced through `Read::read` as
`std::io::Error::other` — not a skip-and-continue, and not silence. **404, 410
or 416 on a resume** is the same conflict (410/404 as `Error::absent`). This is
the sharpest departure: on an **initial** open those statuses still read as
emptiness, because absence is emptiness per the laziness contract; on a
**resume** they mean the resource vanished under a transfer that already
delivered bytes, and reporting that as a clean end hands the caller a truncated
value it cannot detect. A **weak** validator cannot back `If-Range`, so it is
not sent and the strict 200-is-a-conflict rule carries the guard. With **no**
validator at all, `Consistency::Strict` makes a cut a failure rather than an
unguarded re-open; `Relaxed` resumes unconditional and the module doc states
that a mid-read rewrite is then undetectable.

`HttpStream::drop` drains up to `POOLED_DRAIN_LIMIT` (1 MiB) so an abandoned
body returns its connection, and — the one fix over `Pooled` — drains up to
`DRAIN_LIMIT` (64 KiB) when the origin stated no length, instead of giving up.
A chunked body of unknown size is usually short. The drain stays bounded; an
unbounded drain would be worse than burning the connection.

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

- a **status** trigger retries for every method, because the origin answered
  and by answering said nothing was done;
- a **transport** trigger retries `GET`, `HEAD`, `PUT`, `DELETE`, `OPTIONS`;
- for `POST` and `PATCH` a transport trigger retries **only** on
  `ConnectionFailed` and `HostNotFound`, where nothing reached the origin.

The gate lives in `holder::http`'s `Prepared`, **never in `budget.rs`**, or it
changes how `object` retries a `PUT`. `holder::object` sets
`WireRequest::retry_regardless = true` and keeps its behaviour and its
accounting assertions byte-for-byte. A `Payload::Streamed` is not replayable,
so its attempt count is forced to 1 and `may_retry` is never consulted.

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
   the status is settled; a resume's re-open goes to the resolved URL and a 3xx
   there is a failure, not another hop. A redirect between range requests would
   silently change what is being read.
7. `with_max_redirects(0)` restores the object backend's behaviour exactly.

`object`'s one-shot region redirect stays in `object`: it is not a `Location`
to follow, it corrects a signing region, and `redirect.rs` knows nothing about it.

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
| construction, `url`, `media_type`, `set_media_type`, `is_container`, `is_atomic`, `is_tabular`, `ls`, `child_by_path`, `parent`, `partitions`, `reserve`, `capacity` (unstaged) | 0 | 0 |
| `truncate(n)` where `n == size()` | 0 | 0 |
| `size`, `mtime`, `kind`, `is_empty`, `file_exists` | 1 probe | 0 |
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
- every error body and every whole `send()` is bounded at `MAX_DOCUMENT`.

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

**What moves.** `rust/tests/iobase_calls.rs` (a `mod http` block),
`rust/tests/allocations.rs`, `holder/http/tests/accounting.rs`,
`rust/benchmarks/holder.rs`, `rust/benchmarks/holder/http/{mod,bytes,access,resume}.rs`,
`docs/holder/backends/http.md`, `docs/benchmarks.md` (one row).

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

- Following a 3xx inside `holder::object`. Piece 9 changes `holder::http` only;
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
- Do not write a binding. Rust-only is complete work here; document it as such.
- Do not land two pieces in one commit.
