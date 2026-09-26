"""``yggdryl.http``: the requests-shaped client and the server, over the core.

Pins ``python/src/http.rs`` and ``python/yggdryl/http.py``. The client is
driven against ``http.server.ThreadingHTTPServer`` and the server against
``http.client`` and ``urllib.request`` - implementations this crate did not
write - so a request or an answer both sides agree on is one the wire
actually carries. Refusals come first: each is a boundary a caller meets
before anything is sent.
"""

from __future__ import annotations

import concurrent.futures
import datetime
import http.client
import io
import json
import pathlib
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterator
from typing import Any
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

import yggdryl
from yggdryl import IOBase, Scalar
from yggdryl.holder import LocalFolder
from yggdryl.http import (
    Client,
    Headers,
    Pages,
    Request,
    Response,
    Server,
    Session,
    Stream,
)
from yggdryl.media import Parquet

TEXT = "héllo\nwörld\r\nlast"
BIG = bytes(range(256)) * 1024
TABLE = pa.table({"id": [1, 2, 3], "name": ["a", "b", "c"]})


def _parquet_bytes() -> bytes:
    sink = io.BytesIO()
    pq.write_table(TABLE, sink)
    return sink.getvalue()


PARQUET = _parquet_bytes()


class Origin(BaseHTTPRequestHandler):
    """An outside origin: every answer framed by ``Content-Length``."""

    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        return

    def _answer(
        self,
        status: int,
        body: bytes = b"",
        content_type: str = "application/json",
        headers: dict[str, str] | None = None,
    ) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _json(self, value: object, status: int = 200, headers: dict[str, str] | None = None) -> None:
        self._answer(status, json.dumps(value).encode(), headers=headers)

    def _echo(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        parsed = urllib.parse.urlsplit(self.path)
        self._json(
            {
                "method": self.command,
                "path": parsed.path,
                "query": urllib.parse.parse_qs(parsed.query),
                "headers": {name.lower(): value for name, value in self.headers.items()},
                "body": body.decode("utf-8", "replace"),
            }
        )

    def _ranged(self, payload: bytes, content_type: str) -> None:
        span = self.headers.get("Range")
        if span is None:
            self._answer(200, payload, content_type, {"Accept-Ranges": "bytes"})
            return
        first, _, last = span.removeprefix("bytes=").partition("-")
        start = int(first) if first else len(payload) - int(last)
        end = int(last) if first and last else len(payload) - 1
        end = min(end, len(payload) - 1)
        self._answer(
            206,
            payload[start : end + 1],
            content_type,
            {
                "Accept-Ranges": "bytes",
                "Content-Range": f"bytes {start}-{end}/{len(payload)}",
            },
        )

    def do_HEAD(self) -> None:  # noqa: N802
        self.do_GET()

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(parsed.query)
        path = parsed.path
        if path == "/echo":
            self._echo()
        elif path == "/text":
            self._answer(200, TEXT.encode("utf-8"), "text/plain; charset=utf-8")
        elif path == "/redirect":
            self._answer(302, b"", headers={"Location": "/echo?from=redirect"})
        elif path == "/cookie/set":
            self._json({"set": True}, headers={"Set-Cookie": "sid=abc; Path=/"})
        elif path == "/missing":
            self._answer(404, b"no such thing\nsecond line", "text/plain")
        elif path == "/slow":
            time.sleep(0.2)
            self._json({"slept": 0.2})
        elif path == "/big":
            self._answer(200, BIG, "application/octet-stream")
        elif path == "/linked":
            page = int(query.get("page", ["1"])[0])
            headers = {}
            if page < 3:
                headers["Link"] = f'</linked?page={page + 1}>; rel="next"'
            self._json({"data": [{"id": page * 10 + n} for n in range(2)]}, headers=headers)
        elif path == "/items":
            cursor = query.get("cursor", [None])[0]
            pages = {None: ("c2", [1, 2]), "c2": ("c3", [3, 4]), "c3": (None, [5])}
            following, ids = pages[cursor]
            self._json({"items": [{"id": n} for n in ids], "next_cursor": following})
        elif path == "/rows.json":
            self._json([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}])
        elif path == "/data.parquet":
            self._ranged(PARQUET, "application/vnd.apache.parquet")
        else:
            self._answer(404, b"not found", "text/plain")

    def do_POST(self) -> None:  # noqa: N802
        self._echo()

    def do_PUT(self) -> None:  # noqa: N802
        self._echo()

    def do_PATCH(self) -> None:  # noqa: N802
        self._echo()

    def do_DELETE(self) -> None:  # noqa: N802
        self._answer(204, b"", "text/plain")


