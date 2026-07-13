"""Fast time + memory benchmark for yggdryl.uri.Uri (runs in ~1-2 s).

Time: `Uri.parse` is weighed against the stdlib `urllib.parse.urlparse` over one URL
corpus; `from_path` and the `serialize_bytes` / `deserialize_bytes` round-trip are
yggdryl-only (no stdlib single call) and reported in Mops/s. Memory: `tracemalloc` reports
the Python-heap peak per parsed `Uri` and per `serialize_bytes`, validating that the thin
wrapper adds no runaway allocation.

Build the extension in RELEASE first — a debug build is meaningless for the timings (the
memory numbers are build-independent):

    maturin develop --release
    python bindings/python/benchmarks/bench_uri.py
"""

import time
import tracemalloc
from urllib.parse import urlparse

from yggdryl.uri import Uri

ITERS = 10_000

URLS = [
    "https://user:pw@example.com:8080/a/b/c.txt?q=1&x=2#frag",
    "http://example.com/",
    "https://example.com/path/to/archive.tar.gz",
    "ftp://files.example.org:21/pub/readme",
    "http://[::1]:8080/v1/status",
    "postgres://svc:secret@db.internal:5432/app?sslmode=require",
    "s3://bucket-name/keys/2026/07/13/object.parquet",
    "mailto:person@example.com",
    "file:///etc/hosts",
    "wss://stream.example.com/socket?token=abcdef#live",
]

PATHS = [
    r"C:\Users\alice\Documents\report.final.docx",
    r"D:\data\2026\input\records.tar.gz",
    r"\\server\share\team\notes.txt",
    r"src\bindings\python\lib.rs",
    "/usr/local/share/data/set.csv",
    "/var/log/app/service.log.1",
    r"E:\media\video\clip.mp4",
    "relative/dir/without/leading/slash",
]


def mops_s(items, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return items * iters / secs / 1_000_000


def peak_bytes_per(count, build):
    """Peak traced Python-heap bytes while `count` objects built by `build` are alive."""
    gc_objs = build()  # warm any one-time state, then discard
    del gc_objs
    tracemalloc.start()
    kept = build()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    del kept
    return peak / count


def main():
    print(f"yggdryl.uri.Uri - time & memory ({ITERS} iters)\n")

    # ---- time -----------------------------------------------------------------------
    def ygg_parse():
        for s in URLS:
            Uri.parse(s)

    def urllib_parse():
        for s in URLS:
            urlparse(s)

    ygg = mops_s(len(URLS), ITERS, ygg_parse)
    std = mops_s(len(URLS), ITERS, urllib_parse)
    print("time (Mops/s):")
    print(f"  {'parse (vs urllib)':<26} {ygg:7.2f}   urllib {std:6.2f}   {ygg / std:4.2f}x")

    uris = [Uri.parse(s) for s in URLS]
    encoded = [u.serialize_bytes() for u in uris]

    ops = [
        ("from_path", lambda: [Uri.from_path(p) for p in PATHS], len(PATHS)),
        ("serialize_bytes", lambda: [u.serialize_bytes() for u in uris], len(uris)),
        ("deserialize_bytes", lambda: [Uri.deserialize_bytes(b) for b in encoded], len(encoded)),
        ("round-trip", lambda: [Uri.deserialize_bytes(u.serialize_bytes()) for u in uris], len(uris)),
    ]
    for name, op, items in ops:
        print(f"  {name:<26} {mops_s(items, ITERS, op):7.2f}")

    # ---- memory ---------------------------------------------------------------------
    n = 20_000
    per_uri = peak_bytes_per(n, lambda: [Uri.parse(URLS[i % len(URLS)]) for i in range(n)])
    per_ser = peak_bytes_per(n, lambda: [uris[i % len(uris)].serialize_bytes() for i in range(n)])
    print("\nmemory (Python-heap peak, tracemalloc):")
    print(f"  {'bytes / parsed Uri':<26} {per_uri:7.1f}")
    print(f"  {'bytes / serialize_bytes':<26} {per_ser:7.1f}")


if __name__ == "__main__":
    main()
