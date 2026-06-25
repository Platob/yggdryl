"""Integration test comparing yggdryl.HttpSession to Python `requests`.

Both clients hit the same in-process server; we assert their results are
identical and print a small latency comparison (run with ``pytest -s`` to see
the timings). Skipped if `requests` is not installed.
"""

import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

import yggdryl

requests = pytest.importorskip("requests")


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"  # keep-alive, so `requests` reuses connections

    def log_message(self, *args):
        pass

    def do_GET(self):
        body = b"hello from the comparison server"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@pytest.fixture
def base_url():
    server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{port}"
    finally:
        server.shutdown()


def test_results_match_requests(base_url):
    yg = yggdryl.HttpSession()

    yg_get = yg.get(base_url + "/")
    rq_get = requests.get(base_url + "/")
    assert yg_get.status == rq_get.status_code
    assert yg_get.text() == rq_get.text
    assert bytes(yg_get.content) == rq_get.content
    assert yg_get.content_type == rq_get.headers["Content-Type"]

    payload = b"round-trip-body-bytes"
    yg_post = yg.post(base_url + "/echo", payload)
    rq_post = requests.post(base_url + "/echo", data=payload)
    assert bytes(yg_post.content) == rq_post.content == payload


def test_latency_comparison(base_url):
    count = 100

    yg = yggdryl.HttpSession()
    start = time.perf_counter()
    for _ in range(count):
        yg.get(base_url + "/").content
    yg_elapsed = time.perf_counter() - start

    rq = requests.Session()
    start = time.perf_counter()
    for _ in range(count):
        rq.get(base_url + "/").content
    rq_elapsed = time.perf_counter() - start

    print(
        f"\n{count} sequential GETs — yggdryl: {yg_elapsed * 1e3:.1f} ms, "
        f"requests: {rq_elapsed * 1e3:.1f} ms "
        f"({rq_elapsed / yg_elapsed:.2f}x)"
    )
    # Both must complete; yggdryl should be in the same ballpark (or faster).
    assert yg_elapsed > 0 and rq_elapsed > 0