class OriginServer(ThreadingHTTPServer):
    # socketserver's backlog of five drops the SYN of a sixth simultaneous
    # connect, which the kernel then retries a second later.
    request_queue_size = 64


@pytest.fixture(scope="module")
def origin() -> Iterator[str]:
    server = OriginServer(("127.0.0.1", 0), Origin)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()


@pytest.fixture
def session(origin: str) -> Session:
    return Session(origin)


class TestRefusals:
    """Each refusal is met before a byte is sent."""

    def test_a_scheme_that_is_not_http_is_refused(self) -> None:
        with pytest.raises(ValueError, match="http"):
            Session().get("ftp://example.com/file")

    def test_a_relative_url_without_a_base_is_refused(self) -> None:
        with pytest.raises(ValueError):
            Session().get("relative/path")

    def test_one_body_only(self, session: Session) -> None:
        with pytest.raises(ValueError, match="data= and json="):
            session.post("/echo", b"raw", json={"a": 1})

    def test_a_query_value_must_be_text_or_a_number(self, session: Session) -> None:
        with pytest.raises(TypeError, match="params"):
            session.get("/echo", params={"when": object()})

    def test_auth_is_a_pair_or_a_token(self) -> None:
        with pytest.raises(TypeError, match="auth"):
            Session(auth=42)  # type: ignore[arg-type]

    def test_a_negative_timeout_is_refused(self) -> None:
        with pytest.raises(ValueError, match="timeout"):
            Session(timeout=-1.0)

    def test_an_option_that_does_not_read_is_refused(self) -> None:
        with pytest.raises(ValueError, match="timeout"):
            Session(options={"timeout": "soon"})

    def test_an_unknown_method_is_refused(self, origin: str) -> None:
        with pytest.raises(ValueError):
            Request("BREW", f"{origin}/echo")

    def test_a_header_name_must_be_a_token(self) -> None:
        with pytest.raises(ValueError):
            Headers({"bad name": "x"})

    def test_a_missing_header_is_a_key_error(self) -> None:
        with pytest.raises(KeyError):
            Headers()["content-type"]

    def test_a_status_outside_the_grammar_is_refused(self) -> None:
        with pytest.raises(ValueError):
            Response(99)

    def test_a_chunk_of_nothing_is_refused(self) -> None:
        with pytest.raises(ValueError, match="chunk_size"):
            Response(200, body=b"abc").iter_content(0)

    def test_a_fault_is_spelled_one_of_four_ways(self) -> None:
        with Server.bind() as server:
            with pytest.raises(ValueError, match="fault"):
                server.inject("/x", ("explode", 1))  # type: ignore[arg-type]

    def test_only_a_handle_mounts(self) -> None:
        with Server.bind() as server:
            with pytest.raises(TypeError, match="IOBase"):
                server.mount("/", "not a handle")  # type: ignore[arg-type]

    def test_a_shut_down_server_says_so(self) -> None:
        server = Server.bind()
        server.shutdown()
        server.shutdown()
        with pytest.raises(ValueError, match="shut down"):
            _ = server.port


