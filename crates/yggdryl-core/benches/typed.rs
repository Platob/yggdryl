//! Time **and** memory benchmark for the [`typed`](yggdryl_core::typed) serialization layer — the
//! `Encoder`/`Decoder` bulk round-trip and the `Reduce` aggregations over a `FixedSerie`, so the
//! typed column is shown to add **no overhead** over the raw `IOBase` bulk kernels it forwards to.
//! The **allocs/op** column proves the bulk build/decode/reduce paths allocate only what the result
//! owns (a build owns its data buffer; a reduce owns nothing).
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! other benches. Run with `cargo bench -p yggdryl-core --bench typed`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::memory::Heap;
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{
    Decimal128, Decimal16, Decimal256, Decimal8, Float16, Float64, Int128, Int32, Int64, Int8,
    UInt128, F16, I256,
};
use yggdryl_core::typed::{Decoder, Encoder, FixedScalar, FixedSerie, Scalar, Serie};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn measure(items: usize, iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = items as f64 * f64::from(iters);
    (
        total / secs / 1_000_000.0,
        (a1 - a0) as f64 / f64::from(iters),
        (b1 - b0) as f64 / f64::from(iters),
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<40} {mops:8.1}    {allocs:8.2}    {bytes:9.0}");
}

fn main() {
    let iters = 2_000;
    let n = 1 << 16; // 65 536 elements

    let ints: Vec<i64> = (0..n as i64).collect();
    let floats: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let column = FixedSerie::<Int64>::from_values(&ints);
    let fcolumn = FixedSerie::<Float64>::from_values(&floats);

    println!("typed serialization — time & memory ({iters} iters over {n} elements)\n");
    println!(
        "  {:<40} {:>8}    {:>8}    {:>9}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));

    // Build: encode a whole column in one vectorized bulk write (allocates its data buffer).
    row(
        "FixedSerie::from_values (build i64)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int64>::from_values(black_box(&ints)));
        }),
    );
    // Decode: read every element back into a fresh Vec (one allocation the caller owns).
    row(
        "Serie::values (decode i64)",
        measure(n, iters, || {
            black_box(column.values());
        }),
    );
    // Reduce: sum / min / max forward to the data buffer's allocation-free Aggregate kernels.
    row(
        "Serie::sum (reduce i64)",
        measure(n, iters, || {
            black_box(column.sum().unwrap());
        }),
    );
    row(
        "Serie::min (reduce i64)",
        measure(n, iters, || {
            black_box(column.min().unwrap());
        }),
    );
    row(
        "Serie::mean (reduce f64)",
        measure(n, iters, || {
            black_box(fcolumn.mean().unwrap());
        }),
    );
    // Scalar random access — one element decode, allocation-free.
    row(
        "Serie::get (scalar decode i64)",
        measure(n, iters / 16, || {
            for i in 0..n {
                black_box(column.get(black_box(i)));
            }
        }),
    );

    // -- Encodings: bulk build by width (each `from_values` = one vectorized array write) --
    println!("\n  -- encode: from_values by element width --");
    let i8s: Vec<i8> = (0..n).map(|i| i as i8).collect();
    let i32s: Vec<i32> = (0..n as i32).collect();
    let i128s: Vec<i128> = (0..n as i128).collect();
    let u128s: Vec<u128> = (0..n as u128).collect();
    row(
        "from_values i8  (1B)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int8>::from_values(black_box(&i8s)));
        }),
    );
    row(
        "from_values i32 (4B)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int32>::from_values(black_box(&i32s)));
        }),
    );
    row(
        "from_values i128 (16B)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int128>::from_values(black_box(&i128s)));
        }),
    );
    row(
        "from_values u128 (16B)",
        measure(n, iters, || {
            black_box(FixedSerie::<UInt128>::from_values(black_box(&u128s)));
        }),
    );

    // -- Encodings: the non-integer-array paths (bit-packed, byte-packed decimal) --
    println!("\n  -- encode: bit / decimal element paths --");
    let bools: Vec<bool> = (0..n).map(|i| i % 3 == 0).collect();
    let dec128: Vec<i128> = (0..n as i128).collect();
    let dec256: Vec<I256> = (0..n as i128).map(I256::from_i128).collect();
    row(
        "from_values bool (Bit, per-bit)",
        measure(n, iters, || {
            black_box(FixedSerie::<Bit>::from_values(black_box(&bools)));
        }),
    );
    row(
        "from_values Decimal128 (i128 array)",
        measure(n, iters, || {
            black_box(FixedSerie::<Decimal128>::from_values(black_box(&dec128)));
        }),
    );
    row(
        "from_values Decimal256 (I256, 32B)",
        measure(n, iters, || {
            black_box(FixedSerie::<Decimal256>::from_values(black_box(&dec256)));
        }),
    );

    // -- Encodings: the new narrow fixed-width types (Float16 / Decimal8 / Decimal16) --
    println!("\n  -- encode/decode/reduce: Float16 + small decimals --");
    let halves: Vec<F16> = (0..n).map(|i| F16::from_f32(i as f32)).collect();
    let dec8: Vec<i8> = (0..n).map(|i| i as i8).collect();
    let dec16: Vec<i16> = (0..n).map(|i| i as i16).collect();
    let f16col = FixedSerie::<Float16>::from_values(&halves);
    row(
        "from_values Float16 (2B, u16 reinterpret)",
        measure(n, iters, || {
            black_box(FixedSerie::<Float16>::from_values(black_box(&halves)));
        }),
    );
    row(
        "Serie::values (decode Float16)",
        measure(n, iters, || {
            black_box(f16col.values());
        }),
    );
    row(
        "Serie::sum (reduce Float16 -> f64)",
        measure(n, iters, || {
            black_box(f16col.sum().unwrap());
        }),
    );
    row(
        "from_values Decimal8 (1B, i8 array)",
        measure(n, iters, || {
            black_box(FixedSerie::<Decimal8>::from_values(black_box(&dec8)));
        }),
    );
    row(
        "from_values Decimal16 (2B, i16 array)",
        measure(n, iters, || {
            black_box(FixedSerie::<Decimal16>::from_values(black_box(&dec16)));
        }),
    );

    // -- Encodings: build strategy (bulk vs streaming vs nullable) + single scalar --
    println!("\n  -- encode: build strategy + scalar --");
    // Every 4th element is null so `from_options` keeps the validity buffer — the "nullable,
    // validity" build and the null-aware `to_options` rows below actually exercise that path (a
    // null-free `from_options` is now non-nullable).
    let opts: Vec<Option<i64>> = ints
        .iter()
        .enumerate()
        .map(|(index, &v)| if index % 4 == 0 { None } else { Some(v) })
        .collect();
    row(
        "push loop i64 (streaming build)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            for &value in &ints {
                s.push(black_box(value));
            }
            black_box(s);
        }),
    );
    row(
        "append i64 (batch, bulk)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            s.append(black_box(&ints));
            black_box(s);
        }),
    );
    row(
        "from_options i64 (nullable, validity)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int64>::from_options(black_box(&opts)));
        }),
    );
    row(
        "FixedScalar::of i64 (single encode)",
        measure(n, iters / 16, || {
            for &value in &ints {
                black_box(FixedScalar::<Int64>::of(black_box(value)));
            }
        }),
    );

    // -- Encode kernel, isolated from allocation (a REUSED pre-grown buffer) --
    // The large-buffer `from_values` rows above are dominated by the per-op mmap + first-touch page
    // faults of a fresh 0.5-2 MB data buffer; these rows reuse one buffer so only the encode kernel
    // (a `memcpy` on little-endian) is timed — where the i128/u128 whole-slice fast path shows.
    println!("\n  -- encode kernel only (reused buffer, no per-op alloc) --");
    let mut reuse64 = Heap::with_capacity(n * 8);
    Int64::encode_slice(&mut reuse64, 0, &ints).unwrap();
    let mut reuse128 = Heap::with_capacity(n * 16);
    Int128::encode_slice(&mut reuse128, 0, &i128s).unwrap();
    row(
        "encode_slice i64  -> reused Heap",
        measure(n, iters, || {
            Int64::encode_slice(&mut reuse64, 0, black_box(&ints)).unwrap();
            black_box(&reuse64);
        }),
    );
    row(
        "encode_slice i128 -> reused Heap",
        measure(n, iters, || {
            Int128::encode_slice(&mut reuse128, 0, black_box(&i128s)).unwrap();
            black_box(&reuse128);
        }),
    );

    // -- Decode paths: null-aware bulk (to_options) + the isolated decode kernel --
    println!("\n  -- decode: to_options + kernel (reused Vec) --");
    let nullable = FixedSerie::<Int64>::from_options(&opts);
    row(
        "to_options i64 (non-null)",
        measure(n, iters, || {
            black_box(column.to_options());
        }),
    );
    row(
        "to_options i64 (nullable)",
        measure(n, iters, || {
            black_box(nullable.to_options());
        }),
    );
    let mut out64 = vec![0i64; n];
    let mut out128 = vec![0i128; n];
    row(
        "decode_slice i64  -> reused Vec",
        measure(n, iters, || {
            Int64::decode_slice(&reuse64, 0, black_box(&mut out64)).unwrap();
            black_box(&out64);
        }),
    );
    row(
        "decode_slice i128 -> reused Vec",
        measure(n, iters, || {
            Int128::decode_slice(&reuse128, 0, black_box(&mut out128)).unwrap();
            black_box(&out128);
        }),
    );
}
