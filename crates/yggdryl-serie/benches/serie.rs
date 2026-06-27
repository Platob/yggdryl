//! Lightweight timing benchmarks for the [`Serie`] layer: building / factory dispatch,
//! metadata and fast type checks, value access, zero-copy slicing, resize-with-fill,
//! cast, dictionary (categorical) encode / decode, lazy ranges and nested child / path
//! access.
//!
//! Run with `cargo bench -p yggdryl-serie --bench serie`. Uses a plain `main` (the crate
//! sets `harness = false`) so there is no benchmark-framework dependency.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use yggdryl_serie::arrow_array::{ArrayRef, Int32Array, StringArray};
use yggdryl_serie::{
    from_array, CategoricalSerie, DataType, Int32Serie, NestedSerie, Serie, SerieRef, StructSerie,
    TypedSerie, UInt64RangeSerie, VarcharSerie,
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
    println!("{name:<48} {per:>9.1} ns/iter");
}

/// A representative column length.
const ROWS: usize = 4096;

/// A dense `int32` column of `0..ROWS`.
fn int_array() -> ArrayRef {
    Arc::new(Int32Array::from_iter_values(0..ROWS as i32)) as ArrayRef
}

/// A low-cardinality `utf8` column: 8 distinct labels repeated (the categorical case).
fn string_array() -> ArrayRef {
    let labels = [
        "red", "green", "blue", "amber", "cyan", "magenta", "lime", "navy",
    ];
    let values: Vec<&str> = (0..ROWS).map(|i| labels[i % labels.len()]).collect();
    Arc::new(StringArray::from(values)) as ArrayRef
}

fn main() {
    let n = 1_000_000;

    let ints = int_array();
    let strs = string_array();

    // ---- build / factory dispatch ----
    bench("from_array (int32, 4096 rows)", n / 4, || {
        black_box(from_array("c", black_box(ints.clone())).unwrap());
    });
    bench("from_array (utf8, 4096 rows)", n / 4, || {
        black_box(from_array("c", black_box(strs.clone())).unwrap());
    });

    let int_serie = from_array("c", ints.clone()).unwrap();
    let str_serie = from_array("c", strs.clone()).unwrap();

    // ---- metadata / fast type checks ----
    bench("Serie::num_rows", n * 4, || {
        black_box(black_box(&int_serie).num_rows());
    });
    bench("Serie::null_count", n * 4, || {
        black_box(black_box(&int_serie).null_count());
    });
    bench("Serie::category", n * 4, || {
        black_box(black_box(&int_serie).category());
    });
    bench("Serie::data_type", n * 4, || {
        black_box(black_box(&int_serie).data_type());
    });

    // ---- value access ----
    bench("Serie::value_at (int32)", n * 2, || {
        black_box(black_box(&int_serie).value_at(black_box(2000)));
    });
    let typed = int_serie.as_any().downcast_ref::<Int32Serie>().unwrap();
    bench("Int32Serie::value (typed)", n * 4, || {
        black_box(black_box(typed).value(black_box(2000)));
    });

    // ---- zero-copy slice ----
    bench("Serie::slice (zero-copy)", n * 2, || {
        black_box(black_box(&int_serie).slice(black_box(10), black_box(1000)));
    });

    // ---- resize (grow + fill) ----
    let short = from_array(
        "c",
        Arc::new(Int32Array::from_iter_values(0..16)) as ArrayRef,
    )
    .unwrap();
    bench("Serie::resize grow+fill (16 -> 4096)", n / 200, || {
        black_box(black_box(&short).resize(black_box(ROWS)).unwrap());
    });

    // ---- cast ----
    let i64t = DataType::int(64, true);
    bench("Serie::cast int32 -> int64 (4096)", n / 200, || {
        black_box(black_box(&int_serie).cast(black_box(&i64t)).unwrap());
    });
    let f64t = DataType::float(64);
    bench("Serie::cast int32 -> float64 (4096)", n / 200, || {
        black_box(black_box(&int_serie).cast(black_box(&f64t)).unwrap());
    });

    // ---- categorical (dictionary) encode / decode ----
    bench("CategoricalSerie::from_serie (8 distinct)", n / 200, || {
        black_box(CategoricalSerie::from_serie(black_box(&*str_serie)).unwrap());
    });
    let cat = CategoricalSerie::from_serie(&*str_serie).unwrap();
    bench("CategoricalSerie::materialize (decode)", n / 200, || {
        black_box(black_box(&cat).materialize());
    });

    // ---- lazy range ----
    let range = UInt64RangeSerie::new("r", 0, 1, ROWS);
    bench("UInt64RangeSerie::value_at (lazy)", n * 2, || {
        black_box(black_box(&range).value_at(black_box(2000)));
    });
    bench("UInt64RangeSerie::materialize (4096)", n / 500, || {
        black_box(black_box(&range).materialize());
    });

    // ---- nested child / path access ----
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1); ROWS]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values(
        "name",
        vec![Some("x"); ROWS],
    ));
    let rec = StructSerie::from_children("rec", vec![id, name]).unwrap();
    bench("StructSerie::child_by_name", n * 2, || {
        black_box(black_box(&rec).child_by_name(black_box("name")));
    });
    bench("Serie::select (node path)", n, || {
        black_box(black_box(&rec).select(black_box("name")).unwrap());
    });

    // ---- frame (DataFrame) operations ----
    bench("StructSerie::select_columns", n / 4, || {
        black_box(
            black_box(&rec)
                .select_columns(black_box(&["name"]))
                .unwrap(),
        );
    });
    let target = vec![
        yggdryl_serie::Field::new("id", DataType::int(64, true), true),
        yggdryl_serie::Field::new("name", DataType::varchar(), true),
    ];
    bench(
        "StructSerie::select_fields (cast int32->int64)",
        n / 200,
        || {
            black_box(
                black_box(&rec)
                    .select_fields(black_box(target.clone()))
                    .unwrap(),
            );
        },
    );
    bench("StructSerie::sort_by (4096)", n / 500, || {
        black_box(black_box(&rec).sort_by(black_box("id"), false).unwrap());
    });
    let mask: Vec<bool> = (0..ROWS).map(|i| i % 2 == 0).collect();
    bench("StructSerie::filter (4096)", n / 500, || {
        black_box(black_box(&rec).filter(black_box(&mask)).unwrap());
    });
    bench("StructSerie::row -> StructScalar", n / 4, || {
        black_box(black_box(&rec).row(black_box(2000)).unwrap());
    });
    bench("StructSerie::to_record_batch", n / 200, || {
        black_box(black_box(&rec).to_record_batch().unwrap());
    });

    // ---- value mutation (functional rebuild) ----
    let value = yggdryl_scalar::IntScalar::new(42, 32, true);
    bench("Serie::set_at (int32, 4096)", n / 500, || {
        black_box(
            black_box(&int_serie)
                .set_at(black_box(2000), black_box(&value), true)
                .unwrap(),
        );
    });
    bench("Serie::push (int32, 4096)", n / 500, || {
        black_box(black_box(&int_serie).push(black_box(&value), true).unwrap());
    });
}