class TestRequestsShape:
    """The requests vocabulary, each a redirect into the core."""

    def test_get_with_params_and_headers(self, session: Session, origin: str) -> None:
        response = session.get(
            "/echo", params={"q": ["a b", 7], "skip": None}, headers={"X-Trace": "t-1"}
        )
        assert isinstance(response, Response)
        assert response.status_code == 200
        assert response.ok and response.reason == "OK"
        assert str(response.url) == f"{origin}/echo?q=a%20b&q=7"
        document = response.json()
        assert document["query"] == {"q": ["a b", "7"]}
        assert document["headers"]["x-trace"] == "t-1"
        assert document["headers"]["user-agent"].startswith("yggdryl/")
        # The headers answer ignoring case, as a mapping.
        assert response.headers["CONTENT-TYPE"] == "application/json"
        assert "content-length" in response.headers
        assert response.headers.content_length == len(response.content)
        assert json.loads(response.text) == document
        assert isinstance(response.elapsed, datetime.timedelta)
        assert response.version == "HTTP/1.1"
        assert isinstance(response.scalar(), Scalar)

    def test_text_reads_the_declared_charset(self, session: Session) -> None:
        response = session.get("/text")
        assert response.encoding == "utf-8"
        assert response.text == TEXT
        assert list(response.iter_lines()) == [
            "héllo".encode(),
            "wörld".encode(),
            b"last",
        ]

    def test_bodies_as_json_form_and_bytes(self, session: Session) -> None:
        sent = session.post("/echo", json={"id": 7, "tags": ["a"]}).json()
        assert sent["method"] == "POST"
        assert json.loads(sent["body"]) == {"id": 7, "tags": ["a"]}
        assert sent["headers"]["content-type"] == "application/json"

        form = session.put("/echo", {"name": "a b", "n": 1}).json()
        assert form["method"] == "PUT"
        assert urllib.parse.parse_qs(form["body"]) == {"name": ["a b"], "n": ["1"]}

        raw = session.patch("/echo", b"\x00\x01raw").json()
        assert raw["method"] == "PATCH" and raw["body"] == "\x00\x01raw"

        assert session.delete("/echo").status_code == 204
        head = session.head("/big")
        assert head.status_code == 200 and head.content == b""

    def test_a_request_is_prepared_then_sent(self, session: Session, origin: str) -> None:
        request = Request(
            "post", f"{origin}/echo", json=[1, 2], headers={"x-a": "b"}, session=session
        )
        assert request.method == "POST"
        assert request.body == b"[1,2]"
        assert request.headers["x-a"] == "b"
        assert isinstance(request.session, Session)
        assert request.send().json()["body"] == "[1,2]"
        assert session.send(request).json()["method"] == "POST"

    def test_redirects_are_followed_and_kept_in_the_history(
        self, session: Session, origin: str
    ) -> None:
        response = session.get("/redirect")
        assert response.json()["query"] == {"from": ["redirect"]}
        assert [(code, str(url)) for code, url in response.history] == [
            (302, f"{origin}/redirect")
        ]
        assert session.stats["redirects"] == 1

        held = session.get("/redirect", allow_redirects=False)
        assert held.status_code == 302 and held.is_redirect
        assert held.headers.location == "/echo?from=redirect"

    def test_cookies_ride_on_the_next_request(self, session: Session) -> None:
        first = session.get("/cookie/set")
        assert first.cookies == {"sid": "abc"}
        assert session.cookies == {"sid": "abc"}
        assert session.get("/echo").json()["headers"]["cookie"] == "sid=abc"

    def test_a_refusing_status_raises_on_request(self, session: Session) -> None:
        response = session.get("/missing")
        assert response.status_code == 404
        assert not response.ok
        with pytest.raises(ValueError, match="404") as failure:
            response.raise_for_status()
        assert "no such thing" in str(failure.value)
        assert session.get("/echo").raise_for_status().ok

    def test_auth_rides_as_the_authorization_header(self, origin: str) -> None:
        basic = Session(origin, auth=("ann", "s3cret")).get("/echo").json()
        assert basic["headers"]["authorization"] == "Basic YW5uOnMzY3JldA=="
        bearer = Session(origin).get("/echo", auth="t0ken").json()
        assert bearer["headers"]["authorization"] == "Bearer t0ken"

    def test_session_defaults(self, origin: str) -> None:
        session = Session(
            origin,
            headers={"X-Default": "1"},
            timeout=datetime.timedelta(seconds=5),
            options={"max_attempts": 2, "concurrency": 3},
        )
        assert session.timeout == datetime.timedelta(seconds=5)
        assert session.concurrency == 3
        assert session.headers["x-default"] == "1"
        assert str(session.url) == origin
        assert session.get("/echo").json()["headers"]["x-default"] == "1"
        assert session.stats["gets"] == 1

    def test_a_client_shares_its_pool(self, origin: str) -> None:
        client = Client({"timeout": "5s"})
        session = client.session(origin)
        assert session.get("/echo").ok
        assert client.stats["requests"] == 1

    def test_the_module_verbs_use_the_default_session(self, origin: str) -> None:
        assert yggdryl.http.get(f"{origin}/echo", params={"a": 1}).json()["query"] == {
            "a": ["1"]
        }
        assert yggdryl.http.post(f"{origin}/echo", "text").json()["body"] == "text"
        assert isinstance(yggdryl.http.session(), Session)


