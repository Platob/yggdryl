//! Market product integration tests: the five values, their rows, their
//! FIX doors, the symbol, the statement, the book iterator and the three
//! hops from a line to a product.

#[path = "market/book.rs"]
mod book;
#[path = "market/execution.rs"]
mod execution;
/// The three hops - line, message, product - over the bridge's own capture.
#[path = "market/hops.rs"]
mod hops;
/// One book per symbol per instant, over hand-built statements.
#[path = "market/iterator.rs"]
mod iterator;
#[path = "market/order.rs"]
mod order;
#[path = "market/quote.rs"]
mod quote;
#[path = "market/statement.rs"]
mod statement;
#[path = "market/symbol.rs"]
mod symbol;
#[path = "market/trade.rs"]
mod trade;

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::arrow::BatchReader;
use yggdryl::market::Product;
use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar};

/// The committed dictionary, which every door here resolves under.
fn committed_registry() -> Arc<FixRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<FixRegistry>> = std::sync::OnceLock::new();
    Arc::clone(REGISTRY.get_or_init(|| {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
        let folder = yggdryl::local::Folder::new(root).expect("the local seed path");
        Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
    }))
}

/// A codec over the committed dictionary with one explicit intake clock,
/// so a fixture that states no sending time never consults now.
fn codec() -> FixCodec {
    FixCodec::new(committed_registry())
        .try_with_default_sending_time(Some(
            Scalar::datetime64(
                1_704_190_530_000_000_000,
                yggdryl::TimeUnit::Nanosecond,
                yggdryl::Timezone::UTC,
            )
            .expect("a clock"),
        ))
        .expect("a codec")
}

/// The messages a fixture's lines parse into, in line order, before any
/// walk.
fn parsed(codec: &FixCodec, lines: &[&[u8]]) -> Vec<FixMsg> {
    codec
        .parse_lines(lines.iter().copied())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("every line reads")
}

/// The messages as one stream of batches of FIX rows, the shape every
/// Arrow door reads.
fn message_reader(codec: &FixCodec, messages: Vec<FixMsg>) -> BatchReader {
    let schema = yggdryl::fix_schema(codec.registry(), "fix").expect("the fixed row");
    codec
        .arrow_reader(schema, messages)
        .expect("the message rows open")
}

/// Every row a reader answers, by value.
fn rows_of(reader: BatchReader) -> Vec<Scalar> {
    let mut rows = Vec::new();
    for batch in reader {
        let batch: RecordBatch = batch.expect("a batch");
        let held = yggdryl::arrow::batch_to_value(&batch).expect("the batch reads");
        rows.extend(held.as_sequence().expect("rows").iter().cloned());
    }
    rows
}

/// The rows a stream of products publishes, one per product.
fn product_rows<P: Product>(products: impl IntoIterator<Item = yggdryl::Result<P>>) -> Vec<Scalar> {
    products
        .into_iter()
        .map(|held| held.expect("a product").into_row().expect("a row"))
        .collect()
}
