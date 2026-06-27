//! Lightweight timing benchmarks for the [`Scalar`] trait layer: concrete construction,
//! interned `data_type`, the type-erased factory, Arrow conversion, casting and nested
//! recursion. Plain `main` (the crate sets `harness = false`).

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use yggdryl_scalar::arrow_array::{ArrayRef, Int32Array};
use yggdryl_scalar::{
    DataType, Field, IntScalar, Scalar, ScalarValue, StructScalar, VarcharScalar,
};

/// Times `f` over `iters` iterations (after a short warm-up) and prints ns/iter.
fn bench(name: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:<46} {per:>9.1} ns/iter");
}

fn main() {
    let n = 1_000_000;

    let int = IntScalar::new(42, 32, true);
    bench("IntScalar::new", n * 2, || {
        black_box(IntScalar::new(black_box(42), 32, true));
    });
    bench("Scalar::data_type (interned Arc)", n * 2, || {
        black_box(black_box(&int).data_type());
    });
    bench("Scalar::to_array (int32)", n / 2, || {
        black_box(black_box(&int).to_array().unwrap());
    });
    bench("Scalar::to_str (int32)", n / 2, || {
        black_box(black_box(&int).to_str());
    });

    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(7)]));
    bench(
        "ScalarValue::scalar_at (factory -> ScalarRef)",
        n / 2,
        || {
            black_box(ScalarValue::scalar_at(black_box(array.as_ref()), 0).unwrap());
        },
    );

    let i64t = DataType::int(64, true);
    bench("Scalar::cast int32 -> int64", n / 20, || {
        black_box(black_box(&int).cast(black_box(&i64t)).unwrap());
    });

    // nested struct { id: int64, name: utf8 }
    let rec = StructScalar::from_children(
        vec![
            Field::new("id", DataType::int(64, true), false),
            Field::new("name", DataType::varchar(), true),
        ],
        vec![
            IntScalar::new(7, 64, true).into(),
            VarcharScalar::new("y").into(),
        ],
    );
    bench("StructScalar::to_array (recursive)", n / 50, || {
        black_box(black_box(&rec).to_array().unwrap());
    });
    bench("StructScalar::child_named", n, || {
        black_box(black_box(&rec).child_named(black_box("name")));
    });
}
