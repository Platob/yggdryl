"""The byte codings against the standard library's, same wire, same payload.

Each pair encodes and decodes one JSON-lines payload through yggdryl's coding
and through the standard library module carrying the same format, so a row is
a like-for-like engine comparison rather than a format comparison.

Usage:
    python benchmarks/coding.py [--min-time 0.2] [--repeat 5]
"""

from __future__ import annotations

import argparse
import gzip as std_gzip
import statistics
import sys
import timeit
import zlib as std_zlib

from yggdryl.coding import gzip, zlib, zstd

PAYLOAD = b'{"id": 1234567, "venue": "XNAS", "price": "150.2500"}\n' * 20_000


def _measure(label: str, callable_, min_time: float, repeat: int) -> None:
    timer = timeit.Timer(callable_)
    number, _ = timer.autorange()
    while timer.timeit(number) < min_time:
        number *= 2
    samples = [timer.timeit(number) / number for _ in range(repeat)]
    median = statistics.median(samples)
    throughput = len(PAYLOAD) / median / (1024 * 1024)
    print(f"{label:44s} {median * 1e3:9.3f} ms {throughput:9.1f} MiB/s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()

    encoded_gzip = gzip.dumps(PAYLOAD)
    encoded_zlib = zlib.dumps(PAYLOAD)
    encoded_zstd = zstd.dumps(PAYLOAD)
    print(f"payload: {len(PAYLOAD)} bytes; gzip {len(encoded_gzip)}, "
          f"zlib {len(encoded_zlib)}, zstd {len(encoded_zstd)} encoded")

    cases = [
        ("gzip encode (yggdryl)", lambda: gzip.dumps(PAYLOAD)),
        ("gzip encode (stdlib gzip)", lambda: std_gzip.compress(PAYLOAD)),
        ("gzip decode (yggdryl)", lambda: gzip.loads(encoded_gzip)),
        ("gzip decode (stdlib gzip)", lambda: std_gzip.decompress(encoded_gzip)),
        ("zlib encode (yggdryl)", lambda: zlib.dumps(PAYLOAD)),
        ("zlib encode (stdlib zlib)", lambda: std_zlib.compress(PAYLOAD)),
        ("zlib decode (yggdryl)", lambda: zlib.loads(encoded_zlib)),
        ("zlib decode (stdlib zlib)", lambda: std_zlib.decompress(encoded_zlib)),
        ("zstd encode (yggdryl)", lambda: zstd.dumps(PAYLOAD)),
        ("zstd decode (yggdryl)", lambda: zstd.loads(encoded_zstd)),
    ]
    try:
        # Python 3.14+; sys.path puts this script's directory first, so the
        # spec is checked rather than trusting a bare import of "compression".
        import importlib.util

        if importlib.util.find_spec("compression.zstd") is None:
            raise ImportError
        from compression import zstd as std_zstd

        encoded = std_zstd.compress(PAYLOAD)
        cases.append(("zstd encode (stdlib)", lambda: std_zstd.compress(PAYLOAD)))
        cases.append(("zstd decode (stdlib)", lambda: std_zstd.decompress(encoded)))
    except (ImportError, ModuleNotFoundError):
        print("stdlib compression.zstd unavailable on this interpreter; skipped")

    print(f"Python {sys.version.split()[0]}; median of {arguments.repeat}, "
          f"min {arguments.min_time}s per sample")
    for label, callable_ in cases:
        _measure(label, callable_, arguments.min_time, arguments.repeat)
    return 0


if __name__ == "__main__":
    sys.exit(main())