class TestStreaming:
    """A streamed body is read off the wire as it is asked for."""

    def test_iter_content_reads_the_body_in_chunks(self, session: Session) -> None:
        response = session.get("/big", stream=True)
        assert response.kind == "file"
        chunks = list(response.iter_content(64 * 1024))
        assert [len(chunk) for chunk in chunks][:1] == [64 * 1024]
        assert b"".join(chunks) == BIG

    def test_a_held_body_iterates_the_same_bytes(self, session: Session) -> None:
        response = session.get("/big")
        assert b"".join(response.iter_content(10_000)) == BIG
        assert response.content == BIG

    def test_a_closed_response_stands_alone_and_reopens_at_its_cursor(
        self, origin: str
    ) -> None:
        # No session is kept: the response carries its request and session.
        response = Session(origin).get("/data.parquet", stream=True)
        assert response.opened
        assert response.read_range_bytes(0, 100) == PARQUET[:100]
        response.close()
        assert not response.opened
        # The next read re-opens at the cursor: one ranged GET.
        assert response.read_range_bytes(100, len(PARQUET) - 100) == PARQUET[100:]
        assert response.request.session.stats["resumes"] == 1

    def test_into_stream_hands_the_live_body_over(self, session: Session) -> None:
        response = session.stream("/big")
        stream = response.into_stream()
        assert isinstance(stream, Stream)
        assert stream.total == len(BIG)
        assert stream.read(100) == BIG[:100]
        assert stream.delivered == 100
        assert stream.read() == BIG[100:]
        assert stream.resumes == 0
        assert stream.headers.content_length == len(BIG)
        with pytest.raises(ValueError, match="consumed"):
            _ = response.status_code


class TestConcurrency:
    """The interpreter is released while a request is on the wire."""

    def test_threads_share_one_session(self, session: Session) -> None:
        started = time.perf_counter()
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            responses = list(pool.map(lambda _: session.get("/slow"), range(8)))
        wall = time.perf_counter() - started
        assert [response.json() for response in responses] == [{"slept": 0.2}] * 8
        # Eight serial requests cost 1.6 s; side by side they cost one.
        assert wall < 8 * 0.2 / 2

    def test_send_all_answers_in_request_order(self, origin: str) -> None:
        session = Session(origin, options={"concurrency": 8})
        requests = [
            Request("GET", f"{origin}/slow", session=session) for _ in range(7)
        ] + [Request("GET", f"{origin}/echo", params={"n": 1}, session=session)]
        started = time.perf_counter()
        responses = list(session.send_all(requests))
        wall = time.perf_counter() - started
        assert [response.status_code for response in responses] == [200] * 8
        assert responses[-1].json()["query"] == {"n": ["1"]}
        assert wall < 7 * 0.2 / 2

    def test_send_all_takes_a_concurrency_and_keeps_the_jar(self, origin: str) -> None:
        session = Session(origin, options={"concurrency": 1})
        requests = [Request("GET", f"{origin}/slow", session=session) for _ in range(4)]
        requests.append(Request("GET", f"{origin}/cookie/set", session=session))
        started = time.perf_counter()
        responses = list(session.send_all(requests, concurrency=5))
        wall = time.perf_counter() - started
        assert [response.status_code for response in responses] == [200] * 5
        assert wall < 4 * 0.2 / 2
        assert session.cookies == {"sid": "abc"}
        assert session.concurrency == 1
        # Zero threads send nothing, so zero is read as one, as the options do.
        assert list(session.send_all([], concurrency=0)) == []

    def test_send_all_reads_prepared_requests_urls_and_keyword_specs(
        self, origin: str, session: Session
    ) -> None:
        answers = session.send_all(
            [
                Request("GET", f"{origin}/echo", params={"from": "prepared"}, session=session),
                "/echo?from=url",
                {"method": "POST", "url": "/echo", "json": {"n": 1}},
                {"url": "/echo", "params": {"from": "spec"}, "headers": {"X-Probe": "yes"}},
            ],
            concurrency=4,
        )
        echoed = [answer.json() for answer in answers]
        assert [echo["method"] for echo in echoed] == ["GET", "GET", "POST", "GET"]
        assert echoed[0]["query"] == {"from": ["prepared"]}
        assert echoed[1]["query"] == {"from": ["url"]}
        assert json.loads(echoed[2]["body"]) == {"n": 1}
        assert echoed[3]["query"] == {"from": ["spec"]}

    def test_send_all_streams_over_an_endless_source(self, session: Session) -> None:
        pulled = 0

        def urls() -> Iterator[str]:
            nonlocal pulled
            while True:
                pulled += 1
                yield "/echo"

        answers = session.send_all(urls(), concurrency=2)
        first = [next(answers).status_code for _ in range(3)]
        assert first == [200, 200, 200]
        # Pulled only as far as the requests in flight, never the whole source.
        assert 3 <= pulled <= 6

    def test_a_spec_that_is_no_request_raises_where_the_walk_reaches_it(
        self, session: Session
    ) -> None:
        answers = session.send_all(["/echo", {"url": "/echo", "verb": "GET"}])
        assert next(answers).status_code == 200
        with pytest.raises(TypeError, match="verb"):
            next(answers)


