use std::hint::black_box;

use criterion::Criterion;
use yggdryl::IdKey;
use yggdryl::{
    DataType, Field, FieldScalar, Float16, Float32, Float64, IOMode, Scalar, TimeUnit, Timezone,
    Vocabulary, i256,
};

pub(crate) fn value_benchmarks(criterion: &mut Criterion) {
    {
        use yggdryl::graph::{Market, Order};
        use yggdryl::{Bic, Dti, Elf, Fisn, Lei};
        use yggdryl::{Cfi, Figi, Forex, FxSymbol, IdType, Identifier, Isin, Ric};
        let mut codes = criterion.benchmark_group("instrument_codes");
        codes.bench_function("isin", |bench| {
            bench.iter(|| Isin::new(black_box("us0378331005")).unwrap());
        });
        codes.bench_function("figi", |bench| {
            bench.iter(|| Figi::new(black_box("bbg000blnq16")).unwrap());
        });
        codes.bench_function("ric", |bench| {
            bench.iter(|| Ric::new(black_box("0005.HK")).unwrap());
        });
        codes.bench_function("lei", |bench| {
            bench.iter(|| Lei::new(black_box("hwupkr0mpou8fgxbt394")).unwrap());
        });
        codes.bench_function("lei_closed", |bench| {
            bench.iter(|| Lei::is_closed(black_box("HWUPKR0MPOU8FGXBT394")));
        });
        codes.bench_function("bic", |bench| {
            bench.iter(|| Bic::new(black_box("deutdeffxxx")).unwrap());
        });
        codes.bench_function("elf", |bench| {
            bench.iter(|| Elf::new(black_box("2hbr")).unwrap());
        });
        codes.bench_function("dti", |bench| {
            bench.iter(|| Dti::new(black_box("x9j9k872s")).unwrap());
        });
        codes.bench_function("dti_closed", |bench| {
            bench.iter(|| Dti::is_closed(black_box("X9J9K872S")));
        });
        codes.bench_function("fisn", |bench| {
            bench.iter(|| Fisn::new(black_box("apple inc/sh")).unwrap());
        });
        let ric = Ric::new("0005.HK").unwrap();
        codes.bench_function("ric_exchange_code", |bench| {
            bench.iter(|| black_box(&ric).exchange_code());
        });
        codes.bench_function("forex_new", |bench| {
            bench.iter(|| Forex::new(black_box("eur-usd")).unwrap());
        });
        codes.bench_function("forex_from_symbol", |bench| {
            bench.iter(|| FxSymbol::from_symbol(black_box("EUR/USD 1M")).unwrap());
        });
        codes.bench_function("cfi_classification", |bench| {
            bench.iter(|| Cfi::is_classified(black_box("ESVUFR")));
        });
        codes.bench_function("cfi_refine", |bench| {
            bench.iter(|| Cfi::refined(black_box("ESXXXX"), black_box("ESVUFR")));
        });
        let isin = Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap();
        codes.bench_function("market_identifier_setter", |bench| {
            bench.iter(|| {
                let mut element = Order::new();
                let _ = element.insert_securityid(black_box(&isin).clone());
                black_box(element)
            });
        });
        codes.finish();
    }
    {
        // A column entering an enum leaf, through the one plan compiled
        // before the loop: text read as the member's spelling, integers as
        // its code.
        use std::sync::Arc;

        use arrow_array::{ArrayRef, Int32Array, StringArray};
        use yggdryl::{ArrowCastOptions, ArrowCastPlan, Serie};

        const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);
        let mut enums = criterion.benchmark_group("enums");
        let leaves: [(&str, DataType, [&str; 5], [i32; 5]); 2] = [
            (
                "marketdatakind",
                DataType::marketdatakind(),
                ["ORDR", "QUOT", "EXEC", "TRAD", "BOOK"],
                [10, 14, 8, 21, 3],
            ),
            (
                "side",
                DataType::side(),
                ["BUY", "SELL", "1", "2", "SellShort"],
                [1, 2, 5, 1, 2],
            ),
        ];
        for (name, dtype, spellings, codes) in leaves {
            let target = Field::new(name, dtype, true);
            let sources: [(&str, ArrayRef); 2] = [
                (
                    "utf8",
                    Arc::new(StringArray::from_iter_values(
                        (0..ROWS).map(|row| spellings[row % spellings.len()]),
                    )),
                ),
                (
                    "int",
                    Arc::new(Int32Array::from_iter_values(
                        (0..ROWS).map(|row| codes[row % codes.len()]),
                    )),
                ),
            ];
            for (source_name, array) in sources {
                let column = Serie::from_arrow_array(None, array, ArrowCastOptions::new())
                    .expect("the benchmark column lands as itself");
                let plan = ArrowCastPlan::compile(
                    column.require_field().expect("a landed column has a field"),
                    &target,
                    ArrowCastOptions::new(),
                )
                .expect("the benchmark cast is plannable");
                enums.bench_function(format!("{name}_ingest_{source_name}"), |bench| {
                    bench.iter(|| {
                        plan.apply(black_box(&column))
                            .expect("every benchmark cell names a member")
                    });
                });
            }
        }
        enums.finish();
    }
    let record = Scalar::from_struct([
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
        ("price", Scalar::decimal256(i256::from_i128(1_050), 2)),
        ("symbol", Scalar::from("AAPL")),
    ])
    .unwrap();
    let array = Scalar::from_sequence([Scalar::from(42_i64), Scalar::Null]);
    let rows = Scalar::from_sequence([record.clone()]);
    let integer_left = Scalar::from(9_876_543);
    let integer_right = Scalar::from(97);
    let decimal_left = Scalar::decimal128(1_050, 2);
    let decimal_right = Scalar::decimal128(2, 0);
    let instant = Scalar::datetime64(
        1_700_000_000_000_000,
        TimeUnit::Microsecond,
        yggdryl::Timezone::UTC,
    )
    .unwrap();
    let duration = Scalar::duration64(250, TimeUnit::Millisecond).unwrap();
    let duration_scalar = Scalar::from(5);
    let typed_integer = Scalar::from(42_i64);
    let typed_decimal = Scalar::decimal128(1_050, 2);
    let typed_field = Field::new("size", DataType::Int64, false);
    let integer256: i256 = "1234567890123456789012345678901234567890".parse().unwrap();
    let float16 = Float16::from_f16(half::f16::from_f32(1.25));
    let float32 = Float32::from_f32(1.25);
    let float64 = Float64::from_f64(1.25);
    let enum_member = Vocabulary::IOMode(IOMode::Append);
    let integer_scalar = Scalar::from(42);
    let decimal_scalar = Scalar::decimal256(integer256, 2);
    let float_scalar = Scalar::from_float(1.25, 32).unwrap();

    let mut group = criterion.benchmark_group("value");
    group.bench_function("stable_hash_record", |bencher| {
        bencher.iter(|| black_box(&record).stable_hash());
    });
    group.bench_function("typed_infer", |bencher| {
        bencher.iter(|| {
            let integer = FieldScalar::infer(black_box(&typed_integer).clone()).unwrap();
            let decimal = FieldScalar::infer(black_box(&typed_decimal).clone()).unwrap();
            black_box((integer, decimal))
        });
    });
    group.bench_function("typed_new", |bencher| {
        bencher.iter(|| {
            FieldScalar::new(black_box(&typed_field), black_box(&typed_integer).clone()).unwrap()
        });
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
    group.bench_function("from_float32", |bencher| {
        bencher.iter(|| Scalar::from_float(black_box(1.25), black_box(32)).unwrap());
    });
    group.bench_function("family_constructors", |bencher| {
        bencher.iter(|| {
            let date = Scalar::from_date(black_box(20_000), TimeUnit::Day, Timezone::NAIVE);
            let time = Scalar::from_time(black_box(1), TimeUnit::Nanosecond, Timezone::NAIVE);
            let datetime =
                Scalar::from_datetime(black_box(1), TimeUnit::Microsecond, Timezone::UTC);
            let duration = Scalar::from_duration(black_box(1), TimeUnit::Second, Timezone::NAIVE);
            let decimal = Scalar::from_decimal(black_box(integer256), black_box(2));
            black_box((date, time, datetime, duration, decimal))
        });
    });
    group.bench_function("as_f64", |bencher| {
        bencher.iter(|| black_box(&float_scalar).as_f64());
    });
    group.bench_function("as_i128", |bencher| {
        bencher.iter(|| black_box(&integer_scalar).as_i128());
    });
    group.bench_function("as_decimal", |bencher| {
        bencher.iter(|| black_box(&decimal_scalar).as_decimal());
    });
    // The family a temporal belongs to is its identifier's: one read of the
    // variant, no value built.
    group.bench_function("as_temporal", |bencher| {
        bencher.iter(|| black_box(&instant).id().temporal_family());
    });
    group.bench_function("temporal_readers", |bencher| {
        bencher.iter(|| {
            let value = black_box(&instant);
            black_box((
                value.id().temporal_family(),
                value.temporal_count(),
                value.temporal_unit(),
                value.temporal_timezone(),
            ))
        });
    });
    group.bench_function("enum_from_parts", |bencher| {
        bencher.iter(|| Vocabulary::from_parts(black_box("IOMode"), black_box("append")).unwrap());
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
        bencher.iter(|| black_box(&record).dtype().unwrap());
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
        bencher.iter(|| black_box(&record).into_json_bytes().unwrap());
    });
    group.bench_function("json_utf8_record", |bencher| {
        bencher.iter(|| black_box(&record).into_json().unwrap());
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
