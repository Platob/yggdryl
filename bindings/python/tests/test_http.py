"""Tests for yggdryl.HttpSession / HttpResponse against a localhost server.

Hermetic: a stdlib http.server runs in a background thread, so no network is
touched. Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

import yggdryl


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # silence the default stderr logging
        pass

    def _reply(self, status, body=b"", content_type="text/plain", extra=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for key, value in (extra or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/missing":
            return self._reply(404, b"nope")
        if self.path == "/brotli":
            # A Brotli-compressed JSON body, advertised via Content-Encoding: br.
            body = b'{"msg":"brotli over the wire","n":7}'
            packed = yggdryl.Compression("br").compress(body)
            return self._reply(
                200,
                packed,
                content_type="application/json",
                extra={"Content-Encoding": "br"},
            )
        # Echo a custom request header (and any Authorization) back so the client
        # can assert on it.
        echo = self.headers.get("X-Echo", "")
        auth = self.headers.get("Authorization", "")
        self._reply(
            200, b"hello world", extra={"X-Echo-Back": echo, "X-Auth-Back": auth}
        )

    def _echo_body(self, status):
        length = int(self.headers.get("Content-Length", 0))
        data = self.rfile.read(length)
        self._reply(status, data, content_type="application/octet-stream")

    def do_POST(self):
        self._echo_body(201)

    def do_PUT(self):
        self._echo_body(200)

    def do_DELETE(self):
        self._reply(204)


@pytest.fixture
def base_url():
    httpd = HTTPServer(("127.0.0.1", 0), _Handler)
    port = httpd.server_address[1]
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{port}"
    finally:
        httpd.shutdown()


def test_get_status_text_and_headers(base_url):
    session = yggdryl.HttpSession(user_agent="yggdryl-test")
    response = session.get(base_url + "/")
    assert response.status == 200
    assert response.ok is True
    assert response.text() == "hello world"
    assert response.content == b"hello world"
    assert response.content_type == "text/plain"
    assert response.url.startswith("http://127.0.0.1")


def test_post_echoes_body(base_url):
    session = yggdryl.HttpSession()
    response = session.post(base_url + "/submit", b"ping-payload")
    assert response.status == 201
    assert response.content == b"ping-payload"


def test_default_and_request_headers(base_url):
    session = yggdryl.HttpSession(headers={"X-Echo": "from-default"})
    assert session.get(base_url + "/").header("x-echo-back") == "from-default"

    response = session.request("GET", base_url + "/", headers={"X-Echo": "from-request"})
    assert response.header("x-echo-back") == "from-request"


def test_basic_and_bearer_auth(base_url):
    # `basic_auth=(user, pass)` sends a default HTTP Basic Authorization header.
    session = yggdryl.HttpSession(basic_auth=("Aladdin", "open sesame"))
    assert (
        session.get(base_url + "/").header("x-auth-back")
        == "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
    )
    # `bearer_auth=token` sends a Bearer token instead.
    session = yggdryl.HttpSession(bearer_auth="tok-123")
    assert session.get(base_url + "/").header("x-auth-back") == "Bearer tok-123"


def test_404_and_raise_for_status(base_url):
    session = yggdryl.HttpSession()
    # raise_error=False returns the 404 response instead of raising.
    response = session.request("GET", base_url + "/missing", raise_error=False)
    assert response.status == 404
    assert response.ok is False
    with pytest.raises(ValueError):
        response.raise_for_status()
    # The verb helpers raise by default.
    with pytest.raises(ValueError):
        session.get(base_url + "/missing")


def test_arbitrary_method(base_url):
    session = yggdryl.HttpSession()
    response = session.request("DELETE", base_url + "/thing")
    assert response.status == 204


def test_response_timestamps(base_url):
    session = yggdryl.HttpSession()
    response = session.get(base_url + "/")
    # The buffered convenience API drains during send, so both stamps are set:
    # dispatched first, the body fully received at or after that instant.
    assert response.sent_at > 0.0
    assert response.received_at >= response.sent_at


def test_io_body_upload_from_localpath(base_url, tmp_path):
    # Pass a LocalPath (an Io handle) as the body: it streams off disk, never
    # collected into Python — the server echoes the bytes back.
    path = str(tmp_path / "upload.bin")
    yggdryl.LocalPath(path).write(b"file-streamed-upload")
    response = yggdryl.HttpSession().put(base_url + "/up", yggdryl.LocalPath(path))
    assert response.content == b"file-streamed-upload"


def test_set_cookie_seeds_the_jar():
    session = yggdryl.HttpSession()
    session.set_cookie("http://example.com/", "sid", "abc123")
    assert session.cookies["sid"] == "abc123"


def test_module_level_verbs_use_the_shared_session(base_url):
    # The module-level verbs dispatch through the process-wide shared session,
    # like requests.get(...).
    response = yggdryl.get(base_url + "/")
    assert response.status == 200
    assert response.text() == "hello world"
    assert yggdryl.post(base_url + "/submit", b"ping").content == b"ping"
    assert yggdryl.request("DELETE", base_url + "/thing").status == 204


def test_base_url_resolves_relative_targets(base_url):
    session = yggdryl.HttpSession(base_url=base_url + "/")
    assert session.base_url == base_url + "/"
    # A relative target is joined onto the base; the echo server returns the path.
    assert session.get("path/here").header("x-echo-back") == ""
    assert session.get("path/here").status == 200
    # An absolute URL bypasses the base.
    assert session.get(base_url + "/").status == 200


def test_set_base_url_configures_the_shared_singleton(base_url):
    # Point the shared singleton at the test server, then call a module verb with
    # a relative path.
    yggdryl.set_base_url(base_url + "/")
    try:
        assert yggdryl.get("/").text() == "hello world"
    finally:
        # Reset the singleton so other tests' module verbs use absolute URLs.
        yggdryl.set_base_url("http://127.0.0.1:1")


def test_http_version_negotiation(base_url):
    # The session default is "auto"; a response reports the negotiated version,
    # which is HTTP/1.1 (the only wired transport today).
    session = yggdryl.HttpSession()
    assert session.http_version == "auto"
    response = session.get(base_url + "/")
    assert response.http_version == "HTTP/1.1"

    # A session can default to a specific version…
    pinned = yggdryl.HttpSession(http_version="2")
    assert pinned.http_version == "HTTP/2"
    # …but pinning HTTP/2 (no transport yet) raises rather than downgrading.
    with pytest.raises(ValueError):
        pinned.get(base_url + "/")
    # The per-request override raises the same way.
    with pytest.raises(ValueError):
        session.request("GET", base_url + "/", http_version="3")


# A self-signed certificate (PEM), used to exercise the CA installer.
_CA_FIXTURE = b"""-----BEGIN CERTIFICATE-----
MIIBQjCB9aADAgECAhQuzAiSQcN9LmU+b23fQ4OnlJr4nzAFBgMrZXAwFzEVMBMG
A1UEAwwMeWdnZHJ5bC10ZXN0MB4XDTI2MDYyNTE4MDczOFoXDTM2MDYyMjE4MDcz
OFowFzEVMBMGA1UEAwwMeWdnZHJ5bC10ZXN0MCowBQYDK2VwAyEAxQDw21VJgXZq
oYc6cXjHtCyGS+Xhu4OzPcRqzez2t8yjUzBRMB0GA1UdDgQWBBS8VDtYTuBsTuVe
Cc9+2uF8BKgWHzAfBgNVHSMEGDAWgBS8VDtYTuBsTuVeCc9+2uF8BKgWHzAPBgNV
HRMBAf8EBTADAQH/MAUGAytlcANBAKXArPIcky5wHp+VgiKw954G3+1I1PQzmpfJ
r9/00T2PpD5GwhdzsrH/liNZug/eMW7w38c0zU0A05lLhgZEIAM=
-----END CERTIFICATE-----
"""


def test_ca_cert_installer():
    # No CA installed by default; installing one (PEM) is reported.
    assert yggdryl.HttpSession().ca_cert_count == 0
    assert yggdryl.HttpSession(ca_cert=_CA_FIXTURE).ca_cert_count == 1
    # Undecodable PEM and empty input are rejected at install time.
    with pytest.raises(ValueError):
        yggdryl.HttpSession(
            ca_cert=b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----"
        )
    with pytest.raises(ValueError):
        yggdryl.HttpSession(ca_cert=b"")


def test_brotli_response_auto_decodes_with_json_and_accessors(base_url):
    session = yggdryl.HttpSession()
    response = session.get(base_url + "/brotli")
    # text/json/content are the decompressed payload; accessors report the codec.
    assert response.content_encoding == "br"
    assert response.compression == "brotli"
    assert response.mime_type == "application/json"
    # media_type combines Content-Type + Content-Encoding (inner → outer).
    assert response.media_type == ["application/json", "application/x-brotli"]
    assert response.json() == {"msg": "brotli over the wire", "n": 7}

    # The performant byte result is a yggdryl BytesIO handle — parse it in Rust with
    # no native copy. Native converters produce bytes / io.BytesIO on demand.
    import io as _io

    handle = response.io
    assert isinstance(handle, yggdryl.BytesIO)
    assert handle.json() == {"msg": "brotli over the wire", "n": 7}
    assert bytes(handle) == response.content
    native = handle.to_bytes_io()
    assert isinstance(native, _io.BytesIO)
    assert native.getvalue() == response.content


def test_read_timeout_keep_alive_and_copy(base_url):
    # The read timeout defaults to 120s and is configurable.
    assert yggdryl.HttpSession().read_timeout == 120.0
    assert yggdryl.HttpSession(read_timeout=5).read_timeout == 5.0
    # keep_alive is now a TTL in seconds; 0 closes the connection. A request still
    # succeeds whichever the value.
    session = yggdryl.HttpSession()
    assert session.request("GET", base_url + "/", keep_alive=0).status == 200
    assert session.request("GET", base_url + "/", keep_alive=30).status == 200
    # copy() is independent: configuration is carried, the original is unchanged.
    clone = yggdryl.HttpSession(read_timeout=9).copy()
    assert clone.read_timeout == 9.0


def test_verify_and_proxy_options():
    # TLS verification is on by default; it can be disabled (insecure).
    assert yggdryl.HttpSession().verify is True
    assert yggdryl.HttpSession(verify=False).verify is False
    # A proxy can be set explicitly (reported back); a bad proxy URL raises.
    proxied = yggdryl.HttpSession(proxy="http://127.0.0.1:8080")
    assert "127.0.0.1:8080" in proxied.proxy
    with pytest.raises(ValueError):
        yggdryl.HttpSession(proxy="not a url")
