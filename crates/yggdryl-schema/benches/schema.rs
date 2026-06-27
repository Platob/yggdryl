//! Lightweight timing benchmarks for the schema types: parsing/rendering, fast
//! type checks, schema merge and (under the `arrow` feature) Arrow conversion.
//!
//! Run with `cargo bench -p yggdryl-schema --bench schema` (add `--features arrow`
//! for the conversion rows). Uses a plain `main` (the crate sets `harness = false`)
//! so there is no benchmark-framework dependency.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_schema::{DataType, Field, MergeStrategy, TimeUnit, Timezone};

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
    println!("{name:<44} {per:>9.1} ns/iter");
}

/// A representative 8-field schema (a struct-typed field) with a nested struct and a list.
fn sample_schema() -> DataType {
    DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), false),
        Field::new("name", DataType::varchar(), true),
        Field::new("score", DataType::float(64), true),
        Field::new("active", DataType::Boolean, true),
        Field::new(
            "ts",
            DataType::timestamp(TimeUnit::Microsecond, Some(Timezone::Utc)),
            true,
        ),
        Field::new("amount", DataType::decimal(38, 9), true),
        Field::new(
            "tags",
            DataType::list(Field::new("item", DataType::varchar(), true)),
            true,
        ),
        Field::new(
            "address",
            DataType::struct_(vec![
                Field::new("city", DataType::varchar(), true),
                Field::new("zip", DataType::varchar(), true),
            ]),
            true,
        ),
    ])
}

fn main() {
    let n = 2_000_000;

    // ---- parse / render ----
    bench("DataType::from_str (int64)", n, || {
        black_box(DataType::from_str(black_box("int64")).unwrap());
    });
    bench("DataType::from_str (timestamp[us, UTC])", n, || {
        black_box(DataType::from_str(black_box("timestamp[us, UTC]")).unwrap());
    });
    let nested = "struct[id: int64 not null, tags: list[item: utf8], amount: decimal128[38, 9]]";
    bench("DataType::from_str (nested struct)", n / 4, || {
        black_box(DataType::from_str(black_box(nested)).unwrap());
    });
    let dt = DataType::from_str(nested).unwrap();
    bench("DataType::to_str (nested struct)", n / 4, || {
        black_box(black_box(&dt).to_str());
    });

    // ---- fast type checks ----
    let int32 = DataType::int(32, true);
    bench("DataType::is_numeric", n * 4, || {
        black_box(black_box(&int32).is_numeric());
    });
    bench("DataType::bit_size", n * 4, || {
        black_box(black_box(&int32).bit_size());
    });
    bench("DataType::category", n * 4, || {
        black_box(black_box(&dt).category());
    });
    let f64t = DataType::float(64);
    bench("DataType::can_cast_to", n * 2, || {
        black_box(black_box(&int32).can_cast_to(black_box(&f64t)));
    });

    // ---- coercion / merge ----
    let int8 = DataType::int(8, true);
    let int64 = DataType::int(64, true);
    bench("DataType::common_type (int8, int64)", n * 2, || {
        black_box(black_box(&int8).common_type(black_box(&int64)));
    });
    let a = sample_schema();
    let b = {
        let DataType::Struct(mut fields) = sample_schema() else {
            unreachable!()
        };
        fields.push(Field::new("extra", DataType::int(32, true), true));
        DataType::Struct(fields)
    };
    bench("DataType::merge (8-field structs, promote)", n / 8, || {
        black_box(
            black_box(&a)
                .merge(black_box(&b), MergeStrategy::Promote)
                .unwrap(),
        );
    });

    #[cfg(feature = "arrow")]
    {
        let schema = Field::new("rec", sample_schema(), false);
        bench("Field::to_arrow_schema (8 fields)", n / 8, || {
            black_box(black_box(&schema).to_arrow_schema().unwrap());
        });
        let arrow = schema.to_arrow_schema().unwrap();
        bench("Field::from_arrow_schema (8 fields)", n / 8, || {
            black_box(Field::from_arrow_schema("rec", black_box(&arrow), false));
        });
    }
    #[cfg(not(feature = "arrow"))]
    println!("(build with --features arrow for the Arrow-conversion rows)");
}
