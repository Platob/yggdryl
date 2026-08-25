//! Fixtures shared by the foreign-filesystem benchmarks.
//!
//! Every measurement here is a *wrapper overhead* measurement: the same
//! payload, the same operation, once through an `arrowfs` handle and once
//! through the native handle the reader already trusts - `io::Buffer` for the
//! memory filesystem, `local::File` for the local one. The difference is what
//! the vtable and the staging cost.

pub(crate) mod bytes;
pub(crate) mod listing;
pub(crate) mod record;

use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::arrowfs::{ArrowFileSystem, LocalFileSystem, MemoryFileSystem};
use yggdryl::generic::IORecordOptions;
use yggdryl::io::IOBase;
use yggdryl::{DataType, Field, Url};

/// Rows per record fixture, large enough that encoding dominates setup.
pub(crate) const ROWS: i64 = 65_536;

/// Bytes per byte-level fixture, spanning several transfer chunks.
pub(crate) const PAYLOAD: usize = 512 * 1024;

/// The four-column root the record round trips carry.
pub(crate) fn wide() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])
    .expect("a valid struct root")
    .required_field("row")
}

/// One batch holding every row of the fixture.
pub(crate) fn batch() -> RecordBatch {
    let ids: Vec<i64> = (0..ROWS).collect();
    #[allow(clippy::cast_precision_loss)]
    let prices: Vec<f64> = ids.iter().map(|id| *id as f64).collect();
    RecordBatch::try_new(
        wide().into_arrow_schema().expect("a projectable root"),
        vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("SYMBOL-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(prices)),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("VENUE-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("a batch matching the root")
}

/// A repeating byte payload, incompressible enough to stay honest.
pub(crate) fn payload() -> Vec<u8> {
    (0..PAYLOAD).map(|index| (index % 251) as u8).collect()
}

/// One shared in-memory filesystem for the memory-backed measurements.
pub(crate) fn memory() -> Arc<MemoryFileSystem> {
    Arc::new(MemoryFileSystem::new())
}

/// One local filesystem mapping, and the temporary root it works under.
pub(crate) fn local() -> (Arc<LocalFileSystem>, std::path::PathBuf) {
    let mut root = std::env::temp_dir();
    root.push(format!("yggdryl-arrowfs-bench-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("a writable temporary root");
    (Arc::new(LocalFileSystem::new()), root)
}

/// The filesystem-relative spelling of a path under the local bench root.
pub(crate) fn local_location(root: &std::path::Path, name: &str) -> String {
    root.join(name).to_string_lossy().replace('\\', "/")
}

/// A native in-memory handle whose media type comes from a name.
pub(crate) fn buffer(name: &str) -> yggdryl::io::Buffer {
    yggdryl::io::Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("a valid location")
            .media_type(),
    )
}

/// Populate `filesystem` with a small partitioned tree for the listing legs.
///
/// Two years, two months each, four leaves per month, so a flat listing, a
/// recursive one, and a glob all have real work to do.
pub(crate) fn tree(filesystem: &dyn ArrowFileSystem, root: &str) {
    for year in ["2024", "2025"] {
        for month in ["01", "02"] {
            for part in 0..4 {
                let path = format!("{root}/year={year}/month={month}/part-{part}.parquet");
                filesystem
                    .write_full(&path, b"PAR1")
                    .expect("the fixture must write");
            }
        }
    }
}

/// Write the record fixture through a handle and return it published.
pub(crate) fn store(handle: &mut dyn IOBase, source: &RecordBatch) {
    let options = handle
        .record_options()
        .expect("an implemented encoding")
        .with_field(wide());
    handle
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(source.schema(), [source.clone()]),
            &options,
        )
        .expect("the fixture must write");
    handle.close().expect("the fixture must publish");
}