class TestPages:
    """A paginated API is one Response per page, and one table."""

    def test_a_link_header_walks_every_page(self, session: Session) -> None:
        pages = session.pages("/linked")
        assert isinstance(pages, Pages)
        bodies = [page.json() for page in pages]
        assert [row["id"] for body in bodies for row in body["data"]] == [
            10,
            11,
            20,
            21,
            30,
            31,
        ]

    def test_a_cursor_in_the_body_walks_every_page(self, session: Session) -> None:
        table = session.pages("/items").into_arrow_reader().read_all()
        assert table.column("id").to_pylist() == [1, 2, 3, 4, 5]

    def test_pages_read_as_a_serie_reader(self, session: Session) -> None:
        reader = session.pages("/linked", records="data").read_arrow()
        rows = [row for serie in reader for row in serie.as_py()]
        assert [row["id"] for row in rows] == [10, 11, 20, 21, 30, 31]

    def test_the_walk_is_handed_over_once(self, session: Session) -> None:
        pages = session.pages("/items")
        pages.into_arrow_reader()
        with pytest.raises(ValueError, match="already"):
            pages.read_arrow()

    def test_next_is_the_request_for_the_following_page(
        self, session: Session, origin: str
    ) -> None:
        first = session.get("/linked", headers={"x-page": "yes"})
        following = first.next
        assert isinstance(following, Request)
        assert str(following.url) == f"{origin}/linked?page=2"
        assert following.headers["x-page"] == "yes"
        assert following.send().json()["data"][0]["id"] == 20
        assert session.get("/linked", params={"page": 3}).next is None
        assert first.links["next"]["url"] == "/linked?page=2"
        assert first.headers.next_link == "/linked?page=2"


