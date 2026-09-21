//! `rust/src/iceberg/statistics.rs`: the bound folding a writer does per file.
//!
//! Folding is what turns a row group's statistics into the one lower and upper
//! bound a manifest records, and it is reached through `yggdryl::internals`
//! because no caller can name it. The bounds it produces are observed in
//! `rust/tests/iceberg/mod_.rs`.

use yggdryl::DataType;
use yggdryl::internals::iceberg_statistics::fold_encoded;

#[test]
fn malformed_encoded_candidates_never_become_written_bounds() {
    let mut lower = None;
    fold_encoded(&mut lower, &[0; 3], &DataType::Int64, true);
    assert!(lower.is_none());

    let mut upper = None;
    fold_encoded(
        &mut upper,
        &f64::NAN.to_le_bytes(),
        &DataType::Float64,
        false,
    );
    assert!(upper.is_none());

    let promoted = 7_i32.to_le_bytes();
    fold_encoded(&mut lower, &promoted, &DataType::Int64, true);
    assert_eq!(lower.as_deref(), Some(promoted.as_slice()));

    fold_encoded(&mut lower, &[0; 9], &DataType::Int64, true);
    assert_eq!(lower.as_deref(), Some(promoted.as_slice()));
}
