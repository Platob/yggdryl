//! `rust/src/csv/mod.rs`: what the module publishes, and the constants a
//! caller reads a default by.

use yggdryl::csv::{
    Csv, CsvOptions, DEFAULT_CSV_BATCH_BYTE_SIZE, DEFAULT_CSV_INFER_ROW_SIZE,
    overwrite_arrow_reader, read_batch_reader, read_field,
};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;

#[test]
fn the_defaults_are_the_constants() {
    let options = CsvOptions::new();
    assert_eq!(options.batch_byte_size(), Some(DEFAULT_CSV_BATCH_BYTE_SIZE));
    assert_eq!(DEFAULT_CSV_BATCH_BYTE_SIZE, 64 * 1024 * 1024);
    assert_eq!(options.infer_row_size(), DEFAULT_CSV_INFER_ROW_SIZE);
    assert_eq!(DEFAULT_CSV_INFER_ROW_SIZE, 1024);
}

#[test]
fn the_doors_are_published() {
    // Named through the module, so a retired door fails here rather than in
    // a caller.
    let _: fn(&Buffer, &CsvOptions) -> yggdryl::Result<yggdryl::Field> = read_field::<Buffer>;
    let _: fn(
        &Buffer,
        Option<&yggdryl::Field>,
        &CsvOptions,
    ) -> yggdryl::arrow::Result<yggdryl::arrow::BatchReader> = read_batch_reader::<Buffer>;
    let _: fn(&mut Buffer, yggdryl::arrow::BatchReader, &CsvOptions) -> yggdryl::Result<()> =
        overwrite_arrow_reader::<Buffer>;
    let media = Csv::new(Buffer::new());
    assert_eq!(media.options(), &CsvOptions::new());
}
