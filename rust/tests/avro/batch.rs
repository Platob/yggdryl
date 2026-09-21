//! `rust/src/avro/batch.rs`: the column decoders no caller can name.
//!
//! One decoder is built per column, so a variant payload widening the shared
//! enum is paid for by every column in the file. Both enums are file-private,
//! and only their width is pinned, so each is measured through
//! `yggdryl::internals`.

use yggdryl::internals::avro_batch::{column_reader_size, root_step_size};

#[test]
fn variant_payload_does_not_widen_other_column_readers() {
    let column = column_reader_size();
    let root = root_step_size();
    assert!(
        column <= 240 && root <= 240,
        "ColumnReader is {column} bytes and RootStep is {root} bytes"
    );
}
