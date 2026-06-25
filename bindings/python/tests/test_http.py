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
        # Echo a custom request header back so the client can assert on it.
        echo = self.headers.get("X-Echo", "")
        self._reply(200, b"hello world", extra={"X-Echo-Back": echo})

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
