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
import tracemalloc
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import yggdryl

try:
    import requests
except ImportError:  # pragma: no cover - requests is optional
    requests = None

try:
    import httpx  # the modern, HTTP/2-capable client
except ImportError:  # pragma: no cover - httpx is optional
    httpx = None


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
        hx = httpx.Client() if httpx is not None else None

        def fmt_ms(t):
            return f"{t * 1e3:.3f} ms" if t is not None else "—"

        def fmt_mibps(t):
            return f"{mibps(len(big), t):.0f} MiB/s" if t is not None else "—"

        yg_t = timed(lambda: yg.get(base + "/small").content, 400)
        rq_t = timed(lambda: rq.get(base + "/small").content, 400)
        hx_t = timed(lambda: hx.get(base + "/small").content, 400) if hx else None
        rows.append(
            (
                "GET small body (latency)",
                fmt_ms(yg_t),
                fmt_ms(rq_t),
                fmt_ms(hx_t),
                f"{rq_t / yg_t:.2f}×",
            )
        )

        yg_t = timed(lambda: yg.get(base + "/big").content, 20)
        rq_t = timed(lambda: rq.get(base + "/big").content, 20)
        hx_t = timed(lambda: hx.get(base + "/big").content, 20) if hx else None
        rows.append(
            (
                "GET 8 MiB body (throughput)",
                fmt_mibps(yg_t),
                fmt_mibps(rq_t),
                fmt_mibps(hx_t),
                f"{rq_t / yg_t:.2f}×",
            )
        )
        if hx:
            hx.close()
    finally:
        server.shutdown()
    table("HTTP — yggdryl vs requests / httpx (same in-process server)",
          ["workload", "yggdryl", "requests", "httpx", "vs requests"], rows)


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
    for name in ("zstd", "snappy", "brotli"):
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


# ----------------------------------------------------------------------- temporal
def temporal_bench():
    """yggdryl's calendar/time types vs the stdlib ``datetime`` (+ ``zoneinfo``).

    The timing table is an honest side-by-side of the comparable operations; the
    capability table is where yggdryl is *more complete and safer* — a built-in
    duration parser, nanosecond precision, and DST conversion with no external tz
    database (``zoneinfo`` raises if the OS ships no tzdata)."""
    from datetime import datetime

    try:
        from zoneinfo import ZoneInfo
    except ImportError:  # pragma: no cover
        ZoneInfo = None

    def fmt(t):
        return f"{t * 1e6:.3f} µs"

    iso = "2024-07-01T12:00:00+00:00"
    rows = []

    yg_t = timed(lambda: yggdryl.DateTime.from_str(iso), 50_000)
    py_t = timed(lambda: datetime.fromisoformat(iso), 50_000)
    rows.append(("parse ISO datetime", fmt(yg_t), fmt(py_t), f"{py_t / yg_t:.2f}×"))

    ydt = yggdryl.DateTime.from_str(iso)
    pdt = datetime.fromisoformat(iso)
    yg_t = timed(lambda: str(ydt), 50_000)
    py_t = timed(lambda: pdt.isoformat(), 50_000)
    rows.append(("format datetime", fmt(yg_t), fmt(py_t), f"{py_t / yg_t:.2f}×"))

    # zoneinfo needs OS tzdata (or the `tzdata` package); skip the row if absent
    # rather than crash — the very gap the capability table calls out.
    ny = None
    if ZoneInfo is not None:
        try:
            ny = ZoneInfo("America/New_York")
        except Exception:
            ny = None
    if ny is not None:
        yg_t = timed(lambda: ydt.to_timezone("America/New_York").hour, 50_000)
        py_t = timed(lambda: pdt.astimezone(ny).hour, 50_000)
        rows.append(("convert UTC→New York (DST-aware)", fmt(yg_t), fmt(py_t), f"{py_t / yg_t:.2f}×"))

    table(
        "Temporal — yggdryl vs stdlib datetime / zoneinfo (per-call, lower is better)",
        ["workload", "yggdryl", "datetime", "vs datetime"],
        rows,
    )

    # Capabilities: what each library can do at all (the completeness / safety story).
    table(
        "Temporal — capability & safety (where the FFI cost buys real coverage)",
        ["capability", "yggdryl", "stdlib datetime"],
        [
            ("parse a duration string (`1h30m`, `PT15M`)", "✓", "✗ (no parser)"),
            ("sub-microsecond (nanosecond) precision", "✓", "✗ (µs only)"),
            ("DST conversion with no OS tz database", "✓ (embedded)", "✗ (needs tzdata)"),
            ("flexible parse (`2024/07/01` slash form)", "✓", "✗ (dashes only)"),
            ("reject an invalid calendar date", "✓ raises", "✓ raises"),
        ],
    )


def peak_mib(fn):
    """Peak Python-heap allocation (MiB) for one call to `fn`, via tracemalloc."""
    fn()  # warm any import/codepath caches out of the measurement
    tracemalloc.start()
    fn()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    return peak / (1024 * 1024)


# ------------------------------------------------------------------------- memory
def memory_bench():
    """Peak heap held while producing the same result. yggdryl does the bulk work
    in Rust and hands one buffer across the FFI, so the host heap stays flat where
    the pure-Python path balloons with intermediate objects."""
    payload = (
        "col_a,col_b,col_c\n"
        + "".join(f"{i},{i * 2},value_{i % 97}\n" for i in range(150_000))
    ).encode()
    gz = yggdryl.Compression.from_str("gzip")
    packed_yg = gz.compress(payload)
    packed_py = gzip.compress(payload)

    rows = [
        (
            "gzip compress (peak heap)",
            f"{peak_mib(lambda: gz.compress(payload)):.2f} MiB",
            f"{peak_mib(lambda: gzip.compress(payload)):.2f} MiB",
        ),
        (
            "gzip decompress (peak heap)",
            f"{peak_mib(lambda: gz.decompress(packed_yg)):.2f} MiB",
            f"{peak_mib(lambda: gzip.decompress(packed_py)):.2f} MiB",
        ),
    ]
    table(
        "Memory — peak host-heap for the same result",
        ["workload", "yggdryl", "Python stdlib"],
        rows,
    )
    print(
        "\n_The deeper memory win is **streaming**: in the Rust core an `HttpStream` "
        "reads a multi-gigabyte object in a bounded 4 MiB window and `pread`s a "
        "footer with one Range request (`cargo bench -p yggdryl-http`), never "
        "holding the whole body — see the Rust-core numbers below._"
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
    temporal_bench()
    memory_bench()