class TestIOBase:
    """An http location is a handle, and its body reads as records."""

    def test_a_json_body_reads_as_a_value_and_as_records(self, origin: str) -> None:
        handle = IOBase(f"{origin}/rows.json")
        assert isinstance(handle, Request)
        assert handle.read_scalar() == [
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
        ]
        # A structured text document is the one record column its rows parse
        # into: `read_arrow` is the record read that takes it.
        rows = [row for serie in handle.read_arrow() for row in serie.as_py()]
        assert rows == [{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]

    def test_a_parquet_body_reads_by_range(self, origin: str) -> None:
        handle = IOBase(f"{origin}/data.parquet")
        assert isinstance(handle, Parquet)
        assert handle.read_arrow_reader().read_all().equals(TABLE)
        assert isinstance(handle.into_handle(), Request)

    def test_a_session_resolves_children(self, origin: str) -> None:
        session = Session(f"{origin}/")
        child = session / "rows.json"
        assert isinstance(child, Request)
        assert json.loads(child.read_bytes()) == [
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
        ]

    def test_a_response_is_its_body(self, session: Session) -> None:
        response = session.get("/rows.json")
        assert response.read_bytes() == response.content
        assert str(response.media_type) == "application/json"


class TestHeadersAndResponses:
    """What a caller or a handler states by hand."""

    def test_headers_are_a_case_insensitive_mapping(self) -> None:
        headers = Headers(
            [
                ("Content-Type", "text/csv; charset=windows-1252"),
                ("Vary", "Accept"),
                ("Vary", "Accept-Encoding"),
                ("Set-Cookie", "a=1"),
                ("Set-Cookie", "b=2"),
                ("ETag", 'W/"v1"'),
                ("Last-Modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
                ("Content-Range", "bytes 0-9/100"),
                ("Content-Encoding", "gzip"),
            ]
        )
        assert headers["vary"] == "Accept, Accept-Encoding"
        assert headers.get_all("VARY") == ["Accept", "Accept-Encoding"]
        assert headers.get("absent") is None and headers.get("absent", "x") == "x"
        assert headers.set_cookies == ["a=1", "b=2"]
        assert headers.charset == "windows-1252"
        assert str(headers.mime_type) == "text/csv"
        assert headers.content_encoding == ["gzip"]
        assert headers.etag == 'W/"v1"'
        assert headers.content_range == (0, 9, 100)
        assert headers.last_modified == datetime.datetime(
            1994, 11, 6, 8, 49, 37, tzinfo=datetime.timezone.utc
        )
        assert list(headers) == headers.keys() == sorted(headers.keys())
        assert dict(headers.items())["etag"] == 'W/"v1"'
        assert len(headers) == 7
        assert headers == Headers(headers.items())
        assert hash(headers) == hash(Headers(headers.items()))
        assert json.loads(headers.into_json())["vary"] == "Accept, Accept-Encoding"
        # Membership reads like a mapping's: a key of another type is absent.
        assert "ETag" in headers and "absent" not in headers
        assert 5 not in headers and None not in headers

    def test_a_response_built_by_hand(self) -> None:
        response = Response(201, {"X-Id": "7"}, json={"created": True})
        assert response.status_code == 201 and response.reason == "Created"
        assert response.headers["content-type"] == "application/json"
        assert response.json() == {"created": True}
        text = Response(body="plain")
        assert text.headers["content-type"] == "text/plain; charset=utf-8"
        assert text.text == "plain"


@pytest.fixture
def server() -> Iterator[Server]:
    with Server.bind() as bound:
        yield bound


class TestServer:
    """The server, driven by clients this crate did not write."""

    def test_a_mounted_folder_answers_get_range_put_and_delete(
        self, server: Server, tmp_path: pathlib.Path
    ) -> None:
        (tmp_path / "a.txt").write_bytes(b"0123456789")
        server.mount("/files", LocalFolder(tmp_path))
        base = f"http://127.0.0.1:{server.port}"

        with urllib.request.urlopen(f"{base}/files/a.txt") as answer:
            assert answer.status == 200
            assert answer.read() == b"0123456789"
            assert answer.headers["Accept-Ranges"] == "bytes"

        connection = http.client.HTTPConnection("127.0.0.1", server.port)
        connection.request("GET", "/files/a.txt", headers={"Range": "bytes=2-4"})
        ranged = connection.getresponse()
        assert ranged.status == 206
        assert ranged.read() == b"234"
        assert ranged.getheader("Content-Range") == "bytes 2-4/10"

        connection.request("PUT", "/files/b.txt", body=b"new bytes")
        created = connection.getresponse()
        created.read()
        assert created.status == 201
        assert (tmp_path / "b.txt").read_bytes() == b"new bytes"
        connection.request("GET", "/files/b.txt")
        assert connection.getresponse().read() == b"new bytes"

        connection.request("DELETE", "/files/b.txt")
        deleted = connection.getresponse()
        deleted.read()
        assert deleted.status == 204
        connection.request("GET", "/files/b.txt")
        missing = connection.getresponse()
        missing.read()
        assert missing.status == 404
        connection.close()

    def test_routes_answer_a_tuple_or_a_response(self, server: Server) -> None:
        def hello(request: Request) -> tuple[int, dict[str, str], str]:
            return 200, {"x-method": request.method}, f"hi {request.body.decode()}"

        server.route("/hello", hello, method="POST")
        server.route("/made", lambda request: Response(202, json={"url": str(request.url)}))
        server.route("/broken", lambda request: 1 / 0)  # type: ignore[arg-type, return-value]
        server.respond("/fixed", 200, {"content-type": "text/plain"}, b"same")
        base = f"http://127.0.0.1:{server.port}"

        posted = urllib.request.Request(f"{base}/hello", data=b"there", method="POST")
        with urllib.request.urlopen(posted) as answer:
            assert answer.read() == b"hi there"
            assert answer.headers["x-method"] == "POST"
        with urllib.request.urlopen(f"{base}/made") as answer:
            assert answer.status == 202
            assert json.loads(answer.read())["url"] == f"{base}/made"
        with pytest.raises(urllib.error.HTTPError) as failure:
            urllib.request.urlopen(f"{base}/broken")
        assert failure.value.code == 500
        for _ in range(2):
            with urllib.request.urlopen(f"{base}/fixed") as answer:
                assert answer.read() == b"same"

        assert server.request_count == 5
        assert [(entry["method"], entry["path"], entry["status"]) for entry in server.requests] == [
            ("POST", "/hello", 200),
            ("GET", "/made", 202),
            ("GET", "/broken", 500),
            ("GET", "/fixed", 200),
            ("GET", "/fixed", 200),
        ]
        server.clear_requests()
        assert server.request_count == 0 and server.requests == []

    def test_a_route_answers_the_crates_own_session(self, server: Server) -> None:
        # The handler needs the interpreter on the server's thread while the
        # caller waits on the answer: the wait has to release it.
        server.route("/ping", lambda request: (200, None, "pong"))
        assert Session(str(server.url)).get("ping").text == "pong"

    def test_faults(self, server: Server) -> None:
        server.respond("/flaky", 200, None, b"fine")
        base = f"http://127.0.0.1:{server.port}"

        server.inject("/flaky", ("refuse", 503, 0))
        with pytest.raises(urllib.error.HTTPError) as failure:
            urllib.request.urlopen(f"{base}/flaky")
        assert failure.value.code == 503

        server.inject("/flaky", "close_before_answer")
        connection = http.client.HTTPConnection("127.0.0.1", server.port)
        connection.request("GET", "/flaky")
        with pytest.raises(http.client.RemoteDisconnected):
            connection.getresponse()
        connection.close()

        server.inject("/flaky", ("delay", 0.05))
        with urllib.request.urlopen(f"{base}/flaky") as answer:
            assert answer.read() == b"fine"

        # The crate's own client retries a refusal it may repeat.
        session = Session(str(server.url))
        server.inject("/flaky", ("refuse", 503, 0))
        assert session.get("flaky").content == b"fine"
        assert session.stats["retries"] == 1

    def test_a_mounted_session_serves_its_origin_under_its_own_headers(
        self, server: Server
    ) -> None:
        with Server.bind() as upstream:
            upstream.route(
                "/data/probe",
                lambda request: (200, None, request.headers.get("x-probe", "none")),
            )
            # The session's defaults and its container role cross the mount.
            session = Session(str(upstream.url_of("data/")), headers={"x-probe": "kept"})
            server.mount("/proxy", session)
            with urllib.request.urlopen(f"http://127.0.0.1:{server.port}/proxy/probe") as answer:
                assert answer.read() == b"kept"

    def test_the_server_names_where_it_listens(self, server: Server) -> None:
        assert str(server.url) == f"http://127.0.0.1:{server.port}/"
        assert str(server.url_of("a/b")) == f"http://127.0.0.1:{server.port}/a/b"
        assert server.unmount("/nothing") is False
        assert server.unroute("/nothing") is False


@pytest.fixture(scope="module")
def requests_package() -> Any:
    # The reference client every requests-shaped spelling here is held to;
    # CI installs it, so this never skips there.
    return pytest.importorskip("requests")


class TestRequestsInterop:
    """`requests` and this client against one origin, and each against the other's server."""

    @pytest.mark.parametrize(
        ("method", "path", "keywords"),
        [
            ("GET", "/echo", {"params": {"a": "1", "b": ["2", "3"]}, "headers": {"X-Probe": "yes"}}),
            ("GET", "/text", {}),
            ("GET", "/redirect", {}),
            ("GET", "/missing", {}),
            ("POST", "/echo", {"json": {"n": 1, "s": "é"}}),
            ("POST", "/echo", {"data": {"k": "v w"}}),
            ("PUT", "/echo", {"data": b"raw bytes"}),
        ],
    )
    def test_one_origin_answers_both_clients_alike(
        self,
        requests_package: Any,
        origin: str,
        method: str,
        path: str,
        keywords: dict[str, Any],
    ) -> None:
        theirs = requests_package.request(method, f"{origin}{path}", timeout=10, **keywords)
        ours = Session(origin).request(method, path, **keywords)
        assert ours.status_code == theirs.status_code
        assert ours.ok == theirs.ok
        assert ours.reason == theirs.reason
        assert str(ours.url) == theirs.url
        assert [code for code, _ in ours.history] == [hop.status_code for hop in theirs.history]
        if not theirs.url.split("?")[0].endswith("/echo"):
            assert ours.content == theirs.content
            assert ours.text == theirs.text
            return
        echoed, expected = ours.json(), theirs.json()
        # Each client names itself and its codings, and spells a JSON body in
        # its own whitespace and escapes; everything else the origin saw -
        # method, path, query, the caller's headers, the document sent - is
        # the same.
        for echo in (echoed, expected):
            headers = echo["headers"]
            for name in ("user-agent", "accept-encoding", "accept", "connection", "content-length"):
                headers.pop(name, None)
            if headers.get("content-type") == "application/json":
                echo["body"] = json.loads(echo["body"])
        assert echoed == expected

    def test_cookies_and_links_read_alike(self, requests_package: Any, origin: str) -> None:
        theirs = requests_package.get(f"{origin}/cookie/set", timeout=10)
        ours = Session(origin).get("/cookie/set")
        assert ours.cookies == dict(theirs.cookies)
        theirs = requests_package.get(f"{origin}/linked", timeout=10)
        ours = Session(origin).get("/linked")
        assert ours.links["next"]["url"] == theirs.links["next"]["url"]

    def test_requests_reads_a_mounted_folder_with_ranges_and_validators(
        self, requests_package: Any, server: Server, tmp_path: pathlib.Path
    ) -> None:
        (tmp_path / "a.bin").write_bytes(BIG)
        server.mount("/files", LocalFolder(tmp_path))
        url = f"http://127.0.0.1:{server.port}/files/a.bin"
        with requests_package.Session() as client:
            whole = client.get(url, timeout=10)
            assert whole.status_code == 200
            assert whole.content == BIG
            etag = whole.headers["ETag"]
            ranged = client.get(url, headers={"Range": "bytes=100-199"}, timeout=10)
            assert ranged.status_code == 206
            assert ranged.content == BIG[100:200]
            assert ranged.headers["Content-Range"] == f"bytes 100-199/{len(BIG)}"
            unchanged = client.get(url, headers={"If-None-Match": etag}, timeout=10)
            assert unchanged.status_code == 304
            streamed = client.get(url, stream=True, timeout=10)
            assert b"".join(streamed.iter_content(4096)) == BIG
            created = client.put(f"http://127.0.0.1:{server.port}/files/b.txt", data=b"put", timeout=10)
            assert created.status_code == 201
            assert (tmp_path / "b.txt").read_bytes() == b"put"

    def test_the_environment_proxy_is_read_per_request(
        self, origin: str, tmp_path: pathlib.Path
    ) -> None:
        # A tunnelling server stands in for the forward proxy the environment
        # names; a subprocess carries that environment, so this process's own
        # is never changed under the suites beside it.
        import subprocess
        import sys

        proxy = Server.bind(tunnel=True)
        script = (
            "import sys\n"
            "from yggdryl import http\n"
            "answer = http.get(sys.argv[1] + '/text')\n"
            "print(answer.status_code, answer.text)\n"
        )
        port = origin.rsplit(":", 1)[1]
        environment = {
            key: value
            for key, value in __import__("os").environ.items()
            if "proxy" not in key.lower()
        }
        through = dict(environment, http_proxy=f"http://127.0.0.1:{proxy.port}")
        run = subprocess.run(
            [sys.executable, "-c", script, origin],
            env=through,
            capture_output=True,
            text=True,
            timeout=60,
            check=True,
        )
        assert run.stdout.split(maxsplit=1)[0] == "200"
        assert [(entry["method"], entry["target"]) for entry in proxy.requests] == [
            ("CONNECT", f"127.0.0.1:{port}")
        ]

        proxy.clear_requests()
        bypassed = dict(through, NO_PROXY=f"127.0.0.1:{port}")
        run = subprocess.run(
            [sys.executable, "-c", script, origin],
            env=bypassed,
            capture_output=True,
            text=True,
            timeout=60,
            check=True,
        )
        assert run.stdout.split(maxsplit=1)[0] == "200"
        assert proxy.requests == []
        proxy.shutdown()

    def test_a_netrc_entry_is_sent_as_requests_sends_it(
        self, requests_package: Any, server: Server, tmp_path: pathlib.Path
    ) -> None:
        # Both clients read the file `NETRC` names in a subprocess, so this
        # process's own environment and home directory are never consulted.
        import os
        import subprocess
        import sys

        server.respond("/private", 200, {"content-type": "text/plain"}, b"ok")
        netrc = tmp_path / "netrc"
        netrc.write_text("machine 127.0.0.1 login alice password s3cret\n")
        script = (
            "import sys, requests\n"
            "from yggdryl import http\n"
            "requests.get(sys.argv[1], timeout=10)\n"
            "http.get(sys.argv[1])\n"
        )
        environment = {
            key: value for key, value in os.environ.items() if "proxy" not in key.lower()
        }
        environment["NETRC"] = str(netrc)
        subprocess.run(
            [sys.executable, "-c", script, f"http://127.0.0.1:{server.port}/private"],
            env=environment,
            capture_output=True,
            text=True,
            timeout=60,
            check=True,
        )
        sent = [dict(entry["headers"]).get("authorization") for entry in server.requests]
        assert sent == ["Basic YWxpY2U6czNjcmV0"] * 2
