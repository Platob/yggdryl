//! The Avro exchange with an external implementation.
//!
//! `scripts/check_avro_interop.py` drives this target twice around a fastavro
//! round trip: the first run writes a container fastavro must read, the second
//! reads a container fastavro wrote. The reading half prints `SKIPPED` when
//! the external file is absent - the driver fails on that word - so a skipped
//! half can never read as a pass.

use yggdryl::TimeUnit;
use yggdryl::holder::local::File;
use yggdryl::media::avro;
use yggdryl::{DataType, Scalar, Timezone};

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("avro-interop");
    path
}

/// The writer schema both sides agree on, logical types included.
fn schema() -> Scalar {
    yggdryl::text::json::from_utf8(
        r#"{"type": "record", "name": "trade", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "quantity", "type": "long"},
            {"name": "price", "type": ["null", "double"], "default": null},
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "cost", "type": {"type": "bytes", "logicalType": "decimal",
                                       "precision": 10, "scale": 2}},
            {"name": "tags", "type": {"type": "array", "items": "string"}},
            {"name": "extra", "type": {"type": "record", "name": "extra", "fields": [
                {"name": "flag", "type": "boolean"}
            ]}}
        ]}"#,
    )
    .expect("the exchange schema parses")
}

/// The `cost` column as the schema's own declaration holds it.
///
/// The exchange schema spells `decimal(precision: 10, scale: 2)`, and ten
/// digits is a `Decimal64`, so that is the width the coefficient comes back
/// in: reading restates every leaf through its declared `DataType`, the one
/// value contract, rather than handing back whatever width happened to carry
/// it on the wire. Asserting `d128` here would assert the reader forgets what
/// the schema declared.
fn declared_cost(unscaled: i128) -> Scalar {
    DataType::decimal(10, 2)
        .expect("a ten-digit decimal declaration")
        .scalar(Scalar::d128(unscaled, 2))
        .expect("a coefficient of at most ten digits")
}

/// The rows both sides assert, in file order.
fn expected_rows() -> Vec<Scalar> {
    let row = |symbol: &str,
               quantity: i64,
               price: Scalar,
               day: i32,
               at: i64,
               cost: i128,
               tags: &[&str],
               flag: bool| {
        Scalar::from_record([
            ("symbol", Scalar::from(symbol)),
            ("quantity", Scalar::from(quantity)),
            ("price", price),
            ("day", Scalar::date32(day)),
            (
                "at",
                Scalar::datetime64(at, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            ),
            ("cost", declared_cost(cost)),
            (
                "tags",
                Scalar::from_sequence(tags.iter().map(|tag| Scalar::from(*tag))),
            ),
            (
                "extra",
                Scalar::from_record([("flag", Scalar::from(flag))]).expect("unique field names"),
            ),
        ])
        .expect("unique field names")
    };
    vec![
        row(
            "AAPL",
            100,
            Scalar::from(187.5_f64),
            19_782,
            1_700_000_000_000_000,
            18_750,
            &["tech", "large"],
            true,
        ),
        // A pre-epoch date, a negative decimal, an empty array.
        row(
            "MSFT",
            -25,
            Scalar::Null,
            -3_652,
            -1_000_000,
            -99,
            &[],
            false,
        ),
    ]
}

#[test]
fn writes_a_container_for_the_external_reader() {
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let mut handle = File::new(dir.join("from-rust.avro")).expect("a file handle");
    avro::write_container(
        &mut handle,
        &schema(),
        &[("exchange", "yggdryl")],
        &expected_rows(),
    )
    .expect("the container writes");
    println!("avro-interop: wrote");
}

#[test]
fn reads_the_container_the_external_writer_produced() {
    let path = exchange_dir().join("from-fastavro.avro");
    if !path.exists() {
        println!("avro-interop: SKIPPED (no {})", path.display());
        return;
    }
    let handle = File::new(&path).expect("a file handle");
    let container = avro::read_container(&handle).expect("the external container reads");
    assert_eq!(container.rows, expected_rows());
    // The resolved path over the same file must agree with the direct one.
    let reader = avro::Schema::from_json(&schema()).expect("the schema parses");
    let resolved =
        avro::read_container_resolved(&handle, &reader).expect("the external container resolves");
    assert_eq!(resolved.rows, container.rows);
    println!("avro-interop: read");
}

#[test]
fn reads_the_container_the_apache_avro_crate_produced() {
    // The driver builds a scratch crate around apache-avro - a checking tool
    // on the script side only, never a dependency of this crate - which
    // round-trips the Rust-written container into this file.
    let path = exchange_dir().join("from-apache.avro");
    if !path.exists() {
        println!("avro-interop: absent from-apache.avro");
        return;
    }
    let handle = File::new(&path).expect("a file handle");
    let container = avro::read_container(&handle).expect("the apache-avro container reads");
    assert_eq!(container.rows, expected_rows());
    println!("avro-interop: read apache");
}
