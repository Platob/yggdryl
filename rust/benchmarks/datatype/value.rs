use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{EnumScalar, Float16, Float32, Float64, I256, IOMode, Scalar, TimeUnit, TypedScalar};

pub(crate) fn value_benchmarks(criterion: &mut Criterion) {
    let record = Scalar::from_record([
        (
            "at",
            Scalar::datetime64(
                1_700_000_000_000_000,
                TimeUnit::Microsecond,
                yggdryl::Timezone::UTC,
            )
            .unwrap(),
        ),
        ("id", Scalar::from(42_i64)),
        ("price", Scalar::d256(I256::from_i128(1_050), 2)),
        ("symbol", Scalar::from("AAPL")),
    ])
    .unwrap();
    let array = Scalar::from_sequence([Scalar::from(42_i64), Scalar::Null]);
    let rows = Scalar::from_sequence([record.clone()]);
    let integer_left = Scalar::I64(9_876_543);
    let integer_right = Scalar::I64(97);
    let decimal_left = Scalar::d128(1_050, 2);
    let decimal_right = Scalar::d128(2, 0);
    let instant = Scalar::datetime64(
        1_700_000_000_000_000,
        TimeUnit::Microsecond,
        yggdryl::Timezone::UTC,
    )
    .unwrap();
    let duration = Scalar::duration64(250, TimeUnit::Millisecond).unwrap();
    let duration_scalar = Scalar::I64(5);
    let typed: TypedScalar = TypedScalar::try_from_value(Scalar::I64(42)).unwrap();
    let integer256: I256 = "1234567890123456789012345678901234567890".parse().unwrap();
    let float16 = Float16::from_f16(half::f16::from_f32(1.25));
    let float32 = Float32::from_f32(1.25);
    let float64 = Float64::from_f64(1.25);
    let enum_member = EnumScalar::IOMode(IOMode::Append);

    let mut group = criterion.benchmark_group("value");
    group.bench_function("stable_hash_record", |bencher| {
        bencher.iter(|| black_box(&record).stable_hash());
    });
    group.bench_function("stable_hash_typed_value", |bencher| {
        bencher.iter(|| black_box(&typed).stable_hash());
    });
    group.bench_function("stable_hash_i256", |bencher| {
        bencher.iter(|| black_box(&integer256).stable_hash());
    });
    group.bench_function("stable_hash_float16", |bencher| {
        bencher.iter(|| black_box(&float16).stable_hash());
    });
    group.bench_function("stable_hash_float32", |bencher| {
        bencher.iter(|| black_box(&float32).stable_hash());
    });
    group.bench_function("stable_hash_float64", |bencher| {
        bencher.iter(|| black_box(&float64).stable_hash());
    });
    group.bench_function("enum_from_parts", |bencher| {
        bencher.iter(|| EnumScalar::from_parts(black_box("io_mode"), black_box("append")).unwrap());
    });
    group.bench_function("enum_kind", |bencher| {
        bencher.iter(|| black_box(enum_member).kind());
    });
    group.bench_function("enum_value", |bencher| {
        bencher.iter(|| black_box(enum_member).as_str());
    });
    group.bench_function("enum_ordinal", |bencher| {
        bencher.iter(|| black_box(enum_member).ordinal());
    });
    group.bench_function("infer_record_datatype", |bencher| {
        bencher.iter(|| black_box(&record).data_type().unwrap());
    });
    group.bench_function("infer_scalar_field", |bencher| {
        bencher.iter(|| {
            black_box(&Scalar::from(42_i64))
                .inferred_scalar_field()
                .unwrap()
        });
    });
    group.bench_function("infer_array_field", |bencher| {
        bencher.iter(|| black_box(&array).inferred_array_field().unwrap());
    });
    group.bench_function("infer_struct_field", |bencher| {
        bencher.iter(|| black_box(&rows).inferred_struct_field().unwrap());
    });
    group.bench_function("record_field_update", |bencher| {
        bencher.iter(|| black_box(&record).with_field("venue", "XNAS").unwrap());
    });
    group.bench_function("temporal_restate_day_to_nanosecond", |bencher| {
        let date = Scalar::date32(20_000);
        bencher.iter(|| black_box(&date).temporal_count_at(TimeUnit::Nanosecond));
    });
    group.bench_function("json_bytes_record", |bencher| {
        bencher.iter(|| black_box(&record).as_json_bytes().unwrap());
    });
    group.bench_function("json_utf8_record", |bencher| {
        bencher.iter(|| black_box(&record).as_json_utf8().unwrap());
    });
    group.bench_function("checked_add_i64", |bencher| {
        bencher.iter(|| black_box(&integer_left).checked_add(black_box(&integer_right)));
    });
    group.bench_function("checked_sub_i64", |bencher| {
        bencher.iter(|| black_box(&integer_left).checked_sub(black_box(&integer_right)));
    });
    group.bench_function("checked_mul_decimal128", |bencher| {
        bencher.iter(|| black_box(&decimal_left).checked_mul(black_box(&decimal_right)));
    });
    group.bench_function("checked_div_decimal128", |bencher| {
        bencher.iter(|| black_box(&decimal_left).checked_div(black_box(&decimal_right)));
    });
    group.bench_function("checked_rem_decimal128", |bencher| {
        bencher.iter(|| black_box(&decimal_left).checked_rem(black_box(&decimal_right)));
    });
    group.bench_function("checked_neg_i64", |bencher| {
        bencher.iter(|| black_box(&integer_left).checked_neg());
    });
    group.bench_function("checked_abs_i64", |bencher| {
        bencher.iter(|| black_box(&integer_left).checked_abs());
    });
    group.bench_function("checked_add_temporal_duration", |bencher| {
        bencher.iter(|| black_box(&instant).checked_add(black_box(&duration)));
    });
    group.bench_function("checked_sub_temporal", |bencher| {
        bencher.iter(|| black_box(&instant).checked_sub(black_box(&instant)));
    });
    group.bench_function("checked_abs_duration", |bencher| {
        bencher.iter(|| black_box(&duration).checked_abs());
    });
    group.bench_function("checked_mul_duration_integer", |bencher| {
        bencher.iter(|| black_box(&duration).checked_mul(black_box(&duration_scalar)));
    });
    group.bench_function("checked_div_duration_integer", |bencher| {
        bencher.iter(|| black_box(&duration).checked_div(black_box(&duration_scalar)));
    });
    group.finish();
}
