"""Compare yggdryl's gzip codec against the Python standard library.

Both compress the same corpus at the same level; the script reports MB/s for each
and the speedup, so the Rust-backed `yggdryl.compression.Gzip` (flate2 /
miniz_oxide) can be weighed against stdlib `gzip` (the C `zlib`).

Build the extension in RELEASE first — a debug build is ~20x slower and the
numbers are meaningless:

    maturin develop --release
    python bindings/python/benchmarks/bench_compression.py
"""

import gzip as stdlib_gzip
import time

from yggdryl import compression

CORPUS = (b"the quick brown fox jumps over the lazy dog. " * 23_302)[: 1 << 20]
ITERS = 200
LEVELS = (1, 6, 9)


def throughput_mb_s(nbytes, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return nbytes * iters / secs / (1024 * 1024)


def main():
    print(f"gzip throughput over {len(CORPUS) // 1024} KiB, {ITERS} iters:\n")
    header = f"{'level':>5}  {'op':>7}  {'yggdryl':>10}  {'stdlib':>10}  {'speedup':>8}"
    print(header)
    print("-" * len(header))

    for level in LEVELS:
        ygg = compression.Gzip(level)
        packed = ygg.encode_byte_array(CORPUS)

        cases = (
            ("encode", lambda: ygg.encode_byte_array(CORPUS),
             lambda: stdlib_gzip.compress(CORPUS, level)),
            ("decode", lambda: ygg.decode_byte_array(packed),
             lambda: stdlib_gzip.decompress(packed)),
        )
        for op_name, ygg_op, std_op in cases:
            ygg_mb = throughput_mb_s(len(CORPUS), ITERS, ygg_op)
            std_mb = throughput_mb_s(len(CORPUS), ITERS, std_op)
            speedup = ygg_mb / std_mb
            print(
                f"{level:>5}  {op_name:>7}  {ygg_mb:>8.1f}MB  {std_mb:>8.1f}MB  {speedup:>7.2f}x"
            )


if __name__ == "__main__":
    main()
