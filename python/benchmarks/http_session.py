"""Time ``yggdryl.http`` against ``requests`` on the same origin.

Run against a release extension::

    python -m maturin develop --release -m python/Cargo.toml
    python python/benchmarks/http_session.py

The origin is ``yggdryl.http.Server`` in a process of its own - native
threads, so it never competes with either client for the interpreter and
answers fast enough that the clients are what is measured - over HTTP/1.1
keep-alive on loopback. The baseline is ``requests.Session``, one per
thread where threads are used, as its documentation asks. Every case reads
the same bytes through both clients first and refuses to time clients that
disagree. The rows:

- ``get small``: one ``GET`` of a 1 KiB JSON document, the body read;
- ``get + json``: one ``GET`` of ``--rows`` records, parsed into Python values;
- ``download``: one ``GET`` of ``--megabytes`` MiB, the body read whole;
- ``stream``: the same body read in 1 MiB chunks;
- ``fan-out``: ``--requests`` small ``GET`` s on ``--threads`` workers -
  ``Session.send_all`` against a thread pool over ``requests``;
- ``slow fan-out``: 64 ``GET`` s that each wait 10 ms at the origin, on
  ``--threads`` workers.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import multiprocessing
import statistics
import threading
import time
from collections.abc import Callable
from typing import Any

from yggdryl import http as ygg_http


def _serve(rows: int, megabytes: int, ports: Any, stop: Any) -> None:
    small = json.dumps({"id": 1, "items": list(range(200))}).encode()
    document = json.dumps(
        [{"id": index, "symbol": f"S{index % 97}", "price": index * 0.25} for index in range(rows)]
    ).encode()
    big = bytes(range(256)) * (megabytes * 4096)
    with ygg_http.Server.bind(recording=False) as server:
        server.respond("/small", 200, {"content-type": "application/json"}, small)
        server.respond("/rows.json", 200, {"content-type": "application/json"}, document)
        server.respond("/big", 200, {"content-type": "application/octet-stream"}, big)
        server.respond("/slow", 200, {"content-type": "application/json"}, small)
        server.inject("/slow", ("delay", 0.01), times=0)
        ports.put(server.port)
        stop.wait()


def _median_ms(operation: Callable[[], object], repeat: int, iterations: int) -> float:
    operation()
    samples = []
    for _ in range(repeat):
        started = time.perf_counter()
        for _ in range(iterations):
            operation()
        samples.append((time.perf_counter() - started) * 1_000 / iterations)
    return statistics.median(samples)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=500, help="sequential requests per sample")
    parser.add_argument("--repeat", type=int, default=5, help="samples per case; the median is reported")
    parser.add_argument("--rows", type=int, default=1_000)
    parser.add_argument("--megabytes", type=int, default=32)
    parser.add_argument("--requests", type=int, default=1_000, help="requests per fan-out")
    parser.add_argument("--threads", type=int, default=8)
    arguments = parser.parse_args()
    if min(arguments.iterations, arguments.repeat, arguments.megabytes, arguments.requests, arguments.threads) <= 0:
        parser.error("every count must be positive")

    import requests

    context = multiprocessing.get_context("spawn")
    ports: Any = context.Queue()
    stop: Any = context.Event()
    origin = context.Process(
        target=_serve, args=(arguments.rows, arguments.megabytes, ports, stop), daemon=True
    )
    origin.start()
    try:
        base = f"http://127.0.0.1:{ports.get(timeout=30)}"
        ours = ygg_http.Session(base, options={"concurrency": arguments.threads})
        theirs = requests.Session()
        local = threading.local()

        def their_session() -> Any:
            if not hasattr(local, "session"):
                local.session = requests.Session()
            return local.session

        for path in ("/small", "/rows.json", "/big"):
            if ours.get(path).content != theirs.get(f"{base}{path}").content:
                raise RuntimeError(f"the two clients read different bodies at {path}")

        def their_fan_out(path: str, count: int) -> Callable[[], None]:
            def run() -> None:
                with concurrent.futures.ThreadPoolExecutor(arguments.threads) as pool:
                    for answer in pool.map(
                        lambda _: their_session().get(f"{base}{path}").content, range(count)
                    ):
                        assert answer
            return run

        def our_fan_out(path: str, count: int) -> Callable[[], None]:
            def run() -> None:
                for answer in ours.send_all([path] * count):
                    assert answer.content
            return run

        chunk = 1 << 20
        cases: list[tuple[str, Callable[[], object], Callable[[], object], int]] = [
            (
                "get small",
                lambda: ours.get("/small").content,
                lambda: theirs.get(f"{base}/small").content,
                arguments.iterations,
            ),
            (
                "get + json",
                lambda: ours.get("/rows.json").json(),
                lambda: theirs.get(f"{base}/rows.json").json(),
                max(1, arguments.iterations // 10),
            ),
            (
                "download",
                lambda: ours.get("/big").content,
                lambda: theirs.get(f"{base}/big").content,
                1,
            ),
            (
                "stream",
                lambda: sum(len(part) for part in ours.get("/big", stream=True).iter_content(chunk)),
                lambda: sum(
                    len(part) for part in theirs.get(f"{base}/big", stream=True).iter_content(chunk)
                ),
                1,
            ),
            (
                f"fan-out {arguments.requests}",
                our_fan_out("/small", arguments.requests),
                their_fan_out("/small", arguments.requests),
                1,
            ),
            ("slow fan-out 64", our_fan_out("/slow", 64), their_fan_out("/slow", 64), 1),
        ]
        print(f"{arguments.threads} threads, {arguments.rows} rows, {arguments.megabytes} MiB")
        print(f"{'case':18s} {'yggdryl':>12s} {'requests':>12s} {'speedup':>8s}")
        for name, mine, reference, iterations in cases:
            mine_ms = _median_ms(mine, arguments.repeat, iterations)
            reference_ms = _median_ms(reference, arguments.repeat, iterations)
            print(
                f"{name:18s} {mine_ms:9.3f} ms {reference_ms:9.3f} ms "
                f"{reference_ms / mine_ms:7.2f}x"
            )
    finally:
        stop.set()
        origin.join(timeout=10)
        if origin.is_alive():
            origin.terminate()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
