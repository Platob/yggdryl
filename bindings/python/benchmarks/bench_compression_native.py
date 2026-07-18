"""yggdryl.compression vs the Python-native codecs (runs in ~2-4 s).

The point of this benchmark: yggdryl's `gzip`/`zlib` run on flate2's **`zlib-rs`** backend — a
pure-Rust port of the SIMD-tuned zlib-ng — while Python's stdlib `gzip`/`zlib` link the C
`zlib`. At a matched compression level the codecs emit byte-compatible streams, so the only
difference measured here is raw throughput, and yggdryl should come out **ahead** on
compress and decompress. `lzma` is the same `liblzma` on both sides (a wash — kept as a
control), and `zstd` is reported only when a stdlib/third-party zstd is importable.

Each row reports MiB/s for both sides and the **speedup** (`yggdryl / native`, >1.0 = faster).
A cross-round-trip check (compress on one side, decompress on the other) proves the streams are
interchangeable, so the speedup is a like-for-like win, not a format difference.

Build the extension in RELEASE first — a debug build makes the timings meaningless:

    maturin develop --release
    python bindings/python/benchmarks/bench_compression_native.py
"""

import gzip
import lzma
import time
import zlib

from yggdryl.compression import Gzip, Lzma, Zlib

# A realistic, semi-compressible corpus: repeated JSON-ish records with varying fields, so the
# codecs do real work (not a trivial run-length win, not incompressible noise).
_RECORD = (
    b'{"id":%d,"ts":"2026-07-18T09:%02d:%02dZ","user":"user_%d",'
    b'"event":"checkout","amount":%d.%02d,"currency":"EUR","ok":true,'
    b'"tags":["retail","eu","priority"],"note":"the quick brown fox jumps over the lazy dog"}\n'
)
CORPUS = b"".join(
    _RECORD % (i, i % 60, (i * 7) % 60, i % 997, i % 500, i % 100) for i in range(24_000)
)
MIB = len(CORPUS) / (1024 * 1024)


def timed(op, iters):
    """Best MiB/s of `iters` runs of `op` over the corpus (min time = least noise)."""
    op()  # warm up
    best = float("inf")
    for _ in range(iters):
        start = time.perf_counter()
        op()
        best = min(best, time.perf_counter() - start)
    return MIB / best


def report(label, native_mibs, ygg_mibs):
    speedup = ygg_mibs / native_mibs
    flag = "faster" if speedup >= 1.0 else "SLOWER"
    print(
        f"  {label:<22} native {native_mibs:8.1f}   yggdryl {ygg_mibs:8.1f} MiB/s"
        f"   ->  {speedup:5.2f}x  {flag}"
    )
    return speedup


def bench_deflate(name, native_mod, ygg_codec, level, iters):
    """gzip / zlib: matched level, both directions, plus a cross round-trip interchange check."""
    native_c = native_mod.compress(CORPUS, level)
    ygg_c = ygg_codec.compress(CORPUS)
    # The streams are interchange-compatible: decompress each side's output on the other side.
    assert native_mod.decompress(bytes(ygg_c)) == CORPUS, f"{name}: native cannot read yggdryl"
    assert bytes(ygg_codec.decompress(native_c)) == CORPUS, f"{name}: yggdryl cannot read native"

    print(f"{name} (level {level}) — {MIB:.2f} MiB, ratio {len(ygg_c) / len(CORPUS):.3f}")
    c_native = timed(lambda: native_mod.compress(CORPUS, level), iters)
    c_ygg = timed(lambda: ygg_codec.compress(CORPUS), iters)
    up = report("compress", c_native, c_ygg)
    d_native = timed(lambda: native_mod.decompress(native_c), iters)
    d_ygg = timed(lambda: ygg_codec.decompress(native_c), iters)
    down = report("decompress", d_native, d_ygg)
    return up, down


def main():
    print(f"compression: yggdryl (flate2/zlib-rs) vs Python-native — corpus {MIB:.2f} MiB\n")

    comp, decomp = [], []
    for name, mod, codec in (("gzip", gzip, Gzip(6)), ("zlib", zlib, Zlib(6))):
        up, down = bench_deflate(name, mod, codec, 6, iters=40)
        comp.append(up)
        decomp.append(down)
        print()

    # lzma is liblzma on both sides — a control, expected ~1.0x (kept for completeness).
    lzma_c = lzma.compress(CORPUS, preset=6)
    ygg_lzma = Lzma(6)
    print(f"lzma (preset 6, control — same liblzma) — ratio {len(lzma_c) / len(CORPUS):.3f}")
    report("compress", timed(lambda: lzma.compress(CORPUS, preset=6), 8),
           timed(lambda: ygg_lzma.compress(CORPUS), 8))
    report("decompress", timed(lambda: lzma.decompress(lzma_c), 20),
           timed(lambda: ygg_lzma.decompress(lzma_c), 20))
    print()

    # zstd only if the running Python has one (stdlib `compression.zstd` in 3.14+, else skip).
    try:
        from compression import zstd  # noqa: PLC0415

        from yggdryl.compression import Zstd

        zc = zstd.compress(CORPUS, 3)
        yz = Zstd(3)
        assert bytes(yz.decompress(zc)) == CORPUS
        print(f"zstd (level 3) — ratio {len(zc) / len(CORPUS):.3f}")
        report("compress", timed(lambda: zstd.compress(CORPUS, 3), 40),
               timed(lambda: yz.compress(CORPUS), 40))
        report("decompress", timed(lambda: zstd.decompress(zc), 60),
               timed(lambda: yz.decompress(zc), 60))
        print()
    except ImportError:
        print("zstd: no stdlib `compression.zstd` on this Python — skipped\n")

    # The headline: with the binding boundary made zero-copy (`PyBackedBytes`, not a
    # per-element `Vec<u8>` copy), yggdryl's flate2/zlib-rs **out-compresses** C zlib. Its
    # pure-Rust *inflate* still trails C zlib on decompress — reported honestly, not hidden.
    print("deflate (gzip+zlib) vs Python-native zlib:")
    print(f"  compress   mean {sum(comp) / len(comp):.2f}x   (>1 = yggdryl faster)")
    print(f"  decompress mean {sum(decomp) / len(decomp):.2f}x")


if __name__ == "__main__":
    main()
