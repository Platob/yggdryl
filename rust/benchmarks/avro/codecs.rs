//! Codec × block size: what compression costs at each block granularity.
//!
//! Blocks are produced through the record surface, whose writer emits one
//! block per incoming batch, so the batch size *is* the block size. The
//! numbers place the sweet spot rather than hard-coding a guess; the decode
//! side reports encoded bytes as throughput so the codecs stay comparable on
//! the same rows.

use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::IOBase;
use yggdryl::avro::AvroOptions;
use yggdryl::generic::IORecordOptions;
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Url, avro};

/// Rows in the sweep fixture.
const ROWS: usize = crate::bench_profile::corpus(65_536, 1_024);

/// One canonical batch of `rows` trades starting at `base`.
fn batch(base: usize, rows: usize) -> RecordBatch {
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
    ])
    .expect("a struct")
    .required_field("row")
    .into_arrow_schema()
    .expect("an arrow schema");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from_iter_values(
                (base..base + rows).map(|index| index as i64),
            )),
            Arc::new(StringArray::from_iter_values(
                (base..base + rows).map(|index| format!("SYM{:04}", index % 500)),
            )),
            Arc::new(Float64Array::from_iter_values(
                (base..base + rows).map(|index| index as f64 * 0.25),
            )),
        ],
    )
    .expect("a batch")
}

pub(crate) fn codec_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/avro_blocks");
    group.sample_size(20);

    let mut codecs = vec!["null", "deflate", "zstandard"];
    if cfg!(feature = "parquet") {
        codecs.push("snappy");
    }
    for codec in codecs {
        for block_rows in [
            crate::bench_profile::corpus(1_024, 64),
            crate::bench_profile::corpus(8_192, 256),
            crate::bench_profile::corpus(65_536, 1_024),
        ] {
            let mut stored = Buffer::new().with_media_type(
                Url::from_str("file:///sweep.avro")
                    .expect("a url")
                    .media_type(),
            );
            let options = AvroOptions::new()
                .with_codec(codec)
                .with_batch_row_size(block_rows);
            let batches: Vec<RecordBatch> = (0..ROWS / block_rows)
                .map(|index| batch(index * block_rows, block_rows))
                .collect();
            let schema = batches[0].schema();
            avro::overwrite_arrow_reader(
                &mut stored,
                yggdryl::arrow::batch_reader(schema, batches),
                &options,
            )
            .expect("the sweep fixture encodes");
            let encoded = stored.read_all_bytes().expect("the buffer reads").len() as u64;
            // Proven once outside the timers: every row survives the codec.
            let container = avro::read_container(&stored).expect("the fixture decodes");
            assert_eq!(container.rows.len(), ROWS);

            group.throughput(Throughput::Bytes(encoded));
            group.bench_function(
                format!("decode/{codec}/rows_per_block_{block_rows}"),
                |bencher| {
                    bencher.iter(|| avro::read_container(black_box(&stored)).expect("decodes"));
                },
            );
        }
    }

    group.finish();
}
