#!/usr/bin/env python3
"""Benchmark the **yggdryl** Python bindings against the Python stalwarts on
identical workloads — `requests` (HTTP), the stdlib `gzip` (compression) and
`io.BytesIO` (byte IO) — and print a markdown results table.

Every comparison runs the *same* high-level operation through both libraries
against the same in-process server / in-memory payload, so the numbers are
apples-to-apples ("same code, two backends"). Run with the built wheel and
`requests` installed:

    (cd bindings/python && maturin develop) && python3 benchmarks/compare.py
"""

import gzip
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import yggdryl

try:
    import requests
except ImportError:  # pragma: no cover - requests is optional
    requests = None


def timed(fn, iters):
    """Mean seconds per call over `iters` runs, after a short warm-up."""
    for _ in range(max(1, iters // 10)):
        fn()
    start = time.perf_counter()
    for _ in range(iters):
        fn()
    return (time.perf_counter() - start) / iters


def mibps(nbytes, secs):
    return nbytes / (1024 * 1024) / secs


def table(title, header, rows):
    print(f"\n### {title}\n")
    print("| " + " | ".join(header) + " |")
    print("|" + "|".join("---" for _ in header) + "|")
    for row in rows:
        print("| " + " | ".join(str(c) for c in row) + " |")


# --------------------------------------------------------------------------- HTTP
def http_bench():
    if requests is None:
        print("\n(requests not installed — skipping HTTP comparison)")
        return
    big = bytes((i % 251) for i in range(8 * 1024 * 1024))
    small = b"small-response-body"

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"  # keep-alive so both clients pool connections

        def setup(self):
            super().setup()
            # Set TCP_NODELAY so neither client eats the ~40 ms delayed-ACK stall on
            # localhost — a fair fight on processing speed, not Nagle behaviour.
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

        def log_message(self, *args):
            pass

        def do_GET(self):
            body = big if self.path == "/big" else small
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_address[1]}"
    rows = []
    try:
        yg = yggdryl.HttpSession()
        rq = requests.Session()

        yg_t = timed(lambda: yg.get(base + "/small").content, 400)
        rq_t = timed(lambda: rq.get(base + "/small").content, 400)
        rows.append(
            (
                "GET small body (latency)",
                f"{yg_t * 1e3:.3f} ms",
                f"{rq_t * 1e3:.3f} ms",
                f"{rq_t / yg_t:.2f}×",
            )
        )

        n = len(big)
        yg_t = timed(lambda: yg.get(base + "/big").content, 20)
        rq_t = timed(lambda: rq.get(base + "/big").content, 20)
        rows.append(
            (
                "GET 8 MiB body (throughput)",
                f"{mibps(n, yg_t):.0f} MiB/s",
                f"{mibps(n, rq_t):.0f} MiB/s",
                f"{rq_t / yg_t:.2f}×",
            )
        )
    finally:
        server.shutdown()
    table("HTTP — yggdryl vs requests (same in-process server)",
          ["workload", "yggdryl", "requests", "speedup"], rows)


# -------------------------------------------------------------------- compression
def compression_bench():
    payload = (
        "col_a,col_b,col_c\n"
        + "".join(f"{i},{i * 2},value_{i % 97}\n" for i in range(150_000))
    ).encode()
    n = len(payload)
    rows = []

    gz = yggdryl.Compression.from_str("gzip")
    yg_t = timed(lambda: gz.compress(payload), 10)
    py_t = timed(lambda: gzip.compress(payload), 10)
    rows.append(
        (
            "gzip compress",
            f"{mibps(n, yg_t):.0f} MiB/s",
            f"{mibps(n, py_t):.0f} MiB/s",
            f"{py_t / yg_t:.2f}×",
        )
    )
    packed_yg = gz.compress(payload)
    packed_py = gzip.compress(payload)
    yg_t = timed(lambda: gz.decompress(packed_yg), 50)
    py_t = timed(lambda: gzip.decompress(packed_py), 50)
    rows.append(
        (
            "gzip decompress",
            f"{mibps(n, yg_t):.0f} MiB/s",
            f"{mibps(n, py_t):.0f} MiB/s",
            f"{py_t / yg_t:.2f}×",
        )
    )
    table(
        f"Compression — yggdryl vs stdlib gzip ({n // 1024} KiB CSV payload)",
        ["workload", "yggdryl", "stdlib gzip", "speedup"],
        rows,
    )

    # Codecs the standard library does not ship at all.
    extra = []
    for name in ("zstd", "snappy"):
        codec = yggdryl.Compression.from_str(name)
        if not codec.is_available:
            continue
        packed = codec.compress(payload)
        ct = timed(lambda c=codec: c.compress(payload), 10)
        dt = timed(lambda c=codec, p=packed: c.decompress(p), 50)
        extra.append(
            (
                name,
                f"{mibps(n, ct):.0f} MiB/s",
                f"{mibps(n, dt):.0f} MiB/s",
                f"{n / len(packed):.2f}×",
            )
        )
    if extra:
        table(
            "Bonus — codecs the Python stdlib has no equivalent for",
            ["codec", "compress", "decompress", "ratio"],
            extra,
        )


if __name__ == "__main__":
    print("# yggdryl vs Python — same code, measured\n")
    print(
        "_The thin Python binding wins where bulk work runs in Rust in one call "
        "(an HTTP download, a whole-buffer compress). For tiny per-call operations "
        "the FFI crossing dominates, so use the bulk / streaming methods — and see "
        "the Rust-core micro-benchmarks (`cargo bench`) for the library's true "
        "ceiling, free of any FFI._"
    )
    http_bench()
    compression_bench()
